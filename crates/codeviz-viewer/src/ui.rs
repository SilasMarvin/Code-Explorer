//! egui around the graph: the top bar, and the journey docked along the foot; on the
//! canvas, names beside nodes, the key help on `H` and a search box on `/`.

use std::collections::{HashMap, HashSet};

use bevy::{camera::Viewport, prelude::*, window::PrimaryWindow};
use bevy_egui::{
    EguiContext, EguiContexts, EguiInput, EguiPreUpdateSet, EguiPrimaryContextPass,
    PrimaryEguiContext, egui,
};
use codeviz_ir::{Role, UnitId};

use crate::{
    camera::OrbitCamera,
    explorer::Explorer,
    graph_view::{self, CALLEE, CALLER, CURSOR, DEEPER},
    journey_panel::{self, JourneyPanel},
    nucleus::{self, Body, Flow},
    scene::{PALETTE, lane_color},
};

pub(crate) struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Search>()
            .init_resource::<KeyHelp>()
            .init_resource::<Chrome>()
            .add_systems(
                PreUpdate,
                keys_to_the_graph
                    .after(EguiPreUpdateSet::ProcessInput)
                    .before(EguiPreUpdateSet::BeginPass),
            )
            .add_systems(EguiPrimaryContextPass, (fallback_fonts, draw_ui).chain());
    }
}

/// The keyboard belongs to the graph unless a text box is being typed in. egui would
/// otherwise move keyboard focus onto the first button on Tab, after which it takes
/// every key: the graph's shortcuts go dead and Space or Enter press that button. So
/// Tab is kept from egui, and any other widget holding focus lets go of it.
fn keys_to_the_graph(
    mut contexts: Query<(&mut EguiContext, &mut EguiInput), With<PrimaryEguiContext>>,
) {
    for (mut context, mut input) in &mut contexts {
        let ctx = context.get_mut();
        if ctx.text_edit_focused() {
            continue;
        }
        input.events.retain(|event| {
            !matches!(
                event,
                egui::Event::Key {
                    key: egui::Key::Tab,
                    ..
                }
            )
        });
        ctx.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
    }
}

/// egui's proportional font has no arrows; its monospace one does, so it fills in.
fn fallback_fonts(mut contexts: EguiContexts, mut done: Local<bool>) -> Result {
    if *done {
        return Ok(());
    }
    let mut fonts = egui::FontDefinitions::default();
    let monospace = fonts
        .families
        .get(&egui::FontFamily::Monospace)
        .and_then(|family| family.first())
        .cloned();
    if let (Some(name), Some(family)) = (
        monospace,
        fonts.families.get_mut(&egui::FontFamily::Proportional),
    ) {
        family.push(name);
    }
    contexts.ctx_mut()?.set_fonts(fonts);
    *done = true;
    Ok(())
}

/// Whether the key reference is showing.
#[derive(Resource, Debug, Default)]
pub(crate) struct KeyHelp(pub(crate) bool);

#[derive(Resource, Debug, Default)]
struct Search {
    query: String,
}

pub(crate) const MUTED: egui::Color32 = egui::Color32::from_rgb(140, 148, 165);

/// Whether the search box is open.
#[derive(Resource, Default)]
pub(crate) struct Chrome {
    pub(crate) search: bool,
}

#[expect(clippy::too_many_arguments, reason = "a Bevy system")]
fn draw_ui(
    mut contexts: EguiContexts,
    mut explorer: ResMut<Explorer>,
    mut search: ResMut<Search>,
    mut panel: ResMut<JourneyPanel>,
    mut help: ResMut<KeyHelp>,
    mut chrome: ResMut<Chrome>,
    mut cameras: Query<(&mut Camera, &GlobalTransform, &mut OrbitCamera)>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut moved: Local<u64>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let mut root_ui = egui::Ui::new(
        ctx.clone(),
        "codeviz".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.viewport_rect()),
    );

    let top = top_bar(&mut root_ui, &mut explorer, &mut chrome);

    let screen = ctx.viewport_rect();
    let central =
        egui::Rect::from_min_max(egui::pos2(screen.min.x, screen.min.y + top), screen.max);
    // Browsing, no journey is open, so its panel is not drawn.
    let (graph_area, panel_area) = if explorer.browsing() {
        (central, None)
    } else {
        let (graph, panel) = journey_panel::split(central, journey_panel::rows(&explorer));
        (graph, Some(panel))
    };
    if let Ok((mut camera, transform, orbit)) = cameras.single_mut() {
        fit_viewport(&mut camera, &window, graph_area);
        let to_screen = |position: Vec3| {
            camera
                .world_to_viewport(transform, position)
                .ok()
                .map(|p| egui::pos2(p.x, p.y))
                .filter(|p| graph_area.contains(*p))
        };
        let painter = ctx
            .layer_painter(egui::LayerId::background())
            .with_clip_rect(graph_area);
        // A label new to the scene keeps its place once nothing moves any more.
        let settled = !orbit.easing() && *moved == explorer.moved();
        *moved = explorer.moved();
        draw_graph_labels(
            &painter,
            &to_screen,
            &explorer,
            (transform.up().into(), transform.right().into()),
            settled,
        );
        if chrome.search {
            search_box(
                ctx,
                graph_area,
                &mut explorer,
                &mut search,
                &mut chrome.search,
            );
        }
        key_hint(ctx, graph_area, Showing::of(&explorer));
        if help.0 {
            // Down to the window's foot, over the journey panel where it must, so the
            // whole list fits rather than scrolls.
            let room = egui::Rect::from_min_max(
                graph_area.min,
                egui::pos2(graph_area.max.x, ctx.content_rect().max.y),
            );
            key_help(ctx, room, &mut help.0, Showing::of(&explorer));
        }
        if let Some(area) = panel_area {
            let tethers = journey_panel::draw(ctx, area, &mut explorer, &mut panel);
            // Not over the key help, which would read as crossed out.
            if !help.0 {
                draw_tethers(ctx, &tethers, &explorer, &to_screen);
            }
        }
    }
    Ok(())
}

/// The bar along the top: Back and Forward and which mode the view is in on the left;
/// search and what the focus is on the right. Returns how tall it is.
fn top_bar(root_ui: &mut egui::Ui, explorer: &mut Explorer, chrome: &mut Chrome) -> f32 {
    egui::Panel::top("stack")
        .show(root_ui, |ui| {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .selectable_label(chrome.search, "Search")
                        .on_hover_text("Find a function (/)")
                        .clicked()
                    {
                        chrome.search = !chrome.search;
                    }
                    aside_chip(ui, explorer);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        history_buttons(ui, explorer);
                        mode_label(ui, explorer);
                    });
                });
            });
        })
        .response
        .rect
        .height()
}

/// The bar under the pointer tied to the node it stands for.
fn draw_tethers(
    ctx: &egui::Context,
    tethers: &journey_panel::Tethers,
    explorer: &Explorer,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
) {
    let hovered = explorer
        .hovered_tile
        .and_then(|tile| explorer.tile_point(tile))
        .and_then(to_screen);
    if let (Some(bar), Some(node)) = (tethers.hovered, hovered) {
        tether(ctx, bar, node);
    }
}

/// A line from the bar under the pointer up to its node in the graph, ringing it.
fn tether(ctx: &egui::Context, bar: egui::Rect, node: egui::Pos2) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("tether"),
    ));
    let from = egui::pos2(bar.center().x, bar.min.y);
    let color = egui::Color32::WHITE;
    painter.line_segment(
        [from, node],
        egui::Stroke::new(1.5, color.gamma_multiply(0.86)),
    );
    painter.circle_filled(from, 2.5, color);
    painter.circle_stroke(node, 9.0, egui::Stroke::new(1.5, color));
}

/// Back and forward through where the view has been, as a browser's: always there,
/// greyed out with nowhere to go.
fn history_buttons(ui: &mut egui::Ui, explorer: &mut Explorer) {
    if ui
        .add_enabled(explorer.can_go_back(), egui::Button::new("← Back"))
        .on_hover_text("Back before the last click, bar, Enter or walk (Backspace)")
        .on_disabled_hover_text("Nowhere to go back to yet (Backspace)")
        .clicked()
    {
        explorer.go_back();
    }
    if ui
        .add_enabled(explorer.can_go_forward(), egui::Button::new("Forward →"))
        .on_hover_text("Forward again (Shift+Backspace)")
        .on_disabled_hover_text("Nothing gone back from (Shift+Backspace)")
        .clicked()
    {
        explorer.go_forward();
    }
    ui.separator();
}

/// Which mode the view is in, always in the same place: `Browsing`, or `Journey ·`
/// and where it starts.
fn mode_label(ui: &mut egui::Ui, explorer: &Explorer) {
    let (text, hint) = match explorer.root().filter(|_| !explorer.browsing()) {
        Some(root) => (
            format!("Journey · {}", explorer.unit_name(root)),
            "A journey is open: a click on a function it runs moves its cursor; \
             Esc leaves it for browsing",
        ),
        None => (
            "Browsing".to_owned(),
            "No journey is open: a click selects a function and moves nothing; \
             Enter or Shift-click opens its journey",
        ),
    };
    egui::Frame::new()
        .fill(egui::Color32::from_rgba_premultiplied(30, 34, 46, 255))
        .corner_radius(6.0)
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text)
                    .strong()
                    .color(egui::Color32::WHITE),
            )
            .on_hover_text(hint);
        });
    ui.separator();
}

/// A box to find a function by name, over the top of the graph: Enter or a click
/// opens it, Escape closes the box.
fn search_box(
    ctx: &egui::Context,
    area: egui::Rect,
    explorer: &mut Explorer,
    search: &mut Search,
    open: &mut bool,
) {
    let system = explorer.system();
    let rows: Vec<(UnitId, String, String)> = if search.query.trim().is_empty() {
        let mut rows: Vec<_> = system
            .entries()
            .iter()
            .map(|e| (e.unit.clone(), e.label.clone(), e.kind.clone()))
            .collect();
        rows.sort_by(|a, b| a.1.cmp(&b.1));
        rows
    } else {
        let entries = system
            .entries()
            .iter()
            .map(|e| (&e.unit, e.label.clone(), &e.kind));
        let units = system
            .units()
            .filter(|u| u.role == Role::Invocable)
            .map(|u| (&u.id, explorer.unit_name(&u.id), &u.kind));
        let mut ranked: Vec<(u8, UnitId, String, String)> = entries
            .chain(units)
            .filter_map(|(id, name, kind)| {
                let rank = search_rank(&search.query, id.as_str(), &name)?;
                Some((rank, id.clone(), name, kind.clone()))
            })
            .collect();
        ranked.sort_by(|a, b| (a.0, a.2.len(), &a.2).cmp(&(b.0, b.2.len(), &b.2)));
        ranked.dedup_by(|a, b| a.1 == b.1);
        ranked
            .into_iter()
            .take(200)
            .map(|(_, id, name, kind)| (id, name, kind))
            .collect()
    };
    let mut chosen = None;
    egui::Area::new("search".into())
        .pivot(egui::Align2::CENTER_TOP)
        .fixed_pos(egui::pos2(area.center().x, area.min.y + 10.0))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_premultiplied(11, 13, 19, 240))
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(60)))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_width(420.0);
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut search.query)
                            .hint_text("search functions…")
                            .desired_width(f32::INFINITY),
                    );
                    field.request_focus();
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        chosen = rows.first().map(|row| row.0.clone());
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        *open = false;
                    }
                    if rows.is_empty() {
                        ui.label(egui::RichText::new("no function matches").color(MUTED));
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("units")
                        .max_height(260.0)
                        .show(ui, |ui| {
                            for (id, title, kind) in &rows {
                                ui.horizontal(|ui| {
                                    if ui
                                        .selectable_label(explorer.root() == Some(id), title)
                                        .on_hover_text(id.as_str())
                                        .clicked()
                                    {
                                        chosen = Some(id.clone());
                                    }
                                    ui.label(egui::RichText::new(kind).small().color(MUTED));
                                });
                            }
                        });
                });
        });
    // Browsing, the function found is selected where it sits, as a click selects it;
    // in a journey, a journey opens there.
    if let Some(id) = chosen {
        match explorer.map().tile_of(&id) {
            Some(tile) if explorer.browsing() => explorer.focus_tile(tile, None),
            Some(tile) => explorer.open_at(tile),
            None => explorer.set_root(id),
        }
        *open = false;
    }
}

