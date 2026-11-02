//! egui around the graph: the top bar, and the journey docked along the foot; on the
//! canvas, names beside nodes ([`labels`]), the keys ([`key_help`]) and a search box
//! on `/`.

use bevy::{camera::Viewport, prelude::*, transform::TransformSystems, window::PrimaryWindow};
use bevy_egui::{
    EguiContext, EguiContexts, EguiInput, EguiPostUpdateSet, EguiPreUpdateSet,
    EguiPrimaryContextPass, PrimaryEguiContext, egui,
};
use codeviz_ir::UnitId;

use crate::{
    arrows,
    camera::OrbitCamera,
    colors::{CALLER_TEXT, MUTED},
    drive::Marks,
    explorer::Explorer,
    framing::GraphView,
    idle::Awake,
    journey_panel::{self, JourneyPanel},
    key_help::{hint, key_help, key_hint},
    labels::draw_graph_labels,
    text,
};

pub(crate) struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Search>()
            .init_resource::<KeyHelp>()
            .init_resource::<Chrome>()
            .init_resource::<GraphView>()
            .add_systems(
                PreUpdate,
                keys_to_the_graph
                    .after(EguiPreUpdateSet::ProcessInput)
                    .before(EguiPreUpdateSet::BeginPass),
            )
            // Names and arrows are drawn where the camera stands this frame, once it
            // has moved, not where it stood the frame before.
            .configure_sets(
                PostUpdate,
                EguiPostUpdateSet::EndPass.after(TransformSystems::Propagate),
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
/// egui's text styles keep to [`text`]'s sizes.
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
    let ctx = contexts.ctx_mut()?;
    ctx.set_fonts(fonts);
    ctx.all_styles_mut(text::styles);
    *done = true;
    Ok(())
}

/// Whether the key reference is showing.
#[derive(Resource, Debug, Default)]
pub(crate) struct KeyHelp(pub(crate) bool);

#[derive(Resource, Debug, Default)]
struct Search {
    query: String,
    /// The functions the query finds, best first, as far as the box lists them.
    rows: Vec<Row>,
    /// How many it finds in all.
    found: usize,
    /// The query `rows` were found for.
    found_for: Option<String>,
    /// The row ↑ and ↓ have moved to, which Enter picks.
    highlighted: usize,
    /// Whether the box was open the frame before, so opening it selects what it
    /// holds, to type over.
    was_open: bool,
}

/// A function the search finds: what it is called, the module it is in, and its
/// kind.
#[derive(Debug, Clone)]
struct Row {
    unit: UnitId,
    name: String,
    module: String,
    kind: String,
}

/// At most this many functions are listed; the box says how many more it found.
const LISTED: usize = 100;

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
    mut graph_view: ResMut<GraphView>,
    mut marks: ResMut<Marks>,
    mut awake: ResMut<Awake>,
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
    // The graph is drawn under the whole of it, the journey panel over its foot, so a
    // level more or less, which changes how tall the panel is, moves nothing; names,
    // the search box and the framing keep to the free area above the panel.
    graph_view.0 = Some(central);
    let (graph_area, panel_area) = areas(central, &explorer);
    if let Ok((mut camera, transform, _)) = cameras.single_mut() {
        fit_viewport(&mut camera, &window, central);
        let project = |position: Vec3| {
            camera
                .world_to_viewport(transform, position)
                .ok()
                .map(|p| egui::pos2(p.x, p.y))
        };
        let to_screen = |position: Vec3| project(position).filter(|p| graph_area.contains(*p));
        let painter = ctx
            .layer_painter(egui::LayerId::background())
            .with_clip_rect(graph_area);
        // The calls under the names, on under the journey panel as the spheres are.
        let under = painter.with_clip_rect(central);
        arrows::draw(&under, &explorer, (&camera, transform), &mut marks);
        draw_graph_labels(&painter, &project, &explorer, transform.up().into());
        let opening = chrome.search && !search.was_open;
        search.was_open = chrome.search;
        if chrome.search {
            // What it finds is laid out a frame after it is found: one more follows.
            awake.0 |= search_box(
                ctx,
                graph_area,
                &mut explorer,
                &mut search,
                &mut chrome.search,
                opening,
            );
        }
        // In a journey, the hint at the keys sits in the panel's header.
        let expanded = explorer.nucleus().expanded();
        if explorer.browsing() {
            key_hint(ctx, graph_area, expanded);
        }
        if help.0 {
            // Down to the window's foot, over the journey panel where it must, so the
            // whole list fits rather than scrolls.
            let room = egui::Rect::from_min_max(
                graph_area.min,
                egui::pos2(graph_area.max.x, ctx.content_rect().max.y),
            );
            key_help(ctx, room, &mut help.0, explorer.browsing());
        }
        if let Some(area) = panel_area {
            let tethers =
                journey_panel::draw(ctx, area, &hint(false, expanded), &mut explorer, &mut panel);
            // Not over the key help, which would read as crossed out.
            if !help.0 {
                draw_tethers(ctx, &tethers, &explorer, &to_screen);
            }
        }
    }
    Ok(())
}

