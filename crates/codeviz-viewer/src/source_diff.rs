//! A function's source before and after a change, as a unified line diff.

use bevy_egui::egui;

use crate::ui::MUTED;

/// One line of a diff, by line index into the old and new text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Line {
    Same(usize, usize),
    Removed(usize),
    Added(usize),
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

/// Where one version of a function sits: its lines, and the file line of the first.
pub(crate) struct Side<'a> {
    pub(crate) lines: &'a [String],
    pub(crate) first: usize,
}

const ADDED_BG: egui::Color32 = egui::Color32::from_rgb(22, 58, 34);
const REMOVED_BG: egui::Color32 = egui::Color32::from_rgb(72, 26, 28);

/// Old and new side by side in one column: removed lines red with their old line
/// number, added lines green with their new one. With one side missing, the function
/// was added or removed and every line shows as such.
pub(crate) fn show(ui: &mut egui::Ui, old: Option<Side<'_>>, new: Option<Side<'_>>) {
    let none: &[String] = &[];
    let (old_lines, new_lines) = (
        old.as_ref().map_or(none, |s| s.lines),
        new.as_ref().map_or(none, |s| s.lines),
    );
    let (old_first, new_first) = (
        old.as_ref().map_or(1, |s| s.first),
        new.as_ref().map_or(1, |s| s.first),
    );
    let lines = line_diff(old_lines, new_lines);
    let (added, removed) = lines.iter().fold((0, 0), |(a, r), line| match line {
        Line::Added(_) => (a + 1, r),
        Line::Removed(_) => (a, r + 1),
        Line::Same(..) => (a, r),
    });
    ui.label(
        egui::RichText::new(format!("+{added} −{removed} lines"))
            .small()
            .color(MUTED),
    );
    let font = egui::FontId::monospace(12.5);
    let mut first_change = true;
    egui::ScrollArea::horizontal()
        .id_salt("source_diff")
        .show(ui, |ui| {
            for line in lines {
                let (number, sign, text, background) = match line {
                    Line::Same(_, j) => (new_first + j, ' ', &new_lines[j], None),
                    Line::Added(j) => (new_first + j, '+', &new_lines[j], Some(ADDED_BG)),
                    Line::Removed(i) => (old_first + i, '-', &old_lines[i], Some(REMOVED_BG)),
                };
                let row =
                    egui::RichText::new(format!("{number:>4} {sign} {text}")).font(font.clone());
                let row = match background {
                    Some(color) => row.color(egui::Color32::WHITE).background_color(color),
                    None => row.color(egui::Color32::from_rgb(150, 156, 172)),
                };
                let response = ui.add(egui::Label::new(row).wrap_mode(egui::TextWrapMode::Extend));
                if background.is_some() && first_change {
                    first_change = false;
                    response.scroll_to_me(Some(egui::Align::Center));
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
}
