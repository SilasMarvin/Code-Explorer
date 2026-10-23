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
    /// The syntax alone: fast, but method calls on values of unknown type are matched
    /// by name.
    Syntax,
}

/// What to analyze, and how.
#[derive(Debug, Clone, clap::Args)]
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
    let document = analyze_dir(&root, &args.source, None)?;
    write(&document, args.output.as_ref())
}

impl Source {
    /// The same, resolving calls from the syntax alone: what a live view reads with, to
    /// keep up with every save.
    pub(crate) fn syntax_only(&self) -> Self {
        Self {
            resolve: Resolve::Syntax,
            ..self.clone()
        }
    }
}

/// Analyzes the source under `root`, in the language asked for or the one its files
/// suggest. rust-analyzer builds into `target_dir` when one is given.
pub(crate) fn analyze_dir(
    root: &Path,
    source: &Source,
    target_dir: Option<&Path>,
) -> anyhow::Result<Document> {
    Ok(analyze_source(root, source, target_dir)?.document)
}

/// A document analyzed from source, and the files in it that did not wholly parse,
/// each with the line where it stops.
#[derive(Debug)]
pub(crate) struct Analyzed {
    pub(crate) document: Document,
    pub(crate) unparsed: Vec<(PathBuf, usize)>,
}

/// Whether `root` is a Cargo package or workspace, or a directory of them such as a
/// workspace's `crates/`.
fn holds_cargo_packages(root: &Path) -> bool {
    root.join("Cargo.toml").exists()
        || std::fs::read_dir(root)
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .any(|entry| entry.path().join("Cargo.toml").exists())
}

/// [`analyze_dir`], also saying which files did not wholly parse.
pub(crate) fn analyze_source(
    root: &Path,
    source: &Source,
    target_dir: Option<&Path>,
) -> anyhow::Result<Analyzed> {
    let language = match source.lang {
        Some(language) => language,
        None if holds_cargo_packages(root) => Language::Rust,
        None => Language::Python,
    };
    if language == Language::Python {
        let analysis = codeviz_python::analyze_checked(root)?;
        return Ok(Analyzed {
            document: analysis.document,
            unparsed: analysis
                .syntax_errors
                .iter()
                .map(|e| (e.path.clone(), e.line))
                .collect(),
        });
    }
    let rust = |analysis: codeviz_rust::Analysis| Analyzed {
        unparsed: analysis
            .syntax_errors
            .iter()
            .map(|e| (e.path.clone(), e.line))
            .collect(),
        document: analysis.document,
    };
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
        return Ok(rust(codeviz_rust::analyze(root, &options)?));
    }
    eprintln!("rust-analyzer: resolving calls in {}", root.display());
    let syntax = options.clone();
    options.resolver = codeviz_rust::Resolver::RustAnalyzer {
        target_dir: target_dir.map(Path::to_path_buf),
    };
    match codeviz_rust::analyze(root, &options) {
        Ok(analysis) => Ok(rust(analysis)),
        Err(error) if source.resolve == Resolve::Auto => {
            eprintln!("rust-analyzer: {error:#}; resolving from the syntax instead");
            Ok(rust(codeviz_rust::analyze(root, &syntax)?))
        }
        Err(error) => Err(error.into()),
    }
}

/// The target directory of the Cargo workspace at `dir`, unless `CARGO_TARGET_DIR`
/// already picks one.
pub(crate) fn target_dir(dir: &Path) -> Option<PathBuf> {
    if std::env::var_os("CARGO_TARGET_DIR").is_some() {
        return None;
    }
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(dir)
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    metadata
        .get("target_directory")?
        .as_str()
        .map(PathBuf::from)
}

#[derive(Debug, clap::Args)]
pub(crate) struct ImportArgs {
    /// A Buildkite pipeline YAML file.
    pipeline: PathBuf,
    /// The pipeline slug; defaults to the file name.
    #[arg(long)]
    slug: Option<String>,
    /// Write the document here instead of stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,
}

pub(crate) fn import(args: &ImportArgs) -> anyhow::Result<()> {
    let slug = args.slug.clone().unwrap_or_else(|| {
        args.pipeline.file_stem().map_or_else(
            || "pipeline".to_owned(),
            |s| s.to_string_lossy().into_owned(),
        )
    });
    let document = codeviz_buildkite::import_file(&args.pipeline, &slug)?;
    write(&document, args.output.as_ref())
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

pub(crate) fn schema() -> anyhow::Result<()> {
    let schema = serde_json::to_string_pretty(&codeviz_ir::json_schema())?;
    writeln!(std::io::stdout().lock(), "{schema}")?;
    Ok(())
}

pub(crate) fn read(path: &PathBuf) -> anyhow::Result<Document> {
    let json =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    Document::from_json(&json)
        .with_context(|| format!("{} is not a codeviz document", path.display()))
}
