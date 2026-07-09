//! Semantic CAD diff crate.
//!
//! This crate compares CAD source by stable entity IDs and emits JSON/SVG
//! review artifacts. It does not mutate either project.

use cad_model::{BBox, Entity, EntityRecord, Point, ProjectSource, entity_bbox};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use svg::Document;
use svg::node::element::path::Data;
use svg::node::element::{Circle, Group, Line, Path, Polyline, Text};

pub const CRATE_NAME: &str = "cad-diff";
pub const DIFF_SCHEMA_VERSION: &str = "0.1";

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
pub struct DiffReport {
    pub schema_version: String,
    pub status: DiffStatus,
    pub changes: Vec<DiffChange>,
    pub warnings: Vec<DiffWarning>,
}

impl DiffReport {
    #[must_use]
    pub fn new(changes: Vec<DiffChange>, warnings: Vec<DiffWarning>) -> Self {
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
            warnings.extend(text_overlap_warnings(&drawing_name, records));
        }
    }

    DiffReport::new(changes, warnings)
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
                    view_bbox = merge_view_bbox(view_bbox, visual_bbox(record));
                    root = root.add(render_overlay_entity(record, "#00AA00", 0.95, "added"));
                }
            }
            ChangeKind::Removed => {
                if let Some(record) = base_drawings
                    .get(&change.drawing)
                    .and_then(|records| find_record(records, &change.entity_id))
                {
                    view_bbox = merge_view_bbox(view_bbox, visual_bbox(record));
                    root = root.add(render_overlay_entity(record, "#DD2222", 0.95, "removed"));
                }
            }
            ChangeKind::Modified => {
                if let Some(record) = base_drawings
                    .get(&change.drawing)
                    .and_then(|records| find_record(records, &change.entity_id))
                {
                    view_bbox = merge_view_bbox(view_bbox, visual_bbox(record));
                    root = root.add(render_overlay_entity(
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
                    view_bbox = merge_view_bbox(view_bbox, visual_bbox(record));
                    root = root.add(render_overlay_entity(record, "#D6B300", 0.95, "modified"));
                }
            }
            ChangeKind::Unchanged => {
                if let Some(record) = head_drawings
                    .get(&change.drawing)
                    .and_then(|records| find_record(records, &change.entity_id))
                {
                    view_bbox = merge_view_bbox(view_bbox, visual_bbox(record));
                    root = root.add(render_overlay_entity(record, "#999999", 0.25, "unchanged"));
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
    if entity_text_signature(base) != entity_text_signature(head) {
        reasons.insert(ChangeReason::TextChanged);
    }
    if line_width(base_project, base) != line_width(head_project, head) {
        reasons.insert(ChangeReason::LineWidthChanged);
    }

    reasons.into_iter().collect()
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
        Entity::Text {
            at, rotation_deg, ..
        } => format!("text:{at:?}:{rotation_deg}"),
        Entity::Dimension { p1, p2, offset, .. } => format!("dimension:{p1:?}:{p2:?}:{offset}"),
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
    match entity {
        Entity::Text { style, .. } | Entity::Dimension { style, .. } => style.clone(),
        _ => String::new(),
    }
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
        .layers
        .layers
        .get(entity.layer())
        .map(|layer| cad_model::format_decimal_mm(layer.line_width))
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

fn text_overlap_warnings(drawing_name: &str, records: &[EntityRecord]) -> Vec<DiffWarning> {
    let texts = records
        .iter()
        .filter_map(|record| text_bbox(record).map(|bbox| (record, bbox)))
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

fn text_bbox(record: &EntityRecord) -> Option<BBox> {
    match &record.entity {
        Entity::Text { at, value, .. } => {
            let width = value.chars().count() as f64 * 150.0;
            Some(BBox {
                min: *at,
                max: [at[0] + width, at[1] + 250.0],
            })
        }
        _ => None,
    }
}

fn visual_bbox(record: &EntityRecord) -> Option<BBox> {
    match &record.entity {
        Entity::Text { .. } => text_bbox(record).or_else(|| entity_bbox(&record.entity)),
        _ => entity_bbox(&record.entity),
    }
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
    record: &EntityRecord,
    color: &str,
    opacity: f64,
    class_name: &str,
) -> Group {
    let mut group = Group::new()
        .set("class", class_name)
        .set("data-entity-id", record.entity.id().as_str())
        .set("data-layer", record.entity.layer())
        .set("opacity", opacity)
        .set("stroke", color)
        .set("fill", "none")
        .set("stroke-width", "0.35mm");

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
        Entity::Text { at, value, .. } => {
            group = group.add(
                Text::new("")
                    .set("x", at[0])
                    .set("y", svg_y(at[1]))
                    .set("fill", color)
                    .set("stroke", "none")
                    .set("font-size", 250)
                    .add(svg::node::Text::new(value.clone())),
            );
        }
        Entity::Dimension {
            p1,
            p2,
            offset,
            value,
            ..
        } => {
            let d1 = [p1[0], p1[1] + offset];
            let d2 = [p2[0], p2[1] + offset];
            let label = value.clone().unwrap_or_else(|| {
                cad_model::format_decimal_mm(
                    ((p2[0] - p1[0]).powi(2) + (p2[1] - p1[1]).powi(2)).sqrt(),
                )
            });
            group = group
                .add(
                    Line::new()
                        .set("x1", d1[0])
                        .set("y1", svg_y(d1[1]))
                        .set("x2", d2[0])
                        .set("y2", svg_y(d2[1])),
                )
                .add(
                    Text::new("")
                        .set("x", (d1[0] + d2[0]) / 2.0)
                        .set("y", svg_y((d1[1] + d2[1]) / 2.0))
                        .set("fill", color)
                        .set("stroke", "none")
                        .set("font-size", 250)
                        .add(svg::node::Text::new(label)),
                );
        }
        Entity::BlockRef {
            block, at, scale, ..
        } => {
            let size = 100.0 * scale;
            group = group
                .add(
                    Line::new()
                        .set("x1", at[0] - size)
                        .set("y1", svg_y(at[1]))
                        .set("x2", at[0] + size)
                        .set("y2", svg_y(at[1])),
                )
                .add(
                    Line::new()
                        .set("x1", at[0])
                        .set("y1", svg_y(at[1] - size))
                        .set("x2", at[0])
                        .set("y2", svg_y(at[1] + size)),
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
    Path::new().set(
        "d",
        Data::new()
            .move_to((start[0], svg_y(start[1])))
            .elliptical_arc_to((radius, radius, 0, large_arc, sweep, end[0], svg_y(end[1]))),
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
            ChangeKind::Unchanged,
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
  "schema_version": "0.1",
  "status": "warning",
  "changes": [
    {
      "entity_id": "ent_01JZ0000000000000000000000",
      "drawing": "plan_1f",
      "kind": "modified",
      "reasons": [
        "geometry_changed",
        "line_width_changed"
      ]
    },
    {
      "entity_id": "ent_01JZ0000000000000000000001",
      "drawing": "plan_1f",
      "kind": "unchanged",
      "reasons": []
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
  "schema_version": "0.1",
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
  "warnings": []
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
            "[colors.jw_black]\nrgb = \"#000000\"\nprint_width = 0.25\n\n[line_types.solid]\ndash = []\n\n[text_styles.note]\nfont_family = \"Hiragino Sans\"\nheight = 250\nalign = \"left\"\n\n[dimension_styles.dim_100]\ntext_style = \"note\"\narrow_size = 120\nextension_gap = 40\nprecision = 0\nunit = \"mm\"\n",
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
            "[colors.jw_black]\nrgb = \"#000000\"\nprint_width = 0.25\n\n[line_types.solid]\ndash = []\n\n[text_styles.note]\nfont_family = \"Hiragino Sans\"\nheight = 250\nalign = \"left\"\n\n[text_styles.note_big]\nfont_family = \"Hiragino Sans\"\nheight = 300\nalign = \"left\"\n\n[dimension_styles.dim_100]\ntext_style = \"note\"\narrow_size = 120\nextension_gap = 40\nprecision = 0\nunit = \"mm\"\n",
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
}
