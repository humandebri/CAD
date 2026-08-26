use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::thread;
use std::time::{Duration, Instant};

use super::{ProjectWatchEvent, ProjectWatchEventKind};

pub(crate) const PROJECT_WATCH_DEBOUNCE_MS: u64 = 250;
const PROJECT_POLL_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) struct ProjectWatcher {
    pub(crate) _project_path: String,
    _backend: ProjectWatcherBackend,
}

enum ProjectWatcherBackend {
    Native { _watcher: NativeWatcher },
    ManifestPoll { _watcher: ManifestPollWatcher },
}

struct NativeWatcher {
    watcher: Option<RecommendedWatcher>,
    sender: mpsc::Sender<NativeSignal>,
    stopped: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}

enum NativeSignal {
    Dirty,
    Error(String),
    Stop,
}

#[derive(Default)]
struct NativeRuntimeState {
    error_active: bool,
    dirty_after_error: bool,
}

impl NativeRuntimeState {
    fn record_dirty(&mut self) {
        if self.error_active {
            self.dirty_after_error = true;
        }
    }

    fn record_error(&mut self) {
        self.error_active = true;
        self.dirty_after_error = false;
    }

    fn can_refresh(&self) -> bool {
        !self.error_active || self.dirty_after_error
    }

    fn record_refresh_success(&mut self) {
        self.error_active = false;
        self.dirty_after_error = false;
    }
}

struct ManifestPollWatcher {
    stop: mpsc::Sender<()>,
    thread: Option<thread::JoinHandle<()>>,
}

#[derive(Default)]
struct ManifestWatchState {
    previous: Option<Vec<cad_model::SourceFileRevision>>,
    error_reported: bool,
}

enum ManifestRefresh {
    Changed(Vec<String>),
    Unchanged,
    Error(String),
    RepeatedError,
}

impl ManifestWatchState {
    fn refresh(&mut self, root: &Path) -> ManifestRefresh {
        match cad_model::source_manifest(root) {
            Ok(manifest) => {
                let recovering = self.error_reported;
                let paths = self
                    .previous
                    .as_ref()
                    .map(|previous| manifest_changes(previous, &manifest))
                    .unwrap_or_else(|| {
                        if recovering {
                            manifest_paths(&manifest)
                        } else {
                            Vec::new()
                        }
                    });
                self.previous = Some(manifest);
                self.error_reported = false;
                if !paths.is_empty() || recovering {
                    ManifestRefresh::Changed(paths)
                } else {
                    ManifestRefresh::Unchanged
                }
            }
            Err(_) if self.error_reported => ManifestRefresh::RepeatedError,
            Err(error) => {
                self.error_reported = true;
                ManifestRefresh::Error(format!("canonical source monitoring failed: {error}"))
            }
        }
    }

    fn snapshot_paths(&self) -> Vec<String> {
        self.previous
            .as_deref()
            .map(manifest_paths)
            .unwrap_or_default()
    }
}

