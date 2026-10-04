//! Checked replacement of existing canonical files with one project Undo entry.
use super::*;
use cad_model::{ProjectSourceKind, SourceFileRevision};

#[derive(Debug, Clone)]
pub struct SourceReplacementRequest {
    pub drawing: String,
    pub expected_files: Vec<SourceFileRevision>,
    pub files: BTreeMap<String, Vec<u8>>,
}

/// No file creation/deletion, comments, or interop writes. All candidate files
/// are checked together; every publication compares the complete source set.
pub fn apply(root: &Path, request: &SourceReplacementRequest) -> EditResult<DrawingEditResult> {
    with_history_lock(|| apply_locked(root, request, false, || Ok(()), |_| Ok(())))
}

/// Clipboard publication may add complete new block definitions. Existing
/// blocks still use ordinary replacements; source deletion is never allowed.
pub fn apply_with_new_blocks(
    root: &Path,
    request: &SourceReplacementRequest,
) -> EditResult<DrawingEditResult> {
    with_history_lock(|| apply_locked(root, request, true, || Ok(()), |_| Ok(())))
}

/// Additional context checks run inside the history lock before and after writes.
/// A failed check rolls back our files while preserving concurrent external edits.
pub fn apply_with_guard(
    root: &Path,
    request: &SourceReplacementRequest,
    mut guard: impl FnMut() -> EditResult<()>,
) -> EditResult<DrawingEditResult> {
    with_history_lock(|| {
        guard()?;
        apply_locked(root, request, false, || Ok(()), |_| guard())
    })
}

fn manifest(root: &Path) -> EditResult<Vec<SourceFileRevision>> {
    cad_model::source_manifest(root).map_err(|error| EditError::InvalidEntity(error.to_string()))
}

