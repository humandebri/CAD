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
use svg::node::element::{
    Circle, Ellipse as SvgEllipse, Group, Line, Path, Polyline, Rectangle, Text,
};
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
    #[error("drawing has no active layout")]
    MissingActiveLayout,
    #[error("unsupported paper {paper:?}")]
    UnsupportedPaper { paper: String },
    #[error("unsupported scale {scale:?}")]
    UnsupportedScale { scale: String },
    #[error("entity {entity_id:?} references missing layer {layer:?}")]
    MissingLayer { entity_id: String, layer: String },
    #[error("entity {entity_id:?} references missing color {color:?}")]
    MissingColor { entity_id: String, color: String },
    #[error("entity {entity_id:?} references missing pen {pen:?}")]
    MissingPen { entity_id: String, pen: String },
    #[error("entity {entity_id:?} references missing line type {line_type:?}")]
    MissingLineType {
        entity_id: String,
        line_type: String,
    },
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
    if let Some(layout) = drawing.layouts.active() {
        let (paper_width, paper_height) = paper_size_mm(&layout.paper)?;
        let (paper_width, paper_height) = match layout.orientation {
            cad_model::SheetOrientation::Portrait => (paper_width, paper_height),
            cad_model::SheetOrientation::Landscape => (paper_height, paper_width),
        };
        let scale = parse_scale(&layout.scale)?;
        root = root.add(
            Rectangle::new()
                .set("data-layout", layout.name.clone())
                .set("data-paper-frame", true)
                .set("x", layout.origin[0])
                .set("y", svg_y(layout.origin[1] + paper_height * scale))
                .set("width", paper_width * scale)
                .set("height", paper_height * scale)
                .set("fill", "none")
                .set("stroke", "#888")
                .set("stroke-dasharray", "8 4"),
        );
        if let Some([x1, y1, x2, y2]) = layout.plot_area {
            root = root.add(
                Rectangle::new()
                    .set("data-layout", layout.name.clone())
                    .set("data-plot-area", true)
                    .set("x", x1)
                    .set("y", svg_y(y2))
                    .set("width", (x2 - x1) * scale)
                    .set("height", (y2 - y1) * scale)
                    .set("fill", "none")
                    .set("stroke", "#3b82f6")
                    .set("stroke-dasharray", "4 3"),
            );
        }
    }
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
    render_entity_with_depth(project, record, 0)
}