/// The bar along the top: Back and Forward and which mode the view is in on the left;
/// search and the chip for a function set aside on the right. Returns how tall it is.
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
        .hovered()
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

/// Where the view is, always in the same place: `Browsing`, or `Journey ·` and where
/// it starts; then, with a module expanded, `· inside` it.
fn mode_label(ui: &mut egui::Ui, explorer: &Explorer) {
    let (mut text, mut hint) = match explorer.start_unit() {
        Some(_) => (
            format!("Journey · {}", journey_title(explorer)),
            "A journey is open: a click on a function it runs moves its cursor; \
             Esc leaves it for browsing"
                .to_owned(),
        ),
        None => (
            "Browsing".to_owned(),
            "No journey is open: a click selects a function and moves nothing; \
             Enter or Shift-click opens its journey"
                .to_owned(),
        ),
    };
    if let Some(module) = inside(explorer) {
        text = format!("{text} · inside {module}");
        hint = format!("Zoomed into {module}, showing all its functions. X or Esc: back out");
    }
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

/// A box to find a function by name, over the top of the graph: ↑ and ↓ move along
/// what it finds, Enter or a click takes it, Escape closes the box. Opened again, it
/// holds the last query, selected, so typing replaces it. Returns whether what it
/// finds changed.
fn search_box(
    ctx: &egui::Context,
    area: egui::Rect,
    explorer: &mut Explorer,
    search: &mut Search,
    open: &mut bool,
    opening: bool,
) -> bool {
    let refound = search.found_for.as_deref() != Some(search.query.as_str());
    if refound {
        (search.rows, search.found) = find(explorer, &search.query);
        search.found_for = Some(search.query.clone());
        search.highlighted = 0;
    }
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
                    // Before the text box, which would take them to move its caret.
                    let moved = highlight_keys(ui, search);
                    let id = egui::Id::new("search_query");
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut search.query)
                            .id(id)
                            .hint_text("search functions…")
                            .desired_width(f32::INFINITY),
                    );
                    field.request_focus();
                    if opening {
                        select_all(ui.ctx(), id, &search.query);
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        chosen = search
                            .rows
                            .get(search.highlighted)
                            .map(|row| row.unit.clone());
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        *open = false;
                    }
                    if search.rows.is_empty() {
                        ui.label(egui::RichText::new("no function matches").color(MUTED));
                    }
                    let mut drawn = Vec::new();
                    let shown = egui::ScrollArea::vertical()
                        .id_salt("units")
                        .max_height(300.0)
                        .show(ui, |ui| {
                            for (index, row) in search.rows.iter().enumerate() {
                                let highlighted = index == search.highlighted;
                                ui.horizontal(|ui| {
                                    let picked = ui
                                        .selectable_label(highlighted, &row.name)
                                        .on_hover_text(row.unit.as_str());
                                    if picked.clicked() {
                                        chosen = Some(row.unit.clone());
                                    }
                                    if highlighted && moved {
                                        picked.scroll_to_me(None);
                                    }
                                    drawn.push(crate::drive::SearchRow {
                                        rect: picked.rect,
                                        unit: row.unit.to_string(),
                                        text: row.name.clone(),
                                        highlighted,
                                    });
                                    ui.label(egui::RichText::new(&row.module).color(MUTED));
                                    ui.label(egui::RichText::new(&row.kind).small().color(MUTED));
                                });
                            }
                        });
                    // Where each row is, for a scripted session to pick.
                    crate::drive::note_search_rows(ui.ctx(), shown.inner_rect, drawn);
                    let more = search.found.saturating_sub(search.rows.len());
                    if more > 0 {
                        ui.label(
                            egui::RichText::new(format!("{more} more: type more of the name"))
                                .small()
                                .color(MUTED),
                        );
                    }
                });
        });
    // Browsing, the function found is selected where it sits, as a click selects it;
    // in a journey, a journey opens there.
    if let Some(id) = chosen {
        match explorer.map().tile_of(&id) {
            Some(tile) if explorer.browsing() => explorer.click(tile, None),
            Some(tile) => explorer.open_at(tile),
            None => {}
        }
        *open = false;
    }
    refound
}