/// How well `query` finds a function, best first, or `None` when it does not: its
/// name exactly, then its name's start, then anywhere in its path, then each word of
/// the query in order along the path. `.` and `::` are the same, so `orders.place`
/// finds `services::orders::place`.
fn search_rank(query: &str, id: &str, name: &str) -> Option<u8> {
    let normal = |text: &str| text.to_lowercase().replace("::", ".");
    let query = normal(query.trim());
    let name = normal(name);
    // The path without its producer's prefix, as in `rust:`.
    let path = normal(id.split_once(':').map_or(id, |(_, path)| path));
    if name == query || path == query || path.ends_with(&format!(".{query}")) {
        return Some(0);
    }
    if name.starts_with(&query) || name.contains(&format!(".{query}")) {
        return Some(1);
    }
    if path.contains(&query) || name.contains(&query) {
        return Some(2);
    }
    let mut rest = path.as_str();
    for word in query
        .split(|c: char| c == '.' || c.is_whitespace())
        .filter(|w| !w.is_empty())
    {
        let at = rest.find(word)?;
        rest = &rest[at + word.len()..];
    }
    Some(3)
}

/// With a function selected off the journey, a chip that says so; a click on it, as
/// Enter does, starts a journey there.
fn aside_chip(ui: &mut egui::Ui, explorer: &mut Explorer) {
    if explorer.aside().is_none() {
        return;
    }
    let chip = egui::Button::new(
        egui::RichText::new("not on this journey · Enter starts one here")
            .size(13.0)
            .color(egui::Color32::WHITE),
    )
    .fill(egui::Color32::from_rgb(70, 52, 104))
    .stroke(egui::Stroke::new(1.0, CALLER_TEXT))
    .corner_radius(8.0);
    if ui
        .add(chip)
        .on_hover_text(
            "Selected off the journey, in place. Enter, or a click here, starts a journey \
             here; an arrow or Esc goes back to the journey's cursor",
        )
        .clicked()
    {
        explorer.trace_from_focus();
    }
}

/// Restricts 3D rendering to the area between the panels.
fn fit_viewport(camera: &mut Camera, window: &Window, central: egui::Rect) {
    let scale = window.scale_factor();
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped to non-negative pixel counts"
    )]
    let to_physical = |logical: f32| (logical * scale).max(0.0).round() as u32;
    let position = UVec2::new(to_physical(central.min.x), to_physical(central.min.y));
    let size = UVec2::new(to_physical(central.width()), to_physical(central.height()))
        .min(window.physical_size().saturating_sub(position));
    if size.x == 0 || size.y == 0 {
        return;
    }
    let unchanged = camera
        .viewport
        .as_ref()
        .is_some_and(|v| v.physical_position == position && v.physical_size == size);
    if !unchanged {
        camera.viewport = Some(Viewport {
            physical_position: position,
            physical_size: size,
            ..default()
        });
    }
}

/// The journey's entry route when it starts at one, else its root's name.
pub(crate) fn journey_title(explorer: &Explorer) -> String {
    let Some(root) = explorer.root() else {
        return String::new();
    };
    explorer
        .system()
        .entries()
        .iter()
        .find(|e| &e.unit == root)
        .map_or_else(|| explorer.unit_name(root), |e| e.label.clone())
}

const CALLER_TEXT: egui::Color32 = egui::Color32::from_rgb(206, 170, 255);
const CALLEE_TEXT: egui::Color32 = egui::Color32::from_rgb(130, 196, 255);
const DEEPER_TEXT: egui::Color32 = egui::Color32::from_rgb(255, 204, 128);
const CURSOR_TEXT: egui::Color32 = egui::Color32::from_rgb(255, 160, 210);
/// Text on a bar or a badge filled with a node's color.
const ON_FILL: egui::Color32 = egui::Color32::from_rgb(14, 16, 22);
const PILL: egui::Color32 = egui::Color32::from_rgba_premultiplied(9, 11, 16, 205);
/// The dotted thread from a name to its node: grey, as no call is drawn.
const THREAD: egui::Color32 = egui::Color32::from_rgba_premultiplied(120, 124, 134, 190);

/// A function's name, or the route of an entry point.
pub(crate) fn node_name(explorer: &Explorer, tile: usize) -> String {
    let Some(unit) = explorer.tile_unit(tile) else {
        return String::new();
    };
    explorer
        .system()
        .entries()
        .iter()
        .find(|e| &e.unit == unit)
        .map_or_else(
            || {
                explorer
                    .system()
                    .unit(unit)
                    .map_or_else(|| unit.to_string(), |u| u.name.clone())
            },
            |e| e.label.clone(),
        )
}

/// What a graph label names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Named {
    Node(usize),
    Group(usize),
}

/// Tier, then nearer the journey's start (a module first or last within its tier),
/// then bigger first, then a stable tiebreak.
type LabelRank = (u8, usize, std::cmp::Reverse<usize>, usize);

/// Where a label may go around its node, in order of preference.
const SIDES: [egui::Align2; 4] = [
    egui::Align2::LEFT_CENTER,
    egui::Align2::RIGHT_CENTER,
    egui::Align2::CENTER_BOTTOM,
    egui::Align2::CENTER_TOP,
];

/// Room kept around a label's text: its pill.
const PILL_PAD: egui::Vec2 = egui::vec2(5.0, 2.0);
/// How fast a label slides to a new side: the share of the way left it covers in a
/// second is `1 - e^-SLIDE`.
const SLIDE: f32 = 14.0;
/// Seconds a label takes to fade in or out.
const FADE: f32 = 0.18;
/// How far along its side, in steps, a name that must show is nudged when every side
/// is taken, nearest first: no further than a thread to it still reads as its own.
const NUDGES: [i8; 6] = [1, -1, 2, -2, 3, -3];
/// Further along its side, where a name that must show goes in a crowd when nearer
/// would lie nearer another node than its own: its thread ties it to its node.
const THREADED: [i8; 12] = [1, -1, 2, -2, 3, -3, 4, -4, 5, -5, 6, -6];
/// How far, in points from its node's edge, a name may sit and still read as its
/// node's. Past it a name is better left to the pointer, and counted as unnamed;
/// a name held further off takes a nearer place when there is one.
const NEAR: f32 = 40.0;
/// How much of a held name others may cover, as when its badge grows by a number,
/// before it is placed elsewhere for now.
const COVERED_SHARE: f32 = 0.25;

/// Where a label sits against its node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Spot {
    /// Index into [`SIDES`].
    side: usize,
    /// How far along that side it was nudged, in steps.
    shift: i8,
    /// Whether it lies over its node's middle, its last resort in a crowd.
    over: bool,
}

impl Spot {
    fn beside(side: usize, shift: i8) -> Self {
        Self {
            side,
            shift,
            over: false,
        }
    }

    fn over(side: usize) -> Self {
        Self {
            side,
            shift: 0,
            over: true,
        }
    }

    /// Where a label of `extent` sits here, by a node at `node` of radius `edge`.
    fn rect(self, node: egui::Pos2, edge: f32, extent: egui::Vec2) -> egui::Rect {
        let gap = if self.over { 0.0 } else { edge + 6.0 };
        let (anchor, step) = match SIDES[self.side] {
            egui::Align2::LEFT_CENTER => (
                node + egui::vec2(gap, 0.0),
                egui::vec2(0.0, extent.y + PILL_PAD.y * 3.0),
            ),
            egui::Align2::RIGHT_CENTER => (
                node - egui::vec2(gap, 0.0),
                egui::vec2(0.0, extent.y + PILL_PAD.y * 3.0),
            ),
            egui::Align2::CENTER_BOTTOM => (
                node - egui::vec2(0.0, gap),
                egui::vec2(extent.x / 2.0 + PILL_PAD.x, 0.0),
            ),
            _ => (
                node + egui::vec2(0.0, gap),
                egui::vec2(extent.x / 2.0 + PILL_PAD.x, 0.0),
            ),
        };
        SIDES[self.side]
            .anchor_size(anchor, extent)
            .translate(step * f32::from(self.shift))
    }
}

/// How hard a label is placed when no place beside its node is free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Force {
    /// Not at all: it goes unnamed.
    Never,
    /// Nudged along a side, or wherever it misleads least, but never over another
    /// name.
    Free,
    /// Wherever it misleads least, over another name if it must.
    Always,
}

/// A label as last drawn, so the next frame eases it instead of jumping it.
#[derive(Clone)]
struct Shown {
    at: Spot,
    /// Where its text sits, from its node.
    offset: egui::Vec2,
    opacity: f32,
    galley: std::sync::Arc<egui::Galley>,
    stroke: Option<egui::Color32>,
    /// How far its node's edge is from its middle.
    edge: f32,
}

/// Every label drawn last frame.
type ShownLabels = HashMap<Named, Shown>;

/// How long a label keeps the place it first took: browsing, for as long as the map
/// is browsed; in a journey, for as long as that journey is open, gathered or expanded
/// round its start, and again when Back or Esc returns to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Scene {
    Browse,
    Journey {
        start: Option<usize>,
        expanded: bool,
    },
}

/// Where each label went when it first showed, by scene.
type SpotBook = HashMap<Scene, HashMap<Named, Spot>>;

/// Scenes remembered at most; past it the book starts again.
const SCENES_KEPT: usize = 48;

fn spot_book_id() -> egui::Id {
    egui::Id::new("graph_label_spots")
}

/// Places labels so none overlaps another or a node that matters, and draws them
/// eased from where they were. Gathered, a label keeps the place it first took for as
/// long as its scene lasts; otherwise it keeps its side while that side stays free. It
/// slides when it must change sides, and fades in and out instead of popping.
struct Placer<'a> {
    painter: &'a egui::Painter,
    /// What labels keep off: first what was there before any label, then the
    /// labels placed.
    taken: Vec<egui::Rect>,
    /// How many of `taken` were there before any label.
    blocked: usize,
    /// How many of those first are cards over the graph rather than nodes.
    cards: usize,
    /// Whether module names are landmarks, kept off cards and other names alone and
    /// not off nodes, so that what a click lights never moves one: browsing.
    landmarks: bool,
    /// Every named node's disc, by tile: no function's label sits nearer another of
    /// them than its own.
    nodes: Vec<(usize, egui::Pos2, f32)>,
    /// The lit nodes no name may cover, not even one held where it first showed:
    /// all but those only the pointer lights.
    blocking: Vec<(usize, egui::Pos2, f32)>,
    last: ShownLabels,
    next: ShownLabels,
    /// Where each label placed this frame sits.
    anchors: HashMap<Named, egui::Pos2>,
    /// The share of the way to its new side a label covers this frame.
    slide: f32,
    /// How much opacity a label gains or loses this frame.
    fade: f32,
    /// Labels painted over all the others, the last on top.
    top: Vec<Named>,
    /// The journey number each function's label leads with, so a click on the label
    /// goes to that call.
    numbers: HashMap<Named, String>,
    /// Where labels went when they first showed in this scene: they stay there, and
    /// what shows later keeps off them.
    held: HashMap<Named, Spot>,
    /// Where labels new to the scene went this frame, to hold from now on.
    chosen: HashMap<Named, Spot>,
    /// Whether the view rests, so a new label's place is for keeps: while the camera
    /// or the spheres still move, it is only for now.
    keep: bool,
    /// The names drawn come what may: the cursor's and the selection's. Where one
    /// has a place of its own it stays there, so a click or a step never moves it;
    /// where it has none it is drawn right beside its node, over the rest.
    beside: HashSet<Named>,
    /// Those of them drawn only for being the cursor or the selection, which would
    /// have gone unnamed otherwise and so take no room.
    overlaid: HashSet<Named>,
    /// Held names the view has closed up on since, to place for now once every held
    /// name that still has room is placed, and before any new one.
    displaced: Vec<Displaced>,
    /// Names placed for now, away from where they are held, so they return there.
    for_now: HashSet<Named>,
}

