//! Modules as spheres: the graph spreads through all three dimensions, each module's
//! functions gathering in a ball of their own, drawn as a softly shaded sphere fitted
//! to them. Space says where a function lives and nothing else. The modules that hold
//! the focus, what calls it and what it calls are lit and named; the rest recede to
//! faint outlines.

use std::collections::HashSet;

use bevy::prelude::*;

use crate::{
    camera::{ANGLED, OrbitCamera},
    explorer::Explorer,
    graph_view::{NodeLook, node_look},
    scene::{PALETTE, Tile, lane_color},
    ui::node_name,
};

/// Room between a sphere and the middles of its outermost functions, so they sit
/// inside it rather than on it.
const PAD: f32 = 1.4;
/// Steps the fit takes towards the smallest ball.
const FIT_STEPS: u32 = 48;
/// How far out of its middle a sphere the focus does not touch shows its functions.
const HUDDLE: f32 = 0.4;
/// How quickly a sphere huddles or opens out, per second.
const HUDDLE_SPEED: f32 = 6.0;
/// How tall a function's name is on screen, and how far right of it, in pixels.
const NAME_HEIGHT: f32 = 20.0;
const NAME_GAP: f32 = 10.0;
/// Angles either side of the default an opening view tries.
const VIEW_STEPS: i32 = 6;
/// How far Q and E swing the camera round.
const SWING: f32 = 0.9;

/// A module's sphere: where it is, how big, and whether it is lit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Shell {
    pub(crate) cluster: usize,
    pub(crate) center: Vec3,
    pub(crate) radius: f32,
    pub(crate) lit: bool,
    /// Whether it holds the focus.
    pub(crate) home: bool,
}

/// Every module's sphere, fitted to where its functions are now.
pub(crate) fn shells(explorer: &Explorer) -> Vec<Shell> {
    let graph = explorer.graph();
    let mut members: Vec<Vec<Vec3>> = vec![Vec::new(); graph.cluster_count()];
    for tile in 0..explorer.map().tiles.len() {
        if let (Some(cluster), Some(at)) = (graph.cluster(tile), explorer.tile_point(tile))
            && let Some(list) = members.get_mut(cluster)
        {
            list.push(at);
        }
    }
    let lit = lit_clusters(explorer);
    let home = explorer.graph_focus().and_then(|t| graph.cluster(t));
    members
        .iter()
        .enumerate()
        .filter(|(_, points)| !points.is_empty())
        .map(|(cluster, points)| {
            let (center, radius) = enclose(points);
            Shell {
                cluster,
                center,
                radius: radius + PAD,
                lit: lit.contains(&cluster),
                home: home == Some(cluster),
            }
        })
        .collect()
}

/// Draws the functions of each module the focus does not touch in towards its middle,
/// so those spheres shrink out of the way of the ones that matter, each still where
/// it was; the rest open out in full. Eases between, and moves the nodes to match.
pub(crate) fn huddle(
    time: Res<Time>,
    mut explorer: ResMut<Explorer>,
    mut last_moved: Local<u64>,
    mut nodes: Query<(&Tile, &mut Transform)>,
) {
    let graph = explorer.graph();
    let mut sums = vec![(Vec3::ZERO, 0.0_f32); graph.cluster_count()];
    for (tile, &at) in graph.positions.iter().enumerate() {
        if let Some(sum) = graph.cluster(tile).and_then(|c| sums.get_mut(c)) {
            sum.0 += Vec3::from(at);
            sum.1 += 1.0;
        }
    }
    let lit = lit_clusters(&explorer);
    let wanted = |cluster: usize| {
        if explorer.spheres() && explorer.graph_focus().is_some() && !lit.contains(&cluster) {
            HUDDLE
        } else {
            1.0
        }
    };
    let ease = 1.0 - (-HUDDLE_SPEED * time.delta_secs()).exp();
    let before = explorer.huddles();
    let huddles: Vec<(Vec3, f32)> = sums
        .iter()
        .enumerate()
        .map(|(cluster, &(sum, n))| {
            let goal = wanted(cluster);
            let share = before.get(cluster).map_or(goal, |&(_, share)| {
                let next = share + (goal - share) * ease;
                if (goal - next).abs() < 0.002 {
                    goal
                } else {
                    next
                }
            });
            (sum / n.max(1.0), share)
        })
        .collect();
    let easing = huddles
        .iter()
        .zip(before)
        .any(|(now, was)| (now.1 - was.1).abs() > f32::EPSILON)
        || huddles.len() != before.len();
    let moved = explorer.moved() != *last_moved;
    *last_moved = explorer.moved();
    explorer.bypass_change_detection().set_huddles(huddles);
    if !easing && !moved {
        return;
    }
    if easing {
        explorer.set_changed();
    }
    for (tile, mut transform) in &mut nodes {
        if let Some(at) = explorer.tile_point(tile.0) {
            transform.translation = at;
        }
    }
}

