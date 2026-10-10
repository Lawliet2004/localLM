//! Desktop workspace operations. Metadata lives in the existing SQLite settings store.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project { pub id: String, pub name: String, pub path: String }
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskMeta { pub project_id: Option<String>, pub archived: bool, pub pinned: bool }
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceIndex { pub projects: Vec<Project>, pub tasks: BTreeMap<String, TaskMeta> }

pub(crate) fn index(store: &crate::store::Store) -> Result<WorkspaceIndex, String> {
    let mut data: WorkspaceIndex = store.setting("workspace_index_v1")?;
    // Chats always file under a folder: a conversation whose task meta is
    // missing or points at a deleted project is healed into the first project
    // on the next index read (covers script-created and otherwise unfiled rows).
    if let Some(project) = data.projects.first() {
        let project_ids: HashSet<_> = data.projects.iter().map(|project| project.id.as_str()).collect();
        let mut healed = false;
        for conversation in store.list_conversations()? {
            let filed = data
                .tasks
                .get(&conversation.id)
                .and_then(|task| task.project_id.as_ref())
                .is_some_and(|id| project_ids.contains(id.as_str()));
            if !filed {
                data.tasks.entry(conversation.id).or_default().project_id = Some(project.id.clone());
                healed = true;
            }
        }
        if healed {
            store.save_setting("workspace_index_v1", &data)?;
        }
    }
    Ok(data)
}

/// File a conversation under `preferred` when it is still a project, otherwise
/// the first project. No-op when no project exists or the conversation is gone.
pub fn file_task(store: &crate::store::Store, id: &str, preferred: Option<&str>) -> Result<(), String> {
    if !store.conversation_exists(id)? {
        return Ok(());
    }
    let mut data = index(store)?;
    let project = preferred
        .filter(|pid| data.projects.iter().any(|p| &p.id == pid))
        .or_else(|| data.projects.first().map(|p| p.id.as_str()));
    if let Some(project) = project {
        let project = project.to_string();
        data.tasks.entry(id.to_string()).or_default().project_id = Some(project);
        store.save_setting("workspace_index_v1", &data)?;
    }
    Ok(())
}

/// A fork files next to its source: same folder, flags preserved.
pub fn inherit_task_meta(store: &crate::store::Store, source_id: &str, new_id: &str) -> Result<(), String> {
    let mut data = index(store)?;
    let mut meta = data.tasks.get(source_id).cloned().unwrap_or_default();
    let valid = meta
        .project_id
        .as_ref()
        .is_some_and(|id| data.projects.iter().any(|p| &p.id == id));
    if !valid {
        meta.project_id = data.projects.first().map(|p| p.id.clone());
    }
    data.tasks.insert(new_id.to_string(), meta);
    store.save_setting("workspace_index_v1", &data)
}

/// The conversation's project folder, ignoring any isolated task workspace.
pub fn project_workspace(store: &crate::store::Store, id: &str) -> Result<Option<String>, String> {
    let data = index(store)?;
    let project = data.tasks.get(id).and_then(|t| t.project_id.as_ref());
    Ok(data.projects.iter().find(|p| Some(&p.id) == project).map(|p| p.path.clone()))
}

/// The workspace root every tool/terminal/execution path should resolve for
/// this conversation: its isolated task root while one is active, otherwise
/// the project folder.
pub fn task_workspace(store: &crate::store::Store, id: &str) -> Result<Option<String>, String> {
    if let Some(task) = store.active_task_workspace(id)? {
        return Ok(Some(task.isolated_root));
    }
    project_workspace(store, id)
}

#[tauri::command]
pub fn workspace_index(state: tauri::State<'_, crate::AppState>) -> Result<WorkspaceIndex, String> {
    index(&*state.database()?)
}

