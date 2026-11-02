//! How functions look: each node's color and size, the rings round the focus, the
//! cursor and what the pointer is on, and the keys that frame the view. The focus is
//! the center of attention; the pointer only rings the node it is over, so passing
//! over the graph does not repaint it. A node's fill says where it lives (its module);
//! what is near the focus is bright, fading with each call further.

use std::collections::HashMap;

use bevy::prelude::*;
use codeviz_layout::{ForceGraph, Reach};

use crate::{
    camera::OrbitCamera,
    explorer::{Explorer, Side},
    idle::Awake,
    nucleus,
    scene::{PALETTE, Tile, lane_color},
};

/// Calls into the focus.
pub(crate) const CALLER: Color = Color::srgb(0.74, 0.52, 1.0);
/// Calls out of the focus.
pub(crate) const CALLEE: Color = Color::srgb(0.3, 0.72, 1.0);
/// Calls of the journey past the focus's own: one color for every level below the
/// first, so blue only ever means the focus's own calls.
pub(crate) const DEEPER: Color = Color::srgb(1.0, 0.7, 0.22);
/// The journey's cursor, where it is not the focus: its node, its name and the call
/// into it. White stays the focus's.
pub(crate) const CURSOR: Color = Color::srgb(1.0, 0.36, 0.72);
/// Calls further up from the focus, which do not touch it: an arrow, but no color, so
/// violet only ever means into the focus.
pub(crate) const FURTHER: Color = Color::srgb(0.82, 0.85, 0.92);

/// Calls away from the focus that still get their own shade; further is drawn as this.
const FARTHEST_SHADE: u32 = 3;

/// How a node is drawn right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum NodeLook {
    /// The center of attention.
    Focus,
    /// Gathered round the focus's module, the journey's cursor where it is not the
    /// middle.
    Cursor,
    /// Calls into the focus or called from it, this many calls away (capped at
    /// [`FARTHEST_SHADE`]), in its module's color.
    Near(usize, u32),
    /// Unrelated to the focus.
    Faded(usize),
    /// Anything while nothing is focused, in its container's color.
    Plain(usize),
}

/// How bright a dimmed node is, against its full brightness.
const DIM: f32 = 0.12 + 0.88 / 3.0;

/// Every look, in full and dimmed.
pub(crate) fn node_materials(
    materials: &mut Assets<StandardMaterial>,
) -> HashMap<(NodeLook, bool), Handle<StandardMaterial>> {
    let mut table = HashMap::new();
    for (look, material) in node_looks() {
        table.insert((look, false), materials.add(material.clone()));
        table.insert((look, true), materials.add(dimmed(&material, DIM)));
    }
    table
}

/// `material` at `share` of its brightness: darker, and fainter where it is see-through.
fn dimmed(material: &StandardMaterial, share: f32) -> StandardMaterial {
    let base = LinearRgba::from(material.base_color);
    let alpha = if material.alpha_mode == AlphaMode::Blend {
        base.alpha * share
    } else {
        base.alpha
    };
    StandardMaterial {
        base_color: Color::from(LinearRgba::new(
            base.red * share,
            base.green * share,
            base.blue * share,
            alpha,
        )),
        emissive: material.emissive * share,
        ..material.clone()
    }
}

/// How each look is drawn at full brightness.
fn node_looks() -> HashMap<NodeLook, StandardMaterial> {
    let glow = |color: Color, strength: f32| StandardMaterial {
        base_color: color,
        emissive: LinearRgba::from(color) * strength,
        ..default()
    };
    let mut table = HashMap::new();
    table.insert(NodeLook::Focus, glow(Color::WHITE, 4.0));
    table.insert(NodeLook::Cursor, glow(CURSOR, 2.6));
    for hops in 1..=FARTHEST_SHADE {
        // Bright and glowing next door, darker and flat further out.
        let (shade, strength) = match hops {
            1 => (1.0, 2.6),
            2 => (0.75, 0.8),
            _ => (0.5, 0.25),
        };
        let darken = |color: Color| {
            let linear = LinearRgba::from(color);
            Color::from(LinearRgba::rgb(
                linear.red * shade,
                linear.green * shade,
                linear.blue * shade,
            ))
        };
        for color in 0..PALETTE.len() {
            table.insert(
                NodeLook::Near(color, hops),
                // Enough glow to stand out, little enough that the hue holds.
                glow(darken(lane_color(color)), strength * 0.3),
            );
        }
    }
    for color in 0..PALETTE.len() {
        let base = lane_color(color);
        table.insert(NodeLook::Plain(color), glow(base, 0.6));
        table.insert(
            NodeLook::Faded(color),
            StandardMaterial {
                base_color: base.with_alpha(0.12),
                alpha_mode: AlphaMode::Blend,
                unlit: true,
                ..default()
            },
        );
    }
    table
}

