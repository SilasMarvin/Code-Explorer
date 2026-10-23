//! The view as a command line. Y copies the `codeviz` command that opens what is on
//! screen, for a teammate or a note; the same flags are kept as the last view of the
//! documents, which `--resume` opens again.

use std::{borrow::Cow, path::PathBuf};

use bevy::prelude::*;
use bevy_egui::input::egui_wants_any_keyboard_input;
use codeviz_core::{Level, System};
use codeviz_ir::UnitId;

use crate::{
    actions::{Effects, Request, Requests},
    diff_view::{DiffStyle, DiffView},
    disclosure::Disclose,
    explorer::{Clustering, Explorer, Side},
    journey_panel::{JourneyPanel, PanelMode},
    lens::{Measure, Source},
    peek::Peek,
};

pub(crate) struct SharePlugin;

impl Plugin for SharePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Share>().add_systems(
            Update,
            (
                copy_view.run_if(not(egui_wants_any_keyboard_input)),
                keep_view,
            ),
        );
    }
}

/// How the documents on screen were opened, and where their last view is kept.
#[derive(Resource, Debug, Default)]
pub(crate) struct Share {
    /// The command that opens the same documents, ahead of the view's own flags.
    pub(crate) command: Vec<String>,
    pub(crate) state: Option<PathBuf>,
}

/// What a view is, for its flags.
pub(crate) struct View<'a> {
    pub(crate) explorer: &'a Explorer,
    pub(crate) panel: PanelMode,
    pub(crate) diff: Option<&'a DiffView>,
    pub(crate) peek: bool,
}

/// The `codeviz view` flags that open `view` as it stands, leaving out what opening
/// does anyway. Zooms toggled one function at a time, all-outcomes-at-once and
/// disclosing by time have no flags, and open as the defaults.
pub(crate) fn flags(view: &View<'_>) -> Vec<String> {
    let View {
        explorer,
        panel,
        diff,
        peek,
    } = *view;
    let system = explorer.system();
    let mut flags: Vec<String> = Vec::new();
    let mut flag = |name: &str, value: Option<String>| {
        flags.push(format!("--{name}"));
        flags.extend(value);
    };
    // Where it stands: the focus, else the journey's root with its cursor.
    let focused = explorer
        .focused_by_hand()
        .and_then(|tile| explorer.tile_unit(tile))
        .filter(|unit| system.unit(unit).is_some());
    let place = focused.or_else(|| explorer.root());
    if let Some(unit) = place {
        flag("focus", Some(short_id(system, unit)));
    }
    if let Some(scenario) = explorer.scenario().filter(|&s| s > 0) {
        flag("scenario", Some(scenario.to_string()));
    }
    let step = explorer.selected().map_or(0, codeviz_core::SpanId::index);
    if focused.is_none() && place.is_some() && step > 0 {
        flag("step", Some(step.to_string()));
    }
    if let Disclose::Depth(depth) = explorer.disclose()
        && depth != 1
    {
        flag("depth", Some(depth.to_string()));
    }
    if let Some((side, _)) = explorer
        .spotlight()
        .filter(|_| !explorer.spotlit().is_empty())
    {
        let side = match side {
            Side::Callers => "callers",
            Side::Callees => "callees",
        };
        flag("spotlight", Some(side.to_owned()));
    }
    for (index, track) in explorer.tracks().iter().enumerate() {
        if let Some(root) = track.root().filter(|_| index != explorer.active()) {
            flag("also", Some(short_id(system, root)));
        }
    }
    match panel {
        PanelMode::Docked => {}
        PanelMode::Hidden => flag("journey", Some("hidden".to_owned())),
        PanelMode::Full => flag("journey", Some("full".to_owned())),
    }
    let lens = explorer.lens();
    if let Some(reach) = lens.reach {
        flag("reveal", Some(reach.to_string()));
    }
    let usual = if diff.is_some() {
        Source::Changes
    } else {
        Source::Focus
    };
    if lens.source != usual {
        flag("reveal-around", Some(lens.source.label().to_owned()));
    }
    match lens.measure {
        Measure::Calls => {}
        Measure::Space => flag("reveal-by", Some("space".to_owned())),
        Measure::Blend => flag("reveal-by", Some("blend".to_owned())),
    }
    if let Some(diff) = diff {
        if !lens.changes_only {
            flag("show-unchanged", None);
        }
        match diff.style {
            DiffStyle::Status => {}
            DiffStyle::Impact => flag("diff-style", Some("impact".to_owned())),
            DiffStyle::Morph => flag("diff-style", Some("morph".to_owned())),
        }
    }
    if explorer.clustering() == Clustering::Calls {
        flag("cluster", Some("calls".to_owned()));
    }
    if explorer.options.include_unresolved {
        flag("unresolved", None);
    }
    if explorer.options.zoom.default == Level::Detail {
        flag("zoom", Some("detail".to_owned()));
    }
    if peek {
        flag("peek", None);
    }
    flags
}

