//! Command-line entrypoint for cadc.
//!
//! CLI entrypoint for source loading, checking, rendering, and semantic diff.

use clap::{Parser, Subcommand, ValueEnum};
use miette::{IntoDiagnostic, Result, WrapErr, miette};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(name = "cadc", version, about = "Git-native CAD source tooling")]
struct Cli {
    #[arg(long, help = "Print scaffold diagnostics and exit")]
    smoke: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(about = "Strictly validate CAD source")]
    Check {
        #[arg(value_name = "PROJECT")]
        project: PathBuf,

        #[arg(long, value_enum)]
        format: CheckFormat,

        #[arg(long, value_name = "PATH")]
        out: PathBuf,
    },
    #[command(about = "Normalize known CAD numeric values in source files")]
    Format {
        #[arg(value_name = "PROJECT")]
        project: PathBuf,
    },
    #[command(about = "Import a JWW file into a new CAD source project")]
    ImportJww {
        #[arg(value_name = "INPUT")]
        input: PathBuf,

        #[arg(long, value_name = "PROJECT_DIR")]
        out: PathBuf,

        #[arg(
            long,
            help = "Flatten block references instead of preserving definitions"
        )]
        flatten: bool,
    },
    #[command(about = "Export a CAD drawing to experimental JWW version 600")]
    ExportJww {
        #[arg(value_name = "PROJECT")]
        project: PathBuf,

        #[arg(long, value_name = "NAME")]
        drawing: String,

        #[arg(long, value_name = "FILE.jww")]
        out: PathBuf,

        #[arg(long)]
        allow_lossy: bool,

        #[arg(long)]
        force: bool,

        #[arg(long, value_name = "REPORT.json")]
        report: Option<PathBuf>,
    },
    #[command(about = "Export a CAD drawing to PDF")]
    ExportPdf {
        #[arg(value_name = "PROJECT")]
        project: PathBuf,

        #[arg(long, value_name = "NAME")]
        drawing: String,

        #[arg(long, value_name = "NAME")]
        layout: Option<String>,

        #[arg(long, value_name = "FILE.pdf")]
        out: PathBuf,

        #[arg(long)]
        force: bool,
    },
    #[command(about = "Compare CAD source projects by stable entity IDs")]
    Diff {
        #[arg(value_name = "BASE")]
        base: PathBuf,

        #[arg(value_name = "HEAD")]
        head: PathBuf,

        #[arg(long, value_enum)]
        format: DiffFormat,

        #[arg(long, value_name = "PATH")]
        out: PathBuf,
    },
    #[command(about = "Render CAD source to SVG")]
    Render {
        #[arg(value_name = "PROJECT")]
        project: PathBuf,

        #[arg(long, value_enum)]
        format: RenderFormat,

        #[arg(long, value_name = "PATH")]
        out: PathBuf,
    },
}

