use crate::{Result, ToolkitError};
use cad_edit::EditOperation;
use cad_model::{Entity, Point, ProjectSource};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use ulid::Ulid;

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GeneratorRequest {
    pub layer: String,
    #[serde(default)]
    pub pen: Option<String>,
    pub geometry: Geometry,
}
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Geometry {
    DoubleLine {
        p1: Point,
        p2: Point,
        width: f64,
        #[serde(default)]
        centerline: bool,
    },
    Centerline {
        p1: Point,
        p2: Point,
        extension: f64,
    },
    Door {
        hinge: Point,
        width: f64,
        rotation_deg: f64,
        swing_deg: f64,
    },
    Window {
        p1: Point,
        p2: Point,
        depth: f64,
        panels: usize,
    },
    SlidingDoor {
        p1: Point,
        p2: Point,
        depth: f64,
    },
    Ellipse {
        center: Point,
        radius_x: f64,
        radius_y: f64,
        rotation_deg: f64,
        #[serde(default)]
        start_deg: f64,
        #[serde(default = "full_circle")]
        end_deg: f64,
    },
    CubicBezier {
        points: [Point; 4],
        tolerance_mm: f64,
    },
    CircleTangents {
        center: Point,
        radius: f64,
        from: Point,
    },
    TangentCircle {
        line_ids: [String; 2],
        radius: f64,
        near: Point,
    },
    CalculatedText {
        at: Point,
        expression: String,
        precision: u8,
        style: String,
        #[serde(default)]
        prefix: String,
        #[serde(default)]
        suffix: String,
        #[serde(default)]
        rotation_deg: f64,
    },
    MassingProjection {
        footprint_ids: Vec<String>,
        height_mm: f64,
        origin: Point,
        at: Point,
        yaw_deg: f64,
        elevation_deg: f64,
    },
    SunShadow {
        footprint_ids: Vec<String>,
        height_mm: f64,
        sun_azimuth_deg: f64,
        sun_altitude_deg: f64,
    },
    SkyView {
        footprint_ids: Vec<String>,
        height_mm: f64,
        observer: [f64; 3],
        azimuth_samples: usize,
        at: Point,
        radius_mm: f64,
    },
    Divide {
        entity_id: String,
        segments: usize,
    },
}
fn full_circle() -> f64 {
    360.0
}
#[derive(Debug, Serialize)]
pub struct GeneratedEdit {
    pub operation: EditOperation,
    pub warnings: Vec<String>,
    pub analysis_report: Option<crate::massing::Report>,
}

fn invalid(message: &str) -> ToolkitError {
    ToolkitError::Invalid(message.into())
}
fn finite(point: Point) -> Result<()> {
    if point.iter().all(|value| value.is_finite()) {
        Ok(())
    } else {
        Err(invalid("coordinates must be finite"))
    }
}
fn positive(value: f64) -> Result<()> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(invalid("size must be finite and positive"))
    }
}
fn axis(p1: Point, p2: Point) -> Result<(Point, Point, f64)> {
    finite(p1)?;
    finite(p2)?;
    let length = (p2[0] - p1[0]).hypot(p2[1] - p1[1]);
    positive(length)?;
    let u = [(p2[0] - p1[0]) / length, (p2[1] - p1[1]) / length];
    Ok((u, [-u[1], u[0]], length))
}
fn add(a: Point, b: Point, factor: f64) -> Point {
    [a[0] + factor * b[0], a[1] + factor * b[1]]
}
fn polar(center: Point, radius: f64, angle: f64) -> Point {
    add(
        center,
        [angle.to_radians().cos(), angle.to_radians().sin()],
        radius,
    )
}

