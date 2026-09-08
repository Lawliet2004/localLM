use rmcp::{
    service::RunningService,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    RoleClient, ServiceExt,
};
use serde::Serialize;
use serde_json::Value;
use std::{collections::HashMap, sync::Arc, time::Duration};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolView {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorView {
    pub id: String,
    pub description: String,
    pub url: String,
    pub auth_type: String,
    pub connected: bool,
    pub has_credential: bool,
    pub tools: Vec<ToolView>,
}
struct Connection {
    service: RunningService<RoleClient, ()>,
    tools: Vec<ToolView>,
}
pub struct McpHub {
    connections: HashMap<String, Connection>,
    vault: Arc<crate::vault::Vault>,
}

pub struct AgentTool {
    pub connector: String,
    pub tool: ToolView,
    pub alias: String,
    backend: ToolBackend,
}
enum ToolBackend {
    Mcp(rmcp::Peer<RoleClient>),
    Workspace(Arc<crate::workspace::Workspace>),
}
impl AgentTool {
    pub fn workspace(workspace: Arc<crate::workspace::Workspace>, tool: ToolView) -> Self {
        Self {
            connector: "Workspace".into(),
            alias: format!("workspace_{}", tool.name),
            tool,
            backend: ToolBackend::Workspace(workspace),
        }
    }
    pub fn definition(&self) -> Value {
        serde_json::json!({"type":"function","function":{"name":self.alias,"description":format!("{}: {} — {}", self.connector,self.tool.name,self.tool.description),"parameters":self.tool.input_schema}})
    }
    pub async fn call(&self, arguments: Value) -> Result<Value, String> {
        let peer = match &self.backend {
            ToolBackend::Mcp(peer) => peer,
            ToolBackend::Workspace(workspace) => {
                let workspace = workspace.clone();
                let name = self.tool.name.clone();
                return tokio::task::spawn_blocking(move || workspace.call(&name, arguments))
                    .await
                    .map_err(|_| "Workspace operation failed unexpectedly.")?;
            }
        };
        let arguments = arguments
            .as_object()
            .ok_or("Tool arguments must be an object.")?
            .clone();
        let response = peer.call_tool_once(rmcp::model::CallToolRequestParams::new(self.tool.name.clone()).with_arguments(arguments)).await.map_err(|_| "Connector tool request failed. Its remote outcome may be unknown; do not automatically retry.".to_string())?;
        match response {
            rmcp::model::CallToolResponse::Complete(result) => {
                let value =
                    serde_json::to_value(result).map_err(|_| "Could not decode tool result.")?;
                if value.to_string().len() > 262_144 {
                    return Err("Tool result exceeds 256 KiB. Request a narrower result.".into());
                }
                Ok(value)
            }
            _ => Err(
                "This tool requires additional server input, which is not supported yet.".into(),
            ),
        }
    }
}

fn presets() -> Vec<ConnectorView> {
    let catalog: Value = serde_json::from_str(include_str!("../../src/lib/catalog.json"))
        .expect("Bundled connector catalog is valid JSON");
    catalog["connectors"]
        .as_array()
        .expect("Bundled connectors array")
        .iter()
        .map(|item| ConnectorView {
            id: item["name"].as_str().unwrap_or_default().into(),
            description: item["description"].as_str().unwrap_or_default().into(),
            url: item["url"].as_str().unwrap_or_default().into(),
            auth_type: match item["auth"]["type"].as_str() {
                Some("dcr") => "oauth",
                Some("header") => "apiKey",
                _ => "none",
            }
            .into(),
            connected: false,
            has_credential: false,
            tools: Vec::new(),
        })
        .collect()
}
fn preset(id: &str) -> Result<ConnectorView, String> {
    presets()
        .into_iter()
        .find(|item| item.id == id)
        .ok_or_else(|| "Unknown connector.".into())
}
fn validate_token(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 8192
        || value.chars().any(char::is_control)
        || value.trim() != value
    {
        return Err(
            "Enter a token without surrounding spaces or control characters (maximum 8192 bytes)."
                .into(),
        );
    }
    Ok(())
}

impl McpHub {
    pub fn selected_tools(&self, ids: &[String]) -> Result<Vec<AgentTool>, String> {
        let mut tools = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for id in ids {
            if !seen.insert(id) {
                continue;
            }
            let connection = self
                .connections
                .get(id)
                .filter(|value| !value.service.is_closed())
                .ok_or_else(|| format!("Connector {id} is not connected."))?;
            for tool in &connection.tools {
                if tools.len() >= 32 {
                    return Err(
                        "Select fewer connectors: at most 32 tools can be offered in a turn."
                            .into(),
                    );
                }
                tools.push(AgentTool {
                    connector: id.clone(),
                    tool: tool.clone(),
                    alias: format!(
                        "t{}_{}",
                        tools.len(),
                        tool.name
                            .chars()
                            .map(|character| {
                                if character.is_ascii_alphanumeric() || character == '_' {
                                    character
                                } else {
                                    '_'
                                }
                            })
                            .take(48)
                            .collect::<String>()
                    ),
                    backend: ToolBackend::Mcp(connection.service.peer().clone()),
                });
            }
        }
        Ok(tools)
    }
    pub fn new(vault: Arc<crate::vault::Vault>) -> Self {
        Self {
            connections: HashMap::new(),
            vault,
        }
    }
    fn token(&self, id: &str) -> Result<Option<String>, String> {
        self.vault
            .load(&format!("token-{id}"))?
            .map(|bytes| {
                String::from_utf8(bytes).map_err(|_| "Invalid stored token encoding.".into())
            })
            .transpose()
    }
    fn save_token(&self, id: &str, value: &str) -> Result<(), String> {
        preset(id)?;
        validate_token(value)?;
        self.vault.save(&format!("token-{id}"), value.as_bytes())
    }
    pub fn list(&self) -> Result<Vec<ConnectorView>, String> {
        let mut items = presets();
        for item in &mut items {
            item.has_credential = if item.auth_type == "oauth" {
                self.vault.load(&format!("oauth-{}", item.id))?.is_some()
            } else {
                self.token(&item.id)?.is_some()
            };
            if let Some(connection) = self.connections.get(&item.id) {
                item.connected = !connection.service.is_closed();
                if item.connected {
                    item.tools = connection.tools.clone();
                }
            }
        }
        Ok(items)
    }
    pub async fn connect(&mut self, id: &str) -> Result<ConnectorView, String> {
        let mut item = preset(id)?;
        let secret = self.token(id)?;
        if item.auth_type == "apiKey" && secret.is_none() {
            return Err("Add an API token before connecting.".into());
        }
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(60))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| "Could not initialize connector networking.")?;
        let mut config = StreamableHttpClientTransportConfig::with_uri(item.url.clone())
            .max_concurrent_requests(2);
        if let Some(secret) = &secret {
            config = config.auth_header(secret);
        }
        let redact = |error: String| {
            secret
                .as_ref()
                .map(|secret| error.replace(secret, "[redacted]"))
                .unwrap_or(error)
        };
        let service = if item.auth_type == "oauth" {
            let mut manager = crate::oauth::manager(id, &item.url, self.vault.clone()).await?;
            if !manager
                .initialize_from_store()
                .await
                .map_err(|_| "Saved authorization could not be restored. Sign in again.")?
            {
                return Err("Sign in to this connector first.".into());
            }
            let client = rmcp::transport::auth::AuthClient::new(client, manager);
            let transport = StreamableHttpClientTransport::with_client(client, config);
            tokio::time::timeout(Duration::from_secs(30), ().serve(transport))
                .await
                .map_err(|_| "Connector handshake timed out.")?
                .map_err(|_| {
                    "Authorized connection failed. Check account access or sign in again."
                        .to_string()
                })?
        } else {
            let transport = StreamableHttpClientTransport::with_client(client, config);
            tokio::time::timeout(Duration::from_secs(30), ().serve(transport))
                .await
                .map_err(|_| "Connector handshake timed out.")?
                .map_err(|error| redact(format!("Could not connect: {error}")))?
        };
        let result = tokio::time::timeout(Duration::from_secs(30), service.list_all_tools()).await;
        let tools = match result {
            Ok(Ok(tools)) => tools,
            result => {
                let _ = service.cancel().await;
                return Err(match result {
                    Err(_) => "Tool discovery timed out.".into(),
                    Ok(Err(error)) => redact(format!("Tool discovery failed: {error}")),
                    _ => unreachable!(),
                });
            }
        };
        if tools.len() > 512 {
            let _ = service.cancel().await;
            return Err("Connector returned more than 512 tools.".into());
        }
        let views: Vec<_> = tools
            .into_iter()
            .map(|tool| ToolView {
                name: tool.name.into_owned(),
                description: tool
                    .description
                    .map(|text| text.into_owned())
                    .unwrap_or_default(),
                input_schema: Value::Object((*tool.input_schema).clone()),
            })
            .collect();
        if serde_json::to_vec(&views)
            .map_err(|error| error.to_string())?
            .len()
            > 2_097_152
        {
            let _ = service.cancel().await;
            return Err("Connector tool catalog exceeds 2 MiB.".into());
        }
        if let Some(old) = self.connections.remove(id) {
            let _ = old.service.cancel().await;
        }
        self.connections.insert(
            id.into(),
            Connection {
                service,
                tools: views.clone(),
            },
        );
        item.connected = true;
        item.has_credential = secret.is_some() || item.auth_type == "oauth";
        item.tools = views;
        Ok(item)
    }
    pub async fn disconnect(&mut self, id: &str, forget: bool) -> Result<(), String> {
        preset(id)?;
        if let Some(connection) = self.connections.remove(id) {
            connection
                .service
                .cancel()
                .await
                .map_err(|_| "Could not close the connector session cleanly.")?;
        }
        if forget {
            self.vault.clear(&format!("token-{id}"))?;
            self.vault.clear(&format!("oauth-{id}"))?;
        }
        Ok(())
    }
}

