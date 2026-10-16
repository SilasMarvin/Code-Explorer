//! The codebase as a graph of calls: how its nodes look, which links light up, and
//! where the camera goes. Whatever the pointer is over, or else the focused node, is
//! the center of attention. Everything that calls into it turns amber and everything
//! it calls turns blue, brightest one call away and fading with each call further;
//! the rest of the graph fades so that neighborhood reads on its own.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use codeviz_core::Change;
use codeviz_layout::{ForceGraph, Reach};

use crate::{
    camera::{FRONT, OrbitCamera},
    diff_view::{ADDED, DiffView, MODIFIED, MOVED, REMOVED, diff_look, draw_diff_graph},
    explorer::Explorer,
    lens::{Lens, SPECK, Source},
    scene::{ACCENTS, PALETTE, Tile, accent, lane_color},
};

pub(crate) const CALLER: Color = Color::srgb(1.0, 0.62, 0.2);
pub(crate) const CALLEE: Color = Color::srgb(0.3, 0.68, 1.0);

/// Calls away from the focus that still get their own shade; further is drawn as this.
const FARTHEST_SHADE: u32 = 3;

/// How a node is drawn right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum NodeLook {
    /// The center of attention.
    Focus,
    /// Calls into the focus, this many calls away (capped at [`FARTHEST_SHADE`]).
    Upstream(u32),
    /// Called from the focus, this many calls away (capped at [`FARTHEST_SHADE`]).
    Downstream(u32),
    /// Run by the active journey, unrelated to the focus: in that journey's accent.
    Journey(usize),
    /// Left out by quiet mode: a faint speck that keeps the shape of the graph.
    Hidden,
    /// Unrelated to the focus.
    Faded(usize),
    /// Anything while nothing is focused, in its container's color.
    Plain(usize),
    /// Added, removed or rewritten, when showing a diff.
    Changed(Change),
    /// Can reach a change in this many calls (capped at [`FARTHEST_SHADE`]).
    Impacted(u32),
}

/// Steps of brightness a node fades through as the lens reveals or hides it.
pub(crate) const LEVELS: u8 = 6;

/// Every look at every level of brightness, [`LEVELS`] being full.
pub(crate) fn node_materials(
    materials: &mut Assets<StandardMaterial>,
) -> HashMap<(NodeLook, u8), Handle<StandardMaterial>> {
    let mut table = HashMap::new();
    for (look, material) in node_looks() {
        for level in 0..=LEVELS {
            let share = 0.12 + 0.88 * f32::from(level) / f32::from(LEVELS);
            table.insert((look, level), materials.add(dimmed(&material, share)));
        }
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
        table.insert(NodeLook::Upstream(hops), glow(darken(CALLER), strength));
        table.insert(NodeLook::Downstream(hops), glow(darken(CALLEE), strength));
    }
    for track in 0..ACCENTS.len() {
        let base = accent(track);
        table.insert(NodeLook::Journey(track), glow(base, 0.35));
    }
    table.insert(NodeLook::Changed(Change::Added), glow(ADDED, 2.4));
    table.insert(NodeLook::Changed(Change::Modified), glow(MODIFIED, 2.4));
    table.insert(NodeLook::Changed(Change::Moved), glow(MOVED, 2.4));
    table.insert(
        NodeLook::Changed(Change::Removed),
        StandardMaterial {
            base_color: REMOVED.with_alpha(0.55),
            emissive: LinearRgba::from(REMOVED) * 0.8,
            alpha_mode: AlphaMode::Blend,
            ..default()
        },
    );
    for hops in 1..=FARTHEST_SHADE {
        let (shade, strength) = match hops {
            1 => (0.8, 1.0),
            2 => (0.6, 0.45),
            _ => (0.45, 0.15),
        };
        let linear = LinearRgba::from(MODIFIED);
        let color = Color::from(LinearRgba::rgb(
            linear.red * shade,
            linear.green * shade * 0.85,
            linear.blue * shade * 0.6,
        ));
        table.insert(NodeLook::Impacted(hops), glow(color, strength));
    }
    table.insert(
        NodeLook::Hidden,
        StandardMaterial {
            base_color: Color::srgba(0.6, 0.65, 0.75, 0.08),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..default()
        },
    );
    for color in 0..PALETTE.len() {
        let base = lane_color(color);
        table.insert(NodeLook::Plain(color), glow(base, 1.2));
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

pub(crate) fn node_look(explorer: &Explorer, tile: usize) -> NodeLook {
    let color = explorer.tile_color(tile) % PALETTE.len();
    if !explorer.shown(tile) {
        return NodeLook::Hidden;
    }
    let center = explorer.attention();
    if center == Some(tile) {
        return NodeLook::Focus;
    }
    let Some(reach) = explorer.reach() else {
        return NodeLook::Plain(color);
    };
    match relation(reach, tile) {
        Some(Relation::Upstream(hops)) => NodeLook::Upstream(hops.min(FARTHEST_SHADE)),
        Some(Relation::Downstream(hops)) => NodeLook::Downstream(hops.min(FARTHEST_SHADE)),
        None if explorer.journey_lane(tile).is_some() => {
            NodeLook::Journey(explorer.active() % ACCENTS.len())
        }
        None => NodeLook::Faded(color),
    }
}

/// Keeps what reaches the center of attention current.
pub(crate) fn track_attention(mut explorer: ResMut<Explorer>) {
    if explorer.bypass_change_detection().refresh_reach() {
        explorer.set_changed();
    }
}

/// Restyles nodes when attention or the journeys change.
pub(crate) fn style_nodes(
    explorer: Res<Explorer>,
    diff: Option<Res<DiffView>>,
    materials: Res<NodeMaterials>,
    mut nodes: Query<(&Tile, &mut MeshMaterial3d<StandardMaterial>, &mut Transform)>,
) {
    if !explorer.is_changed() && !diff.as_ref().is_some_and(DetectChanges::is_changed) {
        return;
    }
    for (tile, mut material, mut transform) in &mut nodes {
        let look = match &diff {
            Some(diff) => diff_look(&explorer, diff, tile.0),
            None => node_look(&explorer, tile.0),
        };
        // What the lens leaves out shrinks as well as fades, down to a speck.
        let shown = explorer.visibility(tile.0);
        let (scale, level) = if look == NodeLook::Hidden {
            (0.35, LEVELS)
        } else {
            let level = (shown * f32::from(LEVELS))
                .round()
                .clamp(0.0, f32::from(LEVELS));
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "clamped to 0..=LEVELS"
            )]
            (0.35 + 0.65 * shown, level as u8)
        };
        let size = node_size(explorer.graph(), tile.0) * scale;
        if (transform.scale.x - size).abs() > 0.001 {
            transform.scale = Vec3::splat(size);
        }
        if let Some(wanted) = materials.0.get(&(look, level))
            && &material.0 != wanted
        {
            material.0 = wanted.clone();
        }
    }
}

