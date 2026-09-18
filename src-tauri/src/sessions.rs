//! Session management, search hits, and conversation event types (Phase 1).

use serde::{Deserialize, Serialize};

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
