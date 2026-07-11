//! crates/cad-import-jww: one-way JWW boundary importer.
//! It reads a small, tested subset of JWW records and writes the repo CAD source layout.

use cad_jww_codec::{
    DecodedArc as Arc, DecodedBase as EntityBase, DecodedBlock as Block,
    DecodedBlockDefinition as BlockDef, DecodedCircleSolid as CircleSolid,
    DecodedDimension as Dimension, DecodedDocument as JwwDocument, DecodedEntity as JwwEntity,
    DecodedLine as Line, DecodedPoint as Point, DecodedSolid as Solid, DecodedText as Text,
    read_document,
};
#[cfg(test)]
use cad_jww_codec::{
    DecodedHeader as JwwHeader, DecodedLayer as LayerHeader, DecodedLayerGroup as LayerGroupHeader,
    Reader,
};
use rustix::fs::{CWD, RenameFlags, renameat_with};
use rustix::io::Errno;
use serde::Serialize;
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::f64::consts::PI;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;
use ulid::Ulid;

pub const CRATE_NAME: &str = "cad-import-jww";
const CAD_SCHEMA_VERSION: &str = "0.1";
const IMPORT_EPSILON_MM: f64 = 0.001;
const IMPORT_ANGLE_EPSILON_RAD: f64 = 1e-9;
const MAX_OUTPUT_ENTITIES: usize = 250_000;
const MAX_EXPANSION_STEPS: usize = 1_000_000;

