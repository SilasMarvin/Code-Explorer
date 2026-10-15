//! Viewer state that is independent of rendering: the loaded system, the current
//! journey and its layout, the cursor, the outcome being shown, and the codebase map
//! with what reaches the cursor's unit.

use std::collections::HashMap;

use bevy::prelude::*;
use codeviz_core::{
    Journey, JourneyOptions, Level, Scenario, Span, SpanId, SpanTarget, StopReason, System,
    Upstream, display_name, scenarios, upstream,
};
use codeviz_ir::{Certainty, ExitKind, Location, Mode, Unit, UnitId};
use codeviz_layout::{CodebaseMap, LayoutConfig, MapConfig, Scene};

use crate::geometry::{Arrangement, BarBox, MapFrame};

/// How sure we are that a span really happens as drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Confidence {
    /// Resolved by the producer's rules, declared, or observed.
    Known,
    /// Matched by name, ambiguous, or unresolved.
    Guessed,
}

impl Confidence {
    pub(crate) fn of(span: &Span) -> Self {
        match (&span.target, span.certainty) {
            (SpanTarget::Unit(_), Some(Certainty::Inferred)) => Self::Guessed,
            (SpanTarget::Unit(_), _) => Self::Known,
            _ => Self::Guessed,
        }
    }
}

/// A place in the happy path where another outcome splits off.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Divergence {
    pub(crate) span: SpanId,
    pub(crate) scenario: usize,
    pub(crate) kind: ExitKind,
    pub(crate) label: String,
}

/// A span in one of the open journeys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SpanRef {
    pub(crate) track: usize,
    pub(crate) span: SpanId,
}

/// One open journey: where it starts, which outcome it shows, its own cursor, and
/// where it sits in the world.
#[derive(Debug, Default)]
pub(crate) struct Track {
    root: Option<UnitId>,
    history: Vec<UnitId>,
    scenarios: Vec<Scenario>,
    scenario: Option<usize>,
    journey: Option<Journey>,
    scene: Scene,
    divergences: Vec<Divergence>,
    selected: Option<SpanId>,
    offset: Vec3,
    /// Each span's bar in the world, when the arrangement places it.
    boxes: Vec<Option<BarBox>>,
    /// What the bars occupy in the world, as `(min, max)`.
    bounds: (Vec3, Vec3),
    /// The color of each of the scene's lanes, shared with the other journeys.
    colors: Vec<usize>,
}

impl Track {
    pub(crate) fn root(&self) -> Option<&UnitId> {
        self.root.as_ref()
    }

    pub(crate) fn journey(&self) -> Option<&Journey> {
        self.journey.as_ref()
    }

    pub(crate) fn scene(&self) -> &Scene {
        &self.scene
    }

    pub(crate) fn divergences(&self) -> &[Divergence] {
        &self.divergences
    }

    pub(crate) fn selected(&self) -> Option<SpanId> {
        self.selected
    }

    pub(crate) fn span(&self, id: SpanId) -> Option<&Span> {
        self.journey.as_ref()?.span(id)
    }

    /// The unit a span runs, if it resolved to exactly one.
    pub(crate) fn unit_id(&self, id: SpanId) -> Option<&UnitId> {
        match &self.span(id)?.target {
            SpanTarget::Unit(unit) => Some(unit),
            _ => None,
        }
    }

    /// The span's bar in the world.
    pub(crate) fn bar(&self, id: SpanId) -> Option<&BarBox> {
        self.boxes.get(id.index())?.as_ref()
    }

    /// What the bars occupy in the world, as `(min, max)`.
    pub(crate) fn bounds(&self) -> (Vec3, Vec3) {
        self.bounds
    }

    /// The color index for a span's bar: the same module has the same color in
    /// every journey.
    pub(crate) fn color(&self, id: SpanId) -> usize {
        self.scene
            .nodes
            .get(id.index())
            .and_then(|placed| self.colors.get(placed.lane))
            .copied()
            .unwrap_or_default()
    }

    /// The first span that runs `unit`.
    pub(crate) fn find(&self, unit: &UnitId) -> Option<SpanId> {
        self.journey.as_ref().and_then(|j| {
            j.spans()
                .find(|(_, s)| s.target == SpanTarget::Unit(unit.clone()))
                .map(|(id, _)| id)
        })
    }

