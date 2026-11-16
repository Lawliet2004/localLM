//! Workspace snapshots: capture the files an agent changed, and restore them.
//!
//! A snapshot is the localLM answer to "the agent broke my file". The agent
//! loop captures the workspace files it is about to touch before dispatch;
//! `/undo` stages a rollback of the latest
//! snapshot, committing it restores the captured bytes, and discarding it
//! leaves the workspace exactly as it was.
//!
//! Captured bytes never live in the workspace. They are stored in a private,
//! content-addressed blob directory next to the local database
//! (`<data dir>/snapshots/<sha[0..2]>/<sha256>`) and the SQLite rows hold
//! only metadata. A restore republishes a blob through a temporary file in
//! the target's own directory followed by a rename, so a crash cannot leave a
//! half-written file behind, and a symbolic link is never written through.

use cap_fs_ext::{DirExt, MetadataExt};
use cap_std::fs::{Dir, File, OpenOptions, Permissions};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

/// Largest file captured into one snapshot (2 MiB), matching the untracked
/// file cap used by OpenCode's snapshot store. Bigger files are skipped and
/// reported, never truncated.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// Largest number of files captured into one snapshot.
pub const MAX_FILES: usize = 512;
/// Largest number of paths a single capture request may name.
const MAX_REQUESTED_PATHS: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct WorkspacePin {
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl WorkspacePin {
    pub(crate) fn open(path: &Path) -> Result<(Self, Dir), String> {
        let path = std::fs::canonicalize(path).map_err(|error| format!("Cannot open the workspace: {error}"))?;
        let directory = Dir::open_ambient_dir(&path, cap_std::ambient_authority())
            .map_err(|error| format!("Cannot open the workspace: {error}"))?;
        let metadata = directory.dir_metadata().map_err(|error| error.to_string())?;
        Ok((Self { path, device: metadata.dev(), inode: metadata.ino() }, directory))
    }

    pub(crate) fn verify(&self, root: &Path) -> Result<Dir, String> {
        let (current, dir) = Self::open(root)?;
        if current != *self {
            return Err("The workspace changed since this state was recorded.".into());
        }
        Ok(dir)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct TargetVersion {
    device: u64,
    inode: u64,
    sha256: String,
}

impl TargetVersion {
    fn read(file: &File) -> Result<Self, String> {
        let metadata = file.metadata().map_err(|error| error.to_string())?;
        let sha256 = current_digest(file)?;
        if sha256.is_empty() {
            return Err("The current file is too large to review for undo.".into());
        }
        Ok(Self { device: metadata.dev(), inode: metadata.ino(), sha256 })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
enum ReviewedTarget {
    Unavailable,
    Absent,
    Present(TargetVersion),
}

/// One captured file.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotFile {
    /// Workspace-relative path.
    pub path: String,
    /// sha256 of the captured content. Empty when the file was absent.
    pub sha256: String,
    /// Byte length of the captured content.
    pub size: u64,
    /// True when the file did not exist at capture time. A restore deletes a
    /// file the snapshot recorded as absent.
    pub deleted: bool,
}

/// A snapshot of the workspace at one point in time.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// Stable id, e.g. "snap_ab12cd34ef56".
    pub id: String,
    /// The conversation the snapshot belongs to.
    pub conversation_id: String,
    /// Which model step produced it.
    pub step: i64,
    /// Files captured, sorted by path.
    pub files: Vec<SnapshotFile>,
    /// When it was taken (unix seconds).
    pub created_at: i64,
    /// Paths that were named but not captured: too large, or a file type the
    /// snapshot store refuses. Only a fresh capture reports these; a snapshot
    /// read back from the store has an empty list.
    pub skipped: Vec<String>,
    #[serde(skip)]
    workspace: Option<WorkspacePin>,
}

/// A staged revert: the files that would be restored, and their content.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedRevert {
    /// Id of the staged revert.
    pub id: String,
    /// The conversation.
    pub conversation_id: String,
    /// The snapshot being reverted to.
    pub snapshot_id: String,
    /// The paths that would change.
    pub paths: Vec<String>,
    /// When it was staged.
    pub created_at: i64,
    #[serde(skip)]
    targets: Option<Vec<ReviewedTarget>>,
}

/// What a committed revert did.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertReport {
    /// Paths restored to their captured content.
    pub restored: Vec<String>,
    /// Paths deleted because the snapshot recorded them as absent.
    pub deleted: Vec<String>,
    /// Paths whose content had changed since the snapshot and were overwritten.
    pub overwritten: Vec<String>,
    /// Paths that could not be restored, with the reason.
    pub failed: Vec<(String, String)>,
}

/// One captured file, as the frontend sees it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotFileView {
    pub path: String,
    pub sha256: String,
    pub size: u64,
    pub deleted: bool,
}

/// A snapshot, as the frontend sees it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotView {
    pub id: String,
    pub conversation_id: String,
    pub step: i64,
    pub created_at: i64,
    pub files: Vec<SnapshotFileView>,
    pub skipped: Vec<String>,
}

/// A staged revert, as the frontend sees it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedRevertView {
    pub id: String,
    pub conversation_id: String,
    pub snapshot_id: String,
    pub paths: Vec<String>,
    pub created_at: i64,
}

/// A path that could not be restored.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertFailureView {
    pub path: String,
    pub reason: String,
}

/// What a committed revert did, as the frontend sees it.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevertReportView {
    pub restored: Vec<String>,
    pub deleted: Vec<String>,
    pub overwritten: Vec<String>,
    pub failed: Vec<RevertFailureView>,
}

impl From<SnapshotFile> for SnapshotFileView {
    fn from(file: SnapshotFile) -> Self {
        Self {
            path: file.path,
            sha256: file.sha256,
            size: file.size,
            deleted: file.deleted,
        }
    }
}

impl From<Snapshot> for SnapshotView {
    fn from(snapshot: Snapshot) -> Self {
        Self {
            id: snapshot.id,
            conversation_id: snapshot.conversation_id,
            step: snapshot.step,
            created_at: snapshot.created_at,
            files: snapshot.files.into_iter().map(SnapshotFileView::from).collect(),
            skipped: snapshot.skipped,
        }
    }
}

impl From<StagedRevert> for StagedRevertView {
    fn from(staged: StagedRevert) -> Self {
        Self {
            id: staged.id,
            conversation_id: staged.conversation_id,
            snapshot_id: staged.snapshot_id,
            paths: staged.paths,
            created_at: staged.created_at,
        }
    }
}

