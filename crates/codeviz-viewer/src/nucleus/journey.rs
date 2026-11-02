//! A journey's arrangement: the start's module in the middle with all its functions,
//! and every module the journey enters on a ring round it, clockwise in the order it
//! first enters them, opened to show the functions it names in columns; laid out for
//! every depth at once, so a level more or less only shows or hides. The callers
//! from off the journey of the function the cursor is on (I) take places kept free
//! for them, or their own module beyond the ring, moving nothing else.

use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
};

use bevy::prelude::*;
use codeviz_layout::{ForceGraph, atlas};

use super::{Arrangement, Ball, Body, Flow, Link, Nucleus, view_rotation};
use crate::sizes::module_radius;

/// Rows of names an open module stacks before it starts another column.
pub(super) const ROWS: usize = 8;
/// Between two columns of names in an open module, room for a name.
const COLUMN_GAP: f32 = 11.0;
/// Between two functions named in an open module.
const SPACING: f32 = 2.8;
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
    pub(super) fn arrange_modules(&self, graph: &ForceGraph, start: Option<usize>) -> Arrangement {
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
        let (mut points, mut shown) = self.place_functions(&balls, &laid, middle, &cluster_of);
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
        let outside = self.place_outside(
            &mut balls,
            (&mut points, &mut shown),
            (&calls_in, &laid),
            middle,
            &cluster_of,
        );
        links.extend(calls_in);
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
            folded,
            tubes: Vec::new(),
            links,
        }
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
        for tile in callers.chain(journey) {
            let cluster = cluster_of(tile);
            if Some(tile) != start && cluster != middle && !columns[cluster].contains(&tile) {
                columns[cluster].push(tile);
            }
        }
        columns
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

    /// Puts the outside callers `calls` brings in where they show without moving
    /// anything already placed: inside the nucleus where they sit; in a module with a
    /// place on the ring, in the places it keeps free below the names `reserved` lays
    /// out in it, as many as there are; else in their own module, beyond every sphere
    /// the journey could show, towards what they call, clear of the others put there.
    /// Returns them.
    pub(super) fn place_outside(
        &self,
        balls: &mut [Ball],
        (points, shown): (&mut [Vec3], &mut [bool]),
        (calls, reserved): (&[Link], &[Vec<usize>]),
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
        let mut placed = HashSet::new();
        for (cluster, tiles) in &by_module {
            if *cluster == middle {
                for &tile in tiles {
                    shown[tile] = true;
                    placed.insert(tile);
                }
                continue;
            }
            let ball = balls[*cluster];
            let names = &reserved[*cluster];
            if !names.is_empty() {
                // One the journey could name deeper has its place; the rest take the
                // places kept free below the names, while there are any.
                let mut free = names_grid(names.len(), ball.radius)
                    .into_iter()
                    .skip(names.len());
                for &tile in tiles {
                    let at = if names.contains(&tile) {
                        Some(points[tile])
                    } else {
                        free.next().map(|at| ball.center + at)
                    };
                    if let Some(at) = at {
                        points[tile] = at;
                        shown[tile] = true;
                        placed.insert(tile);
                    }
                }
                if ball.body == Body::Closed && tiles.iter().any(|t| placed.contains(t)) {
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
            let ring = outer + ball.radius;
            // The nearest bearing to that, either way round, clear of those put
            // beyond already.
            let step = (ball.radius / ring).max(0.05);
            #[expect(clippy::cast_precision_loss, reason = "a step count")]
            let center = (0..200)
                .map(|k| {
                    let turn = (k / 2) as f32 * step * if k % 2 == 0 { 1.0 } else { -1.0 };
                    core + (right * (angle + turn).cos() + up * (angle + turn).sin()) * ring
                })
                .find(|&at| {
                    beyond
                        .iter()
                        .all(|&(c, r)| at.distance(c) >= r + ball.radius + MARGIN)
                })
                .unwrap_or(core + right * ring);
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
                placed.insert(tile);
            }
        }
        placed
    }

    /// Where each function is drawn, and whether it is: the nucleus's own where they
    /// sit in it; those an open module could name, by `opened`, in columns down its
    /// middle; the rest folded away at their module's middle.
    fn place_functions(
        &self,
        balls: &[Ball],
        opened: &[Vec<usize>],
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
        for (cluster, column) in opened.iter().enumerate() {
            let grid = names_grid(column.len(), balls[cluster].radius);
            for (&tile, at) in column.iter().zip(grid) {
                points[tile] = balls[cluster].center + at;
                shown[tile] = true;
            }
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

/// Where `count` names sit about their module's middle: a column [`SPACING`] apart,
/// top to bottom, and past [`ROWS`] more columns side by side, [`COLUMN_GAP`] apart,
/// as the usual view sees them; drawn in closer where that would not fit a sphere of
/// `radius`.
pub(super) fn grid(count: usize, radius: f32) -> Vec<Vec3> {
    let columns = count.div_ceil(ROWS).max(1);
    let rows = count.div_ceil(columns).max(1);
    let right = view_rotation() * Vec3::X;
    #[expect(clippy::cast_precision_loss, reason = "short lists")]
    let (high, wide) = (
        (rows as f32 - 1.0) * SPACING / 2.0,
        (columns as f32 - 1.0) * COLUMN_GAP / 2.0,
    );
    let room = atlas::disc(radius);
    let fit = (room / high.hypot(wide).max(f32::EPSILON)).min(1.0);
    #[expect(clippy::cast_precision_loss, reason = "short lists")]
    (0..count)
        .map(|i| {
            let (column, row) = (i / rows, i % rows);
            let x = (column as f32 - (columns as f32 - 1.0) / 2.0) * COLUMN_GAP;
            let y = ((rows as f32 - 1.0) / 2.0 - row as f32) * SPACING;
            (right * x + Vec3::Y * y) * fit
        })
        .collect()
}

/// Where an open module's `count` names sit in its sphere of `radius`, by [`grid`],
/// and after them the [`OUTSIDE_SHOWN`] places it keeps free for its callers from
/// outside the journey, so those come and go moving no name.
pub(super) fn names_grid(count: usize, radius: f32) -> Vec<Vec3> {
    grid(count + OUTSIDE_SHOWN, radius)
}
