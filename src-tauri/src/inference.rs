use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;

/// Typed inference failure so the chat loop can pick a recovery path instead
/// of parsing message text. `Display`/`From` keep `Result<_, String>` call
/// sites working; only code that reacts to the kind matches on the enum.
#[derive(Debug)]
pub enum LlmError {
    /// The prompt does not fit the loaded context window (llama.cpp
    /// `exceed_context_size`, OpenAI `context_length_exceeded`, …).
    ContextOverflow(String),
    Timeout(String),
    RateLimit { message: String, retry_after: Option<Duration> },
    /// Connect, DNS, or body-read failures — the server may have stopped.
    Transport(String),
    /// The stream completed cleanly but produced no content or tool calls.
    EmptyResponse,
    Other(String),
}

impl LlmError {
    /// Re-sending the identical request can succeed. Context overflow only
    /// recovers through compaction, which the caller drives explicitly.
    pub fn retryable(&self) -> bool {
        matches!(self, Self::Transport(_) | Self::Timeout(_) | Self::RateLimit { .. })
    }

    /// A reqwest send/read failure: an elapsed deadline is a timeout;
    /// connect, DNS, and body failures are transport problems.
    pub fn from_reqwest(error: &reqwest::Error, timeout: &'static str, transport: impl Into<String>) -> Self {
        if error.is_timeout() {
            Self::Timeout(timeout.into())
        } else {
            Self::Transport(transport.into())
        }
    }

    /// Classify a non-success HTTP response. `detail` is the raw response body
    /// when the caller already read it; `message` is the sanitized
    /// user-facing text the error should display.
    pub fn from_response(status: reqwest::StatusCode, retry_after: Option<Duration>, detail: &str, message: String) -> Self {
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Self::RateLimit { message, retry_after };
        }
        if Self::context_wording(detail) || Self::context_wording(&message) {
            return Self::ContextOverflow(message);
        }
        Self::Other(message)
    }

    /// Classify a provider error surfaced mid-stream from the SSE body.
    pub fn from_message(message: String) -> Self {
        if Self::context_wording(&message) {
            Self::ContextOverflow(message)
        } else {
            Self::Other(message)
        }
    }

    /// Wording llama.cpp, OpenAI-compatible servers, and Anthropic use when a
    /// prompt does not fit the context window.
    pub fn context_wording(text: &str) -> bool {
        let text = text.to_ascii_lowercase();
        const MARKERS: &[&str] = &[
            "context_length_exceeded",
            "exceed_context_size",
            "n_ctx",
            "context window",
            "context size",
            "context length",
            "maximum context",
            "too many tokens",
            "prompt is too long",
            "input is too long",
        ];
        MARKERS.iter().any(|marker| text.contains(marker))
    }
}

impl std::fmt::Display for LlmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ContextOverflow(message)
            | Self::Timeout(message)
            | Self::Transport(message)
            | Self::Other(message)
            | Self::RateLimit { message, .. } => f.write_str(message),
            Self::EmptyResponse => {
                f.write_str("The model stream ended without producing a response.")
            }
        }
    }
}

impl std::error::Error for LlmError {}

impl From<LlmError> for String {
    fn from(error: LlmError) -> String {
        error.to_string()
    }
}

/// `Retry-After` in seconds. The HTTP-date form is ignored — the providers
/// this sees (llama.cpp, OpenAI-compatible, subscriptions) send seconds.
fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
}

