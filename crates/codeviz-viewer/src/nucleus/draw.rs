//! The nucleus in the scene: a sphere per module, where the nucleus has it: a lit, matte, see-through ball in the module's hue, shaded darker towards its
//! silhouette so it reads as round, with one faint rim on that silhouette; nothing
//! shines off it and nothing glows. Where spheres stack, the brightest shows and
//! they do not add up ([`Lighten`]), so a crowded map is no brighter than one ball
//! and the lines and names over it keep their contrast. The light rides with the
//! camera, so every side looks alike however the map turns, and what lies further
//! back shows a little dimmer ([`depth_fog`]). The calls are [`crate::arrows`]'s.

use bevy::{
    mesh::MeshVertexBufferLayoutRef,
    pbr::{
        DistanceFog, ExtendedMaterial, FogFalloff, MaterialExtension, MaterialExtensionKey,
        MaterialExtensionPipeline,
    },
    prelude::*,
    render::render_resource::{
        AsBindGroup, BlendComponent, BlendFactor, BlendOperation, BlendState,
        RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
};

use super::{Body, Nucleus};
use crate::{
    camera::OrbitCamera,
    colors::{MODULE_SHADES, module_color, module_shade},
    explorer::Explorer,
};

/// A module's sphere in the scene.
#[derive(Component, Debug)]
pub(crate) struct ModuleSphere(usize);

/// What a sphere is drawn with: a matte ball, lightening what lies behind it.
pub(crate) type SphereMaterial = ExtendedMaterial<StandardMaterial, Lighten>;

/// Spheres lighten what they cover: each pixel keeps the brightest of the spheres
/// over it (and of what was drawn before them), so however many stack it shows as
/// one ball would. The ball's own shade is drawn as if over black, which the ground
/// all but is; dots inside, brighter than any sphere, show through unchanged.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub(crate) struct Lighten {}

impl MaterialExtension for Lighten {
    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Max ignores the factors; they must be One.
        let max = BlendComponent {
            src_factor: BlendFactor::One,
            dst_factor: BlendFactor::One,
            operation: BlendOperation::Max,
        };
        if let Some(fragment) = descriptor.fragment.as_mut() {
            for target in fragment.targets.iter_mut().flatten() {
                target.blend = Some(BlendState {
                    color: max,
                    alpha: max,
                });
            }
        }
        Ok(())
    }
}

/// What spheres are drawn with.
#[derive(Resource, Debug)]
pub(crate) struct Looks {
    ball: Handle<Mesh>,
    /// By module color: the nucleus, an open module, and browsing, so faint the
    /// module only just reads as one; and fainter still behind a journey, so the
    /// map it passes through stays quiet and its lines and names stand out.
    nucleus: Vec<Handle<SphereMaterial>>,
    open: Vec<Handle<SphereMaterial>>,
    faint: Vec<Handle<SphereMaterial>>,
    behind: Vec<Handle<SphereMaterial>>,
}

/// How much dimmer, towards the ground, the furthest of the map shows than the
/// nearest: enough that depth reads in a still, not so much that the back fades.
const FURTHEST_DIM: f32 = 0.3;

pub(crate) fn create_looks(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SphereMaterial>>,
) {
    // Lit and matte: shaded by the light, with no highlight to shine off it. Drawn
    // as its shade times `alpha` (`Add` hands the blend that, and alpha none), then
    // lightening ([`Lighten`]).
    let matte = |color: Color, alpha: f32| SphereMaterial {
        base: StandardMaterial {
            base_color: color.with_alpha(alpha),
            perceptual_roughness: 1.0,
            reflectance: 0.0,
            alpha_mode: AlphaMode::Add,
            ..default()
        },
        extension: Lighten {},
    };
    let colors: Vec<Color> = (0..MODULE_SHADES).map(module_color).collect();
    let tinted = |alpha: f32, materials: &mut Assets<SphereMaterial>| {
        (colors.iter())
            .map(|&c| materials.add(matte(c, alpha)))
            .collect()
    };
    let looks = Looks {
        ball: meshes.add(Sphere::new(1.0).mesh().uv(64, 40)),
        nucleus: tinted(0.15, &mut materials),
        open: tinted(0.11, &mut materials),
        faint: tinted(0.075, &mut materials),
        behind: tinted(0.045, &mut materials),
    };
    commands.insert_resource(looks);
}

/// Spawns a sphere per module, then keeps each where the nucleus has it, drawn as
/// its body asks.
pub(crate) fn place_bodies(
    mut commands: Commands,
    explorer: Res<Explorer>,
    looks: Res<Looks>,
    mut spheres: Query<(
        &ModuleSphere,
        &mut Transform,
        &mut Visibility,
        &mut MeshMaterial3d<SphereMaterial>,
    )>,
) {
    let nucleus = explorer.nucleus();
    let behind = !nucleus.browsing();
    let clusters = explorer.graph().cluster_count();
    if spheres.is_empty() {
        for cluster in 0..clusters {
            commands.spawn((
                ModuleSphere(cluster),
                Mesh3d(looks.ball.clone()),
                MeshMaterial3d(looks.faint[module_shade(cluster)].clone()),
                Transform::default(),
                Visibility::Hidden,
            ));
        }
    }
    for (sphere, mut transform, mut visibility, mut material) in &mut spheres {
        if let Some(ball) = nucleus.ball(sphere.0) {
            let placed =
                Transform::from_translation(ball.center).with_scale(Vec3::splat(ball.radius));
            transform.set_if_neq(placed);
            let color = module_shade(sphere.0);
            let wanted = match ball.body {
                Body::Nucleus => &looks.nucleus[color],
                Body::Open => &looks.open[color],
                Body::Faint if behind => &looks.behind[color],
                Body::Faint => &looks.faint[color],
            };
            if &material.0 != wanted {
                material.0 = wanted.clone();
            }
            visibility.set_if_neq(Visibility::Inherited);
        }
    }
}

