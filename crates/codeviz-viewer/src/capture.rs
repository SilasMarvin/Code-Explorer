//! Non-interactive mode: render a few frames, save a screenshot, exit.

use std::path::PathBuf;

use bevy::{
    prelude::*,
    render::view::screenshot::{Capturing, Screenshot, save_to_disk},
};

/// Frames to let the scene settle (assets load, camera eases) before capturing.
const SETTLE_FRAMES: u32 = 90;
/// Frames to wait after capture so the file write completes.
const DRAIN_FRAMES: u32 = 30;

pub(crate) struct CapturePlugin(pub(crate) PathBuf);

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Capture {
            path: self.0.clone(),
            frame: 0,
            requested_at: None,
        })
        .add_systems(Update, capture);
    }
}

#[derive(Resource)]
struct Capture {
    path: PathBuf,
    frame: u32,
    requested_at: Option<u32>,
}

fn capture(
    mut commands: Commands,
    mut state: ResMut<Capture>,
    capturing: Query<(), With<Capturing>>,
    mut exit: MessageWriter<AppExit>,
) {
    state.frame += 1;
    match state.requested_at {
        None if state.frame >= SETTLE_FRAMES => {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(state.path.clone()));
            state.requested_at = Some(state.frame);
        }
        Some(at) if capturing.is_empty() && state.frame >= at + DRAIN_FRAMES => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}
