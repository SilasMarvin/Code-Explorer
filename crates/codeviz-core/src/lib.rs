//! Algorithms over codeviz IR documents.
//!
//! A [`System`] merges one or more [`Document`](codeviz_ir::Document)s into a single
//! index. From it, [`Journey::unroll`] expands an entry point into a time-ordered tree
//! of invocations, and [`scenarios`] lists the distinct ways that entry can end so a
//! journey can be restricted to one of them. [`Zoom`] and [`Journey::collapse`] change
//! its level of detail.

mod journey;
mod scenario;
mod system;
mod zoom;

pub use journey::{
    Condition, ExitMarker, Journey, JourneyOptions, Span, SpanId, SpanTarget, StopReason,
};
pub use scenario::{Outcome, Scenario, Seg, Site, StepPath, scenarios};
pub use system::System;
pub use zoom::{Level, Zoom};
