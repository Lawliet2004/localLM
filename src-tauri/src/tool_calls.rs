use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::borrow::Cow;
use std::collections::BTreeMap;

/// Display programs are data, including literal tool markup in code samples.
/// Blank their bytes (preserving offsets) before text-call detection/parsing.
/// Conservative recognition also covers nested Markdown/list/blockquote fences
/// and an interrupted fence: ambiguity must never turn presentation into work.
pub(crate) fn without_presentation_fences(text: &str) -> Cow<'_, str> {
    let mut masked: Option<Vec<u8>> = None;
    let mut fence: Option<(u8, usize)> = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let mut closing = false;
        if let Some((marker, count)) = fence {
            let trimmed = line.trim_start_matches(|c: char| c.is_whitespace() || c == '>');
            let run = trimmed.bytes().take_while(|byte| *byte == marker).count();
            closing = run >= count && trimmed[run..].trim().is_empty();
        } else {
            for (index, marker) in line.bytes().enumerate() {
                if marker != b'`' && marker != b'~' {
                    continue;
                }
                // Inspect each marker run once; long backtick lines must not
                // turn fence recognition into quadratic work.
                if index > 0 && line.as_bytes()[index - 1] == marker {
                    continue;
                }
                let rest = &line[index..];
                let count = rest.bytes().take_while(|byte| *byte == marker).count();
                if count < 3 {
                    continue;
                }
                let language = rest[count..].split_whitespace().next().unwrap_or_default();
                if matches!(language, "openui" | "openui-lang") {
                    fence = Some((marker, count));
                    break;
                }
            }
        }
        if fence.is_some() {
            let bytes = masked.get_or_insert_with(|| text.as_bytes().to_vec());
            for byte in &mut bytes[offset..offset + line.len()] {
                if *byte != b'\n' && *byte != b'\r' {
                    *byte = b' ';
                }
            }
        }
        if closing {
            fence = None;
        }
        offset += line.len();
    }
    match masked {
        Some(bytes) => Cow::Owned(String::from_utf8(bytes).expect("masked complete UTF-8 lines")),
        None => Cow::Borrowed(text),
    }
}

