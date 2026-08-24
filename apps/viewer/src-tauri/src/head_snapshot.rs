use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub(crate) struct HeadSnapshotCache {
    load: Mutex<()>,
    entry: Mutex<Option<CacheEntry>>,
}

struct CacheEntry {
    key: CacheKey,
    project: Arc<cad_model::ProjectSource>,
    _root: tempfile::TempDir,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CacheKey {
    repo: PathBuf,
    project: PathBuf,
    oid: String,
}

trait GitRunner: Send + Sync {
    fn output(&self, directory: &Path, args: &[&OsStr]) -> Result<Vec<u8>, String>;
}

struct ProcessGitRunner;

impl GitRunner for ProcessGitRunner {
    fn output(&self, directory: &Path, args: &[&OsStr]) -> Result<Vec<u8>, String> {
        let output = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(args)
            .output()
            .map_err(|error| format!("failed to run git: {error}"))?;
        if !output.status.success() {
            let command = args
                .iter()
                .map(|arg| arg.to_string_lossy())
                .collect::<Vec<_>>()
                .join(" ");
            return Err(command_stderr(&format!("git {command}"), &output.stderr));
        }
        Ok(output.stdout)
    }
}

impl HeadSnapshotCache {
    pub(crate) fn load(
        &self,
        project_path: &Path,
    ) -> Result<Arc<cad_model::ProjectSource>, String> {
        self.load_with(project_path, &ProcessGitRunner)
    }

    fn load_with<R: GitRunner>(
        &self,
        project_path: &Path,
        runner: &R,
    ) -> Result<Arc<cad_model::ProjectSource>, String> {
        let _load = self
            .load
            .lock()
            .map_err(|_| "HEAD snapshot load lock is poisoned".to_owned())?;
        let project = fs::canonicalize(project_path)
            .map_err(|error| format!("failed to canonicalize project: {error}"))?;
        let cached_repo = {
            let mut entry = self
                .entry
                .lock()
                .map_err(|_| "HEAD snapshot cache is poisoned".to_owned())?;
            if entry
                .as_ref()
                .is_some_and(|entry| entry.key.project != project)
            {
                *entry = None;
            }
            entry.as_ref().map(|entry| entry.key.repo.clone())
        };
        let repo = match cached_repo {
            Some(repo) => repo,
            None => git_root_with(runner, &project)?,
        };
        let relative_project = project
            .strip_prefix(&repo)
            .map_err(|_| "project is not inside the Git repository".to_owned())?;
        let oid = git_output_text(
            runner,
            &repo,
            &[
                OsStr::new("rev-parse"),
                OsStr::new("--verify"),
                OsStr::new("HEAD^{commit}"),
            ],
        )?;
        let key = CacheKey {
            repo: repo.clone(),
            project: project.clone(),
            oid,
        };
        {
            let mut entry = self
                .entry
                .lock()
                .map_err(|_| "HEAD snapshot cache is poisoned".to_owned())?;
            if let Some(cached) = entry.as_ref().filter(|entry| entry.key == key) {
                return Ok(Arc::clone(&cached.project));
            }
            *entry = None;
        }

        let (temp, source) = build_head_snapshot(runner, &key, relative_project)?;
        let source = Arc::new(source);
        *self
            .entry
            .lock()
            .map_err(|_| "HEAD snapshot cache is poisoned".to_owned())? = Some(CacheEntry {
            key,
            project: Arc::clone(&source),
            _root: temp,
        });
        Ok(source)
    }
}

pub(crate) fn git_root(project_path: &Path) -> Result<PathBuf, String> {
    git_root_with(&ProcessGitRunner, project_path)
}

fn git_root_with<R: GitRunner>(runner: &R, project_path: &Path) -> Result<PathBuf, String> {
    let root = git_output_text(
        runner,
        project_path,
        &[OsStr::new("rev-parse"), OsStr::new("--show-toplevel")],
    )?;
    fs::canonicalize(&root).map_err(|error| format!("failed to canonicalize repo: {error}"))
}

fn git_output_text<R: GitRunner>(
    runner: &R,
    directory: &Path,
    args: &[&OsStr],
) -> Result<String, String> {
    let bytes = runner.output(directory, args)?;
    let text =
        String::from_utf8(bytes).map_err(|error| format!("git output was not UTF-8: {error}"))?;
    let text = text.trim();
    if text.is_empty() {
        Err("git returned an empty value".to_owned())
    } else {
        Ok(text.to_owned())
    }
}

fn build_head_snapshot<R: GitRunner>(
    runner: &R,
    key: &CacheKey,
    relative_project: &Path,
) -> Result<(tempfile::TempDir, cad_model::ProjectSource), String> {
    let relative_project_arg = git_project_pathspec(relative_project);
    let head_files = runner.output(
        &key.repo,
        &[
            OsStr::new("ls-tree"),
            OsStr::new("-r"),
            OsStr::new("-z"),
            OsStr::new("--name-only"),
            OsStr::new(&key.oid),
            OsStr::new("--"),
            OsStr::new(&relative_project_arg),
        ],
    )?;
    let canonical_files = head_files
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| PathBuf::from(OsString::from_vec(path.to_vec())))
        .filter_map(|git_path| {
            let relative = git_path.strip_prefix(relative_project).ok()?.to_path_buf();
            cad_model::classify_project_source_path(&relative)?;
            Some((git_path, relative))
        })
        .collect::<Vec<_>>();
    if canonical_files.is_empty() {
        return Err("project has no canonical source files in Git".to_owned());
    }