/// The shortest tail of `unit`'s id, at a `::`, `.` or `/`, that names it and nothing
/// else.
pub(crate) fn short_id(system: &System, unit: &UnitId) -> String {
    let id = unit.as_str();
    let separator = |c: char| matches!(c, ':' | '.' | '/');
    let starts = id
        .char_indices()
        .skip(1)
        .filter(|&(at, c)| !separator(c) && id[..at].ends_with(separator))
        .map(|(at, _)| at);
    let mut starts: Vec<usize> = starts.collect();
    starts.reverse();
    starts
        .into_iter()
        .map(|at| &id[at..])
        .find(|tail| matches!(system.find(tail).as_slice(), [only] if &only.id == unit))
        .unwrap_or(id)
        .to_owned()
}

/// `words` as one line a shell reads back as the same words.
pub(crate) fn command_line(words: &[String]) -> String {
    words.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ")
}

fn quote(word: &str) -> Cow<'_, str> {
    let plain = |c: char| c.is_ascii_alphanumeric() || "-_./:=,+@%".contains(c);
    if !word.is_empty() && word.chars().all(plain) {
        Cow::Borrowed(word)
    } else {
        Cow::Owned(format!("'{}'", word.replace('\'', r"'\''")))
    }
}

/// Y copies the command that opens the view.
fn copy_view(
    keys: Res<ButtonInput<KeyCode>>,
    share: Res<Share>,
    explorer: Res<Explorer>,
    panel: Res<JourneyPanel>,
    diff: Option<Res<DiffView>>,
    peek: Res<Peek>,
    mut requests: ResMut<Requests>,
) {
    if !keys.just_pressed(KeyCode::KeyY) {
        return;
    }
    let mut words = share.command.clone();
    words.extend(flags(&View {
        explorer: &explorer,
        panel: panel.mode,
        diff: diff.as_deref(),
        peek: peek.0,
    }));
    let line = command_line(&words);
    requests.push(Request::Copy {
        what: line.clone(),
        text: line,
    });
}

/// How often the last view is checked for a change worth keeping.
const KEEP_EVERY: f32 = 1.0;

/// Keeps the view's flags as the documents' last view, when they change.
#[expect(clippy::too_many_arguments, reason = "a Bevy system")]
fn keep_view(
    share: Res<Share>,
    effects: Res<Effects>,
    explorer: Res<Explorer>,
    panel: Res<JourneyPanel>,
    diff: Option<Res<DiffView>>,
    peek: Res<Peek>,
    time: Res<Time>,
    mut kept: Local<(f32, Vec<String>)>,
) {
    let Some(path) = share.state.as_ref().filter(|_| *effects == Effects::Live) else {
        return;
    };
    let now = time.elapsed_secs();
    if now - kept.0 < KEEP_EVERY {
        return;
    }
    kept.0 = now;
    let flags = flags(&View {
        explorer: &explorer,
        panel: panel.mode,
        diff: diff.as_deref(),
        peek: peek.0,
    });
    if flags == kept.1 {
        return;
    }
    let json = serde_json::json!({ "command": share.command, "flags": flags });
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(path, format!("{json:#}\n")));
    match written {
        Ok(()) => kept.1 = flags,
        Err(error) => warn!("cannot keep the view in {}: {error}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{explorer::tests::explorer, lens::Lens};

    #[test]
    fn quotes_only_what_a_shell_would_split() {
        let words: Vec<String> = [
            "codeviz",
            "view",
            "my app.json",
            "--focus",
            "rust:shop::{closure#0}",
            "it's",
        ]
        .map(str::to_owned)
        .into();
        assert_eq!(
            command_line(&words),
            r"codeviz view 'my app.json' --focus 'rust:shop::{closure#0}' 'it'\''s'"
        );
    }

    #[test]
    fn a_view_reads_back_as_its_flags() {
        let mut explorer = explorer();
        let view = |explorer: &Explorer, panel| {
            flags(&View {
                explorer,
                panel,
                diff: None,
                peek: false,
            })
            .join(" ")
        };
        let root = explorer.root().cloned();
        assert_eq!(
            view(&explorer, PanelMode::Docked),
            format!(
                "--focus {}",
                short_id(
                    explorer.system(),
                    &root.unwrap_or_else(|| panic!("no root"))
                )
            )
        );
        explorer.step(crate::explorer::Step::Into);
        explorer.step(crate::explorer::Step::Into);
        explorer.set_disclose(Disclose::Depth(3));
        explorer.set_lens(Lens {
            reach: Some(2),
            measure: Measure::Blend,
            ..explorer.lens()
        });
        let flags = view(&explorer, PanelMode::Full);
        for expected in [
            "--step 2",
            "--depth 3",
            "--journey full",
            "--reveal 2",
            "--reveal-by blend",
        ] {
            assert!(flags.contains(expected), "{expected} in {flags}");
        }
        assert!(!flags.contains("--reveal-around"), "the usual start");
    }

    #[test]
    fn names_a_unit_by_its_shortest_unique_tail() {
        let explorer = explorer();
        let system = explorer.system();
        for unit in system.units() {
            let short = short_id(system, &unit.id);
            assert!(
                matches!(system.find(&short).as_slice(), [only] if only.id == unit.id),
                "{short} names {}",
                unit.id
            );
            assert!(short.len() <= unit.id.as_str().len());
        }
    }
}
