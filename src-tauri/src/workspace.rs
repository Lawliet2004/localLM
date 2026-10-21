use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
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

#[path = "workspace_documents.rs"]
mod documents;
#[path = "workspace_notebook.rs"]
mod notebook;
#[path = "workspace_patch.rs"]
pub(crate) mod patch;
#[path = "workspace_search.rs"]
mod search;

pub const TOOL_COUNT: usize = 28;

pub struct Workspace {
    directory: Dir,
    root: std::path::PathBuf,
    transaction_lock: Arc<std::sync::RwLock<()>>,
}

/// Mutations and recovery take the exclusive root guard; pure reads share it.
pub(crate) fn is_write_call(name: &str) -> bool {
    matches!(
        name,
        "apply_patch"
            | "copy_file"
            | "move_file"
            | "delete_file"
            | "recover_patch"
            | "create_file"
            | "create_directory"
            | "edit_file"
            | "write_file"
            | "write_file_base64"
            | "append_file"
            | "multi_edit"
            | "edit_lines"
            | "delete_directory"
            | "move_directory"
            | "copy_directory"
            | "notebook_edit"
    )
}

/// Open each parent separately without following links. Holding the parent
/// handles and refusing leaf links also protects sensitive paths from aliases
/// and directory/symlink swaps between metadata checks and the actual open.
pub(crate) fn parent_no_links(directory: &Dir, path: &str) -> std::io::Result<(Dir, String)> {
    let mut parent = directory.try_clone()?;
    let parts: Vec<_> = path.split('/').collect();
    if parts.is_empty()
        || parts.iter().any(|part| {
            part.is_empty() || matches!(*part, "." | "..") || part.contains(['\\', ':'])
        })
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Use a relative path without links or traversal.",
        ));
    }
    for part in &parts[..parts.len() - 1] {
        parent = parent.open_dir_nofollow(part)?;
    }
    Ok((parent, parts[parts.len() - 1].to_string()))
}

pub(crate) fn open_file_no_links(
    directory: &Dir,
    path: &str,
    options: &mut OpenOptions,
) -> std::io::Result<cap_std::fs::File> {
    let (parent, name) = parent_no_links(directory, path)?;
    options.follow(FollowSymlinks::No);
    #[cfg(unix)]
    {
        use cap_std::fs::OpenOptionsExt;
        options.custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32);
    }
    let file = parent.open_with(name, options)?;
    if !file.metadata()?.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Only regular workspace files are supported.",
        ));
    }
    Ok(file)
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
    GENERATED_DIRS
        .iter()
        .any(|dir| name.eq_ignore_ascii_case(dir))
}

struct SearchLimits {
    scan: usize,
    results: usize,
    depth: usize,
}

