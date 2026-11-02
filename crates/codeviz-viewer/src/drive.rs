//! A scripted session (`--script FILE --screenshot DIR`): runs a script's steps in
//! one window, through the input a person's hands would send. Keys go in as the
//! keyboard sends them; the pointer moves, presses and scrolls as the window reports a
//! mouse doing so, so picking, egui and the viewer's own handlers all see it, and a
//! click that lands on the wrong thing lands there. After each step the view settles
//! before the next. Each `snap` saves `NN-label.png` and `NN-label.json`, what the
//! picture shows by name and place (see [`crate::truth::scene`]) with the checks run on
//! it (see [`crate::checks`]); `session.json` logs how each step went, and
//! `checks.json` sums up the checks, the `expect` lines and the steps that failed.

use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    time::Duration,
};

use bevy::{
    camera::{ImageRenderTarget, RenderTarget},
    ecs::system::SystemParam,
    input::{
        ButtonState, InputSystems,
        keyboard::{Key, KeyboardInput, NativeKeyCode},
        mouse::{MouseButtonInput, MouseMotion, MouseScrollUnit, MouseWheel},
        touch::TouchPhase,
    },
    prelude::*,
    render::{
        render_resource::TextureFormat,
        view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    },
    window::{CursorMoved, PrimaryWindow, WindowEvent, WindowLevel, WindowOccluded},
};
use bevy_egui::{EguiContext, EguiFullOutput, EguiPostUpdateSet, PrimaryEguiContext, egui};
use codeviz_core::SpanId;
use serde_json::{Value, json};

use crate::{
    camera::OrbitCamera,
    checks,
    explorer::Explorer,
    idle::Awake,
    keys::Press,
    labels::node_name,
    scene::Tile,
    script::{
        Aim, Expect, Gesture, Kind, Line, Name, Step, close_matches, snap_label, snap_name, unknown,
    },
    ui::{Chrome, KeyHelp},
};

/// Frames to let the scene settle (assets load, camera eases) before the first step.
const SETTLE_FRAMES: u32 = 90;
/// Time to let it settle as well: on a first run shaders compile in the background,
/// and until they do the frames come quickly and draw nothing.
const SETTLE_TIME: Duration = Duration::from_secs(2);
/// Frames to wait after the last snapshot so its file write completes.
const DRAIN_FRAMES: u32 = 30;
/// Times a snapshot that comes out black is taken again.
const RETAKES: u32 = 3;
/// Frames in a row with nothing moving before a step counts as settled.
const CALM_FRAMES: u32 = 6;
/// Least time a step is given to show, so labels finish fading and sliding.
const LEAST_SETTLE: Duration = Duration::from_millis(500);
/// Longest a step waits to settle before the session moves on, saying so.
const MOST_SETTLE: Duration = Duration::from_secs(8);
/// How long the pointer rests on a function for `hover`: past the nucleus's delay.
const HOVER_REST: Duration = Duration::from_millis(400);
/// Notches of a `scroll`, one a frame.
const NOTCH: f32 = 1.0;
/// Pieces a drag is sent in.
const DRAG_PIECES: u16 = 12;
/// Least room round the spot a double click on a module or a click on empty space
/// lands on, clear of every function and name, in points.
const CLEAR_SPOT: f32 = 6.0;
/// Room enough round a click on empty space to take the spot nearest the middle.
const ROOMY_SPOT: f32 = 30.0;
/// How near the camera must stand to where it stood for `expect camera-same-as`: in
/// the world (a glide back stops a hair short), and in radians.
const SAME_PLACE: f64 = 0.05;
const SAME_ANGLE: f64 = 0.002;

pub(crate) struct DrivePlugin {
    pub(crate) dir: PathBuf,
    pub(crate) lines: Vec<Line>,
    pub(crate) names: Names,
}

/// The functions and modules a script names, as written, with their tiles and
/// clusters.
#[derive(Debug, Clone, Default)]
pub(crate) struct Names {
    units: HashMap<String, usize>,
    modules: HashMap<String, usize>,
}

impl Names {
    fn unit(&self, name: &str) -> Result<usize, String> {
        self.units
            .get(name)
            .copied()
            .ok_or_else(|| format!("`{name}` was not looked up"))
    }

    fn module(&self, name: &str) -> Result<usize, String> {
        self.modules
            .get(name)
            .copied()
            .ok_or_else(|| format!("`{name}` was not looked up"))
    }
}

/// Looks up every function and module a script names, before the window opens.
///
/// # Errors
///
/// Names each line naming nothing known, or more than one function, with the names
/// nearest it.
pub(crate) fn resolve(lines: &[Line], explorer: &Explorer) -> Result<Names, String> {
    let mut names = Names::default();
    let mut errors = Vec::new();
    for line in lines {
        let found = match line.step.names() {
            None => continue,
            Some(Name::Unit(query)) => find_unit(explorer, query).map(|tile| {
                names.units.insert(query.to_owned(), tile);
            }),
            Some(Name::Module(query)) => find_module(explorer, query).map(|cluster| {
                names.modules.insert(query.to_owned(), cluster);
            }),
        };
        if let Err(error) = found {
            errors.push(format!("line {}: {error}: `{}`", line.number, line.text));
        }
    }
    if errors.is_empty() {
        Ok(names)
    } else {
        Err(errors.join("\n"))
    }
}

/// A function as `--focus` finds it: by entry label, or by its id or the end of it.
fn find_unit(explorer: &Explorer, query: &str) -> Result<usize, String> {
    let system = explorer.system();
    match system.find(query).as_slice() {
        [unit] => explorer
            .map()
            .tile_of(&unit.id)
            .ok_or_else(|| format!("{} is not in the graph", unit.id)),
        [] => {
            let known = system
                .units()
                .map(|u| u.id.as_str())
                .chain(system.entries().iter().map(|e| e.label.as_str()));
            Err(unknown("function", query, known))
        }
        many => {
            let ids: Vec<&str> = many.iter().take(6).map(|u| u.id.as_str()).collect();
            Err(format!(
                "`{query}` names {} functions, as {}; give more of the path",
                many.len(),
                ids.join(", ")
            ))
        }
    }
}

/// A module by the name its sphere carries, in any case.
fn find_module(explorer: &Explorer, query: &str) -> Result<usize, String> {
    let names: Vec<String> = (0..explorer.graph().cluster_count())
        .map(|cluster| explorer.cluster_name(cluster))
        .collect();
    names
        .iter()
        .position(|name| name == query)
        .or_else(|| {
            let mut alike = names
                .iter()
                .enumerate()
                .filter(|(_, name)| name.eq_ignore_ascii_case(query));
            match (alike.next(), alike.next()) {
                (Some((cluster, _)), None) => Some(cluster),
                _ => None,
            }
        })
        .ok_or_else(|| {
            let error = unknown("module", query, names.iter().map(String::as_str));
            if error.contains("did you mean") || names.len() > MODULES_LISTED {
                error
            } else {
                format!("{error}; modules: {}", names.join(", "))
            }
        })
}

/// Most modules an unknown module's error lists.
const MODULES_LISTED: usize = 40;

impl Plugin for DrivePlugin {
    fn build(&self, app: &mut App) {
        if let Err(error) = std::fs::create_dir_all(&self.dir) {
            error!("could not make {}: {error}", self.dir.display());
        }
        app.insert_resource(Drive {
            dir: self.dir.clone(),
            lines: self.lines.clone(),
            names: self.names.clone(),
            ..Drive::default()
        })
        .init_resource::<Screen>()
        .add_systems(Startup, stay_on_top)
        // Input goes in with the window's own, so the frame's systems see it.
        .add_systems(PreUpdate, act.before(InputSystems))
        .add_systems(
            PostUpdate,
            (record, advance, let_go_of_cursor)
                .chain()
                .after(EguiPostUpdateSet::EndPass)
                .before(EguiPostUpdateSet::ProcessOutput)
                .after(TransformSystems::Propagate),
        );
    }
}

