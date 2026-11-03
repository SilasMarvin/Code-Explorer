//! How functions look: each node's color and size, and the rings round the start (or
//! the selection), the function set aside, the cursor and what is hovered. A node's
//! fill says what it is to the selection or the journey, in the colors its calls and
//! names carry: white, violet, blue, amber darker the deeper, pink, and grey for the
//! rest; its module shows in its sphere alone. Its size says only what it is to the
//! view ([`Dot`]): a faint speck, lit, or the one in focus. Hovering only rings the
//! node, so passing over the graph does not repaint it.

use std::collections::HashMap;

use bevy::prelude::*;

use crate::{
    camera::OrbitCamera,
    colors::{CALLER, CURSOR, FURTHER, HOVER, SELECTED},
    drive::{Marks, RingMark},
    explorer::Explorer,
    nucleus::{Flow, SHADES},
    scene::Tile,
    sizes::Dot,
};

/// How a node is drawn right now: in the color of what it is to the selection or
/// the journey, as its call and its name are; its module shows in its sphere alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum NodeLook {
    /// The start or the selection: white. The function set aside keeps its role's
    /// fill, and its ring alone is white.
    Start,
    /// The journey's cursor: pink.
    Cursor,
    /// What the start or the selection lights, colored as the call that lights it:
    /// violet for a caller, blue for its own calls, amber further down.
    Role(Flow),
    /// Anything else while nothing is selected, and what is lit for no call: grey.
    Plain,
    /// Unrelated to what is selected: faint grey.
    Faded,
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

/// How each look is drawn at full brightness: flat, with only enough light of its
/// own to read on the dark, and no glow.
fn node_looks() -> HashMap<NodeLook, StandardMaterial> {
    let flat = |color: Color| StandardMaterial {
        base_color: color,
        emissive: LinearRgba::from(color) * 0.5,
        ..default()
    };
    let grey = FURTHER.mix(&Color::BLACK, 0.35);
    let mut table = HashMap::new();
    table.insert(NodeLook::Start, flat(SELECTED));
    table.insert(NodeLook::Cursor, flat(CURSOR));
    for flow in [Flow::Into]
        .into_iter()
        .chain((1..=SHADES).map(Flow::journey))
    {
        table.insert(NodeLook::Role(flow), flat(flow.color()));
    }
    table.insert(NodeLook::Plain, flat(grey));
    table.insert(
        NodeLook::Faded,
        StandardMaterial {
            base_color: grey.with_alpha(0.12),
            alpha_mode: AlphaMode::Blend,
            unlit: true,
            ..default()
        },
    );
    table
}

/// What a function is to the view.
pub(crate) fn dot(explorer: &Explorer, tile: usize) -> Dot {
    let nucleus = explorer.nucleus();
    let focus = [nucleus.start(), explorer.cursor(), explorer.aside()];
    if focus.contains(&Some(tile)) {
        Dot::Focus
    } else if nucleus.lit(tile) && !nucleus.folded(tile) {
        Dot::Lit
    } else {
        Dot::Speck
    }
}

/// How big a function is drawn in the world, seen from `view` through `camera`: its
/// role's size, less where its module's functions sit close, kept within that role's
/// bounds on screen.
pub(crate) fn drawn_radius(
    explorer: &Explorer,
    tile: usize,
    camera: &Camera,
    view: &GlobalTransform,
) -> f32 {
    let dot = dot(explorer, tile);
    let world = dot.radius() * explorer.nucleus().dot_scale(tile);
    let shows = |at: Vec3| camera.world_to_viewport(view, at).ok();
    let per_unit = explorer
        .tile_point(tile)
        .and_then(|at| Some(shows(at)?.distance(shows(at + view.up() * world)?) / world));
    match per_unit {
        Some(per_unit) if per_unit > f32::EPSILON => dot.clamp(world * per_unit) / per_unit,
        _ => world,
    }
}

pub(crate) fn node_look(explorer: &Explorer, tile: usize) -> NodeLook {
    // The journey's cursor is the one pink node, wherever it is.
    if explorer.cursor() == Some(tile) {
        return NodeLook::Cursor;
    }
    role_look(explorer, tile)
}

/// What a function is to the view, were the cursor not on it: how the panel paints
/// the bars of its other runs.
pub(crate) fn role_look(explorer: &Explorer, tile: usize) -> NodeLook {
    let nucleus = explorer.nucleus();
    if nucleus.start() == Some(tile) {
        return NodeLook::Start;
    }
    match nucleus.level(tile) {
        Some(-1) => NodeLook::Role(Flow::Into),
        Some(level) => NodeLook::Role(Flow::journey(level.unsigned_abs())),
        None if nucleus.lit(tile) || nucleus.start().is_none() => NodeLook::Plain,
        None => NodeLook::Faded,
    }
}

/// The color a node is filled with, at full strength, as [`node_look`] has it. A
/// journey bar is painted the same, so a bar and its node read as one thing.
pub(crate) fn node_color(explorer: &Explorer, tile: usize) -> Color {
    look_color(node_look(explorer, tile))
}

/// The color a look fills with, at full strength.
pub(crate) fn look_color(look: NodeLook) -> Color {
    match look {
        NodeLook::Start => SELECTED,
        NodeLook::Cursor => CURSOR,
        NodeLook::Role(flow) => flow.color(),
        NodeLook::Plain | NodeLook::Faded => FURTHER,
    }
}

