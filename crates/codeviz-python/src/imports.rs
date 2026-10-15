//! `import` and `from … import …` statements: the names they bind and the modules they
//! load.

use tree_sitter::Node;

use crate::syntax::{field, named_children, text};

/// The module an import statement appears in.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Importer<'a> {
    pub(crate) module: &'a str,
    pub(crate) is_package: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Imported {
    Module(String),
    /// `from module import name`: a definition, a re-export, or a submodule.
    Name {
        module: String,
        name: String,
    },
}

/// A module the statement executes, with the text it was written as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Load {
    pub(crate) text: String,
    pub(crate) module: String,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Import {
    pub(crate) bindings: Vec<(String, Imported)>,
    pub(crate) loads: Vec<Load>,
}

/// Parses an `import_statement` or `import_from_statement`. Other nodes, wildcard
/// imports, and relative imports that climb above the top-level package yield nothing.
pub(crate) fn parse(node: Node<'_>, source: &str, importer: Importer<'_>) -> Import {
    let mut import = Import::default();
    let names = node
        .children_by_field_name("name", &mut node.walk())
        .collect::<Vec<_>>();
    match node.kind() {
        "import_statement" => {
            for name in names {
                let (dotted, alias) = split_alias(name, source);
                // `import a.b` binds `a`; `import a.b as c` binds `c` to `a.b`.
                let top = dotted.split('.').next().unwrap_or(dotted);
                let (local, bound) = alias.map_or((top, top), |alias| (alias, dotted));
                import
                    .bindings
                    .push((local.to_owned(), Imported::Module(bound.to_owned())));
                import.loads.push(Load {
                    text: dotted.to_owned(),
                    module: dotted.to_owned(),
                });
            }
        }
        "import_from_statement" => {
            let Some(module_name) = field(node, "module_name") else {
                return import;
            };
            let written = text(module_name, source);
            let Some(base) = absolute(module_name, source, importer) else {
                return import;
            };
            import.loads.push(Load {
                text: written.to_owned(),
                module: base.clone(),
            });
            for name in names {
                let (dotted, alias) = split_alias(name, source);
                let separator = if written.ends_with('.') { "" } else { "." };
                import.loads.push(Load {
                    text: format!("{written}{separator}{dotted}"),
                    module: format!("{base}.{dotted}"),
                });
                import.bindings.push((
                    alias.unwrap_or(dotted).to_owned(),
                    Imported::Name {
                        module: base.clone(),
                        name: dotted.to_owned(),
                    },
                ));
            }
        }
        _ => {}
    }
    import
}

/// `a.b as c` → (`a.b`, Some(`c`)); `a.b` → (`a.b`, None).
fn split_alias<'s>(node: Node<'_>, source: &'s str) -> (&'s str, Option<&'s str>) {
    if node.kind() == "aliased_import" {
        let name = field(node, "name").map_or("", |name| text(name, source));
        let alias = field(node, "alias").map(|alias| text(alias, source));
        (name, alias)
    } else {
        (text(node, source), None)
    }
}

/// Resolves the module of a `from` import, following leading dots relative to the
/// importer's package.
fn absolute(module_name: Node<'_>, source: &str, importer: Importer<'_>) -> Option<String> {
    if module_name.kind() != "relative_import" {
        return Some(text(module_name, source).to_owned());
    }
    let children = named_children(module_name);
    let dots = children
        .iter()
        .find(|child| child.kind() == "import_prefix")
        .map_or(0, |prefix| text(*prefix, source).len());
    let rest = children
        .iter()
        .find(|child| child.kind() == "dotted_name")
        .map(|rest| text(*rest, source));

    let mut package: Vec<&str> = importer.module.split('.').collect();
    if !importer.is_package {
        package.pop();
    }
    for _ in 1..dots {
        package.pop()?;
    }
    package.extend(rest);
    (!package.is_empty()).then(|| package.join("."))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_one(code: &str, module: &str, is_package: bool) -> Import {
        let mut parser = tree_sitter::Parser::new();
        assert!(
            parser
                .set_language(&tree_sitter_python::LANGUAGE.into())
                .is_ok()
        );
        let Some(tree) = parser.parse(code, None) else {
            panic!("parse failed");
        };
        let Some(statement) = tree.root_node().named_child(0) else {
            panic!("no statement");
        };
        parse(statement, code, Importer { module, is_package })
    }

    fn name(module: &str, name: &str) -> Imported {
        Imported::Name {
            module: module.into(),
            name: name.into(),
        }
    }

    #[test]
    fn plain_and_aliased_imports() {
        let import = parse_one("import a.b, c.d as e", "x", false);
        assert_eq!(
            import.bindings,
            [
                ("a".into(), Imported::Module("a".into())),
                ("e".into(), Imported::Module("c.d".into())),
            ]
        );
        let loaded: Vec<_> = import.loads.iter().map(|l| l.module.as_str()).collect();
        assert_eq!(loaded, ["a.b", "c.d"]);
    }

    #[test]
    fn relative_imports_climb_packages() {
        let import = parse_one(
            "from ..services import users as svc",
            "shop.routes.users",
            false,
        );
        assert_eq!(
            import.bindings,
            [("svc".into(), name("shop.services", "users"))]
        );
        assert_eq!(
            import.loads,
            [
                Load {
                    text: "..services".into(),
                    module: "shop.services".into()
                },
                Load {
                    text: "..services.users".into(),
                    module: "shop.services.users".into()
                },
            ]
        );

        let import = parse_one("from . import validation", "shop.services", true);
        assert_eq!(
            import.bindings,
            [("validation".into(), name("shop.services", "validation"))]
        );
        assert_eq!(import.loads[1].text, ".validation");
    }

    #[test]
    fn relative_import_above_the_top_level_is_ignored() {
        assert_eq!(
            parse_one("from ... import x", "a.b", false),
            Import::default()
        );
    }
}
