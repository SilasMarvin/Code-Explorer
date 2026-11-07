//! How functions look: each node's color and size, and the rings round the start (or
//! the selection), the function set aside, the cursor and what is hovered. A node's
//! fill says what it is to the selection or the journey, in the colors its calls and
//! names carry: white, violet, blue, amber darker the deeper, pink, and grey for the
//! rest; its module shows in its sphere alone. Its size says only what it is to the
//! view ([`Dot`]): a faint speck, lit, or the one in focus. Hovering rings the node,
//! and shows a speck as big as a lit one, but never repaints it. Each dot is a small
//! matte ball, shaded by the light; a lit one, and its rings, show over everything
//! unlit whatever lies nearer, so a journey reads from any side.

use std::collections::HashMap;

use bevy::{
    mesh::MeshVertexBufferLayoutRef,
    pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline},
    prelude::*,
    render::render_resource::{
        AsBindGroup, CompareFunction, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
};

use crate::{
    arrows::Spine,
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

/// What a function's dot is drawn with: a matte ball, lit or not ([`Over`]).
pub(crate) type DotMaterial = ExtendedMaterial<StandardMaterial, Over>;

/// Whether a dot shows over everything else in the scene, nearer or not: one lit is
/// never hidden by a plain dot or a sphere in front of it. Drawn after the rest
/// ([`OVER_SORT`]), it takes no notice of what lies nearer.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
#[bind_group_data(OverKey)]
pub(crate) struct Over {
    pub(crate) over: bool,
}

#[repr(C)]
#[derive(Eq, PartialEq, Hash, Copy, Clone)]
pub(crate) struct OverKey {
    over: bool,
}

impl From<&Over> for OverKey {
    fn from(material: &Over) -> Self {
        Self {
            over: material.over,
        }
    }
}

impl MaterialExtension for Over {
    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        if key.bind_group_data.over
            && let Some(depth) = descriptor.depth_stencil.as_mut()
        {
            depth.depth_compare = Some(CompareFunction::Always);
        }
        Ok(())
    }
}

/// How much nearer than it is a dot drawn over the rest counts, as the see-through
/// things are drawn back to front: past anything in the scene, so it comes last.
const OVER_SORT: f32 = 1.0e6;

/// Every look, in full and dimmed, drawn among the rest or over them: a dot drawn
/// over is drawn among the see-through things, after all of them.
pub(crate) fn node_materials(
    materials: &mut Assets<DotMaterial>,
) -> HashMap<(NodeLook, bool, bool), Handle<DotMaterial>> {
    let mut table = HashMap::new();
    for (look, material) in node_looks() {
        for over in [false, true] {
            let mut add = |base: StandardMaterial| {
                let base = if over {
                    StandardMaterial {
                        alpha_mode: AlphaMode::Blend,
                        depth_bias: OVER_SORT,
                        ..base
                    }
                } else {
                    base
                };
                materials.add(DotMaterial {
                    base,
                    extension: Over { over },
                })
            };
            table.insert((look, false, over), add(material.clone()));
            table.insert((look, true, over), add(dimmed(&material, DIM)));
        }
    }
    table
}

/// Whether a function's dot is drawn over everything else, whatever lies nearer: the
/// start or the selection, the cursor, the journey's calls and callers, and the
/// function set aside; not one nothing lights.
pub(crate) fn drawn_over(explorer: &Explorer, tile: usize) -> bool {
    explorer.aside() == Some(tile)
        || !matches!(node_look(explorer, tile), NodeLook::Plain | NodeLook::Faded)
}

