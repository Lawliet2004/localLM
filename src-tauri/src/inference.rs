use async_trait::async_trait;
use serde_json::{json, Value};

/// Provider-reported counters for one generation. Stream usage snapshots are
/// cumulative, so merge fields instead of summing events. Missing counters
/// remain unknown; preflight estimates never become claimed cache hits.
#[derive(Clone, Default, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub prompt_ms: Option<f64>,
    pub generation_ms: Option<f64>,
    pub processed_prompt_tokens: Option<u64>,
    pub generation_tokens_per_second: Option<f64>,
    pub prompt_tokens_per_second: Option<f64>,
    #[serde(skip)]
    uncached_input: Option<u64>,
}

impl InferenceUsage {
    pub fn observe(&mut self, event: &Value) {
        let usage = &event["usage"];
        for (field, incoming) in [
            (&mut self.input_tokens, usage["prompt_tokens"].as_u64()),
            (
                &mut self.output_tokens,
                usage["completion_tokens"]
                    .as_u64()
                    .or_else(|| usage["output_tokens"].as_u64()),
            ),
            (
                &mut self.cached_tokens,
                usage["prompt_tokens_details"]["cached_tokens"]
                    .as_u64()
                    .or_else(|| usage["cache_read_input_tokens"].as_u64()),
            ),
            (
                &mut self.cache_write_tokens,
                usage["cache_creation_input_tokens"].as_u64(),
            ),
            (&mut self.uncached_input, usage["input_tokens"].as_u64()),
        ] {
            if incoming.is_some() {
                *field = incoming;
            }
        }
        if let Some(uncached) = self.uncached_input {
            self.input_tokens = Some(
                uncached
                    .saturating_add(self.cached_tokens.unwrap_or(0))
                    .saturating_add(self.cache_write_tokens.unwrap_or(0)),
            );
        }
        let timings = &event["timings"];
        for (field, incoming) in [
            (
                &mut self.generation_tokens_per_second,
                timings["predicted_per_second"].as_f64(),
            ),
            (
                &mut self.prompt_tokens_per_second,
                timings["prompt_per_second"].as_f64(),
            ),
        ] {
            if let Some(value) = incoming.filter(|v| v.is_finite() && *v >= 0.0) {
                *field = Some(value);
            }
        }
        if let Some(tokens) = timings["prompt_n"].as_u64() {
            self.processed_prompt_tokens = Some(tokens);
        }
        if self.output_tokens.is_none() {
            self.output_tokens = timings["predicted_n"].as_u64();
        }
        if self.cached_tokens.is_none() {
            self.cached_tokens = timings["cache_n"].as_u64();
        }
        if self.input_tokens.is_none() {
            if let (Some(prompt), Some(cached)) =
                (timings["prompt_n"].as_u64(), timings["cache_n"].as_u64())
            {
                self.input_tokens = Some(prompt.saturating_add(cached));
            }
        }
        for (field, incoming) in [
            (&mut self.prompt_ms, timings["prompt_ms"].as_f64()),
            (&mut self.generation_ms, timings["predicted_ms"].as_f64()),
        ] {
            if let Some(value) = incoming.filter(|value| value.is_finite() && *value >= 0.0) {
                *field = Some(value);
            }
        }
    }

    pub fn reported(&self) -> bool {
        self.input_tokens.is_some() || self.output_tokens.is_some() || self.cached_tokens.is_some()
    }

    pub fn context(
        &self,
        preflight: u64,
        response_reserve: u32,
        context_length: u32,
        estimated: bool,
    ) -> Value {
        let input = self.input_tokens.unwrap_or(preflight);
        let cached = self.cached_tokens.map(|cached| cached.min(input));
        serde_json::json!({
            "inputTokens":input, "responseReserve":response_reserve, "contextLength":context_length,
            "estimated":self.input_tokens.is_none() && estimated,
            "cachedTokens":cached, "cacheWriteTokens":self.cache_write_tokens, "outputTokens":self.output_tokens,
            "cacheHitRate":self.input_tokens.filter(|total| *total > 0).and_then(|total| cached.map(|cached| cached as f64 / total as f64)),
            "promptMs":self.prompt_ms, "generationMs":self.generation_ms,
        })
    }
}

/// Observed timings use Rust's monotonic Instant. Token counts and inference
/// durations are backend values, never character counts or preflight estimates.
#[derive(Clone, Default, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InferenceRound {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    pub usage: InferenceUsage,
    pub first_token_ms: Option<f64>,
    #[serde(default)]
    pub first_visible_answer_ms: Option<f64>,
    #[serde(default)]
    pub reasoning_ms: Option<f64>,
    pub duration_ms: f64,
    pub complete: bool,
}
#[derive(Clone, Default, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseMetrics {
    pub model: String,
    pub rounds: Vec<InferenceRound>,
    pub total_ms: f64,
    #[serde(default)]
    pub preparation_ms: Option<f64>,
    #[serde(default)]
    pub tools_ms: Option<f64>,
    #[serde(default)]
    pub approval_ms: Option<f64>,
}

#[async_trait]
pub trait InferenceProvider: Send + Sync {
    fn payload(&self, payload: &Value) -> Result<Value, String>;
    fn context_capacity(&self) -> u32;
    fn response_limit(&self) -> Option<u32>;
    /// Whether tool use may be attempted. Remote Unknown is not a claim of
    /// verified support; Backend::capability_report retains its actual state.
    fn supports_tools(&self) -> bool;
    fn context_is_estimate(&self) -> bool;
    async fn check_context(&self, payload: &Value, response_tokens: u32) -> Result<u64, String>;
    /// Count payload tokens without enforcing the context budget. Local
    /// runtimes return the exact tokenizer count; remote providers return a
    /// character-derived estimate.
    async fn count_tokens(&self, payload: &Value) -> Result<u64, String>;
    async fn stream(&self, payload: &Value) -> Result<reqwest::Response, String>;
}

