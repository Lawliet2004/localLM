//! Sandboxing, persistent terminal, FS policy, web tools, guards (Phase 5).
//!
//! Local execution is NOT an isolation boundary (see SPEC). Docker is the
//! opt-in boundary; every Docker call fails loud when the daemon is missing.
//! Approval answers WHETHER a tool runs; the sandbox provider answers WHERE.

use cap_fs_ext::DirExt;
use cap_std::{ambient_authority, fs::Dir};
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

/// Validate a container image reference.
///
/// The image occupies the **image slot** of a `docker run` command line.
/// Docker's CLI stops flag parsing at the first positional argument, so a
/// value that begins with `-` is read as a `docker run` flag — which would let
/// a caller ask for `--privileged`, `--net=host`, or mount the Docker socket.
/// A reference is therefore restricted to the characters a registry
/// reference can actually contain, and may not start with a dash.
pub fn validate_image(image: &str) -> Result<(), String> {
    let trimmed = image.trim();
    if trimmed.is_empty() || trimmed.len() > 255 {
        return Err("Container image must be 1-255 characters.".into());
    }
    if trimmed.starts_with('-') {
        return Err("Container image must not start with '-'.".into());
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/' | ':' | '@'))
    {
        return Err(
            "Container image may only contain letters, digits and . _ - / : @".into(),
        );
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
    validate_image(image)?;
    let timeout_secs = timeout_secs.clamp(1, 300);
    let child = tokio::process::Command::new("docker")
        .args([
            "run", "--rm", "--network", "none", "--cpus", "2", "--memory", "2g",
            "--volume", &format!("{workspace}:C:\\work"), "--workdir", "C:\\work",
            // `--` ends Docker's flag parsing, so the image can never be read
            // as a `docker run` option even if a future caller widens the
            // accepted reference syntax.
            "--", image,
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
pub async fn sandbox_status(
    state: tauri::State<'_, crate::AppState>,
) -> Result<serde_json::Value, String> {
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
        // The pinned image is still validated here: it lands in the image slot
        // of a `docker run` command line, and a reference that begins with `-`
        // would be read by Docker as a run option.
        validate_image(&image)?;
        store.save_setting(DOCKER_IMAGE_KEY, &image)?;
    }
    Ok(())
}

// ---- Persistent terminal sessions ----

pub struct TerminalSession {
    child: crate::execution::ExecutionChild,
    input: tokio::process::ChildStdin,
    output: tokio::process::ChildStdout,
    errors: tokio::process::ChildStderr,
    owner: String,
    powershell: bool,
    timeout: Duration,
}

pub struct TerminalRegistry {
    sessions: HashMap<String, TerminalSession>,
}
impl TerminalRegistry {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
        }
    }
    pub fn create(
        &mut self,
        config: &crate::execution::ExecutionConfig,
        workspace: &str,
        owner: &str,
    ) -> Result<String, String> {
        if self.sessions.len() >= 8 {
            return Err("At most 8 persistent terminal sessions.".into());
        }
        if !config.bash.enabled || config.bash_path.is_empty() {
            return Err("Enable and configure Bash/shell execution in Settings first.".into());
        }
        let execution = crate::execution::LocalExecution::new(config.clone(), workspace)?;
        let cwd = execution.shell_directory()?;
        let powershell = config.bash_path.to_ascii_lowercase().contains("powershell")
            || config.bash_path.to_ascii_lowercase().contains("pwsh");
        let mut command = tokio::process::Command::new(&config.bash_path);
        if powershell {
            command.args(["-NoProfile", "-NonInteractive", "-Command", "-"]);
        } else {
            command.args(["--noprofile", "--norc", "-s"]);
        }
        command
            .current_dir(cwd)
            .envs(&config.bash.environment)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        crate::execution::memory_limit(&mut command, &config.bash);
        let mut child = crate::execution::managed_spawn(command)
            .map_err(|e| format!("Could not start terminal shell: {e}"))?;
        let input = child.stdin().take().ok_or("Missing terminal input")?;
        let output = child.stdout().take().ok_or("Missing terminal output")?;
        let errors = child.stderr().take().ok_or("Missing terminal errors")?;
        let id = format!("term-{}", uuid::Uuid::new_v4());
        self.sessions.insert(
            id.clone(),
            TerminalSession {
                child,
                input,
                output,
                errors,
                owner: owner.into(),
                powershell,
                timeout: Duration::from_secs(config.bash.timeout_seconds),
            },
        );
        Ok(id)
    }
    pub async fn send(
        &mut self,
        id: &str,
        input: &str,
        owner: &str,
    ) -> Result<serde_json::Value, String> {
        use tokio::io::AsyncWriteExt;
        if input.is_empty() || input.len() > 8192 {
            return Err("Terminal input must contain 1–8192 bytes.".into());
        }
        if self
            .sessions
            .get(id)
            .is_none_or(|session| session.owner != owner)
        {
            return Err("Terminal session does not belong to this conversation.".into());
        }
        // Removing before await makes cancellation drop the child/tree, not
        // leave a half-executed command in the persistent registry.
        let mut session = self
            .sessions
            .remove(id)
            .ok_or("Terminal session no longer exists.")?;
        let sentinel = format!("__LOCALLM_DONE_{}__", uuid::Uuid::new_v4().simple());
        let payload = if session.powershell {
            format!("[Console]::InputEncoding=[System.Text.UTF8Encoding]::new(); [Console]::OutputEncoding=[System.Text.UTF8Encoding]::new(); $OutputEncoding=[Console]::OutputEncoding\n$global:LASTEXITCODE=0\n{input}\n$locallm_status=if ($?) {{0}} elseif ($LASTEXITCODE) {{$LASTEXITCODE}} else {{1}}\n[Console]::Out.WriteLine(\"{sentinel}:$locallm_status\"); [Console]::Error.WriteLine(\"{sentinel}:$locallm_status\")\n")
        } else {
            format!("{input}\n__locallm_status=$?\nprintf '\\n{sentinel}:%s\\n' \"$__locallm_status\"\nprintf '\\n{sentinel}:%s\\n' \"$__locallm_status\" >&2\n")
        };
        let started = std::time::Instant::now();
        let operation = async {
            session
                .input
                .write_all(payload.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            session.input.flush().await.map_err(|e| e.to_string())?;
            tokio::try_join!(
                read_until_sentinel(&mut session.output, &sentinel),
                read_until_sentinel(&mut session.errors, &sentinel)
            )
        };
        let ((stdout, exit), (stderr, _)) = tokio::time::timeout(session.timeout, operation).await
            .map_err(|_| "Terminal command timed out. The process tree was stopped and this session is closed.".to_string())??;
        self.sessions.insert(id.to_string(), session);
        Ok(
            serde_json::json!({"stdout":stdout,"stderr":stderr,"exitCode":exit,"isError":exit != 0,"durationMs":started.elapsed().as_millis()}),
        )
    }
    pub async fn close(&mut self, id: &str, owner: &str) -> Result<(), String> {
        if self
            .sessions
            .get(id)
            .is_none_or(|session| session.owner != owner)
        {
            return Err("Terminal session does not belong to this conversation.".into());
        }
        let mut session = self
            .sessions
            .remove(id)
            .ok_or("Terminal session no longer exists.")?;
        let _ = session.child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(3), session.child.wait()).await;
        Ok(())
    }
    /// Close every terminal session.
    ///
    /// Tauri ends the application with `std::process::exit`, so no destructor
    /// runs and the `Drop`-based kill never fires. This is the graceful path
    /// that terminates each shell and reaps it before the process exits.
    pub async fn close_all(&mut self) {
        let ids: Vec<String> = self.sessions.keys().cloned().collect();
        for id in ids {
            let Some(mut session) = self.sessions.remove(&id) else {
                continue;
            };
            let _ = session.child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(3), session.child.wait()).await;
        }
    }
    /// Close every terminal session owned by one conversation.
    ///
    /// Deleting a conversation must not leave its shells running with no
    /// reachable handle.
    pub async fn close_owned(&mut self, owner: &str) {
        let owned: Vec<String> = self
            .sessions
            .iter()
            .filter(|(_, session)| session.owner == owner)
            .map(|(id, _)| id.clone())
            .collect();
        for id in owned {
            let Some(mut session) = self.sessions.remove(&id) else {
                continue;
            };
            let _ = session.child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(3), session.child.wait()).await;
        }
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
    stream: &mut (impl tokio::io::AsyncRead + Unpin),
    sentinel: &str,
) -> Result<(String, i32), String> {
    use tokio::io::AsyncReadExt;
    let mut collected = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let count = stream.read(&mut buffer).await.map_err(|e| e.to_string())?;
        if count == 0 {
            return Err("Terminal exited before command completion. The session is closed.".into());
        }
        collected.extend_from_slice(&buffer[..count]);
        if collected.len() > 65536 + sentinel.len() + 32 {
            return Err("Terminal output exceeded 64 KiB. The process tree was stopped and this session is closed.".into());
        }
        let text = String::from_utf8_lossy(&collected);
        if let Some(position) = text.find(sentinel) {
            if let Some(end) = text[position..].find('\n') {
                let code = text[position + sentinel.len()..position + end]
                    .trim()
                    .strip_prefix(':')
                    .and_then(|s| s.parse::<i32>().ok())
                    .ok_or("Invalid terminal completion marker")?;
                let output = text[..position]
                    .strip_suffix('\n')
                    .unwrap_or(&text[..position]);
                if output.len() > 65536 {
                    return Err("Terminal output exceeded 64 KiB. The session is closed.".into());
                }
                return Ok((output.to_string(), code));
            }
        }
    }
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
        if part == ".locallm-transactions" || part.starts_with(".locallm-txn-") {
            return Err(
                "Internal workspace transaction files are only accessible through recover_patch."
                    .into(),
            );
        }
        if DENY_DIRS.contains(&part) {
            return Err("Access under .ssh is never allowed.".into());
        }
    }
    if let Some(base) = lowered.rsplit('/').next() {
        if DENY_BASENAMES
            .iter()
            .any(|denied| base == *denied || base.starts_with(".env."))
        {
            return Err(
                "Credential files (.env, private keys) are never readable by the model.".into(),
            );
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
        return Err(format!(
            "File exceeds the {} KiB tool limit; read a smaller range.",
            limit / 1024
        ));
    }
    if bytes.iter().take(8192).any(|byte| *byte == 0) {
        return Err("Binary files are not readable as text.".into());
    }
    Ok(())
}