/// The modules holding the focus and the functions lit beside it.
fn lit_clusters(explorer: &Explorer) -> HashSet<usize> {
    let graph = explorer.graph();
    lit_tiles(explorer)
        .into_iter()
        .filter_map(|tile| graph.cluster(tile))
        .collect()
}

/// The focus and the functions lit beside it: what calls it, what it calls, what the
/// journey shows and what the spotlight picks out.
fn lit_tiles(explorer: &Explorer) -> HashSet<usize> {
    let Some(focus) = explorer.graph_focus() else {
        return HashSet::new();
    };
    let graph = explorer.graph();
    let disclosed = explorer.disclosure();
    let next_door = |tile: usize| {
        matches!(
            node_look(explorer, tile),
            NodeLook::Focus | NodeLook::Near(_, 1)
        )
    };
    std::iter::once(focus)
        .chain(graph.callers(focus))
        .chain(graph.callees(focus))
        .chain(disclosed.tiles().keys().copied())
        .chain(explorer.spotlit())
        .filter(|&tile| next_door(tile))
        .collect()
}

/// The angle that best keeps what is lit from hiding itself: of the angles round the
/// default three-quarter view, the one where the lit spheres overlap least, the lit
/// functions' names have room, and calls to and from the focus cross no other lit
/// sphere; the nearer the default the better.
/// `distance` is how far back the camera will be, which sets how much room a name
/// takes up in the world.
pub(crate) fn clear_view(explorer: &Explorer, distance: f32) -> (f32, f32) {
    let graph = explorer.graph();
    let lit = lit_tiles(explorer);
    // About what a pixel spans at that distance, in a typical window.
    let pixel = distance * 0.83 / 900.0;
    // How wide each lit function's name runs to its right, asides and pill included.
    #[expect(clippy::cast_precision_loss, reason = "a short name")]
    let width = |tile: usize| (node_name(explorer, tile).chars().count() as f32 + 12.0) * 7.0;
    let shells: Vec<Shell> = shells(explorer)
        .into_iter()
        .filter(|shell| shell.lit)
        .collect();
    let named: Vec<(usize, Vec3)> = lit
        .iter()
        .filter_map(|&tile| Some((tile, explorer.tile_point(tile)?)))
        .collect();
    let focus = explorer
        .graph_focus()
        .and_then(|tile| Some((tile, explorer.tile_point(tile)?)));
    let mut best = (f32::MAX, ANGLED);
    for yaw_step in -VIEW_STEPS..=VIEW_STEPS {
        for pitch in [-0.6, -0.35, -0.1, 0.15] {
            #[expect(clippy::cast_precision_loss, reason = "a small count")]
            let yaw = ANGLED.0 + yaw_step as f32 * 0.3;
            let rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
            let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
            let flat = |at: Vec3| Vec2::new(at.dot(right), at.dot(up));
            let mut crowding = 0.0;
            // Spheres over spheres.
            for (i, a) in shells.iter().enumerate() {
                for b in &shells[i + 1..] {
                    let apart = flat(a.center).distance(flat(b.center));
                    crowding += (a.radius + b.radius - apart).max(0.0);
                }
            }
            // Names over names, and over the functions they would hide.
            let boxes: Vec<Rect> = named
                .iter()
                .map(|&(tile, at)| {
                    let left = flat(at) + Vec2::new(NAME_GAP * pixel, -NAME_HEIGHT * pixel / 2.0);
                    Rect::from_corners(left, left + Vec2::new(width(tile), NAME_HEIGHT) * pixel)
                })
                .collect();
            for (i, a) in boxes.iter().enumerate() {
                for b in &boxes[i + 1..] {
                    let both = a.intersect(*b);
                    if !both.is_empty() {
                        crowding += 2.0 * both.width().min(both.height() * 4.0);
                    }
                }
                for (j, &(_, at)) in named.iter().enumerate() {
                    if i != j && a.contains(flat(at)) {
                        crowding += NAME_HEIGHT * pixel * 2.0;
                    }
                }
            }
            // A call to or from the focus across a sphere neither end is in.
            if let Some((from, start)) = focus {
                for &(to, end) in &named {
                    let ends = [graph.cluster(from), graph.cluster(to)];
                    for shell in &shells {
                        if ends.contains(&Some(shell.cluster)) {
                            continue;
                        }
                        let apart = to_segment(flat(shell.center), flat(start), flat(end));
                        crowding += (shell.radius - apart).max(0.0);
                    }
                }
            }
            let turn = (yaw - ANGLED.0).abs() + (pitch - ANGLED.1).abs();
            let score = crowding + turn * 4.0;
            if score < best.0 {
                best = (score, (yaw, pitch));
            }
        }
    }
    best.1
}

