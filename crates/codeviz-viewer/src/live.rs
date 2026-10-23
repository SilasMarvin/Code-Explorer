//! A live view: whatever produces the documents keeps sending them as the code
//! changes, and the graph takes each version in where it stands. What just changed
//! pulses, a note says what it was, and a version that cannot be read leaves the last
//! good one showing with a quiet note saying why. `B` draws everything changed since
//! the viewer opened as a diff, the version it opened on standing in for `--before`.

use std::{
    collections::HashMap,
    sync::{
        Mutex, PoisonError,
        mpsc::{Receiver, TryRecvError},
    },
};

use bevy::prelude::*;
use bevy_egui::{egui, input::egui_wants_any_keyboard_input};
use codeviz_core::{Change, Diff, System, align, union};
use codeviz_ir::{Document, UnitId};

use crate::{
    diff_view::{self, DiffStyle, DiffView},
    explorer::Explorer,
    graph_view, scene,
    ui::{MUTED, egui_color},
};

/// News from whatever watches the code a live viewer shows.
#[derive(Debug)]
pub enum Update {
    /// Something changed and is being read again.
    Reading,
    /// The documents as they are now.
    Documents(Vec<Document>),
    /// What changed could not be read; the viewer keeps showing the last good version.
    Failed(String),
}

/// Seconds a changed function pulses for.
const PULSE: f32 = 2.4;
/// Seconds the note on what changed shows for.
const NOTE: f32 = 5.0;
/// At most this many changed functions are named in the note; past it, counted.
const NAMED: usize = 3;

/// Where a live viewer stands: what it shows, what it compares with, and what the
/// watcher last said.
#[derive(Resource)]
pub(crate) struct Live {
    updates: Mutex<Receiver<Update>>,
    /// The documents as the viewer opened on them.
    opened: Vec<Document>,
    /// The documents showing now.
    current: Vec<Document>,
    /// An older version given to diff against, which every version is drawn over.
    before: Option<Vec<Document>>,
    /// Whether, without one, the diff is against the version the viewer opened on.
    pub(crate) since_opened: bool,
    pub(crate) status: Status,
    /// What the last version taken in changed, and when.
    pub(crate) last: Option<(Note, f32)>,
    /// Functions the last versions changed, how, and when.
    pulses: HashMap<UnitId, (Change, f32)>,
    /// Whether the view must be rebuilt though no new version came.
    stale: bool,
    /// How many versions the watcher has sent, read or not.
    heard: u64,
}

/// What the watcher last said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Status {
    Watching,
    Reading,
    Failed(String),
}

/// What one version changed, for the note.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Note {
    /// Changed functions by name, with how, in name order.
    pub(crate) changed: Vec<(String, Change)>,
}

impl Note {
    fn of(diff: &Diff, name: impl Fn(&UnitId) -> String) -> Self {
        let mut changed: Vec<(String, Change)> = diff
            .units
            .iter()
            .map(|(unit, &change)| (name(unit), change))
            .collect();
        changed.sort();
        Self { changed }
    }

    /// `updated · quote rewritten, tax added`, or past a few, by how many each way.
    pub(crate) fn text(&self) -> String {
        if self.changed.is_empty() {
            return "updated · no function changed".to_owned();
        }
        if self.changed.len() <= NAMED {
            let named: Vec<String> = self
                .changed
                .iter()
                .map(|(name, change)| format!("{name} {}", past(*change)))
                .collect();
            return format!("updated · {}", named.join(", "));
        }
        let counts: Vec<String> = [
            Change::Added,
            Change::Modified,
            Change::Moved,
            Change::Removed,
        ]
        .into_iter()
        .filter_map(|change| {
            let n = self.changed.iter().filter(|(_, c)| *c == change).count();
            (n > 0).then(|| format!("{n} {}", past(change)))
        })
        .collect();
        format!(
            "updated · {} functions: {}",
            self.changed.len(),
            counts.join(", ")
        )
    }
}

