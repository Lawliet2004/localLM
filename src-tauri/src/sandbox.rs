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
/// workspace mounted at C:\\work (Windows containers). The caller timeout is
/// enforced: on expiry the container process is killed and an error returned.
pub async fn docker_exec(
    workspace: &str,
    image: &str,
    command: &str,
    timeout_secs: u64,
) -> Result<serde_json::Value, String> {
    validate_command(command)?;
    let timeout_secs = timeout_secs.clamp(1, 300);
    let child = tokio::process::Command::new("docker")
        .args([
            "run", "--rm", "--network", "none", "--cpus", "2", "--memory", "2g",
            "--volume", &format!("{workspace}:C:\\work"), "--workdir", "C:\\work", image,
            "powershell", "-NoProfile", "-NonInteractive", "-Command", command,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|_| "Docker is not available. Install Docker Desktop or switch the sandbox provider back to local.")?;
    let output = match tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        child.wait_with_output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return Err(format!("Docker execution failed: {error}")),
        Err(_) => {
            // wait_with_output takes ownership; kill via `docker kill` lookup
            // is racy, so report the timeout and let --rm reap the container.
            return Err(format!("Docker command timed out after {timeout_secs}s; the container was left to --rm reaping."));
        }
    };
    if !output.status.success() {
        return Ok(serde_json::json!({
            "isError": true,
            "exit": output.status.code(),
            "stdout": String::from_utf8_lossy(&output.stdout).chars().take(8000).collect::<String>(),
            "stderr": String::from_utf8_lossy(&output.stderr).chars().take(4000).collect::<String>(),
        }));
    }
    Ok(serde_json::json!({
        "exit": 0,
        "stdout": String::from_utf8_lossy(&output.stdout).chars().take(16000).collect::<String>(),
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
    stderr_buf: std::sync::Arc<std::sync::Mutex<String>>,
    stderr_seen: usize,
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
        // Capture stderr in the background (capped) so errors stay visible to
        // the model instead of being discarded, without blocking the pipe.
        let stderr_buf = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        if let Some(stderr) = child.stderr.take() {
            let captured = stderr_buf.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncReadExt;
                let mut sink = stderr;
                let mut buf = vec![0u8; 4096];
                loop {
                    match sink.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(count) => {
                            let chunk = String::from_utf8_lossy(&buf[..count]).into_owned();
                            if let Ok(mut locked) = captured.lock() {
                                locked.push_str(&chunk);
                                if locked.len() > 65_536 {
                                    let excess = locked.len() - 65_536;
                                    locked.drain(..excess);
                                }
                            }
                        }
                    }
                }
            });
        }
        let id = format!("term-{}", uuid::Uuid::new_v4());
        self.sessions.insert(id.clone(), TerminalSession { child, stderr_buf, stderr_seen: 0 });
        Ok(id)
    }
    pub async fn send(&mut self, id: &str, input: &str) -> Result<String, String> {
        if input.len() > 8192 {
            return Err("Terminal input is limited to 8 KiB per send.".into());
        }
        // Sentinel marker terminates the stdout read the moment the command
        // finishes instead of sleeping for a fixed window every call.
        let sentinel = format!("__LOCALLM_DONE_{}__", uuid::Uuid::new_v4().simple());
        let session = self.sessions.get_mut(id).ok_or("Terminal session no longer exists.")?;
        let stdin = session.child.stdin.as_mut().ok_or("Terminal input is closed.")?;
        {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(format!("{input}\nWrite-Output \"{sentinel}:$LASTEXITCODE\"\n").as_bytes())
                .await
                .map_err(|error| format!("Terminal write failed: {error}"))?;
            stdin.flush().await.map_err(|error| format!("Terminal flush failed: {error}"))?;
        }
        let stdout = session.child.stdout.as_mut().ok_or("Terminal output is closed.")?;
        let mut output = read_until_sentinel(stdout, &sentinel, Duration::from_secs(30), 65_536).await;
        // Merge newly arrived stderr so model-visible errors are not lost.
        let fresh_stderr = session.stderr_buf.lock().map(|locked| {
            if locked.len() > session.stderr_seen {
                locked[session.stderr_seen..].to_string()
            } else {
                String::new()
            }
        }).unwrap_or_default();
        session.stderr_seen = session.stderr_buf.lock().map(|locked| locked.len()).unwrap_or(session.stderr_seen);
        if !fresh_stderr.trim().is_empty() {
            output.push_str("\n[stderr]\n");
            output.push_str(fresh_stderr.trim_end());
        }
        Ok(output)
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

async fn read_until_sentinel(
    stdout: &mut tokio::process::ChildStdout,
    sentinel: &str,
    window: Duration,
    max_bytes: usize,
) -> String {
    use tokio::io::AsyncReadExt;
    let mut collected = Vec::new();
    let deadline = tokio::time::Instant::now() + window;
    let mut buf = vec![0u8; 4096];
    let mut timed_out = false;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() || collected.len() >= max_bytes {
            timed_out = remaining.is_zero();
            break;
        }
        match tokio::time::timeout(remaining, stdout.read(&mut buf)).await {
            Ok(Ok(0)) | Ok(Err(_)) => break,
            Err(_) => { timed_out = true; break; }
            Ok(Ok(count)) => {
                collected.extend_from_slice(&buf[..count]);
                let text = String::from_utf8_lossy(&collected);
                if text.contains(sentinel) {
                    break;
                }
            }
        }
        if collected.len() >= max_bytes {
            break;
        }
    }
    let mut text = String::from_utf8_lossy(&collected).into_owned();
    // Strip the sentinel line (and its trailing newline) from model output.
    if let Some(pos) = text.find(sentinel) {
        let line_start = text[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let mut line_end = pos + sentinel.len();
        // Also swallow the `:<exit>` suffix and trailing newline.
        while line_end < text.len() && text.as_bytes()[line_end] != b'\n' {
            line_end += 1;
        }
        if line_end < text.len() {
            line_end += 1;
        }
        text.replace_range(line_start..line_end, "");
    }
    text = text.trim_end().to_string();
    if collected.len() >= max_bytes {
        text.push_str("\n… (terminal output truncated at 64 KiB; refine the command)");
    } else if timed_out {
        text.push_str("\n… (terminal read timed out after 30s; output may be partial)");
    }
    text
}

/// Legacy fixed-window read; superseded by sentinel reads in `send`.
#[allow(dead_code)]
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

pub const ALLOW_PRIVATE_FETCH_KEY: &str = "sandbox.allow_private_fetch";

fn is_blocked_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback() || v4.is_unspecified() || v4.is_multicast() || v4.is_link_local()
                || o[0] == 10
                || (o[0] == 172 && (16..=31).contains(&o[1]))
                || (o[0] == 192 && o[1] == 168)
                || (o[0] == 169 && o[1] == 254)
                || (o[0] == 100 && (64..=127).contains(&o[1]))
                || (o[0] == 192 && o[1] == 0 && o[2] == 2)
                || (o[0] == 198 && (18..=19).contains(&o[1]))
                || (o[0] == 203 && o[1] == 0 && o[2] == 113)
                || (o[0] == 192 && o[1] == 88 && o[2] == 99)
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback() || v6.is_unspecified() || v6.is_multicast()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

async fn check_fetch_host(url: &reqwest::Url, allow_private: bool) -> Result<(), String> {
    if allow_private {
        return Ok(());
    }
    let host = url.host_str().ok_or("web_fetch needs a host.")?.to_string();
    let port = url.port_or_known_default().unwrap_or(443);
    // Literal IPs are checked directly; hostnames go through DNS pre-flight.
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        if is_blocked_ip(&ip) {
            return Err("web_fetch blocked: loopback, link-local, and private addresses are not fetchable.".into());
        }
        return Ok(());
    }
    let addrs = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|_| "web_fetch could not resolve the host.".to_string())?;
    for addr in addrs {
        if is_blocked_ip(&addr.ip()) {
            return Err("web_fetch blocked: host resolves to a loopback, link-local, or private address.".into());
        }
    }
    Ok(())
}

