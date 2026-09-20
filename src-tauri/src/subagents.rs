//! Subagents: spawn/fork, foreground/background, depth cap (Phase 3).
//!
//! A child runs the same bounded tool loop as a parent (rounds x calls) with
//! its own ephemeral history: the parent's history is never polluted. Every
//! child owns a `runs` row (checkpoint `subagent`) plus `subagent_runs`
//! lineage, so the Trajectory shows the whole team tree. Foreground children
//! reuse the caller's reconstructed local tools; background children run on a
//! supervisor task with their own SQLite connection and local-only tools.
//! Anything needing approval follows the child policy (reads-only pin by
//! default); denials become errored results, never silent skips.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

pub const MAX_DEPTH_KEY: &str = "subagents.max_depth";
pub const MODELS_KEY: &str = "subagents.models";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubagentRun {
    pub id: String,
    pub conversation_id: String,
    pub parent_run_id: String,
    pub child_run_id: String,
    pub depth: i64,
    pub status: String,
    pub label: String,
    pub prompt: String,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildPolicy {
    ReadsOnly,
    Full,
}

impl ChildPolicy {
    pub fn from_access(mode: crate::permissions::AccessMode) -> Self {
        match mode {
            crate::permissions::AccessMode::FullAccess => ChildPolicy::Full,
            _ => ChildPolicy::ReadsOnly,
        }
    }
    fn allows(&self, trusted_read: bool) -> bool {
        match self {
            ChildPolicy::Full => true,
            ChildPolicy::ReadsOnly => trusted_read,
        }
    }
    fn access_mode(&self) -> crate::permissions::AccessMode {
        match self {
            ChildPolicy::Full => crate::permissions::AccessMode::FullAccess,
            ChildPolicy::ReadsOnly => crate::permissions::AccessMode::Ask,
        }
    }
}

/// Harness tools the detached background driver can serve without &AppState.
/// Store-only / stateless tools run against a fresh SQLite connection;
/// everything else needs the foreground dispatcher (approval, terminals,
/// schedulers, orchestration) and fails loud instead of hanging.
pub fn harness_detached_supported(alias: &str) -> bool {
    matches!(alias,
        "todo_write" | "todo_add" | "todo_update"
        | "goal_set" | "goal_clear"
        | "memory_teach" | "memory_recall"
        | "file_search" | "list_agents" | "list_subagent_models"
        | "preset_guide" | "artifact_read" | "update_context" | "finish")
}

/// Serve the store-only harness subset for background children. Mirrors the
/// foreground `harness::execute` semantics for these aliases without approval
/// prompts (background runs are already pinned by the child policy).
pub fn harness_detached(
    snapshot: &BackendSnapshot,
    request: &ChildRequest,
    alias: &str,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    if !harness_detached_supported(alias) {
        return Err(format!(
            "Harness tool '{alias}' needs the foreground dispatcher (approval, terminals, or orchestration). Ask the parent for what you need instead."
        ));
    }
    let store = crate::store::Store::open(&snapshot.db_path)?;
    let workspace_path = snapshot.workspace_path.clone();
    match alias {
        "update_context" | "finish" => {
            crate::arex::validate(alias, &args)?;
            Ok(if alias == "finish" { serde_json::json!({"answer":crate::arex::finish_answer(&args)}) } else { serde_json::json!({"context":args["context"]}) })
        }
        "todo_write" => {
            let todos = crate::plans::parse_todo_write(&args)?;
            store.save_todos(&request.conversation_id, &todos)?;
            Ok(serde_json::json!({"saved": todos.len()}))
        }
        "todo_add" => {
            let text = args.get("text").and_then(|v| v.as_str())
                .ok_or("todo_add needs a 'text' string.")?.to_string();
            let todo = store.add_todo(&request.conversation_id, &text)?;
            Ok(serde_json::json!({"added": todo}))
        }
        "todo_update" => {
            let index = args.get("index").and_then(|v| v.as_u64())
                .ok_or("todo_update needs an 'index' integer.")? as usize;
            let status = args.get("status").and_then(|v| v.as_str()).map(str::to_string);
            let text = args.get("text").and_then(|v| v.as_str()).map(str::to_string);
            let todos = store.update_todo(&request.conversation_id, index, status.as_deref(), text.as_deref())?;
            Ok(serde_json::json!({"saved": todos.len()}))
        }
        "goal_set" => {
            let objective = args.get("objective").and_then(|v| v.as_str())
                .ok_or("goal_set needs an 'objective' string.")?.to_string();
            crate::plans::validate_objective(&objective)?;
            store.save_goal(&request.conversation_id, &objective)?;
            Ok(serde_json::json!({"goal": objective}))
        }
        "goal_clear" => {
            store.clear_goal(&request.conversation_id)?;
            Ok(serde_json::json!({"cleared": true}))
        }
        "memory_teach" => {
            let fact = args.get("fact").and_then(|v| v.as_str())
                .ok_or("memory_teach needs a 'fact' string.")?.to_string();
            let scope = args.get("scope").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let scope = if scope.trim().is_empty() {
                if workspace_path.is_empty() { "global".to_string() } else { workspace_path.clone() }
            } else { scope };
            let record = store.teach_fact(&scope, &fact, "model")?;
            Ok(serde_json::json!({"id": record.id, "scope": record.scope}))
        }
        "memory_recall" => {
            let scope = args.get("scope").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let scope = if scope.trim().is_empty() {
                if workspace_path.is_empty() { "global".to_string() } else { workspace_path.clone() }
            } else { scope };
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
            let facts = store.recall_facts(&scope, limit.min(50))?;
            Ok(serde_json::json!({"facts": facts}))
        }
        "file_search" => {
            let query = args.get("query").and_then(|v| v.as_str())
                .ok_or("file_search needs a 'query' string.")?.to_string();
            if workspace_path.is_empty() {
                return Err("Choose a workspace folder before file_search.".into());
            }
            let hits = crate::sandbox::file_search(std::path::Path::new(&workspace_path), &query)?;
            Ok(serde_json::json!({"hits": hits}))
        }
        "list_agents" => {
            let wanted = args.get("state").and_then(|v| v.as_str()).unwrap_or("all");
            if !["all", "running", "completed", "failed"].contains(&wanted) {
                return Err("list_agents state must be all, running, completed, or failed.".into());
            }
            let rows = store.subagent_runs_for_conversation(&request.conversation_id)?;
            let rows: Vec<&SubagentRun> =
                rows.iter().filter(|row| wanted == "all" || row.status == wanted).collect();
            Ok(serde_json::json!({"agents": rows}))
        }
        "list_subagent_models" => {
            let allowlist = list_models(&store)?;
            Ok(serde_json::json!({"models": allowlist, "note": "Empty allowlist means the current conversation model only."}))
        }
        "preset_guide" => {
            Ok(serde_json::json!({"guide": crate::presets::authoring_guide(), "presets": crate::presets::list()}))
        }
        "artifact_read" => {
            let id = args.get("id").and_then(|v| v.as_str())
                .ok_or("artifact_read needs an 'id' string.")?.to_string();
            let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(2000) as usize;
            let record = store.artifact(&id)?.ok_or("Unknown artifact id for this workspace.")?;
            if record.conversation_id != request.conversation_id {
                return Err("Unknown artifact id for this workspace.".into());
            }
            Ok(crate::artifacts::read_window(&record, offset, limit.min(20000)))
        }
        _ => Err(format!("Harness tool '{alias}' needs the foreground dispatcher.")),
    }
}

/// Progress/approval emitter shared by foreground children.
pub type ChildEmit = Arc<dyn Fn(String, Option<serde_json::Value>) + Send + Sync>;

/// Everything a background task needs: owned, Send + 'static.
#[derive(Clone, Debug)]
pub struct BackendSnapshot {
    pub kind: String,
    pub endpoint: String,
    pub key: String,
    pub base_url: String,
    pub model_id: String,
    pub context_length: u32,
    pub max_output_tokens: Option<u32>,
    pub supports_tools: bool,
    pub system_prompt: String,
    pub temperature: f64,
    pub top_p: f64,
    pub max_tokens: u32,
    pub workspace_path: String,
    pub execution_config: crate::execution::ExecutionConfig,
    pub active_skills: Vec<String>,
    pub db_path: std::path::PathBuf,
    pub data_dir: std::path::PathBuf,
    pub account_id: Option<String>,
    pub inference_slots: u32,
}

impl BackendSnapshot {
    pub fn backend(&self) -> Result<crate::inference::Backend, String> {
        if self.kind == "local" {
            crate::inference::Backend::local(self.endpoint.clone(), self.key.clone(), self.context_length)
        } else if self.kind == "subscription" {
            let model = crate::providers::RemoteModel {
                id: self.model_id.clone(),
                context_length: Some(self.context_length),
                max_output_tokens: self.max_output_tokens,
                supports_images: false, tool_support: if self.supports_tools {
                    crate::providers::ToolSupport::Supported
                } else {
                    crate::providers::ToolSupport::Unsupported
                },
            };
            crate::inference::Backend::subscription(
                "subscription",
                &self.base_url,
                self.key.clone(),
                self.account_id.clone(),
                &model,
            )
        } else {
            let provider = crate::providers::ProviderConnection {
                id: "snapshot".into(),
                name: "snapshot".into(),
                api_format: crate::providers::OPENAI_CHAT_COMPLETIONS.into(),
                base_url: self.base_url.clone(),
                verified: true,
                last_tested_at: None,
                models: vec![],
                has_api_key: true,
            };
            let model = crate::providers::RemoteModel {
                id: self.model_id.clone(),
                context_length: Some(self.context_length),
                max_output_tokens: self.max_output_tokens,
                supports_images: false, tool_support: if self.supports_tools {
                    crate::providers::ToolSupport::Supported
                } else {
                    crate::providers::ToolSupport::Unsupported
                },
            };
            crate::inference::Backend::openai(&provider, self.key.clone(), &model)
        }
    }
}

/// Capture the current conversation's backend + tooling inputs for children.
pub async fn snapshot_for_conversation(
    state: &crate::AppState,
    conversation_id: &str,
) -> Result<BackendSnapshot, String> {
    let (selection, preferences, workspace_path, execution_config, active_skills, remote_info) = {
        let store = state.database()?;
        let (selection, _) = store.conversation_model(conversation_id)?;
        let preferences = store.preferences()?;
        preferences.validate()?;
        let workspace_path = store.workspace_path().unwrap_or_default();
        let execution_config = store.execution_config().unwrap_or(crate::execution::ExecutionConfig::default());
        let active_skills = store.active_skills().unwrap_or_default();
        let remote_info = if let Some(ref provider_id) = selection.provider_id {
            let provider = store.provider(provider_id)?.ok_or("The selected provider was deleted.")?;
            let model = provider
                .models
                .iter()
                .find(|model| model.id == selection.model_id)
                .ok_or("Configure the selected remote model's context capacity.")?
                .clone();
            Some((provider, model))
        } else {
            None
        };
        (selection, preferences, workspace_path, execution_config, active_skills, remote_info)
    };
    let mut snapshot = BackendSnapshot {
        kind: "local".into(),
        endpoint: String::new(),
        key: String::new(),
        account_id: None,
        base_url: String::new(),
        model_id: selection.model_id.clone(),
        context_length: 0,
        max_output_tokens: None,
        supports_tools: true,
        system_prompt: preferences.system_prompt,
        temperature: preferences.temperature,
        top_p: preferences.top_p,
        max_tokens: preferences.max_tokens,
        workspace_path,
        execution_config,
        active_skills,
        db_path: state.db_path.clone(),
        data_dir: state.data_dir.clone(),
        inference_slots: state.database().ok().and_then(|store| store.runtime_config().ok()).map(|config| config.inference_slots).unwrap_or(1),
    };
    if let (Some(provider_id), Some((provider, model))) = (selection.provider_id, remote_info) {
        if crate::providers::is_subscription_format(&provider.api_format) {
            let kind = if provider.api_format == crate::providers::CHATGPT_SUBSCRIPTION || provider.id == "chatgpt-subscription" {
                "chatgpt"
            } else {
                "grok"
            };
            let token = crate::subscription_auth::get_valid_access_token(&state.daytona_vault, kind).await?;
            let account_id = crate::subscription_auth::load_session(&state.daytona_vault, kind)?
                .and_then(|s| s.account_id);
            snapshot.kind = "subscription".into();
            snapshot.key = token;
            snapshot.account_id = account_id;
            snapshot.base_url = provider.base_url;
            snapshot.context_length = model.context_length.ok_or("Configure the selected remote model's context capacity.")?;
            snapshot.max_output_tokens = model.max_output_tokens;
            snapshot.supports_tools = !matches!(model.tool_support, crate::providers::ToolSupport::Unsupported);
        } else {
            let key = state
                .daytona_vault
                .load(&crate::providers::credential_id(&provider_id))?
                .ok_or("Save an API key for the selected provider.")?;
            snapshot.kind = "remote".into();
            snapshot.key = String::from_utf8(key).map_err(|_| "Saved provider API key is invalid.")?;
            snapshot.base_url = provider.base_url;
            snapshot.context_length = model.context_length.ok_or("Configure the selected remote model's context capacity.")?;
            snapshot.max_output_tokens = model.max_output_tokens;
            snapshot.supports_tools = !matches!(model.tool_support, crate::providers::ToolSupport::Unsupported);
        }
    } else {
        let mut runtime = state.runtime.lock().await;
        let inspected = runtime.inspect();
        if inspected.phase != "ready" {
            return Err("Load a local model before spawning subagents.".into());
        }
        snapshot.endpoint = runtime.endpoint.clone();
        snapshot.key = runtime.api_key.clone();
        snapshot.context_length = runtime.context_length;
    }
    Ok(snapshot)
}

pub fn max_depth(store: &crate::store::Store) -> i64 {
    let configured: i64 = store.setting(MAX_DEPTH_KEY).unwrap_or(3);
    configured.clamp(0, 8)
}

pub struct SubagentRegistry {
    flags: Mutex<HashMap<String, Arc<AtomicBool>>>,
    mailbox: Mutex<HashMap<String, Vec<String>>>,
}

impl SubagentRegistry {
    pub fn new() -> Self {
        Self { flags: Mutex::new(HashMap::new()), mailbox: Mutex::new(HashMap::new()) }
    }
    pub fn register(&self, child_run_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.flags.lock().expect("subagent registry").insert(child_run_id.to_string(), flag.clone());
        flag
    }
    pub fn interrupt(&self, child_run_id: &str) -> bool {
        if let Some(flag) = self.flags.lock().expect("subagent registry").get(child_run_id) {
            flag.store(true, Ordering::SeqCst);
            return true;
        }
        false
    }
    pub fn take_mail(&self, child_run_id: &str) -> Vec<String> {
        self.mailbox.lock().expect("subagent registry").remove(child_run_id).unwrap_or_default()
    }
    pub fn deliver(&self, child_run_id: &str, message: &str) -> bool {
        let live = self.flags.lock().expect("subagent registry").contains_key(child_run_id);
        if !live {
            return false;
        }
        self.mailbox
            .lock()
            .expect("subagent registry")
            .entry(child_run_id.to_string())
            .or_default()
            .push(message.to_string());
        true
    }
    pub fn retire(&self, child_run_id: &str) {
        self.flags.lock().expect("subagent registry").remove(child_run_id);
        self.mailbox.lock().expect("subagent registry").remove(child_run_id);
    }
}

impl Default for SubagentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Process-wide registry so background supervisor tasks (which own no
/// AppState) share interrupt/mailbox state with IPC handlers. The AppState
/// holds a clone of this same registry.
pub fn global_registry() -> Arc<SubagentRegistry> {
    static REGISTRY: std::sync::OnceLock<Arc<SubagentRegistry>> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| Arc::new(SubagentRegistry::new())).clone()
}

#[derive(Clone, Debug)]
pub struct ChildRequest {
    pub label: String,
    pub prompt: String,
    /// Depth of the PARENT run (0 = top-level turn).
    pub depth: i64,
    pub max_rounds: usize,
    pub tool_filter: Option<Vec<String>>,
    pub persona: Option<String>,
    pub output_schema: Option<serde_json::Value>,
    pub policy: ChildPolicy,
    pub parent_run_id: String,
    pub conversation_id: String,
}

pub fn validate_child_request(request: &ChildRequest) -> Result<(), String> {
    if request.label.trim().is_empty() || request.label.len() > 80 {
        return Err("Subagent label must be 1-80 characters.".into());
    }
    if request.prompt.trim().is_empty() || request.prompt.len() > 4000 {
        return Err("Subagent prompt must be 1-4000 characters.".into());
    }
    if request.max_rounds == 0 || request.max_rounds > 8 {
        return Err("Subagent max_rounds must be 1-8.".into());
    }
    if let Some(persona) = &request.persona {
        if persona.len() > 500 {
            return Err("Subagent persona is limited to 500 characters.".into());
        }
    }
    if let Some(schema) = &request.output_schema {
        if !schema.is_object() || schema.to_string().len() > 4096 {
            return Err("outputSchema must be a JSON object under 4 KiB.".into());
        }
    }
    if let Some(filter) = &request.tool_filter {
        if filter.len() > 16 {
            return Err("toolFilter holds at most 16 entries.".into());
        }
    }
    Ok(())
}

/// Group/alias matching for toolFilter entries.
pub fn tool_allowed(filter: &Option<Vec<String>>, tool: &crate::connectors::AgentTool) -> bool {
    let Some(filter) = filter else {
        return true;
    };
    filter.iter().any(|entry| {
        entry == &tool.alias
            || (entry == "workspace" && tool.connector == "Workspace")
            || (entry == "execution" && tool.connector == "Local execution")
            || (entry == "mcp" && tool.is_mcp())
            || (entry == "harness" && tool.is_harness())
            || (entry == "skills" && tool.connector == "Skills")
            || (entry == "system" && tool.connector == "System")
            || (entry == "daytona" && tool.connector == "Daytona")
    })
}

fn child_system_prompt(snapshot: &BackendSnapshot, request: &ChildRequest) -> String {
    let mut prompt = snapshot.system_prompt.clone();
    if let Some(persona) = &request.persona {
        prompt.push_str(&format!("\nAdopt this persona for the subtask: {persona}"));
    }
    if let Some(schema) = &request.output_schema {
        prompt.push_str(&format!(
            "\nReturn ONLY a JSON object matching this schema (no prose, no fences): {schema}. \
             Required keys from the schema must all be present."
        ));
    }
    prompt.push_str("\nYou are a subagent: solve ONLY the assigned subtask and report back concisely.");
    prompt
}

fn check_output_schema(schema: &serde_json::Value, answer: &str) -> Result<(), String> {
    let value: serde_json::Value =
        serde_json::from_str(answer.trim()).map_err(|_| "The subagent did not return valid JSON for the requested outputSchema.".to_string())?;
    if !value.is_object() {
        return Err("The subagent must return a JSON object for the requested outputSchema.".into());
    }
    if let Some(required) = schema.get("required").and_then(|value| value.as_array()) {
        for key in required.iter().filter_map(|key| key.as_str()) {
            if value.get(key).is_none() {
                return Err(format!("Subagent output is missing required key '{key}'."));
            }
        }
    }
    Ok(())
}

#[allow(clippy::large_enum_variant)]
pub enum ChildTool<'a> {
    Borrowed(&'a crate::connectors::AgentTool),
    Owned(crate::connectors::AgentTool),
}

impl<'a> ChildTool<'a> {
    fn tool(&self) -> &crate::connectors::AgentTool {
        match self {
            ChildTool::Borrowed(tool) => tool,
            ChildTool::Owned(tool) => tool,
        }
    }
}

/// Reconstruct the local-only tool subset from a snapshot (workspace +
/// execution + clock + skill reader). MCP peers and Daytona stay with the
/// foreground task; background children document this limit in their result.
pub fn local_tools(snapshot: &BackendSnapshot) -> Vec<crate::connectors::AgentTool> {
    let mut tools = Vec::new();
    if !snapshot.workspace_path.is_empty() {
        if let Ok(workspace) = crate::workspace::Workspace::open(&snapshot.workspace_path) {
            tools.extend(std::sync::Arc::new(workspace).tools());
        }
        if let Ok(execution) = crate::execution::LocalExecution::new(snapshot.execution_config.clone(), &snapshot.workspace_path) {
            tools.push(std::sync::Arc::new(execution).tool());
        }
    }
    if !snapshot.active_skills.is_empty() {
        let skills = crate::skills::Skills::new(snapshot.data_dir.join("skills"));
        if let Ok(Some(reader)) = skills.reader(&snapshot.active_skills) {
            tools.push(reader);
        }
    }
    tools.push(crate::connectors::AgentTool::system_time());
    tools
}

/// Drive the bounded child loop. Shared by foreground and background runs.
#[allow(clippy::too_many_arguments)]
pub async fn drive_child(
    state: &crate::AppState,
    snapshot: &BackendSnapshot,
    request: &ChildRequest,
    tools: Vec<ChildTool<'_>>,
    backend: &crate::inference::Backend,
    interrupt: Arc<AtomicBool>,
    registry: Option<Arc<SubagentRegistry>>,
    child_run_id: &str,
    emit: Option<ChildEmit>,
) -> Result<String, String> {
    use crate::inference::InferenceProvider;
    validate_child_request(request)?;
    if interrupt.load(Ordering::SeqCst) {
        return Err("Subagent was interrupted before starting.".into());
    }
    let tools: Vec<&crate::connectors::AgentTool> = tools.iter().map(|tool| tool.tool()).collect();
    let system = child_system_prompt(snapshot, request);
    let mut messages = vec![
        serde_json::json!({"role": "system", "content": system}),
        serde_json::json!({"role": "user", "content": request.prompt}),
    ];
    let mut preferences = crate::store::Preferences {
        system_prompt: snapshot.system_prompt.clone(),
        temperature: snapshot.temperature,
        top_p: snapshot.top_p,
        max_tokens: snapshot.max_tokens,
        ..Default::default()
    };
    preferences.max_tokens =
        crate::context::fit_response_budget(preferences.max_tokens, backend.context_capacity());
    preferences.validate()?;
    backend.validate_response_tokens(preferences.max_tokens)?;
    let mut answer = String::new();
    let max_rounds = request.max_rounds.min(8);
    // Monotonic per-run sequence: (run_id, seq) is the primary key.
    let mut event_seq: u64 = 1;
    for round in 0..max_rounds {
        if interrupt.load(Ordering::SeqCst) || state.cancel_requested() {
            if let Ok(store) = state.database() {
                let _ = store.update_run_status(child_run_id, crate::agent_run::RunState::Cancelled, Some("Interrupted."), None);
            }
            return Err("Subagent was interrupted.".into());
        }
        // Mailbox: followup messages from send_message continue the loop.
        if let Some(registry) = &registry {
            for note in registry.take_mail(child_run_id) {
                messages.push(serde_json::json!({"role": "user", "content": format!("[parent followup] {note}")}));
            }
        }
        let mut payload = serde_json::json!({
            "messages": messages, "temperature": preferences.temperature,
            "top_p": preferences.top_p, "max_tokens": preferences.max_tokens,
            "stream": true, "cache_prompt": true,
        });
        if snapshot.inference_slots >= 2 {
            payload["id_slot"] = serde_json::json!(1);
        }
        if backend.supports_tools() && !tools.is_empty() {
            payload["tools"] = serde_json::json!(tools.iter().map(|tool| tool.definition()).collect::<Vec<_>>());
        }
        let payload = backend.payload(&payload);
        let _tokens = backend.check_context(&payload, preferences.max_tokens).await?;
        let response = tokio::time::timeout(Duration::from_secs(600), backend.stream(&payload))
            .await
            .map_err(|_| "Subagent model did not respond within ten minutes.")??;
        let mut stream = response.bytes_stream();
        let mut decoder = crate::sse::SseDecoder::default();
        let mut calls = crate::tool_calls::ToolCalls::default();
        let mut round_answer = String::new();
        let mut finish_reason = String::new();
        'stream: loop {
            let next = tokio::time::timeout(Duration::from_secs(300), futures_util::StreamExt::next(&mut stream))
                .await
                .map_err(|_| "Subagent model stopped responding.")?;
            let Some(bytes) = next else {
                decoder.finish()?;
                return Err("Subagent stream ended without a completion marker.".into());
            };
            for event in decoder.push(&bytes.map_err(|error| error.to_string())?)? {
                if event == "[DONE]" {
                    break 'stream;
                }
                let value: serde_json::Value =
                    serde_json::from_str(&event).map_err(|error| format!("Invalid subagent stream: {error}"))?;
                if let Some(error) = value.get("error") {
                    return Err(backend.stream_error(error));
                }
                if let Some(reason) = value["choices"][0]["finish_reason"].as_str() {
                    finish_reason = reason.to_string();
                }
                let delta = &value["choices"][0]["delta"];
                calls.push(delta)?;
                let text = delta["content"].as_str().unwrap_or("");
                answer.push_str(text);
                round_answer.push_str(text);
                if answer.len() > 1_048_576 {
                    return Err("Subagent output exceeded 1 MiB.".into());
                }
            }
        }
        if finish_reason == "length" {
            return Err("Subagent hit the response token limit.".into());
        }
        let calls = calls.finish()?;
        crate::arex::validate_batch(&calls)?;
        if calls.is_empty() {
            break;
        }
        if round + 1 >= max_rounds && !calls.iter().any(|c| c.name == "finish") {
            return Err("Subagent round limit reached.".into());
        }
        messages.push(serde_json::json!({"role": "assistant", "content": round_answer,
            "tool_calls": calls.iter().map(|call| call.model_value()).collect::<Vec<_>>()}));
        for call in calls {
            if interrupt.load(Ordering::SeqCst) {
                return Err("Subagent was interrupted.".into());
            }
            let tool = tools.iter().find(|tool| tool.alias == call.name);
            let Some(tool) = tool else {
                messages.push(serde_json::json!({"role": "tool", "tool_call_id": call.id,
                    "content": "Unknown tool for this subagent. Use only the offered tools."}));
                continue;
            };
            let allowed = request.policy.allows(tool.trusted_read());
            let result = if !allowed && !matches!(request.policy, ChildPolicy::Full) && emit.is_some() {
                match request_approval_for_child(state, request, child_run_id, emit.as_ref(), &call).await {
                    Some(true) => call_child_tool(state, snapshot, request, &tools, child_run_id, round, tool, &call).await,
                    _ => serde_json::json!({"isError": true, "message": "Denied by the subagent approval pin (reads-only). Ask the parent for what you need instead."}),
                }
            } else if !allowed {
                serde_json::json!({"isError": true, "message": "Denied by the subagent approval pin (reads-only). Ask the parent for what you need instead."})
            } else {
                call_child_tool(state, snapshot, request, &tools, child_run_id, round, tool, &call).await
            };
            let control = if tool.harness_name().is_some() && matches!(call.name.as_str(), "finish" | "update_context") { Some(result.clone()) } else { None };
            let (bounded, maybe_artifact) = crate::artifacts::bound_tool_result(
                result, &tool.tool.name, &request.conversation_id, Some(child_run_id),
                crate::artifacts::DEFAULT_MAX_RESULT_CHARS,
            );
            if let Some(artifact) = &maybe_artifact {
                if let Ok(store) = state.database() {
                    let _ = store.save_artifact(artifact);
                }
            }
            if let Ok(store) = state.database() {
                event_seq += 1;
                let _ = store.append_run_event(&crate::agent_run::RunEvent {
                    run_id: child_run_id.to_string(),
                    seq: event_seq,
                    step_id: format!("round-{round}"),
                    tool_call_id: Some(call.id.clone()),
                    event_type: "tool_result".into(),
                    payload: serde_json::json!({"tool": tool.alias, "result": bounded}),
                    created_at: crate::store::now(),
                });
            }
            messages.push(serde_json::json!({"role": "tool", "tool_call_id": call.id, "content": bounded.to_string()}));
            if let Some(control) = control {
                if let Some(final_answer) = crate::arex::apply_control(&call.name, &control, &*state.database()?, &request.conversation_id, child_run_id, &mut messages)? {
                    if let Some(schema) = &request.output_schema { check_output_schema(schema, &final_answer)?; }
                    return Ok(final_answer);
                }
            }
        }
    }
    if let Some(schema) = &request.output_schema {
        check_output_schema(schema, &answer)?;
    }
    Ok(answer)
}