/// `material` at `share` of its brightness: darker, and fainter where it is see-through.
fn dimmed(material: &StandardMaterial, share: f32) -> StandardMaterial {
    let base = LinearRgba::from(material.base_color);
    let alpha = if base.alpha < 1.0 {
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

/// How each look is drawn at full brightness: a matte ball shaded by the light, with
/// only enough light of its own to keep its color on its dark side, no highlight and
/// no glow.
fn node_looks() -> HashMap<NodeLook, StandardMaterial> {
    let matte = |color: Color| StandardMaterial {
        base_color: color,
        emissive: LinearRgba::from(color) * 0.3,
        perceptual_roughness: 1.0,
        reflectance: 0.0,
        ..default()
    };
    let grey = FURTHER.mix(&Color::BLACK, 0.35);
    let mut table = HashMap::new();
    table.insert(NodeLook::Start, matte(SELECTED));
    table.insert(NodeLook::Cursor, matte(CURSOR));
    for flow in [Flow::Into]
        .into_iter()
        .chain((1..=SHADES).map(Flow::journey))
    {
        table.insert(NodeLook::Role(flow), matte(flow.color()));
    }
    table.insert(NodeLook::Plain, matte(grey));
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

/// What a function is to the view: an expanded module's function that nothing
/// lights is drawn as small as a speck, smaller than what is lit.
pub(crate) fn dot(explorer: &Explorer, tile: usize) -> Dot {
    let nucleus = explorer.nucleus();
    let focus = [nucleus.start(), explorer.cursor(), explorer.aside()];
    let opened_plain =
        nucleus.opened_up(tile) && nucleus.level(tile).is_none() && !nucleus.outside(tile);
    if focus.contains(&Some(tile)) {
        Dot::Focus
    } else if nucleus.lit(tile) && !nucleus.folded(tile) && !opened_plain {
        Dot::Lit
    } else {
        Dot::Speck
    }
}

/// How big a function is drawn in the world, seen from `view` through `camera`: its
/// role's size, less where its module's functions sit close, kept within that role's
/// bounds on screen at the depth the camera looks at, nearer bigger and further back
/// smaller as perspective has it ([`Dot::shown`]).
pub(crate) fn drawn_radius(
    explorer: &Explorer,
    tile: usize,
    camera: &Camera,
    view: &GlobalTransform,
) -> f32 {
    // What the pointer rests on shows at least as big as a lit one, so it stands out
    // from its neighbours.
    let dot = match dot(explorer, tile) {
        Dot::Speck if explorer.hovered() == Some(tile) => Dot::Lit,
        dot => dot,
    };
    radius_as(explorer, tile, dot, camera, view)
}

/// How big a function would be drawn in the world in `dot`'s role, whatever it is.
pub(crate) fn radius_as(
    explorer: &Explorer,
    tile: usize,
    dot: Dot,
    camera: &Camera,
    view: &GlobalTransform,
) -> f32 {
    let world = dot.radius();
    let Some(at) = explorer.tile_point(tile) else {
        return world;
    };
    let shows = |at: Vec3| camera.world_to_viewport(view, at).ok();
    let per_unit = shows(at)
        .zip(shows(at + view.up() * world))
        .map(|(a, b)| a.distance(b) / world);
    match per_unit {
        Some(per_unit) if per_unit > f32::EPSILON => {
            dot.shown(world * per_unit, nearer(explorer, view, at)) / per_unit
        }
        _ => world,
    }
}

/// How many times nearer the camera seen through `view` the point `at` lies than
/// what the camera looks at: above 1 nearer, below 1 further back.
pub(crate) fn nearer(explorer: &Explorer, view: &GlobalTransform, at: Vec3) -> f32 {
    let depth = (at - view.translation()).dot(*view.forward());
    explorer
        .view_depth()
        .filter(|_| depth > f32::EPSILON)
        .map_or(1.0, |middle| middle / depth)
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
    mut nodes: Query<(&Tile, &mut MeshMaterial3d<DotMaterial>, &mut Visibility)>,
) {
    if !explorer.is_changed() {
        return;
    }
    let nucleus = explorer.nucleus();
    let spine = Spine::of(&explorer);
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
        let key = (
            look,
            quiet(&explorer, &spine, tile.0),
            drawn_over(&explorer, tile.0),
        );
        if let Some(wanted) = materials.0.get(&key)
            && &material.0 != wanted
        {
            material.0 = wanted.clone();
        }
    }
}

/// Whether a function's dot is drawn dimmer: there, but quieter than the journey.
/// Brought in from outside the journey, a caller of the start folded into its
/// module's count, a call only guessed, an expanded module's function nothing
/// lights, or a call off the cursor's path a journey two levels deep makes ([`Spine`]);
/// never the function set aside.
pub(crate) fn quiet(explorer: &Explorer, spine: &Spine, tile: usize) -> bool {
    let nucleus = explorer.nucleus();
    (nucleus.outside(tile)
        || nucleus.folded(tile)
        || guessed(explorer, tile)
        || plain_beside(explorer, tile)
        || spine.dims(nucleus, tile))
        && explorer.aside() != Some(tile)
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
pub(crate) struct NodeMaterials(pub(crate) HashMap<(NodeLook, bool, bool), Handle<DotMaterial>>);

pub(crate) fn create_node_materials(
    mut commands: Commands,
    mut materials: ResMut<Assets<DotMaterial>>,
) {
    commands.insert_resource(NodeMaterials(node_materials(&mut materials)));
}

/// The orbit camera, apart from the functions.
type TheCamera = (With<OrbitCamera>, Without<Tile>);

/// Sizes every function drawn as [`drawn_radius`] has it, whenever the view or the
/// camera changes, and puts it where the map, spread as `Shift+-` and `Shift+=` have it, has it.
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
        if let Some(at) = explorer.tile_point(tile.0)
            && transform.translation != at
        {
            transform.translation = at;
        }
        // The mesh is one unit across.
        let size = 2.0 * drawn_radius(&explorer, tile.0, &camera, &view);
        if (transform.scale.x - size).abs() > size * 1e-3 {
            transform.scale = Vec3::splat(size);
        }
    }
}

/// A thin ring facing the camera, one per function, nothing else: white round
/// the start (or the selection), and in a journey round the function set aside; pink
/// round the journey's cursor, whose fill is pink too; violet round a call that also
/// calls the start back, so it reads as both; grey round the function hovered, in the
/// graph or as a bar, so pointing never looks like selecting, outside the ring it
/// has.
pub(crate) fn draw_graph(
    explorer: Res<Explorer>,
    cameras: Query<(&Camera, &GlobalTransform), With<OrbitCamera>>,
    mut gizmos: Gizmos<Rings>,
    mut marks: ResMut<Marks>,
) {
    marks.rings.clear();
    let Ok((camera, view)) = cameras.single() else {
        return;
    };
    let facing = view.rotation();
    let mut ring = |tile: usize, color: Color, out: f32| {
        if let Some(at) = explorer.tile_point(tile) {
            let radius = drawn_radius(&explorer, tile, camera, view) * out;
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
        ring(tile, SELECTED, RING);
    }
    let cursor = explorer.cursor();
    if let Some(tile) = cursor {
        ring(tile, CURSOR, RING);
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
        ring(tile, CALLER, RING);
    }
    if let Some(tile) = explorer.hovered() {
        let out = if ringed(tile) || both.contains(&tile) {
            HOVER_RING
        } else {
            RING
        };
        ring(tile, HOVER, out);
    }
}

/// The rings round dots: drawn over everything in the scene, as the lit dots they
/// ring are.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub(crate) struct Rings;

/// Sets the rings to show over the rest, at the width the other lines are drawn.
pub(crate) fn set_up_rings(mut gizmos: ResMut<GizmoConfigStore>) {
    let (config, _) = gizmos.config_mut::<Rings>();
    config.depth_bias = -1.0;
    config.line.width = 2.0;
}

/// How far out a ring sits, against its function's radius.
pub(crate) const RING: f32 = 1.6;
/// How far out the ring round the function hovered sits where it has a ring already:
/// outside that one.
pub(crate) const HOVER_RING: f32 = 2.2;
