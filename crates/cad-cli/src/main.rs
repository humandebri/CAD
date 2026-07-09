//! Command-line entrypoint for cadc.
//!
//! Phase 2 exposes strict source loading and checking. `cadc format` still only
//! loads the project and does not rewrite files yet.

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