impl From<RevertReport> for RevertReportView {
    fn from(report: RevertReport) -> Self {
        Self {
            restored: report.restored,
            deleted: report.deleted,
            overwritten: report.overwritten,
            failed: report
                .failed
                .into_iter()
                .map(|(path, reason)| RevertFailureView { path, reason })
                .collect(),
        }
    }
}

fn db_error(error: rusqlite::Error) -> String {
    format!("Local database: {error}")
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or_default()
}

/// `snap_<12 hex>` for snapshots, `stg_<12 hex>` for staged reverts.
fn new_id(prefix: &str) -> String {
    let hex = format!("{:x}", uuid::Uuid::new_v4());
    format!("{prefix}_{}", &hex[..12])
}

/// Directory that holds the private, content-addressed snapshot blobs.
pub fn content_root(store: &crate::store::Store) -> PathBuf {
    store.data_dir().join("snapshots")
}

/// Reject anything that is not a plain relative path inside the workspace.
fn validate_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.len() > 4096 {
        return Err("Snapshot paths must name a file inside the workspace.".into());
    }
    if path.contains(['\0', '\\', ':']) || path.split('/').any(|part| matches!(part, "" | "." | "..")) {
        return Err(
            "Snapshot paths use forward slashes and cannot contain backslashes.".into(),
        );
    }
    let mut count = 0usize;
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => {
                let part = part.to_str().ok_or("Snapshot paths must be valid UTF-8.")?;
                if part.is_empty() || part == "." || part == ".." {
                    return Err(
                        "Snapshot paths must be relative and cannot traverse outside the workspace."
                            .into(),
                    );
                }
                count += 1;
            }
            _ => {
                return Err("Snapshot paths must be relative to the workspace root.".into());
            }
        }
    }
    if count == 0 {
        return Err("Snapshot paths must name a file inside the workspace.".into());
    }
    Ok(())
}

/// The blob that holds one piece of captured content.
fn blob_path(root: &Path, sha256: &str) -> Result<PathBuf, String> {
    if sha256.len() != 64 || !sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("The captured content reference is invalid.".into());
    }
    Ok(root.join(&sha256[..2]).join(sha256))
}

/// Store captured bytes in the private blob directory. Blobs are
/// content-addressed, so an identical file is stored once.
fn write_blob(root: &Path, sha256: &str, bytes: &[u8]) -> Result<(), String> {
    let path = blob_path(root, sha256)?;
    if let Ok(metadata) = std::fs::symlink_metadata(&path) {
        if metadata.is_file() {
            return Ok(());
        }
    }
    let directory = path
        .parent()
        .ok_or("The snapshot store has no directory for captured content.")?;
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("Cannot create the snapshot store: {error}"))?;
    let temporary = directory.join(format!("{sha256}.{}.tmp", uuid::Uuid::new_v4()));
    let write = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if let Err(error) = write {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("Cannot store the captured content: {error}"));
    }
    if let Err(error) = std::fs::rename(&temporary, &path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("Cannot store the captured content: {error}"));
    }
    Ok(())
}

fn read_blob(root: &Path, sha256: &str) -> Result<Vec<u8>, String> {
    let path = blob_path(root, sha256)?;
    std::fs::read(&path).map_err(|error| {
        format!("The captured content for {sha256} is no longer available: {error}")
    })
}

/// Run a body inside one SQLite transaction. The store is shared behind a
/// mutex, so an explicit transaction keeps a half-written snapshot from
/// becoming visible.
fn with_transaction<T>(
    store: &crate::store::Store,
    body: impl FnOnce(&Connection) -> Result<T, String>,
) -> Result<T, String> {
    store
        .connection
        .execute_batch("BEGIN IMMEDIATE")
        .map_err(db_error)?;
    match body(&store.connection) {
        Ok(value) => {
            store
                .connection
                .execute_batch("COMMIT")
                .map_err(db_error)?;
            Ok(value)
        }
        Err(error) => {
            let _ = store.connection.execute_batch("ROLLBACK");
            Err(error)
        }
    }
}

/// The workspace paths this conversation has already touched.
///
/// A snapshot only needs to cover the files the agent is likely to change,
/// and the tool audit already records every path the agent asked for. This
/// reads those paths back so the capture stays bounded instead of walking
/// the whole workspace.
pub fn tracked_paths(store: &crate::store::Store, conversation_id: &str) -> Vec<String> {
    let Ok(messages) = store.messages(conversation_id) else {
        return Vec::new();
    };
    let mut paths: Vec<String> = Vec::new();
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for message in messages.iter().rev() {
        if message.role != "tool" {
            continue;
        }
        let Ok(audit) = serde_json::from_str::<Value>(&message.content) else {
            continue;
        };
        collect_paths(&audit, &mut paths, &mut seen);
        if paths.len() >= MAX_FILES {
            break;
        }
    }
    paths.truncate(MAX_FILES);
    paths
}

/// Snapshot immediately before a mutation or executable hook. Known paths
/// take precedence so a first edit is covered without a prior tool audit.
pub(crate) fn capture_before_tool(
    state: &crate::AppState, conversation_id: &str, round: usize, root: &str, args: Option<&Value>,
) {
    if root.is_empty() { return; }
    let Ok(store) = state.database() else { return; };
    let mut paths = Vec::new();
    if let Some(args) = args {
        collect_paths(&json!({"request":{"arguments":args}}), &mut paths, &mut Default::default());
    }
    if paths.is_empty() {
        paths = tracked_paths(&store, conversation_id);
    }
    if !paths.is_empty() {
        let _ = capture(&store, conversation_id, round as i64, Path::new(root), &paths);
    }
}

/// Pull workspace-relative paths out of one tool audit row.
fn collect_paths(audit: &Value, paths: &mut Vec<String>, seen: &mut std::collections::BTreeSet<String>) {
    let Some(arguments) = audit
        .get("request")
        .and_then(|request| request.get("arguments"))
    else {
        return;
    };
    let mut push = |value: &str| {
        let trimmed = value.trim();
        if trimmed.is_empty() || trimmed.len() > 4096 {
            return;
        }
        // Only relative, ordinary paths can name a workspace file.
        if trimmed.starts_with('/') || trimmed.starts_with('\\') || trimmed.contains("..") {
            return;
        }
        if seen.insert(trimmed.to_string()) {
            paths.push(trimmed.to_string());
        }
    };
    if let Some(path) = arguments.get("path").and_then(Value::as_str) {
        push(path);
    }
    if let Some(changes) = arguments.get("changes").and_then(Value::as_array) {
        for change in changes {
            if let Some(path) = change.get("path").and_then(Value::as_str) {
                push(path);
            }
        }
    }
    if let Some(destination) = arguments.get("destination").and_then(Value::as_str) {
        push(destination);
    }
    if let Some(source) = arguments.get("source").and_then(Value::as_str) {
        push(source);
    }
    if let Some(items) = arguments.get("paths").and_then(Value::as_array) {
        for path in items.iter().filter_map(Value::as_str) { push(path); }
    }
}