/// Bigger for functions with more calls in or out.
pub(crate) fn node_size(graph: &ForceGraph, tile: usize) -> f32 {
    1.0 + (1.0 + graph.degree(tile) as f32).ln() * 0.55
}

/// Which way a node relates to the focus, and how many calls away: upstream wins a
/// tie, as for a function that both calls the focus and is called by it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Relation {
    Upstream(u32),
    Downstream(u32),
}

pub(crate) fn relation(reach: &Reach, tile: usize) -> Option<Relation> {
    match (reach.upstream.get(&tile), reach.downstream.get(&tile)) {
        (Some(&up), Some(&down)) if down < up => Some(Relation::Downstream(down)),
        (Some(&up), _) => Some(Relation::Upstream(up)),
        (None, Some(&down)) => Some(Relation::Downstream(down)),
        (None, None) => None,
    }
}

/// [`relation`], but none on a side of the focus that is hidden.
pub(crate) fn shown_relation(explorer: &Explorer, reach: &Reach, tile: usize) -> Option<Relation> {
    let hidden = explorer.hidden();
    relation(reach, tile).filter(|r| match r {
        Relation::Upstream(_) => !hidden.callers,
        Relation::Downstream(_) => !hidden.callees,
    })
}

/// Whether a function is drawn only as a small, faint speck: shown, but not the
/// middle and named by nothing.
pub(crate) fn speck(explorer: &Explorer, tile: usize) -> bool {
    let n = explorer.nucleus();
    n.shows(tile) && !n.lit(tile) && n.focus() != Some(tile)
}

/// Whether a function is a speck the nucleus leaves out altogether (N): not drawn,
/// and not pointed at.
pub(crate) fn hidden_speck(explorer: &Explorer, tile: usize) -> bool {
    explorer.nucleus().specks_hidden() && speck(explorer, tile)
}

pub(crate) fn node_look(explorer: &Explorer, tile: usize) -> NodeLook {
    let color = explorer.tile_color(tile) % PALETTE.len();
    // The journey's cursor is the one pink node, wherever it is.
    if explorer.cursor_mark() == Some(tile) {
        return NodeLook::Cursor;
    }
    let center = explorer.lit_focus();
    if center == Some(tile) {
        return NodeLook::Focus;
    }
    let Some(reach) = explorer.reach() else {
        return NodeLook::Plain(color);
    };
    near(explorer, reach, tile).map_or(NodeLook::Faded(color), |hops| NodeLook::Near(color, hops))
}

/// How many calls from the focus a node is, when it is near enough to light: with a
/// journey open, what the focus calls lights as far as the journey is disclosed, and
/// only its direct callers light above it. The spotlight lights what it picks out.
pub(crate) fn near(explorer: &Explorer, reach: &Reach, tile: usize) -> Option<u32> {
    let disclosed = explorer.disclosure();
    let journey = disclosed.tiles();
    let opened = disclosed.opened();
    // Whatever the nucleus names lights.
    let named = explorer.nucleus().names(tile);
    match shown_relation(explorer, reach, tile) {
        Some(Relation::Upstream(hops)) if !opened || hops == 1 => Some(hops),
        Some(Relation::Upstream(hops) | Relation::Downstream(hops)) if named => Some(hops),
        Some(Relation::Downstream(hops)) if !opened || journey.contains_key(&tile) => Some(hops),
        _ if journey.contains_key(&tile) => Some(2),
        _ if explorer.spotlit().contains(&tile) => Some(1),
        _ => None,
    }
    .map(|hops| hops.clamp(1, FARTHEST_SHADE))
}

