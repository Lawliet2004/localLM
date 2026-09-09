//! Local MCP process ownership. Explicit connection is the only launch operation.
use crate::local_mcp_config::LocalServer;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use rmcp::{service::RunningService, RoleClient, ServiceExt};
use std::{process::Stdio, time::Duration};
use tokio::io::AsyncRead;

/// Maximum bytes accepted for one newline-delimited stdio frame (including
/// the delimiter). The pinned SDK transport reads unbounded lines, so bound
/// frames here before handing the pipe to the SDK service.
pub const MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

pub struct LocalSession {
    pub service: RunningService<RoleClient, ()>,
    // KillOnDrop and the Windows job own the entire process tree, including when
    // handshake cancellation drops the future before a session is returned.
    child: Box<dyn ChildWrapper>,
}

impl LocalSession {
    pub async fn close(mut self) -> ShutdownReport {
        let protocol = tokio::time::timeout(Duration::from_secs(3), self.service.cancel())
            .await
            .map(|result| result.is_ok())
            .unwrap_or(false);
        let signal = self.child.start_kill().is_ok();
        let reaped = tokio::time::timeout(Duration::from_secs(3), self.child.wait())
            .await
            .map(|result| result.is_ok())
            .unwrap_or(false);
        ShutdownReport { protocol, signal, reaped }
    }
}

/// What each shutdown stage accomplished. A failed reap keeps the process
/// under KillOnDrop/job ownership and surfaces a retryable disconnect error.
#[derive(Clone, Copy, Debug, Default)]
pub struct ShutdownReport {
    pub protocol: bool,
    pub signal: bool,
    pub reaped: bool,
}

pub async fn connect(config: &LocalServer) -> Result<LocalSession, String> {
    connect_with_timeout(config, Duration::from_secs(30)).await
}

async fn connect_with_timeout(
    config: &LocalServer,
    handshake_timeout: Duration,
) -> Result<LocalSession, String> {
    config.validate_launch()?;
    let mut command = tokio::process::Command::new(&config.executable);
    command
        .args(&config.arguments)
        .current_dir(&config.working_directory)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Server stderr can contain credentials. Discard it without an unread
        // pipe that could block negotiation; protocol errors remain actionable.
        .stderr(Stdio::null());
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
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    command
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .env("NO_COLOR", "1")
        .envs(&config.environment);
    let mut command = CommandWrap::from(command);
    command.wrap(KillOnDrop);
    #[cfg(windows)]
    {
        command.wrap(process_wrap::tokio::CreationFlags(
            windows::Win32::System::Threading::CREATE_NO_WINDOW,
        ));
        command.wrap(process_wrap::tokio::JobObject);
    }
    #[cfg(unix)]
    command.wrap(process_wrap::tokio::ProcessGroup::leader());
    let mut child = command.spawn().map_err(|error| {
        format!(
            "Local connector launch failed ({:?}). Check its executable and working directory.",
            error.kind()
        )
    })?;
    let input = child
        .stdin()
        .take()
        .ok_or("Local connector input pipe unavailable.")?;
    let output = child
        .stdout()
        .take()
        .ok_or("Local connector output pipe unavailable.")?;
    let output = FramedStdout::new(output);
    let service = tokio::time::timeout(handshake_timeout, ().serve((output, input)))
        .await
        .map_err(|_| "Local connector handshake timed out.")?
        .map_err(|_| {
            "Local connector handshake failed. Check its arguments and MCP stdio support."
        })?;
    Ok(LocalSession { service, child })
}

/// Newline-delimited stdout wrapper enforcing [`MAX_FRAME_BYTES`] per frame.
/// A frame that exceeds the bound fails the read, which the SDK surfaces as a
/// closed/failed transport instead of buffering it without limit. Because the
/// caller reads through 8 KiB chunks, a frame is rejected once more than the
/// bound has been observed without a newline, even if the newline has not been
/// seen yet.
struct FramedStdout<R> {
    inner: R,
    pending: Vec<u8>,
    observed: usize,
    failed: bool,
}

impl<R> FramedStdout<R> {
    fn new(inner: R) -> Self {
        Self { inner, pending: Vec::new(), observed: 0, failed: false }
    }
    fn fail(&mut self) -> std::io::Error {
        self.pending.clear();
        self.failed = true;
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Local connector frame exceeds 4 MiB.",
        )
    }
}

