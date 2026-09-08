use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Component, Path},
    sync::Arc,
};

pub struct Workspace {
    directory: Dir,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadRequest {
    path: String,
    #[serde(default = "first_line")]
    start_line: usize,
    #[serde(default = "default_lines")]
    line_count: usize,
}
fn first_line() -> usize {
    1
}
fn default_lines() -> usize {
    200
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathRequest {
    path: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteRequest {
    path: String,
    content: String,
}
fn valid_path(path: &str, root_allowed: bool) -> Result<(), String> {
    if root_allowed && path == "." {
        return Ok(());
    }
    if path.is_empty()
        || path.len() > 4096
        || path.contains(['\\', ':'])
        || path.split('/').any(|part| {
            part.is_empty()
                || part == "."
                || part == ".."
                || part.ends_with(['.', ' '])
                || part.chars().any(char::is_control)
                || is_device(part)
        })
        || Path::new(path)
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err("Use a relative workspace path with forward slashes, without parent traversal or Windows device names.".into());
    }
    Ok(())
}
fn is_device(part: &str) -> bool {
    let stem = part
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
}
impl Workspace {
    pub fn open(path: &str) -> Result<Self, String> {
        if path.is_empty() || !Path::new(path).is_absolute() {
            return Err("Choose an absolute workspace directory.".into());
        }
        Ok(Self {
            directory: Dir::open_ambient_dir(path, ambient_authority())
                .map_err(|error| format!("Could not open workspace: {error}"))?,
        })
    }
    pub fn call(&self, name: &str, arguments: Value) -> Result<Value, String> {
        match name {
            "list_files" => {
                let request: PathRequest =
                    serde_json::from_value(arguments).map_err(|error| error.to_string())?;
                valid_path(&request.path, true)?;
                let directory = self
                    .directory
                    .open_dir(&request.path)
                    .map_err(|error| error.to_string())?;
                let mut entries = Vec::new();
                let mut truncated = false;
                for entry in directory.entries().map_err(|error| error.to_string())? {
                    if entries.len() >= 500 {
                        truncated = true;
                        break;
                    }
                    let entry = entry.map_err(|error| error.to_string())?;
                    let kind = entry.file_type().map_err(|error| error.to_string())?;
                    entries.push(json!({"name":entry.file_name().to_string_lossy(),"kind":if kind.is_symlink() { "link" } else if kind.is_dir() { "directory" } else { "file" }}));
                }
                entries.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
                Ok(json!({"entries":entries,"truncated":truncated}))
            }
            "read_file" => {
                let request: ReadRequest =
                    serde_json::from_value(arguments).map_err(|error| error.to_string())?;
                valid_path(&request.path, false)?;
                if request.start_line == 0 || request.line_count == 0 || request.line_count > 500 {
                    return Err(
                        "Use a positive start_line and line_count between 1 and 500.".into(),
                    );
                }
                let file = self
                    .directory
                    .open(&request.path)
                    .map_err(|error| error.to_string())?;
                let metadata = file.metadata().map_err(|error| error.to_string())?;
                if !metadata.is_file() || metadata.len() > 1_048_576 {
                    return Err("Read supports regular UTF-8 files up to 1 MiB.".into());
                }
                let mut bytes = Vec::new();
                file.take(1_048_577)
                    .read_to_end(&mut bytes)
                    .map_err(|error| error.to_string())?;
                if bytes.len() > 1_048_576 {
                    return Err("File grew beyond the read limit.".into());
                }
                let sha256 = format!("{:x}", Sha256::digest(&bytes));
                let text = String::from_utf8(bytes).map_err(|_| "File is not UTF-8 text.")?;
                let content = text
                    .lines()
                    .enumerate()
                    .skip(request.start_line - 1)
                    .take(request.line_count)
                    .map(|(index, line)| format!("{}: {line}", index + 1))
                    .collect::<Vec<_>>()
                    .join("\n");
                if content.len() > 131_072 {
                    return Err("Selected lines exceed 128 KiB. Read a smaller range.".into());
                }
                Ok(
                    json!({"path":request.path,"sha256":sha256,"totalLines":text.lines().count(),"content":content}),
                )
            }
            "create_file" => {
                let request: WriteRequest =
                    serde_json::from_value(arguments).map_err(|error| error.to_string())?;
                valid_path(&request.path, false)?;
                if request.content.len() > 65_536 {
                    return Err("New file content exceeds 64 KiB.".into());
                }
                let mut file = self
                    .directory
                    .open_with(
                        &request.path,
                        OpenOptions::new().write(true).create_new(true),
                    )
                    .map_err(|error| {
                        format!("Could not create file (existing files are preserved): {error}")
                    })?;
                file.write_all(request.content.as_bytes())
                    .and_then(|_| file.sync_all())
                    .map_err(|error| {
                        format!("File write failed; a partial file may remain: {error}")
                    })?;
                Ok(
                    json!({"path":request.path,"bytesWritten":request.content.len(),"sha256":format!("{:x}",Sha256::digest(request.content.as_bytes()))}),
                )
            }
            "create_directory" => {
                let request: PathRequest =
                    serde_json::from_value(arguments).map_err(|error| error.to_string())?;
                valid_path(&request.path, false)?;
                self.directory
                    .create_dir(&request.path)
                    .map_err(|error| error.to_string())?;
                Ok(json!({"path":request.path,"created":true}))
            }
            _ => Err("Unknown workspace tool.".into()),
        }
    }
    pub fn tools(self: Arc<Self>) -> Vec<crate::connectors::AgentTool> {
        let definitions=[
            ("list_files","List up to 500 entries in a workspace directory. Use path . for the root.",json!({"path":{"type":"string"}}),vec!["path"]),
            ("read_file","Read UTF-8 workspace text with line numbers and SHA-256. Maximum file size 1 MiB.",json!({"path":{"type":"string"},"start_line":{"type":"integer","minimum":1},"line_count":{"type":"integer","minimum":1,"maximum":500}}),vec!["path"]),
            ("create_file","Create a new UTF-8 file inside the workspace. Existing files are never overwritten. Parent directory must exist.",json!({"path":{"type":"string"},"content":{"type":"string"}}),vec!["path","content"]),
            ("create_directory","Create one directory inside the workspace. Parent directory must exist.",json!({"path":{"type":"string"}}),vec!["path"]),
        ];
        definitions.into_iter().map(|(name,description,properties,required)| crate::connectors::AgentTool::workspace(self.clone(),crate::connectors::ToolView { name:name.into(),description:description.into(),input_schema:json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}) })).collect()
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceView {
    path: String,
}
#[tauri::command]
pub fn get_workspace(state: tauri::State<'_, crate::AppState>) -> Result<WorkspaceView, String> {
    Ok(WorkspaceView {
        path: state.database()?.workspace_path()?,
    })
}
#[tauri::command]
pub async fn set_workspace(
    state: tauri::State<'_, crate::AppState>,
    path: String,
) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the current model operation to finish.")?;
    if !path.is_empty() {
        Workspace::open(&path)?;
    }
    state.database()?.save_workspace_path(&path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn junction_cannot_read_or_create_outside_workspace() {
        use std::os::windows::process::CommandExt;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "outside fixture").unwrap();
        let link = root.path().join("escape");
        let output=std::process::Command::new("powershell.exe").args(["-NoProfile","-NonInteractive","-Command","New-Item -ItemType Junction -Path $env:LOCALLM_TEST_LINK -Target $env:LOCALLM_TEST_TARGET -ErrorAction Stop | Out-Null"]).env("LOCALLM_TEST_LINK",&link).env("LOCALLM_TEST_TARGET",outside.path()).creation_flags(0x08000000).output().unwrap();
        assert!(
            output.status.success(),
            "Junction fixture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let workspace = Workspace::open(root.path().to_str().unwrap()).unwrap();
        assert!(workspace
            .call("read_file", json!({"path":"escape/secret.txt"}))
            .is_err());
        assert!(workspace
            .call(
                "create_file",
                json!({"path":"escape/new.txt","content":"must not be written"})
            )
            .is_err());
        assert!(!outside.path().join("new.txt").exists());
        assert_eq!(
            std::fs::read_to_string(outside.path().join("secret.txt")).unwrap(),
            "outside fixture"
        );
    }
    #[test]
    fn reads_line_ranges_creates_files_and_preserves_existing_content() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = Workspace::open(temp.path().to_str().unwrap()).unwrap();
        workspace
            .call(
                "create_file",
                json!({"path":"hello.txt","content":"one\ntwo\nthree"}),
            )
            .unwrap();
        assert!(workspace
            .call(
                "create_file",
                json!({"path":"hello.txt","content":"replacement"})
            )
            .is_err());
        let result = workspace
            .call(
                "read_file",
                json!({"path":"hello.txt","start_line":2,"line_count":1}),
            )
            .unwrap();
        assert_eq!(result["content"], "2: two");
        assert_eq!(result["totalLines"], 3);
        let listed = workspace.call("list_files", json!({"path":"."})).unwrap();
        assert_eq!(listed["entries"][0]["name"], "hello.txt");
    }
    #[test]
    fn rejects_escape_devices_binary_oversized_and_invalid_arguments() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = Workspace::open(temp.path().to_str().unwrap()).unwrap();
        for path in [
            "../secret",
            "/absolute",
            "C:/secret",
            "a\\b",
            "file:stream",
            "CON",
            "nul.txt",
            "a/../secret",
        ] {
            assert!(workspace.call("read_file", json!({"path":path})).is_err());
        }
        std::fs::write(temp.path().join("binary"), [255, 254]).unwrap();
        assert!(workspace
            .call("read_file", json!({"path":"binary"}))
            .is_err());
        std::fs::write(temp.path().join("large"), vec![0; 1_048_577]).unwrap();
        assert!(workspace
            .call("read_file", json!({"path":"large"}))
            .is_err());
        assert!(workspace
            .call("read_file", json!({"path":"binary","line_count":0}))
            .is_err());
        assert!(workspace
            .call("list_files", json!({"path":".","extra":true}))
            .is_err());
    }
}
