//! egui around the graph: call stack and journeys on top, and a side panel, hidden at
//! first, with the view's settings, outcomes, source and details; on the canvas, the
//! focus with what calls it and what it calls, names beside nodes, a key to the
//! colors, and a search box on `/`.

use std::{
    collections::{HashMap, HashSet},
    fmt::Write as _,
    path::Path,
};

use bevy::{camera::Viewport, prelude::*, window::PrimaryWindow};
use bevy_egui::{
    EguiContext, EguiContexts, EguiInput, EguiPreUpdateSet, EguiPrimaryContextPass,
    PrimaryEguiContext, egui,
};
use codeviz_core::{Change, Level, SpanTarget};
use codeviz_ir::{ExitKind, Role, UnitId};

use crate::{
    actions::{self, Place, Request, Requests, Toast},
    camera::OrbitCamera,
    diff_view::{self, DiffStyle, DiffView},
    explorer::{Clustering, Confidence, Explorer, Side, SpanRef, Track},
    graph_view::{self, CALLEE, CALLER, Relation, relation},
    journey_panel::{self, JourneyPanel, PanelMode},
    lens::{self, Measure, Source},
    peek::{self, Code, Peek, Sources},
    scene::{PALETTE, accent, lane_color},
    source_diff,
};

pub(crate) struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Search>()
            .init_resource::<KeyHelp>()
            .init_resource::<StartCard>()
            .init_resource::<Chrome>()
            .init_resource::<Sources>()
            .init_resource::<Peek>()
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
    /// The row Up and Down move and Enter opens.
    highlight: usize,
    /// What the query last found, best first, and the query and focus it found it
    /// for: ranking every function takes too long to do each frame in a big codebase.
    found: Option<(String, Option<usize>, Vec<Candidate>)>,
}

const ERROR: egui::Color32 = egui::Color32::from_rgb(240, 110, 100);
const LINK: egui::Color32 = egui::Color32::from_rgb(160, 190, 230);
const OK: egui::Color32 = egui::Color32::from_rgb(120, 210, 140);
pub(crate) const MUTED: egui::Color32 = egui::Color32::from_rgb(140, 148, 165);

/// What draws around the graph: everything, or bare, with only the graph and the
/// journey; and whether the side panel and the search box are open.
#[derive(Resource, Default)]
pub(crate) struct Chrome {
    pub(crate) bare: bool,
    pub(crate) side: bool,
    pub(crate) search: bool,
}

/// Whether the card of places to start is showing.
#[derive(Resource, Debug, Default)]
pub(crate) struct StartCard(pub(crate) bool);

#[expect(clippy::too_many_arguments, reason = "a Bevy system")]
fn draw_ui(
    mut contexts: EguiContexts,
    mut explorer: ResMut<Explorer>,
    mut search: ResMut<Search>,
    mut sources: ResMut<Sources>,
    mut panel: ResMut<JourneyPanel>,
    mut help: ResMut<KeyHelp>,
    mut chrome: ResMut<Chrome>,
    mut start: ResMut<StartCard>,
    time: Res<Time>,
    mut diff: Option<ResMut<DiffView>>,
    mut cameras: Query<(&mut Camera, &GlobalTransform, &mut OrbitCamera)>,
    window: Single<&Window, With<PrimaryWindow>>,
    mut requests: ResMut<Requests>,
    (toast, mut peek): (Res<Toast>, ResMut<Peek>),
) -> Result {
    explorer.bypass_change_detection().refresh_names();
    let ctx = contexts.ctx_mut()?;
    let mut root_ui = egui::Ui::new(
        ctx.clone(),
        "codeviz".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.viewport_rect()),
    );

    let mut code = Code {
        sources: &mut sources,
        requests: &mut requests,
    };
    let [top, bottom, left, right] = if chrome.bare {
        [0.0; 4]
    } else {
        panels(
            &mut root_ui,
            &mut explorer,
            &mut panel,
            &mut code,
            &mut chrome,
            diff.as_deref(),
            time.elapsed_secs(),
        )
    };

    let screen = ctx.viewport_rect();
    let central = egui::Rect::from_min_max(
        egui::pos2(screen.min.x + left, screen.min.y + top),
        egui::pos2(screen.max.x - right, screen.max.y - bottom),
    );
    let (graph_area, panel_area) =
        journey_panel::split(panel.mode, central, journey_panel::extent(&explorer));
    if let Ok((mut camera, transform, mut orbit)) = cameras.single_mut() {
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
        draw_graph_labels(
            &painter,
            &to_screen,
            &explorer,
            diff.as_deref(),
            transform.up().into(),
        );
        if let Some(diff) = diff.as_deref_mut().filter(|_| !chrome.bare) {
            diff_view::diff_card(ctx, graph_area, &mut explorer, diff);
        }
        let card = if chrome.bare {
            None
        } else {
            corner_cards(
                ctx,
                graph_area,
                &mut explorer,
                &mut start.0,
                &mut panel,
                &mut code,
                (&mut peek.0, diff.as_deref()),
            )
        };
        if chrome.search && !chrome.bare {
            search_box(
                ctx,
                graph_area,
                &mut explorer,
                &mut search,
                &mut chrome.search,
            );
        }
        if panel.mode != PanelMode::Full && !chrome.bare {
            let guesses = explorer
                .guesses_in_view()
                .then(|| explorer.counts_guesses());
            legend(ctx, above(graph_area, panel_area), diff.is_some(), guesses);
        }
        // Only while there is somewhere to follow: the overview frames everything.
        if orbit.free && !chrome.bare && explorer.anchor().is_some() {
            camera_free(ctx, graph_area, card, &explorer, &mut orbit);
        }
        if help.0 {
            key_help(ctx, graph_area, &mut help.0);
        }
        if !chrome.bare {
            actions::draw_toast(ctx, graph_area, &toast, time.elapsed_secs());
        }
        if let Some(area) = panel_area {
            let diff = diff.as_deref();
            docked_journey(ctx, area, &mut explorer, &mut panel, diff, &to_screen);
        }
    }
    Ok(())
}

/// The journey in `area`, tied to the graph: its cursor's bar to its node, and the bar
/// under the pointer to the node it stands for.
fn docked_journey(
    ctx: &egui::Context,
    area: egui::Rect,
    explorer: &mut Explorer,
    panel: &mut JourneyPanel,
    diff: Option<&DiffView>,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
) {
    let tethers = journey_panel::draw(ctx, area, explorer, panel, diff);
    if let (Some(bar), Some(node)) = (tethers.cursor, cursor_node(explorer, to_screen)) {
        tether(ctx, bar, node, false);
    }
    let hovered = explorer
        .hovered_tile
        .and_then(|tile| explorer.tile_point(tile))
        .and_then(to_screen);
    if let (Some(bar), Some(node)) = (tethers.hovered, hovered) {
        tether(ctx, bar, node, true);
    }
}

/// The panels around the graph, and how much of each edge they take: top, bottom, left
/// and right. The side panel shows only when opened (S).
fn panels(
    root_ui: &mut egui::Ui,
    explorer: &mut Explorer,
    panel: &mut JourneyPanel,
    code: &mut Code<'_>,
    chrome: &mut Chrome,
    diff: Option<&DiffView>,
    now: f32,
) -> [f32; 4] {
    let top = egui::Panel::top("stack")
        .show(root_ui, |ui| {
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let (label, hint) = if chrome.side {
                        ("Hide panel", "Hide the side panel (S)")
                    } else {
                        ("Panel", "Settings, outcomes, source and details (S)")
                    };
                    if ui
                        .selectable_label(chrome.side, label)
                        .on_hover_text(hint)
                        .clicked()
                    {
                        chrome.side = !chrome.side;
                    }
                    if ui
                        .selectable_label(chrome.search, "Search")
                        .on_hover_text("Find a function (/)")
                        .clicked()
                    {
                        chrome.search = !chrome.search;
                    }
                    focus_sides(ui, explorer, now);
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        history_buttons(ui, explorer);
                        breadcrumb(ui, explorer, panel);
                    });
                });
            });
        })
        .response
        .rect
        .height();
    let right = if chrome.side {
        egui::Panel::right("source")
            .resizable(true)
            .default_size(460.0)
            .show(root_ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    view_settings(ui, explorer, diff.is_some());
                    ui.separator();
                    outcomes(ui, explorer);
                    ui.separator();
                    match diff {
                        Some(view) => changed_source(ui, explorer, view, code),
                        None => source(ui, explorer, code),
                    }
                    ui.separator();
                    details(ui, explorer, code.requests);
                });
            })
            .response
            .rect
            .width()
    } else {
        0.0
    };
    [top, 0.0, 0.0, right]
}

/// How the graph is drawn: what the lens shows, and what pulls nodes together.
fn view_settings(ui: &mut egui::Ui, explorer: &mut Explorer, diffing: bool) {
    ui.heading("View");
    reveal_controls(ui, explorer, diffing);
    let mut clustering = explorer.clustering();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Cluster by").small().color(MUTED));
        ui.selectable_value(&mut clustering, Clustering::Modules, "Modules")
            .on_hover_text("Functions in the same container pull together (K)");
        ui.selectable_value(&mut clustering, Clustering::Calls, "Calls")
            .on_hover_text(
                "Only calls pull; containers that hold together mostly call themselves (K)",
            );
    });
    if clustering != explorer.clustering() {
        explorer.set_clustering(clustering);
    }
    let mut counted = explorer.counts_guesses();
    let any = !explorer.graph().guesses.is_empty();
    let hint = if any {
        "Calls the analyzer matched by name, without knowing the type: drawn dotted, \
         and left out of callers, calls and what lights up unless counted (U)"
    } else {
        "Every call here was resolved; none is a guess"
    };
    if ui
        .add_enabled(
            any,
            egui::Checkbox::new(&mut counted, "Count calls guessed by name"),
        )
        .on_hover_text(hint)
        .changed()
    {
        explorer.set_guesses(counted);
    }
}

/// Where the cursor's node is on screen, for the tether from its bar.
fn cursor_node(
    explorer: &Explorer,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
) -> Option<egui::Pos2> {
    explorer
        .focus()
        .and_then(|unit| explorer.map().tile_of(unit))
        .and_then(|tile| explorer.tile_point(tile))
        .and_then(to_screen)
}

/// The part of the graph above a docked journey, for what sits at its bottom.
fn above(graph: egui::Rect, panel: Option<egui::Rect>) -> egui::Rect {
    panel.map_or(graph, |p| {
        egui::Rect::from_min_max(graph.min, egui::pos2(graph.max.x, p.min.y))
    })
}