/// Execute one child tool call. Harness-backend tools run through the
/// foreground dispatcher (approval + audit); every other backend calls direct.
#[allow(clippy::too_many_arguments)]
async fn call_child_tool(
    state: &crate::AppState,
    snapshot: &BackendSnapshot,
    request: &ChildRequest,
    tools: &[&crate::connectors::AgentTool],
    child_run_id: &str,
    round: usize,
    tool: &crate::connectors::AgentTool,
    call: &crate::tool_calls::ToolCall,
) -> serde_json::Value {
    if let Some(alias) = tool.harness_name() {
        let emit: Arc<dyn Fn(String, Option<serde_json::Value>) + Send + Sync> =
            Arc::new(|_, _| {});
        // Rebuild owned AgentTools: HarnessCtx needs &[AgentTool], and AgentTool
        // is Clone (MCP peers become detached markers that fail loud on call).
        let owned: Vec<crate::connectors::AgentTool> = tools.iter().map(|tool| (*tool).clone()).collect();
        let ctx = crate::harness::HarnessCtx {
            state,
            snapshot: snapshot.clone(),
            conversation_id: request.conversation_id.clone(),
            run_id: child_run_id.to_string(),
            step_id: format!("round-{round}"),
            access_mode: request.policy.access_mode(),
            depth: request.depth + 1,
            preset_id: "standard".into(),
            inherit_tools: &owned,
            emit,
        };
        match tokio::time::timeout(tool.timeout(), Box::pin(crate::harness::execute(&ctx, alias, call.arguments.clone()))).await {
            Ok(Ok(outcome)) => {
                if let Ok(store) = state.database() {
                    for event in outcome.events {
                        let _ = store.append_run_event(&crate::agent_run::RunEvent {
                            run_id: child_run_id.to_string(),
                            seq: 0,
                            step_id: format!("round-{round}"),
                            tool_call_id: Some(call.id.clone()),
                            event_type: event.event_type,
                            payload: event.payload,
                            created_at: crate::store::now(),
                        });
                    }
                }
                outcome.value
            }
            Ok(Err(error)) => serde_json::json!({"isError": true, "message": error}),
            Err(_) => serde_json::json!({"isError": true, "message": "Subagent tool call timed out."}),
        }
    } else {
        match tokio::time::timeout(tool.timeout(), tool.call(call.arguments.clone())).await {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => serde_json::json!({"isError": true, "message": error}),
            Err(_) => serde_json::json!({"isError": true, "message": "Subagent tool call timed out."}),
        }
    }
}

