//! Strict CAD checker crate.
//!
//! This crate validates parsed CAD source and emits the structured JSON
//! diagnostics consumed by the CLI and later viewer phases.

use cad_model::{Entity, EntityRecord, ModelError, ProjectSource, entity_bbox};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

pub const CRATE_NAME: &str = "cad-check";
pub const CHECK_SCHEMA_VERSION: &str = "0.1";
const EPSILON_MM: f64 = 0.001;

#[must_use]
pub fn crate_name() -> &'static str {
    CRATE_NAME
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Ok,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckDiagnostic {
    pub severity: Severity,
    pub file: String,
    pub line: Option<usize>,
    pub entity_id: Option<String>,
    pub field: Option<String>,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckReport {
    pub schema_version: String,
    pub status: CheckStatus,
    pub diagnostics: Vec<CheckDiagnostic>,
}

impl CheckReport {
    #[must_use]
    pub fn new(diagnostics: Vec<CheckDiagnostic>) -> Self {
        let status = if diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
        {
            CheckStatus::Error
        } else {
            CheckStatus::Ok
        };
        Self {
            schema_version: CHECK_SCHEMA_VERSION.to_owned(),
            status,
            diagnostics,
        }
    }

    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.status == CheckStatus::Ok
    }
}

pub fn check_project(root: impl AsRef<Path>) -> CheckReport {
    let root = root.as_ref();
    match cad_model::load_project(root) {
        Ok(project) => check_loaded_project(&project),
        Err(error) => CheckReport::new(vec![model_error_to_diagnostic(root, &error)]),
    }
}

#[must_use]
pub fn check_loaded_project(project: &ProjectSource) -> CheckReport {
    let mut checker = Checker::new(project);
    checker.check();
    CheckReport::new(checker.diagnostics)
}

struct Checker<'a> {
    project: &'a ProjectSource,
    diagnostics: Vec<CheckDiagnostic>,
}

impl<'a> Checker<'a> {
    fn new(project: &'a ProjectSource) -> Self {
        Self {
            project,
            diagnostics: Vec::new(),
        }
    }

    fn check(&mut self) {
        if self.project.drawings.is_empty() {
            self.push_diagnostic(
                "drawings",
                "project.missing_drawing",
                None,
                "project must contain at least one drawing".to_owned(),
            );
        }
        self.check_layer_definitions();
        self.check_style_definitions();
        self.check_sheets();
        self.check_entities();
    }

    fn check_layer_definitions(&mut self) {
        if let Some(active_layer) = &self.project.layers.active_layer
            && !self.project.layers.layers.contains_key(active_layer)
        {
            self.push_project(
                "reference.undefined_active_layer",
                Some("active_layer".to_owned()),
                format!("active_layer references undefined layer {active_layer:?}"),
            );
        }
        for (layer_id, layer) in &self.project.layers.layers {
            if let Some(group) = &layer.group
                && !self.project.layers.groups.contains_key(group)
            {
                self.push_project(
                    "reference.undefined_layer_group",
                    Some(format!("layers.{layer_id}.group")),
                    format!("layer {layer_id:?} references undefined group {group:?}"),
                );
            }
            if !self.project.styles.colors.contains_key(&layer.color) {
                self.push_project(
                    "reference.undefined_color",
                    Some(format!("layers.{layer_id}.color")),
                    format!(
                        "layer {layer_id:?} references undefined color {:?}",
                        layer.color
                    ),
                );
            }
            if !self
                .project
                .styles
                .line_types
                .contains_key(&layer.line_type)
            {
                self.push_project(
                    "reference.undefined_line_type",
                    Some(format!("layers.{layer_id}.line_type")),
                    format!(
                        "layer {layer_id:?} references undefined line_type {:?}",
                        layer.line_type
                    ),
                );
            }
            if !layer.line_width.is_finite() || layer.line_width <= 0.0 {
                self.push_project(
                    "layer.invalid_line_width",
                    Some(format!("layers.{layer_id}.line_width")),
                    format!("layer {layer_id:?} has non-positive line_width"),
                );
            }
        }
        for (group_id, group) in &self.project.layers.groups {
            if !group.scale_denominator.is_finite() || group.scale_denominator <= 0.0 {
                self.push_project(
                    "layer_group.invalid_scale_denominator",
                    Some(format!("groups.{group_id}.scale_denominator")),
                    format!("layer group {group_id:?} has a non-finite or non-positive scale"),
                );
            }
        }
        for (pen_id, pen) in &self.project.styles.pens {
            if !self.project.styles.colors.contains_key(&pen.color) {
                self.push_project(
                    "reference.undefined_pen_color",
                    Some(format!("pens.{pen_id}.color")),
                    format!("pen {pen_id:?} references undefined color {:?}", pen.color),
                );
            }
            if !self.project.styles.line_types.contains_key(&pen.line_type) {
                self.push_project(
                    "reference.undefined_pen_line_type",
                    Some(format!("pens.{pen_id}.line_type")),
                    format!(
                        "pen {pen_id:?} references undefined line type {:?}",
                        pen.line_type
                    ),
                );
            }
            if !pen.line_width.is_finite() || pen.line_width <= 0.0 {
                self.push_project(
                    "pen.invalid_line_width",
                    Some(format!("pens.{pen_id}.line_width")),
                    format!("pen {pen_id:?} has non-positive line_width"),
                );
            }
        }
    }

