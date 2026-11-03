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

/// A character's width against its font size, at about the widest a name runs: the
/// one guess at how wide a name shows before it is laid out, wherever that is guessed.
pub(crate) const CHAR_WIDTH: f32 = 0.55;

/// How wide a name of `chars` characters shows at font `size`, its text alone, in
/// points, guessed before it is laid out.
pub(crate) fn name_width(chars: usize, size: f32) -> f32 {
    #[expect(clippy::cast_precision_loss, reason = "a name's length")]
    let chars = chars as f32;
    chars * size * CHAR_WIDTH
}

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
