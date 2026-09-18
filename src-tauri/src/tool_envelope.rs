//! Versioned structured tool contracts (harness improvement stage 1).
//!
//! Every tool result that reaches the model uses one envelope so success,
//! partial completion, failure, cancellation, and unknown outcomes are never
//! confused with each other.
//!
//! Execution results already carry the envelope; tests exercise [`make`],
//! [`validate_envelope`], and [`legacy_execution_to_envelope`] directly.

use serde_json::{json, Value};

pub const SCHEMA_VERSION: &str = "locallm.tool-envelope/1";

/// All outcomes the envelope distinguishes. Constructed by execution,
/// cancellation, and timeout paths across the harness.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Success,
    Partial,
    Failure,
    Cancelled,
    Unknown,
}

impl Status {
    #[allow(dead_code)]
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Success => "success",
            Status::Partial => "partial",
            Status::Failure => "failure",
            Status::Cancelled => "cancelled",
            Status::Unknown => "unknown",
        }
    }

    #[allow(dead_code)]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "success" => Some(Status::Success),
            "partial" => Some(Status::Partial),
            "failure" => Some(Status::Failure),
            "cancelled" => Some(Status::Cancelled),
            "unknown" => Some(Status::Unknown),
            _ => None,
        }
    }
}

/// Build an envelope. `data` must be non-null for success/partial; `error`
/// must be `{"code","message"}` for failure/cancelled/unknown.
#[allow(dead_code)]
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

#[allow(dead_code)]
pub fn success(data: Value) -> Value {
    make(Status::Success, data, Vec::new(), None, json!({}))
}

#[allow(dead_code)]
pub fn error_envelope(code: &str, message: &str, retryable: bool) -> Value {
    make(
        Status::Failure,
        Value::Null,
        Vec::new(),
        Some(json!({"code": code, "message": message, "retryable": retryable})),
        json!({}),
    )
}

/// Validate an envelope before it is returned to the model. Returns the
/// parsed status on success; any contract violation is a loud error.
/// Exercised by envelope and execution tests; the chat loop validates
/// through the execution result shape.
#[allow(dead_code)]
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
        Status::Success | Status::Partial => {
            if data.is_null() {
                return Err("Successful tool results must carry data, not null.".into());
            }
            if !error.is_null() {
                return Err("Successful tool results must not carry an error.".into());
            }
        }
        Status::Failure | Status::Cancelled | Status::Unknown => {
            let code = error.get("code").and_then(Value::as_str).unwrap_or("");
            let message = error.get("message").and_then(Value::as_str).unwrap_or("");
            if code.trim().is_empty() || message.trim().is_empty() {
                return Err("Failed tool results must carry an error code and message.".into());
            }
        }
    }
    Ok(status)
}

