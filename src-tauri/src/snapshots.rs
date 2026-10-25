//! Workspace snapshots: capture the files an agent changed, and restore them.
//!
//! A snapshot is the localLM answer to "the agent broke my file". The agent
//! loop captures the workspace files it is about to touch before a model step
//! and again when the step completes; `/undo` stages a rollback of the latest
//! snapshot, committing it restores the captured bytes, and discarding it
//! leaves the workspace exactly as it was.
//!
//! Captured bytes never live in the workspace. They are stored in a private,
//! content-addressed blob directory next to the local database
//! (`<data dir>/snapshots/<sha[0..2]>/<sha256>`) and the SQLite rows hold
//! only metadata. A restore republishes a blob through a temporary file in
//! the target's own directory followed by a rename, so a crash cannot leave a
//! half-written file behind, and a symbolic link is never written through.

use cap_fs_ext::DirExt;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
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
            files: snapshot
                .files
                .into_iter()
                .map(SnapshotFileView::from)
                .collect(),
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
    if path.contains('\0') || path.contains('\\') {
        return Err("Snapshot paths use forward slashes and cannot contain backslashes.".into());
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
            store.connection.execute_batch("COMMIT").map_err(db_error)?;
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

/// Pull workspace-relative paths out of one tool audit row.
fn collect_paths(
    audit: &Value,
    paths: &mut Vec<String>,
    seen: &mut std::collections::BTreeSet<String>,
) {
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
    let directory = cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority())
        .map_err(|error| format!("Cannot open the workspace: {error}"))?;
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
    with_transaction(store, |connection| {
        connection
            .execute(
                "INSERT INTO snapshots(id,conversation_id,step,created_at) VALUES(?1,?2,?3,?4)",
                params![
                    snapshot.id,
                    snapshot.conversation_id,
                    snapshot.step,
                    snapshot.created_at
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

fn load_files(store: &crate::store::Store, snapshot_id: &str) -> Result<Vec<SnapshotFile>, String> {
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

fn load_snapshot(store: &crate::store::Store, id: &str) -> Result<Option<Snapshot>, String> {
    let row = store
        .connection
        .query_row(
            "SELECT id,conversation_id,step,created_at FROM snapshots WHERE id=?1",
            [id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(db_error)?;
    let Some((id, conversation_id, step, created_at)) = row else {
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
    let snapshot =
        load_snapshot(store, snapshot_id)?.ok_or("That snapshot no longer exists.".to_string())?;
    if snapshot.conversation_id != conversation_id {
        return Err("That snapshot belongs to another conversation.".into());
    }
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
                "INSERT INTO staged_reverts(id,conversation_id,snapshot_id,created_at) VALUES(?1,?2,?3,?4)",
                params![id, conversation_id, snapshot_id, created_at],
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
            "SELECT id,conversation_id,snapshot_id,created_at FROM staged_reverts WHERE id=?1",
            [staged_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()
        .map_err(db_error)?;
    let Some((id, conversation_id, snapshot_id, created_at)) = row else {
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
/// from the sha256 recorded when the snapshot was taken are RESTORED ANYWAY
/// (the snapshot is the older truth) but the result reports which paths
/// overwrote newer content, so the caller can tell the user.
pub fn commit(
    store: &crate::store::Store,
    root: &std::path::Path,
    staged_id: &str,
) -> Result<RevertReport, String> {
    let staged =
        staged_revert(store, staged_id)?.ok_or("No staged revert with that id.".to_string())?;
    let snapshot = load_snapshot(store, &staged.snapshot_id)?
        .ok_or("The snapshot for that revert is no longer available.".to_string())?;
    let blobs = content_root(store);
    let mut report = RevertReport::default();
    for file in &snapshot.files {
        match restore(root, &blobs, file, staged_id) {
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
        let _ = store
            .connection
            .execute("DELETE FROM staged_reverts WHERE id=?1", [staged_id])
            .map_err(db_error);
    }
    Ok(report)
}

/// Restore one captured file. The target is never written through a symbolic
/// link, and the content is published with a rename so a crash cannot leave a
/// half-written file.
fn restore(
    root: &Path,
    blobs: &Path,
    file: &SnapshotFile,
    staged_id: &str,
) -> Result<Restored, String> {
    validate_path(&file.path)?;
    let target = root.join(&file.path);
    let metadata = match std::fs::symlink_metadata(&target) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("Cannot inspect '{}': {error}", file.path)),
    };
    if let Some(metadata) = &metadata {
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Refusing to write through the symbolic link '{}'.",
                file.path
            ));
        }
    }
    if file.deleted {
        let Some(metadata) = metadata else {
            return Ok(Restored::Unchanged);
        };
        if !metadata.is_file() {
            return Err(format!(
                "'{}' is not a regular file; it was left alone.",
                file.path
            ));
        }
        std::fs::remove_file(&target)
            .map_err(|error| format!("Cannot delete '{}': {error}", file.path))?;
        return Ok(Restored::Removed);
    }
    let overwritten = match &metadata {
        None => false,
        Some(metadata) if metadata.is_file() => match current_digest(&target) {
            Ok(digest) => digest != file.sha256,
            // The current content cannot be read, so it cannot be the
            // captured one. The snapshot is still the older truth.
            Err(_) => true,
        },
        Some(_) => {
            return Err(format!(
                "'{}' is not a regular file; it was left alone.",
                file.path
            ))
        }
    };
    let permissions = metadata
        .as_ref()
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.permissions());
    let bytes = read_blob(blobs, &file.sha256)?;
    write_atomic(&target, &bytes, staged_id, permissions)?;
    Ok(Restored::Written { overwritten })
}

/// The sha256 of the file as it is on disk now, or an empty string when it
/// cannot be read (which counts as "changed" for the caller).
fn current_digest(target: &Path) -> Result<String, String> {
    let mut file = match std::fs::File::open(target) {
        Ok(file) => file,
        Err(error) => return Err(format!("Cannot read the current file: {error}")),
    };
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65_536];
    let mut read = 0u64;
    loop {
        let count = match file.read(&mut buffer) {
            Ok(count) => count,
            Err(error) => return Err(format!("Cannot read the current file: {error}")),
        };
        if count == 0 {
            break;
        }
        read += count as u64;
        if read > MAX_FILE_BYTES {
            // Bigger than any captured content: it cannot be the same file.
            return Ok(String::new());
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Publish bytes at the target through a temporary file in the target's own
/// directory and a rename, so the target is never half-written. The target's
/// current permissions are kept when it still exists; a file that is created
/// from scratch gets the default permissions of the directory.
fn write_atomic(
    target: &Path,
    bytes: &[u8],
    staged_id: &str,
    permissions: Option<std::fs::Permissions>,
) -> Result<(), String> {
    let parent = target
        .parent()
        .ok_or("The restored file has no parent directory.")?;
    if !parent.exists() {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "Cannot create the directory for '{}': {error}",
                target.display()
            )
        })?;
    }
    let leaf = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("The restored file has no usable name.")?;
    let temporary = parent.join(format!(
        ".{leaf}.locallm-snap-{staged_id}-{}.tmp",
        uuid::Uuid::new_v4()
    ));
    let write = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        if let Some(permissions) = permissions {
            file.set_permissions(permissions)?;
        }
        file.sync_all()
    })();
    if let Err(error) = write {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("Cannot write the restored file: {error}"));
    }
    if let Err(error) = std::fs::rename(&temporary, target) {
        let _ = std::fs::remove_file(&temporary);
        return Err(format!("Cannot publish the restored file: {error}"));
    }
    Ok(())
}

/// The snapshots for a conversation, newest first.
pub fn list(store: &crate::store::Store, conversation_id: &str) -> Result<Vec<Snapshot>, String> {
    let mut statement = store
        .connection
        .prepare(
            "SELECT id FROM snapshots WHERE conversation_id=?1 ORDER BY created_at DESC,rowid DESC",
        )
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
        .prepare(
            "SELECT id FROM snapshots WHERE conversation_id=?1 ORDER BY created_at DESC,rowid DESC",
        )
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
fn workspace_root(store: &crate::store::Store, conversation_id: &str) -> Result<PathBuf, String> {
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
    let staged =
        staged_revert(&store, &staged_id)?.ok_or("No staged revert with that id.".to_string())?;
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
}