pub fn generate(
    project: &ProjectSource,
    drawing: &str,
    request: &GeneratorRequest,
) -> Result<GeneratedEdit> {
    let source = project
        .drawings
        .iter()
        .find(|d| d.name == drawing)
        .ok_or_else(|| invalid("drawing does not exist"))?;
    let mut values = Vec::new();
    let mut warnings = Vec::new();
    let mut analysis_report = None;
    let line = |p1: Point, p2: Point| json!({"type":"line","p1":p1,"p2":p2});
    match &request.geometry {
        Geometry::MassingProjection {
            footprint_ids,
            height_mm,
            origin,
            at,
            yaw_deg,
            elevation_deg,
        } => {
            let report = crate::massing::analyze(&crate::massing::Request {
                schema_version: "cad-massing/1".into(),
                buildings: crate::massing::scene_from_drawing(
                    project,
                    drawing,
                    footprint_ids,
                    *height_mm,
                )?,
                analysis: crate::massing::Analysis::Projection {
                    origin: *origin,
                    at: *at,
                    yaw_deg: *yaw_deg,
                    elevation_deg: *elevation_deg,
                },
            })?;
            if let crate::massing::Output::Projection { edges } = &report.result {
                for edge in edges {
                    values.push(line(edge.p1, edge.p2));
                }
            }
            warnings.extend(report.warnings.clone());
            analysis_report = Some(report);
        }
        Geometry::SunShadow {
            footprint_ids,
            height_mm,
            sun_azimuth_deg,
            sun_altitude_deg,
        } => {
            let report = crate::massing::analyze(&crate::massing::Request {
                schema_version: "cad-massing/1".into(),
                buildings: crate::massing::scene_from_drawing(
                    project,
                    drawing,
                    footprint_ids,
                    *height_mm,
                )?,
                analysis: crate::massing::Analysis::SunShadow {
                    sun_azimuth_deg: *sun_azimuth_deg,
                    sun_altitude_deg: *sun_altitude_deg,
                    ground_z_mm: 0.,
                },
            })?;
            if let crate::massing::Output::SunShadow { regions, area_mm2 } = &report.result {
                for region in regions {
                    for points in &region.loops {
                        values.push(json!({"type":"polyline","points":points,"closed":true}));
                    }
                }
                warnings.push(format!("Ground shadow union area: {area_mm2} mm²."));
            }
            warnings.extend(report.warnings.clone());
            analysis_report = Some(report);
        }
        Geometry::SkyView {
            footprint_ids,
            height_mm,
            observer,
            azimuth_samples,
            at,
            radius_mm,
        } => {
            finite(*at)?;
            positive(*radius_mm)?;
            let report = crate::massing::analyze(&crate::massing::Request {
                schema_version: "cad-massing/1".into(),
                buildings: crate::massing::scene_from_drawing(
                    project,
                    drawing,
                    footprint_ids,
                    *height_mm,
                )?,
                analysis: crate::massing::Analysis::SkyView {
                    observer: *observer,
                    azimuth_samples: *azimuth_samples,
                },
            })?;
            if let crate::massing::Output::SkyView {
                horizontal_sky_view_factor,
                refinement_change,
                horizon,
                ..
            } = &report.result
            {
                let mut points = horizon
                    .iter()
                    .map(|sample| {
                        let az = sample.azimuth_deg.to_radians();
                        let r = radius_mm * sample.altitude_deg.to_radians().cos();
                        [at[0] + r * az.sin(), at[1] + r * az.cos()]
                    })
                    .collect::<Vec<_>>();
                if let Some(first) = points.first().copied() {
                    points.push(first);
                }
                values.push(json!({"type":"circle","center":at,"radius":radius_mm}));
                values.push(json!({"type":"polyline","points":points,"closed":true}));
                warnings.push(format!("Horizontal sky-view factor: {:.8}; refinement change: {:.8} (not a certified error bound). Orthographic sky diagram; north is +y.",horizontal_sky_view_factor,refinement_change));
            }
            warnings.extend(report.warnings.clone());
            analysis_report = Some(report);
        }
        Geometry::DoubleLine {
            p1,
            p2,
            width,
            centerline,
        } => {
            positive(*width)?;
            let (_, normal, _) = axis(*p1, *p2)?;
            for offset in [-width / 2.0, width / 2.0] {
                values.push(line(add(*p1, normal, offset), add(*p2, normal, offset)));
            }
            if *centerline {
                values.push(line(*p1, *p2));
            }
        }
        Geometry::Centerline { p1, p2, extension } => {
            if !extension.is_finite() || *extension < 0.0 {
                return Err(invalid("extension must be non-negative"));
            }
            let (u, _, _) = axis(*p1, *p2)?;
            values.push(line(add(*p1, u, -extension), add(*p2, u, *extension)));
            if request.pen.is_none() {
                warnings.push("Choose a centerline pen to use the project's dash pattern.".into());
            }
        }
        Geometry::Door {
            hinge,
            width,
            rotation_deg,
            swing_deg,
        } => {
            finite(*hinge)?;
            positive(*width)?;
            if !rotation_deg.is_finite()
                || !swing_deg.is_finite()
                || swing_deg.abs() < 1e-9
                || swing_deg.abs() > 180.0
            {
                return Err(invalid(
                    "door swing must be nonzero and at most 180 degrees",
                ));
            }
            values.push(line(
                *hinge,
                polar(*hinge, *width, rotation_deg + swing_deg),
            ));
            values.push(json!({"type":"arc","center":hinge,"radius":width,"start_deg":rotation_deg,"end_deg":rotation_deg+swing_deg}));
        }
        Geometry::Window {
            p1,
            p2,
            depth,
            panels,
        } => {
            positive(*depth)?;
            if *panels == 0 || *panels > 1000 {
                return Err(invalid("window panels must be 1..1000"));
            }
            let (u, n, length) = axis(*p1, *p2)?;
            for offset in [-depth / 2.0, 0.0, depth / 2.0] {
                values.push(line(add(*p1, n, offset), add(*p2, n, offset)));
            }
            for i in 0..=*panels {
                let position = add(*p1, u, length * i as f64 / *panels as f64);
                values.push(line(
                    add(position, n, -depth / 2.0),
                    add(position, n, depth / 2.0),
                ));
            }
        }
        Geometry::SlidingDoor { p1, p2, depth } => {
            positive(*depth)?;
            let (u, n, length) = axis(*p1, *p2)?;
            for (start, end, offset) in [(0.0, 0.55, -depth / 2.0), (0.45, 1.0, depth / 2.0)] {
                let a = add(add(*p1, u, length * start), n, offset);
                let b = add(add(*p1, u, length * end), n, offset);
                values.push(line(a, b));
                values.push(line(add(a, n, -depth / 4.0), add(a, n, depth / 4.0)));
                values.push(line(add(b, n, -depth / 4.0), add(b, n, depth / 4.0)));
            }
        }
        Geometry::Ellipse {
            center,
            radius_x,
            radius_y,
            rotation_deg,
            start_deg,
            end_deg,
        } => {
            finite(*center)?;
            positive(*radius_x)?;
            positive(*radius_y)?;
            if ![rotation_deg, start_deg, end_deg]
                .iter()
                .all(|v| v.is_finite())
                || (end_deg - start_deg).abs() < 1e-9
                || (end_deg - start_deg).abs() > 360.0
            {
                return Err(invalid(
                    "ellipse angles must define a nonzero sweep of at most 360 degrees",
                ));
            }
            values.push(json!({"type":"ellipse","center":center,"radius_x":radius_x,"radius_y":radius_y,"rotation_deg":rotation_deg,"start_deg":start_deg,"end_deg":end_deg}));
        }
        Geometry::CubicBezier {
            points,
            tolerance_mm,
        } => {
            for point in points {
                finite(*point)?;
            }
            positive(*tolerance_mm)?;
            let mut polyline = vec![points[0]];
            flatten_bezier(*points, *tolerance_mm, 0, &mut polyline)?;
            values.push(json!({"type":"polyline","points":polyline,"closed":false}));
            warnings.push(format!("Cubic Bezier converted to a polyline with control-hull distance tolerance {tolerance_mm} mm."));
        }
        Geometry::CircleTangents {
            center,
            radius,
            from,
        } => {
            finite(*center)?;
            finite(*from)?;
            positive(*radius)?;
            let (_, _, distance) = axis(*center, *from)?;
            if distance <= *radius {
                return Err(invalid(
                    "tangent point must lie strictly outside the circle",
                ));
            }
            let direction = (from[1] - center[1])
                .atan2(from[0] - center[0])
                .to_degrees();
            let angle = (radius / distance).acos().to_degrees();
            for sign in [-1.0, 1.0] {
                values.push(line(
                    *from,
                    polar(*center, *radius, direction + sign * angle),
                ));
            }
        }
        Geometry::TangentCircle {
            line_ids,
            radius,
            near,
        } => {
            positive(*radius)?;
            finite(*near)?;
            if line_ids[0] == line_ids[1] {
                return Err(invalid("select two distinct lines for a tangent circle"));
            }
            let mut lines = Vec::new();
            for id in line_ids {
                let entity = source
                    .entities
                    .iter()
                    .find(|r| r.entity.id().as_str() == id)
                    .ok_or_else(|| invalid("tangent circle line does not exist"))?;
                let Entity::Line { p1, p2, .. } = entity.entity else {
                    return Err(invalid(
                        "tangent circle requires two straight line entities",
                    ));
                };
                lines.push((p1, p2));
            }
            let center = tangent_circle_center(lines[0], lines[1], *radius, *near)?;
            values.push(json!({"type":"circle","center":center,"radius":radius}));
            warnings.push("Tangency uses the infinite supporting lines; contact points may be outside the selected segments. The solution nearest the specified point was chosen.".into());
        }
        Geometry::CalculatedText {
            at,
            expression,
            precision,
            style,
            prefix,
            suffix,
            rotation_deg,
        } => {
            finite(*at)?;
            if !rotation_deg.is_finite() || prefix.len().saturating_add(suffix.len()) > 4096 {
                return Err(invalid(
                    "calculated text requires a finite rotation and bounded prefix/suffix",
                ));
            }
            if !project.styles.text_styles.contains_key(style) {
                return Err(invalid("calculated text style does not exist"));
            }
            let result = crate::text::calculate(expression, *precision)?;
            let value = format!("{prefix}{result}{suffix}");
            values.push(json!({"type":"text","at":at,"style":style,"value":value,"rotation_deg":rotation_deg,"mirror_y":false}));
            warnings.push(format!("Calculated {expression} = {result} using scalar f64 arithmetic; the saved annotation is ordinary text and does not recalculate automatically."));
        }
        Geometry::Divide {
            entity_id,
            segments,
        } => {
            if *segments < 2 || *segments > 10000 {
                return Err(invalid("division segments must be 2..10000"));
            }
            let entity = source
                .entities
                .iter()
                .find(|r| r.entity.id().as_str() == entity_id)
                .ok_or_else(|| invalid("division entity does not exist"))?;
            for index in 1..*segments {
                let t = index as f64 / *segments as f64;
                let at = match &entity.entity {
                    Entity::Line { p1, p2, .. } => {
                        [p1[0] + t * (p2[0] - p1[0]), p1[1] + t * (p2[1] - p1[1])]
                    }
                    Entity::Arc {
                        center,
                        radius,
                        start_deg,
                        end_deg,
                        ..
                    } => polar(*center, *radius, start_deg + t * (end_deg - start_deg)),
                    _ => {
                        return Err(invalid(
                            "equal division currently supports lines and circular arcs",
                        ));
                    }
                };
                values.push(json!({"type":"point","at":at,"temporary":false}));
            }
        }
    }
    let mut operations = Vec::new();
    for mut value in values {
        value["schema_version"] = json!(cad_model::CURRENT_SCHEMA_VERSION);
        value["id"] = json!(format!("ent_{}", Ulid::new()));
        value["layer"] = json!(request.layer);
        value["pen"] = json!(request.pen);
        // Decode once here; the edit preview also checks project references.
        let _: Entity = serde_json::from_value(value.clone())?;
        operations.push(EditOperation::Create { entity: value });
    }
    Ok(GeneratedEdit {
        operation: EditOperation::Batch { operations },
        warnings,
        analysis_report,
    })
}
/// Solve signed-distance intersections in coordinates local to the first line.
/// Near-parallel lines are rejected instead of publishing unstable distant centers.
fn tangent_circle_center(
    first: (Point, Point),
    second: (Point, Point),
    radius: f64,
    near: Point,
) -> Result<Point> {
    let (_, n1, _) = axis(first.0, first.1)?;
    let (_, n2, _) = axis(second.0, second.1)?;
    finite(n1)?;
    finite(n2)?;
    let det = n1[0] * n2[1] - n1[1] * n2[0];
    if det.abs() <= 1e-10 {
        return Err(invalid(
            "parallel or nearly parallel lines do not have a stable isolated tangent-circle solution",
        ));
    }
    let delta = [second.0[0] - first.0[0], second.0[1] - first.0[1]];
    finite(delta)?;
    let offset = n2[0] * delta[0] + n2[1] * delta[1];
    let mut centers = Vec::new();
    for sign1 in [-1., 1.] {
        for sign2 in [-1., 1.] {
            let a = sign1 * radius;
            let b = offset + sign2 * radius;
            let center = [
                first.0[0] + (a * n2[1] - n1[1] * b) / det,
                first.0[1] + (n1[0] * b - a * n2[0]) / det,
            ];
            finite(center)?;
            for (origin, normal) in [(first.0, n1), (second.0, n2)] {
                let signed =
                    normal[0] * (center[0] - origin[0]) + normal[1] * (center[1] - origin[1]);
                if !signed.is_finite() || (signed.abs() - radius).abs() > radius * 1e-7 {
                    return Err(invalid(
                        "tangent-circle tangency cannot be represented accurately at these coordinates",
                    ));
                }
            }
            let distance = (center[0] - near[0]).hypot(center[1] - near[1]);
            if !distance.is_finite() {
                return Err(invalid(
                    "tangent-circle coordinates exceed the finite model range",
                ));
            }
            centers.push((distance, center));
        }
    }
    centers.sort_by(|a, b| {
        a.0.total_cmp(&b.0)
            .then_with(|| a.1[0].total_cmp(&b.1[0]))
            .then_with(|| a.1[1].total_cmp(&b.1[1]))
    });
    Ok(centers[0].1)
}