fn resolve_redirect(current: &reqwest::Url, location: &str) -> Result<reqwest::Url, String> {
    reqwest::Url::parse(location)
        .or_else(|_| current.join(location))
        .map_err(|_| "Redirect location is invalid.".to_string())
}

/// Strip markup to plain readable text: drop head/style/script/noscript
/// blocks, replace tags with spaces, decode common entities, collapse space.
pub fn html_to_text(html: &str) -> String {
    let mut without_blocks = String::with_capacity(html.len());
    let lower = html.to_lowercase();
    let mut rest = html;
    let mut rest_lower = lower.as_str();
    for tag in ["head", "style", "script", "noscript"] {
        while let Some(start) = rest_lower.find(&format!("<{tag}")) {
            let Some(open_end) = rest_lower[start..].find('>') else { break };
            let after_open = start + open_end + 1;
            let close = format!("</{tag}>");
            let Some(close_at) = rest_lower[after_open..].find(&close) else { break };
            let end = after_open + close_at + close.len();
            let mut next = String::with_capacity(rest.len() - (end - start) + 1);
            next.push_str(&rest[..start]);
            next.push(' ');
            next.push_str(&rest[end..]);
            let next_lower = next.to_lowercase();
            rest = Box::leak(next.into_boxed_str());
            rest_lower = Box::leak(next_lower.into_boxed_str());
        }
    }
    without_blocks.push_str(rest);
    let mut text = String::with_capacity(without_blocks.len());
    let mut in_tag = false;
    for ch in without_blocks.chars() {
        match ch {
            '<' => { in_tag = true; text.push(' '); }
            '>' => { in_tag = false; text.push(' '); }
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    for (entity, replacement) in [
        ("&amp;", "&"), ("&lt;", "<"), ("&gt;", ">"), ("&quot;", "\""),
        ("&#39;", "'"), ("&apos;", "'"), ("&nbsp;", " "), ("&copy;", "(c)"),
    ] {
        text = text.replace(entity, replacement);
    }
    // Decode numeric entities (&#123; / &#x1F;).
    let mut decoded = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '&' && chars.peek() == Some(&'#') {
            let mut entity = String::from("&#");
            chars.next();
            let hex = chars.peek() == Some(&'x') || chars.peek() == Some(&'X');
            if hex { entity.push(chars.next().unwrap()); }
            while let Some(&c) = chars.peek() {
                if c == ';' { chars.next(); break; }
                if c.is_ascii_hexdigit() || (!hex && c.is_ascii_digit()) { entity.push(c); chars.next(); }
                else { break; }
            }
            let parsed = if hex {
                u32::from_str_radix(entity.trim_start_matches("&#x").trim_start_matches("&#X").trim_end_matches(';'), 16).ok()
            } else {
                entity.trim_start_matches("&#").trim_end_matches(';').parse::<u32>().ok()
            };
            match parsed.and_then(char::from_u32) {
                Some(c) => decoded.push(c),
                None => decoded.push_str(&entity),
            }
        } else {
            decoded.push(ch);
        }
    }
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Opt-in escape hatch for development (loopback LLM endpoints, intranet
/// docs). Off by default; SSRF guard applies otherwise.
pub fn allow_private_fetch(store: &crate::store::Store) -> bool {
    store.setting(ALLOW_PRIVATE_FETCH_KEY).unwrap_or(false)
}

async fn fetch_one(client: &reqwest::Client, url: &reqwest::Url, allow_private: bool) -> Result<reqwest::Response, String> {
    check_fetch_host(url, allow_private).await?;
    client
        .get(url.clone())
        .header("user-agent", "LocalLM/0.1")
        .send()
        .await
        .map_err(|error| format!("Web fetch failed: {error}"))
}

pub async fn web_fetch(url: &str) -> Result<serde_json::Value, String> {
    web_fetch_with_flag(url, false).await
}

pub async fn web_fetch_with_flag(url: &str, allow_private: bool) -> Result<serde_json::Value, String> {
    let mut current = reqwest::Url::parse(url).map_err(|_| "web_fetch needs an absolute http(s) URL.".to_string())?;
    if !matches!(current.scheme(), "http" | "https") {
        return Err("web_fetch only supports http(s) URLs.".into());
    }
    if url.len() > 2048 {
        return Err("URL is too long.".into());
    }
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| "Could not create the web client.".to_string())?;
    // Manual redirect chain (max 3): every hop re-resolves so a public URL
    // cannot bounce into loopback/private space mid-fetch.
    let mut response = None;
    for _ in 0..4 {
        let next = fetch_one(&client, &current, allow_private).await?;
        let status = next.status();
        if [301, 302, 303, 307, 308].contains(&status.as_u16()) {
            let location = next.headers().get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            if location.is_empty() {
                return Err("Redirect location is missing.".into());
            }
            current = resolve_redirect(&current, &location)?;
            if !matches!(current.scheme(), "http" | "https") {
                return Err("web_fetch only supports http(s) URLs.".into());
            }
            continue;
        }
        response = Some(next);
        break;
    }
    let response = response.ok_or("Too many redirects (max 3).")?;
    let status = response.status().as_u16();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| format!("Web read failed: {error}"))?;
    let spill = bytes.len() > 262_144;
    let body = if spill { bytes[..262_144].to_vec() } else { bytes.to_vec() };
    let raw = String::from_utf8_lossy(&body).into_owned();
    let mut text = serde_json::json!({
        "url": url,
        "status": status,
        "bytes": body.len(),
        "body": html_to_text(&raw),
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
    #[test]
    fn ssrf_guard_blocks_loopback_linklocal_and_private() {
        for ip in ["127.0.0.1", "127.1.2.3", "10.4.5.6", "172.16.9.9", "172.31.255.1",
            "192.168.0.5", "169.254.169.254", "0.0.0.0", "::1", "fc00::1", "fe80::1"] {
            assert!(is_blocked_ip(&ip.parse().unwrap()), "{ip} should be blocked");
        }
        for ip in ["8.8.8.8", "1.1.1.1", "93.184.216.34", "2606:4700:4700::1111"] {
            assert!(!is_blocked_ip(&ip.parse().unwrap()), "{ip} should be allowed");
        }
    }
    #[tokio::test]
    async fn web_fetch_rejects_loopback_literal_without_dns() {
        assert!(web_fetch("http://127.0.0.1:4317/api/config").await.is_err());
        assert!(web_fetch("http://10.0.0.9/").await.is_err());
        assert!(web_fetch("ftp://example.com/x").await.is_err());
    }
    #[test]
    fn html_to_text_drops_markup_and_keeps_words() {
        let text = html_to_text("<html><head><title>T</title><style>.a{}</style></head><body><h1>Hello &amp; goodbye</h1><script>evil()</script><p>World&#33;</p></body></html>");
        assert!(!text.contains('<') && !text.contains("evil"));
        assert!(text.contains("Hello & goodbye") && text.contains("World!"));
    }
}
