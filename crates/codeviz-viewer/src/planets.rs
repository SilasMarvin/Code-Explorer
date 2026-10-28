//! Modules as planets: each module a solid, lit ball sized by how many functions it
//! holds, with its functions as dots on the surface, like cities on a globe, so nothing
//! hides inside a volume. Where a planet floats says which modules call each other: the
//! more calls between two modules, the closer they sit. Calls fly between the dots as
//! arcs that leave each surface straight up.
//!
//! Around the focus, every planet holding something that calls it or that it calls
//! turns those dots towards the viewer, the focus's own planet turning the focus to
//! face it, and keeps doing so as the camera orbits. The planets nothing lights stay
//! solid but dim.

use std::{
    collections::{HashMap, HashSet},
    f32::consts::PI,
};

use bevy::prelude::*;
use codeviz_layout::ForceGraph;

use crate::{
    camera::{ANGLED, OrbitCamera},
    explorer::Explorer,
    graph_view::{self, CALLEE, CALLER, FURTHER, NodeLook, fit_distance, orbit_anchor},
    lens::SPECK,
    scene::{PALETTE, Tile, lane_color},
};

/// Calls from the focus that light, on either side.
pub(crate) const HOPS: u32 = 2;

/// Surface each function takes on its planet, so neighboring dots stay apart.
const DOT_SPACING: f32 = 2.8;
/// The smallest planet, for a module of a function or two.
const MIN_RADIUS: f32 = 2.8;
/// Clear space between two planets.
const GAP: f32 = 5.0;
/// How far a dot's middle sits above its planet's surface.
const DOT_LIFT: f32 = 0.08;
/// Steps that settle where the planets float, and where dots sit on each.
const ARRANGE_STEPS: usize = 300;
const SURFACE_STEPS: usize = 120;
/// How quickly a planet turns towards where it is going, per second.
const TURN_RATE: f32 = 4.0;
/// The least Shift+Z swings the camera round, in radians.
const MIN_SWING: f32 = 0.8;

/// How much a planet takes part around the focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Hash)]
pub(crate) enum Involvement {
    #[default]
    None,
    /// Holds something two calls from the focus.
    Further,
    /// Holds a direct caller or callee.
    Near,
    /// Holds the focus.
    Focus,
}

impl Involvement {
    const ALL: [Self; 4] = [Self::None, Self::Further, Self::Near, Self::Focus];

    /// How much it weighs when choosing where to look from.
    fn weight(self) -> f32 {
        match self {
            Self::None => 0.0,
            Self::Further => 0.35,
            Self::Near => 1.0,
            Self::Focus => 3.0,
        }
    }
}

/// One module's planet.
#[derive(Debug, Clone)]
struct World {
    center: Vec3,
    radius: f32,
    /// How the planet is turned now, and what it is turning towards.
    spin: Quat,
    goal: Quat,
    involvement: Involvement,
    /// Where its lit dots sit, together, before it turns: the side it shows.
    lit_side: Vec3,
}

/// Every module's planet and where on it each function sits.
#[derive(Debug, Clone, Default)]
pub(crate) struct Planets {
    /// By cluster.
    worlds: Vec<World>,
    /// By tile: its planet, and where on it the tile sits, as a direction from the
    /// middle before the planet turns.
    homes: Vec<Option<(usize, Vec3)>>,
    /// The planet holding the focus.
    focus_world: Option<usize>,
    /// The functions lit around the focus.
    lit: HashSet<usize>,
}