#[must_use]
pub fn crate_name() -> &'static str {
    CRATE_NAME
}

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("failed to read {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write {path}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("output project already exists: {path}")]
    OutputExists { path: PathBuf },
    #[error("invalid JWW signature")]
    InvalidSignature,
    #[error("unexpected EOF while reading {0}")]
    UnexpectedEof(&'static str),
    #[error("JWW entity list was not found")]
    EntityListNotFound,
    #[error("unknown JWW class pid {0}")]
    UnknownClassPid(u32),
    #[error(
        "unknown JWW entity class {0}; record length is not available, so import cannot continue"
    )]
    UnknownEntityClass(String),
    #[error("invalid JWW block definition count {0}")]
    InvalidBlockDefinitionCount(u32),
    #[error("JWW entity references out-of-range layer group {layer_group} or layer {layer}")]
    InvalidLayerAddress { layer_group: u32, layer: u32 },
    #[error("JWW {kind} limit exceeded ({limit})")]
    ExpansionLimitExceeded { kind: &'static str, limit: usize },
    #[error("generated CAD entity does not have valid geometry")]
    InvalidGeneratedGeometry,
    #[error("failed to parse generated CAD entity")]
    InvalidGeneratedEntity(#[source] serde_json::Error),
    #[error("imported JWW has no supported entities")]
    EmptyImport,
    #[error("failed to serialize import output")]
    Serialize(#[from] serde_json::Error),
}

impl From<cad_jww_codec::CodecError> for ImportError {
    fn from(error: cad_jww_codec::CodecError) -> Self {
        match error {
            cad_jww_codec::CodecError::InvalidSignature => Self::InvalidSignature,
            cad_jww_codec::CodecError::UnexpectedEof(field) => Self::UnexpectedEof(field),
            cad_jww_codec::CodecError::EntityListNotFound => Self::EntityListNotFound,
            cad_jww_codec::CodecError::UnknownClassPid(pid) => Self::UnknownClassPid(pid),
            cad_jww_codec::CodecError::UnknownEntityClass(class_name) => {
                Self::UnknownEntityClass(class_name)
            }
            cad_jww_codec::CodecError::InvalidBlockDefinitionCount(count) => {
                Self::InvalidBlockDefinitionCount(count)
            }
            other => Self::Write {
                path: PathBuf::from("JWW codec"),
                source: std::io::Error::new(std::io::ErrorKind::InvalidData, other),
            },
        }
    }
}

pub type ImportResult<T> = Result<T, ImportError>;

#[derive(Debug, Clone, Serialize)]
pub struct ImportReport {
    pub schema_version: String,
    pub source_path: String,
    pub project_path: String,
    pub project_name: String,
    pub drawing_name: String,
    pub jww_version: u32,
    pub jww_paper_size: u32,
    pub jww_memo: String,
    pub supported_entities: usize,
    pub warnings: Vec<ImportWarning>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportWarning {
    pub code: String,
    pub message: String,
    pub record_type: String,
}

pub fn import_jww_file(
    input_path: impl AsRef<Path>,
    out_dir: impl AsRef<Path>,
) -> ImportResult<ImportReport> {
    let input_path = input_path.as_ref();
    let out_dir = out_dir.as_ref();
    if out_dir.exists() {
        return Err(ImportError::OutputExists {
            path: out_dir.to_path_buf(),
        });
    }
    let data = fs::read(input_path).map_err(|source| ImportError::Read {
        path: input_path.to_path_buf(),
        source,
    })?;
    let document = read_document(&data)?;
    let parent = out_dir
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| ImportError::Write {
        path: parent.to_path_buf(),
        source,
    })?;
    let staging = tempfile::Builder::new()
        .prefix(".cad-jww-import-")
        .tempdir_in(parent)
        .map_err(|source| ImportError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    let report = write_project(input_path, staging.path(), out_dir, &document)?;
    publish_project(staging.path(), out_dir)?;
    Ok(report)
}

fn publish_project(staging_dir: &Path, out_dir: &Path) -> ImportResult<()> {
    renameat_with(CWD, staging_dir, CWD, out_dir, RenameFlags::NOREPLACE).map_err(|error| {
        if error == Errno::EXIST || error == Errno::NOTEMPTY {
            ImportError::OutputExists {
                path: out_dir.to_path_buf(),
            }
        } else {
            ImportError::Write {
                path: out_dir.to_path_buf(),
                source: std::io::Error::from_raw_os_error(error.raw_os_error()),
            }
        }
    })
}

fn write_project(
    input_path: &Path,
    write_dir: &Path,
    project_dir: &Path,
    document: &JwwDocument,
) -> ImportResult<ImportReport> {
    let project_name = sanitize_name(
        input_path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("jww_import"),
    );
    let drawing_name = project_name.clone();
    let mut converted = convert_entities(document)?;
    if converted.entities.is_empty() {
        return Err(ImportError::EmptyImport);
    }
    converted.warnings.push(ImportWarning {
        code: "color_mapping_defaulted".to_owned(),
        message: "JWW header color table is not parsed; default pen color mapping is used"
            .to_owned(),
        record_type: "style".to_owned(),
    });

    fs::create_dir_all(write_dir.join("rules")).map_err(|source| ImportError::Write {
        path: write_dir.join("rules"),
        source,
    })?;
    fs::create_dir_all(write_dir.join("drawings").join(&drawing_name)).map_err(|source| {
        ImportError::Write {
            path: write_dir.join("drawings").join(&drawing_name),
            source,
        }
    })?;
    fs::create_dir_all(write_dir.join("build")).map_err(|source| ImportError::Write {
        path: write_dir.join("build"),
        source,
    })?;

    let content_bbox = entity_extents(&converted.entities)?;
    let origin = sheet_origin(content_bbox);
    let paper = sheet_paper(document, &mut converted.warnings);
    let orientation = sheet_orientation(content_bbox);
    let (scale, scale_warning) = sheet_scale(document, &converted);
    if let Some(warning) = scale_warning {
        converted.warnings.push(warning);
    }
    write_text(
        &write_dir.join("cad.project.toml"),
        &format!("schema_version = \"{CAD_SCHEMA_VERSION}\"\nname = \"{project_name}\"\n"),
    )?;
    write_text(
        &write_dir.join("rules/layers.toml"),
        &layers_toml(&converted, document),
    )?;
    write_text(
        &write_dir.join("rules/styles.toml"),
        &styles_toml(&converted),
    )?;
    write_text(
        &write_dir
            .join("drawings")
            .join(&drawing_name)
            .join("sheet.toml"),
        &format!(
            "schema_version = \"{CAD_SCHEMA_VERSION}\"\npaper = \"{paper}\"\norientation = \"{orientation}\"\nscale = \"{scale}\"\norigin = [{}, {}]\n",
            format_mm(origin[0]),
            format_mm(origin[1])
        ),
    )?;
    write_text(
        &write_dir
            .join("drawings")
            .join(&drawing_name)
            .join("entities.ndjson"),
        &(converted.entities.join("\n") + "\n"),
    )?;

    let report = ImportReport {
        schema_version: CAD_SCHEMA_VERSION.to_owned(),
        source_path: input_path.to_string_lossy().into_owned(),
        project_path: project_dir.to_string_lossy().into_owned(),
        project_name,
        drawing_name,
        jww_version: document.header.version,
        jww_paper_size: document.header.paper_size,
        jww_memo: document.header.memo.clone(),
        supported_entities: converted.entities.len(),
        warnings: converted.warnings,
    };
    write_text(
        &write_dir.join("build/import-jww-report.json"),
        &format!("{}\n", serde_json::to_string_pretty(&report)?),
    )?;
    Ok(report)
}

fn write_text(path: &Path, text: &str) -> ImportResult<()> {
    fs::write(path, text).map_err(|source| ImportError::Write {
        path: path.to_path_buf(),
        source,
    })
}

#[derive(Debug)]
struct ConvertedProject {
    entities: Vec<String>,
    layer_bases: BTreeMap<String, EntityBase>,
    used_layer_groups: BTreeSet<u16>,
    layer_names: BTreeMap<String, String>,
    pen_bases: BTreeMap<String, EntityBase>,
    fill_colors: BTreeMap<String, String>,
    text_styles: BTreeMap<String, TextStyleRequirement>,
    dimension_styles: BTreeMap<String, String>,
    warnings: Vec<ImportWarning>,
}

#[derive(Debug, Clone, Copy)]
struct TextStyleRequirement {
    height: f64,
    width: f64,
    spacing: f64,
}

struct ConversionContext<'a> {
    document: &'a JwwDocument,
    block_defs: BTreeMap<u32, &'a BlockDef>,
    entities: Vec<String>,
    layer_bases: BTreeMap<String, EntityBase>,
    layer_names: BTreeMap<String, String>,
    pen_bases: BTreeMap<String, EntityBase>,
    fill_colors: BTreeMap<String, String>,
    text_styles: BTreeMap<String, TextStyleRequirement>,
    dimension_styles: BTreeMap<String, String>,
    warnings: Vec<ImportWarning>,
    id_index: u128,
    expansion_steps: usize,
    limits: ConversionLimits,
}

#[derive(Debug, Clone, Copy)]
struct ConversionLimits {
    max_output_entities: usize,
    max_expansion_steps: usize,
}

impl Default for ConversionLimits {
    fn default() -> Self {
        Self {
            max_output_entities: MAX_OUTPUT_ENTITIES,
            max_expansion_steps: MAX_EXPANSION_STEPS,
        }
    }
}

impl ConversionContext<'_> {
    fn consume_expansion_step(&mut self) -> ImportResult<()> {
        if self.expansion_steps >= self.limits.max_expansion_steps {
            return Err(ImportError::ExpansionLimitExceeded {
                kind: "expansion step",
                limit: self.limits.max_expansion_steps,
            });
        }
        self.expansion_steps += 1;
        Ok(())
    }

    fn ensure_output_capacity(&self) -> ImportResult<()> {
        if self.entities.len() >= self.limits.max_output_entities {
            return Err(ImportError::ExpansionLimitExceeded {
                kind: "output entity",
                limit: self.limits.max_output_entities,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
struct Transform2D {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    tx: f64,
    ty: f64,
}

impl Transform2D {
    fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx: 0.0,
            ty: 0.0,
        }
    }

    fn from_block(block: &Block) -> Self {
        let cos = block.rotation.cos();
        let sin = block.rotation.sin();
        Self {
            a: cos * block.scale_x,
            b: sin * block.scale_x,
            c: -sin * block.scale_y,
            d: cos * block.scale_y,
            tx: block.ref_x,
            ty: block.ref_y,
        }
    }

    fn compose(&self, rhs: &Self) -> Self {
        Self {
            a: self.a * rhs.a + self.c * rhs.b,
            b: self.b * rhs.a + self.d * rhs.b,
            c: self.a * rhs.c + self.c * rhs.d,
            d: self.b * rhs.c + self.d * rhs.d,
            tx: self.a * rhs.tx + self.c * rhs.ty + self.tx,
            ty: self.b * rhs.tx + self.d * rhs.ty + self.ty,
        }
    }

    fn apply_point(&self, point: [f64; 2]) -> [f64; 2] {
        [
            self.a * point[0] + self.c * point[1] + self.tx,
            self.b * point[0] + self.d * point[1] + self.ty,
        ]
    }

    fn apply_vector(&self, vector: [f64; 2]) -> [f64; 2] {
        [
            self.a * vector[0] + self.c * vector[1],
            self.b * vector[0] + self.d * vector[1],
        ]
    }

    fn average_scale(&self) -> f64 {
        let sx = (self.a * self.a + self.b * self.b).sqrt();
        let sy = (self.c * self.c + self.d * self.d).sqrt();
        (sx + sy) / 2.0
    }

    fn is_uniform_for_curve(&self) -> bool {
        let sx = (self.a * self.a + self.b * self.b).sqrt();
        let sy = (self.c * self.c + self.d * self.d).sqrt();
        let dot = self.a * self.c + self.b * self.d;
        sx > f64::EPSILON
            && sy > f64::EPSILON
            && (sx - sy).abs() <= 1e-9 * sx.max(sy).max(1.0)
            && dot.abs() <= 1e-9 * sx * sy
    }

    fn determinant(&self) -> f64 {
        self.a * self.d - self.b * self.c
    }
}

fn convert_entities(document: &JwwDocument) -> ImportResult<ConvertedProject> {
    convert_entities_with_limits(document, ConversionLimits::default())
}

fn convert_entities_with_limits(
    document: &JwwDocument,
    limits: ConversionLimits,
) -> ImportResult<ConvertedProject> {
    if document.header.write_layer_group > 15 {
        return Err(ImportError::InvalidLayerAddress {
            layer_group: document.header.write_layer_group,
            layer: 0,
        });
    }
    for (layer_group, group) in document.header.layer_groups.iter().enumerate() {
        if group.write_layer > 15 {
            return Err(ImportError::InvalidLayerAddress {
                layer_group: layer_group as u32,
                layer: group.write_layer,
            });
        }
    }
    let mut context = ConversionContext {
        document,
        block_defs: document
            .block_defs
            .iter()
            .map(|block_def| (block_def.number, block_def))
            .collect(),
        entities: Vec::new(),
        layer_bases: BTreeMap::new(),
        layer_names: BTreeMap::new(),
        pen_bases: BTreeMap::new(),
        fill_colors: BTreeMap::new(),
        text_styles: BTreeMap::new(),
        dimension_styles: BTreeMap::new(),
        warnings: Vec::new(),
        id_index: 1,
        expansion_steps: 0,
        limits,
    };
    let mut block_stack = Vec::new();
    for entity in &document.entities {
        convert_entity(
            &mut context,
            entity,
            &Transform2D::identity(),
            &mut block_stack,
        )?;
    }
    let used_layer_groups = context
        .layer_bases
        .values()
        .map(|base| base.layer_group.min(15))
        .collect();
    remember_declared_layers(&mut context);
    if context.layer_bases.is_empty() {
        let base = EntityBase::default();
        context.layer_names.insert(layer_id(base), "0-0".to_owned());
        context.layer_bases.insert(layer_id(base), base);
    }
    Ok(ConvertedProject {
        entities: context.entities,
        layer_bases: context.layer_bases,
        used_layer_groups,
        layer_names: context.layer_names,
        pen_bases: context.pen_bases,
        fill_colors: context.fill_colors,
        text_styles: context.text_styles,
        dimension_styles: context.dimension_styles,
        warnings: context.warnings,
    })
}

fn remember_declared_layers(context: &mut ConversionContext<'_>) {
    for (group_index, group) in context.document.header.layer_groups.iter().enumerate() {
        for (layer_index, layer) in group.layers.iter().enumerate() {
            let is_active = context.document.header.write_layer_group as usize == group_index
                && group.write_layer as usize == layer_index;
            let default_name = format!("{group_index:X}-{layer_index:X}");
            if layer.name == default_name && layer.state == 2 && layer.protect == 0 && !is_active {
                continue;
            }
            let base = EntityBase {
                layer: layer_index as u16,
                layer_group: group_index as u16,
                ..EntityBase::default()
            };
            let id = layer_id(base);
            context.layer_bases.entry(id.clone()).or_insert(base);
            context
                .layer_names
                .entry(id)
                .or_insert_with(|| layer.name.clone());
        }
    }
}

fn convert_entity(
    context: &mut ConversionContext<'_>,
    entity: &JwwEntity,
    transform: &Transform2D,
    block_stack: &mut Vec<u32>,
) -> ImportResult<()> {
    let base = match entity {
        JwwEntity::Line(entity) => entity.base,
        JwwEntity::Arc(entity) => entity.base,
        JwwEntity::Point(entity) => entity.base,
        JwwEntity::Text(entity) => entity.base,
        JwwEntity::Solid(entity) => entity.base,
        JwwEntity::CircleSolid(entity) => entity.base,
        JwwEntity::Block(entity) => entity.base,
        JwwEntity::Dimension(entity) => entity.base,
    };
    if base.layer_group > 15 || base.layer > 15 {
        return Err(ImportError::InvalidLayerAddress {
            layer_group: u32::from(base.layer_group),
            layer: u32::from(base.layer),
        });
    }
    context.consume_expansion_step()?;
    match entity {
        JwwEntity::Line(line) => {
            let line = transform_line(line, transform);
            push_line(context, &line)?;
        }
        JwwEntity::Arc(arc) => {
            if !transform.is_uniform_for_curve() {
                context.warnings.push(import_warning(
                    "unsupported_scaled_curve",
                    "CDataEnko",
                    arc.base,
                    "non-uniform or sheared block transform cannot preserve curve parameters",
                ));
                return Ok(());
            }
            let arc = transform_arc(arc, transform);
            push_arc(context, &arc)?;
        }
        JwwEntity::Text(text) => {
            let text = transform_text(text, transform);
            push_text(context, &text)?;
        }
        JwwEntity::Dimension(dimension) => {
            let dimension = transform_dimension(dimension, transform);
            push_dimension(context, &dimension)?;
        }
        JwwEntity::Point(point) => {
            let point = transform_point_entity(point, transform);
            push_point(context, &point)?;
        }
        JwwEntity::Solid(solid) => {
            let solid = transform_solid(solid, transform);
            push_solid(context, &solid)?;
        }
        JwwEntity::CircleSolid(solid) => {
            if !transform.is_uniform_for_curve() {
                context.warnings.push(import_warning(
                    "unsupported_scaled_curve",
                    "CDataSolid",
                    solid.base,
                    "non-uniform or sheared block transform cannot preserve curve solid",
                ));
            } else {
                let solid = transform_circle_solid(solid, transform);
                push_circle_solid(context, &solid)?;
            }
        }
        JwwEntity::Block(block) => {
            if !block_is_finite(block) {
                context.warnings.push(geometry_skipped(
                    "CDataBlock",
                    block.base,
                    "block transform contains a non-finite value",
                ));
            } else {
                expand_block(context, block, transform, block_stack)?;
            }
        }
    }
    Ok(())
}

fn push_line(context: &mut ConversionContext<'_>, line: &Line) -> ImportResult<()> {
    if !line_is_finite(line) || line_length(line) <= IMPORT_EPSILON_MM {
        context.warnings.push(geometry_skipped(
            "CDataSen",
            line.base,
            "line geometry is non-finite, zero length, or below import epsilon",
        ));
        return Ok(());
    }
    remember_layer(
        context.document,
        &mut context.layer_bases,
        &mut context.layer_names,
        &mut context.warnings,
        line.base,
    );
    remember_pen(&mut context.pen_bases, line.base);
    context.ensure_output_capacity()?;
    context.entities.push(entity_line_json(
        next_id(&mut context.id_index),
        &layer_id(line.base),
        line,
    ));
    Ok(())
}

fn push_arc(context: &mut ConversionContext<'_>, arc: &Arc) -> ImportResult<()> {
    if !arc_is_finite(arc) || arc.radius.abs() <= IMPORT_EPSILON_MM {
        context.warnings.push(geometry_skipped(
            "CDataEnko",
            arc.base,
            "curve geometry is non-finite or radius is below import epsilon",
        ));
        return Ok(());
    }
    if !arc.flatness.is_finite() || arc.flatness <= IMPORT_EPSILON_MM {
        context.warnings.push(geometry_skipped(
            "CDataEnko",
            arc.base,
            "ellipse flatness is invalid or below import epsilon",
        ));
        return Ok(());
    }
    if !arc.is_full_circle && arc.sweep_rad.abs() <= IMPORT_ANGLE_EPSILON_RAD {
        context.warnings.push(geometry_skipped(
            "CDataEnko",
            arc.base,
            "arc span is zero or below import epsilon",
        ));
        return Ok(());
    }
    remember_layer(
        context.document,
        &mut context.layer_bases,
        &mut context.layer_names,
        &mut context.warnings,
        arc.base,
    );
    remember_pen(&mut context.pen_bases, arc.base);
    context.ensure_output_capacity()?;
    context.entities.push(entity_curve_json(
        next_id(&mut context.id_index),
        &layer_id(arc.base),
        arc,
    ));
    Ok(())
}

fn push_text(context: &mut ConversionContext<'_>, text: &Text) -> ImportResult<()> {
    if !text_is_finite(text) || text_height(text) <= IMPORT_EPSILON_MM {
        context.warnings.push(geometry_skipped(
            "CDataMoji",
            text.base,
            "text geometry is non-finite or height is below import epsilon",
        ));
        return Ok(());
    }
    remember_layer(
        context.document,
        &mut context.layer_bases,
        &mut context.layer_names,
        &mut context.warnings,
        text.base,
    );
    remember_pen(&mut context.pen_bases, text.base);
    let style_id = record_text_style(context, text);
    context.ensure_output_capacity()?;
    context.entities.push(entity_text_json(
        next_id(&mut context.id_index),
        &layer_id(text.base),
        &style_id,
        text,
    ));
    Ok(())
}

fn push_dimension(context: &mut ConversionContext<'_>, dimension: &Dimension) -> ImportResult<()> {
    if !dimension_is_finite(dimension) || line_length(&dimension.line) <= IMPORT_EPSILON_MM {
        context.warnings.push(geometry_skipped(
            "CDataSunpou",
            dimension.base,
            "dimension geometry is non-finite or line length is below import epsilon",
        ));
        return Ok(());
    }
    if text_height(&dimension.text) <= IMPORT_EPSILON_MM {
        context.warnings.push(geometry_skipped(
            "CDataSunpou",
            dimension.base,
            "dimension text height is zero or below import epsilon",
        ));
        return Ok(());
    }
    remember_layer(
        context.document,
        &mut context.layer_bases,
        &mut context.layer_names,
        &mut context.warnings,
        dimension.base,
    );
    remember_pen(&mut context.pen_bases, dimension.base);
    let style_id = record_dimension_style(context, &dimension.text);
    context.ensure_output_capacity()?;
    context.entities.push(entity_dimension_json(
        next_id(&mut context.id_index),
        &layer_id(dimension.base),
        &style_id,
        dimension,
    ));
    Ok(())
}

fn push_point(context: &mut ConversionContext<'_>, point: &Point) -> ImportResult<()> {
    if !point_entity_is_finite(point) || point.scale <= 0.0 {
        context.warnings.push(geometry_skipped(
            "CDataTen",
            point.base,
            "point contains a non-finite coordinate, rotation, or scale",
        ));
        return Ok(());
    }
    if point
        .marker_code
        .is_some_and(|code| !known_marker_code(code))
    {
        context.warnings.push(import_warning(
            "unsupported_marker",
            "CDataTen",
            point.base,
            &format!(
                "point marker code {} is preserved and rendered as a generic marker",
                point.marker_code.expect("marker code checked")
            ),
        ));
    }
    remember_layer(
        context.document,
        &mut context.layer_bases,
        &mut context.layer_names,
        &mut context.warnings,
        point.base,
    );
    remember_pen(&mut context.pen_bases, point.base);
    context.ensure_output_capacity()?;
    context.entities.push(entity_point_json(
        next_id(&mut context.id_index),
        &layer_id(point.base),
        point,
    ));
    Ok(())
}

fn known_marker_code(code: u32) -> bool {
    (1..=11).contains(&code) || code >= u32::MAX - 6
}

fn push_solid(context: &mut ConversionContext<'_>, solid: &Solid) -> ImportResult<()> {
    if !solid.points.iter().all(|point| point_is_finite(*point)) {
        context.warnings.push(geometry_skipped(
            "CDataSolid",
            solid.base,
            "solid contains a non-finite point",
        ));
        return Ok(());
    }
    remember_layer(
        context.document,
        &mut context.layer_bases,
        &mut context.layer_names,
        &mut context.warnings,
        solid.base,
    );
    remember_pen(&mut context.pen_bases, solid.base);
    let fill = remember_fill(&mut context.fill_colors, solid.base, solid.color);
    context.ensure_output_capacity()?;
    context.entities.push(entity_solid_json(
        next_id(&mut context.id_index),
        &layer_id(solid.base),
        solid,
        &fill,
    ));
    Ok(())
}

fn push_circle_solid(context: &mut ConversionContext<'_>, solid: &CircleSolid) -> ImportResult<()> {
    if !circle_solid_is_finite(solid)
        || solid.radius.abs() <= IMPORT_EPSILON_MM
        || solid.flatness.abs() <= IMPORT_EPSILON_MM
        || solid.sweep_rad.abs() <= IMPORT_ANGLE_EPSILON_RAD
    {
        context.warnings.push(geometry_skipped(
            "CDataSolid",
            solid.base,
            "curve solid geometry is invalid or below import epsilon",
        ));
        return Ok(());
    }
    remember_layer(
        context.document,
        &mut context.layer_bases,
        &mut context.layer_names,
        &mut context.warnings,
        solid.base,
    );
    remember_pen(&mut context.pen_bases, solid.base);
    let fill = remember_fill(&mut context.fill_colors, solid.base, solid.color);
    context.ensure_output_capacity()?;
    context.entities.push(entity_curve_solid_json(
        next_id(&mut context.id_index),
        &layer_id(solid.base),
        solid,
        &fill,
    ));
    Ok(())
}

fn expand_block(
    context: &mut ConversionContext<'_>,
    block: &Block,
    transform: &Transform2D,
    block_stack: &mut Vec<u32>,
) -> ImportResult<()> {
    if block_stack.len() >= 32 || block_stack.contains(&block.def_number) {
        context.warnings.push(import_warning(
            "block_cycle",
            "CDataBlock",
            block.base,
            &format!("block {} cannot be expanded safely", block.def_number),
        ));
        return Ok(());
    }
    let Some(block_def) = context.block_defs.get(&block.def_number).copied() else {
        context.warnings.push(import_warning(
            "unresolved_block",
            "CDataBlock",
            block.base,
            &format!("block definition {} was not found", block.def_number),
        ));
        return Ok(());
    };
    let _block_name = &block_def.name;
    let child_transform = transform.compose(&Transform2D::from_block(block));
    block_stack.push(block.def_number);
    for child in &block_def.entities {
        convert_entity(context, child, &child_transform, block_stack)?;
    }
    block_stack.pop();
    Ok(())
}

fn transform_line(line: &Line, transform: &Transform2D) -> Line {
    Line {
        base: line.base,
        start: transform.apply_point(line.start),
        end: transform.apply_point(line.end),
    }
}

fn transform_arc(arc: &Arc, transform: &Transform2D) -> Arc {
    let local_axis = [arc.tilt_rad.cos(), arc.tilt_rad.sin()];
    let transformed_axis = transform.apply_vector(local_axis);
    let parameter_sign = if transform.determinant() < 0.0 {
        -1.0
    } else {
        1.0
    };
    Arc {
        base: arc.base,
        center: transform.apply_point(arc.center),
        radius: arc.radius * transform.average_scale().abs(),
        start_rad: parameter_sign * arc.start_rad,
        sweep_rad: parameter_sign * arc.sweep_rad,
        tilt_rad: transformed_axis[1].atan2(transformed_axis[0]),
        flatness: arc.flatness,
        is_full_circle: arc.is_full_circle,
    }
}

fn transform_text(text: &Text, transform: &Transform2D) -> Text {
    let scale = transform.average_scale().abs();
    let angle_rad = text.angle.to_radians();
    let baseline = transform.apply_vector([angle_rad.cos(), angle_rad.sin()]);
    Text {
        base: text.base,
        start: transform.apply_point(text.start),
        size_x: text.size_x * scale,
        size_y: text.size_y * scale,
        spacing: text.spacing * scale,
        angle: baseline[1].atan2(baseline[0]).to_degrees(),
        mirror_y: text.mirror_y ^ (transform.determinant() < 0.0),
        font_name: text.font_name.clone(),
        content: text.content.clone(),
    }
}

fn transform_dimension(dimension: &Dimension, transform: &Transform2D) -> Dimension {
    Dimension {
        base: dimension.base,
        line: transform_line(&dimension.line, transform),
        text: transform_text(&dimension.text, transform),
    }
}

fn transform_point_entity(point: &Point, transform: &Transform2D) -> Point {
    let axis = transform.apply_vector([point.rotation.cos(), point.rotation.sin()]);
    Point {
        base: point.base,
        at: transform.apply_point(point.at),
        temporary: point.temporary,
        marker_code: point.marker_code,
        rotation: axis[1].atan2(axis[0]),
        scale: point.scale * transform.average_scale().abs(),
    }
}

fn transform_solid(solid: &Solid, transform: &Transform2D) -> Solid {
    Solid {
        base: solid.base,
        points: solid.points.map(|point| transform.apply_point(point)),
        color: solid.color,
    }
}

fn transform_circle_solid(solid: &CircleSolid, transform: &Transform2D) -> CircleSolid {
    let axis = transform.apply_vector([solid.tilt_rad.cos(), solid.tilt_rad.sin()]);
    let sign = if transform.determinant() < 0.0 {
        -1.0
    } else {
        1.0
    };
    CircleSolid {
        base: solid.base,
        center: transform.apply_point(solid.center),
        radius: solid.radius * transform.average_scale().abs(),
        flatness: solid.flatness,
        tilt_rad: axis[1].atan2(axis[0]),
        start_rad: sign * solid.start_rad,
        sweep_rad: sign * solid.sweep_rad,
        solid_param: solid.solid_param * transform.average_scale().abs(),
        color: solid.color,
    }
}

fn remember_layer(
    document: &JwwDocument,
    layer_bases: &mut BTreeMap<String, EntityBase>,
    layer_names: &mut BTreeMap<String, String>,
    _warnings: &mut Vec<ImportWarning>,
    base: EntityBase,
) {
    let id = layer_id(base);
    if layer_bases.contains_key(&id) {
        return;
    }
    layer_names.insert(id.clone(), jww_layer_name(document, base));
    layer_bases.insert(id, base);
}

fn remember_pen(pens: &mut BTreeMap<String, EntityBase>, base: EntityBase) {
    pens.entry(pen_id(base)).or_insert(base);
}

fn remember_fill(
    colors: &mut BTreeMap<String, String>,
    base: EntityBase,
    arbitrary: Option<u32>,
) -> String {
    if let Some(rgb) = arbitrary {
        let id = format!("jww_solid_rgb_{:06X}", rgb & 0x00ff_ffff);
        colors
            .entry(id.clone())
            .or_insert_with(|| format!("#{:06X}", rgb & 0x00ff_ffff));
        id
    } else {
        let id = color_id(base);
        colors
            .entry(id.clone())
            .or_insert_with(|| pen_color_rgb(base.pen_color).to_owned());
        id
    }
}

fn jww_layer_name(document: &JwwDocument, base: EntityBase) -> String {
    let group_index = base.layer_group.min(15) as usize;
    let layer_index = base.layer.min(15) as usize;
    let group = &document.header.layer_groups[group_index];
    let layer = &group.layers[layer_index];
    if group.name.is_empty() && layer.name.is_empty() {
        return format!("{group_index:X}-{layer_index:X}");
    }
    format!("{} {}", group.name, layer.name).trim().to_owned()
}

fn geometry_skipped(record_type: &str, base: EntityBase, message: &str) -> ImportWarning {
    import_warning("geometry_skipped", record_type, base, message)
}

fn import_warning(code: &str, record_type: &str, base: EntityBase, message: &str) -> ImportWarning {
    ImportWarning {
        code: code.to_owned(),
        message: format!("{message} on {}", layer_id(base)),
        record_type: record_type.to_owned(),
    }
}

fn record_text_style(context: &mut ConversionContext<'_>, text: &Text) -> String {
    if !text.font_name.is_empty() && text.font_name != "Hiragino Sans" {
        let message = format!(
            "font {:?} is substituted with Hiragino Sans",
            text.font_name
        );
        if !context.warnings.iter().any(|warning| {
            warning.code == "font_substituted" && warning.message.starts_with(&message)
        }) {
            context.warnings.push(import_warning(
                "font_substituted",
                "CDataMoji",
                text.base,
                &message,
            ));
        }
    }
    let style = TextStyleRequirement {
        height: text_height(text),
        width: text.size_x.abs(),
        spacing: text.spacing.abs(),
    };
    let id = text_style_id(style);
    context.text_styles.entry(id.clone()).or_insert(style);
    id
}

fn record_dimension_style(context: &mut ConversionContext<'_>, text: &Text) -> String {
    let text_style = record_text_style(context, text);
    let id = dimension_style_id(TextStyleRequirement {
        height: text_height(text),
        width: text.size_x.abs(),
        spacing: text.spacing.abs(),
    });
    context
        .dimension_styles
        .entry(id.clone())
        .or_insert(text_style);
    id
}

fn text_height(text: &Text) -> f64 {
    text.size_y.abs()
}

fn point_is_finite(point: [f64; 2]) -> bool {
    point[0].is_finite() && point[1].is_finite()
}

fn line_is_finite(line: &Line) -> bool {
    point_is_finite(line.start) && point_is_finite(line.end)
}

fn arc_is_finite(arc: &Arc) -> bool {
    point_is_finite(arc.center)
        && arc.radius.is_finite()
        && arc.start_rad.is_finite()
        && arc.sweep_rad.is_finite()
        && arc.tilt_rad.is_finite()
        && arc.flatness.is_finite()
}

fn text_is_finite(text: &Text) -> bool {
    point_is_finite(text.start)
        && text.size_x.is_finite()
        && text.size_y.is_finite()
        && text.spacing.is_finite()
        && text.angle.is_finite()
}

fn dimension_is_finite(dimension: &Dimension) -> bool {
    line_is_finite(&dimension.line) && text_is_finite(&dimension.text)
}

fn point_entity_is_finite(point: &Point) -> bool {
    point_is_finite(point.at) && point.rotation.is_finite() && point.scale.is_finite()
}

fn circle_solid_is_finite(solid: &CircleSolid) -> bool {
    point_is_finite(solid.center)
        && solid.radius.is_finite()
        && solid.flatness.is_finite()
        && solid.tilt_rad.is_finite()
        && solid.start_rad.is_finite()
        && solid.sweep_rad.is_finite()
        && solid.solid_param.is_finite()
}

fn block_is_finite(block: &Block) -> bool {
    block.ref_x.is_finite()
        && block.ref_y.is_finite()
        && block.scale_x.is_finite()
        && block.scale_y.is_finite()
        && block.rotation.is_finite()
}

fn line_length(line: &Line) -> f64 {
    point_distance(line.start, line.end)
}

fn point_distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = b[0] - a[0];
    let dy = b[1] - a[1];
    (dx * dx + dy * dy).sqrt()
}

fn next_id(index: &mut u128) -> String {
    let ulid = Ulid::from_parts(0, *index);
    *index += 1;
    format!("ent_{ulid}")
}

fn entity_line_json(id: String, layer: &str, line: &Line) -> String {
    json!({
        "schema_version": CAD_SCHEMA_VERSION,
        "id": id,
        "type": "line",
        "layer": layer,
        "pen": pen_id(line.base),
        "p1": line.start,
        "p2": line.end,
    })
    .to_string()
}

fn entity_curve_json(id: String, layer: &str, arc: &Arc) -> String {
    let start_deg = (arc.tilt_rad + arc.start_rad).to_degrees();
    let end_deg = start_deg + arc.sweep_rad.to_degrees();
    if (arc.flatness - 1.0).abs() <= 1e-9 && arc.is_full_circle {
        json!({
            "schema_version": CAD_SCHEMA_VERSION,
            "id": id,
            "type": "circle",
            "layer": layer,
            "pen": pen_id(arc.base),
            "center": arc.center,
            "radius": arc.radius.abs(),
        })
        .to_string()
    } else if (arc.flatness - 1.0).abs() <= 1e-9 {
        json!({
            "schema_version": CAD_SCHEMA_VERSION,
            "id": id,
            "type": "arc",
            "layer": layer,
            "pen": pen_id(arc.base),
            "center": arc.center,
            "radius": arc.radius.abs(),
            "start_deg": start_deg,
            "end_deg": end_deg,
        })
        .to_string()
    } else {
        let ellipse_start_deg = arc.start_rad.to_degrees();
        let ellipse_end_deg = if arc.is_full_circle {
            ellipse_start_deg + 360.0
        } else {
            ellipse_start_deg + arc.sweep_rad.to_degrees()
        };
        json!({
            "schema_version": CAD_SCHEMA_VERSION,
            "id": id,
            "type": "ellipse",
            "layer": layer,
            "pen": pen_id(arc.base),
            "center": arc.center,
            "radius_x": arc.radius.abs(),
            "radius_y": arc.radius.abs() * arc.flatness,
            "rotation_deg": arc.tilt_rad.to_degrees(),
            "start_deg": ellipse_start_deg,
            "end_deg": ellipse_end_deg,
        })
        .to_string()
    }
}

fn entity_text_json(id: String, layer: &str, style: &str, text: &Text) -> String {
    json!({
        "schema_version": CAD_SCHEMA_VERSION,
        "id": id,
        "type": "text",
        "layer": layer,
        "pen": pen_id(text.base),
        "style": style,
        "at": text.start,
        "rotation_deg": text.angle,
        "mirror_y": text.mirror_y,
        "value": text.content,
    })
    .to_string()
}

fn entity_dimension_json(id: String, layer: &str, style: &str, dimension: &Dimension) -> String {
    json!({
        "schema_version": CAD_SCHEMA_VERSION,
        "id": id,
        "type": "dimension",
        "layer": layer,
        "pen": pen_id(dimension.base),
        "style": style,
        "p1": dimension.line.start,
        "p2": dimension.line.end,
        "offset": dimension_offset(&dimension.line, dimension.text.start),
        "text_rotation_deg": dimension.text.angle,
        "text_mirror_y": dimension.text.mirror_y,
        "value": non_empty_string(&dimension.text.content),
    })
    .to_string()
}

fn entity_point_json(id: String, layer: &str, point: &Point) -> String {
    json!({
        "schema_version": CAD_SCHEMA_VERSION,
        "id": id,
        "type": "point",
        "layer": layer,
        "pen": pen_id(point.base),
        "at": point.at,
        "temporary": point.temporary,
        "marker_code": point.marker_code,
        "rotation_deg": point.rotation.to_degrees(),
        "scale": point.scale.abs(),
    })
    .to_string()
}

fn entity_solid_json(id: String, layer: &str, solid: &Solid, fill: &str) -> String {
    json!({
        "schema_version": CAD_SCHEMA_VERSION,
        "id": id,
        "type": "solid",
        "layer": layer,
        "pen": pen_id(solid.base),
        "points": solid.points,
        "fill": fill,
    })
    .to_string()
}

fn entity_curve_solid_json(id: String, layer: &str, solid: &CircleSolid, fill: &str) -> String {
    json!({
        "schema_version": CAD_SCHEMA_VERSION,
        "id": id,
        "type": "curve_solid",
        "layer": layer,
        "pen": pen_id(solid.base),
        "center": solid.center,
        "radius": solid.radius.abs(),
        "flatness": solid.flatness.abs(),
        "rotation_deg": solid.tilt_rad.to_degrees(),
        "start_deg": solid.start_rad.to_degrees(),
        "end_deg": (solid.start_rad + solid.sweep_rad).to_degrees(),
        "solid_param": solid.solid_param.abs(),
        "encoding_code": solid.base.pen_style,
        "fill": fill,
    })
    .to_string()
}

fn non_empty_string(value: &str) -> Option<&str> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

fn dimension_offset(line: &Line, text_at: [f64; 2]) -> f64 {
    let dx = line.end[0] - line.start[0];
    let dy = line.end[1] - line.start[1];
    let len = (dx * dx + dy * dy).sqrt();
    if len <= f64::EPSILON {
        return 0.0;
    }
    let tx = text_at[0] - line.start[0];
    let ty = text_at[1] - line.start[1];
    ((dx * ty) - (dy * tx)) / len
}

fn layer_id(base: EntityBase) -> String {
    format!(
        "jww_g{:X}_l{:X}",
        base.layer_group.min(15),
        base.layer.min(15)
    )
}

fn color_id(base: EntityBase) -> String {
    format!("jww_color_{}", base.pen_color)
}

fn line_type_id(base: EntityBase) -> String {
    format!("jww_line_{}", base.pen_style)
}

fn pen_id(base: EntityBase) -> String {
    format!(
        "jww_pen_c{}_l{}_w{}",
        base.pen_color, base.pen_style, base.pen_width
    )
}

fn text_style_id(style: TextStyleRequirement) -> String {
    format!(
        "jww_text_h{}_w{}_s{}",
        style_suffix(style.height),
        style_suffix(style.width),
        style_suffix(style.spacing)
    )
}

fn dimension_style_id(style: TextStyleRequirement) -> String {
    format!(
        "jww_dimension_h{}_w{}_s{}",
        style_suffix(style.height),
        style_suffix(style.width),
        style_suffix(style.spacing)
    )
}

fn style_suffix(value: f64) -> String {
    format_mm(value.abs()).replace('.', "_")
}

fn layers_toml(project: &ConvertedProject, document: &JwwDocument) -> String {
    let mut out = String::new();
    let active_layer = format!(
        "jww_g{:X}_l{:X}",
        document.header.write_layer_group.min(15),
        document.header.layer_groups[document.header.write_layer_group.min(15) as usize]
            .write_layer
            .min(15)
    );
    if project.layer_bases.contains_key(&active_layer) {
        out.push_str(&format!("active_layer = \"{active_layer}\"\n\n"));
    }
    let mut used_groups = project
        .layer_bases
        .values()
        .map(|base| base.layer_group.min(15))
        .collect::<BTreeSet<_>>();
    for (group_index, group) in document.header.layer_groups.iter().enumerate() {
        let default_name = format!("Group{group_index:X}");
        if group.name != default_name
            || group.state != 2
            || group.protect != 0
            || (group.scale - 1.0).abs() > f64::EPSILON
            || document.header.write_layer_group as usize == group_index
        {
            used_groups.insert(group_index as u16);
        }
    }
    for group_index in used_groups {
        let group = &document.header.layer_groups[group_index as usize];
        out.push_str(&format!(
            "[groups.jww_g{group_index:X}]\nname = \"{}\"\norder = {group_index}\nscale_denominator = {}\nvisible = {}\nlocked = {}\n\n",
            toml_escape(&group.name),
            format_mm(group.scale.max(1.0)),
            group.state != 0,
            group.state == 1 || group.protect != 0,
        ));
    }
    for (id, base) in &project.layer_bases {
        let name = project
            .layer_names
            .get(id)
            .filter(|name| !name.is_empty())
            .map_or(id.as_str(), String::as_str);
        out.push_str(&format!(
            "[layers.\"{id}\"]\nname = \"{}\"\ngroup = \"jww_g{:X}\"\norder = {}\nlocked = {}\nvisible = {}\nprintable = true\ncolor = \"{}\"\nline_type = \"{}\"\nline_width = {}\n\n",
            toml_escape(name),
            base.layer_group.min(15),
            base.layer.min(15),
            layer_locked(document, *base),
            layer_visible(document, *base),
            color_id(*base),
            line_type_id(*base),
            format_mm(line_width(*base))
        ));
    }
    out
}

fn layer_visible(document: &JwwDocument, base: EntityBase) -> bool {
    let group = &document.header.layer_groups[base.layer_group.min(15) as usize];
    let layer = &group.layers[base.layer.min(15) as usize];
    group.state != 0 && layer.state != 0
}

fn layer_locked(document: &JwwDocument, base: EntityBase) -> bool {
    let group = &document.header.layer_groups[base.layer_group.min(15) as usize];
    let layer = &group.layers[base.layer.min(15) as usize];
    group.state == 1 || group.protect != 0 || layer.state == 1 || layer.protect != 0
}

fn toml_escape(value: &str) -> String {
    let mut escaped = String::new();
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0c}' => escaped.push_str("\\f"),
            character if character.is_control() => {
                use std::fmt::Write as _;
                let _ = write!(escaped, "\\u{:04X}", character as u32);
            }
            character => escaped.push(character),
        }
    }
    escaped
}

