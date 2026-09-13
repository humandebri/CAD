//! Dimension references and evaluated drawing primitives share one coordinate scope.

use crate::{Entity, EntityId, EntityRecord, Point, ProjectSource};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DimensionFeature {
    Start,
    End,
    Center,
    Vertex,
    Radius,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DimensionAnchor {
    Fixed {
        point: Point,
    },
    Entity {
        entity_id: EntityId,
        feature: DimensionFeature,
        #[serde(default)]
        index: Option<usize>,
    },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DimensionMeasurement {
    Aligned {
        first: DimensionAnchor,
        second: DimensionAnchor,
    },
    Horizontal {
        first: DimensionAnchor,
        second: DimensionAnchor,
    },
    Vertical {
        first: DimensionAnchor,
        second: DimensionAnchor,
    },
    Radius {
        center: DimensionAnchor,
        rim: DimensionAnchor,
    },
    Diameter {
        center: DimensionAnchor,
        rim: DimensionAnchor,
    },
    Angle {
        vertex: DimensionAnchor,
        first: DimensionAnchor,
        second: DimensionAnchor,
    },
    AngleLines {
        first_start: DimensionAnchor,
        first_end: DimensionAnchor,
        second_start: DimensionAnchor,
        second_end: DimensionAnchor,
    },
}

pub fn dimension_anchors_mut(entity: &mut Entity) -> Vec<&mut DimensionAnchor> {
    match entity {
        Entity::Dimension {
            measurement: Some(measurement),
            ..
        } => match measurement.as_mut() {
            DimensionMeasurement::Aligned { first, second }
            | DimensionMeasurement::Horizontal { first, second }
            | DimensionMeasurement::Vertical { first, second } => vec![first, second],
            DimensionMeasurement::Radius { center, rim }
            | DimensionMeasurement::Diameter { center, rim } => vec![center, rim],
            DimensionMeasurement::Angle {
                vertex,
                first,
                second,
            } => vec![vertex, first, second],
            DimensionMeasurement::AngleLines {
                first_start,
                first_end,
                second_start,
                second_end,
            } => vec![first_start, first_end, second_start, second_end],
        },
        _ => Vec::new(),
    }
}

pub fn remap_dimension_references(entity: &mut Entity, mapping: &BTreeMap<EntityId, EntityId>) {
    for anchor in dimension_anchors_mut(entity) {
        if let DimensionAnchor::Entity { entity_id, .. } = anchor
            && let Some(replacement) = mapping.get(entity_id)
        {
            *entity_id = replacement.clone();
        }
    }
}

pub fn transform_dimension_fixed_anchors(entity: &mut Entity, transform: &impl Fn(Point) -> Point) {
    for anchor in dimension_anchors_mut(entity) {
        if let DimensionAnchor::Fixed { point } = anchor {
            *point = transform(*point);
        }
    }
}

pub fn dimension_scope<'a>(
    project: &'a ProjectSource,
    entity: &Entity,
) -> Result<&'a [EntityRecord], String> {
    let mut scopes = project
        .drawings
        .iter()
        .map(|d| d.entities.as_slice())
        .chain(project.blocks.values().map(|b| b.entities.as_slice()))
        .filter(|records| records.iter().any(|r| r.entity.id() == entity.id()));
    let scope = scopes.next().ok_or_else(|| {
        format!(
            "dimension {} is not in a drawing or block",
            entity.id().as_str()
        )
    })?;
    if scopes.next().is_some() {
        return Err("dimension ID occurs in multiple scopes".to_owned());
    }
    Ok(scope)
}

