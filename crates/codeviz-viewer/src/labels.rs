//! Names on the graph: who is named and in what order, what each name says and how
//! it looks, and where it goes. Every function and module is named where its name
//! fits, placed afresh each frame, greedily, in one fixed order of importance: the
//! start, the cursor, the selection, the modules, the journey's numbered calls level
//! by level in number order, the callers, whatever else is lit, then the rest. A name
//! lower in that order never displaces a higher one, and the same view always shows
//! the same names in the same places, whatever came before. A name stays where it
//! was or goes, never moves: it keeps the side it takes in a neutral view, or else
//! a home its dot's neighbours alone decide, and dots keep names off as though each
//! were the cursor's. In a journey a module
//! round the nucleus keeps the room for its name at every depth, shown or not, and
//! what I brings in comes after everything else lit, so neither a depth nor I moves a
//! name. What the pointer is on is named over the rest, taking no room, so pointing
//! never moves or hides another name.

use std::{cell::RefCell, collections::HashMap, sync::Arc};

use bevy::prelude::*;
use bevy_egui::egui;

use crate::{
    colors::{
        CALLEE, CALLEE_TEXT, CALLER, CALLER_TEXT, CURSOR, CURSOR_TEXT, DEEPER, DEEPER_TEXT,
        FADED_TEXT, MUTED, ON_FILL, PILL, PLAIN_TEXT, egui_color, module_color,
    },
    disclosure::{self, Disclosed},
    explorer::Explorer,
    graph_view, key_help,
    nucleus::{self, Body, Nucleus},
    sizes::Dot,
    text,
    ui::short,
};

/// Room kept around a label's text: its pill.
pub(crate) const PILL_PAD: egui::Vec2 = egui::vec2(5.0, 2.0);
/// Between a dot's edge and its name, in points.
const GAP: f32 = 4.0;
/// Between a module's sphere and its name above it, in points.
const ABOVE: f32 = 2.0;
/// Kept clear round a name: no other name or dot comes nearer, in points.
const CLEAR: f32 = 1.0;
/// How much nearer than its own another dot may lie to a name, in points: a name
/// level with its own dot still reads as its own beside a dot as near above or below.
/// Half a point under labels-near's, for where the text lands once rounded to pixels.
const NEARER: f32 = 1.5;
/// The least radius a sphere shows with, and the least room each of its dots has
/// across, for its plain functions to be named, in points.
const PLAIN_SPHERE: f32 = 40.0;
const PLAIN_APART: f32 = 12.0;
/// How far the note on names left out sits above the hint at the keys, in points.
const NOTE_ABOVE: f32 = 20.0;
/// A character's width against its font size, to see whether a plain name could fit
/// before laying it out.
const CHAR_WIDTH: f32 = 0.5;

/// A function's name as the graph and the journey panel both show it:
/// [`Explorer::function_name`]'s.
pub(crate) fn node_name(explorer: &Explorer, tile: usize) -> String {
    explorer
        .tile_unit(tile)
        .map(|unit| explorer.function_name(unit))
        .unwrap_or_default()
}

/// What a graph label names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Named {
    Node(usize),
    Group(usize),
}

/// Where a function's name goes round its dot, in order of preference: right, left,
/// the four corners, above, below; each as how far its pill's middle sits from the
/// dot's middle, in the dot's reach (its edge and the gap) and in the pill's halves.
const SPOTS: [((f32, f32), (f32, f32)); 8] = [
    ((1.0, 0.0), (1.0, 0.0)),
    ((-1.0, 0.0), (-1.0, 0.0)),
    ((0.7, -0.7), (1.0, -1.0)),
    ((0.7, 0.7), (1.0, 1.0)),
    ((-0.7, -0.7), (-1.0, -1.0)),
    ((-0.7, 0.7), (-1.0, 1.0)),
    ((0.0, -1.0), (0.0, -1.0)),
    ((0.0, 1.0), (0.0, 1.0)),
];

/// How far a lit name with no spot of [`SPOTS`] free, nudged or not, slides along
/// its side, in its height beside its dot or at a corner, or in halves of its width
/// above or below.
const SLIDES: [f32; 2] = [1.0, -1.0];

/// How far a lit name is nudged along its side when its spot is just taken, in its
/// height (or half its width, above or below).
const NUDGE: f32 = 0.5;

/// Spot `index` slid `by` along its side: beside its dot or at a corner, by `by` of
/// its height; above or below, by `by` of half its width.
fn slid(index: usize, node: egui::Pos2, reach: f32, size: egui::Vec2, by: f32) -> egui::Rect {
    let ((rx, _), _) = SPOTS[index];
    let along = if rx == 0.0 {
        egui::vec2(by * size.x / 2.0, 0.0)
    } else {
        egui::vec2(0.0, by * size.y)
    };
    spot(index, node, reach, size).translate(along)
}

/// A pill of `size` at spot `index` round a dot at `node`, its name kept `reach`
/// from the dot's middle.
fn spot(index: usize, node: egui::Pos2, reach: f32, size: egui::Vec2) -> egui::Rect {
    let ((rx, ry), (hx, hy)) = SPOTS[index];
    let middle = node
        + egui::vec2(
            rx * reach + hx * size.x / 2.0,
            ry * reach + hy * size.y / 2.0,
        );
    egui::Rect::from_center_size(middle, size)
}

/// Where a module's name goes: centred just above its sphere, a disc of `radius` at
/// `middle`.
fn above(middle: egui::Pos2, radius: f32, size: egui::Vec2) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(middle.x - size.x / 2.0, middle.y - radius - ABOVE - size.y),
        size,
    )
}

/// A dot on screen, as names keep off it.
#[derive(Debug, Clone, Copy)]
struct Disc {
    tile: usize,
    at: egui::Pos2,
    /// As names keep off it: a dot drawn bigger than a speck as big as the cursor's,
    /// whatever it is to the view, so a name stays put as the cursor or the selection
    /// comes and goes beside it.
    radius: f32,
    /// As big as the cursor's: the same whatever is selected, for the order a name
    /// tries its spots in.
    steady: f32,
    /// As small as it is drawn lit, or as a speck: how near its own name reads as
    /// its, whatever it comes to be, so no other dot ever reads as nearer.
    least: f32,
    /// Lit, the start or the cursor: no name ever covers it. An expanded module's
    /// functions that nothing else lights are not: a lit name may lie over them.
    lit: bool,
    /// A caller from off the journey (I): only names of what I brings in keep off it,
    /// so I moves no other name.
    outside: bool,
    /// Lit, or lit at another depth or by another selection: a name would rather not
    /// cover it, so that one more lit takes no name's side.
    may_light: bool,
}

/// Rectangles and dots by where they are on screen, so a name looks only at what is
/// near it.
#[derive(Default)]
struct Grid {
    cells: HashMap<(i32, i32), Vec<usize>>,
}

impl Grid {
    const CELL: f32 = 64.0;

