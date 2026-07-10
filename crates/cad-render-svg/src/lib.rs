//! SVG renderer crate.
//!
//! This crate turns the typed CAD model into the canonical SVG output. SVG is
//! the only Phase 3 render target, so all view conversion happens here.

use cad_model::{
    BBox, DrawingSource, Entity, EntityRecord, LayerDef, Point, ProjectSource, TextAlign,
    TextStyleDef, entity_bbox,
};
use svg::Document;
use svg::node::element::path::Data;
use svg::node::element::{Circle, Ellipse as SvgEllipse, Group, Line, Path, Polyline, Text};
use thiserror::Error;

pub const CRATE_NAME: &str = "cad-render-svg";

#[must_use]
pub fn crate_name() -> &'static str {
    CRATE_NAME
}

#[derive(Debug, Error)]
pub enum RenderError {
    #[error("project has no drawings")]
    NoDrawings,
    #[error("drawing {name:?} was not found")]
    MissingDrawing { name: String },
    #[error("unsupported paper {paper:?}")]
    UnsupportedPaper { paper: String },
    #[error("unsupported scale {scale:?}")]
    UnsupportedScale { scale: String },
    #[error("entity {entity_id:?} references missing layer {layer:?}")]
    MissingLayer { entity_id: String, layer: String },
    #[error("entity {entity_id:?} references missing color {color:?}")]
    MissingColor { entity_id: String, color: String },
    #[error("text entity {entity_id:?} references missing style {style:?}")]
    MissingTextStyle { entity_id: String, style: String },
    #[error("dimension entity {entity_id:?} references missing style {style:?}")]
    MissingDimensionStyle { entity_id: String, style: String },
    #[error("entity {entity_id:?} does not have a valid bbox")]
    MissingBBox { entity_id: String },
}

pub type RenderResult<T> = Result<T, RenderError>;

pub fn render_project_svg(project: &ProjectSource) -> RenderResult<String> {
    let Some(drawing) = project.drawings.first() else {
        return Err(RenderError::NoDrawings);
    };
    render_drawing_svg(project, &drawing.name)
}

pub fn render_drawing_svg(project: &ProjectSource, drawing_name: &str) -> RenderResult<String> {
    let drawing = project
        .drawings
        .iter()
        .find(|candidate| candidate.name == drawing_name)
        .ok_or_else(|| RenderError::MissingDrawing {
            name: drawing_name.to_owned(),
        })?;
    let viewport = Viewport::from_drawing(project, drawing)?;

    let mut root = Group::new().set("id", drawing.name.clone());
    for record in &drawing.entities {
        root = root.add(render_entity(project, record)?);
    }

    let document = Document::new()
        .set("viewBox", viewport.view_box())
        .set("width", "100%")
        .set("height", "100%")
        .set("preserveAspectRatio", "xMinYMin meet")
        .set("data-paper-width-mm", viewport.paper_width_mm)
        .set("data-paper-height-mm", viewport.paper_height_mm)
        .set("data-drawing", drawing.name.clone())
        .add(root);

    Ok(document.to_string())
}

