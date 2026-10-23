//! Call resolution from rust-analyzer: `rust-analyzer scip` indexes the workspace with
//! full type information, and every reference in it names the definition it resolves
//! to. Only what the producer needs is decoded: where each reference sits, and where
//! each symbol is defined.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

use prost::Message;

use crate::Error;

/// A position in a file: its path relative to the analyzed root, then the zero-based
/// line and byte column, as tree-sitter counts them.
pub(crate) type Position = (String, u32, u32);

/// What rust-analyzer resolved.
#[derive(Debug, Default)]
pub(crate) struct Scip {
    /// Each reference's position, to the symbols it names. One spot can name more than
    /// one: a field shorthand names the field and the binding.
    references: HashMap<Position, Vec<String>>,
    /// Each symbol defined in the analyzed source, to where its name is written.
    pub(crate) definitions: HashMap<String, Vec<Position>>,
}

impl Scip {
    /// Runs rust-analyzer over `root`. Build scripts and proc macros are built into
    /// `target_dir`, or the workspace's own target directory when that is `None`.
    ///
    /// # Errors
    /// Fails if rust-analyzer cannot be run, fails, or writes an index this cannot read.
    pub(crate) fn index(root: &Path, target_dir: Option<&Path>) -> Result<Self, Error> {
        // Unique per run, so analyses running side by side keep their own index.
        static RUNS: AtomicUsize = AtomicUsize::new(0);
        let run = RUNS.fetch_add(1, Ordering::Relaxed);
        let output =
            std::env::temp_dir().join(format!("codeviz-{}-{run}.scip", std::process::id()));
        let mut command = Command::new(rust_analyzer());
        command.arg("scip").arg(root).arg("--output").arg(&output);
        if let Some(dir) = target_dir {
            command.env("CARGO_TARGET_DIR", dir);
        }
        let run = command.output().map_err(Error::RustAnalyzer)?;
        let bytes = std::fs::read(&output);
        let _ = std::fs::remove_file(&output);
        if !run.status.success() {
            // Its log runs long; the cause is at the end.
            let log = String::from_utf8_lossy(&run.stderr);
            let tail: Vec<&str> = log.trim().lines().rev().take(5).collect();
            return Err(Error::RustAnalyzerFailed(
                tail.into_iter().rev().collect::<Vec<_>>().join("\n"),
            ));
        }
        let bytes = bytes.map_err(|source| Error::Io {
            path: output,
            source,
        })?;
        Self::decode(&bytes)
    }

    fn decode(bytes: &[u8]) -> Result<Self, Error> {
        let index = Index::decode(bytes).map_err(Error::Scip)?;
        let mut scip = Self::default();
        for document in index.documents {
            let utf8 = matches!(document.position_encoding, 0 | UTF8);
            if !utf8 {
                continue;
            }
            for occurrence in document.occurrences {
                let (Some(&line), Some(&column)) =
                    (occurrence.range.first(), occurrence.range.get(1))
                else {
                    continue;
                };
                let (Ok(line), Ok(column)) = (u32::try_from(line), u32::try_from(column)) else {
                    continue;
                };
                let position = (document.relative_path.clone(), line, column);
                let symbols = if occurrence.symbol_roles & DEFINITION == 0 {
                    scip.references.entry(position).or_default()
                } else if occurrence.symbol.starts_with("local ") {
                    continue;
                } else {
                    scip.definitions
                        .entry(occurrence.symbol)
                        .or_default()
                        .push(position);
                    continue;
                };
                symbols.push(occurrence.symbol);
            }
        }
        Ok(scip)
    }

    /// What the reference at `position` names, or `None` if rust-analyzer did not
    /// resolve one there.
    pub(crate) fn symbols_at(&self, position: &Position) -> Option<&[String]> {
        self.references.get(position).map(Vec::as_slice)
    }
}

/// The rust-analyzer to run: `$RUST_ANALYZER`, or the one on the path.
fn rust_analyzer() -> PathBuf {
    std::env::var_os("RUST_ANALYZER").map_or_else(|| PathBuf::from("rust-analyzer"), PathBuf::from)
}

/// Whether rust-analyzer can be run.
#[must_use]
pub fn rust_analyzer_available() -> bool {
    Command::new(rust_analyzer())
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// `crate module/Type#method().`: a method or function, `(+1)` when disambiguated.
pub(crate) fn is_function(symbol: &str) -> bool {
    !symbol.starts_with("local ") && symbol.ends_with(").")
}

/// For a method declared in a trait, `module/Trait#method().`, the trait's name and
/// the method's.
pub(crate) fn trait_method(symbol: &str) -> Option<(&str, &str)> {
    let (owner, method) = symbol.rsplit_once('#')?;
    let name = owner.rsplit(['/', '#', ']', ' ']).next()?;
    let method = method.split('(').next()?;
    // An impl's methods are `impl#[Type][Trait]method().`, not `Trait#method().`.
    (!owner.ends_with("impl") && !name.is_empty() && !method.is_empty())
        .then(|| (name.trim_matches('`'), method.trim_matches('`')))
}

const UTF8: i32 = 1;
const DEFINITION: i32 = 1;

// The parts of scip.proto the producer reads, with its field numbers.

#[derive(Clone, PartialEq, Message)]
struct Index {
    #[prost(message, repeated, tag = "2")]
    documents: Vec<Document>,
}

#[derive(Clone, PartialEq, Message)]
struct Document {
    #[prost(string, tag = "1")]
    relative_path: String,
    #[prost(message, repeated, tag = "2")]
    occurrences: Vec<Occurrence>,
    #[prost(int32, tag = "6")]
    position_encoding: i32,
}

#[derive(Clone, PartialEq, Message)]
struct Occurrence {
    /// `[line, column, end column]`, or `[line, column, end line, end column]`.
    #[prost(int32, repeated, tag = "1")]
    range: Vec<i32>,
    #[prost(string, tag = "2")]
    symbol: String,
    #[prost(int32, tag = "3")]
    symbol_roles: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_symbol_shapes() {
        let symbol = |d: &str| format!("rust-analyzer cargo app 0.1.0 {d}");
        assert!(is_function(&symbol("shapes/impl#[Circle][Shape]area().")));
        assert!(is_function(&symbol("helper(+1).")));
        assert!(!is_function(&symbol("shapes/Circle#radius.")));
        assert!(!is_function("local 4"));
        assert_eq!(
            trait_method(&symbol("shapes/Shape#area().")),
            Some(("Shape", "area"))
        );
        assert_eq!(
            trait_method(&symbol("shapes/impl#[Circle][Shape]area().")),
            None
        );
        assert_eq!(trait_method(&symbol("shapes/impl#[Circle]new().")), None);
        assert_eq!(trait_method(&symbol("shapes/helper().")), None);
    }
}
