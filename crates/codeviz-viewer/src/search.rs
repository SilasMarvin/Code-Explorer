//! Finding things by what they are called or what they say: a function by its name or
//! path, an entry point by its route, and inside functions, where they end with an
//! error or a panic, the conditions they branch on, and the calls they make into code
//! outside the analysis. Typing an error message finds where it is raised; picking it
//! opens that function on the way to the error, with every route into it.

use bevy::prelude::*;
use bevy_egui::egui;
use codeviz_core::System;
use codeviz_ir::{ExitKind, Role, Step, UnitId};

use crate::{
    explorer::{Explorer, Side},
    ui::{MUTED, short},
};

/// What a search finds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Found {
    /// A function, or an entry point by its route.
    Unit,
    /// Where a function ends: an error, a panic, or a return with a name.
    Exit(ExitKind),
    /// A condition a function branches on, or one of its arms.
    Condition,
    /// A call into code outside the analysis, as written: std, a dependency.
    Call,
}

impl Found {
    /// Which comes first at an equal match: functions, then what they say.
    fn order(self) -> u8 {
        match self {
            Self::Unit => 0,
            Self::Exit(_) => 1,
            Self::Condition => 2,
            Self::Call => 3,
        }
    }
}

/// One thing a search can find, in the function it is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Item {
    pub(crate) found: Found,
    pub(crate) unit: UnitId,
    /// What it says: a name, an error message, a condition, a call.
    pub(crate) text: String,
    /// What else to show of it: a function's kind, or the guard an arm belongs to.
    pub(crate) note: String,
    /// `text` as a query matches it.
    normal: String,
    /// Whether it is an entry point, named by its route.
    entry: bool,
}

impl Item {
    fn new(found: Found, unit: &UnitId, text: String, note: String) -> Self {
        Self {
            found,
            unit: unit.clone(),
            normal: normal(&text),
            text,
            note,
            entry: false,
        }
    }

    fn entry(entry: &codeviz_ir::Entry) -> Self {
        Self {
            entry: true,
            ..Self::new(
                Found::Unit,
                &entry.unit,
                entry.label.clone(),
                entry.kind.clone(),
            )
        }
    }
}

/// Arm labels that say nothing on their own.
const PLAIN_ARMS: [&str; 9] = [
    "then", "else", "yes", "no", "ok", "true", "false", "some", "none",
];

/// Everything a search can find in a system, worked out once.
#[derive(Debug, Default)]
pub(crate) struct Index {
    items: Vec<Item>,
}

impl Index {
    /// Indexes every entry and function of `system`, named by `name`, and what their
    /// bodies say.
    pub(crate) fn of(system: &System, name: impl Fn(&UnitId) -> String) -> Self {
        let mut items: Vec<Item> = system.entries().iter().map(Item::entry).collect();
        for unit in system.units().filter(|u| u.role == Role::Invocable) {
            items.push(Item::new(
                Found::Unit,
                &unit.id,
                name(&unit.id),
                unit.kind.clone(),
            ));
            if let Some(body) = &unit.body {
                said(body, &unit.id, &mut items);
            }
            for call in unit.calls.iter().filter(|c| c.targets.is_empty()) {
                items.push(Item::new(
                    Found::Call,
                    &unit.id,
                    call.text.clone(),
                    String::new(),
                ));
            }
        }
        // Each once: a function that raises the same error twice is one place.
        let mut seen = std::collections::HashSet::new();
        items.retain(|item| seen.insert((item.found, item.unit.clone(), item.text.clone())));
        Self { items }
    }

    /// What `query` finds, best first, at most `limit` of it: functions before what
    /// they say at an equal match, shorter before longer.
    pub(crate) fn find(&self, query: &str, limit: usize) -> Vec<&Item> {
        let query = normal(query.trim());
        if query.is_empty() {
            return Vec::new();
        }
        let mut ranked: Vec<(u8, &Item)> = self
            .items
            .iter()
            .filter_map(|item| Some((rank(&query, item)?, item)))
            .collect();
        ranked.sort_by(|a, b| {
            let key = |(rank, item): &(u8, &Item)| {
                (*rank, item.found.order(), !item.entry, item.text.len())
            };
            key(a).cmp(&key(b)).then_with(|| a.1.text.cmp(&b.1.text))
        });
        let mut seen = std::collections::HashSet::new();
        ranked
            .into_iter()
            .map(|(_, item)| item)
            // A function found as an entry and as itself shows once, by its route.
            .filter(|item| item.found != Found::Unit || seen.insert(item.unit.clone()))
            .take(limit)
            .collect()
    }
}