#[async_trait]
pub trait InferenceProvider: Send + Sync {
    fn payload(&self, payload: &Value) -> Value;
    fn context_capacity(&self) -> u32;
    fn response_limit(&self) -> Option<u32>;
    fn supports_tools(&self) -> bool;
    fn context_is_estimate(&self) -> bool;
    async fn check_context(&self, payload: &Value, response_tokens: u32) -> Result<u64, String>;
    /// Count payload tokens without enforcing the context budget. Local
    /// runtimes return the exact tokenizer count; remote providers return a
    /// character-derived estimate.
    async fn count_tokens(&self, payload: &Value) -> Result<u64, String>;
    async fn stream(&self, payload: &Value) -> Result<reqwest::Response, LlmError>;
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
    /// Native Anthropic Messages API (`POST {base}/v1/messages`). Selected
    /// explicitly per connection — never inferred from an OpenAI-compatible
    /// endpoint. Credentials stay in this native provider layer.
    Claude {
        client: reqwest::Client,
        base_url: String,
        key: String,
        model_id: String,
        context_length: u32,
        max_output_tokens: Option<u32>,
        supports_tools: bool,
    },
    Subscription {
        client: reqwest::Client,
        provider_name: String,
        base_url: String,
        access_token: String,
        account_id: Option<String>,
        model_id: String,
        context_length: u32,
        max_output_tokens: Option<u32>,
        supports_tools: bool,
    },
}

