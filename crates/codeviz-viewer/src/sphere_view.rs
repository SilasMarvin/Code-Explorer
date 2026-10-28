//! The packed view: every module a lit ball, sized by how many functions it holds,
//! with its functions spread over its surface, and the modules packed inside the ball
//! of the crate (or top-level package) that holds them. Nothing moves when the focus
//! does, so where a function sits can be learned. Calls into and out of the focus are
//! drawn as tubes that bend through the middles of the modules they leave and enter,
//! so calls between the same two modules run together; the modules they touch light
//! up and the rest stay dim.

use bevy::{picking::Pickable, prelude::*};
use codeviz_layout::{Ball, Holder};

use crate::{
    explorer::Explorer,
    graph_view::{self, node_size},
    scene::{PALETTE, lane_color},
};

/// Thick lines, for the calls that touch the focus.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct Tubes;

pub(crate) struct SpherePlugin;

impl Plugin for SpherePlugin {
    fn build(&self, app: &mut App) {
        app.init_gizmo_group::<Tubes>()
            .add_systems(Startup, (widen_tubes, spawn_shells))
            .add_systems(Update, (style_shells, draw_rims).chain());
    }
}

fn widen_tubes(mut store: ResMut<GizmoConfigStore>) {
    store.config_mut::<Tubes>().0.line.width = 4.5;
}

/// A module's ball, by its cluster as the graph numbers them.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Shell(usize);

/// How much a module matters to what is focused: holds nothing near it, holds
/// something a few calls away, or holds the focus or what calls it or what it calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Weight {
    Idle,
    Near,
    Touching,
}

/// Each module's weight, by cluster: what it holds, at its nearest to the focus.
pub(crate) fn module_weights(explorer: &Explorer) -> Vec<Weight> {
    let graph = explorer.graph();
    let mut weights = vec![Weight::Idle; graph.cluster_count()];
    let focus = explorer.graph_focus();
    let reach = explorer.reach();
    for tile in 0..explorer.map().tiles.len() {
        let weight = if focus == Some(tile) {
            Weight::Touching
        } else if focus.is_none() {
            Weight::Near
        } else {
            match reach.and_then(|r| graph_view::near(explorer, r, tile)) {
                Some(1) => Weight::Touching,
                Some(_) => Weight::Near,
                None => Weight::Idle,
            }
        };
        if let Some(slot) = graph.cluster(tile).and_then(|c| weights.get_mut(c)) {
            *slot = (*slot).max(weight);
        }
    }
    weights
}

/// A shaded, see-through ball per module, hidden until the packed view shows. A
/// crate gets only an outline, so it gathers its modules without veiling them.
fn spawn_shells(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    explorer: Res<Explorer>,
) {
    let mesh = meshes.add(
        Sphere::new(1.0)
            .mesh()
            .ico(4)
            .unwrap_or_else(|_| Sphere::new(1.0).mesh().uv(48, 32)),
    );
    for (cluster, ball) in explorer.packing().modules.iter().enumerate() {
        let shell = Shell(cluster);
        let material = materials.add(shell_material(shell, Weight::Idle));
        commands.spawn((
            shell,
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material),
            Transform::from_translation(Vec3::from(ball.center))
                .with_scale(Vec3::splat(ball.radius)),
            Visibility::Hidden,
            // Clicks go through to the functions inside.
            Pickable::IGNORE,
        ));
    }
}

/// A module's ball in its color, glassy, fainter the less it matters.
fn shell_material(shell: Shell, weight: Weight) -> StandardMaterial {
    let alpha = match weight {
        Weight::Touching => 0.16,
        Weight::Near => 0.09,
        Weight::Idle => 0.012,
    };
    StandardMaterial {
        base_color: lane_color(shell.0 % PALETTE.len()).with_alpha(alpha),
        alpha_mode: AlphaMode::Blend,
        perceptual_roughness: 0.4,
        reflectance: 0.3,
        ..default()
    }
}

