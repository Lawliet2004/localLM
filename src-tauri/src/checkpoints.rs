//! Workspace checkpoints and per-turn rollback (docs/EXTENSIONS.md §2.2).
//!
//! Before a turn that can modify the workspace, the harness snapshots the
//! workspace into a *shadow* git repository under app data
//! (`checkpoints/<hash>.git`). It snapshots again when the turn ends. The
//! two snapshots show exactly what the turn changed, including changes made
//! through shell or code execution, which no per-tool journal could see.
//!
//! - The user's own repository is never touched. The shadow repository
//!   is selected with `--git-dir`, and user or system git configuration,
//!   hooks and fsmonitor are disabled for it.
//! - `.gitignore` is respected. Files over [`MAX_FILE_BYTES`] and nested
//!   repositories are excluded, and each exclusion is reported.
//! - Revert restores only the paths that turn changed, and only where the
//!   file still matches what the turn left. A path edited since then is
//!   reported as a conflict and left alone. A revert is itself recorded as
//!   a checkpoint, so it can be reverted too.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const MAX_FILE_BYTES: u64 = 5 * 1024 * 1024;
const MAX_NEW_FILES: usize = 20_000;
const MAX_NEW_BYTES: u64 = 1024 * 1024 * 1024;
const KEEP_PER_WORKSPACE: usize = 100;
const GIT_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CheckpointSettings {
    pub enabled: bool,
}
impl Default for CheckpointSettings {
    fn default() -> Self {
        Self { enabled: true }
    }
}
impl CheckpointSettings {
    pub const KEY: &'static str = "checkpoints";
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Checkpoint {
    pub id: String,
    pub conversation_id: String,
    pub run_id: Option<String>,
    pub workspace: String,
    pub label: String,
    pub before_commit: String,
    pub after_commit: Option<String>,
    pub files_changed: Option<u32>,
    pub excluded: Value,
    pub status: String,
    pub created_at: i64,
}

/// A shadow repository bound to one workspace folder.
pub struct Shadow {
    git_dir: PathBuf,
    workspace: String,
    empty_config: PathBuf,
}

pub fn shadow_dir(data_dir: &Path, workspace: &str) -> PathBuf {
    let key = workspace.replace('\\', "/").trim_end_matches('/').to_lowercase();
    let digest = format!("{:x}", Sha256::digest(key.as_bytes()));
    data_dir.join("checkpoints").join(format!("{}.git", &digest[..20]))
}

impl Shadow {
    pub fn new(data_dir: &Path, workspace: &str) -> Self {
        Self {
            git_dir: shadow_dir(data_dir, workspace),
            workspace: workspace.to_string(),
            empty_config: data_dir.join("checkpoints").join("empty.gitconfig"),
        }
    }

    async fn git(&self, args: &[&str]) -> Result<String, String> {
        let git_dir = format!("--git-dir={}", self.git_dir.display());
        let work_tree = format!("--work-tree={}", self.workspace);
        let hooks = format!("core.hooksPath={}", self.git_dir.join("no-hooks").display());
        let mut full: Vec<&str> = vec![
            git_dir.as_str(), work_tree.as_str(),
            "-c", "core.autocrlf=false", "-c", "core.safecrlf=false", "-c", "core.fsmonitor=false",
            "-c", "core.longpaths=true", "-c", "commit.gpgsign=false", "-c", "gc.auto=0",
            "-c", "safe.directory=*", "-c", hooks.as_str(),
        ];
        full.extend_from_slice(args);
        let config = self.empty_config.to_string_lossy().to_string();
        let envs = [
            ("GIT_CONFIG_GLOBAL", config.as_str()),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_LITERAL_PATHSPECS", "1"),
            ("GIT_AUTHOR_NAME", "LocalLM"),
            ("GIT_AUTHOR_EMAIL", "checkpoints@locallm.invalid"),
            ("GIT_COMMITTER_NAME", "LocalLM"),
            ("GIT_COMMITTER_EMAIL", "checkpoints@locallm.invalid"),
        ];
        crate::git::run(&self.workspace, &full, &envs, GIT_TIMEOUT).await
    }

    async fn ensure(&self) -> Result<(), String> {
        if let Some(parent) = self.empty_config.parent() {
            std::fs::create_dir_all(parent).map_err(|error| format!("Cannot create the checkpoint folder: {error}"))?;
        }
        if !self.empty_config.exists() {
            std::fs::write(&self.empty_config, b"").map_err(|error| error.to_string())?;
        }
        if !self.git_dir.join("HEAD").exists() {
            self.git(&["init", "--quiet"]).await?;
        }
        Ok(())
    }

