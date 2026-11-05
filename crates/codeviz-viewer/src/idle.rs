//! Draws only when something changes. Bevy would otherwise draw every frame the
//! display allows, holding two cores busy over a still graph. The window wakes on
//! input; while anything eases, each frame asks for the next; still, it sleeps.

use std::time::Duration;

use bevy::{
    prelude::*,
    window::RequestRedraw,
    winit::{UpdateMode, WinitSettings},
};

use codeviz_core::SpanId;

use crate::{camera::OrbitCamera, disclosure::Disclose, explorer::Explorer};

pub(crate) struct IdlePlugin;

impl Plugin for IdlePlugin {
    fn build(&self, app: &mut App) {
        // Nothing on screen changes by itself: no timed wake.
        let still = UpdateMode::reactive_low_power(Duration::from_secs(3600));
        app.insert_resource(WinitSettings {
            focused_mode: still,
            unfocused_mode: still,
        })
        .add_systems(Last, keep_awake);
    }
}

/// Set by whatever moved this frame, so the next one draws too.
#[derive(Resource, Default)]
pub(crate) struct Awake(pub(crate) bool);

/// What the view shows that the panel can change after the scene's systems have run
/// for the frame: the cursor, what is under the pointer, the depth, the function set
/// aside and the start.
type Watched = (
    Option<SpanId>,
    Option<usize>,
    (Disclose, usize),
    Option<usize>,
    Option<usize>,
);

fn keep_awake(
    mut awake: ResMut<Awake>,
    cameras: Query<&OrbitCamera>,
    explorer: Option<Res<Explorer>>,
    mut shown: Local<Option<Watched>>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    // A click or hover in the journey panel lands after the scene has drawn the
    // frame, so another one follows to show it.
    let now = explorer.map(|e| {
        (
            e.cursor_span(),
            e.under_pointer,
            (e.disclose(), e.folds_opened()),
            e.aside(),
            e.nucleus().start(),
        )
    });
    let moved = now != *shown;
    *shown = now;
    let busy = std::mem::take(&mut awake.0)
        || moved
        || cameras.iter().any(|c| c.easing() || c.turning_by_hand());
    if busy {
        redraw.write(RequestRedraw);
    }
}
