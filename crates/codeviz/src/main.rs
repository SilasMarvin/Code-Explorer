//! `codeviz`: build, check and explore codeviz IR documents.

mod ir;
mod view;

use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Analyze source code and write an IR document.
    Analyze(ir::AnalyzeArgs),
    /// Open IR documents in the 3D viewer.
    View(view::ViewArgs),
    /// Check IR documents against the rules in docs/ir.md.
    Validate(ir::ValidateArgs),
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Command::Analyze(args) => ir::analyze(&args),
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