/// Keeps the window over the others: once covered, macOS stops drawing it, and
/// snapshots come out black.
fn stay_on_top(mut window: Single<&mut Window, With<PrimaryWindow>>) {
    window.window_level = WindowLevel::AlwaysOnTop;
}

/// What the hands do next, a frame at a time.
#[derive(Debug, Clone)]
enum Act {
    /// The pointer moves here, in window points.
    Pointer(Vec2),
    /// The pointer moves by this much with the button held: a piece of a drag.
    Nudge(Vec2),
    Mouse(MouseButton, ButtonState),
    /// A wheel notch across and down, nearer when `y` is positive.
    Wheel(Vec2),
    Key(Press, ButtonState),
    /// A character typed, as into the search box.
    Text(char, ButtonState),
    /// The window takes this size, in points.
    Resize(Vec2),
    Shift(ButtonState),
    /// Lets this many frames pass.
    Frames(u32),
    /// Lets this much time pass, then from when it is first reached.
    Pause(Duration),
    Until(Duration),
    /// Notes whether the pointer is over this function, by tile, as it should be.
    Expect(usize),
    /// Notes whether the pointer is over no function and no overlay, as it should be.
    ExpectClear,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Phase {
    /// Letting the scene settle before the first step.
    #[default]
    Opening,
    /// Sending the step's input.
    Acting,
    /// Waiting for the view to come to rest, since this frame and time.
    Settling(u32, Duration),
    /// Waiting for a snapshot of the window to be taken, asked for on this frame.
    Capturing(u32),
    /// About to draw into an image instead of the window.
    Offscreen,
    /// Drawing into an image instead of the window, since this frame, for a snapshot
    /// the window could not give: macOS stops drawing a window it counts as covered.
    Retargeted(u32),
    /// Waiting for that snapshot, asked for on this frame.
    CapturingOffscreen(u32),
    /// Waiting for the last snapshot to reach the disk, since this frame.
    Closing(u32),
    Done,
}

#[derive(Resource, Default)]
struct Drive {
    dir: PathBuf,
    lines: Vec<Line>,
    names: Names,
    /// The step under way, by index.
    step: Option<usize>,
    phase: Phase,
    acts: VecDeque<Act>,
    /// Where the pointer is, in window points, once a step has moved it.
    pointer: Option<Vec2>,
    /// Where the window had its cursor before the step's pointer stood in for it.
    real: Option<Vec2>,
    /// What went wrong with the step under way.
    trouble: Vec<String>,
    log: Vec<Value>,
    snaps: Vec<String>,
    frame: u32,
    started: Option<Duration>,
    /// Frames in a row with nothing moving.
    calm: u32,
    /// What the view showed last frame, to tell whether anything changed.
    last_seen: String,
    /// Whether the snapshot just taken came out black, and how often it has.
    black: bool,
    retakes: u32,
    /// Whether macOS counts the window as covered, so it draws nothing into it.
    covered: bool,
    /// The image snapshots are drawn into when the window will not show them, and
    /// where each camera drew before.
    offscreen: Option<Handle<Image>>,
    targets: Vec<(Entity, RenderTarget)>,
    /// What the last snap showed, to say what moved since, and what each step since
    /// may move.
    last_scene: Option<Value>,
    kinds: Vec<Kind>,
    /// Each snap's label, as its file names it, with where the camera stood.
    cameras: Vec<(String, Value)>,
    /// Each snap's checks, by its file stem.
    checked: Vec<(String, Value)>,
    /// How each `expect` line went.
    expects: Vec<Value>,
}

/// The messages a window sends for a mouse and a keyboard.
#[derive(SystemParam)]
struct Hands<'w> {
    keys: MessageWriter<'w, KeyboardInput>,
    moves: MessageWriter<'w, CursorMoved>,
    buttons: MessageWriter<'w, MouseButtonInput>,
    wheels: MessageWriter<'w, MouseWheel>,
    motions: MessageWriter<'w, MouseMotion>,
    events: MessageWriter<'w, WindowEvent>,
}

impl Hands<'_> {
    fn move_to(&mut self, window: Entity, from: Option<Vec2>, to: Vec2) {
        let message = CursorMoved {
            window,
            position: to,
            delta: from.map(|from| to - from),
        };
        self.moves.write(message.clone());
        self.events.write(message.into());
    }

    fn mouse(&mut self, window: Entity, button: MouseButton, state: ButtonState) {
        let message = MouseButtonInput {
            button,
            state,
            window,
        };
        self.buttons.write(message);
        self.events.write(message.into());
    }

    fn wheel(&mut self, window: Entity, notch: Vec2) {
        let message = MouseWheel {
            unit: MouseScrollUnit::Line,
            x: notch.x,
            y: notch.y,
            window,
            phase: TouchPhase::Moved,
        };
        self.wheels.write(message);
        self.events.write(message.into());
    }

    fn motion(&mut self, delta: Vec2) {
        self.motions.write(MouseMotion { delta });
        self.events.write(MouseMotion { delta }.into());
    }

    /// A character as a keyboard layout without its key would send it: no key code a
    /// shortcut answers to, only the text.
    fn text(&mut self, window: Entity, c: char, state: ButtonState) {
        let text = c.to_string();
        self.keys.write(KeyboardInput {
            key_code: KeyCode::Unidentified(NativeKeyCode::Unidentified),
            logical_key: Key::Character(text.as_str().into()),
            state,
            text: (state == ButtonState::Pressed).then(|| text.as_str().into()),
            repeat: false,
            window,
        });
    }

    fn shift(&mut self, window: Entity, state: ButtonState) {
        self.keys.write(KeyboardInput {
            key_code: KeyCode::ShiftLeft,
            logical_key: Key::Shift,
            state,
            text: None,
            repeat: false,
            window,
        });
    }
}

/// The pointer goes where the step's hands have it; keys, buttons and the wheel go in
/// as the window would send them.
fn act(
    mut drive: ResMut<Drive>,
    mut window: Single<(Entity, &mut Window), With<PrimaryWindow>>,
    mut hands: Hands,
    explorer: Res<Explorer>,
    screen: Res<Screen>,
    time: Res<Time<Real>>,
) {
    let (entity, window) = &mut *window;
    let entity = *entity;
    // Systems that read the cursor from the window see the step's pointer; it is put
    // back after the frame, so the window never moves the real one.
    drive.real = window.cursor_position();
    if let Some(at) = drive.pointer {
        window
            .bypass_change_detection()
            .set_cursor_position(Some(at));
    }
    while let Some(next) = drive.acts.pop_front() {
        match next {
            Act::Pointer(to) => {
                hands.move_to(entity, drive.pointer, to);
                drive.pointer = Some(to);
                window
                    .bypass_change_detection()
                    .set_cursor_position(Some(to));
            }
            Act::Nudge(by) => {
                let from = drive.pointer.unwrap_or_default();
                hands.motion(by);
                hands.move_to(entity, Some(from), from + by);
                drive.pointer = Some(from + by);
                window
                    .bypass_change_detection()
                    .set_cursor_position(Some(from + by));
                drive.acts.push_front(Act::Frames(1));
            }
            Act::Mouse(button, state) => hands.mouse(entity, button, state),
            Act::Wheel(notch) => hands.wheel(entity, notch),
            Act::Key(press, state) => {
                hands.keys.write_batch(press.messages(state, entity));
            }
            Act::Text(c, state) => hands.text(entity, c, state),
            Act::Resize(size) => window.resolution.set(size.x, size.y),
            Act::Shift(state) => hands.shift(entity, state),
            Act::Frames(0) => {}
            Act::Frames(n) => {
                drive.acts.push_front(Act::Frames(n - 1));
                return;
            }
            Act::Pause(span) => {
                drive.acts.push_front(Act::Until(time.elapsed() + span));
                return;
            }
            Act::Until(at) => {
                if time.elapsed() < at {
                    drive.acts.push_front(Act::Until(at));
                    return;
                }
            }
            Act::Expect(wanted) => {
                if explorer.under_pointer != Some(wanted) {
                    let at = drive.pointer.unwrap_or_default();
                    let over = match (explorer.under_pointer, screen.overlay_at(at)) {
                        (Some(other), _) => node_name(&explorer, other),
                        (None, Some(overlay)) => format!("the {overlay}"),
                        (None, None) => "nothing".to_owned(),
                    };
                    let trouble = format!(
                        "the pointer at ({:.0}, {:.0}) is over {over}, not {}",
                        at.x,
                        at.y,
                        node_name(&explorer, wanted)
                    );
                    drive.trouble.push(trouble);
                }
            }
            Act::ExpectClear => {
                let at = drive.pointer.unwrap_or_default();
                let over = match (explorer.under_pointer, screen.overlay_at(at)) {
                    (Some(other), _) => Some(node_name(&explorer, other)),
                    (None, Some(overlay)) => Some(format!("the {overlay}")),
                    (None, None) => None,
                };
                if let Some(over) = over {
                    let trouble = format!(
                        "the pointer at ({:.0}, {:.0}) is over {over}, not a clear spot",
                        at.x, at.y
                    );
                    drive.trouble.push(trouble);
                }
            }
        }
    }
}