/// Capture the given workspace-relative files into a snapshot.
///
/// Returns the snapshot id. Files that do not exist are recorded as
/// deleted (their absence is recorded) so a restore can delete them.
/// Files above the size cap are skipped and reported in `skipped`.
pub fn capture(
    store: &crate::store::Store,
    conversation_id: &str,
    step: i64,
    root: &std::path::Path,
    paths: &[String],
) -> Result<Snapshot, String> {
    if conversation_id.trim().is_empty() {
        return Err("A snapshot needs a conversation.".into());
    }
    if paths.len() > MAX_REQUESTED_PATHS {
        return Err(format!(
            "A snapshot captures at most {MAX_REQUESTED_PATHS} paths at once."
        ));
    }
    if !store.conversation_exists(conversation_id)? {
        return Err("That conversation no longer exists.".into());
    }
    let (workspace, directory) = WorkspacePin::open(root)?;
    let mut wanted: Vec<String> = Vec::new();
    for path in paths {
        let path = path.trim();
        validate_path(path)?;
        if !wanted.iter().any(|seen| seen == path) {
            wanted.push(path.to_string());
        }
    }
    wanted.sort();
    let blobs = content_root(store);
    let mut files: Vec<SnapshotFile> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for path in wanted {
        if files.len() >= MAX_FILES {
            skipped.push(path);
            continue;
        }
        match capture_file(&directory, &blobs, &path)? {
            Some(file) => files.push(file),
            None => skipped.push(path),
        }
    }
    let snapshot = Snapshot {
        id: new_id("snap"),
        conversation_id: conversation_id.to_string(),
        step,
        files,
        created_at: now_seconds(),
        skipped,
        workspace: Some(workspace),
    };
    persist(store, &snapshot)?;
    Ok(snapshot)
}

/// Read one workspace file without following a symbolic link. Returns `None`
/// when the file must be skipped (too large, or a credential path the
/// workspace tools also refuse to read).
fn capture_file(
    directory: &cap_std::fs::Dir,
    blobs: &Path,
    path: &str,
) -> Result<Option<SnapshotFile>, String> {
    // The sensitive-path policy is the one the workspace tools apply:
    // credential files are never copied into the snapshot store.
    if crate::sandbox::check_fs_path("", path, false).is_err() {
        return Ok(None);
    }
    let mut parent = directory
        .try_clone()
        .map_err(|error| format!("Cannot inspect '{path}': {error}"))?;
    let parts: Vec<&str> = path.split('/').collect();
    for part in &parts[..parts.len() - 1] {
        match parent.open_dir_nofollow(part) {
            Ok(next) => parent = next,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Some(absent_file(path)));
            }
            Err(error) => return Err(format!("Cannot inspect '{path}': {error}")),
        }
    }
    let name = parts[parts.len() - 1];
    let metadata = match parent.symlink_metadata(name) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Some(absent_file(path)));
        }
        Err(error) => return Err(format!("Cannot inspect '{path}': {error}")),
    };
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "Refusing to capture '{path}': symbolic links are never captured."
        ));
    }
    if !metadata.is_file() {
        return Err(format!(
            "Refusing to capture '{path}': only regular files are captured."
        ));
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Ok(None);
    }
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true);
    let file = crate::workspace::open_file_no_links(directory, path, &mut options)
        .map_err(|error| format!("Cannot read '{path}': {error}"))?;
    let mut data = Vec::new();
    file.take(MAX_FILE_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|error| format!("Cannot read '{path}': {error}"))?;
    if data.len() as u64 > MAX_FILE_BYTES {
        // The file grew past the cap between the check and the read.
        return Ok(None);
    }
    let sha256 = format!("{:x}", Sha256::digest(&data));
    write_blob(blobs, &sha256, &data)?;
    Ok(Some(SnapshotFile {
        path: path.to_string(),
        sha256,
        size: data.len() as u64,
        deleted: false,
    }))
}

/// A file that did not exist when the snapshot was taken.
fn absent_file(path: &str) -> SnapshotFile {
    SnapshotFile {
        path: path.to_string(),
        sha256: String::new(),
        size: 0,
        deleted: true,
    }
}

fn persist(store: &crate::store::Store, snapshot: &Snapshot) -> Result<(), String> {
    let workspace = snapshot.workspace.as_ref().map(serde_json::to_string)
        .transpose().map_err(|error| error.to_string())?;
    with_transaction(store, |connection| {
        connection
            .execute(
                "INSERT INTO snapshots(id,conversation_id,step,created_at,workspace_json) VALUES(?1,?2,?3,?4,?5)",
                params![
                    snapshot.id,
                    snapshot.conversation_id,
                    snapshot.step,
                    snapshot.created_at,
                    workspace,
                ],
            )
            .map_err(db_error)?;
        for file in &snapshot.files {
            connection
                .execute(
                    "INSERT INTO snapshot_files(snapshot_id,path,sha256,size,deleted) VALUES(?1,?2,?3,?4,?5)",
                    params![
                        snapshot.id,
                        file.path,
                        file.sha256,
                        file.size as i64,
                        file.deleted as i64
                    ],
                )
                .map_err(db_error)?;
        }
        Ok(())
    })
}

fn load_files(
    store: &crate::store::Store,
    snapshot_id: &str,
) -> Result<Vec<SnapshotFile>, String> {
    let mut statement = store
        .connection
        .prepare("SELECT path,sha256,size,deleted FROM snapshot_files WHERE snapshot_id=?1 ORDER BY path")
        .map_err(db_error)?;
    let rows = statement
        .query_map([snapshot_id], |row| {
            Ok(SnapshotFile {
                path: row.get(0)?,
                sha256: row.get(1)?,
                size: row.get::<_, i64>(2)? as u64,
                deleted: row.get::<_, i64>(3)? != 0,
            })
        })
        .map_err(db_error)?;
    rows.map(|row| row.map_err(db_error)).collect()
}

