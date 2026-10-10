//! Bundled TypeScript research engine, executed outside the WebView and model context.
use crate::store::PersistedResearch;
use process_wrap::tokio::{CommandWrap, KillOnDrop};
use sha2::{Digest, Sha256};
use serde_json::{json, Value};
use std::{path::{Path, PathBuf}, process::Stdio, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use crate::{AppState, subagents::BackendSnapshot};
const WORKER: &str = include_str!("../resources/web/worker.mjs");
const GOOGLE_CREDENTIAL: &str = "web-search-google";

fn google_api_key(store: &crate::store::Store, vault: &crate::vault::Vault) -> Result<String, String> {
    let legacy: String = store.setting("google_api_key")?;
    if !legacy.is_empty() {
        if vault.load(GOOGLE_CREDENTIAL)?.is_none() { vault.save(GOOGLE_CREDENTIAL, legacy.as_bytes())?; }
        store.save_setting("google_api_key", &String::new())?;
    }
    String::from_utf8(vault.load(GOOGLE_CREDENTIAL)?.unwrap_or_default()).map_err(|_| "Saved Google credential is invalid.".into())
}

fn install_worker(data_dir: &Path) -> Result<PathBuf, String> {
    let hash = format!("{:x}", Sha256::digest(WORKER.as_bytes()));
    let path = data_dir.join(format!("web-research-worker-{}.mjs", &hash[..16]));
    if std::fs::read(&path).ok().as_deref() == Some(WORKER.as_bytes()) {
        return Ok(path);
    }
    let temporary = data_dir.join(format!("web-research-worker-{}.tmp", uuid::Uuid::new_v4()));
    std::fs::write(&temporary, WORKER).map_err(|e| format!("Could not install research worker: {e}"))?;
    if let Err(error) = std::fs::rename(&temporary, &path) {
        let _ = std::fs::remove_file(&temporary);
        // Another concurrent call may have published the same complete bundle.
        if std::fs::read(&path).ok().as_deref() != Some(WORKER.as_bytes()) {
            return Err(format!("Could not install research worker: {error}"));
        }
    }
    Ok(path)
}

fn worker_deadline_seconds(action: Option<&str>) -> u64 {
    match action {
        Some("health") => 15,
        Some("open" | "find") => 30,
        Some("weather" | "verify") => 45,
        Some("search" | "fetch-url") => 90,
        // visit batches pay for up to four extraction sub-calls on top of fetch.
        Some("fetch-urls") => 150,
        _ => 240,
    }
}

fn worker_failure_message(stderr: &str) -> String {
    stderr.lines()
        .map(str::trim)
        .find(|line| !line.is_empty()
            && !line.starts_with("(node:")
            && !line.starts_with("ExperimentalWarning:")
            && !line.starts_with("at ")
            && !line.starts_with("Use `node --trace-warnings`"))
        .unwrap_or("Research worker exited without a diagnostic.")
        .chars().take(500).collect()
}

async fn read_bounded<R: tokio::io::AsyncRead + Unpin>(reader: R, limit: usize, stream: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut bytes).await.map_err(|e| e.to_string())?;
    if bytes.len() > limit {
        return Err(format!("Research worker {stream} exceeded {limit} bytes."));
    }
    Ok(bytes)
}

