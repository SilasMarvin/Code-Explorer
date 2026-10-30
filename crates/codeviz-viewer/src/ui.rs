//! egui around the graph: call stack and the journey on top, and a side panel, hidden at
//! first, with the view's settings, outcomes, source and details; on the canvas, the
//! focus with what calls it and what it calls, names beside nodes, a key to the
//! colors, and a search box on `/`.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use bevy::{camera::Viewport, prelude::*, window::PrimaryWindow};
use bevy_egui::{
    EguiContext, EguiContexts, EguiInput, EguiPreUpdateSet, EguiPrimaryContextPass,
    PrimaryEguiContext, egui,
};
use codeviz_core::{Change, Level, SpanId, SpanTarget};
use codeviz_ir::{ExitKind, Role, UnitId};

use crate::{
    camera::OrbitCamera,
    depth_meter,
    diff_view::{self, DiffStyle, DiffView},
    explorer::{Clustering, Confidence, Explorer, Hidden, Side},
    graph_view::{self, CALLEE, CALLER, CURSOR, DEEPER, Relation},
    journey_panel::{self, JourneyPanel, PanelMode},
    lens::{Measure, Source},
    nucleus::{self, Body, End, Nucleus},
    scene::{PALETTE, lane_color},
    source_diff,
};

pub(crate) struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Search>()
            .init_resource::<KeyHelp>()
            .init_resource::<Chrome>()
            .init_resource::<Sources>()
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

/// Source files read so far; `None` when a file could not be read.
#[derive(Resource, Debug, Default)]
struct Sources(HashMap<PathBuf, Option<Vec<String>>>);

impl Sources {
    fn lines(&mut self, path: &Path) -> Option<&[String]> {
        self.0
            .entry(path.to_path_buf())
            .or_insert_with(|| {
                std::fs::read_to_string(path)
                    .ok()
                    .map(|text| text.lines().map(str::to_owned).collect())
            })
            .as_deref()
    }
}

const ERROR: egui::Color32 = egui::Color32::from_rgb(240, 110, 100);
const OK: egui::Color32 = egui::Color32::from_rgb(120, 210, 140);
pub(crate) const MUTED: egui::Color32 = egui::Color32::from_rgb(140, 148, 165);

/// What draws around the graph: everything, or bare, with only the graph and the
/// journey; whether the side panel and the search box are open; and when a key last
/// turned the reveal's dial, for the depth meter.
#[derive(Resource, Default)]
pub(crate) struct Chrome {
    pub(crate) bare: bool,
    pub(crate) side: bool,
    pub(crate) search: bool,
    pub(crate) dial_turned: Option<f32>,
}

#[expect(clippy::too_many_arguments, reason = "a Bevy system")]
fn draw_ui(
    mut contexts: EguiContexts,
    mut explorer: ResMut<Explorer>,
    mut search: ResMut<Search>,
    mut sources: ResMut<Sources>,
    mut panel: ResMut<JourneyPanel>,
    mut help: ResMut<KeyHelp>,
    mut chrome: ResMut<Chrome>,
    time: Res<Time>,
    mut diff: Option<ResMut<DiffView>>,
    mut cameras: Query<(&mut Camera, &GlobalTransform, &mut OrbitCamera)>,
    window: Single<&Window, With<PrimaryWindow>>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let mut root_ui = egui::Ui::new(
        ctx.clone(),
        "codeviz".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.viewport_rect()),
    );

    let [top, bottom, left, right] = if chrome.bare {
        [0.0; 4]
    } else {
        panels(
            &mut root_ui,
            &mut explorer,
            &mut panel,
            &mut sources,
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
    // Browsing, no journey is open, so its panel is not drawn.
    let mode = if explorer.browsing() {
        PanelMode::Hidden
    } else {
        panel.mode
    };
    let (graph_area, panel_area) = journey_panel::split(mode, central);
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
            (transform.up().into(), transform.right().into()),
        );
        // The graph's right edge clear of the diff card above and a docked journey
        // below, for the depth meter.
        let mut clear = above(graph_area, panel_area);
        if let Some(diff) = diff.as_deref_mut().filter(|_| !chrome.bare) {
            let card = diff_view::diff_card(ctx, graph_area, &mut explorer, diff);
            clear.min.y = clear.min.y.max(card.max.y);
        }
        let only_changes = explorer.lens().changes_only && diff.is_some();
        if !chrome.bare && !help.0 && !only_changes {
            depth_meter::draw(
                ctx,
                clear,
                &explorer,
                chrome.dial_turned,
                time.elapsed_secs(),
            );
        }
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
            let showing = Showing::of(&explorer, diff.is_some());
            legend(ctx, clear, graph_area, showing);
        }
        if orbit.free && !chrome.bare {
            camera_free(ctx, graph_area, &explorer, &mut orbit);
        }
        if help.0 {
            key_help(
                ctx,
                graph_area,
                &mut help.0,
                Showing::of(&explorer, diff.is_some()),
            );
        }
        if let Some(area) = panel_area {
            let tethers =
                journey_panel::draw(ctx, area, &mut explorer, &mut panel, diff.as_deref());
            draw_tethers(ctx, &tethers, &explorer, &to_screen);
        }
    }
    Ok(())
}

/// The panels around the graph, and how much of each edge they take: top, bottom, left
/// and right. The side panel shows only when opened (S).
fn panels(
    root_ui: &mut egui::Ui,
    explorer: &mut Explorer,
    panel: &mut JourneyPanel,
    sources: &mut Sources,
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
                        mode_label(ui, explorer);
                        if !explorer.browsing() {
                            breadcrumb(ui, explorer, panel);
                        }
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
                        Some(view) => changed_source(ui, explorer, view, sources),
                        None => source(ui, explorer, sources),
                    }
                    ui.separator();
                    details(ui, explorer);
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

