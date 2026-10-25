//! Capability registry and microkernel seam (DSH Phase 0).
//!
//! Provides dynamic capability discovery, dependency tracking, runtime mounting/unmounting,
//! and temporal composability mirroring Cordis `ctx.effect()` / `ctx.on()`.
//! Capability configuration is seeded from `capabilities.json` and overlaid
//! with user modifications in SQLite `settings`.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

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
        let parsed: CapabilitiesConfig = serde_json::from_str(raw)
            .map_err(|e| format!("Failed to parse capabilities.json: {e}"))?;

        let mut capabilities = HashMap::new();
        let mut dependencies = HashMap::new();
        let mut effects = HashMap::new();

        for cap in parsed.capabilities {
            // Register known default dependency relationships
            let reqs = match cap.id.as_str() {
                "skills_reader" => vec!["session_log".to_string()],
                "git_tools" | "lsp_tools" => {
                    vec!["workspace_files".to_string(), "local_execution".to_string()]
                }
                "cloud_execution" => vec!["storage_sqlite".to_string()],
                "subagents" => vec!["session_log".to_string(), "agent_loop".to_string()],
                "workflows" => vec!["agent_loop".to_string(), "subagents".to_string()],
                "compaction" => vec!["session_log".to_string()],
                _ => vec![],
            };
            let effs = match cap.id.as_str() {
                "system_time" => vec![
                    "tool:system_time".to_string(),
                    "prompt:system_time".to_string(),
                ],
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

    pub fn register(
        &mut self,
        capability: Capability,
        requires: Vec<String>,
        effects: Vec<String>,
    ) {
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
        self.dependencies
            .get(id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
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

    fn ids(&self) -> impl Iterator<Item = &String> {
        self.capabilities.keys()
    }
}

/// Explicitly saved capability kills (the `capabilities.disabled` setting).
/// Corrupt values propagate as errors — callers must not silently treat an
/// unreadable kill switch as "everything enabled".
pub fn disabled_ids(store: &crate::store::Store) -> Result<Vec<String>, String> {
    store.setting(DISABLED_KEY)
}

/// Effective kill set: explicit ids plus every capability that transitively
/// requires a disabled dependency (e.g. disabling `session_log` also disables
/// `skills_reader`, `subagents`, `workflows` and `compaction`). Catalog
/// `enabled` defaults are NOT consulted here — presets alone decide what is
/// offered — so advanced capabilities stay usable until explicitly killed.
pub fn effective_disabled(store: &crate::store::Store) -> Result<HashSet<String>, String> {
    let registry = Registry::default();
    let mut disabled: HashSet<String> = disabled_ids(store)?.into_iter().collect();
    let mut changed = true;
    while changed {
        changed = false;
        for id in registry.ids() {
            if !disabled.contains(id)
                && registry
                    .requires(id)
                    .iter()
                    .any(|required| disabled.contains(required))
            {
                disabled.insert(id.clone());
                changed = true;
            }
        }
    }
    Ok(disabled)
}

/// Capabilities with a model-facing tool/source surface. Dependency ids
/// (`session_log`, `storage_sqlite`, `agent_loop`) gate through the cascade.
pub const GATED: &[&str] = &[
    "workspace_files",
    "local_execution",
    "cloud_execution",
    "system_time",
    "skills_reader",
    "subagents",
    "workflows",
    "compaction",
    "git_tools",
    "lsp_tools",
    "mcp_resources_prompts",
];

/// Source pseudo-ids ↔ capabilities (single mapping used by assembly and UI).
const SOURCE_CAPABILITIES: &[(&str, &str)] = &[
    ("__workspace", "workspace_files"),
    ("__execution", "local_execution"),
    ("__daytona", "cloud_execution"),
];

/// Capability gating a tool-selection source pseudo-id.
pub fn source_capability(source: &str) -> Option<&'static str> {
    SOURCE_CAPABILITIES
        .iter()
        .find(|(id, _)| *id == source)
        .map(|(_, capability)| *capability)
}

/// Capability gating a harness/catalog alias. Mirrors the source split in
/// `presets::harness_source_allowed` plus orchestration families.
pub fn harness_alias_capability(alias: &str) -> Option<&'static str> {
    match alias {
        name if crate::debug_tools::owns(name) => Some("local_execution"),
        name if crate::development_tools::owns(name) => {
            Some(if crate::development_tools::execution_tool(name) {
                "local_execution"
            } else {
                "workspace_files"
            })
        }
        "file_search" | "file_read" | "file_write" | "format_paths" => Some("workspace_files"),
        "skill_list" | "skill_load" => Some("skills_reader"),
        "git_status" | "git_diff" | "git_stage" | "git_unstage" | "git_commit" | "git_branch"
        | "git_history" | "worktree_enter" | "worktree_exit" | "worktree_list" => Some("git_tools"),
        "lsp_diagnostics"
        | "lsp_symbols"
        | "lsp_references"
        | "lsp_prepare_rename"
        | "lsp_rename"
        | "lsp_definition"
        | "lsp_hover"
        | "lsp_implementations"
        | "lsp_code_actions"
        | "lsp_prepare_code_action"
        | "lsp_apply_edit"
        | "lsp_execute_command" => Some("lsp_tools"),
        "run_code"
        | "execute_command"
        | "terminal_create"
        | "terminal_send"
        | "terminal_close"
        | "process_start"
        | "process_poll"
        | "process_write"
        | "process_stop"
        | "shell_run"
        | "check_run"
        | "check_report"
        | "docker_exec"
        | "ptc_run"
        | "workspace_open_preview" => Some("local_execution"),
        "subagent"
        | "send_message"
        | "interrupt_agent"
        | "list_agents"
        | "list_subagent_models" => Some("subagents"),
        "workflow_run" | "ralph_run" => Some("workflows"),
        "compact_conversation" => Some("compaction"),
        _ => None,
    }
}

/// Capability gating an assembled tool, if any. MCP and automation tools are
/// governed by their own connector/approval policies, not this registry.
pub fn tool_capability(tool: &crate::connectors::AgentTool) -> Option<&'static str> {
    if tool.is_mcp_protocol() {
        return Some("mcp_resources_prompts");
    }
    if tool.is_harness() {
        return tool.harness_name().and_then(harness_alias_capability);
    }
    match tool.connector.as_str() {
        "Workspace" => Some("workspace_files"),
        "Local execution" => Some("local_execution"),
        "Daytona" => Some("cloud_execution"),
        "Skills" => Some("skills_reader"),
        "System" if tool.alias == "system_time" => Some("system_time"),
        _ => None,
    }
}

