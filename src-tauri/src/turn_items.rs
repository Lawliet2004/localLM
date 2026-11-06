//! Journal-backed lifecycle for the reasoning emitted during one model round.
//! The same committed event is delivered live and read back from the journal.
use crate::{chat::ChatEvent, sessions::SessionEvent, AppState};
use serde_json::json;
use std::time::{Duration, Instant};
use tauri::ipc::Channel;

const FLUSH_INTERVAL: Duration = Duration::from_millis(34);

#[allow(clippy::too_many_arguments)]
fn emit_item(state: &AppState, channel: &Channel<ChatEvent>, conversation_id: &str, run_id: &str, step_id: &str, message_id: &str, kind: &str, payload: serde_json::Value) -> bool {
    let mut event = SessionEvent { id: uuid::Uuid::new_v4().to_string(), conversation_id: conversation_id.into(),
        run_id: Some(run_id.into()), seq: 0, step_id: Some(step_id.into()), tool_call_id: None,
        event_type: kind.into(), payload, ignorable: false, created_at: crate::store::now() };
    let seq = match state.database().and_then(|store| store.append_session_event(&event)) {
        Ok(seq) => seq,
        Err(_) => return false,
    };
    event.seq = seq;
    let mut live = ChatEvent::progress(run_id, step_id, message_id, "", None);
    live.activity = None;
    live.item = Some(event);
    let _ = channel.send(live);
    true
}

pub struct ReasoningItem<'a> {
    state: &'a AppState,
    channel: &'a Channel<ChatEvent>,
    conversation_id: &'a str,
    run_id: &'a str,
    step_id: &'a str,
    message_id: &'a str,
    round: usize,
    id: String,
    started_at: Option<i64>,
    first: Option<Instant>,
    last: Option<Instant>,
    last_flush: Instant,
    pending: String,
    closed: bool,
    unavailable: bool,
}

impl<'a> ReasoningItem<'a> {
    pub fn new(state: &'a AppState, channel: &'a Channel<ChatEvent>, conversation_id: &'a str, run_id: &'a str, step_id: &'a str, message_id: &'a str, round: usize) -> Self {
        Self { state, channel, conversation_id, run_id, step_id, message_id, round,
            id: uuid::Uuid::new_v4().to_string(), started_at: None, first: None, last: None,
            last_flush: Instant::now(), pending: String::new(), closed: false, unavailable: false }
    }

    fn emit(&mut self, kind: &str, payload: serde_json::Value) {
        if self.unavailable { return; }
        // The live channel carries the committed event itself. Existing
        // journal polling recovers it after a reload or missed delivery;
        // a second invalidation here would fetch the same row on every delta.
        self.unavailable = !emit_item(self.state, self.channel, self.conversation_id, self.run_id, self.step_id, self.message_id, kind, payload);
    }

    pub fn push(&mut self, text: &str) {
        if text.is_empty() || self.closed || self.unavailable { return; }
        let now = Instant::now();
        let first_chunk = self.started_at.is_none();
        if first_chunk {
            let started_at = crate::store::now();
            self.started_at = Some(started_at);
            self.first = Some(now);
            self.emit("item.started", json!({"item": {"id":self.id,"runId":self.run_id,"round":self.round,
                "kind":"reasoning","status":"running","startedAt":started_at,"payload":{}}}));
        }
        self.last = Some(now);
        self.pending.push_str(text);
        if first_chunk || self.last_flush.elapsed() >= FLUSH_INTERVAL {
            self.flush();
        }
    }

    fn flush(&mut self) {
        if self.pending.is_empty() { return; }
        let text = std::mem::take(&mut self.pending);
        self.emit("item.delta", json!({"itemId":self.id,"delta":{"text":text}}));
        self.last_flush = Instant::now();
    }

    pub fn flush_deadline(&self) -> Option<tokio::time::Instant> {
        if self.pending.is_empty() || self.unavailable { None }
        else { Some(tokio::time::Instant::from_std(self.last_flush + FLUSH_INTERVAL)) }
    }

    pub fn flush_pending(&mut self) {
        if self.last_flush.elapsed() >= FLUSH_INTERVAL { self.flush(); }
    }

    pub fn finish(&mut self) { self.close("completed"); }

    fn close(&mut self, status: &str) {
        if self.closed { return; }
        self.closed = true;
        if self.started_at.is_none() { return; }
        self.flush();
        let duration = self.first.zip(self.last).map(|(first, last)| last.duration_since(first).as_secs_f64() * 1000.0);
        self.emit("item.completed", json!({"itemId":self.id,"status":status,"endedAt":crate::store::now(),"durationMs":duration}));
    }
}

impl Drop for ReasoningItem<'_> {
    fn drop(&mut self) { self.close("interrupted"); }
}

