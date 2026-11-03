//! A journey's arrangement: the start's module in the middle with all its functions,
//! and every module the journey enters on a ring round it, clockwise in the order it
//! first enters them, opened to show the functions it names spread over it, clear of
//! each other's calls ([`super::places`]); laid out for every depth at once, so a
//! level more or less only shows or hides. The callers
//! from off the journey of the function the cursor is on (I) take places kept free
//! for them, or their own module beyond the ring, moving nothing else: every caller
//! from off the journey into any function it runs has its place worked out once, so
//! a step of the cursor only shows some and hides others.

use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
};

use bevy::prelude::*;
use codeviz_layout::ForceGraph;

use super::{
    Arrangement, Ball, Body, Flow, Link, Nucleus,
    places::{self, Placed, Room},
    view_rotation,
};
use crate::sizes::{Dot, module_radius};

/// Between two functions in a sphere, the least for two of the biggest dots, the
/// cursor's, to show apart: closer, every dot of the module is drawn smaller.
const DOT_ROOM: f32 = 2.0 * Dot::Focus.radius() + 0.2;
/// Between the nucleus and a module in the near ring, for its arrows to show.
const NEAR_GAP: f32 = 15.0;
/// Space kept clear between two spheres.
const MARGIN: f32 = 6.0;
/// Between the ring round the nucleus and a module beyond it holding only calls in
/// from outside the journey.
const OUTER_GAP: f32 = 12.0;
/// How many places an open module keeps free below its names for callers from off
/// the journey (I), so they come and go moving nothing.
pub(super) const OUTSIDE_SHOWN: usize = 3;
/// In a journey, how many of the start's callers are named; the rest fold into a
/// count on their module's name, so the journey stays what reads.
const CALLERS_NAMED: usize = 5;

impl Nucleus {
    /// Everything placed around `start`, as the module view has it.
    pub(super) fn arrange_modules(
        &mut self,
        graph: &ForceGraph,
        start: Option<usize>,
    ) -> Arrangement {
        let clusters = self.homes.len();
        let middle = start
            .and_then(|f| graph.cluster(f))
            .or_else(|| (0..clusters).max_by_key(|&c| (self.counts[c], Reverse(c))))
            .unwrap_or(0);
        let cluster_of = |tile: usize| graph.cluster(tile).unwrap_or(middle);
        // Laid out for every depth, so showing a level more or less only draws or
        // stops drawing, and moves nothing.
        let mut links = start.map_or_else(Vec::new, |f| self.named_calls(graph, f));
        let every = links.clone();
        let named = distances(&links, start);
        let folded = start.map_or_else(HashSet::new, |f| self.fold_callers(graph, f, &cluster_of));
        let laid = self.reserve(&every, start, clusters, middle, &cluster_of);
        let columns = open_up(&links, start, clusters, middle, &cluster_of);

        // What the journey does not run through stays where the map has it, unseen,
        // so leaving the journey moves none of it.
        let mut balls: Vec<Ball> = self
            .atlas
            .balls
            .iter()
            .map(|b| Ball {
                body: Body::Closed,
                ..*b
            })
            .collect();
        balls[middle] = Ball {
            center: Vec3::ZERO,
            radius: module_radius(self.counts[middle]),
            body: Body::Nucleus,
        };
        for (cluster, column) in laid.iter().enumerate() {
            if cluster != middle && !column.is_empty() {
                balls[cluster].body = Body::Open;
            }
        }
        let order = self.ring_order(&every, middle, &cluster_of);
        place_ring(&mut balls, &order, middle);
        let (mut points, mut shown) = self.place_functions(&balls, middle, &cluster_of);
        // The start's own module names what the journey runs in it at any depth round
        // the start, and each module round it what it could ever name, spread over the
        // sphere clear of each other's calls; the rest of the start's module stays
        // where the map has it, faint.
        let placed = self.spread_out(&balls, (start, middle), &laid, &every, &cluster_of);
        let mut spaced: HashMap<usize, Vec<Vec3>> = placed.free.clone();
        for (&tile, &point) in &placed.points {
            points[tile] = point;
            shown[tile] = true;
            spaced.entry(cluster_of(tile)).or_default().push(point);
        }
        let dots: HashMap<usize, f32> = spaced
            .iter()
            .map(|(&cluster, at)| (cluster, spaced_dots(at)))
            .collect();
        // What only a deeper level names keeps its place, undrawn.
        for (cluster, column) in laid.iter().enumerate() {
            for tile in column.iter().filter(|t| !columns[cluster].contains(t)) {
                shown[*tile] = false;
            }
            if cluster != middle && columns[cluster].is_empty() {
                balls[cluster].body = Body::Closed;
            }
        }
        let calls_in = self.outside_calls(graph, &links, start);
        let outside = self.show_outside(
            graph,
            (&mut balls, &mut points, &mut shown),
            (&calls_in, &laid, &placed),
            (start, middle),
            &cluster_of,
        );
        links.extend(calls_in);
        let aside = self.place_aside(
            &mut balls,
            (&mut points, &mut shown),
            (&laid, &placed, &outside),
            middle,
            &cluster_of,
        );
        Arrangement {
            start,
            middle,
            balls,
            points,
            shown,
            named,
            columns,
            ring: order,
            outside,
            aside,
            folded,
            tubes: Vec::new(),
            links,
            dots,
        }
    }