/// What a body says that a search can find: its exits' labels, its guards, and arm
/// labels that say something.
fn said(body: &[Step], unit: &UnitId, items: &mut Vec<Item>) {
    for step in body {
        match step {
            Step::Exit {
                kind,
                label: Some(label),
            } => items.push(Item::new(
                Found::Exit(*kind),
                unit,
                label.clone(),
                String::new(),
            )),
            Step::Branch { guard, arms } => {
                if let Some(guard) = guard {
                    items.push(Item::new(
                        Found::Condition,
                        unit,
                        guard.clone(),
                        String::new(),
                    ));
                }
                for arm in arms {
                    if !PLAIN_ARMS.contains(&arm.label.to_lowercase().as_str()) {
                        let note = guard.clone().unwrap_or_default();
                        items.push(Item::new(Found::Condition, unit, arm.label.clone(), note));
                    }
                    said(&arm.body, unit, items);
                }
            }
            Step::Loop { body, .. } => said(body, unit, items),
            Step::Dag { nodes } => {
                for node in nodes {
                    said(&node.body, unit, items);
                }
            }
            _ => {}
        }
    }
}

/// Lowercase, with `::` read as `.`, so `orders.place` finds `orders::place`.
fn normal(text: &str) -> String {
    text.to_lowercase().replace("::", ".")
}

/// An id without its producer's prefix, as in `rust:`.
fn path(id: &str) -> &str {
    id.split_once(':').map_or(id, |(_, path)| path)
}

/// How well `query`, already normal, finds `item`, best first, or `None` when it does
/// not.
fn rank(query: &str, item: &Item) -> Option<u8> {
    match item.found {
        Found::Unit => search_rank(query, item.unit.as_str(), &item.text),
        _ => text_rank(query, &item.normal),
    }
}

/// How well `query` finds what a function says: all of it, its start or a word's,
/// anywhere in it, then each word of the query in order.
fn text_rank(query: &str, text: &str) -> Option<u8> {
    if text == query {
        return Some(0);
    }
    let word_start = text
        .match_indices(query)
        .any(|(at, _)| at == 0 || !text[..at].ends_with(char::is_alphanumeric));
    if word_start {
        return Some(1);
    }
    if text.contains(query) {
        return Some(2);
    }
    in_order(query, text).then_some(3)
}

/// Whether each word of `query` turns up in `text`, in order.
fn in_order(query: &str, text: &str) -> bool {
    let mut rest = text;
    for word in query
        .split(|c: char| c == '.' || c.is_whitespace())
        .filter(|w| !w.is_empty())
    {
        let Some(at) = rest.find(word) else {
            return false;
        };
        rest = &rest[at + word.len()..];
    }
    true
}

/// How well `query` finds a function, best first, or `None` when it does not: its
/// name exactly, then its name's start, then anywhere in its path, then each word of
/// the query in order along the path. `.` and `::` are the same, so `orders.place`
/// finds `services::orders::place`.
pub(crate) fn search_rank(query: &str, id: &str, name: &str) -> Option<u8> {
    let query = normal(query.trim());
    let name = normal(name);
    let path = normal(path(id));
    if name == query || path == query || path.ends_with(&format!(".{query}")) {
        return Some(0);
    }
    if name.starts_with(&query) || name.contains(&format!(".{query}")) {
        return Some(1);
    }
    if path.contains(&query) || name.contains(&query) {
        return Some(2);
    }
    in_order(&query, &path).then_some(3)
}

/// The search box's text, and the index it searches, built when first opened.
#[derive(Resource, Debug, Default)]
pub(crate) struct Search {
    query: String,
    index: Option<Index>,
}

impl Search {
    pub(crate) fn new(query: String) -> Self {
        Self { query, index: None }
    }
}

const ERROR: egui::Color32 = egui::Color32::from_rgb(240, 110, 100);

