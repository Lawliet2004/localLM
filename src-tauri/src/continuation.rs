//! Decisions for one assistant turn: whether tools stay available, whether a
//! failure may be repeated, and when a partial answer must name unfinished work.
//! The chat loop calls these; tests call the same functions.

use serde_json::Value;

/// One admission boundary for direct and nested calls. The shared family
/// counter is reserved before dispatch; unreadable state never grants a run.
pub(crate) fn admit_tool(
    state: &crate::AppState,
    run_id: &str,
    name: &str,
    arguments: &Value,
) -> Result<(), String> {
    let cell = state
        .turn_budgets
        .lock()
        .map_err(|_| "The turn tool-budget registry is unavailable; no tool was dispatched.")?
        .get(run_id)
        .cloned();
    // Standalone harness/test contexts need not own a registered parent turn.
    if let Some(cell) = cell {
        let mut budget = cell
            .lock()
            .map_err(|_| "The turn tool-budget state is unavailable; no tool was dispatched.")?;
        if !budget.try_note_tool_units(name, budget_units(name, arguments)) {
            return Err(
                "The turn search/fetch budget is exhausted; no tool was dispatched.".into(),
            );
        }
    }
    Ok(())
}

/// How many budget units one call of `name` consumes.
///
/// The research tools accept a batch: `search` carries up to four queries and
/// `visit` up to four URLs, and the worker issues one provider request per
/// element. Charging one unit per call would let a single admitted call spend
/// four times the configured budget.
fn budget_units(name: &str, arguments: &Value) -> u32 {
    let count = |value: &Value| -> u32 {
        match value {
            Value::Array(items) => items.len().max(1) as u32,
            _ => 1,
        }
    };
    match name {
        "search" => count(&arguments["query"]),
        "visit" => count(&arguments["url"]),
        _ => 1,
    }
}

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
    pub fn note_round(&mut self) {
        self.rounds_used = self.rounds_used.saturating_add(1);
    }
    pub fn exhausted(&self) -> bool {
        self.rounds_used >= self.max_rounds
            || self.searches_used >= self.max_searches
            || self.fetches_used >= self.max_fetches
    }

    /// True when the next model round must be the partial answer, not another tool.
    pub fn reserve_final_response(&self) -> bool {
        self.exhausted() || self.rounds_used + 1 >= self.max_rounds
    }

    /// Atomically charge a tool name against its bounded family counter and
    /// report whether the call may proceed. `false` — without charging — when
    /// that family's shared turn budget is already exhausted; unbudgeted
    /// names always pass. Nested dispatch paths must call this *before*
    /// running the tool so loops cannot spend budget the turn no longer has.
    pub fn try_note_tool(&mut self, name: &str) -> bool {
        self.try_note_tool_units(name, 1)
    }
    /// Charge `units` budget units for one call of `name`.
    ///
    /// A `search` call may carry up to four queries and a `visit` call up to
    /// four URLs, and the worker issues one provider request per element.
    /// Charging a single unit per call would let one admitted call spend
    /// several times the configured budget.
    pub fn try_note_tool_units(&mut self, name: &str, units: u32) -> bool {
        let units = units.max(1);
        if matches!(name, "web_search" | "search") {
            if self.searches_used.saturating_add(units) > self.max_searches {
                return false;
            }
            self.searches_used = self.searches_used.saturating_add(units);
        }
        if matches!(
            name,
            "weather"
                | "web_read"
                | "web_open"
                | "web_fetch"
                | "web_fetch_url"
                | "web_find"
                | "visit"
        ) {
            if self.fetches_used.saturating_add(units) > self.max_fetches {
                return false;
            }
            self.fetches_used = self.fetches_used.saturating_add(units);
        }
        true
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Continuation {
    /// Tools stay available. Evidence is not a completion signal.
    Proceed {
        evidence: Option<String>,
    },
    CorrectOnce {
        reason: String,
    },
    Partial {
        unresolved: Vec<String>,
    },
    Stop {
        message: String,
        retryable: bool,
    },
}

/// True when the result is a model-visible failure — either the legacy
/// `isError` marker or a typed envelope status.
fn result_failed(result: &Value) -> bool {
    result["isError"] == Value::Bool(true)
        || matches!(
            result.get("status").and_then(Value::as_str),
            Some("failure" | "cancelled" | "unknown")
        )
}

/// The human-readable failure text: flat `message`, then the nested envelope
/// `error.message` (A05: continuation reads typed metadata first).
fn result_error_message(result: &Value) -> &str {
    result["message"]
        .as_str()
        .or_else(|| result.pointer("/error/message").and_then(Value::as_str))
        .or_else(|| result.get("error").and_then(Value::as_str))
        .unwrap_or("Tool failed.")
}

/// Typed outcome certainty from the envelope: `metadata.sideEffects` /
/// `error.sideEffects` marking unknown writes, and cancelled/unknown
/// statuses. Such outcomes must never be automatically replayed.
fn outcome_uncertain(result: &Value) -> bool {
    matches!(
        result.get("status").and_then(Value::as_str),
        Some("cancelled" | "unknown")
    ) || matches!(
        result.pointer("/error/sideEffects").and_then(Value::as_str),
        Some("unknown" | "rolled_back")
    ) || matches!(
        result
            .pointer("/metadata/sideEffects")
            .and_then(Value::as_str),
        Some("unknown")
    )
}

fn evidence_text(tool_name: &str, result: &Value) -> Option<String> {
    if !matches!(tool_name, "web_search" | "search") || result_failed(result) {
        return None;
    }
    result["evidence"]
        .as_str()
        .or_else(|| result.pointer("/data/evidence").and_then(Value::as_str))
        .or_else(|| result["answer"].as_str())
        .or_else(|| result.pointer("/data/answer").and_then(Value::as_str))
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
        || result.get("code").and_then(Value::as_str) == Some("denied")
        || result.pointer("/error/code").and_then(Value::as_str) == Some("denied");
    if denied {
        return Continuation::Stop {
            message: "The action stays denied.".into(),
            retryable: false,
        };
    }
    if result_failed(result) {
        let message = result_error_message(result);
        let class = classify_execution_failure(message);
        if class.denied || class.ambiguous || outcome_uncertain(result) {
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

/// One corrective round when a mutated run finishes without checks.
pub const VERIFY_BEFORE_FINISH: &str = "You changed files but have not run checks. Run check_run (or explain why checks are not applicable) before finishing.";

pub const WEB_LOOKUP_RETRY: &str = "The offered web search tool is available, and no web lookup has been attempted for this request. Use it to check public current information before claiming the data or internet access is unavailable. Follow the original request, preserve source links and uncertainty, and respect any explicit instruction not to browse. Do not invent data.";

/// One correction for a false capability refusal, not an unbounded research
/// loop. Actual retrieval failures and explicit no-browsing requests stand.
pub fn retry_missing_web_lookup(
    answer: &str,
    request: &str,
    search_available: bool,
    lookup_attempted: bool,
    already_retried: bool,
) -> bool {
    if !search_available || lookup_attempted || already_retried {
        return false;
    }
    let request = request.to_lowercase().replace('’', "'");
    if [
        "do not browse",
        "don't browse",
        "do not search",
        "don't search",
        "without browsing",
        "without searching",
        "no internet",
        "offline only",
        "from memory",
        "training data only",
    ]
    .iter()
    .any(|phrase| request.contains(phrase))
    {
        return false;
    }
    let answer = answer.to_lowercase().replace('’', "'");
    let refusal = [
        "do not have",
        "don't have",
        "cannot",
        "can't",
        "unable",
        "lack access",
    ]
    .iter()
    .any(|phrase| answer.contains(phrase));
    let live_capability = [
        "real-time",
        "real time",
        "internet access",
        "access to the internet",
        "browse the",
        "search the web",
        "live access",
        "live data",
        "up-to-date",
    ]
    .iter()
    .any(|phrase| answer.contains(phrase));
    refusal && live_capability
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
    if let Some(answer) = value
        .as_object_mut()
        .and_then(|object| object.remove("answer"))
    {
        if !answer.is_null() {
            value["evidence"] = answer;
        }
    }
    value["synthesis"] = Value::String("main_model".into());
    value
}

/// Model aliases and discovery wrappers can name the same operation. Replay
/// protection keys the original backend identity and prepared arguments.
pub(crate) fn tool_signature(tool: &crate::connectors::AgentTool, arguments: &Value) -> String {
    use sha2::{Digest, Sha256};
    let identity = serde_json::json!([
        tool.connector,
        tool.tool.name,
        tool.signature_arguments(arguments)
    ]);
    format!("tool:{:x}", Sha256::digest(identity.to_string().as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    /// A `search` call may carry up to four queries and a `visit` call up to
    /// four URLs, and the worker issues one provider request per element.
    /// Charging one unit per call would let a single admitted call spend four
    /// times the configured budget.
    #[test]
    fn a_batched_search_charges_one_unit_per_query() {
        let mut budget = Budgets {
            max_searches: 3,
            ..Default::default()
        };
        assert!(budget.try_note_tool_units("search", 1));
        assert!(budget.try_note_tool_units("search", 2));
        assert_eq!(budget.searches_used, 3);
        // The next unit is refused: the budget is spent.
        assert!(!budget.try_note_tool_units("search", 1));
    }

    #[test]
    fn a_batched_visit_charges_one_unit_per_url() {
        let mut budget = Budgets {
            max_fetches: 2,
            ..Default::default()
        };
        assert!(budget.try_note_tool_units("visit", 2));
        assert_eq!(budget.fetches_used, 2);
        assert!(!budget.try_note_tool_units("visit", 1));
    }

    /// The admitted unit count follows the batch the arguments carry.
    #[test]
    fn the_admitted_units_follow_the_argument_batch() {
        let root = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(root.path());
        let budget = std::sync::Arc::new(std::sync::Mutex::new(Budgets {
            max_searches: 2,
            ..Default::default()
        }));
        state
            .turn_budgets
            .lock()
            .unwrap()
            .insert("batch-run".into(), budget.clone());
        let four = json!({"query": ["a", "b", "c", "d"]});
        // Four queries cannot fit in a two-search budget.
        assert!(admit_tool(&state, "batch-run", "search", &four).is_err());
        let one = json!({"query": ["a"]});
        assert!(admit_tool(&state, "batch-run", "search", &one).is_ok());
    }

    #[test]
    fn unsupported_live_data_refusal_gets_one_lookup_correction() {
        let answer = "I do not have access to real-time, proprietary benchmark data for these specific models.";
        let request = "Compare these models with popular benchmarks.";
        assert!(retry_missing_web_lookup(
            answer, request, true, false, false
        ));
        assert!(!retry_missing_web_lookup(
            answer, request, false, false, false
        ));
        assert!(!retry_missing_web_lookup(
            answer, request, true, true, false
        ));
        assert!(!retry_missing_web_lookup(
            answer, request, true, false, true
        ));
        assert!(!retry_missing_web_lookup(
            "The search did not verify this score.",
            request,
            true,
            false,
            false
        ));
        assert!(!retry_missing_web_lookup(
            "The benchmark score is 77.9%.",
            request,
            true,
            false,
            false
        ));
        for request in [
            "Do not browse; answer from memory.",
            "Don't search the web.",
            "Explain using training data only.",
        ] {
            assert!(!retry_missing_web_lookup(
                answer, request, true, false, false
            ));
        }
    }
    #[test]
    fn round_budget_reserves_a_final_response_after_real_rounds() {
        let mut budgets = Budgets {
            max_rounds: 3,
            ..Budgets::default()
        };
        assert!(!budgets.reserve_final_response());
        budgets.note_round(); // Planning or ordinary response both consume a round.
        budgets.note_round();
        assert!(budgets.reserve_final_response());
        assert!(matches!(
            continuation_after_tool(
                false,
                "file_read",
                &json!({"text":"data"}),
                &budgets,
                0,
                false
            ),
            Continuation::Partial { .. }
        ));
        budgets.note_round();
        assert!(budgets.exhausted());
    }

    fn ok_search() -> Value {
        json!({"answer": "Partial hit.", "sources": {"S1": {"url": "https://example.com"}}})
    }

    #[test]
    fn search_success_keeps_tools_in_ordinary_chat_and_plan_mode() {
        let budgets = Budgets::default();
        for plan_mode in [false, true] {
            assert!(!search_disables_tools(true, plan_mode));
            match continuation_after_tool(plan_mode, "web_search", &ok_search(), &budgets, 0, false)
            {
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
        let next =
            continuation_after_tool(false, "web_read", &result, &Budgets::default(), 0, false);
        assert!(matches!(next, Continuation::Proceed { .. }));
    }

    #[test]
    fn budget_exhaustion_names_unresolved_work_and_reserves_a_final_response() {
        let mut budgets = Budgets::default();
        budgets.searches_used = budgets.max_searches;
        match continuation_after_tool(false, "web_search", &ok_search(), &budgets, 0, false) {
            Continuation::Partial { unresolved } => {
                assert!(unresolved
                    .iter()
                    .any(|item| item.contains("Budget exhausted")));
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
        assert!(matches!(
            stop,
            Continuation::Stop {
                retryable: false,
                ..
            }
        ));
        let denial = continuation_after_tool(
            true,
            "run_code",
            &json!({"isError": true, "denied": true, "message": "denied"}),
            &Budgets::default(),
            0,
            false,
        );
        assert!(matches!(
            denial,
            Continuation::Stop {
                retryable: false,
                ..
            }
        ));
    }

    #[test]
    fn a_missing_deliverable_gets_one_correction_then_stops() {
        let budgets = Budgets::default();
        let first = continuation_after_tool(
            false,
            "file_write",
            &json!({"isError": true, "message": "missing file"}),
            &budgets,
            0,
            true,
        );
        assert!(matches!(first, Continuation::CorrectOnce { .. }));
        let second = continuation_after_tool(
            false,
            "file_write",
            &json!({"isError": true, "message": "missing file"}),
            &budgets,
            1,
            true,
        );
        match second {
            Continuation::Partial { unresolved } => {
                assert!(unresolved
                    .iter()
                    .any(|item| item.contains("one correction")));
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
        let value =
            as_research_evidence(json!({"answer": "Worker wrote a full answer.", "sources": {}}));
        assert_eq!(value["synthesis"], "main_model");
        assert_eq!(value["evidence"], "Worker wrote a full answer.");
        assert!(
            value.get("answer").is_none(),
            "do not duplicate the research text"
        );
        assert_eq!(as_research_evidence(value.clone()), value);
    }
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    #[tokio::test]
    async fn mixed_callers_can_reserve_the_last_family_permit_only_once() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        let budget = Arc::new(Mutex::new(Budgets {
            max_searches: 16,
            searches_used: 15,
            ..Default::default()
        }));
        state
            .turn_budgets
            .lock()
            .unwrap()
            .insert("shared-run".into(), budget.clone());
        let barrier = std::sync::Barrier::new(8);
        let accepted = std::thread::scope(|scope| {
            let threads = (0..8)
                .map(|n| {
                    let state = &state;
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        admit_tool(
                            state,
                            "shared-run",
                            if n % 2 == 0 { "search" } else { "web_search" },
                            &json!({}),
                        )
                        .is_ok()
                    })
                })
                .collect::<Vec<_>>();
            threads
                .into_iter()
                .map(|thread| usize::from(thread.join().unwrap()))
                .sum::<usize>()
        });
        assert_eq!(accepted, 1);
        assert_eq!(budget.lock().unwrap().searches_used, 16);
        assert!(admit_tool(&state, "shared-run", "web_search", &json!({})).is_err());
    }
    #[tokio::test]
    async fn family_denial_does_not_spend_another_family_or_an_extra_permit() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        let budget = Arc::new(Mutex::new(Budgets {
            max_searches: 1,
            max_fetches: 0,
            ..Default::default()
        }));
        state
            .turn_budgets
            .lock()
            .unwrap()
            .insert("shared-run".into(), budget.clone());
        assert!(admit_tool(&state, "shared-run", "web_fetch", &json!({})).is_err());
        assert_eq!(budget.lock().unwrap().fetches_used, 0);
        admit_tool(&state, "shared-run", "web_search", &json!({})).unwrap();
        assert!(admit_tool(&state, "shared-run", "search", &json!({})).is_err());
        assert_eq!(budget.lock().unwrap().searches_used, 1);
        admit_tool(&state, "shared-run", "workspace_read_file", &json!({})).unwrap();
    }
}
