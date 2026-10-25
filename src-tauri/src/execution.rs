use process_wrap::tokio::{CommandWrap, KillOnDrop};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
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
    #[serde(default = "default_shell")]
    pub bash_path: String,
    #[serde(default)]
    pub bash: ExecutionPolicy,
    #[serde(default)]
    pub python: ExecutionPolicy,
    #[serde(default)]
    pub python_venv: String,
}

/// User configuration only. Model arguments cannot change enablement,
/// environment, interpreter selection or the approval policy.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ExecutionPolicy {
    pub enabled: bool,
    pub timeout_seconds: u64,
    pub working_directory: String,
    pub environment: BTreeMap<String, String>,
    pub always_ask: bool,
    pub memory_limit_mib: Option<u64>,
}
impl Default for ExecutionPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            timeout_seconds: 60,
            working_directory: String::new(),
            environment: BTreeMap::new(),
            always_ask: false,
            memory_limit_mib: None,
        }
    }
}
fn default_shell() -> String {
    detect(if cfg!(windows) {
        "powershell.exe"
    } else {
        "bash"
    })
}
pub(crate) fn executable(path: &str) -> bool {
    let path = Path::new(path);
    if !path.is_absolute() || !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    true
}

fn detect(name: &str) -> String {
    let found = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .map(|directory| directory.join(name))
        .find(|path| executable(&path.to_string_lossy()));

    #[cfg(windows)]
    let mut found = found;
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
                if py.is_empty() {
                    detect("python3.exe")
                } else {
                    py
                }
            } else {
                p
            }
        } else {
            let p = detect("python3");
            if p.is_empty() {
                detect("python")
            } else {
                p
            }
        };

        Self {
            python_path: python,
            node_path: detect(if cfg!(windows) { "node.exe" } else { "node" }),
            powershell_path: detect(if cfg!(windows) {
                "powershell.exe"
            } else {
                "pwsh"
            }),
            bash_path: default_shell(),
            bash: ExecutionPolicy::default(),
            python: ExecutionPolicy::default(),
            python_venv: String::new(),
        }
    }
}
impl ExecutionConfig {
    pub fn validate(&self) -> Result<(), String> {
        for path in [
            &self.python_path,
            &self.node_path,
            &self.powershell_path,
            &self.bash_path,
        ] {
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
        self.validate_limits()?;
        if !self.python_venv.is_empty()
            && (!Path::new(&self.python_venv).is_absolute()
                || !venv_python(&self.python_venv).is_file())
        {
            return Err(
                "Select a virtual environment containing bin/python or Scripts/python.exe.".into(),
            );
        }
        Ok(())
    }
    fn validate_limits(&self) -> Result<(), String> {
        for (policy, maximum) in [(&self.bash, 300), (&self.python, 90)] {
            if policy
                .memory_limit_mib
                .is_some_and(|mib| !(64..=65536).contains(&mib))
            {
                return Err("Memory limit must be 64–65536 MiB, or empty for no limit.".into());
            }
            if cfg!(windows) && policy.memory_limit_mib.is_some() {
                return Err(
                    "Per-process memory limits are currently available on Unix only.".into(),
                );
            }
            if !(1..=maximum).contains(&policy.timeout_seconds) {
                return Err(format!("Execution timeout must be 1–{maximum} seconds."));
            }
            if policy.working_directory.len() > 32768 || policy.environment.len() > 64 {
                return Err("Execution settings exceed their size limit.".into());
            }
            for (name, value) in &policy.environment {
                if name.is_empty()
                    || name.len() > 128
                    || name.starts_with(|c: char| c.is_ascii_digit())
                    || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    || value.len() > 8192
                    || value.contains('\0')
                {
                    return Err("Environment names must be identifiers and values must be at most 8192 bytes without NUL characters.".into());
                }
            }
        }
        Ok(())
    }
}
fn venv_python(directory: &str) -> PathBuf {
    Path::new(directory).join(if cfg!(windows) {
        "Scripts/python.exe"
    } else {
        "bin/python"
    })
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
fn deserialize_language<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Language, D::Error> {
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
            "",
            "python",
            "py",
            "python3",
            "javascript",
            "js",
            "typescript",
            "ts",
            "powershell",
            "pwsh",
            "ps1",
            "code",
        ];
        if known.contains(&tag.as_str()) || known.contains(&tag_head) {
            rest = &rest[newline + 1..];
        }
    } else {
        return text.to_string();
    }
    let rest = rest.trim_end();
    rest.strip_suffix("```")
        .unwrap_or(rest)
        .trim_end()
        .to_string()
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CodeRequest {
    approved_source: Option<ApprovedSource>,
    #[serde(default, alias = "lang", deserialize_with = "deserialize_language")]
    language: Language,
    #[serde(default)]
    code: String,
    file: Option<String>,
    cwd: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    stdin: String,
    #[serde(default)]
    artifacts: Vec<String>,
    timeout_seconds: Option<u64>,
    /// Optional requested output schema for the structured result channel.
    #[serde(default)]
    result_schema: Option<Value>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ApprovedSource {
    code: String,
    sha256: String,
}
pub type ChunkCallback = std::sync::Arc<dyn Fn(&str, &str) + Send + Sync>;

/// process-wrap's KillOnDrop only sets Tokio's flag on Unix; that kills the
/// leader, not its process group. Explicitly invoke the wrapper's tree kill
/// when the execution future is dropped by Stop, cancellation or a timeout.
pub(crate) struct ExecutionChild(Box<dyn process_wrap::tokio::ChildWrapper>);
impl std::ops::Deref for ExecutionChild {
    type Target = dyn process_wrap::tokio::ChildWrapper;
    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}
impl std::ops::DerefMut for ExecutionChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_mut()
    }
}
impl Drop for ExecutionChild {
    fn drop(&mut self) {
        let _ = self.0.start_kill();
    }
}
pub(crate) fn managed_spawn(command: tokio::process::Command) -> std::io::Result<ExecutionChild> {
    let mut command = CommandWrap::from(arm_parent_death_signal(command));
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
    command.spawn().map(ExecutionChild)
}

