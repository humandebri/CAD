//! Block source mutations share the drawing history journal and source publication machinery.
use super::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockEditRequest {
    pub drawing: String,
    pub expected_revision: String,
    pub operation: BlockEditOperation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum BlockEditOperation {
    Create {
        block: String,
        name: String,
        base_point: Point,
        entity_ids: Vec<String>,
        replace_originals: bool,
        detach_external_dimensions: bool,
    },
    UpdateContents {
        block: String,
        expected_block_revision: String,
        entities: Vec<Entity>,
    },
    Duplicate {
        source_block: String,
        expected_block_revision: String,
        block: String,
        name: String,
        #[serde(default)]
        entities: Option<Vec<Entity>>,
    },
}

pub fn block_content_revision(root: &Path, block: &str) -> EditResult<String> {
    validate_source_component(block, "block")?;
    let file = read_history_file(root, &format!("blocks/{block}/entities.ndjson"))?;
    let definition = read_history_file(root, &format!("blocks/{block}/definition.toml"))?;
    if !file.exists || !definition.exists {
        return Err(EditError::InvalidEntity(format!(
            "block {block:?} does not exist"
        )));
    }
    Ok(block_revision(&definition.bytes, &file.bytes))
}

fn block_revision(definition: &[u8], entities: &[u8]) -> String {
    revision(format!("{}:{}", revision(definition), revision(entities)).as_bytes())
}

fn snapshot_block_revision(files: &BTreeMap<String, HistoryFileInput>, block: &str) -> String {
    block_revision(
        &files[&format!("blocks/{block}/definition.toml")].bytes,
        &files[&format!("blocks/{block}/entities.ndjson")].bytes,
    )
}

pub fn apply_block_edit(root: &Path, request: &BlockEditRequest) -> EditResult<DrawingEditResult> {
    with_history_lock(|| apply_locked(root, request))
}

fn apply_locked(root: &Path, request: &BlockEditRequest) -> EditResult<DrawingEditResult> {
    apply_locked_with_hook(root, request, |_| Ok(()))
}

fn apply_locked_with_hook(
    root: &Path,
    request: &BlockEditRequest,
    mut after_publish: impl FnMut(usize) -> EditResult<()>,
) -> EditResult<DrawingEditResult> {
    let (block, creating) = match &request.operation {
        BlockEditOperation::Create { block, .. } | BlockEditOperation::Duplicate { block, .. } => {
            (block, true)
        }
        BlockEditOperation::UpdateContents { block, .. } => (block, false),
    };
    validate_source_component(block, "block")?;
    let mut originals = BTreeMap::new();
    for path in [
        format!("blocks/{block}/definition.toml"),
        format!("blocks/{block}/entities.ndjson"),
        format!("drawings/{}/entities.ndjson", request.drawing),
    ] {
        let file = read_history_file(root, &path)?;
        if creating && path.starts_with("blocks/") && file.exists {
            return Err(EditError::RevisionConflict);
        }
        originals.insert(path, file);
    }
    if let BlockEditOperation::Duplicate {
        source_block,
        expected_block_revision,
        ..
    } = &request.operation
    {
        validate_source_component(source_block, "block")?;
        for path in [
            format!("blocks/{source_block}/definition.toml"),
            format!("blocks/{source_block}/entities.ndjson"),
        ] {
            let file = read_history_file(root, &path)?;
            if !file.exists {
                return Err(EditError::RevisionConflict);
            }
            originals.insert(path, file);
        }
        if snapshot_block_revision(&originals, source_block) != *expected_block_revision {
            return Err(EditError::RevisionConflict);
        }
    }
    let mut project = cad_model::load_project(root)
        .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
    let drawing_index = project
        .drawings
        .iter()
        .position(|drawing| drawing.name == request.drawing)
        .ok_or_else(|| EditError::DrawingNotFound(request.drawing.clone()))?;
    let drawing_path = format!("drawings/{}/entities.ndjson", request.drawing);
    let drawing_before = &originals[&drawing_path];
    if revision(&drawing_before.bytes) != request.expected_revision {
        return Err(EditError::RevisionConflict);
    }
    let baseline = checker_errors(&project);
    let mut changes = BTreeMap::new();
    let mut result_ids = Vec::new();
    let operation;
    match &request.operation {
        BlockEditOperation::Create {
            block,
            name,
            base_point,
            entity_ids,
            replace_originals,
            detach_external_dimensions,
        } => {
            validate_new_block(&project, block, name)?;
            if !base_point.iter().all(|value| value.is_finite()) || entity_ids.is_empty() {
                return Err(EditError::InvalidEntity(
                    "block requires a finite base point and selected entities".into(),
                ));
            }
            ensure_unique_entity_ids(entity_ids)?;
            let drawing_entities = project.drawings[drawing_index]
                .entities
                .iter()
                .map(|record| record.entity.clone())
                .collect::<Vec<_>>();
            let mut selected = entity_ids
                .iter()
                .map(|id| {
                    entity_index(&drawing_entities, id).map(|index| drawing_entities[index].clone())
                })
                .collect::<EditResult<Vec<_>>>()?;
            for entity in &selected {
                ensure_layer_editable(&project, entity.layer())?;
            }
            prepare_copies(&project, &mut selected, *detach_external_dimensions)?;
            let config = cad_model::BlockDefinitionConfig {
                schema_version: cad_model::CURRENT_SCHEMA_VERSION.into(),
                name: name.clone(),
                base_point: *base_point,
            };
            insert_block(&mut project, block, config, selected, &mut changes)?;
            if *replace_originals {
                let mut remaining = drawing_entities
                    .into_iter()
                    .filter(|entity| !entity_ids.contains(&entity.id().as_str().to_owned()))
                    .collect::<Vec<_>>();
                for entity in &mut remaining {
                    if referenced_ids(entity)
                        .iter()
                        .any(|id| entity_ids.contains(id))
                    {
                        if !detach_external_dimensions {
                            return Err(EditError::InvalidEntity(format!(
                                "dimension {} references selected geometry; choose detach or cancel",
                                entity.id().as_str()
                            )));
                        }
                        ensure_layer_editable(&project, entity.layer())?;
                        detach(&project, entity)?;
                    }
                }
                let first = &project.blocks[block].entities[0].entity;
                let reference = create_entity(
                    serde_json::json!({"type":"block_ref","layer":first.layer(),"pen":first.pen(),"block":block,"at":base_point,"rotation_deg":0.0,"scale":1.0,"mirror_x":false,"mirror_y":false}),
                )?;
                result_ids.push(reference.id().as_str().to_owned());
                remaining.push(reference);
                changes.insert(drawing_path.clone(), serialize_entities(&remaining)?);
                project.drawings[drawing_index].entities = records(remaining);
            }
            operation = "create_block";
        }
        BlockEditOperation::UpdateContents {
            block,
            expected_block_revision,
            entities,
        } => {
            validate_source_component(block, "block")?;
            if snapshot_block_revision(&originals, block) != *expected_block_revision {
                return Err(EditError::RevisionConflict);
            }
            let definition = project
                .blocks
                .get(block)
                .ok_or_else(|| EditError::InvalidEntity(format!("unknown block {block}")))?;
            // A locked definition entity cannot be changed indirectly by replacing its containing file.
            for record in &definition.entities {
                if entities
                    .iter()
                    .find(|entity| entity.id() == record.entity.id())
                    != Some(&record.entity)
                {
                    ensure_layer_editable(&project, record.entity.layer())?;
                }
            }
            for entity in entities {
                validate_entity_geometry(entity)?;
                if definition
                    .entities
                    .iter()
                    .find(|record| record.entity.id() == entity.id())
                    .map(|record| &record.entity)
                    != Some(entity)
                {
                    ensure_layer_editable(&project, entity.layer())?;
                }
            }
            project.blocks.get_mut(block).unwrap().entities = records(entities.clone());
            changes.insert(
                format!("blocks/{block}/entities.ndjson"),
                serialize_entities(entities)?,
            );
            operation = "update_block_contents";
        }
        BlockEditOperation::Duplicate {
            source_block,
            expected_block_revision,
            block,
            name,
            entities: staged_entities,
        } => {
            validate_new_block(&project, block, name)?;
            if block_content_revision(root, source_block)? != *expected_block_revision {
                return Err(EditError::RevisionConflict);
            }
            let source = project
                .blocks
                .get(source_block)
                .ok_or_else(|| EditError::InvalidEntity(format!("unknown block {source_block}")))?;
            let mut config = source.config.clone();
            config.name = name.clone();
            let source_entities = source
                .entities
                .iter()
                .map(|record| record.entity.clone())
                .collect::<Vec<_>>();
            let mut entities = staged_entities
                .clone()
                .unwrap_or_else(|| source_entities.clone());
            if staged_entities.is_some() {
                for entity in &entities {
                    validate_entity_geometry(entity)?;
                    if source_entities
                        .iter()
                        .find(|original| original.id() == entity.id())
                        != Some(entity)
                    {
                        ensure_layer_editable(&project, entity.layer())?;
                    }
                }
            }
            prepare_copies(&project, &mut entities, false)?;
            insert_block(&mut project, block, config, entities, &mut changes)?;
            operation = "duplicate_block";
        }
    }
    let errors = checker_errors(&project)
        .difference(&baseline)
        .cloned()
        .collect::<Vec<_>>();
    if !errors.is_empty() {
        return Err(EditError::CheckFailed(errors.join("; ")));
    }
    let before = changes
        .keys()
        .map(|path| originals[path].clone())
        .collect::<Vec<_>>();
    let after = before
        .iter()
        .map(|file| HistoryFileInput {
            relative_path: file.relative_path.clone(),
            exists: true,
            bytes: changes[&file.relative_path].clone(),
            permissions: file.permissions.clone(),
        })
        .collect::<Vec<_>>();
    let mut stage = stage_history_transaction(
        root,
        &request.drawing,
        "project",
        operation,
        &result_ids,
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
        for original in originals.values() {
            let current = read_history_file(root, &original.relative_path)?;
            if current.exists != original.exists || current.bytes != original.bytes {
                return Err(EditError::RevisionConflict);
            }
        }
        for (target, original) in after.iter().zip(&before) {
            publish_history_file(root, target, original)?;
            published += 1;
            after_publish(published)?;
        }
        commit_history_stage(&mut stage)
    })();
    let history_id = match publication {
        Ok(id) => id,
        Err(error) => {
            let mut rollback_errors = Vec::new();
            for index in (0..published).rev() {
                if let Err(failure) = publish_history_file(root, &before[index], &after[index]) {
                    rollback_errors.push(failure.to_string());
                }
            }
            if rollback_errors.is_empty() {
                stage.abort();
                return Err(error);
            }
            let id = stage.history_id().to_owned();
            stage.preserve_for_recovery();
            return Err(EditError::HistoryUnavailable(format!(
                "{error}; rollback failed: {}; recovery history {id}",
                rollback_errors.join("; ")
            )));
        }
    };
    Ok(DrawingEditResult {
        drawing: request.drawing.clone(),
        revision: changes
            .get(&drawing_path)
            .map_or(request.expected_revision.clone(), |bytes| revision(bytes)),
        entity_id: result_ids.first().cloned(),
        entity_ids: result_ids,
        operation: operation.into(),
        history_id: Some(history_id),
    })
}

