//! egui around the graph: the top bar, and the journey docked along the foot; on the
//! canvas, names beside nodes ([`labels`]), the keys ([`key_help`]) and a search box
//! on `/`.

use bevy::{camera::Viewport, prelude::*, window::PrimaryWindow};
use bevy_egui::{
    EguiContext, EguiContexts, EguiInput, EguiPreUpdateSet, EguiPrimaryContextPass,
    PrimaryEguiContext, egui,
};
use codeviz_ir::{Role, UnitId};

use crate::{
    camera::OrbitCamera,
    colors::{CALLER_TEXT, MUTED},
    explorer::Explorer,
    framing::GraphView,
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
}

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
    // The graph is drawn under the whole of it, the journey panel over its foot, so a
    // level more or less, which changes how tall the panel is, moves nothing; names,
    // the search box and the framing keep to the free area above the panel.
    graph_view.0 = Some(central);
    let (graph_area, panel_area) = areas(central, &explorer);
    if let Ok((mut camera, transform, orbit)) = cameras.single_mut() {
        fit_viewport(&mut camera, &window, central);
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
        // In a journey, the hint at the keys sits in the panel's header.
        if explorer.browsing() {
            key_hint(ctx, graph_area);
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
            let tethers = journey_panel::draw(ctx, area, hint(false), &mut explorer, &mut panel);
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

/// Which mode the view is in, always in the same place: `Browsing`, or `Journey ·`
/// and where it starts.
fn mode_label(ui: &mut egui::Ui, explorer: &Explorer) {
    let (text, hint) = match explorer.start_unit() {
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
            .filter(|e| explorer.map().tile_of(&e.unit).is_some())
            .map(|e| (e.unit.clone(), e.label.clone(), e.kind.clone()))
            .collect();
        rows.sort_by(|a, b| a.1.cmp(&b.1));
        rows
    } else {
        let entries = system
            .entries()
            .iter()
            .filter(|e| explorer.map().tile_of(&e.unit).is_some())
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
                                        .selectable_label(explorer.start_unit() == Some(id), title)
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
            Some(tile) if explorer.browsing() => explorer.click(tile, None),
            Some(tile) => explorer.open_at(tile),
            None => {}
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

/// With a function set aside, a chip that says so; a click on it, as Enter does,
/// starts a journey there.
fn aside_chip(ui: &mut egui::Ui, explorer: &mut Explorer) {
    if explorer.aside().is_none() {
        return;
    }
    let chip = egui::Button::new(
        egui::RichText::new("not on this journey · Enter starts one here")
            .size(text::BODY)
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
    use super::*;

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
}