fn styles_toml(project: &ConvertedProject) -> String {
    let mut colors = BTreeSet::new();
    let mut line_types = BTreeSet::new();
    for base in project
        .layer_bases
        .values()
        .chain(project.pen_bases.values())
    {
        colors.insert(base.pen_color);
        line_types.insert(base.pen_style);
    }
    let mut out = String::new();
    for color in colors {
        out.push_str(&format!(
            "[colors.jww_color_{color}]\nrgb = \"{}\"\nprint_width = 0.25\n\n",
            pen_color_rgb(color)
        ));
    }
    for (id, rgb) in &project.fill_colors {
        if !out.contains(&format!("[colors.{id}]")) {
            out.push_str(&format!(
                "[colors.{id}]\nrgb = \"{rgb}\"\nprint_width = 0.25\n\n"
            ));
        }
    }
    for line_type in line_types {
        out.push_str(&format!(
            "[line_types.jww_line_{line_type}]\ndash = [{}]\n\n",
            dash_pattern(line_type)
        ));
    }
    if project.pen_bases.is_empty() {
        out.push_str("[pens]\n\n");
    } else {
        for (id, base) in &project.pen_bases {
            out.push_str(&format!(
                "[pens.{id}]\ncolor = \"{}\"\nline_type = \"{}\"\nline_width = {}\n\n",
                color_id(*base),
                line_type_id(*base),
                format_mm(line_width(*base)),
            ));
        }
    }
    if project.text_styles.is_empty() {
        out.push_str("[text_styles]\n\n");
    } else {
        for (id, style) in &project.text_styles {
            out.push_str(&format!(
                "[text_styles.{id}]\nfont_family = \"Hiragino Sans\"\nheight = {}\nwidth = {}\nspacing = {}\nalign = \"left\"\n\n",
                format_mm(style.height),
                format_mm(style.width),
                format_mm(style.spacing)
            ));
        }
    }
    if project.dimension_styles.is_empty() {
        out.push_str("[dimension_styles]\n");
    } else {
        for (id, text_style) in &project.dimension_styles {
            out.push_str(&format!(
                "[dimension_styles.{id}]\ntext_style = \"{text_style}\"\narrow_size = 120\nextension_gap = 40\nprecision = 0\nunit = \"mm\"\n\n"
            ));
        }
    }
    out
}

