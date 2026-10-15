//! Rust producer for codeviz: builds an IR [`Document`] from Rust source with tree-sitter.
//!
//! [`analyze`] discovers every Cargo package under a directory, parses its `src/`
//! tree, and resolves call sites syntactically: module paths, `use` imports, `Self`
//! and `self` are followed, but there is no type information. Method calls on other
//! receivers are matched by name and marked [`Certainty::Inferred`]. Calls whose final
//! segment is capitalized are treated as tuple-struct or enum constructors and dropped.
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
mod syntax;

use std::path::{Path, PathBuf};

use codeviz_ir::{Document, Producer};

pub use discover::Package;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AnalyzeOptions {
    /// Include `#[cfg(test)]` modules and `#[test]` functions.
    pub include_tests: bool,
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
}

/// Builds an IR document for every package under `root`.
///
/// # Errors
/// Fails if no package is found, or a manifest or source file cannot be read or parsed.
pub fn analyze(root: &Path, options: AnalyzeOptions) -> Result<Analysis, Error> {
    let packages = discover::packages(root)?;
    if packages.is_empty() {
        return Err(Error::NoPackages(root.to_path_buf()));
    }

    let mut extractor = extract::Extractor::new(options)?;
    let mut files_parsed = 0;
    for package in &packages {
        for file in discover::source_files(package)? {
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
    let crate_names = packages.iter().map(|p| p.crate_name.clone()).collect();
    resolve::assemble(&extractor.finish(), &crate_names, &mut document);
    document.canonicalize();
    Ok(Analysis {
        document,
        packages,
        files_parsed,
    })
}
