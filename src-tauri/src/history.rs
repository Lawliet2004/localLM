use crate::store::Message;
use serde_json::{json, Value};

const NATIVE_MESSAGE: &str = "locallm-native-message-v1";
/// Anchor a native tool round before parallel reads can run. Reuse this row
/// when consuming or cancelling the first member, so replay has one result per
/// call and preserves the model's original opaque context even after a crash.
pub fn reserve_native_read_round(
    store: &crate::store::Store,
    conversation_id: &str,
    audit: &Value,
    message: &Value,
    saved: &mut bool,
) -> Result<Option<Message>, String> {
    if *saved || saved_native_message(message).is_none() {
        return Ok(None);
    }
    let row = store.append_message(conversation_id, "tool", &audit.to_string(), "streaming")?;
    remember_native_round(store, &row, message, saved)?;
    Ok(Some(row))
}
pub fn saved_native_message(message: &Value) -> Option<Value> {
    if message.get("locallm_responses_output").is_some()
        || message.get("locallm_claude_content").is_some()
    {
        Some(json!({"kind":NATIVE_MESSAGE,"message":message}))
    } else {
        None
    }
}
pub fn assistant_from_saved(content: &Value) -> Result<Value, String> {
    if content["kind"] == NATIVE_MESSAGE {
        let message = &content["message"];
        if message["role"] != "assistant"
            || !message.is_object()
            || content.to_string().len() > 5 * 1024 * 1024
        {
            return Err("Saved native assistant context is invalid or oversized.".into());
        }
        Ok(message.clone())
    } else {
        Ok(json!({"role":"assistant","content":content}))
    }
}
/// Keep one native reply per tool round so cache invalidation and restarts do
/// not flatten opaque provider context. A failed save blocks dispatch visibly.
pub fn remember_native_round(
    store: &crate::store::Store,
    row: &Message,
    message: &Value,
    saved: &mut bool,
) -> Result<(), String> {
    if *saved {
        return Ok(());
    }
    if let Some(content) = saved_native_message(message) {
        if let Err(error) = store.save_native_tool_round(&row.id, &content) {
            let audit: Value = serde_json::from_str(&row.content)
                .map_err(|_| "Native context save failed and tool audit cannot be decoded.")?;
            let request = audit.get("request").unwrap_or(&audit);
            let result = crate::tool_envelope::failure_notice(
                "native-context-save-failed",
                "Native model context could not be saved; this tool was not dispatched.",
                false,
                crate::tool_envelope::SIDE_NONE,
                &json!({}),
            );
            store.update_message(&row.id,&json!({"request":request,"result":result}).to_string(),"","complete").map_err(|settle|format!("Native context save failed ({error}); its audit could not settle ({settle}). No new tool was dispatched."))?;
            return Err(format!(
                "Native model context could not be saved ({error}); no new tool was dispatched."
            ));
        }
        *saved = true;
    }
    Ok(())
}

/// Rebuild completed tool exchanges before the turn's consolidated assistant answer.
/// Stored UI messages place the assistant row before its subsequently appended audits.
/// With a compaction cutoff, the dropped prefix is replaced by a checkpoint
/// notice citing the artifact id; audits stay queryable in the database.
///
/// Audits that store `callId` + `stepId` replay as the live shape: one assistant
/// tool-call message per model round, original ids, and that round's content.
/// Legacy rows without those fields keep one assistant+tool pair per audit.
#[allow(dead_code)]
pub fn model_history(history: &[Message]) -> Result<Vec<Value>, String> {
    model_history_with_cutoff(history, None)
}

struct PendingCall {
    call_id: String,
    alias: String,
    arguments: String,
    result: String,
    step_id: Option<String>,
    assistant_content: String,
    native_message: Option<Value>,
}

pub fn model_history_with_cutoff(
    history: &[Message],
    cutoff: Option<(i64, &str)>,
) -> Result<Vec<Value>, String> {
    model_history_with_contents(history, cutoff, &std::collections::HashMap::new())
}

