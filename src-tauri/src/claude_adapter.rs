//! Native Anthropic Messages adapter: actual request, streaming,
//! tool-result, and stop-reason protocol, normalized into the internal
//! OpenAI-style events the chat loop already consumes.
//!
//! Protocol reference (Anthropic Messages API, `anthropic-version:
//! 2023-06-01`): `POST {base}/v1/messages` with `{model, max_tokens,
//! system?, messages, tools?, tool_choice?, stream?}`. Streaming SSE events:
//! `message_start`, `content_block_start` (with `tool_use` blocks carrying
//! `id`/`name`), `content_block_delta` (`text_delta` / `input_json_delta`),
//! `message_delta` (with `stop_reason`), `message_stop`. Stop reasons:
//! `end_turn`, `max_tokens`, `stop_sequence`, `tool_use`. Tool results return
//! as `user` messages with `tool_result` content blocks (`tool_use_id`).
//!
//! Structured outputs: `tool_choice: {type: "tool", name}` forces one tool
//! call whose input is the structured object.

use serde_json::{json, Value};

/// Anthropic API version header sent with every Messages request.
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Thinking signatures and redacted blocks must be replayed verbatim alongside
/// tool use. Keep the native block order rather than reconstructing reasoning
/// from the text shown in the UI.
#[derive(Default)]
pub struct StreamContent {
    blocks: std::collections::BTreeMap<u64, Value>,
    bytes: usize,
}

impl StreamContent {
    pub fn observe(&mut self, event: &Value) -> Result<(), String> {
        let (field, fragment) = match event["type"].as_str() {
            Some("content_block_start") => {
                let block = &event["content_block"];
                if !matches!(
                    block["type"].as_str(),
                    Some("text" | "thinking" | "redacted_thinking" | "tool_use")
                ) {
                    return Ok(());
                }
                let index = event["index"]
                    .as_u64()
                    .ok_or("Claude content block has no index.")?;
                if self.blocks.len() >= 128 || self.blocks.contains_key(&index) {
                    return Err("Claude returned too many or duplicate content blocks.".into());
                }
                self.bytes = self.bytes.saturating_add(block.to_string().len());
                if self.bytes > 4_194_304 {
                    return Err("Claude content exceeded 4 MiB.".into());
                }
                self.blocks.insert(index, block.clone());
                return Ok(());
            }
            Some("content_block_delta") => match event["delta"]["type"].as_str() {
                Some("text_delta") => ("text", &event["delta"]["text"]),
                Some("thinking_delta") => ("thinking", &event["delta"]["thinking"]),
                Some("signature_delta") => ("signature", &event["delta"]["signature"]),
                _ => return Ok(()),
            },
            _ => return Ok(()),
        };
        let index = event["index"]
            .as_u64()
            .ok_or("Claude content delta has no index.")?;
        let fragment = fragment
            .as_str()
            .ok_or("Claude content fragment must be a string.")?;
        self.bytes = self.bytes.saturating_add(fragment.len());
        if self.bytes > 4_194_304 {
            return Err("Claude content exceeded 4 MiB.".into());
        }
        let block = self
            .blocks
            .get_mut(&index)
            .ok_or("Claude content delta arrived before its block.")?;
        let Some(Value::String(text)) = block.get_mut(field) else {
            return Err("Claude content delta does not match its block.".into());
        };
        text.push_str(fragment);
        Ok(())
    }

    pub fn replay(&self, calls: &[crate::tool_calls::ToolCall]) -> Option<Value> {
        if !self.blocks.values().any(|block| {
            matches!(
                block["type"].as_str(),
                Some("thinking" | "redacted_thinking")
            )
        }) {
            return None;
        }
        let mut blocks: Vec<Value> = self.blocks.values().cloned().collect();
        for call in calls {
            if let Some(block) = blocks
                .iter_mut()
                .find(|block| block["type"] == "tool_use" && block["id"] == call.id)
            {
                block["input"] = call.arguments.clone();
            } else {
                blocks.push(
                    json!({"type":"tool_use","id":call.id,"name":call.name,"input":call.arguments}),
                );
            }
        }
        Some(Value::Array(blocks))
    }
}

