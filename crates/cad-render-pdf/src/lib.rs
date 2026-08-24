//! Direct PDF rendering for the canonical CAD drawing model.

use cad_model::{
    Entity, LayoutConfig, ProjectSource, ResolvedStroke, SheetOrientation, TextAlign, TextStyleDef,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const CRATE_NAME: &str = "cad-render-pdf";
const MM_TO_PT: f64 = 72.0 / 25.4;
const MAX_BLOCK_DEPTH: usize = 32;
const M_PLUS_REGULAR: &[u8] = include_bytes!("../assets/mplus/mplus-1p-regular.ttf");
const PDF_FONT_NAME: &str = "CADMPL+Mplus1p-Regular";

#[must_use]
pub fn crate_name() -> &'static str {
    CRATE_NAME
}

#[derive(Debug, Clone, Default)]
pub struct PdfExportOptions {
    pub overwrite: bool,
    pub expected_files: Vec<PdfFileRevision>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PdfFileRevision {
    pub relative_path: String,
    pub revision: String,
    pub exists: bool,
}

#[derive(Debug, Error)]
pub enum PdfError {
    #[error("project has no drawings")]
    NoDrawings,
    #[error("drawing {0:?} was not found")]
    MissingDrawing(String),
    #[error("layout {0:?} was not found")]
    MissingLayout(String),
    #[error("invalid layout: {0}")]
    InvalidLayout(String),
    #[error("project checker failed: {0}")]
    CheckFailed(String),
    #[error("PDF style resolution failed: {0}")]
    Style(String),
    #[error("output already exists: {0}")]
    OutputExists(PathBuf),
    #[error("failed to write {path}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub type PdfResult<T> = Result<T, PdfError>;

pub fn export_drawing_pdf(
    project_path: impl AsRef<Path>,
    drawing_name: &str,
    layout_name: Option<&str>,
    output_path: impl AsRef<Path>,
    options: PdfExportOptions,
) -> PdfResult<()> {
    let root = project_path.as_ref();
    let initial_manifest = cad_model::source_manifest(root)
        .map_err(|error| PdfError::CheckFailed(error.to_string()))?;
    let project =
        cad_model::load_project(root).map_err(|error| PdfError::CheckFailed(error.to_string()))?;
    validate_expected_files(root, &options.expected_files)?;
    let bytes = render_drawing_pdf(&project, drawing_name, layout_name)?;
    validate_expected_files(root, &options.expected_files)?;
    let final_manifest = cad_model::source_manifest(root)
        .map_err(|error| PdfError::CheckFailed(error.to_string()))?;
    if initial_manifest != final_manifest {
        return Err(PdfError::CheckFailed(
            "revision_conflict: project source changed during PDF export".to_owned(),
        ));
    }
    publish(output_path.as_ref(), &bytes, options.overwrite)
}

fn validate_expected_files(root: &Path, expected: &[PdfFileRevision]) -> PdfResult<()> {
    let canonical_root = fs::canonicalize(root).map_err(|error| PdfError::Write {
        path: root.to_path_buf(),
        source: error,
    })?;
    for file in expected {
        let relative = Path::new(&file.relative_path);
        if file.relative_path.is_empty()
            || relative.is_absolute()
            || relative
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(PdfError::CheckFailed(format!(
                "unsafe expected revision path {:?}",
                file.relative_path
            )));
        }
        let path = root.join(relative);
        if path.exists() {
            let canonical = fs::canonicalize(&path).map_err(|error| PdfError::Write {
                path: path.clone(),
                source: error,
            })?;
            if !canonical.starts_with(&canonical_root) {
                return Err(PdfError::CheckFailed(format!(
                    "expected revision path escapes the project: {}",
                    file.relative_path
                )));
            }
        }
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                return Err(PdfError::Write {
                    path,
                    source: error,
                });
            }
        };
        let exists = !bytes.is_empty() || path.exists();
        let revision = blake3::hash(&bytes).to_hex().to_string();
        if exists != file.exists || revision != file.revision {
            return Err(PdfError::CheckFailed(format!(
                "revision_conflict: {} changed during PDF export",
                file.relative_path
            )));
        }
    }
    Ok(())
}

pub fn render_drawing_pdf(
    project: &ProjectSource,
    drawing_name: &str,
    layout_name: Option<&str>,
) -> PdfResult<Vec<u8>> {
    let drawing = project
        .drawings
        .iter()
        .find(|drawing| drawing.name == drawing_name)
        .ok_or_else(|| PdfError::MissingDrawing(drawing_name.to_owned()))?;
    let layout = layout_name
        .map(|name| {
            drawing
                .layouts
                .layouts
                .get(name)
                .ok_or_else(|| PdfError::MissingLayout(name.to_owned()))
        })
        .transpose()?
        .or_else(|| drawing.layouts.active())
        .ok_or_else(|| PdfError::InvalidLayout("active layout is missing".to_owned()))?;
    let check = cad_check::check_loaded_project(project);
    if check
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.severity == cad_check::Severity::Error)
    {
        return Err(PdfError::CheckFailed(
            check
                .diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.severity == cad_check::Severity::Error)
                .map(|diagnostic| diagnostic.message.clone())
                .collect::<Vec<_>>()
                .join("; "),
        ));
    }
    let (paper_width, paper_height) = paper_size_mm(&layout.paper)?;
    let (paper_width, paper_height) = match layout.orientation {
        SheetOrientation::Portrait => (paper_width, paper_height),
        SheetOrientation::Landscape => (paper_height, paper_width),
    };
    validate_layout(layout, paper_width, paper_height)?;
    let scale = parse_scale(&layout.scale)?;
    let mut content = PdfContent::new(paper_width, paper_height, scale, layout)?;
    for record in &drawing.entities {
        if is_printable_entity(project, &record.entity) {
            content.entity(project, &record.entity, 0, Transform::identity())?;
        }
    }
    let (commands, font) = content.finish()?;
    build_pdf(paper_width, paper_height, &commands, &font)
}

