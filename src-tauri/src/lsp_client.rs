//! Bounded stdio LSP client. Configured executables are user-owned; protocol
//! requests never grant servers permission to apply edits or run commands.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};

pub const KEY: &str = "lsp.configuration";
const MAX_FRAME: usize = 8 * 1024 * 1024;
fn yes() -> bool {
    true
}
fn timeout() -> u64 {
    30
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
#[derive(Default)]
pub struct Config {
    pub enabled: bool,
    pub always_ask: bool,
    pub servers: Vec<ServerConfig>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServerConfig {
    pub id: String,
    pub language_id: String,
    pub command: String,
    #[serde(default)]
    pub allowed_commands: Vec<String>,
    #[serde(default = "yes")]
    pub enabled: bool,
    #[serde(default)]
    pub arguments: Vec<String>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub initialization_options: Value,
    #[serde(default)]
    pub settings: Value,
    #[serde(default = "timeout")]
    pub timeout_seconds: u64,
}
impl Config {
    pub fn validate(&self) -> Result<(), String> {
        if self.servers.len() > 8 {
            return Err("Configure at most8 language servers.".into());
        }
        let mut ids = std::collections::HashSet::new();
        for server in &self.servers {
            if server.id.is_empty()
                || server.id.len() > 64
                || !server
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                || !ids.insert(&server.id)
            {
                return Err("Language server IDs must be unique,1–64 letters, numbers, dots, dashes or underscores.".into());
            }
            if server.language_id.is_empty()
                || server.language_id.len() > 64
                || server.language_id.chars().any(char::is_control)
            {
                return Err("A language ID must contain1–64 printable characters.".into());
            }
            if !std::path::Path::new(&server.command).is_absolute()
                || server.command.len() > 4096
                || server.command.contains('\0')
            {
                return Err("Choose an absolute language server executable path.".into());
            }
            if !(1..=120).contains(&server.timeout_seconds)
                || server.arguments.len() > 64
                || server
                    .arguments
                    .iter()
                    .any(|s| s.len() > 8192 || s.contains('\0'))
            {
                return Err("Use a1–120second timeout and at most64 bounded arguments.".into());
            }
            if server.environment.len() > 32
                || server.environment.iter().any(|(k, v)| {
                    k.is_empty()
                        || k.len() > 256
                        || k.contains(['=', '\0'])
                        || v.len() > 8192
                        || v.contains('\0')
                })
            {
                return Err(
                    "Language server environment must contain at most32 bounded string values."
                        .into(),
                );
            }
            if server.allowed_commands.len() > 32
                || server
                    .allowed_commands
                    .iter()
                    .any(|v| v.is_empty() || v.len() > 256)
            {
                return Err("Configure at most 32 bounded LSP command names.".into());
            }
            if server.initialization_options.to_string().len() > 65536
                || server.settings.to_string().len() > 65536
            {
                return Err("Language server initialization/options exceed64KiB.".into());
            }
        }
        Ok(())
    }
    pub fn server(&self, id: &str) -> Result<&ServerConfig, String> {
        if !self.enabled {
            return Err(
                "Language server tools are disabled. Enable and configure a server in Settings."
                    .into(),
            );
        }
        let server = self
            .servers
            .iter()
            .find(|s| s.enabled && s.id == id)
            .ok_or("That language server is not configured or enabled.")?;
        if !crate::execution::executable(&server.command) {
            return Err(format!(
                "Language server '{}' executable is unavailable.",
                server.id
            ));
        }
        Ok(server)
    }
}
pub fn fingerprint(server: &ServerConfig) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(server).unwrap_or_default())
    )
}
pub fn load(store: &crate::store::Store) -> Result<Config, String> {
    let config: Config = store.setting(KEY)?;
    config.validate()?;
    Ok(config)
}