/// The cursor's bar tied to its node, in the cursor's color where its node is drawn in
/// it, and the bar under the pointer to the node it stands for.
fn draw_tethers(
    ctx: &egui::Context,
    tethers: &journey_panel::Tethers,
    explorer: &Explorer,
    to_screen: &impl Fn(Vec3) -> Option<egui::Pos2>,
) {
    if let (Some(bar), Some(node)) = (tethers.cursor, cursor_node(explorer, to_screen)) {
        let color = if explorer.cursor_mark().is_some() {
            egui_color(CURSOR)
        } else {
            egui::Color32::WHITE
        };
        tether(ctx, bar, node, color, false);
    }
    let hovered = explorer
        .hovered_tile
        .and_then(|tile| explorer.tile_point(tile))
        .and_then(to_screen);
    if let (Some(bar), Some(node)) = (tethers.hovered, hovered) {
        tether(ctx, bar, node, egui::Color32::WHITE, true);
    }
}

/// A line from a bar in the journey up to its node in the graph, in `color`: faint for
/// the cursor's, bright for the one under the pointer.
fn tether(
    ctx: &egui::Context,
    bar: egui::Rect,
    node: egui::Pos2,
    color: egui::Color32,
    bright: bool,
) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("tether"),
    ));
    let from = if bright {
        egui::pos2(bar.center().x, bar.min.y)
    } else {
        egui::pos2(bar.min.x + 2.0, bar.min.y)
    };
    let alpha = if bright { 0.86 } else { 0.6 };
    painter.line_segment(
        [from, node],
        egui::Stroke::new(if bright { 1.5 } else { 1.25 }, color.gamma_multiply(alpha)),
    );
    painter.circle_filled(from, 2.5, color);
    if bright {
        painter.circle_stroke(node, 9.0, egui::Stroke::new(1.5, color));
    }
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
        let Some(selected) = explorer.selected() else {
            ui.label("Pick a unit on the left.");
            return;
        };
        // The calls down to the cursor, closures left out as the panel leaves them
        // out; where they do not all fit, those nearest the start give way first, so
        // the line never runs under what the top bar shows on its right.
        let mut frames: Vec<SpanId> = explorer
            .stack(selected)
            .into_iter()
            .filter(|&frame| {
                frame == selected
                    || !explorer
                        .unit_id(frame)
                        .is_some_and(|unit| unit.as_str().contains("{closure"))
            })
            .collect();
        let font = egui::TextStyle::Body.resolve(ui.style());
        let gap = ui.spacing().item_spacing.x;
        let width_of = |text: &str| {
            ui.painter()
                .layout_no_wrap(text.to_owned(), font.clone(), egui::Color32::WHITE)
                .size()
                .x
        };
        let crumb = |frame: SpanId| width_of(&explorer.label(frame)) + width_of("›") + gap * 2.0;
        let room = ui.available_width() - 8.0;
        let mut elided = false;
        while frames.len() > 2
            && frames.iter().map(|&f| crumb(f)).sum::<f32>()
                + if elided {
                    width_of("…") + gap * 2.0
                } else {
                    0.0
                }
                > room
        {
            frames.remove(1);
            elided = true;
        }
        let mut chosen = None;
        for (index, frame) in frames.into_iter().enumerate() {
            if index > 0 {
                ui.label(egui::RichText::new("›").color(MUTED));
            }
            if index == 1 && elided {
                ui.label(egui::RichText::new("…").color(MUTED))
                    .on_hover_text("Calls between, left out for room");
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
            explorer.go_to(frame);
        }
    });
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

