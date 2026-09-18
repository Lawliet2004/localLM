use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const OPENAI_CHAT_COMPLETIONS: &str = "openai-chat-completions";
pub const CHATGPT_SUBSCRIPTION: &str = "chatgpt-subscription";
pub const GROK_SUBSCRIPTION: &str = "grok-subscription";
pub const FREETOKEN_OPENAI_COMPAT: &str = "freetoken-openai";
pub const CLAUDE_MESSAGES: &str = "claude-messages";

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

/// FreeToken documents `/v1/models` and streaming `/v1/chat/completions`
/// at `http://127.0.0.1:1919`. Its engine does not accept an API key
/// for verified loopback endpoints.
pub fn is_freetoken_base_url(raw: &str) -> bool {
    reqwest::Url::parse(raw.trim())
        .ok()
        .and_then(|url| url.host_str().map(|host| matches!(host, "localhost" | "127.0.0.1" | "::1")))
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
        && draft.api_format != CHATGPT_SUBSCRIPTION
        && draft.api_format != GROK_SUBSCRIPTION
        && draft.api_format != FREETOKEN_OPENAI_COMPAT
        && draft.api_format != CLAUDE_MESSAGES
    {
        return Err("Only OpenAI-compatible Chat Completions, native Claude Messages, Subscription providers, or FreeToken are supported.".into());
    }
    validate_base_url(&draft.base_url)?;
    let is_sub = is_subscription_format(&draft.api_format);
    let loopback = is_loopback_base_url(&draft.base_url);
    let is_freetoken = draft.api_format == FREETOKEN_OPENAI_COMPAT && is_freetoken_base_url(&draft.base_url);
    if draft.api_key.is_none() && !existing && !is_sub && !loopback && !is_freetoken {
        return Err("An API key is required when adding a provider. Loopback engines and FreeToken may omit one.".into());
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
    validate_models(&draft.models)
}

pub fn validate_models(models: &[RemoteModel]) -> Result<(), String> {
    if models.len() > 256 {
        return Err("A provider can store at most 256 model IDs.".into());
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
    let local = matches!(host, "localhost" | "127.0.0.1" | "::1");
    if url.scheme() != "https" && !(local && url.scheme() == "http") {
        return Err(
            "Remote providers require HTTPS. HTTP is allowed only for localhost services.".into(),
        );
    }
    Ok(raw.trim().trim_end_matches('/').to_string())
}

/// True when the base URL points at this machine. Verified loopback-only
/// engines (a local llama.cpp server, FreeToken) may run without any
/// authentication; remote hosts always require explicit credentials.
pub fn is_loopback_base_url(raw: &str) -> bool {
    reqwest::Url::parse(raw.trim())
        .ok()
        .and_then(|url| url.host_str().map(|host| matches!(host, "localhost" | "127.0.0.1" | "::1")))
        .unwrap_or(false)
}

#[derive(Debug)]
pub struct ModelListResult {
    pub supported: bool,
    pub models: Vec<String>,
}

#[derive(Deserialize)]
struct ModelListResponse {
    data: Vec<ModelListItem>,
}

#[derive(Deserialize)]
struct ModelListItem {
    id: String,
}

pub fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|_| "Could not create the provider HTTP client.".into())
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
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "Provider response was interrupted.".to_string())?;
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(format!("Provider response exceeded the {} KiB safety limit.", limit / 1024));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub fn sanitize_text(text: &str, key: &str) -> String {
    let body = if key.is_empty() {
        text.to_string()
    } else {
        text.replace(key, "[redacted]")
    };
    body.chars()
        .filter(|character| !character.is_control() || *character == '\n' || *character == '\t')
        .take(512)
        .collect()
}

fn sanitized_error(status: reqwest::StatusCode, body: &str, key: &str) -> String {
    let body = sanitize_text(body, key);
    match status.as_u16() {
        401 | 403 => "Provider rejected the authorization or API key.".into(),
        408 => "Provider request timed out.".into(),
        429 => {
            "Provider rate limit or quota was exceeded. No automatic retry was attempted.".into()
        }
        500..=599 => format!("Provider returned {status}. No automatic retry was attempted."),
        _ if body.trim().is_empty() => format!("Provider request failed ({status})."),
        _ => format!("Provider request failed ({status}): {body}"),
    }
}

pub async fn response_error(response: reqwest::Response, key: &str) -> String {
    let status = response.status();
    let body = bounded_body(response, 16_384).await.unwrap_or_default();
    sanitized_error(status, &body, key)
}

