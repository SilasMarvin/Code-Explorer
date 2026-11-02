//! Where the calls run: the arrow between two modules, bent round every sphere in its
//! way as the usual view sees it, and each named call along it, from function to
//! function, on a strand of its own.

use bevy::prelude::*;
use codeviz_layout::atlas::sideways;

use super::{Link, Nucleus, Tube, view_rotation};
use crate::sizes::{Dot, tube_radius};

/// Between two named calls running side by side along one arrow.
const STRAND_GAP: f32 = 0.45;
/// Kept clear round a sphere an arrow bends round, before the expanded view's zoom.
const DETOUR_CLEAR: f32 = 3.0;
/// Kept clear round a function a call bends round, before the expanded view's zoom.
const NODE_CLEAR: f32 = 0.5;
/// Most bends an arrow takes round what lies in its way.
pub(super) const DETOUR_BENDS: usize = 4;

impl Nucleus {
    /// Where the arrow from one module to another starts and ends, on their spheres
    /// as drawn now, and how thick it is. An arrow running each way between the same
    /// two modules keeps to its own side.
    pub(crate) fn tube_ends(&self, tube: &Tube) -> Option<(Vec3, Vec3, f32)> {
        let (a, b) = (self.ball(tube.from)?, self.ball(tube.to)?);
        let along = (b.center - a.center).normalize_or_zero();
        if along == Vec3::ZERO {
            return None;
        }
        let radius = tube_radius(tube.calls) * self.zoom();
        let both_ways = self
            .now
            .tubes
            .iter()
            .any(|t| t.from == tube.to && t.to == tube.from);
        let side = if both_ways {
            sideways(along) * (self.half_width(tube) + 0.4 * self.zoom())
        } else {
            Vec3::ZERO
        };
        // Leaving and entering where the spheres' surfaces are, shifted to the side.
        let start = a.center + along * a.radius + side;
        let end = b.center - along * b.radius + side;
        Some((start, end, radius))
    }

    /// How many named calls run along an arrow.
    fn strands(&self, tube: &Tube) -> usize {
        self.now
            .links
            .iter()
            .filter(|l| self.tube_of(l) == Some((tube.from, tube.to)))
            .count()
    }

    /// Half the width the named calls along an arrow take, side by side, or its own
    /// when that is wider.
    fn half_width(&self, tube: &Tube) -> f32 {
        #[expect(clippy::cast_precision_loss, reason = "a short list")]
        let spread = self.strands(tube).saturating_sub(1) as f32 * STRAND_GAP / 2.0;
        (spread * self.zoom()).max(tube_radius(tube.calls) * self.zoom())
    }

    /// The pair of modules a named call runs between, when they differ.
    fn tube_of(&self, link: &Link) -> Option<(usize, usize)> {
        let cluster = |tile: usize| self.module_of.get(tile).copied().flatten();
        let (from, to) = (cluster(link.from)?, cluster(link.to)?);
        (from != to).then_some((from, to))
    }

    /// The path an arrow between modules runs along, from where it leaves one to where
    /// it enters the other, bent round every sphere drawn that lies in its way as the
    /// usual view sees it, and how thick it is.
    pub(crate) fn tube_path(&self, tube: &Tube) -> Option<(Vec<Vec3>, f32)> {
        let (start, end, radius) = self.tube_ends(tube)?;
        let spheres: Vec<(Vec3, f32)> = self
            .balls()
            .filter(|&(c, b)| c != tube.from && c != tube.to && self.shows_body(b.body))
            .map(|(_, b)| (b.center, b.radius))
            .collect();
        Some((
            detour(vec![start, end], &spheres, DETOUR_CLEAR * self.zoom()),
            radius,
        ))
    }

    /// The functions drawn full size whose disc a call should keep off: lit, the
    /// start, the cursor, and the outside callers shown; each with where it is drawn
    /// and how big.
    fn discs_in_way(&self, except: [usize; 2]) -> Vec<(Vec3, f32)> {
        self.now
            .named
            .keys()
            .chain(&self.now.outside)
            .chain(self.now.start.iter())
            .chain(self.cursor.iter())
            .filter(|t| !except.contains(t) && self.placed(**t))
            .filter_map(|&t| Some((self.point(t)?, self.dot_radius(Dot::Lit))))
            .collect()
    }

