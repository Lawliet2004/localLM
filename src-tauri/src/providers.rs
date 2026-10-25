use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const OPENAI_RESPONSES: &str = "openai-responses";
pub const OPENAI_CHAT_COMPLETIONS: &str = "openai-chat-completions";
pub const CHATGPT_SUBSCRIPTION: &str = "chatgpt-subscription";
pub const GROK_SUBSCRIPTION: &str = "grok-subscription";
pub const FREETOKEN_OPENAI_COMPAT: &str = "freetoken-openai";
pub const CLAUDE_MESSAGES: &str = "claude-messages";
pub const MAX_PROVIDER_MODELS: usize = 2048;

/// Anthropic's native API is `POST {base}/v1/messages` with `x-api-key`,
/// `anthropic-version: 2023-06-01`, and an SSE stream of `message_*` /
/// `content_block_*` events — NOT an OpenAI-compatible `/chat/completions`
/// endpoint. An OpenAI-compatible base URL must never be treated as native
/// Claude behavior; the formats are selected explicitly per connection.
pub fn is_claude_format(format: &str) -> bool {
    format == CLAUDE_MESSAGES
}

pub fn is_subscription_format(format: &str) -> bool {
    matches!(format, CHATGPT_SUBSCRIPTION | GROK_SUBSCRIPTION)
}

/// `host_str` may keep IPv6 brackets (`[::1]`) — strip them before matching.
fn is_loopback_host(host: &str) -> bool {
    matches!(
        host.trim_matches(|character| character == '[' || character == ']'),
        "localhost" | "127.0.0.1" | "::1"
    )
}