fn is_printable_entity(project: &ProjectSource, entity: &Entity) -> bool {
    let Some(layer) = project.layers.layers.get(entity.layer()) else {
        return false;
    };
    if !layer.printable || !layer.visible {
        return false;
    }
    match layer
        .group
        .as_ref()
        .and_then(|group| project.layers.groups.get(group))
    {
        Some(group) => group.visible,
        None => true,
    }
}

fn validate_layout(layout: &LayoutConfig, paper_width: f64, paper_height: f64) -> PdfResult<()> {
    if layout
        .margins
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
    {
        return Err(PdfError::InvalidLayout(
            "margins must be finite and non-negative".to_owned(),
        ));
    }
    if layout.margins[0] + layout.margins[2] >= paper_width
        || layout.margins[1] + layout.margins[3] >= paper_height
    {
        return Err(PdfError::InvalidLayout(
            "margins must leave a positive printable page area".to_owned(),
        ));
    }
    if let Some(plot_area) = layout.plot_area
        && (plot_area.iter().any(|value| !value.is_finite())
            || plot_area[2] <= plot_area[0]
            || plot_area[3] <= plot_area[1])
    {
        return Err(PdfError::InvalidLayout(
            "plot_area must have positive finite bounds".to_owned(),
        ));
    }
    Ok(())
}

fn paper_size_mm(paper: &str) -> PdfResult<(f64, f64)> {
    match paper.to_ascii_uppercase().as_str() {
        "A0" => Ok((841.0, 1189.0)),
        "A1" => Ok((594.0, 841.0)),
        "A2" => Ok((420.0, 594.0)),
        "A3" => Ok((297.0, 420.0)),
        "A4" => Ok((210.0, 297.0)),
        _ => Err(PdfError::InvalidLayout(format!(
            "unsupported paper {paper:?}"
        ))),
    }
}

fn parse_scale(scale: &str) -> PdfResult<f64> {
    let scale = scale.trim();
    let value = scale
        .strip_prefix("1/")
        .and_then(|value| value.parse::<f64>().ok())
        .or_else(|| {
            scale
                .strip_prefix("1:")
                .and_then(|value| value.parse::<f64>().ok())
        })
        .or_else(|| scale.parse::<f64>().ok())
        .ok_or_else(|| PdfError::InvalidLayout(format!("invalid scale {scale:?}")))?;
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(PdfError::InvalidLayout(format!("invalid scale {scale:?}")))
    }
}

#[derive(Clone, Copy)]
struct Transform {
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    tx: f64,
    ty: f64,
}

impl Transform {
    const fn identity() -> Self {
        Self {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            tx: 0.0,
            ty: 0.0,
        }
    }

    fn block(at: [f64; 2], rotation: f64, scale: f64, base_point: [f64; 2]) -> Self {
        let angle = rotation.to_radians();
        let a = angle.cos() * scale;
        let b = angle.sin() * scale;
        let c = -angle.sin() * scale;
        let d = angle.cos() * scale;
        Self {
            a,
            b,
            c,
            d,
            tx: at[0] - a * base_point[0] - c * base_point[1],
            ty: at[1] - b * base_point[0] - d * base_point[1],
        }
    }

    fn compose(self, rhs: Self) -> Self {
        Self {
            a: self.a * rhs.a + self.c * rhs.b,
            b: self.b * rhs.a + self.d * rhs.b,
            c: self.a * rhs.c + self.c * rhs.d,
            d: self.b * rhs.c + self.d * rhs.d,
            tx: self.a * rhs.tx + self.c * rhs.ty + self.tx,
            ty: self.b * rhs.tx + self.d * rhs.ty + self.ty,
        }
    }

    fn point(self, point: [f64; 2]) -> [f64; 2] {
        [
            self.a * point[0] + self.c * point[1] + self.tx,
            self.b * point[0] + self.d * point[1] + self.ty,
        ]
    }
}

struct PdfContent<'a> {
    commands: Vec<String>,
    font: PdfFontSubset,
    scale: f64,
    layout: &'a LayoutConfig,
    paper_height: f64,
}

impl<'a> PdfContent<'a> {
    fn new(
        paper_width: f64,
        paper_height: f64,
        scale: f64,
        layout: &'a LayoutConfig,
    ) -> PdfResult<Self> {
        Ok(Self {
            commands: {
                let mut commands = Vec::new();
                let left = layout.margins[0] * MM_TO_PT;
                let bottom = layout.margins[3] * MM_TO_PT;
                let width =
                    paper_width * MM_TO_PT - (layout.margins[0] + layout.margins[2]) * MM_TO_PT;
                let height =
                    paper_height * MM_TO_PT - (layout.margins[1] + layout.margins[3]) * MM_TO_PT;
                commands.push(format!(
                    "q {} {} {} {} re W n",
                    fmt(left),
                    fmt(bottom),
                    fmt(width),
                    fmt(height)
                ));
                if let Some([x1, y1, x2, y2]) = layout.plot_area {
                    let to_pdf = |point: [f64; 2]| {
                        [
                            left + (point[0] - layout.origin[0]) * MM_TO_PT / scale,
                            paper_height * MM_TO_PT
                                - layout.margins[1] * MM_TO_PT
                                - (point[1] - layout.origin[1]) * MM_TO_PT / scale,
                        ]
                    };
                    let min = to_pdf([x1, y2]);
                    let max = to_pdf([x2, y1]);
                    commands.push(format!(
                        "{} {} {} {} re W n",
                        fmt(min[0]),
                        fmt(min[1]),
                        fmt(max[0] - min[0]),
                        fmt(max[1] - min[1])
                    ));
                }
                commands
            },
            font: PdfFontSubset::new()?,
            scale,
            layout,
            paper_height,
        })
    }

    fn finish(self) -> PdfResult<(Vec<String>, EmbeddedPdfFont)> {
        Ok((self.commands, self.font.finish()?))
    }

    fn point(&self, point: [f64; 2], transform: Transform) -> [f64; 2] {
        let point = transform.point(point);
        let x = self.layout.margins[0] * MM_TO_PT
            + (point[0] - self.layout.origin[0]) * MM_TO_PT / self.scale;
        let y = self.paper_height * MM_TO_PT
            - self.layout.margins[1] * MM_TO_PT
            - (point[1] - self.layout.origin[1]) * MM_TO_PT / self.scale;
        [x, y]
    }

