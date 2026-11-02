//! Names on the graph: who is named and in what order, what each name says and how
//! it looks, and where it goes beside its node so that none covers another or a node
//! that matters, and none moves while nothing else does.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use bevy::prelude::*;
use bevy_egui::egui;

use crate::{
    colors::{
        CALLEE, CALLEE_TEXT, CALLER, CALLER_TEXT, CURSOR, CURSOR_TEXT, DEEPER, DEEPER_TEXT, MUTED,
        ON_FILL, egui_color, module_color,
    },
    disclosure::{self, Disclosed},
    explorer::Explorer,
    graph_view,
    nucleus::{self, Ball, Body, Flow, Nucleus},
    sizes::Dot,
    text,
    ui::short,
};

pub(crate) const PILL: egui::Color32 = egui::Color32::from_rgba_premultiplied(9, 11, 16, 205);
/// The dotted thread from a name to its node: grey, as no call is drawn.
const THREAD: egui::Color32 = egui::Color32::from_rgba_premultiplied(120, 124, 134, 190);

/// A function's name, or the route of an entry point.
pub(crate) fn node_name(explorer: &Explorer, tile: usize) -> String {
    let Some(unit) = explorer.tile_unit(tile) else {
        return String::new();
    };
    explorer
        .system()
        .entries()
        .iter()
        .find(|e| &e.unit == unit)
        .map_or_else(
            || {
                explorer
                    .system()
                    .unit(unit)
                    .map_or_else(|| unit.to_string(), |u| u.name.clone())
            },
            |e| e.label.clone(),
        )
}

/// What a graph label names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Named {
    Node(usize),
    Group(usize),
}

/// Tier, then nearer the journey's start (a module first or last within its tier),
/// then bigger first, then a stable tiebreak.
type LabelRank = (u8, usize, std::cmp::Reverse<usize>, usize);

/// Where a label may go around its node, in order of preference.
const SIDES: [egui::Align2; 4] = [
    egui::Align2::LEFT_CENTER,
    egui::Align2::RIGHT_CENTER,
    egui::Align2::CENTER_BOTTOM,
    egui::Align2::CENTER_TOP,
];

/// Room kept around a label's text: its pill.
pub(crate) const PILL_PAD: egui::Vec2 = egui::vec2(5.0, 2.0);
/// How fast a label slides to a new side: the share of the way left it covers in a
/// second is `1 - e^-SLIDE`.
const SLIDE: f32 = 14.0;
/// Seconds a label takes to fade in or out.
const FADE: f32 = 0.18;
/// How far along its side, in steps, a name that must show is nudged when every side
/// is taken, nearest first: no further than a thread to it still reads as its own.
const NUDGES: [i8; 6] = [1, -1, 2, -2, 3, -3];
/// Further along its side, where a name that must show goes in a crowd when nearer
/// would lie nearer another node than its own: its thread ties it to its node.
const THREADED: [i8; 12] = [1, -1, 2, -2, 3, -3, 4, -4, 5, -5, 6, -6];
/// How far, in points from its node's edge, a name may sit and still read as its
/// node's. Past it a name is better left to the pointer, and counted as unnamed;
/// a name held further off takes a nearer place when there is one.
const NEAR: f32 = 40.0;

/// Where a label sits against its node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Spot {
    /// Index into [`SIDES`].
    side: usize,
    /// How far along that side it was nudged, in steps.
    shift: i8,
    /// Whether it lies over its node's middle, its last resort in a crowd.
    over: bool,
}

impl Spot {
    fn beside(side: usize, shift: i8) -> Self {
        Self {
            side,
            shift,
            over: false,
        }
    }

    fn over(side: usize) -> Self {
        Self {
            side,
            shift: 0,
            over: true,
        }
    }

    /// Where a label of `extent` sits here, by a node at `node` of radius `edge`.
    fn rect(self, node: egui::Pos2, edge: f32, extent: egui::Vec2) -> egui::Rect {
        let gap = if self.over { 0.0 } else { edge + 6.0 };
        let (anchor, step) = match SIDES[self.side] {
            egui::Align2::LEFT_CENTER => (
                node + egui::vec2(gap, 0.0),
                egui::vec2(0.0, extent.y + PILL_PAD.y * 3.0),
            ),
            egui::Align2::RIGHT_CENTER => (
                node - egui::vec2(gap, 0.0),
                egui::vec2(0.0, extent.y + PILL_PAD.y * 3.0),
            ),
            egui::Align2::CENTER_BOTTOM => (
                node - egui::vec2(0.0, gap),
                egui::vec2(extent.x / 2.0 + PILL_PAD.x, 0.0),
            ),
            _ => (
                node + egui::vec2(0.0, gap),
                egui::vec2(extent.x / 2.0 + PILL_PAD.x, 0.0),
            ),
        };
        SIDES[self.side]
            .anchor_size(anchor, extent)
            .translate(step * f32::from(self.shift))
    }
}

/// How hard a label is placed when no place beside its node is free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Force {
    /// Not at all: it goes unnamed.
    Never,
    /// Nudged along a side, or wherever it misleads least, but never over another
    /// name.
    Free,
    /// Wherever it misleads least, over another name if it must.
    Always,
}

/// A label as last drawn, so the next frame eases it instead of jumping it.
#[derive(Clone)]
struct Shown {
    at: Spot,
    /// Where its text sits, from its node.
    offset: egui::Vec2,
    opacity: f32,
    galley: std::sync::Arc<egui::Galley>,
    stroke: Option<egui::Color32>,
    /// How far its node's edge is from its middle.
    edge: f32,
}

/// Every label drawn last frame.
type ShownLabels = HashMap<Named, Shown>;

/// How long a label keeps the place it first took: browsing, for as long as the map
/// is browsed; in a journey, for as long as that journey is open, gathered or expanded
/// round its start, and again when Back or Esc returns to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Scene {
    Browse,
    Journey {
        start: Option<usize>,
        expanded: bool,
    },
}

/// Where each label went when it first showed, by scene.
type SpotBook = HashMap<Scene, HashMap<Named, Spot>>;

/// Scenes remembered at most; past it the book starts again.
const SCENES_KEPT: usize = 48;

fn spot_book_id() -> egui::Id {
    egui::Id::new("graph_label_spots")
}

/// Places labels so none overlaps another or a node that matters, and draws them
/// eased from where they were. Gathered, a label keeps the place it first took for as
/// long as its scene lasts; otherwise it keeps its side while that side stays free. It
/// slides when it must change sides, and fades in and out instead of popping.
struct Placer<'a> {
    painter: &'a egui::Painter,
    /// What labels keep off: first what was there before any label, then the
    /// labels placed.
    taken: Vec<egui::Rect>,
    /// How many of `taken` were there before any label: the search box, and the
    /// nodes that matter.
    blocked: usize,
    /// Whether module names are landmarks, kept off other names alone and not off
    /// nodes or the search box, so that what a click lights never moves one:
    /// browsing.
    landmarks: bool,
    /// Every named node's disc, by tile: no function's label sits nearer another of
    /// them than its own.
    nodes: Vec<(usize, egui::Pos2, f32)>,
    /// The lit nodes no name may cover, not even one held where it first showed:
    /// all but those only the pointer lights.
    blocking: Vec<(usize, egui::Pos2, f32)>,
    /// Every sphere drawn, by cluster, its middle and radius on screen: a module's
    /// name lies over none but its own.
    spheres: Vec<(usize, egui::Pos2, f32)>,
    /// Browsing, every faint dot on screen, by tile, as big as it is drawn lit:
    /// names keep off them where they can.
    specks: Vec<(usize, egui::Pos2, f32)>,
    last: ShownLabels,
    next: ShownLabels,
    /// Where each label placed this frame sits.
    anchors: HashMap<Named, egui::Pos2>,
    /// The share of the way to its new side a label covers this frame.
    slide: f32,
    /// How much opacity a label gains or loses this frame.
    fade: f32,
    /// Labels painted over all the others, the last on top.
    top: Vec<Named>,
    /// The journey number each function's label leads with, so a click on the label
    /// goes to that call.
    numbers: HashMap<Named, String>,
    /// Where labels went when they first showed in this scene: they stay there, and
    /// what shows later keeps off them.
    held: HashMap<Named, Spot>,
    /// Where labels new to the scene went this frame, to hold from now on.
    chosen: HashMap<Named, Spot>,
    /// Whether the view rests, so a new label's place is for keeps: while the camera
    /// or the spheres still move, it is only for now.
    keep: bool,
    /// The names drawn come what may: the cursor's and the selection's. Where one
    /// has a place of its own it stays there, so a click or a step never moves it;
    /// where it has none it is drawn right beside its node, over the rest.
    beside: HashSet<Named>,
    /// Those of them drawn only for being the cursor or the selection, which would
    /// have gone unnamed otherwise and so take no room.
    overlaid: HashSet<Named>,
    /// Held names the view has closed up on since, to place for now once every held
    /// name that still has room is placed, and before any new one.
    displaced: Vec<Displaced>,
    /// Names placed for now, away from where they are held, so they return there.
    for_now: HashSet<Named>,
}