impl Command {
    fn source_projects(&self) -> Vec<&Path> {
        match self {
            Self::Check { project, .. }
            | Self::Format { project }
            | Self::ExportJww { project, .. }
            | Self::ExportPdf { project, .. }
            | Self::Render { project, .. } => vec![project.as_path()],
            Self::Diff { base, head, .. } => vec![base.as_path(), head.as_path()],
            Self::ImportJww { .. } => Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CheckFormat {
    Json,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum RenderFormat {
    Svg,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum DiffFormat {
    Json,
    Svg,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.smoke {
        println!("cadc scaffold ok");
        return Ok(());
    }

    if let Some(command) = cli.command.as_ref() {
        for project in command.source_projects() {
            recover_project(project)?;
        }
    }

    match cli.command {
        Some(Command::Check {
            project,
            format: CheckFormat::Json,
            out,
        }) => {
            let report = cad_check::check_project(&project);
            write_json_report(&out, &report)?;
            if !report.is_ok() {
                return Err(miette!(
                    "check failed with {} diagnostic(s)",
                    report.diagnostics.len()
                ));
            }
        }
        Some(Command::Format { project }) => {
            let files = format_project(&project).into_diagnostic()?;
            println!("formatted {files} NDJSON file(s)");
        }
        Some(Command::ImportJww {
            input,
            out,
            flatten,
        }) => {
            let options = cad_import_jww::ImportOptions {
                block_mode: if flatten {
                    cad_import_jww::BlockMode::Flatten
                } else {
                    cad_import_jww::BlockMode::Preserve
                },
            };
            let report = cad_import_jww::import_jww_file_with_options(&input, &out, options)
                .into_diagnostic()?;
            println!(
                "imported {} entity/entities from {} into {} ({} warning(s))",
                report.supported_entities,
                input.display(),
                out.display(),
                report.warnings.len()
            );
        }
        Some(Command::ExportJww {
            project,
            drawing,
            out,
            allow_lossy,
            force,
            report,
        }) => {
            if report
                .as_ref()
                .is_some_and(|report_path| paths_refer_to_same_file(&out, report_path))
            {
                return Err(miette!("--out and --report must refer to different files"));
            }
            let export = cad_export_jww::export_jww_file(
                &project,
                &drawing,
                &out,
                cad_export_jww::ExportOptions {
                    allow_lossy,
                    overwrite: force,
                },
            )
            .into_diagnostic()?;
            if let Some(report_path) = report {
                let json = serde_json::to_string_pretty(&export).into_diagnostic()?;
                write_text_file(&report_path, &format!("{json}\n"))?;
            }
            println!(
                "JWW export {:?}: {} record(s), {} warning(s), {} blocker(s)",
                export.status,
                export.expanded_entities,
                export.warnings.len(),
                export.blockers.len()
            );
            if export.status == cad_export_jww::ExportStatus::Blocked {
                return Err(miette!(
                    "JWW export blocked by {} compatibility issue(s)",
                    export.blockers.len()
                ));
            }
        }
        Some(Command::ExportPdf {
            project,
            drawing,
            layout,
            out,
            force,
        }) => {
            cad_render_pdf::export_drawing_pdf(
                &project,
                &drawing,
                layout.as_deref(),
                &out,
                cad_render_pdf::PdfExportOptions {
                    overwrite: force,
                    expected_files: Vec::new(),
                },
            )
            .into_diagnostic()?;
            println!("exported PDF to {}", out.display());
        }
        Some(Command::Diff {
            base,
            head,
            format,
            out,
        }) => {
            let base = cad_model::load_project(&base).into_diagnostic()?;
            let head = cad_model::load_project(&head).into_diagnostic()?;
            match format {
                DiffFormat::Json => {
                    let json = cad_diff::diff_projects_json(&base, &head).into_diagnostic()?;
                    write_text_file(&out, &format!("{json}\n"))?;
                }
                DiffFormat::Svg => {
                    let svg = cad_diff::diff_projects_svg(&base, &head);
                    write_text_file(&out, &format!("{svg}\n"))?;
                }
            }
        }
        Some(Command::Render {
            project,
            format: RenderFormat::Svg,
            out,
        }) => {
            let source = cad_model::load_project(&project).into_diagnostic()?;
            let svg = cad_render_svg::render_project_svg(&source).into_diagnostic()?;
            write_text_file(&out, &format!("{svg}\n"))?;
        }
        None => {}
    }

    Ok(())
}

fn recover_project(project: &Path) -> Result<()> {
    cad_edit::recover_source_transactions(project).map_err(|error| {
        miette!(
            "failed to recover source transactions in {}: {error}",
            project.display()
        )
    })
}

fn paths_refer_to_same_file(left: &std::path::Path, right: &std::path::Path) -> bool {
    if left == right {
        return true;
    }
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => {
            let absolute = |path: &std::path::Path| {
                if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    std::env::current_dir().unwrap_or_default().join(path)
                }
            };
            absolute(left) == absolute(right)
        }
    }
}

fn write_json_report(path: &Path, report: &cad_check::CheckReport) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to create {}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(report)
        .into_diagnostic()
        .wrap_err("failed to serialize check report")?;
    write_atomic_text(path, &format!("{text}\n"))
}

fn write_text_file(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to create {}", parent.display()))?;
    }
    write_atomic_text(path, text)
}

fn write_atomic_text(path: &Path, text: &str) -> Result<()> {
    cad_edit::atomic_publish(path, text.as_bytes(), true)
        .map_err(|error| miette::miette!("failed to publish {}: {error}", path.display()))
        .wrap_err_with(|| format!("failed to publish {}", path.display()))?;
    Ok(())
}

fn format_project(project: &Path) -> std::io::Result<usize> {
    let source = cad_model::load_project(project)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let mut files = Vec::new();
    for drawing in &source.drawings {
        files.push(
            project
                .join("drawings")
                .join(&drawing.name)
                .join("entities.ndjson"),
        );
    }
    for drawing in &source.drawings {
        let path = project
            .join("comments")
            .join(format!("{}.ndjson", drawing.name));
        if path.exists() {
            files.push(path);
        }
    }
    for path in &files {
        format_ndjson_file(path)?;
    }
    Ok(files.len())
}

fn format_ndjson_file(path: &Path) -> std::io::Result<()> {
    let original = fs::read(path)?;
    let mut output = String::new();
    let text = String::from_utf8(original.clone())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    for (index, segment) in text.split_inclusive('\n').enumerate() {
        let (line, ending) = if let Some(line) = segment.strip_suffix("\r\n") {
            (line, "\r\n")
        } else if let Some(line) = segment.strip_suffix('\n') {
            (line, "\n")
        } else {
            (segment, "")
        };
        if line.trim().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("empty NDJSON line at {}:{}", path.display(), index + 1),
            ));
        }
        let value: serde_json::Value = serde_json::from_str(line).map_err(|error| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "invalid NDJSON at {}:{}: {error}",
                    path.display(),
                    index + 1
                ),
            )
        })?;
        output.push_str(
            &serde_json::to_string(&normalize_numbers(value)).map_err(std::io::Error::other)?,
        );
        output.push_str(ending);
    }
    let metadata = fs::metadata(path)?;
    let permissions = cad_edit::permissions_snapshot(&metadata.permissions());
    let expected_revision = blake3::hash(&original).to_hex().to_string();
    cad_edit::atomic_replace_with_permissions(
        path,
        output.as_bytes(),
        &expected_revision,
        &permissions,
    )
    .map_err(|error| std::io::Error::other(error.to_string()))
}

