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
    sync::Arc,
    time::Duration,
};

use bevy::{
    ecs::system::SystemParam,
    input::{
        ButtonState, InputSystems,
        keyboard::{Key, KeyboardInput, NativeKeyCode},
        mouse::{MouseButtonInput, MouseMotion, MouseScrollUnit, MouseWheel},
        touch::TouchPhase,
    },
    prelude::*,
    render::view::screenshot::{Screenshot, ScreenshotCaptured, save_to_disk},
    window::{
        CursorMoved, CursorOptions, Monitor, PrimaryMonitor, PrimaryWindow, RequestRedraw,
        WindowEvent, WindowLevel, WindowOccluded, WindowPosition,
    },
    winit::WinitSettings,
};
use bevy_egui::{EguiContext, EguiFullOutput, EguiPostUpdateSet, PrimaryEguiContext, egui};
use codeviz_core::SpanId;
use serde_json::{Value, json};

use crate::{
    camera::OrbitCamera,
    checks::{self, Since},
    explorer::Explorer,
    idle::Awake,
    keys::Press,
    labels::node_name,
    nucleus::Flow,
    scene::Tile,
    script::{
        Aim, Expect, Gesture, Kind, Line, Name, Step, bar_number, close_matches, snap_label,
        snap_name, unknown,
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
/// Times a snapshot that comes out black is taken again, the window tucked into
/// another corner first; and frames and time given it to draw there.
const RETAKES: u32 = 3;
const RETUCK_FRAMES: u32 = 10;
const RETUCK_TIME: Duration = Duration::from_millis(500);
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
            // A bar may be named as it reads (`Explorer.spotlit`), found when it shows.
            Some(Name::Bar(query)) => {
                if let Ok(tile) = find_unit(explorer, query) {
                    names.units.insert(query.to_owned(), tile);
                }
                Ok(())
            }
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
        // The window is never focused; it draws every frame all the same.
        .insert_resource(WinitSettings::continuous())
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

/// Points of the window left on screen, in a screen's bottom corner.
const PEEK: f32 = 4.0;

/// Shows the window, which opened hidden, over the others but all off screen except
/// a corner `PEEK` points square: macOS stops drawing a window that is covered or
/// wholly off screen, and snapshots come out black. The corner is on one screen and
/// the rest on none; it takes no clicks. Waits for winit to report the screens,
/// asking for frames meanwhile, as an idle window would not draw them. Hidden again
/// (a snapshot came out black), it goes to the next such corner.
pub(crate) fn tuck_away(
    mut window: Single<(&mut Window, &mut CursorOptions), With<PrimaryWindow>>,
    monitors: Query<(&Monitor, Has<PrimaryMonitor>)>,
    mut redraw: MessageWriter<RequestRedraw>,
    mut tucks: Local<usize>,
) {
    let (window, cursor) = &mut *window;
    if window.visible {
        return;
    }
    // Screens in points, the main one first.
    let mut screens: Vec<(Rect, bool)> = monitors
        .iter()
        .map(|(monitor, main)| {
            let scale = monitor.scale_factor;
            let at = (monitor.physical_position.as_dvec2() / scale).as_vec2();
            let size = (monitor.physical_size().as_dvec2() / scale).as_vec2();
            (Rect::from_corners(at, at + size), main)
        })
        .collect();
    screens.sort_by_key(|&(_, main)| !main);
    let size = window.resolution.size();
    // The window's top left in a screen's bottom right, or its top right in the
    // bottom left.
    let corners: Vec<Vec2> = screens
        .iter()
        .flat_map(|(screen, _)| {
            let y = screen.max.y - PEEK;
            [
                Vec2::new(screen.max.x - PEEK, y),
                Vec2::new(screen.min.x + PEEK - size.x, y),
            ]
        })
        .collect();
    // How much of the window a place leaves on the screens.
    let shown = |at: Vec2| -> f32 {
        let placed = Rect::from_corners(at, at + size);
        screens
            .iter()
            .map(|(screen, _)| screen.intersect(placed))
            .filter(|seen| !seen.is_empty())
            .map(|seen| seen.width() * seen.height())
            .sum()
    };
    let fits: Vec<Vec2> = corners
        .iter()
        .copied()
        .filter(|&at| shown(at) <= PEEK * PEEK + 1.0)
        .collect();
    let Some(&at) = fits.get(*tucks % fits.len().max(1)).or(corners.first()) else {
        redraw.write(RequestRedraw);
        return;
    };
    *tucks += 1;
    let at = (at * window.scale_factor()).round().as_ivec2();
    window.position = WindowPosition::At(at);
    window.window_level = WindowLevel::AlwaysOnTop;
    window.visible = true;
    cursor.hit_test = false;
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
    /// Saving what shows on each frame, this many more, as the step before plays out.
    Filming(u32),
    /// Waiting for a snapshot of the window to be taken, asked for on this frame.
    Capturing(u32),
    /// The snapshot came out black: the window was tucked into another corner on this
    /// frame, at this time, to be taken again once it draws there.
    Retucked(u32, Duration),
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
    /// Snapshots that came out black, by name, not yet taken again; how often the one
    /// under way has been; and those still black after every retake, which have no
    /// PNG: the window was not drawn.
    black: Vec<String>,
    retakes: u32,
    not_drawn: Vec<String>,
    /// Whether macOS counts the window as covered, so it draws nothing into it.
    covered: bool,
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
    /// Whether the camera stands where a wheel turn, an orbit or a pan put it, since
    /// the view last framed itself; and what the view was on when it last did.
    by_hand: bool,
    framed_on: (bool, Option<usize>),
    /// R or F, when it framed the view last and nothing but a window sized anew has
    /// moved it since.
    framed_by: Option<Kind>,
    /// Whether a step since the last snapshot may pick something: anything but an
    /// expand and the steps that only look.
    picks: bool,
    /// The module expanded, and whether the camera stood where the hands put it, as
    /// the step under way began; and while a module is expanded, whether the camera
    /// was where the hands put it before the first expand, which shrinking back
    /// returns it to.
    began: (Option<usize>, bool),
    expanded_by_hand: Option<bool>,
    /// What is worth knowing of the step under way that is no failure.
    notes: Vec<String>,
    /// The notice said last as the step under way began, by its count: an `expect
    /// notice` and a pan judge only what was said since.
    said_before: Option<u64>,
    /// Where the pointer was as the step under way began.
    pointer_before: Option<Vec2>,
    /// When the view last moved, as the step under way settles.
    moved_at: Option<Duration>,
    /// Frames of a transition saved so far, by the `frames` step under way.
    filmed: u32,
    /// How many steps that may move the view have run since the session opened.
    moves: usize,
    /// For a pan under way, the point the camera looked at as it began, in the
    /// world, and where it showed then: how far the view moves.
    pan_from: Option<(Vec3, Vec2)>,
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
    /// Where names on the graph are drawn: the graph less a docked panel.
    pub(crate) names: egui::Rect,
    /// How big each name on the graph was laid out, room found for it or not.
    pub(crate) name_sizes: Arc<HashMap<crate::labels::Named, egui::Vec2>>,
    /// Every dot names keep off, and how far: the function, where, how far.
    pub(crate) kept_off: Arc<Vec<(usize, egui::Pos2, f32)>>,
    /// Where names keep off the calls coming into each dot: the function, the line's
    /// last stretch from the dot back.
    pub(crate) arrivals: Arc<Vec<(usize, Vec<egui::Pos2>)>>,
    /// Lit functions left out where a name placed before them took their place: that
    /// place.
    pub(crate) homes: Arc<HashMap<usize, egui::Rect>>,
    pub(crate) texts: Vec<Painted>,
    pub(crate) widgets: Vec<Widget>,
    pub(crate) bars: Option<DrawnBars>,
    /// Areas over the graph and panels, by name, with where they are.
    pub(crate) overlays: Vec<(String, egui::Rect)>,
    /// Each function's name on the graph, a click on which picks it: its pill, and the
    /// function, bottom first.
    pub(crate) labels: Vec<(egui::Rect, usize)>,
    /// The search box's rows, while it shows.
    pub(crate) search: Option<SearchRows>,
}

impl Default for Screen {
    fn default() -> Self {
        Self {
            window: egui::Vec2::ZERO,
            graph: egui::Rect::NOTHING,
            names: egui::Rect::NOTHING,
            name_sizes: Arc::default(),
            kept_off: Arc::default(),
            arrivals: Arc::default(),
            homes: Arc::default(),
            texts: Vec::new(),
            widgets: Vec::new(),
            bars: None,
            overlays: Vec::new(),
            labels: Vec::new(),
            search: None,
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
    /// Each bar, where it is, where it can show (the start's row stays put, the rows
    /// below scroll under it) and what it says.
    pub(crate) bars: Vec<(SpanId, egui::Rect, egui::Rect, String)>,
    /// The badges that unfold a bar's folded calls, by the bar's span.
    pub(crate) chips: Vec<(SpanId, egui::Rect)>,
    /// The bars of the start's callers, in the row above its bar, by function.
    pub(crate) callers: Vec<(usize, egui::Rect, String)>,
    /// The track down the panel's side, drawn when rows lie below what it shows; the
    /// one along its foot, when the journey is wider than it.
    pub(crate) rows_track: Option<egui::Rect>,
    pub(crate) across_track: Option<egui::Rect>,
    /// The marks at its left and right sides saying how many bars lie past them.
    pub(crate) beside: [Option<(egui::Rect, usize)>; 2],
}

fn bars_id() -> egui::Id {
    egui::Id::new("drawn_journey_bars")
}

/// Notes where the journey panel drew its bars this pass and what each says, the
/// badges on them that unfold what is folded, the bars of the start's callers, the
/// tracks down its side and along its foot, and the marks at its sides.
pub(crate) fn note_bars(
    ctx: &egui::Context,
    content: egui::Rect,
    bars: Vec<(SpanId, egui::Rect, egui::Rect, String)>,
    chips: Vec<(SpanId, egui::Rect)>,
    callers: Vec<(usize, egui::Rect, String)>,
    (rows_track, across_track): (Option<egui::Rect>, Option<egui::Rect>),
    beside: [Option<(egui::Rect, usize)>; 2],
) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        d.insert_temp(
            bars_id(),
            DrawnBars {
                pass,
                content,
                bars,
                chips,
                callers,
                rows_track,
                across_track,
                beside,
            },
        );
    });
}

/// What the graph drew this frame that a snapshot cannot read back from the scene:
/// the calls drawn as lines, the lines between modules, and the rings round
/// functions, each noted by the system that draws it: the arrows on screen, the
/// rings in the world.
#[derive(Resource, Debug, Default)]
pub(crate) struct Marks {
    pub(crate) calls: Vec<CallMark>,
    pub(crate) tubes: Vec<TubeMark>,
    pub(crate) rings: Vec<RingMark>,
    pub(crate) exits: Vec<ExitMark>,
}

/// Where a call leaves the free area, labelled with the function at its far end.
#[derive(Debug, Clone)]
pub(crate) struct ExitMark {
    pub(crate) from: usize,
    pub(crate) to: usize,
    /// The function off the free area: the call's caller or callee.
    pub(crate) far: usize,
    pub(crate) text: String,
    pub(crate) rect: egui::Rect,
    /// Where the call's line crosses the free area's edge.
    pub(crate) at: Vec2,
}

/// A call drawn from function to function.
#[derive(Debug, Clone)]
pub(crate) struct CallMark {
    pub(crate) from: usize,
    pub(crate) to: usize,
    pub(crate) flow: Flow,
    pub(crate) color: Color,
    pub(crate) shape: ArrowShape,
}

/// A line between two modules, by its index among the nucleus's.
#[derive(Debug, Clone)]
pub(crate) struct TubeMark {
    pub(crate) index: usize,
    pub(crate) shape: ArrowShape,
}

/// How an arrow shows on screen, in points: its line's width, its head's length and
/// width, where its tip is, the points its line runs through, from its start to its
/// tip, and where along them it breaks round a dot.
#[derive(Debug, Clone)]
pub(crate) struct ArrowShape {
    pub(crate) width: f32,
    pub(crate) head: f32,
    pub(crate) head_width: f32,
    pub(crate) tip: Vec2,
    pub(crate) path: Vec<Vec2>,
    /// Where its line does not show, as how far along it from its start, in points:
    /// a line between modules, behind the spheres it passes.
    pub(crate) gaps: Vec<[f32; 2]>,
    /// The dots it runs under, by tile, each drawn over it.
    pub(crate) under: Vec<usize>,
}

/// A ring round a function, so big in the world, in a color.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RingMark {
    pub(crate) tile: usize,
    pub(crate) radius: f32,
    pub(crate) color: Color,
}

