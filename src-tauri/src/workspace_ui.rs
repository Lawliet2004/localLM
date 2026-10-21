//! Desktop workspace operations. Metadata lives in the existing SQLite settings store.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub path: String,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskMeta {
    pub project_id: Option<String>,
    pub archived: bool,
    pub pinned: bool,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceIndex {
    pub projects: Vec<Project>,
    pub tasks: BTreeMap<String, TaskMeta>,
}

fn index(store: &crate::store::Store) -> Result<WorkspaceIndex, String> {
    let mut data: WorkspaceIndex = store.setting("workspace_index_v1")?;
    // Chats always file under a folder: a conversation whose task meta is
    // missing or points at a deleted project is healed into the first project
    // on the next index read (covers script-created and otherwise unfiled rows).
    if let Some(project) = data.projects.first() {
        let mut healed = false;
        for conversation in store.list_conversations()? {
            let filed = data
                .tasks
                .get(&conversation.id)
                .and_then(|task| task.project_id.as_ref())
                .is_some_and(|id| data.projects.iter().any(|p| &p.id == id));
            if !filed {
                data.tasks.entry(conversation.id).or_default().project_id =
                    Some(project.id.clone());
                healed = true;
            }
        }
        if healed {
            // ponytail: this write doesn't take the operation lock (index is
            // read during generation, so it can't). A save_task_meta landing
            // mid-heal could be clobbered; that chat self-heals into the first
            // project on the next read. Upgrade: a tasks table instead of one
            // JSON blob.
            store.save_setting("workspace_index_v1", &data)?;
        }
    }
    Ok(data)
}

