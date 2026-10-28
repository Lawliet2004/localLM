//! Versioned structured tool contracts (harness improvement stage 1).
//!
//! Every tool result that reaches the model uses one envelope so success,
//! partial completion, failure, cancellation, and unknown outcomes are never
//! confused with each other.
//!
//! Normalization happens once at each model-visible boundary —
//! [`normalize`] for backend `Value`s, [`normalize_error`] for dispatch-level
//! `Err`s — in the parent tool loop, the PTC step loop, and both child
//! drivers (foreground `call_child_tool` + detached harness/direct arms).
//! Execution already produces envelopes; they pass through unchanged.
//! Original backend data is preserved inside `data`, and flat compatibility
//! keys (`isError`, `message`, `denied`, `paused`, exitCode/stdout/stderr,
//! `evidence`, `_artifactId`, `_screenshotArtifactIds`, ...) stay at the top
//! level so history, the UI, research postprocessing, and tests keep working.

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

/// Emit only the versioned contract. Flat keys remain decodable in saved
/// journals and internal compatibility paths, but are not repeated to models.
pub fn model_result(value: &Value) -> Value {
    if value["schemaVersion"] != SCHEMA_VERSION {
        return value.clone();
    }
    let mut canonical = value.clone();
    if let Some(object) = canonical.as_object_mut() {
        object.retain(|key, _| {
            matches!(
                key.as_str(),
                "schemaVersion" | "status" | "data" | "artifacts" | "error" | "metadata"
            )
        });
    }
    if let Some(id) = value["_artifactId"].as_str() {
        if !canonical["artifacts"]
            .as_array()
            .is_some_and(|artifacts| artifacts.iter().any(|artifact| artifact["id"] == id))
        {
            if !canonical["artifacts"].is_array() {
                canonical["artifacts"] = json!([]);
            }
            canonical["artifacts"].as_array_mut().unwrap().push(
                json!({"id":id,"sha256":value["_sha256"],"sizeBytes":value["_originalBytes"]}),
            );
        }
    }
    if let Some(note) = value.get("hookNote") {
        canonical["metadata"]["hookNote"] = note.clone();
    }
    canonical
}

/// Which backend family produced a result. The dispatch site passes the typed
/// variant it already knows — classification never reverse-engineers payload
/// text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    Workspace,
    Execution,
    Cloud,
    Harness,
    System,
    Skills,
    Mcp,
    Automation,
    Other,
}

impl Family {
    pub fn as_str(self) -> &'static str {
        match self {
            Family::Workspace => "workspace",
            Family::Execution => "execution",
            Family::Cloud => "cloud-execution",
            Family::Harness => "harness",
            Family::System => "system",
            Family::Skills => "skills",
            Family::Mcp => "mcp",
            Family::Automation => "automation",
            Family::Other => "other",
        }
    }
}

/// Typed dispatch facts only the call site knows: whether the await hit the
/// caller's timeout, an interrupt, or a policy/approval denial. These map to
/// distinct envelope statuses without any message sniffing.
#[derive(Clone, Copy, Default)]
pub struct Cause {
    pub timeout: bool,
    pub cancelled: bool,
    pub denied: bool,
}

/// `metadata.sideEffects` — what a failed or interrupted call may have changed.
pub const SIDE_NONE: &str = "none";
pub const SIDE_APPLIED: &str = "applied";
pub const SIDE_ROLLED_BACK: &str = "rolled_back";
pub const SIDE_UNKNOWN: &str = "unknown";

/// A result claiming our envelope namespace. Version validation is separate,
/// so an unsupported version cannot fall through as ordinary success data.
pub fn is_envelope(value: &Value) -> bool {
    value
        .get("schemaVersion")
        .and_then(Value::as_str)
        .is_some_and(|version| version.starts_with("locallm.tool-envelope/"))
}

