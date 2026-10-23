//! What each function is called on screen, the same on its node, its bar, in the
//! details and in search. A function goes by its own name, with only as much of its
//! path in front as it takes to tell it from the others in view: its type, then its
//! module, then its crate, so `new` reads `System.new` only where another `new` shows
//! too. A module's top-level code goes by its module's name, and a closure by the
//! function around it, `draw_graph_labels › closure 3`.

use std::collections::HashMap;

use codeviz_ir::{Role, Unit, UnitId};

/// Joins a function to what it is nested in.
const INSIDE: &str = " › ";

/// A unit's own name and what can go in front of it to tell it apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Path {
    name: String,
    /// The segments around it, nearest first, each with what joins it to the next
    /// one in: `.` after a type or module, ` › ` after a function.
    outer: Vec<(String, &'static str)>,
    /// How many of them it always carries: a closure, the function it is in.
    least: usize,
}

impl Path {
    /// The path of `id`, its containers found by `lookup`.
    pub(crate) fn of<'a>(lookup: impl Fn(&UnitId) -> Option<&'a Unit>, id: &UnitId) -> Self {
        let Some(mut unit) = lookup(id) else {
            return Self {
                name: id.to_string(),
                outer: Vec::new(),
                least: 0,
            };
        };
        let parent_of = |unit: &Unit| unit.parent.as_ref().and_then(&lookup);
        // A module's top-level code stands for its module.
        if is_body(&unit.name)
            && let Some(module) = parent_of(unit)
        {
            unit = module;
        }
        let mut path = Self {
            name: segment(unit, parent_of(unit)),
            outer: Vec::new(),
            least: 0,
        };
        let mut nameless = is_anonymous(&unit.name);
        while let Some(parent) = parent_of(unit) {
            let joint = if parent.role == Role::Invocable {
                INSIDE
            } else {
                "."
            };
            path.outer.push((segment(parent, parent_of(parent)), joint));
            if nameless {
                path.least += 1;
                nameless = is_anonymous(&parent.name);
            }
            unit = parent;
        }
        path
    }

    /// The same path, carrying at least `depth` of its containers.
    pub(crate) fn at_least(mut self, depth: usize) -> Self {
        self.least = self.least.max(depth.min(self.outer.len()));
        self
    }

    /// The name with `depth` of its containers in front.
    pub(crate) fn render(&self, depth: usize) -> String {
        let mut text = self.name.clone();
        for (segment, joint) in self.outer.iter().take(depth) {
            text = format!("{segment}{joint}{text}");
        }
        text
    }

    /// The name with as little in front as it always carries.
    pub(crate) fn short(&self) -> String {
        self.render(self.least)
    }

    /// The whole path, crate and all.
    pub(crate) fn full(&self) -> String {
        self.render(self.outer.len())
    }
}

/// `<module>`: a module's own code, rather than something in it.
fn is_body(name: &str) -> bool {
    name.starts_with('<') && name.ends_with('>') && !name.contains('#')
}

/// `{closure#0}` or `<lambda#1>`: a name the producer made up.
fn is_anonymous(name: &str) -> bool {
    let wrapped = (name.starts_with('{') && name.ends_with('}'))
        || (name.starts_with('<') && name.ends_with('>'));
    wrapped && name.contains('#')
}

/// A unit's own part of its path: `closure 0` for `{closure#0}`, and `pricing` for
/// Python's `shop.services.pricing` inside `shop.services`.
fn segment(unit: &Unit, parent: Option<&Unit>) -> String {
    if is_anonymous(&unit.name) {
        return unit.name[1..unit.name.len() - 1].replace('#', " ");
    }
    parent
        .and_then(|p| unit.name.strip_prefix(&format!("{}.", p.name)))
        .unwrap_or(&unit.name)
        .to_owned()
}

/// The names a set of units go by together, each told apart from the rest.
#[derive(Debug, Default)]
pub(crate) struct Names(HashMap<UnitId, String>);

impl Names {
    /// Names every path in `paths` apart from the others: each starts as short as it
    /// can be and takes one more container at a time while it reads the same as
    /// another.
    pub(crate) fn apart(paths: impl IntoIterator<Item = (UnitId, Path)>) -> Self {
        let paths: Vec<(UnitId, Path)> = paths
            .into_iter()
            .collect::<HashMap<_, _>>()
            .into_iter()
            .collect();
        let mut depths: Vec<usize> = paths.iter().map(|(_, p)| p.least).collect();
        loop {
            let mut alike: HashMap<String, Vec<usize>> = HashMap::new();
            for (index, (_, path)) in paths.iter().enumerate() {
                alike
                    .entry(path.render(depths[index]))
                    .or_default()
                    .push(index);
            }
            let mut grew = false;
            for same in alike.values().filter(|same| same.len() > 1) {
                for &index in same {
                    if depths[index] < paths[index].1.outer.len() {
                        depths[index] += 1;
                        grew = true;
                    }
                }
            }
            if !grew {
                break;
            }
        }
        Self(
            paths
                .iter()
                .zip(depths)
                .map(|((id, path), depth)| (id.clone(), path.render(depth)))
                .collect(),
        )
    }