/// A held name with no room where it is held, as it was to be placed.
struct Displaced {
    key: Named,
    node: egui::Pos2,
    edge: f32,
    job: egui::text::LayoutJob,
    stroke: Option<egui::Color32>,
    force: Force,
}

impl<'a> Placer<'a> {
    fn new(
        painter: &'a egui::Painter,
        taken: Vec<egui::Rect>,
        nodes: Vec<(usize, egui::Pos2, f32)>,
    ) -> Self {
        let dt = painter.ctx().input(|i| i.stable_dt).min(0.1);
        Self {
            painter,
            blocked: taken.len(),
            cards: 0,
            landmarks: false,
            taken,
            nodes,
            blocking: Vec::new(),
            last: painter
                .ctx()
                .data_mut(|d| d.get_temp::<ShownLabels>(Self::id()))
                .unwrap_or_default(),
            next: HashMap::new(),
            anchors: HashMap::new(),
            slide: 1.0 - (-SLIDE * dt).exp(),
            fade: dt / FADE,
            top: Vec::new(),
            numbers: HashMap::new(),
            held: HashMap::new(),
            chosen: HashMap::new(),
            keep: false,
            beside: HashSet::new(),
            overlaid: HashSet::new(),
            displaced: Vec::new(),
            for_now: HashSet::new(),
        }
    }

    fn id() -> egui::Id {
        egui::Id::new("graph_labels")
    }

    /// Places `job` on a side of `node` (a disc of `edge` points) where it overlaps
    /// nothing already placed. A label the scene already holds goes where it went when
    /// it first showed, whatever has shown since: what is new keeps off it, not the
    /// reverse. Only where the view has closed up since, so that names placed before
    /// it would cover much of it there, is it placed elsewhere for now, after every
    /// held name with room, unless `force` is [`Force::Always`]; it goes back when
    /// there is room again. A new label goes where [`Placer::new_spot`] finds room. A
    /// held place more than [`NEAR`] from its node, or over another lit node, gives
    /// way to a nearer one.
    /// The cursor's and the selection's names stay where they are placed; one with no
    /// room of its own is drawn right beside its node, over others, but takes no
    /// room, so where the cursor is never decides what else is named. Returns whether
    /// it was placed, or put off until the held names are.
    fn place(
        &mut self,
        key: Named,
        node: egui::Pos2,
        edge: f32,
        job: egui::text::LayoutJob,
        stroke: Option<egui::Color32>,
        force: Force,
    ) -> bool {
        let galley = self.painter.layout_job(job);
        let extent = galley.size();
        let spot = |at: Spot| at.rect(node, edge, extent);
        let previous = self.last.get(&key);
        let held = self.held.get(&key).copied();
        // A held name too far off to read as its node's heals: it takes a nearer
        // place where there is one.
        let far = held.is_some_and(|at| !near(spot(at), node, edge));
        let at = match held {
            Some(at) if far => self
                .new_spot(key, node, edge, extent, force)
                .or((force == Force::Always).then_some(at)),
            Some(at)
                if !self.covers_node(key, spot(at))
                    && (force == Force::Always || self.has_room(spot(at))) =>
            {
                Some(at)
            }
            Some(_) => {
                self.displaced.push(Displaced {
                    key,
                    node,
                    edge,
                    job: (*galley.job).clone(),
                    stroke,
                    force,
                });
                return true;
            }
            None => self.new_spot(key, node, edge, extent, force),
        };
        let drawn = match at {
            Some(at) => at,
            None if self.beside.contains(&key) => {
                let last = previous.map(|p| p.at);
                self.beside_spot(key, node, edge, extent, [last, held])
            }
            None => return false,
        };
        if let Some(at) = at {
            self.taken.push(spot(at).expand2(PILL_PAD));
            if held != Some(at) && self.keep && !self.for_now.contains(&key) {
                self.chosen.insert(key, at);
            }
        } else {
            self.overlaid.insert(key);
        }
        let target = spot(drawn).min - node;
        let (offset, opacity) = match previous {
            Some(p) => (
                p.offset + (target - p.offset) * self.slide,
                (p.opacity + self.fade).min(1.0),
            ),
            // A new label fades in where it belongs.
            None => (target, self.fade.min(1.0)),
        };
        self.anchors.insert(key, node);
        self.next.insert(
            key,
            Shown {
                at: drawn,
                offset,
                opacity,
                galley,
                stroke,
                edge,
            },
        );
        true
    }

    /// Whether a held name at `rect` is inside the graph and the names placed so far
    /// cover little enough of it: as when its badge grows by a number.
    fn has_room(&self, rect: egui::Rect) -> bool {
        let covered: f32 = self
            .names_and_cards()
            .map(|r| r.intersect(rect))
            .filter(egui::Rect::is_positive)
            .map(|r| r.area())
            .sum();
        self.painter.clip_rect().contains_rect(rect) && covered <= rect.area() * COVERED_SHARE
    }

    /// Whether a function's name at `rect` would lie over another lit node's middle;
    /// a landmark's never counts, as module names do not keep off nodes.
    fn covers_node(&self, key: Named, rect: egui::Rect) -> bool {
        if self.landmark(key) {
            return false;
        }
        let own = match key {
            Named::Node(tile) => Some(tile),
            Named::Group(_) => None,
        };
        let pill = rect.expand2(PILL_PAD);
        self.blocking
            .iter()
            .any(|&(tile, at, radius)| Some(tile) != own && pill.distance_to_pos(at) < radius * 0.5)
    }

    /// Where a label new to the scene goes: the side it had last frame if that is
    /// still free, else the first free one. When `force` asks and none is, it is
    /// nudged along a side to the nearest free place, then further, on a thread, else
    /// put where it misleads least, which only [`Force::Always`] lets lie over another
    /// name. Beside its node or nudged, a function's label goes nowhere it would lie
    /// nearer another node than its own, so it never reads as the other's.
    fn new_spot(
        &self,
        key: Named,
        node: egui::Pos2,
        edge: f32,
        extent: egui::Vec2,
        force: Force,
    ) -> Option<Spot> {
        let spot = |at: Spot| at.rect(node, edge, extent);
        // Inside the graph, so no label is cut off at its edge.
        let bounds = self.painter.clip_rect();
        let against = self.against(key);
        let clear = |rect: egui::Rect, pad: egui::Vec2| {
            bounds.contains_rect(rect.expand2(pad))
                && !against.iter().any(|r| r.intersects(rect.expand2(pad)))
                && !self.covers_node(key, rect)
        };
        let fits = |rect: egui::Rect, pad: egui::Vec2| {
            clear(rect, pad) && self.misread(key, rect, node, edge) == 0
        };
        // Near enough its node to read as its own; a name that must show goes
        // further only when nothing nearer is free.
        let close = |at: Spot| near(spot(at), node, edge);
        // A label holds its side until something actually covers it; a new side must
        // be clear with room to spare. That margin keeps a label from flipping back
        // and forth while the view still moves.
        let kept = self
            .last
            .get(&key)
            .map(|p| p.at)
            .filter(|&at| !at.over && close(at) && fits(spot(at), egui::Vec2::ZERO));
        let along = |shifts: &'static [i8]| {
            shifts
                .iter()
                .flat_map(|&shift| (0..SIDES.len()).map(move |side| Spot::beside(side, shift)))
        };
        let free =
            |shifts: &'static [i8]| along(shifts).find(|&at| close(at) && fits(spot(at), PILL_PAD));
        // The nearest clear place a thread can reach.
        let threaded = || {
            along(&THREADED)
                .map(|at| (spot(at).distance_to_pos(node), at))
                .filter(|&(reach, at)| reach <= NEAR + edge && clear(spot(at), PILL_PAD))
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, at)| at)
        };
        // A label nudged off its node goes back beside it as soon as there is room.
        kept.filter(|at| at.shift == 0)
            .or_else(|| free(&[0]))
            .or(kept)
            .or_else(|| (force != Force::Never).then(|| free(&NUDGES)).flatten())
            .or_else(|| (force != Force::Never).then(threaded).flatten())
            .or_else(|| {
                let (at, over_names) = self.least_misleading(key, node, edge, extent);
                match force {
                    Force::Always => Some(at),
                    Force::Free if !over_names && close(at) && !self.covers_node(key, spot(at)) => {
                        Some(at)
                    }
                    Force::Free | Force::Never => None,
                }
            })
    }

    /// Places the held names the view has closed up on since, for now, as new ones
    /// are placed: they go back where they are held when there is room again.
    fn place_displaced(&mut self) {
        for displaced in std::mem::take(&mut self.displaced) {
            let key = displaced.key;
            let held = self.held.remove(&key);
            self.for_now.insert(key);
            self.place(
                key,
                displaced.node,
                displaced.edge,
                displaced.job,
                displaced.stroke,
                displaced.force,
            );
            if let Some(at) = held {
                self.held.insert(key, at);
            }
        }
    }

    /// The cards over the graph and the names placed so far.
    fn names_and_cards(&self) -> impl Iterator<Item = &egui::Rect> {
        self.taken[..self.cards]
            .iter()
            .chain(&self.taken[self.blocked..])
    }

    /// Where the cursor's or the selection's name goes, right beside its node: on the
    /// side it was drawn on last, else the side the scene holds it on, else the first
    /// side, that lies over nothing; else wherever it covers least of the names there.
    fn beside_spot(
        &self,
        key: Named,
        node: egui::Pos2,
        edge: f32,
        extent: egui::Vec2,
        before: [Option<Spot>; 2],
    ) -> Spot {
        let bounds = self.painter.clip_rect();
        // On the side it was on before, if it can, so it moves least.
        let candidates: Vec<Spot> = before
            .into_iter()
            .flatten()
            .filter(|at| !at.over)
            .map(|at| Spot::beside(at.side, 0))
            .chain((0..SIDES.len()).map(|side| Spot::beside(side, 0)))
            .collect();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "an area in whole points, at least 0"
        )]
        let under = |rect: egui::Rect| {
            let pill = rect.expand2(PILL_PAD);
            self.names_and_cards()
                .map(|r| r.intersect(pill))
                .filter(egui::Rect::is_positive)
                .map(|r| r.area() as u32)
                .sum::<u32>()
        };
        candidates
            .iter()
            .copied()
            .min_by_key(|&at| {
                let rect = at.rect(node, edge, extent);
                (
                    under(rect),
                    !bounds.contains_rect(rect),
                    self.misread(key, rect, node, edge),
                )
            })
            .unwrap_or(Spot::beside(0, 0))
    }

    /// Takes a label placed this frame off again: it fades out, and its place is not
    /// held.
    fn unplace(&mut self, key: Named) {
        self.next.remove(&key);
        self.chosen.remove(&key);
    }

    /// Whether `key` names a module as a landmark.
    fn landmark(&self, key: Named) -> bool {
        self.landmarks && matches!(key, Named::Group(_))
    }

    /// What a label of `key` keeps off: everything taken, or for a landmark the cards
    /// and the names placed.
    fn against(&self, key: Named) -> Vec<egui::Rect> {
        if self.landmark(key) {
            self.taken[..self.cards]
                .iter()
                .chain(&self.taken[self.blocked..])
                .copied()
                .collect()
        } else {
            self.taken.clone()
        }
    }

    /// How much nearer, in whole points, another node lies to a label of `key` at
    /// `rect` than its own at `node`; 0 when its own is nearest, or it names no node.
    fn misread(&self, key: Named, rect: egui::Rect, node: egui::Pos2, edge: f32) -> u32 {
        match key {
            Named::Node(tile) => misread_by(rect, tile, node, edge, &self.nodes),
            Named::Group(_) => 0,
        }
    }

    /// Where a name that must show goes when no place is free: off every other lit
    /// node if it can be, wherever it misleads least, over its own node if need be,
    /// then where it lies over the fewest other names and nodes, nearest; and whether
    /// it lies over another name there. Names placed this frame weigh only after
    /// misreading, as their places may in turn hang on this one's.
    fn least_misleading(
        &self,
        key: Named,
        node: egui::Pos2,
        edge: f32,
        extent: egui::Vec2,
    ) -> (Spot, bool) {
        let bounds = self.painter.clip_rect();
        let (blocked, names) = self.taken.split_at(self.blocked);
        let blocked = if self.landmark(key) {
            &blocked[..self.cards]
        } else {
            blocked
        };
        let under = |rects: &[egui::Rect], rect: egui::Rect| {
            rects
                .iter()
                .filter(|r| r.intersects(rect.expand2(PILL_PAD)))
                .count()
        };
        let at = std::iter::once(0)
            .chain(NUDGES)
            .flat_map(|shift| (0..SIDES.len()).map(move |side| Spot::beside(side, shift)))
            .chain((0..SIDES.len()).map(Spot::over))
            .min_by_key(|&at| {
                let rect = at.rect(node, edge, extent);
                (
                    self.covers_node(key, rect),
                    self.misread(key, rect, node, edge),
                    !bounds.contains_rect(rect),
                    under(names, rect),
                    under(blocked, rect),
                    at.over,
                    at.shift.unsigned_abs(),
                )
            })
            .unwrap_or(Spot::beside(0, 0));
        (at, under(names, at.rect(node, edge, extent)) > 0)
    }

    /// Draws what was placed, and fades out what was not where its node, found by
    /// `anchor`, still shows. Remembers it all for the next frame.
    fn finish(mut self, anchor: impl Fn(Named) -> Option<egui::Pos2>) {
        let mut leaving = Vec::new();
        for (key, mut shown) in std::mem::take(&mut self.last) {
            if self.next.contains_key(&key) {
                continue;
            }
            shown.opacity -= self.fade;
            if shown.opacity > 0.0
                && let Some(node) = anchor(key)
            {
                leaving.push((key, node, shown));
            }
        }
        // Leaving labels go underneath the ones that hold their place.
        for (_, node, shown) in &leaving {
            self.paint(*node, shown);
        }
        let rank = |key: &Named| self.top.iter().position(|k| k == key);
        let mut placed: Vec<(&Named, &Shown)> = self.next.iter().collect();
        placed.sort_by_key(|(key, _)| rank(key));
        // A function's name picks it, as a click on the function would.
        let mut hits: Vec<LabelHit> = Vec::new();
        for (key, shown) in placed {
            if let Some(&node) = self.anchors.get(key) {
                let pill = self.paint(node, shown);
                if let Named::Node(tile) = *key
                    && shown.opacity >= 0.5
                {
                    hits.push((pill, tile, self.numbers.get(key).cloned()));
                }
            }
        }
        self.painter
            .ctx()
            .data_mut(|d| d.insert_temp(label_hits_id(), hits));
        self.next
            .extend(leaving.into_iter().map(|(key, _, shown)| (key, shown)));
        let next = self.next;
        self.painter
            .ctx()
            .data_mut(|d| d.insert_temp(Self::id(), next));
    }

    /// A label on its pill, at its eased place beside `node`. Returns the pill.
    fn paint(&self, node: egui::Pos2, shown: &Shown) -> egui::Rect {
        let mut painter = self.painter.clone();
        painter.multiply_opacity(shown.opacity.clamp(0.0, 1.0));
        let min = egui::emath::GuiRounding::round_to_pixels(
            node + shown.offset,
            painter.pixels_per_point(),
        );
        let pill = egui::Rect::from_min_size(min, shown.galley.size()).expand2(PILL_PAD);
        // A name nudged away from its node, or pushed off by a crowd, keeps a thread
        // to it, from the node's edge: dotted and grey, never a call's color, so it
        // is not read as a call.
        let end = pill.clamp(node);
        if !shown.at.over && (shown.at.shift != 0 || end.distance(node) > shown.edge + 10.0) {
            let start = node + (end - node).normalized() * (shown.edge + 1.5);
            painter.extend(egui::Shape::dotted_line(&[start, end], THREAD, 3.0, 0.75));
        }
        painter.rect_filled(pill, 4.0, PILL);
        if let Some(color) = shown.stroke {
            painter.rect_stroke(
                pill,
                4.0,
                egui::Stroke::new(1.5, color),
                egui::StrokeKind::Inside,
            );
        }
        painter.galley(min, shown.galley.clone(), egui::Color32::WHITE);
        pill
    }
}

