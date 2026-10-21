//! User-authored, command-backed tools.
//!
//! OpenCode V2 lets a user add a tool in-process; Claude Code and Command
//! Code ship an in-process "mod" layer. localLM had no path for a user to
//! add a tool at all: the tool set was compiled in, and the only extension
//! point was writing a full MCP server.
//!
//! This module is that path, deliberately narrow: a plugin is a declarative
//! JSON file that names a command. It is not arbitrary Rust, it is not a
//! shell string, and it is auditable — the user can read exactly what a
//! plugin will run before it runs.
//!
//! # Security
//!
//! A plugin runs **with the user's privileges, exactly like `shell_run`**.
//! It can read the user's files, use the user's network and write the user's
//! disk. Therefore:
//!
//! * the plugin tool must be gated behind the `local_execution` capability,
//!   and
//! * the plugin tool must always ask for approval (the registry marks the
//!   schema with `x-locallm-always-ask`), exactly like `shell_run`.
//!
//! The command is **argv, never a shell string**: `command[0]` is the
//! program and `command[1..]` are its arguments, so no quoting, escaping or
//! injection is ever interpreted by a shell. Model-supplied arguments are
//! appended as separate argv entries (`--key value`), never concatenated
//! into one string.
//!
//! One broken plugin file never takes the catalog down: an invalid plugin
//! is skipped and reported, and every other plugin still loads.
//!
//! The command runs with the plugin directory as its working directory, so a
//! relative script path in a plugin file resolves inside that directory.

// The module is wired into the harness catalog and the command list by the
// central integration; until that lands nothing outside this file can reach
// these items, so the reachability lint would only report the wiring gap.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Maximum bytes one plugin file may occupy.
pub const MAX_FILE_BYTES: usize = 256 * 1024;

/// Maximum number of plugins one directory may hold.
pub const MAX_PLUGINS: usize = 64;

/// Maximum number of argv entries one plugin command may have.
pub const MAX_COMMAND_ARGS: usize = 32;

/// Maximum bytes one argv entry may have.
pub const MAX_COMMAND_ARG_BYTES: usize = 4096;

/// Smallest accepted plugin timeout, in seconds.
pub const MIN_TIMEOUT_SECONDS: u64 = 1;

/// Largest accepted plugin timeout, in seconds.
pub const MAX_TIMEOUT_SECONDS: u64 = 600;

/// Timeout used when a plugin file does not declare one.
pub const DEFAULT_TIMEOUT_SECONDS: u64 = 60;

/// Maximum bytes captured from one output stream.
pub const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// Directory name (under the app data directory) that holds the plugins.
pub const PLUGIN_DIR: &str = "plugins";

/// One user-authored tool.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plugin {
    /// The tool name the model calls. Must be unique and must not collide
    /// with a built-in tool.
    pub name: String,
    /// What the tool does.
    pub description: String,
    /// The command to run, argv style. NEVER a shell string.
    pub command: Vec<String>,
    /// JSON Schema for the tool's arguments.
    ///
    /// This is the *normalized* schema: the file may declare its arguments
    /// as a flat property map (`{"environment": {...}}`) or as a full JSON
    /// Schema; both end up here as one object schema with `properties`,
    /// `required` and `additionalProperties: false`.
    pub arguments: Value,
    /// Required argument names.
    pub required: Vec<String>,
    /// The command timeout.
    pub timeout_seconds: u64,
    /// Where the plugin was loaded from.
    pub path: PathBuf,
}

/// One plugin as the settings page sees it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginView {
    /// The tool name, or the file name for a plugin that failed to load.
    pub name: String,
    /// What the tool does.
    pub description: String,
    /// The command that runs, argv style.
    pub command: Vec<String>,
    /// The normalized argument schema.
    pub arguments: Value,
    /// Required argument names.
    pub required: Vec<String>,
    /// The command timeout, in seconds.
    pub timeout_seconds: u64,
    /// Where the plugin was loaded from.
    pub path: String,
    /// `ok` for a loaded plugin, `error` for one that was skipped.
    pub status: String,
    /// Why the plugin was skipped, when it was.
    pub error: Option<String>,
}