impl Drop for NativeWatcher {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.watcher.take();
        let _ = self.sender.send(NativeSignal::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for ManifestPollWatcher {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) fn create_project_watcher<F>(
    root: &Path,
    project_path: String,
    emit: F,
) -> Result<ProjectWatcher, String>
where
    F: Fn(ProjectWatchEvent) + Send + Sync + 'static,
{
    let watch_root = fs::canonicalize(root)
        .map_err(|error| format!("failed to canonicalize project watcher root: {error}"))?;
    let emit = Arc::new(emit);
    let native = start_native_watcher(&watch_root, &project_path, Arc::clone(&emit));
    let backend = select_backend(native, || {
        start_manifest_poll_watcher(
            watch_root,
            project_path.clone(),
            emit,
            PROJECT_POLL_INTERVAL,
        )
    })?;
    Ok(ProjectWatcher {
        _project_path: project_path,
        _backend: backend,
    })
}

fn select_backend<F>(
    native: Result<NativeWatcher, String>,
    fallback: F,
) -> Result<ProjectWatcherBackend, String>
where
    F: FnOnce() -> Result<ManifestPollWatcher, String>,
{
    match native {
        Ok(watcher) => Ok(ProjectWatcherBackend::Native { _watcher: watcher }),
        Err(native_error) => fallback()
            .map(|watcher| ProjectWatcherBackend::ManifestPoll { _watcher: watcher })
            .map_err(|poll_error| {
                format!(
                    "native project watcher failed ({native_error}); canonical source polling fallback failed ({poll_error})"
                )
            }),
    }
}

fn start_native_watcher<F>(
    root: &Path,
    project_path: &str,
    emit: Arc<F>,
) -> Result<NativeWatcher, String>
where
    F: Fn(ProjectWatchEvent) + Send + Sync + 'static,
{
    let (sender, receiver) = mpsc::channel();
    let event_sender = sender.clone();
    let event_root = root.to_path_buf();
    let mut watcher = RecommendedWatcher::new(
        move |result: notify::Result<Event>| {
            let signal = match result {
                Ok(event) if native_event_may_affect_sources(&event_root, &event) => {
                    Some(NativeSignal::Dirty)
                }
                Ok(_) => None,
                Err(error) => Some(NativeSignal::Error(error.to_string())),
            };
            if let Some(signal) = signal {
                let _ = event_sender.send(signal);
            }
        },
        Config::default(),
    )
    .map_err(|error| format!("failed to create native project watcher: {error}"))?;
    watcher
        .watch(root, RecursiveMode::Recursive)
        .map_err(|error| format!("failed to watch project with native watcher: {error}"))?;

    let stopped = Arc::new(AtomicBool::new(false));
    let worker_stopped = Arc::clone(&stopped);
    let worker_root = root.to_path_buf();
    let worker_project_path = project_path.to_owned();
    let (ready, ready_receiver) = mpsc::channel();
    let thread = thread::Builder::new()
        .name("cad native source watcher".to_owned())
        .spawn(move || {
            run_native_worker(
                worker_root,
                worker_project_path,
                emit,
                receiver,
                worker_stopped,
                Duration::from_millis(PROJECT_WATCH_DEBOUNCE_MS),
                Some(ready),
            );
        })
        .map_err(|error| format!("failed to start native project watcher worker: {error}"))?;
    ready_receiver
        .recv()
        .map_err(|_| "native project watcher stopped before initialization".to_owned())?;

    Ok(NativeWatcher {
        watcher: Some(watcher),
        sender,
        stopped,
        thread: Some(thread),
    })
}

fn run_native_worker<F>(
    root: PathBuf,
    project_path: String,
    emit: Arc<F>,
    receiver: mpsc::Receiver<NativeSignal>,
    stopped: Arc<AtomicBool>,
    debounce: Duration,
    ready: Option<mpsc::Sender<()>>,
) where
    F: Fn(ProjectWatchEvent) + Send + Sync + 'static,
{
    let mut state = ManifestWatchState::default();
    emit_manifest_refresh(state.refresh(&root), &project_path, emit.as_ref());
    if let Some(ready) = ready {
        let _ = ready.send(());
    }
    let mut runtime = NativeRuntimeState::default();

    while !stopped.load(Ordering::Acquire) {
        match receiver.recv() {
            Ok(NativeSignal::Dirty) => {
                runtime.record_dirty();
                let deadline = Instant::now() + debounce;
                loop {
                    if stopped.load(Ordering::Acquire) {
                        return;
                    }
                    let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                        break;
                    };
                    match receiver.recv_timeout(remaining) {
                        Ok(NativeSignal::Dirty) => runtime.record_dirty(),
                        Ok(NativeSignal::Error(message)) => {
                            runtime.record_error();
                            emit(watch_error(&project_path, message));
                        }
                        Ok(NativeSignal::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                            return;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => break,
                    }
                }
                if stopped.load(Ordering::Acquire) {
                    return;
                }
                if !runtime.can_refresh() {
                    continue;
                }
                match state.refresh(&root) {
                    ManifestRefresh::Changed(paths) => {
                        runtime.record_refresh_success();
                        emit(watch_changed(&project_path, paths));
                    }
                    ManifestRefresh::Unchanged if runtime.error_active => {
                        runtime.record_refresh_success();
                        emit(watch_changed(&project_path, state.snapshot_paths()));
                    }
                    ManifestRefresh::Unchanged => runtime.record_refresh_success(),
                    ManifestRefresh::RepeatedError => {}
                    ManifestRefresh::Error(message) => {
                        emit(watch_error(&project_path, message));
                    }
                }
            }
            Ok(NativeSignal::Error(message)) => {
                runtime.record_error();
                emit(watch_error(&project_path, message));
            }
            Ok(NativeSignal::Stop) | Err(_) => break,
        }
    }
}

fn start_manifest_poll_watcher<F>(
    root: PathBuf,
    project_path: String,
    emit: Arc<F>,
    interval: Duration,
) -> Result<ManifestPollWatcher, String>
where
    F: Fn(ProjectWatchEvent) + Send + Sync + 'static,
{
    let (stop, receiver) = mpsc::channel();
    let (ready, ready_receiver) = mpsc::channel();
    let thread = thread::Builder::new()
        .name("cad canonical source poll".to_owned())
        .spawn(move || {
            let mut state = ManifestWatchState::default();
            let mut ready = Some(ready);
            loop {
                emit_manifest_refresh(state.refresh(&root), &project_path, emit.as_ref());
                if let Some(ready) = ready.take() {
                    let _ = ready.send(());
                }
                match receiver.recv_timeout(interval) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            }
        })
        .map_err(|error| format!("failed to start canonical source poll thread: {error}"))?;
    ready_receiver
        .recv()
        .map_err(|_| "canonical source poll stopped before initialization".to_owned())?;
    Ok(ManifestPollWatcher {
        stop,
        thread: Some(thread),
    })
}

fn emit_manifest_refresh<F>(refresh: ManifestRefresh, project_path: &str, emit: &F)
where
    F: Fn(ProjectWatchEvent),
{
    match refresh {
        ManifestRefresh::Changed(paths) => emit(watch_changed(project_path, paths)),
        ManifestRefresh::Error(message) => emit(watch_error(project_path, message)),
        ManifestRefresh::Unchanged | ManifestRefresh::RepeatedError => {}
    }
}

fn watch_changed(project_path: &str, paths: Vec<String>) -> ProjectWatchEvent {
    ProjectWatchEvent {
        kind: ProjectWatchEventKind::Changed,
        project_path: project_path.to_owned(),
        paths,
        message: None,
    }
}

fn watch_error(project_path: &str, message: String) -> ProjectWatchEvent {
    ProjectWatchEvent {
        kind: ProjectWatchEventKind::Error,
        project_path: project_path.to_owned(),
        paths: Vec::new(),
        message: Some(message),
    }
}

fn native_event_may_affect_sources(root: &Path, event: &Event) -> bool {
    event.need_rescan()
        || event.paths.is_empty()
        || event
            .paths
            .iter()
            .any(|path| source_or_ancestor_path(root, path))
}

fn source_or_ancestor_path(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    if relative.as_os_str().is_empty()
        || cad_model::classify_project_source_path(relative).is_some()
    {
        return true;
    }
    let components = relative.components().collect::<Vec<_>>();
    if components
        .iter()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return false;
    }
    let component = |index: usize| components.get(index)?.as_os_str().to_str();
    matches!(
        (component(0), components.len()),
        (Some("rules" | "drawings" | "comments" | "blocks"), 1) | (Some("drawings" | "blocks"), 2)
    )
}

fn manifest_changes(
    before: &[cad_model::SourceFileRevision],
    after: &[cad_model::SourceFileRevision],
) -> Vec<String> {
    let before = before
        .iter()
        .map(|file| (file.relative_path.as_str(), (&file.revision, file.exists)))
        .collect::<BTreeMap<_, _>>();
    let after = after
        .iter()
        .map(|file| (file.relative_path.as_str(), (&file.revision, file.exists)))
        .collect::<BTreeMap<_, _>>();
    before
        .keys()
        .chain(after.keys())
        .filter(|path| before.get(**path) != after.get(**path))
        .map(|path| (*path).to_owned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn manifest_paths(manifest: &[cad_model::SourceFileRevision]) -> Vec<String> {
    manifest
        .iter()
        .map(|file| file.relative_path.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::{EventKind, event::Flag};
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::sync::Mutex;

    fn write_source_project(root: &Path) {
        fs::create_dir_all(root.join("rules")).expect("rules should be created");
        fs::create_dir_all(root.join("drawings/plan")).expect("drawing should be created");
        fs::write(
            root.join("cad.project.toml"),
            "schema_version = \"0.2\"\nname = \"watch\"\n",
        )
        .expect("project should be written");
        fs::write(root.join("rules/layers.toml"), "schema_version = \"0.2\"\n")
            .expect("layers should be written");
        fs::write(root.join("rules/styles.toml"), "schema_version = \"0.2\"\n")
            .expect("styles should be written");
        fs::write(
            root.join("drawings/plan/layouts.toml"),
            "schema_version = \"0.2\"\nactive_layout = \"default\"\n[layouts.default]\npaper = \"A3\"\norientation = \"landscape\"\nscale = \"1/100\"\norigin = [0, 0]\nmargins = [0, 0, 0, 0]\n",
        )
        .expect("layouts should be written");
        fs::write(root.join("drawings/plan/entities.ndjson"), "")
            .expect("entities should be written");
    }

    #[test]
    fn manifest_diff_reports_create_update_and_delete_only_once() {
        let before = vec![cad_model::SourceFileRevision {
            relative_path: "rules/styles.toml".to_owned(),
            revision: "old".to_owned(),
            exists: true,
        }];
        let after = vec![cad_model::SourceFileRevision {
            relative_path: "comments/plan.ndjson".to_owned(),
            revision: "new".to_owned(),
            exists: true,
        }];
        assert_eq!(
            manifest_changes(&before, &after),
            vec![
                "comments/plan.ndjson".to_owned(),
                "rules/styles.toml".to_owned()
            ]
        );
    }

    #[test]
    fn raw_rescan_and_source_directories_are_dirty_signals() {
        let root = Path::new("/project");
        let rescan = Event::new(EventKind::Other).set_flag(Flag::Rescan);
        assert!(native_event_may_affect_sources(root, &rescan));
        assert!(native_event_may_affect_sources(
            root,
            &Event::new(EventKind::Any).add_path(root.join("drawings/plan"))
        ));
        assert!(native_event_may_affect_sources(
            root,
            &Event::new(EventKind::Any).add_path(root.join("blocks/door"))
        ));
        assert!(!native_event_may_affect_sources(
            root,
            &Event::new(EventKind::Any).add_path(root.join("build/review.json"))
        ));
        assert!(!native_event_may_affect_sources(
            root,
            &Event::new(EventKind::Any).add_path(root.join("README.md"))
        ));
        assert!(!native_event_may_affect_sources(
            root,
            &Event::new(EventKind::Any).add_path(root.join("interop/jww/original.jww"))
        ));
    }

    #[test]
    fn directory_removal_reports_each_removed_canonical_source() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        write_source_project(temp.path());
        let mut state = ManifestWatchState::default();
        assert!(matches!(
            state.refresh(temp.path()),
            ManifestRefresh::Unchanged
        ));
        fs::rename(
            temp.path().join("drawings/plan"),
            temp.path().join("removed-plan"),
        )
        .expect("drawing should be removed");
        let ManifestRefresh::Changed(paths) = state.refresh(temp.path()) else {
            panic!("directory removal should change the manifest");
        };
        assert_eq!(
            paths,
            vec![
                "drawings/plan/entities.ndjson".to_owned(),
                "drawings/plan/layouts.toml".to_owned()
            ]
        );
    }

    #[test]
    fn one_manifest_batch_reports_create_update_and_delete() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        write_source_project(temp.path());
        let mut state = ManifestWatchState::default();
        assert!(matches!(
            state.refresh(temp.path()),
            ManifestRefresh::Unchanged
        ));
        fs::create_dir_all(temp.path().join("comments")).expect("comments should be created");
        fs::write(temp.path().join("comments/plan.ndjson"), "")
            .expect("comment source should be created");
        fs::write(
            temp.path().join("rules/layers.toml"),
            "schema_version = \"0.2\"\n# updated\n",
        )
        .expect("layers should be updated");
        fs::remove_file(temp.path().join("rules/styles.toml")).expect("styles should be deleted");

        let ManifestRefresh::Changed(paths) = state.refresh(temp.path()) else {
            panic!("source batch should change the manifest");
        };
        assert_eq!(
            paths,
            vec![
                "comments/plan.ndjson".to_owned(),
                "rules/layers.toml".to_owned(),
                "rules/styles.toml".to_owned()
            ]
        );
    }

    #[test]
    fn manifest_refresh_ignores_generated_and_noncanonical_files() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        write_source_project(temp.path());
        let mut state = ManifestWatchState::default();
        assert!(matches!(
            state.refresh(temp.path()),
            ManifestRefresh::Unchanged
        ));
        fs::create_dir_all(temp.path().join("build")).expect("build should be created");
        fs::write(temp.path().join("build/generated.toml"), "ignored")
            .expect("build output should be written");
        fs::create_dir_all(temp.path().join(".cad-history")).expect("history should be created");
        fs::write(temp.path().join(".cad-history/entry.toml"), "ignored")
            .expect("history should be written");
        fs::write(temp.path().join("README.md"), "ignored").expect("readme should be written");
        assert!(matches!(
            state.refresh(temp.path()),
            ManifestRefresh::Unchanged
        ));
    }

    #[test]
    fn manifest_error_is_reported_once_and_recovery_forces_catch_up() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        write_source_project(temp.path());
        let mut state = ManifestWatchState::default();
        assert!(matches!(
            state.refresh(temp.path()),
            ManifestRefresh::Unchanged
        ));
        fs::remove_file(temp.path().join("rules/styles.toml")).expect("styles should be removed");
        let external = tempfile::NamedTempFile::new().expect("external file should be created");
        symlink(external.path(), temp.path().join("rules/styles.toml"))
            .expect("unsafe symlink should be created");
        assert!(matches!(
            state.refresh(temp.path()),
            ManifestRefresh::Error(_)
        ));
        assert!(matches!(
            state.refresh(temp.path()),
            ManifestRefresh::RepeatedError
        ));
        fs::remove_file(temp.path().join("rules/styles.toml"))
            .expect("unsafe symlink should be removed");
        fs::write(
            temp.path().join("rules/styles.toml"),
            "schema_version = \"0.2\"\n",
        )
        .expect("styles should be restored");
        assert!(matches!(
            state.refresh(temp.path()),
            ManifestRefresh::Changed(_)
        ));
    }