pub(crate) fn openai_reasoning_effort_supported(model: &str) -> bool {
    [
        "gpt-5",
        "gpt-5-mini",
        "gpt-5-nano",
        "gpt-5.1",
        "gpt-5.2",
        "gpt-5.4",
        "gpt-5.5",
        "o1",
        "o3",
        "o3-mini",
        "o4-mini",
    ]
    .iter()
    .any(|name| model == *name || model.starts_with(&format!("{name}-20")))
}

pub enum Backend {
    Local {
        client: reqwest::Client,
        endpoint: String,
        key: String,
        context_length: u32,
    },
    OpenAi {
        responses: bool,
        client: reqwest::Client,
        base_url: String,
        key: String,
        model_id: String,
        context_length: u32,
        max_output_tokens: Option<u32>,
        capabilities: crate::provider_capabilities::ModelCapabilities,
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
        capabilities: crate::provider_capabilities::ModelCapabilities,
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
        capabilities: crate::provider_capabilities::ModelCapabilities,
    },
}

impl Backend {
    /// Returns whether a native schema constraint was requested. Every caller
    /// must still validate the final answer against the full original schema.
    pub fn constrain_output(&self, payload: &mut Value, schema: &Value) -> Result<bool, String> {
        crate::tool_validation::compile(schema)?;
        if matches!(self, Self::Local { .. }) {
            payload["response_format"] = json!({"type":"json_object","schema":schema});
            return Ok(true);
        }
        if !self
            .capabilities()
            .controls
            .as_ref()
            .is_some_and(|controls| {
                controls.structured_output == crate::providers::ToolSupport::Supported
            })
        {
            return Ok(false);
        }
        if !matches!(self, Self::Claude { .. }) {
            crate::provider_capabilities::validate_strict_schema(schema)?;
        }
        payload["response_format"] = json!({"type":"json_schema","json_schema":{"name":"child_result","schema":schema,"strict":true}});
        Ok(true)
    }
    pub fn capabilities(&self) -> crate::provider_capabilities::ModelCapabilities {
        match self {
            Self::Local { .. } => crate::provider_capabilities::ModelCapabilities::legacy(true),
            Self::OpenAi { capabilities, .. }
            | Self::Claude { capabilities, .. }
            | Self::Subscription { capabilities, .. } => capabilities.clone(),
        }
    }
    pub fn capability_report(&self) -> Value {
        let mut report = self.capabilities().report();
        if matches!(self, Self::Local { .. }) {
            report["controlSource"] = json!("owned-runtime-protocol");
            report["structuredSchemaRequests"] = json!("supported-with-post-validation");
        }
        report
    }
    fn protocol(&self) -> &str {
        match self {
            Self::OpenAi {
                responses: true, ..
            } => crate::providers::OPENAI_RESPONSES,
            Self::Claude { .. } => crate::providers::CLAUDE_MESSAGES,
            _ => crate::providers::OPENAI_CHAT_COMPLETIONS,
        }
    }
    pub fn local(endpoint: String, key: String, context_length: u32) -> Result<Self, String> {
        let client = crate::http_client::local_model()?;
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
        if let Some(controls) = &model.controls {
            controls.validate_endpoint(&provider.api_format, &provider.base_url, &model.id)?;
        }
        let base_url = crate::providers::validate_base_url(&provider.base_url)?;
        let context_length = model.context_length.ok_or_else(|| format!("Set a context capacity for remote model '{}' before sending a message. LocalLM will not invent a provider limit.", model.id))?;
        let client = crate::providers::client()?;
        Ok(Self::OpenAi {
            responses: provider.api_format == crate::providers::OPENAI_RESPONSES,
            client,
            base_url,
            key,
            model_id: model.id.clone(),
            context_length,
            max_output_tokens: model.max_output_tokens,
            capabilities: crate::provider_capabilities::ModelCapabilities::from_model(model),
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
        if let Some(controls) = &model.controls {
            controls.validate_endpoint(
                crate::providers::CLAUDE_MESSAGES,
                &provider.base_url,
                &model.id,
            )?;
        }
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
            capabilities: crate::provider_capabilities::ModelCapabilities::from_model(model),
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
            capabilities: crate::provider_capabilities::ModelCapabilities::from_model(model),
        })
    }

    pub fn is_responses(&self) -> bool {
        matches!(
            self,
            Self::OpenAi {
                responses: true,
                ..
            }
        )
    }

    pub fn validate_response_tokens(&self, response_tokens: u32) -> Result<(), String> {
        if let Some(limit) = self.response_limit() {
            if response_tokens > limit {
                return Err(format!("Maximum response tokens ({response_tokens}) exceed this model's configured output limit ({limit}). Review this model's maximum output tokens in Providers."));
            }
        }
        Ok(())
    }