fn validate_new_block(project: &ProjectSource, block: &str, name: &str) -> EditResult<()> {
    validate_source_component(block, "block")?;
    if project.blocks.contains_key(block) || name.trim().is_empty() {
        return Err(EditError::InvalidEntity(
            "block ID must be unused and name must not be empty".into(),
        ));
    }
    Ok(())
}

fn records(entities: Vec<Entity>) -> Vec<EntityRecord> {
    entities
        .into_iter()
        .enumerate()
        .map(|(index, entity)| EntityRecord {
            line: index + 1,
            entity,
        })
        .collect()
}

fn serialize_entities(entities: &[Entity]) -> EditResult<Vec<u8>> {
    let mut bytes = Vec::new();
    for entity in entities {
        serde_json::to_writer(&mut bytes, entity)
            .map_err(|error| EditError::InvalidEntity(error.to_string()))?;
        bytes.push(b'\n');
    }
    Ok(bytes)
}

fn insert_block(
    project: &mut ProjectSource,
    block: &str,
    config: cad_model::BlockDefinitionConfig,
    entities: Vec<Entity>,
    changes: &mut BTreeMap<String, Vec<u8>>,
) -> EditResult<()> {
    changes.insert(
        format!("blocks/{block}/definition.toml"),
        toml::to_string_pretty(&config)
            .map_err(|error| EditError::InvalidEntity(error.to_string()))?
            .into_bytes(),
    );
    changes.insert(
        format!("blocks/{block}/entities.ndjson"),
        serialize_entities(&entities)?,
    );
    project.blocks.insert(
        block.into(),
        cad_model::BlockDefinition {
            id: block.into(),
            config,
            entities: records(entities),
        },
    );
    Ok(())
}

