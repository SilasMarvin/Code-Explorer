//! Time as depth: the journey's calls lie along the line of sight in the order they
//! run, so stepping through the journey flies through it. The camera faces the map
//! head-on; the cursor's call sits on the map's plane at its usual size, calls already
//! made recede behind it, darker and smaller, and calls still to come hang nearer,
//! larger and see-through. Everything off the journey stays on the plane, so the map
//! reads as it always does.
//!
//! A node is one function but may be called several times; it sits at whichever of its
//! calls is nearest the cursor's, so it comes back towards the plane as its next call
//! nears.
//!
//! Two ways to place a call off the plane, on `Z`: `fly` moves it straight towards or
//! away from the map, so it drifts out from the middle of the view as it nears, as
//! stars do in a fly-through; `held` slides it along the line from the eye through its
//! place on the map, so it stays on its spot on screen and only its size, glow and
//! fade say when it runs.

use std::collections::HashMap;

use bevy::prelude::*;
use codeviz_core::SpanId;

use crate::{
    camera::{FRONT, OrbitCamera},
    explorer::Explorer,
    graph_view::{LEVELS, NodeLook, NodeMaterials, node_look, node_size},
    scene::Tile,
    ui::Chrome,
};

/// How the journey's order shows in depth.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum TimeMode {
    #[default]
    Off,
    /// Calls move straight off the map's plane, by when they run.
    Fly,
    /// Calls move along the line of sight, so each stays on its spot on screen.
    Held,
}

impl TimeMode {
    pub(crate) fn next(self) -> Self {
        match self {
            Self::Off => Self::Fly,
            Self::Fly => Self::Held,
            Self::Held => Self::Off,
        }
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Fly => "fly",
            Self::Held => "held",
        }
    }

    pub(crate) fn parse(name: &str) -> Self {
        match name {
            "fly" => Self::Fly,
            "held" => Self::Held,
            _ => Self::Off,
        }
    }
}

/// Where time puts each node, eased as the cursor moves.
#[derive(Resource, Debug, Default)]
pub(crate) struct Flight {
    /// How far each node is drawn from where the simulation puts it.
    offsets: HashMap<usize, Vec3>,
    /// How strongly each node on the journey draws, by how far it runs from the cursor.
    fades: HashMap<usize, f32>,
    /// Calls from the cursor's to each node's nearest, eased: negative is past.
    steps: HashMap<usize, f32>,
    /// The mode last drawn, so turning it on turns the camera to face the map.
    mode: TimeMode,
    /// Each node's call nearest the cursor's, by its number, and how many times the
    /// journey calls it.
    nearest: HashMap<usize, (String, usize)>,
    /// The map's plane, as far along the head-on view as the node the camera orbits.
    plane: f32,
}

impl Flight {
    /// Whether time shows as depth.
    pub(crate) fn on(&self) -> bool {
        self.mode != TimeMode::Off
    }

    /// How strongly a node draws: in full off the journey and at the cursor.
    pub(crate) fn fade(&self, tile: usize) -> f32 {
        self.fades.get(&tile).copied().unwrap_or(1.0)
    }

    /// The number of a node's call nearest the cursor's, the one it sits at, and how
    /// many times the journey calls it.
    pub(crate) fn nearest(&self, tile: usize) -> Option<(&str, usize)> {
        self.nearest
            .get(&tile)
            .map(|(number, count)| (number.as_str(), *count))
    }

    /// Calls between the cursor's and a node's nearest, when it is on the journey.
    pub(crate) fn step(&self, tile: usize) -> Option<f32> {
        self.steps.get(&tile).copied()
    }
}

pub(crate) struct FlyPlugin;

impl Plugin for FlyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Flight>();
    }
}

/// The disclosed journey's calls as they run, depth first, by span.
pub(crate) fn run_order<S: Copy>(root: S, children: impl Fn(S) -> Vec<S>) -> Vec<S> {
    let mut order = Vec::new();
    let mut stack = vec![root];
    while let Some(span) = stack.pop() {
        order.push(span);
        stack.extend(children(span).into_iter().rev());
    }
    order
}

/// For each node in `calls`, the calls between `cursor` and its nearest call: negative
/// when that call has run, positive when it is to come. An equally near past call
/// wins, as it has been seen.
pub(crate) fn steps_from(calls: &[usize], cursor: usize) -> HashMap<usize, i32> {
    let mut steps: HashMap<usize, i32> = HashMap::new();
    for (index, &tile) in calls.iter().enumerate() {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_possible_wrap,
            reason = "a journey's calls number far fewer than i32::MAX"
        )]
        let step = index as i32 - cursor as i32;
        steps
            .entry(tile)
            .and_modify(|best| {
                if step.abs() < best.abs() {
                    *best = step;
                }
            })
            .or_insert(step);
    }
    steps
}

