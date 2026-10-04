//! Last field changes on a pinned first-parent path, separately from display dependencies.
use crate::{GitError, Result, Revision, history::entity_history, snapshot};
use serde::Serialize;
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

#[derive(Debug, Clone, Serialize)]
pub struct FieldCommit {
    pub commit_oid: String,
    pub parent_oid: Option<String>,
    pub subject: String,
    pub author: String,
    pub timestamp: i64,
}

#[derive(Debug, Serialize)]
pub struct FieldAttribution {
    /// JSON Pointer into the normalized entity, with arrays treated atomically.
    pub pointer: String,
    pub value: Value,
    pub origin: Option<FieldCommit>,
}

#[derive(Debug, Serialize)]
pub struct DependencyAttribution {
    pub commit: FieldCommit,
    pub reasons: Vec<cad_diff::ChangeReason>,
}

#[derive(Debug, Serialize)]
pub struct EntityBlameReport {
    pub schema_version: String,
    pub entity_id: String,
    pub pinned_revision: String,
    pub first_parent: bool,
    pub field_representation: String,
    pub drawing: String,
    pub drawing_origin: Option<FieldCommit>,
    pub scanned_commits: usize,
    pub truncated: bool,
    pub reliable: bool,
    pub fields: Vec<FieldAttribution>,
    pub dependency_changes: Vec<DependencyAttribution>,
    pub warnings: Vec<String>,
}

fn fields(value: &Value, prefix: &str, result: &mut BTreeMap<String, Value>) {
    if let Value::Object(object) = value
        && !object.is_empty()
    {
        for (name, child) in object {
            let escaped = name.replace('~', "~0").replace('/', "~1");
            fields(child, &format!("{prefix}/{escaped}"), result);
        }
        return;
    }
    result.insert(prefix.to_owned(), value.clone());
}

fn normalized(entity: Option<&cad_model::Entity>) -> Result<BTreeMap<String, Value>> {
    let mut result = BTreeMap::new();
    if let Some(entity) = entity {
        let value = serde_json::to_value(entity).map_err(|e| GitError::Invalid(e.to_string()))?;
        fields(&value, "", &mut result);
    }
    Ok(result)
}

