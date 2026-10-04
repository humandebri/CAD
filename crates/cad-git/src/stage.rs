//! Selective CAD staging. Plan first, then compare-and-swap the complete index.
use crate::{
    GitError, Result, Revision, SnapshotIdentity, git_text, repository_root, snapshot, source_files,
};
use cad_model::{Entity, ProjectSource};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn invalid(e: impl ToString) -> GitError {
    GitError::Invalid(e.to_string())
}
#[derive(Debug, Serialize)]
pub struct StageReport {
    pub schema_version: String,
    pub plan_hash: String,
    pub drawing: String,
    pub requested_ids: Vec<String>,
    pub expanded_ids: Vec<String>,
    pub changed_files: Vec<String>,
    pub index: SnapshotIdentity,
    pub worktree: SnapshotIdentity,
    pub diff: cad_diff::DiffReport,
    pub cad_check: cad_check::CheckReport,
    pub applied: bool,
}
pub struct StagePlan {
    pub report: StageReport,
    pub preview_svg: String,
    project: PathBuf,
    index_path: PathBuf,
    index_bytes: Vec<u8>,
    files: BTreeMap<PathBuf, Vec<u8>>,
    _candidate: tempfile::TempDir,
}
fn refs(entity: &Entity) -> Vec<String> {
    let mut entity = entity.clone();
    cad_model::dimension_anchors_mut(&mut entity)
        .into_iter()
        .filter_map(|anchor| {
            if let cad_model::DimensionAnchor::Entity { entity_id, .. } = anchor {
                Some(entity_id.as_str().to_owned())
            } else {
                None
            }
        })
        .collect()
}