/// Records the real approval wait. It contains no authority to decide the
/// request; the existing approval registry remains the sole decision path.
pub struct ApprovalItem<'a> {
    state: &'a AppState,
    channel: &'a Channel<ChatEvent>,
    conversation_id: &'a str,
    run_id: &'a str,
    step_id: &'a str,
    message_id: &'a str,
    id: String,
    started: Instant,
    available: bool,
    closed: bool,
}

impl<'a> ApprovalItem<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(state: &'a AppState, channel: &'a Channel<ChatEvent>, conversation_id: &'a str,
        run_id: &'a str, step_id: &'a str, message_id: &'a str, round: usize, call_id: &str, name: &str) -> Self {
        let id = uuid::Uuid::new_v4().to_string();
        let available = emit_item(state, channel, conversation_id, run_id, step_id, message_id, "item.started",
            json!({"item":{"id":id,"runId":run_id,"round":round,"kind":"approval","status":"awaiting",
                "startedAt":crate::store::now(),"payload":{"callId":call_id,"stepId":step_id,"name":name}}}));
        Self { state, channel, conversation_id, run_id, step_id, message_id, id,
            started: Instant::now(), available, closed: false }
    }
    pub fn finish(&mut self, decision: &str) {
        if self.closed { return; }
        if self.available {
            self.available = emit_item(self.state, self.channel, self.conversation_id, self.run_id,
                self.step_id, self.message_id, "item.updated", json!({"itemId":self.id,"payload":{"decision":decision}}));
        }
        self.close(if matches!(decision, "approved" | "denied") { "completed" } else { "interrupted" });
    }
    fn close(&mut self, status: &str) {
        if self.closed { return; }
        self.closed = true;
        if self.available {
            let _ = emit_item(self.state, self.channel, self.conversation_id, self.run_id, self.step_id,
                self.message_id, "item.completed", json!({"itemId":self.id,"status":status,"endedAt":crate::store::now(),
                    "durationMs":self.started.elapsed().as_secs_f64() * 1000.0}));
        }
    }
}

impl Drop for ApprovalItem<'_> {
    fn drop(&mut self) { self.close("interrupted"); }
}

struct ToolDraft {
    id: String,
    last: crate::tool_calls::ToolCallDraft,
    last_flush: Instant,
    started: Instant,
    dirty: bool,
}

/// A tool draft only describes what the model is still writing. Completion of
/// this item is not an authorization, dispatch, or successful tool result.
pub struct ToolDraftItems<'a> {
    state: &'a AppState,
    channel: &'a Channel<ChatEvent>,
    conversation_id: &'a str,
    run_id: &'a str,
    step_id: &'a str,
    message_id: &'a str,
    round: usize,
    drafts: std::collections::BTreeMap<u64, ToolDraft>,
    unavailable: bool,
    closed: bool,
}

impl<'a> ToolDraftItems<'a> {
    pub fn new(state: &'a AppState, channel: &'a Channel<ChatEvent>, conversation_id: &'a str, run_id: &'a str, step_id: &'a str, message_id: &'a str, round: usize) -> Self {
        Self { state, channel, conversation_id, run_id, step_id, message_id, round, drafts: Default::default(), unavailable: false, closed: false }
    }
    fn payload(draft: &crate::tool_calls::ToolCallDraft) -> serde_json::Value {
        let mut arguments = serde_json::Map::new();
        if let Some(path) = &draft.path { arguments.insert("path".into(), json!(path)); }
        if let Some(command) = &draft.command { arguments.insert("command".into(), json!(command)); }
        json!({"phase":"preparing","name":draft.name,"callId":draft.id,"arguments":arguments})
    }
    fn emit(&mut self, kind: &str, payload: serde_json::Value) {
        if !self.unavailable { self.unavailable = !emit_item(self.state, self.channel, self.conversation_id, self.run_id, self.step_id, self.message_id, kind, payload); }
    }
    pub fn observe(&mut self, calls: &crate::tool_calls::ToolCalls) {
        if self.unavailable || self.closed { return; }
        for draft in calls.drafts() {
            if let Some(existing) = self.drafts.get_mut(&draft.index) {
                if existing.last != draft { existing.last = draft; existing.dirty = true; }
            } else {
                let id = uuid::Uuid::new_v4().to_string();
                let now = Instant::now();
                let payload = Self::payload(&draft);
                self.emit("item.started", json!({"item":{"id":id,"runId":self.run_id,"round":self.round,"kind":"tool","status":"running","startedAt":crate::store::now(),"payload":payload}}));
                self.drafts.insert(draft.index, ToolDraft { id, last: draft, last_flush: now, started: now, dirty: false });
            }
        }
        self.flush(false);
    }
    fn flush(&mut self, force: bool) {
        let mut updates = Vec::new();
        for draft in self.drafts.values_mut() {
            if draft.dirty && (force || draft.last_flush.elapsed() >= FLUSH_INTERVAL) {
                updates.push(json!({"itemId":draft.id,"payload":Self::payload(&draft.last)}));
                draft.dirty = false; draft.last_flush = Instant::now();
            }
        }
        for update in updates { self.emit("item.updated", update); }
    }
    pub fn flush_deadline(&self) -> Option<tokio::time::Instant> {
        if self.unavailable { return None; }
        self.drafts.values().filter(|draft| draft.dirty).map(|draft| tokio::time::Instant::from_std(draft.last_flush + FLUSH_INTERVAL)).min()
    }
    pub fn flush_pending(&mut self) { self.flush(false); }
    pub fn finish(&mut self, prepared: bool) { self.close(if prepared { "completed" } else { "interrupted" }); }
    fn close(&mut self, status: &str) {
        if self.closed { return; }
        self.closed = true;
        self.flush(true);
        let completed: Vec<_> = self.drafts.values().map(|draft| json!({"itemId":draft.id,"status":status,"endedAt":crate::store::now(),"durationMs":draft.started.elapsed().as_secs_f64() * 1000.0})).collect();
        for payload in completed { self.emit("item.completed", payload); }
    }
}

