use miette::{IntoDiagnostic, Result, miette};
use std::{fs, path::Path};

pub fn copy_part(
    input: &Path,
    out: &Path,
    report_path: &Path,
    coordinate_scale: f64,
) -> Result<()> {
    if out == Path::new("-") {
        return Err(miette!(
            "JWS part output requires a new file; stdout is reserved for the compatibility report"
        ));
    }
    let source = fs::canonicalize(input).into_diagnostic()?;
    let parent = source
        .parent()
        .ok_or_else(|| miette!("JWS source needs a parent directory"))?;
    let output = cad_exchange::files::new_artifact_path(parent, out).into_diagnostic()?;
    let report = if report_path == Path::new("-") {
        None
    } else {
        let report =
            cad_exchange::files::new_artifact_path(parent, report_path).into_diagnostic()?;
        if report == output || report.starts_with(&output) || output.starts_with(&report) {
            return Err(miette!("Part and report require separate paths"));
        }
        Some(report)
    };
    let imported = cad_toolkit::clipboard_jws::read(&source, coordinate_scale).into_diagnostic()?;
    let bytes = serde_json::to_vec_pretty(&imported.report).into_diagnostic()?;
    if let Some(report) = report {
        fs::create_dir_all(
            report
                .parent()
                .ok_or_else(|| miette!("Report needs a parent directory"))?,
        )
        .into_diagnostic()?;
        cad_edit::atomic_publish(&report, &bytes, false).into_diagnostic()?;
    } else {
        println!("{}", String::from_utf8_lossy(&bytes));
    }
    let document = imported.document.ok_or_else(|| {
        miette!("JWS part conversion blocked; compatibility report retained; no part published")
    })?;
    let bytes = cad_toolkit::clipboard::serialize_document(&document).into_diagnostic()?;
    fs::create_dir_all(
        output
            .parent()
            .ok_or_else(|| miette!("Part needs a parent directory"))?,
    )
    .into_diagnostic()?;
    cad_edit::atomic_publish(&output, &bytes, false).into_diagnostic()?;
    Ok(())
}