    fn check_style_definitions(&mut self) {
        for (color_id, color) in &self.project.styles.colors {
            if !is_rgb_hex(&color.rgb) {
                self.push_style(
                    "style.invalid_rgb",
                    Some(format!("colors.{color_id}.rgb")),
                    format!("color {color_id:?} must use #RRGGBB"),
                );
            }
            if !color.print_width.is_finite() || color.print_width <= 0.0 {
                self.push_style(
                    "style.invalid_print_width",
                    Some(format!("colors.{color_id}.print_width")),
                    format!("color {color_id:?} has a non-finite or non-positive print width"),
                );
            }
        }
        for (line_type_id, line_type) in &self.project.styles.line_types {
            if line_type
                .dash
                .iter()
                .any(|value| !value.is_finite() || *value <= 0.0)
            {
                self.push_style(
                    "style.invalid_dash",
                    Some(format!("line_types.{line_type_id}.dash")),
                    format!(
                        "line type {line_type_id:?} contains a non-finite or non-positive dash"
                    ),
                );
            }
        }
        for (style_id, style) in &self.project.styles.text_styles {
            if style.font_family.trim().is_empty()
                || !style.height.is_finite()
                || style.height <= 0.0
                || !style.width.is_finite()
                || style.width <= 0.0
                || !style.spacing.is_finite()
                || style.spacing < 0.0
            {
                self.push_style(
                    "style.invalid_text_style",
                    Some(format!("text_styles.{style_id}")),
                    format!("text style {style_id:?} has invalid font or metrics"),
                );
            }
        }
        for (style_id, style) in &self.project.styles.dimension_styles {
            if !self
                .project
                .styles
                .text_styles
                .contains_key(&style.text_style)
            {
                self.push_style(
                    "reference.undefined_dimension_text_style",
                    Some(format!("dimension_styles.{style_id}.text_style")),
                    format!(
                        "dimension style {style_id:?} references undefined text style {:?}",
                        style.text_style
                    ),
                );
            }
            if !style.arrow_size.is_finite()
                || style.arrow_size <= 0.0
                || !style.extension_gap.is_finite()
                || style.extension_gap < 0.0
                || style.unit.trim().is_empty()
            {
                self.push_style(
                    "style.invalid_dimension_style",
                    Some(format!("dimension_styles.{style_id}")),
                    format!("dimension style {style_id:?} has invalid metrics or unit"),
                );
            }
        }
    }

    fn check_sheets(&mut self) {
        for drawing in &self.project.drawings {
            let file = format!("drawings/{}/sheet.toml", drawing.name);
            if !matches!(
                drawing.sheet.paper.as_str(),
                "A0" | "A1" | "A2" | "A3" | "A4"
            ) {
                self.push_diagnostic(
                    &file,
                    "sheet.invalid_paper",
                    Some("paper".to_owned()),
                    format!(
                        "unsupported paper {:?}; expected A0 through A4",
                        drawing.sheet.paper
                    ),
                );
            }
            if parse_scale(&drawing.sheet.scale).is_none() {
                self.push_diagnostic(
                    &file,
                    "sheet.invalid_scale",
                    Some("scale".to_owned()),
                    "scale must be a finite positive ratio such as 1:100".to_owned(),
                );
            }
            if drawing.sheet.origin.iter().any(|value| !value.is_finite()) {
                self.push_diagnostic(
                    &file,
                    "sheet.invalid_origin",
                    Some("origin".to_owned()),
                    "origin coordinates must be finite".to_owned(),
                );
            }
        }
    }

