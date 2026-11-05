//! What the pointer is on, what counts as hovered, and what a click picks. A function
//! is picked by where it shows on screen rather than by the first sphere a ray meets:
//! a named function wins over a faint unnamed speck lying a pixel nearer, a name picks
//! the function it names, and what is not drawn is never picked.

use bevy::{prelude::*, window::PrimaryWindow};
use bevy_egui::{EguiContexts, input::EguiWantsInput};

use crate::{
    camera::OrbitCamera,
    explorer::{Explorer, Picked},
    graph_view,
    idle::Awake,
    labels,
    nucleus::Nucleus,
    scene::Tile,
};

/// How far past a function's drawn edge, in points, the pointer still counts as on it.
const SLOP: f32 = 3.0;

/// Furthest the pointer may move between press and release for it to be a click
/// rather than a drag, in points.
const CLICK_TRAVEL: f32 = 4.0;

/// A function on screen, as the pointer may find it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OnScreen {
    pub(crate) tile: usize,
    /// Where its middle shows, and how far its edge is from it, in window points.
    pub(crate) at: Vec2,
    pub(crate) radius: f32,
    /// How far in front of the camera it is.
    pub(crate) depth: f32,
    /// A faint unnamed function in the nucleus.
    pub(crate) speck: bool,
    /// Lit, and so drawn over the rest whatever lies nearer.
    pub(crate) over: bool,
}

/// The function the pointer at `pointer` is on, of `spots`: a named function before a
/// speck, one whose disc holds the pointer before one it only grazes, one drawn over
/// the rest before one under it, then the nearest the camera.
pub(crate) fn pick(spots: &[OnScreen], pointer: Vec2) -> Option<usize> {
    spots
        .iter()
        .filter_map(|spot| {
            let off = spot.at.distance(pointer);
            (off <= spot.radius + SLOP).then_some((spot, off <= spot.radius))
        })
        .min_by(|(a, a_in), (b, b_in)| {
            (a.speck, !a_in, !a.over)
                .cmp(&(b.speck, !b_in, !b.over))
                .then(a.depth.total_cmp(&b.depth))
        })
        .map(|(spot, _)| spot.tile)
}

/// Sets the function under the pointer: the one whose name the pointer is on, else
/// the one [`pick`] finds. Over the panels and cards it lets go of what it set, so
/// the journey panel can light a node of its own.
pub(crate) fn point(
    window: Single<&Window, With<PrimaryWindow>>,
    wants: Res<EguiWantsInput>,
    mut contexts: EguiContexts,
    cameras: Query<(&Camera, &GlobalTransform), With<OrbitCamera>>,
    tiles: Query<(&Tile, &GlobalTransform, &InheritedVisibility)>,
    mut explorer: ResMut<Explorer>,
    mut set: Local<Option<usize>>,
) {
    let cursor = window.cursor_position();
    let mut number = None;
    let over = match cursor {
        Some(pointer) if !wants.wants_any_pointer_input() => {
            let named = contexts
                .ctx_mut()
                .ok()
                .and_then(|ctx| labels::label_at(ctx, egui_pos(pointer)))
                .filter(|(tile, _)| explorer.nucleus().drawn(*tile))
                .map(|(tile, on_label)| {
                    number = on_label;
                    tile
                });
            named.or_else(|| {
                let (camera, view) = cameras.single().ok()?;
                let spots: Vec<OnScreen> = tiles
                    .iter()
                    .filter(|(tile, _, seen)| seen.get() && explorer.nucleus().drawn(tile.0))
                    .filter_map(|(tile, placed, _)| spot(&explorer, camera, view, tile.0, placed))
                    .collect();
                pick(&spots, pointer)
            })
        }
        _ => {
            // Off the graph: only what this set is let go of.
            if set.is_some() && explorer.under_pointer == *set {
                explorer.under_pointer = None;
            }
            *set = None;
            if explorer.module_under_pointer.is_some() {
                explorer.module_under_pointer = None;
            }
            return;
        }
    };
    let module = cursor.and_then(|pointer| {
        let (camera, view) = cameras.single().ok()?;
        let ray = camera.viewport_to_world(view, pointer).ok()?;
        module_at(explorer.nucleus(), ray)
    });
    if explorer.module_under_pointer != module {
        explorer.module_under_pointer = module;
    }
    if explorer.under_pointer != over {
        explorer.under_pointer = over;
    }
    if explorer.under_pointer_number != number {
        explorer.under_pointer_number = number;
    }
    *set = over;
}

/// Where a function shows on screen, and how big.
fn spot(
    explorer: &Explorer,
    camera: &Camera,
    view: &GlobalTransform,
    tile: usize,
    placed: &GlobalTransform,
) -> Option<OnScreen> {
    let middle = placed.translation();
    let at = camera.world_to_viewport(view, middle).ok()?;
    // The sphere is half a unit across before it is scaled.
    let radius = placed.scale().x * 0.5;
    let edge = camera
        .world_to_viewport(view, middle + view.up() * radius)
        .ok()?;
    Some(OnScreen {
        tile,
        at,
        radius: at.distance(edge),
        depth: (middle - view.translation()).dot(*view.forward()),
        speck: explorer.nucleus().speck(tile),
        over: graph_view::drawn_over(explorer, tile),
    })
}

