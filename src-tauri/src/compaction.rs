//! Context compaction and checkpointing.
//!
//! Compaction replaces the model-visible prefix of a conversation with a
//! checkpoint notice: older completed turns are copied verbatim into an
//! artifact (readable back through `artifact_read`) and a cutoff marks where
//! the replayed history starts. Message rows are never deleted; the SQLite
//! transcript, audits, and session events stay complete.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const AUTO_KEY_PREFIX: &str = "compaction_auto:";
const KEEP_LAST_KEY: &str = "compaction_keep_last";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Checkpoint {
    pub conversation_id: String,
    pub cutoff: i64,
    pub artifact_id: String,
    pub created_at: i64,
}

/// Structured research state carried across compaction. Verbatim archives
/// keep the full transcript; this summary keeps requirements, findings, and
/// source references queryable without replaying the whole artifact.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResearchStateSummary {
    pub requirements: Vec<String>,
    pub findings: Vec<String>,
    pub source_refs: Vec<String>,
    pub pending_work: Vec<String>,
    pub updated_at: i64,
}

pub fn summarize_research_state(
    requirements: &[String],
    findings: &[String],
    source_refs: &[String],
    pending_work: &[String],
) -> ResearchStateSummary {
    ResearchStateSummary {
        requirements: requirements.iter().take(20).cloned().collect(),
        findings: findings.iter().take(50).cloned().collect(),
        source_refs: source_refs.iter().take(50).cloned().collect(),
        pending_work: pending_work.iter().take(20).cloned().collect(),
        updated_at: crate::store::now(),
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionOutcome {
    pub checkpoint: Checkpoint,
    pub dropped_messages: usize,
    pub kept_messages: usize,
    pub note: String,
}

/// Enabled by default; an explicit per-conversation preference is preserved.
pub fn auto_enabled(store: &crate::store::Store, conversation_id: &str) -> bool {
    store
        .setting::<Option<bool>>(&format!("{AUTO_KEY_PREFIX}{conversation_id}"))
        .unwrap_or(None).unwrap_or(true)
}

pub const AUTO_THRESHOLD_PERCENT: u64 = 80;

/// Global capability switch. Corrupt or missing capability settings fail open
/// in the same way as the rest of the capability registry.
pub fn capability_enabled(store: &crate::store::Store) -> bool {
    crate::capabilities::is_enabled(store, "compaction").unwrap_or(true)
}

pub fn should_compact(input: u64, reserve: u32, capacity: u32) -> bool {
    capacity > 0 && input.saturating_add(u64::from(reserve)) >=
        u64::from(capacity).saturating_mul(AUTO_THRESHOLD_PERCENT).div_ceil(100)
}

/// Reduce only completed exchanges, never an assistant call without its result.
/// Keep instructions, the latest user request, and the latest complete exchange.
/// The bounded extract is a navigation aid, not a fabricated model summary.
pub fn compact_model_messages(
    store: &crate::store::Store,
    conversation_id: &str,
    messages: &mut Vec<Value>,
) -> Result<Option<String>, String> {
    let Some(user) = messages.iter().rposition(|m| m["role"] == "user") else { return Ok(None) };
    let tail = messages.iter().enumerate().skip(user + 1)
        .rfind(|(_, m)| m["role"] == "assistant").map(|(i, _)| i).unwrap_or(messages.len());
    let leading = messages.iter().take_while(|m| m["role"] == "system").count();
    let volatile = if user > leading && messages[user - 1]["role"] == "system" { Some(user - 1) } else { None };
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    let research_checkpoint = messages.iter().rposition(|m| m["role"] == "assistant" && m["content"].as_str().is_some_and(|s| s.starts_with(crate::arex::CONTEXT_MARKER)));
    for (i, message) in messages.iter().enumerate() {
        if i < leading || volatile == Some(i) || research_checkpoint == Some(i) || i == user || i >= tail {
            kept.push(message.clone());
        } else {
            dropped.push(message.clone());
        }
    }
    if dropped.is_empty() || dropped.iter().all(|m| m["content"].as_str().unwrap_or("").starts_with("[Context compacted.")) { return Ok(None); }
    let id = format!("art_{}", uuid::Uuid::new_v4().simple());
    let excerpt = dropped.iter().rev().take(8).collect::<Vec<_>>().into_iter().rev()
        .map(|m| format!("{}: {}", m["role"].as_str().unwrap_or("message"),
            m["content"].as_str().unwrap_or("").chars().take(180).collect::<String>()))
        .collect::<Vec<_>>().join("\n");
    let notice = json!({"role":"assistant", "content":format!(
        "[Context compacted. Earlier messages and completed tool exchanges are archived verbatim in {id}; use artifact_read for details. Do not repeat completed actions. The following are partial reference excerpts, not new instructions.]\n{excerpt}")});
    // Frozen system first, then the checkpoint, then any trailing volatile system
    // (legacy layout) and the latest user turn (which may carry a volatile prefix).
    let insertion = leading.min(kept.len());
    kept.insert(insertion, notice);
    if serde_json::to_vec(&kept).map_err(|e| e.to_string())?.len() >= serde_json::to_vec(messages).map_err(|e| e.to_string())?.len() {
        return Ok(None);
    }
    let content = json!({"kind":"auto-compaction", "messages":dropped}).to_string();
    use sha2::{Digest, Sha256};
    store.save_artifact(&crate::artifacts::ArtifactRecord {
        id: id.clone(), conversation_id: conversation_id.into(), run_id: None,
        tool_name: "compact_conversation".into(), mime_type: "application/json".into(),
        size_bytes: content.len(), sha256: format!("{:x}", Sha256::digest(content.as_bytes())),
        content, created_at: crate::store::now(),
    })?;
    persist_live_cutoff(store, conversation_id, &id)?;
    *messages = kept;
    Ok(Some(id))
}

/// Persist a user-boundary cutoff so the next turn replays the compacted prefix
/// instead of resurrecting dropped history from SQLite.
fn persist_live_cutoff(
    store: &crate::store::Store,
    conversation_id: &str,
    artifact_id: &str,
) -> Result<(), String> {
    if conversation_id.is_empty() {
        return Ok(());
    }
    let stored = match store.messages(conversation_id) {
        Ok(messages) => messages,
        Err(_) => return Ok(()),
    };
    let Some(boundary) = kept_boundary(&stored, 1) else {
        return Ok(());
    };
    let cutoff = stored[boundary - 1].created_at;
    if stored[boundary].created_at <= cutoff {
        let mut next_ts = cutoff + 1;
        for msg in &stored[boundary..] {
            if msg.created_at < next_ts {
                store.update_message_created_at(&msg.id, next_ts)?;
                next_ts += 1;
            } else {
                next_ts = msg.created_at + 1;
            }
        }
    }
    if let Some(existing) = store.compaction(conversation_id)? {
        if cutoff <= existing.cutoff {
            return Ok(());
        }
    }
    store.save_compaction(&Checkpoint {
        conversation_id: conversation_id.to_string(),
        cutoff,
        artifact_id: artifact_id.to_string(),
        created_at: crate::store::now(),
    })
}

pub fn set_auto(store: &crate::store::Store, conversation_id: &str, enabled: bool) -> Result<(), String> {
    store.save_setting(&format!("{AUTO_KEY_PREFIX}{conversation_id}"), &enabled)
}

pub fn keep_last(store: &crate::store::Store) -> usize {
    let val = store.setting::<u32>(KEEP_LAST_KEY).unwrap_or(0);
    if val == 0 { 10 } else { val.clamp(1, 200) as usize }
}

pub fn set_keep_last(store: &crate::store::Store, keep: usize) -> Result<(), String> {
    store.save_setting(KEEP_LAST_KEY, &(keep.clamp(1, 200) as u32))
}

/// Boundary index of the first kept message: the start of the turn that is
/// `keep` turns from the end. A boundary may only fall on a user message so a
/// tool audit is never separated from the turn that requested it.
fn kept_boundary(messages: &[crate::store::Message], keep: usize) -> Option<usize> {
    let turn_starts: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| message.role == "user")
        .map(|(index, _)| index)
        .collect();
    if turn_starts.len() <= keep {
        return None;
    }
    Some(turn_starts[turn_starts.len() - keep])
}

pub fn compact_now(
    store: &crate::store::Store,
    conversation_id: &str,
    keep: usize,
) -> Result<Checkpoint, String> {
    let keep = keep.clamp(1, 200);
    let messages = store.messages(conversation_id)?;
    let boundary = kept_boundary(&messages, keep).ok_or_else(|| {
        format!(
            "Nothing to compact: this conversation has at most {keep} complete turn(s) inside the kept window. Send more messages or lower the kept-turn window."
        )
    })?;
    let cutoff = messages[boundary - 1].created_at;
    // Ensure all kept messages have timestamps strictly after cutoff so the boundary
    // user message is never swallowed into the dropped prefix.
    if messages[boundary].created_at <= cutoff {
        let mut next_ts = cutoff + 1;
        for msg in &messages[boundary..] {
            if msg.created_at < next_ts {
                store.update_message_created_at(&msg.id, next_ts)?;
                next_ts += 1;
            } else {
                next_ts = msg.created_at + 1;
            }
        }
    }
    if let Some(existing) = store.compaction(conversation_id)? {
        if cutoff <= existing.cutoff {
            return Err("Nothing to compact: nothing new is left beyond the window.".into());
        }
    }
    let checkpoint = Checkpoint {
        conversation_id: conversation_id.to_string(),
        cutoff,
        artifact_id: String::new(),
        created_at: crate::store::now(),
    };
    let artifact = checkpoint_artifact(&checkpoint, &messages[..boundary])?;
    let mut checkpoint = checkpoint;
    checkpoint.artifact_id = artifact.id.clone();
    store.save_artifact(&artifact)?;
    store.save_compaction(&checkpoint)?;
    Ok(checkpoint)
}

fn checkpoint_artifact(
    checkpoint: &Checkpoint,
    dropped: &[crate::store::Message],
) -> Result<crate::artifacts::ArtifactRecord, String> {
    let messages: Vec<Value> = dropped
        .iter()
        .map(|message| {
            json!({
                "role": message.role,
                "content": message.content,
                "status": message.status,
                "createdAt": message.created_at,
            })
        })
        .collect();
    let content = json!({
        "kind": "compaction-checkpoint",
        "conversationId": checkpoint.conversation_id,
        "cutoff": checkpoint.cutoff,
        "messageCount": dropped.len(),
        "messages": messages,
    })
    .to_string();
    let id = format!("art_{}", uuid::Uuid::new_v4().simple());
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let sha256 = format!("{:x}", hasher.finalize());
    Ok(crate::artifacts::ArtifactRecord {
        id: id.clone(),
        conversation_id: checkpoint.conversation_id.clone(),
        run_id: None,
        tool_name: "compact_conversation".into(),
        mime_type: "application/json".into(),
        size_bytes: content.len(),
        sha256,
        content,
        created_at: checkpoint.created_at,
    })
}

pub fn outcome(
    store: &crate::store::Store,
    conversation_id: &str,
    keep: usize,
) -> Result<CompactionOutcome, String> {
    let checkpoint = compact_now(store, conversation_id, keep)?;
    let messages = store.messages(conversation_id)?;
    let dropped = messages.iter().filter(|message| message.created_at <= checkpoint.cutoff).count();
    let kept = messages.len() - dropped;
    let note = format!(
        "Compacted {dropped} older message(s) into artifact {}. Recent turns stay in context; the full transcript, audits, and session events remain in the local database. If a needed fact is missing, read the artifact back with artifact_read.",
        checkpoint.artifact_id
    );
    Ok(CompactionOutcome { checkpoint, dropped_messages: dropped, kept_messages: kept, note })
}

#[tauri::command]
pub async fn compact_conversation_cmd(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
) -> Result<CompactionOutcome, String> {
    let keep = {
        let store = state.database()?;
        keep_last(&store)
    };
    let store = state.database()?;
    if !capability_enabled(&store) {
        return Err("Context compaction is disabled in Tools settings. Re-enable the compaction capability before creating a checkpoint.".into());
    }
    let outcome = outcome(&store, &conversation_id, keep)?;
    if let Ok(store) = state.database() {
        crate::chat::emit_session_event(
            &store,
            &conversation_id,
            None,
            None,
            None,
            "compaction",
            json!({"artifact": outcome.checkpoint.artifact_id, "cutoff": outcome.checkpoint.cutoff, "auto": false}),
            false,
        );
    }
    Ok(outcome)
}

#[tauri::command]
pub async fn compaction_status(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
) -> Result<serde_json::Value, String> {
    let store = state.database()?;
    Ok(serde_json::json!({
        "auto": capability_enabled(&store) && auto_enabled(&store, &conversation_id),
        "keepLast": keep_last(&store),
        "checkpoint": store.compaction(&conversation_id)?,
    }))
}

#[tauri::command]
pub async fn set_compaction_auto(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
    enabled: bool,
) -> Result<(), String> {
    let store = state.database()?;
    if !capability_enabled(&store) {
        return Err("Context compaction is disabled in Tools settings. Re-enable the compaction capability first.".into());
    }
    set_auto(&store, &conversation_id, enabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Message;

    #[test]
    fn eighty_percent_includes_response_headroom_and_handles_boundaries() {
        assert!(!should_compact(699, 100, 1000));
        assert!(should_compact(700, 100, 1000));
        assert!(should_compact(u64::MAX, 100, 1000));
        assert!(!should_compact(100, 100, 0));
    }

    #[test]
    fn live_compaction_archives_old_exchanges_without_repeating_or_splitting_tools() {
        let store = crate::store::Store::open_memory().unwrap();
        let id = store.create_conversation().unwrap().id;
        let mut messages = vec![
            json!({"role":"system","content":"Keep instructions"}),
            json!({"role":"user","content":"Implement the feature"}),
            json!({"role":"assistant","tool_calls":[{"id":"old"}],"content":"Reading"}),
            json!({"role":"tool","tool_call_id":"old","content":"x".repeat(12000)}),
            json!({"role":"assistant","tool_calls":[{"id":"latest"}],"content":"Testing"}),
            json!({"role":"tool","tool_call_id":"latest","content":"tests passed"}),
        ];
        let artifact = compact_model_messages(&store, &id, &mut messages).unwrap().unwrap();
        assert_eq!(messages[0]["content"], "Keep instructions");
        assert!(messages.iter().any(|m| m["content"] == "Implement the feature"));
        assert_eq!(messages[messages.len()-2]["tool_calls"][0]["id"], "latest");
        assert_eq!(messages.last().unwrap()["tool_call_id"], "latest");
        assert!(store.artifact(&artifact).unwrap().unwrap().content.contains("old"));
        assert!(compact_model_messages(&store, &id, &mut messages).unwrap().is_none());
    }

    #[test]
    fn live_compaction_keeps_trailing_volatile_after_the_frozen_system() {
        let store = crate::store::Store::open_memory().unwrap();
        let id = store.create_conversation().unwrap().id;
        let mut messages = vec![
            json!({"role":"system","content":"frozen"}),
            json!({"role":"user","content":"old turn"}),
            json!({"role":"assistant","content":"x".repeat(4000)}),
            json!({"role":"system","content":"[injected plan]\nopen todos: x"}),
            json!({"role":"user","content":"draft"}),
        ];
        compact_model_messages(&store, &id, &mut messages).unwrap().unwrap();
        assert_eq!(messages[0]["content"], "frozen");
        assert!(messages[1]["content"].as_str().unwrap().starts_with("[Context compacted."));
        assert_eq!(messages[2]["content"], "[injected plan]\nopen todos: x");
        assert_eq!(messages.last().unwrap()["content"], "draft");
    }

    #[test]
    fn live_compaction_keeps_volatile_prefix_on_the_current_user_turn() {
        let store = crate::store::Store::open_memory().unwrap();
        let id = store.create_conversation().unwrap().id;
        let mut messages = vec![
            json!({"role":"system","content":"frozen"}),
            json!({"role":"user","content":"old turn"}),
            json!({"role":"assistant","content":"x".repeat(4000)}),
            json!({"role":"user","content":"[injected plan]\nopen todos: x\n\ndraft"}),
        ];
        compact_model_messages(&store, &id, &mut messages).unwrap().unwrap();
        assert_eq!(messages[0]["content"], "frozen");
        assert!(messages[1]["content"].as_str().unwrap().starts_with("[Context compacted."));
        assert_eq!(
            messages.last().unwrap()["content"],
            "[injected plan]\nopen todos: x\n\ndraft"
        );
        assert_eq!(
            messages.iter().filter(|message| message["role"] == "system").count(),
            1
        );
    }

    #[test]
    fn live_compaction_persists_a_cutoff_for_the_next_turn() {
        let (store, id) = store_with(&[
            ("user", "turn one", 1),
            ("assistant", "reply one", 2),
            ("user", "turn two", 3),
            ("assistant", "reply two", 4),
        ]);
        let mut messages = vec![
            json!({"role":"system","content":"frozen"}),
            json!({"role":"user","content":"turn one"}),
            json!({"role":"assistant","content":"x".repeat(4000)}),
            json!({"role":"user","content":"turn two"}),
            json!({"role":"assistant","content":"reply two"}),
            json!({"role":"user","content":"draft"}),
        ];
        let artifact = compact_model_messages(&store, &id, &mut messages).unwrap().unwrap();
        let checkpoint = store.compaction(&id).unwrap().unwrap();
        assert_eq!(checkpoint.artifact_id, artifact);
        assert_eq!(checkpoint.cutoff, 2);
        let replay = crate::history::model_history_with_cutoff(
            &store.messages(&id).unwrap(),
            Some((checkpoint.cutoff, checkpoint.artifact_id.as_str())),
        )
        .unwrap();
        let serialized = serde_json::to_string(&replay).unwrap();
        assert!(!serialized.contains("turn one"));
        assert!(serialized.contains("turn two"));
    }

    fn store_with(rows: &[(&str, &str, i64)]) -> (crate::store::Store, String) {
        let store = crate::store::Store::open_memory().unwrap();
        let id = store.create_conversation().unwrap().id;
        for (role, content, created_at) in rows {
            store.insert_message_at(&id, role, content, "complete", *created_at).unwrap();
        }
        (store, id)
    }

    fn row(role: &str, content: &str, created_at: i64) -> Message {
        Message {
            error: None,
            id: format!("m{created_at}-{role}"),
            conversation_id: "chat".into(),
            role: role.into(),
            content: content.into(),
            reasoning: String::new(),
            status: "complete".into(),
            created_at,
        }
    }

    #[test]
    fn compaction_keeps_recent_turns_and_cites_the_artifact() {
        let (store, id) = store_with(&[
            ("user", "fact-ALPHA-1", 1),
            ("assistant", "noted", 2),
            ("user", "turn two", 3),
            ("assistant", "done", 3),
            ("user", "turn three", 4),
            ("assistant", "all good", 5),
            ("user", "turn four", 6),
        ]);
        let checkpoint = compact_now(&store, &id, 2).unwrap();
        assert_eq!(checkpoint.cutoff, 3, "cutoff is the last dropped row's timestamp");
        // Replay keeps the checkpoint notice plus the last two complete turns.
        let previous = store.messages(&id).unwrap();
        let replay = crate::history::model_history_with_cutoff(
            &previous,
            Some((checkpoint.cutoff, checkpoint.artifact_id.as_str())),
        )
        .unwrap();
        assert!(serde_json::to_string(&replay).unwrap().contains(&checkpoint.artifact_id), "checkpoint notice cites the artifact");
        let serialized = serde_json::to_string(&replay).unwrap();
        assert!(!serialized.contains("turn two"), "dropped turn content no longer replays");
        assert!(serialized.contains("turn three"));
        // The dropped transcript stays queryable in the artifact.
        let artifact = store.artifact(&checkpoint.artifact_id).unwrap().unwrap();
        assert!(artifact.content.contains("fact-ALPHA-1"));
        assert!(artifact.content.contains("turn two"));
    }

    #[test]
    fn compaction_never_splits_a_tool_exchange_across_the_boundary() {
        let (store, id) = store_with(&[
            ("user", "read it", 1),
            ("assistant", "working", 2),
            ("tool", "audit", 3),
            ("user", "next question", 4),
        ]);
        let checkpoint = compact_now(&store, &id, 1).unwrap();
        assert_eq!(checkpoint.cutoff, 3, "the tool audit belongs to the dropped turn");
        let kept = store.messages(&id).unwrap();
        let replay =
            crate::history::model_history_with_cutoff(&kept, Some((checkpoint.cutoff, checkpoint.artifact_id.as_str()))).unwrap();
        assert!(replay.len() >= 2);
        assert_eq!(replay.last().unwrap()["content"], "next question");
    }

    #[test]
    fn compaction_refuses_when_the_window_already_fits() {
        let (store, id) = store_with(&[("user", "only turn", 1)]);
        let error = compact_now(&store, &id, 2).unwrap_err();
        assert!(error.contains("Nothing to compact"));
    }

    #[test]
    fn repeated_compaction_stays_recoverable_and_auto_flag_round_trips() {
        let (store, id) = store_with(&[
            ("user", "one", 1),
            ("assistant", "two", 2),
            ("user", "three", 3),
            ("assistant", "four", 4),
            ("user", "five", 5),
            ("assistant", "six", 6),
            ("user", "seven", 7),
            ("assistant", "eight", 8),
            ("user", "nine", 9),
            ("assistant", "ten", 10),
        ]);
        assert!(auto_enabled(&store, &id));
        set_auto(&store, &id, false).unwrap();
        assert!(!auto_enabled(&store, &id));
        set_auto(&store, &id, true).unwrap();
        assert!(auto_enabled(&store, &id));
        let first = compact_now(&store, &id, 4).unwrap();
        let second = compact_now(&store, &id, 2).unwrap();
        assert!(second.cutoff >= first.cutoff);
        let artifact = store.artifact(&second.artifact_id).unwrap().unwrap();
        assert!(artifact.content.contains("\"one\""), "the later checkpoint still spans the earliest messages");
        let outcome = outcome(&store, &id, 2).unwrap_err();
        assert!(outcome.contains("Nothing to compact"), "nothing new is left beyond the window");
        assert_eq!(keep_last(&store), 10);
        set_keep_last(&store, 300).unwrap();
        assert_eq!(keep_last(&store), 200, "the window clamps to a bounded range");
        assert_eq!(row("user", "x", 1).role, "user", "row helper stays exercised");
    }

    #[test]
    fn compaction_handles_real_millisecond_epoch_timestamps_beyond_u32_max() {
        let base_ts = 1_773_244_800_000_i64; // Well beyond u32::MAX (4_294_967_295)
        let (store, id) = store_with(&[
            ("user", "epoch turn 1", base_ts),
            ("assistant", "epoch reply 1", base_ts + 1000),
            ("user", "epoch turn 2", base_ts + 2000),
            ("assistant", "epoch reply 2", base_ts + 3000),
            ("user", "epoch turn 3", base_ts + 4000),
            ("assistant", "epoch reply 3", base_ts + 5000),
        ]);
        let checkpoint = compact_now(&store, &id, 1).unwrap();
        assert!(checkpoint.cutoff > u32::MAX as i64, "cutoff preserves full 64-bit epoch timestamp without clamping");
        assert_eq!(checkpoint.cutoff, base_ts + 3000);

        let messages = store.messages(&id).unwrap();
        let replay = crate::history::model_history_with_cutoff(
            &messages,
            Some((checkpoint.cutoff, checkpoint.artifact_id.as_str())),
        )
        .unwrap();

        let serialized = serde_json::to_string(&replay).unwrap();
        assert!(!serialized.contains("epoch turn 1"), "dropped turn 1 does not replay");
        assert!(!serialized.contains("epoch turn 2"), "dropped turn 2 does not replay");
        assert!(serialized.contains("epoch turn 3"), "kept turn 3 remains in replay");

        let outcome_res = outcome(&store, &id, 1);
        // Note: outcome calls compact_now again, which returns 'nothing new' since cutoff hasn't changed.
        assert!(outcome_res.is_err());
    }

    #[test]
    fn compaction_preserves_turn_boundary_even_with_identical_timestamps() {
        let ts = 500_i64;
        let (store, id) = store_with(&[
            ("user", "turn one", ts),
            ("assistant", "reply one", ts),
            ("user", "turn two", ts),
            ("assistant", "reply two", ts),
        ]);
        let checkpoint = compact_now(&store, &id, 1).unwrap();
        assert_eq!(checkpoint.cutoff, ts);

        let messages = store.messages(&id).unwrap();
        let replay = crate::history::model_history_with_cutoff(
            &messages,
            Some((checkpoint.cutoff, checkpoint.artifact_id.as_str())),
        )
        .unwrap();

        let serialized = serde_json::to_string(&replay).unwrap();
        assert!(!serialized.contains("turn one"));
        assert!(serialized.contains("turn two"), "kept turn two is preserved even with identical timestamps");
    }

    #[test]
    fn repeated_compaction_preserves_constraints_findings_and_pending_work() {
        let summary = summarize_research_state(
            &["Compare A and B benchmarks".to_string()],
            &["A scores 90 [S1]".to_string()],
            &["S1 https://example.com/a".to_string()],
            &["verify B benchmarks".to_string()],
        );
        let text = serde_json::to_string(&summary).unwrap();
        // Round-trip through storage: constraints, findings, refs survive.
        let restored: ResearchStateSummary = serde_json::from_str(&text).unwrap();
        assert_eq!(restored.requirements, vec!["Compare A and B benchmarks".to_string()]);
        assert_eq!(restored.findings, vec!["A scores 90 [S1]".to_string()]);
        assert_eq!(restored.source_refs, vec!["S1 https://example.com/a".to_string()]);
        assert_eq!(restored.pending_work, vec!["verify B benchmarks".to_string()]);
        // Bounds hold for large states.
        let big = summarize_research_state(
            &(0..100).map(|i| format!("req {i}")).collect::<Vec<_>>(),
            &(0..200).map(|i| format!("finding {i}")).collect::<Vec<_>>(),
            &(0..200).map(|i| format!("S{i}")).collect::<Vec<_>>(),
            &(0..100).map(|i| format!("work {i}")).collect::<Vec<_>>(),
        );
        assert_eq!(big.requirements.len(), 20);
        assert_eq!(big.findings.len(), 50);
        assert_eq!(big.source_refs.len(), 50);
        assert_eq!(big.pending_work.len(), 20);
    }
}