/// Selects all of a text box's text, so what is typed replaces it.
fn select_all(ctx: &egui::Context, id: egui::Id, text: &str) {
    if let Some(mut state) = egui::TextEdit::load_state(ctx, id) {
        let all = egui::text::CCursorRange::two(
            egui::text::CCursor::new(0),
            egui::text::CCursor::new(text.chars().count()),
        );
        state.cursor.set_char_range(Some(all));
        state.store(ctx, id);
    }
}

/// ↓ and ↑ move the highlight along the rows found. Returns whether it moved.
fn highlight_keys(ui: &egui::Ui, search: &mut Search) -> bool {
    let (down, up) = ui.input_mut(|i| {
        (
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown),
            i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp),
        )
    });
    let before = search.highlighted;
    if down {
        search.highlighted = (before + 1).min(search.rows.len().saturating_sub(1));
    }
    if up {
        search.highlighted = before.saturating_sub(1);
    }
    search.highlighted != before
}

/// The functions `query` finds, one row each, best first, as many as are listed, and
/// how many it finds in all. With no query, the entry points. A function is found by
/// its name, its path, or its route, and listed as the graph names it, with its type,
/// and its module (as its sphere is named, so two modules of one name are told apart
/// by their crate).
fn find(explorer: &Explorer, query: &str) -> (Vec<Row>, usize) {
    let system = explorer.system();
    let map = explorer.map();
    let route = |unit: &UnitId| system.entries().iter().find(|e| &e.unit == unit);
    let row = |tile: usize, unit: &UnitId| {
        let entry = route(unit);
        Row {
            unit: unit.clone(),
            name: entry.map_or_else(|| explorer.unit_name(unit), |e| e.label.clone()),
            module: explorer
                .graph()
                .cluster(tile)
                .map(|cluster| explorer.cluster_name(cluster))
                .unwrap_or_default(),
            kind: entry
                .map(|e| e.kind.clone())
                .or_else(|| system.unit(unit).map(|u| u.kind.clone()))
                .unwrap_or_default(),
        }
    };
    // Each function once: a closure is found as the function it is part of.
    let functions = map
        .tiles
        .iter()
        .enumerate()
        .map(|(tile, t)| (tile, &t.unit));
    if query.trim().is_empty() {
        let mut rows: Vec<Row> = functions
            .filter(|(_, unit)| route(unit).is_some())
            .map(|(tile, unit)| row(tile, unit))
            .collect();
        rows.sort_by(|a, b| a.name.cmp(&b.name));
        let found = rows.len();
        rows.truncate(LISTED);
        return (rows, found);
    }
    let mut ranked: Vec<(u8, Row)> = functions
        .filter_map(|(tile, unit)| {
            // Its own name, and an entry point's route.
            let names: Vec<&str> = system
                .unit(unit)
                .map(|u| u.name.as_str())
                .into_iter()
                .chain(route(unit).map(|e| e.label.as_str()))
                .collect();
            let rank = search_rank(query, unit.as_str(), &names, &explorer.unit_name(unit))?;
            Some((rank, row(tile, unit)))
        })
        .collect();
    ranked.sort_by(|(a, x), (b, y)| (a, x.name.len(), &x.name).cmp(&(b, y.name.len(), &y.name)));
    let found = ranked.len();
    let rows = ranked
        .into_iter()
        .take(LISTED)
        .map(|(_, row)| row)
        .collect();
    (rows, found)
}

