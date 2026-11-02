//! Interactive 3D viewer for codeviz journeys, built on Bevy.
//!
//! The viewer owns a [`System`] and floats it in 3D as a graph of calls, lighting up
//! what calls into the focused function and what it calls. The
//! [`Journey`](codeviz_core::Journey) through the focus sits over the graph as a flame
//! graph, sharing its cursor.

#![expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value"
)]

mod camera;
mod capture;
mod disclosure;
mod drive;
mod explorer;
mod graph_view;
mod history;
mod idle;
mod journey_panel;
mod module_names;
mod notice;
mod nucleus;
mod pick;
mod scene;
mod script;
mod truth;
mod ui;

use std::path::PathBuf;

use bevy::{prelude::*, window::WindowPlugin};
use bevy_egui::EguiPlugin;
use codeviz_core::{JourneyOptions, System};
use codeviz_ir::UnitId;

use crate::{
    explorer::{Autoplay, Explorer},
    journey_panel::{JourneyPanel, PanelMode},
};

/// Configures and launches the viewer window.
#[derive(Debug)]
pub struct Viewer {
    system: System,
    options: JourneyOptions,
    title: String,
    screenshot: Option<PathBuf>,
    /// Keys a screenshot presses first, once the scene settles.
    keys: Vec<capture::Press>,
    /// A session's steps, each with its line; with one, the screenshot path is the
    /// directory its snapshots go in.
    script: Option<Vec<(String, script::Step)>>,
    scenario: Option<usize>,
    step: usize,
    panel: PanelMode,
    focus: Option<UnitId>,
    /// The function a screenshot puts the pointer over.
    hover: Option<UnitId>,
    /// How many levels of the opened journey show at first.
    depth: Option<u32>,
    /// Whether only the graph and the journey draw, without the panels and cards.
    bare: bool,
    /// Whether the nucleus opens expanded.
    expanded: bool,
}

/// A session's steps, read from a script a step per line: `key shift+i`, `click
/// Explorer::spotlit`, `hover`, `doubleclick`, `bar 1.2`, `button close`, `drag 100 0`,
/// `scroll 2`, `wait 300`, `snap label`.
#[derive(Debug)]
pub struct Script(Vec<(String, script::Step)>);

impl Script {
    /// Reads `text`, looking each function up by name with `find`; one not found
    /// fails its step when it comes, not the script.
    ///
    /// # Errors
    ///
    /// Names the first line it cannot read.
    pub fn parse(
        text: &str,
        find: &dyn Fn(&str) -> Result<UnitId, String>,
    ) -> Result<Self, String> {
        script::parse(text, find).map(Self)
    }
}

impl Viewer {
    #[must_use]
    pub fn new(system: System) -> Self {
        Self {
            system,
            options: JourneyOptions::default(),
            title: "codeviz".to_owned(),
            screenshot: None,
            keys: Vec::new(),
            script: None,
            scenario: None,
            step: 0,
            panel: PanelMode::default(),
            focus: None,
            hover: None,
            depth: None,
            bare: false,
            expanded: false,
        }
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

    /// How the journey shows: `hidden`, `docked` over the graph, or `full` below it.
    #[must_use]
    pub fn with_journey(mut self, mode: &str) -> Self {
        self.panel = match mode {
            "hidden" => PanelMode::Hidden,
            "full" => PanelMode::Full,
            _ => PanelMode::Docked,
        };
        self
    }

    /// Opens the journey that starts at this function; without one, the view opens
    /// browsing.
    #[must_use]
    pub fn with_focus(mut self, unit: UnitId) -> Self {
        self.focus = Some(unit);
        self
    }

    /// Opens as though the pointer were over this function, for a screenshot.
    #[must_use]
    pub fn with_hover(mut self, unit: UnitId) -> Self {
        self.hover = Some(unit);
        self
    }

    /// Opens the journey this many levels deep instead of one.
    #[must_use]
    pub fn with_depth(mut self, depth: u32) -> Self {
        self.depth = Some(depth);
        self
    }

    /// Draws only the graph and the journey, without the panels and cards around them.
    #[must_use]
    pub fn bare(mut self) -> Self {
        self.bare = true;
        self
    }

    /// Opens with the nucleus expanded, showing how calls run inside the focus's
    /// module; X shrinks it back among the others.
    #[must_use]
    pub fn expanded(mut self) -> Self {
        self.expanded = true;
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

    /// Keys to press before the screenshot, once the scene settles, separated by
    /// spaces: `o`, `.`, `up`, `shift+]`.
    ///
    /// # Errors
    ///
    /// Names the first key it cannot read.
    pub fn with_keys(mut self, keys: &str) -> Result<Self, String> {
        self.keys = keys
            .split_whitespace()
            .map(|key| capture::Press::parse(key).ok_or_else(|| format!("unknown key `{key}`")))
            .collect::<Result<_, _>>()?;
        Ok(self)
    }

    /// Runs a session in one window instead of taking one screenshot: the script's
    /// steps, each through the input a person would give, with the view let settle
    /// after each. The screenshot path is the directory the snapshots go in.
    #[must_use]
    pub fn with_script(mut self, script: Script) -> Self {
        self.script = Some(script.0);
        self
    }

    fn explorer(&mut self) -> Explorer {
        let mut explorer =
            Explorer::new(std::mem::take(&mut self.system), None, self.options.clone());
        if self.scenario.is_some() {
            explorer.set_scenario(self.scenario);
        }
        if self.step > 0 {
            explorer.select(explorer.journey().map(codeviz_core::Journey::root));
            for _ in 0..self.step {
                explorer.step(explorer::Step::Into);
            }
        }
        let focus = self
            .focus
            .take()
            .and_then(|unit| explorer.map().tile_of(&unit));
        if let Some(tile) = focus {
            explorer.open_at(tile);
        } else {
            // With no journey asked for, browsing, with nothing selected: the whole map.
            explorer.leave_journey();
            explorer.clear_selection();
        }
        explorer.gather();
        explorer.expand_nucleus(self.expanded);
        explorer.hovered_tile = self
            .hover
            .take()
            .and_then(|unit| explorer.map().tile_of(&unit));
        explorer
    }

    /// Opens the window and blocks until it is closed.
    #[must_use]
    pub fn run(mut self) -> AppExit {
        let mut explorer = self.explorer();
        if let Some(depth) = self.depth {
            explorer.set_disclose(disclosure::Disclose::Depth(depth));
        }
        // Back starts from the view as it opens.
        explorer.forget_history();
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
        .insert_resource(JourneyPanel::new(self.panel))
        .insert_resource(ui::Chrome {
            bare: self.bare,
            ..default()
        })
        .init_resource::<Autoplay>()
        .init_resource::<idle::Awake>();
        if let (Some(dir), Some(steps)) = (&self.screenshot, self.script) {
            // A session waits for the view to rest, so it keeps drawing.
            app.add_plugins(drive::DrivePlugin {
                dir: dir.clone(),
                steps,
            });
        } else if let Some(path) = self.screenshot {
            // A screenshot waits frames out, so it keeps drawing.
            app.add_plugins(capture::CapturePlugin {
                path,
                keys: self.keys,
            });
        } else {
            app.add_plugins(idle::IdlePlugin);
        }
        app.run()
    }
}