    /// Where the journey's functions go in their spheres, worked out once for this
    /// start and journey: the start's module's `laid` round the start, then each
    /// module round it, as [`places::place`] spreads them; and the places each of those
    /// keeps free for callers from off the journey.
    fn spread_out(
        &mut self,
        balls: &[Ball],
        (start, middle): (Option<usize>, usize),
        laid: &[Vec<usize>],
        every: &[Link],
        cluster_of: &impl Fn(usize) -> usize,
    ) -> std::sync::Arc<Placed> {
        // Into the start, and the journey's at every depth.
        let mut calls: Vec<(usize, usize)> = every
            .iter()
            .filter(|l| l.flow == Flow::Into && l.from != l.to)
            .map(|l| (l.from, l.to))
            .collect();
        calls.extend(self.trail.reach.iter().copied());
        let key = (start, calls.clone());
        if let Some((done, placed)) = &self.spread
            && *done == key
        {
            return placed.clone();
        }
        let mut rooms = Vec::new();
        if let Some(start) = start {
            let tiles: Vec<usize> = self
                .naming_order(every)
                .into_iter()
                .filter(|&t| t != start && cluster_of(t) == middle)
                .fold(Vec::new(), |mut named, t| {
                    if !named.contains(&t) {
                        named.push(t);
                    }
                    named
                });
            rooms.push(Room {
                cluster: middle,
                center: balls[middle].center,
                radius: balls[middle].radius,
                tiles,
                middle: Some(start),
                spare: 0,
            });
        }
        for &cluster in &self.ring_order(every, middle, cluster_of) {
            if !laid[cluster].is_empty() {
                rooms.push(Room {
                    cluster,
                    center: balls[cluster].center,
                    radius: balls[cluster].radius,
                    tiles: laid[cluster].clone(),
                    middle: None,
                    spare: OUTSIDE_SHOWN,
                });
            }
        }
        let chars = |tile: usize| self.name_chars.get(tile).copied().unwrap_or(0);
        let placed = std::sync::Arc::new(places::place(&rooms, &calls, &chars));
        self.spread = Some((key, placed.clone()));
        placed
    }

    /// Each of `start`'s callers' calls into it, once.
    pub(super) fn calls_into(graph: &ForceGraph, start: usize) -> Vec<Link> {
        graph
            .callers(start)
            .map(|from| Link {
                from,
                to: start,
                flow: Flow::Into,
            })
            .collect()
    }

