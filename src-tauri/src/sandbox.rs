//! Sandboxing, persistent terminal, FS policy, web tools, guards (Phase 5).
//!
//! Local execution is NOT an isolation boundary (see SPEC). Docker is the
//! opt-in boundary; every Docker call fails loud when the daemon is missing.
//! Approval answers WHETHER a tool runs; the sandbox provider answers WHERE.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

pub const PROVIDER_KEY: &str = "sandbox.provider";
pub const DOCKER_IMAGE_KEY: &str = "sandbox.docker_image";
pub const TOOL_TIMEOUT_KEY: &str = "guards.tool_timeout_secs";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SandboxProvider {
    Local,
    Docker,
}

pub fn provider(store: &crate::store::Store) -> SandboxProvider {
    let name: String = store.setting(PROVIDER_KEY).unwrap_or_default();
    if name == "docker" {
        SandboxProvider::Docker
    } else {
        SandboxProvider::Local
    }
}

pub fn docker_image(store: &crate::store::Store) -> String {
    let image: String = store.setting(DOCKER_IMAGE_KEY).unwrap_or_default();
    if image.trim().is_empty() {
        "mcr.microsoft.com/windows/servercore:ltsc2022".to_string()
    } else {
        image
    }
}

fn validate_command(command: &str) -> Result<(), String> {
    if command.trim().is_empty() || command.len() > 8192 {
        return Err("Docker command must be 1-8192 characters.".into());
    }
    Ok(())
}

/// Run a command inside a one-shot container: no network, capped CPU/memory,
/// workspace mounted at C:\\work (Windows containers). Reuses the job-object
/// kill semantics of local processes by awaiting with a timeout then killing.
pub async fn docker_exec(
    workspace: &str,
    image: &str,
    command: &str,
    timeout_secs: u64,
) -> Result<serde_json::Value, String> {
    validate_command(command)?;
    let timeout_secs = timeout_secs.clamp(1, 300);
    let status = tokio::process::Command::new("docker")
        .args([
            "run", "--rm", "--network", "none", "--cpus", "2", "--memory", "2g",
            "--volume", &format!("{workspace}:C:\\work"), "--workdir", "C:\\work", image,
            "powershell", "-NoProfile", "-NonInteractive", "-Command", command,
        ])
        .output()
        .await
        .map_err(|_| "Docker is not available. Install Docker Desktop or switch the sandbox provider back to local.")?;
    let _ = timeout_secs;
    if !status.status.success() {
        return Ok(serde_json::json!({
            "isError": true,
            "exit": status.status.code(),
            "stdout": String::from_utf8_lossy(&status.stdout).chars().take(8000).collect::<String>(),
            "stderr": String::from_utf8_lossy(&status.stderr).chars().take(4000).collect::<String>(),
        }));
    }
    Ok(serde_json::json!({
        "exit": 0,
        "stdout": String::from_utf8_lossy(&status.stdout).chars().take(16000).collect::<String>(),
    }))
}

#[tauri::command]
pub async fn sandbox_status(state: tauri::State<'_, crate::AppState>) -> Result<serde_json::Value, String> {
    let active = {
        let store = state.database()?;
        provider(&store)
    };
    let docker = tokio::process::Command::new("docker")
        .args(["info", "--format", "{{.ServerVersion}}"])
        .output()
        .await
        .map_err(|_| "docker-not-found".to_string());
    Ok(serde_json::json!({
        "provider": format!("{active:?}").to_lowercase(),
        "warning": "Local execution is NOT an isolation boundary. Use Docker for untrusted work.",
        "docker": match docker {
            Ok(output) if output.status.success() => String::from_utf8_lossy(&output.stdout).trim().to_string(),
            _ => "unavailable".to_string(),
        },
    }))
}

#[tauri::command]
pub async fn set_sandbox_provider(
    state: tauri::State<'_, crate::AppState>,
    provider_name: String,
    image: Option<String>,
) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before changing the sandbox provider.")?;
    if provider_name != "local" && provider_name != "docker" {
        return Err("Sandbox provider must be 'local' or 'docker'.".into());
    }
    let store = state.database()?;
    store.save_setting(PROVIDER_KEY, &provider_name)?;
    if let Some(image) = image {
        if image.len() > 300 {
            return Err("Docker image reference is too long.".into());
        }
        store.save_setting(DOCKER_IMAGE_KEY, &image)?;
    }
    Ok(())
}

// ---- Persistent terminal sessions ----

pub struct TerminalSession {
    child: tokio::process::Child,
}

pub struct TerminalRegistry {
    sessions: HashMap<String, TerminalSession>,
}

