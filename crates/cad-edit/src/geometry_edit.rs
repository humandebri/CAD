use super::*;

pub(super) fn verify_source_checks(path: &Path, operation: &EditOperation) -> EditResult<()> {
    match operation {
        EditOperation::SourceChecked {
            operation,
            expected_files,
        } => {
            if cad_model::source_manifest(path).map_err(|e| invalid(e.to_string()))?
                != *expected_files
            {
                return Err(EditError::RevisionConflict);
            }
            verify_source_checks(path, operation)
        }
        EditOperation::ResolveDimensions { operation, .. } => verify_source_checks(path, operation),
        EditOperation::Batch { operations } => {
            for op in operations {
                verify_source_checks(path, op)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

pub(super) fn warnings(
    project: &ProjectSource,
    entities: &[Entity],
    operation: &EditOperation,
) -> Vec<String> {
    match operation {
        EditOperation::Stretch {
            entity_ids,
            min,
            max,
            ..
        } => entity_ids
            .iter()
            .filter_map(|id| {
                let entity = entities.iter().find(|e| e.id().as_str() == id)?;
                if ensure_layer_editable(project, entity.layer()).is_err() {
                    return Some(format!("{id}: hidden or locked entity excluded"));
                }
                if !matches!(
                    entity,
                    Entity::Line { .. }
                        | Entity::Polyline { .. }
                        | Entity::Circle { .. }
                        | Entity::Arc { .. }
                        | Entity::Text { .. }
                        | Entity::Point { .. }
                        | Entity::BlockRef { .. }
                ) {
                    return Some(format!("{id}: entity is not a direct stretch target"));
                }
                if !matches!(entity, Entity::Line { .. } | Entity::Polyline { .. })
                    && !stretch_bbox(project, entity, &mut BTreeSet::new())
                        .ok()
                        .flatten()
                        .is_some_and(|b| inside(b.min, *min, *max) && inside(b.max, *min, *max))
                {
                    return Some(format!(
                        "{id}: full containment cannot be proven; entity excluded from stretch"
                    ));
                }
                None
            })
            .collect(),
        EditOperation::SourceChecked { operation, .. }
        | EditOperation::ResolveDimensions { operation, .. } => {
            warnings(project, entities, operation)
        }
        EditOperation::Batch { operations } => operations
            .iter()
            .flat_map(|op| warnings(project, entities, op))
            .collect(),
        _ => vec![],
    }
}

pub(super) fn stretch_bbox(
    project: &ProjectSource,
    entity: &Entity,
    visited: &mut BTreeSet<String>,
) -> EditResult<Option<cad_model::BBox>> {
    transformed_bbox(project, entity, [1., 0., 0., 1., 0., 0.], visited)
}

fn transformed_bbox(
    project: &ProjectSource,
    entity: &Entity,
    matrix: [f64; 6],
    visited: &mut BTreeSet<String>,
) -> EditResult<Option<cad_model::BBox>> {
    let transform = |p: Point| {
        [
            matrix[0] * p[0] + matrix[2] * p[1] + matrix[4],
            matrix[1] * p[0] + matrix[3] * p[1] + matrix[5],
        ]
    };
    if let Entity::BlockRef {
        block,
        at,
        rotation_deg,
        scale,
        mirror_x,
        mirror_y,
        ..
    } = entity
    {
        if !visited.insert(block.clone()) {
            return Err(invalid(format!("cyclic block {block}")));
        }
        let definition = project
            .blocks
            .get(block)
            .ok_or_else(|| invalid(format!("missing block {block}")))?;
        let (sin, cos) = rotation_deg.to_radians().sin_cos();
        let sx = scale * if *mirror_x { -1. } else { 1. };
        let sy = scale * if *mirror_y { -1. } else { 1. };
        let base = definition.config.base_point;
        let local = [
            cos * sx,
            sin * sx,
            -sin * sy,
            cos * sy,
            at[0] - cos * sx * base[0] + sin * sy * base[1],
            at[1] - sin * sx * base[0] - cos * sy * base[1],
        ];
        let composed = [
            matrix[0] * local[0] + matrix[2] * local[1],
            matrix[1] * local[0] + matrix[3] * local[1],
            matrix[0] * local[2] + matrix[2] * local[3],
            matrix[1] * local[2] + matrix[3] * local[3],
            matrix[0] * local[4] + matrix[2] * local[5] + matrix[4],
            matrix[1] * local[4] + matrix[3] * local[5] + matrix[5],
        ];
        let mut bounds = Vec::new();
        for record in &definition.entities {
            let Some(b) = transformed_bbox(project, &record.entity, composed, visited)? else {
                visited.remove(block);
                return Ok(None);
            };
            bounds.extend([b.min, b.max]);
        }
        visited.remove(block);
        return Ok(cad_model::BBox::from_points(&bounds));
    }
    let scale = matrix[0].hypot(matrix[1]);
    let reflected = matrix[0] * matrix[3] - matrix[1] * matrix[2] < 0.0;
    let points = match entity {
        Entity::Line { p1, p2, .. } => vec![transform(*p1), transform(*p2)],
        Entity::Polyline { points, .. } | Entity::Solid { points, .. } => {
            points.iter().copied().map(transform).collect()
        }
        Entity::Hatch { loops, .. } => loops.iter().flatten().copied().map(transform).collect(),
        Entity::Circle { center, radius, .. } => {
            let c = transform(*center);
            let r = radius * scale;
            vec![[c[0] - r, c[1] - r], [c[0] + r, c[1] + r]]
        }
        Entity::Arc {
            center,
            radius,
            start_deg,
            end_deg,
            ..
        } => {
            let c = transform(*center);
            let a = transform(polar_point(*center, *radius, *start_deg));
            let b = transform(polar_point(*center, *radius, *end_deg));
            let start = angle(c, a);
            let end = start + (end_deg - start_deg) * if reflected { -1. } else { 1. };
            let mut p = vec![a, b];
            for q in [0., 90., 180., 270.] {
                if angle_on_arc(q, start, end) {
                    p.push(polar_point(c, radius * scale, q));
                }
            }
            p
        }
        Entity::Text {
            style,
            at,
            rotation_deg,
            mirror_y,
            value,
            ..
        } => {
            let Some(style) = project.styles.text_styles.get(style) else {
                return Ok(None);
            };
            let count = value.chars().count();
            let width = (count as f64 * style.width
                + count.saturating_sub(1) as f64 * style.spacing)
                .max(0.0);
            let height = style.height.max(0.0);
            if !width.is_finite() || !height.is_finite() {
                return Ok(None);
            }
            // SVG uses this exact textLength and alignment with spacingAndGlyphs.
            let x = match style.align {
                cad_model::TextAlign::Left => 0.,
                cad_model::TextAlign::Center => -width / 2.,
                cad_model::TextAlign::Right => -width,
            };
            let y = if *mirror_y { -height } else { 0. };
            let (sin, cos) = rotation_deg.to_radians().sin_cos();
            [
                [x, y],
                [x + width, y],
                [x + width, y + height],
                [x, y + height],
            ]
            .map(|p| {
                transform([
                    at[0] + p[0] * cos - p[1] * sin,
                    at[1] + p[0] * sin + p[1] * cos,
                ])
            })
            .to_vec()
        }
        Entity::Dimension { .. } => {
            let Ok(primitives) = cad_model::dimension_primitives(project, entity) else {
                return Ok(None);
            };
            let mut result = Vec::new();
            for primitive in primitives {
                let Some(b) = transformed_bbox(project, &primitive, matrix, visited)? else {
                    return Ok(None);
                };
                result.extend([b.min, b.max]);
            }
            result
        }
        _ => {
            let Some(b) = cad_model::entity_bbox(entity) else {
                return Ok(None);
            };
            [b.min, [b.min[0], b.max[1]], b.max, [b.max[0], b.min[1]]]
                .map(transform)
                .to_vec()
        }
    };
    Ok(cad_model::BBox::from_points(&points))
}

pub(super) fn nearest_curve_point(entity: &Entity, point: Point) -> Option<Point> {
    let (center, radius) = circle_data(entity)?;
    let direction = [point[0] - center[0], point[1] - center[1]];
    let length = direction[0].hypot(direction[1]);
    if length <= 1e-9 {
        return None;
    }
    let candidate = [
        center[0] + radius * direction[0] / length,
        center[1] + radius * direction[1] / length,
    ];
    if on_arc(entity, candidate) {
        Some(candidate)
    } else if let Entity::Arc {
        start_deg, end_deg, ..
    } = entity
    {
        let a = polar_point(center, radius, *start_deg);
        let b = polar_point(center, radius, *end_deg);
        Some(if distance_sq(a, point) < distance_sq(b, point) {
            a
        } else {
            b
        })
    } else {
        None
    }
}

pub(super) fn perpendicular_curve_point(entity: &Entity, reference: Point) -> Option<Point> {
    let candidate = nearest_curve_point(entity, reference)?;
    let (center, _) = circle_data(entity)?;
    let a = [candidate[0] - center[0], candidate[1] - center[1]];
    let b = [reference[0] - center[0], reference[1] - center[1]];
    ((a[0] * b[1] - a[1] * b[0]).abs() <= 1e-9 * a[0].hypot(a[1]) * b[0].hypot(b[1]))
        .then_some(candidate)
}

pub(super) fn remap_retained_vertices(
    before: &[Entity],
    after: &mut [Entity],
    raw: &mut [RawLine],
) -> EditResult<()> {
    let original = before
        .iter()
        .cloned()
        .enumerate()
        .map(|(i, entity)| EntityRecord {
            line: i + 1,
            entity,
        })
        .collect::<Vec<_>>();
    let snapshot = after.to_vec();
    for (position, dimension) in after.iter_mut().enumerate() {
        let mut changed = false;
        for anchor in cad_model::dimension_anchors_mut(dimension) {
            let cad_model::DimensionAnchor::Entity {
                entity_id,
                feature,
                index,
            } = anchor.clone()
            else {
                continue;
            };
            if index == Some(usize::MAX) {
                continue;
            }
            let (
                Some(Entity::Polyline { points: old, .. }),
                Some(Entity::Polyline { points: new, .. }),
            ) = (
                before.iter().find(|e| e.id() == &entity_id),
                snapshot.iter().find(|e| e.id() == &entity_id),
            )
            else {
                continue;
            };
            if old.len() == new.len() {
                continue;
            }
            if !matches!(
                feature,
                cad_model::DimensionFeature::Vertex
                    | cad_model::DimensionFeature::Start
                    | cad_model::DimensionFeature::End
            ) {
                continue;
            }
            let point = cad_model::resolve_dimension_anchor(anchor, &original)
                .map_err(EditError::InvalidEntity)?;
            let mut candidates = Vec::new();
            for entity in &snapshot {
                if entity.id() != &entity_id && before.iter().any(|e| e.id() == entity.id()) {
                    continue;
                }
                if let Entity::Polyline { points, .. } = entity {
                    for (i, p) in points.iter().enumerate() {
                        if *p == point {
                            candidates.push((entity.id().clone(), i));
                        }
                    }
                }
            }
            // A repeated closure point is the same retained topological vertex.
            if candidates.len() == 2 && candidates.iter().all(|(id, _)| *id == candidates[0].0) {
                let indexes = candidates.iter().map(|(_, i)| *i).collect::<Vec<_>>();
                if indexes[0] == 0 && snapshot.iter().find(|e|e.id()==&candidates[0].0).is_some_and(|e|matches!(e,Entity::Polyline{points,closed:true,..} if indexes[1]==points.len()-1)) {
                    candidates.truncate(1);
                }
            }
            *anchor = if candidates.len() == 1 {
                cad_model::DimensionAnchor::Entity {
                    entity_id: candidates[0].0.clone(),
                    feature: cad_model::DimensionFeature::Vertex,
                    index: Some(candidates[0].1),
                }
            } else {
                cad_model::DimensionAnchor::Entity {
                    entity_id,
                    feature: cad_model::DimensionFeature::Vertex,
                    index: Some(usize::MAX),
                }
            };
            let _ = index;
            changed = true;
        }
        if changed {
            raw[position].content = entity_json(dimension)?;
        }
    }
    Ok(())
}

pub(super) fn remap_trim_endpoints(
    before: &[Entity],
    after: &mut [Entity],
    raw: &mut [RawLine],
    operation: &EditOperation,
) -> EditResult<()> {
    fn targets(op: &EditOperation, out: &mut BTreeSet<String>) {
        match op {
            EditOperation::Trim {
                target_entity_id, ..
            } => {
                out.insert(target_entity_id.clone());
            }
            EditOperation::Batch { operations } => {
                for op in operations {
                    targets(op, out);
                }
            }
            EditOperation::SourceChecked { operation, .. }
            | EditOperation::ResolveDimensions { operation, .. } => targets(operation, out),
            _ => {}
        }
    }
    let mut trimmed = BTreeSet::new();
    targets(operation, &mut trimmed);
    if trimmed.is_empty() {
        return Ok(());
    }
    let records = |entities: &[Entity]| {
        entities
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, entity)| EntityRecord {
                line: i + 1,
                entity,
            })
            .collect::<Vec<_>>()
    };
    let old = records(before);
    let new = records(after);
    for (i, dimension) in after.iter_mut().enumerate() {
        let mut changed = false;
        for anchor in cad_model::dimension_anchors_mut(dimension) {
            let cad_model::DimensionAnchor::Entity {
                entity_id, feature, ..
            } = anchor.clone()
            else {
                continue;
            };
            if !trimmed.contains(entity_id.as_str())
                || !matches!(
                    feature,
                    cad_model::DimensionFeature::Start | cad_model::DimensionFeature::End
                )
            {
                continue;
            }
            let point = cad_model::resolve_dimension_anchor(anchor, &old)
                .map_err(EditError::InvalidEntity)?;
            let mut found = Vec::new();
            for candidate in &new {
                if candidate.entity.id() != &entity_id
                    && before.iter().any(|e| e.id() == candidate.entity.id())
                {
                    continue;
                }
                for feature in [
                    cad_model::DimensionFeature::Start,
                    cad_model::DimensionFeature::End,
                ] {
                    let a = cad_model::DimensionAnchor::Entity {
                        entity_id: candidate.entity.id().clone(),
                        feature,
                        index: None,
                    };
                    if cad_model::resolve_dimension_anchor(&a, &new).is_ok_and(|p| p == point) {
                        found.push(a);
                    }
                }
            }
            *anchor = if found.len() == 1 {
                found.remove(0)
            } else {
                cad_model::DimensionAnchor::Entity {
                    entity_id,
                    feature: cad_model::DimensionFeature::Vertex,
                    index: Some(usize::MAX),
                }
            };
            changed = true;
        }
        if changed {
            raw[i].content = entity_json(dimension)?;
        }
    }
    Ok(())
}

pub(super) fn dimension_impacts(before: &[Entity], after: &[Entity]) -> Vec<String> {
    let records = after
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, entity)| EntityRecord {
            line: index + 1,
            entity,
        })
        .collect::<Vec<_>>();
    after
        .iter()
        .filter_map(|dimension| {
            let mut d = dimension.clone();
            let broken = cad_model::dimension_anchors_mut(&mut d)
                .into_iter()
                .any(|anchor| {
                    if cad_model::resolve_dimension_anchor(anchor, &records).is_err() {
                        return true;
                    }
                    if let cad_model::DimensionAnchor::Entity {
                        entity_id,
                        feature,
                        index,
                    } = anchor
                    {
                        let old = before.iter().find(|e| e.id() == entity_id);
                        let new = after.iter().find(|e| e.id() == entity_id);
                        match (old, new) {
                            (Some(a), Some(b)) if entity_kind(a) != entity_kind(b) => !matches!(
                                feature,
                                cad_model::DimensionFeature::Center
                                    | cad_model::DimensionFeature::Radius
                            ),
                            _ if matches!(feature, cad_model::DimensionFeature::Vertex)
                                && *index == Some(usize::MAX) =>
                            {
                                true
                            }
                            _ => false,
                        }
                    } else {
                        false
                    }
                });
            broken.then(|| dimension.id().as_str().to_owned())
        })
        .collect()
}

