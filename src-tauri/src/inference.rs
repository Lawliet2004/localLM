use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use serde_json::Value;
use std::pin::Pin;
use std::time::Duration;

/// Byte stream of translated SSE chunks (`data: {...}\n\n`, `data: [DONE]`).
/// Every backend normalizes to OpenAI-style deltas here, so the agent loop,
/// tool parsing, and live checkpoints stay backend-agnostic.
pub type StreamBody = Pin<Box<dyn Stream<Item = Result<Bytes, String>> + Send>>;

fn boxed<S>(stream: S) -> StreamBody
where
    S: Stream<Item = Result<Bytes, String>> + Send + 'static,
{
    Box::pin(stream)
}

#[async_trait]
pub trait InferenceProvider: Send + Sync {
    fn payload(&self, payload: &Value) -> Value;
    fn context_capacity(&self) -> u32;
    fn response_limit(&self) -> Option<u32>;
    fn supports_tools(&self) -> bool;
    fn context_is_estimate(&self) -> bool;
    async fn check_context(&self, payload: &Value, response_tokens: u32) -> Result<u64, String>;
    async fn stream(&self, payload: &Value) -> Result<StreamBody, String>;
}

pub enum Backend {
    Local {
        client: reqwest::Client,
        endpoint: String,
        key: String,
        context_length: u32,
    },
    OpenAi {
        client: reqwest::Client,
        base_url: String,
        key: String,
        model_id: String,
        context_length: u32,
        max_output_tokens: Option<u32>,
        supports_tools: bool,
    },
    Anthropic {
        client: reqwest::Client,
        base_url: String,
        key: String,
        model_id: String,
        context_length: u32,
        max_output_tokens: Option<u32>,
    },
}

impl Backend {
    pub fn local(endpoint: String, key: String, context_length: u32) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(180))
            .build()
            .map_err(|_| "Could not create the local model HTTP client.".to_string())?;
        Ok(Self::Local {
            client,
            endpoint,
            key,
            context_length,
        })
    }

    pub fn openai(
        provider: &crate::providers::ProviderConnection,
        key: String,
        model: &crate::providers::RemoteModel,
    ) -> Result<Self, String> {
        let base_url = crate::providers::validate_base_url(&provider.base_url)?;
        let context_length = model.context_length.ok_or_else(|| format!("Set a context capacity for remote model '{}' before sending a message. LocalLM will not invent a provider limit.", model.id))?;
        let client = crate::providers::client()?;
        Ok(Self::OpenAi {
            client,
            base_url,
            key,
            model_id: model.id.clone(),
            context_length,
            max_output_tokens: model.max_output_tokens,
            supports_tools: !matches!(
                model.tool_support,
                crate::providers::ToolSupport::Unsupported
            ),
        })
    }

    pub fn anthropic(
        provider: &crate::providers::ProviderConnection,
        key: String,
        model: &crate::providers::RemoteModel,
    ) -> Result<Self, String> {
        let base_url = crate::providers::validate_base_url(&provider.base_url)?;
        let context_length = model.context_length.ok_or_else(|| format!("Set a context capacity for remote model '{}' before sending a message. LocalLM will not invent a provider limit.", model.id))?;
        let client = crate::providers::client()?;
        Ok(Self::Anthropic {
            client,
            base_url,
            key,
            model_id: model.id.clone(),
            context_length,
            max_output_tokens: model.max_output_tokens,
        })
    }

    pub fn validate_response_tokens(&self, response_tokens: u32) -> Result<(), String> {        if let Some(limit) = self.response_limit() {
            if response_tokens > limit {
                return Err(format!("Maximum response tokens ({response_tokens}) exceed this model's configured output limit ({limit}). Lower the response limit in Models & runtime."));
            }
        }
        Ok(())
    }

    pub fn stream_error(&self, value: &Value) -> String {
        let key = match self {
            Self::Local { key, .. } | Self::OpenAi { key, .. } | Self::Anthropic { key, .. } => key,
        };
        format!(
            "Model error: {}",
            crate::providers::sanitize_text(&value.to_string(), key)
        )
    }
}