    /// Record the current workspace state and return the snapshot commit,
    /// kept reachable by `refs/locallm/<ref_name>`.
    pub async fn snapshot(&self, label: &str, ref_name: &str) -> Result<(String, Vec<Value>), String> {
        self.ensure().await?;
        // Modified and deleted files the shadow index already tracks.
        self.git(&["add", "--update"]).await?;
        let others = self.git(&["ls-files", "-z", "--others", "--exclude-standard"]).await?;
        let (keep, excluded) = select_new_files(&others, |path| std::fs::metadata(Path::new(&self.workspace).join(path)).ok().map(|meta| meta.len()))?;
        if !keep.is_empty() {
            let spec = self.git_dir.join("locallm-pathspec");
            std::fs::write(&spec, keep.join("\0")).map_err(|error| error.to_string())?;
            let from = format!("--pathspec-from-file={}", spec.display());
            let added = self.git(&["add", &from, "--pathspec-file-nul"]).await;
            let _ = std::fs::remove_file(&spec);
            added?;
        }
        let tree = self.git(&["write-tree"]).await?.trim().to_string();
        let commit = self.git(&["commit-tree", &tree, "-m", label]).await?.trim().to_string();
        self.git(&["update-ref", &format!("refs/locallm/{ref_name}"), &commit]).await?;
        Ok((commit, excluded))
    }

    pub async fn changes(&self, before: &str, after: &str) -> Result<Vec<(String, String)>, String> {
        Ok(parse_name_status(&self.git(&["diff-tree", "-r", "-z", "--no-renames", "--name-status", before, after]).await?))
    }

    pub async fn diff(&self, before: &str, after: &str) -> Result<(String, bool), String> {
        let text = self.git(&["diff", "--no-ext-diff", "--no-textconv", "--stat", "--patch", before, after]).await?;
        const LIMIT: usize = 262_144;
        if text.len() <= LIMIT {
            return Ok((text, false));
        }
        let mut cut = LIMIT;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        Ok((text[..cut].to_string(), true))
    }

    async fn tree(&self, commit: &str) -> Result<HashMap<String, String>, String> {
        Ok(parse_ls_tree(&self.git(&["ls-tree", "-r", "-z", commit]).await?))
    }

    /// Undo what happened between `before` and `after`, path by path.
    /// Returns (restored, deleted, conflicts). `current` is a snapshot of the
    /// workspace taken just now.
    pub async fn revert(&self, before: &str, after: &str, current: &str) -> Result<(Vec<String>, Vec<String>, Vec<String>), String> {
        let changes = self.changes(before, after).await?;
        let (before_tree, after_tree, current_tree) = (self.tree(before).await?, self.tree(after).await?, self.tree(current).await?);
        let plan = plan_revert(&changes, &before_tree, &after_tree, &current_tree);
        if !plan.restore.is_empty() {
            let spec = self.git_dir.join("locallm-pathspec");
            std::fs::write(&spec, plan.restore.join("\0")).map_err(|error| error.to_string())?;
            let from = format!("--pathspec-from-file={}", spec.display());
            let restored = self.git(&["checkout", before, &from, "--pathspec-file-nul"]).await;
            let _ = std::fs::remove_file(&spec);
            restored?;
        }
        for path in &plan.delete {
            let target = safe_join(&self.workspace, path)?;
            match std::fs::remove_file(&target) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(format!("Could not delete {path}: {error}")),
            }
        }
        Ok((plan.restore, plan.delete, plan.conflicts))
    }

    /// Drop refs for checkpoints no longer kept, then prune their objects.
    pub async fn prune(&self, keep: &[String]) -> Result<usize, String> {
        if !self.git_dir.join("HEAD").exists() {
            return Ok(0);
        }
        let refs = self.git(&["for-each-ref", "--format=%(refname)", "refs/locallm/"]).await?;
        let mut removed = 0;
        for name in refs.lines().filter(|line| !line.is_empty()) {
            let id = name.trim_start_matches("refs/locallm/").split('/').next().unwrap_or("");
            if !keep.iter().any(|kept| kept == id) {
                self.git(&["update-ref", "-d", name]).await?;
                removed += 1;
            }
        }
        if removed > 0 {
            self.git(&["-c", "gc.pruneExpire=now", "gc", "--quiet"]).await?;
        }
        Ok(removed)
    }
}