pub fn model_history_with_contents(
    history: &[Message],
    cutoff: Option<(i64, &str)>,
    contents: &std::collections::HashMap<String, Value>,
) -> Result<Vec<Value>, String> {
    let mut messages = Vec::new();
    if let Some((cutoff, artifact_id)) = cutoff {
        let dropped = history
            .iter()
            .filter(|message| message.created_at <= cutoff)
            .count();
        if dropped > 0 {
            messages.push(json!({"role": "user", "content": format!(
                "Context checkpoint: the first {dropped} messages were compacted into artifact {artifact_id} to fit context. \
                 Audits and full history remain in the local database; ask before expanding anything back.")}));
        }
    }
    let mut answer: Option<&Message> = None;
    let mut pending: Vec<PendingCall> = Vec::new();
    for message in history {
        if cutoff.is_some_and(|(cutoff, _)| message.created_at <= cutoff) {
            continue;
        }
        match message.role.as_str() {
            "user" => {
                flush_tools(&mut messages, &mut pending)?;
                flush_answer(&mut messages, &mut answer, contents)?;
                let content = match contents.get(&message.id) {
                    Some(content) => content.clone(),
                    None => crate::attachments::model_content(&message.content)?,
                };
                messages.push(json!({"role":"user", "content":content}));
            }
            "assistant" => {
                flush_tools(&mut messages, &mut pending)?;
                flush_answer(&mut messages, &mut answer, contents)?;
                answer = Some(message);
            }
            "tool" => {
                let mut call = parse_pending(message)?;
                if let Some(content) = contents.get(&message.id) {
                    if content["kind"] == NATIVE_MESSAGE {
                        call.native_message = Some(assistant_from_saved(content)?);
                    }
                }
                pending.push(call);
            }
            _ => return Err("A saved message has an unsupported role.".into()),
        }
    }
    flush_tools(&mut messages, &mut pending)?;
    flush_answer(&mut messages, &mut answer, contents)?;
    Ok(messages)
}

