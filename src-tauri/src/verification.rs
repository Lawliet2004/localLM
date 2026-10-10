//! Evidence-backed completion: criteria the work must satisfy and records of
//! the checks that actually ran (shell checks, research verification,
//! calculations). The completion state is computed from records only — what
//! the model *said* never counts as evidence.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// A checkable condition for the work in one conversation. `origin` marks
/// whether the user wrote it or the agent proposed it — agent-proposed
/// criteria are labeled in the UI so self-grading stays visible.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskCriterion {
    pub id: String,
    pub conversation_id: String,
    pub text: String,
    pub origin: String,
    pub created_at: i64,
    pub updated_at: i64,
}

/// One check that ran — or tried to. `stale` is computed at read time by
/// comparing `fingerprint` against the current workspace tree; it is never
/// stored.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationRecord {
    pub id: String,
    pub conversation_id: String,
    pub criterion_id: Option<String>,
    /// command | browser | research | calculation | data | manual
    pub kind: String,
    /// How the check ran ("shell command", "claim verifier (research)", ...).
    pub method: String,
    pub command: Option<String>,
    /// native-local | task-worktree | snapshot | mocked — distinguishes a
    /// real run from a fixture or a check against the isolated task copy.
    pub environment: String,
    pub started_at: i64,
    pub ended_at: i64,
    pub exit_code: Option<i32>,
    /// Tail of combined output, at most 8 KiB. The UI must not present this
    /// as the full log.
    pub output_excerpt: String,
    /// Artifact id holding the complete captured output, when one exists.
    pub output_artifact: Option<String>,
    pub output_truncated: bool,
    /// Structured extras — e.g. claim→source mappings for research records.
    pub artifacts: Value,
    /// passed | failed | unavailable | inconclusive
    pub result: String,
    pub limitations: String,
    /// Hash of the workspace tree state this record attests to.
    pub fingerprint: Option<String>,
    pub created_at: i64,
    #[serde(default)]
    pub stale: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompletionState {
    pub state: String,
    pub summary: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VerificationSummary {
    pub criteria: Vec<TaskCriterion>,
    pub records: Vec<VerificationRecord>,
    pub state: CompletionState,
}

fn state(value: &str, summary: impl Into<String>) -> CompletionState {
    CompletionState {
        state: value.to_string(),
        summary: summary.into(),
    }
}

/// Roll the criteria and records up into one honest state. `records` must
/// already carry their computed `stale` flags. The model's own claims are not
/// an input: a turn that says "done" while its check failed stays
/// `checks-failed`.
pub fn completion_state(
    criteria: &[TaskCriterion],
    records: &[VerificationRecord],
) -> CompletionState {
    if records.is_empty() {
        return state(
            "work-completed",
            "Work completed — nothing has been verified.",
        );
    }
    let fresh = |record: &&VerificationRecord| !record.stale;
    if records
        .iter()
        .any(|record| fresh(&record) && record.result == "failed")
    {
        let failed = records
            .iter()
            .filter(|record| fresh(record) && record.result == "failed")
            .count();
        return state(
            "checks-failed",
            format!(
                "{failed} check{} failed.",
                if failed == 1 { "" } else { "s" }
            ),
        );
    }
    let covered = |criterion: &TaskCriterion| {
        records.iter().any(|record| {
            fresh(&record)
                && record.result == "passed"
                && record.criterion_id.as_deref() == Some(criterion.id.as_str())
        })
    };
    let all_fresh_passes = records
        .iter()
        .all(|record| fresh(&record) && record.result == "passed");
    if all_fresh_passes && criteria.iter().all(covered) {
        return state(
            "checks-passed",
            if criteria.is_empty() {
                "All recorded checks passed.".to_string()
            } else {
                format!("All {} criteria verified.", criteria.len())
            },
        );
    }
    if records.iter().any(|record| {
        record.stale || record.result == "unavailable" || record.result == "inconclusive"
    }) {
        return state(
            "needs-review",
            "Checks are stale, unavailable, or inconclusive — review before trusting the work.",
        );
    }
    let verified = criteria
        .iter()
        .filter(|criterion| covered(criterion))
        .count();
    state(
        "partially-verified",
        format!("{verified} of {} criteria verified.", criteria.len()),
    )
}

/// SHA-256 of the workspace tree state. Inside any git work tree (the source
/// repo or an isolated task worktree) this is the tree hash of `git add -A`
/// staged through a temporary index — the real index is never touched. For
/// non-git folders it is a bounded sorted (path, size, mtime) walk. `None`
/// when the root is empty or unreadable.
pub(crate) fn workspace_fingerprint(root: &Path) -> Option<String> {
    if root.as_os_str().is_empty() || !root.is_dir() {
        return None;
    }
    if let Some(tree) = git_tree_hash(root) {
        return Some(format!("tree:{tree}"));
    }
    walk_hash(root)
}

fn git_tree_hash(root: &Path) -> Option<String> {
    if crate::task_workspace::run_git(root, &["rev-parse", "--is-inside-work-tree"])
        .map(|inside| inside.trim() == "true")
        != Ok(true)
    {
        return None;
    }
    let index = std::env::temp_dir().join(format!(
        "localllm-verify-{}.idx",
        uuid::Uuid::new_v4().simple()
    ));
    let run = |args: &[&str]| crate::task_workspace::run_git_indexed(root, &index, args);
    // Seed the temp index from HEAD so tracked files start as their committed
    // state; `read-tree --empty` covers repos without a first commit.
    let seeded = run(&["read-tree", "HEAD"]).is_ok() || run(&["read-tree", "--empty"]).is_ok();
    let result = if seeded {
        run(&["add", "-A", "--ignore-errors"])
            .ok()
            .and_then(|_| run(&["write-tree"]).ok())
    } else {
        None
    };
    let _ = std::fs::remove_file(&index);
    result.map(|oid| oid.trim().to_string())
}

/// Bounded deterministic walk for non-git roots. Skips `.git` and generated
/// directories; caps at 50,000 entries and folds an overflow marker into the
/// hash so a capped scan is still stable and identifiable.
fn walk_hash(root: &Path) -> Option<String> {
    const LIMIT: usize = 50_000;
    let mut entries: Vec<(String, u64, i64)> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let mut overflow = false;
    while let Some(dir) = stack.pop() {
        let Ok(children) = std::fs::read_dir(&dir) else {
            continue;
        };
        for child in children.flatten() {
            if entries.len() >= LIMIT {
                overflow = true;
                break;
            }
            let name = child.file_name();
            let name = name.to_string_lossy();
            if matches!(
                name.as_ref(),
                ".git" | "node_modules" | "target" | ".venv" | "__pycache__" | "dist"
            ) {
                continue;
            }
            let path = child.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                entries.push((rel, meta.len(), mtime));
            }
        }
    }
    // An empty tree is a legitimate state — hash it like any other.
    entries.sort();
    let mut hasher = Sha256::new();
    for (path, size, mtime) in &entries {
        hasher.update(path.as_bytes());
        hasher.update(b"\0");
        hasher.update(size.to_le_bytes());
        hasher.update(mtime.to_le_bytes());
    }
    if overflow {
        hasher.update(b"truncated");
    }
    Some(format!("walk:{:x}", hasher.finalize()))
}