/// Split `ls-files --others` output into files to snapshot and exclusions.
pub fn select_new_files(raw: &str, size_of: impl Fn(&str) -> Option<u64>) -> Result<(Vec<String>, Vec<Value>), String> {
    let mut keep = Vec::new();
    let mut excluded = Vec::new();
    let mut total: u64 = 0;
    for path in raw.split('\0').filter(|path| !path.is_empty()) {
        if path.ends_with('/') {
            excluded.push(json!({"path": path, "reason": "nested repository"}));
            continue;
        }
        let Some(size) = size_of(path) else { continue };
        if size > MAX_FILE_BYTES {
            excluded.push(json!({"path": path, "reason": "larger than 5 MB", "bytes": size}));
            continue;
        }
        total += size;
        keep.push(path.to_string());
    }
    if keep.len() > MAX_NEW_FILES || total > MAX_NEW_BYTES {
        return Err(format!(
            "The workspace has {} new files ({} MB) not yet checkpointed, over the {MAX_NEW_FILES}-file / 1 GB limit. Add build output to .gitignore or choose a smaller workspace; this turn runs without a checkpoint.",
            keep.len(),
            total / 1024 / 1024
        ));
    }
    Ok((keep, excluded))
}

/// Parse `diff-tree -r -z --name-status` into (status, path).
pub fn parse_name_status(raw: &str) -> Vec<(String, String)> {
    let parts: Vec<&str> = raw.split('\0').filter(|part| !part.is_empty()).collect();
    parts.chunks(2).filter(|pair| pair.len() == 2).map(|pair| (pair[0].to_string(), pair[1].to_string())).collect()
}

/// Parse `ls-tree -r -z` into path -> "mode blob".
pub fn parse_ls_tree(raw: &str) -> HashMap<String, String> {
    raw.split('\0')
        .filter_map(|record| {
            let (meta, path) = record.split_once('\t')?;
            let mut fields = meta.split(' ');
            let mode = fields.next()?;
            let _kind = fields.next()?;
            let object = fields.next()?;
            Some((path.to_string(), format!("{mode} {object}")))
        })
        .collect()
}

#[derive(Debug, Default, PartialEq)]
pub struct RevertPlan {
    pub restore: Vec<String>,
    pub delete: Vec<String>,
    pub conflicts: Vec<String>,
}

/// Decide, per changed path, whether reverting is safe. A path is reverted
/// only when its current state is exactly what the turn left behind.
pub fn plan_revert(
    changes: &[(String, String)],
    before: &HashMap<String, String>,
    after: &HashMap<String, String>,
    current: &HashMap<String, String>,
) -> RevertPlan {
    let mut plan = RevertPlan::default();
    for (_, path) in changes {
        if current.get(path) != after.get(path) {
            plan.conflicts.push(path.clone());
        } else if before.contains_key(path) {
            plan.restore.push(path.clone());
        } else {
            plan.delete.push(path.clone());
        }
    }
    plan
}

fn safe_join(workspace: &str, path: &str) -> Result<PathBuf, String> {
    if path.is_empty() || path.starts_with('/') || path.contains(':') || path.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
        return Err(format!("Refusing to delete an unexpected path: {path}"));
    }
    Ok(Path::new(workspace).join(path))
}

/// Whether the turn offers a tool that can change workspace files.
pub fn can_modify_workspace(tools: &[crate::connectors::AgentTool]) -> bool {
    tools.iter().any(|tool| {
        (matches!(tool.connector.as_str(), "Workspace" | "Local execution") && !tool.trusted_read())
            || tool.local_server_name().is_some()
            || tool.harness_name().is_some_and(|name| {
                matches!(name, "terminal_create" | "terminal_send" | "ptc_run" | "docker_exec" | "git_commit" | "subagent" | "workflow_run" | "ralph_run")
            })
    })
}

fn settings(state: &crate::AppState) -> CheckpointSettings {
    state.database().ok().and_then(|store| store.setting(CheckpointSettings::KEY).ok()).unwrap_or_default()
}

