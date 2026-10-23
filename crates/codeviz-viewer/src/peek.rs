//! The focused function's source in a card over the corner of the graph (B), so
//! reading it does not take opening the side panel; in a diff, its old and new
//! source as a line diff, open as soon as a node is clicked. Each line that makes one
//! of the journey's calls carries that call's number on a badge filled like its node,
//! as the graph and the flame graph show it.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use bevy::prelude::*;
use bevy_egui::egui;
use codeviz_core::{Change, System};
use codeviz_ir::UnitId;

use crate::{
    actions::{Place, Requests},
    diff_view::{self, DiffView},
    explorer::Explorer,
    graph_view,
    source_diff::{self, Line},
    ui::{self, MUTED},
};

/// Source files read so far; `None` when a file could not be read.
#[derive(Resource, Debug, Default)]
pub(crate) struct Sources(HashMap<PathBuf, Option<Vec<String>>>);

impl Sources {
    pub(crate) fn lines(&mut self, path: &Path) -> Option<&[String]> {
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

/// What reading source takes: the files read so far, and where to ask for one to
/// open in the editor or for something to be copied.
pub(crate) struct Code<'a> {
    pub(crate) sources: &'a mut Sources,
    pub(crate) requests: &'a mut Requests,
}

/// Whether the source peek is open.
#[derive(Resource, Debug, Default)]
pub(crate) struct Peek(pub(crate) bool);

/// One version of a function: its lines, the file line of the first, and where it is.
struct Body {
    lines: Vec<String>,
    first: usize,
    place: Place,
    uri: String,
}

/// `unit`'s source as `system` locates it.
fn body(system: &System, unit: &UnitId, sources: &mut Sources) -> Option<Body> {
    let location = system.unit(unit)?.location.as_ref()?;
    let range = location.range?;
    let place = Place::of(system, unit)?;
    let lines = sources.lines(&place.file)?;
    let (first, last) = (range.start_line as usize, range.end_line as usize);
    Some(Body {
        lines: lines
            .get(first.checked_sub(1)?..last.min(lines.len()))?
            .to_vec(),
        first,
        place,
        uri: location.uri.clone(),
    })
}

/// Draws the peek in the top left of the graph `area`, for the focused function.
pub(crate) fn draw(
    ctx: &egui::Context,
    area: egui::Rect,
    explorer: &Explorer,
    diff: Option<&DiffView>,
    code: &mut Code<'_>,
    open: &mut bool,
) {
    let Some((tile, unit)) = explorer
        .graph_focus()
        .and_then(|tile| Some((tile, explorer.tile_unit(tile)?.clone())))
    else {
        return;
    };
    let width = (area.width() * 0.36).clamp(320.0, 580.0);
    let height = (area.height() * 0.58).clamp(160.0, 520.0);
    egui::Area::new("peek".into())
        .pivot(egui::Align2::LEFT_TOP)
        .fixed_pos(area.min + egui::vec2(12.0, 10.0))
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(egui::Color32::from_rgba_premultiplied(11, 13, 19, 236))
                .stroke(egui::Stroke::new(1.0, egui::Color32::from_gray(48)))
                .corner_radius(8.0)
                .inner_margin(egui::Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_width(width);
                    let new = body(explorer.system(), &unit, code.sources);
                    let old = diff.and_then(|d| body(d.old(), &unit, code.sources));
                    let change = diff.and_then(|d| d.change(tile));
                    let shown = new.as_ref().or(old.as_ref());
                    header(ui, explorer, change, &unit, shown, code, open);
                    let badges = call_badges(explorer, diff, &unit);
                    egui::ScrollArea::both()
                        .id_salt("peek_source")
                        .max_height(height)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            match (diff, &old, &new) {
                                (Some(_), None, None) | (None, _, None) => {
                                    no_source(ui, explorer, &unit);
                                }
                                (Some(_), old, new) => {
                                    changed(ui, old.as_ref(), new.as_ref(), &badges);
                                }
                                (None, _, Some(new)) => plain(ui, new, &badges),
                            }
                            // Room under the last line for the scroll bar.
                            ui.add_space(10.0);
                        });
                });
        });
}

