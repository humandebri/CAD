//! Command-line entrypoint for cadc.
//!
//! Phase 1 adds the first source-model command boundary. `cadc format` loads
//! the project strictly, but does not rewrite files yet.

use clap::{Parser, Subcommand};
use miette::{IntoDiagnostic, Result};
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
    #[command(about = "Load CAD source and prepare it for future formatting")]
    Format {
        #[arg(value_name = "PROJECT")]
        project: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.smoke {
        println!("cadc scaffold ok");
        return Ok(());
    }

    if let Some(Command::Format { project }) = cli.command {
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

    Ok(())
}