fn apply_locked(
    root: &Path,
    request: &SourceReplacementRequest,
    allow_new_blocks: bool,
    mut before_publication: impl FnMut() -> EditResult<()>,
    mut after_publication: impl FnMut(usize) -> EditResult<()>,
) -> EditResult<DrawingEditResult> {
    if manifest(root)? != request.expected_files {
        return Err(EditError::RevisionConflict);
    }
    if let Some(state) = cad_model::jww_project_compatibility(root)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?
        && !(state.state == cad_model::JwwCompatibilityState::EditableLossless
            && state.original_verified
            && state.edit_capability == cad_model::JwwEditCapability::MappedV600)
    {
        return Err(EditError::InvalidEntity(
            "jww_read_only: no verified editable record mapping".into(),
        ));
    }
    let original = cad_model::load_project(root)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    if !checker_errors(&original).is_empty() {
        return Err(EditError::CheckFailed(
            "original project has CAD errors".into(),
        ));
    }
    if !original
        .drawings
        .iter()
        .any(|drawing| drawing.name == request.drawing)
    {
        return Err(EditError::DrawingNotFound(request.drawing.clone()));
    }
    let mut before = Vec::new();
    let mut after = Vec::new();
    for (relative, bytes) in &request.files {
        if !matches!(cad_model::classify_project_source_path(Path::new(relative)), Some(kind) if kind != ProjectSourceKind::Comment)
        {
            return Err(EditError::InvalidEntity(format!(
                "protected source path: {relative}"
            )));
        }
        let file = read_history_file(root, relative)?;
        let creating_block = allow_new_blocks
            && !file.exists
            && matches!(
                cad_model::classify_project_source_path(Path::new(relative)),
                Some(ProjectSourceKind::BlockDefinition | ProjectSourceKind::BlockEntities)
            );
        if creating_block {
            let parent = Path::new(relative).parent().expect("block parent");
            for leaf in ["definition.toml", "entities.ndjson"] {
                let other = parent.join(leaf).to_string_lossy().into_owned();
                if !request.files.contains_key(&other) || read_history_file(root, &other)?.exists {
                    return Err(EditError::InvalidEntity(
                        "new block requires two new definition/entities files".into(),
                    ));
                }
            }
        } else if !file.exists
            || !request
                .expected_files
                .iter()
                .any(|expected| expected.relative_path == *relative && expected.exists)
        {
            return Err(EditError::InvalidEntity(format!(
                "source replacement requires an existing file: {relative}"
            )));
        }
        if !file.exists || file.bytes != *bytes {
            after.push(HistoryFileInput {
                exists: true,
                bytes: bytes.clone(),
                ..file.clone()
            });
            before.push(file);
        }
    }
    if before.is_empty() {
        return Err(EditError::InvalidEntity(
            "candidate has no source changes".into(),
        ));
    }
    let candidate = tempfile::tempdir().map_err(|source| EditError::Write {
        path: root.to_owned(),
        source,
    })?;
    for file in &request.expected_files {
        let current = read_history_file(root, &file.relative_path)?;
        if !current.exists || revision(&current.bytes) != file.revision {
            return Err(EditError::RevisionConflict);
        }
        let path = candidate.path().join(&file.relative_path);
        fs::create_dir_all(path.parent().expect("canonical source parent")).map_err(|source| {
            EditError::Write {
                path: path.clone(),
                source,
            }
        })?;
        fs::write(
            &path,
            request
                .files
                .get(&file.relative_path)
                .unwrap_or(&current.bytes),
        )
        .map_err(|source| EditError::Write { path, source })?;
    }
    for file in &mut after {
        let path = candidate.path().join(&file.relative_path);
        fs::create_dir_all(path.parent().expect("canonical source parent")).map_err(|source| {
            EditError::Write {
                path: path.clone(),
                source,
            }
        })?;
        fs::write(&path, &file.bytes).map_err(|source| EditError::Write {
            path: path.clone(),
            source,
        })?;
        if file.permissions.is_none() {
            file.permissions = Some(capture_permissions(
                &fs::metadata(&path)
                    .map_err(|source| EditError::Read {
                        path: path.clone(),
                        source,
                    })?
                    .permissions(),
            ));
        }
    }
    let checked = cad_model::load_project(candidate.path())
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    let errors = checker_errors(&checked);
    if !errors.is_empty() {
        return Err(EditError::CheckFailed(
            errors.into_iter().collect::<Vec<_>>().join("; "),
        ));
    }
    let mut stage = stage_history_transaction(
        root,
        &request.drawing,
        "project",
        "source_replacements",
        &[],
        &before,
    )?;
    if let Err(error) = stage
        .prepare_after(&after)
        .and_then(|()| stage.prepare_commit_journal())
    {
        stage.abort();
        return Err(error);
    }
    let mut published = 0;
    let publication = (|| {
        before_publication()?;
        let mut expected = request.expected_files.clone();
        if manifest(root)? != expected {
            return Err(EditError::RevisionConflict);
        }
        for original in &before {
            let current = read_history_file(root, &original.relative_path)?;
            if current.exists != original.exists || current.bytes != original.bytes {
                return Err(EditError::RevisionConflict);
            }
        }
        for (index, (target, current)) in after.iter().zip(&before).enumerate() {
            publish_history_file(root, target, current)?;
            published += 1;
            if let Some(file) = expected
                .iter_mut()
                .find(|file| file.relative_path == target.relative_path)
            {
                file.revision = revision(&target.bytes);
            } else {
                expected.push(SourceFileRevision {
                    relative_path: target.relative_path.clone(),
                    revision: revision(&target.bytes),
                    exists: true,
                });
                expected.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
            }
            after_publication(index + 1)?;
            if manifest(root)? != expected {
                return Err(EditError::RevisionConflict);
            }
        }
        commit_history_stage(&mut stage)
    })();
    let history_id = match publication {
        Ok(id) => id,
        Err(error) => {
            let mut failures = Vec::new();
            for index in (0..published).rev() {
                if let Err(failure) = publish_history_file(root, &before[index], &after[index]) {
                    failures.push(failure.to_string());
                }
            }
            if failures.is_empty() {
                stage.abort();
                return Err(error);
            }
            let id = stage.history_id().to_owned();
            stage.preserve_for_recovery();
            return Err(EditError::HistoryUnavailable(format!(
                "{error}; rollback failed: {}; recovery history {id}",
                failures.join("; ")
            )));
        }
    };
    let drawing_path = format!("drawings/{}/entities.ndjson", request.drawing);
    let drawing_revision = after
        .iter()
        .find(|file| file.relative_path == drawing_path)
        .map(|file| revision(&file.bytes))
        .unwrap_or_else(|| {
            request
                .expected_files
                .iter()
                .find(|file| file.relative_path == drawing_path)
                .expect("validated drawing source")
                .revision
                .clone()
        });
    Ok(DrawingEditResult {
        drawing: request.drawing.clone(),
        revision: drawing_revision,
        entity_id: None,
        entity_ids: vec![],
        operation: "source_replacements".into(),
        history_id: Some(history_id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let created = create_project(&ProjectTemplateRequest {
            parent_dir: temp.path().display().to_string(),
            folder_name: "project".into(),
            project_name: "bulk".into(),
            drawing: "plan".into(),
            paper: "A4".into(),
            orientation: cad_model::SheetOrientation::Landscape,
            scale_denominator: 50,
        })
        .unwrap();
        (temp, created.project_path.into())
    }
    fn request(root: &Path) -> SourceReplacementRequest {
        let mut files = BTreeMap::new();
        let source = cad_model::load_project(root).unwrap();
        let mut project = source.project;
        project.name = "Merged project".into();
        files.insert(
            "cad.project.toml".into(),
            toml::to_string_pretty(&project).unwrap().into_bytes(),
        );
        files.insert("drawings/plan/entities.ndjson".into(), b"{\"schema_version\":\"0.3\",\"id\":\"ent_01JZ0000000000000000000000\",\"type\":\"line\",\"layer\":\"0-1\",\"p1\":[0,0],\"p2\":[100,0]}\n".to_vec());
        SourceReplacementRequest {
            drawing: "plan".into(),
            expected_files: manifest(root).unwrap(),
            files,
        }
    }
    #[test]
    fn checks_all_files_and_undo_redo_restores_exact_bytes() {
        let (_temp, root) = project();
        let request = request(&root);
        let before: BTreeMap<_, _> = request
            .expected_files
            .iter()
            .map(|file| {
                (
                    file.relative_path.clone(),
                    fs::read(root.join(&file.relative_path)).unwrap(),
                )
            })
            .collect();
        let result = apply(&root, &request).unwrap();
        assert!(result.history_id.is_some());
        assert!(cad_check::check_project(&root).is_ok());
        assert!(matches!(
            apply(&root, &request),
            Err(EditError::RevisionConflict)
        ));
        let history = list_drawing_history(&root, "plan").unwrap();
        undo_drawing_edit(
            &root,
            &DrawingHistoryRequest {
                drawing: "plan".into(),
                expected_files: history.current_files,
            },
        )
        .unwrap();
        for (path, bytes) in &before {
            assert_eq!(fs::read(root.join(path)).unwrap(), *bytes);
        }
        let history = list_drawing_history(&root, "plan").unwrap();
        redo_drawing_edit(
            &root,
            &DrawingHistoryRequest {
                drawing: "plan".into(),
                expected_files: history.current_files,
            },
        )
        .unwrap();
        for (path, bytes) in &request.files {
            assert_eq!(fs::read(root.join(path)).unwrap(), *bytes);
        }
    }
    #[test]
    fn rejects_protected_missing_and_invalid_files_without_source_changes() {
        let (_temp, root) = project();
        let original = manifest(&root).unwrap();
        for path in [
            "comments/plan.ndjson",
            "interop/jww/records.ndjson",
            "../other.toml",
            "rules/new.toml",
        ] {
            let mut request = request(&root);
            request.files.insert(path.into(), vec![]);
            assert!(apply(&root, &request).is_err(), "{path}");
            assert_eq!(manifest(&root).unwrap(), original);
        }
        let mut request = request(&root);
        request
            .files
            .get_mut("drawings/plan/entities.ndjson")
            .unwrap()
            .extend_from_slice(b"invalid\n");
        assert!(apply(&root, &request).is_err());
        assert_eq!(manifest(&root).unwrap(), original);
    }
    #[test]
    fn incomplete_block_creation_rolls_back_both_files_and_preserves_external_data() {
        let (_temp, root) = project();
        let mut request = request(&root);
        let before = manifest(&root).unwrap();
        request.files.insert(
            "blocks/new/definition.toml".into(),
            b"schema_version=\"0.3\"\nname=\"New\"\n".to_vec(),
        );
        assert!(apply_with_new_blocks(&root, &request).is_err());
        assert_eq!(manifest(&root).unwrap(), before);
        request
            .files
            .insert("blocks/new/entities.ndjson".into(), vec![]);
        let error = with_history_lock(|| {
            apply_locked(
                &root,
                &request,
                true,
                || Ok(()),
                |count| {
                    if count == 2 {
                        Err(EditError::InvalidEntity(
                            "simulated publication failure".into(),
                        ))
                    } else {
                        Ok(())
                    }
                },
            )
        })
        .unwrap_err();
        assert!(error.to_string().contains("simulated"));
        assert_eq!(manifest(&root).unwrap(), before);
        assert!(cad_check::check_project(&root).is_ok());
        assert!(list_drawing_history(&root, "plan").unwrap().undo.is_empty());
    }

    #[test]
    fn concurrent_unrelated_change_rolls_back_only_our_publication() {
        let (_temp, root) = project();
        let request = request(&root);
        let before_project = fs::read(root.join("cad.project.toml")).unwrap();
        let before_drawing = fs::read(root.join("drawings/plan/entities.ndjson")).unwrap();
        let external = fs::read(root.join("rules/layers.toml"))
            .unwrap()
            .into_iter()
            .chain(b"\n# external edit\n".iter().copied())
            .collect::<Vec<_>>();
        let result = with_history_lock(|| {
            apply_locked(
                &root,
                &request,
                false,
                || Ok(()),
                |count| {
                    if count == 1 {
                        fs::write(root.join("rules/layers.toml"), &external).unwrap();
                    }
                    Ok(())
                },
            )
        });
        assert!(matches!(result, Err(EditError::RevisionConflict)));
        assert_eq!(
            fs::read(root.join("cad.project.toml")).unwrap(),
            before_project
        );
        assert_eq!(
            fs::read(root.join("drawings/plan/entities.ndjson")).unwrap(),
            before_drawing
        );
        assert_eq!(fs::read(root.join("rules/layers.toml")).unwrap(), external);
        assert!(list_drawing_history(&root, "plan").unwrap().undo.is_empty());
    }
}