/// How far from `eye` the map shown begins and ends: the nearest and the furthest
/// of its spheres' surfaces.
fn depth_span(nucleus: &Nucleus, eye: Vec3) -> Option<(f32, f32)> {
    nucleus
        .balls()
        .map(|(_, ball)| {
            let away = ball.center.distance(eye);
            ((away - ball.radius).max(0.0), away + ball.radius)
        })
        .reduce(|(a, b), (c, d)| (a.min(c), b.max(d)))
}

/// How far towards the ground a point `away` from the camera is dimmed, the map
/// spanning `span`: none at its nearest, [`FURTHEST_DIM`] at its furthest.
fn dimmed_at((near, far): (f32, f32), away: f32) -> f32 {
    ((away - near) / (far - near).max(1e-3)).clamp(0.0, 1.0) * FURTHEST_DIM
}

/// Dims what lies further back, a little, towards the ground: the map's nearest
/// surface not at all, its furthest by [`FURTHEST_DIM`], wherever the camera stands.
/// Run once the camera has moved for the frame.
pub(crate) fn depth_fog(
    explorer: Res<Explorer>,
    mut cameras: Query<(&Transform, &mut DistanceFog), With<OrbitCamera>>,
) {
    let Ok((camera, mut fog)) = cameras.single_mut() else {
        return;
    };
    let Some((near, far)) = depth_span(explorer.nucleus(), camera.translation) else {
        return;
    };
    // Linear fog is whole at its end: put the end where the furthest is only
    // [`FURTHEST_DIM`] of the way there.
    fog.falloff = FogFalloff::Linear {
        start: near,
        end: near + (far - near).max(1e-3) / FURTHEST_DIM,
    };
}

/// A rim round each sphere where its surface turns away from the camera, so it
/// reads as one ball with an edge; dimmer the further back, as the sphere is.
pub(crate) fn draw(
    explorer: Res<Explorer>,
    cameras: Query<&Transform, With<OrbitCamera>>,
    mut gizmos: Gizmos,
) {
    let nucleus = explorer.nucleus();
    let Ok(camera) = cameras.single() else {
        return;
    };
    let eye = camera.translation;
    let Some(span) = depth_span(nucleus, eye) else {
        return;
    };
    let behind = !nucleus.browsing();
    for (cluster, ball) in nucleus.balls() {
        let alpha = match ball.body {
            Body::Nucleus => 0.6,
            Body::Open => 0.5,
            Body::Faint if behind => 0.1,
            Body::Faint => 0.15,
        };
        let Some((middle, radius, facing)) = silhouette(ball.center, ball.radius, eye) else {
            continue;
        };
        let near = 1.0 - dimmed_at(span, ball.center.distance(eye));
        gizmos
            .circle(
                Isometry3d::new(middle, facing),
                radius,
                module_color(cluster).with_alpha(alpha * near),
            )
            .resolution(96);
    }
}

/// The circle a sphere of `radius` round `center` shows its edge along, seen from
/// `eye`: its middle, its radius, and a turn whose Z faces the eye. None from inside.
fn silhouette(center: Vec3, radius: f32, eye: Vec3) -> Option<(Vec3, f32, Quat)> {
    let to_eye = eye - center;
    let away = to_eye.length();
    if away <= radius {
        return None;
    }
    let share = radius / away;
    let facing = Quat::from_rotation_arc(Vec3::Z, to_eye / away);
    Some((
        center + to_eye * share * share,
        radius * (1.0 - share * share).sqrt(),
        facing,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rim_lies_where_the_sphere_turns_away_from_the_eye() {
        let (center, radius, eye) = (Vec3::new(1.0, 2.0, 0.0), 3.0, Vec3::new(1.0, 2.0, 10.0));
        let (middle, rim, facing) = silhouette(center, radius, eye).expect("outside");
        // Every point of it is on the sphere, and the line to the eye grazes it there.
        for turn in [0.0_f32, 1.0, 2.5, 4.0] {
            let point = middle + facing * Vec3::new(turn.cos(), turn.sin(), 0.0) * rim;
            assert!((point.distance(center) - radius).abs() < 1e-4);
            assert!((point - center).dot(eye - point).abs() < 1e-3);
        }
        assert!(silhouette(center, radius, center).is_none());
    }

    #[test]
    fn the_back_of_the_map_shows_dimmer_than_its_front_by_a_little() {
        let span = (10.0, 30.0);
        assert!(dimmed_at(span, 10.0).abs() < 1e-6);
        assert!((dimmed_at(span, 30.0) - FURTHEST_DIM).abs() < 1e-6);
        assert!(dimmed_at(span, 20.0) > 0.0 && dimmed_at(span, 20.0) < FURTHEST_DIM);
    }
}