impl Planets {
    /// Sizes a planet per module by its functions, floats them by the calls between
    /// them, starting from where the flat map puts each module, and spreads each
    /// module's functions over its surface.
    pub(crate) fn new(graph: &ForceGraph) -> Self {
        let count = graph.cluster_count();
        let tiles = graph.positions.len();
        let mut members: Vec<Vec<usize>> = vec![Vec::new(); count];
        for tile in 0..tiles {
            if let Some(list) = graph.cluster(tile).and_then(|c| members.get_mut(c)) {
                list.push(tile);
            }
        }
        let radii: Vec<f32> = members.iter().map(|m| radius(m.len())).collect();
        let seeds: Vec<Vec3> = members
            .iter()
            .map(|list| {
                let sum: Vec3 = list.iter().map(|&t| Vec3::from(graph.positions[t])).sum();
                sum / list.len().max(1) as f32
            })
            .collect();
        let mut weights: HashMap<(usize, usize), f32> = HashMap::new();
        let mut neighbors: Vec<Vec<usize>> = vec![Vec::new(); tiles];
        for &(from, to) in &graph.calls {
            neighbors[from].push(to);
            neighbors[to].push(from);
            if let (Some(a), Some(b)) = (graph.cluster(from), graph.cluster(to))
                && a != b
            {
                *weights.entry((a.min(b), a.max(b))).or_default() += 1.0;
            }
        }
        let centers = arrange(&radii, &seeds, &weights);
        let mut homes = vec![None; tiles];
        for (world, list) in members.iter().enumerate() {
            let index: HashMap<usize, usize> =
                list.iter().enumerate().map(|(i, &t)| (t, i)).collect();
            // Each function leans towards the planets it calls or is called from.
            let pulls: Vec<Vec3> = list
                .iter()
                .map(|&tile| {
                    neighbors[tile]
                        .iter()
                        .filter_map(|&other| graph.cluster(other))
                        .filter(|&c| c != world)
                        .map(|c| (centers[c] - centers[world]).normalize_or_zero())
                        .sum()
                })
                .collect();
            // And towards the functions of its own module it calls.
            let links: Vec<(usize, usize)> = list
                .iter()
                .enumerate()
                .flat_map(|(i, &tile)| {
                    neighbors[tile]
                        .iter()
                        .filter_map(|other| index.get(other))
                        .filter(move |&&j| j > i)
                        .map(move |&j| (i, j))
                        .collect::<Vec<_>>()
                })
                .collect();
            let dirs = settle_surface(&pulls, &links, |i| scatter(list[i]));
            for (&tile, dir) in list.iter().zip(dirs) {
                homes[tile] = Some((world, dir));
            }
        }
        let worlds = centers
            .into_iter()
            .zip(radii)
            .map(|(center, radius)| World {
                center,
                radius,
                spin: Quat::IDENTITY,
                goal: Quat::IDENTITY,
                involvement: Involvement::None,
                lit_side: Vec3::ZERO,
            })
            .collect();
        Self {
            worlds,
            homes,
            focus_world: None,
            lit: HashSet::new(),
        }
    }

    /// Where a function's dot sits now.
    pub(crate) fn point(&self, tile: usize) -> Option<Vec3> {
        let (world, home) = (*self.homes.get(tile)?)?;
        let world = self.worlds.get(world)?;
        Some(world.center + world.spin * home * (world.radius + DOT_LIFT))
    }

    /// Which way is up from a function's dot: straight out of its planet.
    pub(crate) fn normal(&self, tile: usize) -> Option<Vec3> {
        let (world, home) = (*self.homes.get(tile)?)?;
        Some(self.worlds.get(world)?.spin * home)
    }

    /// The planet a function lives on.
    pub(crate) fn world_of(&self, tile: usize) -> Option<usize> {
        self.homes
            .get(tile)
            .copied()
            .flatten()
            .map(|(world, _)| world)
    }

