//! Opens IR documents in the Bevy viewer.

use std::path::PathBuf;

use anyhow::{Context, bail};
use codeviz_core::{JourneyOptions, System};
use codeviz_ir::Document;
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
    /// Include calls nothing resolved.
    #[arg(long)]
    unresolved: bool,
    /// Start with summaries, or the units they refine.
    #[arg(long, value_enum, default_value_t = crate::journey::ZoomArg::Summary)]
    zoom: crate::journey::ZoomArg,
    /// Open the journey that starts at this function (entry label or unit id suffix).
    /// Without it, the view opens browsing the whole codebase.
    #[arg(long, value_name = "UNIT")]
    focus: Option<String>,
    /// With --screenshot, as though the pointer were over this function (entry label
    /// or unit id suffix).
    #[arg(long, value_name = "UNIT", requires = "screenshot")]
    hover: Option<String>,
    /// Open the journey this many levels deep instead of one.
    #[arg(long, value_name = "LEVELS")]
    depth: Option<u32>,
    /// Draw only the graph and the journey, without the panels and cards around them.
    #[arg(long)]
    bare: bool,
    /// Open with the journey's start module expanded to show how calls run inside it
    /// (X toggles it).
    #[arg(long, requires = "focus")]
    expand: bool,
    /// Render once, save a PNG screenshot here, and exit.
    #[arg(long, value_name = "PATH")]
    screenshot: Option<PathBuf>,
    /// With --screenshot, keys to press first, separated by spaces: `o`, `.`, `up`,
    /// `shift+]`.
    #[arg(long, value_name = "KEYS", requires = "screenshot")]
    keys: Option<String>,
    /// Run a session from this file in one window, a step per line (`key x`, `click
    /// UNIT`, `click-label UNIT`, `hover UNIT`, `doubleclick UNIT`, `bar 1.2`, `button
    /// TEXT`, `drag DX DY`, `scroll N`, `wait MS`, `snap LABEL`); --screenshot names the
    /// directory each snap's PNG and JSON go in.
    #[arg(
        long,
        value_name = "FILE",
        requires = "screenshot",
        conflicts_with = "keys"
    )]
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
        .map(|query| crate::journey::find_one(&system, query))
        .transpose()?;
    let hover = args
        .hover
        .as_ref()
        .map(|query| crate::journey::find_one(&system, query))
        .transpose()?;
    let script = read_script(args, &system)?;
    let title = format!("codeviz — {title}");

    let mut viewer = Viewer::new(system)
        .with_title(title)
        .with_options(JourneyOptions {
            include_unresolved: args.unresolved,
            zoom: args.zoom.into(),
            ..JourneyOptions::default()
        });
    if let Some(focus) = focus {
        viewer = viewer.with_focus(focus);
    }
    if let Some(hover) = hover {
        viewer = viewer.with_hover(hover);
    }
    if let Some(depth) = args.depth {
        viewer = viewer.with_depth(depth);
    }
    if args.bare {
        viewer = viewer.bare();
    }
    if args.expand {
        viewer = viewer.expanded();
    }
    if let Some(path) = &args.screenshot {
        viewer = viewer.with_screenshot(path);
    }
    if let Some(keys) = &args.keys {
        viewer = viewer.with_keys(keys).map_err(anyhow::Error::msg)?;
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
    let find =
        |query: &str| crate::journey::find_one(system, query).map_err(|error| format!("{error:#}"));
    Script::parse(&text, &find)
        .map(Some)
        .map_err(anyhow::Error::msg)
}
