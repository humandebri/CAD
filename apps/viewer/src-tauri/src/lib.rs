//! apps/viewer/src-tauri: desktop bridge for local CAD review artifacts.
//! The webview calls Rust commands; Rust calls CAD crates directly and uses Git only to read HEAD.

use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use tauri::{Manager, Runtime};

#[derive(Debug, Clone, Serialize)]
pub struct ProjectState {
    project_path: String,
    project_name: String,
    is_git_project: bool,
    import_warning_count: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentRecord {
    id: String,
    drawing: String,
    #[serde(default)]
    entity_ids: Vec<String>,
    text: String,
    status: String,
}

#[derive(Debug, Serialize)]
pub struct ReviewArtifacts {
    sheet_svg: String,
    diff_svg: Option<String>,
    check: cad_check::CheckReport,
    diff: Option<cad_diff::DiffReport>,
    comments: Vec<CommentRecord>,
    diff_unavailable: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AiContextStatus {
    Ready,
    NoEntitySelected,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AiContextState {
    status: AiContextStatus,
    json_path: Option<String>,
    markdown_path: Option<String>,
    message: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AiContext {
    schema_version: String,
    project_path: String,
    project_name: String,
    drawing: String,
    view_mode: String,
    selected_entity_id: String,
    source: AiContextSource,
    entity: serde_json::Value,
    bbox: Option<AiContextBBox>,
    check_diagnostics: Vec<cad_check::CheckDiagnostic>,
    diff_changes: Vec<cad_diff::DiffChange>,
    diff_warnings: Vec<cad_diff::DiffWarning>,
    comments: Vec<CommentRecord>,
    generated_at: String,
}

#[derive(Debug, Serialize)]
pub struct AiContextSource {
    path: String,
    line: usize,
    raw: String,
}

#[derive(Debug, Serialize)]
pub struct AiContextBBox {
    min: [f64; 2],
    max: [f64; 2],
}

#[tauri::command]
fn open_project(project_path: String) -> Result<ProjectState, String> {
    open_project_state(Path::new(&project_path))
}

#[tauri::command]
fn run_review(project_path: String) -> Result<ReviewArtifacts, String> {
    run_review_for_path(Path::new(&project_path))
}

#[tauri::command]
fn import_jww(jww_path: String, out_dir: String) -> Result<ProjectState, String> {
    let report = cad_import_jww::import_jww_file(&jww_path, &out_dir)
        .map_err(|error| format!("failed to import JWW: {error}"))?;
    let mut state = open_project_state(Path::new(&out_dir))?;
    state.import_warning_count = Some(report.warnings.len());
    Ok(state)
}

#[tauri::command]
fn write_ai_context(
    project_path: String,
    view_mode: String,
    selected_entity_id: String,
) -> Result<AiContextState, String> {
    Ok(write_ai_context_for_path(
        Path::new(&project_path),
        &view_mode,
        &selected_entity_id,
    ))
}

#[tauri::command]
fn load_last_project<R: Runtime>(app: tauri::AppHandle<R>) -> Result<Option<String>, String> {
    let path = last_project_path(&app)?;
    if !path.exists() {
        return Ok(None);
    }
    let value = fs::read_to_string(&path)
        .map_err(|error| format!("failed to read last project: {error}"))?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        Ok(Some(trimmed.to_owned()))
    }
}

#[tauri::command]
fn save_last_project<R: Runtime>(
    app: tauri::AppHandle<R>,
    project_path: String,
) -> Result<(), String> {
    let path = last_project_path(&app)?;
    let Some(parent) = path.parent() else {
        return Err("last project path has no parent".to_owned());
    };
    fs::create_dir_all(parent)
        .map_err(|error| format!("failed to create app config dir: {error}"))?;
    fs::write(path, project_path).map_err(|error| format!("failed to save last project: {error}"))
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            open_project,
            run_review,
            import_jww,
            write_ai_context,
            load_last_project,
            save_last_project
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn open_project_state(project_path: &Path) -> Result<ProjectState, String> {
    let source = cad_model::load_project(project_path)
        .map_err(|error| format!("failed to load project: {error}"))?;
    Ok(ProjectState {
        project_path: project_path.to_string_lossy().into_owned(),
        project_name: source.project.name,
        is_git_project: git_root(project_path).is_ok(),
        import_warning_count: None,
    })
}

fn run_review_for_path(project_path: &Path) -> Result<ReviewArtifacts, String> {
    let head = cad_model::load_project(project_path)
        .map_err(|error| format!("failed to load project: {error}"))?;
    let check = cad_check::check_project(project_path);
    let sheet_svg = cad_render_svg::render_project_svg(&head)
        .map_err(|error| format!("failed to render SVG: {error}"))?;
    let comments = load_comments(project_path, &head);

    let diff_result = build_head_base_project(project_path).and_then(|base_dir| {
        cad_model::load_project(base_dir.path())
            .map_err(|error| format!("failed to load HEAD project: {error}"))
            .map(|base| {
                let diff = cad_diff::diff_projects(&base, &head);
                let diff_svg = cad_diff::diff_projects_svg(&base, &head);
                (diff, diff_svg)
            })
    });

    let (diff, diff_svg, diff_unavailable) = match diff_result {
        Ok((diff, diff_svg)) => (Some(diff), Some(diff_svg), None),
        Err(message) => (None, None, Some(message)),
    };

    Ok(ReviewArtifacts {
        sheet_svg,
        diff_svg,
        check,
        diff,
        comments,
        diff_unavailable,
    })
}

fn write_ai_context_for_path(
    project_path: &Path,
    view_mode: &str,
    selected_entity_id: &str,
) -> AiContextState {
    if selected_entity_id.trim().is_empty() {
        return ai_context_state(
            AiContextStatus::NoEntitySelected,
            None,
            None,
            Some("no entity selected".to_owned()),
        );
    }

    match build_ai_context(project_path, view_mode, selected_entity_id) {
        Ok((context, markdown)) => {
            let build_dir = project_path.join("build");
            let json_path = build_dir.join("ai-context.json");
            let markdown_path = build_dir.join("ai-context.md");
            if let Err(error) = fs::create_dir_all(&build_dir) {
                return ai_context_state(
                    AiContextStatus::Error,
                    None,
                    None,
                    Some(format!("failed to create build dir: {error}")),
                );
            }
            let json = match serde_json::to_string_pretty(&context) {
                Ok(json) => json,
                Err(error) => {
                    return ai_context_state(
                        AiContextStatus::Error,
                        None,
                        None,
                        Some(format!("failed to serialize AI context: {error}")),
                    );
                }
            };
            if let Err(error) = fs::write(&json_path, json) {
                return ai_context_state(
                    AiContextStatus::Error,
                    None,
                    None,
                    Some(format!("failed to write ai-context.json: {error}")),
                );
            }
            if let Err(error) = fs::write(&markdown_path, markdown) {
                return ai_context_state(
                    AiContextStatus::Error,
                    Some(path_string(&json_path)),
                    None,
                    Some(format!("failed to write ai-context.md: {error}")),
                );
            }
            ai_context_state(
                AiContextStatus::Ready,
                Some(path_string(&json_path)),
                Some(path_string(&markdown_path)),
                Some("AI context ready".to_owned()),
            )
        }
        Err(message) => ai_context_state(AiContextStatus::Error, None, None, Some(message)),
    }
}

fn build_ai_context(
    project_path: &Path,
    view_mode: &str,
    selected_entity_id: &str,
) -> Result<(AiContext, String), String> {
    let project = cad_model::load_project(project_path)
        .map_err(|error| format!("failed to load project: {error}"))?;
    let Some((drawing_name, record)) = find_entity_record(&project, selected_entity_id) else {
        return Err(format!("entity {selected_entity_id} was not found"));
    };
    let source_path = project_path
        .join("drawings")
        .join(&drawing_name)
        .join("entities.ndjson");
    let raw = read_source_line(&source_path, record.line)?;
    let check = cad_check::check_project(project_path);
    let comments = load_comments(project_path, &project)
        .into_iter()
        .filter(|comment| comment.entity_ids.iter().any(|id| id == selected_entity_id))
        .collect::<Vec<_>>();
    let (diff_changes, diff_warnings) =
        selected_diff_context(project_path, &project, selected_entity_id);
    let bbox = cad_model::entity_bbox(&record.entity).map(|bbox| AiContextBBox {
        min: bbox.min,
        max: bbox.max,
    });
    let entity = serde_json::to_value(&record.entity)
        .map_err(|error| format!("failed to serialize selected entity: {error}"))?;
    let check_diagnostics = check
        .diagnostics
        .into_iter()
        .filter(|diagnostic| diagnostic.entity_id.as_deref() == Some(selected_entity_id))
        .collect::<Vec<_>>();
    let generated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    let context = AiContext {
        schema_version: "0.1".to_owned(),
        project_path: path_string(project_path),
        project_name: project.project.name.clone(),
        drawing: drawing_name.clone(),
        view_mode: view_mode.to_owned(),
        selected_entity_id: selected_entity_id.to_owned(),
        source: AiContextSource {
            path: path_string(&source_path),
            line: record.line,
            raw,
        },
        entity,
        bbox,
        check_diagnostics,
        diff_changes,
        diff_warnings,
        comments,
        generated_at,
    };
    let markdown = ai_context_markdown(&context);
    Ok((context, markdown))
}

fn find_entity_record<'a>(
    project: &'a cad_model::ProjectSource,
    selected_entity_id: &str,
) -> Option<(String, &'a cad_model::EntityRecord)> {
    project.drawings.iter().find_map(|drawing| {
        drawing
            .entities
            .iter()
            .find(|record| record.entity.id().as_str() == selected_entity_id)
            .map(|record| (drawing.name.clone(), record))
    })
}

fn read_source_line(path: &Path, line_number: usize) -> Result<String, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("failed to read selected entity source: {error}"))?;
    text.lines()
        .nth(line_number.saturating_sub(1))
        .map(str::to_owned)
        .ok_or_else(|| format!("selected entity source line {line_number} was not found"))
}

fn selected_diff_context(
    project_path: &Path,
    head: &cad_model::ProjectSource,
    selected_entity_id: &str,
) -> (Vec<cad_diff::DiffChange>, Vec<cad_diff::DiffWarning>) {
    let Ok(base_dir) = build_head_base_project(project_path) else {
        return (Vec::new(), Vec::new());
    };
    let Ok(base) = cad_model::load_project(base_dir.path()) else {
        return (Vec::new(), Vec::new());
    };
    let diff = cad_diff::diff_projects(&base, head);
    let changes = diff
        .changes
        .into_iter()
        .filter(|change| change.entity_id == selected_entity_id)
        .collect::<Vec<_>>();
    let warnings = diff
        .warnings
        .into_iter()
        .filter(|warning| warning.entity_ids.iter().any(|id| id == selected_entity_id))
        .collect::<Vec<_>>();
    (changes, warnings)
}

fn ai_context_markdown(context: &AiContext) -> String {
    let entity_pretty =
        serde_json::to_string_pretty(&context.entity).unwrap_or_else(|_| "{}".to_owned());
    format!(
        "# AI Context\n\n- Project: {}\n- Drawing: {}\n- View mode: {}\n- Selected entity: `{}`\n- Source: `{}:{}`\n- Generated at: {}\n\n## Related Issues\n\n- Check diagnostics: {}\n- Diff changes: {}\n- Diff warnings: {}\n- Comments: {}\n\n## Edit Target\n\n```json\n{}\n```\n\n## Raw NDJSON Line\n\n```json\n{}\n```\n\n## Request Template\n\n選択entity `{}` を意図に合わせてNDJSON/TOMLで修正する。修正後はcheck/render/diffで確認する。\n",
        context.project_name,
        context.drawing,
        context.view_mode,
        context.selected_entity_id,
        context.source.path,
        context.source.line,
        context.generated_at,
        context.check_diagnostics.len(),
        context.diff_changes.len(),
        context.diff_warnings.len(),
        context.comments.len(),
        entity_pretty,
        context.source.raw,
        context.selected_entity_id
    )
}

fn ai_context_state(
    status: AiContextStatus,
    json_path: Option<String>,
    markdown_path: Option<String>,
    message: Option<String>,
) -> AiContextState {
    AiContextState {
        status,
        json_path,
        markdown_path,
        message,
    }
}

fn load_comments(project_path: &Path, project: &cad_model::ProjectSource) -> Vec<CommentRecord> {
    let Some(drawing) = project.drawings.first() else {
        return Vec::new();
    };
    let path = project_path
        .join("comments")
        .join(format!("{}.ndjson", drawing.name));
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| serde_json::from_str::<CommentRecord>(line).ok())
        .collect()
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn build_head_base_project(project_path: &Path) -> Result<tempfile::TempDir, String> {
    let repo = git_root(project_path)?;
    let repo =
        fs::canonicalize(repo).map_err(|error| format!("failed to canonicalize repo: {error}"))?;
    let project_path = fs::canonicalize(project_path)
        .map_err(|error| format!("failed to canonicalize project: {error}"))?;
    let relative_project = project_path
        .strip_prefix(&repo)
        .map_err(|_| "project is not inside the Git repository".to_owned())?;
    let relative_project_arg = git_project_pathspec(relative_project);

    let head_files = git_output_bytes(
        &repo,
        &[
            OsStr::new("ls-tree"),
            OsStr::new("-r"),
            OsStr::new("-z"),
            OsStr::new("--name-only"),
            OsStr::new("HEAD"),
            OsStr::new("--"),
            OsStr::new(&relative_project_arg),
        ],
    )?;
    let head_files = head_files
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| PathBuf::from(OsString::from_vec(path.to_vec())))
        .collect::<Vec<_>>();
    if head_files.is_empty() {
        return Err("project has no tracked files in Git".to_owned());
    }

