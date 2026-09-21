use crate::{inference::{Backend, InferenceProvider}, sse::SseDecoder, AppState};
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tauri::{ipc::Channel, State};

#[derive(Clone, Serialize, Debug, PartialEq)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ToolStreamEvent {
    Started {
        tool_call_id: String,
        tool_name: String,
        command: Option<String>,
        language: Option<String>,
        cwd: Option<String>,
    },
    OutputChunk {
        tool_call_id: String,
        stream: String,
        chunk: String,
    },
    Finished {
        tool_call_id: String,
        exit_code: Option<i32>,
        duration_ms: u128,
        error: Option<String>,
    },
}

#[derive(Default, Debug, Clone)]
pub struct ThinkFilter {
    pub inside_think: bool,
    pub buffer: String,
}

impl ThinkFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_thinking(&self) -> bool {
        self.inside_think
    }

    /// Feeds incremental tokens and returns (answer_chunk, reasoning_chunk)
    pub fn feed(&mut self, text: &str) -> (String, String) {
        let mut answer = String::new();
        let mut reasoning = String::new();

        for ch in text.chars() {
            self.buffer.push(ch);
            let target = if self.inside_think { "</think>" } else { "<think>" };

            if target.starts_with(&self.buffer) {
                if self.buffer == target {
                    self.buffer.clear();
                    self.inside_think = !self.inside_think;
                }
            } else {
                while !self.buffer.is_empty() && !target.starts_with(&self.buffer) {
                    let first_char = self.buffer.remove(0);
                    if self.inside_think {
                        reasoning.push(first_char);
                    } else {
                        answer.push(first_char);
                    }
                }
                if self.buffer == target {
                    self.buffer.clear();
                    self.inside_think = !self.inside_think;
                }
            }
        }

        (answer, reasoning)
    }

    pub fn flush(&mut self) -> (String, String) {
        let mut answer = String::new();
        let mut reasoning = String::new();
        if !self.buffer.is_empty() {
            if self.inside_think {
                reasoning.push_str(&self.buffer);
            } else {
                answer.push_str(&self.buffer);
            }
            self.buffer.clear();
        }
        (answer, reasoning)
    }
}

const FINAL_ANSWER_INSTRUCTION: &str =
    "Give the user a visible final answer now, using the conversation and tool results already collected. State any unresolved work or missing evidence honestly. Do not call tools, repeat progress updates, or expose private reasoning.";
const EMPTY_FINAL_ANSWER_ERROR: &str =
    "The model finished without a visible answer after tool work completed. Please retry the request.";
const RESEARCH_STALLED: &str = "Research stopped because repeated web calls made no progress or reached the research budget. Give the best answer supported by the results already collected, cite available sources, and clearly state any unresolved questions. Do not claim the research or unfinished plan steps are complete. Do not call tools.";
const RESEARCH_STALLED_FALLBACK: &str = "Research stopped after repeated unproductive web calls or reaching its research budget. The model could not produce a supported final answer. The collected tool results remain available; unresolved claims have not been verified.";

#[derive(Default)]
struct ResearchProgress {
    evidence: std::collections::HashSet<u64>,
    domains: std::collections::HashSet<String>,
    calls: usize,
    stagnant: usize,
}

impl ResearchProgress {
    fn observe(&mut self, name: &str, result: &Value) {
        if self.stopped() { return; }
        if !matches!(name, "web_search" | "web_open" | "web_fetch" | "web_fetch_url" | "web_find" | "search" | "visit" | "update_context") { return; }
        self.calls += 1;
        let before = self.evidence.len();
        let domains_before = self.domains.len();
        if name != "update_context" && result["isError"] != true {
            Self::collect(result, &mut self.evidence, &mut self.domains, false);
        }
        // Discovery calls only make progress by surfacing a new domain.
        // Junk-but-novel hits on the same engines (e.g. fresh arxiv papers for
        // a weather query) used to reset the counter forever.
        let progressed = if matches!(name, "web_search" | "search") {
            self.domains.len() > domains_before
        } else {
            self.evidence.len() > before
        };
        self.stagnant = if progressed { 0 } else { self.stagnant + 1 };
    }

    fn collect(
        value: &Value,
        seen: &mut std::collections::HashSet<u64>,
        domains: &mut std::collections::HashSet<String>,
        evidence: bool,
    ) {
        use std::hash::{Hash, Hasher};
        match value {
            Value::Object(fields) => {
                if fields.get("isError") == Some(&Value::Bool(true)) { return; }
                for (key, value) in fields {
                    // ponytail: explicit evidence fields ignore query IDs/timing; extend for new web result schemas.
                    let evidence = matches!(key.as_str(), "url" | "text" | "content" | "snippet" | "excerpt" | "excerpts" | "answer" | "markdown" | "passage" | "body");
                    if matches!(key.as_str(), "url" | "canonicalUrl" | "source_url") {
                        if let Some(host) = value.as_str().and_then(Self::host_of) {
                            domains.insert(host);
                        }
                    }
                    Self::collect(value, seen, domains, evidence);
                }
            }
            Value::Array(values) => for value in values { Self::collect(value, seen, domains, evidence); },
            Value::String(text) if evidence => {
                // MCP servers can wrap structured results in a text content block.
                if let Ok(nested) = serde_json::from_str::<Value>(text) {
                    if nested.is_object() || nested.is_array() {
                        Self::collect(&nested, seen, domains, false);
                        return;
                    }
                }
                let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
                if !normalized.is_empty() {
                    let mut hash = std::collections::hash_map::DefaultHasher::new();
                    normalized.hash(&mut hash);
                    seen.insert(hash.finish());
                }
            }
            _ => {}
        }
    }

    fn host_of(url: &str) -> Option<String> {
        let host = url.split("://").nth(1)?.split('/').next()?.split(':').next()?.trim();
        (!host.is_empty()).then(|| host.to_lowercase())
    }

    fn stopped(&self) -> bool { self.stagnant >= 3 || self.calls >= 16 }
}
const PLAN_MODE_INSTRUCTION: &str = "Plan mode is on. Phase 1 of 2: write a concise numbered checklist of the work, then stop. Each step is one action (search, open a source, calculate with local_run_code, edit a file, compare, verify). 3–12 steps. Do not call tools and do not implement yet.";
const PLAN_MODE_ACTIVE: &str = "Plan mode is on. Phase 1 is done. Phase 2: execute the checklist one task at a time.";
const PLAN_IMPLEMENT_INSTRUCTION: &str = "The plan above is ready. Implement it in order, one task at a time. Work only on the current in_progress task. When it is done, mark it completed with the offered todo tools (todo_write saves the whole list) and start the next pending task. Do not skip ahead or start two tasks at once. Use the offered web tools (search/visit or web_search/web_open) for facts, and local_run_code when offered for exact math or science. Continue through the plan while making progress, then summarize the answer or call finish when offered. If evidence remains unavailable, report the unresolved steps honestly.";
const PLAN_FOCUS_MARKER: &str = "The plan above is ready.";
const PLAN_SEPARATOR: &str = "\n\n---\n\n";
const EMPTY_PLAN_ERROR: &str =
    "The model returned an empty plan. Please retry the request.";

#[derive(Debug, PartialEq, Eq)]
enum FinalizationAction {
    Complete,
    Retry,
    Fallback(String),
    Error,
}

fn research_answer_fallback(tool_name: &str, result: &Value) -> Option<String> {
    if tool_name != "web_search" || result["isError"] == Value::Bool(true) {
        return None;
    }
    result["answer"]
        .as_str()
        .map(str::trim)
        .filter(|answer| !answer.is_empty())
        .map(String::from)
}

/// Ordinary chat may lock tools after the first grounded `web_search` answer.
/// Plan mode must keep tools so the model can finish every hop on the plan.
fn arm_research_finalization(plan_mode: bool, tool_name: &str, result: &Value) -> Option<String> {
    if plan_mode {
        None
    } else {
        research_answer_fallback(tool_name, result)
    }
}

fn replace_in_content(message: &mut Value, from: &str, to: &str) {
    match message.get_mut("content") {
        Some(Value::String(content)) => {
            *content = content.replacen(from, to, 1);
        }
        Some(Value::Array(parts)) => {
            for part in parts.iter_mut() {
                if let Some(Value::String(text)) = part.get_mut("text") {
                    if text.contains(from) {
                        *text = text.replacen(from, to, 1);
                        break;
                    }
                }
            }
        }
        _ => {}
    }
}

fn plan_focus_message(todos: &[crate::plans::Todo]) -> String {
    match crate::plans::current_task_line(todos) {
        Some(focus) => format!("{PLAN_IMPLEMENT_INSTRUCTION}\n\n{focus}"),
        None => PLAN_IMPLEMENT_INSTRUCTION.to_string(),
    }
}

fn is_plan_implement_message(message: &Value) -> bool {
    if message["role"] != "user" {
        return false;
    }
    match message.get("content") {
        Some(Value::String(text)) => text.contains(PLAN_FOCUS_MARKER),
        Some(Value::Array(parts)) => parts.iter().any(|part| {
            part.get("text")
                .and_then(|value| value.as_str())
                .is_some_and(|text| text.contains(PLAN_FOCUS_MARKER))
        }),
        _ => false,
    }
}

fn set_message_text(message: &mut Value, text: &str) {
    match message.get_mut("content") {
        Some(Value::String(content)) => {
            *content = text.to_string();
        }
        Some(Value::Array(parts)) => {
            if let Some(part) = parts.iter_mut().find(|part| part.get("text").is_some()) {
                part["text"] = json!(text);
            } else {
                message["content"] = json!(text);
            }
        }
        _ => {
            message["content"] = json!(text);
        }
    }
}

/// Keep the Phase-2 user turn pointed at the single current checklist item.
fn refresh_plan_focus(messages: &mut Vec<Value>, todos: &[crate::plans::Todo]) {
    let text = plan_focus_message(todos);
    if let Some(message) = messages.iter_mut().rev().find(|message| is_plan_implement_message(message)) {
        set_message_text(message, &text);
    } else {
        messages.push(json!({"role": "user", "content": text}));
    }
}

/// Park the plan on the transcript and retarget the draft so Phase 2 is not
/// still told "do not implement".
fn begin_plan_implementation(messages: &mut Vec<Value>, plan_text: &str) -> Result<(), String> {
    let plan_text = plan_text.trim();
    if plan_text.is_empty() {
        return Err(EMPTY_PLAN_ERROR.into());
    }
    if let Some(draft) = messages.iter_mut().rev().find(|message| message["role"] == "user") {
        replace_in_content(draft, PLAN_MODE_INSTRUCTION, PLAN_MODE_ACTIVE);
    }
    messages.push(json!({"role": "assistant", "content": plan_text}));
    messages.push(json!({"role": "user", "content": PLAN_IMPLEMENT_INSTRUCTION}));
    Ok(())
}

fn inject_plan_mode(plan: &mut TurnPlan, plan_mode: bool) {
    if plan_mode {
        plan.injections.push(("plan_mode".into(), PLAN_MODE_INSTRUCTION.into()));
    }
}

/// A research answer that cites no source is narration, not a result — small
/// models emit "Let me synthesize…" process talk that reads like an answer
/// but carries no grounded claim. Only enforced on stalled turns: ordinary
/// completions keep their text regardless of citation shape.
fn answer_cites_evidence(text: &str) -> bool {
    text.contains("http://") || text.contains("https://")
}

fn finalization_action(
    round_answer: &str,
    fallback: Option<&str>,
    retried: bool,
    stalled: bool,
) -> FinalizationAction {
    if !round_answer.trim().is_empty() && !is_text_tool_call(round_answer)
        && !(stalled && !answer_cites_evidence(round_answer))
    {
        return FinalizationAction::Complete;
    }
    if !retried {
        return FinalizationAction::Retry;
    }
    match fallback {
        Some(answer) => FinalizationAction::Fallback(format!("Research result:\n\n{answer}")),
        None => FinalizationAction::Error,
    }
}

fn needs_final_answer(finalizing: bool, has_calls: bool, round_answer: &str) -> bool {
    finalizing || (!has_calls && round_answer.trim().is_empty())
}

fn is_text_tool_call(text: &str) -> bool {
    let text = text.trim_start();
    if text.starts_with("<function=") || text.starts_with("<tool_call>") {
        return true;
    }
    // A call embedded after prose still counts. `<tool_call>` is unambiguous
    // markup even when left unterminated; a mid-text `<function=` needs its
    // closing tag so a literal mention in the answer is not misread as a call.
    let mut rest = text;
    while let Some(start) = [rest.find("<tool_call>"), rest.find("<function=")]
        .into_iter().flatten().min()
    {
        let after = &rest[start..];
        if after.starts_with("<tool_call>") {
            return true;
        }
        rest = &after["<function=".len()..];
        if rest.contains("</function>") {
            return true;
        }
    }
    false
}

/// Visible text left behind by a suppressed text-form call: complete blocks
/// are stripped, then any leftover unterminated markup truncates the rest —
/// raw call markup never reaches the stored answer.
fn strip_call_markup(text: &str) -> String {
    let stripped = crate::tool_calls::strip_text_tool_calls(text);
    let end = ["<tool_call", "<function="]
        .iter()
        .filter_map(|tag| stripped.find(tag))
        .min()
        .unwrap_or(stripped.len());
    stripped[..end].trim().to_string()
}

/// Deterministic claim audit shared by the `finish` path and plain-text
/// research completions. `args` carries the answer plus any model-cited
/// evidences. Returns the formatted footer and logs the report; failures
/// surface as no footer — the audit never blocks or rewrites text.
async fn claim_audit_footer(
    state: &AppState,
    args: &Value,
    research_evidence: &[(String, String)],
    run: &crate::agent_run::RunRecord,
    step_id: &str,
    conversation_id: &str,
    seq: &mut u32,
) -> Option<String> {
    let report = crate::arex::verify_finish(state, args, research_evidence).await.ok().flatten()?;
    let claims = report["claims"].as_array().map(|c| c.len()).unwrap_or(0);
    if claims == 0 {
        return None;
    }
    let supported = report["supportedCount"].as_u64().unwrap_or(0);
    let conflicting = report["conflictingCount"].as_u64().unwrap_or(0);
    *seq += 1;
    if let Ok(store) = state.database() {
        let _ = store.append_run_event(&crate::agent_run::RunEvent {
            run_id: run.id.clone(),
            seq: *seq as u64,
            step_id: step_id.to_string(),
            tool_call_id: None,
            event_type: "verification".into(),
            payload: report.clone(),
            created_at: crate::store::now(),
        });
        emit_session_event(&store, conversation_id, Some(&run.id), Some(step_id), None,
            "verification", report, false);
    }
    Some(format!(
        "\n\n_Claim audit: {supported} of {claims} answer claims match the collected evidence ({conflicting} conflicting). Deterministic lexical check, not independent verification._"
    ))
}

