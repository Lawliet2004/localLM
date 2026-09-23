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

impl ToolCalls {
    /// Wrap text-parsed calls so they pass through `finish()`'s normal
    /// validation (duplicate ids, JSON balance) like structured deltas do.
    pub fn from_parsed(calls: Vec<ToolCall>) -> Self {
        let mut acc = Self::default();
        for (index, call) in calls.into_iter().enumerate() {
            acc.0.insert(index as u64, PartialCall {
                id: call.id,
                name: call.name,
                arguments: call.arguments.to_string(),
            });
        }
        acc
    }

    /// A finished OpenAI message uses the same argument checks as a streamed call.
    /// Partial or non-object arguments are rejected instead of executed.
    pub fn from_openai_message(message: &Value) -> Result<Vec<ToolCall>, String> {
        let Some(calls) = message.get("tool_calls").and_then(Value::as_array) else {
            return Ok(Vec::new());
        };
        if calls.is_empty() {
            return Ok(Vec::new());
        }
        let mut acc = Self::default();
        for (index, call) in calls.iter().enumerate() {
            let arguments = match &call["function"]["arguments"] {
                Value::String(text) => text.clone(),
                Value::Object(_) => call["function"]["arguments"].to_string(),
                _ => return Err("Tool arguments are not valid JSON. The generation may have been truncated; unfinished tool calls are not executed.".into()),
            };
            let id = call["id"].as_str().unwrap_or("");
            let name = call["function"]["name"].as_str().unwrap_or("");
            acc.push(&json!({
                "tool_calls": [{
                    "index": index,
                    "id": id,
                    "function": { "name": name, "arguments": arguments }
                }]
            }))?;
        }
        acc.finish()
    }
}

/// Some local-model templates (AREX-family) stream literal
/// `<tool_call>{json}</tool_call>` or `<function=name>{json}</function>` text
/// instead of structured tool_call deltas. Prose around the blocks is skipped,
/// so a call the model appended after its commentary still executes through
/// the normal permissioned dispatch instead of leaking into the answer.
/// Returns None when any block is malformed — a partial parse is never executed.
pub fn parse_text_tool_calls(text: &str) -> Option<Vec<ToolCall>> {
    let mut rest = text;
    let mut calls = Vec::new();
    while let Some(start) = [rest.find("<tool_call>"), rest.find("<function=")]
        .into_iter().flatten().min()
    {
        rest = &rest[start..];
        if let Some(body) = rest.strip_prefix("<tool_call>") {
            let end = body.find("</tool_call>")?;
            let inner = body[..end].trim();
            // Two wire forms share the <tool_call> wrapper: a JSON object, or
            // a Qwen/AREX-style <function=name><parameter=…>…</parameter></function> block.
            let call = if inner.starts_with("<function=") {
                parse_function_block(inner, calls.len())?
            } else {
                parse_json_call(inner, calls.len())?
            };
            if calls.iter().any(|existing: &ToolCall| existing.id == call.id) { return None; }
            calls.push(call);
            rest = body[end + "</tool_call>".len()..].trim_start();
        } else if let Some(body) = rest.strip_prefix("<function=") {
            let name_end = body.find('>')?;
            let name = body[..name_end].trim();
            if name.is_empty() || name.len() > 128 { return None; }
            let body = &body[name_end + 1..];
            let (args_part, tail) = match body.find("</function>") {
                Some(end) => (&body[..end], &body[end + "</function>".len()..]),
                None => (body, ""),
            };
            let arguments = parse_function_args(args_part.trim())?;
            calls.push(ToolCall { id: format!("textcall_{}", calls.len()), name: name.into(), arguments });
            rest = tail.trim_start();
        } else {
            break;
        }
        if calls.len() > 8 { return None; }
    }
    if calls.is_empty() { return None; }
    Some(calls)
}

/// `{"name": "…", "arguments": {…}}` (or arguments as a JSON string).
fn parse_json_call(inner: &str, index: usize) -> Option<ToolCall> {
    let parsed: Value = serde_json::from_str(inner).ok()?;
    let name = parsed["name"].as_str()?.trim();
    if name.is_empty() || name.len() > 128 { return None; }
    let arguments = match &parsed["arguments"] {
        Value::Object(_) => parsed["arguments"].clone(),
        Value::String(raw) => serde_json::from_str(raw).ok()?,
        _ => return None,
    };
    if !arguments.is_object() { return None; }
    let id = parsed["id"].as_str()
        .filter(|id| !id.is_empty() && id.len() <= 128)
        .map(String::from)
        .unwrap_or_else(|| format!("textcall_{index}"));
    Some(ToolCall { id, name: name.into(), arguments })
}

/// `<function=name>` followed by a bare JSON object or `<parameter=k>v</parameter>`
/// blocks, optionally closed by `</function>`.
fn parse_function_block(text: &str, index: usize) -> Option<ToolCall> {
    let body = text.strip_prefix("<function=")?;
    let name_end = body.find('>')?;
    let name = body[..name_end].trim();
    if name.is_empty() || name.len() > 128 { return None; }
    let body = &body[name_end + 1..];
    let args_part = match body.find("</function>") {
        Some(end) => &body[..end],
        None => body,
    };
    let arguments = parse_function_args(args_part.trim())?;
    Some(ToolCall { id: format!("textcall_{index}"), name: name.into(), arguments })
}

