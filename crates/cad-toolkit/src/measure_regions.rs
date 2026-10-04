use super::{AreaMode, MeasurementOptions, geometry, loop_area};
use crate::{Result, ToolkitError};
use cad_model::{Entity, Point, ProjectSource};
use geo::{Area, BooleanOps, LineString, MultiPolygon, Polygon, Validation};
use std::f64::consts::PI;

#[derive(Default)]
pub(super) struct Budget {
    leaves: usize,
    vertices: usize,
}
#[derive(Default)]
pub(super) struct Expanded {
    pub length: Option<f64>,
    pub area: Option<f64>,
    pub length_error: Option<f64>,
    pub supported: usize,
    pub unsupported: usize,
    pub regions: Vec<MultiPolygon<f64>>,
    pub area_error: f64,
}
#[derive(Clone, Copy)]
struct Transform {
    matrix: [f64; 4],
    offset: Point,
    scale: f64,
}
impl Transform {
    const IDENTITY: Self = Self {
        matrix: [1.0, 0.0, 0.0, 1.0],
        offset: [0.0, 0.0],
        scale: 1.0,
    };
    fn point(self, p: Point) -> Point {
        [
            self.matrix[0] * p[0] + self.matrix[1] * p[1] + self.offset[0],
            self.matrix[2] * p[0] + self.matrix[3] * p[1] + self.offset[1],
        ]
    }
    fn compose(self, local: Self) -> Result<Self> {
        let a = self.matrix;
        let b = local.matrix;
        let result = Self {
            matrix: [
                a[0] * b[0] + a[1] * b[2],
                a[0] * b[1] + a[1] * b[3],
                a[2] * b[0] + a[3] * b[2],
                a[2] * b[1] + a[3] * b[3],
            ],
            offset: self.point(local.offset),
            scale: self.scale * local.scale,
        };
        if result
            .matrix
            .iter()
            .chain(result.offset.iter())
            .chain([result.scale].iter())
            .any(|n| !n.is_finite())
            || result.scale <= 0.0
        {
            return Err(ToolkitError::Invalid(
                "block transform overflowed or has invalid scale".into(),
            ));
        }
        Ok(result)
    }
}
pub(super) fn expand(
    project: &ProjectSource,
    entity: &Entity,
    options: MeasurementOptions,
    budget: &mut Budget,
) -> Result<Expanded> {
    let mut measured = Expanded::default();
    visit(
        project,
        entity,
        Transform::IDENTITY,
        options,
        budget,
        &mut Vec::new(),
        &mut measured,
    )?;
    Ok(measured)
}
fn add(target: &mut Option<f64>, value: Option<f64>, scale: f64) {
    if let Some(value) = value {
        *target = Some(target.unwrap_or(0.0) + value * scale);
    }
}
fn visit(
    project: &ProjectSource,
    entity: &Entity,
    transform: Transform,
    options: MeasurementOptions,
    budget: &mut Budget,
    stack: &mut Vec<String>,
    measured: &mut Expanded,
) -> Result<()> {
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
        if stack.len() >= 32 || stack.contains(block) {
            return Err(ToolkitError::Invalid(format!(
                "recursive or excessively nested measured block: {block}"
            )));
        }
        let definition = project
            .blocks
            .get(block)
            .ok_or_else(|| ToolkitError::Invalid(format!("missing measured block: {block}")))?;
        let (sin, cos) = rotation_deg.to_radians().sin_cos();
        let sx = scale * if *mirror_x { -1.0 } else { 1.0 };
        let sy = scale * if *mirror_y { -1.0 } else { 1.0 };
        let matrix = [cos * sx, -sin * sy, sin * sx, cos * sy];
        let base = definition.config.base_point;
        let local = Transform {
            matrix,
            offset: [
                at[0] - matrix[0] * base[0] - matrix[1] * base[1],
                at[1] - matrix[2] * base[0] - matrix[3] * base[1],
            ],
            scale: scale.abs(),
        };
        let child_transform = transform.compose(local)?;
        stack.push(block.clone());
        for child in &definition.entities {
            visit(
                project,
                &child.entity,
                child_transform,
                options,
                budget,
                stack,
                measured,
            )?;
        }
        stack.pop();
        return Ok(());
    }
    budget.leaves += 1;
    if budget.leaves > 100_000 {
        return Err(ToolkitError::Invalid(
            "measurement expansion exceeds 100000 leaf entities".into(),
        ));
    }
    let (length, area, error) = geometry(entity)?;
    if length.is_none() && area.is_none() {
        measured.unsupported += 1;
        return Ok(());
    }
    measured.supported += 1;
    add(&mut measured.length, length, transform.scale);
    add(&mut measured.area, area, transform.scale.powi(2));
    add(&mut measured.length_error, error, transform.scale);
    if options.area_mode != AreaMode::Union || area.is_none() {
        return Ok(());
    }
    let loops = match entity {
        Entity::Polyline {
            points,
            closed: true,
            ..
        }
        | Entity::Solid { points, .. } => vec![points.clone()],
        Entity::Hatch { loops, .. } => loops.clone(),
        Entity::Circle { center, radius, .. } => vec![ellipse(
            *center,
            *radius,
            *radius,
            0.0,
            transform.scale,
            options.curve_tolerance_mm,
        )?],
        Entity::Ellipse {
            center,
            radius_x,
            radius_y,
            rotation_deg,
            ..
        } => vec![ellipse(
            *center,
            *radius_x,
            *radius_y,
            *rotation_deg,
            transform.scale,
            options.curve_tolerance_mm,
        )?],
        _ => {
            return Err(ToolkitError::Invalid(
                "closed measured geometry has no region representation".into(),
            ));
        }
    };
    let region = region(&loops, transform, budget)?;
    if matches!(entity, Entity::Circle { .. } | Entity::Ellipse { .. }) {
        measured.area_error +=
            (area.unwrap() * transform.scale.powi(2) - region.unsigned_area()).max(0.0);
    }
    measured.regions.push(region);
    Ok(())
}
fn ellipse(
    center: Point,
    rx: f64,
    ry: f64,
    rotation: f64,
    scale: f64,
    tolerance: f64,
) -> Result<Vec<Point>> {
    let radius = rx.max(ry) * scale;
    let angle = (1.0 - (tolerance / radius).min(1.0)).acos();
    let steps = (PI / angle).ceil().max(16.0);
    if !steps.is_finite() || steps > 65536.0 {
        return Err(ToolkitError::Invalid(
            "curve union tolerance requires more than 65536 segments".into(),
        ));
    }
    let (sin, cos) = rotation.to_radians().sin_cos();
    Ok((0..steps as usize)
        .map(|index| {
            let (s, c) = (2.0 * PI * index as f64 / steps).sin_cos();
            [
                center[0] + cos * rx * c - sin * ry * s,
                center[1] + sin * rx * c + cos * ry * s,
            ]
        })
        .collect())
}
fn region(
    loops: &[Vec<Point>],
    transform: Transform,
    budget: &mut Budget,
) -> Result<MultiPolygon<f64>> {
    loop_area(loops)?; // Validate even-odd nesting before performing boolean operations.
    let mut result = MultiPolygon(Vec::new());
    for points in loops {
        budget.vertices += points.len();
        if budget.vertices > 1_000_000 {
            return Err(ToolkitError::Invalid(
                "measurement regions exceed 1000000 vertices".into(),
            ));
        }
        let transformed = points
            .iter()
            .map(|p| transform.point(*p))
            .collect::<Vec<_>>();
        if transformed.iter().flatten().any(|n| !n.is_finite()) {
            return Err(ToolkitError::Invalid(
                "measured region coordinates overflowed".into(),
            ));
        }
        let polygon = Polygon::new(
            LineString::from(transformed.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>()),
            Vec::new(),
        );
        polygon.check_validation().map_err(|error| {
            ToolkitError::Invalid(format!("transformed measured boundary is invalid: {error}"))
        })?;
        result = result.xor(&MultiPolygon(vec![polygon]));
    }
    Ok(result)
}
