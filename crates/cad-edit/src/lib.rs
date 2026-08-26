//! Atomic editing and snapping for CAD NDJSON drawings.

use cad_check::Severity;
use cad_model::{Entity, EntityRecord, Point, ProjectSource};
use geo::{
    Coord, Line,
    line_intersection::{LineIntersection, line_intersection},
};
use rstar::{AABB, RTree, RTreeObject};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use ulid::Ulid;

pub const CRATE_NAME: &str = "cad-edit";

#[must_use]
pub fn crate_name() -> &'static str {
    CRATE_NAME
}

#[derive(Debug, Error)]
pub enum EditError {
    #[error("drawing {0:?} was not found")]
    DrawingNotFound(String),
    #[error("entity {0:?} was not found")]
    EntityNotFound(String),
    #[error("revision_conflict: drawing changed since the edit session started")]
    RevisionConflict,
    #[error("recovery_required: concurrent source bytes were preserved at {recovery_path}")]
    RecoveryRequired { recovery_path: PathBuf },
    #[error("atomic exchange is unsupported on this platform")]
    AtomicExchangeUnsupported,
    #[error("layer {0:?} is missing, hidden, or locked")]
    LayerNotEditable(String),
    #[error("invalid entity: {0}")]
    InvalidEntity(String),
    #[error("edit introduces checker errors: {0}")]
    CheckFailed(String),
    #[error("history unavailable: {0}")]
    HistoryUnavailable(String),
    #[error("history entry is corrupt: {0}")]
    HistoryCorrupt(String),
    #[error("failed to read {path}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write {path}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
}

pub type EditResult<T> = Result<T, EditError>;

#[derive(Debug, Clone)]
pub struct SourceTransaction {
    project_root: PathBuf,
    operation: String,
}

impl SourceTransaction {
    pub fn new(project_root: impl AsRef<Path>, operation: impl Into<String>) -> EditResult<Self> {
        let project_root =
            fs::canonicalize(project_root.as_ref()).map_err(|source| EditError::Read {
                path: project_root.as_ref().to_path_buf(),
                source,
            })?;
        validate_generated_directory(&project_root, Path::new("build/.cad-transactions"))?;
        validate_generated_directory(&project_root, Path::new("build/.cad-recovery"))?;
        Ok(Self {
            project_root,
            operation: operation.into(),
        })
    }

