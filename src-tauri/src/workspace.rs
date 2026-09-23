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
struct SearchRequest {
    query: String,
    #[serde(default)]
    include_generated: bool,
}
// ponytail: naive recursive DFS — no .gitignore, no glob engine, no
// parallelism — bounded by the caps below. Upgrade to the `ignore` crate
// if searches over huge trees get slow.
const SEARCH_MAX_SCANNED: usize = 50_000;
const SEARCH_MAX_RESULTS: usize = 200;
const SEARCH_MAX_DEPTH: usize = 32;
const GENERATED_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "dist",
    ".git",
    "__pycache__",
    ".next",
    "build",
    "coverage",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Truncation {
    None,
    Result,
    Scan,
    Depth,
}

impl Truncation {
    fn as_str(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Result => Some("result"),
            Self::Scan => Some("scan"),
            Self::Depth => Some("depth"),
        }
    }
}

fn is_generated_dir(name: &str) -> bool {
    GENERATED_DIRS.iter().any(|dir| name.eq_ignore_ascii_case(dir))
}

struct SearchLimits {
    scan: usize,
    results: usize,
    depth: usize,
}

/// Depth-first walk collecting files whose name or `/`-joined relative path
/// contains `needle` (already lowercased). Links are never entered or
/// returned. A bound stops the walk and names which limit was hit.
fn search_in(
    directory: &Dir,
    prefix: &str,
    needle: &str,
    depth: usize,
    scanned: &mut usize,
    results: &mut Vec<String>,
    include_generated: bool,
    limits: &SearchLimits,
) -> Result<Truncation, String> {
    for entry in directory.entries().map_err(|error| error.to_string())? {
        if results.len() >= limits.results {
            return Ok(Truncation::Result);
        }
        if *scanned >= limits.scan {
            return Ok(Truncation::Scan);
        }
        let entry = entry.map_err(|error| error.to_string())?;
        *scanned += 1;
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        if kind.is_symlink() {
            continue;
        }
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        let path = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}/{name}")
        };
        if kind.is_dir() {
            if !include_generated && is_generated_dir(&name) {
                continue;
            }
            if depth >= limits.depth {
                return Ok(Truncation::Depth);
            }
            let child = directory
                .open_dir(&file_name)
                .map_err(|error| error.to_string())?;
            let child_limit = search_in(
                &child,
                &path,
                needle,
                depth + 1,
                scanned,
                results,
                include_generated,
                limits,
            )?;
            if child_limit != Truncation::None {
                return Ok(child_limit);
            }
        } else if path.to_lowercase().contains(needle) {
            results.push(path);
        }
    }
    Ok(Truncation::None)
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
fn valid_path(path: &str, root_allowed: bool) -> Result<(), String> {
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
            "search_files" => {
                let request: SearchRequest =
                    serde_json::from_value(arguments).map_err(|error| error.to_string())?;
                let needle = request.query.trim();
                if needle.is_empty() {
                    return Ok(json!({"results":[],"truncated":false,"truncation":null}));
                }
                if needle.chars().count() > 200 || needle.chars().any(char::is_control) {
                    return Err(
                        "Use a search query of 1–200 characters without control characters."
                            .into(),
                    );
                }
                let needle = needle.to_lowercase();
                let mut results = Vec::new();
                let mut scanned = 0usize;
                let limits = SearchLimits {
                    scan: SEARCH_MAX_SCANNED,
                    results: SEARCH_MAX_RESULTS,
                    depth: SEARCH_MAX_DEPTH,
                };
                let truncation = search_in(
                    &self.directory,
                    "",
                    &needle,
                    0,
                    &mut scanned,
                    &mut results,
                    request.include_generated,
                    &limits,
                )?;
                results.sort();
                let results = results
                    .into_iter()
                    .map(|path| json!({"path":path,"kind":"file"}))
                    .collect::<Vec<_>>();
                Ok(json!({"results":results,"truncated":truncation != Truncation::None,"truncation":truncation.as_str()}))
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
    #[test]
    fn search_finds_nested_files_by_name_or_path_and_validates_the_query() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("deep/nested/dir")).unwrap();
        std::fs::write(temp.path().join("deep/nested/dir/code.rs"), "x").unwrap();
        std::fs::write(temp.path().join("top.txt"), "x").unwrap();
        let workspace = Workspace::open(temp.path().to_str().unwrap()).unwrap();
        // Case-insensitive name match inside a directory that was never listed.
        let found = workspace
            .call("search_files", json!({"query":"CODE"}))
            .unwrap();
        assert_eq!(
            found["results"].as_array().unwrap(),
            &vec![json!({"path":"deep/nested/dir/code.rs","kind":"file"})]
        );
        assert_eq!(found["truncated"], false);
        // A path fragment surfaces the file even when its name does not match.
        let by_path = workspace
            .call("search_files", json!({"query":"nested/dir"}))
            .unwrap();
        assert_eq!(by_path["results"][0]["path"], "deep/nested/dir/code.rs");
        // Directories are never results themselves; their files surface via
        // the path match.
        let dir_only = workspace
            .call("search_files", json!({"query":"deep"}))
            .unwrap();
        assert_eq!(
            dir_only["results"].as_array().unwrap(),
            &vec![json!({"path":"deep/nested/dir/code.rs","kind":"file"})]
        );
        // Blank queries return an empty result rather than an error.
        let empty = workspace.call("search_files", json!({"query":"   "})).unwrap();
        assert_eq!(empty["results"].as_array().unwrap().len(), 0);
        assert_eq!(empty["truncated"], false);
        // Oversized, control-character, and malformed queries are rejected.
        assert!(workspace
            .call("search_files", json!({"query":"x".repeat(201)}))
            .is_err());
        assert!(workspace
            .call("search_files", json!({"query":"a\u{0007}b"}))
            .is_err());
        assert!(workspace
            .call("search_files", json!({"wrong":1}))
            .is_err());
    }
    #[test]
    fn search_marks_results_beyond_the_cap_as_truncated() {
        let temp = tempfile::tempdir().unwrap();
        for index in 0..SEARCH_MAX_RESULTS + 1 {
            std::fs::write(temp.path().join(format!("match{index:04}.txt")), "x").unwrap();
        }
        let workspace = Workspace::open(temp.path().to_str().unwrap()).unwrap();
        let found = workspace
            .call("search_files", json!({"query":"match"}))
            .unwrap();
        assert_eq!(found["results"].as_array().unwrap().len(), SEARCH_MAX_RESULTS);
        assert_eq!(found["truncated"], true);
        assert_eq!(found["truncation"], "result");
        let paths: Vec<&str> = found["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|hit| hit["path"].as_str().unwrap())
            .collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
    }
    #[cfg(windows)]
    #[test]
    fn search_never_descends_into_link_directories() {
        use std::os::windows::process::CommandExt;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("hidden.txt"), "outside fixture").unwrap();
        let link = root.path().join("escape");
        let output=std::process::Command::new("powershell.exe").args(["-NoProfile","-NonInteractive","-Command","New-Item -ItemType Junction -Path $env:LOCALLM_TEST_LINK -Target $env:LOCALLM_TEST_TARGET -ErrorAction Stop | Out-Null"]).env("LOCALLM_TEST_LINK",&link).env("LOCALLM_TEST_TARGET",outside.path()).creation_flags(0x08000000).output().unwrap();
        assert!(
            output.status.success(),
            "Junction fixture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let workspace = Workspace::open(root.path().to_str().unwrap()).unwrap();
        // The junction's target lives outside the workspace; its files must
        // never appear, and the link itself is not a file result.
        for query in ["hidden", "escape", "e"] {
            let found = workspace
                .call("search_files", json!({"query":query}))
                .unwrap();
            assert_eq!(
                found["results"].as_array().unwrap().len(),
                0,
                "query {query} leaked a linked path"
            );
        }
    }

    #[test]
    fn search_skips_generated_directories_unless_asked_and_names_scan_and_depth() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("node_modules/pkg")).unwrap();
        std::fs::write(temp.path().join("node_modules/pkg/hidden.rs"), "x").unwrap();
        std::fs::create_dir_all(temp.path().join("src")).unwrap();
        std::fs::write(temp.path().join("src/kept.rs"), "x").unwrap();
        let workspace = Workspace::open(temp.path().to_str().unwrap()).unwrap();
        let hidden = workspace.call("search_files", json!({"query": "hidden"})).unwrap();
        assert_eq!(hidden["results"].as_array().unwrap().len(), 0);
        assert_eq!(hidden["truncated"], false);
        let kept = workspace.call("search_files", json!({"query": "kept"})).unwrap();
        assert_eq!(kept["results"][0]["path"], "src/kept.rs");
        let included = workspace
            .call("search_files", json!({"query": "hidden", "include_generated": true}))
            .unwrap();
        assert_eq!(included["results"][0]["path"], "node_modules/pkg/hidden.rs");

        // A file past the depth cap is not reported as an authoritative miss.
        let mut deep = temp.path().to_path_buf();
        for index in 0..=SEARCH_MAX_DEPTH + 1 {
            deep.push(format!("d{index}"));
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("needle.txt"), "x").unwrap();
        let depth = workspace.call("search_files", json!({"query": "needle"})).unwrap();
        assert_eq!(depth["truncated"], true);
        assert_eq!(depth["truncation"], "depth");
        assert!(depth["results"].as_array().unwrap().is_empty());

        let mut scanned = 0usize;
        let mut results = Vec::new();
        let dir = cap_std::fs::Dir::open_ambient_dir(temp.path(), cap_std::ambient_authority()).unwrap();
        let scan = search_in(
            &dir,
            "",
            "kept",
            0,
            &mut scanned,
            &mut results,
            false,
            &SearchLimits { scan: 1, results: 50, depth: 8 },
        )
        .unwrap();
        assert_eq!(scan, Truncation::Scan);
        assert!(results.is_empty() || scan == Truncation::Scan);
    }
}