/// Explicit adapter for the legacy flat execution shape
/// (`{provider, exitCode, stdout, stderr, error, isError, durationMs}`).
/// Legacy fields are preserved verbatim inside `data.logs` and at the top
/// level for existing consumers; the envelope status derives from the same
/// `isError`/exit-code rules the old code used.
#[allow(dead_code)]
pub fn legacy_execution_to_envelope(legacy: &Value, duration_ms: u128) -> Value {
    let exit_code = legacy.get("exitCode").cloned().unwrap_or(Value::Null);
    let is_error = legacy
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let message = legacy
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("");
    let truncated = message.contains("exceeded")
        || message.contains("truncated")
        || message.contains("Truncated");
    let (status, error) = if is_error {
        let code = if message.contains("timed out") {
            "timeout"
        } else if truncated {
            "output-truncated"
        } else if exit_code.is_number() {
            "nonzero-exit"
        } else {
            "execution-failed"
        };
        (
            Status::Failure,
            Some(json!({"code": code, "message": if message.is_empty() {
                "Execution failed without a diagnostic message."
            } else {
                message
            }, "retryable": false})),
        )
    } else {
        (Status::Success, None)
    };
    let mut envelope = make(
        status,
        json!({
            "exitCode": exit_code,
            "logs": {
                "stdout": legacy.get("stdout").cloned().unwrap_or(Value::Null),
                "stderr": legacy.get("stderr").cloned().unwrap_or(Value::Null),
            },
            "result": Value::Null,
            "resultStatus": "not_requested",
        }),
        Vec::new(),
        error,
        json!({"durationMs": duration_ms, "truncated": truncated}),
    );
    // Compatibility: keep the legacy flat fields beside the envelope.
    if let (Some(map), Some(legacy_map)) = (envelope.as_object_mut(), legacy.as_object()) {
        for (key, val) in legacy_map {
            map.entry(key.clone()).or_insert_with(|| val.clone());
        }
    }
    envelope
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
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn success_round_trips_and_empty_containers_are_not_failures() {
        for data in [json!({}), json!([]), json!({"items": []}), json!(0), json!(false)] {
            let envelope = success(data);
            assert_eq!(validate_envelope(&envelope).unwrap(), Status::Success);
        }
    }

    #[test]
    fn null_data_is_never_success() {
        let envelope = make(Status::Success, Value::Null, Vec::new(), None, json!({}));
        assert!(validate_envelope(&envelope).is_err());
    }

    #[test]
    fn truncated_output_must_not_be_success() {
        let envelope = make(
            Status::Success,
            json!({"partial": true}),
            Vec::new(),
            None,
            json!({"truncated": true}),
        );
        assert!(validate_envelope(&envelope).unwrap_err().contains("Truncated"));
        let failure = make(
            Status::Failure,
            Value::Null,
            Vec::new(),
            Some(json!({"code": "output-truncated", "message": "cut"})),
            json!({"truncated": true}),
        );
        assert_eq!(validate_envelope(&failure).unwrap(), Status::Failure);
    }

    #[test]
    fn failures_require_machine_readable_errors() {
        assert!(validate_envelope(&error_envelope("timeout", "timed out", false)).is_ok());
        let missing = make(Status::Failure, Value::Null, Vec::new(), None, json!({}));
        assert!(validate_envelope(&missing).is_err());
        let vague = make(
            Status::Unknown,
            Value::Null,
            Vec::new(),
            Some(json!({"code": "", "message": ""})),
            json!({}),
        );
        assert!(validate_envelope(&vague).is_err());
    }

    #[test]
    fn unknown_status_and_version_are_rejected() {
        let mut envelope = success(json!({"ok": true}));
        envelope["status"] = json!("bogus");
        assert!(validate_envelope(&envelope).is_err());
        let mut envelope = success(json!({"ok": true}));
        envelope["schemaVersion"] = json!("v9");
        assert!(validate_envelope(&envelope).is_err());
    }

    #[test]
    fn legacy_execution_maps_to_the_matching_outcome() {
        let ok = json!({"provider":"local","exitCode":0,"stdout":"hi","stderr":"","error":null,"isError":false,"durationMs":3});
        let envelope = legacy_execution_to_envelope(&ok, 3);
        assert_eq!(validate_envelope(&envelope).unwrap(), Status::Success);
        assert_eq!(envelope["stdout"], "hi");
        assert_eq!(envelope["data"]["logs"]["stdout"], "hi");
        let failed = json!({"provider":"local","exitCode":7,"stdout":"","stderr":"boom","error":null,"isError":true,"durationMs":3});
        let envelope = legacy_execution_to_envelope(&failed, 3);
        assert_eq!(validate_envelope(&envelope).unwrap(), Status::Failure);
        assert_eq!(envelope["error"]["code"], "nonzero-exit");
        let timed_out = json!({"provider":"local","exitCode":null,"stdout":"","stderr":"","error":"Execution timed out.","isError":true,"durationMs":3});
        let envelope = legacy_execution_to_envelope(&timed_out, 3);
        assert_eq!(envelope["error"]["code"], "timeout");
    }

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