#[tauri::command]
pub async fn save_project(state: tauri::State<'_, crate::AppState>, mut project: Project) -> Result<WorkspaceIndex, String> {
    let _guard = state.operation.try_lock().map_err(|_| "Wait for the active operation.")?;
    if project.name.trim().is_empty() || project.name.len() > 120 { return Err("Project name must be 1–120 characters.".into()); }
    let path = std::fs::canonicalize(&project.path).map_err(|e| format!("Cannot open project: {e}"))?;
    crate::workspace::Workspace::open(&path.to_string_lossy())?;
    project.path = path.to_string_lossy().into_owned();
    let store = state.database()?;
    let mut data = index(&store)?;
    if data.projects.iter().any(|p| p.path == project.path && p.id != project.id) { return Err("This folder is already a project.".into()); }
    if project.id.is_empty() { project.id = uuid::Uuid::new_v4().to_string(); }
    data.projects.retain(|p| p.id != project.id);
    data.projects.push(project);
    store.save_setting("workspace_index_v1", &data)?;
    Ok(data)
}

#[tauri::command]
pub async fn remove_project(state: tauri::State<'_, crate::AppState>, id: String) -> Result<WorkspaceIndex, String> {
    let _guard = state.operation.try_lock().map_err(|_| "Wait for the active operation.")?;
    let store = state.database()?;
    let mut data = index(&store)?;
    data.projects.retain(|p| p.id != id);
    for task in data.tasks.values_mut() { if task.project_id.as_deref() == Some(&id) { task.project_id = None; } }
    store.save_setting("workspace_index_v1", &data)?;
    Ok(data)
}

pub fn remove_task_meta(store: &crate::store::Store, id: &str) -> Result<(), String> {
    let mut data = index(store)?;
    if data.tasks.remove(id).is_some() { store.save_setting("workspace_index_v1", &data)?; }
    Ok(())
}

#[tauri::command]
pub async fn save_task_meta(state: tauri::State<'_, crate::AppState>, id: String, meta: TaskMeta) -> Result<WorkspaceIndex, String> {
    let _guard = state.operation.try_lock().map_err(|_| "Wait for the active operation.")?;
    let store = state.database()?;
    if !store.conversation_exists(&id)? { return Err("Task no longer exists.".into()); }
    let mut data = index(&store)?;
    if let Some(project) = &meta.project_id {
        if !data.projects.iter().any(|p| &p.id == project) { return Err("Project no longer exists.".into()); }
    }
    data.tasks.insert(id, meta);
    store.save_setting("workspace_index_v1", &data)?;
    Ok(data)
}

fn normalize_root(path: &str) -> String {
    path.trim().trim_end_matches(['/', '\\']).replace('\\', "/")
}

/// Bind a search or directory load to the workspace that issued it.
/// A request names its project or root. The live global workspace is not used
/// when the request still names another open project.
pub fn resolve_search_root(
    index: &WorkspaceIndex,
    saved_root: &str,
    requested_id: Option<&str>,
    requested_root: Option<&str>,
) -> Result<(String, String, u64), String> {
    let saved = normalize_root(saved_root);
    if let Some(id) = requested_id.filter(|id| !id.is_empty()) {
        let project = index.projects.iter().find(|project| project.id == id)
            .ok_or("That workspace is no longer open.")?;
        if let Some(root) = requested_root.filter(|root| !root.is_empty()) {
            if normalize_root(root) != normalize_root(&project.path) {
                return Err("Workspace id does not match the requested folder.".into());
            }
        }
        return Ok((project.id.clone(), project.path.clone(), 1));
    }
    if let Some(root) = requested_root.filter(|root| !root.is_empty()) {
        let wanted = normalize_root(root);
        if let Some(project) = index.projects.iter().find(|project| normalize_root(&project.path) == wanted) {
            return Ok((project.id.clone(), project.path.clone(), 1));
        }
        if !saved.is_empty() && wanted == saved {
            return Ok(("workspace".into(), saved_root.to_string(), 1));
        }
        return Err("Workspace is not open.".into());
    }
    if saved_root.trim().is_empty() {
        return Err("Select a workspace folder first.".into());
    }
    Ok(("workspace".into(), saved_root.to_string(), 1))
}