// ---- Bounded web tools ----

pub const ALLOW_PRIVATE_FETCH_KEY: &str = "sandbox.allow_private_fetch";

/// Whether an address is inside a range the app must never reach: loopback,
/// link-local, private, or otherwise reserved.
///
/// Every outbound HTTP path uses this so a model, a hostile page or a
/// malicious MCP server cannot use one tool to reach a service the other
/// tools refuse.
pub fn is_blocked_ip(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_unspecified()
                || v4.is_multicast()
                || v4.is_link_local()
                || o[0] == 0
                || o[0] >= 224
                || o[0] == 10
                || (o[0] == 172 && (16..=31).contains(&o[1]))
                || (o[0] == 192 && o[1] == 168)
                || (o[0] == 169 && o[1] == 254)
                || (o[0] == 100 && (64..=127).contains(&o[1]))
                || (o[0] == 192 && o[1] == 0 && o[2] == 2)
                || (o[0] == 192 && o[1] == 0 && o[2] == 0)
                || (o[0] == 198 && o[1] == 51 && o[2] == 100)
                || (o[0] == 198 && (18..=19).contains(&o[1]))
                || (o[0] == 203 && o[1] == 0 && o[2] == 113)
                || (o[0] == 192 && o[1] == 88 && o[2] == 99)
        }
        std::net::IpAddr::V6(v6) => {
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_blocked_ip(&std::net::IpAddr::V4(mapped));
            }
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (v6.segments()[0] & 0xe000) != 0x2000
                || (v6.segments()[0] == 0x2001 && (v6.segments()[1] & 0xfe00) == 0)
                || (v6.segments()[0] == 0x2001 && v6.segments()[1] == 0x0db8)
                || v6.segments()[0] == 0x2002
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// DNS answers that were validated for one fetch hop. The addresses are
/// pinned onto the reqwest client that performs the request so the socket
/// cannot re-resolve the name to a different (rebound) answer.
/// A URL whose host has been resolved and checked against the private-address
/// policy. The vetted addresses are pinned onto the client that performs the
/// request, so the socket cannot re-resolve the name to a different answer.
pub struct VettedTarget {
    /// Hostname for `resolve_to_addrs` when DNS pinning applies; `None` for
    /// literal-IP URLs, which reqwest dials directly.
    pub domain: Option<String>,
    /// Every validated address for the host (URL port applied by reqwest).
    pub addrs: Vec<std::net::SocketAddr>,
}