/// Model-visible failure across legacy `isError` results and envelopes —
/// the single predicate consumers share.
pub fn failed(result: &Value) -> bool {
    result.get("isError") == Some(&Value::Bool(true))
        || matches!(
            result.get("status").and_then(Value::as_str),
            Some("failure" | "cancelled" | "unknown")
        )
}

/// Merge the producer's original fields beside the envelope so legacy
/// consumers (audit rows, UI, research postprocessing, `isError` checks) keep
/// working. Envelope keys always win; nothing is dropped from `data`.
pub(crate) fn merge_compat(envelope: &mut Value, original: &Value) {
    if let (Some(map), Some(original_map)) = (envelope.as_object_mut(), original.as_object()) {
        for (key, val) in original_map {
            map.entry(key.clone()).or_insert_with(|| val.clone());
        }
    }
}

pub(crate) fn error_object(
    code: &str,
    message: &str,
    retryable: bool,
    side_effects: &str,
) -> Value {
    let mut error = json!({"code": code, "message": message, "retryable": retryable, "sideEffects": side_effects});
    // The kind is additive: the code and the message stay exactly as they
    // were, so a reader that ignores it is unaffected.
    crate::error::annotate_error_object(&mut error);
    error
}

fn base_metadata(family: Family, source: &Value) -> Value {
    let truncated = source
        .pointer("/metadata/truncated")
        .and_then(Value::as_bool)
        .or_else(|| source.get("truncated").and_then(Value::as_bool))
        .or_else(|| source.get("_truncated").and_then(Value::as_bool))
        .unwrap_or(false);
    json!({"family": family.as_str(), "truncated": truncated})
}

/// Normalize a backend result value into the shared envelope — once, at the
/// boundary where the result becomes model-visible. Idempotent: an envelope
/// produced by execution passes through unchanged; a malformed envelope is
/// reported as failure, never upgraded to success.
///
/// Typed mappings take precedence over string inference: the A10 transaction
/// `status` field, explicit `denied`/`paused`/`code` markers, and the legacy
/// execution shape are recognized before generic success/failure.
pub fn normalize(result: Value, family: Family) -> Value {
    let mut envelope = normalize_inner(result, family);
    if let Err(reason) = validate_envelope(&envelope) {
        return normalize_error(
            &format!("Invalid normalized tool result: {reason}"),
            Cause::default(),
            family,
        );
    }
    // Older execution envelopes predate explicit outcome certainty. Preserve
    // their data, while giving every newly emitted envelope a typed default;
    // absence of a field must never imply that replaying a write is safe.
    let side = envelope
        .pointer("/error/sideEffects")
        .and_then(Value::as_str)
        .or_else(|| {
            envelope
                .pointer("/metadata/sideEffects")
                .and_then(Value::as_str)
        })
        .unwrap_or(if matches!(family, Family::System | Family::Skills) {
            SIDE_NONE
        } else {
            SIDE_UNKNOWN
        })
        .to_string();
    envelope["metadata"]["sideEffects"] = json!(side);
    if envelope.pointer("/metadata/family").is_none() {
        envelope["metadata"]["family"] = json!(family.as_str());
    }
    if let Some(error) = envelope.get_mut("error").and_then(Value::as_object_mut) {
        error.insert("sideEffects".into(), json!(side));
        let retryable = error
            .get("retryable")
            .and_then(Value::as_bool)
            .unwrap_or(false)
            && matches!(side.as_str(), SIDE_NONE | SIDE_ROLLED_BACK);
        error.insert("retryable".into(), json!(retryable));
    }
    envelope
}

