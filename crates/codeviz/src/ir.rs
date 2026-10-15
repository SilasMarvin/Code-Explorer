//! Commands that operate on IR documents.

use std::{io::Write as _, path::PathBuf};

use anyhow::{Context, bail};
use codeviz_ir::{Document, Severity};

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Language {
    Rust,
    Python,
}

#[derive(Debug, clap::Args)]
pub(crate) struct AnalyzeArgs {
    /// Directory to analyze.
    #[arg(default_value = ".")]
    path: PathBuf,
    /// Source language; detected from the directory when omitted.
    #[arg(short, long, value_enum)]
    lang: Option<Language>,
    /// Write the document here instead of stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Include `#[cfg(test)]` code.
    #[arg(long)]
    include_tests: bool,
}

pub(crate) fn analyze(args: &AnalyzeArgs) -> anyhow::Result<()> {
    let root = args
        .path
        .canonicalize()
        .with_context(|| format!("cannot open {}", args.path.display()))?;
    let language = match args.lang {
        Some(language) => language,
        None if root.join("Cargo.toml").exists() => Language::Rust,
        None => Language::Python,
    };
    let document = match language {
        Language::Rust => {
            let options = codeviz_rust::AnalyzeOptions {
                include_tests: args.include_tests,
            };
            codeviz_rust::analyze(&root, options)?.document
        }
        Language::Python => codeviz_python::analyze(&root)?,
    };
    write(&document, args.output.as_ref())
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