/// Resolve a URL's host once and refuse private, loopback, link-local and
/// cloud-metadata answers.
///
/// `allow_private` opts into the private range; it exists for the user's own
/// loopback fixtures and is never set by a model-facing tool.
pub async fn vet_fetch_target(
    url: &reqwest::Url,
    allow_private: bool,
) -> Result<VettedTarget, String> {
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("web_fetch requires HTTP(S) without embedded credentials.".into());
    }
    let host = url.host_str().ok_or("web_fetch needs a host.")?;
    let port = url.port_or_known_default().unwrap_or(443);
    // IPv6 literals arrive bracketed (host_str gives "[::1]"); validate the
    // literal address rather than attempting a DNS lookup on the brackets.
    let literal = host
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = literal.parse::<std::net::IpAddr>() {
        if !allow_private && is_blocked_ip(&ip) {
            return Err(
                "web_fetch blocked: loopback, link-local, and private addresses are not fetchable."
                    .into(),
            );
        }
        return Ok(VettedTarget {
            domain: None,
            addrs: Vec::new(),
        });
    }
    let addrs: Vec<std::net::SocketAddr> = tokio::time::timeout(
        Duration::from_secs(4),
        tokio::net::lookup_host((host, port)),
    )
    .await
    .map_err(|_| "web_fetch host resolution timed out.".to_string())?
    .map_err(|_| "web_fetch could not resolve the host.".to_string())?
    .collect();
    if addrs.is_empty() {
        return Err("web_fetch could not resolve the host.".to_string());
    }
    for addr in &addrs {
        if !allow_private && is_blocked_ip(&addr.ip()) {
            return Err(
                "web_fetch blocked: host resolves to a loopback, link-local, or private address."
                    .into(),
            );
        }
    }
    Ok(VettedTarget {
        domain: Some(host.to_string()),
        addrs,
    })
}