fn normalize_inner(result: Value, family: Family) -> Value {
    if is_envelope(&result) {
        match validate_envelope(&result) {
            Ok(_) => {
                let mut envelope = result;
                if envelope.pointer("/metadata/family").is_none() {
                    envelope["metadata"]["family"] = json!(family.as_str());
                }
                return envelope;
            }
            Err(reason) => {
                let mut envelope = make(
                    Status::Failure,
                    json!({"malformed": result}),
                    Vec::new(),
                    Some(error_object(
                        "malformed-envelope",
                        &reason,
                        false,
                        SIDE_UNKNOWN,
                    )),
                    json!({"family": family.as_str(), "truncated": false}),
                );
                if let Some(map) = envelope.as_object_mut() {
                    map.insert("isError".into(), Value::Bool(true));
                    map.insert("message".into(), Value::String(reason.clone()));
                }
                return envelope;
            }
        }
    }

    let metadata = base_metadata(family, &result);
    let status_hint = result.get("status").and_then(Value::as_str).unwrap_or("");
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let message = result
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| result.pointer("/error/message").and_then(Value::as_str))
        .or_else(|| result.get("error").and_then(Value::as_str))
        .unwrap_or("Tool failed.");

    // Native Workspace owns this transaction protocol. A connector's domain
    // `status` is ordinary data and must never acquire native write semantics.
    let classified = if family == Family::Workspace {
        match status_hint {
            "committed" | "cleaned" if is_error => Some((
                Status::Failure,
                Some((
                    "cleanup-failed",
                    "The transaction completed but cleanup failed; recovery is required.",
                    false,
                    SIDE_APPLIED,
                )),
            )),
            "committed" | "cleaned" => Some((Status::Success, None)),
            "rolled_back" if result.get("isError") == Some(&Value::Bool(false)) => {
                Some((Status::Success, None))
            }
            "rolled_back" => Some((
                Status::Failure,
                Some((
                    "rolled-back",
                    "The change was rolled back; originals were restored.",
                    false,
                    SIDE_ROLLED_BACK,
                )),
            )),
            "not_started" => Some((
                Status::Failure,
                Some((
                    "not-started",
                    "No project files were changed.",
                    false,
                    SIDE_NONE,
                )),
            )),
            "unknown" => Some((
                Status::Unknown,
                Some((
                    "outcome-unknown",
                    "Outcome is unknown; side effects may be partial. Do not automatically retry.",
                    false,
                    SIDE_UNKNOWN,
                )),
            )),
            _ => None,
        }
    } else {
        None
    };
    if let Some((status, error_hint)) = classified {
        let error = error_hint.map(|(code, fallback, retryable, side)| {
            error_object(
                code,
                if message == "Tool failed." {
                    fallback
                } else {
                    message
                },
                retryable,
                side,
            )
        });
        let mut metadata = metadata;
        // A committed transaction applied real writes; make the certainty explicit.
        if matches!(status_hint, "committed" | "cleaned") {
            metadata["sideEffects"] = json!(SIDE_APPLIED);
        } else if status_hint == "rolled_back" {
            metadata["sideEffects"] = json!(SIDE_ROLLED_BACK);
        }
        let mut envelope = make(status, result.clone(), Vec::new(), error, metadata);
        merge_compat(&mut envelope, &result);
        return envelope;
    }

    // Legacy flat execution output (pre-envelope callers, Daytona history) is
    // detected before the generic isError branch so exit-code semantics win.
    if result.get("exitCode").is_some()
        && (result.get("stdout").is_some() || result.get("stderr").is_some())
    {
        return legacy_execution_to_envelope(
            &result,
            result
                .get("durationMs")
                .and_then(Value::as_u64)
                .unwrap_or(0) as u128,
        );
    }

    if is_error {
        // Native dispatch notices may use the generic Other family. MCP
        // domain fields are not proof of pre-dispatch authorization outcomes.
        let native_notice = matches!(family, Family::Harness | Family::Other);
        let paused = native_notice
            && result
                .get("paused")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        let denied = native_notice
            && (result
                .get("denied")
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || result.get("code").and_then(Value::as_str) == Some("denied"));
        let (status, code, retryable, side_effects) = if paused {
            (Status::Cancelled, "paused", false, SIDE_NONE.to_string())
        } else if denied {
            (Status::Failure, "denied", false, SIDE_NONE.to_string())
        } else {
            let code = result
                .get("code")
                .and_then(Value::as_str)
                .or_else(|| result.pointer("/error/code").and_then(Value::as_str))
                .unwrap_or(match family {
                    Family::Workspace => "workspace-error",
                    Family::Mcp => "mcp-error",
                    Family::Automation => "automation-error",
                    Family::Harness => "harness-error",
                    _ => "call-failed",
                });
            let side_effects = result
                .get("sideEffects")
                .or_else(|| result.pointer("/error/sideEffects"))
                .or_else(|| result.pointer("/metadata/sideEffects"))
                .and_then(Value::as_str)
                .filter(|side| {
                    matches!(
                        *side,
                        SIDE_NONE | SIDE_APPLIED | SIDE_ROLLED_BACK | SIDE_UNKNOWN
                    )
                })
                .unwrap_or(SIDE_UNKNOWN)
                .to_string();
            // A generic error is not proof that a write had no effect. Only
            // an explicitly safe native outcome can opt into automatic retry.
            let retryable = matches!(side_effects.as_str(), SIDE_NONE | SIDE_ROLLED_BACK)
                && result
                    .get("retryable")
                    .or_else(|| result.pointer("/error/retryable"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
            (Status::Failure, code, retryable, side_effects)
        };
        let mut envelope = make(
            status,
            result.clone(),
            Vec::new(),
            Some(error_object(code, message, retryable, &side_effects)),
            metadata,
        );
        merge_compat(&mut envelope, &result);
        return envelope;
    }

    let truncated = metadata["truncated"].as_bool().unwrap_or(false);
    let status = if truncated {
        Status::Partial
    } else {
        Status::Success
    };
    let artifacts = result
        .get("artifacts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let data = if result.is_null() {
        json!({})
    } else {
        result.clone()
    };
    let mut envelope = make(status, data, artifacts, None, metadata);
    merge_compat(&mut envelope, &result);
    envelope
}

/// Normalize a dispatch-level error (`Err(String)` or a caller-side outcome)
/// with the typed cause the dispatch site knows. Timeout/cancelled outcomes
/// carry `sideEffects: "unknown"` and are never retryable — nothing may be
/// silently replayed.
pub fn normalize_error(message: &str, cause: Cause, family: Family) -> Value {
    let (status, code, side_effects) = if cause.cancelled {
        (Status::Cancelled, "cancelled", SIDE_UNKNOWN)
    } else if cause.timeout {
        (Status::Unknown, "timeout", SIDE_UNKNOWN)
    } else if cause.denied {
        (Status::Failure, "denied", SIDE_NONE)
    } else {
        (Status::Failure, "call-failed", SIDE_UNKNOWN)
    };
    let mut envelope = make(
        status,
        Value::Null,
        Vec::new(),
        Some(error_object(code, message, false, side_effects)),
        json!({"family": family.as_str(), "truncated": false}),
    );
    // Compatibility: legacy consumers read the flat failure markers.
    if let Some(map) = envelope.as_object_mut() {
        map.insert("isError".into(), Value::Bool(true));
        map.insert("message".into(), Value::String(message.to_string()));
    }
    envelope
}

/// A model-visible failure notice built at a dispatch boundary (denial,
/// invalid arguments, skipped repeats). Keeps `isError`/`message` plus any
/// extra flags (`paused`, `denied`) for existing consumers.
pub fn failure_notice(
    code: &str,
    message: &str,
    retryable: bool,
    side_effects: &str,
    extra: &Value,
) -> Value {
    let mut envelope = make(
        Status::Failure,
        Value::Null,
        Vec::new(),
        Some(error_object(code, message, retryable, side_effects)),
        json!({"truncated": false}),
    );
    if let Some(map) = envelope.as_object_mut() {
        map.insert("isError".into(), Value::Bool(true));
        map.insert("message".into(), Value::String(message.to_string()));
        if let Some(extra_map) = extra.as_object() {
            for (key, val) in extra_map {
                map.entry(key.clone()).or_insert_with(|| val.clone());
            }
        }
    }
    envelope
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
    value
        .get("artifacts")
        .and_then(Value::as_array)
        .ok_or("Tool envelope artifacts must be an array.")?;
    let metadata = value
        .get("metadata")
        .and_then(Value::as_object)
        .ok_or("Tool envelope metadata must be an object.")?;
    if metadata
        .get("truncated")
        .is_some_and(|flag| !flag.is_boolean())
    {
        return Err("Tool envelope metadata.truncated must be boolean.".into());
    }
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
    let message = legacy.get("error").and_then(Value::as_str).unwrap_or("");
    let truncated = legacy["truncated"] == true || message.contains("exceeded")
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
    } else if truncated {
        (Status::Partial, None)
    } else {
        (Status::Success, None)
    };
    let mut envelope = make(
        status,
        json!({
            "exitCode": exit_code,
            "processId": legacy["processId"],
            "cwd": legacy["cwd"],
            "logs": {
                "stdout": legacy.get("stdout").cloned().unwrap_or(Value::Null),
                "stderr": legacy.get("stderr").cloned().unwrap_or(Value::Null),
            },
            "result": Value::Null,
            "resultStatus": "not_requested",
        }),
        Vec::new(),
        error,
        json!({"durationMs": duration_ms, "truncated": truncated,
            "sideEffects":legacy.get("sideEffects").and_then(Value::as_str)
                .filter(|side| matches!(*side, SIDE_NONE | SIDE_APPLIED | SIDE_ROLLED_BACK | SIDE_UNKNOWN))
                .unwrap_or(SIDE_UNKNOWN)}),
    );
    // Compatibility: keep the legacy flat fields beside the envelope.
    if let (Some(map), Some(legacy_map)) = (envelope.as_object_mut(), legacy.as_object()) {
        for (key, val) in legacy_map {
            map.entry(key.clone()).or_insert_with(|| val.clone());
        }
    }
    envelope
}

