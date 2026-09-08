use process_wrap::tokio::{CommandWrap, KillOnDrop};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionConfig {
    pub python_path: String,
    pub node_path: String,
    pub powershell_path: String,
}
fn detect(name: &str) -> String {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|directory| directory.join(name))
        .find(|path| path.is_absolute() && path.is_file())
        .and_then(|path| path.canonicalize().ok())
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default()
}
impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            python_path: detect(if cfg!(windows) {
                "python.exe"
            } else {
                "python3"
            }),
            node_path: detect(if cfg!(windows) { "node.exe" } else { "node" }),
            powershell_path: detect(if cfg!(windows) {
                "powershell.exe"
            } else {
                "pwsh"
            }),
        }
    }
}
impl ExecutionConfig {
    pub fn validate(&self) -> Result<(), String> {
        for path in [&self.python_path, &self.node_path, &self.powershell_path] {
            if !path.is_empty()
                && (path.len() > 32768
                    || !Path::new(path).is_absolute()
                    || !Path::new(path).is_file())
            {
                return Err(
                    "Interpreter paths must point to existing executables using absolute paths."
                        .into(),
                );
            }
        }
        Ok(())
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Language {
    Python,
    Javascript,
    Powershell,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeRequest {
    language: Language,
    code: String,
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
}
fn default_timeout() -> u64 {
    60
}
pub struct LocalExecution {
    config: ExecutionConfig,
    directory: PathBuf,
    _workspace: crate::workspace::Workspace,
}
impl LocalExecution {
    pub fn new(config: ExecutionConfig, directory: &str) -> Result<Self, String> {
        config.validate()?;
        Ok(Self {
            config,
            directory: PathBuf::from(directory),
            _workspace: crate::workspace::Workspace::open(directory)?,
        })
    }
    pub fn tool(self: std::sync::Arc<Self>) -> crate::connectors::AgentTool {
        crate::connectors::AgentTool::execution(self,crate::connectors::ToolView { name:"run_code".into(),description:"Run Python, JavaScript (Node.js), or PowerShell locally in the selected workspace. Requires approval of the complete code. NOT sandboxed: code has the Windows user's permissions, filesystem and network access. Output is bounded; timeout is 1–90 seconds.".into(),input_schema:json!({"type":"object","properties":{"language":{"type":"string","enum":["python","javascript","powershell"]},"code":{"type":"string"},"timeout_seconds":{"type":"integer","minimum":1,"maximum":90}},"required":["language","code"],"additionalProperties":false}) })
    }
    pub async fn run(&self, arguments: Value) -> Result<Value, String> {
        let request: CodeRequest =
            serde_json::from_value(arguments).map_err(|error| error.to_string())?;
        if request.code.trim().is_empty()
            || request.code.len() > 32768
            || !(1..=90).contains(&request.timeout_seconds)
        {
            return Err("Code must contain 1–32768 bytes and timeout must be 1–90 seconds.".into());
        }
        let (program, args): (&str, &[&str]) = match request.language {
            Language::Python => (&self.config.python_path, &["-I", "-X", "utf8", "-u", "-"]),
            Language::Javascript => (&self.config.node_path, &["--input-type=module", "-"]),
            Language::Powershell => (
                &self.config.powershell_path,
                &["-NoProfile", "-NonInteractive", "-Command", "[Console]::InputEncoding=[System.Text.UTF8Encoding]::new(); [Console]::OutputEncoding=[System.Text.UTF8Encoding]::new(); $OutputEncoding=[Console]::OutputEncoding; $ErrorActionPreference='Stop'; $code=[Console]::In.ReadToEnd(); & ([scriptblock]::Create($code))"],
            ),
        };
        if program.is_empty() {
            return Err(
                "Configure this language's interpreter on the Execution page first.".into(),
            );
        }
        let mut command = tokio::process::Command::new(program);
        command
            .args(args)
            .current_dir(&self.directory)
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
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONUTF8", "1")
            .env("NO_COLOR", "1");
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
        {
            command.wrap(process_wrap::tokio::ProcessGroup::leader());
        }
        let mut child = command
            .spawn()
            .map_err(|error| format!("Interpreter launch failed: {error}"))?;
        let mut input = child.stdin().take().ok_or("Missing process input pipe.")?;
        let output = child
            .stdout()
            .take()
            .ok_or("Missing process output pipe.")?;
        let errors = child.stderr().take().ok_or("Missing process error pipe.")?;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let started = std::time::Instant::now();
        let result = tokio::time::timeout(Duration::from_secs(request.timeout_seconds), async {
            let write = async {
                match input.write_all(request.code.as_bytes()).await {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => {}
                    Err(error) => return Err(error.to_string()),
                };
                drop(input);
                Ok(())
            };
            let wait = async { child.wait().await.map_err(|error| error.to_string()) };
            let (_, _, _, status) = tokio::try_join!(
                write,
                read_output(output, &mut stdout),
                read_output(errors, &mut stderr),
                wait
            )?;
            Ok::<_, String>(status)
        })
        .await;
        let (exit_code, error) = match result {
            Ok(Ok(status)) => (status.code(), None),
            result => {
                let error = match result {
                    Err(_) => "Execution timed out.".into(),
                    Ok(Err(error)) => error,
                    _ => unreachable!(),
                };
                let _ = child.start_kill();
                let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
                (None, Some(error))
            }
        };
        drop(child);
        Ok(
            json!({"provider":"local","exitCode":exit_code,"stdout":String::from_utf8_lossy(&stdout),"stderr":String::from_utf8_lossy(&stderr),"error":error,"isError":error.is_some() || exit_code!=Some(0),"durationMs":started.elapsed().as_millis()}),
        )
    }
}
async fn read_output(
    mut stream: impl AsyncRead + Unpin,
    output: &mut Vec<u8>,
) -> Result<(), String> {
    let mut buffer = [0; 4096];
    loop {
        let count = stream
            .read(&mut buffer)
            .await
            .map_err(|error| error.to_string())?;
        if count == 0 {
            return Ok(());
        }
        let available = 65_536 - output.len();
        output.extend_from_slice(&buffer[..count.min(available)]);
        if count > available {
            return Err("Execution output exceeded 64 KiB per stream.".into());
        }
    }
}
#[tauri::command]
pub fn get_execution_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<ExecutionConfig, String> {
    state.database()?.execution_config()
}
#[tauri::command]
pub async fn save_execution_config(
    state: tauri::State<'_, crate::AppState>,
    config: ExecutionConfig,
) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the model operation to finish.")?;
    config.validate()?;
    state.database()?.save_execution_config(&config)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[tokio::test]
    async fn cancelling_execution_kills_spawned_descendants() {
        let temp = tempfile::tempdir().unwrap();
        let execution = std::sync::Arc::new(
            LocalExecution::new(ExecutionConfig::default(), temp.path().to_str().unwrap()).unwrap(),
        );
        let task = tokio::spawn(async move {
            execution.run(json!({"language":"python","code":"import subprocess, sys, time\nsubprocess.Popen([sys.executable, '-c', \"import time; open('started.txt','w').write('started'); time.sleep(3); open('orphan.txt','w').write('survived')\"])\ntime.sleep(20)"})).await
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while !temp.path().join("started.txt").exists() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("Descendant did not start");
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        tokio::time::sleep(Duration::from_millis(3300)).await;
        assert!(
            !temp.path().join("orphan.txt").exists(),
            "Descendant survived cancellation"
        );
    }
    #[tokio::test]
    async fn runs_python_and_javascript_and_reports_nonzero_exit() {
        let temp = tempfile::tempdir().unwrap();
        let execution =
            LocalExecution::new(ExecutionConfig::default(), temp.path().to_str().unwrap()).unwrap();
        for (language, code) in [
            ("python", "print('héllo')"),
            ("javascript", "console.log('héllo')"),
            ("powershell", "Write-Output 'héllo'"),
        ] {
            let result = execution
                .run(json!({"language":language,"code":code}))
                .await
                .unwrap();
            assert_eq!(result["exitCode"], 0, "{result}");
            assert!(
                result["stdout"].as_str().unwrap().contains("héllo"),
                "{language}: {result}"
            );
        }
        let result=execution.run(json!({"language":"python","code":"import sys; print('failed', file=sys.stderr); sys.exit(7)"})).await.unwrap();
        assert_eq!(result["exitCode"], 7);
        assert_eq!(result["isError"], true);
        assert!(result["stderr"].as_str().unwrap().contains("failed"));
    }
    #[tokio::test]
    async fn output_and_time_are_bounded() {
        let temp = tempfile::tempdir().unwrap();
        let execution =
            LocalExecution::new(ExecutionConfig::default(), temp.path().to_str().unwrap()).unwrap();
        let result = execution
            .run(json!({"language":"python","code":"print('x' * 200000)"}))
            .await
            .unwrap();
        assert_eq!(result["stdout"].as_str().unwrap().len(), 65536);
        assert_eq!(result["isError"], true);
        let result=execution.run(json!({"language":"python","code":"import time; time.sleep(10)","timeout_seconds":1})).await.unwrap();
        assert_eq!(result["error"], "Execution timed out.");
        assert!(execution
            .run(json!({"language":"unknown","code":"test"}))
            .await
            .is_err());
    }
}
