//! Core CAD source model crate.
//!
//! This crate owns the typed representation of the NDJSON/TOML source files.
//! Phase 1 reads the local project layout into strict Rust types without
//! checker, renderer, or diff behavior.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use thiserror::Error;
use ulid::Ulid;

pub const CRATE_NAME: &str = "cad-model";
pub const CURRENT_SCHEMA_VERSION: &str = "0.2";

#[must_use]
pub fn crate_name() -> &'static str {
    CRATE_NAME
}

#[derive(Debug, Error)]
pub enum ModelError {
    #[error("failed to read {path}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to list {path}")]
    ListDir {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse TOML {path}")]
    Toml {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("failed to parse NDJSON {path}:{line}")]
    Ndjson {
        path: PathBuf,
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("empty NDJSON line at {path}:{line}")]
    EmptyNdjsonLine { path: PathBuf, line: usize },
    #[error("unsupported schema_version {found:?} in {path}; expected {expected:?}")]
    UnsupportedSchema {
        path: PathBuf,
        line: Option<usize>,
        found: String,
        expected: &'static str,
    },
    #[error("invalid entity id {value:?}; expected ent_<26-character ULID>")]
    InvalidEntityId { value: String },
    #[error("unsafe canonical source path {path}: {reason}")]
    UnsafeSourcePath { path: PathBuf, reason: String },
    #[error("invalid JWW preservation state at {path}: {reason}")]
    JwwPreservation { path: PathBuf, reason: String },
}

pub type ModelResult<T> = Result<T, ModelError>;
pub type Point = [f64; 2];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectSourceKind {
    Project,
    Rule,
    Layout,
    DrawingEntities,
    Comment,
    BlockDefinition,
    BlockEntities,
    JwwOriginal,
    JwwPreservation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFileRevision {
    pub relative_path: String,
    pub revision: String,
    pub exists: bool,
}

pub const JWW_ORIGINAL_RELATIVE_PATH: &str = "interop/jww/original.jww";
pub const JWW_PRESERVATION_RELATIVE_PATH: &str = "interop/jww/preservation.toml";
pub const JWW_RECORDS_RELATIVE_PATH: &str = "interop/jww/records.ndjson";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JwwEditCapability {
    #[default]
    ExactOnly,
    MappedV600,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JwwCompatibilityState {
    EditableLossless,
    PreservedReadOnly,
    UnsupportedVersion,
    Malformed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JwwPreservationManifest {
    pub schema_version: String,
    pub state: JwwCompatibilityState,
    pub jww_version: Option<u32>,
    pub drawing_name: String,
    pub original_relative_path: String,
    pub original_blake3: String,
    pub original_sha256: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub source_revisions: Vec<SourceFileRevision>,
    #[serde(default)]
    pub edit_capability: JwwEditCapability,
    #[serde(default)]
    pub records_relative_path: Option<String>,
    #[serde(default)]
    pub records_blake3: Option<String>,
    #[serde(default)]
    pub records_sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JwwProjectCompatibility {
    pub state: JwwCompatibilityState,
    pub reason: Option<String>,
    pub original_verified: bool,
    pub changed_source_paths: Vec<String>,
    pub edit_capability: JwwEditCapability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedJwwPreservationSnapshot {
    pub manifest: JwwPreservationManifest,
    pub original_bytes: Vec<u8>,
    pub record_provenance_bytes: Option<Vec<u8>>,
    pub source_manifest: Vec<SourceFileRevision>,
}

#[must_use]
pub fn classify_project_source_path(relative: &Path) -> Option<ProjectSourceKind> {
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return None;
    }
    if relative == Path::new("cad.project.toml") {
        return Some(ProjectSourceKind::Project);
    }
    if relative == Path::new(JWW_ORIGINAL_RELATIVE_PATH) {
        return Some(ProjectSourceKind::JwwOriginal);
    }
    if relative == Path::new(JWW_PRESERVATION_RELATIVE_PATH) {
        return Some(ProjectSourceKind::JwwPreservation);
    }
    if relative == Path::new(JWW_RECORDS_RELATIVE_PATH) {
        return Some(ProjectSourceKind::JwwPreservation);
    }
    let components = relative.components().collect::<Vec<_>>();
    let component = |index: usize| components.get(index)?.as_os_str().to_str();
    match (
        component(0),
        components.len(),
        relative.file_name()?.to_str()?,
    ) {
        (Some("rules"), 2, name) if name.ends_with(".toml") => Some(ProjectSourceKind::Rule),
        (Some("drawings"), 3, "layouts.toml") => Some(ProjectSourceKind::Layout),
        (Some("drawings"), 3, "entities.ndjson") => Some(ProjectSourceKind::DrawingEntities),
        (Some("comments"), 2, name) if name.ends_with(".ndjson") => {
            Some(ProjectSourceKind::Comment)
        }
        (Some("blocks"), 3, "definition.toml") => Some(ProjectSourceKind::BlockDefinition),
        (Some("blocks"), 3, "entities.ndjson") => Some(ProjectSourceKind::BlockEntities),
        _ => None,
    }
}

pub fn source_manifest(root: impl AsRef<Path>) -> ModelResult<Vec<SourceFileRevision>> {
    fn visit(
        root: &Path,
        directory: &Path,
        output: &mut Vec<SourceFileRevision>,
    ) -> ModelResult<()> {
        let mut entries = fs::read_dir(directory)
            .map_err(|source| ModelError::ListDir {
                path: directory.to_path_buf(),
                source,
            })?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|source| ModelError::ListDir {
                path: directory.to_path_buf(),
                source,
            })?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let relative = path.strip_prefix(root).expect("visited path is below root");
            if relative
                .components()
                .next()
                .is_some_and(|part| part.as_os_str() == "build")
            {
                continue;
            }
            if relative.components().count() == 1
                && !matches!(
                    relative.file_name().and_then(|name| name.to_str()),
                    Some(
                        "rules"
                            | "drawings"
                            | "comments"
                            | "blocks"
                            | "interop"
                            | "cad.project.toml"
                    )
                )
            {
                continue;
            }
            let file_type = entry.file_type().map_err(|source| ModelError::Read {
                path: path.clone(),
                source,
            })?;
            if file_type.is_symlink() {
                let parts = relative.components().collect::<Vec<_>>();
                let may_hide_source_directory = (parts.len() == 1
                    && matches!(
                        parts.first().and_then(|part| part.as_os_str().to_str()),
                        Some("rules" | "drawings" | "comments" | "blocks" | "interop")
                    ))
                    || (parts.len() == 2
                        && matches!(
                            parts.first().and_then(|part| part.as_os_str().to_str()),
                            Some("drawings" | "blocks" | "interop")
                        ));
                if classify_project_source_path(relative).is_some() || may_hide_source_directory {
                    return Err(ModelError::UnsafeSourcePath {
                        path,
                        reason: "symlinks are not allowed below canonical source roots".to_owned(),
                    });
                }
                continue;
            }
            if file_type.is_dir() {
                if relative.components().count() == 1
                    && !matches!(
                        relative.file_name().and_then(|name| name.to_str()),
                        Some("rules" | "drawings" | "comments" | "blocks" | "interop")
                    )
                {
                    continue;
                }
                visit(root, &path, output)?;
            } else if file_type.is_file() && classify_project_source_path(relative).is_some() {
                let bytes = fs::read(&path).map_err(|source| ModelError::Read {
                    path: path.clone(),
                    source,
                })?;
                output.push(SourceFileRevision {
                    relative_path: relative.to_string_lossy().replace('\\', "/"),
                    revision: blake3::hash(&bytes).to_hex().to_string(),
                    exists: true,
                });
            }
        }
        Ok(())
    }

    let root = root.as_ref();
    let mut output = Vec::new();
    visit(root, root, &mut output)?;
    output.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(output)
}

pub fn jww_relevant_source_manifest(
    root: impl AsRef<Path>,
) -> ModelResult<Vec<SourceFileRevision>> {
    source_manifest(root).map(|files| {
        files
            .into_iter()
            .filter(|file| {
                !matches!(
                    classify_project_source_path(Path::new(&file.relative_path)),
                    Some(ProjectSourceKind::Comment | ProjectSourceKind::JwwPreservation)
                )
            })
            .collect()
    })
}

pub fn load_jww_preservation_manifest(
    root: impl AsRef<Path>,
) -> ModelResult<Option<JwwPreservationManifest>> {
    let root = root.as_ref();
    let path = root.join(JWW_PRESERVATION_RELATIVE_PATH);
    if !path.exists() {
        return Ok(None);
    }
    let text = read_canonical_source(root, &path)?;
    toml::from_str(&text)
        .map(Some)
        .map_err(|source| ModelError::Toml { path, source })
}

fn validate_jww_preservation_manifest(
    root: &Path,
    manifest: &JwwPreservationManifest,
) -> ModelResult<()> {
    let manifest_path = root.join(JWW_PRESERVATION_RELATIVE_PATH);
    if !matches!(manifest.schema_version.as_str(), "0.1" | "0.2") {
        return Err(ModelError::JwwPreservation {
            path: manifest_path,
            reason: format!(
                "unsupported preservation schema {:?}",
                manifest.schema_version
            ),
        });
    }
    if manifest.original_relative_path != JWW_ORIGINAL_RELATIVE_PATH {
        return Err(ModelError::JwwPreservation {
            path: manifest_path,
            reason: "original_relative_path is not canonical".to_owned(),
        });
    }
    Ok(())
}

fn jww_bytes_match(bytes: &[u8], expected_blake3: &str, expected_sha256: &str) -> bool {
    blake3::hash(bytes).to_hex().as_str() == expected_blake3
        && format!("{:x}", Sha256::digest(bytes)) == expected_sha256
}

fn read_jww_record_provenance_bytes(
    root: &Path,
    manifest: &JwwPreservationManifest,
) -> ModelResult<Option<Vec<u8>>> {
    if !jww_record_metadata_is_canonical(manifest) {
        return Err(ModelError::JwwPreservation {
            path: root.join(JWW_PRESERVATION_RELATIVE_PATH),
            reason: "preservation schema and edit capability metadata are inconsistent".to_owned(),
        });
    }
    match manifest.edit_capability {
        JwwEditCapability::ExactOnly => Ok(None),
        JwwEditCapability::MappedV600 => {
            read_canonical_source_bytes(root, &root.join(JWW_RECORDS_RELATIVE_PATH)).map(Some)
        }
    }
}

fn jww_record_metadata_is_canonical(manifest: &JwwPreservationManifest) -> bool {
    match manifest.edit_capability {
        JwwEditCapability::ExactOnly => {
            manifest.schema_version == "0.1"
                && manifest.records_relative_path.is_none()
                && manifest.records_blake3.is_none()
                && manifest.records_sha256.is_none()
        }
        JwwEditCapability::MappedV600 => {
            manifest.schema_version == CURRENT_SCHEMA_VERSION
                && manifest.records_relative_path.as_deref() == Some(JWW_RECORDS_RELATIVE_PATH)
                && manifest.records_blake3.is_some()
                && manifest.records_sha256.is_some()
        }
    }
}

pub fn verified_jww_preservation_snapshot(
    root: impl AsRef<Path>,
) -> ModelResult<Option<VerifiedJwwPreservationSnapshot>> {
    verified_jww_preservation_snapshot_with_hook(root.as_ref(), || {})
}

fn verified_jww_preservation_snapshot_with_hook(
    root: &Path,
    after_verified_reads: impl FnOnce(),
) -> ModelResult<Option<VerifiedJwwPreservationSnapshot>> {
    let source_manifest_before = source_manifest(root)?;
    let Some(manifest) = load_jww_preservation_manifest(root)? else {
        return Ok(None);
    };
    validate_jww_preservation_manifest(root, &manifest)?;
    let original_bytes = read_canonical_source_bytes(root, &root.join(JWW_ORIGINAL_RELATIVE_PATH))?;
    if !jww_bytes_match(
        &original_bytes,
        &manifest.original_blake3,
        &manifest.original_sha256,
    ) {
        return Err(ModelError::JwwPreservation {
            path: root.join(JWW_ORIGINAL_RELATIVE_PATH),
            reason: "preserved JWW original hash does not match the manifest".to_owned(),
        });
    }
    let record_provenance_bytes = read_jww_record_provenance_bytes(root, &manifest)?;
    if let Some(records) = &record_provenance_bytes
        && !jww_bytes_match(
            records,
            manifest.records_blake3.as_deref().expect("validated hash"),
            manifest.records_sha256.as_deref().expect("validated hash"),
        )
    {
        return Err(ModelError::JwwPreservation {
            path: root.join(JWW_RECORDS_RELATIVE_PATH),
            reason: "JWW record provenance hash does not match the manifest".to_owned(),
        });
    }
    after_verified_reads();
    let source_manifest_after = source_manifest(root)?;
    if source_manifest_before != source_manifest_after {
        return Err(ModelError::JwwPreservation {
            path: root.to_path_buf(),
            reason: "revision_conflict: canonical project sources changed while the JWW preservation snapshot was read".to_owned(),
        });
    }
    Ok(Some(VerifiedJwwPreservationSnapshot {
        manifest,
        original_bytes,
        record_provenance_bytes,
        source_manifest: source_manifest_after,
    }))
}

pub fn jww_project_compatibility(
    root: impl AsRef<Path>,
) -> ModelResult<Option<JwwProjectCompatibility>> {
    let root = root.as_ref();
    let Some(manifest) = load_jww_preservation_manifest(root)? else {
        return Ok(None);
    };
    validate_jww_preservation_manifest(root, &manifest)?;
    let original_path = root.join(JWW_ORIGINAL_RELATIVE_PATH);
    let original = read_canonical_source_bytes(root, &original_path)?;
    let original_verified = jww_bytes_match(
        &original,
        &manifest.original_blake3,
        &manifest.original_sha256,
    );
    let records_verified = match manifest.edit_capability {
        JwwEditCapability::ExactOnly => jww_record_metadata_is_canonical(&manifest),
        JwwEditCapability::MappedV600 => {
            let canonical_path = jww_record_metadata_is_canonical(&manifest);
            let expected_blake3 = manifest.records_blake3.as_deref();
            let expected_sha256 = manifest.records_sha256.as_deref();
            if !canonical_path || expected_blake3.is_none() || expected_sha256.is_none() {
                false
            } else {
                read_canonical_source_bytes(root, &root.join(JWW_RECORDS_RELATIVE_PATH))
                    .map(|records| {
                        jww_bytes_match(
                            &records,
                            expected_blake3.unwrap(),
                            expected_sha256.unwrap(),
                        )
                    })
                    .unwrap_or(false)
            }
        }
    };
    let current = jww_relevant_source_manifest(root)?;
    let expected = manifest
        .source_revisions
        .iter()
        .map(|file| (file.relative_path.as_str(), (&file.revision, file.exists)))
        .collect::<BTreeMap<_, _>>();
    let actual = current
        .iter()
        .map(|file| (file.relative_path.as_str(), (&file.revision, file.exists)))
        .collect::<BTreeMap<_, _>>();
    let changed_source_paths = expected
        .keys()
        .chain(actual.keys())
        .filter(|path| expected.get(**path) != actual.get(**path))
        .map(|path| (*path).to_owned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let reason = if !original_verified {
        Some("preserved JWW original hash does not match the manifest".to_owned())
    } else if !records_verified {
        Some("JWW record provenance is missing or failed hash verification".to_owned())
    } else {
        manifest.reason
    };
    Ok(Some(JwwProjectCompatibility {
        state: if original_verified && records_verified {
            manifest.state
        } else {
            JwwCompatibilityState::Malformed
        },
        reason,
        original_verified,
        changed_source_paths,
        edit_capability: if original_verified && records_verified {
            manifest.edit_capability
        } else {
            JwwEditCapability::ExactOnly
        },
    }))
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntityId {
    raw: String,
    ulid: Ulid,
}

impl EntityId {
    pub fn parse(value: &str) -> ModelResult<Self> {
        let Some(encoded) = value.strip_prefix("ent_") else {
            return Err(ModelError::InvalidEntityId {
                value: value.to_owned(),
            });
        };
        let Ok(ulid) = Ulid::from_string(encoded) else {
            return Err(ModelError::InvalidEntityId {
                value: value.to_owned(),
            });
        };
        Ok(Self {
            raw: value.to_owned(),
            ulid,
        })
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.raw
    }

    #[must_use]
    pub fn ulid(&self) -> Ulid {
        self.ulid
    }
}

impl<'de> Deserialize<'de> for EntityId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(serde::de::Error::custom)
    }
}

impl Serialize for EntityId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.raw)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    pub schema_version: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayerRules {
    #[serde(default)]
    pub groups: BTreeMap<String, LayerGroupDef>,
    #[serde(default)]
    pub active_layer: Option<String>,
    pub layers: BTreeMap<String, LayerDef>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayerGroupDef {
    pub name: String,
    pub order: u16,
    pub scale_denominator: f64,
    pub visible: bool,
    pub locked: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayerDef {
    pub name: String,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub order: u16,
    #[serde(default)]
    pub locked: bool,
    pub visible: bool,
    pub printable: bool,
    pub color: String,
    pub line_type: String,
    pub line_width: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StyleRules {
    pub colors: BTreeMap<String, ColorDef>,
    pub line_types: BTreeMap<String, LineTypeDef>,
    #[serde(default)]
    pub pens: BTreeMap<String, PenStyleDef>,
    pub text_styles: BTreeMap<String, TextStyleDef>,
    pub dimension_styles: BTreeMap<String, DimensionStyleDef>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ColorDef {
    pub rgb: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub print_rgb: Option<String>,
    pub print_width: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LineTypeDef {
    pub dash: Vec<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PenStyleDef {
    pub color: String,
    pub line_type: String,
    pub line_width: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TextStyleDef {
    pub font_family: String,
    pub height: f64,
    pub width: f64,
    pub spacing: f64,
    pub align: TextAlign,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DimensionStyleDef {
    pub text_style: String,
    pub arrow_size: f64,
    pub extension_gap: f64,
    pub precision: u8,
    pub unit: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayoutConfig {
    pub name: String,
    pub paper: String,
    pub orientation: SheetOrientation,
    pub scale: String,
    pub origin: Point,
    #[serde(default = "default_layout_margins")]
    pub margins: [f64; 4],
    #[serde(default)]
    pub plot_area: Option<[f64; 4]>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LayoutsConfig {
    pub schema_version: String,
    pub active_layout: String,
    pub layouts: BTreeMap<String, LayoutConfig>,
}

fn default_layout_margins() -> [f64; 4] {
    [0.0; 4]
}

impl LayoutsConfig {
    #[must_use]
    pub fn active(&self) -> Option<&LayoutConfig> {
        self.layouts.get(&self.active_layout)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BlockDefinitionConfig {
    pub schema_version: String,
    pub name: String,
    #[serde(default)]
    pub base_point: Point,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockDefinition {
    pub id: String,
    pub config: BlockDefinitionConfig,
    pub entities: Vec<EntityRecord>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SheetOrientation {
    Portrait,
    Landscape,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Entity {
    Line {
        schema_version: String,
        id: EntityId,
        layer: String,
        #[serde(default)]
        pen: Option<String>,
        p1: Point,
        p2: Point,
    },
    Polyline {
        schema_version: String,
        id: EntityId,
        layer: String,
        #[serde(default)]
        pen: Option<String>,
        points: Vec<Point>,
        closed: bool,
    },
    Arc {
        schema_version: String,
        id: EntityId,
        layer: String,
        #[serde(default)]
        pen: Option<String>,
        center: Point,
        radius: f64,
        start_deg: f64,
        end_deg: f64,
    },
    Circle {
        schema_version: String,
        id: EntityId,
        layer: String,
        #[serde(default)]
        pen: Option<String>,
        center: Point,
        radius: f64,
    },
    Ellipse {
        schema_version: String,
        id: EntityId,
        layer: String,
        #[serde(default)]
        pen: Option<String>,
        center: Point,
        radius_x: f64,
        radius_y: f64,
        rotation_deg: f64,
        start_deg: f64,
        end_deg: f64,
    },
    Text {
        schema_version: String,
        id: EntityId,
        layer: String,
        #[serde(default)]
        pen: Option<String>,
        style: String,
        at: Point,
        rotation_deg: f64,
        #[serde(default)]
        mirror_y: bool,
        value: String,
    },
    Dimension {
        schema_version: String,
        id: EntityId,
        layer: String,
        #[serde(default)]
        pen: Option<String>,
        style: String,
        p1: Point,
        p2: Point,
        offset: f64,
        #[serde(default)]
        text_rotation_deg: f64,
        #[serde(default)]
        text_mirror_y: bool,
        value: Option<String>,
    },
    Point {
        schema_version: String,
        id: EntityId,
        layer: String,
        #[serde(default)]
        pen: Option<String>,
        at: Point,
        #[serde(default)]
        temporary: bool,
        #[serde(default)]
        marker_code: Option<u32>,
        #[serde(default)]
        rotation_deg: f64,
        #[serde(default = "default_entity_scale")]
        scale: f64,
    },
    Solid {
        schema_version: String,
        id: EntityId,
        layer: String,
        #[serde(default)]
        pen: Option<String>,
        points: Vec<Point>,
        fill: String,
    },
    CurveSolid {
        schema_version: String,
        id: EntityId,
        layer: String,
        #[serde(default)]
        pen: Option<String>,
        center: Point,
        radius: f64,
        flatness: f64,
        rotation_deg: f64,
        start_deg: f64,
        end_deg: f64,
        solid_param: f64,
        encoding_code: u16,
        fill: String,
    },
    BlockRef {
        schema_version: String,
        id: EntityId,
        layer: String,
        #[serde(default)]
        pen: Option<String>,
        block: String,
        at: Point,
        rotation_deg: f64,
        scale: f64,
    },
    Hatch {
        schema_version: String,
        id: EntityId,
        layer: String,
        #[serde(default)]
        pen: Option<String>,
        loops: Vec<Vec<Point>>,
        pattern: String,
        angle_deg: f64,
        scale: f64,
        #[serde(default)]
        fill: Option<String>,
    },
}

impl Entity {
    #[must_use]
    pub fn id(&self) -> &EntityId {
        match self {
            Self::Line { id, .. }
            | Self::Polyline { id, .. }
            | Self::Arc { id, .. }
            | Self::Circle { id, .. }
            | Self::Ellipse { id, .. }
            | Self::Text { id, .. }
            | Self::Dimension { id, .. }
            | Self::Point { id, .. }
            | Self::Solid { id, .. }
            | Self::CurveSolid { id, .. }
            | Self::BlockRef { id, .. }
            | Self::Hatch { id, .. } => id,
        }
    }

    #[must_use]
    pub fn schema_version(&self) -> &str {
        match self {
            Self::Line { schema_version, .. }
            | Self::Polyline { schema_version, .. }
            | Self::Arc { schema_version, .. }
            | Self::Circle { schema_version, .. }
            | Self::Ellipse { schema_version, .. }
            | Self::Text { schema_version, .. }
            | Self::Dimension { schema_version, .. }
            | Self::Point { schema_version, .. }
            | Self::Solid { schema_version, .. }
            | Self::CurveSolid { schema_version, .. }
            | Self::BlockRef { schema_version, .. }
            | Self::Hatch { schema_version, .. } => schema_version,
        }
    }

    #[must_use]
    pub fn layer(&self) -> &str {
        match self {
            Self::Line { layer, .. }
            | Self::Polyline { layer, .. }
            | Self::Arc { layer, .. }
            | Self::Circle { layer, .. }
            | Self::Ellipse { layer, .. }
            | Self::Text { layer, .. }
            | Self::Dimension { layer, .. }
            | Self::Point { layer, .. }
            | Self::Solid { layer, .. }
            | Self::CurveSolid { layer, .. }
            | Self::BlockRef { layer, .. }
            | Self::Hatch { layer, .. } => layer,
        }
    }

    #[must_use]
    pub fn pen(&self) -> Option<&str> {
        match self {
            Self::Line { pen, .. }
            | Self::Polyline { pen, .. }
            | Self::Arc { pen, .. }
            | Self::Circle { pen, .. }
            | Self::Ellipse { pen, .. }
            | Self::Text { pen, .. }
            | Self::Dimension { pen, .. }
            | Self::Point { pen, .. }
            | Self::BlockRef { pen, .. }
            | Self::Hatch { pen, .. } => pen.as_deref(),
            Self::Solid { pen, .. } | Self::CurveSolid { pen, .. } => pen.as_deref(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedStroke {
    pub color_rgb: String,
    pub print_color_rgb: String,
    pub line_width_mm: f64,
    pub dash: Vec<f64>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum StyleResolutionError {
    #[error("entity {entity_id:?} references missing layer {layer:?}")]
    MissingLayer { entity_id: String, layer: String },
    #[error("entity {entity_id:?} references missing pen {pen:?}")]
    MissingPen { entity_id: String, pen: String },
    #[error("entity {entity_id:?} references missing color {color:?}")]
    MissingColor { entity_id: String, color: String },
    #[error("entity {entity_id:?} references missing line type {line_type:?}")]
    MissingLineType {
        entity_id: String,
        line_type: String,
    },
}

pub fn resolve_entity_stroke(
    project: &ProjectSource,
    entity: &Entity,
) -> Result<ResolvedStroke, StyleResolutionError> {
    let entity_id = entity.id().as_str();
    let layer = project.layers.layers.get(entity.layer()).ok_or_else(|| {
        StyleResolutionError::MissingLayer {
            entity_id: entity_id.to_owned(),
            layer: entity.layer().to_owned(),
        }
    })?;
    let (color_id, line_type_id, line_width_mm) =
        if let Some(pen_id) = entity.pen() {
            let pen = project.styles.pens.get(pen_id).ok_or_else(|| {
                StyleResolutionError::MissingPen {
                    entity_id: entity_id.to_owned(),
                    pen: pen_id.to_owned(),
                }
            })?;
            (&pen.color, &pen.line_type, pen.line_width)
        } else {
            (&layer.color, &layer.line_type, layer.line_width)
        };
    let color =
        project
            .styles
            .colors
            .get(color_id)
            .ok_or_else(|| StyleResolutionError::MissingColor {
                entity_id: entity_id.to_owned(),
                color: color_id.clone(),
            })?;
    let line_type = project.styles.line_types.get(line_type_id).ok_or_else(|| {
        StyleResolutionError::MissingLineType {
            entity_id: entity_id.to_owned(),
            line_type: line_type_id.clone(),
        }
    })?;
    Ok(ResolvedStroke {
        color_rgb: color.rgb.clone(),
        print_color_rgb: color.print_rgb.clone().unwrap_or_else(|| color.rgb.clone()),
        line_width_mm,
        dash: line_type.dash.clone(),
    })
}

pub fn resolve_print_fill_color(
    project: &ProjectSource,
    entity: &Entity,
    color_id: &str,
) -> Result<String, StyleResolutionError> {
    project
        .styles
        .colors
        .get(color_id)
        .map(|color| color.print_rgb.clone().unwrap_or_else(|| color.rgb.clone()))
        .ok_or_else(|| StyleResolutionError::MissingColor {
            entity_id: entity.id().as_str().to_owned(),
            color: color_id.to_owned(),
        })
}

pub fn resolve_fill_color(
    project: &ProjectSource,
    entity: &Entity,
    color_id: &str,
) -> Result<String, StyleResolutionError> {
    project
        .styles
        .colors
        .get(color_id)
        .map(|color| color.rgb.clone())
        .ok_or_else(|| StyleResolutionError::MissingColor {
            entity_id: entity.id().as_str().to_owned(),
            color: color_id.to_owned(),
        })
}

fn default_entity_scale() -> f64 {
    1.0
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BBox {
    pub min: Point,
    pub max: Point,
}

impl BBox {
    pub fn from_points(points: &[Point]) -> Option<Self> {
        let mut iter = points.iter();
        let first = *iter.next()?;
        if !point_is_finite(first) {
            return None;
        }

        let mut bbox = Self {
            min: first,
            max: first,
        };
        for point in iter {
            if !point_is_finite(*point) {
                return None;
            }
            bbox.min[0] = bbox.min[0].min(point[0]);
            bbox.min[1] = bbox.min[1].min(point[1]);
            bbox.max[0] = bbox.max[0].max(point[0]);
            bbox.max[1] = bbox.max[1].max(point[1]);
        }
        Some(bbox)
    }

    pub fn from_center_radius(center: Point, radius: f64) -> Option<Self> {
        if !point_is_finite(center) || !radius.is_finite() || radius <= 0.0 {
            return None;
        }
        Some(Self {
            min: [center[0] - radius, center[1] - radius],
            max: [center[0] + radius, center[1] + radius],
        })
    }

    #[must_use]
    pub fn width(&self) -> f64 {
        self.max[0] - self.min[0]
    }

    #[must_use]
    pub fn height(&self) -> f64 {
        self.max[1] - self.min[1]
    }
}

pub fn entity_bbox(entity: &Entity) -> Option<BBox> {
    match entity {
        Entity::Line { p1, p2, .. } | Entity::Dimension { p1, p2, .. } => {
            BBox::from_points(&[*p1, *p2])
        }
        Entity::Polyline { points, .. } => BBox::from_points(points),
        Entity::Arc { center, radius, .. } | Entity::Circle { center, radius, .. } => {
            BBox::from_center_radius(*center, *radius)
        }
        Entity::Ellipse {
            center,
            radius_x,
            radius_y,
            rotation_deg,
            start_deg,
            end_deg,
            ..
        } => ellipse_bbox(
            *center,
            *radius_x,
            *radius_y,
            *rotation_deg,
            *start_deg,
            *end_deg,
        ),
        Entity::Solid { points, .. } => BBox::from_points(points),
        Entity::CurveSolid {
            center,
            radius,
            flatness,
            rotation_deg,
            start_deg,
            end_deg,
            ..
        } => ellipse_bbox(
            *center,
            radius.abs(),
            radius.abs() * flatness.abs(),
            *rotation_deg,
            *start_deg,
            *end_deg,
        ),
        Entity::Text { at, .. } => BBox::from_points(&[*at]),
        Entity::Point { at, scale, .. } => {
            let radius = 2.5 * scale.abs();
            BBox::from_points(&[
                [at[0] - radius, at[1] - radius],
                [at[0] + radius, at[1] + radius],
            ])
        }
        Entity::BlockRef { at, scale, .. } => {
            let radius = 2.5 * scale.abs();
            BBox::from_points(&[
                [at[0] - radius, at[1] - radius],
                [at[0] + radius, at[1] + radius],
            ])
        }
        Entity::Hatch { loops, .. } => {
            let points = loops
                .iter()
                .flat_map(|loop_points| loop_points.iter().copied())
                .collect::<Vec<_>>();
            BBox::from_points(&points)
        }
    }
}

/// Offsets a dimension segment along its signed unit normal. Keeping this
/// geometry in the model prevents render and diff implementations diverging.
#[must_use]
pub fn dimension_offset_segment(p1: Point, p2: Point, offset: f64) -> Option<(Point, Point)> {
    if !point_is_finite(p1) || !point_is_finite(p2) || !offset.is_finite() {
        return None;
    }
    let dx = p2[0] - p1[0];
    let dy = p2[1] - p1[1];
    let length = (dx * dx + dy * dy).sqrt();
    if length <= f64::EPSILON {
        return None;
    }
    let normal = [-dy / length, dx / length];
    let delta = [normal[0] * offset, normal[1] * offset];
    Some((
        [p1[0] + delta[0], p1[1] + delta[1]],
        [p2[0] + delta[0], p2[1] + delta[1]],
    ))
}

/// Returns a point on an ellipse where `parameter_deg` is measured in the
/// ellipse's unrotated local coordinate system.
#[must_use]
pub fn ellipse_point(
    center: Point,
    radius_x: f64,
    radius_y: f64,
    rotation_deg: f64,
    parameter_deg: f64,
) -> Point {
    let parameter = parameter_deg.to_radians();
    let rotation = rotation_deg.to_radians();
    let local_x = radius_x * parameter.cos();
    let local_y = radius_y * parameter.sin();
    [
        center[0] + local_x * rotation.cos() - local_y * rotation.sin(),
        center[1] + local_x * rotation.sin() + local_y * rotation.cos(),
    ]
}

/// Computes the exact axis-aligned bounds for a rotated ellipse arc by adding
/// every derivative extremum that falls inside the signed parameter sweep.
#[must_use]
pub fn ellipse_bbox(
    center: Point,
    radius_x: f64,
    radius_y: f64,
    rotation_deg: f64,
    start_deg: f64,
    end_deg: f64,
) -> Option<BBox> {
    if !point_is_finite(center)
        || !radius_x.is_finite()
        || !radius_y.is_finite()
        || !rotation_deg.is_finite()
        || !start_deg.is_finite()
        || !end_deg.is_finite()
        || radius_x <= 0.0
        || radius_y <= 0.0
        || (end_deg - start_deg).abs() <= f64::EPSILON
    {
        return None;
    }

    let rotation = rotation_deg.to_radians();
    let x_extreme = (-radius_y * rotation.sin()).atan2(radius_x * rotation.cos());
    let y_extreme = (radius_y * rotation.cos()).atan2(radius_x * rotation.sin());
    let candidates = [
        start_deg,
        end_deg,
        x_extreme.to_degrees(),
        x_extreme.to_degrees() + 180.0,
        y_extreme.to_degrees(),
        y_extreme.to_degrees() + 180.0,
    ];
    let points = candidates
        .into_iter()
        .filter(|parameter| angle_is_on_sweep(*parameter, start_deg, end_deg))
        .map(|parameter| ellipse_point(center, radius_x, radius_y, rotation_deg, parameter))
        .collect::<Vec<_>>();
    BBox::from_points(&points)
}

fn angle_is_on_sweep(candidate_deg: f64, start_deg: f64, end_deg: f64) -> bool {
    const ANGLE_EPSILON_DEG: f64 = 1e-9;

    let sweep = end_deg - start_deg;
    if sweep.abs() >= 360.0 - ANGLE_EPSILON_DEG {
        return true;
    }
    if sweep > 0.0 {
        (candidate_deg - start_deg).rem_euclid(360.0) <= sweep + ANGLE_EPSILON_DEG
    } else {
        (start_deg - candidate_deg).rem_euclid(360.0) <= -sweep + ANGLE_EPSILON_DEG
    }
}

fn point_is_finite(point: Point) -> bool {
    point[0].is_finite() && point[1].is_finite()
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntityRecord {
    pub line: usize,
    pub entity: Entity,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DrawingSource {
    pub name: String,
    pub layouts: LayoutsConfig,
    pub entities: Vec<EntityRecord>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectSource {
    pub root: PathBuf,
    pub project: ProjectConfig,
    pub layers: LayerRules,
    pub styles: StyleRules,
    pub blocks: BTreeMap<String, BlockDefinition>,
    pub drawings: Vec<DrawingSource>,
}

pub fn load_project(root: impl AsRef<Path>) -> ModelResult<ProjectSource> {
    let root = root.as_ref();
    let project_path = root.join("cad.project.toml");
    let layers_path = root.join("rules/layers.toml");
    let styles_path = root.join("rules/styles.toml");

    let project: ProjectConfig = read_toml(root, &project_path)?;
    ensure_schema(&project_path, None, &project.schema_version)?;

    let layers = read_toml(root, &layers_path)?;
    let styles = read_toml(root, &styles_path)?;
    let blocks = read_blocks(root)?;
    let drawings = read_drawings(root)?;

    Ok(ProjectSource {
        root: root.to_path_buf(),
        project,
        layers,
        styles,
        blocks,
        drawings,
    })
}

#[must_use]
pub fn format_decimal_mm(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    let mut text = format!("{rounded:.3}");
    while text.contains('.') && text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" {
        return "0".to_owned();
    }
    text
}

fn read_drawings(root: &Path) -> ModelResult<Vec<DrawingSource>> {
    let drawings_dir = root.join("drawings");
    let mut entries = fs::read_dir(&drawings_dir)
        .map_err(|source| ModelError::ListDir {
            path: drawings_dir.clone(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| ModelError::ListDir {
            path: drawings_dir.clone(),
            source,
        })?;

    entries.sort_by_key(|entry| entry.file_name());

    entries
        .into_iter()
        .filter_map(|entry| match entry.file_type() {
            Ok(file_type) if file_type.is_dir() => Some(Ok(entry)),
            Ok(file_type) if file_type.is_symlink() => Some(Err(ModelError::UnsafeSourcePath {
                path: entry.path(),
                reason: "drawing directories may not be symlinks".to_owned(),
            })),
            Ok(_) => None,
            Err(source) => Some(Err(ModelError::ListDir {
                path: drawings_dir.clone(),
                source,
            })),
        })
        .map(|entry| {
            let entry = entry?;
            let drawing_dir = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let entities_path = drawing_dir.join("entities.ndjson");
            let layouts_path = drawing_dir.join("layouts.toml");
            let layouts: LayoutsConfig = read_toml(root, &layouts_path)?;
            ensure_schema(&layouts_path, None, &layouts.schema_version)?;
            let entities = read_entities(root, &entities_path)?;

            Ok(DrawingSource {
                name,
                layouts,
                entities,
            })
        })
        .collect()
}

fn read_blocks(root: &Path) -> ModelResult<BTreeMap<String, BlockDefinition>> {
    let blocks_dir = root.join("blocks");
    if !blocks_dir.exists() {
        return Ok(BTreeMap::new());
    }
    let mut entries = fs::read_dir(&blocks_dir)
        .map_err(|source| ModelError::ListDir {
            path: blocks_dir.clone(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| ModelError::ListDir {
            path: blocks_dir.clone(),
            source,
        })?;
    entries.sort_by_key(|entry| entry.file_name());
    entries
        .into_iter()
        .filter_map(|entry| match entry.file_type() {
            Ok(file_type) if file_type.is_dir() => {
                let path = entry.path();
                let has_definition = path.join("definition.toml").exists();
                let has_entities = path.join("entities.ndjson").exists();
                if !has_definition && !has_entities {
                    None
                } else {
                    Some(Ok(entry))
                }
            }
            Ok(file_type) if file_type.is_symlink() => Some(Err(ModelError::UnsafeSourcePath {
                path: entry.path(),
                reason: "block directories may not be symlinks".to_owned(),
            })),
            Ok(_) => None,
            Err(source) => Some(Err(ModelError::ListDir {
                path: blocks_dir.clone(),
                source,
            })),
        })
        .map(|entry| {
            let entry = entry?;
            let block_dir = entry.path();
            let id = entry.file_name().to_string_lossy().into_owned();
            let definition_path = block_dir.join("definition.toml");
            let entities_path = block_dir.join("entities.ndjson");
            let config: BlockDefinitionConfig = read_toml(root, &definition_path)?;
            ensure_schema(&definition_path, None, &config.schema_version)?;
            let entities = read_entities(root, &entities_path)?;
            Ok((
                id.clone(),
                BlockDefinition {
                    id,
                    config,
                    entities,
                },
            ))
        })
        .collect()
}

fn read_toml<T>(root: &Path, path: &Path) -> ModelResult<T>
where
    T: for<'de> Deserialize<'de>,
{
    let text = read_canonical_source(root, path)?;
    toml::from_str(&text).map_err(|source| ModelError::Toml {
        path: path.to_path_buf(),
        source,
    })
}

fn read_entities(root: &Path, path: &Path) -> ModelResult<Vec<EntityRecord>> {
    let text = read_canonical_source(root, path)?;

    text.lines()
        .enumerate()
        .map(|(index, line)| {
            let line_number = index + 1;
            if line.trim().is_empty() {
                return Err(ModelError::EmptyNdjsonLine {
                    path: path.to_path_buf(),
                    line: line_number,
                });
            }

            let entity: Entity =
                serde_json::from_str(line).map_err(|source| ModelError::Ndjson {
                    path: path.to_path_buf(),
                    line: line_number,
                    source,
                })?;
            ensure_schema(path, Some(line_number), entity.schema_version())?;
            Ok(EntityRecord {
                line: line_number,
                entity,
            })
        })
        .collect()
}

fn read_canonical_source(root: &Path, path: &Path) -> ModelResult<String> {
    let bytes = read_canonical_source_bytes(root, path)?;
    String::from_utf8(bytes).map_err(|source| ModelError::Read {
        path: path.to_path_buf(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, source),
    })
}

fn read_canonical_source_bytes(root: &Path, path: &Path) -> ModelResult<Vec<u8>> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| ModelError::UnsafeSourcePath {
            path: path.to_path_buf(),
            reason: "path is outside the project root".to_owned(),
        })?;
    if classify_project_source_path(relative).is_none() {
        return Err(ModelError::UnsafeSourcePath {
            path: path.to_path_buf(),
            reason: "path is not a canonical project source".to_owned(),
        });
    }

    let canonical_root = fs::canonicalize(root).map_err(|source| ModelError::Read {
        path: root.to_path_buf(),
        source,
    })?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        let metadata = fs::symlink_metadata(&current).map_err(|source| ModelError::Read {
            path: current.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(ModelError::UnsafeSourcePath {
                path: current,
                reason: "symlinks are not allowed in canonical source paths".to_owned(),
            });
        }
    }
    let canonical_parent =
        fs::canonicalize(path.parent().unwrap_or(root)).map_err(|source| ModelError::Read {
            path: path.parent().unwrap_or(root).to_path_buf(),
            source,
        })?;
    if !canonical_parent.starts_with(&canonical_root) {
        return Err(ModelError::UnsafeSourcePath {
            path: path.to_path_buf(),
            reason: "resolved parent escapes the project root".to_owned(),
        });
    }

    fs::read(path).map_err(|source| ModelError::Read {
        path: path.to_path_buf(),
        source,
    })
}

fn ensure_schema(path: &Path, line: Option<usize>, found: &str) -> ModelResult<()> {
    if found == CURRENT_SCHEMA_VERSION {
        return Ok(());
    }
    Err(ModelError::UnsupportedSchema {
        path: path.to_path_buf(),
        line,
        found: found.to_owned(),
        expected: CURRENT_SCHEMA_VERSION,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_only_canonical_project_sources() {
        assert_eq!(
            classify_project_source_path(Path::new("cad.project.toml")),
            Some(ProjectSourceKind::Project)
        );
        assert_eq!(
            classify_project_source_path(Path::new("drawings/plan/layouts.toml")),
            Some(ProjectSourceKind::Layout)
        );
        assert_eq!(
            classify_project_source_path(Path::new("blocks/door/entities.ndjson")),
            Some(ProjectSourceKind::BlockEntities)
        );
        assert_eq!(
            classify_project_source_path(Path::new("interop/jww/original.jww")),
            Some(ProjectSourceKind::JwwOriginal)
        );
        assert_eq!(
            classify_project_source_path(Path::new("interop/jww/preservation.toml")),
            Some(ProjectSourceKind::JwwPreservation)
        );
        assert_eq!(
            classify_project_source_path(Path::new("interop/jww/records.ndjson")),
            Some(ProjectSourceKind::JwwPreservation)
        );
        assert_eq!(
            classify_project_source_path(Path::new("interop/jww/extra.bin")),
            None
        );
        assert_eq!(
            classify_project_source_path(Path::new("drawings/plan/sheet.toml")),
            None
        );
        assert_eq!(
            classify_project_source_path(Path::new("build/.cad-history/index.json")),
            None
        );
        assert_eq!(
            classify_project_source_path(Path::new("drawings/plan/entities.ndjson.swp")),
            None
        );
        assert_eq!(
            classify_project_source_path(Path::new("drawings/../../entities.ndjson")),
            None
        );
        assert_eq!(
            classify_project_source_path(Path::new("./cad.project.toml")),
            None
        );
    }

    #[test]
    fn source_manifest_excludes_generated_and_obsolete_files() {
        let temp = minimal_project();
        write(
            temp.path().join("drawings/plan_1f/sheet.toml"),
            "obsolete = true\n",
        )
        .expect("obsolete fixture");
        create_dir_all(temp.path().join("build/.cad-history")).expect("history directory");
        write(temp.path().join("build/.cad-history/index.json"), "{}").expect("history fixture");

        let manifest = source_manifest(temp.path()).expect("manifest should load");
        let paths = manifest
            .into_iter()
            .map(|file| file.relative_path)
            .collect::<Vec<_>>();
        assert!(paths.contains(&"cad.project.toml".to_owned()));
        assert!(paths.contains(&"drawings/plan_1f/layouts.toml".to_owned()));
        assert!(!paths.iter().any(|path| path.ends_with("sheet.toml")));
        assert!(!paths.iter().any(|path| path.starts_with("build/")));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_canonical_source_files() {
        use std::os::unix::fs::symlink;

        for relative in [
            "cad.project.toml",
            "rules/layers.toml",
            "rules/styles.toml",
            "drawings/plan_1f/layouts.toml",
            "drawings/plan_1f/entities.ndjson",
        ] {
            let temp = minimal_project();
            let source = temp.path().join(relative);
            let outside = temp.path().join("outside-source");
            std::fs::rename(&source, &outside).expect("source should move outside canonical path");
            symlink(&outside, &source).expect("source symlink should be created");

            let error = load_project(temp.path()).expect_err("source symlink must fail closed");
            assert!(matches!(error, ModelError::UnsafeSourcePath { .. }));
        }
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_drawing_and_block_directories() {
        use std::os::unix::fs::symlink;

        let drawing_project = minimal_project();
        let drawing = drawing_project.path().join("drawings/plan_1f");
        let outside_drawing = drawing_project.path().join("outside-drawing");
        std::fs::rename(&drawing, &outside_drawing).expect("drawing should move");
        symlink(&outside_drawing, &drawing).expect("drawing symlink should be created");
        assert!(matches!(
            load_project(drawing_project.path()),
            Err(ModelError::UnsafeSourcePath { .. })
        ));

        let block_project = minimal_project();
        let outside_block = block_project.path().join("outside-block");
        create_dir_all(&outside_block).expect("outside block directory");
        write(
            outside_block.join("definition.toml"),
            "schema_version = \"0.2\"\nname = \"fixture\"\nbase_point = [0.0, 0.0]\n",
        )
        .expect("block definition");
        write(outside_block.join("entities.ndjson"), "").expect("block entities");
        create_dir_all(block_project.path().join("blocks")).expect("blocks directory");
        symlink(&outside_block, block_project.path().join("blocks/fixture"))
            .expect("block symlink should be created");
        assert!(matches!(
            load_project(block_project.path()),
            Err(ModelError::UnsafeSourcePath { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn source_manifest_rejects_symlinked_canonical_sources() {
        use std::os::unix::fs::symlink;

        let temp = minimal_project();
        let source = temp.path().join("drawings/plan_1f/layouts.toml");
        let outside = temp.path().join("outside-layouts.toml");
        std::fs::rename(&source, &outside).expect("layout should move");
        symlink(&outside, &source).expect("layout symlink should be created");

        assert!(matches!(
            source_manifest(temp.path()),
            Err(ModelError::UnsafeSourcePath { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn jww_preservation_reader_rejects_symlinked_interop_directory() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let project = temp.path().join("project");
        let outside = temp.path().join("outside-jww");
        create_dir_all(project.join("interop")).expect("interop directory");
        create_dir_all(&outside).expect("outside directory");
        write(
            outside.join("preservation.toml"),
            "schema_version = \"0.1\"\n",
        )
        .expect("outside manifest");
        symlink(&outside, project.join("interop/jww")).expect("interop symlink");

        assert!(matches!(
            load_jww_preservation_manifest(&project),
            Err(ModelError::UnsafeSourcePath { .. })
        ));
    }

    #[test]
    fn verified_jww_snapshot_supports_exact_only_and_detects_source_races() {
        let temp = minimal_project();
        create_dir_all(temp.path().join("interop/jww")).expect("interop directory");
        let original = b"exact original bytes";
        write(temp.path().join(JWW_ORIGINAL_RELATIVE_PATH), original).expect("original");
        write(
            temp.path().join(JWW_PRESERVATION_RELATIVE_PATH),
            format!(
                concat!(
                    "schema_version = \"0.1\"\n",
                    "state = \"preserved_read_only\"\n",
                    "drawing_name = \"plan_1f\"\n",
                    "original_relative_path = \"interop/jww/original.jww\"\n",
                    "original_blake3 = \"{}\"\n",
                    "original_sha256 = \"{}\"\n",
                    "edit_capability = \"exact_only\"\n",
                ),
                blake3::hash(original).to_hex(),
                format!("{:x}", Sha256::digest(original)),
            ),
        )
        .expect("manifest");

        let snapshot = verified_jww_preservation_snapshot(temp.path())
            .expect("snapshot")
            .expect("provenance");
        assert_eq!(snapshot.original_bytes, original);
        assert!(snapshot.record_provenance_bytes.is_none());

        let entities = temp.path().join("drawings/plan_1f/entities.ndjson");
        let error = verified_jww_preservation_snapshot_with_hook(temp.path(), || {
            write(&entities, "").expect("mutate canonical source");
        })
        .expect_err("source mutation must invalidate the snapshot");
        assert!(matches!(error, ModelError::JwwPreservation { .. }));
    }
    use std::fs::{create_dir_all, write};

    #[test]
    fn exposes_crate_name() {
        assert_eq!(crate_name(), "cad-model");
    }

    #[test]
    fn loads_house_small_example() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");

        let project = load_project(root).expect("example project should load");

        assert_eq!(project.project.name, "house-small");
        assert_eq!(project.drawings.len(), 1);
        assert_eq!(project.drawings[0].name, "plan_1f");
        assert_eq!(project.drawings[0].entities.len(), 1);
        assert_eq!(project.drawings[0].entities[0].line, 1);
        assert_eq!(
            project.drawings[0].entities[0].entity.id().as_str(),
            "ent_01JZ0000000000000000000000"
        );
    }

    #[test]
    fn layouts_toml_is_required_for_each_drawing() {
        let temp = minimal_project();
        std::fs::remove_file(temp.path().join("drawings/plan_1f/layouts.toml"))
            .expect("layouts file");

        let error = load_project(temp.path()).expect_err("missing layouts must fail");

        assert!(matches!(error, ModelError::Read { .. }));
    }

    #[test]
    fn obsolete_sheet_file_is_not_loaded_as_a_second_layout_source() {
        let temp = minimal_project();
        write(
            temp.path().join("drawings/plan_1f/sheet.toml"),
            "this is not valid TOML",
        )
        .expect("obsolete sheet file should be writable");

        load_project(temp.path()).expect("schema 0.2 should use layouts.toml only");
    }

    #[test]
    fn parses_all_mvp_entity_types() {
        let source = [
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0.0,0.0],"p2":[1.0,0.0]}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000001","type":"polyline","layer":"0-1","points":[[0.0,0.0],[1.0,0.0]],"closed":false}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000002","type":"arc","layer":"0-1","center":[0.0,0.0],"radius":1.0,"start_deg":0.0,"end_deg":90.0}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000003","type":"circle","layer":"0-1","center":[0.0,0.0],"radius":1.0}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000004","type":"ellipse","layer":"0-1","center":[0.0,0.0],"radius_x":2.0,"radius_y":1.0,"rotation_deg":30.0,"start_deg":0.0,"end_deg":180.0}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000005","type":"text","layer":"0-1","style":"note","at":[0.0,0.0],"rotation_deg":0.0,"value":"room"}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000006","type":"dimension","layer":"0-1","style":"dim_100","p1":[0.0,0.0],"p2":[1.0,0.0],"offset":100.0,"value":null}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000007","type":"block_ref","layer":"0-1","block":"door_910","at":[0.0,0.0],"rotation_deg":0.0,"scale":1.0}"#,
        ];

        for line in source {
            let entity: Entity = serde_json::from_str(line).expect("entity should parse");
            assert_eq!(entity.schema_version(), CURRENT_SCHEMA_VERSION);
        }
    }

    #[test]
    fn rejects_invalid_json_line() {
        let temp = minimal_project();
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            "{\"schema_version\":\"0.2\"",
        )
        .expect("fixture should be writable");

        let error = load_project(temp.path()).expect_err("invalid JSON should fail");

        assert!(matches!(error, ModelError::Ndjson { line: 1, .. }));
    }

    #[test]
    fn rejects_invalid_toml() {
        let temp = minimal_project();
        write(
            temp.path().join("drawings/plan_1f/layouts.toml"),
            "schema_version = ",
        )
        .expect("fixture should be writable");

        let error = load_project(temp.path()).expect_err("invalid TOML should fail");

        assert!(matches!(error, ModelError::Toml { .. }));
    }

    #[test]
    fn rejects_unknown_project_schema() {
        let temp = minimal_project();
        write(
            temp.path().join("cad.project.toml"),
            "schema_version = \"9.9\"\nname = \"bad\"\n",
        )
        .expect("fixture should be writable");

        let error = load_project(temp.path()).expect_err("unknown schema should fail");

        assert!(matches!(
            error,
            ModelError::UnsupportedSchema { line: None, .. }
        ));
    }

    #[test]
    fn rejects_unknown_entity_schema() {
        let temp = minimal_project();
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            r#"{"schema_version":"9.9","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0.0,0.0],"p2":[910.0,0.0]}"#,
        )
        .expect("fixture should be writable");

        let error = load_project(temp.path()).expect_err("unknown entity schema should fail");

        assert!(matches!(
            error,
            ModelError::UnsupportedSchema { line: Some(1), .. }
        ));
    }

    #[test]
    fn rejects_unknown_entity_type() {
        let temp = minimal_project();
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000000","type":"spline","layer":"0-1"}"#,
        )
        .expect("fixture should be writable");

        let error = load_project(temp.path()).expect_err("unknown entity type should fail");

        assert!(matches!(error, ModelError::Ndjson { line: 1, .. }));
    }

    #[test]
    fn rejects_invalid_entity_id() {
        let error = EntityId::parse("bad_01JZ0000000000000000000000")
            .expect_err("non entity prefix should fail");

        assert!(matches!(error, ModelError::InvalidEntityId { .. }));
    }

    #[test]
    fn fixes_decimal_format_snapshot() {
        insta::assert_snapshot!(
            [
                format_decimal_mm(1.2344),
                format_decimal_mm(1.2345),
                format_decimal_mm(10.0),
                format_decimal_mm(-0.0004),
            ]
            .join("\n"),
            @"
1.234
1.235
10
0
"
        );
    }

    #[test]
    fn computes_entity_bbox() {
        let entity: Entity = serde_json::from_str(
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[-1.0,2.0],"p2":[3.0,-4.0]}"#,
        )
        .expect("line should parse");

        let bbox = entity_bbox(&entity).expect("line should have bbox");

        assert_eq!(bbox.min, [-1.0, -4.0]);
        assert_eq!(bbox.max, [3.0, 2.0]);
    }

    #[test]
    fn computes_rotated_ellipse_and_arc_bbox() {
        let full = ellipse_bbox([10.0, 20.0], 4.0, 2.0, 90.0, 0.0, 360.0)
            .expect("full ellipse should have bbox");
        assert!((full.min[0] - 8.0).abs() < 1e-9);
        assert!((full.max[0] - 12.0).abs() < 1e-9);
        assert!((full.min[1] - 16.0).abs() < 1e-9);
        assert!((full.max[1] - 24.0).abs() < 1e-9);

        let quarter = ellipse_bbox([0.0, 0.0], 4.0, 2.0, 0.0, 0.0, 90.0)
            .expect("ellipse arc should have bbox");
        assert!(quarter.min[0].abs() < 1e-9);
        assert!(quarter.min[1].abs() < 1e-9);
        assert!((quarter.max[0] - 4.0).abs() < 1e-9);
        assert!((quarter.max[1] - 2.0).abs() < 1e-9);
    }

    #[test]
    fn defaults_text_mirror_fields_when_omitted() {
        let text: Entity = serde_json::from_str(
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000000","type":"text","layer":"0-1","style":"note","at":[0.0,0.0],"rotation_deg":0.0,"value":"room"}"#,
        )
        .expect("text should parse");
        let dimension: Entity = serde_json::from_str(
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000001","type":"dimension","layer":"0-1","style":"dim_100","p1":[0.0,0.0],"p2":[0.0,10.0],"offset":2.0,"value":null}"#,
        )
        .expect("dimension should parse");

        assert!(matches!(
            text,
            Entity::Text {
                mirror_y: false,
                ..
            }
        ));
        assert!(matches!(
            dimension,
            Entity::Dimension {
                text_rotation_deg: 0.0,
                text_mirror_y: false,
                ..
            }
        ));
    }

    #[test]
    fn offsets_dimension_along_line_normal() {
        let (d1, d2) = dimension_offset_segment([0.0, 0.0], [0.0, 10.0], 2.0)
            .expect("vertical dimension should offset");

        assert_eq!(d1, [-2.0, 0.0]);
        assert_eq!(d2, [-2.0, 10.0]);
    }

    #[test]
    fn resolves_layer_and_entity_pen_styles_through_one_contract() {
        let temp = minimal_project();
        let mut project = load_project(temp.path()).expect("project should load");
        project.styles.colors.insert(
            "red".to_owned(),
            ColorDef {
                rgb: "#FF0000".to_owned(),
                print_rgb: Some("#AA0000".to_owned()),
                print_width: 0.35,
            },
        );
        project.styles.line_types.insert(
            "dash".to_owned(),
            LineTypeDef {
                dash: vec![12.0, 6.0],
            },
        );
        project.styles.pens.insert(
            "red_dash".to_owned(),
            PenStyleDef {
                color: "red".to_owned(),
                line_type: "dash".to_owned(),
                line_width: 0.5,
            },
        );
        let mut entity = project.drawings[0].entities[0].entity.clone();
        let Entity::Line { pen, .. } = &mut entity else {
            panic!("fixture should be a line");
        };
        *pen = Some("red_dash".to_owned());

        assert_eq!(
            resolve_entity_stroke(&project, &entity).expect("style should resolve"),
            ResolvedStroke {
                color_rgb: "#FF0000".to_owned(),
                print_color_rgb: "#AA0000".to_owned(),
                line_width_mm: 0.5,
                dash: vec![12.0, 6.0],
            }
        );
    }

    fn minimal_project() -> tempfile::TempDir {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        create_dir_all(temp.path().join("rules")).expect("rules dir should be created");
        create_dir_all(temp.path().join("drawings/plan_1f"))
            .expect("drawing dir should be created");

        write(
            temp.path().join("cad.project.toml"),
            "schema_version = \"0.2\"\nname = \"fixture\"\n",
        )
        .expect("project TOML should be writable");
        write(
            temp.path().join("rules/layers.toml"),
            "[layers.\"0-1\"]\nname = \"A-WALL\"\nvisible = true\nprintable = true\ncolor = \"jw_black\"\nline_type = \"solid\"\nline_width = 0.25\n",
        )
        .expect("layers TOML should be writable");
        write(
            temp.path().join("rules/styles.toml"),
            "[colors.jw_black]\nrgb = \"#000000\"\nprint_width = 0.25\n\n[line_types.solid]\ndash = []\n\n[text_styles.note]\nfont_family = \"Hiragino Sans\"\nheight = 250\nwidth = 125\nspacing = 0\nalign = \"left\"\n\n[dimension_styles.dim_100]\ntext_style = \"note\"\narrow_size = 120\nextension_gap = 40\nprecision = 0\nunit = \"mm\"\n",
        )
        .expect("styles TOML should be writable");
        write(
            temp.path().join("drawings/plan_1f/layouts.toml"),
            "schema_version = \"0.2\"\nactive_layout = \"default\"\n\n[layouts.default]\nname = \"default\"\npaper = \"A3\"\norientation = \"landscape\"\nscale = \"1/100\"\norigin = [0.0, 0.0]\nmargins = [0.0, 0.0, 0.0, 0.0]\n",
        )
        .expect("layouts TOML should be writable");
        write(
            temp.path().join("drawings/plan_1f/entities.ndjson"),
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0.0,0.0],"p2":[910.0,0.0]}"#,
        )
        .expect("entities NDJSON should be writable");

        temp
    }
}