fn render_entity(project: &ProjectSource, record: &EntityRecord) -> RenderResult<Group> {
    let entity_id = record.entity.id().as_str().to_owned();
    let layer = resolve_layer(project, &record.entity)?;
    let stroke = resolve_stroke(project, &entity_id, layer)?;
    let bbox =
        render_entity_bbox(project, &record.entity).ok_or_else(|| RenderError::MissingBBox {
            entity_id: entity_id.clone(),
        })?;

    let mut group = Group::new()
        .set("data-entity-id", entity_id)
        .set("data-layer", record.entity.layer())
        .set(
            "data-bbox",
            format!(
                "{},{},{},{}",
                cad_model::format_decimal_mm(bbox.min[0]),
                cad_model::format_decimal_mm(bbox.min[1]),
                cad_model::format_decimal_mm(bbox.max[0]),
                cad_model::format_decimal_mm(bbox.max[1])
            ),
        );

    match &record.entity {
        Entity::Line { p1, p2, .. } => {
            group = group.add(
                Line::new()
                    .set("x1", p1[0])
                    .set("y1", svg_y(p1[1]))
                    .set("x2", p2[0])
                    .set("y2", svg_y(p2[1]))
                    .set("fill", "none")
                    .set("stroke", stroke.color.clone())
                    .set("stroke-width", stroke.width_attr()),
            );
        }
        Entity::Polyline { points, closed, .. } => {
            if *closed {
                group = group.add(path_from_points(points, true, &stroke));
            } else {
                group = group.add(
                    Polyline::new()
                        .set("points", points_attr(points))
                        .set("fill", "none")
                        .set("stroke", stroke.color.clone())
                        .set("stroke-width", stroke.width_attr()),
                );
            }
        }
        Entity::Arc {
            center,
            radius,
            start_deg,
            end_deg,
            ..
        } => {
            group = group.add(arc_path(*center, *radius, *start_deg, *end_deg, &stroke));
        }
        Entity::Circle { center, radius, .. } => {
            group = group.add(
                Circle::new()
                    .set("cx", center[0])
                    .set("cy", svg_y(center[1]))
                    .set("r", *radius)
                    .set("fill", "none")
                    .set("stroke", stroke.color.clone())
                    .set("stroke-width", stroke.width_attr()),
            );
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
            if (end_deg - start_deg).abs() >= 360.0 - 1e-9 {
                group = group.add(
                    SvgEllipse::new()
                        .set("cx", center[0])
                        .set("cy", svg_y(center[1]))
                        .set("rx", *radius_x)
                        .set("ry", *radius_y)
                        .set(
                            "transform",
                            format!(
                                "rotate({} {} {})",
                                normalize_zero(-rotation_deg),
                                center[0],
                                svg_y(center[1])
                            ),
                        )
                        .set("fill", "none")
                        .set("stroke", stroke.color.clone())
                        .set("stroke-width", stroke.width_attr()),
                );
            } else {
                group = group.add(ellipse_arc_path(
                    *center,
                    *radius_x,
                    *radius_y,
                    *rotation_deg,
                    *start_deg,
                    *end_deg,
                    &stroke,
                ));
            }
        }
        Entity::Text {
            style,
            at,
            rotation_deg,
            mirror_y,
            value,
            ..
        } => {
            let text_style = project.styles.text_styles.get(style).ok_or_else(|| {
                RenderError::MissingTextStyle {
                    entity_id: record.entity.id().as_str().to_owned(),
                    style: style.clone(),
                }
            })?;
            group = group.add(text_node(
                at,
                *rotation_deg,
                *mirror_y,
                value,
                text_style,
                &stroke.color,
            ));
        }
        Entity::Dimension {
            style,
            p1,
            p2,
            offset,
            text_rotation_deg,
            text_mirror_y,
            value,
            ..
        } => {
            let dimension_style = project.styles.dimension_styles.get(style).ok_or_else(|| {
                RenderError::MissingDimensionStyle {
                    entity_id: record.entity.id().as_str().to_owned(),
                    style: style.clone(),
                }
            })?;
            let text_style = project
                .styles
                .text_styles
                .get(&dimension_style.text_style)
                .ok_or_else(|| RenderError::MissingTextStyle {
                    entity_id: record.entity.id().as_str().to_owned(),
                    style: dimension_style.text_style.clone(),
                })?;
            let (d1, d2) =
                cad_model::dimension_offset_segment(*p1, *p2, *offset).ok_or_else(|| {
                    RenderError::MissingBBox {
                        entity_id: record.entity.id().as_str().to_owned(),
                    }
                })?;
            let label = value.clone().unwrap_or_else(|| {
                cad_model::format_decimal_mm(
                    ((p2[0] - p1[0]).powi(2) + (p2[1] - p1[1]).powi(2)).sqrt(),
                )
            });
            group = group
                .add(
                    Line::new()
                        .set("x1", d1[0])
                        .set("y1", svg_y(d1[1]))
                        .set("x2", d2[0])
                        .set("y2", svg_y(d2[1]))
                        .set("fill", "none")
                        .set("stroke", stroke.color.clone())
                        .set("stroke-width", stroke.width_attr()),
                )
                .add(text_node_with_anchor(
                    &[(d1[0] + d2[0]) / 2.0, (d1[1] + d2[1]) / 2.0],
                    *text_rotation_deg,
                    *text_mirror_y,
                    &label,
                    text_style,
                    "middle",
                    &stroke.color,
                ));
        }
        Entity::BlockRef {
            block,
            at,
            rotation_deg,
            scale,
            ..
        } => {
            let size = 100.0 * scale;
            group = group
                .add(
                    Line::new()
                        .set("x1", at[0] - size)
                        .set("y1", svg_y(at[1]))
                        .set("x2", at[0] + size)
                        .set("y2", svg_y(at[1]))
                        .set("stroke", stroke.color.clone())
                        .set("stroke-width", stroke.width_attr()),
                )
                .add(
                    Line::new()
                        .set("x1", at[0])
                        .set("y1", svg_y(at[1] - size))
                        .set("x2", at[0])
                        .set("y2", svg_y(at[1] + size))
                        .set("stroke", stroke.color.clone())
                        .set("stroke-width", stroke.width_attr()),
                )
                .add(
                    Text::new("")
                        .set("x", at[0])
                        .set("y", svg_y(at[1] + (size * 1.5)))
                        .set("font-size", 250.0 * scale)
                        .set("text-anchor", "middle")
                        .set("transform", rotate_attr(*rotation_deg, *at))
                        .set("fill", stroke.color)
                        .add(svg::node::Text::new(block.clone())),
                );
        }
    }

    Ok(group)
}

