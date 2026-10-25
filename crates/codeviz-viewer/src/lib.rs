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
mod cues;
mod depth_meter;
mod diff_view;
mod disclosure;
mod explorer;
mod floors;
mod graph_view;
mod history;
mod journey_panel;
mod lens;
mod scene;
mod source_diff;
mod truth;
mod ui;

use std::path::PathBuf;

use bevy::{prelude::*, window::WindowPlugin};
use bevy_egui::EguiPlugin;
use codeviz_core::{Diff, JourneyOptions, System};
use codeviz_ir::{Document, UnitId};

use crate::{
    diff_view::{DiffStyle, DiffView},
    explorer::{Autoplay, Clustering, Explorer},
    journey_panel::{JourneyPanel, PanelMode},
    lens::{Lens, Measure, Source},
};

/// Configures and launches the viewer window.
#[derive(Debug)]
pub struct Viewer {
    system: System,
    root: Option<UnitId>,
    options: JourneyOptions,
    title: String,
    screenshot: Option<PathBuf>,
    /// Keys a screenshot presses first, once the scene settles.
    keys: Vec<capture::Press>,
    scenario: Option<usize>,
    step: usize,
    panel: PanelMode,
    clustering: Clustering,
    lens: Lens,
    /// Whether the lens's starting point was chosen, rather than left to the default.
    lens_source: bool,
    /// Whether a diff opens showing only what changed.
    changes_only: bool,
    focus: Option<UnitId>,
    /// How many levels of the opened journey show at first.
    depth: Option<u32>,
    /// Whether only the graph and the journey draw, without the panels and cards.
    bare: bool,
    diff: Option<Comparison>,
    time: floors::TimeMode,
    rock_at: Option<f32>,
}

/// The older version a diff compares against, and both versions together for layout.
#[derive(Debug)]
struct Comparison {
    before: System,
    moved: codeviz_core::Moves,
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
            keys: Vec::new(),
            scenario: None,
            step: 0,
            panel: PanelMode::default(),
            clustering: Clustering::default(),
            lens: Lens::default(),
            lens_source: false,
            changes_only: true,
            focus: None,
            depth: None,
            bare: false,
            diff: None,
            time: floors::TimeMode::Off,
            rock_at: None,
        }
    }

    /// Shows `after` with what changed since `before` drawn over the graph.
    #[must_use]
    pub fn compare(before: Vec<Document>, after: Vec<Document>) -> Self {
        let system = System::new(after.clone());
        // Moved functions take their new ids on both sides.
        let (before, moved) = codeviz_core::align(before, &system);
        let both = System::new(codeviz_core::union(&before, after));
        let mut viewer = Self::new(system);
        viewer.diff = Some(Comparison {
            before: System::new(before),
            moved,
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

    /// Shows only what is near a starting point: `focus`, `changes` or `journey`
    /// (by default the changes in a diff, else the focus), out to `reach` steps
    /// measured in `calls`, `space` or a `blend` of both. Without a reach, everything
    /// shows.
    #[must_use]
    pub fn with_reveal(mut self, source: Option<&str>, measure: &str, reach: Option<u32>) -> Self {
        if let Some(source) = source {
            self.lens.source = match source {
                "changes" => Source::Changes,
                "journey" => Source::Journey,
                _ => Source::Focus,
            };
            self.lens_source = true;
        }
        self.lens.measure = match measure {
            "space" => Measure::Space,
            "blend" => Measure::Blend,
            _ => Measure::Calls,
        };
        self.lens.reach = reach;
        self
    }

    /// Opens a diff with everything showing, not only what changed.
    #[must_use]
    pub fn with_unchanged(mut self) -> Self {
        self.changes_only = false;
        self
    }

    /// Opens with this function focused in the graph.
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

    /// Draws only the graph and the journey, without the panels and cards around them.
    #[must_use]
    pub fn bare(mut self) -> Self {
        self.bare = true;
        self
    }

    /// Shows call order in depth: `floors` lifts the journey over the map as a
    /// diagram on a second floor, strung to its nodes.
    #[must_use]
    pub fn with_time(mut self, mode: &str) -> Self {
        self.time = match mode {
            "floors" => floors::TimeMode::Floors,
            _ => floors::TimeMode::Off,
        };
        self
    }

    /// Holds the time view's camera rock at `phase`, a share of a cycle, so a
    /// screenshot can show one moment of it.
    #[must_use]
    pub fn with_rock_at(mut self, phase: f32) -> Self {
        self.rock_at = Some(phase);
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
        explorer.set_clustering(self.clustering);
        explorer.set_lens(self.lens);
        if let Some(tile) = self
            .focus
            .take()
            .and_then(|unit| explorer.map().tile_of(&unit))
        {
            explorer.open_at(tile);
        }
        explorer
    }

    /// Opens the window and blocks until it is closed.
    #[must_use]
    pub fn run(mut self) -> AppExit {
        let focused = self.focus.is_some();
        let mut explorer = self.explorer();
        let diff = self.diff.take().map(|comparison| {
            let diff =
                Diff::between(&comparison.before, explorer.system()).with_moves(comparison.moved);
            let mut view = DiffView::new(&mut explorer, &diff, comparison.before);
            view.style = comparison.style;
            if let Some(t) = comparison.morph {
                view.morph = t;
                view.playing = false;
            }
            explorer.set_changed_tiles(view.changed().map(|(tile, _)| tile).collect());
            // A diff opens on what changed alone, around the changes.
            explorer.set_lens(Lens {
                source: if self.lens_source {
                    explorer.lens().source
                } else {
                    Source::Changes
                },
                changes_only: self.changes_only,
                ..explorer.lens()
            });
            if !focused && let Some(tile) = view.hub() {
                explorer.open_at(tile);
            }
            explorer.refresh_reach();
            view
        });
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
            MeshPickingPlugin,
            EguiPlugin::default(),
            camera::OrbitCameraPlugin,
            scene::ScenePlugin,
            ui::UiPlugin,
            diff_view::DiffPlugin,
            floors::FloorsPlugin,
            cues::CuesPlugin,
        ))
        .insert_resource(ClearColor(Color::srgb(0.06, 0.07, 0.09)))
        .insert_resource(explorer)
        .insert_resource(JourneyPanel::new(self.panel))
        .insert_resource(ui::Chrome {
            bare: self.bare,
            time: self.time,
            ..default()
        })
        .insert_resource(cues::Rock {
            held_at: self.rock_at,
            ..default()
        })
        .init_resource::<Autoplay>();
        if let Some(diff) = diff {
            app.insert_resource(diff);
        }
        if let Some(path) = self.screenshot {
            app.add_plugins(capture::CapturePlugin {
                path,
                keys: self.keys,
            });
        }
        app.run()
    }
}