    #[expect(clippy::cast_possible_truncation, reason = "a cell index")]
    fn cells(rect: egui::Rect) -> impl Iterator<Item = (i32, i32)> {
        let lo = (rect.min / Self::CELL).floor();
        let hi = (rect.max / Self::CELL).floor();
        (lo.x as i32..=hi.x as i32)
            .flat_map(move |x| (lo.y as i32..=hi.y as i32).map(move |y| (x, y)))
    }

    fn insert(&mut self, rect: egui::Rect, index: usize) {
        for cell in Self::cells(rect) {
            self.cells.entry(cell).or_default().push(index);
        }
    }

    /// What may lie within `rect`, each once.
    fn near(&self, rect: egui::Rect) -> Vec<usize> {
        let mut found: Vec<usize> = Self::cells(rect)
            .filter_map(|cell| self.cells.get(&cell))
            .flatten()
            .copied()
            .collect();
        found.sort_unstable();
        found.dedup();
        found
    }
}

/// How strictly a function's name keeps off dots: off every one, or, for a lit name
/// with nowhere else to go, only off the lit ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Keep {
    EveryDot,
    LitDots,
}

/// Where names have gone this frame, and what they keep off.
struct Placer {
    bounds: egui::Rect,
    /// Names placed, and what was there before any: the search box.
    taken: Vec<egui::Rect>,
    taken_grid: Grid,
    dots: Vec<Disc>,
    dot_grid: Grid,
    /// Every sphere shown: its cluster, middle and radius on screen.
    spheres: Vec<(usize, egui::Pos2, f32)>,
    /// Whether the names placed now are of what I brings in, which keep off its dots.
    outside: bool,
}

impl Placer {
    fn new(bounds: egui::Rect, dots: Vec<Disc>, spheres: Vec<(usize, egui::Pos2, f32)>) -> Self {
        let mut dot_grid = Grid::default();
        for (index, dot) in dots.iter().enumerate() {
            dot_grid.insert(
                egui::Rect::from_center_size(dot.at, egui::Vec2::splat(2.0 * dot.radius)),
                index,
            );
        }
        Self {
            bounds,
            taken: Vec::new(),
            taken_grid: Grid::default(),
            dots,
            dot_grid,
            spheres,
            outside: false,
        }
    }

    /// Whether names placed now keep off `dot`.
    fn sees(&self, dot: &Disc) -> bool {
        self.outside || !dot.outside
    }

    fn take(&mut self, rect: egui::Rect) {
        self.taken_grid.insert(rect, self.taken.len());
        self.taken.push(rect);
    }

    /// Whether `pill` lies over any name placed.
    fn over_names(&self, pill: egui::Rect) -> usize {
        let pill = pill.expand(CLEAR);
        self.taken_grid
            .near(pill)
            .into_iter()
            .filter(|&i| self.taken[i].intersects(pill))
            .count()
    }

    /// The dots other than `own` that a name at `pill` would cover or lie nearer than
    /// its own, as `keep` counts them.
    fn over_dots(&self, pill: egui::Rect, own: usize, keep: Keep) -> usize {
        // Which dot a name reads as nearest is judged from its text, the pill's
        // border aside.
        let text = pill.shrink2(PILL_PAD);
        let mine = self
            .dots
            .iter()
            .find(|d| d.tile == own)
            .map_or(0.0, |d| text.distance_to_pos(d.at) - d.least);
        self.dot_grid
            .near(pill.expand(mine.max(0.0) + CLEAR))
            .into_iter()
            .map(|i| self.dots[i])
            .filter(|d| self.sees(d) && d.tile != own && (keep == Keep::EveryDot || d.lit))
            .filter(|d| {
                pill.distance_to_pos(d.at) - d.radius < CLEAR
                    || text.distance_to_pos(d.at) - d.radius < mine - NEARER
            })
            .count()
    }

    /// Whether a function's name at `pill` fits: on the graph, over no name, and
    /// nearer no other dot than its own.
    fn fits(&self, pill: egui::Rect, own: usize, keep: Keep) -> bool {
        self.bounds.contains_rect(pill)
            && self.over_names(pill) == 0
            && self.over_dots(pill, own, keep) == 0
    }

    /// The first spot round a dot at `node` where a pill of `size` fits, `reach` from
    /// its middle, off every dot, else for a lit one off the lit ones.
    fn find(
        &self,
        own: usize,
        node: egui::Pos2,
        reach: f32,
        size: egui::Vec2,
        lit: bool,
    ) -> Option<(usize, egui::Rect)> {
        let keeps: &[Keep] = if lit {
            &[Keep::EveryDot, Keep::LitDots]
        } else {
            &[Keep::EveryDot]
        };
        keeps.iter().find_map(|&keep| {
            (0..SPOTS.len())
                .map(|i| (i, spot(i, node, reach, size)))
                .find(|&(_, pill)| self.fits(pill, own, keep))
        })
    }

    /// The first spot round a dot at `node`, and how far it is nudged along its side,
    /// where a pill of `size` fits as names keep
    /// off lit dots, taking the spots in order of how few dots a pill of `bare`, the
    /// name alone, would cover, every dot as big as it shows lit, whatever is selected:
    /// the order hangs only on where the dots are, not on what is lit or the numbers a
    /// depth gives the name, so those move a name only where they take its spot.
    fn find_by_dots(
        &self,
        own: usize,
        node: egui::Pos2,
        reach: f32,
        (size, bare): (egui::Vec2, egui::Vec2),
        keep: Keep,
    ) -> Option<(usize, f32)> {
        let mut spots: Vec<((usize, usize), usize)> = (0..SPOTS.len())
            .map(|i| (self.covered(own, reach, spot(i, node, reach, bare)), i))
            .collect();
        spots.sort_unstable();
        // Each spot as it is, else nudged half its height along its side, before the
        // next: a name a little off level beside its dot reads better than one moved
        // round it.
        spots.into_iter().find_map(|(_, i)| {
            [0.0, NUDGE, -NUDGE]
                .into_iter()
                .find(|&by| self.fits(slid(i, node, reach, size, by), own, keep))
                .map(|by| (i, by))
        })
    }

    /// How many dots other than `own` a name at `pill` would cover or lie nearer than
    /// its own, every other dot as big as the cursor's and its own as small as it is
    /// drawn lit: those that may light, then all.
    fn covered(&self, own: usize, reach: f32, pill: egui::Rect) -> (usize, usize) {
        let text = pill.shrink2(PILL_PAD);
        let mine = self
            .dots
            .iter()
            .find(|d| d.tile == own)
            .map_or(0.0, |d| text.distance_to_pos(d.at) - d.least);
        let under: Vec<Disc> = self
            .dot_grid
            .near(pill.expand(reach.max(mine) + CLEAR))
            .into_iter()
            .map(|i| self.dots[i])
            .filter(|d| {
                self.sees(d)
                    && d.tile != own
                    && (pill.distance_to_pos(d.at) < d.steady + CLEAR
                        || text.distance_to_pos(d.at) - d.steady < mine - NEARER)
            })
            .collect();
        (under.iter().filter(|d| d.may_light).count(), under.len())
    }

