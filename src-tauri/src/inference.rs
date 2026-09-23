use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;

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
    async fn stream(&self, payload: &Value) -> Result<reqwest::Response, String>;
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

    async fn stream(&self, payload: &Value) -> Result<reqwest::Response, String> {
        match self {
            Self::Local {
                client,
                endpoint,
                key,
                ..
            } => {
                let response = client
                    .post(crate::providers::endpoint(endpoint, "chat/completions"))
                    .bearer_auth(key)
                    .json(payload)
                    .send()
                    .await
                    .map_err(|_| {
                        "The local model did not respond. Check that llama.cpp is still running."
                            .to_string()
                    })?;
                if response.status().is_success() {
                    Ok(response)
                } else {
                    let status = response.status();
                    let body = crate::providers::bounded_body(response, 16_384)
                        .await
                        .unwrap_or_default();
                    Err(crate::context::runtime_prompt_error(status, &body, key))
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
                    if error.is_timeout() {
                        "Provider request timed out.".to_string()
                    } else {
                        "Could not reach provider. Check the HTTPS base URL and network connection."
                            .to_string()
                    }
                })?;
                if response.status().is_success() {
                    Ok(response)
                } else {
                    Err(crate::providers::response_error(response, key).await)
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
                        if error.is_timeout() {
                            "Claude request timed out.".to_string()
                        } else {
                            "Could not reach Claude. Check the HTTPS base URL and network connection."
                                .to_string()
                        }
                    })?;
                if response.status().is_success() {
                    Ok(response)
                } else {
                    Err(crate::providers::response_error(response, key).await)
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
                    if error.is_timeout() {
                        "Subscription provider request timed out.".to_string()
                    } else {
                        format!("Could not reach {provider_name} subscription service. Check network connection.")
                    }
                })?;

                let status = response.status();
                if status.is_success() {
                    return Ok(response);
                }

                if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    return Err(format!(
                        "{provider_name} subscription rate limit or quota reached. Your rolling quota was exceeded. Try again later or switch to your local model."
                    ));
                }
                if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
                    return Err(format!(
                        "{provider_name} subscription authorization expired. Re-authenticate in Providers."
                    ));
                }
                Err(crate::providers::response_error(response, access_token).await)
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
        assert!(error.contains("System message must be at the beginning"));
        server.await.unwrap();
    }
}
