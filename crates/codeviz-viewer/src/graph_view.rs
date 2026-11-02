//! How functions look: each node's color and size, and the rings round the start (or
//! the selection), the function set aside, the cursor and what is hovered. A node's
//! fill says where it lives, its module; what is near the start is bright, fading with
//! each call further. Its size says only what it is to the view, one size for each:
//! a faint speck, lit, or the one in focus; kept within bounds on screen however near
//! or far the view. Hovering only rings the node, so passing over the graph does not
//! repaint it.

use std::collections::HashMap;

use bevy::prelude::*;
use codeviz_layout::Reach;

use crate::{
    camera::OrbitCamera,
    colors::{CURSOR, MODULE_SHADES, module_color, module_shade},
    explorer::Explorer,
    scene::Tile,
};

/// Calls away from the start that still get their own shade; further is drawn as this.
const FARTHEST_SHADE: u32 = 3;

/// How a node is drawn right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum NodeLook {
    /// The start, or browsing, the selection.
    Start,
    /// The journey's cursor.
    Cursor,
    /// Calls into the start or called from it, this many calls away (capped at
    /// [`FARTHEST_SHADE`]), in its module's color.
    Near(usize, u32),
    /// Unrelated to the start.
    Faded(usize),
    /// Anything while nothing is selected, in its module's color.
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
    table.insert(NodeLook::Start, glow(Color::WHITE, 4.0));
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
        for color in 0..MODULE_SHADES {
            table.insert(
                NodeLook::Near(color, hops),
                // Enough glow to stand out, little enough that the hue holds.
                glow(darken(module_color(color)), strength * 0.3),
            );
        }
    }
    for color in 0..MODULE_SHADES {
        let base = module_color(color);
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

/// What a function is to the view, which alone sets how big it is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dot {
    /// Lit by nothing, or a caller of the start folded into a count.
    Speck,
    /// What the selection or the journey lights, and callers brought in from outside.
    Lit,
    /// The selection, the journey's start and cursor, the function set aside.
    Focus,
}

impl Dot {
    /// Its radius in the world, before the expanded view's zoom: small beside the
    /// room [`crate::nucleus`] leaves between two functions in the most crowded
    /// module.
    pub(crate) const fn radius(self) -> f32 {
        match self {
            Self::Speck => 0.3,
            Self::Lit => 0.5,
            Self::Focus => 0.7,
        }
    }

    /// The least and most its radius shows on screen, in points: never so small a
    /// lit one cannot be seen or clicked, never so big it covers its neighbours.
    const fn bounds(self) -> (f32, f32) {
        match self {
            Self::Speck => (1.5, 4.0),
            Self::Lit => (3.0, 7.0),
            Self::Focus => (4.0, 9.0),
        }
    }

    /// A radius on screen, in points, kept within its bounds.
    pub(crate) fn clamp(self, on_screen: f32) -> f32 {
        let (least, most) = self.bounds();
        on_screen.clamp(least, most)
    }
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

/// Which way a node relates to the start, and how many calls away: upstream wins a
/// tie, as for a function that both calls the start and is called by it.
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
    let color = module_shade(explorer.graph().cluster(tile).unwrap_or_default());
    // The journey's cursor is the one pink node, wherever it is.
    if explorer.cursor() == Some(tile) {
        return NodeLook::Cursor;
    }
    if explorer.nucleus().start() == Some(tile) {
        return NodeLook::Start;
    }
    let Some(reach) = explorer.reach() else {
        return NodeLook::Plain(color);
    };
    near(explorer, reach, tile).map_or(NodeLook::Faded(color), |hops| NodeLook::Near(color, hops))
}

/// How many calls from the start a node is, when it is near enough to light: with a
/// journey open, what the start calls lights as far as the journey is disclosed, and
/// only its direct callers light above it.
pub(crate) fn near(explorer: &Explorer, reach: &Reach, tile: usize) -> Option<u32> {
    let disclosed = explorer.disclosure();
    let journey = disclosed.tiles();
    let opened = disclosed.opened();
    // Whatever the nucleus lights for what it is to the start lights.
    let named = explorer.nucleus().level(tile).is_some();
    match relation(reach, tile) {
        Some(Relation::Upstream(hops)) if !opened || hops == 1 => Some(hops),
        Some(Relation::Upstream(hops) | Relation::Downstream(hops)) if named => Some(hops),
        Some(Relation::Downstream(hops)) if !opened || journey.contains_key(&tile) => Some(hops),
        _ if journey.contains_key(&tile) => Some(2),
        _ => None,
    }
    .map(|hops| hops.clamp(1, FARTHEST_SHADE))
}

/// The color a node is filled with, at full strength: the cursor's, else white for
/// the start, else its module's. A journey bar is painted the same, so a bar and its
/// node read as one thing.
pub(crate) fn node_color(explorer: &Explorer, tile: usize) -> Color {
    if explorer.cursor() == Some(tile) {
        return CURSOR;
    }
    if explorer.nucleus().start() == Some(tile) {
        return Color::WHITE;
    }
    module_color(explorer.graph().cluster(tile).unwrap_or_default())
}

/// Keeps what reaches the start current.
pub(crate) fn track_attention(mut explorer: ResMut<Explorer>) {
    if explorer.bypass_change_detection().refresh_reach() {
        explorer.set_changed();
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
        // What the nucleus lights shows plainly though the start does not reach it;
        // anything else is a small, faint speck, however near the start it is.
        let named = nucleus.lit(tile.0);
        let quiet = nucleus.speck(tile.0);
        // Brought in from outside the journey, or a caller of the start folded into its
        // module's count: there, but quieter than the journey.
        let outside =
            (nucleus.outside(tile.0) || nucleus.folded(tile.0)) && explorer.aside() != Some(tile.0);
        let look = match look {
            NodeLook::Faded(color) if named => NodeLook::Plain(color),
            NodeLook::Near(color, _) if quiet => NodeLook::Faded(color),
            look => look,
        };
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
) {
    let Ok((camera, view)) = cameras.single() else {
        return;
    };
    let mut ring = |tile: usize, grow: f32, color: Color| {
        if let Some(at) = explorer.tile_point(tile) {
            let radius = drawn_radius(&explorer, tile, camera, view);
            gizmos.sphere(Isometry3d::from_translation(at), radius * grow, color);
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