/// Puts the window's cursor back where the real one is, before the window would act
/// on the change and move it.
fn let_go_of_cursor(drive: Res<Drive>, mut window: Single<&mut Window, With<PrimaryWindow>>) {
    if drive.pointer.is_some() {
        window
            .bypass_change_detection()
            .set_cursor_position(drive.real);
    }
}

/// A piece of text egui drew, where, and how faintly.
#[derive(Debug, Clone)]
pub(crate) struct Painted {
    pub(crate) text: String,
    pub(crate) rect: egui::Rect,
    /// What it is clipped to.
    pub(crate) clip: egui::Rect,
    pub(crate) opacity: f32,
    /// Its font size, in points: its largest, where its parts differ.
    pub(crate) size: f32,
}

/// Something that takes a click, and what it says.
#[derive(Debug, Clone)]
pub(crate) struct Widget {
    pub(crate) text: String,
    /// Where it takes a click: the part of it not clipped away.
    pub(crate) rect: egui::Rect,
    /// All of it, as laid out.
    pub(crate) full: egui::Rect,
    pub(crate) enabled: bool,
}

impl Widget {
    /// Whether some of it is clipped away, as by the window's edge.
    pub(crate) fn cut_off(&self) -> bool {
        self.rect.area() + 1.0 < self.full.area()
    }
}

/// The buttons among clickable widgets, each laid out at `.0`, clickable over `.1`,
/// enabled or not, with the text drawn on it. An area takes clicks over all of
/// itself, so one holding another is no button.
pub(crate) fn buttons(
    found: Vec<(egui::Rect, egui::Rect, bool)>,
    texts: &[Painted],
) -> Vec<Widget> {
    let holds = |i: usize| {
        found
            .iter()
            .enumerate()
            .any(|(j, other)| j != i && found[i].0.contains_rect(other.0))
    };
    found
        .iter()
        .enumerate()
        .filter(|&(i, _)| !holds(i))
        .filter_map(|(_, &(full, rect, enabled))| {
            let text = texts
                .iter()
                .filter(|t| full.contains(t.rect.center()))
                .map(|t| t.text.trim())
                .collect::<Vec<_>>()
                .join(" ");
            (!text.is_empty()).then_some(Widget {
                text,
                rect,
                full,
                enabled,
            })
        })
        .collect()
}

/// Words past which a button's text is no label to find it by part of.
const BUTTON_WORDS: usize = 8;

/// The button that says `text`: the one that says exactly that, else the first short
/// one that says it among other words, ignoring case.
fn pick<'a>(widgets: &'a [Widget], text: &str) -> Result<&'a Widget, String> {
    let words = |s: &str| {
        s.split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase()
    };
    let wanted = words(text);
    widgets
        .iter()
        .find(|w| words(&w.text) == wanted)
        .or_else(|| {
            widgets.iter().find(|w| {
                w.text.split_whitespace().count() <= BUTTON_WORDS
                    && words(&w.text).contains(&wanted)
            })
        })
        .ok_or_else(|| {
            let all: Vec<&str> = widgets.iter().map(|w| w.text.as_str()).collect();
            let near = close_matches(text, all.iter().copied());
            let meant = if near.is_empty() {
                String::new()
            } else {
                format!(" (did you mean {}?)", near.join(", "))
            };
            format!(
                "no button says `{text}`{meant}; showing: {}",
                all.join(" | ")
            )
        })
}

/// What egui drew this frame, read once its pass ends: every piece of text, every
/// clickable widget with what it says, the journey's bars and the overlays on top.
#[derive(Resource, Debug)]
pub(crate) struct Screen {
    pub(crate) window: egui::Vec2,
    /// Where the graph is drawn: the 3D camera's viewport.
    pub(crate) graph: egui::Rect,
    pub(crate) texts: Vec<Painted>,
    pub(crate) widgets: Vec<Widget>,
    pub(crate) bars: Option<DrawnBars>,
    /// Areas over the graph and panels, by name, with where they are.
    pub(crate) overlays: Vec<(String, egui::Rect)>,
    /// Each function's name on the graph, a click on which picks it: its pill, and the
    /// function, bottom first.
    pub(crate) labels: Vec<(egui::Rect, usize)>,
}

impl Default for Screen {
    fn default() -> Self {
        Self {
            window: egui::Vec2::ZERO,
            graph: egui::Rect::NOTHING,
            texts: Vec::new(),
            widgets: Vec::new(),
            bars: None,
            overlays: Vec::new(),
            labels: Vec::new(),
        }
    }
}

impl Screen {
    /// The overlay a point falls on, top first.
    pub(crate) fn overlay_at(&self, at: Vec2) -> Option<&str> {
        let at = egui::pos2(at.x, at.y);
        self.overlays
            .iter()
            .rev()
            .find(|(_, rect)| rect.contains(at))
            .map(|(name, _)| name.as_str())
    }
}

/// The journey's bars as last drawn, kept in egui's memory for a session to find.
#[derive(Debug, Clone)]
pub(crate) struct DrawnBars {
    /// The egui pass they were drawn in, so bars no longer drawn are not taken.
    pass: u64,
    /// Where the bars show; one outside it is scrolled out of view.
    pub(crate) content: egui::Rect,
    pub(crate) bars: Vec<(SpanId, egui::Rect)>,
}

fn bars_id() -> egui::Id {
    egui::Id::new("drawn_journey_bars")
}

/// Notes where the journey panel drew its bars this pass.
pub(crate) fn note_bars(ctx: &egui::Context, content: egui::Rect, bars: Vec<(SpanId, egui::Rect)>) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        d.insert_temp(
            bars_id(),
            DrawnBars {
                pass,
                content,
                bars,
            },
        );
    });
}

/// egui areas that sit over the graph, by id.
const OVERLAYS: [(&str, &str); 3] = [
    ("journey_panel", "journey panel"),
    ("key_help", "key help (H)"),
    ("search", "search box"),
];

