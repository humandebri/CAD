//! Direct-edit candidates: ordinary NDJSON, stable role IDs and three-way regeneration.
use crate::{
    Result, ToolkitError,
    drafting::{GeneratorRequest, Geometry},
};
use cad_edit::EditOperation;
use cad_model::{Entity, EntityId, EntityRecord, Point, ProjectSource, SourceFileRevision};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DraftRequest {
    pub key: String,
    pub layer: String,
    #[serde(default)]
    pub pen: Option<String>,
    pub geometry: DraftGeometry,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum DraftGeometry {
    CoordinateCopy {
        source_drawing: String,
        entity_ids: Vec<String>,
        coordinate_mm_per_unit: f64,
        base_point: Point,
        at_mm: Point,
    },
    Repeat {
        geometry: Geometry,
        copies: usize,
        step_mm: Point,
    },
    WallElevations {
        dimension_style: String,
        text_style: String,
        dimension_offset_mm: f64,
        walls: Vec<Wall>,
    },
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Wall {
    pub key: String,
    /// Left and right as seen from inside the room; swapping them reverses projection.
    pub left: Point,
    pub right: Point,
    /// Required even for mm input; never inferred from the drawing's paper scale.
    pub coordinate_mm_per_unit: f64,
    pub ceiling_height_mm: f64,
    pub at_mm: Point,
    #[serde(default)]
    pub openings: Vec<Opening>,
    /// Include explicit assumptions in the drawing, e.g. "CH2400 assumed".
    #[serde(default)]
    pub note: String,
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Opening {
    pub key: String,
    pub first: Point,
    pub second: Point,
    pub sill_height_mm: f64,
    pub height_mm: f64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DraftReport {
    pub schema_version: String,
    pub project: String,
    pub namespace: String,
    pub drawing: String,
    pub key: String,
    pub request: Value,
    pub source_files: Vec<SourceFileRevision>,
    /// Baseline generated values, never used as the editable source of truth.
    pub generated: BTreeMap<String, Entity>,
    /// Historical ID role names retained when migrating ordinal window roles.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub id_roles: BTreeMap<String, String>,
    pub warnings: Vec<String>,
    pub cad_check: cad_check::CheckReport,
}

#[derive(Debug)]
pub struct Candidate {
    pub ndjson: String,
    pub report: DraftReport,
}

fn invalid(message: impl Into<String>) -> ToolkitError {
    ToolkitError::Invalid(message.into())
}
fn point(p: Point) -> Result<()> {
    if p.iter().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(invalid("coordinates must be finite"))
    }
}
fn positive(n: f64, label: &str) -> Result<()> {
    if n.is_finite() && n > 0.001 {
        Ok(())
    } else {
        Err(invalid(format!("{label} must be finite and positive")))
    }
}
fn key(k: &str) -> Result<()> {
    if !k.is_empty()
        && k.len() <= 128
        && k.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        Ok(())
    } else {
        Err(invalid(
            "keys must contain 1..128 ASCII letters, digits, _ or -",
        ))
    }
}
fn id(project: &str, drawing: &str, request: &str, role: &str) -> EntityId {
    let hash = blake3::hash(
        serde_json::to_string(&(project, drawing, request, role))
            .unwrap()
            .as_bytes(),
    );
    let bytes: [u8; 16] = hash.as_bytes()[..16].try_into().unwrap();
    EntityId::parse(&format!(
        "ent_{}",
        ulid::Ulid::from(u128::from_be_bytes(bytes))
    ))
    .unwrap()
}

fn repeat_entity_role(geometry: &Geometry, index: usize) -> Result<String> {
    let Geometry::Window { panels, .. } = geometry else {
        return Ok(format!("entity_{index}"));
    };
    Ok(match index {
        0 => "rail_negative".into(),
        1 => "rail_center".into(),
        2 => "rail_positive".into(),
        3 => "jamb_left".into(),
        _ if *panels > 0 && index - 3 == *panels => "jamb_right".into(),
        _ if index - 3 < *panels => {
            let mut a = index - 3;
            let mut b = *panels;
            while b != 0 {
                (a, b) = (b, a % b);
            }
            format!("mullion_{}_{}", (index - 3) / a, panels / a)
        }
        _ => return Err(invalid("invalid window entity role")),
    })
}

fn previous_baseline(
    report: &DraftReport,
) -> Result<(BTreeMap<String, Entity>, BTreeMap<String, String>)> {
    let request: DraftRequest = serde_json::from_value(report.request.clone())?;
    let mut baseline = BTreeMap::new();
    let mut id_roles = BTreeMap::new();
    for (role, entity) in &report.generated {
        let id_role = report.id_roles.get(role).unwrap_or(role);
        if entity.id() != &id(&report.namespace, &report.drawing, &report.key, id_role) {
            return Err(invalid("previous report contains an invalid role ID"));
        }
        let mut normalized = role.clone();
        if let DraftGeometry::Repeat {
            geometry: geometry @ Geometry::Window { .. },
            ..
        } = &request.geometry
            && let Some((copy, suffix)) = role.split_once('/')
            && copy
                .strip_prefix("copy_")
                .is_some_and(|s| s.parse::<usize>().is_ok())
            && let Some(index) = suffix.strip_prefix("entity_")
        {
            normalized = format!(
                "{copy}/{}",
                repeat_entity_role(
                    geometry,
                    index
                        .parse()
                        .map_err(|_| invalid("invalid previous window role"))?
                )?
            );
        }
        if baseline
            .insert(normalized.clone(), entity.clone())
            .is_some()
        {
            return Err(invalid("previous report has duplicate roles"));
        }
        if id_role != &normalized {
            id_roles.insert(normalized, id_role.clone());
        }
    }
    Ok((baseline, id_roles))
}

fn generated(
    project: &ProjectSource,
    drawing: &str,
    request: &DraftRequest,
) -> Result<(BTreeMap<String, Value>, Vec<String>)> {
    key(&request.key)?;
    let mut values = BTreeMap::new();
    let mut warnings = Vec::new();
    let mut add = |role: String, mut v: Value| -> Result<()> {
        v["schema_version"] = json!(cad_model::CURRENT_SCHEMA_VERSION);
        v["layer"] = json!(request.layer);
        if let Some(pen) = &request.pen {
            v["pen"] = json!(pen);
        }
        if values.insert(role.clone(), v).is_some() {
            return Err(invalid(format!("duplicate role {role}")));
        }
        Ok(())
    };
    match &request.geometry {
        DraftGeometry::CoordinateCopy {
            source_drawing,
            entity_ids,
            coordinate_mm_per_unit,
            base_point,
            at_mm,
        } => {
            if !coordinate_mm_per_unit.is_finite() || *coordinate_mm_per_unit <= 0.0 {
                return Err(invalid("coordinate_mm_per_unit must be positive"));
            }
            point(*base_point)?;
            point(*at_mm)?;
            let source = project
                .drawings
                .iter()
                .find(|d| &d.name == source_drawing)
                .ok_or_else(|| invalid("missing source drawing"))?;
            let ids: BTreeSet<_> = entity_ids.iter().collect();
            if ids.len() != entity_ids.len() || ids.is_empty() || ids.len() > 100_000 {
                return Err(invalid(
                    "entity_ids must contain 1..100000 unique source IDs",
                ));
            }
            let mut groups = BTreeSet::new();
            for id in ids {
                let original = source
                    .entities
                    .iter()
                    .find(|r| r.entity.id().as_str() == id)
                    .ok_or_else(|| invalid(format!("missing source ID {id}")))?;
                groups.insert(
                    project
                        .layers
                        .layers
                        .get(original.entity.layer())
                        .and_then(|l| l.group.clone()),
                );
                let v = convert_geometry(
                    &original.entity,
                    *coordinate_mm_per_unit,
                    *base_point,
                    *at_mm,
                )?;
                add(format!("source/{id}"), v)?;
            }
            if groups.len() > 1 {
                return Err(invalid(
                    "coordinate copy requires one layer group per request; use separate explicit factors for mixed-scale groups",
                ));
            }
            warnings.push(format!("coordinate copy uses explicit factor {coordinate_mm_per_unit}; confirm a known length; original drawing is unchanged"));
        }
        DraftGeometry::Repeat {
            geometry,
            copies,
            step_mm,
        } => {
            if *copies == 0 || *copies > 1000 {
                return Err(invalid("copies must be 1..1000"));
            }
            point(*step_mm)?;
            if matches!(
                geometry,
                Geometry::MassingProjection { .. }
                    | Geometry::SunShadow { .. }
                    | Geometry::SkyView { .. }
            ) {
                return Err(invalid(
                    "repeat does not accept analysis generators; use generate with an analysis report",
                ));
            }
            let generator = GeneratorRequest {
                layer: request.layer.clone(),
                pen: request.pen.clone(),
                geometry: serde_json::from_value(serde_json::to_value(geometry)?)?,
            };
            let edit = crate::drafting::generate(project, drawing, &generator)?;
            warnings.extend(edit.warnings);
            let EditOperation::Batch { operations } = edit.operation else {
                return Err(invalid("expected generated batch"));
            };
            if operations.len().saturating_mul(*copies) > 100_000 {
                return Err(invalid("generated entity limit exceeded"));
            }
            // Generators which modify existing entities are not direct-source constructors.
            for copy in 0..*copies {
                for (index, operation) in operations.iter().enumerate() {
                    let EditOperation::Create { entity } = operation else {
                        return Err(invalid(
                            "repeat requires a generator that only creates geometry",
                        ));
                    };
                    let mut v = entity.clone();
                    let delta = [step_mm[0] * copy as f64, step_mm[1] * copy as f64];
                    for field in ["p1", "p2", "center", "at"] {
                        if let Some(p) = v.get_mut(field) {
                            translate(p, delta)?;
                        }
                    }
                    if let Some(points) = v.get_mut("points").and_then(Value::as_array_mut) {
                        for p in points {
                            translate(p, delta)?;
                        }
                    }
                    add(
                        format!("copy_{copy}/{}", repeat_entity_role(geometry, index)?),
                        v,
                    )?;
                }
            }
        }
        DraftGeometry::WallElevations {
            dimension_style,
            text_style,
            dimension_offset_mm,
            walls,
        } => {
            positive(*dimension_offset_mm, "dimension_offset_mm")?;
            if walls.is_empty() || walls.len() > 100 {
                return Err(invalid("walls must contain 1..100 walls"));
            }
            for wall in walls {
                key(&wall.key)?;
                point(wall.left)?;
                point(wall.right)?;
                point(wall.at_mm)?;
                if !wall.coordinate_mm_per_unit.is_finite() || wall.coordinate_mm_per_unit <= 0.0 {
                    return Err(invalid("coordinate_mm_per_unit must be positive"));
                }
                positive(wall.ceiling_height_mm, "ceiling_height_mm")?;
                if wall.openings.len() > 1000 || wall.note.len() > 4096 {
                    return Err(invalid("wall input limit exceeded"));
                }
                let dx = wall.right[0] - wall.left[0];
                let dy = wall.right[1] - wall.left[1];
                let raw_length = dx.hypot(dy);
                if !raw_length.is_finite() || raw_length <= 0.0 {
                    return Err(invalid("wall length must be finite and positive"));
                }
                let width = raw_length * wall.coordinate_mm_per_unit;
                positive(width, "converted wall length")?;
                let u = [dx / raw_length, dy / raw_length];
                let at = |x: f64, y: f64| [wall.at_mm[0] + x, wall.at_mm[1] + y];
                let rectangle = |x1, y1, x2, y2| json!({"type":"polyline","closed":true,"points":[at(x1,y1),at(x2,y1),at(x2,y2),at(x1,y2),at(x1,y1)]});
                add(
                    format!("{}/outline", wall.key),
                    rectangle(0., 0., width, wall.ceiling_height_mm),
                )?;
                let mut spans = Vec::new();
                for opening in &wall.openings {
                    key(&opening.key)?;
                    point(opening.first)?;
                    point(opening.second)?;
                    positive(opening.height_mm, "opening height")?;
                    if !opening.sill_height_mm.is_finite()
                        || opening.sill_height_mm < 0.
                        || opening.sill_height_mm + opening.height_mm > wall.ceiling_height_mm
                    {
                        return Err(invalid("opening must lie between floor and ceiling"));
                    }
                    let project_point = |p: Point| -> Result<f64> {
                        let v = [p[0] - wall.left[0], p[1] - wall.left[1]];
                        let perpendicular =
                            (v[0] * u[1] - v[1] * u[0]).abs() * wall.coordinate_mm_per_unit;
                        if perpendicular > 1.0 {
                            return Err(invalid("opening points must lie on the wall within 1mm"));
                        }
                        Ok((v[0] * u[0] + v[1] * u[1]) * wall.coordinate_mm_per_unit)
                    };
                    let first = project_point(opening.first)?;
                    let second = project_point(opening.second)?;
                    let (x1, x2) = (first.min(second), first.max(second));
                    positive(x2 - x1, "opening width")?;
                    if x1 < -0.001 || x2 > width + 0.001 {
                        return Err(invalid("opening lies outside wall"));
                    }
                    if spans.iter().any(|&(a, b, c, d)| {
                        x1 < b
                            && x2 > a
                            && opening.sill_height_mm < d
                            && opening.sill_height_mm + opening.height_mm > c
                    }) {
                        return Err(invalid("openings overlap"));
                    }
                    spans.push((
                        x1,
                        x2,
                        opening.sill_height_mm,
                        opening.sill_height_mm + opening.height_mm,
                    ));
                    add(
                        format!("{}/opening_{}", wall.key, opening.key),
                        rectangle(
                            x1,
                            opening.sill_height_mm,
                            x2,
                            opening.sill_height_mm + opening.height_mm,
                        ),
                    )?;
                }
                let outline_id = id(
                    &project.project.name,
                    drawing,
                    &request.key,
                    &format!("{}/outline", wall.key),
                );
                let dimension = |p1, p2, kind: &str, second_index: usize| json!({"type":"dimension","style":dimension_style,"p1":p1,"p2":p2,"offset":-dimension_offset_mm,"value":null,"measurement":{"kind":kind,"first":{"kind":"entity","entity_id":outline_id,"feature":"vertex","index":0},"second":{"kind":"entity","entity_id":outline_id,"feature":"vertex","index":second_index}}});
                add(
                    format!("{}/width", wall.key),
                    dimension(at(0., 0.), at(width, 0.), "horizontal", 1),
                )?;
                add(
                    format!("{}/height", wall.key),
                    dimension(at(0., 0.), at(0., wall.ceiling_height_mm), "vertical", 3),
                )?;
                add(
                    format!("{}/label", wall.key),
                    json!({"type":"text","style":text_style,"at":at(width/2.,wall.ceiling_height_mm+dimension_offset_mm),"rotation_deg":0,"value":wall.key}),
                )?;
                if !wall.note.is_empty() {
                    add(
                        format!("{}/note", wall.key),
                        json!({"type":"text","style":text_style,"at":at(width/2.,-dimension_offset_mm*2.),"rotation_deg":0,"value":wall.note}),
                    )?;
                }
            }
        }
    }
    Ok((values, warnings))
}
fn convert_geometry(entity: &Entity, factor: f64, base: Point, at: Point) -> Result<Value> {
    if matches!(
        entity,
        Entity::Text { .. } | Entity::Dimension { .. } | Entity::BlockRef { .. }
    ) {
        return Err(invalid(
            "coordinate copy supports primitive geometry; recreate annotations with mm styles and copy blocks using copy-entities",
        ));
    }
    let mut v = serde_json::to_value(entity)?;
    let convert = |p: &mut Value| -> Result<()> {
        let p0: Point = serde_json::from_value(p.clone())?;
        *p = json!([
            (p0[0] - base[0]) * factor + at[0],
            (p0[1] - base[1]) * factor + at[1]
        ]);
        Ok(())
    };
    for field in ["p1", "p2", "center", "at"] {
        if let Some(p) = v.get_mut(field) {
            convert(p)?;
        }
    }
    if let Some(points) = v.get_mut("points").and_then(Value::as_array_mut) {
        for p in points {
            convert(p)?;
        }
    }
    if let Some(loops) = v.get_mut("loops").and_then(Value::as_array_mut) {
        for points in loops {
            for p in points
                .as_array_mut()
                .ok_or_else(|| invalid("invalid loop"))?
            {
                convert(p)?;
            }
        }
    }
    for field in ["radius", "radius_x", "radius_y", "solid_param"] {
        if let Some(n) = v.get_mut(field) {
            *n = json!(n.as_f64().ok_or_else(|| invalid("invalid size"))? * factor);
        }
    }
    if matches!(entity, Entity::Hatch { .. } | Entity::Point { .. }) {
        v["scale"] = json!(
            v["scale"]
                .as_f64()
                .ok_or_else(|| invalid("invalid scale"))?
                * factor
        );
    }
    Ok(v)
}

fn translate(v: &mut Value, delta: Point) -> Result<()> {
    let p: Point = serde_json::from_value(v.clone())?;
    point(delta)?;
    *v = json!([p[0] + delta[0], p[1] + delta[1]]);
    Ok(())
}

/// Three-way field merge: retain manual edits, apply independent regenerated changes,
/// reject conflicting changes and deletion of a manually edited generated entity.
fn merge(base: &Value, current: &Value, next: &Value, path: &str) -> Result<Value> {
    if current == base {
        return Ok(next.clone());
    }
    if next == base || current == next {
        return Ok(current.clone());
    }
    if let (Some(b), Some(c), Some(n)) = (base.as_object(), current.as_object(), next.as_object()) {
        let keys: BTreeSet<_> = b.keys().chain(c.keys()).chain(n.keys()).collect();
        let mut result = serde_json::Map::new();
        for key in keys {
            let empty = Value::Null;
            let value = merge(
                b.get(key).unwrap_or(&empty),
                c.get(key).unwrap_or(&empty),
                n.get(key).unwrap_or(&empty),
                &format!("{path}.{key}"),
            )?;
            if !value.is_null() || n.contains_key(key) || c.contains_key(key) {
                result.insert(key.clone(), value);
            }
        }
        return Ok(Value::Object(result));
    }
    Err(invalid(format!(
        "manual edit conflicts with regeneration at {path}; reconcile the source or request first"
    )))
}

pub fn draft(
    project: &ProjectSource,
    drawing: &str,
    request: &DraftRequest,
    previous: Option<&DraftReport>,
) -> Result<Candidate> {
    let root = project
        .root
        .canonicalize()
        .map_err(|e| invalid(e.to_string()))?
        .to_string_lossy()
        .to_string();
    if cad_model::load_jww_preservation_manifest(&project.root)
        .map_err(|e| invalid(e.to_string()))?
        .is_some_and(|p| p.edit_capability == cad_model::JwwEditCapability::ExactOnly)
    {
        return Err(invalid(
            "exact-original-only imports cannot be edited; retain the original JWW",
        ));
    }
    let source = project
        .drawings
        .iter()
        .find(|d| d.name == drawing)
        .ok_or_else(|| {
            invalid("target drawing does not exist; create its canonical layout first")
        })?;
    if let Some(p) = previous
        && (p.schema_version != "cad-source-draft/1"
            || p.namespace != project.project.name
            || p.drawing != drawing
            || p.key != request.key)
    {
        return Err(invalid(
            "previous report belongs to another project, drawing or request key",
        ));
    }
    let (baseline, previous_id_roles) = previous
        .map(previous_baseline)
        .transpose()?
        .unwrap_or_default();
    let previous_values = previous.map(|_| &baseline);
    let mut next = BTreeMap::new();
    let mut id_roles = BTreeMap::new();
    let (values, mut warnings) = generated(project, drawing, request)?;
    for (role, mut value) in values {
        let id_role = previous_id_roles.get(&role).unwrap_or(&role);
        value["id"] = json!(id(&project.project.name, drawing, &request.key, id_role));
        if id_role != &role {
            id_roles.insert(role.clone(), id_role.clone());
        }
        let entity: Entity = serde_json::from_value(value)?;
        next.insert(role, entity);
    }
    let mut owned = BTreeMap::new();
    if let Some(values) = previous_values {
        for (role, entity) in values {
            if owned.insert(entity.id().clone(), role.clone()).is_some() {
                return Err(invalid("previous report has duplicate IDs"));
            }
        }
    }
    let mut candidate = Vec::new();
    let mut seen = BTreeSet::new();
    for record in &source.entities {
        if let Some(role) = owned.get(record.entity.id()) {
            let base = &previous_values.unwrap()[role];
            if let Some(new) = next.get(role) {
                let result = merge(
                    &serde_json::to_value(base)?,
                    &serde_json::to_value(&record.entity)?,
                    &serde_json::to_value(new)?,
                    role,
                )?;
                if &record.entity != base {
                    warnings.push(format!("manual edits retained: {role}"));
                }
                candidate.push(serde_json::from_value::<Entity>(result)?);
                seen.insert(role.clone());
            } else if &record.entity != base {
                return Err(invalid(format!(
                    "cannot delete manually edited role {role}"
                )));
            }
        } else {
            candidate.push(record.entity.clone());
        }
    }
    // A deleted generated role is a deliberate manual change; do not resurrect it.
    for (role, new) in &next {
        if seen.contains(role) {
            continue;
        }
        if previous_values.is_some_and(|p| p.contains_key(role)) {
            warnings.push(format!("manual deletion retained: {role}"));
            continue;
        }
        candidate.push(new.clone());
    }
    let mut checked = project.clone();
    checked
        .drawings
        .iter_mut()
        .find(|d| d.name == drawing)
        .unwrap()
        .entities = candidate
        .iter()
        .enumerate()
        .map(|(i, e)| EntityRecord {
            line: i + 1,
            entity: e.clone(),
        })
        .collect();
    let report = cad_check::check_loaded_project(&checked);
    if !report.is_ok() {
        return Err(invalid(serde_json::to_string(&report)?));
    }
    // Preserve untouched source lines byte-for-byte, including manual formatting.
    let raw = std::fs::read_to_string(
        project
            .root
            .join(format!("drawings/{drawing}/entities.ndjson")),
    )
    .map_err(|e| invalid(e.to_string()))?;
    let lines: BTreeMap<_, _> = source
        .entities
        .iter()
        .zip(raw.split_inclusive('\n'))
        .map(|(r, line)| (r.entity.id().clone(), (&r.entity, line)))
        .collect();
    let newline = if raw.contains("\r\n") { "\r\n" } else { "\n" };
    let mut ndjson = String::new();
    for entity in candidate {
        if !ndjson.is_empty() && !ndjson.ends_with('\n') {
            ndjson.push_str(newline);
        }
        if let Some((_, line)) = lines.get(entity.id()).filter(|(old, _)| *old == &entity) {
            ndjson.push_str(line);
        } else {
            ndjson.push_str(&serde_json::to_string(&entity)?);
            ndjson.push_str(newline);
        }
    }
    Ok(Candidate {
        ndjson,
        report: DraftReport {
            schema_version: "cad-source-draft/1".into(),
            project: root,
            namespace: project.project.name.clone(),
            drawing: drawing.into(),
            key: request.key.clone(),
            request: serde_json::to_value(request)?,
            source_files: cad_model::source_manifest(&project.root)
                .map_err(|e| invalid(e.to_string()))?,
            generated: next,
            id_roles,
            warnings,
            cad_check: report,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path};
    fn source() -> (tempfile::TempDir, ProjectSource) {
        fn copy(from: &Path, to: &Path) {
            fs::create_dir_all(to).unwrap();
            for entry in fs::read_dir(from).unwrap() {
                let entry = entry.unwrap();
                let dest = to.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy(&entry.path(), &dest);
                } else {
                    fs::copy(entry.path(), dest).unwrap();
                }
            }
        }
        let temp = tempfile::tempdir().unwrap();
        copy(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/cad-acceptance"),
            temp.path(),
        );
        let source = cad_model::load_project(temp.path()).unwrap();
        (temp, source)
    }
    fn walls() -> DraftRequest {
        serde_json::from_value(json!({"key":"room","layer":"0-1","geometry":{"type":"wall_elevations","dimension_style":"dim","text_style":"note","dimension_offset_mm":300,"walls":[{"key":"A","left":[0,0],"right":[37,0],"coordinate_mm_per_unit":100,"ceiling_height_mm":2400,"at_mm":[0,6000],"note":"CH2400 assumed","openings":[{"key":"window","first":[5,0],"second":[15,0],"sill_height_mm":900,"height_mm":1200}]}]}})).unwrap()
    }
    fn install(source: &mut ProjectSource, candidate: &Candidate) {
        let file = source.root.join("drawings/acceptance/entities.ndjson");
        fs::write(file, &candidate.ndjson).unwrap();
        assert!(cad_check::check_project(&source.root).is_ok());
        *source = cad_model::load_project(&source.root).unwrap();
    }
    fn save_manual_edits(source: &mut ProjectSource) {
        fs::write(
            source.root.join("drawings/acceptance/entities.ndjson"),
            source.drawings[0]
                .entities
                .iter()
                .map(|r| format!("{}\n", serde_json::to_string(&r.entity).unwrap()))
                .collect::<String>(),
        )
        .unwrap();
        assert!(cad_check::check_project(&source.root).is_ok());
        *source = cad_model::load_project(&source.root).unwrap();
    }
    fn window_request(panels: usize) -> DraftRequest {
        serde_json::from_value(json!({"key":"windows","layer":"0-1","geometry":{"type":"repeat","copies":2,"step_mm":[2000,0],"geometry":{"type":"window","p1":[0,6500],"p2":[1000,6500],"depth":100,"panels":panels}}})).unwrap()
    }
    fn legacy_window_candidate(source: &ProjectSource, request: &DraftRequest) -> Candidate {
        let mut candidate = draft(source, "acceptance", request, None).unwrap();
        let DraftGeometry::Repeat {
            geometry: Geometry::Window { panels, .. },
            copies,
            ..
        } = &request.geometry
        else {
            panic!()
        };
        let DraftGeometry::Repeat { geometry, .. } = &request.geometry else {
            panic!()
        };
        let mut mapping = BTreeMap::new();
        let mut generated = BTreeMap::new();
        for copy in 0..*copies {
            for index in 0..panels + 4 {
                let role = format!(
                    "copy_{copy}/{}",
                    repeat_entity_role(geometry, index).unwrap()
                );
                let old_role = format!("copy_{copy}/entity_{index}");
                let entity = &candidate.report.generated[&role];
                let old_id = id(&source.project.name, "acceptance", &request.key, &old_role);
                mapping.insert(entity.id().clone(), old_id.clone());
                let mut value = serde_json::to_value(entity).unwrap();
                value["id"] = json!(old_id);
                generated.insert(old_role, serde_json::from_value::<Entity>(value).unwrap());
            }
        }
        candidate.report.generated = generated;
        candidate.report.id_roles.clear();
        candidate.ndjson = candidate
            .ndjson
            .lines()
            .map(|line| {
                let mut value: Value = serde_json::from_str(line).unwrap();
                let entity_id: EntityId = serde_json::from_value(value["id"].clone()).unwrap();
                if let Some(replacement) = mapping.get(&entity_id) {
                    value["id"] = json!(replacement);
                }
                format!("{value}\n")
            })
            .collect();
        candidate
    }
    #[test]
    fn window_regeneration_preserves_jamb_edits_and_references_with_legacy_reports() {
        for legacy in [false, true] {
            for panels in [1, 2] {
                let (_temp, mut source) = source();
                let first_request = window_request(panels);
                let first = if legacy {
                    legacy_window_candidate(&source, &first_request)
                } else {
                    draft(&source, "acceptance", &first_request, None).unwrap()
                };
                let right_role = if legacy {
                    format!("copy_0/entity_{}", panels + 3)
                } else {
                    "copy_0/jamb_right".into()
                };
                let left_role = if legacy {
                    "copy_0/entity_3"
                } else {
                    "copy_0/jamb_left"
                };
                let right_id = first.report.generated[&right_role].id().clone();
                let left_id = first.report.generated[left_role].id().clone();
                install(&mut source, &first);
                let right = source.drawings[0]
                    .entities
                    .iter_mut()
                    .find(|r| r.entity.id() == &right_id)
                    .unwrap();
                let mut value = serde_json::to_value(&right.entity).unwrap();
                value["pen"] = json!("dashed");
                right.entity = serde_json::from_value(value).unwrap();
                let dimension_id = EntityId::parse(&format!("ent_{}", ulid::Ulid::new())).unwrap();
                source.drawings[0].entities.push(EntityRecord {
                    line: 0,
                    entity: serde_json::from_value(json!({"schema_version":"0.3","id":dimension_id,"type":"dimension","layer":"0-2","style":"dim","p1":[0,6450],"p2":[1000,6450],"offset":300,"measurement":{"kind":"horizontal","first":{"kind":"entity","entity_id":left_id,"feature":"start"},"second":{"kind":"entity","entity_id":right_id,"feature":"start"}}})).unwrap(),
                });
                save_manual_edits(&mut source);
                // Round-trip each report as the CLI does, including legacy missing id_roles.
                let mut previous: DraftReport =
                    serde_json::from_value(serde_json::to_value(&first.report).unwrap()).unwrap();
                for next_panels in [panels * 2, panels * 4] {
                    let regenerated = draft(
                        &source,
                        "acceptance",
                        &window_request(next_panels),
                        Some(&previous),
                    )
                    .unwrap();
                    assert_eq!(
                        regenerated.report.generated["copy_0/jamb_right"].id(),
                        &right_id
                    );
                    assert_eq!(
                        regenerated.report.generated["copy_0/jamb_left"].id(),
                        &left_id
                    );
                    install(&mut source, &regenerated);
                    let right = &source.drawings[0]
                        .entities
                        .iter()
                        .find(|r| r.entity.id() == &right_id)
                        .unwrap()
                        .entity;
                    assert_eq!(right.pen(), Some("dashed"));
                    let Entity::Line { p1, p2, .. } = right else {
                        panic!()
                    };
                    assert_eq!(*p1, [1000., 6450.]);
                    assert_eq!(*p2, [1000., 6550.]);
                    let dimension = &source.drawings[0]
                        .entities
                        .iter()
                        .find(|r| r.entity.id() == &dimension_id)
                        .unwrap()
                        .entity;
                    assert_eq!(
                        cad_model::evaluate_dimension(&source, dimension)
                            .unwrap()
                            .measured,
                        1000.
                    );
                    assert_eq!(
                        source.drawings[0]
                            .entities
                            .iter()
                            .filter(|r| r.entity.pen() == Some("dashed")
                                && r.entity.id()
                                    != &EntityId::parse("ent_01JZ0000000000000000000105").unwrap())
                            .count(),
                        1
                    );
                    previous =
                        serde_json::from_value(serde_json::to_value(&regenerated.report).unwrap())
                            .unwrap();
                }
            }
        }
    }
    #[test]
    fn window_midpoint_edits_survive_refinement_and_conflict_when_the_mullion_is_removed() {
        let (_temp, mut source) = source();
        let first = draft(&source, "acceptance", &window_request(2), None).unwrap();
        let mid_id = first.report.generated["copy_0/mullion_1_2"].id().clone();
        install(&mut source, &first);
        let mid = source.drawings[0]
            .entities
            .iter_mut()
            .find(|r| r.entity.id() == &mid_id)
            .unwrap();
        let mut value = serde_json::to_value(&mid.entity).unwrap();
        value["pen"] = json!("dashed");
        mid.entity = serde_json::from_value(value).unwrap();
        save_manual_edits(&mut source);
        let refined = draft(
            &source,
            "acceptance",
            &window_request(4),
            Some(&first.report),
        )
        .unwrap();
        install(&mut source, &refined);
        let mid = &source.drawings[0]
            .entities
            .iter()
            .find(|r| r.entity.id() == &mid_id)
            .unwrap()
            .entity;
        assert_eq!(mid.pen(), Some("dashed"));
        let Entity::Line { p1, .. } = mid else {
            panic!()
        };
        assert_eq!(p1[0], 500.);
        assert!(
            draft(
                &source,
                "acceptance",
                &window_request(3),
                Some(&refined.report)
            )
            .unwrap_err()
            .to_string()
            .contains("cannot delete manually edited role")
        );
    }
    #[test]
    fn projection_converts_units_reverses_opposite_walls_and_keeps_source_bytes() {
        let (_temp, source) = source();
        let mut request = walls();
        let before =
            fs::read_to_string(source.root.join("drawings/acceptance/entities.ndjson")).unwrap();
        let candidate = draft(&source, "acceptance", &request, None).unwrap();
        assert!(candidate.ndjson.starts_with(&before));
        let Entity::Polyline { points, .. } = &candidate.report.generated["A/opening_window"]
        else {
            panic!()
        };
        assert_eq!(points[0], [500., 6900.]);
        assert_eq!(points[2], [1500., 8100.]);
        let DraftGeometry::WallElevations { walls, .. } = &mut request.geometry else {
            panic!()
        };
        walls[0].left = [37., 0.];
        walls[0].right = [0., 0.];
        let reverse = draft(&source, "acceptance", &request, None).unwrap();
        let Entity::Polyline { points, .. } = &reverse.report.generated["A/opening_window"] else {
            panic!()
        };
        assert_eq!(points[0], [2200., 6900.]);
        assert_eq!(points[2], [3200., 8100.]);
        assert_eq!(
            candidate.report.generated["A/outline"].id(),
            reverse.report.generated["A/outline"].id()
        );
        assert_eq!(
            fs::read_to_string(source.root.join("drawings/acceptance/entities.ndjson")).unwrap(),
            before
        );
    }
    #[test]
    fn regeneration_merges_manual_fields_rejects_conflicts_and_preserves_deletions() {
        let (_temp, mut source) = source();
        let mut request = walls();
        let first = draft(&source, "acceptance", &request, None).unwrap();
        install(&mut source, &first);
        let label_id = first.report.generated["A/label"].id();
        let deleted_id = first.report.generated["A/opening_window"].id();
        let file = source.root.join("drawings/acceptance/entities.ndjson");
        let mut records: Vec<Value> = fs::read_to_string(&file)
            .unwrap()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        for entity in &mut records {
            if entity["id"] == json!(label_id) {
                entity["value"] = json!("手修正した壁名");
            }
        }
        records.retain(|e| e["id"] != json!(deleted_id));
        fs::write(
            &file,
            records
                .iter()
                .map(|e| format!("{}\n", e))
                .collect::<String>(),
        )
        .unwrap();
        assert!(cad_check::check_project(&source.root).is_ok());
        source = cad_model::load_project(&source.root).unwrap();
        let DraftGeometry::WallElevations { walls, .. } = &mut request.geometry else {
            panic!()
        };
        walls[0].ceiling_height_mm = 2600.;
        let second = draft(&source, "acceptance", &request, Some(&first.report)).unwrap();
        assert!(second.ndjson.contains("手修正した壁名"));
        assert!(!second.ndjson.contains(deleted_id.as_str()));
        assert!(
            second
                .report
                .warnings
                .iter()
                .any(|w| w.contains("manual edits retained"))
        );
        install(&mut source, &second);
        let mut bad = source.clone();
        let e = bad.drawings[0]
            .entities
            .iter_mut()
            .find(|r| r.entity.id() == label_id)
            .unwrap();
        let Entity::Text { at, .. } = &mut e.entity else {
            panic!()
        };
        at[1] += 100.;
        let DraftGeometry::WallElevations { walls, .. } = &mut request.geometry else {
            panic!()
        };
        walls[0].ceiling_height_mm = 2800.;
        assert!(
            draft(&bad, "acceptance", &request, Some(&second.report))
                .unwrap_err()
                .to_string()
                .contains("conflicts")
        );
    }
    #[test]
    fn rejects_missing_conversion_factor_and_invalid_openings_and_duplicate_keys() {
        let (_temp, source) = source();
        let mut request = self::walls();
        let DraftGeometry::WallElevations {
            walls: wall_list, ..
        } = &mut request.geometry
        else {
            panic!()
        };
        wall_list[0].openings[0].first = [5., 5.];
        assert!(draft(&source, "acceptance", &request, None).is_err());
        let mut value = serde_json::to_value(walls()).unwrap();
        value["geometry"]["walls"][0]
            .as_object_mut()
            .unwrap()
            .remove("coordinate_mm_per_unit");
        assert!(serde_json::from_value::<DraftRequest>(value).is_err());
        let mut request = walls();
        let DraftGeometry::WallElevations { walls, .. } = &mut request.geometry else {
            panic!()
        };
        walls[0].openings[0].height_mm = 2000.;
        assert!(draft(&source, "acceptance", &request, None).is_err());
    }
    #[test]
    fn coordinate_copy_scales_geometry_without_touching_original_or_paper_scale() {
        let (_temp, source) = source();
        let request: DraftRequest = serde_json::from_value(json!({"key":"normalize","layer":"0-1","geometry":{"type":"coordinate_copy","source_drawing":"acceptance","entity_ids":["ent_01JZ0000000000000000000101"],"coordinate_mm_per_unit":100,"base_point":[6000,0],"at_mm":[0,0]}})).unwrap();
        let candidate = draft(&source, "acceptance", &request, None).unwrap();
        let Entity::Line { p1, p2, .. } =
            &candidate.report.generated["source/ent_01JZ0000000000000000000101"]
        else {
            panic!()
        };
        assert_eq!(*p1, [0., 0.]);
        assert_eq!(*p2, [150000., 0.]);
        assert_eq!(source.drawings[0].layouts.active().unwrap().scale, "1/50");
    }
    #[test]
    fn wall_dimensions_follow_direct_outline_edits_and_regeneration() {
        let (_temp, mut source) = source();
        let request = walls();
        let first = draft(&source, "acceptance", &request, None).unwrap();
        install(&mut source, &first);
        let outline_id = first.report.generated["A/outline"].id();
        let outline = source.drawings[0]
            .entities
            .iter_mut()
            .find(|r| r.entity.id() == outline_id)
            .unwrap();
        let Entity::Polyline { points, .. } = &mut outline.entity else {
            panic!()
        };
        points[1][0] += 500.;
        points[2][0] += 500.;
        points[2][1] += 200.;
        points[3][1] += 200.;
        save_manual_edits(&mut source);
        for (role, expected) in [("A/width", 4200.), ("A/height", 2600.)] {
            let entity = source.drawings[0]
                .entities
                .iter()
                .find(|r| r.entity.id() == first.report.generated[role].id())
                .unwrap();
            assert_eq!(
                cad_model::evaluate_dimension(&source, &entity.entity)
                    .unwrap()
                    .measured,
                expected
            );
        }
        let regenerated = draft(&source, "acceptance", &request, Some(&first.report)).unwrap();
        install(&mut source, &regenerated);
        let entity = source.drawings[0]
            .entities
            .iter()
            .find(|r| r.entity.id() == first.report.generated["A/width"].id())
            .unwrap();
        assert_eq!(
            cad_model::evaluate_dimension(&source, &entity.entity)
                .unwrap()
                .measured,
            4200.
        );
        let svg = cad_render_svg::render_drawing_svg(&source, "acceptance").unwrap();
        assert!(svg.contains("4200 mm"));
        assert!(svg.contains("2600 mm"));
    }
    #[test]
    fn regeneration_upgrades_legacy_fixed_dimensions_without_losing_manual_edits() {
        let (_temp, mut source) = source();
        let request = walls();
        let mut legacy = draft(&source, "acceptance", &request, None).unwrap();
        for role in ["A/width", "A/height"] {
            let entity = legacy.report.generated.get_mut(role).unwrap();
            let mut value = serde_json::to_value(&*entity).unwrap();
            value["measurement"]["first"] = json!({"kind":"fixed","point":value["p1"]});
            value["measurement"]["second"] = json!({"kind":"fixed","point":value["p2"]});
            *entity = serde_json::from_value(value).unwrap();
        }
        legacy.ndjson = legacy
            .ndjson
            .lines()
            .map(|line| {
                let entity: Entity = serde_json::from_str(line).unwrap();
                let replacement = legacy
                    .report
                    .generated
                    .values()
                    .find(|e| e.id() == entity.id());
                format!(
                    "{}\n",
                    serde_json::to_string(replacement.unwrap_or(&entity)).unwrap()
                )
            })
            .collect();
        install(&mut source, &legacy);
        for record in &mut source.drawings[0].entities {
            if record.entity.id() == legacy.report.generated["A/outline"].id() {
                let Entity::Polyline { points, .. } = &mut record.entity else {
                    panic!()
                };
                points[1][0] += 500.;
                points[2][0] += 500.;
            } else if record.entity.id() == legacy.report.generated["A/width"].id() {
                let Entity::Dimension { offset, .. } = &mut record.entity else {
                    panic!()
                };
                *offset = -450.;
            }
        }
        save_manual_edits(&mut source);
        let migrated = draft(&source, "acceptance", &request, Some(&legacy.report)).unwrap();
        install(&mut source, &migrated);
        let width = &source.drawings[0]
            .entities
            .iter()
            .find(|r| r.entity.id() == legacy.report.generated["A/width"].id())
            .unwrap()
            .entity;
        assert_eq!(
            cad_model::evaluate_dimension(&source, width)
                .unwrap()
                .measured,
            4200.
        );
        let Entity::Dimension { offset, .. } = width else {
            panic!()
        };
        assert_eq!(*offset, -450.);
    }
    #[test]
    fn deleting_a_wall_requires_deleting_its_referenced_dimensions() {
        let (_temp, mut source) = source();
        let request = walls();
        let first = draft(&source, "acceptance", &request, None).unwrap();
        install(&mut source, &first);
        source.drawings[0]
            .entities
            .retain(|r| r.entity.id() != first.report.generated["A/outline"].id());
        assert!(draft(&source, "acceptance", &request, Some(&first.report)).is_err());
        source.drawings[0].entities.retain(|r| {
            !["A/width", "A/height"]
                .iter()
                .any(|role| r.entity.id() == first.report.generated[*role].id())
        });
        save_manual_edits(&mut source);
        let regenerated = draft(&source, "acceptance", &request, Some(&first.report)).unwrap();
        for role in ["A/outline", "A/width", "A/height"] {
            assert!(
                !regenerated
                    .ndjson
                    .contains(first.report.generated[role].id().as_str())
            );
        }
    }
    #[test]
    fn coordinate_copy_preserves_source_pens_unless_explicitly_overridden() {
        let (_temp, source) = source();
        let before = cad_model::source_manifest(&source.root).unwrap();
        let mut request: DraftRequest = serde_json::from_value(json!({"key":"pens","layer":"0-1","geometry":{"type":"coordinate_copy","source_drawing":"acceptance","entity_ids":["ent_01JZ0000000000000000000101","ent_01JZ0000000000000000000105"],"coordinate_mm_per_unit":100,"base_point":[0,0],"at_mm":[0,0]}})).unwrap();
        let copied = draft(&source, "acceptance", &request, None).unwrap();
        assert_eq!(
            copied.report.generated["source/ent_01JZ0000000000000000000105"].pen(),
            Some("dashed")
        );
        assert_eq!(
            copied.report.generated["source/ent_01JZ0000000000000000000101"].pen(),
            None
        );
        request.pen = Some("dashed".into());
        let overridden = draft(&source, "acceptance", &request, None).unwrap();
        assert!(
            overridden
                .report
                .generated
                .values()
                .all(|e| e.pen() == Some("dashed"))
        );
        assert_eq!(cad_model::source_manifest(&source.root).unwrap(), before);
    }
    #[test]
    fn wall_minimum_length_is_checked_in_converted_mm() {
        let (_temp, source) = source();
        let mut request = walls();
        let DraftGeometry::WallElevations { walls, .. } = &mut request.geometry else {
            panic!()
        };
        walls[0].right = [0.00037, 0.];
        walls[0].coordinate_mm_per_unit = 10_000_000.;
        walls[0].openings.clear();
        let candidate = draft(&source, "acceptance", &request, None).unwrap();
        let Entity::Polyline { points, .. } = &candidate.report.generated["A/outline"] else {
            panic!()
        };
        assert!((points[1][0] - points[0][0] - 3700.).abs() < 1e-9);
        let DraftGeometry::WallElevations { walls, .. } = &mut request.geometry else {
            panic!()
        };
        walls[0].coordinate_mm_per_unit = 1.;
        assert!(draft(&source, "acceptance", &request, None).is_err());
        let DraftGeometry::WallElevations { walls, .. } = &mut request.geometry else {
            panic!()
        };
        walls[0].right = walls[0].left;
        assert!(draft(&source, "acceptance", &request, None).is_err());
    }
    #[test]
    fn regeneration_preserves_crlf_and_manual_formatting() {
        let (_temp, mut source) = source();
        let file = source.root.join("drawings/acceptance/entities.ndjson");
        let original = std::fs::read_to_string(&file)
            .unwrap()
            .replace("\n", "\r\n");
        std::fs::write(&file, &original).unwrap();
        assert!(cad_check::check_project(&source.root).is_ok());
        source = cad_model::load_project(&source.root).unwrap();
        let request = walls();
        let first = draft(&source, "acceptance", &request, None).unwrap();
        assert!(first.ndjson.starts_with(&original));
        assert!(!first.ndjson.replace("\r\n", "").contains('\n'));
        install(&mut source, &first);
        let again = draft(&source, "acceptance", &request, Some(&first.report)).unwrap();
        assert_eq!(again.ndjson, first.ndjson);
    }
    #[test]
    fn generation_ids_survive_a_new_checkout_path() {
        let (_temp, source) = source();
        let request = walls();
        let candidate = draft(&source, "acceptance", &request, None).unwrap();
        let (_other, mut relocated) = self::source();
        install(&mut relocated, &candidate);
        let regenerated =
            draft(&relocated, "acceptance", &request, Some(&candidate.report)).unwrap();
        assert_eq!(candidate.ndjson, regenerated.ndjson);
        for (role, entity) in &candidate.report.generated {
            assert_eq!(entity.id(), regenerated.report.generated[role].id());
        }
    }
    #[test]
    fn repeat_is_stable_and_id_collisions_without_a_previous_report_are_blocked() {
        let (_temp, mut source) = source();
        let request: DraftRequest = serde_json::from_value(json!({"key":"walls","layer":"0-1","geometry":{"type":"repeat","copies":3,"step_mm":[2000,0],"geometry":{"type":"double_line","p1":[0,7000],"p2":[1000,7000],"width":100,"centerline":false}}})).unwrap();
        let first = draft(&source, "acceptance", &request, None).unwrap();
        let again = draft(&source, "acceptance", &request, None).unwrap();
        assert_eq!(first.ndjson, again.ndjson);
        assert_eq!(first.report.generated.len(), 6);
        install(&mut source, &first);
        assert!(draft(&source, "acceptance", &request, None).is_err());
        let regeneration = draft(&source, "acceptance", &request, Some(&first.report)).unwrap();
        assert_eq!(regeneration.ndjson, first.ndjson);
    }
}