/// Convert the internal OpenAI-style payload
/// (`{messages, tools, tool_choice, max_tokens, temperature, top_p, model}`)
/// into a native Messages request. The internal `system` message becomes the
/// top-level `system` string; `tool` role messages become `tool_result`
/// blocks; `assistant.tool_calls` become `tool_use` blocks.
pub fn to_messages_request(
    internal: &Value,
    model_id: &str,
    max_output_tokens: Option<u32>,
) -> Value {
    let max_tokens = internal
        .get("max_tokens")
        .and_then(Value::as_u64)
        .map(|value| value.min(u64::from(max_output_tokens.unwrap_or(u32::MAX))) as u32)
        .or(max_output_tokens)
        .unwrap_or(1024)
        .max(1);
    let mut system_parts: Vec<String> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();
    for message in internal
        .get("messages")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("user");
        if role == "system" {
            if let Some(text) = message.get("content").and_then(Value::as_str) {
                system_parts.push(text.to_string());
            }
            continue;
        }
        if role == "tool" {
            let tool_use_id = message
                .get("tool_call_id")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_string();
            let content = message.get("content").and_then(Value::as_str).unwrap_or("");
            messages.push(json!({
                "role": "user",
                "content": [{"type": "tool_result", "tool_use_id": tool_use_id, "content": content}],
            }));
            continue;
        }
        if role == "assistant" {
            if let Some(content) = message
                .get("locallm_claude_content")
                .and_then(Value::as_array)
            {
                messages.push(json!({"role":"assistant","content":content}));
                continue;
            }
            let mut content: Vec<Value> = Vec::new();
            if let Some(text) = message.get("content").and_then(Value::as_str) {
                if !text.is_empty() {
                    content.push(json!({"type": "text", "text": text}));
                }
            }
            for call in message
                .get("tool_calls")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
            {
                let name = call
                    .get("function")
                    .and_then(|function| function.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let id = call.get("id").and_then(Value::as_str).unwrap_or("");
                let input: Value = call
                    .get("function")
                    .and_then(|function| function.get("arguments"))
                    .and_then(Value::as_str)
                    .and_then(|text| serde_json::from_str(text).ok())
                    .unwrap_or(json!({}));
                content.push(json!({"type": "tool_use", "id": id, "name": name, "input": input}));
            }
            if content.is_empty() {
                continue;
            }
            messages.push(json!({"role": "assistant", "content": content}));
            continue;
        }
        let content = message.get("content").cloned().unwrap_or(Value::Null);
        let content = if let Some(parts) = content.as_array() {
            Value::Array(parts.iter().filter_map(|part| {
                if part["type"] != "image_url" { return Some(part.clone()); }
                let url = part["image_url"]["url"].as_str()?;
                let (media, data) = url.strip_prefix("data:")?.split_once(";base64,")?;
                if !["image/png", "image/jpeg", "image/webp"].contains(&media) { return None; }
                Some(json!({"type":"image","source":{"type":"base64","media_type":media,"data":data}}))
            }).collect())
        } else {
            content
        };
        messages.push(json!({"role": "user", "content": content}));
    }
    let mut request = json!({
        "model": model_id,
        "max_tokens": max_tokens,
        "messages": messages,
        "stream": true,
    });
    if !system_parts.is_empty() {
        request["system"] = json!(system_parts.join("\n\n"));
    }
    if let Some(temperature) = internal.get("temperature").and_then(Value::as_f64) {
        request["temperature"] = json!(temperature);
    }
    // Recent Claude models reject setting both sampling controls.
    if request.get("temperature").is_none() {
        if let Some(top_p) = internal.get("top_p").and_then(Value::as_f64) {
            request["top_p"] = json!(top_p);
        }
    }
    if let Some(tools) = internal.get("tools") {
        let converted: Vec<Value> = tools
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .filter_map(|tool| {
                let function = tool.get("function")?;
                Some(json!({
                    "name": function.get("name")?.clone(),
                    "description": function.get("description").cloned().unwrap_or(Value::Null),
                    "input_schema": function.get("parameters").cloned().unwrap_or(json!({"type": "object"})),
                }))
            })
            .collect();
        if !converted.is_empty() {
            request["tools"] = Value::Array(converted);
        }
    }
    if request.get("tools").is_some() {
        if let Some(choice) = internal.get("tool_choice") {
            request["tool_choice"] = match choice.as_str() {
                Some("none") => json!({"type":"none"}),
                Some("required") => json!({"type":"any"}),
                Some("auto") => json!({"type":"auto"}),
                _ if choice["function"]["name"].is_string() => {
                    json!({"type":"tool","name":choice["function"]["name"]})
                }
                _ => json!({"type":"auto"}),
            };
        }
        if internal["parallel_tool_calls"] == false {
            if request.get("tool_choice").is_none() {
                request["tool_choice"] = json!({"type":"auto"});
            }
            request["tool_choice"]["disable_parallel_tool_use"] = json!(true);
        }
    }
    // Adaptive mode avoids imposing a manual thinking budget on models that
    // reject it. Older/unknown models keep their default protocol.
    let adaptive = [
        "claude-sonnet-4-6",
        "claude-opus-4-6",
        "claude-opus-4-7",
        "claude-sonnet-4-7",
    ]
    .iter()
    .any(|prefix| model_id == *prefix || model_id.starts_with(&format!("{prefix}-20")));
    if adaptive {
        match internal["locallm_reasoning_mode"].as_str() {
            Some("quick") | Some("think") => {
                request["thinking"] = json!({"type":"adaptive"});
                request["output_config"] = json!({"effort": if internal["locallm_reasoning_mode"] == "quick" { "low" } else { "high" }});
                if let Some(object) = request.as_object_mut() {
                    object.remove("temperature");
                    object.remove("top_p");
                }
            }
            _ => {}
        }
    }
    if internal["cache_prompt"] == true {
        request["cache_control"] = json!({"type":"ephemeral"});
    }
    // Parallel tool results must arrive together in the next user message.
    let mut merged: Vec<Value> = Vec::new();
    for mut message in request["messages"].as_array().cloned().unwrap_or_default() {
        if message["role"] == "user" && message["content"][0]["type"] == "tool_result" {
            if let Some(previous) = merged.last_mut().filter(|previous| {
                previous["role"] == "user" && previous["content"][0]["type"] == "tool_result"
            }) {
                previous["content"]
                    .as_array_mut()
                    .unwrap()
                    .append(message["content"].as_array_mut().unwrap());
                continue;
            }
        }
        merged.push(message);
    }
    request["messages"] = json!(merged);
    request
}