#[derive(Default)]
pub struct ToolCalls(BTreeMap<u64, PartialCall>);
#[derive(Default, Clone)]
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
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    /// Assemble one OpenAI-compatible streaming delta. Some local runtimes
    /// (AREX-family templates) interleave `content` reasoning prose with tool
    /// deltas in the same round; prose is ignored here and separated by the
    /// chat-level think filter instead.
    pub fn push(&mut self, delta: &Value) -> Result<(), String> {
        let Some(calls) = delta.get("tool_calls").filter(|calls| !calls.is_null()) else {
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
            if let Some(kind) = call.get("type").filter(|kind| !kind.is_null()) {
                if kind != "function" {
                    return Err("Unsupported tool call type.".into());
                }
            }
            // `id` and `function.name` identify the call; `arguments` is the
            // only field OpenAI fragments across deltas. Several
            // OpenAI-compatible servers repeat the id and the name in EVERY
            // delta, so appending them would produce "call_abc"+"call_abc"
            // and "read"+"read" — a call that can no longer be resolved.
            merge_identity(&mut entry.id, call["id"].as_str())?;
            merge_identity(&mut entry.name, call["function"]["name"].as_str())?;
            if let Some(text) = call["function"]["arguments"].as_str() {
                entry.arguments.push_str(text);
            } else if !call["function"]["arguments"].is_null() {
                return Err("Malformed tool call fragment.".into());
            }
            if entry.id.len() > 128 || entry.name.len() > 128 || entry.arguments.len() > 65_536 {
                return Err("Tool call exceeds its size limit.".into());
            }
        }
        Ok(())
    }
    /// Validate the accumulated calls **without consuming** the accumulator.
    ///
    /// The finalizing path needs the validated calls and must still be able to
    /// see that the round produced calls, so it cannot take the accumulator
    /// apart. A malformed call is a real error here, not an empty round.
    pub fn validated(&self) -> Result<Vec<ToolCall>, String> {
        if self.0.len() > 8 {
            return Err("The model requested more than eight tools in one round.".into());
        }
        let mut ids = std::collections::HashSet::new();
        let calls: Vec<ToolCall> = self
            .0
            .values()
            .cloned()
            .map(|call| {
                if call.id.is_empty() || call.name.is_empty() || !ids.insert(call.id.clone()) {
                    return Err("Tool call has missing or duplicate identifiers.".into());
                }
                if call.id.len() > 128 || call.name.len() > 128 || call.arguments.len() > 65_536 {
                    return Err("Tool call exceeds its size limit.".into());
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
    pub fn finish(self) -> Result<Vec<ToolCall>, String> {
        self.validated()
    }
}

/// Merge a repeated tool-call identity field.
///
/// The field identifies the call, so it is **replaced** rather than appended
/// when the server repeats it — appending would corrupt the id and the name.
/// A genuine second fragment (the server splitting the name) is still
/// appended, and a value that conflicts with what was already seen is
/// rejected rather than guessed at.
fn merge_identity(target: &mut String, incoming: Option<&str>) -> Result<(), String> {
    let Some(text) = incoming else {
        // A missing field is not a change.
        return Ok(());
    };
    if text.is_empty() {
        // Some servers send an empty string in later deltas.
        return Ok(());
    }
    if target.is_empty() {
        target.push_str(text);
        return Ok(());
    }
    if text == target {
        // A pure repeat.
        return Ok(());
    }
    if text.starts_with(target.as_str()) {
        // The server repeated the field with more of it.
        target.clear();
        target.push_str(text);
        return Ok(());
    } else if target.starts_with(text) {
        // A genuine fragment of a split field.
        target.push_str(text);
        return Ok(());
    }
    Err("Tool call identity changed between stream fragments.".into())
}

impl ToolCalls {
    /// Wrap text-parsed calls so they pass through `finish()`'s normal
    /// validation (duplicate ids, JSON balance) like structured deltas do.
    pub fn from_parsed(calls: Vec<ToolCall>) -> Self {
        let mut acc = Self::default();
        for (index, call) in calls.into_iter().enumerate() {
            acc.0.insert(
                index as u64,
                PartialCall {
                    id: call.id,
                    name: call.name,
                    arguments: call.arguments.to_string(),
                },
            );
        }
        acc
    }

    /// A finished OpenAI message uses the same argument checks as a streamed call.
    /// Partial or non-object arguments are rejected instead of executed.
    #[cfg(test)]
    pub fn from_openai_message(message: &Value) -> Result<Vec<ToolCall>, String> {
        let Some(raw) = message.get("tool_calls").filter(|calls| !calls.is_null()) else {
            return Ok(Vec::new());
        };
        let calls = raw.as_array().ok_or("Invalid tool call array.")?;
        if calls.is_empty() {
            return Ok(Vec::new());
        }
        let mut acc = Self::default();
        for (index, call) in calls.iter().enumerate() {
            if call.get("type").is_some_and(|kind| kind != "function") {
                return Err("Unsupported tool call type.".into());
            }
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
    if text.len() > 1_048_576 {
        return None;
    }
    let visible = without_presentation_fences(text);
    let mut rest = visible.as_ref();
    let mut calls = Vec::new();
    while let Some(start) = [rest.find("<tool_call>"), rest.find("<function=")]
        .into_iter()
        .flatten()
        .min()
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
            if calls
                .iter()
                .any(|existing: &ToolCall| existing.id == call.id)
            {
                return None;
            }
            calls.push(call);
            rest = body[end + "</tool_call>".len()..].trim_start();
        } else if let Some(body) = rest.strip_prefix("<function=") {
            let name_end = body.find('>')?;
            let name = body[..name_end].trim();
            if name.is_empty() || name.len() > 128 {
                return None;
            }
            let body = &body[name_end + 1..];
            let end = body.find("</function>")?;
            let (args_part, tail) = (&body[..end], &body[end + "</function>".len()..]);
            let arguments = parse_function_args(args_part.trim())?;
            calls.push(ToolCall {
                id: format!("textcall_{}", calls.len()),
                name: name.into(),
                arguments,
            });
            rest = tail.trim_start();
        } else {
            break;
        }
        if calls.len() > 8 {
            return None;
        }
    }
    if calls.is_empty() {
        return None;
    }
    Some(calls)
}

/// `{"name": "…", "arguments": {…}}` (or arguments as a JSON string).
fn parse_json_call(inner: &str, index: usize) -> Option<ToolCall> {
    let parsed: Value = serde_json::from_str(inner).ok()?;
    let name = parsed["name"].as_str()?.trim();
    if name.is_empty() || name.len() > 128 {
        return None;
    }
    let arguments = match &parsed["arguments"] {
        Value::Object(_) => parsed["arguments"].clone(),
        Value::String(raw) => serde_json::from_str(raw).ok()?,
        _ => return None,
    };
    if !arguments.is_object() {
        return None;
    }
    let id = match parsed.get("id") {
        None => format!("textcall_{index}"),
        Some(value) => {
            let id = value
                .as_str()
                .filter(|id| !id.is_empty() && id.len() <= 128)?;
            id.to_string()
        }
    };
    Some(ToolCall {
        id,
        name: name.into(),
        arguments,
    })
}

/// `<function=name>` followed by a bare JSON object or `<parameter=k>v</parameter>`
/// blocks, closed by `</function>`.
fn parse_function_block(text: &str, index: usize) -> Option<ToolCall> {
    let body = text.strip_prefix("<function=")?;
    let name_end = body.find('>')?;
    let name = body[..name_end].trim();
    if name.is_empty() || name.len() > 128 {
        return None;
    }
    let body = &body[name_end + 1..];
    let args_part = body.trim_end().strip_suffix("</function>")?;
    let arguments = parse_function_args(args_part.trim())?;
    Some(ToolCall {
        id: format!("textcall_{index}"),
        name: name.into(),
        arguments,
    })
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
        if key.is_empty() || key.len() > 128 {
            return None;
        }
        let after = &inner[key_end + 1..];
        let end = after.find("</parameter>")?;
        let raw = after[..end].trim();
        let value = serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.into()));
        if args.insert(key.to_string(), value).is_some() {
            return None;
        }
        rest = &after[end + "</parameter>".len()..];
    }
    Some(Value::Object(args))
}

