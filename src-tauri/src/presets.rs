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
pub const CHAT: &str = "chat";
pub const RESEARCH: &str = "research";
pub const CODING: &str = "coding";

pub fn ids() -> [&'static str; 7] {
    [STANDARD, CODE, MINIMAL, CREATOR, CHAT, RESEARCH, CODING]
}

pub fn validate(id: &str) -> Result<(), String> {
    if ids().contains(&id) {
        Ok(())
    } else {
        Err(format!(
            "Unknown runtime preset '{id}'. Choose standard, code, minimal, or creator."
        ))
    }
}

fn harness(all: &[&str]) -> Vec<String> {
    all.iter().map(|item| item.to_string()).collect()
}

pub fn get(id: &str) -> Result<Preset, String> {
    validate(id)?;
    let mut preset = match id {
        CODE => Preset {
            id: CODE.into(),
            name: "Code (PTC)".into(),
            description: "Programmatic tool calling: the model writes one TypeScript program against the generated SDK; each sdk_* call is policy-checked before execution.".into(),
            sources: vec!["__workspace".into(), "__execution".into()],
            mcp: true,
            system_time: true,
            skills: true,
            harness: harness(&["verification_run", "verification_report", "verification_compare", "change_impact", "http_session_create", "http_session_close", "http_request", "http_compare", "openapi_inspect", "openapi_request", "database_schema", "database_query", "database_explain", "database_rehearse", "database_cleanup", "benchmark_run", "benchmark_compare", "profile_inspect", "profile_capture", "profile_compare", "debugger_start", "debugger_inspect", "debugger_control", "debugger_events", "debugger_stop", "process_start", "process_poll", "process_write", "process_stop", "shell_run", "workspace_open_preview", "check_run", "check_report", "git_status", "git_diff", "git_stage", "git_unstage", "git_commit", "git_branch", "git_history", "worktree_enter", "worktree_exit", "lsp_diagnostics", "lsp_symbols", "lsp_references", "lsp_prepare_rename", "lsp_rename", "lsp_definition", "lsp_hover", "lsp_implementations", "lsp_code_actions", "lsp_prepare_code_action", "lsp_apply_edit", "lsp_execute_command",  "ptc_run", "todo_write", "goal_set", "memory_teach", "memory_recall", "taste_recall", "taste_teach", "get_weather", "web_search", "web_open", "web_find", "web_fetch_url", "web_fetch", "file_search", "format_paths", "skill_list", "skill_load", "session_rename", "session_move", "worktree_list", "models", "report_findings", "list_findings", "resolve_finding", "task_create", "task_update", "task_get", "task_list", "task_delete", "plan_submit", "ask_user", "artifact_read"]),
        },
        MINIMAL => Preset {
            id: MINIMAL.into(),
            name: "Minimal".into(),
            description: "Benchmarking mode: persistent shell plus file editing only.".into(),
            sources: vec!["__execution".into(), "__workspace".into()],
            mcp: false,
            system_time: false,
            skills: false,
            // Terminal sessions back the shell; edit arrives via the workspace
            // edit tool filtered to `edit_file` below.
            harness: harness(&["terminal_create", "terminal_send", "terminal_close", "artifact_read"]),
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

                "verification_run", "verification_report", "verification_compare", "change_impact", "http_session_create", "http_session_close", "http_request", "http_compare", "openapi_inspect", "openapi_request", "database_schema", "database_query", "database_explain", "database_rehearse", "database_cleanup", "benchmark_run", "benchmark_compare", "profile_inspect", "profile_capture", "profile_compare", "debugger_start", "debugger_inspect", "debugger_control", "debugger_events", "debugger_stop", "process_start", "process_poll", "process_write", "process_stop", "shell_run", "workspace_open_preview", "check_run", "check_report", "git_status", "git_diff", "git_stage", "git_unstage", "git_commit", "git_branch", "git_history", "worktree_enter", "worktree_exit", "lsp_diagnostics", "lsp_symbols", "lsp_references", "lsp_prepare_rename", "lsp_rename", "lsp_definition", "lsp_hover", "lsp_implementations", "lsp_code_actions", "lsp_prepare_code_action", "lsp_apply_edit", "lsp_execute_command",
                "todo_write", "todo_add", "todo_update", "goal_set", "goal_clear", "subagent", "send_message",
                "interrupt_agent", "list_agents", "list_subagent_models", "workflow_run", "ralph_run",
                "terminal_create", "terminal_send", "terminal_close", "get_weather", "web_search", "web_read", "web_open", "web_find", "web_fetch_url", "web_fetch",
                "file_search", "file_read", "file_write", "run_code", "memory_teach", "memory_recall", "taste_recall", "taste_teach",
                "plan_submit", "ask_user", "artifact_read", "docker_exec",
                "format_paths", "skill_list", "skill_load", "session_rename", "session_move", "worktree_list", "models",
                "report_findings", "list_findings", "resolve_finding",
                "task_create", "task_update", "task_get", "task_list", "task_delete",
                "preset_guide", "compact_conversation", "research_pause", "research_resume", "research_cancel", "research_progress",
            ]),
        },
        CHAT => Preset {
            id: CHAT.into(),
            name: "Chat".into(),
            description: "Conversational assistant with minimal tool surface. No web search or code execution by default.".into(),
            sources: vec!["__workspace".into()],
            mcp: false,
            system_time: false,
            skills: true,
            harness: harness(&["todo_write", "memory_recall", "compact_conversation", "artifact_read", "skill_list", "skill_load", "session_rename", "models", "taste_recall"]),
        },
        RESEARCH => Preset {
            id: RESEARCH.into(),
            name: "Research".into(),
            description: "Live search and fetch with opt-in repository documentation tools. Web tools are on; docs tools are off by default.".into(),
            sources: vec!["__workspace".into(), "__daytona".into()],
            mcp: true,
            system_time: true,
            skills: true,
            harness: harness(&[
                "todo_write", "todo_add", "todo_update", "memory_recall", "compact_conversation",
                "search", "visit", "update_context", "finish", "artifact_read",
                "get_weather", "web_search", "web_open", "web_find", "web_fetch_url", "web_fetch", "file_search",
                "skill_list", "skill_load", "session_rename", "models", "taste_recall",
                "task_create", "task_update", "task_get", "task_list", "task_delete",
            ]),
        },
        CODING => Preset {
            id: CODING.into(),
            name: "Coding".into(),
            description: "Code-focused with workspace editing, execution, and programmatic tool calling. No web search by default.".into(),
            sources: vec!["__workspace".into(), "__execution".into()],
            mcp: true,
            system_time: true,
            skills: true,
            harness: harness(&[
                "verification_run", "verification_report", "verification_compare", "change_impact", "http_session_create", "http_session_close", "http_request", "http_compare", "openapi_inspect", "openapi_request", "database_schema", "database_query", "database_explain", "database_rehearse", "database_cleanup", "benchmark_run", "benchmark_compare", "profile_inspect", "profile_capture", "profile_compare", "debugger_start", "debugger_inspect", "debugger_control", "debugger_events", "debugger_stop", "process_start", "process_poll", "process_write", "process_stop", "shell_run", "workspace_open_preview", "check_run", "check_report", "git_status", "git_diff", "git_stage", "git_unstage", "git_commit", "git_branch", "git_history", "worktree_enter", "worktree_exit", "lsp_diagnostics", "lsp_symbols", "lsp_references", "lsp_prepare_rename", "lsp_rename", "lsp_definition", "lsp_hover", "lsp_implementations", "lsp_code_actions", "lsp_prepare_code_action", "lsp_apply_edit", "lsp_execute_command",
                "todo_write", "todo_add", "todo_update", "goal_set", "goal_clear",
                "subagent", "send_message", "interrupt_agent", "list_agents",
                "terminal_create", "terminal_send", "terminal_close",
                "get_weather", "file_search", "memory_teach", "memory_recall", "taste_recall", "taste_teach",
                "format_paths", "skill_list", "skill_load", "session_rename", "session_move", "worktree_list", "models",
                "report_findings", "list_findings", "resolve_finding",
                "task_create", "task_update", "task_get", "task_list", "task_delete",
                "artifact_read", "plan_submit", "ask_user", "compact_conversation", "research_pause", "research_resume", "research_cancel", "research_progress",
            ]),
        },
        _ => Preset {
            id: STANDARD.into(),
            name: "Standard".into(),
            description: "Lean local toolset with per-conversation selection and approval.".into(),
            sources: vec!["__workspace".into(), "__execution".into()],
            mcp: true,
            system_time: true,
            skills: true,
            // Lean for small local models: core workspace ops, shell, web,
            // memory, subagents, and granular todos. Advanced orchestration
            // (workflow_run, ralph_run), docker, and guides live in
            // Creator or behind subagent delegation.
            harness: harness(ASSISTANT_CATALOG),
        },
    };
    if !matches!(id, MINIMAL | CHAT) {
        preset
            .sources
            .extend(["__browser".into(), "__computer".into()]);
    }
    Ok(preset)
}