/// Before a turn: snapshot the workspace. Returns the checkpoint id (when
/// one was opened) and the `checkpoint` event payload to log.
pub async fn begin_turn(
    state: &crate::AppState,
    tools: &[crate::connectors::AgentTool],
    conversation_id: &str,
    run_id: &str,
) -> (Option<String>, Option<Value>) {
    if !settings(state).enabled || !can_modify_workspace(tools) {
        return (None, None);
    }
    let workspace = match state.database().and_then(|store| store.workspace_path()) {
        Ok(path) if !path.is_empty() => path,
        _ => return (None, None),
    };
    let id = uuid::Uuid::new_v4().to_string();
    let shadow = Shadow::new(&state.data_dir, &workspace);
    match shadow.snapshot("before turn", &format!("{id}/before")).await {
        Ok((commit, excluded)) => {
            let checkpoint = Checkpoint {
                id: id.clone(), conversation_id: conversation_id.to_string(), run_id: Some(run_id.to_string()),
                workspace, label: "turn".into(), before_commit: commit, after_commit: None, files_changed: None,
                excluded: json!(excluded), status: "open".into(), created_at: crate::store::now(),
            };
            let saved = state.database().and_then(|store| store.insert_checkpoint(&checkpoint));
            if let Err(error) = saved {
                return (None, Some(json!({"phase": "before", "outcome": "failed", "error": error})));
            }
            (Some(id.clone()), Some(json!({"phase": "before", "outcome": "snapshot", "checkpointId": id, "excluded": excluded.len()})))
        }
        Err(error) => (None, Some(json!({"phase": "before", "outcome": "skipped", "reason": error}))),
    }
}

/// After a turn, whatever its outcome (a failed turn may still have written
/// files): snapshot again and record what changed. Checkpoints with no
/// changes are dropped.
pub async fn end_turn(state: &crate::AppState, checkpoint_id: &str) -> Option<Value> {
    let checkpoint = state.database().ok()?.checkpoint(checkpoint_id).ok()??;
    let shadow = Shadow::new(&state.data_dir, &checkpoint.workspace);
    let result = async {
        let (after, _) = shadow.snapshot("after turn", &format!("{checkpoint_id}/after")).await?;
        let changes = shadow.changes(&checkpoint.before_commit, &after).await?;
        Ok::<_, String>((after, changes.len() as u32))
    }
    .await;
    let payload = match result {
        Ok((_, 0)) => {
            if let Ok(store) = state.database() {
                let _ = store.delete_checkpoint(checkpoint_id);
            }
            json!({"phase": "after", "outcome": "unchanged", "checkpointId": checkpoint_id})
        }
        Ok((after, files)) => {
            if let Ok(store) = state.database() {
                let _ = store.complete_checkpoint(checkpoint_id, &after, files);
            }
            json!({"phase": "after", "outcome": "recorded", "checkpointId": checkpoint_id, "filesChanged": files})
        }
        Err(error) => json!({"phase": "after", "outcome": "failed", "checkpointId": checkpoint_id, "error": error}),
    };
    retain(state, &shadow, &checkpoint.workspace).await;
    Some(payload)
}

async fn retain(state: &crate::AppState, shadow: &Shadow, workspace: &str) {
    let keep = {
        let Ok(store) = state.database() else { return };
        let Ok(all) = store.checkpoints_for_workspace(workspace) else { return };
        let (kept, dropped) = all.split_at(all.len().min(KEEP_PER_WORKSPACE));
        for checkpoint in dropped {
            let _ = store.delete_checkpoint(&checkpoint.id);
        }
        kept.iter().map(|checkpoint| checkpoint.id.clone()).collect::<Vec<_>>()
    };
    let _ = shadow.prune(&keep).await;
}

#[tauri::command]
pub async fn list_checkpoints(state: tauri::State<'_, crate::AppState>, conversation_id: String) -> Result<Vec<Checkpoint>, String> {
    state.database()?.checkpoints_for_conversation(&conversation_id)
}

#[tauri::command]
pub async fn checkpoint_diff(state: tauri::State<'_, crate::AppState>, id: String) -> Result<Value, String> {
    let checkpoint = state.database()?.checkpoint(&id)?.ok_or("Checkpoint not found.")?;
    let after = checkpoint.after_commit.clone().ok_or("This checkpoint has not been completed.")?;
    let shadow = Shadow::new(&state.data_dir, &checkpoint.workspace);
    let changes = shadow.changes(&checkpoint.before_commit, &after).await?;
    let (diff, truncated) = shadow.diff(&checkpoint.before_commit, &after).await?;
    Ok(json!({
        "changes": changes.iter().map(|(status, path)| json!({"status": status, "path": path})).collect::<Vec<_>>(),
        "diff": diff, "truncated": truncated, "excluded": checkpoint.excluded,
    }))
}

