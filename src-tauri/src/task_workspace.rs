//! Isolated task workspaces.
//!
//! A task workspace gives one conversation its own copy of the project: a git
//! worktree when the workspace is a git repository, or a bounded snapshot copy
//! otherwise. Agent tools, execution, and the terminal resolve the
//! conversation's root to the isolated directory while a task is active, so
//! work can be reviewed as a diff against the task baseline and applied back
//! to the source folder — or discarded — without touching the user's tree
//! first.
//!
//! This is a review/recovery boundary, not a security sandbox: executed code
//! keeps the user's full filesystem and network access.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

pub const KIND_WORKTREE: &str = "git-worktree";
pub const KIND_SNAPSHOT: &str = "snapshot";
pub const STATUS_ACTIVE: &str = "active";
pub const STATUS_APPLIED: &str = "applied";
pub const STATUS_DISCARDED: &str = "discarded";

/// Carried working-tree changes skip files past this size.
const MAX_CARRY_FILE_BYTES: u64 = 5 * 1024 * 1024;
/// Snapshot copies abort past these bounds instead of filling the app dir.
const MAX_SNAPSHOT_FILES: usize = 20_000;
const MAX_SNAPSHOT_BYTES: u64 = 500 * 1024 * 1024;
/// `task_file_diff` never returns more than this much unified text.
const DIFF_TEXT_CAP: usize = 256 * 1024;
const DIFF_MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Exclusion {
    pub path: String,
    pub reason: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskWorkspace {
    pub id: String,
    pub conversation_id: String,
    pub source_root: String,
    pub kind: String,
    pub isolated_root: String,
    pub branch: Option<String>,
    pub baseline_commit: String,
    pub status: String,
    pub exclusions: Vec<Exclusion>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedFile {
    pub path: String,
    /// 'created' | 'written' | 'deleted'
    pub action: String,
    /// SHA-256 of what apply left at the destination (None when it deleted).
    pub wrote_sha256: Option<String>,
    /// Path of the saved pre-apply destination, relative to `backup_dir`.
    pub backup: Option<String>,
    /// Task version kept beside the backup when a conflict was overridden.
    pub task_version: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskApply {
    pub id: String,
    pub task_id: String,
    pub created_at: i64,
    pub backup_dir: String,
    pub files: Vec<AppliedFile>,
    pub undone_at: Option<i64>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskCheckpoint {
    pub commit: Option<String>,
    pub label: String,
    pub created_at: i64,
    /// Synthetic entry: the working tree has changes no checkpoint covers.
    pub uncommitted: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskFileChange {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String,
    pub binary: bool,
    pub size: u64,
    pub baseline_hash: Option<String>,
    pub task_hash: Option<String>,
    pub destination_hash: Option<String>,
    /// The source file no longer matches the task baseline (someone edited
    /// it since the task was created) — applying would overwrite real work.
    pub destination_changed: bool,
    pub conflict: bool,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskReview {
    pub task_id: String,
    pub source_root: String,
    pub isolated_root: String,
    pub kind: String,
    pub branch: Option<String>,
    pub baseline_commit: String,
    pub exclusions: Vec<Exclusion>,
    pub files: Vec<TaskFileChange>,
    pub checkpoints: Vec<TaskCheckpoint>,
    pub applies: Vec<TaskApply>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskFileDiff {
    pub text: Option<String>,
    pub truncated: bool,
    pub binary: bool,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskApplyResult {
    pub applied: Vec<String>,
    /// Conflicts the caller deliberately resolved to keep the destination.
    pub kept_destination: Vec<String>,
    /// Conflicts with no resolution; left untouched for a later apply.
    pub skipped_conflicts: Vec<String>,
    pub backup_dir: String,
    pub apply_id: String,
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskUndoResult {
    pub restored: Vec<String>,
    /// Destination no longer matches what apply wrote; left untouched.
    pub conflicts: Vec<String>,
}

pub(crate) fn git_command(
    command: &mut std::process::Command,
    root: &Path,
    args: &[&str],
    index: Option<&Path>,
) {
    command
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    if let Some(index) = index {
        command.env("GIT_INDEX_FILE", index);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
}

pub(crate) fn run_command(mut command: std::process::Command) -> Result<Vec<u8>, String> {
    let output = command
        .output()
        .map_err(|e| format!("Cannot run Git: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(stderr.trim().chars().take(2_000).collect());
    }
    Ok(output.stdout)
}

pub(crate) fn run_git_env(root: &Path, args: &[&str], index: Option<&Path>) -> Result<String, String> {
    let mut command = std::process::Command::new("git");
    git_command(&mut command, root, args, index);
    let bytes = run_command(command)?;
    Ok(String::from_utf8_lossy(&bytes)
        .chars()
        .take(400_000)
        .collect())
}

pub(crate) fn run_git(root: &Path, args: &[&str]) -> Result<String, String> {
    run_git_env(root, args, None)
}

/// Git inside the isolated task repo. `core.autocrlf=false` keeps checkouts
/// and stages byte-exact regardless of the user's global git config.
pub(crate) fn run_git_iso(root: &Path, args: &[&str]) -> Result<String, String> {
    let mut full = vec!["-c", "core.autocrlf=false"];
    full.extend_from_slice(args);
    run_git(root, &full)
}

/// Same as `run_git_iso` but against a caller-supplied index file, so
/// reviewing untracked changes never touches the real index.
pub(crate) fn run_git_indexed(root: &Path, index: &Path, args: &[&str]) -> Result<String, String> {
    let mut full = vec!["-c", "core.autocrlf=false"];
    full.extend_from_slice(args);
    run_git_env(root, &full, Some(index))
}

fn git_bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let mut full = vec!["-c", "core.autocrlf=false"];
    full.extend_from_slice(args);
    let mut command = std::process::Command::new("git");
    git_command(&mut command, root, &full, None);
    run_command(command)
}

pub(crate) fn require_git() -> Result<(), String> {
    std::process::Command::new("git")
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("Cannot run Git: {e}"))?
        .success()
        .then_some(())
        .ok_or_else(|| {
            "Install Git and make sure `git` is on PATH before isolating a task.".to_string()
        })
}

pub(crate) fn canonical(path: &Path) -> Result<PathBuf, String> {
    std::fs::canonicalize(path).map_err(|e| format!("Cannot open {}: {e}", path.display()))
}

/// Relative repo paths from git output must stay inside the tree.
fn safe_relative(path: &str) -> Result<PathBuf, String> {
    if path.is_empty() || path.len() > 4096 {
        return Err("Invalid file path in task changes.".into());
    }
    let rel = PathBuf::from(path);
    if rel.is_absolute()
        || rel
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(format!("Invalid file path in task changes: {path}"));
    }
    Ok(rel)
}

fn sha256_file(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(format!("{:x}", Sha256::digest(&bytes)))
}

pub(crate) fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Hash for baseline-vs-destination comparison. Text files ignore CRLF/LF
/// differences — an autocrlf checkout must not turn every file into a
/// conflict. Binary content stays byte-exact.
fn comparable_hash(bytes: &[u8]) -> String {
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return sha256_bytes(bytes);
    }
    let mut normalized = Vec::with_capacity(bytes.len());
    let mut cursor = 0;
    while let Some(cr) = bytes[cursor..].iter().position(|b| *b == b'\r') {
        let absolute = cursor + cr;
        if bytes.get(absolute + 1) == Some(&b'\n') {
            normalized.extend_from_slice(&bytes[cursor..absolute]);
            normalized.push(b'\n');
            cursor = absolute + 2;
        } else {
            normalized.extend_from_slice(&bytes[cursor..=absolute]);
            cursor = absolute + 1;
        }
    }
    normalized.extend_from_slice(&bytes[cursor..]);
    sha256_bytes(&normalized)
}

fn comparable_file_hash(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(comparable_hash(&bytes))
}

fn looks_binary(path: &Path) -> bool {
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    use std::io::Read;
    let mut buf = [0u8; 8192];
    let Ok(count) = file.read(&mut buf) else {
        return false;
    };
    buf[..count].contains(&0)
}

/// Files and folders never carried into a task workspace: big binaries, secret
/// material, and generated/dependency trees the agent should not touch.
fn excluded_reason(relative: &Path, size: Option<u64>) -> Option<String> {
    let generated = ["node_modules", "target", "dist", ".venv", "__pycache__"];
    for part in relative.components() {
        let Component::Normal(name) = part else {
            continue;
        };
        let name = name.to_string_lossy();
        if generated.iter().any(|dir| name == *dir) {
            return Some("generated/dependency folder".to_string());
        }
    }
    let name = relative
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())?;
    let secret = name == ".env"
        || name.starts_with(".env.")
        || name.ends_with(".pem")
        || name.ends_with(".key")
        || name.ends_with(".p12")
        || name.ends_with(".pfx")
        || name.starts_with("id_rsa")
        || name.starts_with("id_ed25519")
        || (name.starts_with("credentials") && name.ends_with(".json"));
    if secret {
        return Some("looks like a secret; never copied into task workspaces".to_string());
    }
    if size.is_some_and(|bytes| bytes > MAX_CARRY_FILE_BYTES) {
        return Some("larger than 5 MiB".to_string());
    }
    None
}

/// Stage everything and record the task baseline commit inside the isolated
/// repo. The source repo's index and branch are never touched.
fn commit_baseline(isolated_root: &Path) -> Result<String, String> {
    run_git_iso(isolated_root, &["add", "-A"])?;
    run_git_iso(
        isolated_root,
        &[
            "-c",
            "user.name=LocalLM",
            "-c",
            "user.email=localllm@localhost",
            "commit",
            "--no-verify",
            "--quiet",
            "--allow-empty",
            "-m",
            "LocalLM task baseline",
        ],
    )?;
    Ok(run_git_iso(isolated_root, &["rev-parse", "HEAD"])?
        .trim()
        .to_string())
}

/// Commit the isolated working tree if anything changed since HEAD.
/// Checkpoints are what make apply/restore recoverable, so a failure here is
/// reported to the caller for explicit calls and ignored for auto-checkpoints.
fn checkpoint(task: &TaskWorkspace, label: &str) -> Result<Option<String>, String> {
    let root = Path::new(&task.isolated_root);
    let dirty = run_git_iso(root, &["status", "--porcelain"]).map(|out| !out.trim().is_empty())?;
    if !dirty {
        return Ok(None);
    }
    run_git_iso(root, &["add", "-A"])?;
    run_git_iso(
        root,
        &[
            "-c",
            "user.name=LocalLM",
            "-c",
            "user.email=localllm@localhost",
            "commit",
            "--no-verify",
            "--quiet",
            "-m",
            &format!("checkpoint: {label}"),
        ],
    )?;
    Ok(run_git_iso(root, &["rev-parse", "HEAD"])
        .ok()
        .map(|sha| sha.trim().to_string()))
}

/// Auto-checkpoint before a shell/execution call mutates the task tree.
/// Best-effort by design: a checkpoint hiccup must not break tool calls.
pub fn auto_checkpoint(store: &crate::store::Store, workspace_path: &str, label: &str) {
    if workspace_path.is_empty() {
        return;
    }
    if let Ok(Some(task)) = store.active_task_workspace_by_root(workspace_path) {
        let _ = checkpoint(&task, &format!("before {label}"));
    }
}

/// Every destructive git/filesystem call on an isolated root passes through
/// here first so a corrupted row can never aim `reset --hard`/`clean -fd` at
/// user data.
fn assert_task_root(isolated_root: &Path, data_dir: &Path) -> Result<PathBuf, String> {
    let canonical_root = canonical(isolated_root)?;
    // The children of an already-canonical data dir compare correctly even
    // when the task-workspaces/task-snapshots folder does not exist yet.
    let data_dir = canonical(data_dir)?;
    if canonical_root.starts_with(data_dir.join("task-workspaces"))
        || canonical_root.starts_with(data_dir.join("task-snapshots"))
    {
        Ok(canonical_root)
    } else {
        Err(
            "Refusing destructive operation: path is not under the app's task workspace directory."
                .into(),
        )
    }
}

fn load_task(store: &crate::store::Store, task_id: &str) -> Result<TaskWorkspace, String> {
    store
        .task_workspace(task_id)?
        .ok_or_else(|| "Task workspace no longer exists.".to_string())
}

fn load_active_task(store: &crate::store::Store, task_id: &str) -> Result<TaskWorkspace, String> {
    let task = load_task(store, task_id)?;
    if task.status != STATUS_ACTIVE {
        return Err("This task workspace is no longer active.".into());
    }
    Ok(task)
}

fn task_branch() -> String {
    let short = &uuid::Uuid::new_v4().simple().to_string()[..8];
    format!("localllm/task-{short}")
}

fn copy_into(src: &Path, dst_root: &Path, relative: &Path) -> Result<(), String> {
    let dst = dst_root.join(relative);
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
    }
    std::fs::copy(src, &dst).map_err(|e| format!("Cannot copy {}: {e}", relative.display()))?;
    Ok(())
}

/// Copy the user's uncommitted/untracked working state into the fresh
/// worktree so the task starts from what the user actually sees.
fn carry_working_state(
    source_root: &Path,
    isolated_root: &Path,
    exclusions: &mut Vec<Exclusion>,
) -> Result<(), String> {
    // Files the user deleted since HEAD must not exist in the task either.
    let deleted = run_git(source_root, &["ls-files", "-d", "-z"])?;
    for entry in deleted.split('\0').filter(|entry| !entry.is_empty()) {
        let relative = safe_relative(entry)?;
        let _ = std::fs::remove_file(isolated_root.join(&relative));
    }
    // `diff HEAD` catches staged-but-uncommitted content that `ls-files -m`
    // misses (worktree == index ≠ HEAD); `ls-files -o` adds untracked files.
    let mut entries: std::collections::BTreeSet<String> = Default::default();
    for output in [
        run_git(source_root, &["diff", "--name-only", "-z", "HEAD"])?,
        run_git(source_root, &["ls-files", "-o", "--exclude-standard", "-z"])?,
    ] {
        entries.extend(
            output
                .split('\0')
                .filter(|entry| !entry.is_empty())
                .map(str::to_string),
        );
    }
    for entry in entries {
        let relative = safe_relative(&entry)?;
        let src = source_root.join(&relative);
        if src.is_symlink() || !src.is_file() {
            continue;
        }
        let size = src.metadata().map(|m| m.len()).unwrap_or(0);
        if let Some(reason) = excluded_reason(&relative, Some(size)) {
            exclusions.push(Exclusion {
                path: entry,
                reason,
            });
            continue;
        }
        copy_into(&src, isolated_root, &relative)?;
    }
    Ok(())
}

/// Bounded copy of a non-git folder into the app data dir.
fn copy_snapshot(
    source_root: &Path,
    isolated_root: &Path,
    exclusions: &mut Vec<Exclusion>,
) -> Result<(), String> {
    let mut stack = vec![source_root.to_path_buf()];
    let mut files = 0usize;
    let mut bytes = 0u64;
    while let Some(dir) = stack.pop() {
        for entry in
            std::fs::read_dir(&dir).map_err(|e| format!("Cannot read {}: {e}", dir.display()))?
        {
            let entry = entry.map_err(|e| format!("Cannot read {}: {e}", dir.display()))?;
            let path = entry.path();
            let relative = path
                .strip_prefix(source_root)
                .map_err(|_| "Snapshot path escaped the source folder.".to_string())?;
            let meta = entry
                .metadata()
                .map_err(|e| format!("Cannot stat {}: {e}", path.display()))?;
            if meta.is_dir() {
                if relative.file_name().and_then(|n| n.to_str()) == Some(".git")
                    || excluded_reason(relative, None).is_some()
                {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if !meta.is_file() {
                exclusions.push(Exclusion {
                    path: relative.to_string_lossy().replace('\\', "/"),
                    reason: "not a regular file (link or device)".to_string(),
                });
                continue;
            }
            files += 1;
            bytes = bytes.saturating_add(meta.len());
            if files > MAX_SNAPSHOT_FILES || bytes > MAX_SNAPSHOT_BYTES {
                return Err(
                    "Workspace is too large to snapshot (over 20,000 files or 500 MiB).".into(),
                );
            }
            if let Some(reason) = excluded_reason(relative, Some(meta.len())) {
                exclusions.push(Exclusion {
                    path: relative.to_string_lossy().replace('\\', "/"),
                    reason,
                });
                continue;
            }
            copy_into(&path, isolated_root, relative)?;
        }
    }
    Ok(())
}

/// The source folder for a task workspace: the conversation's project, falling
/// back to the global workspace root for unfiled conversations.
fn source_for(store: &crate::store::Store, conversation_id: &str) -> Result<String, String> {
    if let Some(path) = crate::workspace_ui::project_workspace(store, conversation_id)? {
        if !path.trim().is_empty() {
            return Ok(path);
        }
    }
    let global = store.workspace_path().unwrap_or_default();
    if global.trim().is_empty() {
        Err("Choose a workspace folder before isolating a task.".into())
    } else {
        Ok(global)
    }
}

pub fn create(
    store: &crate::store::Store,
    data_dir: &Path,
    conversation_id: &str,
) -> Result<TaskWorkspace, String> {
    if !store.conversation_exists(conversation_id)? {
        return Err("Task no longer exists.".into());
    }
    if store.active_task_workspace(conversation_id)?.is_some() {
        return Err(
            "This task already has an isolated workspace. Apply or discard it first.".into(),
        );
    }
    require_git()?;
    let source = canonical(Path::new(&source_for(store, conversation_id)?))?;
    let id = uuid::Uuid::new_v4().to_string();
    let mut exclusions = Vec::new();

    let in_git = run_git(&source, &["rev-parse", "--is-inside-work-tree"])
        .map(|out| out.trim() == "true")
        .unwrap_or(false);
    let has_head = in_git && run_git(&source, &["rev-parse", "--verify", "HEAD"]).is_ok();
    let (kind, branch, isolated_root, baseline_commit) = if has_head {
        // Anchor the source to the repo top so carried paths line up.
        let top = run_git(&source, &["rev-parse", "--show-toplevel"])
            .map(|out| PathBuf::from(out.trim()))
            .ok()
            .and_then(|path| canonical(&path).ok())
            .unwrap_or_else(|| source.clone());
        let branch = task_branch();
        let isolated_root = data_dir.join("task-workspaces").join(&id);
        // Checkout byte-exact: the user's global core.autocrlf must not write
        // CRLF into the worktree or every file would read as changed.
        run_git(
            &top,
            &[
                "-c",
                "core.autocrlf=false",
                "worktree",
                "add",
                "-b",
                &branch,
                &isolated_root.to_string_lossy(),
                "HEAD",
            ],
        )?;
        // A failed carry/commit must not leave a stray worktree + branch.
        let prepared = carry_working_state(&top, &isolated_root, &mut exclusions)
            .and_then(|()| commit_baseline(&isolated_root));
        let baseline = match prepared {
            Ok(baseline) => baseline,
            Err(error) => {
                let _ = run_git(
                    &top,
                    &[
                        "worktree",
                        "remove",
                        "--force",
                        &isolated_root.to_string_lossy(),
                    ],
                );
                let _ = run_git(&top, &["branch", "-D", &branch]);
                return Err(error);
            }
        };
        (
            KIND_WORKTREE.to_string(),
            Some(branch),
            isolated_root,
            baseline,
        )
    } else {
        let isolated_root = data_dir.join("task-snapshots").join(&id);
        std::fs::create_dir_all(&isolated_root)
            .map_err(|e| format!("Cannot create task workspace: {e}"))?;
        copy_snapshot(&source, &isolated_root, &mut exclusions)?;
        run_git_iso(&isolated_root, &["init", "--quiet"])?;
        let baseline = commit_baseline(&isolated_root)?;
        (KIND_SNAPSHOT.to_string(), None, isolated_root, baseline)
    };
    let now = crate::store::now();
    let task = TaskWorkspace {
        id,
        conversation_id: conversation_id.to_string(),
        source_root: source.to_string_lossy().into_owned(),
        kind,
        isolated_root: isolated_root.to_string_lossy().into_owned(),
        branch,
        baseline_commit,
        status: STATUS_ACTIVE.to_string(),
        exclusions,
        created_at: now,
        updated_at: now,
    };
    store.save_task_workspace(&task)?;
    Ok(task)
}

/// Working-tree diff against the task baseline, including untracked files,
/// computed with a temporary index so the real index is never touched.
fn diff_changes(
    root: &Path,
    baseline: &str,
) -> Result<Vec<(String, Option<String>, String)>, String> {
    let index = std::env::temp_dir().join(format!(
        "locallm-task-index-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| {
        run_git_indexed(root, &index, &["read-tree", baseline])?;
        run_git_indexed(root, &index, &["add", "-A"])?;
        run_git_indexed(
            root,
            &index,
            &["diff", "--cached", "--name-status", "-M", baseline],
        )
    })();
    let _ = std::fs::remove_file(&index);
    let output = result?;
    let mut changes = Vec::new();
    for line in output.lines() {
        let mut parts = line.split('\t');
        let Some(code) = parts.next() else { continue };
        if code
            .strip_prefix('R')
            .or_else(|| code.strip_prefix('C'))
            .is_some()
        {
            let Some(old) = parts.next() else { continue };
            let Some(new) = parts.next() else { continue };
            changes.push((
                new.to_string(),
                Some(old.to_string()),
                if code.starts_with('R') {
                    "renamed"
                } else {
                    "modified"
                }
                .to_string(),
            ));
            continue;
        }
        let Some(path) = parts.next() else { continue };
        let status = match code.chars().next() {
            Some('A') => "added",
            Some('M') | Some('T') => "modified",
            Some('D') => "deleted",
            _ => continue,
        };
        changes.push((path.to_string(), None, status.to_string()));
    }
    Ok(changes)
}

fn baseline_content_hash(root: &Path, baseline: &str, relative: &str) -> Option<String> {
    git_bytes(root, &["show", &format!("{baseline}:{relative}")])
        .ok()
        .map(|bytes| comparable_hash(&bytes))
}

pub fn review(store: &crate::store::Store, task_id: &str) -> Result<TaskReview, String> {
    let task = load_task(store, task_id)?;
    let isolated_root = Path::new(&task.isolated_root);
    let source_root = Path::new(&task.source_root);
    let mut files = Vec::new();
    for (path, old_path, status) in diff_changes(isolated_root, &task.baseline_commit)? {
        let isolated_path = isolated_root.join(safe_relative(&path)?);
        let destination = source_root.join(safe_relative(&path)?);
        let baseline_hash = baseline_content_hash(
            isolated_root,
            &task.baseline_commit,
            old_path.as_deref().unwrap_or(&path),
        );
        let task_hash = sha256_file(&isolated_path);
        let destination_hash = comparable_file_hash(&destination);
        let destination_changed = match (&baseline_hash, &destination_hash) {
            (Some(base), Some(dest)) => base != dest,
            // Task added the path and the source gained it too, or the task
            // modified it while the user deleted it — both need a decision.
            (None, Some(_)) => true,
            // The destination is already gone: a delete applies cleanly.
            (Some(_), None) => status != "deleted" && status != "renamed",
            (None, None) => false,
        };
        files.push(TaskFileChange {
            binary: task_hash.is_some() && looks_binary(&isolated_path),
            size: isolated_path.metadata().map(|m| m.len()).unwrap_or(0),
            conflict: destination_changed,
            path,
            old_path,
            status,
            baseline_hash,
            task_hash,
            destination_hash,
            destination_changed,
        });
    }
    Ok(TaskReview {
        task_id: task.id.clone(),
        source_root: task.source_root.clone(),
        isolated_root: task.isolated_root.clone(),
        kind: task.kind.clone(),
        branch: task.branch.clone(),
        baseline_commit: task.baseline_commit.clone(),
        exclusions: task.exclusions.clone(),
        checkpoints: list_checkpoints(&task)?,
        applies: store.task_applies(task_id)?,
        files,
    })
}

fn list_checkpoints(task: &TaskWorkspace) -> Result<Vec<TaskCheckpoint>, String> {
    let root = Path::new(&task.isolated_root);
    let parse = |log: &str| -> Vec<TaskCheckpoint> {
        log.lines()
            .filter_map(|line| {
                let mut parts = line.splitn(3, '\t');
                let commit = parts.next()?.to_string();
                let created_at = parts.next()?.parse().unwrap_or(0);
                let label = parts.next().unwrap_or("checkpoint").to_string();
                Some(TaskCheckpoint {
                    commit: Some(commit),
                    label,
                    created_at,
                    uncommitted: false,
                })
            })
            .collect()
    };
    let mut checkpoints = parse(&run_git_iso(
        root,
        &[
            "log",
            "--format=%H%x09%ct%x09%s",
            &format!("{}..HEAD", task.baseline_commit),
        ],
    )?);
    // A restore moves HEAD back, orphaning the checkpoints it superseded —
    // recover them from the reflog so they stay selectable.
    let mut seen: std::collections::HashSet<String> = checkpoints
        .iter()
        .filter_map(|cp| cp.commit.clone())
        .chain([task.baseline_commit.clone()])
        .collect();
    if let Ok(reflog) = run_git_iso(root, &["reflog", "show", "--format=%H"]) {
        for sha in reflog.lines() {
            if !seen.insert(sha.to_string()) {
                continue;
            }
            if run_git_iso(
                root,
                &["merge-base", "--is-ancestor", &task.baseline_commit, sha],
            )
            .is_err()
            {
                continue;
            }
            if let Ok(line) = run_git_iso(root, &["show", "-s", "--format=%H%x09%ct%x09%s", sha]) {
                checkpoints.extend(parse(&line));
            }
        }
    }
    if run_git_iso(root, &["status", "--porcelain"])
        .map(|out| !out.trim().is_empty())
        .unwrap_or(false)
    {
        checkpoints.push(TaskCheckpoint {
            commit: None,
            label: "Uncommitted changes".to_string(),
            created_at: 0,
            uncommitted: true,
        });
    }
    Ok(checkpoints)
}

pub fn checkpoints(
    store: &crate::store::Store,
    task_id: &str,
) -> Result<Vec<TaskCheckpoint>, String> {
    list_checkpoints(&load_task(store, task_id)?)
}

pub fn file_diff(
    store: &crate::store::Store,
    task_id: &str,
    path: &str,
) -> Result<TaskFileDiff, String> {
    let task = load_task(store, task_id)?;
    let relative = safe_relative(path)?;
    let root = Path::new(&task.isolated_root);
    let file = root.join(&relative);
    if file
        .metadata()
        .map(|meta| meta.len() > DIFF_MAX_FILE_BYTES)
        .unwrap_or(false)
    {
        return Ok(TaskFileDiff {
            text: None,
            truncated: false,
            binary: false,
            reason: Some("File is too large to diff.".to_string()),
        });
    }
    if file.is_file() && looks_binary(&file) {
        return Ok(TaskFileDiff {
            text: None,
            truncated: false,
            binary: true,
            reason: Some("Binary files are not shown.".to_string()),
        });
    }
    let index = std::env::temp_dir().join(format!(
        "locallm-task-index-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| {
        run_git_indexed(root, &index, &["read-tree", &task.baseline_commit])?;
        run_git_indexed(root, &index, &["add", "-A"])?;
        run_git_indexed(
            root,
            &index,
            &["diff", "--cached", "-M", &task.baseline_commit, "--", path],
        )
    })();
    let _ = std::fs::remove_file(&index);
    let text = result?;
    if text.contains("Binary files") {
        return Ok(TaskFileDiff {
            text: None,
            truncated: false,
            binary: true,
            reason: Some("Binary files are not shown.".to_string()),
        });
    }
    let truncated = text.len() > DIFF_TEXT_CAP;
    let text = if truncated {
        let mut boundary = DIFF_TEXT_CAP;
        while !text.is_char_boundary(boundary) {
            boundary -= 1;
        }
        format!("{}\n… diff truncated at 256 KiB", &text[..boundary])
    } else {
        text
    };
    Ok(TaskFileDiff {
        text: Some(text).filter(|text| !text.trim().is_empty()),
        truncated,
        binary: false,
        reason: None,
    })
}

pub fn restore_checkpoint(
    store: &crate::store::Store,
    data_dir: &Path,
    task_id: &str,
    commit: &str,
) -> Result<(), String> {
    let task = load_active_task(store, task_id)?;
    if commit.is_empty() || commit.len() > 128 || commit.starts_with('-') {
        return Err("Invalid checkpoint.".into());
    }
    // Guard first: reset/clean must provably stay inside the app task dirs.
    let root = assert_task_root(Path::new(&task.isolated_root), data_dir)?;
    // The checkpoint must resolve to a commit on this task's own history.
    let resolved = run_git_iso(
        &root,
        &["rev-parse", "--verify", &format!("{commit}^{{commit}}")],
    )
    .map_err(|_| "That checkpoint does not belong to this task.".to_string())?
    .trim()
    .to_string();
    if run_git_iso(&root, &["merge-base", "--is-ancestor", &resolved, "HEAD"]).is_err() {
        return Err("That checkpoint does not belong to this task.".into());
    }
    checkpoint(&task, "before restore")?;
    run_git_iso(&root, &["reset", "--hard", &resolved])?;
    run_git_iso(&root, &["clean", "-fd"])?;
    Ok(())
}

/// Apply the task's changes back to the source folder.
///
/// Every file the apply overwrites or deletes is copied into
/// `task-recovery/<task>/<apply-id>` first, and conflicting files keep the
/// task version beside the backup so neither copy is lost.
pub fn apply(
    store: &crate::store::Store,
    data_dir: &Path,
    task_id: &str,
    paths: Option<Vec<String>>,
    resolutions: &BTreeMap<String, String>,
) -> Result<TaskApplyResult, String> {
    let task = load_active_task(store, task_id)?;
    let root = assert_task_root(Path::new(&task.isolated_root), data_dir)?;
    checkpoint(&task, "before apply")?;
    let review = review(store, task_id)?;
    let selected: Option<std::collections::HashSet<String>> =
        paths.map(|paths| paths.into_iter().collect());
    let apply_id = uuid::Uuid::new_v4().to_string();
    let backup_dir = data_dir.join("task-recovery").join(task_id).join(&apply_id);
    let source_root = Path::new(&task.source_root);
    let mut applied = Vec::new();
    let mut kept_destination = Vec::new();
    let mut skipped_conflicts = Vec::new();
    let mut record = Vec::new();

    for change in &review.files {
        if let Some(selected) = &selected {
            if !selected.contains(&change.path) {
                continue;
            }
        }
        let relative = safe_relative(&change.path)?;
        let destination = source_root.join(&relative);
        let task_file = root.join(&relative);
        // Re-validate the destination right before touching it: the review's
        // hash is only a snapshot and the user may have saved since.
        let current_destination = comparable_file_hash(&destination);
        if current_destination != change.destination_hash {
            skipped_conflicts.push(change.path.clone());
            continue;
        }
        // Any file where the source moved since baseline needs an explicit
        // choice — including deletes, which would erase newer user work.
        if change.destination_changed {
            match resolutions.get(&change.path).map(String::as_str) {
                Some("keep-destination") => {
                    kept_destination.push(change.path.clone());
                    continue;
                }
                Some("take-task") => {}
                _ => {
                    skipped_conflicts.push(change.path.clone());
                    continue;
                }
            }
        }
        match change.status.as_str() {
            "deleted" => {
                if destination.exists() {
                    let backup = backup_file(&backup_dir, "backup", &relative, &destination)?;
                    std::fs::remove_file(&destination)
                        .map_err(|e| format!("Cannot remove {}: {e}", change.path))?;
                    record.push(AppliedFile {
                        path: change.path.clone(),
                        action: "deleted".into(),
                        wrote_sha256: None,
                        backup: Some(backup),
                        task_version: None,
                    });
                    applied.push(change.path.clone());
                }
            }
            status => {
                let backup = if destination.exists() {
                    Some(backup_file(&backup_dir, "backup", &relative, &destination)?)
                } else {
                    None
                };
                // Conflicting files keep the task version next to the backup.
                let task_version = if change.destination_changed {
                    Some(backup_file(&backup_dir, "task", &relative, &task_file)?)
                } else {
                    None
                };
                copy_into(&task_file, source_root, &relative)?;
                if status == "renamed" {
                    if let Some(old_path) = &change.old_path {
                        let old_destination = source_root.join(safe_relative(old_path)?);
                        if old_destination.exists() {
                            let stored = backup_file(
                                &backup_dir,
                                "backup",
                                &safe_relative(old_path)?,
                                &old_destination,
                            )
                            .ok();
                            let _ = std::fs::remove_file(&old_destination);
                            record.push(AppliedFile {
                                path: old_path.clone(),
                                action: "deleted".into(),
                                wrote_sha256: None,
                                backup: stored,
                                task_version: None,
                            });
                        }
                    }
                }
                record.push(AppliedFile {
                    path: change.path.clone(),
                    action: if backup.is_some() {
                        "written"
                    } else {
                        "created"
                    }
                    .into(),
                    wrote_sha256: sha256_file(&destination),
                    backup,
                    task_version,
                });
                applied.push(change.path.clone());
            }
        }
    }

    let apply_record = TaskApply {
        id: apply_id.clone(),
        task_id: task.id.clone(),
        created_at: crate::store::now(),
        backup_dir: backup_dir.to_string_lossy().into_owned(),
        files: record,
        undone_at: None,
    };
    store.save_task_apply(&apply_record)?;
    // A partial apply keeps the task active so conflicts can be resolved and
    // applied later; a clean apply (or deliberate keeps) ends isolation.
    if skipped_conflicts.is_empty() {
        let mut task = task;
        task.status = STATUS_APPLIED.to_string();
        task.updated_at = crate::store::now();
        store.save_task_workspace(&task)?;
        if store.workspace_path().unwrap_or_default() == task.isolated_root {
            let _ = store.save_workspace_path(&task.source_root);
        }
    }
    Ok(TaskApplyResult {
        applied,
        kept_destination,
        skipped_conflicts,
        backup_dir: apply_record.backup_dir,
        apply_id,
    })
}

fn backup_file(
    backup_dir: &Path,
    section: &str,
    relative: &Path,
    source: &Path,
) -> Result<String, String> {
    let relative_string = relative.to_string_lossy().replace('\\', "/");
    let target = backup_dir.join(section).join(&relative_string);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("Cannot create backup folder: {e}"))?;
    }
    std::fs::copy(source, &target)
        .map_err(|e| format!("Cannot back up {}: {e}", relative.display()))?;
    Ok(format!("{section}/{relative_string}"))
}

/// Restore what apply overwrote/deleted — only where the destination still
/// equals what apply wrote, so newer user edits are never clobbered.
pub fn undo_apply(
    store: &crate::store::Store,
    task_id: &str,
    apply_id: &str,
) -> Result<TaskUndoResult, String> {
    let task = load_task(store, task_id)?;
    let apply = store
        .task_apply(apply_id)?
        .filter(|apply| apply.task_id == task_id)
        .ok_or_else(|| "Apply record no longer exists.".to_string())?;
    if apply.undone_at.is_some() {
        return Err("This apply was already undone.".into());
    }
    let source_root = Path::new(&task.source_root);
    let backup_dir = Path::new(&apply.backup_dir);
    let mut restored = Vec::new();
    let mut conflicts = Vec::new();
    for file in &apply.files {
        let relative = safe_relative(&file.path)?;
        let destination = source_root.join(&relative);
        // A file apply created that no longer exists has nothing to restore.
        if file.action == "created" && !destination.exists() {
            restored.push(file.path.clone());
            continue;
        }
        let backup = file.backup.as_ref().map(|path| backup_dir.join(path));
        let unchanged = sha256_file(&destination).as_deref() == file.wrote_sha256.as_deref()
            || (!destination.exists() && file.wrote_sha256.is_none());
        if !unchanged {
            conflicts.push(file.path.clone());
            continue;
        }
        match file.action.as_str() {
            // Apply created the file: undo removes the copy it made.
            "created" => {
                let _ = std::fs::remove_file(&destination);
                restored.push(file.path.clone());
            }
            _ => {
                let Some(backup) = backup else {
                    conflicts.push(file.path.clone());
                    continue;
                };
                copy_into(&backup, source_root, &relative)?;
                restored.push(file.path.clone());
            }
        }
    }
    store.mark_task_apply_undone(apply_id, crate::store::now())?;
    Ok(TaskUndoResult {
        restored,
        conflicts,
    })
}

/// Remove the isolated copy and its task branch. The source checkout is never
/// touched — only `worktree remove` on the task path and `branch -D` on the
/// task branch LocalLM created.
pub fn discard(store: &crate::store::Store, data_dir: &Path, task_id: &str) -> Result<(), String> {
    let mut task = load_task(store, task_id)?;
    let root = assert_task_root(Path::new(&task.isolated_root), data_dir)?;
    if task.kind == KIND_WORKTREE {
        let source = Path::new(&task.source_root);
        if root.exists() {
            run_git(
                source,
                &["worktree", "remove", "--force", &root.to_string_lossy()],
            )?;
        } else {
            let _ = run_git(source, &["worktree", "prune"]);
        }
        if let Some(branch) = &task.branch {
            let exists = run_git(source, &["branch", "--list", branch])
                .map(|out| !out.trim().is_empty())
                .unwrap_or(false);
            if exists {
                run_git(source, &["branch", "-D", branch])?;
            }
        }
    } else if root.exists() {
        std::fs::remove_dir_all(&root).map_err(|e| format!("Cannot remove task workspace: {e}"))?;
    }
    task.status = STATUS_DISCARDED.to_string();
    task.updated_at = crate::store::now();
    store.save_task_workspace(&task)?;
    // If the global workspace still points at the removed root, hand it back
    // to the source folder so the UI terminal/files don't open a dead path.
    if store.workspace_path().unwrap_or_default() == task.isolated_root {
        let _ = store.save_workspace_path(&task.source_root);
    }
    Ok(())
}

// ---- Tauri commands ----

#[tauri::command]
pub fn task_workspace_status(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
) -> Result<Option<TaskWorkspace>, String> {
    state.database()?.active_task_workspace(&conversation_id)
}

#[tauri::command]
pub async fn task_workspace_create(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
) -> Result<TaskWorkspace, String> {
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation.")?;
    let (db_path, data_dir) = (state.db_path.clone(), state.data_dir.clone());
    tokio::task::spawn_blocking(move || {
        let store = crate::store::Store::open(&db_path)?;
        create(&store, &data_dir, &conversation_id)
    })
    .await
    .map_err(|_| "Task workspace setup stopped.".to_string())?
}

#[tauri::command]
pub async fn task_review(
    state: tauri::State<'_, crate::AppState>,
    task_id: String,
) -> Result<TaskReview, String> {
    let db_path = state.db_path.clone();
    tokio::task::spawn_blocking(move || review(&crate::store::Store::open(&db_path)?, &task_id))
        .await
        .map_err(|_| "Task review stopped.".to_string())?
}

#[tauri::command]
pub async fn task_file_diff(
    state: tauri::State<'_, crate::AppState>,
    task_id: String,
    path: String,
) -> Result<TaskFileDiff, String> {
    let db_path = state.db_path.clone();
    tokio::task::spawn_blocking(move || {
        file_diff(&crate::store::Store::open(&db_path)?, &task_id, &path)
    })
    .await
    .map_err(|_| "Task diff stopped.".to_string())?
}

#[tauri::command]
pub async fn task_checkpoints(
    state: tauri::State<'_, crate::AppState>,
    task_id: String,
) -> Result<Vec<TaskCheckpoint>, String> {
    let db_path = state.db_path.clone();
    tokio::task::spawn_blocking(move || {
        checkpoints(&crate::store::Store::open(&db_path)?, &task_id)
    })
    .await
    .map_err(|_| "Task checkpoint listing stopped.".to_string())?
}

#[tauri::command]
pub async fn task_restore_checkpoint(
    state: tauri::State<'_, crate::AppState>,
    task_id: String,
    commit: String,
) -> Result<(), String> {
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation.")?;
    let (db_path, data_dir) = (state.db_path.clone(), state.data_dir.clone());
    tokio::task::spawn_blocking(move || {
        let store = crate::store::Store::open(&db_path)?;
        restore_checkpoint(&store, &data_dir, &task_id, &commit)
    })
    .await
    .map_err(|_| "Task restore stopped.".to_string())?
}

#[tauri::command]
pub async fn task_apply(
    state: tauri::State<'_, crate::AppState>,
    task_id: String,
    paths: Option<Vec<String>>,
    resolutions: Option<BTreeMap<String, String>>,
) -> Result<TaskApplyResult, String> {
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation.")?;
    let (db_path, data_dir) = (state.db_path.clone(), state.data_dir.clone());
    tokio::task::spawn_blocking(move || {
        let store = crate::store::Store::open(&db_path)?;
        apply(
            &store,
            &data_dir,
            &task_id,
            paths,
            &resolutions.unwrap_or_default(),
        )
    })
    .await
    .map_err(|_| "Task apply stopped.".to_string())?
}

#[tauri::command]
pub async fn task_undo_apply(
    state: tauri::State<'_, crate::AppState>,
    task_id: String,
    apply_id: String,
) -> Result<TaskUndoResult, String> {
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation.")?;
    let db_path = state.db_path.clone();
    tokio::task::spawn_blocking(move || {
        undo_apply(&crate::store::Store::open(&db_path)?, &task_id, &apply_id)
    })
    .await
    .map_err(|_| "Task undo stopped.".to_string())?
}

#[tauri::command]
pub async fn task_discard(
    state: tauri::State<'_, crate::AppState>,
    task_id: String,
) -> Result<(), String> {
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation.")?;
    let (db_path, data_dir) = (state.db_path.clone(), state.data_dir.clone());
    tokio::task::spawn_blocking(move || {
        let store = crate::store::Store::open(&db_path)?;
        discard(&store, &data_dir, &task_id)
    })
    .await
    .map_err(|_| "Task discard stopped.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    fn git_ok() -> bool {
        static GIT: OnceLock<bool> = OnceLock::new();
        *GIT.get_or_init(|| {
            std::process::Command::new("git")
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        })
    }

    fn require_git() -> bool {
        if !git_ok() {
            eprintln!("skipping task workspace test: git is not on PATH");
            return false;
        }
        true
    }

    fn git_repo(path: &Path) {
        run_git(path, &["init", "-b", "main"]).unwrap();
        run_git(
            path,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--no-verify",
                "--quiet",
                "--allow-empty",
                "-m",
                "seed",
            ],
        )
        .unwrap();
    }

    fn fixture() -> (
        tempfile::TempDir,
        tempfile::TempDir,
        crate::store::Store,
        String,
    ) {
        let source = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&data.path().join("db.sqlite")).unwrap();
        let conversation = store.create_conversation().unwrap().id;
        (source, data, store, conversation)
    }

    fn create_git_task(
        store: &crate::store::Store,
        data: &Path,
        conversation: &str,
    ) -> TaskWorkspace {
        create(store, data, conversation).unwrap()
    }

    #[test]
    fn worktree_task_reviews_multiple_changes_and_carry_respects_exclusions() {
        if !require_git() {
            return;
        }
        let (source, data, store, conversation) = fixture();
        let root = source.path();
        std::fs::write(root.join("tracked.txt"), "one\n").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/code.rs"), "fn old() {}\n").unwrap();
        std::fs::write(root.join(".gitignore"), "ignored.log\n").unwrap();
        git_repo(root);
        run_git(root, &["add", "-A"]).unwrap();
        run_git(
            root,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--no-verify",
                "--quiet",
                "-m",
                "files",
            ],
        )
        .unwrap();
        // User state to carry: a modified file, an untracked file, a secret.
        std::fs::write(root.join("tracked.txt"), "one\nedited by user\n").unwrap();
        std::fs::write(root.join("notes.md"), "draft\n").unwrap();
        std::fs::write(root.join(".env"), "SECRET=1\n").unwrap();
        let mut index = crate::workspace_ui::WorkspaceIndex::default();
        index.projects.push(crate::workspace_ui::Project {
            id: "p".into(),
            name: "P".into(),
            path: root.to_string_lossy().into_owned(),
        });
        index.tasks.insert(
            conversation.clone(),
            crate::workspace_ui::TaskMeta {
                project_id: Some("p".into()),
                archived: false,
                pinned: false,
            },
        );
        store.save_setting("workspace_index_v1", &index).unwrap();

        let task = create_git_task(&store, data.path(), &conversation);
        assert_eq!(task.kind, KIND_WORKTREE);
        assert_eq!(
            task.branch.as_deref().map(|b| &b[..8.min(b.len())]),
            Some("localllm")
        );
        // Carried user edits are inside the baseline commit; the secret is not.
        assert!(
            std::fs::read_to_string(Path::new(&task.isolated_root).join("tracked.txt"))
                .unwrap()
                .contains("edited by user")
        );
        assert!(
            std::fs::read_to_string(Path::new(&task.isolated_root).join("notes.md"))
                .unwrap()
                .contains("draft")
        );
        assert!(!Path::new(&task.isolated_root).join(".env").exists());
        assert!(task.exclusions.iter().any(|ex| ex.path == ".env"));

        // Agent work inside the isolated tree: modify + add + delete.
        std::fs::write(
            Path::new(&task.isolated_root).join("tracked.txt"),
            "task version\n",
        )
        .unwrap();
        std::fs::write(Path::new(&task.isolated_root).join("new.txt"), "created\n").unwrap();
        std::fs::remove_file(Path::new(&task.isolated_root).join("src/code.rs")).unwrap();

        let review = review(&store, &task.id).unwrap();

        let by_path: std::collections::HashMap<_, _> =
            review.files.iter().map(|f| (f.path.as_str(), f)).collect();
        assert_eq!(by_path["tracked.txt"].status, "modified");
        assert_eq!(by_path["new.txt"].status, "added");
        assert_eq!(by_path["src/code.rs"].status, "deleted");
        assert!(!by_path["new.txt"].destination_changed);
        assert!(by_path.values().all(|file| !file.conflict));
        assert!(review.checkpoints.iter().any(|cp| cp.uncommitted));
    }

    #[test]
    fn apply_conflicts_and_resolutions_preserve_both_versions() {
        if !require_git() {
            return;
        }
        let (source, data, store, conversation) = fixture();
        git_repo(source.path());
        std::fs::write(source.path().join("app.txt"), "v1\n").unwrap();
        run_git(source.path(), &["add", "-A"]).unwrap();
        run_git(
            source.path(),
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--no-verify",
                "--quiet",
                "-m",
                "v1",
            ],
        )
        .unwrap();
        store
            .save_workspace_path(&source.path().to_string_lossy())
            .unwrap();
        let task = create_git_task(&store, data.path(), &conversation);
        std::fs::write(
            Path::new(&task.isolated_root).join("app.txt"),
            "task version\n",
        )
        .unwrap();
        std::fs::write(
            Path::new(&task.isolated_root).join("added.txt"),
            "new file\n",
        )
        .unwrap();
        // Human edits the source while the task runs → conflict.
        std::fs::write(source.path().join("app.txt"), "human version\n").unwrap();

        // Unresolved conflict is skipped.
        let outcome = apply(&store, data.path(), &task.id, None, &BTreeMap::new()).unwrap();
        assert_eq!(outcome.skipped_conflicts, vec!["app.txt".to_string()]);
        assert_eq!(
            std::fs::read_to_string(source.path().join("app.txt")).unwrap(),
            "human version\n"
        );

        // 'take-task' backs up the human version and keeps the task version too.
        let outcome = apply(
            &store,
            data.path(),
            &task.id,
            None,
            &BTreeMap::from([("app.txt".to_string(), "take-task".to_string())]),
        )
        .unwrap();
        assert!(outcome.applied.contains(&"app.txt".to_string()));
        assert_eq!(
            std::fs::read_to_string(source.path().join("app.txt")).unwrap(),
            "task version\n"
        );
        let backup_dir = Path::new(&outcome.backup_dir);
        assert_eq!(
            std::fs::read_to_string(backup_dir.join("backup/app.txt")).unwrap(),
            "human version\n"
        );
        assert_eq!(
            std::fs::read_to_string(backup_dir.join("task/app.txt")).unwrap(),
            "task version\n"
        );
    }

    #[test]
    fn undo_apply_restores_written_and_deleted_and_removes_created() {
        if !require_git() {
            return;
        }
        let (source, data, store, conversation) = fixture();
        git_repo(source.path());
        std::fs::write(source.path().join("keep.txt"), "original\n").unwrap();
        std::fs::write(source.path().join("gone.txt"), "remove me\n").unwrap();
        run_git(source.path(), &["add", "-A"]).unwrap();
        run_git(
            source.path(),
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--no-verify",
                "--quiet",
                "-m",
                "v1",
            ],
        )
        .unwrap();
        store
            .save_workspace_path(&source.path().to_string_lossy())
            .unwrap();
        let task = create_git_task(&store, data.path(), &conversation);
        std::fs::write(Path::new(&task.isolated_root).join("keep.txt"), "edited\n").unwrap();
        std::fs::remove_file(Path::new(&task.isolated_root).join("gone.txt")).unwrap();
        std::fs::write(
            Path::new(&task.isolated_root).join("fresh.txt"),
            "created\n",
        )
        .unwrap();
        let outcome = apply(&store, data.path(), &task.id, None, &BTreeMap::new()).unwrap();
        assert_eq!(
            std::fs::read_to_string(source.path().join("keep.txt")).unwrap(),
            "edited\n"
        );
        assert!(!source.path().join("gone.txt").exists());
        assert!(source.path().join("fresh.txt").exists());

        let undone = undo_apply(&store, &task.id, &outcome.apply_id).unwrap();
        assert_eq!(undone.conflicts.len(), 0);
        assert_eq!(
            std::fs::read_to_string(source.path().join("keep.txt")).unwrap(),
            "original\n"
        );
        assert_eq!(
            std::fs::read_to_string(source.path().join("gone.txt")).unwrap(),
            "remove me\n"
        );
        assert!(!source.path().join("fresh.txt").exists());
        assert!(undo_apply(&store, &task.id, &outcome.apply_id).is_err());

        // Undo refuses to clobber when the destination moved on.
        let task2 = create_git_task(&store, data.path(), &conversation);
        std::fs::write(Path::new(&task2.isolated_root).join("keep.txt"), "again\n").unwrap();
        let outcome = apply(&store, data.path(), &task2.id, None, &BTreeMap::new()).unwrap();
        std::fs::write(source.path().join("keep.txt"), "user rewrote after apply\n").unwrap();
        let undone = undo_apply(&store, &task2.id, &outcome.apply_id).unwrap();
        assert_eq!(undone.conflicts, vec!["keep.txt".to_string()]);
        assert_eq!(
            std::fs::read_to_string(source.path().join("keep.txt")).unwrap(),
            "user rewrote after apply\n"
        );
    }

    #[test]
    fn discard_leaves_the_source_tree_untouched() {
        if !require_git() {
            return;
        }
        let (source, data, store, conversation) = fixture();
        git_repo(source.path());
        std::fs::write(source.path().join("stable.txt"), "v1\n").unwrap();
        run_git(source.path(), &["add", "-A"]).unwrap();
        run_git(
            source.path(),
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--no-verify",
                "--quiet",
                "-m",
                "v1",
            ],
        )
        .unwrap();
        std::fs::write(source.path().join("wip.txt"), "uncommitted user work\n").unwrap();
        store
            .save_workspace_path(&source.path().to_string_lossy())
            .unwrap();
        let task = create_git_task(&store, data.path(), &conversation);
        let branch = task.branch.clone().unwrap();
        std::fs::write(
            Path::new(&task.isolated_root).join("stable.txt"),
            "mutated\n",
        )
        .unwrap();
        discard(&store, data.path(), &task.id).unwrap();
        assert_eq!(
            std::fs::read_to_string(source.path().join("stable.txt")).unwrap(),
            "v1\n"
        );
        assert_eq!(
            std::fs::read_to_string(source.path().join("wip.txt")).unwrap(),
            "uncommitted user work\n"
        );
        // The task branch is gone and no stray worktree remains.
        assert!(run_git(source.path(), &["branch", "--list", &branch])
            .unwrap()
            .trim()
            .is_empty());
        assert!(!Path::new(&task.isolated_root).exists());
        assert_eq!(
            store.task_workspace(&task.id).unwrap().unwrap().status,
            STATUS_DISCARDED
        );
        assert!(store
            .active_task_workspace(&conversation)
            .unwrap()
            .is_none());
    }

    #[test]
    fn two_conversations_get_isolated_roots() {
        if !require_git() {
            return;
        }
        let (source, data, store, conversation_a) = fixture();
        let conversation_b = store.create_conversation().unwrap().id;
        git_repo(source.path());
        store
            .save_workspace_path(&source.path().to_string_lossy())
            .unwrap();
        let task_a = create_git_task(&store, data.path(), &conversation_a);
        let task_b = create_git_task(&store, data.path(), &conversation_b);
        assert_ne!(task_a.isolated_root, task_b.isolated_root);
        std::fs::write(Path::new(&task_a.isolated_root).join("a.txt"), "task a\n").unwrap();
        assert!(!Path::new(&task_b.isolated_root).join("a.txt").exists());
    }

    #[test]
    fn non_git_sources_snapshot_with_bounds() {
        if !require_git() {
            return;
        }
        let (source, data, store, conversation) = fixture();
        std::fs::create_dir_all(source.path().join("src")).unwrap();
        std::fs::write(source.path().join("src/app.rs"), "fn main() {}\n").unwrap();
        std::fs::write(source.path().join(".env"), "SECRET=1\n").unwrap();
        std::fs::create_dir_all(source.path().join("node_modules/lib")).unwrap();
        std::fs::write(source.path().join("node_modules/lib/pkg.js"), "x\n").unwrap();
        store
            .save_workspace_path(&source.path().to_string_lossy())
            .unwrap();
        let task = create_git_task(&store, data.path(), &conversation);
        assert_eq!(task.kind, KIND_SNAPSHOT);
        assert!(task.branch.is_none());
        let iso = Path::new(&task.isolated_root);
        assert!(iso.join("src/app.rs").exists());
        assert!(!iso.join(".env").exists());
        assert!(!iso.join("node_modules/lib/pkg.js").exists());
        assert!(task.exclusions.iter().any(|ex| ex.path == ".env"));
        assert!(!task.baseline_commit.is_empty());
    }

    #[test]
    fn checkpoints_list_and_restore() {
        if !require_git() {
            return;
        }
        let (source, data, store, conversation) = fixture();
        git_repo(source.path());
        store
            .save_workspace_path(&source.path().to_string_lossy())
            .unwrap();
        let task = create_git_task(&store, data.path(), &conversation);
        let iso = Path::new(&task.isolated_root);
        std::fs::write(iso.join("file.txt"), "v1\n").unwrap();
        checkpoint(&task, "first").unwrap();
        let first = run_git(iso, &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        std::fs::write(iso.join("file.txt"), "v2\n").unwrap();
        std::fs::write(iso.join("extra.txt"), "x\n").unwrap();

        let list = checkpoints(&store, &task.id).unwrap();
        assert!(list.iter().any(|cp| cp.label.contains("first")));
        assert!(list.iter().any(|cp| cp.uncommitted));

        restore_checkpoint(&store, data.path(), &task.id, &first).unwrap();
        assert_eq!(
            std::fs::read_to_string(iso.join("file.txt")).unwrap(),
            "v1\n"
        );
        assert!(!iso.join("extra.txt").exists());
        // The pre-restore checkpoint preserved v2.
        assert!(checkpoints(&store, &task.id)
            .unwrap()
            .iter()
            .any(|cp| cp.label.contains("before restore")));
        assert!(
            restore_checkpoint(&store, data.path(), &task.id, "0".repeat(40).as_str()).is_err()
        );
    }

    #[test]
    fn destructive_calls_reject_roots_outside_app_task_dirs() {
        if !require_git() {
            return;
        }
        let (source, data, store, conversation) = fixture();
        git_repo(source.path());
        store
            .save_workspace_path(&source.path().to_string_lossy())
            .unwrap();
        let task = create_git_task(&store, data.path(), &conversation);
        // A corrupted record pointing at the user's own tree must be refused:
        // a fresh row (own id + conversation) carrying the poisoned root.
        let mut rogue = task.clone();
        rogue.id = uuid::Uuid::new_v4().to_string();
        rogue.conversation_id = store.create_conversation().unwrap().id;
        rogue.isolated_root = source.path().to_string_lossy().into_owned();
        store.save_task_workspace(&rogue).unwrap();
        let head = run_git(source.path(), &["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_string();
        assert!(restore_checkpoint(&store, data.path(), &rogue.id, &head).is_err());
        assert!(assert_task_root(&PathBuf::from(&rogue.isolated_root), data.path()).is_err());
        assert!(discard(&store, data.path(), &rogue.id).is_err());
    }

    #[test]
    fn file_diff_handles_binary_and_oversized_and_truncates() {
        if !require_git() {
            return;
        }
        let (source, data, store, conversation) = fixture();
        git_repo(source.path());
        store
            .save_workspace_path(&source.path().to_string_lossy())
            .unwrap();
        let task = create_git_task(&store, data.path(), &conversation);
        let iso = Path::new(&task.isolated_root);
        std::fs::write(iso.join("blob.bin"), b"\x00\x01\x02binary").unwrap();
        std::fs::write(iso.join("text.txt"), "hello\n").unwrap();
        std::fs::write(iso.join("long.txt"), "x".repeat(300 * 1024) + "\n").unwrap();
        std::fs::write(iso.join("huge.txt"), "y".repeat(5 * 1024 * 1024)).unwrap();

        let binary = file_diff(&store, &task.id, "blob.bin").unwrap();
        assert!(binary.binary && binary.text.is_none() && binary.reason.is_some());
        let big = file_diff(&store, &task.id, "long.txt").unwrap();
        assert!(big.truncated && big.text.as_deref().unwrap().contains("truncated"));
        let huge = file_diff(&store, &task.id, "huge.txt").unwrap();
        assert!(!huge.binary && huge.text.is_none() && huge.reason.is_some());
        let text = file_diff(&store, &task.id, "text.txt").unwrap();
        assert!(text.text.unwrap().contains("+hello"));
        assert!(file_diff(&store, &task.id, "../escape.txt").is_err());
    }

    #[test]
    fn tables_exist_on_an_existing_database() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("db.sqlite");
        {
            let store = crate::store::Store::open(&path).unwrap();
            let id = store.create_conversation().unwrap().id;
            store.save_setting("workspace_path", &"C:/ws").unwrap();
            drop(store);
            let store = crate::store::Store::open(&path).unwrap();
            // Reopen (migration path) still exposes both task tables.
            assert!(store.active_task_workspace(&id).unwrap().is_none());
            assert!(store.task_applies(&id).unwrap().is_empty());
        }
    }
}
