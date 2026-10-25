//! Conversation-owned asynchronous commands. Supervisors own the process tree;
//! dropping the registry closes their control channels and terminates them.
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    process::Stdio,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    sync::{mpsc, oneshot},
};

const OUTPUT_BYTES: usize = 256 * 1024;
const MAX_PROCESSES: usize = 64;
const MAX_ACTIVE_PER_OWNER: usize = 8;

pub fn registry() -> Vec<crate::tool_spec::ToolSpec> {
    let schema = |properties, required: &[&str]| json!({"type":"object","properties":properties,"required":required,"additionalProperties":false});
    let id = json!({"type":"string","minLength":1,"maxLength":128,"description":"The process ID returned by process_start, owned by this conversation and workspace."});
    vec![
        ("process_start".into(), "Start an approved long-running shell command when you need incremental output or later stdin input. Returns a conversation-owned process ID, state and output cursor promptly. Use process_poll to follow output and process_stop to terminate the process tree. The configured shell permissions, environment, memory limit and timeout apply; this is not OS isolation.".into(), schema(json!({
            "command":{"type":"string","minLength":1,"maxLength":32768},
            "cwd":{"type":"string","maxLength":4096,"description":"Directory inside the authorized workspace; defaults to the configured shell directory."},
            "timeout_seconds":{"type":"integer","minimum":1,"maximum":300,"description":"Lifetime bounded by the configured shell timeout."},
            "yield_ms":{"type":"integer","minimum":0,"maximum":1000,"default":100},
            "stdin":{"type":"string","maxLength":65536},"close_stdin":{"type":"boolean","default":false}
        }), &["command"])),
        ("process_poll".into(), "Read incremental stdout/stderr and the current state of an owned process. Pass the previous nextCursor to avoid repeated output. hasMore means another page is available; truncated and firstAvailableCursor explicitly report output lost from the bounded buffer. Polling does not execute a command.".into(), schema(json!({
            "id":id,"cursor":{"type":"integer","minimum":0,"default":0},
            "max_bytes":{"type":"integer","minimum":8192,"maximum":65536,"default":65536},
            "wait_ms":{"type":"integer","minimum":0,"maximum":1000,"default":0}
        }), &["id"])),
        ("process_write".into(), "Write stdin to an owned running process, for example a line ending in newline. Set close_stdin to deliver EOF. Input can cause side effects and follows shell approval settings. A timeout can mean bytes were partially written; do not blindly retry.".into(), schema(json!({
            "id":id,"input":{"type":"string","maxLength":65536},"close_stdin":{"type":"boolean","default":false}
        }), &["id","input"])),
        ("process_stop".into(), "Stop an owned process and its descendants, then return its final state and bounded output. Already completed processes return their stored result. Stopping cannot undo side effects that already occurred.".into(), schema(json!({"id":id}), &["id"])),
        ("shell_run".into(), "Run a shell command to completion and return exit code, bounded stdout/stderr and duration. On timeout the process keeps running and the returned processId continues the lifecycle through process_poll and process_stop. The configured shell permissions, environment, memory limit and timeout apply; this is not OS isolation.".into(), schema(json!({
            "command":{"type":"string","minLength":1,"maxLength":32768},
            "cwd":{"type":"string","maxLength":4096,"description":"Directory inside the authorized workspace; defaults to the configured shell directory."},
            "timeout_secs":{"type":"integer","minimum":1,"maximum":600,"default":120,"description":"How long to wait for exit before returning the live processId."}
        }), &["command"])),
    ].into_iter().map(crate::tool_spec::ToolSpec::from).collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartArgs {
    command: String,
    cwd: Option<String>,
    timeout_seconds: Option<u64>,
    yield_ms: Option<u64>,
    #[serde(default)]
    stdin: String,
    #[serde(default)]
    close_stdin: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PollArgs {
    id: String,
    cursor: Option<u64>,
    max_bytes: Option<usize>,
    wait_ms: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteArgs {
    id: String,
    input: String,
    #[serde(default)]
    close_stdin: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StopArgs {
    id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShellRunArgs {
    command: String,
    cwd: Option<String>,
    timeout_secs: Option<u64>,
}

struct Frame {
    cursor: u64,
    stream: &'static str,
    text: String,
}
struct Output {
    phase: &'static str,
    exit_code: Option<i32>,
    error: Option<String>,
    frames: VecDeque<Frame>,
    next: u64,
    bytes: usize,
    incomplete: bool,
    started_at: std::time::Instant,
    duration_ms: Option<f64>,
}

impl Default for Output {
    fn default() -> Self {
        Self {
            phase: "running",
            exit_code: None,
            error: None,
            frames: VecDeque::new(),
            next: 0,
            bytes: 0,
            incomplete: false,
            started_at: std::time::Instant::now(),
            duration_ms: None,
        }
    }
}

impl Output {
    fn push(&mut self, stream: &'static str, text: String) {
        if text.is_empty() {
            return;
        }
        self.bytes += text.len();
        self.frames.push_back(Frame {
            cursor: self.next,
            stream,
            text,
        });
        self.next += 1;
        while self.bytes > OUTPUT_BYTES {
            if let Some(frame) = self.frames.pop_front() {
                self.bytes -= frame.text.len();
            } else {
                break;
            }
        }
    }
    fn poll(&self, id: &str, cursor: u64, max_bytes: usize) -> Result<Value, String> {
        if cursor > self.next {
            return Err("Process cursor is beyond the available output.".into());
        }
        let base = self
            .frames
            .front()
            .map(|frame| frame.cursor)
            .unwrap_or(self.next);
        let mut next = cursor.max(base);
        let from = next;
        let mut bytes = 0;
        let mut frames = Vec::new();
        for frame in self.frames.iter().filter(|frame| frame.cursor >= from) {
            if bytes + frame.text.len() > max_bytes {
                break;
            }
            bytes += frame.text.len();
            frames.push(json!({"cursor":frame.cursor,"stream":frame.stream,"text":frame.text}));
            next = frame.cursor + 1;
        }
        let error = if self.phase == "stopped" && !self.incomplete {
            None
        } else {
            self.error.as_deref()
        };
        let message = self.error.as_deref();
        Ok(
            json!({"id":id,"state":self.phase,"durationMs":self.duration_ms,"exitCode":self.exit_code,"error":error,"message":message,
            "output":frames,"nextCursor":next,"hasMore":next < self.next,
            "truncated":cursor < base || self.incomplete,"firstAvailableCursor":base,"sideEffects":"possible",
            "isError":self.incomplete || matches!(self.phase, "failed" | "timed_out" | "unknown") || (self.phase == "exited" && self.exit_code.is_some_and(|code| code != 0))}),
        )
    }
}

enum Control {
    Write {
        input: Vec<u8>,
        close: bool,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Stop {
        reply: oneshot::Sender<()>,
    },
}

struct Process {
    owner: String,
    workspace: String,
    created: std::time::Instant,
    output: Arc<Mutex<Output>>,
    control: mpsc::Sender<Control>,
}

#[derive(Default)]
pub struct ProcessRegistry {
    processes: Mutex<HashMap<String, Arc<Process>>>,
}

/// A cancelled start must not leave a process whose ID was never delivered.
struct PendingStart<'a> {
    registry: &'a ProcessRegistry,
    id: String,
    armed: bool,
}
impl Drop for PendingStart<'_> {
    fn drop(&mut self) {
        if self.armed {
            if let Ok(mut processes) = self.registry.processes.lock() {
                processes.remove(&self.id);
            }
        }
    }
}

impl ProcessRegistry {
    /// Terminate every process this registry owns.
    ///
    /// Tauri ends the application with `std::process::exit`, so no destructor
    /// runs and the `Drop`-based kill never fires. Dropping the registry
    /// entries closes each control channel, which makes the supervisor
    /// terminate the child and reap it.
    pub fn stop_all(&self) {
        if let Ok(mut processes) = self.processes.lock() {
            processes.clear();
        }
    }
    /// Terminate every process owned by one conversation.
    ///
    /// Deleting a conversation must not leave its child processes running
    /// with no reachable handle: `process_stop` needs the id, and the
    /// conversation that produced it no longer exists.
    pub fn stop_owned(&self, owner: &str) {
        if let Ok(mut processes) = self.processes.lock() {
            let owned: Vec<String> = processes
                .iter()
                .filter(|(_, process)| process.owner == owner)
                .map(|(id, _)| id.clone())
                .collect();
            for id in owned {
                processes.remove(&id);
            }
        }
    }
    pub fn has_owned(&self, owner: &str, workspace: &str) -> Result<bool, String> {
        let processes = self
            .processes
            .lock()
            .map_err(|_| "Process registry is unavailable.")?;
        Ok(processes
            .values()
            .any(|process| process.owner == owner && process.workspace == workspace))
    }

    pub async fn execute(
        &self,
        owner: &str,
        workspace: &str,
        config: crate::execution::ExecutionConfig,
        alias: &str,
        args: Value,
    ) -> Result<Value, String> {
        match alias {
            "process_start" => self.start(owner, workspace, config, args).await,
            "process_poll" => self.poll(owner, workspace, args).await,
            "process_write" => {
                if !config.bash.enabled {
                    return Err("Shell execution is disabled; process input is denied.".into());
                }
                self.write(owner, workspace, args).await
            }
            "process_stop" => self.stop(owner, workspace, args).await,
            "shell_run" => self.shell_run(owner, workspace, config, args).await,
            _ => Err(format!("Unknown process tool '{alias}'.")),
        }
    }

    fn owned(&self, owner: &str, workspace: &str, id: &str) -> Result<Arc<Process>, String> {
        let processes = self
            .processes
            .lock()
            .map_err(|_| "Process registry is unavailable.")?;
        let process = processes
            .get(id)
            .ok_or("Process is unknown or its completed record expired.")?;
        if process.owner != owner || process.workspace != workspace {
            return Err("Process does not belong to this conversation and workspace.".into());
        }
        Ok(process.clone())
    }

    pub async fn start(
        &self,
        owner: &str,
        workspace: &str,
        config: crate::execution::ExecutionConfig,
        args: Value,
    ) -> Result<Value, String> {
        self.start_observed(owner, workspace, config, args, None)
            .await
    }
    pub(crate) async fn start_observed(
        &self,
        owner: &str,
        workspace: &str,
        mut config: crate::execution::ExecutionConfig,
        args: Value,
        completion: Option<crate::verification_observer::Completion>,
    ) -> Result<Value, String> {
        let args: StartArgs = serde_json::from_value(args).map_err(|error| error.to_string())?;
        let yield_ms = args.yield_ms.unwrap_or(100);
        let timeout = args.timeout_seconds.unwrap_or(config.bash.timeout_seconds);
        if owner.is_empty() || workspace.is_empty() {
            return Err("Process start needs a conversation and workspace.".into());
        }
        if args.command.trim().is_empty() || args.command.len() > 32768 || args.stdin.len() > 65536
        {
            return Err(
                "Process command must contain 1–32768 bytes and stdin at most 64 KiB.".into(),
            );
        }
        if yield_ms > 1000 || !(1..=300).contains(&timeout) || timeout > config.bash.timeout_seconds
        {
            return Err("Process yield must be 0–1000 ms; timeout must be 1–300 seconds within the configured shell limit.".into());
        }
        if let Some(cwd) = args.cwd {
            config.bash.working_directory = cwd;
        }
        let execution = crate::execution::LocalExecution::new(config.clone(), workspace)?;
        if !execution.command_available() {
            return Err("Shell execution is disabled or no executable shell is configured.".into());
        }
        let cwd = execution.shell_directory()?;
        let mut command = tokio::process::Command::new(&config.bash_path);
        let stem = std::path::Path::new(&config.bash_path)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        if stem.contains("powershell") || stem == "pwsh" {
            command.args(["-NoProfile", "-NonInteractive", "-Command", &args.command]);
        } else {
            command.args(["-c", &args.command]);
        }
        command
            .current_dir(cwd)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for name in [
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
            "TERM",
            "COLORTERM",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONUTF8", "1")
            .envs(&config.bash.environment);
        crate::execution::memory_limit(&mut command, &config.bash);
        let id = format!("proc-{}", uuid::Uuid::new_v4());
        {
            let mut processes = self
                .processes
                .lock()
                .map_err(|_| "Process registry is unavailable.")?;
            let mut active = 0;
            let mut completed = Vec::new();
            for (id, process) in processes.iter() {
                let output = process
                    .output
                    .lock()
                    .map_err(|_| "Process output is unavailable.")?;
                if output.phase == "running" {
                    if process.owner == owner {
                        active += 1;
                    }
                } else {
                    completed.push((id.clone(), process.created));
                }
            }
            if active >= MAX_ACTIVE_PER_OWNER {
                return Err(
                    "At most eight active processes per conversation; stop one first.".into(),
                );
            }
            completed.sort_by_key(|(_, created)| *created);
            for (id, _) in completed {
                if processes.len() < MAX_PROCESSES {
                    break;
                }
                processes.remove(&id);
            }
            if processes.len() >= MAX_PROCESSES {
                return Err("Process registry is full; stop a running process first.".into());
            }
            let started_at = std::time::Instant::now();
            let mut child = crate::execution::managed_spawn(command)
                .map_err(|error| format!("Process launch failed: {error}"))?;
            let stdin = child.stdin().take().ok_or("Missing process input pipe.")?;
            let stdout = child
                .stdout()
                .take()
                .ok_or("Missing process output pipe.")?;
            let stderr = child.stderr().take().ok_or("Missing process error pipe.")?;
            let output = Arc::new(Mutex::new(Output {
                started_at,
                ..Output::default()
            }));
            let out_reader = tokio::spawn(read_output(stdout, "stdout", output.clone()));
            let err_reader = tokio::spawn(read_output(stderr, "stderr", output.clone()));
            let (control, receiver) = mpsc::channel(8);
            processes.insert(
                id.clone(),
                Arc::new(Process {
                    owner: owner.into(),
                    workspace: workspace.into(),
                    created: std::time::Instant::now(),
                    output: output.clone(),
                    control,
                }),
            );
            tokio::spawn(supervise(
                child, stdin, receiver, output, out_reader, err_reader, timeout, completion,
            ));
        }
        let mut pending = PendingStart {
            registry: self,
            id: id.clone(),
            armed: true,
        };
        if !args.stdin.is_empty() || args.close_stdin {
            if let Err(error) = self
                .write(
                    owner,
                    workspace,
                    json!({"id":id,"input":args.stdin,"close_stdin":args.close_stdin}),
                )
                .await
            {
                return Err(format!("Process launched but input setup failed ({error}); cleanup was requested and side effects are unknown."));
            }
        }
        if yield_ms > 0 {
            tokio::time::sleep(Duration::from_millis(yield_ms)).await;
        }
        let value = self.snapshot(owner, workspace, &id, 0, 65536)?;
        pending.armed = false;
        Ok(value)
    }

    fn snapshot(
        &self,
        owner: &str,
        workspace: &str,
        id: &str,
        cursor: u64,
        max_bytes: usize,
    ) -> Result<Value, String> {
        let process = self.owned(owner, workspace, id)?;
        let output = process
            .output
            .lock()
            .map_err(|_| "Process output is unavailable.")?;
        output.poll(id, cursor, max_bytes)
    }

    /// A consistent bounded snapshot for native check/report adapters. It
    /// exposes only an already-owned process and never increases retention.
    ///
    /// The stop request travels on the same bounded queue as `Control::Write`.
    /// When that queue is full the request cannot be delivered, and dropping
    /// the error silently would leave a process running with no acknowledgement.
    /// The failure is therefore reported on the process itself, with the same
    /// wording `ProcessRegistry::stop` uses.
    pub(crate) fn request_stop(&self, owner: &str, workspace: &str, id: &str) {
        if let Ok(process) = self.owned(owner, workspace, id) {
            let (reply, _) = oneshot::channel();
            if process.control.try_send(Control::Stop { reply }).is_err() {
                if let Ok(mut state) = process.output.lock() {
                    state.phase = "unknown";
                    state.error = Some(
                        "Process control queue is unavailable; stop was not acknowledged.".into(),
                    );
                }
            }
        }
    }
    pub(crate) fn retained_output(
        &self,
        owner: &str,
        workspace: &str,
        id: &str,
    ) -> Result<Value, String> {
        self.snapshot(owner, workspace, id, 0, OUTPUT_BYTES)
    }

    pub async fn poll(&self, owner: &str, workspace: &str, args: Value) -> Result<Value, String> {
        let args: PollArgs = serde_json::from_value(args).map_err(|error| error.to_string())?;
        let limit = args.max_bytes.unwrap_or(65536);
        let wait = args.wait_ms.unwrap_or(0);
        if !(8192..=65536).contains(&limit) || wait > 1000 {
            return Err("Process max_bytes must be 8192–65536 and wait_ms 0–1000.".into());
        }
        self.owned(owner, workspace, &args.id)?;
        if wait > 0 {
            tokio::time::sleep(Duration::from_millis(wait)).await;
        }
        self.snapshot(owner, workspace, &args.id, args.cursor.unwrap_or(0), limit)
    }

    pub async fn write(&self, owner: &str, workspace: &str, args: Value) -> Result<Value, String> {
        let args: WriteArgs = serde_json::from_value(args).map_err(|error| error.to_string())?;
        if args.input.len() > 65536 {
            return Err("Process input is limited to 64 KiB.".into());
        }
        let process = self.owned(owner, workspace, &args.id)?;
        let (reply, received) = oneshot::channel();
        process
            .control
            .try_send(Control::Write {
                input: args.input.into_bytes(),
                close: args.close_stdin,
                reply,
            })
            .map_err(|_| "Process exited or its input queue is full.")?;
        tokio::time::timeout(Duration::from_secs(3), received).await.map_err(|_| "Process input timed out; bytes may have been written.")?
            .map_err(|_| "Process exited before acknowledging input; bytes may have been written and side effects are unknown.")??;
        Ok(json!({"id":args.id,"written":true,"stdinClosed":args.close_stdin}))
    }

    /// Synchronous shell front-end over start+poll: same spawn path, approval
    /// class and ownership as process_start. The wait budget is separate from
    /// the supervisor kill timeout, which stays at the configured shell limit
    /// so a long-running command keeps its lifecycle handles on timeout.
    pub async fn shell_run(
        &self,
        owner: &str,
        workspace: &str,
        config: crate::execution::ExecutionConfig,
        args: Value,
    ) -> Result<Value, String> {
        let args: ShellRunArgs = serde_json::from_value(args).map_err(|error| error.to_string())?;
        let wait_secs = args.timeout_secs.unwrap_or(120);
        if !(1..=600).contains(&wait_secs) {
            return Err("shell_run timeout_secs must be 1–600.".into());
        }
        let mut start_args = json!({"command":args.command,"yield_ms":0,"timeout_seconds":config.bash.timeout_seconds,"close_stdin":true});
        if let Some(cwd) = args.cwd {
            start_args["cwd"] = json!(cwd);
        }
        let started = std::time::Instant::now();
        let launch = self.start(owner, workspace, config, start_args).await?;
        let id = launch["id"]
            .as_str()
            .ok_or("Process start did not return an id.")?
            .to_string();
        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut truncated = false;
        let mut cursor = 0u64;
        let deadline = std::time::Instant::now() + Duration::from_secs(wait_secs);
        let mut state = String::from("running");
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let wait_ms = remaining.as_millis().min(1000) as u64;
            let page = self
                .poll(
                    owner,
                    workspace,
                    json!({"id":id,"cursor":cursor,"max_bytes":65536,"wait_ms":wait_ms}),
                )
                .await?;
            for frame in page["output"].as_array().into_iter().flatten() {
                let sink = if frame["stream"] == "stderr" {
                    &mut stderr
                } else {
                    &mut stdout
                };
                if sink.len() < OUTPUT_BYTES {
                    sink.push_str(frame["text"].as_str().unwrap_or(""));
                } else {
                    truncated = true;
                }
            }
            cursor = page["nextCursor"].as_u64().unwrap_or(cursor);
            truncated = truncated || page["truncated"] == true;
            state = page["state"].as_str().unwrap_or("unknown").to_string();
            if state != "running" {
                break;
            }
        }
        let duration_ms = started.elapsed().as_millis() as u64;
        if state == "running" {
            return Ok(
                json!({"status":"running","processId":id,"stdout":stdout,"stderr":stderr,"truncated":truncated,"durationMs":duration_ms,
                "message":"Still running; continue with process_poll or stop it with process_stop."}),
            );
        }
        let exit_code = self.snapshot(owner, workspace, &id, 0, 8192)?["exitCode"].clone();
        Ok(
            json!({"status":state,"processId":id,"exitCode":exit_code,"stdout":stdout,"stderr":stderr,"truncated":truncated,"durationMs":duration_ms,
            "isError":exit_code.as_i64() != Some(0) || state != "exited"}),
        )
    }

    pub async fn stop(&self, owner: &str, workspace: &str, args: Value) -> Result<Value, String> {
        let args: StopArgs = serde_json::from_value(args).map_err(|error| error.to_string())?;
        let process = self.owned(owner, workspace, &args.id)?;
        let running = process
            .output
            .lock()
            .map_err(|_| "Process output is unavailable.")?
            .phase
            == "running";
        if running {
            let (reply, received) = oneshot::channel();
            if process.control.try_send(Control::Stop { reply }).is_ok() {
                tokio::time::timeout(Duration::from_secs(5), received)
                    .await
                    .map_err(|_| "Process stop timed out; its outcome is unknown.")?
                    .map_err(|_| "Process supervisor stopped without a final acknowledgement.")?;
                // The acknowledgement fires as soon as the tree is terminated,
                // but the supervisor seals `phase` only after draining output
                // and finishing observation. A snapshot taken between the two
                // would report a just-stopped process as still "running".
                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                loop {
                    let settled = process
                        .output
                        .lock()
                        .map(|state| state.phase != "running")
                        .unwrap_or(true);
                    if settled || std::time::Instant::now() >= deadline {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            } else if process
                .output
                .lock()
                .map_err(|_| "Process output is unavailable.")?
                .phase
                == "running"
            {
                return Err(
                    "Process control queue is unavailable; stop was not acknowledged.".into(),
                );
            }
        }
        self.snapshot(owner, workspace, &args.id, 0, 65536)
    }
}

async fn read_output(
    mut reader: impl AsyncRead + Unpin,
    stream: &'static str,
    output: Arc<Mutex<Output>>,
) {
    let mut bytes = [0; 4096];
    let mut pending = Vec::new();
    loop {
        match reader.read(&mut bytes).await {
            Ok(count) => {
                let text = crate::terminal_ui::decode(&mut pending, &bytes[..count], count == 0);
                if let Ok(mut state) = output.lock() {
                    state.push(stream, text);
                }
                if count == 0 {
                    break;
                }
            }
            Err(error) => {
                if let Ok(mut state) = output.lock() {
                    state.error = Some(format!("Could not read {stream}: {error}"));
                    state.incomplete = true;
                }
                break;
            }
        }
    }
}

async fn terminate(child: &mut crate::execution::ExecutionChild) -> Result<Option<i32>, String> {
    let kill_error = child.start_kill().err();
    let status = tokio::time::timeout(Duration::from_secs(3), child.wait())
        .await
        .map_err(|_| {
            "Process termination was not confirmed within three seconds; outcome is unknown."
                .to_string()
        })?
        .map_err(|error| format!("Process termination could not be confirmed: {error}"))?;
    if let Some(error) = kill_error {
        return Err(format!(
            "Process leader exited, but terminating its process tree failed: {error}"
        ));
    }
    Ok(status.code())
}

#[allow(clippy::too_many_arguments)]
async fn supervise(
    mut child: crate::execution::ExecutionChild,
    stdin: tokio::process::ChildStdin,
    mut receiver: mpsc::Receiver<Control>,
    output: Arc<Mutex<Output>>,
    out_reader: tokio::task::JoinHandle<()>,
    err_reader: tokio::task::JoinHandle<()>,
    timeout: u64,
    completion: Option<crate::verification_observer::Completion>,
) {
    let mut stdin = Some(stdin);
    let deadline = tokio::time::sleep(Duration::from_secs(timeout));
    tokio::pin!(deadline);
    let (phase, exit_code, error) = loop {
        if receiver.is_closed() {
            break match terminate(&mut child).await {
                Ok(code) => (
                    "stopped",
                    code,
                    Some("Process owner closed; process tree stopped.".into()),
                ),
                Err(error) => ("unknown", None, Some(error)),
            };
        }
        tokio::select! {
            status = child.wait() => break match status {
                Ok(status) => ("exited", status.code(), if status.success() { None } else { Some(format!("Process exited with status {status}.")) }),
                Err(error) => ("failed", None, Some(format!("Process wait failed: {error}"))),
            },
            _ = &mut deadline => break match terminate(&mut child).await {
                Ok(code) => ("timed_out", code, Some("Configured process timeout exceeded; process tree stopped.".into())),
                Err(error) => ("unknown", None, Some(error)),
            },
            control = receiver.recv() => match control {
                Some(Control::Write { input, close, reply }) => {
                    let result = if let Some(pipe) = stdin.as_mut() {
                        tokio::time::timeout(Duration::from_secs(2), async { pipe.write_all(&input).await?; pipe.flush().await }).await
                            .map_err(|_| "Process input timed out; bytes may have been written.".into())
                            .and_then(|result| result.map_err(|error| format!("Process input failed: {error}")))
                    } else { Err("Process stdin is already closed.".into()) };
                    if close { stdin.take(); }
                    let _ = reply.send(result);
                },
                Some(Control::Stop { reply }) => {
                    // The stop is acknowledged as soon as the process tree is
                    // terminated. Sealing the observation (a full recursive
                    // workspace walk, a barrier poll and a database write)
                    // happens afterwards and must not delay the caller: the
                    // stop budget is five seconds.
                    let outcome = terminate(&mut child).await;
                    let _ = reply.send(());
                    break match outcome {
                        Ok(code) => ("stopped", code, Some("Process tree stopped by request; side effects may already have occurred.".into())),
                        Err(error) => ("unknown", None, Some(error)),
                    };
                },
                None => break match terminate(&mut child).await {
                    Ok(code) => ("stopped", code, Some("Process owner closed; process tree stopped.".into())),
                    Err(error) => ("unknown", None, Some(error)),
                },
            }
        }
    };
    if let Ok(mut state) = output.lock() {
        state.duration_ms = Some(state.started_at.elapsed().as_secs_f64() * 1000.);
    }
    drop(stdin);
    drop(child); // Stop any surviving descendants before sealing observation.
    if let Some(completion) = completion {
        completion.finish(phase).await;
    }
    let mut out_reader = out_reader;
    let mut err_reader = err_reader;
    if tokio::time::timeout(Duration::from_secs(1), async {
        let _ = (&mut out_reader).await;
        let _ = (&mut err_reader).await;
    })
    .await
    .is_err()
    {
        out_reader.abort();
        err_reader.abort();
        if let Ok(mut state) = output.lock() {
            state.incomplete = true;
            state.error = Some("Process output could not be fully drained.".into());
        }
    }
    if let Ok(mut state) = output.lock() {
        state.phase = phase;
        state.exit_code = exit_code;
        if error.is_some() {
            state.error = error;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_stop_is_acknowledged_before_the_observation_is_sealed() {
        // Sealing an observation walks the workspace, polls a watcher barrier
        // and writes to the database. The stop must be acknowledged as soon
        // as the process tree is terminated, not after that work.
        let watched = tempfile::tempdir().unwrap();
        let sealed = tempfile::tempdir().unwrap();
        let pending =
            crate::verification_observer::Registry::begin(watched.path().to_str().unwrap())
                .unwrap();
        let completion = crate::verification_observer::Completion {
            pending,
            // The sealing root is not the watched directory, so the barrier
            // marker is never observed and the seal takes its full poll
            // budget.
            root: sealed.path().to_string_lossy().into_owned(),
            sources: json!({}),
            membership: None,
            db: sealed.path().join("observation.sqlite"),
            owner: "owner".into(),
            run: "run".into(),
            id: "observation".into(),
        };
        let directory = tempfile::tempdir().unwrap();
        let registry = ProcessRegistry::default();
        let sleeper = if cfg!(windows) {
            "Start-Sleep -Seconds 30"
        } else {
            "sleep 30"
        };
        let started = registry
            .start_observed(
                "owner",
                directory.path().to_str().unwrap(),
                crate::execution::ExecutionConfig::default(),
                json!({"command":sleeper,"yield_ms":0}),
                Some(completion),
            )
            .await
            .unwrap();
        let id = started["id"].as_str().unwrap().to_string();
        let process = registry
            .owned("owner", directory.path().to_str().unwrap(), &id)
            .unwrap();
        let (reply, received) = oneshot::channel();
        process
            .control
            .send(Control::Stop { reply })
            .await
            .expect("the stop request is queued");
        let acknowledged = std::time::Instant::now();
        tokio::time::timeout(Duration::from_secs(5), received)
            .await
            .expect("the stop is acknowledged")
            .expect("the supervisor acknowledges the stop");
        assert!(
            acknowledged.elapsed() < Duration::from_millis(900),
            "the stop must be acknowledged before the observation is sealed: {:?}",
            acknowledged.elapsed()
        );
    }

    #[tokio::test]
    async fn a_full_control_queue_reports_the_dropped_stop_instead_of_discarding_it() {
        let (control, _receiver) = mpsc::channel::<Control>(1);
        // The queue is full: the stop request cannot be delivered.
        let (reply, _) = oneshot::channel();
        control
            .try_send(Control::Write {
                input: Vec::new(),
                close: false,
                reply,
            })
            .expect("the write fills the queue");
        let output = Arc::new(Mutex::new(Output::default()));
        let process = Arc::new(Process {
            owner: "owner".into(),
            workspace: "root".into(),
            created: std::time::Instant::now(),
            output: output.clone(),
            control,
        });
        let registry = ProcessRegistry::default();
        registry
            .processes
            .lock()
            .unwrap()
            .insert("proc-fixture".into(), process);
        registry.request_stop("owner", "root", "proc-fixture");
        let state = output.lock().unwrap();
        assert_eq!(state.phase, "unknown");
        assert!(
            state
                .error
                .as_deref()
                .unwrap_or_default()
                .contains("Process control queue is unavailable; stop was not acknowledged."),
            "{:?}",
            state.error
        );
    }

    #[test]
    fn process_tools_are_registered_scoped_and_strict() {
        for spec in registry() {
            let alias = &spec.name;
            let tool = crate::connectors::AgentTool::harness(alias).unwrap();
            assert!(tool.is_harness());
            assert_eq!(tool.tool.input_schema["additionalProperties"], false);
            assert_eq!(
                crate::capabilities::harness_alias_capability(alias),
                Some("local_execution")
            );
            assert!(!crate::presets::harness_source_allowed(alias, true, false));
            assert!(crate::presets::harness_source_allowed(alias, false, true));
            for preset in [
                crate::presets::STANDARD,
                crate::presets::CODE,
                crate::presets::CODING,
                crate::presets::CREATOR,
            ] {
                assert!(
                    crate::presets::get(preset).unwrap().harness.contains(alias),
                    "{preset}: {alias}"
                );
            }
            assert!(tool
                .prepare_arguments(json!({"id":"proc-fixture","unexpected":true}))
                .is_err());
        }
        assert!(crate::harness::is_trusted_read("process_poll"));
        for alias in ["process_start", "process_write", "process_stop"] {
            assert!(!crate::harness::is_trusted_read(alias));
        }
        let polls = vec![("process_poll".to_string(), "same-owned-process".to_string()); 4];
        assert!(
            crate::sandbox::check_repetition(&polls, "process_poll", "same-owned-process")
                .is_none()
        );
    }

    async fn harness_call(
        ctx: &crate::harness::HarnessCtx<'_>,
        alias: &str,
        args: Value,
    ) -> Result<Value, String> {
        let mut calls = crate::tool_calls::ToolCalls::default();
        calls.push(&json!({"tool_calls":[{"index":0,"id":"call-process-fixture","type":"function","function":{"name":alias,"arguments":args.to_string()}}]}))?;
        let call = calls.finish()?.remove(0);
        let tool = ctx
            .inherit_tools
            .iter()
            .find(|tool| tool.alias == call.name)
            .ok_or("Fixture tool is not offered.")?;
        let args = tool.prepare_arguments(call.arguments)?;
        Ok(crate::harness::execute(ctx, &call.name, args).await?.value)
    }

    #[tokio::test]
    async fn model_call_schema_dispatch_process_roundtrip_and_stale_capability_denial() {
        let directory = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(directory.path());
        let owner = state.database().unwrap().create_conversation().unwrap().id;
        let aliases = [
            "process_start",
            "process_poll",
            "process_write",
            "process_stop",
        ];
        let tools = aliases
            .iter()
            .map(|alias| crate::connectors::AgentTool::harness(alias).unwrap())
            .collect::<Vec<_>>();
        let ctx = crate::harness::HarnessCtx {
            state: &state,
            snapshot: crate::test_support::snapshot(&state, directory.path(), &aliases),
            conversation_id: owner,
            run_id: "fixture-run".into(),
            step_id: "fixture-step".into(),
            access_mode: crate::permissions::AccessMode::FullAccess,
            depth: 0,
            preset_id: crate::presets::STANDARD.into(),
            inherit_tools: &tools,
            emit: Arc::new(|_, _| {}),
        };
        let command = if cfg!(windows) {
            "$line=[Console]::ReadLine(); Write-Output $line"
        } else {
            "read -r line; printf '%s\\n' \"$line\""
        };
        let started = harness_call(
            &ctx,
            "process_start",
            json!({"command":command,"yield_ms":0}),
        )
        .await
        .unwrap();
        let id = started["id"].as_str().unwrap();
        harness_call(
            &ctx,
            "process_write",
            json!({"id":id,"input":"model fixture output\n","close_stdin":true}),
        )
        .await
        .unwrap();
        let mut result = Value::Null;
        for _ in 0..100 {
            result = harness_call(&ctx, "process_poll", json!({"id":id,"wait_ms":20}))
                .await
                .unwrap();
            if result["state"] != "running" {
                break;
            }
        }
        assert_eq!(result["state"], "exited", "{result}");
        assert_eq!(result["exitCode"], 0, "{result}");
        assert!(result["output"]
            .to_string()
            .contains("model fixture output"));
        assert!(harness_call(
            &ctx,
            "process_start",
            json!({"command":"anything","extra":true})
        )
        .await
        .is_err());
        state
            .database()
            .unwrap()
            .save_setting(crate::capabilities::DISABLED_KEY, &vec!["local_execution"])
            .unwrap();
        let error = harness_call(
            &ctx,
            "process_start",
            json!({"command":"echo denied > denied.txt"}),
        )
        .await
        .unwrap_err();
        assert!(error.contains("disabled"), "{error}");
        assert!(!directory.path().join("denied.txt").exists());
        assert!(harness_call(&ctx, "process_poll", json!({"id":id}))
            .await
            .unwrap_err()
            .contains("disabled"));
    }

    #[tokio::test]
    async fn real_catalog_assembly_preserves_process_approval_and_source_selection() {
        let directory = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(directory.path());
        crate::test_support::select_fixture_model(&state, directory.path());
        let mut config = state.database().unwrap().execution_config().unwrap();
        config.bash.always_ask = true;
        state
            .database()
            .unwrap()
            .save_execution_config(&config)
            .unwrap();
        let plan = crate::chat::assemble_turn(
            &state,
            "",
            "Run a command",
            vec!["__execution".into()],
            Some(vec![]),
            Some(crate::presets::STANDARD),
            None,
        )
        .await
        .unwrap();
        for alias in ["process_start", "process_write", "process_stop"] {
            let tool = plan.tools.iter().find(|tool| tool.alias == alias).unwrap();
            assert!(tool.requires_approval(&json!({})), "{alias}");
        }
        let poll = plan
            .tools
            .iter()
            .find(|tool| tool.alias == "process_poll")
            .unwrap();
        assert!(poll.trusted_read());
        assert!(!poll.requires_approval(&json!({})));
        let without_source = crate::chat::assemble_turn(
            &state,
            "",
            "Run a command",
            vec![],
            Some(vec![]),
            Some(crate::presets::STANDARD),
            None,
        )
        .await
        .unwrap();
        assert!(!without_source
            .tools
            .iter()
            .any(|tool| tool.alias.starts_with("process_")));
        state
            .database()
            .unwrap()
            .save_setting(crate::capabilities::DISABLED_KEY, &vec!["local_execution"])
            .unwrap();
        let disabled = crate::chat::assemble_turn(
            &state,
            "",
            "Run a command",
            vec!["__execution".into()],
            Some(vec![]),
            Some(crate::presets::STANDARD),
            None,
        )
        .await
        .unwrap();
        assert!(!disabled
            .tools
            .iter()
            .any(|tool| tool.alias.starts_with("process_")));
        assert!(disabled
            .tool_notice
            .as_deref()
            .unwrap_or("")
            .contains("disabled"));
        state
            .database()
            .unwrap()
            .save_setting(crate::capabilities::DISABLED_KEY, &Vec::<String>::new())
            .unwrap();
        config.bash.enabled = false;
        state
            .database()
            .unwrap()
            .save_execution_config(&config)
            .unwrap();
        let unavailable = crate::chat::assemble_turn(
            &state,
            "",
            "Run a command",
            vec!["__execution".into()],
            Some(vec![]),
            Some(crate::presets::STANDARD),
            None,
        )
        .await
        .unwrap();
        assert!(!unavailable
            .tools
            .iter()
            .any(|tool| tool.alias.starts_with("process_")));
    }

    #[tokio::test]
    async fn real_advanced_catalogs_offer_their_intended_surfaces() {
        let directory = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(directory.path());
        crate::test_support::select_fixture_model(&state, directory.path());
        for (preset, expected) in [
            (crate::presets::CODE, "ptc_run"),
            (crate::presets::CREATOR, "workflow_run"),
            (crate::presets::MINIMAL, "workspace_edit_file"),
        ] {
            let plan = crate::chat::assemble_turn(
                &state,
                "",
                "Profile fixture",
                vec!["__workspace".into(), "__execution".into()],
                Some(vec![]),
                Some(preset),
                None,
            )
            .await
            .unwrap();
            assert!(
                plan.tools.iter().any(|tool| tool.alias == expected),
                "{preset} must offer {expected}"
            );
            if preset == crate::presets::MINIMAL {
                assert!(
                    !plan.tools.iter().any(|tool| matches!(
                        tool.alias.as_str(),
                        "local_run_code" | "local_execute_command" | "system_time"
                    )),
                    "Minimal must keep its persistent-shell benchmark surface"
                );
                assert_eq!(
                    plan.tools
                        .iter()
                        .filter(|tool| tool.connector == "Workspace")
                        .map(|tool| tool.alias.as_str())
                        .collect::<Vec<_>>(),
                    vec!["workspace_edit_file"]
                );
            }
        }
    }

    async fn finished(registry: &ProcessRegistry, root: &str, id: &str) -> Value {
        for _ in 0..100 {
            let result = registry
                .poll("owner", root, json!({"id":id,"wait_ms":20}))
                .await
                .unwrap();
            if result["state"] != "running" {
                return result;
            }
        }
        panic!("Process did not finish promptly")
    }

    #[tokio::test]
    async fn shell_run_returns_exit_output_and_a_live_process_id_on_timeout() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_str().unwrap();
        let registry = ProcessRegistry::default();
        let config = crate::execution::ExecutionConfig::default();
        let ok = registry
            .execute(
                "owner",
                root,
                config.clone(),
                "shell_run",
                json!({"command":"printf 'out'; printf 'err' >&2","timeout_secs":10}),
            )
            .await
            .unwrap();
        assert_eq!(ok["exitCode"], 0, "{ok}");
        assert_eq!(ok["status"], "exited");
        assert_eq!(ok["stdout"], "out");
        assert_eq!(ok["stderr"], "err");
        assert_eq!(ok["isError"], false);
        let failed = registry
            .execute(
                "owner",
                root,
                config.clone(),
                "shell_run",
                json!({"command":"exit 3","timeout_secs":10}),
            )
            .await
            .unwrap();
        assert_eq!(failed["exitCode"], 3, "{failed}");
        assert_eq!(failed["isError"], true);
        // On timeout the process keeps running under its returned id.
        let sleeper = if cfg!(windows) {
            "Start-Sleep -Seconds 30"
        } else {
            "sleep 30"
        };
        let running = registry
            .execute(
                "owner",
                root,
                config.clone(),
                "shell_run",
                json!({"command":sleeper,"timeout_secs":1}),
            )
            .await
            .unwrap();
        assert_eq!(running["status"], "running", "{running}");
        let id = running["processId"].as_str().unwrap();
        assert!(registry.poll("owner", root, json!({"id":id})).await.is_ok());
        registry
            .stop("owner", root, json!({"id":id}))
            .await
            .unwrap();
        for args in [
            json!({"command":"echo hi","timeout_secs":0}),
            json!({"command":"echo hi","timeout_secs":601}),
            json!({"command":"echo hi","unexpected":1}),
        ] {
            assert!(
                registry
                    .execute("owner", root, config.clone(), "shell_run", args.clone())
                    .await
                    .is_err(),
                "{args}"
            );
        }
        // Same approval family as process_start: never a trusted read.
        assert!(!crate::harness::is_trusted_read("shell_run"));
        assert_eq!(
            crate::capabilities::harness_alias_capability("shell_run"),
            Some("local_execution")
        );
    }

    #[tokio::test]
    async fn asynchronous_process_output_stdin_exit_and_ownership() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().to_str().unwrap();
        let registry = ProcessRegistry::default();
        let command = if cfg!(windows) {
            "$line=[Console]::ReadLine(); Write-Output ('hello '+$line); [Console]::Error.WriteLine('error-line'); exit 7"
        } else {
            "read -r line; printf 'hello %s\\n' \"$line\"; printf 'error-line\\n' >&2; exit 7"
        };
        let started = registry
            .start(
                "owner",
                root,
                crate::execution::ExecutionConfig::default(),
                json!({"command":command,"yield_ms":0}),
            )
            .await
            .unwrap();
        let id = started["id"].as_str().unwrap();
        assert_eq!(started["state"], "running");
        assert!(registry
            .poll("other", root, json!({"id":id}))
            .await
            .is_err());
        assert!(registry
            .write(
                "owner",
                "/another-workspace",
                json!({"id":id,"input":"wrong"})
            )
            .await
            .is_err());
        registry
            .write(
                "owner",
                root,
                json!({"id":id,"input":"world\n","close_stdin":true}),
            )
            .await
            .unwrap();
        let result = finished(&registry, root, id).await;
        assert_eq!(result["exitCode"], 7, "{result}");
        assert!(
            result["output"].to_string().contains("hello world"),
            "{result}"
        );
        assert!(
            result["output"].to_string().contains("error-line"),
            "{result}"
        );
        let cursor = result["nextCursor"].as_u64().unwrap();
        assert_eq!(
            registry
                .poll("owner", root, json!({"id":id,"cursor":cursor}))
                .await
                .unwrap()["output"],
            json!([])
        );
        assert!(registry
            .poll("owner", root, json!({"id":id,"cursor":cursor+1}))
            .await
            .is_err());
    }

    #[tokio::test]
    async fn asynchronous_process_timeout_stop_and_invalid_requests() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().to_str().unwrap();
        let registry = ProcessRegistry::default();
        let command = if cfg!(windows) {
            "Start-Sleep -Seconds 30"
        } else {
            "sleep 30"
        };
        let config = crate::execution::ExecutionConfig::default();
        let started = registry
            .start(
                "owner",
                root,
                config.clone(),
                json!({"command":command,"yield_ms":0,"timeout_seconds":1}),
            )
            .await
            .unwrap();
        let result = finished(&registry, root, started["id"].as_str().unwrap()).await;
        assert_eq!(result["state"], "timed_out");
        let started = registry
            .start(
                "owner",
                root,
                config.clone(),
                json!({"command":command,"yield_ms":0}),
            )
            .await
            .unwrap();
        let stopped = registry
            .stop("owner", root, json!({"id":started["id"]}))
            .await
            .unwrap();
        assert_eq!(stopped["state"], "stopped");
        assert_eq!(stopped["isError"], false, "{stopped}");
        assert!(stopped["error"].is_null(), "{stopped}");
        for args in [
            json!({"command":""}),
            json!({"command":command,"yield_ms":1001}),
            json!({"command":command,"cwd":"../"}),
            json!({"command":command,"unexpected":true}),
            json!({"command":command,"timeout_seconds":301}),
        ] {
            assert!(
                registry
                    .start("owner", root, config.clone(), args.clone())
                    .await
                    .is_err(),
                "{args}"
            );
        }
        let mut disabled = config;
        disabled.bash.enabled = false;
        assert!(registry
            .start("owner", root, disabled, json!({"command":command}))
            .await
            .is_err());
    }

    #[test]
    fn process_output_pagination_reports_loss_and_retains_bounds() {
        let mut output = Output::default();
        for _ in 0..100 {
            output.push("stdout", "x".repeat(4096));
        }
        assert!(output.bytes <= OUTPUT_BYTES);
        let first = output.poll("id", 0, 8192).unwrap();
        assert_eq!(first["truncated"], true);
        assert_eq!(first["output"].as_array().unwrap().len(), 2);
        assert_eq!(first["hasMore"], true);
        let next = output
            .poll("id", first["nextCursor"].as_u64().unwrap(), 8192)
            .unwrap();
        assert_ne!(first["output"][0]["cursor"], next["output"][0]["cursor"]);
    }

    #[cfg(target_os = "linux")]
    async fn descendant_pid(path: &std::path::Path) -> u32 {
        for _ in 0..100 {
            if let Ok(text) = std::fs::read_to_string(path) {
                if let Ok(pid) = text.trim().parse() {
                    return pid;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("Descendant PID was not written")
    }

    #[cfg(target_os = "linux")]
    async fn assert_terminated(pid: u32) {
        for _ in 0..100 {
            match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
                Err(_) => return,
                Ok(stat)
                    if stat
                        .rsplit_once(')')
                        .is_some_and(|(_, state)| state.trim_start().starts_with('Z')) =>
                {
                    return
                }
                _ => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
        panic!("Descendant {pid} survived process cleanup")
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn stopping_and_dropping_registry_terminate_descendants() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_str().unwrap();
        let registry = ProcessRegistry::default();
        let args = json!({"command":"sleep 30 & printf '%s\\n' \"$!\" > descendant.pid; wait","yield_ms":0});
        let started = registry
            .start(
                "owner",
                root,
                crate::execution::ExecutionConfig::default(),
                args.clone(),
            )
            .await
            .unwrap();
        let pid = descendant_pid(&directory.path().join("descendant.pid")).await;
        registry
            .stop("owner", root, json!({"id":started["id"]}))
            .await
            .unwrap();
        assert_terminated(pid).await;
        std::fs::remove_file(directory.path().join("descendant.pid")).unwrap();
        registry
            .start(
                "owner",
                root,
                crate::execution::ExecutionConfig::default(),
                args,
            )
            .await
            .unwrap();
        let pid = descendant_pid(&directory.path().join("descendant.pid")).await;
        drop(registry);
        assert_terminated(pid).await;
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn cancelling_start_cleans_up_an_undelivered_process() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_str().unwrap().to_string();
        let registry = Arc::new(ProcessRegistry::default());
        let task_registry = registry.clone();
        let task = tokio::spawn(async move {
            task_registry.start("owner", &root, crate::execution::ExecutionConfig::default(),
                json!({"command":"sleep 30 & printf '%s\\n' \"$!\" > descendant.pid; wait","yield_ms":1000})).await
        });
        let pid = descendant_pid(&directory.path().join("descendant.pid")).await;
        task.abort();
        let _ = task.await;
        assert!(registry.processes.lock().unwrap().is_empty());
        assert_terminated(pid).await;
    }

    #[tokio::test]
    async fn active_process_limit_and_initial_stdin_are_enforced() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_str().unwrap();
        let registry = ProcessRegistry::default();
        let command = if cfg!(windows) {
            "[Console]::Out.Write([Console]::In.ReadToEnd())"
        } else {
            "cat"
        };
        let started = registry
            .start(
                "owner",
                root,
                crate::execution::ExecutionConfig::default(),
                json!({"command":command,"stdin":"initial input","close_stdin":true,"yield_ms":0}),
            )
            .await
            .unwrap();
        let result = finished(&registry, root, started["id"].as_str().unwrap()).await;
        assert_eq!(result["exitCode"], 0, "{result}");
        assert!(result["output"].to_string().contains("initial input"));
        let command = if cfg!(windows) {
            "Start-Sleep -Seconds 30"
        } else {
            "sleep 30"
        };
        let mut ids = Vec::new();
        for _ in 0..MAX_ACTIVE_PER_OWNER {
            ids.push(
                registry
                    .start(
                        "owner",
                        root,
                        crate::execution::ExecutionConfig::default(),
                        json!({"command":command,"yield_ms":0}),
                    )
                    .await
                    .unwrap()["id"]
                    .clone(),
            );
        }
        assert!(registry
            .start(
                "owner",
                root,
                crate::execution::ExecutionConfig::default(),
                json!({"command":command,"yield_ms":0})
            )
            .await
            .unwrap_err()
            .contains("eight"));
        for id in ids {
            registry
                .stop("owner", root, json!({"id":id}))
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn asynchronous_process_runs_a_real_local_http_server_and_stops_it() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_str().unwrap();
        let config = crate::execution::ExecutionConfig::default();
        let script = "from http.server import BaseHTTPRequestHandler, HTTPServer\nclass Handler(BaseHTTPRequestHandler):\n def do_GET(self):\n  self.send_response(200); self.end_headers(); self.wfile.write(b'process fixture ok')\nserver = HTTPServer(('127.0.0.1', 0), Handler)\nprint(server.server_port, flush=True)\nserver.serve_forever()\n";
        std::fs::write(directory.path().join("server_fixture.py"), script).unwrap();
        let interpreter = if cfg!(windows) {
            format!("& '{}'", config.python_path.replace('\'', "''"))
        } else {
            format!("'{}'", config.python_path.replace('\'', "'\\''"))
        };
        let registry = ProcessRegistry::default();
        let started = registry
            .execute(
                "owner",
                root,
                config.clone(),
                "process_start",
                json!({"command":format!("{interpreter} -u server_fixture.py"),"yield_ms":0}),
            )
            .await
            .unwrap();
        let id = started["id"].as_str().unwrap();
        let mut port = None;
        for _ in 0..100 {
            let output = registry
                .poll("owner", root, json!({"id":id,"wait_ms":20}))
                .await
                .unwrap();
            for frame in output["output"].as_array().unwrap() {
                if frame["stream"] == "stdout" {
                    if let Ok(value) = frame["text"].as_str().unwrap_or("").trim().parse::<u16>() {
                        port = Some(value);
                    }
                }
            }
            if port.is_some() {
                break;
            }
            assert_eq!(output["state"], "running", "{output}");
        }
        let url = format!(
            "http://127.0.0.1:{}/",
            port.expect("Server must publish its allocated port")
        );
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap();
        assert_eq!(
            client.get(&url).send().await.unwrap().text().await.unwrap(),
            "process fixture ok"
        );
        let stopped = registry
            .execute("owner", root, config, "process_stop", json!({"id":id}))
            .await
            .unwrap();
        assert_eq!(stopped["state"], "stopped", "{stopped}");
        assert!(
            client.get(&url).send().await.is_err(),
            "Server survived process_stop"
        );
    }

    #[tokio::test]
    async fn asynchronous_process_completes_a_real_build_and_runs_its_output() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_str().unwrap();
        std::fs::write(
            directory.path().join("main.rs"),
            "fn main() { println!(\"built process fixture\"); }",
        )
        .unwrap();
        let command = if cfg!(windows) {
            "rustc main.rs -o built-fixture.exe; if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }; & './built-fixture.exe'"
        } else {
            "rustc main.rs -o built-fixture && ./built-fixture"
        };
        let registry = ProcessRegistry::default();
        let started = registry
            .start(
                "owner",
                root,
                crate::execution::ExecutionConfig::default(),
                json!({"command":command,"yield_ms":0}),
            )
            .await
            .unwrap();
        let result = finished(&registry, root, started["id"].as_str().unwrap()).await;
        assert_eq!(result["exitCode"], 0, "{result}");
        assert!(
            result["output"]
                .to_string()
                .contains("built process fixture"),
            "{result}"
        );
    }
}
