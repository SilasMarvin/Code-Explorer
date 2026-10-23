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
    let document = analyze_dir(&root, &args.source, None)?;
    write(&document, args.output.as_ref())
}

/// Analyzes the source under `root`, in the language asked for or the one its files
/// suggest. rust-analyzer builds into `target_dir` when one is given.
pub(crate) fn analyze_dir(
    root: &Path,
    source: &Source,
    target_dir: Option<&Path>,
) -> anyhow::Result<Document> {
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
    options.resolver = codeviz_rust::Resolver::RustAnalyzer {
        target_dir: target_dir.map(Path::to_path_buf),
    };
    match codeviz_rust::analyze(root, &options) {
        Ok(analysis) => Ok(analysis.document),
        Err(error) if source.resolve == Resolve::Auto => {
            eprintln!("rust-analyzer: {error:#}; resolving from the syntax instead");
            Ok(codeviz_rust::analyze(root, &syntax)?.document)
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

/// Reads a document for viewing, with its source found: under `root` when given, else
/// under the root it was saved with while that exists, else wherever the same
/// directory turns up around the document or the working directory. So a document
/// made on another machine, or from a checkout since removed, still shows its source.
pub(crate) fn read_rooted(path: &PathBuf, root: Option<&Path>) -> anyhow::Result<Document> {
    let mut document = read(path)?;
    if let Some(root) = root {
        let root = root
            .canonicalize()
            .with_context(|| format!("cannot open {}", root.display()))?;
        document.root = Some(file_uri(&root));
        return Ok(document);
    }
    let Some(stored) = document
        .root
        .as_deref()
        .and_then(|r| r.strip_prefix("file://"))
        .map(PathBuf::from)
    else {
        return Ok(document);
    };
    let samples = relative_files(&document);
    if stored.is_dir() || samples.is_empty() {
        return Ok(document);
    }
    let around = |dir: Option<PathBuf>| {
        dir.and_then(|d| d.canonicalize().ok())
            .map(|d| d.ancestors().map(Path::to_path_buf).collect::<Vec<_>>())
            .unwrap_or_default()
    };
    let mut bases = around(path.parent().map(|p| p.join(".")));
    for base in around(std::env::current_dir().ok()) {
        if !bases.contains(&base) {
            bases.push(base);
        }
    }
    match find_root(&stored, &bases, &samples) {
        Some(found) => {
            eprintln!(
                "{}: its source is not at {}; reading it from {}",
                path.display(),
                stored.display(),
                found.display()
            );
            document.root = Some(file_uri(&found));
        }
        None => eprintln!(
            "{}: cannot find its source (it was at {}); pass --root DIR",
            path.display(),
            stored.display()
        ),
    }
    Ok(document)
}

/// A few of the files a document's locations name relative to its root.
pub(crate) fn relative_files(document: &Document) -> Vec<String> {
    let mut files = Vec::new();
    for unit in &document.units {
        let calls = unit.calls.iter().filter_map(|c| c.location.as_ref());
        for location in unit.location.iter().chain(calls) {
            if !location.uri.contains("://") && !files.contains(&location.uri) {
                files.push(location.uri.clone());
            }
        }
    }
    files
}

/// Where a moved root went: the stored root's last components, longest first, under
/// each of `bases` nearest first (with no components, each base itself), taking the
/// first directory that holds most of `files`.
fn find_root(stored: &Path, bases: &[PathBuf], files: &[String]) -> Option<PathBuf> {
    let samples = &files[..files.len().min(5)];
    let holds = |dir: &Path| {
        let present = samples.iter().filter(|f| dir.join(f).is_file()).count();
        present > 0 && present * 2 >= samples.len()
    };
    let parts: Vec<_> = stored.components().skip(1).collect();
    (0..=parts.len()).find_map(|skip| {
        let tail: PathBuf = parts[skip..].iter().collect();
        bases
            .iter()
            .map(|base| base.join(&tail))
            .find(|dir| dir.is_dir() && holds(dir))
    })
}

pub(crate) fn file_uri(path: &Path) -> String {
    format!("file://{}", path.display())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_moved_root_by_its_last_components() {
        let scratch = std::env::temp_dir().join(format!("codeviz-root-{}", std::process::id()));
        let clone = scratch.join("elsewhere/shop");
        let service = clone.join("services/api");
        std::fs::create_dir_all(service.join("src")).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(service.join("src/main.rs"), "fn main() {}\n")
            .unwrap_or_else(|e| panic!("{e}"));
        let files = vec!["src/main.rs".to_owned(), "src/gone.rs".to_owned()];
        let stored = Path::new("/home/someone/shop/services/api");
        let bases: Vec<PathBuf> = clone
            .join("docs")
            .ancestors()
            .map(Path::to_path_buf)
            .collect();

        assert_eq!(
            find_root(stored, &bases, &files),
            Some(service.clone()),
            "found under the clone, at the same place in it"
        );
        assert_eq!(
            find_root(stored, std::slice::from_ref(&service), &files),
            Some(service.clone()),
            "or as the directory itself"
        );
        assert_eq!(
            find_root(stored, &bases, &["src/other.rs".to_owned()]),
            None,
            "only where the files are"
        );
        let _ = std::fs::remove_dir_all(&scratch);
    }
}
