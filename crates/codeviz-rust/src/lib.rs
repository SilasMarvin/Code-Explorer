//! Rust frontend for codeviz.
//!
//! [`analyze`] discovers every Cargo package under a directory, parses its `src/` tree
//! with tree-sitter, and resolves call sites to a [`CallGraph`].
//!
//! Resolution is syntactic: it follows module paths, `use` imports and `Self`, but has
//! no type information. Method calls on receivers other than `self` are matched by
//! name and marked [`Resolution::Inferred`](codeviz_graph::Resolution::Inferred).
//! Calls whose final segment is capitalized are treated as tuple-struct or enum
//! constructors and dropped.

mod discover;
mod extract;
mod resolve;

use std::path::{Path, PathBuf};

use codeviz_graph::{CallGraph, GraphError};

pub use discover::Package;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AnalyzeOptions {
    /// Include `#[cfg(test)]` modules and `#[test]` functions.
    pub include_tests: bool,
}

#[derive(Debug)]
pub struct Analysis {
    pub graph: CallGraph,
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
    #[error(transparent)]
    Graph(#[from] GraphError),
}

/// Builds a call graph for every package under `root`.
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
            let display_path = file
                .path
                .strip_prefix(root)
                .unwrap_or(&file.path)
                .to_path_buf();
            extractor.extract(&source, &display_path, &file.module)?;
            files_parsed += 1;
        }
    }

    let crate_names = packages.iter().map(|p| p.crate_name.clone()).collect();
    let graph = resolve::resolve(&extractor.finish(), &crate_names)?;
    Ok(Analysis {
        graph,
        packages,
        files_parsed,
    })
}