    /// The path a named call is drawn along: within a module, from one function to
    /// the other; else from the caller to its module's surface, along the arrow
    /// between the two modules, and in to the callee, each call on its own strand
    /// beside the others along the arrow. An end folded away inside its module stops
    /// at the module's surface. Within a module it bends round the other functions
    /// drawn there, so a call never reads as another's.
    pub(crate) fn route(&self, link: &Link) -> Vec<Vec3> {
        let (Some(a), Some(b)) = (self.point(link.from), self.point(link.to)) else {
            return Vec::new();
        };
        let discs = self.discs_in_way([link.from, link.to]);
        let clear = NODE_CLEAR * self.zoom();
        let Some((from, to)) = self.tube_of(link) else {
            return detour(vec![a, b], &discs, clear);
        };
        let tube = self.now.tubes.iter().find(|t| t.from == from && t.to == to);
        let Some((tube, (path, _))) = tube.and_then(|t| Some((t, self.tube_path(t)?))) else {
            return detour(vec![a, b], &discs, clear);
        };
        let (Some(&start), Some(&end)) = (path.first(), path.last()) else {
            return vec![a, b];
        };
        let strand = self
            .now
            .links
            .iter()
            .take_while(|l| *l != link)
            .filter(|l| self.tube_of(l) == Some((from, to)))
            .count();
        let along = (end - start).normalize_or_zero();
        #[expect(clippy::cast_precision_loss, reason = "a short list")]
        let spread =
            (strand as f32 - self.strands(tube).saturating_sub(1) as f32 / 2.0) * STRAND_GAP;
        let offset = sideways(along).cross(along) * spread * self.zoom();
        let mut route = Vec::with_capacity(path.len() + 4);
        if self.placed(link.from) {
            route.extend(detour(vec![a, start + offset], &discs, clear));
            route.pop();
        }
        route.extend(path.iter().map(|&p| p + offset));
        if self.placed(link.to) {
            route.extend(
                detour(vec![end + offset, b], &discs, clear)
                    .into_iter()
                    .skip(1),
            );
        }
        route
    }
}

/// `path` bent round each of `obstacles` (a middle and a radius) it runs across as
/// the usual view sees it, keeping `clear` off it: at the point that runs deepest
/// into one, a bend out past its edge, again for whatever the bent path still runs
/// across, up to [`DETOUR_BENDS`] bends. One holding an end of the path, which no
/// bend can keep off, is left be.
pub(super) fn detour(mut path: Vec<Vec3>, obstacles: &[(Vec3, f32)], clear: f32) -> Vec<Vec3> {
    let back = view_rotation() * Vec3::Z;
    let flat = |v: Vec3| v - back * v.dot(back);
    for _ in 0..DETOUR_BENDS {
        // The segment, the bend that clears it, and how far into the obstacle it ran.
        let mut worst: Option<(usize, Vec3, f32)> = None;
        for (k, pair) in path.windows(2).enumerate() {
            let (a, b) = (pair[0], pair[1]);
            let along = flat(b - a);
            let length = along.length_squared();
            if length < 1e-6 {
                continue;
            }
            for &(middle, radius) in obstacles {
                let reach = radius + clear;
                if flat(a - middle).length() < reach || flat(b - middle).length() < reach {
                    continue;
                }
                let t = flat(middle - a).dot(along) / length;
                if !(0.0..=1.0).contains(&t) {
                    continue;
                }
                let nearest = a + (b - a) * t;
                let off = flat(nearest - middle);
                let into = reach - off.length();
                if into <= 0.0 || worst.is_some_and(|w| w.2 >= into) {
                    continue;
                }
                let out = off
                    .try_normalize()
                    .unwrap_or_else(|| along.cross(back).normalize_or(Vec3::X));
                worst = Some((k, nearest + out * (into + clear * 0.5), into));
            }
        }
        let Some((k, bend, _)) = worst else {
            break;
        };
        path.insert(k + 1, bend);
    }
    path
}
