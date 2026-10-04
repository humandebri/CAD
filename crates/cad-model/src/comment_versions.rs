//! Comment version metadata is app-managed; evaluating it never modifies a comment.
use super::*;

#[derive(Debug, Error)]
pub enum CommentVersionError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentBinding {
    pub schema_version: String,
    pub drawing: String,
    /// Normalized whole-project identity, excluding comments and generated data.
    pub source_blake3: String,
    /// HEAD observed at creation; the source hash also covers working changes.
    pub git_commit: Option<String>,
    pub entity_blake3: String,
    pub entity: Entity,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentBindingState {
    Unbound,
    Current,
    EntityChanged,
    EntityDeleted,
    EntityMoved,
    SourceChanged,
    Invalid,
}

/// Hashes normalized canonical content. All drawings, layouts and definitions
/// are included, so unrelated canonical edits conservatively mark source_changed.
/// Comments, filesystem roots, provenance, generated output and history are excluded.
pub fn comment_source_revision(project: &ProjectSource) -> Result<String, CommentVersionError> {
    let value = serde_json::json!({
        "project":project.project,"layers":project.layers,"styles":project.styles,
        "drawings":project.drawings.iter().map(|drawing|serde_json::json!({
            "name":drawing.name,"layouts":drawing.layouts,
            "entities":drawing.entities.iter().map(|r|&r.entity).collect::<Vec<_>>()
        })).collect::<Vec<_>>(),
        "blocks":project.blocks.iter().map(|(id,block)|serde_json::json!({
            "id":id,"definition":block.config,
            "entities":block.entities.iter().map(|r|&r.entity).collect::<Vec<_>>()
        })).collect::<Vec<_>>()
    });
    Ok(blake3::hash(&serde_json::to_vec(&value)?)
        .to_hex()
        .to_string())
}

fn entity_hash(entity: &Entity) -> Result<String, CommentVersionError> {
    Ok(blake3::hash(&serde_json::to_vec(entity)?)
        .to_hex()
        .to_string())
}

pub fn bind_comment_entity(
    project: &ProjectSource,
    drawing: &str,
    id: &str,
    git_commit: Option<String>,
) -> Result<CommentBinding, CommentVersionError> {
    let entity = project
        .drawings
        .iter()
        .find(|d| d.name == drawing)
        .and_then(|d| d.entities.iter().find(|r| r.entity.id().as_str() == id))
        .ok_or_else(|| {
            CommentVersionError::Invalid("Comment entity is absent from the drawing".into())
        })?
        .entity
        .clone();
    Ok(CommentBinding {
        schema_version: "cad-comment-binding/1".into(),
        drawing: drawing.into(),
        source_blake3: comment_source_revision(project)?,
        git_commit,
        entity_blake3: entity_hash(&entity)?,
        entity,
        extra: BTreeMap::new(),
    })
}

pub fn evaluate_comment_binding(
    project: &ProjectSource,
    drawing: &str,
    ids: &[String],
    binding: Option<&CommentBinding>,
    source_revision: &str,
) -> CommentBindingState {
    let Some(binding) = binding else {
        return CommentBindingState::Unbound;
    };
    let valid_hash =
        |value: &str| value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit());
    if binding.schema_version != "cad-comment-binding/1"
        || binding.drawing != drawing
        || ids.len() != 1
        || ids[0] != binding.entity.id().as_str()
        || !binding.extra.is_empty()
        || !valid_hash(&binding.source_blake3)
        || entity_hash(&binding.entity).ok().as_deref() != Some(binding.entity_blake3.as_str())
    {
        return CommentBindingState::Invalid;
    }
    let entity = project
        .drawings
        .iter()
        .find(|d| d.name == drawing)
        .and_then(|d| {
            d.entities
                .iter()
                .find(|r| r.entity.id() == binding.entity.id())
        });
    let Some(entity) = entity else {
        return if project.drawings.iter().any(|d| {
            d.entities
                .iter()
                .any(|r| r.entity.id() == binding.entity.id())
        }) {
            CommentBindingState::EntityMoved
        } else {
            CommentBindingState::EntityDeleted
        };
    };
    if entity.entity != binding.entity {
        CommentBindingState::EntityChanged
    } else if source_revision != binding.source_blake3 {
        CommentBindingState::SourceChanged
    } else {
        CommentBindingState::Current
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn version_checks_distinguish_geometry_dependencies_moves_and_legacy_comments() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/house-small");
        let mut project = load_project(root).unwrap();
        let id = project.drawings[0].entities[0]
            .entity
            .id()
            .as_str()
            .to_owned();
        let drawing = project.drawings[0].name.clone();
        let ids = vec![id.clone()];
        let binding = bind_comment_entity(&project, &drawing, &id, Some("a".repeat(40))).unwrap();
        let state = |p: &ProjectSource, b: &CommentBinding| {
            evaluate_comment_binding(
                p,
                &drawing,
                &ids,
                Some(b),
                &comment_source_revision(p).unwrap(),
            )
        };
        assert_eq!(state(&project, &binding), CommentBindingState::Current);
        assert_eq!(
            evaluate_comment_binding(&project, &drawing, &ids, None, &binding.source_blake3),
            CommentBindingState::Unbound
        );
        project.styles.colors.values_mut().next().unwrap().rgb = "#123456".into();
        assert_eq!(
            state(&project, &binding),
            CommentBindingState::SourceChanged
        );
        if let Entity::Line { p2, .. } = &mut project.drawings[0].entities[0].entity {
            p2[0] += 100.;
        } else {
            panic!("fixture starts with line");
        }
        assert_eq!(
            state(&project, &binding),
            CommentBindingState::EntityChanged
        );
        let moved = project.drawings[0].entities.remove(0);
        let mut second = project.drawings[0].clone();
        second.name = "other".into();
        second.entities = vec![moved];
        project.drawings.push(second);
        assert_eq!(state(&project, &binding), CommentBindingState::EntityMoved);
        project.drawings.pop();
        assert_eq!(
            state(&project, &binding),
            CommentBindingState::EntityDeleted
        );
        let mut invalid = binding.clone();
        invalid.entity_blake3 = "bad".into();
        assert_eq!(state(&project, &invalid), CommentBindingState::Invalid);
        invalid = binding.clone();
        invalid.schema_version = "future".into();
        assert_eq!(state(&project, &invalid), CommentBindingState::Invalid);
        invalid = binding.clone();
        invalid
            .extra
            .insert("future".into(), serde_json::json!(true));
        assert_eq!(state(&project, &invalid), CommentBindingState::Invalid);
    }
}