/// Translate an OpenAI-shape turn (system + tool_calls history) into an
/// Anthropic Messages request. Assistant `tool_calls` become `tool_use`
/// blocks; `tool` messages become `tool_result` blocks on a user turn.
pub fn anthropic_payload(payload: &Value, model_id: &str) -> Value {
    let mut system_parts = Vec::new();
    let mut messages = Vec::new();
    for message in payload.get("messages").and_then(|value| value.as_array()).cloned().unwrap_or_default() {
        let role = message.get("role").and_then(|value| value.as_str()).unwrap_or("user");
        if role == "system" {
            if let Some(text) = message.get("content").and_then(|value| value.as_str()) {
                system_parts.push(text.to_string());
            }
            continue;
        }
        if role == "tool" {
            let tool_use_id = message.get("tool_call_id").and_then(|value| value.as_str()).unwrap_or("unknown");
            let content = message.get("content").and_then(|value| value.as_str()).unwrap_or("");
            messages.push(serde_json::json!({"role": "user", "content": [{"type": "tool_result", "tool_use_id": tool_use_id, "content": content}]}));
            continue;
        }
        if role == "assistant" {
            let mut blocks = Vec::new();
            if let Some(text) = message.get("content").and_then(|value| value.as_str()) {
                if !text.is_empty() {
                    blocks.push(serde_json::json!({"type": "text", "text": text}));
                }
            }
            for call in message.get("tool_calls").and_then(|value| value.as_array()).cloned().unwrap_or_default() {
                let name = call.get("function").and_then(|f| f.get("name")).and_then(|value| value.as_str()).unwrap_or("unknown");
                let id = call.get("id").and_then(|value| value.as_str()).unwrap_or("unknown");
                let input: Value = call
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(|value| value.as_str())
                    .and_then(|text| serde_json::from_str(text).ok())
                    .unwrap_or(serde_json::json!({}));
                blocks.push(serde_json::json!({"type": "tool_use", "id": id, "name": name, "input": input}));
            }
            if blocks.is_empty() {
                blocks.push(serde_json::json!({"type": "text", "text": ""}));
            }
            messages.push(serde_json::json!({"role": "assistant", "content": blocks}));
            continue;
        }
        messages.push(serde_json::json!({"role": "user", "content": message.get("content").cloned().unwrap_or(Value::Null)}));
    }
    let tools: Vec<Value> = payload
        .get("tools")
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|tool| {
            let function = tool.get("function").unwrap_or(&tool);
            serde_json::json!({
                "name": function.get("name"),
                "description": function.get("description"),
                "input_schema": function.get("parameters").unwrap_or(&serde_json::json!({"type": "object"})),
            })
        })
        .collect();
    let mut request = serde_json::json!({
        "model": model_id,
        "max_tokens": payload.get("max_tokens").cloned().unwrap_or(serde_json::json!(1024)),
        "stream": true,
        "messages": messages,
    });
    if !system_parts.is_empty() {
        request["system"] = Value::String(system_parts.join("\n\n"));
    }
    if !tools.is_empty() {
        request["tools"] = Value::Array(tools);
    }
    if let Some(temperature) = payload.get("temperature") {
        request["temperature"] = temperature.clone();
    }
    request
}