    let temp = tempfile::tempdir().map_err(|error| format!("failed to create tempdir: {error}"))?;
    for (git_path, relative_file) in canonical_files {
        let mut object_path = OsString::from(&key.oid);
        object_path.push(":");
        object_path.push(git_path.as_os_str());
        let content = runner.output(&key.repo, &[OsStr::new("show"), object_path.as_os_str()])?;
        let destination = temp.path().join(relative_file);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create temp parent: {error}"))?;
        }
        fs::write(&destination, content)
            .map_err(|error| format!("failed to write HEAD file: {error}"))?;
    }
    let source = cad_model::load_project(temp.path())
        .map_err(|error| format!("failed to load HEAD project: {error}"))?;
    Ok((temp, source))
}

fn command_stderr(command: &str, stderr: &[u8]) -> String {
    let message = String::from_utf8_lossy(stderr).trim().to_owned();
    if message.is_empty() {
        format!("{command} failed")
    } else {
        format!("{command} failed: {message}")
    }
}

fn path_to_git_arg(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn git_project_pathspec(relative_project: &Path) -> String {
    let pathspec = path_to_git_arg(relative_project);
    if pathspec.is_empty() {
        ".".to_owned()
    } else {
        pathspec
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[derive(Default)]
    struct CountingGitRunner {
        calls: Mutex<Vec<String>>,
        shown: Mutex<Vec<String>>,
        fail_next_ls_tree: AtomicBool,
    }

    impl GitRunner for CountingGitRunner {
        fn output(&self, directory: &Path, args: &[&OsStr]) -> Result<Vec<u8>, String> {
            let command = args
                .first()
                .map(|value| value.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.calls
                .lock()
                .expect("calls should lock")
                .push(command.clone());
            if command == "show" {
                self.shown.lock().expect("shown should lock").push(
                    args.get(1)
                        .map(|value| value.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                );
            }
            if command == "ls-tree" && self.fail_next_ls_tree.swap(false, Ordering::SeqCst) {
                return Err("injected ls-tree failure".to_owned());
            }
            ProcessGitRunner.output(directory, args)
        }
    }

    fn write_project(root: &Path, name: &str) {
        fs::create_dir_all(root.join("rules")).expect("rules should be created");
        fs::create_dir_all(root.join("drawings/plan")).expect("drawing should be created");
        fs::write(
            root.join("cad.project.toml"),
            format!("schema_version = \"0.2\"\nname = \"{name}\"\n"),
        )
        .expect("project should be written");
        fs::write(
            root.join("rules/layers.toml"),
            "active_layer = \"0\"\n[layers.\"0\"]\nname = \"Layer\"\nvisible = true\nprintable = true\ncolor = \"black\"\nline_type = \"solid\"\nline_width = 0.25\n",
        )
        .expect("layers should be written");
        fs::write(
            root.join("rules/styles.toml"),
            "[colors.black]\nrgb = \"#000000\"\nprint_width = 0.25\n[line_types.solid]\ndash = []\n[text_styles]\n[dimension_styles]\n",
        )
        .expect("styles should be written");
        fs::write(
            root.join("drawings/plan/layouts.toml"),
            "schema_version = \"0.2\"\nactive_layout = \"default\"\n[layouts.default]\nname = \"Default\"\npaper = \"A3\"\norientation = \"landscape\"\nscale = \"1/100\"\norigin = [0, 0]\nmargins = [0, 0, 0, 0]\n",
        )
        .expect("layouts should be written");
        fs::write(
            root.join("drawings/plan/entities.ndjson"),
            "{\"schema_version\":\"0.2\",\"id\":\"ent_01JZ0000000000000000000000\",\"type\":\"line\",\"layer\":\"0\",\"p1\":[0,0],\"p2\":[10,0]}\n",
        )
        .expect("entities should be written");
    }

    fn init_repo(root: &Path) {
        run_git(root, &["init"]);
        run_git(root, &["config", "user.email", "cad@example.invalid"]);
        run_git(root, &["config", "user.name", "CAD Test"]);
        run_git(root, &["add", "."]);
        run_git(root, &["commit", "-m", "initial"]);
    }

    fn run_git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn call_counts(runner: &CountingGitRunner) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for call in runner.calls.lock().expect("calls should lock").iter() {
            *counts.entry(call.clone()).or_insert(0) += 1;
        }
        counts
    }

    #[test]
    fn same_oid_reuses_snapshot_and_working_tree_changes_do_not_invalidate_it() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        write_project(temp.path(), "first");
        fs::write(temp.path().join("README.md"), "not source\n").expect("readme should be written");
        fs::create_dir_all(temp.path().join("build")).expect("build should be created");
        fs::write(temp.path().join("build/report.json"), "{}\n")
            .expect("build report should be written");
        init_repo(temp.path());
        let cache = HeadSnapshotCache::default();
        let runner = CountingGitRunner::default();

        let first = cache
            .load_with(temp.path(), &runner)
            .expect("first snapshot should load");
        let first_counts = call_counts(&runner);
        fs::write(
            temp.path().join("drawings/plan/entities.ndjson"),
            "{\"schema_version\":\"0.2\",\"id\":\"ent_01JZ0000000000000000000000\",\"type\":\"line\",\"layer\":\"0\",\"p1\":[0,0],\"p2\":[20,0]}\n",
        )
        .expect("working tree should change");
        let second = cache
            .load_with(temp.path(), &runner)
            .expect("cached snapshot should load");
        let second_counts = call_counts(&runner);

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(second_counts["ls-tree"], first_counts["ls-tree"]);
        assert_eq!(second_counts["show"], first_counts["show"]);
        assert_eq!(second_counts["rev-parse"], first_counts["rev-parse"] + 1);
        assert!(
            runner
                .shown
                .lock()
                .expect("shown should lock")
                .iter()
                .all(|path| !path.contains("README.md") && !path.contains("build/report.json"))
        );
    }

    #[test]
    fn new_head_oid_replaces_the_cached_snapshot() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        write_project(temp.path(), "first");
        init_repo(temp.path());
        let cache = HeadSnapshotCache::default();
        let first = cache.load(temp.path()).expect("first snapshot should load");
        fs::write(
            temp.path().join("cad.project.toml"),
            "schema_version = \"0.2\"\nname = \"second\"\n",
        )
        .expect("project should change");
        run_git(temp.path(), &["add", "."]);
        run_git(temp.path(), &["commit", "-m", "second"]);
        let second = cache.load(temp.path()).expect("new snapshot should load");
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(second.project.name, "second");
    }

    #[test]
    fn project_switch_replaces_the_single_entry_cache() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let first_path = temp.path().join("first");
        let second_path = temp.path().join("second");
        write_project(&first_path, "first");
        write_project(&second_path, "second");
        init_repo(temp.path());
        let cache = HeadSnapshotCache::default();
        let first = cache.load(&first_path).expect("first project should load");
        let second = cache
            .load(&second_path)
            .expect("second project should load");
        let first_again = cache
            .load(&first_path)
            .expect("first project should reload");
        assert_eq!(first.project.name, "first");
        assert_eq!(second.project.name, "second");
        assert!(!Arc::ptr_eq(&first, &first_again));
    }

    #[test]
    fn failed_snapshot_is_not_cached() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        write_project(temp.path(), "first");
        init_repo(temp.path());
        let cache = HeadSnapshotCache::default();
        let runner = CountingGitRunner::default();
        runner.fail_next_ls_tree.store(true, Ordering::SeqCst);
        assert!(cache.load_with(temp.path(), &runner).is_err());
        let snapshot = cache
            .load_with(temp.path(), &runner)
            .expect("failed snapshot should be retried");
        assert_eq!(snapshot.project.name, "first");
        assert_eq!(call_counts(&runner)["ls-tree"], 2);
    }
}