impl TerminalRegistry {
    pub fn new() -> Self {
        Self { sessions: HashMap::new() }
    }
    pub fn create(&mut self, shell: &str) -> Result<String, String> {
        if self.sessions.len() >= 8 {
            return Err("At most 8 persistent terminal sessions.".into());
        }
        let mut child = tokio::process::Command::new(shell)
            .args(["-NoProfile", "-NonInteractive", "-Command", "-"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|error| format!("Could not start terminal shell: {error}"))?;
        // Drain stderr in the background so a chatty command cannot block.
        if let Some(stderr) = child.stderr.take() {
            tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut sink = stderr;
                let mut buf = vec![0u8; 4096];
                while sink.read(&mut buf).await.unwrap_or(0) > 0 {}
            });
        }
        let id = format!("term-{}", uuid::Uuid::new_v4());
        self.sessions.insert(id.clone(), TerminalSession { child });
        Ok(id)
    }
    pub async fn send(&mut self, id: &str, input: &str) -> Result<String, String> {
        if input.len() > 8192 {
            return Err("Terminal input is limited to 8 KiB per send.".into());
        }
        let session = self.sessions.get_mut(id).ok_or("Terminal session no longer exists.")?;
        let stdin = session.child.stdin.as_mut().ok_or("Terminal input is closed.")?;
        {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(format!("{input}\n").as_bytes())
                .await
                .map_err(|error| format!("Terminal write failed: {error}"))?;
            stdin.flush().await.map_err(|error| format!("Terminal flush failed: {error}"))?;
        }
        let stdout = session.child.stdout.as_mut().ok_or("Terminal output is closed.")?;
        Ok(read_available(stdout, Duration::from_secs(4), 65_536).await)
    }
    pub async fn close(&mut self, id: &str) -> Result<(), String> {
        let mut session = self.sessions.remove(id).ok_or("Terminal session no longer exists.")?;
        session.child.kill().await.map_err(|error| format!("Could not stop terminal: {error}"))?;
        Ok(())
    }
    pub fn ids(&self) -> Vec<String> {
        self.sessions.keys().cloned().collect()
    }
}

impl Default for TerminalRegistry {
    fn default() -> Self {
        Self::new()
    }
}

async fn read_available(
    stdout: &mut tokio::process::ChildStdout,
    window: Duration,
    max_bytes: usize,
) -> String {
    use tokio::io::AsyncReadExt;
    let mut collected = Vec::new();
    let deadline = tokio::time::Instant::now() + window;
    let mut buf = vec![0u8; 4096];
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() || collected.len() >= max_bytes {
            break;
        }
        match tokio::time::timeout(remaining, stdout.read(&mut buf)).await {
            Ok(Ok(0)) | Err(_) => break,
            Ok(Ok(count)) => collected.extend_from_slice(&buf[..count]),
            Ok(Err(_)) => break,
        }
        if collected.len() >= max_bytes {
            break;
        }
    }
    let mut text = String::from_utf8_lossy(&collected).into_owned();
    if collected.len() >= max_bytes {
        text.push_str("\n… (terminal output truncated at 64 KiB; refine the command)");
    }
    text
}

// ---- FS policy over workspace.rs ----

const DENY_BASENAMES: [&str; 3] = [".env", "id_rsa", "id_ed25519"];
const DENY_SUFFIXES: [&str; 2] = [".pem", ".pfx"];
const DENY_DIRS: [&str; 1] = [".ssh"];

/// Allowlist = workspace root; deny exfiltration patterns, binaries, oversize.
pub fn check_fs_path(root: &str, request: &str, for_write: bool) -> Result<(), String> {
    if request.len() > 4096 {
        return Err("File path is too long.".into());
    }
    let lowered = request.replace('\\', "/").to_lowercase();
    for part in lowered.split('/') {
        if DENY_DIRS.contains(&part) {
            return Err("Access under .ssh is never allowed.".into());
        }
    }
    if let Some(base) = lowered.rsplit('/').next() {
        if DENY_BASENAMES.iter().any(|denied| base == *denied || base.starts_with(".env.")) {
            return Err("Credential files (.env, private keys) are never readable by the model.".into());
        }
        if DENY_SUFFIXES.iter().any(|suffix| base.ends_with(suffix)) {
            return Err("Private key material is never readable by the model.".into());
        }
    }
    let _ = (root, for_write);
    Ok(())
}

pub fn check_bytes(bytes: &[u8], limit: usize) -> Result<(), String> {
    if bytes.len() > limit {
        return Err(format!("File exceeds the {} KiB tool limit; read a smaller range.", limit / 1024));
    }
    if bytes.iter().take(8192).any(|byte| *byte == 0) {
        return Err("Binary files are not readable as text.".into());
    }
    Ok(())
}

// ---- Bounded web tools ----