/// Normalize one native Messages SSE `data:` payload into internal
/// OpenAI-style delta events (`{choices:[{delta, finish_reason}]}`).
/// Returns `None` for protocol events that carry no model-visible content.
/// Called by the chat streaming loop for `claude-messages` backends.
pub fn normalize_sse_event(data: &str) -> Result<Option<Value>, String> {
    if data.trim() == "[DONE]" {
        return Ok(Some(
            json!({"choices":[{"delta":{},"finish_reason":"stop"}]}),
        ));
    }
    let value: Value =
        serde_json::from_str(data).map_err(|error| format!("Invalid Claude stream: {error}"))?;
    if let Some(error) = value.get("error") {
        return Err(format!("Claude error: {error}"));
    }
    let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
    match event_type {
        "message_start" => Ok(Some(
            json!({"choices":[], "usage":value["message"]["usage"]}),
        )),
        "content_block_start" => {
            let block = &value["content_block"];
            match block["type"].as_str() {
                Some("text") => Ok(Some(
                    json!({"choices":[{"delta":{"content":block["text"]}}]}),
                )),
                Some("thinking") => Ok(Some(
                    json!({"choices":[{"delta":{"reasoning_content":block["thinking"]}}]}),
                )),
                _ => Ok(None),
            }
        }
        "content_block_stop" | "ping" => Ok(None),
        "message_stop" => Ok(Some(json!({"stream_done":true}))),
        "content_block_delta" => {
            let delta = value.get("delta").cloned().unwrap_or(Value::Null);
            let delta_type = delta.get("type").and_then(Value::as_str).unwrap_or("");
            match delta_type {
                "text_delta" => {
                    let text = delta.get("text").and_then(Value::as_str).unwrap_or("");
                    Ok(Some(
                        json!({"choices":[{"delta":{"content":text},"finish_reason":null}]}),
                    ))
                }
                "thinking_delta" => Ok(Some(
                    json!({"choices":[{"delta":{"reasoning_content":delta["thinking"]},"finish_reason":null}]}),
                )),
                // `input_json_delta` fragments accumulate tool arguments; the
                // chat assembler joins them the same way as OpenAI deltas.
                "input_json_delta" => {
                    let fragment = delta
                        .get("partial_json")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let index = value.get("index").and_then(Value::as_u64).unwrap_or(0);
                    Ok(Some(
                        json!({"choices":[{"delta":{"tool_calls":[{"index":index,"function":{"arguments":fragment}}]},"finish_reason":null}]}),
                    ))
                }
                _ => Ok(None),
            }
        }
        "message_delta" => {
            let stop = value
                .get("delta")
                .and_then(|delta| delta.get("stop_reason"))
                .and_then(Value::as_str)
                .unwrap_or("");
            let finish = match stop {
                "tool_use" => "tool_calls",
                "max_tokens" => "length",
                "stop_sequence" | "end_turn" => "stop",
                // A refusal is a completed answer, not a failure: the model
                // declined, and its text is the answer the user should see.
                // Reporting it as an unsupported finish reason discarded the
                // refusal text entirely.
                "refusal" => "refusal",
                // A server-tool pause is resumable, not an error.
                "pause_turn" => "pause_turn",
                "model_context_window_exceeded" => "length",
                "" => return Ok(Some(json!({"choices":[],"usage":value["usage"]}))),
                other => other,
            };
            Ok(Some(
                json!({"choices":[{"delta":{},"finish_reason":finish}],"usage":value["usage"]}),
            ))
        }
        _ => Ok(None),
    }
}

