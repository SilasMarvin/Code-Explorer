//! Commands that operate on IR documents.

use std::{
    io::Write as _,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use codeviz_ir::{Document, Severity};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Language {
    Rust,
    Python,
}

/// How Rust calls are resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Resolve {
    /// rust-analyzer when it is installed, the syntax otherwise.
    Auto,
    /// rust-analyzer, which type-checks the workspace: exact, but slow.
    RustAnalyzer,
    /// The syntax alone: fast, but method calls on values of unknown type are left
    /// unresolved.
    Syntax,
}

/// What to analyze, and how.
#[derive(Debug, clap::Args)]
pub(crate) struct Source {
    /// Source language; detected from the directory when omitted.
    #[arg(short, long, value_enum)]
    lang: Option<Language>,
    /// Include `#[cfg(test)]` code.
    #[arg(long)]
    include_tests: bool,
    /// How Rust calls are resolved.
    #[arg(long, value_enum, default_value_t = Resolve::Auto)]
    resolve: Resolve,
}

#[derive(Debug, clap::Args)]
pub(crate) struct AnalyzeArgs {
    /// Directory to analyze.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Write the document here instead of stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,
    #[command(flatten)]
    source: Source,
}

pub(crate) fn analyze(args: &AnalyzeArgs) -> anyhow::Result<()> {
    let root = args
        .path
        .canonicalize()
        .with_context(|| format!("cannot open {}", args.path.display()))?;
    let document = analyze_dir(&root, &args.source)?;
    write(&document, args.output.as_ref())
}

/// Analyzes the source under `root`, in the language asked for or the one its files
/// suggest.
fn analyze_dir(root: &Path, source: &Source) -> anyhow::Result<Document> {
    let language = match source.lang {
        Some(language) => language,
        None if root.join("Cargo.toml").exists() => Language::Rust,
        None => Language::Python,
    };
    if language == Language::Python {
        return Ok(codeviz_python::analyze(root)?);
    }
    let mut options = codeviz_rust::AnalyzeOptions {
        include_tests: source.include_tests,
        resolver: codeviz_rust::Resolver::Syntax,
    };
    let use_rust_analyzer = match source.resolve {
        Resolve::Syntax => false,
        Resolve::RustAnalyzer => true,
        Resolve::Auto => codeviz_rust::rust_analyzer_available(),
    };
    if !use_rust_analyzer {
        return Ok(codeviz_rust::analyze(root, &options)?.document);
    }
    eprintln!("rust-analyzer: resolving calls in {}", root.display());
    let syntax = options.clone();
    options.resolver = codeviz_rust::Resolver::RustAnalyzer { target_dir: None };
    match codeviz_rust::analyze(root, &options) {
        Ok(analysis) => Ok(analysis.document),
        Err(error) if source.resolve == Resolve::Auto => {
            eprintln!("rust-analyzer: {error:#}; resolving from the syntax instead");
            Ok(codeviz_rust::analyze(root, &syntax)?.document)
        }
        Err(error) => Err(error.into()),
    }
}

fn write(document: &Document, output: Option<&PathBuf>) -> anyhow::Result<()> {
    let json = document.to_json()?;
    match output {
        Some(path) => std::fs::write(path, json + "\n")
            .with_context(|| format!("cannot write {}", path.display()))?,
        None => writeln!(std::io::stdout().lock(), "{json}")?,
    }
    Ok(())
}

#[derive(Debug, clap::Args)]
pub(crate) struct ValidateArgs {
    /// IR documents to check.
    #[arg(required = true)]
    files: Vec<PathBuf>,
}

pub(crate) fn validate(args: &ValidateArgs) -> anyhow::Result<()> {
    let mut errors = 0;
    for path in &args.files {
        let document = read(path)?;
        let diagnostics = codeviz_ir::validate(&document);
        let file_errors = diagnostics
            .iter()
            .filter(|d| d.severity() == Severity::Error)
            .count();
        println!(
            "{}: {} units, {} entries, {file_errors} errors, {} warnings",
            path.display(),
            document.units.len(),
            document.entries.len(),
            diagnostics.len() - file_errors,
        );
        for diagnostic in &diagnostics {
            println!("  {diagnostic}");
        }
        errors += file_errors;
    }
    if errors > 0 {
        bail!("{errors} validation errors");
    }
    Ok(())
}

pub(crate) fn read(path: &PathBuf) -> anyhow::Result<Document> {
    let json =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    Document::from_json(&json)
        .with_context(|| format!("{} is not a codeviz document", path.display()))
}