fn load_snapshot(
    store: &crate::store::Store,
    id: &str,
) -> Result<Option<Snapshot>, String> {
    let row = store
        .connection
        .query_row(
            "SELECT id,conversation_id,step,created_at,workspace_json FROM snapshots WHERE id=?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()
        .map_err(db_error)?;
    let Some((id, conversation_id, step, created_at, workspace)) = row else {
        return Ok(None);
    };
    let files = load_files(store, &id)?;
    Ok(Some(Snapshot {
        id,
        conversation_id,
        step,
        files,
        created_at,
        skipped: Vec::new(),
        workspace: workspace.as_deref().map(serde_json::from_str).transpose()
            .map_err(|error| format!("Invalid snapshot workspace: {error}"))?,
    }))
}

/// Stage a revert of the given snapshot.
///
/// Staging does NOT change any file. It records the intent so the user can
/// review it and then commit or discard it. Returns the staged revert.
pub fn stage(
    store: &crate::store::Store,
    conversation_id: &str,
    snapshot_id: &str,
) -> Result<StagedRevert, String> {
    if conversation_id.trim().is_empty() {
        return Err("A staged revert needs a conversation.".into());
    }
    let snapshot = load_snapshot(store, snapshot_id)?
        .ok_or("That snapshot no longer exists.".to_string())?;
    if snapshot.conversation_id != conversation_id {
        return Err("That snapshot belongs to another conversation.".into());
    }
    let workspace = snapshot.workspace.as_ref()
        .ok_or("This older snapshot has no verified workspace identity and cannot be restored safely.")?;
    let lock = crate::workspace::patch::root_lock(&workspace.path)?;
    let _guard = lock.read().map_err(|_| "Workspace transaction lock unavailable.")?;
    let directory = workspace.verify(&workspace.path)?;
    let targets: Vec<ReviewedTarget> = snapshot.files.iter()
        .map(|file| match review_target(&directory, &file.path) {
            Ok(Some(version)) => ReviewedTarget::Present(version),
            Ok(None) => ReviewedTarget::Absent,
            Err(_) => ReviewedTarget::Unavailable,
        })
        .collect();
    let target_json = serde_json::to_string(&targets).map_err(|error| error.to_string())?;
    let id = new_id("stg");
    let created_at = now_seconds();
    with_transaction(store, |connection| {
        // One staged revert per conversation: a newer intent replaces the
        // older one instead of stacking two competing rollbacks.
        connection
            .execute(
                "DELETE FROM staged_reverts WHERE conversation_id=?1",
                [conversation_id],
            )
            .map_err(db_error)?;
        connection
            .execute(
                "INSERT INTO staged_reverts(id,conversation_id,snapshot_id,created_at,targets_json) VALUES(?1,?2,?3,?4,?5)",
                params![id, conversation_id, snapshot_id, created_at, target_json],
            )
            .map_err(db_error)?;
        Ok(())
    })?;
    Ok(StagedRevert {
        id,
        conversation_id: conversation_id.to_string(),
        snapshot_id: snapshot_id.to_string(),
        paths: snapshot
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect(),
        created_at,
        targets: Some(targets),
    })
}

/// The staged revert with this id, if it still exists.
pub fn staged_revert(
    store: &crate::store::Store,
    staged_id: &str,
) -> Result<Option<StagedRevert>, String> {
    let row = store
        .connection
        .query_row(
            "SELECT id,conversation_id,snapshot_id,created_at,targets_json FROM staged_reverts WHERE id=?1",
            [staged_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()
        .map_err(db_error)?;
    let Some((id, conversation_id, snapshot_id, created_at, targets)) = row else {
        return Ok(None);
    };
    let paths = match load_snapshot(store, &snapshot_id)? {
        Some(snapshot) => snapshot
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect(),
        None => Vec::new(),
    };
    Ok(Some(StagedRevert {
        id,
        conversation_id,
        snapshot_id,
        paths,
        created_at,
        targets: targets.map(|json| serde_json::from_str(&json)
            .map_err(|error| format!("Invalid staged targets: {error}"))).transpose()?,
    }))
}

/// The staged revert of a conversation, if one is waiting for the user.
pub fn pending_revert(
    store: &crate::store::Store,
    conversation_id: &str,
) -> Result<Option<StagedRevert>, String> {
    let id: Option<String> = store
        .connection
        .query_row(
            "SELECT id FROM staged_reverts WHERE conversation_id=?1 ORDER BY created_at DESC,rowid DESC LIMIT 1",
            [conversation_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(db_error)?;
    match id {
        Some(id) => staged_revert(store, &id),
        None => Ok(None),
    }
}

/// Discard a staged revert.
pub fn discard(store: &crate::store::Store, staged_id: &str) -> Result<(), String> {
    let removed = store
        .connection
        .execute("DELETE FROM staged_reverts WHERE id=?1", [staged_id])
        .map_err(db_error)?;
    if removed == 0 {
        return Err("No staged revert with that id.".into());
    }
    Ok(())
}

/// The outcome of restoring one file.
enum Restored {
    /// The captured content was written back.
    Written { overwritten: bool },
    /// A file the snapshot recorded as absent was removed.
    Removed,
    /// The workspace already matched the snapshot.
    Unchanged,
}

/// Commit a staged revert: restore every captured file.
///
/// Files recorded as deleted are removed. Files whose current sha256 differs
/// from the reviewed state are preserved. The report distinguishes restored,
/// removed and changed targets so the user can review a new stage.
pub fn commit(
    store: &crate::store::Store,
    root: &std::path::Path,
    staged_id: &str,
) -> Result<RevertReport, String> {
    let canonical = std::fs::canonicalize(root).map_err(|e| format!("Cannot open the workspace: {e}"))?;
    if !crate::workspace::same_canonical_path(&canonical, root) {
        return Err("The workspace path changed; the revert remains staged.".into());
    }
    let root = canonical;
    let lock = crate::workspace::patch::root_lock(&root)?;
    let _guard = lock.write().map_err(|_| "Workspace transaction lock unavailable.")?;
    let staged = staged_revert(store, staged_id)?
        .ok_or("No staged revert with that id.".to_string())?;
    let snapshot = load_snapshot(store, &staged.snapshot_id)?
        .ok_or("The snapshot for that revert is no longer available.".to_string())?;
    let workspace = snapshot.workspace.as_ref()
        .ok_or("This older snapshot has no verified workspace identity and cannot be restored safely.")?;
    let directory = workspace.verify(&root)?;
    let targets = staged.targets.as_ref().filter(|targets| targets.len() == snapshot.files.len())
        .ok_or("The reviewed target versions are unavailable. Stage the revert again.")?;
    let blobs = content_root(store);
    let mut report = RevertReport::default();
    for (file, reviewed) in snapshot.files.iter().zip(targets) {
        let outcome = match reviewed {
            ReviewedTarget::Absent => restore(&directory, workspace, &blobs, file, None),
            ReviewedTarget::Present(version) => restore(&directory, workspace, &blobs, file, Some(version)),
            ReviewedTarget::Unavailable => Err("The target could not be reviewed. Stage the revert again after resolving it.".into()),
        };
        match outcome {
            Ok(Restored::Written { overwritten }) => {
                report.restored.push(file.path.clone());
                if overwritten {
                    report.overwritten.push(file.path.clone());
                }
            }
            Ok(Restored::Removed) => report.deleted.push(file.path.clone()),
            Ok(Restored::Unchanged) => {}
            Err(reason) => report.failed.push((file.path.clone(), reason)),
        }
    }
    if report.failed.is_empty() {
        // The revert is done. A revert with failures stays staged so the user
        // can retry it after fixing the cause.
        store.connection.execute("DELETE FROM staged_reverts WHERE id=?1", [staged_id])
            .map_err(|error| format!("Files were restored, but the staged revert could not be cleared: {error}"))?;
    }
    Ok(report)
}

/// Restore one captured file. The target is never written through a symbolic
/// link, and the content is published with a rename so a crash cannot leave a
/// half-written file.
fn review_target(root: &Dir, path: &str) -> Result<Option<TargetVersion>, String> {
    match crate::workspace::parent_no_links(root, path) {
        Ok((parent, leaf)) => open_restore_target(&parent, &leaf)?
            .as_ref().map(TargetVersion::read).transpose(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Cannot review '{path}': {error}")),
    }
}

fn restore(root: &Dir, workspace: &WorkspacePin, blobs: &Path, file: &SnapshotFile, reviewed: Option<&TargetVersion>) -> Result<Restored, String> {
    validate_path(&file.path)?;
    crate::sandbox::check_fs_path("", &file.path, true)?;
    let current = review_target(root, &file.path)?;
    if current.as_ref().map(|target| target.sha256.as_str()) == (!file.deleted).then_some(file.sha256.as_str()) {
        return Ok(Restored::Unchanged);
    }
    if current.as_ref() != reviewed {
        return Err("The target changed after review. Stage the revert again to review its current content.".into());
    }
    let (parent, leaf) = match crate::workspace::parent_no_links(root, &file.path) {
        Ok(target) => target,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound && file.deleted => {
            return Ok(Restored::Unchanged);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut parent = root.try_clone().map_err(|e| e.to_string())?;
            let mut parts = file.path.split('/').peekable();
            loop {
                let part = parts.next().ok_or("Missing restore target name.")?;
                if parts.peek().is_none() {
                    break (parent, part.to_string());
                }
                parent = match parent.open_dir_nofollow(part) {
                    Ok(next) => next,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        match parent.create_dir(part) {
                            Ok(()) => (),
                            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
                            Err(error) => return Err(error.to_string()),
                        }
                        parent.open_dir_nofollow(part).map_err(|e| e.to_string())?
                    }
                    Err(error) => return Err(error.to_string()),
                };
            }
        }
        Err(error) => return Err(format!("Cannot open restore parent without following links: {error}")),
    };
    let target = open_restore_target(&parent, &leaf)?;
    let verify_target = || -> Result<(), String> {
        workspace.verify(&workspace.path)?;
        let (current_parent, _) = crate::workspace::parent_no_links(root, &file.path)
            .map_err(|e| format!("Restore parent changed: {e}"))?;
        let expected = parent.dir_metadata().map_err(|e| e.to_string())?;
        let current = current_parent.dir_metadata().map_err(|e| e.to_string())?;
        if (expected.dev(), expected.ino()) != (current.dev(), current.ino()) {
            return Err("Restore parent changed; the target was preserved.".into());
        }
        if open_restore_target(&parent, &leaf)?.as_ref().map(TargetVersion::read).transpose()?.as_ref() != reviewed {
            return Err("The restore target changed during commit; it was preserved.".into());
        }
        Ok(())
    };
    if file.deleted {
        if target.is_none() {
            return Ok(Restored::Unchanged);
        }
        verify_target()?;
        parent.remove_file(&leaf).map_err(|e| format!("Cannot delete '{}': {e}", file.path))?;
        return Ok(Restored::Removed);
    }
    let permissions = target.as_ref().map(|f| f.metadata().map(|m| m.permissions()))
        .transpose().map_err(|e| e.to_string())?;
    let overwritten = current.is_some();
    let bytes = read_blob(blobs, &file.sha256)?;
    write_atomic(&parent, &leaf, &bytes, permissions, verify_target)?;
    Ok(Restored::Written { overwritten })
}

fn open_restore_target(parent: &Dir, leaf: &str) -> Result<Option<File>, String> {
    match crate::workspace::open_file_no_links(parent, leaf, OpenOptions::new().read(true)) {
        Ok(file) => Ok(Some(file)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("Cannot open '{leaf}' as a regular file without following symbolic links: {error}")),
    }
}

fn current_digest(file: &File) -> Result<String, String> {
    let mut data = Vec::new();
    file.take(MAX_FILE_BYTES + 1).read_to_end(&mut data)
        .map_err(|e| format!("Cannot read the current file: {e}"))?;
    Ok(if data.len() as u64 > MAX_FILE_BYTES {
        String::new()
    } else {
        format!("{:x}", Sha256::digest(&data))
    })
}

fn write_atomic(
    parent: &Dir,
    leaf: &str,
    bytes: &[u8],
    permissions: Option<Permissions>,
    verify_target: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let temporary = format!(".locallm-snap-{}.tmp", uuid::Uuid::new_v4());
    let mut file = parent.open_with(&temporary, OpenOptions::new().write(true).create_new(true))
        .map_err(|e| format!("Cannot stage restored file: {e}"))?;
    let replace = permissions.is_some();
    let result = (|| {
        file.write_all(bytes).map_err(|e| e.to_string())?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions).map_err(|e| e.to_string())?;
        }
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        verify_target()?;
        if replace {
            parent.rename(&temporary, parent, leaf)
        } else {
            // An external file created after inspection must be preserved.
            parent.hard_link(&temporary, parent, leaf)
        }
        .map_err(|e| format!("Cannot publish the restored file: {e}"))
    })();
    let _ = parent.remove_file(&temporary);
    result
}

/// The snapshots for a conversation, newest first.
pub fn list(
    store: &crate::store::Store,
    conversation_id: &str,
) -> Result<Vec<Snapshot>, String> {
    let mut statement = store
        .connection
        .prepare("SELECT id FROM snapshots WHERE conversation_id=?1 ORDER BY created_at DESC,rowid DESC")
        .map_err(db_error)?;
    let ids: Vec<String> = statement
        .query_map([conversation_id], |row| row.get::<_, String>(0))
        .map_err(db_error)?
        .map(|row| row.map_err(db_error))
        .collect::<Result<Vec<String>, String>>()?;
    drop(statement);
    let mut snapshots = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(snapshot) = load_snapshot(store, &id)? {
            snapshots.push(snapshot);
        }
    }
    Ok(snapshots)
}

/// Delete snapshots older than `keep` for a conversation, plus their content.
///
/// Returns how many snapshots were removed.
pub fn prune(
    store: &crate::store::Store,
    conversation_id: &str,
    keep: usize,
) -> Result<usize, String> {
    let mut statement = store
        .connection
        .prepare("SELECT id FROM snapshots WHERE conversation_id=?1 ORDER BY created_at DESC,rowid DESC")
        .map_err(db_error)?;
    let ids: Vec<String> = statement
        .query_map([conversation_id], |row| row.get::<_, String>(0))
        .map_err(db_error)?
        .map(|row| row.map_err(db_error))
        .collect::<Result<Vec<String>, String>>()?;
    drop(statement);
    let doomed: Vec<String> = ids.into_iter().skip(keep).collect();
    if doomed.is_empty() {
        return Ok(0);
    }
    let mut blobs: Vec<String> = Vec::new();
    with_transaction(store, |connection| {
        for id in &doomed {
            let mut files = connection
                .prepare("SELECT sha256 FROM snapshot_files WHERE snapshot_id=?1 AND deleted=0")
                .map_err(db_error)?;
            let hashes: Vec<String> = files
                .query_map([id], |row| row.get::<_, String>(0))
                .map_err(db_error)?
                .map(|row| row.map_err(db_error))
                .collect::<Result<Vec<String>, String>>()?;
            drop(files);
            blobs.extend(hashes);
            connection
                .execute("DELETE FROM snapshot_files WHERE snapshot_id=?1", [id])
                .map_err(db_error)?;
            connection
                .execute("DELETE FROM staged_reverts WHERE snapshot_id=?1", [id])
                .map_err(db_error)?;
            connection
                .execute("DELETE FROM snapshots WHERE id=?1", [id])
                .map_err(db_error)?;
        }
        Ok(())
    })?;
    // Content that no snapshot refers to any more is removed with it.
    let root = content_root(store);
    for sha256 in blobs {
        if referenced(store, &sha256)? {
            continue;
        }
        if let Ok(path) = blob_path(&root, &sha256) {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(doomed.len())
}

/// True when any surviving snapshot still refers to this content.
fn referenced(store: &crate::store::Store, sha256: &str) -> Result<bool, String> {
    let count: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM snapshot_files WHERE sha256=?1",
            [sha256],
            |row| row.get(0),
        )
        .map_err(db_error)?;
    Ok(count > 0)
}

/// The workspace a conversation's files live in: the project the task is
/// filed under, or the global workspace when it is not filed.
fn workspace_root(
    store: &crate::store::Store,
    conversation_id: &str,
) -> Result<PathBuf, String> {
    let project = crate::workspace_ui::task_workspace(store, conversation_id)?;
    let root = match project {
        Some(path) if !path.trim().is_empty() => path,
        _ => store.workspace_path()?,
    };
    if root.trim().is_empty() {
        return Err("Select a workspace folder first.".into());
    }
    Ok(PathBuf::from(root))
}

/// List the snapshots for a conversation.
#[tauri::command]
pub fn list_snapshots(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
) -> Result<Vec<SnapshotView>, String> {
    let store = state.database()?;
    Ok(list(&store, &conversation_id)?
        .into_iter()
        .map(SnapshotView::from)
        .collect())
}

/// Stage a revert of a snapshot.
#[tauri::command]
pub fn stage_revert(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
    snapshot_id: String,
) -> Result<StagedRevertView, String> {
    let store = state.database()?;
    let root = workspace_root(&store, &conversation_id)?;
    let snapshot = load_snapshot(&store, &snapshot_id)?.filter(|snapshot| snapshot.conversation_id == conversation_id)
        .ok_or("That snapshot is unavailable in this conversation.")?;
    snapshot.workspace.as_ref().ok_or("This older snapshot has no verified workspace identity and cannot be restored safely.")?
        .verify(&root)?;
    Ok(StagedRevertView::from(stage(
        &store,
        &conversation_id,
        &snapshot_id,
    )?))
}

/// Discard a staged revert.
#[tauri::command]
pub fn discard_revert(
    state: tauri::State<'_, crate::AppState>,
    staged_id: String,
) -> Result<(), String> {
    let store = state.database()?;
    discard(&store, &staged_id)
}

/// Commit a staged revert.
#[tauri::command]
pub fn commit_revert(
    state: tauri::State<'_, crate::AppState>,
    staged_id: String,
) -> Result<RevertReportView, String> {
    let store = state.database()?;
    let staged = staged_revert(&store, &staged_id)?
        .ok_or("No staged revert with that id.".to_string())?;
    let root = workspace_root(&store, &staged.conversation_id)?;
    Ok(RevertReportView::from(commit(&store, &root, &staged_id)?))
}

/// Capture a snapshot of the workspace.
#[tauri::command]
pub fn capture_snapshot(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
    step: i64,
    paths: Vec<String>,
) -> Result<SnapshotView, String> {
    let store = state.database()?;
    let root = workspace_root(&store, &conversation_id)?;
    Ok(SnapshotView::from(capture(
        &store,
        &conversation_id,
        step,
        &root,
        &paths,
    )?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> crate::store::Store {
        crate::store::Store::open_memory().unwrap()
    }

    fn conversation(store: &crate::store::Store) -> String {
        store.create_conversation().unwrap().id
    }

    fn blob_of(store: &crate::store::Store, sha256: &str) -> PathBuf {
        content_root(store).join(&sha256[..2]).join(sha256)
    }

    #[test]
    fn capture_records_the_hash_and_size_of_a_file() {
        let store = store();
        let conversation = conversation(&store);
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("main.rs"), "fn main() {}\n").unwrap();
        let snapshot = capture(
            &store,
            &conversation,
            3,
            workspace.path(),
            &["main.rs".to_string()],
        )
        .unwrap();
        assert_eq!(snapshot.conversation_id, conversation);
        assert_eq!(snapshot.step, 3);
        assert_eq!(snapshot.files.len(), 1);
        let file = &snapshot.files[0];
        assert_eq!(file.path, "main.rs");
        assert_eq!(file.size, 13);
        assert_eq!(
            file.sha256,
            format!("{:x}", Sha256::digest(b"fn main() {}\n"))
        );
        assert!(!file.deleted);
        assert!(snapshot.skipped.is_empty());
        // The captured bytes live beside the database, never in the workspace.
        assert!(blob_of(&store, &file.sha256).is_file());
        assert!(!workspace.path().join("snapshots").exists());
    }

    #[test]
    fn capture_records_a_missing_file_as_deleted() {
        let store = store();
        let conversation = conversation(&store);
        let workspace = tempfile::tempdir().unwrap();
        let snapshot = capture(
            &store,
            &conversation,
            1,
            workspace.path(),
            &["absent.txt".to_string()],
        )
        .unwrap();
        assert_eq!(snapshot.files.len(), 1);
        assert!(snapshot.files[0].deleted);
        assert_eq!(snapshot.files[0].size, 0);
        assert!(snapshot.files[0].sha256.is_empty());
    }

    #[test]
    fn capture_skips_files_above_the_size_cap() {
        let store = store();
        let conversation = conversation(&store);
        let workspace = tempfile::tempdir().unwrap();
        let big = vec![b'x'; MAX_FILE_BYTES as usize + 1];
        std::fs::write(workspace.path().join("big.bin"), &big).unwrap();
        std::fs::write(workspace.path().join("small.txt"), "small").unwrap();
        let snapshot = capture(
            &store,
            &conversation,
            1,
            workspace.path(),
            &["big.bin".to_string(), "small.txt".to_string()],
        )
        .unwrap();
        assert_eq!(snapshot.files.len(), 1);
        assert_eq!(snapshot.files[0].path, "small.txt");
        assert_eq!(snapshot.skipped, vec!["big.bin".to_string()]);
    }

    #[test]
    fn capture_rejects_paths_outside_the_root() {
        let store = store();
        let conversation = conversation(&store);
        let workspace = tempfile::tempdir().unwrap();
        for path in [
            "../outside.txt",
            "/etc/passwd",
            "nested/../../escape.txt",
            "./relative.txt",
            "",
        ] {
            assert!(
                capture(
                    &store,
                    &conversation,
                    1,
                    workspace.path(),
                    &[path.to_string()],
                )
                .is_err(),
                "expected {path} to be refused"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn capture_refuses_a_symlink() {
        let store = store();
        let conversation = conversation(&store);
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("real.txt"), "real").unwrap();
        std::os::unix::fs::symlink("real.txt", workspace.path().join("link.txt")).unwrap();
        assert!(capture(
            &store,
            &conversation,
            1,
            workspace.path(),
            &["link.txt".to_string()],
        )
        .is_err());
    }

    #[test]
    fn staging_changes_nothing_on_disk() {
        let store = store();
        let conversation = conversation(&store);
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("main.rs"), "first\n").unwrap();
        let snapshot = capture(
            &store,
            &conversation,
            1,
            workspace.path(),
            &["main.rs".to_string()],
        )
        .unwrap();
        std::fs::write(workspace.path().join("main.rs"), "second\n").unwrap();
        let staged = stage(&store, &conversation, &snapshot.id).unwrap();
        assert_eq!(staged.snapshot_id, snapshot.id);
        assert_eq!(staged.paths, vec!["main.rs".to_string()]);
        assert_eq!(
            std::fs::read_to_string(workspace.path().join("main.rs")).unwrap(),
            "second\n"
        );
        assert!(pending_revert(&store, &conversation).unwrap().is_some());
    }

    #[test]
    fn commit_restores_the_captured_content() {
        let store = store();
        let conversation = conversation(&store);
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("main.rs"), "first\n").unwrap();
        let snapshot = capture(
            &store,
            &conversation,
            1,
            workspace.path(),
            &["main.rs".to_string()],
        )
        .unwrap();
        std::fs::write(workspace.path().join("main.rs"), "second\n").unwrap();
        let staged = stage(&store, &conversation, &snapshot.id).unwrap();
        let report = commit(&store, workspace.path(), &staged.id).unwrap();
        assert_eq!(report.restored, vec!["main.rs".to_string()]);
        assert!(report.failed.is_empty());
        assert_eq!(
            std::fs::read_to_string(workspace.path().join("main.rs")).unwrap(),
            "first\n"
        );
        // The revert is finished, so nothing is left staged.
        assert!(pending_revert(&store, &conversation).unwrap().is_none());
    }

    #[test]
    fn a_staged_revert_cannot_cross_workspaces_or_overwrite_a_later_edit() {
        let store = store();
        let conversation = conversation(&store);
        let original = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let path = original.path().join("main.rs");
        let other_path = other.path().join("main.rs");
        std::fs::write(&path, "original").unwrap();
        let snapshot = capture(&store, &conversation, 0, original.path(), &["main.rs".into()]).unwrap();
        std::fs::write(&path, "agent change").unwrap();
        std::fs::write(&other_path, "agent change").unwrap();
        let staged = stage(&store, &conversation, &snapshot.id).unwrap();
        assert!(commit(&store, other.path(), &staged.id).is_err());
        assert_eq!(std::fs::read_to_string(other_path).unwrap(), "agent change");
        std::fs::write(&path, "later user edit").unwrap();
        let report = commit(&store, original.path(), &staged.id).unwrap();
        assert!(report.failed[0].1.contains("changed after review"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "later user edit");
        let reviewed = stage(&store, &conversation, &snapshot.id).unwrap();
        assert!(commit(&store, original.path(), &reviewed.id).unwrap().failed.is_empty());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "original");
    }

    #[test]
    fn a_replacement_directory_at_the_same_path_does_not_match_the_snapshot() {
        let store = store();
        let conversation = conversation(&store);
        let parent = tempfile::tempdir().unwrap();
        let root = parent.path().join("workspace");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("main.rs"), "original").unwrap();
        let snapshot = capture(&store, &conversation, 0, &root, &["main.rs".into()]).unwrap();
        let staged = stage(&store, &conversation, &snapshot.id).unwrap();
        std::fs::rename(&root, parent.path().join("moved")).unwrap();
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("main.rs"), "replacement").unwrap();
        assert!(commit(&store, &root, &staged.id).is_err());
        assert_eq!(std::fs::read_to_string(root.join("main.rs")).unwrap(), "replacement");
    }

    #[test]
    fn commit_reports_content_changed_after_the_snapshot() {
        let store = store();
        let conversation = conversation(&store);
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("main.rs"), "first\n").unwrap();
        let snapshot = capture(
            &store,
            &conversation,
            1,
            workspace.path(),
            &["main.rs".to_string()],
        )
        .unwrap();
        std::fs::write(workspace.path().join("main.rs"), "second\n").unwrap();
        let staged = stage(&store, &conversation, &snapshot.id).unwrap();
        let report = commit(&store, workspace.path(), &staged.id).unwrap();
        assert_eq!(report.restored, vec!["main.rs".to_string()]);
        assert_eq!(report.overwritten, vec!["main.rs".to_string()]);
        assert_eq!(
            std::fs::read_to_string(workspace.path().join("main.rs")).unwrap(),
            "first\n"
        );
    }

    #[test]
    fn commit_deletes_a_file_the_snapshot_recorded_as_absent() {
        let store = store();
        let conversation = conversation(&store);
        let workspace = tempfile::tempdir().unwrap();
        let snapshot = capture(
            &store,
            &conversation,
            1,
            workspace.path(),
            &["new.txt".to_string()],
        )
        .unwrap();
        assert!(snapshot.files[0].deleted);
        std::fs::write(workspace.path().join("new.txt"), "written later\n").unwrap();
        let staged = stage(&store, &conversation, &snapshot.id).unwrap();
        let report = commit(&store, workspace.path(), &staged.id).unwrap();
        assert_eq!(report.deleted, vec!["new.txt".to_string()]);
        assert!(!workspace.path().join("new.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn commit_refuses_to_write_through_a_symlink() {
        let store = store();
        let conversation = conversation(&store);
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("real.txt"), "first\n").unwrap();
        let snapshot = capture(
            &store,
            &conversation,
            1,
            workspace.path(),
            &["real.txt".to_string()],
        )
        .unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("target.txt"), "outside\n").unwrap();
        // The captured file is replaced by a link to a file outside the
        // workspace: a restore must refuse it instead of writing through it.
        std::fs::remove_file(workspace.path().join("real.txt")).unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("target.txt"),
            workspace.path().join("real.txt"),
        )
        .unwrap();
        let staged = stage(&store, &conversation, &snapshot.id).unwrap();
        let report = commit(&store, workspace.path(), &staged.id).unwrap();
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, "real.txt");
        assert!(report.restored.is_empty());
        // The link target is untouched.
        assert_eq!(
            std::fs::read_to_string(outside.path().join("target.txt")).unwrap(),
            "outside\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn commit_keeps_the_executable_bit_of_the_restored_file() {
        use std::os::unix::fs::PermissionsExt;
        let store = store();
        let conversation = conversation(&store);
        let workspace = tempfile::tempdir().unwrap();
        let target = workspace.path().join("run.sh");
        std::fs::write(&target, "#!/bin/sh\necho one\n").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755)).unwrap();
        let snapshot = capture(
            &store,
            &conversation,
            1,
            workspace.path(),
            &["run.sh".to_string()],
        )
        .unwrap();
        std::fs::write(&target, "#!/bin/sh\necho two\n").unwrap();
        let staged = stage(&store, &conversation, &snapshot.id).unwrap();
        commit(&store, workspace.path(), &staged.id).unwrap();
        let mode = std::fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "#!/bin/sh\necho one\n"
        );
    }

    #[test]
    fn discard_leaves_the_workspace_alone_and_forgets_the_revert() {
        let store = store();
        let conversation = conversation(&store);
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("main.rs"), "first\n").unwrap();
        let snapshot = capture(
            &store,
            &conversation,
            1,
            workspace.path(),
            &["main.rs".to_string()],
        )
        .unwrap();
        std::fs::write(workspace.path().join("main.rs"), "second\n").unwrap();
        let staged = stage(&store, &conversation, &snapshot.id).unwrap();
        discard(&store, &staged.id).unwrap();
        assert!(pending_revert(&store, &conversation).unwrap().is_none());
        assert!(staged_revert(&store, &staged.id).unwrap().is_none());
        assert!(discard(&store, &staged.id).is_err());
        assert_eq!(
            std::fs::read_to_string(workspace.path().join("main.rs")).unwrap(),
            "second\n"
        );
    }

    #[test]
    fn prune_removes_old_snapshots_and_their_content() {
        let store = store();
        let conversation = conversation(&store);
        let workspace = tempfile::tempdir().unwrap();
        let mut snapshots = Vec::new();
        for index in 0..3 {
            std::fs::write(
                workspace.path().join("main.rs"),
                format!("version {index}\n"),
            )
            .unwrap();
            snapshots.push(
                capture(
                    &store,
                    &conversation,
                    index,
                    workspace.path(),
                    &["main.rs".to_string()],
                )
                .unwrap(),
            );
        }
        let blobs: Vec<PathBuf> = snapshots
            .iter()
            .map(|snapshot| blob_of(&store, &snapshot.files[0].sha256))
            .collect();
        assert!(blobs.iter().all(|blob| blob.is_file()));
        let removed = prune(&store, &conversation, 1).unwrap();
        assert_eq!(removed, 2);
        let remaining = list(&store, &conversation).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, snapshots[2].id);
        assert!(blobs[2].is_file());
        assert!(!blobs[0].exists());
        assert!(!blobs[1].exists());
    }

    #[test]
    fn list_returns_the_newest_snapshot_first() {
        let store = store();
        let conversation = conversation(&store);
        let workspace = tempfile::tempdir().unwrap();
        let mut ids = Vec::new();
        for index in 0..3 {
            std::fs::write(
                workspace.path().join("main.rs"),
                format!("version {index}\n"),
            )
            .unwrap();
            ids.push(
                capture(
                    &store,
                    &conversation,
                    index,
                    workspace.path(),
                    &["main.rs".to_string()],
                )
                .unwrap()
                .id,
            );
        }
        let listed = list(&store, &conversation).unwrap();
        assert_eq!(listed.len(), 3);
        assert_eq!(listed[0].id, ids[2]);
        assert_eq!(listed[1].id, ids[1]);
        assert_eq!(listed[2].id, ids[0]);
    }

    #[cfg(unix)]
    #[test]
    fn restore_never_follows_a_parent_link_for_writes_or_deletes() {
        let store = store();
        let owner = conversation(&store);
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let nested = root.path().join("nested");
        std::fs::create_dir(&nested).unwrap();
        std::fs::write(nested.join("restore.txt"), "captured").unwrap();
        let snapshot = capture(&store, &owner, 1, root.path(),
            &["nested/restore.txt".into(), "nested/delete.txt".into()]).unwrap();
        let staged = stage(&store, &owner, &snapshot.id).unwrap();
        std::fs::rename(&nested, root.path().join("previous")).unwrap();
        std::fs::write(outside.path().join("restore.txt"), "outside").unwrap();
        std::fs::write(outside.path().join("delete.txt"), "preserve").unwrap();
        std::os::unix::fs::symlink(outside.path(), nested).unwrap();
        let report = commit(&store, root.path(), &staged.id).unwrap();
        assert_eq!(report.failed.len(), 2, "{report:?}");
        assert_eq!(std::fs::read_to_string(outside.path().join("restore.txt")).unwrap(), "outside");
        assert_eq!(std::fs::read_to_string(outside.path().join("delete.txt")).unwrap(), "preserve");
    }

}
