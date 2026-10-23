//! Text rendering of entries, scenarios and journeys.

use std::{fmt::Write as _, io::Write as _, path::PathBuf};

use anyhow::{Context, bail};
use codeviz_core::{
    Journey, JourneyOptions, Level, Scenario, Story, System, Zoom, describe_scenario, number_calls,
    outline, scenarios,
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
    /// Levels of calls below the entry that `md` and `mermaid` show; all by default.
    #[arg(long)]
    depth: Option<u32>,
}

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
    let numbers = number_calls(system, &journey, args.depth.unwrap_or(u32::MAX));
    let story = Story::new(system, &journey, scenarios, Some(index), &numbers);
    Ok(match args.format {
        Format::Mermaid => story.mermaid(),
        Format::Md | Format::Text => story.markdown(),
    })
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
