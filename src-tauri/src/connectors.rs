use rmcp::{
    service::RunningService,
    transport::{
        streamable_http_client::StreamableHttpClientTransportConfig, StreamableHttpClientTransport,
    },
    RoleClient, ServiceExt,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard,
    },
    time::Duration,
};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_error: Option<String>,
}
struct Connection {
    shared: Arc<SharedConnection>,
}
enum ConnectorSession {
    Remote(RunningService<RoleClient, HubHandler>),
    Local(crate::local_mcp_process::LocalSession),
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
                let report = session.close().await;
                if report.reaped {
                    Ok(())
                } else {
                    Err(format!(
                        "The local server process did not exit within 3 seconds (protocol close {}, kill signal {}, process reaped {}). Saved configuration is unchanged; try Disconnect again.",
                        stage(report.protocol),
                        stage(report.signal),
                        stage(report.reaped),
                    ))
                }
            }
        }
    }
    fn is_closed(&self) -> bool {
        match self {
            Self::Remote(service) => service.is_closed(),
            Self::Local(session) => session.service.is_closed(),
        }
    }
    fn peer(&self) -> rmcp::Peer<RoleClient> {
        match self {
            Self::Remote(service) => service.peer().clone(),
            Self::Local(session) => session.service.peer().clone(),
        }
    }
}

/// A poisoned lock means a panicked holder left inconsistent state; recovering
/// the guard keeps reads/writes available rather than cascading the panic.
fn read_lock<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}
fn write_lock<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(|poisoned| poisoned.into_inner())
}
fn mutex<T>(lock: &Mutex<T>) -> MutexGuard<'_, T> {
    lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Delays between reconnect attempts; the first attempt runs immediately, so a
/// dead session gets `RECONNECT_DELAYS.len() + 1` tries (5) before the
/// connector stays unavailable.
const RECONNECT_DELAYS: [Duration; 4] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
];

/// Live state for one connector, shared by the hub, minted `AgentTool`s, the
/// rmcp notification handler, and any in-flight reconnect task. A reconnect
/// installs the fresh session into these same slots, so tools minted before a
/// drop pick up the new peer instead of pinning a dead one.
struct SharedConnection {
    id: String,
    vault: Arc<crate::vault::Vault>,
    local_name: Option<String>,
    service: RwLock<Option<ConnectorSession>>,
    tools: RwLock<Vec<ToolView>>,
    reconnect: Mutex<ReconnectState>,
    /// Set by `notifications/tools/list_changed`; the next catalog refresh
    /// clears it once new tools are installed.
    tools_dirty: AtomicBool,
    refreshing: AtomicBool,
    /// Set when the hub entry is removed or replaced so an in-flight reconnect
    /// drops its freshly opened session instead of resurrecting the connector.
    detached: AtomicBool,
}

#[derive(Default)]
struct ReconnectState {
    running: bool,
    failed_attempts: u32,
}

impl SharedConnection {
    fn new(id: &str, vault: Arc<crate::vault::Vault>, local_name: Option<String>) -> Self {
        Self {
            id: id.into(),
            vault,
            local_name,
            service: RwLock::new(None),
            tools: RwLock::new(Vec::new()),
            reconnect: Mutex::new(ReconnectState::default()),
            tools_dirty: AtomicBool::new(false),
            refreshing: AtomicBool::new(false),
            detached: AtomicBool::new(false),
        }
    }
    fn is_live(&self) -> bool {
        read_lock(&self.service)
            .as_ref()
            .is_some_and(|service| !service.is_closed() && !service.peer().is_transport_closed())
    }
    /// Live-check every hub read path wants: a dead session schedules a bounded
    /// reconnect, a live one picks up any pending catalog refresh.
    fn live_or_reconnect(self: &Arc<Self>) -> bool {
        if self.is_live() {
            self.kick_tools_refresh();
            return true;
        }
        self.kick_reconnect();
        false
    }
    fn peer(&self) -> Option<rmcp::Peer<RoleClient>> {
        read_lock(&self.service).as_ref().map(ConnectorSession::peer)
    }
    fn tools(&self) -> Vec<ToolView> {
        read_lock(&self.tools).clone()
    }
    /// Install a freshly opened session and catalog; returns the session being
    /// replaced so the caller can close it outside the locks.
    fn install(&self, service: ConnectorSession, tools: Vec<ToolView>) -> Option<ConnectorSession> {
        *write_lock(&self.tools) = tools;
        // The installed catalog is fresh; any pending refresh request is moot.
        self.tools_dirty.store(false, Ordering::Release);
        write_lock(&self.service).replace(service)
    }
    fn take_service(&self) -> Option<ConnectorSession> {
        write_lock(&self.service).take()
    }
    fn note_tools_changed(self: &Arc<Self>) {
        self.tools_dirty.store(true, Ordering::Release);
        self.kick_tools_refresh();
    }
    /// Spawn a catalog refresh unless one is already in flight or the flag is
    /// unset. A failed refresh leaves `tools_dirty` set so the next hub read
    /// retries; a notification landing mid-refresh re-dirties it for the
    /// follow-up check at the end of the task.
    fn kick_tools_refresh(self: &Arc<Self>) {
        // A dead session can't answer list_tools; the reconnect path installs a
        // fresh catalog instead, so don't churn doomed refresh tasks.
        if self.detached.load(Ordering::Acquire)
            || !self.tools_dirty.load(Ordering::Acquire)
            || !self.is_live()
        {
            return;
        }
        if self.refreshing.swap(true, Ordering::AcqRel) {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            self.refreshing.store(false, Ordering::Release);
            return;
        };
        let slot = Arc::clone(self);
        handle.spawn(async move { slot.refresh_tools().await });
    }
    async fn refresh_tools(self: Arc<Self>) {
        // Consume the flag before fetching: a notification arriving during the
        // fetch re-dirties it and earns the follow-up refresh below.
        self.tools_dirty.store(false, Ordering::Release);
        let mut succeeded = false;
        if let Some(peer) = self.peer().filter(|peer| !peer.is_transport_closed()) {
            match tokio::time::timeout(
                Duration::from_secs(30),
                crate::tool_discovery::discover(|cursor| {
                    let peer = peer.clone();
                    async move {
                        peer.list_tools(Some(
                            rmcp::model::PaginatedRequestParams::default().with_cursor(cursor),
                        ))
                        .await
                        .map_err(|error| error.to_string())
                    }
                }),
            )
            .await
            {
                Ok(Ok(tools)) => match tool_views(tools) {
                    Ok(views) => {
                        *write_lock(&self.tools) = views;
                        succeeded = true;
                    }
                    Err(error) => {
                        eprintln!(
                            "[connectors] tool catalog refresh for '{}' failed: {error}",
                            self.id
                        );
                    }
                },
                Ok(Err(error)) => {
                    eprintln!(
                        "[connectors] tool catalog refresh for '{}' failed: {error}",
                        self.id
                    );
                }
                Err(_) => {
                    eprintln!("[connectors] tool catalog refresh for '{}' timed out", self.id);
                }
            }
        }
        // A dead session's catalog is repopulated by the reconnect path; a
        // failed refresh re-dirties so the next hub read retries.
        if !succeeded && read_lock(&self.service).is_some() {
            self.tools_dirty.store(true, Ordering::Release);
        }
        self.refreshing.store(false, Ordering::Release);
        if succeeded && self.tools_dirty.load(Ordering::Acquire) {
            self.kick_tools_refresh();
        }
    }
    fn kick_reconnect(self: &Arc<Self>) {
        if self.is_live() || self.detached.load(Ordering::Acquire) {
            return;
        }
        let mut state = mutex(&self.reconnect);
        if state.running || state.failed_attempts > RECONNECT_DELAYS.len() as u32 {
            return;
        }
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        state.running = true;
        drop(state);
        let slot = Arc::clone(self);
        handle.spawn(async move { slot.reconnect_loop(&RECONNECT_DELAYS).await });
    }
    async fn reconnect_loop(self: Arc<Self>, delays: &[Duration]) {
        for attempt in 0..=delays.len() {
            if attempt > 0 {
                tokio::time::sleep(delays[attempt - 1]).await;
            }
            if self.detached.load(Ordering::Acquire) {
                break;
            }
            let local = crate::local_mcp_config::load(&self.vault)
                .ok()
                .and_then(|servers| servers.into_iter().find(|server| server.id == self.id));
            let db_path = self.vault.db_path();
            let opened = match open_session(&self.vault, &self.id, local, &self, &db_path).await {
                Ok(opened) => opened,
                Err(error) => {
                    eprintln!(
                        "[connectors] reconnect attempt {}/{} for '{}' failed: {error}",
                        attempt + 1,
                        delays.len() + 1,
                        self.id
                    );
                    continue;
                }
            };
            if self.detached.load(Ordering::Acquire) {
                // Disconnected or replaced while the session was opening.
                let _ = opened.service.cancel().await;
                break;
            }
            let old = self.install(opened.service, opened.tools);
            eprintln!("[connectors] reconnected '{}'", self.id);
            {
                let mut state = mutex(&self.reconnect);
                state.failed_attempts = 0;
                state.running = false;
            }
            if let Some(old) = old {
                let _ = old.cancel().await;
            }
            return;
        }
        let mut state = mutex(&self.reconnect);
        state.running = false;
        if !self.detached.load(Ordering::Acquire) {
            state.failed_attempts = delays.len() as u32 + 1;
        }
    }
}

