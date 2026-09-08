use crate::store::Message;
use serde_json::{json, Value};

/// Rebuild completed tool exchanges before the turn's consolidated assistant answer.
/// Stored UI messages place the assistant row before its subsequently appended audits.
pub fn model_history(history: &[Message]) -> Result<Vec<Value>, String> {
    let mut messages = Vec::new();
    let mut answer: Option<&Message> = None;
    for message in history {
        match message.role.as_str() {
            "user" => {
                flush_answer(&mut messages, &mut answer);
                messages.push(json!({"role":"user", "content":message.content}));
            }
            "assistant" => {
                flush_answer(&mut messages, &mut answer);
                answer = Some(message);
            }
            "tool" => {
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
                // Use the immutable audit row identity: provider call IDs can repeat across turns.
                let call_id = format!("history_{}", message.id);
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
                messages.push(json!({"role":"assistant","content":"","tool_calls":[{"id":call_id,"type":"function","function":{"name":alias,"arguments":serde_json::to_string(arguments).map_err(|error| error.to_string())?}}]}));
                messages.push(json!({"role":"tool","tool_call_id":call_id,"content":serde_json::to_string(&result).map_err(|error| error.to_string())?}));
            }
            _ => return Err("A saved message has an unsupported role.".into()),
        }
    }
    flush_answer(&mut messages, &mut answer);
    Ok(messages)
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
}