/// Restyles nodes when the view changes.
pub(crate) fn style_nodes(
    explorer: Res<Explorer>,
    materials: Res<NodeMaterials>,
    mut nodes: Query<(
        &Tile,
        &mut MeshMaterial3d<StandardMaterial>,
        &mut Visibility,
    )>,
) {
    if !explorer.is_changed() {
        return;
    }
    let nucleus = explorer.nucleus();
    for (tile, mut material, mut visibility) in &mut nodes {
        let drawn = nucleus.drawn(tile.0);
        visibility.set_if_neq(if drawn {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if !drawn {
            continue;
        }
        let look = node_look(&explorer, tile.0);
        // Brought in from outside the journey, a caller of the start folded into its
        // module's count, a call only guessed, or an expanded module's function nothing
        // lights: there, but quieter than the journey.
        let outside = (nucleus.outside(tile.0)
            || nucleus.folded(tile.0)
            || guessed(&explorer, tile.0)
            || plain_beside(&explorer, tile.0))
            && explorer.aside() != Some(tile.0);
        if let Some(wanted) = materials.0.get(&(look, outside))
            && &material.0 != wanted
        {
            material.0 = wanted.clone();
        }
    }
}

/// Whether a function is one of the expanded module's that nothing lights, while a
/// journey or a selection lights others: drawn and named quieter than those.
pub(crate) fn plain_beside(explorer: &Explorer, tile: usize) -> bool {
    let nucleus = explorer.nucleus();
    nucleus.start().is_some()
        && nucleus.opened_up(tile)
        && nucleus.level(tile).is_none()
        && !nucleus.outside(tile)
        && nucleus.start() != Some(tile)
        && explorer.cursor() != Some(tile)
}

/// Whether every call to a function the journey shows is only guessed, or handed
/// off: its dot is drawn faint, as its bars are.
pub(crate) fn guessed(explorer: &Explorer, tile: usize) -> bool {
    !explorer.browsing()
        && explorer.cursor() != Some(tile)
        && explorer.disclosure().tile_guessed(tile)
}

#[derive(Resource, Debug)]
pub(crate) struct NodeMaterials(pub(crate) HashMap<(NodeLook, bool), Handle<StandardMaterial>>);

pub(crate) fn create_node_materials(
    mut commands: Commands,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.insert_resource(NodeMaterials(node_materials(&mut materials)));
}

/// The orbit camera, apart from the functions.
type TheCamera = (With<OrbitCamera>, Without<Tile>);

/// Sizes every function drawn as [`drawn_radius`] has it, whenever the view or the
/// camera changes.
pub(crate) fn size_nodes(
    explorer: Res<Explorer>,
    cameras: Query<(Ref<Camera>, Ref<Transform>), TheCamera>,
    mut nodes: Query<(&Tile, &mut Transform)>,
) {
    let Ok((camera, placed)) = cameras.single() else {
        return;
    };
    if !explorer.is_changed() && !camera.is_changed() && !placed.is_changed() {
        return;
    }
    // Where the camera is this frame: it moves before its transform reaches the rest.
    let view = GlobalTransform::from(*placed);
    for (tile, mut transform) in &mut nodes {
        // The mesh is one unit across.
        let size = 2.0 * drawn_radius(&explorer, tile.0, &camera, &view);
        if (transform.scale.x - size).abs() > size * 1e-3 {
            transform.scale = Vec3::splat(size);
        }
    }
}

/// At most one thin ring per function, facing the camera, nothing else: white round
/// the start (or the selection), and in a journey round the function set aside; pink
/// round the journey's cursor, whose fill is pink too; violet round a call that also
/// calls the start back, so it reads as both; grey round the function hovered, in the
/// graph or as a bar, so pointing never looks like selecting.
pub(crate) fn draw_graph(
    explorer: Res<Explorer>,
    cameras: Query<(&Camera, &GlobalTransform), With<OrbitCamera>>,
    mut gizmos: Gizmos,
    mut marks: ResMut<Marks>,
) {
    marks.rings.clear();
    let Ok((camera, view)) = cameras.single() else {
        return;
    };
    let facing = view.rotation();
    let mut ring = |tile: usize, color: Color| {
        if let Some(at) = explorer.tile_point(tile) {
            let radius = drawn_radius(&explorer, tile, camera, view) * RING;
            gizmos
                .circle(Isometry3d::new(at, facing), radius, color)
                .resolution(32);
            marks.rings.push(RingMark {
                tile,
                radius,
                color,
            });
        }
    };
    let nucleus = explorer.nucleus();
    let start = nucleus.start();
    for tile in start.into_iter().chain(explorer.aside()) {
        ring(tile, SELECTED);
    }
    let cursor = explorer.cursor();
    if let Some(tile) = cursor {
        ring(tile, CURSOR);
    }
    let ringed = |t: usize| [start, cursor, explorer.aside()].contains(&Some(t));
    let mut both: Vec<usize> = nucleus
        .links()
        .iter()
        .map(|l| l.from)
        .filter(|&t| !ringed(t) && nucleus.drawn(t) && nucleus.calls_back(t))
        .collect();
    both.sort_unstable();
    both.dedup();
    for &tile in &both {
        ring(tile, CALLER);
    }
    if let Some(tile) = explorer
        .hovered()
        .filter(|&t| !ringed(t) && !both.contains(&t))
    {
        ring(tile, HOVER);
    }
}

/// How far out a ring sits, against its function's radius.
const RING: f32 = 1.6;
