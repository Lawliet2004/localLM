//! Typed Git operations over the existing UI backend, scoped to the selected
//! workspace. Approval belongs to the dispatcher, never a model argument.
use serde::Deserialize;
use serde_json::{json, Value};

pub const ALIASES: [&str; 7] = [
    "git_status",
    "git_diff",
    "git_stage",
    "git_unstage",
    "git_commit",
    "git_branch",
    "git_history",
];
pub const WORKTREE_ALIASES: [&str; 2] = ["worktree_enter", "worktree_exit"];
pub fn is_worktree(alias: &str) -> bool {
    WORKTREE_ALIASES.contains(&alias)
}
pub fn owns(alias: &str) -> bool {
    ALIASES.contains(&alias)
}
pub fn available() -> bool {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .any(|p| {
            crate::execution::executable(
                &p.join(if cfg!(windows) { "git.exe" } else { "git" })
                    .to_string_lossy(),
            )
        })
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Empty {}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Paths {
    paths: Vec<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Commit {
    message: String,
    expected_tree: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum BranchAction {
    Create,
    Switch,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Branch {
    action: BranchAction,
    name: String,
}
#[derive(Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum DiffKind {
    #[default]
    Working,
    Staged,
    All,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Diff {
    #[serde(default)]
    kind: DiffKind,
    #[serde(default)]
    paths: Vec<String>,
}
fn args<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| e.to_string())
}

fn checked_inventory(output: &str) -> Result<(), String> {
    if output.chars().count() >= 200000 {
        return Err(
            "Git path inventory exceeded its bound; narrow the explicit paths before continuing."
                .into(),
        );
    }
    for path in output.split('\0').filter(|p| !p.is_empty()) {
        crate::sandbox::check_fs_path("", path, false)?;
    }
    Ok(())
}

async fn checked_disk_inventory(root: &str, output: String) -> Result<(), String> {
    checked_inventory(&output)?;
    let root = root.to_string();
    tokio::task::spawn_blocking(move|| {
        use cap_fs_ext::DirExt;
        let directory=cap_std::fs::Dir::open_ambient_dir(root,cap_std::ambient_authority()).map_err(|e|e.to_string())?;
        for path in output.split('\0').filter(|p|!p.is_empty()) {
            crate::workspace_ui::git_paths(&[path.to_string()])?;
            let mut parts=path.split('/').collect::<Vec<_>>();
            let name=parts.pop().ok_or("Empty Git path.")?;
            let mut parent=directory.try_clone().map_err(|e|e.to_string())?;
            let mut missing=false;
            for part in parts {
                match parent.open_dir_nofollow(part) {
                    Ok(next)=>parent=next,
                    Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{missing=true;break;},
                    Err(e)=>return Err(format!("Cannot safely inspect Git path: {e}")),
                }
            }
            if missing {continue;} // A tracked deletion has no current bytes.
            match parent.symlink_metadata(name) {
                Err(e) if e.kind()==std::io::ErrorKind::NotFound=>(),
                Err(e)=>return Err(e.to_string()),
                Ok(meta)=>{
                    if !meta.is_file(){return Err("Typed Git content operations require regular files without symlinks or submodules.".into());}
                    // Reuse native opened-handle link/identity validation; no
                    // content read and no symlink traversal in preflight.
                    use cap_fs_ext::{FollowSymlinks,OpenOptionsFollowExt};
                    let file=parent.open_with(name,cap_std::fs::OpenOptions::new().read(true).follow(FollowSymlinks::No)).map_err(|e|e.to_string())?;
                    crate::workspace::patch::file_identity::single_link_identity(&file)?;
                }
            }
        }
        Ok(())
    }).await.map_err(|e|format!("Git path preflight failed: {e}"))?
}

async fn repository_root(root: &str) -> Result<(), String> {
    let repository = crate::workspace_ui::git(root, &["rev-parse", "--show-toplevel"]).await?;
    let selected = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    let repository = std::fs::canonicalize(repository.trim()).map_err(|e| e.to_string())?;
    if selected != repository {
        return Err(
            "Open the repository root for whole-repository history, branch changes or commits."
                .into(),
        );
    }
    Ok(())
}

/// The repository that owns a workspace, or `None` when the workspace is not
/// inside a repository at all.
///
/// Shared with the model-facing worktree listing so every caller uses one
/// Git invocation path and one "not a repository" answer.
pub(crate) async fn repository_root_of(root: &str) -> Result<Option<String>, String> {
    match crate::workspace_ui::git(root, &["rev-parse", "--show-toplevel"]).await {
        Ok(repository) => Ok(Some(repository.trim().to_string())),
        Err(error) if error.contains("not a git repository") => Ok(None),
        Err(error) => Err(error),
    }
}

pub fn registry() -> Vec<crate::tool_spec::ToolSpec> {
    let schema = |properties, required: &[&str]| json!({"type":"object","properties":properties,"required":required,"additionalProperties":false});
    let paths = json!({"type":"array","items":{"type":"string","minLength":1,"maxLength":4096},"minItems":1,"maxItems":200,"uniqueItems":true,"description":"Literal workspace-relative file paths; no glob/pathspec expansion, traversal or .git access."});
    let mut commit = schema(
        json!({"message":{"type":"string","minLength":1,"maxLength":10000},"expected_tree":{"type":"string","pattern":"^(?:[a-f0-9]{40}|[a-f0-9]{64})$","description":"The full stagedTree hash reviewed in git_diff(kind=staged). Rejected if the index changed. Commit only from the repository root."}}),
        &["message", "expected_tree"],
    );
    commit["x-locallm-always-ask"] = json!(true);
    vec![
        ("git_status".into(),"Inspect Git status, branch, literal changed paths and staged-tree hash in the selected workspace. Does not stage, discard or commit files. Review git_diff(kind=staged) before requesting a commit.".into(),schema(json!({}),&[])),
        ("git_diff".into(),"Read a bounded Git diff: working compares disk to index, staged compares index to HEAD (or empty tree), all compares disk to HEAD. Optional literal paths scope the diff. Staged output includes a checked stagedTree pin for commit review. Git metadata may be refreshed; external writers can race these optimistic checks.".into(),schema(json!({"kind":{"type":"string","enum":["working","staged","all"],"default":"working"},"paths":paths}),&[])),
        ("git_stage".into(),"Stage the explicitly named literal paths, including deletions, using the existing Git backend. Never stages all files implicitly or discards disk edits. Follows workspace source/capability and mutation approval policy.".into(),schema(json!({"paths":paths}),&["paths"])),
        ("git_unstage".into(),"Unstage explicitly named literal paths while retaining their current disk contents. Supports a repository without a first commit. Never resets or discards working files.".into(),schema(json!({"paths":paths}),&["paths"])),
        ("git_commit".into(),"Request an explicitly approved commit of the reviewed staged tree with a message. Requires expected_tree from git_diff(kind=staged); rejects a stale pin or workspace subdirectory. Always asks for user approval, even in Full Access. The model cannot supply confirmation. Uses normal Git hooks/configuration; no automatic retry after uncertain outcomes.".into(),commit),
        ("git_branch".into(),"Create a local branch or switch to an existing branch without force/reset/discard flags. Git refuses conflicting disk changes. No remote push, fetch, deletion or shell interpolation.".into(),schema(json!({"action":{"type":"string","enum":["create","switch"]},"name":{"type":"string","minLength":1,"maxLength":240}}),&["action","name"])),
        ("git_history".into(),"Read the latest at most50 local commits as structured IDs, authors, dates and subjects. Empty repositories return an empty list. limitReached means older history may exist.".into(),schema(json!({}),&[])),
        ("worktree_enter".into(),"Create an isolated Git worktree under app data on a new locallm/<name> branch and switch this conversation's task workspace to it, so further file and shell work happens there until worktree_exit. Always asks for approval.".into(),{
            let mut schema=schema(json!({"name":{"type":"string","pattern":"^[a-z0-9-]{1,40}$"},"base":{"type":"string","maxLength":200,"default":"HEAD"}}),&["name"]);
            schema["x-locallm-always-ask"]=json!(true);schema}),
        ("worktree_exit".into(),"Switch the conversation's task workspace back to the original directory and optionally remove the created worktree. Removal refuses uncommitted changes; the locallm branch is kept. Always asks for approval.".into(),{
            let mut schema=schema(json!({"action":{"type":"string","enum":["keep","remove"]},"expected_clean":{"type":"boolean","default":false}}),&["action"]);
            schema["x-locallm-always-ask"]=json!(true);schema}),
    ].into_iter().map(crate::tool_spec::ToolSpec::from).collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorktreeEnter {
    name: String,
    #[serde(default)]
    base: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WorktreeExit {
    action: String,
    #[serde(default)]
    expected_clean: Option<bool>,
}

fn worktree_key(conversation_id: &str) -> String {
    format!("worktree:{conversation_id}")
}

/// Enter/exit an isolated git worktree for a conversation. Not in ALIASES:
/// it needs live state (data dir, store, workspace switching) and detached
/// children must never reach it.
pub async fn execute_worktree(
    state: &crate::AppState,
    conversation_id: &str,
    workspace: &str,
    alias: &str,
    value: Value,
) -> Result<Value, String> {
    if workspace.is_empty() {
        return Err("Select a workspace before using worktrees.".into());
    }
    if !available() {
        return Err("Git executable is unavailable.".into());
    }
    use crate::workspace_ui::git;
    let key = worktree_key(conversation_id);
    match alias {
        "worktree_enter" => {
            let args: WorktreeEnter = args(value)?;
            let base = args.base.clone().unwrap_or_else(|| "HEAD".into());
            if base.chars().any(|c| c == ' ' || c == '\0') {
                return Err("Invalid base.".into());
            }
            let repo = git(workspace, &["rev-parse", "--show-toplevel"])
                .await
                .map_err(|e| format!("Not a git repository: {e}"))?;
            let repo = std::fs::canonicalize(repo.trim()).map_err(|e| e.to_string())?;
            if state
                .database()?
                .setting::<Value>(&key)
                .unwrap_or_default()
                .is_object()
            {
                return Err(
                    "This conversation already has an active worktree; run worktree_exit first."
                        .into(),
                );
            }
            let dir = state
                .data_dir
                .join("worktrees")
                .join(conversation_id)
                .join(&args.name);
            if dir.exists() {
                return Err(format!("Worktree path {} already exists.", dir.display()));
            }
            std::fs::create_dir_all(dir.parent().unwrap()).map_err(|e| e.to_string())?;
            let branch = format!("locallm/{}", args.name);
            git(
                repo.to_str().unwrap_or_default(),
                &[
                    "worktree",
                    "add",
                    "-b",
                    &branch,
                    &dir.to_string_lossy(),
                    &base,
                ],
            )
            .await
            .map_err(|e| format!("git worktree add failed: {e}"))?;
            let store = state.database()?;
            let previous = crate::workspace_ui::task_workspace(&store, conversation_id)?
                .unwrap_or_else(|| workspace.to_string());
            store.save_setting(
                &key,
                &json!({"original":previous,"path":dir.to_string_lossy(),"branch":branch}),
            )?;
            crate::workspace_ui::select_task_workspace(
                &store,
                &dir.to_string_lossy(),
                Some(conversation_id),
            )?;
            Ok(
                json!({"path":dir.to_string_lossy(),"branch":branch,"previousWorkspace":previous,
                "note":"Further file and shell work in this conversation now happens inside the worktree."}),
            )
        }
        "worktree_exit" => {
            let args: WorktreeExit = args(value)?;
            let record: Value = state
                .database()?
                .setting::<Option<Value>>(&key)?
                .ok_or("No active worktree for this conversation.")?;
            let path = record["path"].as_str().unwrap_or_default().to_string();
            let original = record["original"].as_str().unwrap_or_default().to_string();
            let branch = record["branch"].as_str().unwrap_or_default().to_string();
            if args.action == "remove" {
                let status = git(&path, &["status", "--porcelain"])
                    .await
                    .unwrap_or_default();
                if !status.trim().is_empty() {
                    let reason = if args.expected_clean == Some(true) {
                        "expected_clean asserted but the worktree has uncommitted changes"
                    } else {
                        "the worktree has uncommitted changes; keep it or commit/discard first"
                    };
                    return Err(format!("Refusing to remove worktree: {reason}."));
                }
            }
            crate::workspace_ui::select_task_workspace(
                &*state.database()?,
                &original,
                Some(conversation_id),
            )?;
            if args.action == "remove" {
                git(&original, &["worktree", "remove", &path])
                    .await
                    .map_err(|e| format!("git worktree remove failed: {e}"))?;
            }
            state.database()?.save_setting(&key, &Value::Null)?;
            Ok(
                json!({"restored":original,"removed":args.action=="remove","branch":branch,
                "note":if args.action=="remove"{"Worktree removed; branch kept."}else{"Worktree kept on disk."}}),
            )
        }
        _ => Err("Unknown worktree tool.".into()),
    }
}

pub async fn execute(root: &str, alias: &str, value: Value) -> Result<Value, String> {
    if !owns(alias) {
        return Err("Unknown typed Git tool.".into());
    }
    // Defense for direct/detached calls, in addition to AgentTool preparation.
    let schema = registry()
        .into_iter()
        .find(|spec| spec.name == alias)
        .unwrap()
        .input;
    crate::tool_validation::validate(&schema, &value)?;
    if root.is_empty() {
        return Err("Select a workspace before using Git.".into());
    }
    if !available() {
        return Err("Git executable is unavailable. Install Git or configure PATH before using these tools.".into());
    }
    // Git is an external command boundary, like the shell. Refuse an existing
    // journal before launch; noncooperating external writers can still race
    // this optimistic check. No native lock is held across an async await.
    let path = root.to_string();
    tokio::task::spawn_blocking(move || {
        let lock = crate::workspace::patch::root_lock(std::path::Path::new(&path))?;
        let _guard = lock
            .read()
            .map_err(|_| "Workspace transaction lock unavailable.".to_string())?;
        let dir = cap_std::fs::Dir::open_ambient_dir(&path, cap_std::ambient_authority())
            .map_err(|e| e.to_string())?;
        crate::workspace::patch::ensure_clear(&dir)
    })
    .await
    .map_err(|e| format!("Git workspace preflight failed: {e}"))??;
    use crate::workspace_ui::{git, inspect_git, perform_git_action, GitAction};
    match git(root, &["rev-parse", "--show-toplevel"]).await {
        Ok(repository) => {
            let selected = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
            let worktree = std::fs::canonicalize(repository.trim()).map_err(|e| e.to_string())?;
            if !selected.starts_with(&worktree) {
                return Err("Git configuration redirects the worktree outside the selected workspace. Select the actual worktree before continuing.".into());
            }
        }
        Err(error) if alias == "git_status" && error.contains("not a git repository") => (),
        Err(error) => return Err(error),
    }
    let mut result = match alias {
        "git_status" => {
            let _: Empty = args(value)?;
            let mut result = inspect_git(root, None).await?;
            result.as_object_mut().unwrap().remove("diff");
            result
        }
        "git_history" => {
            let _: Empty = args(value)?;
            repository_root(root).await?;
            let head = git(root, &["rev-parse", "--verify", "HEAD"]).await.is_ok();
            let mut result = perform_git_action(root, GitAction::History).await?;
            result["headVerified"] = json!(head);
            result["historyState"] = json!(if head {
                "available"
            } else {
                "unborn_or_unavailable"
            });
            result["limitReached"] = json!(result["entries"]
                .as_array()
                .is_some_and(|entries| entries.len() >= 50));
            result
        }
        "git_stage" | "git_unstage" => {
            let request: Paths = args(value)?;
            crate::workspace_ui::git_paths(&request.paths)?;
            for path in &request.paths {
                crate::sandbox::check_fs_path("", path, false)?;
            }
            let mut list = vec![
                "ls-files",
                "-z",
                "--cached",
                "--others",
                "--exclude-standard",
                "--",
            ];
            list.extend(request.paths.iter().map(String::as_str));
            checked_disk_inventory(root, git(root, &list).await?).await?;
            let action = if alias == "git_stage" {
                GitAction::Stage {
                    paths: request.paths,
                }
            } else {
                GitAction::Unstage {
                    paths: request.paths,
                }
            };
            perform_git_action(root, action).await?
        }
        "git_commit" => {
            let request: Commit = args(value)?;
            repository_root(root).await?;
            checked_inventory(
                &git(
                    root,
                    &[
                        "diff",
                        "--cached",
                        "--name-only",
                        "-z",
                        "--no-ext-diff",
                        "--no-textconv",
                        "--",
                        ".",
                    ],
                )
                .await?,
            )?;
            perform_git_action(
                root,
                GitAction::Commit {
                    message: request.message,
                    expected_tree: request.expected_tree,
                    confirmed: true,
                },
            )
            .await?
        }
        "git_branch" => {
            let request: Branch = args(value)?;
            repository_root(root).await?;
            match request.action {
                BranchAction::Create => {
                    perform_git_action(root, GitAction::CreateBranch { name: request.name }).await?
                }
                BranchAction::Switch => {
                    if request.name.starts_with('-')
                        || request.name.len() > 240
                        || request.name.trim().is_empty()
                    {
                        return Err("Invalid branch name.".into());
                    }
                    git(root, &["check-ref-format", "--branch", &request.name]).await?;
                    checked_inventory(
                        &git(
                            root,
                            &[
                                "ls-tree",
                                "-r",
                                "--name-only",
                                "-z",
                                &request.name,
                                "--",
                                ".",
                            ],
                        )
                        .await?,
                    )?;
                    inspect_git(root, Some(&request.name)).await?
                }
            }
        }
        "git_diff" => {
            let request: Diff = args(value)?;
            if !request.paths.is_empty() {
                crate::workspace_ui::git_paths(&request.paths)?;
            }
            let staged = matches!(request.kind, DiffKind::Staged);
            // A filtered diff cannot authorize a whole-index commit pin.
            let before = if staged {
                Some(git(root, &["write-tree"]).await?.trim().to_string())
            } else {
                None
            };
            let mut command = vec![
                "diff",
                "--relative",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
            ];
            match request.kind {
                DiffKind::Working => (),
                DiffKind::Staged => command.push("--cached"),
                DiffKind::All => {
                    if git(root, &["rev-parse", "--verify", "HEAD"]).await.is_err() {
                        return Err("All-versus-HEAD diff needs a first commit; read staged and working diffs separately.".into());
                    }
                    command.push("HEAD");
                }
            }
            command.push("--");
            command.extend(request.paths.iter().map(String::as_str));
            if request.paths.is_empty() {
                command.push(".");
            }
            let mut inventory = command.clone();
            inventory.insert(1, "--name-only");
            inventory.insert(2, "-z");
            checked_disk_inventory(root, git(root, &inventory).await?).await?;
            let diff = git(root, &command).await?;
            let after = if staged {
                Some(git(root, &["write-tree"]).await?.trim().to_string())
            } else {
                None
            };
            if before != after {
                return Err("Staged changes changed while reading the diff. Refresh before requesting a commit.".into());
            }
            let repository = if staged {
                Some(git(root, &["rev-parse", "--show-toplevel"]).await?)
            } else {
                None
            };
            let root_review = repository.as_ref().is_some_and(|p| {
                match (std::fs::canonicalize(root), std::fs::canonicalize(p.trim())) {
                    (Ok(selected), Ok(repository)) => selected == repository,
                    _ => false,
                }
            });
            json!({"diff":diff,"truncated":diff.chars().count()>=200000,"paths":request.paths,
                "stagedTree":if root_review && request.paths.is_empty() && diff.chars().count()<200000{after}else{None}})
        }
        _ => unreachable!(),
    };
    // Mutation/status responses never smuggle an unrestricted whole-tree diff
    // past the explicit format/path checks in git_diff.
    if alias != "git_diff" {
        if let Some(object) = result.as_object_mut() {
            object.remove("diff");
        }
    }
    if let Some(status) = result.get("status").and_then(Value::as_str) {
        result["incomplete"] = json!(status.chars().count() >= 200000);
    }
    if result["repository"] == true && result["stagedTree"].is_null() {
        result["incomplete"] = json!(true);
        result["warnings"]=json!(["Staged-tree identity is unavailable. Refresh a staged diff before requesting a commit."]);
    }
    result["isError"] = json!(false);
    result["operation"] = json!(alias);
    if alias != "git_diff" && result.get("incomplete").is_some() {
        result["truncated"] = json!(result["incomplete"] == true);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn fixture() -> tempfile::TempDir {
        assert!(available(), "Git is required for the native Git fixture");
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_str().unwrap();
        for command in [
            vec!["init", "-b", "main"],
            vec!["config", "user.name", "LocalLM fixture"],
            vec!["config", "user.email", "fixture@example.invalid"],
            vec!["config", "commit.gpgSign", "false"],
            vec!["config", "core.hooksPath", ".fixture-empty-hooks"],
        ] {
            crate::workspace_ui::git(root, &command).await.unwrap();
        }
        dir
    }
    async fn call(root: &std::path::Path, alias: &str, value: Value) -> Result<Value, String> {
        let tool = crate::connectors::AgentTool::harness(alias)?;
        execute(
            root.to_str().unwrap(),
            alias,
            tool.prepare_arguments(value)?,
        )
        .await
    }
    #[tokio::test]
    async fn literal_stage_unstage_stale_pin_commit_branch_history_preserve_disk() {
        let dir = fixture().await;
        let root = dir.path();
        let filename = if cfg!(windows) {
            "literal file.txt"
        } else {
            ":(glob)literal.txt"
        };
        std::fs::write(root.join(filename), "one\n").unwrap();
        std::fs::write(root.join("preserved.txt"), "do not stage\n").unwrap();
        let staged = call(root, "git_stage", json!({"paths":[filename]}))
            .await
            .unwrap();
        assert!(
            staged["files"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["path"] == filename && f["status"] == "A "),
            "{staged}"
        );
        call(root, "git_unstage", json!({"paths":[filename]}))
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join(filename)).unwrap(),
            "one\n"
        );
        call(root, "git_stage", json!({"paths":[filename]}))
            .await
            .unwrap();
        let reviewed = call(root, "git_diff", json!({"kind":"staged"}))
            .await
            .unwrap();
        assert!(reviewed["diff"].as_str().unwrap().contains("+one"));
        let old = reviewed["stagedTree"].as_str().unwrap();
        assert!(call(
            root,
            "git_diff",
            json!({"kind":"staged","paths":[filename]})
        )
        .await
        .unwrap()["stagedTree"]
            .is_null());
        std::fs::write(root.join(filename), "two\n").unwrap();
        call(root, "git_stage", json!({"paths":[filename]}))
            .await
            .unwrap();
        assert!(call(
            root,
            "git_commit",
            json!({"message":"stale","expected_tree":old})
        )
        .await
        .unwrap_err()
        .contains("changed"));
        let reviewed = call(root, "git_diff", json!({"kind":"staged"}))
            .await
            .unwrap();
        call(
            root,
            "git_commit",
            json!({"message":"literal $(do-not-execute)","expected_tree":reviewed["stagedTree"]}),
        )
        .await
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("preserved.txt")).unwrap(),
            "do not stage\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join(filename)).unwrap(),
            "two\n"
        );
        let history = call(root, "git_history", json!({})).await.unwrap();
        assert_eq!(
            history["entries"][0]["subject"],
            "literal $(do-not-execute)"
        );
        assert_eq!(history["limitReached"], false);
        call(
            root,
            "git_branch",
            json!({"action":"create","name":"feature/native"}),
        )
        .await
        .unwrap();
        assert_eq!(
            call(
                root,
                "git_branch",
                json!({"action":"switch","name":"feature/native"})
            )
            .await
            .unwrap()["branch"],
            "feature/native"
        );
        std::fs::create_dir(root.join("nested")).unwrap();
        assert!(call(
            &root.join("nested"),
            "git_branch",
            json!({"action":"switch","name":"main"})
        )
        .await
        .unwrap_err()
        .contains("repository root"));
    }
    #[tokio::test]
    async fn schemas_authority_paths_sensitive_content_and_link_guards() {
        let dir = fixture().await;
        let root = dir.path();
        for alias in ALIASES {
            let tool = crate::connectors::AgentTool::harness(alias).unwrap();
            assert_eq!(
                tool.definition()["function"]["parameters"]["additionalProperties"],
                false
            );
            assert!(tool.prepare_arguments(json!({"allow":true})).is_err());
            assert_eq!(
                crate::capabilities::tool_capability(&tool),
                Some("git_tools")
            );
            assert!(
                !tool.concurrent_read(),
                "Git's index internals remain ordered: {alias}"
            );
        }
        let commit = crate::connectors::AgentTool::harness("git_commit").unwrap();
        assert!(
            commit.requires_approval(&json!({})),
            "a model can never self-confirm a commit"
        );
        assert!(commit
            .prepare_arguments(
                json!({"message":"x","expected_tree":"a".repeat(40),"confirmed":true})
            )
            .is_err());
        for path in [
            "../outside",
            "/absolute",
            ".git/config",
            "nested/.git/config",
        ] {
            assert!(call(root, "git_stage", json!({"paths":[path]}))
                .await
                .is_err());
        }
        assert!(call(
            root,
            "git_branch",
            json!({"action":"create","name":"--force"})
        )
        .await
        .is_err());
        std::fs::write(root.join(".env"), "TOP_SECRET=do-not-expose\n").unwrap();
        assert!(call(root, "git_stage", json!({"paths":[".env"]}))
            .await
            .is_err());
        crate::workspace_ui::git(root.to_str().unwrap(), &["add", "--", ".env"])
            .await
            .unwrap();
        assert!(
            call(root, "git_diff", json!({"kind":"staged"}))
                .await
                .is_err(),
            "protected bytes must not enter the model result"
        );
        let status = call(root, "git_status", json!({})).await.unwrap();
        assert!(!status.to_string().contains("TOP_SECRET"));
        assert!(status.get("diff").is_none());
        #[cfg(unix)]
        {
            std::fs::hard_link(root.join(".env"), root.join("alias.txt")).unwrap();
            assert!(call(root, "git_stage", json!({"paths":["alias.txt"]}))
                .await
                .unwrap_err()
                .contains("Hard-linked"));
            std::os::unix::fs::symlink(".env", root.join("link.txt")).unwrap();
            assert!(call(root, "git_stage", json!({"paths":["link.txt"]}))
                .await
                .is_err());
        }
        let outside = tempfile::tempdir().unwrap();
        crate::workspace_ui::git(
            root.to_str().unwrap(),
            &["config", "core.worktree", outside.path().to_str().unwrap()],
        )
        .await
        .unwrap();
        assert!(call(root, "git_diff", json!({}))
            .await
            .unwrap_err()
            .contains("outside the selected workspace"));
    }
    #[tokio::test]
    async fn actual_catalog_model_call_dispatch_and_stale_capability_rejection() {
        let dir = fixture().await;
        let root = dir.path();
        let state = crate::test_support::app_state(root);
        crate::test_support::select_fixture_model(&state, root);
        let conversation = state.database().unwrap().create_conversation().unwrap().id;
        let plan = crate::chat::assemble_turn(
            &state,
            &conversation,
            "Inspect Git",
            vec!["__workspace".into()],
            Some(vec![]),
            Some(crate::presets::STANDARD),
            None,
        )
        .await
        .unwrap();
        for alias in ALIASES {
            assert!(plan.tools.iter().any(|t| t.alias == alias), "{alias}");
        }
        let selected = plan.tools.iter().find(|t| t.alias == "git_status").unwrap();
        assert!(selected.trusted_read());
        let mut stream = crate::tool_calls::ToolCalls::default();
        stream.push(&json!({"tool_calls":[{"index":0,"id":"git-call","type":"function","function":{"name":"git_status","arguments":"{}"}}]})).unwrap();
        let calls = stream.finish().unwrap();
        let model_call = &calls[0];
        let aliases = plan
            .tools
            .iter()
            .map(|t| t.alias.as_str())
            .collect::<Vec<_>>();
        let ctx = crate::harness::HarnessCtx {
            state: &state,
            snapshot: crate::test_support::snapshot(&state, root, &aliases),
            conversation_id: conversation.clone(),
            run_id: "git-fixture".into(),
            step_id: "git-step".into(),
            access_mode: crate::permissions::AccessMode::FullAccess,
            depth: 0,
            preset_id: crate::presets::STANDARD.into(),
            inherit_tools: &plan.tools,
            emit: std::sync::Arc::new(|_, _| {}),
        };
        let result = crate::harness::execute(
            &ctx,
            &model_call.name,
            selected
                .prepare_arguments(model_call.arguments.clone())
                .unwrap(),
        )
        .await
        .unwrap();
        let normalized =
            crate::tool_envelope::normalize(result.value, crate::tool_envelope::Family::Harness);
        assert!(!crate::tool_envelope::failed(&normalized), "{normalized}");
        state
            .database()
            .unwrap()
            .save_setting(
                crate::capabilities::DISABLED_KEY,
                &vec!["workspace_files".to_string()],
            )
            .unwrap();
        assert!(crate::harness::execute(&ctx, "git_status", json!({}))
            .await
            .err()
            .unwrap()
            .contains("disabled"));
        for capability in ["git_tools", "local_execution"] {
            state
                .database()
                .unwrap()
                .save_setting(
                    crate::capabilities::DISABLED_KEY,
                    &vec![capability.to_string()],
                )
                .unwrap();
            assert!(
                crate::harness::execute(&ctx, "git_status", json!({}))
                    .await
                    .err()
                    .unwrap()
                    .contains("disabled"),
                "{capability}"
            );
            let killed = crate::chat::assemble_turn(
                &state,
                &conversation,
                "Inspect Git",
                vec!["__workspace".into()],
                Some(vec![]),
                Some(crate::presets::STANDARD),
                None,
            )
            .await
            .unwrap();
            assert!(!killed.tools.iter().any(|t| owns(&t.alias)), "{capability}");
        }
        assert!(!crate::presets::harness_source_allowed(
            "git_status",
            false,
            true
        ));
        assert!(crate::presets::harness_source_allowed(
            "git_status",
            true,
            false
        ));
        assert!(!crate::presets::get(crate::presets::MINIMAL)
            .unwrap()
            .harness
            .iter()
            .any(|a| owns(a)));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn verbose_git_failure_is_drained_and_never_becomes_success() {
        let dir = fixture().await;
        let root = dir.path().to_str().unwrap();
        crate::workspace_ui::git(root,&["config","alias.fixture-verbose-failure",
            "!printf 'fixture-failed\\n' >&2; printf '%0900000d' 0; printf '%020000d' 0 >&2; exit 17"]).await.unwrap();
        let failure = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            crate::workspace_ui::git(root, &["fixture-verbose-failure"]),
        )
        .await
        .expect("both pipes must drain without blocking a full stderr pipe")
        .unwrap_err();
        assert!(failure.contains("fixture-failed"), "{failure}");
        assert!(failure.len() <= 2000);
        use std::os::unix::fs::PermissionsExt;
        let hook = dir.path().join("fsmonitor-hook");
        std::fs::write(&hook, "#!/bin/sh\nprintf invoked > fsmonitor-invoked\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o700)).unwrap();
        crate::workspace_ui::git(root, &["config", "core.fsmonitor", hook.to_str().unwrap()])
            .await
            .unwrap();
        call(dir.path(), "git_status", json!({})).await.unwrap();
        assert!(
            !dir.path().join("fsmonitor-invoked").exists(),
            "trusted Git reads cannot run fsmonitor scripts"
        );
    }

    #[tokio::test]
    async fn detached_reconstruction_preserves_pin_and_capability_and_pending_journals_block_git() {
        let dir = fixture().await;
        let root = dir.path();
        let state = crate::test_support::app_state(root);
        let conversation = state.database().unwrap().create_conversation().unwrap().id;
        let mut snapshot = crate::test_support::snapshot(&state, root, &ALIASES);
        let policy = crate::capabilities::ToolPolicy::guard(state.database());
        let tools = crate::subagents::local_tools(&snapshot, &policy);
        for alias in ALIASES {
            assert!(tools.iter().any(|t| t.alias == alias), "{alias}");
        }
        assert!(tools
            .iter()
            .find(|t| t.alias == "git_commit")
            .unwrap()
            .requires_approval(&json!({})));
        let request = crate::subagents::ChildRequest {
            label: "Git read fixture".into(),
            prompt: "Inspect status".into(),
            depth: 0,
            max_rounds: 1,
            tool_filter: None,
            persona: None,
            output_schema: None,
            policy: crate::subagents::ChildPolicy::ReadsOnly,
            parent_run_id: "parent".into(),
            conversation_id: conversation,
        };
        let status =
            crate::subagents::harness_detached(&snapshot, &request, "git_status", json!({}))
                .await
                .unwrap();
        assert_eq!(status["repository"], true);
        snapshot.workspace_path.clear();
        assert!(!crate::subagents::local_tools(&snapshot, &policy)
            .iter()
            .any(|t| owns(&t.alias)));
        std::fs::create_dir(root.join(".locallm-transactions")).unwrap();
        std::fs::write(
            root.join(".locallm-transactions")
                .join(format!("{}.json", "a".repeat(32))),
            "pending",
        )
        .unwrap();
        assert!(call(root, "git_status", json!({}))
            .await
            .unwrap_err()
            .contains("pending transaction"));
        state
            .database()
            .unwrap()
            .save_setting(
                crate::capabilities::DISABLED_KEY,
                &vec!["workspace_files".to_string()],
            )
            .unwrap();
        snapshot.workspace_path = root.to_str().unwrap().into();
        let disabled = crate::capabilities::ToolPolicy::guard(state.database());
        assert!(!crate::subagents::local_tools(&snapshot, &disabled)
            .iter()
            .any(|t| owns(&t.alias)));
    }

    #[tokio::test]
    async fn large_staged_diff_keeps_usable_pin_after_model_result_bounding() {
        let dir = fixture().await;
        let root = dir.path();
        std::fs::write(root.join("large.txt"), "bounded diff line\n".repeat(3000)).unwrap();
        call(root, "git_stage", json!({"paths":["large.txt"]}))
            .await
            .unwrap();
        let reviewed = call(root, "git_diff", json!({"kind":"staged"}))
            .await
            .unwrap();
        let normalized = crate::tool_envelope::normalize(
            reviewed.clone(),
            crate::tool_envelope::Family::Harness,
        );
        for budget in [100, 512, 1024, 2048, 8192] {
            let (out, artifact) = crate::artifacts::bound_tool_result(
                normalized.clone(),
                "git_diff",
                "fixture",
                None,
                budget,
            );
            assert!(out.to_string().len() <= budget.max(crate::artifacts::MIN_RESULT_BUDGET));
            assert_eq!(out["data"]["stagedTree"], reviewed["stagedTree"], "{out}");
            assert_eq!(out["data"]["reviewIncomplete"], true);
            assert!(crate::tool_envelope::validate_envelope(&out).is_ok());
            let full: Value = serde_json::from_str(&artifact.unwrap().content).unwrap();
            assert_eq!(full, crate::tool_envelope::model_result(&normalized));
        }
    }
}