/// Resolved capability kills for one boundary. Assembly resolves once and
/// drops gated families; dispatch re-resolves per call so stale entries from
/// an already-frozen catalog still deny. Unreadable settings resolve closed:
/// every gated capability denies with a visible reason.
#[derive(Clone, Debug)]
pub struct ToolPolicy {
    disabled: HashSet<String>,
    failure: Option<String>,
}

impl ToolPolicy {
    pub fn resolve(store: &crate::store::Store) -> Self {
        match effective_disabled(store) {
            Ok(disabled) => Self { disabled, failure: None },
            Err(error) => Self {
                disabled: GATED.iter().map(|id| id.to_string()).collect(),
                failure: Some(format!(
                    "Capability settings are unreadable ({error}); gated tools stay disabled until it is fixed."
                )),
            },
        }
    }

    /// Deny-everything policy for when the store itself cannot be opened.
    pub fn closed(reason: String) -> Self {
        Self {
            disabled: GATED.iter().map(|id| id.to_string()).collect(),
            failure: Some(format!(
                "{reason} Gated tools stay disabled until it is fixed."
            )),
        }
    }

    /// Policy for a fallible database handle (`AppState::database`).
    pub fn guard(store: Result<std::sync::MutexGuard<'_, crate::store::Store>, String>) -> Self {
        match store {
            Ok(store) => Self::resolve(&store),
            Err(error) => Self::closed(format!("Capability settings are unreachable ({error}).")),
        }
    }

    /// Reason this capability cannot run right now, if it is disabled.
    pub fn denial(&self, id: &str) -> Option<String> {
        if let Some(reason) = &self.failure {
            if self.disabled.contains(id) {
                return Some(reason.clone());
            }
            return None;
        }
        self.disabled
            .contains(id)
            .then(|| format!("The '{id}' capability is disabled in Capability settings."))
    }

    /// Reason a harness/catalog alias cannot run, if capability-gated.
    pub fn denial_for_alias(&self, alias: &str) -> Option<String> {
        if crate::debug_tools::owns(alias) || crate::development_tools::execution_tool(alias) {
            if let Some(reason) = self.denial("workspace_files") {
                return Some(reason);
            }
        }
        harness_alias_capability(alias).and_then(|id| self.denial(id))
    }