fn resolve_layer<'a>(project: &'a ProjectSource, entity: &Entity) -> RenderResult<&'a LayerDef> {
    project
        .layers
        .layers
        .get(entity.layer())
        .ok_or_else(|| RenderError::MissingLayer {
            entity_id: entity.id().as_str().to_owned(),
            layer: entity.layer().to_owned(),
        })
}

fn resolve_stroke(
    project: &ProjectSource,
    entity_id: &str,
    layer: &LayerDef,
) -> RenderResult<Stroke> {
    let color =
        project
            .styles
            .colors
            .get(&layer.color)
            .ok_or_else(|| RenderError::MissingColor {
                entity_id: entity_id.to_owned(),
                color: layer.color.clone(),
            })?;
    Ok(Stroke {
        color: color.rgb.clone(),
        width: layer.line_width,
    })
}

fn text_node(
    at: &Point,
    rotation_deg: f64,
    mirror_y: bool,
    value: &str,
    style: &TextStyleDef,
    color: &str,
) -> Text {
    text_node_with_anchor(
        at,
        rotation_deg,
        mirror_y,
        value,
        style,
        text_anchor(&style.align),
        color,
    )
}

fn text_node_with_anchor(
    at: &Point,
    rotation_deg: f64,
    mirror_y: bool,
    value: &str,
    style: &TextStyleDef,
    anchor: &str,
    color: &str,
) -> Text {
    Text::new("")
        .set("x", at[0])
        .set("y", svg_y(at[1]))
        .set("font-family", style.font_family.clone())
        .set("font-size", style.height)
        .set("textLength", text_length(value, style))
        .set("lengthAdjust", "spacingAndGlyphs")
        .set("text-anchor", anchor)
        .set(
            "transform",
            text_transform_attr(rotation_deg, mirror_y, *at),
        )
        .set("fill", color)
        .add(svg::node::Text::new(value.to_owned()))
}

fn text_length(value: &str, style: &TextStyleDef) -> f64 {
    ((value.chars().count() as f64) * style.width + style.spacing).max(0.0)
}

fn path_from_points(points: &[Point], close: bool, stroke: &Stroke) -> Path {
    let Some(first) = points.first() else {
        return Path::new();
    };
    let mut data = Data::new().move_to((first[0], svg_y(first[1])));
    for point in points.iter().skip(1) {
        data = data.line_to((point[0], svg_y(point[1])));
    }
    if close {
        data = data.close();
    }
    Path::new()
        .set("d", data)
        .set("fill", "none")
        .set("stroke", stroke.color.clone())
        .set("stroke-width", stroke.width_attr())
}

fn arc_path(center: Point, radius: f64, start_deg: f64, end_deg: f64, stroke: &Stroke) -> Path {
    let start = polar_point(center, radius, start_deg);
    let end = polar_point(center, radius, end_deg);
    let delta = (end_deg - start_deg).abs();
    let large_arc = if delta > 180.0 { 1 } else { 0 };
    let sweep = if end_deg >= start_deg { 0 } else { 1 };
    let data = Data::new()
        .move_to((start[0], svg_y(start[1])))
        .elliptical_arc_to((radius, radius, 0, large_arc, sweep, end[0], svg_y(end[1])));
    Path::new()
        .set("d", data)
        .set("fill", "none")
        .set("stroke", stroke.color.clone())
        .set("stroke-width", stroke.width_attr())
}

