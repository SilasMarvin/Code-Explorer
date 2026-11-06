//! Opens IR documents in the Bevy viewer.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use codeviz_core::System;
use codeviz_ir::{Document, UnitId};
use codeviz_viewer::{Script, Viewer};

#[derive(Debug, clap::Args)]
pub(crate) struct ViewArgs {
    /// A Rust project's directory (analyzed first), or IR documents (.json); later
    /// documents override earlier ones.
    #[arg(default_value = ".")]
    paths: Vec<PathBuf>,
    #[command(flatten)]
    source: crate::analyze::Source,
    #[command(flatten)]
    display: Display,
}

/// What the viewer opens on and how it shows it.
#[derive(Debug, clap::Args)]
struct Display {
    /// Open the journey that starts at this function: an entry's label (`POST /users`)
    /// or the end of its path (`orders::place`). Without it, the view opens browsing
    /// the whole codebase.
    #[arg(long, value_name = "UNIT")]
    focus: Option<String>,
    /// Open the journey this many levels deep instead of one.
    #[arg(long, value_name = "LEVELS")]
    depth: Option<u32>,
    /// Open with the journey's start module expanded to show how calls run inside it
    /// (X toggles it).
    #[arg(long, requires = "focus")]
    expand: bool,
    /// With --script, the directory each snap's PNG and JSON go in.
    #[arg(long, value_name = "DIR", requires = "script", hide = true)]
    screenshot: Option<PathBuf>,
    /// Run a session from this file in one window, a step per line (`key x`, `click
    /// UNIT`, `expect mode journey`, `snap LABEL`; see docs/harness.md), checking every
    /// snap. Run it through tools/drive, which guards memory and time.
    #[arg(long, value_name = "FILE", requires = "screenshot", hide = true)]
    script: Option<PathBuf>,
    /// Read this session script and look up every function and module it names, then
    /// exit without opening the window.
    #[arg(long, value_name = "FILE", conflicts_with = "script", hide = true)]
    check_script: Option<PathBuf>,
}

pub(crate) fn run(args: &ViewArgs) -> anyhow::Result<()> {
    let documents = args
        .paths
        .iter()
        .map(|path| document(path, &args.source))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let title = args
        .paths
        .iter()
        .map(|path| name(path))
        .collect::<Vec<_>>()
        .join(" + ");
    open(documents, &args.display, &title)
}

/// The document `path` stands for: a directory analyzed, or an IR document read.
fn document(path: &Path, source: &crate::analyze::Source) -> anyhow::Result<Document> {
    const PASS: &str = "pass the project's directory, or an IR document (.json)";
    let shown = path.display();
    if path.is_dir() {
        return crate::analyze::analyze(path, source);
    }
    if !path.exists() {
        bail!("{shown} does not exist: {PASS}");
    }
    match path.extension().and_then(|e| e.to_str()) {
        Some("json") => crate::ir::read(path).map_err(|error| {
            anyhow::anyhow!(
                "{error:#}\n{PASS}; `codeviz analyze` writes one, docs/ir.md describes it"
            )
        }),
        Some("rs") => bail!("{shown} is a Rust source file: {PASS}"),
        _ => bail!("{shown} is neither a directory nor an IR document: {PASS}"),
    }
}

/// What the window calls `path`: a document's file name without `.json`, a
/// directory's own name.
fn name(path: &Path) -> String {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let name = if path.is_dir() {
        path.file_name()
    } else {
        path.file_stem()
    };
    name.map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Opens `documents` and blocks until the window closes.
fn open(documents: Vec<Document>, args: &Display, title: &str) -> anyhow::Result<()> {
    let system = System::new(documents);
    let focus = args
        .focus
        .as_ref()
        .map(|query| find_one(&system, query))
        .transpose()?;
    let script = read_script(args)?;
    let title = format!("codeviz — {title}");

    let mut viewer = Viewer::new(system).with_title(title);
    if let Some(focus) = focus {
        viewer = viewer.with_focus(focus);
    }
    if let Some(depth) = args.depth {
        viewer = viewer.with_depth(depth);
    }
    if args.expand {
        viewer = viewer.expanded();
    }
    if let Some(path) = &args.screenshot {
        viewer = viewer.with_screenshot(path);
    }
    if let Some(script) = script {
        viewer = viewer.with_script(script);
    }
    if args.check_script.is_some() {
        viewer = viewer.checking_script();
    }
    if viewer.run().is_error() {
        bail!("viewer exited with an error");
    }
    Ok(())
}

/// The session `--script` (or `--check-script`) names.
fn read_script(args: &Display) -> anyhow::Result<Option<Script>> {
    let Some(path) = args.script.as_ref().or(args.check_script.as_ref()) else {
        return Ok(None);
    };
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    Script::parse(&text).map(Some).map_err(anyhow::Error::msg)
}

/// The one function `query` names: an entry's label or the end of a unit's id.
fn find_one(system: &System, query: &str) -> anyhow::Result<UnitId> {
    match system.find(query).as_slice() {
        [unit] => Ok(unit.id.clone()),
        [] => bail!("nothing matches `{query}`"),
        many => bail!(
            "`{query}` is ambiguous:\n{}",
            many.iter()
                .map(|u| format!("  {}", u.id))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::*;

    #[derive(clap::Parser)]
    struct Defaults {
        #[command(flatten)]
        source: crate::analyze::Source,
    }

    #[test]
    fn a_path_that_is_not_a_project_or_a_document_says_what_to_pass() {
        let dir = std::env::temp_dir().join(format!("codeviz-view-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.join("other.json"), r#"{ "a": 1 }"#).unwrap();
        let source = Defaults::parse_from(["test"]).source;
        let said: Vec<String> = ["main.rs", "missing", "other.json"]
            .iter()
            .map(|name| format!("{:#}", document(&dir.join(name), &source).unwrap_err()))
            .collect();
        std::fs::remove_dir_all(&dir).unwrap();
        for (said, what) in
            said.iter()
                .zip(["a Rust source file", "does not exist", "missing field"])
        {
            assert!(said.contains(what), "{said}");
            assert!(
                said.contains("pass the project's directory, or an IR document (.json)"),
                "{said}"
            );
        }
    }
}
