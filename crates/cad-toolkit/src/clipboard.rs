//! Portable canonical parts with dependency-aware, reviewed project publication.
use crate::{Result, ToolkitError};
use cad_model::{Entity, EntityId, Point, ProjectSource, SourceFileRevision};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Path, PathBuf},
};

fn invalid(error: impl ToString) -> ToolkitError {
    ToolkitError::Invalid(error.to_string())
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DimensionCopyPolicy {
    IncludeReferences,
    DetachExternal,
    RejectExternal,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClipboardBlock {
    pub config: cad_model::BlockDefinitionConfig,
    pub entities: Vec<Entity>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClipboardDocument {
    pub schema_version: String,
    pub source_project: cad_model::ProjectConfig,
    pub source_drawing: String,
    pub source_snapshot_blake3: String,
    pub base_point: Point,
    pub requested_ids: Vec<String>,
    pub expanded_ids: Vec<String>,
    pub warnings: Vec<String>,
    pub layouts: cad_model::LayoutsConfig,
    pub layers: cad_model::LayerRules,
    pub styles: cad_model::StyleRules,
    pub blocks: BTreeMap<String, ClipboardBlock>,
    pub entities: Vec<Entity>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PasteRequest {
    pub drawing: String,
    pub at: Point,
    #[serde(default)]
    pub rotation_deg: f64,
    #[serde(default = "unit_scale")]
    pub scale: f64,
}
fn unit_scale() -> f64 {
    1.
}
#[derive(Debug, Default, Serialize)]
pub struct DefinitionMappings {
    pub groups: BTreeMap<String, String>,
    pub layers: BTreeMap<String, String>,
    pub colors: BTreeMap<String, String>,
    pub line_types: BTreeMap<String, String>,
    pub pens: BTreeMap<String, String>,
    pub text_styles: BTreeMap<String, String>,
    pub dimension_styles: BTreeMap<String, String>,
    pub blocks: BTreeMap<String, String>,
    pub entities: BTreeMap<String, String>,
}
#[derive(Debug, Serialize)]
pub struct PasteReport {
    pub schema_version: String,
    pub plan_hash: String,
    pub status: String,
    pub drawing: String,
    pub placement: PasteRequest,
    pub clipboard_blake3: String,
    pub source_project: String,
    pub source_drawing: String,
    pub source_snapshot_blake3: String,
    pub target_files: Vec<SourceFileRevision>,
    pub entity_ids: Vec<String>,
    pub changed_files: Vec<String>,
    pub mappings: DefinitionMappings,
    pub warnings: Vec<String>,
    pub cad_check: cad_check::CheckReport,
    pub diff: cad_diff::DiffReport,
    pub history_id: Option<String>,
}
pub struct PastePlan {
    pub report: PasteReport,
    pub preview_svg: String,
    root: PathBuf,
    request: cad_edit::source_replacements::SourceReplacementRequest,
    sealed_report: Vec<u8>,
    sealed_svg: String,
}

fn refs(entity: &Entity) -> Vec<String> {
    cad_model::dimension_anchors_mut(&mut entity.clone())
        .into_iter()
        .filter_map(|anchor| match anchor {
            cad_model::DimensionAnchor::Entity { entity_id, .. } => Some(entity_id.as_str().into()),
            _ => None,
        })
        .collect()
}
fn finite(point: Point) -> bool {
    point.iter().all(|number| number.is_finite())
}
fn collect_blocks(
    project: &ProjectSource,
    entity: &Entity,
    blocks: &mut BTreeSet<String>,
) -> Result<()> {
    if let Entity::BlockRef { block, .. } = entity
        && blocks.insert(block.clone())
    {
        let definition = project
            .blocks
            .get(block)
            .ok_or_else(|| invalid("Missing source block"))?;
        for record in &definition.entities {
            collect_blocks(project, &record.entity, blocks)?;
        }
    }
    Ok(())
}

pub fn capture(
    root: &Path,
    drawing: &str,
    ids: &[String],
    base_point: Point,
    policy: DimensionCopyPolicy,
) -> Result<ClipboardDocument> {
    if ids.is_empty() || ids.len() > 100_000 || !finite(base_point) {
        return Err(invalid("Choose entities and a finite clipboard base point"));
    }
    let before = cad_model::source_manifest(root).map_err(invalid)?;
    let project = cad_model::load_project(root).map_err(invalid)?;
    if !cad_check::check_loaded_project(&project).is_ok() {
        return Err(invalid("Copy source failed CAD validation"));
    }
    let source = project
        .drawings
        .iter()
        .find(|source| source.name == drawing)
        .ok_or_else(|| invalid("Copy drawing is missing"))?;
    let available: BTreeMap<_, _> = source
        .entities
        .iter()
        .map(|record| (record.entity.id().as_str().to_owned(), &record.entity))
        .collect();
    let mut selected: BTreeSet<_> = ids.iter().cloned().collect();
    if selected.len() != ids.len() || selected.iter().any(|id| !available.contains_key(id)) {
        return Err(invalid("Copy selection has duplicate or missing entities"));
    }
    if matches!(policy, DimensionCopyPolicy::IncludeReferences) {
        loop {
            let mut next = selected.clone();
            for id in &selected {
                for reference in refs(available[id]) {
                    if !available.contains_key(&reference) {
                        return Err(invalid(
                            "Dimension reference is outside the source drawing; choose detach",
                        ));
                    }
                    next.insert(reference);
                }
            }
            if next == selected {
                break;
            }
            selected = next;
        }
    }
    let mut entities: Vec<_> = source
        .entities
        .iter()
        .filter(|record| selected.contains(record.entity.id().as_str()))
        .map(|record| record.entity.clone())
        .collect();
    let mut warnings = vec![];
    if selected.len() > ids.len() {
        warnings.push(format!(
            "Included {} referenced geometry entities",
            selected.len() - ids.len()
        ));
    }
    for entity in &mut entities {
        if refs(entity).iter().any(|id| !selected.contains(id)) {
            if !matches!(policy, DimensionCopyPolicy::DetachExternal) {
                return Err(invalid(
                    "Dimension references geometry outside the selection; choose include or detach",
                ));
            }
            warnings.push(format!(
                "Detached external dimension references: {}",
                entity.id().as_str()
            ));
            cad_model::detach_dimension(&project, entity).map_err(invalid)?;
        }
    }
    let mut blocks = BTreeSet::new();
    for entity in &entities {
        collect_blocks(&project, entity, &mut blocks)?;
    }
    let mut document = ClipboardDocument {
        schema_version: "cad-clipboard/1".into(),
        source_project: project.project.clone(),
        source_drawing: drawing.into(),
        source_snapshot_blake3: blake3::hash(&serde_json::to_vec(&before)?)
            .to_hex()
            .to_string(),
        base_point,
        requested_ids: ids.to_vec(),
        expanded_ids: selected.into_iter().collect(),
        warnings,
        layouts: source.layouts.clone(),
        layers: project.layers.clone(),
        styles: project.styles.clone(),
        blocks: blocks
            .iter()
            .map(|id| {
                let block = &project.blocks[id];
                (
                    id.clone(),
                    ClipboardBlock {
                        config: block.config.clone(),
                        entities: block
                            .entities
                            .iter()
                            .map(|record| record.entity.clone())
                            .collect(),
                    },
                )
            })
            .collect(),
        entities,
    };
    if document
        .layouts
        .layouts
        .values()
        .any(|layout| !layout.viewports.is_empty())
    {
        document.warnings.push("Source sheet viewports are metadata only; portable part previews show model geometry and pasting does not copy layouts.".into());
    }
    trim_definitions(&mut document)?;
    let captured_ids: BTreeSet<_> = document
        .entities
        .iter()
        .chain(document.blocks.values().flat_map(|block| &block.entities))
        .map(|entity| entity.id().as_str().to_owned())
        .collect();
    for entity in document
        .blocks
        .values_mut()
        .flat_map(|block| &mut block.entities)
    {
        if refs(entity).iter().any(|id| !captured_ids.contains(id)) {
            if !matches!(policy, DimensionCopyPolicy::DetachExternal) {
                return Err(invalid(
                    "Block dimension references geometry outside the copied part; choose detach",
                ));
            }
            document.warnings.push(format!(
                "Detached external block dimension references: {}",
                entity.id().as_str()
            ));
            cad_model::detach_dimension(&project, entity).map_err(invalid)?;
        }
    }
    validate_document(&document)?;
    if cad_model::source_manifest(root).map_err(invalid)? != before {
        return Err(invalid("Source changed during copy"));
    }
    Ok(document)
}

fn trim_definitions(document: &mut ClipboardDocument) -> Result<()> {
    let mut layers = BTreeSet::new();
    let mut groups = BTreeSet::new();
    let mut colors = BTreeSet::new();
    let mut lines = BTreeSet::new();
    let mut pens = BTreeSet::new();
    let mut texts = BTreeSet::new();
    let mut dimensions = BTreeSet::new();
    for entity in document
        .entities
        .iter()
        .chain(document.blocks.values().flat_map(|block| &block.entities))
    {
        layers.insert(entity.layer().to_owned());
        if let Some(pen) = entity.pen() {
            pens.insert(pen.to_owned());
        }
        match entity {
            Entity::Text { style, .. } => {
                texts.insert(style.clone());
            }
            Entity::Dimension { style, .. } => {
                dimensions.insert(style.clone());
            }
            Entity::Solid { fill, .. }
            | Entity::CurveSolid { fill, .. }
            | Entity::Hatch {
                fill: Some(fill), ..
            } => {
                colors.insert(fill.clone());
            }
            _ => {}
        }
    }
    for id in &layers {
        let layer = document
            .layers
            .layers
            .get(id)
            .ok_or_else(|| invalid("Missing clipboard layer"))?;
        colors.insert(layer.color.clone());
        lines.insert(layer.line_type.clone());
        if let Some(group) = &layer.group {
            groups.insert(group.clone());
        }
    }
    for id in &pens {
        let pen = document
            .styles
            .pens
            .get(id)
            .ok_or_else(|| invalid("Missing clipboard pen"))?;
        colors.insert(pen.color.clone());
        lines.insert(pen.line_type.clone());
    }
    for id in &dimensions {
        texts.insert(
            document
                .styles
                .dimension_styles
                .get(id)
                .ok_or_else(|| invalid("Missing clipboard dimension style"))?
                .text_style
                .clone(),
        );
    }
    document.layers.active_layer = None;
    document.layers.layers.retain(|id, _| layers.contains(id));
    document.layers.groups.retain(|id, _| groups.contains(id));
    document.styles.colors.retain(|id, _| colors.contains(id));
    document
        .styles
        .line_types
        .retain(|id, _| lines.contains(id));
    document.styles.pens.retain(|id, _| pens.contains(id));
    document
        .styles
        .text_styles
        .retain(|id, _| texts.contains(id));
    document
        .styles
        .dimension_styles
        .retain(|id, _| dimensions.contains(id));
    Ok(())
}

fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::create_dir_all(
        path.parent()
            .ok_or_else(|| invalid("Source parent is missing"))?,
    )
    .map_err(invalid)?;
    fs::write(path, bytes).map_err(invalid)
}
fn entity_bytes(entities: &[Entity]) -> Result<Vec<u8>> {
    let mut bytes = vec![];
    for entity in entities {
        bytes.extend(serde_json::to_vec(entity)?);
        bytes.push(b'\n');
    }
    Ok(bytes)
}
pub fn read_document(path: &Path) -> Result<ClipboardDocument> {
    let mut bytes = vec![];
    fs::File::open(path)
        .map_err(invalid)?
        .take(64 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(invalid)?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(invalid("CAD part exceeds 64 MiB"));
    }
    let document = serde_json::from_slice(&bytes)?;
    validate_document(&document)?;
    Ok(document)
}
pub fn serialize_document(document: &ClipboardDocument) -> Result<Vec<u8>> {
    validate_document(document)?;
    let bytes = serde_json::to_vec_pretty(document)?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err(invalid("CAD part exceeds 64 MiB"));
    }
    Ok(bytes)
}
pub fn validate_document(document: &ClipboardDocument) -> Result<()> {
    checked_document_project(document).map(|_| ())
}

pub fn preview_document(document: &ClipboardDocument) -> Result<String> {
    let project = checked_document_project(document)?;
    if document.entities.len()
        + document
            .blocks
            .values()
            .map(|b| b.entities.len())
            .sum::<usize>()
        > 10_000
    {
        return Err(invalid(
            "Part thumbnail exceeds the 10000-entity preview limit",
        ));
    }
    fn budget(
        project: &ProjectSource,
        entity: &Entity,
        remaining: &mut usize,
        depth: usize,
    ) -> Result<()> {
        if depth > 32 {
            return Err(invalid("Part thumbnail block depth exceeds 32"));
        }
        let cost = match entity {
            Entity::Polyline { points, .. } | Entity::Solid { points, .. } => points.len().max(1),
            Entity::Text { value, .. } => value.len().max(1),
            Entity::Hatch {
                loops,
                pattern,
                scale,
                ..
            } => {
                let points: Vec<_> = loops.iter().flatten().copied().collect();
                let extent = cad_model::BBox::from_points(&points)
                    .ok_or_else(|| invalid("Part hatch has no bounds"))?;
                let families = if pattern == "solid" {
                    0.
                } else if pattern == "cross" {
                    2.
                } else {
                    1.
                };
                let estimated = points.len() as f64
                    * (1. + families * (extent.width().hypot(extent.height()) / scale + 3.));
                if !estimated.is_finite() || estimated > 10_000. {
                    return Err(invalid(
                        "Part thumbnail hatch expansion exceeds the preview budget",
                    ));
                }
                estimated.ceil() as usize
            }
            Entity::Dimension { .. } => 50,
            _ => 1,
        };
        *remaining = remaining
            .checked_sub(cost)
            .ok_or_else(|| invalid("Part thumbnail exceeds the 10000-unit expansion budget"))?;
        if let Entity::BlockRef { block, .. } = entity {
            for record in &project
                .blocks
                .get(block)
                .ok_or_else(|| invalid("Part thumbnail block is missing"))?
                .entities
            {
                budget(project, &record.entity, remaining, depth + 1)?;
            }
        }
        Ok(())
    }
    let mut remaining = 10_000;
    for entity in &document.entities {
        budget(&project, entity, &mut remaining, 0)?;
    }
    let mut svg = cad_render_svg::render_project_svg(&project).map_err(invalid)?;
    if svg.len() > 2 * 1024 * 1024 {
        return Err(invalid("Part thumbnail exceeds the 2 MiB preview limit"));
    }
    let bounds = document
        .entities
        .iter()
        .filter_map(|entity| cad_render_svg::render_entity_bbox(&project, entity))
        .reduce(|a, b| cad_model::BBox {
            min: [a.min[0].min(b.min[0]), a.min[1].min(b.min[1])],
            max: [a.max[0].max(b.max[0]), a.max[1].max(b.max[1])],
        });
    if let Some(bounds) = bounds {
        let padding =
            ((bounds.max[0] - bounds.min[0]).max(bounds.max[1] - bounds.min[1]) * 0.03).max(1.);
        if !padding.is_finite()
            || !(bounds.max[0] - bounds.min[0]).is_finite()
            || !(bounds.max[1] - bounds.min[1]).is_finite()
        {
            return Err(invalid("Part thumbnail extent overflow"));
        }
        let view = format!(
            "{} {} {} {}",
            bounds.min[0] - padding,
            -bounds.max[1] - padding,
            bounds.max[0] - bounds.min[0] + 2. * padding,
            bounds.max[1] - bounds.min[1] + 2. * padding
        );
        let start = svg
            .find("viewBox=\"")
            .ok_or_else(|| invalid("Part SVG viewBox is missing"))?
            + 9;
        let end = start
            + svg[start..]
                .find('"')
                .ok_or_else(|| invalid("Part SVG viewBox is invalid"))?;
        svg.replace_range(start..end, &view);
    }
    Ok(svg)
}

fn checked_document_project(document: &ClipboardDocument) -> Result<ProjectSource> {
    if document.schema_version != "cad-clipboard/1"
        || document.entities.is_empty()
        || document.entities.len() > 100_000
        || document.blocks.len() > 10_000
        || !finite(document.base_point)
    {
        return Err(invalid(
            "Invalid clipboard schema, entity count or base point",
        ));
    }
    if serde_json::to_vec(document)?.len() > 64 * 1024 * 1024 {
        return Err(invalid("CAD part exceeds 64 MiB"));
    }
    if document
        .blocks
        .values()
        .map(|block| block.entities.len())
        .sum::<usize>()
        > 100_000
    {
        return Err(invalid("Clipboard block entity limit exceeded"));
    }
    let directory = tempfile::tempdir().map_err(invalid)?;
    let root = directory.path();
    write(
        &root.join("cad.project.toml"),
        toml::to_string_pretty(&document.source_project)
            .map_err(invalid)?
            .as_bytes(),
    )?;
    write(
        &root.join("rules/layers.toml"),
        toml::to_string_pretty(&document.layers)
            .map_err(invalid)?
            .as_bytes(),
    )?;
    write(
        &root.join("rules/styles.toml"),
        toml::to_string_pretty(&document.styles)
            .map_err(invalid)?
            .as_bytes(),
    )?;
    let mut model_layouts = document.layouts.clone();
    for layout in model_layouts.layouts.values_mut() {
        layout.viewports.clear();
    }
    write(
        &root.join("drawings/part/layouts.toml"),
        toml::to_string_pretty(&model_layouts)
            .map_err(invalid)?
            .as_bytes(),
    )?;
    write(
        &root.join("drawings/part/entities.ndjson"),
        &entity_bytes(&document.entities)?,
    )?;
    for (id, block) in &document.blocks {
        if id.is_empty()
            || id.contains(['/', '\\'])
            || Path::new(id).components().count() != 1
            || !matches!(
                Path::new(id).components().next(),
                Some(std::path::Component::Normal(_))
            )
        {
            return Err(invalid("Unsafe clipboard block name"));
        }
        write(
            &root.join(format!("blocks/{id}/definition.toml")),
            toml::to_string_pretty(&block.config)
                .map_err(invalid)?
                .as_bytes(),
        )?;
        write(
            &root.join(format!("blocks/{id}/entities.ndjson")),
            &entity_bytes(&block.entities)?,
        )?;
    }
    let check = cad_check::check_project(root);
    if !check.is_ok() {
        return Err(invalid(format!(
            "Clipboard failed CAD validation: {}",
            serde_json::to_string(&check)?
        )));
    }
    cad_model::load_project(root).map_err(invalid)
}

fn import_map<T: Clone + PartialEq>(
    kind: &str,
    source: &BTreeMap<String, T>,
    target: &mut BTreeMap<String, T>,
    seed: &str,
) -> BTreeMap<String, String> {
    let mut mapping = BTreeMap::new();
    for (ordinal, (id, definition)) in source.iter().enumerate() {
        let same = target.get(id).is_some_and(|value| value == definition);
        let name = if same || !target.contains_key(id) {
            id.clone()
        } else if let Some((name, _)) = target.iter().find(|(_, value)| *value == definition) {
            name.clone()
        } else {
            let mut suffix = ordinal;
            loop {
                let name = format!("clip_{seed}_{kind}_{suffix}");
                if !target.contains_key(&name) {
                    break name;
                }
                suffix += 1;
            }
        };
        target.insert(name.clone(), definition.clone());
        mapping.insert(id.clone(), name);
    }
    mapping
}
fn mapped(map: &BTreeMap<String, String>, id: &str) -> Result<String> {
    map.get(id)
        .cloned()
        .ok_or_else(|| invalid(format!("Missing imported definition {id}")))
}
fn remap(
    entity: &Entity,
    mappings: &DefinitionMappings,
    replacements: &BTreeMap<EntityId, EntityId>,
    delta: Option<Point>,
) -> Result<Entity> {
    let mut entity = if let Some(delta) = delta {
        cad_edit::translate_entity(entity, delta).map_err(invalid)?
    } else {
        entity.clone()
    };
    cad_model::remap_dimension_references(&mut entity, replacements);
    let mut value = serde_json::to_value(&entity)?;
    value["id"] = mapped(&mappings.entities, entity.id().as_str())?.into();
    value["layer"] = mapped(&mappings.layers, entity.layer())?.into();
    if let Some(id) = entity.pen() {
        value["pen"] = mapped(&mappings.pens, id)?.into();
    }
    match &entity {
        Entity::Text { style, .. } => value["style"] = mapped(&mappings.text_styles, style)?.into(),
        Entity::Dimension { style, .. } => {
            value["style"] = mapped(&mappings.dimension_styles, style)?.into()
        }
        Entity::Solid { fill, .. }
        | Entity::CurveSolid { fill, .. }
        | Entity::Hatch {
            fill: Some(fill), ..
        } => value["fill"] = mapped(&mappings.colors, fill)?.into(),
        Entity::BlockRef { block, .. } => value["block"] = mapped(&mappings.blocks, block)?.into(),
        _ => {}
    }
    Ok(serde_json::from_value(value)?)
}

pub fn plan(
    root: &Path,
    document: &ClipboardDocument,
    request: &PasteRequest,
) -> Result<PastePlan> {
    validate_document(document)?;
    if !finite(request.at)
        || !request.rotation_deg.is_finite()
        || !request.scale.is_finite()
        || request.scale <= 0.
    {
        return Err(invalid(
            "Paste requires finite placement and angle, and a positive finite scale",
        ));
    }
    let root = fs::canonicalize(root).map_err(invalid)?;
    let expected = cad_model::source_manifest(&root).map_err(invalid)?;
    let original = cad_model::load_project(&root).map_err(invalid)?;
    if !cad_check::check_loaded_project(&original).is_ok() {
        return Err(invalid("Paste target failed CAD validation"));
    }
    if !original
        .drawings
        .iter()
        .any(|drawing| drawing.name == request.drawing)
    {
        return Err(invalid("Paste target drawing is missing"));
    }
    if let Some(state) = cad_model::jww_project_compatibility(&root).map_err(invalid)?
        && !(state.state == cad_model::JwwCompatibilityState::EditableLossless
            && state.original_verified
            && state.edit_capability == cad_model::JwwEditCapability::MappedV600)
    {
        return Err(invalid("jww_read_only: no verified editable mapping"));
    }
    let clipboard_hash = blake3::hash(&serde_json::to_vec(document)?)
        .to_hex()
        .to_string();
    let seed = blake3::hash(&serde_json::to_vec(&(&clipboard_hash, &expected, request))?)
        .to_hex()
        .to_string();
    let short = &seed[..12];
    let mut target = original.clone();
    let mut mappings = DefinitionMappings {
        colors: import_map(
            "color",
            &document.styles.colors,
            &mut target.styles.colors,
            short,
        ),
        line_types: import_map(
            "line",
            &document.styles.line_types,
            &mut target.styles.line_types,
            short,
        ),
        text_styles: import_map(
            "text",
            &document.styles.text_styles,
            &mut target.styles.text_styles,
            short,
        ),
        groups: import_map(
            "group",
            &document.layers.groups,
            &mut target.layers.groups,
            short,
        ),
        ..Default::default()
    };
    let mut pens = document.styles.pens.clone();
    for pen in pens.values_mut() {
        pen.color = mapped(&mappings.colors, &pen.color)?;
        pen.line_type = mapped(&mappings.line_types, &pen.line_type)?;
    }
    mappings.pens = import_map("pen", &pens, &mut target.styles.pens, short);
    let mut dimensions = document.styles.dimension_styles.clone();
    for style in dimensions.values_mut() {
        style.text_style = mapped(&mappings.text_styles, &style.text_style)?;
    }
    mappings.dimension_styles = import_map(
        "dimension",
        &dimensions,
        &mut target.styles.dimension_styles,
        short,
    );
    let mut layers = document.layers.layers.clone();
    for layer in layers.values_mut() {
        layer.color = mapped(&mappings.colors, &layer.color)?;
        layer.line_type = mapped(&mappings.line_types, &layer.line_type)?;
        layer.group = layer
            .group
            .as_ref()
            .map(|group| mapped(&mappings.groups, group))
            .transpose()?;
    }
    mappings.layers = import_map("layer", &layers, &mut target.layers.layers, short);
    let mut used_blocks: BTreeSet<_> = target.blocks.keys().cloned().collect();
    for (ordinal, id) in document.blocks.keys().enumerate() {
        let mut suffix = ordinal;
        loop {
            let name = format!("clip_{short}_{suffix}");
            if used_blocks.insert(name.clone()) {
                mappings.blocks.insert(id.clone(), name);
                break;
            }
            suffix += 1;
        }
    }
    let mut used_ids: BTreeSet<_> = target
        .drawings
        .iter()
        .flat_map(|drawing| drawing.entities.iter())
        .chain(
            target
                .blocks
                .values()
                .flat_map(|block| block.entities.iter()),
        )
        .map(|record| record.entity.id().as_str().to_owned())
        .collect();
    for entity in document
        .entities
        .iter()
        .chain(document.blocks.values().flat_map(|block| &block.entities))
    {
        let mut nonce = 0_u64;
        loop {
            let hash = blake3::hash(&serde_json::to_vec(&(&seed, entity.id().as_str(), nonce))?);
            let id = format!(
                "ent_{}",
                ulid::Ulid::from(u128::from_be_bytes(
                    hash.as_bytes()[..16].try_into().expect("hash prefix")
                ))
            );
            if used_ids.insert(id.clone()) {
                mappings.entities.insert(entity.id().as_str().into(), id);
                break;
            }
            nonce += 1;
        }
    }
    let replacements = mappings
        .entities
        .iter()
        .map(|(old, new)| {
            Ok((
                EntityId::parse(old).map_err(invalid)?,
                EntityId::parse(new).map_err(invalid)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let mut files = BTreeMap::new();
    if target.layers != original.layers {
        files.insert(
            "rules/layers.toml".into(),
            toml::to_string_pretty(&target.layers)
                .map_err(invalid)?
                .into_bytes(),
        );
    }
    if target.styles != original.styles {
        files.insert(
            "rules/styles.toml".into(),
            toml::to_string_pretty(&target.styles)
                .map_err(invalid)?
                .into_bytes(),
        );
    }
    for (id, block) in &document.blocks {
        let name = mapped(&mappings.blocks, id)?;
        let entities = block
            .entities
            .iter()
            .map(|entity| remap(entity, &mappings, &replacements, None))
            .collect::<Result<Vec<_>>>()?;
        files.insert(
            format!("blocks/{name}/definition.toml"),
            toml::to_string_pretty(&block.config)
                .map_err(invalid)?
                .into_bytes(),
        );
        files.insert(
            format!("blocks/{name}/entities.ndjson"),
            entity_bytes(&entities)?,
        );
    }
    let delta = [
        request.at[0] - document.base_point[0],
        request.at[1] - document.base_point[1],
    ];
    if !finite(delta) {
        return Err(invalid("Paste displacement overflow"));
    }
    let entities = document
        .entities
        .iter()
        .map(|entity| {
            let mut entity = remap(entity, &mappings, &replacements, Some(delta))?;
            if request.scale != 1. {
                entity =
                    cad_edit::scale_entity(&entity, request.at, request.scale).map_err(invalid)?;
            }
            if request.rotation_deg != 0. {
                entity = cad_edit::rotate_entity(&entity, request.at, request.rotation_deg)
                    .map_err(invalid)?;
            }
            Ok(entity)
        })
        .collect::<Result<Vec<_>>>()?;
    let path = format!("drawings/{}/entities.ndjson", request.drawing);
    let mut bytes = fs::read(root.join(&path)).map_err(invalid)?;
    let ending = if bytes.windows(2).any(|bytes| bytes == b"\r\n") {
        b"\r\n".as_slice()
    } else {
        b"\n".as_slice()
    };
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        bytes.extend(ending);
    }
    for entity in &entities {
        bytes.extend(serde_json::to_vec(entity)?);
        bytes.extend(ending);
    }
    files.insert(path, bytes);
    let directory = tempfile::tempdir().map_err(invalid)?;
    for file in &expected {
        let bytes = fs::read(root.join(&file.relative_path)).map_err(invalid)?;
        if blake3::hash(&bytes).to_hex().to_string() != file.revision {
            return Err(invalid("Source changed during paste preview"));
        }
        write(&directory.path().join(&file.relative_path), &bytes)?;
    }
    for (path, bytes) in &files {
        write(&directory.path().join(path), bytes)?;
    }
    let checked = cad_model::load_project(directory.path()).map_err(invalid)?;
    let cad_check = cad_check::check_loaded_project(&checked);
    let diff = cad_diff::diff_projects(&original, &checked);
    let svg = cad_diff::render_diff_svg(
        &original,
        &checked,
        &cad_diff::diff_selected_drawing(&original, &checked, Some(&request.drawing)),
    );
    let mut warnings = document.warnings.clone();
    if original
        .drawings
        .iter()
        .find(|drawing| drawing.name == request.drawing)
        .and_then(|drawing| drawing.layouts.active())
        .is_some_and(|layout| !layout.viewports.is_empty())
    {
        warnings.push("The target sheet displays viewports; pasted model geometry may not be visible. Review it in a model drawing/layout.".into());
    }
    if document
        .layers
        .layers
        .values()
        .any(|layer| !layer.visible || layer.locked)
        || document
            .layers
            .groups
            .values()
            .any(|group| !group.visible || group.locked)
    {
        warnings.push("Copied layer visibility and locks are preserved; some pasted geometry may be hidden or locked".into());
    }
    let mut report = PasteReport {
        schema_version: "cad-paste/1".into(),
        plan_hash: String::new(),
        status: if cad_check.is_ok() {
            "ready"
        } else {
            "blocked"
        }
        .into(),
        drawing: request.drawing.clone(),
        placement: request.clone(),
        clipboard_blake3: clipboard_hash,
        source_project: document.source_project.name.clone(),
        source_drawing: document.source_drawing.clone(),
        source_snapshot_blake3: document.source_snapshot_blake3.clone(),
        target_files: expected.clone(),
        entity_ids: entities
            .iter()
            .map(|entity| entity.id().as_str().into())
            .collect(),
        changed_files: files.keys().cloned().collect(),
        mappings,
        warnings,
        cad_check,
        diff,
        history_id: None,
    };
    let mut hash = blake3::Hasher::new();
    let report_bytes = serde_json::to_vec(&report)?;
    for bytes in [report_bytes.as_slice(), svg.as_bytes()] {
        hash.update(&(bytes.len() as u64).to_le_bytes());
        hash.update(bytes);
    }
    for (path, bytes) in &files {
        hash.update(&(path.len() as u64).to_le_bytes());
        hash.update(path.as_bytes());
        hash.update(&(bytes.len() as u64).to_le_bytes());
        hash.update(blake3::hash(bytes).as_bytes());
    }
    report.plan_hash = hash.finalize().to_hex().to_string();
    let sealed_report = serde_json::to_vec(&report)?;
    if cad_model::source_manifest(&root).map_err(invalid)? != expected {
        return Err(invalid("Source changed during paste preview"));
    }
    Ok(PastePlan {
        report,
        sealed_report,
        sealed_svg: svg.clone(),
        preview_svg: svg,
        root,
        request: cad_edit::source_replacements::SourceReplacementRequest {
            drawing: request.drawing.clone(),
            expected_files: expected,
            files,
        },
    })
}
pub fn apply(plan: &mut PastePlan, expected_hash: &str) -> Result<()> {
    if plan.report.plan_hash != expected_hash
        || serde_json::to_vec(&plan.report)? != plan.sealed_report
        || plan.preview_svg != plan.sealed_svg
        || plan.report.status != "ready"
    {
        return Err(invalid(
            "Paste review does not match a ready sealed candidate",
        ));
    }
    let result = cad_edit::source_replacements::apply_with_new_blocks(&plan.root, &plan.request)
        .map_err(invalid)?;
    plan.report.history_id = result.history_id;
    plan.report.status = "applied".into();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(parent: &Path, name: &str) -> PathBuf {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/cad-acceptance");
        let target = parent.join(name);
        fs::create_dir(&target).unwrap();
        for file in cad_model::source_manifest(&source).unwrap() {
            write(
                &target.join(&file.relative_path),
                &fs::read(source.join(&file.relative_path)).unwrap(),
            )
            .unwrap();
        }
        assert!(cad_check::check_project(&target).is_ok());
        target
    }
    fn id(suffix: &str) -> String {
        format!("ent_01JZ0000000000000000000{suffix}")
    }
    fn snapshots(root: &Path) -> BTreeMap<String, Vec<u8>> {
        cad_model::source_manifest(root)
            .unwrap()
            .into_iter()
            .map(|file| {
                (
                    file.relative_path.clone(),
                    fs::read(root.join(file.relative_path)).unwrap(),
                )
            })
            .collect()
    }
    #[test]
    fn paste_rotation_and_scale_preserve_references_block_local_geometry_and_undo() {
        let temp = tempfile::tempdir().unwrap();
        let root = fixture(temp.path(), "project");
        let before = snapshots(&root);
        let original_styles = cad_model::load_project(&root).unwrap().styles;
        let document = capture(
            &root,
            "acceptance",
            &[
                id("110"),
                id("120"),
                id("130"),
                id("140"),
                id("103"),
                id("104"),
            ],
            [1000., 500.],
            DimensionCopyPolicy::IncludeReferences,
        )
        .unwrap();
        let request = PasteRequest {
            drawing: "acceptance".into(),
            at: [20000., 30000.],
            rotation_deg: 90.,
            scale: 2.,
        };
        let mut candidate = plan(&root, &document, &request).unwrap();
        let mut other = request.clone();
        other.rotation_deg = 0.;
        assert_ne!(
            candidate.report.plan_hash,
            plan(&root, &document, &other).unwrap().report.plan_hash
        );
        for scale in [0., -1., f64::INFINITY, f64::NAN] {
            let mut bad = request.clone();
            bad.scale = scale;
            assert!(plan(&root, &document, &bad).is_err());
        }
        let mut bad = request.clone();
        bad.rotation_deg = 45.;
        assert!(
            plan(&root, &document, &bad)
                .err()
                .unwrap()
                .to_string()
                .contains("90 degrees")
        );
        let legacy: PasteRequest =
            serde_json::from_str(r#"{"drawing":"acceptance","at":[0,0]}"#).unwrap();
        assert_eq!(legacy.scale, 1.);
        assert_eq!(legacy.rotation_deg, 0.);
        assert_eq!(snapshots(&root), before);
        let hash = candidate.report.plan_hash.clone();
        apply(&mut candidate, &hash).unwrap();
        let source = cad_model::load_project(&root).unwrap();
        let get = |old: &str| {
            &source.drawings[0]
                .entities
                .iter()
                .find(|record| {
                    record.entity.id().as_str() == candidate.report.mappings.entities[&id(old)]
                })
                .unwrap()
                .entity
        };
        assert!(
            matches!(get("100"),Entity::Polyline{points,..} if (points[0][0]-21000.).abs()<1e-8&&(points[0][1]-28000.).abs()<1e-8)
        );
        assert_eq!(
            cad_model::evaluate_dimension(&source, get("110"))
                .unwrap()
                .label,
            "10000 mm"
        );
        assert_eq!(
            serde_json::to_value(get("110")).unwrap()["measurement"]["kind"],
            "vertical"
        );
        assert!(
            matches!(get("130"),Entity::Hatch{angle_deg,scale,..} if *angle_deg==120.&&*scale==500.)
        );
        assert!(
            matches!(get("120"),Entity::BlockRef{at,rotation_deg,scale,..} if *at==[21000.,30000.]&&*rotation_deg==90.&&*scale==2.)
        );
        assert!(matches!(get("103"),Entity::Circle{radius,..} if *radius==1200.));
        assert!(
            matches!(get("104"),Entity::Arc{radius,start_deg,end_deg,..} if *radius==1000.&&*start_deg==180.&&*end_deg==90.)
        );
        assert_eq!(
            source.blocks[&candidate.report.mappings.blocks["door"]].config,
            document.blocks["door"].config
        );
        assert_eq!(source.styles, original_styles);
        assert!(cad_check::check_project(&root).is_ok());
        let files = cad_edit::list_drawing_history(&root, "acceptance")
            .unwrap()
            .current_files;
        cad_edit::undo_drawing_edit(
            &root,
            &cad_edit::DrawingHistoryRequest {
                drawing: "acceptance".into(),
                expected_files: files,
            },
        )
        .unwrap();
        assert_eq!(snapshots(&root), before);
    }
    #[test]
    fn cross_project_copy_renames_definitions_preserves_dimensions_and_blocks_and_undo_redo() {
        let temp = tempfile::tempdir().unwrap();
        let source = fixture(temp.path(), "source");
        let target = fixture(temp.path(), "target");
        let mut original = cad_model::load_project(&target).unwrap();
        original.styles.colors.get_mut("black").unwrap().rgb = "#FF0000".into();
        original.styles.text_styles.get_mut("note").unwrap().height += 50.;
        original.styles.pens.get_mut("dashed").unwrap().line_width += 0.5;
        original.layers.layers.get_mut("0-1").unwrap().name = "Target definition".into();
        original
            .layers
            .groups
            .get_mut("design")
            .unwrap()
            .scale_denominator = 100.;
        write(
            &target.join("rules/styles.toml"),
            toml::to_string_pretty(&original.styles).unwrap().as_bytes(),
        )
        .unwrap();
        write(
            &target.join("rules/layers.toml"),
            toml::to_string_pretty(&original.layers).unwrap().as_bytes(),
        )
        .unwrap();
        let source_before = snapshots(&source);
        let before = snapshots(&target);
        let ids = [id("110"), id("120"), id("130"), id("140"), id("105")];
        let document = capture(
            &source,
            "acceptance",
            &ids,
            [1000., 500.],
            DimensionCopyPolicy::IncludeReferences,
        )
        .unwrap();
        assert_eq!(document.entities.len(), 6);
        assert_eq!(document.blocks.len(), 1);
        assert_eq!(document.styles.dimension_styles.len(), 1);
        let request = PasteRequest {
            drawing: "acceptance".into(),
            at: [20000., 20000.],
            rotation_deg: 0.,
            scale: 1.,
        };
        let mut candidate = plan(&target, &document, &request).unwrap();
        assert_eq!(candidate.report.status, "ready");
        assert_eq!(snapshots(&target), before);
        assert_eq!(
            candidate.report.plan_hash,
            plan(&target, &document, &request).unwrap().report.plan_hash
        );
        let hash = candidate.report.plan_hash.clone();
        assert!(apply(&mut candidate, "wrong").is_err());
        apply(&mut candidate, &hash).unwrap();
        assert!(cad_check::check_project(&target).is_ok());
        assert_eq!(snapshots(&source), source_before);
        let merged = cad_model::load_project(&target).unwrap();
        let mapping = &candidate.report.mappings;
        assert_ne!(mapping.layers["0-1"], "0-1");
        assert_ne!(mapping.colors["black"], "black");
        assert_ne!(mapping.text_styles["note"], "note");
        assert_ne!(mapping.pens["dashed"], "dashed");
        assert_eq!(
            merged.styles.colors["black"],
            original.styles.colors["black"]
        );
        assert_eq!(merged.layers.layers["0-1"], original.layers.layers["0-1"]);
        assert_eq!(merged.blocks["door"], original.blocks["door"]);
        let pasted = |suffix: &str| {
            &merged.drawings[0]
                .entities
                .iter()
                .find(|record| record.entity.id().as_str() == mapping.entities[&id(suffix)])
                .unwrap()
                .entity
        };
        let dimension = pasted("110");
        assert!(
            refs(dimension)
                .iter()
                .all(|reference| reference == &mapping.entities[&id("100")])
        );
        assert_eq!(
            cad_model::evaluate_dimension(&merged, dimension)
                .unwrap()
                .label,
            "5000 mm"
        );
        assert!(
            matches!(pasted("120"),Entity::BlockRef{at,block,..}if *at==[20000.,19500.] && block==&mapping.blocks["door"])
        );
        assert_eq!(merged.blocks[&mapping.blocks["door"]].entities.len(), 2);
        assert!(
            merged.blocks[&mapping.blocks["door"]]
                .entities
                .iter()
                .all(|record| !original.blocks["door"]
                    .entities
                    .iter()
                    .any(|old| old.entity.id() == record.entity.id()))
        );
        assert!(
            fs::read(target.join("drawings/acceptance/entities.ndjson"))
                .unwrap()
                .starts_with(&before["drawings/acceptance/entities.ndjson"])
        );
        let history = cad_edit::list_drawing_history(&target, "acceptance").unwrap();
        cad_edit::undo_drawing_edit(
            &target,
            &cad_edit::DrawingHistoryRequest {
                drawing: "acceptance".into(),
                expected_files: history.current_files,
            },
        )
        .unwrap();
        assert_eq!(snapshots(&target), before);
        assert!(cad_check::check_project(&target).is_ok());
        let history = cad_edit::list_drawing_history(&target, "acceptance").unwrap();
        cad_edit::redo_drawing_edit(
            &target,
            &cad_edit::DrawingHistoryRequest {
                drawing: "acceptance".into(),
                expected_files: history.current_files,
            },
        )
        .unwrap();
        assert!(cad_check::check_project(&target).is_ok());
        assert!(
            target
                .join(format!("blocks/{}/entities.ndjson", mapping.blocks["door"]))
                .exists()
        );
    }
    #[test]
    fn dimension_policy_rejects_or_detaches_without_changing_the_source() {
        let temp = tempfile::tempdir().unwrap();
        let root = fixture(temp.path(), "project");
        let before = snapshots(&root);
        assert!(
            capture(
                &root,
                "acceptance",
                &[id("110")],
                [0., 0.],
                DimensionCopyPolicy::RejectExternal
            )
            .is_err()
        );
        let document = capture(
            &root,
            "acceptance",
            &[id("110")],
            [0., 0.],
            DimensionCopyPolicy::DetachExternal,
        )
        .unwrap();
        assert_eq!(document.entities.len(), 1);
        assert!(refs(&document.entities[0]).is_empty());
        assert_eq!(document.warnings.len(), 1);
        let mut candidate = plan(
            &root,
            &document,
            &PasteRequest {
                drawing: "acceptance".into(),
                at: [10000., 0.],
                rotation_deg: 0.,
                scale: 1.,
            },
        )
        .unwrap();
        let hash = candidate.report.plan_hash.clone();
        apply(&mut candidate, &hash).unwrap();
        let merged = cad_model::load_project(&root).unwrap();
        let entity = &merged.drawings[0].entities.last().unwrap().entity;
        assert!(matches!(entity,Entity::Dimension{p1,..}if *p1==[10000.,0.]));
        assert_eq!(
            cad_model::evaluate_dimension(&merged, entity)
                .unwrap()
                .label,
            "5000 mm"
        );
        assert!(before.len() <= snapshots(&root).len());
    }
    #[test]
    fn part_reader_bounds_bytes_and_roundtrips_validated_documents() {
        let temp = tempfile::tempdir().unwrap();
        let root = fixture(temp.path(), "project");
        let document = capture(
            &root,
            "acceptance",
            &[id("120")],
            [0., 0.],
            DimensionCopyPolicy::IncludeReferences,
        )
        .unwrap();
        let path = temp.path().join("part.json");
        fs::write(&path, serialize_document(&document).unwrap()).unwrap();
        assert_eq!(
            serde_json::to_value(read_document(&path).unwrap()).unwrap(),
            serde_json::to_value(&document).unwrap()
        );
        let large = temp.path().join("large.json");
        fs::File::create(&large)
            .unwrap()
            .set_len(64 * 1024 * 1024 + 1)
            .unwrap();
        assert!(
            read_document(&large)
                .unwrap_err()
                .to_string()
                .contains("64 MiB")
        );
    }

    #[test]
    fn nested_mirrored_blocks_and_internal_dimension_references_are_remapped() {
        let temp = tempfile::tempdir().unwrap();
        let root = fixture(temp.path(), "project");
        let mut block = fs::read(root.join("blocks/door/entities.ndjson")).unwrap();
        block.extend(format!("{}\n",serde_json::json!({"schema_version":"0.3","id":id("202"),"type":"dimension","layer":"0-2","style":"dim","p1":[0,0],"p2":[0,900],"offset":100,"measurement":{"kind":"vertical","first":{"kind":"entity","entity_id":id("200"),"feature":"start"},"second":{"kind":"entity","entity_id":id("200"),"feature":"end"}}})).as_bytes());
        fs::write(root.join("blocks/door/entities.ndjson"), block).unwrap();
        write(
            &root.join("blocks/nested/definition.toml"),
            b"schema_version = \"0.3\"\nname = \"Nested\"\nbase_point = [10, 20]\n",
        )
        .unwrap();
        write(&root.join("blocks/nested/entities.ndjson"),format!("{}\n",serde_json::json!({"schema_version":"0.3","id":id("700"),"type":"block_ref","layer":"0-1","block":"door","at":[10,20],"rotation_deg":30,"scale":2,"mirror_x":true,"mirror_y":false})).as_bytes()).unwrap();
        let mut bytes = fs::read(root.join("drawings/acceptance/entities.ndjson")).unwrap();
        bytes.extend(format!("{}\n",serde_json::json!({"schema_version":"0.3","id":id("701"),"type":"block_ref","layer":"0-1","block":"nested","at":[5000,5000],"rotation_deg":90,"scale":3,"mirror_x":false,"mirror_y":true})).as_bytes());
        fs::write(root.join("drawings/acceptance/entities.ndjson"), bytes).unwrap();
        assert!(cad_check::check_project(&root).is_ok());
        let document = capture(
            &root,
            "acceptance",
            &[id("701")],
            [5000., 5000.],
            DimensionCopyPolicy::IncludeReferences,
        )
        .unwrap();
        assert_eq!(document.blocks.len(), 2);
        let mut candidate = plan(
            &root,
            &document,
            &PasteRequest {
                drawing: "acceptance".into(),
                at: [20000., 20000.],
                rotation_deg: 0.,
                scale: 1.,
            },
        )
        .unwrap();
        let hash = candidate.report.plan_hash.clone();
        apply(&mut candidate, &hash).unwrap();
        let merged = cad_model::load_project(&root).unwrap();
        let mapping = &candidate.report.mappings;
        let nested = &merged.blocks[&mapping.blocks["nested"]];
        assert_eq!(nested.config.base_point, [10., 20.]);
        assert!(
            matches!(&nested.entities[0].entity,Entity::BlockRef{block,at,rotation_deg,scale,mirror_x,mirror_y,..}if block==&mapping.blocks["door"]&&*at==[10.,20.]&&*rotation_deg==30.&&*scale==2.&&*mirror_x&&!*mirror_y)
        );
        let child = &merged.blocks[&mapping.blocks["door"]];
        let dimension = &child
            .entities
            .iter()
            .find(|record| matches!(record.entity, Entity::Dimension { .. }))
            .unwrap()
            .entity;
        assert!(
            refs(dimension)
                .iter()
                .all(|reference| reference == &mapping.entities[&id("200")])
        );
        assert_eq!(
            cad_model::evaluate_dimension(&merged, dimension)
                .unwrap()
                .label,
            "900 mm"
        );
        assert!(cad_check::check_project(&root).is_ok());
    }

    #[test]
    fn malformed_parts_changed_sources_and_tampered_plans_never_publish() {
        let temp = tempfile::tempdir().unwrap();
        let root = fixture(temp.path(), "project");
        let before = snapshots(&root);
        let document = capture(
            &root,
            "acceptance",
            &[id("120")],
            [0., 0.],
            DimensionCopyPolicy::IncludeReferences,
        )
        .unwrap();
        let request = PasteRequest {
            drawing: "acceptance".into(),
            at: [10000., 0.],
            rotation_deg: 0.,
            scale: 1.,
        };
        let mut malformed = document.clone();
        malformed
            .blocks
            .insert("../escape".into(), document.blocks["door"].clone());
        assert!(plan(&root, &malformed, &request).is_err());
        let mut malformed = document.clone();
        malformed.entities.push(malformed.entities[0].clone());
        assert!(plan(&root, &malformed, &request).is_err());
        let mut candidate = plan(&root, &document, &request).unwrap();
        let hash = candidate.report.plan_hash.clone();
        candidate.report.changed_files.clear();
        assert!(apply(&mut candidate, &hash).is_err());
        assert_eq!(snapshots(&root), before);
        let mut candidate = plan(&root, &document, &request).unwrap();
        let hash = candidate.report.plan_hash.clone();
        fs::write(
            root.join("rules/layers.toml"),
            before["rules/layers.toml"]
                .iter()
                .copied()
                .chain(b"\n# external\n".iter().copied())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let changed = snapshots(&root);
        assert!(apply(&mut candidate, &hash).is_err());
        assert_eq!(snapshots(&root), changed);
        assert!(
            capture(
                &root,
                "acceptance",
                &[id("120"), id("120")],
                [0., 0.],
                DimensionCopyPolicy::IncludeReferences
            )
            .is_err()
        );
        assert!(
            capture(
                &root,
                "acceptance",
                &[id("999")],
                [0., 0.],
                DimensionCopyPolicy::IncludeReferences
            )
            .is_err()
        );
        assert!(
            plan(
                &root,
                &document,
                &PasteRequest {
                    drawing: "acceptance".into(),
                    at: [f64::INFINITY, 0.],
                    rotation_deg: 0.,
                    scale: 1.,
                }
            )
            .is_err()
        );
    }
}
