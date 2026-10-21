//! Explicit, conversation-owned local HTML previews. This is an execution
//! tool, not a relaxation of the public-network browser automation policy.
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

const LIFETIME: Duration = Duration::from_secs(3600);
const MAX_PREVIEWS: usize = 8;

pub fn registry() -> Vec<crate::tool_spec::ToolSpec> {
    vec![crate::tool_spec::ToolSpec::new(
        "workspace_open_preview",
        "Open a saved workspace HTML project in a visible Chrome or Edge window so the user can play or inspect it. Serves the HTML's directory and relative static assets on an owned, token-scoped loopback URL for one hour. Requires Workspace and Local execution, but not Browser/Computer automation or Playwright. Save and read back the HTML first. Reports launch requested, not a verified page load or gameplay test. Reopening replaces this conversation's previous preview. No arbitrary URL or shell command is accepted.",
        json!({"type":"object","properties":{
            "path":{"type":"string","minLength":1,"maxLength":4096,"description":"Workspace-relative .html or .htm file."},
            "browser":{"type":"string","enum":["chrome","msedge"],"default":"chrome"}
        },"required":["path"],"additionalProperties":false}),
    ).with_output(json!({"type":"object","required":["path","url","browser","launchRequested","pageVerified","expiresInSeconds"],"properties":{
        "path":{"type":"string"},"url":{"type":"string"},"browser":{"type":"string"},
        "launchRequested":{"const":true},"pageVerified":{"const":false},"expiresInSeconds":{"type":"integer"}
    }}))]
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    path: String,
    #[serde(default = "chrome")]
    browser: String,
}
fn chrome() -> String {
    "chrome".into()
}

struct Session(tokio::task::JoinHandle<()>);
impl Drop for Session {
    fn drop(&mut self) {
        self.0.abort();
    }
}
#[derive(Default)]
pub struct Manager {
    sessions: Mutex<HashMap<String, Session>>,
}
impl Manager {
    pub fn stop_conversation(&self, id: &str) {
        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.remove(id);
        }
    }
    pub async fn open(
        &self,
        owner: &str,
        root: &str,
        config: &crate::automation::Config,
        args: Value,
    ) -> Result<Value, String> {
        let args: Args = serde_json::from_value(args).map_err(|e| e.to_string())?;
        crate::workspace::valid_path(&args.path, false)?;
        if !matches!(
            Path::new(&args.path)
                .extension()
                .and_then(|v| v.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("html" | "htm")
        ) {
            return Err("Choose a saved .html or .htm file inside the workspace.".into());
        }
        let executable = browser_executable(config, &args.browser)?;
        let workspace = Arc::new(crate::workspace::Workspace::open(root)?);
        let path = args.path.clone();
        let read_workspace = workspace.clone();
        tokio::task::spawn_blocking(move || read_workspace.preview_asset(&path))
            .await
            .map_err(|_| "Preview file inspection failed.")??;
        let directory = Path::new(&args.path)
            .parent()
            .unwrap_or(Path::new(""))
            .to_string_lossy()
            .into_owned();
        let entry = Path::new(&args.path)
            .file_name()
            .and_then(|v| v.to_str())
            .ok_or("Invalid HTML filename.")?;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("Could not start local preview: {e}"))?;
        let host = listener
            .local_addr()
            .map_err(|e| e.to_string())?
            .to_string();
        let prefix = format!("/{}/", uuid::Uuid::new_v4());
        let mut url =
            reqwest::Url::parse(&format!("http://{host}{prefix}")).map_err(|e| e.to_string())?;
        url.path_segments_mut()
            .map_err(|_| "Invalid preview URL.")?
            .pop_if_empty()
            .push(entry);
        let session = Session(tokio::spawn(async move {
            let permits = Arc::new(tokio::sync::Semaphore::new(16));
            let expires = tokio::time::sleep(LIFETIME);
            tokio::pin!(expires);
            loop {
                let accepted = tokio::select! {
                    _ = &mut expires => break,
                    accepted = listener.accept() => accepted,
                };
                let Ok((stream, _)) = accepted else { break };
                let Ok(permit) = permits.clone().try_acquire_owned() else {
                    continue;
                };
                let (workspace, directory, host, prefix) = (
                    workspace.clone(),
                    directory.clone(),
                    host.clone(),
                    prefix.clone(),
                );
                tokio::spawn(async move {
                    let _permit = permit;
                    let _ = tokio::time::timeout(
                        Duration::from_secs(10),
                        serve(stream, workspace, &directory, &host, &prefix),
                    )
                    .await;
                });
            }
        }));
        // Keep the session local until registration succeeds. Drop aborts it
        // on any error, including an unavailable browser or a full registry.
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| "Preview registry unavailable.")?;
        sessions.retain(|_, session| !session.0.is_finished());
        if sessions.len() >= MAX_PREVIEWS && !sessions.contains_key(owner) {
            return Err(
                "Eight previews are open. Close a preview conversation before opening another."
                    .into(),
            );
        }
        let mut child = tokio::process::Command::new(executable)
            .arg("--new-window")
            .arg(url.as_str())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Could not launch {}: {e}", args.browser))?;
        // Reap the launcher; an existing browser may own the actual window.
        tokio::spawn(async move {
            let _ = child.wait().await;
        });
        sessions.insert(owner.into(), session);
        Ok(
            json!({"path":args.path,"url":url.as_str(),"browser":args.browser,
            "launchRequested":true,"pageVerified":false,"expiresInSeconds":LIFETIME.as_secs()}),
        )
    }
}

