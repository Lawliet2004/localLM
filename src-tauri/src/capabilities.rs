//! Capability registry and microkernel seam (DSH Phase 0).
//!
//! Provides dynamic capability discovery, dependency tracking, runtime mounting/unmounting,
//! and temporal composability mirroring Cordis `ctx.effect()` / `ctx.on()`.
//! Capability configuration is seeded from `capabilities.json` and overlaid
//! with user modifications in SQLite `settings`.

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
    Scheduling,
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

/// Dynamic capability registry supporting dependencies (`requires`) and
/// lifecycle hooks (`effects`).
#[derive(Clone, Debug)]
pub struct Registry {
    capabilities: HashMap<String, Capability>,
    dependencies: HashMap<String, Vec<String>>,
    effects: HashMap<String, Vec<String>>,
    presets: HashMap<String, Preset>,
    default_preset: String,
}

impl Default for Registry {
    fn default() -> Self {
        Self::from_json(include_str!("../capabilities.json"))
            .unwrap_or_else(|_| Self::minimal_fallback())
    }
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_json(raw: &str) -> Result<Self, String> {
        let parsed: CapabilitiesConfig =
            serde_json::from_str(raw).map_err(|e| format!("Failed to parse capabilities.json: {e}"))?;

        let mut capabilities = HashMap::new();
        let mut dependencies = HashMap::new();
        let mut effects = HashMap::new();

        for cap in parsed.capabilities {
            // Register known default dependency relationships
            let reqs = match cap.id.as_str() {
                "skills_reader" => vec!["session_log".to_string()],
                "cloud_execution" => vec!["storage_sqlite".to_string()],
                "subagents" => vec!["session_log".to_string(), "agent_loop".to_string()],
                "workflows" => vec!["agent_loop".to_string(), "subagents".to_string()],
                "compaction" => vec!["session_log".to_string()],
                _ => vec![],
            };
            let effs = match cap.id.as_str() {
                "system_time" => vec!["tool:system_time".to_string(), "prompt:system_time".to_string()],
                "workspace_files" => vec!["tool:workspace".to_string()],
                "local_execution" => vec!["tool:local_exec".to_string()],
                "cloud_execution" => vec!["tool:daytona".to_string()],
                "skills_reader" => vec!["tool:skill_reader".to_string()],
                _ => vec![],
            };

            dependencies.insert(cap.id.clone(), reqs);
            effects.insert(cap.id.clone(), effs);
            capabilities.insert(cap.id.clone(), cap);
        }

        Ok(Self {
            capabilities,
            dependencies,
            effects,
            presets: parsed.presets,
            default_preset: parsed.default_preset,
        })
    }

    fn minimal_fallback() -> Self {
        let mut reg = Self {
            capabilities: HashMap::new(),
            dependencies: HashMap::new(),
            effects: HashMap::new(),
            presets: HashMap::new(),
            default_preset: "standard".into(),
        };
        reg.register(
            Capability {
                id: "system_time".into(),
                kind: CapabilityKind::Tool,
                version: VERSION,
                enabled: true,
                description: "Local clock for the model.".into(),
                config: json!({}),
            },
            vec![],
            vec!["tool:system_time".into()],
        );
        reg
    }

    pub fn register(&mut self, capability: Capability, requires: Vec<String>, effects: Vec<String>) {
        let id = capability.id.clone();
        self.capabilities.insert(id.clone(), capability);
        self.dependencies.insert(id.clone(), requires);
        self.effects.insert(id, effects);
    }

    pub fn dispose(&mut self, id: &str) -> Option<Capability> {
        self.dependencies.remove(id);
        self.effects.remove(id);
        self.capabilities.remove(id)
    }

    pub fn get(&self, id: &str) -> Option<&Capability> {
        self.capabilities.get(id)
    }

    pub fn requires(&self, id: &str) -> &[String] {
        self.dependencies.get(id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn effects(&self, id: &str) -> &[String] {
        self.effects.get(id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn presets(&self) -> &HashMap<String, Preset> {
        &self.presets
    }

    pub fn default_preset(&self) -> &str {
        &self.default_preset
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
        "presets": registry.presets,
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
    fn registry_parses_json_and_resolves_dependencies() {
        let registry = Registry::default();
        assert!(registry.get("system_time").is_some());
        assert_eq!(registry.get("system_time").unwrap().kind, CapabilityKind::Tool);
        assert!(registry.effects("system_time").contains(&"tool:system_time".to_string()));
        assert!(registry.requires("workflows").contains(&"subagents".to_string()));
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

    #[test]
    fn dynamic_register_and_dispose_effects() {
        let mut registry = Registry::minimal_fallback();
        let custom = Capability {
            id: "custom_plug".into(),
            kind: CapabilityKind::Sandbox,
            version: 1,
            enabled: true,
            description: "Custom sandbox plugin".into(),
            config: json!({"timeoutSecs": 30}),
        };
        registry.register(custom, vec!["storage_sqlite".into()], vec!["sandbox:isolate".into()]);
        assert_eq!(registry.requires("custom_plug"), &["storage_sqlite".to_string()]);
        assert_eq!(registry.effects("custom_plug"), &["sandbox:isolate".to_string()]);

        let disposed = registry.dispose("custom_plug");
        assert!(disposed.is_some());
        assert!(registry.get("custom_plug").is_none());
        assert!(registry.requires("custom_plug").is_empty());
    }
}
