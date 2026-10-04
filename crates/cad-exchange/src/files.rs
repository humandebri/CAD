//! Checked, report-first file exchange shared by CLI and Desktop.
use crate::dxf_exchange::{self, ExchangeError, ExchangeReport, Result};
use serde::Serialize;
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Serialize)]
pub struct FileExchangeReport {
    pub schema_version: String,
    pub operation: String,
    /// Conversion readiness only. The saved report precedes publication and does
    /// not attest that the destination was published successfully.
    pub status: String,
    pub output_path: String,
    pub report_path: String,
    pub source: Option<cad_git::SnapshotIdentity>,
    pub exchange: ExchangeReport,
}

fn err(error: impl ToString) -> ExchangeError {
    ExchangeError::Invalid(error.to_string())
}

fn destination(path: &Path) -> Result<PathBuf> {
    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir().map_err(err)?.join(path)
    };
    let mut resolved = PathBuf::new();
    for part in absolute.components() {
        match part {
            Component::ParentDir => {
                resolved.pop();
            }
            Component::CurDir => {}
            _ => resolved.push(part),
        }
        match fs::symlink_metadata(&resolved) {
            Ok(_) => resolved = fs::canonicalize(&resolved).map_err(err)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(err(e)),
        }
    }
    Ok(resolved)
}

fn unused(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(err("Exchange output already exists; choose a new path")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(err(e)),
    }
}

/// Resolve and protect a new user artifact destination. Publication must still
/// use create-new semantics to reject races after this read-only check.
pub fn new_artifact_path(project: &Path, output: &Path) -> Result<PathBuf> {
    let output = artifact_path(project, output)?;
    unused(&output)?;
    Ok(output)
}

/// Resolve a report/artifact path and protect all ancestor CAD projects. Existing
/// ordinary reports may be replaced by the caller; protected data never may.
pub fn artifact_path(project: &Path, output: &Path) -> Result<PathBuf> {
    let output = destination(output)?;
    if output.components().any(|part| {
        matches!(
            part.as_os_str().to_str(),
            Some(".cad-history" | ".cad-recovery" | ".cad-transactions")
        )
    }) {
        return Err(err(
            "Artifact output cannot be inside history or recovery metadata",
        ));
    }
    guard_project_path(project, &output)?;
    protect_ancestor_projects(&output)?;
    Ok(output)
}

fn guard_project_path(root: &Path, output: &Path) -> Result<()> {
    let root = fs::canonicalize(root).map_err(err)?;
    if output.strip_prefix(&root).is_ok_and(|p| {
        cad_model::classify_project_source_path(p).is_some()
            || matches!(
                p.components().next().and_then(|p| p.as_os_str().to_str()),
                Some(
                    "rules"
                        | "drawings"
                        | "blocks"
                        | "comments"
                        | "interop"
                        | ".cad-history"
                        | ".git"
                )
            )
            || p.starts_with("build/.cad-recovery")
            || p.starts_with("build/.cad-transactions")
            || p.starts_with("build/.cad-history")
    }) || (cad_git::repository_root(&root).is_ok()
        && cad_git::metadata_directories(&root)
            .map_err(err)?
            .iter()
            .any(|p| output.starts_with(p)))
    {
        return Err(err(
            "Exchange output cannot be inside canonical source, provenance, recovery, history or Git metadata",
        ));
    }
    Ok(())
}

fn protect_ancestor_projects(output: &Path) -> Result<()> {
    if output.components().any(|p| p.as_os_str() == ".git") {
        return Err(err("Exchange output cannot be inside Git metadata"));
    }
    if let Some(existing) = output.ancestors().skip(1).find(|p| p.is_dir())
        && let Ok(metadata) = cad_git::metadata_directories(existing)
        && metadata.iter().any(|p| output.starts_with(p))
    {
        return Err(err("Exchange output cannot be inside Git metadata"));
    }
    for ancestor in output.ancestors().skip(1) {
        if ancestor.join("cad.project.toml").is_file() {
            guard_project_path(ancestor, output)?;
        }
    }
    Ok(())
}

fn targets(output: &Path, report: &Path) -> Result<(PathBuf, Option<PathBuf>)> {
    let output = destination(output)?;
    unused(&output)?;
    protect_ancestor_projects(&output)?;
    let report = if report == Path::new("-") {
        None
    } else {
        let report = destination(report)?;
        if report == output || report.starts_with(&output) || output.starts_with(&report) {
            return Err(err(
                "Exchange output and report must use separate paths; import reports must be outside the new project",
            ));
        }
        unused(&report)?;
        protect_ancestor_projects(&report)?;
        Some(report)
    };
    Ok((output, report))
}