/// The color a node is filled with, at full strength: the cursor's, else white for
/// the focus, else its module's. A journey bar is painted the same, so a bar and its
/// node read as one thing.
pub(crate) fn node_color(explorer: &Explorer, tile: usize) -> Color {
    if explorer.cursor_mark() == Some(tile) {
        return CURSOR;
    }
    if explorer.lit_focus() == Some(tile) {
        return Color::WHITE;
    }
    lane_color(explorer.tile_color(tile) % PALETTE.len())
}

/// Keeps what reaches the focus current.
pub(crate) fn track_attention(mut explorer: ResMut<Explorer>) {
    if explorer.bypass_change_detection().refresh_reach() {
        explorer.set_changed();
    }
}

/// Restyles nodes when the focus or the journey changes.
pub(crate) fn style_nodes(
    explorer: Res<Explorer>,
    materials: Res<NodeMaterials>,
    mut nodes: Query<(
        &Tile,
        &mut MeshMaterial3d<StandardMaterial>,
        &mut Transform,
        &mut Visibility,
    )>,
) {
    if !explorer.is_changed() {
        return;
    }
    let nucleus = explorer.nucleus();
    for (tile, mut material, mut transform, mut visibility) in &mut nodes {
        // What a module folds away is not drawn.
        let folded = !nucleus.shows(tile.0);
        let look = (!folded).then(|| node_look(&explorer, tile.0));
        // What the nucleus names, or what the function under the pointer calls or is
        // called by, shows plainly though the focus does not reach it; anything else is
        // a small, faint speck, or with specks left out, not drawn at all, however near
        // the focus it is.
        let named = nucleus.lit(tile.0);
        let quiet = speck(&explorer, tile.0);
        // Brought in from outside the journey, or a caller of the start folded into its
        // module's count: there, but quieter than the journey.
        let outside =
            (nucleus.outside(tile.0) || nucleus.folded(tile.0)) && explorer.aside() != Some(tile.0);
        let gone = quiet && nucleus.specks_hidden();
        visibility.set_if_neq(if folded || gone {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        });
        let Some(look) = look.filter(|_| !gone) else {
            continue;
        };
        let look = match look {
            NodeLook::Faded(color) if named => NodeLook::Plain(color),
            NodeLook::Near(color, _) if quiet => NodeLook::Faded(color),
            look => look,
        };
        let (scale, dim) = if quiet {
            (nucleus.node_scale(true), false)
        } else if outside {
            (0.6 * nucleus.node_scale(false), true)
        } else {
            (nucleus.node_scale(false), false)
        };
        let size = node_size(explorer.graph(), tile.0) * scale;
        if (transform.scale.x - size).abs() > 0.001 {
            transform.scale = Vec3::splat(size);
        }
        if let Some(wanted) = materials.0.get(&(look, dim))
            && &material.0 != wanted
        {
            material.0 = wanted.clone();
        }
    }
}

#[derive(Resource, Debug)]
pub(crate) struct NodeMaterials(pub(crate) HashMap<(NodeLook, bool), Handle<StandardMaterial>>);

pub(crate) fn create_node_materials(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(NodeMaterials(node_materials(&mut materials)));
}

/// Every call as a link, then the spotlight, the focus and the node under the pointer.
pub(crate) fn draw_graph(
    explorer: Res<Explorer>,
    clock: Res<Time>,
    mut gizmos: Gizmos,
    mut marked: Local<Option<(usize, f32)>>,
    mut awake: ResMut<Awake>,
) {
    let time = clock.elapsed_secs();
    draw_spotlight(&explorer, time, &mut gizmos);
    // The cursor pulses for a moment wherever it lands.
    let cursor = explorer.cursor_mark();
    if cursor != marked.map(|(tile, _)| tile) {
        *marked = cursor.map(|tile| (tile, time));
    }
    let age = marked.map_or(f32::INFINITY, |(_, since)| time - since);
    if age < CURSOR_PULSE {
        awake.0 = true;
    }
    draw_attention(&explorer, age, &mut gizmos);
}

/// Seconds the cursor pulses for when it lands on a function.
const CURSOR_PULSE: f32 = 1.2;

