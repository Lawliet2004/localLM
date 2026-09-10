use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const OPENAI_CHAT_COMPLETIONS: &str = "openai-chat-completions";
pub const ANTHROPIC_MESSAGES: &str = "anthropic-messages";

/// Supported provider wire formats. Anything else is rejected loudly;
/// LocalLM never silently downgrades to a guessed protocol.
pub fn api_formats() -> [&'static str; 2] {
    [OPENAI_CHAT_COMPLETIONS, ANTHROPIC_MESSAGES]
}

pub fn default_base_url(api_format: &str) -> Option<&'static str> {
    match api_format {
        OPENAI_CHAT_COMPLETIONS => None, // Too many OpenAI-compatible hosts to guess.
        ANTHROPIC_MESSAGES => Some("https://api.anthropic.com"),
        _ => None,
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ToolSupport {
    Unknown,
    Supported,
    Unsupported,
}

impl Default for ToolSupport {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteModel {
    pub id: String,
    pub context_length: Option<u32>,
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub tool_support: ToolSupport,
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
pub struct ModelSelection {
    pub provider_id: Option<String>,
    pub model_id: String,
}

impl Default for ModelSelection {
    fn default() -> Self {
        Self {
            provider_id: None,
            model_id: String::new(),
        }
    }
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
    if !api_formats().contains(&draft.api_format.as_str()) {
        return Err("Unknown provider format. Choose openai-chat-completions or anthropic-messages.".into());
    }
    validate_base_url(&draft.base_url)?;
    if draft.api_key.is_none() && !existing {
        return Err("An API key is required when adding a provider.".into());
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
    if reqwest::Url::parse(base).is_ok_and(|url| url.path().trim_matches('/').len() > 0) {
        format!("{base}/{route}")
    } else {
        format!("{base}/v1/{route}")
    }
}

async fn bounded_body(response: reqwest::Response, limit: usize) -> Result<String, String> {
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
        401 | 403 => "Provider rejected the saved API key.".into(),
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
    let request = client()?.get(endpoint(&base_url, "models"));
    // Anthropic's Models API uses its own auth headers; the response shape
    // ({data: [{id}]}) matches, so listing stays shared.
    let request = if provider.api_format == ANTHROPIC_MESSAGES {
        request.header("x-api-key", key).header("anthropic-version", "2023-06-01")
    } else {
        request.bearer_auth(key)
    };
    let response = request.send().await
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
        "Connection could not be verified: the model-list endpoint returned 404 or 405. Check the API base URL and its path. No inference request was sent.".into()
    };
    Ok(ProviderTestResult {
        verified: result.supported,
        model_list_supported: result.supported,
        models: result.models,
        message,
    })
}

#[derive(Clone, Copy)]
enum Auth {
    Bearer,
    Anthropic,
}

pub async fn test_inference(
    provider: &ProviderConnection,
    key: &str,
    model_id: &str,
) -> Result<ProviderTestResult, String> {
    validate_model_id(model_id)?;
    let base_url = validate_base_url(&provider.base_url)?;
    let (route, payload, auth): (&str, serde_json::Value, Auth) = if provider.api_format == ANTHROPIC_MESSAGES {
        ("messages", serde_json::json!({
            "model": model_id,
            "messages": [{"role": "user", "content": "ping"}],
            "max_tokens": 1,
        }), Auth::Anthropic)
    } else {
        ("chat/completions", serde_json::json!({
            "model": model_id,
            "messages": [{"role": "user", "content": "ping"}],
            "max_tokens": 1,
            "stream": false
        }), Auth::Bearer)
    };
    let request = client()?.post(endpoint(&base_url, route)).json(&payload);
    let request = match auth {
        Auth::Bearer => request.bearer_auth(key),
        Auth::Anthropic => request.header("x-api-key", key).header("anthropic-version", "2023-06-01"),
    };
    let response = request.send().await
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

/// Formats + default base URLs for the provider form (no secrets).
#[tauri::command]
pub fn provider_formats() -> Vec<serde_json::Value> {
    api_formats()
        .iter()
        .map(|format| {
            serde_json::json!({"id": format, "defaultBaseUrl": default_base_url(format)})
        })
        .collect()
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
                    tool_support: ToolSupport::Unknown,
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

    #[tokio::test]
    async fn connection_checks_do_not_verify_missing_routes_and_accept_large_catalogs() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (status, body, verified) in [
            ("404 Not Found", "{}".to_string(), false),
            ("405 Method Not Allowed", "{}".to_string(), false),
            ("200 OK", serde_json::json!({"data": [{"id": "one", "description": "x".repeat(20_000)}]}).to_string(), true),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let count = stream.read(&mut request).await.unwrap();
                assert!(String::from_utf8_lossy(&request[..count]).starts_with("GET /custom/models "));
                stream.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            });
            let provider = ProviderConnection {
                id: "fixture".into(), name: "Fixture".into(), api_format: OPENAI_CHAT_COMPLETIONS.into(),
                base_url: format!("http://{address}/custom"), verified: false, last_tested_at: None,
                models: vec![], has_api_key: true,
            };
            let result = test_connection(&provider, "fixture-key").await.unwrap();
            server.await.unwrap();
            assert_eq!(result.verified, verified);
            assert_eq!(result.model_list_supported, verified);
        }
    }

    fn model() -> RemoteModel {
        RemoteModel {
            id: "manual-model".into(),
            context_length: Some(32_768),
            max_output_tokens: Some(4_096),
            tool_support: ToolSupport::Supported,
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

    #[tokio::test]
    async fn model_listing_uses_the_vault_key_without_following_redirects_or_leaking_it() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            loop {
                let count = stream.read(&mut buffer).await.unwrap();
                request.extend_from_slice(&buffer[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
            assert!(request.contains("authorization: bearer fixture-secret"));
            let body = r#"{"data":[{"id":"one"},{"id":"one"},{"id":"two"}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        let provider = ProviderConnection {
            id: "provider".into(),
            name: "Fixture".into(),
            api_format: OPENAI_CHAT_COMPLETIONS.into(),
            base_url: format!("http://{address}"),
            verified: false,
            last_tested_at: None,
            models: Vec::new(),
            has_api_key: true,
        };
        let result = list_models(&provider, "fixture-secret").await.unwrap();
        server.await.unwrap();
        assert!(result.supported);
        assert_eq!(result.models, ["one", "two"]);

        let redirect_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let redirect_address = redirect_listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = redirect_listener.accept().await.unwrap();
            let mut request = [0; 512];
            let _ = stream.read(&mut request).await;
            stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/v1/models\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await.unwrap();
        });
        let redirect_provider = ProviderConnection {
            base_url: format!("http://{redirect_address}"),
            ..provider
        };
        let error = list_models(&redirect_provider, "fixture-secret")
            .await
            .unwrap_err();
        assert!(error.contains("302"));
        assert!(!error.contains("fixture-secret"));
    }
}
