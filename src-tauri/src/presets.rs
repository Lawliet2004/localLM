//! Runtime modes: Standard / Code (PTC) / Minimal / Creator.
//!
//! Presets are validated config patches over tool visibility, not separate
//! executors. The agent loop, approval, and audit path are identical.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Local source ids (`__workspace`, `__execution`, `__daytona`) offered.
    pub sources: Vec<String>,
    /// Whether remote MCP connector tools are offered.
    pub mcp: bool,
    /// Whether the model clock is offered (still gated by capabilities).
    pub system_time: bool,
    /// Whether installed skill instructions + reader are offered.
    pub skills: bool,
    /// Harness tool aliases offered to the model in this preset.
    pub harness: Vec<String>,
}

pub const STANDARD: &str = "standard";
pub const CODE: &str = "code";
pub const MINIMAL: &str = "minimal";
pub const CREATOR: &str = "creator";

pub fn ids() -> [&'static str; 4] {
    [STANDARD, CODE, MINIMAL, CREATOR]
}

pub fn validate(id: &str) -> Result<(), String> {
    if ids().contains(&id) {
        Ok(())
    } else {
        Err(format!("Unknown runtime preset '{id}'. Choose standard, code, minimal, or creator."))
    }
}

fn harness(all: &[&str]) -> Vec<String> {
    all.iter().map(|item| item.to_string()).collect()
}

pub fn get(id: &str) -> Result<Preset, String> {
    validate(id)?;
    let preset = match id {
        CODE => Preset {
            id: CODE.into(),
            name: "Code (PTC)".into(),
            description: "Programmatic tool calling: the model writes one TypeScript program against the generated SDK; each sdk_* call is policy-checked before execution.".into(),
            sources: vec!["__workspace".into(), "__execution".into()],
            mcp: true,
            system_time: true,
            skills: true,
            harness: harness(&["ptc_run", "todo_write", "goal_set", "memory_teach", "memory_recall", "web_fetch", "file_search", "ask_user"]),
        },
        MINIMAL => Preset {
            id: MINIMAL.into(),
            name: "Minimal".into(),
            description: "Benchmarking mode: persistent shell plus file editing only.".into(),
            sources: vec!["__execution".into()],
            mcp: false,
            system_time: false,
            skills: false,
            // Terminal sessions back the shell; edit arrives via the workspace
            // edit tool filtered to `edit_file` below.
            harness: harness(&["terminal_create", "terminal_send", "terminal_close"]),
        },
        CREATOR => Preset {
            id: CREATOR.into(),
            name: "Creator".into(),
            description: "Standard plus runtime inspector, in-memory plugin testing, and preset-authoring guidance.".into(),
            sources: vec!["__workspace".into(), "__execution".into(), "__daytona".into()],
            mcp: true,
            system_time: true,
            skills: true,
            harness: harness(&[
                "todo_write", "goal_set", "goal_clear", "subagent", "send_message",
                "interrupt_agent", "list_agents", "workflow_run", "ralph_run",
                "terminal_create", "terminal_send", "terminal_close", "web_fetch",
                "file_search", "memory_teach", "memory_recall", "schedule_create",
                "schedule_list", "ask_user", "plugin_test", "preset_guide", "compact_conversation",
            ]),
        },
        _ => Preset {
            id: STANDARD.into(),
            name: "Standard".into(),
            description: "Full local toolset with per-conversation selection and approval.".into(),
            sources: vec!["__workspace".into(), "__execution".into(), "__daytona".into()],
            mcp: true,
            system_time: true,
            skills: true,
            harness: harness(&[
                "todo_write", "goal_set", "goal_clear", "subagent", "send_message",
                "interrupt_agent", "list_agents", "list_subagent_models", "workflow_run",
                "ralph_run", "terminal_create", "terminal_send", "terminal_close",
                "web_fetch", "file_search", "memory_teach", "memory_recall",
                "schedule_create", "schedule_list", "ask_user", "compact_conversation",
            ]),
        },
    };
    Ok(preset)
}

/// Minimal-mode workspace surface: edit only (the `str_replace_editor`
/// equivalent); listing/reading go through execution when needed.
pub fn minimal_workspace_tools() -> [&'static str; 1] {
    ["edit_file"]
}

pub fn list() -> Vec<Preset> {
    ids().iter().map(|id| get(id).expect("built-in preset")).collect()
}

/// TypeScript SDK prelude served to the model in Code preset: one declaration
/// per active tool alias plus the `sdk_` call convention enforced by `ptc_run`.
pub fn ts_sdk(aliases: &[String]) -> String {
    let mut out = String::from(
        "// LocalLM programmatic tool-calling SDK (generated).\n// Call tools ONLY via sdk_<alias>(args) with a JSON-serializable object.\n// Each call is policy-checked individually before execution.\ndeclare function run_tool(name: string, args: unknown): unknown;\n",
    );
    for alias in aliases {
        out.push_str(&format!("declare function sdk_{alias}(args: unknown): unknown;\n"));
    }
    out
}

#[tauri::command]
pub fn list_presets() -> Vec<Preset> {
    list()
}

#[tauri::command]
pub fn get_preset(state: tauri::State<'_, crate::AppState>, conversation_id: String) -> Result<Preset, String> {
    let store = state.database()?;
    let id = store.conversation_preset(&conversation_id).unwrap_or_else(|_| STANDARD.to_string());
    get(&id)
}

#[tauri::command]
pub async fn set_preset(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
    preset: String,
) -> Result<Preset, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before changing the runtime preset.")?;
    validate(&preset)?;
    state.database()?.save_conversation_preset(&conversation_id, &preset)?;
    get(&preset)
}

/// Static authoring guidance surfaced as the `preset_guide` harness tool.
pub fn authoring_guide() -> &'static str {
    "Compose a preset without forking source: pick sources (workspace, execution, daytona), \
     MCP on/off, system_time on/off, skills on/off, and a harness tool subset. Standard = everything; \
     Minimal = execution + edit_file + terminal_* only. New presets need a Rust change today; \
     file-based preset overlays are future work."
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_presets_validate_and_minimal_is_two_tools() {
        for id in ids() {
            let preset = get(id).unwrap();
            assert_eq!(preset.id, id);
        }
        assert!(get("nope").is_err());
        // Minimal benchmark surface: shell (execution) + edit_file + terminal_*.
        let minimal = get(MINIMAL).unwrap();
        assert_eq!(minimal.sources, vec!["__execution".to_string()]);
        assert!(!minimal.mcp && !minimal.system_time && !minimal.skills);
        assert_eq!(minimal_workspace_tools(), ["edit_file"]);
    }
    #[test]
    fn sdk_lists_every_offered_alias() {
        let sdk = ts_sdk(&["a".to_string(), "b".to_string()]);
        assert!(sdk.contains("sdk_a") && sdk.contains("sdk_b"));
    }
}
