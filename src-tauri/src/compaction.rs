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

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskCheckpoint {
    pub version: u8,
    pub original_request: String,
    pub constraints: Vec<String>,
    pub completed_operation_ids: Vec<String>,
    pub findings: Vec<String>,
    pub source_refs: Vec<String>,
    pub unresolved: Vec<String>,
    pub next_action: Option<String>,
    pub archive_ids: Vec<String>,
    #[serde(default)]
    pub attachment_refs: Vec<String>,
}

/// Constraints are sentences the user stated. Completed ids come from tool
/// audits (`tool_call_id`), not from model claims.
pub fn checkpoint_from_audits(
    original_request: &str,
    constraints: &[String],
    completed_operation_ids: &[String],
    findings: &[String],
    source_refs: &[String],
    unresolved: &[String],
    archive_ids: &[String],
) -> TaskCheckpoint {
    TaskCheckpoint {
        version: 1,
        original_request: original_request.to_string(),
        constraints: constraints.iter().take(20).cloned().collect(),
        completed_operation_ids: completed_operation_ids.to_vec(),
        findings: findings.iter().take(50).cloned().collect(),
        source_refs: source_refs.iter().take(50).cloned().collect(),
        unresolved: unresolved.to_vec(),
        next_action: unresolved.first().cloned(),
        archive_ids: archive_ids.to_vec(),
        attachment_refs: Vec::new(),
    }
}

const HANDOFF_MARKER: &str = "\nTask handoff (recorded context):\n";
pub(crate) const HANDOFF_FIELD: &str = "_locallm_handoff";

/// Only internally constructed messages carry checkpoint metadata. Text that
/// happens to quote the display marker is never interpreted as saved state.
fn handoff_message(checkpoint: &TaskCheckpoint) -> Value {
    json!({"role":"assistant", "content":handoff_notice(checkpoint), HANDOFF_FIELD:checkpoint})
}

fn recorded_handoff(message: &Value) -> Option<TaskCheckpoint> {
    if message["role"] != "assistant" { return None; }
    serde_json::from_value(message.get(HANDOFF_FIELD)?.clone()).ok()
}