    fn begin_style(&mut self, stroke: &ResolvedStroke) -> PdfResult<()> {
        let [red, green, blue] = pdf_rgb(&stroke.print_color_rgb)?;
        let dash = stroke
            .dash
            .iter()
            .map(|value| fmt(value * MM_TO_PT / self.scale))
            .collect::<Vec<_>>()
            .join(" ");
        self.commands.push(format!(
            "q {} {} {} RG {} w [{}] 0 d",
            fmt(red),
            fmt(green),
            fmt(blue),
            fmt(stroke.line_width_mm * MM_TO_PT),
            dash
        ));
        Ok(())
    }

    fn set_fill_color(&mut self, color: &str) -> PdfResult<()> {
        let [red, green, blue] = pdf_rgb(color)?;
        self.commands
            .push(format!("{} {} {} rg", fmt(red), fmt(green), fmt(blue)));
        Ok(())
    }

    fn entity(
        &mut self,
        project: &ProjectSource,
        entity: &Entity,
        depth: usize,
        transform: Transform,
    ) -> PdfResult<()> {
        if depth > MAX_BLOCK_DEPTH {
            return Ok(());
        }
        let stroke = cad_model::resolve_entity_stroke(project, entity)
            .map_err(|error| PdfError::Style(error.to_string()))?;
        self.begin_style(&stroke)?;
        match entity {
            Entity::Line { p1, p2, .. } => self.line(transform, *p1, *p2),
            Entity::Polyline { points, closed, .. } => self.polyline(transform, points, *closed),
            Entity::Circle { center, radius, .. } => self.circle(transform, *center, *radius),
            Entity::Arc {
                center,
                radius,
                start_deg,
                end_deg,
                ..
            } => self.arc(transform, *center, *radius, *start_deg, *end_deg),
            Entity::Ellipse {
                center,
                radius_x,
                radius_y,
                rotation_deg,
                start_deg,
                end_deg,
                ..
            } => self.ellipse(
                transform,
                *center,
                *radius_x,
                *radius_y,
                *rotation_deg,
                *start_deg,
                *end_deg,
            ),
            Entity::Solid { points, fill, .. } => {
                let fill = cad_model::resolve_print_fill_color(project, entity, fill)
                    .map_err(|error| PdfError::Style(error.to_string()))?;
                self.set_fill_color(&fill)?;
                self.polygon(transform, points, true);
            }
            Entity::Hatch { loops, fill, .. } => {
                let fill = fill
                    .as_deref()
                    .map(|fill| cad_model::resolve_print_fill_color(project, entity, fill))
                    .transpose()
                    .map_err(|error| PdfError::Style(error.to_string()))?
                    .unwrap_or_else(|| stroke.print_color_rgb.clone());
                self.set_fill_color(&fill)?;
                self.hatch(transform, loops);
            }
            Entity::Text {
                at,
                value,
                rotation_deg,
                mirror_y,
                style,
                ..
            } => {
                let style = project.styles.text_styles.get(style).ok_or_else(|| {
                    PdfError::Style(format!(
                        "text entity {:?} references missing style {style:?}",
                        entity.id().as_str()
                    ))
                })?;
                self.text(
                    transform,
                    *at,
                    value,
                    *rotation_deg,
                    *mirror_y,
                    style,
                    &stroke.print_color_rgb,
                    None,
                )?;
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
            } => self.dimension(
                project,
                entity,
                transform,
                style,
                *p1,
                *p2,
                *offset,
                *text_rotation_deg,
                *text_mirror_y,
                value.as_deref(),
                &stroke,
            )?,
            Entity::Point { at, .. } => {
                self.set_fill_color(&stroke.print_color_rgb)?;
                self.circle_fill(transform, *at, 0.5);
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
                let fill = cad_model::resolve_print_fill_color(project, entity, fill)
                    .map_err(|error| PdfError::Style(error.to_string()))?;
                self.set_fill_color(&fill)?;
                self.curve_solid(
                    transform,
                    *center,
                    radius.abs(),
                    radius.abs() * flatness.abs(),
                    *rotation_deg,
                    *start_deg,
                    *end_deg,
                    *solid_param,
                );
            }
            Entity::BlockRef {
                block,
                at,
                rotation_deg,
                scale,
                ..
            } => {
                if let Some(definition) = project.blocks.get(block) {
                    let nested = transform.compose(Transform::block(
                        *at,
                        *rotation_deg,
                        *scale,
                        definition.config.base_point,
                    ));
                    for record in &definition.entities {
                        if is_printable_entity(project, &record.entity) {
                            self.entity(project, &record.entity, depth + 1, nested)?;
                        }
                    }
                }
            }
        }
        self.commands.push("Q".to_owned());
        Ok(())
    }

    fn line(&mut self, transform: Transform, p1: [f64; 2], p2: [f64; 2]) {
        let p1 = self.point(p1, transform);
        let p2 = self.point(p2, transform);
        self.commands.push(format!(
            "{} {} m {} {} l S",
            fmt(p1[0]),
            fmt(p1[1]),
            fmt(p2[0]),
            fmt(p2[1])
        ));
    }

    fn polyline(&mut self, transform: Transform, points: &[[f64; 2]], closed: bool) {
        if points.len() < 2 {
            return;
        }
        let first = self.point(points[0], transform);
        let mut command = format!("{} {} m", fmt(first[0]), fmt(first[1]));
        for point in &points[1..] {
            let point = self.point(*point, transform);
            command.push_str(&format!(" {} {} l", fmt(point[0]), fmt(point[1])));
        }
        command.push_str(if closed { " h S" } else { " S" });
        self.commands.push(command);
    }

    fn polygon(&mut self, transform: Transform, points: &[[f64; 2]], fill: bool) {
        if points.len() < 3 {
            return;
        }
        let first = self.point(points[0], transform);
        let mut command = format!("{} {} m", fmt(first[0]), fmt(first[1]));
        for point in &points[1..] {
            let point = self.point(*point, transform);
            command.push_str(&format!(" {} {} l", fmt(point[0]), fmt(point[1])));
        }
        command.push_str(if fill { " h f" } else { " h S" });
        self.commands.push(command);
    }

