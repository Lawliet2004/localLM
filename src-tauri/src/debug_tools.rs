//! Conversation-owned, bounded managed Debug Adapter Protocol sessions.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Instant;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, Mutex as AsyncMutex},
};
const MAX_FRAME: usize = 1024 * 1024;
type Slot = Arc<AsyncMutex<Option<Session>>>;
#[derive(Default)]
pub struct Manager {
    sessions: Mutex<HashMap<String, (String, String, Slot)>>,
}
const POLICY_KEY: &str = "debugger.policy";
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Policy {
    pub request_timeout_seconds: u64,
    pub idle_timeout_seconds: u64,
    pub lifetime_seconds: u64,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            request_timeout_seconds: 30,
            idle_timeout_seconds: 900,
            lifetime_seconds: 7200,
        }
    }
}
impl Policy {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=120).contains(&self.request_timeout_seconds)
            || !(1..=86400).contains(&self.idle_timeout_seconds)
            || !(1..=86400).contains(&self.lifetime_seconds)
        {
            return Err(
                "Debugger policy requires request 1–120 seconds and idle/lifetime 1–86400 seconds."
                    .into(),
            );
        }
        Ok(())
    }
}
#[tauri::command]
pub fn get_debugger_policy(state: tauri::State<'_, crate::AppState>) -> Result<Policy, String> {
    state.database()?.setting(POLICY_KEY)
}
#[tauri::command]
pub fn save_debugger_policy(
    state: tauri::State<'_, crate::AppState>,
    policy: Policy,
) -> Result<(), String> {
    policy.validate()?;
    state.database()?.save_setting(POLICY_KEY, &policy)
}
pub fn load_policy(store: &crate::store::Store) -> Result<Policy, String> {
    let p: Policy = store.setting(POLICY_KEY)?;
    p.validate()?;
    Ok(p)
}
#[derive(Default)]
struct References {
    generation: u64,
    next: i64,
    frames: HashMap<i64, (u64, i64)>,
    variables: HashMap<i64, (u64, i64)>,
}
impl References {
    fn invalidate(&mut self) {
        self.generation += 1;
        self.frames.clear();
        self.variables.clear();
    }
    fn issue(&mut self, raw: i64, frame: bool) -> Result<i64, String> {
        if raw < 0 || (!frame && raw == 0) {
            return Ok(raw);
        }
        let map = if frame {
            &mut self.frames
        } else {
            &mut self.variables
        };
        if let Some((id, _)) = map.iter().find(|(_, v)| **v == (self.generation, raw)) {
            return Ok(*id);
        }
        self.next = self
            .next
            .checked_add(1)
            .filter(|id| *id <= i32::MAX as i64)
            .ok_or("Debugger reference ID space exhausted.")?;
        map.insert(self.next, (self.generation, raw));
        Ok(self.next)
    }
    fn resolve(&self, id: i64, frame: bool) -> Result<i64, String> {
        (if frame { &self.frames } else { &self.variables })
            .get(&id)
            .filter(|(g, _)| *g == self.generation)
            .map(|(_, raw)| *raw)
            .ok_or(
                "Stale or unissued debugger reference; inspect the current suspended state again."
                    .into(),
            )
    }
    fn translate_body(&mut self, command: &str, body: &mut Value) -> Result<(), String> {
        let (list, field, frame) = match command {
            "stackTrace" => ("stackFrames", "id", true),
            "scopes" => ("scopes", "variablesReference", false),
            "variables" => ("variables", "variablesReference", false),
            _ => return Ok(()),
        };
        for value in body[list]
            .as_array_mut()
            .ok_or("Adapter returned malformed inspection results.")?
        {
            let raw = value[field]
                .as_i64()
                .ok_or("Adapter inspection reference is not an integer.")?;
            value[field] = json!(self.issue(raw, frame)?);
        }
        Ok(())
    }
}
struct Session {
    policy: Policy,
    alive: Arc<std::sync::atomic::AtomicBool>,
    activity: tokio::sync::watch::Sender<Instant>,
    attached: bool,
    references: References,
    stdin: Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
    incoming: mpsc::Receiver<Result<Value, String>>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    seq: u64,
    launch: Option<u64>,
    capabilities: Value,
    events: VecDeque<Value>,
    lost: u64,
    phase: &'static str,
}
impl Drop for Session {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}
async fn frame(reader: &mut (impl tokio::io::AsyncRead + Unpin)) -> Result<Value, String> {
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= 8192 {
            return Err("DAP header exceeds 8 KiB.".into());
        }
        header.push(
            reader
                .read_u8()
                .await
                .map_err(|e| format!("DAP stream closed: {e}"))?,
        );
    }
    let header = std::str::from_utf8(&header).map_err(|e| e.to_string())?;
    let mut length = None;
    for line in header.split("\r\n").filter(|s| !s.is_empty()) {
        let (key, value) = line.split_once(':').ok_or("Malformed DAP header.")?;
        if key.eq_ignore_ascii_case("Content-Length") {
            if length.is_some() {
                return Err("Duplicate DAP Content-Length.".into());
            }
            length = Some(value.trim().parse::<usize>().map_err(|e| e.to_string())?);
        }
    }
    let n = length
        .filter(|n| *n > 0 && *n <= MAX_FRAME)
        .ok_or("Missing or oversized DAP frame.")?;
    let mut bytes = vec![0; n];
    reader
        .read_exact(&mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    if !v.is_object() || !v["seq"].is_u64() {
        return Err("Malformed DAP message.".into());
    }
    Ok(v)
}
// CodeLLDB reverse-connect mode authenticates its first line. Keep the listener
// bound from allocation through accept; no free-port probe/rebind race.
async fn authenticated_adapter(
    listener: tokio::net::TcpListener,
    token: &str,
) -> Result<tokio::net::TcpStream, String> {
    let (mut stream, peer) = listener.accept().await.map_err(|e| e.to_string())?;
    if !peer.ip().is_loopback() {
        return Err("Adapter peer is not loopback.".into());
    }
    let expected = format!("Auth-Token: {token}\r\n");
    let mut header = vec![0; expected.len()];
    stream
        .read_exact(&mut header)
        .await
        .map_err(|_| "Adapter authentication stream closed.")?;
    if header != expected.as_bytes() {
        return Err("Adapter authentication failed.".into());
    }
    stream.set_nodelay(true).map_err(|e| e.to_string())?;
    Ok(stream)
}
struct Tasks(Vec<tokio::task::JoinHandle<()>>);
impl Drop for Tasks {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}
fn drain_output(
    mut reader: impl tokio::io::AsyncRead + Unpin + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut buffer = [0; 8192];
        while let Ok(n) = reader.read(&mut buffer).await {
            if n == 0 {
                break;
            }
        }
    })
}
impl Session {
    async fn send(&mut self, command: &str, arguments: Value) -> Result<u64, String> {
        let seq = self.seq;
        self.seq += 1;
        self.send_value(
            json!({"seq":seq,"type":"request","command":command,"arguments":arguments}),
        )
        .await?;
        Ok(seq)
    }
    async fn send_value(&mut self, v: Value) -> Result<(), String> {
        let bytes = serde_json::to_vec(&v).map_err(|e| e.to_string())?;
        if bytes.len() > MAX_FRAME {
            return Err("DAP request exceeds 1 MiB.".into());
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
    async fn message(&mut self, v: Value) -> Result<Option<Value>, String> {
        match v["type"].as_str() {
            Some("event") => {
                if v["event"] == "capabilities" {
                    if let Some(update) = v["body"]["capabilities"].as_object() {
                        for (k, v) in update {
                            self.capabilities[k] = v.clone();
                        }
                    }
                }
                match v["event"].as_str() {
                    Some("stopped") => {
                        self.references.invalidate();
                        self.phase = "stopped";
                    }
                    Some("continued") => {
                        self.references.invalidate();
                        self.phase = "running";
                    }
                    Some("terminated") => self.phase = "terminated",
                    _ => (),
                }
                self.events.push_back(v);
                while self.events.len() > 64
                    || self
                        .events
                        .iter()
                        .map(|v| v.to_string().len())
                        .sum::<usize>()
                        > MAX_FRAME
                {
                    self.events.pop_front();
                    self.lost += 1;
                }
                Ok(None)
            }
            Some("request") => {
                // Never honor adapter-initiated process launch or nested debug sessions.
                let seq = self.seq;
                self.seq += 1;
                self.send_value(json!({"seq":seq,"type":"response","request_seq":v["seq"],"command":v["command"],"success":false,"message":"Client reverse requests are disabled."})).await?;
                Ok(None)
            }
            Some("response") => {
                if self.launch == v["request_seq"].as_u64() {
                    self.launch = None;
                    if v["success"] != true {
                        return Err(format!("DAP launch/attach failed: {}", v["message"]));
                    }
                    Ok(None)
                } else {
                    Ok(Some(v))
                }
            }
            _ => Err("Unknown DAP message type.".into()),
        }
    }
    async fn next(&mut self) -> Result<Value, String> {
        self.incoming.recv().await.ok_or("DAP reader ended.")?
    }
    async fn response(&mut self, seq: u64) -> Result<Value, String> {
        for _ in 0..512 {
            let v = self.next().await?;
            if let Some(v) = self.message(v).await? {
                if v["request_seq"] != seq {
                    return Err("Unexpected DAP response ID.".into());
                }
                if v["success"] != true {
                    return Err(format!("DAP request failed: {}", v["message"]));
                }
                return Ok(v["body"].clone());
            }
        }
        Err("DAP message budget exceeded.".into())
    }
    async fn request(&mut self, command: &str, args: Value) -> Result<Value, String> {
        let seq = self.send(command, args).await?;
        self.response(seq).await
    }
    fn event_page(&mut self) -> Value {
        json!({"events":self.events.drain(..).collect::<Vec<_>>(),"droppedEvents":self.lost,"state":self.phase,"generation":self.references.generation,"launchPending":self.launch.is_some(),"untrustedAdapterOutput":true})
    }
}
fn validate_arguments(command: &str, args: &Value) -> Result<(), String> {
    let positive = json!({"type":"integer","minimum":1,"maximum":2147483647});
    let natural = json!({"type":"integer","minimum":0,"maximum":2147483647});
    let page = json!({"type":"integer","minimum":1,"maximum":200});
    let (properties, required) = match command {
        "threads" | "configurationDone" => (json!({}), vec![]),
        "stackTrace" => (
            json!({"threadId":positive,"startFrame":natural,"levels":page}),
            vec!["threadId"],
        ),
        "scopes" => (json!({"frameId":natural}), vec!["frameId"]),
        "variables" => (
            json!({"variablesReference":positive,"start":natural,"count":page,"filter":{"type":"string","enum":["indexed","named"]}}),
            vec!["variablesReference"],
        ),
        "exceptionInfo" | "continue" | "pause" | "next" | "stepIn" | "stepOut" => {
            (json!({"threadId":positive}), vec!["threadId"])
        }
        "setExceptionBreakpoints" => (
            json!({"filters":{"type":"array","maxItems":64,"items":{"type":"string","maxLength":256}}}),
            vec!["filters"],
        ),
        "setBreakpoints" => (
            json!({"source":{"type":"object","properties":{"path":{"type":"string","minLength":1,"maxLength":4096}},"required":["path"],"additionalProperties":false},"breakpoints":{"type":"array","maxItems":200,"items":{"type":"object","properties":{"line":positive,"column":positive,"condition":{"type":"string","maxLength":4096},"hitCondition":{"type":"string","maxLength":256},"logMessage":{"type":"string","maxLength":4096}},"required":["line"],"additionalProperties":false}},"sourceModified":{"type":"boolean"}}),
            vec!["source", "breakpoints"],
        ),
        _ => return Err("Unsupported debugger command.".into()),
    };
    crate::tool_validation::validate(
        &crate::development_tools::schema(properties, &required),
        args,
    )
}
/// Releases a debugger session reservation when the start did not produce a
/// live session.
///
/// The reservation is taken before the adapter is spawned, so the session
/// limit counts a start that is in progress. Without this, a failed or
/// cancelled start left an empty slot behind that counted against the limit
/// until the next start happened to prune it.
struct DebuggerReservation<'a> {
    sessions: &'a std::sync::Mutex<std::collections::HashMap<String, (String, String, Slot)>>,
    id: String,
}

impl Drop for DebuggerReservation<'_> {
    fn drop(&mut self) {
        let Ok(mut sessions) = self.sessions.lock() else {
            return;
        };
        let empty = sessions
            .get(&self.id)
            .and_then(|(_, _, slot)| slot.try_lock().ok())
            .map(|guard| guard.is_none())
            .unwrap_or(false);
        if empty {
            sessions.remove(&self.id);
        }
    }
}