/// Which environment a record ran in. `mocked` exists for test fixtures; real
/// runs are always one of the other three.
pub(crate) fn environment_label(
    task: Option<&crate::task_workspace::TaskWorkspace>,
) -> &'static str {
    match task.map(|task| task.kind.as_str()) {
        Some("git-worktree") => "task-worktree",
        Some("snapshot") => "snapshot",
        _ => "native-local",
    }
}

/// Conversation's effective workspace root for verification runs — the
/// isolated task root when a task is active, the project/workspace otherwise.
fn check_root(
    store: &crate::store::Store,
    conversation_id: &str,
) -> Result<(PathBuf, String), String> {
    let task = store.active_task_workspace(conversation_id)?;
    let root = task
        .as_ref()
        .map(|task| task.isolated_root.clone())
        .or_else(|| {
            crate::workspace_ui::project_workspace(store, conversation_id)
                .ok()
                .flatten()
        })
        .filter(|root| !root.trim().is_empty())
        .unwrap_or_else(|| store.workspace_path().unwrap_or_default());
    if root.trim().is_empty() {
        return Err("Choose a workspace folder before running checks.".into());
    }
    Ok((
        PathBuf::from(root),
        environment_label(task.as_ref()).to_string(),
    ))
}

/// Insert a record. `criterion_id` must belong to the same conversation when
/// set, so a check cannot quietly endorse another conversation's criterion.
pub fn save_record(
    store: &crate::store::Store,
    mut record: VerificationRecord,
) -> Result<VerificationRecord, String> {
    if let Some(criterion_id) = &record.criterion_id {
        match store.criterion(criterion_id)? {
            Some(criterion) if criterion.conversation_id == record.conversation_id => {}
            _ => return Err("Unknown criterion for this conversation.".into()),
        }
    }
    if let Ok(root) = check_root(store, &record.conversation_id) {
        record.fingerprint = workspace_fingerprint(&root.0);
    }
    record.created_at = crate::store::now();
    store.save_verification_record(&record)?;
    Ok(record)
}

