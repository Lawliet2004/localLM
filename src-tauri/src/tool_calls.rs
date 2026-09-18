use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct ToolCalls(BTreeMap<u64, PartialCall>);
#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}
impl ToolCalls {
    pub fn is_empty(&self) -> bool { self.0.is_empty() }
    /// Assemble one OpenAI-compatible streaming delta. Some local runtimes
    /// (AREX-family templates) interleave `content` reasoning prose with tool
    /// deltas in the same round; prose is ignored here and separated by the
    /// chat-level think filter instead.
    pub fn push(&mut self, delta: &Value) -> Result<(), String> {
        let Some(calls) = delta.get("tool_calls") else {
            return Ok(());
        };
        for call in calls.as_array().ok_or("Invalid tool call array.")? {
            let index = call["index"]
                .as_u64()
                .ok_or("Tool call index is missing.")?;
            if index >= 8 {
                return Err("The model requested more than eight tools in one round.".into());
            }
            let entry = self.0.entry(index).or_default();
            for (target, value) in [
                (&mut entry.id, &call["id"]),
                (&mut entry.name, &call["function"]["name"]),
                (&mut entry.arguments, &call["function"]["arguments"]),
            ] {
                if let Some(text) = value.as_str() {
                    target.push_str(text);
                } else if !value.is_null() {
                    return Err("Malformed tool call fragment.".into());
                }
            }
            if entry.id.len() > 128 || entry.name.len() > 128 || entry.arguments.len() > 65_536 {
                return Err("Tool call exceeds its size limit.".into());
            }
        }
        Ok(())
    }
    pub fn finish(self) -> Result<Vec<ToolCall>, String> {
        let mut ids = std::collections::HashSet::new();
        let calls: Vec<ToolCall> = self.0
            .into_values()
            .map(|call| {
                if call.id.is_empty() || call.name.is_empty() || !ids.insert(call.id.clone()) {
                    return Err("Tool call has missing or duplicate identifiers.".into());
                }
                // Truncated generations arrive as partial JSON; detect them
                // here so the chat loop rejects unfinished arguments instead
                // of executing a guessed call.
                if call.arguments.trim().is_empty() {
                    return Err("Tool arguments are empty; the generation may have been truncated.".into());
                }
                let arguments: Value = serde_json::from_str(&call.arguments)
                    .map_err(|_| "Tool arguments are not valid JSON. The generation may have been truncated; unfinished tool calls are not executed.")?;
                if !arguments.is_object() {
                    return Err("Tool arguments must be a JSON object.".into());
                }
                Ok(ToolCall {
                    id: call.id,
                    name: call.name,
                    arguments,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        // Incomplete tool arguments (unbalanced braces from a cut-off stream)
        // are a truncation signal, not an executable call.
        for call in &calls {
            let text = call.arguments.to_string();
            if !braces_balanced(&text) {
                return Err("Tool arguments are incomplete (unbalanced JSON); the generation was truncated and the call was not executed.".into());
            }
        }
        Ok(calls)
    }
}

/// True when every `{`/`[` outside strings is closed. Local runtimes cut
/// streams at the response budget; an unbalanced suffix means truncation.
fn braces_balanced(text: &str) -> bool {
    let mut stack: Vec<char> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for ch in text.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' if stack.pop() != Some(ch) => return false,
            _ => {}
        }
    }
    !in_string && stack.is_empty()
}
impl ToolCall {
    pub fn model_value(&self) -> Value {
        json!({"id":self.id,"type":"function","function":{"name":self.name,"arguments":self.arguments.to_string()}})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn assembles_interleaved_calls_in_index_order() {
        let mut calls = ToolCalls::default();
        calls.push(&json!({"tool_calls":[{"index":1,"id":"b","function":{"name":"second","arguments":"{}"}},{"index":0,"id":"a","function":{"name":"first","arguments":"{\"q\":"}}]})).unwrap();
        calls
            .push(&json!({"tool_calls":[{"index":0,"function":{"arguments":"\"café\"}"}}]}))
            .unwrap();
        let calls = calls.finish().unwrap();
        assert_eq!(calls[0].arguments["q"], "café");
        assert_eq!(calls[1].name, "second");
        assert_eq!(
            calls[0].model_value()["function"]["arguments"],
            "{\"q\":\"café\"}"
        );
    }
    #[test]
    fn rejects_malformed_and_excessive_calls() {
        for fragment in [
            json!({"tool_calls":{}}),
            json!({"tool_calls":[{"index":8}]}),
            json!({"tool_calls":[{"index":0,"function":{"arguments":{}}}]}),
        ] {
            assert!(ToolCalls::default().push(&fragment).is_err());
        }
        for arguments in ["[]", "{", "null", ""] {
            let mut calls = ToolCalls::default();
            calls.push(&json!({"tool_calls":[{"index":0,"id":"a","function":{"name":"tool","arguments":arguments}}]})).unwrap();
            assert!(calls.finish().is_err());
        }
    }
    #[test]
    fn truncated_arguments_are_never_executable() {
        // Balanced JSON with a brace inside a string is fine.
        let mut calls = ToolCalls::default();
        calls.push(&json!({"tool_calls":[{"index":0,"id":"a","function":{"name":"tool","arguments":"{\"q\":\"a{b}\"}"}}]})).unwrap();
        assert!(calls.finish().is_ok());
        assert!(braces_balanced("{\"q\":\"a{b}\"}"));
        assert!(!braces_balanced("{\"q\":\"abc\""));
        assert!(!braces_balanced("{\"q\":"));
    }
}