    /// Lowest point of the journey below its origin, in world units.
    fn height(&self) -> f32 {
        -self.scene.bounds.0[1] + 1.0
    }

    fn reroot(&mut self, system: &System, options: &JourneyOptions, layout: &LayoutConfig) {
        self.scenarios = self
            .root
            .as_ref()
            .map(|root| scenarios(system, root, &options.zoom))
            .unwrap_or_default();
        self.scenario = (!self.scenarios.is_empty()).then_some(0);
        self.rebuild(system, options, layout);
    }

    fn rebuild(&mut self, system: &System, options: &JourneyOptions, layout: &LayoutConfig) {
        let scenario = self.scenario.and_then(|i| self.scenarios.get(i));
        self.journey = self
            .root
            .as_ref()
            .and_then(|root| Journey::unroll(system, root, scenario, options.clone()));
        self.scene = self
            .journey
            .as_ref()
            .map(|journey| codeviz_layout::layout(system, journey, layout))
            .unwrap_or_default();
        self.divergences = self.find_divergences();
        self.selected = self.journey.as_ref().map(Journey::root);
    }

    /// On the happy path, where each failing outcome splits off: the deepest unit on
    /// its chain that the happy path also runs.
    fn find_divergences(&self) -> Vec<Divergence> {
        let (Some(journey), Some(0)) = (&self.journey, self.scenario) else {
            return Vec::new();
        };
        self.scenarios
            .iter()
            .enumerate()
            .skip(1)
            .filter_map(|(index, scenario)| {
                let span = scenario.chain.iter().rev().find_map(|site| {
                    journey
                        .spans()
                        .find(|(_, s)| s.target == SpanTarget::Unit(site.unit.clone()))
                        .map(|(id, _)| id)
                })?;
                Some(Divergence {
                    span,
                    scenario: index,
                    kind: scenario.outcome.kind,
                    label: scenario.outcome.label.clone(),
                })
            })
            .collect()
    }
}

/// Space between stacked journeys.
const TRACK_GAP: f32 = 3.5;
/// Space between layered journeys.
const LAYER_GAP: f32 = 9.0;
/// Space between journeys' columns on one city tile.
const COLUMN_SPACING: f32 = 0.85;
/// Height of one tick in the city.
const CITY_TICK: f32 = 1.0;
/// Square tiles, few to a column, so the map reads as ground.
const CITY_MAP: MapConfig = MapConfig {
    tile_width: 2.6,
    tile_height: 2.6,
    gap: 0.6,
    header: 1.6,
    max_rows: 4,
    max_width: 48.0,
};

fn stacked_map(max_width: f32) -> MapConfig {
    MapConfig {
        max_width,
        ..MapConfig::default()
    }
}

const BAR_HEIGHT: f32 = 0.8;
const BAR_DEPTH: f32 = 0.8;
/// Space left between consecutive bars on one row.
const BAR_GAP: f32 = 0.25;

/// Bars along X at the layout's positions; `lanes` keeps the layout's lane depth.
fn place_bars(track: &mut Track, tick: f32, lanes: bool) {
    let origin = track.offset;
    track.boxes = track
        .scene
        .nodes
        .iter()
        .map(|placed| {
            let length = (placed.length - BAR_GAP).max(tick * 0.4);
            let [x, y, z] = placed.position;
            let start = origin + Vec3::new(x, y, if lanes { z } else { 0.0 });
            let enter = track.span(placed.span).map_or(0, |s| s.enter);
            Some(BarBox {
                center: start + Vec3::X * length / 2.0,
                size: Vec3::new(length, BAR_HEIGHT, BAR_DEPTH),
                start,
                axis: Vec3::X * tick,
                enter,
            })
        })
        .collect();
    track.bounds = box_bounds(&track.boxes);
}