impl PluginView {
    /// The view of a loaded plugin.
    pub fn of(plugin: &Plugin) -> Self {
        Self {
            name: plugin.name.clone(),
            description: plugin.description.clone(),
            command: plugin.command.clone(),
            arguments: plugin.arguments.clone(),
            required: plugin.required.clone(),
            timeout_seconds: plugin.timeout_seconds,
            path: plugin.path.to_string_lossy().into_owned(),
            status: "ok".into(),
            error: None,
        }
    }

    /// The view of a plugin that failed to load.
    pub fn failed(name: &str, path: &Path, error: String) -> Self {
        Self {
            name: name.to_string(),
            description: String::new(),
            command: Vec::new(),
            arguments: Value::Object(Map::new()),
            required: Vec::new(),
            timeout_seconds: DEFAULT_TIMEOUT_SECONDS,
            path: path.to_string_lossy().into_owned(),
            status: "error".into(),
            error: Some(error),
        }
    }
}

/// The plugin file as the user writes it.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginInput {
    /// The tool name the model calls.
    pub name: String,
    /// What the tool does.
    #[serde(default)]
    pub description: String,
    /// The command to run, argv style.
    pub command: Vec<String>,
    /// The argument properties, or a full JSON Schema.
    #[serde(default)]
    pub arguments: Value,
    /// Required argument names.
    #[serde(default)]
    pub required: Vec<String>,
    /// The command timeout, in seconds.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
}

/// The declared result of every plugin tool.
fn output_contract() -> Value {
    json!({
        "type":"object",
        "required":["exit","stdout","stderr","truncated"],
        "properties":{
            "exit":{"type":"integer"},
            "stdout":{"type":"string"},
            "stderr":{"type":"string"},
            "truncated":{"type":"boolean"}
        },
        "additionalProperties":false
    })
}

/// The directory that holds the plugins of one app data directory.
pub fn plugin_root(data_dir: &Path) -> PathBuf {
    data_dir.join(PLUGIN_DIR)
}

/// Load every valid plugin from the plugin directory.
///
/// A plugin that fails validation is SKIPPED and reported, never fatal:
/// one broken file must not take the whole catalog down. The second element
/// of the result holds `(name, reason)` pairs for every skipped file.
pub fn load(root: &Path) -> (Vec<Plugin>, Vec<(String, String)>) {
    let mut plugins: Vec<Plugin> = Vec::new();
    let mut errors: Vec<(String, String)> = Vec::new();
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        // A missing plugin directory simply means no plugins.
        Err(_) => return (plugins, errors),
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("json"))
        .collect();
    files.sort();
    let mut names: HashSet<String> = HashSet::new();
    for file in files {
        let label = file
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("plugin")
            .to_string();
        if plugins.len() >= MAX_PLUGINS {
            errors.push((
                label,
                format!("At most {MAX_PLUGINS} plugins are loaded; this one was skipped."),
            ));
            continue;
        }
        match read_plugin(&file, root) {
            Ok(plugin) => {
                if reserved_name(&plugin.name) {
                    errors.push((
                        plugin.name.clone(),
                        "A plugin may not use the name of a built-in tool.".into(),
                    ));
                    continue;
                }
                if !names.insert(plugin.name.clone()) {
                    errors.push((
                        plugin.name.clone(),
                        "Another plugin already uses this name.".into(),
                    ));
                    continue;
                }
                plugins.push(plugin);
            }
            Err(error) => errors.push((label, error)),
        }
    }
    (plugins, errors)
}

/// Read and validate one plugin file.
fn read_plugin(path: &Path, root: &Path) -> Result<Plugin, String> {
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() as usize > MAX_FILE_BYTES {
        return Err(format!(
            "A plugin file may be at most {MAX_FILE_BYTES} bytes."
        ));
    }
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("Invalid plugin JSON: {error}"))?;
    parse_plugin(&value, path, root)
}