    /// Resolve per-request budgets without changing saved local preferences.
    /// Zero is internal-only: an unknown remote limit uses the provider default
    /// and reserves no invented output capacity during estimated preflight.
    pub fn prepare_preferences(&self, preferences: &mut crate::store::Preferences) -> Result<(), String> {
        preferences.validate_generation_controls()?;
        preferences.max_tokens = match self {
            Self::Local { context_length, .. } => {
                let budget = crate::context::fit_response_budget(preferences.max_tokens, *context_length);
                preferences.max_tokens = budget;
                preferences.validate()?;
                budget
            }
            Self::Claude { max_output_tokens: None, .. } => {
                return Err("Set this API model's maximum output tokens in Providers. The Anthropic Messages protocol requires an explicit output limit.".into());
            }
            _ => self.response_limit().unwrap_or(0),
        };
        self.validate_response_tokens(preferences.max_tokens)
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
    fn payload(&self, payload: &Value) -> Result<Value, String> {
        if !payload.is_object() {
            return Err("Model request must be an object.".into());
        }
        let mut payload = payload.clone();
        // A missing API model limit must not inherit the local 2K default.
        // Keep explicit limits on diagnostic/structured requests unchanged.
        if matches!(self, Self::OpenAi { .. } | Self::Subscription { .. })
            && self.response_limit().is_none()
            && payload["max_tokens"] == 0
        {
            payload.as_object_mut().unwrap().remove("max_tokens");
        }
        let capabilities = self.capabilities();
        if let Some(controls) = &capabilities.controls {
            crate::provider_capabilities::apply(&mut payload, controls, self.protocol())?;
        }
        if let Self::OpenAi {
            responses: true,
            model_id,
            ..
        } = self
        {
            if !self.supports_tools() {
                if let Some(object) = payload.as_object_mut() {
                    for key in ["tools", "tool_choice", "parallel_tool_calls"] {
                        object.remove(key);
                    }
                }
            }
            // Preserve the existing known-family controls for the explicitly
            // selected native protocol. Unknown models keep provider defaults;
            // family inference is not a claim of verified model capability.
            if capabilities.controls.is_none() && openai_reasoning_effort_supported(model_id) {
                if let Some(object) = payload.as_object_mut() {
                    object.remove("temperature");
                    object.remove("top_p");
                    if let Some(effort) = object
                        .get("locallm_reasoning_mode")
                        .and_then(Value::as_str)
                        .and_then(|mode| match mode {
                            "quick" => Some("low"),
                            "think" => Some("high"),
                            _ => None,
                        })
                    {
                        if !object.contains_key("reasoning") {
                            object.insert("reasoning".into(), json!({"effort":effort}));
                        }
                    }
                }
            }
            return crate::responses_adapter::request(&payload, model_id);
        }
        if let Some(messages) = payload["messages"].as_array_mut() {
            for message in messages {
                if let Some(object) = message.as_object_mut() {
                    object.remove("locallm_responses_output");
                }
            }
        }
        if !matches!(self, Self::Claude { .. }) {
            if let Some(messages) = payload.get_mut("messages").and_then(Value::as_array_mut) {
                for message in messages {
                    if let Some(object) = message.as_object_mut() {
                        object.remove("locallm_claude_content");
                    }
                }
            }
        }
        if !self.supports_tools() {
            if let Some(object) = payload.as_object_mut() {
                for key in ["tools", "tool_choice", "parallel_tool_calls"] {
                    object.remove(key);
                }
            }
        }
        Ok(match self {
            Self::Local { .. } => {
                payload
                    .as_object_mut()
                    .map(|object| object.remove("locallm_reasoning_mode"));
                payload["stream_options"] = serde_json::json!({"include_usage":true});
                payload["timings"] = serde_json::json!(true);
                payload
            }
            Self::Claude {
                model_id,
                max_output_tokens,
                ..
            } => {
                if payload
                    .get("output_config")
                    .is_some_and(|value| !value.is_object())
                {
                    return Err("Claude output_config must be an object.".into());
                }
                let mut native = crate::claude_adapter::to_messages_request(
                    &payload,
                    model_id,
                    *max_output_tokens,
                );
                if let Some(config) = payload.get("output_config") {
                    native["output_config"] = config.clone();
                }
                if let Some(format) = payload.get("response_format") {
                    if format["type"] != "json_schema" {
                        return Err("Claude structured output requires a JSON Schema.".into());
                    }
                    crate::tool_validation::compile(&format["json_schema"]["schema"])?;
                    native["output_config"]["format"] =
                        json!({"type":"json_schema","schema":format["json_schema"]["schema"]});
                }
                native
            }
            Self::OpenAi { model_id, .. } | Self::Subscription { model_id, .. } => {
                if let Some(object) = payload.as_object_mut() {
                    let mode = object.remove("locallm_reasoning_mode");
                    // Unknown compatible endpoints keep their default protocol.
                    // Known OpenAI reasoning families accept low/high effort.
                    if capabilities.controls.is_none()
                        && matches!(self, Self::OpenAi { .. })
                        && openai_reasoning_effort_supported(model_id)
                    {
                        if let Some(tokens) = object.remove("max_tokens") {
                            object.insert("max_completion_tokens".into(), tokens);
                        }
                        object.remove("temperature");
                        object.remove("top_p");
                        if let Some(mode) = mode.as_ref().and_then(Value::as_str) {
                            match mode {
                                "quick" => {
                                    object.insert("reasoning_effort".into(), json!("low"));
                                }
                                "think" => {
                                    object.insert("reasoning_effort".into(), json!("high"));
                                }
                                _ => {}
                            }
                        }
                    }
                    object.remove("cache_prompt");
                    object.remove("id_slot");
                    // repeat_penalty is a llama.cpp extension — some
                    // OpenAI-compatible endpoints reject unknown fields.
                    object.remove("repeat_penalty");
                    object.remove("chat_template_kwargs");
                    object.remove("timings");
                    object.insert("model".into(), Value::String(model_id.clone()));
                }
                payload
            }
        })
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
            Self::OpenAi { capabilities, .. }
            | Self::Claude { capabilities, .. }
            | Self::Subscription { capabilities, .. } => capabilities.allows_tools(),
        }
    }