    /// Of the callers of `start`, the journey's start, those past the few nearest the
    /// journey ([`rank_callers`]) fold into a count on their module's name. Worked
    /// out from every caller and the journey at every depth, so a new depth or the
    /// cursor never changes which.
    fn fold_callers(
        &self,
        graph: &ForceGraph,
        start: usize,
        cluster_of: &impl Fn(usize) -> usize,
    ) -> HashSet<usize> {
        let on_journey = |tile: usize| self.trail.whole.contains_key(&tile);
        // One the journey runs too is named as the journey's.
        let mut callers: Vec<usize> = rank_callers(graph, start, on_journey, cluster_of)
            .into_iter()
            .filter(|&t| !on_journey(t))
            .collect();
        // Folding one or two away would save nothing.
        if callers.len() <= CALLERS_NAMED + 2 {
            return HashSet::new();
        }
        callers.split_off(CALLERS_NAMED).into_iter().collect()
    }

    /// The calls named round `start`: each of its callers' into it, then each of the
    /// open journey's from it, in the journey's order.
    fn named_calls(&self, graph: &ForceGraph, start: usize) -> Vec<Link> {
        let mut links = Self::calls_into(graph, start);
        for call in &self.trail.calls {
            if call.from != call.to && !links.iter().any(|l| l.from == call.from && l.to == call.to)
            {
                links.push(Link {
                    from: call.from,
                    to: call.to,
                    flow: Flow::journey(call.level),
                });
            }
        }
        links
    }

    /// Every function below the start at any depth, in the order the journey first
    /// runs it, then any `links` name that the trail's order leaves out.
    fn journey_order<'a>(&'a self, links: &'a [Link]) -> impl Iterator<Item = usize> + 'a {
        self.trail.order.iter().copied().chain(
            links
                .iter()
                .filter(|l| matches!(l.flow, Flow::Journey(_)))
                .map(|l| l.to),
        )
    }

    /// The functions each module outside the middle one could ever name in this
    /// journey, by cluster: the start's callers in `links` first, then everything the
    /// journey runs below the start at any depth, the shallowest first and, level by level,
    /// in the order the journey first runs them. Each module is laid out for all of
    /// them once, so a level more or less shows or hides names in place, and the
    /// names a shallower depth shows come first, at the top.
    fn reserve(
        &self,
        links: &[Link],
        start: Option<usize>,
        clusters: usize,
        middle: usize,
        cluster_of: &impl Fn(usize) -> usize,
    ) -> Vec<Vec<usize>> {
        let mut columns: Vec<Vec<usize>> = vec![Vec::new(); clusters];
        for tile in self.naming_order(links) {
            let cluster = cluster_of(tile);
            if Some(tile) != start && cluster != middle && !columns[cluster].contains(&tile) {
                columns[cluster].push(tile);
            }
        }
        columns
    }

    /// The callers of the start in `links`, then everything the journey runs below
    /// it at any depth, the shallowest first and, level by level, in the order the
    /// journey first runs them: the order a module names them in.
    fn naming_order(&self, links: &[Link]) -> Vec<usize> {
        let callers = links
            .iter()
            .filter(|l| l.flow == Flow::Into)
            .map(|l| l.from);
        let mut journey: Vec<usize> = Vec::new();
        for tile in self.journey_order(links) {
            if !journey.contains(&tile) {
                journey.push(tile);
            }
        }
        // A stable sort: level by level, in the journey's order within each.
        journey.sort_by_key(|t| self.trail.whole.get(t).copied().unwrap_or(u32::MAX));
        callers.chain(journey).collect()
    }

