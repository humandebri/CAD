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
        self.check_layer_definitions();
        self.check_entities();
    }

    fn check_layer_definitions(&mut self) {
        for (layer_id, layer) in &self.project.layers.layers {
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
            if layer.line_width <= 0.0 {
                self.push_project(
                    "layer.invalid_line_width",
                    Some(format!("layers.{layer_id}.line_width")),
                    format!("layer {layer_id:?} has non-positive line_width"),
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
            Entity::Line { .. }
            | Entity::Polyline { .. }
            | Entity::Arc { .. }
            | Entity::Circle { .. } => {}
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
                if *radius <= EPSILON_MM {
                    self.push_entity(
                        file,
                        record,
                        "geometry.invalid_arc",
                        Some("radius"),
                        "arc radius is zero or below epsilon".to_owned(),
                    );
                }
                if (*start_deg - *end_deg).abs() <= EPSILON_MM {
                    self.push_entity(
                        file,
                        record,
                        "geometry.invalid_arc",
                        Some("end_deg"),
                        "arc start_deg and end_deg are effectively equal".to_owned(),
                    );
                }
            }
            Entity::Circle { radius, .. } => {
                if *radius <= EPSILON_MM {
                    self.push_entity(
                        file,
                        record,
                        "geometry.invalid_circle",
                        Some("radius"),
                        "circle radius is zero or below epsilon".to_owned(),
                    );
                }
            }
            Entity::Text { .. } | Entity::Dimension { .. } | Entity::BlockRef { .. } => {}
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
        self.diagnostics.push(CheckDiagnostic {
            severity: Severity::Error,
            file: "rules/layers.toml".to_owned(),
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

    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0))
        && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
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
            "[colors.jw_black]\nrgb = \"#000000\"\nprint_width = 0.25\n\n[line_types.solid]\ndash = []\n\n[text_styles.note]\nfont_family = \"Hiragino Sans\"\nheight = 250\nalign = \"left\"\n\n[dimension_styles.dim_100]\ntext_style = \"note\"\narrow_size = 120\nextension_gap = 40\nprecision = 0\nunit = \"mm\"\n",
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
