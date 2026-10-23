//! Algorithms over codeviz IR documents.
//!
//! A [`System`] merges one or more [`Document`](codeviz_ir::Document)s into a single
//! index. From it, [`Journey::unroll`] expands an entry point into a time-ordered tree
//! of invocations, and [`scenarios`] lists the distinct ways that entry can end so a
//! journey can be restricted to one of them, and [`upstream`] finds everything that
//! can reach a unit. [`Zoom`] and [`Journey::collapse`] change
//! its level of detail. [`Diff`] compares two versions of a system, and [`Review`] orders
//! what it changed for reading.

mod diff;
mod journey;
mod offstage;
mod outline;
mod review;
mod scenario;
mod system;
mod upstream;
mod zoom;

pub use diff::{Call, Change, Diff, Moves, align, union};
pub use journey::{
    Condition, ExitMarker, Journey, JourneyOptions, Span, SpanId, SpanTarget, StopReason,
};
pub use offstage::{FileChange, Offstage, offstage};
pub use outline::{describe_scenario, display_name, outline};
pub use review::{Review, Start};
pub use scenario::{Outcome, Scenario, Seg, Site, StepPath, scenarios};
pub use system::System;
pub use upstream::{Reach, Upstream, UpstreamEdge, upstream};
pub use zoom::{Level, Zoom};