    /// The modules round the nucleus, in order: those the journey enters at any
    /// depth, in the order it first enters them, the shallower first; then those
    /// holding only callers of the start, from `links`. The same at every depth,
    /// collapsed or expanded.
    fn ring_order(
        &self,
        links: &[Link],
        middle: usize,
        cluster_of: &impl Fn(usize) -> usize,
    ) -> Vec<usize> {
        let callers = links
            .iter()
            .filter(|l| l.flow == Flow::Into)
            .map(|l| l.from);
        // Level by level, as the panel's rows run, so the ring reads clockwise as the
        // first row numbers its calls, and a module first entered deeper comes after.
        let mut journey: Vec<usize> = self.journey_order(links).collect();
        journey.sort_by_key(|t| self.trail.whole.get(t).copied().unwrap_or(u32::MAX));
        let mut order: Vec<usize> = Vec::new();
        for tile in journey.into_iter().chain(callers) {
            let cluster = cluster_of(tile);
            if cluster != middle && !order.contains(&cluster) {
                order.push(cluster);
            }
        }
        order
    }
    /// In a journey with I on, a call from each of its callers off the journey into
    /// the function the cursor is on, or the start when the cursor is on none shown.
    fn outside_calls(&self, graph: &ForceGraph, links: &[Link], start: Option<usize>) -> Vec<Link> {
        let Some(start) = start.filter(|_| self.outside && !self.browse) else {
            return Vec::new();
        };
        let shown: HashSet<usize> = links
            .iter()
            .flat_map(|l| [l.from, l.to])
            .chain([start])
            .collect();
        let to = self.cursor.filter(|c| shown.contains(c)).unwrap_or(start);
        let mut callers: Vec<usize> = graph.callers(to).filter(|c| !shown.contains(c)).collect();
        callers.sort_unstable();
        callers.dedup();
        callers
            .into_iter()
            .map(|from| Link {
                from,
                to,
                flow: Flow::Outside,
            })
            .collect()
    }

    /// Shows the outside callers `calls` brings in, each where it goes among every
    /// caller from off the journey into any function it runs at any depth, so the
    /// places are the same wherever the cursor is: a step of the cursor shows some and
    /// hides others, and moves none. Returns those shown.
    fn show_outside(
        &self,
        graph: &ForceGraph,
        (balls, points, shown): (&mut [Ball], &mut [Vec3], &mut [bool]),
        (calls, reserved, placed): (&[Link], &[Vec<usize>], &Placed),
        (start, middle): (Option<usize>, usize),
        cluster_of: &impl Fn(usize) -> usize,
    ) -> HashSet<usize> {
        let Some(start) = start.filter(|_| !calls.is_empty()) else {
            return HashSet::new();
        };
        let mut every = self.every_outside_call(graph, start);
        // Those the journey runs deeper than shown take their own places.
        every.extend(calls.iter().copied());
        every.sort_unstable_by_key(|l| (l.from, l.to));
        every.dedup_by_key(|l| (l.from, l.to));
        let (mut all_balls, mut all_points) = (balls.to_vec(), points.to_vec());
        let mut all_shown = shown.to_vec();
        let taken = self.place_outside(
            &mut all_balls,
            (&mut all_points, &mut all_shown),
            (&every, reserved, placed),
            middle,
            cluster_of,
        );
        let mut outside = HashSet::new();
        for call in calls {
            let tile = call.from;
            if !taken.contains(&tile) {
                continue;
            }
            points[tile] = all_points[tile];
            shown[tile] = true;
            outside.insert(tile);
            let cluster = cluster_of(tile);
            if cluster != middle {
                balls[cluster] = all_balls[cluster];
            }
        }
        outside
    }

    /// Every call into a function the journey runs at any depth, or its start, from
    /// one it does not run and that does not call the start: whatever I could bring in,
    /// wherever the cursor is.
    fn every_outside_call(&self, graph: &ForceGraph, start: usize) -> Vec<Link> {
        let on = |tile: usize| tile == start || self.trail.whole.contains_key(&tile);
        let callers_of_start: HashSet<usize> = graph.callers(start).collect();
        let mut targets: Vec<usize> = self.trail.whole.keys().copied().chain([start]).collect();
        targets.sort_unstable();
        targets
            .into_iter()
            .flat_map(|to| {
                graph.callers(to).map(move |from| Link {
                    from,
                    to,
                    flow: Flow::Outside,
                })
            })
            .filter(|l| !on(l.from) && !callers_of_start.contains(&l.from))
            .collect()
    }