    fn check_entities(&mut self) {
        let blocks = self.defined_blocks();
        let mut ids = BTreeMap::<String, String>::new();

        for drawing in &self.project.drawings {
            for record in &drawing.entities {
                let file = self.entity_file(&drawing.name);
                let id = record.entity.id().as_str().to_owned();
                if let Some(first_file) = ids.insert(id.clone(), file.clone()) {
                    self.push_entity(
                        &file,
                        record,
                        "reference.duplicate_id",
                        Some("id"),
                        format!("entity id {id:?} already appears in {first_file}"),
                    );
                }

                self.check_entity_references(&file, record, &blocks);
                self.check_entity_geometry(&file, record);
                self.check_annotation_layer(&file, record);
            }
        }
    }

    fn check_entity_references(
        &mut self,
        file: &str,
        record: &EntityRecord,
        blocks: &BTreeSet<String>,
    ) {
        let layer = record.entity.layer();
        if !self.project.layers.layers.contains_key(layer) {
            self.push_entity(
                file,
                record,
                "reference.undefined_layer",
                Some("layer"),
                format!("entity references undefined layer {layer:?}"),
            );
        }
        if let Some(pen) = record.entity.pen()
            && !self.project.styles.pens.contains_key(pen)
        {
            self.push_entity(
                file,
                record,
                "reference.undefined_pen",
                Some("pen"),
                format!("entity references undefined pen {pen:?}"),
            );
        }

        match &record.entity {
            Entity::Text { style, .. } => {
                if !self.project.styles.text_styles.contains_key(style) {
                    self.push_entity(
                        file,
                        record,
                        "reference.undefined_text_style",
                        Some("style"),
                        format!("text entity references undefined text style {style:?}"),
                    );
                }
            }
            Entity::Dimension { style, .. } => {
                if !self.project.styles.dimension_styles.contains_key(style) {
                    self.push_entity(
                        file,
                        record,
                        "reference.undefined_dimension_style",
                        Some("style"),
                        format!("dimension entity references undefined dimension style {style:?}"),
                    );
                }
            }
            Entity::BlockRef { block, .. } => {
                if !blocks.contains(block) {
                    self.push_entity(
                        file,
                        record,
                        "reference.undefined_block",
                        Some("block"),
                        format!("block_ref entity references undefined block {block:?}"),
                    );
                }
            }
            Entity::Solid { fill, .. } | Entity::CurveSolid { fill, .. } => {
                if !self.project.styles.colors.contains_key(fill) {
                    self.push_entity(
                        file,
                        record,
                        "reference.undefined_fill",
                        Some("fill"),
                        format!("solid entity references undefined fill color {fill:?}"),
                    );
                }
            }
            Entity::Line { .. }
            | Entity::Polyline { .. }
            | Entity::Arc { .. }
            | Entity::Circle { .. }
            | Entity::Ellipse { .. }
            | Entity::Point { .. } => {}
        }
    }

