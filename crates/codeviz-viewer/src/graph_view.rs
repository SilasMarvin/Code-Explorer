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
    explorer::{Explorer, Side},
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
        _ if explorer.spotlit().contains(&tile) => Some(1),
        _ => None,
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

/// Calls into the focus are violet and calls out of it blue, with dots flowing along
/// them; calls further along, nearer or further from it and on the active journey's
/// path, are pale arrows fading with distance; everything else is faint.
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
        let back = (b - a).normalize_or_zero() * node_size(graph, to) * 0.5;
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
    regions_of(
        explorer,
        (0..explorer.map().tiles.len()).filter(|&t| explorer.visibility(t) >= 0.5),
    )
}

/// Each module's region as `tiles` place it.
fn regions_of(explorer: &Explorer, tiles: impl Iterator<Item = usize>) -> Vec<(usize, Vec3, f32)> {
    let graph = explorer.graph();
    let mut members: Vec<Vec<Vec3>> = vec![Vec::new(); graph.cluster_count()];
    for tile in tiles {
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

/// Looks at the middle of the graph from far enough back to see all of it, rings
/// and all.
fn frame_all(explorer: &Explorer, camera: &mut OrbitCamera) {
    let tiles = 0..explorer.map().tiles.len();
    let points: Vec<Vec3> = tiles
        .clone()
        .filter_map(|t| explorer.tile_point(t))
        .chain(ring_points(explorer, tiles))
        .collect();
    if let Some(middle) = camera.view_middle(&points) {
        camera.frame(middle, points, MIN_ORBIT_DISTANCE);
    }
    // Away from the focus, so it stays where it was put.
    camera.free = true;
}

/// Points round the ring of each module `tiles` are in, so framing takes in the
/// rings as well as their nodes.
fn ring_points(explorer: &Explorer, tiles: impl Iterator<Item = usize>) -> Vec<Vec3> {
    const AROUND: usize = 12;
    regions_of(explorer, tiles)
        .into_iter()
        .flat_map(|(_, middle, radius)| {
            (0..AROUND).map(move |step| {
                #[expect(clippy::cast_precision_loss, reason = "a small count")]
                let angle = step as f32 / AROUND as f32 * std::f32::consts::TAU;
                middle + Vec3::new(angle.cos(), angle.sin(), 0.0) * radius
            })
        })
        .collect()
}

/// When the lens changes, or opens limited, refits the view to what it shows in full:
/// around the focus when that is where it starts, else around everything it reveals.
/// With nothing focused by hand, it opens on the whole graph.
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
    if opening && !changed && explorer.focused_by_hand().is_none() {
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
        let revealed = || {
            targets
                .iter()
                .enumerate()
                .filter(|&(_, &shown)| shown >= 0.5)
                .map(|(tile, _)| tile)
        };
        let points: Vec<Vec3> = revealed()
            .filter_map(|tile| explorer.tile_point(tile))
            .collect();
        let anchor = explorer.anchor().and_then(|t| explorer.tile_point(t));
        frame_points(
            points,
            ring_points(&explorer, revealed()),
            anchor,
            &mut camera,
        );
    }
}

/// When the spotlight comes on, pulls back just far enough that everything it picks
/// out is on screen, keeping the focus in the middle; when it all already shows, the
/// view stays put.
pub(crate) fn frame_spotlight(
    explorer: Res<Explorer>,
    mut last: Local<Option<(Side, f32)>>,
    mut cameras: Query<&mut OrbitCamera>,
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
    for mut camera in &mut cameras {
        let needed = camera.fit_distance(&points, center).max(MIN_ORBIT_DISTANCE);
        if needed > camera.distance {
            camera.frame(center, points.clone(), MIN_ORBIT_DISTANCE);
        }
    }
}

/// Looks at the middle of `points` and their modules' `rings` from far enough back to
/// see them all. Around the anchor when there is one, which the camera never leaves;
/// among many, the farthest few points do not push the view back.
fn frame_points(
    mut points: Vec<Vec3>,
    rings: Vec<Vec3>,
    anchor: Option<Vec3>,
    camera: &mut OrbitCamera,
) {
    /// Points enough that the farthest tenth count as outliers.
    const MANY: usize = 30;
    let Some(middle) = anchor.or_else(|| camera.view_middle(&points)) else {
        return;
    };
    if points.len() >= MANY {
        points.sort_by(|a, b| a.distance(middle).total_cmp(&b.distance(middle)));
        points.truncate((points.len() * 9).div_ceil(10));
    } else {
        points.extend(rings);
    }
    camera.frame(middle, points, MIN_ORBIT_DISTANCE);
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

/// Orbits the focused node from far enough back to see everything it calls,
/// everything that calls it and the journey disclosed from it.
pub(crate) fn orbit_focus(explorer: &Explorer, camera: &mut OrbitCamera) {
    let Some(focus) = explorer.graph_focus() else {
        return;
    };
    let Some(at) = explorer.tile_point(focus) else {
        return;
    };
    camera.frame(at, orbit_points(explorer, focus), MIN_ORBIT_DISTANCE);
}

/// What orbiting `focus` keeps in view: what calls it and what it calls, as far as
/// they show, and what its journey discloses.
fn orbit_points(explorer: &Explorer, focus: usize) -> Vec<Vec3> {
    let graph = explorer.graph();
    graph
        .callers(focus)
        .chain(graph.callees(focus))
        .filter(|&t| explorer.shown(t))
        .chain(explorer.disclosure().tiles().keys().copied())
        .filter_map(|t| explorer.tile_point(t))
        .collect()
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
        } else if opened
            && let Some(anchor) = explorer.anchor()
            && let Some(at) = explorer.tile_point(anchor)
        {
            // Stepping through time only ever pulls back, so the view does not pump.
            let points = orbit_points(&explorer, anchor);
            let distance = camera.fit_distance(&points, at).max(MIN_ORBIT_DISTANCE);
            if explorer.disclose() != Disclose::Time || distance > camera.distance {
                camera.frame(at, points, MIN_ORBIT_DISTANCE);
            }
        } else if explorer.settling()
            && !camera.free
            && let Some(at) = focus.and_then(|t| explorer.tile_point(t))
        {
            camera.target = at;
        }
    }
}
