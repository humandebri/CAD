use crate::{Result, ToolkitError};
use cad_model::{Entity, Point, ProjectSource};
use geo::algorithm::unary_union;
use geo::{Area, Contains, Intersects, LineString, MultiPolygon, Polygon, Validation};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::PI;

#[path = "measure_regions.rs"]
mod regions;

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AreaMode {
    #[default]
    Sum,
    Union,
}

#[derive(Debug, Clone, Copy)]
pub struct MeasurementOptions {
    pub area_mode: AreaMode,
    /// Maximum chord deviation in drawing/model millimetres.
    pub curve_tolerance_mm: f64,
}
impl Default for MeasurementOptions {
    fn default() -> Self {
        Self {
            area_mode: AreaMode::Sum,
            curve_tolerance_mm: 0.1,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct DrawingArea {
    pub drawing: String,
    pub area_mm2: f64,
}

#[derive(Debug, Serialize)]
pub struct Measurement {
    pub drawing: String,
    pub entity_id: String,
    pub layer: String,
    pub length_mm: Option<f64>,
    pub area_mm2: Option<f64>,
    pub numerical_length_error_mm: Option<f64>,
    pub warnings: Vec<String>,
    pub supported_leaf_count: usize,
    pub unsupported_leaf_count: usize,
}
#[derive(Debug, Serialize)]
pub struct MeasurementReport {
    pub schema_version: String,
    pub length_unit: String,
    pub area_unit: String,
    pub measurements: Vec<Measurement>,
    pub total_length_mm: f64,
    pub total_area_mm2: f64,
    pub unsupported_count: usize,
    pub supported_leaf_count: usize,
    pub area_mode: AreaMode,
    pub additive_area_mm2: f64,
    pub area_by_drawing: Vec<DrawingArea>,
    pub curve_tolerance_mm: Option<f64>,
    /// Inscribed-curve area deficit summed before union. Floating point topology
    /// operations are not included; this is not a certified interval bound.
    pub area_approximation_error_estimate_mm2: f64,
}

pub fn measure_project(
    project: &ProjectSource,
    drawing: Option<&str>,
    ids: &[String],
) -> Result<MeasurementReport> {
    measure_project_with_options(project, drawing, ids, MeasurementOptions::default())
}

pub fn measure_project_with_options(
    project: &ProjectSource,
    drawing: Option<&str>,
    ids: &[String],
    options: MeasurementOptions,
) -> Result<MeasurementReport> {
    if !options.curve_tolerance_mm.is_finite() || options.curve_tolerance_mm <= 0.0 {
        return Err(ToolkitError::Invalid(
            "curve tolerance must be finite and positive".into(),
        ));
    }
    if drawing.is_some_and(|name| !project.drawings.iter().any(|d| d.name == name)) {
        return Err(ToolkitError::Invalid("drawing does not exist".into()));
    }
    let mut remaining: BTreeSet<_> = ids.iter().map(String::as_str).collect();
    let mut measurements = Vec::new();
    let mut areas: BTreeMap<String, Vec<MultiPolygon<f64>>> = BTreeMap::new();
    let mut budget = regions::Budget::default();
    let mut area_error = 0.0;
    for source in project
        .drawings
        .iter()
        .filter(|d| drawing.is_none_or(|name| d.name == name))
    {
        for record in &source.entities {
            if !ids.is_empty() && !remaining.contains(record.entity.id().as_str()) {
                continue;
            }
            remaining.remove(record.entity.id().as_str());
            let measured = regions::expand(project, &record.entity, options, &mut budget)?;
            if options.area_mode == AreaMode::Union {
                area_error += measured.area_error;
                areas
                    .entry(source.name.clone())
                    .or_default()
                    .extend(measured.regions);
            }
            measurements.push(Measurement {
                drawing: source.name.clone(), entity_id: record.entity.id().as_str().into(), layer: record.entity.layer().into(),
                length_mm: measured.length, area_mm2: measured.area, numerical_length_error_mm: measured.length_error,
                supported_leaf_count: measured.supported, unsupported_leaf_count: measured.unsupported,
                warnings: if measured.unsupported > 0 { vec![format!("{} leaf entities have no supported measured geometry and are excluded from totals (text, dimensions, points and curve solids).", measured.unsupported)] } else { Vec::new() },
            });
        }
    }
    if !remaining.is_empty() {
        return Err(ToolkitError::Invalid(format!(
            "selected entities were not found: {remaining:?}"
        )));
    }
    let additive_area_mm2: f64 = measurements.iter().filter_map(|m| m.area_mm2).sum();
    let area_by_drawing: Vec<_> = if options.area_mode == AreaMode::Union {
        areas
            .into_iter()
            .map(|(drawing, region)| DrawingArea {
                drawing,
                area_mm2: unary_union(&region).unsigned_area(),
            })
            .collect()
    } else {
        let mut sums = BTreeMap::<String, f64>::new();
        for item in &measurements {
            *sums.entry(item.drawing.clone()).or_default() += item.area_mm2.unwrap_or(0.0);
        }
        sums.into_iter()
            .map(|(drawing, area_mm2)| DrawingArea { drawing, area_mm2 })
            .collect()
    };
    let total_area_mm2 = area_by_drawing.iter().map(|d| d.area_mm2).sum();
    let total_length_mm = measurements.iter().filter_map(|m| m.length_mm).sum();
    if [
        total_area_mm2,
        additive_area_mm2,
        total_length_mm,
        area_error,
    ]
    .iter()
    .any(|n| !n.is_finite())
    {
        return Err(ToolkitError::Invalid(
            "measurement totals overflowed".into(),
        ));
    }
    Ok(MeasurementReport {
        schema_version: "cad-measure/1".into(),
        length_unit: "mm".into(),
        area_unit: "mm2".into(),
        total_length_mm,
        total_area_mm2,
        additive_area_mm2,
        area_by_drawing,
        unsupported_count: measurements.iter().map(|m| m.unsupported_leaf_count).sum(),
        supported_leaf_count: measurements.iter().map(|m| m.supported_leaf_count).sum(),
        area_mode: options.area_mode,
        curve_tolerance_mm: (options.area_mode == AreaMode::Union)
            .then_some(options.curve_tolerance_mm),
        area_approximation_error_estimate_mm2: area_error,
        measurements,
    })
}

type GeometryMeasurement = (Option<f64>, Option<f64>, Option<f64>);
fn geometry(entity: &Entity) -> Result<GeometryMeasurement> {
    let (length, area, error) = match entity {
        Entity::Line { p1, p2, .. } => (Some(distance(*p1, *p2)), None, None),
        Entity::Polyline { points, closed, .. } => {
            let area = if *closed {
                Some(loop_area(std::slice::from_ref(points))?)
            } else {
                None
            };
            (Some(path_length(points, *closed)), area, None)
        }
        Entity::Circle { radius, .. } => {
            (Some(2.0 * PI * radius), Some(PI * radius * radius), None)
        }
        Entity::Arc {
            radius,
            start_deg,
            end_deg,
            ..
        } => (
            Some(radius * (end_deg - start_deg).abs().to_radians()),
            None,
            None,
        ),
        Entity::Ellipse {
            radius_x,
            radius_y,
            start_deg,
            end_deg,
            ..
        } => {
            let start = start_deg.to_radians();
            let span = (end_deg - start_deg).to_radians();
            let speed =
                |t: f64| ((radius_x * t.sin()).powi(2) + (radius_y * t.cos()).powi(2)).sqrt();
            let (length, error) = integrate(
                &speed,
                start.min(start + span),
                start.max(start + span),
                1e-6,
                22,
            )?;
            (
                Some(length),
                if (span.abs() - 2.0 * PI).abs() < 1e-10 {
                    Some(PI * radius_x * radius_y)
                } else {
                    None
                },
                Some(error),
            )
        }
        Entity::Solid { points, .. } => (
            Some(path_length(points, true)),
            Some(loop_area(std::slice::from_ref(points))?),
            None,
        ),
        Entity::Hatch { loops, .. } => (
            Some(loops.iter().map(|points| path_length(points, true)).sum()),
            Some(loop_area(loops)?),
            None,
        ),
        _ => (None, None, None),
    };
    if length.is_some_and(|value| !value.is_finite())
        || area.is_some_and(|value| !value.is_finite())
    {
        return Err(ToolkitError::Invalid("measurement overflowed".into()));
    }
    Ok((length, area, error))
}
fn distance(a: Point, b: Point) -> f64 {
    (b[0] - a[0]).hypot(b[1] - a[1])
}
fn path_length(points: &[Point], closed: bool) -> f64 {
    points
        .windows(2)
        .map(|pair| distance(pair[0], pair[1]))
        .sum::<f64>()
        + if closed && points.len() > 1 {
            distance(points[0], *points.last().unwrap())
        } else {
            0.0
        }
}
/// Even-odd area, independent of winding or ordering. Crossing or touching
/// boundaries are rejected instead of producing an ambiguous area.
pub fn loop_area(loops: &[Vec<Point>]) -> Result<f64> {
    if loops.is_empty() {
        return Err(ToolkitError::Invalid(
            "area requires at least one boundary".into(),
        ));
    }
    let polygons = loops
        .iter()
        .map(|points| {
            let polygon = Polygon::new(
                LineString::from(points.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>()),
                Vec::new(),
            );
            polygon.check_validation().map_err(|error| {
                ToolkitError::Invalid(format!("invalid measured boundary: {error}"))
            })?;
            Ok(polygon)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut area = 0.0;
    for (i, polygon) in polygons.iter().enumerate() {
        let mut depth = 0;
        for (j, other) in polygons.iter().enumerate() {
            if i == j {
                continue;
            }
            if polygon.exterior().intersects(other.exterior()) {
                return Err(ToolkitError::Invalid(
                    "measured boundaries cross or touch".into(),
                ));
            }
            if other.contains(&geo::Point::from(polygon.exterior().0[0])) {
                depth += 1;
            }
        }
        area += polygon.unsigned_area() * if depth % 2 == 0 { 1.0 } else { -1.0 };
    }
    Ok(area)
}

// Adaptive Simpson quadrature; the returned estimate is the accumulated
// difference between refinements, not a certified interval bound.
fn integrate(
    f: &impl Fn(f64) -> f64,
    a: f64,
    b: f64,
    tolerance: f64,
    depth: usize,
) -> Result<(f64, f64)> {
    if a == b {
        return Ok((0.0, 0.0));
    }
    let mid = (a + b) / 2.0;
    let coarse = (b - a) / 6.0 * (f(a) + 4.0 * f(mid) + f(b));
    let left = (mid - a) / 6.0 * (f(a) + 4.0 * f((a + mid) / 2.0) + f(mid));
    let right = (b - mid) / 6.0 * (f(mid) + 4.0 * f((mid + b) / 2.0) + f(b));
    let error = (left + right - coarse).abs() / 15.0;
    if !error.is_finite() {
        return Err(ToolkitError::Invalid("ellipse integration overflow".into()));
    }
    if error <= tolerance {
        return Ok((left + right + (left + right - coarse) / 15.0, error));
    }
    if depth == 0 {
        return Err(ToolkitError::Invalid(
            "ellipse length did not converge".into(),
        ));
    }
    let (l, le) = integrate(f, a, mid, tolerance / 2.0, depth - 1)?;
    let (r, re) = integrate(f, mid, b, tolerance / 2.0, depth - 1)?;
    Ok((l + r, le + re))
}

impl MeasurementReport {
    pub fn csv(&self) -> String {
        fn cell(value: &str) -> String {
            format!("\"{}\"", value.replace('"', "\"\""))
        }
        let mut output =
            "record_type,drawing,entity_id,layer,length_mm,area_mm2,numerical_length_error_mm,warnings,area_mode,area_approximation_error_estimate_mm2\n"
                .to_owned();
        for item in &self.measurements {
            output.push_str(&format!(
                "entity,{},{},{},{},{},{},{},sum,\n",
                cell(&item.drawing),
                cell(&item.entity_id),
                cell(&item.layer),
                item.length_mm.map(|v| v.to_string()).unwrap_or_default(),
                item.area_mm2.map(|v| v.to_string()).unwrap_or_default(),
                item.numerical_length_error_mm
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                cell(&item.warnings.join("; "))
            ));
        }
        output.push_str(&format!(
            "aggregate,,,,{},{},,,{},{}\n",
            self.total_length_mm,
            self.total_area_mm2,
            if self.area_mode == AreaMode::Union {
                "union"
            } else {
                "sum"
            },
            self.area_approximation_error_estimate_mm2
        ));
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn rectangle(min: f64, max: f64) -> Vec<Point> {
        vec![[min, min], [max, min], [max, max], [min, max]]
    }
    #[test]
    fn holes_islands_and_winding_are_independent() {
        let mut outer = rectangle(0.0, 10.0);
        outer.reverse();
        assert_eq!(
            loop_area(&[rectangle(2.0, 8.0), rectangle(4.0, 6.0), outer]).unwrap(),
            68.0
        );
        assert!(loop_area(&[vec![[0.0, 0.0], [10.0, 10.0], [0.0, 10.0], [10.0, 0.0]]]).is_err());
        assert!(loop_area(&[rectangle(0.0, 10.0), rectangle(5.0, 12.0)]).is_err());
    }
    #[test]
    fn curved_measurements_match_circle_and_ellipse_reference() {
        let id = cad_model::EntityId::parse("ent_01JZ0000000000000000000000").unwrap();
        let entity = Entity::Ellipse {
            schema_version: "0.3".into(),
            id,
            layer: "0-1".into(),
            pen: None,
            center: [0.0, 0.0],
            radius_x: 5.0,
            radius_y: 3.0,
            rotation_deg: 45.0,
            start_deg: 0.0,
            end_deg: 360.0,
        };
        let (length, area, error) = geometry(&entity).unwrap();
        assert!((length.unwrap() - 25.5269988634).abs() < 1e-6);
        assert!((area.unwrap() - 15.0 * PI).abs() < 1e-10);
        assert!(error.unwrap() <= 1e-6);
        let (length, _) = integrate(&|_| 5.0, 0.0, 2.0 * PI, 1e-6, 10).unwrap();
        assert!((length - 10.0 * PI).abs() < 1e-10);
    }
    fn source() -> (tempfile::TempDir, ProjectSource) {
        let temp = tempfile::tempdir().unwrap();
        let created = cad_edit::create_project(&cad_edit::ProjectTemplateRequest {
            parent_dir: temp.path().to_string_lossy().into(),
            folder_name: "test".into(),
            project_name: "test".into(),
            drawing: "plan".into(),
            paper: "A4".into(),
            orientation: cad_model::SheetOrientation::Landscape,
            scale_denominator: 100,
        })
        .unwrap();
        let mut project =
            cad_model::load_project(std::path::Path::new(&created.project_path)).unwrap();
        project.drawings[0].entities.clear();
        (temp, project)
    }
    fn record(mut fields: serde_json::Value) -> cad_model::EntityRecord {
        fields["schema_version"] = "0.3".into();
        fields["id"] = format!("ent_{}", ulid::Ulid::new()).into();
        fields["layer"] = "0-1".into();
        cad_model::EntityRecord {
            line: 1,
            entity: serde_json::from_value(fields).unwrap(),
        }
    }
    fn block(
        project: &mut ProjectSource,
        id: &str,
        base: Point,
        entities: Vec<cad_model::EntityRecord>,
    ) {
        project.blocks.insert(
            id.into(),
            cad_model::BlockDefinition {
                id: id.into(),
                config: cad_model::BlockDefinitionConfig {
                    schema_version: "0.3".into(),
                    name: id.into(),
                    base_point: base,
                },
                entities,
            },
        );
    }
    fn union_options() -> MeasurementOptions {
        MeasurementOptions {
            area_mode: AreaMode::Union,
            curve_tolerance_mm: 0.001,
        }
    }
    #[test]
    fn nested_scaled_mirrored_blocks_measure_in_drawing_coordinates() {
        let (_temp, mut project) = source();
        let rectangle = record(
            serde_json::json!({"type":"polyline","closed":true,"points":[[10,20],[14,20],[14,23],[10,23]]}),
        );
        let line = record(serde_json::json!({"type":"line","p1":[0,0],"p2":[3,4]}));
        let point = record(serde_json::json!({"type":"point","at":[0,0]}));
        block(
            &mut project,
            "inner",
            [10.0, 20.0],
            vec![rectangle, line, point],
        );
        let reference = record(
            serde_json::json!({"type":"block_ref","block":"inner","at":[100,200],"rotation_deg":90,"scale":2,"mirror_x":true}),
        );
        block(&mut project, "outer", [100.0, 200.0], vec![reference]);
        let reference = record(
            serde_json::json!({"type":"block_ref","block":"outer","at":[300,400],"rotation_deg":90,"scale":0.5,"mirror_y":true}),
        );
        let id = reference.entity.id().as_str().to_owned();
        project.drawings[0].entities = vec![
            reference,
            record(
                serde_json::json!({"type":"polyline","closed":true,"points":[[296,397],[300,397],[300,400],[296,400]]}),
            ),
        ];
        let sum = measure_project(&project, None, &[]).unwrap();
        assert!((sum.total_length_mm - 33.0).abs() < 1e-10);
        assert!((sum.total_area_mm2 - 24.0).abs() < 1e-10);
        assert_eq!(sum.unsupported_count, 1);
        assert_eq!(sum.supported_leaf_count, 3);
        let selected = measure_project(&project, Some("plan"), &[id]).unwrap();
        assert_eq!(selected.measurements.len(), 1);
        assert_eq!(selected.measurements[0].supported_leaf_count, 2);
        assert_eq!(selected.measurements[0].unsupported_leaf_count, 1);
        assert!(!selected.measurements[0].warnings.is_empty());
        let union = measure_project_with_options(&project, None, &[], union_options()).unwrap();
        assert!((union.total_area_mm2 - 12.0).abs() < 1e-10);
        assert_eq!(union.additive_area_mm2, 24.0);
        assert!(union.csv().contains("aggregate,,,,33,12,,,union,0"));
    }
    #[test]
    fn union_preserves_holes_and_keeps_drawings_separate() {
        let (_temp, mut project) = source();
        project.drawings[0].entities = vec![
            record(
                serde_json::json!({"type":"hatch","loops":[rectangle(0.0,10.0),rectangle(2.0,8.0)],"pattern":"solid","angle_deg":0,"scale":1}),
            ),
            record(
                serde_json::json!({"type":"polyline","closed":true,"points":rectangle(4.0,6.0)}),
            ),
            record(
                serde_json::json!({"type":"polyline","closed":true,"points":rectangle(4.0,6.0)}),
            ),
        ];
        let one = measure_project_with_options(&project, None, &[], union_options()).unwrap();
        assert_eq!(one.total_area_mm2, 68.0);
        assert_eq!(one.additive_area_mm2, 72.0);
        let mut other = project.drawings[0].clone();
        other.name = "other".into();
        for item in &mut other.entities {
            let mut json = serde_json::to_value(&item.entity).unwrap();
            json["id"] = format!("ent_{}", ulid::Ulid::new()).into();
            item.entity = serde_json::from_value(json).unwrap();
        }
        project.drawings.push(other);
        let two = measure_project_with_options(&project, None, &[], union_options()).unwrap();
        assert_eq!(two.total_area_mm2, 136.0);
        assert_eq!(two.area_by_drawing.len(), 2);
        let selected =
            measure_project_with_options(&project, Some("other"), &[], union_options()).unwrap();
        assert_eq!(selected.total_area_mm2, 68.0);
    }
    #[test]
    fn curved_union_reports_approximation_and_limits_work() {
        let (_temp, mut project) = source();
        let a = record(serde_json::json!({"type":"circle","center":[0,0],"radius":10}));
        let b = record(serde_json::json!({"type":"circle","center":[0,0],"radius":10}));
        project.drawings[0].entities = vec![a, b];
        let report = measure_project_with_options(&project, None, &[], union_options()).unwrap();
        let deficit = 100.0 * PI - report.total_area_mm2;
        assert!(deficit > 0.0);
        assert!((report.area_approximation_error_estimate_mm2 - 2.0 * deficit).abs() < 1e-9);
        assert!((report.total_length_mm - 40.0 * PI).abs() < 1e-9);
        assert!(
            measure_project_with_options(
                &project,
                None,
                &[],
                MeasurementOptions {
                    curve_tolerance_mm: 1e-15,
                    ..union_options()
                }
            )
            .is_err()
        );
        assert!(
            measure_project_with_options(
                &project,
                None,
                &[],
                MeasurementOptions {
                    curve_tolerance_mm: f64::NAN,
                    ..union_options()
                }
            )
            .is_err()
        );
        block(
            &mut project,
            "cycle",
            [0.0, 0.0],
            vec![record(
                serde_json::json!({"type":"block_ref","block":"cycle","at":[0,0],"rotation_deg":0,"scale":1}),
            )],
        );
        project.drawings[0].entities = vec![record(
            serde_json::json!({"type":"block_ref","block":"cycle","at":[0,0],"rotation_deg":0,"scale":1}),
        )];
        assert!(
            measure_project(&project, None, &[])
                .unwrap_err()
                .to_string()
                .contains("recursive")
        );
        project.blocks.clear();
        assert!(
            measure_project(&project, None, &[])
                .unwrap_err()
                .to_string()
                .contains("missing")
        );
    }
    #[test]
    fn partially_overlapping_and_touching_boundaries_are_unioned_across_entities() {
        let (_temp, mut project) = source();
        let first = record(
            serde_json::json!({"type":"polyline","closed":true,"points":rectangle(0.0,10.0)}),
        );
        project.drawings[0].entities = vec![
            first,
            record(
                serde_json::json!({"type":"polyline","closed":true,"points":rectangle(5.0,15.0)}),
            ),
        ];
        let partial = measure_project_with_options(&project, None, &[], union_options()).unwrap();
        assert_eq!(partial.total_area_mm2, 175.0);
        project.drawings[0].entities[1] = record(
            serde_json::json!({"type":"polyline","closed":true,"points":rectangle(10.0,20.0)}),
        );
        let touching = measure_project_with_options(&project, None, &[], union_options()).unwrap();
        assert_eq!(touching.total_area_mm2, 200.0);
        project.drawings[0].entities = vec![record(
            serde_json::json!({"type":"ellipse","center":[10,20],"radius_x":10,"radius_y":3,"rotation_deg":37,"start_deg":0,"end_deg":360}),
        )];
        let ellipse = measure_project_with_options(&project, None, &[], union_options()).unwrap();
        assert!(
            (ellipse.total_area_mm2 + ellipse.area_approximation_error_estimate_mm2 - 30.0 * PI)
                .abs()
                < 1e-9
        );
    }
}
