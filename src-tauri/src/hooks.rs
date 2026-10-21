//! User-configured shell hooks around tool dispatch and run completion.
//! Configured only through the Settings UI / Tauri commands — never exposed
//! as model tools. Each hook receives a JSON event on stdin; `pre_tool_use`
//! may veto a call (exit 2), `post_tool_use` may attach a note to the
//! model-visible result, and `run_finished` is fire-and-forget.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::process::Stdio;
use tokio::io::AsyncWriteExt;

pub const HOOKS_KEY: &str = "hooks_v1";
pub const EVENT_PRE_TOOL: &str = "pre_tool_use";
pub const EVENT_POST_TOOL: &str = "post_tool_use";
pub const EVENT_RUN_FINISHED: &str = "run_finished";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Hook {
    pub id: String,
    pub event: String,
    pub matcher: String,
    pub command: String,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u32,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}
fn default_timeout() -> u32 {
    30
}
fn default_enabled() -> bool {
    true
}

fn validate(hooks: &[Hook]) -> Result<(), String> {
    if hooks.len() > 50 {
        return Err("Keep at most 50 hooks.".into());
    }
    for hook in hooks {
        if hook.id.is_empty() || hook.id.len() > 80 {
            return Err("Each hook needs an id of 1-80 characters.".into());
        }
        if !matches!(
            hook.event.as_str(),
            EVENT_PRE_TOOL | EVENT_POST_TOOL | EVENT_RUN_FINISHED
        ) {
            return Err(format!("Unknown hook event '{}'.", hook.event));
        }
        if hook.matcher.is_empty() || hook.matcher.len() > 200 {
            return Err("Each hook needs a matcher (glob like 'workspace_*' or '*').".into());
        }
        if hook.command.is_empty() || hook.command.len() > 8192 {
            return Err("Each hook needs a command of 1-8192 characters.".into());
        }
        if !(1..=60).contains(&hook.timeout_secs) {
            return Err("Hook timeout must be 1-60 seconds.".into());
        }
    }
    Ok(())
}

pub fn hooks(store: &crate::store::Store) -> Vec<Hook> {
    store.setting(HOOKS_KEY).unwrap_or_default()
}
fn save(store: &crate::store::Store, hooks: &[Hook]) -> Result<(), String> {
    validate(hooks)?;
    store.save_setting(HOOKS_KEY, &hooks.to_vec())
}

/// Simple single-star glob: `*`, `prefix*`, `*suffix`, `*middle*`, or exact.
fn matcher_matches(matcher: &str, alias: &str) -> bool {
    if matcher == "*" {
        return true;
    }
    match (matcher.strip_prefix('*'), matcher.strip_suffix('*')) {
        (Some(middle), _)
            if matcher.starts_with('*') && matcher.ends_with('*') && matcher.len() > 1 =>
        {
            alias.contains(middle.trim_end_matches('*'))
        }
        (_, Some(prefix)) if matcher.ends_with('*') => alias.starts_with(prefix),
        (Some(suffix), _) => alias.ends_with(suffix),
        _ => matcher == alias,
    }
}

fn matching<'a>(
    hooks: &'a [Hook],
    event: &'a str,
    alias: &'a str,
) -> impl Iterator<Item = &'a Hook> {
    hooks.iter().filter(move |hook| {
        hook.enabled && hook.event == event && matcher_matches(&hook.matcher, alias)
    })
}