    let temp = tempfile::tempdir().map_err(|error| format!("failed to create tempdir: {error}"))?;
    for git_path in head_files {
        let mut object_path = OsString::from("HEAD:");
        object_path.push(git_path.as_os_str());
        let content = git_output_bytes(&repo, &[OsStr::new("show"), object_path.as_os_str()])?;
        let relative_file = git_path
            .strip_prefix(relative_project)
            .map_err(|_| format!("tracked file {git_path:?} is outside project"))?;
        let destination = temp.path().join(relative_file);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("failed to create temp parent: {error}"))?;
        }
        fs::write(destination, content)
            .map_err(|error| format!("failed to write HEAD file: {error}"))?;
    }
    Ok(temp)
}

fn git_root(project_path: &Path) -> Result<PathBuf, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(project_path)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|error| format!("failed to run git: {error}"))?;
    if !output.status.success() {
        return Err(command_stderr("git rev-parse", &output.stderr));
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("git rev-parse output was not UTF-8: {error}"))?;
    Ok(PathBuf::from(stdout.trim()))
}

fn git_output_bytes(repo: &Path, args: &[&OsStr]) -> Result<Vec<u8>, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|error| format!("failed to run git: {error}"))?;
    if !output.status.success() {
        let command = args
            .iter()
            .map(|arg| arg.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" ");
        return Err(command_stderr(&format!("git {command}"), &output.stderr));
    }
    Ok(output.stdout)
}