/// Validate one parsed plugin file.
fn parse_plugin(value: &Value, path: &Path, root: &Path) -> Result<Plugin, String> {
    let object = value
        .as_object()
        .ok_or("A plugin file must contain a JSON object.")?;
    for key in object.keys() {
        if !matches!(
            key.as_str(),
            "name"
                | "description"
                | "command"
                | "arguments"
                | "required"
                | "timeout_seconds"
                | "timeoutSeconds"
        ) {
            return Err(format!("Unknown plugin field '{key}'."));
        }
    }
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .ok_or("A plugin needs a 'name' string.")?
        .trim()
        .to_string();
    if !valid_name(&name) {
        return Err(format!(
            "The plugin name '{name}' must match ^[a-z][a-z0-9_]{{1,48}}$."
        ));
    }
    let description = object
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();
    if description.len() > 2000 {
        return Err("A plugin description may be at most 2000 characters.".into());
    }
    let command = match object.get("command") {
        Some(Value::Array(items)) => {
            if items.is_empty() || items.len() > MAX_COMMAND_ARGS {
                return Err(format!(
                    "A plugin command must have 1-{MAX_COMMAND_ARGS} entries."
                ));
            }
            let mut command = Vec::with_capacity(items.len());
            for item in items {
                let text = item
                    .as_str()
                    .ok_or("Every plugin command entry must be a string.")?;
                if text.is_empty() || text.len() > MAX_COMMAND_ARG_BYTES {
                    return Err(format!(
                        "Every plugin command entry must be 1-{MAX_COMMAND_ARG_BYTES} bytes."
                    ));
                }
                command.push(text.to_string());
            }
            command
        }
        Some(Value::String(_)) => {
            return Err("A plugin command must be an argv array, never a shell string.".into())
        }
        _ => return Err("A plugin needs a 'command' array.".into()),
    };
    if resolve_program(root, &command[0]).is_none() {
        return Err(format!(
            "The plugin command '{}' is not an executable program.",
            command[0]
        ));
    }
    let arguments = object
        .get("arguments")
        .cloned()
        .unwrap_or(Value::Object(Map::new()));
    let mut schema = normalize_schema(&arguments)?;
    let mut required: Vec<String> = match object.get("required") {
        Some(Value::Array(items)) => {
            let mut required = Vec::with_capacity(items.len());
            for item in items {
                let text = item
                    .as_str()
                    .ok_or("'required' must list argument names as strings.")?;
                required.push(text.to_string());
            }
            required
        }
        Some(_) => return Err("'required' must be an array of argument names.".into()),
        None => Vec::new(),
    };
    if let Some(existing) = schema.get("required").and_then(Value::as_array) {
        for item in existing {
            if let Some(text) = item.as_str() {
                if !required.iter().any(|name| name == text) {
                    required.push(text.to_string());
                }
            }
        }
    }
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for name in &required {
        if !properties.contains_key(name) {
            return Err(format!(
                "'{name}' is required but is not one of the declared arguments."
            ));
        }
    }
    schema["required"] = json!(required);
    // Both spellings of the timeout are accepted: the documented file format
    // writes `timeout_seconds`, the settings UI writes `timeoutSeconds`.
    let timeout_value = object
        .get("timeout_seconds")
        .or_else(|| object.get("timeoutSeconds"));
    let timeout_seconds = match timeout_value {
        Some(value) => value
            .as_u64()
            .ok_or("'timeout_seconds' must be a whole number of seconds.")?,
        None => DEFAULT_TIMEOUT_SECONDS,
    };
    if !(MIN_TIMEOUT_SECONDS..=MAX_TIMEOUT_SECONDS).contains(&timeout_seconds) {
        return Err(format!(
            "'timeout_seconds' must be {MIN_TIMEOUT_SECONDS}-{MAX_TIMEOUT_SECONDS} seconds."
        ));
    }
    Ok(Plugin {
        name,
        description,
        command,
        arguments: schema,
        required,
        timeout_seconds,
        path: path.to_path_buf(),
    })
}