/// How far towards the camera a call `step` calls from the cursor's sits, as a share
/// of the camera's distance: past calls fall away behind the map, quickly at first,
/// future ones rise towards the eye but never even half way, so none fills
/// the view.
pub(crate) fn depth(step: f32) -> f32 {
    if step >= 0.0 {
        NEAREST * (1.0 - (-step / 2.5).exp())
    } else {
        -DEEPEST * (1.0 - (step / 3.0).exp())
    }
}

/// How much larger than on the map a call `step` calls from the cursor's looks, for
/// sizing its name as its node: within reason, so a name stays legible.
pub(crate) fn apparent_size(step: f32) -> f32 {
    (1.0 / (1.0 - depth(step))).clamp(0.8, 1.3)
}

/// The furthest towards the eye a call comes, and the furthest behind the map it falls,
/// as shares of the camera's distance.
const NEAREST: f32 = 0.45;
const DEEPEST: f32 = 2.0;

/// How strongly a call `step` calls from the cursor's draws: in full at the cursor,
/// fading with each call, to a trace four or five calls away.
pub(crate) fn strength(step: f32) -> f32 {
    let away = step.abs();
    // What is to come fades faster: it is nearer, and would hide the map.
    let rate = if step > 0.0 { 0.22 } else { 0.16 };
    (1.0 - away * rate).clamp(TRACE, 1.0)
}

/// What is left of a call far from the cursor.
const TRACE: f32 = 0.12;

/// The disclosed journey's calls in the order they run, by node and span, and where the
/// cursor is among them.
fn journey_calls(explorer: &Explorer) -> (Vec<(usize, SpanId)>, Option<usize>) {
    let Some(journey) = explorer.journey() else {
        return (Vec::new(), None);
    };
    let disclosed = explorer.disclosure();
    let map = explorer.map();
    let shown = |span: SpanId| -> Vec<SpanId> {
        disclosed
            .children(span)
            .iter()
            .copied()
            .filter(|&c| disclosed.number(c).is_some())
            .collect()
    };
    let cursor = explorer.selected();
    let mut calls = Vec::new();
    let mut at = None;
    for span in run_order(journey.root(), shown) {
        // A call whose node the graph lacks takes no place.
        if let Some(tile) = explorer.unit_id(span).and_then(|u| map.tile_of(u)) {
            if Some(span) == cursor {
                at = Some(calls.len());
            }
            calls.push((tile, span));
        }
    }
    (calls, at)
}

/// The nodes of the calls within [`WINDOW`] of the cursor's, as the camera frames them.
pub(crate) fn window(explorer: &Explorer) -> Vec<usize> {
    let (calls, cursor) = journey_calls(explorer);
    let Some(cursor) = cursor else {
        return Vec::new();
    };
    let from = cursor.saturating_sub(WINDOW);
    calls
        .into_iter()
        .skip(from)
        .take(WINDOW * 2 + 1)
        .map(|(tile, _)| tile)
        .collect()
}

/// Calls either side of the cursor's the camera keeps in view.
const WINDOW: usize = 2;

/// For each node `steps` places, the number of the call it sits at and how many calls
/// the journey makes to it.
fn nearest_numbers(
    explorer: &Explorer,
    calls: &[(usize, SpanId)],
    cursor: Option<usize>,
    steps: &HashMap<usize, i32>,
) -> HashMap<usize, (String, usize)> {
    let Some(cursor) = cursor else {
        return HashMap::new();
    };
    let disclosed = explorer.disclosure();
    let mut counts: HashMap<usize, usize> = HashMap::new();
    for &(tile, _) in calls {
        *counts.entry(tile).or_default() += 1;
    }
    steps
        .iter()
        .filter_map(|(&tile, &step)| {
            let index = cursor.checked_add_signed(isize::try_from(step).ok()?)?;
            let &(_, span) = calls.get(index)?;
            let number = disclosed.number(span)?.to_owned();
            Some((tile, (number, counts.get(&tile).copied().unwrap_or(1))))
        })
        .collect()
}

/// Where a point on the map's plane at `flat` is drawn `rise` towards the eye: straight
/// up off the plane, or held on its line of sight so it stays on its spot on screen.
fn lift(mode: TimeMode, flat: Vec3, rise: f32, eye: Vec3) -> Vec3 {
    match mode {
        TimeMode::Held => {
            let sight = eye - flat;
            flat + sight * (rise / sight.z.max(1.0))
        }
        _ => flat + Vec3::Z * rise,
    }
}

