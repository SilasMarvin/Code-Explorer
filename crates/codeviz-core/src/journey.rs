//! Unrolls an entry point into a time-ordered tree of invocations.
//!
//! Time is a logical clock that ticks once per invocation. Alternatives (branch arms)
//! and concurrent work (`dag` nodes, `spawn`) start at the same tick and overlap.

use codeviz_ir::{
    Arm, CallKey, Certainty, DagNode, Dispatch, ExitKind, Invocation, Mode, Step, Unit, UnitId,
};

use crate::{
    Level, Scenario, System, Zoom,
    scenario::{Seg, Site, StepPath, body, refined},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpanId(pub(crate) usize);

impl SpanId {
    #[must_use]
    pub fn index(self) -> usize {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpanTarget {
    Unit(UnitId),
    /// `dispatch: one` with several candidates; not expanded.
    Ambiguous(Vec<UnitId>),
    /// No target resolved; the written callee.
    Unresolved(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    /// The unit is already on the stack.
    Recursion,
    DepthLimit,
    /// Handed off for later; not run now.
    Registered,
    /// The target is not in any loaded document.
    NotLoaded,
}

/// A branch arm that must be taken for an invocation to happen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Condition {
    pub guard: Option<String>,
    pub arm: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExitMarker {
    pub tick: u32,
    pub kind: ExitKind,
    pub label: Option<String>,
    pub conditions: Vec<Condition>,
}

/// One invocation in a journey.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    pub target: SpanTarget,
    pub parent: Option<SpanId>,
    pub children: Vec<SpanId>,
    /// The caller's call site; `None` for the root.
    pub call: Option<(UnitId, CallKey)>,
    pub text: Option<String>,
    pub mode: Mode,
    pub certainty: Option<Certainty>,
    pub propagates: bool,
    /// Arms between the caller's entry and this call.
    pub conditions: Vec<Condition>,
    /// The call sits inside a loop.
    pub repeats: bool,
    pub depth: u32,
    pub enter: u32,
    pub exit: u32,
    /// Exits inside this unit's own body.
    pub exits: Vec<ExitMarker>,
    pub stopped: Option<StopReason>,
    /// This span is the detail of its parent (a `refines` link), not a call.
    pub refinement: bool,
    /// The level shown, when the unit has loaded refinements to zoom between.
    pub zoom: Option<Level>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JourneyOptions {
    pub max_depth: u32,
    /// Show calls nothing resolved (std, dependencies).
    pub include_unresolved: bool,
    /// Expand `register` handoffs as if invoked immediately.
    pub follow_registrations: bool,
    /// Summary or detail for units with `refines` links.
    pub zoom: Zoom,
}

impl Default for JourneyOptions {
    fn default() -> Self {
        Self {
            max_depth: 32,
            include_unresolved: false,
            follow_registrations: false,
            zoom: Zoom::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Journey {
    spans: Vec<Span>,
}

#[derive(Debug, Clone, Copy)]
enum Filter<'c> {
    /// Every arm, as overlapping alternatives.
    All,
    /// Only arms that do not end in an error.
    Completes,
    /// Follow this chain of sites to the exit, then stop.
    Escape(&'c [Site]),
}

enum Flow {
    Continue,
    Stop,
}

impl Journey {
    /// Unrolls `entry`, optionally restricted to one of its [`Scenario`]s.
    ///
    /// Returns `None` if `entry` is not in `system`.
    #[must_use]
    pub fn unroll(
        system: &System,
        entry: &UnitId,
        scenario: Option<&Scenario>,
        options: JourneyOptions,
    ) -> Option<Self> {
        system.unit(entry)?;
        let filter = match scenario {
            None => Filter::All,
            Some(s) if s.completes() => Filter::Completes,
            Some(s) => Filter::Escape(&s.chain),
        };
        let mut builder = Builder {
            system,
            options,
            spans: Vec::new(),
            stack: Vec::new(),
            clock: 0,
        };
        builder.invoke(
            SpanTarget::Unit(entry.clone()),
            None,
            None,
            &Context::default(),
            filter,
        );
        Some(Self {
            spans: builder.spans,
        })
    }

    #[must_use]
    pub fn root(&self) -> SpanId {
        SpanId(0)
    }

    #[must_use]
    pub fn span(&self, id: SpanId) -> Option<&Span> {
        self.spans.get(id.0)
    }

    /// Spans in pre-order.
    pub fn spans(&self) -> impl ExactSizeIterator<Item = (SpanId, &Span)> {
        self.spans.iter().enumerate().map(|(i, s)| (SpanId(i), s))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.spans.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }
}

#[derive(Debug, Clone, Default)]
struct Context {
    conditions: Vec<Condition>,
    repeats: bool,
}

/// Where a call came from, for recording on the callee's span.
struct Origin<'i> {
    caller: &'i UnitId,
    invocation: &'i Invocation,
    certainty: Option<Certainty>,
}

struct Builder<'s> {
    system: &'s System,
    options: JourneyOptions,
    spans: Vec<Span>,
    stack: Vec<UnitId>,
    clock: u32,
}

impl Builder<'_> {
    fn invoke(
        &mut self,
        target: SpanTarget,
        parent: Option<SpanId>,
        origin: Option<&Origin<'_>>,
        cx: &Context,
        filter: Filter<'_>,
    ) -> SpanId {
        let id = SpanId(self.spans.len());
        let depth = u32::try_from(self.stack.len()).unwrap_or(u32::MAX);
        let mode = origin.map_or(Mode::Call, |o| o.invocation.mode);
        let unit = match &target {
            SpanTarget::Unit(unit) => self.system.unit(unit),
            _ => None,
        };
        let stopped = match (&target, unit) {
            (SpanTarget::Unit(_), None) => Some(StopReason::NotLoaded),
            (SpanTarget::Unit(unit), _) if self.stack.contains(unit) => Some(StopReason::Recursion),
            _ if mode == Mode::Register && !self.options.follow_registrations => {
                Some(StopReason::Registered)
            }
            (SpanTarget::Unit(_), _) if depth >= self.options.max_depth => {
                Some(StopReason::DepthLimit)
            }
            _ => None,
        };

        let enter = self.clock;
        self.clock += 1;
        self.spans.push(Span {
            target,
            parent,
            children: Vec::new(),
            call: origin.map(|o| (o.caller.clone(), o.invocation.key.clone())),
            text: origin.map(|o| o.invocation.text.clone()),
            mode,
            certainty: origin.and_then(|o| o.certainty),
            propagates: origin.is_some_and(|o| o.invocation.propagates),
            conditions: cx.conditions.clone(),
            repeats: cx.repeats,
            depth,
            enter,
            exit: enter,
            exits: Vec::new(),
            stopped,
            refinement: false,
            zoom: None,
        });
        if let Some(parent) = parent.and_then(|p| self.spans.get_mut(p.0)) {
            parent.children.push(id);
        }

        if let (Some(unit), None) = (unit, stopped) {
            self.stack.push(unit.id.clone());
            let filter = match filter {
                Filter::Escape([site, ..]) if site.unit == unit.id => filter,
                Filter::Escape(_) => Filter::Completes,
                other => other,
            };
            let detail = refined(self.system, &self.options.zoom, &unit.id);
            let refinable = self
                .system
                .refinements(&unit.id)
                .iter()
                .any(|(target, _)| self.system.unit(target).is_some());
            if let Some(span) = self.spans.get_mut(id.0).filter(|_| refinable) {
                span.zoom = Some(if detail.is_empty() {
                    Level::Summary
                } else {
                    Level::Detail
                });
            }
            if detail.is_empty() {
                let steps = body(unit);
                let cx = Context::default();
                self.block(unit, id, &steps, &mut Vec::new(), &cx, filter);
            } else {
                self.refine(unit, id, &detail, filter);
            }
            self.stack.pop();
        }
        if let Some(span) = self.spans.get_mut(id.0) {
            span.exit = self.clock.max(enter + 1);
        }
        id
    }

    /// Shows the units `unit` refines in its place, in order.
    fn refine(&mut self, unit: &Unit, span: SpanId, detail: &[UnitId], filter: Filter<'_>) {
        // On a scenario, the chain names which refinement the exit comes from.
        let through = match filter {
            Filter::Escape([site, ..]) => match site.path.as_slice() {
                [Seg::Refinement(index)] => Some(*index),
                _ => None,
            },
            _ => None,
        };
        for (index, target) in detail.iter().enumerate() {
            let child_filter = match (through, filter) {
                (Some(through), _) if index > through => break,
                (Some(through), Filter::Escape([_, rest @ ..])) if index == through => {
                    Filter::Escape(rest)
                }
                (Some(_), _) => Filter::Completes,
                (None, other) => other,
            };
            let target_span = SpanTarget::Unit(target.clone());
            let child = self.invoke(
                target_span,
                Some(span),
                None,
                &Context::default(),
                child_filter,
            );
            let certainty = self
                .system
                .refinements(&unit.id)
                .iter()
                .find(|(t, _)| t == target)
                .map(|(_, c)| *c);
            if let Some(child) = self.spans.get_mut(child.0) {
                child.refinement = true;
                child.certainty = certainty;
            }
        }
    }

    fn block(
        &mut self,
        unit: &Unit,
        span: SpanId,
        steps: &[Step],
        path: &mut StepPath,
        cx: &Context,
        filter: Filter<'_>,
    ) -> Flow {
        for (index, step) in steps.iter().enumerate() {
            path.push(Seg::Step(index));
            let flow = match filter {
                Filter::Escape([site, rest @ ..]) if site.path == *path => {
                    self.step(unit, span, step, path, cx, filter, Some(rest));
                    Flow::Stop
                }
                Filter::Escape([site, ..]) if !site.path.starts_with(path) => {
                    self.step(unit, span, step, path, cx, Filter::Completes, None)
                }
                _ => self.step(unit, span, step, path, cx, filter, None),
            };
            path.pop();
            if matches!(flow, Flow::Stop) {
                return Flow::Stop;
            }
        }
        Flow::Continue
    }

    /// Runs one step. `site` is `Some(rest)` when this step is on a scenario's escape
    /// chain; `rest` continues into the callee.
    #[expect(clippy::too_many_arguments, reason = "recursive walker state")]
    fn step(
        &mut self,
        unit: &Unit,
        span: SpanId,
        step: &Step,
        path: &mut StepPath,
        cx: &Context,
        filter: Filter<'_>,
        site: Option<&[Site]>,
    ) -> Flow {
        match step {
            Step::Call { call } => {
                if let Some(invocation) = unit.call(call) {
                    self.call(unit, span, invocation, cx, filter, site);
                }
                Flow::Continue
            }
            Step::Exit { kind, label } => {
                if let Some(current) = self.spans.get_mut(span.0) {
                    current.exits.push(ExitMarker {
                        tick: self.clock,
                        kind: *kind,
                        label: label.clone(),
                        conditions: cx.conditions.clone(),
                    });
                }
                // An exit is a moment on the clock, so it never shares a tick with
                // what comes after it.
                self.clock += 1;
                Flow::Stop
            }
            Step::Branch { guard, arms } => {
                self.branch(unit, span, guard.as_ref(), arms, path, cx, filter)
            }
            Step::Loop { body, .. } => {
                path.push(Seg::LoopBody);
                let inner = Context {
                    repeats: true,
                    ..cx.clone()
                };
                let flow = self.block(unit, span, body, path, &inner, filter);
                path.pop();
                flow
            }
            Step::Dag { nodes } => self.dag(unit, span, nodes, path, cx, filter),
            Step::Unknown => Flow::Continue,
        }
    }

    /// Arms are alternatives: each starts at the branch's tick and they overlap.
    #[expect(clippy::too_many_arguments, reason = "recursive walker state")]
    fn branch(
        &mut self,
        unit: &Unit,
        span: SpanId,
        guard: Option<&String>,
        arms: &[Arm],
        path: &mut StepPath,
        cx: &Context,
        filter: Filter<'_>,
    ) -> Flow {
        let start = self.clock;
        let mut end = start;
        let mut continues = arms.is_empty();
        for (index, arm) in arms.iter().enumerate() {
            path.push(Seg::Arm(index));
            let included = match filter {
                Filter::Escape([site, ..]) => site.path.starts_with(path),
                Filter::Completes => !ends_in_failure(&arm.body),
                Filter::All | Filter::Escape([]) => true,
            };
            if included {
                self.clock = start;
                let mut inner = cx.clone();
                inner.conditions.push(Condition {
                    guard: guard.cloned(),
                    arm: arm.label.clone(),
                });
                let flow = self.block(unit, span, &arm.body, path, &inner, filter);
                continues |= matches!(flow, Flow::Continue);
                end = end.max(self.clock);
            }
            path.pop();
        }
        self.clock = end;
        if continues {
            Flow::Continue
        } else {
            Flow::Stop
        }
    }

    /// Nodes start when everything in their `after` has finished.
    fn dag(
        &mut self,
        unit: &Unit,
        span: SpanId,
        nodes: &[DagNode],
        path: &mut StepPath,
        cx: &Context,
        filter: Filter<'_>,
    ) -> Flow {
        let start = self.clock;
        let mut finished: Vec<Option<u32>> = vec![None; nodes.len()];
        let mut flow = Flow::Continue;
        loop {
            let ready = nodes.iter().enumerate().find_map(|(index, node)| {
                if finished.get(index).copied().flatten().is_some() {
                    return None;
                }
                let deps: Option<Vec<u32>> = node
                    .after
                    .iter()
                    .map(|dep| {
                        let position = nodes.iter().position(|n| &n.id == dep)?;
                        finished.get(position).copied().flatten()
                    })
                    .collect();
                deps.map(|deps| (index, node, deps.into_iter().max().unwrap_or(start)))
            });
            let Some((index, node, begin)) = ready else {
                break;
            };
            path.push(Seg::Node(index));
            let included = match filter {
                Filter::Escape([site, ..]) => site.path.starts_with(path),
                _ => true,
            };
            self.clock = begin;
            if included
                && matches!(
                    self.block(unit, span, &node.body, path, cx, filter),
                    Flow::Stop
                )
                && matches!(filter, Filter::Escape(_))
            {
                flow = Flow::Stop;
            }
            path.pop();
            if let Some(slot) = finished.get_mut(index) {
                *slot = Some(self.clock);
            }
        }
        self.clock = finished.into_iter().flatten().max().unwrap_or(start);
        flow
    }

    fn call(
        &mut self,
        unit: &Unit,
        span: SpanId,
        invocation: &Invocation,
        cx: &Context,
        filter: Filter<'_>,
        site: Option<&[Site]>,
    ) {
        // Callees inherit the full view; on a scenario they either continue its chain
        // or complete normally.
        let child_filter = |target: &UnitId| match (site, filter) {
            (Some(rest @ [next, ..]), _) if &next.unit == target => Filter::Escape(rest),
            (_, Filter::All) => Filter::All,
            _ => Filter::Completes,
        };
        let spawn_start = self.clock;

        match invocation.targets.as_slice() {
            [] => {
                // An unresolved call is shown when asked for, or when it is where a
                // scenario's error originates.
                if self.options.include_unresolved || site == Some(&[]) {
                    let origin = origin(unit, invocation, None);
                    let target = SpanTarget::Unresolved(invocation.text.clone());
                    self.invoke(target, Some(span), Some(&origin), cx, Filter::Completes);
                }
            }
            [target] => {
                let origin = origin(unit, invocation, Some(target.certainty));
                let filter = child_filter(&target.unit);
                self.invoke(
                    SpanTarget::Unit(target.unit.clone()),
                    Some(span),
                    Some(&origin),
                    cx,
                    filter,
                );
            }
            many if invocation.dispatch == Dispatch::All => {
                for target in many {
                    if invocation.mode == Mode::Spawn {
                        self.clock = spawn_start;
                    }
                    let origin = origin(unit, invocation, Some(target.certainty));
                    let filter = child_filter(&target.unit);
                    self.invoke(
                        SpanTarget::Unit(target.unit.clone()),
                        Some(span),
                        Some(&origin),
                        cx,
                        filter,
                    );
                }
            }
            many => {
                // A scenario through one candidate follows it; otherwise stay ambiguous.
                let followed = site
                    .and_then(<[Site]>::first)
                    .and_then(|next| many.iter().find(|t| t.unit == next.unit));
                if let Some(target) = followed {
                    let origin = origin(unit, invocation, Some(target.certainty));
                    let filter = child_filter(&target.unit);
                    self.invoke(
                        SpanTarget::Unit(target.unit.clone()),
                        Some(span),
                        Some(&origin),
                        cx,
                        filter,
                    );
                } else {
                    // Dispatch is only as certain as its least certain candidate.
                    let certainty = many
                        .iter()
                        .map(|t| t.certainty)
                        .find(|c| *c == Certainty::Inferred)
                        .unwrap_or(Certainty::Exact);
                    let origin = origin(unit, invocation, Some(certainty));
                    let ids = many.iter().map(|t| t.unit.clone()).collect();
                    self.invoke(
                        SpanTarget::Ambiguous(ids),
                        Some(span),
                        Some(&origin),
                        cx,
                        Filter::Completes,
                    );
                }
            }
        }
        if invocation.mode == Mode::Spawn {
            // The caller continues without waiting.
            self.clock = spawn_start + 1;
        }
    }
}

fn origin<'i>(
    caller: &'i Unit,
    invocation: &'i Invocation,
    certainty: Option<Certainty>,
) -> Origin<'i> {
    Origin {
        caller: &caller.id,
        invocation,
        certainty,
    }
}

/// An arm whose own steps end in an error or panic cannot complete normally.
fn ends_in_failure(steps: &[Step]) -> bool {
    steps.iter().any(|step| {
        matches!(
            step,
            Step::Exit {
                kind: ExitKind::Error | ExitKind::Panic,
                ..
            }
        )
    })
}