fn referenced_ids(entity: &Entity) -> Vec<String> {
    cad_model::dimension_anchors_mut(&mut entity.clone())
        .into_iter()
        .filter_map(|anchor| match anchor {
            cad_model::DimensionAnchor::Entity { entity_id, .. } => {
                Some(entity_id.as_str().to_owned())
            }
            cad_model::DimensionAnchor::Fixed { .. } => None,
        })
        .collect()
}

fn detach(project: &ProjectSource, entity: &mut Entity) -> EditResult<()> {
    cad_model::detach_dimension(project, entity).map_err(EditError::InvalidEntity)
}

fn prepare_copies(
    project: &ProjectSource,
    entities: &mut [Entity],
    detach_external: bool,
) -> EditResult<()> {
    let ids = entities
        .iter()
        .map(|entity| entity.id().as_str().to_owned())
        .collect::<BTreeSet<_>>();
    let replacements = entities
        .iter()
        .map(|entity| {
            (
                entity.id().clone(),
                cad_model::EntityId::parse(&next_id()).expect("generated entity ID"),
            )
        })
        .collect::<BTreeMap<_, _>>();
    for entity in entities {
        if referenced_ids(entity).iter().any(|id| !ids.contains(id)) {
            if !detach_external {
                return Err(EditError::InvalidEntity(format!(
                    "dimension {} references geometry outside the block; choose detach or cancel",
                    entity.id().as_str()
                )));
            }
            detach(project, entity)?;
        }
        cad_model::remap_dimension_references(entity, &replacements);
        let id = replacements[entity.id()].as_str().to_owned();
        set_entity_id(entity, &id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_duplicates_and_edits_blocks_in_shared_history() {
        let parent = tempfile::tempdir().unwrap();
        let created = create_project(&ProjectTemplateRequest {
            parent_dir: parent.path().to_string_lossy().into_owned(),
            folder_name: "blocks".into(),
            project_name: "Blocks".into(),
            drawing: "plan".into(),
            paper: "A3".into(),
            orientation: cad_model::SheetOrientation::Landscape,
            scale_denominator: 100,
        })
        .unwrap();
        let root = Path::new(&created.project_path);
        let drawing_path = "drawings/plan/entities.ndjson";
        let before = read_history_file(root, drawing_path).unwrap();
        let line =
            create_entity(serde_json::json!({"type":"line","layer":"0-1","p1":[0,0],"p2":[10,0]}))
                .unwrap();
        let bytes = serialize_entities(std::slice::from_ref(&line)).unwrap();
        atomic_replace(&root.join(drawing_path), &bytes, &revision(&before.bytes)).unwrap();
        let request = BlockEditRequest {
            drawing: "plan".into(),
            expected_revision: revision(&bytes),
            operation: BlockEditOperation::Create {
                block: "failed".into(),
                name: "Failed".into(),
                base_point: [0., 0.],
                entity_ids: vec![line.id().as_str().into()],
                replace_originals: true,
                detach_external_dimensions: false,
            },
        };
        let failure = with_history_lock(|| {
            apply_locked_with_hook(root, &request, |count| {
                if count == 2 {
                    Err(EditError::InvalidEntity(
                        "injected publication failure".into(),
                    ))
                } else {
                    Ok(())
                }
            })
        });
        assert!(failure.is_err());
        assert_eq!(fs::read(root.join(drawing_path)).unwrap(), bytes);
        assert!(!root.join("blocks/failed/definition.toml").exists());
        assert!(!root.join("blocks/failed/entities.ndjson").exists());
        assert!(
            !cad_model::load_project(root)
                .unwrap()
                .blocks
                .contains_key("failed")
        );
        let result = apply_block_edit(
            root,
            &BlockEditRequest {
                drawing: "plan".into(),
                expected_revision: revision(&bytes),
                operation: BlockEditOperation::Create {
                    block: "door".into(),
                    name: "Door".into(),
                    base_point: [2., 0.],
                    entity_ids: vec![line.id().as_str().into()],
                    replace_originals: true,
                    detach_external_dimensions: false,
                },
            },
        )
        .unwrap();
        let loaded = cad_model::load_project(root).unwrap();
        assert_eq!(loaded.blocks["door"].entities.len(), 1);
        assert_ne!(loaded.blocks["door"].entities[0].entity.id(), line.id());
        assert!(matches!(
            loaded.drawings[0].entities[0].entity,
            Entity::BlockRef { .. }
        ));
        undo_drawing_edit(
            root,
            &DrawingHistoryRequest {
                drawing: "plan".into(),
                expected_files: vec![],
            },
        )
        .unwrap();
        assert_eq!(fs::read(root.join(drawing_path)).unwrap(), bytes);
        assert!(
            !cad_model::load_project(root)
                .unwrap()
                .blocks
                .contains_key("door")
        );
        redo_drawing_edit(
            root,
            &DrawingHistoryRequest {
                drawing: "plan".into(),
                expected_files: vec![],
            },
        )
        .unwrap();
        let original = block_content_revision(root, "door").unwrap();
        apply_block_edit(
            root,
            &BlockEditRequest {
                drawing: "plan".into(),
                expected_revision: result.revision.clone(),
                operation: BlockEditOperation::Duplicate {
                    source_block: "door".into(),
                    expected_block_revision: original.clone(),
                    block: "door-copy".into(),
                    name: "Door copy".into(),
                    entities: None,
                },
            },
        )
        .unwrap();
        let copied = cad_model::load_project(root).unwrap();
        assert_ne!(
            copied.blocks["door"].entities[0].entity.id(),
            copied.blocks["door-copy"].entities[0].entity.id()
        );
        let changed =
            translate_entity(&copied.blocks["door"].entities[0].entity, [5., 0.]).unwrap();
        apply_block_edit(
            root,
            &BlockEditRequest {
                drawing: "plan".into(),
                expected_revision: result.revision.clone(),
                operation: BlockEditOperation::UpdateContents {
                    block: "door".into(),
                    expected_block_revision: original,
                    entities: vec![changed.clone()],
                },
            },
        )
        .unwrap();
        assert_eq!(
            cad_model::load_project(root).unwrap().blocks["door"].entities[0].entity,
            changed
        );
        let stale = apply_block_edit(
            root,
            &BlockEditRequest {
                drawing: "plan".into(),
                expected_revision: result.revision.clone(),
                operation: BlockEditOperation::UpdateContents {
                    block: "door".into(),
                    expected_block_revision: "stale".into(),
                    entities: vec![],
                },
            },
        );
        assert!(matches!(stale, Err(EditError::RevisionConflict)));
        let cycle=create_entity(serde_json::json!({"type":"block_ref","layer":"0-1","block":"door","at":[0,0],"rotation_deg":0.0,"scale":1.0,"mirror_x":false,"mirror_y":false})).unwrap();
        let previous = fs::read(root.join("blocks/door/entities.ndjson")).unwrap();
        let rejected = apply_block_edit(
            root,
            &BlockEditRequest {
                drawing: "plan".into(),
                expected_revision: result.revision,
                operation: BlockEditOperation::UpdateContents {
                    block: "door".into(),
                    expected_block_revision: block_content_revision(root, "door").unwrap(),
                    entities: vec![cycle],
                },
            },
        );
        assert!(matches!(rejected, Err(EditError::CheckFailed(_))));
        assert_eq!(
            fs::read(root.join("blocks/door/entities.ndjson")).unwrap(),
            previous
        );

        let mut project = cad_model::load_project(root).unwrap();
        let source = project.blocks["door"].entities[0].entity.clone();
        let dimension=create_entity(serde_json::json!({"type":"dimension","layer":"0-1","style":"default","p1":[0,0],"p2":[10,0],"offset":2.0,"value":null,"measurement":{"kind":"aligned","first":{"kind":"entity","entity_id":source.id().as_str(),"feature":"start"},"second":{"kind":"entity","entity_id":source.id().as_str(),"feature":"end"}}})).unwrap();
        project
            .blocks
            .get_mut("door")
            .unwrap()
            .entities
            .push(EntityRecord {
                line: 2,
                entity: dimension.clone(),
            });
        let mut selected = vec![source, dimension.clone()];
        prepare_copies(&project, &mut selected, false).unwrap();
        assert!(
            referenced_ids(&selected[1])
                .iter()
                .all(|id| id == selected[0].id().as_str())
        );
        assert!(prepare_copies(&project, &mut [dimension.clone()], false).is_err());
        let mut detached = vec![dimension.clone()];
        prepare_copies(&project, &mut detached, true).unwrap();
        assert!(referenced_ids(&detached[0]).is_empty());

        let saved_source = fs::read(root.join("blocks/door/entities.ndjson")).unwrap();
        let staged_line =
            translate_entity(&project.blocks["door"].entities[0].entity, [7., 0.]).unwrap();
        let mut staged_dimension = dimension;
        if let Entity::Dimension { style, .. } = &mut staged_dimension {
            *style = project
                .styles
                .dimension_styles
                .keys()
                .next()
                .unwrap()
                .clone();
        }
        apply_block_edit(
            root,
            &BlockEditRequest {
                drawing: "plan".into(),
                expected_revision: revision(&fs::read(root.join(drawing_path)).unwrap()),
                operation: BlockEditOperation::Duplicate {
                    source_block: "door".into(),
                    expected_block_revision: block_content_revision(root, "door").unwrap(),
                    block: "staged-copy".into(),
                    name: "Staged copy".into(),
                    entities: Some(vec![staged_line.clone(), staged_dimension]),
                },
            },
        )
        .unwrap();
        assert_eq!(
            fs::read(root.join("blocks/door/entities.ndjson")).unwrap(),
            saved_source
        );
        let loaded = cad_model::load_project(root).unwrap();
        let duplicated = &loaded.blocks["staged-copy"].entities;
        let mut expected_line = staged_line;
        set_entity_id(&mut expected_line, duplicated[0].entity.id().as_str()).unwrap();
        assert_eq!(duplicated[0].entity, expected_line);
        assert!(
            referenced_ids(&duplicated[1].entity)
                .iter()
                .all(|id| id == duplicated[0].entity.id().as_str())
        );
        undo_drawing_edit(
            root,
            &DrawingHistoryRequest {
                drawing: "plan".into(),
                expected_files: vec![],
            },
        )
        .unwrap();
        assert!(
            !cad_model::load_project(root)
                .unwrap()
                .blocks
                .contains_key("staged-copy")
        );
        assert_eq!(
            fs::read(root.join("blocks/door/entities.ndjson")).unwrap(),
            saved_source
        );
    }
}