/// Persist a research-tool verification report as a record: claim→source
/// mappings are kept verbatim; the result is `passed` only when the claim
/// verifier certified every claim, otherwise `inconclusive`.
pub fn record_research(store: &crate::store::Store, conversation_id: &str, verification: &Value) {
    let claims = verification
        .get("claims")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if claims.is_empty() {
        return;
    }
    let all_supported = verification
        .get("allSupported")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let record = VerificationRecord {
        id: format!("vr_{}", uuid::Uuid::new_v4().simple()),
        conversation_id: conversation_id.to_string(),
        criterion_id: None,
        kind: "research".into(),
        method: "claim verifier (research)".into(),
        command: None,
        environment: store
            .active_task_workspace(conversation_id)
            .ok()
            .flatten()
            .map(|task| environment_label(Some(&task)).to_string())
            .unwrap_or_else(|| "native-local".into()),
        started_at: crate::store::now(),
        ended_at: crate::store::now(),
        exit_code: None,
        output_excerpt: format!(
            "{} claims checked · {} supported · {} unsupported · {} conflicting",
            claims.len(),
            verification
                .get("supportedCount")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            verification
                .get("unsupportedCount")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            verification
                .get("conflictingCount")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        ),
        output_artifact: None,
        output_truncated: false,
        artifacts: json!(claims),
        result: if all_supported {
            "passed"
        } else {
            "inconclusive"
        }
        .into(),
        limitations: "Citation match is heuristic; it does not prove the claim.".into(),
        fingerprint: None,
        created_at: crate::store::now(),
        stale: false,
    };
    // save_record would re-check the criterion (none) and fingerprint (not
    // workspace-backed → stays None); write directly to keep it cheap.
    let _ = store.save_verification_record(&record);
}

/// Persist a run_code outcome as a `calculation` record when the agent tied
/// the call to a criterion.
pub fn record_calculation(
    store: &crate::store::Store,
    conversation_id: &str,
    criterion_id: &str,
    result: &Value,
) {
    let exit_code = result
        .get("exitCode")
        .and_then(Value::as_i64)
        .map(|code| code as i32);
    let output = format!(
        "{}{}",
        result.get("stdout").and_then(Value::as_str).unwrap_or(""),
        result.get("stderr").and_then(Value::as_str).unwrap_or(""),
    );
    let excerpt: String = tail_chars(&output, 8 * 1024);
    let _ = save_record(
        store,
        VerificationRecord {
            id: format!("vr_{}", uuid::Uuid::new_v4().simple()),
            conversation_id: conversation_id.to_string(),
            criterion_id: Some(criterion_id.to_string()),
            kind: "calculation".into(),
            method: "local_run_code".into(),
            command: None,
            environment: check_root(store, conversation_id)
                .map(|(_, env)| env)
                .unwrap_or_else(|_| "native-local".into()),
            started_at: crate::store::now(),
            ended_at: crate::store::now(),
            exit_code,
            output_excerpt: excerpt,
            output_artifact: None,
            output_truncated: output.chars().count() > 8 * 1024,
            artifacts: json!([]),
            result: match exit_code {
                Some(0) => "passed",
                Some(_) => "failed",
                None => "unavailable",
            }
            .into(),
            limitations: "Exit code only; the output is not independently checked.".into(),
            fingerprint: None,
            created_at: crate::store::now(),
            stale: false,
        },
    );
}