    /// Reason an assembled tool cannot run, if capability-gated.
    pub fn denial_for_tool(&self, tool: &crate::connectors::AgentTool) -> Option<String> {
        if tool.is_harness() {
            return tool
                .harness_name()
                .and_then(|alias| self.denial_for_alias(alias));
        }
        tool_capability(tool).and_then(|id| self.denial(id))
    }

    pub fn allows_tool(&self, tool: &crate::connectors::AgentTool) -> bool {
        self.denial_for_tool(tool).is_none()
    }
}

fn source_for(id: &str) -> Option<&'static str> {
    SOURCE_CAPABILITIES
        .iter()
        .find(|(_, capability)| *capability == id)
        .map(|(source, _)| *source)
}

/// Global kill-switch check used by the agent loop. Corrupt settings return
/// an error so callers fail closed instead of silently enabling permissions.
pub fn is_enabled(store: &crate::store::Store, id: &str) -> Result<bool, String> {
    let registry = Registry::default();
    if registry.get(id).is_none() {
        return Err(format!("Unknown capability '{id}'."));
    }
    Ok(!effective_disabled(store)?.contains(id))
}

pub fn list(
    store: &crate::store::Store,
    conversation_id: Option<&str>,
) -> Result<Vec<Capability>, String> {
    let registry = Registry::default();
    let disabled = effective_disabled(store)?;
    let sources: Vec<String> = match conversation_id {
        Some(id) => store.conversation_tools(id).map(|tools| tools.sources)?,
        None => Vec::new(),
    };

    let mut list = Vec::new();
    for (_, cap) in registry.capabilities.into_iter() {
        let globally_off = disabled.contains(&cap.id);
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
    let active_preset: String = store
        .setting(PRESET_KEY)
        .unwrap_or_else(|_| registry.default_preset.clone());

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
        assert_eq!(
            registry.get("system_time").unwrap().kind,
            CapabilityKind::Tool
        );
        assert!(registry
            .effects("system_time")
            .contains(&"tool:system_time".to_string()));
        assert!(registry
            .requires("workflows")
            .contains(&"subagents".to_string()));
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
    fn catalog_defaults_do_not_disable_advanced_capabilities() {
        // `subagents`/`workflows` carry enabled:false in capabilities.json, but
        // presets still offer them — only a saved kill disables them.
        let (_dir, store) = open_store();
        assert!(is_enabled(&store, "subagents").unwrap());
        assert!(is_enabled(&store, "workflows").unwrap());
        let policy = ToolPolicy::resolve(&store);
        assert!(policy.denial("subagents").is_none());
        assert!(policy.denial("workflows").is_none());
    }

    #[test]
    fn disabling_a_dependency_cascades_to_dependents() {
        let (_dir, store) = open_store();
        store
            .save_setting(DISABLED_KEY, &vec!["session_log".to_string()])
            .unwrap();
        let disabled = effective_disabled(&store).unwrap();
        for id in [
            "session_log",
            "skills_reader",
            "subagents",
            "workflows",
            "compaction",
        ] {
            assert!(
                disabled.contains(id),
                "{id} should cascade off with session_log"
            );
            assert!(!is_enabled(&store, id).unwrap());
        }
        // Unrelated capabilities stay enabled.
        for id in ["workspace_files", "local_execution", "system_time"] {
            assert!(is_enabled(&store, id).unwrap());
        }
        store
            .save_setting(DISABLED_KEY, &vec!["storage_sqlite".to_string()])
            .unwrap();
        assert!(!is_enabled(&store, "cloud_execution").unwrap());
        store
            .save_setting(DISABLED_KEY, &vec!["agent_loop".to_string()])
            .unwrap();
        let disabled = effective_disabled(&store).unwrap();
        assert!(disabled.contains("subagents") && disabled.contains("workflows"));
        assert!(
            !disabled.contains("skills_reader"),
            "skills_reader does not require agent_loop"
        );
        store
            .save_setting(DISABLED_KEY, &vec!["subagents".to_string()])
            .unwrap();
        let disabled = effective_disabled(&store).unwrap();
        assert!(
            disabled.contains("workflows"),
            "workflows requires subagents"
        );
    }

    #[test]
    fn corrupt_disabled_setting_fails_closed_and_visibly() {
        let (_dir, store) = open_store();
        // A wrong-typed value (string, not a list) makes every capability
        // read of the kill switch fail — settings deserialize errors surface.
        store.save_setting(DISABLED_KEY, &"not a list").unwrap();
        assert!(disabled_ids(&store).is_err());
        assert!(effective_disabled(&store).is_err());
        assert!(is_enabled(&store, "system_time").is_err());
        assert!(list(&store, None).is_err());
        let policy = ToolPolicy::resolve(&store);
        for id in GATED {
            assert!(
                policy.denial(id).is_some(),
                "{id} must deny on unreadable settings"
            );
        }
        // A policy for an unopened store denies gated capabilities too.
        let closed = ToolPolicy::closed("store unavailable".into());
        for id in GATED {
            assert!(closed.denial(id).is_some(), "{id} must deny when closed");
        }
    }

    #[test]
    fn policy_maps_sources_aliases_and_tools() {
        let (_dir, store) = open_store();
        assert_eq!(source_capability("__workspace"), Some("workspace_files"));
        assert_eq!(source_capability("__execution"), Some("local_execution"));
        assert_eq!(source_capability("__daytona"), Some("cloud_execution"));
        assert!(source_capability("__browser").is_none());
        for (alias, capability) in [
            ("file_read", "workspace_files"),
            ("file_write", "workspace_files"),
            ("file_search", "workspace_files"),
            ("run_code", "local_execution"),
            ("terminal_send", "local_execution"),
            ("ptc_run", "local_execution"),
            ("subagent", "subagents"),
            ("send_message", "subagents"),
            ("list_agents", "subagents"),
            ("workflow_run", "workflows"),
            ("ralph_run", "workflows"),
            ("compact_conversation", "compaction"),
        ] {
            assert_eq!(harness_alias_capability(alias), Some(capability), "{alias}");
        }
        for alias in [
            "todo_write",
            "artifact_read",
            "finish",
            "ask_user",
            "web_search",
        ] {
            assert!(harness_alias_capability(alias).is_none(), "{alias}");
        }
        let policy = ToolPolicy::resolve(&store);
        assert_eq!(
            tool_capability(&crate::connectors::AgentTool::system_time()),
            Some("system_time")
        );
        assert_eq!(
            tool_capability(&crate::connectors::AgentTool::harness("workflow_run").unwrap()),
            Some("workflows")
        );
        assert_eq!(
            tool_capability(&crate::connectors::AgentTool::harness("file_read").unwrap()),
            Some("workspace_files")
        );
        assert!(
            tool_capability(&crate::connectors::AgentTool::harness("todo_write").unwrap())
                .is_none()
        );
        assert!(policy.allows_tool(&crate::connectors::AgentTool::harness("todo_write").unwrap()));
        store
            .save_setting(DISABLED_KEY, &vec!["subagents".to_string()])
            .unwrap();
        let policy = ToolPolicy::resolve(&store);
        assert!(policy.denial_for_alias("subagent").is_some());
        assert!(
            policy.denial_for_alias("workflow_run").is_some(),
            "workflows require subagents"
        );
        assert!(policy.denial_for_alias("todo_write").is_none());
    }

    #[test]
    fn disabled_execution_tool_denies_assembled_tool() {
        let (_dir, store) = open_store();
        let temp = tempfile::tempdir().unwrap();
        let execution = crate::execution::LocalExecution::new(
            crate::execution::ExecutionConfig::default(),
            temp.path().to_str().unwrap(),
        )
        .unwrap();
        let tool = std::sync::Arc::new(execution).tool();
        assert_eq!(tool_capability(&tool), Some("local_execution"));
        assert!(ToolPolicy::resolve(&store).allows_tool(&tool));
        store
            .save_setting(DISABLED_KEY, &vec!["local_execution".to_string()])
            .unwrap();
        let policy = ToolPolicy::resolve(&store);
        assert!(policy.denial_for_tool(&tool).is_some());
        assert!(!policy.allows_tool(&tool));
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
        registry.register(
            custom,
            vec!["storage_sqlite".into()],
            vec!["sandbox:isolate".into()],
        );
        assert_eq!(
            registry.requires("custom_plug"),
            &["storage_sqlite".to_string()]
        );
        assert_eq!(
            registry.effects("custom_plug"),
            &["sandbox:isolate".to_string()]
        );

        let disposed = registry.dispose("custom_plug");
        assert!(disposed.is_some());
        assert!(registry.get("custom_plug").is_none());
        assert!(registry.requires("custom_plug").is_empty());
    }
}