fn render_entity_with_depth(
    project: &ProjectSource,
    record: &EntityRecord,
    depth: usize,
) -> RenderResult<Group> {
    let entity_id = record.entity.id().as_str().to_owned();
    let layer = resolve_layer(project, &record.entity)?;
    let layer_group = layer
        .group
        .as_ref()
        .and_then(|group_id| project.layers.groups.get(group_id));
    let visible = layer.visible && layer_group.is_none_or(|group| group.visible);
    let locked = layer.locked || layer_group.is_some_and(|group| group.locked);
    let stroke = resolve_stroke(project, &record.entity, layer)?;
    let bbox =
        render_entity_bbox(project, &record.entity).ok_or_else(|| RenderError::MissingBBox {
            entity_id: entity_id.clone(),
        })?;

    let mut group = Group::new()
        .set("data-layer", record.entity.layer())
        .set("data-layer-visible", visible)
        .set("data-layer-locked", locked)
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
    group = if depth == 0 {
        group.set("data-entity-id", entity_id)
    } else {
        group.set("data-block-child-id", entity_id)
    };
    if !stroke.dash.is_empty() {
        group = group.set("stroke-dasharray", stroke.dash_attr());
    }
    if !visible {
        group = group.set("style", "display:none");
    }

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
            let measured = ((p2[0] - p1[0]).powi(2) + (p2[1] - p1[1]).powi(2)).sqrt();
            let label = value.clone().unwrap_or_else(|| {
                format!(
                    "{:.*} {}",
                    usize::from(dimension_style.precision),
                    measured,
                    dimension_style.unit
                )
            });
            let extension_start = |source: Point, target: Point| {
                let dx = target[0] - source[0];
                let dy = target[1] - source[1];
                let length = (dx * dx + dy * dy).sqrt();
                if length <= f64::EPSILON {
                    source
                } else {
                    let gap = dimension_style.extension_gap.min(length);
                    [source[0] + dx / length * gap, source[1] + dy / length * gap]
                }
            };
            let e1 = extension_start(*p1, d1);
            let e2 = extension_start(*p2, d2);
            group = group
                .add(stroked_line(e1, d1, &stroke))
                .add(stroked_line(e2, d2, &stroke))
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
                .add(dimension_arrow(
                    d1,
                    d2,
                    dimension_style.arrow_size,
                    &stroke.color,
                ))
                .add(dimension_arrow(
                    d2,
                    d1,
                    dimension_style.arrow_size,
                    &stroke.color,
                ))
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
        Entity::Point {
            at,
            marker_code,
            rotation_deg,
            scale,
            ..
        } => {
            let size = (2.5 * scale.abs()).max(0.5);
            if marker_code.is_none() {
                group = group.add(
                    Circle::new()
                        .set("cx", at[0])
                        .set("cy", svg_y(at[1]))
                        .set("r", size / 2.0)
                        .set("fill", stroke.color.clone())
                        .set("stroke", "none"),
                );
            } else {
                group = group
                    .add(
                        Line::new()
                            .set("x1", at[0] - size)
                            .set("y1", svg_y(at[1]))
                            .set("x2", at[0] + size)
                            .set("y2", svg_y(at[1]))
                            .set("transform", rotate_attr(*rotation_deg, *at))
                            .set("stroke", stroke.color.clone())
                            .set("stroke-width", stroke.width_attr()),
                    )
                    .add(
                        Line::new()
                            .set("x1", at[0])
                            .set("y1", svg_y(at[1] - size))
                            .set("x2", at[0])
                            .set("y2", svg_y(at[1] + size))
                            .set("transform", rotate_attr(*rotation_deg, *at))
                            .set("stroke", stroke.color.clone())
                            .set("stroke-width", stroke.width_attr()),
                    );
            }
        }
        Entity::Solid { points, fill, .. } => {
            let fill = resolve_fill(project, &record.entity, fill)?;
            let Some(first) = points.first() else {
                return Err(RenderError::MissingBBox {
                    entity_id: record.entity.id().as_str().to_owned(),
                });
            };
            let mut data = Data::new().move_to((first[0], svg_y(first[1])));
            for point in points.iter().skip(1) {
                data = data.line_to((point[0], svg_y(point[1])));
            }
            group = group.add(
                Path::new()
                    .set("d", data.close())
                    .set("fill", fill)
                    .set("stroke", "none"),
            );
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
            let fill = resolve_fill(project, &record.entity, fill)?;
            group = group.add(curve_solid_path(
                CurveSolidGeometry {
                    center: *center,
                    radius_x: radius.abs(),
                    radius_y: radius.abs() * flatness.abs(),
                    rotation_deg: *rotation_deg,
                    start_deg: *start_deg,
                    end_deg: *end_deg,
                    solid_param: *solid_param,
                },
                &fill,
            ));
        }
        Entity::BlockRef {
            block,
            at,
            rotation_deg,
            scale,
            ..
        } => {
            if !project.blocks.contains_key(block) {
                let size = 100.0 * scale;
                group = group
                    .add(
                        Line::new()
                            .set("x1", at[0] - size)
                            .set("y1", svg_y(at[1]))
                            .set("x2", at[0] + size)
                            .set("y2", svg_y(at[1]))
                            .set("transform", rotate_attr(*rotation_deg, *at))
                            .set("stroke", stroke.color.clone())
                            .set("stroke-width", stroke.width_attr()),
                    )
                    .add(
                        Line::new()
                            .set("x1", at[0])
                            .set("y1", svg_y(at[1] - size))
                            .set("x2", at[0])
                            .set("y2", svg_y(at[1] + size))
                            .set("transform", rotate_attr(*rotation_deg, *at))
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
            } else {
                let definition = project.blocks.get(block).expect("block checked");
                let base_point = definition.config.base_point;
                let mut block_group = Group::new()
                    .set("data-block-id", block.clone())
                    .set("data-block-reference-id", record.entity.id().as_str())
                    .set(
                        "transform",
                        format!(
                            "translate({} {}) rotate({}) scale({}) translate({} {})",
                            at[0],
                            svg_y(at[1]),
                            normalize_zero(-rotation_deg),
                            scale,
                            normalize_zero(-base_point[0]),
                            normalize_zero(base_point[1]),
                        ),
                    );
                if depth < 32 {
                    for child in &definition.entities {
                        block_group =
                            block_group.add(render_entity_with_depth(project, child, depth + 1)?);
                    }
                } else {
                    block_group = block_group.add(block_marker(block, 1.0, &stroke));
                }
                group = group.add(block_group);
            }
        }
        Entity::Hatch { loops, fill, .. } => {
            let fill_color = fill
                .as_deref()
                .map(|fill| resolve_fill(project, &record.entity, fill))
                .transpose()?
                .unwrap_or_else(|| "none".to_owned());
            let mut data = Data::new();
            let mut has_loop = false;
            for loop_points in loops {
                let Some(first) = loop_points.first() else {
                    continue;
                };
                has_loop = true;
                data = data.move_to((first[0], svg_y(first[1])));
                for point in loop_points.iter().skip(1) {
                    data = data.line_to((point[0], svg_y(point[1])));
                }
                data = data.close();
            }
            if has_loop {
                group = group.add(
                    Path::new()
                        .set("d", data)
                        .set("fill", fill_color)
                        .set("fill-rule", "evenodd")
                        .set("stroke", stroke.color.clone())
                        .set("stroke-width", stroke.width_attr()),
                );
            }
        }
    }

    Ok(group)
}