/// Forward a foreground child's write request to the user's approval modal.
/// Returns Some(allow) on decision, None on timeout/withdrawal. The payload
/// carries `subagentLabel` so the UI can badge the delegating subagent.
async fn request_approval_for_child(
    state: &crate::AppState,
    request: &ChildRequest,
    child_run_id: &str,
    emit: Option<&ChildEmit>,
    call: &crate::tool_calls::ToolCall,
) -> Option<bool> {
    let (approval_id, decision) = state.approvals.request().ok()?;
    if let Some(emit) = emit {
        emit(
            format!("Awaiting approval: {} (subagent {})", call.name, request.label),
            Some(serde_json::json!({"id": approval_id, "connector": "Subagent",
                "name": call.name, "arguments": call.arguments,
                "subagentLabel": request.label, "childRunId": child_run_id})),
        );
    }
    let allow = tokio::time::timeout(std::time::Duration::from_secs(600), decision)
        .await
        .ok()
        .and_then(|result| result.ok());
    state.approvals.remove(&approval_id);
    if let Some(emit) = emit {
        emit("Approval decision recorded".into(), Some(serde_json::Value::Null));
    }
    allow
}

use std::time::Duration;

/// Foreground child: runs inline with the caller's reconstructed local tools
/// plus any inherited tools the filter admits. Returns the final answer.
pub async fn run_child_inline(
    state: &crate::AppState,
    snapshot: &BackendSnapshot,
    request: ChildRequest,
    inherit: &[crate::connectors::AgentTool],
    emit: Option<ChildEmit>,
) -> Result<(String, String), String> {
    validate_child_request(&request)?;
    let backend = snapshot.backend()?;
    let child_run_id = format!("child-{}", uuid::Uuid::new_v4());
    let now = crate::store::now();
    let record = SubagentRun {
        id: format!("sub-{}", uuid::Uuid::new_v4()),
        conversation_id: request.conversation_id.clone(),
        parent_run_id: request.parent_run_id.clone(),
        child_run_id: child_run_id.clone(),
        depth: request.depth + 1,
        status: "running".into(),
        label: request.label.clone(),
        prompt: request.prompt.clone(),
        error: None,
        created_at: now,
        updated_at: now,
    };
    {
        let store = state.database()?;
        store.save_run(&crate::agent_run::RunRecord {
            id: child_run_id.clone(),
            conversation_id: request.conversation_id.clone(),
            status: crate::agent_run::RunState::Preparing,
            model_provider: None,
            model_id: Some(snapshot.model_id.clone()),
            checkpoint: Some("subagent".into()),
            error: None,
            created_at: now,
            updated_at: now,
        })?;
        store.insert_subagent_run(&record)?;
        let _ = store.append_run_event(&crate::agent_run::RunEvent {
            run_id: child_run_id.clone(),
            seq: 0,
            step_id: "start".into(),
            tool_call_id: None,
            event_type: "subagent_start".into(),
            payload: serde_json::json!({"parentRun": request.parent_run_id, "depth": record.depth, "label": request.label}),
            created_at: now,
        });
    }
    let mut owned = local_tools(snapshot);
    let mut tools: Vec<ChildTool> = owned
        .drain(..)
        .map(ChildTool::Owned)
        .collect();
    tools.extend(inherit.iter().map(ChildTool::Borrowed));
    let tools: Vec<ChildTool> = tools
        .into_iter()
        .filter(|tool| tool_allowed(&request.tool_filter, tool.tool()))
        .collect();
    let interrupt = state.subagents.register(&child_run_id);
    let answer = drive_child(state, snapshot, &request, tools, &backend, interrupt, Some(state.subagents.clone()), &child_run_id, emit).await;
    state.subagents.retire(&child_run_id);
    match answer {
        Ok(answer) => {
            if let Ok(store) = state.database() {
                let _ = store.update_subagent_run(&record.id, "completed", None);
                let _ = store.update_run_status(&child_run_id, crate::agent_run::RunState::Completed, None, None);
            }
            Ok((child_run_id, answer))
        }
        Err(error) => {
            if let Ok(store) = state.database() {
                let _ = store.update_subagent_run(&record.id, "failed", Some(&error));
                let _ = store.update_run_status(&child_run_id, crate::agent_run::RunState::Failed, Some(&error), None);
            }
            Err(error)
        }
    }
}