    /// Puts the outside callers `calls` brings in where they show without moving
    /// anything already placed: inside the nucleus where they sit; in a module with a
    /// place on the ring, in the places it keeps free among the names `reserved` lays
    /// out in it (`placed`), as many as there are; else in their own module, beyond
    /// every sphere the journey could show, towards what they call, clear of the
    /// others put there. Returns them.
    pub(super) fn place_outside(
        &self,
        balls: &mut [Ball],
        (points, shown): (&mut [Vec3], &mut [bool]),
        (calls, reserved, placed): (&[Link], &[Vec<usize>], &Placed),
        middle: usize,
        cluster_of: &impl Fn(usize) -> usize,
    ) -> HashSet<usize> {
        let mut by_module: Vec<(usize, Vec<usize>)> = Vec::new();
        for call in calls {
            let cluster = cluster_of(call.from);
            match by_module.iter_mut().find(|(c, _)| *c == cluster) {
                Some((_, tiles)) if !tiles.contains(&call.from) => tiles.push(call.from),
                Some(_) => {}
                None => by_module.push((cluster, vec![call.from])),
            }
        }
        by_module.sort_unstable_by_key(|(cluster, _)| *cluster);
        let core = balls[middle].center;
        // Past every sphere the journey could show at any depth, so the same callers
        // go to the same places at every depth.
        let outer = (0..balls.len())
            .filter(|&c| c == middle || !reserved[c].is_empty())
            .map(|c| balls[c].center.distance(core) + balls[c].radius)
            .fold(0.0, f32::max)
            + OUTER_GAP;
        let rotation = view_rotation();
        let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
        let mut beyond: Vec<(Vec3, f32)> = Vec::new();
        let mut put = HashSet::new();
        for (cluster, tiles) in &by_module {
            if *cluster == middle {
                for &tile in tiles {
                    shown[tile] = true;
                    put.insert(tile);
                }
                continue;
            }
            let ball = balls[*cluster];
            let names = &reserved[*cluster];
            if !names.is_empty() {
                // One the journey could name deeper has its place; the rest take the
                // places kept free among the names, while there are any.
                let mut free = placed.free.get(cluster).into_iter().flatten().copied();
                for &tile in tiles {
                    let at = if names.contains(&tile) {
                        Some(points[tile])
                    } else {
                        free.next()
                    };
                    if let Some(at) = at {
                        points[tile] = at;
                        shown[tile] = true;
                        put.insert(tile);
                    }
                }
                if ball.body == Body::Closed && tiles.iter().any(|t| put.contains(t)) {
                    balls[*cluster].body = Body::Open;
                }
                continue;
            }
            // Towards the functions on the journey it calls, as the usual view sees them.
            let toward: Vec3 = calls
                .iter()
                .filter(|c| tiles.contains(&c.from))
                .map(|c| points[c.to] - core)
                .sum();
            let angle = toward.dot(up).atan2(toward.dot(right));
            let center = beyond_at(core, angle, outer + ball.radius, ball.radius, &beyond);
            beyond.push((center, ball.radius));
            // Faint, as everything brought in from outside the journey is.
            balls[*cluster] = Ball {
                center,
                radius: ball.radius,
                body: Body::Faint,
            };
            for &tile in tiles {
                points[tile] = center + self.offsets[tile];
                shown[tile] = true;
                put.insert(tile);
            }
        }
        put
    }