fn angle(c: Point, p: Point) -> f64 {
    (p[1] - c[1]).atan2(p[0] - c[0]).to_degrees()
}
fn arc_parameter(start: f64, end: f64, a: f64) -> f64 {
    if end >= start {
        (a - start).rem_euclid(360.0)
    } else {
        (start - a).rem_euclid(360.0)
    }
}
fn on_arc(e: &Entity, p: Point) -> bool {
    match e {
        Entity::Arc {
            center,
            start_deg,
            end_deg,
            ..
        } => angle_on_arc(angle(*center, p), *start_deg, *end_deg),
        _ => true,
    }
}
fn line_circle(a: Point, b: Point, c: Point, r: f64, finite_segment: bool) -> Vec<Point> {
    let d = [b[0] - a[0], b[1] - a[1]];
    let f = [a[0] - c[0], a[1] - c[1]];
    let aa = d[0] * d[0] + d[1] * d[1];
    let bb = 2.0 * (f[0] * d[0] + f[1] * d[1]);
    let cc = f[0] * f[0] + f[1] * f[1] - r * r;
    let disc = bb * bb - 4.0 * aa * cc;
    if aa <= 1e-18 || disc < -1e-9 {
        return vec![];
    }
    let mut out = Vec::new();
    for t in [
        (-bb - disc.max(0.0).sqrt()) / (2.0 * aa),
        (-bb + disc.max(0.0).sqrt()) / (2.0 * aa),
    ] {
        if (!finite_segment || (-1e-9..=1.0 + 1e-9).contains(&t))
            && !out
                .iter()
                .any(|p| distance_sq(*p, [a[0] + t * d[0], a[1] + t * d[1]]) < 1e-16)
        {
            out.push([a[0] + t * d[0], a[1] + t * d[1]]);
        }
    }
    out
}
fn circle_data(e: &Entity) -> Option<(Point, f64)> {
    match e {
        Entity::Circle { center, radius, .. } | Entity::Arc { center, radius, .. } => {
            Some((*center, *radius))
        }
        _ => None,
    }
}
pub(super) fn intersections(a: &Entity, b: &Entity) -> EditResult<Vec<Point>> {
    let mut out = match (circle_data(a), circle_data(b)) {
        (Some((c, r)), Some((d, s))) => {
            let distance = distance_sq(c, d).sqrt();
            if distance <= 1e-9 || distance > r + s + 1e-9 || distance < (r - s).abs() - 1e-9 {
                vec![]
            } else {
                let x = (r * r - s * s + distance * distance) / (2.0 * distance);
                let h = (r * r - x * x).max(0.0).sqrt();
                let u = [(d[0] - c[0]) / distance, (d[1] - c[1]) / distance];
                vec![
                    [c[0] + x * u[0] - h * u[1], c[1] + x * u[1] + h * u[0]],
                    [c[0] + x * u[0] + h * u[1], c[1] + x * u[1] - h * u[0]],
                ]
            }
        }
        (Some((c, r)), None) => path_segments(b)?
            .iter()
            .flat_map(|(x, y)| line_circle(*x, *y, c, r, true))
            .collect(),
        (None, Some((c, r))) => path_segments(a)?
            .iter()
            .flat_map(|(x, y)| line_circle(*x, *y, c, r, true))
            .collect(),
        (None, None) => segment_intersections(&path_segments(a)?, &path_segments(b)?)
            .into_iter()
            .map(|(_, p)| p)
            .collect(),
    };
    out.retain(|p| on_arc(a, *p) && on_arc(b, *p));
    let mut unique = Vec::new();
    for p in out {
        if !unique.iter().any(|q| distance_sq(*q, p) < 1e-16) {
            unique.push(p);
        }
    }
    Ok(unique)
}
fn arc_with_range(target: &Entity, start: f64, end: f64) -> EditResult<Entity> {
    let mut v = serde_json::to_value(target).map_err(|e| invalid(e.to_string()))?;
    v["type"] = Value::String("arc".into());
    v["start_deg"] = Value::from(start);
    v["end_deg"] = Value::from(end);
    serde_json::from_value(v).map_err(|e| invalid(e.to_string()))
}
pub(super) fn extend(target: &Entity, boundary: &Entity, pick: Point) -> EditResult<Entity> {
    if let Entity::Arc {
        center,
        start_deg,
        end_deg,
        ..
    } = target
    {
        let span = (end_deg - start_deg).abs();
        if span >= 360.0 - 1e-8 || span <= 1e-8 {
            return Err(invalid("cannot extend zero or full circle sweep"));
        }
        let whole = arc_with_range(target, 0.0, 360.0)?;
        let hits = intersections(&whole, boundary)?;
        let sign = (end_deg - start_deg).signum();
        let pick_t = arc_parameter(*start_deg, *end_deg, angle(*center, pick));
        let start_side = pick_t.min(360.0 - pick_t) < (pick_t - span).abs();
        let endpoint = if start_side { *start_deg } else { *end_deg };
        let direction = if start_side { -sign } else { sign };
        let advance = hits
            .iter()
            .map(|p| arc_parameter(endpoint, endpoint + direction, angle(*center, *p)))
            .filter(|v| *v > 1e-8 && *v + span < 360.0 - 1e-8)
            .min_by(f64::total_cmp)
            .ok_or_else(|| invalid("no forward arc extension intersection"))?;
        return if start_side {
            arc_with_range(target, start_deg + direction * advance, *end_deg)
        } else {
            arc_with_range(target, *start_deg, end_deg + direction * advance)
        };
    }
    let mut points = match target {
        Entity::Line { p1, p2, .. } => vec![*p1, *p2],
        Entity::Polyline {
            points,
            closed: false,
            ..
        } => points.clone(),
        _ => return Err(invalid("extend requires line, open polyline, or arc")),
    };
    if points.len() < 2 {
        return Err(invalid("extend requires two vertices"));
    }
    let start = nearest_path_distance(&points, pick) <= path_length(&points) / 2.0;
    let index = if start { 0 } else { points.len() - 1 };
    let endpoint = points[index];
    let adjacent = points[if start { 1 } else { points.len() - 2 }];
    let d = [endpoint[0] - adjacent[0], endpoint[1] - adjacent[1]];
    let end = [endpoint[0] + d[0], endpoint[1] + d[1]];
    let mut hits = if let Some((c, r)) = circle_data(boundary) {
        line_circle(endpoint, end, c, r, false)
    } else {
        path_segments(boundary)?
            .into_iter()
            .filter_map(|(a, b)| {
                infinite_line_intersection(endpoint, end, a, b)
                    .filter(|p| point_on_segment(*p, a, b))
            })
            .collect()
    };
    hits.retain(|p| {
        on_arc(boundary, *p) && (p[0] - endpoint[0]) * d[0] + (p[1] - endpoint[1]) * d[1] > 1e-9
    });
    let point = hits
        .into_iter()
        .min_by(|a, b| distance_sq(*a, endpoint).total_cmp(&distance_sq(*b, endpoint)))
        .ok_or_else(|| invalid("no forward extension intersection"))?;
    points[index] = point;
    replace_entity_points(target, &points)
}
pub(super) fn trim(target: &Entity, cutter: &Entity, pick: Point) -> EditResult<Vec<Entity>> {
    let hits = intersections(target, cutter)?;
    if let Some((center, _)) = circle_data(target) {
        let (start, end, circle) = match target {
            Entity::Arc {
                start_deg, end_deg, ..
            } => (
                *start_deg,
                *end_deg,
                (end_deg - start_deg).abs() >= 360.0 - 1e-9,
            ),
            _ => (0.0, 360.0, true),
        };
        let mut cuts = hits
            .iter()
            .map(|p| arc_parameter(start, end, angle(center, *p)))
            .collect::<Vec<_>>();
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-8);
        if cuts.is_empty() || (circle && cuts.len() < 2) {
            return Err(invalid("trim requires distinct intersections"));
        }
        let pick_t = arc_parameter(start, end, angle(center, pick));
        let sign = (end - start).signum();
        let span = (end - start).abs();
        if circle {
            let right = cuts.partition_point(|v| *v < pick_t);
            let lo = cuts[(right + cuts.len() - 1) % cuts.len()];
            let hi = cuts[right % cuts.len()];
            let kept_start = hi;
            let mut kept_end = lo;
            while kept_end <= kept_start {
                kept_end += 360.0;
            }
            return Ok(vec![arc_with_range(
                target,
                start + sign * kept_start,
                start + sign * kept_end,
            )?]);
        }
        let lo = cuts.iter().copied().rfind(|v| *v < pick_t).unwrap_or(0.0);
        let hi = cuts.iter().copied().find(|v| *v >= pick_t).unwrap_or(span);
        let mut result = Vec::new();
        if lo > 1e-8 {
            result.push(arc_with_range(target, start, start + sign * lo)?);
        }
        if hi < span - 1e-8 {
            result.push(arc_with_range(target, start + sign * hi, end)?);
        }
        if result.is_empty() {
            return Err(invalid("trim would remove entire arc"));
        }
        return Ok(result);
    }
    if let Entity::Line { p1, p2, .. } = target {
        let length = distance_sq(*p1, *p2);
        if hits.is_empty() || length <= 1e-18 {
            return Err(invalid("trim entities do not intersect"));
        }
        let parameter = |p: Point| {
            ((p[0] - p1[0]) * (p2[0] - p1[0]) + (p[1] - p1[1]) * (p2[1] - p1[1])) / length
        };
        let mut cuts = hits.into_iter().map(parameter).collect::<Vec<_>>();
        cuts.sort_by(f64::total_cmp);
        let t = parameter(pick);
        let lo = cuts.iter().copied().rfind(|v| *v < t).unwrap_or(0.0);
        let hi = cuts.iter().copied().find(|v| *v >= t).unwrap_or(1.0);
        let at = |v: f64| [p1[0] + v * (p2[0] - p1[0]), p1[1] + v * (p2[1] - p1[1])];
        let mut out = Vec::new();
        if lo > 1e-9 {
            out.push(replace_entity_points(target, &[*p1, at(lo)])?);
        }
        if hi < 1.0 - 1e-9 {
            out.push(replace_entity_points(target, &[at(hi), *p2])?);
        }
        if out.is_empty() {
            return Err(invalid("trim would remove entire line"));
        }
        return Ok(out);
    }
    if let Entity::Polyline { points, closed, .. } = target {
        let mut path = points.clone();
        if *closed && points.first() != points.last() {
            path.push(points[0]);
        }
        let total = path_length(&path);
        let mut cuts = hits
            .iter()
            .map(|p| nearest_path_distance(&path, *p))
            .collect::<Vec<_>>();
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-8);
        if cuts.is_empty() || (*closed && cuts.len() < 2) {
            return Err(invalid("trim requires distinct intersections"));
        }
        let pick_t = nearest_path_distance(&path, pick);
        let lo = cuts.iter().copied().rfind(|v| *v < pick_t);
        let hi = cuts.iter().copied().find(|v| *v >= pick_t);
        let slice = |from: f64, to: f64| -> Vec<Point> {
            let mut result = Vec::new();
            let mut distance = 0.0;
            for pair in path.windows(2) {
                let length = distance_sq(pair[0], pair[1]).sqrt();
                let end = distance + length;
                if length > 1e-9 && end >= from && distance <= to {
                    for t in [
                        ((from - distance) / length).clamp(0.0, 1.0),
                        ((to - distance) / length).clamp(0.0, 1.0),
                    ] {
                        let p = [
                            pair[0][0] + t * (pair[1][0] - pair[0][0]),
                            pair[0][1] + t * (pair[1][1] - pair[0][1]),
                        ];
                        if result.last().is_none_or(|q| distance_sq(*q, p) > 1e-16) {
                            result.push(p);
                        }
                    }
                }
                distance = end;
            }
            result
        };
        let pieces = if *closed {
            let lo = lo.unwrap_or(*cuts.last().unwrap());
            let hi = hi.unwrap_or(cuts[0]);
            if hi > lo {
                let mut p = slice(hi, total);
                let tail = slice(0.0, lo);
                p.extend(tail.into_iter().skip(1));
                vec![p]
            } else {
                vec![slice(hi, lo)]
            }
        } else {
            let mut parts = Vec::new();
            if lo.unwrap_or(0.0) > 1e-8 {
                parts.push(slice(0.0, lo.unwrap()));
            }
            if hi.unwrap_or(total) < total - 1e-8 {
                parts.push(slice(hi.unwrap(), total));
            }
            parts
        };
        let mut out = Vec::new();
        for p in pieces {
            if p.len() < 2 {
                continue;
            }
            let mut v = serde_json::to_value(replace_entity_points(target, &p)?)
                .map_err(|e| invalid(e.to_string()))?;
            v["closed"] = Value::Bool(false);
            out.push(serde_json::from_value(v).map_err(|e| invalid(e.to_string()))?);
        }
        if out.is_empty() {
            return Err(invalid("trim would remove entire polyline"));
        }
        return Ok(out);
    }
    Err(invalid("trim requires line, polyline, circle, or arc"))
}

