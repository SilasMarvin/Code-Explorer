//! `codeviz`: build, check and explore codeviz IR documents.

mod cache;
mod diff;
mod ir;
mod journey;
mod state;
mod view;

use std::{ffi::OsString, process::ExitCode};

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
    #[command(args_override_self = true)]
    Diff(diff::DiffArgs),
    /// Open IR documents in the 3D viewer.
    #[command(args_override_self = true)]
    View(view::ViewArgs),
    /// Check IR documents against the rules in docs/ir.md.
    Validate(ir::ValidateArgs),
    /// Print the IR JSON Schema.
    Schema,
}

/// With `--resume`, the command line read again with the flags the view was last left
/// with ahead of those given, so those given win.
fn resumed(cli: Cli) -> Cli {
    let (key, name) = match &cli.command {
        Command::View(args) if args.display.start.resume => args.state_key(),
        Command::Diff(args) if args.display.start.resume => args.state_key(),
        _ => return cli,
    };
    let Some(saved) = state::file(&key, &name).and_then(|file| state::saved_flags(&file)) else {
        eprintln!("nothing to resume: this has not been viewed before");
        return cli;
    };
    let args: Vec<OsString> = std::env::args_os().collect();
    Cli::parse_from(state::resume(&args, &saved))
}

fn main() -> ExitCode {
    let result = match resumed(Cli::parse()).command {
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
