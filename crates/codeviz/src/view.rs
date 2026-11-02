//! Opens IR documents in the Bevy viewer.

use std::path::PathBuf;

use anyhow::{Context, bail};
use codeviz_core::System;
use codeviz_ir::{Document, UnitId};
use codeviz_viewer::{Script, Viewer};

#[derive(Debug, clap::Args)]
pub(crate) struct ViewArgs {
    /// IR documents to load; later documents override earlier ones.
    #[arg(required = true)]
    documents: Vec<PathBuf>,
    #[command(flatten)]
    display: Display,
}

/// What the viewer opens on and how it shows it.
#[derive(Debug, clap::Args)]
struct Display {
    /// Open the journey that starts at this function (entry label or unit id suffix).
    /// Without it, the view opens browsing the whole codebase.
    #[arg(long, value_name = "UNIT")]
    focus: Option<String>,
    /// Open the journey this many levels deep instead of one.
    #[arg(long, value_name = "LEVELS")]
    depth: Option<u32>,
    /// Open with the journey's start module expanded to show how calls run inside it
    /// (X toggles it).
    #[arg(long, requires = "focus")]
    expand: bool,
    /// Render once, save a PNG screenshot here, and exit.
    #[arg(long, value_name = "PATH")]
    screenshot: Option<PathBuf>,
    /// Run a session from this file in one window, a step per line (`key x`, `click
    /// UNIT`, `click-label UNIT`, `hover UNIT`, `doubleclick UNIT`, `bar 1.2`, `button
    /// TEXT`, `drag DX DY`, `scroll N`, `wait MS`, `snap LABEL`); --screenshot names the
    /// directory each snap's PNG and JSON go in.
    #[arg(long, value_name = "FILE", requires = "screenshot")]
    script: Option<PathBuf>,
}

pub(crate) fn run(args: &ViewArgs) -> anyhow::Result<()> {
    let documents = args
        .documents
        .iter()
        .map(crate::ir::read)
        .collect::<anyhow::Result<Vec<_>>>()?;
    let title = args
        .documents
        .iter()
        .filter_map(|p| p.file_stem())
        .map(|s| s.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" + ");
    open(documents, &args.display, &title)
}

/// Opens `documents` and blocks until the window closes.
fn open(documents: Vec<Document>, args: &Display, title: &str) -> anyhow::Result<()> {
    let system = System::new(documents);
    let focus = args
        .focus
        .as_ref()
        .map(|query| find_one(&system, query))
        .transpose()?;
    let script = read_script(args, &system)?;
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
    if viewer.run().is_error() {
        bail!("viewer exited with an error");
    }
    Ok(())
}

/// The session `--script` names, its functions looked up in `system`.
fn read_script(args: &Display, system: &System) -> anyhow::Result<Option<Script>> {
    let Some(path) = &args.script else {
        return Ok(None);
    };
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let find = |query: &str| find_one(system, query).map_err(|error| format!("{error:#}"));
    Script::parse(&text, &find)
        .map(Some)
        .map_err(anyhow::Error::msg)
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