/// Background child: supervisor task with its own SQLite connection and
/// local-only tools. Settles durably; the parent is notified via a tool-role
/// message plus the subagent_runs row.
pub fn spawn_background(snapshot: BackendSnapshot, request: ChildRequest) -> Result<String, String> {
    validate_child_request(&request)?;
    let child_run_id = format!("child-{}", uuid::Uuid::new_v4());
    let record_id = format!("sub-{}", uuid::Uuid::new_v4());
    let store = crate::store::Store::open(&snapshot.db_path)?;
    let now = crate::store::now();
    store.save_run(&crate::agent_run::RunRecord {
        id: child_run_id.clone(),
        conversation_id: request.conversation_id.clone(),
        status: crate::agent_run::RunState::Preparing,
        model_provider: None,
        model_id: Some(snapshot.model_id.clone()),
        checkpoint: Some("subagent".into()),
        error: None,
        created_at: now,
        updated_at: now,
    })?;
    store.insert_subagent_run(&SubagentRun {
        id: record_id,
        conversation_id: request.conversation_id.clone(),
        parent_run_id: request.parent_run_id.clone(),
        child_run_id: child_run_id.clone(),
        depth: request.depth + 1,
        status: "running".into(),
        label: request.label.clone(),
        prompt: request.prompt.clone(),
        error: None,
        created_at: now,
        updated_at: now,
    })?;
    let registry = global_registry();
    let interrupt = registry.register(&child_run_id);
    let task_child = child_run_id.clone();
    let task_registry = registry.clone();
    // Background tasks need an AppState for cancellation checks only; build a
    // minimal handle-free path by reusing drive pieces without &AppState.
    tauri::async_runtime::spawn(async move {
        let outcome = run_background_core(snapshot, request, interrupt, task_registry.clone(), task_child.clone()).await;
        task_registry.retire(&task_child);
        let _ = outcome;
    });
    Ok(child_run_id)
}