    pub fn replace(
        &self,
        relative_path: impl AsRef<Path>,
        bytes: &[u8],
        expected_revision: &str,
        permissions: &PermissionsSnapshot,
    ) -> EditResult<()> {
        let relative_path = relative_path.as_ref();
        validate_source_relative_path(relative_path)?;
        validate_source_path_components(&self.project_root, relative_path)?;
        atomic_exchange_replace(
            &self.project_root.join(relative_path),
            bytes,
            expected_revision,
            permissions,
            Some((&self.project_root, &self.operation)),
        )
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct SourceTransactionJournal {
    target: String,
    staging: String,
    expected_revision: String,
    desired_revision: String,
    #[serde(default = "default_true")]
    expected_exists: bool,
    #[serde(default = "default_true")]
    desired_exists: bool,
    operation: String,
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EditOperation {
    Batch {
        operations: Vec<EditOperation>,
    },
    Create {
        entity: Value,
    },
    Replace {
        entity_id: String,
        entity: Value,
    },
    Translate {
        entity_id: String,
        delta: Point,
        duplicate: bool,
    },
    Delete {
        entity_id: String,
    },
    TranslateMany {
        entity_ids: Vec<String>,
        delta: Point,
        duplicate: bool,
    },
    DeleteMany {
        entity_ids: Vec<String>,
    },
    Rotate {
        entity_ids: Vec<String>,
        center: Point,
        angle_deg: f64,
    },
    Mirror {
        entity_ids: Vec<String>,
        axis_start: Point,
        axis_end: Point,
    },
    Offset {
        entity_ids: Vec<String>,
        distance: f64,
    },
    Trim {
        target_entity_id: String,
        cutter_entity_id: String,
        pick_point: Point,
    },
    Extend {
        target_entity_id: String,
        boundary_entity_id: String,
        pick_point: Point,
    },
    InsertBlock {
        block: String,
        layer: String,
        at: Point,
        rotation_deg: f64,
        scale: f64,
        #[serde(default)]
        entity_id: Option<String>,
    },
    UpdateHatch {
        entity_id: String,
        loops: Vec<Vec<Point>>,
        pattern: String,
        angle_deg: f64,
        scale: f64,
        #[serde(default)]
        fill: Option<String>,
    },
    UpdateLayout {
        layout: String,
        properties: Value,
    },
    UpdateBlockDefinition {
        block: String,
        properties: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrawingEditRequest {
    pub drawing: String,
    pub expected_revision: String,
    pub operation: EditOperation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DrawingEditResult {
    pub drawing: String,
    pub revision: String,
    pub entity_id: Option<String>,
    pub entity_ids: Vec<String>,
    pub operation: String,
    pub history_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrawingHistoryRequest {
    pub drawing: String,
    #[serde(default)]
    pub expected_files: Vec<HistoryFileRevision>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryFileRevision {
    pub relative_path: String,
    pub revision: String,
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryFileSnapshot {
    pub relative_path: String,
    pub exists: bool,
    pub before_revision: String,
    pub after_revision: String,
    pub before_snapshot: Option<String>,
    pub after_snapshot: Option<String>,
    pub before_permissions: Option<PermissionsSnapshot>,
    pub after_permissions: Option<PermissionsSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DrawingHistoryEntry {
    pub history_id: String,
    pub drawing: String,
    pub operation: String,
    pub timestamp: String,
    pub before_revision: String,
    pub after_revision: String,
    pub entity_ids: Vec<String>,
    pub scope: String,
    pub changed_files: Vec<String>,
    pub affected_files: Vec<String>,
    pub files: Vec<HistoryFileSnapshot>,
    pub before_manifest_revision: String,
    pub after_manifest_revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DrawingHistoryState {
    pub drawing: String,
    pub undo: Vec<DrawingHistoryEntry>,
    pub redo: Vec<DrawingHistoryEntry>,
    pub limit: usize,
    pub current_files: Vec<HistoryFileRevision>,
    pub context_blocked: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorDrawingState {
    pub drawing: String,
    pub revision: String,
    pub entities: Vec<Value>,
    pub text_styles: Vec<String>,
    pub dimension_styles: Vec<String>,
    pub pens: Vec<String>,
}

pub fn editor_state(project: &ProjectSource, drawing: &str) -> EditResult<EditorDrawingState> {
    let drawing_source = project
        .drawings
        .iter()
        .find(|candidate| candidate.name == drawing)
        .ok_or_else(|| EditError::DrawingNotFound(drawing.to_owned()))?;
    let path = entities_path(&project.root, drawing);
    let bytes = fs::read(&path).map_err(|source| EditError::Read {
        path: path.clone(),
        source,
    })?;
    Ok(EditorDrawingState {
        drawing: drawing.to_owned(),
        revision: revision(&bytes),
        entities: drawing_source
            .entities
            .iter()
            .map(|record| serde_json::to_value(&record.entity).expect("Entity is serializable"))
            .collect(),
        text_styles: project.styles.text_styles.keys().cloned().collect(),
        dimension_styles: project.styles.dimension_styles.keys().cloned().collect(),
        pens: project.styles.pens.keys().cloned().collect(),
    })
}

pub fn layout_revision(project_path: &Path, drawing: &str) -> EditResult<String> {
    let project = cad_model::load_project(project_path)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    let drawing_source = project
        .drawings
        .iter()
        .find(|candidate| candidate.name == drawing)
        .ok_or_else(|| EditError::DrawingNotFound(drawing.to_owned()))?;
    let path = project_path
        .join("drawings")
        .join(drawing)
        .join("layouts.toml");
    let bytes = if path.exists() {
        fs::read(&path).map_err(|source| EditError::Read { path, source })?
    } else {
        toml::to_string_pretty(&drawing_source.layouts)
            .map_err(|error| EditError::InvalidEntity(error.to_string()))?
            .into_bytes()
    };
    Ok(revision(&bytes))
}

pub fn block_definition_revision(project_path: &Path, block: &str) -> EditResult<String> {
    let path = project_path
        .join("blocks")
        .join(block)
        .join("definition.toml");
    let bytes = fs::read(&path).map_err(|source| EditError::Read {
        path: path.clone(),
        source,
    })?;
    Ok(revision(&bytes))
}

const HISTORY_LIMIT: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistoryIndex {
    #[serde(default)]
    undo: Vec<String>,
    #[serde(default)]
    redo: Vec<String>,
    #[serde(default = "history_limit")]
    limit: usize,
}

fn history_limit() -> usize {
    HISTORY_LIMIT
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PermissionsSnapshot {
    pub readonly: bool,
    #[serde(default)]
    pub unix_mode: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistoryMetadata {
    history_id: String,
    drawing: String,
    #[serde(default = "default_history_scope")]
    scope: String,
    operation: String,
    timestamp: String,
    entity_ids: Vec<String>,
    #[serde(default)]
    changed_files: Vec<String>,
    #[serde(default)]
    affected_files: Vec<String>,
    #[serde(default)]
    files: Vec<HistoryFileSnapshot>,
    #[serde(default)]
    before_manifest_revision: String,
    #[serde(default)]
    after_manifest_revision: String,
}

fn default_history_scope() -> String {
    "drawing".to_owned()
}

fn manifest_revision(files: &[HistoryFileRevision]) -> String {
    let mut canonical = files
        .iter()
        .map(|file| format!("{}\0{}\0{}", file.relative_path, file.exists, file.revision))
        .collect::<Vec<_>>();
    canonical.sort();
    revision(canonical.join("\n").as_bytes())
}

#[derive(Debug, Clone)]
pub struct HistoryFileInput {
    pub relative_path: String,
    pub exists: bool,
    pub bytes: Vec<u8>,
    pub permissions: Option<PermissionsSnapshot>,
}

pub struct HistoryStage {
    root: PathBuf,
    metadata: HistoryMetadata,
    committed: bool,
    prepared_after: BTreeSet<String>,
    journal_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum HistoryTransactionKind {
    Commit,
    Restore { undo: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct HistoryTransactionJournal {
    history_id: String,
    transaction: HistoryTransactionKind,
}

impl Drop for HistoryStage {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(self.entry_dir());
            if let Some(journal_path) = self.journal_path.as_ref() {
                let _ = fs::remove_file(journal_path);
            }
        }
    }
}

impl HistoryStage {
    fn entry_dir(&self) -> PathBuf {
        self.root.join("entries").join(&self.metadata.history_id)
    }

    fn metadata_path(&self) -> PathBuf {
        self.entry_dir().join("metadata.json")
    }

    pub fn history_id(&self) -> &str {
        &self.metadata.history_id
    }

    pub fn prepare_after(&mut self, after_files: &[HistoryFileInput]) -> EditResult<()> {
        for file in after_files {
            validate_history_relative_path(&file.relative_path)?;
            let output_path = self.entry_dir().join("after").join(&file.relative_path);
            let Some(metadata_file) = self
                .metadata
                .files
                .iter_mut()
                .find(|candidate| candidate.relative_path == file.relative_path)
            else {
                return Err(EditError::HistoryUnavailable(format!(
                    "after file {:?} was not staged",
                    file.relative_path
                )));
            };
            metadata_file.exists = file.exists;
            metadata_file.after_revision = revision(&file.bytes);
            metadata_file.after_permissions = file.permissions.clone();
            self.prepared_after.insert(file.relative_path.clone());
            if file.exists {
                write_history_file(&output_path, &file.bytes)?;
                metadata_file.after_snapshot = Some(format!("after/{}", file.relative_path));
            } else {
                metadata_file.after_snapshot = None;
            }
        }
        if self.prepared_after.len() != self.metadata.files.len() {
            return Err(EditError::HistoryUnavailable(
                "history after manifest is incomplete".to_owned(),
            ));
        }
        self.metadata.after_manifest_revision = manifest_revision(
            &self
                .metadata
                .files
                .iter()
                .map(|file| HistoryFileRevision {
                    relative_path: file.relative_path.clone(),
                    revision: file.after_revision.clone(),
                    exists: file.after_snapshot.is_some(),
                })
                .collect::<Vec<_>>(),
        );
        let serialized = serde_json::to_vec_pretty(&self.metadata)
            .map_err(|error| EditError::HistoryUnavailable(error.to_string()))?;
        write_history_file(&self.metadata_path(), &serialized)?;
        Ok(())
    }

    pub fn prepare_commit_journal(&mut self) -> EditResult<()> {
        if self.prepared_after.len() != self.metadata.files.len() {
            return Err(EditError::HistoryUnavailable(
                "cannot journal incomplete history manifest".to_owned(),
            ));
        }
        let journal_path = write_history_transaction_journal(
            &self.root,
            &HistoryTransactionJournal {
                history_id: self.metadata.history_id.clone(),
                transaction: HistoryTransactionKind::Commit,
            },
        )?;
        self.journal_path = Some(journal_path);
        Ok(())
    }

    pub fn abort(self) {
        if let Some(journal_path) = self.journal_path.as_ref() {
            let _ = fs::remove_file(journal_path);
        }
        let _ = fs::remove_dir_all(self.entry_dir());
    }

    pub fn preserve_for_recovery(mut self) {
        self.committed = true;
    }
}

fn write_history_file(path: &Path, bytes: &[u8]) -> EditResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| EditError::HistoryUnavailable("history file has no parent".to_owned()))?;
    fs::create_dir_all(parent).map_err(|source| EditError::Write {
        path: parent.to_path_buf(),
        source,
    })?;
    let temp = tempfile::NamedTempFile::new_in(parent).map_err(|source| EditError::Write {
        path: parent.to_path_buf(),
        source,
    })?;
    fs::write(temp.path(), bytes).map_err(|source| EditError::Write {
        path: temp.path().to_path_buf(),
        source,
    })?;
    temp.as_file()
        .sync_all()
        .map_err(|source| EditError::Write {
            path: temp.path().to_path_buf(),
            source,
        })?;
    fs::rename(temp.path(), path).map_err(|source| EditError::Write {
        path: path.to_path_buf(),
        source,
    })?;
    sync_directory(parent)
}

fn write_history_transaction_journal(
    root: &Path,
    journal: &HistoryTransactionJournal,
) -> EditResult<PathBuf> {
    let transactions = root.join("transactions");
    fs::create_dir_all(&transactions).map_err(|source| EditError::Write {
        path: transactions.clone(),
        source,
    })?;
    let path = transactions.join(format!("{}.json", journal.history_id));
    let bytes = serde_json::to_vec_pretty(journal)
        .map_err(|error| EditError::HistoryUnavailable(error.to_string()))?;
    write_history_file(&path, &bytes)?;
    Ok(path)
}

fn history_root(project_path: &Path, drawing: &str) -> EditResult<PathBuf> {
    validate_drawing_name(drawing)?;
    let root = fs::canonicalize(project_path).map_err(|source| EditError::Read {
        path: project_path.to_path_buf(),
        source,
    })?;
    for relative in [
        Path::new("build"),
        Path::new("build/.cad-history"),
        Path::new("build/.cad-history/entries"),
        Path::new("build/.cad-history/transactions"),
    ] {
        let path = root.join(relative);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(EditError::HistoryUnavailable(format!(
                    "history path contains a symlink: {}",
                    path.display()
                )));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(EditError::HistoryUnavailable(format!(
                    "history path is not a directory: {}",
                    path.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(source) => {
                return Err(EditError::Read { path, source });
            }
        }
    }
    Ok(root.join("build/.cad-history"))
}

fn validate_drawing_name(drawing: &str) -> EditResult<()> {
    let mut components = Path::new(drawing).components();
    if drawing.is_empty()
        || components.next().is_none()
        || components.next().is_some()
        || !Path::new(drawing)
            .components()
            .all(|component| matches!(component, std::path::Component::Normal(_)))
    {
        return Err(EditError::HistoryUnavailable(
            "drawing name is not a safe history directory".to_owned(),
        ));
    }
    Ok(())
}

fn capture_permissions(permissions: &fs::Permissions) -> PermissionsSnapshot {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        PermissionsSnapshot {
            readonly: permissions.readonly(),
            unix_mode: Some(permissions.mode()),
        }
    }
    #[cfg(not(unix))]
    {
        PermissionsSnapshot {
            readonly: permissions.readonly(),
            unix_mode: None,
        }
    }
}

/// Captures the portable permission fields used by atomic source publishing.
#[must_use]
pub fn permissions_snapshot(permissions: &fs::Permissions) -> PermissionsSnapshot {
    capture_permissions(permissions)
}

fn apply_permissions(path: &Path, snapshot: &PermissionsSnapshot) -> EditResult<()> {
    #[cfg(unix)]
    if let Some(mode) = snapshot.unix_mode {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|source| {
            EditError::Write {
                path: path.to_path_buf(),
                source,
            }
        })?;
        return Ok(());
    }

    let mut permissions = fs::metadata(path)
        .map_err(|source| EditError::Read {
            path: path.to_path_buf(),
            source,
        })?
        .permissions();
    permissions.set_readonly(snapshot.readonly);
    fs::set_permissions(path, permissions).map_err(|source| EditError::Write {
        path: path.to_path_buf(),
        source,
    })
}

fn read_history_index(root: &Path) -> EditResult<HistoryIndex> {
    let path = root.join("index.json");
    if !path.exists() {
        return Ok(HistoryIndex {
            undo: Vec::new(),
            redo: Vec::new(),
            limit: HISTORY_LIMIT,
        });
    }
    let bytes = fs::read(&path).map_err(|source| EditError::Read {
        path: path.clone(),
        source,
    })?;
    let mut index: HistoryIndex = serde_json::from_slice(&bytes)
        .map_err(|error| EditError::HistoryCorrupt(format!("{}: {error}", path.display())))?;
    if index.limit == 0 || index.limit > HISTORY_LIMIT {
        index.limit = HISTORY_LIMIT;
    }
    Ok(index)
}

fn write_history_index(root: &Path, index: &HistoryIndex) -> EditResult<()> {
    fs::create_dir_all(root).map_err(|source| EditError::Write {
        path: root.to_path_buf(),
        source,
    })?;
    let path = root.join("index.json");
    let bytes = serde_json::to_vec_pretty(index)
        .map_err(|error| EditError::HistoryUnavailable(error.to_string()))?;
    write_history_file(&path, &bytes)
}

fn history_entry(root: &Path, history_id: &str) -> EditResult<HistoryMetadata> {
    if history_id.is_empty()
        || Path::new(history_id)
            .components()
            .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(EditError::HistoryCorrupt("invalid history id".to_owned()));
    }
    let path = root.join("entries").join(history_id).join("metadata.json");
    let bytes = fs::read(&path)
        .map_err(|source| EditError::HistoryCorrupt(format!("{}: {source}", path.display())))?;
    let metadata: HistoryMetadata = serde_json::from_slice(&bytes)
        .map_err(|error| EditError::HistoryCorrupt(format!("{}: {error}", path.display())))?;
    if metadata.history_id != history_id {
        return Err(EditError::HistoryCorrupt(format!(
            "{} has mismatched history id",
            path.display()
        )));
    }
    Ok(metadata)
}

fn public_history_entry(root: &Path, history_id: &str) -> EditResult<DrawingHistoryEntry> {
    let metadata = history_entry(root, history_id)?;
    let first = metadata.files.first();
    Ok(DrawingHistoryEntry {
        history_id: metadata.history_id,
        drawing: metadata.drawing,
        operation: metadata.operation,
        timestamp: metadata.timestamp,
        before_revision: first
            .map(|file| file.before_revision.clone())
            .unwrap_or_default(),
        after_revision: first
            .map(|file| file.after_revision.clone())
            .unwrap_or_default(),
        entity_ids: metadata.entity_ids,
        scope: metadata.scope,
        changed_files: metadata.changed_files,
        affected_files: metadata.affected_files,
        files: metadata.files,
        before_manifest_revision: metadata.before_manifest_revision,
        after_manifest_revision: metadata.after_manifest_revision,
    })
}

pub fn list_drawing_history(project_path: &Path, drawing: &str) -> EditResult<DrawingHistoryState> {
    let root = history_root(project_path, drawing)?;
    let index = read_history_index(&root)?;
    let undo = index
        .undo
        .iter()
        .map(|history_id| public_history_entry(&root, history_id))
        .filter_map(|entry| match entry {
            Ok(entry) if entry.scope == "project" || entry.drawing == drawing => Some(Ok(entry)),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<EditResult<Vec<_>>>()?;
    let redo = index
        .redo
        .iter()
        .rev()
        .map(|history_id| public_history_entry(&root, history_id))
        .filter(|entry| match entry {
            Ok(entry) => entry.scope == "project" || entry.drawing == drawing,
            Err(_) => true,
        })
        .collect::<EditResult<Vec<_>>>()?;
    let current_files = current_history_files(project_path, drawing)?;
    let context_blocked = [index.undo.last(), index.redo.last()]
        .into_iter()
        .flatten()
        .filter_map(|id| public_history_entry(&root, id).ok())
        .find(|entry| entry.scope != "project" && entry.drawing != drawing)
        .map(|entry| format!("switch to drawing {:?}", entry.drawing));
    Ok(DrawingHistoryState {
        drawing: drawing.to_owned(),
        undo,
        redo,
        limit: index.limit,
        current_files,
        context_blocked,
    })
}

pub fn clear_drawing_history(project_path: &Path, drawing: &str) -> EditResult<()> {
    with_history_lock(|| clear_drawing_history_locked(project_path, drawing))
}

fn clear_drawing_history_locked(project_path: &Path, drawing: &str) -> EditResult<()> {
    let root = history_root(project_path, drawing)?;
    if !root.exists() {
        return Ok(());
    }
    let index = read_history_index(&root)?;
    let mut kept = HistoryIndex {
        undo: Vec::new(),
        redo: Vec::new(),
        limit: index.limit,
    };
    for id in index.undo.iter().chain(index.redo.iter()) {
        let remove = history_entry(&root, id)
            .map(|entry| entry.drawing == drawing && entry.scope != "project")
            .unwrap_or(false);
        if remove {
            let _ = fs::remove_dir_all(root.join("entries").join(id));
        } else if index.undo.contains(id) {
            kept.undo.push(id.clone());
        } else {
            kept.redo.push(id.clone());
        }
    }
    write_history_index(&root, &kept)?;
    Ok(())
}

/// Serializes all entity, comment, and layer history transactions in one process.
/// Callers must stage and publish their source files inside this closure.
pub fn with_history_lock<T, F>(operation: F) -> EditResult<T>
where
    F: FnOnce() -> EditResult<T>,
{
    let _guard = edit_lock()
        .lock()
        .map_err(|_| EditError::InvalidEntity("drawing edit lock is poisoned".to_owned()))?;
    operation()
}

pub fn read_history_file(project_path: &Path, relative_path: &str) -> EditResult<HistoryFileInput> {
    let path = safe_history_path(project_path, relative_path)?;
    match fs::read(&path) {
        Ok(bytes) => {
            let permissions = fs::metadata(&path)
                .map_err(|source| EditError::Read {
                    path: path.clone(),
                    source,
                })?
                .permissions();
            Ok(HistoryFileInput {
                relative_path: relative_path.to_owned(),
                exists: true,
                bytes,
                permissions: Some(capture_permissions(&permissions)),
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(HistoryFileInput {
            relative_path: relative_path.to_owned(),
            exists: false,
            bytes: Vec::new(),
            permissions: None,
        }),
        Err(source) => Err(EditError::Read { path, source }),
    }
}

fn safe_history_path(project_path: &Path, relative_path: &str) -> EditResult<PathBuf> {
    validate_history_relative_path(relative_path)?;
    let root = fs::canonicalize(project_path).map_err(|source| EditError::Read {
        path: project_path.to_path_buf(),
        source,
    })?;
    let path = Path::new(relative_path);
    validate_source_path_components(&root, path)?;
    Ok(root.join(path))
}

fn validate_history_relative_path(relative_path: &str) -> EditResult<()> {
    let path = Path::new(relative_path);
    if relative_path.is_empty()
        || path.is_absolute()
        || cad_model::classify_project_source_path(path).is_none()
        || relative_path == "build/.cad-history"
        || relative_path.starts_with("build/.cad-history/")
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::RootDir
            )
        })
    {
        return Err(EditError::HistoryUnavailable(format!(
            "unsafe history file path {:?}",
            relative_path
        )));
    }
    Ok(())
}

pub fn stage_history_transaction(
    project_path: &Path,
    drawing: &str,
    scope: &str,
    operation: &str,
    entity_ids: &[String],
    before_files: &[HistoryFileInput],
) -> EditResult<HistoryStage> {
    let root = history_root(project_path, drawing)?;
    let history_id = format!("hist_{}", Ulid::new());
    let entry_dir = root.join("entries").join(&history_id);
    fs::create_dir_all(entry_dir.join("before")).map_err(|source| EditError::Write {
        path: entry_dir.clone(),
        source,
    })?;
    let mut files = Vec::new();
    for file in before_files {
        validate_history_relative_path(&file.relative_path)?;
        if file.exists {
            let before_path = entry_dir.join("before").join(&file.relative_path);
            write_history_file(&before_path, &file.bytes)?;
        }
        files.push(HistoryFileSnapshot {
            relative_path: file.relative_path.clone(),
            exists: file.exists,
            before_revision: revision(&file.bytes),
            after_revision: String::new(),
            before_snapshot: file
                .exists
                .then(|| format!("before/{}", file.relative_path)),
            after_snapshot: None,
            before_permissions: file.permissions.clone(),
            after_permissions: None,
        });
    }
    let metadata = HistoryMetadata {
        history_id,
        drawing: drawing.to_owned(),
        scope: scope.to_owned(),
        operation: operation.to_owned(),
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs().to_string())
            .unwrap_or_else(|_| "0".to_owned()),
        entity_ids: entity_ids.to_vec(),
        changed_files: before_files
            .iter()
            .map(|file| file.relative_path.clone())
            .collect(),
        affected_files: before_files
            .iter()
            .map(|file| file.relative_path.clone())
            .collect(),
        before_manifest_revision: manifest_revision(
            &files
                .iter()
                .map(|file| HistoryFileRevision {
                    relative_path: file.relative_path.clone(),
                    revision: file.before_revision.clone(),
                    exists: file.exists,
                })
                .collect::<Vec<_>>(),
        ),
        after_manifest_revision: String::new(),
        files,
    };
    Ok(HistoryStage {
        root,
        metadata,
        committed: false,
        prepared_after: BTreeSet::new(),
        journal_path: None,
    })
}

pub fn commit_history_stage(stage: &mut HistoryStage) -> EditResult<String> {
    if stage.prepared_after.len() != stage.metadata.files.len() {
        return Err(EditError::HistoryUnavailable(
            "cannot commit incomplete history manifest".to_owned(),
        ));
    }
    let mut index = read_history_index(&stage.root)?;
    if index.limit == 0 || index.limit > HISTORY_LIMIT {
        index.limit = HISTORY_LIMIT;
    }
    let mut removed_entries = Vec::new();
    for history_id in index.redo.drain(..) {
        removed_entries.push(history_id);
    }
    let history_id = stage.metadata.history_id.clone();
    index.undo.push(history_id.clone());
    while index.undo.len() + index.redo.len() > index.limit {
        if index.undo.is_empty() {
            if let Some(oldest) = index.redo.first().cloned() {
                index.redo.remove(0);
                removed_entries.push(oldest);
            }
        } else {
            let oldest = index.undo.remove(0);
            removed_entries.push(oldest);
        }
    }
    write_history_index(&stage.root, &index)?;
    stage.committed = true;
    for history_id in removed_entries {
        let _ = fs::remove_dir_all(stage.root.join("entries").join(history_id));
    }
    if let Some(journal_path) = stage.journal_path.take()
        && fs::remove_file(&journal_path).is_ok()
    {
        let _ = sync_directory(&stage.root.join("transactions"));
    }
    Ok(history_id)
}

fn current_history_files(
    project_path: &Path,
    drawing: &str,
) -> EditResult<Vec<HistoryFileRevision>> {
    let mut paths = vec![
        format!("drawings/{drawing}/entities.ndjson"),
        format!("comments/{drawing}.ndjson"),
        "rules/layers.toml".to_owned(),
        format!("drawings/{drawing}/layouts.toml"),
    ];
    let blocks_dir = project_path.join("blocks");
    if blocks_dir.exists() {
        let mut block_dirs = fs::read_dir(&blocks_dir)
            .map_err(|source| EditError::Read {
                path: blocks_dir.clone(),
                source,
            })?
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
            .collect::<Vec<_>>();
        block_dirs.sort_by_key(|entry| entry.file_name());
        for entry in block_dirs {
            let id = entry.file_name().to_string_lossy().into_owned();
            paths.push(format!("blocks/{id}/definition.toml"));
            paths.push(format!("blocks/{id}/entities.ndjson"));
        }
    }
    paths
        .into_iter()
        .map(|relative_path| {
            let input = read_history_file(project_path, &relative_path)?;
            Ok(HistoryFileRevision {
                relative_path,
                revision: revision(&input.bytes),
                exists: input.exists,
            })
        })
        .collect()
}

fn publish_history_file(
    project_path: &Path,
    target: &HistoryFileInput,
    current: &HistoryFileInput,
) -> EditResult<()> {
    let observed = read_history_file(project_path, &target.relative_path)?;
    if observed.exists != current.exists || revision(&current.bytes) != revision(&observed.bytes) {
        return Err(EditError::RevisionConflict);
    }
    let path = safe_history_path(project_path, &target.relative_path)?;
    if !target.exists {
        if observed.exists {
            atomic_delete(
                project_path,
                &target.relative_path,
                &revision(&current.bytes),
            )?;
        }
        return Ok(());
    }
    if observed.exists {
        let permissions = target.permissions.as_ref().ok_or_else(|| {
            EditError::HistoryCorrupt(format!(
                "restored file {} has no permission snapshot",
                target.relative_path
            ))
        })?;
        SourceTransaction::new(project_path, "history.restore")?.replace(
            &target.relative_path,
            &target.bytes,
            &revision(&current.bytes),
            permissions,
        )
    } else {
        atomic_create(&path, &target.bytes, target.permissions.as_ref())
    }
}

fn restore_history_file_if_unchanged(
    project_path: &Path,
    original: &HistoryFileInput,
    published: &HistoryFileInput,
) -> EditResult<()> {
    match publish_history_file(project_path, original, published) {
        Ok(()) | Err(EditError::RevisionConflict) => Ok(()),
        Err(error) => Err(error),
    }
}

fn restore_history_entry(
    project_path: &Path,
    request: &DrawingHistoryRequest,
    undo: bool,
) -> EditResult<DrawingEditResult> {
    let _guard = edit_lock()
        .lock()
        .map_err(|_| EditError::InvalidEntity("drawing edit lock is poisoned".to_owned()))?;
    let root = history_root(project_path, &request.drawing)?;
    let mut index = read_history_index(&root)?;
    let history_id = if undo {
        index.undo.last().cloned()
    } else {
        index.redo.last().cloned()
    }
    .ok_or_else(|| EditError::HistoryUnavailable("no history entry is available".to_owned()))?;
    let metadata = history_entry(&root, &history_id)?;
    if metadata.scope != "project" && metadata.drawing != request.drawing {
        return Err(EditError::HistoryUnavailable(format!(
            "switch to drawing {:?} before restoring history",
            metadata.drawing
        )));
    }
    let mut current_files = Vec::new();
    let mut target_files = Vec::new();
    for file in &metadata.files {
        let current = read_history_file(project_path, &file.relative_path)?;
        let expected_revision = if undo {
            &file.after_revision
        } else {
            &file.before_revision
        };
        if current.exists
            != (if undo {
                file.after_snapshot.is_some()
            } else {
                file.before_snapshot.is_some()
            })
            || revision(&current.bytes) != *expected_revision
        {
            return Err(EditError::RevisionConflict);
        }
        if !request.expected_files.is_empty() {
            let expected = request
                .expected_files
                .iter()
                .find(|candidate| candidate.relative_path == file.relative_path)
                .ok_or(EditError::RevisionConflict)?;
            if expected.exists != current.exists || expected.revision != revision(&current.bytes) {
                return Err(EditError::RevisionConflict);
            }
        }
        let snapshot_rel = if undo {
            file.before_snapshot.as_ref()
        } else {
            file.after_snapshot.as_ref()
        };
        let snapshot_bytes = match snapshot_rel {
            Some(relative) => {
                let snapshot = root.join("entries").join(&history_id).join(relative);
                fs::read(&snapshot).map_err(|source| {
                    EditError::HistoryCorrupt(format!("{}: {source}", snapshot.display()))
                })?
            }
            None => Vec::new(),
        };
        let next_revision = if undo {
            &file.before_revision
        } else {
            &file.after_revision
        };
        if revision(&snapshot_bytes) != *next_revision {
            return Err(EditError::HistoryCorrupt(format!(
                "snapshot revision mismatch for {}",
                file.relative_path
            )));
        }
        current_files.push(current);
        target_files.push(HistoryFileInput {
            relative_path: file.relative_path.clone(),
            exists: snapshot_rel.is_some(),
            bytes: snapshot_bytes,
            permissions: if undo {
                file.before_permissions.clone()
            } else {
                file.after_permissions.clone()
            },
        });
    }
    let restore_journal = write_history_transaction_journal(
        &root,
        &HistoryTransactionJournal {
            history_id: history_id.clone(),
            transaction: HistoryTransactionKind::Restore { undo },
        },
    )?;
    for target in &target_files {
        let current = current_files
            .iter()
            .find(|current| current.relative_path == target.relative_path)
            .expect("current file");
        if let Err(error) = publish_history_file(project_path, target, current) {
            let mut rollback_error = None;
            for (rollback, published) in current_files.iter().zip(&target_files) {
                if let Err(error) =
                    restore_history_file_if_unchanged(project_path, rollback, published)
                {
                    rollback_error = Some(error);
                }
            }
            if let Some(rollback_error) = rollback_error {
                return Err(rollback_error);
            }
            fs::remove_file(&restore_journal).map_err(|source| EditError::Write {
                path: restore_journal,
                source,
            })?;
            return Err(error);
        }
    }
    if undo {
        index.undo.pop();
        index.redo.push(history_id.clone());
    } else {
        index.redo.pop();
        index.undo.push(history_id.clone());
    }
    if let Err(error) = write_history_index(&root, &index) {
        let mut rollback_error = None;
        for (current, published) in current_files.iter().zip(&target_files) {
            if let Err(error) = restore_history_file_if_unchanged(project_path, current, published)
            {
                rollback_error = Some(error);
            }
        }
        if let Some(rollback_error) = rollback_error {
            return Err(rollback_error);
        }
        fs::remove_file(&restore_journal).map_err(|source| EditError::Write {
            path: restore_journal,
            source,
        })?;
        return Err(error);
    }
    if fs::remove_file(&restore_journal).is_ok() {
        let _ = sync_directory(&root.join("transactions"));
    }
    let next_revision = target_files
        .iter()
        .find(|file| file.relative_path == format!("drawings/{}/entities.ndjson", request.drawing))
        .map(|file| revision(&file.bytes))
        .unwrap_or_default();
    Ok(DrawingEditResult {
        drawing: request.drawing.clone(),
        revision: next_revision,
        entity_id: metadata.entity_ids.first().cloned(),
        entity_ids: metadata.entity_ids,
        operation: if undo { "undo" } else { "redo" }.to_owned(),
        history_id: Some(history_id),
    })
}

pub fn undo_drawing_edit(
    project_path: &Path,
    request: &DrawingHistoryRequest,
) -> EditResult<DrawingEditResult> {
    restore_history_entry(project_path, request, true)
}

pub fn redo_drawing_edit(
    project_path: &Path,
    request: &DrawingHistoryRequest,
) -> EditResult<DrawingEditResult> {
    restore_history_entry(project_path, request, false)
}

pub fn apply_edit(
    project_path: &Path,
    request: &DrawingEditRequest,
) -> EditResult<DrawingEditResult> {
    let _guard = edit_lock()
        .lock()
        .map_err(|_| EditError::InvalidEntity("drawing edit lock is poisoned".to_owned()))?;
    match &request.operation {
        EditOperation::UpdateLayout { .. } => {
            return apply_layout_edit_locked(project_path, request);
        }
        EditOperation::UpdateBlockDefinition { .. } => {
            return apply_block_definition_edit_locked(project_path, request);
        }
        _ => {}
    }
    let mut project = cad_model::load_project(project_path)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    let drawing_index = project
        .drawings
        .iter()
        .position(|drawing| drawing.name == request.drawing)
        .ok_or_else(|| EditError::DrawingNotFound(request.drawing.clone()))?;
    let path = entities_path(project_path, &request.drawing);
    let bytes = fs::read(&path).map_err(|source| EditError::Read {
        path: path.clone(),
        source,
    })?;
    if revision(&bytes) != request.expected_revision {
        return Err(EditError::RevisionConflict);
    }
    let text = String::from_utf8(bytes.clone())
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    let relative_path = format!("drawings/{}/entities.ndjson", request.drawing);
    let before_input = HistoryFileInput {
        relative_path: relative_path.clone(),
        exists: true,
        bytes: bytes.clone(),
        permissions: fs::metadata(&path)
            .map_err(|source| EditError::Read {
                path: path.clone(),
                source,
            })
            .map(|metadata| capture_permissions(&metadata.permissions()))
            .ok(),
    };
    let mut history_stage = stage_history_transaction(
        project_path,
        &request.drawing,
        "drawing",
        "pending",
        &[],
        &[before_input],
    )?;
    let mut raw_lines = split_raw_lines(&text);
    let mut entities = project.drawings[drawing_index]
        .entities
        .iter()
        .map(|record| record.entity.clone())
        .collect::<Vec<_>>();
    let baseline_errors = checker_errors(&project);

    let result = match apply_operation(&request.operation, &project, &mut entities, &mut raw_lines)
    {
        Ok(result) => result,
        Err(error) => {
            history_stage.abort();
            return Err(error);
        }
    };

    project.drawings[drawing_index].entities = entities
        .into_iter()
        .enumerate()
        .map(|(index, entity)| EntityRecord {
            line: index + 1,
            entity,
        })
        .collect();
    let candidate_errors = checker_errors(&project);
    let new_errors = candidate_errors
        .difference(&baseline_errors)
        .cloned()
        .collect::<Vec<_>>();
    if !new_errors.is_empty() {
        history_stage.abort();
        return Err(EditError::CheckFailed(new_errors.join("; ")));
    }

    let output = render_raw_lines(&raw_lines);
    let permissions = match fs::metadata(&path) {
        Ok(metadata) => capture_permissions(&metadata.permissions()),
        Err(source) => {
            history_stage.abort();
            return Err(EditError::Read {
                path: path.clone(),
                source,
            });
        }
    };
    let after_revision = revision(output.as_bytes());
    history_stage.metadata.operation = result.operation.to_owned();
    history_stage.metadata.entity_ids = result.entity_ids.clone();
    if let Err(error) = history_stage.prepare_after(&[HistoryFileInput {
        relative_path,
        exists: true,
        bytes: output.as_bytes().to_vec(),
        permissions: Some(permissions.clone()),
    }]) {
        history_stage.abort();
        return Err(error);
    }
    if let Err(error) = history_stage.prepare_commit_journal() {
        history_stage.abort();
        return Err(error);
    }
    if let Err(error) = atomic_replace(&path, output.as_bytes(), &request.expected_revision) {
        history_stage.abort();
        return Err(error);
    }
    let history_id = match commit_history_stage(&mut history_stage) {
        Ok(history_id) => history_id,
        Err(error) => {
            match atomic_replace_with_permissions(&path, &bytes, &after_revision, &permissions) {
                Ok(()) => {
                    history_stage.abort();
                    return Err(error);
                }
                Err(rollback_error) => {
                    history_stage.preserve_for_recovery();
                    return Err(rollback_error);
                }
            }
        }
    };
    Ok(DrawingEditResult {
        drawing: request.drawing.clone(),
        revision: after_revision,
        entity_id: result.entity_ids.first().cloned(),
        entity_ids: result.entity_ids,
        operation: result.operation.to_owned(),
        history_id: Some(history_id),
    })
}

fn apply_layout_edit_locked(
    project_path: &Path,
    request: &DrawingEditRequest,
) -> EditResult<DrawingEditResult> {
    let mut project = cad_model::load_project(project_path)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    let drawing_index = project
        .drawings
        .iter()
        .position(|drawing| drawing.name == request.drawing)
        .ok_or_else(|| EditError::DrawingNotFound(request.drawing.clone()))?;
    let path = project_path
        .join("drawings")
        .join(&request.drawing)
        .join("layouts.toml");
    let original_exists = path.exists();
    let original = if original_exists {
        fs::read(&path).map_err(|source| EditError::Read {
            path: path.clone(),
            source,
        })?
    } else {
        toml::to_string_pretty(&project.drawings[drawing_index].layouts)
            .map_err(|error| EditError::InvalidEntity(error.to_string()))?
            .into_bytes()
    };
    let layout_revision = revision(&original);
    if request.expected_revision != layout_revision {
        return Err(EditError::RevisionConflict);
    }
    let mut layouts = if original_exists {
        let text = String::from_utf8(original.clone())
            .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
        toml::from_str::<cad_model::LayoutsConfig>(&text)
            .map_err(|error| EditError::InvalidEntity(error.to_string()))?
    } else {
        project.drawings[drawing_index].layouts.clone()
    };
    let EditOperation::UpdateLayout { layout, properties } = &request.operation else {
        unreachable!();
    };
    if layout == "active_layout" {
        layouts.active_layout = properties
            .as_str()
            .ok_or_else(|| EditError::InvalidEntity("active layout must be a string".to_owned()))?
            .to_owned();
    } else {
        let current = layouts
            .layouts
            .get(layout)
            .ok_or_else(|| EditError::InvalidEntity(format!("layout {layout:?} was not found")))?;
        let mut value = serde_json::to_value(current)
            .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
        let object = properties.as_object().ok_or_else(|| {
            EditError::InvalidEntity("layout properties must be an object".to_owned())
        })?;
        let target = value
            .as_object_mut()
            .expect("layout config serializes as an object");
        for (key, property) in object {
            target.insert(key.clone(), property.clone());
        }
        layouts.layouts.insert(
            layout.clone(),
            serde_json::from_value(value)
                .map_err(|error| EditError::InvalidEntity(error.to_string()))?,
        );
    }
    let baseline_errors = checker_errors(&project);
    project.drawings[drawing_index].layouts = layouts.clone();
    let new_errors = checker_errors(&project)
        .difference(&baseline_errors)
        .cloned()
        .collect::<Vec<_>>();
    if !new_errors.is_empty() {
        return Err(EditError::CheckFailed(new_errors.join("; ")));
    }
    let output = toml::to_string_pretty(&layouts)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?
        .into_bytes();
    let permissions = fs::metadata(&path)
        .ok()
        .map(|metadata| capture_permissions(&metadata.permissions()));
    let relative_path = format!("drawings/{}/layouts.toml", request.drawing);
    let before_input = HistoryFileInput {
        relative_path: relative_path.clone(),
        exists: original_exists,
        bytes: original.clone(),
        permissions: permissions.clone(),
    };
    let mut history_stage = stage_history_transaction(
        project_path,
        &request.drawing,
        "drawing",
        "update_layout",
        &[],
        &[before_input],
    )?;
    let after_revision = revision(&output);
    history_stage.metadata.entity_ids.clear();
    if let Err(error) = history_stage.prepare_after(&[HistoryFileInput {
        relative_path,
        exists: true,
        bytes: output.clone(),
        permissions: permissions.clone(),
    }]) {
        history_stage.abort();
        return Err(error);
    }
    if let Err(error) = history_stage.prepare_commit_journal() {
        history_stage.abort();
        return Err(error);
    }
    let publish_result = if original_exists {
        atomic_replace(&path, &output, &layout_revision)
    } else {
        atomic_create(&path, &output, permissions.as_ref())
    };
    if let Err(error) = publish_result {
        history_stage.abort();
        return Err(error);
    }
    let history_id = match commit_history_stage(&mut history_stage) {
        Ok(history_id) => history_id,
        Err(error) => {
            let rollback = if original_exists {
                atomic_replace_with_permissions(
                    &path,
                    &original,
                    &after_revision,
                    permissions.as_ref().unwrap_or(&PermissionsSnapshot {
                        readonly: false,
                        unix_mode: None,
                    }),
                )
            } else {
                atomic_delete(
                    project_path,
                    &format!("drawings/{}/layouts.toml", request.drawing),
                    &after_revision,
                )
            };
            match rollback {
                Ok(()) => {
                    history_stage.abort();
                    return Err(error);
                }
                Err(rollback_error) => {
                    history_stage.preserve_for_recovery();
                    return Err(rollback_error);
                }
            }
        }
    };
    Ok(DrawingEditResult {
        drawing: request.drawing.clone(),
        revision: after_revision,
        entity_id: None,
        entity_ids: Vec::new(),
        operation: "update_layout".to_owned(),
        history_id: Some(history_id),
    })
}

fn apply_block_definition_edit_locked(
    project_path: &Path,
    request: &DrawingEditRequest,
) -> EditResult<DrawingEditResult> {
    let mut project = cad_model::load_project(project_path)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    let EditOperation::UpdateBlockDefinition { block, properties } = &request.operation else {
        unreachable!();
    };
    if !project.blocks.contains_key(block) {
        return Err(EditError::InvalidEntity(format!(
            "block definition {block:?} was not found"
        )));
    }
    let path = project_path
        .join("blocks")
        .join(block)
        .join("definition.toml");
    let original = fs::read(&path).map_err(|source| EditError::Read {
        path: path.clone(),
        source,
    })?;
    let before_revision = revision(&original);
    if request.expected_revision != before_revision {
        return Err(EditError::RevisionConflict);
    }
    let text = String::from_utf8(original.clone())
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    let mut config: cad_model::BlockDefinitionConfig =
        toml::from_str(&text).map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    let object = properties
        .as_object()
        .ok_or_else(|| EditError::InvalidEntity("block properties must be an object".to_owned()))?;
    let mut value = serde_json::to_value(&config)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    let target = value
        .as_object_mut()
        .expect("block config serializes as an object");
    for (key, property) in object {
        target.insert(key.clone(), property.clone());
    }
    config = serde_json::from_value(value)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    let baseline_errors = checker_errors(&project);
    if let Some(definition) = project.blocks.get_mut(block) {
        definition.config = config.clone();
    }
    let new_errors = checker_errors(&project)
        .difference(&baseline_errors)
        .cloned()
        .collect::<Vec<_>>();
    if !new_errors.is_empty() {
        return Err(EditError::CheckFailed(new_errors.join("; ")));
    }
    let output = toml::to_string_pretty(&config)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?
        .into_bytes();
    let permissions = fs::metadata(&path)
        .ok()
        .map(|metadata| capture_permissions(&metadata.permissions()));
    let relative_path = format!("blocks/{block}/definition.toml");
    let mut history_stage = stage_history_transaction(
        project_path,
        &request.drawing,
        "project",
        "update_block_definition",
        &[],
        &[HistoryFileInput {
            relative_path: relative_path.clone(),
            exists: true,
            bytes: original.clone(),
            permissions: permissions.clone(),
        }],
    )?;
    let after_revision = revision(&output);
    if let Err(error) = history_stage.prepare_after(&[HistoryFileInput {
        relative_path,
        exists: true,
        bytes: output.clone(),
        permissions: permissions.clone(),
    }]) {
        history_stage.abort();
        return Err(error);
    }
    if let Err(error) = history_stage.prepare_commit_journal() {
        history_stage.abort();
        return Err(error);
    }
    if let Err(error) = atomic_replace(&path, &output, &before_revision) {
        history_stage.abort();
        return Err(error);
    }
    let history_id = match commit_history_stage(&mut history_stage) {
        Ok(history_id) => history_id,
        Err(error) => {
            let rollback = permissions.as_ref().map_or_else(
                || {
                    Err(EditError::HistoryUnavailable(
                        "block permissions are unavailable".to_owned(),
                    ))
                },
                |permissions| {
                    atomic_replace_with_permissions(&path, &original, &after_revision, permissions)
                },
            );
            match rollback {
                Ok(()) => {
                    history_stage.abort();
                    return Err(error);
                }
                Err(rollback_error) => {
                    history_stage.preserve_for_recovery();
                    return Err(rollback_error);
                }
            }
        }
    };
    Ok(DrawingEditResult {
        drawing: request.drawing.clone(),
        revision: after_revision,
        entity_id: None,
        entity_ids: Vec::new(),
        operation: "update_block_definition".to_owned(),
        history_id: Some(history_id),
    })
}

struct OperationResult {
    entity_ids: Vec<String>,
    operation: &'static str,
}

fn apply_operation(
    operation: &EditOperation,
    project: &ProjectSource,
    entities: &mut Vec<Entity>,
    raw_lines: &mut Vec<RawLine>,
) -> EditResult<OperationResult> {
    match operation {
        EditOperation::Batch { operations } => {
            if operations.is_empty() {
                return Err(EditError::InvalidEntity(
                    "batch must contain at least one operation".to_owned(),
                ));
            }
            let mut ids = Vec::new();
            for operation in operations {
                let result = apply_operation(operation, project, entities, raw_lines)?;
                ids.extend(result.entity_ids);
            }
            Ok(OperationResult {
                entity_ids: ids,
                operation: "batch",
            })
        }
        EditOperation::Create { entity } => {
            let entity = create_entity(entity.clone())?;
            ensure_layer_editable(project, entity.layer())?;
            let id = entity.id().as_str().to_owned();
            append_raw_line(raw_lines, entity_json(&entity)?);
            entities.push(entity);
            Ok(OperationResult {
                entity_ids: vec![id],
                operation: "create",
            })
        }
        EditOperation::InsertBlock {
            block,
            layer,
            at,
            rotation_deg,
            scale,
            entity_id,
        } => {
            if !project.blocks.contains_key(block) {
                return Err(EditError::InvalidEntity(format!(
                    "block definition {block:?} does not exist"
                )));
            }
            if !at.iter().all(|value| value.is_finite())
                || !rotation_deg.is_finite()
                || !scale.is_finite()
                || *scale <= 0.0
            {
                return Err(EditError::InvalidEntity(
                    "block reference transform is invalid".to_owned(),
                ));
            }
            ensure_layer_editable(project, layer)?;
            let id = entity_id.clone().unwrap_or_else(next_id);
            let entity = create_entity(serde_json::json!({
                "schema_version": cad_model::CURRENT_SCHEMA_VERSION,
                "id": id,
                "type": "block_ref",
                "layer": layer,
                "block": block,
                "at": at,
                "rotation_deg": rotation_deg,
                "scale": scale,
            }))?;
            let id = entity.id().as_str().to_owned();
            append_raw_line(raw_lines, entity_json(&entity)?);
            entities.push(entity);
            Ok(OperationResult {
                entity_ids: vec![id],
                operation: "insert_block",
            })
        }
        EditOperation::Replace { entity_id, entity } => {
            let index = entity_index(entities, entity_id)?;
            ensure_layer_editable(project, entities[index].layer())?;
            let replacement: Entity = serde_json::from_value(entity.clone())
                .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
            if replacement.id().as_str() != entity_id
                || entity_kind(&replacement) != entity_kind(&entities[index])
            {
                return Err(EditError::InvalidEntity(
                    "replacement must preserve entity id and type".to_owned(),
                ));
            }
            ensure_layer_editable(project, replacement.layer())?;
            validate_entity_geometry(&replacement)?;
            raw_lines[index].content = entity_json(&replacement)?;
            entities[index] = replacement;
            Ok(OperationResult {
                entity_ids: vec![entity_id.clone()],
                operation: "replace",
            })
        }
        EditOperation::Translate {
            entity_id,
            delta,
            duplicate,
        } => apply_operation(
            &EditOperation::TranslateMany {
                entity_ids: vec![entity_id.clone()],
                delta: *delta,
                duplicate: *duplicate,
            },
            project,
            entities,
            raw_lines,
        ),
        EditOperation::TranslateMany {
            entity_ids,
            delta,
            duplicate,
        } => {
            ensure_unique_entity_ids(entity_ids)?;
            if !delta.iter().all(|value| value.is_finite()) {
                return Err(EditError::InvalidEntity(
                    "translation is not finite".to_owned(),
                ));
            }
            let source_ids = entity_ids.clone();
            let mut result_ids = Vec::new();
            for entity_id in source_ids {
                let index = entity_index(entities, &entity_id)?;
                ensure_layer_editable(project, entities[index].layer())?;
                let mut translated = translate_entity(&entities[index], *delta)?;
                validate_entity_geometry(&translated)?;
                let id = if *duplicate {
                    let id = next_id();
                    set_entity_id(&mut translated, &id)?;
                    insert_raw_line_after(raw_lines, index, entity_json(&translated)?);
                    entities.insert(index + 1, translated);
                    id
                } else {
                    raw_lines[index].content = entity_json(&translated)?;
                    entities[index] = translated;
                    entity_id
                };
                result_ids.push(id);
            }
            Ok(OperationResult {
                entity_ids: result_ids,
                operation: if *duplicate { "duplicate" } else { "translate" },
            })
        }
        EditOperation::Delete { entity_id } => apply_operation(
            &EditOperation::DeleteMany {
                entity_ids: vec![entity_id.clone()],
            },
            project,
            entities,
            raw_lines,
        ),
        EditOperation::DeleteMany { entity_ids } => {
            ensure_unique_entity_ids(entity_ids)?;
            let mut indices = entity_ids
                .iter()
                .map(|id| entity_index(entities, id))
                .collect::<EditResult<Vec<_>>>()?;
            indices.sort_unstable();
            indices.dedup();
            for index in indices.into_iter().rev() {
                ensure_layer_editable(project, entities[index].layer())?;
                remove_raw_line(raw_lines, index);
                entities.remove(index);
            }
            Ok(OperationResult {
                entity_ids: entity_ids.clone(),
                operation: "delete",
            })
        }
        EditOperation::Rotate {
            entity_ids,
            center,
            angle_deg,
        } => {
            ensure_unique_entity_ids(entity_ids)?;
            if !center.iter().all(|v| v.is_finite()) || !angle_deg.is_finite() {
                return Err(EditError::InvalidEntity(
                    "rotation is not finite".to_owned(),
                ));
            }
            transform_entities(
                project,
                entities,
                raw_lines,
                entity_ids,
                |entity| rotate_entity(entity, *center, *angle_deg),
                "rotate",
            )
        }
        EditOperation::Mirror {
            entity_ids,
            axis_start,
            axis_end,
        } => {
            ensure_unique_entity_ids(entity_ids)?;
            let dx = axis_end[0] - axis_start[0];
            let dy = axis_end[1] - axis_start[1];
            if !axis_start
                .iter()
                .chain(axis_end.iter())
                .all(|v| v.is_finite())
                || dx.hypot(dy) <= f64::EPSILON
            {
                return Err(EditError::InvalidEntity(
                    "mirror axis is invalid".to_owned(),
                ));
            }
            transform_entities(
                project,
                entities,
                raw_lines,
                entity_ids,
                |entity| mirror_entity(entity, *axis_start, *axis_end),
                "mirror",
            )
        }
        EditOperation::Offset {
            entity_ids,
            distance,
        } => {
            ensure_unique_entity_ids(entity_ids)?;
            if !distance.is_finite() || *distance == 0.0 {
                return Err(EditError::InvalidEntity(
                    "offset distance is invalid".to_owned(),
                ));
            }
            transform_entities(
                project,
                entities,
                raw_lines,
                entity_ids,
                |entity| offset_entity(entity, *distance),
                "offset",
            )
        }
        EditOperation::Trim {
            target_entity_id,
            cutter_entity_id,
            pick_point,
        } => {
            if !pick_point.iter().all(|value| value.is_finite()) {
                return Err(EditError::InvalidEntity(
                    "trim pick point is not finite".to_owned(),
                ));
            }
            if target_entity_id == cutter_entity_id {
                return Err(EditError::InvalidEntity(
                    "trim target and cutter must differ".to_owned(),
                ));
            }
            let target = entity_index(entities, target_entity_id)?;
            let cutter = entity_index(entities, cutter_entity_id)?;
            ensure_layer_editable(project, entities[target].layer())?;
            ensure_layer_editable(project, entities[cutter].layer())?;
            let trimmed = trim_entity(&entities[target], &entities[cutter], *pick_point)?;
            validate_entity_geometry(&trimmed)?;
            raw_lines[target].content = entity_json(&trimmed)?;
            entities[target] = trimmed;
            Ok(OperationResult {
                entity_ids: vec![target_entity_id.clone()],
                operation: "trim",
            })
        }
        EditOperation::Extend {
            target_entity_id,
            boundary_entity_id,
            pick_point,
        } => {
            if !pick_point.iter().all(|value| value.is_finite()) {
                return Err(EditError::InvalidEntity(
                    "extend pick point is not finite".to_owned(),
                ));
            }
            if target_entity_id == boundary_entity_id {
                return Err(EditError::InvalidEntity(
                    "extend target and boundary must differ".to_owned(),
                ));
            }
            let target = entity_index(entities, target_entity_id)?;
            let boundary = entity_index(entities, boundary_entity_id)?;
            ensure_layer_editable(project, entities[target].layer())?;
            ensure_layer_editable(project, entities[boundary].layer())?;
            let extended = extend_entity(&entities[target], &entities[boundary], *pick_point)?;
            validate_entity_geometry(&extended)?;
            raw_lines[target].content = entity_json(&extended)?;
            entities[target] = extended;
            Ok(OperationResult {
                entity_ids: vec![target_entity_id.clone()],
                operation: "extend",
            })
        }
        EditOperation::UpdateHatch {
            entity_id,
            loops,
            pattern,
            angle_deg,
            scale,
            fill,
        } => {
            if !angle_deg.is_finite()
                || !scale.is_finite()
                || *scale <= 0.0
                || pattern.trim().is_empty()
                || loops.iter().any(|points| {
                    points.len() < 3
                        || points
                            .iter()
                            .any(|point| !point[0].is_finite() || !point[1].is_finite())
                })
            {
                return Err(EditError::InvalidEntity(
                    "hatch geometry or properties are invalid".to_owned(),
                ));
            }
            let index = entity_index(entities, entity_id)?;
            ensure_layer_editable(project, entities[index].layer())?;
            if !matches!(entities[index], Entity::Hatch { .. }) {
                return Err(EditError::InvalidEntity(
                    "update_hatch requires a hatch entity".to_owned(),
                ));
            }
            let mut value = serde_json::to_value(&entities[index])
                .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
            value["loops"] = serde_json::to_value(loops)
                .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
            value["pattern"] = Value::String(pattern.clone());
            value["angle_deg"] = Value::from(*angle_deg);
            value["scale"] = Value::from(*scale);
            value["fill"] = fill.clone().map_or(Value::Null, Value::String);
            let updated: Entity = serde_json::from_value(value)
                .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
            validate_entity_geometry(&updated)?;
            raw_lines[index].content = entity_json(&updated)?;
            entities[index] = updated;
            Ok(OperationResult {
                entity_ids: vec![entity_id.clone()],
                operation: "update_hatch",
            })
        }
        EditOperation::UpdateLayout { .. } => Err(EditError::InvalidEntity(
            "layout edits require a layout transaction".to_owned(),
        )),
        EditOperation::UpdateBlockDefinition { .. } => Err(EditError::InvalidEntity(
            "block definition edits require a block transaction".to_owned(),
        )),
    }
}

fn transform_entities<F>(
    project: &ProjectSource,
    entities: &mut [Entity],
    raw_lines: &mut [RawLine],
    entity_ids: &[String],
    transform: F,
    operation: &'static str,
) -> EditResult<OperationResult>
where
    F: Fn(&Entity) -> EditResult<Entity>,
{
    let mut ids = Vec::new();
    for entity_id in entity_ids {
        let index = entity_index(entities, entity_id)?;
        ensure_layer_editable(project, entities[index].layer())?;
        let transformed = transform(&entities[index])?;
        validate_entity_geometry(&transformed)?;
        raw_lines[index].content = entity_json(&transformed)?;
        entities[index] = transformed;
        ids.push(entity_id.clone());
    }
    Ok(OperationResult {
        entity_ids: ids,
        operation,
    })
}

fn transform_json_points(value: &mut Value, transform: &impl Fn(Point) -> Point) {
    match value {
        Value::Array(values) if values.len() == 2 && values.iter().all(Value::is_number) => {
            let point = [
                values[0].as_f64().unwrap_or_default(),
                values[1].as_f64().unwrap_or_default(),
            ];
            let transformed = transform(point);
            values[0] = Value::from(transformed[0]);
            values[1] = Value::from(transformed[1]);
        }
        Value::Array(values) => values
            .iter_mut()
            .for_each(|value| transform_json_points(value, transform)),
        Value::Object(values) => values
            .values_mut()
            .for_each(|value| transform_json_points(value, transform)),
        _ => {}
    }
}

fn rotate_entity(entity: &Entity, center: Point, angle_deg: f64) -> EditResult<Entity> {
    let angle = angle_deg.to_radians();
    let (sin, cos) = angle.sin_cos();
    let mut value = serde_json::to_value(entity)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    transform_json_points(&mut value, &|point| {
        let x = point[0] - center[0];
        let y = point[1] - center[1];
        [center[0] + x * cos - y * sin, center[1] + x * sin + y * cos]
    });
    add_angle_fields(&mut value, angle_deg);
    serde_json::from_value(value).map_err(|error| EditError::InvalidEntity(error.to_string()))
}

fn mirror_entity(entity: &Entity, axis_start: Point, axis_end: Point) -> EditResult<Entity> {
    let dx = axis_end[0] - axis_start[0];
    let dy = axis_end[1] - axis_start[1];
    let length = dx.hypot(dy);
    let (ux, uy) = (dx / length, dy / length);
    let mut value = serde_json::to_value(entity)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    transform_json_points(&mut value, &|point| {
        let x = point[0] - axis_start[0];
        let y = point[1] - axis_start[1];
        let projection = x * ux + y * uy;
        let parallel = [projection * ux, projection * uy];
        [
            axis_start[0] + 2.0 * parallel[0] - x,
            axis_start[1] + 2.0 * parallel[1] - y,
        ]
    });
    if let Some(object) = value.as_object_mut() {
        for key in ["rotation_deg", "text_rotation_deg", "start_deg", "end_deg"] {
            if let Some(number) = object.get(key).and_then(Value::as_f64) {
                object.insert(
                    key.to_owned(),
                    Value::from(2.0 * dy.atan2(dx).to_degrees() - number),
                );
            }
        }
        for key in ["mirror_y", "text_mirror_y"] {
            if let Some(Value::Bool(flag)) = object.get_mut(key) {
                *flag = !*flag;
            }
        }
    }
    serde_json::from_value(value).map_err(|error| EditError::InvalidEntity(error.to_string()))
}

fn add_angle_fields(value: &mut Value, angle_deg: f64) {
    if let Some(object) = value.as_object_mut() {
        for key in ["rotation_deg", "text_rotation_deg", "start_deg", "end_deg"] {
            if let Some(number) = object.get(key).and_then(Value::as_f64) {
                object.insert(key.to_owned(), Value::from(number + angle_deg));
            }
        }
    }
}

fn offset_entity(entity: &Entity, distance: f64) -> EditResult<Entity> {
    match entity {
        Entity::Line { p1, p2, .. } => {
            let normal = offset_normal(*p1, *p2, distance)?;
            replace_entity_points(
                entity,
                &[*p1, *p2].map(|point| [point[0] + normal[0], point[1] + normal[1]]),
            )
        }
        Entity::Polyline { points, closed, .. } if points.len() >= 2 => {
            let result = offset_polyline(points, *closed, distance)?;
            if polyline_self_intersects(&result, *closed) {
                return Err(EditError::InvalidEntity(
                    "offset produces a self-intersecting polyline".to_owned(),
                ));
            }
            replace_entity_points(entity, &result)
        }
        _ => Err(EditError::InvalidEntity(
            "offset supports line and polyline only".to_owned(),
        )),
    }
}

fn offset_polyline(points: &[Point], closed: bool, distance: f64) -> EditResult<Vec<Point>> {
    let segment_count = if closed {
        points.len()
    } else {
        points.len() - 1
    };
    if segment_count == 0 {
        return Err(EditError::InvalidEntity(
            "polyline requires at least one segment".to_owned(),
        ));
    }
    let mut segments = Vec::with_capacity(segment_count);
    for index in 0..segment_count {
        let start = points[index];
        let end = points[(index + 1) % points.len()];
        let normal = offset_normal(start, end, distance)?;
        segments.push((
            [start[0] + normal[0], start[1] + normal[1]],
            [end[0] + normal[0], end[1] + normal[1]],
        ));
    }
    let mut result = Vec::with_capacity(points.len());
    if closed {
        for index in 0..points.len() {
            let previous = segments[(index + segment_count - 1) % segment_count];
            let next = segments[index];
            result.push(
                infinite_line_intersection(previous.0, previous.1, next.0, next.1).ok_or_else(
                    || {
                        EditError::InvalidEntity(
                            "parallel polyline segments cannot be joined".to_owned(),
                        )
                    },
                )?,
            );
        }
    } else {
        result.push(segments[0].0);
        for index in 1..points.len() - 1 {
            result.push(
                infinite_line_intersection(
                    segments[index - 1].0,
                    segments[index - 1].1,
                    segments[index].0,
                    segments[index].1,
                )
                .ok_or_else(|| {
                    EditError::InvalidEntity(
                        "parallel polyline segments cannot be joined".to_owned(),
                    )
                })?,
            );
        }
        result.push(segments[segment_count - 1].1);
    }
    Ok(result)
}

fn polyline_self_intersects(points: &[Point], closed: bool) -> bool {
    let segment_count = if closed {
        points.len()
    } else {
        points.len().saturating_sub(1)
    };
    for left in 0..segment_count {
        for right in left + 1..segment_count {
            let adjacent = right == left + 1 || (closed && left == 0 && right + 1 == segment_count);
            if adjacent {
                continue;
            }
            let left_end = points[(left + 1) % points.len()];
            let right_end = points[(right + 1) % points.len()];
            if finite_line_intersection(points[left], left_end, points[right], right_end).is_some()
            {
                return true;
            }
        }
    }
    false
}

fn offset_normal(start: Point, end: Point, distance: f64) -> EditResult<Point> {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let length = dx.hypot(dy);
    if !length.is_finite() || length <= f64::EPSILON {
        return Err(EditError::InvalidEntity(
            "cannot offset zero-length segment".to_owned(),
        ));
    }
    Ok([-dy / length * distance, dx / length * distance])
}

fn replace_entity_points(entity: &Entity, points: &[Point]) -> EditResult<Entity> {
    let mut value = serde_json::to_value(entity)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    match entity {
        Entity::Line { .. } | Entity::Dimension { .. } => {
            if points.len() != 2 {
                return Err(EditError::InvalidEntity(
                    "line replacement requires exactly two points".to_owned(),
                ));
            }
            value["p1"] = Value::from(points[0].to_vec());
            value["p2"] = Value::from(points[1].to_vec());
        }
        Entity::Polyline { .. } => {
            value["points"] = Value::Array(
                points
                    .iter()
                    .map(|point| Value::from(point.to_vec()))
                    .collect(),
            )
        }
        _ => {
            return Err(EditError::InvalidEntity(
                "entity does not contain editable line points".to_owned(),
            ));
        }
    }
    serde_json::from_value(value).map_err(|error| EditError::InvalidEntity(error.to_string()))
}

fn trim_entity(target: &Entity, cutter: &Entity, pick_point: Point) -> EditResult<Entity> {
    let target_segments = path_segments(target)?;
    let cutter_segments = path_segments(cutter)?;
    let intersections = segment_intersections(&target_segments, &cutter_segments);
    if intersections.is_empty() {
        return Err(EditError::InvalidEntity(
            "trim entities do not intersect".to_owned(),
        ));
    }
    if intersections.len() > 1 {
        return Err(EditError::InvalidEntity(
            "trim intersection is ambiguous".to_owned(),
        ));
    }
    let (segment_index, intersection) = intersections[0];
    match target {
        Entity::Line { p1, p2, .. } => {
            let keep_start = distance_sq(pick_point, *p1) > distance_sq(pick_point, *p2);
            let points = if keep_start {
                [*p1, intersection]
            } else {
                [intersection, *p2]
            };
            replace_entity_points(target, &points)
        }
        Entity::Polyline { points, closed, .. } if !closed => {
            let prefix_distance = path_distance_to_segment(points, segment_index, intersection);
            let pick_distance = nearest_path_distance(points, pick_point);
            if (pick_distance - prefix_distance).abs() <= 1e-9 {
                return Err(EditError::InvalidEntity(
                    "trim pick side is ambiguous".to_owned(),
                ));
            }
            let kept = if pick_distance > prefix_distance {
                let mut kept = points[..=segment_index].to_vec();
                if distance_sq(*kept.last().expect("segment start"), intersection) > 1e-18 {
                    kept.push(intersection);
                }
                kept
            } else {
                let mut kept = vec![intersection];
                kept.extend_from_slice(&points[segment_index + 1..]);
                kept
            };
            replace_entity_points(target, &kept)
        }
        Entity::Polyline { closed: true, .. } => Err(EditError::InvalidEntity(
            "trim does not support closed polylines".to_owned(),
        )),
        _ => Err(EditError::InvalidEntity(
            "trim supports line and open polyline entities only".to_owned(),
        )),
    }
}

fn extend_entity(target: &Entity, boundary: &Entity, pick_point: Point) -> EditResult<Entity> {
    let target_points = match target {
        Entity::Line { p1, p2, .. } => vec![*p1, *p2],
        Entity::Polyline { points, closed, .. } if !closed => points.clone(),
        Entity::Polyline { closed: true, .. } => {
            return Err(EditError::InvalidEntity(
                "extend does not support closed polylines".to_owned(),
            ));
        }
        _ => {
            return Err(EditError::InvalidEntity(
                "extend supports line and open polyline entities only".to_owned(),
            ));
        }
    };
    let boundary_segments = path_segments(boundary)?;
    let target_length = path_length(&target_points);
    let pick_distance = nearest_path_distance(&target_points, pick_point);
    let extend_start = pick_distance <= target_length / 2.0;
    let endpoint = if extend_start {
        target_points[0]
    } else {
        *target_points.last().expect("target endpoint")
    };
    let adjacent = if extend_start {
        target_points[1]
    } else {
        target_points[target_points.len() - 2]
    };
    let extension_line_end = [
        endpoint[0] + endpoint[0] - adjacent[0],
        endpoint[1] + endpoint[1] - adjacent[1],
    ];
    let mut intersections = Vec::new();
    for (boundary_start, boundary_end) in boundary_segments {
        let Some(intersection) =
            infinite_line_intersection(endpoint, extension_line_end, boundary_start, boundary_end)
        else {
            continue;
        };
        if !point_on_segment(intersection, boundary_start, boundary_end)
            || point_on_segment(intersection, endpoint, adjacent)
        {
            continue;
        }
        if !intersections
            .iter()
            .any(|candidate: &Point| distance_sq(*candidate, intersection) <= 1e-18)
        {
            intersections.push(intersection);
        }
    }
    let intersection = match intersections.as_slice() {
        [] => {
            return Err(EditError::InvalidEntity(
                "extend boundary does not intersect the target path".to_owned(),
            ));
        }
        [intersection] => *intersection,
        _ => {
            return Err(EditError::InvalidEntity(
                "extend boundary intersection is ambiguous".to_owned(),
            ));
        }
    };
    let mut points = target_points;
    if extend_start {
        points[0] = intersection;
    } else {
        let last = points.len() - 1;
        points[last] = intersection;
    }
    replace_entity_points(target, &points)
}

fn point_on_segment(point: Point, start: Point, end: Point) -> bool {
    point[0] >= start[0].min(end[0]) - 1e-9
        && point[0] <= start[0].max(end[0]) + 1e-9
        && point[1] >= start[1].min(end[1]) - 1e-9
        && point[1] <= start[1].max(end[1]) + 1e-9
}

fn path_segments(entity: &Entity) -> EditResult<Vec<(Point, Point)>> {
    match entity {
        Entity::Line { p1, p2, .. } => Ok(vec![(*p1, *p2)]),
        Entity::Polyline { points, closed, .. } if points.len() >= 2 => {
            let mut segments = points
                .windows(2)
                .map(|pair| (pair[0], pair[1]))
                .collect::<Vec<_>>();
            if *closed {
                segments.push((*points.last().expect("polyline point"), points[0]));
            }
            Ok(segments)
        }
        _ => Err(EditError::InvalidEntity(
            "operation supports line and polyline entities only".to_owned(),
        )),
    }
}

fn segment_intersections(
    target_segments: &[(Point, Point)],
    cutter_segments: &[(Point, Point)],
) -> Vec<(usize, Point)> {
    let mut intersections = Vec::new();
    for (target_index, (target_start, target_end)) in target_segments.iter().enumerate() {
        for (cutter_start, cutter_end) in cutter_segments {
            let Some(point) =
                finite_line_intersection(*target_start, *target_end, *cutter_start, *cutter_end)
            else {
                continue;
            };
            if !intersections
                .iter()
                .any(|(_, candidate): &(usize, Point)| distance_sq(*candidate, point) <= 1e-18)
            {
                intersections.push((target_index, point));
            }
        }
    }
    intersections
}

fn path_length(points: &[Point]) -> f64 {
    points
        .windows(2)
        .map(|pair| distance_sq(pair[0], pair[1]).sqrt())
        .sum()
}

fn path_distance_to_segment(points: &[Point], segment_index: usize, point: Point) -> f64 {
    points[..segment_index]
        .windows(2)
        .map(|pair| distance_sq(pair[0], pair[1]).sqrt())
        .sum::<f64>()
        + distance_sq(points[segment_index], point).sqrt()
}

fn nearest_path_distance(points: &[Point], point: Point) -> f64 {
    let mut best_distance = f64::INFINITY;
    let mut cumulative_distance = 0.0;
    let mut best_path_distance = 0.0;
    for pair in points.windows(2) {
        let length = distance_sq(pair[0], pair[1]).sqrt();
        if length <= f64::EPSILON {
            continue;
        }
        let dx = pair[1][0] - pair[0][0];
        let dy = pair[1][1] - pair[0][1];
        let t = (((point[0] - pair[0][0]) * dx + (point[1] - pair[0][1]) * dy) / (length * length))
            .clamp(0.0, 1.0);
        let projected = [pair[0][0] + t * dx, pair[0][1] + t * dy];
        if distance_sq(projected, point) < best_distance {
            best_distance = distance_sq(projected, point);
            best_path_distance = cumulative_distance + length * t;
        }
        cumulative_distance += length;
    }
    best_path_distance
}

fn distance_sq(left: Point, right: Point) -> f64 {
    (left[0] - right[0]).powi(2) + (left[1] - right[1]).powi(2)
}

fn finite_line_intersection(a: Point, b: Point, c: Point, d: Point) -> Option<Point> {
    let point = infinite_line_intersection(a, b, c, d)?;
    let within = |p: Point, start: Point, end: Point| {
        p[0] >= start[0].min(end[0]) - 1e-9
            && p[0] <= start[0].max(end[0]) + 1e-9
            && p[1] >= start[1].min(end[1]) - 1e-9
            && p[1] <= start[1].max(end[1]) + 1e-9
    };
    if within(point, a, b) && within(point, c, d) {
        Some(point)
    } else {
        None
    }
}

fn infinite_line_intersection(a: Point, b: Point, c: Point, d: Point) -> Option<Point> {
    let denominator = (a[0] - b[0]) * (c[1] - d[1]) - (a[1] - b[1]) * (c[0] - d[0]);
    if denominator.abs() <= f64::EPSILON {
        return None;
    }
    let left = a[0] * b[1] - a[1] * b[0];
    let right = c[0] * d[1] - c[1] * d[0];
    Some([
        (left * (c[0] - d[0]) - (a[0] - b[0]) * right) / denominator,
        (left * (c[1] - d[1]) - (a[1] - b[1]) * right) / denominator,
    ])
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawLine {
    content: String,
    ending: String,
}

fn split_raw_lines(text: &str) -> Vec<RawLine> {
    text.split_inclusive('\n')
        .map(|line| {
            if let Some(content) = line.strip_suffix("\r\n") {
                RawLine {
                    content: content.to_owned(),
                    ending: "\r\n".to_owned(),
                }
            } else if let Some(content) = line.strip_suffix('\n') {
                RawLine {
                    content: content.to_owned(),
                    ending: "\n".to_owned(),
                }
            } else {
                RawLine {
                    content: line.to_owned(),
                    ending: String::new(),
                }
            }
        })
        .collect()
}

fn preferred_ending(lines: &[RawLine], index: usize) -> String {
    lines
        .get(index)
        .filter(|line| !line.ending.is_empty())
        .or_else(|| {
            lines[..index.min(lines.len())]
                .iter()
                .rev()
                .find(|line| !line.ending.is_empty())
        })
        .or_else(|| {
            lines
                .get(index + 1..)
                .and_then(|rest| rest.iter().find(|line| !line.ending.is_empty()))
        })
        .map_or_else(|| "\n".to_owned(), |line| line.ending.clone())
}

fn append_raw_line(lines: &mut Vec<RawLine>, content: String) {
    if lines.is_empty() {
        lines.push(RawLine {
            content,
            ending: "\n".to_owned(),
        });
        return;
    }
    let last = lines.len() - 1;
    let ending = preferred_ending(lines, last);
    let had_trailing_ending = !lines[last].ending.is_empty();
    if !had_trailing_ending {
        lines[last].ending = ending.clone();
    }
    lines.push(RawLine {
        content,
        ending: if had_trailing_ending {
            ending
        } else {
            String::new()
        },
    });
}

fn insert_raw_line_after(lines: &mut Vec<RawLine>, index: usize, content: String) {
    let ending = preferred_ending(lines, index);
    let original_ending = std::mem::replace(&mut lines[index].ending, ending.clone());
    lines.insert(
        index + 1,
        RawLine {
            content,
            ending: original_ending,
        },
    );
}

fn remove_raw_line(lines: &mut Vec<RawLine>, index: usize) {
    let removed = lines.remove(index);
    if removed.ending.is_empty() && index > 0 {
        lines[index - 1].ending.clear();
    }
}

fn render_raw_lines(lines: &[RawLine]) -> String {
    let mut output = String::new();
    for line in lines {
        output.push_str(&line.content);
        output.push_str(&line.ending);
    }
    output
}

fn checker_errors(project: &ProjectSource) -> BTreeSet<String> {
    cad_check::check_loaded_project(project)
        .diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .map(|diagnostic| {
            format!(
                "{}:{:?}:{:?}:{}",
                diagnostic.code, diagnostic.entity_id, diagnostic.field, diagnostic.message
            )
        })
        .collect()
}

fn ensure_layer_editable(project: &ProjectSource, layer_id: &str) -> EditResult<()> {
    let layer = project
        .layers
        .layers
        .get(layer_id)
        .ok_or_else(|| EditError::LayerNotEditable(layer_id.to_owned()))?;
    let group_locked_or_hidden = layer
        .group
        .as_ref()
        .and_then(|group| project.layers.groups.get(group))
        .is_some_and(|group| group.locked || !group.visible);
    if !layer.visible || layer.locked || group_locked_or_hidden {
        return Err(EditError::LayerNotEditable(layer_id.to_owned()));
    }
    Ok(())
}

fn create_entity(mut value: Value) -> EditResult<Entity> {
    let object = value
        .as_object_mut()
        .ok_or_else(|| EditError::InvalidEntity("entity must be an object".to_owned()))?;
    object.insert(
        "schema_version".to_owned(),
        Value::String(cad_model::CURRENT_SCHEMA_VERSION.to_owned()),
    );
    object.insert("id".to_owned(), Value::String(next_id()));
    let entity = serde_json::from_value(value)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    validate_entity_geometry(&entity)?;
    Ok(entity)
}

fn set_entity_id(entity: &mut Entity, id: &str) -> EditResult<()> {
    let mut value = serde_json::to_value(&*entity)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    value["id"] = Value::String(id.to_owned());
    *entity = serde_json::from_value(value)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    Ok(())
}

fn entity_index(entities: &[Entity], id: &str) -> EditResult<usize> {
    entities
        .iter()
        .position(|entity| entity.id().as_str() == id)
        .ok_or_else(|| EditError::EntityNotFound(id.to_owned()))
}

fn ensure_unique_entity_ids(entity_ids: &[String]) -> EditResult<()> {
    if entity_ids.is_empty() {
        return Err(EditError::InvalidEntity(
            "entity_ids must contain at least one entity".to_owned(),
        ));
    }
    let mut seen = BTreeSet::new();
    for entity_id in entity_ids {
        if !seen.insert(entity_id) {
            return Err(EditError::InvalidEntity(format!(
                "entity id {entity_id:?} appears more than once"
            )));
        }
    }
    Ok(())
}

fn entity_json(entity: &Entity) -> EditResult<String> {
    serde_json::to_string(entity).map_err(|error| EditError::InvalidEntity(error.to_string()))
}

fn validate_entity_geometry(entity: &Entity) -> EditResult<()> {
    let value = serde_json::to_value(entity)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    if !json_numbers_are_finite(&value) {
        return Err(EditError::InvalidEntity(
            "entity contains non-finite coordinates".to_owned(),
        ));
    }
    let invalid = match entity {
        Entity::Line { p1, p2, .. } | Entity::Dimension { p1, p2, .. } => {
            distance_sq(*p1, *p2) <= f64::EPSILON
        }
        Entity::Polyline { points, .. } => {
            points.len() < 2
                || points
                    .windows(2)
                    .any(|pair| distance_sq(pair[0], pair[1]) <= f64::EPSILON)
        }
        Entity::Arc { radius, .. }
        | Entity::Circle { radius, .. }
        | Entity::CurveSolid { radius, .. } => *radius <= f64::EPSILON,
        Entity::Ellipse {
            radius_x, radius_y, ..
        } => *radius_x <= f64::EPSILON || *radius_y <= f64::EPSILON,
        Entity::Solid { points, .. } => points.len() < 3,
        Entity::Text { .. } | Entity::Point { .. } | Entity::BlockRef { .. } => false,
        Entity::Hatch {
            loops,
            pattern,
            angle_deg,
            scale,
            ..
        } => {
            loops.is_empty()
                || loops.iter().any(|points| {
                    points.len() < 3
                        || polygon_area(points).abs() <= f64::EPSILON
                        || points
                            .iter()
                            .any(|point| !point[0].is_finite() || !point[1].is_finite())
                })
                || pattern.trim().is_empty()
                || !angle_deg.is_finite()
                || !scale.is_finite()
                || *scale <= 0.0
        }
    };
    if invalid {
        return Err(EditError::InvalidEntity(
            "entity geometry has zero length or insufficient points".to_owned(),
        ));
    }
    Ok(())
}

fn json_numbers_are_finite(value: &Value) -> bool {
    match value {
        Value::Number(number) => number.as_f64().is_some_and(f64::is_finite),
        Value::Array(values) => values.iter().all(json_numbers_are_finite),
        Value::Object(values) => values.values().all(json_numbers_are_finite),
        Value::Null | Value::Bool(_) | Value::String(_) => true,
    }
}

fn polygon_area(points: &[Point]) -> f64 {
    points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(left, right)| left[0] * right[1] - right[0] * left[1])
        .sum::<f64>()
        * 0.5
}

fn entity_kind(entity: &Entity) -> &'static str {
    match entity {
        Entity::Line { .. } => "line",
        Entity::Polyline { .. } => "polyline",
        Entity::Arc { .. } => "arc",
        Entity::Circle { .. } => "circle",
        Entity::Ellipse { .. } => "ellipse",
        Entity::Text { .. } => "text",
        Entity::Dimension { .. } => "dimension",
        Entity::Point { .. } => "point",
        Entity::Solid { .. } => "solid",
        Entity::CurveSolid { .. } => "curve_solid",
        Entity::BlockRef { .. } => "block_ref",
        Entity::Hatch { .. } => "hatch",
    }
}

fn next_id() -> String {
    format!("ent_{}", Ulid::new())
}

fn revision(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

fn entities_path(project: &Path, drawing: &str) -> PathBuf {
    project
        .join("drawings")
        .join(drawing)
        .join("entities.ndjson")
}

fn edit_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn atomic_replace(path: &Path, bytes: &[u8], expected_revision: &str) -> EditResult<()> {
    let permissions = fs::metadata(path)
        .map_err(|source| EditError::Read {
            path: path.to_path_buf(),
            source,
        })?
        .permissions();
    let snapshot = capture_permissions(&permissions);
    atomic_replace_with_permissions(path, bytes, expected_revision, &snapshot)
}

pub fn atomic_create(
    path: &Path,
    bytes: &[u8],
    permissions: Option<&PermissionsSnapshot>,
) -> EditResult<()> {
    let project_root = path
        .ancestors()
        .find(|candidate| candidate.join("cad.project.toml").is_file())
        .map(|candidate| {
            let relative = path
                .strip_prefix(candidate)
                .expect("ancestor is a lexical prefix")
                .to_path_buf();
            fs::canonicalize(candidate)
                .map(|root| (root, relative))
                .map_err(|source| EditError::Read {
                    path: candidate.to_path_buf(),
                    source,
                })
        })
        .transpose()?;
    if let Some((root, relative)) = project_root.as_ref() {
        validate_source_path_components(root, relative)?;
    }
    let parent = path
        .parent()
        .ok_or_else(|| EditError::InvalidEntity("layout path has no parent".to_owned()))?;
    fs::create_dir_all(parent).map_err(|source| EditError::Write {
        path: parent.to_path_buf(),
        source,
    })?;
    if let Some((root, relative)) = project_root.as_ref() {
        validate_source_path_components(root, relative)?;
    }
    let temp = tempfile::NamedTempFile::new_in(parent).map_err(|source| EditError::Write {
        path: parent.to_path_buf(),
        source,
    })?;
    fs::write(temp.path(), bytes).map_err(|source| EditError::Write {
        path: temp.path().to_path_buf(),
        source,
    })?;
    if let Some(permissions) = permissions {
        apply_permissions(temp.path(), permissions)?;
    }
    temp.as_file()
        .sync_all()
        .map_err(|source| EditError::Write {
            path: temp.path().to_path_buf(),
            source,
        })?;
    match fs::hard_link(temp.path(), path) {
        Ok(()) => sync_directory(parent),
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(EditError::RevisionConflict)
        }
        Err(source) => Err(EditError::Write {
            path: path.to_path_buf(),
            source,
        }),
    }
}

pub fn atomic_delete(
    project_root: &Path,
    relative_path: &str,
    expected_revision: &str,
) -> EditResult<()> {
    let root = fs::canonicalize(project_root).map_err(|source| EditError::Read {
        path: project_root.to_path_buf(),
        source,
    })?;
    let relative = Path::new(relative_path);
    validate_source_path_components(&root, relative)?;
    let path = root.join(relative);
    let parent = path
        .parent()
        .ok_or_else(|| EditError::InvalidEntity("source delete target has no parent".to_owned()))?;
    let current = fs::read(&path).map_err(|source| EditError::Read {
        path: path.clone(),
        source,
    })?;
    if revision(&current) != expected_revision {
        return Err(EditError::RevisionConflict);
    }
    let placeholder =
        tempfile::NamedTempFile::new_in(parent).map_err(|source| EditError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    let (_, staging) = placeholder.keep().map_err(|error| EditError::Write {
        path: error.file.path().to_path_buf(),
        source: error.error,
    })?;
    fs::remove_file(&staging).map_err(|source| EditError::Write {
        path: staging.clone(),
        source,
    })?;
    let journal =
        prepare_delete_journal(&root, &path, &staging, expected_revision, "history.delete")?;
    fs::rename(&path, &staging).map_err(|source| EditError::Write {
        path: path.clone(),
        source,
    })?;
    sync_directory(parent)?;
    let displaced = fs::read(&staging).map_err(|source| EditError::Read {
        path: staging.clone(),
        source,
    })?;
    if revision(&displaced) == expected_revision {
        fs::remove_file(&staging).map_err(|source| EditError::Write {
            path: staging,
            source,
        })?;
        sync_directory(parent)?;
        fs::remove_dir_all(&journal).map_err(|source| EditError::Write {
            path: journal,
            source,
        })?;
        return Ok(());
    }

    match fs::hard_link(&staging, &path) {
        Ok(()) => {
            fs::remove_file(&staging).map_err(|source| EditError::Write {
                path: staging,
                source,
            })?;
            sync_directory(parent)?;
            fs::remove_dir_all(&journal).map_err(|source| EditError::Write {
                path: journal,
                source,
            })?;
            Err(EditError::RevisionConflict)
        }
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
            let recovery_path =
                preserve_source_conflict(Some(&root), &path, &[], &staging, Some(&journal))?;
            Err(EditError::RecoveryRequired { recovery_path })
        }
        Err(source) => Err(EditError::Write { path, source }),
    }
}

pub fn atomic_replace_with_permissions(
    path: &Path,
    bytes: &[u8],
    expected_revision: &str,
    permissions: &PermissionsSnapshot,
) -> EditResult<()> {
    let project_root = find_project_root(path);
    atomic_exchange_replace(
        path,
        bytes,
        expected_revision,
        permissions,
        project_root
            .as_ref()
            .map(|root| (root.as_path(), "source.replace")),
    )
}

fn atomic_exchange_replace(
    path: &Path,
    bytes: &[u8],
    expected_revision: &str,
    permissions: &PermissionsSnapshot,
    project: Option<(&Path, &str)>,
) -> EditResult<()> {
    if let Some((root, _)) = project {
        let canonical_root = fs::canonicalize(root).map_err(|source| EditError::Read {
            path: root.to_path_buf(),
            source,
        })?;
        let canonical_parent =
            fs::canonicalize(path.parent().ok_or_else(|| {
                EditError::InvalidEntity("source target has no parent".to_owned())
            })?)
            .map_err(|source| EditError::Read {
                path: path.to_path_buf(),
                source,
            })?;
        let normalized = canonical_parent.join(path.file_name().ok_or_else(|| {
            EditError::InvalidEntity("source target has no file name".to_owned())
        })?);
        let relative = normalized.strip_prefix(&canonical_root).map_err(|_| {
            EditError::InvalidEntity("source target escapes the project root".to_owned())
        })?;
        validate_source_path_components(&canonical_root, relative)?;
    }
    let parent = path
        .parent()
        .ok_or_else(|| EditError::InvalidEntity("entities path has no parent".to_owned()))?;
    let temp = tempfile::NamedTempFile::new_in(parent).map_err(|source| EditError::Write {
        path: parent.to_path_buf(),
        source,
    })?;
    fs::write(temp.path(), bytes).map_err(|source| EditError::Write {
        path: temp.path().to_path_buf(),
        source,
    })?;
    apply_permissions(temp.path(), permissions)?;
    temp.as_file()
        .sync_all()
        .map_err(|source| EditError::Write {
            path: temp.path().to_path_buf(),
            source,
        })?;
    let current = fs::read(path).map_err(|source| EditError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    if revision(&current) != expected_revision {
        return Err(EditError::RevisionConflict);
    }

    let journal = if let Some((root, operation)) = project {
        Some(prepare_source_journal(
            root,
            path,
            temp.path(),
            bytes,
            expected_revision,
            operation,
        )?)
    } else {
        None
    };

    exchange_paths(temp.path(), path)?;
    sync_directory(parent)?;
    let displaced = fs::read(temp.path()).map_err(|source| EditError::Read {
        path: temp.path().to_path_buf(),
        source,
    })?;
    if revision(&displaced) != expected_revision {
        exchange_paths(temp.path(), path)?;
        sync_directory(parent)?;
        let (_staging_file, staging_path) = temp.keep().map_err(|error| EditError::Write {
            path: error.file.path().to_path_buf(),
            source: error.error,
        })?;
        let recovery_path = preserve_source_conflict(
            project.map(|(root, _)| root),
            path,
            bytes,
            &staging_path,
            journal.as_deref(),
        )?;
        return Err(EditError::RecoveryRequired { recovery_path });
    }

    if let Some(journal) = journal {
        fs::remove_dir_all(&journal).map_err(|source| EditError::Write {
            path: journal,
            source,
        })?;
    }
    Ok(())
}

fn validate_source_relative_path(relative: &Path) -> EditResult<()> {
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir
                    | std::path::Component::RootDir
                    | std::path::Component::Prefix(_)
            )
        })
        || cad_model::classify_project_source_path(relative).is_none()
    {
        return Err(EditError::InvalidEntity(format!(
            "unsafe or non-source transaction path {}",
            relative.display()
        )));
    }
    Ok(())
}

fn validate_source_path_components(root: &Path, relative: &Path) -> EditResult<()> {
    validate_source_relative_path(relative)?;
    let mut current = root.to_path_buf();
    let component_count = relative.components().count();
    for (index, component) in relative.components().enumerate() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(EditError::InvalidEntity(format!(
                    "source path contains a symlink: {}",
                    current.display()
                )));
            }
            Ok(metadata) if index + 1 < component_count && !metadata.is_dir() => {
                return Err(EditError::InvalidEntity(format!(
                    "source parent is not a directory: {}",
                    current.display()
                )));
            }
            Ok(metadata) if index + 1 == component_count && !metadata.is_file() => {
                return Err(EditError::InvalidEntity(format!(
                    "source target is not a regular file: {}",
                    current.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(source) => {
                return Err(EditError::Read {
                    path: current,
                    source,
                });
            }
        }
    }
    Ok(())
}

fn find_project_root(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .find(|candidate| candidate.join("cad.project.toml").is_file())
        .and_then(|candidate| fs::canonicalize(candidate).ok())
}

fn validate_generated_directory(root: &Path, relative: &Path) -> EditResult<()> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(EditError::HistoryUnavailable(format!(
                    "generated path contains a symlink: {}",
                    current.display()
                )));
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(EditError::HistoryUnavailable(format!(
                    "generated path is not a directory: {}",
                    current.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(source) => {
                return Err(EditError::Read {
                    path: current,
                    source,
                });
            }
        }
    }
    Ok(())
}

fn prepare_source_journal(
    root: &Path,
    target: &Path,
    staging: &Path,
    desired: &[u8],
    expected_revision: &str,
    operation: &str,
) -> EditResult<PathBuf> {
    let root = fs::canonicalize(root).map_err(|source| EditError::Read {
        path: root.to_path_buf(),
        source,
    })?;
    validate_generated_directory(&root, Path::new("build/.cad-transactions"))?;
    validate_generated_directory(&root, Path::new("build/.cad-recovery"))?;
    let transaction_root = root
        .join("build/.cad-transactions")
        .join(Ulid::new().to_string());
    fs::create_dir_all(&transaction_root).map_err(|source| EditError::Write {
        path: transaction_root.clone(),
        source,
    })?;
    validate_generated_directory(&root, Path::new("build/.cad-transactions"))?;
    let canonical_parent =
        fs::canonicalize(target.parent().ok_or_else(|| {
            EditError::InvalidEntity("transaction target has no parent".to_owned())
        })?)
        .map_err(|source| EditError::Read {
            path: target.to_path_buf(),
            source,
        })?;
    let normalized_target = canonical_parent.join(target.file_name().ok_or_else(|| {
        EditError::InvalidEntity("transaction target has no file name".to_owned())
    })?);
    let relative_target = normalized_target.strip_prefix(&root).map_err(|_| {
        EditError::InvalidEntity("transaction target escapes the project root".to_owned())
    })?;
    let journal = SourceTransactionJournal {
        target: relative_target.to_string_lossy().replace('\\', "/"),
        staging: staging.to_string_lossy().into_owned(),
        expected_revision: expected_revision.to_owned(),
        desired_revision: revision(desired),
        expected_exists: true,
        desired_exists: true,
        operation: operation.to_owned(),
    };
    let desired_path = transaction_root.join("desired.bin");
    fs::write(&desired_path, desired).map_err(|source| EditError::Write {
        path: desired_path.clone(),
        source,
    })?;
    fs::File::open(&desired_path)
        .and_then(|file| file.sync_all())
        .map_err(|source| EditError::Write {
            path: desired_path,
            source,
        })?;
    let manifest_path = transaction_root.join("manifest.json");
    let manifest = serde_json::to_vec_pretty(&journal)
        .map_err(|error| EditError::HistoryUnavailable(error.to_string()))?;
    fs::write(&manifest_path, manifest).map_err(|source| EditError::Write {
        path: manifest_path.clone(),
        source,
    })?;
    fs::File::open(&manifest_path)
        .and_then(|file| file.sync_all())
        .map_err(|source| EditError::Write {
            path: manifest_path,
            source,
        })?;
    sync_directory(&transaction_root)?;
    if let Some(parent) = transaction_root.parent() {
        sync_directory(parent)?;
    }
    Ok(transaction_root)
}

fn prepare_delete_journal(
    root: &Path,
    target: &Path,
    staging: &Path,
    expected_revision: &str,
    operation: &str,
) -> EditResult<PathBuf> {
    let transaction =
        prepare_source_journal(root, target, staging, &[], expected_revision, operation)?;
    let manifest_path = transaction.join("manifest.json");
    let mut journal: SourceTransactionJournal =
        serde_json::from_slice(&fs::read(&manifest_path).map_err(|source| EditError::Read {
            path: manifest_path.clone(),
            source,
        })?)
        .map_err(|error| EditError::HistoryCorrupt(error.to_string()))?;
    journal.desired_exists = false;
    write_history_file(
        &manifest_path,
        &serde_json::to_vec_pretty(&journal)
            .map_err(|error| EditError::HistoryCorrupt(error.to_string()))?,
    )?;
    sync_directory(&transaction)?;
    Ok(transaction)
}

fn preserve_source_conflict(
    project_root: Option<&Path>,
    target: &Path,
    desired: &[u8],
    staging: &Path,
    journal: Option<&Path>,
) -> EditResult<PathBuf> {
    let recovery_path = project_root
        .map(|root| root.join("build/.cad-recovery"))
        .unwrap_or_else(|| {
            target
                .parent()
                .unwrap_or(Path::new("."))
                .join(".cad-recovery")
        })
        .join(Ulid::new().to_string());
    fs::create_dir_all(&recovery_path).map_err(|source| EditError::Write {
        path: recovery_path.clone(),
        source,
    })?;
    if let Some(root) = project_root {
        validate_generated_directory(root, Path::new("build/.cad-recovery"))?;
    }
    let mut candidates = vec![("desired.bin", desired.to_vec())];
    if let Some(bytes) = read_recovery_candidate(target)? {
        candidates.push(("target.bin", bytes));
    }
    if let Some(bytes) = read_recovery_candidate(staging)? {
        candidates.push(("staging.bin", bytes));
    }
    let recovery_manifest = serde_json::to_vec_pretty(&serde_json::json!({
        "target": target.to_string_lossy(),
        "desired": "desired.bin",
        "observed_target": "target.bin",
        "displaced_staging": "staging.bin"
    }))
    .map_err(|error| EditError::HistoryUnavailable(error.to_string()))?;
    candidates.push(("manifest.json", recovery_manifest));
    for (name, content) in candidates {
        let output = recovery_path.join(name);
        fs::write(&output, content).map_err(|source| EditError::Write {
            path: output.clone(),
            source,
        })?;
        fs::File::open(&output)
            .and_then(|file| file.sync_all())
            .map_err(|source| EditError::Write {
                path: output,
                source,
            })?;
    }
    sync_directory(&recovery_path)?;
    if let Some(parent) = recovery_path.parent() {
        sync_directory(parent)?;
    }
    if staging.exists()
        && fs::remove_file(staging).is_ok()
        && let Some(parent) = staging.parent()
    {
        let _ = sync_directory(parent);
    }
    if let Some(journal) = journal
        && fs::remove_dir_all(journal).is_ok()
        && let Some(parent) = journal.parent()
    {
        sync_directory(parent)?;
    }
    Ok(recovery_path)
}

fn read_recovery_candidate(path: &Path) -> EditResult<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(EditError::Read {
            path: path.to_path_buf(),
            source,
        }),
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn exchange_paths(left: &Path, right: &Path) -> EditResult<()> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        left,
        rustix::fs::CWD,
        right,
        rustix::fs::RenameFlags::EXCHANGE,
    )
    .map_err(|error| EditError::Write {
        path: right.to_path_buf(),
        source: std::io::Error::from(error),
    })
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn exchange_paths(_left: &Path, _right: &Path) -> EditResult<()> {
    Err(EditError::AtomicExchangeUnsupported)
}

pub fn recover_source_transactions(project_root: impl AsRef<Path>) -> EditResult<()> {
    let root = fs::canonicalize(project_root.as_ref()).map_err(|source| EditError::Read {
        path: project_root.as_ref().to_path_buf(),
        source,
    })?;
    validate_generated_directory(&root, Path::new("build/.cad-transactions"))?;
    let transactions = root.join("build/.cad-transactions");
    let entries = if transactions.exists() {
        fs::read_dir(&transactions)
            .map_err(|source| EditError::Read {
                path: transactions.clone(),
                source,
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| EditError::Read {
                path: transactions.clone(),
                source,
            })?
    } else {
        Vec::new()
    };
    for entry in entries {
        let file_type = entry.file_type().map_err(|source| EditError::Read {
            path: entry.path(),
            source,
        })?;
        if file_type.is_symlink() || !file_type.is_dir() {
            return Err(EditError::HistoryCorrupt(format!(
                "transaction entry is not a directory: {}",
                entry.path().display()
            )));
        }
        let transaction = entry.path();
        let manifest_path = transaction.join("manifest.json");
        validate_recovery_regular_file(&manifest_path)?;
        let journal: SourceTransactionJournal =
            serde_json::from_slice(&fs::read(&manifest_path).map_err(|source| {
                EditError::Read {
                    path: manifest_path.clone(),
                    source,
                }
            })?)
            .map_err(|error| EditError::HistoryCorrupt(error.to_string()))?;
        if !journal.expected_exists {
            return Err(EditError::HistoryCorrupt(
                "source recovery journal has an unsupported missing expected target".to_owned(),
            ));
        }
        let relative = Path::new(&journal.target);
        validate_source_path_components(&root, relative)?;
        let target = root.join(relative);
        let target_revision = fs::read(&target).map(|bytes| revision(&bytes)).ok();
        let staging_candidate = PathBuf::from(&journal.staging);
        let staging = if staging_candidate.exists() {
            validate_recovery_staging_path(&root, &target, &staging_candidate)?
        } else {
            validate_recovery_staging_location(&root, &target, &staging_candidate)?;
            staging_candidate
        };
        let staging_revision = fs::read(&staging).map(|bytes| revision(&bytes)).ok();
        if !journal.desired_exists {
            if target_revision.as_deref() == Some(&journal.expected_revision)
                && staging_revision.is_none()
            {
                fs::remove_dir_all(&transaction).map_err(|source| EditError::Write {
                    path: transaction,
                    source,
                })?;
                sync_directory(&transactions)?;
                continue;
            }
            if target_revision.is_none()
                && (staging_revision.as_deref() == Some(&journal.expected_revision)
                    || staging_revision.is_none())
            {
                let _ = fs::remove_file(&staging);
                fs::remove_dir_all(&transaction).map_err(|source| EditError::Write {
                    path: transaction,
                    source,
                })?;
                sync_directory(&transactions)?;
                continue;
            }
            let recovery_path =
                preserve_source_conflict(Some(&root), &target, &[], &staging, Some(&transaction))?;
            return Err(EditError::RecoveryRequired { recovery_path });
        }
        if target_revision.as_deref() == Some(&journal.desired_revision)
            && (staging_revision.as_deref() == Some(&journal.expected_revision)
                || staging_revision.is_none())
        {
            fs::remove_dir_all(&transaction).map_err(|source| EditError::Write {
                path: transaction,
                source,
            })?;
            sync_directory(&transactions)?;
            let _ = fs::remove_file(&staging);
        } else if target_revision.as_deref() == Some(&journal.expected_revision) {
            fs::remove_dir_all(&transaction).map_err(|source| EditError::Write {
                path: transaction,
                source,
            })?;
            sync_directory(&transactions)?;
            let _ = fs::remove_file(&staging);
        } else {
            let desired_path = transaction.join("desired.bin");
            validate_recovery_regular_file(&desired_path)?;
            let desired = fs::read(&desired_path).map_err(|source| EditError::Read {
                path: desired_path,
                source,
            })?;
            let recovery_path = preserve_source_conflict(
                Some(&root),
                &target,
                &desired,
                &staging,
                Some(&transaction),
            )?;
            return Err(EditError::RecoveryRequired { recovery_path });
        }
    }
    recover_history_transactions(&root)
}

fn recover_history_transactions(project_root: &Path) -> EditResult<()> {
    let root = history_root(project_root, "recovery")?;
    let transactions = root.join("transactions");
    if !transactions.exists() {
        return Ok(());
    }
    let mut entries = fs::read_dir(&transactions)
        .map_err(|source| EditError::Read {
            path: transactions.clone(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| EditError::Read {
            path: transactions.clone(),
            source,
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type().map_err(|source| EditError::Read {
            path: path.clone(),
            source,
        })?;
        if file_type.is_symlink() || !file_type.is_file() {
            return Err(EditError::HistoryCorrupt(format!(
                "history transaction is not a regular file: {}",
                path.display()
            )));
        }
        let journal: HistoryTransactionJournal =
            serde_json::from_slice(&fs::read(&path).map_err(|source| EditError::Read {
                path: path.clone(),
                source,
            })?)
            .map_err(|error| EditError::HistoryCorrupt(format!("{}: {error}", path.display())))?;
        let metadata = history_entry(&root, &journal.history_id)?;
        let undo = match journal.transaction {
            HistoryTransactionKind::Commit => None,
            HistoryTransactionKind::Restore { undo } => Some(undo),
        };
        let mut files = Vec::new();
        let mut all_expected = true;
        let mut any_desired = false;
        for file in &metadata.files {
            let expected_before = match undo {
                None => true,
                Some(undo) => !undo,
            };
            let desired_before = undo.is_some_and(|undo| undo);
            let expected =
                history_snapshot_input(&root, &journal.history_id, file, expected_before)?;
            let desired = history_snapshot_input(&root, &journal.history_id, file, desired_before)?;
            let current = read_history_file(project_root, &file.relative_path)?;
            let matches_expected = history_file_matches(&current, &expected);
            let matches_desired = history_file_matches(&current, &desired);
            if !matches_expected && !matches_desired {
                let recovery_path = preserve_history_recovery(project_root, &journal, &metadata)?;
                return Err(EditError::RecoveryRequired { recovery_path });
            }
            all_expected &= matches_expected;
            any_desired |= matches_desired;
            files.push((current, expected, desired));
        }
        if all_expected && !any_desired {
            fs::remove_file(&path).map_err(|source| EditError::Write {
                path: path.clone(),
                source,
            })?;
            if undo.is_none() {
                fs::remove_dir_all(root.join("entries").join(&journal.history_id)).map_err(
                    |source| EditError::Write {
                        path: root.join("entries").join(&journal.history_id),
                        source,
                    },
                )?;
            }
            sync_directory(&transactions)?;
            continue;
        }
        for (current, _expected, desired) in files {
            if !history_file_matches(&current, &desired) {
                publish_history_file(project_root, &desired, &current)?;
            }
        }
        recover_history_index(&root, &journal)?;
        fs::remove_file(&path).map_err(|source| EditError::Write {
            path: path.clone(),
            source,
        })?;
        sync_directory(&transactions)?;
    }
    Ok(())
}

fn history_snapshot_input(
    root: &Path,
    history_id: &str,
    file: &HistoryFileSnapshot,
    before: bool,
) -> EditResult<HistoryFileInput> {
    let snapshot = if before {
        file.before_snapshot.as_ref()
    } else {
        file.after_snapshot.as_ref()
    };
    let bytes = match snapshot {
        Some(relative) => {
            let path = root.join("entries").join(history_id).join(relative);
            fs::read(&path).map_err(|source| EditError::Read { path, source })?
        }
        None => Vec::new(),
    };
    let expected_revision = if before {
        &file.before_revision
    } else {
        &file.after_revision
    };
    if revision(&bytes) != *expected_revision {
        return Err(EditError::HistoryCorrupt(format!(
            "snapshot revision mismatch for {}",
            file.relative_path
        )));
    }
    Ok(HistoryFileInput {
        relative_path: file.relative_path.clone(),
        exists: snapshot.is_some(),
        bytes,
        permissions: if before {
            file.before_permissions.clone()
        } else {
            file.after_permissions.clone()
        },
    })
}

fn history_file_matches(left: &HistoryFileInput, right: &HistoryFileInput) -> bool {
    left.exists == right.exists && revision(&left.bytes) == revision(&right.bytes)
}

fn recover_history_index(root: &Path, journal: &HistoryTransactionJournal) -> EditResult<()> {
    let mut index = read_history_index(root)?;
    match journal.transaction {
        HistoryTransactionKind::Commit => {
            if !index.undo.contains(&journal.history_id) {
                index.redo.clear();
                index.undo.push(journal.history_id.clone());
            }
        }
        HistoryTransactionKind::Restore { undo: true } => {
            if index.undo.last() == Some(&journal.history_id) {
                index.undo.pop();
                index.redo.push(journal.history_id.clone());
            } else if index.redo.last() != Some(&journal.history_id) {
                return Err(EditError::HistoryCorrupt(
                    "undo recovery index does not contain the transaction".to_owned(),
                ));
            }
        }
        HistoryTransactionKind::Restore { undo: false } => {
            if index.redo.last() == Some(&journal.history_id) {
                index.redo.pop();
                index.undo.push(journal.history_id.clone());
            } else if index.undo.last() != Some(&journal.history_id) {
                return Err(EditError::HistoryCorrupt(
                    "redo recovery index does not contain the transaction".to_owned(),
                ));
            }
        }
    }
    write_history_index(root, &index)
}

fn preserve_history_recovery(
    project_root: &Path,
    journal: &HistoryTransactionJournal,
    metadata: &HistoryMetadata,
) -> EditResult<PathBuf> {
    let root = fs::canonicalize(project_root).map_err(|source| EditError::Read {
        path: project_root.to_path_buf(),
        source,
    })?;
    validate_generated_directory(&root, Path::new("build/.cad-recovery"))?;
    let recovery = root
        .join("build/.cad-recovery")
        .join(format!("history-{}", journal.history_id));
    fs::create_dir_all(&recovery).map_err(|source| EditError::Write {
        path: recovery.clone(),
        source,
    })?;
    write_history_file(
        &recovery.join("transaction.json"),
        &serde_json::to_vec_pretty(journal)
            .map_err(|error| EditError::HistoryCorrupt(error.to_string()))?,
    )?;
    write_history_file(
        &recovery.join("metadata.json"),
        &serde_json::to_vec_pretty(metadata)
            .map_err(|error| EditError::HistoryCorrupt(error.to_string()))?,
    )?;
    for file in &metadata.files {
        let current = read_history_file(project_root, &file.relative_path)?;
        if current.exists {
            write_history_file(
                &recovery.join("current").join(&file.relative_path),
                &current.bytes,
            )?;
        }
    }
    sync_directory(&recovery)?;
    Ok(recovery)
}

fn validate_recovery_regular_file(path: &Path) -> EditResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(|source| EditError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(EditError::HistoryCorrupt(format!(
            "transaction artifact is not a regular file: {}",
            path.display()
        )));
    }
    Ok(())
}

fn validate_recovery_staging_path(
    root: &Path,
    target: &Path,
    staging: &Path,
) -> EditResult<PathBuf> {
    let metadata = fs::symlink_metadata(staging).map_err(|source| EditError::Read {
        path: staging.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(EditError::HistoryCorrupt(format!(
            "transaction staging path is not a regular file: {}",
            staging.display()
        )));
    }
    let canonical_staging = fs::canonicalize(staging).map_err(|source| EditError::Read {
        path: staging.to_path_buf(),
        source,
    })?;
    let canonical_target_parent =
        fs::canonicalize(target.parent().ok_or_else(|| {
            EditError::HistoryCorrupt("transaction target has no parent".to_owned())
        })?)
        .map_err(|source| EditError::Read {
            path: target.to_path_buf(),
            source,
        })?;
    if !canonical_staging.starts_with(root)
        || canonical_staging.parent() != Some(canonical_target_parent.as_path())
    {
        return Err(EditError::HistoryCorrupt(format!(
            "transaction staging path escapes its source directory: {}",
            staging.display()
        )));
    }
    Ok(canonical_staging)
}

fn validate_recovery_staging_location(
    root: &Path,
    target: &Path,
    staging: &Path,
) -> EditResult<()> {
    let staging_parent = fs::canonicalize(staging.parent().ok_or_else(|| {
        EditError::HistoryCorrupt("transaction staging path has no parent".to_owned())
    })?)
    .map_err(|source| EditError::Read {
        path: staging.to_path_buf(),
        source,
    })?;
    let target_parent =
        fs::canonicalize(target.parent().ok_or_else(|| {
            EditError::HistoryCorrupt("transaction target has no parent".to_owned())
        })?)
        .map_err(|source| EditError::Read {
            path: target.to_path_buf(),
            source,
        })?;
    if staging_parent != target_parent || !staging_parent.starts_with(root) {
        return Err(EditError::HistoryCorrupt(format!(
            "transaction staging path escapes its source directory: {}",
            staging.display()
        )));
    }
    Ok(())
}

/// Publishes generated bytes from a same-directory staging file. When
/// `overwrite` is false the hard-link step provides an atomic no-replace
/// publish and returns `AlreadyExists` on a concurrent destination creation.
pub fn atomic_publish(path: &Path, bytes: &[u8], overwrite: bool) -> EditResult<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| EditError::Write {
        path: parent.to_path_buf(),
        source,
    })?;
    let staging = tempfile::NamedTempFile::new_in(parent).map_err(|source| EditError::Write {
        path: parent.to_path_buf(),
        source,
    })?;
    fs::write(staging.path(), bytes).map_err(|source| EditError::Write {
        path: staging.path().to_path_buf(),
        source,
    })?;
    staging
        .as_file()
        .sync_all()
        .map_err(|source| EditError::Write {
            path: staging.path().to_path_buf(),
            source,
        })?;
    if overwrite {
        staging.persist(path).map_err(|error| EditError::Write {
            path: path.to_path_buf(),
            source: error.error,
        })?;
    } else if let Err(source) = fs::hard_link(staging.path(), path) {
        if source.kind() == std::io::ErrorKind::AlreadyExists {
            return Err(EditError::Write {
                path: path.to_path_buf(),
                source,
            });
        }
        return Err(EditError::Write {
            path: path.to_path_buf(),
            source,
        });
    }
    sync_directory(parent)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> EditResult<()> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| EditError::Write {
            path: path.to_path_buf(),
            source,
        })
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> EditResult<()> {
    Ok(())
}

pub fn translate_entity(entity: &Entity, delta: Point) -> EditResult<Entity> {
    let mut value = serde_json::to_value(entity)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    let kind = value["type"].as_str().unwrap_or_default().to_owned();
    let keys: &[&str] = match kind.as_str() {
        "line" | "dimension" => &["p1", "p2"],
        "arc" | "circle" | "ellipse" | "curve_solid" => &["center"],
        "text" | "point" | "block_ref" => &["at"],
        "polyline" | "solid" | "hatch" => &[],
        _ => {
            return Err(EditError::InvalidEntity(format!(
                "unsupported entity type {kind:?}"
            )));
        }
    };
    for key in keys {
        translate_json_point(&mut value[*key], delta)?;
    }
    if matches!(kind.as_str(), "polyline" | "solid") {
        let points = value["points"]
            .as_array_mut()
            .ok_or_else(|| EditError::InvalidEntity("points must be an array".to_owned()))?;
        for point in points {
            translate_json_point(point, delta)?;
        }
    }
    if kind == "hatch" {
        transform_json_points(&mut value["loops"], &|point| {
            [point[0] + delta[0], point[1] + delta[1]]
        });
    }
    serde_json::from_value(value).map_err(|error| EditError::InvalidEntity(error.to_string()))
}

fn translate_json_point(value: &mut Value, delta: Point) -> EditResult<()> {
    let point = value
        .as_array_mut()
        .filter(|point| point.len() == 2)
        .ok_or_else(|| EditError::InvalidEntity("point must contain two coordinates".to_owned()))?;
    for axis in 0..2 {
        let coordinate = point[axis].as_f64().ok_or_else(|| {
            EditError::InvalidEntity("point coordinate is not numeric".to_owned())
        })?;
        point[axis] = Value::from(coordinate + delta[axis]);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SnapKind {
    Endpoint,
    Midpoint,
    Intersection,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SnapCandidate {
    pub kind: SnapKind,
    pub point: Point,
    pub distance: f64,
}

#[derive(Debug, Clone, Copy)]
struct SnapSegment {
    start: Point,
    end: Point,
}

impl RTreeObject for SnapSegment {
    type Envelope = AABB<Point>;

    fn envelope(&self) -> Self::Envelope {
        AABB::from_corners(
            [
                self.start[0].min(self.end[0]),
                self.start[1].min(self.end[1]),
            ],
            [
                self.start[0].max(self.end[0]),
                self.start[1].max(self.end[1]),
            ],
        )
    }
}

pub struct SnapIndex {
    segments: RTree<SnapSegment>,
    points: Vec<(SnapKind, Point)>,
}

impl SnapIndex {
    pub fn build(project: &ProjectSource, drawing: &str) -> EditResult<Self> {
        let drawing = project
            .drawings
            .iter()
            .find(|candidate| candidate.name == drawing)
            .ok_or_else(|| EditError::DrawingNotFound(drawing.to_owned()))?;
        let mut segments = Vec::new();
        let mut points = Vec::new();
        for record in &drawing.entities {
            if !layer_visible(project, record.entity.layer()) {
                continue;
            }
            append_segments(&record.entity, &mut segments);
            append_direct_snap_points(&record.entity, &mut points);
        }
        for segment in &segments {
            points.push((SnapKind::Endpoint, segment.start));
            points.push((SnapKind::Endpoint, segment.end));
            points.push((SnapKind::Midpoint, midpoint(segment.start, segment.end)));
        }
        let mut seen = BTreeSet::new();
        points.retain(|(kind, point)| seen.insert((*kind, point[0].to_bits(), point[1].to_bits())));
        Ok(Self {
            segments: RTree::bulk_load(segments),
            points,
        })
    }

    #[must_use]
    pub fn query(&self, point: Point, tolerance: f64, modes: &[SnapKind]) -> Option<SnapCandidate> {
        if tolerance <= 0.0 || !tolerance.is_finite() {
            return None;
        }
        let enabled = modes.iter().copied().collect::<BTreeSet<_>>();
        let mut candidates = self
            .points
            .iter()
            .filter(|(kind, _)| enabled.contains(kind))
            .filter_map(|(kind, candidate)| snap_candidate(*kind, *candidate, point, tolerance))
            .collect::<Vec<_>>();
        if enabled.contains(&SnapKind::Intersection) {
            let envelope = AABB::from_corners(
                [point[0] - tolerance, point[1] - tolerance],
                [point[0] + tolerance, point[1] + tolerance],
            );
            let local = self
                .segments
                .locate_in_envelope_intersecting(envelope)
                .copied()
                .collect::<Vec<_>>();
            for left in 0..local.len() {
                for right in left + 1..local.len() {
                    if let Some(intersection) = segment_intersection(local[left], local[right])
                        && let Some(candidate) =
                            snap_candidate(SnapKind::Intersection, intersection, point, tolerance)
                    {
                        candidates.push(candidate);
                    }
                }
            }
        }
        candidates.into_iter().min_by(|left, right| {
            left.distance
                .partial_cmp(&right.distance)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }
}

fn layer_visible(project: &ProjectSource, layer_id: &str) -> bool {
    project.layers.layers.get(layer_id).is_some_and(|layer| {
        layer.visible
            && layer
                .group
                .as_ref()
                .and_then(|group| project.layers.groups.get(group))
                .is_none_or(|group| group.visible)
    })
}

fn append_segments(entity: &Entity, output: &mut Vec<SnapSegment>) {
    match entity {
        Entity::Line { p1, p2, .. } | Entity::Dimension { p1, p2, .. } => {
            output.push(SnapSegment {
                start: *p1,
                end: *p2,
            });
        }
        Entity::Polyline { points, closed, .. } => {
            output.extend(points.windows(2).map(|pair| SnapSegment {
                start: pair[0],
                end: pair[1],
            }));
            if *closed && points.len() > 2 {
                output.push(SnapSegment {
                    start: *points.last().expect("nonempty"),
                    end: points[0],
                });
            }
        }
        Entity::Solid { points, .. } => {
            output.extend(points.windows(2).map(|pair| SnapSegment {
                start: pair[0],
                end: pair[1],
            }));
            if points.len() > 2 {
                output.push(SnapSegment {
                    start: *points.last().expect("nonempty"),
                    end: points[0],
                });
            }
        }
        _ => {}
    }
}

fn append_direct_snap_points(entity: &Entity, output: &mut Vec<(SnapKind, Point)>) {
    match entity {
        Entity::Arc {
            center,
            radius,
            start_deg,
            end_deg,
            ..
        } => {
            output.push((
                SnapKind::Endpoint,
                polar_point(*center, *radius, *start_deg),
            ));
            output.push((SnapKind::Endpoint, polar_point(*center, *radius, *end_deg)));
            output.push((
                SnapKind::Midpoint,
                polar_point(*center, *radius, (*start_deg + *end_deg) / 2.0),
            ));
        }
        Entity::Ellipse {
            center,
            radius_x,
            radius_y,
            rotation_deg,
            start_deg,
            end_deg,
            ..
        } => {
            output.push((
                SnapKind::Endpoint,
                cad_model::ellipse_point(*center, *radius_x, *radius_y, *rotation_deg, *start_deg),
            ));
            output.push((
                SnapKind::Endpoint,
                cad_model::ellipse_point(*center, *radius_x, *radius_y, *rotation_deg, *end_deg),
            ));
            output.push((
                SnapKind::Midpoint,
                cad_model::ellipse_point(
                    *center,
                    *radius_x,
                    *radius_y,
                    *rotation_deg,
                    (*start_deg + *end_deg) / 2.0,
                ),
            ));
        }
        Entity::Circle { center, .. } | Entity::Point { at: center, .. } => {
            output.push((SnapKind::Midpoint, *center));
        }
        Entity::Text { at, .. } | Entity::BlockRef { at, .. } => {
            output.push((SnapKind::Endpoint, *at));
        }
        _ => {}
    }
}

fn polar_point(center: Point, radius: f64, degrees: f64) -> Point {
    let radians = degrees.to_radians();
    [
        center[0] + radius * radians.cos(),
        center[1] + radius * radians.sin(),
    ]
}

fn midpoint(left: Point, right: Point) -> Point {
    [(left[0] + right[0]) / 2.0, (left[1] + right[1]) / 2.0]
}

fn snap_candidate(
    kind: SnapKind,
    candidate: Point,
    point: Point,
    tolerance: f64,
) -> Option<SnapCandidate> {
    let distance = ((candidate[0] - point[0]).powi(2) + (candidate[1] - point[1]).powi(2)).sqrt();
    (distance <= tolerance).then_some(SnapCandidate {
        kind,
        point: candidate,
        distance,
    })
}

fn segment_intersection(left: SnapSegment, right: SnapSegment) -> Option<Point> {
    let line = |segment: SnapSegment| {
        Line::new(
            Coord {
                x: segment.start[0],
                y: segment.start[1],
            },
            Coord {
                x: segment.end[0],
                y: segment.end[1],
            },
        )
    };
    match line_intersection(line(left), line(right))? {
        LineIntersection::SinglePoint { intersection, .. } => {
            Some([intersection.x, intersection.y])
        }
        LineIntersection::Collinear { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_transaction_rejects_jww_interop_files() {
        let error = validate_source_relative_path(Path::new("interop/jww/original.jww"))
            .expect_err("JWW provenance is not editable source");
        assert!(matches!(error, EditError::InvalidEntity(_)));
    }

    #[test]
    fn source_transaction_publishes_and_cleans_its_journal() {
        let temp = test_project(false);
        let relative = Path::new("drawings/plan/entities.ndjson");
        let path = temp.path().join(relative);
        let original = fs::read(&path).expect("original");
        let permissions =
            capture_permissions(&fs::metadata(&path).expect("metadata").permissions());
        SourceTransaction::new(temp.path(), "test.replace")
            .expect("transaction")
            .replace(
                relative,
                b"replacement\n",
                &revision(&original),
                &permissions,
            )
            .expect("replace");

        assert_eq!(fs::read(&path).expect("replacement"), b"replacement\n");
        assert_eq!(
            fs::read_dir(temp.path().join("build/.cad-transactions"))
                .expect("transaction directory")
                .count(),
            0
        );
    }

    #[cfg(unix)]
    #[test]
    fn source_transaction_rejects_a_symlinked_source_file() {
        use std::os::unix::fs::symlink;

        let temp = test_project(false);
        let relative = Path::new("drawings/plan/entities.ndjson");
        let path = temp.path().join(relative);
        let external = tempfile::NamedTempFile::new().expect("external");
        fs::write(external.path(), b"external\n").expect("external bytes");
        fs::remove_file(&path).expect("remove source fixture");
        symlink(external.path(), &path).expect("source symlink");
        let permissions = capture_permissions(
            &fs::metadata(external.path())
                .expect("external metadata")
                .permissions(),
        );

        let result = SourceTransaction::new(temp.path(), "test.replace")
            .expect("transaction")
            .replace(
                relative,
                b"replacement\n",
                &revision(b"external\n"),
                &permissions,
            );
        assert!(matches!(result, Err(EditError::InvalidEntity(_))));
        assert_eq!(fs::read(external.path()).expect("external"), b"external\n");
    }

    #[test]
    fn ambiguous_crash_recovery_preserves_every_observed_version() {
        let temp = test_project(false);
        let target = entities_path(temp.path(), "plan");
        let original = fs::read(&target).expect("original");
        let staging =
            tempfile::NamedTempFile::new_in(target.parent().expect("parent")).expect("staging");
        fs::write(staging.path(), &original).expect("staging bytes");
        let transaction = prepare_source_journal(
            temp.path(),
            &target,
            staging.path(),
            b"desired\n",
            &revision(&original),
            "test.recovery",
        )
        .expect("journal");
        fs::write(&target, b"external\n").expect("external edit");

        let error = recover_source_transactions(temp.path()).expect_err("recovery conflict");
        let EditError::RecoveryRequired { recovery_path } = error else {
            panic!("expected recovery-required error");
        };
        assert_eq!(fs::read(&target).expect("target"), b"external\n");
        assert_eq!(
            fs::read(recovery_path.join("desired.bin")).expect("desired recovery"),
            b"desired\n"
        );
        assert_eq!(
            fs::read(recovery_path.join("target.bin")).expect("target recovery"),
            b"external\n"
        );
        assert!(recovery_path.join("manifest.json").is_file());
        assert!(!transaction.exists());
    }

    #[test]
    fn completed_exchange_recovery_is_idempotent() {
        let temp = test_project(false);
        let target = entities_path(temp.path(), "plan");
        let original = fs::read(&target).expect("original");
        let staging =
            tempfile::NamedTempFile::new_in(target.parent().expect("parent")).expect("staging");
        fs::write(staging.path(), b"desired\n").expect("staging bytes");
        prepare_source_journal(
            temp.path(),
            &target,
            staging.path(),
            b"desired\n",
            &revision(&original),
            "test.recovery",
        )
        .expect("journal");
        exchange_paths(staging.path(), &target).expect("simulated committed exchange");
        let staging_path = staging.keep().expect("retain crash staging").1;

        recover_source_transactions(temp.path()).expect("first recovery");
        recover_source_transactions(temp.path()).expect("second recovery");
        assert_eq!(fs::read(&target).expect("target"), b"desired\n");
        assert!(!staging_path.exists());
    }

    #[test]
    fn completed_exchange_recovery_accepts_an_already_removed_staging_file() {
        let temp = test_project(false);
        let target = entities_path(temp.path(), "plan");
        let original = fs::read(&target).expect("original");
        let staging =
            tempfile::NamedTempFile::new_in(target.parent().expect("parent")).expect("staging");
        fs::write(staging.path(), b"desired\n").expect("staging bytes");
        prepare_source_journal(
            temp.path(),
            &target,
            staging.path(),
            b"desired\n",
            &revision(&original),
            "test.recovery_without_staging",
        )
        .expect("journal");
        exchange_paths(staging.path(), &target).expect("simulated exchange");
        let staging_path = staging.keep().expect("retain staging").1;
        fs::remove_file(&staging_path).expect("simulated durable staging cleanup");

        recover_source_transactions(temp.path()).expect("completed recovery");

        assert_eq!(fs::read(&target).expect("target"), b"desired\n");
        assert_eq!(
            fs::read_dir(temp.path().join("build/.cad-transactions"))
                .expect("transactions")
                .count(),
            0
        );
    }

    #[test]
    fn completed_delete_recovery_is_idempotent() {
        let temp = test_project(false);
        let relative = "comments/plan.ndjson";
        let target = temp.path().join(relative);
        fs::create_dir_all(target.parent().expect("comments parent")).expect("comments dir");
        fs::write(&target, b"delete me\n").expect("comment source");
        let placeholder = tempfile::NamedTempFile::new_in(target.parent().expect("parent"))
            .expect("staging placeholder");
        let (_, staging) = placeholder.keep().expect("staging path");
        fs::remove_file(&staging).expect("vacate staging path");
        prepare_delete_journal(
            temp.path(),
            &target,
            &staging,
            &revision(b"delete me\n"),
            "test.delete_recovery",
        )
        .expect("delete journal");
        fs::rename(&target, &staging).expect("simulated delete publish");

        recover_source_transactions(temp.path()).expect("first delete recovery");
        recover_source_transactions(temp.path()).expect("second delete recovery");

        assert!(!target.exists());
        assert!(!staging.exists());
        assert_eq!(
            fs::read_dir(temp.path().join("build/.cad-transactions"))
                .expect("transactions")
                .count(),
            0
        );
    }

    #[test]
    fn history_recovery_commits_source_published_before_index_update() {
        let temp = test_project(false);
        let relative = "drawings/plan/entities.ndjson";
        let path = temp.path().join(relative);
        let before = read_history_file(temp.path(), relative).expect("before source");
        let permissions = before.permissions.clone().expect("permissions");
        let desired = b"desired history bytes\n".to_vec();
        let mut stage = stage_history_transaction(
            temp.path(),
            "plan",
            "drawing",
            "test.crash_after_publish",
            &[],
            std::slice::from_ref(&before),
        )
        .expect("history stage");
        stage
            .prepare_after(&[HistoryFileInput {
                relative_path: relative.to_owned(),
                exists: true,
                bytes: desired.clone(),
                permissions: Some(permissions.clone()),
            }])
            .expect("after snapshot");
        stage.prepare_commit_journal().expect("commit journal");
        SourceTransaction::new(temp.path(), "test.crash_after_publish")
            .expect("source transaction")
            .replace(relative, &desired, &revision(&before.bytes), &permissions)
            .expect("source publish");
        stage.preserve_for_recovery();

        recover_source_transactions(temp.path()).expect("history recovery");
        recover_source_transactions(temp.path()).expect("idempotent history recovery");

        assert_eq!(fs::read(path).expect("desired source"), desired);
        let state = list_drawing_history(temp.path(), "plan").expect("history state");
        assert_eq!(state.undo.len(), 1);
        assert!(state.redo.is_empty());
    }

    #[test]
    fn history_recovery_aborts_a_journal_when_source_is_still_expected() {
        let temp = test_project(false);
        let relative = "drawings/plan/entities.ndjson";
        let before = read_history_file(temp.path(), relative).expect("before source");
        let mut stage = stage_history_transaction(
            temp.path(),
            "plan",
            "drawing",
            "test.crash_before_publish",
            &[],
            std::slice::from_ref(&before),
        )
        .expect("history stage");
        stage
            .prepare_after(&[HistoryFileInput {
                relative_path: relative.to_owned(),
                exists: true,
                bytes: b"unpublished desired\n".to_vec(),
                permissions: before.permissions.clone(),
            }])
            .expect("after snapshot");
        let history_id = stage.history_id().to_owned();
        stage.prepare_commit_journal().expect("commit journal");
        stage.preserve_for_recovery();

        recover_source_transactions(temp.path()).expect("abort recovery");

        assert_eq!(
            fs::read(temp.path().join(relative)).expect("expected source"),
            before.bytes
        );
        assert!(
            !temp
                .path()
                .join("build/.cad-history/entries")
                .join(history_id)
                .exists()
        );
        assert!(
            list_drawing_history(temp.path(), "plan")
                .expect("history state")
                .undo
                .is_empty()
        );
    }

    #[test]
    fn history_recovery_finishes_undo_published_before_index_update() {
        let temp = test_project(false);
        let original = fs::read(entities_path(temp.path(), "plan")).expect("original source");
        apply_test_edit(
            temp.path(),
            EditOperation::Translate {
                entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                delta: [1.0, 0.0],
                duplicate: false,
            },
        );
        let root = history_root(temp.path(), "plan").expect("history root");
        let index = read_history_index(&root).expect("history index");
        let history_id = index.undo.last().cloned().expect("undo entry");
        let metadata = history_entry(&root, &history_id).expect("metadata");
        let file = metadata.files.first().expect("source snapshot");
        let current = read_history_file(temp.path(), &file.relative_path).expect("current source");
        let desired =
            history_snapshot_input(&root, &history_id, file, true).expect("before snapshot");
        write_history_transaction_journal(
            &root,
            &HistoryTransactionJournal {
                history_id: history_id.clone(),
                transaction: HistoryTransactionKind::Restore { undo: true },
            },
        )
        .expect("restore journal");
        publish_history_file(temp.path(), &desired, &current).expect("source restore");

        recover_source_transactions(temp.path()).expect("undo recovery");

        assert_eq!(
            fs::read(entities_path(temp.path(), "plan")).expect("restored source"),
            original
        );
        let state = list_drawing_history(temp.path(), "plan").expect("history state");
        assert!(state.undo.is_empty());
        assert_eq!(state.redo.len(), 1);
        assert_eq!(state.redo[0].history_id, history_id);
    }

    #[test]
    fn recovery_rejects_an_external_staging_path_without_touching_it() {
        let temp = test_project(false);
        let target = entities_path(temp.path(), "plan");
        let original = fs::read(&target).expect("original");
        let staging =
            tempfile::NamedTempFile::new_in(target.parent().expect("parent")).expect("staging");
        fs::write(staging.path(), b"desired\n").expect("staging bytes");
        let transaction = prepare_source_journal(
            temp.path(),
            &target,
            staging.path(),
            b"desired\n",
            &revision(&original),
            "test.recovery",
        )
        .expect("journal");
        let external = tempfile::NamedTempFile::new().expect("external");
        fs::write(external.path(), b"do not delete").expect("external bytes");
        let manifest_path = transaction.join("manifest.json");
        let mut journal: SourceTransactionJournal =
            serde_json::from_slice(&fs::read(&manifest_path).expect("manifest")).expect("journal");
        journal.staging = external.path().to_string_lossy().into_owned();
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&journal).expect("journal JSON"),
        )
        .expect("malicious journal");

        assert!(matches!(
            recover_source_transactions(temp.path()),
            Err(EditError::HistoryCorrupt(_))
        ));
        assert_eq!(
            fs::read(external.path()).expect("external"),
            b"do not delete"
        );
        assert_eq!(fs::read(&target).expect("target"), original);
    }

    use std::fs;

    #[test]
    fn translates_every_entity_family() {
        for source in [
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0,0],"p2":[1,1]}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000001","type":"polyline","layer":"0-1","points":[[0,0],[1,1]],"closed":false}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000002","type":"circle","layer":"0-1","center":[0,0],"radius":2}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000003","type":"text","layer":"0-1","style":"note","at":[0,0],"rotation_deg":0,"value":"A"}"#,
        ] {
            let entity: Entity = serde_json::from_str(source).expect("entity");
            let moved = translate_entity(&entity, [5.0, -2.0]).expect("translate");
            let bbox = cad_model::entity_bbox(&moved).expect("bbox");
            assert!(bbox.max[0] >= 5.0);
        }
    }

    #[test]
    fn line_intersection_is_reported() {
        let left = SnapSegment {
            start: [0.0, 0.0],
            end: [10.0, 10.0],
        };
        let right = SnapSegment {
            start: [0.0, 10.0],
            end: [10.0, 0.0],
        };
        assert_eq!(segment_intersection(left, right), Some([5.0, 5.0]));
    }

    #[test]
    fn applies_atomic_translate_and_preserves_other_raw_lines() {
        let temp = test_project(false);
        let project = cad_model::load_project(temp.path()).expect("project");
        let state = editor_state(&project, "plan").expect("editor state");
        let result = apply_edit(
            temp.path(),
            &DrawingEditRequest {
                drawing: "plan".to_owned(),
                expected_revision: state.revision,
                operation: EditOperation::Translate {
                    entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                    delta: [5.0, 2.0],
                    duplicate: false,
                },
            },
        )
        .expect("translate");
        let text = fs::read_to_string(entities_path(temp.path(), "plan")).expect("entities");
        assert!(text.lines().next().expect("line").contains("[5.0,2.0]"));
        assert!(text.contains("  \"type\": \"line\""));
        assert_eq!(
            result.entity_id.as_deref(),
            Some("ent_01JZ0000000000000000000000")
        );
    }

    #[test]
    fn preserves_crlf_unrelated_lines_and_trailing_newline_policy_for_all_edits() {
        let temp = test_project(false);
        let path = entities_path(temp.path(), "plan");
        let original = fs::read_to_string(&path).expect("entities");
        let crlf_without_trailing = original.trim_end_matches('\n').replace('\n', "\r\n");
        fs::write(&path, &crlf_without_trailing).expect("CRLF entities");
        let formatted_line = crlf_without_trailing
            .split("\r\n")
            .nth(1)
            .expect("formatted line")
            .to_owned();

        let mut replacement = editor_state(
            &cad_model::load_project(temp.path()).expect("project"),
            "plan",
        )
        .expect("state")
        .entities[0]
            .clone();
        replacement["p2"] = serde_json::json!([12.0, 0.0]);
        apply_test_edit(
            temp.path(),
            EditOperation::Replace {
                entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                entity: replacement,
            },
        );
        assert!(
            fs::read_to_string(&path)
                .expect("replace output")
                .contains(&formatted_line)
        );

        let created = apply_test_edit(
            temp.path(),
            EditOperation::Create {
                entity: serde_json::json!({
                    "type": "point",
                    "layer": "0-1",
                    "at": [2.0, 3.0]
                }),
            },
        )
        .entity_id
        .expect("created id");
        apply_test_edit(
            temp.path(),
            EditOperation::Translate {
                entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                delta: [1.0, 0.0],
                duplicate: true,
            },
        );
        apply_test_edit(temp.path(), EditOperation::Delete { entity_id: created });

        let output = fs::read_to_string(&path).expect("final output");
        assert!(!output.ends_with('\n'));
        assert!(output.contains(&formatted_line));
        assert!(
            output
                .as_bytes()
                .iter()
                .enumerate()
                .filter(|(_, byte)| **byte == b'\n')
                .all(|(index, _)| index > 0 && output.as_bytes()[index - 1] == b'\r')
        );
        assert_eq!(output.matches("\r\n").count(), output.lines().count() - 1);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_edit_preserves_unix_permission_bits() {
        use std::os::unix::fs::PermissionsExt;

        let temp = test_project(false);
        let path = entities_path(temp.path(), "plan");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).expect("set mode");

        apply_test_edit(
            temp.path(),
            EditOperation::Translate {
                entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                delta: [1.0, 0.0],
                duplicate: false,
            },
        );

        assert_eq!(
            fs::metadata(path).expect("metadata").permissions().mode() & 0o777,
            0o640
        );
    }

    #[test]
    fn rejects_stale_revision_and_locked_layer() {
        let temp = test_project(false);
        let request = DrawingEditRequest {
            drawing: "plan".to_owned(),
            expected_revision: "stale".to_owned(),
            operation: EditOperation::Delete {
                entity_id: "ent_01JZ0000000000000000000000".to_owned(),
            },
        };
        assert!(matches!(
            apply_edit(temp.path(), &request),
            Err(EditError::RevisionConflict)
        ));

        let locked = test_project(true);
        let project = cad_model::load_project(locked.path()).expect("project");
        let state = editor_state(&project, "plan").expect("state");
        let mut request = request;
        request.expected_revision = state.revision;
        assert!(matches!(
            apply_edit(locked.path(), &request),
            Err(EditError::LayerNotEditable(_))
        ));
    }

    #[test]
    fn concurrent_edits_from_one_revision_allow_only_one_publish() {
        use std::sync::{Arc, Barrier};

        let temp = test_project(false);
        let root = Arc::new(temp.path().to_path_buf());
        let project = cad_model::load_project(root.as_ref()).expect("project");
        let revision = editor_state(&project, "plan").expect("state").revision;
        let barrier = Arc::new(Barrier::new(3));
        let handles = [false, true].map(|duplicate| {
            let root = Arc::clone(&root);
            let revision = revision.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                apply_edit(
                    root.as_ref(),
                    &DrawingEditRequest {
                        drawing: "plan".to_owned(),
                        expected_revision: revision,
                        operation: EditOperation::Translate {
                            entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                            delta: [if duplicate { 2.0 } else { 1.0 }, 0.0],
                            duplicate,
                        },
                    },
                )
            })
        });
        barrier.wait();
        let results = handles.map(|handle| handle.join().expect("edit thread"));

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            results
                .iter()
                .filter(|result| matches!(result, Err(EditError::RevisionConflict)))
                .count(),
            1
        );
    }

    #[test]
    fn creates_entity_with_backend_ulid() {
        let temp = test_project(false);
        let project = cad_model::load_project(temp.path()).expect("project");
        let state = editor_state(&project, "plan").expect("state");
        let result = apply_edit(
            temp.path(),
            &DrawingEditRequest {
                drawing: "plan".to_owned(),
                expected_revision: state.revision,
                operation: EditOperation::Create {
                    entity: serde_json::json!({
                        "type": "point",
                        "layer": "0-1",
                        "at": [2.0, 3.0]
                    }),
                },
            },
        )
        .expect("create");
        assert!(
            result
                .entity_id
                .as_deref()
                .is_some_and(|id| id.starts_with("ent_"))
        );
        assert_eq!(
            cad_model::load_project(temp.path())
                .expect("reloaded")
                .drawings[0]
                .entities
                .len(),
            3
        );
    }

    #[test]
    fn updates_layout_as_a_history_transaction() {
        let temp = test_project(false);
        let result = apply_edit(
            temp.path(),
            &DrawingEditRequest {
                drawing: "plan".to_owned(),
                expected_revision: layout_revision(temp.path(), "plan").expect("layout revision"),
                operation: EditOperation::UpdateLayout {
                    layout: "default".to_owned(),
                    properties: serde_json::json!({"scale": "1/50"}),
                },
            },
        )
        .expect("layout update");
        assert_eq!(result.operation, "update_layout");
        let project = cad_model::load_project(temp.path()).expect("project should reload");
        assert_eq!(project.drawings[0].layouts.layouts["default"].scale, "1/50");
        assert!(temp.path().join("drawings/plan/layouts.toml").exists());
    }

    #[test]
    fn updates_block_definition_as_a_history_transaction() {
        let temp = test_project(false);
        fs::create_dir_all(temp.path().join("blocks/door")).expect("block directory");
        fs::write(
            temp.path().join("blocks/door/definition.toml"),
            "schema_version = \"0.2\"\nname = \"Door\"\nbase_point = [0.0, 0.0]\n",
        )
        .expect("definition");
        fs::write(temp.path().join("blocks/door/entities.ndjson"), "").expect("block entities");
        let result = apply_edit(
            temp.path(),
            &DrawingEditRequest {
                drawing: "plan".to_owned(),
                expected_revision: block_definition_revision(temp.path(), "door")
                    .expect("block revision"),
                operation: EditOperation::UpdateBlockDefinition {
                    block: "door".to_owned(),
                    properties: serde_json::json!({"name": "Door Updated"}),
                },
            },
        )
        .expect("block update");
        assert_eq!(result.operation, "update_block_definition");
        let project = cad_model::load_project(temp.path()).expect("project should reload");
        assert_eq!(project.blocks["door"].config.name, "Door Updated");
        let history = list_drawing_history(temp.path(), "plan").expect("history state");
        assert_eq!(history.undo.last().expect("block history").scope, "project");
    }

    #[test]
    fn layout_and_block_updates_reject_stale_target_revisions() {
        let temp = test_project(false);
        let stale_layout = layout_revision(temp.path(), "plan").expect("layout revision");
        fs::write(
            temp.path().join("drawings/plan/layouts.toml"),
            "schema_version = \"0.2\"\nactive_layout = \"default\"\n\n[layouts.default]\nname = \"default\"\npaper = \"A3\"\norientation = \"landscape\"\nscale = \"1/50\"\norigin = [0.0, 0.0]\nmargins = [0.0, 0.0, 0.0, 0.0]\n",
        )
        .expect("external layout update");
        let layout_result = apply_edit(
            temp.path(),
            &DrawingEditRequest {
                drawing: "plan".to_owned(),
                expected_revision: stale_layout,
                operation: EditOperation::UpdateLayout {
                    layout: "default".to_owned(),
                    properties: serde_json::json!({"scale": "1/25"}),
                },
            },
        );
        assert!(matches!(layout_result, Err(EditError::RevisionConflict)));

        fs::create_dir_all(temp.path().join("blocks/door")).expect("block directory");
        let definition = temp.path().join("blocks/door/definition.toml");
        fs::write(
            &definition,
            "schema_version = \"0.2\"\nname = \"Door\"\nbase_point = [0.0, 0.0]\n",
        )
        .expect("definition");
        fs::write(temp.path().join("blocks/door/entities.ndjson"), "").expect("entities");
        let stale_block = block_definition_revision(temp.path(), "door").expect("block revision");
        fs::write(
            &definition,
            "schema_version = \"0.2\"\nname = \"External\"\nbase_point = [0.0, 0.0]\n",
        )
        .expect("external block update");
        let block_result = apply_edit(
            temp.path(),
            &DrawingEditRequest {
                drawing: "plan".to_owned(),
                expected_revision: stale_block,
                operation: EditOperation::UpdateBlockDefinition {
                    block: "door".to_owned(),
                    properties: serde_json::json!({"name": "Local"}),
                },
            },
        );
        assert!(matches!(block_result, Err(EditError::RevisionConflict)));
    }

    #[test]
    fn snap_index_finds_endpoint_midpoint_and_intersection() {
        let temp = test_project(false);
        let project = cad_model::load_project(temp.path()).expect("project");
        let index = SnapIndex::build(&project, "plan").expect("index");
        assert_eq!(
            index
                .query([0.1, 0.1], 1.0, &[SnapKind::Endpoint])
                .map(|value| value.kind),
            Some(SnapKind::Endpoint)
        );
        assert_eq!(
            index
                .query([5.0, 0.1], 1.0, &[SnapKind::Midpoint])
                .map(|value| value.kind),
            Some(SnapKind::Midpoint)
        );
        assert_eq!(
            index
                .query([5.0, 0.0], 1.0, &[SnapKind::Intersection])
                .map(|value| value.kind),
            Some(SnapKind::Intersection)
        );
    }

    #[test]
    fn batch_applies_all_operations_and_rolls_back_on_failure() {
        let temp = test_project(false);
        let project = cad_model::load_project(temp.path()).expect("project");
        let state = editor_state(&project, "plan").expect("state");
        let result = apply_edit(
            temp.path(),
            &DrawingEditRequest {
                drawing: "plan".to_owned(),
                expected_revision: state.revision,
                operation: EditOperation::Batch {
                    operations: vec![EditOperation::TranslateMany {
                        entity_ids: vec![
                            "ent_01JZ0000000000000000000000".to_owned(),
                            "ent_01JZ0000000000000000000001".to_owned(),
                        ],
                        delta: [1.0, 2.0],
                        duplicate: false,
                    }],
                },
            },
        )
        .expect("batch");
        assert_eq!(result.operation, "batch");
        assert_eq!(result.entity_ids.len(), 2);

        let temp = test_project(false);
        let path = entities_path(temp.path(), "plan");
        let before = fs::read(&path).expect("before");
        let project = cad_model::load_project(temp.path()).expect("project");
        let revision = editor_state(&project, "plan").expect("state").revision;
        assert!(matches!(
            apply_edit(
                temp.path(),
                &DrawingEditRequest {
                    drawing: "plan".to_owned(),
                    expected_revision: revision,
                    operation: EditOperation::Batch {
                        operations: vec![
                            EditOperation::TranslateMany {
                                entity_ids: vec![
                                    "ent_01JZ0000000000000000000000".to_owned(),
                                    "ent_01JZ0000000000000000000001".to_owned(),
                                ],
                                delta: [1.0, 0.0],
                                duplicate: false,
                            },
                            EditOperation::Delete {
                                entity_id: "ent_01JZ0000000000000000000099".to_owned(),
                            },
                        ],
                    },
                },
            ),
            Err(EditError::EntityNotFound(_))
        ));
        assert_eq!(fs::read(path).expect("after"), before);
    }

    #[test]
    fn rotate_mirror_offset_trim_and_extend_update_line_geometry() {
        let temp = test_project(false);
        let result = apply_test_edit(
            temp.path(),
            EditOperation::Rotate {
                entity_ids: vec!["ent_01JZ0000000000000000000000".to_owned()],
                center: [0.0, 0.0],
                angle_deg: 90.0,
            },
        );
        assert_eq!(result.operation, "rotate");
        let entity = &cad_model::load_project(temp.path())
            .expect("project")
            .drawings[0]
            .entities[0]
            .entity;
        assert!((entity_point(entity, "p2")[0]).abs() < 1e-9);
        assert!((entity_point(entity, "p2")[1] - 10.0).abs() < 1e-9);

        let temp = test_project(false);
        apply_test_edit(
            temp.path(),
            EditOperation::Mirror {
                entity_ids: vec!["ent_01JZ0000000000000000000000".to_owned()],
                axis_start: [0.0, 0.0],
                axis_end: [0.0, 1.0],
            },
        );
        let entity = &cad_model::load_project(temp.path())
            .expect("project")
            .drawings[0]
            .entities[0]
            .entity;
        assert!((entity_point(entity, "p2")[0] + 10.0).abs() < 1e-9);

        let temp = test_project(false);
        apply_test_edit(
            temp.path(),
            EditOperation::Offset {
                entity_ids: vec!["ent_01JZ0000000000000000000000".to_owned()],
                distance: 2.0,
            },
        );
        let entity = &cad_model::load_project(temp.path())
            .expect("project")
            .drawings[0]
            .entities[0]
            .entity;
        assert!((entity_point(entity, "p1")[1] - 2.0).abs() < 1e-9);

        let temp = test_project(false);
        apply_test_edit(
            temp.path(),
            EditOperation::Trim {
                target_entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                cutter_entity_id: "ent_01JZ0000000000000000000001".to_owned(),
                pick_point: [8.0, 0.0],
            },
        );
        let entity = &cad_model::load_project(temp.path())
            .expect("project")
            .drawings[0]
            .entities[0]
            .entity;
        assert!((entity_point(entity, "p2")[0] - 5.0).abs() < 1e-9);

        let temp = test_project(false);
        fs::write(
            entities_path(temp.path(), "plan"),
            concat!(
                "{\"schema_version\":\"0.2\",\"id\":\"ent_01JZ0000000000000000000000\",\"type\":\"line\",\"layer\":\"0-1\",\"p1\":[0.0,0.0],\"p2\":[2.0,0.0]}\n",
                "{\"schema_version\":\"0.2\",\"id\":\"ent_01JZ0000000000000000000001\",\"type\":\"line\",\"layer\":\"0-1\",\"p1\":[5.0,-5.0],\"p2\":[5.0,5.0]}\n",
            ),
        )
        .expect("entities");
        apply_test_edit(
            temp.path(),
            EditOperation::Extend {
                target_entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                boundary_entity_id: "ent_01JZ0000000000000000000001".to_owned(),
                pick_point: [0.0, 0.0],
            },
        );
        let entity = &cad_model::load_project(temp.path())
            .expect("project")
            .drawings[0]
            .entities[0]
            .entity;
        assert!((entity_point(entity, "p1")[0] - 5.0).abs() < 1e-9);
    }

    #[test]
    fn rejects_duplicate_selection_ids_without_publishing() {
        let temp = test_project(false);
        let path = entities_path(temp.path(), "plan");
        let before = fs::read(&path).expect("before");
        let project = cad_model::load_project(temp.path()).expect("project");
        let revision = editor_state(&project, "plan").expect("state").revision;
        assert!(matches!(
            apply_edit(
                temp.path(),
                &DrawingEditRequest {
                    drawing: "plan".to_owned(),
                    expected_revision: revision,
                    operation: EditOperation::DeleteMany {
                        entity_ids: vec![
                            "ent_01JZ0000000000000000000000".to_owned(),
                            "ent_01JZ0000000000000000000000".to_owned(),
                        ],
                    },
                },
            ),
            Err(EditError::InvalidEntity(_))
        ));
        assert_eq!(fs::read(path).expect("after"), before);
    }

    #[test]
    fn rotates_and_mirrors_every_supported_entity_family() {
        let sources = [
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0,0],"p2":[1,0]}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000001","type":"polyline","layer":"0-1","points":[[0,0],[1,0],[1,1]],"closed":false}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000002","type":"arc","layer":"0-1","center":[0,0],"radius":1,"start_deg":0,"end_deg":90}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000003","type":"circle","layer":"0-1","center":[0,0],"radius":1}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000004","type":"ellipse","layer":"0-1","center":[0,0],"radius_x":2,"radius_y":1,"rotation_deg":0,"start_deg":0,"end_deg":180}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000005","type":"text","layer":"0-1","style":"note","at":[0,0],"rotation_deg":0,"mirror_y":false,"value":"A"}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000006","type":"dimension","layer":"0-1","style":"dim","p1":[0,0],"p2":[1,0],"offset":1,"text_rotation_deg":0,"text_mirror_y":false,"value":null}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000007","type":"point","layer":"0-1","at":[0,0],"temporary":false,"marker_code":null,"rotation_deg":0,"scale":1}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000008","type":"solid","layer":"0-1","points":[[0,0],[1,0],[0,1]],"fill":"black"}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000009","type":"curve_solid","layer":"0-1","center":[0,0],"radius":1,"flatness":0.1,"rotation_deg":0,"start_deg":0,"end_deg":90,"solid_param":1,"encoding_code":1,"fill":"black"}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ000000000000000000000A","type":"block_ref","layer":"0-1","block":"B","at":[0,0],"rotation_deg":0,"scale":1}"#,
        ];
        for source in sources {
            let entity: Entity = serde_json::from_str(source).expect("entity");
            let rotated = rotate_entity(&entity, [0.0, 0.0], 90.0).expect("rotate");
            let mirrored = mirror_entity(&entity, [0.0, 0.0], [0.0, 1.0]).expect("mirror");
            validate_entity_geometry(&rotated).expect("rotated geometry");
            validate_entity_geometry(&mirrored).expect("mirrored geometry");
            assert_eq!(rotated.id().as_str(), entity.id().as_str());
            assert_eq!(mirrored.id().as_str(), entity.id().as_str());
        }
    }

    #[test]
    fn offsets_polyline_segments_at_their_join() {
        let result =
            offset_polyline(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]], false, 1.0).expect("offset");
        assert_eq!(result[0], [0.0, 1.0]);
        assert_eq!(result[1], [9.0, 1.0]);
        assert_eq!(result[2], [9.0, 10.0]);
    }

    #[test]
    fn trims_and_extends_open_polyline_paths() {
        let temp = test_project(false);
        fs::write(
            entities_path(temp.path(), "plan"),
            concat!(
                "{\"schema_version\":\"0.2\",\"id\":\"ent_01JZ0000000000000000000000\",\"type\":\"polyline\",\"layer\":\"0-1\",\"points\":[[0.0,0.0],[5.0,0.0],[5.0,5.0]],\"closed\":false}\n",
                "{\"schema_version\":\"0.2\",\"id\":\"ent_01JZ0000000000000000000001\",\"type\":\"line\",\"layer\":\"0-1\",\"p1\":[3.0,-5.0],\"p2\":[3.0,5.0]}\n",
            ),
        )
        .expect("entities");
        apply_test_edit(
            temp.path(),
            EditOperation::Trim {
                target_entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                cutter_entity_id: "ent_01JZ0000000000000000000001".to_owned(),
                pick_point: [5.0, 4.0],
            },
        );
        let project = cad_model::load_project(temp.path()).expect("project");
        let entity = &project.drawings[0].entities[0].entity;
        let value = serde_json::to_value(entity).expect("polyline json");
        assert_eq!(value["points"], serde_json::json!([[0.0, 0.0], [3.0, 0.0]]));

        let temp = test_project(false);
        fs::write(
            entities_path(temp.path(), "plan"),
            concat!(
                "{\"schema_version\":\"0.2\",\"id\":\"ent_01JZ0000000000000000000000\",\"type\":\"polyline\",\"layer\":\"0-1\",\"points\":[[0.0,0.0],[2.0,0.0],[2.0,2.0]],\"closed\":false}\n",
                "{\"schema_version\":\"0.2\",\"id\":\"ent_01JZ0000000000000000000001\",\"type\":\"line\",\"layer\":\"0-1\",\"p1\":[0.0,5.0],\"p2\":[5.0,5.0]}\n",
            ),
        )
        .expect("entities");
        apply_test_edit(
            temp.path(),
            EditOperation::Extend {
                target_entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                boundary_entity_id: "ent_01JZ0000000000000000000001".to_owned(),
                pick_point: [2.0, 2.0],
            },
        );
        let project = cad_model::load_project(temp.path()).expect("project");
        let entity = &project.drawings[0].entities[0].entity;
        let value = serde_json::to_value(entity).expect("polyline json");
        assert_eq!(
            value["points"],
            serde_json::json!([[0.0, 0.0], [2.0, 0.0], [2.0, 5.0]])
        );
    }

    #[test]
    fn records_undo_redo_snapshots_and_survives_reload() {
        let temp = test_project(false);
        let path = entities_path(temp.path(), "plan");
        let before = fs::read(&path).expect("before");
        let result = apply_test_edit(
            temp.path(),
            EditOperation::Translate {
                entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                delta: [2.0, 0.0],
                duplicate: false,
            },
        );
        let history_id = result.history_id.clone().expect("history id");
        let current = fs::read(&path).expect("current");
        let state = list_drawing_history(temp.path(), "plan").expect("history state");
        assert_eq!(state.undo.len(), 1);
        assert!(state.redo.is_empty());
        assert_eq!(state.undo[0].history_id, history_id);
        assert!(
            temp.path()
                .join("build/.cad-history/entries")
                .join(&history_id)
                .join("before/drawings/plan/entities.ndjson")
                .exists()
        );

        let undone = undo_drawing_edit(
            temp.path(),
            &DrawingHistoryRequest {
                drawing: "plan".to_owned(),
                expected_files: current_history_files(temp.path(), "plan").expect("files"),
            },
        )
        .expect("undo");
        assert_eq!(undone.operation, "undo");
        assert_eq!(fs::read(&path).expect("undone bytes"), before);
        let redone = redo_drawing_edit(
            temp.path(),
            &DrawingHistoryRequest {
                drawing: "plan".to_owned(),
                expected_files: current_history_files(temp.path(), "plan").expect("files"),
            },
        )
        .expect("redo");
        assert_eq!(redone.operation, "redo");
        assert_eq!(fs::read(&path).expect("redone bytes"), current);
        assert_eq!(
            list_drawing_history(temp.path(), "plan")
                .expect("reloaded history")
                .undo
                .len(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn history_restore_preserves_permission_bits() {
        use std::os::unix::fs::PermissionsExt;

        let temp = test_project(false);
        let path = entities_path(temp.path(), "plan");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).expect("mode");
        let result = apply_test_edit(
            temp.path(),
            EditOperation::Translate {
                entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                delta: [1.0, 0.0],
                duplicate: false,
            },
        );
        undo_drawing_edit(
            temp.path(),
            &DrawingHistoryRequest {
                drawing: "plan".to_owned(),
                expected_files: current_history_files(temp.path(), "plan").expect("files"),
            },
        )
        .expect("undo");
        assert_eq!(
            fs::metadata(&path).expect("metadata").permissions().mode() & 0o777,
            0o640
        );
        assert!(result.history_id.is_some());
    }

    #[test]
    fn stale_or_corrupt_history_never_changes_the_source() {
        let temp = test_project(false);
        let path = entities_path(temp.path(), "plan");
        apply_test_edit(
            temp.path(),
            EditOperation::Translate {
                entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                delta: [1.0, 0.0],
                duplicate: false,
            },
        );
        let current = fs::read(&path).expect("current");
        assert!(matches!(
            undo_drawing_edit(
                temp.path(),
                &DrawingHistoryRequest {
                    drawing: "plan".to_owned(),
                    expected_files: vec![HistoryFileRevision {
                        relative_path: "drawings/plan/entities.ndjson".to_owned(),
                        revision: "stale".to_owned(),
                        exists: true,
                    }],
                },
            ),
            Err(EditError::RevisionConflict)
        ));
        assert_eq!(fs::read(&path).expect("unchanged"), current);

        let state = list_drawing_history(temp.path(), "plan").expect("state");
        let metadata_path = temp
            .path()
            .join("build/.cad-history/entries")
            .join(&state.undo[0].history_id)
            .join("metadata.json");
        fs::write(&metadata_path, b"not-json").expect("corrupt metadata");
        assert!(matches!(
            undo_drawing_edit(
                temp.path(),
                &DrawingHistoryRequest {
                    drawing: "plan".to_owned(),
                    expected_files: current_history_files(temp.path(), "plan").expect("files"),
                },
            ),
            Err(EditError::HistoryCorrupt(_))
        ));
        assert_eq!(fs::read(&path).expect("still unchanged"), current);
    }

    #[test]
    fn history_publish_rejects_external_deletion_of_an_empty_file() {
        let temp = test_project(false);
        let path = temp.path().join("comments/plan.ndjson");
        fs::create_dir_all(path.parent().expect("comments parent")).expect("comments dir");
        fs::write(&path, b"").expect("empty comments file");
        let original = HistoryFileInput {
            relative_path: "comments/plan.ndjson".to_owned(),
            exists: true,
            bytes: Vec::new(),
            permissions: None,
        };
        let target = HistoryFileInput {
            relative_path: original.relative_path.clone(),
            exists: true,
            bytes: b"new\n".to_vec(),
            permissions: None,
        };
        fs::remove_file(&path).expect("external deletion");

        assert!(matches!(
            publish_history_file(temp.path(), &target, &original),
            Err(EditError::RevisionConflict)
        ));
        assert!(!path.exists());
    }

    #[test]
    fn history_rollback_does_not_overwrite_an_external_change() {
        let temp = test_project(false);
        let path = temp.path().join("comments/plan.ndjson");
        fs::create_dir_all(path.parent().expect("comments parent")).expect("comments dir");
        let original = HistoryFileInput {
            relative_path: "comments/plan.ndjson".to_owned(),
            exists: true,
            bytes: b"original\n".to_vec(),
            permissions: None,
        };
        let published = HistoryFileInput {
            relative_path: original.relative_path.clone(),
            exists: true,
            bytes: b"published\n".to_vec(),
            permissions: None,
        };
        fs::write(&path, b"external\n").expect("external edit");

        restore_history_file_if_unchanged(temp.path(), &original, &published)
            .expect("rollback check");
        assert_eq!(fs::read(&path).expect("external bytes"), b"external\n");
    }

    #[test]
    fn new_edit_clears_redo_and_clear_history_removes_entries() {
        let temp = test_project(false);
        apply_test_edit(
            temp.path(),
            EditOperation::Translate {
                entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                delta: [1.0, 0.0],
                duplicate: false,
            },
        );
        undo_drawing_edit(
            temp.path(),
            &DrawingHistoryRequest {
                drawing: "plan".to_owned(),
                expected_files: current_history_files(temp.path(), "plan").expect("files"),
            },
        )
        .expect("undo");
        apply_test_edit(
            temp.path(),
            EditOperation::Translate {
                entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                delta: [3.0, 0.0],
                duplicate: false,
            },
        );
        let state = list_drawing_history(temp.path(), "plan").expect("state");
        assert!(state.redo.is_empty());
        clear_drawing_history(temp.path(), "plan").expect("clear");
        let cleared = list_drawing_history(temp.path(), "plan").expect("cleared state");
        assert!(cleared.undo.is_empty());
        assert!(cleared.redo.is_empty());
    }

    #[test]
    fn clear_history_uses_the_shared_history_lock() {
        let temp = test_project(false);
        apply_test_edit(
            temp.path(),
            EditOperation::Translate {
                entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                delta: [1.0, 0.0],
                duplicate: false,
            },
        );
        let (locked_tx, locked_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            with_history_lock(|| {
                locked_tx.send(()).expect("lock signal");
                release_rx.recv().expect("release signal");
                Ok(())
            })
        });
        locked_rx.recv().expect("shared lock should be held");

        let project_path = temp.path().to_path_buf();
        let (attempted_tx, attempted_rx) = std::sync::mpsc::channel();
        let (finished_tx, finished_rx) = std::sync::mpsc::channel();
        let clearer = std::thread::spawn(move || {
            attempted_tx.send(()).expect("attempt signal");
            let result = clear_drawing_history(&project_path, "plan");
            finished_tx.send(()).expect("finish signal");
            result
        });
        attempted_rx.recv().expect("clear should be attempted");
        assert!(matches!(
            finished_rx.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));

        release_tx.send(()).expect("release shared lock");
        holder
            .join()
            .expect("holder thread")
            .expect("holder result");
        clearer.join().expect("clear thread").expect("clear result");
        finished_rx
            .recv()
            .expect("clear should finish after release");
    }

    #[test]
    fn clearing_a_drawing_keeps_project_scoped_history() {
        let temp = test_project(false);
        let other = temp.path().join("drawings/other");
        fs::create_dir_all(&other).expect("other drawing");
        fs::copy(
            temp.path().join("drawings/plan/layouts.toml"),
            other.join("layouts.toml"),
        )
        .expect("other layouts");
        fs::copy(
            temp.path().join("drawings/plan/entities.ndjson"),
            other.join("entities.ndjson"),
        )
        .expect("other entities");
        let before = read_history_file(temp.path(), "rules/layers.toml").expect("layers file");
        let mut stage = stage_history_transaction(
            temp.path(),
            "plan",
            "project",
            "layer.update",
            &[],
            std::slice::from_ref(&before),
        )
        .expect("stage project history");
        stage
            .prepare_after(std::slice::from_ref(&before))
            .expect("prepare project history");
        commit_history_stage(&mut stage).expect("commit project history");

        apply_test_edit(
            temp.path(),
            EditOperation::Translate {
                entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                delta: [1.0, 0.0],
                duplicate: false,
            },
        );
        clear_drawing_history(temp.path(), "plan").expect("clear drawing history");

        let state = list_drawing_history(temp.path(), "plan").expect("history state");
        assert_eq!(state.undo.len(), 1);
        assert_eq!(state.undo[0].scope, "project");
        let other_state = list_drawing_history(temp.path(), "other").expect("other history state");
        assert_eq!(other_state.undo.len(), 1);
        assert_eq!(other_state.undo[0].scope, "project");
    }

    #[cfg(unix)]
    #[test]
    fn history_rejects_a_symlinked_build_directory() {
        use std::os::unix::fs::symlink;

        let temp = test_project(false);
        let outside = tempfile::tempdir().expect("outside dir");
        symlink(outside.path(), temp.path().join("build")).expect("build symlink");

        let result = apply_edit(
            temp.path(),
            &DrawingEditRequest {
                drawing: "plan".to_owned(),
                expected_revision: revision(
                    &fs::read(entities_path(temp.path(), "plan")).expect("entities"),
                ),
                operation: EditOperation::Translate {
                    entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                    delta: [1.0, 0.0],
                    duplicate: false,
                },
            },
        );
        assert!(matches!(result, Err(EditError::HistoryUnavailable(_))));
        assert!(!outside.path().join(".cad-history").exists());
    }

    #[cfg(unix)]
    #[test]
    fn history_rejects_a_symlinked_entries_directory() {
        use std::os::unix::fs::symlink;

        let temp = test_project(false);
        let outside = tempfile::tempdir().expect("outside dir");
        fs::create_dir_all(temp.path().join("build/.cad-history")).expect("history root");
        symlink(
            outside.path(),
            temp.path().join("build/.cad-history/entries"),
        )
        .expect("entries symlink");

        let result = apply_edit(
            temp.path(),
            &DrawingEditRequest {
                drawing: "plan".to_owned(),
                expected_revision: revision(
                    &fs::read(entities_path(temp.path(), "plan")).expect("entities"),
                ),
                operation: EditOperation::Translate {
                    entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                    delta: [1.0, 0.0],
                    duplicate: false,
                },
            },
        );
        assert!(matches!(result, Err(EditError::HistoryUnavailable(_))));
        assert!(
            fs::read_dir(outside.path())
                .expect("outside")
                .next()
                .is_none()
        );
    }

    #[cfg(unix)]
    #[test]
    fn history_apis_reject_symlinked_canonical_sources_without_touching_the_target() {
        use std::os::unix::fs::symlink;

        let temp = test_project(false);
        apply_test_edit(
            temp.path(),
            EditOperation::Translate {
                entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                delta: [1.0, 0.0],
                duplicate: false,
            },
        );
        let entities = entities_path(temp.path(), "plan");
        let external = tempfile::NamedTempFile::new().expect("external target");
        fs::write(external.path(), b"external must survive\n").expect("external bytes");
        fs::remove_file(&entities).expect("remove canonical source");
        symlink(external.path(), &entities).expect("source symlink");

        assert!(matches!(
            list_drawing_history(temp.path(), "plan"),
            Err(EditError::InvalidEntity(_))
        ));
        assert!(matches!(
            undo_drawing_edit(
                temp.path(),
                &DrawingHistoryRequest {
                    drawing: "plan".to_owned(),
                    expected_files: Vec::new(),
                },
            ),
            Err(EditError::InvalidEntity(_))
        ));
        assert_eq!(
            fs::read(external.path()).expect("external bytes"),
            b"external must survive\n"
        );
    }

    #[test]
    fn history_prunes_to_the_latest_one_hundred_generations() {
        let temp = test_project(false);
        for _ in 0..105 {
            apply_test_edit(
                temp.path(),
                EditOperation::Translate {
                    entity_id: "ent_01JZ0000000000000000000000".to_owned(),
                    delta: [1.0, 0.0],
                    duplicate: false,
                },
            );
        }
        let state = list_drawing_history(temp.path(), "plan").expect("history state");
        assert_eq!(state.undo.len(), 100);
        assert!(state.redo.is_empty());
    }

    #[test]
    fn manifest_history_restores_multiple_files_as_one_transaction() {
        let temp = test_project(false);
        let comment_path = temp.path().join("comments/plan.ndjson");
        fs::create_dir_all(comment_path.parent().expect("comments parent")).expect("comments dir");
        fs::write(&comment_path, b"before\r\n").expect("comments");
        let before_entities =
            read_history_file(temp.path(), "drawings/plan/entities.ndjson").expect("entity input");
        let before_comments =
            read_history_file(temp.path(), "comments/plan.ndjson").expect("comment input");
        let mut stage = stage_history_transaction(
            temp.path(),
            "plan",
            "drawing",
            "comment.create",
            &[],
            &[before_entities.clone(), before_comments.clone()],
        )
        .expect("history stage");
        let after_entities = before_entities.clone();
        let after_comments = HistoryFileInput {
            relative_path: "comments/plan.ndjson".to_owned(),
            exists: true,
            bytes: b"after\r\n".to_vec(),
            permissions: before_comments.permissions.clone(),
        };
        stage
            .prepare_after(&[after_entities, after_comments.clone()])
            .expect("after snapshots");
        fs::write(&comment_path, &after_comments.bytes).expect("publish comment");
        let history_id = commit_history_stage(&mut stage).expect("commit history");
        let current = vec![
            HistoryFileRevision {
                relative_path: before_entities.relative_path,
                revision: revision(&before_entities.bytes),
                exists: true,
            },
            HistoryFileRevision {
                relative_path: after_comments.relative_path,
                revision: revision(&after_comments.bytes),
                exists: true,
            },
        ];
        let result = undo_drawing_edit(
            temp.path(),
            &DrawingHistoryRequest {
                drawing: "plan".to_owned(),
                expected_files: current,
            },
        )
        .expect("undo manifest");
        assert_eq!(result.history_id, Some(history_id));
        assert_eq!(
            fs::read(comment_path).expect("restored comment"),
            b"before\r\n"
        );
    }

    fn entity_point(entity: &Entity, key: &str) -> Point {
        let value = serde_json::to_value(entity).expect("entity json");
        value[key]
            .as_array()
            .expect("point")
            .iter()
            .map(|value| value.as_f64().expect("number"))
            .collect::<Vec<_>>()
            .try_into()
            .expect("two coordinates")
    }

    fn apply_test_edit(project_path: &Path, operation: EditOperation) -> DrawingEditResult {
        let project = cad_model::load_project(project_path).expect("project");
        let state = editor_state(&project, "plan").expect("editor state");
        let expected_revision = match &operation {
            EditOperation::UpdateLayout { .. } => {
                layout_revision(project_path, "plan").expect("layout revision")
            }
            EditOperation::UpdateBlockDefinition { block, .. } => {
                block_definition_revision(project_path, block).expect("block revision")
            }
            _ => state.revision,
        };
        apply_edit(
            project_path,
            &DrawingEditRequest {
                drawing: "plan".to_owned(),
                expected_revision,
                operation,
            },
        )
        .expect("edit")
    }

    fn test_project(locked: bool) -> tempfile::TempDir {
        let temp = tempfile::tempdir().expect("tempdir");
        fs::create_dir_all(temp.path().join("rules")).expect("rules");
        fs::create_dir_all(temp.path().join("drawings/plan")).expect("drawing");
        fs::write(
            temp.path().join("cad.project.toml"),
            "schema_version = \"0.2\"\nname = \"edit-test\"\n",
        )
        .expect("project");
        fs::write(
            temp.path().join("rules/layers.toml"),
            format!("active_layer = \"0-1\"\n\n[layers.\"0-1\"]\nname = \"Edit\"\nvisible = true\nlocked = {locked}\nprintable = true\ncolor = \"black\"\nline_type = \"solid\"\nline_width = 0.25\n"),
        )
        .expect("layers");
        fs::write(
            temp.path().join("rules/styles.toml"),
            "[colors.black]\nrgb = \"#000000\"\nprint_width = 0.25\n\n[line_types.solid]\ndash = []\n\n[pens]\n\n[text_styles]\n\n[dimension_styles]\n",
        )
        .expect("styles");
        fs::write(
            temp.path().join("drawings/plan/layouts.toml"),
            "schema_version = \"0.2\"\nactive_layout = \"default\"\n\n[layouts.default]\nname = \"default\"\npaper = \"A3\"\norientation = \"landscape\"\nscale = \"1/1\"\norigin = [0.0, 0.0]\nmargins = [0.0, 0.0, 0.0, 0.0]\n",
        )
        .expect("layouts");
        fs::write(
            entities_path(temp.path(), "plan"),
            concat!(
                "{\"schema_version\":\"0.2\",\"id\":\"ent_01JZ0000000000000000000000\",\"type\":\"line\",\"layer\":\"0-1\",\"p1\":[0.0,0.0],\"p2\":[10.0,0.0]}\n",
                "{ \"schema_version\": \"0.2\", \"id\": \"ent_01JZ0000000000000000000001\",  \"type\": \"line\", \"layer\": \"0-1\", \"p1\": [5.0,-5.0], \"p2\": [5.0,5.0] }\n"
            ),
        )
        .expect("entities");
        temp
    }
}
