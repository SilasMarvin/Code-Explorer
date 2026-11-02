use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};

use crate::Error;

/// The schema identifier this build writes.
pub const SCHEMA: &str = "codeviz/0.1";
const SCHEMA_FAMILY: &str = "codeviz/";
const SCHEMA_MAJOR: &str = "0";

/// A snapshot of one system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    /// `codeviz/<major>.<minor>`.
    pub schema: String,
    pub producer: Producer,
    /// Base for relative [`Location`] URIs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<Revision>,
    #[serde(default)]
    pub units: Vec<Unit>,
    #[serde(default)]
    pub entries: Vec<Entry>,
    /// Correspondences between units, usually across documents. Experimental.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
}

impl Document {
    #[must_use]
    pub fn new(producer: Producer) -> Self {
        Self {
            schema: SCHEMA.to_owned(),
            producer,
            root: None,
            revision: None,
            units: Vec::new(),
            entries: Vec::new(),
            links: Vec::new(),
        }
    }

    /// Parses a document, rejecting a different major schema version.
    ///
    /// # Errors
    /// Fails on malformed JSON or an unsupported schema.
    pub fn from_json(json: &str) -> Result<Self, Error> {
        let document: Self = serde_json::from_str(json)?;
        let major = document
            .schema
            .strip_prefix(SCHEMA_FAMILY)
            .and_then(|version| version.split('.').next());
        if major != Some(SCHEMA_MAJOR) {
            return Err(Error::Schema {
                found: document.schema,
            });
        }
        Ok(document)
    }

    /// Serializes in canonical order so snapshots diff cleanly.
    ///
    /// # Errors
    /// Fails only if serialization itself fails.
    pub fn to_json(&self) -> Result<String, Error> {
        let mut canonical = self.clone();
        canonical.canonicalize();
        Ok(serde_json::to_string_pretty(&canonical)?)
    }

