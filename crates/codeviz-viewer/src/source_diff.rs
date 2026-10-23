//! A function's source before and after a change, as a unified line diff: removed
//! lines on a red ground and added ones on a green one, the status colors the graph
//! fills changed nodes with, with the part of a rewritten line that changed picked out.

use bevy_egui::egui;

use crate::{
    diff_view::{ADDED, REMOVED},
    ui::{MUTED, egui_color},
};

/// One line of a diff, by line index into the old and new text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Line {
    Same(usize, usize),
    Removed(usize),
    Added(usize),
}

impl Line {
    fn changed(self) -> bool {
        !matches!(self, Self::Same(..))
    }
}

/// The fewest lines removed and added to turn `old` into `new`, in order: a longest
/// common subsequence over whole lines, compared without surrounding whitespace so a
/// reindent alone does not count. Functions are short enough for the quadratic table.
pub(crate) fn line_diff<S: AsRef<str>>(old: &[S], new: &[S]) -> Vec<Line> {
    let same = |i: usize, j: usize| old[i].as_ref().trim() == new[j].as_ref().trim();
    let (n, m) = (old.len(), new.len());
    // `common[i][j]`: the longest common run of `old[i..]` and `new[j..]`.
    let mut common = vec![vec![0_u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            common[i][j] = if same(i, j) {
                common[i + 1][j + 1] + 1
            } else {
                common[i + 1][j].max(common[i][j + 1])
            };
        }
    }
    let mut lines = Vec::with_capacity(n.max(m));
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && same(i, j) {
            lines.push(Line::Same(i, j));
            i += 1;
            j += 1;
        } else if i < n && (j == m || common[i + 1][j] >= common[i][j + 1]) {
            lines.push(Line::Removed(i));
            i += 1;
        } else {
            lines.push(Line::Added(j));
            j += 1;
        }
    }
    lines
}

/// Unchanged lines kept either side of a change; longer runs between fold away.
const CONTEXT: usize = 3;

/// A row as shown: a line, or a run of this many unchanged lines folded away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Row {
    Line(Line),
    Folded(usize),
}

/// `lines` with only the unchanged lines within `context` of a change, and the first,
/// which names the function; every longer run of the rest folds into one row. With
/// nothing changed, every line shows.
pub(crate) fn fold(lines: &[Line], context: usize) -> Vec<Row> {
    let changed: Vec<usize> = (0..lines.len()).filter(|&i| lines[i].changed()).collect();
    if changed.is_empty() {
        return lines.iter().copied().map(Row::Line).collect();
    }
    let kept = |i: usize| i == 0 || changed.iter().any(|&c| c.abs_diff(i) <= context);
    let mut rows = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if kept(i) {
            rows.push(Row::Line(lines[i]));
            i += 1;
            continue;
        }
        let run = (i..lines.len()).take_while(|&j| !kept(j)).count();
        // Folding a single line saves nothing.
        if run == 1 {
            rows.push(Row::Line(lines[i]));
        } else {
            rows.push(Row::Folded(run));
        }
        i += run;
    }
    rows
}

/// Where a rewritten line differs from the line it replaced: the byte range of each
/// between what they start and end with in common.
fn differing(old: &str, new: &str) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
    let prefix: usize = old
        .chars()
        .zip(new.chars())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    let suffix: usize = old[prefix..]
        .chars()
        .rev()
        .zip(new[prefix..].chars().rev())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a.len_utf8())
        .sum();
    (prefix..old.len() - suffix, prefix..new.len() - suffix)
}

