//! Read-only, revision-pinned CAD snapshots from Git or the working tree.
use cad_model::{ProjectSource, SourceFileRevision};
use serde::Serialize;
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use thiserror::Error;

pub mod blame;
pub mod commit;
pub mod history;
pub mod merge;
pub mod merge_apply;
pub mod stage;

#[derive(Debug, Error)]
pub enum GitError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Model(#[from] cad_model::ModelError),
}
type Result<T> = std::result::Result<T, GitError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Revision {
    Worktree,
    Index,
    Commit(String),
}
impl Revision {
    pub fn parse(value: &str) -> Self {
        match value {
            "worktree" => Self::Worktree,
            "index" => Self::Index,
            other => Self::Commit(other.to_owned()),
        }
    }
    pub fn label(&self) -> &str {
        match self {
            Self::Worktree => "worktree",
            Self::Index => "index",
            Self::Commit(value) => value,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SnapshotIdentity {
    pub revision: String,
    pub commit_oid: Option<String>,
    pub source_manifest: Vec<SourceFileRevision>,
    pub snapshot_blake3: String,
}

pub struct Snapshot {
    pub source: ProjectSource,
    pub identity: SnapshotIdentity,
    _directory: tempfile::TempDir,
}

/// A pinned byte snapshot that can also be checked when canonical parsing fails.
/// No source recovery or normalization is performed on the original project.
pub struct FrozenSnapshot {
    pub root: PathBuf,
    pub identity: SnapshotIdentity,
    _directory: tempfile::TempDir,
}
impl FrozenSnapshot {
    pub fn load(self) -> Result<Snapshot> {
        let source = cad_model::load_project(&self.root)?;
        Ok(Snapshot {
            source,
            identity: self.identity,
            _directory: self._directory,
        })
    }
}

fn git(directory: &Path, args: &[OsString]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .output()?;
    if !output.status.success() {
        return Err(GitError::Invalid(format!(
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}
fn args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}
fn git_text(directory: &Path, values: &[&str]) -> Result<String> {
    String::from_utf8(git(directory, &args(values))?)
        .map(|text| text.trim_end().to_owned())
        .map_err(|_| GitError::Invalid("Git returned non-UTF8 metadata".to_owned()))
}
pub fn repository_root(project: &Path) -> Result<PathBuf> {
    let text = git_text(project, &["rev-parse", "--show-toplevel"])?;
    Ok(fs::canonicalize(text)?)
}
pub fn head_oid(project: &Path) -> Option<String> {
    git_text(project, &["rev-parse", "--verify", "HEAD^{commit}"]).ok()
}

/// Both locations matter in linked worktrees: HEAD/index are worktree-local,
/// refs and packed-refs live in the common Git directory.
pub fn metadata_directories(project: &Path) -> Result<Vec<PathBuf>> {
    let output = git_text(
        project,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-dir",
            "--git-common-dir",
        ],
    )?;
    let mut paths = output
        .lines()
        .map(fs::canonicalize)
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn supported_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        && (cad_model::classify_project_source_path(path).is_some()
            || [
                cad_model::JWW_ORIGINAL_RELATIVE_PATH,
                cad_model::JWW_PRESERVATION_RELATIVE_PATH,
                cad_model::JWW_RECORDS_RELATIVE_PATH,
            ]
            .iter()
            .any(|name| path == Path::new(name)))
}

fn source_files(project: &Path) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let project_root = fs::canonicalize(project)?;
    let project = project_root.as_path();
    let before = cad_model::source_manifest(project)?;
    let mut paths: Vec<_> = before
        .iter()
        .filter(|file| file.exists)
        .map(|file| PathBuf::from(&file.relative_path))
        .collect();
    for name in [
        cad_model::JWW_ORIGINAL_RELATIVE_PATH,
        cad_model::JWW_PRESERVATION_RELATIVE_PATH,
        cad_model::JWW_RECORDS_RELATIVE_PATH,
    ] {
        let relative = PathBuf::from(name);
        if project.join(&relative).try_exists()? {
            paths.push(relative);
        }
    }
    paths.sort();
    paths.dedup();
    let files: Vec<_> = paths
        .into_iter()
        .map(|relative| {
            let path = project.join(&relative);
            if !fs::canonicalize(&path)?.starts_with(project) {
                return Err(GitError::Invalid(format!(
                    "snapshot source escapes project: {}",
                    path.display()
                )));
            }
            if !fs::symlink_metadata(&path)?.file_type().is_file() {
                return Err(GitError::Invalid(format!(
                    "snapshot source must be a regular file: {}",
                    path.display()
                )));
            }
            Ok((relative, fs::read(path)?))
        })
        .collect::<Result<_>>()?;
    for (path, bytes) in &files {
        if fs::read(project.join(path))? != *bytes {
            return Err(GitError::Invalid(
                "source changed while building snapshot".to_owned(),
            ));
        }
    }
    if cad_model::source_manifest(project)? != before {
        return Err(GitError::Invalid(
            "source changed while building snapshot".to_owned(),
        ));
    }
    Ok(files)
}

pub fn snapshot(project: &Path, revision: &Revision) -> Result<Snapshot> {
    snapshot_files(project, revision)?.load()
}

pub fn snapshot_files(project: &Path, revision: &Revision) -> Result<FrozenSnapshot> {
    let project = fs::canonicalize(project)?;
    let (files, oid) = match revision {
        Revision::Worktree => {
            let before = head_oid(&project);
            let files = source_files(&project)?;
            if before != head_oid(&project) {
                return Err(GitError::Invalid(
                    "HEAD changed while building snapshot".to_owned(),
                ));
            }
            (files, before)
        }
        Revision::Index | Revision::Commit(_) => {
            let repo = repository_root(&project)?;
            let relative = project
                .strip_prefix(&repo)
                .map_err(|_| GitError::Invalid("project is outside repository".to_owned()))?;
            let pathspec = format!(
                ":(literal){}",
                relative
                    .to_str()
                    .ok_or_else(|| GitError::Invalid("project path is not UTF8".to_owned()))?
            );
            let pathspec = if relative.as_os_str().is_empty() {
                ":(top)".to_owned()
            } else {
                pathspec
            };
            let oid = match revision {
                Revision::Commit(value) => Some(git_text(
                    &repo,
                    &[
                        "rev-parse",
                        "--verify",
                        "--end-of-options",
                        &format!("{value}^{{commit}}"),
                    ],
                )?),
                _ => None,
            };
            let command = if let Some(oid) = &oid {
                args(&["ls-tree", "-r", "-z", oid, "--", &pathspec])
            } else {
                args(&["ls-files", "--stage", "-z", "--", &pathspec])
            };
            let listing = git(&repo, &command)?;
            let mut files = Vec::new();
            for entry in listing
                .split(|byte| *byte == 0)
                .filter(|entry| !entry.is_empty())
            {
                let split = entry
                    .iter()
                    .position(|byte| *byte == b'\t')
                    .ok_or_else(|| GitError::Invalid("invalid Git entry".to_owned()))?;
                let path =
                    Path::new(std::str::from_utf8(&entry[split + 1..]).map_err(|_| {
                        GitError::Invalid("CAD source path is not UTF8".to_owned())
                    })?);
                let Ok(path) = path.strip_prefix(relative) else {
                    continue;
                };
                if !supported_path(path) {
                    continue;
                }
                let fields: Vec<_> = std::str::from_utf8(&entry[..split])
                    .map_err(|_| GitError::Invalid("invalid Git metadata".to_owned()))?
                    .split(' ')
                    .collect();
                if fields.len() != 3 || !matches!(fields[0], "100644" | "100755") {
                    return Err(GitError::Invalid(format!(
                        "CAD Git source must be a regular blob: {}",
                        path.display()
                    )));
                }
                let blob = if oid.is_some() {
                    if fields[1] != "blob" {
                        return Err(GitError::Invalid("CAD source is not a blob".to_owned()));
                    }
                    fields[2]
                } else {
                    if fields[2] != "0" {
                        return Err(GitError::Invalid(format!(
                            "unmerged CAD index entry: {}",
                            path.display()
                        )));
                    }
                    fields[1]
                };
                files.push((
                    path.to_path_buf(),
                    git(&repo, &args(&["cat-file", "blob", blob]))?,
                ));
            }
            if oid.is_none() && git(&repo, &command)? != listing {
                return Err(GitError::Invalid(
                    "Git index changed while building snapshot".to_owned(),
                ));
            }
            (files, oid)
        }
    };
    if !files
        .iter()
        .any(|(path, _)| path == Path::new("cad.project.toml"))
    {
        return Err(GitError::Invalid(format!(
            "{} has no CAD project source",
            revision.label()
        )));
    }
    let directory = tempfile::tempdir()?;
    let mut hash = blake3::Hasher::new();
    for (relative, bytes) in files {
        let path = directory.path().join(&relative);
        fs::create_dir_all(path.parent().expect("snapshot file parent"))?;
        fs::write(path, &bytes)?;
        let name = relative.to_string_lossy();
        hash.update(&(name.len() as u64).to_le_bytes());
        hash.update(name.as_bytes());
        hash.update(&(bytes.len() as u64).to_le_bytes());
        hash.update(&bytes);
    }
    let identity = SnapshotIdentity {
        revision: revision.label().to_owned(),
        commit_oid: oid,
        source_manifest: cad_model::source_manifest(directory.path())?,
        snapshot_blake3: hash.finalize().to_hex().to_string(),
    };
    Ok(FrozenSnapshot {
        root: directory.path().to_path_buf(),
        identity,
        _directory: directory,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn run(path: &Path, command: &[&str]) {
        git(path, &args(command)).unwrap();
    }
    fn fixture() -> tempfile::TempDir {
        let temp = tempfile::tempdir().unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        for (path, bytes) in source_files(&source).unwrap() {
            let destination = temp.path().join(path);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::write(destination, bytes).unwrap();
        }
        run(temp.path(), &["init"]);
        run(
            temp.path(),
            &["config", "user.email", "cad@example.invalid"],
        );
        run(temp.path(), &["config", "user.name", "CAD Test"]);
        run(temp.path(), &["add", "."]);
        run(temp.path(), &["commit", "-m", "initial"]);
        temp
    }
    #[test]
    fn commit_index_and_worktree_are_distinct_without_checkout_or_index_writes() {
        let temp = fixture();
        let project = temp.path().join("cad.project.toml");
        fs::write(&project, "schema_version = \"0.3\"\nname = \"staged\"\n").unwrap();
        run(temp.path(), &["add", "cad.project.toml"]);
        fs::write(&project, "schema_version = \"0.3\"\nname = \"working\"\n").unwrap();
        let index = fs::read(temp.path().join(".git/index")).unwrap();
        let working = fs::read(&project).unwrap();
        let base = snapshot(temp.path(), &Revision::Commit("HEAD".to_owned())).unwrap();
        let staged = snapshot(temp.path(), &Revision::Index).unwrap();
        let head = snapshot(temp.path(), &Revision::Worktree).unwrap();
        assert_eq!(staged.source.project.name, "staged");
        assert_eq!(head.source.project.name, "working");
        assert_ne!(base.source.project.name, "staged");
        assert_ne!(
            base.identity.snapshot_blake3,
            staged.identity.snapshot_blake3
        );
        assert_eq!(index, fs::read(temp.path().join(".git/index")).unwrap());
        assert_eq!(working, fs::read(project).unwrap());
    }
    #[test]
    fn project_path_with_git_pathspec_characters_is_literal() {
        let temp = fixture();
        let project = temp.path().join("案件[1]");
        fs::create_dir(&project).unwrap();
        for (path, bytes) in source_files(temp.path()).unwrap() {
            let path = project.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
        }
        run(temp.path(), &["add", "."]);
        run(temp.path(), &["commit", "-m", "nested"]);
        assert!(snapshot(&project, &Revision::Commit("HEAD".to_owned())).is_ok());
        assert!(snapshot(&project, &Revision::Index).is_ok());
    }
    #[test]
    fn rejects_unsafe_paths_and_missing_revisions() {
        assert!(!supported_path(Path::new("../cad.project.toml")));
        assert!(!supported_path(Path::new("/rules/layers.toml")));
        assert!(!supported_path(Path::new("build/anything")));
        let temp = fixture();
        assert!(snapshot(temp.path(), &Revision::Commit("--help".to_owned())).is_err());
        assert!(snapshot(temp.path(), &Revision::Commit("missing-branch".to_owned())).is_err());
    }
}
