//! Bundled TypeScript research engine, executed outside the WebView and model context.
use crate::store::PersistedResearch;
use crate::{subagents::BackendSnapshot, AppState};
use serde_json::{json, Value};
use std::{process::Stdio, sync::Arc, time::Duration};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
const WORKER: &str = include_str!("../resources/web/worker.mjs");

/// Channel-only progress relay: the worker writes `\x1e{json}` lines on
/// stderr; this sink turns them into activity events for the chat UI.
pub type ProgressSink = Arc<dyn Fn(String) + Send + Sync>;

/// Vault identifier of the Google Programmable Search credential.
const GOOGLE_KEY_ID: &str = "web-search-google";
/// Settings entry that records only whether a credential is stored. The
/// credential itself lives in the protected vault.
const GOOGLE_KEY_PRESENT: &str = "google_api_key_present";

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WebSearchSettings {
    provider: String,
    searxng_base_url: String,
    google_api_key: String,
    google_cx_id: String,
    search_fallback_enabled: bool,
}

fn validate_settings(
    provider: &str,
    base_urls: &str,
    google_key: &str,
    google_cx: &str,
) -> Result<(), String> {
    if !["searxng", "google"].contains(&provider) {
        return Err("Provider must be searxng or google.".into());
    }
    if [base_urls, google_key, google_cx]
        .iter()
        .any(|value| value.len() > 512)
    {
        return Err("Configuration values exceed 512 bytes.".into());
    }
    for value in base_urls
        .split([',', ';'])
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let url = reqwest::Url::parse(value)
            .map_err(|_| "SearXNG base URL must be an absolute HTTP or HTTPS URL.".to_string())?;
        if !["http", "https"].contains(&url.scheme())
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err("SearXNG base URL must use HTTP or HTTPS without credentials, a query, or a fragment.".into());
        }
    }
    Ok(())
}

fn validate_request(question: &str, mode: &str) -> Result<(), String> {
    if question.trim().is_empty() || question.len() > 8000 {
        return Err("Research needs 1–8000 bytes of question text.".into());
    }
    if !["fast", "normal", "deep"].contains(&mode) {
        return Err("Research mode must be fast, normal, or deep.".into());
    }
    Ok(())
}