fn validate_request(question: &str, mode: &str) -> Result<(), String> {
    if question.trim().is_empty() || question.len() > 8000 { return Err("Research needs 1–8000 bytes of question text.".into()); }
    if !["fast", "normal", "deep"].contains(&mode) { return Err("Research mode must be fast, normal, or deep.".into()); }
    Ok(())
}
pub async fn run_worker(state: &AppState, snapshot: Option<&BackendSnapshot>, mut input: Value) -> Result<Value, String> {
    if !crate::local_only::retrieval_allowed(state.database()?.setting::<bool>("offline_mode")?)
        && network_action(input.get("action").and_then(Value::as_str)) {
        return Err("Offline mode: external retrieval is disabled.".into());
    }
    let deadline_secs = worker_deadline_seconds(input.get("action").and_then(Value::as_str));
    input["deadlineMs"] = json!(crate::store::now() + (deadline_secs.saturating_sub(15) * 1000) as i64);
    let (execution, searxng, disabled, provider, google_key, google_cx, fallback_enabled) = {
        let store = state.database()?;
        (store.execution_config()?, store.setting::<String>("searxng_base_url")?, store.setting::<bool>("web_search_disabled")?,
         store.setting::<String>("web_search_provider")?, google_api_key(&store, &state.daytona_vault)?, store.setting::<String>("google_cx_id")?,
         store.setting::<Option<bool>>("search_fallback_enabled")?.unwrap_or(true))
    };
    if disabled { return Err("Web research is disabled in settings.".into()); }
    if execution.node_path.is_empty() { return Err("Configure Node.js 22.13+ in Execution settings to use web research.".into()); }
    let worker_path = install_worker(&state.data_dir)?;
    input["databasePath"] = json!(state.data_dir.join("web-research.sqlite"));
    let provider = if provider.is_empty() { "searxng" } else { &provider };
    // Caller-supplied config (e.g. per-call fetch options) merges under the
    // app-managed keys; web-search.json overrides still win last.
    let mut config = input.get("config").and_then(Value::as_object).cloned().unwrap_or_default();
    config.insert("searxngBaseUrl".into(), json!(if searxng.is_empty() { "http://127.0.0.1:8080" } else { &searxng }));
    config.insert("searchProvider".into(), json!(provider));
    config.insert("googleApiKey".into(), json!(google_key));
    config.insert("googleCxId".into(), json!(google_cx));
    config.insert("searchFallback".into(), json!({ "enabled": fallback_enabled, "googleDailyLimit": 90 }));
    input["config"] = Value::Object(config);
    let config_path = state.data_dir.join("web-search.json");
    if config_path.is_file() {
        let bytes = std::fs::read(&config_path).map_err(|e| e.to_string())?;
        if bytes.len() > 64000 { return Err("Research configuration exceeds 64 KiB.".into()); }
        let overrides: Value = serde_json::from_slice(&bytes).map_err(|e| format!("Invalid web-search.json: {e}"))?;
        let entries = overrides.as_object().ok_or("web-search.json must contain an object")?;
        for (key, value) in entries { input["config"][key] = value.clone(); }
    }
    if let Some(model) = snapshot.filter(|s| s.kind == "local" && s.api_format == crate::providers::OPENAI_CHAT_COMPLETIONS) {
        let endpoint = if model.base_url.is_empty() { &model.endpoint } else { &model.base_url };
        crate::local_only::inference_allowed(endpoint)?;
        let mut base = endpoint.trim_end_matches('/').to_string();
        if base.ends_with("/chat/completions") { base.truncate(base.len() - "/chat/completions".len()); }
        if !base.ends_with("/v1") { base.push_str("/v1"); }
        input["localModel"] = json!({"baseUrl":base,"modelName":model.model_id,"apiKey":model.key,"timeoutMs":45000});
        if model.base_url.is_empty() && model.inference_slots == 2 { input["localModel"]["idSlot"] = json!(1); }
        let budget = model.context_length.saturating_sub(1400).min(6000);
        if budget < 1000 { return Err("Configure at least 2400 context tokens for research.".into()); }
        input["localModel"]["maxInputTokens"] = json!(budget);
        let configured = input["config"]["context"].as_object().cloned().unwrap_or_default();
        let total = configured.get("totalInputBudget").and_then(Value::as_u64).unwrap_or(6000).min(budget as u64);
        let evidence = configured.get("evidenceTokenBudget").and_then(Value::as_u64).unwrap_or(2500).min(total.saturating_sub(1000));
        input["config"]["context"] = Value::Object(configured);
        input["config"]["context"]["totalInputBudget"] = json!(total);
        input["config"]["context"]["evidenceTokenBudget"] = json!(evidence);
    }
    let mut command = tokio::process::Command::new(&execution.node_path);
    command.arg(&worker_path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    // The extracted worker has no node_modules beside it; dev checkouts can
    // still satisfy lazy deps (playwright for js_render) via the repo root.
    let dev_modules = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../node_modules");
    if dev_modules.is_dir() {
        command.env("LOCALLM_NODE_MODULES", dev_modules);
    }
    let mut command = CommandWrap::from(command);
    command.wrap(KillOnDrop);
    #[cfg(windows)]
    {
        command.wrap(process_wrap::tokio::CreationFlags(
            windows::Win32::System::Threading::CREATE_NO_WINDOW,
        ));
        command.wrap(process_wrap::tokio::JobObject);
    }
    #[cfg(unix)]
    command.wrap(process_wrap::tokio::ProcessGroup::leader());
    let mut child = command.spawn().map_err(|e| format!("Could not start research worker: {e}"))?;
    let mut stdin = child.stdin().take().ok_or("Research stdin unavailable")?;
    let request = input.to_string();
    let stdout = child.stdout().take().ok_or("Research stdout unavailable")?;
    let stderr = child.stderr().take().ok_or("Research stderr unavailable")?;
    let mut cancel = state.cancel.subscribe();
    let operation = async {
        // try_join returns as soon as either stream exceeds its cap. Waiting
        // for process exit first can deadlock on a full undrained pipe.
        let (_, out, err, status) = tokio::try_join!(
            async {
                stdin.write_all(request.as_bytes()).await.map_err(|e| e.to_string())?;
                drop(stdin);
                Ok::<(), String>(())
            },
            read_bounded(stdout, 2 * 1024 * 1024, "stdout"),
            read_bounded(stderr, 64_000, "stderr"),
            async { child.wait().await.map_err(|e| e.to_string()) },
        )?;
        if !status.success() {
            let raw = String::from_utf8_lossy(&err);
            let message = worker_failure_message(&raw);
            // Boot failures (missing module, syntax error) are permanent for
            // this process — say so plainly so the model does not burn its
            // research budget retrying an identical call.
            let boot_failure = ["ERR_MODULE_NOT_FOUND", "Cannot find package", "Cannot find module", "SyntaxError"]
                .iter().any(|needle| raw.contains(needle));
            return Err(if boot_failure {
                format!("Research worker failed to start (permanent — retrying will not help): {message}")
            } else {
                format!("Research failed: {message}")
            });
        }
        serde_json::from_slice(&out).map_err(|e| format!("Invalid research result: {e}"))
    };
    tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(deadline_secs), operation) => result.map_err(|_| format!("Research worker timed out after {deadline_secs}s."))?,
        _ = cancel.changed() => Err("Research cancelled.".into()),
    }
}
pub async fn research(state: &AppState, snapshot: Option<&BackendSnapshot>, question: &str, mode: &str, conversation_id: Option<&str>) -> Result<Value, String> {
    validate_request(question, mode)?;
    let context = conversation_id.map(|id| recent_context(state, id)).transpose()?.unwrap_or_default();
    let objective = if let Some(id) = conversation_id {
        state.database()?.latest_user_content(id)?
    } else { None };
    let result = run_worker(state, snapshot, json!({"question":question,"mode":mode,"context":context,"objective":objective})).await?;
    state.database()?.save_research_session(result["id"].as_str().ok_or("Missing research id")?, conversation_id, question,
        result["answer"].as_str().unwrap_or(""), &result["sources"].to_string(), Some(&result["trace"].to_string()))?;
    Ok(result)
}
fn network_action(action: Option<&str>) -> bool {
    !matches!(action, Some("open" | "find" | "verify"))
}