    /// Every planet: its cluster, middle, radius, and how much it takes part.
    pub(crate) fn worlds(&self) -> impl Iterator<Item = (usize, Vec3, f32, Involvement)> + '_ {
        self.worlds
            .iter()
            .enumerate()
            .map(|(i, w)| (i, w.center, w.radius, w.involvement))
    }

    /// Notes which planets hold the focus and what is lit around it, `lit` giving each
    /// lit function's calls from the focus.
    pub(crate) fn light(&mut self, focus: Option<usize>, lit: &[(usize, u32)]) {
        for world in &mut self.worlds {
            world.involvement = Involvement::None;
            world.lit_side = Vec3::ZERO;
        }
        self.focus_world = focus.and_then(|t| self.world_of(t));
        self.lit = lit.iter().map(|&(tile, _)| tile).chain(focus).collect();
        for &(tile, hops) in lit {
            let Some((index, home)) = self.homes.get(tile).copied().flatten() else {
                continue;
            };
            let world = &mut self.worlds[index];
            let involvement = if hops <= 1 {
                Involvement::Near
            } else {
                Involvement::Further
            };
            world.involvement = world.involvement.max(involvement);
            world.lit_side += home * if hops <= 1 { 1.0 } else { 0.3 };
        }
        if let (Some(index), Some(tile)) = (self.focus_world, focus)
            && let Some((_, home)) = self.homes.get(tile).copied().flatten()
        {
            let world = &mut self.worlds[index];
            world.involvement = Involvement::Focus;
            // The focus itself is what faces the viewer.
            world.lit_side = home;
        }
    }

    /// Turns each planet that holds something lit so that side faces `eye`, a
    /// direction towards the viewer, leaning towards the focus's planet so the arcs
    /// between them leave from the near side; the focus's planet turns the focus
    /// itself straight at the viewer. The rest turn back to how they started.
    pub(crate) fn face(&mut self, eye: Vec3) {
        let focus_center = self.focus_world.map(|i| self.worlds[i].center);
        for (index, world) in self.worlds.iter_mut().enumerate() {
            let side = world.lit_side.normalize_or_zero();
            world.goal = if world.involvement == Involvement::None || side == Vec3::ZERO {
                Quat::IDENTITY
            } else {
                let toward = match focus_center {
                    Some(center) if Some(index) != self.focus_world => {
                        (center - world.center).normalize_or_zero()
                    }
                    _ => Vec3::ZERO,
                };
                Quat::from_rotation_arc(side, (eye + toward * 0.7).normalize_or(eye))
            };
        }
    }

    /// By planet, whether it takes no part and stands between `eye`, where the viewer
    /// is, and a planet that does, hiding some of it.
    pub(crate) fn in_the_way(&self, eye: Vec3) -> Vec<bool> {
        let behind: Vec<(Vec3, f32, f32)> = self
            .worlds
            .iter()
            .filter(|w| w.involvement >= Involvement::Near)
            .map(|w| {
                let toward = w.center - eye;
                let distance = toward.length().max(1e-3);
                (
                    toward / distance,
                    distance,
                    (w.radius / distance).min(1.0).asin(),
                )
            })
            .collect();
        self.worlds
            .iter()
            .map(|world| {
                if world.involvement >= Involvement::Near {
                    return false;
                }
                let toward = world.center - eye;
                let distance = toward.length().max(1e-3);
                let size = (world.radius / distance).min(1.0).asin();
                behind.iter().any(|&(direction, further, width)| {
                    further > distance
                        && direction.angle_between(toward / distance) < size + width * 0.8
                })
            })
            .collect()
    }

    /// Eases every planet's turn towards its goal. Returns whether any turned.
    pub(crate) fn ease(&mut self, dt: f32) -> bool {
        let share = 1.0 - (-TURN_RATE * dt).exp();
        let mut turned = false;
        for world in &mut self.worlds {
            if world.spin.angle_between(world.goal) > 1e-4 {
                world.spin = world.spin.slerp(world.goal, share);
                turned = true;
            }
        }
        turned
    }

    /// Turns every planet to its goal at once.
    fn settle(&mut self) {
        for world in &mut self.worlds {
            world.spin = world.goal;
        }
    }

    /// The direction to look from, towards the viewer, that best shows the planets
    /// taking part: spread across the view, none hiding behind another, from a little
    /// above. With `away_from`, the best that is clearly another angle than that one.
    pub(crate) fn view(&self, away_from: Option<Vec3>) -> Vec3 {
        let taking_part: Vec<(Vec3, f32, f32)> = self
            .worlds
            .iter()
            .filter(|w| w.involvement != Involvement::None)
            .map(|w| (w.center, w.radius, w.involvement.weight()))
            .collect();
        let preferred =
            (Quat::from_euler(EulerRot::YXZ, ANGLED.0, ANGLED.1, 0.0) * Vec3::Z).normalize();
        // Swinging, a little further round than the least swing, all else equal.
        let preferred = away_from.map_or(preferred, |eye| {
            Quat::from_rotation_y(MIN_SWING * 1.3) * eye
        });
        if taking_part.len() < 2 {
            return preferred;
        }
        let total: f32 = taking_part.iter().map(|&(_, _, w)| w).sum();
        let middle = taking_part.iter().map(|&(c, _, w)| c * w).sum::<Vec3>() / total;
        let candidates: Vec<Vec3> = sphere_points(400)
            .into_iter()
            // From level to well above, never from below, so up stays up.
            .filter(|d| d.y > -0.15 && d.y < 0.85)
            .filter(|&d| away_from.is_none_or(|eye| d.angle_between(eye) > MIN_SWING))
            .collect();
        let spread = |eye: Vec3| {
            taking_part
                .iter()
                .map(|&(c, _, w)| {
                    let offset = c - middle;
                    (offset - eye * offset.dot(eye)).length_squared() * w
                })
                .sum::<f32>()
        };
        let widest = candidates.iter().map(|&d| spread(d)).fold(1e-3, f32::max);
        let hidden = |eye: Vec3| {
            let mut hidden = 0.0;
            for &(center, radius, weight) in &taking_part {
                // What takes no part thins to glass in the way; only planets that
                // take part can hide each other.
                for other in self
                    .worlds
                    .iter()
                    .filter(|w| w.involvement >= Involvement::Near)
                {
                    // Two that take part overlapping on screen hide one another,
                    // whichever is nearer.
                    let offset = other.center - center;
                    if offset.length() < 1e-3 {
                        continue;
                    }
                    let apart = (offset - eye * offset.dot(eye)).length();
                    let overlap = (other.radius + radius - apart) / (2.0 * radius);
                    hidden += overlap.clamp(0.0, 1.0) * weight;
                }
            }
            hidden / total
        };
        candidates
            .into_iter()
            .map(|eye| {
                let score = spread(eye) / widest - 3.0 * hidden(eye) + 0.25 * eye.dot(preferred);
                (eye, score)
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(preferred, |(eye, _)| eye)
    }
}

/// A planet just big enough for `members` dots spaced [`DOT_SPACING`] apart.
fn radius(members: usize) -> f32 {
    (DOT_SPACING * (members as f32 / (4.0 * PI)).sqrt()).max(MIN_RADIUS)
}

/// A direction fixed by `key`, so the same codebase always starts from the same shape.
fn scatter(key: usize) -> Vec3 {
    let unit = |salt: f32| (key as f32 * 0.618_034 + salt).fract();
    let z = unit(0.137) * 2.0 - 1.0;
    let angle = unit(0.71) * 7.0 * std::f32::consts::TAU;
    let ring = (1.0 - z * z).sqrt();
    Vec3::new(ring * angle.cos(), ring * angle.sin(), z)
}

/// `count` directions spread evenly over a sphere.
fn sphere_points(count: usize) -> Vec<Vec3> {
    let golden = PI * (3.0 - 5.0_f32.sqrt());
    (0..count)
        .map(|i| {
            let y = 1.0 - 2.0 * (i as f32 + 0.5) / count as f32;
            let ring = (1.0 - y * y).sqrt();
            let angle = golden * i as f32;
            Vec3::new(ring * angle.cos(), y, ring * angle.sin())
        })
        .collect()
}

/// Where each planet floats: modules that call each other more sit closer, no two
/// overlap, and the whole keeps the flat map's arrangement it starts from, lifted
/// into a thick disc so there is depth to look across.
fn arrange(radii: &[f32], seeds: &[Vec3], weights: &HashMap<(usize, usize), f32>) -> Vec<Vec3> {
    const REPULSION: f32 = 0.6;
    const ATTRACTION: f32 = 0.012;
    const GRAVITY: f32 = 0.03;
    const FLATTEN: f32 = 0.02;
    let count = radii.len();
    if count == 0 {
        return Vec::new();
    }
    let middle = seeds.iter().copied().sum::<Vec3>() / count as f32;
    let spread = (seeds
        .iter()
        .map(|s| s.truncate().distance_squared(middle.truncate()))
        .sum::<f32>()
        / count as f32)
        .sqrt()
        .max(1.0);
    // About the size of a ball holding them all.
    let room = radii
        .iter()
        .map(|r| (r + GAP / 2.0).powi(3))
        .sum::<f32>()
        .cbrt()
        * 1.4;
    let mut at: Vec<Vec3> = seeds
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let flat = (s.truncate() - middle.truncate()) / spread * room;
            flat.extend((scatter(i * 7 + 3).z) * room * 0.5)
        })
        .collect();
    // A few planets gather close; many keep looser, so each still shows.
    let gravity = GRAVITY * (10.0 / count as f32).sqrt().min(1.0);
    for _ in 0..ARRANGE_STEPS {
        let mut force = vec![Vec3::ZERO; count];
        for a in 0..count {
            for b in (a + 1)..count {
                let delta = at[a] - at[b];
                let distance = delta.length().max(0.01);
                let reach = radii[a] + radii[b] + GAP;
                let push = delta / distance * (REPULSION * reach * reach / (distance * distance));
                force[a] += push;
                force[b] -= push;
            }
        }
        for (&(a, b), &weight) in weights {
            let delta = at[b] - at[a];
            let distance = delta.length().max(0.01);
            let reach = radii[a] + radii[b] + GAP;
            let pull = delta / distance * (ATTRACTION * (1.0 + weight).ln() * (distance - reach));
            force[a] += pull;
            force[b] -= pull;
        }
        for (point, force) in at.iter_mut().zip(&force) {
            let mut force = *force - *point * gravity;
            force.z -= point.z * FLATTEN;
            *point += force.clamp_length_max(2.0);
        }
        separate(&mut at, radii);
    }
    at
}