    /// A name's home: of the spots round its dot, as they are or nudged, the one a
    /// pill of `bare`, the name alone, covers fewest dots at, whatever names are
    /// placed and whatever is lit. A name with no side of its own from the neutral
    /// view goes there or nowhere, so a click that names others first never moves it.
    fn home(&self, own: usize, node: egui::Pos2, reach: f32, bare: egui::Vec2) -> (usize, f32) {
        (0..SPOTS.len())
            .flat_map(|i| {
                [0.0, NUDGE, -NUDGE]
                    .into_iter()
                    .enumerate()
                    .map(move |n| (i, n))
            })
            .min_by_key(|&(i, (nudged, by))| {
                (
                    self.covered(own, reach, slid(i, node, reach, bare, by)),
                    i,
                    nudged,
                )
            })
            .map_or((0, 0.0), |(i, (_, by))| (i, by))
    }

    /// For a lit name with no spot that fits: the first of the spots slid along
    /// their side of the dot by [`SLIDES`] that does, so it stays by its dot.
    fn find_slid(
        &self,
        own: usize,
        node: egui::Pos2,
        reach: f32,
        size: egui::Vec2,
        keep: Keep,
    ) -> Option<(usize, f32)> {
        SLIDES.iter().find_map(|&by| {
            (0..SPOTS.len())
                .find(|&i| self.fits(slid(i, node, reach, size, by), own, keep))
                .map(|i| (i, by))
        })
    }

    /// Where a name that must show goes when no spot fits: on the graph, over the
    /// fewest lit dots, then the fewest names.
    fn least_over(
        &self,
        own: usize,
        node: egui::Pos2,
        reach: f32,
        size: egui::Vec2,
    ) -> (usize, egui::Rect) {
        (0..SPOTS.len())
            .map(|i| (i, spot(i, node, reach, size)))
            .min_by_key(|&(_, pill)| {
                (
                    !self.bounds.contains_rect(pill),
                    self.over_dots(pill, own, Keep::LitDots),
                    self.over_names(pill),
                )
            })
            .unwrap_or_else(|| (0, spot(0, node, reach, size)))
    }

    /// Whether a module's name at `pill` fits: on the graph, over no name, over no
    /// other sphere and none of its own dots.
    fn module_fits(&self, cluster: usize, own_dots: &[usize], pill: egui::Rect) -> bool {
        self.bounds.contains_rect(pill)
            && self.over_names(pill) == 0
            && !self
                .spheres
                .iter()
                .any(|&(c, at, radius)| c != cluster && pill.distance_to_pos(at) < radius - 0.5)
            && !self.dot_grid.near(pill.expand(CLEAR)).into_iter().any(|i| {
                let d = self.dots[i];
                self.sees(&d)
                    && own_dots.contains(&d.tile)
                    && pill.distance_to_pos(d.at) < d.radius + CLEAR
            })
    }
}

/// A name ready to paint: its pill, its text and border.
struct Label {
    named: Named,
    /// A function's name: which of [`SPOTS`] round its dot it took, and how far it
    /// is slid along it, as [`slid`] has it.
    side: Option<(usize, f32)>,
    pill: egui::Rect,
    galley: Arc<egui::Galley>,
    stroke: Option<egui::Color32>,
    /// The journey number a click on it goes to.
    number: Option<String>,
}

fn label_hits_id() -> egui::Id {
    egui::Id::new("graph_label_hits")
}

fn name_sizes_id() -> egui::Id {
    egui::Id::new("graph_name_sizes")
}

/// How big each name on the graph was last laid out, its text without the pill, room
/// found for it or not: what a check needs to judge whether one left out would fit.
pub(crate) fn name_sizes(ctx: &egui::Context) -> Arc<HashMap<Named, egui::Vec2>> {
    ctx.data(|d| d.get_temp(name_sizes_id()))
        .unwrap_or_default()
}

fn name_spots_id() -> egui::Id {
    egui::Id::new("graph_name_spots")
}

/// Where each function's name was last tried as the view is, its pill: the side it
/// holds from the neutral view, or its home, whether it fit or not. A lit function
/// left unnamed was left out for want of room there, and nowhere else.
pub(crate) fn name_spots(ctx: &egui::Context) -> Arc<HashMap<Named, egui::Rect>> {
    ctx.data(|d| d.get_temp(name_spots_id()))
        .unwrap_or_default()
}

fn kept_off_id() -> egui::Id {
    egui::Id::new("graph_names_kept_off")
}

/// Every dot names were last kept off, drawn or held for a deeper level: the
/// function, where it is and how far from its middle names were kept, in points.
pub(crate) fn kept_off(ctx: &egui::Context) -> Arc<Vec<(usize, egui::Pos2, f32)>> {
    ctx.data(|d| d.get_temp(kept_off_id())).unwrap_or_default()
}

fn names_area_id() -> egui::Id {
    egui::Id::new("graph_names_area")
}

/// Where names on the graph were last drawn: the free area they are clipped to, the
/// graph less a docked panel.
pub(crate) fn names_area(ctx: &egui::Context) -> Option<egui::Rect> {
    ctx.data(|d| d.get_temp::<egui::Rect>(names_area_id()))
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

/// The tiers of [`Naming::order`], first placed first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Tier {
    Start,
    Cursor,
    Selection,
    Module,
    /// The start's own calls, numbered.
    Numbered,
    Caller,
    /// The journey's calls further down, numbered, level by level.
    Deeper,
    Lit,
    /// What I brings in from off the journey: its callers, and their modules.
    Beyond,
    Plain,
}

impl Tier {
    /// Whether a name of this tier shows come what may.
    fn always(self) -> bool {
        self <= Self::Selection
    }
}

/// What one frame's names are worked out from: what the view is on, and where every
/// dot that may be named shows.
struct Naming<'a> {
    explorer: &'a Explorer,
    nucleus: &'a Nucleus,
    /// Where a point in the world shows, on the graph or off it.
    to_screen: &'a dyn Fn(Vec3) -> Option<egui::Pos2>,
    /// The camera's up, in the world.
    up: Vec3,
    /// The function drawn white: the journey's start, or browsing, the selection.
    white: Option<usize>,
    cursor: Option<usize>,
    /// The function set aside in a journey, named as the selection is.
    aside: Option<usize>,
    hovered: Option<usize>,
    /// The journey's start.
    start: Option<usize>,
    disclosed: Arc<Disclosed>,
    /// The number of the call the cursor is on.
    cursor_number: Option<String>,
    /// By tile, where each dot drawn on the graph shows, and how far from its middle
    /// its name sits: as far as the biggest a dot is drawn, whatever this one is to
    /// the view, so a name stays put as the selection or the cursor comes and goes.
    discs: Vec<Option<(egui::Pos2, f32)>>,
    /// A journey's functions in the panel's number order, the start left out.
    ladder: Vec<usize>,
    /// How big each name laid out is, its text without the pill, whether it found
    /// room or not.
    sizes: RefCell<HashMap<Named, egui::Vec2>>,
    /// Where each function's name was tried as the view is, its held side or its
    /// home, whether it fit or not.
    spots: RefCell<HashMap<Named, egui::Rect>>,
    /// By cluster, whether its plain functions may be named: [`Naming::roomy`].
    roomy: RefCell<HashMap<usize, bool>>,
}