/// Whether a name at `rect` sits within [`NEAR`] of its node, a disc of `edge` at
/// `node`.
fn near(rect: egui::Rect, node: egui::Pos2, edge: f32) -> bool {
    rect.distance_to_pos(node) - edge <= NEAR
}

/// How much nearer, in whole points edge to edge, the nearest other of `nodes` lies
/// to `rect`, a label of `tile` (a disc of `edge` at `node`), than its own; 0 when
/// its own is nearest.
fn misread_by(
    rect: egui::Rect,
    tile: usize,
    node: egui::Pos2,
    edge: f32,
    nodes: &[(usize, egui::Pos2, f32)],
) -> u32 {
    let mine = rect.distance_to_pos(node) - edge;
    // Only nodes near enough to matter, cheaply.
    let reach = rect.expand(mine.max(0.0) + 40.0);
    let theirs = nodes
        .iter()
        .filter(|&&(other, at, radius)| other != tile && reach.expand(radius).contains(at))
        .map(|&(_, at, radius)| rect.distance_to_pos(at) - radius)
        .fold(f32::INFINITY, f32::min);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a small distance, at least 0"
    )]
    let by = (mine - theirs - 0.5).max(0.0).ceil() as u32;
    by
}

/// A journey number as its parts, nearest the root first: `2` before `1.1`, and
/// `1.10` after `1.9`.
type NumberOrder = (usize, Vec<u32>);

fn number_order(number: &str) -> NumberOrder {
    let parts: Vec<u32> = number.split('.').filter_map(|p| p.parse().ok()).collect();
    (parts.len(), parts)
}

fn label_hits_id() -> egui::Id {
    egui::Id::new("graph_label_hits")
}

/// A function's name as drawn: its pill, the function, and the journey number it
/// leads with.
type LabelHit = (egui::Rect, usize, Option<String>);

/// Every function's name as last drawn: its pill and the function, bottom first.
pub(crate) fn drawn_labels(ctx: &egui::Context) -> Vec<(egui::Rect, usize)> {
    ctx.data(|d| d.get_temp::<Vec<LabelHit>>(label_hits_id()))
        .unwrap_or_default()
        .into_iter()
        .map(|(pill, tile, _)| (pill, tile))
        .collect()
}

/// The function whose name, as last drawn, lies under `at`, the topmost, and the
/// journey number the name leads with.
pub(crate) fn label_at(ctx: &egui::Context, at: egui::Pos2) -> Option<(usize, Option<String>)> {
    ctx.data(|d| d.get_temp::<Vec<LabelHit>>(label_hits_id()))?
        .iter()
        .rev()
        .find(|(pill, ..)| pill.contains(at))
        .map(|(_, tile, number)| (*tile, number.clone()))
}

/// What leads a label: text in a color, on a fill when it is a badge.
type Lead = (String, egui::Color32, Option<egui::Color32>);

/// A label in two parts: the name, then a quieter note.
fn label_job(
    lead: Option<Lead>,
    name: &str,
    note: Option<String>,
    color: egui::Color32,
    size: f32,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let format = |color, size| egui::TextFormat {
        font_id: egui::FontId::proportional(size),
        color,
        ..default()
    };
    if let Some((lead, lead_color, fill)) = lead {
        let mut lead_format = format(lead_color, size);
        if let Some(fill) = fill {
            lead_format.background = fill;
        }
        job.append(&lead, 0.0, lead_format);
        job.append(" ", 0.0, format(color, size));
    }
    job.append(name, 0.0, format(color, size));
    if let Some(note) = note {
        job.append(&note, 6.0, format(MUTED, size - 1.5));
    }
    job
}