pub fn resolve_dimension_anchor(
    anchor: &DimensionAnchor,
    scope: &[EntityRecord],
) -> Result<Point, String> {
    let point = match anchor {
        DimensionAnchor::Fixed { point } => *point,
        DimensionAnchor::Entity {
            entity_id,
            feature,
            index,
        } => {
            let entity = &scope
                .iter()
                .find(|r| r.entity.id() == entity_id)
                .ok_or_else(|| {
                    format!(
                        "measurement: missing entity {} in the same scope",
                        entity_id.as_str()
                    )
                })?
                .entity;
            match (entity, feature) {
                (Entity::Line { p1, .. }, DimensionFeature::Start) => *p1,
                (Entity::Line { p2, .. }, DimensionFeature::End) => *p2,
                (Entity::Polyline { points, .. }, DimensionFeature::Vertex) => *points
                    .get(index.ok_or("measurement: vertex requires index")?)
                    .ok_or("measurement: vertex index is out of range")?,
                (
                    Entity::Circle { center, .. } | Entity::Arc { center, .. },
                    DimensionFeature::Center,
                ) => *center,
                (
                    Entity::Circle { center, radius, .. } | Entity::Arc { center, radius, .. },
                    DimensionFeature::Radius,
                ) => [center[0] + radius, center[1]],
                (
                    Entity::Arc {
                        center,
                        radius,
                        start_deg,
                        ..
                    },
                    DimensionFeature::Start,
                ) => radial(*center, *radius, *start_deg),
                (
                    Entity::Arc {
                        center,
                        radius,
                        end_deg,
                        ..
                    },
                    DimensionFeature::End,
                ) => radial(*center, *radius, *end_deg),
                _ => {
                    return Err(format!(
                        "measurement: feature {feature:?} is invalid for {}",
                        entity_id.as_str()
                    ));
                }
            }
        }
    };
    if !point.iter().all(|v| v.is_finite()) {
        return Err("measurement: non-finite point".to_owned());
    }
    Ok(point)
}