    /// Draws the function set aside (`self.aside`) where it shows without moving
    /// anything else, when the journey draws it nowhere already: in a module round
    /// the nucleus, in the first place it keeps free that no outside caller has
    /// taken; in a module beyond the ring already, where it sits in it; else in its
    /// own module beyond every sphere shown, towards where the map has it. Returns it
    /// when placed so.
    pub(super) fn place_aside(
        &self,
        balls: &mut [Ball],
        (points, shown): (&mut [Vec3], &mut [bool]),
        (reserved, placed, outside): (&[Vec<usize>], &Placed, &HashSet<usize>),
        middle: usize,
        cluster_of: &impl Fn(usize) -> usize,
    ) -> Option<usize> {
        let tile = self.aside.filter(|&t| !shown[t])?;
        let cluster = cluster_of(tile);
        let ball = balls[cluster];
        let names = &reserved[cluster];
        if !names.is_empty() {
            let taken: Vec<Vec3> = outside
                .iter()
                .filter(|&&t| cluster_of(t) == cluster)
                .map(|&t| points[t])
                .collect();
            let at = placed
                .free
                .get(&cluster)
                .into_iter()
                .flatten()
                .copied()
                .find(|at| taken.iter().all(|t| t.distance(*at) > 1e-3))?;
            points[tile] = at;
            if ball.body == Body::Closed {
                balls[cluster].body = Body::Open;
            }
        } else if ball.body == Body::Faint {
            points[tile] = ball.center + self.offsets[tile];
        } else {
            let core = balls[middle].center;
            let shown_balls: Vec<(Vec3, f32)> = balls
                .iter()
                .filter(|b| b.body != Body::Closed)
                .map(|b| (b.center, b.radius))
                .collect();
            let outer = shown_balls
                .iter()
                .map(|&(c, r)| c.distance(core) + r)
                .fold(0.0, f32::max)
                + OUTER_GAP;
            let rotation = view_rotation();
            let toward = self.atlas.balls[cluster].center - self.atlas.balls[middle].center;
            let angle = toward
                .dot(rotation * Vec3::Y)
                .atan2(toward.dot(rotation * Vec3::X));
            let center = beyond_at(core, angle, outer + ball.radius, ball.radius, &shown_balls);
            balls[cluster] = Ball {
                center,
                radius: ball.radius,
                body: Body::Faint,
            };
            points[tile] = center + self.offsets[tile];
        }
        shown[tile] = true;
        Some(tile)
    }

    /// Where each function is drawn before the journey's are spread out, and whether
    /// it is: the nucleus's own where they sit in it; the rest folded away at their
    /// module's middle.
    fn place_functions(
        &self,
        balls: &[Ball],
        middle: usize,
        cluster_of: &impl Fn(usize) -> usize,
    ) -> (Vec<Vec3>, Vec<bool>) {
        let tiles = self.offsets.len();
        let mut points: Vec<Vec3> = (0..tiles).map(|t| balls[cluster_of(t)].center).collect();
        let mut shown = vec![false; tiles];
        for tile in (0..tiles).filter(|&t| cluster_of(t) == middle) {
            points[tile] = balls[middle].center + self.offsets[tile];
            shown[tile] = true;
        }
        (points, shown)
    }
}

/// The functions that call `start`, the start itself aside, those nearest the journey
/// first: those that call most of the functions it runs (`on_journey`), then call the
/// start most often, then live in its module. The order the journey's callers are
/// named in, and listed above its start in the panel.
pub(crate) fn rank_callers(
    graph: &ForceGraph,
    start: usize,
    on_journey: impl Fn(usize) -> bool,
    cluster_of: impl Fn(usize) -> usize,
) -> Vec<usize> {
    let mut callers: Vec<usize> = graph.callers(start).filter(|&t| t != start).collect();
    callers.sort_unstable();
    callers.dedup();
    let home = cluster_of(start);
    callers.sort_by_cached_key(|&caller| {
        let into_journey: HashSet<usize> =
            graph.callees(caller).filter(|&t| on_journey(t)).collect();
        let sites = graph
            .sequence(caller)
            .iter()
            .filter(|&&t| t == start)
            .count();
        (
            Reverse(into_journey.len()),
            Reverse(sites),
            cluster_of(caller) != home,
            caller,
        )
    });
    callers
}

/// How many calls each function `links` names is from `start`: its shallowest level
/// in the journey, capped at [`SHADES`], even when it calls the start back; else -1
/// for a caller; the start left out.
pub(super) fn distances(links: &[Link], start: Option<usize>) -> HashMap<usize, i32> {
    let mut named: HashMap<usize, i32> = HashMap::new();
    for link in links {
        let (tile, level) = match link.flow {
            Flow::Into => (link.from, -1),
            #[expect(clippy::cast_possible_wrap, reason = "a small level")]
            Flow::Journey(level) => (link.to, level as i32),
            // An outside caller is shown, not named.
            Flow::Outside | Flow::Inner => continue,
        };
        if Some(tile) == start {
            continue;
        }
        let at = named.entry(tile).or_insert(level);
        if *at < 0 || (level > 0 && level < *at) {
            *at = level;
        }
    }
    named
}