impl<'a> Naming<'a> {
    fn new(
        explorer: &'a Explorer,
        to_screen: &'a dyn Fn(Vec3) -> Option<egui::Pos2>,
        up: Vec3,
        area: egui::Rect,
    ) -> Self {
        let nucleus = explorer.nucleus();
        let disclosed = explorer.disclosure();
        let cursor_number = explorer
            .cursor_span()
            .and_then(|span| disclosed.number(span))
            .filter(|number| !number.is_empty())
            .map(str::to_owned);
        let start = explorer.start();
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
            white: nucleus.start(),
            cursor: explorer.cursor(),
            aside: explorer.aside(),
            hovered: explorer.hovered(),
            start,
            disclosed,
            cursor_number,
            discs: Vec::new(),
            ladder: ladder.into_iter().map(|(_, tile)| tile).collect(),
            sizes: RefCell::default(),
            spots: RefCell::default(),
            roomy: RefCell::default(),
        };
        naming.discs = (0..explorer.map().tiles.len())
            .map(|tile| {
                let world = explorer.tile_point(tile).filter(|_| nucleus.drawn(tile))?;
                let at = (naming.to_screen)(world).filter(|at| area.contains(*at))?;
                Some((at, naming.shows(tile, world, at, Dot::Focus) + GAP))
            })
            .collect();
        naming
    }

    /// How big `tile`'s dot shows in `dot`'s role, at `world` in the world and `at`
    /// on screen.
    fn shows(&self, tile: usize, world: Vec3, at: egui::Pos2, dot: Dot) -> f32 {
        let radius = dot.radius() * self.nucleus.dot_scale(tile);
        dot.clamp((self.to_screen)(world + self.up * radius).map_or(2.0, |p| p.distance(at)))
    }

    /// Every dot on the graph as names keep off it: one drawn bigger than a speck as
    /// big as the cursor's; a speck as big as it is drawn, or for the `neutral` view
    /// as a lit one.
    fn dots(&self, neutral: bool) -> Vec<Disc> {
        let nucleus = self.nucleus;
        self.discs
            .iter()
            .enumerate()
            .filter_map(|(tile, disc)| {
                let (at, _) = (*disc)?;
                let dot = graph_view::dot(self.explorer, tile);
                let world = self.explorer.tile_point(tile)?;
                let steady = self.shows(tile, world, at, Dot::Focus);
                let radius = match dot {
                    Dot::Speck if neutral => self.shows(tile, world, at, Dot::Lit),
                    Dot::Speck => self.shows(tile, world, at, Dot::Speck),
                    _ => steady,
                };
                // An expanded module's function that nothing else lights.
                let plain = nucleus.opened_up(tile)
                    && dot == Dot::Lit
                    && nucleus.level(tile).is_none()
                    && !nucleus.outside(tile);
                let least = match dot {
                    Dot::Speck => self.shows(tile, world, at, Dot::Speck),
                    _ => self.shows(tile, world, at, Dot::Lit),
                };
                Some(Disc {
                    tile,
                    at,
                    radius,
                    steady,
                    least,
                    lit: dot != Dot::Speck && !plain,
                    outside: nucleus.outside(tile),
                    may_light: dot != Dot::Speck
                        || nucleus.browsing()
                        || nucleus.journey_level(tile).is_some(),
                })
            })
            .chain(self.ghosts())
            .collect()
    }

    /// Where the journey's functions not drawn at this depth would show, as big as
    /// the cursor's: names keep off them as off the lit dots drawn, so a name takes
    /// the same side at every depth, and a level more takes away no room.
    fn ghosts(&self) -> impl Iterator<Item = Disc> + '_ {
        let nucleus = self.nucleus;
        nucleus
            .could_light()
            .filter(|&tile| self.discs.get(tile).is_some_and(Option::is_none))
            .filter_map(move |tile| {
                let world = self.explorer.tile_point(tile)?;
                let at = (self.to_screen)(world)?;
                let radius = self.shows(tile, world, at, Dot::Focus);
                Some(Disc {
                    tile,
                    at,
                    radius,
                    steady: radius,
                    least: self.shows(tile, world, at, Dot::Lit),
                    lit: true,
                    outside: false,
                    may_light: true,
                })
            })
    }

    /// A sphere's middle on screen and how far its edge is from it.
    fn sphere_on_screen(&self, cluster: usize) -> Option<(egui::Pos2, f32)> {
        let ball = self.nucleus.ball(cluster)?;
        let middle = (self.to_screen)(ball.center)?;
        let edge = (self.to_screen)(ball.center + self.up * ball.radius).map_or_else(
            || self.edge_off_screen(ball.center, ball.radius, middle),
            |e| middle.distance(e),
        );
        Some((middle, edge))
    }

    /// How far a sphere's edge shows from its middle when the edge is off the graph:
    /// as the camera sees it, the same as anywhere round it.
    fn edge_off_screen(&self, center: Vec3, radius: f32, middle: egui::Pos2) -> f32 {
        // Measured a little way out instead, and scaled up.
        (self.to_screen)(center + self.up * radius * 0.05)
            .map_or(0.0, |p| p.distance(middle) / 0.05)
    }

    /// Every sphere shown on screen.
    fn spheres(&self) -> Vec<(usize, egui::Pos2, f32)> {
        self.nucleus
            .balls()
            .filter(|(_, ball)| self.nucleus.shows_body(ball.body))
            .filter_map(|(cluster, _)| {
                let (middle, edge) = self.sphere_on_screen(cluster)?;
                Some((cluster, middle, edge))
            })
            .collect()
    }

    /// The tier a function's name is placed in; in the `neutral` view, as though
    /// browsing selected nothing, the journey's cursor and selection were on none and
    /// every level of it were shown.
    fn tier(&self, tile: usize, neutral: bool) -> Tier {
        let nucleus = self.nucleus;
        if neutral && nucleus.browsing() {
            Tier::Plain
        } else if self.start == Some(tile) || (self.start.is_none() && self.white == Some(tile)) {
            Tier::Start
        } else if self.cursor == Some(tile) && !neutral {
            Tier::Cursor
        } else if self.aside == Some(tile) && !neutral {
            Tier::Selection
        } else if nucleus.folded(tile) {
            Tier::Plain
        } else if self.disclosed.tiles().contains_key(&tile) {
            // Callers before the levels a depth brings in, so a new depth takes no
            // name away.
            if nucleus.level(tile).is_some_and(|l| l > 1) {
                Tier::Deeper
            } else {
                Tier::Numbered
            }
        } else if nucleus.level(tile) == Some(-1) {
            Tier::Caller
        } else if let Some(level) = nucleus.journey_level(tile).filter(|_| neutral) {
            // In the neutral view, what a deeper level would light is placed as it
            // would be then, so the sides names take are the same at every depth.
            if level > 1 {
                Tier::Deeper
            } else {
                Tier::Numbered
            }
        } else if nucleus.outside(tile) {
            Tier::Beyond
        } else if nucleus.level(tile).is_some() {
            Tier::Lit
        } else {
            Tier::Plain
        }
    }

    /// Everything that may be named, in the order names are placed: by tier; the
    /// journey's calls in number order; modules biggest first; plain functions those
    /// with most calls first; then by tile, so the order never hangs on anything else.
    fn order(&self, neutral: bool) -> Vec<((Tier, usize, usize), Named)> {
        let rungs: HashMap<usize, usize> = self
            .ladder
            .iter()
            .enumerate()
            .map(|(rung, &tile)| (tile, rung))
            .collect();
        let mut order: Vec<((Tier, usize, usize), Named)> = Vec::new();
        for (tile, disc) in self.discs.iter().enumerate() {
            if disc.is_none() {
                continue;
            }
            let tier = self.tier(tile, neutral);
            let within = match tier {
                Tier::Numbered | Tier::Deeper => rungs.get(&tile).copied().unwrap_or(usize::MAX),
                Tier::Plain => usize::MAX - self.explorer.graph().degree(tile),
                _ => 0,
            };
            order.push(((tier, within, tile), Named::Node(tile)));
        }
        let nucleus = self.nucleus;
        for (cluster, ball) in nucleus.balls() {
            let tier = match ball.body {
                Body::Faint if !nucleus.browsing() => Tier::Beyond,
                _ if nucleus.shows_body(ball.body) || nucleus.ring().contains(&cluster) => {
                    Tier::Module
                }
                _ => continue,
            };
            let within = usize::MAX - nucleus.count(cluster);
            order.push(((tier, within, cluster), Named::Group(cluster)));
        }
        order.sort_by_key(|&(rank, _)| rank);
        order
    }

    /// A module's name as drawn.
    fn module_label(&self, group: usize) -> egui::text::LayoutJob {
        let body = self.nucleus.ball(group).map(|b| b.body);
        let color = egui_color(module_color(group)).gamma_multiply(match body {
            Some(Body::Faint) if !self.nucleus.browsing() => 0.6,
            _ => 1.0,
        });
        let label = self.explorer.cluster_name(group);
        label_job(None, &label.to_uppercase(), None, color, text::TITLE)
    }

    /// A function's name as drawn, `tier` its rank's: the text, its pill's border, and
    /// the journey number a click on it goes to.
    fn function_label(
        &self,
        tile: usize,
        tier: Tier,
    ) -> (egui::text::LayoutJob, Option<egui::Color32>, Option<String>) {
        let (explorer, nucleus) = (self.explorer, self.nucleus);
        let (cursor, start) = (self.cursor, self.start);
        let name = node_name(explorer, tile);
        // Colored as its call is drawn; what is lit by nothing, quietly.
        let level = nucleus.level(tile);
        let focus = matches!(tier, Tier::Start | Tier::Selection);
        let quiet = tier == Tier::Plain || (nucleus.outside(tile) && self.aside != Some(tile));
        let color = match level {
            _ if cursor == Some(tile) => CURSOR_TEXT,
            _ if focus => egui::Color32::WHITE,
            // As faint as its dot, where that is faded, so the name reads as its.
            _ if quiet && graph_view::node_look(explorer, tile) == graph_view::NodeLook::Faded => {
                FADED_TEXT
            }
            _ if quiet => PLAIN_TEXT,
            Some(-1) => CALLER_TEXT,
            Some(1) => CALLEE_TEXT,
            Some(2..) => DEEPER_TEXT,
            _ => PLAIN_TEXT,
        };
        let panel = self
            .disclosed
            .tiles()
            .get(&tile)
            .map_or(&[][..], Vec::as_slice);
        // A start the journey calls again says so, after saying it is the start, so
        // the call back is not read as its own; one that calls itself says that. A
        // call of the journey that calls the start back is drawn as the call it is,
        // and says it calls back.
        let again = match round_note(explorer, tile) {
            Some(note) => Some(note.to_owned()),
            None if start == Some(tile) => {
                nucleus::badge(panel, None).map(|again| format!("start · again at {again}"))
            }
            None => None,
        };
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
        // A function the journey runs leads with its place in it, as its bar does: its
        // first call in the panel's order, and how many more it runs; but the cursor's
        // shows the call the cursor is on, as the panel does. The start is the start.
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
        // The number on a badge filled like the dot and its bar, so the two read as
        // one.
        let fill = egui_color(graph_view::node_color(explorer, tile));
        let lead = order.map(|order| (format!(" {order} "), ON_FILL, Some(fill)));
        // The pill's border says which way it relates to the start, as its call does;
        // the cursor's is its own color.
        let stroke = match level {
            _ if cursor == Some(tile) => Some(egui_color(CURSOR)),
            _ if focus => Some(egui::Color32::from_white_alpha(140)),
            _ if quiet => None,
            Some(-1) => Some(egui_color(CALLER)),
            Some(1) => Some(egui_color(CALLEE)),
            Some(2..) => Some(egui_color(DEEPER)),
            _ => None,
        };
        // One size for every function's name, so a click that makes one the selection
        // or the cursor never moves it.
        let job = label_job(lead, &name, said, color, text::BODY);
        (job, stroke, number)
    }

    /// Whether a function's sphere shows big enough, and its dots far enough apart,
    /// for plain names to sit by their own dots rather than float round a speck.
    fn roomy(&self, tile: usize) -> bool {
        let Some(cluster) = self.explorer.graph().cluster(tile) else {
            return false;
        };
        *self.roomy.borrow_mut().entry(cluster).or_insert_with(|| {
            let Some((_, radius)) = self.sphere_on_screen(cluster) else {
                return false;
            };
            #[expect(clippy::cast_precision_loss, reason = "a count of dots")]
            let dots = self.own_dots(cluster).len().max(1) as f32;
            radius >= PLAIN_SPHERE && radius * (std::f32::consts::PI / dots).sqrt() >= PLAIN_APART
        })
    }

    /// The dots of a module drawn on the graph.
    fn own_dots(&self, cluster: usize) -> Vec<usize> {
        let graph = self.explorer.graph();
        (0..self.discs.len())
            .filter(|&t| self.discs[t].is_some() && graph.cluster(t) == Some(cluster))
            .collect()
    }

    /// Lays out the name of `named`, and notes how big it is.
    fn pill(
        &self,
        painter: &egui::Painter,
        named: Named,
        job: egui::text::LayoutJob,
    ) -> (Arc<egui::Galley>, egui::Vec2) {
        let (galley, size) = pill_of(painter, job);
        self.sizes.borrow_mut().insert(named, galley.size());
        (galley, size)
    }

    /// A module's name, centred above its sphere, where it fits.
    fn module_name(
        &self,
        painter: &egui::Painter,
        placer: &Placer,
        cluster: usize,
    ) -> Option<Label> {
        let (middle, edge) = self.sphere_on_screen(cluster)?;
        let (galley, size) = self.pill(painter, Named::Group(cluster), self.module_label(cluster));
        let pill = above(middle, edge, size);
        placer
            .module_fits(cluster, &self.own_dots(cluster), pill)
            .then_some(Label {
                named: Named::Group(cluster),
                side: None,
                pill,
                galley,
                stroke: None,
                number: None,
            })
    }

    /// A function's name beside its dot, of `tier`, where it fits; one that must show
    /// wherever it covers least. As the view is, with the `sides` names took in the
    /// neutral view, a name stays on its side there, or else at its [`Placer::home`],
    /// or goes: so a click or a step that lights or unlights others never moves a
    /// name, only takes it away where what lights takes its place. The start, the
    /// cursor and the selection are named come what may. A plain function is named
    /// only where its sphere shows big enough to keep names by their own dots, and
    /// keeps off every dot; a lit one may lie over a dot nothing lights.
    fn function_name(
        &self,
        painter: &egui::Painter,
        placer: &Placer,
        (tile, tier): (usize, Tier),
        sides: Option<&HashMap<usize, (usize, f32)>>,
    ) -> Option<Label> {
        let (node, reach) = self.discs[tile]?;
        if tier == Tier::Plain && !self.roomy(tile) {
            return None;
        }
        let (job, stroke, number) = self.function_label(tile, tier);
        let lit = tier != Tier::Plain;
        let keep = if lit { Keep::LitDots } else { Keep::EveryDot };
        // An expanded module's own functions are named as the lit are, after them.
        let ranked = lit || self.nucleus.opened_up(tile);
        let label = |side: Option<(usize, f32)>, pill: egui::Rect, galley| Label {
            named: Named::Node(tile),
            side,
            pill,
            galley,
            stroke,
            number: number.clone(),
        };
        let name = node_name(self.explorer, tile);
        let bare = || {
            pill_of(
                painter,
                label_job(None, &name, None, PLAIN_TEXT, text::BODY),
            )
            .1
        };
        if let Some(sides) = sides {
            let held = sides
                .get(&tile)
                .copied()
                .or_else(|| ranked.then(|| placer.home(tile, node, reach, bare())));
            let (index, by) = held?;
            let (galley, size) = self.pill(painter, Named::Node(tile), job.clone());
            let pill = slid(index, node, reach, size, by);
            self.spots.borrow_mut().insert(Named::Node(tile), pill);
            if placer.fits(pill, tile, keep) {
                return Some(label(Some((index, by)), pill, galley));
            }
            if !tier.always() {
                return None;
            }
        }
        if ranked {
            // In the neutral view, or come what may: the side its dot's neighbours, at
            // every depth, leave most room on, that is free. One a deeper level
            // numbers, shown unnumbered, keeps room for its number, so a level more or
            // less leaves it where it is.
            let (galley, size) = self.pill(painter, Named::Node(tile), job.clone());
            let bare = bare();
            let numbered = self
                .nucleus
                .journey_level(tile)
                .filter(|_| number.is_none())
                .map(|level| {
                    let digits = vec!["0"; level as usize].join(".");
                    let lead = (format!(" {digits} "), ON_FILL, Some(PILL));
                    pill_of(
                        painter,
                        label_job(Some(lead), &name, None, PLAIN_TEXT, text::BODY),
                    )
                    .1
                });
            let found = numbered
                .map(|room| room.max(size))
                .and_then(|room| placer.find_by_dots(tile, node, reach, (room, bare), keep))
                .or_else(|| placer.find_by_dots(tile, node, reach, (size, bare), keep));
            if let Some((free, by)) = found {
                return Some(label(
                    Some((free, by)),
                    slid(free, node, reach, size, by),
                    galley,
                ));
            }
        } else {
            // Most plain names have no room: see whether one could, before laying it
            // out.
            #[expect(clippy::cast_precision_loss, reason = "a name's length")]
            let guess = egui::vec2(
                job.text.chars().count() as f32 * text::BODY * CHAR_WIDTH,
                text::BODY,
            ) + 2.0 * PILL_PAD;
            placer.find(tile, node, reach, guess, false)?;
        }
        let (galley, size) = self.pill(painter, Named::Node(tile), job);
        let (taken, pill) = match placer.find(tile, node, reach, size, lit) {
            Some((index, at)) => ((index, 0.0), at),
            None => match placer
                .find_slid(tile, node, reach, size, keep)
                .filter(|_| ranked)
            {
                Some((index, by)) => ((index, by), slid(index, node, reach, size, by)),
                None if tier.always() => {
                    let (index, at) = placer.least_over(tile, node, reach, size);
                    ((index, 0.0), at)
                }
                None => return None,
            },
        };
        Some(label(Some(taken), pill, galley))
    }

    /// What the pointer is on and is not named already: the function hovered, beside
    /// its dot wherever it covers least, and the module under the pointer, above its
    /// sphere where that covers no name; taking no room.
    fn pointed(&self, painter: &egui::Painter, placer: &Placer, labels: &[Label]) -> Vec<Label> {
        let named = |key: Named| labels.iter().any(|l| l.named == key);
        let mut over = Vec::new();
        if let Some(tile) = self.hovered.filter(|&t| !named(Named::Node(t)))
            && let Some((node, reach)) = self.discs[tile]
        {
            let (job, stroke, number) = self.function_label(tile, self.tier(tile, false));
            let (galley, size) = pill_of(painter, job);
            let (taken, pill) = placer
                .find(tile, node, reach, size, true)
                .unwrap_or_else(|| placer.least_over(tile, node, reach, size));
            over.push(Label {
                named: Named::Node(tile),
                side: Some((taken, 0.0)),
                pill,
                galley,
                stroke,
                number,
            });
        }
        let mouse = painter.ctx().input(|i| i.pointer.hover_pos());
        let module = mouse.and_then(|mouse| {
            placer
                .spheres
                .iter()
                .filter(|&&(_, middle, edge)| mouse.distance(middle) <= edge)
                .min_by(|a, b| a.2.total_cmp(&b.2))
                .map(|&(cluster, ..)| cluster)
        });
        if let Some(cluster) = module.filter(|&c| !named(Named::Group(c)))
            && let Some((middle, edge)) = self.sphere_on_screen(cluster)
        {
            let (galley, size) = pill_of(painter, self.module_label(cluster));
            let pill = above(middle, edge, size);
            // Not over a name placed: pointing hides no name.
            if !placer.bounds.contains_rect(pill) || placer.over_names(pill) > 0 {
                return over;
            }
            over.push(Label {
                named: Named::Group(cluster),
                side: None,
                pill,
                galley,
                stroke: None,
                number: None,
            });
        }
        over
    }
}