#[derive(Clone, Debug)]
pub struct Document {
    pub uri: String,
    pub sha256: String,
    pub version: i32,
}
pub struct Reply {
    pub staged_edits: Vec<Value>,
    pub result: Value,
    pub documents: Vec<Document>,
    pub capabilities: Value,
    pub diagnostics: Option<Value>,
}
struct Session {
    _permit: tokio::sync::OwnedSemaphorePermit,
    child: Option<crate::execution::ExecutionChild>,
    stdin: tokio::process::ChildStdin,
    stdout: BufReader<tokio::process::ChildStdout>,
    stderr_task: tokio::task::JoinHandle<()>,
    stderr: Arc<Mutex<Vec<u8>>>,
    next_id: u64,
    staged_edits: Vec<Value>,
    command_active: bool,
    capabilities: Value,
    documents: BTreeMap<String, Document>,
    published: HashMap<String, Value>,
    root: String,
    settings: Value,
    last_used: Instant,
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stderr_task.abort();
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    let _ = tokio::time::timeout(Duration::from_secs(3), child.wait()).await;
                });
            }
        }
    }
}
impl Session {
    fn spawn(
        root: &str,
        server: &ServerConfig,
        permit: tokio::sync::OwnedSemaphorePermit,
    ) -> Result<Self, String> {
        let mut command = tokio::process::Command::new(&server.command);
        command
            .args(&server.arguments)
            .envs(&server.environment)
            .current_dir(root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        let mut child = crate::execution::managed_spawn(command)
            .map_err(|e| format!("Cannot start language server: {e}"))?;
        let stdin = child
            .stdin()
            .take()
            .ok_or("Language server has no stdin.")?;
        let stdout = BufReader::new(
            child
                .stdout()
                .take()
                .ok_or("Language server has no stdout.")?,
        );
        let mut reader = child
            .stderr()
            .take()
            .ok_or("Language server has no stderr.")?;
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let captured = stderr.clone();
        let stderr_task = tokio::spawn(async move {
            let mut bytes = vec![0; 8192];
            loop {
                match reader.read(&mut bytes).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Ok(mut retained) = captured.lock() {
                            let keep = n.min(65536usize.saturating_sub(retained.len()));
                            retained.extend_from_slice(&bytes[..keep]);
                        }
                    }
                }
            }
        });
        Ok(Self {
            _permit: permit,
            child: Some(child),
            stdin,
            stdout,
            stderr_task,
            stderr,
            next_id: 1,
            staged_edits: Vec::new(),
            command_active: false,
            capabilities: Value::Null,
            documents: BTreeMap::new(),
            published: HashMap::new(),
            root: root.into(),
            settings: server.settings.clone(),
            last_used: Instant::now(),
        })
    }
    async fn shutdown(mut self) {
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            self.request("shutdown", Value::Null),
        )
        .await;
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            self.send(json!({"jsonrpc":"2.0","method":"exit"})),
        )
        .await;
        if let Some(child) = self.child.as_mut() {
            let _ = tokio::time::timeout(Duration::from_secs(1), child.wait()).await;
        }
        // Drop still kills the process group/job, including lingering children.
    }
    async fn send(&mut self, message: Value) -> Result<(), String> {
        let bytes = serde_json::to_vec(&message).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_FRAME {
            return Err("LSP request exceeds8MiB.".into());
        }
        self.stdin
            .write_all(format!("Content-Length: {}\r\n\r\n", bytes.len()).as_bytes())
            .await
            .map_err(|e| e.to_string())?;
        self.stdin
            .write_all(&bytes)
            .await
            .map_err(|e| e.to_string())?;
        self.stdin.flush().await.map_err(|e| e.to_string())
    }
    fn stderr_excerpt(&self) -> String {
        self.stderr
            .lock()
            .map(|s| String::from_utf8_lossy(&s).chars().take(1024).collect())
            .unwrap_or_default()
    }
    async fn receive(&mut self) -> Result<Value, String> {
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            if header.len() >= 8192 {
                return Err("LSP headers exceed8KiB.".into());
            }
            header.push(self.stdout.read_u8().await.map_err(|e|format!("Language server closed or malformed its output: {e}. Untrusted server stderr: {}",self.stderr_excerpt()))?);
        }
        if !header.is_ascii() {
            return Err("LSP headers must be ASCII.".into());
        }
        let header = std::str::from_utf8(&header).map_err(|e| e.to_string())?;
        let mut length = None;
        for line in header.split("\r\n").filter(|line| !line.is_empty()) {
            let (name, value) = line.split_once(':').ok_or("Malformed LSP header.")?;
            if name.eq_ignore_ascii_case("Content-Length") {
                if length.is_some() {
                    return Err("Duplicate LSP Content-Length.".into());
                }
                length = Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .map_err(|_| "Invalid LSP Content-Length.")?,
                );
            } else if name.eq_ignore_ascii_case("Content-Type") {
                let value = value.to_ascii_lowercase();
                let charset = value
                    .split(';')
                    .map(str::trim)
                    .find_map(|part| part.strip_prefix("charset="))
                    .map(|s| s.trim_matches('"'));
                if charset.is_some_and(|s| !matches!(s, "utf-8" | "utf8")) {
                    return Err("Only UTF-8 LSP frames are supported.".into());
                }
            }
        }
        let length = length
            .filter(|n| *n > 0 && *n <= MAX_FRAME)
            .ok_or("Missing or oversized LSP Content-Length.")?;
        let mut bytes = vec![0; length];
        self.stdout
            .read_exact(&mut bytes)
            .await
            .map_err(|e| e.to_string())?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("Invalid LSP JSON: {e}"))?;
        if !value.is_object() || value["jsonrpc"] != "2.0" {
            return Err("Language server sent an invalid JSON-RPC2.0 message.".into());
        }
        Ok(value)
    }
    async fn handle_server(&mut self, message: &Value) -> Result<(), String> {
        let method = message["method"].as_str().unwrap_or("");
        if let Some(id) = message.get("id") {
            if !(id.is_string() || id.is_number()) {
                return Err("Invalid language server request ID.".into());
            }
            let result = match method {
                "workspace/workspaceFolders" => Some(json!([{"uri":self.root,"name":"workspace"}])),
                "workspace/configuration" => {
                    let items = message["params"]["items"]
                        .as_array()
                        .ok_or("Invalid workspace/configuration request.")?;
                    if items.len() > 64 {
                        return Err("Too many LSP configuration sections.".into());
                    }
                    Some(Value::Array(
                        items
                            .iter()
                            .map(|item| {
                                let mut value = &self.settings;
                                if let Some(section) = item["section"].as_str() {
                                    for key in section.split('.') {
                                        value = &value[key];
                                    }
                                }
                                value.clone()
                            })
                            .collect(),
                    ))
                }
                "window/workDoneProgress/create" => Some(Value::Null),
                "workspace/applyEdit" => {
                    if self.command_active {
                        if self.staged_edits.len() >= 16 {
                            return Err("Command staged too many edits.".into());
                        }
                        let edit = message["params"]["edit"].clone();
                        if !edit.is_object() {
                            return Err("Command returned malformed WorkspaceEdit.".into());
                        }
                        self.staged_edits.push(edit);
                        Some(
                            json!({"applied":false,"failureReason":"Edit captured as a proposal; explicit review and lsp_apply_edit are required."}),
                        )
                    } else {
                        Some(
                            json!({"applied":false,"failureReason":"Unsolicited edits are disabled."}),
                        )
                    }
                }
                "window/showMessageRequest" => Some(Value::Null),
                _ => None,
            };
            return self.send(match result{Some(result)=>json!({"jsonrpc":"2.0","id":id,"result":result}),None=>json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"Client method not supported; no command or edit was executed."}})}).await;
        }
        if method == "textDocument/publishDiagnostics" {
            let params = &message["params"];
            let uri = params["uri"]
                .as_str()
                .ok_or("Diagnostics missing document URI.")?;
            if self.documents.contains_key(uri) {
                self.published.insert(uri.into(), params.clone());
                if self
                    .published
                    .values()
                    .map(|v| v.to_string().len())
                    .sum::<usize>()
                    > MAX_FRAME
                {
                    return Err("Retained LSP diagnostics exceed8MiB.".into());
                }
            }
        }
        Ok(())
    }
    async fn request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or("LSP ID space exhausted.")?;
        self.send(json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
            .await?;
        for _ in 0..512 {
            let message = self.receive().await?;
            if message.get("method").is_some() {
                self.handle_server(&message).await?;
                continue;
            }
            if message.get("id") != Some(&json!(id)) {
                return Err("Unexpected language server response ID; session stopped to avoid stale-response confusion.".into());
            }
            if let Some(error) = message.get("error") {
                let code = error["code"].as_i64().unwrap_or(-32603);
                let message = error["message"]
                    .as_str()
                    .unwrap_or("Language server request failed.")
                    .chars()
                    .take(1024)
                    .collect::<String>();
                return Err(format!("LSP error {code}: {message}"));
            }
            return message
                .get("result")
                .cloned()
                .ok_or("LSP response has neither result nor error.".into());
        }
        Err("LSP notification/request budget exceeded512 messages.".into())
    }
    async fn initialize(&mut self, server: &ServerConfig) -> Result<(), String> {
        let result=self.request("initialize",json!({"processId":std::process::id(),"rootUri":self.root,
            "clientInfo":{"name":"LocalLM","version":env!("CARGO_PKG_VERSION")},
            "workspaceFolders":[{"uri":self.root,"name":"workspace"}],"initializationOptions":server.initialization_options,
            "capabilities":{"general":{"positionEncodings":["utf-16"]},"workspace":{"workspaceFolders":true,"configuration":true,"applyEdit":true,"workspaceEdit":{"documentChanges":true,"resourceOperations":["create","rename","delete"],"failureHandling":"transactional"}},
                "textDocument":{"synchronization":{"dynamicRegistration":false,"didSave":false},"documentSymbol":{"hierarchicalDocumentSymbolSupport":true},"references":{"dynamicRegistration":false},"definition":{"dynamicRegistration":false,"linkSupport":true},"implementation":{"dynamicRegistration":false,"linkSupport":true},"hover":{"dynamicRegistration":false,"contentFormat":["markdown","plaintext"]},
                    "codeAction":{"dynamicRegistration":false,"codeActionLiteralSupport":{"codeActionKind":{"valueSet":["","quickfix","refactor","refactor.extract","refactor.inline","refactor.rewrite","source","source.organizeImports","source.fixAll"]}},"dataSupport":true,"disabledSupport":true,"isPreferredSupport":true,"resolveSupport":{"properties":["edit"]},"honorsChangeAnnotations":true},"rename":{"dynamicRegistration":false,"prepareSupport":true,"honorsChangeAnnotations":true},"publishDiagnostics":{"versionSupport":true},"diagnostic":{"dynamicRegistration":false,"relatedDocumentSupport":false}}}})).await?;
        let capabilities = result
            .get("capabilities")
            .filter(|v| v.is_object())
            .ok_or("LSP initialize response is missing capabilities.")?;
        if capabilities["positionEncoding"]
            .as_str()
            .is_some_and(|s| s != "utf-16")
        {
            return Err(
                "Language server selected an unsupported position encoding; UTF-16 is required."
                    .into(),
            );
        }
        self.capabilities = capabilities.clone();
        self.send(json!({"jsonrpc":"2.0","method":"initialized","params":{}}))
            .await?;
        if !server.settings.is_null() {
            self.send(json!({"jsonrpc":"2.0","method":"workspace/didChangeConfiguration","params":{"settings":server.settings}})).await?;
        }
        Ok(())
    }
    async fn open_document(
        &mut self,
        server: &ServerConfig,
        uri: &str,
        text: &str,
        sha: &str,
        force_refresh: bool,
    ) -> Result<(), String> {
        let sync = &self.capabilities["textDocumentSync"];
        let open_close =
            sync["openClose"] == true || sync.as_u64().is_some_and(|kind| matches!(kind, 1 | 2));
        if !open_close {
            return Err("Language server does not support opening current document snapshots; no result is trusted as current.".into());
        }
        let previous = self.documents.get(uri).cloned();
        if !force_refresh && previous.as_ref().is_some_and(|d| d.sha256 == sha) {
            return Ok(());
        }
        let version = previous
            .as_ref()
            .map(|d| {
                d.version
                    .checked_add(1)
                    .ok_or("LSP document version exhausted.")
            })
            .transpose()?
            .unwrap_or(1);
        if previous.is_some() {
            self.send(json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":uri}}})).await?;
        }
        if self.documents.len() >= 16 && !self.documents.contains_key(uri) {
            let oldest = self.documents.keys().next().cloned().unwrap();
            self.send(json!({"jsonrpc":"2.0","method":"textDocument/didClose","params":{"textDocument":{"uri":oldest}}})).await?;
            self.documents.remove(&oldest);
            self.published.remove(&oldest);
        }
        self.documents.insert(
            uri.into(),
            Document {
                uri: uri.into(),
                sha256: sha.into(),
                version,
            },
        );
        self.published.remove(uri);
        self.send(json!({"jsonrpc":"2.0","method":"textDocument/didOpen","params":{"textDocument":{"uri":uri,"languageId":server.language_id,"version":version,"text":text}}})).await
    }
    async fn diagnostic(&mut self, uri: &str) -> Result<Value, String> {
        if self
            .capabilities
            .get("diagnosticProvider")
            .is_some_and(|v| v.is_object())
        {
            let result = self
                .request(
                    "textDocument/diagnostic",
                    json!({"textDocument":{"uri":uri}}),
                )
                .await?;
            if result["kind"] != "full" || !result["items"].is_array() {
                return Err("Language server did not return a complete diagnostic report.".into());
            }
            return Ok(result);
        }
        for _ in 0..512 {
            if let Some(params) = self.published.get(uri) {
                let expected = self.documents[uri].version;
                if params["version"]
                    .as_i64()
                    .is_some_and(|v| v != i64::from(expected))
                {
                    self.published.remove(uri);
                } else if params["diagnostics"].is_array() {
                    return Ok(
                        json!({"kind":"full","items":params["diagnostics"],"source":"publishDiagnostics","version":params["version"],"versionVerified":params["version"].as_i64()==Some(i64::from(expected))}),
                    );
                } else {
                    return Err("Malformed publishDiagnostics report.".into());
                }
            }
            let message = self.receive().await?;
            self.handle_server(&message).await?;
        }
        Err("No current diagnostic report within the notification bound; no clean result is claimed.".into())
    }
}