    fn check_entity_geometry(&mut self, file: &str, record: &EntityRecord) {
        if entity_bbox(&record.entity).is_none() {
            self.push_entity(
                file,
                record,
                "geometry.invalid_bbox",
                None,
                "entity does not produce a finite bounding box".to_owned(),
            );
        }

        match &record.entity {
            Entity::Line { p1, p2, .. } => {
                if distance(*p1, *p2) <= EPSILON_MM {
                    self.push_entity(
                        file,
                        record,
                        "geometry.zero_length",
                        Some("p2"),
                        "line length is zero or below epsilon".to_owned(),
                    );
                }
            }
            Entity::Polyline { points, closed, .. } => {
                self.check_polyline(file, record, points, *closed);
            }
            Entity::Arc {
                radius,
                start_deg,
                end_deg,
                ..
            } => {
                if !radius.is_finite() || *radius <= EPSILON_MM {
                    self.push_entity(
                        file,
                        record,
                        "geometry.invalid_arc",
                        Some("radius"),
                        "arc radius is zero or below epsilon".to_owned(),
                    );
                }
                let span = (*end_deg - *start_deg).abs();
                if !start_deg.is_finite()
                    || !end_deg.is_finite()
                    || span <= EPSILON_MM
                    || span > 360.0 + EPSILON_MM
                {
                    self.push_entity(
                        file,
                        record,
                        "geometry.invalid_arc",
                        Some("end_deg"),
                        "arc span must be finite, greater than zero, and at most 360 degrees"
                            .to_owned(),
                    );
                }
            }
            Entity::Circle { radius, .. } => {
                if !radius.is_finite() || *radius <= EPSILON_MM {
                    self.push_entity(
                        file,
                        record,
                        "geometry.invalid_circle",
                        Some("radius"),
                        "circle radius is zero or below epsilon".to_owned(),
                    );
                }
            }
            Entity::Ellipse {
                radius_x,
                radius_y,
                start_deg,
                end_deg,
                ..
            } => {
                if *radius_x <= EPSILON_MM || *radius_y <= EPSILON_MM {
                    self.push_entity(
                        file,
                        record,
                        "geometry.invalid_ellipse",
                        Some(if *radius_x <= EPSILON_MM {
                            "radius_x"
                        } else {
                            "radius_y"
                        }),
                        "ellipse radius is zero or below epsilon".to_owned(),
                    );
                }
                let span = (*end_deg - *start_deg).abs();
                if span <= EPSILON_MM || span > 360.0 + EPSILON_MM {
                    self.push_entity(
                        file,
                        record,
                        "geometry.invalid_ellipse",
                        Some("end_deg"),
                        "ellipse arc span must be greater than zero and at most 360 degrees"
                            .to_owned(),
                    );
                }
            }
            Entity::Point { scale, .. } => {
                if !scale.is_finite() || *scale <= 0.0 {
                    self.push_entity(
                        file,
                        record,
                        "geometry.invalid_point",
                        Some("scale"),
                        "point scale must be finite and positive".to_owned(),
                    );
                }
            }
            Entity::Solid { points, .. } => {
                if !(3..=4).contains(&points.len()) {
                    self.push_entity(
                        file,
                        record,
                        "geometry.invalid_solid",
                        Some("points"),
                        "solid must contain three or four points".to_owned(),
                    );
                }
                if polygon_is_degenerate(points) {
                    self.push_entity(
                        file,
                        record,
                        "geometry.degenerate_solid",
                        Some("points"),
                        "solid points must be distinct and enclose a non-zero area".to_owned(),
                    );
                }
            }
            Entity::CurveSolid {
                radius,
                flatness,
                start_deg,
                end_deg,
                solid_param,
                ..
            } => {
                if !radius.is_finite()
                    || !flatness.is_finite()
                    || *radius <= EPSILON_MM
                    || flatness.abs() <= EPSILON_MM
                {
                    self.push_entity(
                        file,
                        record,
                        "geometry.invalid_curve_solid",
                        Some("radius"),
                        "curve solid radius and flatness must be finite and non-zero".to_owned(),
                    );
                }
                let span = (*end_deg - *start_deg).abs();
                if !span.is_finite() || span <= EPSILON_MM || span > 360.0 + EPSILON_MM {
                    self.push_entity(
                        file,
                        record,
                        "geometry.invalid_curve_solid",
                        Some("end_deg"),
                        "curve solid span must be greater than zero and at most 360 degrees"
                            .to_owned(),
                    );
                }
                if !solid_param.is_finite() || *solid_param < 0.0 || *solid_param >= radius.abs() {
                    self.push_entity(
                        file,
                        record,
                        "geometry.invalid_curve_solid",
                        Some("solid_param"),
                        "curve solid inner parameter must be non-negative and below radius"
                            .to_owned(),
                    );
                }
            }
            Entity::Text { rotation_deg, .. } => {
                self.check_finite_entity_value(file, record, "rotation_deg", *rotation_deg);
            }
            Entity::Dimension {
                p1,
                p2,
                offset,
                text_rotation_deg,
                ..
            } => {
                if distance(*p1, *p2) <= EPSILON_MM {
                    self.push_entity(
                        file,
                        record,
                        "geometry.zero_dimension",
                        Some("p2"),
                        "dimension points must not coincide".to_owned(),
                    );
                }
                self.check_finite_entity_value(file, record, "offset", *offset);
                self.check_finite_entity_value(
                    file,
                    record,
                    "text_rotation_deg",
                    *text_rotation_deg,
                );
            }
            Entity::BlockRef {
                rotation_deg,
                scale,
                ..
            } => {
                self.check_finite_entity_value(file, record, "rotation_deg", *rotation_deg);
                if !scale.is_finite() || *scale <= 0.0 {
                    self.push_entity(
                        file,
                        record,
                        "geometry.invalid_block_scale",
                        Some("scale"),
                        "block scale must be finite and positive".to_owned(),
                    );
                }
            }
        }
    }

    fn check_finite_entity_value(
        &mut self,
        file: &str,
        record: &EntityRecord,
        field: &str,
        value: f64,
    ) {
        if !value.is_finite() {
            self.push_entity(
                file,
                record,
                "geometry.non_finite",
                Some(field),
                format!("{field} must be finite"),
            );
        }
    }