fn record(
    mut contexts: Query<(&mut EguiContext, &EguiFullOutput), With<PrimaryEguiContext>>,
    cameras: Query<&Camera, With<OrbitCamera>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut screen: ResMut<Screen>,
) {
    let Ok((mut context, output)) = contexts.single_mut() else {
        return;
    };
    let Some(output) = &output.0 else {
        return;
    };
    let ctx = context.get_mut().clone();
    let mut texts = Vec::new();
    for clipped in &output.shapes {
        collect_texts(&clipped.shape, clipped.clip_rect, &mut texts);
    }
    let widgets = ctx.viewport(|v| v.prev_pass.widgets.clone());
    let found = widgets
        .layers()
        .flat_map(|(_, rects)| rects)
        .filter(|w| w.sense.senses_click() && !w.sense.senses_drag())
        .map(|w| (w.rect, w.interact_rect, w.enabled))
        .collect();
    let clickable = buttons(found, &texts);
    let pass = ctx.cumulative_pass_nr();
    let bars = ctx
        .data_mut(|d| d.get_temp::<DrawnBars>(bars_id()))
        .filter(|bars| bars.pass + 1 == pass);
    let overlays = ctx.memory(|m| {
        let visible = m.areas().visible_layer_ids();
        OVERLAYS
            .iter()
            .filter(|(id, _)| visible.iter().any(|l| l.id == egui::Id::new(*id)))
            .filter_map(|&(id, name)| Some((name.to_owned(), m.area_rect(egui::Id::new(id))?)))
            .collect()
    });
    let graph = cameras
        .single()
        .ok()
        .and_then(Camera::logical_viewport_rect)
        .map_or(egui::Rect::NOTHING, |r| {
            egui::Rect::from_min_max(egui::pos2(r.min.x, r.min.y), egui::pos2(r.max.x, r.max.y))
        });
    *screen = Screen {
        window: egui::vec2(window.width(), window.height()),
        graph,
        texts,
        widgets: clickable,
        bars,
        overlays,
        labels: crate::labels::drawn_labels(&ctx),
    };
}

fn collect_texts(shape: &egui::Shape, clip: egui::Rect, out: &mut Vec<Painted>) {
    match shape {
        egui::Shape::Text(text) if !text.galley.text().trim().is_empty() => out.push(Painted {
            text: words_of(&text.galley.job),
            rect: text.galley.rect.translate(text.pos.to_vec2()),
            clip,
            opacity: text.opacity_factor,
            size: text
                .galley
                .job
                .sections
                .iter()
                .map(|section| section.format.font_id.size)
                .fold(0.0, f32::max),
        }),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_texts(shape, clip, out);
            }
        }
        _ => {}
    }
}

/// A layout's text as it reads, with a space where a section is set apart from the
/// one before by a gap rather than a character.
fn words_of(job: &egui::text::LayoutJob) -> String {
    let mut text = String::new();
    for section in &job.sections {
        if section.leading_space > 0.0 && !text.is_empty() && !text.ends_with(' ') {
            text.push(' ');
        }
        let range = &section.byte_range;
        text.push_str(&job.text[range.start.0..range.end.0]);
    }
    text
}

/// A function as drawn: by tile, where it sits on screen and how big it shows.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Drawn {
    pub(crate) tile: usize,
    pub(crate) at: Vec2,
    /// Where it is in the world, and how big it is there.
    pub(crate) world: Vec3,
    pub(crate) world_radius: f32,
    pub(crate) radius: f32,
    /// Whether it is inside the graph's part of the window, in front of the camera.
    pub(crate) on_screen: bool,
}

/// Everything a snapshot reads.
#[derive(SystemParam)]
pub(crate) struct View<'w, 's> {
    pub(crate) explorer: Res<'w, Explorer>,
    pub(crate) help: Res<'w, KeyHelp>,
    pub(crate) chrome: Res<'w, Chrome>,
    pub(crate) screen: Res<'w, Screen>,
    tiles: Query<
        'w,
        's,
        (
            &'static Tile,
            &'static GlobalTransform,
            &'static InheritedVisibility,
        ),
    >,
    cameras: Query<
        'w,
        's,
        (
            &'static Camera,
            &'static GlobalTransform,
            &'static OrbitCamera,
        ),
    >,
}

impl View<'_, '_> {
    /// Where a point in the world shows in the window, when in front of the camera.
    pub(crate) fn to_screen(&self, world: Vec3) -> Option<Vec2> {
        let (camera, transform, _) = self.cameras.single().ok()?;
        camera.world_to_viewport(transform, world).ok()
    }

    /// How big a sphere of `radius` at `world` shows, in points.
    pub(crate) fn screen_radius(&self, world: Vec3, radius: f32) -> f32 {
        let Ok((_, transform, _)) = self.cameras.single() else {
            return 0.0;
        };
        let up = transform.up();
        self.to_screen(world)
            .zip(self.to_screen(world + up * radius))
            .map_or(0.0, |(a, b)| a.distance(b))
    }

    pub(crate) fn on_screen(&self, at: Vec2) -> bool {
        self.screen.graph.contains(egui::pos2(at.x, at.y))
    }

    /// Every function drawn, in tile order.
    pub(crate) fn drawn(&self) -> Vec<Drawn> {
        let mut drawn: Vec<Drawn> = self
            .tiles
            .iter()
            .filter(|(_, _, visible)| visible.get())
            .filter_map(|(tile, transform, _)| {
                let world = transform.translation();
                let at = self.to_screen(world)?;
                let world_radius = transform.scale().x * 0.5;
                Some(Drawn {
                    tile: tile.0,
                    at,
                    world,
                    world_radius,
                    radius: self.screen_radius(world, world_radius),
                    on_screen: self.on_screen(at),
                })
            })
            .collect();
        drawn.sort_by_key(|d| d.tile);
        drawn
    }

    /// One function as drawn, or why it is not.
    fn find(&self, tile: usize) -> Result<Drawn, String> {
        let name = || node_name(&self.explorer, tile);
        let Some((_, transform, visible)) = self.tiles.iter().find(|(t, ..)| t.0 == tile) else {
            return Err(format!("{} has no node", name()));
        };
        if !visible.get() {
            let module = self
                .explorer
                .graph()
                .cluster(tile)
                .map(|c| self.explorer.cluster_name(c))
                .unwrap_or_default();
            return Err(format!(
                "{} is not drawn (folded into module {module}, or a hidden speck)",
                name()
            ));
        }
        let world = transform.translation();
        let Some(at) = self.to_screen(world) else {
            return Err(format!("{} is behind the camera", name()));
        };
        if !self.on_screen(at) {
            return Err(format!(
                "{} is drawn off the graph, at ({:.0}, {:.0})",
                name(),
                at.x,
                at.y
            ));
        }
        let world_radius = transform.scale().x * 0.5;
        Ok(Drawn {
            tile,
            at,
            world,
            world_radius,
            radius: self.screen_radius(world, world_radius),
            on_screen: true,
        })
    }

    /// Where the camera is and which way it faces.
    pub(crate) fn camera(&self) -> Option<(Vec3, Quat)> {
        let (_, transform, _) = self.cameras.single().ok()?;
        Some((transform.translation(), transform.rotation()))
    }

    /// Where the camera stands as the orbit has it: what it looks at, from how far,
    /// at what yaw and pitch.
    pub(crate) fn orbit_pose(&self) -> Option<crate::camera::Pose> {
        self.cameras.iter().next().map(|(_, _, orbit)| orbit.pose())
    }

    fn camera_easing(&self) -> bool {
        self.cameras.iter().any(|(_, _, orbit)| orbit.easing())
    }

    /// What would change on screen were the view to move, to tell when it rests.
    fn seen(&self) -> String {
        let e = &self.explorer;
        format!(
            "{:?}",
            (
                e.cursor_span(),
                e.under_pointer,
                e.hovered(),
                e.disclose(),
                e.aside(),
                e.nucleus().start(),
                e.revision(),
                e.moved(),
                e.aim(),
                e.nucleus().expanded(),
                self.help.0,
            )
        )
    }
}

