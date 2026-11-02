//! Interactive 3D viewer for codeviz journeys, built on Bevy.
//!
//! The viewer owns a [`System`] and draws it as module spheres with each module's
//! functions inside. Browsing, a click selects a function and lights what calls it
//! and what it calls. A [`Journey`](codeviz_core::Journey) from one function gathers
//! the view round its start, and docks along the window's foot as a panel of bars
//! sharing the graph's numbers and cursor.

#![expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value"
)]

mod camera;
mod checks;
mod colors;
mod disclosure;
mod drive;
mod explorer;
mod framing;
mod graph_view;
mod history;
mod idle;
mod journey_panel;
mod keys;
mod module_names;
mod notice;
mod nucleus;
mod pick;
mod scene;
mod script;
mod text;
mod truth;
mod ui;

use std::path::PathBuf;

use bevy::{prelude::*, window::WindowPlugin};
use bevy_egui::EguiPlugin;
use codeviz_core::System;
use codeviz_ir::UnitId;

use crate::{explorer::Explorer, journey_panel::JourneyPanel};

/// Configures and launches the viewer window.
#[derive(Debug)]
pub struct Viewer {
    system: System,
    title: String,
    screenshot: Option<PathBuf>,
    /// A session's steps, each with its line; with one, the screenshot path is the
    /// directory its snapshots go in.
    script: Option<Vec<script::Line>>,
    /// Whether to look the script's names up and stop, without opening the window.
    check_script: bool,
    focus: Option<UnitId>,
    /// How many levels of the opened journey show at first.
    depth: Option<u32>,
    /// Whether the nucleus opens expanded.
    expanded: bool,
}

/// A session's steps, read from a script a step per line (`key x`, `click
/// Explorer::spotlit`, `expect mode journey`, `snap label`; see `docs/harness.md`).
/// The functions and modules it names are looked up as the viewer opens, before its
/// window does.
#[derive(Debug)]
pub struct Script(Vec<script::Line>);

impl Script {
    /// Reads `text`.
    ///
    /// # Errors
    ///
    /// Names every line it cannot read, with what was likely meant.
    pub fn parse(text: &str) -> Result<Self, String> {
        script::parse(text).map(Self)
    }
}

impl Viewer {
    #[must_use]
    pub fn new(system: System) -> Self {
        Self {
            system,
            title: "codeviz".to_owned(),
            screenshot: None,
            script: None,
            check_script: false,
            focus: None,
            depth: None,
            expanded: false,
        }
    }

    /// Opens the journey that starts at this function; without one, the view opens
    /// browsing.
    #[must_use]
    pub fn with_focus(mut self, unit: UnitId) -> Self {
        self.focus = Some(unit);
        self
    }

    /// Opens the journey this many levels deep instead of one.
    #[must_use]
    pub fn with_depth(mut self, depth: u32) -> Self {
        self.depth = Some(depth);
        self
    }

    /// Opens with the nucleus expanded, showing how calls run inside the start's
    /// module; X shrinks it back among the others.
    #[must_use]
    pub fn expanded(mut self) -> Self {
        self.expanded = true;
        self
    }

    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// With a script, the directory a session's snapshots go in.
    #[must_use]
    pub fn with_screenshot(mut self, path: impl Into<PathBuf>) -> Self {
        self.screenshot = Some(path.into());
        self
    }

    /// Runs a session in one window instead of interactively: the script's steps,
    /// each through the input a person would give, with the view let settle after
    /// each. The screenshot path is the directory the snapshots go in.
    #[must_use]
    pub fn with_script(mut self, script: Script) -> Self {
        self.script = Some(script.0);
        self
    }

    /// Only reads the script and looks up what it names, without opening the window.
    #[must_use]
    pub fn checking_script(mut self) -> Self {
        self.check_script = true;
        self
    }

    fn explorer(&mut self) -> Explorer {
        // With no journey asked for, browsing, with nothing selected: the whole map.
        let mut explorer = Explorer::new(std::mem::take(&mut self.system));
        let focus = self
            .focus
            .take()
            .and_then(|unit| explorer.map().tile_of(&unit));
        if let Some(tile) = focus {
            explorer.open_at(tile);
        }
        explorer.gather();
        if self.expanded {
            explorer.toggle_expand(None);
        }
        explorer
    }

    /// Opens the window and blocks until it is closed.
    #[must_use]
    pub fn run(mut self) -> AppExit {
        let mut explorer = self.explorer();
        if let Some(depth) = self.depth {
            explorer.set_disclose(disclosure::Disclose(depth));
        }
        // Back starts from the view as it opens.
        explorer.forget_history();
        // A script naming what is not there stops before the window opens.
        let names = match self
            .script
            .as_deref()
            .map(|lines| drive::resolve(lines, &explorer))
        {
            Some(Err(error)) => {
                eprintln!("{error}");
                return AppExit::from_code(2);
            }
            Some(Ok(names)) => names,
            None => drive::Names::default(),
        };
        if self.check_script {
            let steps = self.script.as_ref().map_or(0, Vec::len);
            println!("script ok: {steps} steps, every name found");
            return AppExit::Success;
        }
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
            EguiPlugin::default(),
            camera::OrbitCameraPlugin,
            scene::ScenePlugin,
            ui::UiPlugin,
        ))
        .insert_resource(ClearColor(Color::srgb(0.06, 0.07, 0.09)))
        .insert_resource(explorer)
        .init_resource::<JourneyPanel>()
        .init_resource::<drive::Marks>()
        .init_resource::<idle::Awake>();
        if let (Some(dir), Some(lines)) = (&self.screenshot, self.script) {
            // A session waits for the view to rest, so it keeps drawing.
            app.add_plugins(drive::DrivePlugin {
                dir: dir.clone(),
                lines,
                names,
            });
        } else {
            app.add_plugins(idle::IdlePlugin);
        }
        app.run()
    }
}