/// The function's name, colored by its change in a diff, and where it is written as
/// a link to open it in the editor, with buttons to copy either.
fn header(
    ui: &mut egui::Ui,
    explorer: &Explorer,
    change: Option<Change>,
    unit: &UnitId,
    body: Option<&Body>,
    code: &mut Code<'_>,
    open: &mut bool,
) {
    let color = change.map_or(egui::Color32::WHITE, |c| {
        ui::egui_color(diff_view::change_color(c))
    });
    let sign = change.map(|c| format!("{} ", diff_view::sign(c)));
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "{}{}",
                sign.unwrap_or_default(),
                explorer.unit_name(unit)
            ))
            .strong()
            .size(14.0)
            .color(color),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("×")
                .on_hover_text("Hide the source (B)")
                .clicked()
            {
                *open = false;
            }
        });
    });
    ui.horizontal(|ui| {
        match body {
            Some(body) => ui::location_link(
                ui,
                &format!("{}:{}", body.uri, body.first),
                Some(body.place.clone()),
                code.requests,
            ),
            None => {
                ui.label(egui::RichText::new("no source").small().color(MUTED));
            }
        }
        ui::copy_buttons(ui, explorer.system(), unit, code.requests);
    });
    ui.add_space(2.0);
}

fn no_source(ui: &mut egui::Ui, explorer: &Explorer, unit: &UnitId) {
    match Place::of(explorer.system(), unit) {
        Some(place) => ui::cannot_read(ui, &place.file),
        None => {
            ui.label(egui::RichText::new("No source location.").color(MUTED));
        }
    }
}

/// The journey's calls from `unit`, by the file line each is made on: its number,
/// on its node's color. Only when the open journey starts at `unit`, as its numbers
/// count from there.
fn call_badges(
    explorer: &Explorer,
    diff: Option<&DiffView>,
    unit: &UnitId,
) -> HashMap<usize, Vec<(String, egui::Color32)>> {
    let mut badges: HashMap<usize, Vec<(String, egui::Color32)>> = HashMap::new();
    let Some(journey) = explorer.journey().filter(|_| explorer.root() == Some(unit)) else {
        return badges;
    };
    let disclosed = explorer.disclosure();
    for &span in disclosed.children(journey.root()) {
        let (Some(number), Some((_, location))) =
            (disclosed.number(span), explorer.call_site(span))
        else {
            continue;
        };
        let Some(line) = location.range.map(|r| r.start_line as usize) else {
            continue;
        };
        let fill = explorer
            .unit_id(span)
            .and_then(|u| explorer.map().tile_of(u))
            .map_or(MUTED, |tile| {
                ui::egui_color(graph_view::node_color(explorer, diff, tile))
            });
        badges
            .entry(line)
            .or_default()
            .push((number.to_owned(), fill));
    }
    badges
}

const FONT: f32 = 12.0;

/// The function's lines, numbered and colored by syntax.
fn plain(ui: &mut egui::Ui, body: &Body, badges: &HashMap<usize, Vec<(String, egui::Color32)>>) {
    let language = Language::of(&body.uri);
    let mut state = Lexer::default();
    for (index, text) in body.lines.iter().enumerate() {
        let number = body.first + index;
        let mut job = gutter(number, ' ', badges.get(&number));
        state.highlight(&mut job, text, language, None);
        ui.add(egui::Label::new(job).wrap_mode(egui::TextWrapMode::Extend));
    }
}

