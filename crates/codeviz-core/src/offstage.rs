//! What a diff changed that its graph does not draw: lines outside every function,
//! such as types, constants, imports and tests left out of the analysis, and files the
//! analysis never read, such as manifests, docs and migrations.

use codeviz_ir::Role;

use crate::System;

/// The lines one file lost and gained, by line number in each version, from 1, with
/// the text of each version (empty where the file did not exist).
#[derive(Debug, Clone, Default)]
pub struct FileChange {
    /// Relative to the analyzed root, as unit locations name files.
    pub path: String,
    pub removed: Vec<u32>,
    pub added: Vec<u32>,
    pub old_text: Vec<String>,
    pub new_text: Vec<String>,
}

/// One file's changes the graph does not draw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offstage {
    pub path: String,
    /// What the changes sit in, each by the line that opens it: `const KEYS: [&str; 4]`,
    /// `pub struct Order`, `mod tests`. Empty for a file the analysis did not read.
    pub places: Vec<String>,
    /// Lines added and removed outside every function.
    pub added: usize,
    pub removed: usize,
}

/// Longest a place is named.
const PLACE: usize = 60;

/// The changes in `files` the graph of `before` and `after` does not draw: in a file
/// the analysis read, the lines outside every function (and the comments and
/// attributes just above one) that are not blank; in any other file, every line.
#[must_use]
pub fn offstage(files: &[FileChange], before: &System, after: &System) -> Vec<Offstage> {
    let mut found = Vec::new();
    for file in files {
        let read = [before, after]
            .iter()
            .any(|system| system.units().any(|u| in_file(u, &file.path)));
        if !read {
            found.push(Offstage {
                path: file.path.clone(),
                places: Vec::new(),
                added: file.added.len(),
                removed: file.removed.len(),
            });
            continue;
        }
        let outside = |lines: &[u32], system: &System, text: &[String]| -> Vec<u32> {
            let functions = functions(system, &file.path, text);
            lines
                .iter()
                .copied()
                .filter(|&line| !functions.iter().any(|r| r.contains(&line)))
                .filter(|&line| line_at(text, line).is_some_and(|t| !t.trim().is_empty()))
                .collect()
        };
        let removed = outside(&file.removed, before, &file.old_text);
        let added = outside(&file.added, after, &file.new_text);
        if removed.is_empty() && added.is_empty() {
            continue;
        }
        let mut places: Vec<String> = Vec::new();
        let named = added
            .iter()
            .filter_map(|&line| place(&file.new_text, line))
            .chain(
                removed
                    .iter()
                    .filter_map(|&line| place(&file.old_text, line)),
            );
        for name in named {
            if !places.contains(&name) {
                places.push(name);
            }
        }
        found.push(Offstage {
            path: file.path.clone(),
            places,
            added: added.len(),
            removed: removed.len(),
        });
    }
    found
}

fn in_file(unit: &codeviz_ir::Unit, path: &str) -> bool {
    unit.location.as_ref().is_some_and(|l| l.uri == path)
}

/// Line `number`, from 1.
fn line_at(text: &[String], number: u32) -> Option<&str> {
    text.get((number as usize).checked_sub(1)?)
        .map(String::as_str)
}

/// Whether a line is only a comment or an attribute: what sits above an item and
/// belongs to it.
fn preamble(line: &str) -> bool {
    let line = line.trim_start();
    ["//", "/*", "*", "#", "@"]
        .iter()
        .any(|lead| line.starts_with(lead))
}

/// The lines of every function in `path`, each widened up over the comments and
/// attributes straight above it.
fn functions(system: &System, path: &str, text: &[String]) -> Vec<std::ops::RangeInclusive<u32>> {
    system
        .units()
        .filter(|u| u.role == Role::Invocable && in_file(u, path))
        .filter_map(|u| u.location.as_ref()?.range)
        .map(|range| {
            let mut start = range.start_line;
            while start > 1 && line_at(text, start - 1).is_some_and(preamble) {
                start -= 1;
            }
            start..=range.end_line
        })
        .collect()
}

