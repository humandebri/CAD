//! Experimental CAD source to JWW boundary exporter.

use cad_jww_codec::{Base, BlockDefinition, Document, Header, Layer, LayerGroup, Record};
use cad_model::{Entity, LayerDef, ProjectSource, TextStyleDef};
use encoding_rs::SHIFT_JIS;
use rustix::fs::{CWD, RenameFlags, renameat_with};
use rustix::io::Errno;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const CRATE_NAME: &str = "cad-export-jww";

#[must_use]
pub fn crate_name() -> &'static str {
    CRATE_NAME
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ExportOptions {
    pub allow_lossy: bool,
    pub overwrite: bool,
    pub strict_approximations: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AutoExportOptions {
    pub strict: bool,
    pub overwrite: bool,
    pub require_preservation: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExportStatus {
    Exported,
    Blocked,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExportMode {
    #[deprecated(note = "use generated_best_effort or generated_strict")]
    GeneratedExperimental,
    GeneratedBestEffort,
    GeneratedStrict,
    PreservedExact,
    PreservedEdited,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ExportIssue {
    pub code: String,
    pub message: String,
    pub entity_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportReport {
    pub schema_version: String,
    pub status: ExportStatus,
    pub mode: ExportMode,
    pub output_path: String,
    pub written_entities: usize,
    pub expanded_entities: usize,
    pub warnings: Vec<ExportIssue>,
    pub blockers: Vec<ExportIssue>,
}

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("failed to load CAD project")]
    Model(#[from] cad_model::ModelError),
    #[error("drawing {0:?} was not found")]
    DrawingNotFound(String),
    #[error("output already exists: {0}")]
    OutputExists(PathBuf),
    #[error("failed to encode JWW")]
    Codec(#[from] cad_jww_codec::CodecError),
    #[error("failed to write {path}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("JWW preservation is unavailable: {0}")]
    Preservation(String),
    #[error("revision_conflict: canonical project sources changed during JWW export")]
    RevisionConflict,
}

pub type ExportResult<T> = Result<T, ExportError>;

pub fn export_jww_file(
    project_path: impl AsRef<Path>,
    drawing_name: &str,
    output_path: impl AsRef<Path>,
    options: ExportOptions,
) -> ExportResult<ExportReport> {
    let project_path = project_path.as_ref();
    let expected_manifest = cad_model::source_manifest(project_path)?;
    export_jww_file_with_expected_manifest(
        project_path,
        drawing_name,
        output_path.as_ref(),
        options,
        &expected_manifest,
        || {},
    )
}

fn export_jww_file_with_expected_manifest(
    project_path: &Path,
    drawing_name: &str,
    output_path: &Path,
    options: ExportOptions,
    expected_manifest: &[cad_model::SourceFileRevision],
    before_publish: impl FnOnce(),
) -> ExportResult<ExportReport> {
    ensure_preservation_snapshot_current(project_path, expected_manifest)?;
    let project = cad_model::load_project(project_path)?;
    ensure_preservation_snapshot_current(project_path, expected_manifest)?;
    let prepared = prepare_loaded_project(&project, drawing_name, output_path, options)?;
    let Some(bytes) = prepared.bytes else {
        return Ok(prepared.report);
    };
    before_publish();
    ensure_preservation_snapshot_current(project_path, expected_manifest)?;
    publish(output_path, &bytes, options.overwrite)?;
    Ok(prepared.report)
}

pub fn export_jww_file_auto(
    project_path: impl AsRef<Path>,
    drawing_name: &str,
    output_path: impl AsRef<Path>,
    options: AutoExportOptions,
) -> ExportResult<ExportReport> {
    let project_path = project_path.as_ref();
    let output_path = output_path.as_ref();
    if cad_model::load_jww_preservation_manifest(project_path)?.is_some() {
        return export_jww_file_preserving_with_options(
            project_path,
            drawing_name,
            output_path,
            options.overwrite,
            !options.strict,
        );
    }
    if options.require_preservation {
        return Err(ExportError::Preservation(
            "project has no JWW provenance".to_owned(),
        ));
    }
    export_jww_file(
        project_path,
        drawing_name,
        output_path,
        ExportOptions {
            allow_lossy: !options.strict,
            overwrite: options.overwrite,
            strict_approximations: options.strict,
        },
    )
}

pub fn extract_original_jww(
    project_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
    overwrite: bool,
) -> ExportResult<()> {
    let project_path = project_path.as_ref();
    let snapshot = cad_model::verified_jww_preservation_snapshot(project_path)?
        .ok_or_else(|| ExportError::Preservation("project has no JWW provenance".to_owned()))?;
    ensure_preservation_snapshot_current(project_path, &snapshot.source_manifest)?;
    publish(output_path.as_ref(), &snapshot.original_bytes, overwrite)
}

pub fn export_jww_file_preserving(
    project_path: impl AsRef<Path>,
    drawing_name: &str,
    output_path: impl AsRef<Path>,
    overwrite: bool,
) -> ExportResult<ExportReport> {
    export_jww_file_preserving_with_options(
        project_path,
        drawing_name,
        output_path,
        overwrite,
        false,
    )
}

pub fn export_jww_file_preserving_with_options(
    project_path: impl AsRef<Path>,
    drawing_name: &str,
    output_path: impl AsRef<Path>,
    overwrite: bool,
    allow_lossy: bool,
) -> ExportResult<ExportReport> {
    let project_path = project_path.as_ref();
    let output_path = output_path.as_ref();
    let snapshot = cad_model::verified_jww_preservation_snapshot(project_path)?
        .ok_or_else(|| ExportError::Preservation("project has no JWW provenance".to_owned()))?;
    let manifest = &snapshot.manifest;
    if manifest.drawing_name != drawing_name {
        return Err(ExportError::Preservation(format!(
            "preserved drawing is {:?}, not {drawing_name:?}",
            manifest.drawing_name
        )));
    }
    if output_path.exists() && !overwrite {
        return Err(ExportError::OutputExists(output_path.to_path_buf()));
    }
    let changed_source_paths = changed_jww_source_paths(manifest, &snapshot.source_manifest);
    if changed_source_paths.is_empty() {
        let entity_count = cad_jww_codec::read_document(&snapshot.original_bytes)
            .map(|document| document.entities.len())
            .unwrap_or(0);
        ensure_preservation_snapshot_current(project_path, &snapshot.source_manifest)?;
        publish(output_path, &snapshot.original_bytes, overwrite)?;
        return Ok(ExportReport {
            schema_version: "0.2".to_owned(),
            status: ExportStatus::Exported,
            mode: ExportMode::PreservedExact,
            output_path: output_path.display().to_string(),
            written_entities: entity_count,
            expanded_entities: entity_count,
            warnings: Vec::new(),
            blockers: Vec::new(),
        });
    }
    if manifest.state != cad_model::JwwCompatibilityState::EditableLossless {
        return Err(ExportError::Preservation(
            manifest
                .reason
                .clone()
                .unwrap_or_else(|| "project is not editable-lossless".to_owned()),
        ));
    }
    if manifest.edit_capability != cad_model::JwwEditCapability::MappedV600 {
        return Err(ExportError::Preservation(
            "this import has no verified record provenance; only exact original extraction is safe"
                .to_owned(),
        ));
    }
    let allowed_entity_path = format!("drawings/{drawing_name}/entities.ndjson");
    let incompatible = changed_source_paths
        .iter()
        .filter(|path| {
            path.as_str() != allowed_entity_path
                && !(path.starts_with("blocks/")
                    && (path.ends_with("/definition.toml") || path.ends_with("/entities.ndjson")))
        })
        .cloned()
        .collect::<Vec<_>>();
    if !incompatible.is_empty() {
        if allow_lossy {
            ensure_preservation_snapshot_current(project_path, &snapshot.source_manifest)?;
            return generated_preservation_fallback(
                project_path,
                drawing_name,
                output_path,
                overwrite,
                &snapshot.source_manifest,
                format!(
                    "preserved JWW header cannot represent changes to {}; generated v600 output was used",
                    incompatible.join(", ")
                ),
            );
        }
        return Ok(ExportReport {
            schema_version: "0.2".to_owned(),
            status: ExportStatus::Blocked,
            mode: ExportMode::PreservedEdited,
            output_path: output_path.display().to_string(),
            written_entities: 0,
            expanded_entities: 0,
            warnings: Vec::new(),
            blockers: vec![ExportIssue {
                code: "incompatible_preserved_source_change".to_owned(),
                message: format!(
                    "preserve export cannot apply changes to {}",
                    incompatible.join(", ")
                ),
                entity_id: None,
            }],
        });
    }
    ensure_preservation_snapshot_current(project_path, &snapshot.source_manifest)?;
    let project = cad_model::load_project(project_path)?;
    ensure_preservation_snapshot_current(project_path, &snapshot.source_manifest)?;
    let invalid_project = cad_check::check_loaded_project(&project)
        .diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.severity == cad_check::Severity::Error)
        .map(|diagnostic| ExportIssue {
            code: "invalid_project".to_owned(),
            message: format!("{}: {}", diagnostic.code, diagnostic.message),
            entity_id: diagnostic.entity_id,
        })
        .collect::<Vec<_>>();
    if !invalid_project.is_empty() {
        return Ok(preserved_blocked_report(output_path, invalid_project));
    }
    let provenance_bytes = snapshot.record_provenance_bytes.as_deref().ok_or_else(|| {
        ExportError::Preservation("verified record provenance is unavailable".to_owned())
    })?;
    let provenance = load_record_provenance(provenance_bytes)?;
    let (document, mut report) = build_preserved_document(
        &project,
        drawing_name,
        output_path,
        &provenance,
        allow_lossy,
    )?;
    if report.status == ExportStatus::Blocked {
        if allow_lossy {
            ensure_preservation_snapshot_current(project_path, &snapshot.source_manifest)?;
            return generated_preservation_fallback(
                project_path,
                drawing_name,
                output_path,
                overwrite,
                &snapshot.source_manifest,
                "edited records could not be merged with provenance; generated v600 output was used"
                    .to_owned(),
            );
        }
        return Ok(report);
    }
    let bytes =
        cad_jww_codec::write_document_preserving_header(&snapshot.original_bytes, &document)?;
    ensure_preservation_snapshot_current(project_path, &snapshot.source_manifest)?;
    publish(output_path, &bytes, overwrite)?;
    report.written_entities = document.records.len();
    report.expanded_entities = document.records.len();
    Ok(report)
}

fn generated_preservation_fallback(
    project_path: &Path,
    drawing_name: &str,
    output_path: &Path,
    overwrite: bool,
    expected_manifest: &[cad_model::SourceFileRevision],
    message: String,
) -> ExportResult<ExportReport> {
    generated_preservation_fallback_with_hook(
        project_path,
        drawing_name,
        output_path,
        overwrite,
        expected_manifest,
        message,
        || {},
    )
}

fn generated_preservation_fallback_with_hook(
    project_path: &Path,
    drawing_name: &str,
    output_path: &Path,
    overwrite: bool,
    expected_manifest: &[cad_model::SourceFileRevision],
    message: String,
    before_publish: impl FnOnce(),
) -> ExportResult<ExportReport> {
    let mut report = export_jww_file_with_expected_manifest(
        project_path,
        drawing_name,
        output_path,
        ExportOptions {
            allow_lossy: true,
            overwrite,
            strict_approximations: false,
        },
        expected_manifest,
        before_publish,
    )?;
    report.warnings.insert(
        0,
        ExportIssue {
            code: "preservation_fallback_generated".to_owned(),
            message,
            entity_id: None,
        },
    );
    Ok(report)
}

fn changed_jww_source_paths(
    manifest: &cad_model::JwwPreservationManifest,
    current: &[cad_model::SourceFileRevision],
) -> Vec<String> {
    let expected = manifest
        .source_revisions
        .iter()
        .map(|file| (file.relative_path.as_str(), (&file.revision, file.exists)))
        .collect::<BTreeMap<_, _>>();
    let actual = current
        .iter()
        .filter(|file| {
            !matches!(
                cad_model::classify_project_source_path(Path::new(&file.relative_path)),
                Some(cad_model::ProjectSourceKind::Comment)
            )
        })
        .map(|file| (file.relative_path.as_str(), (&file.revision, file.exists)))
        .collect::<BTreeMap<_, _>>();
    expected
        .keys()
        .chain(actual.keys())
        .filter(|path| expected.get(**path) != actual.get(**path))
        .map(|path| (*path).to_owned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn ensure_preservation_snapshot_current(
    project_path: &Path,
    expected: &[cad_model::SourceFileRevision],
) -> ExportResult<()> {
    if cad_model::source_manifest(project_path)? != expected {
        return Err(ExportError::RevisionConflict);
    }
    Ok(())
}

fn preserved_blocked_report(output_path: &Path, blockers: Vec<ExportIssue>) -> ExportReport {
    ExportReport {
        schema_version: "0.2".to_owned(),
        status: ExportStatus::Blocked,
        mode: ExportMode::PreservedEdited,
        output_path: output_path.display().to_string(),
        written_entities: 0,
        expanded_entities: 0,
        warnings: Vec::new(),
        blockers,
    }
}

#[derive(Default)]
struct RecordProvenance {
    entities: BTreeMap<(String, String), (serde_json::Value, Record)>,
    blocks: BTreeMap<String, BlockDefinition>,
}

fn load_record_provenance(bytes: &[u8]) -> ExportResult<RecordProvenance> {
    let text = std::str::from_utf8(bytes).map_err(|error| {
        ExportError::Preservation(format!("record provenance is not UTF-8: {error}"))
    })?;
    let mut provenance = RecordProvenance::default();
    let mut saw_header = false;
    for (index, line) in text.lines().enumerate() {
        let entry: cad_jww_codec::JwwProvenanceEntry =
            serde_json::from_str(line).map_err(|error| {
                ExportError::Preservation(format!(
                    "invalid record provenance at line {}: {error}",
                    index + 1
                ))
            })?;
        match entry {
            cad_jww_codec::JwwProvenanceEntry::Header { schema_version, .. } => {
                if saw_header || schema_version != cad_jww_codec::RECORD_PROVENANCE_SCHEMA_VERSION {
                    return Err(ExportError::Preservation(
                        "record provenance has an unsupported or duplicate header".to_owned(),
                    ));
                }
                saw_header = true;
            }
            cad_jww_codec::JwwProvenanceEntry::Entity {
                owner,
                entity_id,
                canonical_entity,
                mut record,
                record_float_bits,
                ..
            } => {
                let canonical_entity =
                    serde_json::from_str(&canonical_entity).map_err(|error| {
                        ExportError::Preservation(format!(
                            "invalid canonical entity in record provenance: {error}"
                        ))
                    })?;
                if !cad_jww_codec::restore_record_float_bits(&mut record, &record_float_bits) {
                    return Err(ExportError::Preservation(
                        "record provenance has an invalid float-bit payload".to_owned(),
                    ));
                }
                if provenance
                    .entities
                    .insert((owner, entity_id), (canonical_entity, record))
                    .is_some()
                {
                    return Err(ExportError::Preservation(
                        "record provenance contains a duplicate entity id".to_owned(),
                    ));
                }
            }
            cad_jww_codec::JwwProvenanceEntry::BlockDefinition {
                block_id,
                base,
                number,
                is_referenced,
                reserved,
                name,
            } => {
                if provenance
                    .blocks
                    .insert(
                        block_id,
                        BlockDefinition {
                            base,
                            number,
                            is_referenced,
                            reserved,
                            name,
                            records: Vec::new(),
                        },
                    )
                    .is_some()
                {
                    return Err(ExportError::Preservation(
                        "record provenance contains a duplicate block definition".to_owned(),
                    ));
                }
            }
        }
    }
    if !saw_header {
        return Err(ExportError::Preservation(
            "record provenance header is missing".to_owned(),
        ));
    }
    Ok(provenance)
}

fn build_preserved_document(
    project: &ProjectSource,
    drawing_name: &str,
    output_path: &Path,
    provenance: &RecordProvenance,
    allow_lossy: bool,
) -> ExportResult<(Document, ExportReport)> {
    let drawing = project
        .drawings
        .iter()
        .find(|drawing| drawing.name == drawing_name)
        .ok_or_else(|| ExportError::DrawingNotFound(drawing_name.to_owned()))?;
    let mut context = ExportContext::new(
        output_path,
        ExportOptions {
            allow_lossy: true,
            overwrite: false,
            strict_approximations: false,
        },
    );
    let (header, layer_slots) = build_header(project, drawing_name, &mut context);
    // The original header is retained byte-for-byte in this phase. Header
    // approximation warnings therefore do not describe the produced file.
    context.warnings.clear();
    context.blockers.clear();
    let current_sources = current_entity_sources(drawing_name, project)?;

    let mut block_numbers = provenance
        .blocks
        .iter()
        .map(|(id, definition)| (id.clone(), definition.number))
        .collect::<BTreeMap<_, _>>();
    let mut next_block_number = block_numbers
        .values()
        .copied()
        .max()
        .unwrap_or(0)
        .checked_add(1);
    for id in project.blocks.keys() {
        if block_numbers.contains_key(id) {
            continue;
        }
        let Some(number) = next_block_number else {
            context.blockers.push(ExportIssue {
                code: "block_number_exhausted".to_owned(),
                message: format!("no JWW block definition number remains for {id:?}"),
                entity_id: None,
            });
            continue;
        };
        block_numbers.insert(id.clone(), number);
        next_block_number = number.checked_add(1);
    }

    let records = convert_preserved_entities(
        project,
        drawing_name,
        drawing.entities.iter().map(|record| &record.entity),
        &layer_slots,
        &block_numbers,
        provenance,
        &current_sources,
        &mut context,
        allow_lossy,
    );
    let mut blocks = Vec::new();
    for (block_id, definition) in &project.blocks {
        let owner = format!("block:{block_id}");
        let block_records = convert_preserved_entities(
            project,
            &owner,
            definition.entities.iter().map(|record| &record.entity),
            &layer_slots,
            &block_numbers,
            provenance,
            &current_sources,
            &mut context,
            allow_lossy,
        );
        let original = provenance.blocks.get(block_id);
        if original.is_some() && definition.config.base_point != [0.0, 0.0] {
            let issue = ExportIssue {
                code: "unsupported_preserved_block_base_point".to_owned(),
                message: format!(
                    "preserved JWW block {block_id:?} may change name but not base_point"
                ),
                entity_id: None,
            };
            if allow_lossy {
                context.warnings.push(issue);
            } else {
                context.blockers.push(issue);
            }
        }
        let Some(number) = block_numbers.get(block_id).copied() else {
            continue;
        };
        blocks.push(BlockDefinition {
            base: original.map_or_else(Base::default, |value| value.base),
            number,
            is_referenced: original.map_or(1, |value| value.is_referenced),
            reserved: original.map_or(0, |value| value.reserved),
            name: cp932_text(&definition.config.name, None, &mut context),
            records: block_records,
        });
    }
    validate_preserved_block_archive(&records, &blocks, &mut context.blockers);
    let status = if context.blockers.is_empty() {
        ExportStatus::Exported
    } else {
        ExportStatus::Blocked
    };
    let report = ExportReport {
        schema_version: "0.2".to_owned(),
        status,
        mode: ExportMode::PreservedEdited,
        output_path: output_path.display().to_string(),
        written_entities: 0,
        expanded_entities: 0,
        warnings: context.warnings,
        blockers: context.blockers,
    };
    Ok((
        Document {
            header,
            records,
            blocks,
        },
        report,
    ))
}

#[allow(clippy::too_many_arguments)] // Keeps preservation inputs explicit at the fail-closed boundary.
fn convert_preserved_entities<'a>(
    project: &ProjectSource,
    owner: &str,
    entities: impl Iterator<Item = &'a Entity>,
    layer_slots: &BTreeMap<String, (u16, u16)>,
    block_numbers: &BTreeMap<String, u32>,
    provenance: &RecordProvenance,
    current_sources: &BTreeMap<(String, String), serde_json::Value>,
    context: &mut ExportContext<'_>,
    allow_lossy: bool,
) -> Vec<Record> {
    let mut output = Vec::new();
    for entity in entities {
        let key = (owner.to_owned(), entity.id().as_str().to_owned());
        let current = current_sources.get(&key);
        if let Some((canonical, original)) = provenance.entities.get(&key)
            && current == Some(canonical)
        {
            output.push(original.clone());
            continue;
        }
        if !allow_lossy
            && let Some((canonical, original @ Record::Dimension { .. })) =
                provenance.entities.get(&key)
        {
            let Some(current) = current else {
                context.blockers.push(ExportIssue {
                    code: "unsupported_dimension_edit".to_owned(),
                    message: "the current CAD source for the preserved dimension is unavailable"
                        .to_owned(),
                    entity_id: Some(entity.id().as_str().to_owned()),
                });
                continue;
            };
            if !dimension_differs_only_by_value(canonical, current) {
                context.blockers.push(ExportIssue {
                    code: "unsupported_dimension_edit".to_owned(),
                    message: "existing JWW dimensions may only change their displayed value"
                        .to_owned(),
                    entity_id: Some(entity.id().as_str().to_owned()),
                });
                continue;
            }
            match preserved_dimension_with_value(original, entity) {
                Ok(record) => output.push(record),
                Err(message) => context.blockers.push(ExportIssue {
                    code: "unsupported_dimension_edit".to_owned(),
                    message,
                    entity_id: Some(entity.id().as_str().to_owned()),
                }),
            }
            continue;
        }
        let before_records = output.len();
        let before_warnings = context.warnings.len();
        convert_entity(
            project,
            entity,
            layer_slots,
            block_numbers,
            &mut output,
            context,
        );
        let generated = output.len() - before_records;
        let new_warnings = context.warnings.split_off(before_warnings);
        let original = provenance.entities.get(&key).map(|(_, record)| record);
        let unsupported_warning = new_warnings.iter().find(|warning| {
            warning.code != "dimension_style_approximated"
                && !(warning.code == "font_substituted"
                    && original.is_some_and(|record| {
                        matches!(record, Record::Text { .. } | Record::Dimension { .. })
                    }))
        });
        if generated != 1 || unsupported_warning.is_some() {
            if allow_lossy && generated > 0 {
                context.warnings.extend(new_warnings);
                context.warnings.push(ExportIssue {
                    code: "preserved_entity_expanded".to_owned(),
                    message: format!(
                        "edited entity was replaced by {generated} generated JWW record(s)"
                    ),
                    entity_id: Some(entity.id().as_str().to_owned()),
                });
            } else {
                output.truncate(before_records);
                context.blockers.push(ExportIssue {
                    code: "preserved_entity_not_one_to_one".to_owned(),
                    message: unsupported_warning.map_or_else(
                        || {
                            "edited preserve export requires one JWW record per CAD entity"
                                .to_owned()
                        },
                        |warning| warning.message.clone(),
                    ),
                    entity_id: Some(entity.id().as_str().to_owned()),
                });
            }
            continue;
        }
        context.warnings.extend(new_warnings);
        if let Some(original) = original {
            let generated = output.pop().expect("one generated record");
            let generated_fallback = generated.clone();
            match merge_preserved_record(original, generated) {
                Some(record) => output.push(record),
                None => {
                    if allow_lossy {
                        output.push(generated_fallback);
                        context.warnings.push(ExportIssue {
                            code: "preserved_record_type_changed".to_owned(),
                            message:
                                "the edited entity was regenerated as a different JWW record class"
                                    .to_owned(),
                            entity_id: Some(entity.id().as_str().to_owned()),
                        });
                    } else {
                        output.truncate(before_records);
                        context.blockers.push(ExportIssue {
                            code: "preserved_record_type_changed".to_owned(),
                            message: "the edit changes the underlying JWW record class".to_owned(),
                            entity_id: Some(entity.id().as_str().to_owned()),
                        });
                    }
                }
            }
        }
    }
    output
}

fn dimension_differs_only_by_value(
    canonical: &serde_json::Value,
    current: &serde_json::Value,
) -> bool {
    let (Some(mut canonical), Some(mut current)) =
        (canonical.as_object().cloned(), current.as_object().cloned())
    else {
        return false;
    };
    if canonical.get("type").and_then(serde_json::Value::as_str) != Some("dimension")
        || current.get("type").and_then(serde_json::Value::as_str) != Some("dimension")
    {
        return false;
    }
    canonical.remove("value");
    current.remove("value");
    canonical == current
}

fn preserved_dimension_with_value(original: &Record, entity: &Entity) -> Result<Record, String> {
    let Entity::Dimension { p1, p2, value, .. } = entity else {
        return Err("the preserved dimension changed CAD entity type".to_owned());
    };
    let label = value.clone().unwrap_or_else(|| {
        cad_model::format_decimal_mm(((p2[0] - p1[0]).powi(2) + (p2[1] - p1[1]).powi(2)).sqrt())
    });
    if SHIFT_JIS.encode(&label).2 {
        return Err(format!(
            "dimension text {label:?} cannot be represented in CP932"
        ));
    }
    let mut record = original.clone();
    let Record::Dimension { text, .. } = &mut record else {
        return Err("the preserved record is not a JWW dimension".to_owned());
    };
    let Record::Text { value, .. } = text.as_mut() else {
        return Err("the preserved JWW dimension has no editable text record".to_owned());
    };
    *value = label;
    Ok(record)
}

fn validate_preserved_block_archive(
    records: &[Record],
    blocks: &[BlockDefinition],
    blockers: &mut Vec<ExportIssue>,
) {
    if records.len() > 65_534 {
        blockers.push(ExportIssue {
            code: "record_limit".to_owned(),
            message: "JWW supports at most 65534 top-level records".to_owned(),
            entity_id: None,
        });
    }
    let mut definitions = BTreeSet::new();
    for definition in blocks {
        if definition.records.len() > 65_534 {
            blockers.push(ExportIssue {
                code: "block_record_limit".to_owned(),
                message: format!(
                    "JWW block definition {} exceeds 65534 records",
                    definition.number
                ),
                entity_id: None,
            });
        }
        if !definitions.insert(definition.number) {
            blockers.push(ExportIssue {
                code: "duplicate_block_definition".to_owned(),
                message: format!(
                    "JWW block definition number {} is used more than once",
                    definition.number
                ),
                entity_id: None,
            });
        }
    }
    for record in records.iter().chain(
        blocks
            .iter()
            .flat_map(|definition| definition.records.iter()),
    ) {
        if let Record::Block { def_number, .. } = record
            && !definitions.contains(def_number)
        {
            blockers.push(ExportIssue {
                code: "missing_block_definition".to_owned(),
                message: format!(
                    "JWW block reference points to missing definition number {def_number}"
                ),
                entity_id: None,
            });
        }
    }
}

fn current_entity_sources(
    drawing_name: &str,
    project: &ProjectSource,
) -> ExportResult<BTreeMap<(String, String), serde_json::Value>> {
    let mut sources = BTreeMap::new();
    let drawing = project
        .drawings
        .iter()
        .find(|drawing| drawing.name == drawing_name)
        .ok_or_else(|| ExportError::DrawingNotFound(drawing_name.to_owned()))?;
    for record in &drawing.entities {
        sources.insert(
            (
                drawing_name.to_owned(),
                record.entity.id().as_str().to_owned(),
            ),
            serde_json::to_value(&record.entity).map_err(|error| {
                ExportError::Preservation(format!(
                    "failed to canonicalize CAD entity {:?}: {error}",
                    record.entity.id().as_str()
                ))
            })?,
        );
    }
    for (block_id, block) in &project.blocks {
        for record in &block.entities {
            sources.insert(
                (
                    format!("block:{block_id}"),
                    record.entity.id().as_str().to_owned(),
                ),
                serde_json::to_value(&record.entity).map_err(|error| {
                    ExportError::Preservation(format!(
                        "failed to canonicalize CAD entity {:?}: {error}",
                        record.entity.id().as_str()
                    ))
                })?,
            );
        }
    }
    Ok(sources)
}

fn merge_preserved_record(original: &Record, generated: Record) -> Option<Record> {
    match (original, generated) {
        (
            Record::Text {
                text_type,
                font: original_font,
                ..
            },
            Record::Text {
                base,
                start,
                end,
                size_x,
                size_y,
                spacing,
                angle_deg,
                font: _,
                value,
                ..
            },
        ) => Some(Record::Text {
            base,
            start,
            end,
            text_type: *text_type,
            size_x,
            size_y,
            spacing,
            angle_deg,
            font: original_font.clone(),
            value,
        }),
        (Record::Line { .. }, value @ Record::Line { .. })
        | (Record::Arc { .. }, value @ Record::Arc { .. })
        | (Record::Point { .. }, value @ Record::Point { .. })
        | (Record::Solid { .. }, value @ Record::Solid { .. })
        | (Record::Block { .. }, value @ Record::Block { .. }) => Some(value),
        _ => None,
    }
}

pub fn export_loaded_project(
    project: &ProjectSource,
    drawing_name: &str,
    output_path: &Path,
    options: ExportOptions,
) -> ExportResult<ExportReport> {
    let prepared = prepare_loaded_project(project, drawing_name, output_path, options)?;
    if let Some(bytes) = prepared.bytes {
        publish(output_path, &bytes, options.overwrite)?;
    }
    Ok(prepared.report)
}

struct PreparedJwwExport {
    report: ExportReport,
    bytes: Option<Vec<u8>>,
}

fn prepare_loaded_project(
    project: &ProjectSource,
    drawing_name: &str,
    output_path: &Path,
    options: ExportOptions,
) -> ExportResult<PreparedJwwExport> {
    let drawing = project
        .drawings
        .iter()
        .find(|drawing| drawing.name == drawing_name)
        .ok_or_else(|| ExportError::DrawingNotFound(drawing_name.to_owned()))?;
    let mut context = ExportContext::new(output_path, options);
    let check = cad_check::check_loaded_project(project);
    for diagnostic in check
        .diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.severity == cad_check::Severity::Error)
    {
        context.blockers.push(ExportIssue {
            code: "invalid_project".to_owned(),
            message: format!("{}: {}", diagnostic.code, diagnostic.message),
            entity_id: diagnostic.entity_id,
        });
    }
    let (header, layer_slots) = build_header(project, drawing_name, &mut context);
    let block_numbers = project
        .blocks
        .keys()
        .enumerate()
        .map(|(index, id)| (id.clone(), u32::try_from(index + 1).unwrap_or(u32::MAX)))
        .collect::<BTreeMap<_, _>>();
    let mut records = Vec::new();
    let mut written = 0;
    for record in &drawing.entities {
        let record_count = records.len();
        convert_entity(
            project,
            &record.entity,
            &layer_slots,
            &block_numbers,
            &mut records,
            &mut context,
        );
        if records.len() > record_count {
            written += 1;
        }
    }
    let mut block_definitions = Vec::new();
    for (block_id, definition) in &project.blocks {
        let mut block_records = Vec::new();
        for record in &definition.entities {
            convert_entity(
                project,
                &record.entity,
                &layer_slots,
                &block_numbers,
                &mut block_records,
                &mut context,
            );
        }
        block_definitions.push(BlockDefinition {
            base: Base::default(),
            number: block_numbers[block_id],
            is_referenced: 1,
            reserved: 0,
            name: cp932_text(&definition.config.name, None, &mut context),
            records: block_records,
        });
        if block_definitions
            .last()
            .is_some_and(|definition| definition.records.len() > 65_534)
        {
            context.fatal(
                None,
                "block_record_limit",
                format!("JWW block {block_id:?} exceeds 65534 records"),
            );
        }
    }
    if records.len() > 65_534 {
        context.fatal(
            None,
            "record_limit",
            "JWW supports at most 65534 output records",
        );
    }
    let expanded = records.len();
    if !context.blockers.is_empty() {
        return Ok(PreparedJwwExport {
            report: context.report(ExportStatus::Blocked, 0, expanded),
            bytes: None,
        });
    }
    let bytes = cad_jww_codec::write_document(&Document {
        header,
        records,
        blocks: block_definitions,
    })?;
    Ok(PreparedJwwExport {
        report: context.report(ExportStatus::Exported, written, expanded),
        bytes: Some(bytes),
    })
}

struct ExportContext<'a> {
    output_path: &'a Path,
    options: ExportOptions,
    warnings: Vec<ExportIssue>,
    blockers: Vec<ExportIssue>,
}

impl<'a> ExportContext<'a> {
    fn new(output_path: &'a Path, options: ExportOptions) -> Self {
        Self {
            output_path,
            options,
            warnings: Vec::new(),
            blockers: Vec::new(),
        }
    }

    fn block(&mut self, entity: Option<&Entity>, code: &str, message: impl Into<String>) {
        let issue = ExportIssue {
            code: code.to_owned(),
            message: message.into(),
            entity_id: entity.map(|entity| entity.id().as_str().to_owned()),
        };
        if self.options.allow_lossy {
            self.warnings.push(issue);
        } else {
            self.blockers.push(issue);
        }
    }

    fn approximate(&mut self, entity: Option<&Entity>, code: &str, message: impl Into<String>) {
        let issue = ExportIssue {
            code: code.to_owned(),
            message: message.into(),
            entity_id: entity.map(|entity| entity.id().as_str().to_owned()),
        };
        if self.options.strict_approximations {
            self.blockers.push(issue);
        } else {
            self.warnings.push(issue);
        }
    }

    fn fatal(&mut self, entity: Option<&Entity>, code: &str, message: impl Into<String>) {
        self.blockers.push(ExportIssue {
            code: code.to_owned(),
            message: message.into(),
            entity_id: entity.map(|entity| entity.id().as_str().to_owned()),
        });
    }

    fn report(self, status: ExportStatus, written: usize, expanded: usize) -> ExportReport {
        ExportReport {
            schema_version: "0.2".to_owned(),
            status,
            mode: if self.options.strict_approximations {
                ExportMode::GeneratedStrict
            } else {
                ExportMode::GeneratedBestEffort
            },
            output_path: self.output_path.display().to_string(),
            written_entities: written,
            expanded_entities: expanded,
            warnings: self.warnings,
            blockers: self.blockers,
        }
    }
}

fn build_header(
    project: &ProjectSource,
    drawing_name: &str,
    context: &mut ExportContext<'_>,
) -> (Header, BTreeMap<String, (u16, u16)>) {
    let drawing = project
        .drawings
        .iter()
        .find(|drawing| drawing.name == drawing_name)
        .expect("drawing checked");
    let Some(layout) = drawing.layouts.active() else {
        context.block(
            None,
            "invalid_active_layout",
            "drawing has no active layout",
        );
        return (Header::default(), BTreeMap::new());
    };
    let layout_scale = parse_layout_scale(&layout.scale, context);
    if matches!(layout.orientation, cad_model::SheetOrientation::Landscape) {
        context.approximate(
            None,
            "layout_orientation_approximated",
            "JWW version 600 does not carry an independent layout orientation flag",
        );
    }
    let mut header = Header {
        memo: cp932_text(&project.project.name, None, context),
        paper_size: paper_code(&layout.paper, context),
        ..Header::default()
    };
    for (id, color) in &project.styles.colors {
        let explicit_index = id
            .strip_prefix("jww_color_")
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|index| *index < 10);
        let mapped_index = builtin_color_number(&color.rgb).map(usize::from);
        let Some(index) = explicit_index.or(mapped_index) else {
            continue;
        };
        let Some(screen_colorref) = rgb_hex_to_colorref(&color.rgb) else {
            context.block(
                None,
                "invalid_jww_header_color",
                format!("color {id:?} has invalid RGB value {:?}", color.rgb),
            );
            continue;
        };
        let Some(print_colorref) = color
            .print_rgb
            .as_deref()
            .map_or_else(|| Some(screen_colorref), rgb_hex_to_colorref)
        else {
            context.block(
                None,
                "invalid_jww_print_color",
                format!("color {id:?} has invalid print_rgb value"),
            );
            continue;
        };
        header.screen_pen_colors[index] = screen_colorref;
        header.print_pen_colors[index] = print_colorref;
        header.print_pen_widths[index] = (color.print_width.max(0.01) * 100.0)
            .round()
            .min(f64::from(u32::MAX)) as u32;
    }
    let mut groups = project.layers.groups.iter().collect::<Vec<_>>();
    groups.sort_by_key(|(id, group)| (group.order, (*id).clone()));
    if groups.len() > 16 {
        context.block(
            None,
            "layer_group_limit",
            "JWW supports at most 16 layer groups",
        );
        groups.truncate(16);
    }
    let mut group_slots = BTreeMap::<String, u16>::new();
    if groups.is_empty() {
        group_slots.insert("default".to_owned(), 0);
        header.layer_groups[0].name = "Default".to_owned();
    } else {
        for (slot, (id, group)) in groups.iter().enumerate() {
            group_slots.insert((*id).clone(), slot as u16);
            header.layer_groups[slot] = LayerGroup {
                name: cp932_text(&group.name, None, context),
                state: if group.visible {
                    if group.locked { 1 } else { 2 }
                } else {
                    0
                },
                write_layer: 0,
                scale: layout_scale.unwrap_or_else(|| group.scale_denominator.max(1.0)),
                protect: u32::from(group.locked),
                layers: std::array::from_fn(|_| Layer {
                    state: 0,
                    ..Layer::default()
                }),
            };
        }
    }
    let mut by_group = BTreeMap::<u16, Vec<(&String, &LayerDef)>>::new();
    for (id, layer) in &project.layers.layers {
        let slot = layer
            .group
            .as_ref()
            .and_then(|group| group_slots.get(group))
            .copied()
            .unwrap_or(0);
        by_group.entry(slot).or_default().push((id, layer));
    }
    let mut layer_slots = BTreeMap::new();
    for (group_slot, layers) in &mut by_group {
        layers.sort_by_key(|(id, layer)| (layer.order, (*id).clone()));
        if layers.len() > 16 {
            context.block(
                None,
                "layer_limit",
                format!("layer group {group_slot} contains more than 16 layers"),
            );
            layers.truncate(16);
        }
        for (layer_slot, (id, layer)) in layers.iter().enumerate() {
            let active = project.layers.active_layer.as_deref() == Some(id.as_str());
            let state = if !layer.visible {
                0
            } else if active {
                3
            } else if layer.locked {
                1
            } else {
                2
            };
            header.layer_groups[*group_slot as usize].layers[layer_slot] = Layer {
                name: cp932_text(&layer.name, None, context),
                state,
                protect: u32::from(layer.locked),
            };
            if active {
                header.write_layer_group = u32::from(*group_slot);
                header.layer_groups[*group_slot as usize].write_layer = layer_slot as u32;
            }
            layer_slots.insert((*id).clone(), (*group_slot, layer_slot as u16));
        }
    }
    (header, layer_slots)
}

fn convert_entity(
    project: &ProjectSource,
    entity: &Entity,
    layer_slots: &BTreeMap<String, (u16, u16)>,
    block_numbers: &BTreeMap<String, u32>,
    records: &mut Vec<Record>,
    context: &mut ExportContext<'_>,
) {
    let Some(&(group, layer)) = layer_slots.get(entity.layer()) else {
        context.block(
            Some(entity),
            "undefined_layer_slot",
            "entity layer is not exportable to JWW",
        );
        return;
    };
    let base = match entity_base(project, entity, group, layer, context) {
        Some(base) => base,
        None => return,
    };
    match entity {
        Entity::Line { p1, p2, .. } => records.push(Record::Line {
            base,
            p1: *p1,
            p2: *p2,
        }),
        Entity::Polyline { points, closed, .. } => {
            for pair in points.windows(2) {
                records.push(Record::Line {
                    base,
                    p1: pair[0],
                    p2: pair[1],
                });
            }
            if *closed && points.len() > 2 {
                records.push(Record::Line {
                    base,
                    p1: *points.last().expect("nonempty"),
                    p2: points[0],
                });
            }
        }
        Entity::Arc {
            center,
            radius,
            start_deg,
            end_deg,
            ..
        } => records.push(Record::Arc {
            base,
            center: *center,
            radius: *radius,
            start_rad: start_deg.to_radians(),
            sweep_rad: (end_deg - start_deg).to_radians(),
            tilt_rad: 0.0,
            flatness: 1.0,
            full: false,
        }),
        Entity::Circle { center, radius, .. } => records.push(Record::Arc {
            base,
            center: *center,
            radius: *radius,
            start_rad: 0.0,
            sweep_rad: std::f64::consts::TAU,
            tilt_rad: 0.0,
            flatness: 1.0,
            full: true,
        }),
        Entity::Ellipse {
            center,
            radius_x,
            radius_y,
            rotation_deg,
            start_deg,
            end_deg,
            ..
        } => records.push(Record::Arc {
            base,
            center: *center,
            radius: *radius_x,
            start_rad: start_deg.to_radians(),
            sweep_rad: (end_deg - start_deg).to_radians(),
            tilt_rad: rotation_deg.to_radians(),
            flatness: radius_y / radius_x,
            full: (end_deg - start_deg).abs() >= 360.0 - 1e-9,
        }),
        Entity::Text {
            style,
            at,
            rotation_deg,
            mirror_y,
            value,
            ..
        } => {
            if *mirror_y {
                context.approximate(
                    Some(entity),
                    "mirrored_text",
                    "JWW export cannot preserve mirrored text",
                );
                if context.options.strict_approximations {
                    return;
                }
            }
            let Some(style) = project.styles.text_styles.get(style) else {
                context.block(
                    Some(entity),
                    "undefined_text_style",
                    "text style is missing",
                );
                return;
            };
            records.push(text_record(
                base,
                *at,
                *rotation_deg,
                value,
                style,
                context,
                Some(entity),
            ));
        }
        Entity::Dimension {
            style,
            p1,
            p2,
            offset,
            text_rotation_deg,
            text_mirror_y,
            value,
            ..
        } => {
            if *text_mirror_y {
                context.approximate(
                    Some(entity),
                    "mirrored_text",
                    "JWW export cannot preserve mirrored dimension text",
                );
                if context.options.strict_approximations {
                    return;
                }
            }
            let Some(dimension_style) = project.styles.dimension_styles.get(style) else {
                context.block(
                    Some(entity),
                    "undefined_dimension_style",
                    "dimension style is missing",
                );
                return;
            };
            let Some(text_style) = project.styles.text_styles.get(&dimension_style.text_style)
            else {
                context.block(
                    Some(entity),
                    "undefined_text_style",
                    "dimension text style is missing",
                );
                return;
            };
            context.approximate(
                Some(entity),
                "dimension_style_approximated",
                "JWW export cannot preserve dimension arrow, extension gap, precision, and unit semantics",
            );
            if context.options.strict_approximations {
                return;
            }
            let Some((d1, d2)) = cad_model::dimension_offset_segment(*p1, *p2, *offset) else {
                context.block(
                    Some(entity),
                    "invalid_dimension",
                    "dimension has invalid geometry",
                );
                return;
            };
            let label = value.clone().unwrap_or_else(|| {
                cad_model::format_decimal_mm(
                    ((p2[0] - p1[0]).powi(2) + (p2[1] - p1[1]).powi(2)).sqrt(),
                )
            });
            let text = text_record(
                base,
                [(d1[0] + d2[0]) / 2.0, (d1[1] + d2[1]) / 2.0],
                *text_rotation_deg,
                &label,
                text_style,
                context,
                Some(entity),
            );
            records.push(Record::Dimension {
                base,
                line: Box::new(Record::Line {
                    base,
                    p1: d1,
                    p2: d2,
                }),
                text: Box::new(text),
                sxf_mode: 0,
                aux_lines: std::array::from_fn(|_| Box::new(zero_line_record())),
                aux_points: std::array::from_fn(|_| Box::new(zero_point_record())),
            });
        }
        Entity::Point {
            at,
            temporary,
            marker_code,
            rotation_deg,
            scale,
            ..
        } => records.push(Record::Point {
            base,
            at: *at,
            temporary: *temporary,
            marker: marker_code.map(|code| (code, rotation_deg.to_radians(), *scale)),
        }),
        Entity::Solid { points, fill, .. } => {
            if !(3..=4).contains(&points.len()) {
                context.block(
                    Some(entity),
                    "solid_point_count",
                    "JWW solid requires three or four points",
                );
                return;
            }
            let Some((base, color)) = fill_base(project, fill, base) else {
                context.block(
                    Some(entity),
                    "undefined_fill",
                    "solid fill color is invalid",
                );
                return;
            };
            let p4 = if points.len() == 3 {
                points[2]
            } else {
                points[3]
            };
            records.push(Record::Solid {
                base,
                values: [
                    points[0][0],
                    points[0][1],
                    p4[0],
                    p4[1],
                    points[1][0],
                    points[1][1],
                    points[2][0],
                    points[2][1],
                ],
                color,
            });
        }
        Entity::CurveSolid {
            center,
            radius,
            flatness,
            rotation_deg,
            start_deg,
            end_deg,
            solid_param,
            encoding_code,
            fill,
            ..
        } => {
            let Some((mut base, color)) = fill_base(project, fill, base) else {
                context.block(
                    Some(entity),
                    "undefined_fill",
                    "curve solid fill color is invalid",
                );
                return;
            };
            base.pen_style = u8::try_from((*encoding_code).max(101)).unwrap_or(u8::MAX);
            records.push(Record::Solid {
                base,
                values: [
                    center[0],
                    center[1],
                    *radius,
                    *flatness,
                    rotation_deg.to_radians(),
                    start_deg.to_radians(),
                    (end_deg - start_deg).to_radians(),
                    *solid_param,
                ],
                color,
            });
        }
        Entity::BlockRef {
            block,
            at,
            rotation_deg,
            scale,
            ..
        } => {
            let Some(def_number) = block_numbers.get(block).copied() else {
                context.block(
                    Some(entity),
                    "unsupported_block_ref",
                    format!("block definition {block:?} is not available"),
                );
                return;
            };
            let base_point = project
                .blocks
                .get(block)
                .map(|definition| definition.config.base_point)
                .unwrap_or([0.0, 0.0]);
            let angle = rotation_deg.to_radians();
            let (sin, cos) = angle.sin_cos();
            let base_offset = [
                scale * (base_point[0] * cos - base_point[1] * sin),
                scale * (base_point[0] * sin + base_point[1] * cos),
            ];
            records.push(Record::Block {
                base,
                ref_x: at[0] - base_offset[0],
                ref_y: at[1] - base_offset[1],
                scale_x: *scale,
                scale_y: *scale,
                rotation: rotation_deg.to_radians(),
                def_number,
            });
        }
        Entity::Hatch {
            loops,
            pattern,
            angle_deg,
            scale,
            fill,
            ..
        } => {
            if loops.is_empty() {
                context.block(
                    Some(entity),
                    "unsupported_hatch_loop",
                    "JWW export requires at least one hatch loop",
                );
                return;
            }
            if loops.len() > 1 {
                context.approximate(
                    Some(entity),
                    "hatch_loops_approximated",
                    "multiple hatch loops are emitted as independent solid polygons",
                );
                if context.options.strict_approximations {
                    return;
                }
            }
            let Some((fill_base, color)) = fill
                .as_deref()
                .and_then(|fill| fill_base(project, fill, base))
            else {
                context.block(
                    Some(entity),
                    "undefined_fill",
                    "hatch fill color is missing or invalid",
                );
                return;
            };
            context.approximate(
                Some(entity),
                "hatch_pattern_approximated",
                format!(
                    "hatch pattern {pattern:?}, angle {angle_deg}, and scale {scale} are emitted as solid polygon fill"
                ),
            );
            for loop_points in loops {
                if loop_points.len() < 3
                    || loop_points
                        .iter()
                        .any(|point| !point[0].is_finite() || !point[1].is_finite())
                {
                    context.block(
                        Some(entity),
                        "unsupported_hatch_geometry",
                        "hatch loop must contain at least three finite points",
                    );
                    continue;
                }
                for pair in loop_points[1..].windows(2) {
                    push_solid_record(
                        records,
                        fill_base,
                        [loop_points[0], pair[0], pair[1]],
                        color,
                    );
                }
            }
        }
    }
}

fn text_record(
    base: Base,
    at: [f64; 2],
    rotation_deg: f64,
    value: &str,
    style: &TextStyleDef,
    context: &mut ExportContext<'_>,
    entity: Option<&Entity>,
) -> Record {
    let value = cp932_text(value, entity, context);
    if style.font_family != "MS Gothic" {
        context.approximate(
            entity,
            "font_substituted",
            format!("font {:?} is substituted with MS Gothic", style.font_family),
        );
    }
    let angle = rotation_deg.to_radians();
    let count = value.chars().count();
    let length = count as f64 * style.width + count.saturating_sub(1) as f64 * style.spacing;
    let anchor_offset = match style.align {
        cad_model::TextAlign::Left => 0.0,
        cad_model::TextAlign::Center => length / 2.0,
        cad_model::TextAlign::Right => length,
    };
    let start = [
        at[0] - anchor_offset * angle.cos(),
        at[1] - anchor_offset * angle.sin(),
    ];
    Record::Text {
        base,
        start,
        end: [
            start[0] + length * angle.cos(),
            start[1] + length * angle.sin(),
        ],
        text_type: 1,
        size_x: style.width,
        size_y: style.height,
        spacing: style.spacing,
        angle_deg: rotation_deg,
        font: "MS Gothic".to_owned(),
        value,
    }
}

fn zero_line_record() -> Record {
    Record::Line {
        base: Base::default(),
        p1: [0.0; 2],
        p2: [0.0; 2],
    }
}

fn zero_point_record() -> Record {
    Record::Point {
        base: Base::default(),
        at: [0.0; 2],
        temporary: false,
        marker: None,
    }
}

fn entity_base(
    project: &ProjectSource,
    entity: &Entity,
    group: u16,
    layer: u16,
    context: &mut ExportContext<'_>,
) -> Option<Base> {
    let layer_def = project.layers.layers.get(entity.layer())?;
    let (color, line_type, line_width) = if let Some(pen_id) = entity.pen() {
        let Some(pen) = project.styles.pens.get(pen_id) else {
            context.block(
                Some(entity),
                "undefined_pen",
                format!("pen {pen_id:?} is missing"),
            );
            return None;
        };
        (&pen.color, &pen.line_type, pen.line_width)
    } else {
        (&layer_def.color, &layer_def.line_type, layer_def.line_width)
    };
    let pen_color = stroke_color_number(project, color, entity, context)?;
    let pen_style = line_type_number(line_type, entity, context)?;
    Some(Base {
        pen_style,
        pen_color,
        pen_width: (line_width.max(0.0) * 100.0)
            .round()
            .min(f64::from(u16::MAX)) as u16,
        layer,
        layer_group: group,
        ..Base::default()
    })
}

fn stroke_color_number(
    project: &ProjectSource,
    color_id: &str,
    entity: &Entity,
    context: &mut ExportContext<'_>,
) -> Option<u16> {
    if let Some(value) = color_id
        .strip_prefix("jww_color_")
        .and_then(|value| value.parse().ok())
    {
        return Some(value);
    }
    let Some(color) = project.styles.colors.get(color_id) else {
        context.block(
            Some(entity),
            "undefined_stroke_color",
            format!("color {color_id:?} is missing"),
        );
        return None;
    };
    let rgb = color.rgb.to_ascii_uppercase();
    if let Some(number) = builtin_color_number(&rgb) {
        return Some(number);
    }
    context.block(
        Some(entity),
        "unsupported_stroke_color",
        format!("color {rgb} is mapped to the nearest built-in JWW pen color"),
    );
    context
        .options
        .allow_lossy
        .then(|| nearest_builtin_color_number(&rgb))
}

fn builtin_color_number(rgb: &str) -> Option<u16> {
    match rgb.to_ascii_uppercase().as_str() {
        "#000000" => Some(1),
        "#FF0000" => Some(2),
        "#00AA00" => Some(3),
        "#0000FF" => Some(4),
        "#FFFF00" => Some(5),
        "#FF00FF" => Some(6),
        "#00FFFF" => Some(7),
        "#FFFFFF" => Some(8),
        _ => None,
    }
}

fn nearest_builtin_color_number(rgb: &str) -> u16 {
    let Some(value) = rgb_hex_to_colorref(rgb) else {
        return 1;
    };
    let source = [
        (value & 0xff) as i64,
        ((value >> 8) & 0xff) as i64,
        ((value >> 16) & 0xff) as i64,
    ];
    [
        (1, [0, 0, 0]),
        (2, [255, 0, 0]),
        (3, [0, 170, 0]),
        (4, [0, 0, 255]),
        (5, [255, 255, 0]),
        (6, [255, 0, 255]),
        (7, [0, 255, 255]),
        (8, [255, 255, 255]),
    ]
    .into_iter()
    .min_by_key(|(_, candidate)| {
        source
            .iter()
            .zip(candidate)
            .map(|(left, right)| (left - right).pow(2))
            .sum::<i64>()
    })
    .map_or(1, |(number, _)| number)
}

fn line_type_number(
    line_type: &str,
    entity: &Entity,
    context: &mut ExportContext<'_>,
) -> Option<u8> {
    if let Some(value) = line_type
        .strip_prefix("jww_line_")
        .and_then(|value| value.parse().ok())
    {
        return Some(value);
    }
    if line_type == "solid" {
        return Some(1);
    }
    context.block(
        Some(entity),
        "unsupported_line_type",
        format!("line type {line_type:?} has no JWW mapping"),
    );
    context.options.allow_lossy.then_some(1)
}

fn fill_base(project: &ProjectSource, fill: &str, mut base: Base) -> Option<(Base, Option<u32>)> {
    let colorref = rgb_hex_to_colorref(&project.styles.colors.get(fill)?.rgb)?;
    base.pen_color = 10;
    Some((base, Some(colorref)))
}

fn rgb_hex_to_colorref(value: &str) -> Option<u32> {
    let rgb = value.strip_prefix('#').unwrap_or(value);
    if rgb.len() != 6 || !rgb.is_ascii() {
        return None;
    }
    let red = u32::from_str_radix(&rgb[0..2], 16).ok()?;
    let green = u32::from_str_radix(&rgb[2..4], 16).ok()?;
    let blue = u32::from_str_radix(&rgb[4..6], 16).ok()?;
    Some(red | (green << 8) | (blue << 16))
}

fn paper_code(paper: &str, context: &mut ExportContext<'_>) -> u32 {
    match paper.to_ascii_uppercase().as_str() {
        "A0" => 0,
        "A1" => 1,
        "A2" => 2,
        "A3" => 3,
        "A4" => 4,
        _ => {
            context.block(
                None,
                "unsupported_paper",
                format!("paper {paper:?} is not supported by the exporter"),
            );
            3
        }
    }
}

fn parse_layout_scale(scale: &str, context: &mut ExportContext<'_>) -> Option<f64> {
    let scale = scale.trim();
    let value = scale
        .strip_prefix("1/")
        .and_then(|value| value.parse::<f64>().ok())
        .or_else(|| {
            scale
                .strip_prefix("1:")
                .and_then(|value| value.parse::<f64>().ok())
        })
        .or_else(|| scale.parse::<f64>().ok());
    match value {
        Some(value) if value.is_finite() && value > 0.0 => Some(value),
        _ => {
            context.block(
                None,
                "unsupported_layout_scale",
                format!("layout scale {scale:?} is not a positive numeric scale"),
            );
            None
        }
    }
}

fn push_solid_record(
    records: &mut Vec<Record>,
    base: Base,
    points: [[f64; 2]; 3],
    color: Option<u32>,
) {
    records.push(Record::Solid {
        base,
        values: [
            points[0][0],
            points[0][1],
            points[2][0],
            points[2][1],
            points[1][0],
            points[1][1],
            points[2][0],
            points[2][1],
        ],
        color,
    });
}

fn cp932_text(value: &str, entity: Option<&Entity>, context: &mut ExportContext<'_>) -> String {
    let (encoded, _, had_errors) = SHIFT_JIS.encode(value);
    if !had_errors {
        return value.to_owned();
    }
    context.approximate(
        entity,
        "unencodable_text",
        format!("text {value:?} cannot be represented in CP932"),
    );
    if !context.options.strict_approximations {
        let (decoded, _, _) = SHIFT_JIS.decode(&encoded);
        decoded.into_owned()
    } else {
        String::new()
    }
}

fn publish(output: &Path, bytes: &[u8], overwrite: bool) -> ExportResult<()> {
    let parent = output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| ExportError::Write {
        path: parent.to_path_buf(),
        source,
    })?;
    let staging = tempfile::Builder::new()
        .prefix(".cad-jww-export-")
        .tempfile_in(parent)
        .map_err(|source| ExportError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    fs::write(staging.path(), bytes).map_err(|source| ExportError::Write {
        path: staging.path().to_path_buf(),
        source,
    })?;
    if overwrite {
        fs::rename(staging.path(), output).map_err(|source| ExportError::Write {
            path: output.to_path_buf(),
            source,
        })?;
    } else {
        renameat_with(CWD, staging.path(), CWD, output, RenameFlags::NOREPLACE).map_err(
            |error| {
                if error == Errno::EXIST || error == Errno::NOTEMPTY {
                    ExportError::OutputExists(output.to_path_buf())
                } else {
                    ExportError::Write {
                        path: output.to_path_buf(),
                        source: std::io::Error::from_raw_os_error(error.raw_os_error()),
                    }
                }
            },
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_crate_name() {
        assert_eq!(crate_name(), "cad-export-jww");
    }

    #[test]
    fn converts_canonical_rgb_to_jww_colorref() {
        assert_eq!(rgb_hex_to_colorref("#123456"), Some(0x00563412));
        assert_eq!(rgb_hex_to_colorref("not-a-color"), None);
        assert_eq!(nearest_builtin_color_number("#F01010"), 2);
        assert_eq!(nearest_builtin_color_number("#FAFAFA"), 8);
        assert_eq!(nearest_builtin_color_number("invalid"), 1);
    }

    #[test]
    fn existing_dimension_compatibility_allows_only_value_changes() {
        let canonical = serde_json::json!({
            "schema_version": "0.2",
            "id": "ent_01JZ0000000000000000000006",
            "type": "dimension",
            "layer": "0-1",
            "pen": "pen_1",
            "style": "dim_100",
            "p1": [0.0, 0.0],
            "p2": [100.0, 0.0],
            "offset": 10.0,
            "text_rotation_deg": 0.0,
            "text_mirror_y": false,
            "value": "100"
        });
        let mut value_only = canonical.clone();
        value_only["value"] = serde_json::Value::Null;
        assert!(dimension_differs_only_by_value(&canonical, &value_only));

        for (field, changed) in [
            ("layer", serde_json::json!("0-2")),
            ("pen", serde_json::json!("pen_2")),
            ("style", serde_json::json!("dim_50")),
            ("p1", serde_json::json!([1.0, 0.0])),
            ("p2", serde_json::json!([101.0, 0.0])),
            ("offset", serde_json::json!(11.0)),
            ("text_rotation_deg", serde_json::json!(90.0)),
            ("text_mirror_y", serde_json::json!(true)),
        ] {
            let mut current = value_only.clone();
            current[field] = changed;
            assert!(
                !dimension_differs_only_by_value(&canonical, &current),
                "{field} must be blocked"
            );
        }
    }

    #[test]
    fn imported_fixture_round_trips_supported_entities() {
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/jww-fixtures/Test1.jww");
        let temp = tempfile::tempdir().expect("tempdir");
        let imported = temp.path().join("imported");
        let exported = temp.path().join("exported.jww");
        let reimported = temp.path().join("reimported");
        let source_report = cad_import_jww::import_jww_file(&fixture, &imported).expect("import");
        let report = export_jww_file(
            &imported,
            "test1",
            &exported,
            ExportOptions {
                allow_lossy: true,
                overwrite: false,
                strict_approximations: false,
            },
        )
        .expect("export");
        assert_eq!(report.status, ExportStatus::Exported);
        assert!(!report.warnings.is_empty());
        let source_document = cad_jww_codec::read_document(&fs::read(&fixture).expect("fixture"))
            .expect("fixture should decode");
        let exported_document =
            cad_jww_codec::read_document(&fs::read(&exported).expect("exported JWW"))
                .expect("exported JWW should decode");
        assert_eq!(
            exported_document.header.screen_pen_colors, source_document.header.screen_pen_colors,
            "import/export must preserve the JWW display color table"
        );
        assert_eq!(
            exported_document.header.print_pen_colors, source_document.header.print_pen_colors,
            "import/export must preserve the JWW print color table"
        );
        assert_eq!(
            exported_document.header.print_pen_widths, source_document.header.print_pen_widths,
            "import/export must preserve the JWW print width table"
        );
        let round_trip = cad_import_jww::import_jww_file(&exported, &reimported).expect("reimport");
        assert_eq!(
            round_trip.supported_entities,
            source_report.supported_entities
        );
        assert_eq!(report.expanded_entities, source_report.supported_entities);
    }

    #[test]
    fn unchanged_import_preserves_the_original_jww_byte_for_byte() {
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/jww-fixtures/Test1.jww");
        let temp = tempfile::tempdir().expect("tempdir");
        let imported = temp.path().join("imported");
        let exported = temp.path().join("preserved.jww");
        cad_import_jww::import_jww_file(&fixture, &imported).expect("import");

        let report = export_jww_file_preserving(&imported, "test1", &exported, false)
            .expect("preserve export");

        assert_eq!(report.status, ExportStatus::Exported);
        assert_eq!(report.mode, ExportMode::PreservedExact);
        let edited = cad_jww_codec::read_document(&fs::read(exported).expect("preserved output"))
            .expect("exported document");
        let original = cad_jww_codec::read_document(&fs::read(fixture).expect("fixture"))
            .expect("source document");
        assert_eq!(edited.header, original.header);
        assert_eq!(edited.entities.len(), original.entities.len());
        for (index, (edited, original)) in
            edited.entities.iter().zip(&original.entities).enumerate()
        {
            assert_eq!(edited, original, "record {index}");
        }
        assert_eq!(edited.block_defs, original.block_defs);
    }

    #[test]
    fn verified_snapshot_rejects_tampered_jww_bytes() {
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/jww-fixtures/Test1.jww");
        let temp = tempfile::tempdir().expect("tempdir");
        let imported = temp.path().join("imported");
        cad_import_jww::import_jww_file(&fixture, &imported).expect("import");
        fs::write(
            imported.join(cad_model::JWW_ORIGINAL_RELATIVE_PATH),
            b"replaced original",
        )
        .expect("tamper original");
        assert!(matches!(
            cad_model::verified_jww_preservation_snapshot(&imported),
            Err(cad_model::ModelError::JwwPreservation { .. })
        ));

        let imported = temp.path().join("records-tampered");
        cad_import_jww::import_jww_file(&fixture, &imported).expect("second import");
        fs::write(
            imported.join(cad_model::JWW_RECORDS_RELATIVE_PATH),
            b"replaced provenance",
        )
        .expect("tamper provenance");
        assert!(matches!(
            cad_model::verified_jww_preservation_snapshot(&imported),
            Err(cad_model::ModelError::JwwPreservation { .. })
        ));
    }

    #[test]
    fn final_source_manifest_conflict_preserves_existing_output() {
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/jww-fixtures/Test1.jww");
        let temp = tempfile::tempdir().expect("tempdir");
        let imported = temp.path().join("imported");
        cad_import_jww::import_jww_file(&fixture, &imported).expect("import");
        let snapshot = cad_model::verified_jww_preservation_snapshot(&imported)
            .expect("snapshot")
            .expect("provenance");
        let output = temp.path().join("existing.jww");
        fs::write(&output, b"existing output").expect("existing output");
        let entities = imported.join("drawings/test1/entities.ndjson");
        let mut source = fs::read_to_string(&entities).expect("entities");
        source.push(' ');
        fs::write(&entities, source).expect("concurrent source edit");

        let error = ensure_preservation_snapshot_current(&imported, &snapshot.source_manifest)
            .expect_err("stale snapshot must fail");
        assert!(error.to_string().contains("revision_conflict"));
        assert_eq!(
            fs::read(output).expect("existing output"),
            b"existing output"
        );
    }

    #[test]
    fn generated_fallback_rechecks_sources_before_publish() {
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/jww-fixtures/Test1.jww");
        let temp = tempfile::tempdir().expect("tempdir");
        let imported = temp.path().join("imported");
        cad_import_jww::import_jww_file(&fixture, &imported).expect("import");
        let expected = cad_model::source_manifest(&imported).expect("source manifest");
        let output = temp.path().join("existing.jww");
        fs::write(&output, b"existing output").expect("existing output");
        let entities = imported.join("drawings/test1/entities.ndjson");

        let error = generated_preservation_fallback_with_hook(
            &imported,
            "test1",
            &output,
            true,
            &expected,
            "test fallback".to_owned(),
            || {
                let mut source = fs::read_to_string(&entities).expect("entities");
                source.push(' ');
                fs::write(&entities, source).expect("concurrent source edit");
            },
        )
        .expect_err("stale generated output must not publish");

        assert!(matches!(error, ExportError::RevisionConflict));
        assert_eq!(
            fs::read(&output).expect("existing output"),
            b"existing output"
        );
        assert!(fs::read_dir(temp.path()).expect("temp root").all(|entry| {
            !entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .starts_with(".cad-jww-export-")
        }));
    }

    #[test]
    fn edited_preserve_keeps_full_record_metadata_for_semantically_unchanged_entities() {
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/jww-fixtures/Test1.jww");
        let temp = tempfile::tempdir().expect("tempdir");
        let imported = temp.path().join("imported");
        let exported = temp.path().join("preserved.jww");
        cad_import_jww::import_jww_file(&fixture, &imported).expect("import");
        let entities = imported.join("drawings/test1/entities.ndjson");
        let source = fs::read_to_string(&entities).expect("entities");
        let source = source.replacen('\n', " \n", 1);
        fs::write(&entities, source).expect("mark source changed");
        let project = cad_model::load_project(&imported).expect("project");
        let snapshot = cad_model::verified_jww_preservation_snapshot(&imported)
            .expect("snapshot")
            .expect("JWW provenance");
        let provenance = load_record_provenance(
            snapshot
                .record_provenance_bytes
                .as_deref()
                .expect("record provenance"),
        )
        .expect("provenance");
        let current = current_entity_sources("test1", &project).expect("current source");
        let first_id = project.drawings[0].entities[0].entity.id().as_str();
        assert_eq!(
            current.get(&("test1".to_owned(), first_id.to_owned())),
            provenance
                .entities
                .get(&("test1".to_owned(), first_id.to_owned()))
                .map(|(canonical, _)| canonical)
        );

        let report = export_jww_file_preserving(&imported, "test1", &exported, false)
            .expect("preserve report");

        assert_eq!(report.status, ExportStatus::Exported);
        assert_eq!(report.mode, ExportMode::PreservedEdited);
        assert!(report.blockers.is_empty());
        let edited = cad_jww_codec::read_document(&fs::read(exported).expect("preserved output"))
            .expect("exported document");
        let original = cad_jww_codec::read_document(&fs::read(fixture).expect("fixture"))
            .expect("source document");
        assert_eq!(edited.header, original.header);
        assert_eq!(edited.entities.len(), original.entities.len());
        for (index, (edited, original)) in
            edited.entities.iter().zip(&original.entities).enumerate()
        {
            assert_eq!(edited, original, "record {index}");
        }
        assert_eq!(edited.block_defs, original.block_defs);
    }

    #[test]
    fn edited_preserve_updates_text_without_dropping_jww_text_type() {
        let fixture =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/jww-fixtures/Test1.jww");
        let temp = tempfile::tempdir().expect("tempdir");
        let imported = temp.path().join("imported");
        let exported = temp.path().join("preserved.jww");
        cad_import_jww::import_jww_file(&fixture, &imported).expect("import");
        let entities = imported.join("drawings/test1/entities.ndjson");
        let mut values = fs::read_to_string(&entities)
            .expect("entities")
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("entity"))
            .collect::<Vec<_>>();
        let text_index = values
            .iter()
            .position(|value| value["type"] == "text")
            .expect("fixture text");
        values[text_index]["value"] = serde_json::Value::String("変更済".to_owned());
        fs::write(
            &entities,
            values
                .iter()
                .map(|value| serde_json::to_string(value).expect("entity"))
                .collect::<Vec<_>>()
                .join("\n")
                + "\n",
        )
        .expect("edit entity");

        let report = export_jww_file_preserving(&imported, "test1", &exported, false)
            .expect("preserve export");
        assert_eq!(report.status, ExportStatus::Exported, "{report:?}");
        let original = cad_jww_codec::read_document(&fs::read(fixture).expect("fixture"))
            .expect("original document");
        let edited = cad_jww_codec::read_document(&fs::read(exported).expect("export"))
            .expect("edited document");
        let (
            cad_jww_codec::DecodedEntity::Text(original_text),
            cad_jww_codec::DecodedEntity::Text(edited_text),
        ) = (&original.entities[text_index], &edited.entities[text_index])
        else {
            panic!("mapped record must remain text");
        };

        assert_eq!(edited_text.content, "変更済");
        assert_eq!(edited_text.text_type, original_text.text_type);
    }

    #[test]
    fn edited_preserve_changes_only_dimension_text_value() {
        let temp = tempfile::tempdir().expect("tempdir");
        let input = temp.path().join("dimension.jww");
        let imported = temp.path().join("imported");
        let exported = temp.path().join("preserved.jww");
        let base = Base::default();
        let text = Record::Text {
            base,
            start: [50.0, 12.0],
            end: [60.0, 12.0],
            text_type: 42,
            size_x: 2.5,
            size_y: 2.5,
            spacing: 0.0,
            angle_deg: 0.0,
            font: "ＭＳ ゴシック".to_owned(),
            value: "100".to_owned(),
        };
        let dimension = Record::Dimension {
            base,
            line: Box::new(Record::Line {
                base,
                p1: [0.0, 10.0],
                p2: [100.0, 10.0],
            }),
            text: Box::new(text),
            sxf_mode: 7,
            aux_lines: [
                Box::new(Record::Line {
                    base,
                    p1: [0.0, 0.0],
                    p2: [0.0, 10.0],
                }),
                Box::new(Record::Line {
                    base,
                    p1: [100.0, 0.0],
                    p2: [100.0, 10.0],
                }),
            ],
            aux_points: std::array::from_fn(|index| {
                Box::new(Record::Point {
                    base,
                    at: [index as f64, 0.0],
                    temporary: false,
                    marker: None,
                })
            }),
        };
        fs::write(
            &input,
            cad_jww_codec::write_document(&Document {
                records: vec![dimension],
                ..Document::default()
            })
            .expect("dimension fixture"),
        )
        .expect("fixture");
        cad_import_jww::import_jww_file(&input, &imported).expect("import");
        let entities = imported.join("drawings/dimension/entities.ndjson");
        let mut value: serde_json::Value =
            serde_json::from_str(fs::read_to_string(&entities).expect("entity").trim())
                .expect("dimension entity");
        value["value"] = serde_json::Value::String("100 edited".to_owned());
        fs::write(
            &entities,
            format!("{}\n", serde_json::to_string(&value).expect("entity")),
        )
        .expect("edit dimension");

        let report = export_jww_file_preserving(&imported, "dimension", &exported, false)
            .expect("preserve export");
        let decoded = cad_jww_codec::read_document(&fs::read(exported).expect("export"))
            .expect("exported document");
        let cad_jww_codec::DecodedEntity::Dimension(dimension) = &decoded.entities[0] else {
            panic!("record must remain dimension");
        };

        assert_eq!(report.status, ExportStatus::Exported, "{report:?}");
        assert_eq!(dimension.sxf_mode, 7);
        assert_eq!(dimension.text.text_type, 42);
        assert_eq!(dimension.text.content, "100 edited");
        assert_eq!(dimension.line.start, [0.0, 10.0]);
        assert_eq!(dimension.line.end, [100.0, 10.0]);
        assert_eq!(dimension.text.start, [50.0, 12.0]);
        assert_eq!(dimension.text.end, [60.0, 12.0]);
        assert_eq!(dimension.aux_lines[0].start, [0.0, 0.0]);
        assert_eq!(dimension.aux_lines[0].end, [0.0, 10.0]);
        assert_eq!(dimension.aux_points[3].at, [3.0, 0.0]);

        value["value"] = serde_json::Value::Null;
        fs::write(
            &entities,
            format!("{}\n", serde_json::to_string(&value).expect("entity")),
        )
        .expect("use measured dimension value");
        let measured_output = temp.path().join("measured-dimension.jww");
        let measured = export_jww_file_preserving(&imported, "dimension", &measured_output, false)
            .expect("measured-value export");
        assert_eq!(measured.status, ExportStatus::Exported, "{measured:?}");
        let measured_document =
            cad_jww_codec::read_document(&fs::read(measured_output).expect("measured output"))
                .expect("measured document");
        let cad_jww_codec::DecodedEntity::Dimension(measured_dimension) =
            &measured_document.entities[0]
        else {
            panic!("record must remain dimension");
        };
        assert_eq!(measured_dimension.text.content, "100");
        assert_eq!(measured_dimension.line.start, [0.0, 10.0]);

        value["p1"] = serde_json::json!([10.0, 0.0]);
        fs::write(
            &entities,
            format!("{}\n", serde_json::to_string(&value).expect("entity")),
        )
        .expect("edit dimension geometry");
        let blocked_output = temp.path().join("blocked-dimension.jww");
        let blocked = export_jww_file_preserving(&imported, "dimension", &blocked_output, false)
            .expect("blocked report");
        assert_eq!(blocked.status, ExportStatus::Blocked, "{blocked:?}");
        assert!(
            blocked
                .blockers
                .iter()
                .any(|issue| issue.code == "unsupported_dimension_edit")
        );
        assert!(!blocked_output.exists());

        let best_effort_output = temp.path().join("best-effort-dimension.jww");
        let best_effort = export_jww_file_auto(
            &imported,
            "dimension",
            &best_effort_output,
            AutoExportOptions::default(),
        )
        .expect("best-effort dimension export");
        assert_eq!(
            best_effort.status,
            ExportStatus::Exported,
            "{best_effort:?}"
        );
        assert!(
            best_effort
                .warnings
                .iter()
                .any(|issue| issue.code == "dimension_style_approximated")
        );
        let best_effort_document = cad_jww_codec::read_document(
            &fs::read(best_effort_output).expect("best-effort output"),
        )
        .expect("best-effort document");
        let cad_jww_codec::DecodedEntity::Dimension(best_effort_dimension) =
            &best_effort_document.entities[0]
        else {
            panic!("best-effort record must remain a dimension");
        };
        assert_ne!(best_effort_dimension.line.start, [0.0, 10.0]);
    }

    #[test]
    fn best_effort_preserve_allows_one_entity_to_expand_to_multiple_records() {
        let temp = tempfile::tempdir().expect("tempdir");
        let input = temp.path().join("line.jww");
        let imported = temp.path().join("imported");
        let output = temp.path().join("expanded.jww");
        fs::write(
            &input,
            cad_jww_codec::write_document(&Document {
                records: vec![Record::Line {
                    base: Base::default(),
                    p1: [0.0, 0.0],
                    p2: [10.0, 0.0],
                }],
                ..Document::default()
            })
            .expect("line fixture"),
        )
        .expect("fixture");
        cad_import_jww::import_jww_file(&input, &imported).expect("import");
        let entities = imported.join("drawings/line/entities.ndjson");
        let mut value: serde_json::Value =
            serde_json::from_str(fs::read_to_string(&entities).expect("entity").trim())
                .expect("line entity");
        value["type"] = serde_json::Value::String("polyline".to_owned());
        value.as_object_mut().expect("entity object").remove("p1");
        value.as_object_mut().expect("entity object").remove("p2");
        value["points"] = serde_json::json!([[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]);
        value["closed"] = serde_json::Value::Bool(false);
        fs::write(
            &entities,
            format!("{}\n", serde_json::to_string(&value).expect("entity")),
        )
        .expect("edit line as polyline");

        let report = export_jww_file_auto(&imported, "line", &output, AutoExportOptions::default())
            .expect("best-effort expansion");

        assert_eq!(report.status, ExportStatus::Exported, "{report:?}");
        assert!(
            report
                .warnings
                .iter()
                .any(|issue| issue.code == "preserved_entity_expanded")
        );
        let document = cad_jww_codec::read_document(&fs::read(output).expect("output"))
            .expect("expanded document");
        assert_eq!(document.entities.len(), 2);
        assert!(
            document
                .entities
                .iter()
                .all(|entity| matches!(entity, cad_jww_codec::DecodedEntity::Line(_)))
        );
    }

    #[test]
    fn preserved_archive_rejects_missing_and_duplicate_block_definitions() {
        let base = Base::default();
        let reference = Record::Block {
            base,
            ref_x: 0.0,
            ref_y: 0.0,
            scale_x: 1.0,
            scale_y: 1.0,
            rotation: 0.0,
            def_number: 7,
        };
        let duplicate = BlockDefinition {
            base,
            number: 3,
            is_referenced: 1,
            reserved: 0,
            name: "duplicate".to_owned(),
            records: Vec::new(),
        };
        let mut blockers = Vec::new();
        validate_preserved_block_archive(
            &[reference],
            &[
                BlockDefinition {
                    name: "first".to_owned(),
                    ..duplicate.clone()
                },
                duplicate,
            ],
            &mut blockers,
        );
        assert!(
            blockers
                .iter()
                .any(|issue| issue.code == "missing_block_definition")
        );
        assert!(
            blockers
                .iter()
                .any(|issue| issue.code == "duplicate_block_definition")
        );
    }

    #[test]
    fn preserved_blocks_allow_name_only_and_reject_base_or_definition_loss() {
        let temp = tempfile::tempdir().expect("tempdir");
        let input = temp.path().join("blocks.jww");
        let imported = temp.path().join("imported");
        let base = Base::default();
        fs::write(
            &input,
            cad_jww_codec::write_document(&Document {
                records: vec![Record::Block {
                    base,
                    ref_x: 10.0,
                    ref_y: 20.0,
                    scale_x: 1.0,
                    scale_y: 1.0,
                    rotation: 0.0,
                    def_number: 7,
                }],
                blocks: vec![BlockDefinition {
                    base,
                    number: 7,
                    is_referenced: 1,
                    reserved: 9,
                    name: "original".to_owned(),
                    records: vec![Record::Line {
                        base,
                        p1: [0.0, 0.0],
                        p2: [10.0, 0.0],
                    }],
                }],
                ..Document::default()
            })
            .expect("block fixture"),
        )
        .expect("write fixture");
        cad_import_jww::import_jww_file(&input, &imported).expect("import");
        let block_dir = imported.join("blocks/jww_7");
        fs::write(
            block_dir.join("definition.toml"),
            "schema_version = \"0.2\"\nname = \"renamed\"\nbase_point = [0.0, 0.0]\n",
        )
        .expect("rename block");
        let renamed_output = temp.path().join("renamed.jww");
        let renamed = export_jww_file_preserving(&imported, "blocks", &renamed_output, false)
            .expect("name-only export");
        assert_eq!(renamed.status, ExportStatus::Exported, "{renamed:?}");

        fs::write(
            block_dir.join("definition.toml"),
            "schema_version = \"0.2\"\nname = \"renamed\"\nbase_point = [1.0, 0.0]\n",
        )
        .expect("move block base");
        let base_output = temp.path().join("base-change.jww");
        let base_report = export_jww_file_preserving(&imported, "blocks", &base_output, false)
            .expect("base-point report");
        assert_eq!(base_report.status, ExportStatus::Blocked, "{base_report:?}");
        assert!(
            base_report
                .blockers
                .iter()
                .any(|issue| { issue.code == "unsupported_preserved_block_base_point" })
        );
        assert!(!base_output.exists());

        fs::remove_dir_all(&block_dir).expect("delete block definition");
        let missing_output = temp.path().join("missing-block.jww");
        let missing = export_jww_file_preserving(&imported, "blocks", &missing_output, false)
            .expect("missing block report");
        assert_eq!(missing.status, ExportStatus::Blocked, "{missing:?}");
        assert!(
            missing
                .blockers
                .iter()
                .any(|issue| issue.code == "invalid_project")
        );
        assert!(!missing_output.exists());
    }

    #[test]
    fn strict_blocker_does_not_publish_output() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let mut project = cad_model::load_project(root).expect("example");
        let entity: Entity = serde_json::from_str(
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000009","type":"block_ref","layer":"0-1","block":"door","at":[0.0,0.0],"rotation_deg":0.0,"scale":1.0}"#,
        )
        .expect("block entity");
        project.drawings[0]
            .entities
            .push(cad_model::EntityRecord { line: 2, entity });
        let temp = tempfile::tempdir().expect("tempdir");
        let output = temp.path().join("blocked.jww");
        let report = export_loaded_project(&project, "plan_1f", &output, ExportOptions::default())
            .expect("blocked report");
        assert_eq!(report.status, ExportStatus::Blocked);
        assert!(!output.exists());
        assert!(
            report
                .blockers
                .iter()
                .any(|issue| issue.code == "unsupported_block_ref")
        );
    }

    #[test]
    fn missing_stroke_color_is_reported_and_not_counted_as_written() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let mut project = cad_model::load_project(root).expect("example");
        project
            .layers
            .layers
            .get_mut("0-1")
            .expect("fixture layer")
            .color = "missing-color".to_owned();
        let temp = tempfile::tempdir().expect("tempdir");

        let strict_output = temp.path().join("strict.jww");
        let strict = export_loaded_project(
            &project,
            "plan_1f",
            &strict_output,
            ExportOptions::default(),
        )
        .expect("strict report");
        assert_eq!(strict.status, ExportStatus::Blocked);
        assert_eq!(strict.written_entities, 0);
        assert!(!strict_output.exists());
        assert!(
            strict
                .blockers
                .iter()
                .any(|issue| issue.code == "undefined_stroke_color")
        );

        let lossy_output = temp.path().join("lossy.jww");
        let lossy = export_loaded_project(
            &project,
            "plan_1f",
            &lossy_output,
            ExportOptions {
                allow_lossy: true,
                overwrite: false,
                strict_approximations: false,
            },
        )
        .expect("lossy report");
        assert_eq!(lossy.status, ExportStatus::Blocked);
        assert_eq!(lossy.written_entities, 0);
        assert_eq!(lossy.expanded_entities, 0);
        assert!(!lossy_output.exists());
        assert!(
            lossy
                .blockers
                .iter()
                .any(|issue| issue.code == "invalid_project")
        );
        assert!(
            lossy
                .warnings
                .iter()
                .any(|issue| issue.code == "undefined_stroke_color")
        );
    }

    #[test]
    fn polygon_and_curve_solids_round_trip_through_version_600() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let mut project = cad_model::load_project(root).expect("example");
        for source in [
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000008","type":"solid","layer":"0-1","points":[[0.0,0.0],[100.0,0.0],[100.0,50.0],[0.0,50.0]],"fill":"jw_black"}"#,
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000007","type":"curve_solid","layer":"0-1","center":[200.0,200.0],"radius":100.0,"flatness":0.5,"rotation_deg":30.0,"start_deg":0.0,"end_deg":180.0,"solid_param":20.0,"encoding_code":1,"fill":"jw_black"}"#,
        ] {
            let entity: Entity = serde_json::from_str(source).expect("solid entity");
            project.drawings[0]
                .entities
                .push(cad_model::EntityRecord { line: 2, entity });
        }

        let temp = tempfile::tempdir().expect("tempdir");
        let output = temp.path().join("solid-roundtrip.jww");
        let reimported = temp.path().join("solid-roundtrip");
        let report = export_loaded_project(&project, "plan_1f", &output, ExportOptions::default())
            .expect("solid export");
        assert_eq!(report.status, ExportStatus::Exported);
        let decoded = cad_jww_codec::read_document(&fs::read(&output).expect("JWW bytes"))
            .expect("exported JWW should decode");
        assert_eq!(
            decoded.header.screen_pen_colors[1], 0,
            "canonical black mapped to pen 1 must also update the JWW header table"
        );

        cad_import_jww::import_jww_file(&output, &reimported).expect("solid reimport");
        let reimported_check = cad_check::check_project(&reimported);
        assert!(
            reimported_check.is_ok(),
            "solid round-trip should remain checker-valid: {reimported_check:?}"
        );
        let entities = fs::read_to_string(
            reimported
                .join("drawings")
                .join("solid-roundtrip")
                .join("entities.ndjson"),
        )
        .expect("reimported entities");
        assert!(entities.contains("\"type\":\"solid\""));
        assert!(entities.contains("\"type\":\"curve_solid\""));
    }

    #[test]
    fn simple_hatch_is_emitted_as_solid_polygons_with_a_warning() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let mut project = cad_model::load_project(root).expect("example");
        let entity: Entity = serde_json::from_str(
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000010","type":"hatch","layer":"0-1","loops":[[[0.0,0.0],[100.0,0.0],[100.0,100.0],[0.0,100.0]]],"pattern":"solid","angle_deg":30.0,"scale":2.0,"fill":"jw_black"}"#,
        )
        .expect("hatch entity");
        project.drawings[0]
            .entities
            .push(cad_model::EntityRecord { line: 2, entity });

        let temp = tempfile::tempdir().expect("tempdir");
        let output = temp.path().join("hatch.jww");
        let report = export_loaded_project(&project, "plan_1f", &output, ExportOptions::default())
            .expect("hatch export");
        assert_eq!(report.status, ExportStatus::Exported);
        assert!(
            report
                .warnings
                .iter()
                .any(|issue| issue.code == "hatch_pattern_approximated")
        );

        let decoded = cad_jww_codec::read_document(&fs::read(&output).expect("JWW bytes"))
            .expect("JWW should decode");
        assert!(
            decoded
                .entities
                .iter()
                .any(|entity| matches!(entity, cad_jww_codec::DecodedEntity::Solid(_)))
        );
    }

    #[test]
    fn auto_export_is_best_effort_by_default_and_strict_on_request() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let temp = tempfile::tempdir().expect("tempdir");
        let best_effort_output = temp.path().join("best-effort.jww");
        let strict_output = temp.path().join("strict.jww");

        let best_effort = export_jww_file_auto(
            &root,
            "plan_1f",
            &best_effort_output,
            AutoExportOptions::default(),
        )
        .expect("best-effort export");
        assert_eq!(best_effort.status, ExportStatus::Exported);
        assert!(best_effort_output.exists());
        assert!(
            best_effort
                .warnings
                .iter()
                .any(|issue| issue.code == "layout_orientation_approximated")
        );

        let strict = export_jww_file_auto(
            &root,
            "plan_1f",
            &strict_output,
            AutoExportOptions {
                strict: true,
                ..AutoExportOptions::default()
            },
        )
        .expect("strict report");
        assert_eq!(strict.status, ExportStatus::Blocked);
        assert!(!strict_output.exists());
        assert!(
            strict
                .blockers
                .iter()
                .any(|issue| issue.code == "layout_orientation_approximated")
        );
    }

    #[test]
    fn block_export_offsets_jww_reference_by_definition_base_point() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let mut project = cad_model::load_project(root).expect("example");
        let child: Entity = serde_json::from_str(
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000098","type":"line","layer":"0-1","p1":[10.0,20.0],"p2":[20.0,20.0]}"#,
        )
        .expect("block child");
        project.blocks.insert(
            "door_test".to_owned(),
            cad_model::BlockDefinition {
                id: "door_test".to_owned(),
                config: cad_model::BlockDefinitionConfig {
                    schema_version: cad_model::CURRENT_SCHEMA_VERSION.to_owned(),
                    name: "Door test".to_owned(),
                    base_point: [10.0, 20.0],
                },
                entities: vec![cad_model::EntityRecord {
                    line: 1,
                    entity: child,
                }],
            },
        );
        let reference: Entity = serde_json::from_str(
            r#"{"schema_version":"0.2","id":"ent_01JZ0000000000000000000099","type":"block_ref","layer":"0-1","block":"door_test","at":[100.0,200.0],"rotation_deg":90.0,"scale":2.0}"#,
        )
        .expect("block reference");
        project.drawings[0].entities.push(cad_model::EntityRecord {
            line: 2,
            entity: reference,
        });
        let temp = tempfile::tempdir().expect("tempdir");
        let output = temp.path().join("base-point.jww");

        let report = export_loaded_project(&project, "plan_1f", &output, ExportOptions::default())
            .expect("export");
        assert_eq!(report.status, ExportStatus::Exported);
        let decoded = cad_jww_codec::read_document(
            &fs::read(output).expect("exported JWW should be readable"),
        )
        .expect("exported JWW should decode");
        let block = decoded
            .entities
            .iter()
            .find_map(|entity| match entity {
                cad_jww_codec::DecodedEntity::Block(block) => Some(block),
                _ => None,
            })
            .expect("block reference should be exported");
        assert!((block.ref_x - 140.0).abs() < 1e-9);
        assert!((block.ref_y - 180.0).abs() < 1e-9);
    }
}