/// The plugin tool names as the model sees them.
pub fn registry(root: &Path) -> Vec<crate::tool_spec::ToolSpec> {
    let (plugins, _) = load(root);
    plugins
        .into_iter()
        .map(|plugin| {
            let mut input = plugin.arguments.clone();
            // A plugin runs with the user's privileges: it always asks.
            input["x-locallm-always-ask"] = json!(true);
            crate::tool_spec::ToolSpec::new(
                plugin.name.clone(),
                plugin.description.clone(),
                input,
            )
            .with_output(output_contract())
        })
        .collect()
}

/// Whether an alias names an installed plugin.
pub fn owns(root: &Path, alias: &str) -> bool {
    if !valid_name(alias) {
        return false;
    }
    load(root)
        .0
        .into_iter()
        .any(|plugin| plugin.name == alias)
}

/// Execute a plugin tool.
///
/// The arguments are validated against the plugin's own schema first, so a
/// call the model made up never reaches the command. A command that exits
/// with a non-zero status is a FAILED result, not a crash: the exit code
/// comes back in the envelope.
pub async fn execute(root: &Path, name: &str, args: &Value) -> Result<Value, String> {
    let (plugins, _) = load(root);
    let plugin = plugins
        .into_iter()
        .find(|plugin| plugin.name == name)
        .ok_or_else(|| format!("Unknown plugin '{name}'."))?;
    crate::tool_validation::validate(&plugin.arguments, args)?;

    let program = resolve_program(root, &plugin.command[0])
        .ok_or_else(|| format!("The plugin command '{}' is not executable.", plugin.command[0]))?;
    let mut command = tokio::process::Command::new(program);
    command.args(&plugin.command[1..]);
    if let Some(properties) = plugin.arguments.get("properties").and_then(Value::as_object) {
        if let Some(fields) = args.as_object() {
            for (key, value) in fields {
                if !properties.contains_key(key) {
                    continue;
                }
                // Separate argv entries: no shell ever sees these values.
                command.arg(format!("--{key}"));
                command.arg(argument_text(value));
            }
        }
    }
    if let Some(directory) = plugin.path.parent() {
        command.current_dir(directory);
    }
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = crate::execution::managed_spawn(command)
        .map_err(|error| format!("The plugin could not start: {error}"))?;
    let stdout = child
        .stdout()
        .take()
        .ok_or("The plugin produced no stdout pipe.")?;
    let stderr = child
        .stderr()
        .take()
        .ok_or("The plugin produced no stderr pipe.")?;
    let out_task = tokio::spawn(read_bounded(stdout));
    let err_task = tokio::spawn(read_bounded(stderr));

    let outcome = tokio::time::timeout(
        Duration::from_secs(plugin.timeout_seconds),
        child.wait(),
    )
    .await;
    let status = match outcome {
        Ok(result) => result.map_err(|error| format!("The plugin failed: {error}"))?,
        Err(_) => {
            let _ = child.start_kill();
            let _ = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
            let _ = out_task.await;
            let _ = err_task.await;
            return Err(format!(
                "The plugin '{}' timed out after {} seconds.",
                plugin.name, plugin.timeout_seconds
            ));
        }
    };
    let (stdout_text, stdout_truncated) = out_task.await.unwrap_or_default();
    let (stderr_text, stderr_truncated) = err_task.await.unwrap_or_default();
    Ok(json!({
        "exit": status.code().unwrap_or(-1),
        "stdout": stdout_text,
        "stderr": stderr_text,
        "truncated": stdout_truncated || stderr_truncated,
    }))
}

/// The text of one argument value.
fn argument_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Read one stream to its end, keeping at most [`MAX_OUTPUT_BYTES`].
///
/// The rest of the stream is drained, so a chatty command never blocks on a
/// full pipe; only the kept part is reported and the rest is marked as
/// truncated.
async fn read_bounded<R>(mut reader: R) -> (String, bool)
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;
    let mut kept: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 8192];
    let mut truncated = false;
    while let Ok(read) = reader.read(&mut chunk).await {
        if read == 0 {
            break;
        }
        if kept.len() < MAX_OUTPUT_BYTES {
            let take = (MAX_OUTPUT_BYTES - kept.len()).min(read);
            kept.extend_from_slice(&chunk[..take]);
            if take < read {
                truncated = true;
            }
        } else {
            truncated = true;
        }
    }
    (String::from_utf8_lossy(&kept).into_owned(), truncated)
}