#[derive(Resource, Debug)]
pub(crate) struct NodeMaterials(pub(crate) HashMap<(NodeLook, u8), Handle<StandardMaterial>>);

pub(crate) fn create_node_materials(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(NodeMaterials(node_materials(&mut materials)));
}

/// How strongly to draw a link this many calls from the focus.
fn fade(hops: u32) -> f32 {
    match hops {
        0 | 1 => 0.95,
        2 => 0.5,
        _ => 0.22,
    }
}

/// Every call as a link. Calls leading into the focus are amber and calls leading out
/// of it blue, fading with distance; the active journey's path is in its accent;
/// everything else is faint.
pub(crate) fn draw_graph(
    explorer: Res<Explorer>,
    diff: Option<Res<DiffView>>,
    clock: Res<Time>,
    mut gizmos: Gizmos,
) {
    if let Some(diff) = diff {
        draw_diff_graph(&explorer, &diff, clock.elapsed_secs(), &mut gizmos);
        return;
    }
    let graph = explorer.graph();
    let center = explorer.attention();
    let reach = explorer.reach();
    let point = |tile: usize| explorer.tile_point(tile);
    let map = explorer.map();
    let track = explorer.track();
    let path: HashSet<(usize, usize)> = track
        .scene()
        .edges
        .iter()
        .filter_map(|edge| {
            let tile = |span| track.unit_id(span).and_then(|u| map.tile_of(u));
            Some((tile(edge.caller)?, tile(edge.callee)?))
        })
        .collect();
    let path_color = accent(explorer.active());
    // Calls from the focus, counting the focus itself as zero.
    let hops = |side: &HashMap<usize, u32>, tile: usize| {
        if center == Some(tile) {
            Some(0)
        } else {
            side.get(&tile).copied()
        }
    };

    for &(from, to) in &graph.calls {
        let (Some(a), Some(b)) = (point(from), point(to)) else {
            continue;
        };
        let shown = explorer.visibility(from).min(explorer.visibility(to));
        if shown < SPECK {
            continue;
        }
        // A link lights up when it takes one step nearer the focus (amber) or one
        // step further out from it (blue).
        let lit = reach.and_then(|reach| {
            let up = hops(&reach.upstream, from).zip(hops(&reach.upstream, to));
            let down = hops(&reach.downstream, from).zip(hops(&reach.downstream, to));
            match (up, down) {
                (Some((outer, inner)), _) if outer == inner + 1 => {
                    Some(CALLER.with_alpha(fade(outer)))
                }
                (_, Some((inner, outer))) if outer == inner + 1 => {
                    Some(CALLEE.with_alpha(fade(outer)))
                }
                _ => None,
            }
        });
        let (color, arrow) = match lit {
            Some(color) => (color, true),
            None if path.contains(&(from, to)) => (
                path_color.with_alpha(if center.is_some() { 0.3 } else { 0.85 }),
                true,
            ),
            None if center.is_some() => (Color::srgba(0.6, 0.66, 0.8, 0.04), false),
            None => (Color::srgba(0.6, 0.66, 0.8, 0.16), false),
        };
        let color = color.with_alpha(color.alpha() * shown);
        if arrow {
            // Stop at the callee's surface so the head stays visible.
            let back = (b - a).normalize_or_zero() * node_size(graph, to) * 0.5;
            gizmos.arrow(a, b - back, color).with_tip_length(0.6);
        } else {
            gizmos.line(a, b, color);
        }
    }
    if let Some(at) = center.and_then(point) {
        let radius = center.map_or(1.0, |c| node_size(graph, c)) * 0.5 + 0.35;
        gizmos.sphere(Isometry3d::from_translation(at), radius, Color::WHITE);
    }
    // The cursor's node, when attention is elsewhere.
    let cursor = explorer.focus().and_then(|unit| map.tile_of(unit));
    if let Some(tile) = cursor.filter(|&t| Some(t) != center)
        && let Some(at) = point(tile)
    {
        let radius = node_size(graph, tile) * 0.5 + 0.35;
        gizmos.sphere(
            Isometry3d::from_translation(at),
            radius,
            Color::srgba(1.0, 1.0, 1.0, 0.4),
        );
    }
}