/// A held name with no room where it is held, as it was to be placed.
struct Displaced {
    key: Named,
    node: egui::Pos2,
    edge: f32,
    job: egui::text::LayoutJob,
    stroke: Option<egui::Color32>,
    force: Force,
}

impl<'a> Placer<'a> {
    fn new(
        painter: &'a egui::Painter,
        taken: Vec<egui::Rect>,
        nodes: Vec<(usize, egui::Pos2, f32)>,
    ) -> Self {
        let dt = painter.ctx().input(|i| i.stable_dt).min(0.1);
        Self {
            painter,
            blocked: taken.len(),
            landmarks: false,
            taken,
            nodes,
            blocking: Vec::new(),
            spheres: Vec::new(),
            specks: Vec::new(),
            last: painter
                .ctx()
                .data_mut(|d| d.get_temp::<ShownLabels>(Self::id()))
                .unwrap_or_default(),
            next: HashMap::new(),
            anchors: HashMap::new(),
            slide: 1.0 - (-SLIDE * dt).exp(),
            fade: dt / FADE,
            top: Vec::new(),
            numbers: HashMap::new(),
            held: HashMap::new(),
            chosen: HashMap::new(),
            keep: false,
            beside: HashSet::new(),
            overlaid: HashSet::new(),
            displaced: Vec::new(),
            for_now: HashSet::new(),
        }
    }

    fn id() -> egui::Id {
        egui::Id::new("graph_labels")
    }

    /// Places `job` on a side of `node` (a disc of `edge` points) where it overlaps
    /// nothing already placed. A label the scene already holds goes where it went when
    /// it first showed, whatever has shown since: what is new keeps off it, not the
    /// reverse. Only where the view has closed up since, so that names placed before
    /// it would cover much of it there, is it placed elsewhere for now, after every
    /// held name with room, unless `force` is [`Force::Always`]; it goes back when
    /// there is room again. A new label goes where [`Placer::new_spot`] finds room. A
    /// held place more than [`NEAR`] from its node, or over another lit node, gives
    /// way to a nearer one.
    /// A function's name whose held place, still on the graph, is now taken goes
    /// unnamed instead. The cursor's and the selection's names stay where they are
    /// placed; one with no room of its own is drawn right beside its node, over
    /// others, but takes no room, so where the cursor is never decides what else is
    /// named. Returns whether it was placed, or put off until the held names are.
    fn place(
        &mut self,
        key: Named,
        node: egui::Pos2,
        edge: f32,
        job: egui::text::LayoutJob,
        stroke: Option<egui::Color32>,
        force: Force,
    ) -> bool {
        let galley = self.painter.layout_job(job);
        let extent = galley.size();
        let spot = |at: Spot| at.rect(node, edge, extent);
        let previous = self.last.get(&key);
        let held = self.held.get(&key).copied();
        // A held name too far off to read as its node's heals: it takes a nearer
        // place where there is one.
        let far = held.is_some_and(|at| !near(spot(at), node, edge));
        let at = match held {
            Some(at) if far => self
                .new_spot(key, node, edge, extent, force)
                .or((force == Force::Always).then_some(at)),
            Some(at)
                if !self.covers_node(key, spot(at))
                    && (force == Force::Always || self.has_room(spot(at))) =>
            {
                Some(at)
            }
            // A function's name held where it first showed, still on the graph but
            // now over another node or name, goes unnamed rather than elsewhere: what
            // moves nothing moves no name. The cursor's and the selection's are drawn
            // come what may.
            Some(at)
                if matches!(key, Named::Node(_))
                    && force != Force::Always
                    && !self.beside.contains(&key)
                    && self.painter.clip_rect().contains_rect(spot(at)) =>
            {
                return false;
            }
            // The cursor's or the selection's stays where it is held, over the rest,
            // taking no room.
            Some(at)
                if self.beside.contains(&key)
                    && self.painter.clip_rect().contains_rect(spot(at)) =>
            {
                None
            }
            Some(_) => {
                self.displaced.push(Displaced {
                    key,
                    node,
                    edge,
                    job: (*galley.job).clone(),
                    stroke,
                    force,
                });
                return true;
            }
            None => self.new_spot(key, node, edge, extent, force),
        };
        let drawn = match at {
            Some(at) => at,
            None if self.beside.contains(&key) => {
                let stays =
                    held.filter(|&at| !far && self.painter.clip_rect().contains_rect(spot(at)));
                // Where it is held it keeps what is new off it, as it was there first.
                if let Some(at) = stays {
                    self.taken.push(spot(at).expand2(PILL_PAD));
                    at
                } else {
                    let last = previous.map(|p| p.at);
                    self.beside_spot(key, node, edge, extent, [last, held])
                }
            }
            None => return false,
        };
        if let Some(at) = at {
            self.taken.push(spot(at).expand2(PILL_PAD));
            if held != Some(at) && self.keep && !self.for_now.contains(&key) {
                self.chosen.insert(key, at);
            }
        } else {
            self.overlaid.insert(key);
        }
        let target = spot(drawn).min - node;
        let (offset, opacity) = match previous {
            Some(p) => (
                p.offset + (target - p.offset) * self.slide,
                (p.opacity + self.fade).min(1.0),
            ),
            // A new label fades in where it belongs.
            None => (target, self.fade.min(1.0)),
        };
        self.anchors.insert(key, node);
        self.next.insert(
            key,
            Shown {
                at: drawn,
                offset,
                opacity,
                galley,
                stroke,
                edge,
            },
        );
        true
    }

    /// Whether a held name at `rect` is inside the graph and clear of the names placed
    /// so far.
    fn has_room(&self, rect: egui::Rect) -> bool {
        self.painter.clip_rect().contains_rect(rect)
            && !self.names().iter().any(|r| r.intersects(rect))
    }

    /// Whether a function's name at `rect` would touch another lit node,
    /// or a module's over another module's sphere; a landmark never counts nodes, as
    /// module names browsing do not keep off them.
    fn covers_node(&self, key: Named, rect: egui::Rect) -> bool {
        if let Named::Group(own) = key
            && self.spheres.iter().any(|&(cluster, at, radius)| {
                cluster != own && rect.distance_to_pos(at) < radius - 0.5
            })
        {
            return true;
        }
        if self.landmark(key) {
            return false;
        }
        let own = match key {
            Named::Node(tile) => Some(tile),
            Named::Group(_) => None,
        };
        let pill = rect.expand2(PILL_PAD);
        self.blocking
            .iter()
            .any(|&(tile, at, radius)| Some(tile) != own && pill.distance_to_pos(at) < radius)
    }

