//! `codeviz`: analyze a Rust codebase and explore its call flow.

use std::{
    fmt::Write as _,
    io::{self, Write as _},
    path::PathBuf,
    process::ExitCode,
};

use anyhow::{Context, bail};
use clap::Parser;
use codeviz_graph::{
    CallGraph, Function, FunctionId, StopReason, Trace, TraceOptions, TraceTarget,
};
use codeviz_rust::AnalyzeOptions;
use codeviz_viewer::Viewer;

#[derive(Debug, Parser)]
#[expect(clippy::struct_excessive_bools, reason = "independent CLI flags")]
#[command(version, about)]
struct Cli {
    /// Directory containing one or more Cargo packages.
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Function to trace from, e.g. `get_user` or `handlers::get_user`.
    #[arg(short, long)]
    entry: Option<String>,

    /// Print every analyzed function and exit.
    #[arg(long, conflicts_with = "print_trace")]
    list: bool,

    /// Print the trace from `--entry` as a text tree and exit.
    #[arg(long, requires = "entry")]
    print_trace: bool,

    /// Render the viewer once, save a PNG screenshot here, and exit.
    #[arg(long, value_name = "PATH")]
    screenshot: Option<PathBuf>,

    /// Include calls into code outside the analyzed source.
    #[arg(long)]
    external: bool,

    /// Analyze `#[cfg(test)]` modules and `#[test]` functions.
    #[arg(long)]
    include_tests: bool,
}

fn main() -> ExitCode {
    match run(&Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> anyhow::Result<()> {
    let root = cli
        .path
        .canonicalize()
        .with_context(|| format!("cannot open {}", cli.path.display()))?;
    let analysis = codeviz_rust::analyze(
        &root,
        AnalyzeOptions {
            include_tests: cli.include_tests,
        },
    )?;
    let graph = analysis.graph;
    let entry = cli
        .entry
        .as_deref()
        .map(|query| find_entry(&graph, query))
        .transpose()?;

    if cli.list {
        return write_stdout(&list(&graph));
    }
    let options = TraceOptions {
        include_external: cli.external,
        ..TraceOptions::default()
    };
    if let Some(entry) = entry.filter(|_| cli.print_trace) {
        let trace = Trace::from_root(&graph, entry, options).context("entry not in graph")?;
        return write_stdout(&render_trace(&graph, &trace));
    }

    let title = format!(
        "codeviz — {} ({} functions)",
        root.file_name().map_or_else(
            || root.display().to_string(),
            |n| n.to_string_lossy().into_owned()
        ),
        graph.len()
    );
    let mut viewer = Viewer::new(graph)
        .with_title(title)
        .with_trace_options(options);
    if let Some(entry) = entry {
        viewer = viewer.with_root(entry);
    }
    if let Some(path) = &cli.screenshot {
        viewer = viewer.with_screenshot(path);
    }
    if viewer.run().is_error() {
        bail!("viewer exited with an error");
    }
    Ok(())
}

fn find_entry(graph: &CallGraph, query: &str) -> anyhow::Result<FunctionId> {
    match graph.find(query).as_slice() {
        [id] => Ok(*id),
        [] => bail!("no function matches `{query}` (try --list)"),
        ids => {
            let names: Vec<String> = ids
                .iter()
                .filter_map(|id| graph.function(*id))
                .map(|f| format!("  {}", f.qualified_name()))
                .collect();
            bail!(
                "`{query}` is ambiguous; qualify it further:\n{}",
                names.join("\n")
            )
        }
    }
}

fn list(graph: &CallGraph) -> String {
    let mut rows: Vec<(String, String, bool)> = graph
        .functions()
        .map(|(id, f)| {
            (
                f.qualified_name(),
                f.location.to_string(),
                graph.callers(id).is_empty(),
            )
        })
        .collect();
    rows.sort();
    let mut out = String::new();
    for (name, location, is_entry) in rows {
        let marker = if is_entry { "*" } else { " " };
        let _ = writeln!(out, "{marker} {name}  {location}");
    }
    out.push_str("\n* = entry point (no callers in analyzed source)\n");
    out
}

fn render_trace(graph: &CallGraph, trace: &Trace) -> String {
    let mut out = String::new();
    for (_, node) in trace.nodes() {
        let name = match &node.target {
            TraceTarget::Function { id, resolution } => {
                let name = graph
                    .function(*id)
                    .map(Function::qualified_name)
                    .unwrap_or_default();
                match resolution {
                    codeviz_graph::Resolution::Exact => name,
                    codeviz_graph::Resolution::Inferred => format!("{name} [inferred]"),
                }
            }
            TraceTarget::Ambiguous(ids) => format!("<ambiguous: {} candidates>", ids.len()),
            TraceTarget::External(text) => format!("{text} [external]"),
        };
        let suffix = match node.stopped {
            Some(StopReason::Recursion) => " (recursion)",
            Some(StopReason::DepthLimit) => " (depth limit)",
            None => "",
        };
        let indent = "  ".repeat(node.depth as usize);
        let _ = writeln!(out, "{:>4}  {indent}{name}{suffix}", node.enter);
    }
    out
}

fn write_stdout(text: &str) -> anyhow::Result<()> {
    io::stdout().lock().write_all(text.as_bytes())?;
    Ok(())
}