/// Validate a declared result against its requested output schema. The
/// shared offline JSON Schema validator ([`crate::tool_validation`]) applies
/// the full contract — nested types, enums, ranges, additionalProperties —
/// rather than a handwritten subset. Invalid or remote-`$ref` schemas are
/// rejected loudly at request time.
pub fn validate_against_schema(value: &Value, schema: &Value) -> Result<(), String> {
    if !schema.is_object() {
        return Err("result_schema must be a JSON object.".into());
    }
    crate::tool_validation::validate_value(schema, value)
        .map_err(|error| format!("Result does not match the requested schema ({error})"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn success_round_trips_and_empty_containers_are_not_failures() {
        for data in [
            json!({}),
            json!([]),
            json!({"items": []}),
            json!(0),
            json!(false),
        ] {
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
        assert!(validate_envelope(&envelope)
            .unwrap_err()
            .contains("Truncated"));
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
    fn result_schema_validates_nested_requirements() {
        let schema = json!({"type":"object","required":["answer"],"properties":{"answer":{"type":"string"},"citations":{"type":"array","items":{"type":"string"}}}});
        assert!(
            validate_against_schema(&json!({"answer":"x","citations":["S1"]}), &schema).is_ok()
        );
        assert!(validate_against_schema(&json!({"citations":[]}), &schema).is_err());
        assert!(validate_against_schema(&json!({"answer":3}), &schema).is_err());
        assert!(validate_against_schema(&json!({"answer":"x","citations":[3]}), &schema).is_err());
        assert!(validate_against_schema(&json!({"answer":null}), &schema).is_err());
        assert!(validate_against_schema(&json!(null), &json!({"type":"null"})).is_ok());
        assert!(validate_against_schema(&json!(1), &json!({"enum":[1,2]})).is_ok());
        assert!(validate_against_schema(&json!(9), &json!({"enum":[1,2]})).is_err());
        assert!(validate_against_schema(&json!(1), &json!({"type":"object"})).is_err());
    }

    // --- A05: uniform normalization across every backend family ---

    #[test]
    fn normalize_wraps_plain_success_for_every_family() {
        for family in [
            Family::Workspace,
            Family::Execution,
            Family::Cloud,
            Family::Harness,
            Family::System,
            Family::Skills,
            Family::Mcp,
            Family::Automation,
            Family::Other,
        ] {
            let envelope = normalize(json!({"ok": true, "payload": family.as_str()}), family);
            assert_eq!(envelope["schemaVersion"], SCHEMA_VERSION);
            assert_eq!(envelope["status"], "success");
            assert_eq!(envelope["error"], Value::Null);
            assert_eq!(envelope["data"]["payload"], family.as_str());
            assert_eq!(envelope["metadata"]["family"], family.as_str());
            assert!(validate_envelope(&envelope).is_ok());
            // Legacy compat: original keys stay visible beside the envelope.
            assert_eq!(envelope["ok"], true);
        }
    }

    #[test]
    fn normalize_is_idempotent_on_valid_envelopes() {
        let once = normalize(json!({"answer": "42"}), Family::Harness);
        let twice = normalize(once.clone(), Family::Harness);
        assert_eq!(once, twice);
    }

    #[test]
    fn malformed_envelope_never_becomes_success() {
        // schemaVersion claims the contract but required fields are absent.
        let forged = json!({"schemaVersion": SCHEMA_VERSION, "status": "success"});
        let normalized = normalize(forged, Family::Mcp);
        assert_eq!(normalized["status"], "failure");
        assert_eq!(normalized["error"]["code"], "malformed-envelope");
        assert_eq!(normalized["isError"], true);
        // A forged "success" status on an incomplete envelope stays a failure.
        let double = normalize(normalized, Family::Mcp);
        assert_eq!(double["status"], "failure");
    }

    #[test]
    fn normalize_maps_typed_outcomes_without_string_sniffing() {
        // A10 transaction outcomes: typed `status` field, not message text.
        let committed = normalize(
            json!({"status": "committed", "files": ["a.rs"], "isolation": "single-rename"}),
            Family::Workspace,
        );
        assert_eq!(committed["status"], "success");
        assert_eq!(committed["metadata"]["sideEffects"], "applied");
        assert_eq!(committed["data"]["files"][0], "a.rs");
        let rolled = normalize(
            json!({"status": "rolled_back", "restored": 2}),
            Family::Workspace,
        );
        assert_eq!(rolled["status"], "failure");
        assert_eq!(rolled["error"]["code"], "rolled-back");
        assert_eq!(rolled["error"]["sideEffects"], "rolled_back");
        let unknown = normalize(
            json!({"status": "unknown", "message": "journal lost"}),
            Family::Workspace,
        );
        assert_eq!(unknown["status"], "unknown");
        assert_eq!(unknown["error"]["sideEffects"], "unknown");
        assert_eq!(unknown["error"]["retryable"], false);
        let not_started = normalize(
            json!({"status": "not_started", "reason": "preflight hash mismatch"}),
            Family::Workspace,
        );
        assert_eq!(not_started["status"], "failure");
        assert_eq!(not_started["error"]["sideEffects"], "none");
    }

    #[test]
    fn normalize_classifies_iserror_variants() {
        // `denied`/`paused` are native dispatch markers; foreign backends keep
        // their family code so a connector domain flag cannot claim auth refusal.
        let denied = normalize(
            json!({"isError": true, "denied": true, "message": "no"}),
            Family::Harness,
        );
        assert_eq!(denied["status"], "failure");
        assert_eq!(denied["error"]["code"], "denied");
        assert_eq!(denied["denied"], true);
        let foreign_denied = normalize(
            json!({"isError": true, "denied": true, "message": "no"}),
            Family::Mcp,
        );
        assert_eq!(foreign_denied["status"], "failure");
        assert_eq!(foreign_denied["error"]["code"], "mcp-error");
        let paused = normalize(
            json!({"isError": true, "paused": true, "message": "Pausing."}),
            Family::Harness,
        );
        assert_eq!(paused["status"], "cancelled");
        assert_eq!(paused["paused"], true);
        let mcp_error = normalize(
            json!({"isError": true, "content": [{"type": "text", "text": "upstream boom"}]}),
            Family::Mcp,
        );
        assert_eq!(mcp_error["status"], "failure");
        assert_eq!(mcp_error["error"]["code"], "mcp-error");
        assert!(mcp_error["data"]["content"].is_array());
        // Producer-supplied code/retryable/sideEffects are honored.
        let typed = normalize(
            json!({"isError": true, "message": "quota", "code": "quota-exceeded", "retryable": true, "sideEffects": "none"}),
            Family::Cloud,
        );
        assert_eq!(typed["error"]["code"], "quota-exceeded");
        assert_eq!(typed["error"]["retryable"], true);
    }

    #[test]
    fn normalize_converts_legacy_execution_shape() {
        let ok = normalize(
            json!({"provider": "local", "exitCode": 0, "stdout": "done", "stderr": "", "error": null, "isError": false}),
            Family::Execution,
        );
        assert_eq!(ok["status"], "success");
        assert_eq!(ok["stdout"], "done");
        assert_eq!(ok["data"]["logs"]["stdout"], "done");
        let failed = normalize(
            json!({"exitCode": 3, "stderr": "segv", "isError": true}),
            Family::Execution,
        );
        assert_eq!(failed["status"], "failure");
        assert_eq!(failed["error"]["code"], "nonzero-exit");
        let bounded = normalize(
            json!({"exitCode":0,"stdout":"bounded output","stderr":"","isError":false,
                "truncated":true,"sideEffects":SIDE_APPLIED}),
            Family::Harness,
        );
        assert_eq!(bounded["status"], "partial");
        assert_eq!(bounded["data"]["logs"]["stdout"], "bounded output");
        assert_eq!(bounded["metadata"]["sideEffects"], SIDE_APPLIED);
        assert!(bounded["error"].is_null());
        assert!(validate_envelope(&bounded).is_ok());
    }

    #[test]
    fn normalize_error_carries_typed_causes() {
        let timeout = normalize_error(
            "Tool request timed out.",
            Cause {
                timeout: true,
                ..Default::default()
            },
            Family::Execution,
        );
        assert_eq!(timeout["status"], "unknown");
        assert_eq!(timeout["error"]["code"], "timeout");
        assert_eq!(timeout["error"]["sideEffects"], "unknown");
        assert_eq!(timeout["error"]["retryable"], false);
        assert_eq!(timeout["isError"], true);
        let cancelled = normalize_error(
            "Interrupted.",
            Cause {
                cancelled: true,
                ..Default::default()
            },
            Family::Workspace,
        );
        assert_eq!(cancelled["status"], "cancelled");
        assert_eq!(cancelled["error"]["sideEffects"], "unknown");
        let denied = normalize_error(
            "User said no.",
            Cause {
                denied: true,
                ..Default::default()
            },
            Family::Harness,
        );
        assert_eq!(denied["status"], "failure");
        assert_eq!(denied["error"]["code"], "denied");
        assert_eq!(denied["error"]["sideEffects"], "none");
        let generic = normalize_error("backend gone", Cause::default(), Family::Mcp);
        assert_eq!(generic["status"], "failure");
        assert_eq!(generic["error"]["code"], "call-failed");
        assert_eq!(generic["metadata"]["family"], "mcp");
    }

    #[test]
    fn failure_notice_keeps_legacy_flags() {
        let paused = failure_notice(
            "paused",
            "Pausing.",
            false,
            SIDE_NONE,
            &json!({"paused": true}),
        );
        assert_eq!(paused["isError"], true);
        assert_eq!(paused["paused"], true);
        assert_eq!(paused["status"], "failure");
        assert_eq!(paused["error"]["code"], "paused");
        assert!(validate_envelope(&paused).is_ok());
    }
}