    /// Where a label new to the scene goes: the side it had last frame if that is
    /// still free, else the first free one. When `force` asks and none is, it is
    /// nudged along a side to the nearest free place, then further, on a thread, else
    /// put where it misleads least, which only [`Force::Always`] lets lie over another
    /// name. Beside its node or nudged, a function's label goes nowhere it would lie
    /// nearer another node than its own, so it never reads as the other's.
    fn new_spot(
        &self,
        key: Named,
        node: egui::Pos2,
        edge: f32,
        extent: egui::Vec2,
        force: Force,
    ) -> Option<Spot> {
        let spot = |at: Spot| at.rect(node, edge, extent);
        // Inside the graph, so no label is cut off at its edge.
        let bounds = self.painter.clip_rect();
        let against = self.against(key);
        let clear = |rect: egui::Rect, pad: egui::Vec2| {
            bounds.contains_rect(rect.expand2(pad))
                && !against.iter().any(|r| r.intersects(rect.expand2(pad)))
                && !self.covers_node(key, rect)
        };
        let fits = |rect: egui::Rect, pad: egui::Vec2| {
            clear(rect, pad) && self.misread(key, rect, node, edge) == 0
        };
        // Near enough its node to read as its own; a name that must show goes
        // further only when nothing nearer is free.
        let close = |at: Spot| near(spot(at), node, edge);
        // A label holds its side until something actually covers it; a new side must
        // be clear with room to spare. That margin keeps a label from flipping back
        // and forth while the view still moves.
        let kept = self
            .last
            .get(&key)
            .map(|p| p.at)
            .filter(|&at| !at.over && close(at) && fits(spot(at), egui::Vec2::ZERO));
        let along = |shifts: &'static [i8]| {
            shifts
                .iter()
                .flat_map(|&shift| (0..SIDES.len()).map(move |side| Spot::beside(side, shift)))
        };
        let free =
            |shifts: &'static [i8]| along(shifts).find(|&at| close(at) && fits(spot(at), PILL_PAD));
        // The nearest clear place a thread can reach.
        let threaded = || {
            along(&THREADED)
                .map(|at| (spot(at).distance_to_pos(node), at))
                .filter(|&(reach, at)| reach <= NEAR + edge && clear(spot(at), PILL_PAD))
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, at)| at)
        };
        // Off every faint dot too where it can be, as any may light with the next
        // click, which would leave this name nowhere to stay.
        let off_specks = |at: &Spot| !self.over_speck(key, spot(*at));
        let free_and_clean =
            || along(&[0]).find(|&at| close(at) && fits(spot(at), PILL_PAD) && off_specks(&at));
        // A label nudged off its node goes back beside it as soon as there is room.
        kept.filter(|at| at.shift == 0 && off_specks(at))
            .or_else(free_and_clean)
            .or(kept.filter(off_specks))
            .or(kept.filter(|at| at.shift == 0))
            .or_else(|| free(&[0]))
            .or(kept)
            .or_else(|| (force != Force::Never).then(|| free(&NUDGES)).flatten())
            .or_else(|| (force != Force::Never).then(threaded).flatten())
            .or_else(|| {
                let (at, over_names) = self.least_misleading(key, node, edge, extent);
                match force {
                    // Over another name if it must, but never cut off by the graph's
                    // edge.
                    Force::Always if bounds.contains_rect(spot(at)) => Some(at),
                    Force::Free if !over_names && close(at) && !self.covers_node(key, spot(at)) => {
                        Some(at)
                    }
                    Force::Always | Force::Free | Force::Never => None,
                }
            })
    }

    /// Places the held names the view has closed up on since, for now, as new ones
    /// are placed: they go back where they are held when there is room again.
    fn place_displaced(&mut self) {
        for displaced in std::mem::take(&mut self.displaced) {
            let key = displaced.key;
            let held = self.held.remove(&key);
            self.for_now.insert(key);
            self.place(
                key,
                displaced.node,
                displaced.edge,
                displaced.job,
                displaced.stroke,
                displaced.force,
            );
            if let Some(at) = held {
                self.held.insert(key, at);
            }
        }
    }

    /// The names placed so far.
    fn names(&self) -> &[egui::Rect] {
        &self.taken[self.blocked..]
    }

    /// Where the cursor's or the selection's name goes, right beside its node: on the
    /// side it was drawn on last, else the side the scene holds it on, else the first
    /// side, that lies over nothing; else wherever it covers least of the names there.
    fn beside_spot(
        &self,
        key: Named,
        node: egui::Pos2,
        edge: f32,
        extent: egui::Vec2,
        before: [Option<Spot>; 2],
    ) -> Spot {
        let bounds = self.painter.clip_rect();
        // On the side it was on before, if it can, so it moves least.
        let candidates: Vec<Spot> = before
            .into_iter()
            .flatten()
            .filter(|at| !at.over)
            .map(|at| Spot::beside(at.side, 0))
            .chain((0..SIDES.len()).map(|side| Spot::beside(side, 0)))
            .collect();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "an area in whole points, at least 0"
        )]
        let under = |rect: egui::Rect| {
            let pill = rect.expand2(PILL_PAD);
            self.names()
                .iter()
                .map(|r| r.intersect(pill))
                .filter(egui::Rect::is_positive)
                .map(|r| r.area() as u32)
                .sum::<u32>()
        };
        candidates
            .iter()
            .copied()
            .min_by_key(|&at| {
                let rect = at.rect(node, edge, extent);
                (
                    under(rect),
                    !bounds.contains_rect(rect),
                    self.misread(key, rect, node, edge),
                )
            })
            .unwrap_or(Spot::beside(0, 0))
    }

    /// Takes a label placed this frame off again: it fades out, and its place is not
    /// held.
    fn unplace(&mut self, key: Named) {
        self.next.remove(&key);
        self.chosen.remove(&key);
    }

    /// Whether a function's name at `rect` would touch a faint dot other than its own.
    fn over_speck(&self, key: Named, rect: egui::Rect) -> bool {
        let Named::Node(own) = key else {
            return false;
        };
        let pill = rect.expand2(PILL_PAD);
        self.specks
            .iter()
            .any(|&(tile, at, radius)| tile != own && pill.distance_to_pos(at) < radius)
    }

    /// Whether `key` names a module as a landmark.
    fn landmark(&self, key: Named) -> bool {
        self.landmarks && matches!(key, Named::Group(_))
    }

    /// What a label of `key` keeps off: everything taken, or for a landmark the names
    /// placed.
    fn against(&self, key: Named) -> &[egui::Rect] {
        if self.landmark(key) {
            self.names()
        } else {
            &self.taken
        }
    }

    /// How much nearer, in whole points, another node lies to a label of `key` at
    /// `rect` than its own at `node`; 0 when its own is nearest, or it names no node.
    fn misread(&self, key: Named, rect: egui::Rect, node: egui::Pos2, edge: f32) -> u32 {
        match key {
            Named::Node(tile) => misread_by(rect, tile, node, edge, &self.nodes),
            Named::Group(_) => 0,
        }
    }

    /// Where a name that must show goes when no place is free: on the graph, off every
    /// other lit node if it can be, wherever it misleads least, over its own node if
    /// need be,
    /// then where it lies over the fewest other names and nodes, nearest; and whether
    /// it lies over another name there. Names placed this frame weigh only after
    /// misreading, as their places may in turn hang on this one's.
    fn least_misleading(
        &self,
        key: Named,
        node: egui::Pos2,
        edge: f32,
        extent: egui::Vec2,
    ) -> (Spot, bool) {
        let bounds = self.painter.clip_rect();
        let (blocked, names) = self.taken.split_at(self.blocked);
        let blocked = if self.landmark(key) { &[] } else { blocked };
        let under = |rects: &[egui::Rect], rect: egui::Rect| {
            rects
                .iter()
                .filter(|r| r.intersects(rect.expand2(PILL_PAD)))
                .count()
        };
        let at = std::iter::once(0)
            .chain(NUDGES)
            .flat_map(|shift| (0..SIDES.len()).map(move |side| Spot::beside(side, shift)))
            .chain((0..SIDES.len()).map(Spot::over))
            .min_by_key(|&at| {
                let rect = at.rect(node, edge, extent);
                (
                    !bounds.contains_rect(rect),
                    self.covers_node(key, rect),
                    self.misread(key, rect, node, edge),
                    under(names, rect),
                    under(blocked, rect),
                    at.over,
                    at.shift.unsigned_abs(),
                )
            })
            .unwrap_or(Spot::beside(0, 0));
        (at, under(names, at.rect(node, edge, extent)) > 0)
    }

    /// Draws what was placed, and fades out what was not where its node, found by
    /// `anchor`, still shows. Remembers it all for the next frame.
    fn finish(mut self, anchor: impl Fn(Named) -> Option<egui::Pos2>) {
        let mut leaving = Vec::new();
        for (key, mut shown) in std::mem::take(&mut self.last) {
            if self.next.contains_key(&key) {
                continue;
            }
            shown.opacity -= self.fade;
            if shown.opacity > 0.0
                && let Some(node) = anchor(key)
            {
                leaving.push((key, node, shown));
            }
        }
        // Leaving labels go underneath the ones that hold their place.
        for (_, node, shown) in &leaving {
            self.paint(*node, shown);
        }
        let rank = |key: &Named| self.top.iter().position(|k| k == key);
        let mut placed: Vec<(&Named, &Shown)> = self.next.iter().collect();
        placed.sort_by_key(|(key, _)| rank(key));
        // A function's name picks it, as a click on the function would.
        let mut hits: Vec<LabelHit> = Vec::new();
        for (key, shown) in placed {
            if let Some(&node) = self.anchors.get(key) {
                let pill = self.paint(node, shown);
                if let Named::Node(tile) = *key
                    && shown.opacity >= 0.5
                {
                    hits.push((pill, tile, self.numbers.get(key).cloned()));
                }
            }
        }
        self.painter
            .ctx()
            .data_mut(|d| d.insert_temp(label_hits_id(), hits));
        self.next
            .extend(leaving.into_iter().map(|(key, _, shown)| (key, shown)));
        let next = self.next;
        self.painter
            .ctx()
            .data_mut(|d| d.insert_temp(Self::id(), next));
    }

    /// A label on its pill, at its eased place beside `node`. Returns the pill.
    fn paint(&self, node: egui::Pos2, shown: &Shown) -> egui::Rect {
        let mut painter = self.painter.clone();
        painter.multiply_opacity(shown.opacity.clamp(0.0, 1.0));
        let min = egui::emath::GuiRounding::round_to_pixels(
            node + shown.offset,
            painter.pixels_per_point(),
        );
        let pill = egui::Rect::from_min_size(min, shown.galley.size()).expand2(PILL_PAD);
        // A name nudged away from its node, or pushed off by a crowd, keeps a thread
        // to it, from the node's edge: dotted and grey, never a call's color, so it
        // is not read as a call.
        let end = pill.clamp(node);
        if !shown.at.over && (shown.at.shift != 0 || end.distance(node) > shown.edge + 10.0) {
            let start = node + (end - node).normalized() * (shown.edge + 1.5);
            painter.extend(egui::Shape::dotted_line(&[start, end], THREAD, 3.0, 0.75));
        }
        painter.rect_filled(pill, 4.0, PILL);
        if let Some(color) = shown.stroke {
            painter.rect_stroke(
                pill,
                4.0,
                egui::Stroke::new(1.5, color),
                egui::StrokeKind::Inside,
            );
        }
        painter.galley(min, shown.galley.clone(), egui::Color32::WHITE);
        pill
    }
}

