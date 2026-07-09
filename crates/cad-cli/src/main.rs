//! Command-line entrypoint for cadc.
//!
//! Phase 0 exposes the CLI shell only. Real commands are added in later phases.

use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "cadc", version, about = "Git-native CAD source tooling")]
struct Cli {
    #[arg(long, help = "Print scaffold diagnostics and exit")]
    smoke: bool,
}

fn main() {
    let cli = Cli::parse();
    if cli.smoke {
        println!("cadc scaffold ok");
    }
}