#[tauri::command]
pub async fn revert_checkpoint(state: tauri::State<'_, crate::AppState>, id: String) -> Result<Value, String> {
    let _operation = state.operation.try_lock().map_err(|_| "Stop the active response before reverting changes.")?;
    let checkpoint = state.database()?.checkpoint(&id)?.ok_or("Checkpoint not found.")?;
    let after = checkpoint.after_commit.clone().ok_or("This checkpoint has not been completed.")?;
    let shadow = Shadow::new(&state.data_dir, &checkpoint.workspace);
    // The revert is a checkpoint of its own, so it can be undone as well.
    let revert_id = uuid::Uuid::new_v4().to_string();
    let (current, _) = shadow.snapshot("before revert", &format!("{revert_id}/before")).await?;
    let (restored, deleted, conflicts) = shadow.revert(&checkpoint.before_commit, &after, &current).await?;
    let (post, _) = shadow.snapshot("after revert", &format!("{revert_id}/after")).await?;
    let changed = (restored.len() + deleted.len()) as u32;
    {
        let store = state.database()?;
        if changed > 0 {
            store.insert_checkpoint(&Checkpoint {
                id: revert_id.clone(), conversation_id: checkpoint.conversation_id.clone(), run_id: None,
                workspace: checkpoint.workspace.clone(), label: format!("revert of {}", &checkpoint.id[..8.min(checkpoint.id.len())]),
                before_commit: current, after_commit: None, files_changed: None, excluded: json!([]),
                status: "open".into(), created_at: crate::store::now(),
            })?;
            store.complete_checkpoint(&revert_id, &post, changed)?;
        }
        if conflicts.is_empty() {
            store.set_checkpoint_status(&id, "reverted")?;
        } else if changed > 0 {
            store.set_checkpoint_status(&id, "partially_reverted")?;
        }
    }
    Ok(json!({"restored": restored, "deleted": deleted, "conflicts": conflicts, "revertCheckpointId": (changed > 0).then_some(revert_id)}))
}

#[tauri::command]
pub async fn checkpoint_settings(state: tauri::State<'_, crate::AppState>) -> Result<CheckpointSettings, String> {
    state.database()?.setting(CheckpointSettings::KEY)
}

#[tauri::command]
pub async fn save_checkpoint_settings(state: tauri::State<'_, crate::AppState>, settings: CheckpointSettings) -> Result<CheckpointSettings, String> {
    state.database()?.save_setting(CheckpointSettings::KEY, &settings)?;
    if !settings.enabled {
        clear(&state)?;
    }
    Ok(settings)
}

#[tauri::command]
pub async fn clear_checkpoints(state: tauri::State<'_, crate::AppState>) -> Result<(), String> {
    let _operation = state.operation.try_lock().map_err(|_| "Stop the active response before deleting checkpoints.")?;
    clear(&state)
}