/// A box to find a function, or what one says, over the top of the graph: Enter or a
/// click opens it, Escape closes the box.
pub(crate) fn search_box(
    ctx: &egui::Context,
    area: egui::Rect,
    explorer: &mut Explorer,
    search: &mut Search,
    open: &mut bool,
    now: f32,
) {
    let index = search
        .index
        .get_or_insert_with(|| Index::of(explorer.system(), |id| explorer.unit_name(id)));
    let rows: Vec<Item> = if search.query.trim().is_empty() {
        let mut rows: Vec<Item> = explorer
            .system()
            .entries()
            .iter()
            .map(Item::entry)
            .collect();
        rows.sort_by(|a, b| a.text.cmp(&b.text));
        rows
    } else {
        index
            .find(&search.query, 200)
            .into_iter()
            .cloned()
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
                    ui.set_width(460.0);
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut search.query)
                            .hint_text("search functions, errors, conditions, calls…")
                            .desired_width(f32::INFINITY),
                    );
                    field.request_focus();
                    if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        chosen = rows.first().cloned();
                    }
                    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                        *open = false;
                    }
                    if rows.is_empty() {
                        ui.label(egui::RichText::new("nothing matches").color(MUTED));
                    }
                    egui::ScrollArea::vertical()
                        .id_salt("units")
                        .max_height(300.0)
                        .show(ui, |ui| {
                            for item in &rows {
                                if row(ui, explorer, item) {
                                    chosen = Some(item.clone());
                                }
                            }
                        });
                });
        });
    if let Some(item) = chosen {
        pick(explorer, &item, now);
        *open = false;
    }
}

/// One found thing: what it says, then where it is. Returns whether it was clicked.
fn row(ui: &mut egui::Ui, explorer: &Explorer, item: &Item) -> bool {
    let (mark, color, what) = match item.found {
        Found::Unit => ("", egui::Color32::WHITE, None),
        Found::Exit(ExitKind::Return) => ("✔ ", egui::Color32::WHITE, Some("returns")),
        Found::Exit(ExitKind::Panic) => ("× ", ERROR, Some("panics")),
        Found::Exit(_) => ("× ", ERROR, Some("fails")),
        Found::Condition => ("? ", egui::Color32::WHITE, Some("branches")),
        Found::Call => ("→ ", egui::Color32::WHITE, Some("calls")),
    };
    let mut clicked = false;
    ui.horizontal(|ui| {
        let title = egui::RichText::new(format!("{mark}{}", short(&item.text, 56))).color(color);
        let current = item.found == Found::Unit && explorer.root() == Some(&item.unit);
        clicked = ui
            .selectable_label(current, title)
            .on_hover_text(&item.text)
            .clicked();
        let note = match what {
            None => item.note.clone(),
            Some(what) => format!("{what} in {}", explorer.unit_name(&item.unit)),
        };
        ui.label(egui::RichText::new(note).small().color(MUTED));
    });
    clicked
}

/// Opens what a search found: a function as it is; what a function says opens that
/// function, on the outcome it ends in when it is an exit, with every route into it.
pub(crate) fn pick(explorer: &mut Explorer, item: &Item, now: f32) {
    let Some(tile) = explorer.map().tile_of(&item.unit) else {
        explorer.set_root(item.unit.clone());
        return;
    };
    explorer.open_at(tile);
    if item.found == Found::Unit {
        return;
    }
    if let Found::Exit(_) = item.found {
        let outcome = explorer.scenarios().iter().position(|scenario| {
            scenario.outcome.label == item.text
                && scenario
                    .chain
                    .last()
                    .is_some_and(|site| site.unit == item.unit)
        });
        if outcome.is_some() {
            explorer.set_scenario(outcome);
        }
    }
    explorer.spotlight_all(Side::Callers, now);
}

#[cfg(test)]
mod tests {
    use codeviz_core::JourneyOptions;
    use codeviz_ir::Document;

    use super::*;

