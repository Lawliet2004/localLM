use rmcp::{
    service::RunningService,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    RoleClient, ServiceExt,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, sync::Arc, time::Duration};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolView {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolSelection {
    pub connector_id: String,
    pub tool_name: String,
}

fn select_views<'a>(
    available: &'a [ToolView],
    names: impl Iterator<Item = &'a str>,
    all: bool,
) -> Result<Vec<&'a ToolView>, String> {
    let mut selected = Vec::new();
    for name in names {
        let tool = available
            .iter()
            .find(|tool| tool.name == name)
            .ok_or_else(|| {
                format!("Selected tool {name} is no longer available. Review your tool selection.")
            })?;
        if !selected.iter().any(|item: &&ToolView| item.name == name) {
            selected.push(tool);
        }
    }
    if all {
        return Ok(available.iter().collect());
    }
    Ok(selected)
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
    service: ConnectorSession,
    tools: Vec<ToolView>,
    local_name: Option<String>,
}
enum ConnectorSession {
    Remote(RunningService<RoleClient, ()>),
    Local(crate::local_mcp_process::LocalSession),
}
impl std::ops::Deref for ConnectorSession {
    type Target = RunningService<RoleClient, ()>;
    fn deref(&self) -> &Self::Target {
        match self {
            Self::Remote(service) => service,
            Self::Local(session) => &session.service,
        }
    }
}
impl ConnectorSession {
    async fn cancel(self) -> Result<(), String> {
        match self {
            Self::Remote(service) => service
                .cancel()
                .await
                .map(|_| ())
                .map_err(|_| "Could not close the connector session cleanly.".into()),
            Self::Local(session) => {
                session.close().await;
                Ok(())
            }
        }
    }
}
fn local_view(server: &crate::local_mcp_config::LocalServer) -> ConnectorView {
    ConnectorView {
        id: server.id.clone(),
        description: server.name.clone(),
        url: String::new(),
        auth_type: "local".into(),
        connected: false,
        has_credential: false,
        tools: Vec::new(),
    }
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
pub fn tool_alias(connector: &str, name: &str) -> String {
    if connector == "Skills" && name == "read_file" {
        return "skills_read_file".into();
    }
    if connector == "Workspace" {
        return format!("workspace_{name}");
    }
    if connector == "Local execution" && name == "run_code" {
        return "local_run_code".into();
    }
    use sha2::{Digest, Sha256};
    let identity = format!("{connector}\0{name}");
    let digest = format!("{:x}", Sha256::digest(identity.as_bytes()));
    let readable = format!("{connector}_{name}")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' {
                character
            } else {
                '_'
            }
        })
        .take(47)
        .collect::<String>();
    format!("{readable}_{}", &digest[..16])
}
enum ToolBackend {
    Daytona(Arc<crate::daytona_execution::Executor>),
    Skills(Arc<crate::skills::SkillReader>),
    Execution(Arc<crate::execution::LocalExecution>),
    Mcp {
        peer: rmcp::Peer<RoleClient>,
        local_name: Option<String>,
    },
    Workspace(Arc<crate::workspace::Workspace>),
}
impl AgentTool {
    pub fn local_server_name(&self) -> Option<&str> {
        match &self.backend {
            ToolBackend::Mcp { local_name, .. } => local_name.as_deref(),
            _ => None,
        }
    }
    pub fn daytona(executor: crate::daytona_execution::Executor) -> Self {
        Self {connector:"Daytona".into(),alias:tool_alias("Daytona","run_code"),backend:ToolBackend::Daytona(Arc::new(executor)),tool:ToolView{
            name:"run_code".into(),description:"Execute code in a temporary Daytona cloud sandbox, then delete it. Code and results leave this device. No local workspace files are uploaded. Cloud usage may incur charges.".into(),
            input_schema:serde_json::json!({"type":"object","properties":{"language":{"type":"string","enum":["python","javascript","typescript"]},"code":{"type":"string","maxLength":32768},"timeout_seconds":{"type":"integer","minimum":1,"maximum":90}},"required":["language","code","timeout_seconds"],"additionalProperties":false})}}
    }
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(if matches!(self.backend, ToolBackend::Daytona(_)) {
            300
        } else {
            120
        })
    }
    pub fn skills(reader: Arc<crate::skills::SkillReader>, tool: ToolView) -> Self {
        Self {
            connector: "Skills".into(),
            alias: "skills_read_file".into(),
            tool,
            backend: ToolBackend::Skills(reader),
        }
    }
    pub fn trusted_read(&self) -> bool {
        matches!(&self.backend, ToolBackend::Workspace(_))
            && matches!(self.tool.name.as_str(), "read_file" | "list_files")
    }
    pub fn execution(execution: Arc<crate::execution::LocalExecution>, tool: ToolView) -> Self {
        Self {
            connector: "Local execution".into(),
            alias: "local_run_code".into(),
            tool,
            backend: ToolBackend::Execution(execution),
        }
    }
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
            ToolBackend::Daytona(executor) => return executor.call(arguments).await,
            ToolBackend::Skills(reader) => {
                let reader = reader.clone();
                return tokio::task::spawn_blocking(move || reader.call(arguments))
                    .await
                    .map_err(|_| "Skill read failed unexpectedly.")?;
            }
            ToolBackend::Execution(execution) => return execution.run(arguments).await,
            ToolBackend::Mcp { peer, .. } => peer,
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
    pub fn selected_tools(
        &self,
        ids: &[String],
        selections: &[ToolSelection],
    ) -> Result<Vec<AgentTool>, String> {
        if ids.len() > 32 || selections.len() > 32 {
            return Err("At most 32 tools can be offered in a turn.".into());
        }
        let mut tools = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for id in ids
            .iter()
            .chain(selections.iter().map(|selection| &selection.connector_id))
        {
            if !seen.insert(id) {
                continue;
            }
            let connection = self
                .connections
                .get(id)
                .filter(|value| !value.service.is_closed())
                .ok_or_else(|| format!("Connector {id} is not connected."))?;
            let selected = select_views(
                &connection.tools,
                selections
                    .iter()
                    .filter(|selection| &selection.connector_id == id)
                    .map(|selection| selection.tool_name.as_str()),
                ids.contains(id),
            )?;
            for tool in selected {
                if tools.len() >= 32 {
                    return Err(
                        "Select fewer tools: at most 32 tools can be offered in a turn.".into(),
                    );
                }
                tools.push(AgentTool {
                    connector: id.clone(),
                    tool: tool.clone(),
                    alias: tool_alias(id, &tool.name),
                    backend: ToolBackend::Mcp {
                        peer: connection.service.peer().clone(),
                        local_name: connection.local_name.clone(),
                    },
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
        items.extend(
            crate::local_mcp_config::load(&self.vault)?
                .iter()
                .map(local_view),
        );
        for item in &mut items {
            item.has_credential = if item.auth_type == "local" {
                false
            } else if item.auth_type == "oauth" {
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
    pub fn list_local(&self) -> Result<Vec<crate::local_mcp_config::LocalServerSummary>, String> {
        Ok(crate::local_mcp_config::load(&self.vault)?
            .iter()
            .map(Into::into)
            .collect())
    }
    pub fn read_local(&self, id: &str) -> Result<crate::local_mcp_config::LocalServer, String> {
        if self.connections.contains_key(id) {
            return Err("Disconnect the local server before editing its configuration.".into());
        }
        crate::local_mcp_config::load(&self.vault)?
            .into_iter()
            .find(|server| server.id == id)
            .ok_or("Unknown local connector.".into())
    }
    pub fn save_local(&self, server: crate::local_mcp_config::LocalServer) -> Result<(), String> {
        if self.connections.contains_key(&server.id) {
            return Err("Disconnect the local server before editing its configuration.".into());
        }
        server.validate()?;
        let mut servers = crate::local_mcp_config::load(&self.vault)?;
        if let Some(previous) = servers.iter_mut().find(|previous| previous.id == server.id) {
            *previous = server;
        } else {
            servers.push(server);
        }
        crate::local_mcp_config::save(&self.vault, &servers)
    }
    pub fn remove_local(&self, id: &str) -> Result<(), String> {
        if self.connections.contains_key(id) {
            return Err("Disconnect the local server before removing its configuration.".into());
        }
        let mut servers = crate::local_mcp_config::load(&self.vault)?;
        let index = servers
            .iter()
            .position(|server| server.id == id)
            .ok_or("Unknown local connector.")?;
        servers.remove(index);
        crate::local_mcp_config::save(&self.vault, &servers)
    }
    pub async fn connect(&mut self, id: &str) -> Result<ConnectorView, String> {
        let local = crate::local_mcp_config::load(&self.vault)?
            .into_iter()
            .find(|server| server.id == id);
        let mut item = match &local {
            Some(server) => local_view(server),
            None => preset(id)?,
        };
        if local.is_some() && self.connections.contains_key(id) {
            return Err("Disconnect the local server before reconnecting.".into());
        }
        let secret = if local.is_some() {
            None
        } else {
            self.token(id)?
        };
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
            if local.is_some() {
                return "Local connector discovery failed. Check its MCP tool catalog.".into();
            }
            secret
                .as_ref()
                .map(|secret| error.replace(secret, "[redacted]"))
                .unwrap_or(error)
        };
        let service = if let Some(server) = &local {
            ConnectorSession::Local(crate::local_mcp_process::connect(server).await?)
        } else if item.auth_type == "oauth" {
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
            ConnectorSession::Remote(
                tokio::time::timeout(Duration::from_secs(30), ().serve(transport))
                    .await
                    .map_err(|_| "Connector handshake timed out.")?
                    .map_err(|_| {
                        "Authorized connection failed. Check account access or sign in again."
                            .to_string()
                    })?,
            )
        } else {
            let transport = StreamableHttpClientTransport::with_client(client, config);
            ConnectorSession::Remote(
                tokio::time::timeout(Duration::from_secs(30), ().serve(transport))
                    .await
                    .map_err(|_| "Connector handshake timed out.")?
                    .map_err(|error| redact(format!("Could not connect: {error}")))?,
            )
        };
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            crate::tool_discovery::discover(|cursor| async {
                service
                    .list_tools(Some(
                        rmcp::model::PaginatedRequestParams::default().with_cursor(cursor),
                    ))
                    .await
                    .map_err(|error| error.to_string())
            }),
        )
        .await;
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
                local_name: local.as_ref().map(|server| server.name.clone()),
            },
        );
        item.connected = true;
        item.has_credential = secret.is_some() || item.auth_type == "oauth";
        item.tools = views;
        Ok(item)
    }
    pub async fn disconnect(&mut self, id: &str, forget: bool) -> Result<(), String> {
        let local = crate::local_mcp_config::load(&self.vault)?
            .iter()
            .any(|server| server.id == id);
        if !local {
            preset(id)?;
        }
        if let Some(connection) = self.connections.remove(id) {
            connection
                .service
                .cancel()
                .await
                .map_err(|_| "Could not close the connector session cleanly.")?;
        }
        if forget && !local {
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
pub async fn list_local_connectors(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<crate::local_mcp_config::LocalServerSummary>, String> {
    state.connectors.lock().await.list_local()
}
#[tauri::command]
pub async fn read_local_connector(
    state: tauri::State<'_, crate::AppState>,
    id: String,
) -> Result<crate::local_mcp_config::LocalServer, String> {
    state.connectors.lock().await.read_local(&id)
}
#[tauri::command]
pub async fn save_local_connector(
    state: tauri::State<'_, crate::AppState>,
    server: crate::local_mcp_config::LocalServer,
) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the current model operation before editing connectors.")?;
    state.connectors.lock().await.save_local(server)
}
#[tauri::command]
pub async fn remove_local_connector(
    state: tauri::State<'_, crate::AppState>,
    id: String,
) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the current model operation before editing connectors.")?;
    state.connectors.lock().await.remove_local(&id)
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
    #[tokio::test]
    async fn local_server_discovery_selection_and_disconnect_use_the_hub() {
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("fixture.cjs");
        std::fs::write(&script, r#"
require('node:readline').createInterface({input:process.stdin}).on('line', line => {
 const r = JSON.parse(line); if (r.id === undefined) return;
 const result = r.method === 'initialize'
 ? {protocolVersion:r.params.protocolVersion,capabilities:{tools:{}},serverInfo:{name:'fixture',version:'1'}}
 : r.method === 'tools/call'
 ? {content:[{type:'text',text:JSON.stringify({name:r.params.name,arguments:r.params.arguments})}],isError:false}
 : {tools:[{name:'read_file',description:'fixture',inputSchema:{type:'object'}}]};
 process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:r.id,result})+'\n');
});
"#).unwrap();
        let mut hub = super::McpHub::new(std::sync::Arc::new(crate::vault::Vault::new(
            temp.path().join("vault"),
        )));
        let server = crate::local_mcp_config::LocalServer {
            id: format!("local-{}", uuid::Uuid::new_v4()),
            name: "Fixture".into(),
            executable: crate::execution::ExecutionConfig::default().node_path,
            arguments: vec![script.to_string_lossy().into_owned()],
            working_directory: temp.path().to_string_lossy().into_owned(),
            environment: Default::default(),
        };
        hub.save_local(server.clone()).unwrap();
        assert!(
            !hub.list()
                .unwrap()
                .iter()
                .find(|item| item.id == server.id)
                .unwrap()
                .connected
        );
        let view = hub.connect(&server.id).await.unwrap();
        assert!(view.connected);
        assert_eq!(view.tools.len(), 1);
        assert_eq!(view.auth_type, "local");
        assert!(hub.save_local(server.clone()).is_err());
        assert!(hub.read_local(&server.id).is_err());
        assert!(hub.remove_local(&server.id).is_err());
        let tools = hub
            .selected_tools(
                &[],
                &[super::ToolSelection {
                    connector_id: server.id.clone(),
                    tool_name: "read_file".into(),
                }],
            )
            .unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].local_server_name(), Some("Fixture"));
        assert_eq!(tools[0].connector, server.id);
        assert!(
            !tools[0].trusted_read(),
            "server names must not grant read auto-approval"
        );
        let arguments = serde_json::json!({"path":"folder with spaces/日本語.txt", "nested":{"enabled":true}, "items":[1,"two"]});
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tools[0].call(arguments.clone()),
        )
        .await
        .unwrap()
        .unwrap();
        let returned: serde_json::Value =
            serde_json::from_str(response["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(
            returned,
            serde_json::json!({"name":"read_file", "arguments":arguments})
        );
        assert_eq!(response["isError"], false);
        assert!(tools[0]
            .call(serde_json::json!(["invalid argument shape"]))
            .await
            .is_err());
        hub.disconnect(&server.id, false).await.unwrap();
        assert!(tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tools[0].call(serde_json::json!({}))
        )
        .await
        .unwrap()
        .is_err());
        assert!(hub
            .selected_tools(std::slice::from_ref(&server.id), &[])
            .is_err());
        assert!(
            !hub.list()
                .unwrap()
                .iter()
                .find(|item| item.id == server.id)
                .unwrap()
                .connected
        );
        hub.remove_local(&server.id).unwrap();
    }
    #[test]
    fn local_configuration_crud_keeps_secrets_out_of_summaries() {
        let temp = tempfile::tempdir().unwrap();
        let vault = std::sync::Arc::new(crate::vault::Vault::new(temp.path().join("credentials")));
        let hub = super::McpHub::new(vault.clone());
        let mut server = crate::local_mcp_config::LocalServer {
            id: format!("local-{}", uuid::Uuid::new_v4()),
            name: "Local fixture".into(),
            executable: std::env::current_exe()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            arguments: vec!["private-argument-fixture".into()],
            working_directory: temp.path().to_string_lossy().into_owned(),
            environment: [("API_KEY".into(), "secret-value-fixture".into())].into(),
        };
        hub.save_local(server.clone()).unwrap();
        let summaries = hub.list_local().unwrap();
        let editable = hub.read_local(&server.id).unwrap();
        assert_eq!(editable.arguments, server.arguments);
        assert_eq!(editable.environment, server.environment);
        assert!(hub.read_local("unknown").is_err());
        assert_eq!(summaries.len(), 1);
        let text = serde_json::to_string(&summaries).unwrap();
        assert!(!text.contains("private-argument-fixture"));
        assert!(!text.contains("secret-value-fixture"));
        assert!(text.contains("API_KEY"));
        server.name = "Renamed".into();
        hub.save_local(server.clone()).unwrap();
        let reopened = super::McpHub::new(vault);
        assert_eq!(reopened.list_local().unwrap()[0].name, "Renamed");
        let mut invalid = server.clone();
        invalid.executable = "relative.exe".into();
        assert!(hub.save_local(invalid).is_err());
        assert_eq!(hub.list_local().unwrap().len(), 1);
        hub.remove_local(&server.id).unwrap();
        assert!(hub.list_local().unwrap().is_empty());
        assert!(hub.remove_local(&server.id).is_err());
    }
    use super::*;
    #[test]
    fn only_known_workspace_reads_qualify_for_automatic_approval() {
        let directory = tempfile::tempdir().unwrap();
        let workspace = Arc::new(
            crate::workspace::Workspace::open(directory.path().to_str().unwrap()).unwrap(),
        );
        let tools = workspace.tools();
        assert_eq!(tools.iter().filter(|tool| tool.trusted_read()).count(), 2);
        for tool in tools {
            assert_eq!(
                tool.trusted_read(),
                matches!(tool.tool.name.as_str(), "read_file" | "list_files")
            );
        }
    }
    #[test]
    fn aliases_are_stable_bounded_and_distinguish_sanitization_collisions() {
        let alias = tool_alias("deepwiki", "read_wiki_structure");
        assert_eq!(alias, tool_alias("deepwiki", "read_wiki_structure"));
        assert!(alias.starts_with("deepwiki_read_wiki_structure_"));
        assert_ne!(tool_alias("x", "a-b"), tool_alias("x", "a_b"));
        assert_ne!(tool_alias("x", "read"), tool_alias("y", "read"));
        let long = tool_alias("connector", &"long-tool".repeat(50));
        assert_eq!(long.len(), 64);
        assert!(long
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'));
        assert_eq!(tool_alias("Workspace", "read_file"), "workspace_read_file");
        assert_eq!(tool_alias("Local execution", "run_code"), "local_run_code");
    }
    #[test]
    fn selects_exact_tools_from_large_catalog_and_rejects_stale_names() {
        let tools: Vec<ToolView> = (0..100)
            .map(|index| ToolView {
                name: format!("tool_{index}"),
                description: String::new(),
                input_schema: serde_json::json!({"type":"object"}),
            })
            .collect();
        let selected =
            select_views(&tools, ["tool_92", "tool_3", "tool_92"].into_iter(), false).unwrap();
        assert_eq!(
            selected
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            ["tool_92", "tool_3"]
        );
        assert!(select_views(&tools, ["removed_tool"].into_iter(), false).is_err());
        assert!(select_views(&tools, ["removed_tool"].into_iter(), true).is_err());
        assert_eq!(
            select_views(&tools, std::iter::empty(), true)
                .unwrap()
                .len(),
            100
        );
        assert!(select_views(&tools, std::iter::empty(), false)
            .unwrap()
            .is_empty());
    }
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