/// The caller's source around the line that made the selected call; for the root,
/// its own definition.
fn source(ui: &mut egui::Ui, explorer: &Explorer, sources: &mut Sources) {
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
    ui.label(
        egui::RichText::new(location.to_string())
            .small()
            .color(MUTED),
    );
    let Some(path) = explorer.system().local_path(&owner, &location.uri) else {
        return;
    };
    let Some(lines) = sources.lines(&path) else {
        ui.label(egui::RichText::new(format!("Cannot read {}", path.display())).color(MUTED));
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
fn changed_source(ui: &mut egui::Ui, explorer: &Explorer, view: &DiffView, sources: &mut Sources) {
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
        let lines = sources.lines(&path)?;
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
    ui.label(egui::RichText::new(place).small().color(MUTED));
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

fn details(ui: &mut egui::Ui, explorer: &mut Explorer) {
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
            && let Some(unit) = explorer.unit_id(selected).cloned()
        {
            explorer.set_root(unit);
        }
        if ui
            .add_enabled(explorer.can_go_back(), egui::Button::new("Back"))
            .on_hover_text("The view before (Backspace)")
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
                    .any_unit(unit)
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
    /// Where a call leaves the expanded nucleus towards a module, by cluster.
    Wall(usize),
}

/// Tier, then bigger first, then a stable tiebreak.
type LabelRank = (u8, std::cmp::Reverse<usize>, usize);

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

/// Places labels so none overlaps another or a node that matters, and draws them
/// eased from where they were: a label keeps its side while that side stays free,
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
        }
    }

    fn id() -> egui::Id {
        egui::Id::new("graph_labels")
    }

    /// Places `job` on a side of `node` (a disc of `edge` points) where it overlaps
    /// nothing already placed: the side it had last frame if that is still free,
    /// else the first free one. When `force` asks and none is, it is nudged along a
    /// side to the nearest free place, else put where it misleads least, which only
    /// [`Force::Always`] lets lie over another name. A function's label goes nowhere
    /// it would lie nearer another node than its own, so it never reads as the
    /// other's. Returns whether it was placed.
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
        // Inside the graph, so no label is cut off at its edge.
        let bounds = self.painter.clip_rect();
        let against = self.against(key);
        let fits = |rect: egui::Rect, pad: egui::Vec2| {
            bounds.contains_rect(rect.expand2(pad))
                && !against.iter().any(|r| r.intersects(rect.expand2(pad)))
                && self.misread(key, rect, node, edge) == 0
        };
        let previous = self.last.get(&key);
        // A label holds its side until something actually covers it; a new side must
        // be clear with room to spare. That margin keeps a label from flipping back
        // and forth as the view moves.
        let kept = previous
            .map(|p| p.at)
            .filter(|&at| !at.over && fits(spot(at), egui::Vec2::ZERO));
        let free = |shifts: &[i8]| {
            shifts
                .iter()
                .flat_map(|&shift| (0..SIDES.len()).map(move |side| Spot::beside(side, shift)))
                .find(|&at| fits(spot(at), PILL_PAD))
        };
        // A label nudged off its node goes back beside it as soon as there is room.
        let at = kept
            .filter(|at| at.shift == 0)
            .or_else(|| free(&[0]))
            .or(kept)
            .or_else(|| (force != Force::Never).then(|| free(&NUDGES)).flatten())
            .or_else(|| {
                let (at, over_names) = self.least_misleading(key, node, edge, extent);
                match force {
                    Force::Always => Some(at),
                    Force::Free if !over_names => Some(at),
                    Force::Free | Force::Never => None,
                }
            });
        let Some(at) = at else {
            return false;
        };
        let rect = spot(at);
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
        self.next.insert(
            key,
            Shown {
                at,
                offset,
                opacity,
                galley,
                stroke,
                edge,
            },
        );
        true
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
            Named::Group(_) | Named::Wall(_) => 0,
        }
    }

    /// Where a name that must show goes when no place is free: wherever it misleads
    /// least, over its own node if need be, then where it lies over the fewest other
    /// names and nodes, nearest; and whether it lies over another name there.
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
        // to it, from the node's edge.
        let end = pill.clamp(node);
        if !shown.at.over && (shown.at.shift != 0 || end.distance(node) > shown.edge + 10.0) {
            let start = node + (end - node).normalized() * shown.edge;
            let color = shown
                .stroke
                .unwrap_or(egui::Color32::from_white_alpha(170))
                .gamma_multiply(0.85);
            painter.line_segment([start, end], egui::Stroke::new(1.2, color));
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

fn label_hits_id() -> egui::Id {
    egui::Id::new("graph_label_hits")
}

/// A function's name as drawn: its pill, the function, and the journey number it
/// leads with.
type LabelHit = (egui::Rect, usize, Option<String>);

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
        let top = explorer.lens_top();
        let mut reach = lens.level(top);
        let unit = match (lens.measure, lens.reach) {
            (Measure::Calls, Some(1)) => "call",
            (Measure::Calls, _) => "calls",
            (_, Some(1)) => "step",
            _ => "steps",
        };
        if ui
            .add(
                egui::Slider::new(&mut reach, 1..=top)
                    .show_value(false)
                    .custom_formatter(|_, _| String::new()),
            )
            .on_hover_text("How far out from the start shows in full (- and =)")
            .changed()
        {
            lens.reach = (reach < top).then_some(reach);
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

/// Whether the graph is gathered into spheres.
fn nucleus_open(explorer: &Explorer) -> bool {
    explorer.nucleus().is_some()
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
    (up, right): (Vec3, Vec3),
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
    let center = explorer.lit_focus();
    // Gathered round the focus's module, the journey's cursor is named where it sits.
    let cursor = explorer.cursor_mark();
    // In a journey, a function selected off it, named as the focus is.
    let aside = explorer.aside().filter(|_| nucleus_open(explorer));
    let hovered = explorer.pointed();
    let reach = explorer.reach();
    let related = |tile: usize| reach.and_then(|r| graph_view::shown_relation(explorer, r, tile));
    let spotlit = explorer.spotlit();
    let disclosed = explorer.disclosure();
    let journey = disclosed.tiles();
    let opened = disclosed.opened();
    // The journey's start, which is named as the start come what may.
    let start = explorer
        .root()
        .and_then(|unit| map.tile_of(unit))
        .filter(|_| !explorer.browsing());
    let nucleus = explorer.nucleus();
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
            // Only what the lens shows at least half way gets a name. Gathered round
            // the focus's module, only the middle, what it names and what the pointer
            // is on or draws the calls of, however faint.
            let named = |n: &Nucleus| {
                n.shows(tile) && (n.lit(tile) || center == Some(tile) || hovered == Some(tile))
            };
            let world = explorer.tile_point(tile).filter(|_| match nucleus {
                Some(n) => named(n),
                None => explorer.visibility(tile) >= 0.5,
            })?;
            let node = to_screen(world)?;
            let scale = nucleus.map_or(1.0, |n| n.node_scale(false));
            let radius = graph_view::node_size(graph, tile) * scale / 2.0;
            let edge = to_screen(world + up * radius).map_or(8.0, |p| p.distance(node));
            Some((node, edge))
        })
        .collect();

    // Lower ranks are placed first and win any overlap.
    let mut order: Vec<(LabelRank, Named)> = Vec::new();
    for (tile, disc) in discs.iter().enumerate() {
        if disc.is_none() {
            continue;
        }
        // Gathered round the focus's module, one rule: the middle and the cursor,
        // then what the nucleus names, then the open journey round a middle off it;
        // what the pointer is on and its calls' ends come last, so pointing never
        // moves another name.
        let gathered = nucleus.map(|n| match () {
            () if center == Some(tile) || cursor == Some(tile) || aside == Some(tile) => Some(0),
            () if n.labelled(tile) && n.level(tile).is_some_and(|l| l.abs() <= 1) => Some(1),
            () if n.labelled(tile) => Some(2),
            () if n.context(tile) => Some(3),
            // The few calls in from outside the journey drawn for each module, those
            // nearest it, are named after all the journey's own names.
            () if n.outside(tile) => Some(6),
            () if hovered == Some(tile) || n.touched(tile) => Some(POINTED),
            () => None,
        });
        let tier = match related(tile) {
            _ if gathered.is_some() => match gathered.flatten() {
                Some(tier) => tier,
                None => continue,
            },
            _ if center == Some(tile) || cursor == Some(tile) => 0,
            // What the pointer is over, and what the spotlight picks out, is named,
            // whatever else would be.
            _ if hovered == Some(tile) || spotlit.contains(&tile) => 1,
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
            None if center.is_none() => 6,
            None => continue,
        };
        order.push((
            (tier, std::cmp::Reverse(graph.degree(tile)), tile),
            Named::Node(tile),
        ));
    }
    // Modules are named at the top of their rings, ahead of most function names.
    let regions = graph_view::regions(explorer);
    // Gathered round the focus's module, at the top of their spheres.
    let ring_top = |cluster: usize| {
        if let Some(nucleus) = nucleus {
            let ball = nucleus.ball(cluster)?;
            let top = to_screen(ball.center + up * ball.radius);
            if top.is_some() || !nucleus.browsing() {
                return top;
            }
            // Browsing, a sphere cut by the window's edge is named on what of it
            // shows.
            let (middle, reach) = (ball.center, ball.radius * 0.7);
            return [
                middle,
                middle + right * reach,
                middle - right * reach,
                middle - up * reach,
            ]
            .into_iter()
            .find_map(to_screen);
        }
        let &(_, middle, radius) = regions.iter().find(|r| r.0 == cluster)?;
        to_screen(middle + up * radius)
    };
    let wall = |cluster: usize| to_screen(nucleus?.wall(cluster)?);
    let left_out = nucleus
        .map(|n| n.outside_left_out(graph))
        .unwrap_or_default();
    if let Some(nucleus) = nucleus {
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
        // A call from the function under the pointer that leaves for a module off the
        // ring is named where it leaves.
        let mut walls: Vec<usize> = nucleus
            .touches()
            .iter()
            .flat_map(|t| [t.from, t.to])
            .filter_map(|end| match end {
                End::Module(cluster) => Some(cluster),
                End::Function(_) => None,
            })
            .filter(|&c| nucleus.ball(c).is_some_and(|b| b.body == Body::Closed))
            .collect();
        walls.sort_unstable();
        walls.dedup();
        for cluster in walls {
            order.push((
                (1, std::cmp::Reverse(nucleus.count(cluster)), cluster),
                Named::Wall(cluster),
            ));
        }
        for (cluster, ball) in nucleus.balls() {
            let rank = match ball.body {
                // Browsing, every module is named before any function, biggest first,
                // so what is clicked never moves a module's name or leaves one out.
                _ if browsing => (
                    0,
                    std::cmp::Reverse(usize::MAX),
                    usize::MAX - nucleus.count(cluster),
                ),
                // Which module a named function lives in matters as much as its name.
                Body::Nucleus | Body::Open => {
                    (0, std::cmp::Reverse(nucleus.count(cluster)), cluster)
                }
                Body::Closed | Body::Faint
                    if named.contains(&cluster) || left_out.contains_key(&cluster) =>
                {
                    (6, std::cmp::Reverse(nucleus.count(cluster)), cluster)
                }
                Body::Closed | Body::Faint => continue,
            };
            order.push((rank, Named::Group(cluster)));
        }
    } else if explorer.lens().reach.is_none() {
        for &(cluster, _, radius) in &regions {
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a rough size for ordering"
            )]
            let size = radius as usize;
            let tier = if center.is_some() { 4 } else { 1 };
            order.push((
                (tier, std::cmp::Reverse(size), cluster),
                Named::Group(cluster),
            ));
        }
    }
    order.sort_by_key(|&(key, _)| key);

    // Labels keep off the cards over the graph, as last laid out; the search box,
    // which comes and goes, last, as landmarks do not move for it.
    let cards: Vec<egui::Rect> = ["diff_card", "legend", "camera_free", "search"]
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
    // And off the nodes that matter, so they never hide a neighbor; what only the
    // pointer names is no reason for another name to move.
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
                        Named::Group(_) | Named::Wall(_) => None,
                    })
                    .map(|(node, edge)| {
                        egui::Rect::from_center_size(node, egui::Vec2::splat(edge * 2.0))
                    }),
            )
            .collect(),
        order
            .iter()
            .filter_map(|&(_, named)| match named {
                Named::Node(tile) => discs[tile].map(|(node, edge)| (tile, node, edge)),
                Named::Group(_) | Named::Wall(_) => None,
            })
            .collect(),
    );
    placer.cards = lasting;
    placer.landmarks = nucleus.is_some_and(Nucleus::browsing);
    // The cursor's name, and the one the pointer is on, read over the rest.
    placer.top = [hovered, cursor]
        .into_iter()
        .flatten()
        .map(Named::Node)
        .collect();
    for ((tier, _, _), named) in order {
        match named {
            Named::Group(group) => {
                let Some(at) = ring_top(group) else {
                    continue;
                };
                let body = nucleus.and_then(|n| n.ball(group)).map(|b| b.body);
                let color =
                    egui_color(lane_color(group % PALETTE.len())).gamma_multiply(match body {
                        Some(Body::Nucleus | Body::Open) => 1.0,
                        Some(Body::Faint) => 0.5,
                        Some(Body::Closed) => 0.35,
                        None if center.is_some() => 0.6,
                        None => 0.9,
                    });
                let label = explorer.cluster_name(group);
                // An open module says how much of it is named, and how many callers
                // from outside the journey it holds beyond those drawn.
                let named_note = nucleus
                    .filter(|n| body == Some(Body::Open) && n.opened(group) > 0)
                    .map(|n| format!("{} of {}", n.opened(group), n.count(group)));
                let more = left_out.get(&group).copied().unwrap_or(0);
                let more_note = (more > 0)
                    .then(|| format!("+{more} more caller{}", if more == 1 { "" } else { "s" }));
                let note = [named_note, more_note]
                    .into_iter()
                    .flatten()
                    .reduce(|a, b| format!("{a} · {b}"));
                let browsing = nucleus.is_some_and(Nucleus::browsing);
                // Browsing, one size whatever is clicked, so a name opening or closing
                // its module keeps its place.
                let size = match body {
                    _ if browsing => 11.0,
                    Some(Body::Nucleus) => 15.0,
                    Some(Body::Closed | Body::Faint) => 10.5,
                    _ => 12.0,
                };
                let job = label_job(None, &label.to_uppercase(), note, color, size, false);
                // Expanded, the calls drawn end at the modules on the ring; browsing,
                // every module is a landmark.
                let force = browsing
                    || body == Some(Body::Nucleus)
                    || (body == Some(Body::Open) && nucleus.is_some_and(Nucleus::expanded));
                let force = if force {
                    Force::Always
                } else if more > 0 {
                    Force::Free
                } else {
                    Force::Never
                };
                placer.place(named, at, 0.0, job, None, force);
            }
            Named::Wall(group) => {
                let Some(at) = wall(group) else {
                    continue;
                };
                let color = egui_color(lane_color(group % PALETTE.len()));
                let label = explorer.cluster_name(group).to_uppercase();
                let job = label_job(None, &label, None, color, 10.5, false);
                placer.place(named, at, 4.0, job, None, Force::Always);
            }
            Named::Node(tile) => {
                let Some((node, edge)) = discs[tile] else {
                    continue;
                };
                let change = diff.and_then(|d| d.change(tile));
                // A change is named in full: `new` alone could be any type's.
                let name = match (change, explorer.tile_unit(tile)) {
                    (Some(_), Some(unit)) => explorer.unit_name(unit),
                    _ => node_name(explorer, tile),
                };
                let relation = related(tile);
                // Gathered round the focus's module, colored as its call is drawn; a
                // faint speck named only under the pointer, and the open journey round
                // a middle off it, quietly.
                let level = nucleus.and_then(|n| n.level(tile));
                let speck = nucleus.is_some() && graph_view::speck(explorer, tile);
                let quiet = speck
                    || nucleus.is_some_and(|n| {
                        n.context(tile) || (n.outside(tile) && aside != Some(tile))
                    });
                let color = match (tier, relation) {
                    _ if cursor == Some(tile) => CURSOR_TEXT,
                    (0, _) => egui::Color32::WHITE,
                    _ if quiet => MUTED,
                    _ if nucleus.is_some() => match level {
                        Some(-1) => CALLER_TEXT,
                        Some(1) => CALLEE_TEXT,
                        Some(2..) => DEEPER_TEXT,
                        _ => egui::Color32::from_rgb(200, 205, 216),
                    },
                    (_, Some(Relation::Upstream(1))) => CALLER_TEXT,
                    (_, Some(Relation::Downstream(1))) => CALLEE_TEXT,
                    (_, Some(Relation::Downstream(_))) => DEEPER_TEXT,
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
                    // A start the journey calls again says so, after saying it is the
                    // start, so the call back is not read as its own.
                    _ if nucleus.is_some() && start == Some(tile) => {
                        let panel = journey.get(&tile).map_or(&[][..], Vec::as_slice);
                        let again = nucleus
                            .and_then(|n| nucleus::badge(n.numbers(tile).iter().chain(panel)));
                        (None, again.map(|again| format!("start · again at {again}")))
                    }
                    // Gathered round the focus's module, the journey's number says it;
                    // round a middle off the journey, which of its calls the journey
                    // runs too.
                    _ if nucleus.is_some_and(|n| n.on_journey(tile) && n.names(tile)) => {
                        (None, Some("on the journey".to_owned()))
                    }
                    _ if nucleus.is_some() => (None, None),
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
                let aside = [aside, (!marks.is_empty()).then(|| marks.join(" · "))]
                    .into_iter()
                    .flatten()
                    .reduce(|a, b| format!("{a} · {b}"));
                // A node the journey runs leads with its place in it, as its bar does.
                // Gathered round the focus's module, its numbers below the middle.
                // Gathered round the focus's module, the call nearest the root as the
                // panel numbers it, or the cursor's own, and how many more it runs.
                // Gathered, a function keeps one number for the journey, wherever the
                // cursor is: its first call in the panel's order. The start is the
                // start, however often the journey comes back to it.
                let order = match nucleus {
                    Some(_) if start == Some(tile) => None,
                    Some(n) => {
                        let panel = journey.get(&tile).map_or(&[][..], Vec::as_slice);
                        nucleus::badge(n.numbers(tile).iter().chain(panel))
                    }
                    None => journey
                        .get(&tile)
                        .filter(|numbers| !numbers.is_empty())
                        .map(|numbers| match numbers.as_slice() {
                            [one] => one.clone(),
                            [first, second] => format!("{first}, {second}"),
                            [first, .., last] => format!("{first} … {last}"),
                            [] => String::new(),
                        }),
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
                // read as one; what changed after it.
                let fill = egui_color(graph_view::node_color(explorer, diff, tile));
                let lead = match (order, change) {
                    (Some(order), Some(change)) => Some((
                        format!(" {order} {} ", diff_view::sign(change)),
                        ON_FILL,
                        Some(fill),
                    )),
                    (Some(order), None) => Some((format!(" {order} "), ON_FILL, Some(fill))),
                    (None, _) => lead,
                };
                // The cursor's name keeps the size it had before the cursor came, so it
                // keeps its place too.
                let size = match tier {
                    _ if cursor == Some(tile) && center != Some(tile) => 12.0,
                    0 => 14.0,
                    _ if quiet => 10.0,
                    1 => 12.0,
                    _ => 11.0,
                };
                // The pill's border says which way it relates to the focus, as its link
                // does; the cursor's is its own color.
                let stroke = match relation {
                    _ if cursor == Some(tile) => Some(egui_color(CURSOR)),
                    _ if tier == 0 => Some(egui::Color32::from_white_alpha(140)),
                    _ if quiet => None,
                    _ if nucleus.is_some() => match level {
                        Some(-1) => Some(egui_color(CALLER)),
                        Some(1) => Some(egui_color(CALLEE)),
                        Some(2..) => Some(egui_color(DEEPER)),
                        _ => None,
                    },
                    Some(Relation::Upstream(1)) => Some(egui_color(CALLER)),
                    Some(Relation::Downstream(1)) => Some(egui_color(CALLEE)),
                    _ if spotlit.contains(&tile) => Some(egui_color(match explorer.spotlight() {
                        Some((Side::Callers, _)) => CALLER,
                        _ => CALLEE,
                    })),
                    Some(Relation::Downstream(_)) => Some(egui_color(DEEPER).gamma_multiply(0.7)),
                    _ => None,
                };
                // What is gone is struck through, which reads at any size.
                let struck = change == Some(Change::Removed);
                let job = label_job(lead, &name, aside, color, size, struck);
                // The focus and what the pointer is over are named come what may;
                // what the spotlight picks out, and gathered round the focus's module
                // everything it names, wherever there is room.
                let force = if tier == 0 || hovered == Some(tile) {
                    Force::Always
                } else if spotlit.contains(&tile)
                    || (nucleus.is_some() && tier <= 2)
                    || nucleus.is_some_and(|n| n.outside(tile))
                {
                    Force::Free
                } else {
                    Force::Never
                };
                placer.place(named, node, edge, job, stroke, force);
            }
        }
    }
    // Every function lit and on screen is named, or counted here as not.
    let lit: Vec<usize> = nucleus.map_or_else(Vec::new, |n| {
        (0..discs.len())
            .filter(|&tile| discs[tile].is_some() && n.names(tile))
            .collect()
    });
    let unnamed = lit
        .iter()
        .filter(|&&tile| !placer.next.contains_key(&Named::Node(tile)))
        .count();
    placer.finish(|named| match named {
        Named::Node(tile) => discs.get(tile).copied().flatten().map(|(node, _)| node),
        Named::Group(group) => ring_top(group),
        Named::Wall(group) => wall(group),
    });
    if unnamed > 0 {
        unnamed_note(painter, unnamed, lit.len());
    }
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
    let hidden = explorer.hidden();
    let outside = explorer.outside_shown();
    let gathered_journey = explorer.nucleus().is_some() && !explorer.browsing();
    let mut pressed = None;
    let mut follow = false;
    let mut unhide = None;
    ui.horizontal(|ui| {
        unhide = hidden_chips(ui, hidden);
        for (side, count, noun, key, color) in [
            (Side::Callers, calling, "caller", "I", CALLER),
            (Side::Callees, called.len(), "call", "O", CALLEE),
        ] {
            let plural = if count == 1 { "" } else { "s" };
            let text = egui::RichText::new(format!("● {count} {noun}{plural}  {key}"))
                .color(egui_color(color));
            let hint = match side {
                Side::Callers if gathered_journey => {
                    "Bring in every function off the journey that calls one on it (I)"
                }
                Side::Callers => "Pick out what calls it (I)",
                Side::Callees => "Pick out what it calls (O)",
            };
            let on = lit == Some(side) || (side == Side::Callers && outside);
            if ui
                .add_enabled(count > 0, egui::Button::selectable(on, text))
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
        .on_hover_text(
            "The focus: what I, O and Enter act on. Browsing, the function clicked; in a \
             journey, its cursor, or a function clicked off it",
        );
        ui.label(
            egui::RichText::new(if explorer.browsing() {
                "selected"
            } else {
                "focus"
            })
            .small()
            .color(MUTED),
        );
        if explorer.aside().is_some() {
            let chip = egui::Button::new(
                egui::RichText::new("not on this journey · Enter starts one here")
                    .small()
                    .color(MUTED),
            )
            .corner_radius(8.0);
            if ui
                .add(chip)
                .on_hover_text(
                    "Selected off the journey, in place. Enter, or a click here, starts a \
                     journey here; an arrow or Esc goes back to the journey's cursor",
                )
                .clicked()
            {
                follow = true;
            }
        }
    });
    match pressed {
        Some(Side::Callers) => explorer.press_callers(now),
        Some(side) => explorer.toggle_spotlight(side, now),
        None => {}
    }
    if let Some(side) = unhide {
        explorer.toggle_hidden(side);
    }
    if follow {
        explorer.trace_from_focus();
    }
}

/// What Shift+I and Shift+O left out, said where the sides are; returns the side
/// whose chip was clicked, to bring it back.
fn hidden_chips(ui: &mut egui::Ui, hidden: Hidden) -> Option<Side> {
    let mut unhide = None;
    for (side, on, text) in [
        (Side::Callees, hidden.callees, "calls hidden · Shift+O"),
        (Side::Callers, hidden.callers, "callers hidden · Shift+I"),
    ] {
        let chip = egui::Button::new(
            egui::RichText::new(text)
                .small()
                .color(egui::Color32::WHITE),
        )
        .corner_radius(8.0);
        if on
            && ui
                .add(chip)
                .on_hover_text("Left out of the view; a click, or the key, brings them back")
                .clicked()
        {
            unhide = Some(side);
        }
    }
    unhide
}

/// Says the camera has been panned or zoomed away and no longer follows the journey,
/// top left of the graph; a click, like F, brings it back.
fn camera_free(
    ctx: &egui::Context,
    area: egui::Rect,
    explorer: &Explorer,
    camera: &mut OrbitCamera,
) {
    egui::Area::new("camera_free".into())
        .fixed_pos(area.min + egui::vec2(12.0, 10.0))
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

/// In a diff, what a node's fill says. A hint at the keys along `foot`.
fn legend(ctx: &egui::Context, area: egui::Rect, foot: egui::Rect, view: Showing) {
    if view.diffing {
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
                            for (color, text) in [
                                (diff_view::ADDED, "added"),
                                (diff_view::MODIFIED, "rewritten"),
                                (diff_view::REMOVED, "removed"),
                                (diff_view::MOVED, "moved"),
                                (graph_view::UNCHANGED, "unchanged"),
                            ] {
                                ui.label(egui::RichText::new("●").color(egui_color(color)));
                                ui.label(egui::RichText::new(text).size(12.5));
                                ui.add_space(6.0);
                            }
                        });
                    });
            });
    }
    ctx.layer_painter(egui::LayerId::background())
        .with_clip_rect(foot)
        .text(
            egui::pos2(foot.min.x + 12.0, foot.max.y - 10.0),
            egui::Align2::LEFT_BOTTOM,
            if view.browsing {
                "Browsing · click a function to select it, nothing moves · Enter opens its \
                 journey · I callers · O calls · Backspace back · / search · R whole map · H keys"
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
    /// Gathered into spheres (Z), rather than laid flat.
    gathered: bool,
    diffing: bool,
    /// Browsing, rather than in a journey.
    browsing: bool,
}

/// Which view a key does something in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum When {
    Always,
    /// The graph laid flat.
    Flat,
    /// The graph gathered into spheres (Z).
    Nucleus,
    /// A diff open over the graph.
    Diff,
    /// No journey open.
    Browse,
    /// A journey open.
    Journey,
    /// A journey open, gathered round its start.
    JourneyNucleus,
    /// Browsing, gathered into spheres.
    BrowseNucleus,
}

