use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Preparing,
    Generating,
    AwaitingApproval,
    ExecutingTools,
    PreparingNextRound,
    Completed,
    Cancelled,
    Failed,
    OutcomeUnknown,
}

impl RunState {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunState::Preparing => "preparing",
            RunState::Generating => "generating",
            RunState::AwaitingApproval => "awaiting_approval",
            RunState::ExecutingTools => "executing_tools",
            RunState::PreparingNextRound => "preparing_next_round",
            RunState::Completed => "completed",
            RunState::Cancelled => "cancelled",
            RunState::Failed => "failed",
            RunState::OutcomeUnknown => "outcome_unknown",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "preparing" => Some(RunState::Preparing),
            "generating" => Some(RunState::Generating),
            "awaiting_approval" => Some(RunState::AwaitingApproval),
            "executing_tools" => Some(RunState::ExecutingTools),
            "preparing_next_round" => Some(RunState::PreparingNextRound),
            "completed" => Some(RunState::Completed),
            "cancelled" => Some(RunState::Cancelled),
            "failed" => Some(RunState::Failed),
            "outcome_unknown" => Some(RunState::OutcomeUnknown),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            RunState::Completed | RunState::Cancelled | RunState::Failed | RunState::OutcomeUnknown
        )
    }

    pub fn can_transition_to(&self, next: RunState) -> bool {
        if self.is_terminal() {
            return false;
        }
        match (self, next) {
            // Cancellation or failure can happen from any active state
            (_, RunState::Cancelled) | (_, RunState::Failed) | (_, RunState::OutcomeUnknown) => true,
            (RunState::Preparing, RunState::Generating) => true,
            (RunState::Generating, RunState::Completed) => true,
            (RunState::Generating, RunState::AwaitingApproval) => true,
            (RunState::Generating, RunState::ExecutingTools) => true,
            (RunState::Generating, RunState::PreparingNextRound) => true,
            (RunState::AwaitingApproval, RunState::ExecutingTools) => true,
            (RunState::AwaitingApproval, RunState::PreparingNextRound) => true, // When user denies
            (RunState::ExecutingTools, RunState::ExecutingTools) => true, // Batch: multiple calls in one turn
            (RunState::ExecutingTools, RunState::AwaitingApproval) => true, // Batch: later call needs approval
            (RunState::ExecutingTools, RunState::PreparingNextRound) => true,
            // An in-loop `finish` returns straight to completion while the run
            // is still in ExecutingTools — without this edge the record stays
            // "executing_tools" forever and active_run reports it as live.
            (RunState::ExecutingTools, RunState::Completed) => true,
            (RunState::PreparingNextRound, RunState::Generating) => true,
            (RunState::PreparingNextRound, RunState::Completed) => true,
            _ => false,
        }
    }
}

impl fmt::Display for RunState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// User-facing pause / cancel / resume / restart. These sit beside `RunState`
/// so an in-flight generation state does not have to grow a second enum.
#[derive(Clone, Debug, Default)]
pub struct RunControl {
    pub active_generation: u64,
    pub cancel_generation: u64,
    pub pause: bool,
}

/// Cancel applies only to the generation that was active when it was requested.
pub fn cancel_matches(cancel_generation: u64, run_generation: u64) -> bool {
    cancel_generation != 0 && cancel_generation == run_generation
}

pub fn apply_pause(control: &mut RunControl) {
    control.pause = true;
}

pub fn begin_run(control: &mut RunControl) -> u64 {
    control.active_generation = control.active_generation.saturating_add(1);
    control.pause = false;
    control.active_generation
}

pub fn request_cancel(control: &mut RunControl) -> u64 {
    control.cancel_generation = control.active_generation;
    control.cancel_generation
}

/// Resume gets a new generation. A cancel recorded for the previous one does not apply.
pub fn resume_run(control: &mut RunControl) -> u64 {
    control.pause = false;
    control.active_generation = control.active_generation.saturating_add(1);
    control.active_generation
}

pub fn pause_blocks_new_tool(pause_requested: bool) -> bool {
    pause_requested
}