/// Seed `tool_calls` entries from `content_block_start` tool_use blocks so
/// later `input_json_delta` fragments have an id/name to attach to.
#[cfg(test)]
fn tool_use_seed(data: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(data).ok()?;
    if value.get("type").and_then(Value::as_str)? != "content_block_start" {
        return None;
    }
    let block = value.get("content_block")?;
    if block.get("type").and_then(Value::as_str)? != "tool_use" {
        return None;
    }
    let index = value.get("index").and_then(Value::as_u64).unwrap_or(0);
    Some(json!({"tool_calls":[{"index":index,
        "id": block.get("id").cloned().unwrap_or(Value::Null),
        "function":{"name": block.get("name").cloned().unwrap_or(Value::Null), "arguments":""}}]}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tool_screenshots_become_native_image_blocks() {
        let request = to_messages_request(
            &json!({"messages":[{"role":"user","content":[{"type":"text","text":"Tool screenshot"},{"type":"image_url","image_url":{"url":"data:image/png;base64,aGVsbG8="}}]}]}),
            "vision",
            None,
        );
        assert_eq!(
            request["messages"][0]["content"][1],
            json!({"type":"image","source":{"type":"base64","media_type":"image/png","data":"aGVsbG8="}})
        );
    }

    #[test]
    fn tool_policy_and_cache_settings_survive_native_conversion() {
        for (choice, expected) in [
            (json!("none"), json!({"type":"none"})),
            (json!("required"), json!({"type":"any"})),
            (
                json!({"type":"function","function":{"name":"clock"}}),
                json!({"type":"tool","name":"clock"}),
            ),
        ] {
            let request = to_messages_request(
                &json!({
                    "messages":[{"role":"user","content":"hi"}], "cache_prompt":true,
                    "tools":[{"type":"function","function":{"name":"clock","parameters":{"type":"object"}}}],
                    "tool_choice":choice,"max_tokens":8192,"temperature":0.2,"top_p":0.9
                }),
                "claude",
                Some(16384),
            );
            assert_eq!(request["tool_choice"], expected);
            assert_eq!(request["max_tokens"], 8192);
            assert_eq!(request["cache_control"], json!({"type":"ephemeral"}));
            assert!(request.get("top_p").is_none());
        }
    }

    #[test]
    fn parallel_tool_results_are_one_user_message_before_followup() {
        let request = to_messages_request(
            &json!({"messages":[
                {"role":"tool","tool_call_id":"a","content":"A"},
                {"role":"tool","tool_call_id":"b","content":"B"},
                {"role":"user","content":"Continue"}
            ]}),
            "claude",
            None,
        );
        assert_eq!(request["messages"].as_array().unwrap().len(), 2);
        assert_eq!(
            request["messages"][0]["content"].as_array().unwrap().len(),
            2
        );
        assert_eq!(request["messages"][0]["content"][1]["tool_use_id"], "b");
        assert_eq!(request["messages"][1]["content"], "Continue");
    }

    #[test]
    fn unknown_stop_reasons_are_preserved_and_output_limit_is_respected() {
        let event =
            normalize_sse_event(r#"{"type":"message_delta","delta":{"stop_reason":"refusal"}}"#)
                .unwrap()
                .unwrap();
        assert_eq!(event["choices"][0]["finish_reason"], "refusal");
        let request = to_messages_request(&json!({"max_tokens":8192}), "claude", Some(2048));
        assert_eq!(request["max_tokens"], 2048);
    }

    #[test]
    fn request_mapping_uses_messages_protocol_not_chat_completions() {
        let internal = json!({
            "model": "ignored",
            "max_tokens": 512,
            "temperature": 0.2,
            "messages": [
                {"role": "system", "content": "sys"},
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": "ok", "tool_calls": [{"id": "t1", "type": "function", "function": {"name": "search", "arguments": "{\"q\":\"x\"}"}}]},
                {"role": "tool", "tool_call_id": "t1", "content": "result"}
            ],
            "tools": [{"type": "function", "function": {"name": "search", "description": "d", "parameters": {"type": "object"}}}],
        });
        let request = to_messages_request(&internal, "claude-model", Some(2048));
        assert_eq!(request["model"], "claude-model");
        assert_eq!(request["max_tokens"], 512);
        assert_eq!(request["system"], "sys");
        assert!(request
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(false));
        assert!(request
            .get("messages")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .all(|message| message["role"] != "system"));
        let tool_result = request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|message| message["content"][0]["type"] == "tool_result")
            .unwrap();
        assert_eq!(tool_result["content"][0]["tool_use_id"], "t1");
        assert_eq!(request["tools"][0]["name"], "search");
        assert!(request["tools"][0].get("input_schema").is_some());
    }

    #[test]
    fn stream_events_normalize_to_internal_deltas() {
        let text = normalize_sse_event(
            r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"hello"}}"#,
        )
        .unwrap()
        .unwrap();
        assert_eq!(text["choices"][0]["delta"]["content"], "hello");
        let seed = tool_use_seed(r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"search"}}"#).unwrap();
        assert_eq!(seed["tool_calls"][0]["id"], "t1");
        let fragment = normalize_sse_event(r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"q\":"}}"#).unwrap().unwrap();
        assert_eq!(
            fragment["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
            "{\"q\":"
        );
        let stop_tool =
            normalize_sse_event(r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#)
                .unwrap()
                .unwrap();
        assert_eq!(stop_tool["choices"][0]["finish_reason"], "tool_calls");
        let stop_length =
            normalize_sse_event(r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"}}"#)
                .unwrap()
                .unwrap();
        assert_eq!(stop_length["choices"][0]["finish_reason"], "length");
        assert!(normalize_sse_event(r#"{"type":"ping"}"#).unwrap().is_none());
        assert!(normalize_sse_event(r#"{"type":"bogus"}"#)
            .unwrap()
            .is_none());
        assert!(normalize_sse_event("not json").is_err());
    }
}

#[cfg(test)]
mod reasoning_tests {
    use super::*;
    #[test]
    fn adaptive_thinking_is_model_specific_and_removes_incompatible_sampling() {
        let payload = json!({"messages":[], "max_tokens":2048, "temperature":0.5, "locallm_reasoning_mode":"quick"});
        let modern = to_messages_request(&payload, "claude-sonnet-4-6", None);
        assert_eq!(modern["thinking"]["type"], "adaptive");
        assert_eq!(modern["output_config"]["effort"], "low");
        assert!(modern.get("temperature").is_none());
        assert!(to_messages_request(&payload, "custom-model", None)
            .get("thinking")
            .is_none());
        assert!(
            to_messages_request(&payload, "claude-sonnet-4-6-custom", None)
                .get("thinking")
                .is_none()
        );
    }
}