/// The rows the search box lists, top first, as last drawn: where each is, the
/// function it finds, what it says, and whether Enter takes it.
#[derive(Debug, Clone)]
pub(crate) struct SearchRows {
    pass: u64,
    pub(crate) rows: Vec<SearchRow>,
    /// Where the rows show; one outside it is scrolled out of view.
    pub(crate) content: egui::Rect,
}

#[derive(Debug, Clone)]
pub(crate) struct SearchRow {
    pub(crate) rect: egui::Rect,
    pub(crate) unit: String,
    pub(crate) text: String,
    pub(crate) highlighted: bool,
}

fn search_rows_id() -> egui::Id {
    egui::Id::new("drawn_search_rows")
}

/// Notes the rows the search box drew this pass.
pub(crate) fn note_search_rows(ctx: &egui::Context, content: egui::Rect, rows: Vec<SearchRow>) {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        d.insert_temp(
            search_rows_id(),
            SearchRows {
                pass,
                rows,
                content,
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
    let search = ctx
        .data_mut(|d| d.get_temp::<SearchRows>(search_rows_id()))
        .filter(|rows| rows.pass + 1 == pass);
    let overlays = ctx.memory(|m| {
        let visible = m.areas().visible_layer_ids();
        OVERLAYS
            .iter()
            .filter(|(id, _)| visible.iter().any(|l| l.id == egui::Id::new(*id)))
            .filter_map(|&(id, name)| Some((name.to_owned(), m.area_rect(egui::Id::new(id))?)))
            .collect::<Vec<_>>()
    });
    // The notice, a moment in the bar along the top, and the hint at the keys along
    // the graph's foot: never over a name.
    let overlays = overlays
        .into_iter()
        .chain(crate::notice::drawn(&ctx, pass).map(|r| ("notice".to_owned(), r)))
        .chain(crate::key_help::hint_drawn(&ctx, pass).map(|r| ("key hint".to_owned(), r)))
        .collect();
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
        names: crate::labels::names_area(&ctx).unwrap_or(graph),
        name_sizes: crate::labels::name_sizes(&ctx),
        kept_off: crate::labels::kept_off(&ctx),
        arrivals: crate::labels::arrivals(&ctx),
        homes: crate::labels::homes(&ctx),
        texts,
        widgets: clickable,
        bars,
        overlays,
        labels: crate::labels::drawn_labels(&ctx),
        search,
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
    /// What it is filled with.
    pub(crate) color: Option<Color>,
}

/// Everything a snapshot reads.
#[derive(SystemParam)]
pub(crate) struct View<'w, 's> {
    pub(crate) explorer: Res<'w, Explorer>,
    pub(crate) help: Res<'w, KeyHelp>,
    pub(crate) chrome: Res<'w, Chrome>,
    pub(crate) screen: Res<'w, Screen>,
    pub(crate) marks: Res<'w, Marks>,
    materials: Res<'w, Assets<StandardMaterial>>,
    tiles: Query<
        'w,
        's,
        (
            &'static Tile,
            &'static GlobalTransform,
            &'static InheritedVisibility,
            Option<&'static MeshMaterial3d<StandardMaterial>>,
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

    /// The module whose sphere a click at `at` lands on: the nearest the camera.
    pub(crate) fn module_at(&self, at: Vec2) -> Option<usize> {
        let (camera, transform, _) = self.cameras.single().ok()?;
        let ray = camera.viewport_to_world(transform, at).ok()?;
        crate::pick::module_at(self.explorer.nucleus(), ray)
    }

    pub(crate) fn on_screen(&self, at: Vec2) -> bool {
        self.screen.graph.contains(egui::pos2(at.x, at.y))
    }

    /// Every function drawn, in tile order.
    pub(crate) fn drawn(&self) -> Vec<Drawn> {
        let mut drawn: Vec<Drawn> = self
            .tiles
            .iter()
            .filter(|(_, _, visible, _)| visible.get())
            .filter_map(|(tile, transform, _, material)| {
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
                    color: material
                        .and_then(|m| self.materials.get(&m.0))
                        .map(|m| m.base_color),
                })
            })
            .collect();
        drawn.sort_by_key(|d| d.tile);
        drawn
    }

    /// One function as drawn, or why it is not.
    fn find(&self, tile: usize) -> Result<Drawn, String> {
        let name = || node_name(&self.explorer, tile);
        let Some((_, transform, visible, _)) = self.tiles.iter().find(|(t, ..)| t.0 == tile) else {
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
        if let Some(overlay) = self.screen.overlay_at(at) {
            return Err(format!(
                "{} is drawn under the {overlay}, at ({:.0}, {:.0})",
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
            color: None,
        })
    }

    /// Where the camera is and which way it faces.
    pub(crate) fn camera(&self) -> Option<(Vec3, Quat)> {
        let (_, transform, _) = self.cameras.single().ok()?;
        Some((transform.translation(), transform.rotation()))
    }

    /// Where the camera stands this frame as the orbit has it, on its way or not:
    /// what it looks at, from how far, at what yaw and pitch.
    pub(crate) fn orbit_pose(&self) -> Option<crate::camera::Pose> {
        self.cameras
            .iter()
            .next()
            .map(|(_, _, orbit)| orbit.pose_now())
    }

    pub(crate) fn orbit_held(&self) -> bool {
        self.cameras.iter().any(|(_, _, orbit)| orbit.held_map())
    }

    pub(crate) fn orbit_farthest(&self) -> bool {
        self.cameras.iter().any(|(_, _, orbit)| orbit.farthest())
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
                (e.disclose(), e.folds_opened()),
                e.aside(),
                e.nucleus().start(),
                e.revision(),
                e.moved(),
                e.aim(),
                e.nucleus().expanded(),
                self.help.0,
                // The search box grows over a frame or two as what it finds is laid out.
                &self.screen.overlays,
            )
        )
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
    mut window: Single<&mut Window, With<PrimaryWindow>>,
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
                drive.moved_at = None;
                drive.phase = Phase::Settling(drive.frame, now);
                // A `frames` step next saves the frames as this one plays out.
                let next = drive.step.map_or(0, |i| i + 1);
                if matches!(
                    drive.lines.get(next).map(|l| &l.step),
                    Some(Step::Frames(_))
                ) {
                    finish(&mut drive, &view, None);
                    begin_next(&mut commands, &mut drive, &view, now);
                }
            }
        }
        Phase::Filming(left) => {
            film(&mut commands, &mut drive, &view);
            drive.phase = if left > 1 {
                Phase::Filming(left - 1)
            } else {
                drive.moved_at = None;
                Phase::Settling(drive.frame, now)
            };
        }
        Phase::Settling(frame, since) => {
            if moving {
                drive.moved_at = Some(now);
            }
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
                let settled = drive
                    .moved_at
                    .map_or(Duration::ZERO, |at| at.saturating_sub(since));
                finish(&mut drive, &view, Some((settled, waited)));
                begin_next(&mut commands, &mut drive, &view, now);
            }
        }
        Phase::Capturing(_) | Phase::Retucked(..) => {
            if photograph(
                &mut commands,
                &mut drive,
                &mut window,
                shots.is_empty(),
                now,
            ) {
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

/// Moves a snapshot along. One that comes out black (macOS stopped drawing the
/// window: the display dimmed or slept, or something lies over its corner) is taken
/// again with the window tucked into another corner; still black after `RETAKES`, it
/// is left without a PNG and the session says the window was not drawn, apart from
/// how the view fared. Returns whether it is done.
fn photograph(
    commands: &mut Commands,
    drive: &mut Drive,
    window: &mut Window,
    taken: bool,
    now: Duration,
) -> bool {
    let Some(name) = drive.snaps.last().cloned() else {
        return true;
    };
    match drive.phase {
        Phase::Capturing(at) if taken && drive.frame >= at + 2 => {
            let Some(i) = drive.black.iter().position(|b| *b == name) else {
                drive.retakes = 0;
                return true;
            };
            drive.black.remove(i);
            if drive.retakes < RETAKES {
                drive.retakes += 1;
                info!(
                    "session: {name} came out black (window covered: {}); retake {} of {RETAKES} from another corner",
                    drive.covered, drive.retakes
                );
                // Hidden, the window is tucked away again, into the next corner.
                window.visible = false;
                drive.phase = Phase::Retucked(drive.frame, now);
                return false;
            }
            warn!("session: {name}: the window was not drawn; no PNG");
            drive.not_drawn.push(name);
            drive.retakes = 0;
            return true;
        }
        Phase::Retucked(at, since)
            if drive.frame >= at + RETUCK_FRAMES && now.saturating_sub(since) >= RETUCK_TIME =>
        {
            shoot(commands, &drive.dir, &name);
            drive.phase = Phase::Capturing(drive.frame);
        }
        _ => {}
    }
    false
}

/// How far a pan may fall short of the drag, or overshoot it, without a bound saying
/// why: a share of the drag, and at least this many points.
const PAN_SLACK: f32 = 0.1;
const PAN_SLACK_POINTS: f32 = 4.0;

/// What a notice said since the step under way began, if anything.
fn said_since<'a>(drive: &Drive, view: &'a View) -> Option<&'a str> {
    view.explorer
        .notice()
        .filter(|&(count, _)| Some(count) != drive.said_before)
        .map(|(_, text)| text)
}

