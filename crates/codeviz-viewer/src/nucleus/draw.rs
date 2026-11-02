//! The nucleus in the scene: a glass sphere per module with one rim, an arrow per
//! pair of modules a named call runs between, and the named calls themselves, drawn
//! as the nucleus has them, eased.

use std::collections::HashMap;

use bevy::prelude::*;

use super::{Body, Flow, Link, Nucleus, SHADES, route::DETOUR_BENDS};
use crate::{
    camera::{OrbitCamera, frame_time},
    colors::{CURSOR, MODULE_SHADES, module_color, module_shade},
    explorer::Explorer,
    graph_view::drawn_radius,
};

/// Eases the spheres and the functions to where the nucleus now has them.
pub(crate) fn follow(time: Res<Time>, mut explorer: ResMut<Explorer>) {
    explorer
        .bypass_change_detection()
        .ease_nucleus(frame_time(&time));
}

/// A module's sphere in the scene.
#[derive(Component, Debug)]
pub(crate) struct ModuleSphere(usize);

/// One of the arrows between modules in the scene: a stretch of its shaft, between
/// two of its bends, or its head.
#[derive(Component, Debug)]
pub(crate) struct Arrow {
    index: usize,
    /// Which stretch of the shaft, from where it leaves.
    part: usize,
    head: bool,
}

/// What spheres and arrows are drawn with.
#[derive(Resource, Debug)]
pub(crate) struct Looks {
    sphere: Handle<Mesh>,
    shaft: Handle<Mesh>,
    head: Handle<Mesh>,
    /// By module color: glass for the nucleus, and for an open module.
    nucleus: Vec<Handle<StandardMaterial>>,
    open: Vec<Handle<StandardMaterial>>,
    /// Browsing, glass so faint the module only just reads as one, by color, for
    /// each of [`DEPTH_SHADES`] from the nearest to the furthest back.
    faint: Vec<Vec<Handle<StandardMaterial>>>,
    /// By [`Flow`]: an arrow's shaft and its head.
    shafts: HashMap<Flow, Handle<StandardMaterial>>,
    heads: HashMap<Flow, Handle<StandardMaterial>>,
    /// Browsing with nothing selected, an arrow between modules by how many calls it
    /// carries ([`module_strength`]): its shaft and its head, plainer the more.
    module_shafts: Vec<Handle<StandardMaterial>>,
    module_heads: Vec<Handle<StandardMaterial>>,
}

/// Browsing, how plain a module's glass is, from the nearest module to the furthest
/// back: further back reads fainter, so the map reads as having depth in a still.
const DEPTH_SHADES: [f32; 4] = [0.06, 0.038, 0.022, 0.01];

/// Browsing, how far back each module sits among them all, as `camera` sees them:
/// 0 for the nearest, 1 for the furthest back.
fn depths(nucleus: &Nucleus, camera: &Transform) -> Vec<f32> {
    let forward = *camera.forward();
    let along: Vec<f32> = (0..nucleus.now.balls.len())
        .map(|c| {
            nucleus
                .ball(c)
                .map_or(0.0, |b| (b.center - camera.translation).dot(forward))
        })
        .collect();
    let (near, far) = along
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &d| {
            (lo.min(d), hi.max(d))
        });
    let span = (far - near).max(1e-3);
    along
        .iter()
        .map(|d| ((d - near) / span).clamp(0.0, 1.0))
        .collect()
}

/// How plain an arrow between modules is drawn, browsing with nothing selected, for
/// each [`module_strength`]: its shaft's and its head's opacity.
const MODULE_STRENGTHS: [(f32, f32); 4] = [(0.03, 0.06), (0.06, 0.12), (0.11, 0.22), (0.2, 0.38)];

/// Which of [`MODULE_STRENGTHS`] an arrow carrying `calls` calls is drawn with: one
/// call, a few, up to ten, more.
pub(super) fn module_strength(calls: usize) -> usize {
    match calls {
        0 | 1 => 0,
        2..=3 => 1,
        4..=9 => 2,
        _ => 3,
    }
}