fn message_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts.iter().filter(|part| {
            matches!(part["type"].as_str(), Some("text" | "input_text" | "output_text"))
        }).filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    }
}
fn bounded(text: &str, chars: usize) -> String { text.chars().take(chars).collect() }
fn push_unique(values: &mut Vec<String>, value: String, limit: usize) {
    if !value.trim().is_empty() && !values.contains(&value) {
        values.push(value);
        if values.len() > limit { values.remove(0); }
    }
}
/// Deterministic, bounded handoff from real user text and observed outcomes.
/// Assistant prose remains explicitly unverified; it never certifies a tool.
pub fn task_checkpoint(messages: &[Value], archive_id: &str) -> TaskCheckpoint {
    let mut checkpoint = checkpoint_from_audits("", &[], &[], &[], &[], &[], &[]);
    let mut calls = std::collections::HashMap::new();
    for (message_index, message) in messages.iter().enumerate() {
        if let Some(round) = message["tool_calls"].as_array() {
            for call in round {
                if let Some(id) = call["id"].as_str() {
                    let arguments = call["function"]["arguments"].as_str().and_then(|text| serde_json::from_str::<Value>(text).ok()).unwrap_or(Value::Null);
                    calls.insert(id.to_string(), (call["function"]["name"].as_str().unwrap_or("tool").to_string(), arguments));
                }
            }
        }
        if let Some(previous) = recorded_handoff(message) {
            checkpoint = previous;
            continue;
        }
        let text = message_text(&message["content"]);
        let text = text.as_str();
        if message["role"] == "user" && message["_continuation"] != true {
            if checkpoint.original_request.is_empty() { checkpoint.original_request = bounded(text, 1800); }
            for constraint in explicit_constraints(text) {
                push_unique(&mut checkpoint.constraints, bounded(&constraint, 600), 16);
            }
            for (part_index, part) in message["content"].as_array().into_iter().flatten().enumerate() {
                let kind = part["type"].as_str().unwrap_or("attachment");
                if !matches!(kind, "text" | "input_text" | "output_text") {
                    push_unique(&mut checkpoint.attachment_refs,
                        format!("{kind}: {archive_id}, message {message_index}, content part {part_index}; consult the original archived request for attachment details."), 8);
                }
            }
        } else if message["role"] == "tool" {
            let record: Value = serde_json::from_str(text).unwrap_or(Value::Null);
            let result = record.get("result").unwrap_or(&record);
            let id = message["tool_call_id"].as_str().or_else(|| record["request"]["callId"].as_str());
            if let Some(id) = id {
                let context = calls.remove(id);
                let name = record["request"]["name"].as_str().or_else(|| context.as_ref().map(|(name, _)| name.as_str())).unwrap_or("tool");
                let args = record["request"].get("arguments").or_else(|| context.as_ref().map(|(_, args)| args));
                let paths = args.map(|args| ["path", "source", "destination"].iter().filter_map(|key| args[*key].as_str()).map(|path| bounded(path, 160)).collect::<Vec<_>>().join(", ")).unwrap_or_default();
                let data = if crate::tool_envelope::is_envelope(result) { &result["data"] } else { result };
                let verdict = data.pointer("/report/verdict").or_else(|| data.pointer("/check/report/verdict")).and_then(Value::as_str)
                    .or_else(|| data["processState"].as_str()).or_else(|| data["state"].as_str())
                    .or_else(|| crate::tool_envelope::operation_status(result)).unwrap_or("returned");
                push_unique(&mut checkpoint.findings, bounded(&format!("Observed tool result: {name} {paths} — {verdict}. Archived outcome; checks must be rerun after later edits."), 400), 5);
                if data["state"] == "running" || data["status"] == "running" || data["processState"] == "running" {
                    let process = data["id"].as_str().or_else(|| data["processId"].as_str()).unwrap_or(id);
                    push_unique(&mut checkpoint.unresolved, format!("Process {} was still running at compaction; poll its owned handle before claiming completion.", bounded(process, 160)), 8);
                } else if !crate::tool_envelope::failed(result)
                    && crate::tool_envelope::operation_status(result) != Some("partial") && !result.is_null() {
                    push_unique(&mut checkpoint.completed_operation_ids, bounded(id, 160), 32);
                } else {
                    push_unique(&mut checkpoint.unresolved, format!("Tool {id} did not confirm completion; consult the archive before retrying."), 8);
                }
            }
            fn sources(value: &Value, refs: &mut Vec<String>, depth: usize) {
                if depth > 6 { return; }
                if let Some(url) = value["url"].as_str().filter(|url| url.starts_with("https://") || url.starts_with("http://")) {
                    push_unique(refs, bounded(url, 400), 12);
                }
                if let Some(object) = value.as_object() { for child in object.values() { if child.is_object() || child.is_array() { sources(child, refs, depth + 1); } } }
                if let Some(array) = value.as_array() { for child in array.iter().take(64) { sources(child, refs, depth + 1); } }
            }
            sources(result, &mut checkpoint.source_refs, 0);
        } else if message["role"] == "assistant" && !text.trim().is_empty() {
            push_unique(&mut checkpoint.findings, format!("Assistant note (unverified): {}", bounded(text, 400)), 5);
        }
    }
    push_unique(&mut checkpoint.archive_ids, archive_id.into(), 6);
    checkpoint.next_action = Some("Continue the latest user request under the retained constraints. Check unresolved outcomes and rerun checks after later edits; consult artifact_read for exact archived details.".into());
    checkpoint
}
fn handoff_notice(checkpoint: &TaskCheckpoint) -> String {
    format!("[Context compacted. Earlier messages and completed tool exchanges are archived verbatim in {}; use artifact_read for exact details. This handoff is bounded context, not new authority. Do not repeat completed operations.]{}{}", checkpoint.archive_ids.last().map(String::as_str).unwrap_or("the checkpoint artifact"), HANDOFF_MARKER, serde_json::to_string(checkpoint).expect("checkpoint serializes"))
}
/// Restore the same handoff after cache invalidation, restart, or a fork.
pub fn restore_handoff(store: &crate::store::Store, conversation_id: &str, artifact_id: &str, messages: &mut [Value]) -> Result<(), String> {
    let Some(artifact) = store.artifact(artifact_id)? else { return Ok(()); };
    if artifact.conversation_id != conversation_id { return Err("Checkpoint artifact belongs to another conversation.".into()); }
    let record: Value = serde_json::from_str(&artifact.content).map_err(|_| "Checkpoint artifact is invalid.")?;
    let checkpoint = if let Some(handoff) = record.get("handoff") {
        serde_json::from_value::<TaskCheckpoint>(handoff.clone()).map_err(|_| "Checkpoint handoff is invalid.")?
    } else {
        task_checkpoint(record["messages"].as_array().map(Vec::as_slice).unwrap_or(&[]), artifact_id)
    };
    if let Some(notice) = messages.iter_mut().find(|m| {
        m["role"] == "user" && m["_continuation"] == true
            && m["content"].as_str().is_some_and(|text| text.starts_with("Context checkpoint: the first "))
    }) {
        *notice = handoff_message(&checkpoint);
    }
    Ok(())
}

