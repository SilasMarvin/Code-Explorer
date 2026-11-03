//! Where a journey's functions sit inside their spheres: spread over each, every one
//! put, in the journey's order, at the place in its sphere where the calls drawn so
//! far run through neither its dot nor its name, its own calls run through no dot or
//! name put before it, and its name covers nothing, as near its sphere's middle as
//! that allows; then, in each module round the nucleus, a few places left free the
//! same way for callers from off the journey. Worked out once for every depth, as the
//! usual view sees the spheres, square to it.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use codeviz_layout::atlas;

use super::{inside::spaced, view_rotation};
use crate::sizes::Dot;

/// How a journey is framed, roughly: the half of the free area's height its ring of
/// spheres spans, in points. What sizes a name against the spheres.
const FRAMED: f32 = 380.0;
/// A name as the label placer draws it, in points: its height, its width round its
/// text (its number's badge and its pill), how far from its dot it starts, and the
/// room kept round dots and names.
const NAME_HIGH: f32 = 18.0;
const NAME_PADDED: f32 = 34.0;
const NAME_GAP: f32 = 4.0;
const CLEAR: f32 = 3.0;
/// Characters in a name the journey has none for; and kept for what a name says
/// after it (that it can fail, when it runs), and the start's, that it is the start
/// and where the journey runs it again.
const SOME_NAME: usize = 12;
const NOTE: usize = 8;
const START_NOTE: usize = 22;
/// How many places a sphere offers for each function it lays out, at least; and how
/// much further apart along a row than between rows, as names run along.
const PLACES_EACH: usize = 4;
const LEAST_PLACES: usize = 12;
/// How many places a module round the nucleus keeps free as its names are put, so
/// where the names go never hangs on who else may come in: any more it keeps are
/// found once every sphere's names are put.
const BESIDES: usize = 3;
const ROW_WIDE: f32 = 2.0;
/// What a place costs: a call's line through a dot, through a name, a name or a dot
/// over another's, its name on its left rather than its right, and each unit from its
/// sphere's middle.
const THROUGH_DOT: f32 = 10.0;
const THROUGH_NAME: f32 = 3.0;
const OVER: f32 = 30.0;
const UNDER_START: f32 = 1000.0;
const LEFT: f32 = 0.1;
const FAR: f32 = 0.05;
/// How much a place costs for each function already in its sphere near it, at most,
/// less the further off, out to [`SPREAD_REACH`] of the sphere's room: so they spread
/// over it rather than bunch at its middle.
const SPREAD: f32 = 0.6;
const SPREAD_REACH: f32 = 0.9;

/// Keys at or past this stand for places kept free, not functions.
const SPARE_KEYS: usize = usize::MAX / 2;

/// A sphere to lay out: where it is, and what it names, in order.
pub(super) struct Room {
    pub(super) cluster: usize,
    pub(super) center: Vec3,
    pub(super) radius: f32,
    pub(super) tiles: Vec<usize>,
    /// A function that sits in the middle come what may: the journey's start.
    pub(super) middle: Option<usize>,
    /// Places it leaves free once its names are placed.
    pub(super) spare: usize,
}

/// Where each function goes, and the places each sphere leaves free, in the order
/// they are to be taken.
#[derive(Debug, Default)]
pub(super) struct Placed {
    pub(super) points: HashMap<usize, Vec3>,
    pub(super) free: HashMap<usize, Vec<Vec3>>,
    /// Of those, by cluster and place, the ones with no room clear of the names round
    /// them, or on a call's line: whoever would take one is left for the module
    /// expanded to show.
    pub(super) crowded: HashSet<(usize, usize)>,
    /// The functions whose names go on their left.
    pub(super) left: HashSet<usize>,
}

/// A name's box, flat, as the usual view sees it.
#[derive(Debug, Clone, Copy)]
struct Box2 {
    min: Vec2,
    max: Vec2,
}

impl Box2 {
    fn overlaps(self, other: Self) -> bool {
        self.min.x < other.max.x
            && other.min.x < self.max.x
            && self.min.y < other.max.y
            && other.min.y < self.max.y
    }

    fn near(self, at: Vec2, radius: f32) -> bool {
        at.clamp(self.min, self.max).distance(at) < radius
    }