impl Showing {
    fn of(explorer: &Explorer, diffing: bool) -> Self {
        Self {
            gathered: explorer.nucleus().is_some(),
            diffing,
            browsing: explorer.browsing(),
        }
    }
}

impl When {
    fn applies(self, view: Showing) -> bool {
        match self {
            Self::Always => true,
            Self::Flat => !view.gathered,
            Self::Nucleus => view.gathered,
            Self::Diff => view.diffing,
            Self::Browse => view.browsing,
            Self::Journey => !view.browsing,
            Self::JourneyNucleus => !view.browsing && view.gathered,
            Self::BrowseNucleus => view.browsing && view.gathered,
        }
    }
}

/// A group of keys by title: each key, what it does, and in which view.
type KeyGroup = (&'static str, &'static [(&'static str, &'static str, When)]);

/// Every key, grouped by what it moves, each saying what it does in the view it
/// applies in; H lists those that apply to the view open.
const KEYS: [KeyGroup; 8] = [
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
                "close search, this list, the spotlight, a function selected off the \
                 journey; then leave the journey for browsing",
                When::Journey,
            ),
            (
                "Esc",
                "close search, this list, the spotlight, the side panel",
                When::Browse,
            ),
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
            ("Shift ]", "step over this call's own calls", When::Journey),
            (
                "Shift ↑",
                "step out to what runs after its caller",
                When::Journey,
            ),
            ("1 – 9", "to the call with that number", When::Journey),
            ("click a bar", "to that call", When::Journey),
            ("Space", "play forward through the journey", When::Journey),
        ],
    ),
    (
        "Journey depth",
        &[
            (
                ",  .",
                "show a level less / more of the journey",
                When::Journey,
            ),
            (
                "V",
                "show calls by level / only as far as the cursor ran",
                When::Journey,
            ),
        ],
    ),
    (
        "Calls in and out",
        &[
            ("I", "ring everything that calls the focus", When::Browse),
            ("I", "ring everything that calls the focus", When::Flat),
            (
                "I",
                "bring in every function off the journey that calls one on it, pale",
                When::JourneyNucleus,
            ),
            ("O", "ring everything the focus calls", When::Always),
            (
                "Shift I",
                "hide / show the callers (violet); the top bar says when they are hidden",
                When::Always,
            ),
            (
                "Shift O",
                "hide / show the calls out (blue, amber)",
                When::Always,
            ),
        ],
    ),
    (
        "Spheres",
        &[
            ("Z", "gather the graph into module spheres", When::Flat),
            ("Z", "lay the graph flat again", When::Nucleus),
            (
                "X, dbl-click",
                "expand the start's module / shrink it back; inside, a click centres it",
                When::Journey,
            ),
            ("N", "hide / show the unnamed specks", When::Nucleus),
            (
                "point",
                "expanded: rest on a function to draw its own calls, pale",
                When::JourneyNucleus,
            ),
        ],
    ),
    (
        "Graph",
        &[
            (
                "-  =",
                "reveal a ring less / more round the start",
                When::Flat,
            ),
            ("L", "reveal round: focus → changes → journey", When::Flat),
            ("G", "nearness by: calls → space → blend", When::Flat),
            ("K", "cluster by modules / calls", When::Flat),
            ("C", "only what changed / everything", When::Diff),
            ("D", "status → impact → morph", When::Diff),
            ("T", "play or pause the morph", When::Diff),
        ],
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
            ("Q  E", "turn an eighth of the way round", When::Always),
            ("F", "face the journey's start again", When::Flat),
            (
                "F",
                "back to the usual view, round what is lit",
                When::Nucleus,
            ),
            ("R", "frame the whole graph", When::Flat),
            (
                "R",
                "frame the whole codebase from this angle",
                When::BrowseNucleus,
            ),
            (
                "R",
                "frame the journey from this angle",
                When::JourneyNucleus,
            ),
        ],
    ),
    (
        "Layout",
        &[
            ("J", "show / hide the journey", When::Journey),
            ("M", "swap graph and journey", When::Journey),
            ("S", "show / hide the side panel", When::Always),
            ("H", "this list", When::Always),
        ],
    ),
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
        ];
    }
    &[
        "Each sphere is a module, in a color of its own; it says how many of its functions \
         the journey names (2 of 23). The middle one holds the start.",
        "On a name, 2 +1: the function's first call in the journey, numbered as in the \
         panel, and how many more times the journey runs it; the number stays put as the \
         cursor moves.",
        "The panel: a bar per call, left to right in the order they run, a row a level \
         deeper. A dashed bar runs only sometimes; +N on a bar is N calls folded under it, \
         a click opens them; a green end returns, a red one fails. The line from a bar \
         leads to its function.",
    ]
}