async fn run_background_core(
    snapshot: BackendSnapshot,
    request: ChildRequest,
    interrupt: Arc<AtomicBool>,
    registry: Arc<SubagentRegistry>,
    child_run_id: String,
) -> Result<(), String> {
    let store = Arc::new(Mutex::new(crate::store::Store::open(&snapshot.db_path)?));
    let backend = snapshot.backend()?;
    let owned = local_tools(&snapshot);
    let tools: Vec<&crate::connectors::AgentTool> = owned
        .iter()
        .filter(|tool| tool_allowed(&request.tool_filter, tool))
        .collect();
    // Drive without &AppState: cancellation comes from the interrupt flag.
    let answer = drive_detached(&snapshot, &request, tools, &backend, interrupt, Some(registry), &child_run_id, store.clone()).await;
    let record = store
        .lock()
        .map_err(|e| e.to_string())?
        .subagent_runs_for_conversation(&request.conversation_id)?
        .into_iter()
        .find(|row| row.child_run_id == child_run_id);
    match answer {
        Ok(answer) => {
            if let Ok(s) = store.lock() {
                if let Some(row) = record {
                    let _ = s.update_subagent_run(&row.id, "completed", None);
                }
                let _ = s.update_run_status(&child_run_id, crate::agent_run::RunState::Completed, None, None);
                let notice = serde_json::json!({
                    "request": {"connector": "Subagents", "name": "subagent", "decision": "allowed", "authorization": "background child settled"},
                    "result": {"childRunId": child_run_id, "status": "completed", "summary": answer.chars().take(1500).collect::<String>()},
                });
                let _ = s.append_message(&request.conversation_id, "tool", &notice.to_string(), "complete");
            }
        }
        Err(error) => {
            if let Ok(s) = store.lock() {
                if let Some(row) = record {
                    let _ = s.update_subagent_run(&row.id, "failed", Some(&error));
                }
                let _ = s.update_run_status(&child_run_id, crate::agent_run::RunState::Failed, Some(&error), None);
                let notice = serde_json::json!({
                    "request": {"connector": "Subagents", "name": "subagent", "decision": "allowed", "authorization": "background child settled"},
                    "result": {"childRunId": child_run_id, "status": "failed", "error": error},
                });
                let _ = s.append_message(&request.conversation_id, "tool", &notice.to_string(), "complete");
            }
        }
    }
    Ok(())
}