    fn hatch(&mut self, transform: Transform, loops: &[Vec<[f64; 2]>]) {
        let mut path = String::new();
        for points in loops.iter().filter(|points| points.len() >= 3) {
            let first = self.point(points[0], transform);
            path.push_str(&format!("{} {} m", fmt(first[0]), fmt(first[1])));
            for point in &points[1..] {
                let point = self.point(*point, transform);
                path.push_str(&format!(" {} {} l", fmt(point[0]), fmt(point[1])));
            }
            path.push_str(" h ");
        }
        if !path.is_empty() {
            path.push_str("f*");
            self.commands.push(path);
            for points in loops {
                self.polyline(transform, points, true);
            }
        }
    }

    fn circle(&mut self, transform: Transform, center: [f64; 2], radius: f64) {
        self.arc(transform, center, radius, 0.0, 360.0);
    }

    fn circle_fill(&mut self, transform: Transform, center: [f64; 2], radius: f64) {
        let points = sampled_ellipse(center, radius, radius, 0.0, 0.0, 360.0);
        self.polygon(transform, &points, true);
    }

    fn arc(&mut self, transform: Transform, center: [f64; 2], radius: f64, start: f64, end: f64) {
        let steps = ((end - start).abs() / 15.0).ceil().max(2.0) as usize;
        let points = (0..=steps)
            .map(|index| {
                let angle = (start + (end - start) * index as f64 / steps as f64).to_radians();
                [
                    center[0] + radius * angle.cos(),
                    center[1] + radius * angle.sin(),
                ]
            })
            .collect::<Vec<_>>();
        self.polyline(transform, &points, false);
    }

    #[allow(clippy::too_many_arguments)]
    fn ellipse(
        &mut self,
        transform: Transform,
        center: [f64; 2],
        rx: f64,
        ry: f64,
        rotation: f64,
        start: f64,
        end: f64,
    ) {
        let rotation = rotation.to_radians();
        let steps = ((end - start).abs() / 15.0).ceil().max(2.0) as usize;
        let points = (0..=steps)
            .map(|index| {
                let angle = (start + (end - start) * index as f64 / steps as f64).to_radians();
                let local = [rx * angle.cos(), ry * angle.sin()];
                [
                    center[0] + local[0] * rotation.cos() - local[1] * rotation.sin(),
                    center[1] + local[0] * rotation.sin() + local[1] * rotation.cos(),
                ]
            })
            .collect::<Vec<_>>();
        self.polyline(transform, &points, false);
    }