/// Dots running along a call from caller to callee, so which way it goes reads
/// without a legend.
pub(crate) fn flow(gizmos: &mut Gizmos, a: Vec3, b: Vec3, color: Color, time: f32, seed: usize) {
    let length = a.distance(b);
    if length < 1.0 {
        return;
    }
    // About one dot every few units, all moving at the same speed.
    let count = (length / 7.0).ceil().clamp(1.0, 5.0);
    #[expect(clippy::cast_precision_loss, reason = "a seed for a phase")]
    let offset = (seed as f32 * 0.618_034).fract();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 1..=5"
    )]
    for dot in 0..count as u32 {
        #[expect(clippy::cast_precision_loss, reason = "a small count")]
        let phase = (time * 5.0 / length + (dot as f32 + offset) / count).fract();
        gizmos.sphere(
            Isometry3d::from_translation(a.lerp(b, phase)),
            0.22,
            color.with_alpha((color.alpha() + 0.15).min(1.0)),
        );
    }
}

/// What the spotlight picks out, ringed in its side's color: three quick pulses when
/// it comes on, then a steady ring.
fn draw_spotlight(explorer: &Explorer, time: f32, gizmos: &mut Gizmos) {
    let Some((side, since)) = explorer.spotlight() else {
        return;
    };
    let color = match side {
        Side::Callers => CALLER,
        Side::Callees => CALLEE,
    };
    let age = time - since;
    let pulse = if age < 1.8 {
        0.5 + 0.5 * (age * std::f32::consts::TAU / 0.6).cos()
    } else {
        0.0
    };
    for tile in explorer.spotlit() {
        let Some(at) = explorer.tile_point(tile) else {
            continue;
        };
        let radius = node_size(explorer.graph(), tile) * 0.5 + 0.7 + pulse * 1.2;
        // Two rings, so it reads as a mark rather than a stray line.
        for (grow, alpha) in [(0.0, 0.9), (0.45, 0.45)] {
            gizmos.sphere(
                Isometry3d::from_translation(at),
                radius + grow,
                color.with_alpha(alpha * (0.7 + 0.3 * pulse)),
            );
        }
    }
}

/// A white ring round the focus, and a fainter, wider one round the node under the
/// pointer, in the graph or as a bar in the journey. Gathered round the focus's
/// module, the journey's cursor, where it is not the nucleus's middle, gets two rings
/// where it sits in a glow of its own color, pulsing for a moment `age` seconds after it landed;
/// the function under the pointer, a bar's included, glows as a preview of that.
fn draw_attention(explorer: &Explorer, age: f32, gizmos: &mut Gizmos) {
    let graph = explorer.graph();
    let nucleus = explorer.nucleus();
    // The node's own radius, as big as it is drawn.
    let size = |tile: usize| node_size(graph, tile) * nucleus.node_scale(false) * 0.5;
    let focus = explorer.lit_focus();
    // The focus, and in a journey a function selected off it, ringed in white.
    let ringed = focus.into_iter().chain(explorer.aside());
    for tile in ringed {
        if let Some(at) = explorer.tile_point(tile) {
            let radius = size(tile) + 0.35;
            gizmos.sphere(Isometry3d::from_translation(at), radius, Color::WHITE);
        }
    }
    let zoom = nucleus.zoom();
    let glow = |gizmos: &mut Gizmos, at: Vec3, radius: f32, color: Color, strength: f32| {
        for (grow, alpha) in [(1.3, 0.35), (1.9, 0.2), (2.6, 0.1)] {
            gizmos.sphere(
                Isometry3d::from_translation(at),
                radius + grow * zoom,
                color.with_alpha(alpha * strength),
            );
        }
    };
    if let Some(tile) = explorer.cursor_mark()
        && let Some(at) = explorer.tile_point(tile)
    {
        let pulse = if age < CURSOR_PULSE {
            0.5 - 0.5 * (age * std::f32::consts::TAU / 0.4).cos()
        } else {
            0.0
        };
        for grow in [0.35, 0.8] {
            gizmos.sphere(
                Isometry3d::from_translation(at),
                size(tile) + (grow + pulse * 1.2) * zoom,
                CURSOR,
            );
        }
        glow(gizmos, at, size(tile), CURSOR, 1.0);
    }
    if let Some(tile) = explorer
        .pointed()
        .filter(|&t| Some(t) != focus && Some(t) != explorer.cursor_mark())
        && let Some(at) = explorer.tile_point(tile)
    {
        glow(gizmos, at, size(tile), Color::WHITE, 0.6);
    }
    if let Some(tile) = explorer.pointed().filter(|&t| Some(t) != focus)
        && let Some(at) = explorer.tile_point(tile)
    {
        let radius = size(tile) + 0.6;
        gizmos.sphere(
            Isometry3d::from_translation(at),
            radius,
            Color::srgba(1.0, 1.0, 1.0, 0.55),
        );
    }
}