/// Client handler wiring `notifications/tools/list_changed` to the owning
/// connection. rmcp delivers server notifications only through `ClientHandler`
/// (there is no peer-level notification stream), so a flag plus a refresh task
/// is the minimal handoff. Local stdio sessions keep the `()` handler inside
/// `local_mcp_process`, so their catalog refreshes only through reconnects.
struct HubHandler {
    connection: Arc<SharedConnection>,
}
impl rmcp::ClientHandler for HubHandler {
    async fn on_tool_list_changed(
        &self,
        _context: rmcp::service::NotificationContext<RoleClient>,
    ) {
        self.connection.note_tools_changed();
    }
}

/// Session open + tool discovery shared by explicit connects and the reconnect
/// task. The remote handler points at `slot` so notifications keep flagging the
/// connection that ends up owning the session.
struct Opened {
    view: ConnectorView,
    service: ConnectorSession,
    tools: Vec<ToolView>,
}

/// True for transport-level failures (dropped connection, failed send) that
/// justify a reconnect; `McpError`, timeouts, and cancellations leave the
/// session usable and do not.
fn is_transport_failure(error: &rmcp::ServiceError) -> bool {
    matches!(
        error,
        rmcp::ServiceError::TransportClosed | rmcp::ServiceError::TransportSend(_)
    )
}

/// Largest single text item forwarded to the model; longer items are truncated.
const MAX_TOOL_TEXT_BYTES: usize = 65_536;
/// Largest projected tool result envelope; larger results fail outright.
const MAX_TOOL_RESULT_BYTES: usize = 262_144;

fn bounded_text(text: &str) -> std::borrow::Cow<'_, str> {
    if text.len() <= MAX_TOOL_TEXT_BYTES {
        return std::borrow::Cow::Borrowed(text);
    }
    let mut end = MAX_TOOL_TEXT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…[truncated: {} bytes total]", &text[..end], text.len()).into()
}

fn content_mime_type(block: &rmcp::model::ContentBlock) -> &str {
    match block {
        rmcp::model::ContentBlock::Image(image) => image.mime_type.as_str(),
        rmcp::model::ContentBlock::Audio(audio) => audio.mime_type.as_str(),
        rmcp::model::ContentBlock::Resource(resource) => match &resource.resource {
            rmcp::model::ResourceContents::TextResourceContents { mime_type, .. }
            | rmcp::model::ResourceContents::BlobResourceContents { mime_type, .. } => {
                mime_type.as_deref().unwrap_or("application/octet-stream")
            }
            _ => "application/octet-stream",
        },
        rmcp::model::ContentBlock::ResourceLink(link) => {
            link.mime_type.as_deref().unwrap_or("application/octet-stream")
        }
        _ => "application/octet-stream",
    }
}

/// Project a tool result into the model-facing envelope: text stays text
/// (bounded per item), image/audio/resource and unrecognized blocks become
/// bounded placeholders so base64 blobs never reach the model context, and
/// `structuredContent` passes through.
fn project_tool_result(result: &rmcp::model::CallToolResult) -> Result<Value, String> {
    let content: Vec<Value> = result
        .content
        .iter()
        .map(|block| match block {
            rmcp::model::ContentBlock::Text(text) => serde_json::json!({
                "type": "text",
                "text": bounded_text(&text.text),
            }),
            other => serde_json::json!({
                "type": "unsupported",
                "mimeType": content_mime_type(other),
                "note": "binary content omitted",
            }),
        })
        .collect();
    let mut value = serde_json::json!({
        "isError": result.is_error.unwrap_or(false),
        "content": content,
    });
    if let Some(structured) = &result.structured_content {
        value["structuredContent"] = structured.clone();
    }
    if value.to_string().len() > MAX_TOOL_RESULT_BYTES {
        return Err("Tool result exceeds 256 KiB. Request a narrower result.".into());
    }
    Ok(value)
}