/// Below this height the keys scroll.
const ROOM_FOR_KEYS: f32 = 560.0;

/// Width of the keys' column in the help.
const KEY_WIDTH: f32 = 112.0;

/// The keys that apply to the view open, in as many columns as fit `area`, scrolling
/// past its height.
fn key_help(ctx: &egui::Context, area: egui::Rect, open: &mut bool, view: Showing) {
    let margin = 16.0;
    let width = (area.width() - 2.0 * margin).clamp(240.0, 1180.0);
    let height = (area.height() - 2.0 * margin).max(160.0);
    let columns = match width {
        w if w >= 900.0 => 3,
        w if w >= 560.0 => 2,
        _ => 1,
    };
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
    // Each section to the shortest column so far, in order.
    let mut placed: Vec<Vec<usize>> = vec![Vec::new(); columns];
    let mut lengths = vec![0; columns];
    for (index, (_, keys)) in sections.iter().enumerate() {
        let shortest = (0..columns).min_by_key(|&c| lengths[c]).unwrap_or(0);
        placed[shortest].push(index);
        lengths[shortest] += keys.len() + 2;
    }
    egui::Area::new("key_help".into())
        .pivot(egui::Align2::CENTER_TOP)
        .fixed_pos(egui::pos2(area.center().x, area.min.y + margin))
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_premultiplied(11, 13, 19, 245))
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(60)))
                .corner_radius(10.0)
                .inner_margin(egui::Margin::symmetric(16, 12))
                .show(ui, |ui| {
                    ui.set_width(width - 32.0);
                    ui.horizontal(|ui| {
                        let title = match (view.browsing, view.diffing) {
                            (true, _) => "Keys while browsing: no journey open",
                            (false, true) => "Keys in a journey, over a diff",
                            (false, false) => "Keys in a journey",
                        };
                        ui.label(egui::RichText::new(title).strong().size(14.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("close (H)").clicked() {
                                *open = false;
                            }
                        });
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        for (color, what) in color_key(view.browsing) {
                            ui.label(egui::RichText::new("●").color(color));
                            ui.label(egui::RichText::new(what).size(12.0).color(MUTED));
                            ui.add_space(8.0);
                        }
                    });
                    ui.label(
                        egui::RichText::new(if view.browsing {
                            "Browsing, a click selects a function and moves nothing. Enter \
                             opens its journey."
                        } else {
                            "In a journey, the view gathers round where it starts and shows \
                             only the journey; a click moves its cursor and nothing else. Esc \
                             leaves it for browsing."
                        })
                        .size(12.0)
                        .color(egui::Color32::WHITE),
                    );
                    for line in reading(view.browsing) {
                        ui.add(
                            egui::Label::new(egui::RichText::new(*line).size(12.0).color(MUTED))
                                .wrap(),
                        );
                    }
                    ui.add_space(6.0);
                    let keys = |ui: &mut egui::Ui| {
                        ui.columns(columns, |cols| {
                            for (col, indices) in cols.iter_mut().zip(&placed) {
                                for &index in indices {
                                    let (title, keys) = &sections[index];
                                    key_section(col, title, keys);
                                }
                            }
                        });
                    };
                    // Whole where the window has room, so no section is ever cut off at
                    // a height measured the frame before; scrolling where it has not.
                    if height >= ROOM_FOR_KEYS {
                        keys(ui);
                    } else {
                        egui::ScrollArea::vertical()
                            .max_height(height - 70.0)
                            .auto_shrink([false, true])
                            .show(ui, keys);
                    }
                });
        });
}