    /// Sorts units by id and entries by unit and label. Call order is semantic and kept.
    pub fn canonicalize(&mut self) {
        self.units.sort_by(|a, b| a.id.cmp(&b.id));
        self.entries
            .sort_by(|a, b| (&a.unit, &a.label).cmp(&(&b.unit, &b.label)));
        self.links
            .sort_by(|a, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));
    }

    #[must_use]
    pub fn unit(&self, id: &UnitId) -> Option<&Unit> {
        self.units.iter().find(|unit| &unit.id == id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Producer {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Revision {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    /// RFC 3339.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// `<scheme>:<path>`. The path is opaque to consumers.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UnitId(String);

impl UnitId {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// Joins a scheme and a path.
    #[must_use]
    pub fn from_parts(scheme: &str, path: &str) -> Self {
        Self(format!("{scheme}:{path}"))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The namespace before the first `:`, if the id is well formed.
    #[must_use]
    pub fn scheme(&self) -> Option<&str> {
        self.0.split_once(':').map(|(scheme, _)| scheme)
    }

    #[must_use]
    pub fn path(&self) -> Option<&str> {
        self.0.split_once(':').map(|(_, path)| path)
    }
}

impl fmt::Display for UnitId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Identifies a call site within its unit: `<text>#<n>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CallKey(String);

impl CallKey {
    #[must_use]
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    /// The key for the `occurrence`-th call (0-based) written as `text` in a unit.
    #[must_use]
    pub fn for_text(text: &str, occurrence: usize) -> Self {
        Self(format!("{text}#{occurrence}"))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CallKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Unit {
    pub id: UnitId,
    pub name: String,
    /// Open vocabulary, displayed but never interpreted.
    pub kind: String,
    pub role: Role,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<UnitId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<ContentHash>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<UnitId>,
    /// Finer-grained units this one summarizes. Shorthand for `refines` links. Experimental.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub refines: Vec<UnitId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, Scalar>,
    /// Call sites in source order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub calls: Vec<Invocation>,
    /// Control flow over `calls`. Absent means every call runs once, in order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Vec<Step>>,
}

impl Unit {
    /// A unit with no calls, body or optional metadata.
    #[must_use]
    pub fn new(id: UnitId, name: impl Into<String>, kind: impl Into<String>, role: Role) -> Self {
        Self {
            id,
            name: name.into(),
            kind: kind.into(),
            role,
            parent: None,
            location: None,
            content_hash: None,
            aliases: Vec::new(),
            refines: Vec::new(),
            doc: None,
            attributes: BTreeMap::new(),
            calls: Vec::new(),
            body: None,
        }
    }

    #[must_use]
    pub fn call(&self, key: &CallKey) -> Option<&Invocation> {
        self.calls.iter().find(|call| &call.key == key)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Group,
    Invocable,
    External,
    Channel,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentHash {
    pub algo: String,
    pub value: String,
}

/// A scalar attribute value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Scalar {
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
}

impl From<&str> for Scalar {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl From<String> for Scalar {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<bool> for Scalar {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for Scalar {
    fn from(value: i64) -> Self {
        Self::Int(value)
    }
}

impl fmt::Display for Scalar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool(value) => write!(f, "{value}"),
            Self::Int(value) => write!(f, "{value}"),
            Self::Float(value) => write!(f, "{value}"),
            Self::String(value) => f.write_str(value),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Invocation {
    pub key: CallKey,
    /// The callee as written.
    pub text: String,
    /// What may run. Empty means unresolved.
    #[serde(default)]
    pub targets: Vec<Target>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub dispatch: Dispatch,
    #[serde(default, skip_serializing_if = "is_default")]
    pub mode: Mode,
    /// For [`Mode::Register`]: the unit that will invoke the target later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<UnitId>,
    /// A target ending in error ends the caller with that error here.
    #[serde(default, skip_serializing_if = "is_default")]
    pub propagates: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<Location>,
}

impl Invocation {
    /// A `call`-mode invocation of `targets`, keyed by `text` and occurrence.
    #[must_use]
    pub fn new(text: impl Into<String>, occurrence: usize, targets: Vec<Target>) -> Self {
        let text = text.into();
        Self {
            key: CallKey::for_text(&text, occurrence),
            text,
            targets,
            dispatch: Dispatch::default(),
            mode: Mode::default(),
            via: None,
            propagates: false,
            location: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Target {
    pub unit: UnitId,
    pub certainty: Certainty,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Certainty {
    /// Asserted by a person, agent, or manifest.
    Declared,
    /// Resolved by the producer's rules.
    Exact,
    /// Matched heuristically.
    Inferred,
    /// Seen at runtime.
    Observed,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Dispatch {
    /// Exactly one target runs.
    #[default]
    One,
    /// Every target runs.
    All,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// The caller waits for the target.
    #[default]
    Call,
    /// The target runs concurrently; the caller does not wait.
    Spawn,
    /// The target is handed to `via`, which invokes it later.
    Register,
    #[serde(other)]
    Unknown,
}

/// A control-flow step. See `docs/ir.md` for semantics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum Step {
    Call {
        call: CallKey,
    },
    Branch {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        guard: Option<String>,
        arms: Vec<Arm>,
    },
    Loop {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        body: Vec<Step>,
    },
    Exit {
        kind: ExitKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    Dag {
        nodes: Vec<DagNode>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Arm {
    pub label: String,
    #[serde(default)]
    pub body: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DagNode {
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<String>,
    #[serde(default)]
    pub body: Vec<Step>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitKind {
    Return,
    Error,
    Panic,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub unit: UnitId,
    pub label: String,
    pub kind: String,
    /// Wrappers around the unit, outermost first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chain: Vec<UnitId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Location {
    /// POSIX path relative to the document root, or an absolute URI.
    pub uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<Range>,
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.uri)?;
        if let Some(range) = &self.range {
            write!(f, ":{}:{}", range.start_line, range.start_col)?;
        }
        Ok(())
    }
}

/// 1-based lines; 1-based columns counting UTF-8 bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Range {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_with_defaults_omitted() {
        let mut document = Document::new(Producer {
            name: "test".into(),
            version: "0".into(),
        });
        let mut unit = Unit::new(UnitId::new("rust:app::main"), "main", "fn", Role::Invocable);
        unit.calls.push(Invocation::new("run", 0, Vec::new()));
        unit.body = Some(vec![Step::Call {
            call: CallKey::for_text("run", 0),
        }]);
        document.units.push(unit);

        let json = document.to_json().unwrap();
        assert!(!json.contains("dispatch"), "defaults are omitted: {json}");
        assert!(json.contains(r#""step": "call""#));
        assert_eq!(Document::from_json(&json).unwrap(), document);
    }

    #[test]
    fn unknown_variants_from_newer_producers_are_tolerated() {
        let json = r#"{
            "schema": "codeviz/0.9",
            "producer": { "name": "future", "version": "9" },
            "future_field": true,
            "units": [{
                "id": "x:a", "name": "a", "kind": "thing", "role": "hologram",
                "body": [{ "step": "teleport", "to": "b" }]
            }]
        }"#;
        let document = Document::from_json(json).unwrap();
        assert_eq!(document.units[0].role, Role::Unknown);
        assert_eq!(document.units[0].body, Some(vec![Step::Unknown]));
    }

    #[test]
    fn rejects_other_major_versions() {
        let json = r#"{ "schema": "codeviz/1.0", "producer": { "name": "x", "version": "1" } }"#;
        assert!(matches!(
            Document::from_json(json),
            Err(Error::Schema { .. })
        ));
    }

    #[test]
    fn unit_id_parts() {
        let id = UnitId::new("python:shop.app.main");
        assert_eq!(id.scheme(), Some("python"));
        assert_eq!(id.path(), Some("shop.app.main"));
    }
}

/// A correspondence between two units. Experimental.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub kind: LinkKind,
    pub from: UnitId,
    pub to: UnitId,
    #[serde(default = "declared")]
    pub certainty: Certainty,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// `from` and `to` are the same thing described by two sources.
    SameAs,
    /// `from` summarizes the behavior `to` implements in more detail.
    Refines,
    #[serde(other)]
    Unknown,
}

fn declared() -> Certainty {
    Certainty::Declared
}