    /// Whether the line from `a` to `b` runs through it.
    fn crossed(self, a: Vec2, b: Vec2) -> bool {
        let (mut t0, mut t1) = (0.0_f32, 1.0_f32);
        let along = b - a;
        for (start, delta, low, high) in [
            (a.x, along.x, self.min.x, self.max.x),
            (a.y, along.y, self.min.y, self.max.y),
        ] {
            if delta.abs() < f32::EPSILON {
                if start <= low || start >= high {
                    return false;
                }
                continue;
            }
            let (u0, u1) = ((low - start) / delta, (high - start) / delta);
            t0 = t0.max(u0.min(u1));
            t1 = t1.min(u0.max(u1));
        }
        t0 < t1
    }
}

/// How near the line from `a` to `b` comes to `at`.
fn reach(a: Vec2, b: Vec2, at: Vec2) -> f32 {
    let along = b - a;
    let t = ((at - a).dot(along) / along.length_squared().max(f32::EPSILON)).clamp(0.0, 1.0);
    (a + along * t).distance(at)
}

/// What has been put so far, flat: dots, names, and where each function is.
struct Board<'a> {
    /// A function's dot as names and lines keep off it, in world units.
    dot: f32,
    /// A name's height, how far from its dot it starts, and the world units a
    /// point takes.
    high: f32,
    gap: f32,
    per_point: f32,
    chars: &'a dyn Fn(usize) -> usize,
    calls: &'a [(usize, usize)],
    /// The journey's start, whose name says more.
    start: Option<usize>,
    at: HashMap<usize, Vec2>,
    names: Vec<(usize, Box2)>,
    left: HashSet<usize>,
    /// Each function laid out, and the middle of its sphere.
    home: HashMap<usize, Vec2>,
}