/// Arm `PR_SET_PDEATHSIG` so the operating system kills the child when this
/// process dies.
///
/// Tauri ends the application with `std::process::exit`, so no destructor
/// runs and a `Drop`-based kill never fires. Every child this app owns would
/// therefore outlive it — orphaned, reparented to init, and unreachable
/// through the tool that started it. Arming the parent-death signal makes the
/// kernel kill the child instead, which also covers a crash, a forced
/// termination and a development restart.
///
/// The same guard is already applied to the llama-server runtime; this brings
/// every other model-spawned child under the same rule.
#[cfg(target_os = "linux")]
fn arm_parent_death_signal(mut command: tokio::process::Command) -> tokio::process::Command {
    let parent = rustix::process::getpid();
    // SAFETY: this hook only makes async-signal-safe system calls between
    // fork and exec; it does not allocate or acquire locks.
    unsafe {
        command.pre_exec(move || {
            rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::KILL))?;
            // The parent might have exited before the signal was armed.
            if rustix::process::getppid() != Some(parent) {
                return Err(std::io::ErrorKind::BrokenPipe.into());
            }
            Ok(())
        });
    }
    command
}

#[cfg(not(target_os = "linux"))]
fn arm_parent_death_signal(command: tokio::process::Command) -> tokio::process::Command {
    command
}
pub(crate) fn memory_limit(command: &mut tokio::process::Command, policy: &ExecutionPolicy) {
    #[cfg(unix)]
    if let Some(mib) = policy.memory_limit_mib {
        // setrlimit is applied in the child immediately before exec. No
        // allocation or parent-process resource limit is changed here.
        unsafe {
            command.pre_exec(move || {
                rustix::process::setrlimit(
                    rustix::process::Resource::As,
                    rustix::process::Rlimit {
                        current: Some(mib * 1024 * 1024),
                        maximum: Some(mib * 1024 * 1024),
                    },
                )?;
                Ok(())
            });
        }
    }
    #[cfg(not(unix))]
    let _ = (command, policy);
}

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
    timeout_seconds: Option<u64>,
    #[serde(default)]
    stdin: String,
    #[serde(default)]
    artifacts: Vec<String>,
}

pub struct LocalExecution {
    config: ExecutionConfig,
    directory: PathBuf,
    workspace: crate::workspace::Workspace,
}
impl LocalExecution {
    pub fn new(config: ExecutionConfig, directory: &str) -> Result<Self, String> {
        config.validate_limits()?;
        Ok(Self {
            config,
            directory: std::fs::canonicalize(directory).map_err(|e| e.to_string())?,
            workspace: crate::workspace::Workspace::open(directory)?,
        })
    }
    pub fn code_available(&self) -> bool {
        self.python_available()
            || executable(&self.config.node_path)
            || executable(&self.config.powershell_path)
    }
    fn python_available(&self) -> bool {
        self.config.python.enabled
            && if self.config.python_venv.is_empty() {
                executable(&self.config.python_path)
            } else {
                executable(&venv_python(&self.config.python_venv).to_string_lossy())
            }
    }
    pub fn command_available(&self) -> bool {
        self.config.bash.enabled && executable(&self.config.bash_path)
    }
    pub fn requires_approval(&self, name: &str, arguments: &Value) -> bool {
        if name == "execute_command" {
            self.config.bash.always_ask
        } else {
            arguments
                .get("language")
                .and_then(Value::as_str)
                .and_then(parse_language)
                .unwrap_or_default()
                == Language::Python
                && self.config.python.always_ask
        }
    }
    /// Compile a requested result schema through the bounded offline validator
    /// so invalid or remote-`$ref` schemas are rejected before a process runs.
    fn check_result_schema(schema: &Value) -> Result<(), String> {
        if !schema.is_object() {
            return Err("result_schema must be a JSON object.".into());
        }
        crate::tool_validation::compile(schema).map(|_| ())
    }