/// Pushes any two planets that overlap straight apart.
fn separate(at: &mut [Vec3], radii: &[f32]) {
    for a in 0..at.len() {
        for b in (a + 1)..at.len() {
            let delta = at[a] - at[b];
            let reach = radii[a] + radii[b] + GAP * 0.5;
            let distance = delta.length();
            if distance < reach {
                let fix = delta.normalize_or(Vec3::X) * (reach - distance) / 2.0;
                at[a] += fix;
                at[b] -= fix;
            }
        }
    }
}

/// Spreads a planet's dots over its surface as directions from its middle: each
/// leans towards `pulls` (the planets it talks to), towards the dots it is linked to
/// among `links`, and away from every other dot so they stay apart.
fn settle_surface(
    pulls: &[Vec3],
    links: &[(usize, usize)],
    seed: impl Fn(usize) -> Vec3,
) -> Vec<Vec3> {
    let count = pulls.len();
    let mut dirs: Vec<Vec3> = (0..count)
        .map(|i| (pulls[i].normalize_or_zero() * 1.5 + seed(i)).normalize_or(Vec3::Z))
        .collect();
    if count < 2 {
        return dirs;
    }
    // How far apart, in radians, dots spread evenly would be.
    let spacing = (4.0 * PI / count as f32).sqrt();
    for _ in 0..SURFACE_STEPS {
        let mut force = vec![Vec3::ZERO; count];
        for a in 0..count {
            for b in (a + 1)..count {
                let delta = dirs[a] - dirs[b];
                let squared = delta.length_squared().max(1e-4);
                let push = delta * (spacing * spacing * 0.08 / squared);
                force[a] += push;
                force[b] -= push;
            }
        }
        for &(a, b) in links {
            let delta = dirs[b] - dirs[a];
            force[a] += delta * 0.04;
            force[b] -= delta * 0.04;
        }
        for (i, dir) in dirs.iter_mut().enumerate() {
            let force = force[i] + pulls[i].normalize_or_zero() * spacing * 0.06;
            let along = force - *dir * force.dot(*dir);
            *dir = (*dir + along.clamp_length_max(spacing * 0.3)).normalize_or(*dir);
        }
    }
    dirs
}