fn past(change: Change) -> &'static str {
    match change {
        Change::Added => "added",
        Change::Modified => "rewritten",
        Change::Moved => "moved",
        Change::Removed => "removed",
    }
}

impl Live {
    pub(crate) fn new(
        opened: Vec<Document>,
        before: Option<Vec<Document>>,
        updates: Receiver<Update>,
    ) -> Self {
        Self {
            updates: Mutex::new(updates),
            current: opened.clone(),
            opened,
            before,
            since_opened: false,
            status: Status::Watching,
            last: None,
            pulses: HashMap::new(),
            stale: false,
            heard: 0,
        }
    }

    /// How many versions the watcher has sent, read or not.
    pub(crate) fn heard(&self) -> u64 {
        self.heard
    }

    /// Whether the diff is against a version given from the start, not the one opened.
    pub(crate) fn fixed_before(&self) -> bool {
        self.before.is_some()
    }

    /// Shows the changes since the viewer opened, or stops.
    pub(crate) fn toggle_since_opened(&mut self) {
        if self.before.is_none() {
            self.since_opened = !self.since_opened;
            self.stale = true;
        }
    }

    /// What to diff against, if anything.
    fn baseline(&self) -> Option<&[Document]> {
        match &self.before {
            Some(before) => Some(before),
            None if self.since_opened => Some(&self.opened),
            None => None,
        }
    }

    /// Takes every update waiting. Returns the newest documents among them.
    fn receive(&mut self) -> Option<Vec<Document>> {
        let mut newest = None;
        let updates = self
            .updates
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        loop {
            match updates.try_recv() {
                Ok(Update::Reading) => {
                    if !matches!(self.status, Status::Failed(_)) {
                        self.status = Status::Reading;
                    }
                }
                Ok(Update::Failed(why)) => {
                    self.heard += 1;
                    self.status = Status::Failed(why);
                }
                Ok(Update::Documents(documents)) => {
                    self.heard += 1;
                    self.status = Status::Watching;
                    newest = Some(documents);
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => return newest,
            }
        }
    }

    /// How a function pulses `now`: its change and how far through the pulse it is,
    /// from 0 to 1.
    fn pulse(&self, unit: &UnitId, now: f32) -> Option<(Change, f32)> {
        let &(change, since) = self.pulses.get(unit)?;
        let age = (now - since) / PULSE;
        (0.0..1.0).contains(&age).then_some((change, age))
    }
}

pub(crate) struct LivePlugin;

impl Plugin for LivePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                live_keys.run_if(not(egui_wants_any_keyboard_input)),
                take_updates,
            )
                .chain()
                .before(scene::stop_on_rebuild)
                .run_if(resource_exists::<Live>),
        )
        .add_systems(
            Update,
            draw_pulses
                .after(graph_view::draw_graph)
                .run_if(resource_exists::<Live>),
        );
    }
}

/// B shows what changed since the viewer opened, or stops.
fn live_keys(keys: Res<ButtonInput<KeyCode>>, mut live: ResMut<Live>) {
    if keys.just_pressed(KeyCode::KeyB) {
        live.toggle_since_opened();
    }
}