type Slot = Arc<tokio::sync::Mutex<Option<Session>>>;
pub struct Manager {
    slots: Arc<Mutex<BTreeMap<String, Slot>>>,
    cleanup_started: AtomicBool,
    revision: tokio::sync::watch::Sender<u64>,
    permits: Arc<tokio::sync::Semaphore>,
    /// Incremented whenever the registry is retired. A request that finishes
    /// after a retirement must not put its session back into the pool: the
    /// slot it was using is no longer reachable, so the restored session would
    /// run on with no handle while holding a semaphore permit.
    generation: AtomicU64,
}
impl Default for Manager {
    fn default() -> Self {
        Self {
            slots: Arc::new(Mutex::new(BTreeMap::new())),
            cleanup_started: AtomicBool::new(false),
            revision: tokio::sync::watch::channel(0).0,
            permits: Arc::new(tokio::sync::Semaphore::new(4)),
            generation: AtomicU64::new(0),
        }
    }
}
impl Manager {
    /// Retire every session this manager owns.
    ///
    /// A slot that is busy (a request is in flight) cannot be taken
    /// synchronously. Its session is still live, so the slot is kept and
    /// awaited: the session is taken and shut down once the request releases
    /// it. Dropping the map entry alone would leave the language server
    /// process running without a handle and leak its semaphore permit.
    pub async fn shutdown(&self) {
        self.revision.send_modify(|n| *n = n.wrapping_add(1));
        self.generation.fetch_add(1, Ordering::SeqCst);
        let mut retired = Vec::new();
        let mut busy = Vec::new();
        if let Ok(mut slots) = self.slots.lock() {
            for slot in slots.values() {
                if let Ok(mut session) = slot.try_lock() {
                    if let Some(session) = session.take() {
                        retired.push(session);
                    }
                } else {
                    busy.push(slot.clone());
                }
            }
            slots.clear();
        }
        futures_util::future::join_all(retired.into_iter().map(Session::shutdown)).await;
        for slot in busy {
            // The revision change above cancels the in-flight request, which
            // releases the slot. The bound keeps a wedged request from
            // blocking shutdown forever.
            let mut session = match tokio::time::timeout(Duration::from_secs(5), slot.lock()).await
            {
                Ok(session) => session,
                Err(_) => continue,
            };
            if let Some(session) = session.take() {
                session.shutdown().await;
            }
        }
    }
    pub fn revision(&self) -> tokio::sync::watch::Receiver<u64> {
        self.revision.subscribe()
    }
    pub fn clear(&self) {
        self.revision.send_modify(|n| *n = n.wrapping_add(1));
        if let Ok(mut slots) = self.slots.lock() {
            slots.clear();
        }
    }
    /// Return a finished session to the slot it was taken from.
    ///
    /// A registry that was retired while the request was in flight no longer
    /// owns that slot. Restoring the session there would leave a live
    /// language server process without a handle while it keeps holding a
    /// semaphore permit, so the session is dropped instead: dropping it kills
    /// the process and returns the permit.
    fn restore(
        &self,
        guard: &mut tokio::sync::MutexGuard<'_, Option<Session>>,
        session: Session,
        generation: u64,
    ) {
        if self.generation.load(Ordering::SeqCst) != generation {
            drop(session);
            return;
        }
        **guard = Some(session);
    }

