//! Closed boundary extraction. This never extends edges or heals geometric gaps.
use super::*;

/// Resolve the innermost closed region under `point`, including its immediate holes.
pub fn hatch_region(
    project: &ProjectSource,
    drawing: &str,
    point: Point,
) -> EditResult<Vec<Vec<Point>>> {
    if !point.iter().all(|coordinate| coordinate.is_finite()) {
        return Err(EditError::InvalidEntity(
            "hatch pick point must be finite".into(),
        ));
    }
    let source = project
        .drawings
        .iter()
        .find(|source| source.name == drawing)
        .ok_or_else(|| EditError::DrawingNotFound(drawing.into()))?;
    let entities = source
        .entities
        .iter()
        .filter(|record| layer_visible(project, record.entity.layer()))
        .map(|record| &record.entity)
        .collect::<Vec<_>>();
    region_from_entities(&entities, point)
}

fn region_from_entities(entities: &[&Entity], point: Point) -> EditResult<Vec<Vec<Point>>> {
    let mut loops = Vec::new();
    let mut paths = Vec::new();
    for entity in entities {
        match entity {
            Entity::Polyline {
                points,
                closed: true,
                ..
            } if points.len() >= 3 => loops.push(points.clone()),
            Entity::Circle { center, radius, .. } => {
                loops.push(arc_points(*center, *radius, 0.0, 360.0)?);
            }
            Entity::Arc {
                center,
                radius,
                start_deg,
                end_deg,
                ..
            } => {
                let points = arc_points(*center, *radius, *start_deg, *end_deg)?;
                if (end_deg - start_deg).abs() >= 360.0 - 1e-10 {
                    loops.push(points);
                } else {
                    paths.push(points);
                }
            }
            Entity::Line { p1, p2, .. } => paths.push(vec![*p1, *p2]),
            Entity::Polyline {
                points,
                closed: false,
                ..
            } if points.len() >= 2 => paths.push(points.clone()),
            _ => {}
        }
    }
    loops.extend(connected_loops(&paths));
    loops.retain(|boundary| boundary.len() >= 3 && area(boundary).abs() > 1e-12);
    let areas = loops
        .iter()
        .map(|boundary| area(boundary).abs())
        .collect::<Vec<_>>();
    let bounds = loops
        .iter()
        .map(|boundary| cad_model::BBox::from_points(boundary).unwrap())
        .collect::<Vec<_>>();
    let selected = loops.iter().enumerate().filter(|(_, boundary)| contains(boundary, point))
        .min_by(|(a, _), (b, _)| areas[*a].total_cmp(&areas[*b])).map(|(index, _)| index)
        .ok_or_else(|| EditError::InvalidEntity("hatch boundary: no closed region at the picked point; close gaps or choose a closed boundary".into()))?;
    let outer = &loops[selected];
    let mut result = vec![outer.clone()];
    for (index, boundary) in loops.iter().enumerate() {
        if index == selected
            || (0..2).any(|axis| {
                bounds[index].min[axis] < bounds[selected].min[axis]
                    || bounds[index].max[axis] > bounds[selected].max[axis]
            })
            || !boundary.iter().all(|point| contains(outer, *point))
        {
            continue;
        }
        let has_parent = loops.iter().enumerate().any(|(other_index, other)| {
            other_index != selected
                && other_index != index
                && areas[other_index] > areas[index]
                && contains(outer, other[0])
                && contains(other, boundary[0])
        });
        if !has_parent {
            result.push(boundary.clone());
        }
    }
    Ok(result)
}

fn connected_loops(paths: &[Vec<Point>]) -> Vec<Vec<Point>> {
    // Walking each directed edge with its face on the left also handles shared room walls.
    // Only machine-roundoff equality is accepted; no distance-based gap joining occurs.
    let directed = paths
        .iter()
        .flat_map(|path| [path.clone(), path.iter().rev().copied().collect()])
        .collect::<Vec<Vec<Point>>>();
    let starts = RTree::bulk_load(
        directed
            .iter()
            .enumerate()
            .map(|(i, path)| rstar::primitives::GeomWithData::new(path[0], i))
            .collect(),
    );
    let margin: Point = std::array::from_fn(|axis| {
        directed
            .iter()
            .map(|path| path[0][axis].abs())
            .fold(1.0_f64, f64::max)
            * f64::EPSILON
            * 16.0
    });
    let mut visited = BTreeSet::new();
    let mut loops = Vec::new();
    for first in 0..directed.len() {
        if visited.contains(&first) {
            continue;
        }
        let mut edge = first;
        let mut boundary = Vec::new();
        let mut completed = false;
        loop {
            if !visited.insert(edge) {
                completed = edge == first;
                break;
            }
            let path = &directed[edge];
            boundary.extend(path.iter().take(path.len() - 1).copied());
            let end = *path.last().unwrap();
            let previous = path[path.len() - 2];
            let reverse_angle = (previous[1] - end[1]).atan2(previous[0] - end[0]);
            let envelope = AABB::from_corners(
                [end[0] - margin[0], end[1] - margin[1]],
                [end[0] + margin[0], end[1] + margin[1]],
            );
            let mut adjacent = starts
                .locate_in_envelope_intersecting(envelope)
                .map(|entry| entry.data)
                .collect::<Vec<_>>();
            adjacent.sort_unstable();
            let next = adjacent
                .into_iter()
                .map(|index| (index, &directed[index]))
                .filter(|(index, path)| *index != (edge ^ 1) && same_endpoint(path[0], end))
                .min_by(|(_, a), (_, b)| {
                    let clockwise = |path: &[Point]| {
                        (reverse_angle - (path[1][1] - end[1]).atan2(path[1][0] - end[0]))
                            .rem_euclid(std::f64::consts::TAU)
                    };
                    clockwise(a).total_cmp(&clockwise(b))
                })
                .map(|(index, _)| index);
            let Some(next) = next else {
                break;
            };
            edge = next;
        }
        if completed && boundary.len() >= 3 && area(&boundary) > 1e-12 {
            loops.push(boundary);
        }
    }
    loops
}

