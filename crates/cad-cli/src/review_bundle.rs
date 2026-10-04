//! Review artifacts are generated from frozen byte snapshots, including invalid
//! sources. A blocked report is retained; the original repository is read-only.
use miette::{IntoDiagnostic, Result, miette};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

pub fn export(
    project: &Path,
    base_revision: &str,
    head_revision: &str,
    directory: &Path,
) -> Result<()> {
    validate_output_directory(project, directory)?;
    let base = cad_git::snapshot_files(project, &cad_git::Revision::parse(base_revision));
    let head = cad_git::snapshot_files(project, &cad_git::Revision::parse(head_revision));
    if let Some(parent) = directory.parent().filter(|p| !p.as_os_str().is_empty()) {
        fs::create_dir_all(parent).into_diagnostic()?;
    }
    fs::create_dir(directory).into_diagnostic()?;
    let mut files = Vec::new();
    let mut errors = Vec::new();
    let mut inputs = Vec::new();
    let mut sources = Vec::new();
    let mut snapshot_owners = Vec::new();
    let mut pages = Vec::new();
    let mut annotation_files = Vec::new();
    let mut annotation_warnings = Vec::new();
    for (role, requested, result) in [("base", base_revision, base), ("head", head_revision, head)]
    {
        let frozen = match result {
            Ok(snapshot) => snapshot,
            Err(error) => {
                errors.push(format!("{role}: could not freeze input: {error}"));
                inputs.push(json!({"role":role,"requested_revision":requested,"source":null,"cad_check":null}));
                sources.push(None);
                continue;
            }
        };
        let check = cad_check::check_project(&frozen.root);
        let mut compatibility = Vec::new();
        let source = if check.is_ok() {
            match cad_model::load_project(&frozen.root) {
                Ok(source) => Some(source),
                Err(error) => {
                    errors.push(format!("{role}: {error}"));
                    None
                }
            }
        } else {
            errors.push(format!("{role}: CAD source validation failed"));
            None
        };
        for (ordinal, source_file) in frozen
            .identity
            .source_manifest
            .iter()
            .filter(|f| {
                cad_model::classify_project_source_path(Path::new(&f.relative_path))
                    == Some(cad_model::ProjectSourceKind::Comment)
            })
            .enumerate()
        {
            let bytes = fs::read(frozen.root.join(&source_file.relative_path)).into_diagnostic()?;
            let artifact = format!("{role}.{:03}.comments.ndjson", ordinal + 1);
            let mut warnings = Vec::new();
            let mut version_checks = Vec::new();
            let source_revision = source
                .as_ref()
                .map(cad_model::comment_source_revision)
                .transpose()
                .into_diagnostic()?;
            match std::str::from_utf8(&bytes) {
                Ok(text) => {
                    for (line, record) in text
                        .lines()
                        .enumerate()
                        .filter(|(_, r)| !r.trim().is_empty())
                    {
                        match serde_json::from_str::<Value>(record) {
                            Err(error) => warnings.push(format!("line {}: {error}", line + 1)),
                            Ok(value) => {
                                let binding = value
                                    .get("binding")
                                    .filter(|v| !v.is_null())
                                    .map(|v| {
                                        serde_json::from_value::<cad_model::CommentBinding>(
                                            v.clone(),
                                        )
                                    })
                                    .transpose();
                                let state = match binding {
                                    Err(error) => {
                                        warnings.push(format!(
                                            "line {}: invalid version metadata: {error}",
                                            line + 1
                                        ));
                                        json!("invalid")
                                    }
                                    Ok(binding) => {
                                        if let (Some(project), Some(revision)) =
                                            (&source, &source_revision)
                                        {
                                            let ids = value["entity_ids"]
                                                .as_array()
                                                .into_iter()
                                                .flatten()
                                                .map(|v| v.as_str().map(str::to_owned))
                                                .collect::<Option<Vec<_>>>()
                                                .unwrap_or_default();
                                            let state = cad_model::evaluate_comment_binding(
                                                project,
                                                value["drawing"].as_str().unwrap_or(""),
                                                &ids,
                                                binding.as_ref(),
                                                revision,
                                            );
                                            if !matches!(
                                                state,
                                                cad_model::CommentBindingState::Current
                                                    | cad_model::CommentBindingState::Unbound
                                            ) {
                                                warnings.push(format!(
                                                    "line {}: recorded comment version is {:?}",
                                                    line + 1,
                                                    state
                                                ));
                                            }
                                            json!(state)
                                        } else if binding.is_none() {
                                            json!("unbound")
                                        } else {
                                            json!("source_unavailable")
                                        }
                                    }
                                };
                                version_checks.push(
                                    json!({"line":line+1,"comment_id":value["id"],"state":state}),
                                );
                            }
                        }
                    }
                }
                Err(error) => warnings.push(format!("not UTF-8: {error}")),
            }
            annotation_warnings.extend(
                warnings
                    .iter()
                    .map(|warning| format!("{role} {}: {warning}", source_file.relative_path)),
            );
            annotation_files.push(json!({"role":role,"source_path":source_file.relative_path,"artifact":artifact,"validation":"JSON syntax and recorded version metadata; full comment schema and anchor placement are not validated","warnings":warnings,"version_checks":version_checks}));
            files.push(publish(directory, &artifact, &bytes)?);
        }
        if let Some(source) = &source {
            for drawing in &source.drawings {
                compatibility.push(json!({"drawing":drawing.name,"target":"jww-v600","check":cad_check::check_project_for_target(&frozen.root,cad_check::CheckTarget::JwwV600,Some(&drawing.name))}));
            }
            if let Err(error) = render_side(directory, role, source, &mut files, &mut pages) {
                errors.push(format!("{role}: rendering failed: {error}"));
            }
            let configurations = json!({"project":source.project,"layers":source.layers,"styles":source.styles,
                "drawings":source.drawings.iter().map(|d| json!({"name":d.name,"layouts":d.layouts})).collect::<Vec<_>>(),
                "blocks":source.blocks.iter().map(|(id,b)| json!({"id":id,"definition":b.config})).collect::<Vec<_>>()});
            files.push(publish(
                directory,
                &format!("{role}.configuration.json"),
                &serde_json::to_vec_pretty(&configurations).into_diagnostic()?,
            )?);
        }
        inputs.push(json!({"role":role,"requested_revision":requested,"source":frozen.identity,"cad_check":check,"compatibility_checks":compatibility}));
        sources.push(source);
        snapshot_owners.push(frozen); // Keep files alive through dependency-diff rendering.
    }
    let mut diff_report = None;
    let mut diff_pages = Vec::new();
    if let [Some(base), Some(head)] = sources.as_slice() {
        let diff = cad_diff::diff_projects(base, head);
        files.push(publish(
            directory,
            "diff.json",
            &serde_json::to_vec_pretty(&diff).into_diagnostic()?,
        )?);
        let names: BTreeSet<_> = base
            .drawings
            .iter()
            .chain(head.drawings.iter())
            .map(|d| d.name.as_str())
            .collect();
        for (ordinal, name) in names.into_iter().enumerate() {
            let report = cad_diff::diff_selected_drawing(base, head, Some(name));
            let artifact = format!("{:03}.diff.svg", ordinal + 1);
            files.push(publish(
                directory,
                &artifact,
                cad_diff::render_diff_svg(base, head, &report).as_bytes(),
            )?);
            diff_pages.push(json!({"drawing":name,"artifact":artifact}));
        }
        diff_report = Some(diff);
    }
    let complete = errors.is_empty();
    let status = if complete { "complete" } else { "blocked" };
    let report = json!({"schema_version":"cad-review/1","status":status,"inputs":inputs,"pages":pages,"diff_pages":diff_pages,
        "diff":diff_report,"annotation_files":annotation_files,"annotation_warnings":annotation_warnings,"errors":errors,
        "verification":{"receiving_cad_checked":false,"compatibility":"JWW target checks only; no JWW bytes are exported or certified by this bundle","annotation_versions":"recorded bindings are evaluated against each pinned source; legacy comments are unbound; the source identity covers the whole normalized canonical project","hashes":"file integrity only, not authentication"}});
    files.push(publish(
        directory,
        "review.json",
        &serde_json::to_vec_pretty(&report).into_diagnostic()?,
    )?);
    let mut markdown = format!(
        "# CAD review\n\nStatus: **{status}**. [Full report and warnings](review.json).\n\nAvailable inputs are frozen without a checkout. All drawings and all named layouts are included when their CAD checks pass.\n\n"
    );
    for input in &inputs {
        markdown.push_str(&format!(
            "- {}: requested {}, commit {}, snapshot {}.\n",
            input["role"].as_str().unwrap_or(""),
            md(input["requested_revision"].as_str().unwrap_or("")),
            md(input["source"]["commit_oid"]
                .as_str()
                .unwrap_or("unavailable")),
            md(input["source"]["snapshot_blake3"]
                .as_str()
                .unwrap_or("unavailable"))
        ));
    }
    markdown.push('\n');
    for role in ["base", "head"] {
        if files.iter().any(|f| f["path"] == format!("{role}.pdf")) {
            markdown.push_str(&format!("- [{role} PDF: all layouts]({role}.pdf)\n"));
        }
    }
    for page in &pages {
        markdown.push_str(&format!(
            "- {} / {} / {}: [SVG]({})\n",
            md(page["role"].as_str().unwrap_or("")),
            md(page["drawing"].as_str().unwrap_or("")),
            md(page["layout"].as_str().unwrap_or("")),
            page["artifact"].as_str().unwrap_or("")
        ));
    }
    for page in &diff_pages {
        markdown.push_str(&format!(
            "- {}: [semantic diff SVG]({})\n",
            md(page["drawing"].as_str().unwrap_or("")),
            page["artifact"].as_str().unwrap_or("")
        ));
    }
    markdown.push_str("\nShared block and reference-dimension effects are included in the semantic diff. Configuration snapshots and each version's unchanged raw comment files are listed in the manifest. Recorded comment versions are evaluated against each pinned canonical source; legacy comments remain unbound. Full comment schema and anchor placement are not validated. JWW target warnings are retained in the report; receiving-CAD behavior has not been checked. Curved or font approximations must be reviewed separately. Hash verification detects changed artifacts, not a forged manifest.\n");
    for error in &errors {
        markdown.push_str(&format!("\nBlocked: {}\n", md(error)));
    }
    files.push(publish(directory, "review.md", markdown.as_bytes())?);
    let manifest = json!({"schema_version":"cad-review-bundle/1","status":status,"tool_version":env!("CARGO_PKG_VERSION"),"inputs":inputs,"files":files});
    publish(
        directory,
        "manifest.json",
        &serde_json::to_vec_pretty(&manifest).into_diagnostic()?,
    )?;
    if !complete {
        return Err(miette!(
            "review is blocked; checks and available artifacts retained at {}",
            directory.display()
        ));
    }
    println!("Review bundle generated at {}", directory.display());
    Ok(())
}
fn validate_output_directory(project: &Path, directory: &Path) -> Result<()> {
    let resolved = super::resolve_output_path(directory)?;
    let root = fs::canonicalize(project).into_diagnostic()?;
    if let Ok(relative) = resolved.strip_prefix(&root) {
        let first = relative.components().next().map(|c| c.as_os_str());
        if first.is_none()
            || [
                "rules",
                "drawings",
                "blocks",
                "comments",
                "interop",
                ".cad-history",
                ".git",
            ]
            .iter()
            .any(|name| first == Some(std::ffi::OsStr::new(name)))
            || relative
                .components()
                .any(|c| c.as_os_str().to_string_lossy().starts_with(".cad-"))
        {
            return Err(miette!(
                "Review directory cannot be inside canonical sources, provenance or app-managed recovery/history"
            ));
        }
    }
    if cad_git::metadata_directories(&root)
        .unwrap_or_default()
        .iter()
        .any(|path| resolved.starts_with(path))
    {
        return Err(miette!("Review directory cannot be inside Git metadata"));
    }
    Ok(())
}