    /// `POST /orders -> place -> quote`, where `quote` fails on an unknown sku and
    /// `place` calls `.ok_or`, which nothing resolves.
    fn system() -> System {
        let json = r#"{
          "schema": "codeviz/0.1", "producer": { "name": "t", "version": "0" },
          "entries": [ { "unit": "x:shop::post", "label": "POST /orders", "kind": "http" } ],
          "units": [
            { "id": "x:shop::post", "name": "post", "kind": "fn", "role": "invocable", "calls": [
              { "key": "place#0", "text": "place", "propagates": true,
                "targets": [{ "unit": "x:shop::orders::place", "certainty": "exact" }] } ] },
            { "id": "x:shop::orders::place", "name": "place", "kind": "fn", "role": "invocable",
              "calls": [
                { "key": ".ok_or#0", "text": ".ok_or", "targets": [] },
                { "key": "quote#0", "text": "quote", "propagates": true,
                  "targets": [{ "unit": "x:shop::pricing::quote", "certainty": "exact" }] } ] },
            { "id": "x:shop::pricing::quote", "name": "quote", "kind": "fn", "role": "invocable",
              "body": [
                { "step": "branch", "guard": "match sku", "arms": [
                  { "label": "\"widget\"", "body": [] },
                  { "label": "other", "body": [
                    { "step": "exit", "kind": "error", "label": "AppError::Invalid(\"unknown sku\")" } ] }
                ] },
                { "step": "branch", "guard": "quantity > 10", "arms": [
                  { "label": "then", "body": [] }, { "label": "else", "body": [] } ] }
              ] }
          ]
        }"#;
        System::new([Document::from_json(json).unwrap()])
    }

    fn index() -> Index {
        Index::of(&system(), |id| path(id.as_str()).to_owned())
    }

    fn found(index: &Index, query: &str) -> Vec<(Found, String)> {
        index
            .find(query, 10)
            .into_iter()
            .map(|item| (item.found, item.text.clone()))
            .collect()
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
    fn finds_where_an_error_is_raised() {
        let index = index();
        assert_eq!(
            found(&index, "unknown sku"),
            [(
                Found::Exit(ExitKind::Error),
                "AppError::Invalid(\"unknown sku\")".to_owned()
            )]
        );
        assert_eq!(
            found(&index, "invalid")[0].0,
            Found::Exit(ExitKind::Error),
            "by the error's variant"
        );
        let item = &index.find("unknown sku", 1)[0];
        assert_eq!(item.unit.as_str(), "x:shop::pricing::quote");
    }

    #[test]
    fn finds_conditions_and_calls_outside() {
        let index = index();
        assert_eq!(
            found(&index, "quantity"),
            [(Found::Condition, "quantity > 10".to_owned())]
        );
        assert!(
            found(&index, "widget").contains(&(Found::Condition, "\"widget\"".to_owned())),
            "an arm that says something"
        );
        assert!(
            found(&index, "then").is_empty(),
            "but not one that does not"
        );
        assert_eq!(found(&index, "ok_or"), [(Found::Call, ".ok_or".to_owned())]);
        assert_eq!(
            found(&index, "quote")[0],
            (Found::Unit, "shop::pricing::quote".to_owned()),
            "a function before a call to it"
        );
        assert_eq!(
            found(&index, "post")[0],
            (Found::Unit, "POST /orders".to_owned()),
            "an entry by its route, once"
        );
        assert_eq!(
            found(&index, "post")
                .iter()
                .filter(|(f, _)| *f == Found::Unit)
                .count(),
            1
        );
    }

    #[test]
    fn picking_an_error_opens_its_function_on_it_with_every_route_in() {
        let mut explorer = Explorer::new(system(), None, JourneyOptions::default());
        let index = Index::of(explorer.system(), |id| explorer.unit_name(id));
        let item = index.find("unknown sku", 1)[0].clone();
        pick(&mut explorer, &item, 0.0);
        assert_eq!(
            explorer.root().map(UnitId::as_str),
            Some("x:shop::pricing::quote")
        );
        let outcome = explorer
            .scenario()
            .and_then(|s| explorer.scenarios().get(s))
            .map(|s| s.outcome.label.clone());
        assert_eq!(outcome.as_deref(), Some(item.text.as_str()));
        assert_eq!(
            explorer.spotlight().map(|s| (s.side, s.reach)),
            Some((Side::Callers, None))
        );
        let quote = explorer.map().tile_of(&item.unit).unwrap();
        let starts: Vec<String> = explorer
            .starts(quote)
            .into_iter()
            .map(|s| s.label)
            .collect();
        assert_eq!(starts, ["POST /orders"]);
    }
}