/// Whether a name at `rect` sits within [`NEAR`] of its node, a disc of `edge` at
/// `node`.
fn near(rect: egui::Rect, node: egui::Pos2, edge: f32) -> bool {
    rect.distance_to_pos(node) - edge <= NEAR
}

/// How much nearer, in whole points edge to edge, the nearest other of `nodes` lies
/// to `rect`, a label of `tile` (a disc of `edge` at `node`), than its own; 0 when
/// its own is nearest.
fn misread_by(
    rect: egui::Rect,
    tile: usize,
    node: egui::Pos2,
    edge: f32,
    nodes: &[(usize, egui::Pos2, f32)],
) -> u32 {
    let mine = rect.distance_to_pos(node) - edge;
    // Only nodes near enough to matter, cheaply.
    let reach = rect.expand(mine.max(0.0) + 40.0);
    let theirs = nodes
        .iter()
        .filter(|&&(other, at, radius)| other != tile && reach.expand(radius).contains(at))
        .map(|&(_, at, radius)| rect.distance_to_pos(at) - radius)
        .fold(f32::INFINITY, f32::min);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a small distance, at least 0"
    )]
    let by = (mine - theirs - 0.5).max(0.0).ceil() as u32;
    by
}

fn label_hits_id() -> egui::Id {
    egui::Id::new("graph_label_hits")
}

/// A function's name as drawn: its pill, the function, and the journey number it
/// leads with.
type LabelHit = (egui::Rect, usize, Option<String>);

/// Every function's name as last drawn: its pill and the function, bottom first.
pub(crate) fn drawn_labels(ctx: &egui::Context) -> Vec<(egui::Rect, usize)> {
    ctx.data(|d| d.get_temp::<Vec<LabelHit>>(label_hits_id()))
        .unwrap_or_default()
        .into_iter()
        .map(|(pill, tile, _)| (pill, tile))
        .collect()
}

/// The function whose name, as last drawn, lies under `at`, the topmost, and the
/// journey number the name leads with.
pub(crate) fn label_at(ctx: &egui::Context, at: egui::Pos2) -> Option<(usize, Option<String>)> {
    ctx.data(|d| d.get_temp::<Vec<LabelHit>>(label_hits_id()))?
        .iter()
        .rev()
        .find(|(pill, ..)| pill.contains(at))
        .map(|(_, tile, number)| (*tile, number.clone()))
}

/// What leads a label: text in a color, on a fill when it is a badge.
type Lead = (String, egui::Color32, Option<egui::Color32>);

/// A label in two parts: the name, then a quieter note.
fn label_job(
    lead: Option<Lead>,
    name: &str,
    note: Option<String>,
    color: egui::Color32,
    size: f32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let format = |color, size| egui::TextFormat {
        font_id: egui::FontId::proportional(size),
        color,
        ..default()
    };
    if let Some((lead, lead_color, fill)) = lead {
        let mut lead_format = format(lead_color, size);
        if let Some(fill) = fill {
            lead_format.background = fill;
        }
        job.append(&lead, 0.0, lead_format);
        job.append(" ", 0.0, format(color, size));
    }
    job.append(name, 0.0, format(color, size));
    if let Some(note) = note {
        job.append(&note, 6.0, format(MUTED, text::SMALL));
    }
    job
}

/// What one frame's names are worked out from: what the view is on, and where every
/// node that may be named shows.
struct Naming<'a> {
    explorer: &'a Explorer,
    nucleus: &'a Nucleus,
    to_screen: &'a dyn Fn(Vec3) -> Option<egui::Pos2>,
    /// The camera's up and right, in the world.
    up: Vec3,
    right: Vec3,
    /// The function drawn white: the journey's start, or browsing, the selection.
    white: Option<usize>,
    cursor: Option<usize>,
    /// The function set aside in a journey, named as the start is.
    aside: Option<usize>,
    hovered: Option<usize>,
    /// The journey's start, which is named as the start come what may.
    start: Option<usize>,
    disclosed: Arc<Disclosed>,
    /// The number of the call the cursor is on.
    cursor_number: Option<String>,
    /// By tile, where each node that may be named sits on screen, and how far its edge
    /// is from its middle as names keep off it.
    discs: Vec<Option<(egui::Pos2, f32)>>,
    /// A journey's functions level by level in the panel's order, the start left out.
    ladder: Vec<usize>,
}

impl<'a> Naming<'a> {
    fn new(
        explorer: &'a Explorer,
        to_screen: &'a dyn Fn(Vec3) -> Option<egui::Pos2>,
        (up, right): (Vec3, Vec3),
    ) -> Self {
        let nucleus = explorer.nucleus();
        let disclosed = explorer.disclosure();
        let cursor_number = explorer
            .cursor_span()
            .and_then(|span| disclosed.number(span))
            .filter(|number| !number.is_empty())
            .map(str::to_owned);
        let start = explorer.start();
        // A journey's functions are named level by level in the panel's order, the
        // start first, so a call's caller has room before the call does.
        let mut ladder: Vec<((usize, Vec<u32>), usize)> = disclosed
            .tiles()
            .iter()
            .filter(|&(&tile, _)| start != Some(tile))
            .filter_map(|(&tile, numbers)| {
                numbers
                    .iter()
                    .map(|n| disclosure::number_order(n))
                    .min()
                    .map(|first| (first, tile))
            })
            .collect();
        ladder.sort();
        let mut naming = Self {
            explorer,
            nucleus,
            to_screen,
            up,
            right,
            white: nucleus.start(),
            cursor: explorer.cursor(),
            aside: explorer.aside(),
            hovered: explorer.hovered(),
            start,
            disclosed,
            cursor_number,
            discs: Vec::new(),
            ladder: ladder.into_iter().map(|(_, tile)| tile).collect(),
        };
        naming.discs = (0..explorer.map().tiles.len())
            .map(|tile| {
                // Only the start, what the nucleus lights and what is hovered, however
                // faint.
                let named = nucleus.drawn(tile)
                    && (nucleus.lit(tile)
                        || naming.white == Some(tile)
                        || naming.hovered == Some(tile));
                let world = explorer.tile_point(tile).filter(|_| named)?;
                Some(((naming.to_screen)(world)?, naming.edge_of(world)?))
            })
            .collect();
        naming
    }

    /// How far, on screen, the edge of a node at `world` is from its middle as names
    /// keep off it: as far as the biggest a node is drawn, whatever this one is to the
    /// view, so a name keeps its place as the selection or the cursor comes and goes.
    fn edge_of(&self, world: Vec3) -> Option<f32> {
        let node = (self.to_screen)(world)?;
        let edge = (self.to_screen)(world + self.up * self.nucleus.dot_radius(Dot::Focus))
            .map_or(8.0, |p| p.distance(node));
        Some(Dot::Focus.clamp(edge))
    }

    /// Where a module is named: at the top of its sphere; browsing, a sphere cut by
    /// the window's edge is named on what of it shows.
    fn ring_top(&self, cluster: usize) -> Option<egui::Pos2> {
        let ball = self.nucleus.ball(cluster)?;
        let top = (self.to_screen)(ball.center + self.up * ball.radius);
        if top.is_some() || !self.nucleus.browsing() {
            return top;
        }
        let (middle, reach) = (ball.center, ball.radius * 0.7);
        [
            middle,
            middle + self.right * reach,
            middle - self.right * reach,
            middle - self.up * reach,
        ]
        .into_iter()
        .find_map(self.to_screen)
    }