#[tauri::command]
pub async fn list_connectors(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<ConnectorView>, String> {
    state.connectors.lock().await.list()
}
#[tauri::command]
pub async fn connect_connector(
    state: tauri::State<'_, crate::AppState>,
    id: String,
    api_token: Option<String>,
) -> Result<ConnectorView, String> {
    let mut hub = state.connectors.lock().await;
    if let Some(secret) = api_token {
        hub.save_token(&id, &secret)?;
    }
    hub.connect(&id).await
}
#[tauri::command]
pub async fn disconnect_connector(
    state: tauri::State<'_, crate::AppState>,
    id: String,
    forget: bool,
) -> Result<(), String> {
    state.connectors.lock().await.disconnect(&id, forget).await
}

#[tauri::command]
pub async fn sign_in_connector(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    id: String,
) -> Result<ConnectorView, String> {
    let _operation = state
        .oauth_operation
        .try_lock()
        .map_err(|_| "Another account sign-in is in progress.")?;
    let item = preset(&id)?;
    if item.auth_type != "oauth" {
        return Err("This connector does not use account sign-in.".into());
    }
    let vault = state.connectors.lock().await.vault.clone();
    state.oauth_cancel.send_replace(false);
    crate::oauth::sign_in(&app, &id, &item.url, vault, state.oauth_cancel.subscribe()).await?;
    state.connectors.lock().await.connect(&id).await
}
#[tauri::command]
pub fn cancel_connector_sign_in(state: tauri::State<'_, crate::AppState>) {
    state.oauth_cancel.send_replace(true);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_arbitrary_connector_ids_and_header_injection() {
        assert!(preset("https://attacker.example").is_err());
        for value in ["", " token", "token\r\nAuthorization: another"] {
            assert!(validate_token(value).is_err());
        }
        assert!(validate_token("github_pat_sample").is_ok());
    }
    #[test]
    fn bundled_presets_have_unique_ids_and_secure_endpoints() {
        let items = presets();
        assert_eq!(items.len(), 14);
        let mut ids = std::collections::HashSet::new();
        for item in items {
            assert!(ids.insert(item.id));
            assert!(item.url.starts_with("https://"));
        }
    }
}