/// File a conversation under `preferred` when it is still a project, otherwise
/// the first project. No-op when no project exists or the conversation is gone.
pub fn file_task(
    store: &crate::store::Store,
    id: &str,
    preferred: Option<&str>,
) -> Result<(), String> {
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
pub fn inherit_task_meta(
    store: &crate::store::Store,
    source_id: &str,
    new_id: &str,
) -> Result<(), String> {
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

pub fn task_workspace(store: &crate::store::Store, id: &str) -> Result<Option<String>, String> {
    let data = index(store)?;
    let project = data.tasks.get(id).and_then(|t| t.project_id.as_ref());
    Ok(data
        .projects
        .iter()
        .find(|p| Some(&p.id) == project)
        .map(|p| p.path.clone()))
}

#[tauri::command]
pub fn workspace_index(state: tauri::State<'_, crate::AppState>) -> Result<WorkspaceIndex, String> {
    index(&*state.database()?)
}

#[tauri::command]
pub async fn save_project(
    state: tauri::State<'_, crate::AppState>,
    mut project: Project,
) -> Result<WorkspaceIndex, String> {
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation.")?;
    save_project_record(&*state.database()?, &mut project)
}

fn save_project_record(
    store: &crate::store::Store,
    project: &mut Project,
) -> Result<WorkspaceIndex, String> {
    if project.name.trim().is_empty() || project.name.len() > 120 {
        return Err("Project name must be 1–120 characters.".into());
    }
    let path =
        std::fs::canonicalize(&project.path).map_err(|e| format!("Cannot open project: {e}"))?;
    crate::workspace::Workspace::open(&path.to_string_lossy())?;
    project.path = path.to_string_lossy().into_owned();
    let mut data = index(store)?;
    if data
        .projects
        .iter()
        .any(|p| p.id == project.id && p.path != project.path)
    {
        return Err(
            "This project ID belongs to another folder. Add the folder as a new project.".into(),
        );
    }
    if data
        .projects
        .iter()
        .any(|p| p.path == project.path && p.id != project.id)
    {
        return Err("This folder is already a project.".into());
    }
    if project.id.is_empty() {
        project.id = uuid::Uuid::new_v4().to_string();
    }
    data.projects.retain(|p| p.id != project.id);
    data.projects.push(project.clone());
    store.save_setting("workspace_index_v1", &data)?;
    Ok(data)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSelection {
    pub workspace: WorkspaceIndex,
    pub project: Project,
}

/// Folder pickers change the task association and the global editor root
/// together, so the next send and its preflight use the folder shown in UI.
#[tauri::command]
pub async fn set_task_workspace(
    state: tauri::State<'_, crate::AppState>,
    path: String,
    conversation_id: Option<String>,
) -> Result<WorkspaceSelection, String> {
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation.")?;
    select_task_workspace(&*state.database()?, &path, conversation_id.as_deref())
}

pub(crate) fn select_task_workspace(
    store: &crate::store::Store,
    path: &str,
    conversation_id: Option<&str>,
) -> Result<WorkspaceSelection, String> {
    let path = std::fs::canonicalize(path).map_err(|e| format!("Cannot open workspace: {e}"))?;
    crate::workspace::Workspace::open(&path.to_string_lossy())?;
    let mut data = index(store)?;
    if let Some(id) = &conversation_id {
        if !store.conversation_exists(id)? {
            return Err("Task no longer exists.".into());
        }
    }
    let path = path.to_string_lossy().into_owned();
    let project = data
        .projects
        .iter()
        .find(|p| p.path == path)
        .cloned()
        .unwrap_or_else(|| Project {
            id: uuid::Uuid::new_v4().to_string(),
            name: std::path::Path::new(&path)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Workspace".into()),
            path: path.clone(),
        });
    if !data.projects.iter().any(|p| p.id == project.id) {
        data.projects.push(project.clone());
    }
    if let Some(id) = conversation_id {
        data.tasks.entry(id.to_string()).or_default().project_id = Some(project.id.clone());
    }
    store.save_setting("workspace_index_v1", &data)?;
    store.save_workspace_path(&path)?;
    Ok(WorkspaceSelection {
        workspace: data,
        project,
    })
}

#[tauri::command]
pub async fn remove_project(
    state: tauri::State<'_, crate::AppState>,
    id: String,
) -> Result<WorkspaceIndex, String> {
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation.")?;
    let store = state.database()?;
    let mut data = index(&store)?;
    if let Some(project) = data.projects.iter().find(|project| project.id == id) {
        crate::document_index::forget_project(&store, &project.path)?;
    }
    data.projects.retain(|p| p.id != id);
    for task in data.tasks.values_mut() {
        if task.project_id.as_deref() == Some(&id) {
            task.project_id = None;
        }
    }
    store.save_setting("workspace_index_v1", &data)?;
    Ok(data)
}

pub fn remove_task_meta(store: &crate::store::Store, id: &str) -> Result<(), String> {
    let mut data = index(store)?;
    if data.tasks.remove(id).is_some() {
        store.save_setting("workspace_index_v1", &data)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn save_task_meta(
    state: tauri::State<'_, crate::AppState>,
    id: String,
    meta: TaskMeta,
) -> Result<WorkspaceIndex, String> {
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation.")?;
    let store = state.database()?;
    if !store.conversation_exists(&id)? {
        return Err("Task no longer exists.".into());
    }
    let mut data = index(&store)?;
    if let Some(project) = &meta.project_id {
        if !data.projects.iter().any(|p| &p.id == project) {
            return Err("Project no longer exists.".into());
        }
    }
    data.tasks.insert(id, meta);
    store.save_setting("workspace_index_v1", &data)?;
    Ok(data)
}

fn normalize_root(path: &str) -> String {
    path.trim().trim_end_matches(['/', '\\']).replace('\\', "/")
}

pub fn bound_root(state: &crate::AppState, requested: &str) -> Result<String, String> {
    if requested.trim().is_empty() {
        return Err("Choose a workspace folder first.".into());
    }
    let store = state.database()?;
    let (_, root, _) = resolve_search_root(
        &index(&store)?,
        &store.workspace_path()?,
        None,
        Some(requested),
    )?;
    Ok(root)
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
        let project = index
            .projects
            .iter()
            .find(|project| project.id == id)
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
        if let Some(project) = index
            .projects
            .iter()
            .find(|project| normalize_root(&project.path) == wanted)
        {
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

#[tauri::command]
pub fn workspace_inspect(
    state: tauri::State<'_, crate::AppState>,
    path: String,
    directory: bool,
    workspace_id: Option<String>,
    root: Option<String>,
) -> Result<Value, String> {
    let store = state.database()?;
    let saved = store.workspace_path()?;
    let index = index(&store)?;
    let (id, bound, revision) =
        resolve_search_root(&index, &saved, workspace_id.as_deref(), root.as_deref())?;
    let value = crate::workspace::Workspace::open(&bound)?.call(
        if directory { "list_files" } else { "read_file" },
        if directory {
            json!({"path": path})
        } else {
            json!({"path": path, "line_count": 500})
        },
    )?;
    Ok(stamp(value, &id, &bound, revision))
}

#[tauri::command]
pub fn workspace_search(
    state: tauri::State<'_, crate::AppState>,
    query: String,
    workspace_id: Option<String>,
    root: Option<String>,
) -> Result<Value, String> {
    let store = state.database()?;
    let saved = store.workspace_path()?;
    let index = index(&store)?;
    let (id, bound, revision) =
        resolve_search_root(&index, &saved, workspace_id.as_deref(), root.as_deref())?;
    let value =
        crate::workspace::Workspace::open(&bound)?.call("search_files", json!({"query": query}))?;
    Ok(stamp(value, &id, &bound, revision))
}

pub(crate) async fn git(root: &str, args: &[&str]) -> Result<String, String> {
    use std::process::Stdio;
    use tokio::io::AsyncReadExt;
    const LIMIT: u64 = 800_000;
    let mut cmd = tokio::process::Command::new("git");
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG",
        "GIT_CONFIG_COUNT",
    ] {
        cmd.env_remove(name);
    }
    cmd.arg("-c")
        .arg("core.fsmonitor=false")
        .arg("--literal-pathspecs")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x08000000);
    let operation = async {
        let mut child =
            crate::execution::managed_spawn(cmd).map_err(|e| format!("Cannot run Git: {e}"))?;
        let stdout = child.stdout().take().ok_or("Cannot read Git output.")?;
        let stderr = child.stderr().take().ok_or("Cannot read Git errors.")?;
        // Drain both pipes to EOF while retaining bounded prefixes. Stopping
        // either reader at its cap can deadlock a verbose hook or hide the
        // command's eventual nonzero exit behind a successful output excerpt.
        async fn capture<R: tokio::io::AsyncRead + Unpin>(
            mut reader: R,
            limit: usize,
        ) -> Result<Vec<u8>, String> {
            let mut retained = Vec::new();
            let mut chunk = vec![0u8; 8192];
            loop {
                let n = reader.read(&mut chunk).await.map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                retained.extend_from_slice(&chunk[..n.min(limit.saturating_sub(retained.len()))]);
            }
            Ok(retained)
        }
        let (output, errors) = tokio::join!(capture(stdout, LIMIT as usize), capture(stderr, 8192));
        let output = output?;
        let errors = errors?;
        let status = child.wait().await.map_err(|e| e.to_string())?;
        if !status.success() {
            return Err(String::from_utf8_lossy(&errors)
                .chars()
                .take(2000)
                .collect());
        }
        Ok(String::from_utf8_lossy(&output)
            .chars()
            .take(200_000)
            .collect())
    };
    tokio::time::timeout(std::time::Duration::from_secs(15), operation)
        .await
        .map_err(|_| "Git timed out.")?
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
pub enum GitAction {
    Stage {
        paths: Vec<String>,
    },
    Unstage {
        paths: Vec<String>,
    },
    Commit {
        message: String,
        #[serde(rename = "expectedTree")]
        expected_tree: String,
        confirmed: bool,
    },
    CreateBranch {
        name: String,
    },
    History,
}

pub(crate) fn git_paths(paths: &[String]) -> Result<(), String> {
    if paths.is_empty() || paths.len() > 200 {
        return Err("Select between 1 and 200 files.".into());
    }
    for path in paths {
        if path.is_empty()
            || path.len() > 4096
            || path.contains('\0')
            || path.contains('\\')
            || std::path::Path::new(path)
                .components()
                .any(|part| !matches!(part, std::path::Component::Normal(_)))
            || path
                .split('/')
                .any(|part| part.eq_ignore_ascii_case(".git"))
        {
            return Err("Git paths must name files inside the workspace.".into());
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn workspace_git_action(
    state: tauri::State<'_, crate::AppState>,
    root: String,
    request: GitAction,
) -> Result<Value, String> {
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation.")?;
    let root = bound_root(&state, &root)?;
    perform_git_action(&root, request).await
}

pub(crate) async fn perform_git_action(root: &str, request: GitAction) -> Result<Value, String> {
    let stage = matches!(&request, GitAction::Stage { .. });
    match request {
        GitAction::Stage { paths } | GitAction::Unstage { paths } => {
            git_paths(&paths)?;
            // No shell interpolation, glob expansion, force flags or resets
            // of the working tree. Deletions are staged through git add too.
            let has_head = git(root, &["rev-parse", "--verify", "HEAD"]).await.is_ok();
            let mut args = if stage {
                vec!["add", "--"]
            } else if has_head {
                vec!["restore", "--staged", "--"]
            } else {
                vec!["rm", "--cached", "--ignore-unmatch", "--"]
            };
            args.extend(paths.iter().map(String::as_str));
            git(root, &args).await?;
        }
        GitAction::Commit {
            message,
            expected_tree,
            confirmed,
        } => {
            if !confirmed {
                return Err("Review the staged changes and confirm the commit first.".into());
            }
            if message.trim().is_empty() || message.len() > 10000 || message.contains('\0') {
                return Err("Commit message must contain 1–10000 bytes.".into());
            }
            let repository = git(root, &["rev-parse", "--show-toplevel"]).await?;
            if std::fs::canonicalize(root).map_err(|e| e.to_string())?
                != std::fs::canonicalize(repository.trim()).map_err(|e| e.to_string())?
            {
                return Err(
                    "Open the repository root to review all staged changes before committing."
                        .into(),
                );
            }
            let current_tree = git(root, &["write-tree"]).await?;
            if expected_tree.is_empty() || current_tree.trim() != expected_tree {
                return Err(
                    "Staged changes changed since review. Refresh and confirm again.".into(),
                );
            }
            git(root, &["commit", "-m", message.trim()]).await?;
        }
        GitAction::CreateBranch { name } => {
            if name.len() > 240 || name.starts_with('-') || name.trim().is_empty() {
                return Err("Invalid branch name.".into());
            }
            git(root, &["check-ref-format", "--branch", &name]).await?;
            git(root, &["branch", &name]).await?;
        }
        GitAction::History => {
            if git(root, &["rev-parse", "--verify", "HEAD"]).await.is_err() {
                return Ok(json!({"entries":[]}));
            }
            let log = git(
                root,
                &["log", "-50", "--format=%H%x00%h%x00%an%x00%aI%x00%s"],
            )
            .await?;
            let entries = log.lines().filter_map(|line| {
                let fields: Vec<_> = line.splitn(5, '\0').collect();
                (fields.len() == 5).then(|| json!({"id":fields[0], "shortId":fields[1], "author":fields[2], "date":fields[3], "subject":fields[4]}))
            }).collect::<Vec<_>>();
            return Ok(json!({"entries":entries}));
        }
    }
    inspect_git(root, None).await
}

fn status_files(status: &str) -> Vec<Value> {
    // Porcelain -z emits literal paths (including spaces, newlines and quotes).
    // Renames have a second NUL-delimited original path.
    let mut records = status.split('\0').filter(|part| !part.is_empty());
    let mut files = Vec::new();
    while let Some(record) = records.next() {
        if record.len() < 4 || !record.is_char_boundary(3) {
            continue;
        }
        let code = &record[..2];
        let path = &record[3..];
        let previous = if code.contains(['R', 'C']) {
            records.next()
        } else {
            None
        };
        files.push(json!({ "path": path, "status": code, "previousPath": previous }));
    }
    files
}

#[tauri::command]
pub async fn workspace_git(
    state: tauri::State<'_, crate::AppState>,
    branch: Option<String>,
    root: Option<String>,
) -> Result<Value, String> {
    let _guard = if branch.is_some() {
        Some(
            state
                .operation
                .try_lock()
                .map_err(|_| "Wait for the active operation.")?,
        )
    } else {
        None
    };
    let root = match root {
        Some(root) => bound_root(&state, &root)?,
        None => state.database()?.workspace_path()?,
    };
    if root.is_empty() {
        return Err("Select a workspace folder first.".into());
    }
    inspect_git(&root, branch.as_deref()).await
}

pub(crate) async fn inspect_git(root: &str, branch: Option<&str>) -> Result<Value, String> {
    // A missing executable/permission failure is an error; a normal folder is
    // an intentional empty state, not a raw Git stderr banner.
    match git(root, &["rev-parse", "--is-inside-work-tree"]).await {
        Ok(value) if value.trim() == "true" => (),
        Ok(_) => {
            return Ok(
                json!({"repository":false,"branch":"","branches":[],"status":"","diff":"","files":[]}),
            )
        }
        Err(error) if error.contains("not a git repository") => {
            return Ok(
                json!({"repository":false,"branch":"","branches":[],"status":"","diff":"","files":[]}),
            )
        }
        Err(error) => return Err(error),
    }
    if let Some(branch) = branch {
        if branch.starts_with('-') || branch.len() > 240 || branch.trim().is_empty() {
            return Err("Invalid branch.".into());
        }
        git(root, &["check-ref-format", "--branch", branch]).await?;
        // Never force checkout, reset, or discard changes.
        git(root, &["switch", branch]).await?;
    }
    // Working tree versus HEAD covers staged and unstaged edits; a repository
    // without commits has no HEAD, so combine staged and unstaged diffs.
    let diff = match git(
        root,
        &[
            "diff",
            "--relative",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "HEAD",
            "--",
            ".",
        ],
    )
    .await
    {
        Ok(diff) => diff,
        // An unborn repository still has staged changes. Diff against Git's
        // empty tree as well as the working tree rather than losing the index.
        Err(_) => {
            let staged = git(
                root,
                &[
                    "diff",
                    "--relative",
                    "--cached",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--no-color",
                    "--",
                    ".",
                ],
            )
            .await?;
            let unstaged = git(
                root,
                &[
                    "diff",
                    "--relative",
                    "--no-ext-diff",
                    "--no-textconv",
                    "--no-color",
                    "--",
                    ".",
                ],
            )
            .await?;
            format!("{staged}{unstaged}")
        }
    };
    let prefix = git(root, &["rev-parse", "--show-prefix"])
        .await?
        .trim_end_matches('\n')
        .to_string();
    let files = status_files(
        &git(
            root,
            &[
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--",
                ".",
            ],
        )
        .await?,
    )
    .into_iter()
    .filter_map(|mut file| {
        let path = file["path"].as_str()?.strip_prefix(&prefix)?.to_string();
        file["path"] = json!(path);
        if let Some(previous) = file["previousPath"].as_str() {
            file["previousPath"] = json!(previous.strip_prefix(&prefix).unwrap_or(previous));
        }
        Some(file)
    })
    .collect::<Vec<_>>();
    let branch = git(root, &["branch", "--show-current"])
        .await?
        .trim()
        .to_string();
    let mut branches = git(
        root,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
    )
    .await?
    .lines()
    .map(str::to_string)
    .collect::<Vec<_>>();
    if !branch.is_empty() && !branches.contains(&branch) {
        branches.push(branch.clone());
    }
    Ok(json!({"repository":true,"branch":branch,
        "stagedTree":git(root, &["write-tree"]).await.ok().map(|v| v.trim().to_string()),
        "branches":branches,
        "status":git(root, &["status","--short", "--untracked-files=all", "--", "."]).await?,
        "files":files,"truncated":diff.chars().count() >= 200_000,"diff":diff.chars().take(200_000).collect::<String>()}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn git_actions_stage_unstage_confirm_commit_and_keep_disk_changes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_str().unwrap();
        git(root, &["init", "-b", "main"]).await.unwrap();
        git(root, &["config", "user.name", "LocalLM fixture"])
            .await
            .unwrap();
        git(root, &["config", "user.email", "fixture@example.invalid"])
            .await
            .unwrap();
        let filename = if cfg!(windows) {
            "literal name.txt"
        } else {
            ":(glob)literal.txt"
        };
        std::fs::write(temp.path().join(filename), "one\n").unwrap();
        std::fs::write(temp.path().join("untouched.txt"), "preserve\n").unwrap();
        let result = perform_git_action(
            root,
            GitAction::Stage {
                paths: vec![filename.into()],
            },
        )
        .await
        .unwrap();
        assert_eq!(
            result["files"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|v| v["status"] == "A ")
                .count(),
            1
        );
        perform_git_action(
            root,
            GitAction::Unstage {
                paths: vec![filename.into()],
            },
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(temp.path().join(filename)).unwrap(),
            "one\n"
        );
        let result = perform_git_action(
            root,
            GitAction::Stage {
                paths: vec![filename.into()],
            },
        )
        .await
        .unwrap();
        let tree = result["stagedTree"].as_str().unwrap().to_string();
        assert!(perform_git_action(
            root,
            GitAction::Commit {
                message: "first".into(),
                expected_tree: tree.clone(),
                confirmed: false
            }
        )
        .await
        .is_err());
        assert!(perform_git_action(
            root,
            GitAction::Commit {
                message: "first".into(),
                expected_tree: "stale".into(),
                confirmed: true
            }
        )
        .await
        .is_err());
        perform_git_action(
            root,
            GitAction::Commit {
                message: "first $(literal)".into(),
                expected_tree: tree,
                confirmed: true,
            },
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(temp.path().join("untouched.txt")).unwrap(),
            "preserve\n"
        );
        let history = perform_git_action(root, GitAction::History).await.unwrap();
        assert_eq!(history["entries"][0]["subject"], "first $(literal)");
        perform_git_action(
            root,
            GitAction::CreateBranch {
                name: "feature/test".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(
            inspect_git(root, Some("feature/test")).await.unwrap()["branch"],
            "feature/test"
        );
        std::fs::write(temp.path().join(filename), "two\n").unwrap();
        perform_git_action(
            root,
            GitAction::Stage {
                paths: vec![filename.into()],
            },
        )
        .await
        .unwrap();
        perform_git_action(
            root,
            GitAction::Unstage {
                paths: vec![filename.into()],
            },
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(temp.path().join(filename)).unwrap(),
            "two\n"
        );
        for path in [
            "..",
            "../outside",
            "/absolute",
            ".git/config",
            "nested/.git/config",
        ] {
            assert!(git_paths(&[path.into()]).is_err());
        }
        assert!(perform_git_action(
            root,
            GitAction::CreateBranch {
                name: "--force".into()
            }
        )
        .await
        .is_err());
    }
    #[test]
    fn same_name_projects_keep_ids_paths_and_task_associations() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&temp.path().join("w.sqlite")).unwrap();
        let first = temp.path().join("one/project");
        let second = temp.path().join("two/project");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let mut a = Project {
            id: String::new(),
            name: "project".into(),
            path: first.to_string_lossy().into_owned(),
        };
        save_project_record(&store, &mut a).unwrap();
        let chat = store.create_conversation().unwrap().id;
        file_task(&store, &chat, Some(&a.id)).unwrap();
        let mut collision = Project {
            id: a.id.clone(),
            name: "project".into(),
            path: second.to_string_lossy().into_owned(),
        };
        assert!(save_project_record(&store, &mut collision).is_err());
        collision.id.clear();
        let data = save_project_record(&store, &mut collision).unwrap();
        assert_eq!(data.projects.len(), 2);
        assert_ne!(a.id, collision.id);
        assert_eq!(task_workspace(&store, &chat).unwrap(), Some(a.path));
    }
    #[test]
    fn folder_selection_updates_task_and_editor_together_and_reuses_projects() {
        let temp = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&temp.path().join("w.sqlite")).unwrap();
        let a = store.create_conversation().unwrap().id;
        let b = store.create_conversation().unwrap().id;
        let original = temp.path().join("original");
        let picked = temp.path().join("picked");
        std::fs::create_dir(&original).unwrap();
        std::fs::create_dir(&picked).unwrap();
        select_task_workspace(&store, original.to_str().unwrap(), Some(&a)).unwrap();
        let previous = task_workspace(&store, &a).unwrap();
        let selected = select_task_workspace(&store, picked.to_str().unwrap(), Some(&b)).unwrap();
        assert_eq!(
            task_workspace(&store, &b).unwrap(),
            Some(selected.project.path.clone())
        );
        assert_eq!(store.workspace_path().unwrap(), selected.project.path);
        assert_eq!(task_workspace(&store, &a).unwrap(), previous);
        let again = select_task_workspace(&store, picked.to_str().unwrap(), Some(&b)).unwrap();
        assert_eq!(again.workspace.projects.len(), 2);
        assert_eq!(again.project.id, selected.project.id);
    }
    #[tokio::test]
    async fn review_handles_plain_folders_unborn_index_and_tracked_changes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().to_str().unwrap();
        assert_eq!(inspect_git(root, None).await.unwrap()["repository"], false);
        git(root, &["init", "-b", "main"]).await.unwrap();
        std::fs::write(temp.path().join("space name.txt"), "initial\n").unwrap();
        git(root, &["add", "--", "space name.txt"]).await.unwrap();
        let unborn = inspect_git(root, None).await.unwrap();
        assert!(unborn["diff"].as_str().unwrap().contains("+initial"));
        assert_eq!(unborn["files"][0]["path"], "space name.txt");
        git(
            root,
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.test",
                "commit",
                "-m",
                "fixture",
            ],
        )
        .await
        .unwrap();
        std::fs::write(temp.path().join("space name.txt"), "updated\n").unwrap();
        let changed = inspect_git(root, None).await.unwrap();
        assert_eq!(changed["branch"], "main");
        assert!(changed["diff"].as_str().unwrap().contains("+updated"));
        assert!(inspect_git(root, Some("--force")).await.is_err());
        assert_eq!(
            std::fs::read_to_string(temp.path().join("space name.txt")).unwrap(),
            "updated\n"
        );
        std::fs::create_dir(temp.path().join("nested")).unwrap();
        std::fs::write(temp.path().join("nested/code.txt"), "nested\n").unwrap();
        let nested = inspect_git(temp.path().join("nested").to_str().unwrap(), None)
            .await
            .unwrap();
        assert_eq!(nested["files"].as_array().unwrap().len(), 1);
        assert_eq!(nested["files"][0]["path"], "code.txt");
    }
    #[test]
    fn porcelain_paths_preserve_spaces_newlines_and_renames() {
        let files =
            status_files(" M src/space name.ts\0R  new\nname.txt\0old.txt\0?? untracked.txt\0");
        assert_eq!(files.len(), 3);
        assert_eq!(files[0]["path"], "src/space name.ts");
        assert_eq!(files[1]["path"], "new\nname.txt");
        assert_eq!(files[1]["previousPath"], "old.txt");
        assert_eq!(files[2]["status"], "??");
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
            data.projects.push(Project {
                id: "p".into(),
                name: "Project".into(),
                path: "C:/project".into(),
            });
            data.tasks.insert(
                id.clone(),
                TaskMeta {
                    project_id: Some("p".into()),
                    archived: true,
                    pinned: true,
                },
            );
            store.save_setting("workspace_index_v1", &data).unwrap();
        }
        let store = crate::store::Store::open(&db).unwrap();
        assert_eq!(
            task_workspace(&store, &id).unwrap().as_deref(),
            Some("C:/project")
        );
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
        data.projects.push(Project {
            id: "p".into(),
            name: "P".into(),
            path: "C:/p".into(),
        });
        data.projects.push(Project {
            id: "q".into(),
            name: "Q".into(),
            path: "C:/q".into(),
        });
        data.tasks.insert(
            filed.clone(),
            TaskMeta {
                project_id: Some("q".into()),
                archived: true,
                pinned: true,
            },
        );
        data.tasks.insert(
            ghost.clone(),
            TaskMeta {
                project_id: Some("gone".into()),
                archived: true,
                pinned: false,
            },
        );
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
        assert_eq!(
            index(&store).unwrap().tasks[&extra].project_id.as_deref(),
            Some("q")
        );
        let fallback = store.create_conversation().unwrap().id;
        file_task(&store, &fallback, Some("missing")).unwrap();
        assert_eq!(
            index(&store).unwrap().tasks[&fallback]
                .project_id
                .as_deref(),
            Some("p")
        );
    }

    #[test]
    fn a_delayed_search_stays_bound_to_the_workspace_that_issued_it() {
        let mut index = WorkspaceIndex::default();
        index.projects.push(Project {
            id: "a".into(),
            name: "A".into(),
            path: "C:/proj-a".into(),
        });
        index.projects.push(Project {
            id: "b".into(),
            name: "B".into(),
            path: "C:/proj-b".into(),
        });
        // The global workspace has already moved to B. The in-flight request still names A.
        let (id, root, _) =
            resolve_search_root(&index, "C:/proj-b", Some("a"), Some("C:/proj-a")).unwrap();
        assert_eq!(id, "a");
        assert_eq!(root, "C:/proj-a");
        let (id, root, _) =
            resolve_search_root(&index, "C:/proj-b", None, Some(r"C:\proj-a")).unwrap();
        assert_eq!(id, "a");
        assert_eq!(root, "C:/proj-a");
        assert!(resolve_search_root(&index, "C:/proj-b", None, Some("C:/somewhere-else")).is_err());
        let (id, root, _) = resolve_search_root(&index, "C:/proj-b", None, None).unwrap();
        assert_eq!(id, "workspace");
        assert_eq!(root, "C:/proj-b");
    }
}
