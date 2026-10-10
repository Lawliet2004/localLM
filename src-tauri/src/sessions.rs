//! Session management, search hits, and conversation event types (Phase 1).

use serde::{Deserialize, Serialize};

/// Append-only session log format version (`docs/session-format-status.md`).
/// `Store::open` refuses databases stamped with a newer version.
pub const SESSION_FORMAT_VERSION: u32 = 1;

/// Every `event_type` producers commit to `session_events`. Unknown types may
/// only be written with `ignorable: true`; a non-ignorable unknown type fails
/// on append and on read so a typo or a newer format never passes silently.
pub const KNOWN_EVENT_TYPES: &[&str] = &[
    "system_prompt",
    "context_injection",
    "user_msg",
    "assistant_msg",
    "model_response",
    "reasoning",
    "turn_start",
    "step_start",
    "tool_call",
    "tool_result",
    "approval_requested",
    "approval_decided",
    "step_end",
    "turn_end",
    "compaction",
    "plan",
    "plan_created",
    "verification",
    "finalization_retry",
    "research_answer_fallback",
    "subagent_schedule",
];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub conversation_id: String,
    pub conversation_title: String,
    pub message_id: String,
    pub role: String,
    pub excerpt: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionEvent {
    pub id: String,
    pub conversation_id: String,
    pub run_id: Option<String>,
    pub seq: u64,
    pub step_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub ignorable: bool,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkSessionResult {
    pub new_conversation: crate::store::Conversation,
    pub copied_events_count: u64,
}

/// Reconstruct only content committed by the selected log prefix. Message
/// ordinals are used solely for legacy events without ids or full snapshots.
pub fn fork_messages(events: &[SessionEvent], source: &[crate::store::Message]) -> Vec<crate::store::Message> {
    use crate::store::Message;
    use std::collections::{HashMap, HashSet};
    let users: Vec<_> = source.iter().filter(|m| m.role == "user").collect();
    let assistants: Vec<_> = source.iter().filter(|m| m.role == "assistant").collect();
    let tool_audits: HashMap<_, _> = source.iter().filter(|message| message.role == "tool").filter_map(|message| {
        let audit: serde_json::Value = serde_json::from_str(&message.content).ok()?;
        let call_id = audit.pointer("/request/callId")?.as_str()?.to_string();
        Some((call_id, message))
    }).collect();
    let mut output: Vec<Message> = Vec::new();
    let mut turn = 0usize;
    let mut assistant: Option<usize> = None;
    let mut responses = HashSet::new();
    let mut calls = HashMap::new();
    for event in events {
        let payload = &event.payload;
        if event.event_type == "user_msg" {
            let original = payload["messageId"].as_str().and_then(|id| users.iter().find(|m| m.id == id).copied())
                .or_else(|| users.get(turn).copied());
            let content = payload["content"].as_str().or_else(|| original.map(|m| m.content.as_str())).unwrap_or_default();
            output.push(Message { id: String::new(), conversation_id: String::new(), role: "user".into(), content: content.into(), reasoning: String::new(), status: "complete".into(), created_at: event.created_at, error: None, metrics: None });
            turn += 1;
            assistant = None;
            continue;
        }
        if event.event_type == "tool_call" {
            if let Some(id) = event.tool_call_id.as_deref().or_else(|| payload["callId"].as_str()) {
                calls.insert(id.to_string(), payload.clone());
            }
            continue;
        }
        if event.event_type == "tool_result" {
            let id = event.tool_call_id.as_deref().or_else(|| payload["callId"].as_str());
            let original = id.and_then(|id| tool_audits.get(id));
            let content = if let Some(original) = original {
                original.content.clone()
            } else if id.is_none() {
                payload["result"].as_str().unwrap_or_default().to_string()
            } else {
                let call = id.and_then(|id| calls.get(id));
                serde_json::json!({"request":{"connector":"Harness","name":payload["name"],"arguments":call.map(|c| &c["arguments"]).unwrap_or(&serde_json::Value::Null),"callId":id,"stepId":event.step_id,"assistantContent":""},"result":payload["result"]}).to_string()
            };
            output.push(Message { id: String::new(), conversation_id: String::new(), role: "tool".into(), content, reasoning: String::new(), status: "complete".into(), created_at: event.created_at, error: None, metrics: None });
            continue;
        }
        let key = (turn, event.step_id.clone());
        let snapshot = payload.get("snapshot");
        let original = payload["messageId"].as_str().and_then(|id| assistants.iter().find(|m| m.id == id).copied())
            .or_else(|| turn.checked_sub(1).and_then(|i| assistants.get(i).copied()));
        let text = match event.event_type.as_str() {
            "model_response" => {
                responses.insert(key.clone());
                payload["content"].as_str()
            }
            "assistant_msg" => payload["content"].as_str().or_else(|| original.map(|m| m.content.as_str())),
            "step_end" if !responses.contains(&key) => payload["content"].as_str(),
            "reasoning" => payload["text"].as_str(),
            "plan_created" => payload["content"].as_str(),
            "turn_end" => snapshot.and_then(|s| s["content"].as_str()).or_else(|| original.map(|m| m.content.as_str())),
            _ => None,
        };
        if text.is_none() && snapshot.is_none() { continue; }
        let index = *assistant.get_or_insert_with(|| {
            output.push(Message { id: String::new(), conversation_id: String::new(), role: "assistant".into(), content: String::new(), reasoning: String::new(), status: "interrupted".into(), created_at: event.created_at, error: None, metrics: None });
            output.len() - 1
        });
        let message = &mut output[index];
        if let Some(snapshot) = snapshot {
            message.content = snapshot["content"].as_str().unwrap_or_default().into();
            message.reasoning = snapshot["reasoning"].as_str().unwrap_or_default().into();
            message.status = snapshot["status"].as_str().unwrap_or("interrupted").into();
        } else if event.event_type == "turn_end" {
            if let Some(original) = original {
                message.content = original.content.clone();
                message.reasoning = original.reasoning.clone();
                message.status = original.status.clone();
                message.error = original.error.clone();
                message.metrics = original.metrics.clone();
            }
        } else if event.event_type == "reasoning" {
            message.reasoning.push_str(text.unwrap_or_default());
        } else {
            message.content.push_str(text.unwrap_or_default());
            if event.event_type == "plan_created" { message.content.push_str("\n\n---\n\n"); }
            if event.event_type == "assistant_msg" || payload["final"] == true { message.status = "complete".into(); }
        }
    }
    output
}

#[tauri::command]
pub async fn get_session_events(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
    from_seq: Option<u64>,
    limit: Option<u32>,
) -> Result<Vec<SessionEvent>, String> {
    let store = state.database()?;
    store.session_events(&conversation_id, from_seq, limit)
}

#[tauri::command]
pub async fn fork_session(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
    from_seq: u64,
) -> Result<ForkSessionResult, String> {
    let store = state.database()?;
    let (new_conversation, copied_events_count) = store.fork_session_events(&conversation_id, from_seq)?;
    Ok(ForkSessionResult {
        new_conversation,
        copied_events_count,
    })
}

#[tauri::command]
pub async fn replay_session(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
) -> Result<Vec<SessionEvent>, String> {
    let store = state.database()?;
    store.session_events(&conversation_id, Some(0), Some(10000))
}

#[tauri::command]
pub async fn search_sessions(
    state: tauri::State<'_, crate::AppState>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<Hit>, String> {
    let store = state.database()?;
    store.search_sessions(&query, limit.unwrap_or(50))
}
