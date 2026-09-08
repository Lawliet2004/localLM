use crate::vault::Vault;
use rmcp::transport::auth::{
    AuthError, AuthorizationManager, AuthorizationRequest, AuthorizationSession,
    CredentialRefreshGuard, CredentialStore, StoredCredentials,
};
use std::{sync::Arc, time::Duration};
use tauri_plugin_opener::OpenerExt;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

struct ProtectedStore {
    vault: Arc<Vault>,
    id: String,
}
#[async_trait::async_trait]
impl CredentialStore for ProtectedStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        self.vault
            .load(&self.id)
            .map_err(AuthError::CredentialStoreError)?
            .map(|bytes| {
                serde_json::from_slice(&bytes).map_err(|_| {
                    AuthError::CredentialStoreError("Saved authorization data is invalid.".into())
                })
            })
            .transpose()
    }
    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let bytes = serde_json::to_vec(&credentials).map_err(|_| {
            AuthError::CredentialStoreError("Could not encode authorization data.".into())
        })?;
        self.vault
            .save(&self.id, &bytes)
            .map_err(AuthError::CredentialStoreError)
    }
    async fn clear(&self) -> Result<(), AuthError> {
        self.vault
            .clear(&self.id)
            .map_err(AuthError::CredentialStoreError)
    }
    async fn acquire_refresh_guard(&self) -> Result<Option<CredentialRefreshGuard>, AuthError> {
        Ok(Some(CredentialRefreshGuard::new(
            self.vault.refresh.clone().lock_owned().await,
        )))
    }
}

pub async fn manager(
    id: &str,
    url: &str,
    vault: Arc<Vault>,
) -> Result<AuthorizationManager, String> {
    let mut manager = AuthorizationManager::new(url)
        .await
        .map_err(|_| "Could not initialize account authorization.")?;
    manager.set_credential_store(ProtectedStore {
        vault,
        id: format!("oauth-{id}"),
    });
    Ok(manager)
}

pub async fn sign_in(
    app: &tauri::AppHandle,
    id: &str,
    url: &str,
    vault: Arc<Vault>,
    mut cancel: tokio::sync::watch::Receiver<bool>,
) -> Result<(), String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| "Could not open the local sign-in callback listener.")?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    let redirect = format!("http://127.0.0.1:{port}/callback");
    let mut manager = manager(id, url, vault).await?;
    let resolution = tokio::time::timeout(Duration::from_secs(30), manager.resolve_metadata())
        .await
        .map_err(|_| "Authorization discovery timed out.")?
        .map_err(|_| "The provider's authorization settings could not be discovered.")?;
    manager.set_metadata(resolution.metadata);
    let request = AuthorizationRequest::new(&redirect)
        .with_client_name("LocalLM")
        .with_application_type("native");
    let session = tokio::time::timeout(Duration::from_secs(30),AuthorizationSession::new(manager,request)).await.map_err(|_| "Account registration timed out.")?
        .map_err(|_| "The provider did not accept automatic app registration. It may require a pre-registered OAuth client.")?;
    let authorization_url = reqwest::Url::parse(session.get_authorization_url())
        .map_err(|_| "Provider returned an invalid sign-in URL.")?;
    if authorization_url.scheme() != "https" {
        return Err("Provider sign-in must use HTTPS.".into());
    }
    app.opener()
        .open_url(authorization_url.as_str(), None::<&str>)
        .map_err(|_| "Could not open the browser for sign-in.")?;
    let wait = async {
        loop {
            let (mut socket, _) = listener
                .accept()
                .await
                .map_err(|_| "Local callback listener failed.")?;
            let mut bytes = Vec::new();
            let mut chunk = [0u8; 1024];
            loop {
                let count = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut chunk))
                    .await
                    .map_err(|_| "Sign-in callback timed out.")?
                    .map_err(|_| "Could not read sign-in callback.")?;
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
            let request =
                std::str::from_utf8(&bytes).map_err(|_| "Invalid sign-in callback encoding.")?;
            let target = match callback_target(request) {
                Ok(target) => target,
                Err(_) => {
                    let _ = socket.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                    continue;
                }
            };
            let result = session
                .handle_callback_url(&format!("http://127.0.0.1:{port}{target}"))
                .await;
            let (status, body) = if result.is_ok() {
                (
                    "200 OK",
                    "Sign-in complete. Return to LocalLM. You can close this tab.",
                )
            } else {
                (
                    "400 Bad Request",
                    "Sign-in could not be verified. Return to LocalLM and try again.",
                )
            };
            let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nCache-Control: no-store\r\nContent-Security-Policy: default-src 'none'\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
            let _ = socket.write_all(response.as_bytes()).await;
            return result.map(|_| ()).map_err(|_| {
                "The authorization callback was rejected. Sign in again.".to_string()
            });
        }
    };
    tokio::select! {
        _ = cancel.changed() => Err("Sign-in cancelled.".into()),
        result = tokio::time::timeout(Duration::from_secs(300),wait) => result.map_err(|_| "Sign-in expired after five minutes. Try again.".to_string())?,
    }
}
fn callback_target(request: &str) -> Result<&str, String> {
    let mut parts = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    if parts.next() != Some("GET") {
        return Err("Expected GET callback.".into());
    }
    let target = parts.next().ok_or("Missing callback target.")?;
    if !target.starts_with("/callback?") || target.contains('#') {
        return Err("Unexpected callback path.".into());
    }
    Ok(target)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_accepts_only_the_expected_origin_relative_path() {
        assert!(callback_target("GET /callback?code=abc&state=def HTTP/1.1\r\n\r\n").is_ok());
        for request in [
            "POST /callback?code=x HTTP/1.1",
            "GET https://other/callback?code=x HTTP/1.1",
            "GET /favicon.ico HTTP/1.1",
        ] {
            assert!(callback_target(request).is_err());
        }
    }
}