/// What a function's name and its bar say of it calling round: that it calls itself
/// straight back, or, a call of the journey, that it calls the start back.
pub(crate) fn round_note(explorer: &Explorer, tile: usize) -> Option<&'static str> {
    let start = explorer.start()?;
    if calls_itself(explorer, tile) {
        Some("↺ calls itself")
    } else if tile != start
        && explorer.nucleus().level(tile).is_some_and(|l| l > 0)
        && explorer.graph().callees(tile).any(|t| t == start)
    {
        Some("↺ calls back")
    } else {
        None
    }
}

/// Whether a function calls itself directly.
fn calls_itself(explorer: &Explorer, tile: usize) -> bool {
    explorer
        .tile_unit(tile)
        .and_then(|id| explorer.system().unit(id))
        .is_some_and(|unit| {
            unit.calls
                .iter()
                .any(|call| call.targets.iter().any(|t| t.unit == unit.id))
        })
}

/// Lays out a label's text, and its pill round it.
fn pill_of(painter: &egui::Painter, job: egui::text::LayoutJob) -> (Arc<egui::Galley>, egui::Vec2) {
    let galley = painter.layout_job(job);
    let size = galley.size() + 2.0 * PILL_PAD;
    (galley, size)
}

/// Names beside graph dots and above module spheres, each on a dark pill so it reads
/// over the arrows, placed in [`Naming::order`] wherever each fits; a journey's
/// functions lead with their numbers in it.
pub(crate) fn draw_graph_labels(
    painter: &egui::Painter,
    project: &impl Fn(Vec3) -> Option<egui::Pos2>,
    explorer: &Explorer,
    up: Vec3,
) {
    let naming = Naming::new(explorer, project, up, painter.clip_rect());
    let nucleus = naming.nucleus;
    // First as though nothing were selected and the cursor on no call, for the side
    // each name takes, then as the view is.
    let (neutral, _) = place(&naming, painter, None);
    let sides: HashMap<usize, (usize, f32)> = neutral
        .iter()
        .filter_map(|l| match (l.named, l.side) {
            (Named::Node(tile), Some(side)) => Some((tile, side)),
            _ => None,
        })
        .collect();
    let (mut labels, placer) = place(&naming, painter, Some(&sides));
    let laid_out = naming.sizes.take();
    let tried = naming.spots.take();
    let kept_off: Vec<(usize, egui::Pos2, f32)> = placer
        .dots
        .iter()
        .map(|d| (d.tile, d.at, d.radius))
        .collect();
    painter.ctx().data_mut(|d| {
        d.insert_temp(name_sizes_id(), Arc::new(laid_out));
        d.insert_temp(name_spots_id(), Arc::new(tried));
        d.insert_temp(kept_off_id(), Arc::new(kept_off));
    });
    let over = naming.pointed(painter, &placer, &labels);
    // Every function lit and on screen is named, or counted here as not.
    let lit: Vec<usize> = (0..naming.discs.len())
        .filter(|&tile| {
            naming.discs[tile].is_some() && nucleus.level(tile).is_some() && !nucleus.folded(tile)
        })
        .collect();
    let unnamed = lit
        .iter()
        .filter(|&&tile| {
            !labels
                .iter()
                .chain(&over)
                .any(|l| l.named == Named::Node(tile))
        })
        .count();
    // The first placed on top, and what the pointer is on over everything.
    labels.reverse();
    labels.extend(over);
    paint(painter, &labels);
    if unnamed > 0 {
        unnamed_note(painter, unnamed, lit.len());
    }
}