pub fn entity_blame(
    project: &Path,
    id: &str,
    revision: &str,
    limit: usize,
) -> Result<EntityBlameReport> {
    // History resolves the requested ref once; all subsequent reads use its OID.
    let history = entity_history(project, id, revision, limit)?;
    let current = snapshot(project, &Revision::Commit(history.pinned_revision.clone()))?;
    let (drawing,entity)=current.source.drawings.iter().find_map(|drawing| {
        drawing.entities.iter().find(|record|record.entity.id().as_str()==id)
            .map(|record|(drawing.name.clone(),&record.entity))
    }).ok_or_else(||GitError::Invalid("Entity is absent from the pinned revision; use entity-history to inspect deletions".into()))?;
    let current_fields = normalized(Some(entity))?;
    let repo = crate::repository_root(project)?;
    let mut origins = BTreeMap::new();
    let mut drawing_origin = None;
    let mut dependencies = Vec::new();
    let mut offset = 0;
    while offset < history.events.len() {
        let first = &history.events[offset];
        let mut end = offset + 1;
        while end < history.events.len() && history.events[end].commit_oid == first.commit_oid {
            end += 1;
        }
        let events = &history.events[offset..end];
        // A drawing move emits a removal and an addition in one commit. Pair
        // them before attributing fields, so it does not masquerade as creation.
        let before = events
            .iter()
            .find_map(|e| e.before.as_ref().map(|v| (&e.drawing, v)));
        let after = events
            .iter()
            .find_map(|e| e.after.as_ref().map(|v| (&e.drawing, v)));
        let before_fields = normalized(before.map(|(_, v)| v))?;
        let after_fields = normalized(after.map(|(_, v)| v))?;
        let commit = FieldCommit {
            commit_oid: first.commit_oid.clone(),
            parent_oid: first.parent_oid.clone(),
            subject: first.subject.clone(),
            author: crate::git_text(&repo, &["show", "-s", "--format=%an", &first.commit_oid])?,
            timestamp: first.timestamp,
        };
        for (pointer, value) in &current_fields {
            if !origins.contains_key(pointer)
                && after_fields.get(pointer) == Some(value)
                && before_fields.get(pointer) != after_fields.get(pointer)
            {
                origins.insert(pointer.clone(), commit.clone());
            }
        }
        if drawing_origin.is_none()
            && after.map(|(name, _)| name.as_str()) == Some(drawing.as_str())
            && before.map(|(name, _)| name) != after.map(|(name, _)| name)
        {
            drawing_origin = Some(commit.clone());
        }
        if before.is_some()
            && after.is_some()
            && before_fields == after_fields
            && before.map(|(name, _)| name) == after.map(|(name, _)| name)
        {
            let mut reasons = Vec::new();
            for event in events {
                for reason in &event.reasons {
                    if !reasons.contains(reason) {
                        reasons.push(reason.clone());
                    }
                }
            }
            dependencies.push(DependencyAttribution { commit, reasons });
        }
        offset = end;
    }
    let unresolved =
        current_fields.keys().any(|key| !origins.contains_key(key)) || drawing_origin.is_none();
    let mut warnings = history.warnings;
    let check = cad_check::check_loaded_project(&current.source);
    if !check.is_ok() {
        warnings.push("Pinned revision has CAD validation errors".into());
    }
    if unresolved {
        warnings.push("Some origins were not found in the scanned history; null origins are unresolved, not unchanged".into());
    }
    let reliable = !history.truncated && warnings.is_empty() && !unresolved;
    Ok(EntityBlameReport {
        schema_version: "cad-entity-blame/1".into(),
        entity_id: id.into(),
        pinned_revision: history.pinned_revision,
        first_parent: true,
        field_representation: "normalized_entity_arrays_atomic".into(),
        drawing,
        drawing_origin,
        scanned_commits: history.scanned_commits,
        truncated: history.truncated,
        reliable,
        fields: current_fields
            .into_iter()
            .map(|(pointer, value)| FieldAttribution {
                origin: origins.remove(&pointer),
                pointer,
                value,
            })
            .collect(),
        dependency_changes: dependencies,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fields_survive_drawing_moves_and_separate_style_changes_without_index_writes() {
        let temp = tempfile::tempdir().unwrap();
        let created = cad_edit::create_project(&cad_edit::ProjectTemplateRequest {
            parent_dir: temp.path().display().to_string(),
            folder_name: "repo".into(),
            project_name: "blame".into(),
            drawing: "plan".into(),
            paper: "A3".into(),
            orientation: cad_model::SheetOrientation::Landscape,
            scale_denominator: 100,
        })
        .unwrap();
        let root = Path::new(&created.project_path);
        let git = |args: &[&str]| crate::git_text(root, args).unwrap();
        git(&["init"]);
        git(&["config", "user.name", "CAD test"]);
        git(&["config", "user.email", "cad@example.invalid"]);
        let id = "ent_01JZ0000000000000000000111";
        let file = root.join("drawings/plan/entities.ndjson");
        let mut value = serde_json::json!({"schema_version":"0.3","id":id,"type":"line","layer":"0-1","p1":[0,0],"p2":[100,0]});
        std::fs::write(&file, format!("{value}\n")).unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "created"]);
        let created_oid = git(&["rev-parse", "HEAD"]);
        value["p2"] = serde_json::json!([200, 0]);
        std::fs::write(&file, format!("{value}\n")).unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "longer"]);
        let longer_oid = git(&["rev-parse", "HEAD"]);
        let mut styles = cad_model::load_project(root).unwrap().styles;
        styles.colors.get_mut("black").unwrap().rgb = "#FF0000".into();
        std::fs::write(
            root.join("rules/styles.toml"),
            toml::to_string_pretty(&styles).unwrap(),
        )
        .unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "appearance"]);
        let style_oid = git(&["rev-parse", "HEAD"]);
        std::fs::create_dir(root.join("drawings/moved")).unwrap();
        std::fs::copy(
            root.join("drawings/plan/layouts.toml"),
            root.join("drawings/moved/layouts.toml"),
        )
        .unwrap();
        std::fs::rename(&file, root.join("drawings/moved/entities.ndjson")).unwrap();
        std::fs::write(&file, "").unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "moved"]);
        let moved_oid = git(&["rev-parse", "HEAD"]);
        std::fs::write(root.join("keep.txt"), "unrelated staged content").unwrap();
        git(&["add", "keep.txt"]);
        let index = std::fs::read(root.join(".git/index")).unwrap();
        let source = cad_model::source_manifest(root).unwrap();
        let report = entity_blame(root, id, "HEAD", 50).unwrap();
        assert!(report.reliable, "{:?}", report.warnings);
        assert_eq!(report.pinned_revision, moved_oid);
        assert_eq!(report.drawing, "moved");
        assert_eq!(
            report.drawing_origin.as_ref().unwrap().commit_oid,
            moved_oid
        );
        let origin = |pointer: &str| {
            report
                .fields
                .iter()
                .find(|f| f.pointer == pointer)
                .unwrap()
                .origin
                .as_ref()
                .unwrap()
                .commit_oid
                .clone()
        };
        assert_eq!(origin("/p1"), created_oid);
        assert_eq!(origin("/p2"), longer_oid);
        assert_eq!(origin("/layer"), created_oid);
        assert!(
            report
                .fields
                .iter()
                .filter_map(|f| f.origin.as_ref())
                .all(|origin| origin.author == "CAD test")
        );
        assert!(
            report
                .dependency_changes
                .iter()
                .any(|e| e.commit.commit_oid == style_oid)
        );
        assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
        assert_eq!(cad_model::source_manifest(root).unwrap(), source);
        let limited = entity_blame(root, id, "HEAD", 1).unwrap();
        assert!(limited.truncated);
        assert!(!limited.reliable);
        assert!(limited.fields.iter().any(|f| f.origin.is_none()));
        assert!(entity_blame(root, "ent_01JZ0000000000000000000999", "HEAD", 50).is_err());
        // A bad intermediate version must not acquire confident attribution.
        std::fs::write(root.join("drawings/moved/entities.ndjson"), "broken").unwrap();
        git(&["add", "drawings/moved/entities.ndjson"]);
        git(&["commit", "-m", "bad source"]);
        std::fs::write(
            root.join("drawings/moved/entities.ndjson"),
            format!("{value}\n"),
        )
        .unwrap();
        git(&["add", "drawings/moved/entities.ndjson"]);
        git(&["commit", "-m", "restored"]);
        let incomplete = entity_blame(root, id, "HEAD", 50).unwrap();
        assert!(!incomplete.reliable);
        assert!(!incomplete.warnings.is_empty());
    }
}