impl Drop for ToolDraftItems<'_> {
    fn drop(&mut self) { self.close("interrupted"); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn approval_items_record_decisions_and_interrupted_waits_without_authority() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        let chat = state.database().unwrap().create_conversation().unwrap().id;
        state.database().unwrap().save_run(&crate::agent_run::RunRecord { id: "run".into(), conversation_id: chat.clone(),
            status: crate::agent_run::RunState::Generating, model_provider: None, model_id: None, checkpoint: None, error: None, created_at: 1, updated_at: 1 }).unwrap();
        let live: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = live.clone();
        let channel = Channel::<ChatEvent>::new(move |body| {
            if let tauri::ipc::InvokeResponseBody::Json(text) = body {
                let value: serde_json::Value = serde_json::from_str(&text)?;
                if let Some(item) = value.get("item") { sink.lock().unwrap().push(serde_json::from_value(item.clone())?); }
            }
            Ok(())
        });
        for decision in ["approved", "denied", "expired"] {
            let mut item = ApprovalItem::new(&state, &channel, &chat, "run", "step", "message", 0, decision, "workspace_edit_file");
            item.finish(decision);
            item.finish("approved");
        }
        drop(ApprovalItem::new(&state, &channel, &chat, "run", "step", "message", 0, "cancelled", "workspace_edit_file"));
        let saved = state.database().unwrap().session_events(&chat, None, None).unwrap();
        assert_eq!(*live.lock().unwrap(), saved);
        assert_eq!(saved.iter().filter(|event| event.event_type == "item.started" && event.payload["item"]["status"] == "awaiting").count(), 4);
        assert_eq!(saved.iter().filter(|event| event.event_type == "item.updated").count(), 3);
        let completions: Vec<_> = saved.iter().filter(|event| event.event_type == "item.completed").collect();
        assert_eq!(completions.iter().map(|event| event.payload["status"].as_str().unwrap()).collect::<Vec<_>>(), vec!["completed", "completed", "interrupted", "interrupted"]);
        assert!(!saved.iter().any(|event| event.payload.to_string().contains("arguments")));
    }

    #[test]
    fn live_events_match_committed_replay_and_drop_marks_interruption() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        let chat = state.database().unwrap().create_conversation().unwrap().id;
        state.database().unwrap().save_run(&crate::agent_run::RunRecord { id: "run".into(), conversation_id: chat.clone(),
            status: crate::agent_run::RunState::Generating, model_provider: None, model_id: None, checkpoint: None, error: None, created_at: 1, updated_at: 1 }).unwrap();
        let live: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = live.clone();
        let channel = Channel::<ChatEvent>::new(move |body| {
            if let tauri::ipc::InvokeResponseBody::Json(text) = body {
                let value: serde_json::Value = serde_json::from_str(&text)?;
                if let Some(item) = value.get("item") { sink.lock().unwrap().push(serde_json::from_value(item.clone())?); }
            }
            Ok(())
        });
        {
            let mut item = ReasoningItem::new(&state, &channel, &chat, "run", "step-1", "message", 0);
            item.push("");
            item.push("First ");
            item.push("thought");
            item.finish();
        }
        {
            let mut item = ReasoningItem::new(&state, &channel, &chat, "run", "step-2", "message", 1);
            item.push("Interrupted thought");
        }
        {
            let mut item = ReasoningItem::new(&state, &channel, &chat, "run", "step-empty", "message", 2);
            item.push("");
            item.finish();
        }
        let saved = state.database().unwrap().session_events(&chat, None, None).unwrap();
        assert_eq!(*live.lock().unwrap(), saved);
        assert_eq!(saved.iter().filter(|event| event.event_type == "item.started").count(), 2);
        let deltas: String = saved.iter().filter(|event| event.event_type == "item.delta")
            .filter_map(|event| event.payload["delta"]["text"].as_str()).collect();
        assert_eq!(deltas, "First thoughtInterrupted thought");
        let completions: Vec<_> = saved.iter().filter(|event| event.event_type == "item.completed").collect();
        assert_eq!(completions[0].payload["status"], "completed");
        assert_eq!(completions[1].payload["status"], "interrupted");
        assert!(completions.iter().all(|event| event.payload["durationMs"].is_number()));
    }

    #[test]
    fn partial_parallel_tool_drafts_replay_without_claiming_execution() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        let chat = state.database().unwrap().create_conversation().unwrap().id;
        state.database().unwrap().save_run(&crate::agent_run::RunRecord { id: "run".into(), conversation_id: chat.clone(),
            status: crate::agent_run::RunState::Generating, model_provider: None, model_id: None, checkpoint: None, error: None, created_at: 1, updated_at: 1 }).unwrap();
        let live: Arc<Mutex<Vec<SessionEvent>>> = Arc::new(Mutex::new(Vec::new()));
        let sink = live.clone();
        let channel = Channel::<ChatEvent>::new(move |body| {
            if let tauri::ipc::InvokeResponseBody::Json(text) = body {
                let value: serde_json::Value = serde_json::from_str(&text)?;
                if let Some(item) = value.get("item") { sink.lock().unwrap().push(serde_json::from_value(item.clone())?); }
            }
            Ok(())
        });
        let mut calls = crate::tool_calls::ToolCalls::default();
        {
            let mut items = ToolDraftItems::new(&state, &channel, &chat, "run", "step", "message", 0);
            calls.push(&json!({"tool_calls":[
                {"index":0,"id":"a","function":{"name":"workspace_edit_file","arguments":"{\"path\":\"src/"}},
                {"index":1,"id":"b","function":{"name":"execute_command","arguments":"{\"command\":\"echo hi\",\"other\":"}}
            ]})).unwrap();
            items.observe(&calls);
            calls.push(&json!({"tool_calls":[{"index":0,"function":{"arguments":"app.tsx\",\"content\":\"unfinished"}}]})).unwrap();
            items.observe(&calls);
            assert!(calls.validated().is_err());
            // Dropping an incomplete model stream must not claim dispatch.
        }
        let saved = state.database().unwrap().session_events(&chat, None, None).unwrap();
        assert_eq!(*live.lock().unwrap(), saved);
        assert_eq!(saved.iter().filter(|event| event.event_type == "item.started").count(), 2);
        assert!(saved.iter().any(|event| event.event_type == "item.updated" && event.payload["payload"]["arguments"]["path"] == "src/app.tsx"));
        assert!(saved.iter().any(|event| event.event_type == "item.started" && event.payload["item"]["payload"]["arguments"]["command"] == "echo hi"));
        assert_eq!(saved.iter().filter(|event| event.event_type == "item.completed" && event.payload["status"] == "interrupted").count(), 2);
    }

    #[test]
    fn a_prepared_draft_finishes_once_without_claiming_a_tool_result() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        let chat = state.database().unwrap().create_conversation().unwrap().id;
        state.database().unwrap().save_run(&crate::agent_run::RunRecord { id: "run".into(), conversation_id: chat.clone(),
            status: crate::agent_run::RunState::Generating, model_provider: None, model_id: None, checkpoint: None, error: None, created_at: 1, updated_at: 1 }).unwrap();
        let channel = Channel::<ChatEvent>::new(|_| Ok(()));
        let mut calls = crate::tool_calls::ToolCalls::default();
        calls.push(&json!({"tool_calls":[{"index":0,"id":"a","function":{"name":"workspace_edit_file","arguments":"{\"path\":\"app.ts\"}"}}]})).unwrap();
        assert!(calls.validated().is_ok());
        {
            let mut items = ToolDraftItems::new(&state, &channel, &chat, "run", "step", "message", 0);
            items.observe(&calls);
            items.finish(true);
        }
        let saved = state.database().unwrap().session_events(&chat, None, None).unwrap();
        let completed: Vec<_> = saved.iter().filter(|event| event.event_type == "item.completed").collect();
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].payload["status"], "completed");
        assert!(!saved.iter().any(|event| event.event_type == "tool_call" || event.event_type == "tool_result"));
    }
}
