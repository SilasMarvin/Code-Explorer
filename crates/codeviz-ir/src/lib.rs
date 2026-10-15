//! The codeviz intermediate representation (IR).
//!
//! A [`Document`] is a snapshot of one system: its [`Unit`]s, the [`Invocation`]s
//! between them, optional control flow ([`Step`]s) over those invocations, and the
//! [`Entry`] points where execution starts. The format is specified in `docs/ir.md`;
//! these types are its source of truth.

mod document;
mod validate;

pub use document::{
    Arm, CallKey, Certainty, ContentHash, DagNode, Dispatch, Document, Entry, ExitKind, Invocation,
    Location, Mode, Producer, Range, Revision, Role, SCHEMA, Scalar, Step, Target, Unit, UnitId,
};
pub use validate::{Diagnostic, Rule, Severity, validate};

/// Errors reading or writing a document.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid document JSON")]
    Json(#[from] serde_json::Error),
    #[error("unsupported schema `{found}`; this build reads `{SCHEMA}`")]
    Schema { found: String },
}

/// The JSON Schema for [`Document`].
#[must_use]
pub fn json_schema() -> serde_json::Value {
    serde_json::to_value(schemars::schema_for!(Document)).unwrap_or_default()
}