pub fn detach_dimension(project: &ProjectSource, entity: &mut Entity) -> Result<(), String> {
    let scope = dimension_scope(project, entity)?;
    let mut replacement = entity.clone();
    for anchor in dimension_anchors_mut(&mut replacement) {
        *anchor = DimensionAnchor::Fixed {
            point: resolve_dimension_anchor(anchor, scope)?,
        };
    }
    *entity = replacement;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct DimensionEvaluation {
    pub measured: f64,
    pub label: String,
    pub lines: Vec<(Point, Point)>,
    pub arc: Option<(Point, f64, f64, f64)>,
    pub arrows: Vec<(Point, Point)>,
    pub text_at: Point,
}

fn radial(center: Point, radius: f64, angle: f64) -> Point {
    let (sin, cos) = angle.to_radians().sin_cos();
    [center[0] + radius * cos, center[1] + radius * sin]
}
fn distance(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

pub fn evaluate_dimension(
    project: &ProjectSource,
    entity: &Entity,
) -> Result<DimensionEvaluation, String> {
    let Entity::Dimension {
        style,
        measurement,
        p1,
        p2,
        offset,
        value,
        ..
    } = entity
    else {
        return Err("not a dimension".to_owned());
    };
    let measurement = measurement.as_deref();
    let style = project
        .styles
        .dimension_styles
        .get(style)
        .ok_or("dimension style is undefined")?;
    if !style.arrow_size.is_finite()
        || style.arrow_size <= 0.0
        || !style.extension_gap.is_finite()
        || style.extension_gap < 0.0
    {
        return Err("dimension style requires a positive finite arrow_size and nonnegative finite extension_gap".to_owned());
    }
    if !offset.is_finite() {
        return Err("dimension offset is non-finite".to_owned());
    }
    let scope = if measurement.is_some() {
        dimension_scope(project, entity)?
    } else {
        &[]
    };
    let resolve = |anchor: &DimensionAnchor| resolve_dimension_anchor(anchor, scope);
    let resolved_angle;
    let measurement = if let Some(DimensionMeasurement::AngleLines {
        first_start,
        first_end,
        second_start,
        second_end,
    }) = measurement
    {
        let a = resolve(first_start)?;
        let b = resolve(first_end)?;
        let c = resolve(second_start)?;
        let d = resolve(second_end)?;
        let u = [b[0] - a[0], b[1] - a[1]];
        let v = [d[0] - c[0], d[1] - c[1]];
        let cross = u[0] * v[1] - u[1] * v[0];
        if cross.abs() <= 1e-12 * u[0].hypot(u[1]) * v[0].hypot(v[1])
            || distance(a, b) <= 1e-9
            || distance(c, d) <= 1e-9
        {
            return Err("angle_lines requires two nonparallel nonzero lines".to_owned());
        }
        let t = ((c[0] - a[0]) * v[1] - (c[1] - a[1]) * v[0]) / cross;
        let vertex = [a[0] + t * u[0], a[1] + t * u[1]];
        resolved_angle = Some(DimensionMeasurement::Angle {
            vertex: DimensionAnchor::Fixed { point: vertex },
            first: DimensionAnchor::Fixed {
                point: [vertex[0] + u[0], vertex[1] + u[1]],
            },
            second: DimensionAnchor::Fixed {
                point: [vertex[0] + v[0], vertex[1] + v[1]],
            },
        });
        resolved_angle.as_ref()
    } else {
        measurement
    };
    let mut result = DimensionEvaluation {
        measured: 0.0,
        label: String::new(),
        lines: Vec::new(),
        arc: None,
        arrows: Vec::new(),
        text_at: [0.0, 0.0],
    };
    let mut prefix = "";
    let mut unit = style.unit.as_str();
    match measurement {
        Some(DimensionMeasurement::Angle {
            vertex,
            first,
            second,
        }) => {
            let center = resolve(vertex)?;
            let a = resolve(first)?;
            let b = resolve(second)?;
            if distance(center, a) <= 1e-9 || distance(center, b) <= 1e-9 || offset.abs() <= 1e-9 {
                return Err(
                    "angle requires two nonzero rays and a nonzero offset radius".to_owned(),
                );
            }
            let start = (a[1] - center[1]).atan2(a[0] - center[0]).to_degrees();
            let sweep =
                ((b[1] - center[1]).atan2(b[0] - center[0]).to_degrees() - start).rem_euclid(360.0);
            if sweep <= 1e-9 {
                return Err("angle rays coincide".to_owned());
            }
            result.measured = sweep;
            let radius = offset.abs();
            let d1 = radial(center, radius, start);
            let d2 = radial(center, radius, start + sweep);
            result.lines = vec![(center, d1), (center, d2)];
            result.arc = Some((center, radius, start, start + sweep));
            result.arrows = vec![
                (d1, radial(center, radius, start + 1.0)),
                (d2, radial(center, radius, start + sweep - 1.0)),
            ];
            result.text_at = radial(center, radius, start + sweep / 2.0);
            unit = "°";
        }
        Some(
            DimensionMeasurement::Radius { center, rim }
            | DimensionMeasurement::Diameter { center, rim },
        ) => {
            let center = resolve(center)?;
            let rim = resolve(rim)?;
            let radius = distance(center, rim);
            if radius <= 1e-9 {
                return Err("radius dimension requires distinct center and rim".to_owned());
            }
            let diameter = matches!(measurement, Some(DimensionMeasurement::Diameter { .. }));
            let start = if diameter {
                [2.0 * center[0] - rim[0], 2.0 * center[1] - rim[1]]
            } else {
                center
            };
            result.measured = radius * if diameter { 2.0 } else { 1.0 };
            prefix = if diameter { "φ" } else { "R" };
            result.lines.push((start, rim));
            result.arrows.push((rim, start));
            if diameter {
                result.arrows.push((start, rim));
            }
            result.text_at = [
                (start[0] + rim[0]) / 2.0,
                (start[1] + rim[1]) / 2.0 + offset,
            ];
        }
        _ => {
            let (a, b) = match measurement {
                Some(
                    DimensionMeasurement::Aligned { first, second }
                    | DimensionMeasurement::Horizontal { first, second }
                    | DimensionMeasurement::Vertical { first, second },
                ) => (resolve(first)?, resolve(second)?),
                _ => (*p1, *p2),
            };
            if !a.iter().chain(b.iter()).all(|v| v.is_finite()) {
                return Err("dimension points are non-finite".to_owned());
            }
            let (d1, d2) = match measurement {
                Some(DimensionMeasurement::Horizontal { .. }) => {
                    result.measured = (b[0] - a[0]).abs();
                    ([a[0], a[1] + offset], [b[0], a[1] + offset])
                }
                Some(DimensionMeasurement::Vertical { .. }) => {
                    result.measured = (b[1] - a[1]).abs();
                    ([a[0] + offset, a[1]], [a[0] + offset, b[1]])
                }
                _ => {
                    result.measured = distance(a, b);
                    crate::dimension_offset_segment(a, b, *offset)
                        .ok_or("dimension points coincide")?
                }
            };
            if result.measured <= 1e-9 {
                return Err("dimension measurement is zero".to_owned());
            }
            for (source, target) in [(a, d1), (b, d2)] {
                let length = distance(source, target);
                if length > 1e-9 {
                    let gap = style.extension_gap.min(length);
                    result.lines.push((
                        [
                            source[0] + (target[0] - source[0]) * gap / length,
                            source[1] + (target[1] - source[1]) * gap / length,
                        ],
                        target,
                    ));
                }
            }
            result.lines.push((d1, d2));
            result.arrows = vec![(d1, d2), (d2, d1)];
            result.text_at = [(d1[0] + d2[0]) / 2.0, (d1[1] + d2[1]) / 2.0];
        }
    }
    if !result.measured.is_finite()
        || !result.text_at.iter().all(|value| value.is_finite())
        || result
            .lines
            .iter()
            .any(|(a, b)| !a.iter().chain(b.iter()).all(|value| value.is_finite()))
    {
        return Err("dimension evaluation exceeds the finite coordinate range".to_owned());
    }
    result.label = value.clone().unwrap_or_else(|| {
        format!(
            "{prefix}{:.*} {unit}",
            usize::from(style.precision),
            result.measured
        )
    });
    Ok(result)
}

/// Expands dimensions into the same primitives for screen, print and interchange.
pub fn dimension_primitives(
    project: &ProjectSource,
    entity: &Entity,
) -> Result<Vec<Entity>, String> {
    let evaluated = evaluate_dimension(project, entity)?;
    let Entity::Dimension {
        schema_version,
        id,
        layer,
        pen,
        style,
        text_rotation_deg,
        text_mirror_y,
        ..
    } = entity
    else {
        unreachable!()
    };
    let style = project
        .styles
        .dimension_styles
        .get(style)
        .ok_or("dimension style is undefined")?;
    let line = |p1, p2| Entity::Line {
        schema_version: schema_version.clone(),
        id: id.clone(),
        layer: layer.clone(),
        pen: pen.clone(),
        p1,
        p2,
    };
    let mut primitives = evaluated
        .lines
        .into_iter()
        .map(|(a, b)| line(a, b))
        .collect::<Vec<_>>();
    if let Some((center, radius, start_deg, end_deg)) = evaluated.arc {
        primitives.push(Entity::Arc {
            schema_version: schema_version.clone(),
            id: id.clone(),
            layer: layer.clone(),
            pen: pen.clone(),
            center,
            radius,
            start_deg,
            end_deg,
        });
    }
    for (tip, toward) in evaluated.arrows {
        let length = distance(tip, toward);
        if length <= 1e-9 {
            continue;
        }
        let ux = (toward[0] - tip[0]) / length;
        let uy = (toward[1] - tip[1]) / length;
        for sign in [-1.0, 1.0] {
            primitives.push(line(
                tip,
                [
                    tip[0] + style.arrow_size * (ux - sign * uy * 0.35),
                    tip[1] + style.arrow_size * (uy + sign * ux * 0.35),
                ],
            ));
        }
    }
    let text_style = project
        .styles
        .text_styles
        .get(&style.text_style)
        .ok_or("dimension text style is undefined")?;
    let text_at = dimension_text_anchor(
        evaluated.text_at,
        &evaluated.label,
        text_style,
        *text_rotation_deg,
    );
    primitives.push(Entity::Text {
        schema_version: schema_version.clone(),
        id: id.clone(),
        layer: layer.clone(),
        pen: pen.clone(),
        style: style.text_style.clone(),
        at: text_at,
        rotation_deg: *text_rotation_deg,
        mirror_y: *text_mirror_y,
        value: evaluated.label,
    });
    Ok(primitives)
}

pub fn dimension_text_anchor(
    center: Point,
    label: &str,
    style: &crate::TextStyleDef,
    rotation_deg: f64,
) -> Point {
    let count = label.chars().count();
    let length =
        (count as f64 * style.width + count.saturating_sub(1) as f64 * style.spacing).max(0.0);
    let shift = match style.align {
        crate::TextAlign::Left => -length / 2.0,
        crate::TextAlign::Center => 0.0,
        crate::TextAlign::Right => length / 2.0,
    };
    let (sin, cos) = rotation_deg.to_radians().sin_cos();
    [center[0] + shift * cos, center[1] + shift * sin]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloning_remaps_entity_refs_and_moves_only_fixed_anchors() {
        let old = EntityId::parse("ent_01JZ0000000000000000000000").unwrap();
        let new = EntityId::parse("ent_01JZ0000000000000000000001").unwrap();
        let project = fixture(
            serde_json::json!({"kind":"aligned","first":{"kind":"entity","entity_id":old.as_str(),"feature":"start"},"second":{"kind":"fixed","point":[3,4]}}),
        );
        let mut dimension = project.drawings[0].entities.last().unwrap().entity.clone();
        remap_dimension_references(&mut dimension, &BTreeMap::from([(old, new.clone())]));
        let anchors = dimension_anchors_mut(&mut dimension);
        assert!(matches!(&anchors[0],DimensionAnchor::Entity {entity_id,..} if entity_id==&new));
        transform_dimension_fixed_anchors(&mut dimension, &|p| [p[0] + 300.0, p[1]]);
        let anchors = dimension_anchors_mut(&mut dimension);
        assert!(matches!(&anchors[1],DimensionAnchor::Fixed {point} if point==&[303.0,4.0]));
    }

    #[test]
    fn acceptance_drawing_dimensions_follow_room_width_change() {
        let mut project = crate::load_project(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/cad-acceptance"),
        )
        .unwrap();
        let dimensions = project.drawings[0]
            .entities
            .iter()
            .filter(|record| matches!(record.entity, Entity::Dimension { .. }))
            .map(|record| {
                evaluate_dimension(&project, &record.entity)
                    .unwrap()
                    .measured
            })
            .collect::<Vec<_>>();
        for (actual, expected) in dimensions
            .iter()
            .zip([5000.0, 4000.0, 1500.0, 60.0, 1200.0, 500.0])
        {
            assert!((actual - expected).abs() < 1e-6);
        }
        assert_eq!(dimensions.len(), 6);
        if let Entity::Polyline { points, .. } = &mut project.drawings[0].entities[0].entity {
            points[1][0] += 300.0;
            points[2][0] += 300.0;
        }
        let dimensions = project.drawings[0]
            .entities
            .iter()
            .filter(|record| matches!(record.entity, Entity::Dimension { .. }))
            .map(|record| {
                evaluate_dimension(&project, &record.entity)
                    .unwrap()
                    .measured
            })
            .collect::<Vec<_>>();
        assert_eq!(dimensions[0], 5300.0);
        assert_eq!(dimensions[1], 4000.0);
    }

    fn fixture(measurement: serde_json::Value) -> ProjectSource {
        let mut project = crate::load_project(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small"),
        )
        .unwrap();
        let dimension: Entity = serde_json::from_value(serde_json::json!({
            "schema_version":crate::CURRENT_SCHEMA_VERSION,"id":"ent_01JZ0000000000000000000099","type":"dimension","layer":"0-1","style":"dim_100",
            "p1":[0,0],"p2":[1,0],"offset":100,"measurement":measurement,"value":null
        })).unwrap();
        project.drawings[0].entities.push(EntityRecord {
            line: 99,
            entity: dimension,
        });
        project
    }

    fn fixed(point: Point) -> serde_json::Value {
        serde_json::json!({"kind":"fixed","point":point})
    }

    #[test]
    fn evaluates_all_dimension_kinds_from_common_primitives() {
        for (kind, expected) in [("aligned", 5.0), ("horizontal", 3.0), ("vertical", 4.0)] {
            let project = fixture(
                serde_json::json!({"kind":kind,"first":fixed([0.0,0.0]),"second":fixed([3.0,4.0])}),
            );
            let entity = &project.drawings[0].entities.last().unwrap().entity;
            assert_eq!(
                evaluate_dimension(&project, entity).unwrap().measured,
                expected
            );
            assert!(!dimension_primitives(&project, entity).unwrap().is_empty());
        }
        for (kind, expected) in [("radius", 5.0), ("diameter", 10.0)] {
            let project = fixture(
                serde_json::json!({"kind":kind,"center":fixed([0.0,0.0]),"rim":fixed([3.0,4.0])}),
            );
            assert_eq!(
                evaluate_dimension(
                    &project,
                    &project.drawings[0].entities.last().unwrap().entity
                )
                .unwrap()
                .measured,
                expected
            );
        }
        let project = fixture(
            serde_json::json!({"kind":"angle","vertex":fixed([0.0,0.0]),"first":fixed([1.0,0.0]),"second":fixed([0.0,1.0])}),
        );
        let evaluation = evaluate_dimension(
            &project,
            &project.drawings[0].entities.last().unwrap().entity,
        )
        .unwrap();
        assert_eq!(evaluation.measured, 90.0);
        assert_eq!(evaluation.arc, Some(([0.0, 0.0], 100.0, 0.0, 90.0)));
    }

    #[test]
    fn line_dimension_follows_edits_and_detachment_freezes_current_coordinates() {
        let reference = |feature| serde_json::json!({"kind":"entity","entity_id":"ent_01JZ0000000000000000000000","feature":feature});
        let mut project = fixture(
            serde_json::json!({"kind":"horizontal","first":reference("start"),"second":reference("end")}),
        );
        let index = project.drawings[0].entities.len() - 1;
        let before = evaluate_dimension(&project, &project.drawings[0].entities[index].entity)
            .unwrap()
            .measured;
        if let Entity::Line { p2, .. } = &mut project.drawings[0].entities[0].entity {
            p2[0] += 300.0;
        } else {
            panic!("fixture line");
        }
        let after = evaluate_dimension(&project, &project.drawings[0].entities[index].entity)
            .unwrap()
            .measured;
        assert_eq!(after, before + 300.0);
        let mut detached = project.drawings[0].entities[index].entity.clone();
        detach_dimension(&project, &mut detached).unwrap();
        project.drawings[0].entities[index].entity = detached;
        project.drawings[0].entities.remove(0);
        assert_eq!(
            evaluate_dimension(
                &project,
                &project.drawings[0].entities.last().unwrap().entity
            )
            .unwrap()
            .measured,
            after
        );
    }

    #[test]
    fn dangling_and_cross_scope_references_fail_without_repair() {
        let project = fixture(
            serde_json::json!({"kind":"aligned","first":{"kind":"entity","entity_id":"ent_01JZ0000000000000000000098","feature":"start"},"second":fixed([3.0,4.0])}),
        );
        let entity = &project.drawings[0].entities.last().unwrap().entity;
        assert!(
            evaluate_dimension(&project, entity)
                .unwrap_err()
                .contains("missing entity")
        );
        let mut detached = entity.clone();
        assert!(detach_dimension(&project, &mut detached).is_err());
        assert_eq!(&detached, entity);
    }

    #[test]
    fn angular_line_measurement_computes_current_intersection() {
        let project = fixture(
            serde_json::json!({"kind":"angle_lines","first_start":fixed([2.0,0.0]),"first_end":fixed([12.0,0.0]),"second_start":fixed([5.0,3.0]),"second_end":fixed([5.0,10.0])}),
        );
        let evaluation = evaluate_dimension(
            &project,
            &project.drawings[0].entities.last().unwrap().entity,
        )
        .unwrap();
        assert_eq!(evaluation.measured, 90.0);
        assert_eq!(evaluation.arc.unwrap().0, [5.0, 0.0]);
    }
}
