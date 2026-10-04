//! DXF exchange adapter. Unsupported geometry is reported, never silently lost.
mod import;
use cad_model::{Entity, Point, ProjectSource};
use dxf::{
    Drawing, Point as DxfPoint, Vector, entities as dx,
    enums::{AcadVersion, Units},
    tables,
};
use geo::{BooleanOps, LineString, MultiPolygon, Polygon, TriangulateEarcut, Validation};
pub use import::{DxfImport, import};
use serde::Serialize;
use std::collections::BTreeMap;
use std::f64::consts::TAU;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ExchangeError {
    #[error("{0}")]
    Invalid(String),
}
pub type Result<T> = std::result::Result<T, ExchangeError>;
fn invalid(message: impl ToString) -> ExchangeError {
    ExchangeError::Invalid(message.to_string())
}
#[derive(Debug, Serialize)]
pub struct Issue {
    pub code: String,
    pub entity_id: Option<String>,
    pub message: String,
}
#[derive(Debug, Serialize)]
pub struct ExchangeReport {
    pub schema_version: String,
    pub format: String,
    pub status: String,
    pub source_blake3: Option<String>,
    pub units: String,
    pub record_counts: BTreeMap<String, usize>,
    pub warnings: Vec<Issue>,
    pub blockers: Vec<Issue>,
}
impl ExchangeReport {
    fn new() -> Self {
        Self {
            schema_version: "cad-exchange/1".into(),
            format: "dxf-r2013".into(),
            status: "ready".into(),
            source_blake3: None,
            units: "mm".into(),
            record_counts: BTreeMap::new(),
            warnings: Vec::new(),
            blockers: Vec::new(),
        }
    }
    fn warning(&mut self, entity: Option<&Entity>, code: &str, message: impl Into<String>) {
        self.warnings.push(Issue {
            code: code.into(),
            entity_id: entity.map(|e| e.id().as_str().into()),
            message: message.into(),
        });
    }
    fn block(&mut self, entity: Option<&Entity>, code: &str, message: impl Into<String>) {
        self.blockers.push(Issue {
            code: code.into(),
            entity_id: entity.map(|e| e.id().as_str().into()),
            message: message.into(),
        });
    }
}
pub struct DxfExport {
    pub bytes: Option<Vec<u8>>,
    pub report: ExchangeReport,
}
fn point(p: Point) -> DxfPoint {
    DxfPoint::new(p[0], p[1], 0.0)
}

pub fn export(project: &ProjectSource, drawing_name: &str, strict: bool) -> Result<DxfExport> {
    match export_impl(project, drawing_name, strict) {
        Ok(result) => Ok(result),
        Err(error) => {
            let mut report = ExchangeReport::new();
            report.status = "blocked".into();
            report.block(None, "conversion_failed", error.to_string());
            Ok(DxfExport {
                bytes: None,
                report,
            })
        }
    }
}