    fn check_polyline(
        &mut self,
        file: &str,
        record: &EntityRecord,
        points: &[[f64; 2]],
        closed: bool,
    ) {
        if points.len() < 2 {
            self.push_entity(
                file,
                record,
                "geometry.zero_length",
                Some("points"),
                "polyline must contain at least two points".to_owned(),
            );
            return;
        }

        for window in points.windows(2) {
            let segment_len = distance(window[0], window[1]);
            if segment_len <= EPSILON_MM {
                self.push_entity(
                    file,
                    record,
                    "geometry.zero_length",
                    Some("points"),
                    "polyline contains a zero-length segment".to_owned(),
                );
            } else if segment_len < 1.0 {
                self.push_entity(
                    file,
                    record,
                    "geometry.tiny_gap",
                    Some("points"),
                    "polyline contains a segment shorter than 1mm".to_owned(),
                );
            }
        }

        if closed && distance(points[0], points[points.len() - 1]) > EPSILON_MM {
            self.push_entity(
                file,
                record,
                "geometry.open_closed_polyline",
                Some("closed"),
                "closed polyline does not return to its first point".to_owned(),
            );
        }

        if has_self_intersection(points, closed) {
            self.push_entity(
                file,
                record,
                "geometry.self_intersection",
                Some("points"),
                "polyline has intersecting non-adjacent segments".to_owned(),
            );
        }
    }

    fn check_annotation_layer(&mut self, file: &str, record: &EntityRecord) {
        let is_annotation = matches!(
            record.entity,
            Entity::Text { .. } | Entity::Dimension { .. }
        );
        if !is_annotation {
            return;
        }

        let Some(layer) = self.project.layers.layers.get(record.entity.layer()) else {
            return;
        };
        if !layer.printable {
            self.push_entity(
                file,
                record,
                "layer.non_print_annotation",
                Some("layer"),
                "text or dimension entity is placed on a non-printable layer".to_owned(),
            );
        }
    }

    fn defined_blocks(&self) -> BTreeSet<String> {
        let blocks_dir = self.project.root.join("blocks");
        let Ok(entries) = fs::read_dir(blocks_dir) else {
            return BTreeSet::new();
        };

        entries
            .filter_map(Result::ok)
            .filter_map(|entry| match entry.file_type() {
                Ok(file_type) if file_type.is_dir() => {
                    Some(entry.file_name().to_string_lossy().into_owned())
                }
                _ => None,
            })
            .collect()
    }

    fn entity_file(&self, drawing_name: &str) -> String {
        format!("drawings/{drawing_name}/entities.ndjson")
    }

    fn push_project(&mut self, code: &str, field: Option<String>, message: String) {
        self.push_diagnostic("rules/layers.toml", code, field, message);
    }

    fn push_style(&mut self, code: &str, field: Option<String>, message: String) {
        self.push_diagnostic("rules/styles.toml", code, field, message);
    }

    fn push_diagnostic(&mut self, file: &str, code: &str, field: Option<String>, message: String) {
        self.diagnostics.push(CheckDiagnostic {
            severity: Severity::Error,
            file: file.to_owned(),
            line: None,
            entity_id: None,
            field,
            code: code.to_owned(),
            message,
        });
    }

    fn push_entity(
        &mut self,
        file: &str,
        record: &EntityRecord,
        code: &str,
        field: Option<&str>,
        message: String,
    ) {
        self.diagnostics.push(CheckDiagnostic {
            severity: Severity::Error,
            file: file.to_owned(),
            line: Some(record.line),
            entity_id: Some(record.entity.id().as_str().to_owned()),
            field: field.map(str::to_owned),
            code: code.to_owned(),
            message,
        });
    }
}

