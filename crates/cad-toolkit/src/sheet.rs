//! Reviewed layout viewport updates; source geometry is never transformed.
use crate::{Result, ToolkitError};
use cad_model::{LayoutConfig, LayoutViewport, SourceFileRevision};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
fn invalid(message: impl ToString) -> ToolkitError {
    ToolkitError::Invalid(message.to_string())
}
#[derive(Debug, Serialize)]
pub struct Report {
    pub schema_version: String,
    pub project_path: PathBuf,
    pub drawing: String,
    pub layout: String,
    pub before: LayoutConfig,
    pub after: LayoutConfig,
    pub source_files: Vec<SourceFileRevision>,
    pub plan_hash: String,
    pub warnings: Vec<String>,
}
pub struct Plan {
    pub report: Report,
    pub svg: String,
    request: cad_edit::source_replacements::SourceReplacementRequest,
}
pub fn plan(
    root: &Path,
    drawing: &str,
    layout: &str,
    viewports: Vec<LayoutViewport>,
) -> Result<Plan> {
    let root = root.canonicalize().map_err(invalid)?;
    let files = cad_model::source_manifest(&root).map_err(invalid)?;
    let mut source = cad_model::load_project(&root).map_err(invalid)?;
    if !cad_check::check_loaded_project(&source).is_ok() {
        return Err(invalid("Sheet source failed CAD validation"));
    }
    let index = source
        .drawings
        .iter()
        .position(|d| d.name == drawing)
        .ok_or_else(|| invalid("Sheet drawing is missing"))?;
    let before = source.drawings[index]
        .layouts
        .layouts
        .get(layout)
        .cloned()
        .ok_or_else(|| invalid("Sheet layout is missing"))?;
    let mut after = before.clone();
    after.viewports = viewports;
    cad_model::validate_layout_viewports(&source, &after).map_err(invalid)?;
    source.drawings[index]
        .layouts
        .layouts
        .insert(layout.into(), after.clone());
    let bytes = toml::to_string_pretty(&source.drawings[index].layouts)
        .map_err(invalid)?
        .into_bytes();
    let relative = format!("drawings/{drawing}/layouts.toml");
    if !root.join(&relative).is_file() {
        return Err(invalid(
            "Viewport editing requires an existing layouts.toml",
        ));
    }
    source.drawings[index].layouts.active_layout = layout.into();
    let svg = cad_render_svg::render_drawing_svg(&source, drawing).map_err(invalid)?;
    if cad_model::source_manifest(&root).map_err(invalid)? != files {
        return Err(invalid("Sheet source changed during preview"));
    }
    let mut report=Report {schema_version:"cad-sheet-review/1".into(),project_path:root,drawing:drawing.into(),layout:layout.into(),before,after,source_files:files.clone(),plan_hash:String::new(),warnings:vec!["Viewports replace the sheet's ordinary model display; an empty viewport list restores it. Source geometry and dimension measurements remain unchanged.".into(),"Viewport text uses model-space style dimensions; its printed size follows the viewport scale. Pen widths remain paper millimetres.".into(),"JWW viewport clipping/placement has no validated mapping and blocks export. Use SVG/PDF or a model layout.".into(),"Updating viewports rewrites the layout TOML file with its parsed settings and participates in project Undo.".into()]};
    report.plan_hash = blake3::hash(&serde_json::to_vec(&report)?)
        .to_hex()
        .to_string();
    Ok(Plan {
        report,
        svg,
        request: cad_edit::source_replacements::SourceReplacementRequest {
            drawing: drawing.into(),
            expected_files: files,
            files: BTreeMap::from([(relative, bytes)]),
        },
    })
}
pub fn apply(plan: &Plan, expected_plan: &str) -> Result<()> {
    if expected_plan != plan.report.plan_hash {
        return Err(invalid(
            "Sheet preview hash changed; review a fresh preview",
        ));
    }
    cad_edit::source_replacements::apply(&plan.report.project_path, &plan.request)
        .map_err(invalid)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn mixed_scales_clip_without_editing_geometry_and_restore_with_undo() {
        let temp = tempfile::tempdir().unwrap();
        let created = cad_edit::create_project(&cad_edit::ProjectTemplateRequest {
            parent_dir: temp.path().display().to_string(),
            folder_name: "sheet".into(),
            project_name: "Sheet".into(),
            drawing: "plan".into(),
            paper: "A4".into(),
            orientation: cad_model::SheetOrientation::Landscape,
            scale_denominator: 100,
        })
        .unwrap();
        let root = Path::new(&created.project_path);
        let initial = cad_model::load_project(root).unwrap();
        let created_line=cad_edit::apply_edit(root,&cad_edit::DrawingEditRequest {drawing:"plan".into(),expected_revision:cad_edit::editor_state(&initial,"plan").unwrap().revision,operation:cad_edit::EditOperation::Create {entity:serde_json::json!({"schema_version":cad_model::CURRENT_SCHEMA_VERSION,"id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0,0],"p2":[1000,0]})}}).unwrap();
        let geometry = std::fs::read(root.join("drawings/plan/entities.ndjson")).unwrap();
        let layouts = std::fs::read(root.join("drawings/plan/layouts.toml")).unwrap();
        let views = vec![
            LayoutViewport {
                name: "Plan".into(),
                drawing: "plan".into(),
                origin: [1000., 2000.],
                at_mm: [10., 10.],
                size_mm: [100., 100.],
                scale: "1/100".into(),
                layers: vec![],
            },
            LayoutViewport {
                name: "Detail".into(),
                drawing: "plan".into(),
                origin: [1000., 2000.],
                at_mm: [150., 10.],
                size_mm: [100., 100.],
                scale: "1/20".into(),
                layers: vec!["0-1".into()],
            },
        ];
        let candidate = plan(root, "plan", "default", views.clone()).unwrap();
        assert!(candidate.svg.contains("data-viewport-scale=\"1/20\""));
        assert!(candidate.svg.contains("clipPath"));
        assert!(apply(&candidate, "stale").is_err());
        assert_eq!(
            std::fs::read(root.join("drawings/plan/layouts.toml")).unwrap(),
            layouts
        );
        apply(&candidate, &candidate.report.plan_hash).unwrap();
        assert!(cad_check::check_project(root).is_ok());
        let jww = cad_check::check_project_for_target(
            root,
            cad_check::CheckTarget::JwwV600,
            Some("plan"),
        );
        assert!(
            jww.diagnostics
                .iter()
                .any(|d| d.code == "jww.sheet_viewports_unsupported"
                    && d.severity == cad_check::Severity::Error)
        );
        assert_eq!(
            cad_model::load_project(root).unwrap().drawings[0]
                .layouts
                .active()
                .unwrap()
                .viewports,
            views
        );
        assert_eq!(
            std::fs::read(root.join("drawings/plan/entities.ndjson")).unwrap(),
            geometry
        );
        let part = crate::clipboard::capture(
            root,
            "plan",
            &created_line.entity_ids,
            [0., 0.],
            crate::clipboard::DimensionCopyPolicy::IncludeReferences,
        )
        .unwrap();
        assert_eq!(part.layouts.layouts["default"].viewports.len(), 2);
        assert!(
            crate::clipboard::preview_document(&part)
                .unwrap()
                .contains("data-entity-id")
        );
        assert!(
            part.warnings
                .iter()
                .any(|warning| warning.contains("metadata only"))
        );
        assert!(apply(&candidate, &candidate.report.plan_hash).is_err());
        cad_edit::undo_drawing_edit(
            root,
            &cad_edit::DrawingHistoryRequest {
                drawing: "plan".into(),
                expected_files: cad_edit::list_drawing_history(root, "plan")
                    .unwrap()
                    .current_files,
            },
        )
        .unwrap();
        assert_eq!(
            std::fs::read(root.join("drawings/plan/layouts.toml")).unwrap(),
            layouts
        );
        let mut invalid_views = views;
        invalid_views[0].scale = "0/100".into();
        assert!(plan(root, "plan", "default", invalid_views.clone()).is_err());
        invalid_views[0].scale = "1/100".into();
        invalid_views[1].drawing = "missing".into();
        assert!(plan(root, "plan", "default", invalid_views).is_err());
    }
}