/// Shows the balls while the packed view does, each as bright as its module matters.
fn style_shells(
    explorer: Res<Explorer>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut shells: Query<(&Shell, &MeshMaterial3d<StandardMaterial>, &mut Visibility)>,
) {
    if !explorer.is_changed() {
        return;
    }
    let weights = module_weights(&explorer);
    let shown = if explorer.packed() {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    for (&shell, material, mut visibility) in &mut shells {
        visibility.set_if_neq(shown);
        let weight = weights.get(shell.0).copied().unwrap_or(Weight::Idle);
        if let Some(mut slot) = materials.get_mut(&material.0) {
            *slot = shell_material(shell, weight);
        }
    }
}

/// A crisp edge round each ball as the camera sees it, so a ball reads as one
/// object: bright round the modules that hold the focus and its calls, faint round the
/// rest.
fn draw_rims(
    explorer: Res<Explorer>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut gizmos: Gizmos,
) {
    if !explorer.packed() {
        return;
    }
    let Ok(camera) = cameras.single() else {
        return;
    };
    let eye = camera.translation();
    let packing = explorer.packing();
    let weights = module_weights(&explorer);
    for (cluster, ball) in packing.modules.iter().enumerate() {
        let alpha = match weights.get(cluster) {
            Some(Weight::Touching) => 0.85,
            Some(Weight::Near) => 0.45,
            _ => 0.05,
        };
        let color = lane_color(cluster % PALETTE.len());
        rim(&mut gizmos, eye, ball, color.with_alpha(alpha));
    }
    for holder in packing.holders.iter().filter(|h| shows_holder(h, &weights)) {
        rim(
            &mut gizmos,
            eye,
            &holder.ball,
            Color::srgba(0.6, 0.66, 0.8, 0.1),
        );
    }
}

/// Whether a crate is outlined and named: when it holds more than one module, one of
/// them holding the focus or what calls it or what it calls. The rest go without, so
/// their outlines do not cross the view.
pub(crate) fn shows_holder(holder: &Holder, weights: &[Weight]) -> bool {
    holder.modules.len() > 1
        && holder
            .modules
            .iter()
            .any(|&m| weights.get(m) == Some(&Weight::Touching))
}

/// The outline of `ball` seen from `eye`: the circle where sight lines graze it.
fn rim(gizmos: &mut Gizmos, eye: Vec3, ball: &Ball, color: Color) {
    let center = Vec3::from(ball.center);
    let toward = eye - center;
    let distance = toward.length();
    if distance <= ball.radius {
        return;
    }
    let toward = toward / distance;
    let share = ball.radius / distance;
    let at = center + toward * ball.radius * share;
    let radius = ball.radius * (1.0 - share * share).sqrt();
    gizmos
        .circle(
            Isometry3d::new(at, Quat::from_rotation_arc(Vec3::Z, toward)),
            radius,
            color,
        )
        .resolution(96);
}

/// Each module's ball, for naming modules at their tops.
pub(crate) fn regions(explorer: &Explorer) -> Vec<(usize, Vec3, f32)> {
    explorer
        .packing()
        .modules
        .iter()
        .enumerate()
        .map(|(cluster, ball)| (cluster, Vec3::from(ball.center), ball.radius))
        .collect()
}

/// How far a bundled call bends towards the module middles it runs through: 1 runs
/// through them, 0 is a straight line.
const BUNDLING: f32 = 0.8;
/// Points sampled along each bend of a call.
const SAMPLES: usize = 10;

/// The path of a call from `from` to `to`: out through the middle of the caller's
/// module, through the middles of the crates between, and in through the callee's,
/// as a smooth curve pulled most of the way to those middles.
pub(crate) fn route(explorer: &Explorer, from: usize, to: usize) -> Vec<Vec3> {
    let (Some(a), Some(b)) = (explorer.tile_point(from), explorer.tile_point(to)) else {
        return Vec::new();
    };
    let graph = explorer.graph();
    let packing = explorer.packing();
    let middle = |cluster: usize| packing.modules.get(cluster).map(|m| Vec3::from(m.center));
    let mut controls = vec![a];
    if let (Some(ca), Some(cb)) = (graph.cluster(from), graph.cluster(to)) {
        let (ha, hb) = (packing.holder_of[ca], packing.holder_of[cb]);
        controls.extend(middle(ca));
        if ha != hb {
            controls.push(Vec3::from(packing.holders[ha].ball.center));
            controls.push(Vec3::from(packing.holders[hb].ball.center));
        }
        if cb != ca {
            controls.extend(middle(cb));
        }
    }
    controls.push(b);
    controls.dedup_by(|p, q| p.distance(*q) < 1e-3);
    bundle(&controls, BUNDLING)
}

/// A clamped cubic B-spline through `controls`, each first pulled towards the
/// straight line from first to last by `1 - strength`.
fn bundle(controls: &[Vec3], strength: f32) -> Vec<Vec3> {
    let (Some(&first), Some(&last)) = (controls.first(), controls.last()) else {
        return Vec::new();
    };
    if controls.len() <= 2 {
        return vec![first, last];
    }
    let steps = (controls.len() - 1) as f32;
    let pulled: Vec<Vec3> = controls
        .iter()
        .enumerate()
        .map(|(i, &p)| p * strength + first.lerp(last, i as f32 / steps) * (1.0 - strength))
        .collect();
    // The ends tripled, so the curve starts and ends on them.
    let mut knots = vec![first, first];
    knots.extend(&pulled);
    knots.extend([last, last]);
    let mut points = Vec::new();
    for window in knots.windows(4) {
        let [p0, p1, p2, p3] = [window[0], window[1], window[2], window[3]];
        for sample in 0..SAMPLES {
            let t = sample as f32 / SAMPLES as f32;
            let (t2, t3) = (t * t, t * t * t);
            let point = (p0 * (1.0 - t).powi(3)
                + p1 * (3.0 * t3 - 6.0 * t2 + 4.0)
                + p2 * (-3.0 * t3 + 3.0 * t2 + 3.0 * t + 1.0)
                + p3 * t3)
                / 6.0;
            points.push(point);
        }
    }
    points.push(last);
    points
}

/// `points` cut short by `length` at the end, so an arrow's head stops at the
/// callee's surface.
fn trim_end(points: &[Vec3], mut length: f32) -> Vec<Vec3> {
    let mut points = points.to_vec();
    while points.len() >= 2 && length > 0.0 {
        let n = points.len();
        let (a, b) = (points[n - 2], points[n - 1]);
        let span = a.distance(b);
        if span > length {
            points[n - 1] = b + (a - b) / span * length;
            break;
        }
        length -= span;
        points.pop();
    }
    points
}

/// A call as a tube along `route`, with its head at the callee's surface and dots
/// running from caller to callee.
pub(crate) fn draw_call(
    tubes: &mut Gizmos<Tubes>,
    gizmos: &mut Gizmos,
    route: &[Vec3],
    callee_size: f32,
    color: Color,
    flowing: Option<(f32, usize)>,
) {
    let points = trim_end(route, callee_size * 0.5);
    let n = points.len();
    if n < 2 {
        return;
    }
    tubes.linestrip(points[..n - 1].iter().copied(), color);
    // The head, along the last stretch.
    let tip = points[n - 1];
    let back = points
        .iter()
        .rev()
        .find(|p| p.distance(tip) > 1.2)
        .copied()
        .unwrap_or(points[0]);
    tubes.arrow(back, tip, color).with_tip_length(1.2);
    if let Some((time, seed)) = flowing {
        flow_along(gizmos, &points, color, time, seed);
    }
}

/// Dots running along `points` from first to last, as [`graph_view::flow`] runs them
/// along a straight call.
fn flow_along(gizmos: &mut Gizmos, points: &[Vec3], color: Color, time: f32, seed: usize) {
    let lengths: Vec<f32> = points.windows(2).map(|w| w[0].distance(w[1])).collect();
    let total: f32 = lengths.iter().sum();
    if total < 1.0 {
        return;
    }
    let count = (total / 7.0).ceil().clamp(1.0, 8.0);
    let offset = (seed as f32 * 0.618_034).fract();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 1..=8"
    )]
    for dot in 0..count as u32 {
        let mut along = (time * 5.0 / total + (dot as f32 + offset) / count).fract() * total;
        for (window, &span) in points.windows(2).zip(&lengths) {
            if along <= span && span > 0.0 {
                gizmos.sphere(
                    Isometry3d::from_translation(window[0].lerp(window[1], along / span)),
                    0.2,
                    color.with_alpha((color.alpha() + 0.15).min(1.0)),
                );
                break;
            }
            along -= span;
        }
    }
}