    fn context_is_estimate(&self) -> bool {
        matches!(
            self,
            Self::OpenAi { .. } | Self::Claude { .. } | Self::Subscription { .. }
        )
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
            Self::OpenAi { context_length, .. }
            | Self::Claude { context_length, .. }
            | Self::Subscription { context_length, .. } => {
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
            } => crate::context::count_only(client, endpoint, key, payload).await,
            Self::OpenAi { .. } | Self::Claude { .. } | Self::Subscription { .. } => {
                Ok(crate::context::estimate_remote_tokens(payload))
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
                let request = client
                    .post(crate::providers::endpoint(endpoint, "chat/completions"))
                    .bearer_auth(key)
                    .json(payload);
                let response = crate::inference_retry::send(request)
                    .await
                    .map_err(|error| {
                        format!("The local model did not respond. Check that llama.cpp is still running.{}", error.note())
                    })?;
                if response.status().is_success() {
                    Ok(response)
                } else {
                    let status = response.status();
                    let retry_note = crate::inference_retry::response_note(&response);
                    let body = crate::providers::bounded_body(response, 16_384)
                        .await
                        .unwrap_or_default();
                    Err(format!(
                        "{}{retry_note}",
                        crate::context::runtime_prompt_error(status, &body, key)
                    ))
                }
            }
            Self::OpenAi {
                client,
                base_url,
                key,
                responses,
                ..
            } => {
                let mut request = client
                    .post(crate::providers::endpoint(
                        base_url,
                        if *responses {
                            "responses"
                        } else {
                            "chat/completions"
                        },
                    ))
                    .json(payload);
                // Keyless loopback engines must not receive an empty bearer.
                if !key.is_empty() {
                    request = request.bearer_auth(key);
                }
                let response = crate::inference_retry::send(request).await.map_err(|error| {
                    let message = if error.is_timeout() {
                        "Provider request timed out.".to_string()
                    } else {
                        "Could not reach provider. Check the HTTPS base URL and network connection."
                            .to_string()
                    };
                    format!("{message}{}", error.note())
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
                let request = client
                    .post(crate::providers::endpoint(base_url, "messages"))
                    .header("x-api-key", key)
                    .header(
                        "anthropic-version",
                        crate::claude_adapter::ANTHROPIC_VERSION,
                    )
                    .header("User-Agent", "LocalLM-Desktop/0.1")
                    .json(payload);
                let response = crate::inference_retry::send(request)
                    .await
                    .map_err(|error| {
                        let message = if error.is_timeout() {
                            "Claude request timed out.".to_string()
                        } else {
                            "Could not reach Claude. Check the HTTPS base URL and network connection."
                                .to_string()
                        };
                        format!("{message}{}", error.note())
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

                let response = crate::inference_retry::send(req).await.map_err(|error| {
                    let message = if error.is_timeout() {
                        "Subscription provider request timed out.".to_string()
                    } else {
                        format!("Could not reach {provider_name} subscription service. Check network connection.")
                    };
                    format!("{message}{}", error.note())
                })?;

                let status = response.status();
                let retry_note = crate::inference_retry::response_note(&response);
                if status.is_success() {
                    return Ok(response);
                }

                if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    return Err(format!(
                        "{provider_name} subscription rate limit or quota reached. Your rolling quota was exceeded. Try again later or switch to your local model.{retry_note}"
                    ));
                }
                if status == reqwest::StatusCode::UNAUTHORIZED
                    || status == reqwest::StatusCode::FORBIDDEN
                {
                    return Err(format!(
                        "{provider_name} subscription authorization expired. Re-authenticate in Providers.{retry_note}"
                    ));
                }
                Err(crate::providers::response_error(response, access_token).await)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_backend_preserves_the_enabled_genui_prompt_and_strips_native_replay_metadata() {
        for enabled in [false, true] {
            let system =
                crate::prompt::frozen_system_for_tools("Base instructions", "", None, &[], enabled);
            let messages = crate::prompt::build_turn_messages(
                &system,
                &[],
                None,
                &json!({"role":"user","content":"Hello"}),
            );
            let model = RemoteModel {
                controls: None,
                id: "custom-model".into(),
                context_length: Some(65536),
                max_output_tokens: None,
                supports_images: false,
                tool_support: ToolSupport::Supported,
            };
            let mut claude = provider();
            claude.api_format = crate::providers::CLAUDE_MESSAGES.into();
            let backends = [
                Backend::local("http://127.0.0.1:8080".into(), "local-key".into(), 65536).unwrap(),
                Backend::openai(&provider(), String::new(), &model).unwrap(),
                Backend::openai(&claude, "claude-key".into(), &model).unwrap(),
                Backend::subscription(
                    "Subscription",
                    "https://example.test/v1",
                    "token".into(),
                    None,
                    &model,
                )
                .unwrap(),
            ];
            for backend in backends {
                let payload = backend.payload(&json!({"messages":messages})).unwrap();
                let actual = if matches!(backend, Backend::Claude { .. }) {
                    &payload["system"]
                } else {
                    &payload["messages"][0]["content"]
                };
                assert_eq!(actual, &json!(system));
                assert_eq!(
                    actual
                        .as_str()
                        .unwrap()
                        .contains(crate::prompt::GENUI_GUIDANCE),
                    enabled
                );
                let payload = backend.payload(&json!({"messages":[{"role":"assistant","content":"Hello","locallm_claude_content":[{"type":"text","text":"Hello"}]}]})).unwrap();
                assert!(payload["messages"][0]
                    .get("locallm_claude_content")
                    .is_none());
                if matches!(backend, Backend::Claude { .. }) {
                    assert_eq!(payload["messages"][0]["content"][0]["text"], "Hello");
                }
            }
        }
    }

    #[tokio::test]
    async fn provider_streams_preserve_genui_tools_unicode_and_usage_end_to_end() {
        use futures_util::StreamExt;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (format, prefix, key) in [
            (OPENAI_CHAT_COMPLETIONS, "", ""),
            (OPENAI_CHAT_COMPLETIONS, "/v1", "fixture-key"),
            (OPENAI_CHAT_COMPLETIONS, "/proxy/v1/", ""),
            (crate::providers::CLAUDE_MESSAGES, "", "claude-key"),
            (crate::providers::CLAUDE_MESSAGES, "/v1", "claude-key"),
            ("local", "", "local-key"),
        ] {
            let claude = crate::providers::is_claude_format(format);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base_url = format!("http://{}{prefix}", listener.local_addr().unwrap());
            let events = if claude {
                vec![
                    json!({"type":"message_start","message":{"usage":{"input_tokens":10}}}),
                    json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":"café "}}),
                    json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"日本"}}),
                    json!({"type":"content_block_stop","index":0}),
                    json!({"type":"content_block_start","index":4,"content_block":{"type":"tool_use","id":"call_1","name":"lookup","input":{}}}),
                    json!({"type":"content_block_delta","index":4,"delta":{"type":"input_json_delta","partial_json":"{\"q\":"}}),
                    json!({"type":"content_block_delta","index":4,"delta":{"type":"input_json_delta","partial_json":"\"café 日本\"}"}}),
                    json!({"type":"content_block_stop","index":4}),
                    json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":12}}),
                    json!({"type":"message_stop"}),
                ]
            } else {
                vec![
                    json!({"choices":[{"delta":{"content":"café "}}]}),
                    json!({"choices":[{"delta":{"content":"日本","tool_calls":[{"index":0,"type":"function","id":"call_1","function":{"name":"lookup","arguments":"{\"q\":"}}]}}]}),
                    json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"café 日本\"}"}}]},"finish_reason":"tool_calls"}]}),
                    json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":12}}),
                ]
            };
            let mut wire = String::from("\u{feff}");
            for event in events {
                wire.push_str(&format!("data: {event}\r\n\r\n"));
            }
            if !claude {
                wire.push_str("data: [DONE]\r\n\r\n");
            }
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut buffer = [0; 4096];
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
                            let payload: Value =
                                serde_json::from_slice(&request[end + 4..end + 4 + length])
                                    .unwrap();
                            let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{wire}", wire.len());
                            stream.write_all(response.as_bytes()).await.unwrap();
                            return (headers, payload);
                        }
                    }
                }
            });
            let mut connection = provider();
            connection.api_format = format.into();
            connection.base_url = base_url;
            let model = RemoteModel {
                controls: None,
                id: "fixture-model".into(),
                context_length: Some(65536),
                max_output_tokens: None,
                supports_images: false,
                tool_support: ToolSupport::Supported,
            };
            let backend = if format == "local" {
                Backend::local(connection.base_url.clone(), key.into(), 65536).unwrap()
            } else {
                Backend::openai(&connection, key.into(), &model).unwrap()
            };
            let system =
                crate::prompt::frozen_system_for_tools("Base instructions", "", None, &[], true);
            let messages = crate::prompt::build_turn_messages(
                &system,
                &[],
                None,
                &json!({"role":"user","content":"Look it up"}),
            );
            let payload = backend.payload(&json!({"messages":messages,"max_tokens":64,"stream":true,"tools":[{"type":"function","function":{"name":"lookup","description":"Lookup","parameters":{"type":"object"}}}]})).unwrap();
            let mut stream = backend.stream(&payload).await.unwrap().bytes_stream();
            let mut decoder = crate::sse::SseDecoder::default();
            let mut protocol = crate::sse::ModelStream::new(claude);
            let mut calls = crate::tool_calls::ToolCalls::default();
            let mut usage = InferenceUsage::default();
            let mut text = String::new();
            while let Some(chunk) = stream.next().await {
                // Exercise every UTF-8 split, independent of TCP coalescing.
                for byte in chunk.unwrap() {
                    for event in decoder.push(&[byte]).unwrap() {
                        if let Some(value) = protocol.push(&event).unwrap() {
                            calls.push(&value["choices"][0]["delta"]).unwrap();
                            usage.observe(&value);
                            text.push_str(
                                value["choices"][0]["delta"]["content"]
                                    .as_str()
                                    .unwrap_or(""),
                            );
                        }
                    }
                }
            }
            decoder.finish().unwrap();
            assert!(protocol.done);
            assert_eq!(text, "café 日本");
            let calls = calls.finish().unwrap();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].id, "call_1");
            assert_eq!(calls[0].arguments["q"], "café 日本");
            assert_eq!(usage.input_tokens, Some(10));
            assert_eq!(usage.output_tokens, Some(12));
            let (headers, received) = server.await.unwrap();
            let api_prefix = if prefix.trim_matches('/').is_empty() {
                "/v1"
            } else {
                prefix.trim_end_matches('/')
            };
            let route = if claude {
                "messages"
            } else {
                "chat/completions"
            };
            assert!(
                headers.starts_with(&format!("post {api_prefix}/{route} ")),
                "{headers}"
            );
            if claude {
                assert!(headers.contains("x-api-key: claude-key"));
                assert!(headers.contains("anthropic-version: 2023-06-01"));
                assert!(!headers.contains("authorization:"));
                assert_eq!(received["system"], system);
            } else {
                assert_eq!(received["messages"][0]["content"], system);
                assert!(!headers.contains("x-api-key:"));
                if key.is_empty() {
                    assert!(!headers.contains("authorization:"));
                } else {
                    assert!(headers.contains(&format!("authorization: bearer {key}")));
                }
            }
        }
    }

    #[test]
    fn text_only_provider_payload_removes_tool_options_even_during_planning() {
        let model = crate::providers::RemoteModel {
            controls: None,
            id: "text-only".into(),
            context_length: Some(8192),
            max_output_tokens: None,
            supports_images: false,
            tool_support: crate::providers::ToolSupport::Unsupported,
        };
        let backend = super::Backend::openai(&provider(), "fixture".into(), &model).unwrap();
        let payload = super::InferenceProvider::payload(&backend, &serde_json::json!({"messages":[{"role":"user","content":"Hello"}],"tools":[{"type":"function"}],"tool_choice":"none","parallel_tool_calls":false})).unwrap();
        for key in ["tools", "tool_choice", "parallel_tool_calls"] {
            assert!(payload.get(key).is_none(), "{key}");
        }
        assert_eq!(payload["model"], "text-only");
    }
    use super::*;
    #[tokio::test]
    async fn counting_and_budget_checks_share_cached_counts_across_runtime_controls() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut bytes = [0; 2048];
                let count = stream.read(&mut bytes).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&bytes[..count]);
                if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                    let size: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if request.len() >= end + 4 + size {
                        let payload: Value =
                            serde_json::from_slice(&request[end + 4..end + 4 + size]).unwrap();
                        for field in ["cache_prompt", "id_slot", "timings", "stream_options"] {
                            assert!(payload.get(field).is_none());
                        }
                        break;
                    }
                }
            }
            let body = r#"{"input_tokens":100}"#;
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
        });
        let backend = Backend::local(endpoint, "canonical-count".into(), 4096).unwrap();
        let mut payload = backend.payload(&serde_json::json!({"messages":[{"role":"user","content":"Hello"}],"cache_prompt":true,"id_slot":0,"stream":true})).unwrap();
        assert_eq!(backend.count_tokens(&payload).await.unwrap(), 100);
        server.await.unwrap();
        payload["id_slot"] = serde_json::json!(1);
        payload["cache_prompt"] = serde_json::json!(false);
        assert_eq!(backend.check_context(&payload, 64).await.unwrap(), 100);
        assert!(backend
            .check_context(&payload, 4000)
            .await
            .unwrap_err()
            .contains("Context limit exceeded"));
    }
    #[test]
    fn cache_usage_merges_cumulative_snapshots_without_inventing_hits() {
        let mut usage = InferenceUsage::default();
        assert!(!usage.reported());
        assert!(usage.context(100, 20, 1024, true)["cacheHitRate"].is_null());
        let event = serde_json::json!({"usage":{"prompt_tokens":100,"completion_tokens":12,"prompt_tokens_details":{"cached_tokens":80}}});
        usage.observe(&event);
        usage.observe(&event);
        usage.observe(&serde_json::json!({"choices":[]}));
        let context = usage.context(999, 20, 1024, true);
        assert_eq!(context["inputTokens"], 100);
        assert_eq!(context["cachedTokens"], 80);
        assert_eq!(context["outputTokens"], 12);
        assert_eq!(context["cacheHitRate"], 0.8);
        assert_eq!(context["estimated"], false);
    }

    #[test]
    fn claude_counts_include_cache_reads_and_writes() {
        let mut usage = InferenceUsage::default();
        usage.observe(&serde_json::json!({"usage":{"input_tokens":10,"cache_read_input_tokens":80,"cache_creation_input_tokens":10}}));
        usage.observe(&serde_json::json!({"usage":{"output_tokens":15}}));
        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.output_tokens, Some(15));
        assert_eq!(usage.context(1, 32, 1024, true)["cacheHitRate"], 0.8);
    }

    #[test]
    fn llama_timing_fallback_and_invalid_cache_counts_are_bounded() {
        let mut usage = InferenceUsage::default();
        usage.observe(&serde_json::json!({"timings":{"cache_n":90,"prompt_n":10,"prompt_ms":12.5,"predicted_ms":50}}));
        assert_eq!(usage.context(999, 32, 1024, false)["cacheHitRate"], 0.9);
        usage.observe(&serde_json::json!({"usage":{"prompt_tokens":5,"prompt_tokens_details":{"cached_tokens":100}},"timings":{"prompt_ms":-1}}));
        let context = usage.context(5, 32, 1024, false);
        assert_eq!(context["cachedTokens"], 5);
        assert_eq!(context["cacheHitRate"], 1.0);
        assert_eq!(context["promptMs"], 12.5);
    }
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
                controls: None,
                id: "manual".into(),
                context_length: Some(4096),
                max_output_tokens: None,
                supports_images: false,
                tool_support: ToolSupport::Unknown,
            },
        )
        .unwrap();
        let payload = backend
            .payload(&json!({"cache_prompt":true,"messages":[]}))
            .unwrap();
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
                controls: None,
                id: "gpt-4o".into(),
                context_length: Some(128_000),
                max_output_tokens: Some(16_384),
                supports_images: false,
                tool_support: ToolSupport::Supported,
            },
        )
        .unwrap();

        assert_eq!(backend.context_capacity(), 128_000);
        assert_eq!(backend.response_limit(), Some(16_384));
        assert!(backend.supports_tools());
        assert!(backend.context_is_estimate());

        let payload = backend
            .payload(&json!({"cache_prompt": true, "id_slot": 0, "messages": []}))
            .unwrap();
        assert_eq!(payload["model"], "gpt-4o");
        assert!(payload.get("cache_prompt").is_none());
        assert!(payload.get("id_slot").is_none());
    }

    #[test]
    fn api_response_budgets_use_model_capacity_across_protocols() {
        use crate::providers::{CLAUDE_MESSAGES, OPENAI_RESPONSES};
        for format in [OPENAI_CHAT_COMPLETIONS, OPENAI_RESPONSES, CLAUDE_MESSAGES] {
            for limit in [512, 131_072, 262_144, 393_216] {
                let mut connection = provider();
                connection.api_format = format.into();
                let backend = Backend::openai(&connection, "secret".into(), &RemoteModel {
                    controls: None,
                    id: "api-model".into(),
                    context_length: Some(1_048_576),
                    max_output_tokens: Some(limit),
                    supports_images: false,
                    tool_support: ToolSupport::Supported,
                }).unwrap();
                let saved = crate::store::Preferences::default();
                let mut effective = saved.clone();
                backend.prepare_preferences(&mut effective).unwrap();
                assert_eq!(saved.max_tokens, 2048);
                assert_eq!(effective.max_tokens, limit, "{format}");
                // Child snapshots can inherit an already resolved API budget.
                backend.prepare_preferences(&mut effective).unwrap();
                let payload = backend.payload(&json!({
                    "messages":[{"role":"user","content":"hello"}],
                    "max_tokens":effective.max_tokens,
                })).unwrap();
                let field = if format == OPENAI_RESPONSES { "max_output_tokens" } else { "max_tokens" };
                assert_eq!(payload[field], limit, "{format}");
                assert!(crate::context::validate_budget(4096, limit, backend.context_capacity()).is_ok());
                assert!(crate::context::validate_budget(1_048_576, limit, backend.context_capacity()).is_err());
            }
        }
    }

    #[test]
    fn unknown_api_output_capacity_uses_provider_default_without_local_limit() {
        use crate::providers::{CLAUDE_MESSAGES, OPENAI_RESPONSES};
        for format in [OPENAI_CHAT_COMPLETIONS, OPENAI_RESPONSES, CLAUDE_MESSAGES] {
            let mut connection = provider();
            connection.api_format = format.into();
            let backend = Backend::openai(&connection, "secret".into(), &RemoteModel {
                controls: None,
                id: "api-model".into(),
                context_length: Some(1_048_576),
                max_output_tokens: None,
                supports_images: false,
                tool_support: ToolSupport::Supported,
            }).unwrap();
            let mut effective = crate::store::Preferences::default();
            let result = backend.prepare_preferences(&mut effective);
            if format == CLAUDE_MESSAGES {
                assert!(result.unwrap_err().contains("requires an explicit output limit"));
                continue;
            }
            result.unwrap();
            assert_eq!(effective.max_tokens, 0);
            let payload = backend.payload(&json!({
                "messages":[{"role":"user","content":"hello"}],
                "max_tokens":effective.max_tokens,
            })).unwrap();
            for field in ["max_tokens", "max_completion_tokens", "max_output_tokens"] {
                assert!(payload.get(field).is_none(), "{format}: {field}");
            }
        }
    }

    #[test]
    fn local_response_budget_stays_within_runtime_preferences_and_context() {
        let backend = Backend::local("http://127.0.0.1:1234".into(), "secret".into(), 8192).unwrap();
        let mut preferences = crate::store::Preferences::default();
        backend.prepare_preferences(&mut preferences).unwrap();
        assert_eq!(preferences.max_tokens, 2048);
        preferences.max_tokens = 131_072;
        backend.prepare_preferences(&mut preferences).unwrap();
        assert_eq!(preferences.max_tokens, 4096);
    }

    #[test]
    fn remote_context_and_output_limits_are_explicit() {
        let backend = Backend::openai(
            &provider(),
            "secret".into(),
            &RemoteModel {
                controls: None,
                id: "manual".into(),
                context_length: Some(4096),
                max_output_tokens: Some(512),
                supports_images: false,
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
                controls: None,
                id: "manual".into(),
                context_length: None,
                max_output_tokens: None,
                supports_images: false,
                tool_support: ToolSupport::Unknown
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
                controls: None,
                id: "claude-model".into(),
                context_length: Some(200_000),
                max_output_tokens: Some(4096),
                supports_images: false,
                tool_support: ToolSupport::Supported,
            },
        )
        .unwrap();
        assert!(backend.context_is_estimate());
        assert_eq!(backend.context_capacity(), 200_000);
        let payload = backend
            .payload(&json!({
                "messages": [
                    {"role": "system", "content": "sys"},
                    {"role": "user", "content": "hi"},
                ],
                "max_tokens": 256,
                "stream": true,
                "cache_prompt": true,
            }))
            .unwrap();
        assert_eq!(payload["model"], "claude-model");
        assert_eq!(payload["system"], "sys");
        assert_eq!(payload["stream"], true);
        assert!(payload.get("cache_prompt").is_none());
        assert!(payload["messages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|message| message["role"] != "system"));
        // No key, no backend: consumer subscriptions are not API access.
        assert!(Backend::openai(
            &fixture,
            String::new(),
            &RemoteModel {
                controls: None,
                id: "claude-model".into(),
                context_length: Some(200_000),
                max_output_tokens: None,
                supports_images: false,
                tool_support: ToolSupport::Supported,
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
                controls: None,
                id: "claude-model".into(),
                context_length: Some(200_000),
                max_output_tokens: None,
                supports_images: false,
                tool_support: ToolSupport::Supported,
            },
        )
        .unwrap();
        let payload = backend.payload(&json!({"messages": [{"role": "user", "content": "hi"}], "max_tokens": 64, "stream": true})).unwrap();
        let response = backend.stream(&payload).await.unwrap();
        let body = response.text().await.unwrap();
        let (headers, received) = server.await.unwrap();
        assert!(headers.starts_with("post /v1/messages "), "{headers}");
        assert!(headers.contains("x-api-key: claude-key"), "{headers}");
        assert!(
            headers.contains("anthropic-version: 2023-06-01"),
            "{headers}"
        );
        assert!(
            !headers.contains("authorization: bearer"),
            "Claude must not receive OpenAI bearer auth: {headers}"
        );
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
                controls: None,
                id: "remote-model".into(),
                context_length: Some(4096),
                max_output_tokens: None,
                supports_images: false,
                tool_support: ToolSupport::Supported,
            },
        )
        .unwrap();
        let payload = backend.payload(&json!({"cache_prompt": true, "stream": true, "messages": [{"role": "user", "content": "hello"}]})).unwrap();
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
                controls: None,
                id: "loopback-model".into(),
                context_length: Some(4096),
                max_output_tokens: None,
                supports_images: false,
                tool_support: ToolSupport::Supported,
            },
        )
        .unwrap();
        let payload = backend
            .payload(&json!({"stream": true, "messages": [{"role": "user", "content": "hello"}]}))
            .unwrap();
        let response = backend.stream(&payload).await.unwrap();
        let body = response.text().await.unwrap();
        let headers = server.await.unwrap();
        assert!(
            !headers.contains("authorization"),
            "keyless loopback requests must not carry an empty bearer: {headers}"
        );
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
                        let body =
                            r#"{"error":{"message":"System message must be at the beginning."}}"#;
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
        let backend =
            Backend::local(format!("http://{address}"), "local-key".into(), 8192).unwrap();
        let error = backend
            .stream(&json!({"messages":[{"role":"system","content":"a"},{"role":"system","content":"b"}]}))
            .await
            .unwrap_err();
        assert!(error.contains("System message must be at the beginning"));
        server.await.unwrap();
    }
}

