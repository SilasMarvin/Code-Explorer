//! Rust producer for codeviz: builds an IR [`Document`] from Rust source with tree-sitter.
//!
//! [`analyze`] discovers every Cargo package under a directory and parses its `src/`
//! tree. With [`Resolver::RustAnalyzer`], call sites resolve to what rust-analyzer says
//! they call. Otherwise, and for any call it leaves unresolved, they resolve
//! syntactically: module paths, `use` imports, `Self`, `self` and declared types are
//! followed, through smart pointers and into closures. A method call on a value whose
//! type they do not reveal is left unresolved: it is far more often `Option::map` or
//! `Vec::len` than a method of ours with the same name. A path matched only by its tail
//! is marked [`Certainty::Inferred`]. Calls whose final segment is capitalized are
//! treated as tuple-struct or enum constructors and dropped.
//!
//! Control flow is mapped onto IR steps: `if`/`match` → branch, loops → loop,
//! `return`/`Err(..)`/`panic!` → exit, and `?` (or a tail call in a function returning
//! `Result`) marks the call as propagating its error. Functions passed by name and
//! closures passed as arguments become `register` invocations. Axum
//! `.route("/path", get(handler))` calls become HTTP entries.
//!
//! [`Certainty::Inferred`]: codeviz_ir::Certainty::Inferred

mod body;
mod discover;
mod extract;
mod hash;
mod resolve;
mod scip;
mod syntax;

use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
};

use codeviz_ir::{Document, Producer};

pub use discover::Crate;
pub use scip::rust_analyzer_available;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AnalyzeOptions {
    /// Include `#[cfg(test)]` modules and `#[test]` functions.
    pub include_tests: bool,
    pub resolver: Resolver,
}

/// How call sites find what they call.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Resolver {
    /// From the syntax alone: fast, but a method call on a value of unknown type is
    /// left unresolved.
    #[default]
    Syntax,
    /// From `rust-analyzer scip`, which type-checks the workspace: slow, since build
    /// scripts and proc macros are built first. They are built into `target_dir`, or
    /// the workspace's own target directory when that is `None`.
    RustAnalyzer { target_dir: Option<PathBuf> },
}

#[derive(Debug)]
pub struct Analysis {
    pub document: Document,
    pub crates: Vec<Crate>,
    pub files_parsed: usize,
    /// What was analyzed, and what was skipped or resolved less well than asked, in
    /// lines to show whoever ran it.
    pub notes: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no Cargo package with a library or binary found under {}", .0.display())]
    NoPackages(PathBuf),
    #[error("failed to read {}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse manifest {}", path.display())]
    Manifest {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
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
    #[error("cannot run rust-analyzer")]
    RustAnalyzer(#[source] std::io::Error),
    #[error("cannot read rust-analyzer's index")]
    Scip(#[source] prost::DecodeError),
}

/// Builds an IR document for every crate under `root`, or, when `root` is inside a
/// package, for the part of it under `root`.
///
/// # Errors
/// Fails if no crate is found, or a source file cannot be read or parsed, or
/// rust-analyzer was asked for and cannot be run. A workspace rust-analyzer fails on
/// is resolved from the syntax, and a note says so.
pub fn analyze(root: &Path, options: &AnalyzeOptions) -> Result<Analysis, Error> {
    let mut discovery = discover::crates(root)?;
    if discovery.crates.is_empty()
        && let Some(package) = discover::enclosing_package(root)
    {
        discovery = discover::crates(package)?;
    }
    let mut notes = discovery.notes;
    let crates = discovery.crates;
    let files = discover::source_files(&crates, options.include_tests)?;
    let files: Vec<_> = files
        .into_iter()
        .filter(|f| f.path.starts_with(root))
        .collect();
    if files.is_empty() {
        return Err(Error::NoPackages(root.to_path_buf()));
    }
    let crates: Vec<Crate> = crates
        .into_iter()
        .filter(|c| files.iter().any(|f| f.module.first() == Some(&c.name)))
        .collect();
    notes.insert(0, summary(root, &crates, files.len()));
    let scip = match &options.resolver {
        Resolver::Syntax => None,
        Resolver::RustAnalyzer { target_dir } => Some(scip::Scip::index(
            root,
            &crates,
            target_dir.as_deref(),
            &mut notes,
        )?),
    };

    let mut extractor = extract::Extractor::new(options.include_tests)?;
    for file in &files {
        let source = std::fs::read(&file.path).map_err(|source| Error::Io {
            path: file.path.clone(),
            source,
        })?;
        let relative = file.path.strip_prefix(root).unwrap_or(&file.path);
        extractor.extract(&source, relative, &file.module)?;
    }

    let mut document = Document::new(Producer {
        name: env!("CARGO_PKG_NAME").to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    });
    document.root = Some(format!("file://{}", root.display()));
    let crate_names = crates.iter().map(|c| c.name.clone()).collect();
    resolve::assemble(
        &mut extractor.finish(),
        &crate_names,
        scip.as_ref(),
        &mut document,
    );
    document.canonicalize();
    Ok(Analysis {
        document,
        crates,
        files_parsed: files.len(),
        notes,
    })
}

/// `3 crates (2 libraries, 1 binary) in 2 packages, 41 files`, and the part of a
/// package `root` is inside.
fn summary(root: &Path, crates: &[Crate], files: usize) -> String {
    let mut packages: Vec<&Path> = crates.iter().map(|c| c.package.as_path()).collect();
    packages.dedup();
    let libraries = crates.iter().filter(|c| c.library).count();
    let plural =
        |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut line = format!(
        "{} ({}, {}) in {}, {}",
        plural(crates.len(), "crate", "crates"),
        plural(libraries, "library", "libraries"),
        plural(crates.len() - libraries, "binary", "binaries"),
        plural(packages.len(), "package", "packages"),
        plural(files, "file", "files"),
    );
    if let ([package], Some(krate)) = (packages.as_slice(), crates.first())
        && let Ok(inside) = root.strip_prefix(package)
        && !inside.as_os_str().is_empty()
    {
        let _ = write!(
            line,
            ": only {} of package {}",
            inside.display(),
            krate.package_name
        );
    }
    line
}