fn stamp(mut value: Value, id: &str, root: &str, revision: u64) -> Value {
    value["workspaceId"] = json!(id);
    value["root"] = json!(root);
    value["workspaceRevision"] = json!(revision);
    value
}

async fn read_workspace(
    state: &crate::AppState,
    tool: &'static str,
    arguments: Value,
    workspace_id: Option<String>,
    root: Option<String>,
) -> Result<Value, String> {
    let (id, bound, revision) = {
        let store = state.database()?;
        resolve_search_root(&index(&store)?, &store.workspace_path()?, workspace_id.as_deref(), root.as_deref())?
    };
    // Directory traversal can take seconds. Keep it off the IPC/runtime thread
    // and release the shared database before touching the filesystem.
    tokio::task::spawn_blocking(move || {
        let value = crate::workspace::Workspace::open(&bound)?.call(tool, arguments)?;
        Ok(stamp(value, &id, &bound, revision))
    }).await.map_err(|_| "Workspace reader stopped.".to_string())?
}

#[tauri::command]
pub async fn workspace_inspect(
    state: tauri::State<'_, crate::AppState>,
    path: String,
    directory: bool,
    workspace_id: Option<String>,
    root: Option<String>,
) -> Result<Value, String> {
    read_workspace(&state,
        if directory { "list_files" } else { "read_file" },
        if directory { json!({"path": path}) } else { json!({"path": path, "line_count": 500}) },
        workspace_id, root,
    ).await
}

#[tauri::command]
pub async fn workspace_search(
    state: tauri::State<'_, crate::AppState>,
    query: String,
    workspace_id: Option<String>,
    root: Option<String>,
) -> Result<Value, String> {
    read_workspace(&state, "search_files", json!({"query": query}), workspace_id, root).await
}

async fn git(root: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("-C").arg(root).args(args).kill_on_drop(true)
        .stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    #[cfg(windows)] cmd.creation_flags(0x08000000);
    let mut child = cmd.spawn().map_err(|e| format!("Cannot run Git: {e}"))?;
    let stdout = child.stdout.take().ok_or("Git output unavailable.")?;
    let stderr = child.stderr.take().ok_or("Git diagnostics unavailable.")?;
    let (stdout, stderr, status) = tokio::time::timeout(std::time::Duration::from_secs(15), async {
        tokio::try_join!(capture_git(stdout, 400_000), capture_git(stderr, 8_000), child.wait())
    }).await
        .map_err(|_| "Git timed out.")?.map_err(|e| format!("Cannot run Git: {e}"))?;
    if !status.success() { return Err(String::from_utf8_lossy(&stderr).chars().take(2000).collect()); }
    Ok(String::from_utf8_lossy(&stdout).chars().take(100_000).collect())
}

async fn capture_git<R: tokio::io::AsyncRead + Unpin>(mut reader: R, limit: u64) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut bytes = Vec::new();
    (&mut reader).take(limit).read_to_end(&mut bytes).await?;
    // Keep draining both pipes so a large diff cannot deadlock the process.
    tokio::io::copy(&mut reader, &mut tokio::io::sink()).await?;
    Ok(bytes)
}

#[tauri::command]
pub async fn workspace_git(state: tauri::State<'_, crate::AppState>, branch: Option<String>) -> Result<Value, String> {
    let _guard = state.operation.try_lock().map_err(|_| "Wait for the active operation.")?;
    let root = state.database()?.workspace_path()?;
    if root.is_empty() { return Err("Select a workspace folder first.".into()); }
    if let Some(branch) = branch {
        if branch.starts_with('-') || branch.len() > 240 || branch.trim().is_empty() { return Err("Invalid branch.".into()); }
        git(&root, &["check-ref-format", "--branch", &branch]).await?;
        // Never force checkout, reset, or discard changes.
        git(&root, &["switch", &branch]).await?;
    }
    git_snapshot(&root).await
}

