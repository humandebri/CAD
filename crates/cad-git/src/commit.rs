//! Commit the reviewed complete index without rewriting index or working files.
//! Hooks/signing are intentionally excluded and disclosed in every plan.
use crate::{
    GitError, Result, Revision, SnapshotIdentity, git_text, head_oid, repository_root, snapshot,
};
use serde::Serialize;
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub email: String,
}
#[derive(Debug, Serialize)]
pub struct CommitFile {
    pub status: String,
    pub path: String,
}
#[derive(Debug, Serialize)]
pub struct CommitReport {
    pub schema_version: String,
    pub plan_hash: String,
    pub repository: String,
    pub branch: String,
    pub parent_oid: Option<String>,
    pub tree_oid: String,
    pub index_blake3: String,
    pub author: Identity,
    pub committer: Identity,
    pub message: String,
    pub files: Vec<CommitFile>,
    pub patch: String,
    pub patch_truncated: bool,
    pub cad_source: SnapshotIdentity,
    pub cad_check: cad_check::CheckReport,
    pub hooks_run: bool,
    pub signed: bool,
    pub applied: bool,
    pub commit_oid: Option<String>,
}
pub struct CommitPlan {
    pub report: CommitReport,
    repo: PathBuf,
    index_path: PathBuf,
    index_bytes: Vec<u8>,
    temporary: tempfile::TempDir,
}
fn invalid(error: impl ToString) -> GitError {
    GitError::Invalid(error.to_string())
}
fn report_hash(report: &CommitReport) -> Result<String> {
    let mut value = serde_json::to_value(report).map_err(invalid)?;
    value["plan_hash"] = serde_json::json!("");
    Ok(blake3::hash(&serde_json::to_vec(&value).map_err(invalid)?)
        .to_hex()
        .to_string())
}
fn identity(raw: &str) -> Result<Identity> {
    let end = raw
        .rfind('>')
        .ok_or_else(|| invalid("Git identity has no email"))?;
    let start = raw[..end]
        .rfind('<')
        .ok_or_else(|| invalid("Git identity has no email"))?;
    let name = raw[..start].trim();
    let email = &raw[start + 1..end];
    if name.is_empty() || email.is_empty() {
        return Err(invalid(
            "Configure the Git author and committer before committing",
        ));
    }
    Ok(Identity {
        name: name.into(),
        email: email.into(),
    })
}
fn git(
    repo: &Path,
    hooks: &Path,
    index: Option<&Path>,
    args: &[&str],
    input: Option<&[u8]>,
    identities: Option<(&Identity, &Identity)>,
) -> Result<Vec<u8>> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(repo)
        .arg("-c")
        .arg(format!("core.hooksPath={}", hooks.display()))
        .arg("-c")
        .arg("commit.gpgsign=false")
        .args(args);
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    if let Some((author, committer)) = identities {
        command
            .env("GIT_AUTHOR_NAME", &author.name)
            .env("GIT_AUTHOR_EMAIL", &author.email)
            .env("GIT_COMMITTER_NAME", &committer.name)
            .env("GIT_COMMITTER_EMAIL", &committer.email);
    }
    command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn()?;
    let input_error = if let Some(input) = input {
        child
            .stdin
            .take()
            .ok_or_else(|| invalid("Git input unavailable"))?
            .write_all(input)
            .err()
    } else {
        None
    };
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(invalid(format!(
            "Git commit operation failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    if let Some(error) = input_error {
        return Err(error.into());
    }
    Ok(output.stdout)
}
fn text(bytes: Vec<u8>) -> Result<String> {
    String::from_utf8(bytes)
        .map(|s| s.trim_end().to_owned())
        .map_err(invalid)
}
fn ensure_normal_state(repo: &Path) -> Result<()> {
    for name in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "REBASE_HEAD",
        "rebase-merge",
        "rebase-apply",
        "sequencer",
    ] {
        let path = git_text(
            repo,
            &["rev-parse", "--path-format=absolute", "--git-path", name],
        )?;
        if fs::symlink_metadata(path).is_ok() {
            return Err(invalid(
                "Finish the active merge, rebase, revert or cherry-pick with Git before using reviewed commit",
            ));
        }
    }
    Ok(())
}
fn current_branch(repo: &Path) -> Result<String> {
    let branch = git_text(repo, &["symbolic-ref", "--quiet", "HEAD"]).map_err(|_| {
        invalid("Reviewed commit requires a checked-out branch; detached HEAD is unsupported")
    })?;
    if !branch.starts_with("refs/heads/") {
        return Err(invalid("HEAD must refer to a local branch"));
    }
    git_text(repo, &["check-ref-format", &branch])?;
    Ok(branch)
}

