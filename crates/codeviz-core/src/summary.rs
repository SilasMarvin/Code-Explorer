//! What a diff changes, structurally, as Markdown for a pull request: the functions
//! that changed, grouped by where they live, the calls that appeared or went away, and
//! the entry points that can reach a change.

use std::{
    collections::{BTreeMap, HashMap},
    fmt::Write as _,
};

use codeviz_ir::{Role, UnitId};

use crate::{Change, Diff, System, display_name, markdown::change_word, upstream};

/// Lists longer than this end with how many more there are.
const MAX_ITEMS: usize = 20;
/// Changes named for each entry point, nearest first.
const MAX_REACHED: usize = 4;

impl Diff {
    /// The diff from `before` to `after` as Markdown. `before` is the aligned old
    /// version, with moved functions under their new ids.
    #[must_use]
    pub fn markdown(&self, before: &System, after: &System) -> String {
        if self.is_empty() {
            return "Nothing changed structurally.\n".to_owned();
        }
        let mut out = format!("{}\n", self.totals(before, after));
        self.functions(before, after, &mut out);
        let added = self.calls(&self.added_calls, after, Change::Added);
        list(&mut out, "Calls added", &added);
        let removed = self.calls(&self.removed_calls, before, Change::Removed);
        list(&mut out, "Calls removed", &removed);
        list(
            &mut out,
            "Entry points affected",
            &self.affected(before, after),
        );
        out
    }

    /// `3 functions changed (1 new, 2 rewritten); 4 calls added, 1 removed.` Modules
    /// and types that changed are listed, not counted.
    fn totals(&self, before: &System, after: &System) -> String {
        let functions: Vec<Change> = self
            .units
            .iter()
            .filter(|&(id, &change)| {
                version(change, before, after)
                    .unit(id)
                    .is_some_and(|u| u.role == Role::Invocable)
            })
            .map(|(_, &change)| change)
            .collect();
        let count = |kind| functions.iter().filter(|&&c| c == kind).count();
        let kinds: Vec<String> = [
            (Change::Added, "new"),
            (Change::Modified, "rewritten"),
            (Change::Moved, "moved"),
            (Change::Removed, "removed"),
        ]
        .into_iter()
        .filter(|&(kind, _)| count(kind) > 0)
        .map(|(kind, word)| format!("{} {word}", count(kind)))
        .collect();
        let functions = match functions.len() {
            0 => "No functions changed".to_owned(),
            1 => format!("1 function changed ({})", kinds.join(", ")),
            n => format!("{n} functions changed ({})", kinds.join(", ")),
        };
        format!(
            "{functions}; {} calls added, {} removed.",
            self.added_calls.len(),
            self.removed_calls.len()
        )
    }

    /// Changed functions under the module, class or type each lives in.
    fn functions(&self, before: &System, after: &System, out: &mut String) {
        let mut groups: BTreeMap<String, Vec<(Change, String)>> = BTreeMap::new();
        for (id, &change) in &self.units {
            let Some(unit) = version(change, before, after).unit(id) else {
                continue;
            };
            let home = unit.parent.as_ref().map_or_else(String::new, path);
            // A module or type is named as one; a function by its name alone.
            let what = if unit.role == Role::Invocable {
                format!("{} `{}`", change_word(change), unit.name)
            } else {
                format!("{} {} `{}`", change_word(change), unit.kind, unit.name)
            };
            let line = match self.moved.get(id) {
                Some(was) => format!("{what}, from `{}`", path(was)),
                None => what,
            };
            groups.entry(home).or_default().push((change, line));
        }
        out.push_str("\nChanged functions:\n");
        for (home, mut lines) in groups {
            lines.sort();
            if home.is_empty() {
                for (_, line) in lines {
                    let _ = writeln!(out, "- {line}");
                }
                continue;
            }
            let _ = writeln!(out, "- `{home}`");
            for (_, line) in lines {
                let _ = writeln!(out, "  - {line}");
            }
        }
    }

