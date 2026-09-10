//! Session log operations: fork, replay transcript, search (Phase 1 remainder).
//!
//! Fork copies message history into a new conversation with fresh ids and
//! preserves lineage via a `fork` run event on the child. Replay re-derives
//! the model-visible transcript (documented non-deterministic on re-run).

use serde::{Deserialize, Serialize};

/// Authority version for the session log shape (messages + runs +
/// run_events + subagent_runs). Bump on any model-visible schema change;
/// migrations stay monotonic and additive. See docs/session-format-status.md.
pub const SESSION_FORMAT_VERSION: u32 = 1;

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

#[tauri::command]
pub async fn fork_session(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
    through_message_id: String,
) -> Result<crate::store::Conversation, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before forking.")?;
    let store = state.database()?;
    let forked = store.fork_conversation(&conversation_id, &through_message_id)?;
    // Lineage: a terminal run record carrying the fork event (model-visible).
    let now = crate::store::now();
    let run = crate::agent_run::RunRecord {
        id: format!("run-{}", uuid::Uuid::new_v4()),
        conversation_id: forked.id.clone(),
        status: crate::agent_run::RunState::Completed,
        model_provider: None,
        model_id: None,
        checkpoint: Some("fork".into()),
        error: None,
        created_at: now,
        updated_at: now,
    };
    store.save_run(&run)?;
    store.append_run_event(&crate::agent_run::RunEvent {
        run_id: run.id,
        seq: 0,
        step_id: "fork".into(),
        tool_call_id: None,
        event_type: "fork".into(),
        payload: serde_json::json!({"fromConversation": conversation_id, "throughMessage": through_message_id}),
        created_at: now,
    })?;
    Ok(forked)
}

/// Re-derive the exact model-visible transcript for a conversation: system
/// prompt + skill guidance + compacted history prefix + messages. Read-only.
#[tauri::command]
pub fn replay_session(state: tauri::State<'_, crate::AppState>, conversation_id: String) -> Result<serde_json::Value, String> {
    let store = state.database()?;
    let preferences = store.preferences()?;
    let messages = store.messages(&conversation_id)?;
    let history = crate::history::model_history(&messages)?;
    let checkpoint = store.compaction(&conversation_id)?;
    Ok(serde_json::json!({
        "systemPrompt": preferences.system_prompt,
        "messageCount": messages.len(),
        "historyLength": history.len(),
        "checkpoint": checkpoint,
        "history": history,
        "note": "Re-derivation is deterministic over stored rows; a live re-run is not (model sampling).",
    }))
}

#[tauri::command]
pub fn search_sessions(state: tauri::State<'_, crate::AppState>, query: String, limit: Option<usize>) -> Result<Vec<Hit>, String> {
    state.database()?.search_sessions(&query, limit.unwrap_or(30))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hit_shape_is_stable() {
        let hit = Hit {
            conversation_id: "c".into(), conversation_title: "t".into(), message_id: "m".into(),
            role: "user".into(), excerpt: "hi".into(), created_at: 0,
        };
        assert_eq!(serde_json::to_value(&hit).unwrap()["messageId"], "m");
    }
}
