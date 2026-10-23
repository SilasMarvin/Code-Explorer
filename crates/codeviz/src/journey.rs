//! Text rendering of entries, scenarios and journeys.

use std::{fmt::Write as _, io::Write as _, path::PathBuf};

use anyhow::{Context, bail};
use codeviz_core::{
    Diff, Journey, JourneyOptions, Level, Scenario, Story, System, Zoom, align, depth_to_fit,
    describe_scenario, number_calls, outline, scenarios,
};
use codeviz_ir::UnitId;

#[derive(Debug, clap::Args)]
pub(crate) struct JourneyArgs {
    /// IR documents to load; later documents override earlier ones.
    #[arg(required = true)]
    documents: Vec<PathBuf>,
    /// Entry label (`POST /users`) or unit id suffix (`orders::place`). Lists entries if omitted.
    #[arg(short, long)]
    entry: Option<String>,
    /// Unroll only this scenario (index from the scenario list).
    #[arg(short, long)]
    scenario: Option<usize>,
    /// Include calls nothing resolved.
    #[arg(long)]
    unresolved: bool,
    /// Expand registered handoffs (routes, callbacks, subscriptions) inline.
    #[arg(long)]
    follow: bool,
    /// Show summaries, or the units they refine.
    #[arg(long, value_enum, default_value_t = ZoomArg::Summary)]
    zoom: ZoomArg,
    /// Merge every span inside this container (repeatable; a unit id or id suffix).
    #[arg(long, value_name = "UNIT")]
    collapse: Vec<String>,
    /// How to print the journey: `text`, every span on the clock; `md`, a numbered
    /// outline to paste into a pull request, doc or chat; `mermaid`, a sequence diagram.
    #[arg(long, value_enum, default_value_t = Format::Text)]
    format: Format,
    /// Levels of calls below the entry that `md` and `mermaid` show; by default all of
    /// them, or as many as fit in 40 calls.
    #[arg(long)]
    depth: Option<u32>,
    /// An older version of the documents: `md` and `mermaid` mark the calls that
    /// changed since, and `md` begins with what changed.
    #[arg(long, value_name = "DOCUMENT")]
    before: Vec<PathBuf>,
    /// Link each function `md` names to its source: under BASE
    /// (`--links=https://github.com/org/repo/blob/main`), or by default on GitHub at
    /// the commit the analyzed code's repository is at.
    #[arg(long, value_name = "BASE", num_args = 0..=1, require_equals = true, default_missing_value = "")]
    links: Option<String>,
}

/// Calls a told journey shows when no depth is asked for: about what a reader takes in.
const TOLD_CALLS: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Format {
    Text,
    Md,
    Mermaid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum ZoomArg {
    Summary,
    Detail,
}

impl From<ZoomArg> for Zoom {
    fn from(value: ZoomArg) -> Self {
        Self {
            default: match value {
                ZoomArg::Summary => Level::Summary,
                ZoomArg::Detail => Level::Detail,
            },
            ..Self::default()
        }
    }
}

pub(crate) fn run(args: &JourneyArgs) -> anyhow::Result<()> {
    let documents = args
        .documents
        .iter()
        .map(crate::ir::read)
        .collect::<anyhow::Result<Vec<_>>>()?;
    let system = System::new(documents);
    let mut out = String::new();

    let Some(query) = &args.entry else {
        for entry in system.entries() {
            let _ = writeln!(out, "{:<10} {:<32} {}", entry.kind, entry.label, entry.unit);
        }
        return print(&out);
    };
    let entry = find_one(&system, query)?;
    let zoom = Zoom::from(args.zoom);
    let scenarios = scenarios(&system, &entry, &zoom);
    if args.format != Format::Text {
        return print(&tell(&system, &entry, &scenarios, args, zoom)?);
    }
    let _ = writeln!(out, "scenarios for {entry}:");
    for (index, scenario) in scenarios.iter().enumerate() {
        let _ = writeln!(out, "  [{index}] {}", describe_scenario(&system, scenario));
    }
    let scenario = args
        .scenario
        .map(|i| scenarios.get(i).with_context(|| format!("no scenario {i}")))
        .transpose()?;
    let journey = unroll(&system, &entry, scenario, args, zoom)?;
    let _ = writeln!(
        out,
        "\njourney{}: {} spans, {} ticks, depth {}",
        scenario
            .map(|s| format!(" ({})", s.outcome.label))
            .unwrap_or_default(),
        journey.len(),
        journey.duration(),
        journey.max_depth()
    );
    out.push_str(&outline(&system, &journey));
    print(&out)
}

/// The journey as Markdown or Mermaid. Without a scenario it tells the one that
/// completes, as the viewer opens on, with where the others split off.
fn tell(
    system: &System,
    entry: &UnitId,
    scenarios: &[Scenario],
    args: &JourneyArgs,
    zoom: Zoom,
) -> anyhow::Result<String> {
    let index = args.scenario.unwrap_or(0);
    let scenario = scenarios
        .get(index)
        .with_context(|| format!("no scenario {index}"))?;
    let journey = unroll(system, entry, Some(scenario), args, zoom)?;
    let depth = args
        .depth
        .unwrap_or_else(|| depth_to_fit(system, &journey, TOLD_CALLS));
    let numbers = number_calls(system, &journey, depth);
    let mut story = Story::new(system, &journey, scenarios, Some(index), &numbers);
    if let Some(base) = &args.links {
        let units: Vec<&UnitId> = story.all_beats().filter_map(|b| b.unit.as_ref()).collect();
        let links = crate::links::source_links(system, units, base)?;
        story = story.with_links(&links);
    }
    let Some((diff, old)) = changes(system, &args.before)? else {
        return Ok(match args.format {
            Format::Mermaid => story.mermaid(),
            Format::Md | Format::Text => story.markdown(),
        });
    };
    let story = story.with_diff(&diff, &old);
    Ok(match args.format {
        Format::Mermaid => story.mermaid(),
        Format::Md | Format::Text => {
            format!("{}\n{}", diff.markdown(&old, system), story.markdown())
        }
    })
}

/// What changed since the documents in `before`, with the old version aligned to the
/// new; `None` when there are none.
fn changes(system: &System, before: &[PathBuf]) -> anyhow::Result<Option<(Diff, System)>> {
    if before.is_empty() {
        return Ok(None);
    }
    let documents = before
        .iter()
        .map(crate::ir::read)
        .collect::<anyhow::Result<Vec<_>>>()?;
    let (aligned, moved) = align(documents, system);
    let old = System::new(aligned);
    let diff = Diff::between(&old, system).with_moves(moved);
    Ok(Some((diff, old)))
}

/// The journey from `entry` the arguments ask for.
fn unroll(
    system: &System,
    entry: &UnitId,
    scenario: Option<&Scenario>,
    args: &JourneyArgs,
    zoom: Zoom,
) -> anyhow::Result<Journey> {
    let options = JourneyOptions {
        include_unresolved: args.unresolved,
        follow_registrations: args.follow,
        zoom,
        ..JourneyOptions::default()
    };
    let journey = Journey::unroll(system, entry, scenario, options).context("entry not loaded")?;
    if args.collapse.is_empty() {
        return Ok(journey);
    }
    let containers = args
        .collapse
        .iter()
        .map(|query| find_one(system, query))
        .collect::<anyhow::Result<_>>()?;
    Ok(journey.collapse(system, &containers))
}

pub(crate) fn find_one(system: &System, query: &str) -> anyhow::Result<UnitId> {
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

fn print(text: &str) -> anyhow::Result<()> {
    std::io::stdout().lock().write_all(text.as_bytes())?;
    Ok(())
}
