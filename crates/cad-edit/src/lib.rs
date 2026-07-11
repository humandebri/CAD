//! Atomic editing and snapping for CAD NDJSON drawings.

use cad_check::Severity;
use cad_model::{Entity, EntityRecord, Point, ProjectSource};
use geo::{
    Coord, Line,
    line_intersection::{LineIntersection, line_intersection},
};
use rstar::{AABB, RTree, RTreeObject};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use thiserror::Error;
use ulid::Ulid;

pub const CRATE_NAME: &str = "cad-edit";

#[must_use]
pub fn crate_name() -> &'static str {
    CRATE_NAME
}

#[derive(Debug, Error)]
pub enum EditError {
    #[error("drawing {0:?} was not found")]
    DrawingNotFound(String),
    #[error("entity {0:?} was not found")]
    EntityNotFound(String),
    #[error("revision_conflict: drawing changed since the edit session started")]
    RevisionConflict,
    #[error("layer {0:?} is missing, hidden, or locked")]
    LayerNotEditable(String),
    #[error("invalid entity: {0}")]
    InvalidEntity(String),
    #[error("edit introduces checker errors: {0}")]
    CheckFailed(String),
    #[error("failed to read {path}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write {path}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

pub type EditResult<T> = Result<T, EditError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EditOperation {
    Create {
        entity: Value,
    },
    Replace {
        entity_id: String,
        entity: Value,
    },
    Translate {
        entity_id: String,
        delta: Point,
        duplicate: bool,
    },
    Delete {
        entity_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrawingEditRequest {
    pub drawing: String,
    pub expected_revision: String,
    pub operation: EditOperation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DrawingEditResult {
    pub drawing: String,
    pub revision: String,
    pub entity_id: Option<String>,
    pub operation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorDrawingState {
    pub drawing: String,
    pub revision: String,
    pub entities: Vec<Value>,
    pub text_styles: Vec<String>,
    pub dimension_styles: Vec<String>,
    pub pens: Vec<String>,
}

pub fn editor_state(project: &ProjectSource, drawing: &str) -> EditResult<EditorDrawingState> {
    let drawing_source = project
        .drawings
        .iter()
        .find(|candidate| candidate.name == drawing)
        .ok_or_else(|| EditError::DrawingNotFound(drawing.to_owned()))?;
    let path = entities_path(&project.root, drawing);
    let bytes = fs::read(&path).map_err(|source| EditError::Read {
        path: path.clone(),
        source,
    })?;
    Ok(EditorDrawingState {
        drawing: drawing.to_owned(),
        revision: revision(&bytes),
        entities: drawing_source
            .entities
            .iter()
            .map(|record| serde_json::to_value(&record.entity).expect("Entity is serializable"))
            .collect(),
        text_styles: project.styles.text_styles.keys().cloned().collect(),
        dimension_styles: project.styles.dimension_styles.keys().cloned().collect(),
        pens: project.styles.pens.keys().cloned().collect(),
    })
}

pub fn apply_edit(
    project_path: &Path,
    request: &DrawingEditRequest,
) -> EditResult<DrawingEditResult> {
    let _guard = edit_lock()
        .lock()
        .map_err(|_| EditError::InvalidEntity("drawing edit lock is poisoned".to_owned()))?;
    let mut project = cad_model::load_project(project_path)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    let drawing_index = project
        .drawings
        .iter()
        .position(|drawing| drawing.name == request.drawing)
        .ok_or_else(|| EditError::DrawingNotFound(request.drawing.clone()))?;
    let path = entities_path(project_path, &request.drawing);
    let bytes = fs::read(&path).map_err(|source| EditError::Read {
        path: path.clone(),
        source,
    })?;
    if revision(&bytes) != request.expected_revision {
        return Err(EditError::RevisionConflict);
    }
    let text = String::from_utf8(bytes.clone())
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    let mut raw_lines = split_raw_lines(&text);
    let mut entities = project.drawings[drawing_index]
        .entities
        .iter()
        .map(|record| record.entity.clone())
        .collect::<Vec<_>>();
    let baseline_errors = checker_errors(&project);

    let (entity_id, operation) = match &request.operation {
        EditOperation::Create { entity } => {
            let entity = create_entity(entity.clone())?;
            ensure_layer_editable(&project, entity.layer())?;
            let id = entity.id().as_str().to_owned();
            append_raw_line(&mut raw_lines, entity_json(&entity)?);
            entities.push(entity);
            (Some(id), "create")
        }
        EditOperation::Replace { entity_id, entity } => {
            let index = entity_index(&entities, entity_id)?;
            ensure_layer_editable(&project, entities[index].layer())?;
            let replacement: Entity = serde_json::from_value(entity.clone())
                .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
            if replacement.id().as_str() != entity_id {
                return Err(EditError::InvalidEntity(
                    "replacement must preserve entity id".to_owned(),
                ));
            }
            if entity_kind(&replacement) != entity_kind(&entities[index]) {
                return Err(EditError::InvalidEntity(
                    "replacement must preserve entity type".to_owned(),
                ));
            }
            ensure_layer_editable(&project, replacement.layer())?;
            raw_lines[index].content = entity_json(&replacement)?;
            entities[index] = replacement;
            (Some(entity_id.clone()), "replace")
        }
        EditOperation::Translate {
            entity_id,
            delta,
            duplicate,
        } => {
            if !delta.iter().all(|value| value.is_finite()) {
                return Err(EditError::InvalidEntity(
                    "translation is not finite".to_owned(),
                ));
            }
            let index = entity_index(&entities, entity_id)?;
            ensure_layer_editable(&project, entities[index].layer())?;
            let mut translated = translate_entity(&entities[index], *delta)?;
            let id = if *duplicate {
                let id = next_id();
                set_entity_id(&mut translated, &id)?;
                insert_raw_line_after(&mut raw_lines, index, entity_json(&translated)?);
                entities.insert(index + 1, translated);
                id
            } else {
                raw_lines[index].content = entity_json(&translated)?;
                entities[index] = translated;
                entity_id.clone()
            };
            (Some(id), if *duplicate { "duplicate" } else { "translate" })
        }
        EditOperation::Delete { entity_id } => {
            let index = entity_index(&entities, entity_id)?;
            ensure_layer_editable(&project, entities[index].layer())?;
            remove_raw_line(&mut raw_lines, index);
            entities.remove(index);
            (None, "delete")
        }
    };

    project.drawings[drawing_index].entities = entities
        .into_iter()
        .enumerate()
        .map(|(index, entity)| EntityRecord {
            line: index + 1,
            entity,
        })
        .collect();
    let candidate_errors = checker_errors(&project);
    let new_errors = candidate_errors
        .difference(&baseline_errors)
        .cloned()
        .collect::<Vec<_>>();
    if !new_errors.is_empty() {
        return Err(EditError::CheckFailed(new_errors.join("; ")));
    }

    let output = render_raw_lines(&raw_lines);
    atomic_replace(&path, output.as_bytes(), &request.expected_revision)?;
    Ok(DrawingEditResult {
        drawing: request.drawing.clone(),
        revision: revision(output.as_bytes()),
        entity_id,
        operation: operation.to_owned(),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawLine {
    content: String,
    ending: String,
}

fn split_raw_lines(text: &str) -> Vec<RawLine> {
    text.split_inclusive('\n')
        .map(|line| {
            if let Some(content) = line.strip_suffix("\r\n") {
                RawLine {
                    content: content.to_owned(),
                    ending: "\r\n".to_owned(),
                }
            } else if let Some(content) = line.strip_suffix('\n') {
                RawLine {
                    content: content.to_owned(),
                    ending: "\n".to_owned(),
                }
            } else {
                RawLine {
                    content: line.to_owned(),
                    ending: String::new(),
                }
            }
        })
        .collect()
}

fn preferred_ending(lines: &[RawLine], index: usize) -> String {
    lines
        .get(index)
        .filter(|line| !line.ending.is_empty())
        .or_else(|| {
            lines[..index.min(lines.len())]
                .iter()
                .rev()
                .find(|line| !line.ending.is_empty())
        })
        .or_else(|| {
            lines
                .get(index + 1..)
                .and_then(|rest| rest.iter().find(|line| !line.ending.is_empty()))
        })
        .map_or_else(|| "\n".to_owned(), |line| line.ending.clone())
}

fn append_raw_line(lines: &mut Vec<RawLine>, content: String) {
    if lines.is_empty() {
        lines.push(RawLine {
            content,
            ending: "\n".to_owned(),
        });
        return;
    }
    let last = lines.len() - 1;
    let ending = preferred_ending(lines, last);
    let had_trailing_ending = !lines[last].ending.is_empty();
    if !had_trailing_ending {
        lines[last].ending = ending.clone();
    }
    lines.push(RawLine {
        content,
        ending: if had_trailing_ending {
            ending
        } else {
            String::new()
        },
    });
}

fn insert_raw_line_after(lines: &mut Vec<RawLine>, index: usize, content: String) {
    let ending = preferred_ending(lines, index);
    let original_ending = std::mem::replace(&mut lines[index].ending, ending.clone());
    lines.insert(
        index + 1,
        RawLine {
            content,
            ending: original_ending,
        },
    );
}

fn remove_raw_line(lines: &mut Vec<RawLine>, index: usize) {
    let removed = lines.remove(index);
    if removed.ending.is_empty() && index > 0 {
        lines[index - 1].ending.clear();
    }
}

fn render_raw_lines(lines: &[RawLine]) -> String {
    let mut output = String::new();
    for line in lines {
        output.push_str(&line.content);
        output.push_str(&line.ending);
    }
    output
}

fn checker_errors(project: &ProjectSource) -> BTreeSet<String> {
    cad_check::check_loaded_project(project)
        .diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .map(|diagnostic| {
            format!(
                "{}:{:?}:{:?}:{}",
                diagnostic.code, diagnostic.entity_id, diagnostic.field, diagnostic.message
            )
        })
        .collect()
}

fn ensure_layer_editable(project: &ProjectSource, layer_id: &str) -> EditResult<()> {
    let layer = project
        .layers
        .layers
        .get(layer_id)
        .ok_or_else(|| EditError::LayerNotEditable(layer_id.to_owned()))?;
    let group_locked_or_hidden = layer
        .group
        .as_ref()
        .and_then(|group| project.layers.groups.get(group))
        .is_some_and(|group| group.locked || !group.visible);
    if !layer.visible || layer.locked || group_locked_or_hidden {
        return Err(EditError::LayerNotEditable(layer_id.to_owned()));
    }
    Ok(())
}

fn create_entity(mut value: Value) -> EditResult<Entity> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| EditError::InvalidEntity("entity must be an object".to_owned()))?;
    object.insert("schema_version".to_owned(), Value::String("0.1".to_owned()));
    object.insert("id".to_owned(), Value::String(next_id()));
    serde_json::from_value(value).map_err(|error| EditError::InvalidEntity(error.to_string()))
}

fn set_entity_id(entity: &mut Entity, id: &str) -> EditResult<()> {
    let mut value = serde_json::to_value(&*entity)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    value["id"] = Value::String(id.to_owned());
    *entity = serde_json::from_value(value)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    Ok(())
}

fn entity_index(entities: &[Entity], id: &str) -> EditResult<usize> {
    entities
        .iter()
        .position(|entity| entity.id().as_str() == id)
        .ok_or_else(|| EditError::EntityNotFound(id.to_owned()))
}

fn entity_json(entity: &Entity) -> EditResult<String> {
    serde_json::to_string(entity).map_err(|error| EditError::InvalidEntity(error.to_string()))
}

fn entity_kind(entity: &Entity) -> &'static str {
    match entity {
        Entity::Line { .. } => "line",
        Entity::Polyline { .. } => "polyline",
        Entity::Arc { .. } => "arc",
        Entity::Circle { .. } => "circle",
        Entity::Ellipse { .. } => "ellipse",
        Entity::Text { .. } => "text",
        Entity::Dimension { .. } => "dimension",
        Entity::Point { .. } => "point",
        Entity::Solid { .. } => "solid",
        Entity::CurveSolid { .. } => "curve_solid",
        Entity::BlockRef { .. } => "block_ref",
    }
}

fn next_id() -> String {
    format!("ent_{}", Ulid::new())
}

fn revision(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn entities_path(project: &Path, drawing: &str) -> PathBuf {
    project
        .join("drawings")
        .join(drawing)
        .join("entities.ndjson")
}

fn edit_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn atomic_replace(path: &Path, bytes: &[u8], expected_revision: &str) -> EditResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| EditError::InvalidEntity("entities path has no parent".to_owned()))?;
    let permissions = fs::metadata(path)
        .map_err(|source| EditError::Read {
            path: path.to_path_buf(),
            source,
        })?
        .permissions();
    let temp = tempfile::NamedTempFile::new_in(parent).map_err(|source| EditError::Write {
        path: parent.to_path_buf(),
        source,
    })?;
    fs::write(temp.path(), bytes).map_err(|source| EditError::Write {
        path: temp.path().to_path_buf(),
        source,
    })?;
    fs::set_permissions(temp.path(), permissions).map_err(|source| EditError::Write {
        path: temp.path().to_path_buf(),
        source,
    })?;
    let current = fs::read(path).map_err(|source| EditError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    if revision(&current) != expected_revision {
        return Err(EditError::RevisionConflict);
    }
    fs::rename(temp.path(), path).map_err(|source| EditError::Write {
        path: path.to_path_buf(),
        source,
    })
}

pub fn translate_entity(entity: &Entity, delta: Point) -> EditResult<Entity> {
    let mut value = serde_json::to_value(entity)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    let kind = value["type"].as_str().unwrap_or_default().to_owned();
    let keys: &[&str] = match kind.as_str() {
        "line" | "dimension" => &["p1", "p2"],
        "arc" | "circle" | "ellipse" | "curve_solid" => &["center"],
        "text" | "point" | "block_ref" => &["at"],
        "polyline" | "solid" => &[],
        _ => {
            return Err(EditError::InvalidEntity(format!(
                "unsupported entity type {kind:?}"
            )));
        }
    };
    for key in keys {
        translate_json_point(&mut value[*key], delta)?;
    }
    if matches!(kind.as_str(), "polyline" | "solid") {
        let points = value["points"]
            .as_array_mut()
            .ok_or_else(|| EditError::InvalidEntity("points must be an array".to_owned()))?;
        for point in points {
            translate_json_point(point, delta)?;
        }
    }
    serde_json::from_value(value).map_err(|error| EditError::InvalidEntity(error.to_string()))
}

fn translate_json_point(value: &mut Value, delta: Point) -> EditResult<()> {
    let point = value
        .as_array_mut()
        .filter(|point| point.len() == 2)
        .ok_or_else(|| EditError::InvalidEntity("point must contain two coordinates".to_owned()))?;
    for axis in 0..2 {
        let coordinate = point[axis].as_f64().ok_or_else(|| {
            EditError::InvalidEntity("point coordinate is not numeric".to_owned())
        })?;
        point[axis] = Value::from(coordinate + delta[axis]);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SnapKind {
    Endpoint,
    Midpoint,
    Intersection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapCandidate {
    pub kind: SnapKind,
    pub point: Point,
    pub distance: f64,
}

#[derive(Debug, Clone, Copy)]
struct SnapSegment {
    start: Point,
    end: Point,
}

impl RTreeObject for SnapSegment {
    type Envelope = AABB<Point>;

    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners(
            [
                self.start[0].min(self.end[0]),
                self.start[1].min(self.end[1]),
            ],
            [
                self.start[0].max(self.end[0]),
                self.start[1].max(self.end[1]),
            ],
        )
    }
}

pub struct SnapIndex {
    segments: RTree<SnapSegment>,
    points: Vec<(SnapKind, Point)>,
}

impl SnapIndex {
    pub fn build(project: &ProjectSource, drawing: &str) -> EditResult<Self> {
        let drawing = project
            .drawings
            .iter()
            .find(|candidate| candidate.name == drawing)
            .ok_or_else(|| EditError::DrawingNotFound(drawing.to_owned()))?;
        let mut segments = Vec::new();
        let mut points = Vec::new();
        for record in &drawing.entities {
            if !layer_visible(project, record.entity.layer()) {
                continue;
            }
            append_segments(&record.entity, &mut segments);
            append_direct_snap_points(&record.entity, &mut points);
        }
        for segment in &segments {
            points.push((SnapKind::Endpoint, segment.start));
            points.push((SnapKind::Endpoint, segment.end));
            points.push((SnapKind::Midpoint, midpoint(segment.start, segment.end)));
        }
        let mut seen = BTreeSet::new();
        points.retain(|(kind, point)| seen.insert((*kind, point[0].to_bits(), point[1].to_bits())));
        Ok(Self {
            segments: RTree::bulk_load(segments),
            points,
        })
    }

    #[must_use]
    pub fn query(&self, point: Point, tolerance: f64, modes: &[SnapKind]) -> Option<SnapCandidate> {
        if tolerance <= 0.0 || !tolerance.is_finite() {
            return None;
        }
        let enabled = modes.iter().copied().collect::<BTreeSet<_>>();
        let mut candidates = self
            .points
            .iter()
            .filter(|(kind, _)| enabled.contains(kind))
            .filter_map(|(kind, candidate)| snap_candidate(*kind, *candidate, point, tolerance))
            .collect::<Vec<_>>();
        if enabled.contains(&SnapKind::Intersection) {
            let envelope = AABB::from_corners(
                [point[0] - tolerance, point[1] - tolerance],
                [point[0] + tolerance, point[1] + tolerance],
            );
            let local = self
                .segments
                .locate_in_envelope_intersecting(envelope)
                .copied()
                .collect::<Vec<_>>();
            for left in 0..local.len() {
                for right in left + 1..local.len() {
                    if let Some(intersection) = segment_intersection(local[left], local[right])
                        && let Some(candidate) =
                            snap_candidate(SnapKind::Intersection, intersection, point, tolerance)
                    {
                        candidates.push(candidate);
                    }
                }
            }
        }
        candidates.into_iter().min_by(|left, right| {
            left.distance
                .partial_cmp(&right.distance)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }
}

fn layer_visible(project: &ProjectSource, layer_id: &str) -> bool {
    project.layers.layers.get(layer_id).is_some_and(|layer| {
        layer.visible
            && layer
                .group
                .as_ref()
                .and_then(|group| project.layers.groups.get(group))
                .is_none_or(|group| group.visible)
    })
}

fn append_segments(entity: &Entity, output: &mut Vec<SnapSegment>) {
    match entity {
        Entity::Line { p1, p2, .. } | Entity::Dimension { p1, p2, .. } => {
            output.push(SnapSegment {
                start: *p1,
                end: *p2,
            });
        }
        Entity::Polyline { points, closed, .. } => {
            output.extend(points.windows(2).map(|pair| SnapSegment {
                start: pair[0],
                end: pair[1],
            }));
            if *closed && points.len() > 2 {
                output.push(SnapSegment {
                    start: *points.last().expect("nonempty"),
                    end: points[0],
                });
            }
        }
        Entity::Solid { points, .. } => {
            output.extend(points.windows(2).map(|pair| SnapSegment {
                start: pair[0],
                end: pair[1],
            }));
            if points.len() > 2 {
                output.push(SnapSegment {
                    start: *points.last().expect("nonempty"),
                    end: points[0],
                });
            }
        }
        _ => {}
    }
}

fn append_direct_snap_points(entity: &Entity, output: &mut Vec<(SnapKind, Point)>) {
    match entity {
        Entity::Arc {
            center,
            radius,
            start_deg,
            end_deg,
            ..
        } => {
            output.push((
                SnapKind::Endpoint,
                polar_point(*center, *radius, *start_deg),
            ));
            output.push((SnapKind::Endpoint, polar_point(*center, *radius, *end_deg)));
            output.push((
                SnapKind::Midpoint,
                polar_point(*center, *radius, (*start_deg + *end_deg) / 2.0),
            ));
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
            output.push((
                SnapKind::Endpoint,
                cad_model::ellipse_point(*center, *radius_x, *radius_y, *rotation_deg, *start_deg),
            ));
            output.push((
                SnapKind::Endpoint,
                cad_model::ellipse_point(*center, *radius_x, *radius_y, *rotation_deg, *end_deg),
            ));
            output.push((
                SnapKind::Midpoint,
                cad_model::ellipse_point(
                    *center,
                    *radius_x,
                    *radius_y,
                    *rotation_deg,
                    (*start_deg + *end_deg) / 2.0,
                ),
            ));
        }
        Entity::Circle { center, .. } | Entity::Point { at: center, .. } => {
            output.push((SnapKind::Midpoint, *center));
        }
        Entity::Text { at, .. } | Entity::BlockRef { at, .. } => {
            output.push((SnapKind::Endpoint, *at));
        }
        _ => {}
    }
}

fn polar_point(center: Point, radius: f64, degrees: f64) -> Point {
    let radians = degrees.to_radians();
    [
        center[0] + radius * radians.cos(),
        center[1] + radius * radians.sin(),
    ]
}

fn midpoint(left: Point, right: Point) -> Point {
    [(left[0] + right[0]) / 2.0, (left[1] + right[1]) / 2.0]
}

fn snap_candidate(
    kind: SnapKind,
    candidate: Point,
    point: Point,
    tolerance: f64,
) -> Option<SnapCandidate> {
    let distance = ((candidate[0] - point[0]).powi(2) + (candidate[1] - point[1]).powi(2)).sqrt();
    (distance <= tolerance).then_some(SnapCandidate {
        kind,
        point: candidate,
        distance,
    })
}

fn segment_intersection(left: SnapSegment, right: SnapSegment) -> Option<Point> {
    let line = |segment: SnapSegment| {
        Line::new(
            Coord {
                x: segment.start[0],
                y: segment.start[1],
            },
            Coord {
                x: segment.end[0],
                y: segment.end[1],
            },
        )
    };
    match line_intersection(line(left), line(right))? {
        LineIntersection::SinglePoint { intersection, .. } => {
            Some([intersection.x, intersection.y])
        }
        LineIntersection::Collinear { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn translates_every_entity_family() {
        for source in [
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0,0],"p2":[1,1]}"#,
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000001","type":"polyline","layer":"0-1","points":[[0,0],[1,1]],"closed":false}"#,
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000002","type":"circle","layer":"0-1","center":[0,0],"radius":2}"#,
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000003","type":"text","layer":"0-1","style":"note","at":[0,0],"rotation_deg":0,"value":"A"}"#,
        ] {
            let entity: Entity = serde_json::from_str(source).expect("entity");
            let moved = translate_entity(&entity, [5.0, -2.0]).expect("translate");
            let bbox = cad_model::entity_bbox(&moved).expect("bbox");
            assert!(bbox.max[0] >= 5.0);
        }
    }

    #[test]
    fn line_intersection_is_reported() {
        let left = SnapSegment {
            start: [0.0, 0.0],
            end: [10.0, 10.0],
        };
        let right = SnapSegment {
            start: [0.0, 10.0],
            end: [10.0, 0.0],
        };
        assert_eq!(segment_intersection(left, right), Some([5.0, 5.0]));
    }

    #[test]
    fn applies_atomic_translate_and_preserves_other_raw_lines() {
        let temp = test_project(false);
        let project = cad_model::load_project(temp.path()).expect("project");
        let state = editor_state(&project, "plan").expect("editor state");
        let result = apply_edit(
            temp.path(),
            &DrawingEditRequest {
                drawing: "plan".to_owned(),
                expected_revision: state.revision,
                operation: EditOperation::Translate {
                    entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                    delta: [5.0, 2.0],
                    duplicate: false,
                },
            },
        )
        .expect("translate");
        let text = fs::read_to_string(entities_path(temp.path(), "plan")).expect("entities");
        assert!(text.lines().next().expect("line").contains("[5.0,2.0]"));
        assert!(text.contains("  \"type\": \"line\""));
        assert_eq!(
            result.entity_id.as_deref(),
            Some("ent_01JZ0000000000000000000000")
        );
    }

    #[test]
    fn preserves_crlf_unrelated_lines_and_trailing_newline_policy_for_all_edits() {
        let temp = test_project(false);
        let path = entities_path(temp.path(), "plan");
        let original = fs::read_to_string(&path).expect("entities");
        let crlf_without_trailing = original.trim_end_matches('\n').replace('\n', "\r\n");
        fs::write(&path, &crlf_without_trailing).expect("CRLF entities");
        let formatted_line = crlf_without_trailing
            .split("\r\n")
            .nth(1)
            .expect("formatted line")
            .to_owned();

        let mut replacement = editor_state(
            &cad_model::load_project(temp.path()).expect("project"),
            "plan",
        )
        .expect("state")
        .entities[0]
            .clone();
        replacement["p2"] = serde_json::json!([12.0, 0.0]);
        apply_test_edit(
            temp.path(),
            EditOperation::Replace {
                entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                entity: replacement,
            },
        );
        assert!(
            fs::read_to_string(&path)
                .expect("replace output")
                .contains(&formatted_line)
        );

        let created = apply_test_edit(
            temp.path(),
            EditOperation::Create {
                entity: serde_json::json!({
                    "type": "point",
                    "layer": "0-1",
                    "at": [2.0, 3.0]
                }),
            },
        )
        .entity_id
        .expect("created id");
        apply_test_edit(
            temp.path(),
            EditOperation::Translate {
                entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                delta: [1.0, 0.0],
                duplicate: true,
            },
        );
        apply_test_edit(temp.path(), EditOperation::Delete { entity_id: created });

        let output = fs::read_to_string(&path).expect("final output");
        assert!(!output.ends_with('\n'));
        assert!(output.contains(&formatted_line));
        assert!(
            output
                .as_bytes()
                .iter()
                .enumerate()
                .filter(|(_, byte)| **byte == b'\n')
                .all(|(index, _)| index > 0 && output.as_bytes()[index - 1] == b'\r')
        );
        assert_eq!(output.matches("\r\n").count(), output.lines().count() - 1);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_edit_preserves_unix_permission_bits() {
        use std::os::unix::fs::PermissionsExt;

        let temp = test_project(false);
        let path = entities_path(temp.path(), "plan");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).expect("set mode");

        apply_test_edit(
            temp.path(),
            EditOperation::Translate {
                entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                delta: [1.0, 0.0],
                duplicate: false,
            },
        );

        assert_eq!(
            fs::metadata(path).expect("metadata").permissions().mode() & 0o777,
            0o640
        );
    }

    #[test]
    fn rejects_stale_revision_and_locked_layer() {
        let temp = test_project(false);
        let request = DrawingEditRequest {
            drawing: "plan".to_owned(),
            expected_revision: "stale".to_owned(),
            operation: EditOperation::Delete {
                entity_id: "ent_01JZ0000000000000000000000".to_owned(),
            },
        };
        assert!(matches!(
            apply_edit(temp.path(), &request),
            Err(EditError::RevisionConflict)
        ));

        let locked = test_project(true);
        let project = cad_model::load_project(locked.path()).expect("project");
        let state = editor_state(&project, "plan").expect("state");
        let mut request = request;
        request.expected_revision = state.revision;
        assert!(matches!(
            apply_edit(locked.path(), &request),
            Err(EditError::LayerNotEditable(_))
        ));
    }

    #[test]
    fn concurrent_edits_from_one_revision_allow_only_one_publish() {
        use std::sync::{Arc, Barrier};

        let temp = test_project(false);
        let root = Arc::new(temp.path().to_path_buf());
        let project = cad_model::load_project(root.as_ref()).expect("project");
        let revision = editor_state(&project, "plan").expect("state").revision;
        let barrier = Arc::new(Barrier::new(3));
        let handles = [false, true].map(|duplicate| {
            let root = Arc::clone(&root);
            let revision = revision.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                apply_edit(
                    root.as_ref(),
                    &DrawingEditRequest {
                        drawing: "plan".to_owned(),
                        expected_revision: revision,
                        operation: EditOperation::Translate {
                            entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                            delta: [if duplicate { 2.0 } else { 1.0 }, 0.0],
                            duplicate,
                        },
                    },
                )
            })
        });
        barrier.wait();
        let results = handles.map(|handle| handle.join().expect("edit thread"));

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(EditError::RevisionConflict)))
                .count(),
            1
        );
    }

    #[test]
    fn creates_entity_with_backend_ulid() {
        let temp = test_project(false);
        let project = cad_model::load_project(temp.path()).expect("project");
        let state = editor_state(&project, "plan").expect("state");
        let result = apply_edit(
            temp.path(),
            &DrawingEditRequest {
                drawing: "plan".to_owned(),
                expected_revision: state.revision,
                operation: EditOperation::Create {
                    entity: serde_json::json!({
                        "type": "point",
                        "layer": "0-1",
                        "at": [2.0, 3.0]
                    }),
                },
            },
        )
        .expect("create");
        assert!(
            result
                .entity_id
                .as_deref()
                .is_some_and(|id| id.starts_with("ent_"))
        );
        assert_eq!(
            cad_model::load_project(temp.path())
                .expect("reloaded")
                .drawings[0]
                .entities
                .len(),
            3
        );
    }

    #[test]
    fn snap_index_finds_endpoint_midpoint_and_intersection() {
        let temp = test_project(false);
        let project = cad_model::load_project(temp.path()).expect("project");
        let index = SnapIndex::build(&project, "plan").expect("index");
        assert_eq!(
            index
                .query([0.1, 0.1], 1.0, &[SnapKind::Endpoint])
                .map(|value| value.kind),
            Some(SnapKind::Endpoint)
        );
        assert_eq!(
            index
                .query([5.0, 0.1], 1.0, &[SnapKind::Midpoint])
                .map(|value| value.kind),
            Some(SnapKind::Midpoint)
        );
        assert_eq!(
            index
                .query([5.0, 0.0], 1.0, &[SnapKind::Intersection])
                .map(|value| value.kind),
            Some(SnapKind::Intersection)
        );
    }

    fn apply_test_edit(project_path: &Path, operation: EditOperation) -> DrawingEditResult {
        let project = cad_model::load_project(project_path).expect("project");
        let state = editor_state(&project, "plan").expect("editor state");
        apply_edit(
            project_path,
            &DrawingEditRequest {
                drawing: "plan".to_owned(),
                expected_revision: state.revision,
                operation,
            },
        )
        .expect("edit")
    }

    fn test_project(locked: bool) -> tempfile::TempDir {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("rules")).expect("rules");
        fs::create_dir_all(temp.path().join("drawings/plan")).expect("drawing");
        fs::write(
            temp.path().join("cad.project.toml"),
            "schema_version = \"0.1\"\nname = \"edit-test\"\n",
        )
        .expect("project");
        fs::write(
            temp.path().join("rules/layers.toml"),
            format!("active_layer = \"0-1\"\n\n[layers.\"0-1\"]\nname = \"Edit\"\nvisible = true\nlocked = {locked}\nprintable = true\ncolor = \"black\"\nline_type = \"solid\"\nline_width = 0.25\n"),
        )
        .expect("layers");
        fs::write(
            temp.path().join("rules/styles.toml"),
            "[colors.black]\nrgb = \"#000000\"\nprint_width = 0.25\n\n[line_types.solid]\ndash = []\n\n[pens]\n\n[text_styles]\n\n[dimension_styles]\n",
        )
        .expect("styles");
        fs::write(
            temp.path().join("drawings/plan/sheet.toml"),
            "schema_version = \"0.1\"\npaper = \"A3\"\norientation = \"landscape\"\nscale = \"1/1\"\norigin = [0.0, 0.0]\n",
        )
        .expect("sheet");
        fs::write(
            entities_path(temp.path(), "plan"),
            concat!(
                "{\"schema_version\":\"0.1\",\"id\":\"ent_01JZ0000000000000000000000\",\"type\":\"line\",\"layer\":\"0-1\",\"p1\":[0.0,0.0],\"p2\":[10.0,0.0]}\n",
                "{ \"schema_version\": \"0.1\", \"id\": \"ent_01JZ0000000000000000000001\",  \"type\": \"line\", \"layer\": \"0-1\", \"p1\": [5.0,-5.0], \"p2\": [5.0,5.0] }\n"
            ),
        )
        .expect("entities");
        temp
    }
}