/// The functions each module outside the middle one opens to name, by cluster, in the
/// order `links` reaches them.
pub(super) fn open_up(
    links: &[Link],
    start: Option<usize>,
    clusters: usize,
    middle: usize,
    cluster_of: &impl Fn(usize) -> usize,
) -> Vec<Vec<usize>> {
    let mut columns: Vec<Vec<usize>> = vec![Vec::new(); clusters];
    for link in links {
        for tile in [link.from, link.to] {
            let cluster = cluster_of(tile);
            if Some(tile) != start && cluster != middle && !columns[cluster].contains(&tile) {
                columns[cluster].push(tile);
            }
        }
    }
    columns
}

/// Where a sphere of `radius` goes `ring` from `core`, at the nearest bearing to
/// `angle` as the usual view sees it, either way round, clear of the spheres `taken`.
fn beyond_at(core: Vec3, angle: f32, ring: f32, radius: f32, taken: &[(Vec3, f32)]) -> Vec3 {
    let rotation = view_rotation();
    let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
    let step = (radius / ring).max(0.05);
    #[expect(clippy::cast_precision_loss, reason = "a step count")]
    (0..200)
        .map(|k| {
            let turn = (k / 2) as f32 * step * if k % 2 == 0 { 1.0 } else { -1.0 };
            core + (right * (angle + turn).cos() + up * (angle + turn).sin()) * ring
        })
        .find(|&at| {
            taken
                .iter()
                .all(|&(c, r)| at.distance(c) >= r + radius + MARGIN)
        })
        .unwrap_or(core + right * ring)
}

/// Stands the modules in `order` on a ring round the nucleus, `middle`, clockwise
/// from the top as the usual view sees it, each as big as it is, the ring just wide
/// enough to hold them all with room between.
fn place_ring(balls: &mut [Ball], order: &[usize], middle: usize) {
    let radii: Vec<f32> = order.iter().map(|&c| balls[c].radius).collect();
    let widest = radii.iter().copied().fold(0.0, f32::max);
    let least = balls[middle].radius + NEAR_GAP + widest;
    let (angles, ring) = clockwise(&radii, least, MARGIN);
    let rotation = view_rotation();
    let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
    for (&cluster, angle) in order.iter().zip(angles) {
        balls[cluster].center = (right * angle.cos() + up * angle.sin()) * ring;
    }
}

/// Angles round a ring, clockwise from the top, for spheres as big as `radii` in
/// order, `margin` apart at least and the room left shared evenly between them; and
/// the ring's radius, `least` or more when they need it.
pub(super) fn clockwise(radii: &[f32], least: f32, margin: f32) -> (Vec<f32>, f32) {
    let widths: Vec<f32> = radii.iter().map(|r| 2.0 * r + margin).collect();
    let total: f32 = widths.iter().sum();
    let ring = least.max(total / std::f32::consts::TAU);
    #[expect(clippy::cast_precision_loss, reason = "a short list")]
    let spare = (std::f32::consts::TAU * ring - total).max(0.0) / widths.len().max(1) as f32;
    let mut along = 0.0;
    let mut angles = Vec::with_capacity(widths.len());
    for (index, width) in widths.iter().enumerate() {
        if index > 0 {
            along += (widths[index - 1] + width) / 2.0 + spare;
        }
        angles.push(std::f32::consts::FRAC_PI_2 - along / ring);
    }
    (angles, ring)
}

/// How big, against their role's size, the dots at `places` in one sphere are
/// drawn: smaller where two sit nearer than [`DOT_ROOM`], so they keep apart at any
/// zoom.
pub(super) fn spaced_dots(places: &[Vec3]) -> f32 {
    let nearest = places
        .iter()
        .enumerate()
        .flat_map(|(i, a)| places[i + 1..].iter().map(move |b| a.distance(*b)))
        .fold(f32::INFINITY, f32::min);
    (nearest / DOT_ROOM).min(1.0)
}