/// Where the cameras draw: the window, or for a snapshot the window will not give, an
/// image the window's size.
#[derive(SystemParam)]
struct Canvas<'w, 's> {
    cameras: Query<'w, 's, (Entity, &'static mut RenderTarget), With<Camera>>,
    images: ResMut<'w, Assets<Image>>,
    window: Single<'w, 's, &'static Window, With<PrimaryWindow>>,
}

impl Canvas<'_, '_> {
    /// Points every camera at an image the window's size, at its scale, so the
    /// graph and egui lay out as on the window.
    fn draw_offscreen(&mut self, drive: &mut Drive) {
        let (width, height) = (self.window.physical_width(), self.window.physical_height());
        let fits = drive
            .offscreen
            .as_ref()
            .and_then(|handle| self.images.get(handle))
            .is_some_and(|image| image.width() == width && image.height() == height);
        if !fits {
            let image =
                Image::new_target_texture(width, height, TextureFormat::Rgba8UnormSrgb, None);
            drive.offscreen = Some(self.images.add(image));
        }
        let Some(handle) = drive.offscreen.clone() else {
            return;
        };
        let target = RenderTarget::Image(ImageRenderTarget {
            handle,
            scale_factor: self.window.scale_factor(),
        });
        if drive.targets.is_empty() {
            drive.targets = self
                .cameras
                .iter()
                .map(|(entity, target)| (entity, target.clone()))
                .collect();
        }
        for (_, mut camera) in &mut self.cameras {
            *camera = target.clone();
        }
    }

