//! Bundled TypeScript research engine, executed outside the WebView and model context.
use crate::store::PersistedResearch;
use serde_json::{json, Value};
use std::{process::Stdio, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use crate::{AppState, subagents::BackendSnapshot};
const WORKER: &str = include_str!("../resources/web/worker.mjs");

fn validate_request(question: &str, mode: &str) -> Result<(), String> {
    if question.trim().is_empty() || question.len() > 8000 { return Err("Research needs 1–8000 bytes of question text.".into()); }
    if !["fast", "normal", "deep"].contains(&mode) { return Err("Research mode must be fast, normal, or deep.".into()); }
    Ok(())
}
pub async fn run_worker(state: &AppState, snapshot: Option<&BackendSnapshot>, mut input: Value) -> Result<Value, String> {
    let (execution, searxng, disabled, provider, google_key, google_cx, fallback_enabled) = {
        let store = state.database()?;
        (store.execution_config()?, store.setting::<String>("searxng_base_url")?, store.setting::<bool>("web_search_disabled")?,
         store.setting::<String>("web_search_provider")?, store.setting::<String>("google_api_key")?, store.setting::<String>("google_cx_id")?,
         store.setting::<bool>("search_fallback_enabled").unwrap_or(true))
    };
    if disabled { return Err("Web research is disabled in settings.".into()); }
    if execution.node_path.is_empty() { return Err("Configure Node.js 22.13+ in Execution settings to use web research.".into()); }
    let worker_path = state.data_dir.join("web-research-worker.mjs");
    if std::fs::read(&worker_path).ok().as_deref() != Some(WORKER.as_bytes()) {
        std::fs::write(&worker_path, WORKER).map_err(|e| format!("Could not install research worker: {e}"))?;
    }
    input["databasePath"] = json!(state.data_dir.join("web-research.sqlite"));
    let provider = if provider.is_empty() { "searxng" } else { &provider };
    input["config"] = json!({
        "searxngBaseUrl": if searxng.is_empty() { "http://127.0.0.1:8080" } else { &searxng },
        "searchProvider": provider,
        "googleApiKey": google_key,
        "googleCxId": google_cx,
        "searchFallback": { "enabled": fallback_enabled, "googleDailyLimit": 90 },
    });
    let config_path = state.data_dir.join("web-search.json");
    if config_path.is_file() {
        let bytes = std::fs::read(&config_path).map_err(|e| e.to_string())?;
        if bytes.len() > 64000 { return Err("Research configuration exceeds 64 KiB.".into()); }
        let overrides: Value = serde_json::from_slice(&bytes).map_err(|e| format!("Invalid web-search.json: {e}"))?;
        let entries = overrides.as_object().ok_or("web-search.json must contain an object")?;
        for (key, value) in entries { input["config"][key] = value.clone(); }
    }
    if let Some(model) = snapshot.filter(|s| s.kind == "local") {
        let mut base = model.endpoint.trim_end_matches('/').to_string();
        if base.ends_with("/chat/completions") { base.truncate(base.len() - "/chat/completions".len()); }
        if !base.ends_with("/v1") { base.push_str("/v1"); }
        input["localModel"] = json!({"baseUrl":base,"modelName":model.model_id,"apiKey":model.key,"timeoutMs":45000});
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
    command.arg(&worker_path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000);
    let mut child = command.spawn().map_err(|e| format!("Could not start research worker: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("Research stdin unavailable")?;
    stdin.write_all(input.to_string().as_bytes()).await.map_err(|e| e.to_string())?;
    drop(stdin);
    let mut stdout = child.stdout.take().ok_or("Research stdout unavailable")?.take(2 * 1024 * 1024);
    let mut stderr = child.stderr.take().ok_or("Research stderr unavailable")?.take(64000);
    let mut cancel = state.cancel.subscribe();
    let operation = async {
        let mut out = Vec::new(); let mut err = Vec::new();
        let (a, b, status) = tokio::join!(stdout.read_to_end(&mut out), stderr.read_to_end(&mut err), child.wait());
        a.map_err(|e| e.to_string())?; b.map_err(|e| e.to_string())?;
        if !status.map_err(|e| e.to_string())?.success() { return Err(format!("Research failed: {}", String::from_utf8_lossy(&err))); }
        serde_json::from_slice(&out).map_err(|e| format!("Invalid research result: {e}"))
    };
    tokio::select! {
        result = tokio::time::timeout(Duration::from_secs(240), operation) => result.map_err(|_| "Research deadline exceeded (240s).".to_string())?,
        _ = cancel.changed() => Err("Research cancelled.".into()),
    }
}
pub async fn research(state: &AppState, snapshot: Option<&BackendSnapshot>, question: &str, mode: &str, conversation_id: Option<&str>) -> Result<Value, String> {
    validate_request(question, mode)?;
    let result = run_worker(state, snapshot, json!({"question":question,"mode":mode})).await?;
    state.database()?.save_research_session(result["id"].as_str().ok_or("Missing research id")?, conversation_id, question,
        result["answer"].as_str().unwrap_or(""), &result["sources"].to_string(), Some(&result["trace"].to_string()))?;
    Ok(result)
}
#[tauri::command]
pub fn get_web_search_config(state: tauri::State<'_, AppState>) -> Result<Value, String> {
    let store = state.database()?;
    let provider: String = store.setting("web_search_provider")?;
    let searxng: String = store.setting("searxng_base_url")?;
    let google_key: String = store.setting("google_api_key")?;
    let google_cx: String = store.setting("google_cx_id")?;
    let fallback_enabled: bool = store.setting("search_fallback_enabled").unwrap_or(true);
    Ok(json!({
        "provider": if provider.is_empty() { "searxng".to_string() } else { provider },
        "searxngBaseUrl": if searxng.is_empty() { "http://127.0.0.1:8080".to_string() } else { searxng },
        "googleApiKey": google_key,
        "googleCxId": google_cx,
        "searchFallbackEnabled": fallback_enabled,
    }))
}

#[tauri::command]
pub fn save_web_search_config(state: tauri::State<'_, AppState>, provider: String, searxng_base_url: String, google_api_key: String, google_cx_id: String, search_fallback_enabled: Option<bool>) -> Result<(), String> {
    if !["searxng", "google"].contains(&provider.as_str()) {
        return Err("Provider must be searxng or google.".into());
    }
    if searxng_base_url.len() > 512 || google_api_key.len() > 512 || google_cx_id.len() > 512 {
        return Err("Configuration values exceed 512 characters.".into());
    }
    let store = state.database()?;
    store.save_setting("web_search_provider", &provider)?;
    store.save_setting("searxng_base_url", &searxng_base_url)?;
    store.save_setting("google_api_key", &google_api_key)?;
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
mod tests {
    use super::*;
    #[test]
    fn research_input_is_bounded() {
        assert!(validate_request("", "normal").is_err());
        assert!(validate_request("weather", "unbounded").is_err());
        assert!(validate_request("weather", "fast").is_ok());
    }
    #[test]
    fn research_offered_in_web_presets() {
        for mode in ["standard", "research", "creator", "code"] { assert!(crate::presets::get(mode).unwrap().harness.contains(&"web_search".to_string())); }
        assert!(!crate::presets::get("chat").unwrap().harness.contains(&"web_search".to_string()));
        assert!(crate::harness::definition("web_search").is_some());
    }
}
