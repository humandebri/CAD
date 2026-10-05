//! Architectural checks outside the CAD model. All semantic bindings are explicit.
use cad_model::{Entity, Point, ProjectSource};
use geo::{Area, BooleanOps, BoundingRect, LineString, MultiPolygon, Polygon, Validation};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const SCHEMA: &str = "cad-plan-audit/1";
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct Error(pub String);
pub type Result<T> = std::result::Result<T, Error>;
fn invalid(s: impl Into<String>) -> Error {
    Error(s.into())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    pub schema_version: String,
    /// Geometric tolerance and maximum chord deviation, in model millimetres.
    pub tolerance_mm: f64,
    pub drawings: Vec<DrawingLedger>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrawingLedger {
    pub drawing: String,
    pub walls: WallBinding,
    #[serde(default)]
    pub rooms: Vec<Room>,
    #[serde(default)]
    pub fixtures: Vec<Fixture>,
    #[serde(default)]
    pub openings: Vec<Opening>,
    #[serde(default)]
    pub doors: Vec<Door>,
    #[serde(default)]
    pub clearances: Vec<Clearance>,
    #[serde(default)]
    pub installations: Vec<Installation>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WallBinding {
    pub fill_entity_ids: Vec<String>,
    /// All wall boundary polylines; required only for repair proposals.
    #[serde(default)]
    pub outline_entity_ids: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Room {
    pub key: String,
    pub inner_polygon: Vec<Point>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Product {
    pub name: String,
    pub model: String,
    pub source: String,
    #[serde(default)]
    pub nominal_width_mm: Option<f64>,
    #[serde(default)]
    pub nominal_height_mm: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fixture {
    pub key: String,
    /// Tight union bounding rectangle of explicitly bound entities. No inference.
    pub entity_ids: Vec<String>,
    #[serde(default)]
    pub room: Option<String>,
    #[serde(default)]
    pub wall_mounted: bool,
    #[serde(default)]
    pub product: Option<Product>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Opening {
    pub key: String,
    pub axis: [Point; 2],
    pub depth_mm: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClosedEnd {
    Start,
    End,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Door {
    pub key: String,
    pub opening: String,
    pub swing_entity_id: String,
    pub closed_end: ClosedEnd,
    #[serde(default)]
    pub product: Option<Product>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Clearance {
    Between {
        key: String,
        first: String,
        second: String,
        axis: Axis,
        min_mm: f64,
        basis: String,
    },
    RoomEdge {
        key: String,
        fixture: String,
        room: String,
        side: Side,
        min_mm: f64,
        basis: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Axis {
    X,
    Y,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    MinX,
    MaxX,
    MinY,
    MaxY,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Installation {
    pub key: String,
    pub fixture: String,
    pub room: String,
    pub source: String,
    #[serde(default)]
    pub required_width_mm: Option<f64>,
    #[serde(default)]
    pub required_depth_mm: Option<f64>,
    #[serde(default)]
    pub required_height_mm: Option<f64>,
    /// Explicitly measured installation height, not room ceiling height.
    #[serde(default)]
    pub available_height_mm: Option<f64>,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    Fail,
    Unknown,
}
#[derive(Clone, Debug, Serialize)]
pub struct Check {
    pub drawing: String,
    pub object: String,
    pub code: String,
    pub status: Status,
    pub message: String,
    pub position: Point,
    pub measured: Option<f64>,
    pub required: Option<f64>,
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: String,
    pub status: Status,
    pub checks: Vec<Check>,
    pub coverage: Vec<Coverage>,
    pub scope: String,
}
#[derive(Debug, Serialize)]
pub struct Coverage {
    pub drawing: String,
    pub rooms: usize,
    pub fixtures: usize,
    pub doors: usize,
    pub openings: usize,
    pub wall_regions: usize,
}
#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct Repair {
    pub drawing: String,
    pub door: String,
    pub opening: String,
    pub old_axis: [Point; 2],
    pub new_axis: [Point; 2],
    pub depth_mm: f64,
    pub available: bool,
    pub reason: String,
}
#[derive(Debug)]
pub struct Analysis {
    pub report: Report,
    pub repairs: Vec<Repair>,
}

fn distance(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}
fn finite_point(p: Point) -> bool {
    p.iter().all(|x| x.is_finite())
}
pub fn polygon(points: &[Point]) -> Result<Polygon<f64>> {
    if points.len() < 3 || points.iter().any(|p| !finite_point(*p)) {
        return Err(invalid("polygon requires finite points"));
    }
    let mut ring = points.to_vec();
    if ring.first() != ring.last() {
        ring.push(ring[0]);
    }
    let p = Polygon::new(
        LineString::from(ring.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>()),
        vec![],
    );
    if !p.is_valid() || p.unsigned_area() <= 0.0 {
        return Err(invalid("polygon must be simple and have positive area"));
    }
    Ok(p)
}
fn hatch(e: &Entity) -> Result<MultiPolygon<f64>> {
    let Entity::Hatch { loops, .. } = e else {
        return Err(invalid("wall fill binding must reference a hatch"));
    };
    if loops.is_empty() {
        return Err(invalid("empty wall hatch"));
    }
    // Canonical hatches use even-odd fill, independent of loop order or winding.
    let mut result = MultiPolygon(vec![]);
    for points in loops {
        result = result.xor(&MultiPolygon(vec![polygon(points)?]));
    }
    if !result.is_valid() {
        return Err(invalid("invalid wall hatch regions"));
    }
    Ok(result)
}
fn entity<'a>(map: &BTreeMap<String, &'a Entity>, id: &str) -> Result<&'a Entity> {
    map.get(id)
        .copied()
        .ok_or_else(|| invalid(format!("missing bound entity {id}")))
}
fn wall_geometry(map: &BTreeMap<String, &Entity>, wall: &WallBinding) -> Result<MultiPolygon<f64>> {
    if wall.fill_entity_ids.is_empty() {
        return Err(invalid("wall fills must be bound explicitly"));
    }
    let mut unique = BTreeSet::new();
    for id in wall.fill_entity_ids.iter().chain(&wall.outline_entity_ids) {
        if !unique.insert(id) {
            return Err(invalid("duplicate wall entity binding"));
        }
    }
    let mut result = MultiPolygon(vec![]);
    for id in &wall.fill_entity_ids {
        result = result.union(&hatch(entity(map, id)?)?);
    }
    for id in &wall.outline_entity_ids {
        if !matches!(entity(map, id)?, Entity::Polyline { closed: true, .. }) {
            return Err(invalid("wall outline binding requires a closed polyline"));
        }
    }
    Ok(result)
}
fn fixture_geometry(map: &BTreeMap<String, &Entity>, fixture: &Fixture) -> Result<Polygon<f64>> {
    if fixture.entity_ids.is_empty() {
        return Err(invalid(format!(
            "fixture {} has no entity bindings",
            fixture.key
        )));
    }
    let mut min = [f64::INFINITY; 2];
    let mut max = [f64::NEG_INFINITY; 2];
    for id in &fixture.entity_ids {
        let e = entity(map, id)?;
        if matches!(
            e,
            Entity::Text { .. } | Entity::Dimension { .. } | Entity::BlockRef { .. }
        ) {
            return Err(invalid(
                "fixture bounds require expanded geometry, not annotations or block refs",
            ));
        }
        let b = cad_model::entity_bbox(e)
            .ok_or_else(|| invalid(format!("entity {id} has no finite bounds")))?;
        for i in 0..2 {
            min[i] = min[i].min(b.min[i]);
            max[i] = max[i].max(b.max[i]);
        }
    }
    polygon(&[min, [max[0], min[1]], max, [min[0], max[1]]])
}
/// Drawing-space bounds used by both checks and regenerated schedules.
pub fn fixture_bounds(
    project: &ProjectSource,
    drawing: &str,
    fixture: &Fixture,
) -> Result<[Point; 2]> {
    let source = project
        .drawings
        .iter()
        .find(|d| d.name == drawing)
        .ok_or_else(|| invalid("missing fixture drawing"))?;
    let map = source
        .entities
        .iter()
        .map(|r| (r.entity.id().as_str().to_owned(), &r.entity))
        .collect();
    let b = fixture_geometry(&map, fixture)?.bounding_rect().unwrap();
    Ok([[b.min().x, b.min().y], [b.max().x, b.max().y]])
}
pub fn opening_polygon(o: &Opening) -> Result<Polygon<f64>> {
    let [a, b] = o.axis;
    let len = distance(a, b);
    if !finite_point(a)
        || !finite_point(b)
        || !len.is_finite()
        || len <= 0.0
        || !o.depth_mm.is_finite()
        || o.depth_mm <= 0.0
    {
        return Err(invalid("invalid opening axis or depth"));
    }
    let n = [
        -(b[1] - a[1]) * o.depth_mm / (2.0 * len),
        (b[0] - a[0]) * o.depth_mm / (2.0 * len),
    ];
    polygon(&[
        [a[0] + n[0], a[1] + n[1]],
        [b[0] + n[0], b[1] + n[1]],
        [b[0] - n[0], b[1] - n[1]],
        [a[0] - n[0], a[1] - n[1]],
    ])
}
// A declared empty rectangle is only an opening if wall material bounds both ends.
fn opening_jambs(walls: &MultiPolygon<f64>, o: &Opening, tol: f64) -> Result<bool> {
    let [a, b] = o.axis;
    let len = distance(a, b);
    let u = [(b[0] - a[0]) / len, (b[1] - a[1]) / len];
    let probe = (o.depth_mm / 4.0).min(10.0);
    for axis in [
        [[a[0] - u[0] * probe, a[1] - u[1] * probe], a],
        [b, [b[0] + u[0] * probe, b[1] + u[1] * probe]],
    ] {
        let cap = opening_polygon(&Opening { axis, ..o.clone() })?;
        if MultiPolygon(vec![cap]).difference(walls).unsigned_area() > tol * tol {
            return Ok(false);
        }
    }
    Ok(true)
}
fn arc(e: &Entity) -> Result<(Point, f64, f64, f64)> {
    if let Entity::Arc {
        center,
        radius,
        start_deg,
        end_deg,
        ..
    } = e
    {
        Ok((*center, *radius, *start_deg, *end_deg))
    } else {
        Err(invalid("door swing binding must reference an arc"))
    }
}
fn swing(c: Point, r: f64, start: f64, end: f64, tolerance: f64) -> Result<Polygon<f64>> {
    let step = (2.0 * (1.0 - (tolerance / r).min(1.0)).acos()).to_degrees();
    let steps = ((end - start).abs() / step).ceil().max(2.0) as usize;
    if steps > 8192 {
        return Err(invalid(
            "door swing requires more than 8192 samples; increase tolerance",
        ));
    }
    if (end - start).abs() > 180.0 {
        return Err(invalid("hinged door sweep must not exceed 180 degrees"));
    }
    let mut points = vec![c];
    for i in 0..=steps {
        let a = (start + (end - start) * (i as f64 / steps as f64)).to_radians();
        points.push([c[0] + r * a.cos(), c[1] + r * a.sin()]);
    }
    polygon(&points)
}
fn center(p: &Polygon<f64>) -> Point {
    let b = p.bounding_rect().unwrap();
    [(b.min().x + b.max().x) / 2.0, (b.min().y + b.max().y) / 2.0]
}
fn area_overlap(a: &Polygon<f64>, b: &Polygon<f64>) -> f64 {
    a.intersection(b).unsigned_area()
}
fn metric(v: Option<f64>, required: Option<f64>, tol: f64) -> Status {
    match (v, required) {
        (Some(a), Some(b)) if a + tol >= b => Status::Pass,
        (Some(_), Some(_)) => Status::Fail,
        _ => Status::Unknown,
    }
}
fn positive(v: f64) -> Result<()> {
    if v.is_finite() && v > 0.0 {
        Ok(())
    } else {
        Err(invalid("dimensions and limits must be finite and positive"))
    }
}

pub fn analyze(project: &ProjectSource, ledger: &Ledger) -> Result<Analysis> {
    if ledger.schema_version != SCHEMA {
        return Err(invalid("unsupported audit ledger schema"));
    }
    positive(ledger.tolerance_mm)?;
    if ledger.tolerance_mm > 5.0 {
        return Err(invalid("tolerance must not exceed 5mm"));
    }
    let tol = ledger.tolerance_mm;
    let mut checks = vec![];
    let mut repairs = vec![];
    let mut coverage = vec![];
    let mut drawings = BTreeSet::new();
    for d in &ledger.drawings {
        if !drawings.insert(&d.drawing) {
            return Err(invalid("duplicate drawing ledger"));
        }
        let source = project
            .drawings
            .iter()
            .find(|x| x.name == d.drawing)
            .ok_or_else(|| invalid(format!("missing drawing {}", d.drawing)))?;
        let map = source
            .entities
            .iter()
            .map(|r| (r.entity.id().as_str().to_owned(), &r.entity))
            .collect::<BTreeMap<_, _>>();
        let walls = wall_geometry(&map, &d.walls)?;
        let mut keys = BTreeSet::new();
        for key in d
            .rooms
            .iter()
            .map(|o| &o.key)
            .chain(d.fixtures.iter().map(|o| &o.key))
            .chain(d.openings.iter().map(|o| &o.key))
            .chain(d.doors.iter().map(|o| &o.key))
        {
            if key.is_empty() || !keys.insert(key) {
                return Err(invalid("empty or duplicate object key"));
            }
        }
        for product in d
            .fixtures
            .iter()
            .filter_map(|f| f.product.as_ref())
            .chain(d.doors.iter().filter_map(|f| f.product.as_ref()))
        {
            for v in [product.nominal_width_mm, product.nominal_height_mm]
                .into_iter()
                .flatten()
            {
                positive(v)?;
            }
        }
        let rooms = d
            .rooms
            .iter()
            .map(|o| Ok((o.key.clone(), polygon(&o.inner_polygon)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        let fixtures = d
            .fixtures
            .iter()
            .map(|o| Ok((o.key.clone(), fixture_geometry(&map, o)?)))
            .collect::<Result<BTreeMap<_, _>>>()?;
        let mut add = |key: &str,
                       code: &str,
                       status: Status,
                       message: String,
                       position: Point,
                       measured: Option<f64>,
                       required: Option<f64>| {
            checks.push(Check {
                drawing: d.drawing.clone(),
                object: key.into(),
                code: code.into(),
                status,
                message,
                position,
                measured,
                required,
            })
        };
        for room in &d.rooms {
            let region = &rooms[&room.key];
            let area = walls
                .intersection(&MultiPolygon(vec![region.clone()]))
                .unsigned_area();
            add(&room.key,"room.wall_overlap",if area>tol*tol {Status::Fail}else{Status::Pass},
                "Room inner polygon intersects registered wall material; update stale room bindings.".into(),center(region),Some(area),Some(0.0));
        }
        for f in &d.fixtures {
            let region = &fixtures[&f.key];
            let pos = center(region);
            if !f.wall_mounted {
                let area = walls
                    .intersection(&MultiPolygon(vec![region.clone()]))
                    .unsigned_area();
                add(
                    &f.key,
                    "fixture.wall_overlap",
                    if area > tol * tol {
                        Status::Fail
                    } else {
                        Status::Pass
                    },
                    "Explicit fixture bounding rectangle versus bound wall fills (mm²).".into(),
                    pos,
                    Some(area),
                    Some(0.0),
                );
            }
            if let Some(room) = &f.room {
                let room = rooms
                    .get(room)
                    .ok_or_else(|| invalid("fixture references missing room"))?;
                let area = region.difference(room).unsigned_area();
                add(
                    &f.key,
                    "fixture.room_containment",
                    if area > tol * tol {
                        Status::Fail
                    } else {
                        Status::Pass
                    },
                    "Fixture bounding rectangle outside explicit room inner polygon (mm²).".into(),
                    pos,
                    Some(area),
                    Some(0.0),
                );
            }
        }
        for (i, a) in d.fixtures.iter().enumerate() {
            for b in &d.fixtures[i + 1..] {
                let area = area_overlap(&fixtures[&a.key], &fixtures[&b.key]);
                add(&format!("{} / {}",a.key,b.key),"fixture.fixture_overlap",if area>tol*tol{Status::Fail}else{Status::Pass},"Conservative bounding rectangles overlap (mm²); inspect nonrectangular products.".into(),center(&fixtures[&a.key]),Some(area),Some(0.0));
            }
        }
        for o in &d.openings {
            let region = opening_polygon(o)?;
            add(&o.key,"opening.wall_attachment",if opening_jambs(&walls,o,tol)? {Status::Pass}else{Status::Fail},
                "Both opening ends must have registered wall material across the declared wall thickness.".into(),center(&region),None,None);
            let area = walls
                .intersection(&MultiPolygon(vec![region.clone()]))
                .unsigned_area();
            add(
                &o.key,
                "opening.wall_overlap",
                if area > tol * tol {
                    Status::Fail
                } else {
                    Status::Pass
                },
                "Declared opening versus current CAD wall fills (mm²).".into(),
                center(&region),
                Some(area),
                Some(0.0),
            );
        }
        let mut door_sectors = Vec::new();
        for door in &d.doors {
            let opening = d
                .openings
                .iter()
                .find(|o| o.key == door.opening)
                .ok_or_else(|| invalid("door references missing opening"))?;
            let (c, r, start, end) = arc(entity(&map, &door.swing_entity_id)?)?;
            let closed = match door.closed_end {
                ClosedEnd::Start => start,
                ClosedEnd::End => end,
            }
            .to_radians();
            let tip = [c[0] + r * closed.cos(), c[1] + r * closed.sin()];
            let [a, b] = opening.axis;
            let length = distance(a, b);
            let unit = [(b[0] - a[0]) / length, (b[1] - a[1]) / length];
            let projection = |p: Point| (p[0] - a[0]) * unit[0] + (p[1] - a[1]) * unit[1];
            let normal = |p: Point| -(p[0] - a[0]) * unit[1] + (p[1] - a[1]) * unit[0];
            let lo = projection(c).min(projection(tip));
            let hi = projection(c).max(projection(tip));
            let aligned = (normal(c) - normal(tip)).abs() <= tol;
            let mismatch = lo
                .abs()
                .max((hi - length).abs())
                .max((normal(c).abs() - opening.depth_mm / 2.0).max(0.0));
            add(&door.key,"door.opening_alignment",if aligned&&mismatch<=tol{Status::Pass}else{Status::Fail},"Closed-leaf projection must match the explicitly bound opening; frame allowance is not inferred.".into(),c,Some(mismatch),Some(tol));
            let sector = swing(c, r, start, end, tol)?;
            door_sectors.push((&door.key, sector.clone()));
            let area = walls
                .intersection(&MultiPolygon(vec![sector.clone()]))
                .unsigned_area();
            add(
                &door.key,
                "door.swing_wall_overlap",
                if area > tol * tol {
                    Status::Fail
                } else {
                    Status::Pass
                },
                format!(
                    "Swing sector sampled with at most {tol}mm chord deviation; wall overlap mm²."
                ),
                c,
                Some(area),
                Some(0.0),
            );
            for (key, fixture) in &fixtures {
                let area = area_overlap(&sector, fixture);
                add(
                    &format!("{} / {key}", door.key),
                    "door.swing_fixture_overlap",
                    if area > tol * tol {
                        Status::Fail
                    } else {
                        Status::Pass
                    },
                    "Swing versus conservative fixture bounds (mm²).".into(),
                    c,
                    Some(area),
                    Some(0.0),
                );
            }
            if !aligned || mismatch > tol {
                let axis = [
                    [a[0] + lo * unit[0], a[1] + lo * unit[1]],
                    [a[0] + hi * unit[0], a[1] + hi * unit[1]],
                ];
                let new = Opening {
                    axis,
                    ..opening.clone()
                };
                let new_slot = opening_polygon(&new)?;
                let old_slot = opening_polygon(opening)?;
                let material = walls.union(&MultiPolygon(vec![old_slot]));
                let coverage_area = material
                    .intersection(&MultiPolygon(vec![new_slot.clone()]))
                    .unsigned_area();
                let occupied_by_other = d
                    .openings
                    .iter()
                    .filter(|o| o.key != opening.key)
                    .map(opening_polygon)
                    .collect::<Result<Vec<_>>>()?
                    .iter()
                    .any(|p| area_overlap(p, &new_slot) > tol * tol);
                let old_clear = walls
                    .intersection(&MultiPolygon(vec![opening_polygon(opening)?]))
                    .unsigned_area()
                    <= tol * tol;
                let available = old_clear
                    && opening_jambs(&walls, opening, tol)?
                    && aligned
                    && normal(c).abs() <= opening.depth_mm / 2.0 + tol
                    && !occupied_by_other
                    && new_slot.unsigned_area() - coverage_area <= tol * tol
                    && !d.walls.outline_entity_ids.is_empty();
                repairs.push(Repair{drawing:d.drawing.clone(),door:door.key.clone(),opening:opening.key.clone(),old_axis:opening.axis,new_axis:axis,depth_mm:opening.depth_mm,available,reason:if available{"Close old slot and cut new slot in the explicitly bound straight wall. Review candidate and ledger patch."}else{"No automatic candidate: incompatible direction/offset, insufficient wall material, another opening, or missing outlines."}.into()});
            }
        }
        for (i, (first, a)) in door_sectors.iter().enumerate() {
            for (second, b) in &door_sectors[i + 1..] {
                let area = area_overlap(a, b);
                add(
                    &format!("{first} / {second}"),
                    "door.swing_door_overlap",
                    if area > tol * tol {
                        Status::Fail
                    } else {
                        Status::Pass
                    },
                    "Registered door swing sectors overlap; simultaneous opening needs review."
                        .into(),
                    center(a),
                    Some(area),
                    Some(0.0),
                );
            }
        }
        for rule in &d.clearances {
            let (key, measured, required, basis, pos) = match rule {
                Clearance::Between {
                    key,
                    first,
                    second,
                    axis,
                    min_mm,
                    basis,
                } => {
                    let a = fixtures
                        .get(first)
                        .ok_or_else(|| invalid("clearance references missing fixture"))?
                        .bounding_rect()
                        .unwrap();
                    let b = fixtures
                        .get(second)
                        .ok_or_else(|| invalid("clearance references missing fixture"))?
                        .bounding_rect()
                        .unwrap();
                    let (lo1, hi1, lo2, hi2, cross) = match axis {
                        Axis::X => (
                            a.min().x,
                            a.max().x,
                            b.min().x,
                            b.max().x,
                            a.min().y <= b.max().y && b.min().y <= a.max().y,
                        ),
                        Axis::Y => (
                            a.min().y,
                            a.max().y,
                            b.min().y,
                            b.max().y,
                            a.min().x <= b.max().x && b.min().x <= a.max().x,
                        ),
                    };
                    (
                        key,
                        if cross {
                            Some((lo1 - hi2).max(lo2 - hi1).max(0.0))
                        } else {
                            None
                        },
                        *min_mm,
                        basis,
                        [a.min().x, a.min().y],
                    )
                }
                Clearance::RoomEdge {
                    key,
                    fixture,
                    room,
                    side,
                    min_mm,
                    basis,
                } => {
                    let f = fixtures
                        .get(fixture)
                        .ok_or_else(|| invalid("clearance references missing fixture"))?
                        .bounding_rect()
                        .unwrap();
                    let region = rooms
                        .get(room)
                        .ok_or_else(|| invalid("clearance references missing room"))?;
                    let b = region.bounding_rect().unwrap();
                    let rectangular =
                        (region.unsigned_area() - b.width() * b.height()).abs() <= tol * tol;
                    let v = match side {
                        Side::MinX => f.min().x - b.min().x,
                        Side::MaxX => b.max().x - f.max().x,
                        Side::MinY => f.min().y - b.min().y,
                        Side::MaxY => b.max().y - f.max().y,
                    };
                    (
                        key,
                        if rectangular { Some(v) } else { None },
                        *min_mm,
                        basis,
                        [f.min().x, f.min().y],
                    )
                }
            };
            positive(required)?;
            add(
                key,
                "clearance.minimum",
                if basis.trim().is_empty() {
                    Status::Unknown
                } else {
                    metric(measured, Some(required), tol)
                },
                format!("{basis}; explicit rule, not an inferred building standard."),
                pos,
                measured,
                Some(required),
            );
        }
        for install in &d.installations {
            let fixture = fixtures
                .get(&install.fixture)
                .ok_or_else(|| invalid("installation references missing fixture"))?;
            let binding = d
                .fixtures
                .iter()
                .find(|f| f.key == install.fixture)
                .unwrap();
            if binding
                .room
                .as_ref()
                .is_some_and(|room| room != &install.room)
            {
                return Err(invalid(
                    "installation room conflicts with fixture room binding",
                ));
            }
            let room = rooms
                .get(&install.room)
                .ok_or_else(|| invalid("installation references missing room"))?;
            if fixture.difference(room).unsigned_area() > tol * tol {
                return Err(invalid(
                    "installation fixture is outside the specified room",
                ));
            }
            let b = room.bounding_rect().unwrap();
            let rectangular = (room.unsigned_area() - b.width() * b.height()).abs() <= tol * tol
                && walls
                    .intersection(&MultiPolygon(vec![room.clone()]))
                    .unsigned_area()
                    <= tol * tol;
            for (axis, measured, required) in [
                (
                    "width",
                    if rectangular { Some(b.width()) } else { None },
                    install.required_width_mm,
                ),
                (
                    "depth",
                    if rectangular { Some(b.height()) } else { None },
                    install.required_depth_mm,
                ),
                (
                    "height",
                    install.available_height_mm,
                    install.required_height_mm,
                ),
            ] {
                if let Some(v) = required {
                    positive(v)?;
                }
                if let Some(v) = measured {
                    positive(v)?;
                }
                add(
                    &install.key,
                    &format!("installation.{axis}"),
                    if install.source.trim().is_empty() {
                        Status::Unknown
                    } else {
                        metric(measured, required, tol)
                    },
                    format!(
                        "{}; absent installation dimensions are unknown; room CH is not used as installation height.",
                        install.source
                    ),
                    center(room),
                    measured,
                    required,
                );
            }
        }
        coverage.push(Coverage {
            drawing: d.drawing.clone(),
            rooms: d.rooms.len(),
            fixtures: d.fixtures.len(),
            doors: d.doors.len(),
            openings: d.openings.len(),
            wall_regions: d.walls.fill_entity_ids.len(),
        });
    }
    let status = if checks.iter().any(|c| c.status == Status::Fail) {
        Status::Fail
    } else if checks.is_empty() || checks.iter().any(|c| c.status == Status::Unknown) {
        Status::Unknown
    } else {
        Status::Pass
    };
    Ok(Analysis{report:Report{schema_version:SCHEMA.into(),status,checks,coverage,scope:"Only explicitly mapped geometry and rules. Unmapped objects, structure, fire safety, services and site conditions are not assessed. Fixture bounds are conservative rectangles; door swings are sampled.".into()},repairs})
}

pub struct Candidate {
    pub request: cad_edit::DrawingEditRequest,
    pub ledger: Ledger,
    pub project: ProjectSource,
}
fn stable_id(seed: &str) -> String {
    let digest = blake3::hash(seed.as_bytes());
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest.as_bytes()[..16]);
    let number = u128::from_be_bytes(bytes);
    let alphabet = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let text = (0..26)
        .rev()
        .map(|i| alphabet[((number >> (i * 5)) & 31) as usize] as char)
        .collect::<String>();
    format!("ent_{text}")
}
/// Generate a checked, revision-pinned candidate only. Never publish it here.
pub fn repair_candidate(
    project: &ProjectSource,
    ledger: &Ledger,
    repair: &Repair,
    revision: &str,
    files: Vec<cad_model::SourceFileRevision>,
) -> Result<Candidate> {
    let before = analyze(project, ledger)?;
    if !repair.available || !before.repairs.iter().any(|current| current == repair) {
        return Err(invalid(
            "repair is not eligible for the current project and ledger",
        ));
    }
    let binding = ledger
        .drawings
        .iter()
        .find(|d| d.drawing == repair.drawing)
        .ok_or_else(|| invalid("repair drawing missing"))?;
    let source = project
        .drawings
        .iter()
        .find(|d| d.name == repair.drawing)
        .ok_or_else(|| invalid("repair drawing missing"))?;
    let map = source
        .entities
        .iter()
        .map(|r| (r.entity.id().as_str().to_owned(), &r.entity))
        .collect::<BTreeMap<_, _>>();
    let walls = wall_geometry(&map, &binding.walls)?;
    // Polygon union can merge regions; mixed styles must never be silently replaced.
    for (ids, geometry_field) in [
        (&binding.walls.fill_entity_ids, "loops"),
        (&binding.walls.outline_entity_ids, "points"),
    ] {
        let mut style = None;
        for id in ids {
            let mut attributes =
                serde_json::to_value(entity(&map, id)?).map_err(|e| invalid(e.to_string()))?;
            let fields = attributes.as_object_mut().unwrap();
            fields.remove("id");
            fields.remove(geometry_field);
            if style.as_ref().is_some_and(|first| first != &attributes) {
                return Err(invalid(
                    "repair requires uniformly styled wall fills and outlines",
                ));
            }
            style = Some(attributes);
        }
    }

    let mut boundaries = binding
        .walls
        .outline_entity_ids
        .iter()
        .map(|id| {
            if let Entity::Polyline { points, .. } = entity(&map, id)? {
                polygon(points)
            } else {
                Err(invalid("wall outline must be a polyline"))
            }
        })
        .collect::<Result<Vec<_>>>()?;
    for id in &binding.walls.fill_entity_ids {
        let Entity::Hatch { loops, .. } = entity(&map, id)? else {
            unreachable!()
        };
        // Match the source loops, which can include disjoint exteriors and nested islands.
        for points in loops {
            let shape = polygon(points)?;
            let matching = boundaries
                .iter()
                .position(|b| shape.xor(b).unsigned_area() <= ledger.tolerance_mm.powi(2))
                .ok_or_else(|| {
                    invalid("repair requires complete matching wall fill and outline bindings")
                })?;
            boundaries.remove(matching);
        }
    }
    if !boundaries.is_empty() {
        return Err(invalid("repair has extra wall outline bindings"));
    }
    let old = opening_polygon(&Opening {
        key: repair.opening.clone(),
        axis: repair.old_axis,
        depth_mm: repair.depth_mm,
    })?;
    let new = opening_polygon(&Opening {
        key: repair.opening.clone(),
        axis: repair.new_axis,
        depth_mm: repair.depth_mm,
    })?;
    let fixed = walls
        .union(&MultiPolygon(vec![old]))
        .difference(&MultiPolygon(vec![new]));
    if !fixed.is_valid() {
        return Err(invalid("repair resulted in invalid wall polygons"));
    }
    let mut polygons = fixed.0;
    polygons.sort_by(|a, b| b.unsigned_area().total_cmp(&a.unsigned_area()));
    let mut unused_fills = binding.walls.fill_entity_ids.clone();
    let mut unused_outlines = binding.walls.outline_entity_ids.clone();
    let mut replacements = vec![];
    let mut fill_ids = vec![];
    let mut outline_ids = vec![];
    let all_ids = project
        .drawings
        .iter()
        .flat_map(|d| d.entities.iter().map(|r| r.entity.id().as_str().to_owned()))
        .chain(
            project
                .blocks
                .values()
                .flat_map(|b| b.entities.iter().map(|r| r.entity.id().as_str().to_owned())),
        )
        .collect::<BTreeSet<_>>();
    let mut created = BTreeSet::new();
    let mut counter = 0;
    let mut fresh = || -> Result<String> {
        counter += 1;
        let id = stable_id(&format!(
            "{revision}:{}:{}:{counter}",
            repair.drawing, repair.door
        ));
        if all_ids.contains(&id) || !created.insert(id.clone()) {
            return Err(invalid("deterministic repair ID collision"));
        }
        Ok(id)
    };
    for p in polygons {
        let fill_id = unused_fills
            .iter()
            .max_by(|a, b| {
                hatch(map[*a])
                    .unwrap()
                    .intersection(&MultiPolygon(vec![p.clone()]))
                    .unsigned_area()
                    .total_cmp(
                        &hatch(map[*b])
                            .unwrap()
                            .intersection(&MultiPolygon(vec![p.clone()]))
                            .unsigned_area(),
                    )
            })
            .cloned();
        let template = entity(
            &map,
            fill_id
                .as_ref()
                .unwrap_or(&binding.walls.fill_entity_ids[0]),
        )?;
        let mut value = serde_json::to_value(template).map_err(|e| invalid(e.to_string()))?;
        let id = if let Some(id) = fill_id {
            unused_fills.retain(|x| x != &id);
            id
        } else {
            fresh()?
        };
        value["id"] = serde_json::json!(id);
        let rings = std::iter::once(p.exterior())
            .chain(p.interiors().iter())
            .collect::<Vec<_>>();
        value["loops"] = serde_json::json!(
            rings
                .iter()
                .map(|r| r
                    .0
                    .iter()
                    .take(r.0.len() - 1)
                    .map(|c| [c.x, c.y])
                    .collect::<Vec<_>>())
                .collect::<Vec<_>>()
        );
        fill_ids.push(id);
        replacements.push(value);
        for ring in rings {
            let old_id = unused_outlines
                .iter()
                .max_by(|a, b| {
                    let get = |id: &str| -> f64 {
                        if let Entity::Polyline { points, .. } = map[id] {
                            polygon(points)
                                .map(|poly| {
                                    poly.intersection(&Polygon::new(ring.clone(), vec![]))
                                        .unsigned_area()
                                })
                                .unwrap_or(0.0)
                        } else {
                            0.0
                        }
                    };
                    get(a).total_cmp(&get(b))
                })
                .cloned();
            let mut value = serde_json::to_value(entity(
                &map,
                old_id
                    .as_ref()
                    .unwrap_or(&binding.walls.outline_entity_ids[0]),
            )?)
            .map_err(|e| invalid(e.to_string()))?;
            let id = if let Some(id) = old_id {
                unused_outlines.retain(|x| x != &id);
                id
            } else {
                fresh()?
            };
            value["id"] = serde_json::json!(id);
            value["points"] =
                serde_json::json!(ring.0.iter().map(|c| [c.x, c.y]).collect::<Vec<_>>());
            outline_ids.push(id);
            replacements.push(value);
        }
    }
    let bound = binding
        .walls
        .fill_entity_ids
        .iter()
        .chain(&binding.walls.outline_entity_ids)
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut candidate = project.clone();
    let drawing = candidate
        .drawings
        .iter_mut()
        .find(|d| d.name == repair.drawing)
        .unwrap();
    drawing
        .entities
        .retain(|r| !bound.contains(r.entity.id().as_str()));
    let mut operations = vec![];
    for value in replacements {
        let id = value["id"].as_str().unwrap().to_owned();
        let e: Entity =
            serde_json::from_value(value.clone()).map_err(|e| invalid(e.to_string()))?;
        drawing.entities.push(cad_model::EntityRecord {
            line: drawing.entities.len() + 1,
            entity: e,
        });
        operations.push(if bound.contains(&id) {
            cad_edit::EditOperation::Replace {
                entity_id: id,
                entity: value,
            }
        } else {
            cad_edit::EditOperation::Create { entity: value }
        });
    }
    for id in unused_fills.into_iter().chain(unused_outlines) {
        operations.push(cad_edit::EditOperation::Delete { entity_id: id });
    }
    let check = cad_check::check_loaded_project(&candidate);
    if check.status == cad_check::CheckStatus::Error {
        return Err(invalid(format!(
            "candidate CAD check failed: {:?}",
            check.diagnostics
        )));
    }
    let mut new_ledger = ledger.clone();
    let d = new_ledger
        .drawings
        .iter_mut()
        .find(|d| d.drawing == repair.drawing)
        .unwrap();
    d.walls.fill_entity_ids = fill_ids;
    d.walls.outline_entity_ids = outline_ids;
    d.openings
        .iter_mut()
        .find(|o| o.key == repair.opening)
        .unwrap()
        .axis = repair.new_axis;
    let after = analyze(&candidate, &new_ledger)?;
    // The targeted alignment and swing/wall check must pass, and no new failed check may appear.
    for check in &after.report.checks {
        if check.status == Status::Fail {
            let existed = before.report.checks.iter().any(|old| {
                old.code == check.code
                    && old.object == check.object
                    && old.drawing == check.drawing
                    && old.status == Status::Fail
            });
            if !existed
                || (check.object == repair.door
                    && (check.code == "door.opening_alignment"
                        || check.code == "door.swing_wall_overlap"))
            {
                return Err(invalid(
                    "repair leaves the target invalid or introduces a new failed check",
                ));
            }
        }
    }
    Ok(Candidate {
        request: cad_edit::DrawingEditRequest {
            drawing: repair.drawing.clone(),
            expected_revision: revision.into(),
            operation: cad_edit::EditOperation::SourceChecked {
                expected_files: files,
                operation: Box::new(cad_edit::EditOperation::Batch { operations }),
            },
        },
        ledger: new_ledger,
        project: candidate,
    })
}
