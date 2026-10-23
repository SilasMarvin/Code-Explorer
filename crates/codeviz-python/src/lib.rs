//! Python frontend for codeviz.
//!
//! [`analyze`] parses every `.py` file under a directory with tree-sitter and emits a
//! codeviz IR [`Document`]:
//!
//! - Modules and packages are `group` units named by their dotted path
//!   (`python:shop.store`); classes are `group` units inside them.
//! - Functions, methods, nested functions and lambdas are `invocable` units with
//!   `__qualname__`-style ids: `python:shop.store.Store.insert_order`,
//!   `python:shop.audit.audited.<locals>.decorate`, `…<locals>.<lambda#0>`.
//! - A module's top-level statements, when they call anything, run as the invocable
//!   unit `python:<module>.<module>`: definition-time work such as decorators, route
//!   registration and imports of other analyzed modules lives there.
//! - Module-level variables that functions are registered on (`router = APIRouter()`)
//!   become `channel` units.
//!
//! Resolution is syntactic: it follows scopes, imports, classes, `self`, `cls` and
//! `super()`, and matches other method calls by name (`inferred`). The `lower` module
//! documents how Python control flow maps onto IR steps.

mod discover;
mod hash;
mod imports;
mod index;
mod lower;
mod resolve;
mod syntax;

use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
};

use codeviz_ir::{Document, Producer, Role, Unit, UnitId};
use tree_sitter::Parser;

use crate::{
    index::{Index, Parsed, module_body},
    lower::{Lowered, lower},
    resolve::Resolver,
};

/// The id scheme of every unit this frontend emits.
pub const SCHEME: &str = "python";

/// Registration can make a variable a channel, which can make further arguments
/// registrations; this bounds the rounds needed to settle.
const MAX_CHANNEL_ROUNDS: usize = 8;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no Python files found under {}", .0.display())]
    NoSources(PathBuf),
    #[error("failed to read {}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to walk {}", root.display())]
    Walk {
        root: PathBuf,
        #[source]
        source: walkdir::Error,
    },
    #[error("tree-sitter could not parse {}", .0.display())]
    Parse(PathBuf),
    #[error("incompatible tree-sitter grammar")]
    Language(#[from] tree_sitter::LanguageError),
}

/// Analyzes every Python file under `root`. Locations are relative to `root`.
///
/// # Errors
/// Fails if no Python file is found, or a file cannot be read or parsed.
pub fn analyze(root: &Path) -> Result<Document, Error> {
    Ok(analyze_checked(root)?.document)
}

/// A document, and the files it was built from that did not wholly parse.
#[derive(Debug)]
pub struct Analysis {
    pub document: Document,
    /// Files tree-sitter could not wholly parse, such as one saved halfway through an
    /// edit. What it could parse of them is in the document.
    pub syntax_errors: Vec<SyntaxError>,
}

/// Where a file stops parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    /// Relative to the analyzed directory.
    pub path: PathBuf,
    /// The first line with an error, from 1.
    pub line: usize,
}

/// [`analyze`], also saying which files did not wholly parse.
///
/// # Errors
/// As [`analyze`].
pub fn analyze_checked(root: &Path) -> Result<Analysis, Error> {
    let files = discover::source_files(root)?;
    if files.is_empty() {
        return Err(Error::NoSources(root.to_path_buf()));
    }
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_python::LANGUAGE.into())?;
    let mut parsed = Vec::with_capacity(files.len());
    let mut syntax_errors = Vec::new();
    for file in files {
        let source = std::fs::read_to_string(&file.path).map_err(|source| Error::Io {
            path: file.path.clone(),
            source,
        })?;
        let tree = parser
            .parse(&source, None)
            .ok_or_else(|| Error::Parse(file.path.clone()))?;
        if let Some(error) = first_error(tree.root_node()) {
            syntax_errors.push(SyntaxError {
                path: file
                    .path
                    .strip_prefix(root)
                    .unwrap_or(&file.path)
                    .to_path_buf(),
                line: error.start_position().row + 1,
            });
        }
        parsed.push(Parsed { file, source, tree });
    }

    let index = Index::build(&parsed);
    let resolver = Resolver::new(&index);
    let (lowered, channels) = lower_all(&resolver);
    Ok(Analysis {
        document: assemble(&index, lowered, &channels),
        syntax_errors,
    })
}

