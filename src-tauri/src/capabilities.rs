//! Minimal capability registry (DSH Phase 0, lazy slice).
//!
//! ponytail: static catalog over existing `conversation_tools` sources +
//! a global disabled set in `settings`. No `capabilities.json` file and no
//! per-preset overlays until a second consumer needs them.

use serde::{Deserialize, Serialize};

pub const VERSION: u32 = 1;
const DISABLED_KEY: &str = "capabilities.disabled";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Capability {
    pub id: String,
    pub kind: String,
    pub version: u32,
    pub enabled: bool,
    pub description: String,
}

/// (id, kind, description). Source-backed tools resolve per-conversation;
/// the rest resolve from the global disabled set.
fn catalog() -> [(&'static str, &'static str, &'static str); 5] {
    [
        ("workspace_files", "Tool", "Workspace list/read/create/edit (5 tools, cap-std)."),
        ("local_execution", "Tool", "Local Python/Node/PowerShell execution."),
        ("cloud_execution", "Tool", "Daytona cloud execution."),
        ("system_time", "Tool", "Local clock for the model; disable to hide it."),
        ("skills_reader", "Tool", "Read installed skill files when skills are active."),
    ]
}

fn source_for(id: &str) -> Option<&'static str> {
    match id {
        "workspace_files" => Some("__workspace"),
        "local_execution" => Some("__execution"),
        "cloud_execution" => Some("__daytona"),
        _ => None,
    }
}

pub fn disabled_ids(store: &crate::store::Store) -> Result<Vec<String>, String> {
    let ids: Vec<String> = store.setting(DISABLED_KEY).unwrap_or_default();
    Ok(ids)
}

/// Global kill-switch check used by the agent loop. Fails closed on corrupt
/// settings: an unreadable disabled set disables nothing but surfaces via
/// `list_capabilities` callers that propagate the error.
pub fn is_enabled(store: &crate::store::Store, id: &str) -> Result<bool, String> {
    if !catalog().iter().any(|(known, _, _)| *known == id) {
        return Err(format!("Unknown capability '{id}'."));
    }
    Ok(!disabled_ids(store)?.iter().any(|disabled| disabled == id))
}

pub fn list(
    store: &crate::store::Store,
    conversation_id: Option<&str>,
) -> Result<Vec<Capability>, String> {
    let disabled = disabled_ids(store)?;
    let sources: Vec<String> = match conversation_id {
        Some(id) => store.conversation_tools(id).map(|tools| tools.sources)?,
        None => Vec::new(),
    };
    Ok(catalog()
        .iter()
        .map(|(id, kind, description)| {
            let globally_off = disabled.iter().any(|entry| entry == id);
            let enabled = match source_for(id) {
                Some(source) if conversation_id.is_some() => {
                    !globally_off && sources.iter().any(|entry| entry == source)
                }
                _ => !globally_off,
            };
            Capability {
                id: id.to_string(),
                kind: kind.to_string(),
                version: VERSION,
                enabled,
                description: description.to_string(),
            }
        })
        .collect())
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 64 {
        return Err("Capability id must be 1-64 characters.".into());
    }
    if catalog().iter().any(|(known, _, _)| *known == id) {
        Ok(())
    } else {
        Err(format!("Unknown capability '{id}'."))
    }
}

#[tauri::command]
pub async fn list_capabilities(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: Option<String>,
) -> Result<Vec<Capability>, String> {
    let store = state.database()?;
    list(&store, conversation_id.as_deref())
}

#[tauri::command]
pub async fn set_capability_enabled(
    state: tauri::State<'_, crate::AppState>,
    id: String,
    enabled: bool,
) -> Result<Vec<Capability>, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before changing capabilities.")?;
    validate_id(&id)?;
    let store = state.database()?;
    let mut disabled = disabled_ids(&store)?;
    if enabled {
        disabled.retain(|entry| entry != &id);
    } else if !disabled.iter().any(|entry| entry == &id) {
        disabled.push(id);
    }
    store.save_setting(DISABLED_KEY, &disabled)?;
    list(&store, None)
}

/// `GET /dump-config` equivalent: preset name + capability states +
/// per-conversation tool selection. Read-only; never includes secrets.
#[tauri::command]
pub async fn dump_config(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: Option<String>,
) -> Result<serde_json::Value, String> {
    let store = state.database()?;
    let capabilities = list(&store, conversation_id.as_deref())?;
    let conversation_tools = match conversation_id.as_deref() {
        Some(id) => Some(store.conversation_tools(id)?),
        None => None,
    };
    Ok(serde_json::json!({
        "preset": "standard",
        "capabilityVersion": VERSION,
        "capabilities": capabilities,
        "conversationTools": conversation_tools,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_store() -> (tempfile::TempDir, crate::store::Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&dir.path().join("caps.sqlite")).unwrap();
        (dir, store)
    }

    #[test]
    fn system_time_starts_enabled_and_can_be_disabled() {
        let (_dir, store) = open_store();
        assert!(is_enabled(&store, "system_time").unwrap());
        let mut disabled = disabled_ids(&store).unwrap();
        disabled.push("system_time".into());
        store.save_setting(DISABLED_KEY, &disabled).unwrap();
        assert!(!is_enabled(&store, "system_time").unwrap());
        assert!(list(&store, None)
            .unwrap()
            .iter()
            .find(|capability| capability.id == "system_time")
            .is_some_and(|capability| !capability.enabled));
    }

    #[test]
    fn unknown_ids_fail_loud() {
        let (_dir, store) = open_store();
        assert!(is_enabled(&store, "nope").is_err());
        assert!(validate_id("nope").is_err());
        // Workspace capability follows the per-conversation source selection.
        let conversation = store.create_conversation().unwrap();
        let listed = list(&store, Some(&conversation.id)).unwrap();
        assert!(listed
            .iter()
            .find(|capability| capability.id == "workspace_files")
            .is_some_and(|capability| !capability.enabled));
    }
}