pub(crate) fn create_looks(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let glass = |color: Color, alpha: f32| StandardMaterial {
        base_color: color.with_alpha(alpha),
        perceptual_roughness: 0.25,
        reflectance: 0.6,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        double_sided: true,
        ..default()
    };
    let see_through = |color: Color, alpha: f32| StandardMaterial {
        base_color: color.with_alpha(alpha),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    };
    let colors: Vec<Color> = (0..MODULE_SHADES).map(module_color).collect();
    // A call between modules browsing has its own looks, by how many calls it carries.
    let flows: Vec<Flow> = [Flow::Into, Flow::Outside]
        .into_iter()
        .chain((1..=SHADES).map(Flow::Journey))
        .collect();
    let looks = Looks {
        sphere: meshes.add(Sphere::new(1.0).mesh().uv(64, 40)),
        shaft: meshes.add(Cylinder::new(1.0, 1.0).mesh().resolution(24)),
        head: meshes.add(Cone::new(1.0, 1.0).mesh().resolution(24)),
        nucleus: colors
            .iter()
            .map(|&c| materials.add(glass(c, 0.05)))
            .collect(),
        open: colors
            .iter()
            .map(|&c| materials.add(glass(c, 0.08)))
            .collect(),
        faint: DEPTH_SHADES
            .iter()
            .map(|&alpha| {
                colors
                    .iter()
                    .map(|&c| materials.add(glass(c, alpha)))
                    .collect()
            })
            .collect(),
        shafts: flows
            .iter()
            .map(|&f| {
                let alpha = if f.rank() > 1 { 0.2 } else { 0.32 };
                (f, materials.add(see_through(f.color(), alpha)))
            })
            .collect(),
        heads: flows
            .iter()
            .map(|&f| {
                let alpha = if f.rank() > 1 { 0.55 } else { 0.8 };
                (f, materials.add(see_through(f.color(), alpha)))
            })
            .collect(),
        module_shafts: MODULE_STRENGTHS
            .iter()
            .map(|&(shaft, _)| materials.add(see_through(Flow::Module.color(), shaft)))
            .collect(),
        module_heads: MODULE_STRENGTHS
            .iter()
            .map(|&(_, head)| materials.add(see_through(Flow::Module.color(), head)))
            .collect(),
    };
    commands.insert_resource(looks);
}

/// The orbit camera, apart from the spheres and arrows placed.
type OnlyTheCamera = (With<OrbitCamera>, Without<ModuleSphere>, Without<Arrow>);

