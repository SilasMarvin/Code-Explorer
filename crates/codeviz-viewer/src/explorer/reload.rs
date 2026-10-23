//! Taking in a new version of the codebase while the viewer is open. The graph keeps
//! its shape and each module its color; the focus, the journeys and their cursors stay
//! where they were wherever what they stood on still exists.

use std::collections::HashMap;

use codeviz_core::{Journey, JourneyOptions, Moves, Outcome, Span, SpanId, SpanTarget, System};
use codeviz_ir::UnitId;
use codeviz_layout::{ForceGraph, LayoutConfig, MapConfig};

use super::{Explorer, Track};

/// Simulation steps the nodes a reload kept hold still for, while new ones find their
/// places beside them.
const HOLD_STEPS: usize = 60;

/// A span's place in its journey: for each call from the root down to it, what the
/// call runs and which of its caller's calls to that it is. A journey rebuilt from
/// changed code has the same call at the same path, as long as its callers still make
/// it.
type SpanPath = Vec<(String, usize)>;

/// What a journey keeps across a reload.
struct Carried {
    root: UnitId,
    history: Vec<UnitId>,
    /// The outcome it shows, by kind and label; `None` for every path at once.
    outcome: Option<Outcome>,
    cursor: Option<SpanPath>,
}

impl Carried {
    fn of(track: &Track, follow: &impl Fn(&UnitId) -> UnitId) -> Option<Self> {
        Some(Self {
            root: follow(track.root.as_ref()?),
            history: track.history.iter().map(follow).collect(),
            outcome: track
                .scenario
                .and_then(|index| track.scenarios.get(index))
                .map(|scenario| scenario.outcome.clone()),
            cursor: track
                .journey
                .as_ref()
                .zip(track.selected)
                .map(|(journey, span)| path_to(journey, span, follow)),
        })
    }

    /// The journey rebuilt over `system`, unless where it starts is gone.
    fn restore(
        self,
        system: &System,
        options: &JourneyOptions,
        layout: &LayoutConfig,
    ) -> Option<Track> {
        system.unit(&self.root)?;
        let mut track = Track {
            root: Some(self.root),
            history: self
                .history
                .into_iter()
                .filter(|unit| system.unit(unit).is_some())
                .collect(),
            ..Track::default()
        };
        track.reroot(system, options, layout);
        let scenario = match &self.outcome {
            None => None,
            Some(outcome) => track
                .scenarios
                .iter()
                .position(|s| s.outcome == *outcome)
                .or(track.scenario),
        };
        if scenario != track.scenario {
            track.scenario = scenario;
            track.rebuild(system, options, layout);
        }
        if let (Some(journey), Some(path)) = (&track.journey, &self.cursor) {
            track.selected = Some(find(journey, path).0);
        }
        Some(track)
    }
}

impl Explorer {
    /// Takes in a new version of the codebase: journeys run over `system`, and the
    /// graph lays out `layout` when given, as [`Self::with_layout`] has it. `renamed`
    /// gives the old id of each function that moved, by its new id, so it keeps its
    /// place.
    ///
    /// Every function keeps its place in the graph, and one that is new starts beside
    /// what calls it; the graph then settles around the change. The focus, what was
    /// focused before, the spotlight and the lens carry over, and so does each journey
    /// whose start still exists, with its outcome, its cursor and the calls opened by
    /// hand: a cursor whose call went away lands on the nearest caller that remains.
    pub(crate) fn reload(&mut self, system: System, layout: Option<System>, renamed: &Moves) {
        let new_ids: HashMap<&UnitId, &UnitId> =
            renamed.iter().map(|(now, was)| (was, now)).collect();
        let follow = |id: &UnitId| {
            new_ids
                .get(id)
                .map_or_else(|| id.clone(), |&now| now.clone())
        };

        for (tile, &at) in self.map.tiles.iter().zip(&self.graph.positions) {
            self.places.insert(follow(&tile.unit), at);
            self.places.insert(tile.unit.clone(), at);
        }
        let carried: Vec<Option<Carried>> = self
            .tracks
            .iter()
            .map(|track| Carried::of(track, &follow))
            .collect();
        let expanded: Vec<SpanPath> = self.journey().map_or_else(Vec::new, |journey| {
            self.expanded
                .iter()
                .map(|&span| path_to(journey, span, &follow))
                .collect()
        });

        let placed = layout.as_ref().unwrap_or(&system);
        let map = codeviz_layout::map(placed, &MapConfig::default());
        let mut graph = ForceGraph::new(placed, &map, self.clustering.forces());
        graph.resume(&map, &self.places);
        graph.number_clusters_like(&self.graph);
        let renumbered: Vec<Option<usize>> = self
            .map
            .tiles
            .iter()
            .map(|tile| map.tile_of(&follow(&tile.unit)))
            .collect();
        let moved = |tile: usize| renumbered.get(tile).copied().flatten();
        // New nodes grow in from specks.
        let mut visibility = vec![0.0; map.tiles.len()];
        for (old, new) in renumbered.iter().enumerate() {
            if let Some(shown) = new.and_then(|new| visibility.get_mut(new)) {
                *shown = self.visibility(old);
            }
        }
        self.graph_focus = self.graph_focus.and_then(moved);
        self.focus_history = self
            .focus_history
            .iter()
            .filter_map(|&t| moved(t))
            .collect();
        self.hovered_tile = self.hovered_tile.and_then(moved);

        self.system = system;
        self.layout_system = layout;
        self.map = map;
        self.graph = graph;
        self.visibility = visibility;
        self.settling = true;
        self.holding = HOLD_STEPS;
        self.changed_tiles.clear();
        self.reach = None;
        self.focus_reach = None;

        let active = self.active;
        self.tracks.clear();
        self.active = 0;
        for (index, carried) in carried.into_iter().enumerate() {
            if let Some(track) =
                carried.and_then(|c| c.restore(&self.system, &self.options, &self.layout))
            {
                if index == active {
                    self.active = self.tracks.len();
                }
                self.tracks.push(track);
            }
        }
        if self.tracks.is_empty() {
            // Start again from the focus, else the first entry.
            let root = self
                .graph_focus
                .and_then(|tile| self.tile_unit(tile))
                .or_else(|| self.system.entries().first().map(|e| &e.unit))
                .cloned();
            let mut track = Track {
                root,
                ..Track::default()
            };
            track.reroot(&self.system, &self.options, &self.layout);
            self.tracks.push(track);
        }
        self.expanded = self.journey().map_or_else(Default::default, |journey| {
            expanded
                .iter()
                .map(|path| find(journey, path))
                .filter(|&(_, whole)| whole)
                .map(|(span, _)| span)
                .collect()
        });
        self.disclosure_epoch += 1;
        self.renumbered = renumbered;
        self.reloads += 1;
        self.recolor();
    }

