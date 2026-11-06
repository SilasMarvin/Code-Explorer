//! Analyzing a Rust project into an IR document, saying on stderr what it is doing.

use std::{
    io::IsTerminal as _,
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, Instant},
};

use anyhow::bail;
use codeviz_ir::{Document, Role};
use codeviz_rust::{AnalyzeOptions, Resolver};

/// How calls are resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Resolve {
    /// rust-analyzer when it is installed, the syntax otherwise.
    Auto,
    /// rust-analyzer, which type-checks the workspace: exact, but slow.
    RustAnalyzer,
    /// the syntax alone: fast, but method calls on values of unknown type are left
    /// unresolved.
    Syntax,
}

/// How a Rust project is analyzed.
#[derive(Debug, clap::Args)]
pub(crate) struct Source {
    /// Include `#[cfg(test)]` code.
    #[arg(long)]
    include_tests: bool,
    /// How calls are resolved.
    #[arg(long, value_enum, default_value_t = Resolve::Auto)]
    resolve: Resolve,
}

#[derive(Debug, clap::Args)]
pub(crate) struct AnalyzeArgs {
    /// The Rust project to analyze: a directory with a Cargo.toml, or one holding
    /// Cargo packages.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Write the document here instead of stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,
    #[command(flatten)]
    source: Source,
}

pub(crate) fn run(args: &AnalyzeArgs) -> anyhow::Result<()> {
    let document = analyze(&args.path, &args.source)?;
    crate::ir::write(&document, args.output.as_ref())
}

/// Analyzes the Rust project at `path`, with rust-analyzer when asked for or
/// available, saying on stderr what it found and how long each part took.
pub(crate) fn analyze(path: &Path, source: &Source) -> anyhow::Result<Document> {
    let Ok(root) = path.canonicalize() else {
        bail!("no such directory: {}", path.display());
    };
    let start = Instant::now();
    say(&format!("analyzing {}", root.display()));
    let syntax = AnalyzeOptions {
        include_tests: source.include_tests,
        resolver: Resolver::Syntax,
    };
    let parsed = match codeviz_rust::analyze(&root, &syntax) {
        Ok(analysis) => analysis,
        Err(codeviz_rust::Error::NoPackages(_)) => bail!(
            "no Cargo package with a src/ directory under {}: point codeviz at a Rust \
             project, a directory with a Cargo.toml",
            root.display()
        ),
        Err(error) => return Err(error.into()),
    };
    say(&format!(
        "found {}, {}, {} ({:.1} s)",
        plural(parsed.packages.len(), "package"),
        plural(parsed.files_parsed, "source file"),
        plural(functions(&parsed.document), "function"),
        start.elapsed().as_secs_f64(),
    ));

    let document = match source.resolve {
        Resolve::Syntax => parsed.document,
        Resolve::Auto if !codeviz_rust::rust_analyzer_available() => {
            say(
                "rust-analyzer not found: resolving calls from the syntax alone, which \
                 misses method calls on values of unknown type. For exact calls: \
                 `rustup component add rust-analyzer`",
            );
            parsed.document
        }
        Resolve::RustAnalyzer if !codeviz_rust::rust_analyzer_available() => bail!(
            "rust-analyzer not found: `rustup component add rust-analyzer`, or pass \
             --resolve syntax"
        ),
        resolve => match rust_analyzer(&root, source.include_tests) {
            Ok(document) => document,
            Err(error) if resolve == Resolve::Auto => {
                let error = anyhow::Error::new(error);
                say(&format!(
                    "rust-analyzer failed, so calls are resolved from the syntax alone \
                     (pass --resolve syntax to skip it):\n{error:#}"
                ));
                parsed.document
            }
            Err(error) => {
                return Err(anyhow::Error::new(error).context(
                    "rust-analyzer failed (does `cargo check` pass?); pass --resolve syntax \
                     to resolve calls from the syntax alone",
                ));
            }
        },
    };
    let (calls, resolved) = calls(&document);
    say(&format!(
        "{} of {} reach the project's own functions, {} ({:.1} s in all)",
        resolved,
        plural(calls, "call"),
        plural(document.entries.len(), "entry"),
        start.elapsed().as_secs_f64(),
    ));
    Ok(document)
}

/// Runs the analysis again with rust-analyzer resolving calls, saying how long it takes
/// while it runs.
fn rust_analyzer(root: &Path, include_tests: bool) -> Result<Document, codeviz_rust::Error> {
    say(
        "rust-analyzer: type-checking the workspace to resolve calls (it builds build \
         scripts and proc macros first, so a large workspace takes minutes)",
    );
    let options = AnalyzeOptions {
        include_tests,
        resolver: Resolver::RustAnalyzer { target_dir: None },
    };
    let start = Instant::now();
    let analysis = ticking("rust-analyzer: running", || {
        codeviz_rust::analyze(root, &options)
    })?;
    say(&format!(
        "rust-analyzer: done ({:.1} s)",
        start.elapsed().as_secs_f64()
    ));
    Ok(analysis.document)
}

/// Runs `work`, showing `label` and the seconds it has taken on stderr: every second
/// on one line rewritten in a terminal, every 30 seconds otherwise.
fn ticking<T>(label: &str, work: impl FnOnce() -> T) -> T {
    let terminal = std::io::stderr().is_terminal();
    let every = Duration::from_secs(if terminal { 1 } else { 30 });
    let start = Instant::now();
    let (done, stop) = mpsc::channel::<()>();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            while stop.recv_timeout(every) == Err(mpsc::RecvTimeoutError::Timeout) {
                let seconds = start.elapsed().as_secs();
                if terminal {
                    eprint!("\r  {label} {seconds} s");
                } else {
                    eprintln!("  {label} {seconds} s");
                }
            }
            if terminal {
                eprint!("\r\x1b[K");
            }
        });
        let result = work();
        drop(done);
        result
    })
}

fn say(line: &str) {
    eprintln!("{line}");
}

/// The functions in `document`: every unit that runs.
fn functions(document: &Document) -> usize {
    document
        .units
        .iter()
        .filter(|u| u.role == Role::Invocable)
        .count()
}

/// The calls in `document`, and how many of them resolved to something.
fn calls(document: &Document) -> (usize, usize) {
    let calls = document.units.iter().flat_map(|u| &u.calls);
    let resolved = calls.clone().filter(|c| !c.targets.is_empty()).count();
    (calls.count(), resolved)
}

/// `n` and `noun`, made plural unless `n` is 1.
pub(crate) fn plural(n: usize, noun: &str) -> String {
    match (n, noun.strip_suffix('y')) {
        (1, _) => format!("1 {noun}"),
        (_, Some(stem)) => format!("{n} {stem}ies"),
        _ => format!("{n} {noun}s"),
    }
}