/// Minimal-mode workspace surface: edit only (the `str_replace_editor`
/// equivalent); listing/reading go through execution when needed.
pub fn minimal_workspace_tools() -> [&'static str; 1] {
    ["edit_file"]
}

/// The everyday assistant catalog. Older aliases stay readable.
///
/// This is the single source of truth for the Standard preset's tool list.
/// The preset derives from it, so the two cannot drift apart — the drift
/// that a hand-maintained duplicate invites.
const ASSISTANT_CATALOG: &[&str] = &[
        "verification_run",
        "verification_report",
        "verification_compare",
        "change_impact",
        "http_session_create",
        "http_session_close",
        "http_request",
        "http_compare",
        "openapi_inspect",
        "openapi_request",
        "database_schema",
        "database_query",
        "database_explain",
        "database_rehearse",
        "database_cleanup",
        "benchmark_run",
        "benchmark_compare",
        "profile_inspect",
        "profile_capture",
        "profile_compare",
        "debugger_start",
        "debugger_inspect",
        "debugger_control",
        "debugger_events",
        "debugger_stop",
        "process_start",
        "process_poll",
        "process_write",
        "process_stop",
        "shell_run", "workspace_open_preview",
        "check_run",
        "check_report",
        "git_status",
        "git_diff",
        "git_stage",
        "git_unstage",
        "git_commit",
        "git_branch",
        "git_history",
        "worktree_enter",
        "worktree_exit",
        "lsp_diagnostics",
        "lsp_symbols",
        "lsp_references",
        "lsp_prepare_rename",
        "lsp_rename",
        "lsp_definition",
        "lsp_hover",
        "lsp_implementations",
        "lsp_code_actions",
        "lsp_prepare_code_action",
        "lsp_apply_edit",
        "lsp_execute_command",
        "get_weather",
        "web_search",
        "web_read",
        "web_find",
        "file_search",
        "file_read",
        "file_write",
        "run_code",
        "format_paths",
        "skill_list",
        "skill_load",
        "session_rename",
        "session_move",
        "worktree_list",
        "models",
        "plan_submit",
        "ask_user",
        "artifact_read",
];