    #[allow(clippy::too_many_arguments)]
    fn text(
        &mut self,
        transform: Transform,
        at: [f64; 2],
        value: &str,
        rotation: f64,
        mirror_y: bool,
        style: &TextStyleDef,
        color: &str,
        align: Option<TextAlign>,
    ) -> PdfResult<()> {
        if value.is_empty() {
            return Ok(());
        }
        self.set_fill_color(color)?;
        let align = align.as_ref().unwrap_or(&style.align);
        let length = text_length(value, style);
        let start = match align {
            TextAlign::Left => 0.0,
            TextAlign::Center => -length / 2.0,
            TextAlign::Right => -length,
        };
        let angle = rotation.to_radians();
        let x_axis = [angle.cos(), angle.sin()];
        // PDF glyph space is Y-up, while model Y is mapped through the paper's
        // top edge. Reverse the local text Y axis unless mirror_y explicitly
        // requests the reflected glyph transform.
        let y_sign = if mirror_y { 1.0 } else { -1.0 };
        let y_axis = [-angle.sin() * y_sign, angle.cos() * y_sign];
        let origin = |x: f64, y: f64| {
            self.point(
                [
                    at[0] + x_axis[0] * x + y_axis[0] * y,
                    at[1] + x_axis[1] * x + y_axis[1] * y,
                ],
                transform,
            )
        };
        let base = origin(0.0, 0.0);
        let width = origin(style.width, 0.0);
        let height = origin(0.0, style.height);
        let matrix = [
            width[0] - base[0],
            width[1] - base[1],
            height[0] - base[0],
            height[1] - base[1],
        ];
        let advance = style.width + style.spacing;
        let positions = value
            .chars()
            .enumerate()
            .map(|(index, character)| (origin(start + index as f64 * advance, 0.0), character))
            .collect::<Vec<_>>();
        self.commands.push(format!(
            "BT /{} 1 Tf",
            pdf_font_resource(&style.font_family)
        ));
        for (position, character) in positions {
            self.commands.push(format!(
                "{} {} {} {} {} {} Tm <{}> Tj",
                fmt(matrix[0]),
                fmt(matrix[1]),
                fmt(matrix[2]),
                fmt(matrix[3]),
                fmt(position[0]),
                fmt(position[1]),
                self.font.cid_hex(character)?
            ));
        }
        self.commands.push("ET".to_owned());
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn dimension(
        &mut self,
        project: &ProjectSource,
        entity: &Entity,
        transform: Transform,
        style_id: &str,
        p1: [f64; 2],
        p2: [f64; 2],
        offset: f64,
        text_rotation: f64,
        text_mirror_y: bool,
        value: Option<&str>,
        stroke: &ResolvedStroke,
    ) -> PdfResult<()> {
        let dimension_style = project
            .styles
            .dimension_styles
            .get(style_id)
            .ok_or_else(|| {
                PdfError::Style(format!(
                    "dimension entity {:?} references missing style {style_id:?}",
                    entity.id().as_str()
                ))
            })?;
        let text_style = project
            .styles
            .text_styles
            .get(&dimension_style.text_style)
            .ok_or_else(|| {
                PdfError::Style(format!(
                    "dimension entity {:?} references missing text style {:?}",
                    entity.id().as_str(),
                    dimension_style.text_style
                ))
            })?;
        let (d1, d2) = cad_model::dimension_offset_segment(p1, p2, offset).ok_or_else(|| {
            PdfError::Style(format!(
                "dimension entity {:?} has invalid geometry",
                entity.id().as_str()
            ))
        })?;
        let extension_start = |source: [f64; 2], target: [f64; 2]| {
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
        self.line(transform, extension_start(p1, d1), d1);
        self.line(transform, extension_start(p2, d2), d2);
        self.line(transform, d1, d2);
        self.dimension_arrow(
            transform,
            d1,
            d2,
            dimension_style.arrow_size,
            &stroke.print_color_rgb,
        )?;
        self.dimension_arrow(
            transform,
            d2,
            d1,
            dimension_style.arrow_size,
            &stroke.print_color_rgb,
        )?;
        let measured = ((p2[0] - p1[0]).powi(2) + (p2[1] - p1[1]).powi(2)).sqrt();
        let label = value.map(str::to_owned).unwrap_or_else(|| {
            format!(
                "{:.*} {}",
                usize::from(dimension_style.precision),
                measured,
                dimension_style.unit
            )
        });
        self.text(
            transform,
            [(d1[0] + d2[0]) / 2.0, (d1[1] + d2[1]) / 2.0],
            &label,
            text_rotation,
            text_mirror_y,
            text_style,
            &stroke.print_color_rgb,
            Some(TextAlign::Center),
        )
    }

    fn dimension_arrow(
        &mut self,
        transform: Transform,
        tip: [f64; 2],
        toward: [f64; 2],
        size: f64,
        color: &str,
    ) -> PdfResult<()> {
        let dx = toward[0] - tip[0];
        let dy = toward[1] - tip[1];
        let length = (dx * dx + dy * dy).sqrt();
        if length <= f64::EPSILON {
            return Ok(());
        }
        let ux = dx / length;
        let uy = dy / length;
        let base = [tip[0] + ux * size, tip[1] + uy * size];
        let half = size * 0.35;
        let points = [
            tip,
            [base[0] - uy * half, base[1] + ux * half],
            [base[0] + uy * half, base[1] - ux * half],
        ];
        self.set_fill_color(color)?;
        self.polygon(transform, &points, true);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn curve_solid(
        &mut self,
        transform: Transform,
        center: [f64; 2],
        radius_x: f64,
        radius_y: f64,
        rotation: f64,
        start: f64,
        end: f64,
        solid_param: f64,
    ) {
        let mut points = sampled_ellipse(center, radius_x, radius_y, rotation, start, end);
        let full = (end - start).abs() >= 360.0 - 1e-9;
        if solid_param > 0.0 && solid_param < radius_x {
            let ratio = solid_param / radius_x;
            let mut inner =
                sampled_ellipse(center, solid_param, radius_y * ratio, rotation, start, end);
            inner.reverse();
            points.extend(inner);
        } else if !full {
            points.push(center);
        }
        self.polygon(transform, &points, true);
    }
}

fn text_length(value: &str, style: &TextStyleDef) -> f64 {
    let count = value.chars().count();
    ((count as f64) * style.width + (count.saturating_sub(1) as f64) * style.spacing).max(0.0)
}

struct PdfFontSubset {
    face: ttf_parser::Face<'static>,
    remapper: subsetter::GlyphRemapper,
    unicode_by_cid: BTreeMap<u16, char>,
    width_by_cid: BTreeMap<u16, u16>,
}

struct EmbeddedPdfFont {
    bytes: Vec<u8>,
    unicode_by_cid: BTreeMap<u16, char>,
    width_by_cid: BTreeMap<u16, u16>,
    units_per_em: u16,
    ascender: i16,
    descender: i16,
    cap_height: i16,
    bbox: ttf_parser::Rect,
}

impl PdfFontSubset {
    fn new() -> PdfResult<Self> {
        let face = ttf_parser::Face::parse(M_PLUS_REGULAR, 0)
            .map_err(|error| PdfError::Style(format!("bundled M+ font is invalid: {error:?}")))?;
        Ok(Self {
            face,
            remapper: subsetter::GlyphRemapper::new(),
            unicode_by_cid: BTreeMap::new(),
            width_by_cid: BTreeMap::new(),
        })
    }

    fn cid_hex(&mut self, character: char) -> PdfResult<String> {
        let glyph = self.face.glyph_index(character).ok_or_else(|| {
            PdfError::Style(format!(
                "bundled PDF font has no glyph for U+{:04X}",
                u32::from(character)
            ))
        })?;
        let cid = self.remapper.remap(glyph.0);
        self.unicode_by_cid.entry(cid).or_insert(character);
        // CAD text width is an explicit character-cell width. Advertising a
        // 1000-unit CID advance keeps extraction and selection consistent with
        // the independently positioned cells instead of the font's proportional
        // Latin metrics.
        self.width_by_cid.insert(cid, 1000);
        Ok(format!("{cid:04X}"))
    }

    fn finish(self) -> PdfResult<EmbeddedPdfFont> {
        let bytes = subsetter::subset(M_PLUS_REGULAR, 0, &self.remapper).map_err(|error| {
            PdfError::Style(format!("failed to subset bundled M+ font: {error}"))
        })?;
        Ok(EmbeddedPdfFont {
            bytes,
            unicode_by_cid: self.unicode_by_cid,
            width_by_cid: self.width_by_cid,
            units_per_em: self.face.units_per_em(),
            ascender: self.face.ascender(),
            descender: self.face.descender(),
            cap_height: self
                .face
                .capital_height()
                .unwrap_or_else(|| self.face.ascender()),
            bbox: self.face.global_bounding_box(),
        })
    }
}

fn scale_font_metric(value: i32, units_per_em: u16) -> i32 {
    ((f64::from(value) * 1000.0) / f64::from(units_per_em)).round() as i32
}

fn sampled_ellipse(
    center: [f64; 2],
    radius_x: f64,
    radius_y: f64,
    rotation: f64,
    start: f64,
    end: f64,
) -> Vec<[f64; 2]> {
    let steps = ((end - start).abs() / 7.5).ceil().max(4.0) as usize;
    let rotation = rotation.to_radians();
    (0..=steps)
        .map(|index| {
            let angle = (start + (end - start) * index as f64 / steps as f64).to_radians();
            let local = [radius_x * angle.cos(), radius_y * angle.sin()];
            [
                center[0] + local[0] * rotation.cos() - local[1] * rotation.sin(),
                center[1] + local[0] * rotation.sin() + local[1] * rotation.cos(),
            ]
        })
        .collect()
}

fn pdf_rgb(value: &str) -> PdfResult<[f64; 3]> {
    let value = value.strip_prefix('#').unwrap_or(value);
    if value.len() != 6 || !value.is_ascii() {
        return Err(PdfError::Style(format!("invalid RGB color #{value}")));
    }
    let component = |range: std::ops::Range<usize>| {
        u8::from_str_radix(&value[range], 16)
            .map(|component| f64::from(component) / 255.0)
            .map_err(|_| PdfError::Style(format!("invalid RGB color #{value}")))
    };
    Ok([component(0..2)?, component(2..4)?, component(4..6)?])
}

fn pdf_font_resource(font_family: &str) -> &'static str {
    let family = font_family.to_ascii_lowercase();
    if family.contains("serif") || family.contains("mincho") || family.contains("明朝") {
        "F2"
    } else {
        "F1"
    }
}

fn fmt(value: f64) -> String {
    format!("{value:.4}")
}

fn build_pdf(
    width_mm: f64,
    height_mm: f64,
    commands: &[String],
    font: &EmbeddedPdfFont,
) -> PdfResult<Vec<u8>> {
    let width = width_mm * MM_TO_PT;
    let height = height_mm * MM_TO_PT;
    let stream = if commands.is_empty() {
        String::new()
    } else {
        format!("{}\nQ", commands.join("\n"))
    };
    let units = font.units_per_em;
    let bbox = font.bbox;
    let widths = font
        .width_by_cid
        .iter()
        .map(|(cid, width)| format!("{cid} [{width}]"))
        .collect::<Vec<_>>()
        .join(" ");
    let to_unicode = to_unicode_cmap(&font.unicode_by_cid);
    let objects = vec![
        ascii_object("<< /Type /Catalog /Pages 2 0 R >>"),
        ascii_object("<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        ascii_object(&format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width:.4} {height:.4}] /Resources << /Font << /F1 4 0 R /F2 4 0 R >> >> /Contents 9 0 R >>"
        )),
        ascii_object(&format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /{PDF_FONT_NAME} /Encoding /Identity-H /DescendantFonts [5 0 R] /ToUnicode 7 0 R >>"
        )),
        ascii_object(&format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{PDF_FONT_NAME} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 6 0 R /DW 1000 /W [{widths}] /CIDToGIDMap /Identity >>"
        )),
        ascii_object(&format!(
            "<< /Type /FontDescriptor /FontName /{PDF_FONT_NAME} /Flags 32 /FontBBox [{} {} {} {}] /ItalicAngle 0 /Ascent {} /Descent {} /CapHeight {} /StemV 80 /FontFile2 8 0 R >>",
            scale_font_metric(i32::from(bbox.x_min), units),
            scale_font_metric(i32::from(bbox.y_min), units),
            scale_font_metric(i32::from(bbox.x_max), units),
            scale_font_metric(i32::from(bbox.y_max), units),
            scale_font_metric(i32::from(font.ascender), units),
            scale_font_metric(i32::from(font.descender), units),
            scale_font_metric(i32::from(font.cap_height), units),
        )),
        stream_object(to_unicode.as_bytes(), None),
        stream_object(&font.bytes, Some(font.bytes.len())),
        stream_object(stream.as_bytes(), None),
    ];
    let mut pdf = b"%PDF-1.4\n%\xFF\xFF\xFF\xFF\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        pdf.extend_from_slice(object);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref = pdf.len();
    pdf.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        pdf.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{}\n%%EOF\n",
            objects.len() + 1,
            xref
        )
        .as_bytes(),
    );
    Ok(pdf)
}

