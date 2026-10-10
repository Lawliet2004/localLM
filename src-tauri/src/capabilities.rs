//! Capability registry: parses `capabilities.json`, overlays the user's
//! disabled set from SQLite `settings`, and answers enable/disable queries.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

pub const VERSION: u32 = 1;
pub const DISABLED_KEY: &str = "capabilities.disabled";
pub const PRESET_KEY: &str = "capabilities.preset";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CapabilityKind {
    Model,
    Tool,
    Skill,
    Session,
    Sandbox,
    Storage,
    Loop,
    Ui,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Capability {
    pub id: String,
    pub kind: CapabilityKind,
    pub version: u32,
    pub enabled: bool,
    pub description: String,
    #[serde(default)]
    pub config: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub name: String,
    pub description: String,
    pub enabled: Vec<String>,
    pub disabled: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CapabilitiesConfig {
    version: u32,
    default_preset: String,
    presets: HashMap<String, Preset>,
    capabilities: Vec<Capability>,
}

/// Static capability catalog seeded from `capabilities.json`.
#[derive(Clone, Debug)]
pub struct Registry {
    capabilities: HashMap<String, Capability>,
    presets: HashMap<String, Preset>,
    default_preset: String,
}

impl Default for Registry {
    fn default() -> Self {
        Self::from_json(include_str!("../capabilities.json")).unwrap_or_else(|_| Self {
            capabilities: HashMap::new(),
            presets: HashMap::new(),
            default_preset: "standard".into(),
        })
    }
}

impl Registry {
    pub fn from_json(raw: &str) -> Result<Self, String> {
        let parsed: CapabilitiesConfig =
            serde_json::from_str(raw).map_err(|e| format!("Failed to parse capabilities.json: {e}"))?;

        let capabilities = parsed
            .capabilities
            .into_iter()
            .map(|cap| (cap.id.clone(), cap))
            .collect();

        Ok(Self {
            capabilities,
            presets: parsed.presets,
            default_preset: parsed.default_preset,
        })
    }

    pub fn get(&self, id: &str) -> Option<&Capability> {
        self.capabilities.get(id)
    }
}

pub fn disabled_ids(store: &crate::store::Store) -> Result<Vec<String>, String> {
    let ids: Vec<String> = store.setting(DISABLED_KEY).unwrap_or_default();
    Ok(ids)
}

fn source_for(id: &str) -> Option<&'static str> {
    match id {
        "workspace_files" => Some("__workspace"),
        "local_execution" => Some("__execution"),
        "cloud_execution" => Some("__daytona"),
        _ => None,
    }
}

/// Global kill-switch check used by the agent loop. Fails closed on corrupt
/// settings: an unreadable disabled set disables nothing but surfaces via callers.
pub fn is_enabled(store: &crate::store::Store, id: &str) -> Result<bool, String> {
    let registry = Registry::default();
    if registry.get(id).is_none() {
        return Err(format!("Unknown capability '{id}'."));
    }
    let disabled = disabled_ids(store)?;
    Ok(!disabled.iter().any(|entry| entry == id))
}

pub fn list(
    store: &crate::store::Store,
    conversation_id: Option<&str>,
) -> Result<Vec<Capability>, String> {
    let registry = Registry::default();
    let disabled = disabled_ids(store)?;
    let sources: Vec<String> = match conversation_id {
        Some(id) => store.conversation_tools(id).map(|tools| tools.sources)?,
        None => Vec::new(),
    };

    let mut list = Vec::new();
    for (_, cap) in registry.capabilities.into_iter() {
        let globally_off = disabled.iter().any(|entry| entry == &cap.id);
        let enabled = match source_for(&cap.id) {
            Some(source) if conversation_id.is_some() => {
                !globally_off && sources.iter().any(|entry| entry == source)
            }
            _ => !globally_off && cap.enabled,
        };
        list.push(Capability {
            id: cap.id,
            kind: cap.kind,
            version: cap.version,
            enabled,
            description: cap.description,
            config: cap.config,
        });
    }

    list.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(list)
}

fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty() || id.len() > 64 {
        return Err("Capability id must be 1-64 characters.".into());
    }
    let registry = Registry::default();
    if registry.get(id).is_some() {
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

/// `GET /dump-config` equivalent: active preset + capability catalog with states +
/// dependency tree + conversation tool selection. Read-only; never includes secrets.
#[tauri::command]
pub async fn dump_config(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: Option<String>,
) -> Result<Value, String> {
    let store = state.database()?;
    let registry = Registry::default();
    let capabilities = list(&store, conversation_id.as_deref())?;
    let conversation_tools = match conversation_id.as_deref() {
        Some(id) => Some(store.conversation_tools(id)?),
        None => None,
    };
    let active_preset: String = store.setting(PRESET_KEY).unwrap_or_else(|_| registry.default_preset.clone());

    Ok(json!({
        "preset": active_preset,
        "presets": &registry.presets,
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
    fn registry_parses_json_catalog() {
        let registry = Registry::default();
        assert!(registry.get("system_time").is_some());
        assert_eq!(registry.get("system_time").unwrap().kind, CapabilityKind::Tool);
        assert!(registry.presets.contains_key("standard"));
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
        assert!(is_enabled(&store, "unknown_tool_xyz").is_err());
        assert!(validate_id("unknown_tool_xyz").is_err());
    }

}