pub fn import(input: &Path, out: &Path, report_path: &Path, coordinate_scale: f64) -> Result<()> {
    let output = super::resolve_output_path(out)?;
    if fs::symlink_metadata(&output).is_ok() {
        return Err(miette!("JWS import requires a new project directory"));
    }
    if report_path != Path::new("-") {
        let resolved_report = super::resolve_output_path(report_path)?;
        if resolved_report.starts_with(&output) {
            return Err(miette!(
                "Import report must be outside the new project directory"
            ));
        }
        if fs::symlink_metadata(&resolved_report).is_ok() {
            return Err(miette!("Import report already exists"));
        }
    }
    let data = fs::read(input).into_diagnostic()?;
    let parent = output
        .parent()
        .ok_or_else(|| miette!("Import output needs a parent directory"))?;
    fs::create_dir_all(parent).into_diagnostic()?;
    let candidate = tempfile::Builder::new()
        .prefix(".cad-jws-import-")
        .tempdir_in(parent)
        .into_diagnostic()?;
    let prepared = cad_import_jww::prepare_symbol_import(
        &data,
        input
            .file_name()
            .and_then(|p| p.to_str())
            .unwrap_or("JWS import"),
        coordinate_scale,
        candidate.path(),
    )
    .into_diagnostic()?;
    let bytes = serde_json::to_vec_pretty(&prepared).into_diagnostic()?;
    if report_path == Path::new("-") {
        println!("{}", String::from_utf8_lossy(&bytes));
    } else {
        let path = super::resolve_output_path(report_path)?;
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| miette!("Report needs a parent directory"))?,
        )
        .into_diagnostic()?;
        cad_edit::atomic_publish(&path, &bytes, false).into_diagnostic()?;
    }
    if prepared.status != "converted" {
        return Err(miette!(
            "JWS import blocked; compatibility report retained; no project published"
        ));
    }
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        candidate.path(),
        rustix::fs::CWD,
        &output,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .into_diagnostic()?;
    eprintln!("Imported checked JWS geometry at {}", output.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn portable_conversion_retains_blocked_report_without_overwriting_sources() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("unknown.jws");
        fs::write(&input, b"unknown").unwrap();
        let part = temp.path().join("part.cadpart.json");
        let report = temp.path().join("report.json");
        assert!(copy_part(&input, &part, &report, 100.).is_err());
        assert!(!part.exists());
        assert_eq!(fs::read(&input).unwrap(), b"unknown");
        let bytes = fs::read(&report).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["status"], "blocked");
        assert!(!value["blockers"].as_array().unwrap().is_empty());
        assert!(copy_part(&input, &part, &report, 100.).is_err());
        assert_eq!(fs::read(&report).unwrap(), bytes);
        assert!(copy_part(&input, &input, &temp.path().join("report2.json"), 100.).is_err());
        assert!(copy_part(&input, &part, &part, 100.).is_err());
    }
    #[test]
    fn checked_candidate_is_published_with_report_and_source_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let mut data = b"JwsData.".to_vec();
        data.extend([b'.'; 192]);
        data.extend(600u32.to_le_bytes());
        for value in [0f64, 0.] {
            data.extend(value.to_le_bytes());
        }
        for _ in 0..16 {
            data.extend(100f64.to_le_bytes());
        }
        data.extend([0; 72]);
        for value in [0f64, 0., 10., 20.] {
            data.extend(value.to_le_bytes());
        }
        let mut writer = cad_jww_codec::Writer::default();
        writer.u16(1);
        writer.u16(0xffff);
        writer.u16(600);
        writer.u16(8);
        writer.raw(b"CDataSen");
        writer.u32(0);
        writer.u8(1);
        writer.u16(2);
        for _ in 0..4 {
            writer.u16(0);
        }
        for value in [0., 0., 10., 20.] {
            writer.f64(value);
        }
        writer.u16(0);
        data.extend(writer.into_bytes());
        let input = temp.path().join("part.jws");
        fs::write(&input, &data).unwrap();
        let part_path = temp.path().join("part.cadpart.json");
        copy_part(
            &input,
            &part_path,
            &temp.path().join("part-report.json"),
            100.,
        )
        .unwrap();
        let part = cad_toolkit::clipboard::read_document(&part_path).unwrap();
        assert_eq!(part.entities.len(), 1);
        assert!(matches!(
            part.entities[0],
            cad_model::Entity::Line {
                p2: [1000., 2000.],
                ..
            }
        ));
        let out = temp.path().join("project");
        let report = temp.path().join("import.json");
        import(&input, &out, &report, 100.).unwrap();
        assert!(cad_check::check_project(&out).is_ok());
        assert_eq!(fs::read(&input).unwrap(), data);
        assert!(!out.join("interop").exists());
        let value: serde_json::Value = serde_json::from_slice(&fs::read(report).unwrap()).unwrap();
        assert_eq!(value["status"], "converted");
        assert_eq!(value["check"]["status"], "ok");
        assert!(!value["exact_round_trip"].as_bool().unwrap());
        let source = cad_model::load_project(&out).unwrap();
        assert!(matches!(
            source.drawings[0].entities[0].entity,
            cad_model::Entity::Line {
                p2: [1000., 2000.],
                ..
            }
        ));
    }
    #[test]
    fn blocked_import_retains_report_and_protects_paths() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("bad.jws");
        fs::write(&input, b"unknown").unwrap();
        let out = temp.path().join("project");
        let report = temp.path().join("report.json");
        assert!(import(&input, &out, &report, 100.).is_err());
        assert!(!out.exists());
        let value: serde_json::Value = serde_json::from_slice(&fs::read(&report).unwrap()).unwrap();
        assert_eq!(value["status"], "blocked");
        assert!(!value["blockers"].as_array().unwrap().is_empty());
        let before = fs::read(&report).unwrap();
        assert!(import(&input, &out, &report, 100.).is_err());
        assert_eq!(fs::read(&report).unwrap(), before);
        assert!(import(&input, &out, &out.join("report.json"), 100.).is_err());
        assert!(!out.exists());
        fs::create_dir(&out).unwrap();
        fs::write(out.join("keep"), b"keep").unwrap();
        assert!(import(&input, &out, &temp.path().join("other.json"), 100.).is_err());
        assert_eq!(fs::read(out.join("keep")).unwrap(), b"keep");
    }
}
