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
mod diff_view;
mod explorer;
mod graph_view;
mod journey_panel;
mod scene;
mod source_diff;
mod ui;

use std::path::PathBuf;

use bevy::{prelude::*, window::WindowPlugin};
use bevy_egui::EguiPlugin;
use codeviz_core::{Diff, JourneyOptions, System};
use codeviz_ir::{Document, UnitId};

use crate::{
    diff_view::{DiffStyle, DiffView},
    explorer::{Autoplay, Clustering, Explorer, Quiet},
    journey_panel::{JourneyPanel, PanelMode},
};

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
    also: Vec<UnitId>,
    panel: PanelMode,
    clustering: Clustering,
    quiet: Quiet,
    focus: Option<UnitId>,
    diff: Option<Comparison>,
}

/// The older version a diff compares against, and both versions together for layout.
#[derive(Debug)]
struct Comparison {
    before: System,
    both: Option<System>,
    style: DiffStyle,
    morph: Option<f32>,
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
            also: Vec::new(),
            panel: PanelMode::default(),
            clustering: Clustering::default(),
            quiet: Quiet::default(),
            focus: None,
            diff: None,
        }
    }

    /// Shows `after` with what changed since `before` drawn over the graph.
    #[must_use]
    pub fn compare(before: Vec<Document>, after: Vec<Document>) -> Self {
        let both = System::new(codeviz_core::union(&before, after.clone()));
        let mut viewer = Self::new(System::new(after));
        viewer.diff = Some(Comparison {
            before: System::new(before),
            both: Some(both),
            style: DiffStyle::default(),
            morph: None,
        });
        viewer
    }

    /// How a diff is drawn: `status`, `impact` or `morph`.
    #[must_use]
    pub fn with_diff_style(mut self, name: &str) -> Self {
        if let Some(diff) = &mut self.diff {
            diff.style = match name {
                "impact" => DiffStyle::Impact,
                "morph" => DiffStyle::Morph,
                _ => DiffStyle::Status,
            };
        }
        self
    }

    /// Holds the morph at `t`, from 0 (before) to 1 (after), instead of playing it.
    #[must_use]
    pub fn with_morph(mut self, t: f32) -> Self {
        if let Some(diff) = &mut self.diff {
            diff.morph = Some(t);
        }
        self
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

    /// Clusters the graph by `modules` (functions in a container pull together) or
    /// by `calls` alone.
    #[must_use]
    pub fn with_clustering(mut self, name: &str) -> Self {
        self.clustering = match name {
            "calls" => Clustering::Calls,
            _ => Clustering::Modules,
        };
        self
    }

    /// Shows only what is within `near` (two calls) or `direct` (one call) of the
    /// focus; anything else shows everything.
    #[must_use]
    pub fn with_quiet(mut self, name: &str) -> Self {
        self.quiet = match name {
            "near" => Quiet::Near,
            "direct" => Quiet::Direct,
            _ => Quiet::Off,
        };
        self
    }

    /// Opens with this function focused in the graph.
    #[must_use]
    pub fn with_focus(mut self, unit: UnitId) -> Self {
        self.focus = Some(unit);
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
        let mut explorer = Explorer::with_layout(
            std::mem::take(&mut self.system),
            self.diff.as_mut().and_then(|d| d.both.take()),
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
        explorer.set_clustering(self.clustering);
        explorer.set_quiet(self.quiet);
        if let Some(tile) = self
            .focus
            .take()
            .and_then(|unit| explorer.map().tile_of(&unit))
        {
            explorer.focus_node(tile);
        }
        explorer
    }

    /// Opens the window and blocks until it is closed.
    #[must_use]
    pub fn run(mut self) -> AppExit {
        let mut explorer = self.explorer();
        let diff = self.diff.take().map(|comparison| {
            let diff = Diff::between(&comparison.before, explorer.system());
            let mut view = DiffView::new(&mut explorer, &diff, comparison.before);
            view.style = comparison.style;
            if let Some(t) = comparison.morph {
                view.morph = t;
                view.playing = false;
            }
            explorer.refresh_reach();
            view
        });
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
            diff_view::DiffPlugin,
        ))
        .insert_resource(ClearColor(Color::srgb(0.06, 0.07, 0.09)))
        .insert_resource(explorer)
        .insert_resource(JourneyPanel::new(self.panel))
        .init_resource::<Autoplay>();
        if let Some(diff) = diff {
            app.insert_resource(diff);
        }
        if let Some(path) = self.screenshot {
            app.add_plugins(capture::CapturePlugin(path));
        }
        app.run()
    }
}