/// The closest the orbit comes to the focused node.
const MIN_ORBIT_DISTANCE: f32 = 25.0;

/// F turns back to the usual three-quarter view, framed as it was when the view was
/// pointed there; R frames that same view from the angle the camera has, or browsing,
/// turns back to the usual view of the whole codebase; Q and E turn the view an eighth
/// of the way round, left and right.
pub(crate) fn graph_keys(
    keys: Res<ButtonInput<KeyCode>>,
    explorer: Res<Explorer>,
    mut cameras: Query<(&mut OrbitCamera, &Projection)>,
) {
    let cursor = explorer.graph_focus().and_then(|t| explorer.tile_point(t));
    for (mut camera, projection) in &mut cameras {
        let gathered = explorer.nucleus();
        if keys.just_pressed(KeyCode::KeyF) {
            camera.turn_to(nucleus::VIEW);
            let (yaw, pitch) = nucleus::VIEW;
            let rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
            nucleus::fit(gathered, cursor, &mut camera, rotation, projection);
        }
        for (key, turn) in [(KeyCode::KeyQ, 1.0), (KeyCode::KeyE, -1.0)] {
            if keys.just_pressed(key) {
                camera.turn_by(turn * std::f32::consts::FRAC_PI_4);
                // The nucleus refits to what the new angle sees.
                let rotation = camera.aim_rotation();
                nucleus::fit(gathered, cursor, &mut camera, rotation, projection);
            }
        }
        if keys.just_pressed(KeyCode::KeyR) {
            if gathered.browsing() {
                // The whole codebase, as it was first seen: from the front.
                camera.turn_to(nucleus::VIEW);
                let rotation = camera.aim_rotation();
                nucleus::fit_all(gathered, &mut camera, rotation, projection);
            } else {
                let rotation = camera.aim_rotation();
                nucleus::fit(gathered, cursor, &mut camera, rotation, projection);
            }
        }
    }
}

/// How far back a camera turned to `rotation` and looking at `center` must be for
/// every one of `points` to sit inside the middle of its view, clear of the edges.
pub(crate) fn fit_distance(
    points: &[Vec3],
    center: Vec3,
    rotation: Quat,
    perspective: &PerspectiveProjection,
) -> f32 {
    // The share of the half-view a point may use: the rest is room for its name, and
    // for the journey docked over the bottom right.
    const ROOM: f32 = 0.72;
    let tan_y = (perspective.fov / 2.0).tan() * ROOM;
    let tan_x = tan_y * perspective.aspect_ratio;
    let (right, up, back) = (rotation * Vec3::X, rotation * Vec3::Y, rotation * Vec3::Z);
    points
        .iter()
        .map(|&p| {
            let offset = p - center;
            // A point this far towards the camera needs the camera that much further.
            let toward = offset.dot(back);
            (offset.dot(right).abs() / tan_x).max(offset.dot(up).abs() / tan_y) + toward
        })
        .fold(MIN_ORBIT_DISTANCE, f32::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulls_back_only_as_far_as_what_it_must_show() {
        let view = PerspectiveProjection {
            aspect_ratio: 1.6,
            ..default()
        };
        let head_on = Quat::IDENTITY;
        let near = [Vec3::new(3.0, 2.0, 0.0), Vec3::new(-4.0, -1.0, 0.0)];
        assert!(
            (fit_distance(&near, Vec3::ZERO, head_on, &view) - MIN_ORBIT_DISTANCE).abs() < 1e-3,
            "what is close needs no more than the closest orbit"
        );
        let far = [Vec3::new(0.0, 40.0, 0.0)];
        let needed = fit_distance(&far, Vec3::ZERO, head_on, &view);
        let half_height = needed * (view.fov / 2.0).tan();
        assert!(
            needed > MIN_ORBIT_DISTANCE && half_height > 40.0,
            "far enough back that it shows, with room: {needed}"
        );
        let wide = [Vec3::new(40.0, 0.0, 0.0)];
        assert!(
            fit_distance(&wide, Vec3::ZERO, head_on, &view) < needed,
            "the view is wider than it is tall"
        );
    }
}