/// A line from a bar in the journey up to its node in the graph: faint for the
/// cursor's, bright for the one under the pointer.
fn tether(ctx: &egui::Context, bar: egui::Rect, node: egui::Pos2, bright: bool) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("tether"),
    ));
    let from = if bright {
        egui::pos2(bar.center().x, bar.min.y)
    } else {
        egui::pos2(bar.min.x + 2.0, bar.min.y)
    };
    let alpha = if bright { 220 } else { 90 };
    painter.line_segment(
        [from, node],
        egui::Stroke::new(
            if bright { 1.5 } else { 1.0 },
            egui::Color32::from_white_alpha(alpha),
        ),
    );
    painter.circle_filled(from, 2.5, egui::Color32::WHITE);
    if bright {
        painter.circle_stroke(node, 9.0, egui::Stroke::new(1.5, egui::Color32::WHITE));
    }
}

/// Back and forward through where the view has been, as a browser's.
fn history_buttons(ui: &mut egui::Ui, explorer: &mut Explorer) {
    if ui
        .add_enabled(explorer.can_go_back(), egui::Button::new("‹").small())
        .on_hover_text("Back to the view before (Backspace, Alt+←)")
        .clicked()
    {
        explorer.go_back();
    }
    if ui
        .add_enabled(explorer.can_go_forward(), egui::Button::new("›").small())
        .on_hover_text("Forward again (Alt+→)")
        .clicked()
    {
        explorer.go_forward();
    }
    ui.separator();
}

fn breadcrumb(ui: &mut egui::Ui, explorer: &mut Explorer, panel: &mut JourneyPanel) {
    ui.horizontal(|ui| {
        for (mode, label, hint) in [
            (
                PanelMode::Hidden,
                "Graph",
                "The graph alone (J hides the journey)",
            ),
            (
                PanelMode::Docked,
                "Graph + journey",
                "The journey docked over the graph (J)",
            ),
            (
                PanelMode::Full,
                "Journey",
                "The journey below a strip of graph (M)",
            ),
        ] {
            if ui
                .selectable_label(panel.mode == mode, label)
                .on_hover_text(hint)
                .clicked()
            {
                panel.mode = mode;
            }
        }
        ui.separator();
        journeys(ui, explorer);
        let Some(selected) = explorer.selected() else {
            let hint = match explorer.starts().first() {
                Some(start) => format!(
                    "The whole codebase. Click a node, or Enter to start at {}",
                    start.label
                ),
                None => "The whole codebase. Click a node to start there".to_owned(),
            };
            ui.label(egui::RichText::new(hint).color(MUTED));
            return;
        };
        let mut chosen = None;
        for (index, frame) in explorer.stack(selected).into_iter().enumerate() {
            if index > 0 {
                ui.label(egui::RichText::new("›").color(MUTED));
            }
            let text = egui::RichText::new(explorer.label(frame));
            let text = if frame == selected {
                text.strong()
            } else {
                text
            };
            if ui.link(text).clicked() {
                chosen = Some(frame);
            }
        }
        if let Some(frame) = chosen {
            explorer.select(Some(frame));
        }
    });
}

/// One chip per open journey, in its accent: click to make it active, × to close.
/// Shown once there is more than one.
fn journeys(ui: &mut egui::Ui, explorer: &mut Explorer) {
    let count = explorer.tracks().len();
    if count < 2 {
        return;
    }
    let mut activate = None;
    let mut close = None;
    for index in 0..count {
        let text = egui::RichText::new(format!("■ {}", track_title(explorer, index)))
            .color(egui_color(accent(index)));
        if ui
            .selectable_label(index == explorer.active(), text)
            .on_hover_text("Make active (Tab cycles)")
            .clicked()
        {
            activate = Some(index);
        }
        if ui
            .small_button("×")
            .on_hover_text("Close (X closes the active one)")
            .clicked()
        {
            close = Some(index);
        }
    }
    ui.separator();
    if let Some(track) = activate
        && let Some(span) = explorer.tracks().get(track).and_then(Track::selected)
    {
        explorer.select_in(SpanRef { track, span });
    }
    if let Some(index) = close {
        explorer.close_track(index);
    }
}

/// A function search can find.
#[derive(Debug)]
struct Candidate {
    id: UnitId,
    /// What the graph calls it, without what tells it apart.
    name: String,
    /// Its own part of that: `closure 3` of `draw › closure 3`.
    own: String,
    /// Where it lives: `codeviz_core.system.System`.
    place: String,
    /// The route or command an entry point is reached by.
    route: Option<String>,
    kind: String,
    /// Whether it is nested in another function, as a closure is, and so lives on
    /// that function's node.
    nested: bool,
    /// Calls between it and the focus, more for what is further or out of reach.
    distance: u32,
    /// How many functions call it.
    callers: usize,
}

/// What search knows of every function beyond its name, gathered once per query.
struct Census<'a> {
    explorer: &'a Explorer,
    /// How many nodes call each node.
    callers: Vec<usize>,
    routes: HashMap<&'a UnitId, &'a str>,
}

impl<'a> Census<'a> {
    fn new(explorer: &'a Explorer) -> Self {
        let mut callers = vec![0; explorer.map().tiles.len()];
        for &(from, to) in &explorer.graph().calls {
            if from != to
                && let Some(count) = callers.get_mut(to)
            {
                *count += 1;
            }
        }
        let routes = explorer
            .system()
            .entries()
            .iter()
            .map(|e| (&e.unit, e.label.as_str()))
            .collect();
        Self {
            explorer,
            callers,
            routes,
        }
    }

    fn candidate(&self, unit: &codeviz_ir::Unit) -> Candidate {
        let explorer = self.explorer;
        let path = explorer.unit_path(&unit.id);
        let tile = explorer.map().tile_of(&unit.id);
        Candidate {
            id: unit.id.clone(),
            name: path.short(),
            own: path.own().to_owned(),
            place: path.container(),
            route: self.routes.get(&unit.id).map(|&route| route.to_owned()),
            kind: unit.kind.clone(),
            nested: unit
                .parent
                .as_ref()
                .and_then(|p| explorer.any_unit(p))
                .is_some_and(|p| p.role == Role::Invocable),
            distance: tile.map_or(u32::MAX, |tile| distance(explorer, tile)),
            callers: tile
                .and_then(|tile| self.callers.get(tile))
                .copied()
                .unwrap_or_default(),
        }
    }
}

/// How far a node is from the focus: the calls between them either way, else a step
/// past any of those for one in the focus's module, else further still.
fn distance(explorer: &Explorer, tile: usize) -> u32 {
    const MODULE: u32 = 50;
    let Some(focus) = explorer.graph_focus() else {
        return MODULE + 1;
    };
    if tile == focus {
        return 0;
    }
    let hops = explorer.reach().and_then(|reach| {
        [reach.upstream.get(&tile), reach.downstream.get(&tile)]
            .into_iter()
            .flatten()
            .min()
            .copied()
    });
    let graph = explorer.graph();
    match hops {
        Some(hops) => hops.min(MODULE - 1),
        None if graph.cluster(tile) == graph.cluster(focus) => MODULE,
        None => MODULE + 1,
    }
}

/// The candidates `query` finds, best first: by how well it matches, then nearest the
/// focus, then most called. A closure lives on its function's node, so a search that
/// only finds its function's name wants the function: closures come up only when the
/// query names them, `closure` or a nested function's own name, and then after the
/// functions that are not nested.
fn search_order(query: &str, candidates: &[Candidate]) -> Vec<usize> {
    let mut found: Vec<(bool, u8, u32, std::cmp::Reverse<usize>, usize, usize)> = candidates
        .iter()
        .enumerate()
        .filter(|(_, c)| !c.nested || names_own(query, &c.own))
        .filter_map(|(index, c)| {
            let by_name = search_rank(query, c.id.as_str(), &c.name);
            let by_route = c
                .route
                .as_deref()
                .and_then(|route| search_rank(query, "", route));
            let rank = by_name.into_iter().chain(by_route).min()?;
            Some((
                c.nested,
                rank,
                c.distance,
                std::cmp::Reverse(c.callers),
                c.name.len(),
                index,
            ))
        })
        .collect();
    found.sort_by(|a, b| {
        a.cmp(b)
            .then_with(|| candidates[a.5].name.cmp(&candidates[b.5].name))
    });
    found.into_iter().map(|f| f.5).collect()
}

/// Whether a word of `query` is in `own`, a nested function's own name.
fn names_own(query: &str, own: &str) -> bool {
    let own = own.to_lowercase();
    query
        .to_lowercase()
        .split(|c: char| c == '.' || c == ':' || c == '›' || c.is_whitespace())
        .any(|word| !word.is_empty() && own.contains(word))
}

/// What `query` finds, best first: with no query, the entry points by route.
fn search_rows(explorer: &Explorer, query: &str) -> Vec<Candidate> {
    let system = explorer.system();
    let census = Census::new(explorer);
    if query.trim().is_empty() {
        let mut entries: Vec<Candidate> = system
            .entries()
            .iter()
            .filter_map(|e| explorer.any_unit(&e.unit))
            .map(|unit| census.candidate(unit))
            .collect();
        entries.sort_by(|a, b| a.route.cmp(&b.route));
        entries.dedup_by(|a, b| a.id == b.id);
        return entries;
    }
    let candidates: Vec<Candidate> = system
        .units()
        .filter(|u| u.role == Role::Invocable)
        .map(|unit| census.candidate(unit))
        .collect();
    let order = search_order(query, &candidates);
    let mut candidates: Vec<Option<Candidate>> = candidates.into_iter().map(Some).collect();
    order
        .into_iter()
        .take(200)
        .filter_map(|index| candidates[index].take())
        .collect()
}

/// A box to find a function by name, over the top of the graph: Up and Down pick a
/// row, Enter or a click opens it, Escape closes the box. Each row names the function
/// as the graph does, with where it lives after it.
fn search_box(
    ctx: &egui::Context,
    area: egui::Rect,
    explorer: &mut Explorer,
    search: &mut Search,
    open: &mut bool,
) {
    let focus = explorer.graph_focus();
    if search
        .found
        .as_ref()
        .is_none_or(|(query, at, _)| *query != search.query || *at != focus)
    {
        let rows = search_rows(explorer, &search.query);
        search.found = Some((search.query.clone(), focus, rows));
        search.highlight = 0;
    }
    let rows = search
        .found
        .as_ref()
        .map_or(&[][..], |(_, _, rows)| rows.as_slice());
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
                    ui.set_width(460.0);
                    // Taken before the field sees them, which would move its caret.
                    let (up, down) = ui.input_mut(|i| {
                        (
                            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
                            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
                        )
                    });
                    let last = rows.len().saturating_sub(1);
                    if down {
                        search.highlight = (search.highlight + 1).min(last);
                    }
                    if up {
                        search.highlight = search.highlight.saturating_sub(1);
                    }
                    search.highlight = search.highlight.min(last);
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut search.query)
                            .hint_text("search functions…")
                            .desired_width(f32::INFINITY),
                    );
                    field.request_focus();
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        chosen = rows.get(search.highlight).map(|row| row.id.clone());
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        *open = false;
                    }
                    if rows.is_empty() {
                        ui.label(egui::RichText::new("no function matches").color(MUTED));
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("units")
                        .max_height(280.0)
                        .show(ui, |ui| {
                            for (index, row) in rows.iter().enumerate() {
                                let highlighted = index == search.highlight;
                                let response = search_row(ui, explorer, row, highlighted);
                                if highlighted && (up || down) {
                                    response.scroll_to_me(None);
                                }
                                if response.clicked() {
                                    chosen = Some(row.id.clone());
                                }
                            }
                        });
                });
        });
    if let Some(id) = chosen {
        match explorer.map().tile_of(&id) {
            Some(tile) => explorer.open_at(tile),
            None => explorer.set_root(id),
        }
        *open = false;
    }
}

