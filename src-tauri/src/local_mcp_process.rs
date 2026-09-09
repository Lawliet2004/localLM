//! Local MCP process ownership. Explicit connection is the only launch operation.
use crate::local_mcp_config::LocalServer;
use process_wrap::tokio::{ChildWrapper, CommandWrap, KillOnDrop};
use rmcp::{service::RunningService, RoleClient, ServiceExt};
use std::{process::Stdio, time::Duration};

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
    let service = tokio::time::timeout(handshake_timeout, ().serve((output, input)))
        .await
        .map_err(|_| "Local connector handshake timed out.")?
        .map_err(|_| {
            "Local connector handshake failed. Check its arguments and MCP stdio support."
        })?;
    Ok(LocalSession { service, child })
}

#[cfg(test)]
mod tests {
    use super::*;
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
