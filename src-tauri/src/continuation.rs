//! Decisions for one assistant turn: whether tools stay available, whether a
//! failure may be repeated, and when a partial answer must name unfinished work.
//! The chat loop calls these; tests call the same functions.

use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SideEffect {
    None,
    Possible,
    Confirmed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailureClass {
    pub retryable: bool,
    pub side_effect: SideEffect,
    pub ambiguous: bool,
    pub denied: bool,
}

/// The executor classifies the failure. The model does not.
pub fn classify_execution_failure(message: &str) -> FailureClass {
    let lower = message.to_ascii_lowercase();
    let denied = lower.contains("denied")
        || lower.contains("not permitted")
        || lower.contains("permission")
        || lower.contains("stays denied");
    let ambiguous = !denied
        && (lower.contains("unknown")
            || lower.contains("timed out")
            || lower.contains("timeout")
            || lower.contains("may already have changed")
            || lower.contains("may be incomplete"));
    FailureClass {
        retryable: !denied && !ambiguous,
        side_effect: if ambiguous {
            SideEffect::Possible
        } else if denied {
            SideEffect::None
        } else {
            SideEffect::None
        },
        ambiguous,
        denied,
    }
}

/// Conservative ceilings. The last round is reserved for a partial answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Budgets {
    pub max_rounds: u32,
    pub max_searches: u32,
    pub max_fetches: u32,
    pub max_retries: u32,
    pub rounds_used: u32,
    pub searches_used: u32,
    pub fetches_used: u32,
    pub retries_used: u32,
}

impl Default for Budgets {
    fn default() -> Self {
        Self {
            max_rounds: 64,
            max_searches: 16,
            max_fetches: 16,
            max_retries: 1,
            rounds_used: 0,
            searches_used: 0,
            fetches_used: 0,
            retries_used: 0,
        }
    }
}

impl Budgets {
    pub fn exhausted(&self) -> bool {
        self.rounds_used >= self.max_rounds
            || self.searches_used >= self.max_searches
            || self.fetches_used >= self.max_fetches
    }

    /// True when the next model round must be the partial answer, not another tool.
    pub fn reserve_final_response(&self) -> bool {
        self.exhausted() || self.rounds_used + 1 >= self.max_rounds
    }