fn normalize_numbers(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Number(number) => number
            .as_f64()
            .and_then(|value| serde_json::Number::from_f64((value * 1000.0).round() / 1000.0))
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Number(number)),
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.into_iter().map(normalize_numbers).collect())
        }
        serde_json::Value::Object(values) => serde_json::Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, normalize_numbers(value)))
                .collect(),
        ),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command, format_ndjson_file, normalize_numbers, paths_refer_to_same_file};
    use clap::Parser;
    use std::fs;
    use std::path::Path;

    #[test]
    fn rejects_identical_export_and_report_paths() {
        assert!(paths_refer_to_same_file(
            Path::new("result.jww"),
            Path::new("result.jww")
        ));
        assert!(!paths_refer_to_same_file(
            Path::new("result.jww"),
            Path::new("result.json")
        ));
    }

    #[test]
    fn parses_pdf_export_contract() {
        let cli = Cli::try_parse_from([
            "cadc",
            "export-pdf",
            "project",
            "--drawing",
            "plan",
            "--layout",
            "print",
            "--out",
            "plan.pdf",
            "--force",
        ])
        .expect("PDF command should parse");
        let Some(Command::ExportPdf {
            project,
            drawing,
            layout,
            out,
            force,
        }) = cli.command
        else {
            panic!("expected export-pdf command");
        };
        assert_eq!(project, Path::new("project"));
        assert_eq!(drawing, "plan");
        assert_eq!(layout.as_deref(), Some("print"));
        assert_eq!(out, Path::new("plan.pdf"));
        assert!(force);
    }

    #[test]
    fn every_source_reading_command_declares_its_recovery_roots() {
        let cases = [
            (
                vec![
                    "cadc", "check", "project", "--format", "json", "--out", "out.json",
                ],
                1,
            ),
            (vec!["cadc", "format", "project"], 1),
            (
                vec![
                    "cadc",
                    "export-jww",
                    "project",
                    "--drawing",
                    "plan",
                    "--out",
                    "out.jww",
                ],
                1,
            ),
            (
                vec![
                    "cadc",
                    "export-pdf",
                    "project",
                    "--drawing",
                    "plan",
                    "--out",
                    "out.pdf",
                ],
                1,
            ),
            (
                vec![
                    "cadc", "render", "project", "--format", "svg", "--out", "out.svg",
                ],
                1,
            ),
            (
                vec![
                    "cadc", "diff", "base", "head", "--format", "json", "--out", "out.json",
                ],
                2,
            ),
        ];
        for (arguments, expected_roots) in cases {
            let cli = Cli::try_parse_from(arguments).expect("command should parse");
            assert_eq!(
                cli.command.expect("command").source_projects().len(),
                expected_roots
            );
        }

        let import =
            Cli::try_parse_from(["cadc", "import-jww", "source.jww", "--out", "new-project"])
                .expect("import should parse");
        assert!(
            import
                .command
                .expect("import command")
                .source_projects()
                .is_empty()
        );
    }

    #[test]
    fn format_normalizes_numbers_and_preserves_crlf_and_trailing_newline() {
        let temp = tempfile::tempdir().expect("tempdir should be created");
        let path = temp.path().join("entities.ndjson");
        fs::write(&path, b"{\"x\":1.23456,\"unknown\":\"keep\"}\r\n")
            .expect("fixture should write");
        format_ndjson_file(&path).expect("format should succeed");
        let text = fs::read_to_string(path).expect("formatted file should read");
        assert!(text.contains("1.235"));
        assert!(text.contains("keep"));
        assert!(text.ends_with("\n"));
        assert!(text.contains("\r\n"));
        assert_eq!(
            normalize_numbers(serde_json::json!({"x": 2.3456}))["x"],
            2.346
        );
    }
}
