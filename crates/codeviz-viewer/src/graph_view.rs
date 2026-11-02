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
    colors::{CURSOR, FURTHER},
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
    /// The start, the selection, or the function set aside: white.
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
    table.insert(NodeLook::Start, flat(Color::WHITE));
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
/// role's size, kept within that role's bounds on screen.
pub(crate) fn drawn_radius(
    explorer: &Explorer,
    tile: usize,
    camera: &Camera,
    view: &GlobalTransform,
) -> f32 {
    let dot = dot(explorer, tile);
    let world = explorer.nucleus().dot_radius(dot);
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
    if nucleus.start() == Some(tile) || explorer.aside() == Some(tile) {
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
        NodeLook::Start => Color::WHITE,
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
        // Brought in from outside the journey, or a caller of the start folded into its
        // module's count: there, but quieter than the journey.
        let outside =
            (nucleus.outside(tile.0) || nucleus.folded(tile.0)) && explorer.aside() != Some(tile.0);
        if let Some(wanted) = materials.0.get(&(look, outside))
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

/// One ring per role, nothing else: white round the start (or the selection), and in
/// a journey round the function set aside; pink round the journey's cursor, whose
/// fill is pink too; fainter white round the function hovered, in the graph or as a
/// bar. Each a share of its function's size out from it.
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
    let mut ring = |tile: usize, grow: f32, color: Color| {
        if let Some(at) = explorer.tile_point(tile) {
            let radius = drawn_radius(&explorer, tile, camera, view) * grow;
            gizmos.sphere(Isometry3d::from_translation(at), radius, color);
            marks.rings.push(RingMark {
                tile,
                radius,
                color,
            });
        }
    };
    let start = explorer.nucleus().start();
    for tile in start.into_iter().chain(explorer.aside()) {
        ring(tile, 1.5, Color::WHITE);
    }
    if let Some(tile) = explorer.cursor() {
        ring(tile, 1.5, CURSOR);
    }
    if let Some(tile) = explorer.hovered().filter(|&t| Some(t) != start) {
        ring(tile, 1.8, Color::srgba(1.0, 1.0, 1.0, 0.55));
    }
}
