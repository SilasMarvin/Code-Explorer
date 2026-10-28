//! The flat map with calls in the air: every node stays where the map puts it, on one
//! plane of glass, and the third dimension carries only which way a call runs. Calls
//! into the focus arc over the plane and calls out of it arc under it, so up is
//! upstream and down is downstream; one call further out on either side arcs higher
//! or deeper, and fainter. The camera tilts so over and under separate on screen.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::{
    camera::{ANGLED, OrbitCamera},
    explorer::Explorer,
    graph_view::{CALLEE, CALLER, FURTHER, flow, node_size, orbit_anchor},
    lens::SPECK,
    scene::Tile,
};

/// Calls from the focus that arc, on either side.
pub(crate) const HOPS: u32 = 2;

/// Looking at the map from its near edge, tilted back so what rises off it rises on
/// screen and what hangs under it hangs.
const POSE: (f32, f32) = (0.0, 0.82);

/// Points along each arc.
const SEGMENTS: usize = 40;

/// The glass: a pane under the map, and how far it reaches past the outermost node.
#[derive(Component, Debug)]
pub(crate) struct Glass;

const GLASS_MARGIN: f32 = 6.0;

pub(crate) fn spawn_glass(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        Glass,
        Mesh3d(meshes.add(Rectangle::new(1.0, 1.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgba(0.42, 0.5, 0.68, 0.045),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            double_sided: true,
            cull_mode: None,
            ..default()
        })),
        Transform::default(),
        Visibility::Hidden,
        // Clicks go through the glass to the nodes on it.
        Pickable::IGNORE,
    ));
}

/// Z turns the arcs on and off.
pub(crate) fn toggle(keys: &ButtonInput<KeyCode>, explorer: &mut Explorer) {
    if keys.just_pressed(KeyCode::KeyZ) {
        let on = !explorer.arcs();
        explorer.set_arcs(on);
    }
}