fn render_side(
    directory: &Path,
    role: &str,
    source: &cad_model::ProjectSource,
    files: &mut Vec<Value>,
    pages: &mut Vec<Value>,
) -> Result<()> {
    let selections: Vec<_> = source
        .drawings
        .iter()
        .flat_map(|drawing| {
            drawing
                .layouts
                .layouts
                .keys()
                .map(move |layout| (drawing.name.as_str(), Some(layout.as_str())))
        })
        .collect();
    if !selections.is_empty() {
        files.push(publish(
            directory,
            &format!("{role}.pdf"),
            &cad_render_pdf::render_drawings_pdf(source, &selections).into_diagnostic()?,
        )?);
    }
    for (ordinal, (drawing, layout)) in selections.iter().enumerate() {
        let mut view = source.clone();
        view.drawings
            .iter_mut()
            .find(|d| d.name == *drawing)
            .expect("selected drawing")
            .layouts
            .active_layout = layout.expect("selected layout").into();
        let svg = cad_render_svg::render_drawing_svg(&view, drawing).into_diagnostic()?;
        let artifact = format!("{role}.{:03}.svg", ordinal + 1);
        files.push(publish(directory, &artifact, svg.as_bytes())?);
        pages.push(json!({"role":role,"page":ordinal+1,"drawing":drawing,"layout":layout,"artifact":artifact}));
    }
    Ok(())
}
fn publish(directory: &Path, name: &str, bytes: &[u8]) -> Result<Value> {
    cad_edit::atomic_publish(&directory.join(name), bytes, false).into_diagnostic()?;
    Ok(json!({"path":name,"bytes":bytes.len(),"blake3":blake3::hash(bytes).to_hex().to_string()}))
}
fn md(text: &str) -> String {
    text.chars()
        .flat_map(|ch| match ch {
            '\n' | '\r' => vec![' '],
            '\\' | '`' | '*' | '_' | '[' | ']' | '(' | ')' | '#' | '!' | '|' | '<' | '>' => {
                vec!['\\', ch]
            }
            _ => vec![ch],
        })
        .collect()
}
pub fn verify(directory: &Path) -> Result<()> {
    super::export_set::verify_inventory(directory, "cad-review-bundle/1")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn git(root: &Path, values: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(values)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    fn project(temp: &Path) -> std::path::PathBuf {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/cad-acceptance");
        let frozen = cad_git::snapshot_files(&fixture, &cad_git::Revision::Worktree).unwrap();
        let root = temp.join("repo");
        fs::create_dir(&root).unwrap();
        for file in &frozen.identity.source_manifest {
            let target = root.join(&file.relative_path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(frozen.root.join(&file.relative_path), target).unwrap();
        }
        let second = root.join("drawings/second");
        fs::create_dir(&second).unwrap();
        fs::copy(
            root.join("drawings/acceptance/layouts.toml"),
            second.join("layouts.toml"),
        )
        .unwrap();
        fs::write(second.join("entities.ndjson"),"{\"schema_version\":\"0.3\",\"id\":\"ent_01JZ0000000000000000000950\",\"type\":\"block_ref\",\"layer\":\"0-1\",\"block\":\"door\",\"at\":[0,0],\"rotation_deg\":30,\"scale\":2}\n").unwrap();
        let path = root.join("drawings/acceptance/layouts.toml");
        let original = fs::read_to_string(&path).unwrap();
        fs::write(path,format!("{original}\n[layouts.detail]\nname=\"Detail\"\npaper=\"A4\"\norientation=\"portrait\"\nscale=\"1/100\"\norigin=[-3000.0,-2500.0]\n")).unwrap();
        fs::create_dir(root.join("comments")).unwrap();
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../examples/house-small/comments/plan_1f.ndjson"),
            root.join("comments/plan_1f.ndjson"),
        )
        .unwrap();
        let checked = cad_check::check_project(&root);
        assert!(checked.is_ok(), "{checked:?}");
        git(&root, &["init"]);
        git(&root, &["config", "user.name", "CAD Test"]);
        git(&root, &["config", "user.email", "cad@example.invalid"]);
        git(&root, &["add", "."]);
        git(&root, &["commit", "-m", "base"]);
        root
    }
    #[test]
    fn all_layouts_shared_dependencies_comments_and_hashes_are_reviewable() {
        let temp = tempfile::tempdir().unwrap();
        let root = project(temp.path());
        let entity_path = root.join("drawings/acceptance/entities.ndjson");
        let lines = fs::read_to_string(&entity_path)
            .unwrap()
            .lines()
            .map(|line| {
                let mut entity: Value = serde_json::from_str(line).unwrap();
                if entity["id"] == "ent_01JZ0000000000000000000100" {
                    entity["points"][1][0] = json!(5500);
                }
                entity.to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        fs::write(&entity_path, lines).unwrap();
        let block_path = root.join("blocks/door/entities.ndjson");
        let lines = fs::read_to_string(&block_path)
            .unwrap()
            .lines()
            .map(|line| {
                let mut entity: Value = serde_json::from_str(line).unwrap();
                if entity["type"] == "line" {
                    entity["p2"][1] = json!(1100);
                }
                entity.to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        fs::write(&block_path, lines).unwrap();
        let checked = cad_check::check_project(&root);
        assert!(checked.is_ok(), "{checked:?}");
        let before = cad_model::source_manifest(&root).unwrap();
        let index = fs::read(root.join(".git/index")).unwrap();
        let output = temp.path().join("review");
        export(&root, "HEAD", "worktree", &output).unwrap();
        verify(&output).unwrap();
        let report: Value =
            serde_json::from_slice(&fs::read(output.join("review.json")).unwrap()).unwrap();
        assert_eq!(report["status"], "complete");
        assert_eq!(report["pages"].as_array().unwrap().len(), 6);
        assert_eq!(report["diff_pages"].as_array().unwrap().len(), 2);
        let changes = report["diff"]["changes"].as_array().unwrap();
        for id in [
            "ent_01JZ0000000000000000000110",
            "ent_01JZ0000000000000000000120",
            "ent_01JZ0000000000000000000950",
        ] {
            assert!(changes.iter().any(|c| {
                c["entity_id"] == id
                    && c["reasons"]
                        .as_array()
                        .unwrap()
                        .contains(&json!("dependency_changed"))
            }));
        }
        assert_eq!(report["annotation_files"].as_array().unwrap().len(), 2);
        assert_eq!(
            fs::read(output.join("base.001.comments.ndjson")).unwrap(),
            fs::read(root.join("comments/plan_1f.ndjson")).unwrap()
        );
        assert!(
            !report["inputs"][1]["compatibility_checks"][0]["check"]["diagnostics"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(report["verification"]["receiving_cad_checked"], false);
        assert_eq!(cad_model::source_manifest(&root).unwrap(), before);
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
        assert!(export(&root, "HEAD", "worktree", &output).is_err());
        fs::write(output.join("001.diff.svg"), "tampered").unwrap();
        assert!(verify(&output).is_err());
    }
    #[test]
    fn invalid_canonical_bytes_retain_pinned_checks_and_block_publication() {
        let temp = tempfile::tempdir().unwrap();
        let root = project(temp.path());
        let entity_path = root.join("drawings/acceptance/entities.ndjson");
        fs::write(&entity_path, "{not json}\n").unwrap();
        let frozen = cad_git::snapshot_files(&root, &cad_git::Revision::Worktree).unwrap();
        assert_eq!(
            fs::read(frozen.root.join("drawings/acceptance/entities.ndjson")).unwrap(),
            b"{not json}\n"
        );
        let pinned_hash = frozen.identity.snapshot_blake3;
        let before = cad_model::source_manifest(&root).unwrap();
        let index = fs::read(root.join(".git/index")).unwrap();
        let output = temp.path().join("blocked");
        assert!(export(&root, "HEAD", "worktree", &output).is_err());
        let report: Value =
            serde_json::from_slice(&fs::read(output.join("review.json")).unwrap()).unwrap();
        assert_eq!(report["status"], "blocked");
        assert_eq!(
            report["inputs"][1]["source"]["snapshot_blake3"],
            pinned_hash
        );
        assert_eq!(report["inputs"][1]["cad_check"]["status"], "error");
        assert!(report["diff"].is_null());
        assert!(output.join("base.pdf").is_file());
        assert!(!output.join("head.pdf").exists());
        assert!(verify(&output).is_err());
        assert_eq!(cad_model::source_manifest(&root).unwrap(), before);
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
        let missing = temp.path().join("bad-reference");
        assert!(export(&root, "missing-ref", "HEAD", &missing).is_err());
        let report: Value =
            serde_json::from_slice(&fs::read(missing.join("review.json")).unwrap()).unwrap();
        assert!(!report["errors"].as_array().unwrap().is_empty());
        assert!(report["inputs"][0]["source"].is_null());
    }
    #[test]
    fn comment_versions_are_checked_against_each_pinned_source_without_rewriting_records() {
        let temp = tempfile::tempdir().unwrap();
        let root = project(temp.path());
        let source = cad_model::load_project(&root).unwrap();
        let entity = &source
            .drawings
            .iter()
            .find(|d| d.name == "acceptance")
            .unwrap()
            .entities
            .iter()
            .find(|r| matches!(r.entity, cad_model::Entity::Text { .. }))
            .unwrap();
        let id = entity.entity.id().as_str().to_owned();
        let binding = cad_model::bind_comment_entity(&source, "acceptance", &id, None).unwrap();
        let record = json!({"id":"bound-comment","drawing":"acceptance","entity_ids":[id],"binding":binding});
        let mut invalid = record.clone();
        invalid["id"] = json!("invalid-comment");
        invalid["binding"]["entity_blake3"] = json!("0".repeat(64));
        let bytes = format!("{record}\n{invalid}\n{{\"id\":\"legacy-comment\"}}\n");
        let comments = root.join("comments/acceptance.ndjson");
        fs::write(&comments, &bytes).unwrap();
        git(&root, &["add", "."]);
        git(&root, &["commit", "-m", "record original comment version"]);
        let entity_path = root.join("drawings/acceptance/entities.ndjson");
        let updated = fs::read_to_string(&entity_path)
            .unwrap()
            .lines()
            .filter(|line| serde_json::from_str::<Value>(line).unwrap()["id"] != id)
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        fs::write(entity_path, updated).unwrap();
        // The selected fixture entity must not be a referenced dependency.
        let checked = cad_check::check_project(&root);
        assert!(checked.is_ok(), "{checked:?}");
        let manifest = cad_model::source_manifest(&root).unwrap();
        let index = fs::read(root.join(".git/index")).unwrap();
        let output = temp.path().join("versions");
        export(&root, "HEAD", "worktree", &output).unwrap();
        verify(&output).unwrap();
        let report: Value =
            serde_json::from_slice(&fs::read(output.join("review.json")).unwrap()).unwrap();
        for (role, expected) in [("base", "current"), ("head", "entity_deleted")] {
            let annotations = report["annotation_files"]
                .as_array()
                .unwrap()
                .iter()
                .find(|f| f["role"] == role && f["source_path"] == "comments/acceptance.ndjson")
                .unwrap();
            let checks = annotations["version_checks"].as_array().unwrap();
            assert_eq!(checks[0]["state"], expected);
            assert_eq!(checks[1]["state"], "invalid");
            assert_eq!(checks[2]["state"], "unbound");
            assert!(!annotations["warnings"].as_array().unwrap().is_empty());
            assert_eq!(
                fs::read(output.join(annotations["artifact"].as_str().unwrap())).unwrap(),
                bytes.as_bytes()
            );
        }
        assert_eq!(fs::read(comments).unwrap(), bytes.as_bytes());
        assert_eq!(cad_model::source_manifest(&root).unwrap(), manifest);
        assert_eq!(fs::read(root.join(".git/index")).unwrap(), index);
    }
    #[test]
    fn reports_cannot_be_published_into_sources_or_git_metadata() {
        let temp = tempfile::tempdir().unwrap();
        let root = project(temp.path());
        for relative in [
            "rules/review",
            "drawings/review",
            "blocks/review",
            "comments/review",
            "interop/review",
            ".git/review",
            "build/.cad-recovery/review",
        ] {
            let output = root.join(relative);
            assert!(
                export(&root, "HEAD", "worktree", &output)
                    .unwrap_err()
                    .to_string()
                    .contains("Review directory")
            );
            assert!(!output.exists());
        }
        #[cfg(unix)]
        {
            let alias = root.join("alias");
            std::os::unix::fs::symlink(root.join("rules"), &alias).unwrap();
            assert!(export(&root, "HEAD", "worktree", &alias.join("review")).is_err());
            assert!(!root.join("rules/review").exists());
        }
    }
}