/// How big a function's dot is drawn, by how it looks.
pub(crate) fn dot_size(look: NodeLook) -> f32 {
    match look {
        NodeLook::Focus => 1.25,
        NodeLook::Near(_, 1) | NodeLook::Changed(_) => 0.95,
        NodeLook::Near(_, 2) | NodeLook::Impacted(_) => 0.7,
        NodeLook::Near(..) | NodeLook::Unchanged(_) => 0.55,
        NodeLook::Plain(_) | NodeLook::Dim(_) | NodeLook::Faded(_) | NodeLook::Hidden => 0.42,
    }
}

/// A module's planet in the scene.
#[derive(Component, Debug)]
pub(crate) struct Planet(usize);

/// Each module's planet surface, by its color, how much it takes part, and whether it
/// stands in front of one that takes part.
#[derive(Resource, Debug, Default)]
pub(crate) struct PlanetLooks(HashMap<(usize, Involvement, bool), Handle<StandardMaterial>>);

/// A planet surface per module, hidden until the planets show.
pub(crate) fn spawn_planets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    explorer: Res<Explorer>,
) {
    let mesh = meshes.add(Sphere::new(1.0).mesh().uv(64, 40));
    let mut looks = HashMap::new();
    for color in 0..PALETTE.len() {
        let base = LinearRgba::from(lane_color(color));
        for involvement in Involvement::ALL {
            // Lit through, so each reads as a ball; what takes no part dims well back.
            let (shade, glow) = match involvement {
                Involvement::None => (0.018, 0.002),
                Involvement::Further => (0.04, 0.004),
                Involvement::Near => (0.45, 0.03),
                Involvement::Focus => (0.5, 0.035),
            };
            let material = StandardMaterial {
                base_color: Color::from(LinearRgba::rgb(
                    base.red * shade,
                    base.green * shade,
                    base.blue * shade,
                )),
                emissive: LinearRgba::rgb(base.red * glow, base.green * glow, base.blue * glow),
                perceptual_roughness: 0.75,
                reflectance: 0.25,
                ..default()
            };
            // In the way, a planet thins to glass, so what takes part shows through.
            let ghost = StandardMaterial {
                base_color: material.base_color.with_alpha(0.22),
                alpha_mode: AlphaMode::Blend,
                ..material.clone()
            };
            looks.insert((color, involvement, false), materials.add(material));
            looks.insert((color, involvement, true), materials.add(ghost));
        }
    }
    let first = looks.values().next().cloned().unwrap_or_default();
    for cluster in 0..explorer.graph().cluster_count() {
        commands.spawn((
            Planet(cluster),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(first.clone()),
            Transform::default(),
            Visibility::Hidden,
            // Clicks pass through to the dots on it.
            Pickable::IGNORE,
        ));
    }
    commands.insert_resource(PlanetLooks(looks));
}

/// Z shows the planets or puts the graph back.
pub(crate) fn toggle(keys: &ButtonInput<KeyCode>, shift: bool, explorer: &mut Explorer) {
    if keys.just_pressed(KeyCode::KeyZ) && !shift {
        let on = explorer.planets().is_none();
        explorer.set_planets(on);
    }
}

/// What lights around the focus, with how many calls away.
fn lit(explorer: &Explorer) -> Vec<(usize, u32)> {
    let Some(reach) = explorer.reach() else {
        return Vec::new();
    };
    (0..explorer.map().tiles.len())
        .filter(|&t| explorer.visibility(t) >= SPECK)
        .filter_map(|t| graph_view::near(explorer, reach, t).map(|hops| (t, hops)))
        .collect()
}

