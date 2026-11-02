//! The level of detail of a journey: a unit with `refines` links shows either its own
//! body (the summary) or the units it refines (the detail).

use std::collections::HashMap;

use codeviz_ir::UnitId;

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
}