/// Columns rising from each span's tile, from when it enters to when it returns.
fn place_columns(track: &mut Track, map: &CodebaseMap, frame: &MapFrame) {
    let shift = track.offset;
    track.boxes = track
        .scene
        .nodes
        .iter()
        .map(|placed| {
            let span = track.span(placed.span)?;
            let tile = map.tile(map.tile_of(track.unit_id(placed.span)?)?)?;
            let ground = frame.point(tile.position) + shift;
            let bottom = span.enter as f32 * CITY_TICK;
            let top = (span.exit as f32 * CITY_TICK - 0.12).max(bottom + 0.3);
            let start = Vec3::new(ground.x, bottom, ground.z);
            Some(BarBox {
                center: Vec3::new(ground.x, f32::midpoint(bottom, top), ground.z),
                size: Vec3::new(0.6, top - bottom, 0.6),
                start,
                axis: Vec3::Y * CITY_TICK,
                enter: span.enter,
            })
        })
        .collect();
    track.bounds = box_bounds(&track.boxes);
}

fn box_bounds(boxes: &[Option<BarBox>]) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for bar in boxes.iter().flatten() {
        min = min.min(bar.center - bar.size / 2.0);
        max = max.max(bar.center + bar.size / 2.0);
    }
    if min.x > max.x {
        return (Vec3::ZERO, Vec3::ZERO);
    }
    (min, max)
}

#[derive(Resource, Debug)]
pub(crate) struct Explorer {
    system: System,
    pub(crate) options: JourneyOptions,
    pub(crate) layout: LayoutConfig,
    /// Open journeys, stacked top to bottom.
    tracks: Vec<Track>,
    /// The journey the cursor keys, the outline and the side panels follow.
    active: usize,
    pub(crate) hovered: Option<SpanRef>,
    /// Whether the rest of the codebase is drawn around the journeys.
    codebase: bool,
    map: CodebaseMap,
    /// The configuration `map` was laid out with.
    map_config: Option<MapConfig>,
    /// Where the map lies in the world.
    map_frame: MapFrame,
    arrangement: Arrangement,
    /// The unit the layers were last lined up on.
    aligned_on: Option<UnitId>,
    /// Bumped when the camera should frame everything again.
    framing: u64,
    /// Map tiles each journey runs, with the color each is drawn in there.
    journey_tiles: Vec<HashMap<usize, usize>>,
    /// What can reach the cursor's unit.
    upstream: Option<(UnitId, Upstream)>,
    /// Fewest calls from each map tile to the cursor's unit.
    upstream_tiles: HashMap<usize, u32>,
    pub(crate) hovered_tile: Option<usize>,
    /// Lane label to color index, in order of first appearance across journeys.
    lane_colors: HashMap<String, usize>,
    /// Bumped whenever a journey or the map is rebuilt.
    revision: u64,
}

impl Explorer {
    pub(crate) fn new(system: System, root: Option<UnitId>, options: JourneyOptions) -> Self {
        let root = root.or_else(|| system.entries().first().map(|e| e.unit.clone()));
        let mut explorer = Self {
            system,
            options,
            layout: LayoutConfig::default(),
            tracks: vec![Track::default()],
            active: 0,
            hovered: None,
            codebase: false,
            map: CodebaseMap::default(),
            map_config: None,
            map_frame: MapFrame::default(),
            arrangement: Arrangement::default(),
            aligned_on: None,
            framing: 0,
            journey_tiles: Vec::new(),
            upstream: None,
            upstream_tiles: HashMap::new(),
            hovered_tile: None,
            lane_colors: HashMap::new(),
            revision: 0,
        };
        if let Some(root) = root {
            explorer.set_root(root);
            explorer.track_mut().history.clear();
        }
        explorer
    }

    pub(crate) fn system(&self) -> &System {
        &self.system
    }

    pub(crate) fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub(crate) fn active(&self) -> usize {
        self.active
    }

    /// The active journey.
    pub(crate) fn track(&self) -> &Track {
        // `tracks` is never empty.
        &self.tracks[self.active.min(self.tracks.len() - 1)]
    }

    fn track_mut(&mut self) -> &mut Track {
        let index = self.active.min(self.tracks.len() - 1);
        &mut self.tracks[index]
    }

    pub(crate) fn root(&self) -> Option<&UnitId> {
        self.track().root()
    }

    pub(crate) fn journey(&self) -> Option<&Journey> {
        self.track().journey()
    }

    pub(crate) fn scenarios(&self) -> &[Scenario] {
        &self.track().scenarios
    }

    /// The outcome shown; `None` shows every path at once.
    pub(crate) fn scenario(&self) -> Option<usize> {
        self.track().scenario
    }