/// The module whose drawn sphere `ray` enters first.
fn module_at(nucleus: &Nucleus, ray: Ray3d) -> Option<usize> {
    nucleus
        .balls()
        .filter_map(|(cluster, ball)| {
            let to_middle = ball.center - ray.origin;
            let along = to_middle.dot(*ray.direction);
            let off = to_middle.length_squared() - along * along;
            let reach = ball.radius * ball.radius - off;
            // From inside a sphere, it counts as entered where the ray starts.
            (reach >= 0.0).then(|| (cluster, (along - reach.sqrt()).max(0.0)))
        })
        .filter(|&(_, enters)| enters.is_finite())
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(cluster, _)| cluster)
}

fn egui_pos(at: Vec2) -> bevy_egui::egui::Pos2 {
    bevy_egui::egui::pos2(at.x, at.y)
}

/// A click on a function goes to it: browsing, it is selected; in a journey, the
/// cursor goes to it when the journey runs it, to the call numbered on the name
/// clicked, else it is set aside, selected in place. Shift-click starts the
/// journey there. A press that drags to orbit is no click.
pub(crate) fn click(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window, With<PrimaryWindow>>,
    wants: Res<EguiWantsInput>,
    mut explorer: ResMut<Explorer>,
    mut pressed: Local<Option<Vec2>>,
) {
    let cursor = window.cursor_position();
    if buttons.just_pressed(MouseButton::Left) {
        *pressed = cursor.filter(|_| !wants.wants_any_pointer_input());
    }
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    let clicked = matches!(
        (pressed.take(), cursor),
        (Some(a), Some(b)) if a.distance(b) < CLICK_TRAVEL
    );
    if !clicked {
        return;
    }
    let Some(tile) = explorer.under_pointer else {
        explorer.click_empty();
        return;
    };
    if !explorer.nucleus().drawn(tile) {
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if shift {
        explorer.open_at(tile);
    } else {
        let number = explorer.under_pointer_number.clone();
        explorer.click(tile, number.as_deref());
    }
}

/// Longest between the two clicks of a double click, in seconds.
const DOUBLE_CLICK: f32 = 0.4;

/// A double click on a module expands it: anywhere on its sphere, or on one of its
/// functions; on another module than the one expanded, expands that one instead. On
/// the module expanded, off every function, it shrinks it back. It picks nothing:
/// what its first click changed, as any click does (the selection, the journey's
/// cursor, the function set aside), is put back. A press that drags to orbit is no
/// click.
pub(crate) fn double_click(
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut explorer: ResMut<Explorer>,
    mut pressed: Local<Option<(Vec2, Picked)>>,
    mut last: Local<Option<(f32, Vec2, Picked)>>,
) {
    let cursor = window.cursor_position();
    if buttons.just_pressed(MouseButton::Left) {
        *pressed = cursor.map(|at| (at, explorer.picked()));
    }
    if !buttons.just_released(MouseButton::Left) {
        return;
    }
    let (at, selected) = match (pressed.take(), cursor) {
        (Some((a, selected)), Some(b)) if a.distance(b) < CLICK_TRAVEL => (b, selected),
        _ => {
            *last = None;
            return;
        }
    };
    let now = time.elapsed_secs();
    let Some((.., selected)) = last.take().filter(|(then, was, _)| {
        now - then <= DOUBLE_CLICK && was.distance(at) < 2.0 * CLICK_TRAVEL
    }) else {
        *last = Some((now, at, selected));
        return;
    };
    explorer.keep_picked(selected);
    let expanded = explorer.nucleus().expanded_module();
    match explorer.module_pointed() {
        // Another module than the one expanded, or the first: expanded instead.
        Some(module) if Some(module) != expanded => explorer.expand_pointed(module),
        // Off every function of the module expanded: shrunk back.
        _ if expanded.is_some() && explorer.under_pointer.is_none() => {
            explorer.toggle_expand(None);
        }
        _ => {}
    }
}

/// Seconds the pointer rests on a function, or off it, before it counts as hovered
/// or no longer: passing over one on the way elsewhere draws nothing.
const HOVER_DELAY: f32 = 0.15;

/// Holds a change of what the pointer is over until it has lasted [`HOVER_DELAY`].
#[derive(Debug, Default)]
pub(crate) struct Settle {
    /// What the pointer moved onto, and since when, in seconds of app time.
    pending: Option<(Option<usize>, f32)>,
}

impl Settle {
    /// Given what the pointer is over, what is taken as pointed at now and the time:
    /// whether to take what the pointer is over instead, as it has stayed there long
    /// enough.
    fn settle(&mut self, over: Option<usize>, taken: Option<usize>, now: f32) -> bool {
        if over == taken {
            self.pending = None;
            return false;
        }
        match self.pending {
            Some((tile, since)) if tile == over => {
                let landed = now - since >= HOVER_DELAY;
                if landed {
                    self.pending = None;
                }
                landed
            }
            _ => {
                self.pending = Some((over, now));
                false
            }
        }
    }

    /// Whether a change is being held, so the frames keep coming until it lands.
    fn waiting(&self) -> bool {
        self.pending.is_some()
    }
}

/// What [`hover`] keeps between frames.
#[derive(Debug, Default)]
pub(crate) struct Pointing {
    held: Settle,
    /// Where the pointer was at the last click or key, while it has not moved since.
    resting: Option<Vec2>,
}

/// The one notion of hover: the function under the pointer counts as hovered once
/// the pointer has rested on it, so passing over a function draws nothing; its name
/// and ring show, and its bars light. What is not
/// drawn is never hovered, though the pointer was on it when it went. After a click
/// or a key the view moves under a resting pointer, so nothing counts as hovered
/// until the pointer itself moves.
pub(crate) fn hover(
    time: Res<Time>,
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut explorer: ResMut<Explorer>,
    mut awake: ResMut<Awake>,
    mut pointing: Local<Pointing>,
) {
    let pointer = windows.single().ok().and_then(Window::cursor_position);
    if buttons.get_just_pressed().next().is_some() || keys.get_just_pressed().next().is_some() {
        pointing.resting = pointer;
    }
    if pointing
        .resting
        .zip(pointer)
        .is_none_or(|(at, now)| at.distance(now) > 3.0)
    {
        pointing.resting = None;
    }
    let taken = explorer.nucleus().hovered();
    let over = explorer
        .under_pointer
        .filter(|&t| pointing.resting.is_none() && explorer.nucleus().drawn(t));
    if pointing.held.settle(over, taken, time.elapsed_secs())
        && explorer.bypass_change_detection().hover(over)
    {
        explorer.set_changed();
    }
    if pointing.held.waiting() {
        awake.0 = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(tile: usize, x: f32, radius: f32, depth: f32, speck: bool) -> OnScreen {
        OnScreen {
            tile,
            at: Vec2::new(x, 0.0),
            radius,
            depth,
            speck,
            over: false,
        }
    }

    #[test]
    fn a_lit_dot_drawn_over_a_nearer_plain_one_is_picked() {
        let lit = OnScreen {
            over: true,
            ..spot(1, 1.0, 6.0, 9.0, false)
        };
        let spots = [lit, spot(2, 0.0, 6.0, 3.0, false)];
        assert_eq!(pick(&spots, Vec2::ZERO), Some(1));
    }

    #[test]
    fn a_named_function_wins_over_a_speck_in_front() {
        let spots = [spot(1, 2.0, 3.0, 5.0, true), spot(2, 0.0, 12.0, 9.0, false)];
        assert_eq!(pick(&spots, Vec2::ZERO), Some(2));
    }

    #[test]
    fn a_speck_alone_is_still_picked() {
        let spots = [spot(1, 2.0, 3.0, 5.0, true)];
        assert_eq!(pick(&spots, Vec2::ZERO), Some(1));
        assert_eq!(
            pick(&spots, Vec2::new(20.0, 0.0)),
            None,
            "nor anything far off"
        );
    }

    #[test]
    fn of_two_named_the_one_holding_the_pointer_then_the_nearer() {
        let grazed = [
            spot(1, 13.0, 11.0, 1.0, false),
            spot(2, 0.0, 4.0, 9.0, false),
        ];
        assert_eq!(pick(&grazed, Vec2::ZERO), Some(2), "held beats grazed");
        let both = [spot(1, 1.0, 8.0, 9.0, false), spot(2, 0.0, 8.0, 3.0, false)];
        assert_eq!(pick(&both, Vec2::ZERO), Some(2), "then the nearer");
    }

    #[test]
    fn the_pointer_counts_once_it_rests() {
        let mut held = Settle::default();
        assert!(!held.settle(Some(3), None, 0.0), "not at once");
        assert!(held.waiting());
        assert!(!held.settle(Some(3), None, 0.1));
        assert!(held.settle(Some(3), None, 0.16), "after a rest");
        assert!(!held.waiting());
        // Passing over one function on the way to another takes neither.
        assert!(!held.settle(Some(4), Some(3), 1.0));
        assert!(!held.settle(Some(5), Some(3), 1.1));
        assert!(!held.settle(Some(5), Some(3), 1.2));
        assert!(held.settle(Some(5), Some(3), 1.26));
        // Off and back on before the delay keeps it.
        assert!(!held.settle(None, Some(5), 2.0));
        assert!(!held.settle(Some(5), Some(5), 2.05));
        assert!(!held.waiting());
    }
}