fn tail_chars(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    text.chars().skip(count - max).collect()
}

const CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);
const STREAM_CAP: usize = 4 * 1024 * 1024;

struct CheckOutput {
    exit_code: Option<i32>,
    output: String,
    truncated_stream: bool,
    limitation: String,
}

/// Drain a child stream, keeping at most `STREAM_CAP` bytes but consuming to
/// the end so a chatty process never deadlocks on a full pipe.
async fn drain_bounded(mut stream: impl tokio::io::AsyncRead + Unpin) -> Vec<u8> {
    use tokio::io::AsyncReadExt;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let keep = STREAM_CAP.saturating_sub(buffer.len()).min(n);
                buffer.extend_from_slice(&chunk[..keep]);
            }
        }
    }
    buffer
}

/// Run a shell check command in the workspace, bounded by the same spawn
/// policy as local execution (clean env, job object / process group,
/// kill-on-drop). Full captured output goes to an artifact; the record keeps
/// an 8 KiB tail excerpt.
async fn run_shell_check(
    shell: &str,
    root: &Path,
    command: &str,
    isolation: &crate::isolation::IsolationSettings,
    db_path: Option<&Path>,
) -> CheckOutput {
    let spec = crate::isolation::SpawnSpec {
        program: if cfg!(windows) {
            if !shell.is_empty() && Path::new(shell).is_file() {
                shell.to_string()
            } else {
                "powershell.exe".into()
            }
        } else {
            "sh".into()
        },
        args: if cfg!(windows) {
            vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), command.into()]
        } else {
            vec!["-c".into(), command.into()]
        },
        cwd: root.to_path_buf(),
        env: crate::isolation::SpawnSpec::standard_env(&[]),
        stdin: false,
        stderr: true,
        writable: vec![root.to_path_buf()],
        cpu_seconds: CHECK_TIMEOUT.as_secs() as u32,
    };
    let mut child = match crate::isolation::spawn(&spec, isolation, db_path) {
        Ok(child) => child,
        Err(error) => {
            return CheckOutput {
                exit_code: None,
                output: String::new(),
                truncated_stream: false,
                limitation: format!("Check could not start: {error}"),
            };
        }
    };
    let stdout = child.stdout().take();
    let stderr = child.stderr().take();
    let wait = async {
        let out_fut = async move {
            match stdout {
                Some(stream) => drain_bounded(stream).await,
                None => Vec::new(),
            }
        };
        let err_fut = async move {
            match stderr {
                Some(stream) => drain_bounded(stream).await,
                None => Vec::new(),
            }
        };
        let (out, err, status) = tokio::join!(out_fut, err_fut, child.wait());
        (out, err, status)
    };
    let timed_out;
    let (out, err, status) = match tokio::time::timeout(CHECK_TIMEOUT, wait).await {
        Ok(value) => {
            timed_out = false;
            value
        }
        Err(_) => {
            timed_out = true;
            let _ = child.start_kill();
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await;
            (
                Vec::new(),
                Vec::new(),
                Err(std::io::Error::other("timed out")),
            )
        }
    };
    drop(child);
    let truncated_stream = out.len() >= STREAM_CAP || err.len() >= STREAM_CAP;
    let mut output = String::from("== stdout ==\n");
    output.push_str(&String::from_utf8_lossy(&out));
    output.push_str("\n== stderr ==\n");
    output.push_str(&String::from_utf8_lossy(&err));
    if truncated_stream {
        output.push_str("\n[output truncated at 4 MiB per stream]");
    }
    match (timed_out, status) {
        (true, _) => CheckOutput {
            exit_code: None,
            output,
            truncated_stream,
            limitation: "Check timed out after 10 minutes.".into(),
        },
        (false, Ok(status)) => CheckOutput {
            exit_code: status.code(),
            output,
            truncated_stream,
            limitation: String::new(),
        },
        (false, Err(error)) => CheckOutput {
            exit_code: None,
            output,
            truncated_stream,
            limitation: format!("Check did not finish: {error}"),
        },
    }
}

