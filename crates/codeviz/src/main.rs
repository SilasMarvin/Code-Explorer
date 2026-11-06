//! `codeviz`: explore a Rust project's call flow in 3D.

mod analyze;
mod ir;
mod view;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    version,
    about,
    after_help = "Start with `codeviz view path/to/your/project`; press H in the window for the keys."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Open a Rust project (analyzed first) or IR documents in the 3D viewer.
    View(view::ViewArgs),
    /// Analyze a Rust project and write its IR document.
    Analyze(analyze::AnalyzeArgs),
    /// Check IR documents, such as one written by hand, for errors.
    Validate(ir::ValidateArgs),
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Command::Analyze(args) => analyze::run(&args),
        Command::View(args) => view::run(&args),
        Command::Validate(args) => ir::validate(&args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