    /// A sphere's middle on screen and how far its edge is from it.
    fn sphere_on_screen(&self, ball: Ball) -> Option<(egui::Pos2, f32)> {
        let middle = (self.to_screen)(ball.center)?;
        let edge = (self.to_screen)(ball.center + self.up * ball.radius)?;
        Some((middle, middle.distance(edge)))
    }

    /// Browsing, the module the pointer at `mouse` is over, its name shown however
    /// small: the smallest whose sphere holds the pointer.
    fn pointed_module(&self, mouse: Option<egui::Pos2>) -> Option<usize> {
        self.nucleus
            .balls()
            .filter(|_| self.nucleus.browsing())
            .filter_map(|(cluster, ball)| {
                let (middle, edge) = self.sphere_on_screen(ball)?;
                (mouse?.distance(middle) <= edge).then_some((edge, cluster))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, cluster)| cluster)
    }

    /// Every function that may be named, ranked: lower ranks are placed first and win
    /// any overlap. One rule: the start (or the selection) and the function set aside,
    /// then what the nucleus names, the journey's first calls before its callers, so
    /// the journey has room first; what is hovered comes last, so pointing never moves
    /// another name. The cursor is named where it would be anyway, so where it is
    /// never decides what else is: it is only drawn over the rest.
    fn functions(&self) -> Vec<(LabelRank, Named)> {
        let nucleus = self.nucleus;
        let rungs: HashMap<usize, usize> = self
            .start
            .into_iter()
            .chain(self.ladder.iter().copied())
            .enumerate()
            .map(|(rung, tile)| (tile, rung))
            .collect();
        let mut order = Vec::new();
        for (tile, disc) in self.discs.iter().enumerate() {
            if disc.is_none() {
                continue;
            }
            let tier = match () {
                () if self.white == Some(tile) || self.aside == Some(tile) => 0,
                () if nucleus.named(tile) && nucleus.level(tile) == Some(1) => 1,
                () if nucleus.named(tile) => 2,
                () if self.cursor == Some(tile) => 4,
                // The few calls in from outside the journey drawn for each module,
                // those nearest it, are named after all the journey's own names.
                () if nucleus.outside(tile) => 6,
                () if self.hovered == Some(tile) => POINTED,
                () => continue,
            };
            let rung = rungs.get(&tile).copied().unwrap_or(usize::MAX);
            let degree = self.explorer.graph().degree(tile);
            order.push((
                (tier, rung, std::cmp::Reverse(degree), tile),
                Named::Node(tile),
            ));
        }
        order
    }

    /// Every module that may be named, ranked as [`Self::functions`] ranks functions:
    /// the nucleus and the open modules first, and of the rest only the biggest, where
    /// there is room; browsing, every module big enough, and last the one the pointer
    /// is over.
    fn modules(
        &self,
        pointed_module: Option<usize>,
        left_out: &HashMap<usize, usize>,
    ) -> Vec<(LabelRank, Named)> {
        let nucleus = self.nucleus;
        let mut faint: Vec<(usize, usize)> = nucleus
            .balls()
            .filter(|(_, ball)| ball.body == Body::Faint)
            .map(|(cluster, _)| (nucleus.count(cluster), cluster))
            .collect();
        faint.sort_unstable_by(|a, b| b.cmp(a));
        // None is named through the glass of a sphere in front, nor so near one
        // that its name would lie over it.
        let discs: Vec<(egui::Pos2, f32)> = nucleus
            .balls()
            .filter(|(_, ball)| matches!(ball.body, Body::Nucleus | Body::Open))
            .filter_map(|(_, ball)| self.sphere_on_screen(ball))
            .collect();
        let hidden = |cluster: usize| {
            nucleus
                .ball(cluster)
                .and_then(|ball| (self.to_screen)(ball.center))
                .is_none_or(|at| discs.iter().any(|&(m, r)| at.distance(m) < r + 110.0))
        };
        // Expanded, the rest of the codebase goes unnamed: the ring says where calls go.
        // Browsing, the whole codebase is the map: every module on screen is named,
        // whatever is clicked, so the landmarks never change under a click.
        let browsing = nucleus.browsing();
        let named: HashSet<usize> = faint
            .iter()
            .filter(|_| !nucleus.expanded())
            .filter(|&&(_, cluster)| {
                if browsing {
                    self.ring_top(cluster).is_some()
                } else {
                    !hidden(cluster)
                }
            })
            .take(if browsing { usize::MAX } else { FAINT_NAMED })
            .map(|&(_, cluster)| cluster)
            .collect();
        let mut order = Vec::new();
        for (cluster, ball) in nucleus.balls() {
            let rank = match ball.body {
                // Browsing, the one the pointer is over is named last, over the rest.
                _ if browsing && pointed_module == Some(cluster) => (
                    POINTED,
                    0,
                    std::cmp::Reverse(usize::MAX),
                    usize::MAX - nucleus.count(cluster),
                ),
                // Browsing, a module is named before any function, biggest first,
                // when it shows big enough for its name to be read as its own: by
                // how far the view is zoomed in alone, so what is clicked never moves
                // a module's name or leaves one out.
                _ if browsing => {
                    if self
                        .sphere_on_screen(ball)
                        .is_none_or(|(_, edge)| edge < NAMED_SPHERE)
                    {
                        continue;
                    }
                    (
                        0,
                        0,
                        std::cmp::Reverse(usize::MAX),
                        usize::MAX - nucleus.count(cluster),
                    )
                }
                // Which module a named function lives in matters as much as its name:
                // right after the start's.
                Body::Nucleus | Body::Open => {
                    (0, 1, std::cmp::Reverse(nucleus.count(cluster)), cluster)
                }
                Body::Faint if named.contains(&cluster) || left_out.contains_key(&cluster) => {
                    (6, 0, std::cmp::Reverse(nucleus.count(cluster)), cluster)
                }
                Body::Faint | Body::Closed => continue,
            };
            order.push((rank, Named::Group(cluster)));
        }
        order
    }