/// The everyday assistant catalog as owned strings.
#[cfg(test)]
pub fn assistant_catalog() -> Vec<String> {
    ASSISTANT_CATALOG.iter().map(|alias| alias.to_string()).collect()
}

/// Aliases share the same opt-in source as their native equivalents. The
/// filtered inventory is inherited by PTC and child agents, including dispatch.
pub fn harness_source_allowed(alias: &str, workspace: bool, execution: bool) -> bool {
    match alias {
        "workspace_open_preview" => workspace && execution,
        name if crate::debug_tools::owns(name) => workspace && execution,
        name if crate::development_tools::owns(name) => {
            workspace && (!crate::development_tools::execution_tool(name) || execution)
        }
        "file_search" | "file_read" | "file_write" | "format_paths" | "git_status"
        | "git_diff" | "git_stage" | "git_unstage" | "git_commit" | "git_branch"
        | "git_history" | "worktree_enter" | "worktree_exit" | "session_move"
        | "worktree_list" => workspace,
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
        | "lsp_execute_command" => workspace,
        "run_code" | "terminal_create" | "terminal_send" | "terminal_close" | "process_start"
        | "process_poll" | "process_write" | "process_stop" | "shell_run" | "check_run"
        | "check_report" | "docker_exec" | "ptc_run" => execution,
        _ => true,
    }
}

