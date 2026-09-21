//! KV slot save/restore for instant session resume (docs/EXTENSIONS.md §1.2).
//!
//! With `--slot-save-path`, llama-server can write a slot's token sequence and
//! KV state to disk (`POST /slots/{id}?action=save`) and load it back
//! (`action=restore`). The harness saves the conversation slot when a turn
//! ends, and restores it before the next turn when another conversation has
//! occupied the slot since. Reopening a long chat then skips most of the
//! prefill.
//!
//! Safety rests on two properties:
//! - Files are keyed on model, projector, runtime and launch configuration
//!   ([`cache_key`]), so KV state is never loaded into a different model or
//!   layout. A key mismatch deletes the file.
//! - After a restore, llama.cpp still compares the restored tokens with the
//!   new prompt and reuses only the common prefix (`cache_prompt`). A stale
//!   file, e.g. after compaction, costs only the reuse it cannot provide and
//!   never changes model output.
//!
//! The files hold conversation content. They live next to `locallm.sqlite`,
//! are deleted with their conversation, and are bounded by a disk budget.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::path::Path;

/// The conversation slot (parent turns; subagents use slot 1 when parallel).
pub const CONVERSATION_SLOT: i64 = 0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KvCacheSettings {
    pub enabled: bool,
    pub budget_mb: u32,
}

impl Default for KvCacheSettings {
    fn default() -> Self {
        Self { enabled: true, budget_mb: 4096 }
    }
}

impl KvCacheSettings {
    pub const KEY: &'static str = "kv_slot_cache";

    pub fn validate(&self) -> Result<(), String> {
        if !(256..=262_144).contains(&self.budget_mb) {
            return Err("The KV cache disk budget must be between 256 MB and 256 GB.".into());
        }
        Ok(())
    }

    pub fn budget_bytes(&self) -> u64 {
        u64::from(self.budget_mb) * 1024 * 1024
    }
}

fn file_identity(path: &Path) -> Value {
    let metadata = std::fs::metadata(path).ok();
    let modified = metadata
        .as_ref()
        .and_then(|meta| meta.modified().ok())
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos().to_string());
    serde_json::json!({
        "path": path.to_string_lossy(),
        "bytes": metadata.map(|meta| meta.len()),
        "modified": modified,
    })
}

/// Identity of everything that shapes the KV layout. Hashing multi-GB weights
/// on every load is too slow, so each file is identified by path, size and
/// modification time. A re-downloaded or replaced file changes the key.
pub fn cache_key(model: &Path, projector: Option<&Path>, runtime: &Path, launch: &crate::runtime_config::RuntimeConfig) -> String {
    let identity = serde_json::json!({
        "version": 1,
        "model": file_identity(model),
        "projector": projector.map(file_identity),
        "runtime": file_identity(runtime),
        "launch": launch,
    });
    format!("{:x}", Sha256::digest(identity.to_string().as_bytes()))
}

/// Slot file name for a conversation. llama-server rejects path separators,
/// and conversation ids are UUIDs, so anything else is refused here too.
pub fn filename(conversation_id: &str) -> Result<String, String> {
    let valid = !conversation_id.is_empty()
        && conversation_id.len() <= 64
        && conversation_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !valid {
        return Err("Conversation id is not usable as a KV slot file name.".into());
    }
    Ok(format!("{conversation_id}.bin"))
}

/// Measured result of a save or restore, as reported by llama-server.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotTransfer {
    pub tokens: Option<u64>,
    pub bytes: Option<u64>,
    pub ms: Option<f64>,
}

impl SlotTransfer {
    fn parse(action: &str, body: &Value) -> Self {
        let (tokens, bytes, timing) = match action {
            "save" => ("n_saved", "n_written", "save_ms"),
            _ => ("n_restored", "n_read", "restore_ms"),
        };
        Self {
            tokens: body.get(tokens).and_then(Value::as_u64),
            bytes: body.get(bytes).and_then(Value::as_u64),
            ms: body.get("timings").and_then(|timings| timings.get(timing)).and_then(Value::as_f64),
        }
    }
}

