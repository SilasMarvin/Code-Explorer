//! Draws only when something changes. Bevy would otherwise draw every frame the
//! display allows, holding two cores busy over a still graph. The window wakes on
//! input; while anything eases, plays or settles, each frame asks for the next.

use std::time::Duration;

use bevy::{
    prelude::*,
    window::RequestRedraw,
    winit::{UpdateMode, WinitSettings},
};

use crate::{camera::OrbitCamera, diff_view::DiffView, explorer::Autoplay};

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

fn keep_awake(
    mut awake: ResMut<Awake>,
    autoplay: Res<Autoplay>,
    diff: Option<Res<DiffView>>,
    cameras: Query<&OrbitCamera>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    let busy = std::mem::take(&mut awake.0)
        || autoplay.playing
        || diff.is_some_and(|diff| diff.playing)
        || cameras.iter().any(OrbitCamera::easing);
    if busy {
        redraw.write(RequestRedraw);
    }
}