pub fn plan(project: &Path, drawing: &str, ids: &[String]) -> Result<StagePlan> {
    if ids.is_empty() {
        return Err(invalid("Select at least one entity to stage"));
    }
    let project = fs::canonicalize(project)?;
    let repo = repository_root(&project)?;
    let index_path = PathBuf::from(git_text(
        &repo,
        &["rev-parse", "--path-format=absolute", "--git-path", "index"],
    )?);
    let index_bytes = fs::read(&index_path)?;
    let index = snapshot(&project, &Revision::Index)?;
    let work = snapshot(&project, &Revision::Worktree)?;
    if fs::read(&index_path)? != index_bytes {
        return Err(invalid("Index changed while preparing stage plan"));
    }
    let before = index
        .source
        .drawings
        .iter()
        .find(|d| d.name == drawing)
        .ok_or_else(|| invalid("Drawing must already exist in the index"))?;
    let after = work
        .source
        .drawings
        .iter()
        .find(|d| d.name == drawing)
        .ok_or_else(|| invalid("Drawing was deleted; use whole-project staging"))?;
    let before_map: BTreeMap<_, _> = before
        .entities
        .iter()
        .map(|r| (r.entity.id().as_str().to_owned(), &r.entity))
        .collect();
    let after_map: BTreeMap<_, _> = after
        .entities
        .iter()
        .map(|r| (r.entity.id().as_str().to_owned(), &r.entity))
        .collect();
    let mut selected: BTreeSet<_> = ids.iter().cloned().collect();
    if selected
        .iter()
        .any(|id| !before_map.contains_key(id) && !after_map.contains_key(id))
    {
        return Err(invalid("Stage selection includes missing entities"));
    }
    loop {
        let mut next = selected.clone();
        for id in &selected {
            if let Some(entity) = after_map.get(id) {
                next.extend(refs(entity));
            }
        }
        // A selected/deleted target can invalidate a previously staged dimension.
        // Bring its worktree update or deletion into the same checked candidate.
        for (id, entity) in before_map.iter().chain(after_map.iter()) {
            if refs(entity)
                .iter()
                .any(|reference| selected.contains(reference))
            {
                next.insert(id.clone());
            }
        }
        if next == selected {
            break;
        }
        selected = next;
    }
    let mut candidate = index.source.clone();
    let mut definitions = BTreeSet::new();
    for id in &selected {
        if let Some(entity) = after_map.get(id) {
            copy_dependencies(&mut candidate, &work.source, entity, &mut definitions)?;
        }
    }
    let directory = tempfile::tempdir()?;
    let mut all_files: BTreeMap<_, _> = source_files(&index.source.root)?.into_iter().collect();
    let drawing_path = PathBuf::from(format!("drawings/{drawing}/entities.ndjson"));
    let working_bytes = fs::read(work.source.root.join(&drawing_path))?;
    let original = all_files
        .get(&drawing_path)
        .ok_or_else(|| invalid("Indexed drawing source is missing"))?;
    all_files.insert(
        drawing_path,
        selected_entities(original, &working_bytes, &selected)?,
    );
    if candidate.layers != index.source.layers {
        all_files.insert(
            "rules/layers.toml".into(),
            toml::to_string_pretty(&candidate.layers)
                .map_err(invalid)?
                .into_bytes(),
        );
    }
    if candidate.styles != index.source.styles {
        all_files.insert(
            "rules/styles.toml".into(),
            toml::to_string_pretty(&candidate.styles)
                .map_err(invalid)?
                .into_bytes(),
        );
    }
    for name in definitions {
        for leaf in ["definition.toml", "entities.ndjson"] {
            let relative = PathBuf::from(format!("blocks/{name}/{leaf}"));
            all_files.insert(relative.clone(), fs::read(work.source.root.join(relative))?);
        }
    }
    let original_files: BTreeMap<_, _> = source_files(&index.source.root)?.into_iter().collect();
    let files: BTreeMap<_, _> = all_files
        .iter()
        .filter(|(name, bytes)| original_files.get(*name) != Some(*bytes))
        .map(|(name, bytes)| (name.clone(), bytes.clone()))
        .collect();
    for (relative, bytes) in &all_files {
        let path = directory.path().join(relative);
        fs::create_dir_all(path.parent().unwrap())?;
        fs::write(path, bytes)?;
    }
    let cad_check = cad_check::check_project(directory.path());
    let checked = cad_model::load_project(directory.path())?;
    let diff = cad_diff::diff_projects(&index.source, &checked);
    let preview_svg = cad_diff::render_diff_svg(
        &index.source,
        &checked,
        &cad_diff::diff_selected_drawing(&index.source, &checked, Some(drawing)),
    );
    let mut hash = blake3::Hasher::new();
    hash.update(blake3::hash(&index_bytes).as_bytes());
    hash.update(work.identity.snapshot_blake3.as_bytes());
    hash.update(
        serde_json::to_string(&(drawing, &selected))
            .map_err(invalid)?
            .as_bytes(),
    );
    for (relative, bytes) in &files {
        hash.update(relative.to_string_lossy().as_bytes());
        hash.update(blake3::hash(bytes).as_bytes());
    }
    let report = StageReport {
        schema_version: "cad-stage/1".into(),
        plan_hash: hash.finalize().to_hex().to_string(),
        drawing: drawing.into(),
        requested_ids: ids.to_vec(),
        expanded_ids: selected.into_iter().collect(),
        changed_files: files
            .keys()
            .map(|p| p.to_string_lossy().into_owned())
            .collect(),
        index: index.identity,
        worktree: work.identity,
        diff,
        cad_check,
        applied: false,
    };
    Ok(StagePlan {
        report,
        preview_svg,
        project,
        index_path,
        index_bytes,
        files,
        _candidate: directory,
    })
}

fn selected_entities(index: &[u8], work: &[u8], ids: &BTreeSet<String>) -> Result<Vec<u8>> {
    let decode = |bytes: &[u8]| -> Result<Vec<(String, Vec<u8>)>> {
        bytes
            .split(|b| *b == b'\n')
            .filter(|line| !line.iter().all(|b| b.is_ascii_whitespace()))
            .map(|line| {
                let v: serde_json::Value = serde_json::from_slice(line).map_err(invalid)?;
                Ok((
                    v["id"]
                        .as_str()
                        .ok_or_else(|| invalid("Entity ID is missing"))?
                        .into(),
                    line.strip_suffix(b"\r").unwrap_or(line).to_vec(),
                ))
            })
            .collect()
    };
    let original = decode(index)?;
    let working = decode(work)?;
    let working_map: BTreeMap<_, _> = working.iter().map(|(id, bytes)| (id, bytes)).collect();
    let mut included = BTreeSet::new();
    let mut lines = Vec::new();
    for (id, bytes) in original {
        if ids.contains(&id) {
            if let Some(bytes) = working_map.get(&id) {
                lines.push((*bytes).clone());
                included.insert(id);
            }
        } else {
            lines.push(bytes);
            included.insert(id);
        }
    }
    for (id, bytes) in working {
        if ids.contains(&id) && included.insert(id) {
            lines.push(bytes);
        }
    }
    let ending = if index.windows(2).any(|v| v == b"\r\n") {
        b"\r\n".as_slice()
    } else {
        b"\n".as_slice()
    };
    let mut output = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        output.extend(line);
        if i + 1 < lines.len() || index.ends_with(b"\n") {
            output.extend(ending);
        }
    }
    Ok(output)
}