/// Remove the literal tool-call blocks a text fallback consumed, leaving any
/// surrounding prose for the stored assistant message.
pub fn strip_text_tool_calls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let visible = without_presentation_fences(text);
    let mut rest = text;
    while !rest.is_empty() {
        let visible_rest = &visible[text.len() - rest.len()..];
        let next = [
            visible_rest.find("<tool_call>"),
            visible_rest.find("<function="),
        ]
        .into_iter()
        .flatten()
        .min();
        let Some(start) = next else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        rest = if let Some(body) = after.strip_prefix("<tool_call>") {
            match body.find("</tool_call>") {
                Some(end) => &body[end + "</tool_call>".len()..],
                None => {
                    out.push_str(after);
                    break;
                }
            }
        } else if let Some(body) = after.strip_prefix("<function=") {
            match body.find("</function>") {
                Some(end) => &body[end + "</function>".len()..],
                None => {
                    out.push_str(after);
                    break;
                }
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
    /// Several OpenAI-compatible servers repeat the call id and the tool name
    /// in every delta. Appending them corrupts both, and the call can no
    /// longer be resolved.
    #[test]
    fn repeated_call_identity_is_replaced_not_appended() {
        let mut calls = ToolCalls::default();
        calls
            .push(
                &json!({"tool_calls":[{"index":0,"id":"call_abc","type":"function",
                "function":{"name":"read","arguments":"{\"path\":"}}]}),
            )
            .unwrap();
        calls
            .push(
                &json!({"tool_calls":[{"index":0,"id":"call_abc","type":"function",
                "function":{"name":"read","arguments":"\"a.txt\"}"}}]}),
            )
            .unwrap();
        let finished = calls.finish().unwrap();
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0].id, "call_abc");
        assert_eq!(finished[0].name, "read");
        assert_eq!(finished[0].arguments, json!({"path":"a.txt"}));
    }

    /// A server that sends an empty identity field in a later delta must not
    /// wipe the id it already sent.
    #[test]
    fn an_empty_repeated_identity_does_not_erase_the_call() {
        let mut calls = ToolCalls::default();
        calls
            .push(
                &json!({"tool_calls":[{"index":0,"id":"call_abc","type":"function",
                "function":{"name":"read","arguments":"{}"}}]}),
            )
            .unwrap();
        calls
            .push(&json!({"tool_calls":[{"index":0,"id":"","type":"function",
                "function":{"name":"","arguments":""}}]}))
            .unwrap();
        let finished = calls.finish().unwrap();
        assert_eq!(finished[0].id, "call_abc");
        assert_eq!(finished[0].name, "read");
    }

    /// A genuinely conflicting identity is rejected rather than guessed at.
    #[test]
    fn a_conflicting_call_identity_is_rejected() {
        let mut calls = ToolCalls::default();
        calls
            .push(
                &json!({"tool_calls":[{"index":0,"id":"call_abc","type":"function",
                "function":{"name":"read","arguments":"{}"}}]}),
            )
            .unwrap();
        assert!(calls
            .push(
                &json!({"tool_calls":[{"index":0,"id":"call_other","type":"function",
                "function":{"name":"read","arguments":""}}]})
            )
            .is_err());
    }

    #[test]
    fn null_tool_deltas_are_noops_and_unsupported_call_types_fail() {
        let mut calls = ToolCalls::default();
        calls.push(&json!({"tool_calls":null})).unwrap();
        assert!(calls.finish().unwrap().is_empty());
        assert!(ToolCalls::default()
            .push(&json!({"tool_calls":[{"index":0,"type":"custom"}]}))
            .is_err());
        assert!(ToolCalls::from_openai_message(&json!({"tool_calls":{}})).is_err());
        assert!(ToolCalls::from_openai_message(&json!({"tool_calls":[{"type":"custom","id":"x","function":{"name":"run","arguments":"{}"}}]})).is_err());
    }

    #[test]
    fn text_and_structured_calls_share_count_and_size_limits() {
        let call = ToolCall {
            id: "a".into(),
            name: "run".into(),
            arguments: json!({}),
        };
        assert!(ToolCalls::from_parsed(vec![call.clone(); 9])
            .finish()
            .is_err());
        let mut large = call;
        large.arguments = json!({"code":"x".repeat(65_536)});
        assert!(ToolCalls::from_parsed(vec![large]).finish().is_err());
    }

    #[test]
    fn incomplete_or_ambiguous_text_calls_never_become_executable() {
        for text in [
            "<function=clock>",
            "<function=run>{\"code\":\"x\"}",
            "<tool_call><function=clock></tool_call>",
            "<function=run><parameter=code>first</parameter><parameter=code>second</parameter></function>",
            "<tool_call><function=clock>{}</function>ignored</tool_call>",
            "<tool_call>{\"id\":\"\",\"name\":\"clock\",\"arguments\":{}}</tool_call>",
        ] {
            assert!(parse_text_tool_calls(text).is_none(), "accepted {text}");
        }
        assert!(ToolCalls::from_parsed(
            parse_text_tool_calls("<function=clock></function>").unwrap()
        )
        .finish()
        .is_ok());
    }
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

        let legacy =
            parse_text_tool_calls("<function=search>{\"query\":[\"q\"]}</function>").unwrap();
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
    fn presentation_fences_never_become_text_tool_calls() {
        for opener in [
            "```openui",
            "```openui-lang",
            "~~~~openui",
            "> ```openui",
            "- ```openui",
        ] {
            let code = format!("{opener}\nroot = Response([CodeBlock(\"<tool_call>{{\\\"name\\\":\\\"shell\\\",\\\"arguments\\\":{{}}}}</tool_call>\")])\n");
            // Literal legacy markup, plus Unicode to verify offset masking.
            let source = format!("{code}例 <function=shell>{{}}</function>\n");
            assert!(parse_text_tool_calls(&source).is_none(), "{opener}");
            assert_eq!(strip_text_tool_calls(&source), source.trim());
        }
        let visual = "```openui\nroot = Response([Text(\"<function=shell>{}</function>\")])\n```";
        let mixed =
            format!("{visual}\n<tool_call>{{\"name\":\"clock\",\"arguments\":{{}}}}</tool_call>");
        let calls = parse_text_tool_calls(&mixed).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "clock");
        assert_eq!(strip_text_tool_calls(&mixed), visual);
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
        assert!(
            parse_text_tool_calls("prose <tool_call>{\"name\":\"x\",\"arguments\":{}}").is_none()
        );
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