impl Backend {
    pub fn local(endpoint: String, key: String, context_length: u32) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(600))
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
        if crate::providers::is_claude_format(&provider.api_format) {
            return Self::claude(provider, key, model);
        }
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

    /// Native Claude Messages backend. Implements the actual Anthropic
    /// request, streaming, tool-result, and stop-reason protocol (see
    /// `claude_adapter`); the chat loop normalizes its events into the
    /// internal contract. Requires an explicit `claude-messages` connection —
    /// a consumer subscription is NOT supported API access.
    pub fn claude(
        provider: &crate::providers::ProviderConnection,
        key: String,
        model: &crate::providers::RemoteModel,
    ) -> Result<Self, String> {
        if key.trim().is_empty() {
            return Err("Save an API key for the Claude provider before sending a message. Consumer subscriptions do not provide supported API access.".into());
        }
        let base_url = crate::providers::validate_base_url(&provider.base_url)?;
        let context_length = model.context_length.ok_or_else(|| format!("Set a context capacity for remote model '{}' before sending a message. LocalLM will not invent a provider limit.", model.id))?;
        let client = crate::providers::client()?;
        Ok(Self::Claude {
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

    pub fn subscription(
        provider_name: &str,
        base_url: &str,
        access_token: String,
        account_id: Option<String>,
        model: &crate::providers::RemoteModel,
    ) -> Result<Self, String> {
        let base_url = crate::providers::validate_base_url(base_url)?;
        let context_length = model.context_length.unwrap_or(128_000);
        let client = crate::providers::client()?;
        Ok(Self::Subscription {
            client,
            provider_name: provider_name.to_string(),
            base_url,
            access_token,
            account_id,
            model_id: model.id.clone(),
            context_length,
            max_output_tokens: model.max_output_tokens,
            supports_tools: !matches!(
                model.tool_support,
                crate::providers::ToolSupport::Unsupported
            ),
        })
    }

    pub fn validate_response_tokens(&self, response_tokens: u32) -> Result<(), String> {
        if let Some(limit) = self.response_limit() {
            if response_tokens > limit {
                return Err(format!("Maximum response tokens ({response_tokens}) exceed this model's configured output limit ({limit}). Lower the response limit in Models & runtime."));
            }
        }
        Ok(())
    }

    pub fn stream_error(&self, value: &Value) -> String {
        let key = match self {
            Self::Local { key, .. } | Self::OpenAi { key, .. } | Self::Claude { key, .. } => key,
            Self::Subscription { access_token, .. } => access_token,
        };
        format!(
            "Model error: {}",
            crate::providers::sanitize_text(&value.to_string(), key)
        )
    }

    pub fn usage(&self) -> Option<crate::providers::ProviderUsage> {
        None
    }
}

#[async_trait]
impl InferenceProvider for Backend {
    fn payload(&self, payload: &Value) -> Value {
        match self {
            Self::Local { .. } => payload.clone(),
            Self::Claude { model_id, max_output_tokens, .. } => {
                crate::claude_adapter::to_messages_request(payload, model_id, *max_output_tokens)
            }
            Self::OpenAi { model_id, .. } | Self::Subscription { model_id, .. } => {
                let mut payload = payload.clone();
                if let Some(object) = payload.as_object_mut() {
                    object.remove("cache_prompt");
                    object.remove("id_slot");
                    // repeat_penalty is a llama.cpp extension — some
                    // OpenAI-compatible endpoints reject unknown fields.
                    object.remove("repeat_penalty");
                    object.insert("model".into(), Value::String(model_id.clone()));
                }
                payload
            }
        }
    }

    fn context_capacity(&self) -> u32 {
        match self {
            Self::Local { context_length, .. }
            | Self::OpenAi { context_length, .. }
            | Self::Claude { context_length, .. }
            | Self::Subscription { context_length, .. } => *context_length,
        }
    }

    fn response_limit(&self) -> Option<u32> {
        match self {
            Self::Local { .. } => None,
            Self::OpenAi {
                max_output_tokens, ..
            }
            | Self::Claude {
                max_output_tokens, ..
            }
            | Self::Subscription {
                max_output_tokens, ..
            } => *max_output_tokens,
        }
    }

    fn supports_tools(&self) -> bool {
        match self {
            Self::Local { .. } => true,
            Self::OpenAi { supports_tools, .. }
            | Self::Claude { supports_tools, .. }
            | Self::Subscription { supports_tools, .. } => *supports_tools,
        }
    }

    fn context_is_estimate(&self) -> bool {
        matches!(self, Self::OpenAi { .. } | Self::Claude { .. } | Self::Subscription { .. })
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
            Self::OpenAi { context_length, .. } | Self::Claude { context_length, .. } | Self::Subscription { context_length, .. } => {
                let estimate = self.count_tokens(payload).await?;
                crate::context::validate_budget(estimate, response_tokens, *context_length)?;
                Ok(estimate)
            }
        }
    }

    async fn count_tokens(&self, payload: &Value) -> Result<u64, String> {
        match self {
            Self::Local {
                client,
                endpoint,
                key,
                ..
            } => {
                let mut payload = payload.clone();
                if let Some(object) = payload.as_object_mut() {
                    object.remove("cache_prompt");
                    object.remove("id_slot");
                }
                crate::context::count_only(client, endpoint, key, &payload).await
            }
            Self::OpenAi { .. } | Self::Claude { .. } | Self::Subscription { .. } => {
                let bytes = serde_json::to_vec(payload).map_err(|error| error.to_string())?;
                Ok(((bytes.len() as u64).saturating_add(3) / 4).max(1))
            }
        }
    }

    async fn stream(&self, payload: &Value) -> Result<reqwest::Response, LlmError> {
        match self {
            Self::Local {
                client,
                endpoint,
                key,
                ..
            } => {
                let mut request = payload.clone();
                request["stream_options"] = serde_json::json!({"include_usage": true});
                let response = client
                    .post(crate::providers::endpoint(endpoint, "chat/completions"))
                    .bearer_auth(key)
                    .json(&request)
                    .send()
                    .await
                    .map_err(|error| {
                        LlmError::from_reqwest(
                            &error,
                            "The local model did not respond in time. Check that llama.cpp is still running.",
                            "The local model did not respond. Check that llama.cpp is still running.",
                        )
                    })?;
                if response.status().is_success() {
                    Ok(response)
                } else {
                    let status = response.status();
                    let retry_after = parse_retry_after(response.headers());
                    let body = crate::providers::bounded_body(response, 16_384)
                        .await
                        .unwrap_or_default();
                    Err(LlmError::from_response(
                        status,
                        retry_after,
                        &body,
                        crate::context::runtime_prompt_error(status, &body, key),
                    ))
                }
            }
            Self::OpenAi {
                client,
                base_url,
                key,
                ..
            } => {
                let mut request = client
                    .post(crate::providers::endpoint(base_url, "chat/completions"))
                    .json(payload);
                // Keyless loopback engines must not receive an empty bearer.
                if !key.is_empty() {
                    request = request.bearer_auth(key);
                }
                let response = request.send().await.map_err(|error| {
                    LlmError::from_reqwest(
                        &error,
                        "Provider request timed out.",
                        "Could not reach provider. Check the HTTPS base URL and network connection.",
                    )
                })?;
                if response.status().is_success() {
                    Ok(response)
                } else {
                    let status = response.status();
                    let retry_after = parse_retry_after(response.headers());
                    let message = crate::providers::response_error(response, key).await;
                    Err(LlmError::from_response(status, retry_after, "", message))
                }
            }
            Self::Claude {
                client,
                base_url,
                key,
                ..
            } => {
                // Native Messages endpoint: {base}/v1/messages, x-api-key auth.
                let url = format!("{}/v1/messages", base_url.trim_end_matches('/'));
                let response = client
                    .post(url)
                    .header("x-api-key", key)
                    .header("anthropic-version", crate::claude_adapter::ANTHROPIC_VERSION)
                    .header("User-Agent", "LocalLM-Desktop/0.1")
                    .json(payload)
                    .send()
                    .await
                    .map_err(|error| {
                        LlmError::from_reqwest(
                            &error,
                            "Claude request timed out.",
                            "Could not reach Claude. Check the HTTPS base URL and network connection.",
                        )
                    })?;
                if response.status().is_success() {
                    Ok(response)
                } else {
                    let status = response.status();
                    let retry_after = parse_retry_after(response.headers());
                    let message = crate::providers::response_error(response, key).await;
                    Err(LlmError::from_response(status, retry_after, "", message))
                }
            }
            Self::Subscription {
                client,
                provider_name,
                base_url,
                access_token,
                account_id,
                ..
            } => {
                let mut req = client
                    .post(crate::providers::endpoint(base_url, "chat/completions"))
                    .bearer_auth(access_token)
                    .header("User-Agent", "LocalLM-Desktop/0.1")
                    .json(payload);

                if let Some(acct) = account_id {
                    req = req.header("OpenAI-Account-Id", acct);
                }

                let response = req.send().await.map_err(|error| {
                    LlmError::from_reqwest(
                        &error,
                        "Subscription provider request timed out.",
                        format!("Could not reach {provider_name} subscription service. Check network connection."),
                    )
                })?;

                let status = response.status();
                if status.is_success() {
                    return Ok(response);
                }
                let retry_after = parse_retry_after(response.headers());

                if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    return Err(LlmError::RateLimit {
                        message: format!(
                            "{provider_name} subscription rate limit or quota reached. Your rolling quota was exceeded. Try again later or switch to your local model."
                        ),
                        retry_after,
                    });
                }
                if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
                    return Err(LlmError::Other(format!(
                        "{provider_name} subscription authorization expired. Re-authenticate in Providers."
                    )));
                }
                let message = crate::providers::response_error(response, access_token).await;
                Err(LlmError::from_response(status, retry_after, "", message))
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
                supports_images: false, tool_support: ToolSupport::Unknown,
            },
        )
        .unwrap();
        let payload = backend.payload(&json!({"cache_prompt":true,"messages":[]}));
        assert_eq!(payload["model"], "manual");
        assert!(payload.get("cache_prompt").is_none());
        assert!(payload.get("id_slot").is_none());
        assert!(backend.context_is_estimate());
    }

    #[test]
    fn subscription_backend_configures_expected_properties() {
        let backend = Backend::subscription(
            "ChatGPT",
            "https://chatgpt.com/backend-api/codex",
            "token-123".into(),
            Some("acct-456".into()),
            &RemoteModel {
                id: "gpt-4o".into(),
                context_length: Some(128_000),
                max_output_tokens: Some(16_384),
                supports_images: false, tool_support: ToolSupport::Supported,
            },
        )
        .unwrap();

        assert_eq!(backend.context_capacity(), 128_000);
        assert_eq!(backend.response_limit(), Some(16_384));
        assert!(backend.supports_tools());
        assert!(backend.context_is_estimate());

        let payload = backend.payload(&json!({"cache_prompt": true, "id_slot": 0, "messages": []}));
        assert_eq!(payload["model"], "gpt-4o");
        assert!(payload.get("cache_prompt").is_none());
        assert!(payload.get("id_slot").is_none());
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
                supports_images: false, tool_support: ToolSupport::Unknown,
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
                supports_images: false, tool_support: ToolSupport::Unknown
            }
        )
        .is_err());
    }

    #[test]
    fn claude_backend_uses_messages_protocol_with_versioned_auth() {
        use crate::providers::CLAUDE_MESSAGES;
        let mut fixture = provider();
        fixture.api_format = CLAUDE_MESSAGES.into();
        fixture.base_url = "https://api.anthropic.com".into();
        let backend = Backend::openai(
            &fixture,
            "claude-key".into(),
            &RemoteModel {
                id: "claude-model".into(),
                context_length: Some(200_000),
                max_output_tokens: Some(4096),
                supports_images: false, tool_support: ToolSupport::Supported,
            },
        )
        .unwrap();
        assert!(backend.context_is_estimate());
        assert_eq!(backend.context_capacity(), 200_000);
        let payload = backend.payload(&json!({
            "messages": [
                {"role": "system", "content": "sys"},
                {"role": "user", "content": "hi"},
            ],
            "max_tokens": 256,
            "stream": true,
            "cache_prompt": true,
        }));
        assert_eq!(payload["model"], "claude-model");
        assert_eq!(payload["system"], "sys");
        assert_eq!(payload["stream"], true);
        assert!(payload.get("cache_prompt").is_none());
        assert!(payload["messages"].as_array().unwrap().iter().all(|message| message["role"] != "system"));
        // No key, no backend: consumer subscriptions are not API access.
        assert!(Backend::openai(
            &fixture,
            String::new(),
            &RemoteModel {
                id: "claude-model".into(),
                context_length: Some(200_000),
                max_output_tokens: None,
                supports_images: false, tool_support: ToolSupport::Supported,
            },
        )
        .is_err());
    }

    #[tokio::test]
    async fn claude_stream_hits_messages_endpoint_with_native_headers() {
        use crate::providers::CLAUDE_MESSAGES;
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
                        let body = "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n";
                        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                        stream.write_all(response.as_bytes()).await.unwrap();
                        return (headers, payload);
                    }
                }
            }
        });

        let mut fixture = provider();
        fixture.api_format = CLAUDE_MESSAGES.into();
        fixture.base_url = format!("http://{address}");
        let backend = Backend::openai(
            &fixture,
            "claude-key".into(),
            &RemoteModel {
                id: "claude-model".into(),
                context_length: Some(200_000),
                max_output_tokens: None,
                supports_images: false, tool_support: ToolSupport::Supported,
            },
        )
        .unwrap();
        let payload = backend.payload(&json!({"messages": [{"role": "user", "content": "hi"}], "max_tokens": 64, "stream": true}));
        let response = backend.stream(&payload).await.unwrap();
        let body = response.text().await.unwrap();
        let (headers, received) = server.await.unwrap();
        assert!(headers.starts_with("post /v1/messages "), "{headers}");
        assert!(headers.contains("x-api-key: claude-key"), "{headers}");
        assert!(headers.contains("anthropic-version: 2023-06-01"), "{headers}");
        assert!(!headers.contains("authorization: bearer"), "Claude must not receive OpenAI bearer auth: {headers}");
        assert_eq!(received["model"], "claude-model");
        assert!(body.contains("text_delta") && body.contains("end_turn"));
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
                supports_images: false, tool_support: ToolSupport::Supported,
            },
        )
        .unwrap();
        let payload = backend.payload(&json!({"cache_prompt": true, "stream": true, "messages": [{"role": "user", "content": "hello"}]}));
        let response = backend.stream(&payload).await.unwrap();
        let body = response.text().await.unwrap();
        let (headers, received) = server.await.unwrap();
        assert!(headers.starts_with("post /v1/chat/completions "));
        assert!(headers.contains("authorization: bearer fixture-secret"));
        assert_eq!(received["model"], "remote-model");
        assert!(received.get("cache_prompt").is_none());
        assert!(body.contains("hello") && body.contains("[DONE]"));
    }

    #[tokio::test]
    async fn keyless_loopback_engine_receives_no_authorization_header() {
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
                        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\ndata: [DONE]\n\n";
                        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body);
                        stream.write_all(response.as_bytes()).await.unwrap();
                        return headers;
                    }
                }
            }
        });

        let mut fixture = provider();
        fixture.base_url = format!("http://{address}");
        let backend = Backend::openai(
            &fixture,
            String::new(),
            &RemoteModel {
                id: "loopback-model".into(),
                context_length: Some(4096),
                max_output_tokens: None,
                supports_images: false, tool_support: ToolSupport::Supported,
            },
        )
        .unwrap();
        let payload = backend.payload(&json!({"stream": true, "messages": [{"role": "user", "content": "hello"}]}));
        let response = backend.stream(&payload).await.unwrap();
        let body = response.text().await.unwrap();
        let headers = server.await.unwrap();
        assert!(!headers.contains("authorization"), "keyless loopback requests must not carry an empty bearer: {headers}");
        assert!(body.contains("hi"));
    }

    #[tokio::test]
    async fn local_stream_failure_includes_chat_template_error() {
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
                        let body = r#"{"error":{"message":"System message must be at the beginning."}}"#;
                        let response = format!(
                            "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(response.as_bytes()).await;
                        return;
                    }
                }
            }
        });
        let backend = Backend::local(format!("http://{address}"), "local-key".into(), 8192).unwrap();
        let error = backend
            .stream(&json!({"messages":[{"role":"system","content":"a"},{"role":"system","content":"b"}]}))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("System message must be at the beginning"));
        server.await.unwrap();
    }

    #[test]
    fn retryability_depends_on_failure_kind() {
        assert!(LlmError::Transport("lost".into()).retryable());
        assert!(LlmError::Timeout("late".into()).retryable());
        assert!(LlmError::RateLimit { message: "slow".into(), retry_after: None }.retryable());
        // Context overflow retries only through the explicit compact path.
        assert!(!LlmError::ContextOverflow("full".into()).retryable());
        assert!(!LlmError::EmptyResponse.retryable());
        assert!(!LlmError::Other("bad".into()).retryable());
        // The String conversion keeps `?`/`map_err` call sites unchanged.
        let text: String = LlmError::Transport("lost".into()).into();
        assert_eq!(text, "lost");
        assert_eq!(
            LlmError::EmptyResponse.to_string(),
            "The model stream ended without producing a response."
        );
    }

    #[test]
    fn http_failures_classify_by_status_and_provider_wording() {
        use reqwest::StatusCode;
        match LlmError::from_response(
            StatusCode::TOO_MANY_REQUESTS,
            Some(Duration::from_secs(7)),
            "",
            "Provider rate limit or quota was exceeded.".into(),
        ) {
            LlmError::RateLimit { message, retry_after } => {
                assert_eq!(message, "Provider rate limit or quota was exceeded.");
                assert_eq!(retry_after, Some(Duration::from_secs(7)));
            }
            other => panic!("expected RateLimit, got {other:?}"),
        }
        for wording in [
            // llama.cpp `exceed_context_size_error` message and type fields
            "the request exceeds the available context size. try increasing it",
            r#"{"error":{"code":400,"type":"exceed_context_size_error"}}"#,
            // OpenAI `context_length_exceeded`
            "This model's maximum context length is 8192 tokens",
            // Anthropic 400 invalid_request_error
            "prompt is too long: 200001 tokens > 200000 maximum",
            "n_ctx: 8192",
        ] {
            assert!(
                matches!(
                    LlmError::from_response(StatusCode::BAD_REQUEST, None, wording, "Provider request failed (400).".into()),
                    LlmError::ContextOverflow(_)
                ),
                "{wording}"
            );
        }
        assert!(matches!(
            LlmError::from_response(StatusCode::INTERNAL_SERVER_ERROR, None, "CUDA error", "Provider returned 500.".into()),
            LlmError::Other(_)
        ));
        assert!(matches!(
            LlmError::from_response(StatusCode::UNAUTHORIZED, None, "", "Provider rejected the authorization or API key.".into()),
            LlmError::Other(_)
        ));
        // A mid-stream error event with context wording is also an overflow.
        assert!(matches!(
            LlmError::from_message("Model error: the request exceeds the available context size".into()),
            LlmError::ContextOverflow(_)
        ));
        assert!(matches!(
            LlmError::from_message("Model error: internal error".into()),
            LlmError::Other(_)
        ));
    }

    #[tokio::test]
    async fn reqwest_failures_split_timeout_from_transport() {
        // Refused connection: connect error → Transport.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let error = reqwest::Client::builder()
            .no_proxy()
            .build()
            .unwrap()
            .get(format!("http://127.0.0.1:{port}/"))
            .send()
            .await
            .unwrap_err();
        assert!(matches!(
            LlmError::from_reqwest(&error, "timed out", "unreachable"),
            LlmError::Transport(_)
        ));
        // A socket that accepts but never answers: elapsed deadline → Timeout.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let hold = tokio::spawn(async move {
            let _accepted = listener.accept().await;
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        let error = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(100))
            .build()
            .unwrap()
            .get(format!("http://{address}"))
            .send()
            .await
            .unwrap_err();
        assert!(error.is_timeout());
        assert!(matches!(
            LlmError::from_reqwest(&error, "timed out", "unreachable"),
            LlmError::Timeout(_)
        ));
        hold.abort();
    }

    #[tokio::test]
    async fn local_stream_failures_are_typed() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        for (status_line, body, extra_headers, expect) in [
            (
                "400 Bad Request",
                r#"{"error":{"code":400,"message":"the request exceeds the available context size. try increasing it","type":"exceed_context_size_error"}}"#,
                "",
                "overflow",
            ),
            ("429 Too Many Requests", "{}", "Retry-After: 30\r\n", "rate_limit"),
            ("500 Internal Server Error", r#"{"error":{"message":"decode failed"}}"#, "", "other"),
        ] {
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
                            let sent: Value = serde_json::from_slice(&request[end + 4..end + 4 + length]).unwrap();
                            assert_eq!(sent["stream_options"]["include_usage"], true);
                            let response = format!(
                                "HTTP/1.1 {status_line}\r\nContent-Length: {}\r\n{extra_headers}Connection: close\r\n\r\n{body}",
                                body.len()
                            );
                            let _ = stream.write_all(response.as_bytes()).await;
                            return;
                        }
                    }
                }
            });
            let backend = Backend::local(format!("http://{address}"), "local-key".into(), 8192).unwrap();
            let error = backend
                .stream(&json!({"messages":[{"role":"user","content":"hi"}],"stream":true}))
                .await
                .unwrap_err();
            match expect {
                "overflow" => assert!(matches!(error, LlmError::ContextOverflow(_)), "{error:?}"),
                "rate_limit" => match error {
                    LlmError::RateLimit { retry_after, .. } => {
                        assert_eq!(retry_after, Some(Duration::from_secs(30)))
                    }
                    other => panic!("expected RateLimit, got {other:?}"),
                },
                _ => assert!(matches!(error, LlmError::Other(_)), "{error:?}"),
            }
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn local_stream_refused_connection_is_transport() {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let backend = Backend::local(format!("http://127.0.0.1:{port}"), "local-key".into(), 8192).unwrap();
        let error = backend
            .stream(&json!({"messages":[{"role":"user","content":"hi"}],"stream":true}))
            .await
            .unwrap_err();
        assert!(matches!(error, LlmError::Transport(_)), "{error:?}");
    }
}