fn command_stderr(command: &str, stderr: &[u8]) -> String {
    let message = String::from_utf8_lossy(stderr).trim().to_owned();
    if message.is_empty() {
        format!("{command} failed")
    } else {
        format!("{command} failed: {message}")
    }
}

fn path_to_git_arg(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn git_project_pathspec(relative_project: &Path) -> String {
    let pathspec = path_to_git_arg(relative_project);
    if pathspec.is_empty() {
        ".".to_owned()
    } else {
        pathspec
    }
}

fn last_project_path<R: Runtime>(app: &tauri::AppHandle<R>) -> Result<PathBuf, String> {
    app.path()
        .app_config_dir()
        .map(|path| path.join("last-project.txt"))
        .map_err(|error| format!("failed to resolve app config dir: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn git_tracked_project_can_be_loaded_from_head() {
        let repo = fixture_repo();
        let base_dir =
            build_head_base_project(&repo.project_path).expect("HEAD project should build");
        let project = cad_model::load_project(base_dir.path()).expect("HEAD project should load");

        assert_eq!(project.project.name, "desktop-fixture");
        assert_eq!(project.drawings[0].entities.len(), 1);
    }

    #[test]
    fn git_repo_root_project_can_be_loaded_from_head() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        write_project(temp.path(), false);
        run_git(temp.path(), &["init"]);
        run_git(
            temp.path(),
            &["config", "user.email", "cad@example.invalid"],
        );
        run_git(temp.path(), &["config", "user.name", "CAD Test"]);
        run_git(temp.path(), &["add", "."]);
        run_git(temp.path(), &["commit", "-m", "initial"]);

        let base_dir = build_head_base_project(temp.path()).expect("HEAD project should build");
        let project = cad_model::load_project(base_dir.path()).expect("HEAD project should load");

        assert_eq!(project.project.name, "desktop-fixture");
        assert_eq!(project.drawings[0].entities.len(), 1);
    }

    #[test]
    fn non_ascii_git_project_path_can_be_loaded_from_head() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let project_path = temp.path().join("日本語 project");
        write_project(&project_path, false);
        run_git(temp.path(), &["init"]);
        run_git(
            temp.path(),
            &["config", "user.email", "cad@example.invalid"],
        );
        run_git(temp.path(), &["config", "user.name", "CAD Test"]);
        run_git(temp.path(), &["add", "."]);
        run_git(temp.path(), &["commit", "-m", "initial"]);

        let base_dir = build_head_base_project(&project_path).expect("HEAD project should build");
        let project = cad_model::load_project(base_dir.path()).expect("HEAD project should load");

        assert_eq!(project.project.name, "desktop-fixture");
        assert_eq!(project.drawings[0].entities.len(), 1);
    }

    #[test]
    fn non_git_project_reports_diff_unavailable() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        write_project(temp.path(), false);

        let artifacts = run_review_for_path(temp.path()).expect("review should still render");

        assert!(artifacts.diff.is_none());
        assert!(artifacts.diff_svg.is_none());
        assert!(artifacts.diff_unavailable.is_some());
    }

    #[test]
    fn working_tree_change_is_modified_against_head() {
        let repo = fixture_repo();
        write_entities(
            &repo.project_path,
            &[
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0.0,0.0],"p2":[1200.0,0.0]}"#,
            ],
        );

        let artifacts = run_review_for_path(&repo.project_path).expect("review should run");
        let diff = artifacts.diff.expect("diff should be available");

        assert!(diff.changes.iter().any(|change| {
            change.entity_id == "ent_01JZ0000000000000000000000"
                && change.kind == cad_diff::ChangeKind::Modified
        }));
    }

    #[test]
    fn working_tree_added_entity_is_added_against_head() {
        let repo = fixture_repo();
        write_entities(
            &repo.project_path,
            &[
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0.0,0.0],"p2":[910.0,0.0]}"#,
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000001","type":"text","layer":"0-1","style":"note","at":[100.0,200.0],"rotation_deg":0.0,"value":"new"}"#,
            ],
        );

        let artifacts = run_review_for_path(&repo.project_path).expect("review should run");
        let diff = artifacts.diff.expect("diff should be available");

        assert!(diff.changes.iter().any(|change| {
            change.entity_id == "ent_01JZ0000000000000000000001"
                && change.kind == cad_diff::ChangeKind::Added
        }));
    }

    #[test]
    fn staged_new_project_file_does_not_break_head_diff() {
        let repo = fixture_repo();
        let staged_note = repo.project_path.join("drawings/plan_1f/staged-note.txt");
        fs::write(&staged_note, "not in HEAD yet\n").expect("staged note should be written");
        run_git(
            repo._temp.path(),
            &["add", "project/drawings/plan_1f/staged-note.txt"],
        );
        write_entities(
            &repo.project_path,
            &[
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0.0,0.0],"p2":[1200.0,0.0]}"#,
            ],
        );

        let artifacts = run_review_for_path(&repo.project_path).expect("review should run");

        assert!(artifacts.diff_unavailable.is_none());
        assert!(artifacts.diff.is_some());
        assert!(artifacts.diff_svg.is_some());
    }

    #[test]
    fn import_jww_command_returns_project_state_with_warning_count() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../examples/jww-fixtures/Test1.jww");
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let out_dir = temp.path().join("imported");

        let state = import_jww(
            fixture.to_string_lossy().into_owned(),
            out_dir.to_string_lossy().into_owned(),
        )
        .expect("JWW should import");

        assert_eq!(state.project_name, "test1");
        assert_eq!(state.import_warning_count, Some(58));
        assert!(out_dir.join("build/import-jww-report.json").exists());
    }

    #[test]
    fn ai_context_writes_selected_entity_source_and_related_records() {
        let repo = fixture_repo();

        let state =
            write_ai_context_for_path(&repo.project_path, "diff", "ent_01JZ0000000000000000000000");

        assert_eq!(state.status, AiContextStatus::Ready);
        assert!(repo.project_path.join("build/ai-context.json").exists());
        assert!(repo.project_path.join("build/ai-context.md").exists());

        let json = fs::read_to_string(repo.project_path.join("build/ai-context.json"))
            .expect("AI context JSON should be readable");
        let value: serde_json::Value =
            serde_json::from_str(&json).expect("AI context JSON should parse");
        assert_eq!(value["schema_version"], "0.1");
        assert_eq!(
            value["selected_entity_id"],
            "ent_01JZ0000000000000000000000"
        );
        assert_eq!(value["source"]["line"], 1);
        assert!(
            value["source"]["raw"]
                .as_str()
                .expect("raw line should be a string")
                .contains("\"type\":\"line\"")
        );
        assert_eq!(
            value["comments"][0]["text"],
            serde_json::Value::String("desktop fixture".to_owned())
        );

        let markdown = fs::read_to_string(repo.project_path.join("build/ai-context.md"))
            .expect("AI context markdown should be readable");
        assert!(markdown.contains("Selected entity"));
        assert!(markdown.contains("ent_01JZ0000000000000000000000"));
    }

    #[test]
    fn ai_context_reports_no_entity_selected_without_deleting_existing_context() {
        let repo = fixture_repo();
        fs::create_dir_all(repo.project_path.join("build")).expect("build dir should be created");
        fs::write(repo.project_path.join("build/ai-context.md"), "old context")
            .expect("old context should be writable");

        let state = write_ai_context_for_path(&repo.project_path, "sheet", "");

        assert_eq!(state.status, AiContextStatus::NoEntitySelected);
        assert_eq!(
            fs::read_to_string(repo.project_path.join("build/ai-context.md"))
                .expect("old context should remain"),
            "old context"
        );
    }

    #[test]
    fn ai_context_reports_missing_entity_as_error_state() {
        let repo = fixture_repo();

        let state = write_ai_context_for_path(
            &repo.project_path,
            "sheet",
            "ent_01JZ0000000000000000009999",
        );

        assert_eq!(state.status, AiContextStatus::Error);
        assert!(
            state
                .message
                .expect("missing entity should have a message")
                .contains("was not found")
        );
    }

    #[test]
    fn ai_context_succeeds_without_comments_or_git_diff() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        write_project(temp.path(), false);

        let state =
            write_ai_context_for_path(temp.path(), "sheet", "ent_01JZ0000000000000000000000");

        assert_eq!(state.status, AiContextStatus::Ready);
        let json = fs::read_to_string(temp.path().join("build/ai-context.json"))
            .expect("AI context JSON should be readable");
        let value: serde_json::Value =
            serde_json::from_str(&json).expect("AI context JSON should parse");
        assert_eq!(
            value["comments"]
                .as_array()
                .expect("comments should be an array")
                .len(),
            0
        );
        assert_eq!(
            value["diff_changes"]
                .as_array()
                .expect("diff changes should be an array")
                .len(),
            0
        );
    }

    struct FixtureRepo {
        _temp: tempfile::TempDir,
        project_path: PathBuf,
    }

    fn fixture_repo() -> FixtureRepo {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let project_path = temp.path().join("project");
        write_project(&project_path, true);
        run_git(temp.path(), &["init"]);
        run_git(
            temp.path(),
            &["config", "user.email", "cad@example.invalid"],
        );
        run_git(temp.path(), &["config", "user.name", "CAD Test"]);
        run_git(temp.path(), &["add", "project"]);
        run_git(temp.path(), &["commit", "-m", "initial"]);
        FixtureRepo {
            _temp: temp,
            project_path,
        }
    }

    fn write_project(project_path: &Path, include_comment: bool) {
        fs::create_dir_all(project_path.join("rules")).expect("rules dir should be created");
        fs::create_dir_all(project_path.join("drawings/plan_1f"))
            .expect("drawing dir should be created");
        fs::create_dir_all(project_path.join("comments")).expect("comments dir should be created");
        fs::write(
            project_path.join("cad.project.toml"),
            "schema_version = \"0.1\"\nname = \"desktop-fixture\"\n",
        )
        .expect("project TOML should be written");
        fs::write(
            project_path.join("rules/layers.toml"),
            "[layers.\"0-1\"]\nname = \"A-WALL\"\nvisible = true\nprintable = true\ncolor = \"jw_black\"\nline_type = \"solid\"\nline_width = 0.25\n",
        )
        .expect("layers TOML should be written");
        fs::write(
            project_path.join("rules/styles.toml"),
            "[colors.jw_black]\nrgb = \"#000000\"\nprint_width = 0.25\n\n[line_types.solid]\ndash = []\n\n[text_styles.note]\nfont_family = \"Hiragino Sans\"\nheight = 250\nwidth = 125\nspacing = 0\nalign = \"left\"\n\n[dimension_styles.dim_100]\ntext_style = \"note\"\narrow_size = 120\nextension_gap = 40\nprecision = 0\nunit = \"mm\"\n",
        )
        .expect("styles TOML should be written");
        fs::write(
            project_path.join("drawings/plan_1f/sheet.toml"),
            "schema_version = \"0.1\"\npaper = \"A3\"\norientation = \"landscape\"\nscale = \"1/100\"\norigin = [0.0, 0.0]\n",
        )
        .expect("sheet TOML should be written");
        write_entities(
            project_path,
            &[
                r#"{"schema_version":"0.1","id":"ent_01JZ0000000000000000000000","type":"line","layer":"0-1","p1":[0.0,0.0],"p2":[910.0,0.0]}"#,
            ],
        );
        if include_comment {
            fs::write(
                project_path.join("comments/plan_1f.ndjson"),
                "{\"id\":\"cmt_01JZ0000000000000000000000\",\"drawing\":\"plan_1f\",\"entity_ids\":[\"ent_01JZ0000000000000000000000\"],\"text\":\"desktop fixture\",\"status\":\"open\"}\n",
            )
            .expect("comment should be written");
        }
    }

    fn write_entities(project_path: &Path, lines: &[&str]) {
        let mut file = fs::File::create(project_path.join("drawings/plan_1f/entities.ndjson"))
            .expect("entities should be writable");
        for line in lines {
            writeln!(file, "{line}").expect("entity should be written");
        }
    }

    fn run_git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .expect("git should run");
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