fn copy_dependencies(
    candidate: &mut ProjectSource,
    work: &ProjectSource,
    entity: &Entity,
    blocks: &mut BTreeSet<String>,
) -> Result<()> {
    let color = |candidate: &mut ProjectSource, id: &str| -> Result<()> {
        candidate.styles.colors.insert(
            id.into(),
            work.styles
                .colors
                .get(id)
                .ok_or_else(|| invalid("Missing worktree color"))?
                .clone(),
        );
        Ok(())
    };
    let line_type = |candidate: &mut ProjectSource, id: &str| -> Result<()> {
        candidate.styles.line_types.insert(
            id.into(),
            work.styles
                .line_types
                .get(id)
                .ok_or_else(|| invalid("Missing worktree line type"))?
                .clone(),
        );
        Ok(())
    };
    let layer = work
        .layers
        .layers
        .get(entity.layer())
        .ok_or_else(|| invalid("Missing worktree layer"))?;
    candidate
        .layers
        .layers
        .insert(entity.layer().into(), layer.clone());
    color(candidate, &layer.color)?;
    line_type(candidate, &layer.line_type)?;
    if let Some(group) = &layer.group {
        candidate.layers.groups.insert(
            group.clone(),
            work.layers
                .groups
                .get(group)
                .ok_or_else(|| invalid("Missing worktree group"))?
                .clone(),
        );
    }
    if let Some(id) = entity.pen() {
        let pen = work
            .styles
            .pens
            .get(id)
            .ok_or_else(|| invalid("Missing worktree pen"))?;
        candidate.styles.pens.insert(id.into(), pen.clone());
        color(candidate, &pen.color)?;
        line_type(candidate, &pen.line_type)?;
    }
    let text = |candidate: &mut ProjectSource, id: &str| -> Result<()> {
        candidate.styles.text_styles.insert(
            id.into(),
            work.styles
                .text_styles
                .get(id)
                .ok_or_else(|| invalid("Missing worktree text style"))?
                .clone(),
        );
        Ok(())
    };
    match entity {
        Entity::Text { style, .. } => text(candidate, style)?,
        Entity::Dimension { style, .. } => {
            let definition = work
                .styles
                .dimension_styles
                .get(style)
                .ok_or_else(|| invalid("Missing worktree dimension style"))?;
            candidate
                .styles
                .dimension_styles
                .insert(style.clone(), definition.clone());
            text(candidate, &definition.text_style)?;
        }
        Entity::Solid { fill, .. }
        | Entity::CurveSolid { fill, .. }
        | Entity::Hatch {
            fill: Some(fill), ..
        } => color(candidate, fill)?,
        Entity::BlockRef { block, .. } if blocks.insert(block.clone()) => {
            let definition = work
                .blocks
                .get(block)
                .ok_or_else(|| invalid("Missing worktree block"))?;
            candidate.blocks.insert(block.clone(), definition.clone());
            for child in &definition.entities {
                copy_dependencies(candidate, work, &child.entity, blocks)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// `expected_plan` must come from the reviewed plan. Only the Git index changes.
pub fn apply(plan: &mut StagePlan, expected_plan: &str) -> Result<()> {
    if expected_plan != plan.report.plan_hash {
        return Err(invalid("Stage plan hash differs; review a new plan"));
    }
    if !plan.report.cad_check.is_ok() {
        return Err(invalid("Stage candidate has CAD checker errors"));
    }
    if plan.files.is_empty() {
        return Err(invalid("Stage plan has no changes"));
    }
    let lock_path = plan.index_path.with_extension("lock");
    let mut lock = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)?;
    struct Lock(Option<PathBuf>);
    impl Drop for Lock {
        fn drop(&mut self) {
            if let Some(path) = &self.0 {
                let _ = fs::remove_file(path);
            }
        }
    }
    let mut guard = Lock(Some(lock_path.clone()));
    if fs::read(&plan.index_path)? != plan.index_bytes {
        return Err(invalid("Index changed after stage preview"));
    }
    if snapshot(&plan.project, &Revision::Worktree)?
        .identity
        .snapshot_blake3
        != plan.report.worktree.snapshot_blake3
    {
        return Err(invalid("Working sources changed after stage preview"));
    }
    let repo = repository_root(&plan.project)?;
    let relative = plan.project.strip_prefix(&repo).map_err(invalid)?;
    let temp = tempfile::tempdir_in(plan.index_path.parent().unwrap())?;
    let index = temp.path().join("index");
    fs::write(&index, &plan.index_bytes)?;
    let mut updates = Vec::new();
    for (name, bytes) in &plan.files {
        let oid = git_input(&repo, None, &["hash-object", "-w", "--stdin"], bytes)?;
        let path = relative.join(name);
        let path = path
            .to_str()
            .ok_or_else(|| invalid("Git staging path is not UTF8"))?;
        let mode = git_text(
            &repo,
            &["ls-files", "--stage", "--", &format!(":(literal){path}")],
        )?
        .split_whitespace()
        .next()
        .map(str::to_owned)
        .unwrap_or_else(|| "100644".into());
        if !matches!(mode.as_str(), "100644" | "100755") {
            return Err(invalid("Unsupported index mode"));
        }
        updates.extend(format!("{mode} {}\t{path}", oid.trim()).as_bytes());
        updates.push(0);
    }
    git_input(
        &repo,
        Some(&index),
        &["update-index", "-z", "--index-info"],
        &updates,
    )?;
    let output = fs::read(&index)?;
    if fs::read(&plan.index_path)? != plan.index_bytes
        || snapshot(&plan.project, &Revision::Worktree)?
            .identity
            .snapshot_blake3
            != plan.report.worktree.snapshot_blake3
    {
        return Err(invalid(
            "Sources or index changed while preparing stage transaction",
        ));
    }
    lock.write_all(&output)?;
    lock.sync_all()?;
    drop(lock);
    guard.0 = None;
    if let Err(error) = fs::rename(&lock_path, &plan.index_path) {
        let _ = fs::remove_file(&lock_path);
        return Err(error.into());
    }
    plan.report.applied = true;
    Ok(())
}
fn git_input(repo: &Path, index: Option<&Path>, args: &[&str], input: &[u8]) -> Result<String> {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(repo)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    let mut child = command.spawn()?;
    child.stdin.take().unwrap().write_all(input)?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(invalid(String::from_utf8_lossy(&output.stderr)));
    }
    String::from_utf8(output.stdout).map_err(invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn git(root: &Path, args: &[&str]) -> String {
        git_text(root, args).unwrap()
    }
    fn setup() -> (tempfile::TempDir, PathBuf, Vec<serde_json::Value>) {
        let temp = tempfile::tempdir().unwrap();
        let project = cad_edit::create_project(&cad_edit::ProjectTemplateRequest {
            parent_dir: temp.path().display().to_string(),
            folder_name: "repo".into(),
            project_name: "stage".into(),
            drawing: "plan".into(),
            paper: "A3".into(),
            orientation: cad_model::SheetOrientation::Landscape,
            scale_denominator: 100,
        })
        .unwrap();
        let root = PathBuf::from(project.project_path);
        let entities:Vec<_>=(1..=3).map(|i|json!({"schema_version":"0.3","id":format!("ent_{i:026}"),"type":"line","layer":"0-1","p1":[0,i*100],"p2":[100,i*100]})).collect();
        write(&root, &entities);
        git(&root, &["init"]);
        git(&root, &["config", "user.name", "CAD test"]);
        git(&root, &["config", "user.email", "cad@example.invalid"]);
        git(&root, &["add", "."]);
        git(&root, &["commit", "-m", "baseline"]);
        (temp, root, entities)
    }
    fn write(root: &Path, entities: &[serde_json::Value]) {
        fs::write(
            root.join("drawings/plan/entities.ndjson"),
            entities
                .iter()
                .map(|v| format!("{v}\n"))
                .collect::<String>(),
        )
        .unwrap();
    }
    #[test]
    fn selective_stage_preserves_other_staged_entities_and_files_without_worktree_edits() {
        let (_temp, root, mut values) = setup();
        values[2]["p2"] = json!([222, 300]);
        write(&root, &values);
        fs::write(root.join("note.txt"), "staged note").unwrap();
        git(&root, &["add", "."]);
        values[0]["p2"] = json!([111, 100]);
        values[1]["p2"] = json!([333, 200]);
        values[2]["p2"] = json!([444, 300]);
        write(&root, &values);
        fs::write(root.join("note.txt"), "working note").unwrap();
        let before = fs::read(root.join("drawings/plan/entities.ndjson")).unwrap();
        let ids = vec![values[0]["id"].as_str().unwrap().to_owned()];
        let mut candidate = plan(&root, "plan", &ids).unwrap();
        assert!(candidate.report.cad_check.is_ok());
        let stable = plan(&root, "plan", &ids).unwrap();
        assert_eq!(candidate.report.plan_hash, stable.report.plan_hash);
        let expected = candidate.report.plan_hash.clone();
        apply(&mut candidate, &expected).unwrap();
        assert!(candidate.report.applied);
        let indexed = snapshot(&root, &Revision::Index).unwrap();
        let endpoints: Vec<_> = indexed.source.drawings[0]
            .entities
            .iter()
            .map(|r| {
                let Entity::Line { p2, .. } = r.entity else {
                    panic!()
                };
                p2[0]
            })
            .collect();
        assert_eq!(endpoints, vec![111.0, 100.0, 222.0]);
        assert_eq!(git(&root, &["show", ":note.txt"]), "staged note");
        assert_eq!(
            fs::read(root.join("drawings/plan/entities.ndjson")).unwrap(),
            before
        );
        assert_eq!(
            fs::read_to_string(root.join("note.txt")).unwrap(),
            "working note"
        );
        assert!(apply(&mut candidate, &expected).is_err());
    }
    #[test]
    fn preview_conflicts_and_existing_index_locks_never_change_the_index() {
        let (_temp, root, mut values) = setup();
        values[0]["p2"] = json!([200, 100]);
        write(&root, &values);
        let ids = vec![values[0]["id"].as_str().unwrap().to_owned()];
        let mut candidate = plan(&root, "plan", &ids).unwrap();
        let expected = candidate.report.plan_hash.clone();
        let before = fs::read(&candidate.index_path).unwrap();
        assert!(apply(&mut candidate, "wrong hash").is_err());
        let lock = candidate.index_path.with_extension("lock");
        fs::write(&lock, "another writer").unwrap();
        assert!(apply(&mut candidate, &expected).is_err());
        assert_eq!(fs::read_to_string(&lock).unwrap(), "another writer");
        fs::remove_file(lock).unwrap();
        values[0]["p2"] = json!([300, 100]);
        write(&root, &values);
        assert!(apply(&mut candidate, &expected).is_err());
        assert_eq!(fs::read(&candidate.index_path).unwrap(), before);
        let mut fresh = plan(&root, "plan", &ids).unwrap();
        let expected = fresh.report.plan_hash.clone();
        fs::write(root.join("other.txt"), "new stage").unwrap();
        git(&root, &["add", "other.txt"]);
        let changed = fs::read(&fresh.index_path).unwrap();
        assert!(apply(&mut fresh, &expected).is_err());
        assert_eq!(fs::read(&fresh.index_path).unwrap(), changed);
    }
    #[test]
    fn dimension_closure_and_invalid_candidates_are_reviewed_before_staging() {
        let (_temp, root, mut values) = setup();
        let source = cad_model::load_project(&root).unwrap();
        let style = source.styles.dimension_styles.keys().next().unwrap();
        let id = values[0]["id"].as_str().unwrap().to_owned();
        let dimension_id = "ent_01JZ0000000000000000000555";
        values.push(json!({"schema_version":"0.3","id":dimension_id,"type":"dimension","layer":"0-1","style":style,"p1":[0,100],"p2":[100,100],"offset":100,"measurement":{"kind":"horizontal","first":{"kind":"entity","entity_id":id,"feature":"start"},"second":{"kind":"entity","entity_id":id,"feature":"end"}}}));
        write(&root, &values);
        git(&root, &["add", "."]);
        git(&root, &["commit", "-m", "dimension"]);
        values.remove(0);
        write(&root, &values);
        let mut invalid_plan = plan(&root, "plan", std::slice::from_ref(&id)).unwrap();
        assert!(
            invalid_plan
                .report
                .expanded_ids
                .contains(&dimension_id.to_owned())
        );
        assert!(!invalid_plan.report.cad_check.is_ok());
        let before = fs::read(&invalid_plan.index_path).unwrap();
        let hash = invalid_plan.report.plan_hash.clone();
        assert!(apply(&mut invalid_plan, &hash).is_err());
        assert_eq!(fs::read(&invalid_plan.index_path).unwrap(), before);
        values.pop();
        write(&root, &values);
        let mut candidate = plan(&root, "plan", &[id]).unwrap();
        assert!(candidate.report.cad_check.is_ok());
        let hash = candidate.report.plan_hash.clone();
        apply(&mut candidate, &hash).unwrap();
        assert_eq!(
            snapshot(&root, &Revision::Index).unwrap().source.drawings[0]
                .entities
                .len(),
            2
        );
    }

    #[test]
    fn newly_referenced_block_styles_are_staged_but_unrelated_definitions_are_kept() {
        let (_temp, root, mut values) = setup();
        let styles_path = root.join("rules/styles.toml");
        let mut styles = cad_model::load_project(&root).unwrap().styles;
        styles.colors.insert(
            "new_red".into(),
            cad_model::ColorDef {
                rgb: "#FF0000".into(),
                print_rgb: None,
                print_width: 0.25,
            },
        );
        styles.colors.insert(
            "unrelated_blue".into(),
            cad_model::ColorDef {
                rgb: "#0000FF".into(),
                print_rgb: None,
                print_width: 0.25,
            },
        );
        styles.pens.insert(
            "new_pen".into(),
            cad_model::PenStyleDef {
                color: "new_red".into(),
                line_type: "solid".into(),
                line_width: 0.25,
            },
        );
        fs::write(&styles_path, toml::to_string_pretty(&styles).unwrap()).unwrap();
        let block = root.join("blocks/new_part");
        fs::create_dir_all(&block).unwrap();
        fs::write(
            block.join("definition.toml"),
            "schema_version = \"0.3\"\nname = \"new part\"\nbase_point = [0.0, 0.0]\n",
        )
        .unwrap();
        fs::write(block.join("entities.ndjson"),format!("{}\n",json!({"schema_version":"0.3","id":"ent_01JZ0000000000000000000777","type":"line","layer":"0-1","pen":"new_pen","p1":[0,0],"p2":[100,0]}))).unwrap();
        let id = "ent_01JZ0000000000000000000888".to_owned();
        values.push(json!({"schema_version":"0.3","id":id,"type":"block_ref","layer":"0-1","block":"new_part","at":[500,500],"rotation_deg":0,"scale":1}));
        write(&root, &values);
        let mut candidate = plan(&root, "plan", &[id]).unwrap();
        assert!(
            candidate.report.cad_check.is_ok(),
            "{:?}",
            candidate.report.cad_check
        );
        let hash = candidate.report.plan_hash.clone();
        apply(&mut candidate, &hash).unwrap();
        let indexed = snapshot(&root, &Revision::Index).unwrap();
        assert!(indexed.source.blocks.contains_key("new_part"));
        assert!(indexed.source.styles.colors.contains_key("new_red"));
        assert!(!indexed.source.styles.colors.contains_key("unrelated_blue"));
        assert!(cad_check::check_project(&indexed.source.root).is_ok());
    }
}