fn same_endpoint(a: Point, b: Point) -> bool {
    (0..2).all(|axis| {
        (a[axis] - b[axis]).abs() <= f64::EPSILON * 16.0 * a[axis].abs().max(b[axis].abs()).max(1.0)
    })
}

fn arc_points(center: Point, radius: f64, start: f64, end: f64) -> EditResult<Vec<Point>> {
    const CHORD_ERROR: f64 = 0.001;
    const MAX_SEGMENTS: f64 = 20_000.0;
    if !radius.is_finite()
        || radius <= 0.0
        || !start.is_finite()
        || !end.is_finite()
        || !center.iter().all(|value| value.is_finite())
    {
        return Err(EditError::InvalidEntity(
            "hatch boundary: invalid circle or arc geometry".into(),
        ));
    }
    let sweep = (end - start).clamp(-360.0, 360.0);
    let step_radians = (2.0 * (1.0 - CHORD_ERROR / radius).clamp(-1.0, 1.0).acos())
        .min(std::f64::consts::FRAC_PI_2);
    let steps = (sweep.to_radians().abs() / step_radians).ceil().max(1.0);
    if !steps.is_finite() || steps > MAX_SEGMENTS {
        return Err(EditError::InvalidEntity(
            "hatch boundary: circle/arc exceeds 20000 segments at 0.001-unit chord tolerance"
                .into(),
        ));
    }
    let steps = steps as usize;
    Ok((0..=steps)
        .map(|index| {
            let angle = (start + sweep * index as f64 / steps as f64).to_radians();
            [
                center[0] + radius * angle.cos(),
                center[1] + radius * angle.sin(),
            ]
        })
        .collect())
}

fn area(boundary: &[Point]) -> f64 {
    boundary
        .iter()
        .zip(boundary.iter().cycle().skip(1))
        .take(boundary.len())
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum::<f64>()
        / 2.0
}

fn contains(boundary: &[Point], point: Point) -> bool {
    let mut inside = false;
    for (a, b) in boundary
        .iter()
        .zip(boundary.iter().cycle().skip(1))
        .take(boundary.len())
    {
        if (a[1] > point[1]) != (b[1] > point[1])
            && point[0] < (b[0] - a[0]) * (point[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;
    fn polygon(points: Vec<Point>, id: usize) -> Entity {
        serde_json::from_value(serde_json::json!({"schema_version":cad_model::CURRENT_SCHEMA_VERSION,"type":"polyline","id":format!("ent_01JZ00000000000000000000{id:02}"),"layer":"0-1","points":points,"closed":true})).unwrap()
    }
    #[test]
    fn region_keeps_holes_and_selects_innermost_boundary() {
        let outer = polygon(vec![[0., 0.], [10., 0.], [10., 10.], [0., 10.]], 0);
        let hole = polygon(vec![[3., 3.], [7., 3.], [7., 7.], [3., 7.]], 1);
        assert_eq!(
            region_from_entities(&[&outer, &hole], [1., 1.])
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            region_from_entities(&[&outer, &hole], [5., 5.])
                .unwrap()
                .len(),
            1
        );
        assert!(region_from_entities(&[&outer, &hole], [20., 20.]).is_err());
    }
    #[test]
    fn endpoint_joining_does_not_heal_gaps() {
        let polygon = polygon(vec![[0., 0.], [10., 0.], [10., 10.], [0., 10.]], 0);
        let mut value = serde_json::to_value(polygon).unwrap();
        value["closed"] = false.into();
        let open: Entity = serde_json::from_value(value).unwrap();
        assert!(region_from_entities(&[&open], [5., 5.]).is_err());
        assert!(!same_endpoint([0., 0.], [0.0001, 0.]));
    }

    #[test]
    fn shared_wall_produces_two_bounded_faces() {
        let paths = vec![
            vec![[0., 0.], [1., 0.]],
            vec![[1., 0.], [2., 0.]],
            vec![[2., 0.], [2., 1.]],
            vec![[2., 1.], [1., 1.]],
            vec![[1., 1.], [0., 1.]],
            vec![[0., 1.], [0., 0.]],
            vec![[1., 0.], [1., 1.]],
        ];
        let loops = connected_loops(&paths);
        assert_eq!(loops.len(), 2);
        assert!(
            loops
                .iter()
                .all(|boundary| (area(boundary) - 1.0).abs() < 1e-10)
        );
        let mut broken = paths;
        broken[0][0] = [0.001, 0.];
        let loops = connected_loops(&broken);
        assert!(!loops.iter().any(|boundary| contains(boundary, [0.5, 0.5])));
    }
}
