//! Scenarios: the distinct ways an entry point can end.
//!
//! A scenario is a reachable exit together with the chain of propagating call sites
//! that carries it to the entry. Branches that rejoin are not scenarios, so the count
//! stays linear in call sites rather than exponential in branches.

use std::{borrow::Cow, collections::HashMap, rc::Rc};

use codeviz_ir::{ExitKind, Mode, Step, Unit, UnitId};

use crate::{Level, System, Zoom};

/// Bound on escapes tracked per unit, to keep pathological graphs tractable.
const MAX_ESCAPES: usize = 64;

/// Addresses a step inside a unit's body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Seg {
    Step(usize),
    Arm(usize),
    LoopBody,
    Node(usize),
    /// The n-th unit a summary refines, when zoomed in.
    Refinement(usize),
}

pub type StepPath = Vec<Seg>;

/// A call site or exit within a unit.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Site {
    pub unit: UnitId,
    pub path: StepPath,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Outcome {
    pub kind: ExitKind,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scenario {
    pub outcome: Outcome,
    /// From the entry's call site to the origin of the exit. Empty for normal completion.
    pub chain: Vec<Site>,
}

impl Scenario {
    #[must_use]
    pub fn completes(&self) -> bool {
        self.chain.is_empty()
    }
}

/// The unit's body, or its calls in order when the producer gave no control flow.
pub(crate) fn body(unit: &Unit) -> Cow<'_, [Step]> {
    match &unit.body {
        Some(body) => Cow::Borrowed(body),
        None => Cow::Owned(
            unit.calls
                .iter()
                .map(|call| Step::Call {
                    call: call.key.clone(),
                })
                .collect(),
        ),
    }
}

/// Lists how `entry` can end: normal completion first, then each distinct escape.
#[must_use]
pub fn scenarios(system: &System, entry: &UnitId, zoom: &Zoom) -> Vec<Scenario> {
    let mut finder = Finder {
        system,
        zoom,
        memo: HashMap::new(),
        in_progress: Vec::new(),
    };
    let mut scenarios = vec![Scenario {
        outcome: Outcome {
            kind: ExitKind::Return,
            label: "completes".to_owned(),
        },
        chain: Vec::new(),
    }];
    scenarios.extend(finder.escapes(entry).iter().map(|escape| Scenario {
        outcome: escape.outcome.clone(),
        chain: escape.chain.clone(),
    }));
    scenarios
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Escape {
    outcome: Outcome,
    chain: Vec<Site>,
}

struct Finder<'s> {
    system: &'s System,
    zoom: &'s Zoom,
    memo: HashMap<UnitId, Rc<Vec<Escape>>>,
    in_progress: Vec<UnitId>,
}

impl Finder<'_> {
    fn escapes(&mut self, id: &UnitId) -> Rc<Vec<Escape>> {
        if let Some(known) = self.memo.get(id) {
            return Rc::clone(known);
        }
        let Some(unit) = self.system.unit(id) else {
            return Rc::default();
        };
        if self.in_progress.contains(id) {
            return Rc::default();
        }
        self.in_progress.push(id.clone());
        let mut out = Vec::new();
        let refined = refined(self.system, self.zoom, id);
        if refined.is_empty() {
            self.walk(unit, &body(unit), &mut Vec::new(), &mut out);
        }
        for (index, target) in refined.iter().enumerate() {
            for escape in self.escapes(target).iter() {
                let mut chain = vec![Site {
                    unit: id.clone(),
                    path: vec![Seg::Refinement(index)],
                }];
                chain.extend(escape.chain.iter().cloned());
                out.push(Escape {
                    outcome: escape.outcome.clone(),
                    chain,
                });
            }
        }
        self.in_progress.pop();

        let mut unique = Vec::new();
        for escape in out {
            if !unique.contains(&escape) && unique.len() < MAX_ESCAPES {
                unique.push(escape);
            }
        }
        let escapes = Rc::new(unique);
        self.memo.insert(id.clone(), Rc::clone(&escapes));
        escapes
    }

    fn walk(&mut self, unit: &Unit, steps: &[Step], path: &mut StepPath, out: &mut Vec<Escape>) {
        for (index, step) in steps.iter().enumerate() {
            path.push(Seg::Step(index));
            let site = || Site {
                unit: unit.id.clone(),
                path: path.clone(),
            };
            match step {
                Step::Exit { kind, label } if *kind != ExitKind::Return => out.push(Escape {
                    outcome: Outcome {
                        kind: *kind,
                        label: label.clone().unwrap_or_else(|| "error".to_owned()),
                    },
                    chain: vec![site()],
                }),
                Step::Call { call } => {
                    let Some(invocation) = unit.call(call).filter(|i| i.mode == Mode::Call) else {
                        path.pop();
                        continue;
                    };
                    let resolved: Vec<&UnitId> = invocation
                        .targets
                        .iter()
                        .map(|t| &t.unit)
                        .filter(|id| self.system.unit(id).is_some())
                        .collect();
                    if resolved.is_empty() && invocation.propagates {
                        out.push(Escape {
                            outcome: Outcome {
                                kind: ExitKind::Error,
                                label: format!("{} fails", invocation.text),
                            },
                            chain: vec![site()],
                        });
                    }
                    for target in resolved {
                        for escape in self.escapes(target).iter() {
                            if invocation.propagates || escape.outcome.kind == ExitKind::Panic {
                                let mut chain = vec![site()];
                                chain.extend(escape.chain.iter().cloned());
                                out.push(Escape {
                                    outcome: escape.outcome.clone(),
                                    chain,
                                });
                            }
                        }
                    }
                }
                Step::Branch { arms, .. } => {
                    for (index, arm) in arms.iter().enumerate() {
                        path.push(Seg::Arm(index));
                        self.walk(unit, &arm.body, path, out);
                        path.pop();
                    }
                }
                Step::Loop { body, .. } => {
                    path.push(Seg::LoopBody);
                    self.walk(unit, body, path, out);
                    path.pop();
                }
                Step::Dag { nodes } => {
                    for (index, node) in nodes.iter().enumerate() {
                        path.push(Seg::Node(index));
                        self.walk(unit, &node.body, path, out);
                        path.pop();
                    }
                }
                Step::Exit { .. } | Step::Unknown => {}
            }
            path.pop();
        }
    }
}

/// The loaded units `id` refines, when it is zoomed in; empty otherwise.
pub(crate) fn refined(system: &System, zoom: &Zoom, id: &UnitId) -> Vec<UnitId> {
    if zoom.level(id) != Level::Detail {
        return Vec::new();
    }
    system
        .refinements(id)
        .iter()
        .map(|(target, _)| target)
        .filter(|target| system.unit(target).is_some())
        .cloned()
        .collect()
}