/// Names beside graph nodes, most important first, each on a dark pill so it reads
/// over links and glow. The focus's callees lead with the order it calls them in, and
/// its callers note where in their own calls they call it. A name tries each side of
/// its node and is left out when none is free, and never covers a node that matters;
/// zooming in makes room for more. Container names fill whatever space is left.
#[expect(clippy::too_many_lines, reason = "one ranked pass")]
fn draw_graph_labels(
    painter: &egui::Painter,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
    explorer: &Explorer,
    (up, right): (Vec3, Vec3),
    settled: bool,
) {
    let (graph, map) = (explorer.graph(), explorer.map());
    let center = explorer.lit_focus();
    // Gathered round the focus's module, the journey's cursor is named where it sits.
    let cursor = explorer.cursor_mark();
    // In a journey, a function selected off it, named as the focus is.
    let aside = explorer.aside();
    let aside_tile = aside;
    let hovered = explorer.pointed();
    let disclosed = explorer.disclosure();
    let journey = disclosed.tiles();
    // The number of the call the cursor is on.
    let cursor_number = explorer
        .selected()
        .and_then(|span| disclosed.number(span))
        .filter(|number| !number.is_empty());
    // The journey's start, which is named as the start come what may.
    let start = explorer
        .root()
        .and_then(|unit| map.tile_of(unit))
        .filter(|_| !explorer.browsing());
    let nucleus = explorer.nucleus();
    // Where each shown node sits on screen, and how far its edge is from its middle.
    let discs: Vec<Option<(egui::Pos2, f32)>> = (0..map.tiles.len())
        .map(|tile| {
            // Only the middle, what the nucleus names and what the pointer is on or
            // draws the calls of, however faint.
            let named = nucleus.shows(tile)
                && (nucleus.lit(tile) || center == Some(tile) || hovered == Some(tile));
            let world = explorer.tile_point(tile).filter(|_| named)?;
            let node = to_screen(world)?;
            let radius = graph_view::node_size(graph, tile) * nucleus.node_scale(false) / 2.0;
            let edge = to_screen(world + up * radius).map_or(8.0, |p| p.distance(node));
            Some((node, edge))
        })
        .collect();

    // A journey's functions are named level by level in the panel's order, the start
    // first, so a call's caller has room before the call does.
    let ladder: Vec<usize> = {
        let mut ladder: Vec<(NumberOrder, usize)> = journey
            .iter()
            .filter(|&(&tile, _)| start != Some(tile))
            .filter_map(|(&tile, numbers)| {
                numbers
                    .iter()
                    .map(|n| number_order(n))
                    .min()
                    .map(|first| (first, tile))
            })
            .collect();
        ladder.sort();
        ladder.into_iter().map(|(_, tile)| tile).collect()
    };
    let rungs: HashMap<usize, usize> = start
        .into_iter()
        .chain(ladder.iter().copied())
        .enumerate()
        .map(|(rung, tile)| (tile, rung))
        .collect();

    // Lower ranks are placed first and win any overlap.
    let mut order: Vec<(LabelRank, Named)> = Vec::new();
    for (tile, disc) in discs.iter().enumerate() {
        if disc.is_none() {
            continue;
        }
        // One rule: the middle and the cursor, then what the nucleus names, the
        // journey's first calls before its callers, so the journey has room first,
        // then the open journey round a middle off it; what the pointer is on comes
        // last, so pointing never moves another name.
        // The cursor is named where it would be anyway, so where it is never decides
        // what else is: it is only drawn over the rest.
        let tier = match () {
            () if center == Some(tile) || aside == Some(tile) => 0,
            () if nucleus.labelled(tile) && nucleus.level(tile) == Some(1) => 1,
            () if nucleus.labelled(tile) => 2,
            () if nucleus.context(tile) => 3,
            () if cursor == Some(tile) => 4,
            // The few calls in from outside the journey drawn for each module, those
            // nearest it, are named after all the journey's own names.
            () if nucleus.outside(tile) => 6,
            () if hovered == Some(tile) => POINTED,
            () => continue,
        };
        let rung = rungs.get(&tile).copied().unwrap_or(usize::MAX);
        order.push((
            (tier, rung, std::cmp::Reverse(graph.degree(tile)), tile),
            Named::Node(tile),
        ));
    }
    // Modules are named at the top of their spheres, ahead of most function names.
    let ring_top = |cluster: usize| {
        let ball = nucleus.ball(cluster)?;
        let top = to_screen(ball.center + up * ball.radius);
        if top.is_some() || !nucleus.browsing() {
            return top;
        }
        // Browsing, a sphere cut by the window's edge is named on what of it shows.
        let (middle, reach) = (ball.center, ball.radius * 0.7);
        [
            middle,
            middle + right * reach,
            middle - right * reach,
            middle - up * reach,
        ]
        .into_iter()
        .find_map(to_screen)
    };
    // A sphere's middle on screen and how far its edge is from it.
    let sphere_on_screen = |ball: nucleus::Ball| {
        let middle = to_screen(ball.center)?;
        let edge = to_screen(ball.center + up * ball.radius)?;
        Some((middle, middle.distance(edge)))
    };
    let left_out = nucleus.outside_left_out(graph);
    // Browsing, the module the pointer is over, its name shown however small: the
    // smallest whose sphere holds the pointer.
    let mouse = painter.ctx().input(|i| i.pointer.hover_pos());
    let pointed_module = nucleus
        .balls()
        .filter(|_| nucleus.browsing())
        .filter_map(|(cluster, ball)| {
            let (middle, edge) = sphere_on_screen(ball)?;
            (mouse?.distance(middle) <= edge).then_some((edge, cluster))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, cluster)| cluster);
    {
        // The nucleus and the open modules are named first, and of the rest only the
        // biggest, where there is room.
        let mut closed: Vec<(usize, usize)> = nucleus
            .balls()
            .filter(|(_, ball)| {
                matches!(ball.body, Body::Closed | Body::Faint) && nucleus.shows_body(ball.body)
            })
            .map(|(cluster, _)| (nucleus.count(cluster), cluster))
            .collect();
        closed.sort_unstable_by(|a, b| b.cmp(a));
        // None is named through the glass of a sphere in front, nor so near one
        // that its name would lie over it.
        let discs: Vec<(egui::Pos2, f32)> = nucleus
            .balls()
            .filter(|(_, ball)| matches!(ball.body, Body::Nucleus | Body::Open))
            .filter_map(|(_, ball)| {
                let middle = to_screen(ball.center)?;
                let edge = to_screen(ball.center + up * ball.radius)?;
                Some((middle, middle.distance(edge)))
            })
            .collect();
        let hidden = |cluster: usize| {
            nucleus
                .ball(cluster)
                .and_then(|ball| to_screen(ball.center))
                .is_none_or(|at| discs.iter().any(|&(m, r)| at.distance(m) < r + 110.0))
        };
        // Expanded, the rest of the codebase goes unnamed: the ring says where calls go.
        // Browsing, the whole codebase is the map: every module on screen is named,
        // whatever is clicked, so the landmarks never change under a click.
        let browsing = nucleus.browsing();
        let named: HashSet<usize> = closed
            .iter()
            .filter(|_| !nucleus.expanded())
            .filter(|&&(_, cluster)| {
                if browsing {
                    ring_top(cluster).is_some()
                } else {
                    !hidden(cluster)
                }
            })
            .take(if browsing { usize::MAX } else { CLOSED_NAMED })
            .map(|&(_, cluster)| cluster)
            .collect();
        for (cluster, ball) in nucleus.balls() {
            let rank = match ball.body {
                // Browsing, the one the pointer is over is named last, over the rest.
                _ if browsing && pointed_module == Some(cluster) => (
                    POINTED,
                    0,
                    std::cmp::Reverse(usize::MAX),
                    usize::MAX - nucleus.count(cluster),
                ),
                // Browsing, a module is named before any function, biggest first,
                // when it shows big enough for its name to be read as its own: by
                // how far the view is zoomed in alone, so what is clicked never moves
                // a module's name or leaves one out.
                _ if browsing => {
                    if sphere_on_screen(ball).is_none_or(|(_, edge)| edge < NAMED_SPHERE) {
                        continue;
                    }
                    (
                        0,
                        0,
                        std::cmp::Reverse(usize::MAX),
                        usize::MAX - nucleus.count(cluster),
                    )
                }
                // Which module a named function lives in matters as much as its name:
                // right after the start's.
                Body::Nucleus | Body::Open => {
                    (0, 1, std::cmp::Reverse(nucleus.count(cluster)), cluster)
                }
                Body::Closed | Body::Faint
                    if named.contains(&cluster) || left_out.contains_key(&cluster) =>
                {
                    (6, 0, std::cmp::Reverse(nucleus.count(cluster)), cluster)
                }
                Body::Closed | Body::Faint => continue,
            };
            order.push((rank, Named::Group(cluster)));
        }
    }
    // A label keeps the place it took when it first showed for as long as the scene
    // lasts, and is placed before any new one, which keeps off it.
    let scene = if nucleus.browsing() {
        Scene::Browse
    } else {
        Scene::Journey {
            start,
            expanded: nucleus.expanded(),
        }
    };
    let held = painter
        .ctx()
        .data_mut(|d| {
            d.get_temp_mut_or_default::<SpotBook>(spot_book_id())
                .get(&scene)
                .cloned()
        })
        .unwrap_or_default();
    order.sort_by_key(|&(key, named)| (!held.contains_key(&named), key));
    // Two functions of one name are told apart by their types: `Explorer.root`,
    // `Track.root`.
    let mut by_name: HashMap<String, Vec<usize>> = HashMap::new();
    for &(_, named) in &order {
        if let Named::Node(tile) = named {
            by_name
                .entry(node_name(explorer, tile))
                .or_default()
                .push(tile);
        }
    }
    let homonyms: HashSet<usize> = by_name
        .into_values()
        .filter(|tiles| tiles.len() > 1)
        .flatten()
        .collect();

    // Labels keep off the cards over the graph, as last laid out; the search box,
    // which comes and goes, last, as landmarks do not move for it.
    let cards: Vec<egui::Rect> = ["search"]
        .into_iter()
        .filter_map(|id| painter.ctx().memory(|m| m.area_rect(egui::Id::new(id))))
        .collect();
    let lasting = cards.len()
        - usize::from(
            painter
                .ctx()
                .memory(|m| m.area_rect(egui::Id::new("search")))
                .is_some(),
        );
    // And off the nodes that matter, so they never hide a neighbor: every lit one, the
    // journey's, its callers and those brought in from outside; what only the pointer
    // names is no reason for another name to move.
    let blocking: Vec<(usize, egui::Pos2, f32)> = (0..discs.len())
        .filter(|&tile| {
            nucleus.names(tile)
                || nucleus.marks(tile)
                || nucleus.context(tile)
                || nucleus.outside(tile)
                || center == Some(tile)
        })
        .filter_map(|tile| discs[tile].map(|(node, edge)| (tile, node, edge)))
        .collect();
    let mut placer = Placer::new(
        painter,
        cards
            .into_iter()
            .chain(blocking.iter().map(|&(_, node, edge)| {
                egui::Rect::from_center_size(node, egui::Vec2::splat(edge * 2.0))
            }))
            .collect(),
        order
            .iter()
            .filter_map(|&(_, named)| match named {
                Named::Node(tile) => discs[tile].map(|(node, edge)| (tile, node, edge)),
                Named::Group(_) => None,
            })
            .collect(),
    );
    placer.cards = lasting;
    placer.blocking = blocking;
    placer.landmarks = nucleus.browsing();
    let held_keys: HashSet<Named> = held.keys().copied().collect();
    placer.held = held;
    placer.keep = settled;
    placer.beside = [cursor, center, aside]
        .into_iter()
        .flatten()
        .map(Named::Node)
        .collect();
    // The selection's and the cursor's names, and the one the pointer is on, read
    // over the rest.
    placer.top = [center, aside, hovered, cursor]
        .into_iter()
        .flatten()
        .map(Named::Node)
        .collect();
    if let Some(cluster) = pointed_module {
        placer.top.insert(0, Named::Group(cluster));
    }
    let mut flushed = false;
    for ((tier, ..), named) in order {
        // Every held name that has room is placed; then those that have none, for
        // now; then the new ones.
        if !flushed && !held_keys.contains(&named) {
            placer.place_displaced();
            flushed = true;
        }
        match named {
            Named::Group(group) => {
                let Some(at) = ring_top(group) else {
                    continue;
                };
                let body = nucleus.ball(group).map(|b| b.body);
                let color =
                    egui_color(lane_color(group % PALETTE.len())).gamma_multiply(match body {
                        Some(Body::Nucleus | Body::Open) | None => 1.0,
                        Some(Body::Faint) => 0.5,
                        Some(Body::Closed) => 0.35,
                    });
                let label = explorer.cluster_name(group);
                // An open module says how much of it is named, and how many callers
                // from outside the journey it holds beyond those drawn. Browsing, only
                // under the pointer, so a click never makes a name longer and pushes
                // another module's out.
                // The start's callers folded into a count are counted apart from what
                // it names.
                let folded = nucleus.folded_in(group);
                let named_note = (body == Some(Body::Open)
                    && nucleus.opened(group) > folded
                    && (!nucleus.browsing() || pointed_module == Some(group)))
                .then(|| {
                    format!(
                        "{} of {}",
                        nucleus.opened(group) - folded,
                        nucleus.count(group)
                    )
                });
                let plural = |count: usize| if count == 1 { "" } else { "s" };
                let folded_note =
                    (folded > 0).then(|| format!("+{folded} caller{}", plural(folded)));
                let more = left_out.get(&group).copied().unwrap_or(0);
                let more_note = (more > 0).then(|| format!("+{more} more caller{}", plural(more)));
                let note = [named_note, folded_note, more_note]
                    .into_iter()
                    .flatten()
                    .reduce(|a, b| format!("{a} · {b}"));
                let browsing = nucleus.browsing();
                // Browsing, as big as the module, whatever is clicked, so a name
                // opening or closing its module keeps its place.
                let size = match body {
                    _ if browsing => browse_name_size(nucleus.count(group)),
                    Some(Body::Nucleus) => 15.0,
                    Some(Body::Closed | Body::Faint) => 10.5,
                    _ => 12.0,
                };
                let job = label_job(None, &label.to_uppercase(), note, color, size);
                // Expanded, the calls drawn end at the modules on the ring. Browsing,
                // every module big enough is a landmark, kept off the others' names;
                // the one the pointer is over is named come what may.
                let force = (browsing && tier == POINTED)
                    || body == Some(Body::Nucleus)
                    || (body == Some(Body::Open) && nucleus.expanded());
                let force = if force {
                    Force::Always
                } else if more > 0 || folded > 0 || browsing {
                    Force::Free
                } else {
                    Force::Never
                };
                placer.place(named, at, 0.0, job, None, force);
            }
            Named::Node(tile) => {
                let Some((node, edge)) = discs[tile] else {
                    continue;
                };
                let name = match explorer.tile_unit(tile) {
                    Some(unit) if homonyms.contains(&tile) => explorer.unit_name(unit),
                    _ => node_name(explorer, tile),
                };
                // Colored as its call is drawn; a faint speck named only under the
                // pointer, and the open journey round a middle off it, quietly.
                let level = nucleus.level(tile);
                let quiet = graph_view::speck(explorer, tile)
                    || nucleus.context(tile)
                    || (nucleus.outside(tile) && aside != Some(tile));
                let color = match tier {
                    _ if cursor == Some(tile) => CURSOR_TEXT,
                    0 => egui::Color32::WHITE,
                    _ if quiet => MUTED,
                    _ => match level {
                        Some(-1) => CALLER_TEXT,
                        Some(1) => CALLEE_TEXT,
                        Some(2..) => DEEPER_TEXT,
                        _ => egui::Color32::from_rgb(200, 205, 216),
                    },
                };
                let aside = if start == Some(tile) {
                    // A start the journey calls again says so, after saying it is the
                    // start, so the call back is not read as its own.
                    let panel = journey.get(&tile).map_or(&[][..], Vec::as_slice);
                    let again = nucleus::badge(nucleus.numbers(tile).iter().chain(panel));
                    again.map(|again| format!("start · again at {again}"))
                } else if nucleus.on_journey(tile) && nucleus.names(tile) {
                    // Round a middle off the journey, which of its calls the journey
                    // runs too.
                    Some("on the journey".to_owned())
                } else {
                    None
                };
                // How the journey runs it, after anything else said of it.
                // When it runs in the panel's words; where its runs differ, only that it
                // does not always.
                let when = disclosed.tile_sometimes(tile).then(|| {
                    disclosed
                        .tile_when(tile)
                        .map_or("sometimes".to_owned(), |w| short(w, 26))
                });
                let marks: Vec<String> = when
                    .into_iter()
                    .chain(disclosed.tile_fails(tile).then(|| "can fail".to_owned()))
                    .collect();
                let aside = [aside, (!marks.is_empty()).then(|| marks.join(" · "))]
                    .into_iter()
                    .flatten()
                    .reduce(|a, b| format!("{a} · {b}"));
                // A node the journey runs leads with its place in it, as its bar does:
                // one number for the journey, wherever the cursor is, its first call
                // in the panel's order, and how many more it runs; but the cursor's
                // own shows the call the cursor is on, as the panel does. The start is
                // the start, however often the journey comes back to it.
                let order = if start == Some(tile) {
                    None
                } else {
                    let panel = journey.get(&tile).map_or(&[][..], Vec::as_slice);
                    let numbers = nucleus.numbers(tile).iter().chain(panel);
                    match cursor_number.filter(|_| cursor == Some(tile)) {
                        Some(at) => Some(nucleus::badge_at(numbers, at)),
                        None => nucleus::badge(numbers),
                    }
                };
                // A click on the name goes to the call it leads with; on the start's,
                // to the start.
                if start == Some(tile) {
                    placer.numbers.insert(named, String::new());
                } else if let Some(first) =
                    order.as_deref().and_then(|o| o.split_whitespace().next())
                {
                    placer
                        .numbers
                        .insert(named, first.trim_end_matches(',').to_owned());
                }
                // The number on a badge filled like the node and its bar, so the two
                // read as one.
                let fill = egui_color(graph_view::node_color(explorer, tile));
                let lead = order.map(|order| (format!(" {order} "), ON_FILL, Some(fill)));
                // The cursor's name keeps the size it had before the cursor came, and a
                // name clicked off the journey the size it had before the click, so
                // each keeps its place too.
                let size = match tier {
                    _ if aside_tile == Some(tile) && nucleus.outside(tile) => 10.0,
                    _ if aside_tile == Some(tile) => 11.0,
                    0 => 14.0,
                    _ if quiet => 10.0,
                    1 => 12.0,
                    _ => 11.0,
                };
                // The pill's border says which way it relates to the focus, as its call
                // does; the cursor's is its own color.
                let stroke = match level {
                    _ if cursor == Some(tile) => Some(egui_color(CURSOR)),
                    _ if tier == 0 => Some(egui::Color32::from_white_alpha(140)),
                    _ if quiet => None,
                    Some(-1) => Some(egui_color(CALLER)),
                    Some(1) => Some(egui_color(CALLEE)),
                    Some(2..) => Some(egui_color(DEEPER)),
                    _ => None,
                };
                let job = label_job(lead, &name, aside, color, size);
                // The focus and what the pointer is over are named come what may;
                // everything the nucleus names, wherever there is room.
                let force = if tier == 0 || hovered == Some(tile) {
                    Force::Always
                } else if tier <= 2 || nucleus.outside(tile) {
                    Force::Free
                } else {
                    Force::Never
                };
                placer.place(named, node, edge, job, stroke, force);
            }
        }
    }
    placer.place_displaced();
    // A journey's function is named only under a caller that is: the panel's tree
    // reads on the graph from the start down, never a call without its caller.
    let numbered: HashMap<&str, usize> = journey
        .iter()
        .flat_map(|(&tile, numbers)| numbers.iter().map(move |n| (n.as_str(), tile)))
        .collect();
    let mut named_calls: HashSet<usize> = start.into_iter().collect();
    for &tile in &ladder {
        let key = Named::Node(tile);
        if !placer.next.contains_key(&key) {
            continue;
        }
        let under_named = journey.get(&tile).is_some_and(|numbers| {
            numbers.iter().any(|n| match n.rsplit_once('.') {
                None => true,
                Some((caller, _)) => numbered
                    .get(caller)
                    .is_some_and(|caller| named_calls.contains(caller)),
            })
        });
        // The cursor's name shows wherever it is, but names no calls under it that
        // would not be named without it.
        if !under_named && !placer.beside.contains(&key) && hovered != Some(tile) {
            placer.unplace(key);
        } else if under_named && !placer.overlaid.contains(&key) {
            named_calls.insert(tile);
        }
    }
    // Every function lit and on screen is named, or counted here as not.
    let lit: Vec<usize> = (0..discs.len())
        .filter(|&tile| discs[tile].is_some() && nucleus.names(tile) && !nucleus.folded(tile))
        .collect();
    let unnamed = lit
        .iter()
        .filter(|&&tile| !placer.next.contains_key(&Named::Node(tile)))
        .count();
    let chosen = std::mem::take(&mut placer.chosen);
    placer.finish(|named| match named {
        Named::Node(tile) => discs.get(tile).copied().flatten().map(|(node, _)| node),
        Named::Group(group) => ring_top(group),
    });
    if !chosen.is_empty() {
        painter.ctx().data_mut(|d| {
            let book = d.get_temp_mut_or_default::<SpotBook>(spot_book_id());
            if book.len() >= SCENES_KEPT && !book.contains_key(&scene) {
                book.clear();
            }
            book.entry(scene).or_default().extend(chosen);
        });
    }
    // The call in of an outside caller, or of a caller of the start folded into a
    // count, the pointer rests on, or of a function clicked off the journey, is what
    // is being traced: drawn again over the names, so none hides where it lands.
    if !nucleus.browsing() {
        let traced = nucleus.links().iter().filter(|link| {
            let pointed = Some(link.from) == hovered
                && (link.flow == Flow::Outside || nucleus.folded(link.from));
            pointed || (aside_tile.is_some() && Some(link.from) == aside_tile)
        });
        for link in traced {
            let route: Vec<egui::Pos2> = nucleus
                .route(link)
                .into_iter()
                .filter_map(to_screen)
                .collect();
            let edge = discs
                .get(link.to)
                .copied()
                .flatten()
                .map_or(0.0, |(_, e)| e);
            traced_call(painter, &route, edge);
        }
    }
    if unnamed > 0 {
        unnamed_note(painter, unnamed, lit.len());
    }
}