    pub(crate) fn get(&self, id: &UnitId) -> Option<&str> {
        self.0.get(id).map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use codeviz_core::System;
    use codeviz_ir::Document;

    use super::*;

    /// Units as `(id, name, kind, parent)`.
    fn system(units: &[(&str, &str, &str, Option<&str>)]) -> System {
        let units: Vec<String> = units
            .iter()
            .map(|(id, name, kind, parent)| {
                let role = match *kind {
                    "fn" | "method" | "closure" | "module_body" => "invocable",
                    _ => "group",
                };
                let parent = parent.map_or_else(String::new, |p| format!(r#""parent": "{p}","#));
                format!(
                    r#"{{ "id": "{id}", "name": "{name}", "kind": "{kind}", "role": "{role}", {parent} "calls": [] }}"#
                )
            })
            .collect();
        let json = format!(
            r#"{{ "schema": "codeviz/0.1", "producer": {{ "name": "t", "version": "0" }}, "units": [{}] }}"#,
            units.join(",")
        );
        System::new([Document::from_json(&json).unwrap()])
    }

    fn rust() -> System {
        system(&[
            ("r:core", "core", "crate", None),
            ("r:core::system", "system", "module", Some("r:core")),
            (
                "r:core::system::System",
                "System",
                "struct",
                Some("r:core::system"),
            ),
            (
                "r:core::system::System::new",
                "new",
                "method",
                Some("r:core::system::System"),
            ),
            (
                "r:core::system::System::unit",
                "unit",
                "method",
                Some("r:core::system::System"),
            ),
            ("r:core::diff", "diff", "module", Some("r:core")),
            ("r:core::diff::run", "run", "fn", Some("r:core::diff")),
            ("r:cli", "cli", "crate", None),
            ("r:cli::diff", "diff", "module", Some("r:cli")),
            ("r:cli::diff::run", "run", "fn", Some("r:cli::diff")),
            (
                "r:cli::diff::Checkout",
                "Checkout",
                "struct",
                Some("r:cli::diff"),
            ),
            (
                "r:cli::diff::Checkout::new",
                "new",
                "method",
                Some("r:cli::diff::Checkout"),
            ),
            ("r:cli::ui", "ui", "module", Some("r:cli")),
            ("r:cli::ui::draw", "draw", "fn", Some("r:cli::ui")),
            (
                "r:cli::ui::draw::{closure#3}",
                "{closure#3}",
                "closure",
                Some("r:cli::ui::draw"),
            ),
            (
                "r:cli::ui::draw::{closure#3}::{closure#0}",
                "{closure#0}",
                "closure",
                Some("r:cli::ui::draw::{closure#3}"),
            ),
        ])
    }

    fn names(system: &System, ids: &[&str]) -> Vec<String> {
        let names = Names::apart(ids.iter().map(|id| {
            let id = UnitId::new(*id);
            let path = Path::of(|id| system.unit(id), &id);
            (id, path)
        }));
        ids.iter()
            .map(|id| names.get(&UnitId::new(*id)).unwrap().to_owned())
            .collect()
    }

    #[test]
    fn a_name_alone_until_another_shows_the_same() {
        let system = rust();
        assert_eq!(
            names(
                &system,
                &[
                    "r:core::system::System::new",
                    "r:core::system::System::unit"
                ]
            ),
            ["new", "unit"]
        );
        assert_eq!(
            names(
                &system,
                &["r:core::system::System::new", "r:cli::diff::Checkout::new"]
            ),
            ["System.new", "Checkout.new"],
            "the type tells them apart"
        );
    }

    #[test]
    fn qualifies_only_as_far_as_it_takes() {
        let system = rust();
        assert_eq!(
            names(
                &system,
                &["r:core::diff::run", "r:cli::diff::run", "r:cli::ui::draw"]
            ),
            ["core.diff.run", "cli.diff.run", "draw"],
            "the same module in two crates takes the crate"
        );
    }

    #[test]
    fn a_closure_goes_by_its_function() {
        let system = rust();
        assert_eq!(
            names(
                &system,
                &[
                    "r:cli::ui::draw::{closure#3}",
                    "r:cli::ui::draw::{closure#3}::{closure#0}"
                ]
            ),
            ["draw › closure 3", "draw › closure 3 › closure 0"]
        );
        let path = Path::of(
            |id| system.unit(id),
            &UnitId::new("r:cli::ui::draw::{closure#3}"),
        );
        assert_eq!(path.full(), "cli.ui.draw › closure 3");
    }

    #[test]
    fn module_code_goes_by_its_module() {
        let system = system(&[
            ("p:shop", "shop", "package", None),
            ("p:shop.routes", "shop.routes", "package", Some("p:shop")),
            (
                "p:shop.routes.orders",
                "shop.routes.orders",
                "module",
                Some("p:shop.routes"),
            ),
            (
                "p:shop.routes.orders.<module>",
                "<module>",
                "module_body",
                Some("p:shop.routes.orders"),
            ),
            (
                "p:shop.services",
                "shop.services",
                "package",
                Some("p:shop"),
            ),
            (
                "p:shop.services.orders",
                "shop.services.orders",
                "module",
                Some("p:shop.services"),
            ),
            (
                "p:shop.services.orders.<module>",
                "<module>",
                "module_body",
                Some("p:shop.services.orders"),
            ),
            (
                "p:shop.services.orders.place",
                "place",
                "fn",
                Some("p:shop.services.orders"),
            ),
        ]);
        assert_eq!(
            names(
                &system,
                &[
                    "p:shop.routes.orders.<module>",
                    "p:shop.services.orders.<module>",
                    "p:shop.services.orders.place",
                ]
            ),
            ["routes.orders", "services.orders", "place"]
        );
        let place = Path::of(
            |id| system.unit(id),
            &UnitId::new("p:shop.services.orders.place"),
        );
        assert_eq!(place.full(), "shop.services.orders.place");
    }
}