impl Manager {
    /// Stop every debugger session this manager owns.
    ///
    /// Tauri ends the application with `std::process::exit`, so no destructor
    /// runs and the `Drop`-based kill never fires. Without this the debug
    /// adapter processes outlive the app.
    pub async fn stop_all(&self) {
        let sessions = match self.sessions.lock() {
            Ok(mut sessions) => std::mem::take(&mut *sessions),
            Err(_) => return,
        };
        for (_, (_, _, slot)) in sessions {
            if let Some(mut session) = slot.lock().await.take() {
                let _ = tokio::time::timeout(
                    Duration::from_secs(2),
                    session.request("disconnect", json!({"terminateDebuggee":!session.attached})),
                )
                .await;
                drop(session);
            }
        }
    }
    #[cfg(test)]
    pub async fn execute(
        &self,
        owner: &str,
        root: &str,
        config: &crate::execution::ExecutionConfig,
        alias: &str,
        args: Value,
    ) -> Result<Value, String> {
        self.execute_policy(owner, root, config, alias, args, &Policy::default())
            .await
    }
    pub async fn execute_policy(
        &self,
        owner: &str,
        root: &str,
        config: &crate::execution::ExecutionConfig,
        alias: &str,
        args: Value,
        policy: &Policy,
    ) -> Result<Value, String> {
        policy.validate()?;
        crate::tool_validation::validate(
            &registry()
                .into_iter()
                .find(|x| x.name == alias)
                .ok_or("Unknown debugger tool.")?
                .input,
            &args,
        )?;
        let root = std::fs::canonicalize(root)
            .map_err(|e| e.to_string())?
            .to_string_lossy()
            .into_owned();
        if !config.bash.enabled && alias != "debugger_stop" {
            return Err("Shell execution is disabled; debugger access denied.".into());
        }
        if alias == "debugger_start" {
            if owner.is_empty() {
                return Err("Debugger requires a conversation.".into());
            }
            let executable = args["adapter"].as_str().ok_or("Missing adapter.")?;
            if !crate::execution::executable(executable) {
                return Err("Choose an installed absolute debug adapter executable.".into());
            }
            // Reserve before spawning. The reservation is released when the
            // start fails or is cancelled, so it never counts against the
            // session limit.
            let id = format!("debug-{}", uuid::Uuid::new_v4());
            let slot = Arc::new(AsyncMutex::new(None));
            {
                let mut sessions = self
                    .sessions
                    .lock()
                    .map_err(|_| "Debugger registry unavailable.")?;
                sessions.retain(|_, (_, _, s)| {
                    s.try_lock().map_or(true, |s| {
                        s.as_ref()
                            .is_some_and(|s| s.alive.load(std::sync::atomic::Ordering::Acquire))
                    })
                });
                if sessions.len() >= 16
                    || sessions.values().filter(|(o, _, _)| o == owner).count() >= 4
                {
                    return Err("Debugger session limit reached; stop an existing session.".into());
                }
                sessions.insert(id.clone(), (owner.into(), root.clone(), slot.clone()));
            }
            let _reservation = DebuggerReservation {
                sessions: &self.sessions,
                id: id.clone(),
            };
            let mut guard = slot.lock().await;
            let started = tokio::time::Instant::now();
            let lifetime_deadline = started + Duration::from_secs(policy.lifetime_seconds);
            let tcp = args["transport"] == "codelldb";
            let listener = if tcp {
                Some(
                    tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
                        .await
                        .map_err(|e| format!("Cannot bind owned adapter listener: {e}"))?,
                )
            } else {
                None
            };
            let token = uuid::Uuid::new_v4().simple().to_string();
            let mut command = tokio::process::Command::new(executable);
            if listener.is_some() {
                // Transport switches are exclusively managed, including authentication.
                if args["adapter_args"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .any(|a| {
                        ["--port", "--connect", "--auth-token", "--multi-session"]
                            .iter()
                            .any(|s| a == *s || a.starts_with(&format!("{s}=")))
                    })
                {
                    return Err("CodeLLDB transport arguments are managed by LocalLM.".into());
                }
            }
            command
                .args(
                    args["adapter_args"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str),
                )
                .current_dir(&root)
                .env_clear()
                .envs(&config.bash.environment)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);
            if let Some(listener) = &listener {
                command.args([
                    "--connect",
                    &listener
                        .local_addr()
                        .map_err(|e| e.to_string())?
                        .port()
                        .to_string(),
                    "--auth-token",
                    &token,
                ]);
            }
            for key in [
                "SystemRoot",
                "WINDIR",
                "TEMP",
                "TMP",
                "PATH",
                "PATHEXT",
                "COMSPEC",
                "USERPROFILE",
                "HOME",
                "APPDATA",
                "LOCALAPPDATA",
            ] {
                if !config.bash.environment.contains_key(key) {
                    if let Some(value) = std::env::var_os(key) {
                        command.env(key, value);
                    }
                }
            }
            crate::execution::memory_limit(&mut command, &config.bash);
            let mut child = crate::execution::managed_spawn(command).map_err(|e| e.to_string())?;
            let stdin = child.stdin().take().ok_or("DAP stdin unavailable.")?;
            let stdout = child.stdout().take().ok_or("DAP stdout unavailable.")?;
            let stderr = child.stderr().take().ok_or("DAP stderr unavailable.")?;
            let mut tasks = Tasks(vec![drain_output(stderr)]);
            let (stdin, mut stdout): (
                Box<dyn tokio::io::AsyncWrite + Unpin + Send>,
                Box<dyn tokio::io::AsyncRead + Unpin + Send>,
            ) = if let Some(listener) = listener {
                drop(stdin);
                tasks.0.push(drain_output(stdout));
                let stream = tokio::time::timeout(
                    Duration::from_secs(
                        policy.request_timeout_seconds.min(policy.lifetime_seconds),
                    ),
                    authenticated_adapter(listener, &token),
                )
                .await
                .map_err(|_| "CodeLLDB connection/authentication timed out; adapter stopped.")??;
                let (read, write) = stream.into_split();
                (Box::new(write), Box::new(read))
            } else {
                (Box::new(stdin), Box::new(stdout))
            };
            let (send, incoming) = mpsc::channel(16);
            let (failed, mut failure) = tokio::sync::oneshot::channel();
            let framing_timeout = Duration::from_secs(policy.request_timeout_seconds);
            tasks.0.push(tokio::spawn(async move {
                loop {
                    // Idle policy governs waiting for a frame; once a frame starts,
                    // header and payload must both arrive within the framing budget.
                    let v = match stdout.read_u8().await {
                        Ok(first) => {
                            let first = [first];
                            let mut input = first.as_slice().chain(&mut stdout);
                            tokio::time::timeout(framing_timeout, frame(&mut input))
                                .await
                                .unwrap_or_else(|_| Err("DAP frame timed out.".into()))
                        }
                        Err(e) => Err(format!("DAP stream closed: {e}")),
                    };
                    let done = v.is_err();
                    if send.send(v).await.is_err() || done {
                        break;
                    }
                }
                let _ = failed.send(());
            }));
            let idle = Duration::from_secs(policy.idle_timeout_seconds);
            let (activity, mut touched) = tokio::sync::watch::channel(Instant::now());
            let alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
            let running = alive.clone();
            tasks.0.push(tokio::spawn(async move {
                let deadline = lifetime_deadline;
                loop {
                    let idle_deadline = tokio::time::Instant::from_std(*touched.borrow() + idle);
                    tokio::select! {
                        _=child.wait()=>break,
                        _=&mut failure=>{let _=child.start_kill();let _=child.wait().await;break;},
                        _=tokio::time::sleep_until(deadline)=>{let _=child.start_kill();let _=child.wait().await;break;},
                        _=tokio::time::sleep_until(idle_deadline)=>{let _=child.start_kill();let _=child.wait().await;break;},
                        changed=touched.changed()=>if changed.is_err(){let _=child.start_kill();let _=child.wait().await;break;}
                    }
                }
                running.store(false,std::sync::atomic::Ordering::Release);
            }));
            let mut session = Session {
                policy: policy.clone(),
                alive,
                activity,
                attached: args["mode"] == "attach",
                references: References::default(),
                stdin,
                incoming,
                tasks: std::mem::take(&mut tasks.0),
                seq: 1,
                launch: None,
                capabilities: Value::Null,
                events: VecDeque::new(),
                lost: 0,
                phase: "initializing",
            };
            tokio::time::timeout(Duration::from_secs(policy.request_timeout_seconds).min(lifetime_deadline.saturating_duration_since(tokio::time::Instant::now())),async {
                session.capabilities=session.request("initialize",json!({"clientID":"locallm","adapterID":executable,"pathFormat":"path","linesStartAt1":true,"columnsStartAt1":true,"supportsRunInTerminalRequest":false,"supportsStartDebuggingRequest":false,"supportsVariablePaging":true})).await?;
                let mode=args["mode"].as_str().unwrap_or("launch");
                session.launch=Some(session.send(mode,args["configuration"].clone()).await?);
                if session.events.iter().any(|v|v["event"]=="initialized"){session.phase="configuring";return Ok(());}
                for _ in 0..512 {
                    let v=session.next().await?;let initialized=v["type"]=="event"&&v["event"]=="initialized";
                    if session.message(v).await?.is_some(){return Err("Unexpected response during DAP initialization.".to_string());}
                    if initialized{session.phase="configuring";return Ok(());}
                }
                Err("DAP never sent initialized.".to_string())
            }).await.map_err(|_|"Debugger initialization timed out; adapter stopped.".to_string())??;
            let result = json!({"sessionId":id,"state":session.phase,"capabilities":session.capabilities,"launchPending":session.launch.is_some(),"policy":policy,"message":"Set breakpoints/exceptions, then configurationDone if supported. Timing follows debugger policy."});
            *guard = Some(session);
            return Ok(result);
        }
        let id = args["session_id"]
            .as_str()
            .ok_or("Missing debugger session ID.")?;
        let slot = {
            let sessions = self
                .sessions
                .lock()
                .map_err(|_| "Debugger registry unavailable.")?;
            let (o, w, s) = sessions.get(id).ok_or("Unknown debugger session.")?;
            if o != owner || w != &root {
                return Err("Debugger belongs to another conversation/workspace.".into());
            }
            s.clone()
        };
        let mut guard = slot.lock().await;
        let mut session = guard
            .take()
            .ok_or("Debugger session is closed; start a new session.")?;
        session.activity.send_replace(Instant::now());
        if alias == "debugger_stop" {
            let _ = tokio::time::timeout(
                Duration::from_secs(2),
                session.request("disconnect", json!({"terminateDebuggee":!session.attached})),
            )
            .await;
            drop(session);
            self.sessions
                .lock()
                .map_err(|_| "Debugger registry unavailable.")?
                .remove(id);
            return Ok(
                json!({"sessionId":id,"state":"closed","message":"Adapter process tree stopped. Attached external processes are not owned or terminated by LocalLM."}),
            );
        }
        let result=tokio::time::timeout(Duration::from_secs(session.policy.request_timeout_seconds),async {
            if alias=="debugger_events" {
                let wait=args["wait_ms"].as_u64().unwrap_or(0);
                if wait>0 {if let Ok(v)=tokio::time::timeout(Duration::from_millis(wait),session.next()).await {let v=v?;if session.message(v).await?.is_some(){return Err("Unexpected DAP response.".into());}}}
                for _ in 0..64 {match session.incoming.try_recv(){Ok(v)=>{if session.message(v?).await?.is_some(){return Err("Unexpected DAP response.".into());}},Err(_)=>break}}
                return Ok(session.event_page());
            }
            let command=args["command"].as_str().ok_or("Missing debugger command.")?;
            let mut params=args.get("arguments").cloned().unwrap_or(json!({}));
            validate_arguments(command,&params)?;
            if command=="setBreakpoints" {
                let path=params["source"]["path"].as_str().ok_or("setBreakpoints requires source.path relative to workspace.")?;
                crate::workspace::Workspace::open(&root)?.language_source(path)?;
                params["source"]["path"]=json!(std::path::Path::new(&root).join(path));
                for (field,capability) in [("condition","supportsConditionalBreakpoints"),("hitCondition","supportsHitConditionalBreakpoints"),("logMessage","supportsLogPoints")] {
                    if params["breakpoints"].as_array().into_iter().flatten().any(|b|b.get(field).is_some())&&session.capabilities[capability]!=true{return Err(format!("Adapter does not support {field}."));}
                }
            }
            if command=="configurationDone"&&session.capabilities["supportsConfigurationDoneRequest"]!=true{session.phase="running";return Ok(json!({"sessionId":id,"configurationDoneRequired":false,"events":session.event_page()}));}
            for _ in 0..64 {match session.incoming.try_recv(){Ok(v)=>{if session.message(v?).await?.is_some(){return Err("Unexpected DAP response.".into());}},Err(_)=>break}}
            if matches!(command,"stackTrace"|"scopes"|"variables") {
                if session.phase!="stopped"{return Err("Inspection requires a current stopped event.".into());}
                if command=="scopes"{params["frameId"]=json!(session.references.resolve(params["frameId"].as_i64().ok_or("Missing frame ID.")?,true)?);}
                if command=="variables"{params["variablesReference"]=json!(session.references.resolve(params["variablesReference"].as_i64().ok_or("Missing variable reference.")?,false)?);}
            }
            let capability=match command{"stepOut"=>None,"exceptionInfo"=>Some("supportsExceptionInfoRequest"),_=>None};
            if capability.is_some_and(|key|session.capabilities[key]!=true){return Err("Adapter does not advertise this optional request.".into());}
            if matches!(command,"continue"|"next"|"stepIn"|"stepOut"|"configurationDone"){session.references.invalidate();session.phase="running";}
            let mut body=session.request(command,params).await?;session.references.translate_body(command,&mut body)?;
            Ok(json!({"sessionId":id,"command":command,"body":body,"events":session.event_page(),"untrustedAdapterOutput":true}))
        }).await.map_err(|_|"Debugger request timed out; adapter stopped and session closed. Do not replay an uncertain action.".to_string())?;
        // On any protocol error or cancellation the leased session is dropped.
        if result.is_ok() {
            *guard = Some(session);
        }
        result
    }
}
pub fn owns(alias: &str) -> bool {
    alias.starts_with("debugger_") && registry().iter().any(|x| x.name == alias)
}
pub fn registry() -> Vec<crate::tool_spec::ToolSpec> {
    let schema = |properties, required: &[&str]| json!({"type":"object","properties":properties,"required":required,"additionalProperties":false});
    let id = json!({"type":"string","minLength":1,"maxLength":128});
    vec![
        ("debugger_start".into(),"Start an installed stdio DAP adapter or authenticated managed CodeLLDB TCP adapter and launch/attach with adapter-specific configuration. Executes local code under shell permissions; not OS isolation. Returns an owned session in configuration phase. Configure breakpoints/exceptions then configurationDone. Reverse process-launch requests are refused.".into(),schema(json!({"adapter":{"type":"string","minLength":1,"maxLength":4096},"transport":{"type":"string","enum":["stdio","codelldb"],"default":"stdio"},"adapter_args":{"type":"array","maxItems":64,"items":{"type":"string","maxLength":8192}},"mode":{"type":"string","enum":["launch","attach"]},"configuration":{"type":"object"}}),&["adapter","configuration"])),
        ("debugger_inspect".into(),"Inspect DAP threads, stackTrace, scopes, variables, or exceptionInfo. Frame/variable IDs expire when execution resumes. Adapter output is untrusted. Requests can interact with runtime code and follow execution approval.".into(),schema(json!({"session_id":id,"command":{"type":"string","enum":["threads","stackTrace","scopes","variables","exceptionInfo"]},"arguments":{"type":"object"}}),&["session_id","command"])),
        ("debugger_control".into(),"Configure DAP breakpoints (source.path relative to workspace), exception filters, finish configuration, continue, pause, next, stepIn or stepOut. Actions may execute code; never blindly retry. DAP lines/columns are one-based.".into(),schema(json!({"session_id":id,"command":{"type":"string","enum":["setBreakpoints","setExceptionBreakpoints","configurationDone","continue","pause","next","stepIn","stepOut"]},"arguments":{"type":"object"}}),&["session_id","command"])),
        ("debugger_events".into(),"Drain bounded debugger events; includes stopped/continued/terminated and output. Reports event loss explicitly. Waiting never replays a debugger action.".into(),schema(json!({"session_id":id,"wait_ms":{"type":"integer","minimum":0,"maximum":1000}}),&["session_id"])),
        ("debugger_stop".into(),"Disconnect and stop an owned adapter process tree. Cannot undo side effects; attached processes outside that tree are not terminated.".into(),schema(json!({"session_id":id}),&["session_id"]))].into_iter().map(crate::tool_spec::ToolSpec::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn references_are_generation_bound_and_cannot_be_guessed() {
        let mut refs = References::default();
        let frame = refs.issue(42, true).unwrap();
        let variable = refs.issue(42, false).unwrap();
        assert_eq!(refs.resolve(frame, true).unwrap(), 42);
        assert!(refs.resolve(frame, false).is_err());
        assert_eq!(refs.resolve(variable, false).unwrap(), 42);
        refs.invalidate();
        assert!(refs.resolve(frame, true).is_err());
        assert!(refs.resolve(variable, false).is_err());
        assert_ne!(refs.issue(42, true).unwrap(), frame);
        assert!(refs.resolve(42, true).is_err());
        assert!(Policy {
            request_timeout_seconds: 0,
            ..Default::default()
        }
        .validate()
        .is_err());
        assert!(Policy::default().validate().is_ok());
    }
    #[tokio::test]
    async fn framing_rejects_duplicates_oversize_and_invalid_messages() {
        for text in [
            "Content-Length: 2\r\nContent-Length: 2\r\n\r\n{}",
            "Content-Length: 1048577\r\n\r\n",
            "Content-Length: 2\r\n\r\n{}",
        ] {
            assert!(frame(&mut text.as_bytes()).await.is_err());
        }
        let value = json!({"seq":1,"type":"event","event":"stopped"});
        let body = value.to_string();
        let input = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);
        assert_eq!(frame(&mut input.as_bytes()).await.unwrap(), value);
    }
    #[tokio::test]
    async fn tcp_handshake_rejects_wrong_token_and_bounds_partial_input() {
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let mut client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        client.write_all(b"Auth-Token: wrong\r\n").await.unwrap();
        assert!(authenticated_adapter(listener, "right")
            .await
            .unwrap_err()
            .contains("authentication"));
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let _client = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        assert!(tokio::time::timeout(
            Duration::from_millis(20),
            authenticated_adapter(listener, "secret")
        )
        .await
        .is_err());
    }
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn managed_tcp_startup_timeout_and_cancellation_kill_owned_adapter() {
        for cancel in [false, true] {
            let t = tempfile::tempdir().unwrap();
            let script = t.path().join("adapter.py");
            let pidfile = t.path().join("pid");
            std::fs::write(
                &script,
                "import os,time\nopen('pid','w').write(str(os.getpid()))\ntime.sleep(60)\n",
            )
            .unwrap();
            let manager = Arc::new(Manager::default());
            let m = manager.clone();
            let root = t.path().to_str().unwrap().to_string();
            let task = tokio::spawn(async move {
                let config = crate::execution::ExecutionConfig::default();
                m.execute_policy("owner", &root, &config, "debugger_start", json!({"adapter":config.python_path,"adapter_args":[script],"transport":"codelldb","configuration":{}}), &Policy { request_timeout_seconds:1, ..Default::default() }).await
            });
            tokio::time::timeout(Duration::from_secs(3), async {
                while !pidfile.exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            let pid = std::fs::read_to_string(pidfile).unwrap();
            if cancel {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                assert!(task.await.unwrap().unwrap_err().contains("timed out"));
            }
            tokio::time::timeout(Duration::from_secs(3), async {
                while std::path::Path::new(&format!("/proc/{pid}")).exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        }
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn real_stdio_launch_configure_inspect_ownership_and_stop() {
        adapter_lifecycle(false).await;
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn managed_tcp_launch_configure_inspect_ownership_and_stop() {
        adapter_lifecycle(true).await;
    }
    #[cfg(unix)]
    async fn adapter_lifecycle(tcp: bool) {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("main.py"), "x=42\n").unwrap();
        let script = temp.path().join("adapter.py");
        std::fs::write(&script,r#"import json,sys,socket
if '--connect' in sys.argv:
 port=int(sys.argv[sys.argv.index('--connect')+1]);token=sys.argv[sys.argv.index('--auth-token')+1]
 sock=socket.create_connection(('127.0.0.1',port));sock.sendall(('Auth-Token: '+token+'\r\n').encode())
 print('Diagnostic text is not a DAP frame',flush=True)
 inp=sock.makefile('rb');out=sock.makefile('wb')
else:inp=sys.stdin.buffer;out=sys.stdout.buffer
seq=0
launch=None
def emit(v):
 global seq
 seq+=1;v['seq']=seq;b=json.dumps(v).encode();out.write(('Content-Length: %d\r\n\r\n'%len(b)).encode()+b);out.flush()
def response(r,body={}):emit(dict(type='response',request_seq=r['seq'],command=r['command'],success=True,body=body))
while True:
 h={}
 while True:
  line=inp.readline()
  if not line:sys.exit(0)
  if line==b'\r\n':break
  k,v=line.decode().split(':',1);h[k.lower()]=v.strip()
 r=json.loads(inp.read(int(h['content-length'])));c=r['command']
 if c=='initialize':response(r,dict(supportsConfigurationDoneRequest=True,supportsConditionalBreakpoints=True))
 elif c in ('launch','attach'):
  launch=r;emit(dict(type='event',event='initialized'))
 elif c=='configurationDone':
  response(r);response(launch);emit(dict(type='event',event='stopped',body=dict(reason='breakpoint',threadId=1)))
 elif c=='threads':response(r,dict(threads=[dict(id=1,name='main')]))
 elif c=='stackTrace':response(r,dict(stackFrames=[dict(id=4,name='main',line=1,column=1)]))
 elif c=='scopes':response(r,dict(scopes=[dict(name='locals',variablesReference=1,expensive=False)]))
 elif c=='variables':response(r,dict(variables=[dict(name='x',value='42',variablesReference=0)]))
 elif c=='disconnect':response(r);sys.exit(0)
 else:response(r)
"#).unwrap();
        let manager = Manager::default();
        let config = crate::execution::ExecutionConfig::default();
        let root = temp.path().to_str().unwrap();
        let start=manager.execute("owner",root,&config,"debugger_start",json!({"adapter":config.python_path,"adapter_args":[script],"transport":if tcp {"codelldb"}else{"stdio"},"configuration":{"program":"main.py"}})).await.unwrap();
        let id = start["sessionId"].as_str().unwrap();
        assert_eq!(start["state"], "configuring");
        assert!(manager
            .execute(
                "foreign",
                root,
                &config,
                "debugger_events",
                json!({"session_id":id})
            )
            .await
            .is_err());
        manager.execute("owner",root,&config,"debugger_control",json!({"session_id":id,"command":"setBreakpoints","arguments":{"source":{"path":"main.py"},"breakpoints":[{"line":1,"condition":"x==42"}]}})).await.unwrap();
        manager
            .execute(
                "owner",
                root,
                &config,
                "debugger_control",
                json!({"session_id":id,"command":"configurationDone"}),
            )
            .await
            .unwrap();
        for _ in 0..20 {
            let events = manager
                .execute(
                    "owner",
                    root,
                    &config,
                    "debugger_events",
                    json!({"session_id":id,"wait_ms":100}),
                )
                .await
                .unwrap();
            if events["state"] == "stopped" {
                break;
            }
        }
        let stack = manager
            .execute(
                "owner",
                root,
                &config,
                "debugger_inspect",
                json!({"session_id":id,"command":"stackTrace","arguments":{"threadId":1}}),
            )
            .await
            .unwrap();
        let scopes=manager.execute("owner",root,&config,"debugger_inspect",json!({"session_id":id,"command":"scopes","arguments":{"frameId":stack["body"]["stackFrames"][0]["id"]}})).await.unwrap();
        let vars = manager
            .execute(
                "owner",
                root,
                &config,
                "debugger_inspect",
                json!({"session_id":id,"command":"variables","arguments":{"variablesReference":scopes["body"]["scopes"][0]["variablesReference"]}}),
            )
            .await
            .unwrap();
        assert_eq!(vars["body"]["variables"][0]["value"], "42");
        manager
            .execute(
                "owner",
                root,
                &config,
                "debugger_stop",
                json!({"session_id":id}),
            )
            .await
            .unwrap();
        assert!(manager
            .execute(
                "owner",
                root,
                &config,
                "debugger_events",
                json!({"session_id":id})
            )
            .await
            .is_err());
    }
}

#[cfg(test)]
mod acceptance {
    use super::*;
    #[tokio::test]
    #[ignore = "Requires Python with installed debugpy in LOCALLM_DEBUGPY_PYTHON; permits loopback/process debugging."]
    async fn real_debugpy() {
        real_adapter(false).await;
    }
    #[tokio::test]
    #[ignore = "Requires installed LOCALLM_CODELLDB and C compiler LOCALLM_CC; permits loopback/process debugging."]
    async fn real_codelldb() {
        real_adapter(true).await;
    }
    async fn real_adapter(tcp: bool) {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("main.py"), "x=42\nprint(x)\n").unwrap();
        let config = crate::execution::ExecutionConfig::default();
        let adapter = std::env::var(if tcp {
            "LOCALLM_CODELLDB"
        } else {
            "LOCALLM_DEBUGPY_PYTHON"
        })
        .expect("Configure installed adapter");
        let manager = Manager::default();
        let root = t.path().to_str().unwrap();
        let arguments = if tcp {
            let source = t.path().join("main.c");
            std::fs::write(
                &source,
                "int main(void) { volatile int x = 42; return x == 42 ? 0 : 1; }\n",
            )
            .unwrap();
            let program = t
                .path()
                .join(if cfg!(windows) { "main.exe" } else { "main" });
            let cc = std::env::var("LOCALLM_CC").expect("Configure installed C compiler");
            assert!(std::process::Command::new(cc)
                .args(["-g", "-O0"])
                .arg(&source)
                .arg("-o")
                .arg(&program)
                .status()
                .unwrap()
                .success());
            json!({"adapter":adapter,"transport":"codelldb","configuration":{"program":program,"terminal":"console","stopOnEntry":true}})
        } else {
            json!({"adapter":adapter,"adapter_args":["-m","debugpy.adapter"],"configuration":{"program":"main.py","python":adapter,"console":"internalConsole","stopOnEntry":true}})
        };
        let start = manager
            .execute("owner", root, &config, "debugger_start", arguments)
            .await
            .unwrap();
        let id = start["sessionId"].as_str().unwrap();
        manager
            .execute(
                "owner",
                root,
                &config,
                "debugger_control",
                json!({"session_id":id,"command":"configurationDone"}),
            )
            .await
            .unwrap();
        let mut stopped = false;
        for _ in 0..20 {
            let events = manager
                .execute(
                    "owner",
                    root,
                    &config,
                    "debugger_events",
                    json!({"session_id":id,"wait_ms":500}),
                )
                .await
                .unwrap();
            if events["state"] == "stopped" {
                stopped = true;
                break;
            }
        }
        assert!(stopped, "debugpy never suspended the fixture");
        let threads = manager
            .execute(
                "owner",
                root,
                &config,
                "debugger_inspect",
                json!({"session_id":id,"command":"threads"}),
            )
            .await
            .unwrap();
        assert!(!threads["body"]["threads"].as_array().unwrap().is_empty());
        manager
            .execute(
                "owner",
                root,
                &config,
                "debugger_stop",
                json!({"session_id":id}),
            )
            .await
            .unwrap();
    }
}