/// Lays the journey out in depth by when each call runs, easing as the cursor moves,
/// and fades each call by how far it runs from the cursor's; eases everything back to
/// the map when off.
#[expect(clippy::too_many_arguments, reason = "a Bevy system")]
pub(crate) fn fly(
    time: Res<Time>,
    chrome: Res<Chrome>,
    mut flight: ResMut<Flight>,
    mut explorer: ResMut<Explorer>,
    materials: Res<NodeMaterials>,
    diff: Option<Res<crate::diff_view::DiffView>>,
    mut cameras: Query<(&mut OrbitCamera, &Transform)>,
    mut nodes: Query<
        (&Tile, &mut Transform, &mut MeshMaterial3d<StandardMaterial>),
        Without<OrbitCamera>,
    >,
) {
    let flight = &mut *flight;
    let mode = chrome.time;
    let turned_on = mode != TimeMode::Off && flight.mode == TimeMode::Off;
    let turned_off = mode == TimeMode::Off && flight.mode != TimeMode::Off;
    flight.mode = mode;
    if mode == TimeMode::Off && flight.offsets.is_empty() && flight.fades.is_empty() {
        return;
    }
    let Ok((mut camera, view)) = cameras.single_mut() else {
        return;
    };
    if turned_on {
        camera.turn_to(FRONT);
    }
    let explorer_ref = explorer.bypass_change_detection();
    let (calls, cursor) = journey_calls(explorer_ref);
    let tiles: Vec<usize> = calls.iter().map(|&(tile, _)| tile).collect();
    let steps = match cursor.filter(|_| mode != TimeMode::Off) {
        Some(cursor) => steps_from(&tiles, cursor),
        None => HashMap::new(),
    };
    flight.nearest = nearest_numbers(explorer_ref, &calls, cursor, &steps);
    // The map's plane: through the node the camera orbits, square to the head-on view.
    let plane = explorer_ref
        .anchor()
        .and_then(|t| explorer_ref.force_point(t))
        .map_or(0.0, |p| p.z);
    let eye = view.translation;
    let reach = camera.distance;
    let ease = 1.0 - (-time.delta_secs() * 5.0).exp();

    // Eased steps, so a node glides from one place in time to the next.
    for (&tile, &step) in &steps {
        #[expect(clippy::cast_precision_loss, reason = "a small count")]
        let step = step as f32;
        let eased = flight.steps.entry(tile).or_insert(step);
        *eased += (step - *eased) * ease;
    }
    flight.steps.retain(|tile, _| steps.contains_key(tile));
    flight.plane = plane;

    let mut offsets = std::mem::take(&mut flight.offsets);
    for tile in 0..explorer_ref.map().tiles.len() {
        let Some(home) = explorer_ref.force_point(tile) else {
            continue;
        };
        let goal = if mode == TimeMode::Off {
            Vec3::ZERO
        } else {
            let flat = Vec3::new(home.x, home.y, plane);
            let rise = flight.steps.get(&tile).map_or(0.0, |&s| depth(s) * reach);
            lift(mode, flat, rise, eye) - home
        };
        let offset = offsets.entry(tile).or_insert(Vec3::ZERO);
        *offset += (goal - *offset) * ease;
    }
    offsets.retain(|_, offset| mode != TimeMode::Off || offset.length() > 0.01);
    flight.offsets = offsets;
    explorer_ref.set_nudges(flight.offsets.clone());

    // Fades ease too; one left at full is dropped once it has been drawn so.
    for (&tile, &step) in &flight.steps {
        let fade = flight.fades.entry(tile).or_insert(1.0);
        *fade += (strength(step) - *fade) * ease;
    }
    for (&tile, fade) in &mut flight.fades {
        if !flight.steps.contains_key(&tile) {
            *fade += (1.0 - *fade) * ease;
        }
    }
    let styling = diff.is_none();
    for (tile, mut transform, mut material) in &mut nodes {
        if let Some(at) = explorer_ref.tile_point(tile.0) {
            transform.translation = at;
        }
        let Some(&fade) = flight.fades.get(&tile.0) else {
            continue;
        };
        if !styling {
            continue;
        }
        let step = flight.steps.get(&tile.0).copied();
        let Some((look, level)) = time_look(explorer_ref, tile.0, fade, step) else {
            continue;
        };
        if let Some(wanted) = materials.0.get(&(look, level))
            && material.0 != *wanted
        {
            material.0 = wanted.clone();
        }
    }
    flight
        .fades
        .retain(|tile, fade| flight.steps.contains_key(tile) || (*fade - 1.0).abs() > 0.01);
    if turned_off {
        // Everything restyles as it was.
        explorer.set_changed();
    }
}