/// One search result: its name, an entry point's route, where it lives and its kind,
/// muted, all one row to click.
fn search_row(
    ui: &mut egui::Ui,
    explorer: &Explorer,
    row: &Candidate,
    highlighted: bool,
) -> egui::Response {
    let font = egui::FontId::proportional(13.0);
    let small = egui::FontId::proportional(11.5);
    let mut job = egui::text::LayoutJob::default();
    let text = |color| egui::TextFormat {
        font_id: font.clone(),
        color,
        ..default()
    };
    let muted = |color| egui::TextFormat {
        font_id: small.clone(),
        color,
        valign: egui::Align::Center,
        ..default()
    };
    job.append(&row.name, 0.0, text(egui::Color32::WHITE));
    if let Some(route) = &row.route {
        job.append(route, 8.0, muted(egui::Color32::from_rgb(200, 205, 216)));
    }
    if !row.place.is_empty() {
        job.append(&row.place, 8.0, muted(MUTED));
    }
    job.append(&row.kind, 8.0, muted(MUTED.gamma_multiply(0.7)));
    job.wrap = egui::text::TextWrapping::truncate_at_width(ui.available_width());
    let full = explorer.unit_path(&row.id).full();
    ui.add(egui::Button::selectable(highlighted, job).frame_when_inactive(highlighted))
        .on_hover_text(full)
}

/// How many places to start the card offers.
const STARTS_SHOWN: usize = 6;

/// Where to start, over the top left of the graph: the entry points that reach the
/// most first, or without any the functions nothing calls; demo and test code last.
/// A click opens one; W hides the card.
fn start_card(
    ctx: &egui::Context,
    area: egui::Rect,
    explorer: &mut Explorer,
    open: &mut bool,
    panel: &mut JourneyPanel,
) -> Option<egui::Rect> {
    if explorer.starts().is_empty() {
        return None;
    }
    let entries = explorer.starts().iter().any(|s| explorer.is_entry(s.tile));
    let total = explorer.map().tiles.len();
    let mut chosen = None;
    let shown = egui::Area::new("starts".into())
        .pivot(egui::Align2::LEFT_TOP)
        .fixed_pos(egui::pos2(area.min.x + 12.0, area.min.y + 10.0))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_premultiplied(11, 13, 19, 225))
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(50)))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_max_width(340.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Start here").strong());
                        let about = if entries {
                            "entry points, by how much they reach"
                        } else {
                            "no entry points declared: what nothing calls"
                        };
                        ui.label(egui::RichText::new(about).small().color(MUTED));
                    });
                    for (index, start) in explorer.starts().iter().take(STARTS_SHOWN).enumerate() {
                        ui.horizontal(|ui| {
                            // An entry by its route, anything else by its name as the
                            // graph has it.
                            let (mark, label) = if explorer.is_entry(start.tile) {
                                ("▶ ", start.label.clone())
                            } else {
                                ("", node_name(explorer, start.tile))
                            };
                            let text = egui::RichText::new(format!("{mark}{}", short(&label, 34)));
                            let text = if start.aside { text.color(MUTED) } else { text };
                            let hint = if index == 0 { " (Enter)" } else { "" };
                            if ui
                                .selectable_label(false, text)
                                .on_hover_text(format!("Open its journey{hint}"))
                                .clicked()
                            {
                                chosen = Some(start.tile);
                            }
                            let note = if start.aside {
                                format!("demo or test · reaches {}", start.reach)
                            } else {
                                format!("reaches {} of {total}", start.reach)
                            };
                            ui.label(egui::RichText::new(note).small().color(MUTED));
                        });
                    }
                    let more = explorer.starts().len().saturating_sub(STARTS_SHOWN);
                    let footer = if more > 0 {
                        format!("{more} more under / search · W hides this")
                    } else {
                        "W hides this".to_owned()
                    };
                    ui.label(egui::RichText::new(footer).small().color(MUTED));
                });
        });
    if let Some(tile) = chosen {
        explorer.open_at(tile);
        if panel.mode == PanelMode::Hidden {
            panel.mode = PanelMode::Docked;
        }
        *open = false;
    }
    Some(shown.response.rect)
}

fn outcomes(ui: &mut egui::Ui, explorer: &mut Explorer) {
    ui.heading("Outcomes");
    let mut chosen = None;
    let current = explorer.scenario();
    for (index, scenario) in explorer.scenarios().iter().enumerate() {
        let (mark, color) = match scenario.outcome.kind {
            ExitKind::Return => ("✔", OK),
            _ => ("×", ERROR),
        };
        let text = egui::RichText::new(format!("{mark} {}", scenario.outcome.label)).color(color);
        let via: Vec<String> = scenario
            .chain
            .iter()
            .skip(1)
            .map(|site| explorer.unit_name(&site.unit))
            .collect();
        let response = ui.selectable_label(current == Some(index), text);
        let response = if via.is_empty() {
            response
        } else {
            response.on_hover_text(format!("via {}", via.join(" → ")))
        };
        if response.clicked() {
            chosen = Some(Some(index));
        }
    }
    let all = egui::RichText::new("All paths at once").color(MUTED);
    if ui.selectable_label(current.is_none(), all).clicked() {
        chosen = Some(None);
    }
    if let Some(scenario) = chosen {
        explorer.set_scenario(scenario);
    }
}

/// How well `query` finds a function, best first, or `None` when it does not: its
/// name exactly, then its name's start, then anywhere in its path, then each word of
/// the query in order along the path. `.` and `::` are the same, so `orders.place`
/// finds `services::orders::place`.
fn search_rank(query: &str, id: &str, name: &str) -> Option<u8> {
    let normal = |text: &str| text.to_lowercase().replace("::", ".").replace(" › ", ".");
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

/// The caller's source around the line that made the selected call; for the root,
/// its own definition.
fn source(ui: &mut egui::Ui, explorer: &Explorer, code: &mut Code<'_>) {
    let Some(selected) = explorer.selected() else {
        return;
    };
    let (owner, location, title) = if let Some((caller, location)) = explorer.call_site(selected) {
        let title = format!(
            "{} calls {}",
            explorer.unit_name(caller),
            explorer.label(selected)
        );
        (caller.clone(), location.clone(), title)
    } else {
        let Some(unit) = explorer.unit(selected) else {
            return;
        };
        let Some(location) = unit.location.clone() else {
            ui.label(egui::RichText::new("No source location.").color(MUTED));
            return;
        };
        let title = format!("{} starts here", explorer.label(selected));
        (unit.id.clone(), location, title)
    };
    ui.label(egui::RichText::new(title).strong());
    let path = explorer.system().local_path(&owner, &location.uri);
    let place = path.clone().map(|file| Place {
        file,
        line: location.range.map_or(1, |r| r.start_line),
        column: location.range.map_or(1, |r| r.start_col),
    });
    location_link(ui, &location.to_string(), place, code.requests);
    let Some(path) = path else {
        return;
    };
    let Some(lines) = code.sources.lines(&path) else {
        cannot_read(ui, &path);
        return;
    };
    let line = location.range.map_or(1, |r| r.start_line) as usize;
    // The whole enclosing function when known, otherwise a window around the line.
    let (first, last) = explorer
        .system()
        .unit(&owner)
        .and_then(|u| u.location.as_ref())
        .and_then(|l| l.range)
        .filter(|r| (r.start_line as usize..=r.end_line as usize).contains(&line))
        .map_or((line.saturating_sub(8).max(1), line + 8), |r| {
            (r.start_line as usize, r.end_line as usize)
        });
    let font = egui::FontId::monospace(12.5);
    for number in first..=last.min(lines.len()) {
        let Some(text) = lines.get(number - 1) else {
            break;
        };
        let current = number == line;
        let row = egui::RichText::new(format!("{number:>4} {text}")).font(font.clone());
        let row = if current {
            row.color(egui::Color32::WHITE)
                .background_color(egui::Color32::from_rgb(52, 70, 110))
        } else {
            row.color(egui::Color32::from_rgb(190, 196, 210))
        };
        let response = ui.label(row);
        if current {
            response.scroll_to_me(Some(egui::Align::Center));
        }
    }
}

/// The focused function as the diff changed it: its old and new source as one
/// unified diff.
fn changed_source(ui: &mut egui::Ui, explorer: &Explorer, view: &DiffView, code: &mut Code<'_>) {
    let Some((tile, unit)) = explorer
        .graph_focus()
        .and_then(|tile| Some((tile, explorer.tile_unit(tile)?.clone())))
    else {
        ui.label(egui::RichText::new("Click a node to see how it changed.").color(MUTED));
        return;
    };
    let change = view.change(tile);
    let heading = format!(
        "{}  {}",
        change.map_or(" ", diff_view::sign),
        explorer.unit_name(&unit)
    );
    let color = change.map_or(egui::Color32::WHITE, |c| {
        egui_color(diff_view::change_color(c))
    });
    ui.label(
        egui::RichText::new(heading)
            .strong()
            .size(15.0)
            .color(color),
    );
    let mut side = |system: &codeviz_core::System| -> Option<(Vec<String>, usize, String)> {
        let location = system.unit(&unit)?.location.as_ref()?;
        let range = location.range?;
        let path = system.local_path(&unit, &location.uri)?;
        let lines = code.sources.lines(&path)?;
        let (first, last) = (range.start_line as usize, range.end_line as usize);
        let body = lines
            .get(first.checked_sub(1)?..last.min(lines.len()))?
            .to_vec();
        Some((body, first, location.uri.clone()))
    };
    let (old, new) = (side(view.old()), side(explorer.system()));
    let place = match (&old, &new) {
        (Some((_, a, uri_a)), Some((_, b, uri_b))) if uri_a != uri_b || a != b => {
            format!("was {uri_a}:{a}, now {uri_b}:{b}")
        }
        (_, Some((_, line, uri))) | (Some((_, line, uri)), None) => format!("{uri}:{line}"),
        (None, None) => {
            ui.label(egui::RichText::new("No source to show.").color(MUTED));
            return;
        }
    };
    location_link(
        ui,
        &place,
        Place::of(explorer.system(), &unit),
        code.requests,
    );
    if let Some(was) = view.moved_from(tile) {
        ui.label(
            egui::RichText::new(format!("was {was}"))
                .small()
                .color(color),
        );
    }
    if change.is_none() {
        ui.label(egui::RichText::new("Unchanged.").small().color(MUTED));
    }
    source_diff::show(ui, as_side(old.as_ref()), as_side(new.as_ref()));
}

fn as_side(side: Option<&(Vec<String>, usize, String)>) -> Option<source_diff::Side<'_>> {
    side.map(|(lines, first, _)| source_diff::Side {
        lines: lines.as_slice(),
        first: *first,
    })
}

