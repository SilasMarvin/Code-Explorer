//! Flattening YAML values into scalar attributes.

use std::collections::BTreeMap;

use codeviz_ir::Scalar;
use serde_json::Value;

pub(crate) type Attributes = BTreeMap<String, Scalar>;

/// Writes `value` under `prefix`: maps become `prefix.key`, lists of scalars are joined
/// with `, `, other lists are indexed as `prefix.0`, and nulls are dropped.
pub(crate) fn flatten(prefix: &str, value: &Value, out: &mut Attributes) {
    match value {
        Value::Null => {}
        Value::Bool(value) => {
            out.insert(prefix.to_owned(), Scalar::Bool(*value));
        }
        Value::Number(number) => {
            let scalar = number
                .as_i64()
                .map(Scalar::Int)
                .or_else(|| number.as_f64().map(Scalar::Float))
                .unwrap_or_else(|| Scalar::String(number.to_string()));
            out.insert(prefix.to_owned(), scalar);
        }
        Value::String(value) => {
            out.insert(prefix.to_owned(), Scalar::String(value.clone()));
        }
        Value::Array(items) => {
            if let Some(joined) = join_scalars(items) {
                out.insert(prefix.to_owned(), Scalar::String(joined));
            } else {
                for (index, item) in items.iter().enumerate() {
                    flatten(&format!("{prefix}.{index}"), item, out);
                }
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                flatten(&format!("{prefix}.{key}"), item, out);
            }
        }
    }
}

fn join_scalars(items: &[Value]) -> Option<String> {
    items
        .iter()
        .map(|item| match item {
            Value::String(value) => Some(value.clone()),
            Value::Number(_) | Value::Bool(_) => Some(item.to_string()),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()
        .map(|items| items.join(", "))
}

/// The number of jobs a step expands to: `parallelism`, or the size of its matrix
/// before adjustments.
pub(crate) fn instances(extra: &BTreeMap<String, Value>) -> Option<i64> {
    if let Some(parallelism) = extra.get("parallelism").and_then(Value::as_i64) {
        return Some(parallelism);
    }
    let matrix = extra.get("matrix")?;
    let len = |value: &Value| value.as_array().map(Vec::len);
    let count = match matrix.get("setup") {
        Some(Value::Object(dimensions)) => dimensions.values().map(len).product::<Option<usize>>(),
        Some(setup) => len(setup),
        None => len(matrix),
    }?;
    i64::try_from(count).ok()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn flattens_nested_values() {
        let mut out = Attributes::new();
        flatten(
            "retry",
            &json!({ "automatic": [{ "exit_status": -1, "limit": 2 }], "manual": false }),
            &mut out,
        );
        flatten("artifact_paths", &json!(["a/*", "b"]), &mut out);
        assert_eq!(out["retry.automatic.0.exit_status"], Scalar::Int(-1));
        assert_eq!(out["retry.manual"], Scalar::Bool(false));
        assert_eq!(out["artifact_paths"], Scalar::from("a/*, b"));
    }

    #[test]
    fn counts_instances() {
        let matrix = |value: Value| BTreeMap::from([("matrix".to_owned(), value)]);
        let setup =
            json!({ "setup": { "os": ["linux", "macos"], "rust": ["1.99", "nightly", "beta"] } });
        assert_eq!(instances(&matrix(setup)), Some(6));
        assert_eq!(instances(&matrix(json!(["a", "b"]))), Some(2));
        let parallel = BTreeMap::from([("parallelism".to_owned(), json!(4))]);
        assert_eq!(instances(&parallel), Some(4));
    }
}