/// Delete every checkpoint row and shadow repository. The shadow
/// repositories hold copies of workspace files.
fn clear(state: &crate::AppState) -> Result<(), String> {
    state.database()?.delete_all_checkpoints()?;
    let dir = state.data_dir.join("checkpoints");
    match std::fs::remove_dir_all(&dir) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Could not delete checkpoint data: {error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_files_exclude_nested_repos_and_large_files() {
        let raw = "src/a.rs\0nested/\0big.bin\0gone.txt\0";
        let (keep, excluded) = select_new_files(raw, |path| match path {
            "src/a.rs" => Some(10),
            "big.bin" => Some(MAX_FILE_BYTES + 1),
            _ => None,
        })
        .unwrap();
        assert_eq!(keep, vec!["src/a.rs".to_string()]);
        assert_eq!(excluded.len(), 2);
        assert_eq!(excluded[0]["reason"], "nested repository");
        let many: String = (0..=MAX_NEW_FILES).map(|index| format!("f{index}\0")).collect();
        assert!(select_new_files(&many, |_| Some(1)).is_err());
    }

    #[test]
    fn parses_git_plumbing_output() {
        // Captured from git 2.49.
        let changes = parse_name_status("D\0b [x].txt\0M\0src/a.txt\0A\0src/new.txt\0");
        assert_eq!(changes, vec![("D".into(), "b [x].txt".into()), ("M".into(), "src/a.txt".into()), ("A".into(), "src/new.txt".into())]);
        let tree = parse_ls_tree("100644 blob 567609b1234a9b8806c5a05da6c866e480aa148d\t.gitignore\0100644 blob c1827f07e114c20547dc6a7296588870a4b5b62c\tsrc/a b.txt\0");
        assert_eq!(tree["src/a b.txt"], "100644 c1827f07e114c20547dc6a7296588870a4b5b62c");
    }

    #[test]
    fn revert_skips_paths_changed_since_the_turn() {
        let map = |pairs: &[(&str, &str)]| pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>();
        let before = map(&[("kept.txt", "1"), ("edited.txt", "1"), ("removed.txt", "1")]);
        let after = map(&[("kept.txt", "2"), ("edited.txt", "2"), ("added.txt", "9")]);
        let current = map(&[("kept.txt", "2"), ("edited.txt", "3"), ("added.txt", "9")]);
        let changes: Vec<(String, String)> = ["kept.txt", "edited.txt", "removed.txt", "added.txt"].iter().map(|p| ("M".to_string(), p.to_string())).collect();
        let plan = plan_revert(&changes, &before, &after, &current);
        assert_eq!(plan.restore, vec!["kept.txt".to_string(), "removed.txt".to_string()]);
        assert_eq!(plan.delete, vec!["added.txt".to_string()]);
        assert_eq!(plan.conflicts, vec!["edited.txt".to_string()]);
    }

    #[test]
    fn deletion_paths_stay_inside_the_workspace() {
        assert!(safe_join("C:/ws", "src/a.txt").is_ok());
        for bad in ["../x", "/abs", "C:/x", "a//b", "a/./b", ""] {
            assert!(safe_join("C:/ws", bad).is_err(), "{bad}");
        }
    }

    #[tokio::test]
    async fn snapshot_revert_round_trip_leaves_the_user_repository_alone() {
        if crate::git::run(".", &["--version"], &[], Duration::from_secs(10)).await.is_err() {
            return; // Git is optional on build machines.
        }
        let data = tempfile::tempdir().unwrap();
        let ws_dir = tempfile::tempdir().unwrap();
        let ws = ws_dir.path().to_string_lossy().to_string();
        std::fs::write(ws_dir.path().join("keep.txt"), "v1\n").unwrap();
        std::fs::write(ws_dir.path().join("edit.txt"), "v1\n").unwrap();
        std::fs::write(ws_dir.path().join(".gitignore"), "build/\n").unwrap();
        std::fs::create_dir(ws_dir.path().join("build")).unwrap();
        std::fs::write(ws_dir.path().join("build").join("out.o"), "x").unwrap();
        let shadow = Shadow::new(data.path(), &ws);
        let (before, _) = shadow.snapshot("before", "t/before").await.unwrap();
        // The "turn": modify, delete, create.
        std::fs::write(ws_dir.path().join("keep.txt"), "v2\n").unwrap();
        std::fs::remove_file(ws_dir.path().join("edit.txt")).unwrap();
        std::fs::write(ws_dir.path().join("new.txt"), "n\n").unwrap();
        let (after, _) = shadow.snapshot("after", "t/after").await.unwrap();
        let changes = shadow.changes(&before, &after).await.unwrap();
        assert_eq!(changes.len(), 3);
        assert!(changes.iter().all(|(_, path)| !path.starts_with("build/")));
        // The user edits new.txt after the turn: that path must be kept.
        std::fs::write(ws_dir.path().join("new.txt"), "user\n").unwrap();
        let (current, _) = shadow.snapshot("now", "r/before").await.unwrap();
        let (restored, deleted, conflicts) = shadow.revert(&before, &after, &current).await.unwrap();
        assert_eq!(std::fs::read_to_string(ws_dir.path().join("keep.txt")).unwrap(), "v1\n");
        assert_eq!(std::fs::read_to_string(ws_dir.path().join("edit.txt")).unwrap(), "v1\n");
        assert_eq!(std::fs::read_to_string(ws_dir.path().join("new.txt")).unwrap(), "user\n");
        assert_eq!((restored.len(), deleted.len(), conflicts), (2, 0, vec!["new.txt".to_string()]));
        assert!(!ws_dir.path().join(".git").exists(), "the workspace must not gain a repository");
        assert_eq!(shadow.prune(&["t".to_string()]).await.unwrap(), 1);
    }
}
