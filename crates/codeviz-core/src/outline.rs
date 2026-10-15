//! Plain-text rendering of journeys and scenarios, for terminals, tests and agents.

use std::fmt::Write as _;

use codeviz_ir::{Certainty, ExitKind, Mode, UnitId};

use crate::{
    Condition, ExitMarker, Journey, Level, Scenario, Span, SpanId, SpanTarget, StopReason, System,
};

/// A unit's name with its parent's, which reads well in every scheme: `Store.user`.
#[must_use]
pub fn display_name(system: &System, id: &UnitId) -> String {
    let Some(unit) = system.unit(id) else {
        return id.to_string();
    };
    match unit.parent.as_ref().and_then(|p| system.unit(p)) {
        Some(parent) => format!("{}.{}", parent.name, unit.name),
        None => unit.name.clone(),
    }
}

/// `error: unknown sku  (via orders.place → pricing.quote)`.
#[must_use]
pub fn describe_scenario(system: &System, scenario: &Scenario) -> String {
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
        .map(|site| display_name(system, &site.unit))
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

/// One line per span, indented by depth, prefixed with its `enter-exit` ticks. Exits
/// appear under the span they leave, in time order with its calls.
#[must_use]
pub fn outline(system: &System, journey: &Journey) -> String {
    let mut out = String::new();
    if !journey.is_empty() {
        span(system, journey, journey.root(), &mut out);
    }
    out
}

enum Event<'j> {
    Child(SpanId),
    Exit(&'j ExitMarker),
}

fn span(system: &System, journey: &Journey, id: SpanId, out: &mut String) {
    let Some(current) = journey.span(id) else {
        return;
    };
    let indent = "  ".repeat(current.depth as usize);
    let _ = writeln!(
        out,
        "{:>4}-{:<4} {indent}{}{}{}",
        current.enter,
        current.exit,
        span_name(system, current),
        tags(current),
        when(&current.conditions)
    );

    let mut events: Vec<(u32, u8, Event<'_>)> = current
        .children
        .iter()
        .filter_map(|&child| {
            journey
                .span(child)
                .map(|s| (s.enter, 0, Event::Child(child)))
        })
        .chain(
            current
                .exits
                .iter()
                .map(|exit| (exit.tick, 1, Event::Exit(exit))),
        )
        .collect();
    events.sort_by_key(|(tick, order, _)| (*tick, *order));
    for (_, _, event) in events {
        match event {
            Event::Child(child) => span(system, journey, child, out),
            Event::Exit(exit) => {
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
                let _ = writeln!(
                    out,
                    "{:>4}      {indent}  {kind}{label}{}",
                    exit.tick,
                    when(&exit.conditions)
                );
            }
        }
    }
}

fn span_name(system: &System, span: &Span) -> String {
    match &span.target {
        SpanTarget::Unit(id) => display_name(system, id),
        SpanTarget::Ambiguous(ids) => {
            let names: Vec<String> = ids.iter().map(|id| display_name(system, id)).collect();
            format!(
                "{} → one of {}",
                span.text.as_deref().unwrap_or("?"),
                names.join(" | ")
            )
        }
        SpanTarget::Unresolved(text) => format!("{text} [unresolved]"),
    }
}

fn tags(span: &Span) -> String {
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
    if span.refinement {
        tags.push("detail".to_owned());
    }
    match span.zoom {
        Some(Level::Summary) => tags.push("zoomable".to_owned()),
        Some(Level::Detail) => tags.push("zoomed in".to_owned()),
        None => {}
    }
    if span.merged > 1 {
        tags.push(format!("{} merged", span.merged));
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
    if tags.is_empty() {
        String::new()
    } else {
        format!(" [{}]", tags.join(", "))
    }
}

fn when(conditions: &[Condition]) -> String {
    if conditions.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = conditions
        .iter()
        .map(|c| format!("{} ⇒ {}", c.guard.as_deref().unwrap_or("?"), c.arm))
        .collect();
    format!("   when {}", parts.join("; "))
}