impl Board<'_> {
    /// The box of `tile`'s name with its dot at `at`, on its right or its left.
    fn name(&self, tile: Option<usize>, at: Vec2, left: bool) -> Box2 {
        let note = if tile.is_some() && tile == self.start {
            START_NOTE
        } else {
            NOTE
        };
        let chars = tile
            .map(|t| (self.chars)(t))
            .filter(|&c| c > 0)
            .unwrap_or(SOME_NAME)
            + note;
        let wide =
            (crate::text::name_width(chars, crate::text::BODY) + NAME_PADDED) * self.per_point;
        let start = self.dot + self.gap;
        let (x0, x1) = if left {
            (at.x - start - wide, at.x - start)
        } else {
            (at.x + start, at.x + start + wide)
        };
        Box2 {
            min: Vec2::new(x0, at.y - self.high / 2.0),
            max: Vec2::new(x1, at.y + self.high / 2.0),
        }
    }

    /// Where a call's other end is for now: where it was put, else the middle of its
    /// sphere, unless that is the sphere being laid out.
    fn end(&self, tile: usize, laying: Vec2) -> Option<Vec2> {
        self.at.get(&tile).copied().or_else(|| {
            self.home
                .get(&tile)
                .copied()
                .filter(|home| home.distance(laying) > f32::EPSILON)
        })
    }

    /// The lines drawn so far, as far as they are known laying out the sphere whose
    /// middle is `laying`: every call with both ends put, or one put and the other in
    /// another sphere, from its middle.
    fn lines(&self, laying: Vec2) -> impl Iterator<Item = (usize, usize, Vec2, Vec2)> + '_ {
        self.calls.iter().filter_map(move |&(from, to)| {
            let (a, b) = (self.end(from, laying)?, self.end(to, laying)?);
            (self.at.contains_key(&from) || self.at.contains_key(&to)).then_some((from, to, a, b))
        })
    }

    /// What `tile` (or a place left free, with none) costs at `at` in a sphere whose
    /// middle is `middle`, its name on the left or not; and the cheaper side.
    fn cost(&self, tile: Option<usize>, at: Vec2, (middle, room): (Vec2, f32)) -> (f32, bool) {
        let keep = self.dot * 2.0;
        let mut cost = at.distance(middle) * FAR;
        let apart = room * SPREAD_REACH;
        for &there in self.at.values() {
            if there.distance(middle) <= room {
                cost += SPREAD * (1.0 - there.distance(at) / apart).max(0.0);
            }
        }
        // Its dot: on no line drawn, near no dot, under no name. A place kept free on
        // a line costs as much as one crowded: whoever took it would read as that call's.
        let on_line = if tile.is_none() { OVER } else { THROUGH_DOT };
        for (from, to, a, b) in self.lines(middle) {
            if Some(from) != tile && Some(to) != tile && reach(a, b, at) < keep {
                cost += on_line;
            }
        }
        for (&other, &there) in &self.at {
            if Some(other) != tile && there.distance(at) < keep {
                cost += OVER;
            }
        }
        for &(named, name) in &self.names {
            // The start is named come what may: nothing goes under its name, nor so
            // near that its name would have to move over it.
            if Some(named) == self.start && name.near(at, self.dot + self.high) {
                cost += UNDER_START;
            } else if name.near(at, self.dot) {
                cost += OVER;
            }
        }
        // A place kept free keeps clear of the names round it, and of where they
        // might go.
        if tile.is_none() {
            cost += OVER
                * (f32::from(u8::from(self.crowded(at)))
                    + f32::from(u8::from(self.either_side(at))));
        }
        // Its lines: through no dot and no name put before it.
        if let Some(tile) = tile {
            let ends: Vec<(usize, Vec2)> = self
                .calls
                .iter()
                .filter_map(|&(from, to)| match (from == tile, to == tile) {
                    (true, false) => Some((to, self.end(to, middle)?)),
                    (false, true) => Some((from, self.end(from, middle)?)),
                    _ => None,
                })
                .collect();
            for &(other, end) in &ends {
                for (&dot, &there) in &self.at {
                    if dot != other && reach(at, end, there) < keep {
                        cost += THROUGH_DOT;
                    }
                }
                for &(named, name) in &self.names {
                    if named != other && name.crossed(at, end) {
                        cost += THROUGH_NAME;
                    }
                }
            }
        }
        // Its name: on no line drawn, over no dot or name; on its right where it can.
        let side = |left: bool| {
            let name = self.name(tile, at, left);
            let mut cost = if left { LEFT } else { 0.0 };
            for (from, to, a, b) in self.lines(middle) {
                if Some(from) != tile && Some(to) != tile && name.crossed(a, b) {
                    cost += THROUGH_NAME;
                }
            }
            for (&other, &there) in &self.at {
                if Some(other) != tile && name.near(there, self.dot) {
                    cost += OVER;
                }
            }
            for &(_, other) in &self.names {
                if name.overlaps(other) {
                    cost += OVER;
                }
            }
            cost
        };
        let (right, left) = (side(false), side(true));
        if left < right {
            (cost + left, true)
        } else {
            (cost + right, false)
        }
    }

    /// Whether a place kept free at `at` has no room clear of the functions round
    /// it: near a dot, or under a name, by a name's height besides, as names stay
    /// their size when the view draws back, so whoever comes in reads as no one's.
    fn crowded(&self, at: Vec2) -> bool {
        let clear = self.dot + self.high / 2.0 + self.gap;
        self.at
            .iter()
            .any(|(&tile, &there)| tile < SPARE_KEYS && there.distance(at) < self.dot * 2.0)
            || self
                .names
                .iter()
                .any(|&(tile, name)| tile < SPARE_KEYS && name.near(at, clear))
    }

    /// Whether a name of a function on either side of its dot comes near `at`: the
    /// name may take either side.
    fn either_side(&self, at: Vec2) -> bool {
        self.at.iter().any(|(&tile, &there)| {
            tile < SPARE_KEYS
                && [false, true]
                    .into_iter()
                    .any(|left| self.name(Some(tile), there, left).near(at, self.dot))
        })
    }

    fn put(&mut self, tile: usize, at: Vec2, left: bool) {
        let name = self.name(Some(tile), at, left);
        self.at.insert(tile, at);
        self.names.push((tile, name));
        if left {
            self.left.insert(tile);
        }
    }
}

