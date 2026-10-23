//! `codeviz`: build, check and explore codeviz IR documents.

mod diff;
mod ir;
mod journey;
mod links;
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
    /// Import a Buildkite pipeline as an IR document.
    Import(ir::ImportArgs),
    /// List entries, scenarios, and print journeys from IR documents.
    Journey(journey::JourneyArgs),
    /// Open what changed between two git revisions in the 3D viewer.
    Diff(diff::DiffArgs),
    /// Open IR documents in the 3D viewer.
    View(view::ViewArgs),
    /// Check IR documents against the rules in docs/ir.md.
    Validate(ir::ValidateArgs),
    /// Print the IR JSON Schema.
    Schema,
}

fn main() -> ExitCode {
    let result = match Cli::parse().command {
        Command::Analyze(args) => ir::analyze(&args),
        Command::Import(args) => ir::import(&args),
        Command::Journey(args) => journey::run(&args),
        Command::Diff(args) => diff::run(&args),
        Command::View(args) => view::run(&args),
        Command::Validate(args) => ir::validate(&args),
        Command::Schema => ir::schema(),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
