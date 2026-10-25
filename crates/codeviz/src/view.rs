//! Opens IR documents in the Bevy viewer.

use std::path::PathBuf;

use anyhow::bail;
use codeviz_core::{JourneyOptions, System};
use codeviz_ir::Document;
use codeviz_viewer::Viewer;

#[derive(Debug, clap::Args)]
pub(crate) struct ViewArgs {
    /// IR documents to load; later documents override earlier ones.
    #[arg(required = true)]
    documents: Vec<PathBuf>,
    /// Compare against these documents, an older version of the same system, and draw
    /// what changed over the graph.
    #[arg(long, value_name = "PATH", num_args = 1..)]
    before: Vec<PathBuf>,
    #[command(flatten)]
    display: Display,
}

/// What the viewer opens on and how it shows it.
#[derive(Debug, clap::Args)]
pub(crate) struct Display {
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
    /// How to draw the diff.
    #[arg(long, value_enum, default_value_t = DiffStyleArg::Status)]
    diff_style: DiffStyleArg,
    /// Hold the morph at this point, from 0 (before) to 1 (after), instead of playing it.
    #[arg(long, value_name = "T")]
    morph: Option<f32>,
    /// Show only what is within this many steps of where the reveal starts; the rest
    /// dims, then shrinks to specks. Everything shows without it.
    #[arg(long, value_name = "STEPS")]
    reveal: Option<u32>,
    /// What the reveal starts from; by default a diff's changes, else the focus.
    #[arg(long, value_enum)]
    reveal_around: Option<AroundArg>,
    /// Open a diff with what did not change showing too, not only the changes.
    #[arg(long)]
    show_unchanged: bool,
    /// How the reveal measures nearness.
    #[arg(long, value_enum, default_value_t = NearArg::Calls)]
    reveal_by: NearArg,
    /// Open with this function (entry label or unit id suffix) focused in the graph.
    #[arg(long, value_name = "UNIT")]
    focus: Option<String>,
    /// Open the journey this many levels deep instead of one.
    #[arg(long, value_name = "LEVELS")]
    depth: Option<u32>,
    /// Draw only the graph and the journey, without the panels and cards around them.
    #[arg(long)]
    bare: bool,
    /// Show the journey's order as depth (Z cycles it).
    #[arg(long, value_enum)]
    time: Option<TimeArg>,
    /// Render once, save a PNG screenshot here, and exit.
    #[arg(long, value_name = "PATH")]
    screenshot: Option<PathBuf>,
    /// With --screenshot, keys to press first, separated by spaces: `o`, `.`, `up`,
    /// `shift+]`.
    #[arg(long, value_name = "KEYS", requires = "screenshot")]
    keys: Option<String>,
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
enum DiffStyleArg {
    /// What changed, and the calls that came and went.
    Status,
    /// Everything that can reach a change, out to the entry points.
    Impact,
    /// The graph before, then after.
    Morph,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum AroundArg {
    /// The focused function.
    Focus,
    /// A diff's changes, counting only towards their callers.
    Changes,
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
enum TimeArg {
    /// Each call moves straight off the map's plane by when it runs: what ran behind,
    /// what is to come nearer.
    Fly,
    /// Each call moves along the line of sight instead, so it keeps its spot on screen.
    Held,
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
    let before = args
        .before
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
    open(documents, before, &args.display, &title)
}

/// Opens `documents`, diffed against `before` unless it is empty, and blocks until the
/// window closes.
pub(crate) fn open(
    documents: Vec<Document>,
    before: Vec<Document>,
    args: &Display,
    title: &str,
) -> anyhow::Result<()> {
    let system = System::new(documents.clone());
    let root = match &args.entry {
        Some(query) => Some(crate::journey::find_one(&system, query)?),
        None => None,
    };
    let focus = args
        .focus
        .as_ref()
        .map(|query| crate::journey::find_one(&system, query))
        .transpose()?;
    let title = format!("codeviz — {title}");

    let viewer = if before.is_empty() {
        Viewer::new(system)
    } else {
        let mut viewer =
            Viewer::compare(before, documents).with_diff_style(match args.diff_style {
                DiffStyleArg::Status => "status",
                DiffStyleArg::Impact => "impact",
                DiffStyleArg::Morph => "morph",
            });
        if let Some(t) = args.morph {
            viewer = viewer.with_morph(t);
        }
        viewer
    };
    let mut viewer = viewer.with_title(title).with_options(JourneyOptions {
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
                AroundArg::Changes => "changes",
                AroundArg::Journey => "journey",
            }),
            match args.reveal_by {
                NearArg::Calls => "calls",
                NearArg::Space => "space",
                NearArg::Blend => "blend",
            },
            args.reveal,
        );
    if args.show_unchanged {
        viewer = viewer.with_unchanged();
    }
    if let Some(depth) = args.depth {
        viewer = viewer.with_depth(depth);
    }
    if args.bare {
        viewer = viewer.bare();
    }
    if let Some(time) = args.time {
        viewer = viewer.with_time(match time {
            TimeArg::Fly => "fly",
            TimeArg::Held => "held",
        });
    }
    if let Some(path) = &args.screenshot {
        viewer = viewer.with_screenshot(path);
    }
    if let Some(keys) = &args.keys {
        viewer = viewer.with_keys(keys).map_err(anyhow::Error::msg)?;
    }
    if viewer.run().is_error() {
        bail!("viewer exited with an error");
    }
    Ok(())
}
