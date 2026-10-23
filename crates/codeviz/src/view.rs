//! Opens IR documents in the Bevy viewer.

use std::path::PathBuf;

use anyhow::bail;
use codeviz_core::{JourneyOptions, System};
use codeviz_ir::{Document, UnitId};
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
    /// Read the documents' source from this directory instead of the root they were
    /// made in. Without it, a root that no longer exists is looked for around the
    /// document and the working directory.
    #[arg(long, value_name = "DIR")]
    root: Option<PathBuf>,
    #[command(flatten)]
    pub(crate) display: Display,
}

impl ViewArgs {
    /// What names these documents, for keeping their last view, and a name for it.
    pub(crate) fn state_key(&self) -> (String, String) {
        let key = format!(
            "view\n{}\nbefore\n{}",
            crate::state::key(self.documents.iter().map(PathBuf::as_path)),
            crate::state::key(self.before.iter().map(PathBuf::as_path))
        );
        (key, self.title())
    }

    fn title(&self) -> String {
        self.documents
            .iter()
            .filter_map(|p| p.file_stem())
            .map(|s| s.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" + ")
    }
}

/// Where the documents a viewer opens came from.
pub(crate) struct Origin {
    pub(crate) title: String,
    /// The command that opens the same documents, ahead of the view's own flags.
    pub(crate) command: Vec<String>,
    /// What names the documents, for keeping their last view, and a name for it.
    pub(crate) key: (String, String),
    /// Files under the first open in the editor from under the second.
    pub(crate) edit_root: Option<(PathBuf, PathBuf)>,
}

/// What the viewer opens on and how it shows it.
#[derive(Debug, clap::Args)]
pub(crate) struct Display {
    #[command(flatten)]
    pub(crate) start: Start,
    /// Include calls nothing resolved.
    #[arg(long)]
    unresolved: bool,
    /// Start with summaries, or the units they refine.
    #[arg(long, value_enum, default_value_t = crate::journey::ZoomArg::Summary)]
    zoom: crate::journey::ZoomArg,
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
    /// Open with the focus's source showing over the graph (B).
    #[arg(long)]
    peek: bool,
    #[command(flatten)]
    render: Render,
}

/// Where the viewer opens: the function it focuses, the journey from it and how far
/// into it.
#[derive(Debug, clap::Args)]
pub(crate) struct Start {
    /// Entry label (`POST /users`) or unit id suffix to start from.
    #[arg(short, long)]
    entry: Option<String>,
    /// Open with this function (entry label or unit id suffix) focused in the graph.
    #[arg(long, value_name = "UNIT")]
    focus: Option<String>,
    /// Open with the function at this place in the source focused: `src/orders.rs:42`.
    #[arg(long, value_name = "FILE:LINE", overrides_with = "focus")]
    at: Option<String>,
    /// Open on this outcome (as numbered by `codeviz journey`) instead of the happy path.
    #[arg(long)]
    scenario: Option<usize>,
    /// Open with the cursor this many steps into the journey.
    #[arg(long, default_value_t = 0)]
    step: usize,
    /// Open the journey this many levels deep instead of one.
    #[arg(long, value_name = "LEVELS")]
    depth: Option<u32>,
    /// Open with what calls the focus, or what it calls, picked out (I and O).
    #[arg(long, value_enum)]
    spotlight: Option<SpotlightArg>,
    /// Also open journeys from these entries.
    #[arg(long, value_name = "ENTRY")]
    also: Vec<String>,
    /// Open as this was last left, with any flags given here on top.
    #[arg(long)]
    pub(crate) resume: bool,
}