/// A call being traced, along `route` on screen, its head on the edge of the
/// function it calls, `edge` from that function's middle: white, over the names.
fn traced_call(painter: &egui::Painter, route: &[egui::Pos2], edge: f32) {
    let [.., before, last] = route else {
        return;
    };
    let along = (*last - *before).normalized();
    let tip = *last - along * (edge + 1.0);
    let ink = egui::Color32::from_white_alpha(235);
    let mut points = route[..route.len() - 1].to_vec();
    points.push(tip - along * 4.0);
    painter.add(egui::Shape::line(points, egui::Stroke::new(1.6, ink)));
    let across = egui::vec2(-along.y, along.x);
    let back = tip - along * 8.0;
    painter.add(egui::Shape::convex_polygon(
        vec![tip, back + across * 4.0, back - across * 4.0],
        ink,
        egui::Stroke::NONE,
    ));
}

/// Browsing, how far a sphere's edge must show from its middle, in points, for the
/// module to be named without the pointer on it.
const NAMED_SPHERE: f32 = 13.0;

/// Browsing, how big a module's name is, by how many functions it holds.
fn browse_name_size(count: usize) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "a function count")]
    (9.5 + 1.2 * (count.max(1) as f32).ln()).clamp(10.0, 15.5)
}

/// How many of the functions lit had no room for their names, at the graph's foot
/// above the hint at the keys.
fn unnamed_note(painter: &egui::Painter, unnamed: usize, lit: usize) {
    let area = painter.clip_rect();
    let text = format!(
        "{unnamed} of {lit} lit functions unnamed for want of room · zoom in, or point at one"
    );
    let galley = painter.layout_no_wrap(text, egui::FontId::proportional(12.0), MUTED);
    let at = egui::pos2(area.min.x + 12.0, area.max.y - 30.0 - galley.size().y);
    painter.rect_filled(
        egui::Rect::from_min_size(at, galley.size()).expand2(PILL_PAD),
        4.0,
        PILL,
    );
    painter.galley(at, galley, MUTED);
}

/// A hint at the keys along `foot`.
fn key_hint(ctx: &egui::Context, foot: egui::Rect, view: Showing) {
    ctx.layer_painter(egui::LayerId::background())
        .with_clip_rect(foot)
        .text(
            egui::pos2(foot.min.x + 12.0, foot.max.y - 10.0),
            egui::Align2::LEFT_BOTTOM,
            if view.browsing {
                "Browsing · click a function to select it, nothing moves · Enter opens its \
                 journey · Backspace back · / search · R whole map · H keys"
            } else {
                "Journey · click a call to move the cursor there · ] [ ↓ ↑ ← → step · , . \
                 less or more · I calls in from outside · Esc leaves for browsing · H keys"
            },
            egui::FontId::proportional(12.0),
            MUTED,
        );
}

/// Gathered round the focus's module, how many of the other modules are named.
const CLOSED_NAMED: usize = 16;

/// Gathered round the focus's module, the tier of a name only the pointer asks for:
/// placed after every other, so it never moves one.
const POINTED: u8 = 7;

/// What the view open is, for the keys that apply in it.
#[derive(Debug, Clone, Copy)]
struct Showing {
    /// Browsing, rather than in a journey.
    browsing: bool,
}

/// Which view a key does something in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum When {
    Always,
    /// No journey open.
    Browse,
    /// A journey open.
    Journey,
}

impl Showing {
    fn of(explorer: &Explorer) -> Self {
        Self {
            browsing: explorer.browsing(),
        }
    }
}

impl When {
    fn applies(self, view: Showing) -> bool {
        match self {
            Self::Always => true,
            Self::Browse => view.browsing,
            Self::Journey => !view.browsing,
        }
    }
}

/// A group of keys by title: each key, what it does, and in which view.
type KeyGroup = (&'static str, &'static [(&'static str, &'static str, When)]);

