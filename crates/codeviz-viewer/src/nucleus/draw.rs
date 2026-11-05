//! The nucleus in the scene: a sphere per module, drawn as a flat tinted disc facing
//! the camera with one rim on its edge, where the nucleus has it. The calls
//! are [`crate::arrows`]'s.

use bevy::prelude::*;

use super::{Body, Nucleus};
use crate::{
    camera::OrbitCamera,
    colors::{MODULE_SHADES, module_color, module_shade},
    explorer::Explorer,
};

/// A module's sphere in the scene: a disc facing the camera, as big as the sphere,
/// so it and its rim are one circle on one centre however near the view.
#[derive(Component, Debug)]
pub(crate) struct ModuleSphere(usize);

/// What spheres are drawn with.
#[derive(Resource, Debug)]
pub(crate) struct Looks {
    disc: Handle<Mesh>,
    /// By module color: the nucleus, and an open module.
    nucleus: Vec<Handle<StandardMaterial>>,
    open: Vec<Handle<StandardMaterial>>,
    /// Browsing, so faint the module only just reads as one, by color, for each of
    /// [`DEPTH_SHADES`] from the nearest to the furthest back.
    faint: Vec<Vec<Handle<StandardMaterial>>>,
}

/// Browsing, how strong a module's disc is, from the nearest module to the furthest
/// back: further back reads fainter, so the map reads as having depth in a still.
const DEPTH_SHADES: [f32; 4] = [0.1, 0.075, 0.055, 0.04];

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

pub(crate) fn create_looks(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Flat, lit by nothing: no highlight or glow, only the module's tint.
    let tint = |color: Color, alpha: f32| StandardMaterial {
        base_color: color.with_alpha(alpha),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        double_sided: true,
        ..default()
    };
    let colors: Vec<Color> = (0..MODULE_SHADES).map(module_color).collect();
    let looks = Looks {
        disc: meshes.add(Circle::new(1.0).mesh().resolution(96)),
        nucleus: colors
            .iter()
            .map(|&c| materials.add(tint(c, 0.1)))
            .collect(),
        open: colors
            .iter()
            .map(|&c| materials.add(tint(c, 0.12)))
            .collect(),
        faint: DEPTH_SHADES
            .iter()
            .map(|&alpha| {
                colors
                    .iter()
                    .map(|&c| materials.add(tint(c, alpha)))
                    .collect()
            })
            .collect(),
    };
    commands.insert_resource(looks);
}

/// The orbit camera, apart from the spheres placed.
type OnlyTheCamera = (With<OrbitCamera>, Without<ModuleSphere>);

/// Spawns a sphere per module, then keeps each where the nucleus has it, facing the
/// camera, drawn as its body asks.
pub(crate) fn place_bodies(
    mut commands: Commands,
    explorer: Res<Explorer>,
    looks: Res<Looks>,
    mut spheres: Query<(
        &ModuleSphere,
        &mut Transform,
        &mut Visibility,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
    cameras: Query<&Transform, OnlyTheCamera>,
) {
    let nucleus = explorer.nucleus();
    let clusters = explorer.graph().cluster_count();
    if spheres.is_empty() {
        for cluster in 0..clusters {
            commands.spawn((
                ModuleSphere(cluster),
                Mesh3d(looks.disc.clone()),
                MeshMaterial3d(looks.faint[0][module_shade(cluster)].clone()),
                Transform::default(),
                Visibility::Hidden,
            ));
        }
    }
    let camera = cameras.single().ok();
    let facing = camera.map_or(Quat::IDENTITY, |c| c.rotation);
    let depths = camera
        .filter(|_| nucleus.browsing())
        .map(|camera| depths(nucleus, camera));
    for (sphere, mut transform, mut visibility, mut material) in &mut spheres {
        if let Some(ball) = nucleus.ball(sphere.0) {
            let placed = Transform::from_translation(ball.center)
                .with_rotation(facing)
                .with_scale(Vec3::splat(ball.radius));
            transform.set_if_neq(placed);
            let depth = depths
                .as_ref()
                .and_then(|d| d.get(sphere.0).copied())
                .unwrap_or(0.0);
            dress(&looks, sphere.0, ball.body, depth, &mut material);
            visibility.set_if_neq(Visibility::Inherited);
        }
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
        Body::Faint => &looks.faint[shade][color],
    };
    if &material.0 != wanted {
        material.0 = wanted.clone();
    }
}

/// A rim round each sphere's disc, on the disc's own centre and edge, so the two read
/// as one outline; browsing, fainter the further back.
pub(crate) fn draw(
    explorer: Res<Explorer>,
    cameras: Query<&Transform, With<OrbitCamera>>,
    mut gizmos: Gizmos,
) {
    let nucleus = explorer.nucleus();
    let Ok(camera) = cameras.single() else {
        return;
    };
    let facing = camera.rotation;
    let depths = nucleus.browsing().then(|| depths(nucleus, camera));
    for (cluster, ball) in nucleus.balls() {
        let near = depths
            .as_ref()
            .and_then(|d| d.get(cluster))
            .map_or(1.0, |depth| 1.0 - 0.6 * depth);
        let alpha = match ball.body {
            Body::Nucleus => 0.85,
            Body::Open => 0.7,
            Body::Faint => 0.35,
        };
        gizmos
            .circle(
                Isometry3d::new(ball.center, facing),
                ball.radius,
                module_color(cluster).with_alpha(alpha * near),
            )
            .resolution(96);
    }
}
