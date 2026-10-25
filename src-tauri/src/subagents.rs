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
/// `file_*` run through the same workspace-bounded handler as the foreground
/// catalog. Everything else needs the foreground dispatcher (approval,
/// terminals, schedulers, orchestration) and fails loud instead of hanging.
pub fn harness_detached_supported(alias: &str) -> bool {
    crate::git_tools::owns(alias)
        || (crate::lsp_tools::owns(alias) && alias != "lsp_execute_command")
        || matches!(
            alias,
            "todo_write"
                | "todo_add"
                | "todo_update"
                | "goal_set"
                | "goal_clear"
                | "memory_teach"
                | "memory_recall"
                | "file_search"
                | "file_read"
                | "file_write"
                | "list_agents"
                | "list_subagent_models"
                | "preset_guide"
                | "artifact_read"
                | "update_context"
                | "finish"
        )
}

/// Serve the store-only / workspace-bounded harness subset for background
/// children. Mirrors the foreground `harness::execute` semantics for these
/// aliases without approval prompts (background runs are already pinned by
/// the child policy).
pub async fn harness_detached(
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
    if let Some(reason) = crate::capabilities::ToolPolicy::resolve(&store).denial_for_alias(alias) {
        return Err(reason);
    }
    let workspace_path = snapshot.workspace_path.clone();
    match alias {
        name if crate::lsp_tools::owns(name) => {
            // A detached child owns a short-lived manager. It never borrows
            // the parent's live sessions or its pending approval authority.
            let manager = crate::lsp_client::Manager::default();
            crate::lsp_tools::execute(
                &manager,
                &snapshot.db_path,
                &request.conversation_id,
                &request.parent_run_id,
                &workspace_path,
                name,
                args,
                false,
            )
            .await
        }
        name if crate::git_tools::owns(name) => {
            crate::git_tools::execute(&workspace_path, name, args).await
        }
        // Same containment as the foreground catalog path: file_read is a
        // bounded read; file_write only creates (never overwrites).
        "file_read" | "file_write" => {
            crate::harness::execute_workspace_tool(
                &workspace_path,
                snapshot.execution_config.clone(),
                alias,
                args,
            )
            .await
        }
        "update_context" | "finish" => {
            crate::arex::validate(alias, &args)?;
            Ok(if alias == "finish" {
                serde_json::json!({"answer":crate::arex::finish_answer(&args)})
            } else {
                serde_json::json!({"context":args["context"]})
            })
        }
        "todo_write" => {
            let todos = crate::plans::parse_todo_write(&args)?;
            store.save_todos(&request.conversation_id, &todos)?;
            Ok(serde_json::json!({"saved": todos.len()}))
        }
        "todo_add" => {
            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or("todo_add needs a 'text' string.")?
                .to_string();
            let todo = store.add_todo(&request.conversation_id, &text)?;
            Ok(serde_json::json!({"added": todo}))
        }
        "todo_update" => {
            let index = args
                .get("index")
                .and_then(|v| v.as_u64())
                .ok_or("todo_update needs an 'index' integer.")? as usize;
            let status = args
                .get("status")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let todos = store.update_todo(
                &request.conversation_id,
                index,
                status.as_deref(),
                text.as_deref(),
            )?;
            Ok(serde_json::json!({"saved": todos.len()}))
        }
        "goal_set" => {
            let objective = args
                .get("objective")
                .and_then(|v| v.as_str())
                .ok_or("goal_set needs an 'objective' string.")?
                .to_string();
            crate::plans::validate_objective(&objective)?;
            store.save_goal(&request.conversation_id, &objective)?;
            Ok(serde_json::json!({"goal": objective}))
        }
        "goal_clear" => {
            store.clear_goal(&request.conversation_id)?;
            Ok(serde_json::json!({"cleared": true}))
        }
        "memory_teach" => {
            let fact = args
                .get("fact")
                .and_then(|v| v.as_str())
                .ok_or("memory_teach needs a 'fact' string.")?
                .to_string();
            let scope = args
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let scope = if scope.trim().is_empty() {
                if workspace_path.is_empty() {
                    "global".to_string()
                } else {
                    workspace_path.clone()
                }
            } else {
                scope
            };
            let record = store.teach_fact(&scope, &fact, "model")?;
            Ok(serde_json::json!({"id": record.id, "scope": record.scope}))
        }
        "memory_recall" => {
            let scope = args
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let scope = if scope.trim().is_empty() {
                if workspace_path.is_empty() {
                    "global".to_string()
                } else {
                    workspace_path.clone()
                }
            } else {
                scope
            };
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
            let facts = store.recall_facts(&scope, limit.min(50))?;
            Ok(serde_json::json!({"facts": facts}))
        }
        "file_search" => {
            let query = args
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or("file_search needs a 'query' string.")?
                .to_string();
            if workspace_path.is_empty() {
                return Err("Choose a workspace folder before file_search.".into());
            }
            let hits = crate::sandbox::file_search(std::path::Path::new(&workspace_path), &query)?;
            Ok(serde_json::json!({"hits": hits}))
        }
        "list_agents" => {
            let wanted = args.get("state").and_then(|v| v.as_str()).unwrap_or("all");
            if !["all", "running", "completed", "failed", "interrupted"].contains(&wanted) {
                return Err(
                    "list_agents state must be all, running, completed, failed, or interrupted."
                        .into(),
                );
            }
            let rows = store.subagent_runs_for_conversation(&request.conversation_id)?;
            let rows: Vec<&SubagentRun> = rows
                .iter()
                .filter(|row| wanted == "all" || row.status == wanted)
                .collect();
            Ok(serde_json::json!({"agents": rows}))
        }
        "list_subagent_models" => {
            let allowlist = list_models(&store)?;
            Ok(
                serde_json::json!({"models": allowlist, "note": "Empty allowlist means the current conversation model only."}),
            )
        }
        "preset_guide" => Ok(
            serde_json::json!({"guide": crate::presets::authoring_guide(), "presets": crate::presets::list()}),
        ),
        "artifact_read" => {
            let id = args
                .get("id")
                .and_then(|v| v.as_str())
                .ok_or("artifact_read needs an 'id' string.")?
                .to_string();
            let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(2000) as usize;
            let record = store
                .artifact(&id)?
                .ok_or("Unknown artifact id for this workspace.")?;
            if record.conversation_id != request.conversation_id {
                return Err("Unknown artifact id for this workspace.".into());
            }
            Ok(crate::artifacts::read_window(
                &record,
                offset,
                limit.min(20000),
            ))
        }
        _ => Err(format!(
            "Harness tool '{alias}' needs the foreground dispatcher."
        )),
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
    pub api_format: String,
    pub model_id: String,
    pub context_length: u32,
    pub max_output_tokens: Option<u32>,
    pub supports_tools: bool,
    pub model_capabilities: Option<crate::provider_capabilities::ModelCapabilities>,
    pub allowed_tool_aliases: Vec<String>,
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
        let mut backend = if self.kind == "local" {
            crate::inference::Backend::local(
                self.endpoint.clone(),
                self.key.clone(),
                self.context_length,
            )
        } else if self.kind == "subscription" {
            let model = crate::providers::RemoteModel {
                controls: self
                    .model_capabilities
                    .as_ref()
                    .and_then(|caps| caps.controls.clone()),
                id: self.model_id.clone(),
                context_length: Some(self.context_length),
                max_output_tokens: self.max_output_tokens,
                supports_images: false,
                tool_support: if let Some(caps) = &self.model_capabilities {
                    caps.tool_support.clone()
                } else if self.supports_tools {
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
                api_format: if self.api_format.is_empty() {
                    crate::providers::OPENAI_CHAT_COMPLETIONS.into()
                } else {
                    self.api_format.clone()
                },
                base_url: self.base_url.clone(),
                verified: true,
                last_tested_at: None,
                models: vec![],
                has_api_key: true,
            };
            let model = crate::providers::RemoteModel {
                controls: self
                    .model_capabilities
                    .as_ref()
                    .and_then(|caps| caps.controls.clone()),
                id: self.model_id.clone(),
                context_length: Some(self.context_length),
                max_output_tokens: self.max_output_tokens,
                supports_images: false,
                tool_support: if let Some(caps) = &self.model_capabilities {
                    caps.tool_support.clone()
                } else if self.supports_tools {
                    crate::providers::ToolSupport::Supported
                } else {
                    crate::providers::ToolSupport::Unsupported
                },
            };
            crate::inference::Backend::openai(&provider, self.key.clone(), &model)
        }?;
        if let Some(probe) = self
            .model_capabilities
            .as_ref()
            .and_then(|caps| caps.probe.clone())
        {
            match &mut backend {
                crate::inference::Backend::OpenAi { capabilities, .. }
                | crate::inference::Backend::Claude { capabilities, .. } => {
                    capabilities.probe = Some(probe)
                }
                _ => {}
            }
        }
        Ok(backend)
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
        let workspace_path = crate::workspace_ui::task_workspace(&store, conversation_id)?
            .unwrap_or(store.workspace_path()?);
        let execution_config = store
            .execution_config()
            .unwrap_or(crate::execution::ExecutionConfig::default());
        let active_skills = store.active_skills().unwrap_or_default();
        let remote_info = if let Some(ref provider_id) = selection.provider_id {
            let provider = store
                .provider(provider_id)?
                .ok_or("The selected provider was deleted.")?;
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
        (
            selection,
            preferences,
            workspace_path,
            execution_config,
            active_skills,
            remote_info,
        )
    };
    let mut snapshot = BackendSnapshot {
        model_capabilities: None,
        kind: "local".into(),
        endpoint: String::new(),
        key: String::new(),
        account_id: None,
        base_url: String::new(),
        api_format: String::new(),
        model_id: selection.model_id.clone(),
        context_length: 0,
        max_output_tokens: None,
        supports_tools: true,
        allowed_tool_aliases: Vec::new(),
        system_prompt: preferences.system_prompt,
        temperature: preferences.temperature,
        top_p: preferences.top_p,
        max_tokens: preferences.max_tokens,
        workspace_path,
        execution_config,
        active_skills,
        db_path: state.db_path.clone(),
        data_dir: state.data_dir.clone(),
        inference_slots: state
            .database()
            .ok()
            .and_then(|store| store.runtime_config().ok())
            .map(|config| config.inference_slots)
            .unwrap_or(1),
    };
    if let (Some(provider_id), Some((provider, model))) = (selection.provider_id, remote_info) {
        snapshot.model_capabilities = Some(
            crate::provider_capabilities::ModelCapabilities::from_model(&model),
        );
        if crate::providers::is_subscription_format(&provider.api_format) {
            let kind = if provider.api_format == crate::providers::CHATGPT_SUBSCRIPTION
                || provider.id == "chatgpt-subscription"
            {
                "chatgpt"
            } else {
                "grok"
            };
            let token =
                crate::subscription_auth::get_valid_access_token(&state.daytona_vault, kind)
                    .await?;
            let account_id = crate::subscription_auth::load_session(&state.daytona_vault, kind)
                .await?
                .and_then(|s| s.account_id);
            snapshot.kind = "subscription".into();
            snapshot.key = token;
            snapshot.account_id = account_id;
            snapshot.api_format = provider.api_format.clone();
            snapshot.base_url = provider.base_url;
            snapshot.context_length = model
                .context_length
                .ok_or("Configure the selected remote model's context capacity.")?;
            snapshot.max_output_tokens = model.max_output_tokens;
            snapshot.supports_tools = snapshot.model_capabilities.as_ref().is_none_or(|caps| {
                !matches!(
                    caps.tool_support,
                    crate::providers::ToolSupport::Unsupported
                )
            });
        } else {
            let key = crate::providers::resolve_api_key(
                &provider,
                state
                    .daytona_vault
                    .load_blocking(&crate::providers::credential_id(&provider_id))
                    .await?,
            )?;
            snapshot.kind = "remote".into();
            snapshot.key = key;
            snapshot.api_format = provider.api_format.clone();
            snapshot.base_url = provider.base_url;
            snapshot.context_length = model
                .context_length
                .ok_or("Configure the selected remote model's context capacity.")?;
            snapshot.max_output_tokens = model.max_output_tokens;
            snapshot.supports_tools = snapshot.model_capabilities.as_ref().is_none_or(|caps| {
                !matches!(
                    caps.tool_support,
                    crate::providers::ToolSupport::Unsupported
                )
            });
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
    // `setting::<i64>` returns i64::default() (0) for a missing key, so a
    // scalar read silently caps depth at 0 and every delegation fails.
    // Option distinguishes "never configured" from an explicit 0.
    let configured: Option<i64> = store.setting(MAX_DEPTH_KEY).unwrap_or_default();
    configured.unwrap_or(3).clamp(0, 8)
}

/// The interrupted error string; settle paths map it to RunState::Cancelled.
pub const INTERRUPTED: &str = "Subagent was interrupted.";

/// Per-child async cancellation: the interrupt flag (cheap synchronous
/// pre-check and shared store-visible state) plus a watch channel so blocking
/// awaits — token counting, request headers, stream collection, tool calls —
/// resolve promptly instead of running out their timeouts. Foreground runs
/// additionally watch the app-wide cancel channel, which `begin_generation`
/// resets per run, so an old Stop can never cancel a new run's child.
pub struct ChildCancel {
    flag: Arc<AtomicBool>,
    watch: Option<tokio::sync::watch::Receiver<bool>>,
    global: Option<tokio::sync::watch::Receiver<bool>>,
}

impl ChildCancel {
    /// No cancellation source — for tests and callers without a registry.
    pub fn none() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
            watch: None,
            global: None,
        }
    }

    pub fn new(
        flag: Arc<AtomicBool>,
        watch: Option<tokio::sync::watch::Receiver<bool>>,
        global: Option<tokio::sync::watch::Receiver<bool>>,
    ) -> Self {
        Self {
            flag,
            watch,
            global,
        }
    }

    pub fn is_set(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
            || self.watch.as_ref().is_some_and(|watch| *watch.borrow())
            || self.global.as_ref().is_some_and(|watch| *watch.borrow())
    }

    /// Resolves only when a cancellation actually arrives. A channel that
    /// closes without delivering `true` narrows the wait; when no channel can
    /// still signal, the future parks so the select's work arm wins normally.
    pub async fn cancelled(&mut self) {
        loop {
            if self.is_set() {
                return;
            }
            if self.watch.is_some() && self.global.is_some() {
                enum Arm {
                    Watch(bool),
                    Global(bool),
                }
                let arm = {
                    let (watch, global) =
                        (self.watch.as_mut().unwrap(), self.global.as_mut().unwrap());
                    tokio::select! {
                        result = watch.changed() => Arm::Watch(result.is_ok()),
                        result = global.changed() => Arm::Global(result.is_ok()),
                    }
                };
                match arm {
                    Arm::Watch(false) => self.watch = None,
                    Arm::Global(false) => self.global = None,
                    _ => {}
                }
            } else if let Some(watch) = &mut self.watch {
                if watch.changed().await.is_err() {
                    self.watch = None;
                }
            } else if let Some(global) = &mut self.global {
                if global.changed().await.is_err() {
                    self.global = None;
                }
            } else {
                std::future::pending::<()>().await;
            }
        }
    }
}

pub struct SubagentRegistry {
    flags: Mutex<HashMap<String, Arc<AtomicBool>>>,
    cancels: Mutex<HashMap<String, tokio::sync::watch::Sender<bool>>>,
    mailbox: Mutex<HashMap<String, Vec<String>>>,
}

impl SubagentRegistry {
    pub fn new() -> Self {
        Self {
            flags: Mutex::new(HashMap::new()),
            cancels: Mutex::new(HashMap::new()),
            mailbox: Mutex::new(HashMap::new()),
        }
    }
    pub fn register(&self, child_run_id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        let (cancel, _) = tokio::sync::watch::channel(false);
        let mut cancels = self
            .cancels
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.flags
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(child_run_id.to_string(), flag.clone());
        cancels.insert(child_run_id.to_string(), cancel);
        flag
    }
    /// Async wakeup for a live child. A receiver subscribed after the
    /// interrupt still observes the stored `true`, so no wakeup is lost.
    pub fn cancellation(&self, child_run_id: &str) -> Option<tokio::sync::watch::Receiver<bool>> {
        self.cancels
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(child_run_id)
            .map(|sender| sender.subscribe())
    }
    pub fn interrupt(&self, child_run_id: &str) -> bool {
        let live = {
            let flags = self
                .flags
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(flag) = flags.get(child_run_id) {
                flag.store(true, Ordering::SeqCst);
                true
            } else {
                false
            }
        };
        if live {
            if let Some(cancel) = self
                .cancels
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .get(child_run_id)
            {
                // send_replace stores the value even with no receivers yet.
                cancel.send_replace(true);
            }
        }
        live
    }
    pub fn take_mail(&self, child_run_id: &str) -> Vec<String> {
        self.mailbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(child_run_id)
            .unwrap_or_default()
    }
    pub fn deliver(&self, child_run_id: &str, message: &str) -> bool {
        let live = self
            .flags
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains_key(child_run_id);
        if !live {
            return false;
        }
        self.mailbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(child_run_id.to_string())
            .or_default()
            .push(message.to_string());
        true
    }
    pub fn retire(&self, child_run_id: &str) {
        self.flags
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(child_run_id);
        self.cancels
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(child_run_id);
        self.mailbox
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(child_run_id);
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
    REGISTRY
        .get_or_init(|| Arc::new(SubagentRegistry::new()))
        .clone()
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
        // Compile through the bounded offline validator before any inference:
        // malformed schemas and remote $refs reject here, not after the run.
        crate::tool_validation::compile(schema)
            .map_err(|error| format!("outputSchema is not a usable JSON Schema: {error}"))?;
        // The output contract is a JSON object; a schema that cannot admit one
        // (e.g. type "array") can never be satisfied — reject it up front.
        let permits_object = match schema.get("type") {
            None => true,
            Some(serde_json::Value::String(kind)) => kind == "object",
            Some(serde_json::Value::Array(kinds)) => {
                kinds.iter().any(|kind| kind.as_str() == Some("object"))
            }
            Some(_) => true,
        };
        if !permits_object {
            return Err(
                "outputSchema must describe a JSON object; subagent answers are objects.".into(),
            );
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
    prompt.push_str(
        "\nYou are a subagent: solve ONLY the assigned subtask and report back concisely.",
    );
    prompt
}

/// Enforce the requested output schema fully (nested types, enums, ranges,
/// additionalProperties, $defs) through the shared offline validator. The
/// object contract is preserved: even a schema that admits scalars still
/// requires the child to return a JSON object.
fn check_output_schema(schema: &serde_json::Value, answer: &str) -> Result<(), String> {
    let value: serde_json::Value = serde_json::from_str(answer.trim()).map_err(|_| {
        "The subagent did not return valid JSON for the requested outputSchema.".to_string()
    })?;
    if !value.is_object() {
        return Err(
            "The subagent must return a JSON object for the requested outputSchema.".into(),
        );
    }
    crate::tool_validation::validate_value(schema, &value)
        .map_err(|error| format!("Subagent output does not match outputSchema: {error}"))
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

/// Execution aliases a parent catalog may have authorized. The namespaced
/// alias is the direct tool; the bare alias is how chat assembles the
/// preset's harness entry — the same `LocalExecution` backend either way.
const CODE_ALIASES: &[&str] = &["local_run_code", "run_code"];
const COMMAND_ALIASES: &[&str] = &["local_execute_command", "execute_command"];

/// Aliases that can be rebuilt from a snapshot without the parent's live
/// session: workspace, local execution, detached-safe harness, skills, clock.
fn detached_reconstructable(alias: &str) -> bool {
    if alias == "workspace_open_preview" {
        return false;
    }
    alias.starts_with("workspace_")
        || CODE_ALIASES.contains(&alias)
        || COMMAND_ALIASES.contains(&alias)
        || alias == "skills_read_file"
        || alias == "system_time"
        || matches!(
            alias,
            crate::tool_discovery::SEARCH | crate::tool_discovery::INVOKE
        )
        || harness_detached_supported(alias)
}

/// Capability gating a detached alias, including aliases that never reach the
/// harness registry (namespaced local tools, hashed connector names).
fn detached_capability(alias: &str) -> Option<&'static str> {
    crate::capabilities::harness_alias_capability(alias).or_else(|| {
        if alias.starts_with("workspace_") {
            Some("workspace_files")
        } else if CODE_ALIASES.contains(&alias) || COMMAND_ALIASES.contains(&alias) {
            Some("local_execution")
        } else if alias == "skills_read_file" {
            Some("skills_reader")
        } else if alias == "system_time" {
            Some("system_time")
        } else if alias.starts_with("Daytona") {
            Some("cloud_execution")
        } else {
            None
        }
    })
}

/// Parent-authorized aliases a background child cannot use, with structured
/// reasons: capability kills, reconstructable-but-dropped (runtime, source or
/// request filter), and tools bound to the live parent session (connectors,
/// automations, orchestration, terminals). Reported honestly — foreground
/// children borrowing live peers are never listed.
fn detached_tool_limits(
    snapshot: &BackendSnapshot,
    tools: &[&crate::connectors::AgentTool],
    policy: &crate::capabilities::ToolPolicy,
) -> Vec<serde_json::Value> {
    let offered: std::collections::HashSet<&str> =
        tools.iter().map(|tool| tool.alias.as_str()).collect();
    snapshot
        .allowed_tool_aliases
        .iter()
        .filter(|alias| !offered.contains(alias.as_str()))
        .map(|alias| {
            let reason = if detached_capability(alias)
                .and_then(|capability| policy.denial(capability))
                .is_some()
            {
                "disabled in Capability settings"
            } else if detached_reconstructable(alias) {
                "not offered this run (source, runtime or child tool filter)"
            } else {
                "needs the parent's live session (connector, automation or orchestration)"
            };
            serde_json::json!({"alias": alias, "reason": reason})
        })
        .collect()
}

/// Reconstruct the local-only tool subset from a snapshot (workspace +
/// execution + detached-safe harness + clock + skill reader), minus families
/// killed in Capability settings. `snapshot.allowed_tool_aliases` — the
/// aliases the parent catalog was actually authorized to offer — is the exact
/// allowlist: canonical `run_code`/`execute_command` and namespaced
/// `local_*` spellings are both rebuilt verbatim so tool filters and replayed
/// history stay accurate. Only configured, available interpreters/shells are
/// offered. MCP peers, automations and Daytona stay with the foreground task;
/// [`detached_tool_limits`] reports that limit instead of claiming borrowed
/// peers are usable.
pub fn local_tools(
    snapshot: &BackendSnapshot,
    policy: &crate::capabilities::ToolPolicy,
) -> Vec<crate::connectors::AgentTool> {
    let allowed = |alias: &str| {
        snapshot
            .allowed_tool_aliases
            .iter()
            .any(|entry| entry == alias)
    };
    let mut tools = Vec::new();
    if !snapshot.workspace_path.is_empty() {
        if policy.denial("workspace_files").is_none() {
            if let Ok(workspace) = crate::workspace::Workspace::open(&snapshot.workspace_path) {
                tools.extend(std::sync::Arc::new(workspace).tools());
            }
        }
        if policy.denial("local_execution").is_none() {
            if let Ok(execution) = crate::execution::LocalExecution::new(
                snapshot.execution_config.clone(),
                &snapshot.workspace_path,
            ) {
                let execution = std::sync::Arc::new(execution);
                if execution.code_available() {
                    for &alias in CODE_ALIASES {
                        if allowed(alias) {
                            let mut tool = execution.clone().tool();
                            tool.alias = alias.into();
                            tools.push(tool);
                        }
                    }
                }
                if execution.command_available() {
                    for &alias in COMMAND_ALIASES {
                        if allowed(alias) {
                            let mut tool = execution.clone().command_tool();
                            tool.alias = alias.into();
                            tools.push(tool);
                        }
                    }
                }
            }
        }
    }
    if !snapshot.active_skills.is_empty() && policy.denial("skills_reader").is_none() {
        let skills = crate::skills::Skills::new(snapshot.data_dir.join("skills"));
        if let Ok(Some(reader)) = skills.reader(&snapshot.active_skills) {
            tools.push(reader);
        }
    }
    if policy.denial("system_time").is_none() {
        tools.push(crate::connectors::AgentTool::system_time());
    }
    // Authorized detached-safe harness aliases: their handlers already exist
    // in `harness_detached`; until now they were simply never reconstructed.
    // The file_* aliases are workspace-bound — without a workspace they would
    // only fail at call time, so they are not offered (the limits disclosure
    // reports them as unavailable this run instead).
    let workspace_bound = |alias: &str| {
        crate::git_tools::owns(alias)
            || crate::lsp_tools::owns(alias)
            || matches!(alias, "file_read" | "file_write" | "file_search")
    };
    for spec in crate::harness::registry() {
        let alias = &spec.name;
        if crate::git_tools::owns(alias) && !crate::git_tools::available() {
            continue;
        }
        if workspace_bound(alias) && snapshot.workspace_path.is_empty() {
            continue;
        }
        if (harness_detached_supported(alias)
            || matches!(
                alias.as_str(),
                crate::tool_discovery::SEARCH | crate::tool_discovery::INVOKE
            ))
            && allowed(alias)
            && policy.denial_for_alias(alias).is_none()
        {
            if let Ok(mut tool) = crate::connectors::AgentTool::harness(alias) {
                if crate::lsp_tools::owns(alias) {
                    let configured = crate::store::Store::open(&snapshot.db_path)
                        .and_then(|store| crate::lsp_client::load(&store));
                    if !configured
                        .as_ref()
                        .is_ok_and(|config| crate::lsp_tools::configure_tool(&mut tool, config))
                    {
                        continue;
                    }
                }
                tools.push(tool);
            }
        }
    }
    tools.retain(|tool| {
        snapshot.supports_tools
            && snapshot.allowed_tool_aliases.contains(&tool.alias)
            && policy.allows_tool(tool)
    });
    tools
}

/// Merge reconstructed local tools with inherited parent tools, deduplicated
/// by alias. Borrowed tools win collisions: they are the parent's live,
/// authorized instances (MCP/automation peers cannot be reconstructed).
fn merge_child_tools<'a>(
    owned: Vec<crate::connectors::AgentTool>,
    inherit: &'a [crate::connectors::AgentTool],
    filter: &Option<Vec<String>>,
    policy: &crate::capabilities::ToolPolicy,
) -> Vec<ChildTool<'a>> {
    let mut seen = std::collections::HashSet::new();
    let mut tools: Vec<ChildTool<'a>> = inherit
        .iter()
        .filter(|tool| seen.insert(tool.alias.clone()))
        .map(ChildTool::Borrowed)
        .collect();
    tools.extend(
        owned
            .into_iter()
            .filter(|tool| seen.insert(tool.alias.clone()))
            .map(ChildTool::Owned),
    );
    tools
        .into_iter()
        .filter(|tool| tool_allowed(filter, tool.tool()) && policy.allows_tool(tool.tool()))
        .collect()
}

/// Drive the bounded child loop. Shared by foreground and background runs.
#[allow(clippy::too_many_arguments)]
pub async fn drive_child(
    state: &crate::AppState,
    snapshot: &BackendSnapshot,
    request: &ChildRequest,
    tools: Vec<ChildTool<'_>>,
    backend: &crate::inference::Backend,
    mut cancel: ChildCancel,
    registry: Option<Arc<SubagentRegistry>>,
    child_run_id: &str,
    emit: Option<ChildEmit>,
) -> Result<String, String> {
    use crate::inference::InferenceProvider;
    validate_child_request(request)?;
    if cancel.is_set() {
        return Err("Subagent was interrupted before starting.".into());
    }
    let tools: Vec<&crate::connectors::AgentTool> = tools.iter().map(|tool| tool.tool()).collect();
    let mut system = child_system_prompt(snapshot, request);
    system.push_str(&format!(
        "\nModel capability metadata (not independent verification): {}",
        backend.capability_report()
    ));
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
    backend.prepare_preferences(&mut preferences)?;
    let mut answer = String::new();
    let max_rounds = request.max_rounds.min(8);
    // Monotonic per-run sequence: (run_id, seq) is the primary key.
    let mut event_seq: u64 = 1;
    for round in 0..max_rounds {
        if cancel.is_set() {
            if let Ok(store) = state.database() {
                let _ = store.update_run_status(
                    child_run_id,
                    crate::agent_run::RunState::Cancelled,
                    Some("Interrupted."),
                    None,
                );
            }
            return Err(INTERRUPTED.into());
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
            payload["tools"] =
                serde_json::json!(crate::tool_discovery::advertised(tools.iter().copied())
                    .into_iter()
                    .map(|tool| tool.definition())
                    .collect::<Vec<_>>());
        }
        if let Some(schema) = &request.output_schema {
            backend.constrain_output(&mut payload, schema)?;
        }
        let payload = backend.payload(&payload)?;
        // Every blocking await — token counting, request headers, the stream,
        // tool calls, approval — is selected against the child's cancel watch
        // so an interrupt lands promptly instead of after a deadman timeout.
        let _tokens = match tokio::select! {
            _ = cancel.cancelled() => return Err(INTERRUPTED.into()),
            result = tokio::time::timeout(Duration::from_secs(45), backend.check_context(&payload, preferences.max_tokens)) => result,
        } {
            Ok(result) => result?,
            Err(_) => return Err("Subagent context check timed out.".into()),
        };
        let response = match tokio::select! {
            _ = cancel.cancelled() => return Err(INTERRUPTED.into()),
            result = tokio::time::timeout(Duration::from_secs(600), backend.stream(&payload)) => result,
        } {
            Ok(result) => result?,
            Err(_) => return Err("Subagent model did not respond within ten minutes.".into()),
        };
        let outcome = collect_stream(backend, response, &mut answer, &mut cancel).await?;
        if outcome.finish == "length" {
            return Err("Subagent hit the response token limit.".into());
        }
        let calls = outcome.calls;
        crate::arex::validate_batch(&calls)?;
        if calls.is_empty() {
            break;
        }
        if round + 1 >= max_rounds && !calls.iter().any(|c| c.name == "finish") {
            return Err("Subagent round limit reached.".into());
        }
        messages.push(outcome.assistant_message);
        for mut call in calls {
            if cancel.is_set() {
                return Err(INTERRUPTED.into());
            }
            let visible = crate::tool_discovery::advertised(tools.iter().copied())
                .into_iter()
                .any(|tool| tool.alias == call.name);
            let resolved = if visible {
                crate::tool_discovery::resolve(
                    tools.iter().copied(),
                    &call.name,
                    call.arguments.clone(),
                )
            } else {
                Err("Tool is deferred or unavailable. Use tool_search and tool_invoke for this child's authorized catalog.".into())
            };
            let (resolved_tool, arguments) = match resolved {
                Ok(value) => value,
                Err(error) => {
                    messages.push(serde_json::json!({"role":"tool","tool_call_id":call.id,"content":crate::tool_envelope::failure_notice("unavailable-tool",&error,false,crate::tool_envelope::SIDE_NONE,&serde_json::json!({})).to_string()}));
                    continue;
                }
            };
            let tool = &resolved_tool;
            call.name = tool.alias.clone();
            call.arguments = arguments;
            // Capability kills re-check at dispatch; stale offers deny visibly.
            if let Some(reason) =
                crate::capabilities::ToolPolicy::guard(state.database()).denial_for_tool(tool)
            {
                messages.push(serde_json::json!({"role":"tool","tool_call_id":call.id,
                    "content":crate::tool_envelope::failure_notice("capability-disabled", &reason, false, crate::tool_envelope::SIDE_NONE, &serde_json::json!({})).to_string()}));
                continue;
            }
            match tool.prepare_arguments(call.arguments.clone()) {
                Ok(arguments) => call.arguments = arguments,
                Err(error) => {
                    messages.push(serde_json::json!({"role":"tool","tool_call_id":call.id,"content":crate::tool_envelope::failure_notice("invalid-arguments", &error, false, crate::tool_envelope::SIDE_NONE, &serde_json::json!({})).to_string()}));
                    continue;
                }
            }
            let allowed = request.policy.allows(tool.trusted_read())
                && !tool.requires_approval(&call.arguments);
            let result = if !allowed && emit.is_some() {
                // Dropping the approval future releases the pending entry via
                // its guard; nothing is left awaiting a decision.
                let decision = tokio::select! {
                    _ = cancel.cancelled() => {
                        if let Some(emit) = emit.as_ref() { emit("Approval withdrawn (subagent interrupted)".into(), Some(serde_json::Value::Null)); }
                        return Err(INTERRUPTED.into());
                    }
                    decision = request_approval_for_child(state, request, child_run_id, emit.as_ref(), &call) => decision,
                };
                match decision {
                    Some(true) => tokio::select! {
                        _ = cancel.cancelled() => return Err(cancelled_tool_call(state, child_run_id, round, &call, &mut event_seq)),
                        result = call_child_tool(state, snapshot, request, &tools, child_run_id, round, tool, &call, emit.as_ref()) => result,
                    },
                    _ => crate::tool_envelope::failure_notice("denied", "Denied by the subagent approval pin (reads-only). Ask the parent for what you need instead.", false, crate::tool_envelope::SIDE_NONE, &serde_json::json!({"denied":true})),
                }
            } else if !allowed {
                crate::tool_envelope::failure_notice("denied", "Denied by the subagent approval pin (reads-only). Ask the parent for what you need instead.", false, crate::tool_envelope::SIDE_NONE, &serde_json::json!({"denied":true}))
            } else {
                // Dropping the call future aborts in-flight work; managed
                // child processes die with the execution wrapper on drop.
                tokio::select! {
                    _ = cancel.cancelled() => return Err(cancelled_tool_call(state, child_run_id, round, &call, &mut event_seq)),
                    result = call_child_tool(state, snapshot, request, &tools, child_run_id, round, tool, &call, emit.as_ref()) => result,
                }
            };
            let control = if tool.harness_name().is_some()
                && matches!(call.name.as_str(), "finish" | "update_context")
            {
                Some(result.clone())
            } else {
                None
            };
            let (bounded, maybe_artifact) = crate::artifacts::bound_tool_result(
                result,
                &tool.tool.name,
                &request.conversation_id,
                Some(child_run_id),
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
                if let Some(final_answer) = crate::arex::apply_control(
                    &call.name,
                    &control,
                    &*state.database()?,
                    &request.conversation_id,
                    child_run_id,
                    &mut messages,
                )? {
                    if let Some(schema) = &request.output_schema {
                        check_output_schema(schema, &final_answer)?;
                    }
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

/// Durable audit record for a tool call abandoned mid-flight: the work future
/// is dropped (managed processes die with it), the side effect is reported as
/// unknown, and the call is never replayed.
fn cancelled_tool_call(
    state: &crate::AppState,
    child_run_id: &str,
    round: usize,
    call: &crate::tool_calls::ToolCall,
    event_seq: &mut u64,
) -> String {
    if let Ok(store) = state.database() {
        *event_seq += 1;
        let _ = store.append_run_event(&crate::agent_run::RunEvent {
            run_id: child_run_id.to_string(),
            seq: *event_seq,
            step_id: format!("round-{round}"),
            tool_call_id: Some(call.id.clone()),
            event_type: "tool_cancelled".into(),
            payload: serde_json::json!({"tool": call.name, "cancelled": true,
                "message": "Subagent interrupted before the tool call settled; any side effects are unknown and it is not replayed."}),
            created_at: crate::store::now(),
        });
    }
    INTERRUPTED.to_string()
}

/// Same audit for the detached driver (its own store handle).
fn cancelled_tool_call_detached(
    store: &Mutex<crate::store::Store>,
    child_run_id: &str,
    round: usize,
    call: &crate::tool_calls::ToolCall,
    event_seq: &mut u64,
) -> String {
    if let Ok(store) = store.lock() {
        *event_seq += 1;
        let _ = store.append_run_event(&crate::agent_run::RunEvent {
            run_id: child_run_id.to_string(),
            seq: *event_seq,
            step_id: format!("round-{round}"),
            tool_call_id: Some(call.id.clone()),
            event_type: "tool_cancelled".into(),
            payload: serde_json::json!({"tool": call.name, "cancelled": true,
                "message": "Subagent interrupted before the tool call settled; any side effects are unknown and it is not replayed."}),
            created_at: crate::store::now(),
        });
    }
    INTERRUPTED.to_string()
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
    emit: Option<&ChildEmit>,
) -> serde_json::Value {
    if tool.harness_name() == Some(crate::tool_discovery::SEARCH) {
        let policy = crate::capabilities::ToolPolicy::guard(state.database());
        return match crate::tool_discovery::search(tools.iter().copied(), &call.arguments, &policy)
        {
            Ok(value) => crate::tool_envelope::normalize(value, tool.envelope_family()),
            Err(error) => crate::tool_envelope::failure_notice(
                "discovery-error",
                &error,
                false,
                crate::tool_envelope::SIDE_NONE,
                &serde_json::json!({}),
            ),
        };
    }
    if let Some(alias) = tool.harness_name() {
        // A child runs inside its parent's turn, so it charges the parent's
        // search/fetch budget and obeys the parent's replay-protection set.
        // Without this a child could spend an unbounded number of provider
        // searches, and could re-execute a call the parent refused to repeat
        // because its outcome was uncertain.
        if let Err(error) =
            crate::continuation::admit_tool(state, &request.parent_run_id, alias, &call.arguments)
        {
            return crate::tool_envelope::failure_notice(
                "tool-budget-blocked",
                &error,
                false,
                crate::tool_envelope::SIDE_NONE,
                &serde_json::json!({"budget":true}),
            );
        }
        let signature = crate::continuation::tool_signature(tool, &call.arguments);
        let blocked = state
            .turn_blocked
            .lock()
            .ok()
            .and_then(|map| map.get(&request.parent_run_id).cloned());
        if let Some(blocked) = blocked {
            let already = blocked
                .lock()
                .map(|set| set.contains(&signature))
                .unwrap_or(true);
            if already {
                return crate::tool_envelope::failure_notice(
                    "replay-blocked",
                    "Not repeated: an earlier attempt this turn had an uncertain outcome.",
                    false,
                    crate::tool_envelope::SIDE_UNKNOWN,
                    &serde_json::json!({}),
                );
            }
        }
        // Foreground children keep the real progress channel so nested MCP
        // input forms reach the chat; detached paths fall back to a no-op.
        let emit: ChildEmit = match emit {
            Some(emit) => emit.clone(),
            None => Arc::new(|_, _| {}),
        };
        // Rebuild owned AgentTools: HarnessCtx needs &[AgentTool], and AgentTool
        // is Clone (MCP peers become detached markers that fail loud on call).
        let owned: Vec<crate::connectors::AgentTool> =
            tools.iter().map(|tool| (*tool).clone()).collect();
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
        if let Some(blocked) =
            crate::hooks::pre_use(state, &request.conversation_id, alias, &call.arguments)
                .await
                .blocked
        {
            return crate::tool_envelope::failure_notice(
                "hook-blocked",
                &blocked,
                false,
                crate::tool_envelope::SIDE_NONE,
                &serde_json::json!({"hookBlocked":true}),
            );
        }
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
                crate::tool_envelope::normalize(outcome.value, tool.envelope_family())
            }
            Ok(Err(error)) => crate::tool_envelope::normalize_error(&error, crate::tool_envelope::Cause::default(), tool.envelope_family()),
            Err(_) => crate::tool_envelope::normalize_error("Subagent tool call timed out. Its outcome may be unknown; do not automatically retry.", crate::tool_envelope::Cause { timeout: true, ..Default::default() }, tool.envelope_family()),
        }
    } else {
        // Live MCP peers inherited from the parent keep their session; the
        // emit channel lets them surface typed input forms on this chat.
        // A child runs inside its parent's turn, so an MCP call is charged
        // against the parent's search/fetch budget and obeys the parent's
        // replay-protection set too.
        if let Err(error) = crate::continuation::admit_tool(
            state,
            &request.parent_run_id,
            &tool.tool.name,
            &call.arguments,
        ) {
            return crate::tool_envelope::failure_notice(
                "tool-budget-blocked",
                &error,
                false,
                crate::tool_envelope::SIDE_NONE,
                &serde_json::json!({"budget":true}),
            );
        }
        let signature = crate::continuation::tool_signature(tool, &call.arguments);
        let blocked = state
            .turn_blocked
            .lock()
            .ok()
            .and_then(|map| map.get(&request.parent_run_id).cloned());
        if let Some(blocked) = blocked {
            let already = blocked
                .lock()
                .map(|set| set.contains(&signature))
                .unwrap_or(true);
            if already {
                return crate::tool_envelope::failure_notice(
                    "replay-blocked",
                    "Not repeated: an earlier attempt this turn had an uncertain outcome.",
                    false,
                    crate::tool_envelope::SIDE_UNKNOWN,
                    &serde_json::json!({}),
                );
            }
        }
        let mcp_ui = match (tool.is_mcp(), emit) {
            (true, Some(emit)) => Some(crate::mcp_input::CallUi::new(
                state.mcp_inputs.clone(),
                emit.clone(),
            )),
            _ => None,
        };
        match tokio::time::timeout(tool.timeout(), tool.call_with_input(call.arguments.clone(), None, mcp_ui)).await {
            Ok(Ok(value)) => crate::tool_envelope::normalize(value, tool.envelope_family()),
            Ok(Err(error)) => crate::tool_envelope::normalize_error(&error, crate::tool_envelope::Cause::default(), tool.envelope_family()),
            Err(_) => crate::tool_envelope::normalize_error("Subagent tool call timed out. Its outcome may be unknown; do not automatically retry.", crate::tool_envelope::Cause { timeout: true, ..Default::default() }, tool.envelope_family()),
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
    let _pending = state.approvals.guard(&approval_id);
    if let Some(emit) = emit {
        emit(
            format!(
                "Awaiting approval: {} (subagent {})",
                call.name, request.label
            ),
            Some(
                serde_json::json!({"id": approval_id, "connector": "Subagent",
                "name": call.name, "arguments": call.arguments,
                "subagentLabel": request.label, "childRunId": child_run_id}),
            ),
        );
    }
    let allow = tokio::time::timeout(std::time::Duration::from_secs(600), decision)
        .await
        .ok()
        .and_then(|result| result.ok());
    state.approvals.remove(&approval_id);
    if let Some(emit) = emit {
        emit(
            "Approval decision recorded".into(),
            Some(serde_json::Value::Null),
        );
    }
    allow
}

use std::time::Duration;

/// Foreground child: runs inline with the caller's reconstructed local tools
/// plus any inherited tools the filter admits. Returns the final answer.
/// Binds a child run's id to its parent's shared turn budget and
/// replay-protection set, and removes the binding when the child ends.
///
/// The binding makes `continuation::admit_tool` and the replay guard resolve
/// the child's run id to the **parent's** cells, so a child (and any nested
/// child) charges the same turn budget the parent loop uses.
struct ChildTurnBinding<'a> {
    budgets: &'a std::sync::Mutex<
        std::collections::HashMap<
            String,
            std::sync::Arc<std::sync::Mutex<crate::continuation::Budgets>>,
        >,
    >,
    blocked: &'a std::sync::Mutex<
        std::collections::HashMap<
            String,
            std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
        >,
    >,
    child_run_id: String,
}

impl Drop for ChildTurnBinding<'_> {
    fn drop(&mut self) {
        if let Ok(mut map) = self.budgets.lock() {
            map.remove(&self.child_run_id);
        }
        if let Ok(mut map) = self.blocked.lock() {
            map.remove(&self.child_run_id);
        }
    }
}

/// Bind a child run to its parent's turn cells. Returns `None` when the
/// parent has no registered turn (a detached or standalone context); the
/// child then runs with its own bounds, as it did before.
fn bind_child_to_parent<'a>(
    state: &'a crate::AppState,
    parent_run_id: &str,
    child_run_id: &str,
) -> Option<ChildTurnBinding<'a>> {
    let budgets = state.turn_budgets.lock().ok()?;
    let blocked = state.turn_blocked.lock().ok()?;
    let budget_cell = budgets.get(parent_run_id).cloned();
    let blocked_cell = blocked.get(parent_run_id).cloned();
    drop(budgets);
    drop(blocked);
    let budget_cell = budget_cell?;
    let blocked_cell = blocked_cell?;
    if let Ok(mut map) = state.turn_budgets.lock() {
        map.insert(child_run_id.to_string(), budget_cell);
    }
    if let Ok(mut map) = state.turn_blocked.lock() {
        map.insert(child_run_id.to_string(), blocked_cell);
    }
    Some(ChildTurnBinding {
        budgets: &state.turn_budgets,
        blocked: &state.turn_blocked,
        child_run_id: child_run_id.to_string(),
    })
}

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
    let capability_policy = crate::capabilities::ToolPolicy::guard(state.database());
    let owned = local_tools(snapshot, &capability_policy);
    let tools = merge_child_tools(owned, inherit, &request.tool_filter, &capability_policy);
    // Bind the child's run id to the parent's shared turn budget and
    // replay-protection set. A child runs inside its parent's turn, so it must
    // charge the same search/fetch counters and obey the same "never repeat a
    // call with an uncertain outcome" rule. Binding the child's id to the
    // parent's cells — rather than looking the child's own id up — also makes
    // nested children resolve to the same cells, so a grandchild cannot
    // escape the budget either.
    let _turn_binding = bind_child_to_parent(state, &request.parent_run_id, &child_run_id);
    let interrupt = state.subagents.register(&child_run_id);
    // Per-child watch for interrupts plus the app-wide (generation-scoped)
    // cancel: a parent's Stop interrupts its inline child promptly.
    let cancel = ChildCancel::new(
        interrupt,
        state.subagents.cancellation(&child_run_id),
        Some(state.cancel.subscribe()),
    );
    let answer = drive_child(
        state,
        snapshot,
        &request,
        tools,
        &backend,
        cancel,
        Some(state.subagents.clone()),
        &child_run_id,
        emit,
    )
    .await;
    state.subagents.retire(&child_run_id);
    match answer {
        Ok(answer) => {
            if let Ok(store) = state.database() {
                let _ = store.update_subagent_run(&record.id, "completed", None);
                let _ = store.update_run_status(
                    &child_run_id,
                    crate::agent_run::RunState::Completed,
                    None,
                    None,
                );
            }
            Ok((child_run_id, answer))
        }
        Err(error) => {
            let cancelled = error == INTERRUPTED || error.starts_with("Subagent was interrupted");
            if let Ok(store) = state.database() {
                let _ = store.update_subagent_run(
                    &record.id,
                    if cancelled { "interrupted" } else { "failed" },
                    Some(&error),
                );
                let _ = store.update_run_status(
                    &child_run_id,
                    if cancelled {
                        crate::agent_run::RunState::Cancelled
                    } else {
                        crate::agent_run::RunState::Failed
                    },
                    Some(&error),
                    None,
                );
            }
            Err(error)
        }
    }
}

/// Background child: supervisor task with its own SQLite connection and
/// local-only tools. Settles durably; the parent is notified via a tool-role
/// message plus the subagent_runs row.
pub fn spawn_background(
    snapshot: BackendSnapshot,
    request: ChildRequest,
) -> Result<String, String> {
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
    // Detached children get only the per-child interrupt watch — never the
    // app-wide generation cancel — so an unrelated Stop cannot kill a
    // background run that outlives its parent's turn.
    let cancel = ChildCancel::new(interrupt, registry.cancellation(&child_run_id), None);
    let task_child = child_run_id.clone();
    let task_registry = registry.clone();
    // Background tasks need an AppState for cancellation checks only; build a
    // minimal handle-free path by reusing drive pieces without &AppState.
    tauri::async_runtime::spawn(async move {
        let outcome = run_background_core(
            snapshot,
            request,
            cancel,
            task_registry.clone(),
            task_child.clone(),
        )
        .await;
        task_registry.retire(&task_child);
        let _ = outcome;
    });
    Ok(child_run_id)
}

async fn run_background_core(
    snapshot: BackendSnapshot,
    request: ChildRequest,
    cancel: ChildCancel,
    registry: Arc<SubagentRegistry>,
    child_run_id: String,
) -> Result<(), String> {
    let store = Arc::new(Mutex::new(crate::store::Store::open(&snapshot.db_path)?));
    let capability_policy = match store.lock() {
        Ok(store) => crate::capabilities::ToolPolicy::resolve(&store),
        Err(_) => {
            crate::capabilities::ToolPolicy::closed("Capability settings are unreachable.".into())
        }
    };
    let backend = snapshot.backend()?;
    let owned = local_tools(&snapshot, &capability_policy);
    let tools: Vec<&crate::connectors::AgentTool> = owned
        .iter()
        .filter(|tool| tool_allowed(&request.tool_filter, tool))
        .collect();
    // Drive without &AppState: cancellation comes from the interrupt watch.
    let answer = drive_detached(
        &snapshot,
        &request,
        tools,
        &backend,
        cancel,
        Some(registry),
        &child_run_id,
        store.clone(),
    )
    .await;
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
                let _ = s.update_run_status(
                    &child_run_id,
                    crate::agent_run::RunState::Completed,
                    None,
                    None,
                );
                let notice = serde_json::json!({
                    "request": {"connector": "Subagents", "name": "subagent", "arguments": {}, "decision": "allowed", "authorization": "background child settled"},
                    "result": {"childRunId": child_run_id, "status": "completed", "summary": answer.chars().take(1500).collect::<String>()},
                });
                let _ = s.append_message(
                    &request.conversation_id,
                    "tool",
                    &notice.to_string(),
                    "complete",
                );
            }
        }
        Err(error) => {
            let cancelled = error == INTERRUPTED || error.starts_with("Subagent was interrupted");
            if let Ok(s) = store.lock() {
                if let Some(row) = record {
                    let _ = s.update_subagent_run(
                        &row.id,
                        if cancelled { "interrupted" } else { "failed" },
                        Some(&error),
                    );
                }
                let _ = s.update_run_status(
                    &child_run_id,
                    if cancelled {
                        crate::agent_run::RunState::Cancelled
                    } else {
                        crate::agent_run::RunState::Failed
                    },
                    Some(&error),
                    None,
                );
                let notice = serde_json::json!({
                    "request": {"connector": "Subagents", "name": "subagent", "arguments": {}, "decision": "allowed", "authorization": "background child settled"},
                    "result": {"childRunId": child_run_id, "status": if cancelled { "interrupted" } else { "failed" }, "error": error},
                });
                let _ = s.append_message(
                    &request.conversation_id,
                    "tool",
                    &notice.to_string(),
                    "complete",
                );
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
    mut cancel: ChildCancel,
    registry: Option<Arc<SubagentRegistry>>,
    child_run_id: &str,
    store: Arc<Mutex<crate::store::Store>>,
) -> Result<String, String> {
    use crate::inference::InferenceProvider;
    validate_child_request(request)?;
    if cancel.is_set() {
        return Err("Subagent was interrupted before starting.".into());
    }
    let mut system = child_system_prompt(snapshot, request);
    system.push_str(&format!(
        "\nModel capability metadata (not independent verification): {}",
        backend.capability_report()
    ));
    // Structured disclosure: parent-authorized tools this background child
    // cannot serve — connectors/automations bound to the live session,
    // capability kills, filtered or unavailable runtimes. The child sees the
    // aliases honestly instead of silently missing them, and the run log
    // carries the same list for review.
    let limits = {
        let policy = match store.lock() {
            Ok(store) => crate::capabilities::ToolPolicy::resolve(&store),
            Err(_) => crate::capabilities::ToolPolicy::closed(
                "Capability settings are unreachable.".into(),
            ),
        };
        detached_tool_limits(snapshot, &tools, &policy)
    };
    let mut event_seq: u64 = 1;
    if !limits.is_empty() {
        let names: Vec<&str> = limits
            .iter()
            .filter_map(|limit| limit["alias"].as_str())
            .collect();
        system.push_str(&format!(
            "\nThis run executes in the background: these parent-authorized tools are not usable here — {}. Ask the parent for what you need instead.",
            names.iter().take(24).cloned().collect::<Vec<_>>().join(", ")
        ));
        if let Ok(s) = store.lock() {
            let _ = s.append_run_event(&crate::agent_run::RunEvent {
                run_id: child_run_id.to_string(),
                seq: event_seq,
                step_id: "setup".into(),
                tool_call_id: None,
                event_type: "child_tool_limits".into(),
                payload: serde_json::json!({"unavailable": limits}),
                created_at: crate::store::now(),
            });
            event_seq += 1;
        }
    }
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
    backend.prepare_preferences(&mut preferences)?;
    let mut answer = String::new();
    let max_rounds = request.max_rounds.min(8);
    for round in 0..max_rounds {
        if cancel.is_set() {
            if let Ok(s) = store.lock() {
                let _ = s.update_run_status(
                    child_run_id,
                    crate::agent_run::RunState::Cancelled,
                    Some("Interrupted."),
                    None,
                );
            }
            return Err(INTERRUPTED.into());
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
            payload["tools"] =
                serde_json::json!(crate::tool_discovery::advertised(tools.iter().copied())
                    .into_iter()
                    .map(|tool| tool.definition())
                    .collect::<Vec<_>>());
        }
        if let Some(schema) = &request.output_schema {
            backend.constrain_output(&mut payload, schema)?;
        }
        let payload = backend.payload(&payload)?;
        // The interrupt watch is selected against every blocking await so a
        // detached child stops promptly instead of out its deadman timeout.
        let _tokens = match tokio::select! {
            _ = cancel.cancelled() => return Err(INTERRUPTED.into()),
            result = tokio::time::timeout(Duration::from_secs(45), backend.check_context(&payload, preferences.max_tokens)) => result,
        } {
            Ok(result) => result?,
            Err(_) => return Err("Subagent context check timed out.".into()),
        };
        let response = match tokio::select! {
            _ = cancel.cancelled() => return Err(INTERRUPTED.into()),
            result = tokio::time::timeout(Duration::from_secs(600), backend.stream(&payload)) => result,
        } {
            Ok(result) => result?,
            Err(_) => return Err("Subagent model did not respond within ten minutes.".into()),
        };
        let outcome = collect_stream(backend, response, &mut answer, &mut cancel).await?;
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
        messages.push(outcome.assistant_message);
        for mut call in outcome.calls {
            if cancel.is_set() {
                return Err(INTERRUPTED.into());
            }
            let visible = crate::tool_discovery::advertised(tools.iter().copied())
                .into_iter()
                .any(|tool| tool.alias == call.name);
            let resolved = if visible {
                crate::tool_discovery::resolve(
                    tools.iter().copied(),
                    &call.name,
                    call.arguments.clone(),
                )
            } else {
                Err("Tool is deferred or unavailable. Use tool_search and tool_invoke for this child's authorized catalog.".into())
            };
            let (resolved_tool, arguments) = match resolved {
                Ok(value) => value,
                Err(error) => {
                    messages.push(serde_json::json!({"role":"tool","tool_call_id":call.id,"content":crate::tool_envelope::failure_notice("unavailable-tool",&error,false,crate::tool_envelope::SIDE_NONE,&serde_json::json!({})).to_string()}));
                    continue;
                }
            };
            let tool = &resolved_tool;
            call.name = tool.alias.clone();
            call.arguments = arguments;
            // Capability kills re-check at dispatch; stale offers deny visibly.
            let capability_denial = {
                let policy = match store.lock() {
                    Ok(store) => crate::capabilities::ToolPolicy::resolve(&store),
                    Err(_) => crate::capabilities::ToolPolicy::closed(
                        "Capability settings are unreachable.".into(),
                    ),
                };
                policy.denial_for_tool(tool)
            };
            if let Some(reason) = capability_denial {
                messages.push(serde_json::json!({"role":"tool","tool_call_id":call.id,
                    "content":crate::tool_envelope::failure_notice("capability-disabled", &reason, false, crate::tool_envelope::SIDE_NONE, &serde_json::json!({})).to_string()}));
                continue;
            }
            match tool.prepare_arguments(call.arguments.clone()) {
                Ok(arguments) => call.arguments = arguments,
                Err(error) => {
                    messages.push(serde_json::json!({"role":"tool","tool_call_id":call.id,"content":crate::tool_envelope::failure_notice("invalid-arguments", &error, false, crate::tool_envelope::SIDE_NONE, &serde_json::json!({})).to_string()}));
                    continue;
                }
            }
            let result = if !request.policy.allows(tool.trusted_read())
                || tool.requires_approval(&call.arguments)
            {
                crate::tool_envelope::failure_notice(
                    "denied",
                    "Denied by the subagent approval pin (reads-only).",
                    false,
                    crate::tool_envelope::SIDE_NONE,
                    &serde_json::json!({"denied":true}),
                )
            } else if tool.harness_name() == Some(crate::tool_discovery::SEARCH) {
                let policy = match store.lock() {
                    Ok(store) => crate::capabilities::ToolPolicy::resolve(&store),
                    Err(_) => crate::capabilities::ToolPolicy::closed(
                        "Capability settings unreachable.".into(),
                    ),
                };
                match crate::tool_discovery::search(tools.iter().copied(), &call.arguments, &policy)
                {
                    Ok(value) => crate::tool_envelope::normalize(value, tool.envelope_family()),
                    Err(error) => crate::tool_envelope::normalize_error(
                        &error,
                        crate::tool_envelope::Cause::default(),
                        tool.envelope_family(),
                    ),
                }
            } else if let Some(alias) = tool.harness_name() {
                // Background children have no AppState: serve the store-only
                // and workspace-bounded harness subset locally; everything
                // else fails loud. Awaited inside the cancel select so an
                // interrupt cannot strand behind a blocked read.
                tokio::select! {
                    _ = cancel.cancelled() => return Err(cancelled_tool_call_detached(&store, child_run_id, round, &call, &mut event_seq)),
                    result = harness_detached(snapshot, request, alias, call.arguments.clone()) => match result {
                        Ok(value) => crate::tool_envelope::normalize(value, tool.envelope_family()),
                        Err(error) => crate::tool_envelope::normalize_error(&error, crate::tool_envelope::Cause::default(), tool.envelope_family()),
                    },
                }
            } else {
                // Dropping the call future aborts in-flight work; managed
                // child processes die with the execution wrapper on drop.
                tokio::select! {
                    _ = cancel.cancelled() => return Err(cancelled_tool_call_detached(&store, child_run_id, round, &call, &mut event_seq)),
                    result = tokio::time::timeout(tool.timeout(), tool.call(call.arguments.clone())) => match result {
                        Ok(Ok(value)) => crate::tool_envelope::normalize(value, tool.envelope_family()),
                        Ok(Err(error)) => crate::tool_envelope::normalize_error(&error, crate::tool_envelope::Cause::default(), tool.envelope_family()),
                        Err(_) => crate::tool_envelope::normalize_error("Subagent tool call timed out. Its outcome may be unknown; do not automatically retry.", crate::tool_envelope::Cause { timeout: true, ..Default::default() }, tool.envelope_family()),
                    },
                }
            };
            let control = if tool.harness_name().is_some()
                && matches!(call.name.as_str(), "finish" | "update_context")
            {
                Some(result.clone())
            } else {
                None
            };
            let (bounded, maybe_artifact) = crate::artifacts::bound_tool_result(
                result,
                &tool.tool.name,
                &request.conversation_id,
                Some(child_run_id),
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
                if let Some(final_answer) = crate::arex::apply_control(
                    &call.name,
                    &control,
                    &*store.lock().map_err(|_| "Store lock poisoned")?,
                    &request.conversation_id,
                    child_run_id,
                    &mut messages,
                )? {
                    if let Some(schema) = &request.output_schema {
                        check_output_schema(schema, &final_answer)?;
                    }
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
    calls: Vec<crate::tool_calls::ToolCall>,
    assistant_message: serde_json::Value,
}

async fn collect_stream(
    backend: &crate::inference::Backend,
    response: reqwest::Response,
    answer: &mut String,
    cancel: &mut ChildCancel,
) -> Result<StreamOutcome, String> {
    let mut stream = response.bytes_stream();
    let mut decoder = crate::sse::SseDecoder::default();
    let mut protocol = crate::sse::ModelStream::for_backend(backend);
    let mut calls = crate::tool_calls::ToolCalls::default();
    let mut text = String::new();
    let mut finish = String::new();
    'stream: loop {
        // Interrupt wins over the per-chunk deadman: dropping the body stream
        // aborts the HTTP read rather than waiting out the 300s timeout.
        let next = tokio::select! {
            _ = cancel.cancelled() => return Err(INTERRUPTED.into()),
            next = tokio::time::timeout(Duration::from_secs(300), futures_util::StreamExt::next(&mut stream)) => next,
        }
        .map_err(|_| "Subagent model stopped responding.")?;
        let Some(bytes) = next else {
            decoder.finish()?;
            return Err("Subagent stream ended without a completion marker.".into());
        };
        for event in decoder.push(&bytes.map_err(|error| error.to_string())?)? {
            let value = protocol.push(&event)?;
            let Some(value) = value else {
                if protocol.done {
                    break 'stream;
                }
                continue;
            };
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
            if protocol.done {
                break 'stream;
            }
        }
    }
    if !matches!(finish.as_str(), "stop" | "tool_calls" | "length") {
        return Err(format!(
            "Subagent returned an unsupported finish reason: '{finish}'."
        ));
    }
    if calls.is_empty() && (text.contains("<tool_call>") || text.contains("<function=")) {
        let parsed = crate::tool_calls::parse_text_tool_calls(&text)
            .ok_or("Subagent produced an incomplete tool call.")?;
        calls = crate::tool_calls::ToolCalls::from_parsed(parsed);
        let visible = crate::tool_calls::strip_text_tool_calls(&text);
        answer.truncate(answer.len().saturating_sub(text.len()));
        answer.push_str(&visible);
        text = visible;
    }
    let calls = calls.finish()?;
    let assistant_message = protocol.assistant_message(&text, &calls);
    Ok(StreamOutcome {
        finish,
        calls,
        assistant_message,
    })
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
                label: step
                    .label
                    .clone()
                    .unwrap_or_else(|| format!("stage-{}", index + 1)),
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
            let inherited: Vec<crate::connectors::AgentTool> = inherit
                .iter()
                .filter(|tool| !tool.is_mcp())
                .cloned()
                .collect();
            match run_child_inline(state, snapshot, request.clone(), &inherited, None).await {
                Ok((child_run_id, answer)) => {
                    prior.push_str(&format!(
                        "\n[{}] {}\n",
                        request.label,
                        answer.chars().take(1000).collect::<String>()
                    ));
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
            label: step
                .label
                .clone()
                .unwrap_or_else(|| format!("branch-{}", index + 1)),
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
    let mut global_cancel = state.cancel.subscribe();
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
        // Wake on the app-wide cancel instead of sleeping out the whole 5s
        // poll interval; the loop head interrupts the children and exits.
        tokio::select! {
            _ = global_cancel.changed() => {},
            _ = tokio::time::sleep(Duration::from_secs(5)) => {},
        }
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
    if !value
        .get("status")?
        .as_str()
        .map(|status| ["complete", "blocked", "continue"].contains(&status))?
    {
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
        let inherited: Vec<crate::connectors::AgentTool> = inherit
            .iter()
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
                rounds
                    .push(serde_json::json!({"round": round, "status": "failed", "error": error}));
                break;
            }
        }
    }
    Ok(serde_json::json!({"objective": objective, "rounds": rounds, "final": handoff}))
}

// ---- Model-facing + UI IPC ----

#[tauri::command]
pub fn list_subagent_runs(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
) -> Result<Vec<SubagentRun>, String> {
    state
        .database()?
        .subagent_runs_for_conversation(&conversation_id)
}

#[tauri::command]
pub fn interrupt_subagent(
    state: tauri::State<'_, crate::AppState>,
    child_run_id: String,
) -> Result<bool, String> {
    if child_run_id.is_empty() || child_run_id.len() > 128 {
        return Err("Child run id must be 1-128 characters.".into());
    }
    Ok(state.subagents.interrupt(&child_run_id))
}

#[tauri::command]
pub fn list_subagent_models(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let store = state.database()?;
    list_models(&store)
}

/// Configured subagent model allowlist (empty = current conversation model).
pub fn list_models(store: &crate::store::Store) -> Result<Vec<serde_json::Value>, String> {
    let configured: Vec<serde_json::Value> = store.setting(MODELS_KEY).unwrap_or_default();
    for entry in &configured {
        if entry.get("providerId").is_none()
            || entry
                .get("modelId")
                .and_then(|value| value.as_str())
                .is_none()
        {
            return Err("subagents.models entries need providerId and modelId.".into());
        }
    }
    Ok(configured.into_iter().take(16).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn child_native_responses_stream_preserves_original_ids_context_and_terminal_output() {
        use crate::inference::InferenceProvider;
        use serde_json::json;
        let output = json!([
            {"type":"reasoning","id":"rs-child","summary":[],"encrypted_content":"opaque-child"},
            {"type":"function_call","id":"fc-child","call_id":"call-child","name":"tool","arguments":"{\"input\":\"child\"}","status":"completed"}
        ]);
        let final_output = json!([{"type":"message","id":"msg-child","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"Child done","annotations":[]}]}]);
        let wire = |output: serde_json::Value| {
            format!(
                "data: {}\r\n\r\n",
                json!({"type":"response.completed","response":{"status":"completed","output":output,"usage":{"input_tokens":30,"output_tokens":5}}})
            )
        };
        let (endpoint, requests, server) = crate::test_support::scripted_sse(vec![
            wire(output.clone()),
            wire(final_output.clone()),
        ])
        .await;
        let backend = crate::inference::Backend::OpenAi {
            responses: true,
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            base_url: endpoint,
            key: "fixture-responses-key".into(),
            model_id: "configured-model".into(),
            context_length: 8192,
            max_output_tokens: Some(1000),
            capabilities: crate::provider_capabilities::ModelCapabilities::legacy(true),
        };
        let mut messages = vec![json!({"role":"user","content":"Echo child"})];
        let request = backend
            .payload(&json!({"messages":messages,"stream":true}))
            .unwrap();
        let mut answer = String::new();
        let mut cancel = ChildCancel::none();
        let response = backend.stream(&request).await.unwrap();
        let first = collect_stream(&backend, response, &mut answer, &mut cancel)
            .await
            .unwrap();
        assert_eq!(first.finish, "tool_calls");
        assert_eq!(first.calls.len(), 1);
        assert_eq!(first.calls[0].id, "call-child");
        assert_eq!(first.calls[0].arguments, json!({"input":"child"}));
        assert_eq!(first.assistant_message["locallm_responses_output"], output);
        messages.push(first.assistant_message);
        messages.push(
            json!({"role":"tool","tool_call_id":"call-child","content":"{\"processed\":true}"}),
        );
        let second = backend
            .payload(&json!({"messages":messages,"stream":true}))
            .unwrap();
        let response = backend.stream(&second).await.unwrap();
        let result = collect_stream(&backend, response, &mut answer, &mut cancel)
            .await
            .unwrap();
        assert_eq!(result.finish, "stop");
        assert!(result.calls.is_empty());
        assert_eq!(answer, "Child done");
        assert_eq!(
            result.assistant_message["locallm_responses_output"],
            final_output
        );
        server.await.unwrap();
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        for request in requests.iter() {
            assert!(request.headers.starts_with("POST /v1/responses "));
            assert!(request
                .headers
                .to_lowercase()
                .contains("authorization: bearer fixture-responses-key"));
            assert_eq!(request.body["store"], false);
        }
        let input = requests[1].body["input"].as_array().unwrap();
        assert!(input
            .iter()
            .any(|item| item["encrypted_content"] == "opaque-child"));
        assert!(input
            .iter()
            .any(|item| item["type"] == "function_call_output" && item["call_id"] == "call-child"));
    }
    #[tokio::test]
    async fn child_streams_use_native_claude_completion_and_text_tool_fallback() {
        use crate::inference::InferenceProvider;
        use tokio::io::AsyncWriteExt;
        for claude in [true, false] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let wire = if claude {
                [
                    r#"{"type":"content_block_start","index":9,"content_block":{"type":"tool_use","id":"a","name":"clock","input":{}}}"#,
                    r#"{"type":"content_block_stop","index":9}"#,
                    r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
                    r#"{"type":"message_stop"}"#,
                ].iter().map(|event|format!("data: {event}\n\n")).collect::<String>()
            } else {
                let chunk = serde_json::json!({"choices":[{"delta":{"content":"Checking <function=clock>{}</function>"},"finish_reason":"stop"}]});
                format!("data: {chunk}\n\ndata: [DONE]\n\n")
            };
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{wire}",wire.len()).as_bytes()).await.unwrap();
                // Keep the request socket alive until the client has consumed
                // the body, rather than closing it with unread request bytes.
                use tokio::io::AsyncReadExt;
                let mut bytes = [0; 4096];
                let _ = stream.read(&mut bytes).await;
            });
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let backend = if claude {
                crate::inference::Backend::Claude {
                    client,
                    base_url: endpoint,
                    key: "test-key".into(),
                    model_id: "claude".into(),
                    context_length: 4096,
                    max_output_tokens: Some(512),
                    capabilities: crate::provider_capabilities::ModelCapabilities::legacy(true),
                }
            } else {
                crate::inference::Backend::Local {
                    client,
                    endpoint,
                    key: "test-key".into(),
                    context_length: 4096,
                }
            };
            let response = backend.stream(&serde_json::json!({"messages":[{"role":"user","content":"clock"}],"stream":true})).await.unwrap();
            let mut answer = String::new();
            let mut cancel = ChildCancel::none();
            let result = collect_stream(&backend, response, &mut answer, &mut cancel)
                .await
                .unwrap();
            assert_eq!(result.calls.len(), 1);
            assert_eq!(result.calls[0].name, "clock");
            assert_eq!(result.calls[0].arguments, serde_json::json!({}));
            assert!(!answer.contains("<function="));
            server.await.unwrap();
        }
    }
    #[test]
    fn depth_policy_and_filter_rules() {
        assert_eq!(
            ChildPolicy::from_access(crate::permissions::AccessMode::Ask),
            ChildPolicy::ReadsOnly
        );
        assert_eq!(
            ChildPolicy::from_access(crate::permissions::AccessMode::FullAccess),
            ChildPolicy::Full
        );
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
        for alias in [
            "todo_write",
            "todo_add",
            "todo_update",
            "goal_set",
            "goal_clear",
            "memory_teach",
            "memory_recall",
            "file_search",
            "file_read",
            "file_write",
            "list_agents",
            "list_subagent_models",
            "preset_guide",
            "artifact_read",
        ] {
            assert!(
                harness_detached_supported(alias),
                "{alias} should run detached"
            );
        }
        for alias in [
            "subagent",
            "terminal_send",
            "docker_exec",
            "workflow_run",
            "ask_user",
            "plan_submit",
            "ptc_run",
        ] {
            assert!(
                !harness_detached_supported(alias),
                "{alias} needs the foreground dispatcher"
            );
        }
    }
    #[test]
    fn child_requests_validate_loudly() {
        let base = ChildRequest {
            label: "x".into(),
            prompt: "do it".into(),
            depth: 0,
            max_rounds: 4,
            tool_filter: None,
            persona: None,
            output_schema: None,
            policy: ChildPolicy::ReadsOnly,
            parent_run_id: "p".into(),
            conversation_id: "c".into(),
        };
        assert!(validate_child_request(&base).is_ok());
        assert!(validate_child_request(&ChildRequest {
            prompt: "".into(),
            ..base.clone()
        })
        .is_err());
        assert!(validate_child_request(&ChildRequest {
            max_rounds: 9,
            ..base.clone()
        })
        .is_err());
        assert!(validate_child_request(&ChildRequest {
            output_schema: Some(serde_json::json!({"type": "object", "required": ["a"]})),
            ..base.clone()
        })
        .is_ok());
        // Schemas are compiled before inference: invalid or remote refs reject.
        for schema in [
            serde_json::json!({"type": "nonsense"}),
            serde_json::json!({"$ref": "https://schemas.example/remote.json"}),
            serde_json::json!({"properties": {"a": {"minimum": "NaN"}}}),
            serde_json::json!({"type": "array"}),
            serde_json::json!({"type": ["string", "integer"]}),
        ] {
            assert!(
                validate_child_request(&ChildRequest {
                    output_schema: Some(schema.clone()),
                    ..base.clone()
                })
                .is_err(),
                "{schema} must be rejected before inference"
            );
        }
        // Object-permitting unions and $defs schemas are accepted.
        for schema in [
            serde_json::json!({"type": ["object", "null"]}),
            serde_json::json!({"$defs": {"item": {"type": "string"}}, "type": "object", "properties": {"items": {"type": "array", "items": {"$ref": "#/$defs/item"}}}}),
        ] {
            assert!(
                validate_child_request(&ChildRequest {
                    output_schema: Some(schema.clone()),
                    ..base.clone()
                })
                .is_ok(),
                "{schema} should be accepted"
            );
        }
    }
    #[test]
    fn handoff_parsing_prefers_last_fence() {
        let answer = "first ```handoff\n{\"status\":\"continue\",\"summary\":\"a\"}\n``` then ```handoff\n{\"status\":\"blocked\",\"summary\":\"b\",\"blocker\":\"x\"}\n```";
        let handoff = parse_handoff(answer).unwrap();
        assert_eq!(handoff.status, "blocked");
        assert!(parse_handoff("no fence here").is_none());
        assert!(check_output_schema(&serde_json::json!({"required": ["a"]}), r#"{"a":1}"#).is_ok());
        assert!(
            check_output_schema(&serde_json::json!({"required": ["a"]}), r#"{"b":1}"#).is_err()
        );
        assert!(check_output_schema(&serde_json::json!({}), "not json").is_err());
    }
    #[test]
    fn output_schema_fully_enforces_nested_constraints() {
        use serde_json::json;
        let schema = json!({
            "type": "object",
            "required": ["answer", "confidence"],
            "properties": {
                "answer": {"type": "string", "minLength": 1},
                "confidence": {"type": "number", "minimum": 0, "maximum": 1},
                "kind": {"enum": ["research", "code"]},
                "nested": {"type": "object", "required": ["ok"], "properties": {"ok": {"type": "boolean"}}, "additionalProperties": false},
                "tags": {"type": "array", "maxItems": 3, "items": {"type": "string"}}
            },
            "additionalProperties": false
        });
        assert!(check_output_schema(
            &schema,
            r#"{"answer":"done","confidence":0.9,"kind":"code","nested":{"ok":true},"tags":["a"]}"#
        )
        .is_ok());
        for (label, answer) in [
            ("missing required", r#"{"answer":"done"}"#),
            (
                "nested wrong type",
                r#"{"answer":"done","confidence":0.9,"nested":{"ok":"yes"}}"#,
            ),
            (
                "enum violation",
                r#"{"answer":"done","confidence":0.9,"kind":"other"}"#,
            ),
            ("range violation", r#"{"answer":"done","confidence":1.5}"#),
            (
                "extra key",
                r#"{"answer":"done","confidence":0.9,"surprise":1}"#,
            ),
            (
                "nested extra key",
                r#"{"answer":"done","confidence":0.9,"nested":{"ok":true,"extra":1}}"#,
            ),
            (
                "array bound",
                r#"{"answer":"done","confidence":0.9,"tags":["a","b","c","d"]}"#,
            ),
            ("empty string", r#"{"answer":"","confidence":0.9}"#),
        ] {
            assert!(
                check_output_schema(&schema, answer).is_err(),
                "{label}: {answer}"
            );
        }
        // The object contract still holds even when the schema admits scalars.
        assert!(check_output_schema(&json!({}), r#"[1,2]"#).is_err());
        assert!(check_output_schema(&json!({"type": ["object", "array"]}), r#"[1]"#).is_err());
    }
    #[test]
    fn local_tools_honor_capability_kills() {
        let dir = tempfile::tempdir().unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let workspace = dir.path().to_str().unwrap().to_string();
        let snapshot = BackendSnapshot {
            model_capabilities: None,
            kind: "local".into(),
            endpoint: String::new(),
            key: String::new(),
            base_url: String::new(),
            api_format: String::new(),
            model_id: "m".into(),
            context_length: 4096,
            max_output_tokens: None,
            supports_tools: true,
            allowed_tool_aliases: vec![
                "workspace_list_files".into(),
                "workspace_read_file".into(),
                "workspace_create_file".into(),
                "workspace_create_directory".into(),
                "workspace_edit_file".into(),
                "local_run_code".into(),
                "system_time".into(),
            ],
            system_prompt: String::new(),
            temperature: 0.0,
            top_p: 1.0,
            max_tokens: 256,
            workspace_path: workspace,
            execution_config: crate::execution::ExecutionConfig::default(),
            active_skills: Vec::new(),
            db_path: store_dir.path().join("s.sqlite"),
            data_dir: store_dir.path().to_path_buf(),
            account_id: None,
            inference_slots: 1,
        };
        let store = crate::store::Store::open(&snapshot.db_path).unwrap();
        let policy = crate::capabilities::ToolPolicy::resolve(&store);
        let tools = local_tools(&snapshot, &policy);
        assert!(tools.iter().any(|tool| tool.connector == "Workspace"));
        assert!(tools.iter().any(|tool| tool.connector == "Local execution"));
        assert!(tools.iter().any(|tool| tool.alias == "system_time"));
        store
            .save_setting(
                crate::capabilities::DISABLED_KEY,
                &vec!["workspace_files".to_string(), "system_time".to_string()],
            )
            .unwrap();
        let tools = local_tools(&snapshot, &crate::capabilities::ToolPolicy::resolve(&store));
        assert!(
            !tools.iter().any(|tool| tool.connector == "Workspace"),
            "workspace tools must drop when killed"
        );
        assert!(!tools.iter().any(|tool| tool.alias == "system_time"));
        assert!(tools.iter().any(|tool| tool.connector == "Local execution"));
        // Dependency cascade: session_log kill removes the skills reader path.
        store
            .save_setting(
                crate::capabilities::DISABLED_KEY,
                &vec!["session_log".to_string()],
            )
            .unwrap();
        let policy = crate::capabilities::ToolPolicy::resolve(&store);
        assert!(policy.denial("skills_reader").is_some());
        // Corrupt settings deny everything gated.
        store
            .save_setting(crate::capabilities::DISABLED_KEY, &"corrupt")
            .unwrap();
        let tools = local_tools(&snapshot, &crate::capabilities::ToolPolicy::resolve(&store));
        assert!(
            tools.is_empty(),
            "closed policy must drop all gated tools: {}",
            tools
                .iter()
                .map(|tool| tool.alias.as_str())
                .collect::<Vec<_>>()
                .join(",")
        );
    }
    #[tokio::test]
    async fn detached_dispatch_honors_capability_kills() {
        use serde_json::json;
        let dir = tempfile::tempdir().unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let snapshot = BackendSnapshot {
            model_capabilities: None,
            kind: "local".into(),
            endpoint: String::new(),
            key: String::new(),
            base_url: String::new(),
            api_format: String::new(),
            model_id: "m".into(),
            context_length: 4096,
            max_output_tokens: None,
            supports_tools: true,
            allowed_tool_aliases: Vec::new(),
            system_prompt: String::new(),
            temperature: 0.0,
            top_p: 1.0,
            max_tokens: 256,
            workspace_path: dir.path().to_str().unwrap().to_string(),
            execution_config: crate::execution::ExecutionConfig::default(),
            active_skills: Vec::new(),
            db_path: store_dir.path().join("s.sqlite"),
            data_dir: store_dir.path().to_path_buf(),
            account_id: None,
            inference_slots: 1,
        };
        let store = crate::store::Store::open(&snapshot.db_path).unwrap();
        let conversation = store.create_conversation().unwrap();
        let request = ChildRequest {
            label: "x".into(),
            prompt: "do it".into(),
            depth: 0,
            max_rounds: 4,
            tool_filter: None,
            persona: None,
            output_schema: None,
            policy: ChildPolicy::ReadsOnly,
            parent_run_id: "p".into(),
            conversation_id: conversation.id,
        };
        assert!(
            harness_detached(&snapshot, &request, "list_agents", json!({}))
                .await
                .is_ok()
        );
        store
            .save_setting(
                crate::capabilities::DISABLED_KEY,
                &vec!["subagents".to_string()],
            )
            .unwrap();
        let error = harness_detached(&snapshot, &request, "list_agents", json!({}))
            .await
            .unwrap_err();
        assert!(error.contains("disabled"), "{error}");
        // Workspace capability gates the detached file_search alias.
        store
            .save_setting(
                crate::capabilities::DISABLED_KEY,
                &vec!["workspace_files".to_string()],
            )
            .unwrap();
        assert!(
            harness_detached(&snapshot, &request, "file_search", json!({"query": "x"}))
                .await
                .is_err()
        );
        // Ungated store-only tools keep working while kills are saved.
        assert!(
            harness_detached(&snapshot, &request, "goal_clear", json!({}))
                .await
                .is_ok()
        );
        // Corrupt kill switch fails closed even for capability-gated aliases.
        store
            .save_setting(crate::capabilities::DISABLED_KEY, &"corrupt")
            .unwrap();
        assert!(
            harness_detached(&snapshot, &request, "list_agents", json!({}))
                .await
                .is_err()
        );
        assert!(
            harness_detached(&snapshot, &request, "file_search", json!({"query": "x"}))
                .await
                .is_err()
        );
    }

    // ---- A02/A06 fixtures ----
    use std::path::Path;

    fn fixture_snapshot(
        workspace: &Path,
        store_dir: &Path,
        endpoint: &str,
        aliases: &[&str],
    ) -> BackendSnapshot {
        BackendSnapshot {
            model_capabilities: None,
            kind: "local".into(),
            endpoint: endpoint.into(),
            key: "fixture".into(),
            base_url: String::new(),
            api_format: String::new(),
            model_id: "fixture".into(),
            context_length: 8192,
            max_output_tokens: None,
            supports_tools: true,
            allowed_tool_aliases: aliases.iter().map(|alias| alias.to_string()).collect(),
            system_prompt: String::new(),
            temperature: 0.0,
            top_p: 1.0,
            max_tokens: 512,
            workspace_path: workspace.to_string_lossy().into_owned(),
            execution_config: crate::execution::ExecutionConfig::default(),
            active_skills: Vec::new(),
            db_path: store_dir.join("s.sqlite"),
            data_dir: store_dir.to_path_buf(),
            account_id: None,
            inference_slots: 1,
        }
    }

    fn fixture_request(policy: ChildPolicy, conversation_id: &str) -> ChildRequest {
        ChildRequest {
            label: "child".into(),
            prompt: "work".into(),
            depth: 0,
            max_rounds: 4,
            tool_filter: None,
            persona: None,
            output_schema: None,
            policy,
            parent_run_id: "parent".into(),
            conversation_id: conversation_id.into(),
        }
    }

    /// run_events rows reference runs(id); real callers save the run before
    /// driving, so fixtures do the same to exercise event writes honestly.
    fn fixture_run(store: &crate::store::Store, child_run_id: &str, conversation_id: &str) {
        let now = crate::store::now();
        store
            .save_run(&crate::agent_run::RunRecord {
                id: child_run_id.to_string(),
                conversation_id: conversation_id.to_string(),
                status: crate::agent_run::RunState::Preparing,
                model_provider: None,
                model_id: Some("fixture".into()),
                checkpoint: Some("subagent".into()),
                error: None,
                created_at: now,
                updated_at: now,
            })
            .unwrap();
    }

    /// What the fake runtime does with a `chat/completions` request.
    enum StreamStage {
        /// Hold the socket open without responding — request headers never arrive.
        Hang,
        /// Send SSE headers + one content chunk, then hold the body open.
        Dribble,
        /// Write a complete SSE reply then wait for the client to go away.
        Complete(String),
    }

    /// Minimal model-runtime fixture. `/v1/chat/completions/input_tokens`
    /// either answers `{"input_tokens":10}` or hangs (slow token count);
    /// `/v1/chat/completions` follows `stage`. Returns the endpoint plus a
    /// request counter tests wait on before interrupting, so cancellation is
    /// provably racing a specific blocked stage instead of a sleep.
    fn fake_model_server(
        count_stage: bool,
        stage: StreamStage,
    ) -> (
        String,
        tokio::task::JoinHandle<()>,
        tokio::sync::watch::Receiver<usize>,
    ) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (hits, hits_rx) = tokio::sync::watch::channel(0usize);
        let stage = std::sync::Arc::new(stage);
        let server = tokio::spawn(async move {
            let listener = tokio::net::TcpListener::from_std(listener).unwrap();
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let stage = stage.clone();
                let hits = hits.clone();
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 8192];
                    loop {
                        let Ok(n) = socket.read(&mut chunk).await else {
                            return;
                        };
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&chunk[..n]);
                        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                        if buf.len() > 65_536 {
                            return;
                        }
                    }
                    hits.send_modify(|count| *count += 1);
                    let request = String::from_utf8_lossy(&buf);
                    let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
                    if path.ends_with("/input_tokens") {
                        if count_stage {
                            let body = r#"{"input_tokens":10}"#;
                            let reply = format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                            let _ = socket.write_all(reply.as_bytes()).await;
                        } else {
                            // Slow token count: socket stays open, no reply.
                            std::future::pending::<()>().await;
                        }
                        return;
                    }
                    match &*stage {
                        StreamStage::Hang => std::future::pending::<()>().await,
                        StreamStage::Dribble => {
                            let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n";
                            let _ = socket.write_all(head.as_bytes()).await;
                            let _ = socket.write_all(b"data: {\"choices\":[{\"delta\":{\"content\":\"working\"}}]}\n\n").await;
                            // Mid-stream stall: the body never completes.
                            std::future::pending::<()>().await;
                        }
                        StreamStage::Complete(body) => {
                            let reply = format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                            let _ = socket.write_all(reply.as_bytes()).await;
                            let mut sink = [0u8; 4096];
                            let _ = socket.read(&mut sink).await;
                        }
                    }
                });
            }
        });
        (endpoint, server, hits_rx)
    }

    async fn wait_hits(hits: &mut tokio::sync::watch::Receiver<usize>, wanted: usize) {
        tokio::time::timeout(
            Duration::from_secs(10),
            hits.wait_for(|count| *count >= wanted),
        )
        .await
        .expect("fake model server did not receive the request")
        .unwrap();
    }

    // ---- A02: exact authorized parent alias reconstruction ----

    #[test]
    fn workspace_preview_is_foreground_only_even_with_a_workspace_prefix() {
        assert!(!detached_reconstructable("workspace_open_preview"));
        assert!(!harness_detached_supported("workspace_open_preview"));
        assert_eq!(
            detached_capability("workspace_open_preview"),
            Some("local_execution")
        );
    }

    #[test]
    fn local_tools_rebuild_exact_authorized_execution_aliases() {
        let workspace = tempfile::tempdir().unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&store_dir.path().join("s.sqlite")).unwrap();
        let policy = crate::capabilities::ToolPolicy::resolve(&store);
        if !crate::execution::LocalExecution::new(
            crate::execution::ExecutionConfig::default(),
            workspace.path().to_str().unwrap(),
        )
        .map(|execution| execution.code_available() && execution.command_available())
        .unwrap_or(false)
        {
            eprintln!("no interpreter/shell configured; skipping alias reconstruction assertions");
            return;
        }
        // Namespaced aliases only: nothing else is offered.
        let snapshot = fixture_snapshot(
            workspace.path(),
            store_dir.path(),
            "",
            &["local_run_code", "local_execute_command"],
        );
        let aliases: Vec<String> = local_tools(&snapshot, &policy)
            .iter()
            .map(|tool| tool.alias.clone())
            .collect();
        assert!(
            aliases.contains(&"local_run_code".to_string())
                && aliases.contains(&"local_execute_command".to_string())
        );
        assert!(
            !aliases.contains(&"run_code".to_string())
                && !aliases.contains(&"execute_command".to_string()),
            "only authorized spellings may be offered: {aliases:?}"
        );
        // Canonical harness spellings only: rebuilt verbatim, never widened.
        let snapshot = fixture_snapshot(
            workspace.path(),
            store_dir.path(),
            "",
            &["run_code", "execute_command"],
        );
        let tools = local_tools(&snapshot, &policy);
        let aliases: Vec<String> = tools.iter().map(|tool| tool.alias.clone()).collect();
        assert!(
            aliases.contains(&"run_code".to_string())
                && aliases.contains(&"execute_command".to_string())
        );
        assert!(
            !aliases.contains(&"local_run_code".to_string())
                && !aliases.contains(&"local_execute_command".to_string())
        );
        // Both spellings may be authorized at once; each keeps the live
        // Execution backend rather than a dead Harness stub.
        let snapshot = fixture_snapshot(
            workspace.path(),
            store_dir.path(),
            "",
            &[
                "run_code",
                "local_run_code",
                "execute_command",
                "local_execute_command",
            ],
        );
        let tools = local_tools(&snapshot, &policy);
        for alias in [
            "run_code",
            "local_run_code",
            "execute_command",
            "local_execute_command",
        ] {
            let tool = tools
                .iter()
                .find(|tool| tool.alias == alias)
                .unwrap_or_else(|| panic!("{alias} should be reconstructed"));
            assert_eq!(
                tool.connector, "Local execution",
                "{alias} must run through LocalExecution"
            );
        }
    }

    #[test]
    fn local_tools_preserve_always_ask_and_availability_gates() {
        let workspace = tempfile::tempdir().unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&store_dir.path().join("s.sqlite")).unwrap();
        let policy = crate::capabilities::ToolPolicy::resolve(&store);
        let mut snapshot = fixture_snapshot(
            workspace.path(),
            store_dir.path(),
            "",
            &["local_run_code", "local_execute_command"],
        );
        snapshot.execution_config.python.always_ask = true;
        snapshot.execution_config.bash.always_ask = true;
        let tools = local_tools(&snapshot, &policy);
        let run_code = tools
            .iter()
            .find(|tool| tool.alias == "local_run_code")
            .expect("run_code should reconstruct");
        let command = tools
            .iter()
            .find(|tool| tool.alias == "local_execute_command")
            .expect("execute_command should reconstruct");
        // The user's always-ask pin survives reconstruction verbatim.
        assert!(run_code
            .requires_approval(&serde_json::json!({"language": "python", "code": "print(1)"})));
        assert!(command.requires_approval(&serde_json::json!({"command": "ls"})));
        // Unavailable runtimes are never offered: point both at a path that
        // does not exist and confirm the tools disappear entirely.
        snapshot.execution_config.python_path = "/nonexistent/python3".into();
        snapshot.execution_config.node_path = "/nonexistent/node".into();
        snapshot.execution_config.powershell_path = "/nonexistent/pwsh".into();
        snapshot.execution_config.bash_path = "/nonexistent/bash".into();
        let tools = local_tools(&snapshot, &policy);
        assert!(
            tools.iter().all(|tool| tool.connector != "Local execution"),
            "unavailable runtimes must not be offered: {:?}",
            tools
                .iter()
                .map(|tool| tool.alias.clone())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn local_tools_rebuild_detached_safe_harness_aliases_and_report_limits() {
        let workspace = tempfile::tempdir().unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&store_dir.path().join("s.sqlite")).unwrap();
        let policy = crate::capabilities::ToolPolicy::resolve(&store);
        let authorized = [
            "file_read",
            "file_write",
            "file_search",
            "todo_write",
            "finish",
            "update_context",
            "list_agents",
            "subagent",
            "send_message",
            "interrupt_agent",
            "terminal_send",
            "ptc_run",
            "ask_user",
            "plan_submit",
            "acme_wiki_search_0123456789abcdef", // hashed MCP connector alias
        ];
        let snapshot = fixture_snapshot(workspace.path(), store_dir.path(), "", &authorized);
        let tools = local_tools(&snapshot, &policy);
        let aliases: Vec<String> = tools.iter().map(|tool| tool.alias.clone()).collect();
        for alias in [
            "file_read",
            "file_write",
            "file_search",
            "todo_write",
            "finish",
            "update_context",
            "list_agents",
        ] {
            let tool = tools
                .iter()
                .find(|tool| tool.alias == alias)
                .unwrap_or_else(|| panic!("{alias} should be reconstructed"));
            assert!(
                tool.is_harness(),
                "{alias} must dispatch through the detached harness handler"
            );
        }
        for alias in [
            "subagent",
            "send_message",
            "interrupt_agent",
            "terminal_send",
            "ptc_run",
            "ask_user",
            "plan_submit",
            "acme_wiki_search_0123456789abcdef",
        ] {
            assert!(
                !aliases.contains(&alias.to_string()),
                "{alias} needs the parent's live session and must not be reconstructed"
            );
        }
        // Structured limitation disclosure: session-bound tools report the
        // real reason, never a false "unavailable" claim.
        let refs: Vec<&crate::connectors::AgentTool> = tools.iter().collect();
        let limits = detached_tool_limits(&snapshot, &refs, &policy);
        let reason = |alias: &str| {
            limits
                .iter()
                .find(|limit| limit["alias"] == alias)
                .map(|limit| limit["reason"].as_str().unwrap().to_string())
        };
        assert_eq!(
            reason("acme_wiki_search_0123456789abcdef").as_deref(),
            Some("needs the parent's live session (connector, automation or orchestration)")
        );
        assert_eq!(
            reason("terminal_send").as_deref(),
            Some("needs the parent's live session (connector, automation or orchestration)")
        );
        assert!(reason("file_read").is_none() && reason("system_time").is_none());
        // Capability kills are reported distinctly, not as session limits.
        store
            .save_setting(
                crate::capabilities::DISABLED_KEY,
                &vec!["workspace_files".to_string()],
            )
            .unwrap();
        let policy = crate::capabilities::ToolPolicy::resolve(&store);
        let tools = local_tools(&snapshot, &policy);
        let refs: Vec<&crate::connectors::AgentTool> = tools.iter().collect();
        assert!(refs.iter().all(|tool| !matches!(
            tool.alias.as_str(),
            "file_read" | "file_write" | "file_search"
        )));
        let limits = detached_tool_limits(&snapshot, &refs, &policy);
        let reason = |alias: &str| {
            limits
                .iter()
                .find(|limit| limit["alias"] == alias)
                .map(|limit| limit["reason"].as_str().unwrap().to_string())
        };
        assert_eq!(
            reason("file_search").as_deref(),
            Some("disabled in Capability settings")
        );
    }

    #[test]
    fn merge_dedups_foreground_catalog_by_alias_and_keeps_borrowed_peers() {
        let workspace = tempfile::tempdir().unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&store_dir.path().join("s.sqlite")).unwrap();
        let policy = crate::capabilities::ToolPolicy::resolve(&store);
        let snapshot = fixture_snapshot(
            workspace.path(),
            store_dir.path(),
            "",
            &["local_run_code", "system_time", "todo_write"],
        );
        let owned = local_tools(&snapshot, &policy);
        assert!(owned.iter().any(|tool| tool.alias == "local_run_code"));
        // Parent catalog: live borrowed instances — same aliases plus a
        // foreground-only harness peer the child could never rebuild.
        let execution = crate::execution::LocalExecution::new(
            snapshot.execution_config.clone(),
            &snapshot.workspace_path,
        )
        .unwrap();
        let inherit = vec![
            std::sync::Arc::new(execution).tool(),
            crate::connectors::AgentTool::system_time(),
            crate::connectors::AgentTool::harness("send_message").unwrap(),
        ];
        let merged = merge_child_tools(owned.clone(), &inherit, &None, &policy);
        let aliases: Vec<&str> = merged
            .iter()
            .map(|tool| tool.tool().alias.as_str())
            .collect();
        assert_eq!(
            aliases
                .iter()
                .filter(|alias| **alias == "local_run_code")
                .count(),
            1,
            "borrowed/local duplicates must collapse"
        );
        assert_eq!(
            aliases
                .iter()
                .filter(|alias| **alias == "system_time")
                .count(),
            1
        );
        // The live borrowed tool wins the collision (it is the authorized
        // instance); owned reconstruction fills only genuine gaps.
        let run_code = merged
            .iter()
            .find(|tool| tool.tool().alias == "local_run_code")
            .unwrap();
        assert!(matches!(run_code, ChildTool::Borrowed(_)));
        // Foreground-only borrowed peers are kept, not degraded.
        assert!(merged
            .iter()
            .any(|tool| tool.tool().alias == "send_message"
                && matches!(tool, ChildTool::Borrowed(_))));
        // Capability kills and tool filters still apply to the merged set.
        store
            .save_setting(
                crate::capabilities::DISABLED_KEY,
                &vec!["local_execution".to_string()],
            )
            .unwrap();
        let policy = crate::capabilities::ToolPolicy::resolve(&store);
        let merged = merge_child_tools(local_tools(&snapshot, &policy), &inherit, &None, &policy);
        assert!(
            !merged
                .iter()
                .any(|tool| tool.tool().alias == "local_run_code"),
            "killed source must drop at merge too"
        );
        let merged = merge_child_tools(
            owned.clone(),
            &inherit,
            &Some(vec!["harness".to_string()]),
            &crate::capabilities::ToolPolicy::resolve(&store),
        );
        let filtered: Vec<&str> = merged
            .iter()
            .map(|tool| tool.tool().alias.as_str())
            .collect();
        assert_eq!(
            filtered,
            vec!["send_message", "todo_write"],
            "only harness-family tools survive the filter: {filtered:?}"
        );
    }

    // ---- A06: responsive cancellation at every blocking stage ----

    #[tokio::test]
    async fn detached_child_interrupt_during_token_count() {
        let workspace = tempfile::tempdir().unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let (endpoint, server, mut hits) =
            fake_model_server(false /* count hangs */, StreamStage::Hang);
        let snapshot = fixture_snapshot(workspace.path(), store_dir.path(), &endpoint, &[]);
        let store = std::sync::Arc::new(std::sync::Mutex::new(
            crate::store::Store::open(&snapshot.db_path).unwrap(),
        ));
        let request = fixture_request(
            ChildPolicy::Full,
            &store.lock().unwrap().create_conversation().unwrap().id,
        );
        let registry = global_registry();
        let child_run_id = format!("child-{}", uuid::Uuid::new_v4());
        let flag = registry.register(&child_run_id);
        let cancel = ChildCancel::new(flag, registry.cancellation(&child_run_id), None);
        let interrupt_id = child_run_id.clone();
        let (snap, req) = (snapshot.clone(), request.clone());
        let task = tokio::spawn({
            let registry = registry.clone();
            async move {
                let backend = snap.backend().unwrap();
                drive_detached(
                    &snap,
                    &req,
                    Vec::new(),
                    &backend,
                    cancel,
                    Some(registry),
                    &child_run_id,
                    store,
                )
                .await
            }
        });
        wait_hits(&mut hits, 1).await; // parked inside the token count
        assert!(registry.interrupt(&interrupt_id));
        let outcome = tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            outcome.unwrap_err(),
            INTERRUPTED,
            "interrupt must win over the 15s token-count timeout"
        );
        registry.retire(&interrupt_id);
        server.abort();
    }

    #[tokio::test]
    async fn detached_child_interrupt_while_waiting_for_headers() {
        let workspace = tempfile::tempdir().unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        // Token count answers; the completion request never gets headers.
        let (endpoint, server, mut hits) = fake_model_server(true, StreamStage::Hang);
        let snapshot = fixture_snapshot(workspace.path(), store_dir.path(), &endpoint, &[]);
        let store = std::sync::Arc::new(std::sync::Mutex::new(
            crate::store::Store::open(&snapshot.db_path).unwrap(),
        ));
        let request = fixture_request(
            ChildPolicy::Full,
            &store.lock().unwrap().create_conversation().unwrap().id,
        );
        let registry = global_registry();
        let child_run_id = format!("child-{}", uuid::Uuid::new_v4());
        let flag = registry.register(&child_run_id);
        let cancel = ChildCancel::new(flag, registry.cancellation(&child_run_id), None);
        let interrupt_id = child_run_id.clone();
        let (snap, req) = (snapshot.clone(), request.clone());
        let task = tokio::spawn({
            let registry = registry.clone();
            async move {
                let backend = snap.backend().unwrap();
                drive_detached(
                    &snap,
                    &req,
                    Vec::new(),
                    &backend,
                    cancel,
                    Some(registry),
                    &child_run_id,
                    store,
                )
                .await
            }
        });
        wait_hits(&mut hits, 2).await; // count done; parked on response headers
        assert!(registry.interrupt(&interrupt_id));
        let outcome = tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            outcome.unwrap_err(),
            INTERRUPTED,
            "interrupt must win over the ten-minute header wait"
        );
        registry.retire(&interrupt_id);
        server.abort();
    }

    #[tokio::test]
    async fn detached_child_interrupt_mid_stream() {
        let workspace = tempfile::tempdir().unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        // Headers + one chunk arrive, then the body stalls forever.
        let (endpoint, server, mut hits) = fake_model_server(true, StreamStage::Dribble);
        let snapshot = fixture_snapshot(workspace.path(), store_dir.path(), &endpoint, &[]);
        let store = std::sync::Arc::new(std::sync::Mutex::new(
            crate::store::Store::open(&snapshot.db_path).unwrap(),
        ));
        let request = fixture_request(
            ChildPolicy::Full,
            &store.lock().unwrap().create_conversation().unwrap().id,
        );
        let registry = global_registry();
        let child_run_id = format!("child-{}", uuid::Uuid::new_v4());
        let flag = registry.register(&child_run_id);
        let cancel = ChildCancel::new(flag, registry.cancellation(&child_run_id), None);
        let interrupt_id = child_run_id.clone();
        let (snap, req) = (snapshot.clone(), request.clone());
        let task = tokio::spawn({
            let registry = registry.clone();
            async move {
                let backend = snap.backend().unwrap();
                drive_detached(
                    &snap,
                    &req,
                    Vec::new(),
                    &backend,
                    cancel,
                    Some(registry),
                    &child_run_id,
                    store,
                )
                .await
            }
        });
        wait_hits(&mut hits, 2).await;
        // Give collect_stream a beat to consume the chunk and park on the
        // stalled body; the interrupt must not wait out the 300s deadman.
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(registry.interrupt(&interrupt_id));
        let outcome = tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            outcome.unwrap_err(),
            INTERRUPTED,
            "interrupt must win over the per-chunk stream timeout"
        );
        registry.retire(&interrupt_id);
        server.abort();
    }

    #[tokio::test]
    async fn detached_child_interrupt_during_tool_await() {
        let workspace = tempfile::tempdir().unwrap();
        let store_dir = tempfile::tempdir().unwrap();
        let executable = crate::execution::LocalExecution::new(
            crate::execution::ExecutionConfig::default(),
            workspace.path().to_str().unwrap(),
        )
        .map(|execution| execution.code_available())
        .unwrap_or(false);
        if !executable {
            eprintln!("no interpreter configured; skipping detached tool-await test");
            return;
        }
        // Round 1: token count ok, then a complete `local_run_code` tool call
        // that sleeps far beyond the test's patience.
        let args =
            serde_json::json!({"language": "python", "code": "import time\ntime.sleep(120)"})
                .to_string();
        let sse = format!(
            "data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{{\"name\":\"local_run_code\",\"arguments\":{}}}}}]}}}}]}}\n\ndata: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"tool_calls\"}}]}}\n\ndata: [DONE]\n\n",
            serde_json::json!(args)
        );
        let (endpoint, server, mut hits) = fake_model_server(true, StreamStage::Complete(sse));
        let snapshot = fixture_snapshot(
            workspace.path(),
            store_dir.path(),
            &endpoint,
            &["local_run_code"],
        );
        let store = std::sync::Arc::new(std::sync::Mutex::new(
            crate::store::Store::open(&snapshot.db_path).unwrap(),
        ));
        let request = fixture_request(
            ChildPolicy::Full,
            &store.lock().unwrap().create_conversation().unwrap().id,
        );
        let policy = crate::capabilities::ToolPolicy::resolve(&store.lock().unwrap());
        let owned = local_tools(&snapshot, &policy);
        assert!(owned.iter().any(|tool| tool.alias == "local_run_code"));
        let child_run_id = format!("child-{}", uuid::Uuid::new_v4());
        fixture_run(
            &store.lock().unwrap(),
            &child_run_id,
            &request.conversation_id,
        );
        let registry = global_registry();
        let flag = registry.register(&child_run_id);
        let cancel = ChildCancel::new(flag, registry.cancellation(&child_run_id), None);
        let interrupt_id = child_run_id.clone();
        let (snap, req) = (snapshot.clone(), request.clone());
        let task = tokio::spawn({
            let registry = registry.clone();
            let store = store.clone();
            async move {
                let backend = snap.backend().unwrap();
                let tools: Vec<&crate::connectors::AgentTool> = owned.iter().collect();
                drive_detached(
                    &snap,
                    &req,
                    tools,
                    &backend,
                    cancel,
                    Some(registry),
                    &child_run_id,
                    store,
                )
                .await
            }
        });
        wait_hits(&mut hits, 2).await;
        // The tool call is running its 120s sleep; interrupt and expect the
        // run to drop the execution future (KillOnDrop) instead of waiting.
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(registry.interrupt(&interrupt_id));
        let outcome = tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            outcome.unwrap_err(),
            INTERRUPTED,
            "interrupt must win over a long-running tool"
        );
        // The abandoned call is audited as cancelled/unknown, never replayed.
        let events = store.lock().unwrap().run_events(&interrupt_id).unwrap();
        let cancelled = events
            .iter()
            .find(|event| event.event_type == "tool_cancelled")
            .expect("a tool_cancelled event should be recorded");
        assert_eq!(cancelled.payload["cancelled"], serde_json::json!(true));
        assert!(cancelled.payload["message"]
            .as_str()
            .unwrap()
            .contains("side effects are unknown"));
        registry.retire(&interrupt_id);
        server.abort();
    }

    #[tokio::test]
    async fn foreground_child_interrupt_wakes_token_count_and_stop_watch() {
        for via_global_stop in [false, true] {
            let workspace = tempfile::tempdir().unwrap();
            let state_dir = tempfile::tempdir().unwrap();
            let state = std::sync::Arc::new(crate::test_support::app_state(state_dir.path()));
            // Slow token count — the foreground path shares the same watch selects.
            let (endpoint, server, mut hits) = fake_model_server(false, StreamStage::Hang);
            let mut snapshot =
                crate::test_support::snapshot(&state, workspace.path(), &["system_time"]);
            snapshot.endpoint = endpoint;
            let conversation = state.database().unwrap().create_conversation().unwrap();
            let request = fixture_request(ChildPolicy::ReadsOnly, &conversation.id);
            let child_run_id = format!("child-{}", uuid::Uuid::new_v4());
            let flag = state.subagents.register(&child_run_id);
            let cancel = ChildCancel::new(
                flag,
                state.subagents.cancellation(&child_run_id),
                Some(state.cancel.subscribe()),
            );
            let interrupt_id = child_run_id.clone();
            let task = tokio::spawn({
                let state = state.clone();
                let registry = state.subagents.clone();
                let snapshot = snapshot.clone();
                let request = request.clone();
                async move {
                    let backend = snapshot.backend().unwrap();
                    drive_child(
                        state.as_ref(),
                        &snapshot,
                        &request,
                        Vec::new(),
                        &backend,
                        cancel,
                        Some(registry),
                        &child_run_id,
                        None,
                    )
                    .await
                }
            });
            wait_hits(&mut hits, 1).await;
            if via_global_stop {
                // The parent's generation-scoped Stop reaches the same child watch.
                state.cancel.send_replace(true);
            } else {
                assert!(state.subagents.interrupt(&interrupt_id));
            }
            let outcome = tokio::time::timeout(Duration::from_secs(10), task)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(outcome.unwrap_err(), INTERRUPTED);
            state.subagents.retire(&interrupt_id);
            state.cancel.send_replace(false);
            server.abort();
        }
    }

    #[tokio::test]
    async fn foreground_child_interrupt_during_tool_await() {
        let workspace = tempfile::tempdir().unwrap();
        let state_dir = tempfile::tempdir().unwrap();
        let state = std::sync::Arc::new(crate::test_support::app_state(state_dir.path()));
        let executable = crate::execution::LocalExecution::new(
            crate::execution::ExecutionConfig::default(),
            workspace.path().to_str().unwrap(),
        )
        .map(|execution| execution.code_available())
        .unwrap_or(false);
        if !executable {
            eprintln!("no interpreter configured; skipping foreground tool-await test");
            return;
        }
        let args =
            serde_json::json!({"language": "python", "code": "import time\ntime.sleep(120)"})
                .to_string();
        let sse = format!(
            "data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{{\"name\":\"local_run_code\",\"arguments\":{}}}}}]}}}}]}}\n\ndata: {{\"choices\":[{{\"delta\":{{}},\"finish_reason\":\"tool_calls\"}}]}}\n\ndata: [DONE]\n\n",
            serde_json::json!(args)
        );
        let (endpoint, server, mut hits) = fake_model_server(true, StreamStage::Complete(sse));
        let mut snapshot =
            crate::test_support::snapshot(&state, workspace.path(), &["local_run_code"]);
        snapshot.endpoint = endpoint;
        let conversation = state.database().unwrap().create_conversation().unwrap();
        let request = fixture_request(ChildPolicy::Full, &conversation.id);
        let policy = crate::capabilities::ToolPolicy::guard(state.database());
        let owned = local_tools(&snapshot, &policy);
        let tools: Vec<ChildTool> = merge_child_tools(owned, &[], &request.tool_filter, &policy);
        let child_run_id = format!("child-{}", uuid::Uuid::new_v4());
        fixture_run(&state.database().unwrap(), &child_run_id, &conversation.id);
        let flag = state.subagents.register(&child_run_id);
        let cancel = ChildCancel::new(
            flag,
            state.subagents.cancellation(&child_run_id),
            Some(state.cancel.subscribe()),
        );
        let interrupt_id = child_run_id.clone();
        let task = tokio::spawn({
            let state = state.clone();
            let registry = state.subagents.clone();
            async move {
                let backend = snapshot.backend().unwrap();
                drive_child(
                    state.as_ref(),
                    &snapshot,
                    &request,
                    tools,
                    &backend,
                    cancel,
                    Some(registry),
                    &child_run_id,
                    None,
                )
                .await
            }
        });
        wait_hits(&mut hits, 2).await;
        tokio::time::sleep(Duration::from_millis(500)).await; // tool is in its 120s sleep
        assert!(state.subagents.interrupt(&interrupt_id));
        let outcome = tokio::time::timeout(Duration::from_secs(10), task)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            outcome.unwrap_err(),
            INTERRUPTED,
            "interrupt must drop the in-flight tool future"
        );
        let events = state.database().unwrap().run_events(&interrupt_id).unwrap();
        assert!(
            events
                .iter()
                .any(|event| event.event_type == "tool_cancelled"),
            "the abandoned tool call must be audited: {events:?}"
        );
        state.subagents.retire(&interrupt_id);
        server.abort();
    }

    #[tokio::test]
    async fn child_cancel_watch_scopes_to_generation_and_survives_late_subscribe() {
        let flag = std::sync::Arc::new(AtomicBool::new(false));
        // An old Stop committed before this child subscribed must not fire:
        // begin_generation reset the channel, so the fresh receiver sees false.
        let (stop, _) = tokio::sync::watch::channel(false);
        stop.send_replace(true);
        stop.send_replace(false); // begin_generation equivalent
        let cancel = ChildCancel::new(flag.clone(), None, Some(stop.subscribe()));
        assert!(
            !cancel.is_set(),
            "a stale stop must not cancel a new run's child"
        );
        stop.send_replace(true); // this generation's own Stop does fire
        assert!(cancel.is_set());
        // Interrupt recorded before the receiver subscribes still wakes it —
        // the watch stores the value, so no lost wakeup.
        let registry = SubagentRegistry::new();
        let flag = registry.register("child-late");
        assert!(registry.interrupt("child-late"));
        let receiver = registry.cancellation("child-late");
        let cancel = ChildCancel::new(flag, receiver, None);
        assert!(
            cancel.is_set(),
            "interrupt before subscribe must still be observed"
        );
        // Interrupting one child does not touch another.
        let other = registry.register("child-other");
        let other = ChildCancel::new(other, registry.cancellation("child-other"), None);
        assert!(!other.is_set());
        registry.retire("child-late");
        assert!(registry.cancellation("child-late").is_none());
    }
}