fn details(ui: &mut egui::Ui, explorer: &mut Explorer, requests: &mut Requests) {
    let Some(selected) = explorer.selected() else {
        return;
    };
    let Some(span) = explorer.span(selected).cloned() else {
        return;
    };
    ui.label(
        egui::RichText::new(explorer.label(selected))
            .strong()
            .size(15.0),
    );
    if let Some(unit) = explorer.unit(selected) {
        ui.label(egui::RichText::new(unit.id.as_str()).small().color(MUTED));
        copy_buttons(ui, explorer.system(), &unit.id, requests);
        if let Some(signature) = unit.attributes.get("signature") {
            ui.label(egui::RichText::new(signature.to_string()).monospace());
        }
        if let Some(doc) = &unit.doc {
            ui.label(egui::RichText::new(doc).italics());
        }
    }
    if Confidence::of(&span) == Confidence::Guessed {
        ui.label(egui::RichText::new("Guessed: matched by name, not resolved").color(MUTED));
    }
    if let SpanTarget::Ambiguous(ids) = &span.target {
        ui.label("Could be any of:");
        for id in ids {
            ui.label(egui::RichText::new(explorer.unit_name(id)).monospace());
        }
    }
    for condition in &span.conditions {
        ui.label(format!(
            "only when {} ⇒ {}",
            condition.guard.as_deref().unwrap_or("?"),
            condition.arm
        ));
    }
    if span.repeats {
        ui.label("inside a loop: may run many times");
    }
    for exit in &span.exits {
        let color = if exit.kind == ExitKind::Return {
            OK
        } else {
            ERROR
        };
        let text = format!("exits: {}", exit.label.as_deref().unwrap_or("returns"));
        ui.label(egui::RichText::new(text).color(color));
    }
    let divergences: Vec<(usize, String)> = explorer
        .divergences()
        .iter()
        .filter(|d| d.span == selected)
        .map(|d| (d.scenario, d.label.clone()))
        .collect();
    for (scenario, label) in divergences {
        let text = egui::RichText::new(format!("× can fail here: {label}")).color(ERROR);
        if ui.link(text).clicked() {
            explorer.set_scenario(Some(scenario));
        }
    }

    ui.horizontal(|ui| {
        let can_reroot = explorer.unit_id(selected).is_some() && span.parent.is_some();
        if ui
            .add_enabled(can_reroot, egui::Button::new("Start here"))
            .clicked()
        {
            explorer.trace_from_focus();
        }
        if ui
            .add_enabled(explorer.can_go_back(), egui::Button::new("Back"))
            .on_hover_text("The view before (Backspace, Alt+←)")
            .clicked()
        {
            explorer.go_back();
        }
        if let (Some(level), SpanTarget::Unit(unit)) = (span.zoom, &span.target) {
            let label = match level {
                Level::Summary => "Zoom in",
                Level::Detail => "Zoom out",
            };
            if ui.button(label).clicked() {
                explorer.toggle_zoom(unit.clone());
            }
        }
    });
}

/// Where something is written, as a link that opens it in the editor when it is on
/// disk.
pub(crate) fn location_link(
    ui: &mut egui::Ui,
    text: &str,
    place: Option<Place>,
    requests: &mut Requests,
) {
    let text = egui::RichText::new(text).small();
    match place {
        Some(place) => {
            if ui
                .link(text.color(LINK))
                .on_hover_text("Open in the editor (E opens the focus)")
                .clicked()
            {
                requests.push(Request::Edit(place));
            }
        }
        None => {
            ui.label(text.color(MUTED));
        }
    }
}

/// Buttons that copy a function's id, and where it is written as `path:line`.
pub(crate) fn copy_buttons(
    ui: &mut egui::Ui,
    system: &codeviz_core::System,
    unit: &UnitId,
    requests: &mut Requests,
) {
    let place = system.unit(unit).and_then(|u| {
        let location = u.location.as_ref()?;
        Some(format!(
            "{}:{}",
            location.uri,
            location.range.map_or(1, |r| r.start_line)
        ))
    });
    ui.horizontal(|ui| {
        if ui
            .small_button("Copy id")
            .on_hover_text(unit.as_str())
            .clicked()
        {
            requests.push(Request::Copy {
                text: unit.to_string(),
                what: "its id".to_owned(),
            });
        }
        if let Some(place) = place
            && ui
                .small_button("Copy path:line")
                .on_hover_text(&place)
                .clicked()
        {
            requests.push(Request::Copy {
                what: place.clone(),
                text: place,
            });
        }
    });
}