/// Paints `labels`, the last on top, and remembers where each function's name is,
/// so a click on it picks it.
fn paint(painter: &egui::Painter, labels: &[Label]) {
    let mut hits: Vec<LabelHit> = Vec::new();
    for label in labels {
        painter.rect_filled(label.pill, 4.0, PILL);
        if let Some(color) = label.stroke {
            painter.rect_stroke(
                label.pill,
                4.0,
                egui::Stroke::new(1.5, color),
                egui::StrokeKind::Inside,
            );
        }
        let at = egui::emath::GuiRounding::round_to_pixels(
            label.pill.min + PILL_PAD,
            painter.pixels_per_point(),
        );
        painter.galley(at, label.galley.clone(), egui::Color32::WHITE);
        if let Named::Node(tile) = label.named {
            hits.push((label.pill, tile, label.number.clone()));
        }
    }
    let area = painter.clip_rect();
    painter.ctx().data_mut(|d| {
        d.insert_temp(label_hits_id(), hits);
        d.insert_temp(names_area_id(), area);
    });
}

/// Every name in [`Naming::order`] where it fits, as the view is, or with no `sides`
/// held, in the neutral view; and what they took.
fn place(
    naming: &Naming,
    painter: &egui::Painter,
    sides: Option<&HashMap<usize, (usize, f32)>>,
) -> (Vec<Label>, Placer) {
    let nucleus = naming.nucleus;
    let neutral = sides.is_none();
    let mut placer = Placer::new(painter.clip_rect(), naming.dots(neutral), naming.spheres());
    // Names keep off the search box while it shows, as last laid out, and browsing,
    // off the hint at the keys along the graph's foot and the note on names left out
    // above it.
    let search = egui::Id::new("search");
    if let Some(open) = painter.ctx().memory(|m| {
        m.areas()
            .visible_layer_ids()
            .iter()
            .any(|l| l.id == search)
            .then(|| m.area_rect(search))
            .flatten()
    }) {
        placer.take(open);
    }
    if nucleus.browsing() {
        let hint = key_help::hint_rect(painter, painter.clip_rect(), nucleus.expanded());
        placer.take(hint.union(hint.translate(egui::vec2(0.0, -NOTE_ABOVE))));
    }
    let mut labels: Vec<Label> = Vec::new();
    for ((tier, ..), named) in naming.order(neutral) {
        placer.outside = tier == Tier::Beyond;
        let label = match named {
            Named::Group(cluster) => naming.module_name(painter, &placer, cluster),
            Named::Node(tile) => naming.function_name(painter, &placer, (tile, tier), sides),
        };
        if let Some(label) = label {
            placer.take(label.pill);
            // A module on the ring not shown at this depth keeps its name's room.
            let shown = match named {
                Named::Group(cluster) => nucleus
                    .ball(cluster)
                    .is_some_and(|b| nucleus.shows_body(b.body)),
                Named::Node(_) => true,
            };
            if shown {
                labels.push(label);
            }
        }
    }
    (labels, placer)
}

