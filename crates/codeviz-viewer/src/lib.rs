//! Interactive 3D viewer for codeviz call traces, built on Bevy.
//!
//! The viewer owns a [`CallGraph`] and renders the [`Trace`](codeviz_graph::Trace) from
//! one root function at a time, laid out by [`codeviz_layout`]: time runs along X, stack
//! depth down Y, and lanes (modules by default) along Z.

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
use codeviz_graph::{CallGraph, FunctionId, TraceOptions};

use crate::explorer::{Explorer, Playback};

/// Configures and launches the viewer window.
#[derive(Debug)]
pub struct Viewer {
    graph: CallGraph,
    root: Option<FunctionId>,
    trace_options: TraceOptions,
    title: String,
    screenshot: Option<PathBuf>,
}

impl Viewer {
    #[must_use]
    pub fn new(graph: CallGraph) -> Self {
        Self {
            graph,
            root: None,
            trace_options: TraceOptions::default(),
            title: "codeviz".to_owned(),
            screenshot: None,
        }
    }

    /// The function to trace from on startup. Without one, pick it in the UI.
    #[must_use]
    pub fn with_root(mut self, root: FunctionId) -> Self {
        self.root = Some(root);
        self
    }

    /// Initial trace options; adjustable in the UI.
    #[must_use]
    pub fn with_trace_options(mut self, options: TraceOptions) -> Self {
        self.trace_options = options;
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
        .insert_resource(Explorer::new(self.graph, self.root, self.trace_options))
        .init_resource::<Playback>();
        if let Some(path) = self.screenshot {
            app.add_plugins(capture::CapturePlugin(path));
        }
        app.run()
    }
}
