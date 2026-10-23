//! The codebase as a graph of calls: how its nodes look, which links light up, and
//! where the camera goes. Whatever the pointer is over, or else the focused node, is
//! the center of attention. Each channel carries one thing: a link's color says which
//! way a call runs (violet into the focus, blue out of it, with dots flowing along
//! it), a node's fill says where it lives (its module) or, under a diff, what
//! changed. What is near the focus is bright, fading with each call further; the rest
//! of the graph fades so that neighborhood reads on its own.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use bevy::prelude::*;
use codeviz_core::Change;
use codeviz_layout::{ForceGraph, Reach};

use crate::{
    camera::{FRONT, OrbitCamera},
    diff_view::{
        ADDED, DiffView, MODIFIED, MOVED, REMOVED, change_color, diff_look, draw_diff_graph,
    },
    disclosure::{Disclose, Disclosed},
    explorer::{Explorer, Side, Spotlight},
    lens::{Lens, SPECK, Source},
    scene::{PALETTE, Tile, lane_color},
};

/// Calls into the focus.
pub(crate) const CALLER: Color = Color::srgb(0.74, 0.52, 1.0);
/// Calls out of the focus.
pub(crate) const CALLEE: Color = Color::srgb(0.3, 0.72, 1.0);
/// What a diff left alone.
pub(crate) const UNCHANGED: Color = Color::srgb(0.6, 0.64, 0.72);
/// Calls further along, which do not touch the focus: an arrow, but no color, so violet
/// and blue only ever mean into and out of the focus.
pub(crate) const FURTHER: Color = Color::srgb(0.82, 0.85, 0.92);

/// Calls away from the focus that still get their own shade; further is drawn as this.
const FARTHEST_SHADE: u32 = 3;

/// How a node is drawn right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum NodeLook {
    /// The center of attention.
    Focus,
    /// Calls into the focus or called from it, this many calls away (capped at
    /// [`FARTHEST_SHADE`]), in its module's color.
    Near(usize, u32),
    /// Left alone by a diff but near the focus, this many calls away.
    Unchanged(u32),
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
        for color in 0..PALETTE.len() {
            table.insert(
                NodeLook::Near(color, hops),
                // Enough glow to stand out, little enough that the hue holds.
                glow(darken(lane_color(color)), strength * 0.3),
            );
        }
        table.insert(
            NodeLook::Unchanged(hops),
            glow(darken(UNCHANGED), strength * 0.4),
        );
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
    near(explorer, reach, tile).map_or(NodeLook::Faded(color), |hops| NodeLook::Near(color, hops))
}

/// How many calls from the focus a node is, when it is near enough to light: with a
/// journey open, what the focus calls lights as far as the journey is disclosed, and
/// only its direct callers light above it. The spotlight lights what it picks out.
pub(crate) fn near(explorer: &Explorer, reach: &Reach, tile: usize) -> Option<u32> {
    let disclosed = explorer.disclosure();
    let journey = disclosed.tiles();
    let opened = disclosed.opened();
    match relation(reach, tile) {
        Some(Relation::Upstream(hops)) if !opened || hops == 1 => Some(hops),
        Some(Relation::Downstream(hops)) if !opened || journey.contains_key(&tile) => Some(hops),
        _ if journey.contains_key(&tile) => Some(2),
        _ => explorer.spotlight_hops(tile),
    }
    .map(|hops| hops.clamp(1, FARTHEST_SHADE))
}