/// Says a source file cannot be read, and how to point at where it is.
pub(crate) fn cannot_read(ui: &mut egui::Ui, path: &Path) {
    ui.label(
        egui::RichText::new(format!("Cannot read {}", actions::home_relative(path))).color(MUTED),
    );
    ui.label(
        egui::RichText::new("Open with --root DIR to read the source from where it is.")
            .small()
            .color(MUTED),
    );
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
pub(crate) fn track_title(explorer: &Explorer, index: usize) -> String {
    let Some(root) = explorer.tracks().get(index).and_then(|t| t.root()) else {
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
/// Text on a bar or a badge filled with a node's color.
const ON_FILL: egui::Color32 = egui::Color32::from_rgb(14, 16, 22);
const PILL: egui::Color32 = egui::Color32::from_rgba_premultiplied(9, 11, 16, 205);

/// The 1-based places in `caller`'s calls, in written order, where it calls `callee`.
fn call_steps(explorer: &Explorer, from: usize, to_tile: usize) -> Vec<usize> {
    explorer
        .graph()
        .sequence(from)
        .iter()
        .enumerate()
        .filter(|&(_, &to)| to == to_tile)
        .map(|(index, _)| index + 1)
        .collect()
}

/// `#2` or `#2, #5`.
fn numbered(steps: &[usize]) -> String {
    steps
        .iter()
        .map(|s| format!("#{s}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// What a node is called on screen: its function's name, told apart from the others
/// in view.
pub(crate) fn node_name(explorer: &Explorer, tile: usize) -> String {
    explorer
        .tile_unit(tile)
        .map_or_else(String::new, |unit| explorer.unit_name(unit))
}

/// The route or command an entry point is reached by: `POST /users`.
fn entry_label(explorer: &Explorer, tile: usize) -> Option<&str> {
    let unit = explorer.tile_unit(tile)?;
    explorer
        .system()
        .entries()
        .iter()
        .find(|e| &e.unit == unit)
        .map(|e| e.label.as_str())
}

/// What a graph label names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Named {
    Node(usize),
    Group(usize),
    Crate(usize),
}

/// What the spotlight picks out that is named come what may; past this many, the rest
/// are named where there is room and listed on the spotlight's card.
const SPOTLIGHT_NAMES: usize = 15;
/// Entry points named first while nothing is in focus, those reaching the most.
const ENTRY_NAMES: usize = 12;
/// Function names shown at most while nothing is in focus.
const OVERVIEW_NAMES: usize = 40;
/// Function names placed at most, past those that must show, so a hub's whole
/// neighborhood cannot stall the frame.
const MAX_NAMES: usize = 120;
/// How wide on screen a crate's ring must be before it is named, in points, and how
/// many crates are named at most, the biggest first.
const CRATE_NAME_RING: f32 = 40.0;
const MAX_CRATE_NAMES: usize = 24;
/// Under several crates, how wide on screen a module's ring must be before it is
/// named, in points.
const MODULE_NAME_RING: f32 = 28.0;

/// Tier, then bigger first, then a stable tiebreak.
type LabelRank = (u8, std::cmp::Reverse<usize>, usize);

/// Where a label may go around its node, in order of preference: beside it, above or
/// below it, then off a corner.
const SIDES: [egui::Align2; 8] = [
    egui::Align2::LEFT_CENTER,
    egui::Align2::RIGHT_CENTER,
    egui::Align2::CENTER_BOTTOM,
    egui::Align2::CENTER_TOP,
    egui::Align2::LEFT_BOTTOM,
    egui::Align2::LEFT_TOP,
    egui::Align2::RIGHT_BOTTOM,
    egui::Align2::RIGHT_TOP,
];

/// How far a label that must show steps out from its node, each time its place is
/// taken, and how many times at most.
const FAN_STEP: f32 = 16.0;
const FAN_STEPS: usize = 4;
/// Room kept around a label's text: its pill.
const PILL_PAD: egui::Vec2 = egui::vec2(5.0, 2.0);
/// How fast a label slides to a new side: the share of the way left it covers in a
/// second is `1 - e^-SLIDE`.
const SLIDE: f32 = 14.0;
/// Seconds a label takes to fade in or out.
const FADE: f32 = 0.18;

/// A label as last drawn, so the next frame eases it instead of jumping it.
#[derive(Clone)]
struct Shown {
    /// Index into [`SIDES`].
    side: usize,
    /// How many steps out from its node it fanned.
    step: usize,
    /// Where its text sits, from its node.
    offset: egui::Vec2,
    opacity: f32,
    galley: std::sync::Arc<egui::Galley>,
    stroke: Option<egui::Color32>,
}

/// Every label drawn last frame.
type ShownLabels = HashMap<Named, Shown>;

/// Places labels so none overlaps another or a node that matters, and draws them
/// eased from where they were: a label keeps its side while that side stays free,
/// slides when it must change sides, and fades in and out instead of popping.
struct Placer<'a> {
    painter: &'a egui::Painter,
    taken: Vec<egui::Rect>,
    last: ShownLabels,
    next: ShownLabels,
    /// Where each label placed this frame sits.
    anchors: HashMap<Named, egui::Pos2>,
    /// The labels placed this frame, in the order they were placed: the most
    /// important first.
    placed: Vec<Named>,
    /// The share of the way to its new side a label covers this frame.
    slide: f32,
    /// How much opacity a label gains or loses this frame.
    fade: f32,
}

impl<'a> Placer<'a> {
    fn new(painter: &'a egui::Painter, taken: Vec<egui::Rect>) -> Self {
        let dt = painter.ctx().input(|i| i.stable_dt).min(0.1);
        Self {
            painter,
            taken,
            last: painter
                .ctx()
                .data_mut(|d| d.get_temp::<ShownLabels>(Self::id()))
                .unwrap_or_default(),
            next: HashMap::new(),
            anchors: HashMap::new(),
            placed: Vec::new(),
            slide: 1.0 - (-SLIDE * dt).exp(),
            fade: dt / FADE,
        }
    }

    fn id() -> egui::Id {
        egui::Id::new("graph_labels")
    }

    /// Places `job` on a side of `node` (a disc of `edge` points) where it overlaps
    /// nothing already placed: the side it had last frame if that is still free,
    /// else the first free one; then, as far as `insist` asks, further out from the
    /// node, and else the side least covered anyway. Returns whether it was placed.
    fn place(
        &mut self,
        key: Named,
        node: egui::Pos2,
        edge: f32,
        job: egui::text::LayoutJob,
        stroke: Option<egui::Color32>,
        insist: Insist,
    ) -> bool {
        let galley = self.painter.layout_job(job);
        let extent = galley.size();
        // A side of the node, and how many steps out from it.
        let spot = |(side, step): (usize, usize)| {
            let gap = edge + 6.0 + step as f32 * FAN_STEP;
            // The label's corner nearest the node sits a gap off it, diagonally for a
            // corner.
            let align = SIDES[side];
            let away = align.to_sign();
            let corner = if away.x != 0.0 && away.y != 0.0 {
                std::f32::consts::FRAC_1_SQRT_2
            } else {
                1.0
            };
            align.anchor_size(node - away * gap * corner, extent)
        };
        // Inside the graph, so no label is cut off at its edge.
        let bounds = self.painter.clip_rect();
        let fits = |rect: egui::Rect, pad: egui::Vec2| {
            bounds.contains_rect(rect.expand2(pad))
                && !self.taken.iter().any(|r| r.intersects(rect.expand2(pad)))
        };
        let previous = self.last.get(&key);
        let around = |steps: std::ops::RangeInclusive<usize>| {
            steps.flat_map(|step| (0..SIDES.len()).map(move |side| (side, step)))
        };
        // A label holds its place until something actually covers it; a new place
        // must be clear with room to spare. That margin keeps a label from flipping
        // back and forth as the view moves. One that must show fans out further from
        // its node before it gives up and overlaps.
        let kept = previous
            .map(|p| (p.side, p.step))
            .filter(|&at| fits(spot(at), egui::Vec2::ZERO));
        let at = kept
            .or_else(|| around(0..=0).find(|&at| fits(spot(at), PILL_PAD)))
            .or_else(|| {
                (insist != Insist::No)
                    .then(|| around(1..=FAN_STEPS).find(|&at| fits(spot(at), PILL_PAD)))
                    .flatten()
            })
            .or_else(|| {
                (insist == Insist::Always).then(|| {
                    let side = least_covered(
                        |side| spot((side, 0)),
                        &self.taken,
                        previous.map(|p| p.side),
                    );
                    (side, 0)
                })
            });
        let Some((side, step)) = at else {
            return false;
        };
        let rect = spot((side, step));
        self.taken.push(rect.expand2(PILL_PAD));
        let target = rect.min - node;
        let (offset, opacity) = match previous {
            Some(p) => (
                p.offset + (target - p.offset) * self.slide,
                (p.opacity + self.fade).min(1.0),
            ),
            // A new label fades in where it belongs.
            None => (target, self.fade.min(1.0)),
        };
        self.anchors.insert(key, node);
        self.placed.push(key);
        self.next.insert(
            key,
            Shown {
                side,
                step,
                offset,
                opacity,
                galley,
                stroke,
            },
        );
        true
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
        // The most important last, on top of anything that still overlaps it.
        for key in self.placed.iter().rev() {
            if let (Some(&node), Some(shown)) = (self.anchors.get(key), self.next.get(key)) {
                self.paint(node, shown);
            }
        }
        self.next
            .extend(leaving.into_iter().map(|(key, _, shown)| (key, shown)));
        let next = self.next;
        self.painter
            .ctx()
            .data_mut(|d| d.insert_temp(Self::id(), next));
    }

    /// A label on its pill, at its eased place beside `node`.
    fn paint(&self, node: egui::Pos2, shown: &Shown) {
        let mut painter = self.painter.clone();
        painter.multiply_opacity(shown.opacity.clamp(0.0, 1.0));
        let min = egui::emath::GuiRounding::round_to_pixels(
            node + shown.offset,
            painter.pixels_per_point(),
        );
        let pill = egui::Rect::from_min_size(min, shown.galley.size()).expand2(PILL_PAD);
        if shown.step > 0 {
            // Fanned out from its node: a thin line back to it.
            let color = shown.stroke.unwrap_or(MUTED).gamma_multiply(0.7);
            painter.line_segment([node, pill.clamp(node)], egui::Stroke::new(1.0, color));
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
    }
}

/// The side whose spot, found by `spot`, overlaps the least of `taken`; on a tie, the
/// side the label had, then the earliest.
fn least_covered(
    spot: impl Fn(usize) -> egui::Rect,
    taken: &[egui::Rect],
    previous: Option<usize>,
) -> usize {
    let covered = |side: usize| -> f32 {
        let rect = spot(side);
        taken
            .iter()
            .filter_map(|r| {
                let overlap = r.intersect(rect);
                overlap.is_positive().then(|| overlap.area())
            })
            .sum()
    };
    previous
        .into_iter()
        .chain(0..SIDES.len())
        .min_by(|&a, &b| covered(a).total_cmp(&covered(b)))
        .unwrap_or_default()
}

/// How hard a label tries to show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Insist {
    /// Only where it has room beside its node.
    No,
    /// Further out from its node if need be, with a line back to it.
    FanOut,
    /// Over whatever is in the way, if it must.
    Always,
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
    struck: bool,
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
    let mut name_format = format(color, size);
    if struck {
        name_format.strikethrough = egui::Stroke::new(1.2, color);
    }
    job.append(name, 0.0, name_format);
    if let Some(note) = note {
        job.append(&note, 6.0, format(MUTED, size - 1.5));
    }
    job
}

/// What the lens shows: how far, around what, and measured how.
fn reveal_controls(ui: &mut egui::Ui, explorer: &mut Explorer, diffing: bool) {
    let mut lens = explorer.lens();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Show").small().color(MUTED));
        if lens.changes_only && diffing {
            ui.label("only changes")
                .on_hover_text("Everything else is hidden; C or the Changes card shows it");
            return;
        }
        if ui
            .selectable_label(lens.reach.is_none(), "All")
            .on_hover_text("Every function (=, until it shows all)")
            .clicked()
        {
            lens.reach = None;
        }
        let mut reach = lens.reach.unwrap_or(lens::MAX_REACH + 1);
        let unit = match (lens.measure, lens.reach) {
            (Measure::Calls, Some(1)) => "call",
            (Measure::Calls, _) => "calls",
            (_, Some(1)) => "step",
            _ => "steps",
        };
        if ui
            .add(
                egui::Slider::new(&mut reach, 0..=lens::MAX_REACH + 1)
                    .show_value(false)
                    .custom_formatter(|_, _| String::new()),
            )
            .on_hover_text("How far out from the start shows in full (- and =)")
            .changed()
        {
            lens.reach = (reach <= lens::MAX_REACH).then_some(reach);
        }
        if let Some(reach) = lens.reach {
            ui.label(egui::RichText::new(format!("{reach} {unit}")).small());
        }
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Around").small().color(MUTED));
        for source in [Source::Focus, Source::Changes, Source::Journey] {
            if source == Source::Changes && !diffing {
                continue;
            }
            ui.selectable_value(&mut lens.source, source, source.label())
                .on_hover_text("What the reveal starts from (L)");
        }
    });
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Near by").small().color(MUTED));
        ui.selectable_value(&mut lens.measure, Measure::Calls, "calls")
            .on_hover_text("Calls between, along call chains (G)");
        ui.selectable_value(&mut lens.measure, Measure::Space, "space")
            .on_hover_text("Distance in the graph, in steps of a typical call's length (G)");
        ui.selectable_value(&mut lens.measure, Measure::Blend, "blend")
            .on_hover_text(
                "Distance in the graph, with each call between counting against it; \
                 direct callers and callees always show (G)",
            );
    });
    if lens != explorer.lens() {
        explorer.set_lens(lens);
    }
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
    diff: Option<&DiffView>,
    up: Vec3,
) {
    let (graph, map) = (explorer.graph(), explorer.map());
    // Under a diff, what changed is named first and what it reaches next.
    let diff_tier = |tile: usize| {
        let diff = diff?;
        if diff.presence(tile) < 0.3 {
            return Some(u8::MAX);
        }
        match (diff.change(tile), diff.style, diff.impact(tile)) {
            (Some(_), _, _) => Some(1),
            (None, DiffStyle::Impact, Some(_)) if diff.affected.iter().any(|a| a.tile == tile) => {
                Some(1)
            }
            (None, DiffStyle::Impact, Some(_)) => Some(3),
            _ => None,
        }
    };
    let center = explorer.graph_focus();
    let hovered = explorer.hovered_tile;
    let reach = explorer.reach();
    let related = |tile: usize| reach.and_then(|r| relation(r, tile));
    let spotlit = explorer.spotlit();
    let crowded = spotlit.len() > SPOTLIGHT_NAMES;
    let named_first: HashSet<usize> = spotlit_in_order(explorer)
        .into_iter()
        .take(SPOTLIGHT_NAMES)
        .collect();
    let disclosed = explorer.disclosure();
    let journey = disclosed.tiles();
    let opened = disclosed.opened();
    let stack: Vec<usize> = explorer
        .selected()
        .map(|s| explorer.stack(s))
        .unwrap_or_default()
        .into_iter()
        .filter_map(|span| explorer.unit_id(span).and_then(|u| map.tile_of(u)))
        .collect();
    // Where each shown node sits on screen, and how far its edge is from its middle.
    let discs: Vec<Option<(egui::Pos2, f32)>> = (0..map.tiles.len())
        .map(|tile| {
            // Only what the lens shows at least half way gets a name.
            let world = explorer
                .tile_point(tile)
                .filter(|_| explorer.visibility(tile) >= 0.5)?;
            let node = to_screen(world)?;
            let radius = graph_view::node_size(graph, tile) / 2.0;
            let edge = to_screen(world + up * radius).map_or(8.0, |p| p.distance(node));
            Some((node, edge))
        })
        .collect();

    // With nothing in focus, only the busiest functions on screen are named, so the
    // whole graph does not drown in names; zooming in brings fewer onto the screen,
    // and so names more of them.
    let overview: HashSet<usize> = if center.is_none() {
        explorer
            .busiest()
            .iter()
            .copied()
            .filter(|&t| discs.get(t).is_some_and(Option::is_some))
            .take(OVERVIEW_NAMES)
            .collect()
    } else {
        HashSet::new()
    };
    let first_entries: HashSet<usize> = explorer
        .starts()
        .iter()
        .filter(|s| explorer.is_entry(s.tile))
        .take(ENTRY_NAMES)
        .map(|s| s.tile)
        .collect();
    // Lower ranks are placed first and win any overlap.
    let mut order: Vec<(LabelRank, Named)> = Vec::new();
    for (tile, disc) in discs.iter().enumerate() {
        if disc.is_none() {
            continue;
        }
        let tier = match related(tile) {
            _ if center == Some(tile) => 0,
            // What the pointer is over, and what the spotlight picks out, is named,
            // whatever else would be.
            _ if hovered == Some(tile) || spotlit.contains(&tile) => 1,
            // Where the codebase starts is named while nothing is in focus, the
            // entries that reach the most at least.
            _ if center.is_none() && first_entries.contains(&tile) => 1,
            _ if diff_tier(tile) == Some(u8::MAX) => continue,
            // What the open journey discloses is named first, with its numbers; with
            // a journey open, what it folds away and callers further up go unnamed.
            _ if journey.contains_key(&tile) => 1,
            // A change is named even where the journey folds it away, after what the
            // journey shows.
            _ if diff_tier(tile).is_some() => diff_tier(tile).map_or(2, |tier| tier.max(2)),
            Some(Relation::Downstream(_)) if opened => continue,
            Some(Relation::Upstream(hops)) if opened && hops > 1 => continue,
            Some(Relation::Upstream(1) | Relation::Downstream(1)) => 1,
            _ if stack.contains(&tile) => 2,
            Some(Relation::Upstream(2) | Relation::Downstream(2)) => 3,
            _ if explorer.journey_lane(tile).is_some() => 5,
            Some(_) => 6,
            None if overview.contains(&tile) => 3,
            None => continue,
        };
        order.push((
            (tier, std::cmp::Reverse(graph.degree(tile)), tile),
            Named::Node(tile),
        ));
    }
    // Modules are named at the top of their rings, ahead of most function names.
    let regions = graph_view::regions(explorer);
    let ring_top = |cluster: usize| {
        let &(_, middle, radius) = regions.iter().find(|r| r.0 == cluster)?;
        to_screen(middle + up * radius)
    };
    let districts = graph_view::districts(explorer);
    let district_top = |top: usize| {
        let &(_, middle, radius) = districts.iter().find(|d| d.0 == top)?;
        to_screen(middle + up * radius)
    };
    // How wide a ring of `radius` around `middle` shows on screen.
    let on_screen = |middle: Vec3, radius: f32| {
        let (Some(a), Some(b)) = (to_screen(middle), to_screen(middle + up * radius)) else {
            return 0.0;
        };
        a.distance(b)
    };
    if explorer.lens().reach.is_none() {
        // Crates are named ahead of their modules, which are named once their rings
        // are big enough on screen to tell apart.
        let mut crates: Vec<&(usize, Vec3, f32)> = districts
            .iter()
            .filter(|&&(_, middle, radius)| on_screen(middle, radius) >= CRATE_NAME_RING)
            .collect();
        crates.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
        for &&(top, _, radius) in crates.iter().take(MAX_CRATE_NAMES) {
            let tier = if center.is_some() { 5 } else { 1 };
            order.push((
                (tier, std::cmp::Reverse(rough(radius)), top),
                Named::Crate(top),
            ));
        }
        for &(cluster, middle, radius) in &regions {
            if !districts.is_empty() && on_screen(middle, radius) < MODULE_NAME_RING {
                continue;
            }
            let tier = if center.is_some() { 4 } else { 2 };
            order.push((
                (tier, std::cmp::Reverse(rough(radius)), cluster),
                Named::Group(cluster),
            ));
        }
    }
    order.sort_by_key(|&(key, _)| key);

    // Labels keep off the cards and the journey over the graph and the hint under
    // them, as last laid out.
    let cards: Vec<egui::Rect> = [
        "diff_card",
        "legend",
        "search",
        "camera_free",
        "journey_panel",
        "starts",
        "spotlight",
        "peek",
    ]
    .into_iter()
    .filter_map(|id| painter.ctx().memory(|m| m.area_rect(egui::Id::new(id))))
    .chain(painter.ctx().data(|d| d.get_temp::<egui::Rect>(hint_id())))
    .collect();
    // And off the nodes that matter, so they never hide a neighbor.
    let mut placer = Placer::new(
        painter,
        cards
            .into_iter()
            .chain(
                order
                    .iter()
                    .filter(|((tier, _, _), _)| *tier <= 3)
                    .filter_map(|&(_, named)| match named {
                        Named::Node(tile) => discs[tile],
                        Named::Group(_) | Named::Crate(_) => None,
                    })
                    .map(|(node, edge)| {
                        egui::Rect::from_center_size(node, egui::Vec2::splat(edge * 2.0))
                    }),
            )
            .collect(),
    );
    let mut names = 0;
    for ((tier, _, _), named) in order {
        match named {
            Named::Crate(top) => {
                let Some(at) = district_top(top) else {
                    continue;
                };
                let label = explorer.crate_name(top);
                if label.is_empty() {
                    continue;
                }
                // With something in focus, crates step back behind what it touches.
                let (strength, size) = if center.is_some() {
                    (0.45, 12.5)
                } else {
                    (1.0, 15.0)
                };
                let color = egui::Color32::from_rgb(214, 219, 230).gamma_multiply(strength);
                let job = label_job(None, &label, None, color, size, false);
                placer.place(named, at, 0.0, job, None, Insist::No);
            }
            Named::Group(group) => {
                let Some(at) = ring_top(group) else {
                    continue;
                };
                let color = egui_color(lane_color(group % PALETTE.len()))
                    .gamma_multiply(if center.is_some() { 0.6 } else { 0.9 });
                let label = explorer.cluster_name(group);
                if label.is_empty() {
                    continue;
                }
                let job = label_job(None, &label.to_uppercase(), None, color, 12.0, false);
                placer.place(named, at, 0.0, job, None, Insist::No);
            }
            Named::Node(tile) => {
                let Some((node, edge)) = discs[tile] else {
                    continue;
                };
                // The focus is named come what may, and what the spotlight picks out
                // too while it is few. Of a hub's many neighbors, the first fan out to
                // find room and the rest are named only where there is some; the
                // spotlight's card lists them all.
                // What the pointer is over is named come what may too.
                let insist =
                    if tier == 0 || hovered == Some(tile) || spotlit.contains(&tile) && !crowded {
                        Insist::Always
                    } else if named_first.contains(&tile) {
                        Insist::FanOut
                    } else {
                        Insist::No
                    };
                if names >= MAX_NAMES && insist == Insist::No {
                    continue;
                }
                let change = diff.and_then(|d| d.change(tile));
                let name = node_name(explorer, tile);
                let relation = related(tile);
                let color = match (tier, relation) {
                    (0, _) => egui::Color32::WHITE,
                    (_, Some(Relation::Upstream(1))) => CALLER_TEXT,
                    (_, Some(Relation::Downstream(1))) => CALLEE_TEXT,
                    // Only what touches the focus is colored, as its links are.
                    (_, Some(_)) => egui::Color32::from_rgb(200, 205, 216),
                    (2 | 5, None) => egui::Color32::from_rgb(225, 229, 238),
                    _ => egui::Color32::from_rgb(160, 167, 182),
                };
                let impacted =
                    diff.is_some_and(|d| d.style == DiffStyle::Impact && d.impact(tile).is_some());
                let color = match change {
                    Some(change) if tier != 0 => egui_color(diff_view::change_color(change)),
                    _ if impacted && tier != 0 => {
                        egui_color(diff_view::MODIFIED).gamma_multiply(0.8)
                    }
                    // Amber and blue are not drawn under a diff, and what did not change
                    // steps back.
                    _ if diff.is_some() && tier != 0 => egui::Color32::from_rgb(150, 156, 172),
                    _ => color,
                };
                let impact_note = diff
                    .filter(|d| d.style == DiffStyle::Impact)
                    .and_then(|d| d.affected.iter().find(|a| a.tile == tile))
                    .map(|a| match a.hops {
                        0 => "changed".to_owned(),
                        1 => "calls a change".to_owned(),
                        n => format!("{n} calls from a change"),
                    });
                let (lead, aside) = match (relation, center) {
                    _ if change.is_some() => (
                        change.map(|c| (diff_view::sign(c).to_owned(), color, None)),
                        diff.and_then(|d| d.moved_from(tile))
                            .map(|was| format!("was {}", tail(was))),
                    ),
                    _ if impact_note.is_some() => (None, impact_note),
                    (Some(Relation::Downstream(1)), Some(focus)) => {
                        let steps = call_steps(explorer, focus, tile);
                        (Some((numbered(&steps), color, None)), None)
                    }
                    // With the spotlight on, names alone, so more of them fit.
                    (Some(Relation::Upstream(1)), Some(_)) if !spotlit.is_empty() => (None, None),
                    (Some(Relation::Upstream(1)), Some(focus)) => {
                        let steps = call_steps(explorer, tile, focus);
                        let total = graph.sequence(tile).len();
                        (
                            None,
                            Some(format!("calls it {} of {total}", numbered(&steps))),
                        )
                    }
                    _ => (None, None),
                };
                // How the journey runs it, after anything else said of it.
                let marks: Vec<&str> = [
                    (disclosed.tile_sometimes(tile), "sometimes"),
                    (disclosed.tile_fails(tile), "can fail"),
                ]
                .into_iter()
                .filter_map(|(on, mark)| on.then_some(mark))
                .collect();
                // An entry point says how it is reached, after its name, unless that
                // only says its name again.
                let route = entry_label(explorer, tile)
                    .filter(|route| !route.to_lowercase().contains(&name.to_lowercase()))
                    .map(str::to_owned);
                let aside = [route, aside, (!marks.is_empty()).then(|| marks.join(" · "))]
                    .into_iter()
                    .flatten()
                    .reduce(|a, b| format!("{a} · {b}"));
                // A node the journey runs leads with its place in it, as its bar does.
                let order = journey
                    .get(&tile)
                    .filter(|numbers| !numbers.is_empty())
                    .map(|numbers| match numbers.as_slice() {
                        [one] => one.clone(),
                        [first, second] => format!("{first}, {second}"),
                        [first, .., last] => format!("{first} … {last}"),
                        [] => String::new(),
                    });
                // The number on a badge filled like the node and its bar, so the two
                // read as one; what changed after it.
                let fill = egui_color(graph_view::node_color(explorer, diff, tile));
                let lead = match (order, change) {
                    (Some(order), Some(change)) => Some((
                        format!(" {order} {} ", diff_view::sign(change)),
                        ON_FILL,
                        Some(fill),
                    )),
                    (Some(order), None) => Some((format!(" {order} "), ON_FILL, Some(fill))),
                    // An entry point leads with a flag, as its node flies one.
                    (None, None) if lead.is_none() && explorer.is_entry(tile) => {
                        Some(("▶".to_owned(), egui::Color32::WHITE, None))
                    }
                    (None, _) => lead,
                };
                let size = match tier {
                    0 => 14.0,
                    1 => 12.0,
                    _ => 11.0,
                };
                // The pill's border says which way it relates to the focus, as its link
                // does.
                let stroke = match relation {
                    _ if tier == 0 => Some(egui::Color32::from_white_alpha(140)),
                    Some(Relation::Upstream(1)) => Some(egui_color(CALLER)),
                    Some(Relation::Downstream(1)) => Some(egui_color(CALLEE)),
                    _ if spotlit.contains(&tile) => Some(egui_color(match explorer.spotlight() {
                        Some((Side::Callers, _)) => CALLER,
                        _ => CALLEE,
                    })),
                    _ => None,
                };
                // What is gone is struck through, which reads at any size.
                let struck = change == Some(Change::Removed);
                let job = label_job(lead, &name, aside, color, size, struck);
                if placer.place(named, node, edge, job, stroke, insist) {
                    names += 1;
                }
            }
        }
    }
    placer.finish(|named| match named {
        Named::Node(tile) => discs.get(tile).copied().flatten().map(|(node, _)| node),
        Named::Group(group) => ring_top(group),
        Named::Crate(top) => district_top(top),
    });
}

