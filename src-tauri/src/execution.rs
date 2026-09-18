use process_wrap::tokio::{CommandWrap, KillOnDrop};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionConfig {
    pub python_path: String,
    pub node_path: String,
    pub powershell_path: String,
}
fn detect(name: &str) -> String {
    let mut found = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|directory| directory.join(name))
        .find(|path| path.is_absolute() && path.is_file());

    #[cfg(windows)]
    if found.is_none() && (name == "powershell.exe" || name == "pwsh.exe") {
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let default_ps = PathBuf::from(system_root)
            .join("System32")
            .join("WindowsPowerShell")
            .join("v1.0")
            .join("powershell.exe");
        if default_ps.is_file() {
            found = Some(default_ps);
        }
    }

    if let Some(path) = found {
        let path_str = path.to_string_lossy();
        if let Some(stripped) = path_str.strip_prefix(r"\\?\") {
            stripped.to_string()
        } else {
            path_str.into_owned()
        }
    } else {
        String::new()
    }
}
impl Default for ExecutionConfig {
    fn default() -> Self {
        let python = if cfg!(windows) {
            let p = detect("python.exe");
            if p.is_empty() {
                let py = detect("py.exe");
                if py.is_empty() { detect("python3.exe") } else { py }
            } else {
                p
            }
        } else {
            let p = detect("python3");
            if p.is_empty() { detect("python") } else { p }
        };

        Self {
            python_path: python,
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Language {
    #[default]
    Python,
    Javascript,
    Powershell,
}
fn parse_language(raw: &str) -> Option<Language> {
    let token = raw
        .trim()
        .split(|character: char| character.is_whitespace() || ['.', '-', '_'].contains(&character))
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    match token.as_str() {
        "python" | "python3" | "py" => Some(Language::Python),
        "javascript" | "js" | "node" | "nodejs" => Some(Language::Javascript),
        "powershell" | "pwsh" | "ps" | "ps1" => Some(Language::Powershell),
        _ => None,
    }
}
fn deserialize_language<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Language, D::Error> {
    match Value::deserialize(deserializer)? {
        Value::Null => Ok(Language::Python),
        Value::String(raw) => parse_language(&raw)
            .ok_or_else(|| serde::de::Error::custom("Use python, javascript, or powershell.")),
        _ => Err(serde::de::Error::custom("language must be a string.")),
    }
}
/// Strip a wrapping markdown fence so ` ```python\n2+2\n``` ` runs as `2+2`.
fn normalize_code(code: &str) -> String {
    let mut text = code.trim();
    if let Some(rest) = text.strip_prefix('\u{feff}') {
        text = rest.trim();
    }
    if !text.starts_with("```") {
        return text.to_string();
    }
    let mut rest = &text[3..];
    if let Some(stripped) = rest.strip_prefix('\r') {
        rest = stripped;
    }
    if let Some(newline) = rest.find('\n') {
        let tag = rest[..newline].trim().to_ascii_lowercase();
        let tag_head = tag
            .split_whitespace()
            .next()
            .unwrap_or("")
            .split(['.', '-', '_'])
            .next()
            .unwrap_or("");
        let known = [
            "", "python", "py", "python3", "javascript", "js", "typescript", "ts", "powershell",
            "pwsh", "ps1", "code",
        ];
        if known.contains(&tag.as_str()) || known.contains(&tag_head) {
            rest = &rest[newline + 1..];
        }
    } else {
        return text.to_string();
    }
    let rest = rest.trim_end();
    rest.strip_suffix("```").unwrap_or(rest).trim_end().to_string()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeRequest {
    #[serde(default, alias = "lang", deserialize_with = "deserialize_language")]
    language: Language,
    code: String,
    #[serde(default = "default_timeout")]
    timeout_seconds: u64,
    /// Optional requested output schema for the structured result channel.
    #[serde(default)]
    result_schema: Option<Value>,
}
fn default_timeout() -> u64 {
    60
}
pub type ChunkCallback = std::sync::Arc<dyn Fn(&str, &str) + Send + Sync>;

/// Name of the structured result channel. `locallm_result(...)` still wins.
/// If the process exits 0 with no marker, a last Python expression/assignment
/// or non-empty stdout is used as the result so small models can `print(2+2)`.
pub const RESULT_MARKER: &str = "__LOCALLM_RESULT_JSON__";
/// Maximum serialized structured-result bytes (64 KiB). Larger payloads are
/// spilled to artifacts instead of being silently truncated into "success".
pub const MAX_RESULT_BYTES: usize = 65_536;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandRequest {
    command: String,
    cwd: Option<String>,
    #[serde(default = "default_command_timeout")]
    timeout_seconds: u64,
}
fn default_command_timeout() -> u64 {
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
        crate::connectors::AgentTool::execution(self,crate::connectors::ToolView { name:"run_code".into(),description:"Run code in the selected workspace. Python is the default (also javascript, powershell). NOT sandboxed: same files and network as this Windows user. Timeout 1–90 seconds. For exact math, dates, parsing, or data work, use this instead of calculating by hand. Write plain source — no markdown fences. The last expression, last assignment, or printed output is the answer. locallm_result(value) is optional JSON. Example: {\"language\":\"python\",\"code\":\"round(19.99 * 1.08, 2)\"}".into(),input_schema:json!({"type":"object","properties":{"language":{"type":"string","enum":["python","javascript","powershell"],"description":"Defaults to python. py, js, and pwsh aliases are accepted."},"code":{"type":"string","description":"Source to run. Last expression, last assignment, or print is the answer. Do not wrap in markdown fences."},"timeout_seconds":{"type":"integer","minimum":1,"maximum":90}},"required":["code"],"additionalProperties":false}) })
    }
    pub fn command_tool(self: std::sync::Arc<Self>) -> crate::connectors::AgentTool {
        crate::connectors::AgentTool::execution(
            self,
            crate::connectors::ToolView {
                name: "execute_command".into(),
                description: "Execute a shell command (CLI, build tool, test runner, git, etc.) locally in the workspace directory. Requires approval. Output is bounded; timeout is 1–300 seconds. Do not use this for calculations — call local_run_code with Python instead.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "command": {
                            "type": "string",
                            "description": "The shell command to execute"
                        },
                        "cwd": {
                            "type": "string",
                            "description": "Optional sub-directory relative to workspace root"
                        },
                        "timeout_seconds": {
                            "type": "integer",
                            "minimum": 1,
                            "maximum": 300
                        }
                    },
                    "required": ["command"],
                    "additionalProperties": false
                }),
            },
        )
    }
    pub async fn run(&self, arguments: Value) -> Result<Value, String> {
        self.run_with_stream(arguments, None).await
    }
    pub async fn run_with_stream(
        &self,
        arguments: Value,
        on_chunk: Option<ChunkCallback>,
    ) -> Result<Value, String> {
        let mut request: CodeRequest =
            serde_json::from_value(arguments).map_err(|error| error.to_string())?;
        request.code = normalize_code(&request.code);
        if request.code.trim().is_empty()
            || request.code.len() > 32768
            || !(1..=90).contains(&request.timeout_seconds)
        {
            return Err("Code must contain 1–32768 bytes and timeout must be 1–90 seconds.".into());
        }
        let (program, args, wrapped_code): (&str, Vec<String>, String) = match request.language {
            Language::Python => (
                &self.config.python_path,
                vec![
                    "-I".into(),
                    "-X".into(),
                    "utf8".into(),
                    "-u".into(),
                    "-c".into(),
                    python_repl_driver().into(),
                ],
                String::new(),
            ),
            Language::Javascript => (
                &self.config.node_path,
                vec!["--input-type=module".into(), "-".into()],
                format!("{}\n", result_prelude_javascript()),
            ),
            Language::Powershell => (
                &self.config.powershell_path,
                vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), "[Console]::InputEncoding=[System.Text.UTF8Encoding]::new(); [Console]::OutputEncoding=[System.Text.UTF8Encoding]::new(); $OutputEncoding=[Console]::OutputEncoding; $ErrorActionPreference='Stop'; function locallm_result($v){ $j = $v | ConvertTo-Json -Depth 20 -Compress; Write-Output '__LOCALLM_RESULT_JSON__' + $j }; $code=[Console]::In.ReadToEnd(); & ([scriptblock]::Create($code))".into()],
                String::new(),
            ),
        };
        if program.is_empty() {
            return Err(
                "Configure this language's interpreter on the Execution page first.".into(),
            );
        }
        let mut command = tokio::process::Command::new(program);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        command
            .args(&arg_refs)
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
            "TERM",
            "COLORTERM",
        ] {
            if let Some(value) = std::env::var_os(name) {
                command.env(name, value);
            }
        }
        command
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONUTF8", "1");
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
        let out_cb = on_chunk.clone();
        let err_cb = on_chunk;
        let result = tokio::time::timeout(Duration::from_secs(request.timeout_seconds), async {
            let write = async {
                // Structured-result helpers ride in front of the approved code
                // so plain prints stay logs and only marker lines count. For
                // PowerShell the helper is defined in the -Command prelude.
                let payload = if matches!(request.language, Language::Powershell) {
                    request.code.clone()
                } else {
                    format!("{wrapped_code}{}", request.code)
                };
                match input.write_all(payload.as_bytes()).await {
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
                read_output(output, &mut stdout, "stdout", out_cb),
                read_output(errors, &mut stderr, "stderr", err_cb),
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
        let stdout_text = String::from_utf8_lossy(&stdout).into_owned();
        let stderr_text = String::from_utf8_lossy(&stderr).into_owned();
        let duration_ms = started.elapsed().as_millis();
        let structured = parse_structured_results(&stdout_text);
        let legacy = json!({"provider":"local","exitCode":exit_code,"stdout":stdout_text,"stderr":stderr_text,"error":error,"isError":error.is_some() || exit_code!=Some(0),"durationMs":duration_ms});
        Ok(finish_execution_result(legacy, structured, request.result_schema.as_ref(), duration_ms))
    }

    pub async fn run_command(&self, arguments: Value) -> Result<Value, String> {
        self.run_command_with_stream(arguments, None).await
    }

    pub async fn run_command_with_stream(
        &self,
        arguments: Value,
        on_chunk: Option<ChunkCallback>,
    ) -> Result<Value, String> {
        let request: CommandRequest =
            serde_json::from_value(arguments).map_err(|error| error.to_string())?;
        if request.command.trim().is_empty()
            || request.command.len() > 32768
            || !(1..=300).contains(&request.timeout_seconds)
        {
            return Err("Command must contain 1–32768 bytes and timeout must be 1–300 seconds.".into());
        }

        let target_dir = if let Some(sub) = &request.cwd {
            let p = Path::new(sub);
            if p.is_absolute() {
                if !p.starts_with(&self.directory) {
                    return Err("cwd must be within the workspace directory.".into());
                }
                p.to_path_buf()
            } else {
                let joined = self.directory.join(p);
                if !joined.starts_with(&self.directory) {
                    return Err("cwd must be within the workspace directory.".into());
                }
                joined
            }
        } else {
            self.directory.clone()
        };

        if !target_dir.is_dir() {
            return Err("Specified working directory does not exist.".into());
        }

        let mut cmd = if cfg!(windows) {
            let shell = if !self.config.powershell_path.is_empty() && Path::new(&self.config.powershell_path).is_file() {
                &self.config.powershell_path
            } else {
                "powershell.exe"
            };
            let mut c = tokio::process::Command::new(shell);
            c.args(["-NoProfile", "-NonInteractive", "-Command", &request.command]);
            c
        } else {
            let mut c = tokio::process::Command::new("sh");
            c.args(["-c", &request.command]);
            c
        };

        cmd.current_dir(&target_dir)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        for name in [
            "SystemRoot", "WINDIR", "TEMP", "TMP", "PATH", "PATHEXT",
            "COMSPEC", "USERPROFILE", "HOME", "APPDATA", "LOCALAPPDATA",
            "TERM", "COLORTERM",
        ] {
            if let Some(value) = std::env::var_os(name) {
                cmd.env(name, value);
            }
        }
        cmd.env("PYTHONIOENCODING", "utf-8")
           .env("PYTHONUTF8", "1");

        let mut command = CommandWrap::from(cmd);
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
            .map_err(|error| format!("Command launch failed: {error}"))?;

        let output = child
            .stdout()
            .take()
            .ok_or("Missing process output pipe.")?;
        let errors = child.stderr().take().ok_or("Missing process error pipe.")?;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let started = std::time::Instant::now();

        let out_cb = on_chunk.clone();
        let err_cb = on_chunk;

        let result = tokio::time::timeout(Duration::from_secs(request.timeout_seconds), async {
            let wait = async { child.wait().await.map_err(|error| error.to_string()) };
            let (_, _, status) = tokio::try_join!(
                read_output(output, &mut stdout, "stdout", out_cb),
                read_output(errors, &mut stderr, "stderr", err_cb),
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
        Ok(json!({
            "provider": "local",
            "command": request.command,
            "cwd": target_dir.to_string_lossy(),
            "exitCode": exit_code,
            "stdout": String::from_utf8_lossy(&stdout),
            "stderr": String::from_utf8_lossy(&stderr),
            "error": error,
            "isError": error.is_some() || exit_code != Some(0),
            "durationMs": started.elapsed().as_millis()
        }))
    }
}
async fn read_output(
    mut stream: impl AsyncRead + Unpin,
    output: &mut Vec<u8>,
    stream_name: &'static str,
    on_chunk: Option<ChunkCallback>,
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
        let to_take = count.min(available);
        if to_take > 0 {
            output.extend_from_slice(&buffer[..to_take]);
            if let Some(ref cb) = on_chunk {
                let chunk_text = String::from_utf8_lossy(&buffer[..to_take]);
                cb(stream_name, &chunk_text);
            }
        }
        if count > available {
            return Err("Execution output exceeded 64 KiB per stream.".into());
        }
    }
}

/// Python driver: reads user source from stdin and runs it as a notebook cell.
/// Last expression / last simple assignment become locallm_result; print stays
/// on stdout for the host fallback. math/json/datetime/decimal/fractions/
/// statistics/re are preloaded. Never uses eval on unparsed text.
fn python_repl_driver() -> &'static str {
    r#"import ast, collections, datetime, decimal, fractions, itertools, json, math, random, re, statistics, sys
_emitted = {"n": 0}
def locallm_result(value):
    _emitted["n"] += 1
    print("__LOCALLM_RESULT_JSON__" + json.dumps(value, ensure_ascii=False, allow_nan=False, separators=(",", ":"), default=str), flush=True)
def _strip(src):
    s = src.replace("\r\n", "\n").strip()
    if s.startswith("```"):
        s = s[3:]
        line, _, rest = s.partition("\n")
        tag = line.strip().split()[0].split(".")[0].lower() if line.strip() else ""
        if tag in ("", "python", "py", "python3", "javascript", "js", "typescript", "ts", "powershell", "pwsh", "ps1", "code"):
            s = rest
        if s.endswith("```"):
            s = s[:-3].rstrip()
    return s
src = _strip(sys.stdin.read())
g = {
    "__name__": "__main__",
    "__builtins__": __builtins__,
    "locallm_result": locallm_result,
    "math": math, "json": json, "datetime": datetime, "decimal": decimal,
    "fractions": fractions, "statistics": statistics, "re": re,
    "collections": collections, "itertools": itertools, "random": random,
}
if not src:
    raise SystemExit(0)
tree = ast.parse(src, filename="<run_code>")
if not tree.body:
    raise SystemExit(0)
*head, tail = tree.body
if head:
    exec(compile(ast.Module(body=head, type_ignores=[]), "<run_code>", "exec"), g, g)
if isinstance(tail, ast.Expr):
    value = eval(compile(ast.Expression(tail.value), "<run_code>", "eval"), g, g)
    if _emitted["n"] == 0 and value is not None:
        locallm_result(value)
else:
    exec(compile(ast.Module(body=[tail], type_ignores=[]), "<run_code>", "exec"), g, g)
    if _emitted["n"] == 0:
        name = None
        if isinstance(tail, ast.Assign) and len(tail.targets) == 1 and isinstance(tail.targets[0], ast.Name):
            name = tail.targets[0].id
        elif isinstance(tail, ast.AnnAssign) and isinstance(tail.target, ast.Name) and tail.value is not None:
            name = tail.target.id
        if name is not None and name in g:
            locallm_result(g[name])
"#
}

/// JavaScript helper: strict JSON serialization, one value per marker line.
fn result_prelude_javascript() -> &'static str {
    r#"function locallm_result(value) { console.log("__LOCALLM_RESULT_JSON__" + JSON.stringify(value)); }
"#
}

#[derive(Debug, PartialEq, Eq)]
enum StructuredOutcome {
    /// Exactly one well-formed marker line.
    Value(Value),
    /// No marker line at all.
    Missing,
    /// A marker line that is not valid JSON, or more than one marker line.
    Malformed(String),
    /// A marker payload larger than [`MAX_RESULT_BYTES`].
    TooLarge(usize),
}

/// Scan stdout for structured-result marker lines. Plain prints are ignored;
/// only lines starting with [`RESULT_MARKER`] count. Raw `repr()` output,
/// `None`, `True`, or single-quoted Python values are never parsed — the
/// helper emits real JSON or the result is malformed. Console recoding can
/// wrap a long marker line (observed on Windows as `MARKER`, `+`, payload
/// lines); the scan therefore rejoins a bare marker line with following
/// continuation lines before parsing.
fn parse_structured_results(stdout: &str) -> StructuredOutcome {
    // Normalize the observed Windows wrap shape `MARKER\r\n+\r\npayload`
    // (and stray `+` continuations) before scanning for marker lines.
    let mut normalized = String::with_capacity(stdout.len());
    let mut lines = stdout.lines().peekable();
    while let Some(line) = lines.next() {
        if line.trim_end() == RESULT_MARKER {
            // Bare marker: the payload arrives on following lines. Rejoin
            // `+` continuation lines and the first non-empty payload line.
            let mut payload = String::new();
            while let Some(next) = lines.peek() {
                let trimmed = next.trim();
                if trimmed.is_empty() {
                    lines.next();
                    continue;
                }
                if trimmed == "+" {
                    lines.next();
                    continue;
                }
                payload.push_str(trimmed);
                lines.next();
                break;
            }
            // Any further wrapped fragments rejoin while parsing fails.
            loop {
                if payload.is_empty() {
                    break;
                }
                match serde_json::from_str::<Value>(payload.trim()) {
                    Ok(_) => break,
                    Err(_) => match lines.peek() {
                        Some(next) if !next.trim().is_empty() => {
                            payload.push_str(next.trim());
                            lines.next();
                        }
                        _ => break,
                    },
                }
            }
            normalized.push_str(RESULT_MARKER);
            normalized.push_str(&payload);
            normalized.push('\n');
        } else {
            normalized.push_str(line);
            normalized.push('\n');
        }
    }
    let mut values = Vec::new();
    for line in normalized.lines() {
        let Some(payload) = line.strip_prefix(RESULT_MARKER) else { continue };
        if payload.len() > MAX_RESULT_BYTES {
            return StructuredOutcome::TooLarge(payload.len());
        }
        match serde_json::from_str::<Value>(payload.trim()) {
            Ok(value) => values.push(value),
            Err(error) => return StructuredOutcome::Malformed(format!("Structured result is not valid JSON: {error}")),
        }
    }
    match values.len() {
        0 => StructuredOutcome::Missing,
        1 => StructuredOutcome::Value(values.into_iter().next().unwrap()),
        count => StructuredOutcome::Malformed(format!("Expected exactly one structured result but found {count}.")),
    }
}

/// Last non-empty stdout, excluding structured-result marker lines.
fn stdout_as_result(stdout: &str) -> Option<Value> {
    let text = stdout
        .lines()
        .filter(|line| !line.starts_with(RESULT_MARKER))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    if text.is_empty() {
        None
    } else {
        Some(Value::String(text))
    }
}

/// Combine the legacy exit/stdout record with the structured channel into one
/// versioned envelope. The envelope stays backward compatible: legacy flat
/// fields are preserved, and `data.result` carries the structured value while
/// `data.logs` carries the plain stdout/stderr text.
fn finish_execution_result(
    legacy: Value,
    structured: StructuredOutcome,
    schema: Option<&Value>,
    duration_ms: u128,
) -> Value {
    let exit_ok = legacy.get("exitCode") == Some(&json!(0));
    let timed_out = legacy
        .get("error")
        .and_then(Value::as_str)
        .is_some_and(|message| message.contains("timed out"));
    let stream_truncated = legacy
        .get("error")
        .and_then(Value::as_str)
        .is_some_and(|message| message.contains("exceeded 64 KiB"));
    let stdout_text = legacy.get("stdout").and_then(Value::as_str).unwrap_or("").to_string();
    let stderr_text = legacy.get("stderr").and_then(Value::as_str).unwrap_or("").to_string();

    // Missing-result detection, serialization errors, schema mismatches, and
    // truncated streams are explicit failures — never silent success. A
    // successful run with no marker still counts printed stdout (or a Python
    // last-expression captured by the driver) so small models can print(2+2).
    let exit_code_value = legacy.get("exitCode").cloned().unwrap_or(Value::Null);
    let (status, data_result, result_status, error) = match structured {
        StructuredOutcome::Missing if stream_truncated => (
            crate::tool_envelope::Status::Failure,
            Value::Null,
            "truncated",
            Some(json!({"code": "output-truncated", "message": "An output stream exceeded 64 KiB; the structured result cannot be trusted complete.", "retryable": false})),
        ),
        StructuredOutcome::Missing if timed_out => (
            crate::tool_envelope::Status::Failure,
            Value::Null,
            "unknown",
            Some(json!({"code": "timeout", "message": "Execution timed out; whether the result was produced is unknown. Do not rerun side-effecting code automatically.", "retryable": false})),
        ),
        StructuredOutcome::Missing if !exit_ok => (
            crate::tool_envelope::Status::Failure,
            Value::Null,
            "exit-nonzero",
            Some(json!({"code": "nonzero-exit", "message": format!("Process exited with {exit_code_value}; no structured result was emitted."), "retryable": false})),
        ),
        StructuredOutcome::Missing => match stdout_as_result(&stdout_text) {
            Some(value) => (crate::tool_envelope::Status::Success, value, "ok", None),
            None => (
                crate::tool_envelope::Status::Failure,
                Value::Null,
                "missing",
                Some(json!({"code": "missing-result", "message": "Execution produced no result. Print the answer or end with an expression, e.g. print(2+2) or locallm_result(value).", "retryable": true})),
            ),
        },
        StructuredOutcome::Malformed(message) => (
            crate::tool_envelope::Status::Failure,
            Value::Null,
            "malformed",
            Some(json!({"code": "malformed-result", "message": message, "retryable": false})),
        ),
        StructuredOutcome::TooLarge(bytes) => (
            crate::tool_envelope::Status::Failure,
            Value::Null,
            "too-large",
            Some(json!({"code": "result-too-large", "message": format!("Structured result is {bytes} bytes, above the 64 KiB channel limit. Write large output to a workspace file and return its path instead."), "retryable": false})),
        ),
        StructuredOutcome::Value(value) => {
            if stream_truncated {
                (crate::tool_envelope::Status::Failure, Value::Null, "truncated",
                    Some(json!({"code": "output-truncated", "message": "An output stream exceeded 64 KiB; the structured result cannot be trusted complete.", "retryable": false})))
            } else if timed_out {
                (crate::tool_envelope::Status::Failure, Value::Null, "unknown",
                    Some(json!({"code": "timeout", "message": "Execution timed out; whether the result was produced is unknown. Do not rerun side-effecting code automatically.", "retryable": false})))
            } else if !exit_ok {
                (crate::tool_envelope::Status::Failure, Value::Null, "exit-nonzero",
                    Some(json!({"code": "nonzero-exit", "message": format!("Process exited with {:?}; structured result is not accepted.", legacy.get("exitCode").unwrap_or(&Value::Null)), "retryable": false})))
            } else if let Some(schema) = schema {
                match crate::tool_envelope::validate_against_schema(&value, schema) {
                    Ok(()) => (crate::tool_envelope::Status::Success, value, "ok", None),
                    Err(message) => (crate::tool_envelope::Status::Failure, Value::Null, "schema-mismatch",
                        Some(json!({"code": "schema-mismatch", "message": message, "retryable": false}))),
                }
            } else {
                (crate::tool_envelope::Status::Success, value, "ok", None)
            }
        }
    };

    let truncated = stream_truncated
        || matches!(result_status, "too-large" | "truncated");
    let mut envelope = crate::tool_envelope::make(
        status,
        json!({
            "exitCode": legacy.get("exitCode").cloned().unwrap_or(Value::Null),
            "logs": {"stdout": stdout_text, "stderr": stderr_text},
            "result": data_result,
            "resultStatus": result_status,
        }),
        Vec::new(),
        error,
        json!({"durationMs": duration_ms, "truncated": truncated}),
    );
    if let (Some(map), Some(legacy_map)) = (envelope.as_object_mut(), legacy.as_object()) {
        for (key, val) in legacy_map {
            map.entry(key.clone()).or_insert_with(|| val.clone());
        }
    }
    envelope
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

#[tauri::command]
pub fn detect_interpreters() -> Result<ExecutionConfig, String> {
    Ok(ExecutionConfig::default())
}

#[tauri::command]
pub async fn test_interpreter(path: String) -> Result<String, String> {
    let clean_path = if let Some(stripped) = path.strip_prefix(r"\\?\") {
        stripped.to_string()
    } else {
        path
    };
    let p = Path::new(&clean_path);
    if clean_path.trim().is_empty() || !p.is_absolute() || !p.is_file() {
        return Err("Interpreter path must point to an existing executable.".into());
    }
    let lower = clean_path.to_lowercase();
    let mut cmd = tokio::process::Command::new(&clean_path);
    if lower.contains("powershell") || lower.contains("pwsh") {
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", "$PSVersionTable.PSVersion.ToString()"]);
    } else {
        cmd.arg("--version");
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(windows)]
    {
        #[allow(unused_imports)]
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let result = tokio::time::timeout(Duration::from_secs(5), cmd.output()).await;
    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if !stdout.is_empty() {
                Ok(stdout)
            } else if !stderr.is_empty() {
                Ok(stderr)
            } else {
                Ok(format!("Executable verified (exit code {})", output.status.code().unwrap_or(0)))
            }
        }
        Ok(Err(e)) => Err(format!("Failed to run executable: {e}")),
        Err(_) => Err("Interpreter check timed out after 5 seconds.".into()),
    }
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
            ("python", "print('héllo'); locallm_result({'greeting': 'héllo'})"),
            ("javascript", "console.log('héllo'); locallm_result({greeting: 'héllo'});"),
            ("powershell", "Write-Output 'héllo'; locallm_result(@{greeting='héllo'})"),
        ] {
            let result = execution
                .run(json!({"language":language,"code":code}))
                .await
                .unwrap();
            assert_eq!(result["exitCode"], 0, "{result}");
            assert_eq!(result["status"], "success", "{result}");
            assert!(
                result["stdout"].as_str().unwrap().contains("héllo"),
                "{language}: {result}"
            );
            assert_eq!(result["data"]["result"]["greeting"], "héllo", "{language}: {result}");
            assert!(crate::tool_envelope::validate_envelope(&result).is_ok(), "{language}: {result}");
        }
        let result=execution.run(json!({"language":"python","code":"import sys; print('failed', file=sys.stderr); sys.exit(7)"})).await.unwrap();
        assert_eq!(result["exitCode"], 7);
        assert_eq!(result["isError"], true);
        assert_eq!(result["status"], "failure");
        assert_eq!(result["error"]["code"], "nonzero-exit");
        assert!(result["stderr"].as_str().unwrap().contains("failed"));
        assert!(crate::tool_envelope::validate_envelope(&result).is_ok(), "{result}");
    }
    #[tokio::test]
    async fn structured_results_validate_nested_none_unicode_and_schema() {
        let temp = tempfile::tempdir().unwrap();
        let execution =
            LocalExecution::new(ExecutionConfig::default(), temp.path().to_str().unwrap()).unwrap();
        // Nested values, booleans, Unicode, and None-as-null survive the channel.
        let result = execution
            .run(json!({"language":"python","code":"print('log line that must not corrupt the result'); locallm_result({'nested': {'flag': True, 'missing': None, 'items': [1, 'héllo世界🌍', None]}})"}))
            .await
            .unwrap();
        assert_eq!(result["status"], "success", "{result}");
        assert_eq!(result["data"]["result"]["nested"]["flag"], true);
        assert_eq!(result["data"]["result"]["nested"]["missing"], Value::Null);
        assert_eq!(result["data"]["result"]["nested"]["items"][1], "héllo世界🌍");
        assert_eq!(result["data"]["result"]["nested"]["items"][2], Value::Null);
        assert!(result["data"]["logs"]["stdout"].as_str().unwrap().contains("log line"));
        // A requested schema is enforced; wrong shapes fail explicitly.
        let schema = json!({"type":"object","required":["answer"],"properties":{"answer":{"type":"string"}}});
        let ok = execution
            .run(json!({"language":"python","code":"locallm_result({'answer': '42'})","result_schema":schema}))
            .await
            .unwrap();
        assert_eq!(ok["status"], "success", "{ok}");
        let bad = execution
            .run(json!({"language":"python","code":"locallm_result({'wrong': 1})","result_schema":schema}))
            .await
            .unwrap();
        assert_eq!(bad["status"], "failure", "{bad}");
        assert_eq!(bad["error"]["code"], "schema-mismatch");
        // Invalid JSON, Python reprs, and missing results fail explicitly.
        let raw = execution
            .run(json!({"language":"python","code":"print('__LOCALLM_RESULT_JSON__{not json')"}))
            .await
            .unwrap();
        assert_eq!(raw["error"]["code"], "malformed-result", "{raw}");
        let repr = execution
            .run(json!({"language":"python","code":"print('__LOCALLM_RESULT_JSON__' + repr({'a': None}))"}))
            .await
            .unwrap();
        assert_eq!(repr["error"]["code"], "malformed-result", "{repr}");
        let printed = execution
            .run(json!({"language":"python","code":"print('just a log, no result')"}))
            .await
            .unwrap();
        assert_eq!(printed["status"], "success", "{printed}");
        assert_eq!(printed["data"]["result"], "just a log, no result", "{printed}");
        let missing = execution
            .run(json!({"language":"python","code":"pass"}))
            .await
            .unwrap();
        assert_eq!(missing["error"]["code"], "missing-result", "{missing}");
        assert_eq!(missing["error"]["retryable"], true, "{missing}");
        let nan = execution
            .run(json!({"language":"python","code":"locallm_result(float('nan'))"}))
            .await
            .unwrap();
        assert_eq!(nan["status"], "failure", "{nan}");
        assert_eq!(nan["exitCode"], 1);
        assert!(crate::tool_envelope::validate_envelope(&nan).is_ok(), "{nan}");
    }
    #[test]
    fn structured_result_parsing_joins_wrapped_lines_and_rejects_reprs() {
        assert!(matches!(parse_structured_results("log\n"), StructuredOutcome::Missing));
        match parse_structured_results("__LOCALLM_RESULT_JSON__{\"a\":1}\n") {
            StructuredOutcome::Value(value) => assert_eq!(value["a"], 1),
            other => panic!("unexpected {other:?}"),
        }
        // Windows console wrapping splits the marker payload across lines.
        match parse_structured_results("__LOCALLM_RESULT_JSON__\n+\n{\"greeting\":\"hi\"}\n") {
            StructuredOutcome::Value(value) => assert_eq!(value["greeting"], "hi"),
            other => panic!("unexpected {other:?}"),
        }
        // Payload on the line after a bare marker also rejoins.
        match parse_structured_results("__LOCALLM_RESULT_JSON__\n{\"b\":2}\n") {
            StructuredOutcome::Value(value) => assert_eq!(value["b"], 2),
            other => panic!("unexpected {other:?}"),
        }
        assert!(matches!(
            parse_structured_results("__LOCALLM_RESULT_JSON__{not json}\n"),
            StructuredOutcome::Malformed(_)
        ));
        assert!(matches!(
            parse_structured_results("__LOCALLM_RESULT_JSON__{'a': None}\n"),
            StructuredOutcome::Malformed(_)
        ));
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
        assert_eq!(result["status"], "failure");
        let result=execution.run(json!({"language":"python","code":"import time; print('starting'); locallm_result('started-marker'); time.sleep(10)","timeout_seconds":1})).await.unwrap();
        assert_eq!(result["error"]["code"], "timeout", "{result}");
        assert_eq!(result["status"], "failure");
        // A timed-out side effect reports unknown outcome, not success.
        assert!(result["data"]["result"].is_null(), "{result}");
        assert!(execution
            .run(json!({"language":"unknown","code":"test"}))
            .await
            .is_err());
    }
    #[test]
    fn normalize_code_strips_markdown_fences_and_bom() {
        assert_eq!(normalize_code("  2+2  "), "2+2");
        assert_eq!(normalize_code("```python\n2+2\n```"), "2+2");
        assert_eq!(normalize_code("```\nprint(1)\n```"), "print(1)");
        assert_eq!(normalize_code("\u{feff}```py\n3*3\n```\n"), "3*3");
        assert_eq!(parse_language("Python 3"), Some(Language::Python));
        assert_eq!(parse_language("py"), Some(Language::Python));
        assert_eq!(parse_language("node"), Some(Language::Javascript));
        assert_eq!(parse_language("pwsh"), Some(Language::Powershell));
        assert_eq!(parse_language("typescript"), None);
    }
    #[tokio::test]
    async fn python_notebook_pattern_uses_last_expression_print_and_default_language() {
        let temp = tempfile::tempdir().unwrap();
        let execution =
            LocalExecution::new(ExecutionConfig::default(), temp.path().to_str().unwrap()).unwrap();
        let expr = execution
            .run(json!({"code": "round(19.99 * 1.08, 2)"}))
            .await
            .unwrap();
        assert_eq!(expr["status"], "success", "{expr}");
        assert_eq!(expr["data"]["result"], 21.59, "{expr}");
        let assigned = execution
            .run(json!({"language": "py", "code": "total = 10 + 5"}))
            .await
            .unwrap();
        assert_eq!(assigned["status"], "success", "{assigned}");
        assert_eq!(assigned["data"]["result"], 15, "{assigned}");
        let fenced = execution
            .run(json!({"code": "```python\nmath.sqrt(9)\n```"}))
            .await
            .unwrap();
        assert_eq!(fenced["status"], "success", "{fenced}");
        assert_eq!(fenced["data"]["result"], 3.0, "{fenced}");
        let explicit = execution
            .run(json!({"language":"python","code":"print('log'); locallm_result({'answer': 7}); 99"}))
            .await
            .unwrap();
        assert_eq!(explicit["status"], "success", "{explicit}");
        assert_eq!(explicit["data"]["result"]["answer"], 7, "{explicit}");
        assert!(crate::tool_envelope::validate_envelope(&expr).is_ok(), "{expr}");
    }
    #[tokio::test]
    async fn runs_shell_command_and_streams_output() {
        let temp = tempfile::tempdir().unwrap();
        let execution =
            LocalExecution::new(ExecutionConfig::default(), temp.path().to_str().unwrap()).unwrap();
        let streamed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let streamed_clone = streamed.clone();
        let cb: ChunkCallback = std::sync::Arc::new(move |stream, chunk| {
            streamed_clone.lock().unwrap().push((stream.to_string(), chunk.to_string()));
        });
        let cmd_str = if cfg!(windows) { "Write-Output 'test_streaming_chunk'" } else { "echo 'test_streaming_chunk'" };
        let result = execution
            .run_command_with_stream(
                json!({"command": cmd_str}),
                Some(cb),
            )
            .await
            .unwrap();
        assert_eq!(result["exitCode"], 0, "{result}");
        assert!(result["stdout"].as_str().unwrap().contains("test_streaming_chunk"));
        let chunks = streamed.lock().unwrap();
        assert!(!chunks.is_empty());
        assert!(chunks.iter().any(|(s, c)| s == "stdout" && c.contains("test_streaming_chunk")));
    }
}