/// The color a node is filled with, at full strength: white for the center of
/// attention, else what changed under a diff, else its module's. A journey bar is
/// painted the same, so a bar and its node read as one thing.
pub(crate) fn node_color(explorer: &Explorer, diff: Option<&DiffView>, tile: usize) -> Color {
    if explorer.attention() == Some(tile) {
        return Color::WHITE;
    }
    match diff {
        Some(diff) => diff.change(tile).map_or(UNCHANGED, change_color),
        None => lane_color(explorer.tile_color(tile) % PALETTE.len()),
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

/// Every call as a link, then the spotlight and the center of attention.
pub(crate) fn draw_graph(
    explorer: Res<Explorer>,
    diff: Option<Res<DiffView>>,
    clock: Res<Time>,
    mut gizmos: Gizmos,
) {
    draw_regions(&explorer, &mut gizmos);
    let time = clock.elapsed_secs();
    match diff {
        Some(diff) => draw_diff_graph(&explorer, &diff, time, &mut gizmos),
        None => draw_calls(&explorer, time, &mut gizmos),
    }
    draw_spotlight(&explorer, time, &mut gizmos);
    draw_attention(&explorer, &mut gizmos);
}

/// The calls along the spotlight's routes: from each node it picks out towards the
/// focus when it picks out callers, away from the focus when callees.
pub(crate) struct Routes {
    side: Side,
    focus: usize,
    hops: HashMap<usize, u32>,
}

impl Routes {
    pub(crate) fn of(explorer: &Explorer) -> Option<Self> {
        Some(Self {
            side: explorer.spotlight()?.side,
            focus: explorer.graph_focus()?,
            hops: explorer.spotlit_hops(),
        })
    }

    /// When a call lies on a route, how many calls its end further from the focus is
    /// from it.
    pub(crate) fn call(&self, from: usize, to: usize) -> Option<u32> {
        let hops = |tile: usize| {
            if tile == self.focus {
                Some(0)
            } else {
                self.hops.get(&tile).copied()
            }
        };
        let (outer, inner) = match self.side {
            Side::Callers => (from, to),
            Side::Callees => (to, from),
        };
        hops(inner)?;
        hops(outer).filter(|&hops| hops > 0)
    }
}

/// A call on one of the spotlight's routes that does not touch the focus: a pale
/// arrow, steady out to the end of the route so the whole of it reads, with dots
/// running the way the call does.
pub(crate) fn draw_route(
    gizmos: &mut Gizmos,
    (a, b): (Vec3, Vec3),
    hops: u32,
    shown: f32,
    time: f32,
    seed: usize,
) {
    #[expect(clippy::cast_precision_loss, reason = "a small count of calls")]
    let alpha = (0.95 - 0.08 * hops as f32).max(0.55) * shown;
    let color = FURTHER.with_alpha(alpha);
    gizmos.arrow(a, b, color).with_tip_length(0.9);
    flow(gizmos, a, b, color.with_alpha(alpha * 0.7), time, seed);
}

/// Calls into the focus are violet and calls out of it blue, with dots flowing along
/// them; calls on the spotlight's routes are pale arrows with dots of their own;
/// calls further along, nearer or further from it and on the active journey's path,
/// are pale arrows fading with distance; everything else is faint.
fn draw_calls(explorer: &Explorer, time: f32, gizmos: &mut Gizmos) {
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
        .filter(|edge| explorer.disclosed(edge.callee))
        .filter_map(|edge| {
            let node_of = |span| track.unit_id(span).and_then(|u| map.tile_of(u));
            Some((node_of(edge.caller)?, node_of(edge.callee)?))
        })
        .collect();
    let disclosed = explorer.disclosure();
    let journey = disclosed.tiles();
    let opened = disclosed.opened();
    let spotlit = explorer.spotlit();
    let routes = Routes::of(explorer);
    // Calls from the focus, counting the focus itself as zero.
    let hops = |side: &HashMap<usize, u32>, tile: usize| {
        if center == Some(tile) {
            Some(0)
        } else {
            side.get(&tile).copied()
        }
    };

    for (index, &(from, to)) in graph.calls.iter().enumerate() {
        let (Some(a), Some(b)) = (point(from), point(to)) else {
            continue;
        };
        let shown = explorer.visibility(from).min(explorer.visibility(to));
        if shown < SPECK {
            continue;
        }
        let back = (b - a).normalize_or_zero() * node_size(graph, to) * 0.5;
        if center != Some(from)
            && center != Some(to)
            && let Some(hops) = routes.as_ref().and_then(|r| r.call(from, to))
        {
            draw_route(gizmos, (a, b - back), hops, shown, time, index);
            continue;
        }
        // A link lights up when it takes one step nearer the focus (violet) or one
        // step further out from it (blue).
        let lit = reach.and_then(|reach| {
            let up = hops(&reach.upstream, from).zip(hops(&reach.upstream, to));
            let down = hops(&reach.downstream, from).zip(hops(&reach.downstream, to));
            match (up, down) {
                (Some((outer, inner)), _)
                    if outer == inner + 1 && (!opened || outer == 1 || spotlit.contains(&from)) =>
                {
                    Some((CALLER, outer))
                }
                (_, Some((inner, outer)))
                    if outer == inner + 1
                        && (!opened
                            || journey.contains_key(&to) && journey.contains_key(&from)
                            || spotlit.contains(&to)) =>
                {
                    Some((CALLEE, outer))
                }
                _ => None,
            }
        });
        match lit {
            // Stop at the callee's surface so the head stays visible.
            Some((color, 1)) => {
                let color = color.with_alpha(fade(1) * shown);
                gizmos.arrow(a, b - back, color).with_tip_length(1.1);
                flow(gizmos, a, b - back, color, time, index);
            }
            Some((_, outer)) => {
                gizmos
                    .arrow(a, b - back, FURTHER.with_alpha(fade(outer) * 0.8 * shown))
                    .with_tip_length(0.8);
            }
            None if path.contains(&(from, to)) => {
                gizmos
                    .arrow(a, b - back, FURTHER.with_alpha(0.4 * shown))
                    .with_tip_length(0.8);
            }
            None => {
                let alpha = if center.is_some() {
                    0.04
                } else if graph.cluster(from) == graph.cluster(to) {
                    0.18
                } else {
                    // Calls between modules step back, so each module's own wiring reads.
                    0.06
                };
                gizmos.line(a, b, Color::srgba(0.6, 0.66, 0.8, alpha * shown));
            }
        }
    }
}

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

/// What the spotlight picks out next to the focus, ringed in its side's color, and
/// where its routes start, ringed pale: three quick pulses when it comes on or
/// widens, then a steady ring.
fn draw_spotlight(explorer: &Explorer, time: f32, gizmos: &mut Gizmos) {
    let Some(spotlight) = explorer.spotlight() else {
        return;
    };
    let color = match spotlight.side {
        Side::Callers => CALLER,
        Side::Callees => CALLEE,
    };
    let age = time - spotlight.since;
    let pulse = if age < 1.8 {
        0.5 + 0.5 * (age * std::f32::consts::TAU / 0.6).cos()
    } else {
        0.0
    };
    let starts: HashSet<usize> = match (spotlight.side, explorer.graph_focus()) {
        (Side::Callers, Some(focus)) => explorer.starts(focus).iter().map(|s| s.tile).collect(),
        _ => HashSet::new(),
    };
    for (tile, hops) in explorer.spotlit_hops() {
        let color = match hops {
            1 => color,
            _ if starts.contains(&tile) => FURTHER,
            _ => continue,
        };
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

/// A white ring round the center of attention, and a fainter one round the journey's
/// cursor when attention is elsewhere.
fn draw_attention(explorer: &Explorer, gizmos: &mut Gizmos) {
    let graph = explorer.graph();
    let center = explorer.attention();
    if let Some(at) = center.and_then(|c| explorer.tile_point(c)) {
        let radius = center.map_or(1.0, |c| node_size(graph, c)) * 0.5 + 0.35;
        gizmos.sphere(Isometry3d::from_translation(at), radius, Color::WHITE);
    }
    let cursor = explorer
        .focus()
        .and_then(|unit| explorer.map().tile_of(unit));
    if let Some(tile) = cursor.filter(|&t| Some(t) != center)
        && let Some(at) = explorer.tile_point(tile)
    {
        let radius = node_size(graph, tile) * 0.5 + 0.35;
        gizmos.sphere(
            Isometry3d::from_translation(at),
            radius,
            Color::srgba(1.0, 1.0, 1.0, 0.4),
        );
    }
}

/// Each module's region, as the functions showing place it: its cluster, middle, and
/// a radius holding most of them.
pub(crate) fn regions(explorer: &Explorer) -> Vec<(usize, Vec3, f32)> {
    let graph = explorer.graph();
    let mut members: Vec<Vec<Vec3>> = vec![Vec::new(); graph.cluster_count()];
    for tile in 0..explorer.map().tiles.len() {
        if explorer.visibility(tile) < 0.5 {
            continue;
        }
        if let (Some(cluster), Some(at)) = (graph.cluster(tile), explorer.tile_point(tile))
            && let Some(list) = members.get_mut(cluster)
        {
            list.push(at);
        }
    }
    members
        .iter()
        .enumerate()
        .filter(|(_, points)| !points.is_empty())
        .map(|(cluster, points)| {
            let middle = points.iter().copied().sum::<Vec3>() / points.len() as f32;
            let mut distances: Vec<f32> = points
                .iter()
                .map(|p| (p.truncate() - middle.truncate()).length())
                .collect();
            distances.sort_by(f32::total_cmp);
            let radius = distances[(distances.len() * 9 / 10).min(distances.len() - 1)] + 4.0;
            (cluster, middle, radius)
        })
        .collect()
}

/// A faint ring round each module's region, in its color.
fn draw_regions(explorer: &Explorer, gizmos: &mut Gizmos) {
    for (cluster, middle, radius) in regions(explorer) {
        let color = lane_color(cluster % PALETTE.len());
        gizmos
            .circle(
                Isometry3d::from_translation(middle),
                radius,
                color.with_alpha(0.22),
            )
            .resolution(64);
    }
}

/// The closest the orbit comes to the focused node.
const MIN_ORBIT_DISTANCE: f32 = 25.0;

/// F turns to face the graph head-on, centered on the focus again; R pulls back to
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
    frame_points(&points, None, camera);
    // Away from the focus, so it stays where it was put.
    camera.free = true;
}

/// When the lens changes, or opens limited, refits the view to what it shows in full:
/// around the focus when that is where it starts, else around everything it reveals.
pub(crate) fn frame_reveal(
    explorer: Res<Explorer>,
    mut last: Local<Option<Lens>>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    let lens = explorer.lens();
    let opening = last.is_none();
    // A reveal set from the start frames what it shows too.
    let changed = last.map_or(lens.reach.is_some(), |l| l != lens);
    *last = Some(lens);
    if opening && !changed && explorer.anchor().is_none() {
        // Nothing to center on: open on the whole graph.
        for mut camera in &mut cameras {
            frame_all(&explorer, &mut camera);
        }
        return;
    }
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
        let anchor = explorer.anchor().and_then(|t| explorer.tile_point(t));
        frame_points(&points, anchor, &mut camera);
    }
}

/// When the spotlight comes on or widens, pulls back just far enough that everything
/// it picks out is on screen, keeping the focus in the middle; when it all already
/// shows, the view stays put. Once it shows routes alone, it closes in on them too.
pub(crate) fn frame_spotlight(
    explorer: Res<Explorer>,
    mut last: Local<Option<Spotlight>>,
    mut cameras: Query<(&mut OrbitCamera, &Transform, &Projection)>,
) {
    let spotlight = explorer.spotlight();
    if spotlight == *last {
        return;
    }
    *last = spotlight;
    if spotlight.is_none() {
        return;
    }
    let Some(center) = explorer.anchor().and_then(|t| explorer.tile_point(t)) else {
        return;
    };
    let points: Vec<Vec3> = explorer
        .spotlit()
        .into_iter()
        .filter_map(|t| explorer.tile_point(t))
        .collect();
    for (mut camera, transform, projection) in &mut cameras {
        let Projection::Perspective(perspective) = projection else {
            continue;
        };
        let needed = fit_distance(&points, center, transform.rotation, perspective);
        if needed > camera.distance || explorer.routes_only() {
            camera.glide_to(center, needed);
        }
    }
}

/// How far back a camera turned to `rotation` and looking at `center` must be for
/// every one of `points` to sit inside the middle of its view, clear of the edges.
fn fit_distance(
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

/// Looks at the middle of `points` from far enough back to see them all.
/// Around the anchor when there is one, which the camera never leaves; a few far
/// outliers do not push the view back.
fn frame_points(points: &[Vec3], anchor: Option<Vec3>, camera: &mut OrbitCamera) {
    if points.is_empty() {
        return;
    }
    let middle =
        anchor.unwrap_or_else(|| points.iter().copied().sum::<Vec3>() / points.len() as f32);
    let mut distances: Vec<f32> = points.iter().map(|p| p.distance(middle)).collect();
    distances.sort_by(f32::total_cmp);
    let radius = distances[(distances.len() * 9 / 10).min(distances.len() - 1)];
    camera.glide_to(middle, (radius * 2.6).max(MIN_ORBIT_DISTANCE));
}

/// Keeps the anchor, the node you clicked, dead center: the camera orbits it and
/// follows it as the graph moves.
/// Not once the viewer has panned or zoomed away.
pub(crate) fn hold_center(explorer: Res<Explorer>, mut cameras: Query<&mut OrbitCamera>) {
    let Some(at) = explorer.anchor().and_then(|t| explorer.tile_point(t)) else {
        return;
    };
    for mut camera in &mut cameras {
        if !camera.free {
            camera.target = at;
        }
    }
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
        .fold(journey_reach(explorer, at), f32::max);
    camera.glide_to(at, orbit_distance(reach));
}

/// How far from `at` the disclosed journey reaches.
fn journey_reach(explorer: &Explorer, at: Vec3) -> f32 {
    explorer
        .disclosure()
        .tiles()
        .keys()
        .filter_map(|&t| explorer.tile_point(t))
        .map(|p| p.distance(at))
        .fold(0.0, f32::max)
}

/// How far back to orbit to see `reach` either side of center: a 45° view shows
/// about 0.4 of its distance either side, and this leaves room.
fn orbit_distance(reach: f32) -> f32 {
    (reach * 3.4).max(MIN_ORBIT_DISTANCE)
}

/// Swings round to a newly focused node, and keeps it centered while the graph moves.
/// As more of the journey opens, pulls back to take it in.
pub(crate) fn follow_focus(
    explorer: Res<Explorer>,
    mut followed: Local<Option<usize>>,
    mut disclosed: Local<Option<Arc<Disclosed>>>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    let focus = explorer.graph_focus();
    let changed = focus != *followed;
    *followed = focus;
    let disclosure = explorer.disclosure();
    let opened = disclosed
        .as_ref()
        .is_some_and(|d| !Arc::ptr_eq(d, &disclosure));
    *disclosed = Some(disclosure);
    for mut camera in &mut cameras {
        if changed {
            orbit_focus(&explorer, &mut camera);
        } else if opened && let Some(at) = explorer.anchor().and_then(|t| explorer.tile_point(t)) {
            // Stepping through time only ever pulls back, so the view does not pump.
            let distance = orbit_distance(journey_reach(&explorer, at));
            if explorer.disclose() != Disclose::Time || distance > camera.distance {
                camera.glide_to(at, distance);
            }
        } else if explorer.settling()
            && !camera.free
            && let Some(at) = focus.and_then(|t| explorer.tile_point(t))
        {
            camera.target = at;
        }
    }
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