/// Old and new as one line diff, each line colored by syntax over the diff's own
/// green and red.
fn changed(
    ui: &mut egui::Ui,
    old: Option<&Body>,
    new: Option<&Body>,
    badges: &HashMap<usize, Vec<(String, egui::Color32)>>,
) {
    let none: &[String] = &[];
    let (old_lines, new_lines) = (
        old.map_or(none, |b| b.lines.as_slice()),
        new.map_or(none, |b| b.lines.as_slice()),
    );
    let (old_first, new_first) = (old.map_or(1, |b| b.first), new.map_or(1, |b| b.first));
    let language = Language::of(new.or(old).map_or("", |b| b.uri.as_str()));
    let (mut before, mut after) = (Lexer::default(), Lexer::default());
    for line in source_diff::line_diff(old_lines, new_lines) {
        let (number, sign, text, lexer, background) = match line {
            Line::Same(_, j) => (new_first + j, ' ', &new_lines[j], &mut after, None),
            Line::Added(j) => (
                new_first + j,
                '+',
                &new_lines[j],
                &mut after,
                Some(source_diff::ADDED_BG),
            ),
            Line::Removed(i) => (
                old_first + i,
                '-',
                &old_lines[i],
                &mut before,
                Some(source_diff::REMOVED_BG),
            ),
        };
        let marks = if sign == '-' {
            None
        } else {
            badges.get(&number)
        };
        let mut job = gutter(number, sign, marks);
        lexer.highlight(&mut job, text, language, background);
        if let Line::Same(i, _) = line {
            // The old side reads the same line too, to keep its comments in step.
            before.highlight(
                &mut egui::text::LayoutJob::default(),
                &old_lines[i],
                language,
                None,
            );
        }
        ui.add(egui::Label::new(job).wrap_mode(egui::TextWrapMode::Extend));
    }
}

/// A line's start: its number, its diff sign, and the badges of the calls it makes.
fn gutter(
    number: usize,
    sign: char,
    badges: Option<&Vec<(String, egui::Color32)>>,
) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    let format = |color| egui::TextFormat {
        font_id: egui::FontId::monospace(FONT),
        color,
        ..default()
    };
    job.append(
        &format!("{number:>4} {sign}"),
        0.0,
        format(egui::Color32::from_gray(95)),
    );
    let badges = badges.map_or(&[][..], Vec::as_slice);
    // Lines without a call keep the same indent as those with one.
    let width: usize = badges.iter().map(|(n, _)| n.len() + 2).sum();
    for (number, fill) in badges {
        job.append(
            &format!(" {number} "),
            0.0,
            egui::TextFormat {
                background: *fill,
                ..format(egui::Color32::from_rgb(14, 16, 22))
            },
        );
    }
    job.append(
        &" ".repeat(BADGE_ROOM.saturating_sub(width) + 1),
        0.0,
        format(egui::Color32::TRANSPARENT),
    );
    job
}

/// Room kept before each line for call badges, in characters.
const BADGE_ROOM: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Language {
    Rust,
    Python,
    Other,
}

impl Language {
    fn of(uri: &str) -> Self {
        match Path::new(uri).extension().and_then(|e| e.to_str()) {
            Some("rs") => Self::Rust,
            Some("py" | "pyi") => Self::Python,
            _ => Self::Other,
        }
    }

    fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::Rust => &[
                "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else",
                "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match",
                "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct",
                "super", "trait", "true", "type", "unsafe", "use", "where", "while",
            ],
            Self::Python => &[
                "and", "as", "assert", "async", "await", "break", "class", "continue", "def",
                "del", "elif", "else", "except", "False", "finally", "for", "from", "global", "if",
                "import", "in", "is", "lambda", "None", "nonlocal", "not", "or", "pass", "raise",
                "return", "self", "True", "try", "while", "with", "yield",
            ],
            Self::Other => &[],
        }
    }

    fn line_comment(self) -> &'static str {
        match self {
            Self::Python => "#",
            Self::Rust | Self::Other => "//",
        }
    }
}

/// What a piece of source is, for its color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Token {
    Plain,
    Keyword,
    Text,
    Number,
    Comment,
    /// A function or macro being called or defined.
    Call,
    Type,
    Punctuation,
}