#[cfg(test)]
mod reasoning_payload_tests {
    use super::*;
    #[test]
    fn remote_mapping_does_not_leak_local_extensions_or_guess_unknown_families() {
        let backend = |model: &str| Backend::OpenAi {
            responses: false,
            client: reqwest::Client::new(),
            base_url: "https://api.example.test".into(),
            key: "test".into(),
            model_id: model.into(),
            context_length: 8192,
            max_output_tokens: None,
            capabilities: crate::provider_capabilities::ModelCapabilities::legacy(true),
        };
        let internal = json!({"locallm_reasoning_mode":"quick", "chat_template_kwargs":{"enable_thinking":false}, "max_tokens":2048,"temperature":0.7,"top_p":0.95});
        let known = backend("gpt-5").payload(&internal).unwrap();
        assert_eq!(known["reasoning_effort"], "low");
        assert_eq!(known["max_completion_tokens"], 2048);
        assert!(known.get("max_tokens").is_none());
        assert!(known.get("temperature").is_none());
        assert!(known.get("top_p").is_none());
        assert!(backend("gpt-5-pro")
            .payload(&internal)
            .unwrap()
            .get("reasoning_effort")
            .is_none());
        assert!(known.get("chat_template_kwargs").is_none());
        assert!(known.get("locallm_reasoning_mode").is_none());
        assert!(backend("custom-model")
            .payload(&internal)
            .unwrap()
            .get("reasoning_effort")
            .is_none());
    }
    #[test]
    fn native_responses_reasoning_controls_use_native_fields_and_strip_private_history_elsewhere() {
        let backend = |responses, model: &str| Backend::OpenAi {
            responses,
            client: reqwest::Client::new(),
            base_url: "https://api.example.test".into(),
            key: String::new(),
            model_id: model.into(),
            context_length: 8192,
            max_output_tokens: None,
            capabilities: crate::provider_capabilities::ModelCapabilities::legacy(true),
        };
        let input = json!({"messages":[{"role":"user","content":"Explain"}],"max_tokens":2000,"temperature":0.7,"top_p":0.95,"locallm_reasoning_mode":"think"});
        let native = backend(true, "gpt-5").payload(&input).unwrap();
        assert_eq!(native["reasoning"]["effort"], "high");
        assert_eq!(native["max_output_tokens"], 2000);
        for key in [
            "temperature",
            "top_p",
            "reasoning_effort",
            "locallm_reasoning_mode",
            "messages",
        ] {
            assert!(native.get(key).is_none(), "{key}");
        }
        let unknown = backend(true, "custom-model").payload(&input).unwrap();
        assert!(unknown.get("reasoning").is_none());
        assert_eq!(unknown["temperature"], 0.7);
        let explicit =
            json!({"messages":[],"reasoning":{"effort":"medium"},"locallm_reasoning_mode":"quick"});
        assert_eq!(
            backend(true, "gpt-5").payload(&explicit).unwrap()["reasoning"]["effort"],
            "medium"
        );
        let private = json!({"messages":[{"role":"assistant","content":"Safe","locallm_responses_output":[{"type":"reasoning","encrypted_content":"private-response-context"}],"locallm_claude_content":[{"type":"thinking","signature":"private-claude-context"}]}]});
        let chat = backend(false, "custom-model").payload(&private).unwrap();
        assert_eq!(
            chat["messages"],
            json!([{"role":"assistant","content":"Safe"}])
        );
    }
}