/// FreeToken documents `/v1/models` and streaming `/v1/chat/completions`
/// at `http://127.0.0.1:1919`. Its engine does not accept an API key
/// for verified loopback endpoints.
pub fn is_freetoken_base_url(raw: &str) -> bool {
    reqwest::Url::parse(raw.trim())
        .ok()
        .and_then(|url| url.host_str().map(is_loopback_host))
        .unwrap_or(false)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum ToolSupport {
    #[default]
    Unknown,
    Supported,
    Unsupported,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteModel {
    pub id: String,
    pub context_length: Option<u32>,
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub tool_support: ToolSupport,
    #[serde(default)]
    pub supports_images: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<crate::provider_capabilities::Controls>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderConnection {
    pub id: String,
    pub name: String,
    pub api_format: String,
    pub base_url: String,
    pub verified: bool,
    pub last_tested_at: Option<i64>,
    pub models: Vec<RemoteModel>,
    #[serde(default)]
    pub has_api_key: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderDraft {
    pub id: Option<String>,
    pub name: String,
    pub api_format: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub models: Vec<RemoteModel>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[derive(Default)]
pub struct ModelSelection {
    pub provider_id: Option<String>,
    pub model_id: String,
}

impl ModelSelection {
    pub fn validate(&self) -> Result<(), String> {
        if let Some(provider_id) = &self.provider_id {
            if provider_id.is_empty()
                || provider_id.len() > 120
                || !provider_id
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            {
                return Err("Provider selection has an invalid provider identifier.".into());
            }
            validate_model_id(&self.model_id)?;
        } else if self.model_id.len() > 32_768 || self.model_id.chars().any(char::is_control) {
            return Err("Local model selection is invalid.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderTestResult {
    pub verified: bool,
    pub model_list_supported: bool,
    pub models: Vec<String>,
    pub message: String,
    #[serde(skip)]
    pub discovered_models: Vec<RemoteModel>,
}

/// Provider usage snapshot for spending-limit display. Counts come from the
/// provider where available; estimates are labeled, never silently billed.
#[derive(Clone, Debug, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub estimated: bool,
    pub spending_limit_usd: Option<f64>,
}

pub fn validate_model_id(id: &str) -> Result<(), String> {
    if id.trim() != id || id.is_empty() || id.len() > 512 || id.chars().any(char::is_control) {
        return Err(
            "Model ID must contain 1–512 non-control characters without surrounding whitespace."
                .into(),
        );
    }
    Ok(())
}

pub fn credential_id(provider_id: &str) -> String {
    format!("provider-key-{provider_id}")
}

/// The caller holds AppState::operation while changing a connection or secret.
pub fn prepare_connection(
    store: &crate::store::Store,
    vault: &crate::vault::Vault,
    mut draft: ProviderDraft,
) -> Result<
    (
        ProviderConnection,
        Option<ProviderConnection>,
        Option<String>,
    ),
    String,
> {
    if let Some(id) = draft.id.as_deref() {
        validate_provider_id(id)?;
    }
    let existing = draft
        .id
        .as_deref()
        .map(|id| store.provider(id))
        .transpose()?
        .flatten();
    if draft.id.is_some() && existing.is_none() {
        return Err("Provider no longer exists. Add a new provider instead.".into());
    }
    validate_provider_draft(&draft, existing.is_some())?;
    if is_subscription_format(&draft.api_format)
        || existing
            .as_ref()
            .is_some_and(|provider| is_subscription_format(&provider.api_format))
    {
        return Err("Account subscriptions are managed separately from API providers.".into());
    }
    let id = draft
        .id
        .take()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let base_url = validate_base_url(&draft.base_url)?;
    let credential = credential_id(&id);
    let saved_key = vault.contains(&credential)?;
    let same_endpoint = existing.as_ref().is_some_and(|provider| {
        provider.base_url == base_url && provider.api_format == draft.api_format
    });
    if saved_key && !same_endpoint && draft.api_key.is_none() {
        return Err("Enter an API key for the new endpoint. A saved key cannot be reused after changing the base URL or API format.".into());
    }
    let has_api_key = draft.api_key.is_some() || saved_key;
    if requires_api_key(&draft.api_format, &base_url) && !has_api_key {
        return Err(
            "An API key is required for this provider. Claude endpoints always need one.".into(),
        );
    }
    let keep_verification = same_endpoint && draft.api_key.is_none();
    let provider = ProviderConnection {
        id: id.clone(),
        name: draft.name,
        api_format: draft.api_format,
        base_url,
        verified: keep_verification && existing.as_ref().is_some_and(|provider| provider.verified),
        last_tested_at: if keep_verification {
            existing
                .as_ref()
                .and_then(|provider| provider.last_tested_at)
        } else {
            None
        },
        models: draft.models,
        has_api_key,
    };
    Ok((provider, existing, draft.api_key))
}

#[cfg(test)]
pub fn save_connection(
    store: &crate::store::Store,
    vault: &crate::vault::Vault,
    draft: ProviderDraft,
) -> Result<ProviderConnection, String> {
    let (provider, existing, key) = prepare_connection(store, vault, draft)?;
    let id = provider.id.clone();
    let credential = credential_id(&id);
    // Persist metadata first so a database failure cannot replace a working key.
    // If the protected vault refuses the new key, restore the prior metadata.
    store.save_provider(&provider)?;
    if let Some(key) = key {
        if let Err(error) = vault.save(&credential, key.as_bytes()) {
            let restored = if let Some(previous) = existing {
                store.save_provider(&previous)
            } else {
                store.delete_provider(&id).map(|_| ())
            };
            return Err(match restored {
                Ok(()) => error,
                Err(_) => format!("{error} Provider settings could not be restored; review this connection before using it."),
            });
        }
    }
    Ok(provider)
}

#[cfg(test)]
pub fn delete_api_key(
    store: &crate::store::Store,
    vault: &crate::vault::Vault,
    id: &str,
) -> Result<(), String> {
    validate_provider_id(id)?;
    let provider = store.provider(id)?.ok_or("Provider no longer exists.")?;
    if is_subscription_format(&provider.api_format) {
        return Err("Account subscriptions are managed separately from API providers.".into());
    }
    // Invalidate first: a failed deletion still leaves the connection blocked.
    store.update_provider_test(id, false, None, &provider.models)?;
    vault.clear(&credential_id(id))
}

#[cfg(test)]
pub fn delete_connection(
    store: &crate::store::Store,
    vault: &crate::vault::Vault,
    id: &str,
) -> Result<(), String> {
    validate_provider_id(id)?;
    let provider = store.provider(id)?.ok_or("Provider no longer exists.")?;
    if is_subscription_format(&provider.api_format) {
        return Err("Account subscriptions are managed separately from API providers.".into());
    }
    vault.clear(&credential_id(id))?;
    store.delete_provider(id)?;
    Ok(())
}

pub fn validate_provider_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 120
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err("Provider identifier is invalid.".into());
    }
    Ok(())
}

pub fn validate_provider_draft(draft: &ProviderDraft, existing: bool) -> Result<(), String> {
    if draft.name.trim() != draft.name || draft.name.is_empty() || draft.name.chars().count() > 120
    {
        return Err(
            "Provider name must contain 1–120 characters without surrounding whitespace.".into(),
        );
    }
    if draft.api_format != OPENAI_CHAT_COMPLETIONS
        && draft.api_format != OPENAI_RESPONSES
        && draft.api_format != CHATGPT_SUBSCRIPTION
        && draft.api_format != GROK_SUBSCRIPTION
        && draft.api_format != FREETOKEN_OPENAI_COMPAT
        && draft.api_format != CLAUDE_MESSAGES
    {
        return Err("Choose OpenAI-compatible Chat Completions, native Responses, Claude Messages, Subscription providers, or FreeToken.".into());
    }
    validate_base_url(&draft.base_url)?;
    if draft.api_key.is_none() && !existing && requires_api_key(&draft.api_format, &draft.base_url)
    {
        return Err(
            "An API key is required for this provider. Claude endpoints always need one.".into(),
        );
    }
    if let Some(key) = &draft.api_key {
        if key.trim() != key
            || key.is_empty()
            || key.len() > 131_072
            || key.chars().any(char::is_control)
        {
            return Err("API key is empty, too long, or contains control characters.".into());
        }
    }
    validate_models(&draft.models)?;
    for model in &draft.models {
        if let Some(controls) = &model.controls {
            controls.validate_endpoint(&draft.api_format, &draft.base_url, &model.id)?;
        }
    }
    Ok(())
}

/// Build a non-persisted connection view of a draft so test/model-list calls
/// can run before Save. Writes nothing: no store row, no vault secret.
pub fn connection_from_draft(draft: ProviderDraft) -> Result<ProviderConnection, String> {
    if let Some(id) = draft.id.as_deref() {
        validate_provider_id(id)?;
    }
    validate_provider_draft(&draft, draft.id.is_some())?;
    if is_subscription_format(&draft.api_format) {
        return Err("Account subscriptions are managed separately from API providers.".into());
    }
    let base_url = validate_base_url(&draft.base_url)?;
    let has_api_key = draft.api_key.is_some();
    Ok(ProviderConnection {
        id: draft.id.unwrap_or_default(),
        name: draft.name,
        api_format: draft.api_format,
        base_url,
        verified: false,
        last_tested_at: None,
        models: draft.models,
        has_api_key,
    })
}

pub fn validate_models(models: &[RemoteModel]) -> Result<(), String> {
    if models.len() > MAX_PROVIDER_MODELS {
        return Err(format!(
            "A provider can store at most {MAX_PROVIDER_MODELS} model IDs."
        ));
    }
    let mut ids = std::collections::HashSet::new();
    for model in models {
        validate_model_id(&model.id)?;
        if !ids.insert(&model.id) {
            return Err("Provider model IDs must be unique.".into());
        }
        if let Some(context) = model.context_length {
            if !(128..=2_000_000).contains(&context) {
                return Err(
                    "Model context capacity must be between 128 and 2,000,000 tokens.".into(),
                );
            }
        }
        if let Some(output) = model.max_output_tokens {
            if !(1..=1_000_000).contains(&output) {
                return Err("Model output capacity must be between 1 and 1,000,000 tokens.".into());
            }
        }
    }
    Ok(())
}

pub fn validate_base_url(raw: &str) -> Result<String, String> {
    let url =
        reqwest::Url::parse(raw.trim()).map_err(|_| "Provider base URL is invalid.".to_string())?;
    let host = url
        .host_str()
        .ok_or("Provider base URL must include a host.")?;
    if url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "Provider base URL must not contain credentials, a query, or a fragment.".into(),
        );
    }
    let local = is_loopback_host(host);
    if url.scheme() != "https" && !(local && url.scheme() == "http") {
        return Err(
            "Remote providers require HTTPS. HTTP is allowed only for localhost services.".into(),
        );
    }
    Ok(raw.trim().trim_end_matches('/').to_string())
}

/// True when the base URL points at this machine. Verified loopback-only
/// engines (a local llama.cpp server, FreeToken) may run without any
/// authentication.
pub fn is_loopback_base_url(raw: &str) -> bool {
    reqwest::Url::parse(raw.trim())
        .ok()
        .and_then(|url| url.host_str().map(is_loopback_host))
        .unwrap_or(false)
}

/// Whether the provider must present an API key. Claude endpoints always
/// authenticate with `x-api-key` — loopback included. Custom OpenAI-compatible
/// endpoints may be unauthenticated; their connection test determines whether
/// a key is needed. Subscriptions use their imported session instead.
pub fn requires_api_key(format: &str, base_url: &str) -> bool {
    if is_subscription_format(format)
        || matches!(format, OPENAI_CHAT_COMPLETIONS | OPENAI_RESPONSES)
    {
        return false;
    }
    if is_claude_format(format) {
        return true;
    }
    if format == FREETOKEN_OPENAI_COMPAT && is_freetoken_base_url(base_url) {
        return false;
    }
    !is_loopback_base_url(base_url)
}

/// Keep verification, foreground chat, and child agents on the same key policy.
pub fn resolve_api_key(
    provider: &ProviderConnection,
    saved_key: Option<Vec<u8>>,
) -> Result<String, String> {
    match saved_key {
        Some(bytes) => {
            String::from_utf8(bytes).map_err(|_| "Saved provider API key is invalid.".into())
        }
        None if !requires_api_key(&provider.api_format, &provider.base_url) => Ok(String::new()),
        None => Err("Save an API key for the selected provider before continuing.".into()),
    }
}

#[derive(Debug)]
pub struct ModelListResult {
    pub supported: bool,
    pub models: Vec<String>,
    pub details: Vec<RemoteModel>,
}

#[derive(Deserialize)]
struct ModelListResponse {
    data: Vec<ModelListItem>,
}

#[derive(Deserialize)]
struct ModelListItem {
    id: String,
    #[serde(flatten)]
    metadata: serde_json::Map<String, serde_json::Value>,
}

impl ModelListItem {
    fn into_model(self) -> RemoteModel {
        let value = serde_json::Value::Object(self.metadata);
        let capacity = |paths: &[&str], minimum: u64, maximum: u64| {
            paths
                .iter()
                .filter_map(|path| value.pointer(path)?.as_u64())
                .filter(|capacity| (minimum..=maximum).contains(capacity))
                .min()
                .map(|capacity| capacity as u32)
        };
        let parameters = value
            .get("supported_parameters")
            .and_then(serde_json::Value::as_array)
            .filter(|items| {
                items.len() <= 128
                    && items.iter().all(|value| {
                        value
                            .as_str()
                            .is_some_and(|name| !name.is_empty() && name.len() <= 128)
                    })
            });
        let controls = parameters.map(|parameters| {
            use crate::provider_capabilities::{Controls, Source};
            let has = |name: &str| {
                parameters
                    .iter()
                    .any(|parameter| parameter.as_str() == Some(name))
            };
            let support = |name: &str| {
                if has(name) {
                    ToolSupport::Supported
                } else {
                    ToolSupport::Unsupported
                }
            };
            Controls {
                source: Source::ProviderReported,
                temperature: support("temperature"),
                top_p: support("top_p"),
                reasoning_effort: if has("reasoning") || has("reasoning_effort") {
                    ToolSupport::Supported
                } else {
                    ToolSupport::Unknown
                },
                tool_choice: support("tool_choice"),
                parallel_tool_calls: support("parallel_tool_calls"),
                structured_output: if has("structured_outputs") {
                    ToolSupport::Supported
                } else {
                    ToolSupport::Unknown
                },
                ..Default::default()
            }
        });
        RemoteModel {
            controls,
            id: self.id,
            context_length: capacity(
                &[
                    "/context_length",
                    "/context_window",
                    "/max_model_len",
                    "/top_provider/context_length",
                ],
                128,
                2_000_000,
            ),
            max_output_tokens: capacity(
                &[
                    "/max_output_tokens",
                    "/max_completion_tokens",
                    "/top_provider/max_completion_tokens",
                ],
                1,
                1_000_000,
            ),
            tool_support: match parameters {
                Some(parameters)
                    if parameters
                        .iter()
                        .any(|parameter| parameter.as_str() == Some("tools")) =>
                {
                    ToolSupport::Supported
                }
                Some(_) => ToolSupport::Unsupported,
                None => ToolSupport::Unknown,
            },
            supports_images: value
                .pointer("/architecture/input_modalities")
                .and_then(serde_json::Value::as_array)
                .is_some_and(|modalities| {
                    modalities
                        .iter()
                        .any(|modality| modality.as_str() == Some("image"))
                }),
        }
    }
}

pub fn client() -> Result<reqwest::Client, String> {
    crate::http_client::model_provider()
}

pub fn endpoint(base_url: &str, route: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if reqwest::Url::parse(base).is_ok_and(|url| !url.path().trim_matches('/').is_empty()) {
        format!("{base}/{route}")
    } else {
        format!("{base}/v1/{route}")
    }
}

pub async fn bounded_body(response: reqwest::Response, limit: usize) -> Result<String, String> {
    if let Some(body) = response
        .extensions()
        .get::<crate::inference_retry::ErrorBody>()
    {
        if body.0.len() > limit {
            return Err(format!(
                "Provider response exceeded the {} KiB safety limit.",
                limit / 1024
            ));
        }
        return Ok(body.0.clone());
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "Provider response was interrupted.".to_string())?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(format!(
                "Provider response exceeded the {} KiB safety limit.",
                limit / 1024
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub fn sanitize_text(text: &str, key: &str) -> String {
    let mut body: String = text
        .chars()
        .filter(|character| !character.is_control() || *character == '\n' || *character == '\t')
        .collect();
    if !key.is_empty() {
        // Stream errors and HTTP error bodies can JSON-escape a credential.
        if let Ok(quoted) = serde_json::to_string(key) {
            let escaped = &quoted[1..quoted.len() - 1];
            body = body.replace(escaped, "[redacted]");
        }
        body = body.replace(key, "[redacted]");
    }
    body.chars().take(512).collect()
}

fn sanitized_error(status: reqwest::StatusCode, body: &str, key: &str) -> String {
    let body = sanitize_text(body, key);
    match status.as_u16() {
        401 | 403 => "Provider rejected the authorization or API key.".into(),
        408 => "Provider request timed out.".into(),
        429 => "Provider rate limit or quota was exceeded.".into(),
        500..=599 => format!("Provider returned {status}."),
        _ if body.trim().is_empty() => format!("Provider request failed ({status})."),
        _ => format!("Provider request failed ({status}): {body}"),
    }
}

pub async fn response_error(response: reqwest::Response, key: &str) -> String {
    let status = response.status();
    let retry_note = crate::inference_retry::response_note(&response);
    let body = bounded_body(response, 16_384).await.unwrap_or_default();
    format!("{}{retry_note}", sanitized_error(status, &body, key))
}

pub async fn list_models(
    provider: &ProviderConnection,
    key: &str,
) -> Result<ModelListResult, String> {
    let base_url = validate_base_url(&provider.base_url)?;
    // Anthropic serves GET {base}/v1/models with the same `{data:[{id}]}`
    // envelope, authenticated by x-api-key + anthropic-version — never Bearer.
    let claude = is_claude_format(&provider.api_format);
    let mut request = client()?
        .get(endpoint(&base_url, "models"))
        .timeout(Duration::from_secs(15))
        .header("User-Agent", "LocalLM-Desktop/0.1");
    if claude {
        request = request.header(
            "anthropic-version",
            crate::claude_adapter::ANTHROPIC_VERSION,
        );
        if !key.is_empty() {
            request = request.header("x-api-key", key);
        }
    // Keyless loopback engines must not receive an empty bearer header.
    } else if !key.is_empty() {
        request = request.bearer_auth(key);
    }
    let response = request.send().await.map_err(|error| {
        if error.is_timeout() {
            "Provider request timed out.".to_string()
        } else {
            "Could not reach provider. Check the HTTPS base URL and network connection.".to_string()
        }
    })?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::METHOD_NOT_ALLOWED
    {
        return Ok(ModelListResult {
            supported: false,
            models: Vec::new(),
            details: Vec::new(),
        });
    }
    let body = bounded_body(
        response,
        if status.is_success() {
            8_388_608
        } else {
            16_384
        },
    )
    .await?;
    if !status.is_success() {
        return Err(sanitized_error(status, &body, key));
    }
    let parsed: ModelListResponse = serde_json::from_str(&body)
        .map_err(|_| "Provider returned a malformed model-list response.".to_string())?;
    let mut ids = Vec::new();
    let mut details = Vec::new();
    for item in parsed.data {
        validate_model_id(&item.id)?;
        if !ids.contains(&item.id) {
            ids.push(item.id.clone());
            let mut model = item.into_model();
            if let Some(controls) = model.controls.as_mut() {
                controls.bind_endpoint(&provider.api_format, &base_url, &model.id);
            }
            details.push(model);
        }
        if ids.len() == MAX_PROVIDER_MODELS {
            break;
        }
    }
    Ok(ModelListResult {
        supported: true,
        models: ids,
        details,
    })
}

pub async fn test_connection(
    provider: &ProviderConnection,
    key: &str,
) -> Result<ProviderTestResult, String> {
    let result = list_models(provider, key).await?;
    let message = if result.supported {
        format!("Connection verified. Model listing returned {} model{}; this test did not send an inference request.", result.models.len(), if result.models.len() == 1 { "" } else { "s" })
    } else {
        "Connection could not be verified via /models. Enter a model ID and click 'Test inference' to verify.".into()
    };
    Ok(ProviderTestResult {
        verified: result.supported,
        model_list_supported: result.supported,
        models: result.models,
        message,
        discovered_models: result.details,
    })
}

fn inference_test_payload(claude: bool, model_id: &str) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "model": model_id,
        "messages": [{"role": "user", "content": "ping"}],
        "stream": false
    });
    let limit = if !claude && crate::inference::openai_reasoning_effort_supported(model_id) {
        "max_completion_tokens"
    } else {
        "max_tokens"
    };
    payload[limit] = serde_json::json!(1);
    payload
}
fn configured_inference_test_payload(
    provider: &ProviderConnection,
    model_id: &str,
) -> Result<serde_json::Value, String> {
    let mut payload = inference_test_payload(is_claude_format(&provider.api_format), model_id);
    if let Some(controls) = provider
        .models
        .iter()
        .find(|model| model.id == model_id)
        .and_then(|model| model.controls.as_ref())
    {
        controls.validate_endpoint(&provider.api_format, &provider.base_url, model_id)?;
        if let Some(object) = payload.as_object_mut() {
            object.remove("max_completion_tokens");
            object.insert("max_tokens".into(), serde_json::json!(1));
        }
        crate::provider_capabilities::apply(&mut payload, controls, &provider.api_format)?;
    }
    if provider.api_format == OPENAI_RESPONSES {
        crate::responses_adapter::request(&payload, model_id)
    } else {
        Ok(payload)
    }
}

/// A one-token request proving the endpoint actually serves inference for
/// `model_id`. The response must parse as the protocol's JSON shape — a
/// successful status alone is not verification (proxies and landing pages
/// answer 200/HTML without being model APIs).
pub async fn test_inference(
    provider: &ProviderConnection,
    key: &str,
    model_id: &str,
) -> Result<ProviderTestResult, String> {
    validate_model_id(model_id)?;
    let base_url = validate_base_url(&provider.base_url)?;
    let claude = is_claude_format(&provider.api_format);
    let responses = provider.api_format == OPENAI_RESPONSES;
    let payload = configured_inference_test_payload(provider, model_id)?;
    let mut request = client()?
        .post(endpoint(
            &base_url,
            if claude {
                "messages"
            } else if responses {
                "responses"
            } else {
                "chat/completions"
            },
        ))
        .timeout(Duration::from_secs(10))
        .header("User-Agent", "LocalLM-Desktop/0.1")
        .json(&payload);
    if claude {
        request = request.header(
            "anthropic-version",
            crate::claude_adapter::ANTHROPIC_VERSION,
        );
        if !key.is_empty() {
            request = request.header("x-api-key", key);
        }
    } else if !key.is_empty() {
        request = request.bearer_auth(key);
    }
    let route = if claude {
        "messages"
    } else if responses {
        "responses"
    } else {
        "chat completions"
    };
    let response = request.send().await.map_err(|error| {
        if error.is_timeout() {
            "Provider inference request timed out (10s limit).".to_string()
        } else {
            format!("Could not reach provider {route} endpoint. Check base URL and network.")
        }
    })?;

    let status = response.status();
    let body = bounded_body(response, 16_384).await?;
    if !status.is_success() {
        return Err(sanitized_error(status, &body, key));
    }
    let parsed: serde_json::Value = serde_json::from_str(&body).map_err(|_| {
        format!("Provider {route} endpoint did not return a valid JSON response. Check that the base URL points at an API endpoint.")
    })?;
    let plausible = if responses {
        parsed["object"] == "response"
            && parsed["id"].as_str().is_some_and(|id| !id.is_empty())
            && matches!(parsed["status"].as_str(), Some("completed" | "incomplete"))
            && parsed["output"].is_array()
            && parsed["error"].is_null()
    } else if claude {
        // Native Messages replies `{id, type:"message", role:"assistant", content:[..]}`.
        parsed.get("type").and_then(serde_json::Value::as_str) == Some("message")
            && parsed.get("role").and_then(serde_json::Value::as_str) == Some("assistant")
            && parsed
                .get("content")
                .and_then(serde_json::Value::as_array)
                .is_some()
    } else {
        parsed
            .get("choices")
            .and_then(serde_json::Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"))
            .is_some_and(|message| {
                message.get("role").and_then(serde_json::Value::as_str) == Some("assistant")
                    && (message.get("content").is_some()
                        || message
                            .get("tool_calls")
                            .and_then(serde_json::Value::as_array)
                            .is_some())
            })
    };
    if !plausible {
        return Err(format!(
            "Provider {route} response was valid JSON but not a model reply."
        ));
    }
    Ok(ProviderTestResult {
        verified: true,
        model_list_supported: false,
        models: vec![model_id.to_string()],
        message: format!(
            "Inference verified: model '{model_id}' successfully completed a test request."
        ),
        discovered_models: Vec::new(),
    })
}

pub fn merge_listed_models(existing: &[RemoteModel], listed: &[RemoteModel]) -> Vec<RemoteModel> {
    // Keep manually entered IDs, including models hidden from the public catalog.
    // Explicit capacities and capability overrides survive subsequent refreshes.
    let mut models = existing.to_vec();
    for discovered in listed {
        if let Some(saved) = models.iter_mut().find(|model| model.id == discovered.id) {
            let configured = saved.context_length.is_some();
            saved.context_length = saved.context_length.or(discovered.context_length);
            saved.max_output_tokens = saved.max_output_tokens.or(discovered.max_output_tokens);
            if saved.tool_support == ToolSupport::Unknown {
                saved.tool_support = discovered.tool_support.clone();
            }
            if !configured {
                saved.supports_images |= discovered.supports_images;
            }
            if saved.controls.as_ref().is_none_or(|controls| {
                controls.source == crate::provider_capabilities::Source::ProviderReported
            }) {
                saved.controls = discovered.controls.clone().or_else(|| {
                    saved
                        .controls
                        .as_ref()
                        .map(|_| crate::provider_capabilities::Controls {
                            source: crate::provider_capabilities::Source::ProviderReported,
                            ..Default::default()
                        })
                });
            }
        } else if models.len() < MAX_PROVIDER_MODELS {
            models.push(discovered.clone());
        }
    }
    models
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inference_probes_use_the_same_token_limit_field_as_chat() {
        for model in ["gpt-5", "o3", "gpt-5-mini-2025-08-07"] {
            let payload = inference_test_payload(false, model);
            assert_eq!(payload["max_completion_tokens"], 1);
            assert!(payload.get("max_tokens").is_none());
        }
        for (claude, model) in [(false, "custom-model"), (true, "claude-model")] {
            let payload = inference_test_payload(claude, model);
            assert_eq!(payload["max_tokens"], 1);
            assert!(payload.get("max_completion_tokens").is_none());
        }
        use crate::provider_capabilities::{Controls, TokenField};
        for (format, field, wire) in [
            (OPENAI_CHAT_COMPLETIONS, TokenField::MaxTokens, "max_tokens"),
            (
                OPENAI_CHAT_COMPLETIONS,
                TokenField::MaxCompletionTokens,
                "max_completion_tokens",
            ),
            (
                OPENAI_RESPONSES,
                TokenField::MaxOutputTokens,
                "max_output_tokens",
            ),
            (CLAUDE_MESSAGES, TokenField::MaxTokens, "max_tokens"),
        ] {
            let mut model = model();
            model.id = "gpt-5".into();
            model.controls = Some(Controls {
                token_limit_field: field,
                ..Default::default()
            });
            let provider = ProviderConnection {
                id: "fixture".into(),
                name: "Fixture".into(),
                api_format: format.into(),
                base_url: "https://example.test/v1".into(),
                verified: true,
                last_tested_at: None,
                has_api_key: true,
                models: vec![model],
            };
            let payload = configured_inference_test_payload(&provider, "gpt-5").unwrap();
            assert_eq!(payload[wire], 1);
            for other in ["max_tokens", "max_completion_tokens", "max_output_tokens"] {
                if other != wire {
                    assert!(payload.get(other).is_none(), "{format}: {other}");
                }
            }
        }
    }

    #[test]
    fn endpoints_preserve_explicit_api_prefixes() {
        assert_eq!(
            endpoint("https://example.test", "models"),
            "https://example.test/v1/models"
        );
        assert_eq!(
            endpoint("https://example.test/v1/", "models"),
            "https://example.test/v1/models"
        );
        assert_eq!(
            endpoint("https://example.test/api/v2", "chat/completions"),
            "https://example.test/api/v2/chat/completions"
        );
        assert_eq!(
            endpoint("https://example.test/openai", "models"),
            "https://example.test/openai/models"
        );
    }

    fn model() -> RemoteModel {
        RemoteModel {
            controls: None,
            id: "manual-model".into(),
            context_length: Some(32_768),
            max_output_tokens: Some(4_096),
            supports_images: false,
            tool_support: ToolSupport::Supported,
        }
    }

    fn draft() -> ProviderDraft {
        ProviderDraft {
            id: None,
            name: "Custom endpoint".into(),
            api_format: OPENAI_CHAT_COMPLETIONS.into(),
            base_url: "https://api.example.test/prefix/v1".into(),
            api_key: Some("fixture-secret-one".into()),
            models: vec![model()],
        }
    }

    #[test]
    fn saved_provider_and_encrypted_key_survive_reopening_then_can_be_rotated_and_deleted() {
        let directory = tempfile::tempdir().unwrap();
        let database = directory.path().join("providers.sqlite");
        let vault_path = directory.path().join("vault");
        let vault = crate::vault::Vault::new(vault_path.clone());
        let store = crate::store::Store::open(&database).unwrap();
        let saved = save_connection(&store, &vault, draft()).unwrap();
        let other = save_connection(&store, &vault, draft()).unwrap();
        assert_ne!(
            saved.id, other.id,
            "same provider can have multiple accounts"
        );
        let sealed =
            std::fs::read(vault_path.join(format!("{}.sealed", credential_id(&saved.id)))).unwrap();
        assert!(!sealed
            .windows(b"fixture-secret-one".len())
            .any(|part| part == b"fixture-secret-one"));
        assert!(!serde_json::to_string(&saved)
            .unwrap()
            .contains("fixture-secret"));
        store
            .update_provider_test(&saved.id, true, Some(100), &saved.models)
            .unwrap();
        let choice = ModelSelection {
            provider_id: Some(saved.id.clone()),
            model_id: model().id,
        };
        store.save_preferred_model(&choice).unwrap();
        let chat = store.create_conversation().unwrap();
        drop(store);
        let store = crate::store::Store::open(&database).unwrap();
        let vault = crate::vault::Vault::new(vault_path);
        assert_eq!(store.preferred_model().unwrap(), choice);
        assert_eq!(
            vault.load(&credential_id(&saved.id)).unwrap().unwrap(),
            b"fixture-secret-one"
        );
        let mut edit = draft();
        edit.id = Some(saved.id.clone());
        edit.name = "Renamed connection".into();
        edit.api_key = None;
        let renamed = save_connection(&store, &vault, edit.clone()).unwrap();
        assert!(
            renamed.verified,
            "metadata-only edits keep successful authentication"
        );
        edit.api_key = Some("fixture-secret-two".into());
        let rotated = save_connection(&store, &vault, edit).unwrap();
        assert!(!rotated.verified);
        assert_eq!(
            vault.load(&credential_id(&saved.id)).unwrap().unwrap(),
            b"fixture-secret-two"
        );
        store
            .update_provider_test(&saved.id, true, Some(101), &saved.models)
            .unwrap();
        delete_api_key(&store, &vault, &saved.id).unwrap();
        assert!(!vault.contains(&credential_id(&saved.id)).unwrap());
        let retained = store.provider(&saved.id).unwrap().unwrap();
        assert!(!retained.verified);
        assert_eq!(retained.models, saved.models);
        assert_eq!(
            store.conversation_model(&chat.id).unwrap(),
            (choice.clone(), false)
        );
        assert!(vault.contains(&credential_id(&other.id)).unwrap());
        delete_connection(&store, &vault, &saved.id).unwrap();
        assert!(store.provider(&saved.id).unwrap().is_none());
        assert_eq!(store.preferred_model().unwrap(), ModelSelection::default());
        assert_eq!(store.conversation_model(&chat.id).unwrap(), (choice, true));
        assert!(store.provider(&other.id).unwrap().is_some());
    }

    #[test]
    fn draft_connection_validates_without_persisting() {
        // A valid draft yields a testable connection carrying the draft's
        // endpoint and models — no store or vault is involved.
        let connection = connection_from_draft(draft()).unwrap();
        assert_eq!(connection.base_url, "https://api.example.test/prefix/v1");
        assert_eq!(connection.models.len(), 1);
        assert!(connection.has_api_key);
        assert!(!connection.verified);
        // The same validations as save apply.
        let mut bad = draft();
        bad.base_url = "http://api.example.test/v1".into();
        assert!(connection_from_draft(bad).unwrap_err().contains("HTTPS"));
        let mut subscription = draft();
        subscription.api_format = CHATGPT_SUBSCRIPTION.into();
        assert!(connection_from_draft(subscription).is_err());
        let mut claude = draft();
        claude.api_format = CLAUDE_MESSAGES.into();
        claude.api_key = None;
        assert!(connection_from_draft(claude)
            .unwrap_err()
            .contains("API key"));
    }

    #[test]
    fn a_saved_key_cannot_follow_a_changed_endpoint_or_api_format() {
        let directory = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&directory.path().join("providers.sqlite")).unwrap();
        let vault = crate::vault::Vault::new(directory.path().join("vault"));
        let saved = save_connection(&store, &vault, draft()).unwrap();
        for (url, format) in [
            ("https://different.example.test/v1", OPENAI_CHAT_COMPLETIONS),
            (saved.base_url.as_str(), CLAUDE_MESSAGES),
        ] {
            let mut edit = draft();
            edit.id = Some(saved.id.clone());
            edit.base_url = url.into();
            edit.api_format = format.into();
            edit.api_key = None;
            assert!(save_connection(&store, &vault, edit.clone())
                .unwrap_err()
                .contains("new endpoint"));
            assert_eq!(
                store.provider(&saved.id).unwrap().unwrap().base_url,
                saved.base_url
            );
            edit.api_key = Some("new-endpoint-key".into());
            assert!(save_connection(&store, &vault, edit).is_ok());
            // Restore the original endpoint and key for the next case.
            let mut original = draft();
            original.id = Some(saved.id.clone());
            save_connection(&store, &vault, original).unwrap();
        }
        let mut stale = draft();
        stale.id = Some("deleted-provider".into());
        assert!(save_connection(&store, &vault, stale)
            .unwrap_err()
            .contains("no longer exists"));
        assert!(!vault.contains(&credential_id("deleted-provider")).unwrap());
    }

    #[test]
    fn failed_vault_write_restores_previous_provider_metadata() {
        let directory = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&directory.path().join("providers.sqlite")).unwrap();
        let vault_path = directory.path().join("vault");
        let vault = crate::vault::Vault::new(vault_path.clone());
        let original = ProviderConnection {
            id: "fixture".into(),
            name: "Original".into(),
            api_format: OPENAI_CHAT_COMPLETIONS.into(),
            base_url: "http://localhost:8080".into(),
            verified: true,
            last_tested_at: Some(123),
            models: vec![model()],
            has_api_key: false,
        };
        store.save_provider(&original).unwrap();
        // A directory at the credential filename forces atomic rename to fail.
        std::fs::create_dir_all(vault_path.join(format!("{}.sealed", credential_id("fixture"))))
            .unwrap();
        let mut edit = draft();
        edit.id = Some("fixture".into());
        assert!(save_connection(&store, &vault, edit).is_err());
        assert_eq!(store.provider("fixture").unwrap().unwrap(), original);
    }

    #[test]
    fn discovery_fills_missing_api_output_capacity_and_preserves_manual_budget() {
        let mut advertised = model();
        advertised.context_length = Some(1_048_576);
        advertised.max_output_tokens = Some(262_144);
        let mut missing = advertised.clone();
        missing.max_output_tokens = None;
        let imported = merge_listed_models(&[missing], &[advertised.clone()]);
        assert_eq!(imported[0].max_output_tokens, Some(262_144));
        let mut manual = advertised.clone();
        manual.max_output_tokens = Some(131_072);
        let preserved = merge_listed_models(&[manual], &[advertised]);
        assert_eq!(preserved[0].max_output_tokens, Some(131_072));
    }

    #[test]
    fn discovery_imports_reported_capacities_without_inventing_missing_limits() {
        let response: ModelListResponse = serde_json::from_value(serde_json::json!({"data": [
            {"id":"router/model", "context_length":131072,
             "top_provider":{"context_length":65536,"max_completion_tokens":8192},
             "supported_parameters":["tools","temperature"],
             "architecture":{"input_modalities":["text","image"]}},
            {"id":"groq/model", "context_window":32768},
            {"id":"basic/model"},
            {"id":"invalid-metadata", "context_length":0,"max_output_tokens":"unknown","supported_parameters":null}
        ]})).unwrap();
        let models: Vec<_> = response
            .data
            .into_iter()
            .map(ModelListItem::into_model)
            .collect();
        assert_eq!(models[0].context_length, Some(65536));
        assert_eq!(models[0].max_output_tokens, Some(8192));
        assert_eq!(models[0].tool_support, ToolSupport::Supported);
        assert!(models[0].supports_images);
        let controls = models[0].controls.as_ref().unwrap();
        assert_eq!(
            controls.source,
            crate::provider_capabilities::Source::ProviderReported
        );
        assert_eq!(controls.temperature, ToolSupport::Supported);
        assert_eq!(controls.top_p, ToolSupport::Unsupported);
        assert_eq!(
            controls.structured_output,
            ToolSupport::Unknown,
            "response-format support alone does not establish strict schema support"
        );
        assert_eq!(models[1].context_length, Some(32768));
        for model in &models[2..] {
            assert_eq!(model.context_length, None);
            assert_eq!(model.max_output_tokens, None);
            assert_eq!(model.tool_support, ToolSupport::Unknown);
        }
        let merged = merge_listed_models(&[model()], &models);
        assert_eq!(merged[0], model(), "manual IDs survive catalog refresh");
        assert_eq!(merged.len(), 5);
        assert_eq!(
            merge_listed_models(&merged, &models),
            merged,
            "refresh does not duplicate IDs"
        );
        let mut manual = models[0].clone();
        manual.controls = Some(crate::provider_capabilities::Controls {
            structured_output: ToolSupport::Supported,
            ..Default::default()
        });
        assert_eq!(
            merge_listed_models(&[manual.clone()], &models)[0].controls,
            manual.controls,
            "manual controls survive metadata refresh"
        );
        let mut unknown = models[0].clone();
        unknown.controls = None;
        let refreshed = merge_listed_models(&[models[0].clone()], &[unknown]);
        assert_eq!(
            refreshed[0].controls.as_ref().unwrap().structured_output,
            ToolSupport::Unknown
        );
        let mut configured = models[0].clone();
        configured.context_length = Some(4096);
        configured.supports_images = false;
        configured.tool_support = ToolSupport::Unsupported;
        assert_eq!(
            merge_listed_models(&[configured.clone()], &models)[0],
            configured
        );
        validate_models(&merged).unwrap();
    }

    #[test]
    fn provider_urls_require_https_except_loopback() {
        assert_eq!(
            validate_base_url("https://api.example.test/v1/").unwrap(),
            "https://api.example.test/v1"
        );
        assert!(validate_base_url("http://api.example.test/v1").is_err());
        assert!(validate_base_url("http://127.0.0.1:8080/v1").is_ok());
        assert!(validate_base_url("https://user:secret@api.example.test").is_err());
        assert!(validate_base_url("https://api.example.test?key=secret").is_err());
    }

    #[test]
    fn custom_provider_keys_are_optional_and_model_limits_are_validated() {
        let mut draft = ProviderDraft {
            id: None,
            name: "Fixture".into(),
            api_format: OPENAI_CHAT_COMPLETIONS.into(),
            base_url: "https://api.example.test".into(),
            api_key: None,
            models: vec![model()],
        };
        assert!(validate_provider_draft(&draft, false).is_ok());
        draft.api_key = Some("fixture-key".into());
        assert!(validate_provider_draft(&draft, false).is_ok());
        draft.api_key = None;
        assert!(validate_provider_draft(&draft, true).is_ok());
        draft.models[0].context_length = Some(64);
        assert!(validate_provider_draft(&draft, true).is_err());

        let sub_draft = ProviderDraft {
            id: None,
            name: "ChatGPT Sub".into(),
            api_format: CHATGPT_SUBSCRIPTION.into(),
            base_url: "https://chatgpt.com/backend-api/codex".into(),
            api_key: None,
            models: vec![model()],
        };
        assert!(validate_provider_draft(&sub_draft, false).is_ok());
    }

    #[test]
    fn keyless_custom_https_and_loopback_engines_are_supported() {
        let mut draft = ProviderDraft {
            id: None,
            name: "FreeToken".into(),
            api_format: OPENAI_CHAT_COMPLETIONS.into(),
            base_url: "http://127.0.0.1:1919".into(),
            api_key: None,
            models: vec![model()],
        };
        assert!(validate_provider_draft(&draft, false).is_ok());
        draft.api_format = FREETOKEN_OPENAI_COMPAT.into();
        assert!(validate_provider_draft(&draft, false).is_ok());
        draft.base_url = "http://192.168.1.10:1919".into();
        assert!(
            validate_provider_draft(&draft, false).is_err(),
            "non-loopback HTTP hosts still need a key"
        );
        draft.base_url = "https://api.example.test".into();
        assert!(validate_provider_draft(&draft, false).is_err());
        draft.api_format = OPENAI_CHAT_COMPLETIONS.into();
        assert!(validate_provider_draft(&draft, false).is_ok());
        assert!(is_loopback_base_url("http://localhost:1919"));
        assert!(!is_loopback_base_url("https://example.test"));
    }

    #[test]
    fn selection_rejects_missing_remote_model_ids_but_allows_local_selection() {
        assert!(ModelSelection {
            provider_id: None,
            model_id: String::new()
        }
        .validate()
        .is_ok());
        assert!(ModelSelection {
            provider_id: Some("provider".into()),
            model_id: String::new()
        }
        .validate()
        .is_err());
        assert!(ModelSelection {
            provider_id: Some("provider".into()),
            model_id: "model".into()
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn streamed_provider_errors_redact_keys_and_bound_untrusted_text() {
        let message = sanitize_text("provider echoed fixture-secret\u{0000}", "fixture-secret");
        assert_eq!(message, "provider echoed [redacted]");
        assert!(sanitize_text(&"x".repeat(600), "fixture-secret").len() <= 512);
        let key = "fixture\\secret\"quoted";
        let error = serde_json::json!({"message":format!("bad key {key}")}).to_string();
        assert_eq!(
            sanitize_text(&error, key),
            r#"{"message":"bad key [redacted]"}"#
        );
        assert_eq!(
            sanitize_text("fixture-\u{0000}secret", "fixture-secret"),
            "[redacted]"
        );
    }

    #[test]
    fn claude_format_is_explicit_and_never_inferred_from_openai() {
        assert!(is_claude_format(CLAUDE_MESSAGES));
        assert!(!is_claude_format(OPENAI_CHAT_COMPLETIONS));
        assert!(!is_claude_format(FREETOKEN_OPENAI_COMPAT));
        assert!(!is_subscription_format(CLAUDE_MESSAGES));
    }

    #[test]
    fn ipv6_and_bracketed_loopback_hosts_are_local() {
        assert!(is_loopback_base_url("http://[::1]:8080/v1"));
        assert!(is_loopback_base_url("http://[::1]"));
        assert!(validate_base_url("http://[::1]:8080/v1").is_ok());
        assert!(!requires_api_key(
            OPENAI_CHAT_COMPLETIONS,
            "https://api.example.test"
        ));
        assert!(!requires_api_key(
            OPENAI_CHAT_COMPLETIONS,
            "http://127.0.0.1:8080"
        ));
        assert!(!requires_api_key(
            OPENAI_RESPONSES,
            "https://api.example.test/v1"
        ));
        assert!(!requires_api_key(
            OPENAI_RESPONSES,
            "http://127.0.0.1:8080/v1"
        ));
        assert!(
            requires_api_key(CLAUDE_MESSAGES, "http://127.0.0.1:8080"),
            "Claude always needs a key, even on loopback"
        );
        assert!(!requires_api_key(
            CHATGPT_SUBSCRIPTION,
            "https://chatgpt.com/backend-api/codex"
        ));
    }

    #[test]
    fn claude_drafts_require_a_key_even_on_loopback() {
        let mut draft = ProviderDraft {
            id: None,
            name: "Claude".into(),
            api_format: CLAUDE_MESSAGES.into(),
            base_url: "http://127.0.0.1:8080".into(),
            api_key: None,
            models: vec![model()],
        };
        assert!(validate_provider_draft(&draft, false).is_err());
        draft.api_key = Some("claude-key".into());
        assert!(validate_provider_draft(&draft, false).is_ok());
    }

    #[test]
    fn saved_keyless_custom_providers_share_the_key_policy_for_tests_chat_and_children() {
        let store = crate::store::Store::open_memory().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let vault = crate::vault::Vault::new(directory.path().join("vault"));
        for base_url in [
            "https://custom.example.test",
            "http://localhost:11434",
            "http://127.0.0.1:1234/v1",
        ] {
            let mut draft = draft();
            draft.base_url = base_url.into();
            draft.api_key = None;
            let saved = save_connection(&store, &vault, draft).unwrap();
            assert!(!saved.has_api_key);
            assert_eq!(
                resolve_api_key(&saved, vault.load(&credential_id(&saved.id)).unwrap()).unwrap(),
                ""
            );
            assert_eq!(
                resolve_api_key(&saved, Some(b"fixture-key".to_vec())).unwrap(),
                "fixture-key"
            );
            assert!(resolve_api_key(&saved, Some(vec![0xff])).is_err());
            let mut claude = saved;
            claude.api_format = CLAUDE_MESSAGES.into();
            assert!(resolve_api_key(&claude, None).is_err());
        }
    }

    /// Serve exactly one HTTP request and capture (lowercased headers, body).
    async fn serve_once(
        status: &'static str,
        response_body: &'static str,
    ) -> (
        std::net::SocketAddr,
        tokio::task::JoinHandle<(String, String)>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
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
                        .and_then(|line| line.parse().ok())
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        let body = String::from_utf8_lossy(&request[end + 4..end + 4 + length])
                            .into_owned();
                        let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response_body}", response_body.len());
                        stream.write_all(response.as_bytes()).await.unwrap();
                        return (headers, body);
                    }
                }
            }
        });
        (address, handle)
    }

    fn fixture(format: &str, address: std::net::SocketAddr) -> ProviderConnection {
        ProviderConnection {
            id: "fixture".into(),
            name: "Fixture".into(),
            api_format: format.into(),
            base_url: format!("http://{address}"),
            verified: false,
            last_tested_at: None,
            models: vec![model()],
            has_api_key: true,
        }
    }

    #[tokio::test]
    async fn claude_model_list_uses_x_api_key_and_version_never_bearer() {
        let body = r#"{"data":[{"id":"claude-fixture","type":"model","display_name":"Fixture","created_at":"2026-01-01T00:00:00Z"}],"has_more":false}"#;
        let (address, server) = serve_once("200 OK", body).await;
        let result = list_models(&fixture(CLAUDE_MESSAGES, address), "claude-key")
            .await
            .unwrap();
        let (headers, _) = server.await.unwrap();
        assert!(headers.starts_with("get /v1/models "), "{headers}");
        assert!(headers.contains("x-api-key: claude-key"), "{headers}");
        assert!(
            headers.contains("anthropic-version: 2023-06-01"),
            "{headers}"
        );
        assert!(
            !headers.contains("authorization"),
            "Claude must not receive bearer auth: {headers}"
        );
        assert!(result.supported);
        assert_eq!(result.models, vec!["claude-fixture".to_string()]);
    }

    #[tokio::test]
    async fn openai_model_list_uses_bearer_auth() {
        let body = r#"{"data":[{"id":"acme-1"}]}"#;
        let (address, server) = serve_once("200 OK", body).await;
        let result = list_models(&fixture(OPENAI_CHAT_COMPLETIONS, address), "fixture-secret")
            .await
            .unwrap();
        let (headers, _) = server.await.unwrap();
        assert!(headers.starts_with("get /v1/models "), "{headers}");
        assert!(
            headers.contains("authorization: bearer fixture-secret"),
            "{headers}"
        );
        assert!(result.supported);
        assert_eq!(result.models, vec!["acme-1".to_string()]);
    }

    #[tokio::test]
    async fn claude_inference_posts_native_messages_payload() {
        let body = r#"{"id":"msg_fixture","type":"message","role":"assistant","content":[{"type":"text","text":"ok"}],"stop_reason":"max_tokens"}"#;
        let (address, server) = serve_once("200 OK", body).await;
        let result = test_inference(
            &fixture(CLAUDE_MESSAGES, address),
            "claude-key",
            "claude-fixture",
        )
        .await
        .unwrap();
        let (headers, payload) = server.await.unwrap();
        let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(headers.starts_with("post /v1/messages "), "{headers}");
        assert!(headers.contains("x-api-key: claude-key"), "{headers}");
        assert!(
            headers.contains("anthropic-version: 2023-06-01"),
            "{headers}"
        );
        assert!(!headers.contains("authorization"), "{headers}");
        assert_eq!(payload["model"], "claude-fixture");
        assert_eq!(payload["max_tokens"], 1);
        assert_eq!(payload["messages"][0]["content"], "ping");
        assert!(result.verified);
    }

    #[tokio::test]
    async fn openai_inference_posts_chat_completions_and_reads_choices() {
        let body = r#"{"id":"cmpl-fixture","choices":[{"message":{"role":"assistant","content":"ok"},"finish_reason":"length"}]}"#;
        let (address, server) = serve_once("200 OK", body).await;
        let result = test_inference(
            &fixture(OPENAI_CHAT_COMPLETIONS, address),
            "fixture-secret",
            "acme-1",
        )
        .await
        .unwrap();
        let (headers, payload) = server.await.unwrap();
        let payload: serde_json::Value = serde_json::from_str(&payload).unwrap();
        assert!(
            headers.starts_with("post /v1/chat/completions "),
            "{headers}"
        );
        assert!(
            headers.contains("authorization: bearer fixture-secret"),
            "{headers}"
        );
        assert_eq!(payload["model"], "acme-1");
        assert_eq!(payload["max_tokens"], 1);
        assert!(result.verified);
        assert_eq!(result.models, vec!["acme-1".to_string()]);
    }

    #[tokio::test]
    async fn a_successful_html_or_wrong_shape_reply_is_not_verification() {
        let (address, server) = serve_once("200 OK", "<html><body>welcome</body></html>").await;
        let error = test_inference(&fixture(OPENAI_CHAT_COMPLETIONS, address), "k", "acme-1")
            .await
            .unwrap_err();
        assert!(error.contains("valid JSON"), "{error}");
        server.await.unwrap();

        let (address, server) = serve_once("200 OK", r#"{"detail":"queued"}"#).await;
        let error = test_inference(&fixture(OPENAI_CHAT_COMPLETIONS, address), "k", "acme-1")
            .await
            .unwrap_err();
        assert!(error.contains("not a model reply"), "{error}");
        server.await.unwrap();

        let (address, server) = serve_once("200 OK", r#"{"results":[]}"#).await;
        let error = test_inference(&fixture(CLAUDE_MESSAGES, address), "k", "claude-x")
            .await
            .unwrap_err();
        assert!(error.contains("not a model reply"), "{error}");
        server.await.unwrap();

        for (format, body) in [
            (OPENAI_CHAT_COMPLETIONS, r#"{"choices":[]}"#),
            (OPENAI_CHAT_COMPLETIONS, r#"{"choices":[{}]}"#),
            (OPENAI_CHAT_COMPLETIONS, r#"{"choices":[{"message":{}}]}"#),
            (CLAUDE_MESSAGES, r#"{"type":"message"}"#),
            (CLAUDE_MESSAGES, r#"{"content":[]}"#),
            (
                CLAUDE_MESSAGES,
                r#"{"type":"message","role":"user","content":[]}"#,
            ),
        ] {
            let (address, server) = serve_once("200 OK", body).await;
            let error = test_inference(&fixture(format, address), "k", "fixture-model")
                .await
                .unwrap_err();
            assert!(error.contains("not a model reply"), "{format}: {error}");
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn inference_error_bodies_are_sanitized() {
        let (address, server) = serve_once(
            "401 Unauthorized",
            r#"{"error":{"message":"bad key fixture-secret"}}"#,
        )
        .await;
        let error = test_inference(
            &fixture(OPENAI_CHAT_COMPLETIONS, address),
            "fixture-secret",
            "acme-1",
        )
        .await
        .unwrap_err();
        assert!(error.contains("authorization or API key"), "{error}");
        assert!(!error.contains("fixture-secret"), "{error}");
        server.await.unwrap();
    }
}
