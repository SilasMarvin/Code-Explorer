//! `W` writes up what the view shows, as Markdown on the clipboard, to paste into a
//! pull request, a design doc or a chat thread: the active journey as far as it is
//! disclosed, in the outcome chosen, numbered as on screen; under a diff, what the
//! diff changes first, and the journey's changed calls marked. Shift+W copies the
//! journey as a Mermaid sequence diagram instead. A screenshot run never touches the
//! clipboard: it logs the text and writes it beside the image instead.

use std::collections::HashMap;

use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui, input::egui_wants_any_keyboard_input};
use codeviz_core::{SpanId, Story};

use crate::{capture::Capture, diff_view::DiffView, explorer::Explorer, ui::MUTED};

/// Seconds the confirmation shows.
const TOAST_SECONDS: f32 = 2.5;

pub(crate) struct SharePlugin;

impl Plugin for SharePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Share>()
            .add_systems(Update, share_key.run_if(not(egui_wants_any_keyboard_input)))
            .add_systems(EguiPrimaryContextPass, deliver);
    }
}

/// Text waiting for the clipboard with what it is, and the confirmation showing.
#[derive(Resource, Debug, Default)]
struct Share {
    pending: Option<(String, &'static str)>,
    toast: Option<(String, f32)>,
}

fn share_key(
    keys: Res<ButtonInput<KeyCode>>,
    explorer: Res<Explorer>,
    diff: Option<Res<DiffView>>,
    mut share: ResMut<Share>,
) {
    if !keys.just_pressed(KeyCode::KeyW) {
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    share.pending = Some(if shift {
        (diagram(&explorer, diff.as_deref()), "Mermaid")
    } else {
        (write_up(&explorer, diff.as_deref()), "Markdown")
    });
}

/// Copies what is pending, or in a screenshot run logs it and writes it beside the
/// image; then confirms it on screen for a moment.
fn deliver(
    mut contexts: EguiContexts,
    mut share: ResMut<Share>,
    capture: Option<Res<Capture>>,
    time: Res<Time>,
) -> Result {
    let ctx = contexts.ctx_mut()?;
    let now = time.elapsed_secs();
    if let Some((text, kind)) = share.pending.take() {
        let lines = text.lines().count();
        let message = if let Some(capture) = capture {
            let path = capture.path().with_extension("copied.md");
            info!("would copy to the clipboard:\n{text}");
            if let Err(error) = std::fs::write(&path, &text) {
                error!("could not write {}: {error}", path.display());
            }
            format!("Would copy {lines} lines of {kind} (screenshot: written beside it)")
        } else {
            ctx.copy_text(text);
            format!("Copied {lines} lines of {kind}")
        };
        share.toast = Some((message, now));
    }
    let Some((message, since)) = &share.toast else {
        return Ok(());
    };
    let age = now - since;
    if age > TOAST_SECONDS {
        share.toast = None;
        return Ok(());
    }
    let fade = ((TOAST_SECONDS - age) / 0.4).clamp(0.0, 1.0);
    egui::Area::new("share_toast".into())
        .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 72.0))
        .order(egui::Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            ui.set_opacity(fade);
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(egui::RichText::new(message).color(egui::Color32::WHITE));
                ui.label(
                    egui::RichText::new("W: the view as Markdown · Shift W: as a diagram")
                        .color(MUTED),
                );
            });
        });
    ctx.request_repaint();
    Ok(())
}

/// The active journey as Markdown, as disclosed now; under a diff, what the diff
/// changes first, and the journey's changes marked.
pub(crate) fn write_up(explorer: &Explorer, diff: Option<&DiffView>) -> String {
    let changes = diff.map(|view| view.diff().markdown(view.old(), explorer.system()));
    let journey = story(explorer, diff).map(|story| story.markdown());
    match (changes, journey) {
        (Some(changes), Some(journey)) => format!("{changes}\n{journey}"),
        (Some(text), None) | (None, Some(text)) => text,
        (None, None) => "Nothing to tell: no journey is open.\n".to_owned(),
    }
}

/// The active journey as a Mermaid sequence diagram, fenced to paste as it is.
pub(crate) fn diagram(explorer: &Explorer, diff: Option<&DiffView>) -> String {
    story(explorer, diff).map_or_else(
        || "Nothing to draw: no journey is open.\n".to_owned(),
        |story| format!("```mermaid\n{}```\n", story.mermaid()),
    )
}

/// The active journey as the view tells it.
fn story(explorer: &Explorer, diff: Option<&DiffView>) -> Option<Story> {
    let journey = explorer.journey()?;
    let disclosed = explorer.disclosure();
    let numbers: HashMap<SpanId, String> = disclosed
        .numbered()
        .map(|(span, number)| (span, number.to_owned()))
        .collect();
    let story = Story::new(
        explorer.system(),
        journey,
        explorer.scenarios(),
        explorer.scenario(),
        &numbers,
    );
    Some(match diff {
        Some(view) => story.with_diff(view.diff(), view.old()),
        None => story,
    })
}

#[cfg(test)]
mod tests {
    use codeviz_core::{JourneyOptions, System, number_calls};
    use codeviz_ir::Document;

    use super::*;
    use crate::disclosure::Disclose;

    fn demo(name: &str) -> System {
        let path = format!("{}/../../demo/out/{name}.json", env!("CARGO_MANIFEST_DIR"));
        System::new([Document::from_json(&std::fs::read_to_string(path).unwrap()).unwrap()])
    }

    #[test]
    fn core_numbers_calls_as_the_view_discloses_them() {
        for name in ["axum-server", "python-service", "deploy-pipeline"] {
            let system = demo(name);
            let roots: Vec<_> = system.entries().iter().map(|e| e.unit.clone()).collect();
            let mut explorer = Explorer::new(system, None, JourneyOptions::default());
            for root in roots {
                let Some(tile) = explorer.map().tile_of(&root) else {
                    continue;
                };
                explorer.open_at(tile);
                for depth in 1..=4 {
                    explorer.set_disclose(Disclose::Depth(depth));
                    let shown: HashMap<SpanId, String> = explorer
                        .disclosure()
                        .numbered()
                        .map(|(span, number)| (span, number.to_owned()))
                        .collect();
                    let journey = explorer.journey().unwrap();
                    let core = number_calls(explorer.system(), journey, depth);
                    assert_eq!(core, shown, "{name} {root} at depth {depth}");
                }
            }
        }
    }

    #[test]
    fn writes_up_the_journey_as_disclosed() {
        let system = demo("axum-server");
        let mut explorer = Explorer::new(system, None, JourneyOptions::default());
        let place = codeviz_ir::UnitId::new("rust:axum_server::handlers::place_order");
        let tile = explorer.map().tile_of(&place).unwrap();
        explorer.open_at(tile);
        explorer.set_disclose(Disclose::Depth(2));
        let text = write_up(&explorer, None);
        assert!(
            text.starts_with(
                "`POST /users/{id}/orders` → `handlers.place_order`, when it completes:"
            ),
            "{text}"
        );
        assert!(text.contains("\n  - 1.3 `pricing.quote` — "), "{text}");
        assert!(!text.contains("1.3.1"), "depth 2 stops there:\n{text}");
        let diagram = diagram(&explorer, None);
        assert!(
            diagram.starts_with("```mermaid\nsequenceDiagram\n"),
            "{diagram}"
        );
        assert!(diagram.contains(": 1.3 quote"), "{diagram}");
    }
}