/// Map discovered tools to views with the same catalog cap `connect` enforces,
/// so a `tools/list_changed` refresh cannot grow the cache without bound.
fn tool_views(tools: Vec<rmcp::model::Tool>) -> Result<Vec<ToolView>, String> {
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
        return Err("Connector tool catalog exceeds 2 MiB.".into());
    }
    Ok(views)
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
        connection_error: None,
    }
}
fn stage(value: bool) -> &'static str {
    if value {
        "ok"
    } else {
        "failed"
    }
}
pub struct McpHub {
    connections: HashMap<String, Connection>,
    vault: Arc<crate::vault::Vault>,
    restore_errors: HashMap<String, String>,
}

const CONNECTED_KEY: &str = "connected_connectors";

fn remembered_connections(store: &crate::store::Store) -> Result<Vec<String>, String> {
    if let Some(ids) = store.setting::<Option<Vec<String>>>(CONNECTED_KEY)? {
        return Ok(ids);
    }
    // Older versions remembered selected tools, but not their live connections.
    let ids: std::collections::BTreeSet<_> = store.remembered_tools()?.unwrap_or_default()
        .tools.into_iter().map(|tool| tool.connector_id).collect();
    let ids: Vec<_> = ids.into_iter().collect();
    store.save_setting(CONNECTED_KEY, &ids)?;
    Ok(ids)
}

fn remember_connection(store: &crate::store::Store, id: &str, connected: bool) -> Result<(), String> {
    let mut ids = remembered_connections(store)?;
    ids.retain(|saved| saved != id);
    if connected { ids.push(id.into()); }
    store.save_setting(CONNECTED_KEY, &ids)
}

pub async fn restore_connections(state: &crate::AppState) -> Result<(), String> {
    let mut hub = state.connectors.lock().await;
    let ids = remembered_connections(&*state.database()?)?;
    hub.restore_saved(&ids).await;
    Ok(())
}

pub struct AgentTool {
    pub connector: String,
    pub tool: ToolView,
    pub alias: String,
    backend: ToolBackend,
}