/// Turns the planets towards the camera as it moves, and puts every dot and planet
/// where it now is. Steers the light to come from over the viewer's shoulder, so the
/// planets read as balls from any side.
#[expect(
    clippy::type_complexity,
    clippy::too_many_arguments,
    reason = "a Bevy system"
)]
pub(crate) fn place(
    time: Res<Time>,
    mut explorer: ResMut<Explorer>,
    looks: Res<PlanetLooks>,
    mut was_on: Local<bool>,
    cameras: Query<&Transform, (With<OrbitCamera>, Without<Tile>, Without<Planet>)>,
    mut lights: Query<
        &mut Transform,
        (
            With<DirectionalLight>,
            Without<OrbitCamera>,
            Without<Tile>,
            Without<Planet>,
        ),
    >,
    mut nodes: Query<(&Tile, &mut Transform, &mut Visibility), Without<Planet>>,
    mut globes: Query<(
        &Planet,
        &mut Transform,
        &mut Visibility,
        &mut MeshMaterial3d<StandardMaterial>,
    )>,
) {
    let on = explorer.planets().is_some();
    let toggled = on != *was_on;
    *was_on = on;
    if !on {
        if toggled {
            for (_, _, mut visibility, _) in &mut globes {
                *visibility = Visibility::Hidden;
            }
            for (tile, mut transform, mut visibility) in &mut nodes {
                if let Some(at) = explorer.tile_point(tile.0) {
                    transform.translation = at;
                }
                visibility.set_if_neq(Visibility::Inherited);
            }
            for mut light in &mut lights {
                *light = Transform::from_xyz(2.0, 6.0, 10.0).looking_at(Vec3::ZERO, Vec3::Y);
            }
        }
        return;
    }
    let Ok(camera) = cameras.single() else {
        return;
    };
    let relit = toggled || explorer.is_changed();
    let lit = relit.then(|| lit(&explorer));
    let focus = explorer.graph_focus();
    let explorer = explorer.bypass_change_detection();
    let Some(planets) = explorer.planets_mut() else {
        return;
    };
    if let Some(lit) = &lit {
        planets.light(focus, lit);
    }
    planets.face(*camera.back());
    if toggled {
        planets.settle();
    }
    let turned = planets.ease(time.delta_secs()) || toggled;
    let in_the_way = planets.in_the_way(camera.translation);
    for (tile, mut transform, mut visibility) in &mut nodes {
        if turned && let Some(at) = planets.point(tile.0) {
            transform.translation = at;
        }
        // A planet thinned to glass shows none of its dots but what is lit.
        let glass = planets
            .world_of(tile.0)
            .is_some_and(|w| in_the_way.get(w).copied().unwrap_or(false));
        visibility.set_if_neq(if glass && !planets.lit.contains(&tile.0) {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        });
    }
    let worlds: Vec<_> = planets.worlds().collect();
    for (planet, mut transform, mut visibility, mut material) in &mut globes {
        let Some(&(cluster, center, radius, involvement)) = worlds.get(planet.0) else {
            continue;
        };
        if turned || relit {
            *visibility = Visibility::Inherited;
            *transform = Transform::from_translation(center).with_scale(Vec3::splat(radius));
        }
        let ghost = in_the_way.get(planet.0).copied().unwrap_or(false);
        if let Some(wanted) = looks.0.get(&(cluster % PALETTE.len(), involvement, ghost))
            && &material.0 != wanted
        {
            material.0 = wanted.clone();
        }
    }
    // Light from above and to the left of the viewer.
    let from = camera.back() * 1.0 + camera.up() * 0.8 + camera.left() * 0.6;
    for mut light in &mut lights {
        *light = Transform::from_translation(from).looking_at(Vec3::ZERO, Vec3::Y);
    }
}

/// A soft rim round each planet, in its module's color, so it reads as one object
/// against the dark; brighter round the planets that take part, and none round one
/// thinned to glass.
pub(crate) fn draw_rims(
    explorer: Res<Explorer>,
    cameras: Query<&Transform, With<OrbitCamera>>,
    mut gizmos: Gizmos,
) {
    let (Some(planets), Ok(camera)) = (explorer.planets(), cameras.single()) else {
        return;
    };
    let in_the_way = planets.in_the_way(camera.translation);
    for (cluster, center, radius, involvement) in planets.worlds() {
        // Glass has no rim, so it never reads as a ring over what it covers.
        if in_the_way.get(cluster).copied().unwrap_or(false) {
            continue;
        }
        let toward = camera.translation - center;
        let distance = toward.length();
        if distance <= radius {
            continue;
        }
        // The outline a ball shows from here: a little nearer than its middle.
        let ratio = radius / distance;
        let middle = center + toward / distance * (radius * ratio);
        let outline = radius * (1.0 - ratio * ratio).sqrt() * 1.015;
        let alpha = match involvement {
            Involvement::None => 0.1,
            Involvement::Further => 0.18,
            Involvement::Near | Involvement::Focus => 0.6,
        };
        gizmos
            .circle(
                Isometry3d::new(middle, camera.rotation),
                outline,
                lane_color(cluster % PALETTE.len()).with_alpha(alpha),
            )
            .resolution(64);
    }
    // A flat white ring round the focus, and a fainter one round the dot under the
    // pointer, facing the viewer as the planets' rims do.
    let ring = |tile: usize, grow: f32, color: Color, gizmos: &mut Gizmos| {
        if let Some(at) = planets.point(tile) {
            let radius = graph_view::drawn_size(&explorer, tile) * 0.5 + grow;
            gizmos
                .circle(Isometry3d::new(at, camera.rotation), radius, color)
                .resolution(32);
        }
    };
    let focus = explorer.graph_focus();
    if let Some(tile) = focus {
        ring(tile, 0.3, Color::WHITE, &mut gizmos);
    }
    if let Some(tile) = explorer.hovered_tile.filter(|&t| Some(t) != focus) {
        ring(tile, 0.5, Color::srgba(1.0, 1.0, 1.0, 0.55), &mut gizmos);
    }
}