/// What line `number` sits in, by the line that opens it: the nearest line at or
/// above it that starts at the margin and opens something rather than closing it, up
/// to any value it sets. A comment or attribute counts as part of what follows it.
/// Imports are all one.
fn place(text: &[String], number: u32) -> Option<String> {
    let index = (number as usize).checked_sub(1)?;
    let opens = |line: &String| {
        !line.trim().is_empty()
            && !preamble(line)
            && !line.trim_start().starts_with(['}', ')', ']'])
    };
    let index = if text.get(index).is_some_and(|line| preamble(line)) {
        index + text[index..].iter().position(opens)?
    } else {
        index
    };
    let found = text[..=index.min(text.len().checked_sub(1)?)]
        .iter()
        .rev()
        .find(|line| opens(line) && !line.starts_with(char::is_whitespace))?;
    // Up to any value, which is what changes: `const LIMIT: usize`.
    let name = found.trim();
    let name = name.split_once(" = ").map_or(name, |(name, _)| name);
    let name = name
        .trim_end_matches(['{', '(', '[', '=', ' ', ':', ';'])
        .trim_end();
    if ["use ", "pub use ", "pub(crate) use ", "import ", "from "]
        .iter()
        .any(|lead| name.starts_with(lead))
    {
        return Some("imports".to_owned());
    }
    Some(match name.char_indices().nth(PLACE) {
        Some((cut, _)) => format!("{}…", &name[..cut]),
        None => name.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use codeviz_ir::Document;

    use super::*;

    const SOURCE: &str = "use std::fmt;

/// How many to keep.
const LIMIT: usize = 4;

pub struct Order {
    pub id: u64,
}

/// Places an order.
fn place(order: &Order) -> u64 {
    order.id + 1
}

#[cfg(test)]
mod tests {
    #[test]
    fn works() {}
}";

    fn system(path: &str) -> System {
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }},
            "units": [{{ "id": "x:place", "name": "place", "kind": "fn", "role": "invocable",
              "location": {{ "uri": "{path}", "range": {{ "start_line": 11, "start_col": 1, "end_line": 13, "end_col": 2 }} }} }}] }}"#
        );
        System::new(vec![Document::from_json(&json).unwrap()])
    }

    fn text() -> Vec<String> {
        SOURCE.lines().map(str::to_owned).collect()
    }

    #[test]
    fn names_what_changed_outside_functions_by_what_holds_it() {
        let file = FileChange {
            path: "src/lib.rs".to_owned(),
            // The constant, a field, the function and its doc, a test, a blank line,
            // an import.
            added: vec![4, 7, 10, 12, 18, 9, 1],
            new_text: text(),
            ..FileChange::default()
        };
        let (old, new) = (system("src/lib.rs"), system("src/lib.rs"));
        assert_eq!(
            offstage(&[file], &old, &new),
            [Offstage {
                path: "src/lib.rs".to_owned(),
                places: vec![
                    "const LIMIT: usize".to_owned(),
                    "pub struct Order".to_owned(),
                    "mod tests".to_owned(),
                    "imports".to_owned(),
                ],
                added: 4,
                removed: 0,
            }],
            "the function and its doc are drawn, and a blank line is nothing"
        );
    }

    #[test]
    fn a_doc_comment_belongs_to_what_it_documents() {
        assert_eq!(place(&text(), 3).as_deref(), Some("const LIMIT: usize"));
        assert_eq!(place(&text(), 15).as_deref(), Some("mod tests"));
        assert_eq!(
            place(&text(), 17).as_deref(),
            Some("mod tests"),
            "an attribute on a test inside the module"
        );
    }

    #[test]
    fn a_file_the_analysis_never_read_counts_whole() {
        let file = FileChange {
            path: "Cargo.toml".to_owned(),
            added: vec![3, 4],
            removed: vec![3],
            ..FileChange::default()
        };
        let none = system("src/lib.rs");
        assert_eq!(
            offstage(&[file], &none, &none),
            [Offstage {
                path: "Cargo.toml".to_owned(),
                places: Vec::new(),
                added: 2,
                removed: 1,
            }]
        );
    }

    #[test]
    fn changes_inside_functions_alone_are_not_listed() {
        let file = FileChange {
            path: "src/lib.rs".to_owned(),
            added: vec![12],
            removed: vec![12],
            old_text: text(),
            new_text: text(),
        };
        let both = system("src/lib.rs");
        assert_eq!(offstage(&[file], &both, &both), []);
    }
}