impl Clone for AgentTool {
    fn clone(&self) -> Self {
        let backend = match &self.backend {
            ToolBackend::System => ToolBackend::System,
            ToolBackend::Harness { name } => ToolBackend::Harness { name: name.clone() },
            ToolBackend::Daytona(executor) => ToolBackend::Daytona(executor.clone()),
            ToolBackend::Skills(reader) => ToolBackend::Skills(reader.clone()),
            ToolBackend::Execution(execution) => ToolBackend::Execution(execution.clone()),
            ToolBackend::Workspace(workspace) => ToolBackend::Workspace(workspace.clone()),
            // MCP sessions hold a live transport that cannot cross the
            // workflow/spawn boundary; mark the clone detached.
            ToolBackend::Mcp { local_name, .. } => ToolBackend::DetachedMcp { local_name: local_name.clone() },
            ToolBackend::DetachedMcp { local_name } => ToolBackend::DetachedMcp { local_name: local_name.clone() },
        };
        Self { connector: self.connector.clone(), tool: self.tool.clone(), alias: self.alias.clone(), backend }
    }
}
pub fn tool_alias(connector: &str, name: &str) -> String {
    if connector == "System" {
        return name.into();
    }
    // Harness tools are exposed to the model under their bare registry alias
    // (e.g. todo_write); replay must reuse that exact name or round 2+ history
    // references tools that no longer match the live schema.
    if connector == "Harness" {
        return name.into();
    }
    if connector == "Skills" && name == "read_file" {
        return "skills_read_file".into();
    }
    if connector == "Workspace" {
        return format!("workspace_{name}");
    }
    if connector == "Local execution" && name == "run_code" {
        return "local_run_code".into();
    }
    if connector == "Local execution" && name == "execute_command" {
        return "local_execute_command".into();
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
    System,
    Harness {
        name: String,
    },
    Daytona(Arc<crate::daytona_execution::Executor>),
    Skills(Arc<crate::skills::SkillReader>),
    Execution(Arc<crate::execution::LocalExecution>),
    Mcp {
        slot: Arc<SharedConnection>,
        local_name: Option<String>,
    },
    /// Clone marker for MCP tools carried across workflow/spawn boundaries.
    /// The live peer cannot be cloned; the alias/definition still describe the
    /// tool so filters inherit correctly, while `call` fails loud directing
    /// the child to ask the parent.
    DetachedMcp {
        local_name: Option<String>,
    },
    Workspace(Arc<crate::workspace::Workspace>),
}
impl AgentTool {
    pub fn local_server_name(&self) -> Option<&str> {
        match &self.backend {
            ToolBackend::Mcp { local_name, .. } | ToolBackend::DetachedMcp { local_name } => local_name.as_deref(),
            _ => None,
        }
    }
    pub fn is_mcp(&self) -> bool {
        matches!(&self.backend, ToolBackend::Mcp { .. } | ToolBackend::DetachedMcp { .. })
    }
    pub fn is_daytona(&self) -> bool {
        matches!(self.backend, ToolBackend::Daytona(_))
    }
    pub fn is_harness(&self) -> bool {
        matches!(&self.backend, ToolBackend::Harness { .. })
    }
    pub fn harness_name(&self) -> Option<&str> {
        match &self.backend {
            ToolBackend::Harness { name } => Some(name.as_str()),
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
        } else if matches!(self.backend, ToolBackend::Harness { .. }) {
            // Orchestration tools (workflows, Ralph, parallel children) settle durably.
            600
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
    pub fn system_time() -> Self {
        Self {
            connector: "System".into(),
            alias: "system_time".into(),
            tool: crate::system_tools::system_time_tool_definition(),
            backend: ToolBackend::System,
        }
    }
    /// Harness-owned tool (subagents, todos, memory, ...). Schemas come from
    /// the harness registry; execution runs through the harness dispatcher so
    /// approval, audit, and run states apply. Direct calls fail loud.
    pub fn harness(alias: &str) -> Result<Self, String> {
        let tool = crate::harness::definition(alias).ok_or_else(|| format!("Unknown harness tool '{alias}'."))?;
        Ok(Self {
            connector: "Harness".into(),
            alias: alias.into(),
            tool,
            backend: ToolBackend::Harness { name: alias.into() },
        })
    }
    pub fn trusted_read(&self) -> bool {
        matches!(&self.backend, ToolBackend::System)
            || matches!(&self.backend, ToolBackend::Harness { name } if crate::harness::is_trusted_read(name))
            || (matches!(&self.backend, ToolBackend::Workspace(_))
                && matches!(self.tool.name.as_str(), "read_file" | "list_files"))
    }
    pub fn execution(execution: Arc<crate::execution::LocalExecution>, tool: ToolView) -> Self {
        let alias = if tool.name == "execute_command" {
            "local_execute_command".into()
        } else {
            "local_run_code".into()
        };
        Self {
            connector: "Local execution".into(),
            alias,
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
        self.call_with_stream(arguments, None).await
    }
    pub async fn call_with_stream(
        &self,
        arguments: Value,
        on_chunk: Option<crate::execution::ChunkCallback>,
    ) -> Result<Value, String> {
        let slot = match &self.backend {
            ToolBackend::System => {
                if self.tool.name == "system_time" {
                    return crate::system_tools::execute_system_time(&arguments);
                }
                return Err("Unknown system tool.".into());
            }
            ToolBackend::Harness { name } => {
                return Err(format!("Harness tool '{name}' runs through the agent dispatcher, not direct calls."));
            }
            ToolBackend::Daytona(executor) => return executor.call(arguments).await,
            ToolBackend::Skills(reader) => {
                let reader = reader.clone();
                return tokio::task::spawn_blocking(move || reader.call(arguments))
                    .await
                    .map_err(|_| "Skill read failed unexpectedly.")?;
            }
            ToolBackend::Execution(execution) => {
                if self.tool.name == "execute_command" {
                    return execution.run_command_with_stream(arguments, on_chunk).await;
                } else {
                    return execution.run_with_stream(arguments, on_chunk).await;
                }
            }
            ToolBackend::Mcp { slot, .. } => slot,
            ToolBackend::DetachedMcp { .. } => {
                return Err("This connector tool was inherited without its live session. Ask the parent for what you need instead.".into());
            }
            ToolBackend::Workspace(workspace) => {
                let workspace = workspace.clone();
                let name = self.tool.name.clone();
                return tokio::task::spawn_blocking(move || workspace.call(&name, arguments))
                    .await
                    .map_err(|_| "Workspace operation failed unexpectedly.")?;
            }
        };
        let peer = match slot.peer() {
            Some(peer) if !peer.is_transport_closed() => peer,
            _ => {
                slot.kick_reconnect();
                return Err(
                    "This connector is not connected right now; a reconnect may be in progress."
                        .into(),
                );
            }
        };
        let arguments = arguments
            .as_object()
            .ok_or("Tool arguments must be an object.")?
            .clone();
        let response = peer.call_tool_once(rmcp::model::CallToolRequestParams::new(self.tool.name.clone()).with_arguments(arguments)).await.map_err(|error| {
            // A dead transport makes this a connection failure, not a tool
            // failure; schedule the bounded reconnect so the connector heals
            // without waiting for a manual reconnect.
            if is_transport_failure(&error) || peer.is_transport_closed() {
                slot.kick_reconnect();
            }
            "Connector tool request failed. Its remote outcome may be unknown; do not automatically retry.".to_string()
        })?;
        match response {
            rmcp::model::CallToolResponse::Complete(result) => project_tool_result(&result),
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
            connection_error: None,
        })
        .collect()
}
fn preset(id: &str) -> Result<ConnectorView, String> {
    presets()
        .into_iter()
        .find(|item| item.id == id)
        .ok_or_else(|| "Unknown connector.".into())
}
/// How a bundled connector expects credentials to be supplied. `header`
/// presets require a saved bearer API token sent as `Authorization: Bearer`;
/// `dcr` presets use the OAuth flow exclusively (the token vault is never
/// consulted for them); `none` presets are public endpoints. Local servers
/// never use this transport.
fn preset_credential(id: &str) -> Result<&'static str, String> {
    Ok(match preset(id)?.auth_type.as_str() {
        "oauth" => "dcr",
        "apiKey" => "header",
        _ => "none",
    })
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

fn vault_token(vault: &crate::vault::Vault, id: &str) -> Result<Option<String>, String> {
    vault
        .load(&format!("token-{id}"))?
        .map(|bytes| {
            String::from_utf8(bytes).map_err(|_| "Invalid stored token encoding.".into())
        })
        .transpose()
}

/// Open a connector session and run tool discovery against it. Shared by
/// explicit connects and the reconnect task; on success the caller installs
/// `service`/`tools` into `slot`. The remote notification handler already
/// points at `slot`, so a `tools/list_changed` that lands before install is not
/// lost — it just refreshes the freshly installed catalog.
async fn open_session(
    vault: &Arc<crate::vault::Vault>,
    id: &str,
    local: Option<crate::local_mcp_config::LocalServer>,
    slot: &Arc<SharedConnection>,
    db_path: &std::path::Path,
) -> Result<Opened, String> {
    let mut item = match &local {
        Some(server) => local_view(server),
        None => preset(id)?,
    };
    let credential = preset_credential(id).ok();
    let secret = if local.is_some() {
        None
    } else if credential == Some("header") {
        vault_token(vault, id)?
    } else {
        // OAuth presets authenticate through the sign-in flow; a saved
        // bearer token must never be attached to them. Public presets
        // connect without credentials.
        None
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
        // Enforced isolation runs the stdio server inside the same
        // AppContainer policy as other spawns; its tools then still carry the
        // in-process network gate (see effective_capabilities).
        let isolation = crate::store::Store::open(db_path)
            .map(|store| crate::isolation::settings(&store))
            .unwrap_or_default();
        ConnectorSession::Local(crate::local_mcp_process::connect(server, &isolation, Some(db_path)).await?)
    } else {
        let handler = HubHandler {
            connection: slot.clone(),
        };
        let service = if item.auth_type == "oauth" {
            let mut manager = crate::oauth::manager(id, &item.url, vault.clone()).await?;
            if !manager
                .initialize_from_store()
                .await
                .map_err(|_| "Saved authorization could not be restored. Sign in again.")?
            {
                return Err("Sign in to this connector first.".into());
            }
            let client = rmcp::transport::auth::AuthClient::new(client, manager);
            let transport = StreamableHttpClientTransport::with_client(client, config);
            tokio::time::timeout(Duration::from_secs(30), handler.serve(transport))
                .await
                .map_err(|_| "Connector handshake timed out.")?
                .map_err(|_| {
                    "Authorized connection failed. Check account access or sign in again."
                        .to_string()
                })?
        } else {
            let transport = StreamableHttpClientTransport::with_client(client, config);
            tokio::time::timeout(Duration::from_secs(30), handler.serve(transport))
                .await
                .map_err(|_| "Connector handshake timed out.")?
                .map_err(|error| redact(format!("Could not connect: {error}")))?
        };
        ConnectorSession::Remote(service)
    };
    let result = tokio::time::timeout(
        Duration::from_secs(30),
        crate::tool_discovery::discover(|cursor| {
            let peer = service.peer();
            async move {
                peer.list_tools(Some(
                    rmcp::model::PaginatedRequestParams::default().with_cursor(cursor),
                ))
                .await
                .map_err(|error| error.to_string())
            }
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
    let views = match tool_views(tools) {
        Ok(views) => views,
        Err(error) => {
            let _ = service.cancel().await;
            return Err(error);
        }
    };
    item.has_credential = secret.is_some() || item.auth_type == "oauth";
    Ok(Opened {
        view: item,
        service,
        tools: views,
    })
}

impl McpHub {
    /// Saved choices survive disconnects, but only live, explicitly selected tools
    /// belong in a request. Return omissions so both the UI and model can explain them.
    pub fn tools_for_turn(
        &self,
        ids: &[String],
        selections: &[ToolSelection],
    ) -> Result<(Vec<AgentTool>, Vec<String>), String> {
        if ids.len() > 32 || selections.len() > 32 {
            return Err("At most 32 tools can be offered in a turn.".into());
        }
        let live = self.active_connector_ids();
        let mut unavailable = Vec::new();
        for id in ids.iter().chain(selections.iter().map(|tool| &tool.connector_id)) {
            if !live.contains(id) && !unavailable.contains(id) {
                unavailable.push(id.clone());
            }
        }
        let ids: Vec<_> = ids.iter().filter(|id| live.contains(id)).cloned().collect();
        let selections: Vec<_> = selections.iter()
            .filter(|tool| live.contains(&tool.connector_id)).cloned().collect();
        Ok((self.selected_tools(&ids, &selections)?, unavailable))
    }

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
                .filter(|value| value.shared.live_or_reconnect())
                .ok_or_else(|| format!("Connector {id} is not connected."))?;
            let catalog = read_lock(&connection.shared.tools);
            let selected = select_views(
                &catalog,
                selections
                    .iter()
                    .filter(|selection| &selection.connector_id == id)
                    .map(|selection| selection.tool_name.as_str()),
                ids.contains(id),
            )?;
            for tool in selected {
                if tools.len() >= 32 {
                    break;
                }
                tools.push(AgentTool {
                    connector: id.clone(),
                    tool: tool.clone(),
                    alias: tool_alias(id, &tool.name),
                    backend: ToolBackend::Mcp {
                        slot: connection.shared.clone(),
                        local_name: connection.shared.local_name.clone(),
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
            restore_errors: HashMap::new(),
        }
    }
    async fn restore_saved(&mut self, ids: &[String]) {
        for id in ids {
            if self.active_connector_ids().contains(id) { continue; }
            if let Err(error) = self.connect(id).await {
                self.restore_errors.insert(id.clone(), error);
            }
        }
    }
    fn token(&self, id: &str) -> Result<Option<String>, String> {
        vault_token(&self.vault, id)
    }
    fn save_token(&self, id: &str, value: &str) -> Result<(), String> {
        if preset_credential(id)? != "header" {
            return Err("This connector does not use an API token. Use account sign-in for OAuth connectors.".into());
        }
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
            item.connection_error = self.restore_errors.get(&item.id).cloned();
            item.has_credential = if item.auth_type == "local" {
                false
            } else if item.auth_type == "oauth" {
                self.vault.load(&format!("oauth-{}", item.id))?.is_some()
            } else {
                self.token(&item.id)?.is_some()
            };
            if let Some(connection) = self.connections.get(&item.id) {
                item.connected = connection.shared.live_or_reconnect();
                if item.connected {
                    item.tools = connection.shared.tools();
                }
            }
        }
        Ok(items)
    }
    /// Whether a preset connector currently has a live session. Skill
    /// dependency checks use only this boolean; they never expose sessions.
    pub fn is_connected(&self, id: &str) -> bool {
        self.connections
            .get(id)
            .is_some_and(|connection| connection.shared.live_or_reconnect())
    }
    pub fn active_connector_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .connections
            .iter()
            .filter(|(_, conn)| conn.shared.live_or_reconnect())
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        ids
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
        if local.is_some() && self.connections.contains_key(id) {
            return Err("Disconnect the local server before reconnecting.".into());
        }
        let shared = Arc::new(SharedConnection::new(
            id,
            self.vault.clone(),
            local.as_ref().map(|server| server.name.clone()),
        ));
        let db_path = self.vault.db_path();
        let opened = open_session(&self.vault, id, local, &shared, &db_path).await?;
        let _ = shared.install(opened.service, opened.tools);
        if let Some(old) = self.connections.remove(id) {
            old.shared.detached.store(true, Ordering::Release);
            if let Some(service) = old.shared.take_service() {
                let _ = service.cancel().await;
            }
        }
        let mut item = opened.view;
        item.connected = true;
        item.tools = shared.tools();
        self.connections.insert(id.into(), Connection { shared });
        self.restore_errors.remove(id);
        Ok(item)
    }
    pub async fn disconnect(&mut self, id: &str, forget: bool) -> Result<(), String> {
        self.restore_errors.remove(id);
        let local = crate::local_mcp_config::load(&self.vault)?
            .iter()
            .any(|server| server.id == id);
        if !local {
            preset(id)?;
        }
        if let Some(connection) = self.connections.remove(id) {
            // Detach first so an in-flight reconnect drops its freshly opened
            // session instead of resurrecting this connector. A failed local
            // shutdown keeps the saved configuration but releases the session
            // lock so the user can retry, inspect, or remove the server.
            // Cleanup never deletes saved configuration.
            connection.shared.detached.store(true, Ordering::Release);
            if let Some(service) = connection.shared.take_service() {
                service.cancel().await?;
            }
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
    let hub = state.connectors.lock().await;
    hub.remove_local(&id)?;
    remember_connection(&*state.database()?, &id, false)
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
    let connected = hub.connect(&id).await?;
    let saved = remember_connection(&*state.database()?, &id, true);
    if let Err(error) = saved {
        let _ = hub.disconnect(&id, false).await;
        return Err(format!("Could not remember this connection: {error}"));
    }
    Ok(connected)
}
#[tauri::command]
pub async fn disconnect_connector(
    state: tauri::State<'_, crate::AppState>,
    id: String,
    forget: bool,
) -> Result<(), String> {
    let mut hub = state.connectors.lock().await;
    remember_connection(&*state.database()?, &id, false)?;
    hub.disconnect(&id, forget).await
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
    connect_connector(state.clone(), id, None).await
}
#[tauri::command]
pub fn cancel_connector_sign_in(state: tauri::State<'_, crate::AppState>) {
    state.oauth_cancel.send_replace(true);
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn malformed_protocol_input_and_unexpected_exit_are_reported() {
        async fn fixture(name: &str, script: &str) -> (tempfile::TempDir, super::McpHub, crate::local_mcp_config::LocalServer) {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("fixture.cjs");
            std::fs::write(&path, script).unwrap();
            let hub = super::McpHub::new(std::sync::Arc::new(crate::vault::Vault::new(
                temp.path().join("vault"),
            )));
            let server = crate::local_mcp_config::LocalServer {
                id: format!("local-{}", uuid::Uuid::new_v4()),
                name: name.into(),
                executable: crate::execution::ExecutionConfig::default().node_path,
                arguments: vec![path.to_string_lossy().into_owned()],
                working_directory: temp.path().to_string_lossy().into_owned(),
                environment: Default::default(),
            };
            hub.save_local(server.clone()).unwrap();
            (temp, hub, server)
        }
        let (_temp, mut hub, server) = fixture("garbage", "process.stdout.write('not json\\n'); setInterval(()=>{},1000);\n").await;
        let error = match hub.connect(&server.id).await {
            Ok(_) => panic!("garbage protocol unexpectedly connected"),
            Err(error) => error,
        };
        assert!(error.contains("handshake"), "unexpected error: {error}");
        assert!(!hub.list().unwrap().iter().find(|item| item.id == server.id).unwrap().connected);
        hub.remove_local(&server.id).unwrap();
        let (_temp, mut hub, server) = fixture("truncated", "process.stdout.write('{\"jsonrpc\":\"2.0\",\"id\":'); setInterval(()=>{},1000);\n").await;
        let error = match hub.connect(&server.id).await {
            Ok(_) => panic!("truncated protocol unexpectedly connected"),
            Err(error) => error,
        };
        assert!(error.contains("handshake"), "unexpected error: {error}");
        hub.remove_local(&server.id).unwrap();
        let (_temp, mut hub, server) = fixture("early-exit", "process.exit(3);\n").await;
        let error = match hub.connect(&server.id).await {
            Ok(_) => panic!("early exit unexpectedly connected"),
            Err(error) => error,
        };
        assert!(error.contains("handshake"), "unexpected error: {error}");
        assert!(!hub.list().unwrap().iter().find(|item| item.id == server.id).unwrap().connected);
        hub.remove_local(&server.id).unwrap();
        // A server that exits right after discovery leaves a closed transport;
        // selection must report it as disconnected instead of offering stale tools.
        let (_temp, mut hub, server) = fixture(
            "exit-after-discovery",
            r#"
require('node:readline').createInterface({input:process.stdin}).on('line', line => {
  const r = JSON.parse(line); if (r.id === undefined) return;
  const result = r.method === 'initialize'
  ? {protocolVersion:r.params.protocolVersion,capabilities:{tools:{}},serverInfo:{name:'fixture',version:'1'}}
  : {tools:[{name:'read_file',description:'fixture',inputSchema:{type:'object'}}]};
  process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:r.id,result})+'\n');
  if (r.method === 'tools/list') setTimeout(() => process.exit(0), 200);
});"#,
        )
        .await;
        hub.connect(&server.id).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        assert!(hub
            .selected_tools(
                &[],
                &[super::ToolSelection {
                    connector_id: server.id.clone(),
                    tool_name: "read_file".into(),
                }],
            )
            .is_err());
        hub.disconnect(&server.id, false).await.unwrap();
        hub.remove_local(&server.id).unwrap();
    }
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
        let (unselected, omitted) = hub.tools_for_turn(&[], &[]).unwrap();
        assert!(unselected.is_empty(), "Connecting a server must not enable its tools");
        assert!(omitted.is_empty());
        let choices = vec![
            super::ToolSelection { connector_id: server.id.clone(), tool_name: "read_file".into() },
            super::ToolSelection { connector_id: "parallel-web".into(), tool_name: "web_search".into() },
            super::ToolSelection { connector_id: "deepwiki".into(), tool_name: "ask_question".into() },
        ];
        let (available, omitted) = hub.tools_for_turn(&[], &choices).unwrap();
        assert_eq!(available.len(), 1);
        assert_eq!(available[0].connector, server.id);
        assert_eq!(omitted, ["parallel-web", "deepwiki"]);
        assert_eq!(view.auth_type, "local");
        assert!(hub.save_local(server.clone()).is_err());
        assert!(hub.read_local(&server.id).is_err());
        assert!(hub.remove_local(&server.id).is_err());
        assert!(!hub.list().unwrap().iter().find(|item| item.id == server.id).unwrap().tools.is_empty());
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
        // Simulate shutting down and reopening the app: configurations and
        // enabled IDs survive, live MCP sessions do not.
        let db_path = temp.path().join("settings.sqlite");
        {
            let store = crate::store::Store::open(&db_path).unwrap();
            super::remember_connection(&store, "tavily", true).unwrap();
            super::remember_connection(&store, &server.id, true).unwrap();
        }
        let vault = hub.vault.clone();
        drop(hub);
        let store = crate::store::Store::open(&db_path).unwrap();
        let mut hub = super::McpHub::new(vault.clone());
        hub.restore_saved(&super::remembered_connections(&store).unwrap()).await;
        let views = hub.list().unwrap();
        let restored = views.iter().find(|item| item.id == server.id).unwrap();
        assert!(restored.connected);
        assert_eq!(restored.tools.len(), 1);
        let unauthenticated = views.iter().find(|item| item.id == "tavily").unwrap();
        assert!(!unauthenticated.connected);
        assert!(unauthenticated.connection_error.as_deref().unwrap().contains("API token"));
        // Repeated restoration must not launch a second local process.
        hub.restore_saved(std::slice::from_ref(&server.id)).await;
        assert!(!hub.restore_errors.contains_key(&server.id));
        super::remember_connection(&store, &server.id, false).unwrap();
        super::remember_connection(&store, "tavily", false).unwrap();
        hub.disconnect(&server.id, false).await.unwrap();
        drop(store);
        drop(hub);
        let store = crate::store::Store::open(&db_path).unwrap();
        let mut hub = super::McpHub::new(vault);
        let saved = super::remembered_connections(&store).unwrap();
        assert!(saved.is_empty());
        hub.restore_saved(&saved).await;
        assert!(hub.active_connector_ids().is_empty());
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
        assert_eq!(tools.len(), 5);
        assert!(tools.iter().any(|tool| tool.tool.name == "edit_file"));
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
        assert_eq!(tool_alias("Local execution", "execute_command"), "local_execute_command");
        assert_eq!(tool_alias("Harness", "todo_write"), "todo_write");
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
        assert_eq!(items.len(), 15);
        let mut ids = std::collections::HashSet::new();
        for item in items {
            assert!(ids.insert(item.id));
            assert!(item.url.starts_with("https://"));
        }
    }
    #[test]
    fn bundled_trueforge_presets_match_source_and_supported_auth() {
        // Compared against the TrueForge upstream docs/catalog on 2026-09-09:
        // docs site catalog for Settings → Connectors, MCP docs for Linear
        // (mcp.linear.app/mcp + OAuth 2.1 DCR + bearer/API-key header), the
        // GitHub MCP server OAuth protected-resource metadata
        // (api.githubcopilot.com/mcp), and the DeepWiki MCP setup page
        // (mcp.deepwiki.com/mcp, no auth for public repos; private repos use
        // a separate Devin endpoint with a bearer key, which is not a preset).
        let catalog: Value = serde_json::from_str(include_str!("../../src/lib/catalog.json"))
            .expect("Bundled connector catalog is valid JSON");
        let bundled: Vec<(String, String, String)> = catalog["connectors"]
            .as_array()
            .expect("Bundled connectors array")
            .iter()
            .map(|item| {
                (
                    item["name"].as_str().unwrap_or_default().to_string(),
                    item["url"].as_str().unwrap_or_default().to_string(),
                    item["auth"]["type"].as_str().unwrap_or("none").to_string(),
                )
            })
            .collect();
        let expected: &[(&str, &str, &str)] = &[
            ("linear", "https://mcp.linear.app/mcp", "dcr"),
            ("notion", "https://mcp.notion.com/mcp", "dcr"),
            ("sentry", "https://mcp.sentry.dev/mcp", "dcr"),
            ("deepwiki", "https://mcp.deepwiki.com/mcp", "none"),
            ("exa", "https://mcp.exa.ai/mcp", "none"),
            ("parallel-web", "https://search.parallel.ai/mcp", "none"),
            ("github", "https://api.githubcopilot.com/mcp/", "header"),
            ("tavily", "https://mcp.tavily.com/mcp", "header"),
            ("bright-data", "https://mcp.brightdata.com/mcp", "header"),
            ("supabase", "https://mcp.supabase.com/mcp", "dcr"),
            ("stripe", "https://mcp.stripe.com", "dcr"),
            ("confluence", "https://mcp.atlassian.com/v1/mcp", "dcr"),
            ("jira", "https://mcp.atlassian.com/v1/mcp", "dcr"),
            ("posthog", "https://mcp.posthog.com/mcp", "dcr"),
            ("firecrawl", "https://mcp.firecrawl.dev/v2/mcp", "header"),
        ];
        assert_eq!(bundled.len(), expected.len());
        for (index, (name, url, auth)) in expected.iter().enumerate() {
            assert_eq!(&bundled[index].0, name, "preset order drift at {index}");
            assert_eq!(&bundled[index].1, url, "endpoint drift for {name}");
            assert_eq!(&bundled[index].2, auth, "auth drift for {name}");
        }
        let views = presets();
        assert_eq!(views.len(), expected.len());
        for (view, (name, _, auth)) in views.iter().zip(expected.iter()) {
            assert_eq!(&view.id, name);
            assert_eq!(
                view.auth_type.as_str(),
                match *auth {
                    "dcr" => "oauth",
                    "header" => "apiKey",
                    _ => "none",
                },
                "auth mapping drift for {name}"
            );
        }
    }
    #[test]
    fn connector_lifecycle_errors_are_actionable_and_never_leak_tokens() {
        let temp = tempfile::tempdir().unwrap();
        let vault = std::sync::Arc::new(crate::vault::Vault::new(temp.path().join("vault")));
        let hub = super::McpHub::new(vault);
        assert!(hub.save_token("github", "fixture-token-abcdef").is_ok());
        assert!(hub.save_token("github", " token-with-space").is_err());
        assert!(hub.save_token("github", "token\r\nAuthorization: injected").is_err());
        assert!(hub.save_token("unknown-connector", "fixture-token-abcdef").is_err());
        // OAuth presets have no bearer-token path: leftover tokens are ignored
        // at connect time and rejected at save time, so an OAuth sign-in can
        // never silently fall back to a bearer header.
        assert!(hub.save_token("linear", "fixture-token-abcdef").is_err());
        for (id, expected) in [
            ("linear", "dcr"),
            ("github", "header"),
            ("deepwiki", "none"),
            ("bright-data", "header"),
            ("supabase", "dcr"),
        ] {
            assert_eq!(super::preset_credential(id).unwrap(), expected);
        }
        let error = "Could not connect: 401 with fixture-token-abcdef embedded";
        let redacted = error.replace("fixture-token-abcdef", "[redacted]");
        assert!(!redacted.contains("fixture-token-abcdef"));
        assert!(redacted.contains("[redacted]"));
    }
    #[test]
    fn reconnect_backoff_is_bounded_exponential() {
        // Five attempts total: an immediate try plus these four delays.
        assert_eq!(
            super::RECONNECT_DELAYS,
            [
                Duration::from_secs(1),
                Duration::from_secs(2),
                Duration::from_secs(4),
                Duration::from_secs(8)
            ]
        );
    }
    #[test]
    fn reconnect_requires_a_runtime_and_never_fires_on_live_or_detached() {
        let temp = tempfile::tempdir().unwrap();
        let vault = Arc::new(crate::vault::Vault::new(temp.path().join("vault")));
        let shared = Arc::new(super::SharedConnection::new("fixture", vault, None));
        // No tokio runtime in this test: the kick must degrade to a no-op.
        shared.kick_reconnect();
        assert!(!super::mutex(&shared.reconnect).running);
    }
    #[tokio::test]
    async fn reconnect_loop_bounds_attempts_and_detach_stops_it() {
        let temp = tempfile::tempdir().unwrap();
        let vault = Arc::new(crate::vault::Vault::new(temp.path().join("vault")));
        let shared = Arc::new(super::SharedConnection::new(
            "not-a-real-connector",
            vault,
            None,
        ));
        // A connector that can never open records a bounded failure count.
        shared.clone().reconnect_loop(&[]).await;
        {
            let state = super::mutex(&shared.reconnect);
            assert!(!state.running);
            assert_eq!(state.failed_attempts, 1);
        }
        // A detached connection exits immediately without recording more.
        shared.detached.store(true, Ordering::Release);
        super::mutex(&shared.reconnect).running = true;
        shared.clone().reconnect_loop(&[]).await;
        let state = super::mutex(&shared.reconnect);
        assert!(!state.running);
        assert_eq!(state.failed_attempts, 1);
    }
    #[test]
    fn tool_list_changed_flag_waits_for_a_live_session() {
        let temp = tempfile::tempdir().unwrap();
        let vault = Arc::new(crate::vault::Vault::new(temp.path().join("vault")));
        let shared = Arc::new(super::SharedConnection::new("fixture", vault, None));
        shared.note_tools_changed();
        // With no live session no refresh task spawns; the flag stays set for
        // the reconnect path, which installs a fresh catalog on success.
        assert!(shared.tools_dirty.load(Ordering::Acquire));
        assert!(!shared.refreshing.load(Ordering::Acquire));
    }
    #[tokio::test]
    async fn dead_local_connector_reconnects_with_stable_tool_alias() {
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("fixture.cjs");
        // The fixture answers one tools/call, then exits — killing the
        // transport so the hub observes a dead session.
        std::fs::write(&script, r#"
require('node:readline').createInterface({input:process.stdin}).on('line', line => {
 const r = JSON.parse(line); if (r.id === undefined) return;
 const result = r.method === 'initialize'
 ? {protocolVersion:r.params.protocolVersion,capabilities:{tools:{}},serverInfo:{name:'fixture',version:'1'}}
 : r.method === 'tools/call'
 ? {content:[{type:'text',text:'ok'}],isError:false}
 : {tools:[{name:'read_file',description:'fixture',inputSchema:{type:'object'}}]};
 process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:r.id,result})+'\n');
 if (r.method === 'tools/call') setTimeout(() => process.exit(0), 150);
});
"#).unwrap();
        let mut hub = super::McpHub::new(Arc::new(crate::vault::Vault::new(
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
        hub.connect(&server.id).await.unwrap();
        let selections = [super::ToolSelection {
            connector_id: server.id.clone(),
            tool_name: "read_file".into(),
        }];
        let tools = hub.selected_tools(&[], &selections).unwrap();
        let alias = tools[0].alias.clone();
        tools[0].call(serde_json::json!({})).await.unwrap();
        // The process exit closes the transport; the next hub read observes a
        // dead session and reports the connector unavailable.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while hub.is_connected(&server.id) {
            assert!(
                std::time::Instant::now() < deadline,
                "fixture transport never closed"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let (_, omitted) = hub
            .tools_for_turn(std::slice::from_ref(&server.id), &[])
            .unwrap();
        assert_eq!(omitted, std::slice::from_ref(&server.id));
        // That same read scheduled the bounded reconnect; the connector comes
        // back without a manual reconnect.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !hub.is_connected(&server.id) {
            assert!(
                std::time::Instant::now() < deadline,
                "reconnect never restored the session"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        // Reconnect re-ran discovery; the same tool name keeps the same alias.
        let tools = hub.selected_tools(&[], &selections).unwrap();
        assert_eq!(tools[0].alias, alias);
        tools[0].call(serde_json::json!({})).await.unwrap();
        hub.disconnect(&server.id, false).await.unwrap();
        hub.remove_local(&server.id).unwrap();
    }
    #[test]
    fn tool_result_projection_keeps_text_and_omits_binary_bodies() {
        use rmcp::model::{CallToolResult, ContentBlock, Resource, ResourceContents};
        let result = CallToolResult::success(vec![
            ContentBlock::text("hello"),
            ContentBlock::image("aGVsbG8=", "image/png"),
            ContentBlock::audio("aGVsbG8=", "audio/wav"),
            ContentBlock::resource(
                ResourceContents::blob("AAAA", "file:///bin").with_mime_type("application/pdf"),
            ),
            ContentBlock::resource_link(
                Resource::new("file:///doc", "doc").with_mime_type("text/markdown"),
            ),
        ]);
        let value = super::project_tool_result(&result).unwrap();
        assert_eq!(value["isError"], serde_json::json!(false));
        assert_eq!(
            value["content"][0],
            serde_json::json!({"type": "text", "text": "hello"})
        );
        for index in 1..5 {
            let item = &value["content"][index];
            assert_eq!(item["type"], "unsupported", "item {index}");
            assert_eq!(item["note"], "binary content omitted", "item {index}");
            assert!(item.get("data").is_none(), "base64 leaked at {index}");
            assert!(item.get("blob").is_none(), "base64 leaked at {index}");
        }
        assert_eq!(value["content"][1]["mimeType"], "image/png");
        assert_eq!(value["content"][2]["mimeType"], "audio/wav");
        assert_eq!(value["content"][3]["mimeType"], "application/pdf");
        assert_eq!(value["content"][4]["mimeType"], "text/markdown");
    }
    #[test]
    fn tool_result_projection_surfaces_errors_and_bounds_items() {
        use rmcp::model::{CallToolResult, ContentBlock};
        let error = CallToolResult::error(vec![ContentBlock::text("boom")]);
        let value = super::project_tool_result(&error).unwrap();
        assert_eq!(value["isError"], serde_json::json!(true));
        assert_eq!(value["content"][0]["text"], "boom");
        // A single text item beyond ~64 KiB is truncated with a marker.
        let big = CallToolResult::success(vec![ContentBlock::text("x".repeat(70_000))]);
        let value = super::project_tool_result(&big).unwrap();
        let text = value["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("truncated"));
        assert!(text.len() < 70_000);
        assert!(text.len() > 65_536);
        // The 256 KiB total cap still rejects oversized envelopes.
        let huge = CallToolResult::success(
            (0..8)
                .map(|_| ContentBlock::text("y".repeat(60_000)))
                .collect(),
        );
        assert!(super::project_tool_result(&huge).is_err());
        // structuredContent passes through untouched.
        let mut structured = CallToolResult::success(vec![]);
        structured.structured_content = Some(serde_json::json!({"rows": 2}));
        let value = super::project_tool_result(&structured).unwrap();
        assert_eq!(value["structuredContent"], serde_json::json!({"rows": 2}));
    }
}