    /// Points every camera back where it drew before.
    fn draw_on_window(&mut self, drive: &mut Drive) {
        for (entity, target) in drive.targets.drain(..) {
            if let Ok((_, mut camera)) = self.cameras.get_mut(entity) {
                *camera = target;
            }
        }
    }
}

/// Runs the script: lets each step's input play out, waits for the view to rest,
/// takes snapshots, and exits once the last is on disk.
#[expect(clippy::too_many_arguments, reason = "a Bevy system")]
fn advance(
    mut commands: Commands,
    mut drive: ResMut<Drive>,
    view: View,
    mut awake: ResMut<Awake>,
    shots: Query<(), With<Screenshot>>,
    time: Res<Time<Real>>,
    mut exit: MessageWriter<AppExit>,
    mut covered: MessageReader<WindowOccluded>,
    mut canvas: Canvas,
) {
    drive.frame += 1;
    if let Some(message) = covered.read().last() {
        drive.covered = message.occluded;
        info!(
            "session: window covered: {} (frame {})",
            message.occluded, drive.frame
        );
    }
    let now = time.elapsed();
    let started = *drive.started.get_or_insert(now);
    let seen = view.seen();
    let moving = std::mem::take(&mut awake.0) || view.camera_easing() || seen != drive.last_seen;
    drive.last_seen = seen;
    drive.calm = if moving { 0 } else { drive.calm + 1 };
    match drive.phase {
        Phase::Opening => {
            if drive.frame >= SETTLE_FRAMES
                && now.saturating_sub(started) >= SETTLE_TIME
                && drive.calm >= CALM_FRAMES
            {
                let entry = json!({ "step": 0, "do": "open", "ok": true, "then": crate::truth::state(&view) });
                info!("session: {entry}");
                drive.log.push(entry);
                begin_next(&mut commands, &mut drive, &view, now);
            }
        }
        Phase::Acting => {
            if drive.acts.is_empty() {
                drive.phase = Phase::Settling(drive.frame, now);
            }
        }
        Phase::Settling(frame, since) => {
            let waited = now.saturating_sub(since);
            let rested = drive.calm >= CALM_FRAMES
                && drive.frame >= frame + CALM_FRAMES
                && waited >= LEAST_SETTLE;
            if rested || waited >= MOST_SETTLE {
                if !rested {
                    let trouble = format!(
                        "still moving after {:.0}s; went on anyway",
                        MOST_SETTLE.as_secs_f32()
                    );
                    drive.trouble.push(trouble);
                }
                finish(&mut drive, &view, Some(waited));
                begin_next(&mut commands, &mut drive, &view, now);
            }
        }
        Phase::Capturing(_)
        | Phase::Offscreen
        | Phase::Retargeted(_)
        | Phase::CapturingOffscreen(_) => {
            if photograph(&mut commands, &mut drive, &mut canvas, shots.is_empty()) {
                finish(&mut drive, &view, None);
                begin_next(&mut commands, &mut drive, &view, now);
            }
        }
        Phase::Closing(at) => {
            if shots.is_empty() && drive.frame >= at + DRAIN_FRAMES {
                let session = json!({ "snaps": drive.snaps, "steps": drive.log });
                write(&drive.dir.join("session.json"), &session);
                write(&drive.dir.join("checks.json"), &summary(&drive));
                drive.phase = Phase::Done;
                exit.write(AppExit::Success);
            }
        }
        Phase::Done => {}
    }
}

/// Moves a snapshot along: from the window, else, when the window is covered or
/// comes out black, drawn into an image, taken again while that comes out black.
/// Returns whether it is done.
fn photograph(
    commands: &mut Commands,
    drive: &mut Drive,
    canvas: &mut Canvas,
    taken: bool,
) -> bool {
    match drive.phase {
        Phase::Capturing(at) if taken && drive.frame >= at + 2 => {
            if !std::mem::take(&mut drive.black) {
                return true;
            }
            let trouble = "the window came out black (macOS stopped drawing it); \
                           drew the snapshot offscreen";
            drive.trouble.push(trouble.to_owned());
            canvas.draw_offscreen(drive);
            drive.phase = Phase::Retargeted(drive.frame);
        }
        Phase::Offscreen => {
            canvas.draw_offscreen(drive);
            drive.phase = Phase::Retargeted(drive.frame);
        }
        // A few frames for the cameras and egui to take the image's size.
        Phase::Retargeted(at) if drive.frame >= at + 3 => {
            if let (Some(image), Some(name)) = (drive.offscreen.clone(), drive.snaps.last()) {
                let name = name.clone();
                shoot(commands, Screenshot::image(image), &drive.dir, &name);
                drive.phase = Phase::CapturingOffscreen(drive.frame);
            }
        }
        Phase::CapturingOffscreen(at) if taken && drive.frame >= at + 2 => {
            if std::mem::take(&mut drive.black) {
                if drive.retakes < RETAKES {
                    drive.retakes += 1;
                    drive.phase = Phase::Retargeted(drive.frame);
                    return false;
                }
                let trouble = format!("still black after {RETAKES} offscreen retakes");
                drive.trouble.push(trouble);
            }
            drive.retakes = 0;
            canvas.draw_on_window(drive);
            return true;
        }
        _ => {}
    }
    false
}

/// Logs how the step under way went.
fn finish(drive: &mut Drive, view: &View, waited: Option<Duration>) {
    let Some(index) = drive.step else {
        return;
    };
    let trouble = std::mem::take(&mut drive.trouble);
    let mut entry = json!({
        "step": index + 1,
        "line": drive.lines[index].number,
        "do": drive.lines[index].text,
        "ok": trouble.is_empty(),
        "then": crate::truth::state(view),
    });
    if !trouble.is_empty() {
        entry["trouble"] = json!(trouble);
    }
    if let Some(waited) = waited {
        entry["settled_ms"] = json!(waited.as_millis());
    }
    info!("session: {entry}");
    drive.log.push(entry);
}

/// Starts the next step that has anything to do, logging those that fail before they
/// start; closes the session after the last.
fn begin_next(commands: &mut Commands, drive: &mut Drive, view: &View, now: Duration) {
    loop {
        let index = drive.step.map_or(0, |i| i + 1);
        drive.step = Some(index);
        let Some(step) = drive.lines.get(index).map(|line| line.step.clone()) else {
            drive.phase = Phase::Closing(drive.frame);
            return;
        };
        if let Step::Snap(label) = &step {
            snap(commands, drive, view, label.as_deref());
            return;
        }
        drive.kinds.push(step.kind());
        match plan(drive, view, &step) {
            Ok(acts) => {
                drive.acts = acts.into();
                drive.phase = if drive.acts.is_empty() {
                    Phase::Settling(drive.frame, now)
                } else {
                    Phase::Acting
                };
                return;
            }
            Err(trouble) => {
                drive.trouble.push(trouble);
                finish(drive, view, None);
            }
        }
    }
}

/// What the view shows now, as a snap's JSON has it, with where the pointer is.
fn scene_now(drive: &Drive, view: &View) -> Value {
    let mut scene = crate::truth::scene(view);
    scene["state"]["pointer"] = json!(drive.pointer.map(|at| [at.x.round(), at.y.round()]));
    scene
}

/// Saves the window as it shows next, and what it shows beside it, with the checks
/// run on it.
fn snap(commands: &mut Commands, drive: &mut Drive, view: &View, label: Option<&str>) {
    let name = snap_name(drive.snaps.len() + 1, label);
    let mut scene = scene_now(drive, view);
    scene["snap"] = json!(name);
    if let Some((since, before)) = drive.snaps.last().zip(drive.last_scene.as_ref()) {
        scene["moved"] = checks::moved(before, &scene, since);
    }
    let checked = checks::run(&scene, drive.last_scene.as_ref(), &drive.kinds);
    drive.kinds.clear();
    drive.checked.push((name.clone(), checked.clone()));
    drive
        .cameras
        .push((snap_label(label), scene["state"]["camera"].clone()));
    scene["checks"] = checked;
    let mut written = scene.clone();
    drive.last_scene = Some(scene);
    written["log"] = json!(drive.log);
    write(&drive.dir.join(format!("{name}.json")), &written);
    if drive.covered {
        // The window draws nothing while covered; the next frames draw offscreen.
        drive.snaps.push(name);
        drive.phase = Phase::Offscreen;
        return;
    }
    shoot(commands, Screenshot::primary_window(), &drive.dir, &name);
    drive.snaps.push(name);
    drive.phase = Phase::Capturing(drive.frame);
}

/// Takes the window, or the image, as it shows next, saving it unless it comes out
/// black, which a window macOS has stopped drawing does.
fn shoot(commands: &mut Commands, shot: Screenshot, dir: &std::path::Path, name: &str) {
    let mut save = save_to_disk(dir.join(format!("{name}.png")));
    commands.spawn(shot).observe(
        move |captured: On<ScreenshotCaptured>, mut drive: ResMut<Drive>| {
            if is_black(&captured.image) {
                drive.black = true;
            } else {
                save(captured);
            }
        },
    );
}

/// Whether an image is black throughout, judged from a sample of its pixels.
fn is_black(image: &Image) -> bool {
    let Some(data) = &image.data else {
        return true;
    };
    // Every 97th pixel, its color bytes but not its alpha.
    let (pixels, _) = data.as_chunks::<4>();
    pixels
        .iter()
        .step_by(97)
        .all(|pixel| pixel[..3].iter().all(|&byte| byte < 8))
}

fn write(path: &std::path::Path, value: &Value) {
    if let Err(error) = std::fs::write(path, format!("{value:#}\n")) {
        error!("could not write {}: {error}", path.display());
    }
}

/// The hands' moves for a step, or why it cannot be taken. An `expect` takes no
/// moves: it is judged here, on the view as the step before left it.
fn plan(drive: &mut Drive, view: &View, step: &Step) -> Result<Vec<Act>, String> {
    let key = |press| {
        vec![
            Act::Key(press, ButtonState::Pressed),
            Act::Frames(1),
            Act::Key(press, ButtonState::Released),
            Act::Frames(1),
        ]
    };
    let click = |at: Vec2| {
        let mut acts = vec![Act::Pointer(at), Act::Frames(3)];
        acts.extend(PRESS);
        acts
    };
    Ok(match step {
        Step::Key(press) => key(*press),
        Step::Type(text) => text
            .chars()
            .flat_map(|c| {
                [
                    Act::Text(c, ButtonState::Pressed),
                    Act::Frames(1),
                    Act::Text(c, ButtonState::Released),
                    Act::Frames(1),
                ]
            })
            .collect(),
        Step::Point { gesture, unit } => {
            let tile = drive.names.unit(unit)?;
            let at = if matches!(gesture, Gesture::LabelClick | Gesture::LabelHover) {
                label_point(view, tile)?
            } else {
                view.find(tile)?.at
            };
            let mut acts = vec![Act::Pointer(at), Act::Frames(3), Act::Expect(tile)];
            acts.extend(gesture_acts(*gesture));
            acts
        }
        Step::ClickAt(at) => click(*at),
        Step::ClickEmpty => {
            let at = empty_point(view)?;
            let mut acts = vec![Act::Pointer(at), Act::Frames(3), Act::ExpectClear];
            acts.extend(PRESS);
            acts
        }
        Step::DoubleClickModule(module) => {
            let at = module_point(view, drive.names.module(module)?)?;
            let mut acts = vec![Act::Pointer(at), Act::Frames(3), Act::ExpectClear];
            acts.extend([PRESS, PRESS].concat());
            acts
        }
        Step::Bar(number) => click(bar_point(view, number)?),
        Step::Button(text) => {
            let (at, note) = button_point(view, text)?;
            drive.trouble.extend(note);
            click(at)
        }
        Step::Drag(by) => {
            let start = drive
                .pointer
                .filter(|&p| view.on_screen(p) && view.screen.overlay_at(p).is_none())
                .unwrap_or_else(|| graph_middle(view));
            let piece = *by / f32::from(DRAG_PIECES);
            let mut acts = vec![
                Act::Pointer(start),
                Act::Frames(2),
                Act::Mouse(MouseButton::Left, ButtonState::Pressed),
                Act::Frames(1),
            ];
            acts.extend((0..DRAG_PIECES).map(|_| Act::Nudge(piece)));
            acts.push(Act::Mouse(MouseButton::Left, ButtonState::Released));
            acts.push(Act::Frames(1));
            acts
        }
        Step::Scroll { by, at } => {
            let to = match at {
                Some(Aim::Unit(unit)) => Some(view.find(drive.names.unit(unit)?)?.at),
                Some(Aim::At(point)) => Some(*point),
                None => drive.pointer.is_none().then(|| graph_middle(view)),
            };
            let mut acts = Vec::new();
            if let Some(to) = to {
                acts.extend([Act::Pointer(to), Act::Frames(3)]);
            }
            acts.extend(wheel_acts(*by));
            acts
        }
        Step::Window(size) => vec![Act::Resize(*size), Act::Frames(10)],
        Step::Wait(span) => vec![Act::Pause(*span)],
        Step::Expect(expect) => {
            let judged = judge(drive, view, expect);
            note_expect(drive, judged.as_ref().err());
            judged?;
            Vec::new()
        }
        Step::Snap(_) => Vec::new(),
    })
}

/// The press and release of the left button.
const PRESS: [Act; 4] = [
    Act::Mouse(MouseButton::Left, ButtonState::Pressed),
    Act::Frames(1),
    Act::Mouse(MouseButton::Left, ButtonState::Released),
    Act::Frames(1),
];

/// What the hands do on a function once the pointer is on it.
fn gesture_acts(gesture: Gesture) -> Vec<Act> {
    match gesture {
        Gesture::Hover | Gesture::LabelHover => vec![Act::Pause(HOVER_REST)],
        Gesture::Click | Gesture::LabelClick => PRESS.to_vec(),
        Gesture::ShiftClick => {
            let mut acts = vec![Act::Shift(ButtonState::Pressed), Act::Frames(1)];
            acts.extend(PRESS);
            // Held a little past the release, as a hand would: picking reads the
            // mouse a frame after the keys.
            acts.extend([Act::Frames(2), Act::Shift(ButtonState::Released)]);
            acts
        }
        Gesture::DoubleClick => [PRESS, PRESS].concat(),
    }
}

/// A wheel turn, a notch a frame, across and down together while both last.
fn wheel_acts(by: Vec2) -> Vec<Act> {
    let (across, down) = (by.x.abs(), by.y.abs());
    let mut acts = Vec::new();
    let mut turned = 0.0;
    while turned < across.max(down) {
        let notch = Vec2::new(
            if turned < across { by.x.signum() } else { 0.0 },
            if turned < down { by.y.signum() } else { 0.0 },
        );
        acts.extend([Act::Wheel(notch * NOTCH), Act::Frames(1)]);
        turned += 1.0;
    }
    acts
}

/// Notes how the `expect` line under way went.
fn note_expect(drive: &mut Drive, failed: Option<&String>) {
    let index = drive.step.unwrap_or_default();
    let line = &drive.lines[index];
    let entry = json!({
        "step": index + 1,
        "line": line.number,
        "expect": line.text,
        "ok": failed.is_none(),
        "why": failed,
    });
    drive.expects.push(entry);
}

fn graph_middle(view: &View) -> Vec2 {
    let c = view.screen.graph.center();
    Vec2::new(c.x, c.y)
}

/// Whether the view is as an `expect` line says, or how it is not.
fn judge(drive: &Drive, view: &View, expect: &Expect) -> Result<(), String> {
    let explorer = &*view.explorer;
    let name =
        |tile: Option<usize>| tile.map_or_else(|| "none".to_owned(), |t| node_name(explorer, t));
    let wanted = |unit: &Option<String>| unit.as_deref().map(|u| drive.names.unit(u)).transpose();
    let same = |what: &str, got: Option<usize>, want: Option<usize>| {
        if got == want {
            Ok(())
        } else {
            Err(format!("the {what} is {}, not {}", name(got), name(want)))
        }
    };
    match expect {
        Expect::State { field, value } => {
            let state = crate::truth::state(view);
            let got = match &state[field.as_str()] {
                Value::String(text) => text.clone(),
                Value::Null => "none".to_owned(),
                other => other.to_string(),
            };
            if got == *value {
                Ok(())
            } else {
                Err(format!("{field} is {got}, not {value}"))
            }
        }
        Expect::Selection(unit) => same("selection", explorer.selection(), wanted(unit)?),
        Expect::Cursor(unit) => same("journey's cursor", explorer.cursor(), wanted(unit)?),
        Expect::Start(unit) => same("journey's start", explorer.start(), wanted(unit)?),
        Expect::Named(unit) => {
            let tile = drive.names.unit(unit)?;
            if view.screen.labels.iter().any(|&(_, t)| t == tile) {
                Ok(())
            } else {
                Err(format!("{} has no name drawn", name(Some(tile))))
            }
        }
        Expect::Drawn(unit) => view.find(drive.names.unit(unit)?).map(|_| ()),
        Expect::CameraSameAs(label) => {
            let Some((_, then)) = drive.cameras.iter().rev().find(|(l, _)| l == label) else {
                return Err(format!("no snapshot `{label}` was taken"));
            };
            let now = &crate::truth::state(view)["camera"];
            camera_differs(then, now).map_or(Ok(()), |how| {
                Err(format!(
                    "the camera is not where it was at `{label}`: {how}"
                ))
            })
        }
        Expect::NothingMoved => {
            let Some(before) = &drive.last_scene else {
                return Err("no snapshot before to compare with".to_owned());
            };
            let result = checks::run(&scene_now(drive, view), Some(before), &[Kind::Still]);
            verdict_of("nothing-moved", &result["nothing-moved"], true)
        }
        Expect::Check { name, pass } => {
            let result = checks::run(
                &scene_now(drive, view),
                drive.last_scene.as_ref(),
                &drive.kinds,
            );
            verdict_of(name, &result[name.as_str()], *pass)
        }
    }
}

/// Whether a check came out as expected, or what it said instead.
fn verdict_of(name: &str, result: &Value, pass: bool) -> Result<(), String> {
    if result["pass"] == json!(pass) {
        return Ok(());
    }
    let said = match &result["pass"] {
        Value::Bool(true) => "passed".to_owned(),
        Value::Bool(false) => format!(
            "failed ({} offenders; first {})",
            result["count"],
            result["offenders"].get(0).unwrap_or(&result["why"])
        ),
        _ => format!("did not apply: {}", result["why"].as_str().unwrap_or("")),
    };
    Err(format!("check {name} {said}"))
}

/// How the camera stands apart from where it stood, if it does.
fn camera_differs(then: &Value, now: &Value) -> Option<String> {
    let number = |v: &Value| v.as_f64().unwrap_or(f64::NAN);
    let mut how = Vec::new();
    for axis in 0..3 {
        let (a, b) = (number(&then["target"][axis]), number(&now["target"][axis]));
        if (a - b).abs() > SAME_PLACE || a.is_nan() != b.is_nan() {
            how.push(format!("target[{axis}] {a} → {b}"));
        }
    }
    for (field, slack) in [
        ("distance", SAME_PLACE),
        ("yaw", SAME_ANGLE),
        ("pitch", SAME_ANGLE),
    ] {
        let (a, b) = (number(&then[field]), number(&now[field]));
        if (a - b).abs() > slack {
            how.push(format!("{field} {a} → {b}"));
        }
    }
    (!how.is_empty()).then(|| how.join(", "))
}

/// How the session went: whether it is clean, each check over the snaps, the `expect`
/// lines, the steps that failed, and each snap's checks.
fn summary(drive: &Drive) -> Value {
    let checked = checks::summarise(&drive.checked);
    let checks_pass = checked
        .as_object()
        .is_some_and(|all| all.values().all(|c| c["pass"] == json!(true)));
    let expects_pass = drive.expects.iter().all(|e| e["ok"] == json!(true));
    let failed: Vec<&Value> = drive
        .log
        .iter()
        .filter(|s| s["ok"] != json!(true))
        .collect();
    let snaps: serde_json::Map<String, Value> = drive
        .checked
        .iter()
        .map(|(name, checks)| (name.clone(), checks.clone()))
        .collect();
    json!({
        "ok": checks_pass && expects_pass && failed.is_empty(),
        "checks": checked,
        "expects": drive.expects,
        "failed_steps": failed.iter().map(|s| json!({
            "step": s["step"], "line": s["line"], "do": s["do"], "trouble": s["trouble"],
        })).collect::<Vec<_>>(),
        "snaps": snaps,
    })
}

/// How clear a point is of every function, name, text and button drawn, in points.
fn clearance(view: &View, drawn: &[Drawn], at: Vec2) -> f32 {
    let p = egui::pos2(at.x, at.y);
    let dots = drawn
        .iter()
        .filter(|d| d.on_screen)
        .map(|d| d.at.distance(at) - d.radius);
    let names = view
        .screen
        .labels
        .iter()
        .map(|(pill, _)| pill.distance_to_pos(p));
    let texts = view.screen.texts.iter().map(|t| t.rect.distance_to_pos(p));
    let buttons = view
        .screen
        .widgets
        .iter()
        .map(|w| w.rect.distance_to_pos(p));
    dots.chain(names)
        .chain(texts)
        .chain(buttons)
        .fold(f32::MAX, f32::min)
}

/// Whether a point is on the graph with nothing over it.
fn open_ground(view: &View, at: Vec2) -> bool {
    view.on_screen(at) && view.screen.overlay_at(at).is_none()
}

/// Where to double-click a module's sphere: the spot inside it farthest from its
/// functions and names.
fn module_point(view: &View, cluster: usize) -> Result<Vec2, String> {
    let module = view.explorer.cluster_name(cluster);
    let nucleus = view.explorer.nucleus();
    let ball = nucleus
        .ball(cluster)
        .filter(|ball| nucleus.shows_body(ball.body))
        .ok_or_else(|| format!("module {module} has no sphere drawn"))?;
    let centre = view
        .to_screen(ball.center)
        .ok_or_else(|| format!("module {module} is behind the camera"))?;
    let radius = view.screen_radius(ball.center, ball.radius);
    let drawn = view.drawn();
    let spots = [0.0, 0.25, 0.45, 0.65].into_iter().flat_map(|share| {
        (0..12).map(move |i| {
            let angle = std::f32::consts::TAU * (i as f32) / 12.0;
            centre + Vec2::from_angle(angle) * radius * share
        })
    });
    spots
        .filter(|&at| open_ground(view, at))
        .map(|at| (clearance(view, &drawn, at), at))
        .filter(|(room, _)| *room >= CLEAR_SPOT)
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, at)| at)
        .ok_or_else(|| {
            format!("no spot on module {module}'s sphere is clear of its functions and names")
        })
}