pub fn explicit_constraints(request: &str) -> Vec<String> {
    request
        .lines()
        .flat_map(|line| line.split(". "))
        .map(|sentence| sentence.trim().trim_end_matches('.'))
        .filter(|sentence| {
            let lower = sentence.to_ascii_lowercase();
            lower.contains("must")
                || lower.contains("do not")
                || lower.contains("don't")
                || lower.contains("never")
                || lower.contains("only ")
        })
        .map(str::to_string)
        .collect()
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
        .unwrap_or(None)
        .unwrap_or(true)
}

pub const AUTO_THRESHOLD_PERCENT: u64 = 80;

/// Global capability switch. Corrupt capability settings fail closed: an
/// unreadable kill switch disables compaction rather than enabling it.
pub fn capability_enabled(store: &crate::store::Store) -> bool {
    crate::capabilities::is_enabled(store, "compaction").unwrap_or(false)
}

pub fn should_compact(input: u64, reserve: u32, capacity: u32) -> bool {
    capacity > 0
        && input.saturating_add(u64::from(reserve))
            >= u64::from(capacity)
                .saturating_mul(AUTO_THRESHOLD_PERCENT)
                .div_ceil(100)
}

/// A planned compaction: the kept buffer (with the checkpoint notice already
/// inserted) and the messages that were dropped.
pub struct CompactionPlan {
    /// The artifact id the checkpoint notice points at.
    pub id: String,
    /// The messages that stay in the model-visible buffer.
    pub kept: Vec<Value>,
    /// Removed messages and, for the first active exchange, its request.
    /// Handoff attachment indices are relative to this exact array.
    pub archived: Vec<Value>,
}

/// Plan a compaction of the model-visible buffer.
///
/// Returns the kept and dropped message vectors for a retention window, or
/// `None` when there is nothing to drop.
///
/// `keep` counts **turns** in the history region — a turn being a user
/// message and everything that follows it up to the next user message. The
/// window is cut on a user boundary, so an assistant message and its tool
/// results are never split apart.
///
/// The current user turn and its entire tool exchange are always kept.
pub fn plan_compaction(messages: &[Value], keep: usize) -> Option<CompactionPlan> {
    let is_user = |m: &Value| m["role"] == "user" && m["_continuation"] != true;
    let user = messages.iter().rposition(is_user)?;
    let leading = messages
        .iter()
        .take_while(|m| m["role"] == "system")
        .count();
    let volatile = if user > leading && messages[user - 1]["role"] == "system" {
        Some(user - 1)
    } else {
        None
    };
    // Turn starts inside the **history** region only — strictly before the
    // current draft, which is always kept.
    let history_turn_starts: Vec<usize> = messages
        .iter()
        .enumerate()
        .take(user)
        .filter(|(_, m)| is_user(m))
        .map(|(i, _)| i)
        .collect();
    if history_turn_starts.len() <= keep {
        return plan_active_exchange(messages, user, keep.min(2));
    }
    let retention_start = if keep == 0 {
        user
    } else {
        history_turn_starts[history_turn_starts.len() - keep]
    };
    let mut kept = Vec::new();
    let mut dropped = Vec::new();
    let research_checkpoint = messages.iter().rposition(|m| {
        m["role"] == "assistant"
            && m["content"]
                .as_str()
                .is_some_and(|s| s.starts_with(crate::arex::CONTEXT_MARKER))
    });
    for (i, message) in messages.iter().enumerate() {
        if i < leading
            || volatile == Some(i)
            || research_checkpoint == Some(i)
            || i >= retention_start
        {
            kept.push(message.clone());
        } else {
            dropped.push(message.clone());
        }
    }
    if dropped.is_empty()
        || dropped.iter().all(|m| {
            m["content"]
                .as_str()
                .unwrap_or("")
                .starts_with("[Context compacted.")
        })
    {
        return None;
    }
    let id = format!("art_{}", uuid::Uuid::new_v4().simple());
    let checkpoint = task_checkpoint(&dropped, &id);
    let notice = handoff_message(&checkpoint);
    // Frozen system first, then the checkpoint, then any trailing volatile system
    // (legacy layout) and the latest user turn (which may carry a volatile prefix).
    let insertion = leading.min(kept.len());
    kept.insert(insertion, notice);
    Some(CompactionPlan { id, kept, archived: dropped })
}