    pub fn prepare_arguments(&self, name: &str, mut arguments: Value) -> Result<Value, String> {
        if arguments.get("approved_source").is_some() {
            return Err("Approval snapshots are set by the native dispatcher only.".into());
        }
        if let Some(schema) = arguments
            .get("result_schema")
            .filter(|_| name == "run_code")
        {
            Self::check_result_schema(schema)?;
        }
        let configured = if name == "execute_command" {
            &self.config.bash.working_directory
        } else if arguments["language"]
            .as_str()
            .and_then(parse_language)
            .unwrap_or_default()
            == Language::Python
        {
            &self.config.python.working_directory
        } else {
            ""
        };
        let cwd = self.working_directory(arguments["cwd"].as_str(), configured)?;
        arguments["cwd"] = json!(cwd.to_string_lossy());
        if let Some(path) = arguments["file"].as_str().filter(|_| name == "run_code") {
            use sha2::Digest;
            let source = self.workspace.read_script(path)?;
            arguments["approved_source"] = json!({"sha256":format!("{:x}", sha2::Sha256::digest(source.as_bytes())),"code":source});
        }
        Ok(arguments)
    }
    fn working_directory(
        &self,
        requested: Option<&str>,
        configured: &str,
    ) -> Result<PathBuf, String> {
        let path = requested.filter(|p| !p.is_empty()).unwrap_or(configured);
        let path = if path.is_empty() {
            self.directory.clone()
        } else {
            self.directory.join(path)
        };
        let resolved = std::fs::canonicalize(path)
            .map_err(|e| format!("Cannot open working directory: {e}"))?;
        if !resolved.starts_with(&self.directory) || !resolved.is_dir() {
            return Err("Working directory must be a directory within the authorized workspace. Native execution still runs with your account permissions; this is not OS isolation.".into());
        }
        Ok(resolved)
    }
    pub(crate) fn shell_directory(&self) -> Result<PathBuf, String> {
        self.working_directory(None, &self.config.bash.working_directory)
    }
    fn collect_artifacts(&self, paths: &[String]) -> (Vec<Value>, Vec<Value>) {
        let mut artifacts = Vec::new();
        let mut errors = Vec::new();
        for path in paths {
            match self.workspace.output_artifact(path) {
                Ok((bytes, mime)) => {
                    use sha2::Digest;
                    artifacts.push(json!({"path":path, "bytes":bytes.len(), "mimeType":mime, "sha256":format!("{:x}", sha2::Sha256::digest(&bytes))}));
                }
                Err(error) => errors.push(json!({"path":path, "error":error})),
            }
        }
        (artifacts, errors)
    }
    pub fn tool(self: std::sync::Arc<Self>) -> crate::connectors::AgentTool {
        let mut languages = Vec::new();
        if self.python_available() {
            languages.push("python");
        }
        if executable(&self.config.node_path) {
            languages.push("javascript");
        }
        if executable(&self.config.powershell_path) {
            languages.push("powershell");
        }
        crate::connectors::AgentTool::execution(self, crate::connectors::ToolView {
            name: "run_code".into(),
            description: "Execute approved source in the workspace with your account permissions (not OS-isolated). Python accepts source OR a relative script file, arguments and stdin; project imports and the configured virtual environment are supported. No dependencies are installed. Last Python expression or locallm_result(value) returns JSON. Other languages require code. Inspect generated files with workspace tools.".into(),
            input_schema: json!({"type":"object", "properties": {
                "language":{"type":"string", "enum":languages, "description":"Defaults to Python when enabled; otherwise specify an offered language."},
                "code":{"type":"string", "maxLength":32768, "description":"Source code, without markdown fences. Mutually exclusive with file."},
                "file":{"type":"string", "maxLength":4096, "description":"Python script path relative to the authorized workspace. Mutually exclusive with code."},
                "cwd":{"type":"string", "description":"Working directory inside the authorized workspace."},
                "args":{"type":"array", "maxItems":128, "items":{"type":"string", "maxLength":8192}},
                "stdin":{"type":"string", "maxLength":65536},
                "artifacts":{"type":"array", "maxItems":16, "items":{"type":"string", "maxLength":4096}, "description":"Workspace-relative output files to show after execution. PNG/JPEG/WebP and UTF-8 text up to 2 MiB can be previewed."},
                "timeout_seconds":{"type":"integer", "minimum":1, "maximum":90},
                "result_schema":{"type":"object", "description":"Optional JSON Schema the structured result must satisfy. A mismatch fails the run instead of returning the value."}
            }, "oneOf":[{"required":["code"]},{"required":["file"]}], "additionalProperties":false})
        })
    }
    pub fn command_tool(self: std::sync::Arc<Self>) -> crate::connectors::AgentTool {
        crate::connectors::AgentTool::execution(
            self,
            crate::connectors::ToolView {
                name: "execute_command".into(),
                description: "Execute a shell command (CLI, build tool, test runner, git, etc.) locally in the workspace directory. Uses the configured shell (Bash by default on Linux). Approval follows the conversation and shell settings. Output is bounded; timeout is capped by the user setting (up to 300 seconds). Do not use this for calculations — call local_run_code with Python instead.".into(),
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
                        "stdin": { "type": "string", "maxLength": 65536 },
                        "artifacts": { "type": "array", "maxItems": 16, "items": { "type": "string", "maxLength": 4096 }, "description": "Workspace-relative output files to show after execution." },
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
        if let Some(schema) = &request.result_schema {
            Self::check_result_schema(schema)?;
        }
        let python = request.language == Language::Python;
        if python && !self.config.python.enabled {
            return Err("Python execution is disabled in Settings.".into());
        }
        if request.file.is_some() && (!python || !request.code.trim().is_empty()) {
            return Err("Use either Python source or a Python file, never both.".into());
        }
        if let Some(file) = &request.file {
            request.code = self.workspace.read_script(file)?;
            if let Some(approved) = &request.approved_source {
                use sha2::Digest;
                if approved.code != request.code
                    || format!("{:x}", sha2::Sha256::digest(request.code.as_bytes()))
                        != approved.sha256
                {
                    return Err("The Python file changed after approval. Review and approve its current source again.".into());
                }
            }
        }
        if (!python && (!request.args.is_empty() || !request.stdin.is_empty()))
            || request.args.len() > 128
            || request
                .args
                .iter()
                .any(|arg| arg.len() > 8192 || arg.contains('\0'))
            || request.stdin.len() > 65536
            || request.artifacts.len() > 16
        {
            return Err("Python accepts up to 128 arguments and 64 KiB of stdin. Other code languages do not accept arguments or stdin.".into());
        }
        let timeout = request.timeout_seconds.unwrap_or(if python {
            self.config.python.timeout_seconds
        } else {
            60
        });
        let timeout = if python {
            timeout.min(self.config.python.timeout_seconds)
        } else {
            timeout
        };
        let directory = self.working_directory(
            request.cwd.as_deref(),
            if python {
                &self.config.python.working_directory
            } else {
                ""
            },
        )?;
        request.code = normalize_code(&request.code);
        if request.code.trim().is_empty()
            || request.code.len() > 32768
            || !(1..=90).contains(&timeout)
        {
            return Err("Code must contain 1–32768 bytes and timeout must be 1–90 seconds.".into());
        }
        let python_executable = if self.config.python_venv.is_empty() {
            self.config.python_path.clone()
        } else {
            venv_python(&self.config.python_venv)
                .to_string_lossy()
                .into_owned()
        };
        let (program, args, wrapped_code): (&str, Vec<String>, String) = match request.language {
            Language::Python => (
                &python_executable,
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
        if !executable(program) {
            return Err(
                "Configure this language's interpreter on the Execution page first.".into(),
            );
        }
        let mut command = tokio::process::Command::new(program);
        let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
        command
            .args(&arg_refs)
            .current_dir(&directory)
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
        if python {
            command.envs(&self.config.python.environment);
            if !self.config.python_venv.is_empty() {
                command.env("VIRTUAL_ENV", &self.config.python_venv);
                let inherited_path = self
                    .config
                    .python
                    .environment
                    .get("PATH")
                    .map(std::ffi::OsString::from)
                    .or_else(|| std::env::var_os("PATH"))
                    .unwrap_or_default();
                let paths = std::iter::once(
                    venv_python(&self.config.python_venv)
                        .parent()
                        .unwrap_or(Path::new(&self.config.python_venv))
                        .to_path_buf(),
                )
                .chain(std::env::split_paths(&inherited_path));
                command.env(
                    "PATH",
                    std::env::join_paths(paths).map_err(|e| e.to_string())?,
                );
            }
        }
        if python {
            memory_limit(&mut command, &self.config.python);
        }
        let mut child = managed_spawn(command)
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
        let result = tokio::time::timeout(Duration::from_secs(timeout), async {
            let write = async {
                // Structured-result helpers ride in front of the approved code
                // so plain prints stay logs and only marker lines count. For
                // PowerShell the helper is defined in the -Command prelude.
                let payload = if python {
                    json!({"code":request.code, "args":request.args, "stdin":request.stdin,
                        "file":request.file.as_ref().map(|file| self.directory.join(file).to_string_lossy().into_owned())}).to_string()
                } else if matches!(request.language, Language::Powershell) {
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
        let (artifacts, artifact_errors) = self.collect_artifacts(&request.artifacts);
        let legacy = json!({"provider":"local","cwd":directory.to_string_lossy(),"workspace":self.directory.to_string_lossy(),"interpreter":program,"exitCode":exit_code,"stdout":stdout_text,"stderr":stderr_text,"error":error,"isError":error.is_some() || exit_code!=Some(0),"durationMs":duration_ms,"artifacts":artifacts,"artifactErrors":artifact_errors});
        Ok(finish_execution_result(
            legacy,
            structured,
            request.result_schema.as_ref(),
            duration_ms,
        ))
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
        if !self.command_available() {
            return Err(
                "Shell execution is disabled or no shell is configured in Settings.".into(),
            );
        }
        let timeout = request
            .timeout_seconds
            .unwrap_or(self.config.bash.timeout_seconds)
            .min(self.config.bash.timeout_seconds);
        if request.command.trim().is_empty()
            || request.command.len() > 32768
            || !(1..=300).contains(&timeout)
            || request.stdin.len() > 65536
            || request.artifacts.len() > 16
        {
            return Err(
                "Command must contain 1–32768 bytes and timeout must be 1–300 seconds.".into(),
            );
        }

        let target_dir =
            self.working_directory(request.cwd.as_deref(), &self.config.bash.working_directory)?;
        let mut cmd = tokio::process::Command::new(&self.config.bash_path);
        let powershell = Path::new(&self.config.bash_path)
            .file_stem()
            .is_some_and(|s| {
                s.to_string_lossy()
                    .to_ascii_lowercase()
                    .contains("powershell")
                    || s == "pwsh"
            });
        if powershell {
            cmd.args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &request.command,
            ]);
        } else {
            cmd.args(["-c", &request.command]);
        }

        cmd.current_dir(&target_dir)
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
                cmd.env(name, value);
            }
        }
        cmd.env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONUTF8", "1")
            .envs(&self.config.bash.environment);

        memory_limit(&mut cmd, &self.config.bash);
        let mut child =
            managed_spawn(cmd).map_err(|error| format!("Command launch failed: {error}"))?;
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

        let result = tokio::time::timeout(Duration::from_secs(timeout), async {
            let write = async {
                match input.write_all(request.stdin.as_bytes()).await {
                    Ok(()) => (),
                    Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => (),
                    Err(e) => return Err(e.to_string()),
                }
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
        let (artifacts, artifact_errors) = self.collect_artifacts(&request.artifacts);
        Ok(json!({
            "provider": "local",
            "command": request.command,
            "cwd": target_dir.to_string_lossy(),
            "workspace": self.directory.to_string_lossy(),
            "artifacts": artifacts,
            "artifactErrors": artifact_errors,
            "interpreter": self.config.bash_path,
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
    let mut pending = Vec::new();
    loop {
        let count = stream
            .read(&mut buffer)
            .await
            .map_err(|error| error.to_string())?;
        if count == 0 {
            if let Some(ref cb) = on_chunk {
                let text = crate::terminal_ui::decode(&mut pending, &[], true);
                if !text.is_empty() {
                    cb(stream_name, &text);
                }
            }
            return Ok(());
        }
        let available = 65_536 - output.len();
        let to_take = count.min(available);
        if to_take > 0 {
            output.extend_from_slice(&buffer[..to_take]);
            if let Some(ref cb) = on_chunk {
                let chunk_text =
                    crate::terminal_ui::decode(&mut pending, &buffer[..to_take], false);
                if !chunk_text.is_empty() {
                    cb(stream_name, &chunk_text);
                }
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
    r#"import ast, collections, datetime, decimal, fractions, itertools, json, math, random, re, statistics, sys, io, os
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
request = json.loads(sys.stdin.read())
src = _strip(request["code"])
filename = request.get("file") or "<run_code>"
sys.stdin = io.StringIO(request.get("stdin", ""))
sys.argv = [filename] + request.get("args", [])
sys.path.insert(0, os.path.dirname(filename) if request.get("file") else os.getcwd())
sys.path[1:1] = [path for path in os.environ.get("PYTHONPATH", "").split(os.pathsep) if path]
g = {
    "__name__": "__main__",
    "__builtins__": __builtins__,
    "__file__": filename,
    "locallm_result": locallm_result,
    "math": math, "json": json, "datetime": datetime, "decimal": decimal,
    "fractions": fractions, "statistics": statistics, "re": re,
    "collections": collections, "itertools": itertools, "random": random,
}
if not src:
    raise SystemExit(0)
tree = ast.parse(src, filename=filename)
if not tree.body:
    raise SystemExit(0)
*head, tail = tree.body
if head:
    exec(compile(ast.Module(body=head, type_ignores=[]), filename, "exec"), g, g)
if isinstance(tail, ast.Expr):
    value = eval(compile(ast.Expression(tail.value), filename, "eval"), g, g)
    if _emitted["n"] == 0 and value is not None:
        locallm_result(value)
else:
    exec(compile(ast.Module(body=[tail], type_ignores=[]), filename, "exec"), g, g)
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
        let Some(payload) = line.strip_prefix(RESULT_MARKER) else {
            continue;
        };
        if payload.len() > MAX_RESULT_BYTES {
            return StructuredOutcome::TooLarge(payload.len());
        }
        match serde_json::from_str::<Value>(payload.trim()) {
            Ok(value) => values.push(value),
            Err(error) => {
                return StructuredOutcome::Malformed(format!(
                    "Structured result is not valid JSON: {error}"
                ))
            }
        }
    }
    match values.len() {
        0 => StructuredOutcome::Missing,
        1 => StructuredOutcome::Value(values.into_iter().next().unwrap()),
        count => StructuredOutcome::Malformed(format!(
            "Expected exactly one structured result but found {count}."
        )),
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
    let stdout_text = legacy
        .get("stdout")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let stderr_text = legacy
        .get("stderr")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

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
            Some(
                json!({"code": "output-truncated", "message": "An output stream exceeded 64 KiB; the structured result cannot be trusted complete.", "retryable": false}),
            ),
        ),
        StructuredOutcome::Missing if timed_out => (
            crate::tool_envelope::Status::Failure,
            Value::Null,
            "unknown",
            Some(
                json!({"code": "timeout", "message": "Execution timed out; whether the result was produced is unknown. Do not rerun side-effecting code automatically.", "retryable": false}),
            ),
        ),
        StructuredOutcome::Missing if !exit_ok => (
            crate::tool_envelope::Status::Failure,
            Value::Null,
            "exit-nonzero",
            Some(
                json!({"code": "nonzero-exit", "message": format!("Process exited with {exit_code_value}; no structured result was emitted."), "retryable": false}),
            ),
        ),
        StructuredOutcome::Missing => match stdout_as_result(&stdout_text) {
            Some(value) => (crate::tool_envelope::Status::Success, value, "ok", None),
            None => (
                crate::tool_envelope::Status::Failure,
                Value::Null,
                "missing",
                Some(
                    json!({"code": "missing-result", "message": "Execution produced no result. Print the answer or end with an expression, e.g. print(2+2) or locallm_result(value).", "retryable": true}),
                ),
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
            Some(
                json!({"code": "result-too-large", "message": format!("Structured result is {bytes} bytes, above the 64 KiB channel limit. Write large output to a workspace file and return its path instead."), "retryable": false}),
            ),
        ),
        StructuredOutcome::Value(value) => {
            if stream_truncated {
                (
                    crate::tool_envelope::Status::Failure,
                    Value::Null,
                    "truncated",
                    Some(
                        json!({"code": "output-truncated", "message": "An output stream exceeded 64 KiB; the structured result cannot be trusted complete.", "retryable": false}),
                    ),
                )
            } else if timed_out {
                (
                    crate::tool_envelope::Status::Failure,
                    Value::Null,
                    "unknown",
                    Some(
                        json!({"code": "timeout", "message": "Execution timed out; whether the result was produced is unknown. Do not rerun side-effecting code automatically.", "retryable": false}),
                    ),
                )
            } else if !exit_ok {
                (
                    crate::tool_envelope::Status::Failure,
                    Value::Null,
                    "exit-nonzero",
                    Some(
                        json!({"code": "nonzero-exit", "message": format!("Process exited with {:?}; structured result is not accepted.", legacy.get("exitCode").unwrap_or(&Value::Null)), "retryable": false}),
                    ),
                )
            } else if let Some(schema) = schema {
                match crate::tool_envelope::validate_against_schema(&value, schema) {
                    Ok(()) => (crate::tool_envelope::Status::Success, value, "ok", None),
                    Err(message) => (
                        crate::tool_envelope::Status::Failure,
                        Value::Null,
                        "schema-mismatch",
                        Some(
                            json!({"code": "schema-mismatch", "message": message, "retryable": false}),
                        ),
                    ),
                }
            } else {
                (crate::tool_envelope::Status::Success, value, "ok", None)
            }
        }
    };

    let truncated = stream_truncated || matches!(result_status, "too-large" | "truncated");
    let mut envelope = crate::tool_envelope::make(
        status,
        json!({
            "exitCode": legacy.get("exitCode").cloned().unwrap_or(Value::Null),
            "logs": {"stdout": stdout_text, "stderr": stderr_text},
            "result": data_result,
            "resultStatus": result_status,
        }),
        legacy["artifacts"].as_array().cloned().unwrap_or_default(),
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
    cmd.kill_on_drop(true);
    if lower.contains("powershell") || lower.contains("pwsh") {
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$PSVersionTable.PSVersion.ToString()",
        ]);
    } else {
        cmd.arg("--version");
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
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
            if !output.status.success() {
                return Err(format!(
                    "Interpreter check failed ({}): {}",
                    output.status,
                    stderr.chars().take(2000).collect::<String>()
                ));
            }
            if !stdout.is_empty() {
                Ok(stdout)
            } else if !stderr.is_empty() {
                Ok(stderr)
            } else {
                Ok(format!(
                    "Executable verified (exit code {})",
                    output.status.code().unwrap_or(0)
                ))
            }
        }
        Ok(Err(e)) => Err(format!("Failed to run executable: {e}")),
        Err(_) => Err("Interpreter check timed out after 5 seconds.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    #[test]
    fn migrates_existing_settings_and_validates_user_limits() {
        let mut config: ExecutionConfig =
            serde_json::from_value(json!({"pythonPath":"", "nodePath":"", "powershellPath":""}))
                .unwrap();
        assert!(config.bash.enabled && config.python.enabled);
        assert!(!config.bash.always_ask && !config.python.always_ask);
        config.python.timeout_seconds = 91;
        assert!(config.validate().is_err());
        config.python.timeout_seconds = 60;
        config
            .python
            .environment
            .insert("BAD=NAME".into(), "value".into());
        assert!(config.validate().is_err());
    }
    #[tokio::test]
    async fn python_scripts_use_project_imports_arguments_stdin_environment_and_venv() {
        let temp = tempfile::tempdir().unwrap();
        let config = ExecutionConfig::default();
        let venv = temp.path().join(".venv");
        let status = tokio::process::Command::new(&config.python_path)
            .args(["-m", "venv", "--without-pip"])
            .arg(&venv)
            .status()
            .await
            .unwrap();
        assert!(status.success());
        std::fs::write(temp.path().join("helper.py"), "VALUE = 'héllo世界'\n").unwrap();
        std::fs::write(temp.path().join("script.py"), "import helper, sys, os, json\nfrom pathlib import Path\nPath('result.csv').write_text('value\\n42\\n')\nlocallm_result({'value': helper.VALUE, 'args': sys.argv[1:], 'stdin': input(), 'env': os.environ['TEST_VALUE'], 'venv': sys.prefix, 'file': __file__})\n").unwrap();
        let mut config = config;
        config.python_venv = venv.to_string_lossy().into_owned();
        config
            .python
            .environment
            .insert("TEST_VALUE".into(), "from settings".into());
        let execution = LocalExecution::new(config, temp.path().to_str().unwrap()).unwrap();
        let result = execution.run(json!({"file":"script.py", "args":["with spaces", "$(literal)"], "stdin":"test input\n", "artifacts":["result.csv"]})).await.unwrap();
        assert_eq!(result["status"], "success", "{result}");
        assert_eq!(result["data"]["result"]["value"], "héllo世界");
        assert_eq!(result["data"]["result"]["stdin"], "test input");
        assert_eq!(
            result["data"]["result"]["args"],
            json!(["with spaces", "$(literal)"])
        );
        assert_eq!(result["data"]["result"]["env"], "from settings");
        assert_eq!(
            result["data"]["result"]["venv"],
            venv.to_string_lossy().as_ref()
        );
        assert_eq!(
            std::fs::read_to_string(temp.path().join("result.csv")).unwrap(),
            "value\n42\n"
        );
        assert_eq!(result["artifacts"][0]["path"], "result.csv");
        assert_eq!(result["artifacts"][0]["mimeType"], "text/plain");
        assert_eq!(result["artifacts"][0]["sha256"].as_str().unwrap().len(), 64);
        assert!(execution
            .run(json!({"file":"script.py", "code":"print('ambiguous')"}))
            .await
            .is_err());
        let result = execution
            .run(json!({"code":"raise ValueError('readable failure')"}))
            .await
            .unwrap();
        assert_eq!(result["isError"], true);
        assert!(result["stderr"]
            .as_str()
            .unwrap()
            .contains("ValueError: readable failure"));
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn bash_supports_arrays_stdin_limits_and_rejects_escaped_working_directories() {
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("workspace");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::os::unix::fs::symlink(parent.path(), root.join("outside")).unwrap();
        let mut config = ExecutionConfig::default();
        config.bash.timeout_seconds = 1;
        config
            .bash
            .environment
            .insert("TEST_VALUE".into(), "héllo".into());
        let execution = LocalExecution::new(config, root.to_str().unwrap()).unwrap();
        let result = execution.run_command(json!({"command":"items=(a b); read -r line; printf artifact > out.txt; printf '%s:%s:%s' \"${items[1]}\" \"$line\" \"$TEST_VALUE\"", "stdin":"world\n", "cwd":"sub", "artifacts":["sub/out.txt"]})).await.unwrap();
        assert_eq!(result["exitCode"], 0, "{result}");
        assert_eq!(result["stdout"], "b:world:héllo");
        assert_eq!(result["artifacts"][0]["path"], "sub/out.txt");
        assert_eq!(result["cwd"], root.join("sub").to_string_lossy().as_ref());
        for cwd in ["..", "sub/../..", "outside"] {
            assert!(
                execution
                    .run_command(json!({"command":"pwd", "cwd":cwd}))
                    .await
                    .is_err(),
                "{cwd}"
            );
        }
        let result = execution
            .run_command(json!({"command":"printf failed >&2; exit 7"}))
            .await
            .unwrap();
        assert_eq!(result["exitCode"], 7);
        assert_eq!(result["stderr"], "failed");
        let result = execution
            .run_command(json!({"command":"sleep 10", "timeout_seconds":300}))
            .await
            .unwrap();
        assert_eq!(result["isError"], true);
        assert!(result["durationMs"].as_u64().unwrap() < 5000, "{result}");
    }
    #[tokio::test]
    async fn disabled_tools_and_stricter_approval_are_enforced() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = ExecutionConfig::default();
        config.python.enabled = false;
        config.bash.enabled = false;
        let execution = LocalExecution::new(config.clone(), temp.path().to_str().unwrap()).unwrap();
        assert!(!execution.command_available());
        assert!(execution
            .run(json!({"code":"print('forbidden')"}))
            .await
            .is_err());
        assert!(execution
            .run_command(json!({"command":"echo forbidden"}))
            .await
            .is_err());
        config.bash.always_ask = true;
        config.python.always_ask = true;
        let execution = std::sync::Arc::new(
            LocalExecution::new(config, temp.path().to_str().unwrap()).unwrap(),
        );
        assert!(execution
            .clone()
            .command_tool()
            .requires_approval(&json!({"command":"echo preview"})));
        let tool = execution.tool();
        assert!(tool.requires_approval(&json!({"code":"1+1"})));
        assert!(!tool.requires_approval(&json!({"language":"javascript", "code":"1+1"})));
        assert!(!tool.tool.input_schema["properties"]["language"]["enum"]
            .as_array()
            .unwrap()
            .contains(&json!("python")));
    }
    #[test]
    fn a_missing_interpreter_does_not_disable_other_native_tools() {
        let temp = tempfile::tempdir().unwrap();
        let config = ExecutionConfig {
            python_path: temp
                .path()
                .join("removed-python")
                .to_string_lossy()
                .into_owned(),
            node_path: String::new(),
            powershell_path: String::new(),
            ..Default::default()
        };
        let execution = LocalExecution::new(config, temp.path().to_str().unwrap()).unwrap();
        assert!(execution.command_available());
        assert!(!execution.code_available());
    }
    #[tokio::test]
    async fn script_approval_snapshot_contains_source_and_rejects_later_changes() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("script.py"), "print('approved')\n").unwrap();
        let execution = std::sync::Arc::new(
            LocalExecution::new(ExecutionConfig::default(), temp.path().to_str().unwrap()).unwrap(),
        );
        let tool = execution.clone().tool();
        let arguments = tool.prepare_arguments(json!({"file":"script.py"})).unwrap();
        assert_eq!(arguments["approved_source"]["code"], "print('approved')\n");
        assert!(execution.run(arguments.clone()).await.unwrap()["stdout"]
            .as_str()
            .unwrap()
            .contains("approved"));
        std::fs::write(temp.path().join("script.py"), "print('changed')\n").unwrap();
        assert!(execution
            .run(arguments)
            .await
            .unwrap_err()
            .contains("changed after approval"));
        assert!(tool
            .prepare_arguments(
                json!({"file":"script.py","approved_source":{"code":"forged","sha256":"forged"}})
            )
            .is_err());
        assert!(tool
            .prepare_arguments(json!({"code":"print('a')","file":"script.py"}))
            .is_err());
        assert!(tool.prepare_arguments(json!({})).is_err());
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn configured_memory_limit_is_applied_only_to_the_python_child() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = ExecutionConfig::default();
        config.python.memory_limit_mib = Some(128);
        let execution = LocalExecution::new(config, temp.path().to_str().unwrap()).unwrap();
        let result = execution.run(json!({"code":"import resource\nlocallm_result(resource.getrlimit(resource.RLIMIT_AS)[0])"})).await.unwrap();
        assert_eq!(result["data"]["result"], 128 * 1024 * 1024);
        let result = execution
            .run(json!({"code":"data = bytearray(512 * 1024 * 1024)"}))
            .await
            .unwrap();
        assert_eq!(result["isError"], true);
        assert!(result["stderr"].as_str().unwrap().contains("MemoryError"));
        assert_ne!(
            rustix::process::getrlimit(rustix::process::Resource::As).current,
            Some(128 * 1024 * 1024)
        );
    }
    #[tokio::test]
    async fn streaming_preserves_utf8_split_between_reads() {
        let (mut writer, reader) = tokio::io::duplex(16);
        let collected = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let capture = collected.clone();
        let callback: ChunkCallback =
            std::sync::Arc::new(move |_, text| capture.lock().unwrap().push_str(text));
        let write = async move {
            for byte in "héllo世界🌍".as_bytes() {
                writer.write_all(&[*byte]).await.unwrap();
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        };
        let mut bytes = Vec::new();
        let (_, result) = tokio::join!(
            write,
            read_output(reader, &mut bytes, "stdout", Some(callback))
        );
        result.unwrap();
        assert_eq!(*collected.lock().unwrap(), "héllo世界🌍");
    }
    #[tokio::test]
    async fn runs_python_and_javascript_and_reports_nonzero_exit() {
        let temp = tempfile::tempdir().unwrap();
        let execution =
            LocalExecution::new(ExecutionConfig::default(), temp.path().to_str().unwrap()).unwrap();
        for (language, code) in [
            (
                "python",
                "print('héllo'); locallm_result({'greeting': 'héllo'})",
            ),
            (
                "javascript",
                "console.log('héllo'); locallm_result({greeting: 'héllo'});",
            ),
            (
                "powershell",
                "Write-Output 'héllo'; locallm_result(@{greeting='héllo'})",
            ),
        ] {
            if language == "powershell" && execution.config.powershell_path.is_empty() {
                continue; // PowerShell is an optional interpreter on Unix.
            }
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
            assert_eq!(
                result["data"]["result"]["greeting"], "héllo",
                "{language}: {result}"
            );
            assert!(
                crate::tool_envelope::validate_envelope(&result).is_ok(),
                "{language}: {result}"
            );
        }
        let result=execution.run(json!({"language":"python","code":"import sys; print('failed', file=sys.stderr); sys.exit(7)"})).await.unwrap();
        assert_eq!(result["exitCode"], 7);
        assert_eq!(result["isError"], true);
        assert_eq!(result["status"], "failure");
        assert_eq!(result["error"]["code"], "nonzero-exit");
        assert!(result["stderr"].as_str().unwrap().contains("failed"));
        assert!(
            crate::tool_envelope::validate_envelope(&result).is_ok(),
            "{result}"
        );
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
        assert_eq!(
            result["data"]["result"]["nested"]["items"][1],
            "héllo世界🌍"
        );
        assert_eq!(result["data"]["result"]["nested"]["items"][2], Value::Null);
        assert!(result["data"]["logs"]["stdout"]
            .as_str()
            .unwrap()
            .contains("log line"));
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
        assert_eq!(
            printed["data"]["result"], "just a log, no result",
            "{printed}"
        );
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
        assert!(
            crate::tool_envelope::validate_envelope(&nan).is_ok(),
            "{nan}"
        );
    }
    #[tokio::test]
    async fn result_schema_is_advertised_compiled_and_enforced() {
        let temp = tempfile::tempdir().unwrap();
        let execution = std::sync::Arc::new(
            LocalExecution::new(ExecutionConfig::default(), temp.path().to_str().unwrap()).unwrap(),
        );
        let tool = execution.clone().tool();
        // The model-facing schema advertises result_schema.
        assert_eq!(
            tool.tool.input_schema["properties"]["result_schema"]["type"],
            "object"
        );
        // prepare_arguments accepts a valid schema and rejects unusable ones
        // before any interpreter can launch.
        tool.prepare_arguments(json!({"code":"1+1","result_schema":{"type":"number"}}))
            .unwrap();
        for schema in [
            json!({"type":"nonsense"}),
            json!({"$ref":"https://schemas.example/remote.json"}),
            json!({"$ref":"file:///etc/passwd"}),
        ] {
            assert!(
                tool.prepare_arguments(json!({"code":"1+1","result_schema":schema}))
                    .is_err(),
                "{schema}"
            );
        }
        // run() (direct public path, e.g. catalog dispatch) rejects the same
        // schemas before launch: the side-effect file must not exist.
        for schema in [
            json!({"type":"nonsense"}),
            json!({"$ref":"https://schemas.example/remote.json"}),
            json!("not an object"),
        ] {
            assert!(
                execution
                    .run(json!({"code":"1+1","result_schema":schema}))
                    .await
                    .is_err(),
                "{schema}"
            );
        }
        assert!(execution
            .run(json!({"code":"open('sentinel.txt','w').write('x')","result_schema":{"type":"nonsense"}}))
            .await.is_err());
        assert!(
            !temp.path().join("sentinel.txt").exists(),
            "rejected schema ran a process"
        );
        // Public dispatch validates the advertised type too.
        assert!(tool
            .call(json!({"code":"1+1","result_schema":"nope"}))
            .await
            .is_err());
        // Full JSON Schema enforcement on the structured result channel.
        let schema = json!({"type":"object","required":["answer"],"properties":{"answer":{"type":"integer","minimum":2}},"additionalProperties":false});
        let ok = execution
            .run(json!({"code":"locallm_result({'answer': 7})","result_schema":schema}))
            .await
            .unwrap();
        assert_eq!(ok["status"], "success", "{ok}");
        assert_eq!(ok["data"]["result"]["answer"], 7);
        for (label, code) in [
            ("wrong type", "locallm_result({'answer': '7'})"),
            ("below minimum", "locallm_result({'answer': 1})"),
            ("extra key", "locallm_result({'answer': 7, 'extra': True})"),
        ] {
            let bad = execution
                .run(json!({"code": code, "result_schema": schema}))
                .await
                .unwrap();
            assert_eq!(bad["status"], "failure", "{label}: {bad}");
            assert_eq!(bad["error"]["code"], "schema-mismatch", "{label}: {bad}");
        }
    }
    #[test]
    fn structured_result_parsing_joins_wrapped_lines_and_rejects_reprs() {
        assert!(matches!(
            parse_structured_results("log\n"),
            StructuredOutcome::Missing
        ));
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
        assert!(
            crate::tool_envelope::validate_envelope(&expr).is_ok(),
            "{expr}"
        );
    }
    #[tokio::test]
    async fn runs_shell_command_and_streams_output() {
        let temp = tempfile::tempdir().unwrap();
        let execution =
            LocalExecution::new(ExecutionConfig::default(), temp.path().to_str().unwrap()).unwrap();
        let streamed = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let streamed_clone = streamed.clone();
        let cb: ChunkCallback = std::sync::Arc::new(move |stream, chunk| {
            streamed_clone
                .lock()
                .unwrap()
                .push((stream.to_string(), chunk.to_string()));
        });
        let cmd_str = if cfg!(windows) {
            "Write-Output 'test_streaming_chunk'"
        } else {
            "echo 'test_streaming_chunk'"
        };
        let result = execution
            .run_command_with_stream(json!({"command": cmd_str}), Some(cb))
            .await
            .unwrap();
        assert_eq!(result["exitCode"], 0, "{result}");
        assert!(result["stdout"]
            .as_str()
            .unwrap()
            .contains("test_streaming_chunk"));
        let chunks = streamed.lock().unwrap();
        assert!(!chunks.is_empty());
        assert!(chunks
            .iter()
            .any(|(s, c)| s == "stdout" && c.contains("test_streaming_chunk")));
    }
}

/// Probe the selected interpreter, not the shell's `node` alias. This also
/// catches executable impostors and graphical-app PATH differences.
pub(crate) async fn probe_node(path: &str, minimum: (u32, u32, u32)) -> Result<String, String> {
    use std::sync::OnceLock;
    type ProbeCache = std::collections::HashMap<String, (std::time::Instant, String)>;
    static CACHE: OnceLock<tokio::sync::Mutex<ProbeCache>> = OnceLock::new();
    let stamp = std::fs::metadata(path)
        .ok()
        .map(|m| (m.len(), m.modified().ok()));
    let cache_key = format!("{path}:{stamp:?}:{minimum:?}");
    let mut cache = CACHE
        .get_or_init(|| tokio::sync::Mutex::new(ProbeCache::new()))
        .lock()
        .await;
    if let Some((time, version)) = cache.get(&cache_key) {
        if time.elapsed() < Duration::from_secs(30) {
            return Ok(version.clone());
        }
    }
    if !executable(path) {
        return Err("Configure the absolute path to an installed Node.js executable in Execution settings. Desktop apps may not inherit your shell/version-manager PATH.".into());
    }
    let mut command = tokio::process::Command::new(path);
    command
        .args([
            "-p",
            "JSON.stringify({version:process.versions.node,release:process.release.name})",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command
        .spawn()
        .map_err(|e| format!("Cannot start Node.js: {e}"))?;
    let mut output = child
        .stdout
        .take()
        .ok_or("Node output unavailable")?
        .take(4097);
    let mut bytes = Vec::new();
    let status = tokio::time::timeout(Duration::from_secs(5), async {
        output
            .read_to_end(&mut bytes)
            .await
            .map_err(|e| e.to_string())?;
        child.wait().await.map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "Node.js prerequisite probe timed out.")??;
    if !status.success() || bytes.len() > 4096 {
        return Err("The selected executable could not run the Node.js prerequisite probe.".into());
    }
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "The selected executable did not return Node.js version information.")?;
    let version = value["version"]
        .as_str()
        .ok_or("Node.js version is unavailable")?;
    let parts: Vec<u32> = version
        .split('.')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .map_err(|_| "Invalid Node.js version")?;
    if value["release"] != "node" || parts.len() != 3 || (parts[0], parts[1], parts[2]) < minimum {
        return Err(format!("Node.js {}.{}.{}+ is required; selected runtime reports {version}. Install a supported Node version and choose its absolute executable path in Execution settings.", minimum.0, minimum.1, minimum.2));
    }
    if cache.len() >= 64 {
        cache.clear();
    }
    cache.insert(cache_key, (std::time::Instant::now(), version.into()));
    Ok(version.into())
}

#[cfg(test)]
mod prerequisite_tests {
    use super::*;
    #[cfg(unix)]
    #[tokio::test]
    async fn probes_old_wrong_missing_and_feature_specific_node_versions() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let node = temp.path().join("node");
        assert!(probe_node(node.to_str().unwrap(), (18, 0, 0))
            .await
            .unwrap_err()
            .contains("absolute path"));
        std::fs::write(
            &node,
            "#!/bin/sh\nprintf '%s' '{\"release\":\"node\",\"version\":\"18.20.0\"}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&node, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            probe_node(node.to_str().unwrap(), (18, 0, 0))
                .await
                .unwrap(),
            "18.20.0"
        );
        assert!(probe_node(node.to_str().unwrap(), (22, 13, 0))
            .await
            .unwrap_err()
            .contains("22.13.0"));
        std::fs::write(&node, "#!/bin/sh\nprintf 'not node'\n").unwrap();
        assert!(probe_node(node.to_str().unwrap(), (18, 0, 0))
            .await
            .is_err());
    }
}