/// How well `query` finds a function, best first, or `None` when it does not: by the
/// function's own `names` (its name, an entry point's route) first, exactly, then
/// their start, then anywhere in them; or exactly as the graph names it in full
/// (`qualified`, as `orders.place`) or by the end of its path; then anywhere in its
/// path, its module's name among it; then each word of the query in order along the
/// path. `.` and `::` are the same, so `orders.place` finds `services::orders::place`.
fn search_rank(query: &str, id: &str, names: &[&str], qualified: &str) -> Option<u8> {
    let normal = |text: &str| text.to_lowercase().replace("::", ".");
    let query = normal(query.trim());
    let names: Vec<String> = names.iter().map(|name| normal(name)).collect();
    // The path without its producer's prefix, as in `rust:`.
    let path = normal(id.split_once(':').map_or(id, |(_, path)| path));
    if names.contains(&query)
        || normal(qualified) == query
        || path == query
        || path.ends_with(&format!(".{query}"))
    {
        return Some(0);
    }
    if names.iter().any(|name| name.starts_with(&query)) {
        return Some(1);
    }
    if names.iter().any(|name| name.contains(&query)) {
        return Some(2);
    }
    if path.contains(&query) || normal(qualified).contains(&query) {
        return Some(3);
    }
    let mut rest = path.as_str();
    for word in query
        .split(|c: char| c == '.' || c.is_whitespace())
        .filter(|w| !w.is_empty())
    {
        let at = rest.find(word)?;
        rest = &rest[at + word.len()..];
    }
    Some(4)
}

/// With a function set aside, a chip that says what it is to the journey; a click on
/// it, as Enter does, starts a journey there.
fn aside_chip(ui: &mut egui::Ui, explorer: &mut Explorer) {
    if explorer.aside().is_none() {
        return;
    }
    let said = if explorer.aside_caller().is_some() {
        let start = explorer
            .start_unit()
            .map(|unit| explorer.function_name(unit))
            .unwrap_or_default();
        format!("calls {start} · Enter starts a journey here")
    } else {
        "not on this journey · Enter starts one here".to_owned()
    };
    let chip = egui::Button::new(
        egui::RichText::new(said)
            .size(text::BODY)
            .color(egui::Color32::WHITE),
    )
    .fill(egui::Color32::from_rgb(70, 52, 104))
    .stroke(egui::Stroke::new(1.0, CALLER_TEXT))
    .corner_radius(8.0);
    if ui
        .add(chip)
        .on_hover_text(
            "Selected in place. Enter, or a click here, starts a journey here; an arrow \
             or Esc goes back to the journey",
        )
        .clicked()
        && let Some(tile) = explorer.current()
    {
        explorer.open_at(tile);
    }
}

/// The graph's free area within its view `central`, and the strip along the foot the
/// journey panel takes when a journey is open.
pub(crate) fn areas(central: egui::Rect, explorer: &Explorer) -> (egui::Rect, Option<egui::Rect>) {
    if explorer.browsing() {
        (central, None)
    } else {
        let (free, strip) = journey_panel::split(central, journey_panel::rows(explorer));
        (free, Some(strip))
    }
}

