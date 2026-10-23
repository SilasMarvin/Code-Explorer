//! Commands that operate on IR documents.

use std::{
    io::Write as _,
    path::{Path, PathBuf},
    sync::Once,
    time::Instant,
};

use anyhow::{Context, bail};
use codeviz_ir::{Certainty, Document, Severity};

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

impl Source {
    /// The flags that ask for this, where it is not the default.
    pub(crate) fn flags(&self) -> Vec<String> {
        let mut flags = Vec::new();
        if let Some(language) = self.lang {
            let name = match language {
                Language::Rust => "rust",
                Language::Python => "python",
            };
            flags.extend(["--lang".to_owned(), name.to_owned()]);
        }
        if self.include_tests {
            flags.push("--include-tests".to_owned());
        }
        let resolve = match self.resolve {
            Resolve::Auto => None,
            Resolve::RustAnalyzer => Some("rust-analyzer"),
            Resolve::Syntax => Some("syntax"),
        };
        if let Some(resolve) = resolve {
            flags.extend(["--resolve".to_owned(), resolve.to_owned()]);
        }
        flags
    }
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
    let label = root.display().to_string();
    let analysis = analyze_dir(&root, &args.source, None, &label)?;
    eprintln!("{}", analysis.summary(&label));
    write(&analysis.document, args.output.as_ref())
}

/// How an analysis resolved calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Resolved {
    RustAnalyzer,
    /// From the syntax alone, where some calls are matched by name.
    Syntax,
}

/// A document, how its calls were resolved, and how long it took.
#[derive(Debug)]
pub(crate) struct Analysis {
    pub(crate) document: Document,
    pub(crate) resolved: Resolved,
    pub(crate) seconds: f32,
}

impl Analysis {
    /// One line on how well calls resolved: how many call sites name what they call
    /// for sure, how many only by a guess, and how many name nothing analyzed here.
    pub(crate) fn summary(&self, label: &str) -> String {
        let (mut resolved, mut guessed, mut unresolved) = (0, 0, 0);
        for call in self.document.units.iter().flat_map(|unit| &unit.calls) {
            if call.targets.is_empty() {
                unresolved += 1;
            } else if call
                .targets
                .iter()
                .any(|t| t.certainty == Certainty::Inferred)
            {
                guessed += 1;
            } else {
                resolved += 1;
            }
        }
        let how = match self.resolved {
            Resolved::RustAnalyzer => "by rust-analyzer",
            Resolved::Syntax => "from the syntax",
        };
        format!(
            "{label}: {} call sites resolved {how} in {:.1}s: {resolved} exact, {guessed} \
             guessed by name, {unresolved} unresolved (std, dependencies or unknown)",
            resolved + guessed + unresolved,
            self.seconds,
        )
    }
}

/// Said once, however many analyses fall back.
const NO_RUST_ANALYZER: &str = "warning: rust-analyzer is not installed, so Rust calls are \
    resolved from the syntax alone\n         and method calls on values of unknown type are \
    guessed by name or left out.\n         Install it with `rustup component add \
    rust-analyzer` (or `mise install` in a repository that pins it),\n         or pass \
    `--resolve syntax` to choose this.";

/// Whether analyzing with these options may run rust-analyzer.
pub(crate) fn uses_rust_analyzer(source: &Source) -> bool {
    source.lang != Some(Language::Python)
        && match source.resolve {
            Resolve::Syntax => false,
            Resolve::RustAnalyzer => true,
            Resolve::Auto => codeviz_rust::rust_analyzer_available(),
        }
}

/// Analyzes the source under `root`, in the language asked for or the one its files
/// suggest, saying what it does under `label`. rust-analyzer builds into `target_dir`
/// when one is given.
pub(crate) fn analyze_dir(
    root: &Path,
    source: &Source,
    target_dir: Option<&Path>,
    label: &str,
) -> anyhow::Result<Analysis> {
    static WARNED: Once = Once::new();
    let started = Instant::now();
    let done = |document: Document, resolved: Resolved| Analysis {
        document,
        resolved,
        seconds: started.elapsed().as_secs_f32(),
    };
    let language = match source.lang {
        Some(language) => language,
        None if root.join("Cargo.toml").exists() => Language::Rust,
        None => Language::Python,
    };
    if language == Language::Python {
        return Ok(done(codeviz_python::analyze(root)?, Resolved::Syntax));
    }
    let mut options = codeviz_rust::AnalyzeOptions {
        include_tests: source.include_tests,
        resolver: codeviz_rust::Resolver::Syntax,
    };
    let use_rust_analyzer = match source.resolve {
        Resolve::Syntax => false,
        Resolve::RustAnalyzer => true,
        Resolve::Auto => {
            let available = codeviz_rust::rust_analyzer_available();
            if !available {
                WARNED.call_once(|| eprintln!("{NO_RUST_ANALYZER}"));
            }
            available
        }
    };
    if !use_rust_analyzer {
        let document = codeviz_rust::analyze(root, &options)?.document;
        return Ok(done(document, Resolved::Syntax));
    }
    eprintln!("{label}: resolving calls with rust-analyzer…");
    let syntax = options.clone();
    options.resolver = codeviz_rust::Resolver::RustAnalyzer {
        target_dir: target_dir.map(Path::to_path_buf),
    };
    match codeviz_rust::analyze(root, &options) {
        Ok(analysis) => Ok(done(analysis.document, Resolved::RustAnalyzer)),
        Err(error) if source.resolve == Resolve::Auto => {
            eprintln!(
                "warning: {label}: rust-analyzer failed, so calls are resolved from the syntax \
                 alone and method calls are guessed by name or left out:\n{error:#}"
            );
            let document = codeviz_rust::analyze(root, &syntax)?.document;
            Ok(done(document, Resolved::Syntax))
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

    #[test]
    fn summary_counts_call_sites_by_how_they_resolved() {
        let target = |unit: &str, certainty: &str| {
            format!(r#"{{ "unit": "x:{unit}", "certainty": "{certainty}" }}"#)
        };
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }},
              "units": [{{ "id": "x:a", "name": "a", "kind": "fn", "role": "invocable", "calls": [
                {{ "key": "b#0", "text": "b", "targets": [{}] }},
                {{ "key": "c#0", "text": "c", "targets": [{}, {}] }},
                {{ "key": "len#0", "text": "len", "targets": [] }}
              ] }}] }}"#,
            target("b", "exact"),
            target("c", "exact"),
            target("d", "inferred"),
        );
        let analysis = Analysis {
            document: Document::from_json(&json).unwrap_or_else(|e| panic!("{e}")),
            resolved: Resolved::Syntax,
            seconds: 0.25,
        };
        assert_eq!(
            analysis.summary("app"),
            "app: 3 call sites resolved from the syntax in 0.2s: 1 exact, 1 guessed by name, \
             1 unresolved (std, dependencies or unknown)"
        );
    }
}