/// The closest the orbit comes to the focused node.
const MIN_ORBIT_DISTANCE: f32 = 25.0;

/// F turns to face the graph head-on, still centered on the focus; R pulls back to
/// take in the whole graph.
pub(crate) fn graph_keys(
    keys: Res<ButtonInput<KeyCode>>,
    explorer: Res<Explorer>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    for mut camera in &mut cameras {
        if keys.just_pressed(KeyCode::KeyF) {
            camera.turn_to(FRONT);
            orbit_focus(&explorer, &mut camera);
        }
        if keys.just_pressed(KeyCode::KeyR) {
            frame_all(&explorer, &mut camera);
        }
    }
}

/// Looks at the middle of the graph from far enough back to see all of it.
fn frame_all(explorer: &Explorer, camera: &mut OrbitCamera) {
    let points: Vec<Vec3> = (0..explorer.map().tiles.len())
        .filter_map(|t| explorer.tile_point(t))
        .collect();
    frame_points(&points, camera);
}

/// When the lens changes, or opens limited, refits the view to what it shows in full:
/// around the focus when that is where it starts, else around everything it reveals.
pub(crate) fn frame_reveal(
    explorer: Res<Explorer>,
    mut last: Local<Option<Lens>>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    let lens = explorer.lens();
    // A reveal set from the start frames what it shows too.
    let changed = last.map_or(lens.reach.is_some(), |l| l != lens);
    *last = Some(lens);
    if !changed {
        return;
    }
    for mut camera in &mut cameras {
        if lens.source == Source::Focus && !lens.changes_only && explorer.graph_focus().is_some() {
            orbit_focus(&explorer, &mut camera);
            continue;
        }
        let Some(targets) = explorer.reveal_targets() else {
            frame_all(&explorer, &mut camera);
            continue;
        };
        let points: Vec<Vec3> = targets
            .iter()
            .enumerate()
            .filter(|&(_, &shown)| shown >= 0.5)
            .filter_map(|(tile, _)| explorer.tile_point(tile))
            .collect();
        frame_points(&points, &mut camera);
    }
}

/// Looks at the middle of `points` from far enough back to see them all.
fn frame_points(points: &[Vec3], camera: &mut OrbitCamera) {
    if points.is_empty() {
        return;
    }
    let middle = points.iter().copied().sum::<Vec3>() / points.len() as f32;
    let radius = points
        .iter()
        .map(|p| p.distance(middle))
        .fold(0.0, f32::max);
    camera.glide_to(middle, (radius * 2.6).max(MIN_ORBIT_DISTANCE));
}

/// Orbits the focused node from far enough back to see everything it calls and
/// everything that calls it.
pub(crate) fn orbit_focus(explorer: &Explorer, camera: &mut OrbitCamera) {
    let graph = explorer.graph();
    let Some(focus) = explorer.graph_focus() else {
        return;
    };
    let Some(at) = explorer.tile_point(focus) else {
        return;
    };
    let reach = graph
        .callers(focus)
        .chain(graph.callees(focus))
        .filter(|&t| explorer.shown(t))
        .filter_map(|t| explorer.tile_point(t))
        .map(|p| p.distance(at))
        .fold(0.0, f32::max);
    // A 45° view shows about 0.4 of its distance either side of center; leave room.
    camera.glide_to(at, (reach * 3.4).max(MIN_ORBIT_DISTANCE));
}

/// Swings round to a newly focused node, and keeps it centered while the graph moves.
pub(crate) fn follow_focus(
    explorer: Res<Explorer>,
    mut followed: Local<Option<usize>>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    let focus = explorer.graph_focus();
    let changed = focus != *followed;
    *followed = focus;
    for mut camera in &mut cameras {
        if changed {
            orbit_focus(&explorer, &mut camera);
        } else if explorer.settling()
            && let Some(at) = focus.and_then(|t| explorer.tile_point(t))
        {
            camera.target = at;
        }
    }
}