/// Rendering once, for a picture.
#[derive(Debug, clap::Args)]
struct Render {
    /// Draw only the graph and the journey, without the panels and cards around them.
    #[arg(long)]
    bare: bool,
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
enum SpotlightArg {
    /// What calls the focus.
    Callers,
    /// What the focus calls.
    Callees,
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
enum ClusterArg {
    /// Functions in the same container pull together.
    Modules,
    /// Only calls pull.
    Calls,
}

/// The function whose source spans `place`, written `FILE:LINE`; a closure stands for
/// the function around it.
fn find_at(system: &System, place: &str) -> anyhow::Result<UnitId> {
    let Some((file, line)) = place
        .rsplit_once(':')
        .and_then(|(file, line)| Some((file, line.parse::<u32>().ok()?)))
    else {
        bail!("`{place}` is not FILE:LINE");
    };
    let found = system.containing(file, line);
    let mut files: Vec<String> = found
        .iter()
        .filter_map(|u| {
            let uri = &u.location.as_ref()?.uri;
            Some(
                system
                    .local_path(&u.id, uri)
                    .map_or_else(|| uri.clone(), |path| path.display().to_string()),
            )
        })
        .collect();
    files.sort_unstable();
    files.dedup();
    match (found.first(), files.as_slice()) {
        (None, _) => bail!("no function spans {place}"),
        (Some(unit), [_]) => Ok(system.owner(&unit.id).clone()),
        (Some(_), _) => bail!(
            "`{file}` is ambiguous:\n{}",
            files
                .iter()
                .map(|f| format!("  {f}"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    }
}

pub(crate) fn run(args: &ViewArgs) -> anyhow::Result<()> {
    let documents = args
        .documents
        .iter()
        .map(|path| crate::ir::read_rooted(path, args.root.as_deref()))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let before = args
        .before
        .iter()
        .map(|path| crate::ir::read_rooted(path, None))
        .collect::<anyhow::Result<Vec<_>>>()?;
    let mut command = vec!["codeviz".to_owned(), "view".to_owned()];
    let shown = |paths: &[PathBuf]| {
        paths
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
    };
    command.extend(shown(&args.documents));
    if !args.before.is_empty() {
        command.push("--before".to_owned());
        command.extend(shown(&args.before));
    }
    if let Some(root) = &args.root {
        command.extend(["--root".to_owned(), root.display().to_string()]);
    }
    let origin = Origin {
        title: args.title(),
        command,
        key: args.state_key(),
        edit_root: None,
    };
    open(documents, before, &args.display, origin)
}

/// Opens `documents`, diffed against `before` unless it is empty, and blocks until the
/// window closes.
pub(crate) fn open(
    documents: Vec<Document>,
    before: Vec<Document>,
    args: &Display,
    origin: Origin,
) -> anyhow::Result<()> {
    let system = System::new(documents.clone());
    let start = start(&system, &args.start)?;
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
    let viewer = viewer
        .with_title(format!("codeviz — {}", origin.title))
        .with_options(JourneyOptions {
            include_unresolved: args.unresolved,
            zoom: args.zoom.into(),
            ..JourneyOptions::default()
        });
    let mut viewer = start(viewer)
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
    if args.peek {
        viewer = viewer.with_peek();
    }
    if let Some((from, to)) = origin.edit_root {
        viewer = viewer.with_edit_root(from, to);
    }
    let (key, name) = &origin.key;
    viewer = viewer.with_share(origin.command, crate::state::file(key, name));
    if render(viewer, &args.render)?.run().is_error() {
        bail!("viewer exited with an error");
    }
    Ok(())
}

/// What opens a viewer where `args` says, its functions found in `system`.
fn start(system: &System, args: &Start) -> anyhow::Result<impl FnOnce(Viewer) -> Viewer + use<>> {
    let find = |query: &String| crate::journey::find_one(system, query);
    let root = args.entry.as_ref().map(find).transpose()?;
    let also = args
        .also
        .iter()
        .map(find)
        .collect::<anyhow::Result<Vec<_>>>()?;
    let focus = match (&args.focus, &args.at) {
        (Some(query), _) => Some(find(query)?),
        (None, Some(place)) => Some(find_at(system, place)?),
        (None, None) => None,
    };
    let (scenario, depth, spotlight, step) = (args.scenario, args.depth, args.spotlight, args.step);
    Ok(move |viewer: Viewer| {
        let mut viewer = viewer.with_step(step);
        if let Some(root) = root {
            viewer = viewer.with_root(root);
        }
        for root in also {
            viewer = viewer.also(root);
        }
        if let Some(focus) = focus {
            viewer = viewer.with_focus(focus);
        }
        if let Some(scenario) = scenario {
            viewer = viewer.with_scenario(scenario);
        }
        if let Some(depth) = depth {
            viewer = viewer.with_depth(depth);
        }
        if let Some(side) = spotlight {
            viewer = viewer.with_spotlight(match side {
                SpotlightArg::Callers => "callers",
                SpotlightArg::Callees => "callees",
            });
        }
        viewer
    })
}

/// `viewer` rendering once for a picture, when `args` asks.
fn render(mut viewer: Viewer, args: &Render) -> anyhow::Result<Viewer> {
    if args.bare {
        viewer = viewer.bare();
    }
    if let Some(path) = &args.screenshot {
        viewer = viewer.with_screenshot(path);
    }
    if let Some(keys) = &args.keys {
        viewer = viewer.with_keys(keys).map_err(anyhow::Error::msg)?;
    }
    Ok(viewer)
}
