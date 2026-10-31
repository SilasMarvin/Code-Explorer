//! The codebase as a graph of calls: how its nodes look, which links light up, and
//! where the camera goes. The focus, the journey cursor's node, is the center of
//! attention; the pointer only rings the node it is over, so passing over the graph
//! does not repaint it. Each channel carries one thing: a link's color says which
//! way a call runs (violet into the focus, blue out of it, with dots flowing along
//! it), a node's fill says where it lives (its module) or, under a diff, what
//! changed. What is near the focus is bright, fading with each call further; the rest
//! of the graph fades so that neighborhood reads on its own.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};
use codeviz_core::Change;
use codeviz_layout::{ForceGraph, Reach};

use crate::{
    camera::{FRONT, OrbitCamera},
    diff_view::{
        ADDED, DiffView, MODIFIED, MOVED, REMOVED, change_color, diff_look, draw_diff_graph,
    },
    disclosure::{Disclose, Disclosed},
    explorer::{Explorer, Side},
    idle::Awake,
    journey_panel::{JourneyPanel, PanelMode},
    lens::{Lens, SPECK, Source},
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
/// What a diff left alone.
pub(crate) const UNCHANGED: Color = Color::srgb(0.6, 0.64, 0.72);
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

/// [`relation`], but none on a side of the focus that is hidden.
pub(crate) fn shown_relation(explorer: &Explorer, reach: &Reach, tile: usize) -> Option<Relation> {
    let hidden = explorer.hidden();
    relation(reach, tile).filter(|r| match r {
        Relation::Upstream(_) => !hidden.callers,
        Relation::Downstream(_) => !hidden.callees,
    })
}

/// Gathered round the focus's module, whether a function is drawn only as a small,
/// faint speck: shown, but not the middle and named by nothing.
pub(crate) fn speck(explorer: &Explorer, tile: usize) -> bool {
    explorer
        .nucleus()
        .is_some_and(|n| n.shows(tile) && !n.lit(tile) && n.focus() != Some(tile))
}

/// Whether a function is a speck the nucleus leaves out altogether (N): not drawn,
/// and not pointed at.
pub(crate) fn hidden_speck(explorer: &Explorer, tile: usize) -> bool {
    explorer
        .nucleus()
        .is_some_and(nucleus::Nucleus::specks_hidden)
        && speck(explorer, tile)
}

pub(crate) fn node_look(explorer: &Explorer, tile: usize) -> NodeLook {
    let color = explorer.tile_color(tile) % PALETTE.len();
    if !explorer.shown(tile) {
        return NodeLook::Hidden;
    }
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
    // Gathered round the focus's module, whatever it names lights.
    let named = explorer.nucleus().is_some_and(|n| n.names(tile));
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

/// The color a node is filled with, at full strength: white for the focus, else what
/// changed under a diff, else the cursor's where it is not the focus, else its
/// module's. A journey bar is painted the same, so a bar and its node read as one thing.
pub(crate) fn node_color(explorer: &Explorer, diff: Option<&DiffView>, tile: usize) -> Color {
    if diff.is_none() && explorer.cursor_mark() == Some(tile) {
        return CURSOR;
    }
    if explorer.lit_focus() == Some(tile) {
        return Color::WHITE;
    }
    match diff {
        Some(diff) => diff.change(tile).map_or(UNCHANGED, change_color),
        None => lane_color(explorer.tile_color(tile) % PALETTE.len()),
    }
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
    diff: Option<Res<DiffView>>,
    materials: Res<NodeMaterials>,
    mut nodes: Query<(
        &Tile,
        &mut MeshMaterial3d<StandardMaterial>,
        &mut Transform,
        &mut Visibility,
    )>,
) {
    if !explorer.is_changed() && !diff.as_ref().is_some_and(DetectChanges::is_changed) {
        return;
    }
    let nucleus = explorer.nucleus();
    for (tile, mut material, mut transform, mut visibility) in &mut nodes {
        // Gathered round the focus's module, what a module folds away is not drawn.
        let folded = nucleus.is_some_and(|n| !n.shows(tile.0));
        let look = (!folded).then(|| match &diff {
            Some(diff) => diff_look(&explorer, diff, tile.0),
            None => node_look(&explorer, tile.0),
        });
        // Gathered round the focus's module, what it names, or what the function under
        // the pointer calls or is called by, shows plainly though the focus does not
        // reach it; anything else is a small, faint speck, or with specks left out, not
        // drawn at all, however near the focus it is.
        let named = nucleus.is_some_and(|n| n.lit(tile.0));
        let quiet = speck(&explorer, tile.0);
        // Brought in from outside the journey: there, but quieter than the journey.
        let outside =
            nucleus.is_some_and(|n| n.outside(tile.0)) && explorer.aside() != Some(tile.0);
        let gone = quiet && nucleus.is_some_and(nucleus::Nucleus::specks_hidden);
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
        let (scale, level) = match nucleus {
            Some(n) if quiet => (n.node_scale(true), LEVELS),
            Some(n) if outside => (0.6 * n.node_scale(false), LEVELS / 3),
            Some(n) => (scale * n.node_scale(false), level),
            None => (scale, level),
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

/// Every call as a link, then the spotlight, the focus and the node under the pointer.
pub(crate) fn draw_graph(
    explorer: Res<Explorer>,
    diff: Option<Res<DiffView>>,
    clock: Res<Time>,
    mut gizmos: Gizmos,
    mut marked: Local<Option<(usize, f32)>>,
    mut awake: ResMut<Awake>,
) {
    let gathered = explorer.nucleus().is_some();
    if !gathered {
        draw_regions(&explorer, &mut gizmos);
    }
    let time = clock.elapsed_secs();
    match diff {
        Some(diff) => draw_diff_graph(&explorer, &diff, time, &mut gizmos),
        // Gathered round the focus's module, the nucleus draws its own calls.
        None if gathered => {}
        None => draw_calls(&explorer, time, &mut gizmos),
    }
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

/// Calls into the focus are violet and calls out of it blue, with dots flowing along
/// them; calls further down from it are amber and further up pale, fading with
/// distance, as are calls on the journey's path; everything else is faint.
fn draw_calls(explorer: &Explorer, time: f32, gizmos: &mut Gizmos) {
    let graph = explorer.graph();
    let center = explorer.graph_focus();
    let reach = explorer.reach();
    let hidden = explorer.hidden();
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
            // A side of the focus that is hidden lights nothing.
            let up = hops(&reach.upstream, from)
                .zip(hops(&reach.upstream, to))
                .filter(|_| !hidden.callers);
            let down = hops(&reach.downstream, from)
                .zip(hops(&reach.downstream, to))
                .filter(|_| !hidden.callees);
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
            // Further down from the focus, one other color; further up, pale.
            Some((color, outer)) => {
                let color = if color == CALLEE { DEEPER } else { FURTHER };
                gizmos
                    .arrow(a, b - back, color.with_alpha(fade(outer) * 0.8 * shown))
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

/// A white ring round the focus, and a fainter, wider one round the node under the
/// pointer, in the graph or as a bar in the journey. Gathered round the focus's
/// module, the journey's cursor, where it is not the nucleus's middle, gets two rings
/// where it sits in a glow of its own color, pulsing for a moment `age` seconds after it landed;
/// the function under the pointer, a bar's included, glows as a preview of that.
fn draw_attention(explorer: &Explorer, age: f32, gizmos: &mut Gizmos) {
    let graph = explorer.graph();
    let nucleus = explorer.nucleus();
    // The node's own radius, as big as it is drawn.
    let size =
        |tile: usize| node_size(graph, tile) * nucleus.map_or(1.0, |n| n.node_scale(false)) * 0.5;
    let focus = explorer.lit_focus();
    // The focus, and in a journey a function selected off it, ringed in white.
    let ringed = focus
        .into_iter()
        .chain(explorer.aside().filter(|_| nucleus.is_some()));
    for tile in ringed {
        if let Some(at) = explorer.tile_point(tile) {
            let radius = size(tile) + 0.35;
            gizmos.sphere(Isometry3d::from_translation(at), radius, Color::WHITE);
        }
    }
    let zoom = nucleus.map_or(1.0, nucleus::Nucleus::zoom);
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
    if nucleus.is_some()
        && let Some(tile) = explorer
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

/// F turns to face the graph head-on, centered on the journey's start again (or,
/// gathered into spheres, back to the three-quarter view, framed as it was when the
/// view was pointed there); R frames that same view from the angle the camera has, or
/// flat, pulls back to take in the whole graph, or browsing, turns back to the usual
/// view of the whole codebase; Q and E turn the view an eighth of the way round, left
/// and right.
pub(crate) fn graph_keys(
    keys: Res<ButtonInput<KeyCode>>,
    explorer: Res<Explorer>,
    mut cameras: Query<(&mut OrbitCamera, &Projection)>,
) {
    let cursor = explorer.graph_focus().and_then(|t| explorer.tile_point(t));
    for (mut camera, projection) in &mut cameras {
        if keys.just_pressed(KeyCode::KeyF) {
            if let Some(gathered) = explorer.nucleus() {
                camera.turn_to(nucleus::VIEW);
                let (yaw, pitch) = nucleus::VIEW;
                let rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
                nucleus::fit(gathered, cursor, &mut camera, rotation, projection);
            } else {
                camera.turn_to(FRONT);
                orbit_anchor(&explorer, &mut camera);
            }
        }
        for (key, turn) in [(KeyCode::KeyQ, 1.0), (KeyCode::KeyE, -1.0)] {
            if keys.just_pressed(key) {
                camera.turn_by(turn * std::f32::consts::FRAC_PI_4);
                // The nucleus refits to what the new angle sees.
                if let Some(gathered) = explorer.nucleus() {
                    let rotation = camera.aim_rotation();
                    nucleus::fit(gathered, cursor, &mut camera, rotation, projection);
                }
            }
        }
        if keys.just_pressed(KeyCode::KeyR) {
            match explorer.nucleus() {
                Some(gathered) if gathered.browsing() => {
                    // The whole codebase, as it was first seen: from the front.
                    camera.turn_to(nucleus::VIEW);
                    let rotation = camera.aim_rotation();
                    nucleus::fit_all(gathered, &mut camera, rotation, projection);
                }
                Some(gathered) => {
                    let rotation = camera.aim_rotation();
                    nucleus::fit(gathered, cursor, &mut camera, rotation, projection);
                }
                None => frame_all(&explorer, &mut camera),
            }
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
    if explorer.nucleus().is_some() {
        return;
    }
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
        if lens.source == Source::Focus && !lens.changes_only && explorer.anchor().is_some() {
            orbit_anchor(&explorer, &mut camera);
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

/// When the spotlight comes on, pulls back just far enough that everything it picks
/// out is on screen, keeping the focus in the middle; when it all already shows, the
/// view stays put.
pub(crate) fn frame_spotlight(
    explorer: Res<Explorer>,
    mut last: Local<Option<(Side, f32)>>,
    mut cameras: Query<(&mut OrbitCamera, &Transform, &Projection)>,
) {
    let spotlight = explorer.spotlight();
    if spotlight == *last || explorer.nucleus().is_some() {
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
        .chain(explorer.graph_focus())
        .filter_map(|t| explorer.tile_point(t))
        .collect();
    for (mut camera, transform, projection) in &mut cameras {
        let Projection::Perspective(perspective) = projection else {
            continue;
        };
        let needed = fit_distance(&points, center, transform.rotation, perspective);
        if needed > camera.distance {
            camera.glide_to(center, needed);
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

/// Keeps the anchor, where the journey starts, dead center: the camera orbits it and
/// follows it as the graph moves. Gathered into spheres, nothing moves but by hand,
/// so the camera stays where it was framed. Not once the viewer has panned or zoomed
/// away.
pub(crate) fn hold_center(explorer: Res<Explorer>, mut cameras: Query<&mut OrbitCamera>) {
    if explorer.nucleus().is_some() {
        return;
    }
    let Some(at) = explorer.anchor().and_then(|t| explorer.tile_point(t)) else {
        return;
    };
    for mut camera in &mut cameras {
        if !camera.free {
            camera.target = at;
        }
    }
}

/// Orbits the node the journey starts at from far enough back to see everything it
/// calls and everything that calls it, the journey as disclosed, and the cursor.
pub(crate) fn orbit_anchor(explorer: &Explorer, camera: &mut OrbitCamera) {
    let graph = explorer.graph();
    let Some(anchor) = explorer.anchor() else {
        return;
    };
    let Some(at) = explorer.tile_point(anchor) else {
        return;
    };
    let reach = graph
        .callers(anchor)
        .chain(graph.callees(anchor))
        .filter(|&t| explorer.shown(t))
        .chain(explorer.graph_focus())
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

/// What the camera last followed: where it was aimed and how many times, the focus,
/// and how much of the journey showed.
#[derive(Default)]
pub(crate) struct Followed {
    aim: Option<(Option<usize>, u64)>,
    focus: Option<usize>,
    disclosed: Option<Arc<Disclosed>>,
}

/// Turns to the node the journey starts at whenever the view is pointed somewhere by
/// hand: a node clicked, searched for, numbered or picked from the changes, going back
/// or forward, another journey. Moving the cursor never turns it: at most the camera
/// pulls back, around the same node, so the cursor's node and whatever opened for it
/// stay on screen, and not even that once the viewer has panned or zoomed away. A
/// level more or less of the journey refits the view to what shows.
pub(crate) fn follow_focus(
    explorer: Res<Explorer>,
    mut last: Local<Followed>,
    mut cameras: Query<(&mut OrbitCamera, &Transform, &Projection)>,
) {
    let aim = (explorer.anchor(), explorer.aim());
    let focus = explorer.graph_focus();
    let disclosure = explorer.disclosure();
    let aimed = last.aim != Some(aim);
    let opened = last
        .disclosed
        .as_ref()
        .is_some_and(|d| !Arc::ptr_eq(d, &disclosure));
    let moved = last.focus != focus;
    *last = Followed {
        aim: Some(aim),
        focus,
        disclosed: Some(disclosure),
    };
    let Some(at) = explorer.anchor().and_then(|t| explorer.tile_point(t)) else {
        return;
    };
    // Gathered round the focus's module, the nucleus frames itself.
    if explorer.nucleus().is_some() {
        return;
    }
    for (mut camera, transform, projection) in &mut cameras {
        if aimed {
            orbit_anchor(&explorer, &mut camera);
            continue;
        }
        if camera.free || !(opened || moved) {
            continue;
        }
        let Projection::Perspective(perspective) = projection else {
            continue;
        };
        let cursor: Vec<Vec3> = focus
            .and_then(|t| explorer.tile_point(t))
            .into_iter()
            .collect();
        let mut needed = fit_distance(&cursor, at, transform.rotation, perspective);
        if opened {
            needed = needed.max(orbit_distance(journey_reach(&explorer, at)));
        }
        // A level more or less refits either way; anything the cursor did only ever
        // pulls back, so stepping never pumps the view in and out.
        let refit = opened && !moved && explorer.disclose() != Disclose::Time;
        if refit || needed > camera.distance {
            camera.glide_to(at, needed);
        }
    }
}

/// Seconds after the cursor moves, or the view is pointed somewhere, in which the
/// camera still makes sure the cursor shows: time for the nucleus to settle round it.
const REVEAL_WATCH: f32 = 2.0;
/// The share of the half-view the cursor may use before the camera pulls back for it,
/// leaving room for its name.
const REVEAL_ROOM: f32 = 0.82;
/// Room kept between the cursor and the docked journey, in the view's half-widths.
const PANEL_ROOM: f32 = 0.06;

/// What the cursor showing depends on: the cursor, the nucleus's middle, the
/// journey's start, the hand that last pointed the view, and how much of it shows.
type Watched = (Option<usize>, Option<usize>, Option<usize>, u64, Disclose);

/// When the cursor lands off screen, or under the docked journey, pulls back just far
/// enough to bring it into view, round wherever the camera looks, without framing
/// anything else afresh. Only after the cursor moves or the view changes, so a view
/// panned or zoomed by hand stays as it was put unless the cursor would leave it.
pub(crate) fn reveal_cursor(
    time: Res<Time>,
    explorer: Res<Explorer>,
    panel: Res<JourneyPanel>,
    mut contexts: EguiContexts,
    mut last: Local<Option<Watched>>,
    mut until: Local<f32>,
    mut cameras: Query<(&mut OrbitCamera, &Camera, &GlobalTransform)>,
) {
    let cursor = explorer.graph_focus();
    let watched = (
        cursor,
        explorer.lit_focus(),
        explorer.anchor(),
        explorer.aim(),
        explorer.disclose(),
    );
    let now = time.elapsed_secs();
    if last.replace(watched) != Some(watched) {
        *until = now + REVEAL_WATCH;
    }
    // Gathered into spheres, the cursor moves and the camera does not: the journey
    // was framed whole on entering it.
    if now > *until || explorer.nucleus().is_some() {
        return;
    }
    let Some(at) = cursor.and_then(|t| explorer.tile_point(t)) else {
        return;
    };
    let docked = (panel.mode == PanelMode::Docked)
        .then(|| contexts.ctx_mut().ok())
        .flatten()
        .and_then(|ctx| ctx.memory(|m| m.area_rect(egui::Id::new("journey_panel"))));
    for (mut orbit, camera, view) in &mut cameras {
        // Whatever the camera is easing to may show it already.
        if orbit.easing() {
            continue;
        }
        let Some(ndc) = camera.world_to_ndc(view, at) else {
            continue;
        };
        let depth = (at - view.translation()).dot(*view.forward());
        let covered = docked.and_then(|rect| {
            let area = camera.logical_viewport_rect()?;
            let to_ndc = |p: egui::Pos2| {
                let r = (Vec2::new(p.x, p.y) - area.min) / area.size();
                Vec2::new(r.x * 2.0 - 1.0, 1.0 - r.y * 2.0)
            };
            Some(Rect::from_corners(to_ndc(rect.min), to_ndc(rect.max)))
        });
        if let Some(distance) = reveal_distance(ndc.truncate(), depth, orbit.distance, covered) {
            orbit.pull_back(distance);
        }
    }
}

/// How far from what it orbits a camera now `distance` away must pull back for a point
/// at `ndc` on its view, `depth` ahead of it, to show within [`REVEAL_ROOM`] of the
/// middle and clear of `covered` (in the same terms); `None` when it already does.
/// Pulling back draws every point in towards the middle of the view, along the line
/// from it, by the ratio of its depths before and after.
fn reveal_distance(ndc: Vec2, depth: f32, distance: f32, covered: Option<Rect>) -> Option<f32> {
    if depth <= 0.1 {
        // Behind the camera: back past it, and look again.
        return Some(distance - depth + distance * 0.5);
    }
    // The share of its offset from the middle the point may keep.
    let mut keep = [ndc.x, ndc.y]
        .into_iter()
        .filter(|v| v.abs() > REVEAL_ROOM)
        .map(|v| REVEAL_ROOM / v.abs())
        .fold(1.0_f32, f32::min);
    if let Some(rect) = covered.map(|r| r.inflate(PANEL_ROOM))
        && !rect.contains(Vec2::ZERO)
        && rect.contains(ndc * keep)
    {
        // Along the line to the middle, it leaves the cover where the line meets it.
        let enter = [
            (ndc.x, rect.min.x, rect.max.x),
            (ndc.y, rect.min.y, rect.max.y),
        ]
        .into_iter()
        .filter(|&(v, ..)| v.abs() > f32::EPSILON)
        .map(|(v, low, high)| {
            let near = if v > 0.0 { low } else { high };
            (near / v).max(0.0)
        })
        .fold(0.0_f32, f32::max);
        keep = keep.min(enter * 0.98);
    }
    (keep < 0.999 && keep > 0.0).then(|| distance + depth / keep - depth)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulls_back_for_the_cursor_only_as_far_as_it_must() {
        assert_eq!(
            reveal_distance(Vec2::new(0.3, -0.2), 50.0, 40.0, None),
            None,
            "on screen already"
        );
        let back = reveal_distance(Vec2::new(1.64, 0.0), 50.0, 40.0, None).unwrap();
        assert!(
            (back - (40.0 + 50.0)).abs() < 1e-3,
            "twice as deep halves its offset: {back}"
        );
        let panel = Rect::from_corners(Vec2::new(0.0, -1.0), Vec2::new(1.0, -0.6));
        let under = reveal_distance(Vec2::new(0.5, -0.8), 50.0, 40.0, Some(panel)).unwrap();
        let ndc = Vec2::new(0.5, -0.8) * (50.0 / (50.0 + under - 40.0));
        assert!(
            ndc.y > -0.6 + PANEL_ROOM - 0.02 && ndc.y < -0.5,
            "just clear of the journey: {ndc}"
        );
        assert_eq!(
            reveal_distance(Vec2::new(-0.5, -0.8), 50.0, 40.0, Some(panel)),
            None,
            "beside it is clear"
        );
    }

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