/// Translate one Anthropic SSE event (the `event:` + `data:` pair, without
/// the trailing blank line) into OpenAI-style `data:` chunk(s). Returns None
/// for heartbeats the loop must skip.
pub fn translate_anthropic_event(event: &str) -> Option<String> {
    let mut kind = "";
    let mut data = "";
    for line in event.lines() {
        if let Some(rest) = line.strip_prefix("event:") {
            kind = rest.trim();
        } else if let Some(rest) = line.strip_prefix("data:") {
            data = rest.trim();
        }
    }
    if data.is_empty() {
        return None;
    }
    match kind {
        "content_block_start" => {
            let value: Value = serde_json::from_str(data).ok()?;
            let block = value.get("content_block")?;
            if block.get("type")?.as_str()? != "tool_use" {
                return None;
            }
            let index = value.get("index")?.as_u64()?;
            let id = block.get("id")?.as_str().unwrap_or("unknown");
            let name = block.get("name")?.as_str().unwrap_or("unknown");
            Some(format!("data: {}\n\n", serde_json::json!({"choices": [{"delta": {
                "tool_calls": [{"index": index, "id": id, "function": {"name": name, "arguments": ""}}]
            }, "finish_reason": Value::Null}]})))
        }
        "content_block_delta" => {
            let value: Value = serde_json::from_str(data).ok()?;
            let delta = value.get("delta")?;
            if let Some(text) = delta.get("text").and_then(|value| value.as_str()) {
                return Some(format!("data: {}\n\n", serde_json::json!({"choices": [{"delta": {"content": text}, "finish_reason": Value::Null}]})));
            }
            if delta.get("type")?.as_str()? == "input_json_delta" {
                let index = value.get("index")?.as_u64()?;
                let partial = delta.get("partial_json")?.as_str().unwrap_or("");
                return Some(format!("data: {}\n\n", serde_json::json!({"choices": [{"delta": {
                    "tool_calls": [{"index": index, "function": {"arguments": partial}}]
                }, "finish_reason": Value::Null}]})));
            }
            None
        }
        "message_delta" => {
            let value: Value = serde_json::from_str(data).ok()?;
            let stop = value.get("delta")?.get("stop_reason")?.as_str().unwrap_or("stop");
            let finish = if stop == "tool_use" { "tool_calls" } else { "stop" };
            Some(format!("data: {}\n\n", serde_json::json!({"choices": [{"delta": {}, "finish_reason": finish}]})))
        }
        "message_stop" => Some("data: [DONE]\n\n".to_string()),
        "error" => Some(format!("data: {}\n\n", serde_json::json!({"error": data}))),
        _ => None,
    }
}

fn anthropic_stream(response: reqwest::Response) -> StreamBody {
    let byte_stream = response.bytes_stream();
    boxed(futures_util::stream::unfold(
        (byte_stream, String::new(), false),
        |(mut stream, mut buffer, mut finished)| async move {
            loop {
                if let Some(end) = buffer.find("\n\n") {
                    let event: String = buffer.drain(..end + 2).collect();
                    if let Some(chunk) = translate_anthropic_event(event.trim_end()) {
                        return Some((Ok(Bytes::from(chunk)), (stream, buffer, finished)));
                    }
                    continue;
                }
                if finished {
                    return None;
                }
                match stream.next().await {
                    Some(Ok(bytes)) => buffer.push_str(&String::from_utf8_lossy(&bytes)),
                    Some(Err(error)) => {
                        finished = true;
                        return Some((Err(error.to_string()), (stream, buffer, finished)));
                    }
                    None => {
                        finished = true;
                        let rest = std::mem::take(&mut buffer);
                        if let Some(chunk) = translate_anthropic_event(rest.trim_end()) {
                            return Some((Ok(Bytes::from(chunk)), (stream, buffer, finished)));
                        }
                        return Some((Ok(Bytes::from("data: [DONE]\n\n")), (stream, buffer, finished)));
                    }
                }
            }
        },
    ))
}

#[async_trait]
impl InferenceProvider for Backend {
    fn payload(&self, payload: &Value) -> Value {
        match self {
            Self::Local { .. } => payload.clone(),
            Self::OpenAi { model_id, .. } => {
                let mut payload = payload.clone();
                if let Some(object) = payload.as_object_mut() {
                    object.remove("cache_prompt");
                    object.insert("model".into(), Value::String(model_id.clone()));
                }
                payload
            }
            Self::Anthropic { model_id, .. } => anthropic_payload(payload, model_id),
        }
    }

    fn context_capacity(&self) -> u32 {
        match self {
            Self::Local { context_length, .. }
            | Self::OpenAi { context_length, .. }
            | Self::Anthropic { context_length, .. } => *context_length,
        }
    }

    fn response_limit(&self) -> Option<u32> {
        match self {
            Self::Local { .. } => None,
            Self::OpenAi { max_output_tokens, .. } | Self::Anthropic { max_output_tokens, .. } => {
                *max_output_tokens
            }
        }
    }

    fn supports_tools(&self) -> bool {
        match self {
            Self::Local { .. } => true,
            Self::Anthropic { .. } => true,
            Self::OpenAi { supports_tools, .. } => *supports_tools,
        }
    }

    fn context_is_estimate(&self) -> bool {
        matches!(self, Self::OpenAi { .. } | Self::Anthropic { .. })
    }