/// Lays the nodes flat on the plane, or lifts them back, when the arcs turn on or off;
/// sizes the glass to the map under them.
pub(crate) fn place(
    explorer: Res<Explorer>,
    mut last: Local<bool>,
    mut nodes: Query<(&Tile, &mut Transform), Without<Glass>>,
    mut glass: Query<(&mut Transform, &mut Visibility), With<Glass>>,
) {
    let on = explorer.arcs();
    if on != *last {
        *last = on;
        for (tile, mut transform) in &mut nodes {
            if let Some(at) = explorer.tile_point(tile.0) {
                transform.translation = at;
            }
        }
    }
    for (mut transform, mut visibility) in &mut glass {
        let wanted = if on {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        visibility.set_if_neq(wanted);
        if !on {
            continue;
        }
        let Some((low, high)) = bounds(&explorer) else {
            continue;
        };
        let middle = (low + high) / 2.0;
        let size = (high - low) + Vec2::splat(GLASS_MARGIN * 2.0);
        // Just under the plane, so a node's lower half reads as under the glass.
        transform.translation = middle.extend(-0.02);
        transform.scale = size.extend(1.0);
    }
}

/// The corners of what shows on the map.
fn bounds(explorer: &Explorer) -> Option<(Vec2, Vec2)> {
    (0..explorer.map().tiles.len())
        .filter(|&t| explorer.visibility(t) >= SPECK)
        .filter_map(|t| explorer.tile_point(t))
        .map(Vec3::truncate)
        .fold(None, |acc, p| match acc {
            None => Some((p, p)),
            Some((low, high)) => Some((low.min(p), high.max(p))),
        })
}

/// What the arcs were last framed for: on or off, the anchor, the focus, the aim and
/// the window's shape.
type Framed = (bool, Option<usize>, Option<usize>, u64, u32);

/// Turns to the tilted pose and frames every arc when the arcs come on, the focus
/// moves, the view is pointed somewhere new, its shape changes or the graph settles;
/// turns back to the usual view when they go off.
pub(crate) fn frame(
    explorer: Res<Explorer>,
    mut last: Local<Option<Framed>>,
    mut moved: Local<u64>,
    mut cameras: Query<(&mut OrbitCamera, &Projection)>,
) {
    let on = explorer.arcs();
    for (mut camera, projection) in &mut cameras {
        let Projection::Perspective(perspective) = projection else {
            continue;
        };
        // The window's shape, in hundredths, so a resize refits.
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a coarse key"
        )]
        let shape = (perspective.aspect_ratio * 100.0).round() as u32;
        let now = (
            on,
            explorer.anchor(),
            explorer.graph_focus(),
            explorer.aim(),
            shape,
        );
        let was = last.replace(now);
        // While the graph settles its nodes move under the arcs: follow them, unless
        // the viewer has taken the camera.
        let settling = std::mem::replace(&mut *moved, explorer.moved()) != explorer.moved();
        let follow = on && settling && !camera.free;
        if !follow && (was == Some(now) || was.is_none() && !on) {
            continue;
        }
        if !on {
            camera.offset = Vec3::ZERO;
            camera.turn_to(ANGLED);
            orbit_anchor(&explorer, &mut camera);
            continue;
        }
        let Some(center) = explorer.anchor().and_then(|t| explorer.tile_point(t)) else {
            continue;
        };
        let points: Vec<Vec3> = links(&explorer)
            .iter()
            .flat_map(|link| {
                [
                    link.from,
                    link.to,
                    link.from.midpoint(link.to).with_z(link.lift),
                ]
            })
            .chain([center])
            .collect();
        let tan_y = (perspective.fov / 2.0).tan();
        let view = View {
            rotation: Quat::from_euler(EulerRot::YXZ, POSE.0, POSE.1, 0.0),
            tan: Vec2::new(tan_y * perspective.aspect_ratio, tan_y),
        };
        let (middle, distance) = view.fit(&points, center, camera.distance);
        camera.turn_to(POSE);
        camera.offset = middle - center;
        camera.glide_to(middle, distance);
    }
}

/// The share of the view, from -1 to 1 either way, that the arcs fill: clear of the
/// edges for their names, and of the legend and the journey docked along the bottom.
const ROOM: Rect = Rect {
    min: Vec2::new(-0.86, -0.66),
    max: Vec2::new(0.86, 0.9),
};

/// A camera turned to `rotation`, seeing `tan` of its distance either side.
#[derive(Debug, Clone, Copy)]
struct View {
    rotation: Quat,
    tan: Vec2,
}

impl View {
    /// Where `point` lands on screen, from -1 to 1 either way, seen from `middle` at
    /// `distance`; `None` behind the camera.
    fn project(self, point: Vec3, middle: Vec3, distance: f32) -> Option<Vec2> {
        let eye = middle + self.rotation * Vec3::Z * distance;
        let local = self.rotation.inverse() * (point - eye);
        let depth = -local.z;
        (depth > 0.1).then(|| local.truncate() / (self.tan * depth))
    }

    /// The point to look at and how far back to stand so that `points` fill [`ROOM`]:
    /// a few rounds of measuring where they land, then moving and pulling back by how
    /// far off they are. Perspective makes near things bigger, so one round is not
    /// enough.
    fn fit(self, points: &[Vec3], start: Vec3, distance: f32) -> (Vec3, f32) {
        let (right, up) = (self.rotation * Vec3::X, self.rotation * Vec3::Y);
        let (mut middle, mut distance) = (start, distance.max(MIN_DISTANCE));
        for _ in 0..24 {
            let landed: Vec<Vec2> = points
                .iter()
                .filter_map(|&p| self.project(p, middle, distance))
                .collect();
            if landed.len() < points.len() {
                // Something is behind the camera: stand further back and look again.
                distance *= 2.0;
                continue;
            }
            let Some(spread) = landed
                .iter()
                .map(|&p| Rect::from_corners(p, p))
                .reduce(|a, b| a.union(b))
            else {
                break;
            };
            let off = spread.center() - ROOM.center();
            middle += (right * off.x * self.tan.x + up * off.y * self.tan.y) * distance;
            let scale = (spread.size() / ROOM.size()).max_element();
            distance = (distance * scale.clamp(0.5, 2.0)).max(MIN_DISTANCE);
        }
        (middle, distance)
    }
}