/// `POST {endpoint}/slots/{slot}?action=save|restore`. Only the managed local
/// runtime has this endpoint; it shares the runtime's bearer key.
pub async fn slot_action(
    client: &reqwest::Client,
    endpoint: &str,
    key: &str,
    slot: i64,
    action: &str,
    filename: &str,
) -> Result<SlotTransfer, String> {
    if !matches!(action, "save" | "restore") {
        return Err(format!("Unsupported slot action '{action}'."));
    }
    let url = format!("{}/slots/{slot}?action={action}", endpoint.trim_end_matches('/'));
    let response = client
        .post(url)
        .bearer_auth(key)
        .json(&serde_json::json!({"filename": filename}))
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|_| format!("KV slot {action} did not reach the local runtime."))?;
    let status = response.status();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        let detail = body["error"]["message"].as_str().unwrap_or("no detail");
        return Err(format!("KV slot {action} failed ({status}): {detail}"));
    }
    Ok(SlotTransfer::parse(action, &body))
}

/// A saved slot file as indexed in SQLite.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotEntry {
    pub conversation_id: String,
    pub cache_key: String,
    pub bytes: u64,
    pub updated_at: i64,
}

/// Least-recently-updated entries to delete so the rest fit `budget`.
/// `keep` (the conversation just saved) is never selected.
pub fn plan_purge(entries: &[SlotEntry], budget: u64, keep: &str) -> Vec<String> {
    let mut total: u64 = entries.iter().map(|entry| entry.bytes).sum();
    let mut oldest: Vec<&SlotEntry> = entries.iter().filter(|entry| entry.conversation_id != keep).collect();
    oldest.sort_by_key(|entry| entry.updated_at);
    let mut purge = Vec::new();
    for entry in oldest {
        if total <= budget {
            break;
        }
        total -= entry.bytes;
        purge.push(entry.conversation_id.clone());
    }
    purge
}

/// Remove a slot file, ignoring a file that is already gone.
pub fn remove_file(dir: &Path, conversation_id: &str) -> Result<(), String> {
    let path = dir.join(filename(conversation_id)?);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Could not delete KV slot file: {error}")),
    }
}

/// Bytes on disk for a slot file, if it exists.
pub fn file_bytes(dir: &Path, conversation_id: &str) -> Option<u64> {
    std::fs::metadata(dir.join(filename(conversation_id).ok()?)).ok().map(|meta| meta.len())
}

struct Target {
    client: reqwest::Client,
    endpoint: String,
    key: String,
    dir: std::path::PathBuf,
    cache_key: String,
    resident: Option<String>,
    settings: KvCacheSettings,
}

/// Everything a save/restore needs, or `None` when persistence does not
/// apply: remote backend, runtime without slot support, or feature disabled.
async fn target(state: &crate::AppState, backend: &crate::inference::Backend) -> Option<Target> {
    let crate::inference::Backend::Local { client, endpoint, key, .. } = backend else {
        return None;
    };
    let settings: KvCacheSettings = state.database().ok()?.setting(KvCacheSettings::KEY).unwrap_or_default();
    if !settings.enabled {
        return None;
    }
    let runtime = state.runtime.lock().await;
    Some(Target {
        client: client.clone(),
        endpoint: endpoint.clone(),
        key: key.clone(),
        dir: runtime.slot_dir.clone()?,
        cache_key: runtime.cache_key.clone(),
        resident: runtime.slot_resident.clone(),
        settings,
    })
}

async fn set_resident(state: &crate::AppState, conversation_id: &str) {
    state.runtime.lock().await.slot_resident = Some(conversation_id.to_string());
}

fn forget(state: &crate::AppState, dir: &Path, conversation_id: &str) {
    let _ = remove_file(dir, conversation_id);
    if let Ok(store) = state.database() {
        let _ = store.delete_kv_slot(conversation_id);
    }
}