fn model_error_to_diagnostic(root: &Path, error: &ModelError) -> CheckDiagnostic {
    match error {
        ModelError::Read { path, .. }
        | ModelError::ListDir { path, .. }
        | ModelError::Toml { path, .. } => CheckDiagnostic {
            severity: Severity::Error,
            file: relative_path(root, path),
            line: None,
            entity_id: None,
            field: None,
            code: model_error_code(error).to_owned(),
            message: error.to_string(),
        },
        ModelError::Ndjson { path, line, .. } => CheckDiagnostic {
            severity: Severity::Error,
            file: relative_path(root, path),
            line: Some(*line),
            entity_id: None,
            field: None,
            code: "format.invalid_ndjson".to_owned(),
            message: error.to_string(),
        },
        ModelError::EmptyNdjsonLine { path, line } => CheckDiagnostic {
            severity: Severity::Error,
            file: relative_path(root, path),
            line: Some(*line),
            entity_id: None,
            field: None,
            code: "format.empty_ndjson_line".to_owned(),
            message: error.to_string(),
        },
        ModelError::UnsupportedSchema {
            path,
            line,
            found: _,
            expected: _,
        } => CheckDiagnostic {
            severity: Severity::Error,
            file: relative_path(root, path),
            line: *line,
            entity_id: None,
            field: Some("schema_version".to_owned()),
            code: "format.unsupported_schema".to_owned(),
            message: error.to_string(),
        },
        ModelError::InvalidEntityId { .. } => CheckDiagnostic {
            severity: Severity::Error,
            file: ".".to_owned(),
            line: None,
            entity_id: None,
            field: Some("id".to_owned()),
            code: "format.invalid_entity_id".to_owned(),
            message: error.to_string(),
        },
    }
}

fn model_error_code(error: &ModelError) -> &'static str {
    match error {
        ModelError::Read { .. } | ModelError::ListDir { .. } => "format.missing_file",
        ModelError::Toml { .. } => "format.invalid_toml",
        ModelError::Ndjson { .. } => "format.invalid_ndjson",
        ModelError::EmptyNdjsonLine { .. } => "format.empty_ndjson_line",
        ModelError::UnsupportedSchema { .. } => "format.unsupported_schema",
        ModelError::InvalidEntityId { .. } => "format.invalid_entity_id",
    }
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn distance(a: [f64; 2], b: [f64; 2]) -> f64 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    (dx * dx + dy * dy).sqrt()
}

fn is_rgb_hex(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value.as_bytes()[1..].iter().all(u8::is_ascii_hexdigit)
}

fn parse_scale(value: &str) -> Option<f64> {
    let (numerator, denominator) = value.split_once(':').or_else(|| value.split_once('/'))?;
    let numerator = numerator.trim().parse::<f64>().ok()?;
    let denominator = denominator.trim().parse::<f64>().ok()?;
    (numerator.is_finite() && denominator.is_finite() && numerator > 0.0 && denominator > 0.0)
        .then_some(numerator / denominator)
}

fn polygon_is_degenerate(points: &[[f64; 2]]) -> bool {
    if points.len() < 3
        || points
            .iter()
            .any(|point| point.iter().any(|value| !value.is_finite()))
    {
        return true;
    }
    let mut unique = BTreeSet::new();
    for point in points {
        unique.insert((point[0].to_bits(), point[1].to_bits()));
    }
    if unique.len() < 3 {
        return true;
    }
    let area_twice: f64 = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum();
    area_twice.abs() <= EPSILON_MM * EPSILON_MM
}

fn has_self_intersection(points: &[[f64; 2]], closed: bool) -> bool {
    let mut segments: Vec<([f64; 2], [f64; 2])> = points
        .windows(2)
        .map(|window| (window[0], window[1]))
        .collect();
    if closed && points.len() > 2 {
        segments.push((points[points.len() - 1], points[0]));
    }

    for left_index in 0..segments.len() {
        for right_index in (left_index + 1)..segments.len() {
            if segments_are_adjacent(left_index, right_index, segments.len(), closed) {
                continue;
            }
            if segments_intersect(segments[left_index], segments[right_index]) {
                return true;
            }
        }
    }
    false
}

fn segments_are_adjacent(left: usize, right: usize, len: usize, closed: bool) -> bool {
    right == left + 1 || (closed && left == 0 && right + 1 == len)
}

fn segments_intersect(a: ([f64; 2], [f64; 2]), b: ([f64; 2], [f64; 2])) -> bool {
    let d1 = direction(a.0, a.1, b.0);
    let d2 = direction(a.0, a.1, b.1);
    let d3 = direction(b.0, b.1, a.0);
    let d4 = direction(b.0, b.1, a.1);

    let crosses = ((d1 > EPSILON_MM && d2 < -EPSILON_MM) || (d1 < -EPSILON_MM && d2 > EPSILON_MM))
        && ((d3 > EPSILON_MM && d4 < -EPSILON_MM) || (d3 < -EPSILON_MM && d4 > EPSILON_MM));
    crosses
        || (d1.abs() <= EPSILON_MM && point_on_segment(b.0, a))
        || (d2.abs() <= EPSILON_MM && point_on_segment(b.1, a))
        || (d3.abs() <= EPSILON_MM && point_on_segment(a.0, b))
        || (d4.abs() <= EPSILON_MM && point_on_segment(a.1, b))
}