/// The closest the tilted camera comes.
const MIN_DISTANCE: f32 = 25.0;

/// A call drawn as an arc: from where to where, how high (up, positive) or deep it
/// goes, which way it runs and how many calls from the focus its far end is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Link {
    pub(crate) from: Vec3,
    pub(crate) to: Vec3,
    pub(crate) callee: usize,
    pub(crate) lift: f32,
    pub(crate) upstream: bool,
    pub(crate) hops: u32,
}

/// Which side of the focus a call lies on, and how far: callers of the focus are one
/// up, their callers two; calls the focus makes are one down, what those call two.
pub(crate) fn side(
    upstream: &HashMap<usize, u32>,
    downstream: &HashMap<usize, u32>,
    focus: usize,
    (from, to): (usize, usize),
) -> Option<(bool, u32)> {
    let hops = |side: &HashMap<usize, u32>, tile: usize| {
        if tile == focus {
            Some(0)
        } else {
            side.get(&tile).copied()
        }
    };
    if let (Some(outer), Some(inner)) = (hops(upstream, from), hops(upstream, to))
        && outer == inner + 1
        && outer <= HOPS
    {
        return Some((true, outer));
    }
    if let (Some(inner), Some(outer)) = (hops(downstream, from), hops(downstream, to))
        && outer == inner + 1
        && outer <= HOPS
    {
        return Some((false, outer));
    }
    None
}

/// Every call that arcs, with how high: the first deck at about half the distance to
/// the focus's neighbors, the second twice that, so each deck clears the one inside it.
pub(crate) fn links(explorer: &Explorer) -> Vec<Link> {
    let (Some(focus), Some(reach)) = (explorer.graph_focus(), explorer.reach()) else {
        return Vec::new();
    };
    let Some(center) = explorer.tile_point(focus) else {
        return Vec::new();
    };
    let graph = explorer.graph();
    let near: Vec<f32> = graph
        .callers(focus)
        .chain(graph.callees(focus))
        .filter_map(|t| explorer.tile_point(t))
        .map(|p| p.distance(center))
        .collect();
    let deck = deck_height(&near);
    // A call as long as twice the focus's typical one flies at its deck's full height;
    // shorter ones lower, so a short hop is a hump rather than a spike.
    let span = deck * 5.0;
    graph
        .calls
        .iter()
        .filter(|&&(from, to)| explorer.visibility(from).min(explorer.visibility(to)) >= SPECK)
        .filter_map(|&(from, to)| {
            let (upstream, hops) = side(&reach.upstream, &reach.downstream, focus, (from, to))?;
            let (a, b) = (explorer.tile_point(from)?, explorer.tile_point(to)?);
            #[expect(clippy::cast_precision_loss, reason = "a hop count")]
            let full = deck * (1.0 + 0.8 * (hops - 1) as f32);
            let height = full * (0.45 + 0.55 * (a.distance(b) / span).min(1.0));
            Some(Link {
                from: a,
                to: b,
                callee: to,
                lift: if upstream { height } else { -height },
                upstream,
                hops,
            })
        })
        .collect()
}

/// The node one call from the focus that links `tile`, two calls out, to it: the
/// caller it calls, when upstream, or the callee that calls it.
pub(crate) fn through(explorer: &Explorer, tile: usize, upstream: bool) -> Option<usize> {
    let graph = explorer.graph();
    let focus = explorer.graph_focus()?;
    if upstream {
        graph
            .callees(tile)
            .find(|&next| graph.callers(focus).any(|c| c == next))
    } else {
        graph
            .callers(tile)
            .find(|&before| graph.callees(focus).any(|c| c == before))
    }
}