/// `<function>` bodies are either a bare JSON object or Qwen/AREX-style
/// `<parameter=name>value</parameter>` blocks. Structured parameter values
/// (query arrays, evidences objects) arrive as JSON text inside the block.
fn parse_function_args(body: &str) -> Option<Value> {
    let body = body.trim();
    if body.starts_with('{') {
        let args: Value = serde_json::from_str(body).ok()?;
        return args.is_object().then_some(args);
    }
    if body.is_empty() {
        return Some(json!({}));
    }
    let mut args = serde_json::Map::new();
    let mut rest = body;
    while !rest.trim().is_empty() {
        let inner = rest.trim_start().strip_prefix("<parameter=")?;
        let key_end = inner.find('>')?;
        let key = inner[..key_end].trim();
        if key.is_empty() || key.len() > 128 { return None; }
        let after = &inner[key_end + 1..];
        let end = after.find("</parameter>")?;
        let raw = after[..end].trim();
        let value = serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.into()));
        args.insert(key.to_string(), value);
        rest = &after[end + "</parameter>".len()..];
    }
    Some(Value::Object(args))
}

/// Remove the literal tool-call blocks a text fallback consumed, leaving any
/// surrounding prose for the stored assistant message.
pub fn strip_text_tool_calls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let next = [rest.find("<tool_call>"), rest.find("<function=")]
            .into_iter().flatten().min();
        let Some(start) = next else { out.push_str(rest); break };
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        rest = if let Some(body) = after.strip_prefix("<tool_call>") {
            match body.find("</tool_call>") {
                Some(end) => &body[end + "</tool_call>".len()..],
                None => { out.push_str(after); break }
            }
        } else if let Some(body) = after.strip_prefix("<function=") {
            match body.find("</function>") {
                Some(end) => &body[end + "</function>".len()..],
                None => { out.push_str(after); break }
            }
        } else {
            unreachable!()
        };
    }
    out.trim().to_string()
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
    fn text_tool_calls_parse_only_complete_blocks() {
        let calls = parse_text_tool_calls(
            "<tool_call>\n{\"name\":\"search\",\"arguments\":{\"query\":[\"a\",\"b\"]}}\n</tool_call>\n<tool_call>{\"name\":\"visit\",\"arguments\":{\"url\":\"https://example.org\",\"goal\":\"check\"}}</tool_call>",
        )
        .unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "search");
        assert_eq!(calls[0].arguments["query"][0], "a");
        assert_eq!(calls[1].name, "visit");
        // Structured calls flow through the normal finisher validation.
        let assembled = ToolCalls::from_parsed(calls).finish().unwrap();
        assert_eq!(assembled[0].name, "search");

        let legacy = parse_text_tool_calls("<function=search>{\"query\":[\"q\"]}</function>").unwrap();
        assert_eq!(legacy[0].name, "search");

        // AREX's native Qwen-style form: parameter blocks inside the wrapper.
        let arex = parse_text_tool_calls(
            "<tool_call>\n<function=visit>\n<parameter=url>\nhttps://example.org/a\n</parameter>\n<parameter=goal>\nCheck the claim\n</parameter>\n</function>\n</tool_call>",
        )
        .unwrap();
        assert_eq!(arex[0].name, "visit");
        assert_eq!(arex[0].arguments["url"], "https://example.org/a");
        assert_eq!(arex[0].arguments["goal"], "Check the claim");

        // Structured parameters arrive as JSON inside the parameter block.
        let with_json = parse_text_tool_calls(
            "<tool_call><function=search><parameter=query>[\"first\",\"second\"]</parameter></function></tool_call>",
        )
        .unwrap();
        assert_eq!(with_json[0].arguments["query"][1], "second");

        // Malformed or argument-less blocks are never executed.
        for bad in [
            "<tool_call>{\"name\":\"search\",\"arguments\":\"{unclosed\"}</tool_call>",
            "<tool_call>{\"name\":\"search\"}</tool_call>",
            "<tool_call>{\"name\":\"search\",\"arguments\":[]}</tool_call>",
            "<tool_call>not json</tool_call>",
            "<tool_call>{\"name\":\"search\",\"arguments\":{}}", // truncated, no closing tag
        ] {
            assert!(parse_text_tool_calls(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn text_tool_calls_parse_after_prose() {
        // The observed failure: commentary first, then the call markup.
        let calls = parse_text_tool_calls(
            "Let me search specifically for benchmark results.\n\n<tool_call>\n<function=search>\n<parameter=query>\n[\"GPT-6 Astra benchmarks\"]\n</parameter>\n</function>\n</tool_call>",
        )
        .unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "search");
        assert_eq!(calls[0].arguments["query"][0], "GPT-6 Astra benchmarks");

        // Prose between two call blocks is skipped as well.
        let calls = parse_text_tool_calls(
            "first <tool_call>{\"name\":\"search\",\"arguments\":{\"query\":[\"a\"]}}</tool_call> then <tool_call>{\"name\":\"visit\",\"arguments\":{\"url\":\"https://example.org\",\"goal\":\"g\"}}</tool_call>",
        )
        .unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].name, "visit");

        // A malformed embedded block still fails the whole parse.
        assert!(parse_text_tool_calls("prose <tool_call>not json</tool_call>").is_none());
        assert!(parse_text_tool_calls("prose <tool_call>{\"name\":\"x\",\"arguments\":{}}").is_none());
    }

    #[test]
    fn strip_removes_call_markup_but_keeps_prose() {
        let stripped = strip_text_tool_calls(
            "<tool_call>{\"name\":\"search\",\"arguments\":{}}</tool_call>\nChecking results.",
        );
        assert_eq!(stripped, "Checking results.");
        let kept = strip_text_tool_calls("prose before <tool_call>{malformed");
        assert_eq!(kept, "prose before <tool_call>{malformed");
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
