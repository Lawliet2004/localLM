//! Versioned structured tool contracts.
//!
//! Every tool result that reaches the model uses one envelope so success and
//! failure are never confused with each other.

use serde_json::{json, Value};

pub const SCHEMA_VERSION: &str = "locallm.tool-envelope/1";

/// Outcomes the envelope distinguishes. Execution, cancellation, and timeout
/// paths construct these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Success,
    Failure,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Success => "success",
            Status::Failure => "failure",
        }
    }
}

/// Build an envelope. `data` must be non-null for success; `error` must be
/// `{"code","message"}` for failure.
pub fn make(
    status: Status,
    data: Value,
    artifacts: Vec<Value>,
    error: Option<Value>,
    metadata: Value,
) -> Value {
    json!({
        "schemaVersion": SCHEMA_VERSION,
        "status": status.as_str(),
        "data": data,
        "artifacts": artifacts,
        "error": error.unwrap_or(Value::Null),
        "metadata": metadata,
    })
}

/// Minimal JSON-schema-subset validator for requested output schemas.
/// Supports `type` (object/array/string/number/integer/boolean/null),
/// `required`, `properties`, `items`, and `enum`. Anything richer is rejected
/// loudly at request time rather than guessed at.
pub fn validate_against_schema(value: &Value, schema: &Value) -> Result<(), String> {
    let Some(schema_map) = schema.as_object() else {
        return Err("result_schema must be a JSON object.".into());
    };
    if let Some(expected) = schema_map.get("type").and_then(Value::as_str) {
        let matches = match expected {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "number" => value.is_number(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => return Err(format!("Unsupported schema type '{expected}'.")),
        };
        if !matches {
            return Err(format!("Result does not match the requested schema (expected {expected})."));
        }
    }
    if let Some(allowed) = schema_map.get("enum").and_then(Value::as_array) {
        if !allowed.iter().any(|option| option == value) {
            return Err("Result is not one of the schema's allowed values.".into());
        }
    }
    if let (Some(obj), Some(required)) = (
        value.as_object(),
        schema_map.get("required").and_then(Value::as_array),
    ) {
        for key in required.iter().filter_map(Value::as_str) {
            if !obj.contains_key(key) {
                return Err(format!("Result is missing required key '{key}'."));
            }
        }
        if let Some(properties) = schema_map.get("properties").and_then(Value::as_object) {
            for (key, subschema) in properties {
                if let Some(field) = obj.get(key) {
                    validate_against_schema(field, subschema).map_err(|error| format!("{key}: {error}"))?;
                }
            }
        }
    }
    if let (Some(items), Some(item_schema)) = (value.as_array(), schema_map.get("items")) {
        for (index, item) in items.iter().enumerate() {
            validate_against_schema(item, item_schema)
                .map_err(|error| format!("index {index}: {error}"))?;
        }
    }
    Ok(())
}

#[cfg(test)]
impl Status {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "success" => Some(Status::Success),
            "failure" => Some(Status::Failure),
            _ => None,
        }
    }
}

/// Test-only: execution tests validate real envelopes through this. Returns
/// the parsed status; any contract violation is a loud error.
#[cfg(test)]
pub fn validate_envelope(value: &Value) -> Result<Status, String> {
    let version = value
        .get("schemaVersion")
        .and_then(Value::as_str)
        .ok_or("Tool result is missing schemaVersion.")?;
    if version != SCHEMA_VERSION {
        return Err(format!("Unsupported tool envelope version '{version}'."));
    }
    let status = value
        .get("status")
        .and_then(Value::as_str)
        .and_then(Status::parse)
        .ok_or("Tool result has an unknown status.")?;
    let data = value.get("data").ok_or("Tool result is missing data.")?;
    let error = value.get("error").ok_or("Tool result is missing error.")?;
    let truncated = value
        .get("metadata")
        .and_then(|meta| meta.get("truncated"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if truncated && status == Status::Success {
        return Err("Truncated tool output must not be reported as success.".into());
    }
    match status {
        Status::Success => {
            if data.is_null() {
                return Err("Successful tool results must carry data, not null.".into());
            }
            if !error.is_null() {
                return Err("Successful tool results must not carry an error.".into());
            }
        }
        Status::Failure => {
            let code = error.get("code").and_then(Value::as_str).unwrap_or("");
            let message = error.get("message").and_then(Value::as_str).unwrap_or("");
            if code.trim().is_empty() || message.trim().is_empty() {
                return Err("Failed tool results must carry an error code and message.".into());
            }
        }
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn schema_subset_validates_nested_requirements() {
        let schema = json!({"type":"object","required":["answer"],"properties":{"answer":{"type":"string"},"citations":{"type":"array","items":{"type":"string"}}}});
        assert!(validate_against_schema(&json!({"answer":"x","citations":["S1"]}), &schema).is_ok());
        assert!(validate_against_schema(&json!({"citations":[]}), &schema).is_err());
        assert!(validate_against_schema(&json!({"answer":3}), &schema).is_err());
        assert!(validate_against_schema(&json!({"answer":"x","citations":[3]}), &schema).is_err());
        assert!(validate_against_schema(&json!({"answer":null}), &schema).is_err());
        assert!(validate_against_schema(&json!(null), &json!({"type":"null"})).is_ok());
        assert!(validate_against_schema(&json!(1), &json!({"enum":[1,2]})).is_ok());
        assert!(validate_against_schema(&json!(9), &json!({"enum":[1,2]})).is_err());
        assert!(validate_against_schema(&json!(1), &json!({"type":"object"})).is_err());
    }
}