    /// How many times a new version of the codebase was taken in.
    pub(crate) fn reloads(&self) -> u64 {
        self.reloads
    }

    /// The node `tile` became on the last reload, if it is still there.
    pub(crate) fn renumbered(&self, tile: usize) -> Option<usize> {
        self.renumbered.get(tile).copied().flatten()
    }
}

/// What a span runs, as its path names it: the unit, under its new id if it moved.
fn key(span: &Span, follow: &impl Fn(&UnitId) -> UnitId) -> String {
    match &span.target {
        SpanTarget::Unit(unit) => follow(unit).to_string(),
        _ => format!("?{}", span.text.as_deref().unwrap_or_default()),
    }
}

fn path_to(journey: &Journey, span: SpanId, follow: &impl Fn(&UnitId) -> UnitId) -> SpanPath {
    let mut path = Vec::new();
    let mut at = span;
    while let Some(this) = journey.span(at)
        && let Some(caller) = this.parent.and_then(|p| journey.span(p))
    {
        let wanted = key(this, follow);
        let nth = caller
            .children
            .iter()
            .take_while(|&&child| child != at)
            .filter_map(|&child| journey.span(child))
            .filter(|child| key(child, follow) == wanted)
            .count();
        path.push((wanted, nth));
        at = this.parent.unwrap_or(at);
    }
    path.reverse();
    path
}

/// The span at `path`, or as far along it as the journey still goes; says whether it
/// went all the way. Of fewer calls to the same thing than before, it takes the last.
fn find(journey: &Journey, path: &SpanPath) -> (SpanId, bool) {
    let same = UnitId::clone;
    let mut at = journey.root();
    for (wanted, nth) in path {
        let Some(span) = journey.span(at) else {
            return (at, false);
        };
        let mut matching = span.children.iter().copied().filter(|&child| {
            journey
                .span(child)
                .is_some_and(|c| key(c, &same) == *wanted)
        });
        let found = matching.clone().nth(*nth).or_else(|| matching.next_back());
        match found {
            Some(next) => at = next,
            None => return (at, false),
        }
    }
    (at, true)
}

#[cfg(test)]
mod tests {
    use bevy::math::Vec3;
    use codeviz_ir::Document;

    use super::*;
    use crate::explorer::{Step, tests::explorer};