/// Takes in the newest version, if one came, or the diff switched on or off.
fn take_updates(
    mut commands: Commands,
    mut live: ResMut<Live>,
    mut explorer: ResMut<Explorer>,
    view: Option<ResMut<DiffView>>,
    time: Res<Time>,
) {
    let live = &mut *live;
    let newest = live.receive();
    let newest = newest.filter(|documents| *documents != live.current);
    if newest.is_none() && !live.stale {
        return;
    }
    live.stale = false;
    let now = time.elapsed_secs();
    let documents = newest.clone().unwrap_or_else(|| live.current.clone());
    let system = System::new(documents.clone());
    let (previous, renamed) = align(live.current.clone(), &system);
    if newest.is_some() {
        let changes = Diff::between(&System::new(previous), &system).with_moves(renamed.clone());
        let note = Note::of(&changes, |unit| {
            let name = codeviz_core::display_name(&system, unit);
            if system.unit(unit).is_some() {
                name
            } else {
                explorer.unit_name(unit)
            }
        });
        live.pulses.retain(|_, (_, since)| now - *since < PULSE);
        for (unit, &change) in &changes.units {
            if change != Change::Removed {
                live.pulses.insert(unit.clone(), (change, now));
            }
        }
        live.last = Some((note, now));
    }
    // A morph moves the nodes away from where the graph lays them out.
    if let Some(view) = view.as_deref().filter(|v| v.style == DiffStyle::Morph) {
        let shared = view.shared().to_vec();
        explorer.place_nodes(&shared);
    }
    let rebuilt = if let Some(baseline) = live.baseline() {
        let (before, moved) = align(baseline.to_vec(), &system);
        let both = System::new(union(&before, documents.clone()));
        explorer.reload(system, Some(both), &renamed);
        let diff = Diff::between(&System::new(before.clone()), explorer.system()).with_moves(moved);
        let mut rebuilt = DiffView::new(&mut explorer, &diff, System::new(before));
        if let Some(view) = view.as_deref() {
            rebuilt.carry_from(view);
        }
        explorer.set_changed_tiles(rebuilt.changed().map(|(tile, _)| tile).collect());
        Some(rebuilt)
    } else {
        explorer.reload(system, None, &renamed);
        None
    };
    live.current = documents;
    match (rebuilt, view) {
        (Some(rebuilt), Some(mut view)) => *view = rebuilt,
        (Some(rebuilt), None) => commands.insert_resource(rebuilt),
        (None, Some(_)) => commands.remove_resource::<DiffView>(),
        (None, None) => {}
    }
}

/// Rings ripple out from each function that just changed, in its change's color.
fn draw_pulses(explorer: Res<Explorer>, live: Res<Live>, time: Res<Time>, mut gizmos: Gizmos) {
    let now = time.elapsed_secs();
    for tile in 0..explorer.map().tiles.len() {
        let Some((change, age)) = explorer
            .tile_unit(tile)
            .and_then(|unit| live.pulse(unit, now))
        else {
            continue;
        };
        let Some(at) = explorer.tile_point(tile) else {
            continue;
        };
        let size = graph_view::node_size(explorer.graph(), tile) * 0.5;
        let color = diff_view::change_color(change);
        // Three ripples, each growing out and fading, the last of them slowest.
        for ripple in 0..3 {
            let phase = (age * 3.0 - ripple as f32 * 0.35).clamp(0.0, 1.0);
            if phase <= 0.0 || phase >= 1.0 {
                continue;
            }
            gizmos.sphere(
                Isometry3d::from_translation(at),
                size + 0.4 + phase * 2.6,
                color.with_alpha((1.0 - phase) * (1.0 - age) * 0.95),
            );
        }
        gizmos.sphere(
            Isometry3d::from_translation(at),
            size + 0.45,
            color.with_alpha(0.9 * (1.0 - age)),
        );
    }
}