impl<R: AsyncRead + Unpin> AsyncRead for FramedStdout<R> {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        if self.failed {
            return std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Local connector frame exceeds 4 MiB.",
            )));
        }
        if !self.pending.is_empty() {
            let take = self.pending.len().min(buffer.remaining());
            buffer.put_slice(&self.pending.drain(..take).collect::<Vec<_>>());
            return std::task::Poll::Ready(Ok(()));
        }
        let mut chunk = vec![0u8; buffer.remaining().clamp(1, 8192)];
        let mut read = tokio::io::ReadBuf::new(&mut chunk);
        match std::pin::Pin::new(&mut self.inner).poll_read(context, &mut read) {
            std::task::Poll::Pending => std::task::Poll::Pending,
            std::task::Poll::Ready(Err(error)) => std::task::Poll::Ready(Err(error)),
            std::task::Poll::Ready(Ok(())) => {
                let bytes = read.filled();
                if bytes.is_empty() {
                    // EOF with a partial frame still counts toward the bound.
                    if self.observed > MAX_FRAME_BYTES {
                        return std::task::Poll::Ready(Err(self.fail()));
                    }
                    return std::task::Poll::Ready(Ok(()));
                }
                // Track bytes seen since the last newline so a frame spread
                // across many small reads is still bounded before its
                // delimiter arrives.
                let mut newline_end = None;
                for (index, byte) in bytes.iter().enumerate() {
                    self.observed += 1;
                    if *byte == b'\n' {
                        newline_end = Some(index + 1);
                        break;
                    }
                    if self.observed > MAX_FRAME_BYTES {
                        return std::task::Poll::Ready(Err(self.fail()));
                    }
                }
                let (frame, rest) = match newline_end {
                    Some(end) => bytes.split_at(end),
                    None => (bytes, &[][..]),
                };
                if newline_end.is_none() {
                    self.pending.extend_from_slice(frame);
                    context.waker().wake_by_ref();
                    return std::task::Poll::Pending;
                }
                // A complete frame resets the bound for the next line; any
                // bytes after the delimiter start the next frame's count.
                self.observed = rest.len();
                let take = frame.len().min(buffer.remaining());
                buffer.put_slice(&frame[..take]);
                self.pending.extend_from_slice(&frame[take..]);
                self.pending.extend_from_slice(rest);
                std::task::Poll::Ready(Ok(()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    #[tokio::test]
    async fn oversized_frame_fails_connection_instead_of_buffering() {
        async fn read_all(reader: FramedStdout<&[u8]>) -> std::io::Result<Vec<u8>> {
            let mut reader = reader;
            let mut output = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let count = reader.read(&mut chunk).await?;
                if count == 0 {
                    break;
                }
                output.extend_from_slice(&chunk[..count]);
            }
            Ok(output)
        }
        async fn read_small(reader: FramedStdout<&[u8]>) -> std::io::Result<Vec<u8>> {
            let mut reader = reader;
            let mut output = Vec::new();
            // Small reads force an oversized frame across many chunks.
            let mut chunk = [0u8; 64];
            loop {
                let count = reader.read(&mut chunk).await?;
                if count == 0 {
                    break;
                }
                output.extend_from_slice(&chunk[..count]);
                if output.len() > MAX_FRAME_BYTES + 4096 {
                    panic!("oversized frame was not bounded");
                }
            }
            Ok(output)
        }
        assert_eq!(
            read_all(FramedStdout::new(&b"{\"a\":1}\n{\"b\":2}\n"[..])).await.unwrap(),
            b"{\"a\":1}\n{\"b\":2}\n",
        );
        let mut oversized = vec![b'x'; MAX_FRAME_BYTES + 1];
        oversized.push(b'\n');
        assert!(read_small(FramedStdout::new(&oversized[..])).await.is_err());
        // Unterminated accumulation cannot grow without limit either.
        let unterminated = vec![b'y'; MAX_FRAME_BYTES + 16];
        assert!(read_small(FramedStdout::new(&unterminated[..])).await.is_err());
    }
    #[tokio::test]
    async fn oversized_stdio_frame_fails_hub_connection() {
        let temp = tempfile::tempdir().unwrap();
        let script = temp.path().join("flood.cjs");
        std::fs::write(
            &script,
            "process.stdout.write('x'.repeat(4 * 1024 * 1024 + 8) + '\\n'); setInterval(()=>{},1000);\n",
        )
        .unwrap();
        let hub_config = LocalServer {
            id: format!("local-{}", uuid::Uuid::new_v4()),
            name: "flood".into(),
            executable: crate::execution::ExecutionConfig::default().node_path,
            arguments: vec![script.to_string_lossy().into_owned()],
            working_directory: temp.path().to_string_lossy().into_owned(),
            environment: Default::default(),
        };
        let error = match connect_with_timeout(&hub_config, Duration::from_secs(15)).await {
            Ok(_) => panic!("oversized frame unexpectedly connected"),
            Err(error) => error,
        };
        assert!(error.contains("handshake"), "unexpected error: {error}");
    }
    #[cfg(windows)]
    #[tokio::test]
    async fn process_tree_exits_after_close_drop_and_handshake_cancellation() {
        use windows::Win32::{
            Foundation::{CloseHandle, WAIT_OBJECT_0},
            System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
        };
        for scenario in ["close", "drop", "cancel", "timeout"] {
            let directory = tempfile::tempdir().unwrap();
            let script = directory.path().join("tree.cjs");
            std::fs::write(
                &script,
                r#"
const fs = require('node:fs');
const child = require('node:child_process').spawn(process.execPath,
 ['-e', 'setInterval(()=>{},1000)'], {stdio:'ignore'});
fs.writeFileSync('pids.json', JSON.stringify([process.pid,child.pid]));
setInterval(()=>{},1000);
require('node:readline').createInterface({input:process.stdin}).on('line', line => {
 if (process.argv[2] === 'silent') return;
 const r = JSON.parse(line);
 if (r.method === 'initialize') process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:r.id,
 result:{protocolVersion:r.params.protocolVersion,capabilities:{tools:{}},
 serverInfo:{name:'tree',version:'1'}}})+'\n');
});
"#,
            )
            .unwrap();
            let config = LocalServer {
                id: format!("local-{}", uuid::Uuid::new_v4()),
                name: "tree".into(),
                executable: crate::execution::ExecutionConfig::default().node_path,
                arguments: vec![
                    script.to_string_lossy().into_owned(),
                    if matches!(scenario, "cancel" | "timeout") {
                        "silent"
                    } else {
                        "respond"
                    }
                    .into(),
                ],
                working_directory: directory.path().to_string_lossy().into_owned(),
                environment: Default::default(),
            };
            let task =
                tokio::spawn(
                    async move { connect_with_timeout(&config, Duration::from_secs(3)).await },
                );
            let pids: Vec<u32> = tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if let Ok(bytes) = std::fs::read(directory.path().join("pids.json")) {
                        if let Ok(pids) = serde_json::from_slice(&bytes) {
                            break pids;
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .expect("fixture did not publish process IDs");
            // Open handles while both processes are alive so PID reuse cannot
            // produce a false success when checking termination below.
            let handles: Vec<_> = pids
                .into_iter()
                .map(|pid| unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid).unwrap() })
                .collect();
            match scenario {
                "cancel" => {
                    task.abort();
                    assert!(matches!(task.await, Err(error) if error.is_cancelled()));
                }
                "timeout" => {
                    assert!(
                        matches!(task.await.unwrap(), Err(error) if error.contains("timed out"))
                    );
                }
                "close" => {
                    let report = task.await.unwrap().unwrap().close().await;
                    assert!(report.reaped, "close did not reap the fixture tree");
                }
                _ => drop(task.await.unwrap().unwrap()),
            }
            for handle in handles {
                let status = unsafe { WaitForSingleObject(handle, 5000) };
                unsafe {
                    CloseHandle(handle).unwrap();
                }
                assert_eq!(status, WAIT_OBJECT_0, "process survived {scenario}");
            }
        }
    }
    #[tokio::test]
    async fn negotiates_with_a_real_stdio_server_and_closes() {
        let directory = tempfile::tempdir().unwrap();
        let script = directory.path().join("server.cjs");
        std::fs::write(&script, r#"
const readline = require('node:readline');
readline.createInterface({input:process.stdin}).on('line', line => {
 const request = JSON.parse(line);
 if (request.id === undefined) return;
 const result = request.method === 'initialize'
 ? {protocolVersion:request.params.protocolVersion,capabilities:{tools:{}},serverInfo:{name:'fixture',version:'1'}}
 : {tools:[]};
 process.stdout.write(JSON.stringify({jsonrpc:'2.0',id:request.id,result})+'\n');
}).on('close', () => process.exit(0));
"#).unwrap();
        let config = LocalServer {
            id: format!("local-{}", uuid::Uuid::new_v4()),
            name: "fixture".into(),
            executable: crate::execution::ExecutionConfig::default().node_path,
            arguments: vec![script.to_string_lossy().into_owned()],
            working_directory: directory.path().to_string_lossy().into_owned(),
            environment: Default::default(),
        };
        assert!(
            !config.executable.is_empty(),
            "Node is required for this process integration test"
        );
        let session = connect(&config).await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(5), session.service.list_tools(None))
            .await
            .unwrap()
            .unwrap();
        assert!(result.tools.is_empty());
        let report = session.close().await;
        assert!(report.reaped, "fixture process was not reaped");
    }
    #[tokio::test]
    async fn invalid_executable_returns_no_configuration_secrets() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("invalid.exe");
        std::fs::write(&executable, b"not an executable").unwrap();
        let config = LocalServer {
            id: format!("local-{}", uuid::Uuid::new_v4()),
            name: "fixture".into(),
            executable: executable.to_string_lossy().into_owned(),
            arguments: vec!["secret-argument".into()],
            working_directory: directory.path().to_string_lossy().into_owned(),
            environment: [("PRIVATE_TOKEN".into(), "secret-value".into())].into(),
        };
        let error = match connect(&config).await {
            Ok(_) => panic!("unexpected launch"),
            Err(error) => error,
        };
        assert!(error.contains("launch failed"));
        assert!(!error.contains("secret"));
        assert!(!error.contains("PRIVATE_TOKEN"));
    }
}