/// Where to click on nothing: of the spots on the graph roomily clear of every
/// function, name and sphere (else of every function and name), the one nearest the
/// middle.
fn empty_point(view: &View) -> Result<Vec2, String> {
    let drawn = view.drawn();
    let n = view.explorer.nucleus();
    let spheres: Vec<(Vec2, f32)> = n
        .balls()
        .filter(|(_, ball)| n.shows_body(ball.body))
        .filter_map(|(_, ball)| {
            Some((
                view.to_screen(ball.center)?,
                view.screen_radius(ball.center, ball.radius),
            ))
        })
        .collect();
    let graph = view.screen.graph;
    let spots = (1..24).flat_map(|i| {
        (1..16).map(move |j| {
            Vec2::new(
                graph.min.x + graph.width() * (i as f32) / 24.0,
                graph.min.y + graph.height() * (j as f32) / 16.0,
            )
        })
    });
    let rated: Vec<(f32, f32, Vec2)> = spots
        .filter(|&at| open_ground(view, at))
        .map(|at| {
            let room = clearance(view, &drawn, at);
            let off_spheres = spheres
                .iter()
                .map(|&(c, r)| at.distance(c) - r)
                .fold(f32::MAX, f32::min);
            (room.min(off_spheres), room, at)
        })
        .collect();
    let middle = graph_middle(view);
    let best = |key: fn(&(f32, f32, Vec2)) -> f32| {
        let roomy = rated
            .iter()
            .filter(|r| key(r) >= ROOMY_SPOT)
            .min_by(|a, b| a.2.distance(middle).total_cmp(&b.2.distance(middle)));
        roomy
            .or_else(|| {
                rated
                    .iter()
                    .filter(|r| key(r) >= CLEAR_SPOT)
                    .max_by(|a, b| key(a).total_cmp(&key(b)))
            })
            .map(|r| r.2)
    };
    best(|r| r.0)
        .or_else(|| best(|r| r.1))
        .ok_or_else(|| "no spot on the graph is clear of every function and name".to_owned())
}

