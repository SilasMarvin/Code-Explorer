//! Draws only when something changes. Bevy would otherwise draw every frame the
//! display allows, holding two cores busy over a still graph. The window wakes on
//! input; while anything eases, plays or settles, each frame asks for the next.

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
        app.insert_resource(WinitSettings {
            focused_mode: UpdateMode::reactive(Duration::from_secs(1)),
            unfocused_mode: UpdateMode::reactive_low_power(Duration::from_secs(10)),
        })
        .init_resource::<Awake>()
        .add_systems(Last, keep_awake);
    }
}

/// Set by whatever moved this frame, so the next one draws too.
#[derive(Resource, Default)]
pub(crate) struct Awake(pub(crate) bool);

/// What the view shows that the panel can change after the scene's systems have run
/// for the frame: the cursor, what is pointed at, the depth and the middle.
type Shown = (
    Option<SpanId>,
    Option<usize>,
    Disclose,
    Option<usize>,
    Option<usize>,
);

fn keep_awake(
    mut awake: ResMut<Awake>,
    cameras: Query<&OrbitCamera>,
    explorer: Option<Res<Explorer>>,
    mut shown: Local<Option<Shown>>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    // A click or hover in the journey panel lands after the scene has drawn the
    // frame, so another one follows to show it.
    let now = explorer.map(|e| {
        (
            e.selected(),
            e.hovered_tile,
            e.disclose(),
            e.aside(),
            e.nucleus_focus(),
        )
    });
    let moved = now != *shown;
    *shown = now;
    let busy = std::mem::take(&mut awake.0) || moved || cameras.iter().any(OrbitCamera::easing);
    if busy {
        redraw.write(RequestRedraw);
    }
}