/// Logs how the step under way went: whether it went as meant, where the pointer was
/// as it began, and, when it settled, how long the view kept moving and how long the
/// session waited.
fn finish(drive: &mut Drive, view: &View, settled: Option<(Duration, Duration)>) {
    let Some(index) = drive.step else {
        return;
    };
    let mut trouble = std::mem::take(&mut drive.trouble);
    let notes = std::mem::take(&mut drive.notes);
    // A pan says how far the view moved, against how far the drag went: what lay
    // under the hand moves with it, and only a bound, which says so, stops it.
    let pan = match (drive.pan_from.take(), &drive.lines[index].step) {
        (Some((world, was)), Step::Pan(asked)) => {
            let by = view.to_screen(world).map_or(Vec2::ZERO, |now| now - was);
            let said = said_since(drive, view);
            let short = (by - *asked).length() > (asked.length() * PAN_SLACK).max(PAN_SLACK_POINTS);
            if short && said.is_none() {
                trouble.push(format!(
                    "the pan moved the view by {by:.0} of {asked:.0} dragged, and nothing said why"
                ));
            }
            if !short && said.is_some_and(|t| t.contains("edge")) {
                trouble.push(
                    "said the map's edge stopped the pan, yet it moved all the way".to_owned(),
                );
            }
            Some((by, *asked))
        }
        _ => None,
    };
    let mut entry = json!({
        "step": index + 1,
        "line": drive.lines[index].number,
        "do": drive.lines[index].text,
        "ok": trouble.is_empty(),
        "pointer": drive.pointer_before.map(|at| [at.x.round(), at.y.round()]),
        "then": crate::truth::state(view),
    });
    if !trouble.is_empty() {
        entry["trouble"] = json!(trouble);
    }
    if !notes.is_empty() {
        entry["note"] = json!(notes);
    }
    if let Some((settled, waited)) = settled {
        entry["settled_ms"] = json!(settled.as_millis());
        entry["waited_ms"] = json!(waited.as_millis());
    }
    if let Some((by, asked)) = pan {
        entry["view_moved"] = json!(by.length() >= 1.0);
        entry["moved_by"] = json!([by.x.round(), by.y.round()]);
        entry["dragged_by"] = json!([asked.x.round(), asked.y.round()]);
    }
    // Shrinking back returns the camera to where it stood before the first expand,
    // where the hands may have put it; R and F frame it anew.
    let module = view.explorer.nucleus().expanded_module();
    let expanded = module.is_some();
    match (drive.began.0.is_some(), expanded) {
        (false, true) => drive.expanded_by_hand = Some(drive.began.1),
        // The expansion taken to another module: flown there and framed, but by Back
        // or Forward, put back where the camera stood then, perhaps by hand.
        (true, true) if module != drive.began.0 => {
            drive.by_hand = drive.lines[index].step.kind() == Kind::Back;
        }
        (true, false) => {
            let saved = drive.expanded_by_hand.take();
            let kind = drive.lines[index].step.kind();
            if !matches!(kind, Kind::FrameAll | Kind::FrameFocus) {
                drive.by_hand = saved.unwrap_or(false);
            }
        }
        _ => {}
    }
    // Opening or leaving a journey, or another, frames the view anew.
    let on = (view.explorer.browsing(), view.explorer.start());
    if on != drive.framed_on {
        drive.framed_on = on;
        drive.by_hand = false;
    }
    info!("session: {entry}");
    drive.log.push(entry);
}