impl Token {
    /// Quiet colors that read on the dark card and over a diff's green and red.
    fn color(self) -> egui::Color32 {
        match self {
            Self::Plain => egui::Color32::from_rgb(196, 202, 216),
            Self::Keyword => egui::Color32::from_rgb(206, 178, 244),
            Self::Text => egui::Color32::from_rgb(168, 206, 156),
            Self::Number => egui::Color32::from_rgb(214, 204, 160),
            Self::Comment => egui::Color32::from_rgb(112, 120, 138),
            Self::Call => egui::Color32::from_rgb(244, 246, 252),
            Self::Type => egui::Color32::from_rgb(140, 208, 200),
            Self::Punctuation => egui::Color32::from_rgb(140, 147, 164),
        }
    }
}

/// Colors source a line at a time, carrying a block comment from one line to the
/// next. Rough by design: enough to tell keywords, strings and comments apart.
#[derive(Debug, Default)]
struct Lexer {
    in_block_comment: bool,
}

impl Lexer {
    fn highlight(
        &mut self,
        job: &mut egui::text::LayoutJob,
        text: &str,
        language: Language,
        background: Option<egui::Color32>,
    ) {
        for (token, piece) in self.tokens(text, language) {
            job.append(
                piece,
                0.0,
                egui::TextFormat {
                    font_id: egui::FontId::monospace(FONT),
                    color: token.color(),
                    background: background.unwrap_or_default(),
                    ..default()
                },
            );
        }
    }

    /// `text` cut into colored pieces that together are all of it, neighbors of a
    /// kind joined.
    fn tokens<'a>(&mut self, text: &'a str, language: Language) -> Vec<(Token, &'a str)> {
        let mut spans: Vec<(Token, usize, usize)> = Vec::new();
        let mut at = 0;
        while at < text.len() {
            let rest = &text[at..];
            let (token, length) = self.next(rest, language);
            // At least one whole character, so every step moves on.
            let mut end = at + length.max(1).min(rest.len());
            while !text.is_char_boundary(end) {
                end += 1;
            }
            match spans.last_mut() {
                Some((last, _, last_end)) if *last == token => *last_end = end,
                _ => spans.push((token, at, end)),
            }
            at = end;
        }
        spans
            .into_iter()
            .map(|(token, start, end)| (token, &text[start..end]))
            .collect()
    }

    /// The token `rest` starts with, and its length in bytes.
    fn next(&mut self, rest: &str, language: Language) -> (Token, usize) {
        if self.in_block_comment {
            return match rest.find("*/") {
                Some(end) => {
                    self.in_block_comment = false;
                    (Token::Comment, end + 2)
                }
                None => (Token::Comment, rest.len()),
            };
        }
        if rest.starts_with(language.line_comment()) {
            return (Token::Comment, rest.len());
        }
        if language == Language::Rust && rest.starts_with("/*") {
            self.in_block_comment = true;
            return (Token::Comment, 2);
        }
        let first = rest.chars().next().unwrap_or(' ');
        if first == '"' || (first == '\'' && language == Language::Python) {
            return (Token::Text, quoted(rest, first));
        }
        if first == '\'' {
            // A char literal closes within a few characters; a lifetime does not.
            let close = rest[1..].find('\'').map(|i| i + 2);
            return match close.filter(|&c| c <= 4 || rest[1..].starts_with('\\')) {
                Some(_) => (Token::Text, quoted(rest, '\'')),
                None => (Token::Type, word_length(&rest[1..]) + 1),
            };
        }
        if first.is_ascii_digit() {
            return (Token::Number, word_length(rest));
        }
        if first.is_alphabetic() || first == '_' {
            let length = word_length(rest);
            let word = &rest[..length];
            let after = rest[length..].trim_start();
            let macro_call = language == Language::Rust
                && rest[length..].starts_with('!')
                && !rest[length..].starts_with("!=");
            let token = if language.keywords().contains(&word) {
                Token::Keyword
            } else if macro_call || after.starts_with('(') {
                Token::Call
            } else if first.is_uppercase() {
                Token::Type
            } else {
                Token::Plain
            };
            return (token, length + usize::from(macro_call));
        }
        if first.is_whitespace() {
            let length = rest.len() - rest.trim_start().len();
            return (Token::Plain, length);
        }
        (Token::Punctuation, first.len_utf8())
    }
}

