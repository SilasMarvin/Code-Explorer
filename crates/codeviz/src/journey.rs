//! Text rendering of entries, scenarios and journeys.

use std::{fmt::Write as _, io::Write as _, path::PathBuf};

use anyhow::{Context, bail};
use codeviz_core::{
    Journey, JourneyOptions, Level, System, Zoom, describe_scenario, outline, scenarios,
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
    let _ = writeln!(out, "scenarios for {entry}:");
    for (index, scenario) in scenarios.iter().enumerate() {
        let _ = writeln!(out, "  [{index}] {}", describe_scenario(&system, scenario));
    }
    let scenario = args
        .scenario
        .map(|i| scenarios.get(i).with_context(|| format!("no scenario {i}")))
        .transpose()?;
    let options = JourneyOptions {
        include_unresolved: args.unresolved,
        follow_registrations: args.follow,
        zoom,
        ..JourneyOptions::default()
    };
    let mut journey =
        Journey::unroll(&system, &entry, scenario, options).context("entry not loaded")?;
    if !args.collapse.is_empty() {
        let containers = args
            .collapse
            .iter()
            .map(|query| find_one(&system, query))
            .collect::<anyhow::Result<_>>()?;
        journey = journey.collapse(&system, &containers);
    }
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