pub fn plan(project: &Path, message: &str) -> Result<CommitPlan> {
    let message = message.trim();
    if message.is_empty() || message.contains('\0') || message.len() > 65536 {
        return Err(invalid(
            "Commit message must be nonempty, contain no NUL, and fit within 64 KiB",
        ));
    }
    let project = fs::canonicalize(project)?;
    let repo = repository_root(&project)?;
    ensure_normal_state(&repo)?;
    let branch = current_branch(&repo)?;
    let parent = head_oid(&repo);
    let index_path = PathBuf::from(git_text(
        &repo,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    )?);
    let index_bytes = fs::read(&index_path)?;
    let temporary = tempfile::Builder::new()
        .prefix("cad-reviewed-commit-")
        .tempdir_in(
            index_path
                .parent()
                .ok_or_else(|| invalid("Index has no parent"))?,
        )?;
    let hooks = temporary.path().join("hooks");
    fs::create_dir(&hooks)?;
    let index = temporary.path().join("index");
    fs::write(&index, &index_bytes)?;
    let tree = text(git(
        &repo,
        &hooks,
        Some(&index),
        &["write-tree"],
        None,
        None,
    )?)?;
    let base = match &parent {
        Some(oid) => oid.clone(),
        None => text(git(&repo, &hooks, None, &["mktree"], Some(b""), None)?)?,
    };
    let author = identity(&git_text(&repo, &["var", "GIT_AUTHOR_IDENT"])?)?;
    let committer = identity(&git_text(&repo, &["var", "GIT_COMMITTER_IDENT"])?)?;
    let names = git(
        &repo,
        &hooks,
        None,
        &[
            "diff-tree",
            "--no-commit-id",
            "--no-renames",
            "--name-status",
            "-r",
            "-z",
            &base,
            &tree,
            "--",
        ],
        None,
        None,
    )?;
    let fields = names
        .split(|b| *b == 0)
        .filter(|f| !f.is_empty())
        .collect::<Vec<_>>();
    if fields.len() % 2 != 0 {
        return Err(invalid("Git returned invalid staged file metadata"));
    }
    let files = fields
        .chunks_exact(2)
        .map(|f| {
            Ok(CommitFile {
                status: String::from_utf8(f[0].to_vec()).map_err(invalid)?,
                path: String::from_utf8(f[1].to_vec()).map_err(invalid)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let bytes = git(
        &repo,
        &hooks,
        None,
        &[
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-renames",
            "--no-color",
            &base,
            &tree,
            "--",
        ],
        None,
        None,
    )?;
    let truncated = bytes.len() > 1048576 || std::str::from_utf8(&bytes).is_err();
    let patch = String::from_utf8_lossy(&bytes[..bytes.len().min(1048576)]).into_owned();
    let staged = snapshot(&project, &Revision::Index)?;
    let cad_check = cad_check::check_loaded_project(&staged.source);
    if fs::read(&index_path)? != index_bytes
        || current_branch(&repo)? != branch
        || head_oid(&repo) != parent
    {
        return Err(invalid(
            "Index or branch changed during commit preview; prepare a new plan",
        ));
    }
    let mut report = CommitReport {
        schema_version: "cad-git-commit/1".into(),
        plan_hash: String::new(),
        repository: repo.display().to_string(),
        branch,
        parent_oid: parent,
        tree_oid: tree,
        index_blake3: blake3::hash(&index_bytes).to_hex().to_string(),
        author,
        committer,
        message: message.into(),
        files,
        patch,
        patch_truncated: truncated,
        cad_source: staged.identity,
        cad_check,
        hooks_run: false,
        signed: false,
        applied: false,
        commit_oid: None,
    };
    report.plan_hash = report_hash(&report)?;
    Ok(CommitPlan {
        report,
        repo,
        index_path,
        index_bytes,
        temporary,
    })
}

pub fn apply(plan: &mut CommitPlan, expected_plan: &str) -> Result<()> {
    if expected_plan != plan.report.plan_hash || report_hash(&plan.report)? != expected_plan {
        return Err(invalid(
            "Commit plan changed; review the complete index again",
        ));
    }
    if plan.report.files.is_empty() || !plan.report.cad_check.is_ok() || plan.report.patch_truncated
    {
        return Err(invalid(
            "Commit requires a complete, nonempty preview and a valid staged CAD project",
        ));
    }
    let lock_path = plan.index_path.with_extension("lock");
    let lock = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)?;
    struct Lock {
        path: PathBuf,
        file: Option<fs::File>,
    }
    impl Drop for Lock {
        fn drop(&mut self) {
            drop(self.file.take());
            let _ = fs::remove_file(&self.path);
        }
    }
    let _guard = Lock {
        path: lock_path,
        file: Some(lock),
    };
    ensure_normal_state(&plan.repo)?;
    if fs::read(&plan.index_path)? != plan.index_bytes
        || current_branch(&plan.repo)? != plan.report.branch
        || head_oid(&plan.repo) != plan.report.parent_oid
    {
        return Err(invalid("Index or branch changed after commit preview"));
    }
    let hooks = plan.temporary.path().join("hooks");
    let mut args = vec![
        "commit-tree",
        plan.report.tree_oid.as_str(),
        "--no-gpg-sign",
        "-F",
        "-",
    ];
    if let Some(parent) = &plan.report.parent_oid {
        args.extend(["-p", parent.as_str()]);
    }
    let oid = text(git(
        &plan.repo,
        &hooks,
        None,
        &args,
        Some(format!("{}\n", plan.report.message).as_bytes()),
        Some((&plan.report.author, &plan.report.committer)),
    )?)?;
    publish_prepared_reference(plan, &oid).map_err(|e| {
        invalid(format!(
            "{e}; commit object {oid} exists; inspect the reviewed branch before retrying"
        ))
    })?;
    plan.report.applied = true;
    plan.report.commit_oid = Some(oid);
    Ok(())
}

// Git locks HEAD and its referent at prepare. Check the symbolic target while
// those locks are held, before commit; a same-OID checkout race must also abort.
// https://git-scm.com/docs/git-update-ref
fn publish_prepared_reference(plan: &CommitPlan, oid: &str) -> Result<()> {
    let hooks = plan.temporary.path().join("hooks");
    let child = Command::new("git")
        .arg("-C")
        .arg(&plan.repo)
        .arg("-c")
        .arg(format!("core.hooksPath={}", hooks.display()))
        .args([
            "update-ref",
            "--create-reflog",
            "-m",
            "CAD reviewed commit",
            "--stdin",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    struct Process(Option<std::process::Child>);
    impl Drop for Process {
        fn drop(&mut self) {
            if let Some(child) = &mut self.0 {
                // Closing stdin aborts an unfinished Git transaction and lets
                // Git release its ref locks. Killing it would strand those locks.
                drop(child.stdin.take());
                let _ = child.wait();
            }
        }
    }
    let mut process = Process(Some(child));
    let child = process.0.as_mut().unwrap();
    let mut input = child
        .stdin
        .take()
        .ok_or_else(|| invalid("Git transaction input unavailable"))?;
    let mut output = BufReader::new(
        child
            .stdout
            .take()
            .ok_or_else(|| invalid("Git transaction output unavailable"))?,
    );
    let parent = plan
        .report
        .parent_oid
        .clone()
        .unwrap_or_else(|| "0".repeat(oid.len()));
    input.write_all(format!("start\nupdate HEAD {oid} {parent}\nprepare\n").as_bytes())?;
    input.flush()?;
    for expected in ["start: ok", "prepare: ok"] {
        let mut line = String::new();
        output.read_line(&mut line)?;
        if line.trim_end() != expected {
            drop(input);
            let finished = process.0.take().unwrap().wait_with_output()?;
            return Err(invalid(format!(
                "Git could not prepare the reference transaction: {}",
                String::from_utf8_lossy(&finished.stderr).trim()
            )));
        }
    }
    // Both ref locks remain held until the transaction is explicitly committed.
    if current_branch(&plan.repo)? != plan.report.branch
        || fs::read(&plan.index_path)? != plan.index_bytes
    {
        return Err(invalid(
            "Index or checked-out branch changed before reference publication",
        ));
    }
    ensure_normal_state(&plan.repo)?;
    input.write_all(b"commit\n")?;
    drop(input);
    let finished = process.0.take().unwrap().wait_with_output()?;
    if !finished.status.success() {
        return Err(invalid(format!(
            "Git reference publication failed: {}",
            String::from_utf8_lossy(&finished.stderr).trim()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn setup(initial: bool) -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let created = cad_edit::create_project(&cad_edit::ProjectTemplateRequest {
            parent_dir: temp.path().display().to_string(),
            folder_name: "repo".into(),
            project_name: "commit".into(),
            drawing: "plan".into(),
            paper: "A3".into(),
            orientation: cad_model::SheetOrientation::Landscape,
            scale_denominator: 100,
        })
        .unwrap();
        let root = PathBuf::from(created.project_path);
        git_text(&root, &["init", "-b", "main"]).unwrap();
        git_text(&root, &["config", "user.name", "CAD Test"]).unwrap();
        git_text(&root, &["config", "user.email", "cad@example.invalid"]).unwrap();
        git_text(&root, &["add", "."]).unwrap();
        if initial {
            git_text(&root, &["commit", "-m", "baseline"]).unwrap();
        }
        (temp, root)
    }
    #[test]
    fn reviewed_index_includes_unrelated_staging_and_preserves_working_bytes_and_index() {
        let (_temp, root) = setup(true);
        fs::write(root.join("drawings/plan/entities.ndjson"),"{\"schema_version\":\"0.3\",\"id\":\"ent_01JZ0000000000000000000000\",\"type\":\"line\",\"layer\":\"0-1\",\"p1\":[0,0],\"p2\":[1234,5678]}\n").unwrap();
        fs::write(root.join("other.txt"), "already staged\n").unwrap();
        git_text(&root, &["add", "."]).unwrap();
        fs::write(root.join("other.txt"), "working content\n").unwrap();
        let head = head_oid(&root).unwrap();
        let index = fs::read(root.join(".git/index")).unwrap();
        let manifest = cad_model::source_manifest(&root).unwrap();
        let mut candidate = plan(&root, "寸法変更\n\n既存stageを含む").unwrap();
        assert!(candidate.report.files.iter().any(|f| f.path == "other.txt"));
        assert!(candidate.report.patch.contains("already staged"));
        assert!(!candidate.report.patch.contains("working content"));
        assert_eq!(head_oid(&root).as_deref(), Some(head.as_str()));
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for name in ["pre-commit", "commit-msg", "reference-transaction"] {
                let path = root.join(".git/hooks").join(name);
                fs::write(&path, "#!/bin/sh\necho ran > hook-ran\nexit 1\n").unwrap();
                fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        let expected = candidate.report.plan_hash.clone();
        apply(&mut candidate, &expected).unwrap();
        assert!(candidate.report.applied);
        let oid = candidate.report.commit_oid.as_ref().unwrap();
        assert_eq!(head_oid(&root).as_ref(), Some(oid));
        assert_eq!(
            git_text(&root, &["rev-parse", "HEAD^{tree}"]).unwrap(),
            candidate.report.tree_oid
        );
        assert_eq!(
            git_text(&root, &["log", "-1", "--format=%an <%ae>"]).unwrap(),
            "CAD Test <cad@example.invalid>"
        );
        assert_eq!(
            git_text(&root, &["log", "-1", "--format=%B"]).unwrap(),
            candidate.report.message
        );
        assert_eq!(
            git_text(&root, &["show", "HEAD:other.txt"]).unwrap(),
            "already staged"
        );
        assert_eq!(
            fs::read_to_string(root.join("other.txt")).unwrap(),
            "working content\n"
        );
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
        assert_eq!(cad_model::source_manifest(&root).unwrap(), manifest);
        assert!(!root.join("hook-ran").exists());
        assert!(!root.join(".git/index.lock").exists());
        assert!(
            git_text(&root, &["diff", "--cached", "--name-only"])
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn stale_index_branch_locks_and_active_git_operations_cannot_publish() {
        let (_temp, root) = setup(false);
        let mut candidate = plan(&root, "first").unwrap();
        let expected = candidate.report.plan_hash.clone();
        fs::write(root.join(".git/index.lock"), b"existing lock").unwrap();
        assert!(apply(&mut candidate, &expected).is_err());
        assert_eq!(
            fs::read(root.join(".git/index.lock")).unwrap(),
            b"existing lock"
        );
        fs::remove_file(root.join(".git/index.lock")).unwrap();
        fs::write(root.join("extra.txt"), "extra").unwrap();
        git_text(&root, &["add", "extra.txt"]).unwrap();
        let changed = fs::read(root.join(".git/index")).unwrap();
        assert!(apply(&mut candidate, &expected).is_err());
        assert!(head_oid(&root).is_none());
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), changed);
        let mut fresh = plan(&root, "first").unwrap();
        let hash = fresh.report.plan_hash.clone();
        apply(&mut fresh, &hash).unwrap();
        assert!(fresh.report.parent_oid.is_none());
        assert_eq!(
            git_text(&root, &["rev-list", "--count", "HEAD"]).unwrap(),
            "1"
        );
        fs::write(root.join("next.txt"), "next").unwrap();
        git_text(&root, &["add", "next.txt"]).unwrap();
        let mut branch_plan = plan(&root, "next").unwrap();
        let expected = branch_plan.report.plan_hash.clone();
        git_text(&root, &["switch", "-c", "other"]).unwrap();
        let before = head_oid(&root);
        assert!(apply(&mut branch_plan, &expected).is_err());
        assert_eq!(head_oid(&root), before);
        assert!(!root.join(".git/index.lock").exists());
        fs::write(root.join(".git/MERGE_HEAD"), before.as_ref().unwrap()).unwrap();
        assert!(plan(&root, "merge").is_err());
        fs::remove_file(root.join(".git/MERGE_HEAD")).unwrap();
        git_text(&root, &["switch", "--detach"]).unwrap();
        assert!(plan(&root, "detached").is_err());
    }
    #[test]
    fn empty_invalid_and_incomplete_previews_block_commit() {
        let (_temp, root) = setup(true);
        assert!(plan(&root, " ").is_err());
        let head = head_oid(&root);
        let mut empty = plan(&root, "empty").unwrap();
        let hash = empty.report.plan_hash.clone();
        assert!(apply(&mut empty, &hash).is_err());
        fs::write(root.join("large.txt"), "x".repeat(1100000)).unwrap();
        git_text(&root, &["add", "large.txt"]).unwrap();
        let mut large = plan(&root, "large").unwrap();
        assert!(large.report.patch_truncated);
        let hash = large.report.plan_hash.clone();
        assert!(apply(&mut large, &hash).is_err());
        assert_eq!(head_oid(&root), head);
        fs::write(root.join("drawings/plan/entities.ndjson"), "broken").unwrap();
        git_text(&root, &["add", "drawings/plan/entities.ndjson"]).unwrap();
        assert!(plan(&root, "invalid CAD").is_err());
        assert_eq!(head_oid(&root), head);
    }

    #[test]
    fn same_oid_checkout_race_aborts_prepared_transaction_and_releases_git_locks() {
        let (_temp, root) = setup(true);
        fs::write(root.join("extra.txt"), "new staged content").unwrap();
        git_text(&root, &["add", "extra.txt"]).unwrap();
        let candidate = plan(&root, "race").unwrap();
        let parent = candidate.report.parent_oid.as_ref().unwrap();
        let oid = git_text(
            &root,
            &[
                "commit-tree",
                &candidate.report.tree_oid,
                "-p",
                parent,
                "-m",
                "candidate",
            ],
        )
        .unwrap();
        git_text(&root, &["branch", "other", parent]).unwrap();
        git_text(&root, &["symbolic-ref", "HEAD", "refs/heads/other"]).unwrap();
        let index = fs::read(root.join(".git/index")).unwrap();
        assert!(publish_prepared_reference(&candidate, &oid).is_err());
        assert_eq!(git_text(&root, &["rev-parse", "main"]).unwrap(), *parent);
        assert_eq!(git_text(&root, &["rev-parse", "other"]).unwrap(), *parent);
        assert!(!root.join(".git/HEAD.lock").exists());
        assert!(!root.join(".git/refs/heads/other.lock").exists());
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    }

    #[test]
    fn altering_a_public_report_cannot_change_the_reviewed_commit() {
        let (_temp, root) = setup(true);
        fs::write(root.join("extra.txt"), "staged").unwrap();
        git_text(&root, &["add", "extra.txt"]).unwrap();
        let mut candidate = plan(&root, "reviewed").unwrap();
        let expected = candidate.report.plan_hash.clone();
        let head = head_oid(&root);
        let index = fs::read(root.join(".git/index")).unwrap();
        candidate.report.message = "different message".into();
        assert!(apply(&mut candidate, &expected).is_err());
        assert_eq!(head_oid(&root), head);
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    }
}
