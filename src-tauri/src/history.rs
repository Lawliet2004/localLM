use crate::store::Message;
use serde_json::{json, Value};

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
}

pub fn model_history_with_cutoff(history: &[Message], cutoff: Option<(i64, &str)>) -> Result<Vec<Value>, String> {
    let mut messages = Vec::new();
    if let Some((cutoff, artifact_id)) = cutoff {
        let dropped = history.iter().filter(|message| message.created_at <= cutoff).count();
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
                flush_answer(&mut messages, &mut answer);
                messages.push(json!({"role":"user", "content":crate::attachments::model_content(&message.content)?}));
            }
            "assistant" => {
                flush_tools(&mut messages, &mut pending)?;
                flush_answer(&mut messages, &mut answer);
                answer = Some(message);
            }
            "tool" => pending.push(parse_pending(message)?),
            _ => return Err("A saved message has an unsupported role.".into()),
        }
    }
    flush_tools(&mut messages, &mut pending)?;
    flush_answer(&mut messages, &mut answer);
    Ok(messages)
}

fn parse_pending(message: &Message) -> Result<PendingCall, String> {
    let audit: Value = serde_json::from_str(&message.content)
        .map_err(|_| "A saved tool audit is invalid. Export the conversation to inspect it before continuing.")?;
    let request = audit.get("request").unwrap_or(&audit);
    let name = request["name"]
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or("A saved tool audit has no tool name.")?;
    let connector = request["connector"].as_str().unwrap_or("tool");
    let arguments = request["arguments"]
        .as_object()
        .ok_or("A saved tool audit has invalid arguments.")?;
    let call_id = request["callId"]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("history_{}", message.id));
    let step_id = request["stepId"].as_str().filter(|value| !value.is_empty()).map(str::to_string);
    let assistant_content = request["assistantContent"].as_str().unwrap_or("").to_string();
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
            messages.push(json!({"role": "tool", "tool_call_id": call.call_id, "content": call.result}));
        }
        index = end;
    }
    Ok(())
}

fn flush_answer(messages: &mut Vec<Value>, answer: &mut Option<&Message>) {
    if let Some(answer) = answer.take() {
        if answer.status == "complete" && !answer.content.is_empty() {
            messages.push(json!({"role":"assistant","content":answer.content}));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn row(id: &str, role: &str, content: &str, status: &str) -> Message {
        Message {
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
            row("t1", "tool", &live_audit("call_a", "step-0", "Opening files", "read_file", json!({"text":"A"})), "complete"),
            row("t2", "tool", &live_audit("call_b", "step-0", "Opening files", "read_file", json!({"text":"B"})), "complete"),
        ];
        let messages = model_history(&history).unwrap();
        assert_eq!(
            messages.iter().map(|m| m["role"].as_str().unwrap()).collect::<Vec<_>>(),
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
