//! Every text size, defined once, each for one kind of text wherever it is drawn:
//! small print (a name's note, a module's count, badges, hints, the key list's
//! lines), body text (a function's name, a journey bar, the bars along the top), and
//! a title (a module's name, the journey's name, the key list's headings). Nothing
//! is drawn smaller than [`SMALL`].

use bevy_egui::egui::{self, FontFamily, FontId, TextStyle};

/// Notes, counts, badges and hints.
pub(crate) const SMALL: f32 = 11.0;
/// A function's name, on the graph and on its bar, and the controls.
pub(crate) const BODY: f32 = 12.0;
/// A module's name, and headings.
pub(crate) const TITLE: f32 = 13.0;

/// egui's own text styles at these sizes, so what egui lays out keeps to them too.
pub(crate) fn styles(style: &mut egui::Style) {
    style.text_styles = [
        (
            TextStyle::Small,
            FontId::new(SMALL, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(BODY, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(BODY, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(SMALL, FontFamily::Monospace),
        ),
        (
            TextStyle::Heading,
            FontId::new(TITLE, FontFamily::Proportional),
        ),
    ]
    .into();
}
