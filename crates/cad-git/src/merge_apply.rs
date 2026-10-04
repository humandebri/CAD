//! Review and apply clean field merges to existing canonical working files.
//! This is a CAD source edit, not a Git branch merge or index mutation.
use crate::{
    GitError, Result, Revision, SnapshotIdentity, git_text, head_oid, merge, repository_root,
    snapshot, source_files,
};
use cad_model::ProjectSourceKind;
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Serialize)]
pub struct MergeApplyReport {
    pub schema_version: String,
    pub plan_hash: String,
    pub status: String,
    pub drawing: String,
    pub merge: merge::MergeReport,
    pub worktree: SnapshotIdentity,
    pub changed_files: Vec<String>,
    pub blockers: Vec<String>,
    pub cad_check: cad_check::CheckReport,
    pub diff: Option<cad_diff::DiffReport>,
    pub history_id: Option<String>,
}

pub struct MergeApplyPlan {
    pub report: MergeApplyReport,
    pub preview_svg: String,
    project: PathBuf,
    repo: PathBuf,
    index_path: PathBuf,
    index_bytes: Option<Vec<u8>>,
    head: Option<String>,
    branch: String,
    original_files: BTreeMap<PathBuf, Vec<u8>>,
    request: cad_edit::source_replacements::SourceReplacementRequest,
    reviewed_report: Vec<u8>,
    reviewed_svg: String,
}
fn invalid(error: impl ToString) -> GitError {
    GitError::Invalid(error.to_string())
}
fn index_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
fn normal_state(repo: &Path) -> Result<()> {
    for state in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-apply",
        "rebase-merge",
        "sequencer",
    ] {
        let path = git_text(
            repo,
            &["rev-parse", "--path-format=absolute", "--git-path", state],
        )?;
        if Path::new(&path).exists() {
            return Err(invalid(
                "Finish the active Git operation before applying a CAD merge candidate",
            ));
        }
    }
    Ok(())
}

pub fn plan(project: &Path, drawing: &str, base: &str, theirs: &str) -> Result<MergeApplyPlan> {
    if !matches!(Revision::parse(base), Revision::Commit(_))
        || !matches!(Revision::parse(theirs), Revision::Commit(_))
    {
        return Err(invalid(
            "Base and incoming revisions must identify Git commits; ours is always the worktree",
        ));
    }
    let project = fs::canonicalize(project)?;
    let repo = repository_root(&project)?;
    normal_state(&repo)?;
    let index_path = PathBuf::from(git_text(
        &repo,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    )?);
    let index_bytes = index_bytes(&index_path)?;
    let head = head_oid(&repo);
    let branch = git_text(&repo, &["symbolic-ref", "--quiet", "HEAD"]).unwrap_or_default();
    let base = snapshot(&project, &Revision::parse(base))?;
    let ours = snapshot(&project, &Revision::Worktree)?;
    let theirs = snapshot(&project, &Revision::parse(theirs))?;
    for input in [&base, &ours, &theirs] {
        if !cad_check::check_loaded_project(&input.source).is_ok() {
            return Err(invalid(format!(
                "merge input {} failed CAD validation",
                input.identity.revision
            )));
        }
    }
    if !ours.source.drawings.iter().any(|item| item.name == drawing) {
        return Err(invalid("Active drawing is missing from the worktree"));
    }
    let original_files: BTreeMap<_, _> = source_files(&ours.source.root)?.into_iter().collect();
    let merged = merge::merge(&base, &ours, &theirs)?;
    let names: BTreeSet<String> = merged
        .files
        .keys()
        .cloned()
        .chain(
            original_files
                .keys()
                .map(|path| path.to_string_lossy().into_owned()),
        )
        .collect();
    let changed_files: Vec<_> = names
        .into_iter()
        .filter(|path| merged.files.get(path) != original_files.get(Path::new(path)))
        .collect();
    let mut blockers = vec![];
    if !merged.report.conflicts.is_empty() {
        blockers.push(format!(
            "{} merge conflicts require resolution",
            merged.report.conflicts.len()
        ));
    }
    if changed_files.is_empty() {
        blockers.push("Candidate has no source changes".into());
    }
    let mut replacements = BTreeMap::new();
    for path in &changed_files {
        if !matches!(cad_model::classify_project_source_path(Path::new(path)), Some(kind) if kind != ProjectSourceKind::Comment)
        {
            blockers.push(format!("Protected comments or provenance changed: {path}"));
        } else if !original_files.contains_key(Path::new(path)) || !merged.files.contains_key(path)
        {
            blockers.push(format!(
                "File creation/deletion requires a separate project migration: {path}"
            ));
        } else {
            replacements.insert(path.clone(), merged.files[path].clone());
        }
    }
    if let Some(state) = cad_model::jww_project_compatibility(&ours.source.root)?
        && !(state.state == cad_model::JwwCompatibilityState::EditableLossless
            && state.original_verified
            && state.edit_capability == cad_model::JwwEditCapability::MappedV600)
    {
        blockers.push("JWW source has no verified editable record mapping".into());
    }
    let candidate = tempfile::tempdir()?;
    for (path, bytes) in &merged.files {
        let target = candidate.path().join(path);
        fs::create_dir_all(target.parent().expect("candidate source parent"))?;
        fs::write(target, bytes)?;
    }
    let cad_check = cad_check::check_project(candidate.path());
    if !cad_check.is_ok() {
        blockers.push("Merged project failed CAD validation".into());
    }
    let loaded = cad_model::load_project(candidate.path()).ok();
    let diff = loaded
        .as_ref()
        .map(|source| cad_diff::diff_projects(&ours.source, source));
    let preview_svg = loaded
        .as_ref()
        .map(|source| {
            cad_diff::render_diff_svg(
                &ours.source,
                source,
                &cad_diff::diff_selected_drawing(&ours.source, source, Some(drawing)),
            )
        })
        .unwrap_or_default();
    let request = cad_edit::source_replacements::SourceReplacementRequest {
        drawing: drawing.into(),
        expected_files: ours.identity.source_manifest.clone(),
        files: replacements,
    };
    let mut report = MergeApplyReport {
        schema_version: "cad-merge-apply/1".into(),
        plan_hash: String::new(),
        status: if blockers.is_empty() {
            "ready"
        } else {
            "blocked"
        }
        .into(),
        drawing: drawing.into(),
        merge: merged.report,
        worktree: ours.identity,
        changed_files,
        blockers,
        cad_check,
        diff,
        history_id: None,
    };
    let mut hash = blake3::Hasher::new();
    let context = serde_json::to_vec(&(&index_bytes, &head, &branch)).map_err(invalid)?;
    for part in [
        context.as_slice(),
        serde_json::to_vec(&report).map_err(invalid)?.as_slice(),
        preview_svg.as_bytes(),
    ] {
        hash.update(&(part.len() as u64).to_le_bytes());
        hash.update(part);
    }
    report.plan_hash = hash.finalize().to_hex().to_string();
    let reviewed_report = serde_json::to_vec(&report).map_err(invalid)?;
    let result = MergeApplyPlan {
        report,
        reviewed_svg: preview_svg.clone(),
        preview_svg,
        project,
        repo,
        index_path,
        index_bytes,
        head,
        branch,
        original_files,
        request,
        reviewed_report,
    };
    verify_context(&result)?;
    Ok(result)
}

