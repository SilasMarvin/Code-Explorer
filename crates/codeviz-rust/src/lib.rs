//! Rust producer for codeviz: builds an IR [`Document`] from Rust source with tree-sitter.
//!
//! [`analyze`] discovers every Cargo package under a directory and parses its `src/`
//! tree. With [`Resolver::RustAnalyzer`], call sites resolve to what rust-analyzer says
//! they call. Otherwise, and for any call it leaves unresolved, they resolve
//! syntactically: module paths, `use` imports and re-exports, `Self`, `self`, declared
//! types, return types and closure captures are followed. A method call on a receiver
//! of unknown type is matched by name only if the name is not a common std method and
//! one type in a crate the caller depends on has it; the match is marked
//! [`Certainty::Inferred`]. Calls whose final segment is capitalized are treated as
//! tuple-struct or enum constructors and dropped.
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
mod std_methods;
mod syntax;

use std::path::{Path, PathBuf};

use codeviz_ir::{Document, Producer};

pub use discover::Package;
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
    /// matched by name.
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
    pub packages: Vec<Package>,
    pub files_parsed: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no Cargo packages with a src/ directory found under {}", .0.display())]
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
    #[error("rust-analyzer failed: {0}")]
    RustAnalyzerFailed(String),
    #[error("cannot read rust-analyzer's index")]
    Scip(#[source] prost::DecodeError),
}

/// Builds an IR document for every package under `root`.
///
/// # Errors
/// Fails if no package is found, or a manifest or source file cannot be read or parsed,
/// or rust-analyzer was asked for and failed.
pub fn analyze(root: &Path, options: &AnalyzeOptions) -> Result<Analysis, Error> {
    let packages = discover::packages(root)?;
    if packages.is_empty() {
        return Err(Error::NoPackages(root.to_path_buf()));
    }
    let scip = match &options.resolver {
        Resolver::Syntax => None,
        Resolver::RustAnalyzer { target_dir } => {
            Some(scip::Scip::index(root, target_dir.as_deref())?)
        }
    };

    let mut extractor = extract::Extractor::new(options.include_tests)?;
    let mut files_parsed = 0;
    for package in &packages {
        for file in discover::source_files(package, options.include_tests)? {
            let source = std::fs::read(&file.path).map_err(|source| Error::Io {
                path: file.path.clone(),
                source,
            })?;
            let relative = file.path.strip_prefix(root).unwrap_or(&file.path);
            extractor.extract(&source, relative, &file.module)?;
            files_parsed += 1;
        }
    }

    let mut document = Document::new(Producer {
        name: env!("CARGO_PKG_NAME").to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
    });
    document.root = Some(format!("file://{}", root.display()));
    let crates = discover::visible_crates(&packages, options.include_tests);
    resolve::assemble(
        &mut extractor.finish(),
        &crates,
        scip.as_ref(),
        &mut document,
    );
    document.canonicalize();
    Ok(Analysis {
        document,
        packages,
        files_parsed,
    })
}