/// One group of keys, each key in a fixed column and what it does wrapping beside it.
fn key_section(ui: &mut egui::Ui, title: &str, keys: &[(&str, &str)]) {
    ui.label(egui::RichText::new(title).strong().size(13.0));
    ui.add_space(2.0);
    for (key, what) in keys {
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(KEY_WIDTH, 14.0),
                egui::Layout::left_to_right(egui::Align::Min),
                |ui| {
                    ui.set_min_width(KEY_WIDTH);
                    ui.label(
                        egui::RichText::new(*key)
                            .monospace()
                            .size(11.5)
                            .color(egui::Color32::WHITE),
                    );
                },
            );
            ui.add(egui::Label::new(egui::RichText::new(*what).size(12.0).color(MUTED)).wrap());
        });
    }
    ui.add_space(8.0);
}

/// The last two segments of a path: `orders::Order::total` is `Order.total`.
fn tail(path: &str) -> String {
    let segments: Vec<&str> = path.split("::").collect();
    segments[segments.len().saturating_sub(2)..].join(".")
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
            let job = label_job(None, "draw_graph", None, egui::Color32::WHITE, 12.5, false);
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
        for (gathered, diffing, browsing) in [
            (true, false, false),
            (true, false, true),
            (false, false, false),
            (false, true, false),
        ] {
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
                    let view = Showing {
                        gathered,
                        diffing,
                        browsing,
                    };
                    key_help(ui.ctx(), area, &mut open, view);
                });
                output.textures_delta.clear();
            }
            let shown = ctx
                .memory(|m| m.area_rect(egui::Id::new("key_help")))
                .expect("the list shows");
            eprintln!("{gathered} {diffing}: {shown:?}");
            assert!(
                area.contains_rect(shown),
                "inside the window: {shown:?} in {area:?}"
            );
        }
    }
}
