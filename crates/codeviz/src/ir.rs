//! Reading, writing and checking IR documents.

use std::{
    io::Write as _,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use codeviz_ir::{Document, Severity};

use crate::analyze::plural;

/// Writes `document` to `output`, or to stdout.
pub(crate) fn write(document: &Document, output: Option<&PathBuf>) -> anyhow::Result<()> {
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
            "{}: {}, {}, {}, {}",
            path.display(),
            plural(document.units.len(), "unit"),
            plural(document.entries.len(), "entry"),
            plural(file_errors, "error"),
            plural(diagnostics.len() - file_errors, "warning"),
        );
        for diagnostic in &diagnostics {
            println!("  {diagnostic}");
        }
        errors += file_errors;
    }
    if errors > 0 {
        bail!("{}", plural(errors, "validation error"));
    }
    Ok(())
}

pub(crate) fn read(path: &Path) -> anyhow::Result<Document> {
    let json =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    Document::from_json(&json)
        .with_context(|| format!("{} is not a codeviz document", path.display()))
}