    pub(crate) fn divergences(&self) -> &[Divergence] {
        self.track().divergences()
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn can_go_back(&self) -> bool {
        !self.track().history.is_empty()
    }

    /// The span the active journey's cursor is on.
    pub(crate) fn selected(&self) -> Option<SpanId> {
        self.track().selected
    }

    pub(crate) fn select(&mut self, span: Option<SpanId>) {
        self.track_mut().selected = span;
    }

    /// Makes `track` active with its cursor on `span`.
    pub(crate) fn select_in(&mut self, at: SpanRef) {
        if at.track < self.tracks.len() {
            if self.active != at.track {
                self.active = at.track;
                self.refresh_journey_tiles();
            }
            self.select(Some(at.span));
        }
    }

    pub(crate) fn set_root(&mut self, root: UnitId) {
        if self.root() == Some(&root) {
            return;
        }
        let track = self.track_mut();
        if let Some(previous) = track.root.replace(root) {
            track.history.push(previous);
        }
        self.reroot();
    }

    pub(crate) fn back(&mut self) {
        if let Some(previous) = self.track_mut().history.pop() {
            self.track_mut().root = Some(previous);
            self.reroot();
        }
    }

    /// Recomputes outcomes for the active journey's root and opens on the happy path.
    pub(crate) fn reroot(&mut self) {
        let index = self.active.min(self.tracks.len() - 1);
        if let Some(track) = self.tracks.get_mut(index) {
            track.reroot(&self.system, &self.options, &self.layout);
        }
        self.arrange();
    }

    pub(crate) fn set_scenario(&mut self, scenario: Option<usize>) {
        if self.scenario() != scenario {
            self.track_mut().scenario = scenario;
            self.rebuild();
            // Land on the exit the outcome ends at.
            if scenario.is_some_and(|s| s > 0) {
                let exit = self.journey().and_then(|j| {
                    j.spans()
                        .filter(|(_, s)| s.exits.iter().any(|e| e.kind != ExitKind::Return))
                        .map(|(id, _)| id)
                        .last()
                        .or_else(|| j.spans().last().map(|(id, _)| id))
                });
                self.select(exit);
            }
        }
    }

    /// Re-roots the active journey at the selected span's unit.
    pub(crate) fn trace_from_selected(&mut self) {
        if let Some(id) = self.focus().cloned() {
            self.set_root(id);
        }
    }

    pub(crate) fn toggle_zoom(&mut self, unit: UnitId) {
        let level = match self.options.zoom.level(&unit) {
            Level::Summary => Level::Detail,
            Level::Detail => Level::Summary,
        };
        self.options.zoom.set(unit, level);
        for track in &mut self.tracks {
            track.reroot(&self.system, &self.options, &self.layout);
        }
        self.arrange();
    }

    /// Rebuilds the active journey and re-places everything around it.
    pub(crate) fn rebuild(&mut self) {
        let index = self.active.min(self.tracks.len() - 1);
        if let Some(track) = self.tracks.get_mut(index) {
            track.rebuild(&self.system, &self.options, &self.layout);
        }
        self.arrange();
    }

    pub(crate) fn arrangement(&self) -> Arrangement {
        self.arrangement
    }

    pub(crate) fn set_arrangement(&mut self, arrangement: Arrangement) {
        if self.arrangement != arrangement {
            self.arrangement = arrangement;
            self.arrange();
        }
    }

    /// Bumped when the camera should frame everything again.
    pub(crate) fn framing(&self) -> u64 {
        self.framing
    }

    /// Whether the codebase map is drawn: when asked for, and always as the ground of
    /// the city.
    pub(crate) fn shows_map(&self) -> bool {
        self.codebase || self.arrangement == Arrangement::City
    }

    /// In layers, slides the panes so the cursor's function lines up in all of them.
    /// Returns whether anything moved.
    pub(crate) fn realign(&mut self) -> bool {
        if self.arrangement != Arrangement::Layers || self.tracks.len() < 2 {
            return false;
        }
        let focus = self.focus().cloned();
        if focus == self.aligned_on {
            return false;
        }
        self.aligned_on = focus;
        self.place();
        self.revision += 1;
        true
    }

    /// Re-places everything after a journey, the set of journeys or the arrangement
    /// changed, and asks for a redraw and a fresh camera framing.
    fn arrange(&mut self) {
        for track in &mut self.tracks {
            let colors = &mut self.lane_colors;
            track.colors = track
                .scene
                .lanes
                .iter()
                .map(|lane| {
                    let next = colors.len();
                    *colors.entry(lane.label.clone()).or_insert(next)
                })
                .collect();
        }
        self.aligned_on = self.focus().cloned();
        self.place();
        self.hovered = None;
        self.revision += 1;
        self.framing += 1;
    }

    /// Computes every bar's box and the map's frame for the arrangement.
    fn place(&mut self) {
        let tick = self.layout.tick_spacing;
        match self.arrangement {
            Arrangement::Stacked => {
                let mut top = 0.0;
                for track in &mut self.tracks {
                    track.offset = Vec3::new(0.0, top, 0.0);
                    top -= track.height() + TRACK_GAP;
                    place_bars(track, tick, true);
                }
                if self.shows_map() {
                    let (min, max) = self.journey_bounds();
                    self.layout_map(stacked_map((max.x - min.x).max(30.0)));
                    self.map_frame =
                        MapFrame::wall(Vec3::new(min.x, max.y + 2.5, 0.0), self.map_tile());
                }
            }
            Arrangement::Layers => {
                let focus = self.aligned_on.clone();
                let start_of = |track: &Track| {
                    let id = focus.as_ref().and_then(|f| track.find(f))?;
                    track.scene.nodes.get(id.index()).map(|n| n.position[0])
                };
                let anchor = start_of(self.track());
                for (index, track) in self.tracks.iter_mut().enumerate() {
                    let shift = anchor
                        .zip(start_of(track))
                        .map_or(0.0, |(anchor, own)| anchor - own);
                    track.offset = Vec3::new(shift, 0.0, -(index as f32) * LAYER_GAP);
                    place_bars(track, tick, false);
                }
                if self.shows_map() {
                    let (min, max) = self.journey_bounds();
                    self.layout_map(stacked_map((max.x - min.x).max(30.0)));
                    self.map_frame = MapFrame::floor(
                        Vec3::new(min.x, min.y - 3.0, max.z + 6.0),
                        self.map_tile(),
                    );
                }
            }
            Arrangement::City => {
                self.layout_map(CITY_MAP);
                self.map_frame = MapFrame::floor(Vec3::ZERO, self.map_tile());
                let count = self.tracks.len();
                for (index, track) in self.tracks.iter_mut().enumerate() {
                    let shift = (index as f32 - (count as f32 - 1.0) / 2.0) * COLUMN_SPACING;
                    track.offset = Vec3::X * shift;
                    place_columns(track, &self.map, &self.map_frame);
                }
            }
        }
        self.refresh_journey_tiles();
    }

    /// Lays the map out again only when its configuration changed.
    fn layout_map(&mut self, config: MapConfig) {
        if self.map_config != Some(config) {
            self.map = codeviz_layout::map(&self.system, &config);
            self.map_config = Some(config);
            self.upstream = None;
        }
    }

    fn map_tile(&self) -> Vec2 {
        self.map_config
            .map_or(Vec2::ONE, |c| Vec2::new(c.tile_width, c.tile_height))
    }

    fn journey_bounds(&self) -> (Vec3, Vec3) {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for track in &self.tracks {
            min = min.min(track.bounds.0);
            max = max.max(track.bounds.1);
        }
        if min.x > max.x {
            return (Vec3::ZERO, Vec3::ZERO);
        }
        (min, max)
    }

    /// Opens another journey from `unit` below the others and makes it active. Its
    /// cursor lands on the unit the active cursor was on, when it reaches it.
    pub(crate) fn add_track(&mut self, unit: UnitId) {
        let focus = self.focus().cloned();
        let mut track = Track {
            root: Some(unit),
            ..Track::default()
        };
        track.reroot(&self.system, &self.options, &self.layout);
        if let Some(landing) = focus.and_then(|f| track.find(&f)) {
            track.selected = Some(landing);
        }
        self.tracks.push(track);
        self.active = self.tracks.len() - 1;
        self.arrange();
    }

    /// Opens the selected span's unit as another journey.
    pub(crate) fn pull_out_selected(&mut self) {
        if let Some(unit) = self.focus().cloned() {
            self.add_track(unit);
        }
    }

    /// Closes a journey; the last one stays open.
    pub(crate) fn close_track(&mut self, index: usize) {
        if self.tracks.len() > 1 && index < self.tracks.len() {
            self.tracks.remove(index);
            if self.active >= index && self.active > 0 {
                self.active -= 1;
            }
            self.arrange();
        }
    }

    /// Everything drawn, as `(min, max)`: the journeys, and the map when shown.
    pub(crate) fn bounds(&self) -> (Vec3, Vec3) {
        let (mut min, mut max) = self.journey_bounds();
        if self.shows_map() {
            let (a, b) = self.map.bounds;
            for corner in [[a[0], a[1]], [b[0], b[1]], [a[0], b[1]], [b[0], a[1]]] {
                let point = self.map_frame.point(corner);
                min = min.min(point);
                max = max.max(point);
            }
        }
        (min, max)
    }

    pub(crate) fn codebase(&self) -> bool {
        self.codebase
    }

    pub(crate) fn toggle_codebase(&mut self) {
        self.codebase = !self.codebase;
        self.hovered_tile = None;
        self.arrange();
    }

    fn refresh_journey_tiles(&mut self) {
        self.journey_tiles = self
            .tracks
            .iter()
            .map(|track| {
                track
                    .scene
                    .nodes
                    .iter()
                    .filter_map(|placed| {
                        let unit = track.unit_id(placed.span)?;
                        Some((self.map.tile_of(unit)?, track.color(placed.span)))
                    })
                    .collect()
            })
            .collect();
    }

    pub(crate) fn map(&self) -> &CodebaseMap {
        &self.map
    }

    /// Where the map lies in the world.
    pub(crate) fn map_frame(&self) -> &MapFrame {
        &self.map_frame
    }

    /// A map position in world space.
    pub(crate) fn map_point(&self, position: [f32; 2]) -> Vec3 {
        self.map_frame.point(position)
    }

    /// The color a tile's unit is drawn in on the active journey, if it runs it.
    pub(crate) fn journey_lane(&self, tile: usize) -> Option<usize> {
        self.journey_tiles.get(self.active)?.get(&tile).copied()
    }

    /// The unit under the active cursor.
    pub(crate) fn focus(&self) -> Option<&UnitId> {
        let track = self.track();
        track.selected.and_then(|id| track.unit_id(id))
    }

    /// What can reach the cursor's unit, once [`Self::refresh_upstream`] has run.
    pub(crate) fn upstream(&self) -> Option<&Upstream> {
        self.upstream.as_ref().map(|(_, up)| up)
    }

    /// Recomputes [`Self::upstream`] when the cursor's unit changed. Returns whether it did.
    pub(crate) fn refresh_upstream(&mut self) -> bool {
        let focus = self.focus().cloned();
        if self.upstream.as_ref().map(|(id, _)| id) == focus.as_ref() {
            return false;
        }
        self.upstream = focus.map(|id| {
            let up = upstream(&self.system, &id);
            (id, up)
        });
        self.upstream_tiles.clear();
        let distances = self.upstream.as_ref().map(|(_, up)| &up.distance);
        for (unit, &distance) in distances.into_iter().flatten() {
            if let Some(tile) = self.map.tile_of(unit) {
                let best = self.upstream_tiles.entry(tile).or_insert(distance);
                *best = (*best).min(distance);
            }
        }
        true
    }

    /// Fewest calls from a tile to the cursor's unit, if it can reach it at all.
    pub(crate) fn tile_distance(&self, tile: usize) -> Option<u32> {
        self.upstream_tiles.get(&tile).copied()
    }

    /// Starts the active journey at `unit`, with the cursor on the unit it was on
    /// before when the new journey reaches it: from a caller, straight to the call
    /// that led here.
    pub(crate) fn start_from(&mut self, unit: UnitId) {
        let focus = self.focus().cloned();
        self.set_root(unit);
        if let Some(landing) = focus.and_then(|f| self.track().find(&f)) {
            self.select(Some(landing));
        }
    }

    pub(crate) fn span(&self, id: SpanId) -> Option<&Span> {
        self.track().span(id)
    }

    /// The unit a span of the active journey runs, if it resolved to exactly one.
    pub(crate) fn unit_id(&self, id: SpanId) -> Option<&UnitId> {
        self.track().unit_id(id)
    }

    pub(crate) fn unit(&self, id: SpanId) -> Option<&Unit> {
        self.system.unit(self.unit_id(id)?)
    }

    pub(crate) fn label(&self, id: SpanId) -> String {
        self.label_in(self.track(), id)
    }

    pub(crate) fn label_in(&self, track: &Track, id: SpanId) -> String {
        let Some(span) = track.span(id) else {
            return String::new();
        };
        let name = match &span.target {
            SpanTarget::Unit(unit) => self.unit_name(unit),
            SpanTarget::Ambiguous(ids) => format!(
                "{} → one of {}",
                span.text.as_deref().unwrap_or("?"),
                ids.len()
            ),
            SpanTarget::Unresolved(text) => text.clone(),
        };
        let name = if span.merged > 1 {
            format!("{name} ({} merged)", span.merged)
        } else {
            name
        };
        match span.stopped {
            Some(StopReason::Recursion) => format!("{name} ↻"),
            Some(StopReason::DepthLimit) => format!("{name} …"),
            Some(StopReason::Registered) => format!("→ {name}"),
            Some(StopReason::NotLoaded) | None => name,
        }
    }

    pub(crate) fn unit_name(&self, id: &UnitId) -> String {
        display_name(&self.system, id)
    }

    /// The call stack at a span of the active journey, outermost first.
    pub(crate) fn stack(&self, id: SpanId) -> Vec<SpanId> {
        let mut stack: Vec<SpanId> =
            std::iter::successors(Some(id), |s| self.span(*s).and_then(|s| s.parent)).collect();
        stack.reverse();
        stack
    }

    /// Where the call that started `id` is written: the caller's file and line.
    pub(crate) fn call_site(&self, id: SpanId) -> Option<(&UnitId, &Location)> {
        let (caller, key) = self.span(id)?.call.as_ref()?;
        let location = self.system.unit(caller)?.call(key)?.location.as_ref()?;
        Some((caller, location))
    }

    /// Moves the active cursor. Returns the new position.
    pub(crate) fn step(&mut self, step: Step) -> Option<SpanId> {
        let journey = self.journey()?;
        let current = self.selected().unwrap_or_else(|| journey.root());
        let span = journey.span(current)?;
        let nth = |index: usize| journey.spans().nth(index).map(|(id, _)| id);
        // The first span after `id`'s subtree, in pre-order.
        let after = |id: SpanId| {
            let end = journey.span(id).map_or(0, |s| s.exit);
            journey
                .spans()
                .skip(id.index() + 1)
                .find(|(other, s)| s.enter >= end || !is_within(journey, *other, id))
                .map(|(id, _)| id)
        };
        let next = match step {
            Step::Into => nth(current.index() + 1),
            Step::Back => current.index().checked_sub(1).and_then(nth),
            Step::Over => after(current),
            Step::Out => span.parent.and_then(after).or(span.parent),
            Step::Start => Some(journey.root()),
        };
        if next.is_some() {
            self.select(next);
        }
        next
    }
}

fn is_within(journey: &Journey, span: SpanId, ancestor: SpanId) -> bool {
    std::iter::successors(Some(span), |s| journey.span(*s).and_then(|s| s.parent))
        .any(|s| s == ancestor)
}

/// Debugger-style cursor movement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    /// The next call in time, entering callees.
    Into,
    /// The next call after this one returns.
    Over,
    /// The next call after the caller returns.
    Out,
    /// The previous call in time.
    Back,
    Start,
}