    async fn check_context(&self, payload: &Value, response_tokens: u32) -> Result<u64, String> {
        self.validate_response_tokens(response_tokens)?;
        match self {
            Self::Local {
                client,
                endpoint,
                key,
                context_length,
            } => {
                crate::context::check(
                    client,
                    endpoint,
                    key,
                    payload,
                    response_tokens,
                    *context_length,
                )
                .await
            }
            Self::OpenAi { context_length, .. } | Self::Anthropic { context_length, .. } => {
                let bytes = serde_json::to_vec(payload).map_err(|error| error.to_string())?;
                let estimate = ((bytes.len() as u64).saturating_add(3) / 4).max(1);
                crate::context::validate_budget(estimate, response_tokens, *context_length)?;
                Ok(estimate)
            }
        }
    }

    async fn stream(&self, payload: &Value) -> Result<StreamBody, String> {
        match self {
            Self::Anthropic { client, base_url, key, .. } => {
                let model_id = match self {
                    Self::Anthropic { model_id, .. } => model_id.clone(),
                    _ => unreachable!(),
                };
                let response = client
                    .post(crate::providers::endpoint(base_url, "messages"))
                    .header("x-api-key", key)
                    .header("anthropic-version", "2023-06-01")
                    .json(&anthropic_payload(payload, &model_id))
                    .send()
                    .await
                    .map_err(|error| {
                        if error.is_timeout() {
                            "Provider request timed out.".to_string()
                        } else {
                            "Could not reach provider. Check the HTTPS base URL and network connection.".to_string()
                        }
                    })?;
                if !response.status().is_success() {
                    return Err(crate::providers::response_error(response, key).await);
                }
                Ok(anthropic_stream(response))
            }
            _ => {
                let response = match self {
                    Self::Local { client, endpoint, key, .. } => client
                        .post(crate::providers::endpoint(endpoint, "chat/completions"))
                        .bearer_auth(key)
                        .json(payload)
                        .send()
                        .await
                        .map_err(|_| {
                            "The local model did not respond. Check that llama.cpp is still running.".to_string()
                        })?,
                    Self::OpenAi { client, base_url, key, .. } => client
                        .post(crate::providers::endpoint(base_url, "chat/completions"))
                        .bearer_auth(key)
                        .json(payload)
                        .send()
                        .await
                        .map_err(|error| {
                            if error.is_timeout() {
                                "Provider request timed out.".to_string()
                            } else {
                                "Could not reach provider. Check the HTTPS base URL and network connection.".to_string()
                            }
                        })?,
                    Self::Anthropic { .. } => unreachable!(),
                };
                if response.status().is_success() {
                    let stream = response
                        .bytes_stream()
                        .map(|result| result.map_err(|error| error.to_string()));
                    return Ok(boxed(stream));
                }
                match self {
                    Self::Local { .. } => Err(format!(
                        "Local model request failed ({}). Check the runtime log and context limits.",
                        response.status()
                    )),
                    Self::OpenAi { key, .. } => Err(crate::providers::response_error(response, key).await),
                    Self::Anthropic { .. } => unreachable!(),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{ProviderConnection, RemoteModel, ToolSupport, OPENAI_CHAT_COMPLETIONS};
    use serde_json::json;

    fn provider() -> ProviderConnection {
        ProviderConnection {
            id: "provider".into(),
            name: "Fixture".into(),
            api_format: OPENAI_CHAT_COMPLETIONS.into(),
            base_url: "http://127.0.0.1:1234".into(),
            verified: true,
            last_tested_at: None,
            models: Vec::new(),
            has_api_key: true,
        }
    }

    #[test]
    fn remote_payload_adds_model_and_omits_local_only_parameters() {
        let backend = Backend::openai(
            &provider(),
            "secret".into(),
            &RemoteModel {
                id: "manual".into(),
                context_length: Some(4096),
                max_output_tokens: None,
                tool_support: ToolSupport::Unknown,
            },
        )
        .unwrap();
        let payload = backend.payload(&json!({"cache_prompt":true,"messages":[]}));
        assert_eq!(payload["model"], "manual");
        assert!(payload.get("cache_prompt").is_none());
        assert!(backend.context_is_estimate());
    }

    #[test]
    fn remote_context_and_output_limits_are_explicit() {
        let backend = Backend::openai(
            &provider(),
            "secret".into(),
            &RemoteModel {
                id: "manual".into(),
                context_length: Some(4096),
                max_output_tokens: Some(512),
                tool_support: ToolSupport::Unknown,
            },
        )
        .unwrap();
        assert!(backend.validate_response_tokens(512).is_ok());
        assert!(backend.validate_response_tokens(513).is_err());
        assert!(Backend::openai(
            &provider(),
            "secret".into(),
            &RemoteModel {
                id: "manual".into(),
                context_length: None,
                max_output_tokens: None,
                tool_support: ToolSupport::Unknown
            }
        )
        .is_err());
    }

    #[tokio::test]
    async fn remote_stream_uses_openai_payload_and_preserves_sse_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut buffer = [0; 2048];
                let count = stream.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
                if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        let payload =
                            serde_json::from_slice::<Value>(&request[end + 4..end + 4 + length])
                                .unwrap();
                        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\ndata: [DONE]\n\n";
                        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                        stream.write_all(response.as_bytes()).await.unwrap();
                        return (headers, payload);
                    }
                }
            }
        });

        let mut fixture = provider();
        fixture.base_url = format!("http://{address}");
        let backend = Backend::openai(
            &fixture,
            "fixture-secret".into(),
            &RemoteModel {
                id: "remote-model".into(),
                context_length: Some(4096),
                max_output_tokens: None,
                tool_support: ToolSupport::Supported,
            },
        )
        .unwrap();
        let payload = backend.payload(&json!({"cache_prompt": true, "stream": true, "messages": [{"role": "user", "content": "hello"}]}));
        let mut stream = backend.stream(&payload).await.unwrap();
        use futures_util::StreamExt;
        let mut body = Vec::new();
        while let Some(chunk) = stream.next().await {
            body.extend_from_slice(&chunk.unwrap());
        }
        let body = String::from_utf8(body).unwrap();
        let (headers, received) = server.await.unwrap();
        assert!(headers.starts_with("post /v1/chat/completions "));
        assert!(headers.contains("authorization: bearer fixture-secret"));
        assert_eq!(received["model"], "remote-model");
        assert!(received.get("cache_prompt").is_none());
        assert!(body.contains("hello") && body.contains("[DONE]"));
    }

    #[test]
    fn anthropic_payload_moves_system_and_tools() {
        let request = anthropic_payload(
            &json!({
                "max_tokens": 64,
                "messages": [
                    {"role": "system", "content": "be brief"},
                    {"role": "user", "content": "hi"},
                    {"role": "assistant", "content": "", "tool_calls": [
                        {"id": "t1", "type": "function", "function": {"name": "list_files", "arguments": "{\"path\":\".\"}"}}
                    ]},
                    {"role": "tool", "tool_call_id": "t1", "content": "a.txt"},
                ],
                "tools": [{"type": "function", "function": {"name": "list_files", "description": "list", "parameters": {"type": "object"}}}],
            }),
            "claude-test",
        );
        assert_eq!(request["model"], "claude-test");
        assert_eq!(request["system"], "be brief");
        assert_eq!(request["messages"][1]["content"][0]["type"], "tool_use");
        assert_eq!(request["messages"][2]["content"][0]["type"], "tool_result");
        assert_eq!(request["tools"][0]["input_schema"]["type"], "object");
    }

    #[test]
    fn anthropic_events_translate_text_tool_use_and_stop() {
        let text = translate_anthropic_event("event: content_block_delta\ndata: {\"delta\":{\"type\":\"text_delta\",\"text\":\"hel\"}}").unwrap();
        assert!(text.contains("\"content\":\"hel\""));
        let start = translate_anthropic_event("event: content_block_start\ndata: {\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"t1\",\"name\":\"list_files\"}}").unwrap();
        assert!(start.contains("\"name\":\"list_files\"") && start.contains("\"index\":0"));
        let partial = translate_anthropic_event("event: content_block_delta\ndata: {\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\"}}").unwrap();
        assert!(partial.contains("path"));
        let stop = translate_anthropic_event("event: message_delta\ndata: {\"delta\":{\"stop_reason\":\"tool_use\"}}").unwrap();
        assert!(stop.contains("tool_calls"));
        assert!(translate_anthropic_event("event: ping\ndata: {}").is_none());
        assert_eq!(translate_anthropic_event("event: message_stop\ndata: {}").unwrap(), "data: [DONE]\n\n");
    }
}