/// A small note over the graph's top edge: live, reading, what the last version
/// changed, or why the newest could not be read. With no fixed version to compare
/// with, a switch for the diff since the viewer opened.
pub(crate) fn status_chip(ctx: &egui::Context, area: egui::Rect, live: &mut Live, now: f32) {
    let recent = live
        .last
        .as_ref()
        .filter(|(_, at)| now - at < NOTE)
        .map(|(note, at)| (note.text(), now - at));
    let mut toggled = false;
    egui::Area::new("live".into())
        .pivot(egui::Align2::CENTER_TOP)
        .fixed_pos(egui::pos2(area.center().x, area.min.y + 10.0))
        .order(egui::Order::Middle)
        .interactable(true)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_premultiplied(11, 13, 19, 215))
                .corner_radius(12.0)
                .inner_margin(egui::Margin::symmetric(10, 4))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        match (&live.status, &recent) {
                            (Status::Failed(why), _) => {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "⚠ {} · showing the last good version",
                                        crate::ui::short(why, 90)
                                    ))
                                    .size(12.5)
                                    .color(FAILED),
                                )
                                .on_hover_text(why.as_str());
                            }
                            (Status::Reading, _) => {
                                ui.label(egui::RichText::new("● reading…").size(12.5).color(MUTED));
                            }
                            (Status::Watching, Some((text, age))) => {
                                // Bright as it lands, settling to muted.
                                let fade = (age / NOTE).clamp(0.0, 1.0);
                                let color = egui_color(Color::WHITE).lerp_to_gamma(MUTED, fade);
                                ui.label(egui::RichText::new(text).size(12.5).color(color));
                            }
                            (Status::Watching, None) => {
                                ui.label(egui::RichText::new("● live").size(12.5).color(MUTED));
                            }
                        }
                        if !live.fixed_before() {
                            ui.separator();
                            toggled = ui
                                .selectable_label(
                                    live.since_opened,
                                    egui::RichText::new("since opened").size(12.5),
                                )
                                .on_hover_text("Draw what changed since the viewer opened (B)")
                                .clicked();
                        }
                    });
                });
        });
    if toggled {
        live.toggle_since_opened();
    }
}

/// Quiet: a note that the newest version is not showing, not an alarm.
const FAILED: egui::Color32 = egui::Color32::from_rgb(232, 150, 140);

#[cfg(test)]
mod tests {
    use super::*;

    fn note(changed: &[(&str, Change)]) -> Note {
        Note {
            changed: changed.iter().map(|(n, c)| ((*n).to_owned(), *c)).collect(),
        }
    }

    #[test]
    fn the_note_names_a_few_changes_and_counts_many() {
        assert_eq!(note(&[]).text(), "updated · no function changed");
        assert_eq!(
            note(&[("orders.place", Change::Modified), ("tax", Change::Added)]).text(),
            "updated · orders.place rewritten, tax added"
        );
        assert_eq!(
            note(&[
                ("a", Change::Modified),
                ("b", Change::Modified),
                ("c", Change::Added),
                ("d", Change::Removed),
            ])
            .text(),
            "updated · 4 functions: 1 added, 2 rewritten, 1 removed"
        );
    }

    #[test]
    fn a_pulse_runs_its_course_and_stops() {
        let (_, updates) = std::sync::mpsc::channel();
        let mut live = Live::new(Vec::new(), None, updates);
        let unit = UnitId::new("x:a");
        live.pulses.insert(unit.clone(), (Change::Added, 10.0));
        assert_eq!(live.pulse(&unit, 9.0), None, "not before it starts");
        assert_eq!(live.pulse(&unit, 10.0), Some((Change::Added, 0.0)));
        assert!(live.pulse(&unit, 10.0 + PULSE / 2.0).is_some());
        assert_eq!(live.pulse(&unit, 10.0 + PULSE + 0.01), None);
    }

    #[test]
    fn updates_leave_the_newest_documents_and_the_last_word() {
        let (send, updates) = std::sync::mpsc::channel();
        let mut live = Live::new(Vec::new(), None, updates);
        let document = |name: &str| {
            Document::from_json(&format!(
                r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "{name}", "version": "0" }}, "units": [] }}"#
            ))
            .unwrap()
        };
        send.send(Update::Reading).unwrap();
        send.send(Update::Documents(vec![document("first")]))
            .unwrap();
        send.send(Update::Documents(vec![document("second")]))
            .unwrap();
        send.send(Update::Failed("src/a.rs:3 does not parse".to_owned()))
            .unwrap();
        let newest = live.receive().unwrap();
        assert_eq!(newest[0].producer.name, "second");
        assert_eq!(
            live.status,
            Status::Failed("src/a.rs:3 does not parse".to_owned())
        );
        send.send(Update::Reading).unwrap();
        assert!(live.receive().is_none());
        assert!(
            matches!(live.status, Status::Failed(_)),
            "reading again does not hide why the last try failed"
        );
        assert!(!live.fixed_before());
        live.toggle_since_opened();
        assert!(live.since_opened && live.baseline().is_some());
    }
}
