//! Interactive user-facing terminal: a real PTY (ConPTY on Windows, forkpty
//! elsewhere) running the system shell inside the workspace directory.
//!
//! Sessions persist while the app runs so the inspector can close and reattach
//! without killing the shell. A capped scrollback buffer replays on attach.
//! This is separate from the model's `sandbox::TerminalRegistry`, which is a
//! sentinel-driven command pipe, not an interactive console.

use portable_pty::{native_pty_system, Child, ChildKiller, CommandBuilder, MasterPty, PtySize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

/// Scrollback replayed when the UI reattaches to a live session.
const REPLAY_CAP: usize = 256 * 1024;
const MAX_SESSIONS: usize = 8;
const MAX_INPUT: usize = 64 * 1024;

pub struct PtySession {
    id: String,
    // MasterPty is Send but not Sync; the mutex makes the session shareable.
    master: Mutex<Box<dyn MasterPty + Send>>,
    writer: Mutex<Box<dyn Write + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    // Separate handle for termination: the exit watcher holds `child` across
    // wait(), so locking `child` to kill a live shell would deadlock.
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    output: Mutex<OutputState>,
    exit_code: Mutex<Option<u32>>,
    #[cfg(unix)]
    pid: Option<u32>,
}

#[derive(Default)]
struct OutputState {
    buffer: String,
    pending: Vec<u8>,
    sink: Option<tauri::ipc::Channel<Value>>,
    queued: String,
    inflight: Option<u64>,
    next_seq: u64,
    skipped: usize,
}

fn queue_output(output: &mut OutputState, text: &str) {
    if output.sink.is_none() {
        return;
    }
    output.queued.push_str(text);
    if output.queued.len() > REPLAY_CAP {
        let mut excess = output.queued.len() - REPLAY_CAP;
        while !output.queued.is_char_boundary(excess) {
            excess += 1;
        }
        output.queued.drain(..excess);
        output.skipped += excess;
    }
}
fn send_pending(output: &mut OutputState) {
    if output.inflight.is_some() || output.queued.is_empty() {
        return;
    }
    let Some(sink) = output.sink.as_ref() else {
        return;
    };
    let mut end = output.queued.len().min(64 * 1024);
    while !output.queued.is_char_boundary(end) {
        end -= 1;
    }
    let mut data: String = output.queued.drain(..end).collect();
    if output.skipped > 0 {
        data = format!(
            "\r\n[LocalLM skipped {} output bytes while the terminal was busy]\r\n{data}",
            output.skipped
        );
        output.skipped = 0;
    }
    output.next_seq = output.next_seq.wrapping_add(1);
    let seq = output.next_seq;
    output.inflight = Some(seq);
    if sink
        .send(json!({"type":"output", "data":data, "seq":seq}))
        .is_err()
    {
        output.sink = None;
        output.inflight = None;
        output.queued.clear();
    }
}

// PTYs split arbitrary bytes, including the middle of UTF-8 characters.
pub(crate) fn decode(pending: &mut Vec<u8>, bytes: &[u8], eof: bool) -> String {
    pending.extend_from_slice(bytes);
    let mut text = String::new();
    let mut consumed = 0;
    while consumed < pending.len() {
        match std::str::from_utf8(&pending[consumed..]) {
            Ok(rest) => {
                text.push_str(rest);
                consumed = pending.len();
            }
            Err(error) => {
                let end = consumed + error.valid_up_to();
                text.push_str(std::str::from_utf8(&pending[consumed..end]).unwrap_or_default());
                consumed = end;
                if let Some(length) = error.error_len() {
                    text.push('\u{fffd}');
                    consumed += length;
                } else {
                    break;
                }
            }
        }
    }
    pending.drain(..consumed);
    if eof && !pending.is_empty() {
        text.push_str(&String::from_utf8_lossy(pending));
        pending.clear();
    }
    text
}

impl PtySession {
    pub fn kill(&self) {
        if self.exit_code.lock().is_ok_and(|code| code.is_some()) {
            return;
        }
        #[cfg(unix)]
        let foreground = self
            .master
            .lock()
            .ok()
            .and_then(|master| master.process_group_leader());
        if let Ok(mut killer) = self.killer.lock() {
            let _ = killer.kill();
        }
        #[cfg(unix)]
        for group in [foreground, self.pid.and_then(|pid| i32::try_from(pid).ok())]
            .into_iter()
            .flatten()
        {
            if let Some(pid) = rustix::process::Pid::from_raw(group) {
                let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
            }
        }
    }
}

pub type PtyRegistry = Mutex<HashMap<String, Arc<PtySession>>>;

fn emit(session: &PtySession, event: Value) {
    if let Ok(output) = session.output.lock() {
        if let Some(channel) = output.sink.as_ref() {
            let _ = channel.send(event);
        }
    }
}

fn lock<'a, T>(mutex: &'a Mutex<T>) -> Result<std::sync::MutexGuard<'a, T>, String> {
    mutex
        .lock()
        .map_err(|_| "Terminal state lock is unavailable.".into())
}