pub async fn list_models(
    provider: &ProviderConnection,
    key: &str,
) -> Result<ModelListResult, String> {
    let base_url = validate_base_url(&provider.base_url)?;
    let mut request = client()?.get(endpoint(&base_url, "models"));
    // Keyless loopback engines must not receive an empty bearer header.
    if !key.is_empty() {
        request = request.bearer_auth(key);
    }
    let response = request.send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                "Provider request timed out.".to_string()
            } else {
                "Could not reach provider. Check the HTTPS base URL and network connection."
                    .to_string()
            }
        })?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::METHOD_NOT_ALLOWED
    {
        return Ok(ModelListResult {
            supported: false,
            models: Vec::new(),
        });
    }
    let body = bounded_body(response, if status.is_success() { 2_097_152 } else { 16_384 }).await?;
    if !status.is_success() {
        return Err(sanitized_error(status, &body, key));
    }
    let parsed: ModelListResponse = serde_json::from_str(&body)
        .map_err(|_| "Provider returned a malformed model-list response.".to_string())?;
    let mut ids = Vec::new();
    for item in parsed.data {
        validate_model_id(&item.id)?;
        if !ids.contains(&item.id) {
            ids.push(item.id);
        }
        if ids.len() == 256 {
            break;
        }
    }
    Ok(ModelListResult {
        supported: true,
        models: ids,
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
    })
}

pub async fn test_inference(
    provider: &ProviderConnection,
    key: &str,
    model_id: &str,
) -> Result<ProviderTestResult, String> {
    validate_model_id(model_id)?;
    let base_url = validate_base_url(&provider.base_url)?;
    let payload = serde_json::json!({
        "model": model_id,
        "messages": [{"role": "user", "content": "ping"}],
        "max_tokens": 1,
        "stream": false
    });
    let mut request = client()?
        .post(endpoint(&base_url, "chat/completions"))
        .header("User-Agent", "LocalLM-Desktop/0.1")
        .json(&payload);
    if !key.is_empty() {
        request = request.bearer_auth(key);
    }
    let response = request.send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                "Provider inference request timed out (10s limit).".to_string()
            } else {
                "Could not reach provider chat completions endpoint. Check base URL and network.".to_string()
            }
        })?;

    let status = response.status();
    let body = bounded_body(response, 16_384).await?;
    if !status.is_success() {
        return Err(sanitized_error(status, &body, key));
    }
    Ok(ProviderTestResult {
        verified: true,
        model_list_supported: false,
        models: vec![model_id.to_string()],
        message: format!("Inference verified: model '{model_id}' successfully completed a test request."),
    })
}

pub fn merge_listed_models(existing: &[RemoteModel], ids: &[String]) -> Vec<RemoteModel> {
    ids.iter()
        .map(|id| {
            existing
                .iter()
                .find(|model| model.id == *id)
                .cloned()
                .unwrap_or(RemoteModel {
                    id: id.clone(),
                    context_length: None,
                    max_output_tokens: None,
                    supports_images: false, tool_support: ToolSupport::Unknown,
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_preserve_explicit_api_prefixes() {
        assert_eq!(endpoint("https://example.test", "models"), "https://example.test/v1/models");
        assert_eq!(endpoint("https://example.test/v1/", "models"), "https://example.test/v1/models");
        assert_eq!(endpoint("https://example.test/api/v2", "chat/completions"), "https://example.test/api/v2/chat/completions");
        assert_eq!(endpoint("https://example.test/openai", "models"), "https://example.test/openai/models");
    }

    fn model() -> RemoteModel {
        RemoteModel {
            id: "manual-model".into(),
            context_length: Some(32_768),
            max_output_tokens: Some(4_096),
            supports_images: false, tool_support: ToolSupport::Supported,
        }
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
    fn provider_drafts_require_a_key_only_when_creating_and_validate_model_limits() {
        let mut draft = ProviderDraft {
            id: None,
            name: "Fixture".into(),
            api_format: OPENAI_CHAT_COMPLETIONS.into(),
            base_url: "https://api.example.test".into(),
            api_key: None,
            models: vec![model()],
        };
        assert!(validate_provider_draft(&draft, false).is_err());
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
    fn loopback_engines_may_omit_a_key_but_remote_hosts_never_do() {
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
        assert!(validate_provider_draft(&draft, false).is_err(), "non-loopback HTTP hosts still need a key");
        draft.base_url = "https://api.example.test".into();
        assert!(validate_provider_draft(&draft, false).is_err());
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
    }

    #[test]
    fn claude_format_is_explicit_and_never_inferred_from_openai() {
        assert!(is_claude_format(CLAUDE_MESSAGES));
        assert!(!is_claude_format(OPENAI_CHAT_COMPLETIONS));
        assert!(!is_claude_format(FREETOKEN_OPENAI_COMPAT));
        assert!(!is_subscription_format(CLAUDE_MESSAGES));
    }
}