fn resolve_redirect(current: &reqwest::Url, location: &str) -> Result<reqwest::Url, String> {
    reqwest::Url::parse(location)
        .or_else(|_| current.join(location))
        .map_err(|_| "Redirect location is invalid.".to_string())
}

/// Strip markup to plain readable text: drop head/style/script/noscript
/// blocks, replace tags with spaces, decode common entities, collapse space.
pub fn html_to_text(html: &str) -> String {
    let mut without_blocks = html.to_string();
    for tag in ["head", "style", "script", "noscript"] {
        // HTML tag names are ASCII. Unicode lowercasing can change byte length
        // and invalidate offsets into the original UTF-8 string.
        let lower = without_blocks.to_ascii_lowercase();
        let opening = format!("<{tag}");
        let closing = format!("</{tag}>");
        let mut next = String::with_capacity(without_blocks.len());
        let mut search_from = 0;
        let mut copy_from = 0;
        while let Some(relative) = lower[search_from..].find(&opening) {
            let start = search_from + relative;
            let boundary = lower.as_bytes().get(start + opening.len());
            if !boundary
                .is_some_and(|byte| *byte == b'>' || *byte == b'/' || byte.is_ascii_whitespace())
            {
                search_from = start + opening.len();
                continue;
            }
            let Some(open_end) = lower[start..].find('>') else {
                break;
            };
            let after_open = start + open_end + 1;
            next.push_str(&without_blocks[copy_from..start]);
            next.push(' ');
            let Some(close_at) = lower[after_open..].find(&closing) else {
                copy_from = without_blocks.len(); // An unclosed script/style consumes the remaining document.
                break;
            };
            copy_from = after_open + close_at + closing.len();
            search_from = copy_from;
        }
        next.push_str(&without_blocks[copy_from..]);
        without_blocks = next; // Owned buffers are released on every pass.
    }
    let mut text = String::with_capacity(without_blocks.len());
    let mut in_tag = false;
    for ch in without_blocks.chars() {
        match ch {
            '<' => {
                in_tag = true;
                text.push(' ');
            }
            '>' => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(ch),
            _ => {}
        }
    }
    for (entity, replacement) in [
        ("&amp;", "&"),
        ("&lt;", "<"),
        ("&gt;", ">"),
        ("&quot;", "\""),
        ("&#39;", "'"),
        ("&apos;", "'"),
        ("&nbsp;", " "),
        ("&copy;", "(c)"),
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
            if hex {
                entity.push(chars.next().unwrap());
            }
            while let Some(&c) = chars.peek() {
                if c == ';' {
                    chars.next();
                    break;
                }
                if c.is_ascii_hexdigit() || (!hex && c.is_ascii_digit()) {
                    entity.push(c);
                    chars.next();
                } else {
                    break;
                }
            }
            let parsed = if hex {
                u32::from_str_radix(
                    entity
                        .trim_start_matches("&#x")
                        .trim_start_matches("&#X")
                        .trim_end_matches(';'),
                    16,
                )
                .ok()
            } else {
                entity
                    .trim_start_matches("&#")
                    .trim_end_matches(';')
                    .parse::<u32>()
                    .ok()
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

/// Maximum response bytes web_fetch will consume, matching the historical
/// 256 KiB excerpt cap.
const FETCH_MAX_BYTES: usize = 262_144;

/// One validated fetch hop: resolve once, block private/loopback answers
/// (unless the explicit private-network opt-in is set), then pin the vetted
/// addresses on the client performing the request. Proxies are disabled
/// because a proxy would resolve and connect itself, bypassing the pin.
/// A fresh client is built per hop so redirect targets get their own pin.
async fn fetch_one(url: &reqwest::Url, allow_private: bool) -> Result<reqwest::Response, String> {
    let vetted = vet_fetch_target(url, allow_private).await?;
    fetch_vetted(url, vetted).await
}

async fn fetch_vetted(
    url: &reqwest::Url,
    vetted: VettedTarget,
) -> Result<reqwest::Response, String> {
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .no_proxy();
    if let Some(domain) = vetted.domain {
        if !vetted.addrs.is_empty() {
            builder = builder.resolve_to_addrs(&domain, &vetted.addrs);
        }
    }
    let client = builder
        .build()
        .map_err(|_| "Could not create the web client.".to_string())?;
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

pub async fn web_fetch_with_flag(
    url: &str,
    allow_private: bool,
) -> Result<serde_json::Value, String> {
    let mut current = reqwest::Url::parse(url)
        .map_err(|_| "web_fetch needs an absolute http(s) URL.".to_string())?;
    if !matches!(current.scheme(), "http" | "https") {
        return Err("web_fetch only supports http(s) URLs.".into());
    }
    if url.len() > 2048 {
        return Err("URL is too long.".into());
    }
    // Manual redirect chain (max 3): every hop re-vets and re-pins so a
    // public URL cannot bounce into loopback/private space mid-fetch.
    let mut response = None;
    for _ in 0..4 {
        let next = fetch_one(&current, allow_private).await?;
        let status = next.status();
        if [301, 302, 303, 307, 308].contains(&status.as_u16()) {
            let location = next
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_string();
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
    let mut response = response.ok_or("Too many redirects (max 3).")?;
    let status = response.status().as_u16();
    // Content-Length is only an early hint (and can lie); the stream read
    // below is the real bound and stops at the cap regardless.
    let declared = response.content_length();
    let mut spill = declared.is_some_and(|len| len > FETCH_MAX_BYTES as u64);
    let mut body =
        Vec::with_capacity(declared.map_or(8192, |len| len.min(FETCH_MAX_BYTES as u64) as usize));
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| format!("Web read failed: {error}"))?
    {
        let room = FETCH_MAX_BYTES.saturating_sub(body.len());
        if chunk.len() > room {
            body.extend_from_slice(&chunk[..room]);
            spill = true;
            break;
        }
        body.extend_from_slice(&chunk);
    }
    // Dropping the response aborts the socket early by design; the remainder
    // of an oversized body is never pulled into memory.
    let raw = String::from_utf8_lossy(&body).into_owned();
    let mut text = serde_json::json!({
        "url": url,
        "status": status,
        "bytes": body.len(),
        "body": html_to_text(&raw),
    });
    if spill {
        text["truncated"] =
            serde_json::json!("Body exceeded 256 KiB; remainder spilled. Narrow the request.");
    }
    Ok(text)
}

/// Fixed-string file search inside the workspace. Traversal and opens go
/// through capability `Dir` handles, so no path component — including one
/// swapped for a link mid-walk — can resolve outside the root. Links are
/// never entered or read, `check_fs_path` credential policy applies to every
/// candidate file, and per-file/aggregate read budgets keep total work
/// bounded. Skips hidden dirs, target-style build output, and dependency
/// folders.
pub fn file_search(root: &std::path::Path, needle: &str) -> Result<Vec<String>, String> {
    use std::io::Read;
    if needle.is_empty() || needle.len() > 200 {
        return Err("Search text must be 1-200 characters.".into());
    }
    const SKIP: [&str; 6] = [
        ".git",
        "node_modules",
        "target",
        ".venv",
        "dist",
        "__pycache__",
    ];
    const MAX_FILES: usize = 2000;
    const MAX_HITS: usize = 50;
    const MAX_FILE_BYTES: u64 = 524_288;
    const MAX_DEPTH: usize = 32;
    const READ_BUDGET: usize = 8 * 1024 * 1024;
    let root_dir = Dir::open_ambient_dir(root, ambient_authority())
        .map_err(|error| format!("Could not search workspace: {error}"))?;
    let mut hits = Vec::new();
    let mut files_seen = 0usize;
    let mut entries_seen = 0usize;
    let mut bytes_read = 0usize;
    let mut budget_exhausted = false;
    let mut stack = vec![(root_dir, String::new(), 0usize)];
    'walk: while let Some((dir, prefix, depth)) = stack.pop() {
        let entries = dir
            .entries()
            .map_err(|error| format!("Could not search workspace: {error}"))?;
        for entry in entries.filter_map(|entry| entry.ok()) {
            entries_seen += 1;
            if entries_seen > 20_000 {
                hits.push("… (file_search stopped at its directory entry limit; results may be incomplete)".into());
                break 'walk;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || SKIP.contains(&name.as_str()) {
                continue;
            }
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(_) => continue,
            };
            // Links are never followed or reported, at any depth.
            if kind.is_symlink() {
                continue;
            }
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if kind.is_dir() {
                if depth < MAX_DEPTH {
                    if let Ok(child) = dir.open_dir_nofollow(&name) {
                        stack.push((child, relative, depth + 1));
                    }
                }
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            files_seen += 1;
            if files_seen > MAX_FILES {
                return Err(
                    "Workspace is too large for file_search; narrow the workspace folder.".into(),
                );
            }
            // Same sensitive-file policy as the workspace tools.
            if check_fs_path("", &relative, false).is_err() {
                continue;
            }
            if bytes_read >= READ_BUDGET {
                budget_exhausted = true;
                break 'walk;
            }
            // The open itself refuses links and non-regular files. A metadata
            // check before an ordinary open would leave a symlink/FIFO race.
            let file = match crate::workspace::open_file_no_links(
                &dir,
                &name,
                cap_std::fs::OpenOptions::new().read(true),
            ) {
                Ok(file) => file,
                Err(_) => continue,
            };
            // Metadata size is an early optimization only; the bounded read
            // below is enforced even if the file grows before or during it.
            match file.metadata() {
                Ok(meta) if meta.len() <= MAX_FILE_BYTES => {}
                _ => continue,
            }
            let mut bytes = Vec::new();
            if (&file)
                .take((MAX_FILE_BYTES + 1).min((READ_BUDGET - bytes_read) as u64))
                .read_to_end(&mut bytes)
                .is_err()
            {
                continue;
            }
            bytes_read += bytes.len();
            if bytes_read >= READ_BUDGET {
                budget_exhausted = true;
            }
            if bytes.len() as u64 > MAX_FILE_BYTES {
                continue;
            }
            if bytes.iter().take(1024).any(|byte| *byte == 0) {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            let display = relative
                .split('/')
                .fold(root.to_path_buf(), |path, part| path.join(part))
                .to_string_lossy()
                .into_owned();
            for (number, line) in text.lines().enumerate() {
                if line.contains(needle) {
                    hits.push(format!(
                        "{}:{}: {}",
                        display,
                        number + 1,
                        line.chars().take(200).collect::<String>()
                    ));
                    if hits.len() >= MAX_HITS {
                        break 'walk;
                    }
                    break;
                }
            }
        }
    }
    if budget_exhausted {
        hits.push(
            "… (file_search stopped at its aggregate read budget; results may be incomplete)"
                .to_string(),
        );
    }
    Ok(hits)
}

// ---- Guards ----

/// Loop hygiene: three identical consecutive tool calls in a row end the
/// streak with an errored result instead of burning more rounds.
pub fn check_repetition(
    history: &[(String, String)],
    alias: &str,
    arguments: &str,
) -> Option<String> {
    // Repeated polls are how a model follows a quiet long-running process.
    // The owned process lifetime, output and turn budgets remain bounded.
    if matches!(alias, "process_poll" | "check_report") {
        return None;
    }
    if history.len() >= 2
        && history
            .iter()
            .rev()
            .take(2)
            .all(|(name, args)| name == alias && args == arguments)
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
    #[cfg(unix)]
    #[tokio::test]
    async fn persistent_bash_preserves_state_binds_owner_and_closes_failed_sessions() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join("sub")).unwrap();
        let mut config = crate::execution::ExecutionConfig::default();
        config.bash.timeout_seconds = 1;
        let mut registry = super::TerminalRegistry::new();
        let id = registry
            .create(&config, temp.path().to_str().unwrap(), "owner")
            .unwrap();
        assert!(registry.send(&id, "echo denied", "other").await.is_err());
        assert!(registry.close(&id, "other").await.is_err());
        let result = registry
            .send(
                &id,
                "cd sub; value='héllo世界'; printf '%s' \"$value\"",
                "owner",
            )
            .await
            .unwrap();
        assert_eq!(result["stdout"], "héllo世界");
        let result = registry
            .send(
                &id,
                "printf '%s:%s' \"$value\" \"${PWD##*/}\"; printf 'problem' >&2; false",
                "owner",
            )
            .await
            .unwrap();
        assert_eq!(result["stdout"], "héllo世界:sub");
        assert_eq!(result["stderr"], "problem");
        assert_eq!(result["exitCode"], 1);
        assert_eq!(result["isError"], true);
        assert!(registry
            .send(&id, "sleep 10; touch orphan-marker", "owner")
            .await
            .unwrap_err()
            .contains("timed out"));
        assert!(!registry.ids().contains(&id));
        let id = registry
            .create(&config, temp.path().to_str().unwrap(), "owner")
            .unwrap();
        registry.close(&id, "owner").await.unwrap();
        assert!(!registry.ids().contains(&id));
    }
    use super::*;
    #[tokio::test]
    async fn vetted_dns_addresses_are_used_by_the_actual_socket() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 4096];
            let count = socket.read(&mut request).await.unwrap();
            assert!(String::from_utf8_lossy(&request[..count]).contains("does-not-resolve.invalid"));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nokay")
                .await
                .unwrap();
        });
        let url = reqwest::Url::parse(&format!(
            "http://does-not-resolve.invalid:{}/",
            address.port()
        ))
        .unwrap();
        // A private fixture address stands in for a previously vetted public
        // DNS answer. The invalid hostname proves no second resolver is used.
        let response = fetch_vetted(
            &url,
            VettedTarget {
                domain: Some("does-not-resolve.invalid".into()),
                addrs: vec![address],
            },
        )
        .await
        .unwrap();
        assert_eq!(response.text().await.unwrap(), "okay");
        server.await.unwrap();
        for target in [
            "http://[::ffff:10.0.0.1]/",
            "http://198.51.100.1/",
            "http://240.0.0.1/",
            "http://[2001:db8::1]/",
            "http://name:secret@1.1.1.1/",
        ] {
            assert!(
                vet_fetch_target(&reqwest::Url::parse(target).unwrap(), false)
                    .await
                    .is_err(),
                "{target}"
            );
        }
    }
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
        let history = vec![
            ("t".to_string(), "{}".to_string()),
            ("t".to_string(), "{}".to_string()),
        ];
        assert!(check_repetition(&history, "t", "{}").is_some());
        assert!(check_repetition(&history, "t", "{\"x\":1}").is_none());
        assert!(check_repetition(&history[..1], "t", "{}").is_none());
    }
    #[test]
    fn file_search_finds_text_and_skips_deps() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello needle world").unwrap();
        std::fs::create_dir(dir.path().join("node_modules")).unwrap();
        std::fs::write(
            dir.path().join("node_modules").join("b.txt"),
            "needle hidden",
        )
        .unwrap();
        let hits = file_search(dir.path(), "needle").unwrap();
        assert_eq!(hits.len(), 1);
        assert!(hits[0].contains("a.txt"));
    }
    #[cfg(unix)]
    #[test]
    fn file_search_never_reads_through_links() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("linked.txt"), "needle outside file").unwrap();
        std::fs::create_dir(outside.path().join("private")).unwrap();
        std::fs::write(
            outside.path().join("private").join("inner.txt"),
            "needle outside dir",
        )
        .unwrap();
        symlink(
            outside.path().join("linked.txt"),
            root.path().join("linked-file.txt"),
        )
        .unwrap();
        symlink(
            outside.path().join("private"),
            root.path().join("linked-dir"),
        )
        .unwrap();
        let hits = file_search(root.path(), "needle").unwrap();
        assert!(hits.is_empty(), "search escaped through links: {hits:?}");
    }
    #[test]
    fn file_search_applies_credential_policy_and_size_cap() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("secret.pem"), "needle fake key").unwrap();
        std::fs::write(dir.path().join("app.env"), "needle visible").unwrap();
        std::fs::write(dir.path().join("big.log"), "x".repeat(600_000)).unwrap();
        std::fs::write(dir.path().join("ok.txt"), "needle here").unwrap();
        let hits = file_search(dir.path(), "needle").unwrap();
        assert_eq!(
            hits.len(),
            2,
            "credential and oversized files must be skipped: {hits:?}"
        );
        assert!(hits
            .iter()
            .all(|hit| hit.contains("ok.txt") || hit.contains("app.env")));
    }
    #[tokio::test]
    async fn vet_fetch_target_validates_literals_and_private_opt_in() {
        // Literal IPs need no DNS and would be dialed directly.
        let url = reqwest::Url::parse("http://8.8.8.8:8080/x").unwrap();
        let vetted = vet_fetch_target(&url, false).await.unwrap();
        assert!(vetted.domain.is_none() && vetted.addrs.is_empty());
        // Bracketed IPv6 literals parse as literals: public passes, private blocked.
        let url = reqwest::Url::parse("http://[2606:4700:4700::1111]/x").unwrap();
        assert!(vet_fetch_target(&url, false).await.is_ok());
        for url in [
            "http://[::1]/",
            "http://[fc00::1]/",
            "http://[fe80::1]/",
            "http://[::ffff:10.0.0.1]/",
        ] {
            let url = reqwest::Url::parse(url).unwrap();
            assert!(
                vet_fetch_target(&url, false).await.is_err(),
                "{url} should be blocked"
            );
        }
        // The explicit private-network opt-in still permits them.
        let url = reqwest::Url::parse("http://127.0.0.1:4317/x").unwrap();
        assert!(vet_fetch_target(&url, true).await.is_ok());
    }
    #[tokio::test]
    async fn web_fetch_private_opt_in_reads_bounded_stream() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 8192];
            let _ = socket.read(&mut buf).await;
            // Advertise a much larger body than the read cap, then stall so a
            // full-body read would block until the stall finishes.
            let body = "A".repeat(300_000);
            let head = format!("HTTP/1.1 200 OK\r\ncontent-length: 10000000\r\ncontent-type: text/plain\r\n\r\n{body}");
            socket.write_all(head.as_bytes()).await.unwrap();
            tokio::time::sleep(Duration::from_secs(5)).await;
        });
        let start = std::time::Instant::now();
        let result = web_fetch_with_flag(&format!("http://127.0.0.1:{port}/big"), true)
            .await
            .unwrap();
        assert!(
            result["truncated"].is_string(),
            "oversized body must be flagged"
        );
        assert!(result["bytes"].as_u64().unwrap() <= 262_144);
        assert!(result["body"].as_str().unwrap().len() <= 262_144);
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "read must stop at the cap instead of draining the declared body"
        );
        server.await.unwrap();
    }
    #[tokio::test]
    async fn web_fetch_rejects_ipv6_loopback_literal() {
        assert!(web_fetch("http://[::1]:4317/api").await.is_err());
        assert!(web_fetch("http://[fe80::1]/").await.is_err());
    }
    #[test]
    fn ssrf_guard_blocks_loopback_linklocal_and_private() {
        for ip in [
            "127.0.0.1",
            "127.1.2.3",
            "10.4.5.6",
            "172.16.9.9",
            "172.31.255.1",
            "192.168.0.5",
            "169.254.169.254",
            "0.0.0.0",
            "::1",
            "fc00::1",
            "fe80::1",
        ] {
            assert!(
                is_blocked_ip(&ip.parse().unwrap()),
                "{ip} should be blocked"
            );
        }
        for ip in [
            "8.8.8.8",
            "1.1.1.1",
            "93.184.216.34",
            "2606:4700:4700::1111",
        ] {
            assert!(
                !is_blocked_ip(&ip.parse().unwrap()),
                "{ip} should be allowed"
            );
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
    #[test]
    fn html_to_text_preserves_unicode_offsets_and_handles_repeated_blocks() {
        assert_eq!(html_to_text("İ<script>hidden</script>"), "İ");
        assert_eq!(
            html_to_text("İ<STYLE>hidden</STYLE><p>界 visible</p>"),
            "İ 界 visible"
        );
        assert_eq!(
            html_to_text(&format!(
                "{}<p>visible</p>",
                "<script>hidden</script>".repeat(500)
            )),
            "visible"
        );
        assert_eq!(html_to_text("<scripture>visible</scripture>"), "visible");
    }
}