fn ellipse_arc_path(
    center: Point,
    radius_x: f64,
    radius_y: f64,
    rotation_deg: f64,
    start_deg: f64,
    end_deg: f64,
    stroke: &Stroke,
) -> Path {
    let start = cad_model::ellipse_point(center, radius_x, radius_y, rotation_deg, start_deg);
    let end = cad_model::ellipse_point(center, radius_x, radius_y, rotation_deg, end_deg);
    let delta = (end_deg - start_deg).abs();
    let large_arc = if delta > 180.0 { 1 } else { 0 };
    let sweep = if end_deg >= start_deg { 0 } else { 1 };
    let data = Data::new()
        .move_to((start[0], svg_y(start[1])))
        .elliptical_arc_to((
            radius_x,
            radius_y,
            normalize_zero(-rotation_deg),
            large_arc,
            sweep,
            end[0],
            svg_y(end[1]),
        ));
    Path::new()
        .set("d", data)
        .set("fill", "none")
        .set("stroke", stroke.color.clone())
        .set("stroke-width", stroke.width_attr())
}

fn points_attr(points: &[Point]) -> String {
    points
        .iter()
        .map(|point| format!("{},{}", point[0], svg_y(point[1])))
        .collect::<Vec<_>>()
        .join(" ")
}

fn polar_point(center: Point, radius: f64, deg: f64) -> Point {
    let rad = deg.to_radians();
    [
        normalize_zero(center[0] + (radius * rad.cos())),
        normalize_zero(center[1] + (radius * rad.sin())),
    ]
}

fn svg_y(y: f64) -> f64 {
    normalize_zero(-y)
}

fn normalize_zero(value: f64) -> f64 {
    if value.abs() < 0.000_000_001 {
        0.0
    } else {
        value
    }
}

fn rotate_attr(rotation_deg: f64, at: Point) -> String {
    format!(
        "rotate({} {} {})",
        normalize_zero(-rotation_deg),
        at[0],
        svg_y(at[1])
    )
}

fn text_transform_attr(rotation_deg: f64, mirror_y: bool, at: Point) -> String {
    if !mirror_y {
        return rotate_attr(rotation_deg, at);
    }
    let screen_rotation = (-rotation_deg).to_radians();
    let a = normalize_zero(screen_rotation.cos());
    let b = normalize_zero(screen_rotation.sin());
    let c = b;
    let d = normalize_zero(-screen_rotation.cos());
    let anchor = [at[0], svg_y(at[1])];
    let e = normalize_zero(anchor[0] - (a * anchor[0] + c * anchor[1]));
    let f = normalize_zero(anchor[1] - (b * anchor[0] + d * anchor[1]));
    format!("matrix({a} {b} {c} {d} {e} {f})")
}

fn text_anchor(align: &TextAlign) -> &'static str {
    match align {
        TextAlign::Left => "start",
        TextAlign::Center => "middle",
        TextAlign::Right => "end",
    }
}

#[derive(Debug, Clone)]
struct Stroke {
    color: String,
    width: f64,
}

impl Stroke {
    fn width_attr(&self) -> String {
        format!("{}mm", cad_model::format_decimal_mm(self.width))
    }
}

#[derive(Debug, Clone, Copy)]
struct Viewport {
    paper_width_mm: f64,
    paper_height_mm: f64,
    view_box: (f64, f64, f64, f64),
}

impl Viewport {
    fn from_drawing(project: &ProjectSource, drawing: &DrawingSource) -> RenderResult<Self> {
        let (paper_width_mm, paper_height_mm) = paper_size_mm(&drawing.sheet.paper)?;
        let (paper_width_mm, paper_height_mm) = match drawing.sheet.orientation {
            cad_model::SheetOrientation::Portrait => (paper_width_mm, paper_height_mm),
            cad_model::SheetOrientation::Landscape => (paper_height_mm, paper_width_mm),
        };
        let scale = parse_scale(&drawing.sheet.scale)?;
        let fallback = BBox {
            min: drawing.sheet.origin,
            max: [
                drawing.sheet.origin[0] + (paper_width_mm * scale),
                drawing.sheet.origin[1] + (paper_height_mm * scale),
            ],
        };
        let content_bbox = drawing_content_bbox(project, drawing)?;
        let content_bbox = content_bbox.map_or(fallback, |bbox| union_bbox(fallback, bbox));
        let view_box = bbox_view_box(content_bbox);
        Ok(Self {
            paper_width_mm,
            paper_height_mm,
            view_box,
        })
    }

