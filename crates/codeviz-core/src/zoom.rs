//! Two ways to change the level of detail of a journey.
//!
//! - **Refinement** ([`Zoom`]): a unit with `refines` links shows either its own body
//!   (the summary) or the units it refines (the detail).
//! - **Collapse** ([`Journey::collapse`]): every span inside a chosen container merges
//!   into one span for that container. It needs no authored summary.

use std::collections::{HashMap, HashSet};

use codeviz_ir::UnitId;

use crate::{Journey, Span, SpanId, SpanTarget, System};

/// How much detail a unit with `refines` links shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Level {
    /// The unit's own body.
    #[default]
    Summary,
    /// The units it refines, in its place.
    Detail,
}

/// The level of every refinable unit: a default plus per-unit overrides.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Zoom {
    pub default: Level,
    pub overrides: HashMap<UnitId, Level>,
}

impl Zoom {
    #[must_use]
    pub fn level(&self, unit: &UnitId) -> Level {
        self.overrides.get(unit).copied().unwrap_or(self.default)
    }

    pub fn set(&mut self, unit: UnitId, level: Level) {
        self.overrides.insert(unit, level);
    }
}

impl Journey {
    /// Merges every span inside one of `containers` into a single span for the
    /// outermost such container. Calls that leave the container stay visible.
    #[must_use]
    pub fn collapse(&self, system: &System, containers: &HashSet<UnitId>) -> Self {
        let container_of = |span: &Span| -> Option<UnitId> {
            let SpanTarget::Unit(unit) = &span.target else {
                return None;
            };
            let mut outermost = containers.contains(unit).then(|| unit.clone());
            for ancestor in system.ancestors(unit) {
                if containers.contains(&ancestor.id) {
                    outermost = Some(ancestor.id.clone());
                }
            }
            outermost
        };

        let mut spans: Vec<Span> = Vec::with_capacity(self.len());
        // Old span index → (new span index, container it was merged into).
        let mut mapped: Vec<(usize, Option<UnitId>)> = Vec::with_capacity(self.len());
        for (_, span) in self.spans() {
            let parent = span.parent.and_then(|p| mapped.get(p.index()).cloned());
            // Unresolved and ambiguous calls belong to their caller's container.
            let container = container_of(span).or_else(|| match (&span.target, &parent) {
                (SpanTarget::Unresolved(_) | SpanTarget::Ambiguous(_), Some((_, container))) => {
                    container.clone()
                }
                _ => None,
            });

            if let (Some(container), Some((into, parent_container))) = (&container, &parent)
                && Some(container) == parent_container.as_ref()
            {
                if let Some(target) = spans.get_mut(*into) {
                    target.exit = target.exit.max(span.exit);
                    target.exits.extend(span.exits.iter().cloned());
                    target.merged += 1;
                }
                mapped.push((*into, Some(container.clone())));
                continue;
            }

            let index = spans.len();
            let mut copy = span.clone();
            copy.children.clear();
            copy.parent = parent.as_ref().map(|(p, _)| SpanId(*p));
            copy.depth = parent
                .as_ref()
                .and_then(|(p, _)| spans.get(*p))
                .map_or(0, |p| p.depth + 1);
            if let Some(container) = &container {
                copy.target = SpanTarget::Unit(container.clone());
                copy.merged = 1;
            }
            if let Some(parent) = parent.as_ref().and_then(|(p, _)| spans.get_mut(*p)) {
                parent.children.push(SpanId(index));
            }
            spans.push(copy);
            mapped.push((index, container));
        }
        Self::from_spans(spans)
    }
}