fn export_impl(project: &ProjectSource, drawing_name: &str, strict: bool) -> Result<DxfExport> {
    let source = project
        .drawings
        .iter()
        .find(|d| d.name == drawing_name)
        .ok_or_else(|| invalid("drawing not found"))?;
    let scale = source
        .layouts
        .active()
        .and_then(|l| cad_model::parse_layout_scale(&l.scale))
        .ok_or_else(|| invalid("active layout scale is invalid"))?;
    let check = cad_check::check_loaded_project(project);
    if !check.is_ok() {
        return Err(invalid(format!(
            "DXF source failed CAD validation: {check:?}"
        )));
    }
    let mut report = ExchangeReport::new();
    report.warning(None,"layout_modelspace","Model-space DXF does not preserve CAD page layouts, source IDs, reference-dimension relationships, or separate screen/print colors.");
    let mut drawing = Drawing::new();
    drawing.header.version = AcadVersion::R2013;
    drawing.header.default_drawing_units = Units::Millimeters;
    for (name, style) in &project.styles.line_types {
        let dash: Vec<_> = style
            .dash
            .iter()
            .enumerate()
            .map(|(i, v)| v * scale * if i % 2 == 0 { 1.0 } else { -1.0 })
            .collect();
        drawing.add_line_type(tables::LineType {
            name: name.clone(),
            description: name.clone(),
            total_pattern_length: dash.iter().map(|v| v.abs()).sum(),
            element_count: dash.len() as i32,
            dash_dot_space_lengths: dash,
            ..Default::default()
        });
    }
    for (id, layer) in &project.layers.layers {
        let group = layer
            .group
            .as_ref()
            .and_then(|id| project.layers.groups.get(id));
        drawing.add_layer(tables::Layer {
            name: id.clone(),
            line_type_name: layer.line_type.clone(),
            is_layer_on: layer.visible && group.is_none_or(|g| g.visible),
            is_layer_plotted: layer.printable,
            ..Default::default()
        });
        if layer.locked || group.is_some_and(|g| g.locked) {
            report.warning(
                None,
                "layer_lock",
                "DXF exchange does not preserve the source editing lock state.",
            );
        }
    }
    for (name, style) in &project.styles.text_styles {
        drawing.add_style(tables::Style {
            name: name.clone(),
            primary_font_file_name: style.font_family.clone(),
            ..Default::default()
        });
    }
    for (name, definition) in &project.blocks {
        let mut block = dxf::Block {
            name: name.clone(),
            description: definition.config.name.clone(),
            base_point: point(definition.config.base_point),
            ..Default::default()
        };
        for record in &definition.entities {
            block
                .entities
                .extend(convert_entity(project, &record.entity, &mut report)?);
        }
        drawing.add_block(block);
    }
    for record in &source.entities {
        for entity in convert_entity(project, &record.entity, &mut report)? {
            drawing.add_entity(entity);
        }
    }
    if strict && !report.warnings.is_empty() {
        report.block(
            None,
            "strict_approximation",
            "Strict export blocks all reported substitutions or semantic reductions.",
        );
    }
    let bytes = if report.blockers.is_empty() {
        let mut bytes = Vec::new();
        drawing.save(&mut bytes).map_err(invalid)?;
        Some(bytes)
    } else {
        report.status = "blocked".into();
        None
    };
    Ok(DxfExport { bytes, report })
}

