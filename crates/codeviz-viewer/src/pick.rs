//! What the pointer is on, and what a click picks. A function is picked by where it
//! shows on screen rather than by the first sphere a ray meets: a named function wins
//! over a faint unnamed speck lying a pixel nearer, a name picks the function it
//! names, and a speck the nucleus leaves out (N) is never picked.

use bevy::{prelude::*, window::PrimaryWindow};
use bevy_egui::{EguiContexts, input::EguiWantsInput};

use crate::{
    camera::OrbitCamera,
    explorer::Explorer,
    graph_view::{hidden_speck, speck},
    journey_panel::{JourneyPanel, PanelMode},
    scene::Tile,
    ui,
};

/// How far past a function's drawn edge, in points, the pointer still counts as on it.
const SLOP: f32 = 3.0;

/// Furthest the pointer may move between press and release for it to be a click
/// rather than a drag, in points.
const CLICK_TRAVEL: f32 = 4.0;

/// A function on screen, as the pointer may find it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Spot {
    pub(crate) tile: usize,
    /// Where its middle shows, and how far its edge is from it, in window points.
    pub(crate) at: Vec2,
    pub(crate) radius: f32,
    /// How far in front of the camera it is.
    pub(crate) depth: f32,
    /// A faint unnamed function in the nucleus.
    pub(crate) speck: bool,
}

/// The function the pointer at `pointer` is on, of `spots`: a named function before a
/// speck, one whose disc holds the pointer before one it only grazes, then the
/// nearest the camera.
pub(crate) fn pick(spots: &[Spot], pointer: Vec2) -> Option<usize> {
    spots
        .iter()
        .filter_map(|spot| {
            let off = spot.at.distance(pointer);
            (off <= spot.radius + SLOP).then_some((spot, off <= spot.radius))
        })
        .min_by(|(a, a_in), (b, b_in)| {
            (a.speck, !a_in)
                .cmp(&(b.speck, !b_in))
                .then(a.depth.total_cmp(&b.depth))
        })
        .map(|(spot, _)| spot.tile)
}

/// Whether a function can be pointed at: drawn, and not a speck left out.
fn pointable(explorer: &Explorer, tile: usize) -> bool {
    (explorer.shown(tile) || explorer.nucleus().shows(tile)) && !hidden_speck(explorer, tile)
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
                .and_then(|ctx| ui::label_at(ctx, egui_pos(pointer)))
                .filter(|(tile, _)| pointable(&explorer, *tile))
                .map(|(tile, on_label)| {
                    number = on_label;
                    tile
                });
            named.or_else(|| {
                let (camera, view) = cameras.single().ok()?;
                let spots: Vec<Spot> = tiles
                    .iter()
                    .filter(|(tile, _, seen)| seen.get() && pointable(&explorer, tile.0))
                    .filter_map(|(tile, placed, _)| spot(&explorer, camera, view, tile.0, placed))
                    .collect();
                pick(&spots, pointer)
            })
        }
        _ => {
            // Off the graph: only what this set is let go of.
            if set.is_some() && explorer.hovered_tile == *set {
                explorer.hovered_tile = None;
            }
            *set = None;
            return;
        }
    };
    if explorer.hovered_tile != over {
        explorer.hovered_tile = over;
    }
    if explorer.pointed_number != number {
        explorer.pointed_number = number;
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
) -> Option<Spot> {
    let middle = placed.translation();
    let at = camera.world_to_viewport(view, middle).ok()?;
    // The sphere is half a unit across before it is scaled.
    let radius = placed.scale().x * 0.5;
    let edge = camera
        .world_to_viewport(view, middle + view.up() * radius)
        .ok()?;
    Some(Spot {
        tile,
        at,
        radius: at.distance(edge),
        depth: (middle - view.translation()).dot(*view.forward()),
        speck: speck(explorer, tile),
    })
}

fn egui_pos(at: Vec2) -> bevy_egui::egui::Pos2 {
    bevy_egui::egui::pos2(at.x, at.y)
}

/// A click on a function moves the focus to it: browsing, it is selected; in a
/// journey, the cursor goes to it when the journey runs it, to the call numbered on
/// the name clicked, else it is selected alone, for now. Shift-click starts the
/// journey there. A press that drags to orbit is no click.
pub(crate) fn click(
    buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    window: Single<&Window, With<PrimaryWindow>>,
    wants: Res<EguiWantsInput>,
    mut explorer: ResMut<Explorer>,
    mut panel: ResMut<JourneyPanel>,
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
    let Some(tile) = explorer.hovered_tile.filter(|_| clicked) else {
        return;
    };
    if !explorer.shown(tile) || hidden_speck(&explorer, tile) {
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    if shift {
        explorer.open_at(tile);
    } else {
        let number = explorer.pointed_number.clone();
        explorer.focus_tile(tile, number.as_deref());
    }
    // Opening a journey shows it in the panel.
    if shift && panel.mode == PanelMode::Hidden {
        panel.mode = PanelMode::Docked;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spot(tile: usize, x: f32, radius: f32, depth: f32, speck: bool) -> Spot {
        Spot {
            tile,
            at: Vec2::new(x, 0.0),
            radius,
            depth,
            speck,
        }
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
}