/// Points along each arc.
const SEGMENTS: usize = 32;

/// An arc from `a` to `b` leaving each straight up from its surface (`up_a`, `up_b`),
/// higher the further it flies.
fn arc(start: Vec3, start_up: Vec3, end: Vec3, end_up: Vec3) -> Vec<Vec3> {
    let lift = (start.distance(end) * 0.4).clamp(1.2, 30.0);
    let (leave, land) = (start + start_up * lift, end + end_up * lift);
    (0..=SEGMENTS)
        .map(|i| {
            let along = i as f32 / SEGMENTS as f32;
            let rest = 1.0 - along;
            start * rest.powi(3)
                + leave * (3.0 * rest * rest * along)
                + land * (3.0 * rest * along * along)
                + end * along.powi(3)
        })
        .collect()
}

/// Only the calls around the focus fly: into it violet and out of it blue, with dots
/// running along them, one call further out pale; every other call stays out of sight,
/// so the rest of the codebase is just its planets.
pub(crate) fn draw_calls(explorer: &Explorer, time: f32, gizmos: &mut Gizmos) {
    let (Some(planets), Some(reach)) = (explorer.planets(), explorer.reach()) else {
        return;
    };
    let center = explorer.graph_focus();
    let hops = |side: &HashMap<usize, u32>, tile: usize| {
        if center == Some(tile) {
            Some(0)
        } else {
            side.get(&tile).copied()
        }
    };
    for (index, &(from, to)) in explorer.graph().calls.iter().enumerate() {
        let shown = explorer.visibility(from).min(explorer.visibility(to));
        if shown < SPECK {
            continue;
        }
        let up = hops(&reach.upstream, from).zip(hops(&reach.upstream, to));
        let down = hops(&reach.downstream, from).zip(hops(&reach.downstream, to));
        let lit = match (up, down) {
            (Some((outer, inner)), _) if outer == inner + 1 && outer <= HOPS => (CALLER, outer),
            (_, Some((inner, outer))) if outer == inner + 1 && outer <= HOPS => (CALLEE, outer),
            _ => continue,
        };
        let (Some(a), Some(b), Some(up_a), Some(up_b)) = (
            planets.point(from),
            planets.point(to),
            planets.normal(from),
            planets.normal(to),
        ) else {
            continue;
        };
        let points = arc(a, up_a, b, up_b);
        let (color, tip) = match lit {
            (color, 1) => (color.with_alpha(0.95 * shown), 1.0),
            (_, _) => (FURTHER.with_alpha(0.45 * shown), 0.7),
        };
        gizmos.linestrip(points.iter().copied(), color);
        // The head lands on the callee's dot.
        let n = points.len();
        gizmos
            .arrow(points[n - 4], points[n - 1], color)
            .with_tip_length(tip);
        if lit.1 == 1 {
            flow_along(gizmos, &points, color, time, index);
        }
    }
}

/// Dots running along an arc from caller to callee, as the flat graph runs them along
/// a line.
fn flow_along(gizmos: &mut Gizmos, points: &[Vec3], color: Color, time: f32, seed: usize) {
    let lengths: Vec<f32> = points.windows(2).map(|p| p[0].distance(p[1])).collect();
    let total: f32 = lengths.iter().sum();
    if total < 1.0 {
        return;
    }
    let count = (total / 7.0).ceil().clamp(1.0, 6.0);
    let offset = (seed as f32 * 0.618_034).fract();
    let mut dot = 0.0;
    while dot < count {
        let mut left = (time * 5.0 / total + (dot + offset) / count).fract() * total;
        dot += 1.0;
        for (pair, &length) in points.windows(2).zip(&lengths) {
            if left <= length {
                let at = pair[0].lerp(pair[1], left / length.max(1e-4));
                gizmos.sphere(
                    Isometry3d::from_translation(at),
                    0.14,
                    color.with_alpha((color.alpha() + 0.15).min(1.0)),
                );
                break;
            }
            left -= length;
        }
    }
}

/// What the camera was last framed for: whether the planets showed, the focus, how
/// many times the view was pointed somewhere by hand, and the window's shape.
type Framed = (bool, Option<usize>, u64, u32);

/// When the planets show or the focus moves, looks from where the planets taking part
/// are best seen, far enough back to take them all in; Shift+Z swings round to the
/// best view from another side.
/// When the planets go, turns back to the usual view.
pub(crate) fn frame(
    explorer: Res<Explorer>,
    keys: Res<ButtonInput<KeyCode>>,
    mut last: Local<Option<Framed>>,
    mut cameras: Query<(&mut OrbitCamera, &Projection)>,
) {
    let on = explorer.planets().is_some();
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let swing = on && shift && keys.just_pressed(KeyCode::KeyZ);
    for (mut camera, projection) in &mut cameras {
        let Projection::Perspective(perspective) = projection else {
            continue;
        };
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a coarse key"
        )]
        let shape = (perspective.aspect_ratio * 100.0).round() as u32;
        let now = (on, explorer.graph_focus(), explorer.aim(), shape);
        let was = last.replace(now);
        if was == Some(now) && !swing {
            continue;
        }
        if !on {
            if was.is_some_and(|w| w.0) {
                camera.turn_to(ANGLED);
                orbit_anchor(&explorer, &mut camera);
            }
            continue;
        }
        let Some(planets) = explorer.planets() else {
            continue;
        };
        let away = swing.then(|| {
            let (yaw, pitch) = camera.angles();
            Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0) * Vec3::Z
        });
        let angles = angles_from(planets.view(away));
        pose(planets, angles, perspective, &mut camera);
    }
}

