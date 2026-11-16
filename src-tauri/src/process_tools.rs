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
    sync::{mpsc, oneshot, Notify},
};

const MIN_PAGE_BYTES: usize = 8192;
const OUTPUT_BYTES: usize = 256 * 1024;
const MAX_PROCESSES: usize = 64;
const MAX_ACTIVE_PER_OWNER: usize = 8;
const MAX_POLL_WAIT_MS: u64 = 30_000;
const DEFAULT_SHELL_WAIT_SECS: u64 = 120;

pub fn registry() -> Vec<crate::tool_spec::ToolSpec> {
    let schema = |properties, required: &[&str]| json!({"type":"object","properties":properties,"required":required,"additionalProperties":false});
    let id = json!({"type":"string","minLength":1,"maxLength":128,"description":"The process ID returned by process_start, owned by this conversation and workspace."});
    vec![
        ("process_start".into(), "Start an approved long-running shell command when you need incremental output or later stdin input. Returns a conversation-owned process ID, state and output cursor promptly. Use process_poll to follow output and process_stop to terminate the process tree. The configured shell permissions, environment, memory limit and timeout apply; this is not OS isolation.".into(), schema(json!({
            "command":{"type":"string","minLength":1,"maxLength":32768},
            "cwd":{"type":"string","maxLength":4096,"description":"Directory inside the authorized workspace; defaults to the configured shell directory."},
            "timeout_seconds":{"type":"integer","minimum":1,"maximum":crate::execution::MAX_MANAGED_SHELL_SECONDS,"description":"Process lifetime in seconds. Reduced to the configured shell timeout when larger. timeout_secs is an alias."},
            "timeout_secs":{"type":"integer","minimum":1,"maximum":crate::execution::MAX_MANAGED_SHELL_SECONDS,"description":"Alias of timeout_seconds."},
            "yield_ms":{"type":"integer","minimum":0,"maximum":1000,"default":100,"description":"Initial wait in milliseconds. Larger values are reduced to 1000."},
            "stdin":{"type":"string","maxLength":65536},"close_stdin":{"type":"boolean","default":false}
        }), &["command"])),
        ("process_poll".into(), "Read incremental stdout/stderr and the current state of an owned process. While waiting for a long command, pass wait_ms=30000 and the previous nextCursor to wait for output or exit. Keep following the same process ID until you have its terminal status. hasMore means another page is available; truncated and firstAvailableCursor report output lost from the bounded buffer.".into(), schema(json!({
            "id":id,"cursor":{"type":"integer","minimum":0,"default":0},
            "max_bytes":{"type":"integer","minimum":8192,"maximum":65536,"default":65536},
            "wait_ms":{"type":"integer","minimum":0,"maximum":MAX_POLL_WAIT_MS,"default":0,"description":"Wait for output or exit for up to 30 seconds."}
        }), &["id"])),
        ("process_write".into(), "Write stdin to an owned running process, for example a line ending in newline. Set close_stdin to deliver EOF. Input can cause side effects and follows shell approval settings. A timeout can mean bytes were partially written; do not blindly retry.".into(), schema(json!({
            "id":id,"input":{"type":"string","maxLength":65536},"close_stdin":{"type":"boolean","default":false}
        }), &["id","input"])),
        ("process_stop".into(), "Stop an owned process and its descendants, then return its final state and bounded output. Already completed processes return their stored result. Stopping cannot undo side effects that already occurred.".into(), schema(json!({"id":id}), &["id"])),
        ("shell_run".into(), "Run an approved shell command and wait for exit. Returns a processId and its state. If only the wait elapsed and the process is running, use process_poll or process_stop. When the configured shell lifetime expires the process tree is stopped and the result is timed_out. The configured shell permissions, environment and memory limit apply.".into(), schema(json!({
            "command":{"type":"string","minLength":1,"maxLength":32768},
            "cwd":{"type":"string","maxLength":4096,"description":"Directory inside the authorized workspace; defaults to the configured shell directory."},
            "timeout_secs":{"type":"integer","minimum":1,"maximum":600,"default":DEFAULT_SHELL_WAIT_SECS,"description":"Seconds to wait for exit before returning the live processId. The tool can return sooner to fit the current tool budget. Does not extend the configured shell lifetime. timeout_seconds is an alias."},
            "timeout_seconds":{"type":"integer","minimum":1,"maximum":600,"description":"Alias of timeout_secs."}
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
    timeout_seconds: Option<u64>,
}

/// Models mix the timeout and wait field names. Rewrite to the field the tool
/// accepts, and reduce an over-limit wait to the ceiling, before schema checks.
pub fn align_arguments(name: &str, args: &mut Value) {
    let Some(obj) = args.as_object_mut() else { return };
    match name {
        "process_start" | "check_run" | "verification_run" => {
            copy_field(obj, "timeout_seconds", "timeout_secs");
            clamp_int(obj, "yield_ms", 0, 1000);
        }
        "shell_run" => {
            copy_field(obj, "timeout_secs", "timeout_seconds");
            clamp_int(obj, "timeout_secs", 1, 600);
        }
        "process_poll" => clamp_int(obj, "wait_ms", 0, MAX_POLL_WAIT_MS),
        "check_report" | "verification_report" => clamp_int(obj, "wait_ms", 0, 30_000),
        _ => {}
    }
}

/// Leave time for the waiting tool to return its process handle or report.
/// The policy enforced by the supervisor still owns the process lifetime.
pub(crate) fn limit_wait(name: &str, args: &mut Value, tool_timeout: Duration) {
    let remaining = tool_timeout.saturating_sub(Duration::from_secs(1));
    match name {
        "shell_run" => {
            let wait = args["timeout_secs"].as_u64().unwrap_or(DEFAULT_SHELL_WAIT_SECS);
            args["timeout_secs"] = json!(wait.min(remaining.as_secs().max(1)));
        }
        "process_poll" | "check_report" | "verification_report" => {
            if let Some(wait) = args["wait_ms"].as_u64() {
                args["wait_ms"] = json!(wait.min(remaining.as_millis() as u64));
            }
        }
        _ => {}
    }
}

fn copy_field(obj: &mut serde_json::Map<String, Value>, canonical: &str, alias: &str) {
    if !obj.contains_key(canonical) {
        if let Some(value) = obj.remove(alias) {
            obj.insert(canonical.to_string(), value);
        }
    } else {
        obj.remove(alias);
    }
}

fn clamp_int(obj: &mut serde_json::Map<String, Value>, key: &str, min: u64, max: u64) {
    let Some(value) = obj.get(key).and_then(Value::as_u64) else { return };
    let clamped = value.clamp(min, max);
    if clamped != value {
        obj.insert(key.into(), json!(clamped));
    }
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
    observation_pending: bool,
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
            observation_pending: false,
            started_at: std::time::Instant::now(),
            duration_ms: None,
        }
    }
}

impl Output {
    fn push(&mut self, stream: &'static str, text: String) {
        let mut remaining = text.as_str();
        while !remaining.is_empty() {
            let mut end = remaining.len().min(MIN_PAGE_BYTES);
            while !remaining.is_char_boundary(end) {
                end -= 1;
            }
            let (frame, rest) = remaining.split_at(end);
            self.bytes += frame.len();
            self.frames.push_back(Frame {
                cursor: self.next,
                stream,
                text: frame.to_owned(),
            });
            self.next += 1;
            remaining = rest;
        }
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
        let mut result = json!({"id":id,"state":self.phase,"durationMs":self.duration_ms,"exitCode":self.exit_code,"error":error,"message":message,
            "output":frames,"nextCursor":next,"hasMore":next < self.next,
            "truncated":cursor < base || self.incomplete,"firstAvailableCursor":base,"sideEffects":"possible",
            "isError":self.incomplete || matches!(self.phase, "failed" | "timed_out" | "unknown") || (self.phase == "exited" && self.exit_code.is_some_and(|code| code != 0))});
        if self.phase == "timed_out" {
            result["code"] = json!("timeout");
        }
        if self.observation_pending {
            result["observationPending"] = json!(true);
        }
        Ok(result)
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
    changed: Arc<Notify>,
    control: mpsc::Sender<Control>,
    stop: tokio::sync::watch::Sender<bool>,
    terminated: tokio::sync::watch::Receiver<bool>,
    waiting_run: Mutex<Option<String>>,
}

#[derive(Default)]
pub struct ProcessRegistry {
    processes: Mutex<HashMap<String, Arc<Process>>>,
    #[cfg(test)]
    pub(crate) polls_started: std::sync::atomic::AtomicUsize,
    #[cfg(test)]
    pub(crate) poll_started: Notify,
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

/// Cancelling the wait also cancels the command. A successful return of a
/// running process explicitly transfers control to the returned handle.
struct ShellWait {
    process: Arc<Process>,
    returned: bool,
}

impl Drop for ShellWait {
    fn drop(&mut self) {
        if self.returned {
            if let Ok(mut run) = self.process.waiting_run.lock() {
                *run = None;
            }
        } else {
            self.process.stop.send_replace(true);
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
            for process in processes.values() { process.stop.send_replace(true); }
            processes.clear();
        }
    }
    /// Await process-tree termination separately from output/verification sealing.
    pub async fn shutdown(&self) -> Result<(), String> {
        let terminated = self.processes.lock().map_err(|_| "Process registry is unavailable.".to_string())?
            .values().map(|process| process.terminated.clone()).collect::<Vec<_>>();
        self.stop_all();
        tokio::time::timeout(Duration::from_secs(5), futures_util::future::try_join_all(
            terminated.into_iter().map(|mut watch| async move {
                while !*watch.borrow() {
                    watch.changed().await.map_err(|_| "Process termination was not acknowledged.".to_string())?;
                }
                Ok::<_, String>(())
            }),
        )).await.map_err(|_| "Process shutdown exceeded five seconds; some outcomes are unknown.".to_string())??;
        Ok(())
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
                if let Some(process) = processes.remove(&id) { process.stop.send_replace(true); }
            }
        }
    }

    /// Settle waits abandoned by this run, without touching deliberately
    /// detached commands or another turn's process.
    pub(crate) async fn settle_cancelled_shells(&self, run_id: &str) -> Vec<Value> {
        let waiting = self.processes.lock().map(|processes| {
            processes.iter().filter(|(_, process)| {
                process.waiting_run.lock().is_ok_and(|run| run.as_deref() == Some(run_id))
            }).map(|(id, process)| (id.clone(), process.clone())).collect::<Vec<_>>()
        }).unwrap_or_default();
        futures_util::future::join_all(waiting.into_iter().map(|(id, process)| async move {
            process.stop.send_replace(true);
            let mut terminated = process.terminated.clone();
            let confirmed = tokio::time::timeout(Duration::from_secs(5), async {
                while !*terminated.borrow() {
                    if terminated.changed().await.is_err() { return false; }
                }
                true
            }).await.unwrap_or(false);
            if let Ok(mut run) = process.waiting_run.lock() { *run = None; }
            json!({"processId":id,"processState":if confirmed { "stopped" } else { "unknown" }})
        })).await
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
        mut args: Value,
    ) -> Result<Value, String> {
        align_arguments(alias, &mut args);
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
        mut args: Value,
        completion: Option<crate::verification_observer::Completion>,
    ) -> Result<Value, String> {
        align_arguments("process_start", &mut args);
        let args: StartArgs = serde_json::from_value(args).map_err(|error| error.to_string())?;
        let yield_ms = args.yield_ms.unwrap_or(100).min(1000);
        let lifetime = config.bash.timeout_seconds.min(crate::execution::MAX_MANAGED_SHELL_SECONDS).max(1);
        let requested = args.timeout_seconds.unwrap_or(lifetime);
        let timeout = requested.clamp(1, lifetime);
        if owner.is_empty() || workspace.is_empty() {
            return Err("Process start needs a conversation and workspace.".into());
        }
        if args.command.trim().is_empty() || args.command.len() > 32768 || args.stdin.len() > 65536
        {
            return Err(
                "Process command must contain 1–32768 bytes and stdin at most 64 KiB.".into(),
            );
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
                } else if !output.observation_pending {
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
                observation_pending: completion.is_some(),
                ..Output::default()
            }));
            let changed = Arc::new(Notify::new());
            let out_reader = tokio::spawn(read_output(stdout, "stdout", output.clone(), changed.clone()));
            let err_reader = tokio::spawn(read_output(stderr, "stderr", output.clone(), changed.clone()));
            let (control, receiver) = mpsc::channel(8);
            let (stop, stop_requested) = tokio::sync::watch::channel(false);
            let (terminated, termination) = tokio::sync::watch::channel(false);
            processes.insert(
                id.clone(),
                Arc::new(Process {
                    owner: owner.into(),
                    workspace: workspace.into(),
                    created: std::time::Instant::now(),
                    output: output.clone(),
                    changed: changed.clone(),
                    control,
                    stop,
                    terminated: termination,
                    waiting_run: Mutex::new(None),
                }),
            );
            tokio::spawn(supervise(
                child, stdin, receiver, stop_requested, terminated, output, changed, out_reader, err_reader, timeout, completion,
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
        let mut value = self.snapshot(owner, workspace, &id, 0, 65536)?;
        value["appliedTimeoutSeconds"] = json!(timeout);
        if requested != timeout {
            value["timeoutClamped"] = json!(true);
        }
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
                    if state.phase == "running" {
                        state.phase = "unknown";
                        state.error = Some(
                            "Process control queue is unavailable; stop was not acknowledged.".into(),
                        );
                    }
                }
                process.changed.notify_waiters();
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

    pub(crate) async fn wait_for_terminal(
        &self, owner: &str, workspace: &str, id: &str, wait: Duration,
    ) -> Result<(), String> {
        let process = self.owned(owner, workspace, id)?;
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            let notified = process.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if {
                let output = process.output.lock().map_err(|_| "Process output is unavailable.")?;
                output.phase != "running" && !output.observation_pending
            } {
                return Ok(());
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return Ok(());
            }
        }
    }

    pub async fn poll(&self, owner: &str, workspace: &str, args: Value) -> Result<Value, String> {
        let args: PollArgs = serde_json::from_value(args).map_err(|error| error.to_string())?;
        let limit = args.max_bytes.unwrap_or(65536);
        let wait = args.wait_ms.unwrap_or(0).min(MAX_POLL_WAIT_MS);
        if !(8192..=65536).contains(&limit) {
            return Err("Process max_bytes must be 8192–65536.".into());
        }
        let process = self.owned(owner, workspace, &args.id)?;
        #[cfg(test)]
        {
            self.polls_started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.poll_started.notify_one();
        }
        let cursor = args.cursor.unwrap_or(0);
        let deadline = tokio::time::Instant::now() + Duration::from_millis(wait);
        loop {
            let notified = process.changed.notified();
            tokio::pin!(notified);
            // Subscribe before inspecting output so a write or exit between the
            // snapshot and the await cannot be missed.
            notified.as_mut().enable();
            let page = process.output.lock().map_err(|_| "Process output is unavailable.")?
                .poll(&args.id, cursor, limit)?;
            if page["state"] != "running" || page["nextCursor"].as_u64() != Some(cursor)
                || page["truncated"] == true || tokio::time::Instant::now() >= deadline
            {
                return Ok(page);
            }
            let _ = tokio::time::timeout_at(deadline, notified).await;
        }
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
        self.shell_run_tracked(owner, workspace, config, args, None, |_| Ok(())).await
    }

    pub(crate) async fn shell_run_tracked(
        &self,
        owner: &str,
        workspace: &str,
        config: crate::execution::ExecutionConfig,
        mut args: Value,
        run_id: Option<&str>,
        on_launch: impl FnOnce(&str) -> Result<(), String>,
    ) -> Result<Value, String> {
        align_arguments("shell_run", &mut args);
        let args: ShellRunArgs = serde_json::from_value(args).map_err(|error| error.to_string())?;
        let wait_secs = args.timeout_secs.or(args.timeout_seconds).unwrap_or(DEFAULT_SHELL_WAIT_SECS).clamp(1, 600);
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
        let process = self.owned(owner, workspace, &id)?;
        let mut wait = ShellWait { process, returned: false };
        *wait.process.waiting_run.lock().map_err(|_| "Process ownership is unavailable.")? = run_id.map(str::to_owned);
        on_launch(&id)?;
        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut truncated = false;
        let mut cursor = 0u64;
        let mut next_cursor = 0u64;
        let mut capped = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(wait_secs);
        let page = loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let wait_ms = remaining.as_millis().min(1000) as u64;
            let page = self.poll(
                owner,
                workspace,
                json!({"id":id,"cursor":cursor,"max_bytes":65536,"wait_ms":wait_ms}),
            ).await?;
            truncated |= page["truncated"] == true;
            cursor = page["nextCursor"].as_u64().unwrap_or(cursor);
            if !capped {
                next_cursor = next_cursor.max(page["firstAvailableCursor"].as_u64().unwrap_or(0));
                for frame in page["output"].as_array().into_iter().flatten() {
                    let text = frame["text"].as_str().unwrap_or("");
                    if stdout.len() + stderr.len() + text.len() > OUTPUT_BYTES {
                        capped = true;
                        break;
                    }
                    if frame["stream"] == "stderr" {
                        stderr.push_str(text);
                    } else {
                        stdout.push_str(text);
                    }
                    next_cursor = frame["cursor"].as_u64().unwrap_or(next_cursor) + 1;
                }
            }
            if (page["state"] != "running" || remaining.is_zero()) && page["hasMore"] != true {
                break page;
            }
            if remaining.is_zero() && page["state"] == "running" {
                // The producer may still be running; bound the wait even if
                // it keeps adding pages as we consume them.
                break page;
            }
        };
        let state = page["state"].as_str().unwrap_or("unknown");
        let has_more = capped || page["hasMore"] == true;
        let mut result = json!({
            "status":state, "processState":state, "processId":id,
            "stdout":stdout, "stderr":stderr, "truncated":truncated || has_more,
            "nextCursor":next_cursor, "hasMore":has_more,
            "durationMs":started.elapsed().as_millis() as u64,
        });
        if state == "running" {
            result["message"] = json!("Still running. Use process_poll with this processId and wait_ms=30000 to follow it, or process_stop to stop it.");
        } else {
            result["exitCode"] = page["exitCode"].clone();
            result["isError"] = json!(page["isError"] == true || state != "exited" || page["exitCode"] != 0);
            result["error"] = page["message"].clone();
        }
        wait.returned = true;
        Ok(result)
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
                let acknowledgement = tokio::time::timeout(Duration::from_secs(5), received).await;
                if !matches!(acknowledgement, Ok(Ok(()))) && process.output.lock()
                    .map_err(|_| "Process output is unavailable.")?.phase == "running"
                {
                    return Err(match acknowledgement {
                        Err(_) => "Process stop timed out; its outcome is unknown.",
                        _ => "Process supervisor stopped without a final acknowledgement.",
                    }.into());
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
    changed: Arc<Notify>,
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
                changed.notify_waiters();
                if count == 0 {
                    break;
                }
            }
            Err(error) => {
                if let Ok(mut state) = output.lock() {
                    state.error = Some(format!("Could not read {stream}: {error}"));
                    state.incomplete = true;
                }
                changed.notify_waiters();
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
    mut stop: tokio::sync::watch::Receiver<bool>,
    terminated: tokio::sync::watch::Sender<bool>,
    output: Arc<Mutex<Output>>,
    changed: Arc<Notify>,
    out_reader: tokio::task::JoinHandle<()>,
    err_reader: tokio::task::JoinHandle<()>,
    timeout: u64,
    completion: Option<crate::verification_observer::Completion>,
) {
    let mut stdin = Some(stdin);
    let mut stop_reply = None;
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
            _ = stop.changed() => break match terminate(&mut child).await {
                Ok(code) => ("stopped", code, Some("Process owner closed; process tree stopped.".into())),
                Err(error) => ("unknown", None, Some(error)),
            },
            status = child.wait() => break match status {
                Ok(status) => ("exited", status.code(), if status.success() { None } else { Some(format!("Process exited with status {status}.")) }),
                Err(error) => ("failed", None, Some(format!("Process wait failed: {error}"))),
            },
            _ = &mut deadline => break match terminate(&mut child).await {
                Ok(code) => ("timed_out", code, Some(format!("The process reached its {timeout}-second shell lifetime and was stopped."))),
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
                    stop_reply = Some(reply);
                    break match terminate(&mut child).await {
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
    terminated.send_replace(matches!(phase, "exited" | "stopped" | "timed_out"));
    let mut completion = completion;
    if stop_reply.is_none() {
        if let Some(observation) = completion.take() {
            observation.finish(phase).await;
        }
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
        state.observation_pending = completion.is_some();
        if let Some(error) = error {
            state.error = Some(match state.error.take() {
                Some(output_error) => format!("{error} {output_error}"),
                None => error,
            });
        }
    }
    changed.notify_waiters();
    // An explicit stop returns the terminal snapshot, including drained
    // output. A workspace scan for verification can finish afterwards.
    drop(receiver);
    if let Some(reply) = stop_reply {
        let _ = reply.send(());
    }
    if let Some(observation) = completion {
        observation.finish(phase).await;
        if let Ok(mut state) = output.lock() {
            state.observation_pending = false;
        }
        changed.notify_waiters();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixed_timeout_fields_are_rewritten_before_dispatch() {
        let mut args = json!({"command":"cargo test","timeout_secs":3600,"yield_ms":30000});
        align_arguments("process_start", &mut args);
        assert_eq!(args["timeout_seconds"], 3600);
        assert!(args.get("timeout_secs").is_none());
        assert_eq!(args["yield_ms"], 1000);
        let mut wait = json!({"command":"echo hi","timeout_seconds":900});
        align_arguments("shell_run", &mut wait);
        assert_eq!(wait["timeout_secs"], 600);
        assert!(wait.get("timeout_seconds").is_none());
        limit_wait("shell_run", &mut wait, Duration::from_secs(300));
        assert_eq!(wait["timeout_secs"], 299);
        let mut poll = json!({"id":"fixture","wait_ms":30_000});
        align_arguments("process_poll", &mut poll);
        assert_eq!(poll["wait_ms"], 30_000);
        limit_wait("process_poll", &mut poll, Duration::from_secs(5));
        assert_eq!(poll["wait_ms"], 4_000);
    }

    #[tokio::test]
    async fn a_stop_is_acknowledged_before_the_observation_is_sealed() {
        // Sealing an observation walks the workspace, polls a watcher barrier
        // and writes to the database. The stop must be acknowledged as soon
        // as the process tree is terminated, not after that work.
        let watched = tempfile::tempdir().unwrap();
        let sealed = tempfile::tempdir().unwrap();
        let pending = crate::verification_observer::Registry::begin(
            watched.path().to_str().unwrap(),
        )
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
        let process = registry.owned("owner", directory.path().to_str().unwrap(), &id).unwrap();
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
        assert_eq!(process.output.lock().unwrap().phase, "stopped");
        assert!(
            acknowledged.elapsed() < Duration::from_millis(900),
            "the stop must be acknowledged before the observation is sealed: {:?}",
            acknowledged.elapsed()
        );
        assert!(process.output.lock().unwrap().observation_pending);
        registry.wait_for_terminal("owner", directory.path().to_str().unwrap(), &id, Duration::from_secs(5)).await.unwrap();
        assert!(!process.output.lock().unwrap().observation_pending);
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
            changed: Arc::new(Notify::new()),
            control,
            stop: tokio::sync::watch::channel(false).0,
            terminated: tokio::sync::watch::channel(false).1,
            waiting_run: Mutex::new(None),
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
                    crate::presets::get(preset)
                        .unwrap()
                        .harness
                        .contains(alias),
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
        let mut cursor = 0;
        let mut output = String::new();
        for _ in 0..100 {
            result = harness_call(&ctx, "process_poll", json!({"id":id,"cursor":cursor,"wait_ms":20}))
                .await
                .unwrap();
            for frame in result["output"].as_array().unwrap() {
                output.push_str(frame["text"].as_str().unwrap());
            }
            cursor = result["nextCursor"].as_u64().unwrap();
            if result["state"] != "running" && result["hasMore"] == false {
                break;
            }
        }
        assert_eq!(result["state"], "exited", "{result}");
        assert_eq!(result["exitCode"], 0, "{result}");
        assert!(output.contains("model fixture output"));
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
        let mut cursor = 0;
        for _ in 0..100 {
            let result = registry
                .poll("owner", root, json!({"id":id,"cursor":cursor,"wait_ms":20}))
                .await
                .unwrap();
            if result["state"] != "running" {
                return registry.retained_output("owner", root, id).unwrap();
            }
            cursor = result["nextCursor"].as_u64().unwrap();
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
            json!({"command":"echo hi","timeout_seconds":601}),
        ] {
            assert!(
                registry
                    .execute("owner", root, config.clone(), "shell_run", args.clone())
                    .await
                    .is_ok(),
                "{args}"
            );
        }
        assert!(registry.execute("owner", root, config.clone(), "shell_run", json!({"command":"echo hi","unexpected":1})).await.is_err());
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
        assert_eq!(result["code"], "timeout");
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
        let oversized = registry
            .start("owner", root, config.clone(), json!({"command":command,"yield_ms":1001,"timeout_seconds":config.bash.timeout_seconds + 1}))
            .await
            .unwrap();
        assert_eq!(oversized["appliedTimeoutSeconds"], config.bash.timeout_seconds);
        assert_eq!(oversized["timeoutClamped"], true);
        registry.stop("owner", root, json!({"id":oversized["id"]})).await.unwrap();
        for args in [
            json!({"command":""}),
            json!({"command":command,"cwd":"../"}),
            json!({"command":command,"unexpected":true}),
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

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn cancelling_an_attached_shell_settles_its_handle_and_prevents_a_late_write() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_str().unwrap().to_string();
        let registry = Arc::new(ProcessRegistry::default());
        let (started, id) = oneshot::channel();
        let task_registry = registry.clone();
        let task_root = root.clone();
        let task = tokio::spawn(async move {
            task_registry.shell_run_tracked("owner", &task_root, crate::execution::ExecutionConfig::default(),
                json!({"command":"(sleep 1; printf changed > late.txt) & printf '%s\\n' \"$!\" > descendant.pid; wait"}),
                Some("run"), |id| { let _ = started.send(id.to_string()); Ok(()) }).await
        });
        let handle = id.await.unwrap();
        let pid = descendant_pid(&directory.path().join("descendant.pid")).await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        let settled = registry.settle_cancelled_shells("run").await;
        assert_eq!(settled, vec![json!({"processId":handle,"processState":"stopped"})]);
        assert_terminated(pid).await;
        assert!(!directory.path().join("late.txt").exists());
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

    #[test]
    fn minimum_page_advances_over_replacement_decoded_output() {
        let mut output = Output::default();
        let text = String::from_utf8_lossy(&vec![0xff; 4096]).into_owned();
        output.push("stdout", text.clone());
        let first = output.poll("test", 0, MIN_PAGE_BYTES).unwrap();
        assert_eq!(first["hasMore"], true);
        let next = first["nextCursor"].as_u64().unwrap();
        assert!(next > 0);
        let second = output.poll("test", next, MIN_PAGE_BYTES).unwrap();
        let actual = [&first, &second].into_iter().flat_map(|page| page["output"].as_array().unwrap())
            .map(|frame| frame["text"].as_str().unwrap()).collect::<String>();
        assert_eq!(actual, text);
        assert_eq!(second["hasMore"], false);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shell_run_drains_final_pages_and_returns_without_mandatory_sleep() {
        let root = tempfile::tempdir().unwrap();
        let registry = ProcessRegistry::default();
        let result = registry.shell_run("owner", root.path().to_str().unwrap(),
            crate::execution::ExecutionConfig::default(),
            json!({"command":r"head -c 150000 /dev/zero | tr '\000' x", "timeout_secs":10}),
        ).await.unwrap();
        assert_eq!(result["stdout"].as_str().unwrap().len(), 150000);
        assert_eq!(result["hasMore"], false);
        assert_eq!(result["truncated"], false);
        let start = std::time::Instant::now();
        let page = registry.poll("owner", root.path().to_str().unwrap(), json!({
            "id":result["processId"], "wait_ms":1000, "cursor":result["nextCursor"],
        })).await.unwrap();
        assert_eq!(page["state"], "exited");
        assert!(start.elapsed() < Duration::from_millis(800));
    }

}

// Transcript controls operate only on a process already owned by this chat.
// They neither launch commands nor widen execution permissions. Use the
// registry's original workspace so a later project selection cannot strand it.
fn ui_process_workspace(state: &crate::AppState, conversation_id: &str, id: &str) -> Result<String, String> {
    let processes = state.processes.processes.lock().map_err(|_| "Process registry is unavailable.")?;
    let process = processes.get(id).ok_or("This process is no longer available; its saved state is historical.")?;
    if process.owner != conversation_id { return Err("Process belongs to another conversation.".into()); }
    Ok(process.workspace.clone())
}
#[tauri::command]
pub async fn get_owned_process(state: tauri::State<'_, crate::AppState>, conversation_id: String, id: String, cursor: u64) -> Result<Value, String> {
    let workspace = ui_process_workspace(&state, &conversation_id, &id)?;
    state.processes.poll(&conversation_id, &workspace, json!({"id":id,"cursor":cursor,"max_bytes":65536,"wait_ms":0})).await
}
#[tauri::command]
pub async fn stop_owned_process(state: tauri::State<'_, crate::AppState>, conversation_id: String, id: String) -> Result<Value, String> {
    let workspace = ui_process_workspace(&state, &conversation_id, &id)?;
    state.processes.stop(&conversation_id, &workspace, json!({"id":id})).await
}

#[cfg(test)]
mod transcript_control_tests {
    use super::*;
    #[tokio::test]
    async fn transcript_lookup_cannot_read_or_stop_another_chats_process() {
        let root = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(root.path());
        let workspace = root.path().to_string_lossy().into_owned();
        let config = state.database().unwrap().execution_config().unwrap();
        let command = if cfg!(windows) { "Start-Sleep -Seconds 30" } else { "sleep 30" };
        let launch = state.processes.start("owner", &workspace, config, json!({"command":command,"yield_ms":0})).await.unwrap();
        let id = launch["id"].as_str().unwrap();
        assert!(ui_process_workspace(&state, "other", id).is_err());
        assert_eq!(ui_process_workspace(&state, "owner", id).unwrap(), workspace);
        let stopped = state.processes.stop("owner", &workspace, json!({"id":id})).await.unwrap();
        assert_eq!(stopped["state"], "stopped");
    }
}