/// Detached driver: same bounds as drive_child without &AppState.
#[allow(clippy::too_many_arguments)]
pub async fn drive_detached(
    snapshot: &BackendSnapshot,
    request: &ChildRequest,
    tools: Vec<&crate::connectors::AgentTool>,
    backend: &crate::inference::Backend,
    interrupt: Arc<AtomicBool>,
    registry: Option<Arc<SubagentRegistry>>,
    child_run_id: &str,
    store: Arc<Mutex<crate::store::Store>>,
) -> Result<String, String> {
    use crate::inference::InferenceProvider;
    validate_child_request(request)?;
    if interrupt.load(Ordering::SeqCst) {
        return Err("Subagent was interrupted before starting.".into());
    }
    let system = child_system_prompt(snapshot, request);
    let mut messages = vec![
        serde_json::json!({"role": "system", "content": system}),
        serde_json::json!({"role": "user", "content": request.prompt}),
    ];
    let mut preferences = crate::store::Preferences {
        system_prompt: snapshot.system_prompt.clone(),
        temperature: snapshot.temperature,
        top_p: snapshot.top_p,
        max_tokens: snapshot.max_tokens,
        ..Default::default()
    };
    preferences.max_tokens =
        crate::context::fit_response_budget(preferences.max_tokens, backend.context_capacity());
    preferences.validate()?;
    backend.validate_response_tokens(preferences.max_tokens)?;
    let mut answer = String::new();
    let max_rounds = request.max_rounds.min(8);
    let mut event_seq: u64 = 1;
    for round in 0..max_rounds {
        if interrupt.load(Ordering::SeqCst) {
            if let Ok(s) = store.lock() {
                let _ = s.update_run_status(child_run_id, crate::agent_run::RunState::Cancelled, Some("Interrupted."), None);
            }
            return Err("Subagent was interrupted.".into());
        }
        if let Some(registry) = &registry {
            for note in registry.take_mail(child_run_id) {
                messages.push(serde_json::json!({"role": "user", "content": format!("[parent followup] {note}")}));
            }
        }
        let mut payload = serde_json::json!({
            "messages": messages, "temperature": preferences.temperature,
            "top_p": preferences.top_p, "max_tokens": preferences.max_tokens,
            "stream": true, "cache_prompt": true,
        });
        if snapshot.inference_slots >= 2 {
            payload["id_slot"] = serde_json::json!(1);
        }
        if backend.supports_tools() && !tools.is_empty() {
            payload["tools"] = serde_json::json!(tools.iter().map(|tool| tool.definition()).collect::<Vec<_>>());
        }
        let payload = backend.payload(&payload);
        let _tokens = tokio::time::timeout(Duration::from_secs(30), backend.check_context(&payload, preferences.max_tokens))
            .await
            .map_err(|_| "Subagent context check timed out.")??;
        let response = tokio::time::timeout(Duration::from_secs(600), backend.stream(&payload))
            .await
            .map_err(|_| "Subagent model did not respond within ten minutes.")??;
        let outcome = collect_stream(backend, response, &mut answer).await?;
        crate::arex::validate_batch(&outcome.calls)?;
        if outcome.finish == "length" {
            return Err("Subagent hit the response token limit.".into());
        }
        if outcome.calls.is_empty() {
            break;
        }
        if round + 1 >= max_rounds && !outcome.calls.iter().any(|c| c.name == "finish") {
            return Err("Subagent round limit reached.".into());
        }
        messages.push(serde_json::json!({"role": "assistant", "content": outcome.text,
            "tool_calls": outcome.calls.iter().map(|call| call.model_value()).collect::<Vec<_>>()}));
        for call in outcome.calls {
            if interrupt.load(Ordering::SeqCst) {
                return Err("Subagent was interrupted.".into());
            }
            let tool = tools.iter().find(|tool| tool.alias == call.name);
            let Some(tool) = tool else {
                messages.push(serde_json::json!({"role": "tool", "tool_call_id": call.id,
                    "content": "Unknown tool for this subagent. Use only the offered tools."}));
                continue;
            };
            let result = if !request.policy.allows(tool.trusted_read()) {
                serde_json::json!({"isError": true, "message": "Denied by the subagent approval pin (reads-only)."})
            } else if let Some(alias) = tool.harness_name() {
                // Background children have no AppState: serve the store-only
                // harness subset locally; everything else fails loud.
                match harness_detached(snapshot, request, alias, call.arguments.clone()) {
                    Ok(value) => value,
                    Err(error) => serde_json::json!({"isError": true, "message": error}),
                }
            } else {
                match tokio::time::timeout(tool.timeout(), tool.call(call.arguments.clone())).await {
                    Ok(Ok(value)) => value,
                    Ok(Err(error)) => serde_json::json!({"isError": true, "message": error}),
                    Err(_) => serde_json::json!({"isError": true, "message": "Subagent tool call timed out."}),
                }
            };
            let control = if tool.harness_name().is_some() && matches!(call.name.as_str(), "finish" | "update_context") { Some(result.clone()) } else { None };
            let (bounded, maybe_artifact) = crate::artifacts::bound_tool_result(
                result, &tool.tool.name, &request.conversation_id, Some(child_run_id),
                crate::artifacts::DEFAULT_MAX_RESULT_CHARS,
            );
            if let Some(artifact) = &maybe_artifact {
                if let Ok(s) = store.lock() {
                    let _ = s.save_artifact(artifact);
                }
            }
            event_seq += 1;
            if let Ok(s) = store.lock() {
                let _ = s.append_run_event(&crate::agent_run::RunEvent {
                    run_id: child_run_id.to_string(),
                    seq: event_seq,
                    step_id: format!("round-{round}"),
                    tool_call_id: Some(call.id.clone()),
                    event_type: "tool_result".into(),
                    payload: serde_json::json!({"tool": tool.alias, "result": bounded}),
                    created_at: crate::store::now(),
                });
            }
            messages.push(serde_json::json!({"role": "tool", "tool_call_id": call.id, "content": bounded.to_string()}));
            if let Some(control) = control {
                if let Some(final_answer) = crate::arex::apply_control(&call.name, &control, &*store.lock().map_err(|_| "Store lock poisoned")?, &request.conversation_id, child_run_id, &mut messages)? {
                    if let Some(schema) = &request.output_schema { check_output_schema(schema, &final_answer)?; }
                    return Ok(final_answer);
                }
            }
        }
    }
    if let Some(schema) = &request.output_schema {
        check_output_schema(schema, &answer)?;
    }
    Ok(answer)
}