    /// A placer for this frame's names, `order` the nodes to name: what names keep
    /// off, held places, and which go over the rest.
    fn placer<'p>(
        &self,
        painter: &'p egui::Painter,
        order: &[(LabelRank, Named)],
        held: HashMap<Named, Spot>,
        pointed_module: Option<usize>,
    ) -> Placer<'p> {
        let nucleus = self.nucleus;
        // Function names keep off the search box, as last laid out; module names
        // browsing do not move for it.
        let search = painter
            .ctx()
            .memory(|m| m.area_rect(egui::Id::new("search")));
        // And off the nodes that matter, so they never hide a neighbor: every lit one,
        // the journey's, its callers and those brought in from outside; what only the
        // pointer names is no reason for another name to move.
        // And off the places a journey's open modules keep free for callers from
        // outside it, so those come and go covering no name.
        let free = nucleus
            .kept_free()
            .iter()
            .filter_map(|&at| Some((usize::MAX, (self.to_screen)(at)?, self.edge_of(at)?)));
        let blocking: Vec<(usize, egui::Pos2, f32)> = (0..self.discs.len())
            .filter(|&tile| {
                nucleus.level(tile).is_some()
                    || self.cursor == Some(tile)
                    || nucleus.outside(tile)
                    || self.white == Some(tile)
            })
            .filter_map(|tile| self.discs[tile].map(|(node, edge)| (tile, node, edge)))
            .chain(free)
            .collect();
        let mut placer = Placer::new(
            painter,
            search
                .into_iter()
                .chain(blocking.iter().map(|&(_, node, edge)| {
                    egui::Rect::from_center_size(node, egui::Vec2::splat(edge * 2.0))
                }))
                .collect(),
            order
                .iter()
                .filter_map(|&(_, named)| match named {
                    Named::Node(tile) => self.discs[tile].map(|(node, edge)| (tile, node, edge)),
                    Named::Group(_) => None,
                })
                .collect(),
        );
        placer.blocking = blocking;
        placer.spheres = nucleus
            .balls()
            .filter(|(_, ball)| nucleus.shows_body(ball.body))
            .filter_map(|(cluster, ball)| {
                let (middle, edge) = self.sphere_on_screen(ball)?;
                Some((cluster, middle, edge))
            })
            .collect();
        placer.landmarks = nucleus.browsing();
        if nucleus.browsing() {
            // As big as each would be drawn lit.
            let speck_radius = nucleus.dot_radius(Dot::Lit);
            placer.specks = (0..self.discs.len())
                .filter(|&tile| nucleus.speck(tile))
                .filter_map(|tile| {
                    let world = self.explorer.tile_point(tile)?;
                    let at = (self.to_screen)(world)?;
                    let edge = (self.to_screen)(world + self.up * speck_radius)
                        .map_or(1.0, |p| p.distance(at));
                    Some((tile, at, Dot::Lit.clamp(edge)))
                })
                .collect();
        }
        placer.held = held;
        placer.beside = [self.cursor, self.white, self.aside]
            .into_iter()
            .flatten()
            .map(Named::Node)
            .collect();
        // The selection's and the cursor's names, and the one the pointer is on, read
        // over the rest.
        placer.top = [self.white, self.aside, self.hovered, self.cursor]
            .into_iter()
            .flatten()
            .map(Named::Node)
            .collect();
        if let Some(cluster) = pointed_module {
            placer.top.insert(0, Named::Group(cluster));
        }
        placer
    }

    /// A module's name as drawn, `tier` its rank's, and how hard it is placed. An open
    /// module says how much of it is named, and how many callers from outside the
    /// journey it holds beyond those drawn; browsing, never, so neither a click nor
    /// the pointer makes a name longer and moves it. The start's callers folded into
    /// a count are counted apart from what it names.
    fn module_label(
        &self,
        group: usize,
        tier: u8,
        left_out: &HashMap<usize, usize>,
    ) -> (egui::text::LayoutJob, Force) {
        let nucleus = self.nucleus;
        let browsing = nucleus.browsing();
        let body = nucleus.ball(group).map(|b| b.body);
        let color = egui_color(module_color(group)).gamma_multiply(match body {
            Some(Body::Faint) => 0.5,
            _ => 1.0,
        });
        let label = self.explorer.cluster_name(group);
        let folded = nucleus.folded_in(group);
        let named_note = (body == Some(Body::Open) && nucleus.opened(group) > folded && !browsing)
            .then(|| {
                format!(
                    "{} of {}",
                    nucleus.opened(group) - folded,
                    nucleus.count(group)
                )
            });
        let plural = |count: usize| if count == 1 { "" } else { "s" };
        let folded_note = (folded > 0).then(|| format!("+{folded} caller{}", plural(folded)));
        let more = left_out.get(&group).copied().unwrap_or(0);
        let more_note = (more > 0).then(|| format!("+{more} more caller{}", plural(more)));
        let note = [named_note, folded_note, more_note]
            .into_iter()
            .flatten()
            .reduce(|a, b| format!("{a} · {b}"));
        let job = label_job(None, &label.to_uppercase(), note, color, text::TITLE);
        // Expanded, the calls drawn end at the modules on the ring. Browsing, every
        // module big enough is a landmark, kept off the others' names; the one the
        // pointer is over is named come what may.
        let force = if (browsing && tier == POINTED)
            || body == Some(Body::Nucleus)
            || (body == Some(Body::Open) && nucleus.expanded())
        {
            Force::Always
        } else if more > 0 || folded > 0 || browsing {
            Force::Free
        } else {
            Force::Never
        };
        (job, force)
    }

    /// A function's name as drawn, `tier` its rank's, `homonym` whether another named
    /// function has its name: the text, its pill's border, how hard it is placed, and
    /// the journey number a click on it goes to.
    fn function_label(
        &self,
        tile: usize,
        tier: u8,
        homonym: bool,
    ) -> (
        egui::text::LayoutJob,
        Option<egui::Color32>,
        Force,
        Option<String>,
    ) {
        let (explorer, nucleus) = (self.explorer, self.nucleus);
        let (cursor, start) = (self.cursor, self.start);
        let name = match explorer.tile_unit(tile) {
            Some(unit) if homonym => explorer.unit_name(unit),
            _ => node_name(explorer, tile),
        };
        // Colored as its call is drawn; a faint speck named only when hovered, quietly.
        let level = nucleus.level(tile);
        let quiet = nucleus.speck(tile) || (nucleus.outside(tile) && self.aside != Some(tile));
        let color = match tier {
            _ if cursor == Some(tile) => CURSOR_TEXT,
            0 => egui::Color32::WHITE,
            _ if quiet => MUTED,
            _ => match level {
                Some(-1) => CALLER_TEXT,
                Some(1) => CALLEE_TEXT,
                Some(2..) => DEEPER_TEXT,
                _ => egui::Color32::from_rgb(200, 205, 216),
            },
        };
        let panel = self
            .disclosed
            .tiles()
            .get(&tile)
            .map_or(&[][..], Vec::as_slice);
        // A start the journey calls again says so, after saying it is the start, so
        // the call back is not read as its own.
        let again = (start == Some(tile))
            .then(|| nucleus::badge(panel, None))
            .flatten()
            .map(|again| format!("start · again at {again}"));
        // How the journey runs it, after anything else said of it. When it runs in the
        // panel's words; where its runs differ, only that it does not always.
        let disclosed = &self.disclosed;
        let when = disclosed.tile_sometimes(tile).then(|| {
            disclosed
                .tile_when(tile)
                .map_or("sometimes".to_owned(), |w| short(w, 26))
        });
        let marks: Vec<String> = when
            .into_iter()
            .chain(disclosed.tile_fails(tile).then(|| "can fail".to_owned()))
            .collect();
        let said = [again, (!marks.is_empty()).then(|| marks.join(" · "))]
            .into_iter()
            .flatten()
            .reduce(|a, b| format!("{a} · {b}"));
        // A node the journey runs leads with its place in it, as its bar does: one
        // number for the journey, wherever the cursor is, its first call in the
        // panel's order, and how many more it runs; but the cursor's own shows the call
        // the cursor is on, as the panel does. The start is the start, however often
        // the journey comes back to it.
        let order = if start == Some(tile) {
            None
        } else {
            let at = self
                .cursor_number
                .as_deref()
                .filter(|_| cursor == Some(tile));
            nucleus::badge(panel, at)
        };
        // A click on the name goes to the call it leads with; on the start's, to the
        // start.
        let number = if start == Some(tile) {
            Some(String::new())
        } else {
            order
                .as_deref()
                .and_then(|o| o.split_whitespace().next())
                .map(|first| first.trim_end_matches(',').to_owned())
        };
        // The number on a badge filled like the node and its bar, so the two read as
        // one.
        let fill = egui_color(graph_view::node_color(explorer, tile));
        let lead = order.map(|order| (format!(" {order} "), ON_FILL, Some(fill)));
        // The pill's border says which way it relates to the start, as its call does;
        // the cursor's is its own color.
        let stroke = match level {
            _ if cursor == Some(tile) => Some(egui_color(CURSOR)),
            _ if tier == 0 => Some(egui::Color32::from_white_alpha(140)),
            _ if quiet => None,
            Some(-1) => Some(egui_color(CALLER)),
            Some(1) => Some(egui_color(CALLEE)),
            Some(2..) => Some(egui_color(DEEPER)),
            _ => None,
        };
        // One size for every function's name, so a click that makes one the selection
        // or the cursor never moves it.
        let job = label_job(lead, &name, said, color, text::BODY);
        // The start and what is hovered are named come what may; everything the
        // nucleus names, wherever there is room.
        let force = if tier == 0 || self.hovered == Some(tile) {
            Force::Always
        } else if tier <= 2 || nucleus.outside(tile) {
            Force::Free
        } else {
            Force::Never
        };
        (job, stroke, force, number)
    }

    /// Takes back the name of every journey function whose caller went unnamed: the
    /// panel's tree reads on the graph from the start down, never a call without its
    /// caller.
    fn unname_orphans(&self, placer: &mut Placer) {
        let journey = self.disclosed.tiles();
        let numbered: HashMap<&str, usize> = journey
            .iter()
            .flat_map(|(&tile, numbers)| numbers.iter().map(move |n| (n.as_str(), tile)))
            .collect();
        let mut named_calls: HashSet<usize> = self.start.into_iter().collect();
        for &tile in &self.ladder {
            let key = Named::Node(tile);
            if !placer.next.contains_key(&key) {
                continue;
            }
            let under_named = journey.get(&tile).is_some_and(|numbers| {
                numbers.iter().any(|n| match n.rsplit_once('.') {
                    None => true,
                    Some((caller, _)) => numbered
                        .get(caller)
                        .is_some_and(|caller| named_calls.contains(caller)),
                })
            });
            // The cursor's name shows wherever it is, but names no calls under it that
            // would not be named without it.
            if !under_named && !placer.beside.contains(&key) && self.hovered != Some(tile) {
                placer.unplace(key);
            } else if under_named && !placer.overlaid.contains(&key) {
                named_calls.insert(tile);
            }
        }
    }

    /// The call in of an outside caller, or of a caller of the start folded into a
    /// count, the pointer rests on, or of a function clicked off the journey, is what
    /// is being traced: drawn again over the names, so none hides where it lands.
    fn trace(&self, painter: &egui::Painter) {
        let nucleus = self.nucleus;
        if nucleus.browsing() {
            return;
        }
        let traced = nucleus.links().iter().filter(|link| {
            let pointed = Some(link.from) == self.hovered
                && (link.flow == Flow::Outside || nucleus.folded(link.from));
            pointed || (self.aside.is_some() && Some(link.from) == self.aside)
        });
        for link in traced {
            let route: Vec<egui::Pos2> = nucleus
                .route(link)
                .into_iter()
                .filter_map(self.to_screen)
                .collect();
            let edge = self
                .discs
                .get(link.to)
                .copied()
                .flatten()
                .map_or(0.0, |(_, e)| e);
            traced_call(painter, &route, edge);
        }
    }
}