/// The cheapest of `spots` for `tile` (or a place kept free, with none) in the
/// sphere whose middle and room are given, taken from them, and its name's side.
fn best(
    board: &Board,
    tile: Option<usize>,
    spots: &mut [Option<Vec2>],
    sphere: (Vec2, f32),
) -> Option<(Vec2, bool)> {
    let (index, at, left) = spots
        .iter()
        .enumerate()
        .filter_map(|(i, p)| Some((i, (*p)?)))
        .map(|(i, at)| {
            let (cost, left) = board.cost(tile, at, sphere);
            (cost, i, at, left)
        })
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)))
        .map(|(_, i, at, left)| (i, at, left))?;
    spots[index] = None;
    Some((at, left))
}

/// Up to `count` places over the sphere whose middle and room are given, closer than
/// names keep rows apart, none on a dot put or a place in `spots`.
fn closer(
    board: &Board,
    spots: &[Option<Vec2>],
    (middle, reach): (Vec2, f32),
    count: usize,
) -> Vec<Option<Vec2>> {
    spaced((count, count), reach, ROW_WIDE, 0.0)
        .into_iter()
        .map(|at| middle + at)
        .filter(|at| {
            let apart = |there: &Vec2| there.distance(*at) >= board.dot * 2.0;
            board.at.values().all(apart) && spots.iter().flatten().all(apart)
        })
        .map(Some)
        .collect()
}

/// `placed` with the names `board` put on their left; and of the places kept free,
/// those a call's line runs by, now every function is put, crowded: a line laid out
/// after a place was kept may still run by it, and whoever took it would read as that
/// call's.
fn finish(board: Board, mut placed: Placed) -> Placed {
    let rotation = view_rotation();
    let flat = |at: Vec3| Vec2::new(at.dot(rotation * Vec3::X), at.dot(rotation * Vec3::Y));
    let by_line = |at: Vec2| {
        board.calls.iter().any(|&(from, to)| {
            let (Some(&a), Some(&b)) = (board.at.get(&from), board.at.get(&to)) else {
                return false;
            };
            from != to && reach(a, b, at) < board.dot * 2.0
        })
    };
    let crowded: Vec<(usize, usize)> = (placed.free.iter())
        .flat_map(|(&cluster, spots)| {
            (spots.iter().enumerate())
                .filter(|&(_, &at)| by_line(flat(at)))
                .map(move |(seat, _)| (cluster, seat))
        })
        .collect();
    placed.crowded.extend(crowded);
    placed.left = board.left;
    placed
}