fn convert_entity(
    project: &ProjectSource,
    entity: &Entity,
    report: &mut ExchangeReport,
) -> Result<Vec<dx::Entity>> {
    let mut entities = Vec::new();
    match entity {
        Entity::Line { p1, p2, .. } => entities.push(dx::Entity::new(dx::EntityType::Line(
            dx::Line::new(point(*p1), point(*p2)),
        ))),
        Entity::Polyline { points, closed, .. } => {
            let mut poly = dx::LwPolyline::default();
            poly.set_is_closed(*closed);
            let end = if *closed && points.first() == points.last() {
                points.len().saturating_sub(1)
            } else {
                points.len()
            };
            poly.vertices = points[..end]
                .iter()
                .map(|p| dxf::LwPolylineVertex {
                    x: p[0],
                    y: p[1],
                    ..Default::default()
                })
                .collect();
            entities.push(dx::Entity::new(dx::EntityType::LwPolyline(poly)));
        }
        Entity::Circle { center, radius, .. } => entities.push(dx::Entity::new(
            dx::EntityType::Circle(dx::Circle::new(point(*center), *radius)),
        )),
        Entity::Arc {
            center,
            radius,
            start_deg,
            end_deg,
            ..
        } => {
            let (start, end) = if end_deg < start_deg {
                report.warning(Some(entity),"arc_direction","Clockwise arc orientation is reversed to DXF counterclockwise representation; the locus is preserved.");
                (*end_deg, *start_deg)
            } else {
                (*start_deg, *end_deg)
            };
            if (end - start).abs() >= 360.0 - 1e-9 {
                entities.push(dx::Entity::new(dx::EntityType::Circle(dx::Circle::new(
                    point(*center),
                    *radius,
                ))));
            } else {
                entities.push(dx::Entity::new(dx::EntityType::Arc(dx::Arc::new(
                    point(*center),
                    *radius,
                    start.rem_euclid(360.0),
                    end.rem_euclid(360.0),
                ))));
            }
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
            let (major, minor, rotation, shift) = if radius_x >= radius_y {
                (*radius_x, *radius_y, *rotation_deg, 0.0)
            } else {
                (*radius_y, *radius_x, rotation_deg + 90.0, 90.0)
            };
            let (start, end) = if end_deg < start_deg {
                report.warning(Some(entity),"ellipse_direction","Clockwise ellipse orientation is reversed to DXF representation; the locus is preserved.");
                (*end_deg, *start_deg)
            } else {
                (*start_deg, *end_deg)
            };
            let (start, end) = if (end - start).abs() >= 360.0 - 1e-9 {
                (0.0, TAU)
            } else {
                (
                    (start - shift).to_radians().rem_euclid(TAU),
                    (end - shift).to_radians().rem_euclid(TAU),
                )
            };
            entities.push(dx::Entity::new(dx::EntityType::Ellipse(dx::Ellipse {
                center: point(*center),
                major_axis: Vector::new(
                    major * rotation.to_radians().cos(),
                    major * rotation.to_radians().sin(),
                    0.0,
                ),
                minor_axis_ratio: minor / major,
                start_parameter: start,
                end_parameter: end,
                ..Default::default()
            })));
        }
        Entity::Point {
            at,
            temporary,
            marker_code,
            ..
        } => {
            if *temporary || marker_code.is_some() {
                report.warning(
                    Some(entity),
                    "point_marker",
                    "Point marker and temporary editing metadata are reduced to a DXF point.",
                );
            }
            entities.push(dx::Entity::new(dx::EntityType::ModelPoint(
                dx::ModelPoint::new(point(*at)),
            )));
        }
        Entity::Text {
            style,
            at,
            rotation_deg,
            mirror_y,
            writing_mode,
            value,
            ..
        } => {
            let style_def = project
                .styles
                .text_styles
                .get(style)
                .ok_or_else(|| invalid("text style is missing"))?;
            let justify = match style_def.align {
                cad_model::TextAlign::Left => dxf::enums::HorizontalTextJustification::Left,
                cad_model::TextAlign::Center => dxf::enums::HorizontalTextJustification::Center,
                cad_model::TextAlign::Right => dxf::enums::HorizontalTextJustification::Right,
            };
            let mut text = dx::Text {
                location: point(*at),
                second_alignment_point: point(*at),
                text_height: style_def.height,
                relative_x_scale_factor: style_def.width / style_def.height,
                rotation: *rotation_deg,
                value: value.clone(),
                text_style_name: style.clone(),
                horizontal_text_justification: justify,
                ..Default::default()
            };
            text.set_is_text_upside_down(*mirror_y);
            if *writing_mode == cad_model::TextWritingMode::VerticalUpright {
                report.warning(Some(entity), "upright_text_expanded", "Upright annotation columns expand to positioned single-character TEXT entities; native vertical font substitutions and text editing semantics are not preserved.");
                text.horizontal_text_justification = dxf::enums::HorizontalTextJustification::Left;
                for (position, character) in
                    cad_model::upright_text_glyphs(*at, *rotation_deg, *mirror_y, value, style_def)
                {
                    let mut glyph = text.clone();
                    glyph.location = point(position);
                    glyph.second_alignment_point = point(position);
                    glyph.value = character.to_string();
                    entities.push(dx::Entity::new(dx::EntityType::Text(glyph)));
                }
            } else {
                entities.push(dx::Entity::new(dx::EntityType::Text(text)));
            }
            report.warning(Some(entity),"text_metrics","Font selection, fixed character width and character spacing depend on the receiving CAD; compare the text after exchange.");
        }
        Entity::BlockRef {
            block,
            at,
            rotation_deg,
            scale,
            mirror_x,
            mirror_y,
            ..
        } => {
            entities.push(dx::Entity::new(dx::EntityType::Insert(dx::Insert {
                name: block.clone(),
                location: point(*at),
                rotation: *rotation_deg,
                x_scale_factor: scale * if *mirror_x { -1.0 } else { 1.0 },
                y_scale_factor: scale * if *mirror_y { -1.0 } else { 1.0 },
                z_scale_factor: *scale,
                ..Default::default()
            })));
        }
        Entity::Dimension { .. } => {
            report.warning(Some(entity),"dimension_expansion","Evaluated dimensions expand to geometry and text; editable dimension associations are not preserved.");
            for primitive in cad_model::dimension_primitives(project, entity).map_err(invalid)? {
                entities.extend(convert_entity(project, &primitive, report)?);
            }
            return Ok(entities);
        }
        Entity::Solid { points, fill, .. } => {
            entities.extend(triangulated_fill(std::slice::from_ref(points))?);
            let color = cad_model::resolve_fill_color(project, entity, fill).map_err(invalid)?;
            style_entities(project, entity, &mut entities, Some(&color), report)?;
            report.warning(
                Some(entity),
                "solid_triangles",
                "Filled polygon is represented by DXF SOLID triangles.",
            );
            count_records(&entities, report);
            return Ok(entities);
        }
        Entity::Hatch {
            loops,
            pattern,
            angle_deg,
            scale,
            fill,
            ..
        } => {
            if pattern == "solid" {
                entities.extend(triangulated_fill(loops)?);
            } else {
                let mut angles = vec![*angle_deg];
                if pattern == "cross" {
                    angles.push(angle_deg + 90.0);
                }
                for angle in angles {
                    for (p1, p2) in
                        cad_model::hatch_line_segments(loops, angle, *scale).map_err(invalid)?
                    {
                        entities.push(dx::Entity::new(dx::EntityType::Line(dx::Line::new(
                            point(p1),
                            point(p2),
                        ))));
                    }
                }
            }
            let color = fill
                .as_ref()
                .map(|fill| cad_model::resolve_fill_color(project, entity, fill))
                .transpose()
                .map_err(invalid)?;
            style_entities(project, entity, &mut entities, color.as_deref(), report)?;
            for points in loops {
                for pair in points
                    .iter()
                    .zip(points.iter().cycle().skip(1))
                    .take(points.len())
                {
                    if pair.0 == pair.1 {
                        continue;
                    }
                    let mut boundary = vec![dx::Entity::new(dx::EntityType::Line(dx::Line::new(
                        point(*pair.0),
                        point(*pair.1),
                    )))];
                    style_entities(project, entity, &mut boundary, None, report)?;
                    entities.extend(boundary);
                }
            }
            report.warning(Some(entity),"hatch_expansion","Hatch fill or clipped lines and boundary edges expand to DXF primitives; hatch associativity is not preserved.");
            count_records(&entities, report);
            return Ok(entities);
        }
        Entity::CurveSolid {
            center,
            radius,
            flatness,
            rotation_deg,
            start_deg,
            end_deg,
            solid_param,
            fill,
            ..
        } => {
            let tolerance = 0.1_f64;
            let max_radius = radius * flatness.abs().max(1.0);
            let max_step = (2.0 * (1.0 - (tolerance / max_radius).min(1.0)).acos())
                .min(std::f64::consts::PI / 2.0);
            let span = (end_deg - start_deg).clamp(-360.0, 360.0);
            let steps = (span.abs().to_radians() / max_step).ceil().max(4.0) as usize;
            if steps > 100000 {
                return Err(invalid("curved solid tessellation exceeds 100000 segments"));
            }
            let full = span.abs() >= 360.0 - 1e-9;
            let sample = |rx: f64, ry: f64| {
                (0..=steps)
                    .map(|i| {
                        cad_model::ellipse_point(
                            *center,
                            rx,
                            ry,
                            *rotation_deg,
                            start_deg + span * i as f64 / steps as f64,
                        )
                    })
                    .collect::<Vec<_>>()
            };
            let mut outer = sample(*radius, radius * flatness.abs());
            if full {
                outer.pop();
            }
            let loops = if *solid_param > 0.0 && *solid_param < *radius {
                let mut inner = sample(*solid_param, solid_param * flatness.abs());
                if full {
                    inner.pop();
                    vec![outer, inner]
                } else {
                    inner.reverse();
                    outer.extend(inner);
                    vec![outer]
                }
            } else {
                if !full {
                    outer.push(*center);
                }
                vec![outer]
            };
            entities.extend(triangulated_fill(&loops)?);
            let color = cad_model::resolve_fill_color(project, entity, fill).map_err(invalid)?;
            style_entities(project, entity, &mut entities, Some(&color), report)?;
            report.warning(Some(entity),"curve_solid_tessellation","Curved solid expands to SOLID triangles with 0.1 mm maximum affine circle chord deviation in model space.");
            count_records(&entities, report);
            return Ok(entities);
        }
    }
    style_entities(project, entity, &mut entities, None, report)?;
    count_records(&entities, report);
    Ok(entities)
}
fn triangulated_fill(loops: &[Vec<Point>]) -> Result<Vec<dx::Entity>> {
    let mut region = MultiPolygon::<f64>(Vec::new());
    for points in loops {
        let polygon = Polygon::new(
            LineString::from(points.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>()),
            Vec::new(),
        );
        polygon.check_validation().map_err(invalid)?;
        region = region.xor(&MultiPolygon(vec![polygon]));
    }
    let mut entities = Vec::new();
    for polygon in region.0 {
        for triangle in polygon.earcut_triangles() {
            let [a, b, c] = triangle.to_array();
            entities.push(dx::Entity::new(dx::EntityType::Solid(dx::Solid {
                first_corner: point([a.x, a.y]),
                second_corner: point([b.x, b.y]),
                third_corner: point([c.x, c.y]),
                fourth_corner: point([c.x, c.y]),
                ..Default::default()
            })));
            if entities.len() > 100000 {
                return Err(invalid("DXF triangle expansion exceeds 100000 primitives"));
            }
        }
    }
    Ok(entities)
}
fn style_entities(
    project: &ProjectSource,
    source: &Entity,
    entities: &mut [dx::Entity],
    fill: Option<&str>,
    report: &mut ExchangeReport,
) -> Result<()> {
    let stroke = cad_model::resolve_entity_stroke(project, source).map_err(invalid)?;
    let color = fill.unwrap_or(&stroke.print_color_rgb);
    let rgb = i32::from_str_radix(color.trim_start_matches('#'), 16).map_err(invalid)?;
    let widths = [
        0, 5, 9, 13, 15, 18, 20, 25, 30, 35, 40, 50, 53, 60, 70, 80, 90, 100, 106, 120, 140, 158,
        200, 211,
    ];
    let raw = stroke.line_width_mm * 100.0;
    let width = *widths
        .iter()
        .min_by(|a, b| {
            (**a as f64 - raw)
                .abs()
                .total_cmp(&(**b as f64 - raw).abs())
        })
        .unwrap();
    if (width as f64 - raw).abs() > 1e-6 {
        report.warning(
            Some(source),
            "lineweight_approximation",
            format!(
                "Line width {} mm is approximated as {} mm.",
                stroke.line_width_mm,
                width as f64 / 100.0
            ),
        );
    }
    let line_type = source
        .pen()
        .and_then(|id| project.styles.pens.get(id))
        .map(|p| p.line_type.as_str())
        .unwrap_or(&project.layers.layers[source.layer()].line_type);
    for entity in entities {
        entity.common.layer = source.layer().into();
        entity.common.color_24_bit = rgb;
        entity.common.color = dxf::Color::from_index(7);
        entity.common.lineweight_enum_value = width;
        entity.common.line_type_name = line_type.into();
    }
    Ok(())
}
fn count_records(entities: &[dx::Entity], report: &mut ExchangeReport) {
    for entity in entities {
        let name = match entity.specific {
            dx::EntityType::Line(_) => "LINE",
            dx::EntityType::LwPolyline(_) => "LWPOLYLINE",
            dx::EntityType::Circle(_) => "CIRCLE",
            dx::EntityType::Arc(_) => "ARC",
            dx::EntityType::Ellipse(_) => "ELLIPSE",
            dx::EntityType::Text(_) => "TEXT",
            dx::EntityType::Insert(_) => "INSERT",
            dx::EntityType::ModelPoint(_) => "POINT",
            dx::EntityType::Solid(_) => "SOLID",
            _ => "OTHER",
        };
        *report.record_counts.entry(name.into()).or_default() += 1;
    }
}

#[cfg(test)]
mod tests;