    #[test]
    fn runtime_error_is_followed_by_a_catch_up_event_without_switching_backend() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        write_source_project(temp.path());
        let events = Arc::new(Mutex::new(Vec::new()));
        let event_sink = Arc::clone(&events);
        let (sender, receiver) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = Arc::clone(&stopped);
        let root = temp.path().to_path_buf();
        let thread = thread::spawn(move || {
            run_native_worker(
                root,
                "project".to_owned(),
                Arc::new(move |event| event_sink.lock().expect("events should lock").push(event)),
                receiver,
                worker_stopped,
                Duration::ZERO,
                None,
            );
        });
        sender
            .send(NativeSignal::Error("runtime failure".to_owned()))
            .expect("error should send");
        sender.send(NativeSignal::Dirty).expect("dirty should send");
        sender.send(NativeSignal::Stop).expect("stop should send");
        thread.join().expect("worker should stop");
        let events = events.lock().expect("events should lock");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, ProjectWatchEventKind::Error);
        assert_eq!(events[1].kind, ProjectWatchEventKind::Changed);
    }

    #[test]
    fn runtime_error_requires_a_dirty_signal_after_the_error() {
        let mut runtime = NativeRuntimeState::default();
        runtime.record_dirty();
        assert!(runtime.can_refresh());
        runtime.record_error();
        assert!(!runtime.can_refresh());
        runtime.record_dirty();
        assert!(runtime.can_refresh());
        runtime.record_refresh_success();
        assert!(!runtime.error_active);
    }

    #[test]
    fn stopped_native_worker_drops_pending_dirty_event() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        write_source_project(temp.path());
        let events = Arc::new(Mutex::new(Vec::new()));
        let event_sink = Arc::clone(&events);
        let (sender, receiver) = mpsc::channel();
        let stopped = Arc::new(AtomicBool::new(false));
        let worker_stopped = Arc::clone(&stopped);
        let root = temp.path().to_path_buf();
        let (ready, ready_receiver) = mpsc::channel();
        let thread = thread::spawn(move || {
            run_native_worker(
                root,
                "project".to_owned(),
                Arc::new(move |event| event_sink.lock().expect("events should lock").push(event)),
                receiver,
                worker_stopped,
                Duration::from_secs(1),
                Some(ready),
            );
        });
        ready_receiver.recv().expect("worker should initialize");
        sender.send(NativeSignal::Dirty).expect("dirty should send");
        stopped.store(true, Ordering::Release);
        sender.send(NativeSignal::Stop).expect("stop should send");
        thread.join().expect("worker should stop");
        assert!(events.lock().expect("events should lock").is_empty());
    }

    #[test]
    fn native_preference_uses_a_native_backend() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        write_source_project(temp.path());
        let watcher = create_project_watcher(
            temp.path(),
            temp.path().to_string_lossy().into_owned(),
            |_| {},
        )
        .expect("watcher should start");
        assert!(matches!(
            watcher._backend,
            ProjectWatcherBackend::Native { .. }
        ));
    }

    #[test]
    fn forced_native_failure_selects_manifest_poll_backend() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        write_source_project(temp.path());
        let backend = select_backend(Err("injected native failure".to_owned()), || {
            start_manifest_poll_watcher(
                temp.path().to_path_buf(),
                temp.path().to_string_lossy().into_owned(),
                Arc::new(|_| {}),
                Duration::from_millis(10),
            )
        })
        .expect("fallback should start");
        assert!(matches!(
            backend,
            ProjectWatcherBackend::ManifestPoll { .. }
        ));
    }
}