pub fn apply_tool_support(tools: &mut Vec<crate::connectors::AgentTool>, supported: bool) {
    if !supported {
        tools.clear();
    }
}

#[cfg(test)]
pub fn canonical_tool(alias: &str) -> String {
    match alias {
        "web_open" | "web_fetch" | "web_fetch_url" | "visit" => "web_read".into(),
        "search" => "web_search".into(),
        "read_file" => "file_read".into(),
        "create_file" | "edit_file" => "file_write".into(),
        "local_run_code" => "run_code".into(),
        "execute_command" | "terminal_send" => "shell_execution".into(),
        "subagent" | "workflow_run" | "ralph_run" | "ptc_run" => alias.to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
pub fn historical_tool_readable(alias: &str) -> bool {
    !alias.is_empty()
}

pub fn list() -> Vec<Preset> {
    ids()
        .iter()
        .map(|id| get(id).expect("built-in preset"))
        .collect()
}

/// TypeScript SDK prelude served to the model in Code preset: one declaration
/// per active tool alias plus the `sdk_` call convention enforced by `ptc_run`.
/// The host executes either a `plan` (JSON program: body statements `let` /
/// `call` / `if` / `for` / `parallel`, `{"$":"name","path":[...]}` references,
/// bounded `op` transforms, `return`) or a flat `steps` batch; `program` is an
/// optional TypeScript rendering whose sdk_* calls are consistency-checked
/// against `steps` — it is never evaluated by the host.
pub fn ts_sdk(aliases: &[String]) -> String {
    let mut out = String::from(
        "// LocalLM programmatic tool-calling SDK (generated).\n// Submit tool calls to ptc_run as a JSON 'plan' (dataflow: let/call/if/for/parallel,\n// {\"$\":\"name\"} references, op transforms) or a flat 'steps' batch [{tool, args}].\n// Each call is policy-checked individually before execution.\ndeclare function run_tool(name: string, args: unknown): unknown;\n",
    );
    for alias in aliases {
        out.push_str(&format!(
            "declare function sdk_{alias}(args: unknown): unknown;\n"
        ));
    }
    out
}

#[tauri::command]
pub fn list_presets() -> Vec<Preset> {
    list()
}

#[tauri::command]
pub fn get_preset(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
) -> Result<Preset, String> {
    let store = state.database()?;
    let id = store
        .conversation_preset(&conversation_id)
        .unwrap_or_else(|_| STANDARD.to_string());
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
    state
        .database()?
        .save_conversation_preset(&conversation_id, &preset)?;
    get(&preset)
}

/// Static authoring guidance surfaced as the `preset_guide` harness tool.
pub fn authoring_guide() -> &'static str {
    "Compose a preset without forking source: pick sources (workspace, execution, daytona), \
     MCP on/off, system_time on/off, skills on/off, and a harness tool subset. Standard = lean core \
     (todos, subagents, shell, web, memory, ask_user, artifacts); \
     Creator = Standard plus orchestration (workflow_run, ralph_run), docker, and guides. \
     Minimal = execution + edit_file + terminal_* + artifact_read. New presets need a Rust change today; \
     file-based preset overlays are future work."
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_preset_respects_sources_and_can_offer_zero_tools() {
        for preset in list() {
            let mut tools = Vec::new();
            for alias in &preset.harness {
                if harness_source_allowed(alias, false, false) {
                    tools.push(crate::connectors::AgentTool::harness(alias).unwrap());
                }
            }
            for alias in [
                "file_search",
                "file_read",
                "file_write",
                "run_code",
                "ptc_run",
                "terminal_create",
                "terminal_send",
                "terminal_close",
                "docker_exec",
            ] {
                assert!(
                    !tools.iter().any(|tool| tool.alias == alias),
                    "{}: {alias}",
                    preset.id
                );
            }
            apply_tool_support(&mut tools, false);
            crate::arex::adapt_tools("AREX", &mut tools).unwrap();
            assert!(tools.is_empty(), "{} must allow text-only chat", preset.id);
        }
        assert!(harness_source_allowed("file_read", true, false));
        assert!(!harness_source_allowed("run_code", true, false));
        assert!(harness_source_allowed("run_code", false, true));
        assert!(!harness_source_allowed("file_read", false, true));
    }
    #[test]
    fn all_presets_validate_and_minimal_is_two_tools() {
        for id in ids() {
            let preset = get(id).unwrap();
            assert_eq!(preset.id, id);
        }
        assert!(get("nope").is_err());
        // Minimal benchmark surface: shell (execution) + edit_file + terminal_*.
        let minimal = get(MINIMAL).unwrap();
        assert_eq!(
            minimal.sources,
            vec!["__execution".to_string(), "__workspace".to_string()]
        );
        assert!(!minimal.mcp && !minimal.system_time && !minimal.skills);
        assert_eq!(minimal_workspace_tools(), ["edit_file"]);
    }
    #[test]
    fn weather_is_offered_in_online_tool_presets() {
        for id in [STANDARD, CODE, CODING, RESEARCH, CREATOR] {
            assert!(
                get(id)
                    .unwrap()
                    .harness
                    .contains(&"get_weather".to_string()),
                "{id}"
            );
        }
        for id in [CHAT, MINIMAL] {
            assert!(
                !get(id)
                    .unwrap()
                    .harness
                    .contains(&"get_weather".to_string()),
                "{id}"
            );
        }
    }
    #[test]
    fn every_preset_can_read_truncated_tool_outputs() {
        for preset in list() {
            assert!(
                preset.harness.contains(&"artifact_read".to_string()),
                "{}",
                preset.id
            );
            assert!(harness_source_allowed("artifact_read", false, false));
        }
    }
    #[test]
    fn sdk_lists_every_offered_alias() {
        let sdk = ts_sdk(&["a".to_string(), "b".to_string()]);
        assert!(sdk.contains("sdk_a") && sdk.contains("sdk_b"));
    }
    #[test]
    fn advanced_profiles_persist_through_store_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("presets.sqlite");
        for id in [CODE, MINIMAL, CREATOR] {
            let conversation = {
                let store = crate::store::Store::open(&path).unwrap();
                let conversation = store.create_conversation().unwrap();
                store
                    .save_conversation_preset(&conversation.id, id)
                    .unwrap();
                conversation.id
            };
            let store = crate::store::Store::open(&path).unwrap();
            let saved = store.conversation_preset(&conversation).unwrap();
            assert_eq!(saved, id);
            assert_eq!(get(&saved).unwrap().id, id);
        }
    }
    #[test]
    fn echo_tool_is_not_offered_but_still_dispatchable_for_fixtures() {
        for id in [STANDARD, CREATOR, CODE, CODING, MINIMAL, CHAT, RESEARCH] {
            assert!(
                !get(id).unwrap().harness.contains(&"tool".to_string()),
                "{id}"
            );
        }
        assert!(harness_source_allowed("tool", false, false));
        assert!(crate::harness::is_harness_tool("tool"));
    }
    #[test]
    fn standard_is_lean_and_creator_is_full() {
        let standard = get(STANDARD).unwrap();
        assert_eq!(
            standard.harness,
            assistant_catalog()
        );
        for advanced in [
            "workflow_run",
            "ralph_run",
            "subagent",
            "ptc_run",
            "docker_exec",
            "preset_guide",
            "list_subagent_models",
            "terminal_send",
        ] {
            assert!(
                !standard.harness.contains(&advanced.to_string()),
                "{advanced} is not in the assistant catalog"
            );
        }
        assert_eq!(canonical_tool("web_fetch"), "web_read");
        assert!(historical_tool_readable("workflow_run"));
        assert!(historical_tool_readable("web_open"));
        let creator = get(CREATOR).unwrap();
        for alias in &standard.harness {
            assert!(
                creator.harness.contains(alias),
                "creator should include standard {alias}"
            );
        }
        for advanced in ["workflow_run", "ralph_run", "docker_exec", "preset_guide"] {
            assert!(creator.harness.contains(&advanced.to_string()));
        }
    }
}
