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
    #[cfg(test)]
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
/// Position of a `<function` tag whose next char is `=` or whitespace
/// (`<function=…`, `<function name=…`, `<function\nname=…`).
pub fn find_function_tag(text: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(pos) = text[from..].find("<function") {
        let pos = from + pos;
        match text.as_bytes().get(pos + "<function".len()) {
            Some(b'=') | Some(b' ') | Some(b'\n') | Some(b'\t') | Some(b'\r') => return Some(pos),
            _ => from = pos + 1,
        }
    }
    None
}

/// Bare-JSON wire form: the whole reply is one object `{"tool_call": {…}}`
/// (or an array of call objects). Detection is deliberately positional — only
/// a reply that opens with `{` and holds a "tool_call" key counts, so fenced
/// or mid-prose JSON stays visible text. A reply that opens `{ "tool_call"`
/// but fails to parse is still an attempted call (the caller repairs it).
pub fn starts_bare_tool_call(text: &str) -> bool {
    let text = text.trim_start();
    if !text.starts_with('{') || !text.contains("\"tool_call\"") {
        return false;
    }
    match serde_json::from_str::<Value>(text) {
        Ok(value) => value.get("tool_call").is_some(),
        Err(_) => text[1..].trim_start().starts_with("\"tool_call\""),
    }
}

fn parse_bare_tool_calls(text: &str) -> Option<Vec<ToolCall>> {
    let parsed: Value = serde_json::from_str(text.trim()).ok()?;
    let items: Vec<&Value> = match parsed.get("tool_call")? {
        Value::Array(items) => items.iter().collect(),
        other => vec![other],
    };
    if items.is_empty() || items.len() > 8 {
        return None;
    }
    let mut calls: Vec<ToolCall> = Vec::new();
    for item in items {
        let call = json_call_from_value(item, calls.len())?;
        if calls.iter().any(|existing| existing.id == call.id) {
            return None;
        }
        calls.push(call);
    }
    Some(calls)
}

pub fn parse_text_tool_calls(text: &str) -> Option<Vec<ToolCall>> {
    if text.trim_start().starts_with('{') {
        if let Some(calls) = parse_bare_tool_calls(text) {
            return Some(calls);
        }
    }
    let mut rest = text;
    let mut calls = Vec::new();
    while let Some(start) = [rest.find("<tool_call>"), find_function_tag(rest)]
        .into_iter().flatten().min()
    {
        rest = &rest[start..];
        if let Some(body) = rest.strip_prefix("<tool_call>") {
            let end = body.find("</tool_call>")?;
            let inner = body[..end].trim();
            // Two wire forms share the <tool_call> wrapper: a JSON object, or
            // a Qwen/AREX-style <function=…>/<function name=…> block.
            let call = if inner.starts_with("<function") {
                parse_function_block(inner, calls.len())?
            } else {
                parse_json_call(inner, calls.len())?
            };
            if calls.iter().any(|existing: &ToolCall| existing.id == call.id) { return None; }
            calls.push(call);
            rest = body[end + "</tool_call>".len()..].trim_start();
        } else if rest.starts_with("<function") {
            let (name, arguments, tail) = parse_function_open(function_body(rest), calls.len())?;
            calls.push(ToolCall { id: format!("textcall_{}", calls.len()), name, arguments });
            rest = tail.trim_start();
        } else {
            break;
        }
        if calls.len() > 8 { return None; }
    }
    if calls.is_empty() { return None; }
    Some(calls)
}

/// Both `<function=name>` and the XML-attribute `<function name="name">` open
/// a call block. Returns (name, arguments, text-after-close).
fn parse_function_open(body: &str, _index: usize) -> Option<(String, Value, &str)> {
    let tag_end = body.find('>')?;
    let tag = &body[..tag_end];
    // `<function=name>` — name is the raw tag body; `<function name="n">` —
    // the name is a quoted attribute.
    let name = if tag.contains('"') || tag.contains('\'') {
        xml_attr(tag, "name")?.trim().to_string()
    } else {
        tag.trim().to_string()
    };
    if name.is_empty() || name.len() > 128 { return None; }
    let rest = &body[tag_end + 1..];
    let (args_part, tail) = match rest.find("</function>") {
        Some(end) => (&rest[..end], &rest[end + "</function>".len()..]),
        None => (rest, ""),
    };
    let arguments = parse_function_args(args_part.trim())?;
    Some((name, arguments, tail))
}

/// `key="value"` or `key='value'` inside a tag body.
fn xml_attr(tag: &str, key: &str) -> Option<String> {
    for quote in ['"', '\''] {
        let needle = format!("{key}={quote}");
        if let Some(pos) = tag.find(&needle) {
            let value_start = pos + needle.len();
            let end = tag[value_start..].find(quote)?;
            return Some(tag[value_start..value_start + end].to_string());
        }
    }
    None
}

/// `{"name": "…", "arguments": {…}}` (or arguments as a JSON string).
fn parse_json_call(inner: &str, index: usize) -> Option<ToolCall> {
    json_call_from_value(&serde_json::from_str(inner).ok()?, index)
}

