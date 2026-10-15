//! Buildkite frontend for codeviz.
//!
//! [`import`] reads a pipeline definition (`pipeline.yml`) and describes it as a codeviz
//! [`Document`]: the pipeline is an invocable unit whose body is a `dag` of its steps,
//! and each step is a unit that calls what it runs.
//!
//! - Ordering (`wait`, `block`, `depends_on`, groups) becomes `dag` edges.
//! - `if` becomes a `branch` around the step's call, with the condition as its guard.
//! - A `block` step is a gate unit followed by a branch: if nobody unblocks it, the
//!   build ends there.
//! - A command line is a call; a line that runs a repository script targets an
//!   external `script:<path>` unit, other tools stay unresolved.
//! - Plugins are calls to external `buildkite-plugin:<name>` units, and triggers call
//!   the triggered pipeline, spawning it when `async`.
//! - Per-job settings with no IR equivalent (parallelism, matrix, retry, agents,
//!   concurrency, artifacts) are attributes.

mod attributes;
mod emit;
mod graph;
mod ids;
mod model;

use std::path::{Path, PathBuf};

use codeviz_ir::Document;

/// The unit id scheme for pipelines and their steps.
pub const SCHEME: &str = "buildkite";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read {}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("invalid pipeline YAML")]
    Yaml(#[from] Box<serde_saphyr::Error>),
    #[error("`{0}` is not a pipeline slug (lowercase letters, digits and `-`)")]
    InvalidSlug(String),
    #[error("step key `{0}` is empty or contains whitespace")]
    InvalidKey(String),
    #[error("step key `{0}` is used twice")]
    DuplicateKey(String),
    #[error("step `{step}` depends on unknown step `{depends_on}`")]
    UnknownDependency { step: String, depends_on: String },
    #[error("step {step}: unsupported {what}")]
    Unsupported { step: String, what: String },
}

/// Describes the pipeline `slug` defined by `yaml`.
///
/// # Errors
/// Fails if the YAML is malformed, a step has an unknown type, keys are invalid or
/// repeated, or `depends_on` names a step that does not exist.
pub fn import(yaml: &str, slug: &str) -> Result<Document, Error> {
    build(yaml, slug, None)
}

/// Like [`import`], reading `path` and recording it as every unit's location.
///
/// # Errors
/// As [`import`], or if `path` cannot be read.
pub fn import_file(path: &Path, slug: &str) -> Result<Document, Error> {
    let yaml = std::fs::read_to_string(path).map_err(|source| Error::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let uri = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    build(&yaml, slug, Some(&uri))
}

fn build(yaml: &str, slug: &str, uri: Option<&str>) -> Result<Document, Error> {
    if slug.is_empty()
        || !slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(Error::InvalidSlug(slug.to_owned()));
    }
    let mut pipeline = model::parse(yaml)?;
    ids::assign(&mut pipeline.steps)?;
    let nodes = graph::schedule(&pipeline.steps)?;
    Ok(emit::Emitter::new(slug, uri).document(&pipeline, &nodes))
}
