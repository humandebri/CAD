use miette::{IntoDiagnostic, Result, miette};
use std::path::Path;

pub fn export_dxf(
    project: &Path,
    drawing: &str,
    out: &Path,
    report: &Path,
    strict: bool,
) -> Result<()> {
    let result =
        cad_exchange::files::export_dxf(project, drawing, out, report, strict).into_diagnostic()?;
    if result.status == "blocked" {
        return Err(miette!(
            "DXF conversion blocked; compatibility report retained"
        ));
    }
    Ok(())
}

pub fn import_dxf(input: &Path, out: &Path, report: &Path, unit_mm: Option<f64>) -> Result<()> {
    let result = cad_exchange::files::import_dxf(input, out, report, unit_mm).into_diagnostic()?;
    if result.status == "blocked" {
        return Err(miette!(
            "DXF import blocked; compatibility report retained; no project published"
        ));
    }
    eprintln!("Imported checked project at {}", out.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[test]
    fn export_import_checks_candidate_and_never_overwrites_existing_outputs() {
        let temp = tempfile::tempdir().unwrap();
        let created = cad_edit::create_project(&cad_edit::ProjectTemplateRequest {
            parent_dir: temp.path().display().to_string(),
            folder_name: "source".into(),
            project_name: "exchange".into(),
            drawing: "plan".into(),
            paper: "A3".into(),
            orientation: cad_model::SheetOrientation::Landscape,
            scale_denominator: 100,
        })
        .unwrap();
        let root = Path::new(&created.project_path);
        fs::write(root.join("drawings/plan/entities.ndjson"), "{\"schema_version\":\"0.3\",\"id\":\"ent_01JZ0000000000000000000000\",\"type\":\"line\",\"layer\":\"0-1\",\"p1\":[0,0],\"p2\":[1234,5678]}\n").unwrap();
        let before = cad_model::source_manifest(root).unwrap();
        let dxf = temp.path().join("output.dxf");
        let report = temp.path().join("export-report.json");
        export_dxf(root, "plan", &dxf, &report, false).unwrap();
        assert_eq!(cad_model::source_manifest(root).unwrap(), before);
        let output = temp.path().join("imported");
        let import_report = temp.path().join("import-report.json");
        import_dxf(&dxf, &output, &import_report, None).unwrap();
        assert!(cad_check::check_project(&output).is_ok());
        let source = cad_model::load_project(&output).unwrap();
        assert!(
            matches!(&source.drawings[0].entities[0].entity,cad_model::Entity::Line { p2,..} if *p2==[1234.0,5678.0])
        );
        assert!(import_dxf(&dxf, &output, &temp.path().join("second.json"), None).is_err());
        assert!(export_dxf(root, "plan", &dxf, &report, false).is_err());
        let malformed = temp.path().join("bad.dxf");
        fs::write(&malformed, b"broken").unwrap();
        let blocked = temp.path().join("blocked");
        let blocked_report = temp.path().join("blocked.json");
        assert!(import_dxf(&malformed, &blocked, &blocked_report, None).is_err());
        assert!(!blocked.exists());
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(blocked_report).unwrap()).unwrap();
        assert_eq!(report["exchange"]["status"], "blocked");
        assert!(
            !report["exchange"]["blockers"]
                .as_array()
                .unwrap()
                .is_empty()
        );
    }
}