fn invalid(message: impl Into<String>) -> EditError {
    EditError::InvalidEntity(message.into())
}
fn finite(points: &[Point]) -> EditResult<()> {
    if points.iter().flatten().all(|v| v.is_finite()) {
        Ok(())
    } else {
        Err(invalid("coordinates must be finite"))
    }
}
fn inside(p: Point, min: Point, max: Point) -> bool {
    (0..2).all(|i| p[i] >= min[i] && p[i] <= max[i])
}

pub(super) fn apply(
    operation: &EditOperation,
    project: &ProjectSource,
    entities: &mut Vec<Entity>,
    raw: &mut Vec<RawLine>,
) -> EditResult<OperationResult> {
    match operation {
        EditOperation::Endpoint {
            entity_id,
            vertex_index,
            to,
        } => {
            finite(&[*to])?;
            transform_entities(
                project,
                entities,
                raw,
                std::slice::from_ref(entity_id),
                |e| {
                    let mut points = match e {
                        Entity::Line { p1, p2, .. } => vec![*p1, *p2],
                        Entity::Polyline { points, .. } => points.clone(),
                        _ => {
                            return Err(invalid(format!(
                                "{entity_id}: endpoint requires line or polyline"
                            )));
                        }
                    };
                    *points
                        .get_mut(*vertex_index)
                        .ok_or_else(|| invalid(format!("{entity_id}: invalid vertex_index")))? =
                        *to;
                    if matches!(e, Entity::Polyline { closed: true, .. }) {
                        let last = points.len() - 1;
                        if *vertex_index == 0 {
                            points[last] = *to;
                        } else if *vertex_index == last {
                            points[0] = *to;
                        }
                    }
                    replace_entity_points(e, &points)
                },
                "endpoint",
            )
        }
        EditOperation::Stretch {
            entity_ids,
            min,
            max,
            delta,
        } => {
            finite(&[*min, *max, *delta])?;
            ensure_unique_entity_ids(entity_ids)?;
            if (0..2).any(|i| min[i] > max[i]) {
                return Err(invalid("stretch min exceeds max"));
            }
            let editable_ids = entity_ids
                .iter()
                .filter_map(|id| match entity_index(entities, id) {
                    Ok(index)
                        if ensure_layer_editable(project, entities[index].layer()).is_ok() =>
                    {
                        Some(Ok(id.clone()))
                    }
                    Ok(_) => None,
                    Err(e) => Some(Err(e)),
                })
                .collect::<EditResult<Vec<_>>>()?;
            transform_entities(
                project,
                entities,
                raw,
                &editable_ids,
                |e| {
                    let shift = |p: Point| {
                        if inside(p, *min, *max) {
                            [p[0] + delta[0], p[1] + delta[1]]
                        } else {
                            p
                        }
                    };
                    match e {
                        Entity::Line { p1, p2, .. } => {
                            replace_entity_points(e, &[shift(*p1), shift(*p2)])
                        }
                        Entity::Polyline { points, .. } => replace_entity_points(
                            e,
                            &points.iter().copied().map(shift).collect::<Vec<_>>(),
                        ),
                        _ if matches!(
                            e,
                            Entity::Circle { .. }
                                | Entity::Arc { .. }
                                | Entity::Text { .. }
                                | Entity::Point { .. }
                                | Entity::BlockRef { .. }
                        ) && stretch_bbox(project, e, &mut BTreeSet::new())?.is_some_and(
                            |b| inside(b.min, *min, *max) && inside(b.max, *min, *max),
                        ) =>
                        {
                            translate_entity(e, *delta)
                        }
                        _ => Ok(e.clone()),
                    }
                },
                "stretch",
            )
        }
        EditOperation::Rectangle { layer, p1, p2 } => {
            finite(&[*p1, *p2])?;
            if p1[0] == p2[0] || p1[1] == p2[1] {
                return Err(invalid("rectangle width and height must be nonzero"));
            }
            apply_operation(
                &EditOperation::Create {
                    entity: serde_json::json!({"schema_version":cad_model::CURRENT_SCHEMA_VERSION,"type":"polyline","layer":layer,"points":[p1,[p2[0],p1[1]],p2,[p1[0],p2[1]],p1],"closed":true}),
                },
                project,
                entities,
                raw,
            )
        }
        EditOperation::RectangularArray {
            entity_ids,
            rows,
            columns,
            row_spacing,
            column_spacing,
        } => {
            ensure_unique_entity_ids(entity_ids)?;
            if *rows == 0
                || *columns == 0
                || (*rows == 1 && *columns == 1)
                || rows
                    .checked_mul(*columns)
                    .and_then(|n| n.checked_mul(entity_ids.len()))
                    .is_none_or(|n| n > 20_000)
                || !row_spacing.is_finite()
                || !column_spacing.is_finite()
                || (*rows > 1 && *row_spacing == 0.0)
                || (*columns > 1 && *column_spacing == 0.0)
            {
                return Err(invalid(
                    "invalid array count or spacing (maximum 20000 entities)",
                ));
            }
            let mut ids = Vec::new();
            for row in 0..*rows {
                for col in 0..*columns {
                    if row == 0 && col == 0 {
                        continue;
                    }
                    let r = apply_operation(
                        &EditOperation::TranslateMany {
                            entity_ids: entity_ids.clone(),
                            delta: [col as f64 * column_spacing, row as f64 * row_spacing],
                            duplicate: true,
                        },
                        project,
                        entities,
                        raw,
                    )?;
                    ids.extend(r.entity_ids);
                }
            }
            Ok(OperationResult {
                entity_ids: ids,
                operation: "rectangular_array",
            })
        }
        EditOperation::Fillet {
            first_entity_id,
            second_entity_id,
            first_pick,
            second_pick,
            radius,
        } => join_lines(
            project,
            entities,
            raw,
            first_entity_id,
            second_entity_id,
            *first_pick,
            *second_pick,
            Join::Fillet(*radius),
        ),
        EditOperation::Chamfer {
            first_entity_id,
            second_entity_id,
            first_pick,
            second_pick,
            first_distance,
            second_distance,
        } => join_lines(
            project,
            entities,
            raw,
            first_entity_id,
            second_entity_id,
            *first_pick,
            *second_pick,
            Join::Chamfer(*first_distance, *second_distance),
        ),
        _ => unreachable!(),
    }
}

