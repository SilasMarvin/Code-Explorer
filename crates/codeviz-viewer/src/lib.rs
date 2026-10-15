//! Interactive 3D viewer for codeviz journeys, built on Bevy.
//!
//! The viewer owns a [`System`] and renders the [`Journey`](codeviz_core::Journey) from
//! one entry at a time, laid out by [`codeviz_layout`]: time runs along X, depth down Y,
//! and lanes (containers by default) along Z.

#![expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value"
)]

mod camera;
mod capture;
mod explorer;
mod scene;
mod ui;

use std::path::PathBuf;

use bevy::{prelude::*, window::WindowPlugin};
use bevy_egui::EguiPlugin;
use codeviz_core::{JourneyOptions, System};
use codeviz_ir::UnitId;

use crate::explorer::{Explorer, Playback};

/// Configures and launches the viewer window.
#[derive(Debug)]
pub struct Viewer {
    system: System,
    root: Option<UnitId>,
    options: JourneyOptions,
    title: String,
    screenshot: Option<PathBuf>,
}

impl Viewer {
    #[must_use]
    pub fn new(system: System) -> Self {
        Self {
            system,
            root: None,
            options: JourneyOptions::default(),
            title: "codeviz".to_owned(),
            screenshot: None,
        }
    }

    /// The unit to start from. Without one, the first entry is used.
    #[must_use]
    pub fn with_root(mut self, root: UnitId) -> Self {
        self.root = Some(root);
        self
    }

    /// Initial journey options; adjustable in the UI.
    #[must_use]
    pub fn with_options(mut self, options: JourneyOptions) -> Self {
        self.options = options;
        self
    }

    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Instead of running interactively, save a screenshot to `path` once the scene
    /// has settled, then exit.
    #[must_use]
    pub fn with_screenshot(mut self, path: impl Into<PathBuf>) -> Self {
        self.screenshot = Some(path.into());
        self
    }

    /// Opens the window and blocks until it is closed.
    #[must_use]
    pub fn run(self) -> AppExit {
        let mut app = App::new();
        app.add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: self.title,
                resolution: (1600, 1000).into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins((
            MeshPickingPlugin,
            EguiPlugin::default(),
            camera::OrbitCameraPlugin,
            scene::ScenePlugin,
            ui::UiPlugin,
        ))
        .insert_resource(ClearColor(Color::srgb(0.06, 0.07, 0.09)))
        .insert_resource(Explorer::new(self.system, self.root, self.options))
        .init_resource::<Playback>();
        if let Some(path) = self.screenshot {
            app.add_plugins(capture::CapturePlugin(path));
        }
        app.run()
    }
}