/// Compact only whole settled tool rounds in a long current turn. The user
/// request and any unfinished exchange remain verbatim. Retention can shrink
/// with the caller's budget windows when even two recent rounds will not fit.
fn plan_active_exchange(messages: &[Value], user: usize, retain: usize) -> Option<CompactionPlan> {
    let starts = messages.iter().enumerate().skip(user + 1)
        .filter(|(_, m)| m["role"] == "assistant" && m["tool_calls"].as_array().is_some_and(|calls| !calls.is_empty()))
        .map(|(index, _)| index).collect::<Vec<_>>();
    let settled = starts.iter().copied().enumerate().take_while(|(round, start)| {
        let end = starts.get(round + 1).copied().unwrap_or(messages.len());
        let mut pending = std::collections::HashSet::new();
        let valid = messages[*start]["tool_calls"].as_array().is_some_and(|calls| calls.iter().all(|call| {
            call["id"].as_str().is_some_and(|id| !id.is_empty() && pending.insert(id))
        }));
        valid && messages[*start + 1..end].iter().filter(|m| m["role"] == "tool")
            .all(|m| m["tool_call_id"].as_str().is_some_and(|id| pending.remove(id)))
            && pending.is_empty()
    }).count();
    let drop_rounds = settled.saturating_sub(retain);
    if drop_rounds == 0 { return None; }
    let cut = starts.get(drop_rounds).copied().unwrap_or(messages.len());
    let research_checkpoint = messages.iter().rposition(|message| message["role"] == "assistant"
        && message["content"].as_str().is_some_and(|text| text.starts_with(crate::arex::CONTEXT_MARKER)));
    let mut archived = messages[user + 1..cut].iter().enumerate()
        .filter(|(index, _)| Some(user + 1 + index) != research_checkpoint).map(|(_, message)| message.clone()).collect::<Vec<_>>();
    let id = format!("art_{}", uuid::Uuid::new_v4().simple());
    if !archived.iter().any(|message| recorded_handoff(message).is_some()) {
        // Include the request once so multipart references resolve in the
        // archive. Later handoffs already reference that first copy.
        archived.insert(0, messages[user].clone());
    }
    let checkpoint = task_checkpoint(&archived, &id);
    let mut kept = messages[..user + 1].to_vec();
    kept.push(handoff_message(&checkpoint));
    if let Some(index) = research_checkpoint.filter(|index| *index > user && *index < cut) { kept.push(messages[index].clone()); }
    kept.extend_from_slice(&messages[cut..]);
    if serde_json::to_vec(&kept).ok()?.len() >= serde_json::to_vec(messages).ok()?.len() { return None; }
    Some(CompactionPlan { id, kept, archived })
}

/// Publish exactly the plan that was counted and accepted.
pub fn commit_plan(
    store: &crate::store::Store,
    conversation_id: &str,
    messages: &mut Vec<Value>,
    plan: CompactionPlan,
    keep: usize,
    draft_saved: bool,
) -> Result<String, String> {
    let CompactionPlan { id, kept, archived } = plan;
    let transaction = store.connection.unchecked_transaction().map_err(|e| e.to_string())?;
    let handoff = kept.iter().filter_map(recorded_handoff)
        .find(|handoff| handoff.archive_ids.last() == Some(&id))
        .unwrap_or_else(|| task_checkpoint(&archived, &id));
    let content = json!({"kind":"auto-compaction", "messages":archived, "handoff":handoff}).to_string();
    use sha2::{Digest, Sha256};
    store.save_artifact(&crate::artifacts::ArtifactRecord {
        id: id.clone(),
        conversation_id: conversation_id.into(),
        run_id: None,
        tool_name: "compact_conversation".into(),
        mime_type: "application/json".into(),
        size_bytes: content.len(),
        sha256: format!("{:x}", Sha256::digest(content.as_bytes())),
        content,
        created_at: crate::store::now(),
    })?;
    persist_live_cutoff(store, conversation_id, &id, keep, draft_saved)?;
    transaction.commit().map_err(|e| e.to_string())?;
    *messages = kept;
    Ok(id)
}