fn parse_pending(message: &Message) -> Result<PendingCall, String> {
    let audit: Value = serde_json::from_str(&message.content).map_err(|_| {
        "A saved tool audit is invalid. Export the conversation to inspect it before continuing."
    })?;
    let request = audit.get("request").unwrap_or(&audit);
    let name = request["name"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or("A saved tool audit has no tool name.")?;
    let connector = request["connector"].as_str().unwrap_or("tool");
    // Early background settlements omitted arguments. Recognize only that
    // exact legacy notice; malformed ordinary tool audits still fail closed.
    let legacy_settlement = connector == "Subagents"
        && name == "subagent"
        && request["authorization"] == "background child settled"
        && request.get("arguments").is_none()
        && audit["result"]["childRunId"].is_string()
        && matches!(
            audit["result"]["status"].as_str(),
            Some("completed" | "failed" | "interrupted")
        );
    let empty = serde_json::Map::new();
    let arguments = if legacy_settlement {
        &empty
    } else {
        request["arguments"]
            .as_object()
            .ok_or("A saved tool audit has invalid arguments.")?
    };
    let call_id = request["callId"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("history_{}", message.id));
    let step_id = request["stepId"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let assistant_content = request["assistantContent"]
        .as_str()
        .unwrap_or("")
        .to_string();
    let alias = crate::connectors::tool_alias(connector, name);
    let result = if message.status == "interrupted" || message.status == "streaming" {
        json!({"isError":true,"outcome":"unknown","message":"This previous tool action was interrupted. It may already have changed data. Do not automatically repeat it.","recordedResult":audit.get("result")})
    } else if let Some(result) = audit.get("result") {
        result.clone()
    } else if request["decision"] == "denied" {
        json!({"isError":true,"message":"The user denied this previous tool request. Do not repeat it without a new instruction."})
    } else {
        json!({"isError":true,"outcome":"unknown","message":"No result was saved for this previous action. Do not automatically repeat it."})
    };
    Ok(PendingCall {
        call_id,
        alias,
        arguments: serde_json::to_string(arguments).map_err(|error| error.to_string())?,
        result: serde_json::to_string(&result).map_err(|error| error.to_string())?,
        step_id,
        assistant_content,
        native_message: None,
    })
}

fn flush_tools(messages: &mut Vec<Value>, pending: &mut Vec<PendingCall>) -> Result<(), String> {
    let calls = std::mem::take(pending);
    let mut index = 0;
    while index < calls.len() {
        let grouped = calls[index].step_id.is_some();
        let mut end = index + 1;
        if grouped {
            while end < calls.len() && calls[end].step_id == calls[index].step_id {
                end += 1;
            }
        }
        let group = &calls[index..end];
        if let Some(native) = group.iter().find_map(|call| call.native_message.as_ref()) {
            // The native round is usable only when it covers the whole audited
            // group. Program-plan and other host-generated call rows share the
            // parent step but are absent from the model's own output; forcing
            // native replay there would drop audited work, so mixed or
            // malformed groups degrade to the canonical replay below — every
            // audited call and result still replays, only the opaque provider
            // context is flattened.
            let usable = native["tool_calls"].as_array().is_some_and(|native_calls| {
                let ids: std::collections::HashSet<&str> = native_calls
                    .iter()
                    .filter_map(|call| call["id"].as_str())
                    .collect();
                native_calls.len() <= 8
                    && ids.len() == native_calls.len()
                    && group.iter().all(|call| ids.contains(call.call_id.as_str()))
            });
            if usable {
                messages.push(native.clone());
                for call in native["tool_calls"].as_array().unwrap() {
                    let id = call["id"].as_str().unwrap();
                    let result=group.iter().find(|call|call.call_id==id).map(|call|call.result.clone()).unwrap_or_else(||crate::tool_envelope::failure_notice("unrecorded-tool-result","No durable result exists for this previous call. Its outcome is unknown; do not automatically repeat it.",false,crate::tool_envelope::SIDE_UNKNOWN,&json!({})).to_string());
                    messages.push(json!({"role":"tool","tool_call_id":id,"content":result}));
                }
                index = end;
                continue;
            }
        }
        let tool_calls: Vec<Value> = group
            .iter()
            .map(|call| {
                json!({"id": call.call_id, "type": "function", "function": {"name": call.alias, "arguments": call.arguments}})
            })
            .collect();
        messages.push(json!({
            "role": "assistant",
            "content": group[0].assistant_content,
            "tool_calls": tool_calls
        }));
        for call in group {
            messages.push(
                json!({"role": "tool", "tool_call_id": call.call_id, "content": call.result}),
            );
        }
        index = end;
    }
    Ok(())
}

fn flush_answer(
    messages: &mut Vec<Value>,
    answer: &mut Option<&Message>,
    contents: &std::collections::HashMap<String, Value>,
) -> Result<(), String> {
    if let Some(answer) = answer.take() {
        if answer.status == "complete" {
            let content = contents
                .get(&answer.id)
                .cloned()
                .unwrap_or_else(|| json!(answer.content));
            // A finish tool can supply the displayed answer without a final
            // assistant text message. Do not invent one during replay.
            if !content.is_null() && content.as_str() != Some("") {
                messages.push(assistant_from_saved(&content)?);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn background_settlements_replay_with_followups_including_legacy_rows() {
        for status in ["completed", "failed"] {
            for legacy in [true, false] {
                let mut notice = json!({"request":{"connector":"Subagents","name":"subagent","decision":"allowed","authorization":"background child settled"},"result":{"childRunId":"child-1","status":status}});
                if !legacy {
                    notice["request"]["arguments"] = json!({});
                }
                let messages = vec![
                    row("u", "user", "Delegate it", "complete"),
                    row("a", "assistant", "Started", "complete"),
                    row("t", "tool", &notice.to_string(), "complete"),
                    row("follow", "user", "What happened?", "complete"),
                ];
                let replay = model_history(&messages).unwrap();
                assert_eq!(replay.last().unwrap()["content"], "What happened?");
                assert!(replay.iter().any(
                    |m| m["role"] == "tool" && m["content"].as_str().unwrap().contains(status)
                ));
                notice["request"]["authorization"] = json!("ordinary action");
                if legacy {
                    assert!(
                        model_history(&[row("bad", "tool", &notice.to_string(), "complete")])
                            .is_err()
                    );
                }
            }
        }
    }
    #[test]
    fn rendered_user_injections_and_final_chunk_replay_without_ui_duplication() {
        let history = vec![
            row("u", "user", "Read this", "complete"),
            row("a", "assistant", "Opening filesDone", "complete"),
            row(
                "t",
                "tool",
                &live_audit(
                    "call_a",
                    "step-0",
                    "Opening files",
                    "read_file",
                    json!({"text":"A"}),
                ),
                "complete",
            ),
        ];
        let rendered = json!("[injected system_time]\n2026-10-15\n\nRead this");
        let contents = [("u".into(), rendered.clone()), ("a".into(), json!("Done"))]
            .into_iter()
            .collect();
        let replay = model_history_with_contents(&history, None, &contents).unwrap();
        assert_eq!(replay[0]["content"], rendered);
        assert_eq!(replay[1]["content"], "Opening files");
        assert_eq!(replay[3]["content"], "Done");
        assert!(!replay
            .iter()
            .any(|message| message["content"] == "Opening filesDone"));
    }

    #[test]
    fn tool_provided_answer_does_not_invent_an_assistant_message() {
        let history = vec![row("a", "assistant", "Displayed finish answer", "complete")];
        let contents = [("a".into(), Value::Null)].into_iter().collect();
        assert!(model_history_with_contents(&history, None, &contents)
            .unwrap()
            .is_empty());
    }
    fn row(id: &str, role: &str, content: &str, status: &str) -> Message {
        Message {
            metrics: None,
            error: None,
            id: id.into(),
            conversation_id: "chat".into(),
            role: role.into(),
            content: content.into(),
            reasoning: "private reasoning".into(),
            status: status.into(),
            created_at: 0,
        }
    }
    fn audit(result: Value) -> String {
        json!({"request":{"connector":"Workspace","name":"read_file","arguments":{"path":"code.txt"},"decision":"allowed"},"result":result}).to_string()
    }
    fn live_audit(call_id: &str, step: &str, content: &str, name: &str, result: Value) -> String {
        json!({"request":{
            "connector":"Workspace","name":name,"arguments":{"path":"code.txt"},"decision":"allowed",
            "callId":call_id,"stepId":step,"assistantContent":content
        },"result":result}).to_string()
    }
    #[test]
    fn replays_results_before_final_answer_and_followup_without_reasoning() {
        let history = vec![
            row("u", "user", "Read a code", "complete"),
            row("a", "assistant", "I read it", "complete"),
            row(
                "t",
                "tool",
                &audit(json!({"text":"UNIQUE-729"})),
                "complete",
            ),
            row("u2", "user", "What was the code?", "complete"),
        ];
        let messages = model_history(&history).unwrap();
        assert_eq!(
            messages
                .iter()
                .map(|message| message["role"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["user", "assistant", "tool", "assistant", "user"]
        );
        assert_eq!(
            messages[1]["tool_calls"][0]["id"],
            messages[2]["tool_call_id"]
        );
        assert!(messages[2]["content"]
            .as_str()
            .unwrap()
            .contains("UNIQUE-729"));
        assert_eq!(messages[3]["content"], "I read it");
        assert!(!serde_json::to_string(&messages)
            .unwrap()
            .contains("private reasoning"));
    }
    #[test]
    fn interrupted_and_denied_actions_keep_their_outcomes_without_partial_answers() {
        let denied =
            json!({"connector":"x","name":"write","arguments":{},"decision":"denied"}).to_string();
        let messages = model_history(&[
            row("a", "assistant", "Unfinished claim", "interrupted"),
            row("t", "tool", &audit(json!({"partial":true})), "interrupted"),
            row("t2", "tool", &denied, "complete"),
        ])
        .unwrap();
        assert_eq!(messages.len(), 4);
        assert!(messages[1]["content"].as_str().unwrap().contains("unknown"));
        assert!(messages[3]["content"].as_str().unwrap().contains("denied"));
        assert_ne!(
            messages[0]["tool_calls"][0]["id"],
            messages[2]["tool_calls"][0]["id"]
        );
    }
    #[test]
    fn malformed_saved_tool_history_is_not_silently_discarded() {
        assert!(model_history(&[row("t", "tool", "invalid JSON", "complete")]).is_err());
        assert!(model_history(&[row("t", "tool", "{}", "complete")]).is_err());
    }
    #[test]
    fn grouped_step_replays_original_ids_and_round_content() {
        let history = vec![
            row("u", "user", "Read both", "complete"),
            row("a", "assistant", "Done", "complete"),
            row(
                "t1",
                "tool",
                &live_audit(
                    "call_a",
                    "step-0",
                    "Opening files",
                    "read_file",
                    json!({"text":"A"}),
                ),
                "complete",
            ),
            row(
                "t2",
                "tool",
                &live_audit(
                    "call_b",
                    "step-0",
                    "Opening files",
                    "read_file",
                    json!({"text":"B"}),
                ),
                "complete",
            ),
        ];
        let messages = model_history(&history).unwrap();
        assert_eq!(
            messages
                .iter()
                .map(|m| m["role"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["user", "assistant", "tool", "tool", "assistant"]
        );
        assert_eq!(messages[1]["content"], "Opening files");
        assert_eq!(messages[1]["tool_calls"].as_array().unwrap().len(), 2);
        assert_eq!(messages[1]["tool_calls"][0]["id"], "call_a");
        assert_eq!(messages[1]["tool_calls"][1]["id"], "call_b");
        assert_eq!(messages[2]["tool_call_id"], "call_a");
        assert_eq!(messages[3]["tool_call_id"], "call_b");
        assert_eq!(messages[4]["content"], "Done");
    }
    #[test]
    fn legacy_audits_without_step_stay_one_pair_each() {
        let messages = model_history(&[
            row("t1", "tool", &audit(json!({"a":1})), "complete"),
            row("t2", "tool", &audit(json!({"b":2})), "complete"),
        ])
        .unwrap();
        assert_eq!(messages.len(), 4);
        assert_eq!(messages[0]["tool_calls"][0]["id"], "history_t1");
        assert_eq!(messages[2]["tool_calls"][0]["id"], "history_t2");
    }
}

#[cfg(test)]
mod native_history_tests {
    use super::*;
    fn native_round() -> Value {
        json!({"role":"assistant","content":"Working","tool_calls":[{"id":"call-1","type":"function","function":{"name":"tool_invoke","arguments":"{\"name\":\"echo\",\"arguments\":{\"input\":\"hello\"}}"}},{"id":"call-2","type":"function","function":{"name":"echo","arguments":"{}"}}],
        "locallm_responses_output":[{"type":"reasoning","id":"rs-1","summary":[],"encrypted_content":"encrypted-context"},
        {"type":"function_call","id":"fc-1","call_id":"call-1","name":"tool_invoke","arguments":"{\"name\":\"echo\",\"arguments\":{\"input\":\"hello\"}}"},
        {"type":"function_call","id":"fc-2","call_id":"call-2","name":"echo","arguments":"{}"}]})
    }
    #[test]
    fn sqlite_restart_rebuild_preserves_native_round_and_marks_missing_results_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        let store = crate::store::Store::open(&path).unwrap();
        let conversation = store.create_conversation().unwrap().id;
        store
            .append_message(
                &conversation,
                "user",
                "Use the selected fixture.",
                "complete",
            )
            .unwrap();
        let answer = store
            .append_message(&conversation, "assistant", "Final", "complete")
            .unwrap();
        let audit = json!({"request":{"connector":"Fixture","name":"echo","arguments":{"input":"hello"},"callId":"call-1","stepId":"step-1","decision":"allowed"},"result":{"processed":true}});
        let tool = store
            .append_message(
                &conversation,
                "tool",
                &audit["request"].to_string(),
                "streaming",
            )
            .unwrap();
        let native = native_round();
        let mut saved = false;
        remember_native_round(&store, &tool, &native, &mut saved).unwrap();
        assert!(saved);
        store
            .update_message(&tool.id, &audit.to_string(), "", "complete")
            .unwrap();
        let final_message = json!({"role":"assistant","content":"Final","tool_calls":[],"locallm_responses_output":[{"type":"message","id":"msg-final","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"Final","annotations":[]}]}]});
        store
            .save_model_content(&answer.id, &saved_native_message(&final_message).unwrap())
            .unwrap();
        drop(store);
        let store = crate::store::Store::open(&path).unwrap();
        let replay = model_history_with_contents(
            &store.messages(&conversation).unwrap(),
            None,
            &store.model_contents(&conversation).unwrap(),
        )
        .unwrap();
        assert_eq!(replay[1], native);
        assert_eq!(replay[2]["tool_call_id"], "call-1");
        assert_eq!(replay[3]["tool_call_id"], "call-2");
        let unknown: Value = serde_json::from_str(replay[3]["content"].as_str().unwrap()).unwrap();
        assert!(crate::tool_envelope::failed(&unknown));
        assert_eq!(unknown["error"]["sideEffects"], "unknown");
        assert_eq!(replay[4], final_message);
        let request =
            crate::responses_adapter::request(&json!({"messages":replay}), "fixture-model")
                .unwrap();
        assert!(request["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["encrypted_content"] == "encrypted-context"));
        let fork = store.fork_conversation(&conversation, &tool.id).unwrap();
        let forked = model_history_with_contents(
            &store.messages(&fork.id).unwrap(),
            None,
            &store.model_contents(&fork.id).unwrap(),
        )
        .unwrap();
        assert!(forked.iter().any(
            |message| message["locallm_responses_output"] == native["locallm_responses_output"]
        ));
        store
            .update_message(
                &tool.id,
                &audit.to_string().replace("hello", "user edited"),
                "",
                "complete",
            )
            .unwrap();
        assert!(
            !store
                .model_contents(&conversation)
                .unwrap()
                .contains_key(&tool.id),
            "editing a completed tool audit must invalidate its old native context"
        );

        assert!(request["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["type"] == "function_call_output" && item["call_id"] == "call-2"));
    }
    #[test]
    fn metadata_write_failure_settles_audit_and_prevents_dispatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        let store = crate::store::Store::open(&path).unwrap();
        let conversation = store.create_conversation().unwrap().id;
        let audit = json!({"connector":"Fixture","name":"echo","arguments":{},"decision":"allowed","callId":"call-1","stepId":"s"});
        let row = store
            .append_message(&conversation, "tool", &audit.to_string(), "streaming")
            .unwrap();
        rusqlite::Connection::open(&path).unwrap().execute_batch("CREATE TRIGGER fixture_deny_context BEFORE INSERT ON native_tool_rounds BEGIN SELECT RAISE(ABORT,'fixture rejects native context'); END;").unwrap();
        let mut saved = false;
        let result = remember_native_round(&store, &row, &native_round(), &mut saved);
        if result.is_ok() {
            std::fs::write(dir.path().join("must-not-dispatch"), "bad").unwrap();
        }
        assert!(result.is_err());
        assert!(!saved);
        assert!(!dir.path().join("must-not-dispatch").exists());
        let row = store.messages(&conversation).unwrap().remove(0);
        assert_eq!(row.status, "complete");
        let result: Value = serde_json::from_str(&row.content).unwrap();
        assert_eq!(
            result["result"]["error"]["code"],
            "native-context-save-failed"
        );
        assert_eq!(result["request"], audit);
    }
    #[test]
    fn mixed_program_and_model_calls_replay_canonically_without_losing_audits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        let store = crate::store::Store::open(&path).unwrap();
        let conversation = store.create_conversation().unwrap().id;
        store
            .append_message(&conversation, "user", "Run the plan", "complete")
            .unwrap();
        let native = json!({"role":"assistant","content":"Running","tool_calls":[{"id":"call-1","type":"function","function":{"name":"ptc_run","arguments":"{\"plan\":{}}"}}],
            "locallm_responses_output":[{"type":"reasoning","id":"rs-5","summary":[],"encrypted_content":"ptc-reasoning"},{"type":"function_call","id":"fc-1","call_id":"call-1","name":"ptc_run","arguments":"{\"plan\":{}}"}]});
        let model_audit = json!({"request":{"connector":"Harness","name":"ptc_run","arguments":{"plan":{}},"callId":"call-1","stepId":"step-2","decision":"allowed","assistantContent":"Running"},"result":{"summary":"ok"}});
        let row = store
            .append_message(&conversation, "tool", &model_audit.to_string(), "complete")
            .unwrap();
        let mut saved = false;
        remember_native_round(&store, &row, &native, &mut saved).unwrap();
        // Inner plan calls carry host-generated ids under the same step.
        for (id, name) in [("plan-a-1", "read_file"), ("plan-a-2", "list_files")] {
            let inner = json!({"request":{"connector":"Workspace","name":name,"arguments":{},"callId":id,"stepId":"step-2","decision":"allowed","program":true},"result":{"text":"data"}});
            store
                .append_message(&conversation, "tool", &inner.to_string(), "complete")
                .unwrap();
        }
        let replay = model_history_with_contents(
            &store.messages(&conversation).unwrap(),
            None,
            &store.model_contents(&conversation).unwrap(),
        )
        .unwrap();
        // Native replay cannot cover the mixed group; canonical replay keeps
        // every audited call and result instead of failing the conversation.
        assert_eq!(replay[1]["tool_calls"].as_array().unwrap().len(), 3);
        assert!(replay[1]["locallm_responses_output"].is_null());
        for (index, id) in [(2, "call-1"), (3, "plan-a-1"), (4, "plan-a-2")] {
            assert_eq!(replay[index]["tool_call_id"], id);
        }
        // The request still passes Responses conversion canonically.
        crate::responses_adapter::request(&json!({"messages":replay}), "fixture-model").unwrap();
    }
    #[test]
    fn finish_round_anchor_replays_native_and_suppressed_calls_stay_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history.sqlite");
        let store = crate::store::Store::open(&path).unwrap();
        let conversation = store.create_conversation().unwrap().id;
        store
            .append_message(&conversation, "user", "Wrap up", "complete")
            .unwrap();
        // run_finish_call's audit shape: one completed audit row for the finish call.
        let finish_result = json!({"answer":"All done","evidences":[],"confidence":0.9});
        let audit = json!({"request":{"connector":"Harness","name":"finish","arguments":{"answer":"All done"},"callId":"call-f","stepId":"step-4","decision":"allowed","assistantContent":"Wrapping up"},"result":finish_result});
        let row = store
            .append_message(&conversation, "tool", &audit.to_string(), "complete")
            .unwrap();
        let native = json!({"role":"assistant","content":"Wrapping up","tool_calls":[
            {"id":"call-f","type":"function","function":{"name":"finish","arguments":"{\"answer\":\"All done\"}"}},
            {"id":"call-x","type":"function","function":{"name":"shell_command","arguments":"{\"cmd\":\"rm -rf x\"}"}}],
            "locallm_responses_output":[
                {"type":"reasoning","id":"rs-9","summary":[],"encrypted_content":"finish-reasoning"},
                {"type":"function_call","id":"fc-f","call_id":"call-f","name":"finish","arguments":"{\"answer\":\"All done\"}"},
                {"type":"function_call","id":"fc-x","call_id":"call-x","name":"shell_command","arguments":"{\"cmd\":\"rm -rf x\"}"}]});
        let mut saved = false;
        remember_native_round(&store, &row, &native, &mut saved).unwrap();
        assert!(saved);
        // A reasoning-only final answer is a valid saved model content.
        let answer = store
            .append_message(&conversation, "assistant", "All done", "complete")
            .unwrap();
        let reasoning_only = json!({"role":"assistant","content":"All done","tool_calls":[],"locallm_responses_output":[
            {"type":"reasoning","id":"rs-10","summary":[],"encrypted_content":"final-reasoning"},
            {"type":"message","id":"msg-1","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"All done","annotations":[]}]}]});
        store
            .save_model_content(&answer.id, &saved_native_message(&reasoning_only).unwrap())
            .unwrap();
        let replay = model_history_with_contents(
            &store.messages(&conversation).unwrap(),
            None,
            &store.model_contents(&conversation).unwrap(),
        )
        .unwrap();
        assert_eq!(replay[1], native);
        assert_eq!(replay[2]["tool_call_id"], "call-f");
        assert_eq!(replay[3]["tool_call_id"], "call-x");
        let suppressed: Value =
            serde_json::from_str(replay[3]["content"].as_str().unwrap()).unwrap();
        assert!(crate::tool_envelope::failed(&suppressed));
        assert_eq!(suppressed["error"]["sideEffects"], "unknown");
        assert_eq!(replay[4], reasoning_only);
        // The whole rebuild must satisfy Responses parity and output pairing.
        let request =
            crate::responses_adapter::request(&json!({"messages":replay}), "fixture-model")
                .unwrap();
        let input = request["input"].as_array().unwrap();
        assert!(input
            .iter()
            .any(|item| item["encrypted_content"] == "finish-reasoning"));
        assert!(input
            .iter()
            .any(|item| item["type"] == "function_call_output" && item["call_id"] == "call-f"));
        assert!(input
            .iter()
            .any(|item| item["type"] == "function_call_output" && item["call_id"] == "call-x"));
        assert!(input
            .iter()
            .any(|item| item["encrypted_content"] == "final-reasoning"));
    }
    #[test]
    fn native_wrapper_is_typed_and_does_not_change_ordinary_answers() {
        assert_eq!(
            assistant_from_saved(&json!("ordinary")).unwrap(),
            json!({"role":"assistant","content":"ordinary"})
        );
        assert!(
            assistant_from_saved(&json!({"kind":NATIVE_MESSAGE,"message":{"role":"tool"}}))
                .is_err()
        );
        assert!(saved_native_message(&json!({"role":"assistant","content":"ordinary"})).is_none());
    }
}