fn publish_report(report: &FileExchangeReport, path: Option<&Path>) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(report).map_err(err)?;
    if let Some(path) = path {
        fs::create_dir_all(
            path.parent()
                .ok_or_else(|| err("Report needs a parent directory"))?,
        )
        .map_err(err)?;
        cad_edit::atomic_publish(path, &bytes, false).map_err(err)?;
    } else {
        println!("{}", String::from_utf8_lossy(&bytes));
    }
    Ok(())
}

pub fn export_dxf(
    project: &Path,
    drawing: &str,
    output: &Path,
    report: &Path,
    strict: bool,
) -> Result<FileExchangeReport> {
    let (output, report_path) = targets(output, report)?;
    guard_project_path(project, &output)?;
    if let Some(report) = &report_path {
        guard_project_path(project, report)?;
    }
    let snapshot = cad_git::snapshot(project, &cad_git::Revision::Worktree).map_err(err)?;
    let result = dxf_exchange::export(&snapshot.source, drawing, strict)?;
    let report = FileExchangeReport {
        schema_version: "cad-file-exchange/1".into(),
        operation: "export_dxf".into(),
        status: if result.bytes.is_some() {
            "ready"
        } else {
            "blocked"
        }
        .into(),
        output_path: output.display().to_string(),
        report_path: report_path
            .as_ref()
            .map_or("-".into(), |p| p.display().to_string()),
        source: Some(snapshot.identity),
        exchange: result.report,
    };
    // The compatibility report survives blocked conversions and output-publication failures.
    publish_report(&report, report_path.as_deref())?;
    if let Some(bytes) = result.bytes {
        fs::create_dir_all(
            output
                .parent()
                .ok_or_else(|| err("Output needs a parent directory"))?,
        )
        .map_err(err)?;
        cad_edit::atomic_publish(&output, &bytes, false).map_err(err)?;
    }
    Ok(report)
}