/// How high the first deck of arcs flies, from how far the focus's neighbors sit
/// from it: high enough to clear the map, low enough to stay on screen.
fn deck_height(distances: &[f32]) -> f32 {
    if distances.is_empty() {
        return 6.0;
    }
    #[expect(clippy::cast_precision_loss, reason = "a small count")]
    let mean = distances.iter().sum::<f32>() / distances.len() as f32;
    (mean * 0.4).clamp(4.0, 30.0)
}

/// The points of an arc from `a` to `b` that rises `lift` off the plane (or sinks, when
/// negative): steep at the ends and broad on top, so each deck reads as a level.
pub(crate) fn arc_points(a: Vec3, b: Vec3, lift: f32) -> Vec<Vec3> {
    (0..=SEGMENTS)
        .map(|step| {
            #[expect(clippy::cast_precision_loss, reason = "a small count")]
            let t = step as f32 / SEGMENTS as f32;
            let rise = (std::f32::consts::PI * t).sin().max(0.0).powf(0.55);
            a.lerp(b, t) + Vec3::Z * lift * rise
        })
        .collect()
}

/// Calls into the focus and their callers over the plane, calls out of it and theirs
/// under it; the rest of the graph's calls lie faint on the plane.
pub(crate) fn draw_arcs(explorer: &Explorer, time: f32, gizmos: &mut Gizmos) {
    let graph = explorer.graph();
    let center = explorer.graph_focus();
    for &(from, to) in &graph.calls {
        let (Some(a), Some(b)) = (explorer.tile_point(from), explorer.tile_point(to)) else {
            continue;
        };
        let shown = explorer.visibility(from).min(explorer.visibility(to));
        if shown < SPECK {
            continue;
        }
        let alpha = if center.is_some() { 0.022 } else { 0.12 };
        gizmos.line(a, b, Color::srgba(0.6, 0.66, 0.8, alpha * shown));
    }
    draw_plane_edge(explorer, gizmos);
    for (index, link) in links(explorer).iter().enumerate() {
        let points = arc_points(link.from, link.to, link.lift);
        let color = match (link.upstream, link.hops) {
            (true, 1) => CALLER.with_alpha(0.95),
            (false, 1) => CALLEE.with_alpha(0.95),
            _ => FURTHER.with_alpha(0.5),
        };
        let n = points.len();
        if link.upstream {
            gizmos.linestrip(points[..n - 1].iter().copied(), color);
        } else {
            // Dashed under the glass, as a drawing shows what lies behind a surface.
            for pair in points[..n - 1].windows(2).step_by(2) {
                gizmos.line(pair[0], pair[1], color);
            }
        }
        // The last stretch stops at the callee's surface, so the head stays visible.
        let tail = points[n - 2];
        let reach = (link.to - tail).normalize_or_zero() * node_size(graph, link.callee) * 0.5;
        let tip = if link.hops == 1 { 1.1 } else { 0.8 };
        gizmos
            .arrow(tail, link.to - reach, color)
            .with_tip_length(tip);
        if link.hops == 1 {
            flow_along(gizmos, &points, color, time, index);
        }
    }
}