/// A rough size, for ordering.
fn rough(radius: f32) -> usize {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a rough size for ordering"
    )]
    let size = radius.max(0.0) as usize;
    size
}

/// What the spotlight picks out, most telling first: the focus's calls in the order
/// it makes them, or its callers with the most calls of their own first.
fn spotlit_in_order(explorer: &Explorer) -> Vec<usize> {
    let mut spotlit = explorer.spotlit();
    let (Some((side, _)), Some(focus)) = (explorer.spotlight(), explorer.graph_focus()) else {
        return spotlit;
    };
    let graph = explorer.graph();
    match side {
        Side::Callees => {
            let first = |tile: usize| graph.sequence(focus).iter().position(|&t| t == tile);
            spotlit.sort_by_key(|&t| (first(t).unwrap_or(usize::MAX), t));
        }
        Side::Callers => spotlit.sort_by_key(|&t| (std::cmp::Reverse(graph.degree(t)), t)),
    }
    spotlit
}

/// When the spotlight picks out more than the graph can name, every one of them on a
/// card at the top left, grouped by module in its color, each opening on a click.
fn spotlight_card(
    ctx: &egui::Context,
    area: egui::Rect,
    explorer: &mut Explorer,
) -> Option<egui::Rect> {
    let (side, _) = explorer.spotlight()?;
    let spotlit = spotlit_in_order(explorer);
    if spotlit.len() <= SPOTLIGHT_NAMES {
        return None;
    }
    let graph = explorer.graph();
    let mut groups: Vec<(usize, Vec<usize>)> = Vec::new();
    for &tile in &spotlit {
        let cluster = graph.cluster(tile).unwrap_or_default();
        match groups.iter_mut().find(|(c, _)| *c == cluster) {
            Some((_, tiles)) => tiles.push(tile),
            None => groups.push((cluster, vec![tile])),
        }
    }
    groups.sort_by_key(|(cluster, tiles)| (std::cmp::Reverse(tiles.len()), *cluster));
    let focus = explorer.graph_focus();
    let (title, color) = match side {
        Side::Callers => (format!("{} callers", spotlit.len()), CALLER_TEXT),
        Side::Callees => (format!("{} calls", spotlit.len()), CALLEE_TEXT),
    };
    let mut chosen = None;
    let shown = egui::Area::new("spotlight".into())
        .pivot(egui::Align2::LEFT_TOP)
        .fixed_pos(egui::pos2(area.min.x + 12.0, area.min.y + 10.0))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_premultiplied(11, 13, 19, 225))
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(50)))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_max_width(300.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(title).strong().color(color));
                        let note = "the graph names what fits";
                        ui.label(egui::RichText::new(note).small().color(MUTED));
                    });
                    egui::ScrollArea::vertical()
                        .id_salt("spotlit")
                        .max_height((area.height() * 0.6).max(160.0))
                        .show(ui, |ui| {
                            for (cluster, tiles) in &groups {
                                let module = egui_color(lane_color(cluster % PALETTE.len()));
                                ui.label(
                                    egui::RichText::new(
                                        explorer.cluster_name(*cluster).to_uppercase(),
                                    )
                                    .small()
                                    .color(module),
                                );
                                for &tile in tiles {
                                    let steps = match (side, focus) {
                                        (Side::Callees, Some(focus)) => {
                                            numbered(&call_steps(explorer, focus, tile))
                                        }
                                        _ => String::new(),
                                    };
                                    let text = format!("{steps} {}", node_name(explorer, tile));
                                    if ui
                                        .selectable_label(false, text.trim())
                                        .on_hover_text("Open it")
                                        .clicked()
                                    {
                                        chosen = Some(tile);
                                    }
                                }
                            }
                        });
                });
        });
    if let Some(tile) = chosen {
        explorer.open_at(tile);
    }
    Some(shown.response.rect)
}