/// Whether a plugin name is well formed.
fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    if bytes.len() < 2 || bytes.len() > 49 {
        return false;
    }
    if !bytes[0].is_ascii_lowercase() {
        return false;
    }
    bytes[1..]
        .iter()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
}

/// Whether a name belongs to a built-in tool.
fn reserved_name(name: &str) -> bool {
    // Built-in families are prefixed; a plugin may never shadow them.
    if name.starts_with("workspace_")
        || name.starts_with("local_")
        || name.starts_with("skills_")
        || name.starts_with("mcp_")
    {
        return true;
    }
    crate::harness::registry().iter().any(|spec| spec.name == name)
}

/// Turn the declared arguments into one object schema.
fn normalize_schema(arguments: &Value) -> Result<Value, String> {
    let object = arguments
        .as_object()
        .ok_or("'arguments' must be a JSON Schema object.")?;
    let properties = if object.contains_key("properties") {
        object
            .get("properties")
            .cloned()
            .unwrap_or(Value::Object(Map::new()))
    } else {
        Value::Object(object.clone())
    };
    if !properties.is_object() {
        return Err("'arguments.properties' must be an object.".into());
    }
    let mut schema = if object.contains_key("properties") {
        arguments.clone()
    } else {
        json!({"type":"object","properties":properties.clone()})
    };
    schema["type"] = json!("object");
    schema["properties"] = properties;
    schema["additionalProperties"] = json!(false);
    crate::tool_validation::compile(&schema)
        .map_err(|error| format!("Invalid argument schema: {error}"))?;
    Ok(schema)
}

/// Find the executable a plugin command names.
///
/// A name with a path separator is resolved against the plugin directory; a
/// bare name is looked up on `PATH`.
fn resolve_program(root: &Path, program: &str) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }
    if program.contains('/') || program.contains('\\') {
        let candidate = Path::new(program);
        let path = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            root.join(candidate)
        };
        return if is_executable(&path) { Some(path) } else { None };
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|directory| directory.join(program))
        .find(|path| is_executable(path))
}

/// Whether a path is an executable file.
fn is_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    true
}

/// List the installed plugins and any load errors.
#[tauri::command]
pub fn list_plugins(state: tauri::State<'_, crate::AppState>) -> Result<Vec<PluginView>, String> {
    let root = plugin_root(&state.data_dir);
    let (plugins, errors) = load(&root);
    let mut views: Vec<PluginView> = plugins.iter().map(PluginView::of).collect();
    for (name, error) in errors {
        views.push(PluginView::failed(&name, &root.join(format!("{name}.json")), error));
    }
    Ok(views)
}

/// Create or replace one plugin file.
#[tauri::command]
pub fn save_plugin(
    state: tauri::State<'_, crate::AppState>,
    plugin: PluginInput,
) -> Result<PluginView, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the current model operation to finish.")?;
    let root = plugin_root(&state.data_dir);
    std::fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let mut arguments = plugin.arguments.clone();
    if arguments.is_null() {
        arguments = Value::Object(Map::new());
    }
    let document = json!({
        "name": plugin.name.trim(),
        "description": plugin.description.trim(),
        "command": plugin.command,
        "arguments": arguments,
        "required": plugin.required,
        "timeout_seconds": plugin.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECONDS),
    });
    let path = root.join(format!("{}.json", plugin.name.trim()));
    // Validate before writing: a plugin that cannot load never reaches disk.
    let parsed = parse_plugin(&document, &path, &root)?;
    if reserved_name(&parsed.name) {
        return Err("A plugin may not use the name of a built-in tool.".into());
    }
    let text = serde_json::to_string_pretty(&document).map_err(|error| error.to_string())?;
    std::fs::write(&path, text).map_err(|error| error.to_string())?;
    Ok(PluginView::of(&parsed))
}