/// How many of the functions lit had no room for their names, at the graph's foot
/// above the hint at the keys.
fn unnamed_note(painter: &egui::Painter, unnamed: usize, lit: usize) {
    let area = painter.clip_rect();
    let note = format!(
        "{unnamed} of {lit} lit functions have no room for a name · zoom in, or point at one"
    );
    let galley = painter.layout_no_wrap(note, egui::FontId::proportional(text::SMALL), MUTED);
    let at = egui::pos2(
        area.min.x + 12.0,
        area.max.y - 10.0 - NOTE_ABOVE - galley.size().y,
    );
    painter.rect_filled(
        egui::Rect::from_min_size(at, galley.size()).expand2(PILL_PAD),
        4.0,
        PILL,
    );
    painter.galley(at, galley, MUTED);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> egui::Rect {
        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0))
    }

    fn disc(tile: usize, x: f32, y: f32, lit: bool) -> Disc {
        Disc {
            tile,
            at: egui::pos2(x, y),
            radius: 4.0,
            steady: 4.0,
            least: 4.0,
            lit,
            outside: false,
            may_light: lit,
        }
    }

    const SIZE: egui::Vec2 = egui::vec2(60.0, 18.0);

    #[test]
    fn a_name_goes_right_of_its_dot_when_that_is_free() {
        let placer = Placer::new(bounds(), vec![disc(0, 100.0, 100.0, true)], Vec::new());
        let (side, pill) = placer
            .find(0, egui::pos2(100.0, 100.0), 12.0, SIZE, true)
            .expect("room");
        assert_eq!(side, 0);
        assert!(pill.min.x > 100.0 && (pill.center().y - 100.0).abs() < 0.5);
    }

    #[test]
    fn a_name_keeps_off_names_placed_before_it_and_other_dots() {
        let mut placer = Placer::new(
            bounds(),
            vec![disc(0, 100.0, 100.0, true), disc(1, 140.0, 100.0, true)],
            Vec::new(),
        );
        let right = spot(0, egui::pos2(100.0, 100.0), 12.0, SIZE);
        assert!(!placer.fits(right, 0, Keep::LitDots), "over the other dot");
        let (side, pill) = placer
            .find(0, egui::pos2(100.0, 100.0), 12.0, SIZE, true)
            .expect("room on the left");
        assert!(pill.max.x < 100.0);
        placer.take(pill);
        let again = placer.find(0, egui::pos2(100.0, 100.0), 12.0, SIZE, true);
        assert_ne!(again, Some((side, pill)), "its place is taken now");
    }

    #[test]
    fn a_lit_name_may_lie_over_a_faint_dot_but_never_a_lit_one() {
        let placer = Placer::new(
            bounds(),
            vec![disc(0, 100.0, 100.0, true), disc(1, 140.0, 100.0, false)],
            Vec::new(),
        );
        let right = spot(0, egui::pos2(100.0, 100.0), 12.0, SIZE);
        assert!(!placer.fits(right, 0, Keep::EveryDot));
        assert!(placer.fits(right, 0, Keep::LitDots));
    }

    #[test]
    fn a_names_home_hangs_on_the_dots_alone() {
        let node = egui::pos2(100.0, 100.0);
        let mut placer = Placer::new(
            bounds(),
            vec![disc(0, 100.0, 100.0, true), disc(1, 140.0, 100.0, false)],
            Vec::new(),
        );
        let (side, by) = placer.home(0, node, 12.0, SIZE);
        assert_ne!(side, 0, "right covers the other dot, lit or not");
        placer.take(slid(side, node, 12.0, SIZE, by));
        assert_eq!(
            placer.home(0, node, 12.0, SIZE),
            (side, by),
            "names take no home"
        );
    }

    #[test]
    fn a_name_that_does_not_fit_on_the_graph_is_left_out() {
        let placer = Placer::new(bounds(), vec![disc(0, 5.0, 5.0, true)], Vec::new());
        let tight = egui::vec2(790.0, 590.0);
        assert!(
            placer
                .find(0, egui::pos2(5.0, 5.0), 12.0, tight, true)
                .is_none()
        );
    }

    #[test]
    fn a_module_name_sits_centred_above_its_sphere_and_off_others() {
        let placer = Placer::new(
            bounds(),
            vec![disc(0, 200.0, 160.0, false)],
            vec![
                (0, egui::pos2(200.0, 200.0), 50.0),
                (1, egui::pos2(200.0, 110.0), 25.0),
            ],
        );
        let pill = above(egui::pos2(200.0, 200.0), 50.0, SIZE);
        assert!((pill.center().x - 200.0).abs() < 0.5);
        assert!((pill.max.y - (150.0 - ABOVE)).abs() < 0.5);
        assert!(!placer.module_fits(0, &[0], pill), "over the sphere above");
        let clear = Placer::new(
            bounds(),
            Vec::new(),
            vec![(0, egui::pos2(200.0, 200.0), 50.0)],
        );
        assert!(clear.module_fits(0, &[], pill));
        let own = Placer::new(
            bounds(),
            vec![disc(0, 200.0, 140.0, false)],
            vec![(0, egui::pos2(200.0, 200.0), 50.0)],
        );
        assert!(!own.module_fits(0, &[0], pill), "over its own dot");
    }

    #[test]
    fn graph_and_bar_say_alike_what_calls_itself_and_what_calls_back() {
        // walk calls itself, and visit, which calls walk.
        let json = r#"{ "schema": "codeviz/0.1", "producer": { "name": "t", "version": "0" }, "units": [
            { "id": "x:walk", "name": "walk", "kind": "fn", "role": "invocable", "calls": [
                { "key": "w#0", "text": "walk", "targets": [ { "unit": "x:walk", "certainty": "exact" } ] },
                { "key": "v#0", "text": "visit", "targets": [ { "unit": "x:visit", "certainty": "exact" } ] } ] },
            { "id": "x:visit", "name": "visit", "kind": "fn", "role": "invocable", "calls": [
                { "key": "w#0", "text": "walk", "targets": [ { "unit": "x:walk", "certainty": "exact" } ] } ] } ] }"#;
        let system =
            codeviz_core::System::new([codeviz_ir::Document::from_json(json).expect("reads")]);
        let explorer = crate::explorer::tests::opened(system, "x:walk");
        let walk = explorer.start().expect("a start");
        let visit = (0..explorer.map().tiles.len())
            .find(|&t| t != walk && explorer.tile_unit(t).is_some())
            .expect("visit");
        assert_eq!(round_note(&explorer, walk), Some("↺ calls itself"));
        assert_eq!(round_note(&explorer, visit), Some("↺ calls back"));
    }

    #[test]
    fn names_are_placed_in_one_order_of_importance() {
        assert!(Tier::Start < Tier::Cursor && Tier::Cursor < Tier::Selection);
        assert!(Tier::Selection < Tier::Module && Tier::Module < Tier::Numbered);
        assert!(Tier::Numbered < Tier::Caller && Tier::Caller < Tier::Deeper);
        assert!(Tier::Deeper < Tier::Lit);
        assert!(Tier::Lit < Tier::Beyond && Tier::Beyond < Tier::Plain);
        assert!(Tier::Selection.always() && !Tier::Numbered.always());
    }
}