    fn view_box(&self) -> (f64, f64, f64, f64) {
        self.view_box
    }
}

fn drawing_content_bbox(
    project: &ProjectSource,
    drawing: &DrawingSource,
) -> RenderResult<Option<BBox>> {
    drawing
        .entities
        .iter()
        .map(|record| {
            render_entity_bbox(project, &record.entity).ok_or_else(|| RenderError::MissingBBox {
                entity_id: record.entity.id().as_str().to_owned(),
            })
        })
        .try_fold(None, |acc, bbox| {
            let bbox = bbox?;
            Ok(Some(acc.map_or(bbox, |current| union_bbox(current, bbox))))
        })
}

fn render_entity_bbox(project: &ProjectSource, entity: &Entity) -> Option<BBox> {
    match entity {
        Entity::Text {
            style,
            at,
            rotation_deg,
            mirror_y,
            value,
            ..
        } => {
            let text_style = project.styles.text_styles.get(style)?;
            text_bbox(
                at,
                *rotation_deg,
                *mirror_y,
                value,
                text_style,
                text_anchor(&text_style.align),
            )
        }
        Entity::Dimension {
            style,
            p1,
            p2,
            offset,
            text_rotation_deg,
            text_mirror_y,
            value,
            ..
        } => {
            let dimension_style = project.styles.dimension_styles.get(style)?;
            let text_style = project
                .styles
                .text_styles
                .get(&dimension_style.text_style)?;
            let (d1, d2) = cad_model::dimension_offset_segment(*p1, *p2, *offset)?;
            let label = value.clone().unwrap_or_else(|| {
                cad_model::format_decimal_mm(
                    ((p2[0] - p1[0]).powi(2) + (p2[1] - p1[1]).powi(2)).sqrt(),
                )
            });
            let line_bbox = BBox::from_points(&[*p1, *p2, d1, d2])?;
            let text_bbox = text_bbox(
                &[(d1[0] + d2[0]) / 2.0, (d1[1] + d2[1]) / 2.0],
                *text_rotation_deg,
                *text_mirror_y,
                &label,
                text_style,
                "middle",
            )?;
            Some(union_bbox(line_bbox, text_bbox))
        }
        _ => entity_bbox(entity),
    }
}

fn text_bbox(
    at: &Point,
    rotation_deg: f64,
    mirror_y: bool,
    value: &str,
    style: &TextStyleDef,
    anchor: &str,
) -> Option<BBox> {
    let width = text_length(value, style);
    let height = style.height.max(0.0);
    if !at[0].is_finite()
        || !at[1].is_finite()
        || !rotation_deg.is_finite()
        || !width.is_finite()
        || !height.is_finite()
    {
        return None;
    }
    let x0 = match anchor {
        "middle" => at[0] - (width / 2.0),
        "end" => at[0] - width,
        _ => at[0],
    };
    let (y0, y1) = if mirror_y {
        (at[1] - height, at[1])
    } else {
        (at[1], at[1] + height)
    };
    let corners = [[x0, y0], [x0 + width, y0], [x0, y1], [x0 + width, y1]];
    let rotated = corners.map(|point| rotate_point(point, *at, rotation_deg));
    BBox::from_points(&rotated)
}

fn union_bbox(left: BBox, right: BBox) -> BBox {
    BBox {
        min: [left.min[0].min(right.min[0]), left.min[1].min(right.min[1])],
        max: [left.max[0].max(right.max[0]), left.max[1].max(right.max[1])],
    }
}

fn rotate_point(point: Point, origin: Point, rotation_deg: f64) -> Point {
    let rad = rotation_deg.to_radians();
    let cos = rad.cos();
    let sin = rad.sin();
    let dx = point[0] - origin[0];
    let dy = point[1] - origin[1];
    [
        origin[0] + (dx * cos) - (dy * sin),
        origin[1] + (dx * sin) + (dy * cos),
    ]
}

fn bbox_view_box(bbox: BBox) -> (f64, f64, f64, f64) {
    let largest = bbox.width().max(bbox.height());
    let padding = 100.0_f64.max(largest * 0.05);
    (
        bbox.min[0] - padding,
        -bbox.max[1] - padding,
        bbox.width() + (padding * 2.0),
        bbox.height() + (padding * 2.0),
    )
}

