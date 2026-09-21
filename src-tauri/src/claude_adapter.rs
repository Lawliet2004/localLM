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

/// Convert the internal OpenAI-style payload
/// (`{messages, tools, tool_choice, max_tokens, temperature, top_p, model}`)
/// into a native Messages request. The internal `system` message becomes the
/// top-level `system` string; `tool` role messages become `tool_result`
/// blocks; `assistant.tool_calls` become `tool_use` blocks.
pub fn to_messages_request(internal: &Value, model_id: &str, max_output_tokens: Option<u32>) -> Value {
    let max_tokens = internal
        .get("max_tokens")
        .and_then(Value::as_u64)
        .map(|value| value.min(4096) as u32)
        .or(max_output_tokens)
        .unwrap_or(1024)
        .max(1);
    let mut system_parts: Vec<String> = Vec::new();
    let mut messages: Vec<Value> = Vec::new();
    for message in internal.get("messages").and_then(Value::as_array).cloned().unwrap_or_default() {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("user");
        if role == "system" {
            if let Some(text) = message.get("content").and_then(Value::as_str) {
                system_parts.push(text.to_string());
            }
            continue;
        }
        if role == "tool" {
            let tool_use_id = message.get("tool_call_id").and_then(Value::as_str).unwrap_or("unknown").to_string();
            let content = message.get("content").and_then(Value::as_str).unwrap_or("");
            messages.push(json!({
                "role": "user",
                "content": [{"type": "tool_result", "tool_use_id": tool_use_id, "content": content}],
            }));
            continue;
        }
        if role == "assistant" {
            let mut content: Vec<Value> = Vec::new();
            if let Some(text) = message.get("content").and_then(Value::as_str) {
                if !text.is_empty() {
                    content.push(json!({"type": "text", "text": text}));
                }
            }
            for call in message.get("tool_calls").and_then(Value::as_array).cloned().unwrap_or_default() {
                let name = call.get("function").and_then(|function| function.get("name")).and_then(Value::as_str).unwrap_or("");
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
    if let Some(top_p) = internal.get("top_p").and_then(Value::as_f64) {
        request["top_p"] = json!(top_p);
    }
    // Messages supports top_k; the other extended sampler fields (min_p,
    // penalties, seed) have no equivalent and are intentionally not copied.
    if let Some(top_k) = internal.get("top_k").and_then(Value::as_u64) {
        request["top_k"] = json!(top_k);
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
    request
}

/// Normalize one native Messages SSE `data:` payload into internal
/// OpenAI-style delta events (`{choices:[{delta, finish_reason}]}`).
/// Returns `None` for protocol events that carry no model-visible content.
/// Called by the chat streaming loop for `claude-messages` backends.
pub fn normalize_sse_event(data: &str) -> Result<Option<Value>, String> {
    if data.trim() == "[DONE]" {
        return Ok(Some(json!({"choices":[{"delta":{},"finish_reason":"stop"}]})));
    }
    let value: Value = serde_json::from_str(data).map_err(|error| format!("Invalid Claude stream: {error}"))?;
    if let Some(error) = value.get("error") {
        return Err(format!("Claude error: {error}"));
    }
    let event_type = value.get("type").and_then(Value::as_str).unwrap_or("");
    match event_type {
        "message_start" | "content_block_start" | "content_block_stop" | "message_stop" | "ping" => Ok(None),
        "content_block_delta" => {
            let delta = value.get("delta").cloned().unwrap_or(Value::Null);
            let delta_type = delta.get("type").and_then(Value::as_str).unwrap_or("");
            match delta_type {
                "text_delta" => {
                    let text = delta.get("text").and_then(Value::as_str).unwrap_or("");
                    Ok(Some(json!({"choices":[{"delta":{"content":text},"finish_reason":null}]})))
                }
                // `input_json_delta` fragments accumulate tool arguments; the
                // chat assembler joins them the same way as OpenAI deltas.
                "input_json_delta" => {
                    let fragment = delta.get("partial_json").and_then(Value::as_str).unwrap_or("");
                    let index = value.get("index").and_then(Value::as_u64).unwrap_or(0);
                    Ok(Some(json!({"choices":[{"delta":{"tool_calls":[{"index":index,"function":{"arguments":fragment}}]},"finish_reason":null}]})))
                }
                _ => Ok(None),
            }
        }
        "message_delta" => {
            let stop = value.get("delta").and_then(|delta| delta.get("stop_reason")).and_then(Value::as_str).unwrap_or("");
            let finish = match stop {
                "tool_use" => "tool_calls",
                "max_tokens" => "length",
                "stop_sequence" => "stop",
                "end_turn" | "" => "stop",
                _ => "stop",
            };
            Ok(Some(json!({"choices":[{"delta":{},"finish_reason":finish}]})))
        }
        _ => Ok(None),
    }
}

/// Seed `tool_calls` entries from `content_block_start` tool_use blocks so
/// later `input_json_delta` fragments have an id/name to attach to.
pub fn tool_use_seed(data: &str) -> Option<Value> {
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
        assert!(request.get("stream").and_then(Value::as_bool).unwrap_or(false));
        assert!(request.get("messages").and_then(Value::as_array).unwrap().iter().all(|message| message["role"] != "system"));
        let tool_result = request["messages"].as_array().unwrap().iter().find(|message| message["content"][0]["type"] == "tool_result").unwrap();
        assert_eq!(tool_result["content"][0]["tool_use_id"], "t1");
        assert_eq!(request["tools"][0]["name"], "search");
        assert!(request["tools"][0].get("input_schema").is_some());
    }

    #[test]
    fn stream_events_normalize_to_internal_deltas() {
        let text = normalize_sse_event(r#"{"type":"content_block_delta","delta":{"type":"text_delta","text":"hello"}}"#).unwrap().unwrap();
        assert_eq!(text["choices"][0]["delta"]["content"], "hello");
        let seed = tool_use_seed(r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"search"}}"#).unwrap();
        assert_eq!(seed["tool_calls"][0]["id"], "t1");
        let fragment = normalize_sse_event(r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"q\":"}}"#).unwrap().unwrap();
        assert_eq!(fragment["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"], "{\"q\":");
        let stop_tool = normalize_sse_event(r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#).unwrap().unwrap();
        assert_eq!(stop_tool["choices"][0]["finish_reason"], "tool_calls");
        let stop_length = normalize_sse_event(r#"{"type":"message_delta","delta":{"stop_reason":"max_tokens"}}"#).unwrap().unwrap();
        assert_eq!(stop_length["choices"][0]["finish_reason"], "length");
        assert!(normalize_sse_event(r#"{"type":"ping"}"#).unwrap().is_none());
        assert!(normalize_sse_event(r#"{"type":"bogus"}"#).unwrap().is_none());
        assert!(normalize_sse_event("not json").is_err());
    }
}