pub fn import_dxf(
    input: &Path,
    output: &Path,
    report: &Path,
    unit_mm: Option<f64>,
) -> Result<FileExchangeReport> {
    let (output, report_path) = targets(output, report)?;
    let bytes = fs::read(input).map_err(err)?;
    let result = dxf_exchange::import(
        &bytes,
        input
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("DXF import"),
        unit_mm,
    )?;
    let parent = output
        .parent()
        .ok_or_else(|| err("Import output needs a parent directory"))?;
    let candidate = if let Some(source) = &result.source {
        fs::create_dir_all(parent).map_err(err)?;
        let candidate = tempfile::Builder::new()
            .prefix(".cad-dxf-import-")
            .tempdir_in(parent)
            .map_err(err)?;
        for file in cad_model::source_manifest(&source.root)
            .map_err(err)?
            .into_iter()
            .filter(|f| f.exists)
        {
            let target = candidate.path().join(&file.relative_path);
            fs::create_dir_all(
                target
                    .parent()
                    .ok_or_else(|| err("Source needs a parent directory"))?,
            )
            .map_err(err)?;
            cad_edit::atomic_publish(
                &target,
                &fs::read(source.root.join(&file.relative_path)).map_err(err)?,
                false,
            )
            .map_err(err)?;
        }
        let check = cad_check::check_project(candidate.path());
        if !check.is_ok() {
            return Err(err(format!(
                "Checked DXF candidate failed validation: {check:?}"
            )));
        }
        Some(candidate)
    } else {
        None
    };
    let report = FileExchangeReport {
        schema_version: "cad-file-exchange/1".into(),
        operation: "import_dxf".into(),
        status: if candidate.is_some() {
            "ready"
        } else {
            "blocked"
        }
        .into(),
        output_path: output.display().to_string(),
        report_path: report_path
            .as_ref()
            .map_or("-".into(), |p| p.display().to_string()),
        source: None,
        exchange: result.report,
    };
    publish_report(&report, report_path.as_deref())?;
    if let Some(candidate) = candidate {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            candidate.path(),
            rustix::fs::CWD,
            &output,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(err)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn new_artifacts_protect_other_projects_and_resolved_aliases() {
        let temp = tempfile::tempdir().unwrap();
        let other = temp.path().join("other");
        fs::create_dir(&other).unwrap();
        fs::write(other.join("cad.project.toml"), b"marker").unwrap();
        for path in [
            "rules/new.json",
            "drawings/new.json",
            "blocks/new.json",
            "comments/new.json",
            "interop/new.json",
            "build/.cad-history/new.json",
            "build/.cad-recovery/new.json",
            "build/.cad-transactions/new.json",
        ] {
            assert!(
                new_artifact_path(temp.path(), &other.join(path)).is_err(),
                "{path}"
            );
        }
        assert!(new_artifact_path(temp.path(), &other.join("build/part.json")).is_ok());
        assert!(new_artifact_path(temp.path(), &temp.path().join(".git/new.json")).is_err());
        #[cfg(unix)]
        {
            fs::create_dir(other.join("rules")).unwrap();
            let alias = temp.path().join("alias");
            std::os::unix::fs::symlink(other.join("rules"), &alias).unwrap();
            assert!(new_artifact_path(temp.path(), &alias.join("new.json")).is_err());
        }
        assert_eq!(fs::read(other.join("cad.project.toml")).unwrap(), b"marker");
    }
    #[test]
    fn reports_survive_blocked_exports_and_outputs_cannot_replace_project_data() {
        let temp = tempfile::tempdir().unwrap();
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let source = cad_git::snapshot_files(&fixture, &cad_git::Revision::Worktree).unwrap();
        let root = temp.path().join("project");
        fs::create_dir(&root).unwrap();
        for file in source.identity.source_manifest.iter().filter(|f| f.exists) {
            let target = root.join(&file.relative_path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(source.root.join(&file.relative_path), target).unwrap();
        }
        let before = cad_model::source_manifest(&root).unwrap();
        for dir in [
            "rules",
            "drawings",
            "blocks",
            "comments",
            "interop",
            ".cad-history",
            "build/.cad-recovery",
            "build/.cad-transactions",
            "build/.cad-history",
        ] {
            let output = root.join(dir).join("new.dxf");
            assert!(
                export_dxf(
                    &root,
                    "plan_1f",
                    &output,
                    &temp.path().join("guard.json"),
                    false
                )
                .is_err()
            );
            assert!(!output.exists());
        }
        let output = temp.path().join("blocked.dxf");
        let report = temp.path().join("blocked.json");
        let blocked = export_dxf(&root, "plan_1f", &output, &report, true).unwrap();
        assert_eq!(blocked.status, "blocked");
        assert!(!output.exists());
        assert!(report.is_file());
        assert!(!blocked.exchange.warnings.is_empty());
        assert_eq!(cad_model::source_manifest(&root).unwrap(), before);
        #[cfg(unix)]
        {
            let alias = temp.path().join("alias");
            std::os::unix::fs::symlink(root.join("rules"), &alias).unwrap();
            assert!(
                export_dxf(
                    &root,
                    "plan_1f",
                    &alias.join("alias.dxf"),
                    &temp.path().join("alias.json"),
                    false
                )
                .is_err()
            );
            let dangling = temp.path().join("dangling");
            std::os::unix::fs::symlink(temp.path().join("absent"), &dangling).unwrap();
            assert!(
                export_dxf(
                    &root,
                    "plan_1f",
                    &dangling,
                    &temp.path().join("dangling.json"),
                    false
                )
                .is_err()
            );
        }
    }
    #[test]
    fn import_publishes_only_after_a_new_external_report_can_be_saved() {
        let temp = tempfile::tempdir().unwrap();
        let input = temp.path().join("input.dxf");
        fs::write(&input, "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nLINE\n8\n0\n10\n0\n20\n0\n11\n100\n21\n200\n0\nENDSEC\n0\nEOF\n").unwrap();
        let output = temp.path().join("new-project");
        let report = temp.path().join("import.json");
        assert!(import_dxf(&input, &output, &output.join("report.json"), None).is_err());
        assert!(!output.exists());
        fs::write(&report, "keep").unwrap();
        assert!(import_dxf(&input, &output, &report, None).is_err());
        assert_eq!(fs::read(&report).unwrap(), b"keep");
        assert!(!output.exists());
        let report = temp.path().join("good.json");
        let result = import_dxf(&input, &output, &report, None).unwrap();
        assert_eq!(result.status, "ready");
        assert!(cad_check::check_project(&output).is_ok());
        assert!(matches!(
            cad_model::load_project(&output).unwrap().drawings[0].entities[0].entity,
            cad_model::Entity::Line {
                p2: [100., 200.],
                ..
            }
        ));
        assert!(report.is_file());
        assert!(import_dxf(&input, &output, &temp.path().join("second.json"), None).is_err());
        let metadata_output = temp.path().join(".git/unsafe-project");
        assert!(
            import_dxf(
                &input,
                &metadata_output,
                &temp.path().join("unsafe.json"),
                None
            )
            .is_err()
        );
        assert!(!metadata_output.exists());
        let bad_input = temp.path().join("bad.dxf");
        fs::write(&bad_input, "broken").unwrap();
        let blocked = temp.path().join("blocked");
        let blocked_report = temp.path().join("blocked.json");
        assert_eq!(
            import_dxf(&bad_input, &blocked, &blocked_report, None)
                .unwrap()
                .status,
            "blocked"
        );
        assert!(blocked_report.is_file());
        assert!(!blocked.exists());
    }
}