fn ascii_object(value: &str) -> Vec<u8> {
    value.as_bytes().to_vec()
}

fn stream_object(bytes: &[u8], length1: Option<usize>) -> Vec<u8> {
    let mut object = match length1 {
        Some(length1) => {
            format!("<< /Length {} /Length1 {length1} >>\nstream\n", bytes.len()).into_bytes()
        }
        None => format!("<< /Length {} >>\nstream\n", bytes.len()).into_bytes(),
    };
    object.extend_from_slice(bytes);
    object.extend_from_slice(b"\nendstream");
    object
}

fn to_unicode_cmap(unicode_by_cid: &BTreeMap<u16, char>) -> String {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /CADMPlusToUnicode def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    for chunk in unicode_by_cid.iter().collect::<Vec<_>>().chunks(100) {
        cmap.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (cid, character) in chunk {
            cmap.push_str(&format!("<{cid:04X}> <{}>\n", utf16be_hex(**character)));
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    cmap
}

fn utf16be_hex(character: char) -> String {
    let mut units = [0_u16; 2];
    character
        .encode_utf16(&mut units)
        .iter()
        .map(|unit| format!("{unit:04X}"))
        .collect()
}

fn publish(output: &Path, bytes: &[u8], overwrite: bool) -> PdfResult<()> {
    match cad_edit::atomic_publish(output, bytes, overwrite) {
        Ok(()) => Ok(()),
        Err(cad_edit::EditError::Write { source, .. })
            if !overwrite && source.kind() == std::io::ErrorKind::AlreadyExists =>
        {
            Err(PdfError::OutputExists(output.to_path_buf()))
        }
        Err(error) => Err(PdfError::Write {
            path: output.to_path_buf(),
            source: std::io::Error::other(error.to_string()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emits_a_single_page_pdf_with_layout_dimensions() {
        let layout = LayoutConfig {
            name: "default".to_owned(),
            paper: "A4".to_owned(),
            orientation: SheetOrientation::Portrait,
            scale: "1/1".to_owned(),
            origin: [0.0, 0.0],
            margins: [0.0; 4],
            plot_area: None,
        };
        let content = PdfContent::new(210.0, 297.0, 1.0, &layout).expect("PDF content");
        let (commands, font) = content.finish().expect("embedded font");
        let pdf = build_pdf(210.0, 297.0, &commands, &font).expect("PDF");
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(String::from_utf8_lossy(&pdf).contains("595.2756 841.8898"));
    }

    #[test]
    fn rejects_non_ascii_rgb_without_panicking() {
        let error = pdf_rgb("aééx").expect_err("invalid RGB should fail");
        assert!(matches!(error, PdfError::Style(_)));
    }

    #[test]
    fn embedded_font_subset_is_deterministic_and_rejects_missing_glyphs() {
        let make_subset = || {
            let mut font = PdfFontSubset::new().expect("bundled font");
            assert_eq!(font.cid_hex('和').expect("Japanese glyph"), "0001");
            assert_eq!(font.cid_hex('室').expect("Japanese glyph"), "0002");
            font.finish().expect("font subset")
        };
        let first = make_subset();
        let second = make_subset();
        assert_eq!(first.bytes, second.bytes);
        assert!(first.bytes.len() < M_PLUS_REGULAR.len());

        let mut font = PdfFontSubset::new().expect("bundled font");
        let error = font
            .cid_hex('\u{1F9EA}')
            .expect_err("unsupported emoji must not become a replacement glyph");
        assert!(error.to_string().contains("U+1F9EA"));
    }

    #[test]
    fn applies_landscape_layout_margins_and_plot_clip() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let mut project = cad_model::load_project(root).expect("example");
        let layout = project.drawings[0]
            .layouts
            .layouts
            .get_mut("default")
            .expect("default layout");
        layout.paper = "A4".to_owned();
        layout.orientation = SheetOrientation::Landscape;
        layout.margins = [10.0, 20.0, 30.0, 40.0];
        layout.plot_area = Some([0.0, 0.0, 100.0, 100.0]);

        let pdf = render_drawing_pdf(&project, "plan_1f", None).expect("PDF");
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.contains("841.8898 595.2756"));
        assert!(text.contains("q 28.3465 113.3858"));
        assert!(text.contains(" re W n"));
    }

    #[test]
    fn block_rendering_applies_base_point_rotation_scale_and_text_transform() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let mut project = cad_model::load_project(root).expect("example");
        let child_line: Entity = serde_json::from_str(
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000001","type":"line","layer":"0-1","p1":[10.0,20.0],"p2":[20.0,20.0]}"#,
        )
        .expect("child line");
        let child_text: Entity = serde_json::from_str(
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000002","type":"text","layer":"0-1","style":"note","at":[10.0,20.0],"rotation_deg":0.0,"value":"B"}"#,
        )
        .expect("child text");
        project.blocks.insert(
            "fixture".to_owned(),
            cad_model::BlockDefinition {
                id: "fixture".to_owned(),
                config: cad_model::BlockDefinitionConfig {
                    schema_version: "0.2".to_owned(),
                    name: "fixture".to_owned(),
                    base_point: [10.0, 20.0],
                },
                entities: vec![
                    cad_model::EntityRecord {
                        line: 1,
                        entity: child_line,
                    },
                    cad_model::EntityRecord {
                        line: 2,
                        entity: child_text,
                    },
                ],
            },
        );
        let reference: Entity = serde_json::from_str(
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000003","type":"block_ref","layer":"0-1","block":"fixture","at":[100.0,200.0],"rotation_deg":90.0,"scale":2.0}"#,
        )
        .expect("block reference");
        let layout = LayoutConfig {
            name: "default".to_owned(),
            paper: "A4".to_owned(),
            orientation: SheetOrientation::Portrait,
            scale: "1/1".to_owned(),
            origin: [0.0, 0.0],
            margins: [0.0; 4],
            plot_area: None,
        };
        let mut content = PdfContent::new(210.0, 297.0, 1.0, &layout).expect("PDF content");

        content
            .entity(&project, &reference, 0, Transform::identity())
            .expect("block should render");

        let commands = content.commands.join("\n");
        assert!(commands.contains("283.4646 274.9606 m 283.4646 218.2677 l S"));
        assert!(commands.contains("BT /F1 1 Tf"));
        assert!(commands.contains("-708.6614"));
        assert!(commands.contains("1417.3228"));
        assert!(commands.contains("<0001> Tj"));
    }

    #[test]
    fn hatch_uses_one_even_odd_path_for_multiple_loops() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let project = cad_model::load_project(root).expect("example");
        let hatch: Entity = serde_json::from_str(
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000004","type":"hatch","layer":"0-1","loops":[[[0.0,0.0],[20.0,0.0],[20.0,20.0],[0.0,20.0]],[[5.0,5.0],[5.0,15.0],[15.0,15.0],[15.0,5.0]]],"pattern":"solid","angle_deg":0.0,"scale":1.0,"fill":"jw_black"}"#,
        )
        .expect("hatch");
        let layout = LayoutConfig {
            name: "default".to_owned(),
            paper: "A4".to_owned(),
            orientation: SheetOrientation::Portrait,
            scale: "1/1".to_owned(),
            origin: [0.0, 0.0],
            margins: [0.0; 4],
            plot_area: None,
        };
        let mut content = PdfContent::new(210.0, 297.0, 1.0, &layout).expect("PDF content");

        content
            .entity(&project, &hatch, 0, Transform::identity())
            .expect("hatch should render");

        let fill = content
            .commands
            .iter()
            .find(|command| command.ends_with("f*"))
            .expect("one even-odd fill command");
        assert_eq!(fill.matches(" m").count(), 2);
        assert_eq!(
            content
                .commands
                .iter()
                .filter(|command| command.ends_with("f*"))
                .count(),
            1
        );
    }

    #[test]
    fn applies_pen_dash_unicode_text_and_dimension_style() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let mut project = cad_model::load_project(root).expect("example");
        project.styles.colors.insert(
            "red".to_owned(),
            cad_model::ColorDef {
                rgb: "#FF0000".to_owned(),
                print_rgb: Some("#00FF00".to_owned()),
                print_width: 0.5,
            },
        );
        project.styles.line_types.insert(
            "dash".to_owned(),
            cad_model::LineTypeDef {
                dash: vec![120.0, 60.0],
            },
        );
        project.styles.pens.insert(
            "red_dash".to_owned(),
            cad_model::PenStyleDef {
                color: "red".to_owned(),
                line_type: "dash".to_owned(),
                line_width: 0.5,
            },
        );
        let text: Entity = serde_json::from_str(
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000005","type":"text","layer":"0-1","pen":"red_dash","style":"note","at":[1000.0,1000.0],"rotation_deg":0.0,"value":"和室"}"#,
        )
        .expect("text");
        let dimension: Entity = serde_json::from_str(
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000006","type":"dimension","layer":"0-1","pen":"red_dash","style":"dim_100","p1":[0.0,0.0],"p2":[910.0,0.0],"offset":120.0,"value":null}"#,
        )
        .expect("dimension");
        let layout = LayoutConfig {
            name: "default".to_owned(),
            paper: "A4".to_owned(),
            orientation: SheetOrientation::Landscape,
            scale: "1/100".to_owned(),
            origin: [0.0, 0.0],
            margins: [10.0; 4],
            plot_area: None,
        };
        let mut content = PdfContent::new(297.0, 210.0, 100.0, &layout).expect("PDF content");
        content
            .entity(&project, &text, 0, Transform::identity())
            .expect("text should render");
        content
            .entity(&project, &dimension, 0, Transform::identity())
            .expect("dimension should render");

        let commands = content.commands.join("\n");
        assert!(commands.contains("0.0000 1.0000 0.0000 RG"));
        assert!(commands.contains("0.0000 1.0000 0.0000 rg"));
        assert!(!commands.contains("1.0000 0.0000 0.0000 rg"));
        assert!(commands.contains("1.4173 w [3.4016 1.7008] 0 d"));
        assert!(commands.contains(" h f"));

        let (commands, font) = content.finish().expect("embedded font");
        assert!(font.bytes.len() < M_PLUS_REGULAR.len());
        let pdf = build_pdf(297.0, 210.0, &commands, &font).expect("PDF");
        let source = String::from_utf8_lossy(&pdf);
        assert!(source.contains("/Subtype /Type0"));
        assert!(source.contains("/Encoding /Identity-H"));
        assert!(source.contains("/FontFile2"));
        assert!(source.contains("/ToUnicode"));
        assert!(!source.contains("Heisei"));
        assert_valid_xref(&pdf);
        if let Ok(path) = std::env::var("CAD_PDF_TEST_OUTPUT") {
            fs::write(path, pdf).expect("visual PDF fixture should be written");
        }
    }

    fn assert_valid_xref(pdf: &[u8]) {
        let marker = b"startxref\n";
        let marker_index = pdf
            .windows(marker.len())
            .rposition(|window| window == marker)
            .expect("startxref marker");
        let offset_start = marker_index + marker.len();
        let offset_end = pdf[offset_start..]
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| offset_start + index)
            .expect("startxref value");
        let xref_offset = std::str::from_utf8(&pdf[offset_start..offset_end])
            .expect("xref offset should be ASCII")
            .parse::<usize>()
            .expect("xref offset should be numeric");
        assert!(pdf[xref_offset..].starts_with(b"xref\n0 "));

        let lines = pdf[xref_offset..]
            .split(|byte| *byte == b'\n')
            .collect::<Vec<_>>();
        let object_count = std::str::from_utf8(lines[1])
            .expect("xref count should be ASCII")
            .split_whitespace()
            .nth(1)
            .expect("xref count")
            .parse::<usize>()
            .expect("xref count should be numeric");
        for object_number in 1..object_count {
            let offset = std::str::from_utf8(lines[2 + object_number])
                .expect("xref entry should be ASCII")
                .split_whitespace()
                .next()
                .expect("xref entry offset")
                .parse::<usize>()
                .expect("xref entry offset should be numeric");
            assert!(
                pdf[offset..].starts_with(format!("{object_number} 0 obj").as_bytes()),
                "xref entry {object_number} points at the wrong object"
            );
        }
    }

    #[test]
    fn refuses_to_overwrite_without_explicit_permission() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let temp = tempfile::tempdir().expect("tempdir");
        let output = temp.path().join("drawing.pdf");
        fs::write(&output, b"existing").expect("existing output");
        let error = export_drawing_pdf(
            root,
            "plan_1f",
            None,
            &output,
            PdfExportOptions {
                overwrite: false,
                expected_files: Vec::new(),
            },
        )
        .expect_err("existing output must be protected");
        assert!(matches!(error, PdfError::OutputExists(_)));
        assert_eq!(fs::read(&output).expect("output bytes"), b"existing");
    }

    #[test]
    fn exports_an_explicit_layout_and_force_replaces_an_existing_pdf() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let temp = tempfile::tempdir().expect("tempdir");
        let output = temp.path().join("drawing.pdf");
        export_drawing_pdf(
            &root,
            "plan_1f",
            Some("default"),
            &output,
            PdfExportOptions::default(),
        )
        .expect("explicit layout export");
        assert!(fs::read(&output).expect("PDF").starts_with(b"%PDF-"));

        fs::write(&output, b"replace me").expect("replace fixture");
        export_drawing_pdf(
            root,
            "plan_1f",
            Some("default"),
            &output,
            PdfExportOptions {
                overwrite: true,
                expected_files: Vec::new(),
            },
        )
        .expect("forced export");
        assert!(fs::read(&output).expect("forced PDF").starts_with(b"%PDF-"));
    }

    #[test]
    fn refuses_pdf_when_expected_source_revision_is_stale() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let temp = tempfile::tempdir().expect("tempdir");
        let output = temp.path().join("drawing.pdf");
        let path = root.join("drawings/plan_1f/entities.ndjson");
        let expected = PdfFileRevision {
            relative_path: "drawings/plan_1f/entities.ndjson".to_owned(),
            revision: "stale".to_owned(),
            exists: true,
        };
        let error = export_drawing_pdf(
            root,
            "plan_1f",
            None,
            &output,
            PdfExportOptions {
                overwrite: false,
                expected_files: vec![expected],
            },
        )
        .expect_err("stale source must be rejected");
        assert!(error.to_string().contains("revision_conflict"));
        assert!(!output.exists());
        assert!(path.exists());
    }
}
