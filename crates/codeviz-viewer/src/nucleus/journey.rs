//! A journey: its start, what calls it, and its calls down to the disclosed depth
//! lit where they sit, the rest dimmed; with I on, the callers from off the journey
//! of the function the cursor is on lit where they sit too.

use std::{
    cmp::Reverse,
    collections::{HashMap, HashSet},
};

use codeviz_layout::CallGraph;

use super::{Arrangement, Body, Flow, Link, Nucleus};

/// In a journey, how many of the start's callers are named; the rest fold into a
/// count on their module's name, so the journey stays what reads.
const CALLERS_NAMED: usize = 5;

impl Nucleus {
    /// What a journey from `start` lights: its callers and its calls down to the
    /// disclosed depth, each where it sits, their modules opened, and with I on the
    /// callers from off it into the function the cursor is on.
    pub(super) fn arrange_journey(&self, graph: &CallGraph, start: Option<usize>) -> Arrangement {
        let middle = start
            .and_then(|f| graph.cluster(f))
            .or_else(|| (0..self.counts.len()).max_by_key(|&c| (self.counts[c], Reverse(c))))
            .unwrap_or(0);
        let cluster_of = |tile: usize| graph.cluster(tile).unwrap_or(middle);
        let mut links = start.map_or_else(Vec::new, |f| self.named_calls(graph, f));
        let named = distances(&links, start);
        let folded = start.map_or_else(HashSet::new, |f| self.fold_callers(graph, f, &cluster_of));
        let calls_in = self.outside_calls(graph, &links, start);
        let outside = calls_in.iter().map(|l| l.from).collect();
        links.extend(calls_in);
        let columns = open_up(&links, start, self.counts.len(), &cluster_of);
        let mut balls = self.faint_balls();
        for (cluster, ball) in balls.iter_mut().enumerate() {
            if cluster == middle && start.is_some() {
                ball.body = Body::Nucleus;
            } else if !columns[cluster].is_empty() {
                ball.body = Body::Open;
            }
        }
        Arrangement {
            start,
            middle,
            balls,
            named,
            columns,
            outside,
            folded,
            tubes: Vec::new(),
            links,
        }
    }

    /// Each of `start`'s callers' calls into it, once.
    pub(super) fn calls_into(graph: &CallGraph, start: usize) -> Vec<Link> {
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
        graph: &CallGraph,
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
    fn named_calls(&self, graph: &CallGraph, start: usize) -> Vec<Link> {
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

    /// In a journey with I on, a call from each of its callers off the journey into
    /// the function the cursor is on, or the start when the cursor is on none shown.
    fn outside_calls(&self, graph: &CallGraph, links: &[Link], start: Option<usize>) -> Vec<Link> {
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
}

/// The functions that call `start`, the start itself aside, those nearest the journey
/// first: those that call most of the functions it runs (`on_journey`), then call the
/// start most often, then live in its module. The order the journey's callers are
/// named in, and listed above its start in the panel.
pub(crate) fn rank_callers(
    graph: &CallGraph,
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

/// The functions each module holds that `links` name, the start aside, by cluster,
/// in the order `links` reaches them.
pub(super) fn open_up(
    links: &[Link],
    start: Option<usize>,
    clusters: usize,
    cluster_of: &impl Fn(usize) -> usize,
) -> Vec<Vec<usize>> {
    let mut columns: Vec<Vec<usize>> = vec![Vec::new(); clusters];
    for link in links {
        for tile in [link.from, link.to] {
            let cluster = cluster_of(tile);
            if Some(tile) != start && !columns[cluster].contains(&tile) {
                columns[cluster].push(tile);
            }
        }
    }
    columns
}