/// The length of the quoted text `rest` starts with, through its closing quote or to
/// the end of the line.
fn quoted(rest: &str, quote: char) -> usize {
    let mut escaped = false;
    for (at, c) in rest.char_indices().skip(1) {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            c if c == quote => return at + c.len_utf8(),
            _ => {}
        }
    }
    rest.len()
}

fn word_length(rest: &str) -> usize {
    rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(rest.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens(text: &str, language: Language) -> Vec<(Token, String)> {
        Lexer::default()
            .tokens(text, language)
            .into_iter()
            .filter(|(token, piece)| *token != Token::Plain || !piece.trim().is_empty())
            .map(|(token, piece)| (token, piece.trim().to_owned()))
            .collect()
    }

    #[test]
    fn colors_rust_by_what_each_piece_is() {
        use Token::{Call, Comment, Keyword, Number, Plain, Punctuation, Text, Type};
        let line = r#"let total: Price<'a> = quote(&cart, "a \" b", 'x', 42)?; // why"#;
        let got = tokens(line, Language::Rust);
        let expected = [
            (Keyword, "let"),
            (Plain, "total"),
            (Punctuation, ":"),
            (Type, "Price"),
            (Punctuation, "<"),
            (Type, "'a"),
            (Punctuation, ">"),
            (Punctuation, "="),
            (Call, "quote"),
            (Punctuation, "(&"),
            (Plain, "cart"),
            (Punctuation, ","),
            (Text, r#""a \" b""#),
            (Punctuation, ","),
            (Text, "'x'"),
            (Punctuation, ","),
            (Number, "42"),
            (Punctuation, ")?;"),
            (Comment, "// why"),
        ];
        assert_eq!(
            got,
            expected
                .iter()
                .map(|&(t, s)| (t, s.to_owned()))
                .collect::<Vec<_>>()
        );
        assert_eq!(
            tokens("println!(\"x\")", Language::Rust)[0],
            (Call, "println!".to_owned())
        );
    }

    #[test]
    fn carries_a_block_comment_across_lines() {
        let mut lexer = Lexer::default();
        let first = lexer.tokens("x /* one", Language::Rust);
        assert_eq!(first.last().map(|t| t.0), Some(Token::Comment));
        let second = lexer.tokens("two */ y", Language::Rust);
        assert_eq!(second[0], (Token::Comment, "two */"));
        assert_eq!(
            tokens("a != b", Language::Rust)[0],
            (Token::Plain, "a".to_owned()),
            "not a macro"
        );
        assert_eq!(second.last().map(|t| t.0), Some(Token::Plain));
    }

    #[test]
    fn reads_python_comments_and_quotes() {
        let got = tokens("def f(x='a'):  # note", Language::Python);
        assert_eq!(got[0], (Token::Keyword, "def".to_owned()));
        assert!(got.contains(&(Token::Text, "'a'".to_owned())));
        assert_eq!(got.last(), Some(&(Token::Comment, "# note".to_owned())));
    }

    #[test]
    fn every_line_keeps_all_its_text() {
        for line in ["  é = \"ü", "'", "a'b'c", "/* x */ y // z", "r#\"raw\"#"] {
            let joined: String = Lexer::default()
                .tokens(line, Language::Rust)
                .into_iter()
                .map(|(_, piece)| piece)
                .collect();
            assert_eq!(joined, line);
        }
    }
}