/// `searxngWslDistro` in web-search.json names the WSL distribution whose
/// `locallm-searxng` systemd unit serves search. When set, the app holds the
/// distro open for its own lifetime instead of relying on an external keepalive.
#[cfg(windows)]
fn searxng_wsl_distro(state: &AppState) -> Option<String> {
    let bytes = std::fs::read(state.data_dir.join("web-search.json")).ok()?;
    if bytes.len() > 64000 {
        return None;
    }
    let overrides: Value = serde_json::from_slice(&bytes).ok()?;
    let distro = overrides.get("searxngWslDistro")?.as_str()?.trim();
    (distro.len() <= 64
        && distro.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')))
    .then(|| distro.to_string())
}

#[cfg(windows)]
pub fn start_searxng_wsl(state: &AppState) {
    let Some(distro) = searxng_wsl_distro(state) else { return };
    let managed = state
        .database()
        .ok()
        .and_then(|store| {
            let provider: String = store.setting("web_search_provider").ok()?;
            let disabled: bool = store.setting("web_search_disabled").ok()?;
            let url: String = store.setting("searxng_base_url").ok()?;
            Some(
                !disabled
                    && (provider.is_empty() || provider == "searxng")
                    && crate::providers::is_loopback_base_url(if url.is_empty() {
                        "http://127.0.0.1:8080"
                    } else {
                        &url
                    }),
            )
        })
        .unwrap_or(false);
    if !managed {
        return;
    }
    let mut command = tokio::process::Command::new("wsl");
    command
        .args(["-d", &distro, "--exec", "sleep", "infinity"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut command = CommandWrap::from(command);
    command.wrap(KillOnDrop);
    command.wrap(process_wrap::tokio::CreationFlags(
        windows::Win32::System::Threading::CREATE_NO_WINDOW,
    ));
    command.wrap(process_wrap::tokio::JobObject);
    match command.spawn() {
        Ok(child) => {
            if let Ok(mut hold) = state.searxng_wsl.lock() {
                *hold = Some((distro, child));
            }
        }
        Err(error) => eprintln!("Could not start the SearXNG WSL keepalive: {error}"),
    }
}

/// Stop the unit while the VM is still up so SearXNG is down even before the
/// distro idles out, then release the hold that keeps the VM alive.
#[cfg(windows)]
pub fn stop_searxng_wsl(state: &AppState) {
    use std::os::windows::process::CommandExt;
    let Ok(mut hold) = state.searxng_wsl.lock() else { return };
    let Some((distro, mut child)) = hold.take() else { return };
    let _ = std::process::Command::new("wsl")
        .args(["-d", &distro, "-u", "root", "--exec", "systemctl", "stop", "locallm-searxng"])
        .creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0)
        .status();
    let _ = child.start_kill();
}

#[cfg(not(windows))]
pub fn start_searxng_wsl(_: &AppState) {}
#[cfg(not(windows))]
pub fn stop_searxng_wsl(_: &AppState) {}

pub(crate) fn recent_context(state: &AppState, conversation_id: &str) -> Result<String, String> {
    state.database()?.recent_chat_context(conversation_id)
}
#[tauri::command]
pub fn get_web_search_config(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    web_search_config(&state)
}
fn web_search_config(state: &AppState) -> Result<Value, String> {
    let store = state.database()?;
    let provider: String = store.setting("web_search_provider")?;
    let searxng: String = store.setting("searxng_base_url")?;
    let google_key = google_api_key(&store, &state.daytona_vault)?;
    let google_cx: String = store.setting("google_cx_id")?;
    let fallback_enabled = store.setting::<Option<bool>>("search_fallback_enabled")?.unwrap_or(true);
    Ok(json!({
        "provider": if provider.is_empty() { "searxng".to_string() } else { provider },
        "searxngBaseUrl": if searxng.is_empty() { "http://127.0.0.1:8080".to_string() } else { searxng },
        "hasGoogleApiKey": !google_key.is_empty(),
        "googleCxId": google_cx,
        "searchFallbackEnabled": fallback_enabled,
    }))
}

#[tauri::command]
pub fn save_web_search_config(state: tauri::State<'_, AppState>, provider: String, searxng_base_url: String, google_api_key: String, google_cx_id: String, search_fallback_enabled: Option<bool>, clear_google_api_key: Option<bool>) -> Result<(), String> {
    if !["searxng", "google"].contains(&provider.as_str()) {
        return Err("Provider must be searxng or google.".into());
    }
    if searxng_base_url.len() > 512 || google_api_key.len() > 512 || google_cx_id.len() > 512 {
        return Err("Configuration values exceed 512 characters.".into());
    }
    let store = state.database()?;
    store.save_setting("web_search_provider", &provider)?;
    store.save_setting("searxng_base_url", &searxng_base_url)?;
    if clear_google_api_key.unwrap_or(false) { state.daytona_vault.clear(GOOGLE_CREDENTIAL)?; }
    else if !google_api_key.is_empty() { state.daytona_vault.save(GOOGLE_CREDENTIAL, google_api_key.as_bytes())?; }
    else { self::google_api_key(&store, &state.daytona_vault)?; }
    store.save_setting("google_api_key", &String::new())?;
    store.save_setting("google_cx_id", &google_cx_id)?;
    store.save_setting("search_fallback_enabled", &search_fallback_enabled.unwrap_or(true))?;
    Ok(())
}

#[tauri::command]
pub async fn web_search(state: tauri::State<'_, AppState>, question: String, mode: Option<String>, conversation_id: Option<String>) -> Result<Value, String> {
    let snapshot = if let Some(id) = &conversation_id { Some(crate::subagents::snapshot_for_conversation(&state, id).await?) } else { None };
    research(&state, snapshot.as_ref(), &question, mode.as_deref().unwrap_or("normal"), conversation_id.as_deref()).await
}
#[tauri::command]
pub async fn web_search_health(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    run_worker(&state, None, json!({"action":"health"})).await
}
#[tauri::command]
pub fn list_research_sessions(state: tauri::State<'_, AppState>, conversation_id: Option<String>, limit: Option<u32>) -> Result<Vec<PersistedResearch>, String> {
    state.database()?.list_research_sessions(conversation_id.as_deref(), limit.unwrap_or(20).min(100) as i64)
}
#[tauri::command]
pub fn get_research_session(state: tauri::State<'_, AppState>, id: String) -> Result<Option<PersistedResearch>, String> { state.database()?.research_session(&id) }
#[tauri::command]
pub fn delete_research_session(state: tauri::State<'_, AppState>, id: String) -> Result<bool, String> { state.database()?.delete_research_session(&id) }
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    #[test]
    fn search_fallback_defaults_true_and_respects_saved_values() {
        let directory = tempfile::tempdir().unwrap(); let state = test_state(directory.path());
        assert_eq!(web_search_config(&state).unwrap()["searchFallbackEnabled"], true);
        for saved in [false, true] {
            state.database().unwrap().save_setting("search_fallback_enabled", &saved).unwrap();
            assert_eq!(web_search_config(&state).unwrap()["searchFallbackEnabled"], saved);
        }
    }
    #[test]
    fn google_key_migrates_to_vault_and_is_never_returned_to_ui() {
        let directory = tempfile::tempdir().unwrap(); let state = test_state(directory.path());
        state.database().unwrap().save_setting("google_api_key", &"legacy-test-secret").unwrap();
        let config = web_search_config(&state).unwrap();
        assert!(config.get("googleApiKey").is_none());
        assert_eq!(config["hasGoogleApiKey"], true);
        assert_eq!(state.daytona_vault.load("web-search-google").unwrap().unwrap(), b"legacy-test-secret");
        assert_eq!(state.database().unwrap().setting::<String>("google_api_key").unwrap(), "");
    }
    pub(crate) fn test_state(directory: &Path) -> AppState {
        let vault = std::sync::Arc::new(crate::vault::Vault::new(directory.join("vault")));
        let runtime = crate::runtime::Runtime::new(directory.join("runtime.log"));
        let runtime_snapshot = runtime.published();
        AppState {
            runtime_installer: Default::default(), model_installer: Default::default(),
            daytona_vault: vault.clone(), daytona_operation: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            daytona_journal: std::sync::Arc::new(std::sync::Mutex::new(crate::daytona_journal::Journal::open(&directory.join("journal.sqlite")).unwrap())),
            skills: tokio::sync::Mutex::new(crate::skills::Skills::new(directory.join("skills"))),
            approvals: Default::default(), ask_user: Default::default(),
            store: std::sync::Mutex::new(crate::store::Store::open(&directory.join("app.sqlite")).unwrap()),
            runtime: tokio::sync::Mutex::new(runtime),
            runtime_snapshot,
            operation: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            resources: Default::default(), cancel: tokio::sync::watch::channel(false).0,
            run_control: Default::default(), connectors: tokio::sync::Mutex::new(crate::connectors::McpHub::new(vault)),
            oauth_operation: tokio::sync::Mutex::new(()), oauth_cancel: tokio::sync::watch::channel(false).0,
            subagents: crate::subagents::global_registry(), terminals: tokio::sync::Mutex::new(crate::sandbox::TerminalRegistry::new()),
            pty_sessions: Default::default(), web_search_cache: Default::default(),
            searxng_wsl: Default::default(),
            qualifying: Default::default(),
            db_path: directory.join("app.sqlite"), data_dir: directory.to_path_buf(),
        }
    }
    #[tokio::test]
    async fn offline_blocks_network_before_worker_install_but_allows_stored_actions() {
        let directory = tempfile::tempdir().unwrap();
        let state = test_state(directory.path());
        state.database().unwrap().save_setting("offline_mode", &true).unwrap();
        for action in [Some("search"), Some("fetch-url"), Some("fetch-urls"), Some("weather"), Some("health"), None] {
            let input = action.map(|action| json!({"action": action})).unwrap_or(json!({"question":"test"}));
            assert_eq!(run_worker(&state, None, input).await.unwrap_err(), "Offline mode: external retrieval is disabled.");
        }
        assert!(!std::fs::read_dir(directory.path()).unwrap().any(|entry| entry.unwrap().file_name().to_string_lossy().starts_with("web-research-worker")));
        for action in ["open", "find", "verify"] {
            assert_ne!(run_worker(&state, None, json!({"action":action})).await.err().as_deref(), Some("Offline mode: external retrieval is disabled."));
        }
    }
    #[test]
    fn research_input_is_bounded() {
        assert!(validate_request("", "normal").is_err());
        assert!(validate_request("weather", "unbounded").is_err());
        assert!(validate_request("weather", "fast").is_ok());
    }
    #[tokio::test]
    async fn keyless_loopback_snapshot_succeeds_but_remote_and_keyless_claude_fail() {
        let directory = tempfile::tempdir().unwrap();
        let state = test_state(directory.path());
        let conversation = state.database().unwrap().create_conversation().unwrap();
        for (base, format, allowed) in [("http://127.0.0.1:1919/v1", crate::providers::OPENAI_CHAT_COMPLETIONS, true),
            ("https://remote.example/v1", crate::providers::OPENAI_CHAT_COMPLETIONS, false),
            ("http://localhost:1919", crate::providers::CLAUDE_MESSAGES, false)] {
            let provider = crate::providers::ProviderConnection {
                id: "fixture".into(), name: "fixture".into(), base_url: base.into(), api_format: format.into(),
                verified: true, last_tested_at: None, has_api_key: false,
                models: vec![crate::providers::RemoteModel { id: "test".into(), context_length: Some(4096), max_output_tokens: None,
                    supports_images: false, tool_support: crate::providers::ToolSupport::Supported }],
            };
            state.database().unwrap().save_provider(&provider).unwrap();
            state.database().unwrap().save_conversation_model(&conversation.id, &crate::providers::ModelSelection {
                provider_id: Some(provider.id), model_id: "test".into(),
            }).unwrap();
            let snapshot = crate::subagents::snapshot_for_conversation(&state, &conversation.id).await;
            assert_eq!(snapshot.is_ok(), allowed, "{base} {format}");
            if allowed {
                let mut snapshot = snapshot.unwrap();
                assert_eq!(snapshot.kind, "local");
                assert!(crate::subagents::local_tools(&snapshot).is_empty(), "Children must not acquire unoffered tools");
                snapshot.offered_tool_aliases.push(crate::connectors::AgentTool::system_time().alias);
                assert_eq!(crate::subagents::local_tools(&snapshot).len(), 1);
            }
        }
    }
    #[test]
    fn worker_error_discards_node_warnings_and_stack_noise() {
        let stderr = "(node:42) ExperimentalWarning: SQLite is experimental\nUse `node --trace-warnings` to show where the warning was created\nFetch failed: HTTP 404\n    at fetchUrl (worker.mjs:12)";
        assert_eq!(worker_failure_message(stderr), "Fetch failed: HTTP 404");
    }
    #[tokio::test]
    async fn oversized_worker_stream_fails_before_a_blocked_producer_finishes() {
        use tokio::io::AsyncWriteExt;
        let (mut writer, reader) = tokio::io::duplex(8);
        let producer = tokio::spawn(async move { writer.write_all(&vec![b'x'; 512]).await });
        let result = tokio::time::timeout(Duration::from_secs(2), read_bounded(reader, 32, "stdout"))
            .await.expect("bounded read must not wait for producer exit");
        assert!(result.unwrap_err().contains("stdout exceeded 32 bytes"));
        producer.abort();
    }
    #[test]
    fn short_worker_actions_have_shorter_deadlines_than_full_research() {
        assert_eq!(worker_deadline_seconds(Some("health")), 15);
        assert_eq!(worker_deadline_seconds(Some("open")), 30);
        assert_eq!(worker_deadline_seconds(Some("fetch-url")), 90);
        assert_eq!(worker_deadline_seconds(None), 240);
    }
    #[test]
    fn concurrent_worker_installations_publish_one_complete_bundle() {
        let directory = tempfile::tempdir().unwrap();
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8).map(|_| scope.spawn(|| install_worker(directory.path()).unwrap())).collect();
            let paths: Vec<_> = handles.into_iter().map(|handle| handle.join().unwrap()).collect();
            assert!(paths.iter().all(|path| path == &paths[0]));
            assert_eq!(std::fs::read(&paths[0]).unwrap(), WORKER.as_bytes());
        });
    }
    #[test]
    fn research_offered_in_web_presets() {
        for mode in ["standard", "research", "creator", "code"] { assert!(crate::presets::get(mode).unwrap().harness.contains(&"web_search".to_string())); }
        assert!(!crate::presets::get("chat").unwrap().harness.contains(&"web_search".to_string()));
        assert!(crate::harness::definition("web_search").is_some());
    }
}