/// The focus by name, then a button for each side of it: what calls it (I) and what it
/// calls (O), each in the color its links are drawn in. A button puts its side in the
/// spotlight, which frames everything it picks out. The name says what those keys, and
/// Enter, act on: the focus moves with the journey's cursor.
fn focus_sides(ui: &mut egui::Ui, explorer: &mut Explorer, now: f32) {
    let Some(focus) = explorer.graph_focus() else {
        return;
    };
    let calling = explorer.callers_of(focus).len();
    let mut called: Vec<usize> = explorer
        .graph()
        .callees(focus)
        .filter(|&t| t != focus)
        .collect();
    called.sort_unstable();
    called.dedup();
    let lit = explorer.spotlight().map(|(side, _)| side);
    let mut pressed = None;
    ui.horizontal(|ui| {
        for (side, count, noun, key, color) in [
            (Side::Callers, calling, "caller", "I", CALLER),
            (Side::Callees, called.len(), "call", "O", CALLEE),
        ] {
            let plural = if count == 1 { "" } else { "s" };
            let text = egui::RichText::new(format!("● {count} {noun}{plural}  {key}"))
                .color(egui_color(color));
            let mut hint = match side {
                Side::Callers => "Pick out what calls it, and bring it all into view (I)",
                Side::Callees => "Pick out what it calls, and bring it all into view (O)",
            }
            .to_owned();
            let guessed = explorer.guessed_neighbors(focus, side);
            if guessed > 0 {
                let _ = write!(
                    hint,
                    "\n{guessed} more only guessed by name, not counted (U counts them)"
                );
            }
            if ui
                .add_enabled(count > 0, egui::Button::selectable(lit == Some(side), text))
                .on_hover_text(hint)
                .clicked()
            {
                pressed = Some(side);
            }
        }
        ui.label(
            egui::RichText::new(node_name(explorer, focus))
                .strong()
                .color(egui::Color32::WHITE),
        )
        .on_hover_text("The focus: what I, O and Enter act on. It moves with the journey's cursor");
        ui.label(egui::RichText::new("focus").small().color(MUTED));
    });
    if let Some(side) = pressed {
        explorer.toggle_spotlight(side, now);
    }
}

/// The cards down the graph's top left, each under the one before: where to start or
/// the spotlight's list, then the focus's source. Returns where the last went, for the
/// camera chip to go under.
fn corner_cards(
    ctx: &egui::Context,
    area: egui::Rect,
    explorer: &mut Explorer,
    start: &mut bool,
    panel: &mut JourneyPanel,
    code: &mut Code<'_>,
    (peek, diff): (&mut bool, Option<&DiffView>),
) -> Option<egui::Rect> {
    let card = if *start {
        start_card(ctx, area, explorer, start, panel)
    } else {
        spotlight_card(ctx, area, explorer)
    };
    if !*peek {
        return card;
    }
    let below = card.map_or(area, |card| {
        egui::Rect::from_min_max(egui::pos2(area.min.x, card.max.y), area.max)
    });
    peek::draw(ctx, below, explorer, diff, code, peek).or(card)
}

/// Says the camera has been panned or zoomed away and no longer follows the journey,
/// top left of the graph, under any card there; a click, like F, brings it back.
fn camera_free(
    ctx: &egui::Context,
    area: egui::Rect,
    under: Option<egui::Rect>,
    explorer: &Explorer,
    camera: &mut OrbitCamera,
) {
    let at = under.map_or(area.min.y + 10.0, |card| card.max.y + 8.0);
    egui::Area::new("camera_free".into())
        .fixed_pos(egui::pos2(area.min.x + 12.0, at))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            let chip = egui::Button::new(
                egui::RichText::new("camera free · F to follow")
                    .size(12.5)
                    .color(MUTED),
            )
            .fill(egui::Color32::from_rgba_premultiplied(11, 13, 19, 215))
            .corner_radius(8.0);
            if ui
                .add(chip)
                .on_hover_text(
                    "The camera stays where you put it. F, or a click here, \
                     brings it back round the journey's start",
                )
                .clicked()
            {
                graph_view::orbit_anchor(explorer, camera);
            }
        });
}