/// Where each of `rooms`' functions goes, the rooms in turn and each one's in order,
/// clear of the lines of `calls` (caller, callee) and of each other, names `chars`
/// characters long; and the places each room leaves free.
pub(super) fn place(
    rooms: &[Room],
    calls: &[(usize, usize)],
    chars: &dyn Fn(usize) -> usize,
) -> Placed {
    let rotation = view_rotation();
    let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
    let flat = |at: Vec3| Vec2::new(at.dot(right), at.dot(up));
    let world = |at: Vec2| right * at.x + up * at.y;
    // As big as the journey is framed: from the middle out to its furthest sphere.
    let span = rooms
        .iter()
        .map(|r| flat(r.center).length() + r.radius)
        .fold(0.0, f32::max)
        .max(1.0);
    let per_point = span / FRAMED;
    let mut board = Board {
        dot: Dot::Focus.radius() + CLEAR * per_point,
        high: NAME_HIGH * per_point,
        gap: NAME_GAP * per_point,
        per_point,
        chars,
        calls,
        start: rooms.iter().find_map(|r| r.middle),
        at: HashMap::new(),
        names: Vec::new(),
        left: HashSet::new(),
        home: rooms
            .iter()
            .flat_map(|r| {
                r.tiles
                    .iter()
                    .chain(&r.middle)
                    .map(|&t| (t, flat(r.center)))
            })
            .collect(),
    };
    let mut placed = Placed::default();
    // Places left free are kept clear as dots are, under keys no function has.
    let mut spare_key = usize::MAX;
    debug_assert!(rooms.iter().flat_map(|r| &r.tiles).all(|&t| t < SPARE_KEYS));
    let mut keep_free = |board: &mut Board, room: &Room, spots: &mut [Option<Vec2>], count| {
        let (middle, reach) = (flat(room.center), atlas::disc(room.radius));
        let free = placed.free.entry(room.cluster).or_default();
        for _ in 0..count {
            // Never on a dot or a place already kept.
            for spot in spots.iter_mut() {
                if spot
                    .is_some_and(|at| board.at.values().any(|t| t.distance(at) < board.dot * 2.0))
                {
                    *spot = None;
                }
            }
            if let Some((at, left)) = best(board, None, spots, (middle, reach)) {
                if board.crowded(at) {
                    placed.crowded.insert((room.cluster, free.len()));
                }
                // Kept for whoever takes it: what is put after keeps off it.
                board.put(spare_key, at, left);
                spare_key -= 1;
                free.push(room.center + world(at - middle));
            }
        }
    };
    let mut left_over = Vec::new();
    for room in rooms {
        let middle = flat(room.center);
        let laid = room.tiles.len() + usize::from(room.middle.is_some());
        let besides = if room.middle.is_some() { 0 } else { BESIDES };
        let reach = atlas::disc(room.radius);
        // Rows no nearer than a name is high, where the sphere has room enough.
        let mut spots: Vec<Option<Vec2>> = spaced(
            (
                laid + besides,
                ((laid + besides) * PLACES_EACH).max(LEAST_PLACES),
            ),
            reach,
            ROW_WIDE,
            board.high + board.dot,
        )
        .into_iter()
        .map(|at| Some(middle + at))
        .collect();
        if let Some(tile) = room.middle {
            // The first place is the middle one.
            let at = spots.first().copied().flatten().unwrap_or(middle);
            board.put(tile, at, false);
            spots[0] = None;
        }
        for &tile in room.tiles.iter().filter(|&&t| Some(t) != room.middle) {
            if let Some((at, left)) = best(&board, Some(tile), &mut spots, (middle, reach)) {
                board.put(tile, at, left);
            }
        }
        // As many free as every sphere round the nucleus keeps, so the spheres laid
        // out after lie the same whoever may come in; the rest once every function
        // is put.
        // Places kept free may sit closer, between the rows of names, where one may
        // be clear of them all when the rows are full.
        let count = (laid + besides.max(room.spare)) * PLACES_EACH;
        spots.extend(closer(&board, &spots, (middle, reach), count));
        keep_free(&mut board, room, &mut spots, besides);
        left_over.push((room, spots));
        for &tile in room.tiles.iter().chain(&room.middle) {
            if let Some(&at) = board.at.get(&tile) {
                placed.points.insert(tile, room.center + world(at - middle));
            }
        }
    }
    for (room, mut spots) in left_over {
        let kept = if room.middle.is_some() { 0 } else { BESIDES };
        let more = room.spare.saturating_sub(kept);
        keep_free(&mut board, room, &mut spots, more);
    }
    finish(board, placed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_one_takes_a_place_on_a_call() {
        let rotation = view_rotation();
        let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
        let flat = |at: Vec3| Vec2::new(at.dot(right), at.dot(up));
        // The start in the middle, a caller far off to its right and a call far off to
        // its left: their lines run across the middle sphere, where places are kept.
        let room = |cluster, center, tiles: Vec<usize>, middle| Room {
            cluster,
            center,
            radius: 8.0,
            tiles,
            middle,
            spare: 8,
        };
        let rooms = [
            room(0, Vec3::ZERO, vec![1, 2], Some(0)),
            room(1, right * 40.0 + up * 3.0, vec![3], None),
            room(2, -right * 40.0 - up * 2.0, vec![4], None),
        ];
        let calls = [(3, 0), (0, 1), (1, 2), (2, 4), (0, 4)];
        let placed = place(&rooms, &calls, &|_| 8);
        let mut open = 0;
        for (&cluster, spots) in &placed.free {
            for (seat, &at) in spots.iter().enumerate() {
                if placed.crowded.contains(&(cluster, seat)) {
                    continue;
                }
                open += 1;
                for &(from, to) in &calls {
                    let (a, b) = (placed.points[&from], placed.points[&to]);
                    let by = reach(flat(a), flat(b), flat(at));
                    assert!(
                        by >= Dot::Focus.radius() * 2.0,
                        "place {seat} of {cluster} is {by} from {from} → {to}"
                    );
                }
            }
        }
        assert!(open > 0, "some places stay open");
    }
}