fn flatten_bezier(
    points: [Point; 4],
    tolerance: f64,
    depth: usize,
    out: &mut Vec<Point>,
) -> Result<()> {
    fn segment_distance(p: Point, a: Point, b: Point) -> f64 {
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let squared = dx * dx + dy * dy;
        let t = if squared == 0.0 {
            0.0
        } else {
            (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / squared).clamp(0.0, 1.0)
        };
        (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
    }
    let error = segment_distance(points[1], points[0], points[3])
        .max(segment_distance(points[2], points[0], points[3]));
    if error <= tolerance {
        out.push(points[3]);
        return Ok(());
    }
    if depth >= 24 || out.len() > 10000 {
        return Err(invalid("Bezier tolerance requires too many segments"));
    }
    let mid = |a: Point, b: Point| [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
    let a = mid(points[0], points[1]);
    let b = mid(points[1], points[2]);
    let c = mid(points[2], points[3]);
    let d = mid(a, b);
    let e = mid(b, c);
    let f = mid(d, e);
    flatten_bezier([points[0], a, d, f], tolerance, depth + 1, out)?;
    flatten_bezier([f, e, c, points[3]], tolerance, depth + 1, out)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextReplaceRequest {
    pub find: String,
    pub replace: String,
    #[serde(default)]
    pub entity_ids: Vec<String>,
    #[serde(default)]
    pub style: Option<String>,
}
/// Literal text edits preserve IDs; dimension labels are deliberately excluded.
pub fn replace_text(
    project: &ProjectSource,
    drawing: &str,
    request: &TextReplaceRequest,
) -> Result<GeneratedEdit> {
    if request.find.is_empty() {
        return Err(invalid("find must not be empty"));
    }
    let source = project
        .drawings
        .iter()
        .find(|d| d.name == drawing)
        .ok_or_else(|| invalid("drawing does not exist"))?;
    let mut remaining: BTreeSet<_> = request.entity_ids.iter().map(String::as_str).collect();
    if remaining.len() != request.entity_ids.len() {
        return Err(invalid("text selection contains duplicate IDs"));
    }
    let mut operations = Vec::new();
    for record in &source.entities {
        if !request.entity_ids.is_empty() && !remaining.contains(record.entity.id().as_str()) {
            continue;
        }
        remaining.remove(record.entity.id().as_str());
        if let Entity::Text { value, .. } = &record.entity
            && value.contains(&request.find)
        {
            let mut entity: Value = serde_json::to_value(&record.entity)?;
            entity["value"] = json!(value.replace(&request.find, &request.replace));
            if let Some(style) = &request.style {
                entity["style"] = json!(style);
            }
            operations.push(EditOperation::Replace {
                entity_id: record.entity.id().as_str().into(),
                entity,
            });
        }
    }
    if !remaining.is_empty() {
        return Err(invalid("text selection includes missing entities"));
    }
    if operations.is_empty() {
        return Err(invalid("no matching text entities"));
    }
    Ok(GeneratedEdit {
        operation: EditOperation::Batch { operations },
        warnings: Vec::new(),
        analysis_report: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn massing_generators_are_checked_and_undoable_without_changing_footprints() {
        let temp = tempfile::tempdir().unwrap();
        let created = cad_edit::create_project(&cad_edit::ProjectTemplateRequest {
            parent_dir: temp.path().display().to_string(),
            folder_name: "massing".into(),
            project_name: "Massing".into(),
            drawing: "plan".into(),
            paper: "A4".into(),
            orientation: cad_model::SheetOrientation::Landscape,
            scale_denominator: 50,
        })
        .unwrap();
        let root = std::path::Path::new(&created.project_path);
        let id = format!("ent_{}", Ulid::new());
        let source = cad_model::load_project(root).unwrap();
        let created_footprint = cad_edit::apply_edit(root, &cad_edit::DrawingEditRequest {
            drawing: "plan".into(), expected_revision: cad_edit::editor_state(&source,"plan").unwrap().revision,
            operation: EditOperation::Create { entity: json!({"schema_version":cad_model::CURRENT_SCHEMA_VERSION,"id":id,"type":"polyline","layer":"0-1","closed":true,"points":[[0,0],[4000,0],[4000,4000],[0,4000],[0,0]]}) },
        }).unwrap();
        let id = created_footprint.entity_ids[0].clone();
        let path = root.join("drawings/plan/entities.ndjson");
        let before = std::fs::read(&path).unwrap();
        let mut geometries = vec![
            Geometry::MassingProjection {
                footprint_ids: vec![id.clone()],
                height_mm: 5000.,
                origin: [0., 0.],
                at: [10000., 0.],
                yaw_deg: 30.,
                elevation_deg: 30.,
            },
            Geometry::SkyView {
                footprint_ids: vec![id.clone()],
                height_mm: 5000.,
                observer: [-5000., -5000., 0.],
                azimuth_samples: 180,
                at: [10000., 10000.],
                radius_mm: 1000.,
            },
        ];
        for azimuth in [0., 90., 180., 270., 32.] {
            geometries.push(Geometry::SunShadow {
                footprint_ids: vec![id.clone()],
                height_mm: 5000.,
                sun_azimuth_deg: azimuth,
                sun_altitude_deg: 45.,
            });
        }
        for geometry in geometries {
            let source = cad_model::load_project(root).unwrap();
            let generated = generate(
                &source,
                "plan",
                &GeneratorRequest {
                    layer: "0-1".into(),
                    pen: None,
                    geometry,
                },
            )
            .unwrap();
            assert!(generated.analysis_report.is_some());
            assert!(!generated.warnings.is_empty());
            let mut edit = cad_edit::DrawingEditRequest {
                drawing: "plan".into(),
                expected_revision: cad_edit::editor_state(&source, "plan").unwrap().revision,
                operation: generated.operation,
            };
            let preview = cad_edit::preview_edit(root, &edit).unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), before);
            edit.operation = EditOperation::SourceChecked {
                operation: Box::new(edit.operation),
                expected_files: preview.source_files,
            };
            cad_edit::apply_edit(root, &edit).unwrap();
            assert!(cad_check::check_project(root).is_ok());
            assert!(std::fs::read(&path).unwrap().starts_with(&before));
            assert!(cad_edit::apply_edit(root, &edit).is_err());
            cad_edit::undo_drawing_edit(
                root,
                &cad_edit::DrawingHistoryRequest {
                    drawing: "plan".into(),
                    expected_files: cad_edit::list_drawing_history(root, "plan")
                        .unwrap()
                        .current_files,
                },
            )
            .unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), before);
        }
    }
    fn project() -> ProjectSource {
        cad_model::load_project(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small"),
        )
        .unwrap()
    }

    #[test]
    fn tangent_circle_selects_all_four_sides_and_rotated_lines() {
        let first = ([0., 0.], [10., 0.]);
        let second = ([0., 0.], [0., 10.]);
        for x in [-5., 5.] {
            for y in [-5., 5.] {
                for (a, b) in [
                    (first, second),
                    (second, first),
                    ((first.1, first.0), (second.1, second.0)),
                ] {
                    let center = tangent_circle_center(a, b, 5., [x * 2., y * 2.]).unwrap();
                    assert_eq!(center, [x, y]);
                }
            }
        }
        let transform = |p: Point| {
            let angle = 30_f64.to_radians();
            [
                1000. + p[0] * angle.cos() - p[1] * angle.sin(),
                2000. + p[0] * angle.sin() + p[1] * angle.cos(),
            ]
        };
        let center = tangent_circle_center(
            (transform(first.0), transform(first.1)),
            (transform(second.0), transform(second.1)),
            5.,
            transform([10., 10.]),
        )
        .unwrap();
        let expected = transform([5., 5.]);
        assert!((center[0] - expected[0]).abs() < 1e-9);
        assert!((center[1] - expected[1]).abs() < 1e-9);
        for bad in [
            ([0., 10.], [10., 10.]),
            ([0., 10.], [10., 10. + 1e-11]),
            ([0., 0.], [0., 0.]),
        ] {
            assert!(tangent_circle_center(first, bad, 5., [10., 10.]).is_err());
        }
        assert!(
            tangent_circle_center(
                ([1e16, 1e16], [1e16 + 10., 1e16]),
                ([1e16, 1e16], [1e16, 1e16 + 10.]),
                1.,
                [1e16, 1e16]
            )
            .is_err()
        );
    }

    #[test]
    fn tangent_circle_is_a_checked_undoable_edit_and_rejects_missing_inputs() {
        let temp = tempfile::tempdir().unwrap();
        let created = cad_edit::create_project(&cad_edit::ProjectTemplateRequest {
            parent_dir: temp.path().display().to_string(),
            folder_name: "circle".into(),
            project_name: "circle".into(),
            drawing: "plan".into(),
            paper: "A4".into(),
            orientation: cad_model::SheetOrientation::Landscape,
            scale_denominator: 50,
        })
        .unwrap();
        let root = std::path::Path::new(&created.project_path);
        let source = cad_model::load_project(root).unwrap();
        let ids = [
            format!("ent_{}", Ulid::new()),
            format!("ent_{}", Ulid::new()),
        ];
        let applied = cad_edit::apply_edit(
            root,
            &cad_edit::DrawingEditRequest {
                drawing: "plan".into(),
                expected_revision: cad_edit::editor_state(&source, "plan").unwrap().revision,
                operation: EditOperation::Batch {
                    operations: ids
                        .iter()
                        .zip([[10., 0.], [0., 10.]])
                        .map(|(id, p2)| EditOperation::Create {
                            entity: json!({"schema_version":cad_model::CURRENT_SCHEMA_VERSION,
                    "type":"line","id":id,"layer":"0-1","p1":[0,0],"p2":p2}),
                        })
                        .collect(),
                },
            },
        )
        .unwrap();
        let ids: [String; 2] = applied.entity_ids.try_into().unwrap();
        let path = root.join("drawings/plan/entities.ndjson");
        let before = std::fs::read(&path).unwrap();
        let source = cad_model::load_project(root).unwrap();
        let request = |line_ids, radius, near| GeneratorRequest {
            layer: "0-1".into(),
            pen: None,
            geometry: Geometry::TangentCircle {
                line_ids,
                radius,
                near,
            },
        };
        for invalid_request in [
            request([ids[0].clone(), ids[0].clone()], 5., [10., 10.]),
            request([ids[0].clone(), "missing".into()], 5., [10., 10.]),
            request(ids.clone(), 0., [10., 10.]),
            request(ids.clone(), 5., [f64::NAN, 10.]),
        ] {
            assert!(generate(&source, "plan", &invalid_request).is_err());
        }
        let generated = generate(&source, "plan", &request(ids.clone(), 5., [10., 10.])).unwrap();
        assert_eq!(generated.warnings.len(), 1);
        let mut edit = cad_edit::DrawingEditRequest {
            drawing: "plan".into(),
            expected_revision: cad_edit::editor_state(&source, "plan").unwrap().revision,
            operation: generated.operation,
        };
        let preview = cad_edit::preview_edit(root, &edit).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
        edit.operation = EditOperation::SourceChecked {
            operation: Box::new(edit.operation),
            expected_files: preview.source_files,
        };
        cad_edit::apply_edit(root, &edit).unwrap();
        assert!(cad_check::check_project(root).is_ok());
        let updated = cad_model::load_project(root).unwrap();
        let circle = updated.drawings[0]
            .entities
            .iter()
            .find(|e| matches!(e.entity, Entity::Circle { .. }))
            .unwrap();
        let Entity::Circle { center, radius, .. } = circle.entity else {
            panic!()
        };
        assert_eq!(center, [5., 5.]);
        assert_eq!(radius, 5.);
        assert!(
            generate(
                &updated,
                "plan",
                &request(
                    [ids[0].clone(), circle.entity.id().as_str().into()],
                    5.,
                    [10., 10.]
                )
            )
            .is_err()
        );
        assert!(cad_edit::apply_edit(root, &edit).is_err());
        cad_edit::undo_drawing_edit(
            root,
            &cad_edit::DrawingHistoryRequest {
                drawing: "plan".into(),
                expected_files: cad_edit::list_drawing_history(root, "plan")
                    .unwrap()
                    .current_files,
            },
        )
        .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn generated_wall_applies_as_one_undoable_checked_edit() {
        let temp = tempfile::tempdir().unwrap();
        let created = cad_edit::create_project(&cad_edit::ProjectTemplateRequest {
            parent_dir: temp.path().to_string_lossy().into(),
            folder_name: "test".into(),
            project_name: "test".into(),
            drawing: "plan".into(),
            paper: "A4".into(),
            orientation: cad_model::SheetOrientation::Landscape,
            scale_denominator: 50,
        })
        .unwrap();
        let path = std::path::Path::new(&created.project_path);
        let source = cad_model::load_project(path).unwrap();
        let generated = generate(
            &source,
            "plan",
            &GeneratorRequest {
                layer: "0-1".into(),
                pen: None,
                geometry: Geometry::DoubleLine {
                    p1: [0.0, 0.0],
                    p2: [1000.0, 0.0],
                    width: 100.0,
                    centerline: false,
                },
            },
        )
        .unwrap();
        let mut request = cad_edit::DrawingEditRequest {
            drawing: "plan".into(),
            expected_revision: cad_edit::editor_state(&source, "plan").unwrap().revision,
            operation: generated.operation,
        };
        let preview = cad_edit::preview_edit(path, &request).unwrap();
        assert_eq!(preview.entities.len(), 2);
        request.operation = EditOperation::SourceChecked {
            operation: Box::new(request.operation),
            expected_files: preview.source_files,
        };
        let applied = cad_edit::apply_edit(path, &request).unwrap();
        assert_eq!(applied.entity_ids.len(), 2);
        assert!(cad_check::check_project(path).is_ok());
        assert!(cad_edit::apply_edit(path, &request).is_err());
        cad_edit::undo_drawing_edit(
            path,
            &cad_edit::DrawingHistoryRequest {
                drawing: "plan".into(),
                expected_files: Vec::new(),
            },
        )
        .unwrap();
        assert!(
            cad_model::load_project(path).unwrap().drawings[0]
                .entities
                .is_empty()
        );
        cad_edit::redo_drawing_edit(
            path,
            &cad_edit::DrawingHistoryRequest {
                drawing: "plan".into(),
                expected_files: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(
            cad_model::load_project(path).unwrap().drawings[0]
                .entities
                .len(),
            2
        );
    }
    #[test]
    fn rotated_wall_and_negative_door_swing_keep_real_geometry() {
        let source = project();
        let drawing = &source.drawings[0].name;
        let edit = generate(
            &source,
            drawing,
            &GeneratorRequest {
                layer: "0-1".into(),
                pen: None,
                geometry: Geometry::DoubleLine {
                    p1: [0.0, 0.0],
                    p2: [0.0, 1000.0],
                    width: 100.0,
                    centerline: true,
                },
            },
        )
        .unwrap();
        let EditOperation::Batch { operations } = edit.operation else {
            panic!()
        };
        assert_eq!(operations.len(), 3);
        let EditOperation::Create { entity } = &operations[0] else {
            panic!()
        };
        assert_eq!(entity["p1"], json!([50.0, 0.0]));
        let door = generate(
            &source,
            drawing,
            &GeneratorRequest {
                layer: "0-1".into(),
                pen: None,
                geometry: Geometry::Door {
                    hinge: [100.0, 200.0],
                    width: 900.0,
                    rotation_deg: 0.0,
                    swing_deg: -90.0,
                },
            },
        )
        .unwrap();
        let EditOperation::Batch { operations } = door.operation else {
            panic!()
        };
        let EditOperation::Create { entity } = &operations[0] else {
            panic!()
        };
        assert!((entity["p2"][0].as_f64().unwrap() - 100.0).abs() < 1e-9);
        assert_eq!(entity["p2"][1], -700.0);
    }
    #[test]
    fn circle_tangents_are_perpendicular_and_reject_internal_point() {
        let source = project();
        let drawing = &source.drawings[0].name;
        let edit = generate(
            &source,
            drawing,
            &GeneratorRequest {
                layer: "0-1".into(),
                pen: None,
                geometry: Geometry::CircleTangents {
                    center: [0.0, 0.0],
                    radius: 5.0,
                    from: [13.0, 0.0],
                },
            },
        )
        .unwrap();
        let EditOperation::Batch { operations } = edit.operation else {
            panic!()
        };
        for operation in operations {
            let EditOperation::Create { entity } = operation else {
                panic!()
            };
            let x = entity["p2"][0].as_f64().unwrap();
            let y = entity["p2"][1].as_f64().unwrap();
            assert!((x * x + y * y - 25.0).abs() < 1e-9);
            assert!(((13.0 - x) * x - y * y).abs() < 1e-9);
        }
        assert!(
            generate(
                &source,
                drawing,
                &GeneratorRequest {
                    layer: "0-1".into(),
                    pen: None,
                    geometry: Geometry::CircleTangents {
                        center: [0.0, 0.0],
                        radius: 5.0,
                        from: [2.0, 0.0]
                    }
                }
            )
            .is_err()
        );
    }
    #[test]
    fn every_generator_applies_checked_geometry_and_literal_replace_keeps_ids() {
        let temp = tempfile::tempdir().unwrap();
        let created = cad_edit::create_project(&cad_edit::ProjectTemplateRequest {
            parent_dir: temp.path().display().to_string(),
            folder_name: "tools".into(),
            project_name: "tools".into(),
            drawing: "plan".into(),
            paper: "A3".into(),
            orientation: cad_model::SheetOrientation::Landscape,
            scale_denominator: 100,
        })
        .unwrap();
        let root = std::path::Path::new(&created.project_path);
        let text_style = cad_model::load_project(root)
            .unwrap()
            .styles
            .text_styles
            .keys()
            .next()
            .unwrap()
            .clone();
        let cases = vec![
            Geometry::CalculatedText {
                at: [5000., 5000.],
                expression: "1000*2000/1e6".into(),
                precision: 2,
                style: text_style,
                prefix: "面積: ".into(),
                suffix: " m²".into(),
                rotation_deg: 30.,
            },
            Geometry::Centerline {
                p1: [0.0, 0.0],
                p2: [1000.0, 0.0],
                extension: 50.0,
            },
            Geometry::Door {
                hinge: [1000.0, 1000.0],
                width: 900.0,
                rotation_deg: 30.0,
                swing_deg: -90.0,
            },
            Geometry::Window {
                p1: [2000.0, 0.0],
                p2: [3000.0, 500.0],
                depth: 100.0,
                panels: 3,
            },
            Geometry::SlidingDoor {
                p1: [3000.0, 0.0],
                p2: [4000.0, 500.0],
                depth: 100.0,
            },
            Geometry::Ellipse {
                center: [4000.0, 4000.0],
                radius_x: 1000.0,
                radius_y: 500.0,
                rotation_deg: 30.0,
                start_deg: 0.0,
                end_deg: 360.0,
            },
            Geometry::CubicBezier {
                points: [[0.0, 0.0], [100.0, 200.0], [200.0, 300.0], [400.0, 500.0]],
                tolerance_mm: 0.1,
            },
            Geometry::CircleTangents {
                center: [0.0, 0.0],
                radius: 500.0,
                from: [1300.0, 0.0],
            },
        ];
        for geometry in cases {
            let source = cad_model::load_project(root).unwrap();
            let generated = generate(
                &source,
                "plan",
                &GeneratorRequest {
                    layer: "0-1".into(),
                    pen: None,
                    geometry,
                },
            )
            .unwrap();
            let request = cad_edit::DrawingEditRequest {
                drawing: "plan".into(),
                expected_revision: cad_edit::editor_state(&source, "plan").unwrap().revision,
                operation: generated.operation,
            };
            cad_edit::apply_edit(root, &request).unwrap();
            assert!(cad_check::check_project(root).is_ok());
        }
        let source = cad_model::load_project(root).unwrap();
        assert!(source.drawings[0].entities.iter().any(|r| matches!(&r.entity,
            Entity::Text {value,rotation_deg,..} if value == "面積: 2.00 m²" && *rotation_deg == 30.)));
        let line_id = source.drawings[0]
            .entities
            .iter()
            .find(|r| matches!(r.entity, Entity::Line { .. }))
            .unwrap()
            .entity
            .id()
            .as_str()
            .to_owned();
        let generated = generate(
            &source,
            "plan",
            &GeneratorRequest {
                layer: "0-1".into(),
                pen: None,
                geometry: Geometry::Divide {
                    entity_id: line_id,
                    segments: 5,
                },
            },
        )
        .unwrap();
        let request = cad_edit::DrawingEditRequest {
            drawing: "plan".into(),
            expected_revision: cad_edit::editor_state(&source, "plan").unwrap().revision,
            operation: generated.operation,
        };
        cad_edit::apply_edit(root, &request).unwrap();
        assert!(cad_check::check_project(root).is_ok());
        let mut source = cad_model::load_project(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/cad-acceptance"),
        )
        .unwrap();
        let text_record = source.drawings[0]
            .entities
            .iter_mut()
            .find(|r| matches!(r.entity, Entity::Text { .. }))
            .unwrap();
        let Entity::Text { value, .. } = &mut text_record.entity else {
            panic!()
        };
        *value = "既存壁と既存建具".into();
        let id = text_record.entity.id().as_str().to_owned();
        let generated = replace_text(
            &source,
            &source.drawings[0].name,
            &TextReplaceRequest {
                find: "既存".into(),
                replace: "改修".into(),
                entity_ids: vec![id.clone()],
                style: None,
            },
        )
        .unwrap();
        let EditOperation::Batch { operations } = generated.operation else {
            panic!()
        };
        assert_eq!(operations.len(), 1);
        let EditOperation::Replace { entity_id, entity } = &operations[0] else {
            panic!()
        };
        assert_eq!(entity_id, &id);
        assert_eq!(entity["id"], id);
        assert_eq!(entity["value"], "改修壁と改修建具");
        assert!(
            replace_text(
                &source,
                &source.drawings[0].name,
                &TextReplaceRequest {
                    find: "既存".into(),
                    replace: "改修".into(),
                    entity_ids: vec!["missing".into()],
                    style: None
                }
            )
            .is_err()
        );
    }

    #[test]
    fn bezier_deviation_is_bounded_including_collinear_overshoot() {
        let controls = [[0.0, 0.0], [100.0, 200.0], [-100.0, 200.0], [0.0, 0.0]];
        let mut points = vec![controls[0]];
        flatten_bezier(controls, 0.1, 0, &mut points).unwrap();
        assert!(points.len() > 20);
        assert_eq!(*points.last().unwrap(), controls[3]);
        let mut overshoot = vec![[0.0, 0.0]];
        flatten_bezier(
            [[0.0, 0.0], [200.0, 0.0], [200.0, 0.0], [100.0, 0.0]],
            0.1,
            0,
            &mut overshoot,
        )
        .unwrap();
        assert!(overshoot.iter().any(|p| p[0] > 100.0));
    }
}