/// Persist a user-boundary cutoff so the next turn replays the compacted prefix
/// instead of resurrecting dropped history from SQLite.
///
/// The window is the same one the model just saw, so the next turn replays
/// the turns that stayed in context instead of silently narrowing to one.
fn persist_live_cutoff(
    store: &crate::store::Store,
    conversation_id: &str,
    artifact_id: &str,
    keep: usize,
    draft_saved: bool,
) -> Result<(), String> {
    if conversation_id.is_empty() {
        return Ok(());
    }
    let stored = store.messages(conversation_id)?;
    let retained = keep + usize::from(draft_saved);
    let boundary = if retained == 0 {
        stored.len()
    } else {
        let Some(boundary) = kept_boundary(&stored, retained) else {
            return Ok(());
        };
        boundary
    };
    let Some(last_archived) = boundary.checked_sub(1).and_then(|i| stored.get(i)) else {
        return Ok(());
    };
    let cutoff = last_archived.created_at;
    if stored.get(boundary).is_some_and(|m| m.created_at <= cutoff) {
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

pub fn set_auto(
    store: &crate::store::Store,
    conversation_id: &str,
    enabled: bool,
) -> Result<(), String> {
    store.save_setting(&format!("{AUTO_KEY_PREFIX}{conversation_id}"), &enabled)
}

pub fn keep_last(store: &crate::store::Store) -> usize {
    let val = store.setting::<u32>(KEEP_LAST_KEY).unwrap_or(0);
    if val == 0 {
        10
    } else {
        val.clamp(1, 200) as usize
    }
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
    let tx = store.connection.unchecked_transaction().map_err(|e| e.to_string())?;
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
    tx.commit().map_err(|e| e.to_string())?;
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
                "content": if message.role == "user" {
                    crate::attachments::model_content(&message.content).unwrap_or_else(|_| json!(message.content))
                } else { json!(message.content) },
                "status": message.status,
                "createdAt": message.created_at,
            })
        })
        .collect();
    let id = format!("art_{}", uuid::Uuid::new_v4().simple());
    let content = json!({
        "kind": "compaction-checkpoint",
        "conversationId": checkpoint.conversation_id,
        "cutoff": checkpoint.cutoff,
        "messageCount": dropped.len(),
        "handoff": task_checkpoint(&messages, &id),
        "messages": messages,
    })
    .to_string();
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
    let dropped = messages
        .iter()
        .filter(|message| message.created_at <= checkpoint.cutoff)
        .count();
    let kept = messages.len() - dropped;
    let note = format!(
        "Compacted {dropped} older message(s) into artifact {}. Recent turns stay in context; the full transcript, audits, and session events remain in the local database. If a needed fact is missing, read the artifact back with artifact_read.",
        checkpoint.artifact_id
    );
    Ok(CompactionOutcome {
        checkpoint,
        dropped_messages: dropped,
        kept_messages: kept,
        note,
    })
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
    fn checkpoint_keeps_the_request_constraints_and_audited_operations() {
        let request =
            "Compare the two reports. You must cite a page. Do not repeat a completed write.";
        let constraints = explicit_constraints(request);
        assert!(constraints.iter().any(|line| line.contains("must cite")));
        assert!(constraints
            .iter()
            .any(|line| line.to_lowercase().contains("do not")));
        let checkpoint = checkpoint_from_audits(
            request,
            &constraints,
            &["call-1".into()],
            &["The value is 19.99 on page 2".into()],
            &["doc-1:page:2".into()],
            &["Second source was not opened".into()],
            &["art_1".into()],
        );
        assert_eq!(checkpoint.version, 1);
        assert_eq!(checkpoint.original_request, request);
        assert_eq!(
            checkpoint.completed_operation_ids,
            vec!["call-1".to_string()]
        );
        assert_eq!(
            checkpoint.unresolved,
            vec!["Second source was not opened".to_string()]
        );
        assert_eq!(
            checkpoint.next_action.as_deref(),
            Some("Second source was not opened")
        );
        let again = checkpoint_from_audits(
            request,
            &constraints,
            &["call-1".into()],
            &[],
            &[],
            &checkpoint.unresolved,
            &["art_1".into()],
        );
        assert_eq!(
            again.completed_operation_ids,
            checkpoint.completed_operation_ids
        );
    }

    #[test]
    fn compaction_preserves_the_retained_window_and_complete_active_exchange() {
        let messages = vec![
            json!({"role":"system","content":"Keep instructions"}),
            json!({"role":"user","content":"Historical requirement"}),
            json!({"role":"assistant","tool_calls":[{"id":"old"}]}),
            json!({"role":"tool","tool_call_id":"old","content":"x".repeat(12000)}),
            json!({"role":"user","content":"Current request"}),
            json!({"role":"assistant","tool_calls":[{"id":"latest"}]}),
            json!({"role":"tool","tool_call_id":"latest","content":"tests passed"}),
            json!({"role":"user","content":"Continue","_continuation":true}),
        ];
        assert!(plan_compaction(&messages, 10).is_none());
        assert!(plan_compaction(&messages, 1).is_none());
        assert!(plan_compaction(&messages[4..], 0).is_none());
        let plan = plan_compaction(&messages, 0).unwrap();
        assert_eq!(&plan.archived, &messages[1..4]);
        assert_eq!(&plan.kept[2..], &messages[4..]);
    }

    fn compact_model_messages(
        store: &crate::store::Store,
        id: &str,
        messages: &mut Vec<Value>,
        keep: usize,
    ) -> Result<Option<String>, String> {
        plan_compaction(messages, keep)
            .map(|plan| commit_plan(store, id, messages, plan, keep, false))
            .transpose()
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
        compact_model_messages(&store, &id, &mut messages, 0)
            .unwrap()
            .unwrap();
        assert_eq!(messages[0]["content"], "frozen");
        assert!(messages[1]["content"]
            .as_str()
            .unwrap()
            .starts_with("[Context compacted."));
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
        compact_model_messages(&store, &id, &mut messages, 0)
            .unwrap()
            .unwrap();
        assert_eq!(messages[0]["content"], "frozen");
        assert!(messages[1]["content"]
            .as_str()
            .unwrap()
            .starts_with("[Context compacted."));
        assert_eq!(
            messages.last().unwrap()["content"],
            "[injected plan]\nopen todos: x\n\ndraft"
        );
        assert_eq!(
            messages
                .iter()
                .filter(|message| message["role"] == "system")
                .count(),
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
        // Keep one history turn: the older turn and its reply are dropped.
        let artifact = compact_model_messages(&store, &id, &mut messages, 1)
            .unwrap()
            .unwrap();
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
            store
                .insert_message_at(&id, role, content, "complete", *created_at)
                .unwrap();
        }
        (store, id)
    }

    fn row(role: &str, content: &str, created_at: i64) -> Message {
        Message {
            metrics: None,
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
        assert_eq!(
            checkpoint.cutoff, 3,
            "cutoff is the last dropped row's timestamp"
        );
        // Replay keeps the checkpoint notice plus the last two complete turns.
        let previous = store.messages(&id).unwrap();
        let replay = crate::history::model_history_with_cutoff(
            &previous,
            Some((checkpoint.cutoff, checkpoint.artifact_id.as_str())),
        )
        .unwrap();
        assert!(
            serde_json::to_string(&replay)
                .unwrap()
                .contains(&checkpoint.artifact_id),
            "checkpoint notice cites the artifact"
        );
        let serialized = serde_json::to_string(&replay).unwrap();
        assert!(
            !serialized.contains("turn two"),
            "dropped turn content no longer replays"
        );
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
        assert_eq!(
            checkpoint.cutoff, 3,
            "the tool audit belongs to the dropped turn"
        );
        let kept = store.messages(&id).unwrap();
        let replay = crate::history::model_history_with_cutoff(
            &kept,
            Some((checkpoint.cutoff, checkpoint.artifact_id.as_str())),
        )
        .unwrap();
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
        assert!(
            artifact.content.contains("\"one\""),
            "the later checkpoint still spans the earliest messages"
        );
        let outcome = outcome(&store, &id, 2).unwrap_err();
        assert!(
            outcome.contains("Nothing to compact"),
            "nothing new is left beyond the window"
        );
        assert_eq!(keep_last(&store), 10);
        set_keep_last(&store, 300).unwrap();
        assert_eq!(
            keep_last(&store),
            200,
            "the window clamps to a bounded range"
        );
        assert_eq!(
            row("user", "x", 1).role,
            "user",
            "row helper stays exercised"
        );
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
        assert!(
            checkpoint.cutoff > u32::MAX as i64,
            "cutoff preserves full 64-bit epoch timestamp without clamping"
        );
        assert_eq!(checkpoint.cutoff, base_ts + 3000);

        let messages = store.messages(&id).unwrap();
        let replay = crate::history::model_history_with_cutoff(
            &messages,
            Some((checkpoint.cutoff, checkpoint.artifact_id.as_str())),
        )
        .unwrap();

        let serialized = serde_json::to_string(&replay).unwrap();
        assert!(
            !serialized.contains("epoch turn 1"),
            "dropped turn 1 does not replay"
        );
        assert!(
            !serialized.contains("epoch turn 2"),
            "dropped turn 2 does not replay"
        );
        assert!(
            serialized.contains("epoch turn 3"),
            "kept turn 3 remains in replay"
        );

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
        assert!(
            serialized.contains("turn two"),
            "kept turn two is preserved even with identical timestamps"
        );
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
        assert_eq!(
            restored.requirements,
            vec!["Compare A and B benchmarks".to_string()]
        );
        assert_eq!(restored.findings, vec!["A scores 90 [S1]".to_string()]);
        assert_eq!(
            restored.source_refs,
            vec!["S1 https://example.com/a".to_string()]
        );
        assert_eq!(
            restored.pending_work,
            vec!["verify B benchmarks".to_string()]
        );
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

    #[test]
    fn accepted_plan_is_the_archive_loaded_after_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("archive.sqlite");
        let store = crate::store::Store::open(&db).unwrap();
        let owner = store.create_conversation().unwrap().id;
        store.begin_turn(&owner, "earlier request").unwrap();
        let text = format!("{}unique tail", "a".repeat(4000));
        let mut messages = vec![
            json!({"role":"system","content":"instructions"}),
            json!({"role":"user","content":"earlier request"}),
            json!({"role":"assistant","content":text}),
            json!({"role":"user","content":"current request"}),
        ];
        let plan = plan_compaction(&messages, 0).unwrap();
        let exact_payload = plan.kept.clone();
        let id = plan.id.clone();
        assert_eq!(commit_plan(&store, &owner, &mut messages, plan, 0, false).unwrap(), id);
        assert_eq!(messages, exact_payload);
        store.begin_turn(&owner, "current request").unwrap();
        drop(store);
        let store = crate::store::Store::open(&db).unwrap();
        let checkpoint = store.compaction(&owner).unwrap().unwrap();
        assert_eq!(checkpoint.artifact_id, id);
        let archived = store.artifact(&id).unwrap().unwrap();
        assert_eq!(archived.conversation_id, owner);
        assert!(archived.content.contains("unique tail"));
        let replay = crate::history::model_history_with_cutoff(&store.messages(&owner).unwrap(), Some((checkpoint.cutoff, &id))).unwrap();
        assert!(replay.iter().any(|m| m["content"] == "current request"));
    }

}