struct StreamOutcome {
    finish: String,
    text: String,
    calls: Vec<crate::tool_calls::ToolCall>,
}

async fn collect_stream(
    backend: &crate::inference::Backend,
    response: reqwest::Response,
    answer: &mut String,
) -> Result<StreamOutcome, String> {
    let mut stream = response.bytes_stream();
    let mut decoder = crate::sse::SseDecoder::default();
    let mut calls = crate::tool_calls::ToolCalls::default();
    let mut text = String::new();
    let mut finish = String::new();
    'stream: loop {
        let next = tokio::time::timeout(Duration::from_secs(300), futures_util::StreamExt::next(&mut stream))
            .await
            .map_err(|_| "Subagent model stopped responding.")?;
        let Some(bytes) = next else {
            decoder.finish()?;
            return Err("Subagent stream ended without a completion marker.".into());
        };
        for event in decoder.push(&bytes.map_err(|error| error.to_string())?)? {
            if event == "[DONE]" {
                break 'stream;
            }
            let value: serde_json::Value =
                serde_json::from_str(&event).map_err(|error| format!("Invalid subagent stream: {error}"))?;
            if let Some(error) = value.get("error") {
                return Err(backend.stream_error(error));
            }
            if let Some(reason) = value["choices"][0]["finish_reason"].as_str() {
                finish = reason.to_string();
            }
            let delta = &value["choices"][0]["delta"];
            calls.push(delta)?;
            let chunk = delta["content"].as_str().unwrap_or("");
            answer.push_str(chunk);
            text.push_str(chunk);
            if answer.len() > 1_048_576 {
                return Err("Subagent output exceeded 1 MiB.".into());
            }
        }
    }
    Ok(StreamOutcome { finish, text, calls: calls.finish()? })
}

// ---- Workflows + Ralph (Phase 4 orchestration slice) ----

#[derive(Clone, Debug, Deserialize)]
pub struct WorkflowStep {
    pub prompt: String,
    pub label: Option<String>,
}