/// Spawns a sphere per module, and an arrow per pair of modules as many as are
/// needed, then keeps each where the nucleus has it, drawn as its body asks.
pub(crate) fn place_bodies(
    mut commands: Commands,
    explorer: Res<Explorer>,
    looks: Res<Looks>,
    mut spheres: Query<
        (
            &ModuleSphere,
            &mut Transform,
            &mut Visibility,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        Without<Arrow>,
    >,
    mut arrows: Query<
        (
            &Arrow,
            &mut Transform,
            &mut Visibility,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        Without<ModuleSphere>,
    >,
    cameras: Query<&Transform, OnlyTheCamera>,
) {
    let nucleus = explorer.nucleus();
    let clusters = explorer.graph().cluster_count();
    if spheres.is_empty() {
        for cluster in 0..clusters {
            commands.spawn((
                ModuleSphere(cluster),
                Mesh3d(looks.sphere.clone()),
                MeshMaterial3d(looks.faint[0][module_shade(cluster)].clone()),
                Transform::default(),
                Visibility::Hidden,
                Pickable::IGNORE,
            ));
        }
    }
    let depths = cameras
        .single()
        .ok()
        .filter(|_| nucleus.browsing())
        .map(|camera| depths(nucleus, camera));
    for (sphere, mut transform, mut visibility, mut material) in &mut spheres {
        if let Some(ball) = nucleus.ball(sphere.0) {
            let shown = nucleus.shows_body(ball.body);
            let placed =
                Transform::from_translation(ball.center).with_scale(Vec3::splat(ball.radius));
            transform.set_if_neq(placed);
            let depth = depths
                .as_ref()
                .and_then(|d| d.get(sphere.0).copied())
                .unwrap_or(0.0);
            dress(&looks, sphere.0, ball.body, depth, &mut material);
            // A journey shows the modules it runs through, and nothing else.
            visibility.set_if_neq(if shown {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
        }
    }

    place_arrows(&mut commands, nucleus, &looks, &mut arrows);
}

/// Spawns as many arrows as the nucleus has between modules, each a shaft in
/// stretches round what lies in its way and a head, and keeps each where its path
/// runs, colored as its flow; hides those left over.
fn place_arrows(
    commands: &mut Commands,
    nucleus: &Nucleus,
    looks: &Looks,
    arrows: &mut Query<
        (
            &Arrow,
            &mut Transform,
            &mut Visibility,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        Without<ModuleSphere>,
    >,
) {
    let tubes = nucleus.tubes();
    let have = arrows
        .iter()
        .filter(|(a, ..)| !a.head && a.part == 0)
        .count();
    for index in have..tubes.len() {
        for (part, head) in (0..=DETOUR_BENDS)
            .map(|part| (part, false))
            .chain([(DETOUR_BENDS, true)])
        {
            commands.spawn((
                Arrow { index, part, head },
                Mesh3d(if head {
                    looks.head.clone()
                } else {
                    looks.shaft.clone()
                }),
                MeshMaterial3d(looks.shafts[&Flow::Into].clone()),
                Transform::default(),
                Visibility::Hidden,
                Pickable::IGNORE,
            ));
        }
    }
    let paths: Vec<Option<(Vec<Vec3>, f32)>> = tubes.iter().map(|t| nucleus.tube_path(t)).collect();
    let zoom = nucleus.zoom();
    for (arrow, mut transform, mut visibility, mut material) in arrows.iter_mut() {
        let Some((tube, (path, radius))) = tubes
            .get(arrow.index)
            .zip(paths.get(arrow.index).and_then(Option::as_ref))
        else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        let last = path.len().saturating_sub(2);
        let part = if arrow.head { last } else { arrow.part };
        let (Some(&start), Some(&end)) = (path.get(part), path.get(part + 1)) else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        let along = end - start;
        let length = along.length();
        let total: f32 = path.windows(2).map(|p| p[0].distance(p[1])).sum();
        if length < 1e-3 || total < 0.5 {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        }
        let direction = along / length;
        // Small and narrow, so the calls running in beside it stay in view.
        let head = (radius * 6.0).max(0.9 * zoom).min(length * 0.4);
        let width = (radius * 3.5).max(0.35 * zoom);
        let rotation = Quat::from_rotation_arc(Vec3::Y, direction);
        // The last stretch stops where the head begins.
        let shaft = if part == last { length - head } else { length };
        let placed = if arrow.head {
            Transform {
                translation: end - direction * head / 2.0,
                rotation,
                scale: Vec3::new(width, head, width),
            }
        } else {
            Transform {
                translation: start + direction * shaft / 2.0,
                rotation,
                scale: Vec3::new(*radius, shaft, *radius),
            }
        };
        transform.set_if_neq(placed);
        let strength = module_strength(tube.calls);
        let wanted = match (tube.flow, arrow.head) {
            (Flow::Module, true) => &looks.module_heads[strength],
            (Flow::Module, false) => &looks.module_shafts[strength],
            (flow, true) => &looks.heads[&flow],
            (flow, false) => &looks.shafts[&flow],
        };
        if &material.0 != wanted {
            material.0 = wanted.clone();
        }
        visibility.set_if_neq(Visibility::Inherited);
    }
}

/// Gives a module's sphere the look its body asks for, `depth` back among them all
/// (0 nearest, 1 furthest).
fn dress(
    looks: &Looks,
    cluster: usize,
    body: Body,
    depth: f32,
    material: &mut MeshMaterial3d<StandardMaterial>,
) {
    let color = module_shade(cluster);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "a share of a short list"
    )]
    let shade = ((depth * DEPTH_SHADES.len() as f32) as usize).min(DEPTH_SHADES.len() - 1);
    let wanted = match body {
        Body::Nucleus => &looks.nucleus[color],
        Body::Open => &looks.open[color],
        // A closed sphere is hidden.
        Body::Faint | Body::Closed => &looks.faint[shade][color],
    };
    if &material.0 != wanted {
        material.0 = wanted.clone();
    }
}

/// A rim round each glass sphere, facing the camera, so its edge reads as one
/// outline; and the named calls, function to function along their arrows.
pub(crate) fn draw(
    explorer: Res<Explorer>,
    cameras: Query<(&Camera, &GlobalTransform, &Transform), With<OrbitCamera>>,
    mut gizmos: Gizmos,
) {
    let nucleus = explorer.nucleus();
    let Ok((lens, view, camera)) = cameras.single() else {
        return;
    };
    draw_rims(nucleus, camera, &mut gizmos);
    // Short of a function's surface, so the head stays visible.
    let short = |from: Vec3, to: Vec3, tile: usize| {
        if !nucleus.placed(tile) {
            return to;
        }
        to - (to - from).normalize_or_zero() * drawn_radius(&explorer, tile, lens, view)
    };
    // The journey's call into the cursor, in the cursor's color, or into the function
    // the pointer is on, and the calls in of a function selected off the journey, or
    // of an outside caller or a folded caller of the start the pointer is on, are
    // drawn last and brightest, so they read over the rest.
    let aside = explorer.aside();
    let picked = |link: &Link| {
        let into = link.flow != Flow::Into
            && link.flow != Flow::Outside
            && (nucleus.is_cursor(link.to) || nucleus.hovered() == Some(link.to));
        let pointed = (link.flow == Flow::Outside || nucleus.folded(link.from))
            && nucleus.hovered() == Some(link.from);
        into || pointed || (aside.is_some() && Some(link.from) == aside)
    };
    let (picked, rest): (Vec<_>, Vec<_>) = nucleus
        .links()
        .iter()
        .enumerate()
        .partition(|(_, link)| picked(link));
    for (index, link) in rest.into_iter().chain(picked.iter().copied()) {
        let route = nucleus.route(link);
        let [.., last_but_one, last] = route.as_slice() else {
            continue;
        };
        let bright = picked.iter().any(|(i, _)| *i == index);
        let color = link_color(nucleus, link, bright);
        let end = short(*last_but_one, *last, link.to);
        gizmos.linestrip(route[..route.len() - 1].iter().copied(), color);
        // Its head as big beside the function it ends at, however near the view.
        let head = drawn_radius(&explorer, link.to, lens, view) * if bright { 3.0 } else { 2.2 };
        gizmos
            .arrow(*last_but_one, end, color)
            .with_tip_length(head);
    }
}

/// A rim round each sphere drawn, facing the camera; browsing, fainter the further
/// back.
fn draw_rims(nucleus: &Nucleus, camera: &Transform, gizmos: &mut Gizmos) {
    let facing = camera.rotation;
    let depths = nucleus.browsing().then(|| depths(nucleus, camera));
    for (cluster, ball) in nucleus.balls() {
        let near = depths
            .as_ref()
            .and_then(|d| d.get(cluster))
            .map_or(1.0, |depth| 1.0 - 0.8 * depth);
        let color = module_color(cluster);
        // One rim, so the glass's edge reads as one outline.
        let alpha = match ball.body {
            Body::Nucleus => 0.85,
            Body::Open => 0.7,
            Body::Faint => 0.2,
            Body::Closed => continue,
        };
        gizmos
            .circle(
                Isometry3d::new(ball.center, facing),
                ball.radius,
                color.with_alpha(alpha * near),
            )
            .resolution(96);
    }
}

/// A named call's color: its flow's, but the cursor's for the call into the cursor,
/// and brighter for the one into the function the pointer is on.
fn link_color(nucleus: &Nucleus, link: &Link, bright: bool) -> Color {
    match link.flow {
        Flow::Outside if bright => Color::WHITE.with_alpha(0.85),
        // Secondary: there to be found, not read first.
        Flow::Outside => link.flow.color().with_alpha(0.08),
        // A caller of the start folded into its module's count: there, but quiet.
        Flow::Into if !bright && nucleus.folded(link.from) => link.flow.color().with_alpha(0.16),
        _ if bright && nucleus.is_cursor(link.to) => CURSOR,
        _ if bright => link.flow.color().mix(&Color::WHITE, 0.45),
        _ => link.flow.color().with_alpha(0.9),
    }
}
