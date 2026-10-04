//! Annotation text edits; dimension labels are evaluated separately.
use crate::{
    Result, ToolkitError,
    drafting::{GeneratedEdit, TextReplaceRequest, replace_text},
};
use cad_edit::EditOperation;
use cad_model::{Entity, ProjectSource};
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeSet;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextStyleRequest {
    pub style: String,
    #[serde(default)]
    pub entity_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TextEditRequest {
    Replace {
        find: String,
        replace: String,
        entity_ids: Vec<String>,
        style: Option<String>,
    },
    SetStyle {
        style: String,
        entity_ids: Vec<String>,
    },
    SetWritingMode {
        writing_mode: cad_model::TextWritingMode,
        entity_ids: Vec<String>,
    },
}

pub fn generate(
    project: &ProjectSource,
    drawing: &str,
    request: &TextEditRequest,
) -> Result<GeneratedEdit> {
    match request {
        TextEditRequest::Replace {
            find,
            replace,
            entity_ids,
            style,
        } => replace_text(
            project,
            drawing,
            &TextReplaceRequest {
                find: find.clone(),
                replace: replace.clone(),
                entity_ids: entity_ids.clone(),
                style: style.clone(),
            },
        ),
        TextEditRequest::SetStyle { style, entity_ids } => set_style(
            project,
            drawing,
            &TextStyleRequest {
                style: style.clone(),
                entity_ids: entity_ids.clone(),
            },
        ),
        TextEditRequest::SetWritingMode {
            writing_mode,
            entity_ids,
        } => set_writing_mode(project, drawing, *writing_mode, entity_ids),
    }
}

pub fn set_writing_mode(
    project: &ProjectSource,
    drawing: &str,
    writing_mode: cad_model::TextWritingMode,
    entity_ids: &[String],
) -> Result<GeneratedEdit> {
    let invalid = |message: &str| ToolkitError::Invalid(message.into());
    let source = project
        .drawings
        .iter()
        .find(|d| d.name == drawing)
        .ok_or_else(|| invalid("drawing does not exist"))?;
    let mut remaining: BTreeSet<_> = entity_ids.iter().map(String::as_str).collect();
    if remaining.len() != entity_ids.len() {
        return Err(invalid("text selection contains duplicate IDs"));
    }
    let mut operations = Vec::new();
    for record in &source.entities {
        let id = record.entity.id().as_str();
        if !entity_ids.is_empty() && !remaining.remove(id) {
            continue;
        }
        if let Entity::Text {
            writing_mode: current,
            ..
        } = &record.entity
            && *current != writing_mode
        {
            let mut entity = serde_json::to_value(&record.entity)?;
            entity["writing_mode"] = json!(writing_mode);
            operations.push(EditOperation::Replace {
                entity_id: id.into(),
                entity,
            });
        }
    }
    if !remaining.is_empty() {
        return Err(invalid("text selection includes missing entities"));
    }
    if operations.is_empty() {
        return Err(invalid("no text writing modes would change"));
    }
    Ok(GeneratedEdit { operation: EditOperation::Batch { operations }, warnings: vec!["Vertical upright text places one Unicode scalar per cell, with LF starting a column to the left. It does not perform vertical punctuation substitution, combining-character shaping, ruby or tate-chu-yoko.".into()], analysis_report: None })
}

pub fn set_style(
    project: &ProjectSource,
    drawing: &str,
    request: &TextStyleRequest,
) -> Result<GeneratedEdit> {
    let invalid = |message: &str| ToolkitError::Invalid(message.into());
    if !project.styles.text_styles.contains_key(&request.style) {
        return Err(invalid("text style does not exist"));
    }
    let source = project
        .drawings
        .iter()
        .find(|d| d.name == drawing)
        .ok_or_else(|| invalid("drawing does not exist"))?;
    let mut remaining: BTreeSet<_> = request.entity_ids.iter().map(String::as_str).collect();
    if remaining.len() != request.entity_ids.len() {
        return Err(invalid("text selection contains duplicate IDs"));
    }
    let mut operations = Vec::new();
    for record in &source.entities {
        let id = record.entity.id().as_str();
        if !request.entity_ids.is_empty() && !remaining.remove(id) {
            continue;
        }
        if let Entity::Text { style, .. } = &record.entity
            && style != &request.style
        {
            let mut entity = serde_json::to_value(&record.entity)?;
            entity["style"] = json!(request.style);
            operations.push(EditOperation::Replace {
                entity_id: id.into(),
                entity,
            });
        }
    }
    if !remaining.is_empty() {
        return Err(invalid("text selection includes missing entities"));
    }
    if operations.is_empty() {
        return Err(invalid("no text styles would change"));
    }
    Ok(GeneratedEdit {
        operation: EditOperation::Batch { operations },
        warnings: Vec::new(),
        analysis_report: None,
    })
}

/// Finite scalar arithmetic, never a program evaluator or unit-conversion engine.
pub fn calculate(expression: &str, precision: u8) -> Result<String> {
    if expression.len() > 4096 || !expression.is_ascii() || precision > 12 {
        return Err(ToolkitError::Invalid(
            "use an ASCII expression up to 4096 bytes and precision 0..12".into(),
        ));
    }
    let mut parser = Arithmetic {
        input: expression.as_bytes(),
        position: 0,
        depth: 0,
    };
    let value = parser.expression()?;
    parser.whitespace();
    if parser.position != parser.input.len() {
        return Err(parser.error("unexpected expression suffix"));
    }
    let text = format!("{value:.precision$}", precision = precision as usize);
    // Rounded negative zero is displayed as zero.
    Ok(
        if text.starts_with('-') && text.parse::<f64>().ok() == Some(0.) {
            text[1..].into()
        } else {
            text
        },
    )
}

struct Arithmetic<'a> {
    input: &'a [u8],
    position: usize,
    depth: usize,
}
impl Arithmetic<'_> {
    fn error(&self, message: &str) -> ToolkitError {
        ToolkitError::Invalid(format!(
            "{message} at expression byte {}",
            self.position + 1
        ))
    }
    fn whitespace(&mut self) {
        while self
            .input
            .get(self.position)
            .is_some_and(u8::is_ascii_whitespace)
        {
            self.position += 1;
        }
    }
    fn take(&mut self, byte: u8) -> bool {
        self.whitespace();
        if self.input.get(self.position) == Some(&byte) {
            self.position += 1;
            true
        } else {
            false
        }
    }
    fn finite(&self, value: f64) -> Result<f64> {
        if value.is_finite() {
            Ok(value)
        } else {
            Err(self.error("arithmetic exceeds the finite range"))
        }
    }
    fn expression(&mut self) -> Result<f64> {
        let mut value = self.term()?;
        loop {
            if self.take(b'+') {
                let right = self.term()?;
                value = self.finite(value + right)?;
            } else if self.take(b'-') {
                let right = self.term()?;
                value = self.finite(value - right)?;
            } else {
                return Ok(value);
            }
        }
    }
    fn term(&mut self) -> Result<f64> {
        let mut value = self.unary()?;
        loop {
            let divide = if self.take(b'*') {
                false
            } else if self.take(b'/') {
                true
            } else {
                return Ok(value);
            };
            let right = self.unary()?;
            if divide && right == 0. {
                return Err(self.error("division by zero"));
            }
            let result = self.finite(if divide { value / right } else { value * right })?;
            if result == 0. && value != 0. && right != 0. {
                return Err(self.error("arithmetic underflows the finite range"));
            }
            value = result;
        }
    }
    fn unary(&mut self) -> Result<f64> {
        let mut sign = 1.;
        loop {
            if self.take(b'-') {
                sign = -sign;
            } else if !self.take(b'+') {
                break;
            }
        }
        if self.take(b'(') {
            self.depth += 1;
            if self.depth > 64 {
                return Err(self.error("parentheses nesting exceeds 64"));
            }
            let value = self.expression()?;
            if !self.take(b')') {
                return Err(self.error("closing parenthesis is missing"));
            }
            self.depth -= 1;
            return self.finite(sign * value);
        }
        let start = self.position;
        let mut digits = 0;
        while self
            .input
            .get(self.position)
            .is_some_and(u8::is_ascii_digit)
        {
            self.position += 1;
            digits += 1;
        }
        if self.input.get(self.position) == Some(&b'.') {
            self.position += 1;
            while self
                .input
                .get(self.position)
                .is_some_and(u8::is_ascii_digit)
            {
                self.position += 1;
                digits += 1;
            }
        }
        let mantissa_end = self.position;
        if digits == 0 {
            return Err(self.error("a number or parenthesized expression is required"));
        }
        if matches!(self.input.get(self.position), Some(b'e' | b'E')) {
            self.position += 1;
            if matches!(self.input.get(self.position), Some(b'+' | b'-')) {
                self.position += 1;
            }
            let exponent_start = self.position;
            while self
                .input
                .get(self.position)
                .is_some_and(u8::is_ascii_digit)
            {
                self.position += 1;
            }
            if self.position == exponent_start {
                return Err(self.error("exponent digits are missing"));
            }
        }
        let literal =
            std::str::from_utf8(&self.input[start..self.position]).expect("ASCII was checked");
        let value = literal
            .parse::<f64>()
            .map_err(|_| self.error("invalid number"))?;
        if value == 0.
            && self.input[start..mantissa_end]
                .iter()
                .any(|b| matches!(b, b'1'..=b'9'))
        {
            return Err(self.error("number underflows the finite range"));
        }
        self.finite(sign * value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn writing_direction_preserves_annotation_identity_and_supports_checked_undo() {
        let temp = tempfile::tempdir().unwrap();
        let created = cad_edit::create_project(&cad_edit::ProjectTemplateRequest {
            parent_dir: temp.path().display().to_string(),
            folder_name: "vertical".into(),
            project_name: "Vertical".into(),
            drawing: "plan".into(),
            paper: "A4".into(),
            orientation: cad_model::SheetOrientation::Portrait,
            scale_denominator: 100,
        })
        .unwrap();
        let root = std::path::Path::new(&created.project_path);
        let initial = cad_model::load_project(root).unwrap();
        let style = initial.styles.text_styles.keys().next().unwrap().clone();
        let created_text=cad_edit::apply_edit(root,&cad_edit::DrawingEditRequest {drawing:"plan".into(),expected_revision:cad_edit::editor_state(&initial,"plan").unwrap().revision,operation:EditOperation::Create {entity:json!({"type":"text","layer":"0-1","style":style,"at":[1000,2000],"rotation_deg":30,"mirror_y":true,"value":"室名\nA2"})}}).unwrap();
        let source = cad_model::load_project(root).unwrap();
        let original = std::fs::read(root.join("drawings/plan/entities.ndjson")).unwrap();
        let id = created_text.entity_ids[0].clone();
        let generated = set_writing_mode(
            &source,
            "plan",
            cad_model::TextWritingMode::VerticalUpright,
            std::slice::from_ref(&id),
        )
        .unwrap();
        let request = cad_edit::DrawingEditRequest {
            drawing: "plan".into(),
            expected_revision: cad_edit::editor_state(&source, "plan").unwrap().revision,
            operation: EditOperation::SourceChecked {
                expected_files: cad_model::source_manifest(root).unwrap(),
                operation: Box::new(generated.operation),
            },
        };
        cad_edit::preview_edit(root, &request).unwrap();
        assert_eq!(
            std::fs::read(root.join("drawings/plan/entities.ndjson")).unwrap(),
            original
        );
        cad_edit::apply_edit(root, &request).unwrap();
        let updated = cad_model::load_project(root).unwrap();
        assert!(cad_check::check_loaded_project(&updated).is_ok());
        let mut invalid = updated.clone();
        if let Entity::Text { value, .. } = &mut invalid.drawings[0].entities[0].entity {
            value.push('\t');
        }
        assert!(
            cad_check::check_loaded_project(&invalid)
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "text.invalid_upright_control")
        );
        let part = crate::clipboard::capture(
            root,
            "plan",
            std::slice::from_ref(&id),
            [1000., 2000.],
            crate::clipboard::DimensionCopyPolicy::IncludeReferences,
        )
        .unwrap();
        assert!(matches!(
            part.entities[0],
            Entity::Text {
                writing_mode: cad_model::TextWritingMode::VerticalUpright,
                ..
            }
        ));
        assert!(
            crate::clipboard::preview_document(&part)
                .unwrap()
                .contains("data-writing-mode=\"vertical_upright\"")
        );
        let mut expected = serde_json::to_value(&source.drawings[0].entities[0].entity).unwrap();
        expected["writing_mode"] = json!("vertical_upright");
        assert_eq!(
            serde_json::to_value(&updated.drawings[0].entities[0].entity).unwrap(),
            expected
        );
        let difference = cad_diff::diff_projects(&source, &updated);
        assert_eq!(difference.changes.len(), 1);
        assert!(
            difference.changes[0]
                .reasons
                .contains(&cad_diff::ChangeReason::GeometryChanged)
        );
        assert!(cad_diff::render_diff_svg(&source, &updated, &difference).contains("室"));
        assert!(cad_edit::apply_edit(root, &request).is_err());
        let state = cad_edit::list_drawing_history(root, "plan").unwrap();
        cad_edit::undo_drawing_edit(
            root,
            &cad_edit::DrawingHistoryRequest {
                drawing: "plan".into(),
                expected_files: state.current_files,
            },
        )
        .unwrap();
        assert_eq!(
            std::fs::read(root.join("drawings/plan/entities.ndjson")).unwrap(),
            original
        );
        assert!(
            set_writing_mode(
                &source,
                "plan",
                cad_model::TextWritingMode::Horizontal,
                std::slice::from_ref(&id)
            )
            .is_err()
        );
        assert!(
            set_writing_mode(
                &source,
                "plan",
                cad_model::TextWritingMode::VerticalUpright,
                &[id.clone(), id]
            )
            .is_err()
        );
        assert!(
            set_writing_mode(
                &source,
                "plan",
                cad_model::TextWritingMode::VerticalUpright,
                &["missing".into()]
            )
            .is_err()
        );
    }

    #[test]
    fn arithmetic_obeys_precedence_unary_scientific_literals_and_rounding() {
        for (expression, precision, expected) in [
            ("2 + 3 * 4", 0, "14"),
            ("(2+3)*4", 2, "20.00"),
            ("-(-2) + +3", 1, "5.0"),
            ("1e3 / 4", 2, "250.00"),
            (".5 + 1.5E-1", 3, "0.650"),
            ("1/3", 4, "0.3333"),
            ("-0.004", 2, "0.00"),
            ("1000*2000/1e6", 2, "2.00"),
        ] {
            assert_eq!(calculate(expression, precision).unwrap(), expected);
        }
        for expression in [
            "",
            "1+",
            "(1+2",
            "1+2)",
            "1 2",
            "1/0",
            "1/(2-2)",
            "1e999",
            "1e-999",
            "1e308*10",
            "1e-300*1e-300",
            "1e-300/1e300",
            "NaN",
            "Infinity",
            "sqrt(4)",
            "2^3",
            "2**3",
            "2×3",
            ".",
            "1e+",
            "2mm",
        ] {
            assert!(calculate(expression, 2).is_err(), "accepted {expression}");
        }
        assert!(calculate("1", 13).is_err());
        assert!(calculate(&"1".repeat(4097), 2).is_err());
        assert!(calculate(&format!("{}1{}", "(".repeat(65), ")".repeat(65)), 2).is_err());
        assert_eq!(
            calculate(&format!("{}1{}", "(".repeat(64), ")".repeat(64)), 0).unwrap(),
            "1"
        );
    }

    #[test]
    fn style_only_preserves_text_geometry_and_excludes_dimensions() {
        let mut source = cad_model::load_project(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/cad-acceptance"),
        )
        .unwrap();
        let original_style = source.styles.text_styles.keys().next().unwrap().clone();
        source.styles.text_styles.insert(
            "new_note".into(),
            source.styles.text_styles[&original_style].clone(),
        );
        let drawing = source
            .drawings
            .iter()
            .find(|d| {
                d.entities
                    .iter()
                    .any(|e| matches!(e.entity, Entity::Text { .. }))
            })
            .unwrap();
        let text = drawing
            .entities
            .iter()
            .find(|e| matches!(e.entity, Entity::Text { .. }))
            .unwrap();
        let id = text.entity.id().as_str().to_string();
        let request = |entity_ids, style: &str| TextStyleRequest {
            entity_ids,
            style: style.into(),
        };
        let generated = set_style(
            &source,
            &drawing.name,
            &request(vec![id.clone()], "new_note"),
        )
        .unwrap();
        let EditOperation::Batch { operations } = generated.operation else {
            panic!()
        };
        assert_eq!(operations.len(), 1);
        let EditOperation::Replace { entity_id, entity } = &operations[0] else {
            panic!()
        };
        assert_eq!(entity_id, &id);
        let mut expected = serde_json::to_value(&text.entity).unwrap();
        expected["style"] = json!("new_note");
        assert_eq!(entity, &expected);
        let all = set_style(&source, &drawing.name, &request(Vec::new(), "new_note")).unwrap();
        let EditOperation::Batch { operations } = all.operation else {
            panic!()
        };
        assert!(
            operations.iter().all(
                |o| matches!(o, EditOperation::Replace {entity, ..} if entity["type"] == "text")
            )
        );
        assert!(
            set_style(
                &source,
                &drawing.name,
                &request(vec![id.clone(), id.clone()], "new_note")
            )
            .is_err()
        );
        assert!(
            set_style(
                &source,
                &drawing.name,
                &request(vec!["missing".into()], "new_note")
            )
            .is_err()
        );
        assert!(
            set_style(
                &source,
                &drawing.name,
                &request(vec![id.clone()], "missing")
            )
            .is_err()
        );
        let Entity::Text { style, .. } = &text.entity else {
            panic!()
        };
        assert!(set_style(&source, &drawing.name, &request(vec![id], style)).is_err());
    }
}