/// Shared implementation for the `verify_run` tool and the user-initiated
/// "Run check" command.
pub async fn run_check(
    store_db: &Path,
    conversation_id: &str,
    command: &str,
    criterion_id: Option<&str>,
    kind: &str,
) -> Result<VerificationRecord, String> {
    let command = command.trim();
    if command.is_empty() || command.len() > 4096 {
        return Err("A check command must be 1-4096 characters.".into());
    }
    if let Some(criterion_id) = criterion_id {
        let store = crate::store::Store::open(store_db)?;
        match store.criterion(criterion_id)? {
            Some(criterion) if criterion.conversation_id == conversation_id => {}
            _ => return Err("Unknown criterion for this conversation.".into()),
        }
    }
    let (store, root, environment, shell, isolation) = {
        let store = crate::store::Store::open(store_db)?;
        let (root, environment) = check_root(&store, conversation_id)?;
        if !root.is_dir() {
            return Err(format!(
                "Workspace folder {} no longer exists.",
                root.display()
            ));
        }
        let shell = store.execution_config()?.powershell_path;
        let isolation = crate::isolation::settings(&store);
        (store, root, environment, shell, isolation)
    };
    // Same guard as other execution: snapshot the isolated task repo before a
    // command can mutate it, so there is always a pre-check state to restore.
    crate::task_workspace::auto_checkpoint(&store, &root.to_string_lossy(), "verify_run");
    let started = crate::store::now();
    let output = run_shell_check(&shell, &root, command, &isolation, Some(store_db)).await;
    let ended = crate::store::now();

    let artifact_text = output.output;
    let size = artifact_text.len();
    let artifact = crate::artifacts::ArtifactRecord {
        id: format!("art_{}", uuid::Uuid::new_v4().simple()),
        conversation_id: conversation_id.to_string(),
        run_id: None,
        tool_name: "verify_run".into(),
        mime_type: "text/plain".into(),
        size_bytes: size,
        sha256: format!("{:x}", Sha256::digest(artifact_text.as_bytes())),
        content: artifact_text.clone(),
        created_at: ended,
    };
    let store = crate::store::Store::open(store_db)?;
    store.save_artifact(&artifact)?;
    let excerpt = tail_chars(&artifact_text, 8 * 1024);
    let record = VerificationRecord {
        id: format!("vr_{}", uuid::Uuid::new_v4().simple()),
        conversation_id: conversation_id.to_string(),
        criterion_id: criterion_id.map(str::to_string),
        kind: kind.to_string(),
        method: if cfg!(windows) {
            "shell command (powershell)"
        } else {
            "shell command (sh)"
        }
        .into(),
        command: Some(command.to_string()),
        environment,
        started_at: started,
        ended_at: ended,
        exit_code: output.exit_code,
        output_excerpt: excerpt,
        output_artifact: Some(artifact.id),
        output_truncated: output.truncated_stream || artifact_text.chars().count() > 8 * 1024,
        artifacts: json!([]),
        result: match output.exit_code {
            Some(0) => "passed",
            Some(_) => "failed",
            None => "unavailable",
        }
        .into(),
        limitations: output.limitation,
        fingerprint: None, // computed in save_record, post-run tree state
        created_at: ended,
        stale: false,
    };
    save_record(&store, record)
}

fn summary_impl(
    store: &crate::store::Store,
    conversation_id: &str,
) -> Result<VerificationSummary, String> {
    let criteria = store.criteria_for(conversation_id)?;
    let mut records = store.verification_records_for(conversation_id)?;
    let current = check_root(store, conversation_id)
        .ok()
        .and_then(|(root, _)| workspace_fingerprint(&root));
    for record in &mut records {
        record.stale = match (&record.fingerprint, &current) {
            (Some(stored), Some(now)) => stored != now,
            _ => false,
        };
    }
    let state = completion_state(&criteria, &records);
    Ok(VerificationSummary {
        criteria,
        records,
        state,
    })
}