async fn git_snapshot(root: &str) -> Result<Value, String> {
    // Working tree versus HEAD covers staged and unstaged edits; a repository
    // without commits has no HEAD, so fall back to the unstaged diff there.
    let diff = async {
        match git(root, &["diff", "--no-ext-diff", "--no-color", "HEAD"]).await {
            Ok(diff) => Ok(diff),
            Err(_) => git(root, &["diff", "--no-ext-diff", "--no-color"]).await,
        }
    };
    let (diff, branch, branches, status) = tokio::try_join!(
        diff,
        git(root, &["branch", "--show-current"]),
        git(root, &["for-each-ref", "--format=%(refname:short)", "refs/heads/"]),
        git(root, &["status", "--short"]),
    )?;
    Ok(json!({"branch":branch.trim(),
        "branches":branches.lines().collect::<Vec<_>>(),
        "status":status,
        "diff":diff}))
}



#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn git_capture_drains_large_output_and_keeps_only_the_bounded_prefix() {
        use tokio::io::AsyncWriteExt;
        let (mut writer, reader) = tokio::io::duplex(16);
        let producer = tokio::spawn(async move { writer.write_all(&vec![b'x'; 4096]).await.unwrap(); });
        let bytes = tokio::time::timeout(std::time::Duration::from_secs(2), capture_git(reader, 32)).await.unwrap().unwrap();
        producer.await.unwrap();
        assert_eq!(bytes, vec![b'x'; 32]);
    }

    #[tokio::test]
    async fn git_snapshot_preserves_branch_status_diff_truncation_and_diagnostics() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_str().unwrap();
        git(root, &["init", "-b", "main"]).await.unwrap();
        let path = directory.path().join("sample.txt");
        std::fs::write(&path, "before\n").unwrap();
        git(root, &["add", "sample.txt"]).await.unwrap();
        git(root, &["-c", "user.name=LocalLM Test", "-c", "user.email=test@example.invalid", "commit", "--no-verify", "-m", "fixture"]).await.unwrap();
        git(root, &["branch", "other"]).await.unwrap();
        std::fs::write(path, "界".repeat(150_000)).unwrap();
        let value = git_snapshot(root).await.unwrap();
        assert_eq!(value["branch"], "main");
        assert_eq!(value["branches"], json!(["main", "other"]));
        assert_eq!(value["status"], " M sample.txt\n");
        let mut command = tokio::process::Command::new("git");
        command.arg("-C").arg(root).args(["diff", "--no-ext-diff", "--no-color", "HEAD"]);
        #[cfg(windows)] command.creation_flags(0x08000000);
        let full = command.output().await.unwrap();
        let expected: String = String::from_utf8_lossy(&full.stdout).chars().take(100_000).collect();
        assert_eq!(value["diff"], expected);
        assert_eq!(expected.chars().count(), 100_000);
        assert!(git(root, &["show", "missing-ref"]).await.unwrap_err().contains("missing-ref"));
    }

    #[tokio::test]
    async fn workspace_reader_keeps_project_binding_and_releases_the_database() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("project");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("sample.txt"), "audit needle\n").unwrap();
        let state = crate::web_search::tests::test_state(temp.path());
        let data = WorkspaceIndex { projects: vec![Project {
            id: "bound".into(), name: "Bound".into(), path: root.to_string_lossy().into_owned(),
        }], tasks: Default::default() };
        state.database().unwrap().save_setting("workspace_index_v1", &data).unwrap();
        let search = read_workspace(&state, "search_files", json!({"query":"sample"}), Some("bound".into()), None);
        tokio::pin!(search);
        let value = tokio::select! {
            result = &mut search => result.unwrap(),
            _ = tokio::task::yield_now() => {
                assert!(state.store.try_lock().is_ok(), "filesystem work must not hold the chat database");
                search.await.unwrap()
            },
        };
        assert_eq!(value["workspaceId"], "bound");
        assert_eq!(value["root"], data.projects[0].path);
        assert!(value.to_string().contains("sample.txt"));
        assert!(read_workspace(&state, "read_file", json!({"path":"../outside.txt"}), Some("bound".into()), None).await.is_err());
    }

    #[test]
    fn project_association_survives_reopen_and_isolated_tasks_do_not_inherit_it() {
        let temp = tempfile::tempdir().unwrap();
        let db = temp.path().join("workspace.sqlite");
        let id;
        {
            let store = crate::store::Store::open(&db).unwrap();
            id = store.create_conversation().unwrap().id;
            let mut data = WorkspaceIndex::default();
            data.projects.push(Project { id: "p".into(), name: "Project".into(), path: "C:/project".into() });
            data.tasks.insert(id.clone(), TaskMeta { project_id: Some("p".into()), archived: true, pinned: true });
            store.save_setting("workspace_index_v1", &data).unwrap();
        }
        let store = crate::store::Store::open(&db).unwrap();
        assert_eq!(task_workspace(&store, &id).unwrap().as_deref(), Some("C:/project"));
        assert!(task_workspace(&store, "other").unwrap().is_none());
        assert!(index(&store).unwrap().tasks[&id].archived);
    }
    #[test]
    fn index_files_unfiled_conversations_under_first_project() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&temp.path().join("w.sqlite")).unwrap();
        let filed = store.create_conversation().unwrap().id;
        let ghost = store.create_conversation().unwrap().id;
        let metaless = store.create_conversation().unwrap().id;
        let mut data = WorkspaceIndex::default();
        data.projects.push(Project { id: "p".into(), name: "P".into(), path: "C:/p".into() });
        data.projects.push(Project { id: "q".into(), name: "Q".into(), path: "C:/q".into() });
        data.tasks.insert(filed.clone(), TaskMeta { project_id: Some("q".into()), archived: true, pinned: true });
        data.tasks.insert(ghost.clone(), TaskMeta { project_id: Some("gone".into()), archived: true, pinned: false });
        store.save_setting("workspace_index_v1", &data).unwrap();
        let healed = index(&store).unwrap();
        // A stale folder is refiled to the first project; flags are preserved.
        assert_eq!(healed.tasks[&ghost].project_id.as_deref(), Some("p"));
        assert!(healed.tasks[&ghost].archived);
        assert_eq!(healed.tasks[&filed].project_id.as_deref(), Some("q"));
        assert_eq!(healed.tasks[&metaless].project_id.as_deref(), Some("p"));
        // file_task prefers the given project, then falls back to the first.
        let extra = store.create_conversation().unwrap().id;
        file_task(&store, &extra, Some("q")).unwrap();
        assert_eq!(index(&store).unwrap().tasks[&extra].project_id.as_deref(), Some("q"));
        let fallback = store.create_conversation().unwrap().id;
        file_task(&store, &fallback, Some("missing")).unwrap();
        assert_eq!(index(&store).unwrap().tasks[&fallback].project_id.as_deref(), Some("p"));
    }

    #[test]
    fn a_delayed_search_stays_bound_to_the_workspace_that_issued_it() {
        let mut index = WorkspaceIndex::default();
        index.projects.push(Project { id: "a".into(), name: "A".into(), path: "C:/proj-a".into() });
        index.projects.push(Project { id: "b".into(), name: "B".into(), path: "C:/proj-b".into() });
        // The global workspace has already moved to B. The in-flight request still names A.
        let (id, root, _) = resolve_search_root(&index, "C:/proj-b", Some("a"), Some("C:/proj-a")).unwrap();
        assert_eq!(id, "a");
        assert_eq!(root, "C:/proj-a");
        let (id, root, _) = resolve_search_root(&index, "C:/proj-b", None, Some(r"C:\proj-a")).unwrap();
        assert_eq!(id, "a");
        assert_eq!(root, "C:/proj-a");
        assert!(resolve_search_root(&index, "C:/proj-b", None, Some("C:/somewhere-else")).is_err());
        let (id, root, _) = resolve_search_root(&index, "C:/proj-b", None, None).unwrap();
        assert_eq!(id, "workspace");
        assert_eq!(root, "C:/proj-b");
    }
}
