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
    /// Entry label (`POST /users`) or unit id suffix to start from.
    #[arg(short, long)]
    entry: Option<String>,
    /// Include calls nothing resolved.
    #[arg(long)]
    unresolved: bool,
    /// Start with summaries, or the units they refine.
    #[arg(long, value_enum, default_value_t = crate::journey::ZoomArg::Summary)]
    zoom: crate::journey::ZoomArg,
    /// Open on this outcome (as numbered by `codeviz journey`) instead of the happy path.
    #[arg(long)]
    scenario: Option<usize>,
    /// Open with the cursor this many steps into the journey.
    #[arg(long, default_value_t = 0)]
    step: usize,
    /// How the journey shows over the graph.
    #[arg(long, value_enum, default_value_t = JourneyArg::Docked)]
    journey: JourneyArg,
    /// What pulls the graph's nodes together.
    #[arg(long, value_enum, default_value_t = ClusterArg::Modules)]
    cluster: ClusterArg,
    /// Show only what is within this many steps of where the reveal starts; the rest
    /// dims, then shrinks to specks. Everything shows without it.
    #[arg(long, value_name = "STEPS")]
    reveal: Option<u32>,
    /// What the reveal starts from; by default the focus.
    #[arg(long, value_enum)]
    reveal_around: Option<AroundArg>,
    /// How the reveal measures nearness.
    #[arg(long, value_enum, default_value_t = NearArg::Calls)]
    reveal_by: NearArg,
    /// Open with this function (entry label or unit id suffix) focused in the graph.
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
    #[command(flatten)]
    third: Third,
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

/// What the third dimension shows at first.
#[derive(Debug, clap::Args)]
struct Third {
    /// Open with the third dimension showing this (Z toggles it).
    #[arg(long, value_enum, value_name = "VIEW")]
    z: Option<ZArg>,
    /// With `--z nucleus`, open with the nucleus expanded to show how calls run inside
    /// the focus's module (X toggles it).
    #[arg(long, requires = "z")]
    expand: bool,
    /// Open browsing, with no journey open and the focus selected, or without
    /// --focus, nothing: a click selects a function and moves nothing; Enter opens the
    /// selected function's journey.
    #[arg(long, conflicts_with = "expand")]
    browse: bool,
}

impl Third {
    fn apply(&self, mut viewer: Viewer) -> Viewer {
        if let Some(ZArg::Nucleus) = self.z {
            viewer = viewer.with_nucleus();
        }
        if self.expand {
            viewer = viewer.expanded();
        }
        if self.browse {
            viewer = viewer.browsing();
        }
        viewer
    }
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum JourneyArg {
    /// The graph alone.
    Hidden,
    /// Docked in a corner over the graph.
    Docked,
    /// Below a strip of graph.
    Full,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum ZArg {
    /// The focus's module a glass sphere in the middle, every other module a sphere
    /// around it; those holding its callers and callees near, opened to name them.
    Nucleus,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum AroundArg {
    /// The focused function.
    Focus,
    /// The functions the active journey runs.
    Journey,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum NearArg {
    /// Calls between, along call chains.
    Calls,
    /// Distance in the graph, in steps of a typical call's length.
    Space,
    /// Distance in the graph, with each call between counting against it.
    Blend,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum ClusterArg {
    /// Functions in the same container pull together.
    Modules,
    /// Only calls pull.
    Calls,
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
    let root = match &args.entry {
        Some(query) => Some(crate::journey::find_one(&system, query)?),
        None => None,
    };
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
    if let Some(root) = root {
        viewer = viewer.with_root(root);
    }
    if let Some(focus) = focus {
        viewer = viewer.with_focus(focus);
    }
    if let Some(hover) = hover {
        viewer = viewer.with_hover(hover);
    }
    if let Some(scenario) = args.scenario {
        viewer = viewer.with_scenario(scenario);
    }
    viewer = viewer
        .with_step(args.step)
        .with_journey(match args.journey {
            JourneyArg::Hidden => "hidden",
            JourneyArg::Docked => "docked",
            JourneyArg::Full => "full",
        })
        .with_clustering(match args.cluster {
            ClusterArg::Modules => "modules",
            ClusterArg::Calls => "calls",
        })
        .with_reveal(
            args.reveal_around.map(|around| match around {
                AroundArg::Focus => "focus",
                AroundArg::Journey => "journey",
            }),
            match args.reveal_by {
                NearArg::Calls => "calls",
                NearArg::Space => "space",
                NearArg::Blend => "blend",
            },
            args.reveal,
        );
    if let Some(depth) = args.depth {
        viewer = viewer.with_depth(depth);
    }
    if args.bare {
        viewer = viewer.bare();
    }
    viewer = args.third.apply(viewer);
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