enum Join {
    Fillet(f64),
    Chamfer(f64, f64),
}

#[allow(clippy::too_many_arguments)]
fn join_lines(
    project: &ProjectSource,
    entities: &mut Vec<Entity>,
    raw: &mut Vec<RawLine>,
    first: &str,
    second: &str,
    pick1: Point,
    pick2: Point,
    join: Join,
) -> EditResult<OperationResult> {
    finite(&[pick1, pick2])?;
    if first == second {
        return Err(invalid("join requires two different lines"));
    }
    let a = entity_index(entities, first)?;
    let b = entity_index(entities, second)?;
    ensure_layer_editable(project, entities[a].layer())?;
    ensure_layer_editable(project, entities[b].layer())?;
    let (p1, p2, q1, q2) = match (&entities[a], &entities[b]) {
        (Entity::Line { p1, p2, .. }, Entity::Line { p1: q1, p2: q2, .. }) => (*p1, *p2, *q1, *q2),
        _ => return Err(invalid("fillet/chamfer requires two lines")),
    };
    let intersection = infinite_line_intersection(p1, p2, q1, q2)
        .ok_or_else(|| invalid("parallel lines cannot be joined"))?;
    let direction = |p: Point, x: Point, y: Point| -> EditResult<(Point, Point)> {
        let d = [y[0] - x[0], y[1] - x[1]];
        let n = d[0].hypot(d[1]);
        let dot = (p[0] - intersection[0]) * d[0] + (p[1] - intersection[1]) * d[1];
        if n <= 1e-9 || dot.abs() <= 1e-9 {
            return Err(invalid("pick side is ambiguous"));
        }
        let u = [d[0] / n * dot.signum(), d[1] / n * dot.signum()];
        let projection =
            |v: Point| (v[0] - intersection[0]) * u[0] + (v[1] - intersection[1]) * u[1];
        Ok((u, if projection(x) > projection(y) { x } else { y }))
    };
    let (u, keep1) = direction(pick1, p1, p2)?;
    let (v, keep2) = direction(pick2, q1, q2)?;
    let theta = (u[0] * v[0] + u[1] * v[1]).clamp(-1.0, 1.0).acos();
    let (d1, d2) = match join {
        Join::Chamfer(a, b) => (a, b),
        Join::Fillet(r) => (r / (theta / 2.0).tan(), r / (theta / 2.0).tan()),
    };
    if !d1.is_finite() || !d2.is_finite() || d1 <= 0.0 || d2 <= 0.0 {
        return Err(invalid("join distances/radius must be positive"));
    }
    let t1 = [intersection[0] + u[0] * d1, intersection[1] + u[1] * d1];
    let t2 = [intersection[0] + v[0] * d2, intersection[1] + v[1] * d2];
    for (d, keep) in [(d1, keep1), (d2, keep2)] {
        if d >= distance_sq(intersection, keep).sqrt() - 1e-9 {
            return Err(invalid("join exceeds retained line length"));
        }
    }
    let replace = |entity: &Entity, original: Point, keep: Point, t: Point| {
        replace_entity_points(
            entity,
            &if original == keep {
                [keep, t]
            } else {
                [t, keep]
            },
        )
    };
    let first_new = replace(&entities[a], p1, keep1, t1)?;
    let second_new = replace(&entities[b], q1, keep2, t2)?;
    let mut value = serde_json::to_value(&entities[a]).map_err(|e| invalid(e.to_string()))?;
    value["id"] = Value::String(next_id());
    match join {
        Join::Chamfer(..) => {
            value["p1"] = serde_json::json!(t1);
            value["p2"] = serde_json::json!(t2);
        }
        Join::Fillet(r) => {
            let bisector = [u[0] + v[0], u[1] + v[1]];
            let n = bisector[0].hypot(bisector[1]);
            let h = r / (theta / 2.0).sin();
            let c = [
                intersection[0] + bisector[0] / n * h,
                intersection[1] + bisector[1] / n * h,
            ];
            let start = (t1[1] - c[1]).atan2(t1[0] - c[0]).to_degrees();
            let end = (t2[1] - c[1]).atan2(t2[0] - c[0]).to_degrees();
            let sweep = (end - start + 180.0).rem_euclid(360.0) - 180.0;
            value["type"] = Value::String("arc".into());
            value["center"] = serde_json::json!(c);
            value["radius"] = Value::from(r);
            value["start_deg"] = Value::from(start);
            value["end_deg"] = Value::from(start + sweep);
            value.as_object_mut().unwrap().remove("p1");
            value.as_object_mut().unwrap().remove("p2");
        }
    }
    let joined = create_entity(value)?;
    validate_entity_geometry(&joined)?;
    raw[a].content = entity_json(&first_new)?;
    raw[b].content = entity_json(&second_new)?;
    entities[a] = first_new;
    entities[b] = second_new;
    let id = joined.id().as_str().to_owned();
    append_raw_line(raw, entity_json(&joined)?);
    entities.push(joined);
    Ok(OperationResult {
        entity_ids: vec![first.into(), second.into(), id],
        operation: match join {
            Join::Fillet(_) => "fillet",
            Join::Chamfer(..) => "chamfer",
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entity(mut value: Value) -> Entity {
        value["schema_version"] = cad_model::CURRENT_SCHEMA_VERSION.into();
        value["id"] = "ent_01JZ0000000000000000000010".into();
        value["layer"] = "0-1".into();
        serde_json::from_value(value).unwrap()
    }
    #[test]
    fn trim_circle_and_signed_arc_preserves_unpicked_intervals() {
        let circle = entity(serde_json::json!({"type":"circle","center":[0,0],"radius":10}));
        let cutter = entity(serde_json::json!({"type":"line","p1":[0,-20],"p2":[0,20]}));
        let result = trim(&circle, &cutter, [10., 0.]).unwrap();
        assert_eq!(result.len(), 1);
        if let Entity::Arc {
            start_deg, end_deg, ..
        } = result[0]
        {
            assert!((end_deg - start_deg - 180.).abs() < 1e-8);
            assert!((start_deg - 90.).abs() < 1e-8);
        } else {
            panic!("expected arc");
        }
        let arc = entity(
            serde_json::json!({"type":"arc","center":[0,0],"radius":10,"start_deg":180,"end_deg":0}),
        );
        let result = trim(&arc, &cutter, [10., 0.]).unwrap();
        if let Entity::Arc {
            start_deg, end_deg, ..
        } = result[0]
        {
            assert_eq!(start_deg, 180.);
            assert!((end_deg - 90.).abs() < 1e-8);
        } else {
            panic!("expected arc");
        }
    }
    #[test]
    fn trim_line_interior_creates_two_parts() {
        let line = entity(serde_json::json!({"type":"line","p1":[-20,0],"p2":[20,0]}));
        let circle = entity(serde_json::json!({"type":"circle","center":[0,0],"radius":10}));
        let result = trim(&line, &circle, [0., 0.]).unwrap();
        assert_eq!(result.len(), 2);
        assert!(matches!(
            result[0],
            Entity::Line {
                p1: [-20., 0.],
                p2: [-10., 0.],
                ..
            }
        ));
    }
    #[test]
    fn extension_uses_nearest_forward_hit_and_rejects_behind() {
        let line = entity(serde_json::json!({"type":"line","p1":[0,0],"p2":[5,0]}));
        let circle = entity(serde_json::json!({"type":"circle","center":[10,0],"radius":2}));
        assert!(matches!(
            extend(&line, &circle, [5., 0.]).unwrap(),
            Entity::Line { p2: [8., 0.], .. }
        ));
        assert!(extend(&line, &circle, [0., 0.]).is_err());
    }
    #[test]
    fn offsets_curves_and_rejects_collapsed_radius() {
        let circle = entity(serde_json::json!({"type":"circle","center":[0,0],"radius":10}));
        assert!(matches!(
            offset_entity(&circle, 5.).unwrap(),
            Entity::Circle { radius: 15., .. }
        ));
        assert!(offset_entity(&circle, -10.).is_err());
    }

    #[test]
    fn full_negative_arc_trim_and_closed_path_offset() {
        let arc = entity(
            serde_json::json!({"type":"arc","center":[0,0],"radius":10,"start_deg":360,"end_deg":0}),
        );
        let cutter = entity(serde_json::json!({"type":"line","p1":[0,-20],"p2":[0,20]}));
        let result = trim(&arc, &cutter, [10., 0.]).unwrap();
        assert!(
            matches!(result[0],Entity::Arc{start_deg,end_deg,..} if (end_deg-start_deg+180.).abs()<1e-8)
        );
        let points = offset_polyline(
            &[[0., 0.], [10., 0.], [10., 10.], [0., 10.], [0., 0.]],
            true,
            1.,
        )
        .unwrap();
        assert_eq!(points[0], points[4]);
        assert!(!polyline_self_intersects(&points, true));
    }

    #[test]
    fn snap_curves_share_intersection_and_nearest_geometry() {
        let circle = entity(serde_json::json!({"type":"circle","center":[0,0],"radius":10}));
        let line = entity(serde_json::json!({"type":"line","p1":[-20,0],"p2":[20,0]}));
        let mut project = cad_model::load_project(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small"),
        )
        .unwrap();
        project.drawings[0].entities = vec![circle, line]
            .into_iter()
            .enumerate()
            .map(|(i, entity)| EntityRecord {
                line: i + 1,
                entity,
            })
            .collect();
        let index = SnapIndex::build(&project, "plan_1f").unwrap();
        let snap = index
            .query(
                [10.1, 0.1],
                1.,
                &[SnapKind::Intersection, SnapKind::Nearest],
            )
            .unwrap();
        assert_eq!(snap.kind, SnapKind::Intersection);
        assert_eq!(snap.point, [10., 0.]);
    }
}