fn json_call_from_value(parsed: &Value, index: usize) -> Option<ToolCall> {
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

/// Content after `<function` and its separator (`=` or whitespace).
fn function_body(tagged: &str) -> &str {
    tagged["<function".len()..]
        .trim_start()
        .strip_prefix('=')
        .map(str::trim_start)
        .unwrap_or_else(|| tagged["<function".len()..].trim_start())
}

/// `<function=name>` or `<function name="n">` followed by a bare JSON object
/// or parameter blocks, optionally closed by `</function>`.
fn parse_function_block(text: &str, index: usize) -> Option<ToolCall> {
    let (name, arguments, _) = parse_function_open(function_body(text), index)?;
    Some(ToolCall { id: format!("textcall_{index}"), name, arguments })
}

/// `<function>` bodies are either a bare JSON object or parameter blocks:
/// `<parameter=name>v</parameter>` (Qwen/AREX) or the XML-attribute form
/// `<param name="k">v</param>` / `<parameter name="k">v</parameter>` emitted
/// by MiniCPM-style templates. Structured parameter values (query arrays,
/// evidences objects) arrive as JSON text inside the block.
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
        let inner = rest.trim_start();
        let (body, close) = if let Some(b) = inner.strip_prefix("<parameter=") {
            (b, "</parameter>")
        } else if let Some(b) = inner.strip_prefix("<param ") {
            (b, "</param>")
        } else {
            let b = inner.strip_prefix("<parameter ")?;
            (b, "</parameter>")
        };
        let tag_end = body.find('>')?;
        let tag = &body[..tag_end];
        let key = if !tag.contains('"') && !tag.contains('\'') {
            tag.trim().to_string()
        } else {
            xml_attr(tag, "name")?.trim().to_string()
        };
        if key.is_empty() || key.len() > 128 { return None; }
        let after = &body[tag_end + 1..];
        let end = after.find(close)?;
        let raw = after[..end].trim();
        let value = serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.into()));
        args.insert(key, value);
        rest = &after[end + close.len()..];
    }
    Some(Value::Object(args))
}

/// Remove the literal tool-call blocks a text fallback consumed, leaving any
/// surrounding prose for the stored assistant message.
pub fn strip_text_tool_calls(text: &str) -> String {
    if starts_bare_tool_call(text) {
        return String::new();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while !rest.is_empty() {
        let next = [rest.find("<tool_call>"), find_function_tag(rest)]
            .into_iter().flatten().min();
        let Some(start) = next else { out.push_str(rest); break };
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        rest = if let Some(body) = after.strip_prefix("<tool_call>") {
            match body.find("</tool_call>") {
                Some(end) => &body[end + "</tool_call>".len()..],
                None => { out.push_str(after); break }
            }
        } else if after.starts_with("<function") {
            match after.find("</function>") {
                Some(end) => &after[end + "</function>".len()..],
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
    fn bare_json_tool_calls_parse_and_strip() {
        // The whole-reply `{"tool_call": {…}}` convention some system prompts
        // use (no markup wrapper at all).
        let calls = parse_text_tool_calls(
            "{\"tool_call\": {\"name\": \"get_weather\", \"arguments\": {\"latitude\": 22.57, \"longitude\": 88.36}}}",
        )
        .unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "get_weather");
        assert_eq!(calls[0].arguments["latitude"], 22.57);

        // Pretty-printed and array forms parse identically.
        let pretty = "{\n  \"tool_call\": [\n    {\"name\": \"a\", \"arguments\": {}},\n    {\"name\": \"b\", \"arguments\": {\"x\": 1}}\n  ]\n}";
        let calls = parse_text_tool_calls(pretty).unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[1].arguments["x"], 1);

        // Strict contract: trailing prose or a missing call shape never executes.
        for bad in [
            "{\"tool_call\": {\"name\": \"a\", \"arguments\": {}}} trailing",
            "```json\n{\"tool_call\": {\"name\": \"a\", \"arguments\": {}}}\n```",
            "{\"tool_call\": \"yes\"}",
            "{\"tool_call\": {\"name\": \"a\"}}",
            "{\"tool_call\": []}",
            "{\"note\": \"x\", \"tool_call\": {\"name\": \"a\", \"arguments\": {}}} extra",
        ] {
            assert!(parse_text_tool_calls(bad).is_none(), "{bad}");
        }
        // Detection stays positional: prose + JSON is visible text, not a call.
        assert!(!starts_bare_tool_call("The format is {\"tool_call\": {\"name\": \"a\", \"arguments\": {}}}"));
        assert!(!starts_bare_tool_call("{\"name\": \"a\", \"arguments\": {}}"));
        assert!(starts_bare_tool_call("{\n \"tool_call\": {\"name\": \"a\", \"arguments\": {}}}"));
        assert!(starts_bare_tool_call("{\"tool_call\": {malformed"));
        assert_eq!(
            strip_text_tool_calls("{\"tool_call\": {\"name\": \"a\", \"arguments\": {}}}"),
            ""
        );
    }

    #[test]
    fn xml_attribute_tool_calls_parse() {
        // MiniCPM-style space/attribute form observed in eval runs.
        let calls = parse_text_tool_calls(
            "<function name=\"web_fetch_url\"><param name=\"url\">https://example.org/a</param></function>\n<function name=\"visit\"><param name=\"url\">https://example.org</param><param name=\"goal\">check</param></function>",
        )
        .unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "web_fetch_url");
        assert_eq!(calls[0].arguments["url"], "https://example.org/a");
        assert_eq!(calls[1].arguments["goal"], "check");

        // Same form inside a <tool_call> wrapper.
        let wrapped = parse_text_tool_calls(
            "<tool_call><function name=\"search\"><parameter name=\"query\">[\"a\",\"b\"]</parameter></function></tool_call>",
        )
        .unwrap();
        assert_eq!(wrapped[0].name, "search");
        assert_eq!(wrapped[0].arguments["query"][1], "b");

        // Unterminated attribute block never executes.
        assert!(parse_text_tool_calls("<function name=\"search\"><param name=\"q\">x").is_none());
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