#[cfg(test)]
mod handoff_regressions {
    use super::*;
    #[test]
    fn old_requirements_survive_live_compaction_and_durable_rebuild() {
        let store = crate::store::Store::open_memory().unwrap();
        let id = store.create_conversation().unwrap().id;
        let request = "Never change SENTINEL_API_CONTRACT. Keep the legacy endpoint.";
        let mut messages = vec![json!({"role":"system","content":"system"})];
        for (role, text) in [("user",request),("assistant","Initial notes"),("user","Later work"),("assistant","Result")] {
            store.append_message(&id, role, text, "complete").unwrap();
            messages.push(json!({"role":role,"content":text}));
        }
        messages.push(json!({"role":"user","content":"Continue"}));
        let plan = plan_compaction(&messages, 1).unwrap();
        assert!(serde_json::to_string(&plan.kept).unwrap().contains("SENTINEL_API_CONTRACT"));
        let archive = commit_plan(&store, &id, &mut messages, plan, 1, false).unwrap();
        let checkpoint = store.compaction(&id).unwrap().unwrap();
        let mut replay = crate::history::model_history_with_cutoff(&store.messages(&id).unwrap(), Some((checkpoint.cutoff, &archive))).unwrap();
        restore_handoff(&store, &id, &archive, &mut replay).unwrap();
        assert!(serde_json::to_string(&replay).unwrap().contains("SENTINEL_API_CONTRACT"));
        let literal = json!({"role":"user","content":"Context checkpoint: the first line is an example. Never alter protected.txt."});
        replay.push(literal.clone());
        restore_handoff(&store, &id, &archive, &mut replay).unwrap();
        assert_eq!(replay.last(), Some(&literal));
        let followup = plan_compaction(&replay, 0).unwrap();
        assert_eq!(followup.kept.last(), Some(&literal));
        let next = task_checkpoint(&messages, "next-archive");
        assert!(next.constraints.iter().any(|rule| rule.contains("SENTINEL_API_CONTRACT")));
    }
    #[test]
    fn long_active_exchange_keeps_recent_native_rounds_and_unfinished_calls() {
        let mut messages = vec![json!({"role":"system","content":"system"}), json!({"role":"user","content":"Keep the API. Implement the feature."})];
        for n in 0..6 {
            messages.push(json!({"role":"assistant","content":"","locallm_responses_output":[],"tool_calls":[{"id":format!("c{n}"),"type":"function","function":{"name":"read_file","arguments":"{}"}}]}));
            if n < 5 { messages.push(json!({"role":"tool","tool_call_id":format!("c{n}"),"content":json!({"content":"x".repeat(2000)}).to_string()})); }
        }
        let plan = plan_compaction(&messages, 10).unwrap();
        assert!(plan.kept.iter().any(|message| message["tool_calls"][0]["id"] == "c5"));
        assert!(!plan.kept.iter().any(|message| message["tool_calls"][0]["id"] == "c0"));
        assert_eq!(plan.kept.last(), messages.last());
        let text = serde_json::to_string(&plan.kept).unwrap();
        assert!(text.contains("c0") && text.contains("completedOperationIds"));
    }

