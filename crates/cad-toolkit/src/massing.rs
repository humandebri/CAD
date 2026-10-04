//! Explicit flat-ground prism calculations. Coordinates are east/north/up mm.
use crate::{Result, ToolkitError};
use cad_model::{Entity, Point, ProjectSource};
use geo::{Area, BooleanOps, Contains, Intersects, LineString, MultiPolygon, Polygon, Validation};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
fn invalid(e: impl ToString) -> ToolkitError {
    ToolkitError::Invalid(e.to_string())
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Building {
    pub id: String,
    pub loops: Vec<Vec<Point>>,
    pub base_z_mm: f64,
    pub height_mm: f64,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Analysis {
    Projection {
        origin: Point,
        at: Point,
        yaw_deg: f64,
        elevation_deg: f64,
    },
    SunShadow {
        sun_azimuth_deg: f64,
        sun_altitude_deg: f64,
        ground_z_mm: f64,
    },
    SkyView {
        observer: [f64; 3],
        azimuth_samples: usize,
    },
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema_version: String,
    pub buildings: Vec<Building>,
    pub analysis: Analysis,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    pub building_id: String,
    pub p1: Point,
    pub p2: Point,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ShadowRegion {
    pub loops: Vec<Vec<Point>>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Horizon {
    pub azimuth_deg: f64,
    pub altitude_deg: f64,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Output {
    Projection {
        edges: Vec<Edge>,
    },
    SunShadow {
        regions: Vec<ShadowRegion>,
        area_mm2: f64,
    },
    SkyView {
        horizontal_sky_view_factor: f64,
        coarse_factor: f64,
        refinement_change: f64,
        azimuth_step_deg: f64,
        horizon: Vec<Horizon>,
    },
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema_version: String,
    pub input_blake3: String,
    pub conditions: Request,
    pub coordinate_system: String,
    pub warnings: Vec<String>,
    pub result: Output,
}
struct Prepared {
    building: Building,
    region: MultiPolygon<f64>,
}
fn point_ok(point: Point) -> bool {
    point.iter().all(|v| v.is_finite() && v.abs() <= 1e9)
}
fn prepare(request: &Request) -> Result<Vec<Prepared>> {
    if request.schema_version != "cad-massing/1" || request.buildings.len() > 128 {
        return Err(invalid(
            "Massing needs schema cad-massing/1 and at most 128 prisms",
        ));
    }
    let mut ids = BTreeSet::new();
    let mut vertices = 0;
    request.buildings.iter().map(|source|{
        if source.id.trim().is_empty()||!ids.insert(&source.id)||!source.base_z_mm.is_finite()||source.base_z_mm.abs()>1e7||!source.height_mm.is_finite()||source.height_mm<=0.||source.height_mm>1e7 {return Err(invalid("Prism IDs must be unique; height must be positive; z and height are limited to 10000000 mm"));}
        let mut building=source.clone();
        for points in &mut building.loops {if points.first()==points.last(){points.pop();}vertices+=points.len();if points.len()<3||points.iter().any(|p|!point_ok(*p)){return Err(invalid("Footprints need at least three finite east/north vertices within ±1000000000 mm"));}}
        if vertices>4096{return Err(invalid("Massing footprints exceed 4096 vertices"));}
        if crate::measure::loop_area(&building.loops)?<=0. {return Err(invalid("Prism footprint has no positive area"));}
        let mut region=MultiPolygon(vec![]);
        for points in &building.loops {let polygon=Polygon::new(LineString::from(points.iter().map(|p|(p[0],p[1])).collect::<Vec<_>>()),vec![]);region=region.xor(&MultiPolygon(vec![polygon]));}
        region.check_validation().map_err(invalid)?;
        Ok(Prepared{building,region})
    }).collect()
}
pub fn scene_from_drawing(
    project: &ProjectSource,
    drawing: &str,
    ids: &[String],
    height_mm: f64,
) -> Result<Vec<Building>> {
    if ids.is_empty() || ids.len() > 128 || ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err(invalid("Select 1 to 128 distinct footprint entities"));
    }
    let source = project
        .drawings
        .iter()
        .find(|d| d.name == drawing)
        .ok_or_else(|| invalid("Massing drawing is missing"))?;
    ids.iter()
        .map(|id| {
            let entity = &source
                .entities
                .iter()
                .find(|r| r.entity.id().as_str() == id)
                .ok_or_else(|| invalid("Massing footprint is missing"))?
                .entity;
            let loops = match entity {
                Entity::Polyline {
                    points,
                    closed: true,
                    ..
                }
                | Entity::Solid { points, .. } => vec![points.clone()],
                Entity::Hatch { loops, .. } => loops.clone(),
                _ => {
                    return Err(invalid(format!(
                        "{id}: select closed polylines, solid polygons or hatch boundaries"
                    )));
                }
            };
            Ok(Building {
                id: id.clone(),
                loops,
                base_z_mm: 0.,
                height_mm,
            })
        })
        .collect()
}
pub fn analyze(request: &Request) -> Result<Report> {
    let prepared = prepare(request)?;
    let warnings=vec!["Flat-ground vertical prisms with horizontal roofs; no terrain, roof slopes or hidden-edge removal.".into(),"Results are geometric model calculations, not a statutory compliance assessment. Solar angles are explicit inputs; no date/time/location or atmospheric refraction is inferred.".into(),"Keep the full conditions report for regeneration. Generated CAD entities are ordinary 2D geometry.".into()];
    let result = match &request.analysis {
        Analysis::Projection {
            origin,
            at,
            yaw_deg,
            elevation_deg,
        } => {
            if !point_ok(*origin)
                || !point_ok(*at)
                || !yaw_deg.is_finite()
                || !elevation_deg.is_finite()
                || !(0. ..=90.).contains(elevation_deg)
            {
                return Err(invalid(
                    "Projection needs finite coordinates/yaw and elevation from 0 to 90 degrees",
                ));
            }
            let (s, c) = yaw_deg.rem_euclid(360.).to_radians().sin_cos();
            let (se, ce) = elevation_deg.to_radians().sin_cos();
            let project = |p: Point, z: f64| -> [f64; 2] {
                let x = p[0] - origin[0];
                let y = p[1] - origin[1];
                [at[0] + c * x - s * y, at[1] + se * (s * x + c * y) + ce * z]
            };
            let mut edges = vec![];
            for item in &prepared {
                for ring in &item.building.loops {
                    for i in 0..ring.len() {
                        let p = ring[i];
                        let q = ring[(i + 1) % ring.len()];
                        let z = item.building.base_z_mm;
                        let roof = z + item.building.height_mm;
                        for (p1, p2) in [
                            (project(p, z), project(q, z)),
                            (project(p, roof), project(q, roof)),
                            (project(p, z), project(p, roof)),
                        ] {
                            if p1.iter().chain(p2.iter()).any(|v| !v.is_finite()) {
                                return Err(invalid("Projection coordinates overflow"));
                            }
                            if (p2[0] - p1[0]).hypot(p2[1] - p1[1]) > 1e-8 {
                                edges.push(Edge {
                                    building_id: item.building.id.clone(),
                                    p1,
                                    p2,
                                });
                            }
                        }
                    }
                }
            }
            Output::Projection { edges }
        }
        Analysis::SunShadow {
            sun_azimuth_deg,
            sun_altitude_deg,
            ground_z_mm,
        } => {
            if !sun_azimuth_deg.is_finite()
                || !sun_altitude_deg.is_finite()
                || *sun_altitude_deg <= 0.
                || *sun_altitude_deg > 90.
                || !ground_z_mm.is_finite()
                || ground_z_mm.abs() > 1e7
            {
                return Err(invalid(
                    "Shadow needs finite sun angles, altitude in (0,90], and a finite ground height",
                ));
            }
            let az = sun_azimuth_deg.rem_euclid(360.).to_radians();
            let cot = if *sun_altitude_deg == 90. {
                0.
            } else {
                1. / sun_altitude_deg.to_radians().tan()
            };
            let direction = [-az.sin() * cot, -az.cos() * cot];
            let mut shadow = MultiPolygon(vec![]);
            for item in &prepared {
                if item.building.base_z_mm < *ground_z_mm {
                    return Err(invalid(
                        "Shadow ground must be at or below every prism base",
                    ));
                }
                let low = item.building.base_z_mm - ground_z_mm;
                let high = low + item.building.height_mm;
                let shifted = |p: Point, z: f64| -> Result<Point> {
                    let p = [p[0] + direction[0] * z, p[1] + direction[1] * z];
                    if !point_ok(p) {
                        return Err(invalid(
                            "Shadow exceeds the coordinate range; increase solar altitude or reduce scene extent",
                        ));
                    }
                    Ok(p)
                };
                for z in [low, high] {
                    let mut region = MultiPolygon(vec![]);
                    for ring in &item.building.loops {
                        let points = ring
                            .iter()
                            .map(|p| shifted(*p, z))
                            .collect::<Result<Vec<_>>>()?;
                        let polygon = Polygon::new(
                            LineString::from(
                                points.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>(),
                            ),
                            vec![],
                        );
                        region = region.xor(&MultiPolygon(vec![polygon]));
                    }
                    shadow = shadow.union(&region);
                }
                if cot != 0. {
                    for ring in &item.building.loops {
                        for i in 0..ring.len() {
                            let p = ring[i];
                            let q = ring[(i + 1) % ring.len()];
                            let points = [
                                shifted(p, low)?,
                                shifted(q, low)?,
                                shifted(q, high)?,
                                shifted(p, high)?,
                            ];
                            let polygon = Polygon::new(
                                LineString::from(
                                    points.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>(),
                                ),
                                vec![],
                            );
                            if polygon.unsigned_area() > 0. {
                                polygon.check_validation().map_err(invalid)?;
                                shadow = shadow.union(&MultiPolygon(vec![polygon]));
                            }
                        }
                    }
                }
            }
            shadow.check_validation().map_err(invalid)?;
            let area_mm2 = shadow.unsigned_area();
            if !area_mm2.is_finite() {
                return Err(invalid("Shadow area overflow"));
            }
            let regions = shadow
                .0
                .iter()
                .map(|polygon| ShadowRegion {
                    loops: std::iter::once(polygon.exterior())
                        .chain(polygon.interiors())
                        .map(|ring| ring.0.iter().map(|p| [p.x, p.y]).collect())
                        .collect(),
                })
                .collect();
            Output::SunShadow { regions, area_mm2 }
        }
        Analysis::SkyView {
            observer,
            azimuth_samples,
        } => {
            if !point_ok([observer[0], observer[1]])
                || !observer[2].is_finite()
                || observer[2].abs() > 1e7
                || !(36..=2048).contains(azimuth_samples)
            {
                return Err(invalid(
                    "Sky view needs a finite observer and 36 to 2048 coarse azimuth samples",
                ));
            }
            let vertices = prepared
                .iter()
                .flat_map(|p| &p.building.loops)
                .map(Vec::len)
                .sum::<usize>();
            if vertices.saturating_mul(*azimuth_samples).saturating_mul(3) > 8_000_000 {
                return Err(invalid(
                    "Sky-view ray budget exceeded; reduce footprints or samples",
                ));
            }
            let observer_xy = geo::Point::new(observer[0], observer[1]);
            for item in &prepared {
                if item.building.base_z_mm > observer[2] {
                    return Err(invalid(
                        "Sky horizons require prism bases at or below observer height; floating overhangs are unsupported",
                    ));
                }
                if observer[2] < item.building.base_z_mm + item.building.height_mm
                    && item.region.intersects(&observer_xy)
                {
                    return Err(invalid(
                        "Sky observer is inside or on a prism footprint below its roof",
                    ));
                }
            }
            let compute = |samples: usize| -> Result<(f64, Vec<Horizon>)> {
                let mut horizon = Vec::with_capacity(samples);
                let mut sum = 0.;
                for bin in 0..samples {
                    let azimuth_deg = (bin as f64 + 0.5) * 360. / samples as f64;
                    let az = azimuth_deg.to_radians();
                    let direction = [az.sin(), az.cos()];
                    let mut altitude: f64 = 0.;
                    for item in &prepared {
                        let rise = item.building.base_z_mm + item.building.height_mm - observer[2];
                        if rise <= 0. {
                            continue;
                        }
                        if let Some(distance) =
                            entry_distance(item, [observer[0], observer[1]], direction)?
                        {
                            altitude = altitude.max(rise.atan2(distance));
                        }
                    }
                    sum += altitude.cos().powi(2);
                    horizon.push(Horizon {
                        azimuth_deg,
                        altitude_deg: altitude.to_degrees(),
                    });
                }
                Ok((sum / samples as f64, horizon))
            };
            let (coarse_factor, _) = compute(*azimuth_samples)?;
            let (horizontal_sky_view_factor, horizon) = compute(azimuth_samples * 2)?;
            Output::SkyView {
                horizontal_sky_view_factor,
                coarse_factor,
                refinement_change: (horizontal_sky_view_factor - coarse_factor).abs(),
                azimuth_step_deg: 180. / *azimuth_samples as f64,
                horizon,
            }
        }
    };
    Ok(Report {
        schema_version: "cad-massing-result/1".into(),
        input_blake3: blake3::hash(&serde_json::to_vec(request)?)
            .to_hex()
            .to_string(),
        conditions: request.clone(),
        coordinate_system:
            "model millimetres: x east, y north, z up; solar azimuth north=0°, east=90°".into(),
        warnings,
        result,
    })
}
fn cross(a: Point, b: Point) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn entry_distance(item: &Prepared, observer: Point, direction: Point) -> Result<Option<f64>> {
    let mut hits = vec![];
    for ring in &item.building.loops {
        for i in 0..ring.len() {
            let a = ring[i];
            let b = ring[(i + 1) % ring.len()];
            let edge = [b[0] - a[0], b[1] - a[1]];
            let offset = [a[0] - observer[0], a[1] - observer[1]];
            let det = cross(direction, edge);
            if det.abs() <= 1e-12 * edge[0].hypot(edge[1]) {
                continue;
            }
            let t = cross(offset, edge) / det;
            let u = cross(offset, direction) / det;
            if !t.is_finite() || !u.is_finite() {
                return Err(invalid("Sky ray intersection overflow"));
            }
            if t > 0. && (0. ..=1.).contains(&u) {
                hits.push(t);
            }
        }
    }
    hits.sort_by(f64::total_cmp);
    hits.dedup_by(|a, b| (*a - *b).abs() <= 1e-9);
    for pair in hits.windows(2) {
        let t = (pair[0] + pair[1]) / 2.;
        let p = geo::Point::new(
            observer[0] + direction[0] * t,
            observer[1] + direction[1] * t,
        );
        if item.region.contains(&p) {
            return Ok(Some(pair[0]));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn concentric_polygon_courtyard_agrees_with_constant_horizon_limit() {
        let ring = |radius: f64| {
            (0..360)
                .map(|i| {
                    let a = (i as f64).to_radians();
                    [radius * a.sin(), radius * a.cos()]
                })
                .collect()
        };
        let req = Request {
            schema_version: "cad-massing/1".into(),
            buildings: vec![Building {
                id: "courtyard".into(),
                loops: vec![ring(20.), ring(10.)],
                base_z_mm: 0.,
                height_mm: 10.,
            }],
            analysis: Analysis::SkyView {
                observer: [0., 0., 0.],
                azimuth_samples: 360,
            },
        };
        let Output::SkyView {
            horizontal_sky_view_factor,
            horizon,
            ..
        } = analyze(&req).unwrap().result
        else {
            panic!()
        };
        // A circular courtyard with height = radius has a 45° horizon and SVF 0.5.
        assert!((horizontal_sky_view_factor - 0.5).abs() < 0.0001);
        assert!(horizon.iter().all(|h| (h.altitude_deg - 45.).abs() < 0.01));
        let mut floating = req;
        floating.buildings[0].base_z_mm = 1.;
        assert!(
            analyze(&floating)
                .unwrap_err()
                .to_string()
                .contains("floating overhangs")
        );
    }
    fn square(min: f64, max: f64) -> Vec<Point> {
        vec![[min, min], [max, min], [max, max], [min, max]]
    }
    fn building() -> Building {
        Building {
            id: "room".into(),
            loops: vec![square(0., 4.)],
            base_z_mm: 0.,
            height_mm: 5.,
        }
    }
    fn request(analysis: Analysis) -> Request {
        Request {
            schema_version: "cad-massing/1".into(),
            buildings: vec![building()],
            analysis,
        }
    }
    #[test]
    fn shadow_matches_analytic_rectangles_and_zenith_preserves_holes() {
        for az in [0., 90., 180., 270.] {
            let report = analyze(&request(Analysis::SunShadow {
                sun_azimuth_deg: az,
                sun_altitude_deg: 45.,
                ground_z_mm: 0.,
            }))
            .unwrap();
            let Output::SunShadow { area_mm2, .. } = report.result else {
                panic!()
            };
            assert!((area_mm2 - 36.).abs() < 1e-6);
        }
        let mut req = request(Analysis::SunShadow {
            sun_azimuth_deg: 0.,
            sun_altitude_deg: 90.,
            ground_z_mm: 0.,
        });
        req.buildings[0].loops.push(square(1., 3.));
        let Output::SunShadow { regions, area_mm2 } = analyze(&req).unwrap().result else {
            panic!()
        };
        assert_eq!(area_mm2, 12.);
        assert_eq!(regions[0].loops.len(), 2);
    }
    #[test]
    fn overlaps_union_and_concave_footprints_remain_valid() {
        let mut req = request(Analysis::SunShadow {
            sun_azimuth_deg: 90.,
            sun_altitude_deg: 45.,
            ground_z_mm: 0.,
        });
        let mut duplicate = building();
        duplicate.id = "second".into();
        req.buildings.push(duplicate);
        let Output::SunShadow { area_mm2, .. } = analyze(&req).unwrap().result else {
            panic!()
        };
        assert!((area_mm2 - 36.).abs() < 1e-6);
        req.buildings.truncate(1);
        req.buildings[0].loops = vec![vec![
            [0., 0.],
            [4., 0.],
            [4., 2.],
            [2., 2.],
            [2., 4.],
            [0., 4.],
        ]];
        assert!(analyze(&req).is_ok());
    }
    #[test]
    fn projection_plan_and_elevation_have_expected_heights_and_no_zero_edges() {
        let report = analyze(&request(Analysis::Projection {
            origin: [0., 0.],
            at: [100., 200.],
            yaw_deg: 0.,
            elevation_deg: 0.,
        }))
        .unwrap();
        let Output::Projection { edges } = report.result else {
            panic!()
        };
        assert!(
            edges
                .iter()
                .any(|e| e.p1 == [100., 200.] && e.p2 == [100., 205.])
        );
        assert!(edges.iter().all(|e| e.p1 != e.p2));
        let report = analyze(&request(Analysis::Projection {
            origin: [0., 0.],
            at: [0., 0.],
            yaw_deg: 90.,
            elevation_deg: 90.,
        }))
        .unwrap();
        let Output::Projection { edges } = report.result else {
            panic!()
        };
        assert_eq!(edges.len(), 8);
    }
    #[test]
    fn open_sky_obstruction_and_hole_observers_have_explicit_sampling() {
        let mut req = request(Analysis::SkyView {
            observer: [-5., 2., 0.],
            azimuth_samples: 360,
        });
        let report = analyze(&req).unwrap();
        let Output::SkyView {
            horizontal_sky_view_factor,
            horizon,
            azimuth_step_deg,
            ..
        } = report.result
        else {
            panic!()
        };
        assert!(horizontal_sky_view_factor > 0.8 && horizontal_sky_view_factor < 1.);
        assert_eq!(horizon.len(), 720);
        assert_eq!(azimuth_step_deg, 0.5);
        assert!(horizon.iter().any(|h| h.altitude_deg > 0.));
        req.buildings.clear();
        let Output::SkyView {
            horizontal_sky_view_factor,
            ..
        } = analyze(&req).unwrap().result
        else {
            panic!()
        };
        assert_eq!(horizontal_sky_view_factor, 1.);
        req.buildings.push(building());
        req.analysis = Analysis::SkyView {
            observer: [2., 2., 0.],
            azimuth_samples: 180,
        };
        assert!(analyze(&req).is_err());
        req.buildings[0].loops.push(square(1., 3.));
        assert!(analyze(&req).is_ok());
        req.analysis = Analysis::SkyView {
            observer: [2., 2., 6.],
            azimuth_samples: 180,
        };
        let Output::SkyView {
            horizontal_sky_view_factor,
            ..
        } = analyze(&req).unwrap().result
        else {
            panic!()
        };
        assert_eq!(horizontal_sky_view_factor, 1.);
    }
    #[test]
    fn invalid_geometry_angles_ids_and_unsupported_ground_are_rejected() {
        let mut req = request(Analysis::SunShadow {
            sun_azimuth_deg: 0.,
            sun_altitude_deg: 0.,
            ground_z_mm: 0.,
        });
        assert!(analyze(&req).is_err());
        req.analysis = Analysis::SunShadow {
            sun_azimuth_deg: 0.,
            sun_altitude_deg: 45.,
            ground_z_mm: 1.,
        };
        assert!(analyze(&req).is_err());
        req.analysis = Analysis::Projection {
            origin: [0., 0.],
            at: [0., 0.],
            yaw_deg: 0.,
            elevation_deg: 45.,
        };
        req.buildings.push(building());
        assert!(analyze(&req).is_err());
        req.buildings.truncate(1);
        req.buildings[0].loops = vec![vec![[0., 0.], [4., 4.], [0., 4.], [4., 0.]]];
        assert!(analyze(&req).is_err());
        req.buildings[0] = building();
        req.buildings[0].height_mm = f64::NAN;
        assert!(analyze(&req).is_err());
    }
}
