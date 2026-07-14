//! Direct PDF rendering for the canonical CAD drawing model.

use cad_model::{Entity, LayoutConfig, ProjectSource, SheetOrientation};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const CRATE_NAME: &str = "cad-render-pdf";
const MM_TO_PT: f64 = 72.0 / 25.4;
const MAX_BLOCK_DEPTH: usize = 32;

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
    let mut content = PdfContent::new(paper_width, paper_height, scale, layout);
    for record in &drawing.entities {
        if is_printable_entity(project, &record.entity) {
            content.entity(project, &record.entity, 0, Transform::identity());
        }
    }
    Ok(build_pdf(paper_width, paper_height, &content.commands))
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
    scale: f64,
    layout: &'a LayoutConfig,
    paper_height: f64,
}

impl<'a> PdfContent<'a> {
    fn new(paper_width: f64, paper_height: f64, scale: f64, layout: &'a LayoutConfig) -> Self {
        Self {
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
            scale,
            layout,
            paper_height,
        }
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

    fn entity(
        &mut self,
        project: &ProjectSource,
        entity: &Entity,
        depth: usize,
        transform: Transform,
    ) {
        if depth > MAX_BLOCK_DEPTH {
            return;
        }
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
            Entity::Solid { points, .. } => self.polygon(transform, points, true),
            Entity::Hatch { loops, .. } => self.hatch(transform, loops),
            Entity::Text {
                at,
                value,
                rotation_deg,
                ..
            } => self.text(transform, *at, value, *rotation_deg),
            Entity::Dimension { p1, p2, .. } => self.line(transform, *p1, *p2),
            Entity::Point { at, .. } => self.circle(transform, *at, 0.5),
            Entity::CurveSolid { center, radius, .. } => self.circle(transform, *center, *radius),
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
                            self.entity(project, &record.entity, depth + 1, nested);
                        }
                    }
                }
            }
        }
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

    fn text(&mut self, transform: Transform, at: [f64; 2], value: &str, rotation: f64) {
        let point = self.point(at, transform);
        let safe = value
            .chars()
            .map(|character| if character.is_ascii() { character } else { '?' })
            .collect::<String>()
            .replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)");
        let angle = rotation.to_radians();
        let cos = angle.cos();
        let sin = angle.sin();
        let a = transform.a * cos + transform.c * sin;
        let world_y = transform.b * cos + transform.d * sin;
        let b = -world_y;
        let c = world_y;
        let d = a;
        self.commands.push(format!(
            "BT /F1 10 Tf {} {} {} {} {} {} Tm ({}) Tj ET",
            fmt(a),
            fmt(b),
            fmt(c),
            fmt(d),
            fmt(point[0]),
            fmt(point[1]),
            safe
        ));
    }
}

fn fmt(value: f64) -> String {
    format!("{value:.4}")
}

fn build_pdf(width_mm: f64, height_mm: f64, commands: &[String]) -> Vec<u8> {
    let width = width_mm * MM_TO_PT;
    let height = height_mm * MM_TO_PT;
    let stream = if commands.is_empty() {
        String::new()
    } else {
        format!("{}\nQ", commands.join("\n"))
    };
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width:.4} {height:.4}] /Resources << /Font << /F1 4 0 R >> >> /Contents 5 0 R >>"
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
        format!(
            "<< /Length {} >>\nstream\n{}\nendstream",
            stream.len(),
            stream
        ),
    ];
    let mut pdf = b"%PDF-1.4\n%\xFF\xFF\xFF\xFF\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{}\nendobj\n", index + 1, object).as_bytes());
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
    pdf
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
        let content = PdfContent::new(210.0, 297.0, 1.0, &layout);
        let pdf = build_pdf(210.0, 297.0, &content.commands);
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(String::from_utf8_lossy(&pdf).contains("595.2756 841.8898"));
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
        let mut content = PdfContent::new(210.0, 297.0, 1.0, &layout);

        content.entity(&project, &reference, 0, Transform::identity());

        let commands = content.commands.join("\n");
        assert!(commands.contains("283.4646 274.9606 m 283.4646 218.2677 l S"));
        assert!(commands.contains("BT /F1 10 Tf 0.0000 -2.0000 2.0000 0.0000"));
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
        let mut content = PdfContent::new(210.0, 297.0, 1.0, &layout);

        content.entity(&project, &hatch, 0, Transform::identity());

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