/// Before a turn: when the conversation slot holds another conversation,
/// load this conversation's saved state. Returns the `kv_slot` event payload
/// to log, or `None` when nothing was attempted.
pub async fn restore_for_turn(state: &crate::AppState, backend: &crate::inference::Backend, conversation_id: &str) -> Option<Value> {
    let target = target(state, backend).await?;
    if target.resident.as_deref() == Some(conversation_id) {
        return None;
    }
    // Whatever happens next, this turn's requests will occupy the slot.
    set_resident(state, conversation_id).await;
    let entry = state.database().ok()?.kv_slot(conversation_id).ok()??;
    let base = serde_json::json!({"action": "restore", "idSlot": CONVERSATION_SLOT});
    let outcome = if entry.cache_key != target.cache_key {
        forget(state, &target.dir, conversation_id);
        serde_json::json!({"outcome": "discarded", "reason": "The model, runtime or launch configuration changed since this cache was saved."})
    } else if file_bytes(&target.dir, conversation_id).is_none() {
        forget(state, &target.dir, conversation_id);
        serde_json::json!({"outcome": "missing", "reason": "The saved cache file no longer exists."})
    } else {
        let file = filename(conversation_id).ok()?;
        match slot_action(&target.client, &target.endpoint, &target.key, CONVERSATION_SLOT, "restore", &file).await {
            Ok(transfer) => serde_json::json!({"outcome": "restored", "tokens": transfer.tokens, "bytes": transfer.bytes, "ms": transfer.ms}),
            Err(error) => serde_json::json!({"outcome": "failed", "error": error}),
        }
    };
    Some(merge(base, outcome))
}

/// After a turn: save the conversation slot, then delete the least recently
/// used files beyond the disk budget. Returns the `kv_slot` event payload.
pub async fn save_after_turn(state: &crate::AppState, backend: &crate::inference::Backend, conversation_id: &str) -> Option<Value> {
    let target = target(state, backend).await?;
    let file = filename(conversation_id).ok()?;
    let base = serde_json::json!({"action": "save", "idSlot": CONVERSATION_SLOT});
    let transfer = match slot_action(&target.client, &target.endpoint, &target.key, CONVERSATION_SLOT, "save", &file).await {
        Ok(transfer) => transfer,
        Err(error) => return Some(merge(base, serde_json::json!({"outcome": "failed", "error": error}))),
    };
    set_resident(state, conversation_id).await;
    let bytes = file_bytes(&target.dir, conversation_id).or(transfer.bytes).unwrap_or(0);
    let entry = SlotEntry { conversation_id: conversation_id.to_string(), cache_key: target.cache_key.clone(), bytes, updated_at: crate::store::now() };
    let mut purged = Vec::new();
    if let Ok(store) = state.database() {
        let _ = store.save_kv_slot(&entry);
        let entries = store.kv_slots().unwrap_or_default();
        purged = plan_purge(&entries, target.settings.budget_bytes(), conversation_id);
    }
    for id in &purged {
        forget(state, &target.dir, id);
    }
    Some(merge(base, serde_json::json!({"outcome": "saved", "tokens": transfer.tokens, "bytes": bytes, "ms": transfer.ms, "purged": purged.len()})))
}

fn merge(mut base: Value, extra: Value) -> Value {
    if let (Some(base), Value::Object(extra)) = (base.as_object_mut(), extra) {
        base.extend(extra);
    }
    base
}

/// Disk usage of saved slot files, for the settings panel.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KvCacheUsage {
    pub settings: KvCacheSettings,
    pub files: usize,
    pub bytes: u64,
    /// Whether the loaded runtime exposes slot save/restore at all.
    pub runtime_supported: Option<bool>,
}

fn slot_dir(state: &crate::AppState) -> std::path::PathBuf {
    state.data_dir.join("kv-slots")
}

#[tauri::command]
pub async fn kv_cache_usage(state: tauri::State<'_, crate::AppState>) -> Result<KvCacheUsage, String> {
    let runtime_supported = {
        let runtime = state.runtime.lock().await;
        (!runtime.endpoint.is_empty()).then(|| runtime.slot_dir.is_some())
    };
    let store = state.database()?;
    let entries = store.kv_slots()?;
    Ok(KvCacheUsage {
        settings: store.setting(KvCacheSettings::KEY)?,
        files: entries.len(),
        bytes: entries.iter().map(|entry| entry.bytes).sum(),
        runtime_supported,
    })
}