/// The glass's rim, and a few faint lines across it, so the plane reads as a surface
/// that arcs leave and return to.
fn draw_plane_edge(explorer: &Explorer, gizmos: &mut Gizmos) {
    let Some((low, high)) = bounds(explorer) else {
        return;
    };
    let (low, high) = (
        low - Vec2::splat(GLASS_MARGIN),
        high + Vec2::splat(GLASS_MARGIN),
    );
    let rim = Color::srgba(0.55, 0.65, 0.85, 0.28);
    let corners = [
        low,
        Vec2::new(high.x, low.y),
        high,
        Vec2::new(low.x, high.y),
        low,
    ];
    gizmos.linestrip(corners.iter().map(|c| c.extend(0.0)), rim);
    let grid = Color::srgba(0.55, 0.65, 0.85, 0.05);
    let lines = 10;
    for line in 1..lines {
        #[expect(clippy::cast_precision_loss, reason = "a small count")]
        let t = line as f32 / lines as f32;
        let x = low.x + (high.x - low.x) * t;
        let y = low.y + (high.y - low.y) * t;
        gizmos.line(Vec3::new(x, low.y, 0.0), Vec3::new(x, high.y, 0.0), grid);
        gizmos.line(Vec3::new(low.x, y, 0.0), Vec3::new(high.x, y, 0.0), grid);
    }
}

/// Dots running along an arc from caller to callee, as along a straight call.
fn flow_along(gizmos: &mut Gizmos, points: &[Vec3], color: Color, time: f32, seed: usize) {
    let stretch = SEGMENTS / 4;
    for start in (0..SEGMENTS).step_by(stretch) {
        let end = (start + stretch).min(points.len() - 1);
        flow(
            gizmos,
            points[start],
            points[end],
            color,
            time,
            seed + start,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arcs_leave_and_land_on_the_plane() {
        let (a, b) = (Vec3::new(-5.0, 0.0, 0.0), Vec3::new(5.0, 2.0, 0.0));
        let up = arc_points(a, b, 4.0);
        assert_eq!(up.first(), Some(&a));
        assert!(up.last().is_some_and(|p| p.distance(b) < 1e-4));
        let top = up.iter().map(|p| p.z).fold(f32::MIN, f32::max);
        assert!((top - 4.0).abs() < 1e-3, "rises as far as asked: {top}");
        let down = arc_points(a, b, -4.0);
        assert!(down.iter().all(|p| p.z <= 1e-4), "sinks under the plane");
    }

    #[test]
    fn the_tilted_view_takes_in_every_arc() {
        let view = View {
            rotation: Quat::from_euler(EulerRot::YXZ, POSE.0, POSE.1, 0.0),
            tan: Vec2::new(0.66, 0.41),
        };
        let points = [
            Vec3::new(-40.0, 30.0, 0.0),
            Vec3::new(35.0, -50.0, 0.0),
            Vec3::new(5.0, 0.0, 18.0),
            Vec3::new(0.0, -10.0, -25.0),
        ];
        let (middle, distance) = view.fit(&points, Vec3::ZERO, 60.0);
        let landed: Vec<Vec2> = points
            .iter()
            .map(|&p| view.project(p, middle, distance).expect("in front"))
            .collect();
        let grown = Rect::from_corners(ROOM.min - 0.02, ROOM.max + 0.02);
        assert!(
            landed.iter().all(|&p| grown.contains(p)),
            "inside: {landed:?}"
        );
        let spread = landed
            .iter()
            .map(|&p| Rect::from_corners(p, p))
            .reduce(|a, b| a.union(b))
            .expect("points");
        assert!(
            (spread.size() / ROOM.size()).max_element() > 0.95,
            "and fills it: {spread:?}"
        );
    }

    #[test]
    fn callers_go_over_and_callees_under() {
        let focus = 0;
        let upstream = HashMap::from([(1, 1), (2, 2), (3, 3)]);
        let downstream = HashMap::from([(4, 1), (5, 2)]);
        let side = |call| side(&upstream, &downstream, focus, call);
        assert_eq!(side((1, 0)), Some((true, 1)), "a caller of the focus");
        assert_eq!(side((2, 1)), Some((true, 2)), "a caller's caller");
        assert_eq!(side((3, 2)), None, "three up is too far");
        assert_eq!(side((0, 4)), Some((false, 1)), "a call the focus makes");
        assert_eq!(side((4, 5)), Some((false, 2)), "what a callee calls");
        assert_eq!(
            side((1, 4)),
            None,
            "neither nearer nor further from the focus"
        );
    }
}
