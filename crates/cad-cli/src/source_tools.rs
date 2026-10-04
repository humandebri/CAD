use miette::{IntoDiagnostic, Result, miette};
use serde_json::json;
use std::{fs, path::Path};

pub fn schemas(out: &Path) -> Result<()> {
    let mut schemas = cad_model::source_schema::schemas();
    schemas.insert(
        "draft-request".into(),
        serde_json::to_value(schemars::schema_for!(
            cad_toolkit::source_draft::DraftRequest
        ))
        .into_diagnostic()?,
    );
    create_output_directory(out)?;
    for (name, schema) in schemas {
        cad_edit::atomic_publish(
            &out.join(format!("{name}.schema.json")),
            &serde_json::to_vec_pretty(&schema).into_diagnostic()?,
            false,
        )
        .into_diagnostic()?;
    }
    Ok(())
}

fn create_output_directory(out: &Path) -> Result<()> {
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).into_diagnostic()?;
    }
    // The leaf remains exclusive: do not overwrite an existing output set.
    fs::create_dir(out).into_diagnostic()
}

pub fn coordinates(project: &Path, drawing: &str, out: &Path) -> Result<()> {
    let initial = cad_model::source_manifest(project).into_diagnostic()?;
    let source = cad_model::load_project(project).into_diagnostic()?;
    let drawing_source = source
        .drawings
        .iter()
        .find(|d| d.name == drawing)
        .ok_or_else(|| miette!("missing drawing {drawing}"))?;
    let manifest = cad_model::source_manifest(project).into_diagnostic()?;
    let preservation = cad_model::verified_jww_preservation_snapshot(project).into_diagnostic()?;
    let mut import_coordinates = None;
    let mut origin = "canonical_model_mm";
    let mut warnings = Vec::new();
    if let Some(snapshot) = preservation.filter(|s| s.manifest.drawing_name == drawing) {
        origin = "jww_import_requires_unit_confirmation";
        if let Ok(document) = cad_jww_codec::read_document(&snapshot.original_bytes) {
            import_coordinates = Some(cad_import_jww::coordinate_report(&document));
        }
        warnings.push("The original drawing retained JWW record coordinates; current edits may have changed their meaning. Group scale is evidence, not a confirmed conversion factor. Compare a known dimension and specify an explicit conversion for a derived drawing.");
    }
    if initial != manifest {
        return Err(miette!("source changed while reading coordinates"));
    }
    let layouts: Vec<_> = drawing_source.layouts.layouts.iter().map(|(id, l)| json!({"id":id,"scale":l.scale,"model_units_per_paper_mm":cad_model::parse_layout_scale(&l.scale),"origin":l.origin})).collect();
    let report = json!({"schema_version":"cad-coordinate-report/1","drawing":drawing,"coordinate_origin":origin,"canonical_unit":"mm","layout_scale_changes_stored_coordinates":false,"import":import_coordinates,"layouts":layouts,"source_files":manifest,"warnings":warnings});
    if out == Path::new("-") {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).into_diagnostic()?
        );
    } else {
        super::review_bundle::validate_output_directory(project, out)?;
        cad_edit::atomic_publish(
            out,
            &serde_json::to_vec_pretty(&report).into_diagnostic()?,
            false,
        )
        .into_diagnostic()?;
    }
    Ok(())
}

pub fn draft(
    project: &Path,
    drawing: &str,
    request: &Path,
    previous: Option<&Path>,
    out: &Path,
) -> Result<()> {
    super::review_bundle::validate_output_directory(project, out)?;
    cad_edit::recover_source_transactions(project).into_diagnostic()?;
    let initial = cad_model::source_manifest(project).into_diagnostic()?;
    let source = cad_model::load_project(project).into_diagnostic()?;
    let check = cad_check::check_loaded_project(&source);
    if !check.is_ok() {
        return Err(miette!(
            "source failed CAD validation: {}",
            serde_json::to_string(&check).into_diagnostic()?
        ));
    }
    let request =
        serde_json::from_slice(&fs::read(request).into_diagnostic()?).into_diagnostic()?;
    let previous = previous
        .map(|path| -> Result<cad_toolkit::source_draft::DraftReport> {
            serde_json::from_slice(&fs::read(path).into_diagnostic()?).into_diagnostic()
        })
        .transpose()?;
    let candidate = cad_toolkit::source_draft::draft(&source, drawing, &request, previous.as_ref())
        .into_diagnostic()?;
    if initial != cad_model::source_manifest(project).into_diagnostic()?
        || initial != candidate.report.source_files
    {
        return Err(miette!(
            "source changed while drafting; regenerate the candidate"
        ));
    }
    create_output_directory(out)?;
    cad_edit::atomic_publish(
        &out.join("entities.ndjson"),
        candidate.ndjson.as_bytes(),
        false,
    )
    .into_diagnostic()?;
    cad_edit::atomic_publish(
        &out.join("report.json"),
        &serde_json::to_vec_pretty(&candidate.report).into_diagnostic()?,
        false,
    )
    .into_diagnostic()?;
    println!(
        "Checked candidate: {} (source unchanged)",
        out.join("entities.ndjson").display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schemas_create_missing_parents_without_overwriting_existing_outputs() {
        let temp = tempfile::tempdir().unwrap();
        let out = temp.path().join("build/ci-source-schemas");
        schemas(&out).unwrap();
        let before = fs::read(out.join("entity.schema.json")).unwrap();
        assert!(schemas(&out).is_err());
        assert_eq!(fs::read(out.join("entity.schema.json")).unwrap(), before);
        assert_eq!(fs::read_dir(&out).unwrap().count(), 7);
    }
    #[test]
    fn draft_outputs_are_checked_read_only_and_cannot_write_into_canonical_directories() {
        let project =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/direct-edit-guide");
        let request =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/direct-edit/repeat.json");
        let before = cad_model::source_manifest(&project).unwrap();
        let temp = tempfile::tempdir().unwrap();
        let out = temp.path().join("build/candidate");
        draft(&project, "acceptance", &request, None, &out).unwrap();
        let report: cad_toolkit::source_draft::DraftReport =
            serde_json::from_slice(&fs::read(out.join("report.json")).unwrap()).unwrap();
        assert!(report.cad_check.is_ok());
        assert_eq!(report.source_files, before);
        assert_eq!(report.generated.len(), 6);
        assert!(
            draft(
                &project,
                "acceptance",
                &request,
                None,
                &project.join("drawings/forbidden-candidate")
            )
            .is_err()
        );
        assert!(!project.join("drawings/forbidden-candidate").exists());
        assert_eq!(cad_model::source_manifest(&project).unwrap(), before);
    }
    #[test]
    fn coordinate_reports_distinguish_preserved_jww_records_from_model_mm() {
        let input = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/jww-fixtures/apartment-plan.jww");
        let temp = tempfile::tempdir().unwrap();
        let project = temp.path().join("imported");
        let report = cad_import_jww::import_jww_file(&input, &project).unwrap();
        assert!(!report.coordinates.unwrap().model_normalized);
        let before = cad_model::source_manifest(&project).unwrap();
        let out = temp.path().join("coordinates.json");
        coordinates(&project, "apartment-plan", &out).unwrap();
        let report: serde_json::Value = serde_json::from_slice(&fs::read(out).unwrap()).unwrap();
        assert_eq!(
            report["coordinate_origin"],
            "jww_import_requires_unit_confirmation"
        );
        assert_eq!(
            report["import"]["group_scale_denominators"]["jww_g0"],
            100.0
        );
        assert_eq!(cad_model::source_manifest(&project).unwrap(), before);
    }
}