/// Every key, grouped by what it moves, each saying what it does in the view it
/// applies in; H lists those that apply to the view open.
const KEYS: [KeyGroup; 7] = [
    (
        "Browse and journey",
        &[
            (
                "click",
                "select a function where it sits: its callers and calls light up; nothing moves",
                When::Browse,
            ),
            (
                "click",
                "a function the journey runs: move the cursor there (the run nearest it); \
                 nothing moves",
                When::Journey,
            ),
            (
                "click",
                "any other function: select it in place, with its calls into the journey",
                When::Journey,
            ),
            (
                "click a name",
                "as a click on its function; on the journey, to the call its number names",
                When::Always,
            ),
            (
                "Enter",
                "open the journey of the function selected",
                When::Browse,
            ),
            (
                "Enter",
                "start a journey at the function selected, or at the cursor",
                When::Journey,
            ),
            (
                "Shift click",
                "open the journey of a function at once, as deep as now",
                When::Always,
            ),
            (
                "Esc",
                "close search, this list, a function selected off the journey; then \
                 leave the journey for browsing",
                When::Journey,
            ),
            ("Esc", "close search, this list", When::Browse),
            (
                "Backspace",
                "back: the selection before, or across into the journey before (← Back)",
                When::Always,
            ),
            ("Shift Backspace", "forward again (Forward →)", When::Always),
            ("/", "search for a function", When::Always),
        ],
    ),
    (
        "Journey cursor",
        &[
            (
                "↓  ↑",
                "into the first call it makes / out to its caller",
                When::Journey,
            ),
            (
                "←  →",
                "previous / next call by the same caller",
                When::Journey,
            ),
            (
                "[  ]",
                "back / forward one call, in the order they run",
                When::Journey,
            ),
            ("click a bar", "to that call", When::Journey),
        ],
    ),
    (
        "Journey depth",
        &[(
            ",  .",
            "show a level less / more of the journey",
            When::Journey,
        )],
    ),
    (
        "Calls in",
        &[(
            "I",
            "bring in every function off the journey that calls one on it, pale",
            When::Journey,
        )],
    ),
    (
        "Spheres",
        &[(
            "X, dbl-click",
            "expand the start's module / shrink it back; inside, a click centres it",
            When::Journey,
        )],
    ),
    (
        "Camera",
        &[
            ("drag", "orbit (or two fingers)", When::Always),
            (
                "right drag",
                "pan (or Shift drag, Shift two fingers)",
                When::Always,
            ),
            (
                "wheel, pinch",
                "zoom towards the pointer (or Ctrl two fingers)",
                When::Always,
            ),
            (
                "F",
                "back to the usual view, round what is lit",
                When::Always,
            ),
            (
                "R",
                "frame the whole codebase from this angle",
                When::Browse,
            ),
            ("R", "frame the journey from this angle", When::Journey),
        ],
    ),
    ("Help", &[("H", "this list", When::Always)]),
];

/// What each color means, as the help leads with it.
fn color_key(browsing: bool) -> Vec<(egui::Color32, &'static str)> {
    if browsing {
        return vec![
            (egui::Color32::WHITE, "the function selected"),
            (egui_color(CALLER), "calls into it"),
            (egui_color(CALLEE), "its own calls out"),
        ];
    }
    vec![
        (egui::Color32::WHITE, "the journey's start"),
        (egui_color(CALLER), "calls into it"),
        (egui_color(CALLEE), "its own calls"),
        (egui_color(DEEPER), "calls further down, darker the deeper"),
        (egui_color(CURSOR), "the journey's cursor"),
    ]
}

/// How what is drawn reads, beyond its colors, as the help says it.
fn reading(browsing: bool) -> &'static [&'static str] {
    if browsing {
        return &[
            "Each sphere is a module, in a color of its own, named the same whatever is \
             clicked; a lit one says how many of its functions are named (3 of 39). Two \
             modules of one name are told apart by their crate (layout::graph).",
            "A name set apart from its function keeps a dotted grey thread to it. A \
             function with no room for its name near it is named when you point at it, \
             and counted at the foot.",
        ];
    }
    &[
        "Each sphere is a module, in a color of its own, saying how many of its functions \
         the journey names (2 of 23); the middle one holds the start. +5 callers: callers \
         of the start drawn pale and left unnamed; point at one for its name.",
        "On a name, 2 +1: its first call in the journey, numbered as in the panel, and how \
         many more times the journey runs it; the cursor's shows the call it is on. A name \
         set apart keeps a dotted grey thread to its function. Two functions of one name \
         show their types (Explorer.root, Track.root).",
        "The panel: a bar per call, left to right as they run, a row a level deeper. \
         Dashed: runs only sometimes, and says when (if …, in a loop). Pale: a call only \
         guessed at, or handed off to run later. A triangle and a count: calls folded \
         under it; a click opens them. A green end returns, a red one fails.",
    ]
}

/// Width of the keys' column in the help.
const KEY_WIDTH: f32 = 84.0;

/// Widest the help grows.
const HELP_WIDTH: f32 = 820.0;

/// Roughly how wide a character of the help's text is, in points, to balance its
/// columns before they are laid out.
const HELP_CHAR: f32 = 5.6;

/// The keys that apply to the view open, on a sheet down the right of `area`, in two
/// columns where they fit: whole at 1600 × 1000, and in a smaller window scrolling,
/// with a cue at its foot while more lies below.
fn key_help(ctx: &egui::Context, area: egui::Rect, open: &mut bool, view: Showing) {
    let margin = 12.0;
    let width = (area.width() - 2.0 * margin).clamp(240.0, HELP_WIDTH);
    let height = (area.height() - 2.0 * margin).max(160.0);
    let columns = if width >= 560.0 { 2 } else { 1 };
    let sections: Vec<(&str, Vec<(&str, &str)>)> = KEYS
        .iter()
        .map(|(title, keys)| {
            let keys = keys
                .iter()
                .filter(|(.., when)| when.applies(view))
                .map(|&(key, what, _)| (key, what))
                .collect();
            (*title, keys)
        })
        .filter(|(_, keys): &(_, Vec<_>)| !keys.is_empty())
        .collect();
    let placed = balance(&sections, columns, width);
    egui::Area::new("key_help".into())
        .pivot(egui::Align2::RIGHT_TOP)
        .fixed_pos(egui::pos2(area.max.x - margin, area.min.y + margin))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgb(11, 13, 19))
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(60)))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.set_width(width - 28.0);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.horizontal(|ui| {
                        let title = if view.browsing {
                            "Keys while browsing: no journey open"
                        } else {
                            "Keys in a journey"
                        };
                        ui.label(egui::RichText::new(title).strong().size(14.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("close (H)").clicked() {
                                *open = false;
                            }
                        });
                    });
                    let body = |ui: &mut egui::Ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            for (color, what) in color_key(view.browsing) {
                                ui.label(egui::RichText::new("●").color(color));
                                ui.label(egui::RichText::new(what).size(11.5).color(MUTED));
                                ui.add_space(8.0);
                            }
                        });
                        ui.label(
                            egui::RichText::new(if view.browsing {
                                "Browsing, a click selects a function and moves nothing. \
                                 Enter opens its journey."
                            } else {
                                "In a journey, the view gathers round where it starts and \
                                 shows only the journey; a click moves its cursor and nothing \
                                 else. Esc leaves it for browsing."
                            })
                            .size(11.5)
                            .color(egui::Color32::WHITE),
                        );
                        for line in reading(view.browsing) {
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(*line).size(11.5).color(MUTED),
                                )
                                .wrap(),
                            );
                        }
                        ui.add_space(4.0);
                        ui.columns(columns, |cols| {
                            for (col, indices) in cols.iter_mut().zip(&placed) {
                                col.spacing_mut().item_spacing.y = 1.0;
                                for &index in indices {
                                    let (title, keys) = &sections[index];
                                    key_section(col, title, keys);
                                }
                            }
                        });
                    };
                    // As tall as the window lets it be: a scroll area is otherwise only
                    // as tall as egui guesses an area's room to be.
                    let scroll = egui::ScrollArea::vertical()
                        .max_height(height - 40.0)
                        .min_scrolled_height(height - 40.0)
                        .auto_shrink([false, true])
                        .show(ui, body);
                    more_below(ui, &scroll);
                });
        });
}

/// Which sections of keys go in which of `columns` on a sheet `width` wide: each to
/// the shortest column so far, in order, by the lines its keys wrap to.
fn balance(sections: &[(&str, Vec<(&str, &str)>)], columns: usize, width: f32) -> Vec<Vec<usize>> {
    #[expect(clippy::cast_precision_loss, reason = "a column count")]
    let column_width = (width - 40.0) / columns as f32 - KEY_WIDTH - 8.0;
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a positive count of characters"
    )]
    let per_line = (column_width / HELP_CHAR).max(10.0) as usize;
    let mut placed: Vec<Vec<usize>> = vec![Vec::new(); columns];
    let mut lengths = vec![0; columns];
    for (index, (_, keys)) in sections.iter().enumerate() {
        let shortest = (0..columns).min_by_key(|&c| lengths[c]).unwrap_or(0);
        placed[shortest].push(index);
        lengths[shortest] += 2 + keys
            .iter()
            .map(|(_, what)| what.chars().count().div_ceil(per_line))
            .sum::<usize>();
    }
    placed
}

/// Whether the help had more below its foot last frame.
fn more_below_id() -> egui::Id {
    egui::Id::new("key_help_more_below")
}

/// While more of a scrolled sheet lies below, a cue at its foot, over the sheet.
fn more_below(ui: &egui::Ui, scroll: &egui::scroll_area::ScrollAreaOutput<()>) {
    let below = scroll.content_size.y - scroll.state.offset.y - scroll.inner_rect.height();
    ui.ctx()
        .data_mut(|d| d.insert_temp(more_below_id(), below > 4.0));
    if below <= 4.0 {
        return;
    }
    let foot = scroll.inner_rect;
    let galley = ui.painter().layout_no_wrap(
        "▼ more below: scroll".to_owned(),
        egui::FontId::proportional(12.0),
        CURSOR_TEXT,
    );
    let at = egui::pos2(
        foot.center().x - galley.size().x / 2.0,
        foot.max.y - galley.size().y - 4.0,
    );
    let pill = egui::Rect::from_min_size(at, galley.size()).expand2(PILL_PAD);
    ui.painter().rect_filled(pill, 4.0, PILL);
    ui.painter().galley(at, galley, CURSOR_TEXT);
}

/// One group of keys, each key in a fixed column and what it does wrapping beside it.
fn key_section(ui: &mut egui::Ui, title: &str, keys: &[(&str, &str)]) {
    ui.label(egui::RichText::new(title).strong().size(12.5));
    for (key, what) in keys {
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(KEY_WIDTH, 13.0),
                egui::Layout::left_to_right(egui::Align::Min),
                |ui| {
                    ui.set_min_width(KEY_WIDTH);
                    ui.label(
                        egui::RichText::new(*key)
                            .monospace()
                            .size(11.0)
                            .color(egui::Color32::WHITE),
                    );
                },
            );
            ui.add(egui::Label::new(egui::RichText::new(*what).size(11.5).color(MUTED)).wrap());
        });
    }
    ui.add_space(6.0);
}

pub(crate) fn short(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_owned(),
    }
}