/// Whether a step leaves the camera where the user's hands put it: `Some(true)` for a
/// wheel turn, an orbit or a pan, `Some(false)` for what frames the view itself.
fn by_hand(kind: Kind) -> Option<bool> {
    match kind {
        Kind::Zoom { .. } | Kind::Orbit | Kind::Pan => Some(true),
        Kind::FrameAll | Kind::FrameFocus | Kind::Expand => Some(false),
        _ => None,
    }
}

/// What happened since the last snapshot, as the checks read it.
fn since(drive: &Drive) -> Since {
    Since {
        kinds: drive.kinds.clone(),
        by_hand: drive.by_hand,
        playing: false,
        opening: drive.moves == 0,
        framed_by: drive.framed_by,
        picks: drive.picks,
    }
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
        drive.pointer_before = drive.pointer;
        if let Step::Frames(n) = step {
            drive.filmed = 0;
            drive.phase = Phase::Filming(n);
            return;
        }
        if !matches!(step, Step::Expect(_)) {
            drive.said_before = view.explorer.notice().map(|(count, _)| count);
        }
        drive.kinds.push(step.kind());
        drive.picks |= step.kind() != Kind::Expand
            && !matches!(
                step,
                Step::Expect(_)
                    | Step::Wait(_)
                    | Step::Point {
                        gesture: Gesture::Hover | Gesture::LabelHover,
                        ..
                    }
            );
        match step.kind() {
            kind @ (Kind::FrameAll | Kind::FrameFocus) => drive.framed_by = Some(kind),
            Kind::Still | Kind::Resize => {}
            _ => drive.framed_by = None,
        }
        drive.moves += usize::from(step.kind() != Kind::Still);
        drive.pan_from = match step {
            Step::Pan(_) => view.orbit_pose().and_then(|pose| {
                let looked_at = pose.parts().0;
                Some((looked_at, view.to_screen(looked_at)?))
            }),
            _ => None,
        };
        drive.began = (view.explorer.nucleus().expanded_module(), drive.by_hand);
        if let Some(hand) = by_hand(step.kind()) {
            drive.by_hand = hand;
        }
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
    let checked = checks::run(&scene, drive.last_scene.as_ref(), &since(drive));
    drive.kinds.clear();
    drive.picks = false;
    drive.checked.push((name.clone(), checked.clone()));
    drive
        .cameras
        .push((snap_label(label), scene["state"]["camera"].clone()));
    scene["checks"] = checked;
    let mut written = scene.clone();
    drive.last_scene = Some(scene);
    written["log"] = json!(drive.log);
    write(&drive.dir.join(format!("{name}.json")), &written);
    shoot(commands, &drive.dir, &name);
    drive.snaps.push(name);
    drive.phase = Phase::Capturing(drive.frame);
}

