//! Entity history follows the pinned first-parent chain and semantic dependencies.
use crate::{GitError, Result, Revision, Snapshot, git_text, repository_root, snapshot};
use cad_model::Entity;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
pub struct EntityHistoryEvent {
    pub commit_oid: String,
    pub parent_oid: Option<String>,
    pub subject: String,
    pub timestamp: i64,
    pub drawing: String,
    pub kind: cad_diff::ChangeKind,
    pub reasons: Vec<cad_diff::ChangeReason>,
    pub before: Option<Entity>,
    pub after: Option<Entity>,
}
#[derive(Debug, Serialize)]
pub struct EntityHistoryReport {
    pub schema_version: String,
    pub entity_id: String,
    pub pinned_revision: String,
    pub first_parent: bool,
    pub scanned_commits: usize,
    pub truncated: bool,
    pub events: Vec<EntityHistoryEvent>,
    pub warnings: Vec<String>,
}
fn invalid(message: impl ToString) -> GitError {
    GitError::Invalid(message.to_string())
}
fn entity(snapshot: &Snapshot, id: &str) -> Option<Entity> {
    snapshot
        .source
        .drawings
        .iter()
        .flat_map(|d| &d.entities)
        .find(|r| r.entity.id().as_str() == id)
        .map(|r| r.entity.clone())
}
pub fn entity_history(
    project: &Path,
    id: &str,
    revision: &str,
    limit: usize,
) -> Result<EntityHistoryReport> {
    if !(1..=500).contains(&limit) {
        return Err(invalid("History limit must be 1..500 commits"));
    }
    cad_model::EntityId::parse(id).map_err(invalid)?;
    let repo = repository_root(project)?;
    let oid = git_text(
        &repo,
        &[
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{revision}^{{commit}}"),
        ],
    )?;
    // Walk every first-parent commit so renamed drawings and block/style changes are included.
    let commits = git_text(
        &repo,
        &[
            "rev-list",
            "--first-parent",
            &format!("--max-count={}", limit + 1),
            &oid,
        ],
    )?;
    let commits: Vec<_> = commits.lines().collect();
    let mut report = EntityHistoryReport {
        schema_version: "cad-entity-history/1".into(),
        entity_id: id.into(),
        pinned_revision: oid,
        first_parent: true,
        scanned_commits: commits.len().min(limit),
        truncated: commits.len() > limit,
        events: Vec::new(),
        warnings: Vec::new(),
    };
    let empty_dir = tempfile::tempdir()?;
    let optional_snapshot = |commit: &str| -> Result<Option<Snapshot>> {
        match snapshot(project, &Revision::Commit(commit.into())) {
            Ok(value) => Ok(Some(value)),
            Err(GitError::Invalid(message)) if message.ends_with(" has no CAD project source") => {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    };
    for commit in commits.into_iter().take(limit) {
        let parent = git_text(&repo, &["rev-list", "--parents", "-n", "1", commit])?
            .split_whitespace()
            .nth(1)
            .map(str::to_owned);
        let after = match optional_snapshot(commit) {
            Ok(value) => value,
            Err(error) => {
                report
                    .warnings
                    .push(format!("{commit}: source unavailable: {error}"));
                continue;
            }
        };
        let before = if let Some(parent) = &parent {
            match optional_snapshot(parent) {
                Ok(value) => value,
                Err(error) => {
                    report
                        .warnings
                        .push(format!("{commit}: parent source unavailable: {error}"));
                    continue;
                }
            }
        } else {
            None
        };
        for (label, snapshot) in [("source", after.as_ref()), ("parent", before.as_ref())] {
            if let Some(snapshot) = snapshot {
                let check = cad_check::check_loaded_project(&snapshot.source);
                if !check.is_ok() {
                    report.warnings.push(format!("{commit}: {label} CAD validation errors; reference-dependent geometry may be incomplete: {}",serde_json::to_string(&check).map_err(invalid)?));
                }
            }
        }
        let Some(template) = after.as_ref().or(before.as_ref()) else {
            continue;
        };
        let mut empty = template.source.clone();
        empty.root = empty_dir.path().into();
        empty.drawings.clear();
        empty.blocks.clear();
        let after_source = after.as_ref().map(|s| &s.source).unwrap_or(&empty);
        let before_source = before.as_ref().map(|s| &s.source).unwrap_or(&empty);
        let after_entity = after.as_ref().and_then(|s| entity(s, id));
        let before_entity = before.as_ref().and_then(|s| entity(s, id));
        for change in cad_diff::diff_projects(before_source, after_source)
            .changes
            .into_iter()
            .filter(|change| {
                change.entity_id == id && change.kind != cad_diff::ChangeKind::Unchanged
            })
        {
            let subject = git_text(&repo, &["show", "-s", "--format=%s", commit])?;
            let timestamp = git_text(&repo, &["show", "-s", "--format=%ct", commit])?
                .parse()
                .map_err(invalid)?;
            report.events.push(EntityHistoryEvent {
                commit_oid: commit.into(),
                parent_oid: parent.clone(),
                subject,
                timestamp,
                drawing: change.drawing,
                before: if change.kind == cad_diff::ChangeKind::Added {
                    None
                } else {
                    before_entity.clone()
                },
                after: if change.kind == cad_diff::ChangeKind::Removed {
                    None
                } else {
                    after_entity.clone()
                },
                kind: change.kind,
                reasons: change.reasons,
            });
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn history_tracks_geometry_creation_and_deletion_at_a_pinned_head_without_index_writes() {
        let temp = tempfile::tempdir().unwrap();
        let created = cad_edit::create_project(&cad_edit::ProjectTemplateRequest {
            parent_dir: temp.path().display().to_string(),
            folder_name: "repo".into(),
            project_name: "history".into(),
            drawing: "plan".into(),
            paper: "A3".into(),
            orientation: cad_model::SheetOrientation::Landscape,
            scale_denominator: 100,
        })
        .unwrap();
        let root = Path::new(&created.project_path);
        let git = |args: &[&str]| git_text(root, args).unwrap();
        git(&["init"]);
        git(&["config", "user.name", "CAD test"]);
        git(&["config", "user.email", "cad@example.invalid"]);
        git(&["commit", "--allow-empty", "-m", "before CAD project"]);
        let id = "ent_01JZ0000000000000000000111";
        let file = root.join("drawings/plan/entities.ndjson");
        let mut value = serde_json::json!({"schema_version":"0.3","id":id,"type":"line","layer":"0-1","p1":[0,0],"p2":[100,0]});
        let source = cad_model::load_project(root).unwrap();
        let style = source.styles.dimension_styles.keys().next().unwrap();
        let dimension_id = "ent_01JZ0000000000000000000222";
        let dimension = serde_json::json!({"schema_version":"0.3","id":dimension_id,"type":"dimension","layer":"0-1","style":style,"p1":[0,0],"p2":[100,0],"offset":100,"measurement":{"kind":"horizontal","first":{"kind":"entity","entity_id":id,"feature":"start"},"second":{"kind":"entity","entity_id":id,"feature":"end"}}});
        std::fs::write(&file, format!("{value}\n{dimension}\n")).unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "created"]);
        value["p2"] = serde_json::json!([200, 0]);
        std::fs::write(&file, format!("{value}\n{dimension}\n")).unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "longer"]);
        let path = root.join("rules/styles.toml");
        let mut styles = cad_model::load_project(root).unwrap().styles;
        styles.colors.get_mut("black").unwrap().rgb = "#FF0000".into();
        std::fs::write(path, toml::to_string_pretty(&styles).unwrap()).unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "appearance"]);
        std::fs::write(&file, "").unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "removed"]);
        let index = std::fs::read(root.join(".git/index")).unwrap();
        let report = entity_history(root, id, "HEAD", 50).unwrap();
        assert_eq!(
            report
                .events
                .iter()
                .map(|e| e.kind.clone())
                .collect::<Vec<_>>(),
            vec![
                cad_diff::ChangeKind::Removed,
                cad_diff::ChangeKind::Modified,
                cad_diff::ChangeKind::Modified,
                cad_diff::ChangeKind::Added
            ]
        );
        assert!(
            report.events[1]
                .reasons
                .contains(&cad_diff::ChangeReason::StyleChanged)
        );
        assert!(report.warnings.is_empty());
        assert!(!report.truncated);
        assert_eq!(std::fs::read(root.join(".git/index")).unwrap(), index);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
        let dimensions = entity_history(root, dimension_id, "HEAD", 50).unwrap();
        let dependency = dimensions
            .events
            .iter()
            .find(|event| event.subject == "longer")
            .unwrap();
        assert_eq!(dependency.before, dependency.after);
        assert!(
            dependency
                .reasons
                .contains(&cad_diff::ChangeReason::DependencyChanged)
        );
        let limited = entity_history(root, id, "HEAD", 1).unwrap();
        assert!(limited.truncated);
        assert_eq!(limited.events.len(), 1);
        // Moving a drawing retains the entity ID and emits a removed/added pair at one commit.
        std::fs::write(&file, format!("{value}\n{dimension}\n")).unwrap();
        git(&["add", "."]);
        git(&["commit", "-m", "recreated"]);
        std::fs::rename(root.join("drawings/plan"), root.join("drawings/renamed")).unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-m", "drawing moved"]);
        let moved = entity_history(root, id, "HEAD", 50).unwrap();
        let moved_events: Vec<_> = moved
            .events
            .iter()
            .filter(|event| event.subject == "drawing moved")
            .collect();
        assert_eq!(moved_events.len(), 2);
        assert!(
            moved_events
                .iter()
                .any(|event| event.kind == cad_diff::ChangeKind::Removed
                    && event.before.is_some()
                    && event.after.is_none())
        );
        assert!(
            moved_events
                .iter()
                .any(|event| event.kind == cad_diff::ChangeKind::Added
                    && event.before.is_none()
                    && event.after.is_some())
        );
        std::fs::remove_file(root.join("cad.project.toml")).unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-m", "project removed"]);
        let removed = entity_history(root, id, "HEAD", 50).unwrap();
        assert_eq!(removed.events[0].subject, "project removed");
        assert_eq!(removed.events[0].kind, cad_diff::ChangeKind::Removed);
        assert!(removed.warnings.is_empty());
    }
}