/// What the colors mean, in two lines: what a node's fill says, then which way a
/// link runs, and that a dotted one is a guess when any shows (`guesses` says whether
/// they count). A hint at the keys below.
fn legend(ctx: &egui::Context, area: egui::Rect, diffing: bool, guesses: Option<bool>) {
    egui::Area::new("legend".into())
        .pivot(egui::Align2::LEFT_BOTTOM)
        .fixed_pos(egui::pos2(area.min.x + 12.0, area.max.y - 30.0))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_premultiplied(11, 13, 19, 215))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(10, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let swatch = |ui: &mut egui::Ui, mark: &str, color: Color, text: &str| {
                            ui.label(egui::RichText::new(mark).color(egui_color(color)));
                            ui.label(egui::RichText::new(text).size(12.5));
                            ui.add_space(6.0);
                        };
                        if diffing {
                            swatch(ui, "●", diff_view::ADDED, "added");
                            swatch(ui, "●", diff_view::MODIFIED, "rewritten");
                            swatch(ui, "●", diff_view::REMOVED, "removed");
                            swatch(ui, "●", diff_view::MOVED, "moved");
                            swatch(ui, "●", graph_view::UNCHANGED, "unchanged");
                        } else {
                            ui.label(
                                egui::RichText::new("node color = its module, as its ring")
                                    .size(12.5),
                            );
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for (color, text) in [
                            (CALLER, "calls into the focus"),
                            (CALLEE, "calls out of it"),
                        ] {
                            ui.label(egui::RichText::new("→").color(egui_color(color)));
                            ui.label(egui::RichText::new(text).size(12.5));
                            ui.add_space(6.0);
                        }
                        ui.label(egui::RichText::new("→").color(egui_color(graph_view::FURTHER)));
                        ui.label(egui::RichText::new("further along").size(12.5));
                        ui.add_space(6.0);
                        if let Some(counted) = guesses {
                            let text = if counted {
                                "guessed by name, counted (U)"
                            } else {
                                "guessed by name, not counted (U)"
                            };
                            ui.label(
                                egui::RichText::new("┈┈").color(egui::Color32::from_gray(200)),
                            );
                            ui.label(egui::RichText::new(text).size(12.5));
                            ui.add_space(6.0);
                        }
                        ui.label(
                            egui::RichText::new("journey bars match their nodes")
                                .size(12.5)
                                .color(MUTED),
                        );
                    });
                });
        });
    let hint = ctx
        .layer_painter(egui::LayerId::background())
        .with_clip_rect(area)
        .text(
            egui::pos2(area.min.x + 12.0, area.max.y - 10.0),
            egui::Align2::LEFT_BOTTOM,
            "click a node to open it · I callers · O calls · , . less or more of the journey \
             · Backspace back · B source · E editor · / search · W where to start · S panel · H keys",
            egui::FontId::proportional(12.0),
            MUTED,
        );
    // Remembered so graph labels keep off it, as they do the cards.
    ctx.data_mut(|d| d.insert_temp(hint_id(), hint));
}

fn hint_id() -> egui::Id {
    egui::Id::new("legend_hint")
}

/// A heading and its keys.
type Group = (&'static str, &'static [(&'static str, &'static str)]);

/// Every key, grouped by what it moves, in columns.
const KEYS: [&[Group]; 4] = [
    &[(
        "Journey",
        &[
            ("← →", "previous / next call by the same caller"),
            ("↓", "into the first call this one makes"),
            ("↑", "out to its caller"),
            ("[  ]", "back / forward one call in time"),
            ("Shift ]", "step over this call's calls"),
            ("Shift ↑", "step out to what runs after its caller"),
            ("Home  End", "first / last call"),
            (",  .", "show a level less / more of it"),
            ("V", "show calls by level / as the cursor reaches them"),
            ("Space", "play"),
            ("Tab", "next open journey"),
            (
                "Enter",
                "start the journey at the focus; with none, open where to start",
            ),
            (
                "Shift Enter",
                "pull the cursor's call out as its own journey",
            ),
            ("X", "close the journey"),
        ],
    )],
    &[(
        "Graph",
        &[
            ("I", "pick out what calls the focus, and show it all"),
            ("O", "pick out what the focus calls, and show it all"),
            ("U", "count calls guessed by name, or leave them out"),
            ("1 – 9", "open the call with that number"),
            ("Backspace  Alt ←", "back to the view before"),
            ("Alt →", "forward again"),
            (
                "Esc",
                "close search, this list, where to start, the spotlight, the panel",
            ),
            ("- / =", "show less / more around the start"),
            ("Q", "show all → 2 steps → 1 step"),
            ("L", "reveal around: focus → changes → journey"),
            ("G", "nearness by: calls → space → blend"),
            ("C", "diff: only what changed / everything"),
            ("K", "cluster by modules / calls"),
            ("F", "follow the journey's start again, head-on"),
            ("R", "frame the whole graph"),
            ("D", "diff: status → impact → morph"),
            ("T", "play or pause the morph"),
        ],
    )],
    &[(
        "Camera",
        &[
            ("two fingers", "orbit"),
            ("Shift two fingers", "pan"),
            ("pinch, Ctrl two fingers", "zoom towards the pointer"),
            ("wheel", "zoom towards the pointer"),
            ("left / middle drag", "orbit"),
            ("right / Shift drag", "pan"),
        ],
    )],
    &[
        (
            "Layout",
            &[
                ("J", "show or hide the journey"),
                ("M", "swap graph and journey"),
                ("S", "show or hide the side panel"),
                ("B", "show or hide the focus's source"),
                ("/", "search for a function; ↑ ↓ pick, Enter opens"),
                ("W", "show or hide where to start"),
                ("H", "this list"),
            ],
        ),
        (
            "Elsewhere",
            &[
                ("E", "open the focus in your editor"),
                ("Y", "copy the view as a command line"),
                ("P", "save a picture of the view"),
            ],
        ),
    ],
];

fn key_group(ui: &mut egui::Ui, title: &str, keys: &[(&str, &str)]) {
    ui.label(egui::RichText::new(title).strong().size(14.0));
    ui.add_space(4.0);
    egui::Grid::new(title)
        .spacing(egui::vec2(12.0, 4.0))
        .show(ui, |ui| {
            for (key, what) in keys {
                ui.label(
                    egui::RichText::new(*key)
                        .monospace()
                        .color(egui::Color32::WHITE),
                );
                ui.label(egui::RichText::new(*what).color(MUTED));
                ui.end_row();
            }
        });
}

fn key_help(ctx: &egui::Context, area: egui::Rect, open: &mut bool) {
    egui::Area::new("key_help".into())
        .pivot(egui::Align2::CENTER_CENTER)
        .fixed_pos(area.center())
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_premultiplied(11, 13, 19, 240))
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(60)))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::symmetric(18, 14))
                .show(ui, |ui| {
                    ui.horizontal_top(|ui| {
                        for column in KEYS {
                            ui.vertical(|ui| {
                                for &(title, keys) in column {
                                    key_group(ui, title, keys);
                                    ui.add_space(10.0);
                                }
                            });
                            ui.add_space(16.0);
                        }
                    });
                    if ui.small_button("close").clicked() {
                        *open = false;
                    }
                });
        });
}

/// The last two segments of a path: `orders::Order::total` is `Order.total`.
fn tail(path: &str) -> String {
    let segments: Vec<&str> = path.split("::").collect();
    segments[segments.len().saturating_sub(2)..].join(".")
}

/// `text` cut to `max` characters, with an ellipsis when it was cut.
fn short(text: &str, max: usize) -> String {
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
            let mut placer = Placer::new(&painter, blocked.to_vec());
            let job = label_job(None, "draw_graph", None, egui::Color32::WHITE, 12.5, false);
            assert!(placer.place(Named::Node(0), node, 5.0, job, None, Insist::No));
            placer.finish(|_| Some(node));
        });
        output.textures_delta.clear();
        let shown = ctx
            .data_mut(|d| d.get_temp::<ShownLabels>(Placer::id()))
            .and_then(|labels| labels.get(&Named::Node(0)).cloned())
            .unwrap_or_else(|| panic!("the label was not remembered"));
        (shown.side, shown.offset)
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

    fn candidate(id: &str, own: &str, nested: bool, distance: u32, callers: usize) -> Candidate {
        let (place, _) = id.rsplit_once("::").unwrap_or_default();
        let name = if nested {
            format!("{} › {own}", place.rsplit("::").next().unwrap_or_default())
        } else {
            own.to_owned()
        };
        Candidate {
            id: UnitId::new(format!("rust:{id}")),
            name,
            own: own.to_owned(),
            place: place.replace("::", "."),
            route: None,
            kind: "fn".to_owned(),
            nested,
            distance,
            callers,
        }
    }

    fn found(query: &str, candidates: &[Candidate]) -> Vec<String> {
        search_order(query, candidates)
            .into_iter()
            .map(|index| {
                candidates[index]
                    .id
                    .as_str()
                    .trim_start_matches("rust:")
                    .to_owned()
            })
            .collect()
    }

    #[test]
    fn search_leaves_closures_out_unless_asked_for() {
        let candidates = [
            candidate("ui::draw_graph_labels", "draw_graph_labels", false, 9, 1),
            candidate(
                "ui::draw_graph_labels::{closure#3}",
                "closure 3",
                true,
                9,
                0,
            ),
            candidate("ui::draw_ui", "draw_ui", false, 9, 1),
        ];
        assert_eq!(
            found("draw", &candidates),
            ["ui::draw_ui", "ui::draw_graph_labels"],
            "a closure lives on its function's node"
        );
        assert_eq!(
            found("draw closure", &candidates),
            ["ui::draw_graph_labels::{closure#3}"]
        );
        assert_eq!(
            found("draw_graph_labels › closure 3", &candidates),
            ["ui::draw_graph_labels::{closure#3}"],
            "as the screen names it"
        );
    }

    #[test]
    fn search_puts_what_is_near_the_focus_first_then_what_is_most_called() {
        let candidates = [
            candidate("core::system::System::new", "new", false, 51, 30),
            candidate("viewer::Viewer::new", "new", false, 51, 2),
            candidate("viewer::explorer::Explorer::new", "new", false, 1, 0),
            candidate("viewer::names::Names::newest", "newest", false, 0, 0),
        ];
        assert_eq!(
            found("new", &candidates),
            [
                "viewer::explorer::Explorer::new",
                "core::system::System::new",
                "viewer::Viewer::new",
                "viewer::names::Names::newest",
            ]
        );
    }

    #[test]
    fn a_label_that_must_show_takes_the_least_covered_side() {
        // Each side a unit square along x.
        let spot = |side: usize| {
            let x = side as f32;
            egui::Rect::from_min_size(egui::pos2(x, 0.0), egui::vec2(1.0, 1.0))
        };
        let half = |x: f32| egui::Rect::from_min_size(egui::pos2(x, 0.0), egui::vec2(0.5, 1.0));
        let everywhere = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(8.0, 1.0));
        assert_eq!(least_covered(spot, &[everywhere, half(3.0)], None), 0);
        assert_eq!(
            least_covered(spot, &[everywhere, half(3.0)], Some(5)),
            5,
            "covered alike, it keeps its side"
        );
        assert_eq!(least_covered(spot, &[half(0.0), half(1.0)], Some(0)), 2);
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
}