fn verify_context(plan: &MergeApplyPlan) -> Result<()> {
    verify_metadata(plan)?;
    if source_files(&plan.project)?
        .into_iter()
        .collect::<BTreeMap<_, _>>()
        != plan.original_files
    {
        return Err(invalid("Source changed; review a new merge candidate"));
    }
    Ok(())
}

fn verify_metadata(plan: &MergeApplyPlan) -> Result<()> {
    normal_state(&plan.repo)?;
    if index_bytes(&plan.index_path)? != plan.index_bytes
        || head_oid(&plan.repo) != plan.head
        || git_text(&plan.repo, &["symbolic-ref", "--quiet", "HEAD"]).unwrap_or_default()
            != plan.branch
    {
        return Err(invalid("Git context changed; review a new merge candidate"));
    }
    let provenance = |files: BTreeMap<PathBuf, Vec<u8>>| {
        files
            .into_iter()
            .filter(|(path, _)| cad_model::classify_project_source_path(path).is_none())
            .collect::<BTreeMap<_, _>>()
    };
    if provenance(source_files(&plan.project)?.into_iter().collect())
        != provenance(plan.original_files.clone())
    {
        return Err(invalid(
            "JWW provenance changed; review a new merge candidate",
        ));
    }
    Ok(())
}

pub fn apply(plan: &mut MergeApplyPlan, expected_hash: &str) -> Result<()> {
    if plan.report.plan_hash != expected_hash
        || serde_json::to_vec(&plan.report).map_err(invalid)? != plan.reviewed_report
        || plan.preview_svg != plan.reviewed_svg
    {
        return Err(invalid("Merge review does not match the sealed candidate"));
    }
    if plan.report.status != "ready" {
        return Err(invalid("Blocked merge candidate cannot be applied"));
    }
    verify_context(plan)?;
    let result =
        cad_edit::source_replacements::apply_with_guard(&plan.project, &plan.request, || {
            verify_metadata(plan)
                .map_err(|error| cad_edit::EditError::InvalidEntity(error.to_string()))
        })
        .map_err(invalid)?;
    plan.report.history_id = result.history_id;
    plan.report.status = "applied".into();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (tempfile::TempDir, PathBuf) {
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
        let root = PathBuf::from(created.project_path);
        let base = json!({"schema_version":"0.3","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0.,0.],"p2":[100.,0.]});
        fs::write(
            root.join("drawings/plan/entities.ndjson"),
            format!("{base}\n"),
        )
        .unwrap();
        for args in [
            vec!["init"],
            vec!["config", "user.name", "Merge Test"],
            vec!["config", "user.email", "merge@example.invalid"],
            vec!["add", "."],
            vec!["commit", "-m", "base"],
        ] {
            git_text(&root, &args).unwrap();
        }
        let mut incoming = base.clone();
        incoming["p1"] = json!([0., 20.]);
        fs::write(
            root.join("drawings/plan/entities.ndjson"),
            format!("{incoming}\n"),
        )
        .unwrap();
        git_text(&root, &["add", "."]).unwrap();
        git_text(&root, &["commit", "-m", "incoming"]).unwrap();
        let mut ours = base;
        ours["p2"] = json!([200., 0.]);
        fs::write(
            root.join("drawings/plan/entities.ndjson"),
            format!("{ours}\n"),
        )
        .unwrap();
        (temp, root)
    }
    #[test]
    fn clean_merge_is_read_only_until_reviewed_and_undo_restores_ours() {
        let (_temp, root) = fixture();
        let path = root.join("drawings/plan/entities.ndjson");
        let before = fs::read(&path).unwrap();
        let index = fs::read(root.join(".git/index")).unwrap();
        let head = head_oid(&root);
        let mut candidate = plan(&root, "plan", "HEAD~1", "HEAD").unwrap();
        assert_eq!(candidate.report.status, "ready");
        assert_eq!(
            candidate.report.plan_hash,
            plan(&root, "plan", "HEAD~1", "HEAD")
                .unwrap()
                .report
                .plan_hash
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(apply(&mut candidate, "wrong").is_err());
        let hash = candidate.report.plan_hash.clone();
        apply(&mut candidate, &hash).unwrap();
        let merged: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(merged["p1"], json!([0., 20.]));
        assert_eq!(merged["p2"], json!([200., 0.]));
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
        assert_eq!(head_oid(&root), head);
        assert!(cad_check::check_project(&root).is_ok());
        assert_eq!(candidate.report.status, "applied");
        assert!(apply(&mut candidate, &hash).is_err());
        let history = cad_edit::list_drawing_history(&root, "plan").unwrap();
        cad_edit::undo_drawing_edit(
            &root,
            &cad_edit::DrawingHistoryRequest {
                drawing: "plan".into(),
                expected_files: history.current_files,
            },
        )
        .unwrap();
        assert_eq!(fs::read(path).unwrap(), before);
    }
    #[test]
    fn conflicts_changed_context_and_tampered_reports_cannot_publish() {
        let (_temp, root) = fixture();
        let path = root.join("drawings/plan/entities.ndjson");
        let before = fs::read(&path).unwrap();
        let mut candidate = plan(&root, "plan", "HEAD~1", "HEAD").unwrap();
        let hash = candidate.report.plan_hash.clone();
        candidate.report.changed_files.clear();
        assert!(apply(&mut candidate, &hash).is_err());
        let mut candidate = plan(&root, "plan", "HEAD~1", "HEAD").unwrap();
        let hash = candidate.report.plan_hash.clone();
        fs::write(
            root.join("rules/layers.toml"),
            fs::read(root.join("rules/layers.toml"))
                .unwrap()
                .into_iter()
                .chain(b"\n# concurrent\n".iter().copied())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(apply(&mut candidate, &hash).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        let mut ours: serde_json::Value = serde_json::from_slice(&before).unwrap();
        ours["p1"] = json!([0., 40.]);
        fs::write(&path, format!("{ours}\n")).unwrap();
        let mut blocked = plan(&root, "plan", "HEAD~1", "HEAD").unwrap();
        assert_eq!(blocked.report.status, "blocked");
        assert!(!blocked.report.merge.conflicts.is_empty());
        let hash = blocked.report.plan_hash.clone();
        assert!(apply(&mut blocked, &hash).is_err());
        assert_eq!(fs::read(&path).unwrap(), format!("{ours}\n").into_bytes());
    }
    #[test]
    fn protected_comment_and_file_topology_changes_remain_reviewable_but_blocked() {
        let (_temp, root) = fixture();
        fs::create_dir_all(root.join("comments")).unwrap();
        fs::write(
            root.join("comments/plan.ndjson"),
            "{\"id\":\"external-comment\"}\n",
        )
        .unwrap();
        git_text(&root, &["add", "comments/plan.ndjson"]).unwrap();
        git_text(&root, &["commit", "-m", "comment"]).unwrap();
        fs::remove_file(root.join("comments/plan.ndjson")).unwrap();
        let candidate = plan(&root, "plan", "HEAD~2", "HEAD").unwrap();
        assert_eq!(candidate.report.status, "blocked");
        assert!(
            candidate
                .report
                .blockers
                .iter()
                .any(|message| message.contains("Protected"))
        );
        assert!(plan(&root, "plan", "worktree", "HEAD").is_err());
        fs::write(root.join("rules/new.toml"), "# extra rule\n").unwrap();
        git_text(&root, &["add", "rules/new.toml"]).unwrap();
        git_text(&root, &["commit", "-m", "extra file"]).unwrap();
        fs::remove_file(root.join("rules/new.toml")).unwrap();
        let candidate = plan(&root, "plan", "HEAD~3", "HEAD").unwrap();
        assert!(
            candidate
                .report
                .blockers
                .iter()
                .any(|message| message.contains("creation/deletion"))
        );
    }
}