fn stroked_line(start: Point, end: Point, stroke: &Stroke) -> Line {
    Line::new()
        .set("x1", start[0])
        .set("y1", svg_y(start[1]))
        .set("x2", end[0])
        .set("y2", svg_y(end[1]))
        .set("fill", "none")
        .set("stroke", stroke.color.clone())
        .set("stroke-width", stroke.width_attr())
}

fn block_marker(block: &str, scale: f64, stroke: &Stroke) -> Group {
    let size = 100.0 * scale;
    Group::new()
        .add(
            Line::new()
                .set("x1", -size)
                .set("y1", 0.0)
                .set("x2", size)
                .set("y2", 0.0)
                .set("stroke", stroke.color.clone())
                .set("stroke-width", stroke.width_attr()),
        )
        .add(
            Line::new()
                .set("x1", 0.0)
                .set("y1", -size)
                .set("x2", 0.0)
                .set("y2", size)
                .set("stroke", stroke.color.clone())
                .set("stroke-width", stroke.width_attr()),
        )
        .add(
            Text::new("")
                .set("x", 0.0)
                .set("y", size * 1.5)
                .set("font-size", 250.0 * scale)
                .set("text-anchor", "middle")
                .set("fill", stroke.color.clone())
                .add(svg::node::Text::new(block.to_owned())),
        )
}

fn dimension_arrow(tip: Point, toward: Point, size: f64, color: &str) -> Path {
    let dx = toward[0] - tip[0];
    let dy = toward[1] - tip[1];
    let length = (dx * dx + dy * dy).sqrt();
    if length <= f64::EPSILON || !size.is_finite() || size <= 0.0 {
        return Path::new();
    }
    let ux = dx / length;
    let uy = dy / length;
    let base = [tip[0] + ux * size, tip[1] + uy * size];
    let half = size * 0.35;
    let left = [base[0] - uy * half, base[1] + ux * half];
    let right = [base[0] + uy * half, base[1] - ux * half];
    Path::new()
        .set(
            "d",
            Data::new()
                .move_to((tip[0], svg_y(tip[1])))
                .line_to((left[0], svg_y(left[1])))
                .line_to((right[0], svg_y(right[1])))
                .close(),
        )
        .set("fill", color)
        .set("stroke", "none")
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
    entity: &Entity,
    _layer: &LayerDef,
) -> RenderResult<Stroke> {
    let resolved = cad_model::resolve_entity_stroke(project, entity).map_err(style_error)?;
    Ok(Stroke {
        color: resolved.color_rgb,
        width: resolved.line_width_mm,
        dash: resolved.dash,
    })
}

fn resolve_fill(project: &ProjectSource, entity: &Entity, fill: &str) -> RenderResult<String> {
    cad_model::resolve_fill_color(project, entity, fill).map_err(style_error)
}