/// The size a node is drawn at, for trimming a call to its surface.
pub(crate) fn callee_size(explorer: &Explorer, tile: usize) -> f32 {
    node_size(explorer.graph(), tile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bundled_call_starts_and_ends_on_its_functions() {
        let controls = [
            Vec3::ZERO,
            Vec3::new(5.0, 5.0, 0.0),
            Vec3::new(10.0, 5.0, 0.0),
            Vec3::new(15.0, 0.0, 0.0),
        ];
        let points = bundle(&controls, BUNDLING);
        assert!(points[0].distance(controls[0]) < 1e-4);
        assert!(points.last().unwrap().distance(controls[3]) < 1e-4);
        let highest = points.iter().map(|p| p.y).fold(0.0, f32::max);
        assert!(highest > 2.0, "bends towards the middles: {highest}");
        assert!(highest < 5.0, "without passing through them: {highest}");
    }

    #[test]
    fn trimming_stops_short_of_the_end() {
        let points = [Vec3::ZERO, Vec3::X * 2.0, Vec3::X * 4.0];
        let trimmed = trim_end(&points, 1.0);
        assert!(trimmed.last().unwrap().distance(Vec3::X * 3.0) < 1e-4);
        assert_eq!(trim_end(&points, 3.0).len(), 2);
    }
}