/// Shared tail of the AREX `finish` path — deterministic claim audit against
/// the evidence collected this run, then append and emit the answer. The
/// audit annotates the answer but never blocks or rewrites it.
#[allow(clippy::too_many_arguments)]
async fn present_research_answer(
    state: &AppState,
    channel: &Channel<ChatEvent>,
    run: &crate::agent_run::RunRecord,
    step_id: &str,
    conversation_id: &str,
    assistant_id: &str,
    seq: &mut u32,
    answer: &mut String,
    final_answer: &str,
    finish_args: Option<&Value>,
    research_evidence: &[(String, String)],
) -> Result<(), String> {
    let mut display = final_answer.to_string();
    if let Some(args) = finish_args {
        if let Some(footer) = claim_audit_footer(
            state,
            args,
            research_evidence,
            run,
            step_id,
            conversation_id,
            seq,
        ).await {
            display.push_str(&footer);
        }
    }
    if !answer.trim().is_empty() { answer.push_str("\n\n"); }
    answer.push_str(&display);
    *seq += 1;
    channel.send(ChatEvent::new(run, Some(step_id), *seq, "Research answer ready", assistant_id, &display, "", None, None)).map_err(|e| e.to_string())?;
    Ok(())
}

/// Runs a `finish` call that arrived outside the permissioned tool loop — as
/// literal markup on a suppressed-choice round or as a structured call on a
/// finalizing round. Records the same audit trail a dispatched call would,
/// then delivers the answer through the shared finish tail. Returns true
/// when the answer was delivered.
#[allow(clippy::too_many_arguments)]
async fn run_finish_call(
    state: &AppState,
    channel: &Channel<ChatEvent>,
    run: &crate::agent_run::RunRecord,
    step_id: &str,
    conversation_id: &str,
    assistant_id: &str,
    seq: &mut u32,
    answer: &mut String,
    research_evidence: &[(String, String)],
    access_mode: crate::permissions::AccessMode,
    call: &crate::tool_calls::ToolCall,
    round_answer: &str,
) -> Result<bool, String> {
    if crate::arex::validate("finish", &call.arguments).is_err() {
        return Ok(false);
    }
    // Same result shape the harness dispatch produces for finish.
    let result = json!({
        "answer": crate::arex::finish_answer(&call.arguments),
        "evidences": call.arguments["evidences"],
        "confidence": call.arguments["confidence"],
    });
    let audit = json!({"connector":"Harness","localServerName":null,"name":"finish","arguments":call.arguments,"decision":"allowed","accessMode":access_mode,"authorization":"finish is the model's answer channel on a finalizing round","callId":call.id,"stepId":step_id,"assistantContent":round_answer});
    state.database()?.append_message(conversation_id, "tool", &json!({"request":audit,"result":result}).to_string(), "complete")?;
    if let Ok(store) = state.database() {
        emit_session_event(&store, conversation_id, Some(&run.id), Some(step_id), Some(&call.id),
            "tool_call", json!({"callId": call.id, "name": "finish", "arguments": call.arguments}), false);
        emit_session_event(&store, conversation_id, Some(&run.id), Some(step_id), Some(&call.id),
            "tool_result", json!({"callId": call.id, "name": "finish", "decision": "allowed", "result": result, "artifactId": null}), false);
    }
    let mut sink = Vec::new();
    let Some(final_answer) = crate::arex::apply_control("finish", &result, &*state.database()?, conversation_id, &run.id, &mut sink)? else {
        return Ok(false);
    };
    present_research_answer(state, channel, run, step_id, conversation_id, assistant_id, seq, answer, &final_answer, Some(&call.arguments), research_evidence).await?;
    Ok(true)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notice: Option<String>,
    pub message_id: String,
    pub content: String,
    pub reasoning: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elapsed_secs: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub round_index: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_stream: Option<ToolStreamEvent>,
    /// Measured llama-server timings for a finished round (telemetry.rs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timings: Option<Value>,
}

