use miette::{IntoDiagnostic, Result, miette};
use serde_json::json;
use std::fs;
use std::path::Path;

pub fn export(
    project: &Path,
    base: &str,
    ours: &str,
    theirs: &str,
    directory: &Path,
) -> Result<()> {
    let base = cad_git::snapshot(project, &cad_git::Revision::parse(base)).into_diagnostic()?;
    let ours = cad_git::snapshot(project, &cad_git::Revision::parse(ours)).into_diagnostic()?;
    let theirs = cad_git::snapshot(project, &cad_git::Revision::parse(theirs)).into_diagnostic()?;
    for snapshot in [&base, &ours, &theirs] {
        let check = cad_check::check_project(&snapshot.source.root);
        if !check.is_ok() {
            return Err(miette!(
                "merge input {} failed CAD validation: {}",
                snapshot.identity.revision,
                serde_json::to_string(&check).into_diagnostic()?
            ));
        }
    }
    let candidate = cad_git::merge::merge(&base, &ours, &theirs).into_diagnostic()?;
    if let Some(parent) = directory.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).into_diagnostic()?;
    }
    fs::create_dir(directory).into_diagnostic()?;
    let mut files = Vec::new();
    for (relative, bytes) in &candidate.files {
        let path = directory.join(relative);
        fs::create_dir_all(path.parent().expect("source parent")).into_diagnostic()?;
        cad_edit::atomic_publish(&path, bytes, false).into_diagnostic()?;
        files.push(json!({"path":relative,"bytes":bytes.len(),"blake3":blake3::hash(bytes).to_hex().to_string()}));
    }
    let check = cad_check::check_project(directory);
    let clean = candidate.report.conflicts.is_empty() && check.is_ok();
    let report = json!({"status":if clean {"clean_candidate"}else{"blocked_candidate"},"merge":candidate.report,"cad_check":check,"candidate_files":files});
    let build = directory.join("build");
    fs::create_dir(&build).into_diagnostic()?;
    cad_edit::atomic_publish(
        &build.join("merge-report.json"),
        &serde_json::to_vec_pretty(&report).into_diagnostic()?,
        false,
    )
    .into_diagnostic()?;
    if check.is_ok() {
        let merged = cad_model::load_project(directory).into_diagnostic()?;
        let diff = cad_diff::diff_projects(&ours.source, &merged);
        cad_edit::atomic_publish(
            &build.join("merge.diff.json"),
            &serde_json::to_vec_pretty(&diff).into_diagnostic()?,
            false,
        )
        .into_diagnostic()?;
        for (index, drawing) in merged.drawings.iter().enumerate() {
            let diff = cad_diff::diff_selected_drawing(&ours.source, &merged, Some(&drawing.name));
            let svg = cad_diff::render_diff_svg(&ours.source, &merged, &diff);
            cad_edit::atomic_publish(
                &build.join(format!("{:03}.merge.diff.svg", index + 1)),
                svg.as_bytes(),
                false,
            )
            .into_diagnostic()?;
        }
    }
    if !clean {
        return Err(miette!(
            "merge requires conflict or checker resolution; candidate and report retained at {}",
            directory.display()
        ));
    }
    println!(
        "Merge candidate validated at {}; original worktree and index are unchanged",
        directory.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn git(root: &Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[test]
    fn publishes_checked_candidate_and_retains_conflicts_without_changing_index() {
        let temp = tempfile::tempdir().unwrap();
        let created = cad_edit::create_project(&cad_edit::ProjectTemplateRequest {
            parent_dir: temp.path().display().to_string(),
            folder_name: "repo".into(),
            project_name: "merge".into(),
            drawing: "plan".into(),
            paper: "A4".into(),
            orientation: cad_model::SheetOrientation::Landscape,
            scale_denominator: 50,
        })
        .unwrap();
        let root = Path::new(&created.project_path);
        let path = root.join("drawings/plan/entities.ndjson");
        let base = json!({"schema_version":"0.3","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0.0,0.0],"p2":[100.0,0.0]});
        let write = |entity: &serde_json::Value| fs::write(&path, format!("{entity}\n")).unwrap();
        write(&base);
        git(root, &["init"]);
        git(root, &["config", "user.email", "cad@example.invalid"]);
        git(root, &["config", "user.name", "CAD Test"]);
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "base"]);
        let mut theirs = base.clone();
        theirs["p1"] = json!([0.0, 20.0]);
        write(&theirs);
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "theirs"]);
        let mut ours = base.clone();
        ours["p2"] = json!([200.0, 0.0]);
        write(&ours);
        let index = fs::read(root.join(".git/index")).unwrap();
        let bytes = fs::read(&path).unwrap();
        let output = temp.path().join("clean");
        export(root, "HEAD~1", "worktree", "HEAD", &output).unwrap();
        let merged: serde_json::Value = serde_json::from_slice(
            &fs::read(output.join("drawings/plan/entities.ndjson")).unwrap(),
        )
        .unwrap();
        assert_eq!(merged["p1"], theirs["p1"]);
        assert_eq!(merged["p2"], ours["p2"]);
        assert!(cad_check::check_project(&output).is_ok());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
        ours["p1"] = json!([0.0, 40.0]);
        write(&ours);
        let blocked = temp.path().join("blocked");
        assert!(export(root, "HEAD~1", "worktree", "HEAD", &blocked).is_err());
        let report: serde_json::Value =
            serde_json::from_slice(&fs::read(blocked.join("build/merge-report.json")).unwrap())
                .unwrap();
        assert_eq!(report["status"], "blocked_candidate");
        assert_eq!(
            report["merge"]["conflicts"][0]["field"],
            "/ent_01JZ0000000000000000000000/p1"
        );
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    }
}