/// Depth-first walk collecting files whose name or `/`-joined relative path
/// contains `needle` (already lowercased). Links are never entered or
/// returned. A bound stops the walk and names which limit was hit.
#[allow(clippy::too_many_arguments)] // Recursive traversal shares one bounded search state.
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
                .open_dir_nofollow(&file_name)
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
    /// Replace every occurrence instead of requiring a unique match.
    #[serde(default)]
    replace_all: bool,
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
/// Full-file diff for a reviewed language-server proposal. Unlike display-only
/// workspace previews this must retain every changed byte, including line ends.
pub(crate) fn language_diff(path: &str, before: &str, after: &str) -> String {
    let old = before.split_inclusive('\n').count();
    let new = after.split_inclusive('\n').count();
    let mut diff = format!(
        "--- a/{path}\n+++ b/{path}\n@@ -{},{} +{},{} @@\n",
        usize::from(old > 0),
        old,
        usize::from(new > 0),
        new
    );
    for (prefix, text) in [('-', before), ('+', after)] {
        for line in text.split_inclusive('\n') {
            diff.push(prefix);
            diff.push_str(line);
            if !line.ends_with('\n') {
                diff.push_str("\n\\ No newline at end of file\n");
            }
        }
    }
    diff
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
        let mut boundary = 16_384;
        while !diff.is_char_boundary(boundary) {
            boundary -= 1;
        }
        diff.truncate(boundary);
        diff.push_str("\n…diff truncated…\n");
    }
    diff
}
impl Workspace {
    /// Static preview reads share the transaction lock and the same scoped,
    /// no-follow opens as model file reads. Never publish arbitrary documents,
    /// dotfiles, directory listings, devices or unbounded file contents.
    pub(crate) fn preview_asset(&self, path: &str) -> Result<(Vec<u8>, &'static str), String> {
        valid_path(path, false)?;
        if path.split('/').any(|part| part.starts_with('.')) {
            return Err("Hidden preview resources are not served.".into());
        }
        let extension = Path::new(path).extension().and_then(|v| v.to_str()).unwrap_or("").to_ascii_lowercase();
        let mime = match extension.as_str() {
            "html" | "htm" => "text/html; charset=utf-8",
            "css" => "text/css; charset=utf-8",
            "js" | "mjs" => "text/javascript; charset=utf-8",
            "json" => "application/json",
            "png" => "image/png", "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif", "webp" => "image/webp", "svg" => "image/svg+xml",
            "ico" => "image/x-icon", "woff" => "font/woff", "woff2" => "font/woff2",
            "ttf" => "font/ttf", "wasm" => "application/wasm",
            "mp3" => "audio/mpeg", "ogg" => "audio/ogg", "wav" => "audio/wav",
            _ => return Err("Only static web assets can be previewed.".into()),
        };
        let _guard = self.transaction_lock.read().map_err(|_| "Workspace transaction lock unavailable.")?;
        let file = open_file_no_links(&self.directory, path, OpenOptions::new().read(true)).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        file.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        if bytes.len() > 4 * 1024 * 1024 { return Err("Preview resource exceeds 4 MiB.".into()); }
        Ok((bytes, mime))
    }
    /// Raw UTF-8 source for the configured language server, using the same
    /// capability path, credential, recovery and opened-handle link policy.
    pub(crate) fn language_source(&self, path: &str) -> Result<String, String> {
        let _guard = self
            .transaction_lock
            .read()
            .map_err(|_| "Workspace transaction lock unavailable.")?;
        patch::ensure_clear(&self.directory)?;
        valid_path(path, false)?;
        let file = open_file_no_links(&self.directory, path, OpenOptions::new().read(true))
            .map_err(|e| e.to_string())?;
        patch::file_identity::single_link_identity(&file)?;
        let mut bytes = Vec::new();
        file.take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 1024 * 1024 || bytes.contains(&0) {
            return Err(
                "Language server documents must be UTF-8 text without NUL and at most1MiB.".into(),
            );
        }
        String::from_utf8(bytes).map_err(|_| "Language server document must be UTF-8.".into())
    }
    /// Online backup preserves WAL consistency. Pin/check native identities and
    /// refuse linked main/sidecar files; external writers still share SQLite locks.
    pub(crate) fn sqlite_backup(&self, path: &str, target: &Path) -> Result<(), String> {
        let _guard = self
            .transaction_lock
            .read()
            .map_err(|_| "Workspace transaction lock unavailable.")?;
        patch::ensure_clear(&self.directory)?;
        valid_path(path, false)?;
        let main = open_file_no_links(&self.directory, path, OpenOptions::new().read(true))
            .map_err(|e| e.to_string())?;
        let identity = patch::file_identity::single_link_identity(&main)?;
        for suffix in ["-wal", "-shm"] {
            let side = format!("{path}{suffix}");
            match self.directory.symlink_metadata(&side) {
                Ok(_) => {
                    let file =
                        open_file_no_links(&self.directory, &side, OpenOptions::new().read(true))
                            .map_err(|e| e.to_string())?;
                    patch::file_identity::single_link_identity(&file)?;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.to_string()),
            }
        }
        let source = rusqlite::Connection::open_with_flags(
            self.root.join(path),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| e.to_string())?;
        source
            .busy_timeout(std::time::Duration::from_millis(250))
            .map_err(|e| e.to_string())?;
        let size: i64 = source
            .query_row(
                "SELECT page_count*page_size FROM pragma_page_count(),pragma_page_size()",
                [],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        if size > 32 * 1024 * 1024 {
            return Err("SQLite backup exceeds 32 MiB.".into());
        }
        let mut destination = rusqlite::Connection::open(target).map_err(|e| e.to_string())?;
        {
            let backup = rusqlite::backup::Backup::new(&source, &mut destination)
                .map_err(|e| e.to_string())?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                if std::time::Instant::now() > deadline {
                    return Err("SQLite online backup timed out under concurrent writes.".into());
                }
                match backup.step(64).map_err(|e| e.to_string())? {
                    rusqlite::backup::StepResult::Done => break,
                    rusqlite::backup::StepResult::More => (),
                    rusqlite::backup::StepResult::Busy | rusqlite::backup::StepResult::Locked => {
                        std::thread::sleep(std::time::Duration::from_millis(10))
                    }
                    _ => return Err("Unknown SQLite backup state.".into()),
                }
                if backup.progress().pagecount > 8192 * 16 {
                    return Err("SQLite backup grew beyond the page budget.".into());
                }
            }
        }
        destination
            .execute_batch("PRAGMA journal_mode=DELETE;")
            .map_err(|e| e.to_string())?;
        let check = open_file_no_links(&self.directory, path, OpenOptions::new().read(true))
            .map_err(|e| e.to_string())?;
        if patch::file_identity::single_link_identity(&check)? != identity {
            return Err("SQLite source identity changed during backup.".into());
        }
        Ok(())
    }
    pub(crate) fn language_optional_source(&self, path: &str) -> Result<Option<String>, String> {
        valid_path(path, false)?;
        match self.directory.symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
            Ok(_) => self.language_source(path).map(Some),
        }
    }
    /// Bounded binary snapshot using the shared no-follow, identity and recovery policy.
    pub(crate) fn development_bytes(&self, path: &str, max: usize) -> Result<Vec<u8>, String> {
        let _guard = self
            .transaction_lock
            .read()
            .map_err(|_| "Workspace transaction lock unavailable.")?;
        patch::ensure_clear(&self.directory)?;
        valid_path(path, false)?;
        let file = open_file_no_links(&self.directory, path, OpenOptions::new().read(true))
            .map_err(|e| e.to_string())?;
        patch::file_identity::single_link_identity(&file)?;
        let mut bytes = Vec::new();
        file.take(max as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > max {
            return Err("Development input exceeds the byte limit.".into());
        }
        Ok(bytes)
    }
    pub fn output_artifact(&self, path: &str) -> Result<(Vec<u8>, &'static str), String> {
        let _guard = self
            .transaction_lock
            .read()
            .map_err(|_| "Workspace transaction lock unavailable.")?;
        patch::ensure_clear(&self.directory)?;
        valid_path(path, false)?;
        let file = open_file_no_links(&self.directory, path, OpenOptions::new().read(true))
            .map_err(|e| e.to_string())?;
        if !file.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("Artifact must be a regular file.".into());
        }
        let mut bytes = Vec::new();
        file.take(2 * 1024 * 1024 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 2 * 1024 * 1024 {
            return Err("Artifact preview exceeds 2 MiB. Open the file in your workspace.".into());
        }
        let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png"
        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            "image/jpeg"
        } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
            "image/webp"
        } else if std::str::from_utf8(&bytes).is_ok() && !bytes.contains(&0) {
            "text/plain"
        } else {
            return Err("Preview supports PNG, JPEG, WebP or UTF-8 text. Open other artifacts in the workspace.".into());
        };
        Ok((bytes, mime))
    }
    /// Read executable source without line-number formatting or partial reads.
    pub fn read_script(&self, path: &str) -> Result<String, String> {
        let _guard = self
            .transaction_lock
            .read()
            .map_err(|_| "Workspace transaction lock unavailable.")?;
        patch::ensure_clear(&self.directory)?;
        valid_path(path, false)?;
        let file = open_file_no_links(&self.directory, path, OpenOptions::new().read(true))
            .map_err(|e| e.to_string())?;
        if !file.metadata().map_err(|e| e.to_string())?.is_file() {
            return Err("Script must be a regular file.".into());
        }
        let mut bytes = Vec::new();
        file.take(32769)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 32768 {
            return Err("Python file exceeds 32768 bytes.".into());
        }
        String::from_utf8(bytes).map_err(|_| "Python source must be UTF-8.".into())
    }
    pub fn open(path: &str) -> Result<Self, String> {
        if path.is_empty() || !Path::new(path).is_absolute() {
            return Err("Choose an absolute workspace directory.".into());
        }
        Ok(Self {
            directory: Dir::open_ambient_dir(path, ambient_authority())
                .map_err(|error| format!("Could not open workspace: {error}"))?,
            root: std::fs::canonicalize(path).map_err(|error| error.to_string())?,
            transaction_lock: patch::root_lock(Path::new(path))?,
        })
    }
    pub fn call(&self, name: &str, arguments: Value) -> Result<Value, String> {
        // Reads share the root guard; writes and recovery take it exclusively
        // so a transaction can never overlap a native reader.
        let _write_guard;
        let _read_guard;
        if is_write_call(name) {
            _write_guard = Some(
                self.transaction_lock
                    .write()
                    .map_err(|_| "Workspace transaction lock unavailable.")?,
            );
            _read_guard = None;
        } else {
            _read_guard = Some(
                self.transaction_lock
                    .read()
                    .map_err(|_| "Workspace transaction lock unavailable.")?,
            );
            _write_guard = None;
        }
        if name != "recover_patch" {
            patch::ensure_clear(&self.directory)?;
        }
        match name {
            "apply_patch" | "copy_file" | "move_file" | "delete_file" | "recover_patch"
            | "file_info" | "write_file" | "write_file_base64" | "append_file" | "multi_edit"
            | "edit_lines" | "list_tree" | "delete_directory" | "copy_directory"
            | "move_directory" => {
                let mut result = self.patch_call(name, arguments)?;
                // A written file is formatted in place before the result is
                // reported, so the model never has to guess whether the
                // project's own format check will pass.
                if matches!(
                    name,
                    "write_file"
                        | "write_file_base64"
                        | "append_file"
                        | "multi_edit"
                        | "edit_lines"
                ) {
                    self.format_changed_file(&mut result);
                }
                Ok(result)
            }
            "search_content" => self.search_content(arguments),
            "glob_files" => self.glob_files(arguments),
            "list_files" => {
                let request: PathRequest =
                    serde_json::from_value(arguments).map_err(|error| error.to_string())?;
                valid_path(&request.path, true)?;
                let directory = if request.path == "." {
                    self.directory.try_clone()
                } else {
                    parent_no_links(&self.directory, &request.path)
                        .and_then(|(parent, name)| parent.open_dir_nofollow(name))
                }
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
                let file = open_file_no_links(
                    &self.directory,
                    &request.path,
                    OpenOptions::new().read(true),
                )
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
                let mut file = open_file_no_links(
                    &self.directory,
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
                // The formatter rewrites the file in place, so the handle is
                // released first.
                drop(file);
                let path = request.path.clone();
                let pre_format_sha256 = format!("{:x}", Sha256::digest(request.content.as_bytes()));
                let mut result = json!({"path":path,"bytesWritten":request.content.len(),"sha256":pre_format_sha256.clone()});
                self.apply_auto_format(&mut result, &path, &pre_format_sha256);
                Ok(result)
            }
            "create_directory" => {
                let request: PathRequest =
                    serde_json::from_value(arguments).map_err(|error| error.to_string())?;
                valid_path(&request.path, false)?;
                let (parent, name) = parent_no_links(&self.directory, &request.path)
                    .map_err(|error| error.to_string())?;
                parent.create_dir(name).map_err(|error| error.to_string())?;
                Ok(json!({"path":request.path,"created":true}))
            }
            "edit_file" => {
                let request: EditRequest =
                    serde_json::from_value(arguments).map_err(|error| error.to_string())?;
                valid_path(&request.path, false)?;
                if request.old_text.len() > 65_536 {
                    return Err("The text to replace must contain at most 65536 bytes.".into());
                }
                if request.new_text.len() > 65_536 {
                    return Err("Replacement text exceeds 64 KiB.".into());
                }
                let expected = parse_hash(&request.expected_sha256)?;
                let file = open_file_no_links(
                    &self.directory,
                    &request.path,
                    OpenOptions::new().read(true).write(true),
                )
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
                    return Err(
                        "This file changed after it was read. Read it again and retry the edit."
                            .into(),
                    );
                }
                let text = String::from_utf8(bytes).map_err(|_| "File is not UTF-8 text.")?;
                // The shared matcher tolerates smart quotes, dash variants,
                // exotic spaces, trailing whitespace and CRLF, and keeps the
                // line endings of the region it replaces.
                let (updated, replacements, kind) = crate::edit_match::replace(
                    &text,
                    &request.old_text,
                    &request.new_text,
                    request.replace_all,
                )?;
                {
                    use std::io::{Seek, SeekFrom};
                    let mut writer = file;
                    writer
                        .seek(SeekFrom::Start(0))
                        .map_err(|error| error.to_string())?;
                    writer.set_len(0).map_err(|error| error.to_string())?;
                    writer
                        .write_all(updated.as_bytes())
                        .and_then(|_| writer.sync_all())
                        .map_err(|error| {
                            format!("File write failed; the file may be incomplete: {error}")
                        })?;
                }
                let pre_format_sha256 = format!("{:x}", Sha256::digest(updated.as_bytes()));
                let path = request.path.clone();
                let mut result = json!({"path":path,"replacements":replacements,"bytesWritten":updated.len(),"sha256":pre_format_sha256.clone(),"diff":unified_diff(&path,&text,&updated),"match":kind.as_str()});
                self.apply_auto_format(&mut result, &path, &pre_format_sha256);
                Ok(result)
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
                        "Use a search query of 1–200 characters without control characters.".into(),
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
                Ok(
                    json!({"results":results,"truncated":truncation != Truncation::None,"truncation":truncation.as_str()}),
                )
            }
            "notebook_read" | "notebook_edit" => self.notebook_call(name, arguments),
            "document_inspect" | "document_read" | "view_image" => {
                documents::call(&self.directory, name, arguments)
            }
            _ => Err("Unknown workspace tool.".into()),
        }
    }
    /// Run the project formatter for one written file.
    ///
    /// Formatting never fails the write: the bytes are already on disk. A
    /// formatter that fails is reported in the tool result, not raised, so a
    /// broken toolchain cannot lose a successful write.
    fn auto_format(&self, relative: &str) -> Option<crate::formatter::FormatOutcome> {
        match crate::formatter::format_file_detailed(
            &self.root,
            relative,
            crate::formatter::AUTO_TIMEOUT_SECONDS,
        ) {
            Ok(None) => None,
            Ok(Some(detailed)) => Some(detailed.outcome),
            Err(failure) => {
                let (formatter, detail) = match &failure {
                    crate::formatter::FormatFailure::FormatterFailed { formatter, detail } => {
                        (Some(formatter.clone()), detail.clone())
                    }
                    _ => (None, failure.message()),
                };
                Some(crate::formatter::FormatOutcome {
                    path: relative.to_string(),
                    formatter,
                    changed: false,
                    detail: format!("Formatting could not run: {detail}"),
                })
            }
        }
    }
    /// The current SHA-256 of a workspace file, read through the same
    /// no-follow path the read tool uses.
    fn current_sha256(&self, relative: &str) -> Option<String> {
        let mut options = OpenOptions::new();
        options.read(true);
        let file = open_file_no_links(&self.directory, relative, &mut options).ok()?;
        let mut bytes = Vec::new();
        file.take(1_048_577).read_to_end(&mut bytes).ok()?;
        if bytes.len() > 1_048_576 {
            return None;
        }
        Some(format!("{:x}", Sha256::digest(&bytes)))
    }
    /// Attach the formatting outcome to a write result and refresh its hash.
    ///
    /// The reported `sha256` is the hash of the file AFTER formatting. A
    /// formatter rewrites the file in place, so the optimistic-concurrency pin
    /// a model takes from this result has to match the bytes on disk; a
    /// pre-format hash would make the next edit fail its pin check even
    /// though nothing else touched the file. The pre-format hash is kept as
    /// `preFormatSha256` so the write itself stays auditable, and
    /// `reformatted` says whether the formatter changed anything.
    fn apply_auto_format(&self, result: &mut Value, path: &str, pre_format_sha256: &str) {
        let outcome = self.auto_format(path);
        let changed = outcome.as_ref().map(|outcome| outcome.changed).unwrap_or(false);
        let post_format_sha256 = self
            .current_sha256(path)
            .unwrap_or_else(|| pre_format_sha256.to_string());
        let Some(object) = result.as_object_mut() else {
            return;
        };
        object.insert("sha256".to_string(), json!(post_format_sha256.clone()));
        if changed {
            object.insert("preFormatSha256".to_string(), json!(pre_format_sha256));
        }
        object.insert("reformatted".to_string(), json!(changed));
        object.insert(
            "formatted".to_string(),
            match &outcome {
                Some(outcome) => json!({
                    "path": outcome.path,
                    "formatter": outcome.formatter.clone(),
                    "changed": outcome.changed,
                    "detail": outcome.detail,
                }),
                None => Value::Null,
            },
        );
        // Keep the per-change hash in step with the bytes on disk.
        if let Some(changes) = object.get_mut("changes").and_then(|value| value.as_array_mut()) {
            if let Some(change) = changes.first_mut().and_then(|change| change.as_object_mut()) {
                if change.get("path").and_then(|value| value.as_str()) == Some(path) {
                    change.insert("sha256".to_string(), json!(post_format_sha256));
                    if changed {
                        change.insert("preFormatSha256".to_string(), json!(pre_format_sha256));
                    }
                }
            }
        }
    }
    /// Format the single file a journalled write changed.
    fn format_changed_file(&self, result: &mut Value) {
        let Some(change) = result
            .get("changes")
            .and_then(|changes| changes.as_array())
            .and_then(|changes| changes.first())
        else {
            return;
        };
        let Some(path) = change.get("path").and_then(|path| path.as_str()) else {
            return;
        };
        if change.get("kind").and_then(|kind| kind.as_str()) == Some("delete") {
            return;
        }
        let Some(pre_format_sha256) = change.get("sha256").and_then(|sha| sha.as_str()) else {
            return;
        };
        let (path, pre_format_sha256) = (path.to_string(), pre_format_sha256.to_string());
        self.apply_auto_format(result, &path, &pre_format_sha256);
    }
    pub fn tools(self: Arc<Self>) -> Vec<crate::connectors::AgentTool> {
        let mut definitions=vec![
            ("list_files","List up to 500 entries in a workspace directory. Use path . for the root.",json!({"path":{"type":"string"}}),vec!["path"]),
            ("read_file","Read UTF-8 workspace text with line numbers and SHA-256. Maximum file size 1 MiB.",json!({"path":{"type":"string"},"start_line":{"type":"integer","minimum":1},"line_count":{"type":"integer","minimum":1,"maximum":500}}),vec!["path"]),
            ("create_file","Create a new UTF-8 file inside the workspace. Existing files are never overwritten. Parent directory must exist.",json!({"path":{"type":"string"},"content":{"type":"string"}}),vec!["path","content"]),
            ("create_directory","Create one directory inside the workspace. Parent directory must exist.",json!({"path":{"type":"string"}}),vec!["path"]),
            ("edit_file","Replace one unique text match in an existing UTF-8 file. Requires the SHA-256 from a fresh read_file; fails when the file changed or the match is missing, and fails on an ambiguous match unless replace_all is set. Smart quotes, dash variants, exotic spaces, trailing whitespace and CRLF are tolerated when matching, and the line endings of the replaced region are preserved. Returns a unified diff of the change for review. Maximum file size 1 MiB.",json!({"path":{"type":"string"},"expected_sha256":{"type":"string"},"old_text":{"type":"string"},"new_text":{"type":"string"},"replace_all":{"type":"boolean","description":"Replace every occurrence of old_text instead of requiring a unique match. Default false."}}),vec!["path","expected_sha256","old_text","new_text"]),
        ];
        definitions.extend(search::definitions());
        definitions.extend(patch::definitions());
        definitions.extend(documents::definitions());
        definitions.extend(notebook::definitions());
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
    #[cfg(unix)]
    #[test]
    fn sensitive_files_cannot_be_accessed_through_internal_link_aliases() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join(".ssh")).unwrap();
        std::fs::write(root.path().join(".ssh/key.txt"), "private fixture").unwrap();
        std::fs::write(root.path().join("secret.pem"), "private fixture").unwrap();
        symlink("secret.pem", root.path().join("innocent.txt")).unwrap();
        symlink(".ssh", root.path().join("innocent-dir")).unwrap();
        let workspace = Workspace::open(root.path().to_str().unwrap()).unwrap();
        for path in ["innocent.txt", "innocent-dir/key.txt"] {
            assert!(workspace.call("read_file", json!({"path":path})).is_err());
            assert!(workspace.call("edit_file", json!({"path":path,"expected_sha256":"0".repeat(64),"old_text":"private","new_text":"changed"})).is_err());
        }
        assert!(workspace
            .call("list_files", json!({"path":"innocent-dir"}))
            .is_err());
        assert!(workspace
            .call(
                "create_file",
                json!({"path":"innocent-dir/new.txt","content":"x"})
            )
            .is_err());
        assert!(workspace
            .call("create_directory", json!({"path":"innocent-dir/new"}))
            .is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("secret.pem")).unwrap(),
            "private fixture"
        );
    }
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
            .call(
                "create_file",
                json!({"path":"note.txt","content":"alpha\nbeta\ngamma\nbeta again"}),
            )
            .unwrap();
        let read = workspace
            .call("read_file", json!({"path":"note.txt"}))
            .unwrap();
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
        let reread = workspace
            .call("read_file", json!({"path":"note.txt"}))
            .unwrap();
        assert!(reread["content"].as_str().unwrap().contains("BETA"));
        assert_eq!(reread["sha256"], edited["sha256"]);
        // The stale hash no longer matches the changed file.
        assert!(workspace
            .call("edit_file", json!({"path":"note.txt","expected_sha256":hash,"old_text":"gamma","new_text":"GAMMA"}))
            .is_err());
        assert!(workspace
            .call(
                "edit_file",
                json!({"path":"missing.txt","expected_sha256":hash,"old_text":"a","new_text":"b"})
            )
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
    fn edits_tolerate_typographic_differences_and_can_replace_every_occurrence() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = Workspace::open(temp.path().to_str().unwrap()).unwrap();
        workspace
            .call(
                "create_file",
                json!({"path":"quote.txt","content":"let s = \u{201C}hi\u{201D};\n"}),
            )
            .unwrap();
        let read = workspace
            .call("read_file", json!({"path":"quote.txt"}))
            .unwrap();
        let hash = read["sha256"].as_str().unwrap().to_string();
        let edited = workspace
            .call(
                "edit_file",
                json!({"path":"quote.txt","expected_sha256":hash,"old_text":"\"hi\"","new_text":"\"bye\""}),
            )
            .unwrap();
        assert_eq!(edited["match"], "normalized");
        assert_eq!(edited["replacements"], 1);
        assert_eq!(
            std::fs::read_to_string(temp.path().join("quote.txt")).unwrap(),
            "let s = \"bye\";\n"
        );
        // An ambiguous match still needs replace_all.
        workspace
            .call("create_file", json!({"path":"many.txt","content":"x x x"}))
            .unwrap();
        let read = workspace
            .call("read_file", json!({"path":"many.txt"}))
            .unwrap();
        let hash = read["sha256"].as_str().unwrap().to_string();
        assert!(workspace
            .call(
                "edit_file",
                json!({"path":"many.txt","expected_sha256":hash,"old_text":"x","new_text":"y"})
            )
            .is_err());
        let edited = workspace
            .call(
                "edit_file",
                json!({"path":"many.txt","expected_sha256":hash,"old_text":"x","new_text":"y","replace_all":true}),
            )
            .unwrap();
        assert_eq!(edited["match"], "exact");
        assert_eq!(edited["replacements"], 3);
        assert_eq!(
            std::fs::read_to_string(temp.path().join("many.txt")).unwrap(),
            "y y y"
        );
    }
    #[test]
    fn edits_preserve_crlf_line_endings() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = Workspace::open(temp.path().to_str().unwrap()).unwrap();
        std::fs::write(temp.path().join("crlf.txt"), "one\r\ntwo\r\n").unwrap();
        let read = workspace
            .call("read_file", json!({"path":"crlf.txt"}))
            .unwrap();
        let hash = read["sha256"].as_str().unwrap().to_string();
        let edited = workspace
            .call(
                "edit_file",
                json!({"path":"crlf.txt","expected_sha256":hash,"old_text":"one\ntwo","new_text":"one\nthree"}),
            )
            .unwrap();
        assert_eq!(edited["match"], "normalized");
        assert_eq!(edited["replacements"], 1);
        assert_eq!(
            std::fs::read_to_string(temp.path().join("crlf.txt")).unwrap(),
            "one\r\nthree\r\n"
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
        let empty = workspace
            .call("search_files", json!({"query":"   "}))
            .unwrap();
        assert_eq!(empty["results"].as_array().unwrap().len(), 0);
        assert_eq!(empty["truncated"], false);
        // Oversized, control-character, and malformed queries are rejected.
        assert!(workspace
            .call("search_files", json!({"query":"x".repeat(201)}))
            .is_err());
        assert!(workspace
            .call("search_files", json!({"query":"a\u{0007}b"}))
            .is_err());
        assert!(workspace.call("search_files", json!({"wrong":1})).is_err());
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
        assert_eq!(
            found["results"].as_array().unwrap().len(),
            SEARCH_MAX_RESULTS
        );
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
        let hidden = workspace
            .call("search_files", json!({"query": "hidden"}))
            .unwrap();
        assert_eq!(hidden["results"].as_array().unwrap().len(), 0);
        assert_eq!(hidden["truncated"], false);
        let kept = workspace
            .call("search_files", json!({"query": "kept"}))
            .unwrap();
        assert_eq!(kept["results"][0]["path"], "src/kept.rs");
        let included = workspace
            .call(
                "search_files",
                json!({"query": "hidden", "include_generated": true}),
            )
            .unwrap();
        assert_eq!(included["results"][0]["path"], "node_modules/pkg/hidden.rs");

        // A file past the depth cap is not reported as an authoritative miss.
        let mut deep = temp.path().to_path_buf();
        for index in 0..=SEARCH_MAX_DEPTH + 1 {
            deep.push(format!("d{index}"));
        }
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("needle.txt"), "x").unwrap();
        let depth = workspace
            .call("search_files", json!({"query": "needle"}))
            .unwrap();
        assert_eq!(depth["truncated"], true);
        assert_eq!(depth["truncation"], "depth");
        assert!(depth["results"].as_array().unwrap().is_empty());

        let mut scanned = 0usize;
        let mut results = Vec::new();
        let dir =
            cap_std::fs::Dir::open_ambient_dir(temp.path(), cap_std::ambient_authority()).unwrap();
        let scan = search_in(
            &dir,
            "",
            "kept",
            0,
            &mut scanned,
            &mut results,
            false,
            &SearchLimits {
                scan: 1,
                results: 50,
                depth: 8,
            },
        )
        .unwrap();
        assert_eq!(scan, Truncation::Scan);
        assert!(results.is_empty() || scan == Truncation::Scan);
    }
    #[test]
    fn writes_report_the_format_outcome_and_the_hash_of_the_file_on_disk() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = Workspace::open(temp.path().to_str().unwrap()).unwrap();
        let result = workspace
            .call(
                "write_file",
                json!({"path":"main.rs","content":"fn  main( ) {let x=1;}\n"}),
            )
            .unwrap();
        // The write succeeded and the reported hash is the file's current
        // hash: a formatter rewrites the file in place, so the pin the model
        // takes from this result has to match the bytes on disk.
        let on_disk = std::fs::read(temp.path().join("main.rs")).unwrap();
        assert_eq!(result["sha256"], format!("{:x}", Sha256::digest(&on_disk)));
        assert_eq!(result["changes"][0]["sha256"], result["sha256"]);
        // rustfmt may not be installed: the outcome is then absent, and the
        // write still succeeds.
        let formatted = &result["formatted"];
        if crate::formatter::program_available("rustfmt") {
            assert_eq!(formatted["formatter"], "rustfmt");
            assert!(formatted["changed"].is_boolean());
        } else {
            assert!(formatted.is_null(), "{formatted}");
        }
        assert!(result["reformatted"].is_boolean());
    }
    #[test]
    fn a_file_without_a_formatter_reports_no_outcome() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = Workspace::open(temp.path().to_str().unwrap()).unwrap();
        let result = workspace
            .call(
                "write_file",
                json!({"path":"notes.txt","content":"hello\n"}),
            )
            .unwrap();
        assert!(result["formatted"].is_null());
        assert_eq!(result["reformatted"], false);
        let on_disk = std::fs::read(temp.path().join("notes.txt")).unwrap();
        assert_eq!(result["sha256"], format!("{:x}", Sha256::digest(&on_disk)));
    }
    #[test]
    fn edits_report_the_format_outcome_and_the_hash_of_the_file_on_disk() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = Workspace::open(temp.path().to_str().unwrap()).unwrap();
        workspace
            .call(
                "create_file",
                json!({"path":"lib.rs","content":"pub fn a() {}\n"}),
            )
            .unwrap();
        let read = workspace
            .call("read_file", json!({"path":"lib.rs"}))
            .unwrap();
        let hash = read["sha256"].as_str().unwrap().to_string();
        let edited = workspace
            .call(
                "edit_file",
                json!({"path":"lib.rs","expected_sha256":hash,"old_text":"a()","new_text":"b()"}),
            )
            .unwrap();
        let on_disk = std::fs::read(temp.path().join("lib.rs")).unwrap();
        assert_eq!(edited["sha256"], format!("{:x}", Sha256::digest(&on_disk)));
        assert!(edited["formatted"].is_null() || edited["formatted"]["formatter"].is_string());
        // The next edit can use the hash the previous edit reported.
        let second = workspace
            .call(
                "edit_file",
                json!({"path":"lib.rs","expected_sha256":edited["sha256"],"old_text":"b()","new_text":"c()"}),
            )
            .unwrap();
        assert_eq!(second["replacements"], 1);
    }
}