pub(crate) fn egui_color(color: Color) -> egui::Color32 {
    let [r, g, b, a] = color.to_srgba().to_u8_array();
    egui::Color32::from_rgba_unmultiplied(r, g, b, a)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs one frame that places a label for `key` at `node` with `blocked` taken,
    /// and returns the side it took and where its text was drawn from the node.
    fn frame(ctx: &egui::Context, node: egui::Pos2, blocked: &[egui::Rect]) -> (usize, egui::Vec2) {
        let shown = placed(ctx, node, blocked, false);
        (shown.at.side, shown.offset)
    }

    /// Runs one frame that places a label for `key` at `node` with `blocked` taken,
    /// forced or not, and returns it as drawn.
    fn placed(ctx: &egui::Context, node: egui::Pos2, blocked: &[egui::Rect], force: bool) -> Shown {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let mut output = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx();
            let painter = ctx.layer_painter(egui::LayerId::background());
            let mut placer = Placer::new(&painter, blocked.to_vec(), Vec::new());
            let job = label_job(None, "draw_graph", None, egui::Color32::WHITE, 12.5);
            let force = if force { Force::Always } else { Force::Never };
            assert!(placer.place(Named::Node(0), node, 5.0, job, None, force));
            placer.finish(|_| Some(node));
        });
        output.textures_delta.clear();
        ctx.data_mut(|d| d.get_temp::<ShownLabels>(Placer::id()))
            .and_then(|labels| labels.get(&Named::Node(0)).cloned())
            .unwrap_or_else(|| panic!("the label was not remembered"))
    }

    #[test]
    fn a_name_that_must_show_is_nudged_aside_not_dropped() {
        let ctx = egui::Context::default();
        let node = egui::pos2(400.0, 300.0);
        // Every side of the node taken, but free further up and down.
        let crowd = egui::Rect::from_center_size(node, egui::vec2(500.0, 44.0));
        let shown = placed(&ctx, node, &[crowd], true);
        assert_ne!(shown.at.shift, 0, "nudged along a side");
        let text = egui::Rect::from_min_size(node + shown.offset, shown.galley.size());
        assert!(!text.intersects(crowd), "clear of what was there: {text:?}");
        assert!(
            text.center().distance(node) < 80.0,
            "and still near its node: {text:?}"
        );
    }

    /// Runs one frame of `place` calls on a fresh placer set up by `setup`, and
    /// returns the labels as drawn and the placer's rooms taken.
    fn placing(
        setup: impl Fn(&mut Placer),
        labels: &[(Named, egui::Pos2, Force)],
    ) -> (ShownLabels, HashSet<Named>, usize) {
        let ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 600.0),
            )),
            ..Default::default()
        };
        let mut overlaid = HashSet::new();
        let mut taken = 0;
        let mut output = ctx.run_ui(input, |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::background());
            let mut placer = Placer::new(&painter, Vec::new(), Vec::new());
            setup(&mut placer);
            for &(key, node, force) in labels {
                let job = label_job(None, "draw_graph", None, egui::Color32::WHITE, 12.5);
                placer.place(key, node, 5.0, job, None, force);
            }
            placer.place_displaced();
            overlaid.clone_from(&placer.overlaid);
            taken = placer.taken.len();
            placer.finish(|_| Some(egui::Pos2::ZERO));
        });
        output.textures_delta.clear();
        let shown = ctx
            .data_mut(|d| d.get_temp::<ShownLabels>(Placer::id()))
            .unwrap_or_default();
        (shown, overlaid, taken)
    }

    #[test]
    fn a_held_name_stays_put_and_new_ones_keep_off_it() {
        let (a, b) = (egui::pos2(400.0, 300.0), egui::pos2(400.0, 318.0));
        let left = Spot::beside(1, 0);
        let (shown, ..) = placing(
            |placer| {
                placer.held.insert(Named::Node(0), left);
            },
            &[
                (Named::Node(0), a, Force::Free),
                (Named::Node(1), b, Force::Free),
            ],
        );
        assert_eq!(shown[&Named::Node(0)].at, left, "where it first went");
        let rect = |key: Named, node: egui::Pos2| {
            let label = &shown[&key];
            egui::Rect::from_min_size(node + label.offset, label.galley.size())
        };
        assert!(
            !rect(Named::Node(1), b).intersects(rect(Named::Node(0), a)),
            "the new name keeps off the held one"
        );
    }

    #[test]
    fn a_name_held_far_from_its_node_takes_a_nearer_place() {
        let node = egui::pos2(400.0, 300.0);
        let far = Spot::beside(0, 6);
        let (shown, ..) = placing(
            |placer| {
                placer.held.insert(Named::Node(0), far);
            },
            &[(Named::Node(0), node, Force::Free)],
        );
        let at = shown[&Named::Node(0)].at;
        assert_eq!(at.shift, 0, "right beside its node: {at:?}");
    }

    #[test]
    fn a_held_name_gives_way_to_a_lit_node_under_it_and_none_goes_far() {
        let (a, b) = (egui::pos2(400.0, 300.0), egui::pos2(440.0, 300.0));
        // Held to the right of `a`, where `b` has since lit up.
        let right = Spot::beside(0, 0);
        let (shown, ..) = placing(
            |placer| {
                placer.held.insert(Named::Node(0), right);
                placer.blocking = vec![(0, a, 5.0), (1, b, 5.0)];
            },
            &[(Named::Node(0), a, Force::Free)],
        );
        let label = &shown[&Named::Node(0)];
        let rect = egui::Rect::from_min_size(a + label.offset, label.galley.size());
        assert!(!rect.contains(b), "off the lit node: {rect:?}");
        assert!(near(rect, a, 5.0), "and still near its own: {rect:?}");
    }

    #[test]
    fn a_name_in_a_crowd_of_lit_nodes_goes_unnamed_rather_than_over_one() {
        let node = egui::pos2(400.0, 300.0);
        // Lit nodes all round, near enough that every place beside it lies over one.
        let mut crowd = vec![(0, node, 5.0)];
        for x in -8_i8..=8 {
            for y in -8_i8..=8 {
                if (x, y) != (0, 0) {
                    let at = node + egui::vec2(f32::from(x) * 12.0, f32::from(y) * 9.0);
                    crowd.push((crowd.len(), at, 4.0));
                }
            }
        }
        let (shown, ..) = placing(
            |placer| placer.blocking.clone_from(&crowd),
            &[(Named::Node(0), node, Force::Free)],
        );
        if let Some(label) = shown.get(&Named::Node(0)) {
            let rect = egui::Rect::from_min_size(node + label.offset, label.galley.size());
            let pill = rect.expand2(PILL_PAD);
            assert!(
                crowd[1..]
                    .iter()
                    .all(|&(_, at, r)| pill.distance_to_pos(at) >= r * 0.5),
                "over a lit node: {rect:?}"
            );
        }
    }

    #[test]
    fn the_cursor_keeps_the_place_its_name_holds() {
        let node = egui::pos2(400.0, 300.0);
        let nudged = Spot::beside(1, 1);
        let (shown, overlaid, _) = placing(
            |placer| {
                placer.held.insert(Named::Node(0), nudged);
                placer.beside.insert(Named::Node(0));
            },
            &[(Named::Node(0), node, Force::Free)],
        );
        assert_eq!(shown[&Named::Node(0)].at, nudged, "no jump to its node");
        assert!(!overlaid.contains(&Named::Node(0)));
    }

    #[test]
    fn the_key_list_fits_whole_at_1600_by_1000() {
        for browsing in [false, true] {
            let ctx = egui::Context::default();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 1000.0));
            let area = egui::Rect::from_min_max(egui::pos2(0.0, 24.0), screen.max);
            let mut open = true;
            for _ in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                };
                let mut output = ctx.run_ui(input, |ui| {
                    let view = Showing { browsing };
                    key_help(ui.ctx(), area, &mut open, view);
                });
                output.textures_delta.clear();
            }
            let more = ctx.data(|d| d.get_temp::<bool>(more_below_id()));
            assert_eq!(
                more,
                Some(false),
                "nothing left below, browsing: {browsing}"
            );
        }
    }

    #[test]
    fn the_cursor_is_named_beside_its_node_without_taking_room() {
        let node = egui::pos2(400.0, 300.0);
        // A name already on every side of the cursor's node.
        let crowd = |placer: &mut Placer| {
            for side in 0..SIDES.len() {
                let rect = Spot::beside(side, 0).rect(node, 5.0, egui::vec2(70.0, 14.0));
                placer.taken.push(rect);
            }
            placer.beside.insert(Named::Node(0));
        };
        let (shown, overlaid, taken) = placing(crowd, &[(Named::Node(0), node, Force::Never)]);
        assert_eq!(shown[&Named::Node(0)].at.shift, 0, "right beside its node");
        assert!(overlaid.contains(&Named::Node(0)), "drawn over the others");
        assert_eq!(taken, SIDES.len(), "and taking no room from them");
    }

    #[test]
    fn a_name_never_sits_nearer_another_node_than_its_own() {
        let node = egui::pos2(400.0, 300.0);
        let nodes = [(0, node, 5.0), (1, egui::pos2(520.0, 300.0), 5.0)];
        // To the right of its node, touching it: its own is nearest.
        let beside = egui::Rect::from_min_size(egui::pos2(411.0, 292.0), egui::vec2(50.0, 16.0));
        assert_eq!(misread_by(beside, 0, node, 5.0, &nodes), 0);
        // Nudged down and across, it lies nearer the other.
        let nudged = egui::Rect::from_min_size(egui::pos2(440.0, 310.0), egui::vec2(50.0, 16.0));
        assert!(misread_by(nudged, 0, node, 5.0, &nodes) > 0);
        assert_eq!(misread_by(nudged, 1, nodes[1].1, 5.0, &nodes), 0);
    }

    #[test]
    fn search_reads_dots_as_paths() {
        let place = |query| {
            search_rank(
                query,
                "rust:axum_server::services::orders::place",
                "orders.place",
            )
        };
        assert_eq!(place("orders.place"), Some(0), "as the graph names it");
        assert_eq!(place("orders::place"), Some(0), "as Rust writes it");
        assert_eq!(place("ord"), Some(1));
        assert_eq!(place("services.orders"), Some(2));
        assert_eq!(
            place("server place"),
            Some(3),
            "words in order along the path"
        );
        assert_eq!(place("place orders"), None, "but not out of order");
        assert_eq!(place("quote"), None);
    }

    #[test]
    fn labels_hold_their_side_and_ease_to_a_new_one() {
        let ctx = egui::Context::default();
        let node = egui::pos2(400.0, 300.0);
        let right = egui::Rect::from_min_max(egui::pos2(405.0, 280.0), egui::pos2(600.0, 320.0));
        let left = egui::Rect::from_min_max(egui::pos2(200.0, 280.0), egui::pos2(395.0, 320.0));

        let (side, start) = frame(&ctx, node, &[right]);
        assert_eq!(side, 1, "the right is taken, so it goes left");
        let (side, offset) = frame(&ctx, node, &[]);
        assert_eq!(side, 1, "the right freeing up does not flip it back");
        assert_eq!(offset, start);

        let (side, offset) = frame(&ctx, node, &[left]);
        assert_eq!(side, 0, "covered, it moves");
        assert!(
            offset.x > start.x && offset.x < 0.0,
            "it slides over rather than jumping: {start:?} → {offset:?}"
        );
        let mut settled = offset;
        for _ in 0..60 {
            settled = frame(&ctx, node, &[left]).1;
        }
        assert!(settled.x > 0.0, "and gets there: {settled:?}");
    }

    #[test]
    fn the_key_list_fits_a_small_window() {
        for browsing in [false, true] {
            let ctx = egui::Context::default();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0));
            // Below the top bar.
            let area = egui::Rect::from_min_max(egui::pos2(0.0, 30.0), screen.max);
            let mut open = true;
            for _ in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                };
                let mut output = ctx.run_ui(input, |ui| {
                    let view = Showing { browsing };
                    key_help(ui.ctx(), area, &mut open, view);
                });
                output.textures_delta.clear();
            }
            let shown = ctx
                .memory(|m| m.area_rect(egui::Id::new("key_help")))
                .expect("the list shows");
            eprintln!("{browsing}: {shown:?}");
            assert!(
                area.contains_rect(shown),
                "inside the window: {shown:?} in {area:?}"
            );
        }
    }
}
