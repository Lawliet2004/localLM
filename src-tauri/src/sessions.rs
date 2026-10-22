//! Session management, search hits, and conversation event types (Phase 1).

use serde::{Deserialize, Serialize};

/// A lightweight invalidation, not a second copy of tool/model payloads.
/// Readers fetch committed journal rows through the existing cursor command.
static ACTIVITY_TARGET: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

pub fn initialize_activity_events(app: tauri::AppHandle) {
    let _ = ACTIVITY_TARGET.set(app);
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActivityNotification<'a> {
    conversation_id: &'a str,
    seq: u64,
}

pub fn notify_activity(conversation_id: &str, seq: u64) {
    use tauri::Emitter;
    if let Some(app) = ACTIVITY_TARGET.get() {
        let _ = app.emit(
            "locallm://session-activity",
            ActivityNotification {
                conversation_id,
                seq,
            },
        );
    }
}

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
    let (new_conversation, copied_events_count) =
        store.fork_session_events(&conversation_id, from_seq)?;
    crate::workspace_ui::inherit_task_meta(&store, &conversation_id, &new_conversation.id)?;
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

#[cfg(test)]
mod activity_notification_tests {
    #[test]
    fn invalidation_contains_only_committed_cursor_and_conversation() {
        let value = serde_json::to_value(super::ActivityNotification {
            conversation_id: "chat",
            seq: 42,
        })
        .unwrap();
        assert_eq!(value, serde_json::json!({"conversationId":"chat","seq":42}));
        // Store-only/test environments have no app handle and remain supported.
        super::notify_activity("chat", 42);
    }
}