fn spawn_session(
    cwd: &str,
    cols: u16,
    rows: u16,
    configured_shell: Option<&str>,
) -> Result<Arc<PtySession>, String> {
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: rows.clamp(1, 500),
            cols: cols.clamp(1, 500),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| format!("Could not allocate a terminal: {error}"))?;

    #[cfg(windows)]
    let shell = "powershell.exe";
    #[cfg(not(windows))]
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());

    let shell = configured_shell
        .filter(|shell| !shell.is_empty())
        .unwrap_or(shell.as_ref());
    let mut command = CommandBuilder::new(shell);
    if shell.to_ascii_lowercase().contains("powershell")
        || shell.to_ascii_lowercase().contains("pwsh")
    {
        command.arg("-NoLogo");
    }
    command.env("TERM", "xterm-256color");
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_default();
    let cwd = if cwd.is_empty() { home.as_str() } else { cwd };
    if !cwd.is_empty() {
        command.cwd(cwd);
    }
    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| format!("Could not start the shell: {error}"))?;
    let killer = child.clone_killer();
    #[cfg(unix)]
    let pid = child.process_id();
    drop(pair.slave);

    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|error| format!("Could not read terminal output: {error}"))?;
    let writer = pair
        .master
        .take_writer()
        .map_err(|error| format!("Could not open terminal input: {error}"))?;

    let session = Arc::new(PtySession {
        id: format!("pty-{}", uuid::Uuid::new_v4()),
        master: Mutex::new(pair.master),
        writer: Mutex::new(writer),
        child: Mutex::new(child),
        killer: Mutex::new(killer),
        output: Mutex::new(OutputState::default()),
        exit_code: Mutex::new(None),
        #[cfg(unix)]
        pid,
    });

    // PTY output -> capped scrollback buffer + whichever UI channel is attached.
    let output_session = session.clone();
    std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            let count = reader.read(&mut chunk).unwrap_or(0);
            if let Ok(mut output) = output_session.output.lock() {
                let text = decode(&mut output.pending, &chunk[..count], count == 0);
                output.buffer.push_str(&text);
                if output.buffer.len() > REPLAY_CAP {
                    let mut excess = output.buffer.len() - REPLAY_CAP;
                    while !output.buffer.is_char_boundary(excess) {
                        excess += 1;
                    }
                    output.buffer.drain(..excess);
                }
                queue_output(&mut output, &text);
                send_pending(&mut output);
            }
            if count == 0 {
                break;
            }
        }
    });

    // Exit watcher: records the code and notifies the attached UI so it can
    // offer a restart instead of silently freezing.
    let exit_session = session.clone();
    std::thread::spawn(move || {
        let code = exit_session
            .child
            .lock()
            .map(|mut child| child.wait().map(|status| status.exit_code()).unwrap_or(1))
            .unwrap_or(1);
        if let Ok(mut slot) = exit_session.exit_code.lock() {
            *slot = Some(code);
        }
        emit(&exit_session, json!({"type":"exit","code":code}));
    });

    Ok(session)
}