/// Sequential fan-out: each step runs as a fresh child that sees prior
/// summaries. Parallel: background children polled to settle (10 min budget).
/// Parent `inherit` tools carry into workflow children, except live MCP peers
/// (which cannot cross the clone boundary) — stages document that limit.
#[allow(clippy::too_many_arguments)]
pub async fn run_workflow(
    state: &crate::AppState,
    snapshot: &BackendSnapshot,
    conversation_id: &str,
    parent_run_id: &str,
    depth: i64,
    policy: ChildPolicy,
    mode: &str,
    steps: Vec<WorkflowStep>,
    max_rounds: usize,
    inherit: &[crate::connectors::AgentTool],
) -> Result<serde_json::Value, String> {
    if steps.is_empty() || steps.len() > 5 {
        return Err("workflow_run needs 1-5 steps.".into());
    }
    for step in &steps {
        if step.prompt.trim().is_empty() || step.prompt.len() > 4000 {
            return Err("Each workflow step needs a 1-4000 character prompt.".into());
        }
    }
    if mode != "sequential" && mode != "parallel" {
        return Err("workflow mode must be 'sequential' or 'parallel'.".into());
    }
    if mode == "sequential" {
        let mut results = Vec::new();
        let mut prior = String::new();
        for (index, step) in steps.iter().enumerate() {
            let prompt = if prior.is_empty() {
                step.prompt.clone()
            } else {
                format!("{}\n\n[prior stage summaries]\n{prior}", step.prompt)
            };
            let request = ChildRequest {
                label: step.label.clone().unwrap_or_else(|| format!("stage-{}", index + 1)),
                prompt,
                depth,
                max_rounds,
                tool_filter: None,
                persona: None,
                output_schema: None,
                policy,
                parent_run_id: parent_run_id.to_string(),
                conversation_id: conversation_id.to_string(),
            };
            let inherited: Vec<crate::connectors::AgentTool> = inherit.iter()
                .filter(|tool| !tool.is_mcp())
                .cloned()
                .collect();
            match run_child_inline(state, snapshot, request.clone(), &inherited, None).await {
                Ok((child_run_id, answer)) => {
                    prior.push_str(&format!("\n[{}] {}\n", request.label, answer.chars().take(1000).collect::<String>()));
                    results.push(serde_json::json!({"label": request.label, "childRunId": child_run_id, "status": "completed",
                        "summary": answer.chars().take(1500).collect::<String>()}));
                }
                Err(error) => {
                    results.push(serde_json::json!({"label": request.label, "status": "failed", "error": error}));
                    break;
                }
            }
        }
        return Ok(serde_json::json!({"mode": "sequential", "results": results}));
    }
    // Parallel: background children, then poll the durable rows to settle.
    let mut ids = Vec::new();
    for (index, step) in steps.iter().enumerate() {
        let request = ChildRequest {
            label: step.label.clone().unwrap_or_else(|| format!("branch-{}", index + 1)),
            prompt: step.prompt.clone(),
            depth,
            max_rounds,
            tool_filter: None,
            persona: None,
            output_schema: None,
            policy,
            parent_run_id: parent_run_id.to_string(),
            conversation_id: conversation_id.to_string(),
        };
        ids.push(spawn_background(snapshot.clone(), request)?);
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
    loop {
        if state.cancel_requested() {
            for id in &ids {
                state.subagents.interrupt(id);
            }
            return Err("Workflow was cancelled.".into());
        }
        let rows = {
            let store = state.database()?;
            store.subagent_runs_for_conversation(conversation_id)?
        };
        let mut settled = 0;
        for id in &ids {
            if let Some(row) = rows.iter().find(|row| &row.child_run_id == id) {
                if row.status != "running" {
                    settled += 1;
                }
            }
        }
        if settled >= ids.len() || tokio::time::Instant::now() > deadline {
            let mut results = Vec::new();
            for id in &ids {
                let row = rows.iter().find(|row| &row.child_run_id == id);
                results.push(match row {
                    Some(row) if row.status == "completed" => serde_json::json!({"label": row.label, "childRunId": id, "status": "completed"}),
                    Some(row) => serde_json::json!({"label": row.label, "childRunId": id, "status": row.status, "error": row.error}),
                    None => serde_json::json!({"childRunId": id, "status": "unknown"}),
                });
            }
            // Summaries live in each child's run events + settle notices.
            return Ok(serde_json::json!({"mode": "parallel", "results": results,
                "note": "Summaries are in each child's Trajectory events and settle notices."}));
        }
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Handoff {
    pub status: String,
    pub summary: String,
    pub evidence: Option<String>,
    pub next_steps: Option<String>,
    pub blocker: Option<String>,
}

fn parse_handoff(answer: &str) -> Option<Handoff> {
    // Last ```handoff fenced JSON block wins; otherwise the whole answer is
    // treated as an in-progress summary by the caller.
    let mut last: Option<&str> = None;
    let mut search = answer;
    while let Some(start) = search.find("```handoff") {
        let rest = &search[start + "```handoff".len()..];
        if let Some(end) = rest.find("```") {
            last = Some(rest[..end].trim());
            search = &rest[end + 3..];
        } else {
            break;
        }
    }
    let text = last?;
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    if !value.get("status")?.as_str().map(|status| ["complete", "blocked", "continue"].contains(&status))? {
        return None;
    }
    serde_json::from_value(value).ok()
}

/// Ralph loop: fixed objective, fresh child per round, shared workspace,
/// bounded handoff. Stops on complete/blocked or when rounds run out.
#[allow(clippy::too_many_arguments)]
pub async fn run_ralph(
    state: &crate::AppState,
    snapshot: &BackendSnapshot,
    conversation_id: &str,
    parent_run_id: &str,
    depth: i64,
    policy: ChildPolicy,
    objective: &str,
    max_rounds: usize,
    inherit: &[crate::connectors::AgentTool],
) -> Result<serde_json::Value, String> {
    if objective.trim().is_empty() || objective.len() > 2000 {
        return Err("ralph_run needs an objective of 1-2000 characters.".into());
    }
    let max_rounds = max_rounds.clamp(1, 5);
    let mut handoff: Option<Handoff> = None;
    let mut rounds = Vec::new();
    for round in 1..=max_rounds {
        let prompt = match &handoff {
            None => format!(
                "Objective (immutable): {objective}\n\nWork one iteration. End with a ```handoff fence: \
                 {{\"status\":\"complete\"|\"blocked\"|\"continue\",\"summary\":\"...\",\"evidence\":\"...\",\"next_steps\":\"...\",\"blocker\":\"...\"}}"
            ),
            Some(previous) => format!(
                "Objective (immutable): {objective}\n\nPrevious handoff: {}\n\nContinue one iteration and end with a new ```handoff fence.",
                serde_json::to_string(previous).unwrap_or_default()
            ),
        };
        let request = ChildRequest {
            label: format!("ralph-{round}"),
            prompt,
            depth,
            max_rounds: 4,
            tool_filter: None,
            persona: None,
            output_schema: None,
            policy,
            parent_run_id: parent_run_id.to_string(),
            conversation_id: conversation_id.to_string(),
        };
        let inherited: Vec<crate::connectors::AgentTool> = inherit.iter()
            .filter(|tool| !tool.is_mcp())
            .cloned()
            .collect();
        match run_child_inline(state, snapshot, request, &inherited, None).await {
            Ok((child_run_id, answer)) => {
                let parsed = parse_handoff(&answer).unwrap_or(Handoff {
                    status: "continue".into(),
                    summary: answer.chars().take(1000).collect(),
                    evidence: None,
                    next_steps: None,
                    blocker: None,
                });
                let terminal = parsed.status == "complete" || parsed.status == "blocked";
                rounds.push(serde_json::json!({"round": round, "childRunId": child_run_id, "handoff": parsed}));
                handoff = Some(parsed);
                if terminal {
                    break;
                }
            }
            Err(error) => {
                rounds.push(serde_json::json!({"round": round, "status": "failed", "error": error}));
                break;
            }
        }
    }
    Ok(serde_json::json!({"objective": objective, "rounds": rounds, "final": handoff}))
}

// ---- Model-facing + UI IPC ----

#[tauri::command]
pub fn list_subagent_runs(state: tauri::State<'_, crate::AppState>, conversation_id: String) -> Result<Vec<SubagentRun>, String> {
    state.database()?.subagent_runs_for_conversation(&conversation_id)
}

#[tauri::command]
pub fn interrupt_subagent(state: tauri::State<'_, crate::AppState>, child_run_id: String) -> Result<bool, String> {
    if child_run_id.is_empty() || child_run_id.len() > 128 {
        return Err("Child run id must be 1-128 characters.".into());
    }
    Ok(state.subagents.interrupt(&child_run_id))
}

#[tauri::command]
pub fn list_subagent_models(state: tauri::State<'_, crate::AppState>) -> Result<Vec<serde_json::Value>, String> {
    let store = state.database()?;
    list_models(&store)
}

/// Configured subagent model allowlist (empty = current conversation model).
pub fn list_models(store: &crate::store::Store) -> Result<Vec<serde_json::Value>, String> {
    let configured: Vec<serde_json::Value> = store.setting(MODELS_KEY).unwrap_or_default();
    for entry in &configured {
        if entry.get("providerId").is_none() || entry.get("modelId").and_then(|value| value.as_str()).is_none() {
            return Err("subagents.models entries need providerId and modelId.".into());
        }
    }
    Ok(configured.into_iter().take(16).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn depth_policy_and_filter_rules() {
        assert_eq!(ChildPolicy::from_access(crate::permissions::AccessMode::Ask), ChildPolicy::ReadsOnly);
        assert_eq!(ChildPolicy::from_access(crate::permissions::AccessMode::FullAccess), ChildPolicy::Full);
        assert!(ChildPolicy::ReadsOnly.allows(true) && !ChildPolicy::ReadsOnly.allows(false));
        let tool = crate::connectors::AgentTool::system_time();
        assert!(tool_allowed(&None, &tool));
        assert!(tool_allowed(&Some(vec!["system".into()]), &tool));
        assert!(!tool_allowed(&Some(vec!["workspace".into()]), &tool));
        // Harness tools route through the dispatcher, not direct calls.
        let harness = crate::connectors::AgentTool::harness("todo_write").unwrap();
        assert!(harness.is_harness() && !harness.is_mcp());
        assert!(tool_allowed(&Some(vec!["harness".into()]), &harness));
        assert!(!tool_allowed(&Some(vec!["mcp".into()]), &harness));
        assert!(!tool_allowed(&Some(vec!["workspace".into()]), &harness));
    }
    #[test]
    fn detached_support_covers_store_tools_only() {
        for alias in ["todo_write", "todo_add", "todo_update", "goal_set", "goal_clear",
            "memory_teach", "memory_recall", "file_search", "list_agents",
            "list_subagent_models", "preset_guide", "artifact_read"] {
            assert!(harness_detached_supported(alias), "{alias} should run detached");
        }
        for alias in ["subagent", "terminal_send", "docker_exec", "workflow_run", "ask_user", "ptc_run"] {
            assert!(!harness_detached_supported(alias), "{alias} needs the foreground dispatcher");
        }
    }
    #[test]
    fn child_requests_validate_loudly() {
        let base = ChildRequest {
            label: "x".into(), prompt: "do it".into(), depth: 0, max_rounds: 4,
            tool_filter: None, persona: None, output_schema: None,
            policy: ChildPolicy::ReadsOnly, parent_run_id: "p".into(), conversation_id: "c".into(),
        };
        assert!(validate_child_request(&base).is_ok());
        assert!(validate_child_request(&ChildRequest { prompt: "".into(), ..base.clone() }).is_err());
        assert!(validate_child_request(&ChildRequest { max_rounds: 9, ..base.clone() }).is_err());
        assert!(validate_child_request(&ChildRequest {
            output_schema: Some(serde_json::json!({"type": "object", "required": ["a"]})), ..base.clone()
        })
        .is_ok());
    }
    #[test]
    fn handoff_parsing_prefers_last_fence() {
        let answer = "first ```handoff\n{\"status\":\"continue\",\"summary\":\"a\"}\n``` then ```handoff\n{\"status\":\"blocked\",\"summary\":\"b\",\"blocker\":\"x\"}\n```";
        let handoff = parse_handoff(answer).unwrap();
        assert_eq!(handoff.status, "blocked");
        assert!(parse_handoff("no fence here").is_none());
        assert!(check_output_schema(&serde_json::json!({"required": ["a"]}), r#"{"a":1}"#).is_ok());
        assert!(check_output_schema(&serde_json::json!({"required": ["a"]}), r#"{"b":1}"#).is_err());
        assert!(check_output_schema(&serde_json::json!({}), "not json").is_err());
    }
}
