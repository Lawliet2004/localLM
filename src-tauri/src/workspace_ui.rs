//! Desktop workspace operations. Metadata lives in the existing SQLite settings store.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project { pub id: String, pub name: String, pub path: String }
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskMeta { pub project_id: Option<String>, pub archived: bool, pub pinned: bool }
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceIndex { pub projects: Vec<Project>, pub tasks: BTreeMap<String, TaskMeta> }

fn index(store: &crate::store::Store) -> Result<WorkspaceIndex, String> { store.setting("workspace_index_v1") }

pub fn task_workspace(store: &crate::store::Store, id: &str) -> Result<Option<String>, String> {
    let data = index(store)?;
    let project = data.tasks.get(id).and_then(|t| t.project_id.as_ref());
    Ok(data.projects.iter().find(|p| Some(&p.id) == project).map(|p| p.path.clone()))
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

#[tauri::command]
pub fn workspace_inspect(state: tauri::State<'_, crate::AppState>, path: String, directory: bool) -> Result<Value, String> {
    let root = state.database()?.workspace_path()?;
    crate::workspace::Workspace::open(&root)?.call(if directory { "list_files" } else { "read_file" },
        if directory { json!({"path":path}) } else { json!({"path":path,"line_count":500}) })
}

async fn git(root: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new("git");
    cmd.arg("-C").arg(root).args(args).kill_on_drop(true);
    #[cfg(windows)] cmd.creation_flags(0x08000000);
    let out = tokio::time::timeout(std::time::Duration::from_secs(15), cmd.output()).await
        .map_err(|_| "Git timed out.")?.map_err(|e| format!("Cannot run Git: {e}"))?;
    if !out.status.success() { return Err(String::from_utf8_lossy(&out.stderr).chars().take(2000).collect()); }
    Ok(String::from_utf8_lossy(&out.stdout).chars().take(100_000).collect())
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
    Ok(json!({"branch":git(&root, &["branch","--show-current"]).await?.trim(),
        "branches":git(&root, &["for-each-ref","--format=%(refname:short)","refs/heads/"]).await?.lines().collect::<Vec<_>>(),
        "status":git(&root, &["status","--short"]).await?,
        "diff":git(&root, &["diff","--no-ext-diff","--no-color"]).await?}))
}

#[tauri::command]
pub async fn workspace_command(state: tauri::State<'_, crate::AppState>, command: String, channel: tauri::ipc::Channel<Value>) -> Result<Value, String> {
    let _guard = state.operation.try_lock().map_err(|_| "Wait for the active operation.")?;
    let executor = {
        let store = state.database()?;
        crate::execution::LocalExecution::new(store.execution_config()?, &store.workspace_path()?)?
    };
    state.cancel.send_replace(false);
    let mut cancel = state.cancel.subscribe();
    let callback = std::sync::Arc::new(move |stream: &str, chunk: &str| { let _ = channel.send(json!({"stream":stream,"chunk":chunk})); });
    tokio::select! {
        _ = cancel.changed() => Err("Command stopped; partial output is preserved in the terminal.".into()),
        result = executor.run_command_with_stream(json!({"command":command,"timeout_seconds":300}), Some(callback)) => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