/// Saves the window as this frame shows it, mid-transition, and what it shows beside
/// it, with the checks that hold of any one picture: none judged against the snapshot
/// before, none counted in the session's verdict.
fn film(commands: &mut Commands, drive: &mut Drive, view: &View) {
    drive.filmed += 1;
    let label = format!("frame {}", drive.filmed);
    let name = snap_name(drive.snaps.len() + 1, Some(&label));
    let mut scene = scene_now(drive, view);
    scene["snap"] = json!(name);
    scene["frame"] = json!(drive.filmed);
    let playing = Since {
        playing: true,
        ..Since::default()
    };
    scene["checks"] = checks::run(&scene, None, &playing);
    write(&drive.dir.join(format!("{name}.json")), &scene);
    shoot(commands, &drive.dir, &name);
    drive.snaps.push(name);
}

/// Takes the window as it shows next, saving it as `name` unless it comes out black,
/// which a window macOS has stopped drawing does.
fn shoot(commands: &mut Commands, dir: &std::path::Path, name: &str) {
    let mut save = save_to_disk(dir.join(format!("{name}.png")));
    let name = name.to_owned();
    commands.spawn(Screenshot::primary_window()).observe(
        move |captured: On<ScreenshotCaptured>, mut drive: ResMut<Drive>| {
            if is_black(&captured.image) {
                drive.black.push(name.clone());
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
        Step::Key(press) => {
            if view.chrome.search && press.types() {
                note_typing(drive);
            }
            key(*press)
        }
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
        Step::DoubleClickModule(module) => double_click_module(drive, view, module)?,
        // A bar of the journey, else of the start's callers in the row above it.
        Step::Bar(bar) => match bar_span(view, &drive.names, bar) {
            Ok(span) => click(bar_point(view, span, bar)?),
            Err(why) => click(caller_bar_point(view, &drive.names, bar).ok_or(why)?),
        },
        Step::Chip(bar) => {
            let span = bar_span(view, &drive.names, bar)?;
            click(chip_point(view, span, bar)?)
        }
        Step::SearchPick(text) => click(search_point(view, text)?),
        Step::Button(text) => {
            let (at, note) = button_point(view, text)?;
            drive.trouble.extend(note);
            click(at)
        }
        Step::Drag(by) => drag(drive, view, *by, MouseButton::Left),
        Step::Pan(by) => drag(drive, view, *by, MouseButton::Right),
        Step::Scroll { by, at } => scroll(drive, view, *by, at.as_ref())?,
        Step::Window(size) => vec![Act::Resize(*size), Act::Frames(10)],
        Step::Wait(span) => vec![Act::Pause(*span)],
        Step::Expect(expect) => {
            let judged = judge(drive, view, expect);
            note_expect(drive, judged.as_ref().err());
            judged?;
            Vec::new()
        }
        Step::Snap(_) | Step::Frames(_) => Vec::new(),
    })
}

/// A double click on a module's sphere, clear of its functions and names where it
/// can be, else on one of its functions, which expands the module as well.
fn double_click_module(drive: &mut Drive, view: &View, module: &str) -> Result<Vec<Act>, String> {
    let (at, clear) = module_point(view, drive.names.module(module)?)?;
    let mut acts = vec![Act::Pointer(at), Act::Frames(3)];
    if clear {
        acts.push(Act::ExpectClear);
    } else {
        drive.notes.push(format!(
            "no spot on module {module}'s sphere is clear of its functions and names: \
             double-clicked on one of them, which expands its module too"
        ));
    }
    acts.extend([PRESS, PRESS].concat());
    Ok(acts)
}

/// A wheel turn with the pointer on a function or a point first, else where it is
/// (the graph's middle before it has moved).
fn scroll(drive: &Drive, view: &View, by: Vec2, at: Option<&Aim>) -> Result<Vec<Act>, String> {
    let to = match at {
        Some(Aim::Unit(unit)) => {
            let drawn = view.find(drive.names.unit(unit)?).map_err(|why| {
                format!(
                    "cannot scroll at {unit}: {why}; scroll at a point, or bring it on \
                     screen first (F, R or a pan)"
                )
            })?;
            Some(drawn.at)
        }
        Some(Aim::At(point)) => Some(*point),
        None => drive.pointer.is_none().then(|| graph_middle(view)),
    };
    let mut acts = Vec::new();
    if let Some(to) = to {
        acts.extend([Act::Pointer(to), Act::Frames(3)]);
    }
    acts.extend(wheel_acts(by));
    Ok(acts)
}

/// Notes that a key step types into the open search box rather than reaching the
/// view.
fn note_typing(drive: &mut Drive) {
    let line = &drive.lines[drive.step.unwrap_or_default()].text;
    let note = format!(
        "the search box has the keyboard: `{}` types into it rather than reaching the \
         view (`type` for text; `key esc` closes the box)",
        line.trim_start_matches("key ")
    );
    drive.notes.push(note);
}

/// A drag with a button held: the left orbits, the right pans. It starts where the
/// pointer is, when that is on open graph, else in the graph's middle.
fn drag(drive: &Drive, view: &View, by: Vec2, button: MouseButton) -> Vec<Act> {
    let start = drive
        .pointer
        .filter(|&p| open_ground(view, p))
        .unwrap_or_else(|| graph_middle(view));
    let piece = by / f32::from(DRAG_PIECES);
    let mut acts = vec![
        Act::Pointer(start),
        Act::Frames(2),
        Act::Mouse(button, ButtonState::Pressed),
        Act::Frames(1),
    ];
    acts.extend((0..DRAG_PIECES).map(|_| Act::Nudge(piece)));
    acts.push(Act::Mouse(button, ButtonState::Released));
    acts.push(Act::Frames(1));
    acts
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
        Expect::Aside(unit) => same("function set aside", explorer.aside(), wanted(unit)?),
        Expect::Named(unit) => {
            let tile = drive.names.unit(unit)?;
            if view.screen.labels.iter().any(|&(_, t)| t == tile) {
                Ok(())
            } else {
                Err(format!("{} has no name drawn", name(Some(tile))))
            }
        }
        Expect::Drawn(unit) => view.find(drive.names.unit(unit)?).map(|_| ()),
        // Only what the step before said, not a notice left from earlier.
        Expect::Notice(words) => {
            let words = words.to_lowercase();
            match said_since(drive, view) {
                Some(text) if text.to_lowercase().contains(&words) => Ok(()),
                Some(text) => Err(format!("the step before said `{text}`, not `{words}`")),
                None => Err(format!(
                    "the step before said nothing; no notice says `{words}`"
                )),
            }
        }
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
            // Judged as the steps since may move things: a click expanded in a
            // journey may re-centre, a depth change draw the camera back, X in a
            // journey move the camera but none of its dots; any other step is held
            // to moving nothing.
            let kinds = drive
                .kinds
                .iter()
                .map(|&k| match k {
                    Kind::Click | Kind::Depth | Kind::Expand => k,
                    _ => Kind::Still,
                })
                .chain([Kind::Still])
                .collect();
            let still = Since {
                kinds,
                by_hand: drive.by_hand,
                ..Since::default()
            };
            let result = checks::run(&scene_now(drive, view), Some(before), &still);
            verdict_of("nothing-moved", &result["nothing-moved"], true)
        }
        Expect::Check { name, pass } => {
            let result = checks::run(
                &scene_now(drive, view),
                drive.last_scene.as_ref(),
                &since(drive),
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
/// lines, the steps that failed, the notes on steps that did not, and each snap's
/// checks.
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
        "notes": drive.log.iter().filter(|s| s.get("note").is_some()).map(|s| json!({
            "step": s["step"], "line": s["line"], "do": s["do"], "note": s["note"],
        })).collect::<Vec<_>>(),
        "snaps": snaps,
        // Snapshots with no PNG: macOS drew nothing into the window. Their JSON and
        // checks hold all the same; the session's verdict leaves them out.
        "window_not_drawn": drive.not_drawn.iter().chain(&drive.black).collect::<Vec<_>>(),
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

/// Where to double-click a module's sphere: of the spots where no sphere lies in
/// front of it, the one farthest from its functions and names, and whether it is
/// clear of them; where none is, the spot farthest from the names alone, which may be
/// on a function.
fn module_point(view: &View, cluster: usize) -> Result<(Vec2, bool), String> {
    let module = view.explorer.cluster_name(cluster);
    let nucleus = view.explorer.nucleus();
    let ball = nucleus
        .ball(cluster)
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
    let spots: Vec<Vec2> = spots
        .filter(|&at| open_ground(view, at) && view.module_at(at) == Some(cluster))
        .collect();
    let best = |drawn: &[Drawn]| {
        spots
            .iter()
            .map(|&at| (clearance(view, drawn, at), at))
            .filter(|(room, _)| *room >= CLEAR_SPOT)
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, at)| at)
    };
    best(&drawn)
        .map(|at| (at, true))
        .or_else(|| best(&[]).map(|at| (at, false)))
        .ok_or_else(|| format!("no spot on module {module}'s sphere is clear of its names"))
}

/// Where to click on nothing: of the spots on the graph roomily clear of every
/// function, name and sphere (else of every function and name), the one nearest the
/// middle.
fn empty_point(view: &View) -> Result<Vec2, String> {
    let drawn = view.drawn();
    let n = view.explorer.nucleus();
    let spheres: Vec<(Vec2, f32)> = n
        .balls()
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

/// The journey bar a step names: by its number (`1.2`), else by its function (the
/// start's bar first), else by the name it reads.
fn bar_span(view: &View, names: &Names, bar: &str) -> Result<SpanId, String> {
    let explorer = &*view.explorer;
    if view.screen.bars.is_none() {
        return Err("the journey panel is not showing: no journey is open".to_owned());
    }
    let disclosed = explorer.disclosure();
    let spans: Vec<SpanId> = explorer
        .journey()
        .into_iter()
        .flat_map(|journey| journey.spans().map(|(id, _)| id))
        .filter(|&id| disclosed.shows(id))
        .collect();
    let track = explorer.track();
    let tile = (!bar_number(bar)).then(|| names.unit(bar).ok()).flatten();
    let found = if bar_number(bar) {
        spans.iter().find(|&&id| disclosed.number(id) == Some(bar))
    } else {
        tile.and_then(|tile| {
            let root = explorer.journey().map(codeviz_core::Journey::root);
            let of = |id: &&SpanId| track.tile(**id, explorer.map()) == Some(tile);
            spans
                .iter()
                .filter(of)
                .find(|&&id| Some(id) == root)
                .or_else(|| spans.iter().find(of))
        })
        .or_else(|| spans.iter().find(|&&id| explorer.label(id) == bar))
    };
    found.copied().ok_or_else(|| {
        let numbers: Vec<&str> = spans
            .iter()
            .filter_map(|&id| disclosed.number(id))
            .filter(|n| !n.is_empty())
            .collect();
        format!(
            "no bar is numbered or named {bar} at this depth; bars: {}",
            numbers.join(" ")
        )
    })
}

/// Where to click a journey bar: inside its left end, clear of the badge that unfolds
/// it.
fn bar_point(view: &View, span: SpanId, bar: &str) -> Result<Vec2, String> {
    let Some(drawn) = &view.screen.bars else {
        return Err("the journey panel is not showing: no journey is open".to_owned());
    };
    let Some(&(_, rect, clip, _)) = drawn.bars.iter().find(|(s, ..)| *s == span) else {
        return Err(format!("bar {bar} is not drawn"));
    };
    let shown = rect.intersect(clip);
    if !shown.is_positive() {
        return Err(format!("bar {bar} is scrolled out of the journey panel"));
    }
    let x = shown.min.x + (shown.width() / 2.0).min(12.0);
    Ok(Vec2::new(x, shown.center().y))
}

/// Where to click the bar of the start's caller `bar` names, in the row above the
/// start's bar, where it is drawn.
fn caller_bar_point(view: &View, names: &Names, bar: &str) -> Option<Vec2> {
    let tile = names.unit(bar).ok()?;
    let drawn = view.screen.bars.as_ref()?;
    let &(_, rect, _) = drawn.callers.iter().find(|(t, ..)| *t == tile)?;
    Some(Vec2::new(
        rect.min.x + (rect.width() / 2.0).min(12.0),
        rect.center().y,
    ))
}

/// Where to click the badge on a journey bar that unfolds what is folded under it.
fn chip_point(view: &View, span: SpanId, bar: &str) -> Result<Vec2, String> {
    let Some(drawn) = &view.screen.bars else {
        return Err("the journey panel is not showing: no journey is open".to_owned());
    };
    let folded = view.explorer.disclosure().folded(span);
    let Some(&(_, chip)) = drawn.chips.iter().find(|(s, _)| *s == span) else {
        return Err(if folded == 0 {
            format!("bar {bar} has nothing folded under it, so no badge")
        } else {
            format!("bar {bar} folds {folded} calls, but has no badge drawn (no room for one)")
        });
    };
    let room = (drawn.bars.iter())
        .find(|(s, ..)| *s == span)
        .map_or(drawn.content, |&(_, _, room, _)| room);
    let shown = chip.intersect(room);
    if !shown.is_positive() {
        return Err(format!(
            "bar {bar}'s badge is scrolled out of the journey panel"
        ));
    }
    let c = shown.center();
    Ok(Vec2::new(c.x, c.y))
}

/// Where to click the search box's row for `text`: the row that says it, else whose
/// function's id ends with it, else the first that holds it.
fn search_point(view: &View, text: &str) -> Result<Vec2, String> {
    let Some(search) = &view.screen.search else {
        return Err("the search box is not open (`key /` opens it)".to_owned());
    };
    let wanted = text.trim().to_lowercase();
    let rows = &search.rows;
    let row = rows
        .iter()
        .find(|r| r.text.to_lowercase() == wanted)
        .or_else(|| {
            rows.iter().find(|r| {
                let unit = r.unit.to_lowercase();
                unit == wanted || unit.ends_with(&format!("::{wanted}"))
            })
        })
        .or_else(|| {
            rows.iter()
                .find(|r| r.text.to_lowercase().contains(&wanted))
        })
        .ok_or_else(|| {
            let listed: Vec<&str> = rows.iter().take(12).map(|r| r.text.as_str()).collect();
            format!(
                "no search row says `{text}`; rows: {}{}",
                listed.join(" | "),
                if rows.len() > listed.len() {
                    " …"
                } else {
                    ""
                }
            )
        })?;
    let shown = row.rect.intersect(search.content);
    if !shown.is_positive() {
        return Err(format!(
            "the search row `{}` is scrolled out of view",
            row.text
        ));
    }
    let c = shown.center();
    Ok(Vec2::new(c.x, c.y))
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