/// The functions of `order` that share their name with another of them: they are told
/// apart by their types, `Explorer.root`, `Track.root`.
fn homonyms(explorer: &Explorer, order: &[(LabelRank, Named)]) -> HashSet<usize> {
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for &(_, named) in order {
        if let Named::Node(tile) = named {
            by_name
                .entry(node_name(explorer, tile))
                .or_default()
                .push(tile);
        }
    }
    by_name
        .into_values()
        .filter(|tiles| tiles.len() > 1)
        .flatten()
        .collect()
}

/// Names beside graph nodes, most important first, each on a dark pill so it reads
/// over the arrows. A journey's functions lead with their numbers in it. A name tries
/// each side of its node and is left out when none is free, and never covers a node
/// that matters; zooming in makes room for more. Module names fill whatever space is
/// left.
pub(crate) fn draw_graph_labels(
    painter: &egui::Painter,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
    explorer: &Explorer,
    up_right: (Vec3, Vec3),
    settled: bool,
) {
    let naming = Naming::new(explorer, to_screen, up_right);
    let nucleus = naming.nucleus;
    let left_out = nucleus.outside_left_out(explorer.graph());
    let mouse = painter.ctx().input(|i| i.pointer.hover_pos());
    let pointed_module = naming.pointed_module(mouse);
    let mut order = naming.functions();
    order.extend(naming.modules(pointed_module, &left_out));
    // A label keeps the place it took when it first showed for as long as the scene
    // lasts, and is placed before any new one, which keeps off it.
    let scene = if nucleus.browsing() {
        Scene::Browse
    } else {
        Scene::Journey {
            start: naming.start,
            expanded: nucleus.expanded(),
        }
    };
    let held = painter
        .ctx()
        .data_mut(|d| {
            d.get_temp_mut_or_default::<SpotBook>(spot_book_id())
                .get(&scene)
                .cloned()
        })
        .unwrap_or_default();
    order.sort_by_key(|&(key, named)| (!held.contains_key(&named), key));
    let homonyms = homonyms(explorer, &order);

    let held_keys: HashSet<Named> = held.keys().copied().collect();
    let mut placer = naming.placer(painter, &order, held, pointed_module);
    placer.keep = settled;
    let mut flushed = false;
    for ((tier, ..), named) in order {
        // Every held name that has room is placed; then those that have none, for
        // now; then the new ones.
        if !flushed && !held_keys.contains(&named) {
            placer.place_displaced();
            flushed = true;
        }
        match named {
            Named::Group(group) => {
                let Some(at) = naming.ring_top(group) else {
                    continue;
                };
                let (job, force) = naming.module_label(group, tier, &left_out);
                placer.place(named, at, 0.0, job, None, force);
            }
            Named::Node(tile) => {
                let Some((node, edge)) = naming.discs[tile] else {
                    continue;
                };
                let (job, stroke, force, number) =
                    naming.function_label(tile, tier, homonyms.contains(&tile));
                if let Some(number) = number {
                    placer.numbers.insert(named, number);
                }
                placer.place(named, node, edge, job, stroke, force);
            }
        }
    }
    placer.place_displaced();
    naming.unname_orphans(&mut placer);
    // Every function lit and on screen is named, or counted here as not.
    let lit: Vec<usize> = (0..naming.discs.len())
        .filter(|&tile| {
            naming.discs[tile].is_some() && nucleus.level(tile).is_some() && !nucleus.folded(tile)
        })
        .collect();
    let unnamed = lit
        .iter()
        .filter(|&&tile| !placer.next.contains_key(&Named::Node(tile)))
        .count();
    let chosen = std::mem::take(&mut placer.chosen);
    placer.finish(|named| match named {
        Named::Node(tile) => naming
            .discs
            .get(tile)
            .copied()
            .flatten()
            .map(|(node, _)| node),
        Named::Group(group) => naming.ring_top(group),
    });
    if !chosen.is_empty() {
        painter.ctx().data_mut(|d| {
            let book = d.get_temp_mut_or_default::<SpotBook>(spot_book_id());
            if book.len() >= SCENES_KEPT && !book.contains_key(&scene) {
                book.clear();
            }
            book.entry(scene).or_default().extend(chosen);
        });
    }
    naming.trace(painter);
    if unnamed > 0 {
        unnamed_note(painter, unnamed, lit.len());
    }
}

/// A call being traced, along `route` on screen, its head on the edge of the
/// function it calls, `edge` from that function's middle: white, over the names.
fn traced_call(painter: &egui::Painter, route: &[egui::Pos2], edge: f32) {
    let [.., before, last] = route else {
        return;
    };
    let along = (*last - *before).normalized();
    let tip = *last - along * (edge + 1.0);
    let ink = egui::Color32::from_white_alpha(235);
    let mut points = route[..route.len() - 1].to_vec();
    points.push(tip - along * 4.0);
    painter.add(egui::Shape::line(points, egui::Stroke::new(1.6, ink)));
    let across = egui::vec2(-along.y, along.x);
    let back = tip - along * 8.0;
    painter.add(egui::Shape::convex_polygon(
        vec![tip, back + across * 4.0, back - across * 4.0],
        ink,
        egui::Stroke::NONE,
    ));
}

/// Browsing, how far a sphere's edge must show from its middle, in points, for the
/// module to be named without the pointer on it.
const NAMED_SPHERE: f32 = 13.0;

/// How many of the functions lit had no room for their names, at the graph's foot
/// above the hint at the keys.
fn unnamed_note(painter: &egui::Painter, unnamed: usize, lit: usize) {
    let area = painter.clip_rect();
    let note = format!(
        "{unnamed} of {lit} lit functions unnamed for want of room · zoom in, or point at one"
    );
    let galley = painter.layout_no_wrap(note, egui::FontId::proportional(text::SMALL), MUTED);
    let at = egui::pos2(area.min.x + 12.0, area.max.y - 30.0 - galley.size().y);
    painter.rect_filled(
        egui::Rect::from_min_size(at, galley.size()).expand2(PILL_PAD),
        4.0,
        PILL,
    );
    painter.galley(at, galley, MUTED);
}

/// In a journey, how many of the modules holding only callers from outside it are
/// named.
const FAINT_NAMED: usize = 16;

