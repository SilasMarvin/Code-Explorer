//! Text rendering of entries, scenarios and journeys.

use std::{fmt::Write as _, io::Write as _, path::PathBuf};

use anyhow::{Context, bail};
use codeviz_core::{Journey, JourneyOptions, Scenario, SpanTarget, StopReason, System, scenarios};
use codeviz_ir::{Certainty, ExitKind, Mode, UnitId};

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
    let entry = match system.find(query).as_slice() {
        [unit] => unit.id.clone(),
        [] => bail!("nothing matches `{query}`"),
        many => bail!(
            "`{query}` is ambiguous:\n{}",
            many.iter()
                .map(|u| format!("  {}", u.id))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    };

    let scenarios = scenarios(&system, &entry);
    let _ = writeln!(out, "scenarios for {entry}:");
    for (index, scenario) in scenarios.iter().enumerate() {
        let _ = writeln!(out, "  [{index}] {}", describe(&system, scenario));
    }
    let scenario = args
        .scenario
        .map(|i| scenarios.get(i).with_context(|| format!("no scenario {i}")))
        .transpose()?;
    let options = JourneyOptions {
        include_unresolved: args.unresolved,
        follow_registrations: args.follow,
        ..JourneyOptions::default()
    };
    let journey =
        Journey::unroll(&system, &entry, scenario, options).context("entry not loaded")?;
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
    render(&system, &journey, &mut out);
    print(&out)
}

fn describe(system: &System, scenario: &Scenario) -> String {
    let kind = match scenario.outcome.kind {
        ExitKind::Return => "ok",
        ExitKind::Error => "error",
        ExitKind::Panic => "panic",
        ExitKind::Unknown => "?",
    };
    let via: Vec<String> = scenario
        .chain
        .iter()
        .skip(1)
        .map(|site| name(system, &site.unit))
        .collect();
    if via.is_empty() {
        format!("{kind}: {}", scenario.outcome.label)
    } else {
        format!(
            "{kind}: {}  (via {})",
            scenario.outcome.label,
            via.join(" → ")
        )
    }
}

fn render(system: &System, journey: &Journey, out: &mut String) {
    for (_, span) in journey.spans() {
        let name = match &span.target {
            SpanTarget::Unit(id) => name(system, id),
            SpanTarget::Ambiguous(ids) => format!(
                "{} ({} candidates)",
                span.text.as_deref().unwrap_or("?"),
                ids.len()
            ),
            SpanTarget::Unresolved(text) => format!("{text} [unresolved]"),
        };
        let mut tags = Vec::new();
        match span.mode {
            Mode::Spawn => tags.push("spawn".to_owned()),
            Mode::Register => tags.push("register".to_owned()),
            _ => {}
        }
        if span.certainty == Some(Certainty::Inferred) {
            tags.push("inferred".to_owned());
        }
        if span.propagates {
            tags.push("?".to_owned());
        }
        if span.repeats {
            tags.push("loop".to_owned());
        }
        match span.stopped {
            Some(StopReason::Recursion) => tags.push("recursion".to_owned()),
            Some(StopReason::DepthLimit) => tags.push("depth limit".to_owned()),
            Some(StopReason::NotLoaded) => tags.push("not loaded".to_owned()),
            Some(StopReason::Registered) | None => {}
        }
        let conditions: Vec<String> = span
            .conditions
            .iter()
            .map(|c| format!("{} ⇒ {}", c.guard.as_deref().unwrap_or("?"), c.arm))
            .collect();
        let indent = "  ".repeat(span.depth as usize);
        let tags = if tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", tags.join(", "))
        };
        let when = if conditions.is_empty() {
            String::new()
        } else {
            format!("   when {}", conditions.join("; "))
        };
        let _ = writeln!(
            out,
            "{:>4}-{:<4} {indent}{name}{tags}{when}",
            span.enter, span.exit
        );
        for exit in &span.exits {
            let kind = match exit.kind {
                ExitKind::Return => "↩ return",
                ExitKind::Error => "✗ error",
                ExitKind::Panic => "‼ panic",
                ExitKind::Unknown => "? exit",
            };
            let label = exit
                .label
                .as_deref()
                .map(|l| format!(": {l}"))
                .unwrap_or_default();
            let when: Vec<String> = exit
                .conditions
                .iter()
                .map(|c| format!("{} ⇒ {}", c.guard.as_deref().unwrap_or("?"), c.arm))
                .collect();
            let when = if when.is_empty() {
                String::new()
            } else {
                format!("   when {}", when.join("; "))
            };
            let _ = writeln!(out, "{:>4}      {indent}  {kind}{label}{when}", exit.tick);
        }
    }
}

/// `scheme:a::b::c` → `b::c`: the unit with its parent, which reads well in every scheme.
fn name(system: &System, id: &UnitId) -> String {
    let Some(unit) = system.unit(id) else {
        return id.to_string();
    };
    match unit.parent.as_ref().and_then(|p| system.unit(p)) {
        Some(parent) => format!("{}.{}", parent.name, unit.name),
        None => unit.name.clone(),
    }
}

fn print(text: &str) -> anyhow::Result<()> {
    std::io::stdout().lock().write_all(text.as_bytes())?;
    Ok(())
}