/// Restricts 3D rendering to the area under the top bar.
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

/// The module expanded, by the name its sphere carries.
pub(crate) fn inside(explorer: &Explorer) -> Option<String> {
    explorer
        .nucleus()
        .expanded_module()
        .map(|module| explorer.cluster_name(module).to_uppercase())
}

/// The journey's entry route when it starts at one, else its start's name.
pub(crate) fn journey_title(explorer: &Explorer) -> String {
    let Some(start) = explorer.start_unit() else {
        return String::new();
    };
    explorer
        .system()
        .entries()
        .iter()
        .find(|e| &e.unit == start)
        .map_or_else(|| explorer.unit_name(start), |e| e.label.clone())
}

pub(crate) fn short(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use codeviz_core::System;
    use codeviz_ir::Document;

    use super::*;

    #[test]
    fn search_finds_each_function_once_by_route_name_or_path() {
        // place_order serves a route; place calls it; many helpers share a prefix.
        let helpers: Vec<String> = (0..LISTED + 5)
            .map(|n| {
                format!(r#"{{ "id": "x:orders::helper{n}", "name": "helper{n}", "kind": "fn", "role": "invocable", "parent": "x:orders" }}"#)
            })
            .collect();
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }},
            "entries": [ {{ "unit": "x:handlers::place_order", "label": "POST /users/{{id}}/orders", "kind": "http" }} ],
            "units": [
                {{ "id": "x:handlers", "name": "handlers", "kind": "module", "role": "group" }},
                {{ "id": "x:orders", "name": "orders", "kind": "module", "role": "group" }},
                {{ "id": "x:handlers::place_order", "name": "place_order", "kind": "fn", "role": "invocable", "parent": "x:handlers" }},
                {{ "id": "x:orders::place", "name": "place", "kind": "fn", "role": "invocable", "parent": "x:orders" }},
                {} ] }}"#,
            helpers.join(",")
        );
        let explorer = Explorer::new(System::new([Document::from_json(&json).unwrap()]));
        let names = |query: &str| -> Vec<String> {
            find(&explorer, query)
                .0
                .into_iter()
                .map(|row| row.name)
                .collect()
        };
        assert_eq!(
            names("POST /users/{id}/orders"),
            ["POST /users/{id}/orders"]
        );
        assert_eq!(
            names("place_order"),
            ["POST /users/{id}/orders"],
            "found by its name too, and listed once, by its route"
        );
        assert_eq!(names("orders.place"), ["orders.place"]);
        assert_eq!(
            names("order")[..2],
            ["POST /users/{id}/orders", "orders.place"],
            "a function whose name holds the word before one whose module does"
        );
        let (rows, found) = find(&explorer, "helper");
        assert_eq!(
            (rows.len(), found),
            (LISTED, LISTED + 5),
            "listed up to a hundred"
        );
        assert_eq!(rows[0].module, "orders", "with the module it is in");
        assert_eq!(
            names(""),
            ["POST /users/{id}/orders"],
            "no query: the entry points"
        );
    }

    #[test]
    fn search_reads_dots_as_paths() {
        let place = |query| {
            search_rank(
                query,
                "rust:axum_server::services::orders::place",
                &["place"],
                "orders.place",
            )
        };
        assert_eq!(place("orders.place"), Some(0), "as the graph names it");
        assert_eq!(place("orders::place"), Some(0), "as Rust writes it");
        assert_eq!(place("pla"), Some(1), "its name's start");
        assert_eq!(place("lac"), Some(2), "within its name");
        assert_eq!(place("ord"), Some(3), "its module's name");
        assert_eq!(place("services.orders"), Some(3));
        assert_eq!(
            place("server place"),
            Some(4),
            "words in order along the path"
        );
        assert_eq!(place("place orders"), None, "but not out of order");
        assert_eq!(place("quote"), None);
    }
}