/// The tier of a name only the pointer asks for:
/// placed after every other, so it never moves one.
const POINTED: u8 = 7;

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs one frame that places a label for `key` at `node` with `blocked` taken,
    /// and returns the side it took and where its text was drawn from the node.
    fn frame(ctx: &egui::Context, node: egui::Pos2, blocked: &[egui::Rect]) -> (usize, egui::Vec2) {
        let shown = placed(ctx, node, blocked, false);
        (shown.at.side, shown.offset)
    }

    /// Runs one frame that places a label for `key` at `node` with `blocked` taken,
    /// forced or not, and returns it as drawn.
    fn placed(ctx: &egui::Context, node: egui::Pos2, blocked: &[egui::Rect], force: bool) -> Shown {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx();
            let painter = ctx.layer_painter(egui::LayerId::background());
            let mut placer = Placer::new(&painter, blocked.to_vec(), Vec::new());
            let job = label_job(None, "draw_graph", None, egui::Color32::WHITE, 12.5);
            let force = if force { Force::Always } else { Force::Never };
            assert!(placer.place(Named::Node(0), node, 5.0, job, None, force));
            placer.finish(|_| Some(node));
        });
        output.textures_delta.clear();
        ctx.data_mut(|d| d.get_temp::<ShownLabels>(Placer::id()))
            .and_then(|labels| labels.get(&Named::Node(0)).cloned())
            .unwrap_or_else(|| panic!("the label was not remembered"))
    }

    #[test]
    fn a_name_that_must_show_is_nudged_aside_not_dropped() {
        let ctx = egui::Context::default();
        let node = egui::pos2(400.0, 300.0);
        // Every side of the node taken, but free further up and down.
        let crowd = egui::Rect::from_center_size(node, egui::vec2(500.0, 44.0));
        let shown = placed(&ctx, node, &[crowd], true);
        assert_ne!(shown.at.shift, 0, "nudged along a side");
        let text = egui::Rect::from_min_size(node + shown.offset, shown.galley.size());
        assert!(!text.intersects(crowd), "clear of what was there: {text:?}");
        assert!(
            text.center().distance(node) < 80.0,
            "and still near its node: {text:?}"
        );
    }

    /// Runs one frame of `place` calls on a fresh placer set up by `setup`, and
    /// returns the labels as drawn and the placer's rooms taken.
    fn placing(
        setup: impl Fn(&mut Placer),
        labels: &[(Named, egui::Pos2, Force)],
    ) -> (ShownLabels, HashSet<Named>, usize) {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let mut overlaid = HashSet::new();
        let mut taken = 0;
        let mut output = ctx.run_ui(input, |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::background());
            let mut placer = Placer::new(&painter, Vec::new(), Vec::new());
            setup(&mut placer);
            for &(key, node, force) in labels {
                let job = label_job(None, "draw_graph", None, egui::Color32::WHITE, 12.5);
                placer.place(key, node, 5.0, job, None, force);
            }
            placer.place_displaced();
            overlaid.clone_from(&placer.overlaid);
            taken = placer.taken.len();
            placer.finish(|_| Some(egui::Pos2::ZERO));
        });
        output.textures_delta.clear();
        let shown = ctx
            .data_mut(|d| d.get_temp::<ShownLabels>(Placer::id()))
            .unwrap_or_default();
        (shown, overlaid, taken)
    }

    #[test]
    fn a_held_name_stays_put_and_new_ones_keep_off_it() {
        let (a, b) = (egui::pos2(400.0, 300.0), egui::pos2(400.0, 318.0));
        let left = Spot::beside(1, 0);
        let (shown, ..) = placing(
            |placer| {
                placer.held.insert(Named::Node(0), left);
            },
            &[
                (Named::Node(0), a, Force::Free),
                (Named::Node(1), b, Force::Free),
            ],
        );
        assert_eq!(shown[&Named::Node(0)].at, left, "where it first went");
        let rect = |key: Named, node: egui::Pos2| {
            let label = &shown[&key];
            egui::Rect::from_min_size(node + label.offset, label.galley.size())
        };
        assert!(
            !rect(Named::Node(1), b).intersects(rect(Named::Node(0), a)),
            "the new name keeps off the held one"
        );
    }

    #[test]
    fn a_name_held_far_from_its_node_takes_a_nearer_place() {
        let node = egui::pos2(400.0, 300.0);
        let far = Spot::beside(0, 6);
        let (shown, ..) = placing(
            |placer| {
                placer.held.insert(Named::Node(0), far);
            },
            &[(Named::Node(0), node, Force::Free)],
        );
        let at = shown[&Named::Node(0)].at;
        assert_eq!(at.shift, 0, "right beside its node: {at:?}");
    }

    #[test]
    fn a_held_name_over_a_node_lit_since_goes_unnamed_rather_than_elsewhere() {
        let (a, b) = (egui::pos2(400.0, 300.0), egui::pos2(440.0, 300.0));
        // Held to the right of `a`, where `b` has since lit up.
        let right = Spot::beside(0, 0);
        let (shown, ..) = placing(
            |placer| {
                placer.held.insert(Named::Node(0), right);
                placer.blocking = vec![(0, a, 5.0), (1, b, 5.0)];
            },
            &[(Named::Node(0), a, Force::Free)],
        );
        assert!(
            !shown.contains_key(&Named::Node(0)),
            "neither over the lit node nor moved"
        );
    }

    #[test]
    fn a_name_in_a_crowd_of_lit_nodes_goes_unnamed_rather_than_over_one() {
        let node = egui::pos2(400.0, 300.0);
        // Lit nodes all round, near enough that every place beside it lies over one.
        let mut crowd = vec![(0, node, 5.0)];
        for x in -8_i8..=8 {
            for y in -8_i8..=8 {
                if (x, y) != (0, 0) {
                    let at = node + egui::vec2(f32::from(x) * 12.0, f32::from(y) * 9.0);
                    crowd.push((crowd.len(), at, 4.0));
                }
            }
        }
        let (shown, ..) = placing(
            |placer| placer.blocking.clone_from(&crowd),
            &[(Named::Node(0), node, Force::Free)],
        );
        if let Some(label) = shown.get(&Named::Node(0)) {
            let rect = egui::Rect::from_min_size(node + label.offset, label.galley.size());
            let pill = rect.expand2(PILL_PAD);
            assert!(
                crowd[1..]
                    .iter()
                    .all(|&(_, at, r)| pill.distance_to_pos(at) >= r * 0.5),
                "over a lit node: {rect:?}"
            );
        }
    }

    #[test]
    fn the_cursor_keeps_the_place_its_name_holds() {
        let node = egui::pos2(400.0, 300.0);
        let nudged = Spot::beside(1, 1);
        let (shown, overlaid, _) = placing(
            |placer| {
                placer.held.insert(Named::Node(0), nudged);
                placer.beside.insert(Named::Node(0));
            },
            &[(Named::Node(0), node, Force::Free)],
        );
        assert_eq!(shown[&Named::Node(0)].at, nudged, "no jump to its node");
        assert!(!overlaid.contains(&Named::Node(0)));
    }

    #[test]
    fn the_cursor_is_named_beside_its_node_without_taking_room() {
        let node = egui::pos2(400.0, 300.0);
        // A name already on every side of the cursor's node.
        let crowd = |placer: &mut Placer| {
            for side in 0..SIDES.len() {
                let rect = Spot::beside(side, 0).rect(node, 5.0, egui::vec2(70.0, 14.0));
                placer.taken.push(rect);
            }
            placer.beside.insert(Named::Node(0));
        };
        let (shown, overlaid, taken) = placing(crowd, &[(Named::Node(0), node, Force::Never)]);
        assert_eq!(shown[&Named::Node(0)].at.shift, 0, "right beside its node");
        assert!(overlaid.contains(&Named::Node(0)), "drawn over the others");
        assert_eq!(taken, SIDES.len(), "and taking no room from them");
    }

    #[test]
    fn a_name_never_sits_nearer_another_node_than_its_own() {
        let node = egui::pos2(400.0, 300.0);
        let nodes = [(0, node, 5.0), (1, egui::pos2(520.0, 300.0), 5.0)];
        // To the right of its node, touching it: its own is nearest.
        let beside = egui::Rect::from_min_size(egui::pos2(411.0, 292.0), egui::vec2(50.0, 16.0));
        assert_eq!(misread_by(beside, 0, node, 5.0, &nodes), 0);
        // Nudged down and across, it lies nearer the other.
        let nudged = egui::Rect::from_min_size(egui::pos2(440.0, 310.0), egui::vec2(50.0, 16.0));
        assert!(misread_by(nudged, 0, node, 5.0, &nodes) > 0);
        assert_eq!(misread_by(nudged, 1, nodes[1].1, 5.0, &nodes), 0);
    }

    #[test]
    fn labels_hold_their_side_and_ease_to_a_new_one() {
        let ctx = egui::Context::default();
        let node = egui::pos2(400.0, 300.0);
        let right = egui::Rect::from_min_max(egui::pos2(405.0, 280.0), egui::pos2(600.0, 320.0));
        let left = egui::Rect::from_min_max(egui::pos2(200.0, 280.0), egui::pos2(395.0, 320.0));

        let (side, start) = frame(&ctx, node, &[right]);
        assert_eq!(side, 1, "the right is taken, so it goes left");
        let (side, offset) = frame(&ctx, node, &[]);
        assert_eq!(side, 1, "the right freeing up does not flip it back");
        assert_eq!(offset, start);

        let (side, offset) = frame(&ctx, node, &[left]);
        assert_eq!(side, 0, "covered, it moves");
        assert!(
            offset.x > start.x && offset.x < 0.0,
            "it slides over rather than jumping: {start:?} → {offset:?}"
        );
        let mut settled = offset;
        for _ in 0..60 {
            settled = frame(&ctx, node, &[left]).1;
        }
        assert!(settled.x > 0.0, "and gets there: {settled:?}");
    }
}