/// Where to click a function's name: the middle of its pill, the topmost drawn.
fn label_point(view: &View, tile: usize) -> Result<Vec2, String> {
    let (pill, _) = view
        .screen
        .labels
        .iter()
        .rev()
        .find(|(_, t)| *t == tile)
        .ok_or_else(|| format!("{} has no name drawn", node_name(&view.explorer, tile)))?;
    let shown = pill.intersect(view.screen.graph);
    if !shown.is_positive() {
        return Err(format!(
            "{}'s name is drawn off the graph",
            node_name(&view.explorer, tile)
        ));
    }
    let c = shown.center();
    Ok(Vec2::new(c.x, c.y))
}

/// Where to click the journey bar numbered `number` (or, failing that, named it, as
/// the journey's start must be): inside its left end, clear of the badge that unfolds
/// it.
fn bar_point(view: &View, number: &str) -> Result<Vec2, String> {
    let Some(drawn) = &view.screen.bars else {
        return Err("the journey panel is not showing: no journey is open".to_owned());
    };
    let disclosed = view.explorer.disclosure();
    // By number, else by the name on it, among the bars disclosed.
    let find = |matches: &dyn Fn(SpanId) -> bool| {
        view.explorer.journey().and_then(|journey| {
            journey
                .spans()
                .map(|(id, _)| id)
                .filter(|&id| disclosed.shows(id))
                .find(|&id| matches(id))
        })
    };
    let span = find(&|id| disclosed.number(id) == Some(number))
        .or_else(|| find(&|id| view.explorer.label(id) == number))
        .ok_or_else(|| {
            let numbers: Vec<&str> = view
                .explorer
                .journey()
                .into_iter()
                .flat_map(|journey| journey.spans().map(|(id, _)| id))
                .filter_map(|id| disclosed.number(id))
                .filter(|n| !n.is_empty())
                .collect();
            format!(
                "no bar is numbered or named {number} at this depth; bars: {}",
                numbers.join(" ")
            )
        })?;
    let Some(&(_, rect)) = drawn.bars.iter().find(|(s, _)| *s == span) else {
        return Err(format!("bar {number} is not drawn"));
    };
    let shown = rect.intersect(drawn.content);
    if !shown.is_positive() {
        return Err(format!("bar {number} is scrolled out of the journey panel"));
    }
    let x = shown.min.x + (shown.width() / 2.0).min(12.0);
    Ok(Vec2::new(x, shown.center().y))
}

/// Where to click the button or chip that says `text`, and a note when part of it
/// is clipped away.
fn button_point(view: &View, text: &str) -> Result<(Vec2, Option<String>), String> {
    let widget = pick(&view.screen.widgets, text)?;
    if !widget.enabled {
        return Err(format!("the button `{}` is disabled", widget.text));
    }
    let note = widget.cut_off().then(|| {
        format!(
            "the button `{}` is cut off: only {:.0} of its {:.0} points wide show",
            widget.text,
            widget.rect.width(),
            widget.full.width()
        )
    });
    let c = widget.rect.center();
    Ok((Vec2::new(c.x, c.y), note))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(words: &str, x: f32) -> Painted {
        let rect = egui::Rect::from_min_size(egui::pos2(x, 0.0), egui::vec2(30.0, 10.0));
        Painted {
            text: words.to_owned(),
            rect,
            clip: rect,
            opacity: 1.0,
            size: 12.0,
        }
    }

    fn rect(x0: f32, x1: f32) -> egui::Rect {
        egui::Rect::from_min_max(egui::pos2(x0, -5.0), egui::pos2(x1, 15.0))
    }

    #[test]
    fn an_area_holding_buttons_is_no_button_even_when_they_are_cut_off() {
        // An overlay wider than the window, its close button mostly past the edge.
        let found = vec![
            (rect(-50.0, 1650.0), rect(0.0, 1600.0), true),
            (rect(-30.0, 10.0), rect(0.0, 10.0), true),
            (rect(100.0, 140.0), rect(100.0, 140.0), true),
        ];
        let texts = [text("close search, this list", 100.0), text("close", -25.0)];
        let widgets = buttons(found, &texts);
        let said: Vec<&str> = widgets.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(said, ["close", "close search, this list"]);
        let close = pick(&widgets, "Close").unwrap();
        assert!(close.cut_off(), "only 10 of its 40 points show");
        assert_eq!(close.rect.center().x, 5.0, "clicked where it shows");
    }

    #[test]
    fn a_button_is_found_by_part_of_a_short_label_only() {
        let widget = |text: &str| Widget {
            text: text.to_owned(),
            rect: rect(0.0, 10.0),
            full: rect(0.0, 10.0),
            enabled: true,
        };
        let widgets = [
            widget("one two three four five six seven eight nine callers"),
            widget("● 4 callers  I"),
        ];
        assert_eq!(pick(&widgets, "callers").unwrap().text, "● 4 callers  I");
        assert!(
            pick(&widgets, "nothing")
                .unwrap_err()
                .contains("no button says")
        );
    }
}