pub fn propose(
    store: &crate::store::Store,
    conversation_id: &str,
    origin: &str,
    criteria: &[String],
) -> Result<Vec<TaskCriterion>, String> {
    if criteria.is_empty() || criteria.len() > 20 {
        return Err("Provide 1-20 criteria.".into());
    }
    let now = crate::store::now();
    let mut saved = Vec::new();
    for text in criteria {
        let text = text.trim();
        if text.is_empty() || text.len() > 500 {
            return Err("Each criterion must be 1-500 characters.".into());
        }
        let criterion = TaskCriterion {
            id: format!("cr_{}", uuid::Uuid::new_v4().simple()),
            conversation_id: conversation_id.to_string(),
            text: text.to_string(),
            origin: origin.to_string(),
            created_at: now,
            updated_at: now,
        };
        store.save_criterion(&criterion)?;
        saved.push(criterion);
    }
    Ok(saved)
}

#[tauri::command]
pub async fn verification_summary(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
) -> Result<VerificationSummary, String> {
    let db_path = state.db_path.clone();
    tokio::task::spawn_blocking(move || {
        summary_impl(&crate::store::Store::open(&db_path)?, &conversation_id)
    })
    .await
    .map_err(|_| "Verification summary stopped.".to_string())?
}

#[tauri::command]
pub async fn criteria_upsert(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
    id: Option<String>,
    text: String,
) -> Result<TaskCriterion, String> {
    let text = text.trim().to_string();
    if text.is_empty() || text.len() > 500 {
        return Err("A criterion must be 1-500 characters.".into());
    }
    let db_path = state.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let store = crate::store::Store::open(&db_path)?;
        let now = crate::store::now();
        let criterion = match id {
            Some(id) => {
                let mut existing = store
                    .criterion(&id)?
                    .filter(|criterion| criterion.conversation_id == conversation_id)
                    .ok_or("Unknown criterion for this conversation.")?;
                existing.text = text.clone();
                existing.updated_at = now;
                existing
            }
            None => TaskCriterion {
                id: format!("cr_{}", uuid::Uuid::new_v4().simple()),
                conversation_id: conversation_id.clone(),
                text: text.clone(),
                origin: "user".into(),
                created_at: now,
                updated_at: now,
            },
        };
        store.save_criterion(&criterion)?;
        Ok(criterion)
    })
    .await
    .map_err(|_| "Saving the criterion stopped.".to_string())?
}

#[tauri::command]
pub async fn criteria_delete(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
    id: String,
) -> Result<(), String> {
    let db_path = state.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let store = crate::store::Store::open(&db_path)?;
        match store.criterion(&id)? {
            Some(criterion) if criterion.conversation_id == conversation_id => {
                store.delete_criterion(&id)
            }
            _ => Err("Unknown criterion for this conversation.".into()),
        }
    })
    .await
    .map_err(|_| "Deleting the criterion stopped.".to_string())?
}

#[tauri::command]
pub async fn verification_run_user(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
    command: String,
    criterion_id: Option<String>,
) -> Result<VerificationRecord, String> {
    let db_path = state.db_path.clone();
    run_check(
        &db_path,
        &conversation_id,
        &command,
        criterion_id.as_deref(),
        "command",
    )
    .await
}