fn browser_executable(
    config: &crate::automation::Config,
    browser: &str,
) -> Result<PathBuf, String> {
    if !matches!(browser, "chrome" | "msedge") {
        return Err("Choose chrome or msedge.".into());
    }
    if config.browser_channel == browser && !config.browser_executable_path.is_empty() {
        let path = PathBuf::from(&config.browser_executable_path);
        if crate::execution::executable(&path.to_string_lossy()) {
            return Ok(path);
        }
        return Err(
            "The browser executable configured in Browser use settings is unavailable.".into(),
        );
    }
    let names: &[&str] = if browser == "chrome" {
        &[
            "google-chrome",
            "google-chrome-stable",
            "chrome",
            "chrome.exe",
        ]
    } else {
        &[
            "microsoft-edge",
            "microsoft-edge-stable",
            "msedge",
            "msedge.exe",
        ]
    };
    let mut candidates: Vec<PathBuf> = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .filter(|p| p.is_absolute())
        .flat_map(|p| names.iter().map(move |n| p.join(n)))
        .collect();
    #[cfg(target_os = "macos")]
    candidates.push(PathBuf::from(if browser == "chrome" {
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
    } else {
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge"
    }));
    #[cfg(windows)]
    for base in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
        if let Some(base) = std::env::var_os(base) {
            candidates.push(PathBuf::from(base).join(if browser == "chrome" {
                "Google/Chrome/Application/chrome.exe"
            } else {
                "Microsoft/Edge/Application/msedge.exe"
            }));
        }
    }
    // Keep mutable on platforms that add conventional installation paths.
    candidates.retain(|p| crate::execution::executable(&p.to_string_lossy()));
    candidates.into_iter().next().ok_or_else(|| format!("{} was not found. Install it or set its executable in Browser use settings. Browser automation need not be enabled for previews.", browser))
}

fn decode_path(raw: &str) -> Option<String> {
    let mut bytes = Vec::new();
    let mut input = raw.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = char::from(input.next()?).to_digit(16)?;
            let low = char::from(input.next()?).to_digit(16)?;
            bytes.push((high * 16 + low) as u8);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).ok()
}

fn request_path(request: &str, host: &str, prefix: &str) -> Result<(String, bool), u16> {
    let mut lines = request.split("\r\n");
    let mut first = lines.next().ok_or(400u16)?.split_whitespace();
    let method = first.next().ok_or(400u16)?;
    let target = first.next().ok_or(400u16)?;
    if first.next() != Some("HTTP/1.1") || first.next().is_some() {
        return Err(400);
    }
    if !matches!(method, "GET" | "HEAD") {
        return Err(405);
    }
    let hosts = lines
        .filter_map(|line| line.split_once(':'))
        .filter(|(key, _)| key.eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.trim())
        .collect::<Vec<_>>();
    if hosts != [host] {
        return Err(403);
    }
    let raw = target
        .split('?')
        .next()
        .unwrap_or(target)
        .strip_prefix(prefix)
        .ok_or(404u16)?;
    let path = decode_path(raw).ok_or(400u16)?;
    crate::workspace::valid_path(&path, false).map_err(|_| 403u16)?;
    Ok((path, method == "HEAD"))
}

