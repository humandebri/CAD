//! Experimental CAD source to JWW boundary exporter.

use cad_jww_codec::{Base, BlockDefinition, Document, Header, Layer, LayerGroup, Record};
use cad_model::{Entity, LayerDef, ProjectSource, TextStyleDef};
use encoding_rs::SHIFT_JIS;
use rustix::fs::{CWD, RenameFlags, renameat_with};
use rustix::io::Errno;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const CRATE_NAME: &str = "cad-export-jww";

#[must_use]
pub fn crate_name() -> &'static str {
    CRATE_NAME
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ExportOptions {
    pub allow_lossy: bool,
    pub overwrite: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExportStatus {
    Exported,
    Blocked,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ExportIssue {
    pub code: String,
    pub message: String,
    pub entity_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportReport {
    pub schema_version: String,
    pub status: ExportStatus,
    pub output_path: String,
    pub written_entities: usize,
    pub expanded_entities: usize,
    pub warnings: Vec<ExportIssue>,
    pub blockers: Vec<ExportIssue>,
}

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("failed to load CAD project")]
    Model(#[from] cad_model::ModelError),
    #[error("drawing {0:?} was not found")]
    DrawingNotFound(String),
    #[error("output already exists: {0}")]
    OutputExists(PathBuf),
    #[error("failed to encode JWW")]
    Codec(#[from] cad_jww_codec::CodecError),
    #[error("failed to write {path}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub type ExportResult<T> = Result<T, ExportError>;

pub fn export_jww_file(
    project_path: impl AsRef<Path>,
    drawing_name: &str,
    output_path: impl AsRef<Path>,
    options: ExportOptions,
) -> ExportResult<ExportReport> {
    let project = cad_model::load_project(project_path)?;
    export_loaded_project(&project, drawing_name, output_path.as_ref(), options)
}

pub fn export_loaded_project(
    project: &ProjectSource,
    drawing_name: &str,
    output_path: &Path,
    options: ExportOptions,
) -> ExportResult<ExportReport> {
    let drawing = project
        .drawings
        .iter()
        .find(|drawing| drawing.name == drawing_name)
        .ok_or_else(|| ExportError::DrawingNotFound(drawing_name.to_owned()))?;
    let mut context = ExportContext::new(output_path, options);
    let check = cad_check::check_loaded_project(project);
    for diagnostic in check
        .diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.severity == cad_check::Severity::Error)
    {
        context.blockers.push(ExportIssue {
            code: "invalid_project".to_owned(),
            message: format!("{}: {}", diagnostic.code, diagnostic.message),
            entity_id: diagnostic.entity_id,
        });
    }
    let (header, layer_slots) = build_header(project, drawing_name, &mut context);
    let block_numbers = project
        .blocks
        .keys()
        .enumerate()
        .map(|(index, id)| (id.clone(), u32::try_from(index + 1).unwrap_or(u32::MAX)))
        .collect::<BTreeMap<_, _>>();
    let mut records = Vec::new();
    let mut written = 0;
    for record in &drawing.entities {
        let record_count = records.len();
        convert_entity(
            project,
            &record.entity,
            &layer_slots,
            &block_numbers,
            &mut records,
            &mut context,
        );
        if records.len() > record_count {
            written += 1;
        }
    }
    let mut block_definitions = Vec::new();
    for (block_id, definition) in &project.blocks {
        let mut block_records = Vec::new();
        for record in &definition.entities {
            convert_entity(
                project,
                &record.entity,
                &layer_slots,
                &block_numbers,
                &mut block_records,
                &mut context,
            );
        }
        block_definitions.push(BlockDefinition {
            number: block_numbers[block_id],
            name: definition.config.name.clone(),
            records: block_records,
        });
    }
    if records.len() > 65_534 {
        context.block(
            None,
            "record_limit",
            "JWW supports at most 65534 output records",
        );
    }
    let expanded = records.len();
    if !context.blockers.is_empty() {
        return Ok(context.report(ExportStatus::Blocked, 0, expanded));
    }
    if output_path.exists() && !options.overwrite {
        return Err(ExportError::OutputExists(output_path.to_path_buf()));
    }
    let bytes = cad_jww_codec::write_document(&Document {
        header,
        records,
        blocks: block_definitions,
    })?;
    publish(output_path, &bytes, options.overwrite)?;
    Ok(context.report(ExportStatus::Exported, written, expanded))
}

struct ExportContext<'a> {
    output_path: &'a Path,
    options: ExportOptions,
    warnings: Vec<ExportIssue>,
    blockers: Vec<ExportIssue>,
}

impl<'a> ExportContext<'a> {
    fn new(output_path: &'a Path, options: ExportOptions) -> Self {
        Self {
            output_path,
            options,
            warnings: Vec::new(),
            blockers: Vec::new(),
        }
    }

    fn block(&mut self, entity: Option<&Entity>, code: &str, message: impl Into<String>) {
        let issue = ExportIssue {
            code: code.to_owned(),
            message: message.into(),
            entity_id: entity.map(|entity| entity.id().as_str().to_owned()),
        };
        if self.options.allow_lossy {
            self.warnings.push(issue);
        } else {
            self.blockers.push(issue);
        }
    }

    fn warn(&mut self, entity: Option<&Entity>, code: &str, message: impl Into<String>) {
        self.warnings.push(ExportIssue {
            code: code.to_owned(),
            message: message.into(),
            entity_id: entity.map(|entity| entity.id().as_str().to_owned()),
        });
    }

    fn report(self, status: ExportStatus, written: usize, expanded: usize) -> ExportReport {
        ExportReport {
            schema_version: "0.2".to_owned(),
            status,
            output_path: self.output_path.display().to_string(),
            written_entities: written,
            expanded_entities: expanded,
            warnings: self.warnings,
            blockers: self.blockers,
        }
    }
}

fn build_header(
    project: &ProjectSource,
    drawing_name: &str,
    context: &mut ExportContext<'_>,
) -> (Header, BTreeMap<String, (u16, u16)>) {
    let drawing = project
        .drawings
        .iter()
        .find(|drawing| drawing.name == drawing_name)
        .expect("drawing checked");
    let Some(layout) = drawing.layouts.active() else {
        context.block(
            None,
            "invalid_active_layout",
            "drawing has no active layout",
        );
        return (Header::default(), BTreeMap::new());
    };
    let layout_scale = parse_layout_scale(&layout.scale, context);
    if matches!(layout.orientation, cad_model::SheetOrientation::Landscape) {
        context.warn(
            None,
            "layout_orientation_approximated",
            "JWW version 600 does not carry an independent layout orientation flag",
        );
    }
    let mut header = Header {
        memo: cp932_text(&project.project.name, None, context),
        paper_size: paper_code(&layout.paper, context),
        ..Header::default()
    };
    let mut groups = project.layers.groups.iter().collect::<Vec<_>>();
    groups.sort_by_key(|(id, group)| (group.order, (*id).clone()));
    if groups.len() > 16 {
        context.block(
            None,
            "layer_group_limit",
            "JWW supports at most 16 layer groups",
        );
        groups.truncate(16);
    }
    let mut group_slots = BTreeMap::<String, u16>::new();
    if groups.is_empty() {
        group_slots.insert("default".to_owned(), 0);
        header.layer_groups[0].name = "Default".to_owned();
    } else {
        for (slot, (id, group)) in groups.iter().enumerate() {
            group_slots.insert((*id).clone(), slot as u16);
            header.layer_groups[slot] = LayerGroup {
                name: cp932_text(&group.name, None, context),
                state: if group.visible {
                    if group.locked { 1 } else { 2 }
                } else {
                    0
                },
                write_layer: 0,
                scale: layout_scale.unwrap_or_else(|| group.scale_denominator.max(1.0)),
                protect: u32::from(group.locked),
                layers: std::array::from_fn(|_| Layer {
                    state: 0,
                    ..Layer::default()
                }),
            };
        }
    }
    let mut by_group = BTreeMap::<u16, Vec<(&String, &LayerDef)>>::new();
    for (id, layer) in &project.layers.layers {
        let slot = layer
            .group
            .as_ref()
            .and_then(|group| group_slots.get(group))
            .copied()
            .unwrap_or(0);
        by_group.entry(slot).or_default().push((id, layer));
    }
    let mut layer_slots = BTreeMap::new();
    for (group_slot, layers) in &mut by_group {
        layers.sort_by_key(|(id, layer)| (layer.order, (*id).clone()));
        if layers.len() > 16 {
            context.block(
                None,
                "layer_limit",
                format!("layer group {group_slot} contains more than 16 layers"),
            );
            layers.truncate(16);
        }
        for (layer_slot, (id, layer)) in layers.iter().enumerate() {
            let active = project.layers.active_layer.as_deref() == Some(id.as_str());
            let state = if !layer.visible {
                0
            } else if active {
                3
            } else if layer.locked {
                1
            } else {
                2
            };
            header.layer_groups[*group_slot as usize].layers[layer_slot] = Layer {
                name: cp932_text(&layer.name, None, context),
                state,
                protect: u32::from(layer.locked),
            };
            if active {
                header.write_layer_group = u32::from(*group_slot);
                header.layer_groups[*group_slot as usize].write_layer = layer_slot as u32;
            }
            layer_slots.insert((*id).clone(), (*group_slot, layer_slot as u16));
        }
    }
    (header, layer_slots)
}

fn convert_entity(
    project: &ProjectSource,
    entity: &Entity,
    layer_slots: &BTreeMap<String, (u16, u16)>,
    block_numbers: &BTreeMap<String, u32>,
    records: &mut Vec<Record>,
    context: &mut ExportContext<'_>,
) {
    let Some(&(group, layer)) = layer_slots.get(entity.layer()) else {
        context.block(
            Some(entity),
            "undefined_layer_slot",
            "entity layer is not exportable to JWW",
        );
        return;
    };
    let base = match entity_base(project, entity, group, layer, context) {
        Some(base) => base,
        None => return,
    };
    match entity {
        Entity::Line { p1, p2, .. } => records.push(Record::Line {
            base,
            p1: *p1,
            p2: *p2,
        }),
        Entity::Polyline { points, closed, .. } => {
            for pair in points.windows(2) {
                records.push(Record::Line {
                    base,
                    p1: pair[0],
                    p2: pair[1],
                });
            }
            if *closed && points.len() > 2 {
                records.push(Record::Line {
                    base,
                    p1: *points.last().expect("nonempty"),
                    p2: points[0],
                });
            }
        }
        Entity::Arc {
            center,
            radius,
            start_deg,
            end_deg,
            ..
        } => records.push(Record::Arc {
            base,
            center: *center,
            radius: *radius,
            start_rad: start_deg.to_radians(),
            sweep_rad: (end_deg - start_deg).to_radians(),
            tilt_rad: 0.0,
            flatness: 1.0,
            full: false,
        }),
        Entity::Circle { center, radius, .. } => records.push(Record::Arc {
            base,
            center: *center,
            radius: *radius,
            start_rad: 0.0,
            sweep_rad: std::f64::consts::TAU,
            tilt_rad: 0.0,
            flatness: 1.0,
            full: true,
        }),
        Entity::Ellipse {
            center,
            radius_x,
            radius_y,
            rotation_deg,
            start_deg,
            end_deg,
            ..
        } => records.push(Record::Arc {
            base,
            center: *center,
            radius: *radius_x,
            start_rad: start_deg.to_radians(),
            sweep_rad: (end_deg - start_deg).to_radians(),
            tilt_rad: rotation_deg.to_radians(),
            flatness: radius_y / radius_x,
            full: (end_deg - start_deg).abs() >= 360.0 - 1e-9,
        }),
        Entity::Text {
            style,
            at,
            rotation_deg,
            mirror_y,
            value,
            ..
        } => {
            if *mirror_y {
                context.block(
                    Some(entity),
                    "mirrored_text",
                    "JWW export cannot preserve mirrored text",
                );
                if !context.options.allow_lossy {
                    return;
                }
            }
            let Some(style) = project.styles.text_styles.get(style) else {
                context.block(
                    Some(entity),
                    "undefined_text_style",
                    "text style is missing",
                );
                return;
            };
            records.push(text_record(
                base,
                *at,
                *rotation_deg,
                value,
                style,
                context,
                Some(entity),
            ));
        }
        Entity::Dimension {
            style,
            p1,
            p2,
            offset,
            text_rotation_deg,
            text_mirror_y,
            value,
            ..
        } => {
            if *text_mirror_y {
                context.block(
                    Some(entity),
                    "mirrored_text",
                    "JWW export cannot preserve mirrored dimension text",
                );
                if !context.options.allow_lossy {
                    return;
                }
            }
            let Some(dimension_style) = project.styles.dimension_styles.get(style) else {
                context.block(
                    Some(entity),
                    "undefined_dimension_style",
                    "dimension style is missing",
                );
                return;
            };
            let Some(text_style) = project.styles.text_styles.get(&dimension_style.text_style)
            else {
                context.block(
                    Some(entity),
                    "undefined_text_style",
                    "dimension text style is missing",
                );
                return;
            };
            context.block(
                Some(entity),
                "dimension_style_approximated",
                "JWW export cannot preserve dimension arrow, extension gap, precision, and unit semantics",
            );
            if !context.options.allow_lossy {
                return;
            }
            let Some((d1, d2)) = cad_model::dimension_offset_segment(*p1, *p2, *offset) else {
                context.block(
                    Some(entity),
                    "invalid_dimension",
                    "dimension has invalid geometry",
                );
                return;
            };
            let label = value.clone().unwrap_or_else(|| {
                cad_model::format_decimal_mm(
                    ((p2[0] - p1[0]).powi(2) + (p2[1] - p1[1]).powi(2)).sqrt(),
                )
            });
            let text = text_record(
                base,
                [(d1[0] + d2[0]) / 2.0, (d1[1] + d2[1]) / 2.0],
                *text_rotation_deg,
                &label,
                text_style,
                context,
                Some(entity),
            );
            records.push(Record::Dimension {
                base,
                line: Box::new(Record::Line {
                    base,
                    p1: d1,
                    p2: d2,
                }),
                text: Box::new(text),
            });
        }
        Entity::Point {
            at,
            temporary,
            marker_code,
            rotation_deg,
            scale,
            ..
        } => records.push(Record::Point {
            base,
            at: *at,
            temporary: *temporary,
            marker: marker_code.map(|code| (code, rotation_deg.to_radians(), *scale)),
        }),
        Entity::Solid { points, fill, .. } => {
            if !(3..=4).contains(&points.len()) {
                context.block(
                    Some(entity),
                    "solid_point_count",
                    "JWW solid requires three or four points",
                );
                return;
            }
            let Some((base, color)) = fill_base(project, fill, base) else {
                context.block(
                    Some(entity),
                    "undefined_fill",
                    "solid fill color is invalid",
                );
                return;
            };
            let p4 = if points.len() == 3 {
                points[2]
            } else {
                points[3]
            };
            records.push(Record::Solid {
                base,
                values: [
                    points[0][0],
                    points[0][1],
                    p4[0],
                    p4[1],
                    points[1][0],
                    points[1][1],
                    points[2][0],
                    points[2][1],
                ],
                color,
            });
        }
        Entity::CurveSolid {
            center,
            radius,
            flatness,
            rotation_deg,
            start_deg,
            end_deg,
            solid_param,
            encoding_code,
            fill,
            ..
        } => {
            let Some((mut base, color)) = fill_base(project, fill, base) else {
                context.block(
                    Some(entity),
                    "undefined_fill",
                    "curve solid fill color is invalid",
                );
                return;
            };
            base.pen_style = u8::try_from((*encoding_code).max(101)).unwrap_or(u8::MAX);
            records.push(Record::Solid {
                base,
                values: [
                    center[0],
                    center[1],
                    *radius,
                    *flatness,
                    rotation_deg.to_radians(),
                    start_deg.to_radians(),
                    (end_deg - start_deg).to_radians(),
                    *solid_param,
                ],
                color,
            });
        }
        Entity::BlockRef {
            block,
            at,
            rotation_deg,
            scale,
            ..
        } => {
            let Some(def_number) = block_numbers.get(block).copied() else {
                context.block(
                    Some(entity),
                    "unsupported_block_ref",
                    format!("block definition {block:?} is not available"),
                );
                return;
            };
            let base_point = project
                .blocks
                .get(block)
                .map(|definition| definition.config.base_point)
                .unwrap_or([0.0, 0.0]);
            let angle = rotation_deg.to_radians();
            let (sin, cos) = angle.sin_cos();
            let base_offset = [
                scale * (base_point[0] * cos - base_point[1] * sin),
                scale * (base_point[0] * sin + base_point[1] * cos),
            ];
            records.push(Record::Block {
                base,
                ref_x: at[0] - base_offset[0],
                ref_y: at[1] - base_offset[1],
                scale_x: *scale,
                scale_y: *scale,
                rotation: rotation_deg.to_radians(),
                def_number,
            });
        }
        Entity::Hatch {
            loops,
            pattern,
            angle_deg,
            scale,
            fill,
            ..
        } => {
            if loops.len() != 1 {
                context.block(
                    Some(entity),
                    "unsupported_hatch_loop",
                    "JWW export supports only one closed hatch loop",
                );
                return;
            }
            let loop_points = &loops[0];
            if loop_points.len() < 3
                || loop_points
                    .iter()
                    .any(|point| !point[0].is_finite() || !point[1].is_finite())
            {
                context.block(
                    Some(entity),
                    "unsupported_hatch_geometry",
                    "hatch loop must contain at least three finite points",
                );
                return;
            }
            let Some((fill_base, color)) = fill
                .as_deref()
                .and_then(|fill| fill_base(project, fill, base))
            else {
                context.block(
                    Some(entity),
                    "undefined_fill",
                    "hatch fill color is missing or invalid",
                );
                return;
            };
            context.warn(
                Some(entity),
                "hatch_pattern_approximated",
                format!(
                    "hatch pattern {pattern:?}, angle {angle_deg}, and scale {scale} are emitted as solid polygon fill"
                ),
            );
            for pair in loop_points[1..].windows(2) {
                push_solid_record(
                    records,
                    fill_base,
                    [loop_points[0], pair[0], pair[1]],
                    color,
                );
            }
        }
    }
}

fn text_record(
    base: Base,
    at: [f64; 2],
    rotation_deg: f64,
    value: &str,
    style: &TextStyleDef,
    context: &mut ExportContext<'_>,
    entity: Option<&Entity>,
) -> Record {
    let value = cp932_text(value, entity, context);
    if style.font_family != "MS Gothic" {
        context.block(
            entity,
            "font_substituted",
            format!("font {:?} is substituted with MS Gothic", style.font_family),
        );
    }
    let angle = rotation_deg.to_radians();
    let count = value.chars().count();
    let length = count as f64 * style.width + count.saturating_sub(1) as f64 * style.spacing;
    let anchor_offset = match style.align {
        cad_model::TextAlign::Left => 0.0,
        cad_model::TextAlign::Center => length / 2.0,
        cad_model::TextAlign::Right => length,
    };
    let start = [
        at[0] - anchor_offset * angle.cos(),
        at[1] - anchor_offset * angle.sin(),
    ];
    Record::Text {
        base,
        start,
        end: [
            start[0] + length * angle.cos(),
            start[1] + length * angle.sin(),
        ],
        text_type: 1,
        size_x: style.width,
        size_y: style.height,
        spacing: style.spacing,
        angle_deg: rotation_deg,
        font: "MS Gothic".to_owned(),
        value,
    }
}

fn entity_base(
    project: &ProjectSource,
    entity: &Entity,
    group: u16,
    layer: u16,
    context: &mut ExportContext<'_>,
) -> Option<Base> {
    let layer_def = project.layers.layers.get(entity.layer())?;
    let (color, line_type, line_width) = if let Some(pen_id) = entity.pen() {
        let Some(pen) = project.styles.pens.get(pen_id) else {
            context.block(
                Some(entity),
                "undefined_pen",
                format!("pen {pen_id:?} is missing"),
            );
            return None;
        };
        (&pen.color, &pen.line_type, pen.line_width)
    } else {
        (&layer_def.color, &layer_def.line_type, layer_def.line_width)
    };
    let pen_color = stroke_color_number(project, color, entity, context)?;
    let pen_style = line_type_number(line_type, entity, context)?;
    Some(Base {
        pen_style,
        pen_color,
        pen_width: (line_width.max(0.0) * 100.0)
            .round()
            .min(f64::from(u16::MAX)) as u16,
        layer,
        layer_group: group,
        ..Base::default()
    })
}

fn stroke_color_number(
    project: &ProjectSource,
    color_id: &str,
    entity: &Entity,
    context: &mut ExportContext<'_>,
) -> Option<u16> {
    if let Some(value) = color_id
        .strip_prefix("jww_color_")
        .and_then(|value| value.parse().ok())
    {
        return Some(value);
    }
    let Some(color) = project.styles.colors.get(color_id) else {
        context.block(
            Some(entity),
            "undefined_stroke_color",
            format!("color {color_id:?} is missing"),
        );
        return None;
    };
    let rgb = color.rgb.to_ascii_uppercase();
    let known = [
        ("#000000", 1),
        ("#FF0000", 2),
        ("#00AA00", 3),
        ("#0000FF", 4),
        ("#FFFF00", 5),
        ("#FF00FF", 6),
        ("#00FFFF", 7),
        ("#FFFFFF", 8),
    ];
    if let Some((_, number)) = known.iter().find(|(known, _)| *known == rgb) {
        return Some(*number);
    }
    context.block(
        Some(entity),
        "unsupported_stroke_color",
        format!("color {rgb} has no built-in JWW pen mapping"),
    );
    context.options.allow_lossy.then_some(1)
}

fn line_type_number(
    line_type: &str,
    entity: &Entity,
    context: &mut ExportContext<'_>,
) -> Option<u8> {
    if let Some(value) = line_type
        .strip_prefix("jww_line_")
        .and_then(|value| value.parse().ok())
    {
        return Some(value);
    }
    if line_type == "solid" {
        return Some(1);
    }
    context.block(
        Some(entity),
        "unsupported_line_type",
        format!("line type {line_type:?} has no JWW mapping"),
    );
    context.options.allow_lossy.then_some(1)
}

fn fill_base(project: &ProjectSource, fill: &str, mut base: Base) -> Option<(Base, Option<u32>)> {
    let color = project.styles.colors.get(fill)?.rgb.trim_start_matches('#');
    let rgb = u32::from_str_radix(color, 16).ok()?;
    base.pen_color = 10;
    Some((base, Some(rgb)))
}

fn paper_code(paper: &str, context: &mut ExportContext<'_>) -> u32 {
    match paper.to_ascii_uppercase().as_str() {
        "A0" => 0,
        "A1" => 1,
        "A2" => 2,
        "A3" => 3,
        "A4" => 4,
        _ => {
            context.block(
                None,
                "unsupported_paper",
                format!("paper {paper:?} is not supported by the exporter"),
            );
            3
        }
    }
}

fn parse_layout_scale(scale: &str, context: &mut ExportContext<'_>) -> Option<f64> {
    let scale = scale.trim();
    let value = scale
        .strip_prefix("1/")
        .and_then(|value| value.parse::<f64>().ok())
        .or_else(|| {
            scale
                .strip_prefix("1:")
                .and_then(|value| value.parse::<f64>().ok())
        })
        .or_else(|| scale.parse::<f64>().ok());
    match value {
        Some(value) if value.is_finite() && value > 0.0 => Some(value),
        _ => {
            context.block(
                None,
                "unsupported_layout_scale",
                format!("layout scale {scale:?} is not a positive numeric scale"),
            );
            None
        }
    }
}

fn push_solid_record(
    records: &mut Vec<Record>,
    base: Base,
    points: [[f64; 2]; 3],
    color: Option<u32>,
) {
    records.push(Record::Solid {
        base,
        values: [
            points[0][0],
            points[0][1],
            points[2][0],
            points[2][1],
            points[1][0],
            points[1][1],
            points[2][0],
            points[2][1],
        ],
        color,
    });
}

fn cp932_text(value: &str, entity: Option<&Entity>, context: &mut ExportContext<'_>) -> String {
    let (encoded, _, had_errors) = SHIFT_JIS.encode(value);
    if !had_errors {
        return value.to_owned();
    }
    context.block(
        entity,
        "unencodable_text",
        format!("text {value:?} cannot be represented in CP932"),
    );
    if context.options.allow_lossy {
        let (decoded, _, _) = SHIFT_JIS.decode(&encoded);
        decoded.into_owned()
    } else {
        String::new()
    }
}

fn publish(output: &Path, bytes: &[u8], overwrite: bool) -> ExportResult<()> {
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| ExportError::Write {
        path: parent.to_path_buf(),
        source,
    })?;
    let staging = tempfile::Builder::new()
        .prefix(".cad-jww-export-")
        .tempfile_in(parent)
        .map_err(|source| ExportError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    fs::write(staging.path(), bytes).map_err(|source| ExportError::Write {
        path: staging.path().to_path_buf(),
        source,
    })?;
    if overwrite {
        fs::rename(staging.path(), output).map_err(|source| ExportError::Write {
            path: output.to_path_buf(),
            source,
        })?;
    } else {
        renameat_with(CWD, staging.path(), CWD, output, RenameFlags::NOREPLACE).map_err(
            |error| {
                if error == Errno::EXIST || error == Errno::NOTEMPTY {
                    ExportError::OutputExists(output.to_path_buf())
                } else {
                    ExportError::Write {
                        path: output.to_path_buf(),
                        source: std::io::Error::from_raw_os_error(error.raw_os_error()),
                    }
                }
            },
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(crate_name(), "cad-export-jww");
    }

    #[test]
    fn imported_fixture_round_trips_supported_entities() {
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/jww-fixtures/Test1.jww");
        let temp = tempfile::tempdir().expect("tempdir");
        let imported = temp.path().join("imported");
        let exported = temp.path().join("exported.jww");
        let reimported = temp.path().join("reimported");
        let source_report = cad_import_jww::import_jww_file(&fixture, &imported).expect("import");
        let report = export_jww_file(
            &imported,
            "test1",
            &exported,
            ExportOptions {
                allow_lossy: true,
                overwrite: false,
            },
        )
        .expect("export");
        assert_eq!(report.status, ExportStatus::Exported);
        assert!(!report.warnings.is_empty());
        let round_trip = cad_import_jww::import_jww_file(&exported, &reimported).expect("reimport");
        assert_eq!(
            round_trip.supported_entities,
            source_report.supported_entities
        );
        assert_eq!(report.expanded_entities, source_report.supported_entities);
    }

    #[test]
    fn strict_blocker_does_not_publish_output() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let mut project = cad_model::load_project(root).expect("example");
        let entity: Entity = serde_json::from_str(
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000009","type":"block_ref","layer":"0-1","block":"door","at":[0.0,0.0],"rotation_deg":0.0,"scale":1.0}"#,
        )
        .expect("block entity");
        project.drawings[0]
            .entities
            .push(cad_model::EntityRecord { line: 2, entity });
        let temp = tempfile::tempdir().expect("tempdir");
        let output = temp.path().join("blocked.jww");
        let report = export_loaded_project(&project, "plan_1f", &output, ExportOptions::default())
            .expect("blocked report");
        assert_eq!(report.status, ExportStatus::Blocked);
        assert!(!output.exists());
        assert!(
            report
                .blockers
                .iter()
                .any(|issue| issue.code == "unsupported_block_ref")
        );
    }

    #[test]
    fn missing_stroke_color_is_reported_and_not_counted_as_written() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let mut project = cad_model::load_project(root).expect("example");
        project
            .layers
            .layers
            .get_mut("0-1")
            .expect("fixture layer")
            .color = "missing-color".to_owned();
        let temp = tempfile::tempdir().expect("tempdir");

        let strict_output = temp.path().join("strict.jww");
        let strict = export_loaded_project(
            &project,
            "plan_1f",
            &strict_output,
            ExportOptions::default(),
        )
        .expect("strict report");
        assert_eq!(strict.status, ExportStatus::Blocked);
        assert_eq!(strict.written_entities, 0);
        assert!(!strict_output.exists());
        assert!(
            strict
                .blockers
                .iter()
                .any(|issue| issue.code == "undefined_stroke_color")
        );

        let lossy_output = temp.path().join("lossy.jww");
        let lossy = export_loaded_project(
            &project,
            "plan_1f",
            &lossy_output,
            ExportOptions {
                allow_lossy: true,
                overwrite: false,
            },
        )
        .expect("lossy report");
        assert_eq!(lossy.status, ExportStatus::Blocked);
        assert_eq!(lossy.written_entities, 0);
        assert_eq!(lossy.expanded_entities, 0);
        assert!(!lossy_output.exists());
        assert!(
            lossy
                .blockers
                .iter()
                .any(|issue| issue.code == "invalid_project")
        );
        assert!(
            lossy
                .warnings
                .iter()
                .any(|issue| issue.code == "undefined_stroke_color")
        );
    }

    #[test]
    fn polygon_and_curve_solids_round_trip_through_version_600() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let mut project = cad_model::load_project(root).expect("example");
        for source in [
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000008","type":"solid","layer":"0-1","points":[[0.0,0.0],[100.0,0.0],[100.0,50.0],[0.0,50.0]],"fill":"jw_black"}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000007","type":"curve_solid","layer":"0-1","center":[200.0,200.0],"radius":100.0,"flatness":0.5,"rotation_deg":30.0,"start_deg":0.0,"end_deg":180.0,"solid_param":20.0,"encoding_code":1,"fill":"jw_black"}"#,
        ] {
            let entity: Entity = serde_json::from_str(source).expect("solid entity");
            project.drawings[0]
                .entities
                .push(cad_model::EntityRecord { line: 2, entity });
        }

        let temp = tempfile::tempdir().expect("tempdir");
        let output = temp.path().join("solid-roundtrip.jww");
        let reimported = temp.path().join("solid-roundtrip");
        let report = export_loaded_project(&project, "plan_1f", &output, ExportOptions::default())
            .expect("solid export");
        assert_eq!(report.status, ExportStatus::Exported);

        cad_import_jww::import_jww_file(&output, &reimported).expect("solid reimport");
        let reimported_check = cad_check::check_project(&reimported);
        assert!(
            reimported_check.is_ok(),
            "solid round-trip should remain checker-valid: {reimported_check:?}"
        );
        let entities = fs::read_to_string(
            reimported
                .join("drawings")
                .join("solid-roundtrip")
                .join("entities.ndjson"),
        )
        .expect("reimported entities");
        assert!(entities.contains("\"type\":\"solid\""));
        assert!(entities.contains("\"type\":\"curve_solid\""));
    }

    #[test]
    fn simple_hatch_is_emitted_as_solid_polygons_with_a_warning() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let mut project = cad_model::load_project(root).expect("example");
        let entity: Entity = serde_json::from_str(
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000010","type":"hatch","layer":"0-1","loops":[[[0.0,0.0],[100.0,0.0],[100.0,100.0],[0.0,100.0]]],"pattern":"solid","angle_deg":30.0,"scale":2.0,"fill":"jw_black"}"#,
        )
        .expect("hatch entity");
        project.drawings[0]
            .entities
            .push(cad_model::EntityRecord { line: 2, entity });

        let temp = tempfile::tempdir().expect("tempdir");
        let output = temp.path().join("hatch.jww");
        let report = export_loaded_project(&project, "plan_1f", &output, ExportOptions::default())
            .expect("hatch export");
        assert_eq!(report.status, ExportStatus::Exported);
        assert!(
            report
                .warnings
                .iter()
                .any(|issue| issue.code == "hatch_pattern_approximated")
        );

        let decoded = cad_jww_codec::read_document(&fs::read(&output).expect("JWW bytes"))
            .expect("JWW should decode");
        assert!(
            decoded
                .entities
                .iter()
                .any(|entity| matches!(entity, cad_jww_codec::DecodedEntity::Solid(_)))
        );
    }

    #[test]
    fn block_export_offsets_jww_reference_by_definition_base_point() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let mut project = cad_model::load_project(root).expect("example");
        let child: Entity = serde_json::from_str(
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000098","type":"line","layer":"0-1","p1":[10.0,20.0],"p2":[20.0,20.0]}"#,
        )
        .expect("block child");
        project.blocks.insert(
            "door_test".to_owned(),
            cad_model::BlockDefinition {
                id: "door_test".to_owned(),
                config: cad_model::BlockDefinitionConfig {
                    schema_version: cad_model::CURRENT_SCHEMA_VERSION.to_owned(),
                    name: "Door test".to_owned(),
                    base_point: [10.0, 20.0],
                },
                entities: vec![cad_model::EntityRecord {
                    line: 1,
                    entity: child,
                }],
            },
        );
        let reference: Entity = serde_json::from_str(
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000099","type":"block_ref","layer":"0-1","block":"door_test","at":[100.0,200.0],"rotation_deg":90.0,"scale":2.0}"#,
        )
        .expect("block reference");
        project.drawings[0].entities.push(cad_model::EntityRecord {
            line: 2,
            entity: reference,
        });
        let temp = tempfile::tempdir().expect("tempdir");
        let output = temp.path().join("base-point.jww");

        let report = export_loaded_project(&project, "plan_1f", &output, ExportOptions::default())
            .expect("export");
        assert_eq!(report.status, ExportStatus::Exported);
        let decoded = cad_jww_codec::read_document(
            &fs::read(output).expect("exported JWW should be readable"),
        )
        .expect("exported JWW should decode");
        let block = decoded
            .entities
            .iter()
            .find_map(|entity| match entity {
                cad_jww_codec::DecodedEntity::Block(block) => Some(block),
                _ => None,
            })
            .expect("block reference should be exported");
        assert!((block.ref_x - 140.0).abs() < 1e-9);
        assert!((block.ref_y - 180.0).abs() < 1e-9);
    }
}