/// Insert a freshly spawned session, honouring the session cap.
///
/// The caller spawns the shell first and inserts it afterwards, so the
/// registry lock is never held across PTY allocation or process spawn. The
/// cap is re-checked here because other sessions may have opened while this
/// shell was starting; a session that does not fit is killed instead of being
/// left running without a handle.
fn insert_session(
    sessions: &mut HashMap<String, Arc<PtySession>>,
    session: Arc<PtySession>,
) -> Result<String, String> {
    if sessions.len() >= MAX_SESSIONS {
        session.kill();
        return Err("Too many open terminals; close one first.".into());
    }
    let id = session.id.clone();
    sessions.insert(id.clone(), session);
    Ok(id)
}

/// Spawn a fresh shell in the workspace root. Opening is separate from
/// attaching so a remounted UI can reattach without duplicating sessions.
#[tauri::command]
pub fn terminal_open(
    state: tauri::State<'_, crate::AppState>,
    cols: u16,
    rows: u16,
    root: Option<String>,
) -> Result<Value, String> {
    let root = match root.filter(|root| !root.is_empty()) {
        Some(root) => crate::workspace_ui::bound_root(&state, &root)?,
        None => state.database()?.workspace_path()?,
    };
    let config = state.database()?.execution_config()?;
    // The session is created first: allocating a PTY, spawning the shell and
    // starting the reader/exit threads must never happen while the registry
    // lock is held, or one slow spawn blocks every other terminal command.
    let session = spawn_session(&root, cols, rows, Some(&config.bash_path))?;
    let mut sessions = lock(&state.pty_sessions)?;
    let id = insert_session(&mut sessions, session)?;
    Ok(json!({"id":id}))
}

/// Attach the UI to a live session: replays scrollback, retargets output to
/// this channel, and reports the exit code if the shell already ended.
#[tauri::command]
pub fn terminal_attach(
    state: tauri::State<'_, crate::AppState>,
    id: String,
    channel: tauri::ipc::Channel<Value>,
) -> Result<Value, String> {
    let sessions = lock(&state.pty_sessions)?;
    let session = sessions
        .get(&id)
        .cloned()
        .ok_or("Terminal session no longer exists.")?;
    let exit = *lock(&session.exit_code)?;
    {
        // Replay and live delivery share one lock: output can neither be
        // duplicated nor slip between replay and channel installation.
        let mut output = lock(&session.output)?;
        output.sink = Some(channel);
        output.inflight = None;
        output.queued = output.buffer.clone();
        output.skipped = 0;
        send_pending(&mut output);
    }
    if let Some(code) = exit {
        emit(&session, json!({"type":"exit","code":code}));
    }
    Ok(json!({"id":session.id,"exited":exit.is_some()}))
}

/// xterm acknowledges only after parsing a chunk. At most one output chunk
/// can be queued in IPC; unsent output stays bounded in native memory.
#[tauri::command]
pub fn terminal_ack(
    state: tauri::State<'_, crate::AppState>,
    id: String,
    seq: u64,
) -> Result<(), String> {
    let session = lock(&state.pty_sessions)?
        .get(&id)
        .cloned()
        .ok_or("Terminal session no longer exists.")?;
    let mut output = lock(&session.output)?;
    if output.inflight == Some(seq) {
        output.inflight = None;
        send_pending(&mut output);
    }
    Ok(())
}

#[tauri::command]
pub fn terminal_input(
    state: tauri::State<'_, crate::AppState>,
    id: String,
    data: String,
) -> Result<(), String> {
    if data.len() > MAX_INPUT {
        return Err("Terminal input is limited to 64 KiB.".into());
    }
    let sessions = lock(&state.pty_sessions)?;
    let session = sessions
        .get(&id)
        .ok_or("Terminal session no longer exists.")?;
    let mut writer = lock(&session.writer)?;
    writer
        .write_all(data.as_bytes())
        .and_then(|_| writer.flush())
        .map_err(|error| format!("Terminal write failed: {error}"))
}