async fn serve<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    workspace: Arc<crate::workspace::Workspace>,
    directory: &str,
    host: &str,
    prefix: &str,
) -> std::io::Result<()> {
    let mut request = Vec::new();
    let (status, head, asset) = loop {
        let mut buf = [0u8; 1024];
        let count = stream.read(&mut buf).await?;
        if count == 0 {
            return Ok(());
        }
        request.extend_from_slice(&buf[..count]);
        if request.len() > 16384 {
            break (431, false, None);
        }
        if request.windows(4).any(|w| w == b"\r\n\r\n") {
            let parsed = std::str::from_utf8(&request)
                .ok()
                .and_then(|r| request_path(r, host, prefix).ok());
            if let Some((path, head)) = parsed {
                let path = if directory.is_empty() {
                    path
                } else {
                    format!("{directory}/{path}")
                };
                let asset = tokio::task::spawn_blocking(move || workspace.preview_asset(&path))
                    .await
                    .ok()
                    .and_then(Result::ok);
                break (if asset.is_some() { 200 } else { 404 }, head, asset);
            }
            let status = std::str::from_utf8(&request)
                .map_or(400, |r| request_path(r, host, prefix).err().unwrap_or(400));
            break (status, false, None);
        }
    };
    let (bytes, mime) = asset.unwrap_or_else(|| {
        (
            b"Preview resource unavailable".to_vec(),
            "text/plain; charset=utf-8",
        )
    });
    let headers = format!("HTTP/1.1 {status} {}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: frame-ancestors 'none'\r\n\r\n", if status == 200 {"OK"} else {"Error"}, bytes.len());
    stream.write_all(headers.as_bytes()).await?;
    if !head {
        stream.write_all(&bytes).await?;
    }
    stream.shutdown().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_routes_require_owned_host_token_and_safe_relative_path() {
        let host = "127.0.0.1:1234";
        let prefix = "/owned-token/";
        let request =
            |target: &str, header: &str| format!("GET {target} HTTP/1.1\r\nHost: {header}\r\n\r\n");
        assert_eq!(
            request_path(
                &request("/owned-token/game%20one.html?v=2", host),
                host,
                prefix
            ),
            Ok(("game one.html".into(), false))
        );
        for target in [
            "/wrong/game.html",
            "/owned-token/../secret.html",
            "/owned-token/%2e%2e/secret.html",
            "/owned-token/%2Fetc/passwd",
            "/owned-token/a%5cb.html",
            "/owned-token/%00.html",
            "/owned-token/bad%ZZ.html",
        ] {
            assert!(
                request_path(&request(target, host), host, prefix).is_err(),
                "{target}"
            );
        }
        assert_eq!(
            request_path(
                &request("/owned-token/game.html", "attacker.example"),
                host,
                prefix
            ),
            Err(403)
        );
        assert!(request_path("GET /owned-token/game.html HTTP/1.1\r\nHost: 127.0.0.1:1234\r\nHost: attacker.example\r\n\r\n",host,prefix).is_err());
        assert_eq!(
            request_path(
                "POST /owned-token/game.html HTTP/1.1\r\nHost: 127.0.0.1:1234\r\n\r\n",
                host,
                prefix
            ),
            Err(405)
        );
    }

    // Exercise the real HTTP handler using in-memory I/O; no TCP permission,
    // real browser, display, provider or external service is needed.
    async fn response(
        workspace: Arc<crate::workspace::Workspace>,
        directory: &str,
        request: &str,
    ) -> Vec<u8> {
        let (mut client, server) = tokio::io::duplex(32768);
        let directory = directory.to_string();
        let task = tokio::spawn(async move {
            serve(
                server,
                workspace,
                &directory,
                "127.0.0.1:1234",
                "/owned-token/",
            )
            .await
            .unwrap();
        });
        client.write_all(request.as_bytes()).await.unwrap();
        let mut bytes = Vec::new();
        client.read_to_end(&mut bytes).await.unwrap();
        task.await.unwrap();
        bytes
    }

    #[tokio::test]
    async fn preview_serves_html_relative_assets_and_live_edits_without_exposing_siblings() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("game")).unwrap();
        std::fs::write(
            dir.path().join("game/index.html"),
            "<script src='game.js'></script>",
        )
        .unwrap();
        std::fs::write(dir.path().join("game/game.js"), "startGame()").unwrap();
        std::fs::write(dir.path().join("game/.secret.html"), "hidden").unwrap();
        std::fs::write(dir.path().join("game/secret.txt"), "private").unwrap();
        std::fs::write(dir.path().join("sibling.html"), "outside project").unwrap();
        let workspace =
            Arc::new(crate::workspace::Workspace::open(dir.path().to_str().unwrap()).unwrap());
        for (path, content, mime) in [
            ("index.html", "<script src='game.js'></script>", "text/html"),
            ("game.js", "startGame()", "text/javascript"),
        ] {
            let raw = response(
                workspace.clone(),
                "game",
                &format!("GET /owned-token/{path} HTTP/1.1\r\nHost: 127.0.0.1:1234\r\n\r\n"),
            )
            .await;
            let text = String::from_utf8(raw).unwrap();
            assert!(text.starts_with("HTTP/1.1 200"), "{text}");
            assert!(text.contains(mime));
            assert!(text.ends_with(content));
            assert!(text.contains("Cache-Control: no-store"));
        }
        std::fs::write(dir.path().join("game/game.js"), "updatedGame()").unwrap();
        let raw = response(
            workspace.clone(),
            "game",
            "GET /owned-token/game.js HTTP/1.1\r\nHost: 127.0.0.1:1234\r\n\r\n",
        )
        .await;
        assert!(String::from_utf8(raw).unwrap().ends_with("updatedGame()"));
        for path in [
            "secret.txt",
            ".secret.html",
            "../sibling.html",
            "missing.html",
        ] {
            let raw = response(
                workspace.clone(),
                "game",
                &format!("GET /owned-token/{path} HTTP/1.1\r\nHost: 127.0.0.1:1234\r\n\r\n"),
            )
            .await;
            let text = String::from_utf8(raw).unwrap();
            assert!(!text.starts_with("HTTP/1.1 200"), "{path}");
            assert!(
                !text.contains("private")
                    && !text.contains("outside project")
                    && !text.contains("hidden")
            );
        }
        let raw = response(
            workspace,
            "game",
            "HEAD /owned-token/game.js HTTP/1.1\r\nHost: 127.0.0.1:1234\r\n\r\n",
        )
        .await;
        assert!(String::from_utf8(raw).unwrap().ends_with("\r\n\r\n"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn preview_refuses_symlink_files_directories_and_special_files() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.html"), "never publish").unwrap();
        symlink(
            outside.path().join("secret.html"),
            dir.path().join("linked.html"),
        )
        .unwrap();
        symlink(outside.path(), dir.path().join("linked")).unwrap();
        std::fs::create_dir(dir.path().join("directory.html")).unwrap();
        let workspace =
            Arc::new(crate::workspace::Workspace::open(dir.path().to_str().unwrap()).unwrap());
        for path in ["linked.html", "linked/secret.html", "directory.html"] {
            let raw = response(
                workspace.clone(),
                "",
                &format!("GET /owned-token/{path} HTTP/1.1\r\nHost: 127.0.0.1:1234\r\n\r\n"),
            )
            .await;
            let text = String::from_utf8(raw).unwrap();
            assert!(text.starts_with("HTTP/1.1 404"), "{text}");
            assert!(!text.contains("never publish"));
        }
    }

    #[tokio::test]
    async fn closing_a_conversation_and_dropping_manager_abort_owned_sessions() {
        let manager = Manager::default();
        let handle = tokio::spawn(std::future::pending());
        let abort = handle.abort_handle();
        manager
            .sessions
            .lock()
            .unwrap()
            .insert("one".into(), Session(handle));
        manager.stop_conversation("another");
        assert!(!abort.is_finished());
        manager.stop_conversation("one");
        tokio::task::yield_now().await;
        assert!(abort.is_finished());
        let handle = tokio::spawn(std::future::pending());
        let abort = handle.abort_handle();
        manager
            .sessions
            .lock()
            .unwrap()
            .insert("two".into(), Session(handle));
        drop(manager);
        tokio::task::yield_now().await;
        assert!(abort.is_finished());
    }

    #[test]
    fn preview_requires_workspace_execution_and_mutation_approval() {
        assert!(crate::presets::harness_source_allowed(
            "workspace_open_preview",
            true,
            true
        ));
        for (workspace, execution) in [(false, false), (true, false), (false, true)] {
            assert!(!crate::presets::harness_source_allowed(
                "workspace_open_preview",
                workspace,
                execution
            ));
        }
        let tool = crate::connectors::AgentTool::harness("workspace_open_preview").unwrap();
        assert!(!tool.trusted_read());
        assert_eq!(
            crate::permissions::AccessMode::AutoApprove.automatic_reason(tool.trusted_read()),
            None
        );
        assert_eq!(
            crate::capabilities::tool_capability(&tool),
            Some("local_execution")
        );
        assert!(tool
            .prepare_arguments(json!({"path":"game.html","command":"anything"}))
            .is_err());
        assert!(tool
            .prepare_arguments(json!({"path":"game.html","browser":"arbitrary-executable"}))
            .is_err());
    }
}