    /// Calls by caller, as `` `a` → `b`, `c` ``. Those made inside a function that is
    /// itself `inside` (new, or removed) come with it, so they are counted last rather
    /// than listed.
    fn calls(
        &self,
        calls: &std::collections::HashSet<crate::Call>,
        system: &System,
        inside: Change,
    ) -> Vec<String> {
        let mut within = 0;
        let mut by_caller: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (from, to) in calls {
            if self.change(system.owner(from)) == Some(inside) {
                within += 1;
                continue;
            }
            by_caller
                .entry(display_name(system, from))
                .or_default()
                .push(format!("`{}`", display_name(system, to)));
        }
        let lines: Vec<String> = by_caller
            .into_iter()
            .map(|(caller, mut callees)| {
                callees.sort();
                format!("`{caller}` → {}", callees.join(", "))
            })
            .collect();
        let mut lines = cut(lines, "callers");
        let what = if inside == Change::Added {
            "new"
        } else {
            "removed"
        };
        match within {
            0 => {}
            1 => lines.push(format!("and 1 inside {what} functions")),
            n => lines.push(format!("and {n} inside {what} functions")),
        }
        lines
    }

    /// Each entry point that can reach a change, nearest first, with the changes it
    /// reaches. Without declared entries, the functions nothing calls stand in.
    fn affected(&self, before: &System, after: &System) -> Vec<String> {
        // Entry label → (fewest calls to a change, the changes it reaches).
        let mut reached: HashMap<String, (u32, Vec<(u32, String)>)> = HashMap::new();
        let mut changed: Vec<(&UnitId, Change)> = self
            .units
            .iter()
            .map(|(id, &change)| (id, change))
            .collect();
        changed.sort();
        for (id, change) in changed {
            let system = version(change, before, after);
            let up = upstream(system, id);
            let mut entries: Vec<(u32, String)> = up
                .entries
                .iter()
                .filter_map(|e| {
                    let hops = *up.distance.get(&e.unit)?;
                    Some((
                        hops,
                        format!("`{}` (`{}`)", e.label, display_name(system, &e.unit)),
                    ))
                })
                .collect();
            if system.entries().is_empty() {
                entries.extend(
                    up.distance
                        .iter()
                        .filter(|(unit, _)| is_root(system, unit))
                        .map(|(unit, &hops)| (hops, format!("`{}`", display_name(system, unit)))),
                );
            }
            let name = format!("`{}`", display_name(system, id));
            for (hops, entry) in entries {
                let slot = reached.entry(entry).or_insert((hops, Vec::new()));
                slot.0 = slot.0.min(hops);
                if !slot.1.iter().any(|(_, n)| *n == name) {
                    slot.1.push((hops, name.clone()));
                }
            }
        }
        let mut lines: Vec<(u32, String)> = reached
            .into_iter()
            .map(|(entry, (hops, mut changes))| {
                changes.sort();
                let names: Vec<String> = changes.into_iter().map(|(_, n)| n).collect();
                let line = match names.as_slice() {
                    _ if hops == 0 => format!("{entry} changed itself"),
                    [] => entry,
                    few if few.len() <= MAX_REACHED => {
                        format!("{entry} reaches {}", few.join(", "))
                    }
                    many => format!(
                        "{entry} reaches {}, and {} more",
                        many[..MAX_REACHED - 1].join(", "),
                        many.len() - (MAX_REACHED - 1)
                    ),
                };
                (hops, line)
            })
            .collect();
        lines.sort();
        cut(
            lines.into_iter().map(|(_, line)| line).collect(),
            "entry points",
        )
    }
}

/// The version a change is seen in: the old one for a removal, else the new one.
fn version<'s>(change: Change, before: &'s System, after: &'s System) -> &'s System {
    if change == Change::Removed {
        before
    } else {
        after
    }
}

/// A function nothing calls: where a program without declared entries starts.
fn is_root(system: &System, unit: &UnitId) -> bool {
    system
        .unit(unit)
        .is_some_and(|u| u.role == Role::Invocable && system.owner(unit) == unit)
        && system.callers(unit).is_empty()
}

/// An id without its scheme: `axum_server::orders::place`.
fn path(id: &UnitId) -> String {
    id.as_str()
        .split_once(':')
        .map_or(id.as_str(), |(_, path)| path)
        .to_owned()
}

/// A titled list; nothing when it is empty.
fn list(out: &mut String, title: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    let _ = writeln!(out, "\n{title}:");
    for item in items {
        let _ = writeln!(out, "- {item}");
    }
}

/// `lines` cut at [`MAX_ITEMS`], ending with how many more `what` there are.
fn cut(mut lines: Vec<String>, what: &str) -> Vec<String> {
    if lines.len() > MAX_ITEMS {
        let rest = lines.len() - MAX_ITEMS;
        lines.truncate(MAX_ITEMS);
        lines.push(format!("… and {rest} more {what}"));
    }
    lines
}