/// Delete one plugin file.
#[tauri::command]
pub fn remove_plugin(state: tauri::State<'_, crate::AppState>, name: String) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the current model operation to finish.")?;
    let name = name.trim();
    if !valid_name(name) {
        return Err("Unknown plugin.".into());
    }
    let root = plugin_root(&state.data_dir);
    let path = root.join(format!("{name}.json"));
    if !path.is_file() {
        return Err("Unknown plugin.".into());
    }
    std::fs::remove_file(&path).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The argv of a shell that runs one script, or `None` when there is no
    /// shell on this machine.
    fn shell(script: &str) -> Option<Vec<String>> {
        if cfg!(windows) {
            let program = std::env::var("COMSPEC")
                .ok()
                .filter(|value| !value.trim().is_empty())
                .or_else(|| Some("cmd.exe".to_string()))?;
            Some(vec![program, "/C".to_string(), script.to_string()])
        } else {
            let program = if Path::new("/bin/sh").is_file() {
                "/bin/sh".to_string()
            } else {
                resolve_program(Path::new("."), "sh")?
                    .to_string_lossy()
                    .into_owned()
            };
            // The trailing placeholder is `$0`: the arguments a plugin
            // appends then arrive as `$1` and later.
            Some(vec![
                program,
                "-c".to_string(),
                script.to_string(),
                "locallm-plugin".to_string(),
            ])
        }
    }

    fn write(root: &Path, name: &str, body: &str) {
        std::fs::create_dir_all(root).unwrap();
        std::fs::write(root.join(format!("{name}.json")), body).unwrap();
    }

    fn plugin_file(name: &str, command: Value, extra: &str) -> String {
        format!(
            r#"{{"name":"{name}","description":"A test plugin.","command":{command},"arguments":{{"environment":{{"type":"string","description":"Target"}}}},"required":["environment"]{extra}}}"#
        )
    }

    #[test]
    fn a_valid_plugin_loads_and_appears_in_the_registry() {
        let directory = tempfile::tempdir().unwrap();
        let Some(command) = shell("true") else {
            return;
        };
        write(
            directory.path(),
            "deploy",
            &plugin_file("deploy", json!(command), r#","timeoutSeconds":30"#),
        );
        let (plugins, errors) = load(directory.path());
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(plugins.len(), 1);
        let plugin = &plugins[0];
        assert_eq!(plugin.name, "deploy");
        assert_eq!(plugin.description, "A test plugin.");
        assert_eq!(plugin.timeout_seconds, 30);
        assert_eq!(plugin.required, vec!["environment".to_string()]);
        assert!(plugin.arguments["properties"]["environment"].is_object());
        assert_eq!(plugin.arguments["type"], json!("object"));
        assert_eq!(plugin.arguments["additionalProperties"], json!(false));

        let specs = registry(directory.path());
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "deploy");
        assert!(specs[0].has_output_contract());
        // A plugin always asks: it runs with the user's privileges.
        assert_eq!(specs[0].input["x-locallm-always-ask"], json!(true));
        assert!(owns(directory.path(), "deploy"));
        assert!(!owns(directory.path(), "missing"));
    }

    #[test]
    fn a_plugin_with_an_invalid_name_is_skipped_and_reported() {
        let directory = tempfile::tempdir().unwrap();
        let Some(command) = shell("true") else {
            return;
        };
        write(
            directory.path(),
            "Bad-Name",
            &plugin_file("Bad-Name", json!(command), ""),
        );
        let (plugins, errors) = load(directory.path());
        assert!(plugins.is_empty());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].1.contains("name"), "{:?}", errors[0]);
    }

    #[test]
    fn a_plugin_that_collides_with_a_built_in_is_skipped() {
        let directory = tempfile::tempdir().unwrap();
        let Some(command) = shell("true") else {
            return;
        };
        write(
            directory.path(),
            "file_read",
            &plugin_file("file_read", json!(command), ""),
        );
        let (plugins, errors) = load(directory.path());
        assert!(plugins.is_empty());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].1.contains("built-in"), "{:?}", errors[0]);
    }

    #[test]
    fn a_plugin_with_a_shell_string_command_is_skipped() {
        let directory = tempfile::tempdir().unwrap();
        write(
            directory.path(),
            "shellish",
            &plugin_file("shellish", json!("bash -lc scripts/deploy.sh"), ""),
        );
        let (plugins, errors) = load(directory.path());
        assert!(plugins.is_empty());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].1.contains("argv"), "{:?}", errors[0]);
    }

    #[test]
    fn a_plugin_that_requires_an_unknown_argument_is_skipped() {
        let directory = tempfile::tempdir().unwrap();
        let Some(command) = shell("true") else {
            return;
        };
        write(
            directory.path(),
            "wrong",
            &format!(
                r#"{{"name":"wrong","command":{},"arguments":{{"environment":{{"type":"string"}}}},"required":["missing"]}}"#,
                serde_json::to_string(&command).unwrap(),
            ),
        );
        let (plugins, errors) = load(directory.path());
        assert!(plugins.is_empty());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].1.contains("missing"), "{:?}", errors[0]);
    }

    #[test]
    fn a_plugin_with_a_long_timeout_is_skipped() {
        let directory = tempfile::tempdir().unwrap();
        let Some(command) = shell("true") else {
            return;
        };
        write(
            directory.path(),
            "slow",
            &plugin_file("slow", json!(command), r#","timeout_seconds":601"#),
        );
        let (plugins, errors) = load(directory.path());
        assert!(plugins.is_empty());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].1.contains("timeout_seconds"), "{:?}", errors[0]);
    }

    #[test]
    fn the_documented_file_format_loads_and_both_timeout_spellings_are_read() {
        let directory = tempfile::tempdir().unwrap();
        let Some(command) = shell("true") else {
            return;
        };
        let body = format!(
            r#"{{"name":"deploy","description":"Deploy the current build to staging.","command":{},"arguments":{{"environment":{{"type":"string","description":"Target environment"}}}},"required":["environment"],"timeout_seconds":60}}"#,
            serde_json::to_string(&command).unwrap()
        );
        write(directory.path(), "deploy", &body);
        let (plugins, errors) = load(directory.path());
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "deploy");
        assert_eq!(plugins[0].description, "Deploy the current build to staging.");
        assert_eq!(plugins[0].timeout_seconds, 60);
        assert_eq!(plugins[0].command, command);
        assert_eq!(plugins[0].required, vec!["environment".to_string()]);

        // The camelCase spelling of the same field is accepted too.
        write(
            directory.path(),
            "deploy",
            &plugin_file("deploy", json!(command), r#","timeoutSeconds":90"#),
        );
        let (plugins, errors) = load(directory.path());
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(plugins[0].timeout_seconds, 90);
    }

    #[test]
    fn a_broken_plugin_file_is_reported_and_the_others_still_load() {
        let directory = tempfile::tempdir().unwrap();
        let Some(command) = shell("true") else {
            return;
        };
        write(directory.path(), "broken", "{not json");
        write(
            directory.path(),
            "good",
            &plugin_file("good", json!(command), ""),
        );
        let (plugins, errors) = load(directory.path());
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].name, "good");
        assert_eq!(errors.len(), 1);
        assert!(errors[0].1.contains("JSON"), "{:?}", errors[0]);
    }

    #[test]
    fn a_plugin_file_that_is_too_large_is_skipped() {
        let directory = tempfile::tempdir().unwrap();
        let Some(command) = shell("true") else {
            return;
        };
        let padding = "x".repeat(MAX_FILE_BYTES);
        write(
            directory.path(),
            "huge",
            &format!(
                r#"{{"name":"huge","description":"{padding}","command":{}}}"#,
                serde_json::to_string(&command).unwrap()
            ),
        );
        let (plugins, errors) = load(directory.path());
        assert!(plugins.is_empty());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].1.contains("bytes"), "{:?}", errors[0]);
    }

    #[test]
    fn a_plugin_command_that_is_not_executable_is_skipped() {
        let directory = tempfile::tempdir().unwrap();
        write(
            directory.path(),
            "missing",
            &plugin_file("missing", json!(["locallm-plugin-that-does-not-exist"]), ""),
        );
        let (plugins, errors) = load(directory.path());
        assert!(plugins.is_empty());
        assert_eq!(errors.len(), 1);
        assert!(errors[0].1.contains("executable"), "{:?}", errors[0]);
    }

    #[tokio::test]
    async fn execution_runs_the_command_and_returns_exit_stdout_and_stderr() {
        let directory = tempfile::tempdir().unwrap();
        let Some(command) = shell("echo out; echo err 1>&2") else {
            return;
        };
        write(
            directory.path(),
            "runner",
            &plugin_file("runner", json!(command), r#","timeout_seconds":20"#),
        );
        let result = execute(directory.path(), "runner", &json!({"environment":"staging"}))
            .await
            .unwrap();
        assert_eq!(result["exit"], json!(0));
        assert!(result["stdout"].as_str().unwrap().contains("out"));
        assert!(result["stderr"].as_str().unwrap().contains("err"));
        assert_eq!(result["truncated"], json!(false));
        // The declared result contract holds.
        let contract = registry(directory.path())[0].output.clone().unwrap();
        crate::tool_validation::validate_value(&contract, &result).unwrap();
    }

    #[tokio::test]
    async fn execution_passes_arguments_as_separate_argv_entries() {
        let directory = tempfile::tempdir().unwrap();
        let Some(command) = shell("printf '%s\\n' \"$@\"") else {
            return;
        };
        write(
            directory.path(),
            "echoer",
            &plugin_file("echoer", json!(command), r#","timeout_seconds":20"#),
        );
        let result = execute(directory.path(), "echoer", &json!({"environment":"staging"}))
            .await
            .unwrap();
        assert_eq!(result["exit"], json!(0));
        assert!(result["stdout"].as_str().unwrap().contains("--environment"));
        assert!(result["stdout"].as_str().unwrap().contains("staging"));
    }

    #[tokio::test]
    async fn a_failing_command_returns_its_exit_code_instead_of_an_error() {
        let directory = tempfile::tempdir().unwrap();
        let Some(command) = shell("exit 3") else {
            return;
        };
        write(
            directory.path(),
            "failing",
            &plugin_file("failing", json!(command), r#","timeout_seconds":20"#),
        );
        let result = execute(directory.path(), "failing", &json!({"environment":"staging"}))
            .await
            .unwrap();
        assert_eq!(result["exit"], json!(3));
    }

    #[tokio::test]
    async fn execution_validates_the_arguments() {
        let directory = tempfile::tempdir().unwrap();
        let Some(command) = shell("true") else {
            return;
        };
        write(
            directory.path(),
            "strict",
            &plugin_file("strict", json!(command), r#","timeout_seconds":20"#),
        );
        // An unknown argument is rejected.
        assert!(execute(
            directory.path(),
            "strict",
            &json!({"environment":"staging","extra":true})
        )
        .await
        .is_err());
        // A missing required argument is rejected.
        assert!(execute(directory.path(), "strict", &json!({})).await.is_err());
        // An unknown plugin is rejected.
        assert!(execute(directory.path(), "missing", &json!({})).await.is_err());
    }

    #[tokio::test]
    async fn a_slow_plugin_is_stopped_and_reported_as_a_timeout() {
        let directory = tempfile::tempdir().unwrap();
        let Some(command) = shell("sleep 30") else {
            return;
        };
        write(
            directory.path(),
            "slow",
            &plugin_file("slow", json!(command), r#","timeoutSeconds":1"#),
        );
        let error = execute(directory.path(), "slow", &json!({"environment":"x"}))
            .await
            .unwrap_err();
        assert!(error.contains("timed out"), "{error}");
    }

    #[test]
    fn names_follow_the_documented_pattern() {
        assert!(valid_name("deploy"));
        assert!(valid_name("a_b_c"));
        assert!(!valid_name("Deploy"));
        assert!(!valid_name("a"));
        assert!(!valid_name("a-b"));
        assert!(!valid_name(""));
        assert!(!valid_name(&"a".repeat(50)));
        assert!(valid_name(&"a".repeat(49)));
    }
}