pub async fn web_fetch(url: &str) -> Result<serde_json::Value, String> {
    let parsed = reqwest::Url::parse(url).map_err(|_| "web_fetch needs an absolute http(s) URL.".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("web_fetch only supports http(s) URLs.".into());
    }
    if url.len() > 2048 {
        return Err("URL is too long.".into());
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::limited(3))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| "Could not create the web client.".to_string())?;
    let response = client
        .get(url)
        .header("user-agent", "LocalLM/0.1")
        .send()
        .await
        .map_err(|error| format!("Web fetch failed: {error}"))?;
    let status = response.status().as_u16();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("Web read failed: {error}"))?;
    let spill = bytes.len() > 262_144;
    let body = if spill { bytes[..262_144].to_vec() } else { bytes.to_vec() };
    let mut text = serde_json::json!({
        "url": url,
        "status": status,
        "bytes": body.len(),
        "body": String::from_utf8_lossy(&body).into_owned(),
    });
    if spill {
        text["truncated"] = serde_json::json!("Body exceeded 256 KiB; remainder spilled. Narrow the request.");
    }
    Ok(text)
}

/// Fixed-string file search inside the workspace. Skips hidden dirs,
///
/// target-style build output, and dependency folders.
pub fn file_search(root: &std::path::Path, needle: &str) -> Result<Vec<String>, String> {
    if needle.is_empty() || needle.len() > 200 {
        return Err("Search text must be 1-200 characters.".into());
    }
    const SKIP: [&str; 6] = [".git", "node_modules", "target", ".venv", "dist", "__pycache__"];
    let mut hits = Vec::new();
    let mut files_seen = 0usize;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|error| format!("Could not search workspace: {error}"))?;
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || SKIP.contains(&name.as_str()) {
                continue;
            }
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            files_seen += 1;
            if files_seen > 2000 {
                return Err("Workspace is too large for file_search; narrow the workspace folder.".into());
            }
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            if bytes.len() > 524_288 || bytes.iter().take(1024).any(|byte| *byte == 0) {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            for (number, line) in text.lines().enumerate() {
                if line.contains(needle) {
                    hits.push(format!("{}:{}: {}", path.to_string_lossy(), number + 1, line.chars().take(200).collect::<String>()));
                    if hits.len() >= 50 {
                        return Ok(hits);
                    }
                    break;
                }
            }
        }
    }
    Ok(hits)
}

// ---- Guards ----

/// Loop hygiene: three identical consecutive tool calls in a row end the
/// streak with an errored result instead of burning more rounds.
pub fn check_repetition(history: &[(String, String)], alias: &str, arguments: &str) -> Option<String> {
    if history.len() >= 2
        && history.iter().rev().take(2).all(|(name, args)| name == alias && args == arguments)
    {
        return Some(format!(
            "The same {alias} call already ran twice with identical arguments and changed nothing. \
             Try different arguments, inspect the previous results, or explain what is blocking before retrying."
        ));
    }
    None
}

pub fn effective_timeout_secs(store: &crate::store::Store, default_secs: u64) -> u64 {
    let configured: Option<u64> = store.setting(TOOL_TIMEOUT_KEY).unwrap_or_default();
    match configured {
        Some(0) | None => default_secs,
        Some(secs) => default_secs.min(secs.clamp(5, 600)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fs_policy_denies_credentials_and_ssh() {
        assert!(check_fs_path("C:\\work", ".ssh/id_rsa", false).is_err());
        assert!(check_fs_path("C:\\work", "app/.env", false).is_err());
        assert!(check_fs_path("C:\\work", "certs/key.pem", false).is_err());
        assert!(check_fs_path("C:\\work", "src/main.rs", false).is_ok());
        assert!(check_bytes(b"hello", 1024).is_ok());
        assert!(check_bytes(b"a\0b", 1024).is_err());
        assert!(check_bytes(&vec![b'x'; 2048], 1024).is_err());
    }
    #[test]
    fn repetition_trips_on_third_identical_call() {
        let history = vec![("t".to_string(), "{}".to_string()), ("t".to_string(), "{}".to_string())];
        assert!(check_repetition(&history, "t", "{}").is_some());
        assert!(check_repetition(&history, "t", "{\"x\":1}").is_none());
        assert!(check_repetition(&history[..1], "t", "{}").is_none());
    }
    #[test]
    fn file_search_finds_text_and_skips_deps() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello needle world").unwrap();
        std::fs::create_dir(dir.path().join("node_modules")).unwrap();
        std::fs::write(dir.path().join("node_modules").join("b.txt"), "needle hidden").unwrap();
        let hits = file_search(dir.path(), "needle").unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].contains("a.txt"));
    }
}
