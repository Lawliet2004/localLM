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
    buffer: Mutex<Vec<u8>>,
    sink: Mutex<Option<tauri::ipc::Channel<Value>>>,
    exit_code: Mutex<Option<u32>>,
}

impl PtySession {
    pub fn kill(&self) {
        if let Ok(mut killer) = self.killer.lock() {
            let _ = killer.kill();
        }
    }
}

pub type PtyRegistry = Mutex<HashMap<String, Arc<PtySession>>>;

fn emit(session: &PtySession, event: Value) {
    if let Ok(sink) = session.sink.lock() {
        if let Some(channel) = sink.as_ref() {
            let _ = channel.send(event);
        }
    }
}

fn lock<'a, T>(mutex: &'a Mutex<T>) -> Result<std::sync::MutexGuard<'a, T>, String> {
    mutex.lock().map_err(|_| "Terminal state lock is unavailable.".into())
}

fn spawn_session(cwd: &str, cols: u16, rows: u16) -> Result<Arc<PtySession>, String> {
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

    let mut command = CommandBuilder::new(shell);
    #[cfg(windows)]
    command.arg("-NoLogo");
    command.env("TERM", "xterm-256color");
    let home = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")).unwrap_or_default();
    let cwd = if cwd.is_empty() { home.as_str() } else { cwd };
    if !cwd.is_empty() {
        command.cwd(cwd);
    }
    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|error| format!("Could not start the shell: {error}"))?;
    let killer = child.clone_killer();
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
        buffer: Mutex::new(Vec::new()),
        sink: Mutex::new(None),
        exit_code: Mutex::new(None),
    });

    // PTY output -> capped scrollback buffer + whichever UI channel is attached.
    let output_session = session.clone();
    std::thread::spawn(move || {
        let mut chunk = [0u8; 8192];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(count) => {
                    if let Ok(mut buffer) = output_session.buffer.lock() {
                        buffer.extend_from_slice(&chunk[..count]);
                        if buffer.len() > REPLAY_CAP {
                            let excess = buffer.len() - REPLAY_CAP;
                            buffer.drain(..excess);
                        }
                    }
                    emit(
                        &output_session,
                        json!({"type":"output","data":String::from_utf8_lossy(&chunk[..count])}),
                    );
                }
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

/// Spawn a fresh shell in the workspace root. Opening is separate from
/// attaching so a remounted UI can reattach without duplicating sessions.
#[tauri::command]
pub fn terminal_open(state: tauri::State<'_, crate::AppState>, cols: u16, rows: u16) -> Result<Value, String> {
    let root = state.database()?.workspace_path()?;
    let mut sessions = lock(&state.pty_sessions)?;
    if sessions.len() >= MAX_SESSIONS {
        return Err("Too many open terminals; close one first.".into());
    }
    let session = spawn_session(&root, cols, rows)?;
    sessions.insert(session.id.clone(), session.clone());
    Ok(json!({"id":session.id}))
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
        // Hold the buffer while installing the sink: the reader locks the same
        // buffer before emitting, so replayed bytes stay ahead of live output.
        let buffer = lock(&session.buffer)?;
        let mut sink = lock(&session.sink)?;
        if !buffer.is_empty() {
            let _ = channel.send(json!({"type":"output","data":String::from_utf8_lossy(&buffer).into_owned()}));
        }
        *sink = Some(channel);
    }
    if let Some(code) = exit {
        emit(&session, json!({"type":"exit","code":code}));
    }
    Ok(json!({"id":session.id,"exited":exit.is_some()}))
}

#[tauri::command]
pub fn terminal_input(state: tauri::State<'_, crate::AppState>, id: String, data: String) -> Result<(), String> {
    if data.len() > MAX_INPUT {
        return Err("Terminal input is limited to 64 KiB.".into());
    }
    let sessions = lock(&state.pty_sessions)?;
    let session = sessions.get(&id).ok_or("Terminal session no longer exists.")?;
    let mut writer = lock(&session.writer)?;
    writer
        .write_all(data.as_bytes())
        .and_then(|_| writer.flush())
        .map_err(|error| format!("Terminal write failed: {error}"))
}

#[tauri::command]
pub fn terminal_resize(state: tauri::State<'_, crate::AppState>, id: String, cols: u16, rows: u16) -> Result<(), String> {
    let session = lock(&state.pty_sessions)?.get(&id).cloned().ok_or("Terminal session no longer exists.")?;
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

    // Killing a live shell must not block: the exit watcher holds `child`
    // across wait(), so kill() has to go through the cloned killer.
    #[test]
    fn kill_live_session_does_not_deadlock() {
        let session = spawn_session("", 80, 24).expect("spawn shell");
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
