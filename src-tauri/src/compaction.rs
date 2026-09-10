//! Context compaction: explicit checkpoints, never silent (Phase 8, context slice).
//!
//! Compaction stores the compacted message prefix as an artifact checkpoint
//! and records a cutoff. History building drops the prefix and inserts a
//! short checkpoint notice that cites the artifact id, so anything the model
//! sees stays reconstructable from the durable log. Auto-compaction is OFF by
//! default; when ON it only triggers on an explicit preflight rejection path
//! and always appends a `compaction` run event first.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Checkpoint {
    pub conversation_id: String,
    pub cutoff: i64,
    pub artifact_id: String,
    pub created_at: i64,
}

pub const AUTO_KEY: &str = "compaction.auto";
pub const KEEP_LAST_KEY: &str = "compaction.keep_last";

pub fn auto_enabled(store: &crate::store::Store) -> bool {
    store.setting(AUTO_KEY).unwrap_or(false)
}

pub fn keep_last(store: &crate::store::Store) -> usize {
    let keep: usize = store.setting(KEEP_LAST_KEY).unwrap_or(20);
    keep.clamp(4, 200)
}

/// Split messages into (dropped prefix, kept suffix) at the cutoff timestamp.
pub fn split_at(messages: &[crate::store::Message], cutoff: i64) -> (Vec<&crate::store::Message>, Vec<&crate::store::Message>) {
    let mut prefix = Vec::new();
    let mut kept = Vec::new();
    for message in messages {
        if message.created_at <= cutoff {
            prefix.push(message);
        } else {
            kept.push(message);
        }
    }
    (prefix, kept)
}

pub fn checkpoint_notice(checkpoint: &Checkpoint, dropped: usize) -> String {
    format!(
        "Context checkpoint: the first {dropped} messages were compacted into artifact {} to fit context. \
         Audits and full history remain in the local database; ask the user before expanding it back.",
        checkpoint.artifact_id
    )
}

/// Compact now: keep the newest `keep` messages, checkpoint the rest.
/// Returns the checkpoint. Fails loud when there is nothing worth dropping.
pub fn compact_now(store: &crate::store::Store, conversation_id: &str, keep: usize) -> Result<Checkpoint, String> {
    let messages = store.messages(conversation_id)?;
    if messages.len() <= keep + 2 {
        return Err("Nothing to compact: the conversation is already short.".into());
    }
    let cutoff = messages[messages.len() - keep - 1].created_at;
    let prefix: Vec<&crate::store::Message> = messages.iter().filter(|message| message.created_at <= cutoff).collect();
    let dump = serde_json::to_string(&prefix.iter().map(|message| {
        serde_json::json!({"role": message.role, "content": message.content, "reasoning": message.reasoning, "status": message.status, "createdAt": message.created_at})
    }).collect::<Vec<_>>())
    .map_err(|error| error.to_string())?;
    let digest = format!("{:x}", Sha256::digest(dump.as_bytes()));
    let artifact = crate::artifacts::ArtifactRecord {
        id: format!("compact-{}", uuid::Uuid::new_v4()),
        conversation_id: conversation_id.to_string(),
        run_id: None,
        tool_name: "compact_conversation".into(),
        mime_type: "application/json".into(),
        size_bytes: dump.len(),
        sha256: digest,
        content: dump,
        created_at: crate::store::now(),
    };
    store.save_artifact(&artifact)?;
    let checkpoint = Checkpoint {
        conversation_id: conversation_id.to_string(),
        cutoff,
        artifact_id: artifact.id,
        created_at: crate::store::now(),
    };
    store.save_compaction(&checkpoint)?;
    Ok(checkpoint)
}

#[tauri::command]
pub async fn compact_conversation_cmd(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
    keep_last_count: Option<usize>,
) -> Result<Checkpoint, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before compacting.")?;
    let store = state.database()?;
    let keep = keep_last_count.unwrap_or_else(|| keep_last(&store)).clamp(4, 200);
    compact_now(&store, &conversation_id, keep)
}

#[tauri::command]
pub fn compaction_status(state: tauri::State<'_, crate::AppState>, conversation_id: String) -> Result<serde_json::Value, String> {
    let store = state.database()?;
    Ok(serde_json::json!({
        "checkpoint": store.compaction(&conversation_id)?,
        "auto": auto_enabled(&store),
        "keepLast": keep_last(&store),
    }))
}

#[tauri::command]
pub async fn set_compaction_auto(
    state: tauri::State<'_, crate::AppState>,
    auto: bool,
    keep_last_count: Option<usize>,
) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before changing compaction.")?;
    let store = state.database()?;
    store.save_setting(AUTO_KEY, &auto)?;
    if let Some(keep) = keep_last_count {
        store.save_setting(KEEP_LAST_KEY, &keep.clamp(4, 200))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn split_notices_and_bounds() {
        let notice = checkpoint_notice(
            &Checkpoint { conversation_id: "c".into(), cutoff: 5, artifact_id: "a1".into(), created_at: 0 },
            12,
        );
        assert!(notice.contains("a1") && notice.contains("12"));
    }
}