    /// Functions as `(name, calls)`, as the explorer fixture writes them.
    fn system(units: &[(&str, &[&str])]) -> System {
        let units: Vec<String> = units
            .iter()
            .map(|(name, calls)| {
                let calls: Vec<String> = calls
                    .iter()
                    .map(|c| {
                        format!(
                            r#"{{ "key": "{c}#0", "text": "{c}", "targets": [{{ "unit": "x:{c}", "certainty": "exact" }}] }}"#
                        )
                    })
                    .collect();
                format!(
                    r#"{{ "id": "x:{name}", "name": "{name}", "kind": "fn", "role": "invocable", "calls": [{}] }}"#,
                    calls.join(",")
                )
            })
            .collect();
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [{}] }}"#,
            units.join(",")
        );
        System::new([Document::from_json(&json).unwrap()])
    }

    fn tile(explorer: &Explorer, name: &str) -> Option<usize> {
        explorer.map().tile_of(&UnitId::new(format!("x:{name}")))
    }

    fn point(explorer: &Explorer, name: &str) -> Vec3 {
        tile(explorer, name)
            .and_then(|t| explorer.tile_point(t))
            .unwrap()
    }

    fn at(explorer: &Explorer) -> String {
        explorer.label(explorer.selected().unwrap())
    }

    #[test]
    fn a_reload_keeps_the_graph_still_and_the_focus_and_cursor_where_they_were() {
        let mut explorer = explorer();
        explorer.settle_now();
        explorer.open_at(tile(&explorer, "a").unwrap());
        explorer.step(Step::Into);
        assert_eq!(at(&explorer), "c");
        let before: Vec<Vec3> = ["main", "a", "c"]
            .iter()
            .map(|n| point(&explorer, n))
            .collect();

        // b goes; a now calls d as well.
        explorer.reload(
            system(&[("main", &["a"]), ("a", &["c", "d"]), ("c", &[]), ("d", &[])]),
            None,
            &Moves::new(),
        );
        let after: Vec<Vec3> = ["main", "a", "c"]
            .iter()
            .map(|n| point(&explorer, n))
            .collect();
        assert_eq!(before, after, "nothing that stayed moves");
        assert!(tile(&explorer, "b").is_none());
        assert!(
            point(&explorer, "d").distance(point(&explorer, "a")) < 8.0,
            "d starts beside a, which calls it"
        );
        assert_eq!(explorer.anchor(), tile(&explorer, "a"), "the camera's node");
        assert_eq!(explorer.graph_focus(), tile(&explorer, "c"), "the cursor's");
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:a"));
        assert_eq!(at(&explorer), "c", "the cursor stays on its call");
        assert_eq!(explorer.reloads(), 1);
        assert!(explorer.settling(), "the graph settles around the change");
    }

    #[test]
    fn a_cursor_whose_call_went_away_lands_on_its_caller() {
        let mut explorer = explorer();
        explorer.step(Step::Into);
        explorer.step(Step::Into);
        assert_eq!(at(&explorer), "c");
        explorer.reload(
            system(&[("main", &["a", "b"]), ("a", &[]), ("b", &[]), ("c", &[])]),
            None,
            &Moves::new(),
        );
        assert_eq!(at(&explorer), "a");
    }

    #[test]
    fn a_renamed_function_keeps_its_place_and_the_focus() {
        let mut explorer = explorer();
        explorer.settle_now();
        let a = tile(&explorer, "a").unwrap();
        explorer.open_at(a);
        let was = point(&explorer, "a");
        let renamed = Moves::from([(UnitId::new("x:a2"), UnitId::new("x:a"))]);
        explorer.reload(
            system(&[
                ("main", &["a2", "b"]),
                ("a2", &["c"]),
                ("b", &[]),
                ("c", &[]),
            ]),
            None,
            &renamed,
        );
        assert_eq!(point(&explorer, "a2"), was);
        assert_eq!(explorer.graph_focus(), tile(&explorer, "a2"));
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:a2"));
        assert_eq!(explorer.renumbered(a), tile(&explorer, "a2"));
    }

    #[test]
    fn a_journey_whose_start_went_away_closes() {
        let mut explorer = explorer();
        explorer.add_track(UnitId::new("x:b"));
        assert_eq!((explorer.tracks().len(), explorer.active()), (2, 1));
        explorer.reload(
            system(&[("main", &["a"]), ("a", &["c"]), ("c", &[])]),
            None,
            &Moves::new(),
        );
        assert_eq!(explorer.tracks().len(), 1);
        assert_eq!(explorer.active(), 0);
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:main"));
    }

    #[test]
    fn a_function_that_comes_back_returns_to_its_place() {
        let mut explorer = explorer();
        explorer.settle_now();
        let was = point(&explorer, "b");
        let without_b = system(&[("main", &["a"]), ("a", &["c"]), ("c", &[])]);
        explorer.reload(without_b, None, &Moves::new());
        for _ in 0..50 {
            explorer.simulate(2);
        }
        explorer.reload(
            system(&[("main", &["a", "b"]), ("a", &["c"]), ("b", &[]), ("c", &[])]),
            None,
            &Moves::new(),
        );
        assert_eq!(point(&explorer, "b"), was);
    }

    #[test]
    fn calls_opened_by_hand_stay_open() {
        let mut explorer = explorer();
        let a_span = explorer
            .journey()
            .and_then(|j| j.span(j.root())?.children.first().copied())
            .unwrap();
        explorer.toggle_expanded(a_span);
        assert!(
            explorer
                .disclosed_tiles()
                .contains_key(&tile(&explorer, "c").unwrap())
        );
        explorer.reload(
            system(&[("main", &["b", "a"]), ("a", &["c"]), ("b", &[]), ("c", &[])]),
            None,
            &Moves::new(),
        );
        assert!(
            explorer
                .disclosed_tiles()
                .contains_key(&tile(&explorer, "c").unwrap()),
            "a's calls stay open, though a is now main's second call"
        );
    }
}