fn style_error(error: cad_model::StyleResolutionError) -> RenderError {
    match error {
        cad_model::StyleResolutionError::MissingLayer { entity_id, layer } => {
            RenderError::MissingLayer { entity_id, layer }
        }
        cad_model::StyleResolutionError::MissingPen { entity_id, pen } => {
            RenderError::MissingPen { entity_id, pen }
        }
        cad_model::StyleResolutionError::MissingColor { entity_id, color } => {
            RenderError::MissingColor { entity_id, color }
        }
        cad_model::StyleResolutionError::MissingLineType {
            entity_id,
            line_type,
        } => RenderError::MissingLineType {
            entity_id,
            line_type,
        },
    }
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
    let count = value.chars().count();
    ((count as f64) * style.width + (count.saturating_sub(1) as f64) * style.spacing).max(0.0)
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
    let mut data = Data::new().move_to((start[0], svg_y(start[1])));
    if delta >= 360.0 - 1e-9 {
        let midpoint = polar_point(
            center,
            radius,
            start_deg + if end_deg >= start_deg { 180.0 } else { -180.0 },
        );
        data = data
            .elliptical_arc_to((radius, radius, 0, 1, sweep, midpoint[0], svg_y(midpoint[1])))
            .elliptical_arc_to((radius, radius, 0, 1, sweep, start[0], svg_y(start[1])));
    } else {
        data = data.elliptical_arc_to((radius, radius, 0, large_arc, sweep, end[0], svg_y(end[1])));
    }
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

struct CurveSolidGeometry {
    center: Point,
    radius_x: f64,
    radius_y: f64,
    rotation_deg: f64,
    start_deg: f64,
    end_deg: f64,
    solid_param: f64,
}

fn curve_solid_path(geometry: CurveSolidGeometry, fill: &str) -> Path {
    let CurveSolidGeometry {
        center,
        radius_x,
        radius_y,
        rotation_deg,
        start_deg,
        end_deg,
        solid_param,
    } = geometry;
    let full = (end_deg - start_deg).abs() >= 360.0 - 1e-9;
    let start = cad_model::ellipse_point(center, radius_x, radius_y, rotation_deg, start_deg);
    let end = cad_model::ellipse_point(center, radius_x, radius_y, rotation_deg, end_deg);
    let delta = (end_deg - start_deg).abs();
    let large_arc = i32::from(delta > 180.0);
    let sweep = i32::from(end_deg < start_deg);
    let mut data = Data::new().move_to((start[0], svg_y(start[1])));
    if full {
        let midpoint =
            cad_model::ellipse_point(center, radius_x, radius_y, rotation_deg, start_deg + 180.0);
        data = data
            .elliptical_arc_to((
                radius_x,
                radius_y,
                normalize_zero(-rotation_deg),
                1,
                sweep,
                midpoint[0],
                svg_y(midpoint[1]),
            ))
            .elliptical_arc_to((
                radius_x,
                radius_y,
                normalize_zero(-rotation_deg),
                1,
                sweep,
                start[0],
                svg_y(start[1]),
            ));
    } else {
        data = data.elliptical_arc_to((
            radius_x,
            radius_y,
            normalize_zero(-rotation_deg),
            large_arc,
            sweep,
            end[0],
            svg_y(end[1]),
        ));
    }
    if solid_param > 0.0 && solid_param < radius_x {
        let ratio = solid_param / radius_x;
        let inner_x = solid_param;
        let inner_y = radius_y * ratio;
        let inner_end = cad_model::ellipse_point(center, inner_x, inner_y, rotation_deg, end_deg);
        let inner_start =
            cad_model::ellipse_point(center, inner_x, inner_y, rotation_deg, start_deg);
        data = data.line_to((inner_end[0], svg_y(inner_end[1])));
        if full {
            let inner_midpoint =
                cad_model::ellipse_point(center, inner_x, inner_y, rotation_deg, start_deg + 180.0);
            data = data
                .elliptical_arc_to((
                    inner_x,
                    inner_y,
                    normalize_zero(-rotation_deg),
                    1,
                    1 - sweep,
                    inner_midpoint[0],
                    svg_y(inner_midpoint[1]),
                ))
                .elliptical_arc_to((
                    inner_x,
                    inner_y,
                    normalize_zero(-rotation_deg),
                    1,
                    1 - sweep,
                    inner_start[0],
                    svg_y(inner_start[1]),
                ));
        } else {
            data = data.elliptical_arc_to((
                inner_x,
                inner_y,
                normalize_zero(-rotation_deg),
                large_arc,
                1 - sweep,
                inner_start[0],
                svg_y(inner_start[1]),
            ));
        }
        data = data.close();
    } else if full {
        data = data.close();
    } else {
        data = data.line_to((center[0], svg_y(center[1]))).close();
    }
    Path::new()
        .set("d", data)
        .set("fill", fill)
        .set("fill-rule", "evenodd")
        .set("stroke", "none")
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
    dash: Vec<f64>,
}

impl Stroke {
    fn width_attr(&self) -> String {
        format!("{}mm", cad_model::format_decimal_mm(self.width))
    }

    fn dash_attr(&self) -> String {
        self.dash
            .iter()
            .map(|value| cad_model::format_decimal_mm(*value))
            .collect::<Vec<_>>()
            .join(" ")
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
        let layout = drawing
            .layouts
            .active()
            .ok_or(RenderError::MissingActiveLayout)?;
        let (paper_width_mm, paper_height_mm) = paper_size_mm(&layout.paper)?;
        let (paper_width_mm, paper_height_mm) = match layout.orientation {
            cad_model::SheetOrientation::Portrait => (paper_width_mm, paper_height_mm),
            cad_model::SheetOrientation::Landscape => (paper_height_mm, paper_width_mm),
        };
        let scale = parse_scale(&layout.scale)?;
        let fallback = BBox {
            min: layout.origin,
            max: [
                layout.origin[0] + (paper_width_mm * scale),
                layout.origin[1] + (paper_height_mm * scale),
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
    render_entity_bbox_with_depth(project, entity, 0)
}

fn render_entity_bbox_with_depth(
    project: &ProjectSource,
    entity: &Entity,
    depth: usize,
) -> Option<BBox> {
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
        Entity::BlockRef {
            block,
            at,
            rotation_deg,
            scale,
            ..
        } => {
            if depth >= 32 {
                return entity_bbox(entity);
            }
            let Some(definition) = project.blocks.get(block) else {
                return entity_bbox(entity);
            };
            let mut bounds = None;
            for child in &definition.entities {
                let child_bbox = render_entity_bbox_with_depth(project, &child.entity, depth + 1)?;
                let corners = [
                    [child_bbox.min[0], child_bbox.min[1]],
                    [child_bbox.min[0], child_bbox.max[1]],
                    [child_bbox.max[0], child_bbox.min[1]],
                    [child_bbox.max[0], child_bbox.max[1]],
                ];
                let transformed = corners.map(|point| {
                    let local = [
                        point[0] - definition.config.base_point[0],
                        point[1] - definition.config.base_point[1],
                    ];
                    rotate_point(
                        [at[0] + local[0] * scale, at[1] + local[1] * scale],
                        *at,
                        *rotation_deg,
                    )
                });
                if let Some(child_bbox) = BBox::from_points(&transformed) {
                    bounds =
                        Some(bounds.map_or(child_bbox, |current| union_bbox(current, child_bbox)));
                }
            }
            bounds.or_else(|| entity_bbox(entity))
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
    let scale = scale.trim();
    let Some((numerator, denominator)) = scale.split_once('/').or_else(|| scale.split_once(':'))
    else {
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
    fn group_visibility_and_lock_are_applied_without_changing_layer_flags() {
        let root =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let mut project = cad_model::load_project(root).expect("example should load");
        project.layers.groups.insert(
            "group-a".to_owned(),
            cad_model::LayerGroupDef {
                name: "Group A".to_owned(),
                order: 0,
                scale_denominator: 100.0,
                visible: false,
                locked: true,
            },
        );
        {
            let layer = project.layers.layers.get_mut("0-1").expect("fixture layer");
            layer.group = Some("group-a".to_owned());
            layer.visible = true;
            layer.locked = false;
        }

        let svg = render_project_svg(&project).expect("svg should render");

        assert!(svg.contains("data-layer-locked=\"true\""));
        assert!(svg.contains("data-layer-visible=\"false\""));
        assert!(svg.contains("style=\"display:none\""));
        let layer = project.layers.layers.get("0-1").expect("fixture layer");
        assert!(layer.visible);
        assert!(!layer.locked);
    }

    #[test]
    fn renders_representative_entities_snapshot() {
        let temp = fixture_project();
        let project = cad_model::load_project(temp.path()).expect("fixture should load");

        let svg = render_project_svg(&project).expect("svg should render");

        insta::assert_snapshot!(svg, @r##"
<svg data-drawing="plan_1f" data-paper-height-mm="297" data-paper-width-mm="420" height="100%" preserveAspectRatio="xMinYMin meet" viewBox="-2625 -31825 46750 34450" width="100%" xmlns="http://www.w3.org/2000/svg">
<g id="plan_1f">
<rect data-layout="default" data-paper-frame="true" fill="none" height="29700" stroke="#888" stroke-dasharray="8 4" width="42000" x="0" y="-29700"/>
<g data-bbox="0,0,910,0" data-entity-id="ent_01JZ0000000000000000000000" data-layer="0-1" data-layer-locked="false" data-layer-visible="true">
<line fill="none" stroke="#000000" stroke-width="0.25mm" x1="0" x2="910" y1="0" y2="0"/>
</g>
<g data-bbox="-500,-500,500,500" data-entity-id="ent_01JZ0000000000000000000001" data-layer="0-1" data-layer-locked="false" data-layer-visible="true">
<path d="M500,0 A500,500,0,0,0,0,-500" fill="none" stroke="#000000" stroke-width="0.25mm"/>
</g>
<g data-bbox="100,200,600,450" data-entity-id="ent_01JZ0000000000000000000002" data-layer="0-1" data-layer-locked="false" data-layer-visible="true">
<text fill="#000000" font-family="Hiragino Sans" font-size="250" lengthAdjust="spacingAndGlyphs" text-anchor="start" textLength="500" transform="rotate(0 100 -200)" x="100" y="-200">

note
</text>
</g>
<g data-bbox="0,0,910,370" data-entity-id="ent_01JZ0000000000000000000003" data-layer="0-1" data-layer-locked="false" data-layer-visible="true">
<line fill="none" stroke="#000000" stroke-width="0.25mm" x1="0" x2="0" y1="-40" y2="-120"/>
<line fill="none" stroke="#000000" stroke-width="0.25mm" x1="910" x2="910" y1="-40" y2="-120"/>
<line fill="none" stroke="#000000" stroke-width="0.25mm" x1="0" x2="910" y1="-120" y2="-120"/>
<path d="M0,-120 L120,-162 L120,-78 z" fill="#000000" stroke="none"/>
<path d="M910,-120 L790,-78 L790,-162 z" fill="#000000" stroke="none"/>
<text fill="#000000" font-family="Hiragino Sans" font-size="250" lengthAdjust="spacingAndGlyphs" text-anchor="middle" textLength="750" transform="rotate(0 455 -120)" x="455" y="-120">

910 mm
</text>
</g>
<g data-bbox="452.5,-2.5,457.5,2.5" data-entity-id="ent_01JZ0000000000000000000004" data-layer="0-1" data-layer-locked="false" data-layer-visible="true">
<line stroke="#000000" stroke-width="0.25mm" transform="rotate(0 455 0)" x1="355" x2="555" y1="0" y2="0"/>
<line stroke="#000000" stroke-width="0.25mm" transform="rotate(0 455 0)" x1="455" x2="455" y1="100" y2="-100"/>
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
                r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000000","type":"ellipse","layer":"0-1","center":[10.0,20.0],"radius_x":4.0,"radius_y":2.0,"rotation_deg":30.0,"start_deg":0.0,"end_deg":360.0}"#,
                r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000001","type":"ellipse","layer":"0-1","center":[0.0,0.0],"radius_x":8.0,"radius_y":3.0,"rotation_deg":45.0,"start_deg":0.0,"end_deg":180.0}"#,
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
    fn full_curve_solid_ring_uses_two_arcs_for_each_boundary() {
        let path = curve_solid_path(
            CurveSolidGeometry {
                center: [0.0, 0.0],
                radius_x: 10.0,
                radius_y: 5.0,
                rotation_deg: 0.0,
                start_deg: 0.0,
                end_deg: 360.0,
                solid_param: 4.0,
            },
            "#000000",
        )
        .to_string();

        assert_eq!(path.matches('A').count(), 4);
        assert!(path.contains("fill-rule=\"evenodd\""));
    }

    #[test]
    fn renders_mirrored_text_and_rotated_dimension_geometry() {
        let temp = fixture_project();
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            [
                r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000000","type":"text","layer":"0-1","style":"note","at":[10.0,20.0],"rotation_deg":30.0,"mirror_y":true,"value":"mirror"}"#,
                r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000001","type":"dimension","layer":"0-1","style":"dim_100","p1":[0.0,0.0],"p2":[0.0,10.0],"offset":2.0,"text_rotation_deg":90.0,"text_mirror_y":true,"value":"10"}"#,
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

    #[test]
    fn block_rendering_uses_base_point_and_keeps_children_non_interactive() {
        let temp = fixture_project();
        write(
            temp.path().join("blocks/door_910/definition.toml"),
            "schema_version = \"0.2\"\nname = \"Door\"\nbase_point = [10.0, 20.0]\n",
        )
        .expect("block definition");
        write(
            temp.path().join("blocks/door_910/entities.ndjson"),
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000099","type":"line","layer":"0-1","p1":[10.0,20.0],"p2":[20.0,20.0]}"#,
        )
        .expect("block entities");
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000004","type":"block_ref","layer":"0-1","block":"door_910","at":[100.0,200.0],"rotation_deg":0.0,"scale":2.0}"#,
        )
        .expect("block reference");
        let project = cad_model::load_project(temp.path()).expect("fixture should load");

        let svg = render_project_svg(&project).expect("block should render");

        assert!(
            svg.contains("transform=\"translate(100 -200) rotate(0) scale(2) translate(-10 20)\"")
        );
        assert!(svg.contains("data-bbox=\"100,200,120,200\""));
        assert!(svg.contains("data-block-child-id=\"ent_01JZ0000000000000000000099\""));
        assert!(!svg.contains("data-entity-id=\"ent_01JZ0000000000000000000099\""));
    }

    #[test]
    fn hatch_loops_share_one_evenodd_path() {
        let temp = fixture_project();
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            r##"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000008","type":"hatch","layer":"0-1","loops":[[[0.0,0.0],[10.0,0.0],[10.0,10.0],[0.0,10.0]],[[2.0,2.0],[2.0,8.0],[8.0,8.0],[8.0,2.0]]],"pattern":"solid","angle_deg":0.0,"scale":1.0,"fill":"jw_black"}"##,
        )
        .expect("hatch entity");
        let project = cad_model::load_project(temp.path()).expect("fixture should load");

        let svg = render_project_svg(&project).expect("hatch should render");

        assert_eq!(svg.matches("fill-rule=\"evenodd\"").count(), 1);
        let path = svg
            .split("fill-rule=\"evenodd\"")
            .next()
            .expect("hatch path");
        assert!(path.matches('M').count() >= 2);
    }

    fn fixture_project() -> tempfile::TempDir {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        create_dir_all(temp.path().join("rules")).expect("rules dir should be created");
        create_dir_all(temp.path().join("drawings/plan_1f"))
            .expect("drawing dir should be created");
        create_dir_all(temp.path().join("blocks/door_910")).expect("block dir should be created");

        write(
            temp.path().join("cad.project.toml"),
            "schema_version = \"0.2\"\nname = \"fixture\"\n",
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
            temp.path().join("drawings/plan_1f/layouts.toml"),
            "schema_version = \"0.2\"\nactive_layout = \"default\"\n\n[layouts.default]\nname = \"default\"\npaper = \"A3\"\norientation = \"landscape\"\nscale = \"1/100\"\norigin = [0.0, 0.0]\nmargins = [0.0, 0.0, 0.0, 0.0]\n",
        )
        .expect("layouts TOML should be writable");
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            [
                r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0.0,0.0],"p2":[910.0,0.0]}"#,
                r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000001","type":"arc","layer":"0-1","center":[0.0,0.0],"radius":500.0,"start_deg":0.0,"end_deg":90.0}"#,
                r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000002","type":"text","layer":"0-1","style":"note","at":[100.0,200.0],"rotation_deg":0.0,"value":"note"}"#,
                r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000003","type":"dimension","layer":"0-1","style":"dim_100","p1":[0.0,0.0],"p2":[910.0,0.0],"offset":120.0,"value":null}"#,
                r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000004","type":"block_ref","layer":"0-1","block":"door_910","at":[455.0,0.0],"rotation_deg":0.0,"scale":1.0}"#,
            ]
            .join("\n"),
        )
        .expect("entities NDJSON should be writable");

        temp
    }
}