/// How far `point` is from the segment `a`–`b`.
fn to_segment(point: Vec2, a: Vec2, b: Vec2) -> f32 {
    let along = b - a;
    let t = ((point - a).dot(along) / along.length_squared().max(1e-6)).clamp(0.0, 1.0);
    point.distance(a + along * t)
}

/// Near enough the smallest ball round `points`: from their middle, step towards
/// whichever is farthest, a shorter step each time, keeping the tightest ball seen.
fn enclose(points: &[Vec3]) -> (Vec3, f32) {
    let farthest = |center: Vec3| {
        points
            .iter()
            .map(|&p| (p, p.distance(center)))
            .fold(
                (center, 0.0),
                |best, next| if next.1 > best.1 { next } else { best },
            )
    };
    #[expect(clippy::cast_precision_loss, reason = "a count of functions")]
    let mut center = points.iter().copied().sum::<Vec3>() / points.len() as f32;
    let mut best = (center, farthest(center).1);
    for step in 1..=FIT_STEPS {
        let (far, radius) = farthest(center);
        if radius < best.1 {
            best = (center, radius);
        }
        #[expect(clippy::cast_precision_loss, reason = "a small count")]
        let share = 1.0 / (step as f32 + 1.0);
        center = center.lerp(far, share);
    }
    let (_, radius) = farthest(center);
    if radius < best.1 {
        (center, radius)
    } else {
        best
    }
}

/// One sphere mesh per module, drawn only while it is lit.
#[derive(Component, Debug)]
pub(crate) struct ShellMesh(usize);

/// Per module color: the sphere round the focus's module, and round the others lit.
#[derive(Resource, Debug)]
pub(crate) struct ShellMaterials(Vec<(Handle<StandardMaterial>, Handle<StandardMaterial>)>);

pub(crate) fn spawn_shells(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    explorer: Res<Explorer>,
) {
    let mesh = meshes.add(Sphere::new(1.0).mesh().uv(48, 32));
    // Premultiplied, so the light's highlight on the glass shows at full strength
    // however faint the tint: it is what makes a disc read as a ball.
    let shell = |color: Color, alpha: f32| {
        let linear = LinearRgba::from(color);
        StandardMaterial {
            base_color: Color::from(LinearRgba::new(
                linear.red * alpha,
                linear.green * alpha,
                linear.blue * alpha,
                alpha,
            )),
            emissive: linear * 0.03,
            perceptual_roughness: 0.55,
            reflectance: 0.35,
            alpha_mode: AlphaMode::Premultiplied,
            ..default()
        }
    };
    let table: Vec<_> = (0..PALETTE.len())
        .map(|color| {
            let color = lane_color(color);
            (
                materials.add(shell(color, 0.2)),
                materials.add(shell(color, 0.13)),
            )
        })
        .collect();
    for cluster in 0..explorer.graph().cluster_count() {
        commands.spawn((
            ShellMesh(cluster),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(table[cluster % PALETTE.len()].1.clone()),
            Transform::default(),
            Visibility::Hidden,
            // Clicks go through to the functions inside.
            Pickable::IGNORE,
        ));
    }
    commands.insert_resource(ShellMaterials(table));
}