/// Steps the cursor on a timer.
#[derive(Resource, Debug)]
pub(crate) struct Autoplay {
    pub(crate) playing: bool,
    pub(crate) steps_per_second: f32,
    pub(crate) elapsed: f32,
}

impl Default for Autoplay {
    fn default() -> Self {
        Self {
            playing: false,
            steps_per_second: 2.0,
            elapsed: 0.0,
        }
    }
}

/// Whether a span is reached by a call that always happens on this path.
pub(crate) fn is_conditional(span: &Span) -> bool {
    !span.conditions.is_empty() || span.repeats
}

pub(crate) fn is_handoff(span: &Span) -> bool {
    span.mode == Mode::Register
}

#[cfg(test)]
mod tests {
    use codeviz_ir::Document;

    use super::*;

    /// `main -> (a -> c), b`
    fn explorer() -> Explorer {
        let unit = |name: &str, calls: &[&str]| {
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
        };
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [{}, {}, {}, {}] }}"#,
            unit("main", &["a", "b"]),
            unit("a", &["c"]),
            unit("b", &[]),
            unit("c", &[])
        );
        let system = System::new([Document::from_json(&json).unwrap()]);
        Explorer::new(
            system,
            Some(UnitId::new("x:main")),
            JourneyOptions::default(),
        )
    }

    fn at(explorer: &Explorer) -> String {
        explorer.label(explorer.selected().unwrap())
    }

    #[test]
    fn stepping_like_a_debugger() {
        let mut explorer = explorer();
        assert_eq!(at(&explorer), "main");

        explorer.step(Step::Into);
        assert_eq!(at(&explorer), "a");
        explorer.step(Step::Over);
        assert_eq!(at(&explorer), "b", "over skips a's callee c");

        explorer.step(Step::Back);
        explorer.step(Step::Back);
        assert_eq!(at(&explorer), "a");
        explorer.step(Step::Into);
        assert_eq!(at(&explorer), "c");
        explorer.step(Step::Out);
        assert_eq!(
            at(&explorer),
            "b",
            "out leaves a and lands on what follows it"
        );

        explorer.step(Step::Start);
        assert_eq!(at(&explorer), "main");
    }

    #[test]
    fn starting_from_a_caller_keeps_the_cursor_on_the_callee() {
        let mut explorer = explorer();
        explorer.step(Step::Into);
        explorer.step(Step::Into);
        assert_eq!(at(&explorer), "c");

        explorer.toggle_codebase();
        explorer.refresh_upstream();
        let callers: Vec<&str> = explorer
            .upstream()
            .unwrap()
            .direct()
            .map(|e| e.caller.as_str())
            .collect();
        assert_eq!(callers, ["x:a"]);

        explorer.start_from(UnitId::new("x:a"));
        assert_eq!(explorer.root().map(UnitId::as_str), Some("x:a"));
        assert_eq!(at(&explorer), "c");
    }

    #[test]
    fn pulling_out_a_journey_stacks_it_below_with_its_own_cursor() {
        let mut explorer = explorer();
        explorer.step(Step::Into);
        explorer.step(Step::Into);
        assert_eq!(at(&explorer), "c");

        explorer.add_track(UnitId::new("x:a"));
        assert_eq!(explorer.tracks().len(), 2);
        assert_eq!(explorer.active(), 1);
        assert_eq!(at(&explorer), "c", "lands on the unit the cursor was on");
        let [first, second] = explorer.tracks() else {
            panic!("two journeys");
        };
        assert!(second.bounds().1.y < first.bounds().0.y, "stacked below");
        assert_eq!(
            first.color(first.find(&UnitId::new("x:c")).unwrap()),
            second.color(second.find(&UnitId::new("x:c")).unwrap()),
            "one module, one color, in every journey"
        );

        explorer.step(Step::Back);
        assert_eq!(at(&explorer), "a");
        let first_cursor = explorer.tracks()[0].selected();
        assert_eq!(
            first_cursor.map(|s| explorer.label_in(&explorer.tracks()[0], s)),
            Some("c".to_owned()),
            "each journey keeps its own cursor"
        );

        explorer.close_track(1);
        assert_eq!(explorer.tracks().len(), 1);
        assert_eq!(at(&explorer), "c");
        explorer.close_track(0);
        assert_eq!(explorer.tracks().len(), 1, "the last journey stays");
    }

    #[test]
    fn rerooting_keeps_history() {
        let mut explorer = explorer();
        explorer.step(Step::Into);
        explorer.trace_from_selected();
        assert_eq!(explorer.journey().map(Journey::len), Some(2));
        assert!(explorer.can_go_back());

        explorer.back();
        assert_eq!(explorer.journey().map(Journey::len), Some(4));
        assert!(!explorer.can_go_back());
    }
}