#[tauri::command]
pub async fn save_kv_cache_settings(state: tauri::State<'_, crate::AppState>, settings: KvCacheSettings) -> Result<KvCacheUsage, String> {
    settings.validate()?;
    state.database()?.save_setting(KvCacheSettings::KEY, &settings)?;
    if !settings.enabled {
        clear_all(&state)?;
    }
    kv_cache_usage(state).await
}

#[tauri::command]
pub async fn clear_kv_cache(state: tauri::State<'_, crate::AppState>) -> Result<KvCacheUsage, String> {
    clear_all(&state)?;
    kv_cache_usage(state).await
}

/// Delete every saved slot file and its index row. Turning the feature off
/// clears the files too: they hold conversation content.
fn clear_all(state: &crate::AppState) -> Result<(), String> {
    let dir = slot_dir(state);
    let entries = state.database()?.kv_slots()?;
    for entry in entries {
        remove_file(&dir, &entry.conversation_id)?;
        state.database()?.delete_kv_slot(&entry.conversation_id)?;
    }
    Ok(())
}

/// Called when a conversation is deleted; the index row cascades.
pub fn delete_for_conversation(state: &crate::AppState, conversation_id: &str) {
    let _ = remove_file(&slot_dir(state), conversation_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, bytes: u64, updated_at: i64) -> SlotEntry {
        SlotEntry { conversation_id: id.into(), cache_key: "k".into(), bytes, updated_at }
    }

    #[test]
    fn purge_removes_oldest_until_under_budget_and_keeps_current() {
        let entries = [entry("a", 400, 1), entry("b", 400, 2), entry("c", 400, 3)];
        assert_eq!(plan_purge(&entries, 1200, "c"), Vec::<String>::new());
        assert_eq!(plan_purge(&entries, 800, "c"), vec!["a".to_string()]);
        assert_eq!(plan_purge(&entries, 100, "a"), vec!["b".to_string(), "c".to_string()]);
    }

    #[test]
    fn filenames_are_plain_and_ids_are_checked() {
        assert_eq!(filename("3f2a-11").unwrap(), "3f2a-11.bin");
        let too_long = "x".repeat(65);
        for bad in ["", "../x", "a/b", "a\\b", "a.b", too_long.as_str()] {
            assert!(filename(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn cache_key_changes_with_model_file_and_launch_config() {
        let dir = tempfile::tempdir().unwrap();
        let model = dir.path().join("m.gguf");
        let runtime = dir.path().join("llama-server.exe");
        std::fs::write(&model, b"weights").unwrap();
        std::fs::write(&runtime, b"exe").unwrap();
        let config = crate::runtime_config::RuntimeConfig::default();
        let base = cache_key(&model, None, &runtime, &config);
        assert_eq!(base, cache_key(&model, None, &runtime, &config));
        let wider = crate::runtime_config::RuntimeConfig { context_length: 8192, ..Default::default() };
        assert_ne!(base, cache_key(&model, None, &runtime, &wider));
        std::fs::write(&model, b"other weights").unwrap();
        assert_ne!(base, cache_key(&model, None, &runtime, &config));
    }

    #[test]
    fn transfer_results_are_parsed_from_server_fields() {
        let saved = SlotTransfer::parse("save", &serde_json::json!({"id_slot":0,"filename":"a.bin","n_saved":1745,"n_written":14309796,"timings":{"save_ms":49.865}}));
        assert_eq!(saved, SlotTransfer { tokens: Some(1745), bytes: Some(14_309_796), ms: Some(49.865) });
        let restored = SlotTransfer::parse("restore", &serde_json::json!({"n_restored":1745,"n_read":14309796,"timings":{"restore_ms":42.9}}));
        assert_eq!(restored.tokens, Some(1745));
        assert_eq!(SlotTransfer::parse("restore", &serde_json::json!({})), SlotTransfer::default());
    }

    #[test]
    fn settings_default_on_with_a_bounded_budget() {
        let settings = KvCacheSettings::default();
        assert!(settings.enabled);
        assert!(settings.validate().is_ok());
        assert!(KvCacheSettings { budget_mb: 10, ..settings.clone() }.validate().is_err());
        assert_eq!(settings.budget_bytes(), 4096 * 1024 * 1024);
    }
}
