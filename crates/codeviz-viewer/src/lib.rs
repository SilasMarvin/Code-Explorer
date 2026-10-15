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

use crate::explorer::{Autoplay, Explorer};

/// Configures and launches the viewer window.
#[derive(Debug)]
pub struct Viewer {
    system: System,
    root: Option<UnitId>,
    options: JourneyOptions,
    title: String,
    screenshot: Option<PathBuf>,
    scenario: Option<usize>,
    step: usize,
    codebase: bool,
    also: Vec<UnitId>,
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
            scenario: None,
            step: 0,
            codebase: false,
            also: Vec::new(),
        }
    }

    /// The unit to start from. Without one, the first entry is used.
    #[must_use]
    pub fn with_root(mut self, root: UnitId) -> Self {
        self.root = Some(root);
        self
    }

    /// Opens on this outcome (index into the entry's scenarios) instead of the happy path.
    #[must_use]
    pub fn with_scenario(mut self, scenario: usize) -> Self {
        self.scenario = Some(scenario);
        self
    }

    /// Opens with the cursor this many steps into the journey.
    #[must_use]
    pub fn with_step(mut self, step: usize) -> Self {
        self.step = step;
        self
    }

    /// Opens another journey from `root` below the first, with its cursor on the
    /// first journey's cursor unit when it reaches it.
    #[must_use]
    pub fn also(mut self, root: UnitId) -> Self {
        self.also.push(root);
        self
    }

    /// Opens with the rest of the codebase shown around the journey.
    #[must_use]
    pub fn with_codebase(mut self, codebase: bool) -> Self {
        self.codebase = codebase;
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

    fn explorer(&mut self) -> Explorer {
        let mut explorer = Explorer::new(
            std::mem::take(&mut self.system),
            self.root.take(),
            self.options.clone(),
        );
        if self.scenario.is_some() {
            explorer.set_scenario(self.scenario);
        }
        if self.step > 0 {
            explorer.select(explorer.journey().map(codeviz_core::Journey::root));
            for _ in 0..self.step {
                explorer.step(explorer::Step::Into);
            }
        }
        for root in self.also.drain(..) {
            explorer.add_track(root);
        }
        if self.codebase {
            explorer.toggle_codebase();
        }
        explorer
    }

    /// Opens the window and blocks until it is closed.
    #[must_use]
    pub fn run(mut self) -> AppExit {
        let explorer = self.explorer();
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
            camera::JourneyCameraPlugin,
            scene::ScenePlugin,
            ui::UiPlugin,
        ))
        .insert_resource(ClearColor(Color::srgb(0.06, 0.07, 0.09)))
        .insert_resource(explorer)
        .init_resource::<Autoplay>();
        if let Some(path) = self.screenshot {
            app.add_plugins(capture::CapturePlugin(path));
        }
        app.run()
    }
}
