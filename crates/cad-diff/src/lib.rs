//! Semantic CAD diff crate.
//!
//! This crate compares CAD source by stable entity IDs and emits JSON/SVG
//! review artifacts. It does not mutate either project.

use cad_model::{
    BBox, Entity, EntityRecord, Point, ProjectSource, TextAlign, TextStyleDef, entity_bbox,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path as FsPath;
use svg::Document;
use svg::node::element::path::Data;
use svg::node::element::{Circle, Ellipse as SvgEllipse, Group, Line, Path, Polyline, Text};

pub const CRATE_NAME: &str = "cad-diff";
pub const DIFF_SCHEMA_VERSION: &str = "0.2";

#[must_use]
pub fn crate_name() -> &'static str {
    CRATE_NAME
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiffStatus {
    Ok,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
    Unchanged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ChangeReason {
    GeometryChanged,
    LayerChanged,
    StyleChanged,
    TextChanged,
    LineWidthChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WarningKind {
    TextOverlap,
    OutsidePaper,
    LineWidthChanged,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiffChange {
    pub entity_id: String,
    pub drawing: String,
    pub kind: ChangeKind,
    pub reasons: Vec<ChangeReason>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiffWarning {
    pub kind: WarningKind,
    pub entity_ids: Vec<String>,
    pub drawing: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationChangeKind {
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigurationChange {
    pub path: String,
    pub kind: ConfigurationChangeKind,
    pub before: Option<serde_json::Value>,
    pub after: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiffReport {
    pub schema_version: String,
    pub status: DiffStatus,
    pub changes: Vec<DiffChange>,
    pub warnings: Vec<DiffWarning>,
    pub configuration_changes: Vec<ConfigurationChange>,
}

impl DiffReport {
    #[must_use]
    pub fn new(
        changes: Vec<DiffChange>,
        warnings: Vec<DiffWarning>,
        configuration_changes: Vec<ConfigurationChange>,
    ) -> Self {
        let status = if warnings.is_empty() {
            DiffStatus::Ok
        } else {
            DiffStatus::Warning
        };
        Self {
            schema_version: DIFF_SCHEMA_VERSION.to_owned(),
            status,
            changes,
            warnings,
            configuration_changes,
        }
    }
}

pub fn diff_projects(base: &ProjectSource, head: &ProjectSource) -> DiffReport {
    let mut changes = Vec::new();
    let mut warnings = Vec::new();

    let base_drawings = drawings_by_name(base);
    let head_drawings = drawings_by_name(head);
    let drawing_names = base_drawings
        .keys()
        .chain(head_drawings.keys())
        .cloned()
        .collect::<BTreeSet<_>>();

    for drawing_name in drawing_names {
        let base_entities = base_drawings
            .get(&drawing_name)
            .map_or_else(BTreeMap::new, |records| entities_by_id(records));
        let head_entities = head_drawings
            .get(&drawing_name)
            .map_or_else(BTreeMap::new, |records| entities_by_id(records));
        let entity_ids = base_entities
            .keys()
            .chain(head_entities.keys())
            .cloned()
            .collect::<BTreeSet<_>>();

        for entity_id in entity_ids {
            match (base_entities.get(&entity_id), head_entities.get(&entity_id)) {
                (None, Some(head_record)) => {
                    changes.push(DiffChange {
                        entity_id,
                        drawing: drawing_name.clone(),
                        kind: ChangeKind::Added,
                        reasons: Vec::new(),
                    });
                    push_head_warnings(head, &drawing_name, head_record, &mut warnings);
                }
                (Some(_base_record), None) => {
                    changes.push(DiffChange {
                        entity_id,
                        drawing: drawing_name.clone(),
                        kind: ChangeKind::Removed,
                        reasons: Vec::new(),
                    });
                }
                (Some(base_record), Some(head_record)) => {
                    let reasons =
                        change_reasons(base, head, &base_record.entity, &head_record.entity);
                    let kind = if reasons.is_empty() {
                        ChangeKind::Unchanged
                    } else {
                        ChangeKind::Modified
                    };
                    changes.push(DiffChange {
                        entity_id,
                        drawing: drawing_name.clone(),
                        kind,
                        reasons: reasons.clone(),
                    });
                    if reasons.contains(&ChangeReason::LineWidthChanged) {
                        warnings.push(DiffWarning {
                            kind: WarningKind::LineWidthChanged,
                            entity_ids: vec![head_record.entity.id().as_str().to_owned()],
                            drawing: drawing_name.clone(),
                            message: "line width changed".to_owned(),
                        });
                    }
                    push_head_warnings(head, &drawing_name, head_record, &mut warnings);
                }
                (None, None) => {}
            }
        }

        if let Some(records) = head_drawings.get(&drawing_name) {
            warnings.extend(text_overlap_warnings(head, &drawing_name, records));
        }
    }

    DiffReport::new(changes, warnings, configuration_changes(base, head))
}

fn configuration_changes(base: &ProjectSource, head: &ProjectSource) -> Vec<ConfigurationChange> {
    let mut changes = Vec::new();
    collect_json_changes(
        "project",
        &serde_json::to_value(&base.project).expect("project config is serializable"),
        &serde_json::to_value(&head.project).expect("project config is serializable"),
        &mut changes,
    );
    collect_json_changes(
        "layers",
        &serde_json::to_value(&base.layers).expect("layer rules are serializable"),
        &serde_json::to_value(&head.layers).expect("layer rules are serializable"),
        &mut changes,
    );
    collect_json_changes(
        "styles",
        &serde_json::to_value(&base.styles).expect("style rules are serializable"),
        &serde_json::to_value(&head.styles).expect("style rules are serializable"),
        &mut changes,
    );
    let base_sheets = base
        .drawings
        .iter()
        .map(|drawing| (&drawing.name, &drawing.sheet))
        .collect::<BTreeMap<_, _>>();
    let head_sheets = head
        .drawings
        .iter()
        .map(|drawing| (&drawing.name, &drawing.sheet))
        .collect::<BTreeMap<_, _>>();
    for name in base_sheets
        .keys()
        .chain(head_sheets.keys())
        .copied()
        .collect::<BTreeSet<_>>()
    {
        let before = base_sheets
            .get(name)
            .map(|sheet| serde_json::to_value(sheet).expect("sheet is serializable"));
        let after = head_sheets
            .get(name)
            .map(|sheet| serde_json::to_value(sheet).expect("sheet is serializable"));
        collect_optional_json_changes(
            &format!("drawings.{name}.sheet"),
            before.as_ref(),
            after.as_ref(),
            &mut changes,
        );
    }
    let base_blocks = block_file_signatures(&base.root);
    let head_blocks = block_file_signatures(&head.root);
    for path in base_blocks
        .keys()
        .chain(head_blocks.keys())
        .collect::<BTreeSet<_>>()
    {
        let before = base_blocks
            .get(path)
            .map(|value| serde_json::Value::String(value.clone()));
        let after = head_blocks
            .get(path)
            .map(|value| serde_json::Value::String(value.clone()));
        collect_optional_json_changes(
            &format!("blocks.{path}"),
            before.as_ref(),
            after.as_ref(),
            &mut changes,
        );
    }
    changes
}

fn block_file_signatures(root: &FsPath) -> BTreeMap<String, String> {
    fn visit(root: &FsPath, current: &FsPath, output: &mut BTreeMap<String, String>) {
        let Ok(entries) = fs::read_dir(current) else {
            return;
        };
        let mut entries = entries.filter_map(Result::ok).collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, output);
            } else if let Ok(bytes) = fs::read(&path)
                && let Ok(relative) = path.strip_prefix(root)
            {
                output.insert(
                    relative.to_string_lossy().replace('\\', "/"),
                    blake3::hash(&bytes).to_hex().to_string(),
                );
            }
        }
    }
    let blocks = root.join("blocks");
    let mut output = BTreeMap::new();
    visit(&blocks, &blocks, &mut output);
    output
}

fn collect_optional_json_changes(
    path: &str,
    before: Option<&serde_json::Value>,
    after: Option<&serde_json::Value>,
    output: &mut Vec<ConfigurationChange>,
) {
    match (before, after) {
        (Some(before), Some(after)) => collect_json_changes(path, before, after, output),
        (Some(before), None) => output.push(ConfigurationChange {
            path: path.to_owned(),
            kind: ConfigurationChangeKind::Removed,
            before: Some(before.clone()),
            after: None,
        }),
        (None, Some(after)) => output.push(ConfigurationChange {
            path: path.to_owned(),
            kind: ConfigurationChangeKind::Added,
            before: None,
            after: Some(after.clone()),
        }),
        (None, None) => {}
    }
}

fn collect_json_changes(
    path: &str,
    before: &serde_json::Value,
    after: &serde_json::Value,
    output: &mut Vec<ConfigurationChange>,
) {
    if before == after {
        return;
    }
    if let (serde_json::Value::Object(before), serde_json::Value::Object(after)) = (before, after) {
        for key in before.keys().chain(after.keys()).collect::<BTreeSet<_>>() {
            collect_optional_json_changes(
                &format!("{path}.{key}"),
                before.get(key),
                after.get(key),
                output,
            );
        }
        return;
    }
    output.push(ConfigurationChange {
        path: path.to_owned(),
        kind: ConfigurationChangeKind::Modified,
        before: Some(before.clone()),
        after: Some(after.clone()),
    });
}

pub fn diff_projects_json(
    base: &ProjectSource,
    head: &ProjectSource,
) -> serde_json::Result<String> {
    serde_json::to_string_pretty(&diff_projects(base, head))
}

pub fn diff_projects_svg(base: &ProjectSource, head: &ProjectSource) -> String {
    let report = diff_projects(base, head);
    let base_drawings = drawings_by_name(base);
    let head_drawings = drawings_by_name(head);
    let mut root = Group::new().set("id", "semantic-diff");
    let mut view_bbox: Option<BBox> = None;

    for change in &report.changes {
        match change.kind {
            ChangeKind::Added => {
                if let Some(record) = head_drawings
                    .get(&change.drawing)
                    .and_then(|records| find_record(records, &change.entity_id))
                {
                    view_bbox = merge_view_bbox(view_bbox, visual_bbox(head, record));
                    root = root.add(render_overlay_entity(
                        head, record, "#00AA00", 0.95, "added",
                    ));
                }
            }
            ChangeKind::Removed => {
                if let Some(record) = base_drawings
                    .get(&change.drawing)
                    .and_then(|records| find_record(records, &change.entity_id))
                {
                    view_bbox = merge_view_bbox(view_bbox, visual_bbox(base, record));
                    root = root.add(render_overlay_entity(
                        base, record, "#DD2222", 0.95, "removed",
                    ));
                }
            }
            ChangeKind::Modified => {
                if let Some(record) = base_drawings
                    .get(&change.drawing)
                    .and_then(|records| find_record(records, &change.entity_id))
                {
                    view_bbox = merge_view_bbox(view_bbox, visual_bbox(base, record));
                    root = root.add(render_overlay_entity(
                        base,
                        record,
                        "#DD2222",
                        0.45,
                        "modified-base",
                    ));
                }
                if let Some(record) = head_drawings
                    .get(&change.drawing)
                    .and_then(|records| find_record(records, &change.entity_id))
                {
                    view_bbox = merge_view_bbox(view_bbox, visual_bbox(head, record));
                    root = root.add(render_overlay_entity(
                        head, record, "#D6B300", 0.95, "modified",
                    ));
                }
            }
            ChangeKind::Unchanged => {
                if let Some(record) = head_drawings
                    .get(&change.drawing)
                    .and_then(|records| find_record(records, &change.entity_id))
                {
                    view_bbox = merge_view_bbox(view_bbox, visual_bbox(head, record));
                    root = root.add(render_overlay_entity(
                        head,
                        record,
                        "#999999",
                        0.25,
                        "unchanged",
                    ));
                }
            }
        }
    }

    let view_box = view_bbox.map_or((-100.0, -100.0, 200.0, 200.0), bbox_view_box);
    Document::new()
        .set("viewBox", view_box)
        .set("width", "100%")
        .set("height", "100%")
        .set("preserveAspectRatio", "xMinYMin meet")
        .set("data-diff-schema-version", DIFF_SCHEMA_VERSION)
        .add(root)
        .to_string()
}

fn drawings_by_name(project: &ProjectSource) -> BTreeMap<String, &[EntityRecord]> {
    project
        .drawings
        .iter()
        .map(|drawing| (drawing.name.clone(), drawing.entities.as_slice()))
        .collect()
}

fn entities_by_id(records: &[EntityRecord]) -> BTreeMap<String, &EntityRecord> {
    records
        .iter()
        .map(|record| (record.entity.id().as_str().to_owned(), record))
        .collect()
}

fn find_record<'a>(records: &'a [EntityRecord], entity_id: &str) -> Option<&'a EntityRecord> {
    records
        .iter()
        .find(|record| record.entity.id().as_str() == entity_id)
}

fn change_reasons(
    base_project: &ProjectSource,
    head_project: &ProjectSource,
    base: &Entity,
    head: &Entity,
) -> Vec<ChangeReason> {
    let mut reasons = BTreeSet::new();

    if entity_geometry_signature(base) != entity_geometry_signature(head) {
        reasons.insert(ChangeReason::GeometryChanged);
    }
    if base.layer() != head.layer() {
        reasons.insert(ChangeReason::LayerChanged);
    }
    if entity_style_signature(base) != entity_style_signature(head) {
        reasons.insert(ChangeReason::StyleChanged);
    }
    if resolved_style_signature(base_project, base) != resolved_style_signature(head_project, head)
    {
        reasons.insert(ChangeReason::StyleChanged);
    }
    if entity_text_signature(base) != entity_text_signature(head) {
        reasons.insert(ChangeReason::TextChanged);
    }
    if line_width(base_project, base) != line_width(head_project, head) {
        reasons.insert(ChangeReason::LineWidthChanged);
    }

    reasons.into_iter().collect()
}

fn resolved_style_signature(project: &ProjectSource, entity: &Entity) -> String {
    let layer = project.layers.layers.get(entity.layer());
    let pen = entity.pen().and_then(|id| project.styles.pens.get(id));
    let color_id = pen.map_or_else(
        || layer.map(|value| value.color.as_str()),
        |value| Some(value.color.as_str()),
    );
    let line_type_id = pen.map_or_else(
        || layer.map(|value| value.line_type.as_str()),
        |value| Some(value.line_type.as_str()),
    );
    let color = color_id.and_then(|id| project.styles.colors.get(id));
    let line_type = line_type_id.and_then(|id| project.styles.line_types.get(id));
    let annotation = match entity {
        Entity::Text { style, .. } => {
            serde_json::to_string(&project.styles.text_styles.get(style)).ok()
        }
        Entity::Dimension { style, .. } => {
            project
                .styles
                .dimension_styles
                .get(style)
                .and_then(|dimension| {
                    serde_json::to_string(&(
                        dimension,
                        project.styles.text_styles.get(&dimension.text_style),
                    ))
                    .ok()
                })
        }
        _ => None,
    };
    format!("{layer:?}:{pen:?}:{color:?}:{line_type:?}:{annotation:?}")
}

fn entity_geometry_signature(entity: &Entity) -> String {
    match entity {
        Entity::Line { p1, p2, .. } => format!("line:{p1:?}:{p2:?}"),
        Entity::Polyline { points, closed, .. } => format!("polyline:{points:?}:{closed}"),
        Entity::Arc {
            center,
            radius,
            start_deg,
            end_deg,
            ..
        } => format!("arc:{center:?}:{radius}:{start_deg}:{end_deg}"),
        Entity::Circle { center, radius, .. } => format!("circle:{center:?}:{radius}"),
        Entity::Ellipse {
            center,
            radius_x,
            radius_y,
            rotation_deg,
            start_deg,
            end_deg,
            ..
        } => {
            format!("ellipse:{center:?}:{radius_x}:{radius_y}:{rotation_deg}:{start_deg}:{end_deg}")
        }
        Entity::Text {
            at,
            rotation_deg,
            mirror_y,
            ..
        } => format!("text:{at:?}:{rotation_deg}:{mirror_y}"),
        Entity::Point {
            at,
            temporary,
            marker_code,
            rotation_deg,
            scale,
            ..
        } => format!("point:{at:?}:{temporary}:{marker_code:?}:{rotation_deg}:{scale}"),
        Entity::Solid { points, fill, .. } => format!("solid:{points:?}:{fill}"),
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
        } => format!(
            "curve_solid:{center:?}:{radius}:{flatness}:{rotation_deg}:{start_deg}:{end_deg}:{solid_param}:{encoding_code}:{fill}"
        ),
        Entity::Dimension {
            p1,
            p2,
            offset,
            text_rotation_deg,
            text_mirror_y,
            ..
        } => format!("dimension:{p1:?}:{p2:?}:{offset}:{text_rotation_deg}:{text_mirror_y}"),
        Entity::BlockRef {
            block,
            at,
            rotation_deg,
            scale,
            ..
        } => format!("block_ref:{block}:{at:?}:{rotation_deg}:{scale}"),
    }
}

fn entity_style_signature(entity: &Entity) -> String {
    let annotation = match entity {
        Entity::Text { style, .. } | Entity::Dimension { style, .. } => style.as_str(),
        _ => "",
    };
    format!("{annotation}:{}", entity.pen().unwrap_or_default())
}

fn entity_text_signature(entity: &Entity) -> String {
    match entity {
        Entity::Text { value, .. } => value.clone(),
        Entity::Dimension { value, .. } => value.clone().unwrap_or_default(),
        _ => String::new(),
    }
}

fn line_width(project: &ProjectSource, entity: &Entity) -> Option<String> {
    if matches!(entity, Entity::Text { .. }) {
        return None;
    }
    project
        .styles
        .pens
        .get(entity.pen().unwrap_or_default())
        .map(|pen| pen.line_width)
        .or_else(|| {
            project
                .layers
                .layers
                .get(entity.layer())
                .map(|layer| layer.line_width)
        })
        .map(cad_model::format_decimal_mm)
}

fn push_head_warnings(
    project: &ProjectSource,
    drawing_name: &str,
    record: &EntityRecord,
    warnings: &mut Vec<DiffWarning>,
) {
    if is_outside_paper(project, drawing_name, &record.entity) {
        warnings.push(DiffWarning {
            kind: WarningKind::OutsidePaper,
            entity_ids: vec![record.entity.id().as_str().to_owned()],
            drawing: drawing_name.to_owned(),
            message: "entity is outside paper bounds".to_owned(),
        });
    }
}

fn is_outside_paper(project: &ProjectSource, drawing_name: &str, entity: &Entity) -> bool {
    let Some(drawing) = project
        .drawings
        .iter()
        .find(|drawing| drawing.name == drawing_name)
    else {
        return false;
    };
    let Some(bbox) = entity_bbox(entity) else {
        return false;
    };
    let Some((paper_width, paper_height)) = paper_model_size(&drawing.sheet) else {
        return false;
    };
    bbox.min[0] < drawing.sheet.origin[0]
        || bbox.min[1] < drawing.sheet.origin[1]
        || bbox.max[0] > drawing.sheet.origin[0] + paper_width
        || bbox.max[1] > drawing.sheet.origin[1] + paper_height
}

fn paper_model_size(sheet: &cad_model::SheetConfig) -> Option<(f64, f64)> {
    let (paper_width, paper_height) = match sheet.paper.as_str() {
        "A0" => (841.0, 1189.0),
        "A1" => (594.0, 841.0),
        "A2" => (420.0, 594.0),
        "A3" => (297.0, 420.0),
        "A4" => (210.0, 297.0),
        _ => return None,
    };
    let (paper_width, paper_height) = match sheet.orientation {
        cad_model::SheetOrientation::Portrait => (paper_width, paper_height),
        cad_model::SheetOrientation::Landscape => (paper_height, paper_width),
    };
    parse_scale(&sheet.scale).map(|scale| (paper_width * scale, paper_height * scale))
}

fn parse_scale(scale: &str) -> Option<f64> {
    let (numerator, denominator) = scale.split_once('/')?;
    let numerator = numerator.parse::<f64>().ok()?;
    let denominator = denominator.parse::<f64>().ok()?;
    if numerator <= 0.0 || denominator <= 0.0 {
        return None;
    }
    Some(denominator / numerator)
}

fn text_overlap_warnings(
    project: &ProjectSource,
    drawing_name: &str,
    records: &[EntityRecord],
) -> Vec<DiffWarning> {
    let texts = records
        .iter()
        .filter(|record| entity_is_effectively_visible(project, &record.entity))
        .filter_map(|record| text_bbox(project, record).map(|bbox| (record, bbox)))
        .collect::<Vec<_>>();
    let mut warnings = Vec::new();
    for left_index in 0..texts.len() {
        for right_index in (left_index + 1)..texts.len() {
            let (left, left_bbox) = texts[left_index];
            let (right, right_bbox) = texts[right_index];
            if bboxes_overlap(left_bbox, right_bbox) {
                warnings.push(DiffWarning {
                    kind: WarningKind::TextOverlap,
                    entity_ids: vec![
                        left.entity.id().as_str().to_owned(),
                        right.entity.id().as_str().to_owned(),
                    ],
                    drawing: drawing_name.to_owned(),
                    message: "text bounding boxes overlap".to_owned(),
                });
            }
        }
    }
    warnings
}

fn entity_is_effectively_visible(project: &ProjectSource, entity: &Entity) -> bool {
    let Some(layer) = project.layers.layers.get(entity.layer()) else {
        return true;
    };
    layer.visible
        && layer
            .group
            .as_ref()
            .and_then(|id| project.layers.groups.get(id))
            .is_none_or(|group| group.visible)
}

fn text_bbox(project: &ProjectSource, record: &EntityRecord) -> Option<BBox> {
    match &record.entity {
        Entity::Text {
            style,
            at,
            rotation_deg,
            mirror_y,
            value,
            ..
        } => project
            .styles
            .text_styles
            .get(style)
            .and_then(|text_style| {
                text_bbox_for_style(
                    at,
                    *rotation_deg,
                    *mirror_y,
                    value,
                    text_style,
                    text_anchor(&text_style.align),
                )
            }),
        _ => None,
    }
}

fn visual_bbox(project: &ProjectSource, record: &EntityRecord) -> Option<BBox> {
    match &record.entity {
        Entity::Text { .. } => text_bbox(project, record).or_else(|| entity_bbox(&record.entity)),
        Entity::Dimension { .. } => {
            dimension_bbox(project, &record.entity).or_else(|| entity_bbox(&record.entity))
        }
        _ => entity_bbox(&record.entity),
    }
}

fn dimension_bbox(project: &ProjectSource, entity: &Entity) -> Option<BBox> {
    let Entity::Dimension {
        style,
        p1,
        p2,
        offset,
        text_rotation_deg,
        text_mirror_y,
        value,
        ..
    } = entity
    else {
        return None;
    };
    let dimension_style = project.styles.dimension_styles.get(style)?;
    let text_style = project
        .styles
        .text_styles
        .get(&dimension_style.text_style)?;
    let (d1, d2) = cad_model::dimension_offset_segment(*p1, *p2, *offset)?;
    let label = dimension_label(p1, p2, value);
    let line_bbox = BBox::from_points(&[*p1, *p2, d1, d2])?;
    let text_bbox = text_bbox_for_style(
        &[(d1[0] + d2[0]) / 2.0, (d1[1] + d2[1]) / 2.0],
        *text_rotation_deg,
        *text_mirror_y,
        &label,
        text_style,
        "middle",
    )?;
    Some(union_bbox(line_bbox, text_bbox))
}

fn text_bbox_for_style(
    at: &Point,
    rotation_deg: f64,
    mirror_y: bool,
    value: &str,
    style: &TextStyleDef,
    anchor: &str,
) -> Option<BBox> {
    let width = text_length(value, style);
    let height = style.height.max(0.0);
    if !at[0].is_finite()
        || !at[1].is_finite()
        || !rotation_deg.is_finite()
        || !width.is_finite()
        || !height.is_finite()
    {
        return None;
    }
    let x0 = match anchor {
        "middle" => at[0] - (width / 2.0),
        "end" => at[0] - width,
        _ => at[0],
    };
    let (y0, y1) = if mirror_y {
        (at[1] - height, at[1])
    } else {
        (at[1], at[1] + height)
    };
    let corners = [[x0, y0], [x0 + width, y0], [x0, y1], [x0 + width, y1]];
    BBox::from_points(&corners.map(|point| rotate_point(point, *at, rotation_deg)))
}

fn bboxes_overlap(left: BBox, right: BBox) -> bool {
    left.min[0] <= right.max[0]
        && left.max[0] >= right.min[0]
        && left.min[1] <= right.max[1]
        && left.max[1] >= right.min[1]
}

fn merge_view_bbox(left: Option<BBox>, right: Option<BBox>) -> Option<BBox> {
    match (left, right) {
        (None, None) => None,
        (Some(bbox), None) | (None, Some(bbox)) => Some(bbox),
        (Some(left), Some(right)) => Some(BBox {
            min: [left.min[0].min(right.min[0]), left.min[1].min(right.min[1])],
            max: [left.max[0].max(right.max[0]), left.max[1].max(right.max[1])],
        }),
    }
}

fn union_bbox(left: BBox, right: BBox) -> BBox {
    BBox {
        min: [left.min[0].min(right.min[0]), left.min[1].min(right.min[1])],
        max: [left.max[0].max(right.max[0]), left.max[1].max(right.max[1])],
    }
}

fn bbox_view_box(bbox: BBox) -> (f64, f64, f64, f64) {
    let largest = bbox.width().max(bbox.height());
    let padding = 100.0_f64.max(largest * 0.05);
    (
        bbox.min[0] - padding,
        -bbox.max[1] - padding,
        bbox.width() + (padding * 2.0),
        bbox.height() + (padding * 2.0),
    )
}

fn render_overlay_entity(
    project: &ProjectSource,
    record: &EntityRecord,
    color: &str,
    opacity: f64,
    class_name: &str,
) -> Group {
    let layer_visible = entity_is_effectively_visible(project, &record.entity);
    let mut group = Group::new()
        .set("class", class_name)
        .set("data-entity-id", record.entity.id().as_str())
        .set("data-layer", record.entity.layer())
        .set("data-layer-visible", layer_visible)
        .set("opacity", opacity)
        .set("stroke", color)
        .set("fill", "none")
        .set("stroke-width", "0.35mm");
    if !layer_visible {
        group = group.set("style", "display:none");
    }
    if let Some(bbox) = visual_bbox(project, record) {
        group = group.set("data-bbox", bbox_attr(bbox));
    }

    match &record.entity {
        Entity::Line { p1, p2, .. } => {
            group = group.add(
                Line::new()
                    .set("x1", p1[0])
                    .set("y1", svg_y(p1[1]))
                    .set("x2", p2[0])
                    .set("y2", svg_y(p2[1])),
            );
        }
        Entity::Polyline { points, closed, .. } => {
            if *closed {
                group = group.add(path_from_points(points, true));
            } else {
                group = group.add(Polyline::new().set("points", points_attr(points)));
            }
        }
        Entity::Arc {
            center,
            radius,
            start_deg,
            end_deg,
            ..
        } => {
            group = group.add(arc_path(*center, *radius, *start_deg, *end_deg));
        }
        Entity::Circle { center, radius, .. } => {
            group = group.add(
                Circle::new()
                    .set("cx", center[0])
                    .set("cy", svg_y(center[1]))
                    .set("r", *radius),
            );
        }
        Entity::Ellipse {
            center,
            radius_x,
            radius_y,
            rotation_deg,
            start_deg,
            end_deg,
            ..
        } => {
            if (end_deg - start_deg).abs() >= 360.0 - 1e-9 {
                group = group.add(
                    SvgEllipse::new()
                        .set("cx", center[0])
                        .set("cy", svg_y(center[1]))
                        .set("rx", *radius_x)
                        .set("ry", *radius_y)
                        .set(
                            "transform",
                            format!(
                                "rotate({} {} {})",
                                normalize_zero(-rotation_deg),
                                center[0],
                                svg_y(center[1])
                            ),
                        ),
                );
            } else {
                group = group.add(ellipse_arc_path(
                    *center,
                    *radius_x,
                    *radius_y,
                    *rotation_deg,
                    *start_deg,
                    *end_deg,
                ));
            }
        }
        Entity::Text {
            style,
            at,
            rotation_deg,
            mirror_y,
            value,
            ..
        } => {
            if let Some(text_style) = project.styles.text_styles.get(style) {
                group = group.add(text_node(
                    at,
                    *rotation_deg,
                    *mirror_y,
                    value,
                    text_style,
                    text_anchor(&text_style.align),
                    color,
                ));
            } else {
                group = group.add(
                    Text::new("")
                        .set("x", at[0])
                        .set("y", svg_y(at[1]))
                        .set("fill", color)
                        .set("stroke", "none")
                        .set("font-size", 250)
                        .set(
                            "transform",
                            text_transform_attr(*rotation_deg, *mirror_y, *at),
                        )
                        .add(svg::node::Text::new(value.clone())),
                );
            }
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
            let Some((d1, d2)) = cad_model::dimension_offset_segment(*p1, *p2, *offset) else {
                return group;
            };
            let label = dimension_label(p1, p2, value);
            group = group.add(
                Line::new()
                    .set("x1", d1[0])
                    .set("y1", svg_y(d1[1]))
                    .set("x2", d2[0])
                    .set("y2", svg_y(d2[1])),
            );
            if let Some(text_style) =
                project
                    .styles
                    .dimension_styles
                    .get(style)
                    .and_then(|dimension_style| {
                        project.styles.text_styles.get(&dimension_style.text_style)
                    })
            {
                group = group.add(text_node(
                    &[(d1[0] + d2[0]) / 2.0, (d1[1] + d2[1]) / 2.0],
                    *text_rotation_deg,
                    *text_mirror_y,
                    &label,
                    text_style,
                    "middle",
                    color,
                ));
            } else {
                group = group.add(
                    Text::new("")
                        .set("x", (d1[0] + d2[0]) / 2.0)
                        .set("y", svg_y((d1[1] + d2[1]) / 2.0))
                        .set("fill", color)
                        .set("stroke", "none")
                        .set("font-size", 250)
                        .set(
                            "transform",
                            text_transform_attr(
                                *text_rotation_deg,
                                *text_mirror_y,
                                [(d1[0] + d2[0]) / 2.0, (d1[1] + d2[1]) / 2.0],
                            ),
                        )
                        .add(svg::node::Text::new(label)),
                );
            }
        }
        Entity::Point {
            at,
            rotation_deg,
            scale,
            ..
        } => {
            let size = (2.5 * scale.abs()).max(0.5);
            group = group
                .add(
                    Line::new()
                        .set("x1", at[0] - size)
                        .set("y1", svg_y(at[1]))
                        .set("x2", at[0] + size)
                        .set("y2", svg_y(at[1]))
                        .set("transform", rotate_attr(*rotation_deg, *at)),
                )
                .add(
                    Line::new()
                        .set("x1", at[0])
                        .set("y1", svg_y(at[1] - size))
                        .set("x2", at[0])
                        .set("y2", svg_y(at[1] + size))
                        .set("transform", rotate_attr(*rotation_deg, *at)),
                );
        }
        Entity::Solid { points, .. } => {
            group = group.add(path_from_points(points, true).set("fill", color));
        }
        Entity::CurveSolid {
            center,
            radius,
            flatness,
            rotation_deg,
            start_deg,
            end_deg,
            ..
        } => {
            group = group.add(ellipse_arc_path(
                *center,
                radius.abs(),
                radius.abs() * flatness.abs(),
                *rotation_deg,
                *start_deg,
                *end_deg,
            ));
        }
        Entity::BlockRef {
            block,
            at,
            rotation_deg,
            scale,
            ..
        } => {
            let size = 100.0 * scale;
            group = group
                .add(
                    Line::new()
                        .set("x1", at[0] - size)
                        .set("y1", svg_y(at[1]))
                        .set("x2", at[0] + size)
                        .set("y2", svg_y(at[1]))
                        .set("transform", rotate_attr(*rotation_deg, *at)),
                )
                .add(
                    Line::new()
                        .set("x1", at[0])
                        .set("y1", svg_y(at[1] - size))
                        .set("x2", at[0])
                        .set("y2", svg_y(at[1] + size))
                        .set("transform", rotate_attr(*rotation_deg, *at)),
                )
                .add(
                    Text::new("")
                        .set("x", at[0])
                        .set("y", svg_y(at[1] + size * 1.5))
                        .set("fill", color)
                        .set("stroke", "none")
                        .set("font-size", 250)
                        .add(svg::node::Text::new(block.clone())),
                );
        }
    }
    group
}

fn bbox_attr(bbox: BBox) -> String {
    format!(
        "{},{},{},{}",
        cad_model::format_decimal_mm(bbox.min[0]),
        cad_model::format_decimal_mm(bbox.min[1]),
        cad_model::format_decimal_mm(bbox.max[0]),
        cad_model::format_decimal_mm(bbox.max[1])
    )
}

fn text_node(
    at: &Point,
    rotation_deg: f64,
    mirror_y: bool,
    value: &str,
    style: &TextStyleDef,
    anchor: &str,
    color: &str,
) -> Text {
    Text::new("")
        .set("x", at[0])
        .set("y", svg_y(at[1]))
        .set("fill", color)
        .set("stroke", "none")
        .set("font-family", style.font_family.clone())
        .set("font-size", style.height)
        .set("textLength", text_length(value, style))
        .set("lengthAdjust", "spacingAndGlyphs")
        .set("text-anchor", anchor)
        .set(
            "transform",
            text_transform_attr(rotation_deg, mirror_y, *at),
        )
        .add(svg::node::Text::new(value.to_owned()))
}

fn text_length(value: &str, style: &TextStyleDef) -> f64 {
    let count = value.chars().count();
    ((count as f64) * style.width + (count.saturating_sub(1) as f64) * style.spacing).max(0.0)
}

fn text_anchor(align: &TextAlign) -> &'static str {
    match align {
        TextAlign::Left => "start",
        TextAlign::Center => "middle",
        TextAlign::Right => "end",
    }
}

fn dimension_label(p1: &Point, p2: &Point, value: &Option<String>) -> String {
    value.clone().unwrap_or_else(|| {
        cad_model::format_decimal_mm(((p2[0] - p1[0]).powi(2) + (p2[1] - p1[1]).powi(2)).sqrt())
    })
}

fn rotate_attr(rotation_deg: f64, at: Point) -> String {
    format!(
        "rotate({} {} {})",
        normalize_zero(-rotation_deg),
        at[0],
        svg_y(at[1])
    )
}

fn text_transform_attr(rotation_deg: f64, mirror_y: bool, at: Point) -> String {
    if !mirror_y {
        return rotate_attr(rotation_deg, at);
    }
    let screen_rotation = (-rotation_deg).to_radians();
    let a = normalize_zero(screen_rotation.cos());
    let b = normalize_zero(screen_rotation.sin());
    let c = b;
    let d = normalize_zero(-screen_rotation.cos());
    let anchor = [at[0], svg_y(at[1])];
    let e = normalize_zero(anchor[0] - (a * anchor[0] + c * anchor[1]));
    let f = normalize_zero(anchor[1] - (b * anchor[0] + d * anchor[1]));
    format!("matrix({a} {b} {c} {d} {e} {f})")
}

fn rotate_point(point: Point, origin: Point, rotation_deg: f64) -> Point {
    let rad = rotation_deg.to_radians();
    let cos = rad.cos();
    let sin = rad.sin();
    let dx = point[0] - origin[0];
    let dy = point[1] - origin[1];
    [
        origin[0] + (dx * cos) - (dy * sin),
        origin[1] + (dx * sin) + (dy * cos),
    ]
}

fn path_from_points(points: &[Point], close: bool) -> Path {
    let Some(first) = points.first() else {
        return Path::new();
    };
    let mut data = Data::new().move_to((first[0], svg_y(first[1])));
    for point in points.iter().skip(1) {
        data = data.line_to((point[0], svg_y(point[1])));
    }
    if close {
        data = data.close();
    }
    Path::new().set("d", data)
}

fn arc_path(center: Point, radius: f64, start_deg: f64, end_deg: f64) -> Path {
    let start = polar_point(center, radius, start_deg);
    let end = polar_point(center, radius, end_deg);
    let delta = (end_deg - start_deg).abs();
    let large_arc = if delta > 180.0 { 1 } else { 0 };
    let sweep = if end_deg >= start_deg { 0 } else { 1 };
    let mut data = Data::new().move_to((start[0], svg_y(start[1])));
    if delta >= 360.0 - 1e-9 {
        let midpoint = polar_point(
            center,
            radius,
            start_deg + if end_deg >= start_deg { 180.0 } else { -180.0 },
        );
        data = data
            .elliptical_arc_to((radius, radius, 0, 1, sweep, midpoint[0], svg_y(midpoint[1])))
            .elliptical_arc_to((radius, radius, 0, 1, sweep, start[0], svg_y(start[1])));
    } else {
        data = data.elliptical_arc_to((radius, radius, 0, large_arc, sweep, end[0], svg_y(end[1])));
    }
    Path::new().set("d", data)
}

fn ellipse_arc_path(
    center: Point,
    radius_x: f64,
    radius_y: f64,
    rotation_deg: f64,
    start_deg: f64,
    end_deg: f64,
) -> Path {
    let start = cad_model::ellipse_point(center, radius_x, radius_y, rotation_deg, start_deg);
    let end = cad_model::ellipse_point(center, radius_x, radius_y, rotation_deg, end_deg);
    let delta = (end_deg - start_deg).abs();
    let large_arc = if delta > 180.0 { 1 } else { 0 };
    let sweep = if end_deg >= start_deg { 0 } else { 1 };
    Path::new().set(
        "d",
        Data::new()
            .move_to((start[0], svg_y(start[1])))
            .elliptical_arc_to((
                radius_x,
                radius_y,
                normalize_zero(-rotation_deg),
                large_arc,
                sweep,
                end[0],
                svg_y(end[1]),
            )),
    )
}

fn points_attr(points: &[Point]) -> String {
    points
        .iter()
        .map(|point| format!("{},{}", point[0], svg_y(point[1])))
        .collect::<Vec<_>>()
        .join(" ")
}

fn polar_point(center: Point, radius: f64, deg: f64) -> Point {
    let rad = deg.to_radians();
    [
        normalize_zero(center[0] + radius * rad.cos()),
        normalize_zero(center[1] + radius * rad.sin()),
    ]
}

fn svg_y(y: f64) -> f64 {
    normalize_zero(-y)
}

fn normalize_zero(value: f64) -> f64 {
    if value.abs() < 0.000_000_001 {
        0.0
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{create_dir_all, write};

    #[test]
    fn exposes_crate_name() {
        assert_eq!(crate_name(), "cad-diff");
    }

    #[test]
    fn detects_added_removed_modified_and_unchanged() {
        let base = fixture_project(BaseFixture::Base);
        let head = fixture_project(BaseFixture::Head);
        let base = cad_model::load_project(base.path()).expect("base should load");
        let head = cad_model::load_project(head.path()).expect("head should load");

        let report = diff_projects(&base, &head);

        assert_change(
            &report,
            "ent_01JZ0000000000000000000000",
            ChangeKind::Modified,
        );
        assert_change(
            &report,
            "ent_01JZ0000000000000000000001",
            ChangeKind::Modified,
        );
        assert_change(
            &report,
            "ent_01JZ0000000000000000000002",
            ChangeKind::Removed,
        );
        assert_change(&report, "ent_01JZ0000000000000000000003", ChangeKind::Added);
    }

    #[test]
    fn json_report_snapshot_is_stable() {
        let base = fixture_project(BaseFixture::Base);
        let head = fixture_project(BaseFixture::Head);
        let base = cad_model::load_project(base.path()).expect("base should load");
        let head = cad_model::load_project(head.path()).expect("head should load");

        let json = diff_projects_json(&base, &head).expect("json should serialize");

        insta::assert_snapshot!(json, @r#"
{
  "schema_version": "0.2",
  "status": "warning",
  "changes": [
    {
      "entity_id": "ent_01JZ0000000000000000000000",
      "drawing": "plan_1f",
      "kind": "modified",
      "reasons": [
        "geometry_changed",
        "style_changed",
        "line_width_changed"
      ]
    },
    {
      "entity_id": "ent_01JZ0000000000000000000001",
      "drawing": "plan_1f",
      "kind": "modified",
      "reasons": [
        "style_changed"
      ]
    },
    {
      "entity_id": "ent_01JZ0000000000000000000002",
      "drawing": "plan_1f",
      "kind": "removed",
      "reasons": []
    },
    {
      "entity_id": "ent_01JZ0000000000000000000003",
      "drawing": "plan_1f",
      "kind": "added",
      "reasons": []
    },
    {
      "entity_id": "ent_01JZ0000000000000000000004",
      "drawing": "plan_1f",
      "kind": "added",
      "reasons": []
    }
  ],
  "warnings": [
    {
      "kind": "line_width_changed",
      "entity_ids": [
        "ent_01JZ0000000000000000000000"
      ],
      "drawing": "plan_1f",
      "message": "line width changed"
    },
    {
      "kind": "outside_paper",
      "entity_ids": [
        "ent_01JZ0000000000000000000003"
      ],
      "drawing": "plan_1f",
      "message": "entity is outside paper bounds"
    },
    {
      "kind": "text_overlap",
      "entity_ids": [
        "ent_01JZ0000000000000000000001",
        "ent_01JZ0000000000000000000004"
      ],
      "drawing": "plan_1f",
      "message": "text bounding boxes overlap"
    }
  ],
  "configuration_changes": [
    {
      "path": "layers.layers.0-1.line_width",
      "kind": "modified",
      "before": 0.25,
      "after": 0.35
    }
  ]
}
"#);
    }

    #[test]
    fn style_and_text_change_snapshot_is_stable() {
        let base = text_change_project(TextFixture::Base);
        let head = text_change_project(TextFixture::Head);
        let base = cad_model::load_project(base.path()).expect("base should load");
        let head = cad_model::load_project(head.path()).expect("head should load");

        let json = diff_projects_json(&base, &head).expect("json should serialize");

        insta::assert_snapshot!(json, @r#"
{
  "schema_version": "0.2",
  "status": "ok",
  "changes": [
    {
      "entity_id": "ent_01JZ0000000000000000000000",
      "drawing": "plan_1f",
      "kind": "modified",
      "reasons": [
        "style_changed",
        "text_changed"
      ]
    }
  ],
  "warnings": [],
  "configuration_changes": []
}
"#);
    }

    #[test]
    fn svg_diff_contains_overlay_classes() {
        let base = fixture_project(BaseFixture::Base);
        let head = fixture_project(BaseFixture::Head);
        let base = cad_model::load_project(base.path()).expect("base should load");
        let head = cad_model::load_project(head.path()).expect("head should load");

        let svg = diff_projects_svg(&base, &head);

        assert!(svg.contains("class=\"modified\""));
        assert!(svg.contains("class=\"removed\""));
        assert!(svg.contains("class=\"added\""));
        assert!(svg.contains("#00AA00"));
        assert!(svg.contains("#DD2222"));
        assert!(svg.contains("#D6B300"));
        assert!(svg.contains("data-bbox="));
    }

    #[test]
    fn svg_diff_uses_text_styles_for_overlay_bbox() {
        let base = styled_bbox_project(false);
        let head = styled_bbox_project(true);
        let base = cad_model::load_project(base.path()).expect("base should load");
        let head = cad_model::load_project(head.path()).expect("head should load");

        let svg = diff_projects_svg(&base, &head);

        assert!(svg.contains("data-bbox=\"10,20,95,50\""));
        assert!(svg.contains("data-bbox=\"-15,0,115,50\""));
        assert!(svg.contains("font-size=\"30\""));
        assert!(svg.contains("textLength=\"85\""));
        assert!(!svg.contains("data-bbox=\"10,20,310,270\""));
    }

    #[test]
    fn ellipse_signature_and_svg_path_include_all_geometry() {
        let entity: Entity = serde_json::from_str(
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"ellipse","layer":"0-1","center":[1.0,2.0],"radius_x":8.0,"radius_y":3.0,"rotation_deg":45.0,"start_deg":0.0,"end_deg":180.0}"#,
        )
        .expect("ellipse should parse");

        let signature = entity_geometry_signature(&entity);
        let path = ellipse_arc_path([1.0, 2.0], 8.0, 3.0, 45.0, 0.0, 180.0).to_string();

        assert_eq!(signature, "ellipse:[1.0, 2.0]:8:3:45:0:180");
        assert!(path.contains("A8,3,-45,0,0"));
    }

    #[test]
    fn mirrored_text_and_dimension_metadata_affect_diff_geometry() {
        let text: Entity = serde_json::from_str(
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"text","layer":"0-1","style":"note","at":[10.0,20.0],"rotation_deg":30.0,"mirror_y":true,"value":"mirror"}"#,
        )
        .expect("text should parse");
        let dimension: Entity = serde_json::from_str(
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000001","type":"dimension","layer":"0-1","style":"dim_100","p1":[0.0,0.0],"p2":[0.0,10.0],"offset":2.0,"text_rotation_deg":90.0,"text_mirror_y":true,"value":"10"}"#,
        )
        .expect("dimension should parse");
        let style = TextStyleDef {
            font_family: "Hiragino Sans".to_owned(),
            height: 2.5,
            width: 1.25,
            spacing: 0.0,
            align: TextAlign::Left,
        };

        let node = text_node(
            &[10.0, 20.0],
            30.0,
            true,
            "mirror",
            &style,
            "start",
            "#000000",
        )
        .to_string();

        assert!(entity_geometry_signature(&text).ends_with(":true"));
        assert!(entity_geometry_signature(&dimension).ends_with(":90:true"));
        assert!(node.contains("transform=\"matrix("));
    }

    fn assert_change(report: &DiffReport, entity_id: &str, kind: ChangeKind) {
        let change = report
            .changes
            .iter()
            .find(|change| change.entity_id == entity_id)
            .expect("change should exist");
        assert_eq!(change.kind, kind);
    }

    enum BaseFixture {
        Base,
        Head,
    }

    enum TextFixture {
        Base,
        Head,
    }

    fn fixture_project(kind: BaseFixture) -> tempfile::TempDir {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        create_dir_all(temp.path().join("rules")).expect("rules dir should be created");
        create_dir_all(temp.path().join("drawings/plan_1f"))
            .expect("drawing dir should be created");

        write(
            temp.path().join("cad.project.toml"),
            "schema_version = \"0.1\"\nname = \"fixture\"\n",
        )
        .expect("project TOML should be writable");
        let line_width = match kind {
            BaseFixture::Base => 0.25,
            BaseFixture::Head => 0.35,
        };
        write(
            temp.path().join("rules/layers.toml"),
            format!(
                "[layers.\"0-1\"]\nname = \"A-WALL\"\nvisible = true\nprintable = true\ncolor = \"jw_black\"\nline_type = \"solid\"\nline_width = {line_width}\n"
            ),
        )
        .expect("layers TOML should be writable");
        write(
            temp.path().join("rules/styles.toml"),
            "[colors.jw_black]\nrgb = \"#000000\"\nprint_width = 0.25\n\n[line_types.solid]\ndash = []\n\n[text_styles.note]\nfont_family = \"Hiragino Sans\"\nheight = 250\nwidth = 125\nspacing = 0\nalign = \"left\"\n\n[dimension_styles.dim_100]\ntext_style = \"note\"\narrow_size = 120\nextension_gap = 40\nprecision = 0\nunit = \"mm\"\n",
        )
        .expect("styles TOML should be writable");
        write(
            temp.path().join("drawings/plan_1f/sheet.toml"),
            "schema_version = \"0.1\"\npaper = \"A3\"\norientation = \"landscape\"\nscale = \"1/100\"\norigin = [0.0, 0.0]\n",
        )
        .expect("sheet TOML should be writable");
        let entities = match kind {
            BaseFixture::Base => vec![
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0.0,0.0],"p2":[910.0,0.0]}"#,
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000001","type":"text","layer":"0-1","style":"note","at":[100.0,200.0],"rotation_deg":0.0,"value":"same"}"#,
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000002","type":"text","layer":"0-1","style":"note","at":[400.0,200.0],"rotation_deg":0.0,"value":"removed"}"#,
            ],
            BaseFixture::Head => vec![
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0.0,0.0],"p2":[1200.0,0.0]}"#,
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000001","type":"text","layer":"0-1","style":"note","at":[100.0,200.0],"rotation_deg":0.0,"value":"same"}"#,
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000003","type":"line","layer":"0-1","p1":[50000.0,0.0],"p2":[51000.0,0.0]}"#,
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000004","type":"text","layer":"0-1","style":"note","at":[150.0,220.0],"rotation_deg":0.0,"value":"overlap"}"#,
            ],
        };
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            entities.join("\n"),
        )
        .expect("entities NDJSON should be writable");

        temp
    }

    fn text_change_project(kind: TextFixture) -> tempfile::TempDir {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        create_dir_all(temp.path().join("rules")).expect("rules dir should be created");
        create_dir_all(temp.path().join("drawings/plan_1f"))
            .expect("drawing dir should be created");

        write(
            temp.path().join("cad.project.toml"),
            "schema_version = \"0.1\"\nname = \"text-change-fixture\"\n",
        )
        .expect("project TOML should be writable");
        write(
            temp.path().join("rules/layers.toml"),
            "[layers.\"0-1\"]\nname = \"A-NOTE\"\nvisible = true\nprintable = true\ncolor = \"jw_black\"\nline_type = \"solid\"\nline_width = 0.25\n",
        )
        .expect("layers TOML should be writable");
        write(
            temp.path().join("rules/styles.toml"),
            "[colors.jw_black]\nrgb = \"#000000\"\nprint_width = 0.25\n\n[line_types.solid]\ndash = []\n\n[text_styles.note]\nfont_family = \"Hiragino Sans\"\nheight = 250\nwidth = 125\nspacing = 0\nalign = \"left\"\n\n[text_styles.note_big]\nfont_family = \"Hiragino Sans\"\nheight = 300\nwidth = 150\nspacing = 0\nalign = \"left\"\n\n[dimension_styles.dim_100]\ntext_style = \"note\"\narrow_size = 120\nextension_gap = 40\nprecision = 0\nunit = \"mm\"\n",
        )
        .expect("styles TOML should be writable");
        write(
            temp.path().join("drawings/plan_1f/sheet.toml"),
            "schema_version = \"0.1\"\npaper = \"A3\"\norientation = \"landscape\"\nscale = \"1/100\"\norigin = [0.0, 0.0]\n",
        )
        .expect("sheet TOML should be writable");

        let entity = match kind {
            TextFixture::Base => {
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"text","layer":"0-1","style":"note","at":[100.0,200.0],"rotation_deg":0.0,"value":"before"}"#
            }
            TextFixture::Head => {
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"text","layer":"0-1","style":"note_big","at":[100.0,200.0],"rotation_deg":0.0,"value":"after"}"#
            }
        };
        write(temp.path().join("drawings/plan_1f/entities.ndjson"), entity)
            .expect("entities NDJSON should be writable");

        temp
    }

    fn styled_bbox_project(include_entities: bool) -> tempfile::TempDir {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        create_dir_all(temp.path().join("rules")).expect("rules dir should be created");
        create_dir_all(temp.path().join("drawings/plan_1f"))
            .expect("drawing dir should be created");

        write(
            temp.path().join("cad.project.toml"),
            "schema_version = \"0.1\"\nname = \"styled-bbox-fixture\"\n",
        )
        .expect("project TOML should be writable");
        write(
            temp.path().join("rules/layers.toml"),
            "[layers.\"0-1\"]\nname = \"A-NOTE\"\nvisible = true\nprintable = true\ncolor = \"jw_black\"\nline_type = \"solid\"\nline_width = 0.25\n",
        )
        .expect("layers TOML should be writable");
        write(
            temp.path().join("rules/styles.toml"),
            "[colors.jw_black]\nrgb = \"#000000\"\nprint_width = 0.25\n\n[line_types.solid]\ndash = []\n\n[text_styles.wide]\nfont_family = \"Hiragino Sans\"\nheight = 30\nwidth = 40\nspacing = 5\nalign = \"left\"\n\n[dimension_styles.dim_wide]\ntext_style = \"wide\"\narrow_size = 12\nextension_gap = 4\nprecision = 0\nunit = \"mm\"\n",
        )
        .expect("styles TOML should be writable");
        write(
            temp.path().join("drawings/plan_1f/sheet.toml"),
            "schema_version = \"0.1\"\npaper = \"A3\"\norientation = \"landscape\"\nscale = \"1/100\"\norigin = [0.0, 0.0]\n",
        )
        .expect("sheet TOML should be writable");
        let entities = if include_entities {
            [
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"text","layer":"0-1","style":"wide","at":[10.0,20.0],"rotation_deg":0.0,"value":"AB"}"#,
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000001","type":"dimension","layer":"0-1","style":"dim_wide","p1":[0.0,0.0],"p2":[100.0,0.0],"offset":20.0,"value":"100"}"#,
            ]
            .join("\n")
        } else {
            String::new()
        };
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            entities,
        )
        .expect("entities NDJSON should be writable");

        temp
    }
}