fn line_width(base: EntityBase) -> f64 {
    if base.pen_width == 0 {
        0.25
    } else {
        (base.pen_width as f64 / 100.0).max(0.05)
    }
}

fn pen_color_rgb(color: u16) -> &'static str {
    match color {
        1 => "#000000",
        2 => "#FF0000",
        3 => "#00AA00",
        4 => "#0000FF",
        5 => "#FFFF00",
        6 => "#FF00FF",
        7 => "#00FFFF",
        8 => "#FFFFFF",
        _ => "#333333",
    }
}

fn dash_pattern(line_type: u8) -> &'static str {
    match line_type {
        2 => "8.0, 4.0",
        3 => "2.0, 3.0",
        4 => "12.0, 4.0, 2.0, 4.0",
        _ => "",
    }
}

fn sheet_paper(document: &JwwDocument, warnings: &mut Vec<ImportWarning>) -> &'static str {
    match document.header.paper_size {
        0 => "A0",
        1 => "A1",
        2 => "A2",
        3 => "A3",
        4 => "A4",
        paper_size => {
            warnings.push(ImportWarning {
                code: "unsupported_paper_size".to_owned(),
                message: format!("JWW paper size {paper_size} is unsupported; A3 is used"),
                record_type: "header".to_owned(),
            });
            "A3"
        }
    }
}