    #[test]
    fn repeated_active_compaction_keeps_the_attachment_location_and_unfinished_parallel_round() {
        let store = crate::store::Store::open_memory().unwrap();
        let owner = store.create_conversation().unwrap().id;
        let user = json!({"role":"user","content":[
            {"type":"text","text":"Never modify protected.txt. Inspect the attachment."},
            {"type":"image_url","image_url":{"url":"data:image/png;base64,AA=="}}
        ]});
        let round = |ids: &[&str]| json!({"role":"assistant","content":"","tool_calls":
            ids.iter().map(|id|json!({"id":id,"type":"function","function":{"name":"read_file","arguments":"{}"}})).collect::<Vec<_>>()
        });
        let result = |id: &str| json!({"role":"tool","tool_call_id":id,
            "content":json!({"evidence":"data ".repeat(1000)}).to_string()
        });
        let mut messages = vec![user.clone(), round(&["first"]), result("first")];
        let first = plan_compaction(&messages, 0).unwrap();
        let first_id = commit_plan(&store, &owner, &mut messages, first, 0, true).unwrap();
        let first_artifact = store.artifact(&first_id).unwrap().unwrap();
        let first_archive: Value = serde_json::from_str(&first_artifact.content).unwrap();
        assert_eq!(first_archive["messages"][0]["content"][1], user["content"][1]);
        assert_eq!(first_archive["handoff"]["constraints"], json!(["Never modify protected.txt"]));
        assert!(first_archive["handoff"]["attachmentRefs"][0].as_str().unwrap().contains(
            &format!("{first_id}, message 0, content part 1")
        ));

        let unfinished = [round(&["left", "right"]), result("left")];
        messages.extend([round(&["second"]), result("second")]);
        messages.extend(unfinished.clone());
        let second = plan_compaction(&messages, 0).unwrap();
        assert!(second.kept.ends_with(&unfinished));
        let second_id = commit_plan(&store, &owner, &mut messages, second, 0, true).unwrap();
        let second_artifact = store.artifact(&second_id).unwrap().unwrap();
        let second_archive: Value = serde_json::from_str(&second_artifact.content).unwrap();
        assert_eq!(second_archive["handoff"]["attachmentRefs"], first_archive["handoff"]["attachmentRefs"]);
        assert_eq!(second_archive["handoff"]["completedOperationIds"], json!(["first", "second"]));
        assert!(second_archive["messages"].as_array().unwrap().iter().all(|message| message != &user));
        assert!(plan_compaction(&messages, 0).is_none());
    }
}