/// How a node `step` calls from the cursor's is drawn, at `fade`: what is still to come
/// is a see-through ghost, so it never hides the map behind it. Nothing for a node the
/// lens leaves out.
fn time_look(
    explorer: &Explorer,
    tile: usize,
    fade: f32,
    step: Option<f32>,
) -> Option<(NodeLook, u8)> {
    let look = node_look(explorer, tile);
    if look == NodeLook::Hidden {
        return None;
    }
    let ahead = step.is_some_and(|s| s > 0.5);
    let look = match look {
        NodeLook::Near(color, _) | NodeLook::Faded(color) | NodeLook::Plain(color) if ahead => {
            NodeLook::Ghost(color)
        }
        look => look,
    };
    let shown = explorer.visibility(tile) * fade;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to 0..=LEVELS"
    )]
    let level = (shown * f32::from(LEVELS))
        .round()
        .clamp(0.0, f32::from(LEVELS)) as u8;
    Some((look, level))
}

/// From each call off the map's plane, a pale stem down to its spot on the map, so it
/// is plain where it belongs however far it has drifted.
pub(crate) fn draw_flight(
    flight: Res<Flight>,
    explorer: Res<Explorer>,
    cameras: Query<&Transform, With<OrbitCamera>>,
    mut gizmos: Gizmos,
) {
    if !flight.on() {
        return;
    }
    let Ok(view) = cameras.single() else {
        return;
    };
    let graph = explorer.graph();
    for &tile in flight.steps.keys() {
        let fade = flight.fade(tile);
        let (Some(home), Some(at)) = (explorer.force_point(tile), explorer.tile_point(tile)) else {
            continue;
        };
        let home = Vec3::new(home.x, home.y, flight.plane);
        // Held on its line of sight, a call's stem is hidden behind it.
        if flight.mode == TimeMode::Held || fade <= TRACE + 0.02 || home.distance(at) < 0.3 {
            continue;
        }
        let color = Color::srgba(0.85, 0.88, 0.95, 0.15 + 0.4 * fade);
        gizmos.line(home, at, color);
        gizmos.circle(
            Isometry3d::new(home, view.rotation),
            node_size(graph, tile) * 0.22,
            color,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_journey_runs_depth_first() {
        // 0 calls 1 then 4; 1 calls 2 then 3.
        let tree = |n: u32| match n {
            0 => vec![1, 4],
            1 => vec![2, 3],
            _ => vec![],
        };
        assert_eq!(run_order(0, tree), vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn a_node_called_again_sits_at_its_call_nearest_the_cursor() {
        // Node 7 runs at calls 1 and 5; node 3 at 2 only.
        let calls = [0, 7, 3, 8, 9, 7];
        let at = steps_from(&calls, 2);
        assert_eq!(at[&3], 0, "the cursor's node sits on the plane");
        assert_eq!(at[&7], -1, "nearer its first call");
        assert_eq!(at[&0], -2);
        assert_eq!(steps_from(&calls, 4)[&7], 1, "nearer its second");
        assert_eq!(steps_from(&calls, 3)[&7], -2, "a tie goes to the past");
    }

    #[test]
    fn the_past_falls_away_and_the_future_comes_near_but_not_too_near() {
        assert!(depth(0.0).abs() < 1e-6, "the cursor's call is on the plane");
        assert!(
            depth(1.0) > 0.0 && depth(2.0) > depth(1.0),
            "to come is nearer"
        );
        assert!(
            depth(-1.0) < 0.0 && depth(-2.0) < depth(-1.0),
            "past is further"
        );
        assert!(depth(100.0) <= NEAREST, "never most of the way to the eye");
        assert!(depth(-100.0) >= -DEEPEST);
    }

    #[test]
    fn calls_fade_away_from_the_cursor() {
        assert!((strength(0.0) - 1.0).abs() < 1e-6);
        assert!(strength(1.0) < 1.0 && strength(-1.0) < 1.0);
        assert!(
            strength(2.0) < strength(-2.0),
            "what is to come fades faster"
        );
        assert!(strength(3.0) > strength(4.0));
        assert!((strength(40.0) - TRACE).abs() < 1e-6, "a trace, never gone");
    }
}
