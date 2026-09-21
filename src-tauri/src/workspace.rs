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
struct EditRequest {
    path: String,
    expected_sha256: String,
    old_text: String,
    new_text: String,
}
fn parse_hash(value: &str) -> Result<[u8; 32], String> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("Use the 64-character SHA-256 shown by read_file.".into());
    }
    let mut hash = [0u8; 32];
    for (index, chunk) in value.as_bytes().chunks(2).enumerate() {
        hash[index] =
            u8::from_str_radix(std::str::from_utf8(chunk).map_err(|_| "Invalid hash.")?, 16)
                .map_err(|_| "Invalid hash.")?;
    }
    Ok(hash)
}
pub(crate) fn valid_path(path: &str, root_allowed: bool) -> Result<(), String> {
    // ponytail: FS exfiltration deny-list lives in sandbox.rs; path-shape
    // rules stay here next to the cap-std enforcement.
    crate::sandbox::check_fs_path("", path, false)?;
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
fn unified_diff(path: &str, before: &str, after: &str) -> String {
    // A small reviewable hunk around the single unique replacement site.
    // The edit backend already guarantees exactly one match.
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();
    let mut start = 0;
    while start < before_lines.len()
        && start < after_lines.len()
        && before_lines[start] == after_lines[start]
    {
        start += 1;
    }
    let mut end_before = before_lines.len();
    let mut end_after = after_lines.len();
    while end_before > start
        && end_after > start
        && before_lines[end_before - 1] == after_lines[end_after - 1]
    {
        end_before -= 1;
        end_after -= 1;
    }
    let context = 3;
    let show_start = start.saturating_sub(context);
    let show_end_before = (end_before + context).min(before_lines.len());
    let show_end_after = (end_after + context).min(after_lines.len());
    let mut diff = format!("--- a/{path}\n+++ b/{path}\n");
    diff.push_str(&format!(
        "@@ -{},{} +{},{} @@\n",
        show_start + 1,
        show_end_before - show_start,
        show_start + 1,
        show_end_after - show_start
    ));
    for (index, line) in before_lines[show_start..start].iter().enumerate() {
        let _ = index;
        diff.push_str(&format!(" {line}\n"));
    }
    for line in &before_lines[start..end_before] {
        diff.push_str(&format!("-{line}\n"));
    }
    for line in &after_lines[start..end_after] {
        diff.push_str(&format!("+{line}\n"));
    }
    for line in &after_lines[end_after..show_end_after] {
        diff.push_str(&format!(" {line}\n"));
    }
    if diff.len() > 16_384 {
        diff.truncate(16_384);
        diff.push_str("\n…diff truncated…\n");
    }
    diff
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
                #[derive(Deserialize)]
                #[serde(deny_unknown_fields)]
                struct WriteRequest {
                    path: String,
                    content: String,
                }
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
            "edit_file" => {
                let request: EditRequest =
                    serde_json::from_value(arguments).map_err(|error| error.to_string())?;
                valid_path(&request.path, false)?;
                if request.old_text.is_empty() || request.old_text.len() > 65_536 {
                    return Err("The text to replace must contain 1–65536 bytes.".into());
                }
                if request.new_text.len() > 65_536 {
                    return Err("Replacement text exceeds 64 KiB.".into());
                }
                if request.old_text == request.new_text {
                    return Err("Replacement text must differ from the text it replaces.".into());
                }
                let expected = parse_hash(&request.expected_sha256)?;
                let file = self
                    .directory
                    .open_with(&request.path, OpenOptions::new().read(true).write(true))
                    .map_err(|error| error.to_string())?;
                let metadata = file.metadata().map_err(|error| error.to_string())?;
                if !metadata.is_file() || metadata.len() > 1_048_576 {
                    return Err("Edit supports regular UTF-8 files up to 1 MiB.".into());
                }
                let mut bytes = Vec::new();
                (&file)
                    .take(1_048_577)
                    .read_to_end(&mut bytes)
                    .map_err(|error| error.to_string())?;
                if bytes.len() > 1_048_576 {
                    return Err("File grew beyond the edit limit.".into());
                }
                if Sha256::digest(&bytes).as_slice() != expected {
                    return Err("This file changed after it was read. Read it again and retry the edit.".into());
                }
                let text = String::from_utf8(bytes).map_err(|_| "File is not UTF-8 text.")?;
                let matches = text.matches(&request.old_text).count();
                if matches == 0 {
                    return Err("The text to replace was not found. Read the current file and retry.".into());
                }
                if matches > 1 {
                    return Err("The text to replace occurs more than once. Use a longer unique match.".into());
                }
                let updated = text.replacen(&request.old_text, &request.new_text, 1);
                {
                    use std::io::{Seek, SeekFrom};
                    let mut writer = file;
                    writer.seek(SeekFrom::Start(0)).map_err(|error| error.to_string())?;
                    writer.set_len(0).map_err(|error| error.to_string())?;
                    writer
                        .write_all(updated.as_bytes())
                        .and_then(|_| writer.sync_all())
                        .map_err(|error| {
                            format!("File write failed; the file may be incomplete: {error}")
                        })?;
                }
                Ok(json!({"path":request.path,"replacements":1,"bytesWritten":updated.len(),"sha256":format!("{:x}",Sha256::digest(updated.as_bytes())),"diff":unified_diff(&request.path,&text,&updated)}))
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
            ("edit_file","Replace one unique text match in an existing UTF-8 file. Requires the SHA-256 from a fresh read_file; fails when the file changed, the match is missing, or it occurs more than once. Returns a unified diff of the change for review. Maximum file size 1 MiB.",json!({"path":{"type":"string"},"expected_sha256":{"type":"string"},"old_text":{"type":"string"},"new_text":{"type":"string"}}),vec!["path","expected_sha256","old_text","new_text"]),
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
    fn edits_require_a_fresh_read_and_one_unique_match() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = Workspace::open(temp.path().to_str().unwrap()).unwrap();
        workspace
            .call("create_file", json!({"path":"note.txt","content":"alpha\nbeta\ngamma\nbeta again"}))
            .unwrap();
        let read = workspace.call("read_file", json!({"path":"note.txt"})).unwrap();
        let hash = read["sha256"].as_str().unwrap().to_string();
        assert!(workspace
            .call("edit_file", json!({"path":"note.txt","expected_sha256":hash,"old_text":"beta","new_text":"BETA"}))
            .is_err());
        let edited = workspace
            .call("edit_file", json!({"path":"note.txt","expected_sha256":hash,"old_text":"alpha\nbeta\ngamma","new_text":"alpha\nBETA\ngamma"}))
            .unwrap();
        assert_eq!(edited["replacements"], 1);
        let diff = edited["diff"].as_str().unwrap();
        assert!(diff.contains("--- a/note.txt") && diff.contains("+++ b/note.txt"));
        assert!(diff.contains("-beta") && diff.contains("+BETA"));
        let reread = workspace.call("read_file", json!({"path":"note.txt"})).unwrap();
        assert!(reread["content"].as_str().unwrap().contains("BETA"));
        assert_eq!(reread["sha256"], edited["sha256"]);
        // The stale hash no longer matches the changed file.
        assert!(workspace
            .call("edit_file", json!({"path":"note.txt","expected_sha256":hash,"old_text":"gamma","new_text":"GAMMA"}))
            .is_err());
        assert!(workspace
            .call("edit_file", json!({"path":"missing.txt","expected_sha256":hash,"old_text":"a","new_text":"b"}))
            .is_err());
        assert!(workspace
            .call("edit_file", json!({"path":"note.txt","expected_sha256":reread["sha256"],"old_text":"absent","new_text":"b"}))
            .is_err());
        assert!(workspace
            .call("edit_file", json!({"path":"note.txt","expected_sha256":"zz","old_text":"gamma","new_text":"GAMMA"}))
            .is_err());
        assert_eq!(
            std::fs::read_to_string(temp.path().join("note.txt")).unwrap(),
            "alpha\nBETA\ngamma\nbeta again"
        );
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