/// The first place tree-sitter could not make sense of, if there is one: the
/// innermost error or missing token on the way down to the first.
fn first_error(root: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
    if !root.has_error() {
        return None;
    }
    let mut cursor = root.walk();
    let mut node = root;
    loop {
        if node.is_error() || node.is_missing() {
            return Some(node);
        }
        match node
            .children(&mut cursor)
            .find(tree_sitter::Node::has_error)
        {
            Some(child) => node = child,
            None => return Some(node),
        }
    }
}

/// Lowers every unit, repeating until the set of channel variables is stable.
fn lower_all(resolver: &Resolver<'_, '_>) -> (Vec<Lowered>, BTreeSet<UnitId>) {
    let executables = &resolver.index().executables;
    let mut channels = BTreeSet::new();
    let mut round = 0;
    loop {
        let lowered: Vec<_> = executables
            .iter()
            .map(|executable| lower(resolver, &channels, executable))
            .collect();
        let vias: BTreeSet<_> = lowered
            .iter()
            .flat_map(|l| l.vias.iter().cloned())
            .collect();
        round += 1;
        if vias == channels || round == MAX_CHANNEL_ROUNDS {
            return (lowered, vias);
        }
        channels = vias;
    }
}

fn assemble(index: &Index<'_>, lowered: Vec<Lowered>, channels: &BTreeSet<UnitId>) -> Document {
    let mut document = Document::new(Producer {
        name: "codeviz-python".into(),
        version: env!("CARGO_PKG_VERSION").into(),
    });
    let live_modules = live_module_bodies(&lowered);
    let mut units: Vec<Unit> = index.units.clone();

    for channel in channels {
        units.push(channel_unit(index, channel));
    }
    let mut positions: HashMap<UnitId, usize> = units
        .iter()
        .enumerate()
        .map(|(position, unit)| (unit.id.clone(), position))
        .collect();

    for lowered in lowered {
        if let Some(module) = &lowered.module {
            if !live_modules.contains(module) {
                continue;
            }
            positions.insert(lowered.unit.clone(), units.len());
            units.push(module_body_unit(index, module));
        }
        document.entries.extend(lowered.entries.iter().cloned());
        let Some(&position) = positions.get(&lowered.unit) else {
            continue;
        };
        let (calls, body) = lowered.finish(|call| {
            call.loads
                .as_ref()
                .is_none_or(|module| live_modules.contains(module))
        });
        units[position].calls = calls;
        units[position].body = body;
    }

    document.units = units;
    document.canonicalize();
    document
}

/// Modules whose top-level code does something: calls anything other than an import,
/// declares an entry, or imports a module that does.
fn live_module_bodies(lowered: &[Lowered]) -> BTreeSet<String> {
    let modules: Vec<(&str, &Lowered)> = lowered
        .iter()
        .filter_map(|l| Some((l.module.as_deref()?, l)))
        .collect();
    let mut live: BTreeSet<String> = modules
        .iter()
        .filter(|(_, l)| !l.entries.is_empty() || l.calls.iter().any(|call| call.loads.is_none()))
        .map(|(module, _)| (*module).to_owned())
        .collect();
    loop {
        let newly_live: Vec<String> = modules
            .iter()
            .filter(|(module, _)| !live.contains(*module))
            .filter(|(_, l)| {
                l.calls
                    .iter()
                    .any(|call| call.loads.as_ref().is_some_and(|m| live.contains(m)))
            })
            .map(|(module, _)| (*module).to_owned())
            .collect();
        if newly_live.is_empty() {
            return live;
        }
        live.extend(newly_live);
    }
}

fn module_body_unit(index: &Index<'_>, module: &str) -> Unit {
    let module_id = UnitId::from_parts(SCHEME, module);
    let module_unit = index.units.iter().find(|unit| unit.id == module_id);
    let mut unit = Unit::new(
        module_body(module),
        "<module>",
        "module_body",
        Role::Invocable,
    );
    unit.parent = Some(module_id);
    unit.location = module_unit.and_then(|m| m.location.clone());
    unit.content_hash = module_unit.and_then(|m| m.content_hash.clone());
    unit
}

fn channel_unit(index: &Index<'_>, channel: &UnitId) -> Unit {
    let variable = index.variables.get(channel);
    let name = variable.map_or_else(
        || channel.path().unwrap_or_default().to_owned(),
        |variable| variable.name.clone(),
    );
    let mut unit = Unit::new(channel.clone(), name, "object", Role::Channel);
    if let Some(variable) = variable {
        unit.parent = Some(variable.module.clone());
        unit.location = Some(variable.location.clone());
        if let Some(constructor) = &variable.constructor {
            unit.attributes
                .insert("type".into(), constructor.clone().into());
        }
    }
    unit
}