impl ChatEvent {
    pub fn progress(
        run_id: &str,
        step_id: &str,
        message_id: &str,
        activity: &str,
        approval: Option<Value>,
    ) -> Self {
        // Channel-only progress emission (durable log comes from audit rows).
        Self {
            run_id: Some(run_id.to_string()),
            step_id: Some(step_id.to_string()),
            seq: Some(0),
            state: None,
            activity: Some(activity.to_string()),
            context: None,
            notice: None,
            message_id: message_id.to_string(),
            content: String::new(),
            reasoning: String::new(),
            approval,
            elapsed_secs: None,
            round_index: None,
            timings: None,
            tool_stream: None,
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        run: &crate::agent_run::RunRecord,
        step_id: Option<&str>,
        seq: u32,
        activity: &str,
        message_id: &str,
        content: &str,
        reasoning: &str,
        context: Option<Value>,
        approval: Option<Value>,
    ) -> Self {
        Self {
            run_id: Some(run.id.clone()),
            step_id: step_id.map(String::from),
            seq: Some(seq),
            state: Some(run.status.as_str().to_string()),
            activity: Some(activity.to_string()),
            context,
            notice: None,
            message_id: message_id.to_string(),
            content: content.to_string(),
            reasoning: reasoning.to_string(),
            approval,
            elapsed_secs: None,
            round_index: None,
            timings: None,
            tool_stream: None,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn emit_session_event(
    store: &crate::store::Store,
    conversation_id: &str,
    run_id: Option<&str>,
    step_id: Option<&str>,
    tool_call_id: Option<&str>,
    event_type: &str,
    payload: Value,
    ignorable: bool,
) {
    let ev = crate::sessions::SessionEvent {
        id: uuid::Uuid::new_v4().to_string(),
        conversation_id: conversation_id.to_string(),
        run_id: run_id.map(String::from),
        seq: 0,
        step_id: step_id.map(String::from),
        tool_call_id: tool_call_id.map(String::from),
        event_type: event_type.to_string(),
        payload,
        ignorable,
        created_at: crate::store::now(),
    };
    let _ = store.append_session_event(&ev);
}

/// One assembled chat turn: everything the model will see, rendered exactly
/// once so send and preflight counting cannot drift apart.
pub struct TurnPlan {
    pub preset: crate::presets::Preset,
    pub tools: Vec<crate::connectors::AgentTool>,
    /// Frozen model-visible tool schemas (stable bytes for prompt cache).
    pub tool_definitions: Vec<Value>,
    pub tool_notice: Option<String>,
    /// Injections logged as context_injection events; only kinds listed in
    /// `prompt::VOLATILE_KINDS` actually reach the model on the user draft.
    pub injections: Vec<(String, String)>,
    pub skill_instructions: String,
    pub ptc_sdk: Option<String>,
    pub preferences: crate::store::Preferences,
    /// Replayed conversation history without the current draft message.
    pub history: Vec<Value>,
    pub draft_message: Value,
    pub backend: Backend,
    pub selection: crate::providers::ModelSelection,
    pub access_mode: crate::permissions::AccessMode,
    pub context_length: u32,
    /// llama.cpp slot for this conversation (parent = 0). None for remote.
    pub id_slot: Option<i64>,
    /// The AREX research contract was applied to this turn's tool set — the
    /// model answers through its `finish` tool, which must stay reachable on
    /// finalizing rounds.
    pub arex_tools: bool,
}

impl TurnPlan {
    /// AREX contract on the local llama.cpp runtime — the only backend where
    /// tool_choice suppression removes the model's trained answer path and
    /// literal call markup leaks into the answer text.
    fn local_arex(&self) -> bool {
        self.arex_tools && matches!(self.backend, Backend::Local { .. })
    }
}

fn add_tool_notice(notice: &mut Option<String>, next: String) {
    if let Some(existing) = notice {
        existing.push(' ');
        existing.push_str(&next);
    } else {
        *notice = Some(next);
    }
}

fn append_workspace_tools(
    tools: &mut Vec<crate::connectors::AgentTool>,
    path: &str,
    minimal: bool,
) -> Option<String> {
    let workspace = match crate::workspace::Workspace::open(path) {
        Ok(workspace) => workspace,
        Err(_) if path.trim().is_empty() => return Some(
            "Workspace Files was skipped because this conversation has no workspace. Select a project or choose a folder in Tools to use workspace files.".into(),
        ),
        Err(error) => return Some(format!(
            "Workspace Files was skipped because its folder is unavailable: {error}. Choose another project or folder in Tools to use workspace files."
        )),
    };
    let mut workspace_tools = std::sync::Arc::new(workspace).tools();
    if minimal {
        workspace_tools.retain(|tool| crate::presets::minimal_workspace_tools().contains(&tool.tool.name.as_str()));
    }
    tools.extend(workspace_tools);
    None
}

/// Assemble the exact payload components for one turn of `conversation_id`.
/// An empty id means a brand-new conversation: defaults apply and nothing is
/// read from per-conversation rows.
#[allow(clippy::too_many_arguments)]
async fn assemble_turn(
    state: &AppState,
    conversation_id: &str,
    content: &str,
    connector_ids: Vec<String>,
    connector_tools: Option<Vec<crate::connectors::ToolSelection>>,
    new_chat_preset: Option<&str>,
) -> Result<TurnPlan, String> {
    let (exists, preset_id) = {
        let store = state.database()?;
        let exists = !conversation_id.is_empty() && store.conversation_exists(conversation_id)?;
        let preset_id = match new_chat_preset.filter(|_| !exists) {
            Some(id) => id.to_string(),
            None => store.conversation_preset(conversation_id).unwrap_or_else(|_| crate::presets::STANDARD.to_string()),
        };
        (exists, preset_id)
    };
    let mut connector_ids = connector_ids;
    // Runtime preset intersects the requested sources (config patch, no fork).
    let preset = crate::presets::get(&preset_id)?;
    let use_daytona = connector_ids.iter().any(|id| id == "__daytona") && preset.sources.iter().any(|source| source == "__daytona");
    connector_ids.retain(|id| id != "__daytona");
    let use_workspace = connector_ids.iter().any(|id| id == "__workspace") && preset.sources.iter().any(|source| source == "__workspace");
    let use_execution = connector_ids.iter().any(|id| id == "__execution") && preset.sources.iter().any(|source| source == "__execution");
    connector_ids.retain(|id| id != "__execution");
    connector_ids.retain(|id| id != "__workspace");
    if !preset.mcp {
        connector_ids.clear();
    }
    let connector_tools = if preset.mcp { connector_tools.unwrap_or_default() } else { Vec::new() };
    let (mut tools, unavailable_connectors) = if preset.mcp {
        let hub = state.connectors.lock().await;
        hub.tools_for_turn(&connector_ids, &connector_tools)?
    } else {
        (Vec::new(), Vec::new())
    };
    let mut tool_notice = if unavailable_connectors.is_empty() { None } else {
        Some(format!("Unavailable for this reply: {}. Chat can continue. Connect these services in Connectors if your request needs them, or deselect them in Tools.", unavailable_connectors.join(", ")))
    };
    if use_workspace {
        let path = state.database()?.workspace_path()?;
        if let Some(notice) = append_workspace_tools(&mut tools, &path, preset.id == crate::presets::MINIMAL) {
            add_tool_notice(&mut tool_notice, notice);
        }
    }
    if use_daytona {
        let bytes = state
            .daytona_vault
            .load("daytona")?
            .ok_or("Save a Daytona API key in Execution first.")?;
        let key = String::from_utf8(bytes).map_err(|_| "Saved Daytona key is invalid.")?;
        tools.push(crate::connectors::AgentTool::daytona(
            crate::daytona_execution::Executor::new(
                &key,
                state.daytona_journal.clone(),
                state.daytona_operation.clone(),
            )?,
        ));
    }
    if use_execution {
        let (config, path) = {
            let store = state.database()?;
            (store.execution_config()?, store.workspace_path()?)
        };
        match crate::execution::LocalExecution::new(config, &path) {
            Ok(execution) => {
                let exec = std::sync::Arc::new(execution);
                tools.push(exec.clone().tool());
                tools.push(exec.command_tool());
            }
            Err(_) if path.trim().is_empty() => add_tool_notice(
                &mut tool_notice,
                "Local code was skipped because this conversation has no workspace. Select a project or choose a folder in Tools to enable it.".into(),
            ),
            Err(error) => add_tool_notice(
                &mut tool_notice,
                format!("Local code was skipped because it is unavailable: {error}. Choose another project or folder in Tools to enable it."),
            ),
        }
    }
    let active_skills = if preset.skills { state.database()?.active_skills()? } else { Vec::new() };
    let skill_instructions = {
        let skills = state.skills.lock().await;
        let instructions = skills.instructions(&active_skills)?;
        if preset.skills {
            if let Some(reader) = skills.reader(&active_skills)? {
                tools.push(reader);
            }
        }
        instructions
    };
    for alias in &preset.harness {
        if !tools.iter().any(|tool| &tool.alias == alias) {
            tools.push(crate::connectors::AgentTool::harness(alias)?);
        }
    }
    if tools.len() > 64 {
        return Err("Select fewer tool sources: at most 64 tools can be offered in a turn.".into());
    }
    let (selection, selection_required) = if exists {
        state.database()?.conversation_model(conversation_id)?
    } else {
        (state.database()?.preferred_model()?, false)
    };
    if selection_required {
        return Err("This conversation needs another provider selection before it can continue. Its history is preserved.".into());
    }
    let draft_content = crate::attachments::model_content(content.trim())?;
    let supports_images;
    let backend = if let Some(provider_id) = selection.provider_id.clone() {
        let provider = state.database()?.provider(&provider_id)?.ok_or("The selected provider was deleted. Choose another provider before sending a message.")?;
        if !provider.verified {
            return Err("Test the selected provider connection successfully before sending a message.".into());
        }
        let model = provider.models.iter().find(|model| model.id == selection.model_id).ok_or("Configure the selected remote model's context capacity before sending a message.")?;
        supports_images = model.supports_images;
        if crate::providers::is_subscription_format(&provider.api_format) {
            let kind = if provider.api_format == crate::providers::CHATGPT_SUBSCRIPTION || provider.id == "chatgpt-subscription" {
                "chatgpt"
            } else {
                "grok"
            };
            let token = crate::subscription_auth::get_valid_access_token(&state.daytona_vault, kind).await?;
            let account_id = crate::subscription_auth::load_session(&state.daytona_vault, kind)?
                .and_then(|s| s.account_id);
            Backend::subscription(&provider.name, &provider.base_url, token, account_id, model)?
        } else if provider.api_format == crate::providers::OPENAI_CHAT_COMPLETIONS
            || provider.api_format == crate::providers::FREETOKEN_OPENAI_COMPAT
            || provider.api_format == crate::providers::CLAUDE_MESSAGES
        {
            let key = match state.daytona_vault.load(&crate::providers::credential_id(&provider_id))? {
                Some(bytes) => String::from_utf8(bytes).map_err(|_| "Saved provider API key is invalid.")?,
                // Verified loopback-only engines (llama.cpp servers, FreeToken)
                // may run without any authentication; remote hosts never do.
                // Claude Messages always requires an API key.
                None if crate::providers::is_loopback_base_url(&provider.base_url)
                    && provider.api_format != crate::providers::CLAUDE_MESSAGES => String::new(),
                None => return Err("Save an API key for the selected provider before sending a message.".into()),
            };
            Backend::openai(&provider, key, model)?
        } else {
            return Err("This provider format is not supported by the current chat adapter.".into());
        }
    } else {
        let mut runtime = state.runtime.lock().await;
        if runtime.inspect().phase != "ready" {
            return Err("Load a local model before sending a message, or choose a tested API provider.".into());
        }
        supports_images = runtime.supports_images;
        Backend::local(runtime.endpoint.clone(), runtime.api_key.clone(), runtime.context_length)?
    };
    let context_length = backend.context_capacity();
    let access_mode = if exists {
        state.database()?.conversation_tools(conversation_id)?.access_mode
    } else {
        crate::permissions::AccessMode::default()
    };
    // Volatile injections ride on the current user draft so strict templates see
    // one leading system message and the prompt cache keeps the frozen prefix.
    let mut injections: Vec<(String, String)> = Vec::new();
    if let Some(notice) = &tool_notice {
        injections.push(("unavailable_tools".into(), format!("{notice} Do not claim to have used these unavailable services. If you cannot fulfill the request with the available tools, explain what is missing.")));
    }
    let workspace_path = state.database()?.workspace_path().unwrap_or_default();
    if preset.harness.iter().any(|alias| alias == "memory_recall") {
        let scope = if workspace_path.is_empty() { "global".to_string() } else { workspace_path.clone() };
        if let Ok(store) = state.database() {
            if let Ok(facts) = store.recall_facts(&scope, 20) {
                if let Some(block) = crate::memory::recall_block(&facts) {
                    injections.push(("memory".into(), block));
                }
            }
            if let Ok(todos) = store.todos(conversation_id) {
                if let Ok(goal) = store.goal(conversation_id) {
                    let todo_tools: Vec<&str> = ["todo_write", "todo_add", "todo_update"]
                        .into_iter()
                        .filter(|alias| tools.iter().any(|tool| tool.alias.as_str() == *alias))
                        .collect();
                    if let Some(line) = crate::plans::summary_line(&todos, goal.as_deref(), &todo_tools) {
                        injections.push(("plan".into(), line));
                    }
                }
            }
        }
    }
    let (preferences, history) = {
        let store = state.database()?;
        let mut preferences = store.preferences()?.apply_model_defaults();
        preferences.max_tokens =
            crate::context::fit_response_budget(preferences.max_tokens, context_length);
        preferences.validate()?;
        backend.validate_response_tokens(preferences.max_tokens)?;
        let previous = if exists { store.messages(conversation_id)? } else { Vec::new() };
        let checkpoint = store.compaction(conversation_id)?;
        let history = crate::history::model_history_with_cutoff(
            &previous,
            checkpoint.as_ref().map(|checkpoint| (checkpoint.cutoff, checkpoint.artifact_id.as_str())),
        )?;
        if !supports_images && (draft_content.is_array() || history.iter().any(|m| m["role"] == "user" && m["content"].is_array())) {
            return Err("This task contains image attachments. Load a local model with a vision projector, or select a provider model configured with image input support in Models & runtime.".into());
        }
        (preferences, history)
    };
    let mut tool_schemas_suppressed = false;
    if matches!(&backend, Backend::Local { .. })
        && !tools.is_empty()
        && state.database()?.local_tool_calling_supported(&preferences.runtime_path, &preferences.model_path)? == Some(false)
    {
        tools.clear();
        tool_schemas_suppressed = true;
        let message = "Tool calling is disabled for this local runtime because it previously emitted an unsupported text-form tool call. It can still answer from its available knowledge.";
        tool_notice = Some(match tool_notice {
            Some(existing) => format!("{existing} {message}"),
            None => message.into(),
        });
    }
    // The experimental ZAYA chat template can emit tool-call prose instead of
    // structured calls. Supplying the full harness then makes it reason until
    // the response limit with no executable call. Keep local ZAYA turns
    // answer-only until structured tool parsing is verified for this runtime.
    if preferences.model_path.ends_with(crate::model_catalog::ZAYA1_FILENAME) {
        tool_schemas_suppressed = true;
        if !tools.is_empty() {
            tools.clear();
            let message = "ZAYA1 tool calling is disabled for this local runtime; answering with the model's available knowledge.";
            tool_notice = Some(match tool_notice {
                Some(existing) => format!("{existing} {message}"),
                None => message.into(),
            });
        }
    }
    let system_time_enabled = state
        .database()
        .and_then(|store| crate::capabilities::is_enabled(&store, "system_time"))
        .unwrap_or(true);
    let system_time_on = if backend.supports_tools() {
        system_time_enabled
    } else {
        false
    };
    if system_time_enabled {
        // Small models cannot infer "today" and rarely call the clock tool
        // unprompted; ground date-sensitive questions directly. Rides on the
        // user draft, so the frozen cache prefix is untouched.
        injections.push(("system_time".into(), format!(
            "Current local date and time: {}.",
            chrono::Local::now().format("%A, %d %B %Y, %H:%M (%:z)")
        )));
    }
    if backend.supports_tools() {
        if system_time_on && !tool_schemas_suppressed && !tools.iter().any(|tool| tool.alias == "system_time") {
            tools.push(crate::connectors::AgentTool::system_time());
        }
    } else if !tools.is_empty() {
        return Err("The selected remote model is configured without tool-calling support. Deselect tools or choose a model that supports tool calling.".into());
    }
    let arex_tools = crate::arex::adapt_tools(if selection.provider_id.is_some() { &selection.model_id } else { &preferences.model_path }, &mut tools)?;
    if arex_tools {
        injections.push(("arex_research".into(), crate::arex::RESEARCH_GUIDANCE.into()));
    }
    if tools.len() > 64 { return Err("Select fewer tool sources: at most 64 tools can be offered in a turn.".into()); }
    crate::prompt::sort_tools_by_alias(&mut tools);
    let ptc_sdk = if preset.id == crate::presets::CODE {
        let aliases: Vec<String> = tools.iter().map(|tool| tool.alias.clone()).collect();
        Some(crate::presets::ts_sdk(&aliases))
    } else {
        None
    };
    let selection_hash = {
        let tools_value = if exists {
            serde_json::to_value(state.database()?.conversation_tools(conversation_id)?).unwrap_or(Value::Null)
        } else {
            json!({"sources": connector_ids, "tools": connector_tools})
        };
        let skills = if exists { state.database()?.active_skills().unwrap_or_default() } else { Vec::new() };
        crate::prompt::selection_hash(&preset.id, &tools_value, &skills, system_time_on, tool_schemas_suppressed)
    };
    let selection_hash = format!("{selection_hash}:{}", tools.iter().map(|t| t.alias.as_str()).collect::<Vec<_>>().join(","));
    let tool_definitions = freeze_tool_catalog(
        &*state.database()?,
        conversation_id,
        exists,
        selection_hash,
        &mut tools,
    )?;
    let id_slot = match &backend {
        Backend::Local { .. } => Some(0),
        _ => None,
    };
    Ok(TurnPlan {
        preset,
        tools,
        tool_definitions,
        tool_notice,
        injections,
        skill_instructions,
        ptc_sdk,
        preferences,
        history,
        draft_message: json!({"role": "user", "content": draft_content}),
        backend,
        selection,
        access_mode,
        context_length,
        id_slot,
        arex_tools,
    })
}

fn freeze_tool_catalog(
    store: &crate::store::Store,
    conversation_id: &str,
    exists: bool,
    hash: String,
    tools: &mut Vec<crate::connectors::AgentTool>,
) -> Result<Vec<Value>, String> {
    crate::prompt::sort_tools_by_alias(tools);
    let aliases: Vec<String> = tools.iter().map(|tool| tool.alias.clone()).collect();
    let defs: Vec<Value> = tools.iter().map(|tool| tool.definition()).collect();
    if !exists || conversation_id.is_empty() {
        return Ok(defs);
    }
    if let Some(freeze) = store.prompt_freeze(conversation_id)? {
        if freeze.selection_hash == hash {
            let frozen_defs: Vec<Value> =
                serde_json::from_str(&freeze.tools_json).unwrap_or_else(|_| defs.clone());
            let frozen_aliases: Vec<String> =
                serde_json::from_str(&freeze.aliases_json).unwrap_or_else(|_| aliases.clone());
            let mut live: std::collections::HashMap<String, crate::connectors::AgentTool> =
                std::mem::take(tools).into_iter().map(|tool| (tool.alias.clone(), tool)).collect();
            let mut ordered = Vec::new();
            for alias in &frozen_aliases {
                if let Some(tool) = live.remove(alias) {
                    ordered.push(tool);
                }
            }
            ordered.extend(live.into_values());
            *tools = ordered;
            return Ok(frozen_defs);
        }
    }
    store.save_prompt_freeze(&crate::store::PromptFreeze {
        conversation_id: conversation_id.into(),
        selection_hash: hash,
        tools_json: serde_json::to_string(&defs).map_err(|error| error.to_string())?,
        aliases_json: serde_json::to_string(&aliases).map_err(|error| error.to_string())?,
        created_at: crate::store::now(),
    })?;
    Ok(defs)
}

fn frozen_system_for(plan: &TurnPlan) -> String {
    if plan.tools.iter().any(|tool| tool.alias == "local_run_code") {
        crate::prompt::frozen_system_with_guidance(
            &plan.preferences.system_prompt,
            &plan.skill_instructions,
            plan.ptc_sdk.as_deref(),
            Some(crate::prompt::CODE_EXECUTION_GUIDANCE),
        )
    } else {
        crate::prompt::frozen_system(
            &plan.preferences.system_prompt,
            &plan.skill_instructions,
            plan.ptc_sdk.as_deref(),
        )
    }
}

/// Render the model-visible message list. One leading frozen system, then
/// history, then the draft with any volatile prefix. Tool schemas live on the plan.
fn build_turn_messages(plan: &TurnPlan) -> Vec<Value> {
    let frozen = frozen_system_for(plan);
    crate::prompt::build_turn_messages(
        &frozen,
        &plan.history,
        crate::prompt::volatile_scratchpad(&plan.injections).as_deref(),
        &plan.draft_message,
    )
}

#[tauri::command]
pub fn cancel_generation(state: State<'_, AppState>) {
    state.cancel.send_replace(true);
}

/// Count before every generation round. Compaction never runs inside a tool call.
async fn prepare_context(
    state: &AppState, plan: &TurnPlan, conversation_id: &str,
    messages: &mut Vec<Value>, denied: bool, finalizing: bool,
) -> Result<(u64, Option<String>), String> {
    let payload = plan.backend.payload(&request_payload(messages, &plan.tool_definitions, &plan.preferences, denied, finalizing, false, plan.id_slot, plan.local_arex()));
    let mut tokens = plan.backend.count_tokens(&payload).await?;
    let mut artifact = None;
    let auto = {
        let store = state.database()?;
        crate::compaction::capability_enabled(&store)
            && crate::compaction::auto_enabled(&store, conversation_id)
    };
    if auto && crate::compaction::should_compact(tokens, plan.preferences.max_tokens, plan.context_length) {
        artifact = crate::compaction::compact_model_messages(&*state.database()?, conversation_id, messages)?;
        if artifact.is_some() {
            let payload = plan.backend.payload(&request_payload(messages, &plan.tool_definitions, &plan.preferences, denied, finalizing, false, plan.id_slot, plan.local_arex()));
            tokens = plan.backend.count_tokens(&payload).await?;
        }
    }
    crate::context::validate_budget(tokens, plan.preferences.max_tokens, plan.context_length)?;
    Ok((tokens, artifact))
}

#[tauri::command]
pub async fn send_message(
    state: State<'_, AppState>,
    conversation_id: String,
    content: String,
    connector_ids: Option<Vec<String>>,
    connector_tools: Option<Vec<crate::connectors::ToolSelection>>,
    plan_mode: Option<bool>,
    channel: Channel<ChatEvent>,
) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Another model operation is in progress.")?;
    if content.trim().is_empty() || content.len() > 1_048_576 {
        return Err("Enter a message no larger than 1 MiB.".into());
    }
    {
        let store = state.database()?;
        if let Some(path) = crate::workspace_ui::task_workspace(&store, &conversation_id)? {
            crate::workspace::Workspace::open(&path)?;
            store.save_workspace_path(&path)?;
        }
    }
    let mut plan = assemble_turn(
        &state,
        &conversation_id,
        &content,
        connector_ids.unwrap_or_default(),
        connector_tools,
        None,
    )
    .await?;
    let plan_mode = plan_mode.unwrap_or(false);
    // Rides on the draft as a volatile injection so the prompt-cache prefix
    // and the logged injections stay identical to every other turn.
    inject_plan_mode(&mut plan, plan_mode);
    if let Some(notice) = &plan.tool_notice {
        let mut event = ChatEvent::progress("", "", "", "Preparing request…", None);
        event.notice = Some(notice.clone());
        let _ = channel.send(event);
    }
    state.cancel.send_replace(false);
    let mut cancellation = state.cancel.subscribe();
    let mut messages = build_turn_messages(&plan);
    let (mut input_tokens, compacted_once) = tokio::select! {
        _ = cancellation.changed() => return Err("Message cancelled before generation; it was not saved.".into()),
        result = prepare_context(&state, &plan, &conversation_id, &mut messages, false, false) => result?,
    };
    let assistant = state.database()?.begin_turn(&conversation_id, &content)?;
    let run_id = format!("run-{}", uuid::Uuid::new_v4());
    let now = crate::store::now();
    let mut run = crate::agent_run::RunRecord {
        id: run_id.clone(),
        conversation_id: conversation_id.clone(),
        status: crate::agent_run::RunState::Preparing,
        model_provider: plan.selection.provider_id.clone(),
        model_id: Some(plan.selection.model_id.clone()),
        checkpoint: None,
        error: None,
        created_at: now,
        updated_at: now,
    };
    state.database()?.save_run(&run)?;
    if let Ok(store) = state.database() {
        emit_session_event(
            &store,
            &conversation_id,
            Some(&run.id),
            Some("turn_start"),
            None,
            "user_msg",
            json!({"role": "user", "content": content.trim()}),
            false,
        );
        emit_session_event(
            &store,
            &conversation_id,
            Some(&run.id),
            Some("turn_start"),
            None,
            "system_prompt",
            json!({"role": "system", "content": plan.preferences.system_prompt}),
            false,
        );
        for (kind, text) in &plan.injections {
            emit_session_event(
                &store,
                &conversation_id,
                Some(&run.id),
                Some("turn_start"),
                None,
                "context_injection",
                json!({"kind": kind, "chars": text.len(), "content": text}),
                false,
            );
        }
        if let Some(checkpoint) = &compacted_once {
            emit_session_event(
                &store,
                &conversation_id,
                Some(&run.id),
                Some("turn_start"),
                None,
                "compaction",
                json!({"artifact": checkpoint, "thresholdPercent": 80, "auto": true}),
                false,
            );
        }
        emit_session_event(
            &store,
            &conversation_id,
            Some(&run.id),
            Some("turn_start"),
            None,
            "turn_start",
            json!({
                "runId": run.id,
                "modelProvider": plan.selection.provider_id,
                "modelId": plan.selection.model_id,
                "contextLength": plan.context_length,
                "inputTokens": input_tokens,
            }),
            false,
        );
    }
    let mut seq: u32 = 0;
    // Model-visible ⟺ logged: every injected context block lands in the log.
    for (kind, text) in &plan.injections {
        seq += 1;
        let _ = state.database()?.append_run_event(&crate::agent_run::RunEvent {
            run_id: run.id.clone(),
            seq: seq as u64,
            step_id: "turn_start".into(),
            tool_call_id: None,
            event_type: "context_injection".into(),
            payload: json!({"kind": kind, "chars": text.len()}),
            created_at: crate::store::now(),
        });
    }
    if let Some(checkpoint) = &compacted_once {
        seq += 1;
        let _ = state.database()?.append_run_event(&crate::agent_run::RunEvent {
            run_id: run.id.clone(),
            seq: seq as u64,
            step_id: "turn_start".into(),
            tool_call_id: None,
            event_type: "compaction".into(),
            payload: json!({"artifact": checkpoint, "thresholdPercent": 80, "auto": true}),
            created_at: crate::store::now(),
        });
    }
    // Snapshot once for harness children (subagents, workflows).
    let snapshot = crate::subagents::snapshot_for_conversation(&state, &conversation_id).await?;
    let preset_id = plan.preset.id.clone();
    let mut answer = String::new();
    let mut reasoning = String::new();
    let mut research_fallback: Option<String> = None;
    let mut finalization_retried = false;
    // Reload this conversation's saved KV state if another conversation has
    // used the slot since (kv_slots.rs). Logged; never model-visible.
    if let Some(payload) = crate::kv_slots::restore_for_turn(&state, &plan.backend, &conversation_id).await {
        if let Ok(store) = state.database() {
            emit_session_event(&store, &conversation_id, Some(&run.id), None, None, "kv_slot", payload, true);
        }
    }
    // Snapshot the workspace so this turn's file changes can be reviewed and
    // reverted (checkpoints.rs). A skipped snapshot is logged with its reason.
    let (checkpoint_id, checkpoint_event) = crate::checkpoints::begin_turn(&state, &plan.tools, &conversation_id, &run.id).await;
    if let (Some(payload), Ok(store)) = (checkpoint_event, state.database()) {
        emit_session_event(&store, &conversation_id, Some(&run.id), None, None, "checkpoint", payload, true);
    }
    let result: Result<bool,String> = async {
        seq += 1;
        channel.send(ChatEvent::new(
            &run,
            None,
            seq,
            "Preparing turn",
            &assistant.id,
            "",
            "",
            None,
            None,
        )).map_err(|error| error.to_string())?;

        let mut tool_use_denied = false;
        // Loop-hygiene streaks: consecutive identical calls, guarded centrally.
        let mut recent_calls: Vec<(String, String)> = Vec::new();
        let mut research_progress = ResearchProgress::default();
        let mut research_evidence: Vec<(String, String)> = Vec::new();
        for round in 0..64 {
            if *cancellation.borrow() { return Ok(false); }
            let step_id = format!("step-{round}");
            // Plan mode spends round 0 on a tool-free planning pass; the
            // ordinary tool loop then implements the plan from round 1 on.
            let planning = plan_mode && round == 0;
            if plan_mode && round > 0 {
                if let Ok(todos) = state.database()?.todos(&conversation_id) {
                    refresh_plan_focus(&mut messages, &todos);
                }
            }
            let finalizing = research_fallback.is_some() || finalization_retried;
            let _ = run.transition_to(crate::agent_run::RunState::Generating);
            let _ = state.database()?.update_run_status(&run.id, run.status, None, Some(&step_id));
            seq += 1;
            let _ = state.database()?.append_run_event(&crate::agent_run::RunEvent {
                run_id: run.id.clone(),
                seq: seq as u64,
                step_id: step_id.clone(),
                tool_call_id: None,
                event_type: "step_start".into(),
                payload: json!({"round": round}),
                created_at: crate::store::now(),
            });
            if let Ok(store) = state.database() {
                emit_session_event(
                    &store,
                    &conversation_id,
                    Some(&run.id),
                    Some(&step_id),
                    None,
                    "step_start",
                    json!({"round": round}),
                    false,
                );
            }

            if round > 0 {
                let (tokens, compacted) = tokio::select! {
                    _ = cancellation.changed() => return Ok(false),
                    result = prepare_context(&state, &plan, &conversation_id, &mut messages, tool_use_denied, finalizing) => result?,
                };
                input_tokens = tokens;
                if let Some(artifact) = compacted {
                    let store = state.database()?;
                    emit_session_event(&store, &conversation_id, Some(&run.id), Some(&step_id), None,
                        "compaction", json!({"artifact":artifact,"auto":true,"thresholdPercent":80,"inputTokens":tokens}), false);
                    let mut event = ChatEvent::progress(&run.id, &step_id, &assistant.id, "Context compacted · continuing…", None);
                    event.notice = Some("Context reached 80%. Older activity was archived; continuing this response.".into());
                    channel.send(event).map_err(|e| e.to_string())?;
                }
            } else if compacted_once.is_some() {
                let mut event = ChatEvent::progress(&run.id, &step_id, &assistant.id, "Context compacted · continuing…", None);
                event.notice = Some("Context reached 80%. Older activity was archived; continuing this response.".into());
                let _ = channel.send(event);
            }
            let payload = plan.backend.payload(&request_payload(&messages, &plan.tool_definitions, &plan.preferences, tool_use_denied, finalizing, planning, plan.id_slot, plan.local_arex()));
            seq += 1;
            let _ = channel.send(ChatEvent::new(
                &run,
                Some(&step_id),
                seq,
                if planning { "Creating plan…" } else { "Generating response…" },
                &assistant.id,
                "",
                "",
                Some(json!({"inputTokens":input_tokens,"responseReserve":plan.preferences.max_tokens,"contextLength":plan.context_length,"estimated":plan.backend.context_is_estimate()})),
                None,
            ));

            let response = tokio::select! {
                _ = cancellation.changed() => return Ok(false),
                result = tokio::time::timeout(Duration::from_secs(600), plan.backend.stream(&payload)) => result.map_err(|_| "Model did not respond within ten minutes.")??,
            };
            let mut stream = response.bytes_stream();
            let mut decoder = SseDecoder::default();
            let mut checkpoint = Instant::now();
            let mut calls = crate::tool_calls::ToolCalls::default();
            let mut round_answer = String::new();
            let mut round_reasoning = String::new();
            let mut finish_reason = String::new();
            let mut round_timings: Option<crate::telemetry::RoundTimings> = None;
            let mut think_filter = ThinkFilter::new();
            // A call attempt on a no-tool round is not a visible answer — but
            // its markup is stripped from the answer text all the same.
            let mut suppressed_call = false;
            let round_start_time = Instant::now();
            'stream: loop {
                let next = tokio::select! {
                    _ = cancellation.changed() => return Ok(false),
                    next = tokio::time::timeout(Duration::from_secs(300), stream.next()) => next.map_err(|_| "Model stopped responding.")?,
                };
                let Some(bytes) = next else { decoder.finish()?; return Err("Model stream ended without a completion marker.".into()); };
                let is_claude = matches!(plan.backend, crate::inference::Backend::Claude { .. });
                for event in decoder.push(&bytes.map_err(|error| error.to_string())?)? {
                    // Native Claude Messages events normalize into the same
                    // internal delta contract as OpenAI-style streams.
                    let value: Value = if is_claude {
                        if let Some(seed) = crate::claude_adapter::tool_use_seed(&event) {
                            calls.push(&seed)?;
                        }
                        match crate::claude_adapter::normalize_sse_event(&event)? {
                            Some(normalized) => normalized,
                            None => continue,
                        }
                    } else {
                        if event == "[DONE]" { break 'stream; }
                        serde_json::from_str(&event).map_err(|error| format!("Invalid model stream: {error}"))?
                    };
                    if let Some(error) = value.get("error") { return Err(plan.backend.stream_error(error)); }
                    if let Some(reason) = value["choices"][0]["finish_reason"].as_str() { finish_reason = reason.to_string(); }
                    if let Some(timings) = crate::telemetry::RoundTimings::from_chunk(&value) { round_timings = Some(timings); }
                    let delta = &value["choices"][0]["delta"];
                    calls.push(delta)?;
                    let raw_content = delta["content"].as_str().unwrap_or("");
                    let api_thought = delta["reasoning_content"].as_str().or_else(|| delta["reasoning"].as_str()).unwrap_or("");

                    let (ans_chunk, filter_thought) = think_filter.feed(raw_content);
                    let mut thought_chunk = filter_thought;
                    if !api_thought.is_empty() {
                        thought_chunk.push_str(api_thought);
                    }

                    answer.push_str(&ans_chunk); round_answer.push_str(&ans_chunk);
                    reasoning.push_str(&thought_chunk); round_reasoning.push_str(&thought_chunk);
                    if answer.len() + reasoning.len() > 4_194_304 { return Err("Model output exceeded 4 MiB.".into()); }
                    if !ans_chunk.is_empty() || !thought_chunk.is_empty() {
                        seq += 1;
                        let elapsed_secs = round_start_time.elapsed().as_secs_f64();
                        let mut evt = ChatEvent::new(
                            &run,
                            Some(&step_id),
                            seq,
                            if think_filter.is_thinking() || !thought_chunk.is_empty() { "Thinking…" } else if planning { "Drafting plan…" } else { "Streaming response…" },
                            &assistant.id,
                            &ans_chunk,
                            &thought_chunk,
                            None,
                            None,
                        );
                        evt.elapsed_secs = Some(elapsed_secs);
                        evt.round_index = Some(round);
                        channel.send(evt).map_err(|error| error.to_string())?;
                    }
                    if checkpoint.elapsed() > Duration::from_millis(500) {
                        state.database()?.update_message(&assistant.id,&answer,&reasoning,"streaming")?;
                        checkpoint = Instant::now();
                    }
                }
            }
            let (flush_ans, flush_th) = think_filter.flush();
            if !flush_ans.is_empty() || !flush_th.is_empty() {
                answer.push_str(&flush_ans); round_answer.push_str(&flush_ans);
                reasoning.push_str(&flush_th); round_reasoning.push_str(&flush_th);
                seq += 1;
                let mut evt = ChatEvent::new(
                    &run,
                    Some(&step_id),
                    seq,
                    "Streaming response…",
                    &assistant.id,
                    &flush_ans,
                    &flush_th,
                    None,
                    None,
                );
                evt.elapsed_secs = Some(round_start_time.elapsed().as_secs_f64());
                evt.round_index = Some(round);
                let _ = channel.send(evt);
            }
            // Measured runtime timings only; providers that report none leave
            // no event, and the UI shows the readout as unavailable.
            if let Some(timings) = &round_timings {
                let payload = timings.event(round, plan.id_slot);
                if let Ok(store) = state.database() {
                    emit_session_event(&store, &conversation_id, Some(&run.id), Some(&step_id), None, "timings", payload.clone(), true);
                }
                let mut evt = ChatEvent::progress(&run.id, &step_id, &assistant.id, "", None);
                evt.activity = None;
                evt.round_index = Some(round);
                evt.timings = Some(payload);
                let _ = channel.send(evt);
            }
            if planning {
                // No tools were offered in this round, so its answer is the
                // plan. Park it in the transcript, then let the ordinary tool
                // loop run Phase 2 from the next round on.
                check_finish_reason(&finish_reason)?;
                begin_plan_implementation(&mut messages, &round_answer)?;
                let plan_text = round_answer.trim();
                if let Ok(store) = state.database() {
                    let existing = store.todos(&conversation_id).unwrap_or_default();
                    if existing.is_empty() {
                        let seeded = crate::plans::parse_plan_steps(plan_text);
                        if !seeded.is_empty() {
                            let _ = store.save_todos(&conversation_id, &seeded);
                        }
                    }
                    if let Ok(todos) = store.todos(&conversation_id) {
                        refresh_plan_focus(&mut messages, &todos);
                    }
                }
                answer.push_str(PLAN_SEPARATOR);
                state.database()?.update_message(&assistant.id, &answer, &reasoning, "streaming")?;
                if let Ok(store) = state.database() {
                    emit_session_event(
                        &store,
                        &conversation_id,
                        Some(&run.id),
                        Some(&step_id),
                        None,
                        "plan_created",
                        json!({"round": round, "chars": plan_text.len(), "content": plan_text}),
                        false,
                    );
                    emit_session_event(
                        &store,
                        &conversation_id,
                        Some(&run.id),
                        Some(&step_id),
                        None,
                        "step_end",
                        json!({"round": round, "phase": "plan", "toolCallsCount": 0}),
                        false,
                    );
                }
                let _ = run.transition_to(crate::agent_run::RunState::PreparingNextRound);
                state.database()?.update_run_status(&run.id, run.status, None, Some(&step_id))?;
                seq += 1;
                channel.send(ChatEvent::new(
                    &run,
                    Some(&step_id),
                    seq,
                    "Plan ready · implementing…",
                    &assistant.id,
                    PLAN_SEPARATOR,
                    "",
                    None,
                    None,
                )).map_err(|error| error.to_string())?;
                continue;
            }
            if !finalizing && calls.is_empty() && is_text_tool_call(&round_answer) {
                match crate::tool_calls::parse_text_tool_calls(&round_answer) {
                    // Some templates (AREX-family) emit literal tool-call text;
                    // execute it through the normal permissioned dispatch.
                    Some(parsed) => {
                        calls = crate::tool_calls::ToolCalls::from_parsed(parsed);
                        let stripped = crate::tool_calls::strip_text_tool_calls(&round_answer);
                        if answer.ends_with(&round_answer) {
                            answer.truncate(answer.len() - round_answer.len());
                            answer.push_str(&stripped);
                        }
                        round_answer = stripped;
                    }
                    None => {
                        if matches!(&plan.backend, Backend::Local { .. }) {
                            state.database()?.record_local_tool_calling_support(
                                &plan.preferences.runtime_path,
                                &plan.preferences.model_path,
                                false,
                            )?;
                        }
                        return Err("The model emitted a text-form tool call that this runtime cannot execute. Tool use is unavailable for this model response; retry without tools or use a runtime with structured tool-call support.".into());
                    }
                }
            }
            // A thinking-only token limit may recover with a tool-free answer pass.
            // Other runtime failures (including content filtering) must still fail.
            if finish_reason != "length" { check_finish_reason(&finish_reason)?; }
            if finalizing {
                // AREX answers through its finish tool; the finalizing payload
                // keeps that channel open for local AREX, but a call can still
                // arrive as literal markup or a structured delta. A finish call
                // runs through the same control path as the tool loop; any
                // other call is suppressed — stripped from the answer and still
                // judged "no visible answer" for the retry/fallback decision.
                let has_markup = is_text_tool_call(&round_answer)
                    || round_answer.contains("<tool_call")
                    || round_answer.contains("<function=");
                if has_markup {
                    let stripped = strip_call_markup(&round_answer);
                    if answer.ends_with(&round_answer) {
                        answer.truncate(answer.len() - round_answer.len());
                        answer.push_str(&stripped);
                    }
                    if let Some(parsed) = crate::tool_calls::parse_text_tool_calls(&round_answer) {
                        if let Some(call) = parsed.iter().find(|call| call.name == "finish") {
                            if run_finish_call(&state, &channel, &run, &step_id, &conversation_id, &assistant.id, &mut seq, &mut answer, &research_evidence, plan.access_mode, call, &round_answer).await? {
                                return Ok(true);
                            }
                        }
                    }
                    round_answer = stripped;
                    suppressed_call = true;
                }
                if !calls.is_empty() {
                    suppressed_call = true;
                    let finished = std::mem::take(&mut calls).finish().unwrap_or_default();
                    if let Some(call) = finished.iter().find(|call| call.name == "finish") {
                        if run_finish_call(&state, &channel, &run, &step_id, &conversation_id, &assistant.id, &mut seq, &mut answer, &research_evidence, plan.access_mode, call, &round_answer).await? {
                            return Ok(true);
                        }
                    }
                }
            }
            if needs_final_answer(finalizing, !calls.is_empty(), &round_answer) {
                // On a finalizing round a suppressed call attempt still counts
                // as no visible answer; retry once, then use the grounded result.
                let final_answer = if calls.is_empty() && !suppressed_call { &round_answer } else { "" };
                match finalization_action(final_answer, research_fallback.as_deref(), finalization_retried, research_progress.stopped()) {
                    FinalizationAction::Complete => {}
                    FinalizationAction::Retry => {
                        finalization_retried = true;
                        // Never replay private reasoning or incomplete tool calls.
                        // The existing transcript already contains the tool evidence.
                        if !round_answer.trim().is_empty() && !is_text_tool_call(&round_answer) {
                            messages.push(json!({"role":"assistant","content":round_answer}));
                        }
                        messages.push(json!({"role":"user","content":if research_progress.stopped() { RESEARCH_STALLED } else { FINAL_ANSWER_INSTRUCTION }}));
                        if let Ok(store) = state.database() {
                            emit_session_event(
                                &store,
                                &conversation_id,
                                Some(&run.id),
                                Some(&step_id),
                                None,
                                "finalization_retry",
                                json!({"round": round, "reason": "no_visible_answer", "finishReason": finish_reason, "reasoningLength": round_reasoning.len()}),
                                false,
                            );
                        }
                        let _ = run.transition_to(crate::agent_run::RunState::PreparingNextRound);
                        state.database()?.update_run_status(&run.id, run.status, None, Some(&step_id))?;
                        continue;
                    }
                    FinalizationAction::Fallback(fallback) => {
                        answer = strip_call_markup(&answer);
                        if !answer.trim().is_empty() {
                            answer.push_str("\n\n");
                        }
                        answer.push_str(&fallback);
                        seq += 1;
                        channel.send(ChatEvent::new(
                            &run,
                            Some(&step_id),
                            seq,
                            "Using completed research answer…",
                            &assistant.id,
                            &fallback,
                            "",
                            None,
                            None,
                        )).map_err(|error| error.to_string())?;
                        if let Ok(store) = state.database() {
                            emit_session_event(
                                &store,
                                &conversation_id,
                                Some(&run.id),
                                Some(&step_id),
                                None,
                                "research_answer_fallback",
                                json!({"round": round, "answerLength": fallback.len()}),
                                false,
                            );
                        }
                        return Ok(true);
                    }
                    FinalizationAction::Error => return Err(EMPTY_FINAL_ANSWER_ERROR.into()),
                }
            }
            // A token-limited text response can continue; incomplete tool arguments cannot.
            let continue_text = finish_reason == "length" && calls.is_empty() && !round_answer.is_empty()
                && {
                    let store = state.database()?;
                    crate::compaction::capability_enabled(&store)
                        && crate::compaction::auto_enabled(&store, &conversation_id)
                };
            if !continue_text { check_finish_reason(&finish_reason)?; }
            let calls = calls.finish()?;
            crate::arex::validate_batch(&calls)?;
            if !calls.is_empty() && matches!(&plan.backend, Backend::Local { .. }) {
                state.database()?.record_local_tool_calling_support(
                    &plan.preferences.runtime_path,
                    &plan.preferences.model_path,
                    true,
                )?;
            }
            if let Ok(store) = state.database() {
                if !round_reasoning.is_empty() {
                    emit_session_event(
                        &store,
                        &conversation_id,
                        Some(&run.id),
                        Some(&step_id),
                        None,
                        "reasoning",
                        json!({"text": round_reasoning}),
                        true,
                    );
                }
            }
            {
                let store = state.database()?;
                emit_session_event(&store, &conversation_id, Some(&run.id), Some(&step_id), None,
                    "model_response", json!({"content":round_answer,"final":calls.is_empty() && !continue_text}), false);
            }
            if continue_text {
                messages.push(json!({"role":"assistant","content":round_answer}));
                if !messages.iter().any(|m| m["content"] == crate::prompt::CONTINUE_INSTRUCTION) {
                    messages.push(json!({"role":"user","content":crate::prompt::CONTINUE_INSTRUCTION}));
                }
                let store = state.database()?;
                emit_session_event(&store, &conversation_id, Some(&run.id), Some(&step_id), None,
                    "step_end", json!({"continuing":true,"reason":"response_token_limit"}), false);
                let _ = run.transition_to(crate::agent_run::RunState::PreparingNextRound);
                store.update_run_status(&run.id, run.status, None, Some(&step_id))?;
                continue;
            }
            if calls.is_empty() {
                if round_answer.trim().is_empty() {
                    return Err(EMPTY_FINAL_ANSWER_ERROR.into());
                }
                // Plain-text completions skip `finish`, so the claim audit
                // would never run — apply it here when the turn did research.
                if plan.local_arex() && !research_evidence.is_empty() {
                    let audit_args = json!({"answer": answer, "evidences": []});
                    if let Some(footer) = claim_audit_footer(
                        &state,
                        &audit_args,
                        &research_evidence,
                        &run,
                        &step_id,
                        &conversation_id,
                        &mut seq,
                    ).await {
                        answer.push_str(&footer);
                        seq += 1;
                        channel.send(ChatEvent::new(
                            &run,
                            Some(&step_id),
                            seq,
                            "Claim audit",
                            &assistant.id,
                            &footer,
                            "",
                            None,
                            None,
                        )).map_err(|error| error.to_string())?;
                    }
                }
                let _ = run.transition_to(crate::agent_run::RunState::Completed);
                let _ = state.database()?.update_run_status(&run.id, run.status, None, Some(&step_id));
                if let Ok(store) = state.database() {
                    emit_session_event(
                        &store,
                        &conversation_id,
                        Some(&run.id),
                        Some(&step_id),
                        None,
                        "step_end",
                        json!({"round": round, "content": round_answer, "toolCallsCount": 0}),
                        false,
                    );
                }
                return Ok(true);
            }
            if tool_use_denied { return Err("Tool use stopped after your denial. Send a new message to authorize further actions.".into()); }
            if round >= 63 && !calls.iter().any(|c| c.name == "finish") { return Err("Tool round limit reached. Review the results before continuing.".into()); }
            messages.push(json!({"role":"assistant","content":round_answer,"tool_calls":calls.iter().map(|call| call.model_value()).collect::<Vec<_>>() }));
            if let Ok(store) = state.database() {
                for call in &calls {
                    emit_session_event(
                        &store,
                        &conversation_id,
                        Some(&run.id),
                        Some(&step_id),
                        Some(&call.id),
                        "tool_call",
                        json!({
                            "callId": call.id,
                            "name": call.name,
                            "arguments": call.arguments,
                        }),
                        false,
                    );
                }
            }
            let tool_calls_count = calls.len();
            let mut explicit_finish = None;
            let mut finish_args: Option<Value> = None;
            let research_calls_before = research_progress.calls;
            for call in calls {
                let Some(tool) = plan.tools.iter().find(|tool| tool.alias == call.name) else {
                    let missing = json!({"isError":true,"message":format!("Tool '{}' is not available in this turn. Do not retry it; use another offered tool or explain the gap.", call.name)});
                    messages.push(json!({"role":"tool","tool_call_id":call.id,"content":missing.to_string()}));
                    continue;
                };
                if *cancellation.borrow() { return Ok(false); }
                let automatic_reason = plan.access_mode.automatic_reason(tool.trusted_read()).filter(|_| !tool.always_asks());
                let blocked_by_denial = tool_use_denied;
                let allow = if blocked_by_denial { false } else if automatic_reason.is_some() { true } else {
                    let _ = run.transition_to(crate::agent_run::RunState::AwaitingApproval);
                    let _ = state.database()?.update_run_status(&run.id, run.status, None, Some(&step_id));
                    let (approval_id, decision) = state.approvals.request()?;
                    seq += 1;
                    let sent = channel.send(ChatEvent::new(
                        &run,
                        Some(&step_id),
                        seq,
                        &format!("Awaiting approval: {}", tool.tool.name),
                        &assistant.id,
                        "",
                        "",
                        None,
                        Some(json!({"id":approval_id,"connector":tool.connector,"localServerName":tool.local_server_name(),"name":tool.tool.name,"arguments":call.arguments})),
                    ));
                    if sent.is_err() { state.approvals.remove(&approval_id); return Err("The approval interface disconnected.".into()); }
                    let allow = tokio::select! {
                        _ = cancellation.changed() => None,
                        result = tokio::time::timeout(Duration::from_secs(600),decision) => Some(result.ok().and_then(Result::ok).unwrap_or(false)),
                    };
                    state.approvals.remove(&approval_id);
                    seq += 1;
                    channel.send(ChatEvent::new(
                        &run,
                        Some(&step_id),
                        seq,
                        "Approval decision recorded",
                        &assistant.id,
                        "",
                        "",
                        None,
                        Some(Value::Null),
                    )).map_err(|error| error.to_string())?;
                    let Some(allow) = allow else { return Ok(false); };
                    allow
                };
                if !allow { tool_use_denied = true; }
                let authorization = if blocked_by_denial { "blocked by an earlier denial in this turn" } else { automatic_reason.unwrap_or("user approval decision") };
                let audit = json!({"connector":tool.connector,"localServerName":tool.local_server_name(),"name":tool.tool.name,"arguments":call.arguments,"decision":if allow { "allowed" } else { "denied" },"accessMode":plan.access_mode,"authorization":authorization,"callId":call.id,"stepId":step_id,"assistantContent":round_answer});
                let row = state.database()?.append_message(&conversation_id,"tool",&audit.to_string(),if allow { "streaming" } else { "complete" })?;

                let _ = run.transition_to(crate::agent_run::RunState::ExecutingTools);
                let _ = state.database()?.update_run_status(&run.id, run.status, None, Some(&step_id));
                seq += 1;
                channel.send(ChatEvent::new(
                    &run,
                    Some(&step_id),
                    seq,
                    &format!("Executing {}", tool.tool.name),
                    &assistant.id,
                    "",
                    "",
                    None,
                    None,
                )).map_err(|error| error.to_string())?;

                let tool_exec_start = Instant::now();
                if allow {
                    let cmd_arg = if tool.tool.name == "execute_command" {
                        call.arguments.get("command").and_then(|v| v.as_str()).map(String::from)
                    } else {
                        call.arguments.get("code").and_then(|v| v.as_str()).map(String::from)
                    };
                    let lang_arg = call.arguments.get("language").and_then(|v| v.as_str()).map(String::from);
                    let cwd_arg = call.arguments.get("cwd").and_then(|v| v.as_str()).map(String::from);
                    seq += 1;
                    let _ = channel.send(ChatEvent {
                        run_id: Some(run.id.clone()),
                        step_id: Some(step_id.clone()),
                        seq: Some(seq),
                        state: Some(run.status.as_str().to_string()),
                        activity: Some(format!("Running {}", tool.tool.name)),
                        context: None,
                        notice: None,
                        message_id: assistant.id.clone(),
                        content: String::new(),
                        reasoning: String::new(),
                        approval: None,
                        elapsed_secs: Some(0.0),
                        round_index: Some(round),
                        timings: None,
                        tool_stream: Some(ToolStreamEvent::Started {
                            tool_call_id: call.id.clone(),
                            tool_name: tool.tool.name.clone(),
                            command: cmd_arg,
                            language: lang_arg,
                            cwd: cwd_arg,
                        }),
                    });
                }

                let result = if !allow { json!({"isError":true,"message":"The user denied this tool request. Do not repeat it without a new instruction."}) } else {
                    let args_text = call.arguments.to_string();
                    // Guard: three identical calls in a row end loudly, not in a loop.
                    if let Some(note) = crate::sandbox::check_repetition(&recent_calls, &call.name, &args_text) {
                        recent_calls.push((call.name.clone(), args_text));
                        json!({"isError":true,"message":note})
                    } else {
                        recent_calls.push((call.name.clone(), args_text));
                        let timeout_secs = state
                            .database()
                            .map(|store| crate::sandbox::effective_timeout_secs(&store, tool.timeout().as_secs()))
                            .unwrap_or(tool.timeout().as_secs());
                        let app_state: &AppState = &state;
                        let mut pending_events: Vec<crate::harness::PendingEvent> = Vec::new();
                        let emit_tool_channel = channel.clone();
                        let emit_tool_call_id = call.id.clone();
                        let emit_tool_run_id = run.id.clone();
                        let emit_tool_step_id = step_id.clone();
                        let emit_tool_msg_id = assistant.id.clone();
                        let emit_tool_state = run.status.as_str().to_string();
                        let emit_tool_act = format!("Executing {}", tool.tool.name);
                        let tool_chunk_cb: crate::execution::ChunkCallback = std::sync::Arc::new(move |stream_name, chunk_text| {
                            let _ = emit_tool_channel.send(ChatEvent {
                                run_id: Some(emit_tool_run_id.clone()),
                                step_id: Some(emit_tool_step_id.clone()),
                                seq: Some(0),
                                state: Some(emit_tool_state.clone()),
                                activity: Some(emit_tool_act.clone()),
                                context: None,
                                notice: None,
                                message_id: emit_tool_msg_id.clone(),
                                content: String::new(),
                                reasoning: String::new(),
                                approval: None,
                                elapsed_secs: Some(tool_exec_start.elapsed().as_secs_f64()),
                                round_index: Some(round),
                                timings: None,
                                tool_stream: Some(ToolStreamEvent::OutputChunk {
                                    tool_call_id: emit_tool_call_id.clone(),
                                    stream: stream_name.to_string(),
                                    chunk: chunk_text.to_string(),
                                }),
                            });
                        });
                        let outcome = tokio::select! {
                            _ = cancellation.changed() => None,
                            result = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async {
                                if crate::harness::is_harness_tool(&call.name) {
                                    let emit_run = run.id.clone();
                                    let emit_step = step_id.clone();
                                    let emit_message = assistant.id.clone();
                                    let emit_channel = channel.clone();
                                    let emit: std::sync::Arc<dyn Fn(String, Option<Value>) + Send + Sync> =
                                        std::sync::Arc::new(move |activity, approval| {
                                            // Channel-only progress (seq 0); durability comes from audit rows.
                                            let _ = emit_channel.send(ChatEvent::progress(
                                                &emit_run,
                                                &emit_step,
                                                &emit_message,
                                                &activity,
                                                approval,
                                            ));
                                        });
                                    let ctx = crate::harness::HarnessCtx {
                                        state: app_state,
                                        snapshot: snapshot.clone(),
                                        conversation_id: conversation_id.clone(),
                                        run_id: run.id.clone(),
                                        step_id: step_id.clone(),
                                        access_mode: plan.access_mode,
                                        depth: 0,
                                        preset_id: preset_id.clone(),
                                        inherit_tools: &plan.tools,
                                        emit,
                                    };
                                    match crate::harness::execute(&ctx, &call.name, call.arguments.clone()).await {
                                        Ok(outcome) => {
                                            pending_events.extend(outcome.events);
                                            Ok(outcome.value)
                                        }
                                        Err(error) => Err(error),
                                    }
                                } else {
                                    tool.call_with_stream(call.arguments.clone(), Some(tool_chunk_cb)).await
                                }
                            }) => Some(result.unwrap_or_else(|_| Err("Tool request timed out. Its remote outcome may be unknown; do not automatically retry.".into()))),
                        };
                        for event in pending_events.drain(..) {
                            seq += 1;
                            let _ = state.database()?.append_run_event(&crate::agent_run::RunEvent {
                                run_id: run.id.clone(),
                                seq: seq as u64,
                                step_id: step_id.clone(),
                                tool_call_id: Some(call.id.clone()),
                                event_type: event.event_type,
                                payload: event.payload,
                                created_at: crate::store::now(),
                            });
                        }
                        let duration_ms = tool_exec_start.elapsed().as_millis();
                        let exit_code = match &outcome {
                            Some(Ok(val)) => val.get("exitCode").and_then(|v| v.as_i64()).map(|c| c as i32),
                            _ => None,
                        };
                        let error_info = match &outcome {
                            Some(Ok(val)) => val.get("error").and_then(|v| v.as_str()).map(String::from),
                            Some(Err(err)) => Some(err.clone()),
                            None => Some("Cancelled".into()),
                        };
                        seq += 1;
                        let _ = channel.send(ChatEvent {
                            run_id: Some(run.id.clone()),
                            step_id: Some(step_id.clone()),
                            seq: Some(seq),
                            state: Some(run.status.as_str().to_string()),
                            activity: Some(format!("Finished {}", tool.tool.name)),
                            context: None,
                            notice: None,
                            message_id: assistant.id.clone(),
                            content: String::new(),
                            reasoning: String::new(),
                            approval: None,
                            elapsed_secs: Some(tool_exec_start.elapsed().as_secs_f64()),
                            round_index: Some(round),
                            timings: None,
                            tool_stream: Some(ToolStreamEvent::Finished {
                                tool_call_id: call.id.clone(),
                                exit_code,
                                duration_ms,
                                error: error_info,
                            }),
                        });
                        match outcome {
                            None => {
                                let _ = run.transition_to(crate::agent_run::RunState::Cancelled);
                                let _ = state.database()?.update_run_status(&run.id, run.status, Some("Cancelled; the action may already have changed data."), Some(&step_id));
                                state.database()?.update_message(&row.id,&json!({"request":audit,"result":"Cancelled; the action may already have changed data."}).to_string(),"","interrupted")?;
                                return Ok(false);
                            },
                            Some(Ok(value)) => value,
                            Some(Err(error)) => json!({"isError":true,"message":error}),
                        }
                    }
                };

                research_progress.observe(&tool.tool.name, &result);
                if matches!(tool.tool.name.as_str(), "search" | "visit" | "web_search" | "web_open" | "web_find" | "web_fetch" | "web_fetch_url") {
                    crate::arex::collect_evidence(&result, &mut research_evidence);
                }
                let control = if tool.connector == "Harness" && matches!(call.name.as_str(), "update_context" | "finish") { Some(result.clone()) } else { None };
                if let Some(fallback) = arm_research_finalization(plan_mode, &call.name, &result) {
                    research_fallback = Some(fallback);
                    finalization_retried = false;
                }

                // Excerpt size follows the tokens still free after the counted
                // input and response reserve; full results stay in artifacts.
                let excerpt_chars = crate::artifacts::excerpt_budget(
                    plan.context_length,
                    input_tokens,
                    plan.preferences.max_tokens,
                );
                let (bounded_val, maybe_artifact) = crate::artifacts::bound_tool_result(
                    result,
                    &tool.tool.name,
                    &conversation_id,
                    Some(&run.id),
                    excerpt_chars,
                );
                if let Some(artifact) = &maybe_artifact {
                    let _ = state.database()?.save_artifact(artifact);
                }
                state.database()?.update_message(&row.id,&json!({"request":audit,"result":bounded_val}).to_string(),"","complete")?;
                messages.push(json!({"role":"tool","tool_call_id":call.id,"content":bounded_val.to_string()}));
                if let Some(control) = control {
                    explicit_finish = crate::arex::apply_control(&call.name, &control, &*state.database()?, &conversation_id, &run.id, &mut messages)?;
                    if explicit_finish.is_some() {
                        finish_args = Some(call.arguments.clone());
                    }
                }
                if let Ok(store) = state.database() {
                    emit_session_event(
                        &store,
                        &conversation_id,
                        Some(&run.id),
                        Some(&step_id),
                        Some(&call.id),
                        "tool_result",
                        json!({
                            "callId": call.id,
                            "name": tool.tool.name,
                            "decision": if allow { "allowed" } else { "denied" },
                            "result": bounded_val,
                            "artifactId": maybe_artifact.as_ref().map(|a| a.id.clone()),
                        }),
                        false,
                    );
                }
            }
            if let Some(final_answer) = explicit_finish {
                let finish_args = finish_args.take();
                present_research_answer(&state, &channel, &run, &step_id, &conversation_id, &assistant.id, &mut seq, &mut answer, &final_answer, finish_args.as_ref(), &research_evidence).await?;
                return Ok(true);
            }
            if !tool_use_denied && research_progress.stopped() {
                research_fallback = Some(RESEARCH_STALLED_FALLBACK.into());
                messages.push(json!({"role":"user","content":RESEARCH_STALLED}));
                let mut event = ChatEvent::progress(&run.id, &step_id, &assistant.id, "Research stopped · preparing answer…", None);
                event.notice = Some("Research is no longer making progress or has reached its budget. Preparing an answer from the collected results.".into());
                channel.send(event).map_err(|e| e.to_string())?;
            } else if !tool_use_denied && research_progress.calls > research_calls_before && research_progress.stagnant == 2 {
                messages.push(json!({"role":"user","content":"The last web calls added no new evidence. Do not rephrase the same search. Read an unvisited source, change the source or research approach, or answer from existing evidence and explain the gap."}));
            }
            let _ = run.transition_to(crate::agent_run::RunState::PreparingNextRound);
            let _ = state.database()?.update_run_status(&run.id, run.status, None, Some(&step_id));
            if let Ok(store) = state.database() {
                emit_session_event(
                    &store,
                    &conversation_id,
                    Some(&run.id),
                    Some(&step_id),
                    None,
                    "step_end",
                    json!({"round": round, "content": round_answer, "toolCallsCount": tool_calls_count}),
                    false,
                );
            }
        }
        Err("Tool round limit reached.".into())
    }.await;
    let status = match &result {
        Ok(true) => "complete",
        Ok(false) => "interrupted",
        Err(_) => "error",
    };
    // A delivered answer means nothing is still in progress. Models routinely
    // finish without todo bookkeeping, so close out open items here — the
    // checklist must not sit on a stale "Now" badge after the turn ends.
    if matches!(result, Ok(true)) {
        if let Ok(store) = state.database() {
            if let Ok(mut todos) = store.todos(&conversation_id) {
                if todos.iter().any(|todo| todo.status != crate::plans::COMPLETED) {
                    let now = crate::store::now();
                    for todo in &mut todos {
                        if todo.status != crate::plans::COMPLETED {
                            todo.status = crate::plans::COMPLETED.into();
                            todo.updated_at = now;
                        }
                    }
                    if store.save_todos(&conversation_id, &todos).is_ok() {
                        emit_session_event(
                            &store,
                            &conversation_id,
                            Some(&run.id),
                            None,
                            None,
                            "plan",
                            json!({"kind": "todos", "count": todos.len(), "closed": true}),
                            false,
                        );
                    }
                }
            }
        }
    }
    state.database()?.finish_message(
        &assistant.id,
        &answer,
        &reasoning,
        status,
        result.as_ref().err().map(String::as_str),
    )?;
    let final_run_state = match &result {
        Ok(true) => crate::agent_run::RunState::Completed,
        Ok(false) => crate::agent_run::RunState::Cancelled,
        Err(_) => crate::agent_run::RunState::Failed,
    };
    let _ = run.transition_to(final_run_state);
    let _ = state.database()?.update_run_status(
        &run.id,
        run.status,
        result.as_ref().err().map(String::as_str),
        None,
    );
    if let Ok(store) = state.database() {
        emit_session_event(
            &store,
            &conversation_id,
            Some(&run.id),
            None,
            None,
            "turn_end",
            json!({
                "status": status,
                "error": result.as_ref().err().map(String::as_str),
                "answerLength": answer.len(),
                "reasoningLength": reasoning.len(),
            }),
            false,
        );
    }
    if let Some(id) = &checkpoint_id {
        if let Some(payload) = crate::checkpoints::end_turn(&state, id).await {
            if let Ok(store) = state.database() {
                emit_session_event(&store, &conversation_id, Some(&run.id), None, None, "checkpoint", payload, true);
            }
        }
    }
    // Persist the slot only after a completed turn: a cancelled request can
    // leave the slot busy, and a failed save would only be noise.
    if matches!(result, Ok(true)) {
        if let Some(payload) = crate::kv_slots::save_after_turn(&state, &plan.backend, &conversation_id).await {
            if let Ok(store) = state.database() {
                emit_session_event(&store, &conversation_id, Some(&run.id), None, None, "kv_slot", payload, true);
            }
        }
    }
    result.map(|_| ())
}

fn check_finish_reason(reason: &str) -> Result<(), String> {
    match reason {
        "length" => Err("Response token limit reached before the model finished. Increase Maximum response tokens in Models & runtime, or ask for a shorter response. Any partial output has been saved; unfinished tool calls were not executed.".into()),
        "content_filter" => Err("The model runtime stopped this response because of its content filter.".into()),
        _ => Ok(()),
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreflightBreakdown {
    /// Exact or estimated tokens of the whole rendered payload (including the draft).
    pub total: u64,
    /// Frozen system prompt, skill guidance, and static style (not volatile injections).
    pub instructions: u64,
    /// Selected tool schemas.
    pub tools: u64,
    /// Replayed conversation history (including past tool exchanges).
    pub history: u64,
    /// Volatile scratchpad (memory, plan, unavailable tools) on the current user turn.
    pub scratchpad: u64,
    /// The current composer draft as a user message.
    pub draft: u64,
    pub response_reserve: u32,
    pub context_length: u32,
    /// True when every count comes from the runtime tokenizer; false for
    /// provider byte estimates.
    pub exact: bool,
    /// False when total plus the response reserve exceeds the context.
    pub fits: bool,
    /// Connector notice also shown at send time (e.g. unavailable selections).
    pub notice: Option<String>,
    /// Guidance shown only when the draft does not fit.
    pub overflow: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextPreflight {
    pub available: bool,
    pub reason: Option<String>,
    pub breakdown: Option<PreflightBreakdown>,
}

fn unavailable_preflight(reason: String) -> ContextPreflight {
    ContextPreflight { available: false, reason: Some(reason), breakdown: None }
}

/// Count the current draft against the same rendered payload a send would
/// build, split into instructions, tool schemas, history, and the draft.
/// Advisory only: failures return `available: false` instead of blocking.
#[tauri::command]
pub async fn context_preflight(
    state: State<'_, AppState>,
    conversation_id: Option<String>,
    draft: String,
    preset: Option<String>,
    connector_ids: Option<Vec<String>>,
    connector_tools: Option<Vec<crate::connectors::ToolSelection>>,
    plan_mode: Option<bool>,
) -> Result<ContextPreflight, String> {
    let conversation_id = conversation_id.unwrap_or_default();
    if draft.len() > 1_048_576 {
        return Ok(unavailable_preflight("Drafts count toward the same 1 MiB message limit.".into()));
    }
    let mut plan = match assemble_turn(
        &state,
        &conversation_id,
        &draft,
        connector_ids.unwrap_or_default(),
        connector_tools,
        preset.as_deref(),
    )
    .await
    {
        Ok(plan) => plan,
        Err(error) => return Ok(unavailable_preflight(error)),
    };
    inject_plan_mode(&mut plan, plan_mode.unwrap_or(false));
    let frozen = frozen_system_for(&plan);
    let volatile = crate::prompt::volatile_scratchpad(&plan.injections);
    let messages = crate::prompt::build_turn_messages(
        &frozen,
        &plan.history,
        volatile.as_deref(),
        &plan.draft_message,
    );
    let without_scratchpad = crate::prompt::build_turn_messages(
        &frozen,
        &plan.history,
        None,
        &plan.draft_message,
    );
    let prefix = crate::prompt::cache_prefix(&without_scratchpad);
    let core = without_scratchpad.first().cloned().into_iter().collect::<Vec<_>>();

    async fn count(
        plan: &TurnPlan,
        messages: &[Value],
        tools: &[Value],
    ) -> Result<u64, String> {
        let messages = crate::prompt::with_user_turn(messages);
        plan.backend
            .count_tokens(&plan.backend.payload(&request_payload(&messages, tools, &plan.preferences, false, false, false, None, false)))
            .await
    }

    let (total, without_scratchpad_count, prefix_count, core_with_tools, instructions) = match tokio::try_join!(
        count(&plan, &messages, &plan.tool_definitions),
        count(&plan, &without_scratchpad, &plan.tool_definitions),
        count(&plan, &prefix, &plan.tool_definitions),
        count(&plan, &core, &plan.tool_definitions),
        count(&plan, &core, &[]),
    ) {
        Ok(values) => values,
        Err(error) => return Ok(unavailable_preflight(error)),
    };
    let fits = crate::context::validate_budget(total, plan.preferences.max_tokens, plan.context_length).is_ok();
    Ok(ContextPreflight {
        available: true,
        reason: None,
        breakdown: Some(PreflightBreakdown {
            total,
            instructions,
            tools: core_with_tools.saturating_sub(instructions),
            history: prefix_count.saturating_sub(core_with_tools),
            scratchpad: total.saturating_sub(without_scratchpad_count),
            draft: without_scratchpad_count.saturating_sub(prefix_count),
            response_reserve: plan.preferences.max_tokens,
            context_length: plan.context_length,
            exact: !plan.backend.context_is_estimate(),
            fits,
            notice: plan.tool_notice.clone(),
            overflow: (!fits).then(|| {
                "This draft plus the response reserve will not fit the loaded context. Shorten the message, deselect tools, compact the conversation, or increase the context and reload the model.".into()
            }),
        }),
    })
}

#[allow(clippy::too_many_arguments)]
fn request_payload(
    messages: &[Value],
    tools: &[Value],
    preferences: &crate::store::Preferences,
    denied: bool,
    finalizing: bool,
    planning: bool,
    id_slot: Option<i64>,
    arex: bool,
) -> Value {
    let mut payload = json!({"messages":messages,"temperature":preferences.temperature,"top_p":preferences.top_p,"max_tokens":preferences.max_tokens,"stream":true,"cache_prompt":true});
    preferences.sampling.apply(&mut payload);
    // ZAYA's hybrid reasoning loop can otherwise consume the entire response
    // reserve without emitting an answer. Its runtime supports this field and
    // treats it as a separate thinking budget; other local models are unchanged.
    // (For AREX a forced budget close spills the thinking into visible content
    // instead — verified on llama.cpp b10855 — so it gets enable_thinking only.)
    if preferences.model_path.ends_with(crate::model_catalog::ZAYA1_FILENAME) {
        payload["reasoning_budget_tokens"] = json!(if finalizing { 0 } else { 2048.min(preferences.max_tokens / 2) });
    }
    // Keep the tools array identical on every round — including finalizing and
    // planning rounds. Strict templates (AREX) render <tools> at the very top of
    // the prompt, so omitting it shifts the token stream at position ~3 and
    // forces a full reprocess. tool_choice:none still prevents actual calls.
    if !tools.is_empty() {
        payload["tools"] = json!(tools);
    }
    // …except local AREX: its trained answer path is the finish tool, so the
    // finalizing round leaves tool_choice at auto. A structured finish is then
    // executed like any finish call; stray markup is stripped by the loop.
    if denied || planning || (finalizing && !arex) {
        payload["tool_choice"] = json!("none");
    }
    // On a finalizing round thinking only competes with the answer for the
    // response reserve — disable it for the templates that honor the kwarg.
    if finalizing && (arex || preferences.model_path.ends_with(crate::model_catalog::MODEL_FILENAME)) {
        payload["chat_template_kwargs"] = json!({"enable_thinking": false});
    }
    if let Some(slot) = id_slot {
        payload["id_slot"] = json!(slot);
    }
    payload
}

#[cfg(test)]
mod finish_tests {
    #[test]
    fn connector_round_without_an_answer_requests_finalization() {
        // Earlier progress text is deliberately not an input: only this round counts.
        assert!(super::needs_final_answer(false, false, ""));
        assert!(super::needs_final_answer(false, false, " \n"));
        assert!(!super::needs_final_answer(false, true, ""));
        assert!(!super::needs_final_answer(false, false, "The result is 42."));
        assert!(super::needs_final_answer(true, true, ""));
    }

    #[test]
    fn text_tool_markup_is_not_a_final_answer() {
        assert_eq!(super::finalization_action("<tool_call>{}</tool_call>", None, false, false), super::FinalizationAction::Retry);
        // A call appended after commentary is still not an answer: retry once,
        // then fall back to the grounded research result.
        assert_eq!(super::finalization_action("Let me search.\n<tool_call>{}</tool_call>", None, false, false), super::FinalizationAction::Retry);
        assert_eq!(
            super::finalization_action("Let me search.\n<tool_call>{}</tool_call>", Some("verified answer"), true, false),
            super::FinalizationAction::Fallback("Research result:\n\nverified answer".into()),
        );
    }

    #[test]
    fn thinking_only_exhaustion_gets_one_tool_free_answer_attempt() {
        let mut filter = super::ThinkFilter::new();
        let (answer, reasoning) = filter.feed("<think>The tool found the result, but the response budget ran out.");
        assert!(answer.is_empty());
        assert!(!reasoning.is_empty());
        assert!(super::needs_final_answer(false, false, &answer));
        assert_eq!(super::finalization_action(&answer, None, false, false), super::FinalizationAction::Retry);
        let prefs = crate::store::Preferences { model_path: crate::model_catalog::ZAYA1_FILENAME.into(), ..Default::default() };
        let payload = super::request_payload(&[], &[serde_json::json!({"type":"function"})], &prefs, false, true, false, None, false);
        // Tools stay rendered on finalizing rounds so the prompt prefix (and
        // llama.cpp cache) is identical to tool-call rounds.
        assert!(payload.get("tools").is_some());
        assert_eq!(payload["tool_choice"], "none");
        assert_eq!(payload["reasoning_budget_tokens"], 0);
        assert_eq!(super::finalization_action("", None, true, false), super::FinalizationAction::Error);
        assert_eq!(super::finalization_action("The result is 42.", None, true, false), super::FinalizationAction::Complete);
        assert!(super::check_finish_reason("length").is_err(), "incomplete tool calls must still fail");
    }
    #[test]
    fn arex_context_rewrites_do_not_reset_research_progress() {
        let mut progress = super::ResearchProgress::default();
        progress.observe("search", &serde_json::json!({"results":[{"url":"https://example.org"}]}));
        for i in 0..4 {
            progress.observe("update_context", &serde_json::json!({"context":format!("reworded notes {i}"),"text":format!("new wording {i}")}));
        }
        assert!(progress.stopped());
    }

    #[test]
    fn research_progress_ignores_mcp_metadata_and_stays_stopped() {
        let mut progress = super::ResearchProgress::default();
        for i in 0..5 {
            let wrapped = serde_json::json!({"content":[{"type":"text","text":serde_json::json!({"request_id":i,"results":[{"url":"https://example.org","excerpts":["same passage"]}]}).to_string()}]});
            progress.observe("web_fetch", &wrapped);
        }
        assert!(progress.stopped());
        progress.observe("web_open", &serde_json::json!({"text":"late new evidence"}));
        assert!(progress.stopped());
        let payload = super::request_payload(&[], &[serde_json::json!({"type":"function"})], &crate::store::Preferences::default(), false, true, false, None, false);
        assert!(payload.get("tools").is_some());
        assert_eq!(payload["tool_choice"], "none");
    }

    #[test]
    fn research_progress_has_a_budget_even_when_results_keep_changing() {
        let mut progress = super::ResearchProgress::default();
        for i in 0..16 {
            // Each hit on a genuinely new domain is progress; the call budget
            // is the only brake left once domains keep changing.
            let result = serde_json::json!({"results":[{"url":format!("https://site{i}.example.com/page"),"text":format!("result {i}")}]});
            progress.observe("web_search", &result);
            assert_eq!(progress.stopped(), i == 15);
        }
    }

    #[test]
    fn research_progress_stops_reworded_searches_with_repeated_evidence() {
        let mut progress = super::ResearchProgress::default();
        for i in 0..5 {
            let result = serde_json::json!({"requestId":i,"query":format!("query {i}"),"results":[{"url":"https://example.org","text":"same evidence"}]});
            progress.observe("web_search", &result);
            assert_eq!(progress.stopped(), i >= 3);
        }
    }

    #[test]
    fn research_progress_stops_searches_recycling_seen_domains() {
        // Different snippets on already-seen domains are not progress for a
        // discovery call — this is the junk-but-novel failure mode.
        let mut progress = super::ResearchProgress::default();
        for i in 0..4 {
            let result = serde_json::json!({"results":[{"url":format!("https://arxiv.org/abs/{i}"),"text":format!("novel snippet {i}")},{"url":format!("https://github.com/x/{i}"),"text":format!("other {i}")}]});
            progress.observe("search", &result);
            assert_eq!(progress.stopped(), i == 3);
        }
        // A search that surfaces a new domain still counts as progress.
        let mut fresh = super::ResearchProgress::default();
        fresh.observe("search", &serde_json::json!({"results":[{"url":"https://a.example.com/1","text":"x"}]}));
        fresh.observe("search", &serde_json::json!({"results":[{"url":"https://b.example.com/2","text":"y"}]}));
        assert!(!fresh.stopped());
    }

    #[test]
    fn research_progress_allows_new_reading_and_ignores_other_tools() {
        let mut progress = super::ResearchProgress::default();
        for _ in 0..10 { progress.observe("read_file", &serde_json::json!({})); }
        assert!(!progress.stopped());
        for i in 0..10 {
            progress.observe("web_open", &serde_json::json!({"text":format!("new passage {i}")}));
            assert!(!progress.stopped());
        }
        for _ in 0..4 { progress.observe("web_fetch", &serde_json::json!({"isError":true,"message":"failed"})); }
        assert!(progress.stopped());
    }

    #[test]
    fn zaya_payload_caps_reasoning_without_changing_other_models() {
        let messages = vec![serde_json::json!({"role":"user","content":"Hello"})];
        let zaya = crate::store::Preferences { model_path: "ZAYA1-8B-Q4_K_M.gguf".into(), max_tokens: 8192, ..Default::default() };
        let regular = crate::store::Preferences::default();
        assert_eq!(super::request_payload(&messages, &[], &zaya, false, false, false, None, false)["reasoning_budget_tokens"], 2048);
        let small_budget = crate::store::Preferences { max_tokens: 512, ..zaya.clone() };
        assert_eq!(super::request_payload(&messages, &[], &small_budget, false, false, false, None, false)["reasoning_budget_tokens"], 256);
        assert!(super::request_payload(&messages, &[], &regular, false, false, false, None, false).get("reasoning_budget_tokens").is_none());
        let slotted = super::request_payload(&messages, &[], &regular, false, false, false, Some(0), false);
        assert_eq!(slotted["id_slot"], 0);
        assert_eq!(slotted["cache_prompt"], true);
    }
    #[test]
    fn token_exhaustion_is_not_treated_as_success_or_executable_tool_output() {
        assert!(super::check_finish_reason("length")
            .unwrap_err()
            .contains("unfinished tool calls were not executed"));
        assert!(super::check_finish_reason("stop").is_ok());
        assert!(super::check_finish_reason("tool_calls").is_ok());
        assert!(super::check_finish_reason("content_filter").is_err());
    }

    #[test]
    fn web_research_answer_is_available_when_final_synthesis_is_empty() {
        let result = serde_json::json!({
            "answer": "The verified answer is 42. [S1](https://example.com)",
            "sources": {"S1": {"url": "https://example.com"}},
        });
        let fallback = super::research_answer_fallback("web_search", &result);
        assert_eq!(fallback.as_deref(), Some("The verified answer is 42. [S1](https://example.com)"));
        assert_eq!(
            super::finalization_action("", fallback.as_deref(), false, false),
            super::FinalizationAction::Retry,
        );
        assert_eq!(
            super::finalization_action("", fallback.as_deref(), true, false),
            super::FinalizationAction::Fallback("Research result:\n\nThe verified answer is 42. [S1](https://example.com)".into()),
        );
    }

    #[test]
    fn empty_final_synthesis_without_research_fails_loudly() {
        assert_eq!(
            super::finalization_action("\n  ", None, true, false),
            super::FinalizationAction::Error,
        );
        assert_eq!(
            super::finalization_action("A visible answer", Some("research"), false, false),
            super::FinalizationAction::Complete,
        );
    }

    #[test]
    fn sampler_fields_are_sent_only_when_configured() {
        let messages = vec![serde_json::json!({"role":"user","content":"Hello"})];
        let plain = super::request_payload(&messages, &[], &crate::store::Preferences::default(), false, false, false, None, false);
        assert!(plain.get("seed").is_none());
        assert!(plain.get("top_k").is_none());
        let mut preferences = crate::store::Preferences::default();
        preferences.sampling.seed = Some(42);
        preferences.sampling.min_p = Some(0.05);
        let seeded = super::request_payload(&messages, &[], &preferences, false, false, false, None, false);
        assert_eq!(seeded["seed"], 42);
        assert_eq!(seeded["min_p"], 0.05);
        assert!(seeded.get("repeat_penalty").is_none());
    }

    #[test]
    fn stalled_turn_rejects_uncited_narration_as_an_answer() {
        // The failure mode: a stalled research loop emits "Let me synthesize…"
        // process talk that reads like an answer but cites nothing collected.
        let narration = "Let me synthesize what I've gathered so far. Based on my research, I need to identify a candidate.";
        assert_eq!(
            super::finalization_action(narration, Some("best collected answer"), false, true),
            super::FinalizationAction::Retry,
        );
        assert_eq!(
            super::finalization_action(narration, Some("best collected answer"), true, true),
            super::FinalizationAction::Fallback("Research result:\n\nbest collected answer".into()),
        );
        // A stalled answer that does cite a source still completes.
        assert_eq!(
            super::finalization_action("Per https://a.example/report, X is Y.", Some("fallback"), true, true),
            super::FinalizationAction::Complete,
        );
        // Non-stalled turns keep accepting citation-free answers.
        assert_eq!(
            super::finalization_action(narration, Some("fallback"), true, false),
            super::FinalizationAction::Complete,
        );
    }

    #[test]
    fn finalization_forces_tool_choice_none_and_minicpm_thinking_off() {
        let messages = vec![serde_json::json!({"role":"user","content":"Hello"})];
        let preferences = crate::store::Preferences {
            model_path: crate::model_catalog::MODEL_FILENAME.into(),
            ..Default::default()
        };
        let tools = vec![serde_json::json!({"type":"function","function":{"name":"web_search"}})];
        let payload = super::request_payload(&messages, &tools, &preferences, false, true, false, None, false);
        // Tools stay in the payload: templates render them at the prompt head,
        // so removing them would invalidate the whole KV cache prefix.
        assert!(payload.get("tools").is_some());
        assert_eq!(payload["tool_choice"], "none");
        assert_eq!(payload["chat_template_kwargs"]["enable_thinking"], false);
    }

    #[test]
    fn arex_finalization_keeps_tool_channel_and_disables_thinking() {
        // AREX's trained answer path is the finish tool — pinning
        // tool_choice:none removes it and the model leaks call markup instead.
        // The round loop executes a finish call and strips other call markup.
        let messages = vec![serde_json::json!({"role":"user","content":"Hello"})];
        let preferences = crate::store::Preferences {
            model_path: "BAAI_AREX-Turbo-Q4_K_M.gguf".into(),
            ..Default::default()
        };
        let tools = vec![serde_json::json!({"type":"function","function":{"name":"finish"}})];
        let payload = super::request_payload(&messages, &tools, &preferences, false, true, false, None, true);
        assert!(payload.get("tools").is_some());
        assert!(payload.get("tool_choice").is_none());
        assert_eq!(payload["chat_template_kwargs"]["enable_thinking"], false);
        // Only the finalizing round opens the channel — normal rounds and
        // denied/planning rounds are unchanged.
        let normal = super::request_payload(&messages, &tools, &preferences, false, false, false, None, true);
        assert!(normal.get("tool_choice").is_none());
        assert!(normal.get("chat_template_kwargs").is_none());
        let denied = super::request_payload(&messages, &tools, &preferences, true, true, false, None, true);
        assert_eq!(denied["tool_choice"], "none");
        let planning = super::request_payload(&messages, &tools, &preferences, false, false, true, None, true);
        assert_eq!(planning["tool_choice"], "none");
    }

    #[test]
    fn suppressed_call_markup_never_reaches_the_answer() {
        assert_eq!(
            super::strip_call_markup("Notes.\n<tool_call><function=search><parameter=query>[\"x\"]</parameter></function></tool_call>"),
            "Notes."
        );
        // Unterminated markup (finish_reason=length mid-call) truncates too.
        assert_eq!(super::strip_call_markup("prose <tool_call>{\"name\":"), "prose");
        assert_eq!(super::strip_call_markup("prose <function=search>{\"q\":"), "prose");
        assert_eq!(super::strip_call_markup("clean answer"), "clean answer");
    }

    #[test]
    fn text_form_tool_markup_is_not_treated_as_an_answer() {
        assert!(super::is_text_tool_call("<function=web_search>"));
        assert!(super::is_text_tool_call("  <tool_call>{}"));
        // Calls embedded after commentary count too — the model's intent is
        // unambiguous once a block exists or `<tool_call>` markup appears.
        assert!(super::is_text_tool_call("Let me search first.\n<tool_call>{}</tool_call>"));
        assert!(super::is_text_tool_call("Checking: <tool_call>{\"name\":"));
        assert!(super::is_text_tool_call("prose <function=search>{}</function>"));
        // A stray `<function=` mention without a closing tag stays prose.
        assert!(!super::is_text_tool_call("Here is an example: <function=search>."));
    }

    #[test]
    fn missing_workspace_skips_workspace_tools_without_failing_the_turn() {
        let mut tools = Vec::new();
        let notice = super::append_workspace_tools(&mut tools, "", false);

        assert!(tools.is_empty());
        assert_eq!(
            notice.as_deref(),
            Some("Workspace Files was skipped because this conversation has no workspace. Select a project or choose a folder in Tools to use workspace files."),
        );
    }

    #[test]
    fn think_filter_parses_normal_text() {
        let mut filter = super::ThinkFilter::new();
        let (ans, th) = filter.feed("Hello world");
        assert_eq!(ans, "Hello world");
        assert_eq!(th, "");
        let (ans, th) = filter.flush();
        assert_eq!(ans, "");
        assert_eq!(th, "");
    }

    #[test]
    fn think_filter_parses_single_chunk_think() {
        let mut filter = super::ThinkFilter::new();
        let (ans, th) = filter.feed("<think>Let me think</think>Here is the answer");
        assert_eq!(th, "Let me think");
        assert_eq!(ans, "Here is the answer");
    }

    #[test]
    fn think_filter_parses_split_chunks() {
        let mut filter = super::ThinkFilter::new();
        let (a1, t1) = filter.feed("<th");
        assert_eq!(a1, "");
        assert_eq!(t1, "");

        let (a2, t2) = filter.feed("ink>Reasoning");
        assert_eq!(a2, "");
        assert_eq!(t2, "Reasoning");

        let (a3, t3) = filter.feed(" more</th");
        assert_eq!(a3, "");
        assert_eq!(t3, " more");

        let (a4, t4) = filter.feed("ink>Answer");
        assert_eq!(a4, "Answer");
        assert_eq!(t4, "");
    }

    #[test]
    fn think_filter_handles_false_alarm_tags() {
        let mut filter = super::ThinkFilter::new();
        let (ans, th) = filter.feed("<this is not a tag> and some text");
        assert_eq!(ans, "<this is not a tag> and some text");
        assert_eq!(th, "");
    }

    #[test]
    fn think_filter_handles_false_alarm_prefix_and_flush() {
        let mut filter = super::ThinkFilter::new();
        let (a1, t1) = filter.feed("<thin");
        assert_eq!(a1, "");
        assert_eq!(t1, "");

        let (a2, t2) = filter.feed("g of beauty");
        assert_eq!(a2, "<thing of beauty");
        assert_eq!(t2, "");

        let (a3, t3) = filter.feed("<think>in progress");
        assert_eq!(a3, "");
        assert_eq!(t3, "in progress");

        let (flush_a, flush_t) = filter.flush();
        assert_eq!(flush_a, "");
        assert_eq!(flush_t, "");
    }

    #[test]
    fn tool_stream_event_serialization_matches_frontend_contract() {
        let started = super::ToolStreamEvent::Started {
            tool_call_id: "call-1".into(),
            tool_name: "execute_command".into(),
            command: Some("npm test".into()),
            language: None,
            cwd: Some("c:/workspace".into()),
        };
        let started_val = serde_json::to_value(&started).unwrap();
        assert_eq!(started_val["type"], "started");
        assert_eq!(started_val["toolCallId"], "call-1");
        assert_eq!(started_val["toolName"], "execute_command");
        assert_eq!(started_val["command"], "npm test");
        assert_eq!(started_val["cwd"], "c:/workspace");

        let chunk = super::ToolStreamEvent::OutputChunk {
            tool_call_id: "call-1".into(),
            stream: "stdout".into(),
            chunk: "PASS 1 test\n".into(),
        };
        let chunk_val = serde_json::to_value(&chunk).unwrap();
        assert_eq!(chunk_val["type"], "outputChunk");
        assert_eq!(chunk_val["toolCallId"], "call-1");
        assert_eq!(chunk_val["stream"], "stdout");
        assert_eq!(chunk_val["chunk"], "PASS 1 test\n");

        let finished = super::ToolStreamEvent::Finished {
            tool_call_id: "call-1".into(),
            exit_code: Some(0),
            duration_ms: 1250,
            error: None,
        };
        let finished_val = serde_json::to_value(&finished).unwrap();
        assert_eq!(finished_val["type"], "finished");
        assert_eq!(finished_val["toolCallId"], "call-1");
        assert_eq!(finished_val["exitCode"], 0);
        assert_eq!(finished_val["durationMs"], 1250);
        assert!(finished_val["error"].is_null());
    }

    #[test]
    fn planning_payload_keeps_tools_and_forces_tool_choice_none() {
        let messages = vec![serde_json::json!({"role":"user","content":"Build a thing"})];
        let preferences = crate::store::Preferences::default();
        let tools = vec![serde_json::json!({"type":"function","function":{"name":"web_search"}})];
        let planning = super::request_payload(&messages, &tools, &preferences, false, false, true, None, false);
        // Same rendered prefix as tool-call rounds — cache survives the
        // plan→implement transition; tool_choice:none blocks calls.
        assert!(planning.get("tools").is_some());
        assert_eq!(planning["tool_choice"], "none");
        let implementing = super::request_payload(&messages, &tools, &preferences, false, false, false, None, false);
        assert!(implementing.get("tools").is_some());
        assert!(implementing.get("tool_choice").is_none());
        let slotted = super::request_payload(&messages, &[], &preferences, false, false, true, Some(0), false);
        assert_eq!(slotted["id_slot"], 0);
        assert_eq!(slotted["cache_prompt"], true);
    }

    #[test]
    fn plan_mode_does_not_lock_tools_after_the_first_web_search() {
        let result = serde_json::json!({
            "answer": "Partial hit. [S1](https://example.com)",
            "sources": {"S1": {"url": "https://example.com"}},
        });
        assert!(super::arm_research_finalization(true, "web_search", &result).is_none());
        assert_eq!(
            super::arm_research_finalization(false, "web_search", &result).as_deref(),
            Some("Partial hit. [S1](https://example.com)"),
        );
    }

    #[test]
    fn begin_plan_implementation_rewrites_the_draft_and_appends_phase_two() {
        let prefix = crate::prompt::volatile_scratchpad(&[(
            "plan_mode".into(),
            super::PLAN_MODE_INSTRUCTION.into(),
        )])
        .unwrap();
        let mut messages = vec![
            serde_json::json!({"role":"system","content":"sys"}),
            serde_json::json!({"role":"user","content":format!("{prefix}\n\nInvestigate fusion yield")}),
        ];
        super::begin_plan_implementation(&mut messages, "1. Search\n2. Calculate\n3. Verify").unwrap();
        assert_eq!(messages.len(), 4);
        let draft = messages[1]["content"].as_str().unwrap();
        assert!(draft.contains(super::PLAN_MODE_ACTIVE));
        assert!(!draft.contains("Do not call tools and do not implement yet"));
        assert!(draft.ends_with("Investigate fusion yield"));
        assert_eq!(messages[2]["role"], "assistant");
        assert_eq!(messages[2]["content"], "1. Search\n2. Calculate\n3. Verify");
        assert_eq!(messages[3]["role"], "user");
        assert!(messages[3]["content"].as_str().unwrap().contains("Continue through the plan while making progress"));
        assert!(messages[3]["content"].as_str().unwrap().contains("report the unresolved steps honestly"));
        assert!(super::begin_plan_implementation(&mut messages, "   ").is_err());
    }

    #[test]
    fn begin_plan_implementation_rewrites_multimodal_draft_prefix() {
        let prefix = crate::prompt::volatile_scratchpad(&[(
            "plan_mode".into(),
            super::PLAN_MODE_INSTRUCTION.into(),
        )])
        .unwrap();
        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [
                {"type": "text", "text": format!("{prefix}\n")},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,aa"}}
            ]
        })];
        super::begin_plan_implementation(&mut messages, "1. Look").unwrap();
        let text = messages[0]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains(super::PLAN_MODE_ACTIVE));
        assert!(!text.contains("Do not call tools and do not implement yet"));
        assert_eq!(messages[1]["role"], "assistant");
    }

    #[test]
    fn refresh_plan_focus_keeps_the_model_on_the_current_task() {
        let mut messages = vec![
            serde_json::json!({"role":"user","content":"Investigate"}),
            serde_json::json!({"role":"assistant","content":"1. Search\n2. Open"}),
            serde_json::json!({"role":"user","content":super::PLAN_IMPLEMENT_INSTRUCTION}),
        ];
        let todos = vec![
            crate::plans::Todo { text: "Search".into(), status: crate::plans::COMPLETED.into(), updated_at: 0 },
            crate::plans::Todo { text: "Open the source".into(), status: crate::plans::IN_PROGRESS.into(), updated_at: 0 },
        ];
        super::refresh_plan_focus(&mut messages, &todos);
        let focus = messages[2]["content"].as_str().unwrap();
        assert!(focus.contains("The plan above is ready"));
        assert!(focus.contains("one task at a time"));
        assert!(focus.contains("1/2 complete"));
        assert!(focus.contains("Open the source"));
        assert!(focus.contains("index 1"));
        super::refresh_plan_focus(&mut messages, &todos);
        assert_eq!(messages.len(), 3);
    }
}