/// For each removed or added line paired with one on the other side, the part of it
/// that changed: a run of removed lines straight before a run of added ones pairs
/// line for line.
fn pairs<S: AsRef<str>>(
    lines: &[Line],
    old: &[S],
    new: &[S],
) -> std::collections::HashMap<Line, std::ops::Range<usize>> {
    let mut parts = std::collections::HashMap::new();
    let mut i = 0;
    while i < lines.len() {
        let removed = lines[i..]
            .iter()
            .take_while(|l| matches!(l, Line::Removed(_)))
            .count();
        let added = lines[i + removed..]
            .iter()
            .take_while(|l| matches!(l, Line::Added(_)))
            .count();
        for k in 0..removed.min(added) {
            if let (Line::Removed(a), Line::Added(b)) = (lines[i + k], lines[i + removed + k]) {
                let (was, now) = differing(old[a].as_ref(), new[b].as_ref());
                // A line rewritten through and through has nothing to pick out.
                if was.len() < old[a].as_ref().trim().len() {
                    parts.insert(Line::Removed(a), was);
                }
                if now.len() < new[b].as_ref().trim().len() {
                    parts.insert(Line::Added(b), now);
                }
            }
        }
        i += (removed + added).max(1);
    }
    parts
}

/// One version of a function: its lines, the file line of the first, and its file.
pub(crate) struct Version {
    pub(crate) lines: Vec<String>,
    pub(crate) first: usize,
    pub(crate) uri: String,
}

/// Lines removed and added between two versions, either of which may be missing.
pub(crate) fn counts(old: Option<&Version>, new: Option<&Version>) -> (usize, usize) {
    let none: &[String] = &[];
    let lines = line_diff(
        old.map_or(none, |v| &v.lines),
        new.map_or(none, |v| &v.lines),
    );
    lines.iter().fold((0, 0), |(a, r), line| match line {
        Line::Added(_) => (a + 1, r),
        Line::Removed(_) => (a, r + 1),
        Line::Same(..) => (a, r),
    })
}