/// Fits each lit module's sphere to its functions, and hides the rest.
pub(crate) fn place_shells(
    explorer: Res<Explorer>,
    materials: Res<ShellMaterials>,
    mut meshes: Query<(
        &ShellMesh,
        &mut Transform,
        &mut Visibility,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
) {
    let shells = if explorer.spheres() {
        shells(&explorer)
    } else {
        Vec::new()
    };
    for (mesh, mut transform, mut visibility, mut material) in &mut meshes {
        let Some(shell) = shells.iter().find(|s| s.cluster == mesh.0 && s.lit) else {
            visibility.set_if_neq(Visibility::Hidden);
            continue;
        };
        visibility.set_if_neq(Visibility::Visible);
        let wanted =
            Transform::from_translation(shell.center).with_scale(Vec3::splat(shell.radius));
        transform.set_if_neq(wanted);
        let (home, lit) = &materials.0[shell.cluster % PALETTE.len()];
        let wanted = if shell.home { home } else { lit };
        if &material.0 != wanted {
            material.0 = wanted.clone();
        }
    }
}

/// A soft rim round each lit sphere's outline, so it reads as one ball, and a faint
/// outline round each of the rest.
pub(crate) fn draw_rims(
    explorer: Res<Explorer>,
    cameras: Query<&GlobalTransform, With<OrbitCamera>>,
    mut gizmos: Gizmos,
) {
    if !explorer.spheres() {
        return;
    }
    let Ok(camera) = cameras.single() else {
        return;
    };
    let eye = camera.translation();
    let focused = explorer.graph_focus().is_some();
    let shells = shells(&explorer);
    let veiled = veiled(&shells, eye);
    for shell in &shells {
        if veiled.contains(&shell.cluster) {
            continue;
        }
        let Some((middle, radius, facing)) = outline(shell.center, shell.radius, eye) else {
            continue;
        };
        let color = lane_color(shell.cluster % PALETTE.len());
        let rings: &[(f32, f32)] = if shell.lit {
            &[(1.0, 0.6), (0.985, 0.25), (0.965, 0.1)]
        } else if focused {
            &[(1.0, 0.09)]
        } else {
            &[(1.0, 0.3)]
        };
        let isometry = Isometry3d::new(middle, Quat::from_rotation_arc(Vec3::Z, facing));
        for &(share, alpha) in rings {
            gizmos
                .circle(isometry, radius * share, color.with_alpha(alpha))
                .resolution(64);
        }
    }
}

/// The modules the focus does not touch that are seen over or through a lit sphere:
/// drawn, they would read as part of it.
fn veiled(shells: &[Shell], eye: Vec3) -> HashSet<usize> {
    let lit: Vec<&Shell> = shells.iter().filter(|shell| shell.lit).collect();
    shells
        .iter()
        .filter(|shell| !shell.lit && lit.iter().any(|other| overlaps(shell, other, eye)))
        .map(|shell| shell.cluster)
        .collect()
}

/// Hides the faded functions of every veiled module, so no stray points seem to sit
/// in a lit sphere; anything lit in them still shows.
pub(crate) fn veil(
    explorer: Res<Explorer>,
    cameras: Query<&GlobalTransform, With<OrbitCamera>>,
    mut nodes: Query<(&Tile, &mut Visibility)>,
) {
    let veiled = match cameras.single() {
        Ok(camera) if explorer.spheres() => veiled(&shells(&explorer), camera.translation()),
        _ => HashSet::new(),
    };
    let graph = explorer.graph();
    for (tile, mut visibility) in &mut nodes {
        let hidden = graph.cluster(tile.0).is_some_and(|c| veiled.contains(&c))
            && matches!(node_look(&explorer, tile.0), NodeLook::Faded(_));
        visibility.set_if_neq(if hidden {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        });
    }
}

/// Where a sphere's outline is, seen from `eye`: the middle of the circle where sight
/// grazes it, its radius, and which way it faces.
fn outline(center: Vec3, radius: f32, eye: Vec3) -> Option<(Vec3, f32, Vec3)> {
    let toward = eye - center;
    let distance = toward.length();
    if distance <= radius {
        return None;
    }
    let facing = toward / distance;
    let middle = center + facing * (radius * radius / distance);
    let ring = radius * (1.0 - (radius / distance).powi(2)).sqrt();
    Some((middle, ring, facing))
}

/// Whether two spheres seen from `eye` overlap on screen, one in front of the other.
fn overlaps(a: &Shell, b: &Shell, eye: Vec3) -> bool {
    let (to_a, to_b) = (a.center - eye, b.center - eye);
    let half_angle = |shell: &Shell, to: Vec3| (shell.radius / to.length()).min(1.0).asin();
    to_a.angle_between(to_b) < half_angle(a, to_a) + half_angle(b, to_b)
}

/// Z turns the spheres on and off; Q and E swing the camera round the focus.
pub(crate) fn sphere_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut explorer: ResMut<Explorer>,
    mut cameras: Query<&mut OrbitCamera>,
) {
    if keys.just_pressed(KeyCode::KeyZ) {
        let on = !explorer.spheres();
        explorer.set_spheres(on);
    }
    let swing = if keys.just_pressed(KeyCode::KeyQ) {
        -SWING
    } else if keys.just_pressed(KeyCode::KeyE) {
        SWING
    } else {
        return;
    };
    for mut camera in &mut cameras {
        camera.swing(swing);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fit_hugs_what_it_holds() {
        // A crowd on one side does not drag the ball off the far point.
        let mut points = vec![Vec3::new(-10.0, 0.0, 0.0), Vec3::new(10.0, 0.0, 0.0)];
        points.extend((0..20).map(|i| Vec3::new(9.0, 0.1 * i as f32, 0.0)));
        let (center, radius) = enclose(&points);
        assert!(radius < 10.6, "tight: {radius}");
        assert!(points.iter().all(|p| p.distance(center) <= radius + 1e-3));
        let corners: Vec<Vec3> = (0..8)
            .map(|i| Vec3::new((i & 1) as f32, (i >> 1 & 1) as f32, (i >> 2) as f32))
            .collect();
        let (_, radius) = enclose(&corners);
        assert!((to_segment(Vec2::new(1.0, 1.0), Vec2::ZERO, Vec2::X * 2.0) - 1.0).abs() < 1e-5);
        assert!((radius - 3.0_f32.sqrt() / 2.0).abs() < 0.05, "{radius}");
    }

    #[test]
    fn the_outline_is_where_sight_grazes_the_sphere() {
        let (middle, ring, facing) = outline(Vec3::ZERO, 3.0, Vec3::new(0.0, 0.0, 5.0)).unwrap();
        assert_eq!(facing, Vec3::Z);
        // A 3-4-5 triangle: the graze point is 4 from the eye.
        assert!((middle.z - 1.8).abs() < 1e-4 && (ring - 2.4).abs() < 1e-4);
        assert!(outline(Vec3::ZERO, 3.0, Vec3::new(0.0, 0.0, 2.0)).is_none());
        let shell = |x: f32, z: f32| Shell {
            cluster: 0,
            center: Vec3::new(x, 0.0, z),
            radius: 1.0,
            lit: false,
            home: false,
        };
        let eye = Vec3::new(0.0, 0.0, 20.0);
        assert!(
            overlaps(&shell(0.0, 0.0), &shell(0.5, -30.0), eye),
            "behind it"
        );
        assert!(
            !overlaps(&shell(0.0, 0.0), &shell(5.0, 0.0), eye),
            "beside it"
        );
    }
}