pub fn pause_notice(tool_in_flight: bool) -> &'static str {
    if tool_in_flight {
        "Pausing after current action."
    } else {
        "Paused before the next action."
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LedgerOp {
    pub id: String,
    pub kind: String,
    pub status: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RestartReport {
    pub replayed_write_ids: Vec<String>,
    pub unconditional_success: bool,
}

/// Completed operations stay. An interrupted write is not replayed. In-flight
/// work becomes outcome-unknown and must not be shown as success.
pub fn restart_operations(ops: &mut [LedgerOp]) -> RestartReport {
    for op in ops.iter_mut() {
        if matches!(op.status.as_str(), "running" | "awaiting_approval") {
            op.status = "outcome_unknown".into();
        }
    }
    RestartReport {
        replayed_write_ids: Vec::new(),
        unconditional_success: false,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub id: String,
    pub conversation_id: String,
    pub status: RunState,
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
    pub checkpoint: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl RunRecord {
    pub fn transition_to(&mut self, next: RunState) -> Result<(), String> {
        if !self.status.can_transition_to(next) {
            return Err(format!("Invalid run transition from {} to {}", self.status, next));
        }
        self.status = next;
        self.updated_at = crate::store::now();
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEvent {
    pub run_id: String,
    pub seq: u64,
    pub step_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    pub event_type: String,
    pub payload: Value,
    pub created_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn live_model_run_code(base: &str) -> (String, serde_json::Value, u32) {
        let client = reqwest::Client::builder().build().unwrap();
        let request = serde_json::json!({
            "messages": [{
                "role": "user",
                "content": "Call the run_code tool. Set its code argument to exactly round(19.99 * 1.08, 2). Do not calculate the number yourself."
            }],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "run_code",
                    "description": "Run Python. The last expression is the result.",
                    "parameters": {
                        "type": "object",
                        "properties": {"code": {"type": "string"}},
                        "required": ["code"]
                    }
                }
            }],
            "tool_choice": {"type": "function", "function": {"name": "run_code"}},
            "temperature": 0,
            "max_tokens": 256,
            "chat_template_kwargs": {"enable_thinking": false}
        });
        let response = client.post(format!("{base}/v1/chat/completions"))
            .json(&request)
            .send().await
            .unwrap_or_else(|error| panic!("model request failed: {error}"))
            .text().await
            .unwrap_or_else(|error| panic!("model response body failed: {error}"));
        let parsed: serde_json::Value = serde_json::from_str(&response).unwrap_or_else(|_| panic!("model response was not JSON: {response}"));
        let message = &parsed["choices"][0]["message"];
        let calls = match crate::tool_calls::ToolCalls::from_openai_message(message) {
            Ok(calls) if !calls.is_empty() => calls,
            _ => {
                let content = message["content"].as_str().unwrap_or("");
                match crate::chat::decide_text_tool_call(content, 0) {
                    crate::chat::TextCallDecision::Execute(parsed_calls) => parsed_calls.finish().unwrap_or_else(|error| panic!("{error}: {response}")),
                    _ => panic!("model did not invoke run_code: {response}"),
                }
            }
        };
        let name = calls[0].name.clone();
        let arguments = calls[0].arguments.clone();
        if name != "run_code" {
            panic!("model invoked {name} instead of run_code: {response}");
        }
        (name, arguments, 1)
    }

    #[test]
    fn valid_transitions() {
        assert!(RunState::Preparing.can_transition_to(RunState::Generating));
        assert!(RunState::Generating.can_transition_to(RunState::AwaitingApproval));
        assert!(RunState::AwaitingApproval.can_transition_to(RunState::ExecutingTools));
        assert!(RunState::ExecutingTools.can_transition_to(RunState::ExecutingTools));
        assert!(RunState::ExecutingTools.can_transition_to(RunState::AwaitingApproval));
        assert!(RunState::ExecutingTools.can_transition_to(RunState::PreparingNextRound));
        assert!(RunState::ExecutingTools.can_transition_to(RunState::Completed));
        assert!(RunState::PreparingNextRound.can_transition_to(RunState::Generating));
        assert!(RunState::Generating.can_transition_to(RunState::Completed));
    }

    #[test]
    fn terminal_states_reject_transitions() {
        assert!(!RunState::Completed.can_transition_to(RunState::Generating));
        assert!(!RunState::Cancelled.can_transition_to(RunState::ExecutingTools));
        assert!(!RunState::Failed.can_transition_to(RunState::PreparingNextRound));
        assert!(!RunState::OutcomeUnknown.can_transition_to(RunState::Completed));
    }

    #[test]
    fn any_active_state_can_cancel_or_fail() {
        for state in [
            RunState::Preparing,
            RunState::Generating,
            RunState::AwaitingApproval,
            RunState::ExecutingTools,
            RunState::PreparingNextRound,
        ] {
            assert!(state.can_transition_to(RunState::Cancelled));
            assert!(state.can_transition_to(RunState::Failed));
            assert!(state.can_transition_to(RunState::OutcomeUnknown));
        }
    }

    #[test]
    fn pause_starts_no_new_tool_and_names_an_in_flight_action() {
        assert!(super::pause_blocks_new_tool(true));
        assert!(!super::pause_blocks_new_tool(false));
        assert_eq!(super::pause_notice(true), "Pausing after current action.");
        assert_eq!(super::pause_notice(false), "Paused before the next action.");
    }

    #[test]
    fn cancel_cannot_hit_a_later_resumed_run() {
        let mut control = super::RunControl::default();
        let first = super::begin_run(&mut control);
        super::request_cancel(&mut control);
        assert!(super::cancel_matches(control.cancel_generation, first));
        let second = super::resume_run(&mut control);
        assert_ne!(first, second);
        assert!(!super::cancel_matches(control.cancel_generation, second));
        assert!(!control.pause);
    }

    #[test]
    fn restart_keeps_completed_work_and_does_not_replay_a_write() {
        let mut ops = vec![
            super::LedgerOp { id: "done".into(), kind: "file_write".into(), status: "succeeded".into() },
            super::LedgerOp { id: "write".into(), kind: "file_write".into(), status: "running".into() },
            super::LedgerOp { id: "read".into(), kind: "file_read".into(), status: "awaiting_approval".into() },
        ];
        let report = super::restart_operations(&mut ops);
        assert_eq!(ops[0].status, "succeeded");
        assert_eq!(ops[1].status, "outcome_unknown");
        assert_eq!(ops[2].status, "outcome_unknown");
        assert!(report.replayed_write_ids.is_empty());
        assert!(!report.unconditional_success);
    }

    #[test]
    fn shipped_packaged_entry_saves_through_workspace_and_recovers() {
        let malformed = "<tool_call>{not-json";
        let mut malformed_executions = 0u32;
        let mut repairs = 0u32;
        match crate::chat::decide_text_tool_call(malformed, repairs) {
            crate::chat::TextCallDecision::Repair { error } => {
                assert!(!error.is_empty());
                assert!(error.len() <= 240);
                repairs += 1;
            }
            crate::chat::TextCallDecision::Execute(_) => malformed_executions += 1,
            crate::chat::TextCallDecision::Fail { .. } | crate::chat::TextCallDecision::NotACall => {
                panic!("malformed call was not repaired; executions={malformed_executions}");
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let workspace_path = std::env::var("LOCALLM_WORKSPACE").unwrap_or_else(|_| dir.path().to_str().unwrap().to_string());
        std::fs::create_dir_all(&workspace_path).unwrap();
        let db_path = std::path::PathBuf::from(&workspace_path).join("entry.sqlite");
        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(std::path::Path::new(&workspace_path).join("done.txt"));
        let _ = std::fs::remove_file(std::path::Path::new(&workspace_path).join("result.txt"));
        let config = crate::execution::ExecutionConfig::default();
        let store = crate::store::Store::open(&db_path).unwrap();
        let mut task = crate::research_tasks::ResearchTask::new("conv", "keep the tariff", crate::research_tasks::TaskBudgets::default()).unwrap();
        task.conversation_id = store.create_conversation().unwrap().id;
        store.save_research_task(&task).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let (later_tool, arguments, model_invoked) = if let Ok(path) = std::env::var("LOCALLM_MODEL_MESSAGE") {
            let raw = std::fs::read_to_string(path).unwrap();
            let message: serde_json::Value = serde_json::from_str(&raw).unwrap();
            let calls = match crate::tool_calls::ToolCalls::from_openai_message(&message) {
                Ok(calls) if !calls.is_empty() => calls,
                _ => {
                    let content = message["content"].as_str().unwrap_or("");
                    match crate::chat::decide_text_tool_call(content, repairs) {
                        crate::chat::TextCallDecision::Execute(parsed) => parsed.finish().unwrap(),
                        _ => panic!("model message did not invoke a tool: {raw}"),
                    }
                }
            };
            (calls[0].name.clone(), calls[0].arguments.clone(), 1u32)
        } else if let Ok(base) = std::env::var("LOCALLM_LIVE_BASE") {
            runtime.block_on(live_model_run_code(&base))
        } else {
            let valid = "<tool_call>{\"name\":\"run_code\",\"arguments\":{\"code\":\"round(19.99 * 1.08, 2)\"}}</tool_call>";
            match crate::chat::decide_text_tool_call(valid, repairs) {
                crate::chat::TextCallDecision::Execute(calls) => {
                    let calls = calls.finish().unwrap();
                    (calls[0].name.clone(), calls[0].arguments.clone(), 0)
                }
                crate::chat::TextCallDecision::Repair { .. } | crate::chat::TextCallDecision::Fail { .. } => {
                    malformed_executions += 1;
                    panic!("the repaired follow-up was not executed; executions={malformed_executions}");
                }
                crate::chat::TextCallDecision::NotACall => panic!("valid run_code markup was ignored; executions={malformed_executions}"),
            }
        };
        let model_code = arguments.get("code").and_then(|value| value.as_str()).unwrap_or("").replace(['\n', '\r'], " ");
        assert_eq!(later_tool, "run_code");
        if std::env::var("LOCALLM_MODEL_MESSAGE").is_ok() {
            assert_eq!(model_invoked, 1);
        }
        crate::chat::begin_catalog_op(&store, &mut task, "calc", "run_code");
        let ran = runtime.block_on(crate::chat::run_catalog_and_record(
            &db_path, &task.id, &workspace_path, config.clone(), "calc", "run_code", arguments,
        )).unwrap_or_else(|error| panic!("run_code failed: {error}"));
        let result = &ran["data"]["result"];
        let text = result.as_f64().map(|value| format!("{value:.2}")).or_else(|| result.as_str().map(|value| value.to_string())).unwrap_or_else(|| panic!("run_code result was {ran}"));
        assert_eq!(text, "21.59", "run_code result was {ran}; model code was {model_code}");
        assert_eq!(malformed_executions, 0);
        task = store.research_task(&task.id).unwrap().unwrap();
        crate::chat::begin_catalog_op(&store, &mut task, "done-file", "file_write");
        runtime.block_on(crate::chat::run_catalog_and_record(
            &db_path, &task.id, &workspace_path, config.clone(), "done-file", "file_write",
            serde_json::json!({"path": "done.txt", "content": "DONE"}),
        )).unwrap();
        task = store.research_task(&task.id).unwrap().unwrap();
        crate::chat::begin_catalog_op(&store, &mut task, "save-result", "file_write");
        runtime.block_on(crate::chat::run_catalog_and_record(
            &db_path, &task.id, &workspace_path, config.clone(), "save-result", "file_write",
            serde_json::json!({"path": "result.txt", "content": text}),
        )).unwrap();
        let read = runtime.block_on(crate::harness::execute_workspace_tool(
            &workspace_path, config.clone(), "file_read", serde_json::json!({"path": "result.txt"}),
        )).unwrap();
        let body = read["content"].as_str().unwrap();
        assert!(body.contains(&text));
        assert_eq!(body.matches(&text).count(), 1);

        let mut control = super::RunControl::default();
        let first = super::begin_run(&mut control);
        super::apply_pause(&mut control);
        let pause_blocks = super::pause_blocks_new_tool(control.pause);
        assert!(pause_blocks);
        assert_eq!(super::pause_notice(false), "Paused before the next action.");
        let resumed = super::resume_run(&mut control);
        assert!(!control.pause);
        assert_ne!(first, resumed);
        super::request_cancel(&mut control);
        assert!(super::cancel_matches(control.cancel_generation, resumed));
        let later = super::resume_run(&mut control);
        let cancel_blocks_later = super::cancel_matches(control.cancel_generation, later);

        task = store.research_task(&task.id).unwrap().unwrap();
        crate::chat::begin_catalog_op(&store, &mut task, "not-run", "file_write");
        let recovered = crate::research_tasks::recover_interrupted(&store).unwrap();
        assert!(recovered.replayed_write_ids.is_empty());
        assert!(!recovered.unconditional_success);
        let reloaded = store.research_task(&task.id).unwrap().unwrap();
        assert_eq!(reloaded.operations.iter().find(|op| op.id == "op-calc").unwrap().status, crate::research_tasks::OperationStatus::Completed);
        assert_eq!(reloaded.operations.iter().find(|op| op.id == "op-done-file").unwrap().status, crate::research_tasks::OperationStatus::Completed);
        assert_eq!(reloaded.operations.iter().find(|op| op.id == "op-save-result").unwrap().status, crate::research_tasks::OperationStatus::Completed);
        assert_eq!(reloaded.operations.iter().find(|op| op.id == "op-not-run").unwrap().status, crate::research_tasks::OperationStatus::Unknown);
        let again = runtime.block_on(crate::harness::execute_workspace_tool(
            &workspace_path, config, "file_read", serde_json::json!({"path": "result.txt"}),
        )).unwrap();
        assert_eq!(again["content"].as_str().unwrap().matches(&text).count(), 1);
        let staged = crate::chat::stage_paused_resume(&store, &task.conversation_id).unwrap();
        let mut messages = vec![serde_json::json!({"role":"user","content":"Continue from the paused checkpoint."})];
        let applied = crate::chat::take_staged_checkpoint(&store, &task.conversation_id, &mut messages);
        let continued = messages[0]["content"].as_str().unwrap().to_string();
        let resume_from_checkpoint = applied
            && staged.completed.iter().any(|id| id == "op-calc")
            && staged.completed.iter().all(|id| continued.contains(id.as_str()))
            && !staged.completed.iter().any(|id| id == "op-not-run")
            && continued.contains("Do not repeat them.");
        assert!(resume_from_checkpoint, "resume note was {continued}");
        assert!(!crate::chat::take_staged_checkpoint(&store, &task.conversation_id, &mut messages));
        let report = recovered;
        let request = "Compare the two sources. You must keep the tariff. Do not repeat a completed write.";
        let constraints = crate::compaction::explicit_constraints(request);
        let checkpoint = crate::compaction::checkpoint_from_audits(request, &constraints, &["file_write".into()], &[], &[], &["second source".into()], &[]);
        store.save_setting("task_checkpoint_v1", &serde_json::to_string(&checkpoint).unwrap()).unwrap();
        store.save_setting("offline_mode", &true).unwrap();
        drop(store);
        let store = crate::store::Store::open(&db_path).unwrap();
        let loaded: crate::compaction::TaskCheckpoint = serde_json::from_str(&store.setting::<String>("task_checkpoint_v1").unwrap()).unwrap();
        assert!(loaded.constraints.iter().any(|line| line.to_lowercase().contains("must keep")));
        let offline_retrieval = crate::local_only::retrieval_allowed(store.setting::<bool>("offline_mode").unwrap());
        assert!(!offline_retrieval);
        let remote = crate::local_only::inference_allowed("https://api.openai.com/v1");
        let remote_rejected = remote.as_ref().err().map(|error| error.contains("was not called")).unwrap_or(false);
        assert!(remote_rejected);
        let inflight = reloaded.operations.iter().find(|op| op.id == "op-not-run").unwrap().status.as_str();
        assert_eq!(inflight, "unknown");
        assert!(resume_from_checkpoint);
        assert!(!cancel_blocks_later);
        assert!(pause_blocks);
        if let Ok(out) = std::env::var("LOCALLM_LEDGER_OUT") {
            let payload = serde_json::json!({
                "calculated": text,
                "file": body,
                "done": std::fs::read_to_string(std::path::Path::new(&workspace_path).join("done.txt")).unwrap_or_default().trim().to_string(),
                "inflight": inflight,
                "replayed": report.replayed_write_ids.len(),
                "checkpoint": continued,
                "retrievalAllowed": offline_retrieval,
                "remoteError": remote.unwrap_err(),
                "modelInvoked": model_invoked,
                "modelCode": model_code,
                "constraints": loaded.constraints.len(),
            });
            std::fs::write(out, payload.to_string()).unwrap();
        }
        let _ = later;
    }

    #[test]
    fn roundtrip_state_strings() {
        for state in [
            RunState::Preparing,
            RunState::Generating,
            RunState::AwaitingApproval,
            RunState::ExecutingTools,
            RunState::PreparingNextRound,
            RunState::Completed,
            RunState::Cancelled,
            RunState::Failed,
            RunState::OutcomeUnknown,
        ] {
            assert_eq!(RunState::from_str(state.as_str()), Some(state));
        }
    }
}