    fn slot(&self, key: String) -> Result<Slot, String> {
        if !self.cleanup_started.swap(true, Ordering::SeqCst) {
            let weak = Arc::downgrade(&self.slots);
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                    let Some(slots) = weak.upgrade() else {
                        break;
                    };
                    if let Ok(mut slots) = slots.lock() {
                        slots.retain(|_, slot| {
                            if let Ok(mut session) = slot.try_lock() {
                                if session.as_ref().is_some_and(|s| {
                                    s.last_used.elapsed() > Duration::from_secs(120)
                                }) {
                                    session.take();
                                    return false;
                                }
                            }
                            true
                        });
                    };
                }
            });
        }
        let mut slots = self
            .slots
            .lock()
            .map_err(|_| "Language server registry unavailable.")?;
        if let Some(slot) = slots.get(&key) {
            return Ok(slot.clone());
        }
        if slots.len() >= 4 {
            let removable = slots
                .iter()
                .find_map(|(key, slot)| slot.try_lock().ok().map(|_| key.clone()));
            if let Some(key) = removable {
                slots.remove(&key);
            } else {
                return Err(
                    "All4 language server sessions are busy; retry when one finishes.".into(),
                );
            }
        }
        let slot = Arc::new(tokio::sync::Mutex::new(None));
        slots.insert(key, slot.clone());
        Ok(slot)
    }
    pub async fn request(
        &self,
        owner: &str,
        root: &str,
        server: &ServerConfig,
        document: Option<(&str, &str, &str)>,
        method: &str,
        params: Value,
    ) -> Result<Reply, String> {
        self.request_guarded(
            owner,
            root,
            server,
            document,
            method,
            params,
            self.revision(),
        )
        .await
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn request_guarded(
        &self,
        owner: &str,
        root: &str,
        server: &ServerConfig,
        document: Option<(&str, &str, &str)>,
        method: &str,
        params: Value,
        mut revision: tokio::sync::watch::Receiver<u64>,
    ) -> Result<Reply, String> {
        if revision.has_changed().unwrap_or(true) {
            return Err(
                "Language server settings changed before dispatch; refresh the call.".into(),
            );
        }
        let canonical = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
        let root_uri = reqwest::Url::from_directory_path(&canonical)
            .map_err(|_| "Invalid language server workspace URI.")?
            .to_string();
        let key = format!("{owner}:{}:{}", root_uri, fingerprint(server));
        let generation = self.generation.load(Ordering::SeqCst);
        let slot = self.slot(key)?;
        let operation = async {
            let mut guard = slot.lock().await;
            // Take ownership across awaits: cancellation/error drops and kills
            // the session rather than leaving a pending response in the pool.
            let mut session = match guard.take() {
                Some(session) => session,
                None => {
                    let permit = self
                        .permits
                        .clone()
                        .acquire_owned()
                        .await
                        .map_err(|_| "Language server slots closed.")?;
                    let mut s = Session::spawn(root, server, permit)?;
                    s.root = root_uri;
                    s.initialize(server).await?;
                    s
                }
            };
            if let Some((uri, text, sha)) = document {
                let refresh = method == "__diagnostics"
                    && !session
                        .capabilities
                        .get("diagnosticProvider")
                        .is_some_and(Value::is_object);
                session
                    .open_document(server, uri, text, sha, refresh)
                    .await?;
            }
            let result = if method == "__diagnostics" {
                session
                    .diagnostic(document.ok_or("Diagnostics need a document.")?.0)
                    .await?
            } else if method == "__initialize" {
                json!({"ready":true})
            } else {
                if method == "workspace/executeCommand" {
                    let name = params["command"].as_str().ok_or("Missing LSP command.")?;
                    if !server.allowed_commands.iter().any(|v| v == name)
                        || !session.capabilities["executeCommandProvider"]["commands"]
                            .as_array()
                            .is_some_and(|a| a.iter().any(|v| v == name))
                    {
                        return Err(
                            "LSP command is not both user-allowed and server-advertised.".into(),
                        );
                    }
                    session.staged_edits.clear();
                    session.command_active = true;
                }
                let capability = match method {
                    "workspace/executeCommand" => "executeCommandProvider",
                    "textDocument/prepareCallHierarchy"
                    | "callHierarchy/incomingCalls"
                    | "callHierarchy/outgoingCalls" => "callHierarchyProvider",
                    "textDocument/definition" => "definitionProvider",
                    "textDocument/hover" => "hoverProvider",
                    "textDocument/implementation" => "implementationProvider",
                    "textDocument/codeAction" | "codeAction/resolve" => "codeActionProvider",
                    "textDocument/documentSymbol" => "documentSymbolProvider",
                    "textDocument/references" => "referencesProvider",
                    "textDocument/rename" | "textDocument/prepareRename" => "renameProvider",
                    _ => return Err("Unsupported native LSP method.".into()),
                };
                if !session
                    .capabilities
                    .get(capability)
                    .is_some_and(|v| v == true || v.is_object())
                {
                    return Err(format!("Language server does not advertise {capability}."));
                }
                if method == "textDocument/rename"
                    && session.capabilities["renameProvider"]["prepareProvider"] == true
                {
                    let prepared=session.request("textDocument/prepareRename",json!({"textDocument":params["textDocument"],"position":params["position"]})).await?;
                    if prepared.is_null() {
                        return Err("Language server says this symbol cannot be renamed.".into());
                    }
                }
                session.request(method, params).await?
            };
            session.command_active = false;
            let reply = Reply {
                staged_edits: std::mem::take(&mut session.staged_edits),
                result,
                documents: session.documents.values().cloned().collect(),
                capabilities: session.capabilities.clone(),
                diagnostics: document.and_then(|(uri, _, _)| session.published.get(uri).cloned()),
            };
            session.last_used = Instant::now();
            self.restore(&mut guard, session, generation);
            Ok(reply)
        };
        tokio::select! {
            _=revision.changed()=>Err("Language server settings changed; the previous session was stopped. Inspect before retrying.".into()),
            result=tokio::time::timeout(Duration::from_secs(server.timeout_seconds),operation)=>result.map_err(|_|"Language server timed out; no successful or clean result is claimed.".to_string())?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in language server: it accepts the handshake bytes and never
    /// answers. The test only needs a live child process holding a permit.
    fn fixture_server() -> ServerConfig {
        ServerConfig {
            id: "fixture".into(),
            language_id: "text".into(),
            command: if cfg!(windows) {
                "cmd.exe".into()
            } else {
                "/bin/cat".into()
            },
            allowed_commands: Vec::new(),
            enabled: true,
            arguments: if cfg!(windows) {
                vec!["/c".into(), "pause".into()]
            } else {
                Vec::new()
            },
            environment: BTreeMap::new(),
            initialization_options: Value::Null,
            settings: Value::Null,
            timeout_seconds: 5,
        }
    }

    #[tokio::test]
    async fn shutdown_reclaims_a_busy_slot_and_returns_its_permit() {
        let manager = Manager::default();
        let root = tempfile::tempdir().unwrap();
        let permit = manager
            .permits
            .clone()
            .acquire_owned()
            .await
            .expect("a permit is available");
        let session = Session::spawn(root.path().to_str().unwrap(), &fixture_server(), permit)
            .expect("fixture language server starts");
        let slot: Slot = Arc::new(tokio::sync::Mutex::new(Some(session)));
        manager
            .slots
            .lock()
            .expect("slot registry")
            .insert("fixture".into(), slot.clone());
        assert_eq!(manager.permits.available_permits(), 3);
        // An in-flight request holds the slot, so the registry cannot take
        // the session synchronously.
        let holder = slot.clone();
        let request = tokio::spawn(async move {
            let guard = holder.lock().await;
            tokio::time::sleep(Duration::from_millis(50)).await;
            drop(guard);
        });
        manager.shutdown().await;
        request.await.expect("the simulated request finishes");
        // The session was taken and stopped instead of being orphaned: the
        // slot is empty and the semaphore permit came back.
        assert!(slot.lock().await.is_none());
        assert_eq!(manager.permits.available_permits(), 4);
        assert!(manager.slots.lock().expect("slot registry").is_empty());
    }

    #[tokio::test]
    async fn a_request_that_finishes_after_a_retirement_never_restores_its_session() {
        let manager = Manager::default();
        let root = tempfile::tempdir().unwrap();
        let permit = manager
            .permits
            .clone()
            .acquire_owned()
            .await
            .expect("a permit is available");
        let session = Session::spawn(root.path().to_str().unwrap(), &fixture_server(), permit)
            .expect("fixture language server starts");
        let slot: Slot = Arc::new(tokio::sync::Mutex::new(None));
        // The request took the session out of its slot and is still running.
        let mut guard = slot.lock().await;
        let generation = manager.generation.load(Ordering::SeqCst);
        // The registry is retired while the request is in flight.
        manager.generation.fetch_add(1, Ordering::SeqCst);
        manager.revision.send_modify(|n| *n = n.wrapping_add(1));
        manager.restore(&mut guard, session, generation);
        // The session was not restored into the unreachable slot: it was
        // dropped, which killed the process and returned the permit.
        assert!(guard.is_none());
        drop(guard);
        assert_eq!(manager.permits.available_permits(), 4);
    }
}