fn sheet_orientation(content_bbox: Option<cad_model::BBox>) -> &'static str {
    let Some(bbox) = content_bbox else {
        return "landscape";
    };
    if bbox.width() >= bbox.height() {
        "landscape"
    } else {
        "portrait"
    }
}

fn sheet_scale(
    document: &JwwDocument,
    project: &ConvertedProject,
) -> (String, Option<ImportWarning>) {
    let mut scales = BTreeSet::<String>::new();
    for group_index in &project.used_layer_groups {
        let scale = document.header.layer_groups[*group_index as usize].scale;
        if scale.is_finite() && scale > 0.0 {
            scales.insert(format_mm(scale));
        }
    }
    match scales.len() {
        0 => (
            "1/1".to_owned(),
            Some(ImportWarning {
                code: "invalid_layer_group_scale".to_owned(),
                message: "no valid JWW layer group scale was found; 1/1 is used".to_owned(),
                record_type: "header".to_owned(),
            }),
        ),
        1 => (
            format!("1/{}", scales.iter().next().expect("one scale")),
            None,
        ),
        _ => (
            "1/1".to_owned(),
            Some(ImportWarning {
                code: "mixed_layer_group_scale".to_owned(),
                message: "multiple JWW layer group scales are used; 1/1 is used".to_owned(),
                record_type: "header".to_owned(),
            }),
        ),
    }
}

fn sheet_origin(content_bbox: Option<cad_model::BBox>) -> [f64; 2] {
    let Some(bbox) = content_bbox else {
        return [-1000.0, -1000.0];
    };
    [bbox.min[0] - 1000.0, bbox.min[1] - 1000.0]
}

fn entity_extents(entities: &[String]) -> ImportResult<Option<cad_model::BBox>> {
    let mut result: Option<cad_model::BBox> = None;
    for entity in entities {
        let entity = serde_json::from_str::<cad_model::Entity>(entity)
            .map_err(ImportError::InvalidGeneratedEntity)?;
        let bbox = cad_model::entity_bbox(&entity).ok_or(ImportError::InvalidGeneratedGeometry)?;
        result = Some(result.map_or(bbox, |current| cad_model::BBox {
            min: [
                current.min[0].min(bbox.min[0]),
                current.min[1].min(bbox.min[1]),
            ],
            max: [
                current.max[0].max(bbox.max[0]),
                current.max[1].max(bbox.max[1]),
            ],
        }));
    }
    Ok(result)
}

