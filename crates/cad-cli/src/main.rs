//! Command-line entrypoint for cadc.
//!
//! CLI entrypoint for source loading, checking, rendering, and semantic diff.

use clap::{Parser, Subcommand, ValueEnum};
use miette::{IntoDiagnostic, Result, WrapErr, miette};
use std::fs;
use std::path::PathBuf;

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
    #[command(about = "Load CAD source and prepare it for future formatting")]
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
            let source = cad_model::load_project(&project).into_diagnostic()?;
            let entity_count: usize = source
                .drawings
                .iter()
                .map(|drawing| drawing.entities.len())
                .sum();
            println!(
                "format scaffold ok: {} drawing(s), {} entity/entities",
                source.drawings.len(),
                entity_count
            );
        }
        Some(Command::ImportJww { input, out }) => {
            let report = cad_import_jww::import_jww_file(&input, &out).into_diagnostic()?;
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

fn write_json_report(path: &PathBuf, report: &cad_check::CheckReport) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to create {}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(report)
        .into_diagnostic()
        .wrap_err("failed to serialize check report")?;
    fs::write(path, format!("{text}\n"))
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to write {}", path.display()))
}

fn write_text_file(path: &PathBuf, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, text)
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::paths_refer_to_same_file;
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
}