fn point_on_segment(point: [f64; 2], segment: ([f64; 2], [f64; 2])) -> bool {
    point[0] >= segment.0[0].min(segment.1[0]) - EPSILON_MM
        && point[0] <= segment.0[0].max(segment.1[0]) + EPSILON_MM
        && point[1] >= segment.0[1].min(segment.1[1]) - EPSILON_MM
        && point[1] <= segment.0[1].max(segment.1[1]) + EPSILON_MM
}

fn direction(a: [f64; 2], b: [f64; 2], c: [f64; 2]) -> f64 {
    ((c[0] - a[0]) * (b[1] - a[1])) - ((b[0] - a[0]) * (c[1] - a[1]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{create_dir_all, write};
    use std::path::PathBuf;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(crate_name(), "cad-check");
    }

    #[test]
    fn accepts_house_small_example() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");

        let report = check_project(root);

        assert_eq!(report.status, CheckStatus::Ok);
        assert!(report.diagnostics.is_empty());
    }

    #[test]
    fn reports_invalid_ndjson_as_whole_check_failure() {
        let temp = fixture_project();
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            "{\"schema_version\":\"0.1\"",
        )
        .expect("entities should be writable");

        let report = check_project(temp.path());

        assert_code(&report, "format.invalid_ndjson");
        assert_eq!(report.diagnostics[0].line, Some(1));
    }

    #[test]
    fn reports_unknown_field_as_whole_check_failure() {
        let temp = fixture_project();
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0.0,0.0],"p2":[910.0,0.0],"extra":true}"#,
        )
        .expect("entities should be writable");

        let report = check_project(temp.path());

        assert_code(&report, "format.invalid_ndjson");
        assert_eq!(report.diagnostics[0].line, Some(1));
    }

    #[test]
    fn reports_missing_required_field_as_whole_check_failure() {
        let temp = fixture_project();
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000001","type":"line","layer":"0-1","p1":[0.0,0.0]}"#,
        )
        .expect("entities should be writable");

        let report = check_project(temp.path());

        assert_code(&report, "format.invalid_ndjson");
        assert_eq!(report.diagnostics[0].line, Some(1));
    }

    #[test]
    fn reports_reference_errors() {
        let temp = fixture_project();
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            [
                line_entity("ent_01JZ0000000000000000000000", "missing"),
                text_entity("ent_01JZ0000000000000000000001", "missing_style", "0-1"),
                dimension_entity("ent_01JZ0000000000000000000002", "missing_dim", "0-1"),
                block_entity("ent_01JZ0000000000000000000003", "door_910", "0-1"),
            ]
            .join("\n"),
        )
        .expect("entities should be writable");

        let report = check_project(temp.path());

        assert_code(&report, "reference.undefined_layer");
        assert_code(&report, "reference.undefined_text_style");
        assert_code(&report, "reference.undefined_dimension_style");
        assert_code(&report, "reference.undefined_block");
    }

    #[test]
    fn reports_duplicate_ids() {
        let temp = fixture_project();
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            [
                line_entity("ent_01JZ0000000000000000000000", "0-1"),
                line_entity("ent_01JZ0000000000000000000000", "0-1"),
            ]
            .join("\n"),
        )
        .expect("entities should be writable");

        let report = check_project(temp.path());

        assert_code(&report, "reference.duplicate_id");
    }

    #[test]
    fn reports_layer_definition_errors() {
        let temp = fixture_project();
        write(
            temp.path().join("rules/layers.toml"),
            "[layers.\"0-1\"]\nname = \"A-WALL\"\nvisible = true\nprintable = true\ncolor = \"missing\"\nline_type = \"missing\"\nline_width = 0.0\n",
        )
        .expect("layers should be writable");

        let report = check_project(temp.path());

        assert_code(&report, "reference.undefined_color");
        assert_code(&report, "reference.undefined_line_type");
        assert_code(&report, "layer.invalid_line_width");
    }

    #[test]
    fn reports_geometry_errors() {
        let temp = fixture_project();
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            [
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0.0,0.0],"p2":[0.0,0.0]}"#,
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000001","type":"arc","layer":"0-1","center":[0.0,0.0],"radius":0.0,"start_deg":0.0,"end_deg":0.0}"#,
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000002","type":"polyline","layer":"0-1","points":[[0.0,0.0],[0.5,0.0],[1.0,0.0]],"closed":false}"#,
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000003","type":"polyline","layer":"0-1","points":[[0.0,0.0],[1.0,1.0],[0.0,1.0],[1.0,0.0]],"closed":false}"#,
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000004","type":"polyline","layer":"0-1","points":[[0.0,0.0],[1.0,0.0],[1.0,1.0]],"closed":true}"#,
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000005","type":"polyline","layer":"0-1","points":[],"closed":false}"#,
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000006","type":"ellipse","layer":"0-1","center":[0.0,0.0],"radius_x":0.0,"radius_y":1.0,"rotation_deg":0.0,"start_deg":0.0,"end_deg":361.0}"#,
            ]
            .join("\n"),
        )
        .expect("entities should be writable");

        let report = check_project(temp.path());

        assert_code(&report, "geometry.zero_length");
        assert_code(&report, "geometry.invalid_arc");
        assert_code(&report, "geometry.tiny_gap");
        assert_code(&report, "geometry.self_intersection");
        assert_code(&report, "geometry.open_closed_polyline");
        assert_code(&report, "geometry.invalid_bbox");
        assert_code(&report, "geometry.invalid_ellipse");
    }

    #[test]
    fn reports_annotation_on_non_printable_layer() {
        let temp = fixture_project();
        write(
            temp.path().join("rules/layers.toml"),
            "[layers.\"0-1\"]\nname = \"A-WALL\"\nvisible = true\nprintable = false\ncolor = \"jw_black\"\nline_type = \"solid\"\nline_width = 0.25\n",
        )
        .expect("layers should be writable");
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            text_entity("ent_01JZ0000000000000000000001", "note", "0-1"),
        )
        .expect("entities should be writable");

        let report = check_project(temp.path());

        assert_code(&report, "layer.non_print_annotation");
    }

    #[test]
    fn json_report_shape_is_stable() {
        let report = CheckReport::new(vec![CheckDiagnostic {
            severity: Severity::Error,
            file: "drawings/plan_1f/entities.ndjson".to_owned(),
            line: Some(1),
            entity_id: Some("ent_01JZ0000000000000000000000".to_owned()),
            field: Some("layer".to_owned()),
            code: "reference.undefined_layer".to_owned(),
            message: "entity references undefined layer".to_owned(),
        }]);

        insta::assert_snapshot!(
            serde_json::to_string_pretty(&report).expect("report should serialize"),
            @r#"
{
  "schema_version": "0.1",
  "status": "error",
  "diagnostics": [
    {
      "severity": "error",
      "file": "drawings/plan_1f/entities.ndjson",
      "line": 1,
      "entity_id": "ent_01JZ0000000000000000000000",
      "field": "layer",
      "code": "reference.undefined_layer",
      "message": "entity references undefined layer"
    }
  ]
}
"#
        );
    }

    fn assert_code(report: &CheckReport, code: &str) {
        assert!(
            report
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == code),
            "missing diagnostic code {code}; diagnostics: {:#?}",
            report.diagnostics
        );
    }

    fn fixture_project() -> tempfile::TempDir {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        create_dir_all(temp.path().join("rules")).expect("rules dir should be created");
        create_dir_all(temp.path().join("drawings/plan_1f"))
            .expect("drawing dir should be created");

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
            line_entity("ent_01JZ0000000000000000000000", "0-1"),
        )
        .expect("entities NDJSON should be writable");

        temp
    }

    fn line_entity(id: &str, layer: &str) -> String {
        format!(
            r#"{{"schema_version":"0.1","id":"{id}","type":"line","layer":"{layer}","p1":[0.0,0.0],"p2":[910.0,0.0]}}"#
        )
    }

    fn text_entity(id: &str, style: &str, layer: &str) -> String {
        format!(
            r#"{{"schema_version":"0.1","id":"{id}","type":"text","layer":"{layer}","style":"{style}","at":[0.0,0.0],"rotation_deg":0.0,"value":"note"}}"#
        )
    }

    fn dimension_entity(id: &str, style: &str, layer: &str) -> String {
        format!(
            r#"{{"schema_version":"0.1","id":"{id}","type":"dimension","layer":"{layer}","style":"{style}","p1":[0.0,0.0],"p2":[1.0,0.0],"offset":100.0,"value":null}}"#
        )
    }

    fn block_entity(id: &str, block: &str, layer: &str) -> String {
        format!(
            r#"{{"schema_version":"0.1","id":"{id}","type":"block_ref","layer":"{layer}","block":"{block}","at":[0.0,0.0],"rotation_deg":0.0,"scale":1.0}}"#
        )
    }
}
