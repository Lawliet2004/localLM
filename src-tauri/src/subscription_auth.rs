use crate::vault::Vault;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tauri_plugin_opener::OpenerExt;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

pub const PROVIDER_CHATGPT: &str = "chatgpt";
pub const PROVIDER_GROK: &str = "grok";

pub const VAULT_CHATGPT_SUB: &str = "subscription-chatgpt";
pub const VAULT_GROK_SUB: &str = "subscription-grok";

// Standard client IDs used by developer CLI tools for public PKCE
pub const OPENAI_CODEX_CLIENT_ID: &str = "app-codex-cli";
pub const GROK_BUILD_CLIENT_ID: &str = "grok-build-cli";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionSession {
    pub provider: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
    pub account_id: Option<String>,
    pub plan_type: Option<String>,
    pub source: String,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionStatus {
    pub provider: String,
    pub connected: bool,
    pub plan_type: Option<String>,
    pub account_id: Option<String>,
    pub source: Option<String>,
    pub expires_at: Option<i64>,
    pub cli_detected: bool,
    pub cli_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CliDetectionResult {
    pub codex_detected: bool,
    pub codex_path: Option<String>,
    pub grok_detected: bool,
    pub grok_path: Option<String>,
}

fn vault_id_for_provider(provider: &str) -> Result<&'static str, String> {
    match provider {
        PROVIDER_CHATGPT | "chatgpt-subscription" => Ok(VAULT_CHATGPT_SUB),
        PROVIDER_GROK | "grok-subscription" => Ok(VAULT_GROK_SUB),
        _ => Err(format!("Unknown subscription provider: {provider}")),
    }
}

pub fn load_session(vault: &Vault, provider: &str) -> Result<Option<SubscriptionSession>, String> {
    let id = vault_id_for_provider(provider)?;
    match vault.load(id)? {
        Some(bytes) => {
            let session: SubscriptionSession = serde_json::from_slice(&bytes)
                .map_err(|e| format!("Corrupted subscription session: {e}"))?;
            Ok(Some(session))
        }
        None => Ok(None),
    }
}

pub fn save_session(vault: &Vault, session: &SubscriptionSession) -> Result<(), String> {
    let id = vault_id_for_provider(&session.provider)?;
    let bytes = serde_json::to_vec(session)
        .map_err(|e| format!("Failed to serialize subscription session: {e}"))?;
    vault.save(id, &bytes)
}

pub fn clear_session(vault: &Vault, provider: &str) -> Result<(), String> {
    let id = vault_id_for_provider(provider)?;
    vault.clear(id)
}

fn user_home_dir() -> Option<PathBuf> {
    std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()
        .map(PathBuf::from)
}

pub fn codex_auth_path() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("CODEX_HOME") {
        let p = PathBuf::from(explicit).join("auth.json");
        if p.exists() {
            return Some(p);
        }
    }
    if let Some(home) = user_home_dir() {
        let p = home.join(".codex").join("auth.json");
        if p.exists() {
            return Some(p);
        }
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        let p = PathBuf::from(appdata).join("codex").join("auth.json");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

pub fn grok_auth_path() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("GROK_HOME") {
        let p = PathBuf::from(explicit).join("auth.json");
        if p.exists() {
            return Some(p);
        }
    }
    if let Some(home) = user_home_dir() {
        let p = home.join(".grok").join("auth.json");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

pub fn detect_cli_sessions() -> CliDetectionResult {
    let codex = codex_auth_path();
    let grok = grok_auth_path();
    CliDetectionResult {
        codex_detected: codex.is_some(),
        codex_path: codex.map(|p| p.to_string_lossy().into_owned()),
        grok_detected: grok.is_some(),
        grok_path: grok.map(|p| p.to_string_lossy().into_owned()),
    }
}

pub fn parse_codex_auth_content(content: &str) -> Result<SubscriptionSession, String> {
    let json: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| format!("Invalid JSON in codex auth file: {e}"))?;

    let access_token = json
        .get("access_token")
        .or_else(|| json.pointer("/tokens/access_token"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing access_token in Codex auth.json".to_string())?
        .trim()
        .to_string();

    let refresh_token = json
        .get("refresh_token")
        .or_else(|| json.pointer("/tokens/refresh_token"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string());

    let account_id = json
        .get("account_id")
        .or_else(|| json.pointer("/tokens/account_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string());

    let expires_at = json
        .get("expires_at")
        .or_else(|| json.pointer("/tokens/expires_at"))
        .and_then(|v| v.as_i64());

    let plan_type = json
        .get("plan_type")
        .or_else(|| json.get("plan"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| Some("ChatGPT Subscription".into()));

    Ok(SubscriptionSession {
        provider: PROVIDER_CHATGPT.into(),
        access_token,
        refresh_token,
        expires_at,
        account_id,
        plan_type,
        source: "cli_sync".into(),
        updated_at: crate::store::now(),
    })
}

pub fn parse_grok_auth_content(content: &str) -> Result<SubscriptionSession, String> {
    let json: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| format!("Invalid JSON in Grok auth file: {e}"))?;

    let access_token = json
        .get("access_token")
        .or_else(|| json.pointer("/tokens/access_token"))
        .or_else(|| json.pointer("/auth/access_token"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing access_token in Grok auth.json".to_string())?
        .trim()
        .to_string();

    let refresh_token = json
        .get("refresh_token")
        .or_else(|| json.pointer("/tokens/refresh_token"))
        .or_else(|| json.pointer("/auth/refresh_token"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string());

    let expires_at = json
        .get("expires_at")
        .or_else(|| json.pointer("/tokens/expires_at"))
        .and_then(|v| v.as_i64());

    let account_id = json
        .get("account_id")
        .or_else(|| json.get("user_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string());

    let plan_type = json
        .get("tier")
        .or_else(|| json.get("plan"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or_else(|| Some("Grok Subscription".into()));

    Ok(SubscriptionSession {
        provider: PROVIDER_GROK.into(),
        access_token,
        refresh_token,
        expires_at,
        account_id,
        plan_type,
        source: "cli_sync".into(),
        updated_at: crate::store::now(),
    })
}

pub fn import_cli_session(vault: &Vault, provider: &str) -> Result<SubscriptionSession, String> {
    match provider {
        PROVIDER_CHATGPT | "chatgpt-subscription" => {
            let path = codex_auth_path()
                .ok_or_else(|| "Codex CLI auth file not found (~/.codex/auth.json)".to_string())?;
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
            let session = parse_codex_auth_content(&content)?;
            save_session(vault, &session)?;
            Ok(session)
        }
        PROVIDER_GROK | "grok-subscription" => {
            let path = grok_auth_path()
                .ok_or_else(|| "Grok Build CLI auth file not found (~/.grok/auth.json)".to_string())?;
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read {}: {e}", path.display()))?;
            let session = parse_grok_auth_content(&content)?;
            save_session(vault, &session)?;
            Ok(session)
        }
        _ => Err(format!("Unknown subscription provider: {provider}")),
    }
}

pub fn save_manual_token(
    vault: &Vault,
    provider: &str,
    token: &str,
    refresh_token: Option<&str>,
    account_id: Option<&str>,
) -> Result<SubscriptionSession, String> {
    if token.trim().is_empty() {
        return Err("Access token cannot be empty.".into());
    }
    let canonical_provider = match provider {
        PROVIDER_CHATGPT | "chatgpt-subscription" => PROVIDER_CHATGPT,
        PROVIDER_GROK | "grok-subscription" => PROVIDER_GROK,
        _ => return Err(format!("Unknown provider: {provider}")),
    };
    let session = SubscriptionSession {
        provider: canonical_provider.to_string(),
        access_token: token.trim().to_string(),
        refresh_token: refresh_token.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        expires_at: None,
        account_id: account_id.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()),
        plan_type: Some("Subscription (Manual Token)".into()),
        source: "manual".into(),
        updated_at: crate::store::now(),
    };
    save_session(vault, &session)?;
    Ok(session)
}

pub fn get_subscription_status(
    vault: &Vault,
    provider: &str,
) -> Result<SubscriptionStatus, String> {
    let canonical = match provider {
        PROVIDER_CHATGPT | "chatgpt-subscription" => PROVIDER_CHATGPT,
        PROVIDER_GROK | "grok-subscription" => PROVIDER_GROK,
        _ => return Err(format!("Unknown provider: {provider}")),
    };
    let session = load_session(vault, canonical)?;
    let (cli_detected, cli_path) = match canonical {
        PROVIDER_CHATGPT => {
            let p = codex_auth_path();
            (p.is_some(), p.map(|p| p.to_string_lossy().into_owned()))
        }
        PROVIDER_GROK => {
            let p = grok_auth_path();
            (p.is_some(), p.map(|p| p.to_string_lossy().into_owned()))
        }
        _ => (false, None),
    };
    match session {
        Some(s) => Ok(SubscriptionStatus {
            provider: canonical.to_string(),
            connected: true,
            plan_type: s.plan_type,
            account_id: s.account_id,
            source: Some(s.source),
            expires_at: s.expires_at,
            cli_detected,
            cli_path,
        }),
        None => Ok(SubscriptionStatus {
            provider: canonical.to_string(),
            connected: false,
            plan_type: None,
            account_id: None,
            source: None,
            expires_at: None,
            cli_detected,
            cli_path,
        }),
    }
}

pub async fn refresh_subscription_token(
    vault: &Vault,
    provider: &str,
    mut session: SubscriptionSession,
) -> Result<SubscriptionSession, String> {
    let refresh_token = match &session.refresh_token {
        Some(t) if !t.is_empty() => t.clone(),
        _ => return Ok(session),
    };

    let client = crate::providers::client()?;
    let now_sec = crate::store::now() / 1000;

    match provider {
        PROVIDER_CHATGPT | "chatgpt-subscription" => {
            let body = format!(
                "grant_type=refresh_token&client_id={}&refresh_token={}",
                OPENAI_CODEX_CLIENT_ID,
                urlencoding::encode(&refresh_token)
            );
            let response = client
                .post("https://auth.openai.com/oauth/token")
                .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(body)
                .send()
                .await
                .map_err(|e| format!("Failed to reach OpenAI refresh endpoint: {e}"))?;

            if response.status().is_success() {
                let json: serde_json::Value = response
                    .json()
                    .await
                    .map_err(|e| format!("Invalid refresh response: {e}"))?;

                if let Some(new_access) = json.get("access_token").and_then(|v| v.as_str()) {
                    session.access_token = new_access.to_string();
                }
                if let Some(new_refresh) = json.get("refresh_token").and_then(|v| v.as_str()) {
                    session.refresh_token = Some(new_refresh.to_string());
                }
                if let Some(expires_in) = json.get("expires_in").and_then(|v| v.as_i64()) {
                    session.expires_at = Some(now_sec + expires_in);
                }
                session.updated_at = crate::store::now();
                let _ = save_session(vault, &session);
            }
        }
        PROVIDER_GROK | "grok-subscription" => {
            let body = format!(
                "grant_type=refresh_token&client_id={}&refresh_token={}",
                GROK_BUILD_CLIENT_ID,
                urlencoding::encode(&refresh_token)
            );
            let response = client
                .post("https://auth.x.ai/oauth/token")
                .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(body)
                .send()
                .await
                .map_err(|e| format!("Failed to reach Grok refresh endpoint: {e}"))?;

            if response.status().is_success() {
                let json: serde_json::Value = response
                    .json()
                    .await
                    .map_err(|e| format!("Invalid refresh response: {e}"))?;

                if let Some(new_access) = json.get("access_token").and_then(|v| v.as_str()) {
                    session.access_token = new_access.to_string();
                }
                if let Some(new_refresh) = json.get("refresh_token").and_then(|v| v.as_str()) {
                    session.refresh_token = Some(new_refresh.to_string());
                }
                if let Some(expires_in) = json.get("expires_in").and_then(|v| v.as_i64()) {
                    session.expires_at = Some(now_sec + expires_in);
                }
                session.updated_at = crate::store::now();
                let _ = save_session(vault, &session);
            }
        }
        _ => {}
    }

    Ok(session)
}

pub async fn get_valid_access_token(vault: &Vault, provider: &str) -> Result<String, String> {
    let session = load_session(vault, provider)?
        .ok_or_else(|| format!("No active subscription session found for {provider}. Sign in or sync from CLI first."))?;

    let now_sec = crate::store::now() / 1000;
    let needs_refresh = session
        .expires_at
        .map(|exp| now_sec >= exp - 180)
        .unwrap_or(false);

    if needs_refresh && session.refresh_token.is_some() {
        let refreshed = refresh_subscription_token(vault, provider, session).await?;
        Ok(refreshed.access_token)
    } else {
        Ok(session.access_token)
    }
}

pub fn base64url_encode(data: &[u8]) -> String {
    const CHARSET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut result = String::new();
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i];
        let b1 = if i + 1 < data.len() { data[i + 1] } else { 0 };
        let b2 = if i + 2 < data.len() { data[i + 2] } else { 0 };

        result.push(CHARSET[(b0 >> 2) as usize] as char);
        result.push(CHARSET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);

        if i + 1 < data.len() {
            result.push(CHARSET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        }
        if i + 2 < data.len() {
            result.push(CHARSET[(b2 & 0x3f) as usize] as char);
        }
        i += 3;
    }
    result
}

pub fn generate_pkce() -> (String, String) {
    use aes_gcm::aead::OsRng;
    use aes_gcm::aead::rand_core::RngCore;
    let mut random_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut random_bytes);
    let verifier = base64url_encode(&random_bytes);

    let mut hasher = Sha256::new();
    hasher.update(verifier.as_bytes());
    let hash = hasher.finalize();
    let challenge = base64url_encode(&hash);

    (verifier, challenge)
}

pub async fn sign_in_subscription(
    app: &tauri::AppHandle,
    provider: &str,
    vault: Arc<Vault>,
    mut cancel: tokio::sync::watch::Receiver<bool>,
) -> Result<SubscriptionSession, String> {
    let canonical = match provider {
        PROVIDER_CHATGPT | "chatgpt-subscription" => PROVIDER_CHATGPT,
        PROVIDER_GROK | "grok-subscription" => PROVIDER_GROK,
        _ => return Err(format!("Unsupported subscription provider: {provider}")),
    };

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("Could not open local OAuth callback listener: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| e.to_string())?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    let (verifier, challenge) = generate_pkce();
    let state_param = format!("locallm-{}", uuid::Uuid::new_v4());

    let (auth_url, token_url, client_id) = match canonical {
        PROVIDER_CHATGPT => (
            format!(
                "https://auth.openai.com/authorize?response_type=code&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&state={}&scope=openid%20profile%20email%20model.request",
                OPENAI_CODEX_CLIENT_ID,
                urlencoding::encode(&redirect_uri),
                challenge,
                state_param
            ),
            "https://auth.openai.com/oauth/token",
            OPENAI_CODEX_CLIENT_ID,
        ),
        PROVIDER_GROK => (
            format!(
                "https://auth.x.ai/authorize?response_type=code&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&state={}&scope=openid%20profile%20grok.chat",
                GROK_BUILD_CLIENT_ID,
                urlencoding::encode(&redirect_uri),
                challenge,
                state_param
            ),
            "https://auth.x.ai/oauth/token",
            GROK_BUILD_CLIENT_ID,
        ),
        _ => unreachable!(),
    };

    app.opener()
        .open_url(&auth_url, None::<&str>)
        .map_err(|e| format!("Could not open system browser for subscription sign-in: {e}"))?;

    let wait_for_code = async {
        loop {
            let (mut socket, _) = listener
                .accept()
                .await
                .map_err(|_| "Local callback listener failed.".to_string())?;

            let mut bytes = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                let count = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut chunk))
                    .await
                    .map_err(|_| "Sign-in callback timed out.".to_string())?
                    .map_err(|_| "Could not read sign-in callback.".to_string())?;
                if count == 0 {
                    break;
                }
                bytes.extend_from_slice(&chunk[..count]);
                if bytes.len() > 16384 {
                    return Err("Sign-in callback exceeded the size limit.".to_string());
                }
                if bytes.windows(4).any(|part| part == b"\r\n\r\n") {
                    break;
                }
            }

            let request = std::str::from_utf8(&bytes)
                .map_err(|_| "Invalid callback encoding.".to_string())?;

            let target = match parse_callback_request(request) {
                Ok(target) => target,
                Err(_) => {
                    let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    continue;
                }
            };

            let code = extract_query_param(&target, "code");
            let returned_state = extract_query_param(&target, "state");

            if returned_state.as_deref() != Some(&state_param) {
                let _ = socket.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\n\r\nInvalid state parameter.").await;
                return Err("Security verification failed: OAuth state mismatch.".to_string());
            }

            let code = match code {
                Some(c) => c,
                None => {
                    let err = extract_query_param(&target, "error").unwrap_or_else(|| "Unknown auth error".into());
                    let _ = socket.write_all(format!("HTTP/1.1 400 Bad Request\r\nContent-Type: text/plain\r\n\r\nSign-in rejected: {err}").as_bytes()).await;
                    return Err(format!("Provider sign-in was not approved: {err}"));
                }
            };

            let html_body = "<html><body style=\"font-family:sans-serif;text-align:center;padding:50px;\"><h2>Sign-in Successful!</h2><p>LocalLM is now linked to your subscription. You may close this tab.</p></body></html>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                html_body.len(),
                html_body
            );
            let _ = socket.write_all(response.as_bytes()).await;

            return Ok(code);
        }
    };

    let auth_code = tokio::select! {
        _ = cancel.changed() => return Err("Sign-in cancelled.".into()),
        res = tokio::time::timeout(Duration::from_secs(300), wait_for_code) => {
            res.map_err(|_| "Sign-in timed out after 5 minutes.".to_string())??
        }
    };

    let client = crate::providers::client()?;
    let exchange_body = format!(
        "grant_type=authorization_code&client_id={}&code={}&redirect_uri={}&code_verifier={}",
        urlencoding::encode(client_id),
        urlencoding::encode(&auth_code),
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(&verifier)
    );
    let token_resp = client
        .post(token_url)
        .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(exchange_body)
        .send()
        .await
        .map_err(|e| format!("Token exchange request failed: {e}"))?;

    let status = token_resp.status();
    let body = token_resp
        .text()
        .await
        .unwrap_or_default();

    if !status.is_success() {
        return Err(format!("Token exchange failed ({status}): {body}"));
    }

    let json: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("Invalid token response: {e}"))?;

    let access_token = json
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Token response missing access_token".to_string())?
        .to_string();

    let refresh_token = json
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let now_sec = crate::store::now() / 1000;
    let expires_at = json
        .get("expires_in")
        .and_then(|v| v.as_i64())
        .map(|exp| now_sec + exp);

    let session = SubscriptionSession {
        provider: canonical.to_string(),
        access_token,
        refresh_token,
        expires_at,
        account_id: None,
        plan_type: Some(format!("{} Subscription", if canonical == PROVIDER_CHATGPT { "ChatGPT" } else { "Grok" })),
        source: "oauth".into(),
        updated_at: crate::store::now(),
    };

    save_session(&vault, &session)?;
    Ok(session)
}

fn parse_callback_request(request: &str) -> Result<String, String> {
    let mut parts = request.lines().next().unwrap_or_default().split_whitespace();
    if parts.next() != Some("GET") {
        return Err("Expected GET callback.".into());
    }
    let target = parts.next().ok_or("Missing callback target.")?;
    if !target.starts_with("/callback?") {
        return Err("Unexpected callback path.".into());
    }
    Ok(target.to_string())
}

fn extract_query_param(target: &str, param: &str) -> Option<String> {
    let query = target.split_once('?')?;
    for pair in query.1.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == param {
                return Some(urlencoding::decode(v).unwrap_or_default().into_owned());
            }
        }
    }
    None
}

mod urlencoding {
    pub fn encode(input: &str) -> String {
        let mut out = String::new();
        for byte in input.bytes() {
            if byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_' || byte == b'.' || byte == b'~' {
                out.push(byte as char);
            } else {
                out.push_str(&format!("%{:02X}", byte));
            }
        }
        out
    }

    pub fn decode(input: &str) -> Result<std::borrow::Cow<'_, str>, ()> {
        let bytes = input.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' && i + 2 < bytes.len() {
                if let Ok(val) = u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).map_err(|_| ())?, 16) {
                    out.push(val);
                    i += 3;
                    continue;
                }
            }
            if bytes[i] == b'+' {
                out.push(b' ');
            } else {
                out.push(bytes[i]);
            }
            i += 1;
        }
        String::from_utf8(out).map(std::borrow::Cow::Owned).map_err(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_codex_auth_handles_standard_format() {
        let json = r#"{
            "auth_mode": "chatgpt",
            "access_token": "test-access-token-123",
            "refresh_token": "test-refresh-token-456",
            "account_id": "acct_abc123",
            "expires_at": 1800000000
        }"#;

        let session = parse_codex_auth_content(json).unwrap();
        assert_eq!(session.provider, PROVIDER_CHATGPT);
        assert_eq!(session.access_token, "test-access-token-123");
        assert_eq!(session.refresh_token.as_deref(), Some("test-refresh-token-456"));
        assert_eq!(session.account_id.as_deref(), Some("acct_abc123"));
        assert_eq!(session.expires_at, Some(1800000000));
        assert_eq!(session.source, "cli_sync");
    }

    #[test]
    fn parse_grok_auth_handles_standard_format() {
        let json = r#"{
            "access_token": "grok-access-789",
            "refresh_token": "grok-refresh-012",
            "user_id": "user_xyz",
            "tier": "SuperGrok"
        }"#;

        let session = parse_grok_auth_content(json).unwrap();
        assert_eq!(session.provider, PROVIDER_GROK);
        assert_eq!(session.access_token, "grok-access-789");
        assert_eq!(session.refresh_token.as_deref(), Some("grok-refresh-012"));
        assert_eq!(session.account_id.as_deref(), Some("user_xyz"));
        assert_eq!(session.plan_type.as_deref(), Some("SuperGrok"));
    }

    #[test]
    fn pkce_generation_produces_non_empty_base64url_strings() {
        let (verifier, challenge) = generate_pkce();
        assert!(!verifier.is_empty());
        assert!(!challenge.is_empty());
        assert_ne!(verifier, challenge);
        assert!(verifier.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_'));
        assert!(challenge.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn query_param_extraction_works() {
        let target = "/callback?code=abc-123&state=state-456";
        assert_eq!(extract_query_param(target, "code"), Some("abc-123".into()));
        assert_eq!(extract_query_param(target, "state"), Some("state-456".into()));
        assert_eq!(extract_query_param(target, "unknown"), None);
    }
}