/// One hook invocation. Returns (exit_code, stdout, stderr) — or a synthetic
/// timeout result so callers can apply their own severity.
async fn run_hook(
    workspace: &str,
    config: &crate::execution::ExecutionConfig,
    command: &str,
    stdin_json: &Value,
    timeout_secs: u32,
) -> Result<(i32, String, String), String> {
    let mut process = tokio::process::Command::new(&config.bash_path);
    let stem = std::path::Path::new(&config.bash_path)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    if stem.contains("powershell") || stem == "pwsh" {
        process.args(["-NoProfile", "-NonInteractive", "-Command", command]);
    } else {
        process.args(["-c", command]);
    }
    process
        .current_dir(workspace)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in [
        "SystemRoot",
        "WINDIR",
        "TEMP",
        "TMP",
        "PATH",
        "PATHEXT",
        "COMSPEC",
        "USERPROFILE",
        "HOME",
        "APPDATA",
        "LOCALAPPDATA",
    ] {
        if let Some(value) = std::env::var_os(name) {
            process.env(name, value);
        }
    }
    let mut child = process
        .spawn()
        .map_err(|error| format!("Hook command failed to start: {error}"))?;
    let payload = stdin_json.to_string();
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(payload.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs as u64),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| format!("Hook timed out after {timeout_secs}s."))?
    .map_err(|error| format!("Hook failed: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    Ok((output.status.code().unwrap_or(-1), stdout, stderr))
}

/// The store + execution config + workspace needed to spawn hook processes.
/// Returns None (hooks silently skip) when the run has no workspace or shell.
fn hook_context(
    state: &crate::AppState,
) -> Result<(Vec<Hook>, crate::execution::ExecutionConfig, String), String> {
    let store = state.database()?;
    let hooks = hooks(&store);
    if hooks.is_empty() {
        return Ok((Vec::new(), store.execution_config()?, String::new()));
    }
    let config = store.execution_config()?;
    if !config.bash.enabled {
        return Ok((Vec::new(), config, String::new()));
    }
    let workspace = store.workspace_path().unwrap_or_default();
    Ok((hooks, config, workspace))
}

pub struct PreUse {
    /// stderr to surface to the model as a hook-blocked failure envelope.
    pub blocked: Option<String>,
    /// Non-fatal hook problems surfaced as a tool notice.
    pub notices: Vec<String>,
}

/// Run all enabled pre_tool_use hooks for a call about to execute. The first
/// exit-2 blocks the call; other failures are notices, never vetoes.
pub async fn pre_use(
    state: &crate::AppState,
    conversation_id: &str,
    alias: &str,
    arguments: &Value,
) -> PreUse {
    let mut verdict = PreUse {
        blocked: None,
        notices: Vec::new(),
    };
    let Ok((hooks, config, workspace)) = hook_context(state) else {
        return verdict;
    };
    if workspace.is_empty() {
        return verdict;
    }
    let stdin = json!({"event": EVENT_PRE_TOOL, "conversationId": conversation_id, "tool": alias, "arguments": arguments});
    for hook in matching(&hooks, EVENT_PRE_TOOL, alias) {
        match run_hook(
            &workspace,
            &config,
            &hook.command,
            &stdin,
            hook.timeout_secs,
        )
        .await
        {
            Ok((2, _, stderr)) => {
                verdict.blocked = Some(if stderr.trim().is_empty() {
                    format!("Hook '{}' blocked the call.", hook.id)
                } else {
                    stderr.trim().chars().take(2048).collect()
                });
                return verdict;
            }
            Ok((code, _, stderr)) => {
                if code != 0 {
                    verdict.notices.push(format!(
                        "Hook '{}' exited {code}: {}",
                        hook.id,
                        stderr.trim().chars().take(300).collect::<String>()
                    ));
                }
            }
            Err(error) => verdict
                .notices
                .push(format!("Hook '{}' failed: {error}", hook.id)),
        }
    }
    verdict
}

/// Run all enabled post_tool_use hooks; returns the combined note (stdout,
/// capped at 4 KiB) appended to the model-visible result.
pub async fn post_use(
    state: &crate::AppState,
    conversation_id: &str,
    alias: &str,
    arguments: &Value,
    result: &Value,
    decision: &str,
) -> Option<String> {
    let Ok((hooks, config, workspace)) = hook_context(state) else {
        return None;
    };
    if workspace.is_empty() {
        return None;
    }
    let stdin = json!({"event": EVENT_POST_TOOL, "conversationId": conversation_id, "tool": alias,
        "arguments": arguments, "result": result, "decision": decision});
    let mut note = String::new();
    for hook in matching(&hooks, EVENT_POST_TOOL, alias) {
        if let Ok((0, stdout, _)) = run_hook(
            &workspace,
            &config,
            &hook.command,
            &stdin,
            hook.timeout_secs,
        )
        .await
        {
            let trimmed = stdout.trim();
            if !trimmed.is_empty() && note.chars().count() < 4096 {
                if !note.is_empty() {
                    note.push('\n');
                }
                note.push_str(
                    &trimmed
                        .chars()
                        .take(4096 - note.chars().count())
                        .collect::<String>(),
                );
            }
        }
    }
    (!note.is_empty()).then_some(note)
}

/// Fire-and-forget run_finished hooks. Call once per run completion.
pub fn run_finished(state: &crate::AppState, conversation_id: String, detail: Value) {
    let Ok((hooks, config, workspace)) = hook_context(state) else {
        return;
    };
    if workspace.is_empty() {
        return;
    }
    let stdin =
        json!({"event": EVENT_RUN_FINISHED, "conversationId": conversation_id, "result": detail});
    for hook in hooks
        .iter()
        .filter(|hook| hook.enabled && hook.event == EVENT_RUN_FINISHED)
        .cloned()
        .collect::<Vec<_>>()
    {
        let (config, workspace, stdin) = (config.clone(), workspace.clone(), stdin.clone());
        tokio::spawn(async move {
            let _ = run_hook(
                &workspace,
                &config,
                &hook.command,
                &stdin,
                hook.timeout_secs,
            )
            .await;
        });
    }
}

#[tauri::command]
pub fn get_hooks(state: tauri::State<'_, crate::AppState>) -> Result<Vec<Hook>, String> {
    let store = state.database()?;
    Ok(hooks(&store))
}

#[tauri::command]
pub fn save_hooks(
    state: tauri::State<'_, crate::AppState>,
    hooks: Vec<Hook>,
) -> Result<(), String> {
    let store = state.database()?;
    save(&store, &hooks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, crate::AppState) {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        crate::test_support::select_fixture_model(&state, dir.path());
        state
            .database()
            .unwrap()
            .save_workspace_path(dir.path().to_str().unwrap())
            .unwrap();
        (dir, state)
    }

    #[tokio::test]
    // The database guard is explicitly dropped before the await below;
    // `await_holding_lock` does not model an explicit `drop`.
    #[allow(clippy::await_holding_lock)]
    async fn pre_hook_exit_2_blocks_with_stderr_and_matcher_globs() {
        let (_dir, state) = fixture();
        let store = state.database().unwrap();
        save(
            &store,
            &[
                Hook {
                    id: "guard".into(),
                    event: EVENT_PRE_TOOL.into(),
                    matcher: "workspace_*".into(),
                    command: "cat >/dev/null; echo no >&2; exit 2".into(),
                    timeout_secs: 10,
                    enabled: true,
                },
                Hook {
                    id: "other".into(),
                    event: EVENT_PRE_TOOL.into(),
                    matcher: "check_run".into(),
                    command: "exit 2".into(),
                    timeout_secs: 10,
                    enabled: true,
                },
            ],
        )
        .unwrap();
        drop(store);
        let verdict = pre_use(&state, "conv", "workspace_edit_file", &json!({})).await;
        assert_eq!(
            verdict.blocked.as_deref(),
            Some("no"),
            "{:?}",
            verdict.notices
        );
        // Non-matching aliases are untouched.
        let verdict = pre_use(&state, "conv", "check_report", &json!({})).await;
        assert!(verdict.blocked.is_none());
    }

    #[tokio::test]
    async fn post_hook_stdout_becomes_a_bounded_note() {
        let (_dir, state) = fixture();
        state
            .database()
            .unwrap()
            .save_setting(
                HOOKS_KEY,
                &vec![Hook {
                    id: "note".into(),
                    event: EVENT_POST_TOOL.into(),
                    matcher: "*".into(),
                    command: "echo reviewed".into(),
                    timeout_secs: 10,
                    enabled: true,
                }],
            )
            .unwrap();
        let note = post_use(
            &state,
            "conv",
            "check_run",
            &json!({}),
            &json!({"ok": true}),
            "approved",
        )
        .await;
        assert_eq!(note.as_deref(), Some("reviewed"));
    }

    #[tokio::test]
    async fn pre_hook_nonzero_and_timeout_are_notices_not_blocks() {
        let (_dir, state) = fixture();
        state
            .database()
            .unwrap()
            .save_setting(
                HOOKS_KEY,
                &vec![
                    Hook {
                        id: "warn".into(),
                        event: EVENT_PRE_TOOL.into(),
                        matcher: "*".into(),
                        command: "exit 1".into(),
                        timeout_secs: 10,
                        enabled: true,
                    },
                    Hook {
                        id: "slow".into(),
                        event: EVENT_PRE_TOOL.into(),
                        matcher: "*".into(),
                        command: "sleep 5".into(),
                        timeout_secs: 1,
                        enabled: true,
                    },
                ],
            )
            .unwrap();
        let verdict = pre_use(&state, "conv", "check_run", &json!({})).await;
        assert!(verdict.blocked.is_none());
        assert_eq!(verdict.notices.len(), 2, "{:?}", verdict.notices);
    }

    #[test]
    fn hooks_validate_and_match() {
        assert!(matcher_matches("workspace_*", "workspace_edit_file"));
        assert!(matcher_matches("*", "anything"));
        assert!(matcher_matches("*_rename", "lsp_rename"));
        assert!(matcher_matches("check_run", "check_run"));
        assert!(!matcher_matches("workspace_*", "check_run"));
        assert!(!matcher_matches("*_rename", "check_run"));
        let mut hook = Hook {
            id: "h".into(),
            event: "bogus".into(),
            matcher: "*".into(),
            command: "true".into(),
            timeout_secs: 30,
            enabled: true,
        };
        assert!(validate(&[hook.clone()]).is_err());
        hook.event = EVENT_PRE_TOOL.into();
        hook.timeout_secs = 0;
        assert!(validate(&[hook.clone()]).is_err());
        hook.timeout_secs = 30;
        hook.command.clear();
        assert!(validate(&[hook]).is_err());
    }
}