#[tauri::command]
pub fn verification_record_output(
    state: tauri::State<'_, crate::AppState>,
    record_id: String,
) -> Result<Value, String> {
    let store = state.database()?;
    let record = store
        .verification_record(&record_id)?
        .ok_or("Unknown verification record.")?;
    let Some(artifact_id) = record.output_artifact else {
        return Ok(
            json!({"text": record.output_excerpt, "truncated": false, "note": "Only the saved excerpt is available — no full log was captured."}),
        );
    };
    let artifact = store
        .artifact(&artifact_id)?
        .ok_or("The recorded log artifact is gone.")?;
    // Bounded read: the UI labels anything past the cap as truncated instead
    // of quietly showing a partial log as complete.
    const FULL_LOG_CAP: usize = 2 * 1024 * 1024;
    if artifact.content.len() > FULL_LOG_CAP {
        let text: String = artifact.content.chars().take(FULL_LOG_CAP).collect();
        Ok(
            json!({"text": text, "truncated": true, "note": "Showing the first 2 MiB of the saved log."}),
        )
    } else {
        Ok(json!({"text": artifact.content, "truncated": false}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn criterion(id: &str) -> TaskCriterion {
        TaskCriterion {
            id: id.into(),
            conversation_id: "c".into(),
            text: id.into(),
            origin: "user".into(),
            created_at: 0,
            updated_at: 0,
        }
    }
    fn record(
        id: &str,
        criterion_id: Option<&str>,
        result: &str,
        stale: bool,
    ) -> VerificationRecord {
        VerificationRecord {
            id: id.into(),
            conversation_id: "c".into(),
            criterion_id: criterion_id.map(str::to_string),
            kind: "command".into(),
            method: "test".into(),
            command: None,
            environment: "native-local".into(),
            started_at: 0,
            ended_at: 0,
            exit_code: None,
            output_excerpt: String::new(),
            output_artifact: None,
            output_truncated: false,
            artifacts: json!([]),
            result: result.into(),
            limitations: String::new(),
            fingerprint: None,
            created_at: 0,
            stale,
        }
    }

    #[test]
    fn completion_state_matrix() {
        let criteria = vec![criterion("a"), criterion("b")];
        // No records: done-but-unverified regardless of what the model says.
        assert_eq!(completion_state(&criteria, &[]).state, "work-completed");
        assert_eq!(completion_state(&[], &[]).state, "work-completed");
        // A non-stale failure wins over everything — including a model that
        // reported success.
        assert_eq!(
            completion_state(
                &criteria,
                &[
                    record("r1", Some("a"), "passed", false),
                    record("r2", Some("b"), "failed", false)
                ]
            )
            .state,
            "checks-failed"
        );
        assert_eq!(
            completion_state(&[], &[record("r1", None, "failed", false)]).state,
            "checks-failed"
        );
        // Every criterion with a fresh passing record -> checks-passed.
        assert_eq!(
            completion_state(
                &criteria,
                &[
                    record("r1", Some("a"), "passed", false),
                    record("r2", Some("b"), "passed", false)
                ]
            )
            .state,
            "checks-passed"
        );
        // A stale pass does not cover a criterion.
        assert_eq!(
            completion_state(
                &criteria,
                &[
                    record("r1", Some("a"), "passed", true),
                    record("r2", Some("b"), "passed", false)
                ]
            )
            .state,
            "needs-review"
        );
        assert_eq!(
            completion_state(&[], &[record("r1", None, "inconclusive", false)]).state,
            "needs-review"
        );
        assert_eq!(
            completion_state(&[], &[record("r1", None, "unavailable", false)]).state,
            "needs-review"
        );
        // Half-covered criteria without stale/inconclusive records.
        assert_eq!(
            completion_state(&criteria, &[record("r1", Some("a"), "passed", false)]).state,
            "partially-verified"
        );
        // An unlinked pass with no criteria still reports all checks green.
        assert_eq!(
            completion_state(&[], &[record("r1", None, "passed", false)]).state,
            "checks-passed"
        );
    }

    #[test]
    fn fingerprint_changes_when_the_tree_changes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("a.txt"), "one").unwrap();
        let first = workspace_fingerprint(root).unwrap();
        std::fs::write(root.join("a.txt"), "two").unwrap();
        let second = workspace_fingerprint(root).unwrap();
        assert_ne!(first, second);
        std::fs::write(root.join("b.txt"), "new").unwrap();
        assert_ne!(workspace_fingerprint(root).unwrap(), second);
        assert!(workspace_fingerprint(Path::new("")).is_none());
    }

    #[test]
    fn git_fingerprint_covers_uncommitted_and_untracked_files() {
        if crate::task_workspace::require_git().is_err() {
            eprintln!("git not on PATH; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        crate::task_workspace::run_git(root, &["init", "-q"]).unwrap();
        crate::task_workspace::run_git(
            root,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "init",
                "--allow-empty",
            ],
        )
        .unwrap();
        std::fs::write(root.join("tracked.txt"), "committed").unwrap();
        crate::task_workspace::run_git(root, &["add", "tracked.txt"]).unwrap();
        crate::task_workspace::run_git(
            root,
            &[
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@t",
                "commit",
                "-qm",
                "add",
            ],
        )
        .unwrap();
        let clean = workspace_fingerprint(root).unwrap();
        assert!(clean.starts_with("tree:"));
        // The real index is untouched by fingerprinting.
        let index_before = std::fs::read(root.join(".git/index")).unwrap();
        std::fs::write(root.join("untracked.txt"), "loose").unwrap();
        let dirty = workspace_fingerprint(root).unwrap();
        assert_ne!(clean, dirty);
        let index_after = std::fs::read(root.join(".git/index")).unwrap();
        assert_eq!(index_before, index_after);
    }

    fn run<F: std::future::Future>(fut: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(fut)
    }

    /// File-backed store + workspace fixture; `run_check` reopens the store
    /// by path, so in-memory databases cannot exercise it.
    fn fixture() -> (tempfile::TempDir, PathBuf, crate::store::Store, String) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("app.db");
        let work = dir.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        let store = crate::store::Store::open(&db).unwrap();
        let conversation = store.create_conversation().unwrap().id;
        store.save_workspace_path(work.to_str().unwrap()).unwrap();
        (dir, db, store, conversation)
    }

    #[test]
    fn run_check_records_exit_code_artifact_and_excerpt() {
        let (dir, db, store, conversation) = fixture();
        let record = run(run_check(&db, &conversation, "exit 3", None, "command")).unwrap();
        assert_eq!(record.result, "failed");
        assert_eq!(record.exit_code, Some(3));
        assert_eq!(record.environment, "native-local");
        assert!(record.output_artifact.is_some());
        let log = store
            .artifact(record.output_artifact.as_deref().unwrap())
            .unwrap()
            .unwrap();
        assert!(log.content.contains("== stdout =="));
        let ok = run(run_check(
            &db,
            &conversation,
            "echo verified",
            None,
            "command",
        ))
        .unwrap();
        assert_eq!(ok.result, "passed");
        assert!(ok.output_excerpt.contains("verified"));
        assert!(ok.fingerprint.is_some());
        drop(dir);
    }

    #[test]
    fn editing_files_marks_records_stale() {
        let (dir, db, store, conversation) = fixture();
        let record = run(run_check(&db, &conversation, "echo hi", None, "command")).unwrap();
        assert!(record.fingerprint.is_some());
        let summary = summary_impl(&store, &conversation).unwrap();
        assert!(!summary.records[0].stale);
        std::fs::write(dir.path().join("work").join("later.txt"), "changed").unwrap();
        let summary = summary_impl(&store, &conversation).unwrap();
        assert!(summary.records[0].stale);
        assert_eq!(summary.state.state, "needs-review");
    }

    #[test]
    fn records_survive_store_reopen() {
        let (_dir, db, store, conversation) = fixture();
        propose(&store, &conversation, "agent", &["It compiles".to_string()]).unwrap();
        run(run_check(&db, &conversation, "echo ok", None, "command")).unwrap();
        drop(store);
        let reopened = crate::store::Store::open(&db).unwrap();
        let summary = summary_impl(&reopened, &conversation).unwrap();
        assert_eq!(summary.criteria.len(), 1);
        assert_eq!(summary.criteria[0].origin, "agent");
        assert_eq!(summary.records.len(), 1);
        assert_eq!(summary.records[0].result, "passed");
        assert!(!summary.records[0].stale);
    }

    #[test]
    fn run_check_rejects_foreign_criteria() {
        let (dir, db, store, conversation) = fixture();
        let other = crate::store::Store::open(&db).unwrap();
        let foreign = other.create_conversation().unwrap().id;
        let criterion = propose(&other, &foreign, "user", &["Something".to_string()])
            .unwrap()
            .remove(0);
        assert!(run(run_check(
            &db,
            &conversation,
            "echo hi",
            Some(&criterion.id),
            "command"
        ))
        .is_err());
        let _ = store;
        drop(dir);
    }
}