/// The camera's yaw and pitch for looking from `eye`, a direction towards the viewer.
fn angles_from(eye: Vec3) -> (f32, f32) {
    (eye.x.atan2(eye.z), -eye.y.clamp(-1.0, 1.0).asin())
}

/// Turns the camera to `angles` and backs it off far enough to take in every planet
/// that takes part, around their middle.
fn pose(
    planets: &Planets,
    angles: (f32, f32),
    perspective: &PerspectiveProjection,
    camera: &mut OrbitCamera,
) {
    let rotation = Quat::from_euler(EulerRot::YXZ, angles.0, angles.1, 0.0);
    let (right, up) = (rotation * Vec3::X, rotation * Vec3::Y);
    let mut points: Vec<Vec3> = Vec::new();
    for (_, center, radius, involvement) in planets.worlds() {
        if involvement == Involvement::None {
            continue;
        }
        for offset in [right, -right, up, -up] {
            points.push(center + offset * radius);
        }
    }
    if points.is_empty() {
        points = planets.worlds().map(|(_, center, _, _)| center).collect();
    }
    if points.is_empty() {
        return;
    }
    // The middle of what shows, as seen from there.
    let (low, high) = points.iter().fold(
        (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN)),
        |(low, high), p| {
            let flat = Vec2::new(p.dot(right), p.dot(up));
            (low.min(flat), high.max(flat))
        },
    );
    let mean = points.iter().copied().sum::<Vec3>() / points.len() as f32;
    let back = rotation * Vec3::Z;
    let middle = (low + high) / 2.0;
    let center = right * middle.x + up * middle.y + back * mean.dot(back);
    camera.turn_to(angles);
    camera.glide_to(center, fit_distance(&points, center, rotation, perspective));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planets_never_overlap_and_callers_sit_closer() {
        let radii = [3.0, 2.0, 2.0, 4.0];
        let seeds = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
        ];
        // 0 and 3 call each other a lot; 1 and 2 not at all.
        let weights = HashMap::from([((0, 3), 40.0), ((0, 1), 1.0)]);
        let at = arrange(&radii, &seeds, &weights);
        for a in 0..at.len() {
            for b in (a + 1)..at.len() {
                assert!(
                    at[a].distance(at[b]) >= radii[a] + radii[b],
                    "{a} and {b} overlap"
                );
            }
        }
        let gap = |a: usize, b: usize| at[a].distance(at[b]) - radii[a] - radii[b];
        assert!(gap(0, 3) < gap(1, 2), "busy modules sit closer");
    }

    #[test]
    fn dots_lie_on_the_surface_leaning_towards_what_they_call() {
        let pulls = [Vec3::X, Vec3::X, Vec3::ZERO, Vec3::ZERO, -Vec3::X];
        let dirs = settle_surface(&pulls, &[(0, 1)], scatter);
        for dir in &dirs {
            assert!((dir.length() - 1.0).abs() < 1e-3);
        }
        assert!(dirs[0].x > 0.0 && dirs[1].x > 0.0);
        assert!(dirs[4].x < 0.0);
        assert!(dirs[0].distance(dirs[1]) > 0.1, "dots keep apart");
    }

    #[test]
    fn the_focus_turns_to_face_the_viewer() {
        let mut planets = Planets {
            worlds: vec![World {
                center: Vec3::ZERO,
                radius: 2.0,
                spin: Quat::IDENTITY,
                goal: Quat::IDENTITY,
                involvement: Involvement::None,
                lit_side: Vec3::ZERO,
            }],
            homes: vec![Some((0, Vec3::X)), Some((0, -Vec3::Y))],
            focus_world: None,
            lit: HashSet::new(),
        };
        planets.light(Some(1), &[(0, 1)]);
        planets.face(Vec3::Z);
        planets.settle();
        let normal = planets.normal(1).unwrap();
        assert!(
            normal.distance(Vec3::Z) < 1e-3,
            "faces the viewer: {normal}"
        );
        assert!(planets.point(1).unwrap().z > 1.9);
    }

    #[test]
    fn angles_point_the_camera_back_along_the_eye() {
        let eye = Vec3::new(0.3, 0.4, 0.8).normalize();
        let (yaw, pitch) = angles_from(eye);
        let back = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0) * Vec3::Z;
        assert!(back.distance(eye) < 1e-4, "{back} vs {eye}");
    }
}