#[tauri::command]
pub fn terminal_resize(
    state: tauri::State<'_, crate::AppState>,
    id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    let session = lock(&state.pty_sessions)?
        .get(&id)
        .cloned()
        .ok_or("Terminal session no longer exists.")?;
    let master = lock(&session.master)?;
    master
        .resize(PtySize {
            rows: rows.clamp(1, 500),
            cols: cols.clamp(1, 500),
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|error| format!("Terminal resize failed: {error}"))
}

#[tauri::command]
pub fn terminal_close(state: tauri::State<'_, crate::AppState>, id: String) -> Result<(), String> {
    // Drop the registry guard before killing: a wedged registry would block
    // every other terminal command, and kill must never run under it.
    let session = lock(&state.pty_sessions)?.remove(&id);
    if let Some(session) = session {
        session.kill();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn output_waits_for_acknowledgement_and_bounds_backlog() {
        let events = Arc::new(Mutex::new(Vec::<Value>::new()));
        let collected = events.clone();
        let sink = tauri::ipc::Channel::new(move |body| {
            if let tauri::ipc::InvokeResponseBody::Json(value) = body {
                collected
                    .lock()
                    .unwrap()
                    .push(serde_json::from_str(&value).unwrap());
            }
            Ok(())
        });
        let mut output = OutputState {
            sink: Some(sink),
            ..Default::default()
        };
        queue_output(&mut output, "first");
        send_pending(&mut output);
        assert_eq!(events.lock().unwrap().len(), 1);
        for _ in 0..100 {
            queue_output(&mut output, &"世界".repeat(1024));
            send_pending(&mut output);
        }
        assert_eq!(events.lock().unwrap().len(), 1);
        assert!(output.queued.len() <= REPLAY_CAP);
        assert!(output.skipped > 0);
        output.inflight = None;
        send_pending(&mut output);
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert!(events[1]["data"].as_str().unwrap().contains("skipped"));
        assert!(!events[1]["data"].as_str().unwrap().contains('\u{fffd}'));
        assert_ne!(events[0]["seq"], events[1]["seq"]);
    }
    #[cfg(unix)]
    #[test]
    fn closing_pty_terminates_foreground_process_group() {
        let temp = tempfile::tempdir().unwrap();
        let session = spawn_session(temp.path().to_str().unwrap(), 80, 24, None).unwrap();
        session
            .writer
            .lock()
            .unwrap()
            .write_all(b"sleep 2; touch orphan-marker\n")
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(300));
        session.kill();
        std::thread::sleep(std::time::Duration::from_millis(2200));
        assert!(!temp.path().join("orphan-marker").exists());
    }
    #[test]
    fn decodes_unicode_across_pty_chunk_boundaries() {
        let mut pending = Vec::new();
        assert_eq!(decode(&mut pending, &[b'a', 0xf0, 0x9f], false), "a");
        assert_eq!(decode(&mut pending, &[0x98, 0x80, b'b'], false), "😀b");
        assert!(pending.is_empty());
        assert_eq!(decode(&mut pending, &[0xff, b'x', 0xe2], false), "�x");
        assert_eq!(decode(&mut pending, &[], true), "�");
    }

    #[cfg(unix)]
    #[test]
    fn the_session_cap_is_enforced_at_insertion_and_rejects_the_extra_shell() {
        let temp = tempfile::tempdir().unwrap();
        let mut sessions: HashMap<String, Arc<PtySession>> = HashMap::new();
        for _ in 0..MAX_SESSIONS {
            let session = spawn_session(temp.path().to_str().unwrap(), 80, 24, None).unwrap();
            insert_session(&mut sessions, session).unwrap();
        }
        assert_eq!(sessions.len(), MAX_SESSIONS);
        // The extra shell is spawned first and only rejected when it is
        // inserted; it must be killed instead of left running untracked.
        let extra = spawn_session(temp.path().to_str().unwrap(), 80, 24, None).unwrap();
        let extra_id = extra.id.clone();
        assert!(insert_session(&mut sessions, extra).is_err());
        assert_eq!(sessions.len(), MAX_SESSIONS);
        assert!(!sessions.contains_key(&extra_id));
        for session in sessions.values() {
            session.kill();
        }
    }

    // Killing a live shell must not block: the exit watcher holds `child`
    // across wait(), so kill() has to go through the cloned killer.
    #[test]
    fn kill_live_session_does_not_deadlock() {
        let session = spawn_session("", 80, 24, None).expect("spawn shell");
        let (tx, rx) = std::sync::mpsc::channel();
        let target = session.clone();
        std::thread::spawn(move || {
            target.kill();
            let _ = tx.send(());
        });
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .expect("kill() deadlocked on the child mutex");
    }
}