fn format_mm(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    let mut text = format!("{rounded:.3}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" { "0".to_owned() } else { text }
}

fn sanitize_name(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let trimmed = out.trim_matches('_');
    if trimmed.is_empty() {
        "jww_import".to_owned()
    } else {
        trimmed.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::{Arc as SyncArc, Barrier};

    #[test]
    fn rejects_invalid_signature() {
        let error = read_document(b"NotJwwData")
            .map_err(ImportError::from)
            .expect_err("invalid JWW should fail");
        assert!(matches!(error, ImportError::InvalidSignature));
    }

    #[test]
    fn rejects_unknown_entity_class_as_fatal() {
        let mut data = Vec::new();
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&0xFFFFu16.to_le_bytes());
        data.extend_from_slice(&600u16.to_le_bytes());
        let class_name = b"CDataUnknown";
        data.extend_from_slice(&(class_name.len() as u16).to_le_bytes());
        data.extend_from_slice(class_name);
        let mut reader = Reader::new(&data);

        let error = cad_jww_codec::read_entity_list(&mut reader, 600)
            .map_err(ImportError::from)
            .expect_err("unknown class should fail");

        assert!(matches!(error, ImportError::UnknownEntityClass(name) if name == "CDataUnknown"));
    }

    #[test]
    fn rejects_invalid_block_definition_count() {
        let data = 10_001u32.to_le_bytes();

        let error = cad_jww_codec::read_block_definitions(&data, 600)
            .map_err(ImportError::from)
            .expect_err("invalid count should fail");

        assert!(matches!(
            error,
            ImportError::InvalidBlockDefinitionCount(10_001)
        ));
    }

    #[test]
    fn rejects_block_dag_when_expansion_budget_is_exceeded() {
        let document = test_document(
            vec![JwwEntity::Block(test_block(0.0, 0.0, 1.0, 1.0, 0.0, 1))],
            vec![
                BlockDef {
                    number: 1,
                    name: "ROOT".to_owned(),
                    entities: vec![
                        JwwEntity::Block(test_block(0.0, 0.0, 1.0, 1.0, 0.0, 2)),
                        JwwEntity::Block(test_block(0.0, 0.0, 1.0, 1.0, 0.0, 2)),
                    ],
                },
                BlockDef {
                    number: 2,
                    name: "LEAF".to_owned(),
                    entities: vec![JwwEntity::Line(Line {
                        base: EntityBase::default(),
                        start: [0.0, 0.0],
                        end: [10.0, 0.0],
                    })],
                },
            ],
        );

        let error = convert_entities_with_limits(
            &document,
            ConversionLimits {
                max_output_entities: 100,
                max_expansion_steps: 3,
            },
        )
        .expect_err("expansion budget should fail");

        assert!(matches!(
            error,
            ImportError::ExpansionLimitExceeded {
                kind: "expansion step",
                limit: 3
            }
        ));
    }

    #[test]
    fn rejects_output_entity_limit_without_truncating() {
        let document = test_document(
            vec![
                JwwEntity::Line(Line {
                    base: EntityBase::default(),
                    start: [0.0, 0.0],
                    end: [10.0, 0.0],
                }),
                JwwEntity::Line(Line {
                    base: EntityBase::default(),
                    start: [0.0, 10.0],
                    end: [10.0, 10.0],
                }),
            ],
            Vec::new(),
        );

        let error = convert_entities_with_limits(
            &document,
            ConversionLimits {
                max_output_entities: 1,
                max_expansion_steps: 10,
            },
        )
        .expect_err("output budget should fail");

        assert!(matches!(
            error,
            ImportError::ExpansionLimitExceeded {
                kind: "output entity",
                limit: 1
            }
        ));
    }

    #[test]
    fn skips_non_finite_geometry_before_generating_entities() {
        let mut invalid_text = test_text("invalid", [0.0, 0.0], 2.5);
        invalid_text.size_x = f64::NAN;
        let mut invalid_dimension_text = test_text("invalid", [5.0, 2.0], 2.5);
        invalid_dimension_text.angle = f64::INFINITY;
        let document = test_document(
            vec![
                JwwEntity::Line(Line {
                    base: EntityBase::default(),
                    start: [0.0, 0.0],
                    end: [10.0, 0.0],
                }),
                JwwEntity::Line(Line {
                    base: EntityBase::default(),
                    start: [0.0, 0.0],
                    end: [f64::NAN, 0.0],
                }),
                JwwEntity::Arc(test_arc(f64::INFINITY, 0.0, PI / 2.0, 0.0, 1.0, false)),
                JwwEntity::Text(invalid_text),
                JwwEntity::Dimension(Dimension {
                    base: EntityBase::default(),
                    line: Line {
                        base: EntityBase::default(),
                        start: [0.0, 0.0],
                        end: [10.0, 0.0],
                    },
                    text: invalid_dimension_text,
                }),
                JwwEntity::Block(test_block(f64::NAN, 0.0, 1.0, 1.0, 0.0, 1)),
            ],
            Vec::new(),
        );

        let converted = convert_ok(&document);

        assert_eq!(converted.entities.len(), 1);
        assert_eq!(
            converted
                .warnings
                .iter()
                .filter(|warning| warning.code == "geometry_skipped")
                .count(),
            5
        );
    }

    #[test]
    fn converts_point_records() {
        let document = test_document(
            vec![JwwEntity::Point(Point {
                base: EntityBase::default(),
                at: [10.0, 20.0],
                temporary: false,
                marker_code: None,
                rotation: 0.0,
                scale: 1.0,
            })],
            Vec::new(),
        );

        let converted = convert_ok(&document);

        assert_eq!(converted.entities.len(), 1);
        assert!(converted.entities[0].contains("\"type\":\"point\""));
        assert!(converted.warnings.is_empty());
    }

    #[test]
    fn preserves_unknown_point_marker_with_warning() {
        let document = test_document(
            vec![JwwEntity::Point(Point {
                base: EntityBase::default(),
                at: [10.0, 20.0],
                temporary: false,
                marker_code: Some(999),
                rotation: 0.0,
                scale: 1.0,
            })],
            Vec::new(),
        );

        let converted = convert_ok(&document);

        assert!(converted.entities[0].contains("\"marker_code\":999"));
        assert!(converted.warnings.iter().any(|warning| {
            warning.code == "unsupported_marker" && warning.record_type == "CDataTen"
        }));
    }

    #[test]
    fn converts_polygon_and_curve_solid_records() {
        let document = test_document(
            vec![
                JwwEntity::Solid(Solid {
                    base: EntityBase::default(),
                    points: [[0.0, 0.0], [10.0, 0.0], [10.0, 5.0], [0.0, 5.0]],
                    color: Some(1),
                }),
                JwwEntity::CircleSolid(CircleSolid {
                    base: EntityBase::default(),
                    center: [20.0, 20.0],
                    radius: 10.0,
                    flatness: 0.5,
                    tilt_rad: PI / 4.0,
                    start_rad: 0.0,
                    sweep_rad: PI,
                    solid_param: 2.0,
                    color: Some(2),
                }),
            ],
            Vec::new(),
        );

        let converted = convert_ok(&document);
        let values = entity_values(&converted.entities);

        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["type"], "solid");
        assert_eq!(values[0]["points"].as_array().map(Vec::len), Some(4));
        assert_eq!(values[1]["type"], "curve_solid");
        assert_eq!(values[1]["flatness"], 0.5);
        assert!((values[1]["rotation_deg"].as_f64().expect("rotation") - 45.0).abs() < 1e-9);
        assert!((values[1]["end_deg"].as_f64().expect("end angle") - 180.0).abs() < 1e-9);
        assert!(converted.warnings.is_empty());
    }

    #[test]
    fn skips_tiny_line_with_geometry_warning() {
        let document = test_document(
            vec![
                JwwEntity::Line(Line {
                    base: EntityBase::default(),
                    start: [0.0, 0.0],
                    end: [0.0005, 0.0],
                }),
                JwwEntity::Line(Line {
                    base: EntityBase::default(),
                    start: [0.0, 0.0],
                    end: [1.0, 0.0],
                }),
            ],
            Vec::new(),
        );

        let converted = convert_ok(&document);
        let values = entity_values(&converted.entities);

        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["type"], "line");
        assert!(converted.warnings.iter().any(|warning| {
            warning.code == "geometry_skipped" && warning.record_type == "CDataSen"
        }));
    }

    #[test]
    fn skips_invalid_curve_geometry_with_warning() {
        let document = test_document(
            vec![
                JwwEntity::Arc(Arc {
                    base: EntityBase::default(),
                    center: [0.0, 0.0],
                    radius: 0.0005,
                    start_rad: 0.0,
                    sweep_rad: PI / 2.0,
                    tilt_rad: 0.0,
                    flatness: 1.0,
                    is_full_circle: true,
                }),
                JwwEntity::Arc(Arc {
                    base: EntityBase::default(),
                    center: [0.0, 0.0],
                    radius: 10.0,
                    start_rad: 0.0,
                    sweep_rad: 5e-10,
                    tilt_rad: 0.0,
                    flatness: 1.0,
                    is_full_circle: false,
                }),
            ],
            Vec::new(),
        );

        let converted = convert_ok(&document);

        assert!(converted.entities.is_empty());
        assert_eq!(
            converted
                .warnings
                .iter()
                .filter(|warning| warning.code == "geometry_skipped"
                    && warning.record_type == "CDataEnko")
                .count(),
            2
        );
    }

    #[test]
    fn converts_jww_radians_and_ellipse_parameters() {
        let document = test_document(
            vec![
                JwwEntity::Arc(test_arc(10.0, 0.0, PI / 2.0, 0.0, 1.0, false)),
                JwwEntity::Arc(test_arc(20.0, 0.0, PI, PI / 4.0, 0.5, false)),
                JwwEntity::Arc(test_arc(30.0, 0.0, 0.0, PI / 6.0, 0.5, true)),
            ],
            Vec::new(),
        );

        let values = entity_values(&convert_ok(&document).entities);

        assert_eq!(values[0]["type"], "arc");
        assert_eq!(values[0]["start_deg"], 0.0);
        assert_eq!(values[0]["end_deg"], 90.0);
        assert_eq!(values[1]["type"], "ellipse");
        assert_eq!(values[1]["radius_x"], 20.0);
        assert_eq!(values[1]["radius_y"], 10.0);
        assert!(
            (values[1]["rotation_deg"]
                .as_f64()
                .expect("rotation should be numeric")
                - 45.0)
                .abs()
                < 1e-9
        );
        assert_eq!(values[1]["end_deg"], 180.0);
        assert_eq!(values[2]["type"], "ellipse");
        assert!(
            (values[2]["rotation_deg"]
                .as_f64()
                .expect("rotation should be numeric")
                - 30.0)
                .abs()
                < 1e-9
        );
        assert_eq!(values[2]["end_deg"], 360.0);
    }

    #[test]
    fn skips_zero_length_dimension_with_warning() {
        let document = test_document(
            vec![JwwEntity::Dimension(Dimension {
                base: EntityBase::default(),
                line: Line {
                    base: EntityBase::default(),
                    start: [0.0, 0.0],
                    end: [0.0005, 0.0],
                },
                text: Text {
                    base: EntityBase::default(),
                    start: [0.0, 10.0],
                    size_x: 2.5,
                    size_y: 2.5,
                    spacing: 0.0,
                    angle: 0.0,
                    mirror_y: false,
                    font_name: "Hiragino Sans".to_owned(),
                    content: "0".to_owned(),
                },
            })],
            Vec::new(),
        );

        let converted = convert_ok(&document);

        assert!(converted.entities.is_empty());
        assert!(converted.warnings.iter().any(|warning| {
            warning.code == "geometry_skipped" && warning.record_type == "CDataSunpou"
        }));
    }

    #[test]
    fn expands_block_definition_entities_with_transform() {
        let document = test_document(
            vec![JwwEntity::Block(test_block(10.0, 20.0, 2.0, 2.0, 0.0, 1))],
            vec![BlockDef {
                number: 1,
                name: "BED".to_owned(),
                entities: vec![
                    JwwEntity::Line(Line {
                        base: EntityBase::default(),
                        start: [0.0, 0.0],
                        end: [3.0, 0.0],
                    }),
                    JwwEntity::Text(Text {
                        base: EntityBase::default(),
                        start: [1.0, 1.0],
                        size_x: 2.5,
                        size_y: 2.5,
                        spacing: 0.0,
                        angle: 0.0,
                        mirror_y: false,
                        font_name: "Hiragino Sans".to_owned(),
                        content: "B".to_owned(),
                    }),
                ],
            }],
        );

        let converted = convert_ok(&document);
        let values = entity_values(&converted.entities);

        assert_eq!(values.len(), 2);
        assert_eq!(values[0]["type"], "line");
        assert_eq!(values[0]["p1"], json!([10.0, 20.0]));
        assert_eq!(values[0]["p2"], json!([16.0, 20.0]));
        assert_eq!(values[1]["type"], "text");
        assert_eq!(values[1]["at"], json!([12.0, 22.0]));
        assert_eq!(values[1]["style"], "jww_text_h5_w5_s0");
        assert!(
            converted
                .warnings
                .iter()
                .all(|warning| warning.record_type != "CDataBlock")
        );
    }

    #[test]
    fn creates_text_styles_from_jww_text_height() {
        let document = test_document(
            vec![
                JwwEntity::Text(test_text("small", [0.0, 0.0], 2.5)),
                JwwEntity::Text(test_text("large", [10.0, 0.0], 5.0)),
            ],
            Vec::new(),
        );

        let converted = convert_ok(&document);
        let values = entity_values(&converted.entities);
        let styles = styles_toml(&converted);

        assert_eq!(values[0]["style"], "jww_text_h2_5_w2_5_s0");
        assert_eq!(values[1]["style"], "jww_text_h5_w5_s0");
        assert!(styles.contains("[text_styles.jww_text_h2_5_w2_5_s0]\n"));
        assert!(styles.contains("height = 2.5\n"));
        assert!(styles.contains("width = 2.5\n"));
        assert!(styles.contains("spacing = 0\n"));
        assert!(styles.contains("[text_styles.jww_text_h5_w5_s0]\n"));
        assert!(styles.contains("height = 5\n"));
    }

    #[test]
    fn separates_text_styles_by_width_and_spacing() {
        let mut narrow = test_text("same", [0.0, 0.0], 2.5);
        narrow.size_x = 1.25;
        narrow.spacing = 0.0;
        let mut spaced = test_text("same", [10.0, 0.0], 2.5);
        spaced.size_x = 1.25;
        spaced.spacing = 0.5;

        let converted = convert_ok(&test_document(
            vec![JwwEntity::Text(narrow), JwwEntity::Text(spaced)],
            Vec::new(),
        ));
        let values = entity_values(&converted.entities);
        let styles = styles_toml(&converted);

        assert_eq!(values[0]["style"], "jww_text_h2_5_w1_25_s0");
        assert_eq!(values[1]["style"], "jww_text_h2_5_w1_25_s0_5");
        assert!(styles.contains("width = 1.25\n"));
        assert!(styles.contains("spacing = 0.5\n"));
    }

    #[test]
    fn creates_dimension_style_from_dimension_text_height() {
        let document = test_document(
            vec![JwwEntity::Dimension(Dimension {
                base: EntityBase::default(),
                line: Line {
                    base: EntityBase::default(),
                    start: [0.0, 0.0],
                    end: [100.0, 0.0],
                },
                text: test_text("100", [50.0, 10.0], 3.5),
            })],
            Vec::new(),
        );

        let converted = convert_ok(&document);
        let values = entity_values(&converted.entities);
        let styles = styles_toml(&converted);

        assert_eq!(values[0]["style"], "jww_dimension_h3_5_w3_5_s0");
        assert!(styles.contains("[text_styles.jww_text_h3_5_w3_5_s0]\n"));
        assert!(styles.contains("[dimension_styles.jww_dimension_h3_5_w3_5_s0]\n"));
        assert!(styles.contains("text_style = \"jww_text_h3_5_w3_5_s0\"\n"));
    }

    #[test]
    fn skips_tiny_text_height_with_geometry_warning() {
        let document = test_document(
            vec![
                JwwEntity::Text(test_text("tiny", [0.0, 0.0], 0.0005)),
                JwwEntity::Dimension(Dimension {
                    base: EntityBase::default(),
                    line: Line {
                        base: EntityBase::default(),
                        start: [0.0, 0.0],
                        end: [100.0, 0.0],
                    },
                    text: test_text("100", [50.0, 10.0], 0.0005),
                }),
            ],
            Vec::new(),
        );

        let converted = convert_ok(&document);

        assert!(converted.entities.is_empty());
        assert!(converted.warnings.iter().any(|warning| {
            warning.code == "geometry_skipped" && warning.record_type == "CDataMoji"
        }));
        assert!(converted.warnings.iter().any(|warning| {
            warning.code == "geometry_skipped" && warning.record_type == "CDataSunpou"
        }));
    }

    #[test]
    fn maps_jww_paper_size_and_single_layer_group_scale() {
        let mut warnings = Vec::new();
        let mut document = test_document(
            vec![JwwEntity::Line(Line {
                base: EntityBase::default(),
                start: [0.0, 0.0],
                end: [100.0, 0.0],
            })],
            Vec::new(),
        );
        document.header.paper_size = 2;
        document.header.layer_groups[0].scale = 50.0;
        let converted = convert_ok(&document);

        assert_eq!(sheet_paper(&document, &mut warnings), "A2");
        let (scale, warning) = sheet_scale(&document, &converted);
        assert_eq!(scale, "1/50");
        assert!(warning.is_none());
        assert!(warnings.is_empty());
    }

    #[test]
    fn preserves_named_or_non_default_empty_layers() {
        let mut document = test_document(
            vec![JwwEntity::Line(Line {
                base: EntityBase::default(),
                start: [0.0, 0.0],
                end: [100.0, 0.0],
            })],
            Vec::new(),
        );
        document.header.layer_groups[2].name = "Equipment".to_owned();
        document.header.layer_groups[2].layers[3].name = "Spare".to_owned();
        document.header.layer_groups[2].layers[3].state = 1;
        let converted = convert_ok(&document);
        let layers = layers_toml(&converted, &document);

        assert!(converted.layer_bases.contains_key("jww_g2_l3"));
        assert!(layers.contains("[groups.jww_g2]"));
        assert!(layers.contains("[layers.\"jww_g2_l3\"]"));
        assert!(layers.contains("name = \"Spare\""));
        assert!(layers.contains("locked = true"));
    }

    #[test]
    fn warns_for_mixed_layer_group_scale() {
        let mut document = test_document(
            vec![
                JwwEntity::Line(Line {
                    base: EntityBase {
                        layer_group: 0,
                        ..EntityBase::default()
                    },
                    start: [0.0, 0.0],
                    end: [100.0, 0.0],
                }),
                JwwEntity::Line(Line {
                    base: EntityBase {
                        layer_group: 1,
                        ..EntityBase::default()
                    },
                    start: [0.0, 10.0],
                    end: [100.0, 10.0],
                }),
            ],
            Vec::new(),
        );
        document.header.layer_groups[0].scale = 50.0;
        document.header.layer_groups[1].scale = 100.0;
        let converted = convert_ok(&document);

        let (scale, warning) = sheet_scale(&document, &converted);

        assert_eq!(scale, "1/1");
        assert!(matches!(
            warning,
            Some(ImportWarning { code, .. }) if code == "mixed_layer_group_scale"
        ));
    }

    #[test]
    fn expands_nested_blocks() {
        let document = test_document(
            vec![JwwEntity::Block(test_block(10.0, 0.0, 1.0, 1.0, 0.0, 1))],
            vec![
                BlockDef {
                    number: 1,
                    name: "OUTER".to_owned(),
                    entities: vec![JwwEntity::Block(test_block(5.0, 0.0, 1.0, 1.0, 0.0, 2))],
                },
                BlockDef {
                    number: 2,
                    name: "INNER".to_owned(),
                    entities: vec![JwwEntity::Line(Line {
                        base: EntityBase::default(),
                        start: [0.0, 0.0],
                        end: [1.0, 0.0],
                    })],
                },
            ],
        );

        let converted = convert_ok(&document);
        let values = entity_values(&converted.entities);

        assert_eq!(values.len(), 1);
        assert_eq!(values[0]["p1"], json!([15.0, 0.0]));
        assert_eq!(values[0]["p2"], json!([16.0, 0.0]));
    }

    #[test]
    fn reports_unresolved_and_cyclic_blocks() {
        let unresolved = convert_ok(&test_document(
            vec![JwwEntity::Block(test_block(0.0, 0.0, 1.0, 1.0, 0.0, 9))],
            Vec::new(),
        ));
        assert!(
            unresolved
                .warnings
                .iter()
                .any(|warning| warning.code == "unresolved_block")
        );

        let cyclic = convert_ok(&test_document(
            vec![JwwEntity::Block(test_block(0.0, 0.0, 1.0, 1.0, 0.0, 1))],
            vec![BlockDef {
                number: 1,
                name: "LOOP".to_owned(),
                entities: vec![JwwEntity::Block(test_block(0.0, 0.0, 1.0, 1.0, 0.0, 1))],
            }],
        ));
        assert!(
            cyclic
                .warnings
                .iter()
                .any(|warning| warning.code == "block_cycle")
        );
    }

    #[test]
    fn skips_non_uniform_scaled_curves_in_blocks() {
        let document = test_document(
            vec![JwwEntity::Block(test_block(0.0, 0.0, 2.0, 1.0, 0.0, 1))],
            vec![BlockDef {
                number: 1,
                name: "CURVE".to_owned(),
                entities: vec![JwwEntity::Arc(Arc {
                    base: EntityBase::default(),
                    center: [0.0, 0.0],
                    radius: 10.0,
                    start_rad: 0.0,
                    sweep_rad: PI / 2.0,
                    tilt_rad: 0.0,
                    flatness: 1.0,
                    is_full_circle: true,
                })],
            }],
        );

        let converted = convert_ok(&document);

        assert!(converted.entities.is_empty());
        assert!(
            converted
                .warnings
                .iter()
                .any(|warning| warning.code == "unsupported_scaled_curve")
        );
    }

    #[test]
    fn reflects_arc_and_ellipse_sweep_in_uniform_block() {
        let document = test_document(
            vec![JwwEntity::Block(test_block(0.0, 0.0, -1.0, 1.0, 0.0, 1))],
            vec![BlockDef {
                number: 1,
                name: "REFLECTED".to_owned(),
                entities: vec![
                    JwwEntity::Arc(test_arc(10.0, 0.0, PI / 2.0, 0.0, 1.0, false)),
                    JwwEntity::Arc(test_arc(20.0, 0.0, PI / 2.0, 0.0, 0.5, false)),
                ],
            }],
        );

        let values = entity_values(&convert_ok(&document).entities);

        assert_eq!(values[0]["start_deg"], 180.0);
        assert_eq!(values[0]["end_deg"], 90.0);
        assert_eq!(values[1]["rotation_deg"], 180.0);
        assert_eq!(values[1]["start_deg"], 0.0);
        assert_eq!(values[1]["end_deg"], -90.0);
    }

    #[test]
    fn preserves_reflected_text_and_dimension_metadata() {
        let document = test_document(
            vec![JwwEntity::Block(test_block(0.0, 0.0, -1.0, 1.0, 0.0, 1))],
            vec![BlockDef {
                number: 1,
                name: "REFLECTED_TEXT".to_owned(),
                entities: vec![
                    JwwEntity::Text(test_text("mirror", [10.0, 5.0], 2.5)),
                    JwwEntity::Dimension(Dimension {
                        base: EntityBase::default(),
                        line: Line {
                            base: EntityBase::default(),
                            start: [0.0, 0.0],
                            end: [100.0, 0.0],
                        },
                        text: test_text("100", [50.0, 10.0], 2.5),
                    }),
                ],
            }],
        );

        let values = entity_values(&convert_ok(&document).entities);

        assert_eq!(values[0]["mirror_y"], true);
        assert!((values[0]["rotation_deg"].as_f64().expect("rotation") - 180.0).abs() < 1e-9);
        assert_eq!(values[1]["text_mirror_y"], true);
        assert!(
            (values[1]["text_rotation_deg"]
                .as_f64()
                .expect("dimension rotation")
                - 180.0)
                .abs()
                < 1e-9
        );
        assert_eq!(values[1]["offset"], -10.0);
    }

    #[test]
    fn rejects_sheared_curve_transform_as_non_uniform() {
        let transform = Transform2D {
            a: 1.0,
            b: 0.0,
            c: 0.6,
            d: 0.8,
            tx: 0.0,
            ty: 0.0,
        };

        assert!(!transform.is_uniform_for_curve());
    }

    #[test]
    fn parses_sample_fixture_and_imports_project() {
        let input =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/jww-fixtures/Test1.jww");
        let out = tempfile::tempdir().expect("tempdir should exist");
        let project = out.path().join("imported");
        let report = import_jww_file(&input, &project).expect("sample should import");

        assert!(report.supported_entities > 0);
        assert_eq!(report.project_path, project.to_string_lossy());
        assert!(
            fs::read_dir(out.path())
                .expect("import parent should be readable")
                .all(|entry| !entry
                    .expect("directory entry should be readable")
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".cad-jww-import-"))
        );
        let loaded = cad_model::load_project(&project).expect("imported project should load");
        let check = cad_check::check_project(&project);
        assert!(
            check.is_ok(),
            "imported project should check cleanly: {check:?}"
        );
        let svg = cad_render_svg::render_project_svg(&loaded).expect("svg should render");
        assert!(svg.contains("<svg"));

        let counts = entity_type_counts(
            &project
                .join("drawings")
                .join("test1")
                .join("entities.ndjson"),
        );
        insta::assert_snapshot!(
            format!(
                "supported={}\nwarnings={}\nline={}\narc={}\ncircle={}\ntext={}\npoint={}\ndimension={}",
                report.supported_entities,
                report.warnings.len(),
                counts.get("line").copied().unwrap_or_default(),
                counts.get("arc").copied().unwrap_or_default(),
                counts.get("circle").copied().unwrap_or_default(),
                counts.get("text").copied().unwrap_or_default(),
                counts.get("point").copied().unwrap_or_default(),
                counts.get("dimension").copied().unwrap_or_default()
            ),
            @r###"
supported=1686
warnings=2
line=1642
arc=4
circle=0
text=36
point=4
dimension=0
        "###
        );
    }

    #[test]
    fn refuses_existing_output_directory() {
        let input =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/jww-fixtures/Test1.jww");
        let out = tempfile::tempdir().expect("tempdir should exist");
        let error = import_jww_file(&input, out.path()).expect_err("existing dir should fail");
        assert!(matches!(error, ImportError::OutputExists { .. }));
    }

    #[test]
    fn no_replace_publish_preserves_existing_directory() {
        let parent = tempfile::tempdir().expect("tempdir should exist");
        let staging = parent.path().join("staging");
        let destination = parent.path().join("destination");
        fs::create_dir(&staging).expect("staging should be created");
        fs::create_dir(&destination).expect("destination should be created");
        fs::write(destination.join("sentinel.txt"), "keep").expect("sentinel should be written");

        let error =
            publish_project(&staging, &destination).expect_err("publish should not replace");

        assert!(matches!(error, ImportError::OutputExists { .. }));
        assert_eq!(
            fs::read_to_string(destination.join("sentinel.txt")).expect("sentinel should remain"),
            "keep"
        );
        assert!(staging.exists());
    }

    #[test]
    fn concurrent_import_publishes_exactly_one_project() {
        let input =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/jww-fixtures/Test1.jww");
        let parent = tempfile::tempdir().expect("tempdir should exist");
        let destination = parent.path().join("concurrent");
        let barrier = SyncArc::new(Barrier::new(2));
        let handles = (0..2)
            .map(|_| {
                let input = input.clone();
                let destination = destination.clone();
                let barrier = SyncArc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    import_jww_file(input, destination)
                })
            })
            .collect::<Vec<_>>();

        let results = handles
            .into_iter()
            .map(|handle| handle.join().expect("import thread should finish"))
            .collect::<Vec<_>>();

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(ImportError::OutputExists { .. })))
                .count(),
            1
        );
        cad_model::load_project(&destination).expect("published project should load");
    }

    #[test]
    fn non_finite_only_import_leaves_no_output_project() {
        let parent = tempfile::tempdir().expect("tempdir should exist");
        let input = parent.path().join("nan-line.jww");
        fs::write(&input, minimal_jww_with_line(f64::NAN)).expect("fixture should be written");
        let destination = parent.path().join("nan-line");

        let error = import_jww_file(&input, &destination).expect_err("empty import should fail");

        assert!(matches!(error, ImportError::EmptyImport));
        assert!(!destination.exists());
        assert!(
            fs::read_dir(parent.path())
                .expect("parent should be readable")
                .all(|entry| !entry
                    .expect("entry should be readable")
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".cad-jww-import-"))
        );
    }

    #[test]
    fn typed_entity_extents_include_curve_geometry() {
        let circle = vec![
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"circle","layer":"0-1","center":[0.0,0.0],"radius":5000.0}"#.to_owned(),
        ];
        let ellipse = vec![
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000001","type":"ellipse","layer":"0-1","center":[0.0,0.0],"radius_x":10.0,"radius_y":100.0,"rotation_deg":0.0,"start_deg":0.0,"end_deg":360.0}"#.to_owned(),
        ];
        let ellipse_arc = vec![
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000002","type":"ellipse","layer":"0-1","center":[0.0,0.0],"radius_x":10.0,"radius_y":100.0,"rotation_deg":0.0,"start_deg":0.0,"end_deg":90.0}"#.to_owned(),
        ];

        let circle_bbox = entity_extents(&circle)
            .expect("circle should parse")
            .expect("circle should have bbox");
        let ellipse_bbox = entity_extents(&ellipse)
            .expect("ellipse should parse")
            .expect("ellipse should have bbox");
        let arc_bbox = entity_extents(&ellipse_arc)
            .expect("ellipse arc should parse")
            .expect("ellipse arc should have bbox");

        assert_eq!(circle_bbox.min, [-5000.0, -5000.0]);
        assert_eq!(sheet_orientation(Some(ellipse_bbox)), "portrait");
        assert!(arc_bbox.min[0].abs() < 1e-9);
        assert!((arc_bbox.max[1] - 100.0).abs() < 1e-9);
        assert_eq!(sheet_origin(Some(circle_bbox)), [-6000.0, -6000.0]);
    }

    #[test]
    fn converts_dimension_record_from_minimal_jww() {
        let out = tempfile::tempdir().expect("tempdir should exist");
        let input = out.path().join("dimension.jww");
        fs::write(&input, minimal_jww_with_dimension()).expect("fixture should be written");
        let project = out.path().join("dimension");

        import_jww_file(&input, &project).expect("dimension fixture should import");

        let entities = fs::read_to_string(
            project
                .join("drawings")
                .join("dimension")
                .join("entities.ndjson"),
        )
        .expect("entities should be readable");
        assert!(entities.contains("\"type\":\"dimension\""));
        assert!(entities.contains("\"value\":\"1000\""));
    }

    #[test]
    fn imports_minimal_jww_block_definition_by_flattening_entities() {
        let out = tempfile::tempdir().expect("tempdir should exist");
        let input = out.path().join("block-line.jww");
        fs::write(&input, minimal_jww_with_block_line()).expect("fixture should be written");
        let project = out.path().join("block-line");

        let report = import_jww_file(&input, &project).expect("block fixture should import");

        assert_eq!(report.supported_entities, 1);
        let entities = entity_values_from_file(
            &project
                .join("drawings")
                .join("block-line")
                .join("entities.ndjson"),
        );
        assert_eq!(entities[0]["type"], "line");
        assert_eq!(entities[0]["p1"], json!([10.0, 20.0]));
        assert_eq!(entities[0]["p2"], json!([13.0, 20.0]));
        assert!(
            report
                .warnings
                .iter()
                .all(|warning| warning.code != "unresolved_block")
        );
    }

    #[test]
    fn malformed_block_child_is_fatal_and_leaves_no_project() {
        let out = tempfile::tempdir().expect("tempdir should exist");
        let input = out.path().join("broken-block.jww");
        let mut data = minimal_jww_with_block_line();
        data.pop();
        fs::write(&input, data).expect("fixture should be written");
        let project = out.path().join("broken-block");

        let error = import_jww_file(&input, &project).expect_err("broken block should fail");

        assert!(matches!(error, ImportError::UnexpectedEof(_)));
        assert!(!project.exists());
    }

    fn entity_type_counts(path: &Path) -> BTreeMap<String, usize> {
        fs::read_to_string(path)
            .expect("entities should be readable")
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("entity JSON"))
            .filter_map(|value| {
                value
                    .get("type")
                    .and_then(|value| value.as_str())
                    .map(str::to_owned)
            })
            .fold(BTreeMap::<String, usize>::new(), |mut counts, kind| {
                *counts.entry(kind).or_default() += 1;
                counts
            })
    }

    fn entity_values_from_file(path: &Path) -> Vec<serde_json::Value> {
        fs::read_to_string(path)
            .expect("entities should be readable")
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).expect("entity JSON"))
            .collect()
    }

    fn test_document(entities: Vec<JwwEntity>, block_defs: Vec<BlockDef>) -> JwwDocument {
        JwwDocument {
            header: JwwHeader {
                version: 600,
                memo: String::new(),
                paper_size: 0,
                write_layer_group: 0,
                layer_groups: std::array::from_fn(|group_index| LayerGroupHeader {
                    layers: std::array::from_fn(|layer_index| LayerHeader {
                        name: format!("{group_index:X}-{layer_index:X}"),
                        state: 2,
                        ..LayerHeader::default()
                    }),
                    name: format!("Group{group_index:X}"),
                    scale: 1.0,
                    state: 2,
                    write_layer: 0,
                    protect: 0,
                }),
            },
            entities,
            block_defs,
        }
    }

    fn test_block(
        ref_x: f64,
        ref_y: f64,
        scale_x: f64,
        scale_y: f64,
        rotation: f64,
        def_number: u32,
    ) -> Block {
        Block {
            base: EntityBase::default(),
            ref_x,
            ref_y,
            scale_x,
            scale_y,
            rotation,
            def_number,
        }
    }

    fn test_text(content: &str, start: [f64; 2], size_y: f64) -> Text {
        Text {
            base: EntityBase::default(),
            start,
            size_x: size_y,
            size_y,
            spacing: 0.0,
            angle: 0.0,
            mirror_y: false,
            font_name: "Hiragino Sans".to_owned(),
            content: content.to_owned(),
        }
    }

    fn test_arc(
        radius: f64,
        start_rad: f64,
        sweep_rad: f64,
        tilt_rad: f64,
        flatness: f64,
        is_full_circle: bool,
    ) -> Arc {
        Arc {
            base: EntityBase::default(),
            center: [0.0, 0.0],
            radius,
            start_rad,
            sweep_rad,
            tilt_rad,
            flatness,
            is_full_circle,
        }
    }

    fn entity_values(entities: &[String]) -> Vec<serde_json::Value> {
        entities
            .iter()
            .map(|entity| serde_json::from_str(entity).expect("entity JSON"))
            .collect()
    }

    fn convert_ok(document: &JwwDocument) -> ConvertedProject {
        convert_entities(document).expect("test document should convert")
    }

    fn minimal_jww_with_line(end_x: f64) -> Vec<u8> {
        let mut data = minimal_jww_header();
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&0xFFFFu16.to_le_bytes());
        data.extend_from_slice(&600u16.to_le_bytes());
        let class_name = b"CDataSen";
        data.extend_from_slice(&(class_name.len() as u16).to_le_bytes());
        data.extend_from_slice(class_name);
        append_entity_base(&mut data);
        data.extend_from_slice(&0.0f64.to_le_bytes());
        data.extend_from_slice(&0.0f64.to_le_bytes());
        data.extend_from_slice(&end_x.to_le_bytes());
        data.extend_from_slice(&0.0f64.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data
    }

    fn minimal_jww_header() -> Vec<u8> {
        let mut data = Vec::<u8>::new();
        data.extend_from_slice(b"JwwData.");
        data.extend_from_slice(&600u32.to_le_bytes());
        data.push(0);
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        for _ in 0..16 {
            data.extend_from_slice(&0u32.to_le_bytes());
            data.extend_from_slice(&0u32.to_le_bytes());
            data.extend_from_slice(&1.0f64.to_le_bytes());
            data.extend_from_slice(&0u32.to_le_bytes());
            for _ in 0..16 {
                data.extend_from_slice(&0u32.to_le_bytes());
                data.extend_from_slice(&0u32.to_le_bytes());
            }
        }
        data
    }

    fn minimal_jww_with_dimension() -> Vec<u8> {
        let mut data = Vec::<u8>::new();
        data.extend_from_slice(b"JwwData.");
        data.extend_from_slice(&600u32.to_le_bytes());
        data.push(0);
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        for _ in 0..16 {
            data.extend_from_slice(&0u32.to_le_bytes());
            data.extend_from_slice(&0u32.to_le_bytes());
            data.extend_from_slice(&1.0f64.to_le_bytes());
            data.extend_from_slice(&0u32.to_le_bytes());
            for _ in 0..16 {
                data.extend_from_slice(&0u32.to_le_bytes());
                data.extend_from_slice(&0u32.to_le_bytes());
            }
        }
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&0xFFFFu16.to_le_bytes());
        data.extend_from_slice(&600u16.to_le_bytes());
        let class_name = b"CDataSunpou";
        data.extend_from_slice(&(class_name.len() as u16).to_le_bytes());
        data.extend_from_slice(class_name);
        append_entity_base(&mut data);
        append_entity_base(&mut data);
        data.extend_from_slice(&0.0f64.to_le_bytes());
        data.extend_from_slice(&0.0f64.to_le_bytes());
        data.extend_from_slice(&1000.0f64.to_le_bytes());
        data.extend_from_slice(&0.0f64.to_le_bytes());
        append_entity_base(&mut data);
        data.extend_from_slice(&0.0f64.to_le_bytes());
        data.extend_from_slice(&200.0f64.to_le_bytes());
        data.extend_from_slice(&0.0f64.to_le_bytes());
        data.extend_from_slice(&200.0f64.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&1.0f64.to_le_bytes());
        data.extend_from_slice(&1.0f64.to_le_bytes());
        data.extend_from_slice(&0.0f64.to_le_bytes());
        data.extend_from_slice(&0.0f64.to_le_bytes());
        data.push(0);
        data.push(4);
        data.write_all(b"1000").unwrap();
        data.extend_from_slice(&0u16.to_le_bytes());
        for _ in 0..2 {
            append_entity_base(&mut data);
            data.extend_from_slice(&0.0f64.to_le_bytes());
            data.extend_from_slice(&0.0f64.to_le_bytes());
            data.extend_from_slice(&0.0f64.to_le_bytes());
            data.extend_from_slice(&0.0f64.to_le_bytes());
        }
        for _ in 0..4 {
            append_entity_base(&mut data);
            data.extend_from_slice(&0.0f64.to_le_bytes());
            data.extend_from_slice(&0.0f64.to_le_bytes());
            data.extend_from_slice(&0u32.to_le_bytes());
        }
        data.extend_from_slice(&0u32.to_le_bytes());
        data
    }

    fn minimal_jww_with_block_line() -> Vec<u8> {
        let mut data = Vec::<u8>::new();
        data.extend_from_slice(b"JwwData.");
        data.extend_from_slice(&600u32.to_le_bytes());
        data.push(0);
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        for _ in 0..16 {
            data.extend_from_slice(&0u32.to_le_bytes());
            data.extend_from_slice(&0u32.to_le_bytes());
            data.extend_from_slice(&1.0f64.to_le_bytes());
            data.extend_from_slice(&0u32.to_le_bytes());
            for _ in 0..16 {
                data.extend_from_slice(&0u32.to_le_bytes());
                data.extend_from_slice(&0u32.to_le_bytes());
            }
        }

        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&0xFFFFu16.to_le_bytes());
        data.extend_from_slice(&600u16.to_le_bytes());
        let block_class = b"CDataBlock";
        data.extend_from_slice(&(block_class.len() as u16).to_le_bytes());
        data.extend_from_slice(block_class);
        append_entity_base(&mut data);
        data.extend_from_slice(&10.0f64.to_le_bytes());
        data.extend_from_slice(&20.0f64.to_le_bytes());
        data.extend_from_slice(&1.0f64.to_le_bytes());
        data.extend_from_slice(&1.0f64.to_le_bytes());
        data.extend_from_slice(&0.0f64.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());

        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&0xFFFFu16.to_le_bytes());
        data.extend_from_slice(&600u16.to_le_bytes());
        let list_class = b"CDataList";
        data.extend_from_slice(&(list_class.len() as u16).to_le_bytes());
        data.extend_from_slice(list_class);
        append_entity_base(&mut data);
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&1u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.push(3);
        data.write_all(b"BLK").unwrap();

        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&0xFFFFu16.to_le_bytes());
        data.extend_from_slice(&600u16.to_le_bytes());
        let line_class = b"CDataSen";
        data.extend_from_slice(&(line_class.len() as u16).to_le_bytes());
        data.extend_from_slice(line_class);
        append_entity_base(&mut data);
        data.extend_from_slice(&0.0f64.to_le_bytes());
        data.extend_from_slice(&0.0f64.to_le_bytes());
        data.extend_from_slice(&3.0f64.to_le_bytes());
        data.extend_from_slice(&0.0f64.to_le_bytes());
        data
    }

    fn append_entity_base(data: &mut Vec<u8>) {
        data.extend_from_slice(&0u32.to_le_bytes());
        data.push(1);
        data.extend_from_slice(&1u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
        data.extend_from_slice(&0u16.to_le_bytes());
    }
}