fn paper_size_mm(paper: &str) -> RenderResult<(f64, f64)> {
    match paper {
        "A0" => Ok((841.0, 1189.0)),
        "A1" => Ok((594.0, 841.0)),
        "A2" => Ok((420.0, 594.0)),
        "A3" => Ok((297.0, 420.0)),
        "A4" => Ok((210.0, 297.0)),
        _ => Err(RenderError::UnsupportedPaper {
            paper: paper.to_owned(),
        }),
    }
}

fn parse_scale(scale: &str) -> RenderResult<f64> {
    let Some((numerator, denominator)) = scale.split_once('/') else {
        return Err(RenderError::UnsupportedScale {
            scale: scale.to_owned(),
        });
    };
    let Ok(numerator) = numerator.parse::<f64>() else {
        return Err(RenderError::UnsupportedScale {
            scale: scale.to_owned(),
        });
    };
    let Ok(denominator) = denominator.parse::<f64>() else {
        return Err(RenderError::UnsupportedScale {
            scale: scale.to_owned(),
        });
    };
    if numerator <= 0.0 || denominator <= 0.0 {
        return Err(RenderError::UnsupportedScale {
            scale: scale.to_owned(),
        });
    }
    Ok(denominator / numerator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{create_dir_all, write};

    #[test]
    fn exposes_crate_name() {
        assert_eq!(crate_name(), "cad-render-svg");
    }

    #[test]
    fn renders_house_small_with_entity_metadata() {
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let project = cad_model::load_project(root).expect("example should load");

        let svg = render_project_svg(&project).expect("svg should render");

        assert!(svg.contains("<svg"));
        assert!(svg.contains("data-entity-id=\"ent_01JZ0000000000000000000000\""));
        assert!(svg.contains("data-layer=\"0-1\""));
        assert!(svg.contains("data-bbox=\"0,0,910,0\""));
    }

    #[test]
    fn renders_representative_entities_snapshot() {
        let temp = fixture_project();
        let project = cad_model::load_project(temp.path()).expect("fixture should load");

        let svg = render_project_svg(&project).expect("svg should render");

        insta::assert_snapshot!(svg, @r##"
<svg data-drawing="plan_1f" data-paper-height-mm="297" data-paper-width-mm="420" height="100%" preserveAspectRatio="xMinYMin meet" viewBox="-2625 -31825 46750 34450" width="100%" xmlns="http://www.w3.org/2000/svg">
<g id="plan_1f">
<g data-bbox="0,0,910,0" data-entity-id="ent_01JZ0000000000000000000000" data-layer="0-1">
<line fill="none" stroke="#000000" stroke-width="0.25mm" x1="0" x2="910" y1="0" y2="0"/>
</g>
<g data-bbox="-500,-500,500,500" data-entity-id="ent_01JZ0000000000000000000001" data-layer="0-1">
<path d="M500,0 A500,500,0,0,0,0,-500" fill="none" stroke="#000000" stroke-width="0.25mm"/>
</g>
<g data-bbox="100,200,600,450" data-entity-id="ent_01JZ0000000000000000000002" data-layer="0-1">
<text fill="#000000" font-family="Hiragino Sans" font-size="250" lengthAdjust="spacingAndGlyphs" text-anchor="start" textLength="500" transform="rotate(0 100 -200)" x="100" y="-200">

note
</text>
</g>
<g data-bbox="0,0,910,370" data-entity-id="ent_01JZ0000000000000000000003" data-layer="0-1">
<line fill="none" stroke="#000000" stroke-width="0.25mm" x1="0" x2="910" y1="-120" y2="-120"/>
<text fill="#000000" font-family="Hiragino Sans" font-size="250" lengthAdjust="spacingAndGlyphs" text-anchor="middle" textLength="375" transform="rotate(0 455 -120)" x="455" y="-120">

910
</text>
</g>
<g data-bbox="455,0,455,0" data-entity-id="ent_01JZ0000000000000000000004" data-layer="0-1">
<line stroke="#000000" stroke-width="0.25mm" x1="355" x2="555" y1="0" y2="0"/>
<line stroke="#000000" stroke-width="0.25mm" x1="455" x2="455" y1="100" y2="-100"/>
<text fill="#000000" font-size="250" text-anchor="middle" transform="rotate(0 455 0)" x="455" y="-150">

door_910
</text>
</g>
</g>
</svg>
        "##);
    }

    #[test]
    fn renders_full_and_partial_ellipses() {
        let temp = fixture_project();
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            [
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"ellipse","layer":"0-1","center":[10.0,20.0],"radius_x":4.0,"radius_y":2.0,"rotation_deg":30.0,"start_deg":0.0,"end_deg":360.0}"#,
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000001","type":"ellipse","layer":"0-1","center":[0.0,0.0],"radius_x":8.0,"radius_y":3.0,"rotation_deg":45.0,"start_deg":0.0,"end_deg":180.0}"#,
            ]
            .join("\n"),
        )
        .expect("ellipse entities should be writable");
        let project = cad_model::load_project(temp.path()).expect("fixture should load");

        let svg = render_project_svg(&project).expect("ellipses should render");

        assert!(svg.contains("<ellipse"));
        assert!(svg.contains("transform=\"rotate(-30 10 -20)\""));
        assert!(svg.contains("A8,3,-45,0,0"));
        assert!(svg.contains("data-bbox="));
    }

    #[test]
    fn renders_mirrored_text_and_rotated_dimension_geometry() {
        let temp = fixture_project();
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            [
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"text","layer":"0-1","style":"note","at":[10.0,20.0],"rotation_deg":30.0,"mirror_y":true,"value":"mirror"}"#,
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000001","type":"dimension","layer":"0-1","style":"dim_100","p1":[0.0,0.0],"p2":[0.0,10.0],"offset":2.0,"text_rotation_deg":90.0,"text_mirror_y":true,"value":"10"}"#,
            ]
            .join("\n"),
        )
        .expect("mirrored entities should be writable");
        let project = cad_model::load_project(temp.path()).expect("fixture should load");

        let svg = render_project_svg(&project).expect("mirrored entities should render");

        assert_eq!(svg.matches("transform=\"matrix(").count(), 2);
        assert!(svg.contains("x1=\"-2\" x2=\"-2\""));
        assert!(svg.contains("data-bbox="));
    }

    fn fixture_project() -> tempfile::TempDir {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        create_dir_all(temp.path().join("rules")).expect("rules dir should be created");
        create_dir_all(temp.path().join("drawings/plan_1f"))
            .expect("drawing dir should be created");
        create_dir_all(temp.path().join("blocks/door_910")).expect("block dir should be created");

        write(
            temp.path().join("cad.project.toml"),
            "schema_version = \"0.1\"\nname = \"fixture\"\n",
        )
        .expect("project TOML should be writable");
        write(
            temp.path().join("rules/layers.toml"),
            "[layers.\"0-1\"]\nname = \"A-WALL\"\nvisible = true\nprintable = true\ncolor = \"jw_black\"\nline_type = \"solid\"\nline_width = 0.25\n",
        )
        .expect("layers TOML should be writable");
        write(
            temp.path().join("rules/styles.toml"),
            "[colors.jw_black]\nrgb = \"#000000\"\nprint_width = 0.25\n\n[line_types.solid]\ndash = []\n\n[text_styles.note]\nfont_family = \"Hiragino Sans\"\nheight = 250\nwidth = 125\nspacing = 0\nalign = \"left\"\n\n[dimension_styles.dim_100]\ntext_style = \"note\"\narrow_size = 120\nextension_gap = 40\nprecision = 0\nunit = \"mm\"\n",
        )
        .expect("styles TOML should be writable");
        write(
            temp.path().join("drawings/plan_1f/sheet.toml"),
            "schema_version = \"0.1\"\npaper = \"A3\"\norientation = \"landscape\"\nscale = \"1/100\"\norigin = [0.0, 0.0]\n",
        )
        .expect("sheet TOML should be writable");
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            [
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0.0,0.0],"p2":[910.0,0.0]}"#,
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000001","type":"arc","layer":"0-1","center":[0.0,0.0],"radius":500.0,"start_deg":0.0,"end_deg":90.0}"#,
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000002","type":"text","layer":"0-1","style":"note","at":[100.0,200.0],"rotation_deg":0.0,"value":"note"}"#,
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000003","type":"dimension","layer":"0-1","style":"dim_100","p1":[0.0,0.0],"p2":[910.0,0.0],"offset":120.0,"value":null}"#,
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000004","type":"block_ref","layer":"0-1","block":"door_910","at":[455.0,0.0],"rotation_deg":0.0,"scale":1.0}"#,
            ]
            .join("\n"),
        )
        .expect("entities NDJSON should be writable");

        temp
    }
}
