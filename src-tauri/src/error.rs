//! Structured failure classification.
//!
//! The backend's results already carry a stable `code`; this module gives
//! every failure a **kind** as well, so the model, the frontend and the audit
//! trail can tell a refusal from a timeout from a bad request without
//! matching on English prose.
//!
//! The kind is additive: the existing `code` and `message` stay exactly as
//! they were, so nothing that reads them has to change.

use serde::Serialize;
use serde_json::Value;

/// What kind of failure this is.
///
/// The set is small and stable on purpose: it is a contract, not a
/// description.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorKind {
    /// The request or its arguments were not usable.
    InvalidRequest,
    /// A policy, capability or approval gate refused the action.
    Denied,
    /// The action was cancelled or interrupted.
    Cancelled,
    /// The thing the request referred to does not exist.
    NotFound,
    /// A transport or remote-service failure.
    Network,
    /// A bound was exceeded: time, size, count or budget.
    LimitExceeded,
    /// The operation did not finish in time.
    Timeout,
    /// The failure happened inside the application.
    Internal,
}

impl ErrorKind {
    /// The stable wire name of this kind.
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorKind::InvalidRequest => "invalidRequest",
            ErrorKind::Denied => "denied",
            ErrorKind::Cancelled => "cancelled",
            ErrorKind::NotFound => "notFound",
            ErrorKind::Network => "network",
            ErrorKind::LimitExceeded => "limitExceeded",
            ErrorKind::Timeout => "timeout",
            ErrorKind::Internal => "internal",
        }
    }
}

/// A failure with a machine-readable kind, a stable code and a human
/// message.
///
/// The human message is still what a person reads; the kind and the code are
/// what a program reads.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    /// The classification of this failure.
    pub kind: ErrorKind,
    /// The stable, specific code for this failure.
    pub code: String,
    /// The human-readable explanation.
    pub message: String,
}

impl AppError {
    /// Build a failure from its parts.
    pub fn new(kind: ErrorKind, code: &str, message: impl Into<String>) -> Self {
        Self {
            kind,
            code: code.to_string(),
            message: message.into(),
        }
    }

    /// The classification of this failure as a JSON value.
    pub fn kind_value(&self) -> Value {
        Value::String(self.kind.as_str().to_string())
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AppError {}

/// A failure that still reads as its plain message everywhere it is used.
///
/// This keeps every existing `Result<_, String>` signature working while
/// giving the new code a typed way to describe a failure.
impl From<AppError> for String {
    fn from(error: AppError) -> String {
        error.message
    }
}

/// The kind that belongs to a stable failure code.
///
/// The codes are the ones the tool layer already produces. A code that is not
/// listed here is an `internal` failure, which is the safe default: it never
/// claims to be retryable or a refusal.
pub fn kind_for_code(code: &str) -> ErrorKind {
    match code {
        "denied" | "deny" | "tool-budget-blocked" | "hook-blocked" | "capability-disabled"
        | "replay-blocked" | "policy-denied" => ErrorKind::Denied,
        "cancelled" | "canceled" | "interrupted" | "aborted" => ErrorKind::Cancelled,
        "not-found" | "unknown-tool" | "unavailable-tool" | "missing-tool" => ErrorKind::NotFound,
        "timeout" | "timed-out" | "deadline" => ErrorKind::Timeout,
        "limit-exceeded" | "too-large" | "too-many" | "quota-exceeded" | "budget-exhausted" => {
            ErrorKind::LimitExceeded
        }
        "network" | "connection-failed" | "unreachable" => ErrorKind::Network,
        "invalid-arguments" | "invalid-request" | "validation-failed" | "schema-mismatch"
        | "malformed-arguments" => ErrorKind::InvalidRequest,
        _ => ErrorKind::Internal,
    }
}

/// Add the classification to a tool result's error object.
///
/// The `code` and `message` are untouched; `kind` is added beside them, so a
/// reader that ignores it is unaffected.
pub fn annotate_error_object(error: &mut Value) {
    if let Some(map) = error.as_object_mut() {
        if !map.contains_key("kind") {
            let kind = map
                .get("code")
                .and_then(Value::as_str)
                .map(kind_for_code)
                .unwrap_or(ErrorKind::Internal);
            map.insert("kind".into(), Value::String(kind.as_str().to_string()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn every_code_maps_to_a_kind() {
        assert_eq!(kind_for_code("denied"), ErrorKind::Denied);
        assert_eq!(kind_for_code("tool-budget-blocked"), ErrorKind::Denied);
        assert_eq!(kind_for_code("cancelled"), ErrorKind::Cancelled);
        assert_eq!(kind_for_code("timeout"), ErrorKind::Timeout);
        assert_eq!(kind_for_code("something-unknown"), ErrorKind::Internal);
    }

    #[test]
    fn annotating_adds_the_kind_without_touching_the_code_or_message() {
        let mut error = json!({"code":"denied","message":"The user denied this tool request."});
        annotate_error_object(&mut error);
        assert_eq!(error["code"], "denied");
        assert_eq!(error["message"], "The user denied this tool request.");
        assert_eq!(error["kind"], "denied");
    }

    #[test]
    fn annotating_twice_keeps_the_first_kind() {
        let mut error = json!({"code":"denied","message":"no"});
        annotate_error_object(&mut error);
        error["code"] = json!("timeout");
        annotate_error_object(&mut error);
        assert_eq!(error["kind"], "denied");
    }
}