/// Read the Google Programmable Search credential from the protected vault.
async fn stored_google_key(state: &AppState) -> Result<String, String> {
    match state.daytona_vault.load_blocking(GOOGLE_KEY_ID).await? {
        Some(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        None => Ok(String::new()),
    }
}
/// Native web_fetch uses Rust HTTP; these aliases specifically need the
/// bundled Node worker, including reads of saved research documents.
pub(crate) fn worker_tool(alias: &str) -> bool {
    matches!(
        alias,
        "weather"
            | "get_weather"
            | "web_search"
            | "web_read"
            | "web_open"
            | "web_find"
            | "web_fetch_url"
            | "search"
            | "visit"
    )
}

pub(crate) async fn worker_runtime_error(
    config: &crate::execution::ExecutionConfig,
) -> Option<String> {
    crate::execution::probe_node(&config.node_path, (22, 13, 0))
        .await
        .err()
}

pub async fn run_worker(
    state: &AppState,
    snapshot: Option<&BackendSnapshot>,
    input: Value,
) -> Result<Value, String> {
    run_worker_with_settings(state, snapshot, input, None, None).await
}

async fn run_worker_with_settings(
    state: &AppState,
    snapshot: Option<&BackendSnapshot>,
    mut input: Value,
    probe: Option<WebSearchSettings>,
    progress: Option<ProgressSink>,
) -> Result<Value, String> {
    let weather = input["action"] == "weather";
    let clearing_cache = input["action"] == "clear-cache";
    if clearing_cache {
        input["weatherCachePath"] = json!(state.data_dir.join("weather-cache.sqlite"));
    }
    if weather {
        let config: crate::weather::WeatherConfig = state.database()?.setting("weather_config")?;
        if !config.enabled {
            return Err("Weather is disabled in Settings.".into());
        }
        if input["temperatureUnit"].is_null() {
            input["temperatureUnit"] = json!(config.temperature_unit);
        }
        if input["location"]
            .as_str()
            .is_none_or(|s| s.trim().is_empty())
        {
            input["location"] = json!(config.location);
        }
        input["weatherCachePath"] = json!(state.data_dir.join("weather-cache.sqlite"));
    }
    let google_key = stored_google_key(state).await?;
    let (execution, searxng, disabled, provider, google_cx, fallback_enabled) = {
        let store = state.database()?;
        (
            store.execution_config()?,
            store.setting::<String>("searxng_base_url")?,
            store.setting::<bool>("web_search_disabled")?,
            store.setting::<String>("web_search_provider")?,
            // The Google credential is a secret: it is read from the
            // protected vault, never from the plain settings table.
            store.setting::<String>("google_cx_id")?,
            store
                .setting::<bool>("search_fallback_enabled")
                .unwrap_or(true),
        )
    };
    let saved_read = matches!(input["action"].as_str(), Some("open" | "find"));
    if disabled && !weather && !clearing_cache && !saved_read && probe.is_none() {
        return Err("Web research is disabled in settings.".into());
    }
    crate::local_only::check_retrieval_action(
        state.database()?.setting::<bool>("offline_mode")?,
        input["action"].as_str().unwrap_or(""),
    )?;
    if let Some(error) = worker_runtime_error(&execution).await {
        return Err(error);
    }
    let worker_path = state.data_dir.join("web-research-worker.mjs");
    if std::fs::read(&worker_path).ok().as_deref() != Some(WORKER.as_bytes()) {
        std::fs::write(&worker_path, WORKER)
            .map_err(|e| format!("Could not install research worker: {e}"))?;
    }
    input["databasePath"] = json!(state.data_dir.join("web-research.sqlite"));
    let provider = if provider.is_empty() {
        "searxng"
    } else {
        &provider
    };
    // Caller-supplied config (e.g. per-call fetch options) merges under the
    // app-managed keys; web-search.json overrides still win last.
    let mut config = input
        .get("config")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    config.insert(
        "searxngBaseUrl".into(),
        json!(if searxng.is_empty() {
            "http://127.0.0.1:8080"
        } else {
            &searxng
        }),
    );
    config.insert("searchProvider".into(), json!(provider));
    config.insert("googleApiKey".into(), json!(google_key));
    config.insert("googleCxId".into(), json!(google_cx));
    config.insert(
        "searchFallback".into(),
        json!({ "enabled": fallback_enabled, "googleDailyLimit": 90 }),
    );
    input["config"] = Value::Object(config);
    let config_path = state.data_dir.join("web-search.json");
    if !weather && config_path.is_file() {
        let bytes = std::fs::read(&config_path).map_err(|e| e.to_string())?;
        if bytes.len() > 64000 {
            return Err("Research configuration exceeds 64 KiB.".into());
        }
        let overrides: Value =
            serde_json::from_slice(&bytes).map_err(|e| format!("Invalid web-search.json: {e}"))?;
        let entries = overrides
            .as_object()
            .ok_or("web-search.json must contain an object")?;
        for (key, value) in entries {
            input["config"][key] = value.clone();
        }
    }
    // Connection checks test the current form without changing saved settings.
    if let Some(settings) = probe {
        input["config"]["searchProvider"] = json!(settings.provider);
        input["config"]["searxngBaseUrl"] = json!(settings.searxng_base_url);
        // An empty field keeps the stored credential; the probe only replaces
        // it when the user typed a new one.
        let key = if settings.google_api_key.is_empty() {
            stored_google_key(state).await?
        } else {
            settings.google_api_key.clone()
        };
        input["config"]["googleApiKey"] = json!(key);
        input["config"]["googleCxId"] = json!(settings.google_cx_id);
        input["config"]["searchFallback"]["enabled"] = json!(settings.search_fallback_enabled);
    }
    if let Some(model) = snapshot.filter(|s| s.kind == "local") {
        let mut base = model.endpoint.trim_end_matches('/').to_string();
        if base.ends_with("/chat/completions") {
            base.truncate(base.len() - "/chat/completions".len());
        }
        if !base.ends_with("/v1") {
            base.push_str("/v1");
        }
        input["localModel"] =
            json!({"baseUrl":base,"modelName":model.model_id,"apiKey":model.key,"timeoutMs":45000});
        let budget = model.context_length.saturating_sub(1400).min(6000);
        if budget < 1000 {
            return Err("Configure at least 2400 context tokens for research.".into());
        }
        input["localModel"]["maxInputTokens"] = json!(budget);
        let configured = input["config"]["context"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        let total = configured
            .get("totalInputBudget")
            .and_then(Value::as_u64)
            .unwrap_or(6000)
            .min(budget as u64);
        let evidence = configured
            .get("evidenceTokenBudget")
            .and_then(Value::as_u64)
            .unwrap_or(2500)
            .min(total.saturating_sub(1000));
        input["config"]["context"] = Value::Object(configured);
        input["config"]["context"]["totalInputBudget"] = json!(total);
        input["config"]["context"]["evidenceTokenBudget"] = json!(evidence);
    }
    let mut command = tokio::process::Command::new(&execution.node_path);
    command
        .arg(&worker_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = crate::execution::managed_spawn(command)
        .map_err(|e| format!("Could not start research worker: {e}"))?;
    let mut stdin = child.stdin().take().ok_or("Research stdin unavailable")?;
    stdin
        .write_all(input.to_string().as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    drop(stdin);
    let mut stdout = child
        .stdout()
        .take()
        .ok_or("Research stdout unavailable")?
        .take(2 * 1024 * 1024);
    let stderr = child.stderr().take().ok_or("Research stderr unavailable")?;
    let mut cancel = state.cancel.subscribe();
    let operation = async {
        let mut out = Vec::new();
        // stderr doubles as the progress channel: lines prefixed with \x1e are
        // JSON progress events relayed live; everything else is error output.
        let drain_stderr = async move {
            let mut lines = BufReader::new(stderr.take(64000)).lines();
            let mut err = String::new();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(raw) = line.strip_prefix('\u{1e}') {
                    if let Some(emit) = &progress {
                        let message = serde_json::from_str::<Value>(raw)
                            .ok()
                            .and_then(|v| {
                                v.get("message").and_then(Value::as_str).map(str::to_string)
                            })
                            .unwrap_or_else(|| raw.chars().take(160).collect());
                        emit(message);
                    }
                } else if err.len() < 60_000 {
                    if !err.is_empty() {
                        err.push('\n');
                    }
                    err.extend(line.chars().take(60_000 - err.len()));
                }
            }
            err
        };
        let (a, err, status) =
            tokio::join!(stdout.read_to_end(&mut out), drain_stderr, child.wait());
        a.map_err(|e| e.to_string())?;
        if !status.map_err(|e| e.to_string())?.success() {
            return Err(format!("Research failed: {err}"));
        }
        serde_json::from_slice(&out).map_err(|e| format!("Invalid research result: {e}"))
    };
    tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(240), operation) => result.map_err(|_| "Research deadline exceeded (240s).".to_string())?,
        _ = cancel.changed() => Err("Research cancelled.".into()),
    }
}
pub async fn research(
    state: &AppState,
    snapshot: Option<&BackendSnapshot>,
    question: &str,
    mode: &str,
    conversation_id: Option<&str>,
    filters: Option<&Value>,
    progress: Option<ProgressSink>,
) -> Result<Value, String> {
    validate_request(question, mode)?;
    let freshness = filters
        .and_then(|f| f.get("freshness"))
        .and_then(Value::as_str)
        .filter(|f| ["day", "week", "month", "year", "realtime"].contains(f));
    let bounded = |key: &str, max: usize| {
        filters
            .and_then(|f| f.get(key))
            .and_then(Value::as_str)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty() && s.len() <= max)
            .map(str::to_string)
    };
    let language = bounded("language", 32);
    let category = bounded("category", 64);
    let mut input = json!({"question":question,"mode":mode});
    if let Some(f) = freshness {
        input["freshness"] = json!(f);
    }
    if let Some(l) = language {
        input["language"] = json!(l);
    }
    if let Some(c) = category {
        input["category"] = json!(c);
    }
    let result = run_worker_with_settings(state, snapshot, input, None, progress).await?;
    state.database()?.save_research_session(
        result["id"].as_str().ok_or("Missing research id")?,
        conversation_id,
        question,
        result["answer"].as_str().unwrap_or(""),
        &result["sources"].to_string(),
        Some(&result["trace"].to_string()),
    )?;
    Ok(result)
}
#[tauri::command]
pub fn get_web_search_config(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let store = state.database()?;
    let provider: String = store.setting("web_search_provider")?;
    let searxng: String = store.setting("searxng_base_url")?;
    let google_cx: String = store.setting("google_cx_id")?;
    let fallback_enabled: bool = store.setting("search_fallback_enabled").unwrap_or(true);
    // The credential itself never leaves the vault. The UI only learns
    // whether one is stored.
    let has_google_api_key = state.daytona_vault.contains(GOOGLE_KEY_ID)?;
    Ok(json!({
        "provider": if provider.is_empty() { "searxng".to_string() } else { provider },
        "searxngBaseUrl": if searxng.is_empty() { "http://127.0.0.1:8080".to_string() } else { searxng },
        "hasGoogleApiKey": has_google_api_key,
        "googleCxId": google_cx,
        "searchFallbackEnabled": fallback_enabled,
        "enabled": !store.setting::<bool>("web_search_disabled")?,
        "mode": default_research_mode(&store)?,
    }))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn save_web_search_config(
    state: tauri::State<'_, AppState>,
    provider: String,
    searxng_base_url: String,
    google_api_key: String,
    google_cx_id: String,
    search_fallback_enabled: Option<bool>,
    mode: Option<String>,
    enabled: Option<bool>,
    clear_google_api_key: Option<bool>,
) -> Result<(), String> {
    validate_settings(&provider, &searxng_base_url, &google_api_key, &google_cx_id)?;
    if let Some(mode) = &mode {
        validate_request("settings", mode)?;
    }
    // The Google credential is written to the protected vault. The settings
    // table keeps only the presence flag.
    if !google_api_key.is_empty() {
        state
            .daytona_vault
            .save_blocking(GOOGLE_KEY_ID, google_api_key.into_bytes())
            .await?;
        state.database()?.save_setting(GOOGLE_KEY_PRESENT, &true)?;
    } else if clear_google_api_key == Some(true) {
        state.daytona_vault.clear_blocking(GOOGLE_KEY_ID).await?;
        state.database()?.save_setting(GOOGLE_KEY_PRESENT, &false)?;
    } else {
        // An empty field keeps the stored credential.
        state.database()?.save_setting(
            GOOGLE_KEY_PRESENT,
            &state.daytona_vault.contains(GOOGLE_KEY_ID)?,
        )?;
    }
    let store = state.database()?;
    store.save_setting("web_search_provider", &provider)?;
    store.save_setting("searxng_base_url", &searxng_base_url)?;
    store.save_setting("google_cx_id", &google_cx_id)?;
    if let Some(mode) = mode {
        store.save_setting("research_mode", &mode)?;
    }
    if let Some(enabled) = enabled {
        store.save_setting("web_search_disabled", &!enabled)?;
    }
    store.save_setting(
        "search_fallback_enabled",
        &search_fallback_enabled.unwrap_or(true),
    )?;
    Ok(())
}
pub fn default_research_mode(store: &crate::store::Store) -> Result<String, String> {
    let mode: String = store.setting("research_mode")?;
    Ok(if mode.is_empty() {
        "normal".into()
    } else {
        mode
    })
}
#[tauri::command]
pub async fn clear_web_cache(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before clearing the cache.")?;
    let result = run_worker(&state, None, json!({"action":"clear-cache"})).await?;
    state
        .web_search_cache
        .lock()
        .map_err(|_| "Cache lock unavailable.")?
        .clear();
    Ok(result)
}

#[tauri::command]
pub async fn web_search(
    state: tauri::State<'_, AppState>,
    question: String,
    mode: Option<String>,
    conversation_id: Option<String>,
) -> Result<Value, String> {
    let snapshot = if let Some(id) = &conversation_id {
        Some(crate::subagents::snapshot_for_conversation(&state, id).await?)
    } else {
        None
    };
    let mode = mode.unwrap_or(default_research_mode(&*state.database()?)?);
    research(
        &state,
        snapshot.as_ref(),
        &question,
        &mode,
        conversation_id.as_deref(),
        None,
        None,
    )
    .await
}
#[tauri::command]
pub async fn web_search_health(
    state: tauri::State<'_, AppState>,
    config: Option<WebSearchSettings>,
) -> Result<Value, String> {
    if let Some(settings) = &config {
        validate_settings(
            &settings.provider,
            &settings.searxng_base_url,
            &settings.google_api_key,
            &settings.google_cx_id,
        )?;
    }
    run_worker_with_settings(&state, None, json!({"action":"health"}), config, None).await
}
#[tauri::command]
pub fn list_research_sessions(
    state: tauri::State<'_, AppState>,
    conversation_id: Option<String>,
    limit: Option<u32>,
) -> Result<Vec<PersistedResearch>, String> {
    state.database()?.list_research_sessions(
        conversation_id.as_deref(),
        limit.unwrap_or(20).min(100) as i64,
    )
}
#[tauri::command]
pub fn get_research_session(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<Option<PersistedResearch>, String> {
    state.database()?.research_session(&id)
}
#[tauri::command]
pub fn delete_research_session(
    state: tauri::State<'_, AppState>,
    id: String,
) -> Result<bool, String> {
    state.database()?.delete_research_session(&id)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn the_google_credential_lives_in_the_vault_and_the_settings_table_keeps_only_a_flag() {
        let directory = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(directory.path());
        let secret = "fixture-google-key";
        state
            .daytona_vault
            .save(GOOGLE_KEY_ID, secret.as_bytes())
            .unwrap();
        state
            .database()
            .unwrap()
            .save_setting(GOOGLE_KEY_PRESENT, &true)
            .unwrap();
        // The secret is never readable from the plain settings table.
        let settings: String = state
            .database()
            .unwrap()
            .setting("google_api_key")
            .unwrap_or_default();
        assert!(
            settings.is_empty(),
            "the credential must not live in the settings table"
        );
        // The sealed file holds no cleartext.
        let sealed = std::fs::read(
            directory
                .path()
                .join("fixture-state/credentials/web-search-google.sealed"),
        )
        .unwrap();
        assert!(
            !sealed
                .windows(secret.len())
                .any(|part| part == secret.as_bytes()),
            "the sealed credential must not contain the secret"
        );
        // The vault serves the credential and reports only its presence.
        assert_eq!(stored_google_key(&state).await.unwrap(), secret);
        assert!(state.daytona_vault.contains(GOOGLE_KEY_ID).unwrap());
    }
    #[tokio::test]
    async fn worker_prerequisites_distinguish_native_and_worker_tools() {
        for alias in [
            "get_weather",
            "weather",
            "web_search",
            "web_read",
            "web_open",
            "web_find",
            "web_fetch_url",
            "search",
            "visit",
        ] {
            assert!(worker_tool(alias), "{alias}");
        }
        for alias in [
            "web_fetch",
            "file_search",
            "artifact_read",
            "update_context",
            "finish",
        ] {
            assert!(!worker_tool(alias), "{alias}");
        }
        let directory = tempfile::tempdir().unwrap();
        let mut config = crate::execution::ExecutionConfig::default();
        assert!(
            worker_runtime_error(&config).await.is_none(),
            "Test environment needs Node for the bundled worker"
        );
        for path in [
            String::new(),
            directory
                .path()
                .join("missing-node")
                .to_string_lossy()
                .into_owned(),
            directory.path().to_string_lossy().into_owned(),
        ] {
            config.node_path = path;
            assert!(worker_runtime_error(&config)
                .await
                .unwrap()
                .contains("Node.js"));
        }
        #[cfg(unix)]
        {
            let path = directory.path().join("not-executable");
            std::fs::write(&path, "not executable").unwrap();
            config.node_path = path.to_string_lossy().into_owned();
            assert!(worker_runtime_error(&config).await.is_some());
        }
    }

    #[tokio::test]
    async fn stashed_fetch_document_is_openable_and_findable() {
        let directory = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(directory.path());
        crate::test_support::select_fixture_model(&state, directory.path());
        // Persist a fetched page into a session without any network, then
        // revisit it exactly the way web_find does.
        let stored = run_worker(
            &state,
            None,
            json!({"action":"store","sessionId":"fetch-conv","document":{
                "id":"doc-1","url":"https://example.com/page","title":"Fixture page",
                "text":"The fixture body mentions needle-term and repeats needle-term.",
                "metadata":{"extractionMethod":"main_content"}
            }}),
        )
        .await
        .unwrap();
        assert_eq!(stored["documentId"], "doc-1");
        let found = run_worker(&state, None, json!({"action":"find","sessionId":"fetch-conv","documentId":"doc-1","term":"needle-term"})).await.unwrap();
        assert_eq!(found["id"], "doc-1");
        let passages = found["passages"].as_array().unwrap();
        assert!(
            passages
                .iter()
                .any(|p| p["text"].as_str().unwrap_or("").contains("needle-term")),
            "{found}"
        );
        let opened = run_worker(
            &state,
            None,
            json!({"action":"open","sessionId":"fetch-conv","documentId":"doc-1"}),
        )
        .await
        .unwrap();
        assert!(
            opened["passages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p["text"].as_str().unwrap_or("").contains("fixture body")),
            "{opened}"
        );
        assert!(run_worker(
            &state,
            None,
            json!({"action":"find","sessionId":"fetch-conv","documentId":"other","term":"x"})
        )
        .await
        .is_err());
    }

    #[tokio::test]
    async fn real_catalog_omits_unavailable_worker_tools_and_preserves_native_and_offline_reads() {
        let directory = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(directory.path());
        crate::test_support::select_fixture_model(&state, directory.path());
        let valid = state.database().unwrap().execution_config().unwrap();
        for path in [
            String::new(),
            directory
                .path()
                .join("missing-node")
                .to_string_lossy()
                .into_owned(),
            directory.path().to_string_lossy().into_owned(),
        ] {
            let mut invalid = valid.clone();
            invalid.node_path = path;
            state
                .database()
                .unwrap()
                .save_execution_config(&invalid)
                .unwrap();
            let plan = crate::chat::assemble_turn(
                &state,
                "",
                "Check catalog",
                vec!["__workspace".into()],
                Some(vec![]),
                Some(crate::presets::RESEARCH),
                None,
            )
            .await
            .unwrap();
            assert!(!plan.tools.iter().any(|tool| worker_tool(&tool.alias)));
            assert!(
                plan.tools.iter().any(|tool| tool.alias == "web_fetch"),
                "Native Rust HTTP fetch does not need Node"
            );
            assert!(plan.tools.iter().any(|tool| tool.alias == "artifact_read"));
            assert!(plan
                .tool_notice
                .as_deref()
                .unwrap_or("")
                .contains("Node.js"));
            let error = run_worker(
                &state,
                None,
                json!({"action":"open","sessionId":"fixture","documentId":"fixture"}),
            )
            .await
            .unwrap_err();
            assert!(error.contains("Node.js"), "{error}");
        }
        state
            .database()
            .unwrap()
            .save_execution_config(&valid)
            .unwrap();
        let online = crate::chat::assemble_turn(
            &state,
            "",
            "Check catalog",
            vec!["__workspace".into()],
            Some(vec![]),
            Some(crate::presets::RESEARCH),
            None,
        )
        .await
        .unwrap();
        assert!(online.tools.iter().any(|tool| tool.alias == "web_search"));
        assert!(online.tools.iter().any(|tool| tool.alias == "web_open"));
        state
            .database()
            .unwrap()
            .save_setting("offline_mode", &true)
            .unwrap();
        let offline = crate::chat::assemble_turn(
            &state,
            "",
            "Check catalog",
            vec!["__workspace".into()],
            Some(vec![]),
            Some(crate::presets::RESEARCH),
            None,
        )
        .await
        .unwrap();
        assert!(!offline
            .tools
            .iter()
            .any(|tool| crate::local_only::external_web_tool(&tool.alias)));
        assert!(offline.tools.iter().any(|tool| tool.alias == "web_open"));
        assert!(offline.tools.iter().any(|tool| tool.alias == "web_find"));
    }

    #[test]
    fn search_settings_validate_instance_lists_and_proxy_paths() {
        for url in [
            "",
            "http://127.0.0.1:8080",
            "https://search.example/searx/; http://localhost:8081/search",
        ] {
            assert!(validate_settings("searxng", url, "", "").is_ok());
        }
        for url in [
            "localhost:8080",
            "file:///tmp/search",
            "https://search.example/?q=test",
            "https://user:secret@search.example",
            "https://search.example/#fragment",
        ] {
            assert!(validate_settings("searxng", url, "", "").is_err(), "{url}");
        }
    }
    #[test]
    fn research_input_is_bounded() {
        assert!(validate_request("", "normal").is_err());
        assert!(validate_request("weather", "unbounded").is_err());
        assert!(validate_request("weather", "fast").is_ok());
    }
    #[test]
    fn research_offered_in_web_presets() {
        for mode in ["standard", "research", "creator", "code"] {
            assert!(crate::presets::get(mode)
                .unwrap()
                .harness
                .contains(&"web_search".to_string()));
        }
        assert!(!crate::presets::get("chat")
            .unwrap()
            .harness
            .contains(&"web_search".to_string()));
        assert!(crate::harness::definition("web_search").is_some());
    }
}