    pub fn note_tool(&mut self, name: &str) {
        if matches!(name, "web_search" | "search") {
            self.searches_used = self.searches_used.saturating_add(1);
        }
        if matches!(
            name,
            "web_read" | "web_open" | "web_fetch" | "web_fetch_url" | "web_find" | "visit"
        ) {
            self.fetches_used = self.fetches_used.saturating_add(1);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Continuation {
    /// Tools stay available. Evidence is not a completion signal.
    Proceed { evidence: Option<String> },
    CorrectOnce { reason: String },
    Partial { unresolved: Vec<String> },
    Stop { message: String, retryable: bool },
}

fn evidence_text(tool_name: &str, result: &Value) -> Option<String> {
    if !matches!(tool_name, "web_search" | "search") || result["isError"] == Value::Bool(true) {
        return None;
    }
    result["evidence"]
        .as_str()
        .or_else(|| result["answer"].as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

/// Same rules in ordinary chat and Plan mode. A successful search does not
/// disable later tools. A failed page leaves room for another source.
pub fn continuation_after_tool(
    _plan_mode: bool,
    tool_name: &str,
    result: &Value,
    budgets: &Budgets,
    corrections_used: u32,
    deliverable_missing: bool,
) -> Continuation {
    let denied = result["denied"] == Value::Bool(true)
        || result.get("code").and_then(Value::as_str) == Some("denied");
    if denied {
        return Continuation::Stop {
            message: "The action stays denied.".into(),
            retryable: false,
        };
    }
    if result["isError"] == Value::Bool(true) {
        let message = result["message"].as_str().unwrap_or("Tool failed.");
        let class = classify_execution_failure(message);
        if class.denied || class.ambiguous {
            return Continuation::Stop {
                message: message.to_string(),
                retryable: false,
            };
        }
    }
    if budgets.reserve_final_response() {
        return Continuation::Partial {
            unresolved: vec![format!(
                "Budget exhausted during {tool_name} before every step finished."
            )],
        };
    }
    if deliverable_missing {
        if corrections_used == 0 {
            return Continuation::CorrectOnce {
                reason: "A requested deliverable is missing.".into(),
            };
        }
        return Continuation::Partial {
            unresolved: vec!["Deliverable still missing after one correction.".into()],
        };
    }
    Continuation::Proceed {
        evidence: evidence_text(tool_name, result),
    }
}

/// Successful search is evidence in both modes. It never forces tools off.
pub fn search_disables_tools(_search_ok: bool, _plan_mode: bool) -> bool {
    false
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseDisposition {
    Execute,
    Repair { error: String },
    Fail { error: String },
}

/// Malformed input is never executed. One repair carries a bounded error.
pub fn malformed_call_disposition(valid: bool, repairs_used: u32, error: &str) -> ParseDisposition {
    if valid {
        return ParseDisposition::Execute;
    }
    let bounded: String = error.chars().take(240).collect();
    if repairs_used == 0 {
        ParseDisposition::Repair { error: bounded }
    } else {
        ParseDisposition::Fail { error: bounded }
    }
}

/// Research worker output is evidence for the main model, not a second answer.
pub fn as_research_evidence(mut value: Value) -> Value {
    if let Some(answer) = value.get("answer").cloned() {
        if !answer.is_null() {
            value["evidence"] = answer;
        }
    }
    value["synthesis"] = Value::String("main_model".into());
    value
}

pub fn call_signature(name: &str, arguments: &Value) -> String {
    format!("{name}:{}", arguments)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ok_search() -> Value {
        json!({"answer": "Partial hit.", "sources": {"S1": {"url": "https://example.com"}}})
    }

    #[test]
    fn search_success_keeps_tools_in_ordinary_chat_and_plan_mode() {
        let budgets = Budgets::default();
        for plan_mode in [false, true] {
            assert!(!search_disables_tools(true, plan_mode));
            match continuation_after_tool(plan_mode, "web_search", &ok_search(), &budgets, 0, false) {
                Continuation::Proceed { evidence } => {
                    assert_eq!(evidence.as_deref(), Some("Partial hit."));
                }
                other => panic!("search must not finish the turn: {other:?}"),
            }
        }
    }

    #[test]
    fn failed_page_still_allows_another_source() {
        let result = json!({"isError": true, "message": "Page unavailable: HTTP 404"});
        let next = continuation_after_tool(false, "web_read", &result, &Budgets::default(), 0, false);
        assert!(matches!(next, Continuation::Proceed { .. }));
    }

    #[test]
    fn budget_exhaustion_names_unresolved_work_and_reserves_a_final_response() {
        let mut budgets = Budgets::default();
        budgets.searches_used = budgets.max_searches;
        match continuation_after_tool(false, "web_search", &ok_search(), &budgets, 0, false) {
            Continuation::Partial { unresolved } => {
                assert!(unresolved.iter().any(|item| item.contains("Budget exhausted")));
            }
            other => panic!("expected a partial result, got {other:?}"),
        }
        assert!(budgets.reserve_final_response());
    }

    #[test]
    fn ambiguous_failure_is_not_retried_and_denial_stays_denial() {
        let timed_out = classify_execution_failure(
            "Tool request timed out. Its remote outcome may be unknown; do not automatically retry.",
        );
        assert!(timed_out.ambiguous && !timed_out.retryable);
        assert_eq!(timed_out.side_effect, SideEffect::Possible);
        let denied = classify_execution_failure("Permission denied for this action.");
        assert!(denied.denied && !denied.retryable);
        let stop = continuation_after_tool(
            false,
            "file_write",
            &json!({"isError": true, "message": "Tool request timed out. Its remote outcome may be unknown."}),
            &Budgets::default(),
            0,
            false,
        );
        assert!(matches!(stop, Continuation::Stop { retryable: false, .. }));
        let denial = continuation_after_tool(
            true,
            "run_code",
            &json!({"isError": true, "denied": true, "message": "denied"}),
            &Budgets::default(),
            0,
            false,
        );
        assert!(matches!(denial, Continuation::Stop { retryable: false, .. }));
    }

    #[test]
    fn a_missing_deliverable_gets_one_correction_then_stops() {
        let budgets = Budgets::default();
        let first = continuation_after_tool(false, "file_write", &json!({"isError": true, "message": "missing file"}), &budgets, 0, true);
        assert!(matches!(first, Continuation::CorrectOnce { .. }));
        let second = continuation_after_tool(false, "file_write", &json!({"isError": true, "message": "missing file"}), &budgets, 1, true);
        match second {
            Continuation::Partial { unresolved } => {
                assert!(unresolved.iter().any(|item| item.contains("one correction")));
            }
            other => panic!("expected stop after one correction, got {other:?}"),
        }
    }

    #[test]
    fn malformed_calls_are_not_executed_and_repair_once() {
        assert!(matches!(
            malformed_call_disposition(false, 0, "invalid json"),
            ParseDisposition::Repair { .. }
        ));
        assert!(matches!(
            malformed_call_disposition(false, 1, "invalid json"),
            ParseDisposition::Fail { .. }
        ));
        assert!(matches!(
            malformed_call_disposition(true, 0, ""),
            ParseDisposition::Execute
        ));
        let long = "x".repeat(500);
        match malformed_call_disposition(false, 0, &long) {
            ParseDisposition::Repair { error } => assert!(error.len() <= 240),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn research_output_is_evidence_for_the_main_model() {
        let value = as_research_evidence(json!({"answer": "Worker wrote a full answer.", "sources": {}}));
        assert_eq!(value["synthesis"], "main_model");
        assert_eq!(value["evidence"], "Worker wrote a full answer.");
    }
}