/// Old and new in one column: removed lines red with their old line number, added
/// lines green with their new one. With one side missing, the function was added or
/// removed and every line shows as such. `folded` keeps only what is near a change.
/// It scrolls both ways within `max_height`, and back to the first change whenever
/// `key`, what it shows, changes.
pub(crate) fn show(
    ui: &mut egui::Ui,
    old: Option<&Version>,
    new: Option<&Version>,
    folded: bool,
    max_height: f32,
    key: egui::Id,
) {
    let none: &[String] = &[];
    let (old_lines, new_lines) = (
        old.map_or(none, |v| v.lines.as_slice()),
        new.map_or(none, |v| v.lines.as_slice()),
    );
    let (old_first, new_first) = (old.map_or(1, |v| v.first), new.map_or(1, |v| v.first));
    let lines = line_diff(old_lines, new_lines);
    let parts = pairs(&lines, old_lines, new_lines);
    let rows = if folded {
        fold(&lines, CONTEXT)
    } else {
        lines.iter().copied().map(Row::Line).collect()
    };
    let font = egui::FontId::monospace(12.5);
    let (added, removed) = (egui_color(ADDED), egui_color(REMOVED));
    let shown = egui::Id::new("source_diff_shown");
    let fresh = ui.ctx().data_mut(|d| {
        let fresh = d.get_temp::<egui::Id>(shown) != Some(key);
        d.insert_temp(shown, key);
        fresh
    });
    egui::ScrollArea::both()
        .id_salt("source_diff")
        .max_height(max_height)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut first_change = fresh;
            for row in rows {
                let mut job = egui::text::LayoutJob::default();
                let format = |color: egui::Color32| egui::TextFormat {
                    font_id: font.clone(),
                    color,
                    ..Default::default()
                };
                let (ground, line) = match row {
                    Row::Folded(count) => {
                        job.append(
                            &format!("     ··· {count} unchanged lines"),
                            0.0,
                            format(MUTED),
                        );
                        (None, None)
                    }
                    Row::Line(line) => {
                        let (number, sign, text, tint) = match line {
                            Line::Same(_, j) => (new_first + j, ' ', &new_lines[j], None),
                            Line::Added(j) => (new_first + j, '+', &new_lines[j], Some(added)),
                            Line::Removed(i) => (old_first + i, '−', &old_lines[i], Some(removed)),
                        };
                        job.append(&format!("{number:>4} "), 0.0, format(MUTED));
                        job.append(&format!("{sign} "), 0.0, format(tint.unwrap_or(MUTED)));
                        let text_color = if tint.is_some() {
                            egui::Color32::from_rgb(236, 239, 244)
                        } else {
                            egui::Color32::from_rgb(150, 156, 172)
                        };
                        match (parts.get(&line), tint) {
                            (Some(part), Some(tint)) => {
                                job.append(&text[..part.start], 0.0, format(text_color));
                                let mut picked = format(egui::Color32::WHITE);
                                picked.background = tint.gamma_multiply(0.42);
                                job.append(&text[part.clone()], 0.0, picked);
                                job.append(&text[part.end..], 0.0, format(text_color));
                            }
                            _ => job.append(text, 0.0, format(text_color)),
                        }
                        (tint, Some(line))
                    }
                };
                let galley = ui.painter().layout_job(job);
                let size = egui::vec2(
                    (galley.size().x + 8.0).max(ui.available_width()),
                    galley.size().y + 2.0,
                );
                let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                if let Some(tint) = ground {
                    ui.painter()
                        .rect_filled(rect, 0.0, tint.gamma_multiply(0.16));
                }
                ui.painter().galley(
                    rect.min + egui::vec2(4.0, 1.0),
                    galley,
                    egui::Color32::WHITE,
                );
                if first_change && line.is_some_and(Line::changed) {
                    first_change = false;
                    // Down to it, but not across: the line's start stays in view.
                    let start = egui::Rect::from_min_size(rect.min, egui::vec2(1.0, rect.height()));
                    ui.scroll_to_rect(start, Some(egui::Align::Center));
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_what_both_share_and_marks_the_rest() {
        let old = ["fn a() {", "    one();", "    two();", "}"];
        let new = [
            "fn a() {",
            "        one();",
            "    three();",
            "    two();",
            "}",
        ];
        assert_eq!(
            line_diff(&old, &new),
            [
                Line::Same(0, 0),
                Line::Same(1, 1),
                Line::Added(2),
                Line::Same(2, 3),
                Line::Same(3, 4),
            ],
            "a reindent is not a change"
        );
        assert_eq!(
            line_diff(&["x", "y"], &["y"]),
            [Line::Removed(0), Line::Same(1, 0)]
        );
        assert_eq!(line_diff::<&str>(&[], &["z"]), [Line::Added(0)]);
    }

    #[test]
    fn folds_what_is_far_from_a_change_but_the_first_line() {
        let same = |i| Line::Same(i, i);
        let mut lines: Vec<Line> = (0..20).map(same).collect();
        lines[12] = Line::Added(12);
        assert_eq!(
            fold(&lines, 3),
            [
                Row::Line(same(0)),
                Row::Folded(8),
                Row::Line(same(9)),
                Row::Line(same(10)),
                Row::Line(same(11)),
                Row::Line(Line::Added(12)),
                Row::Line(same(13)),
                Row::Line(same(14)),
                Row::Line(same(15)),
                Row::Folded(4),
            ]
        );
        let short: Vec<Line> = (0..3).map(same).collect();
        assert_eq!(fold(&short, 3).len(), 3, "nothing changed, all of it shows");
    }

    #[test]
    fn picks_out_what_changed_in_a_rewritten_line() {
        let old = ["    let p = bulk_discount(x);"];
        let new = ["    let p = apply_discount(x);"];
        let lines = line_diff(&old, &new);
        let parts = pairs(&lines, &old, &new);
        assert_eq!(&old[0][parts[&Line::Removed(0)].clone()], "bulk");
        assert_eq!(&new[0][parts[&Line::Added(0)].clone()], "apply");
    }
}
