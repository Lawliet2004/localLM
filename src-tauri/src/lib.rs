pub mod agent_run;
mod api_tools;
pub mod approval;
mod arex;
pub mod artifacts;
mod attachments;
mod automation;
pub mod capabilities;
mod chat;
mod check_tools;
mod claude_adapter;
mod commands;
pub mod compaction;
mod computer_use;
mod connectors;
mod context;
pub mod continuation;
mod database_rehearsal;
mod database_tools;
pub mod daytona;
pub mod daytona_cleanup;
pub mod daytona_execution;
pub mod daytona_journal;
mod daytona_settings;
mod debug_tools;
mod development_credentials;
mod development_tools;
mod document_index;
pub mod download;
mod edit_match;
mod editor;
pub mod error;
mod execution;
mod export;
mod findings;
mod formatter;
pub mod gguf;
mod git_tools;
mod hardware;
pub mod harness;
mod history;
mod hooks;
mod http_client;
mod impact_tools;
mod inference;
mod inference_retry;
mod inspector_browser;
mod install_recovery;
mod instructions;
mod latency_diagnostics;
pub mod local_mcp_config;
pub mod local_mcp_process;
pub mod local_only;
mod lsp_client;
mod lsp_tools;
mod managed_storage;
mod mcp_http;
mod mcp_input;
mod mcp_protocol;
pub mod memory;
mod model_catalog;
mod model_install;
mod model_library;
mod oauth;
mod openapi_tools;
mod performance_tools;
mod permission_rules;
mod permissions;
pub mod plans;
mod plugins;
mod presentation;
mod presentation_store;
mod presets;
mod process_tools;
mod profile_tools;
mod prompt;
mod provider_capabilities;
mod providers;
mod ptc;
#[cfg(test)]
mod ptc_contract_regressions;
mod remote_database;
mod research_tasks;
mod responses_adapter;
mod rolling_window;
mod runtime;
pub mod runtime_archive;
mod runtime_config;
pub mod runtime_install;
mod runtime_install_commands;
mod runtime_inventory;
mod runtime_log;
pub mod sandbox;
mod semantic_edits;
mod semantic_impact;
pub mod session_tools;
pub mod sessions;
pub mod skills;
pub mod snapshots;
mod source_inventory;
mod sse;
mod store;
pub mod subagents;
pub mod subscription_auth;
pub mod system_tools;
pub mod tasks;
pub mod taste;
mod terminal_ui;
#[cfg(test)]
mod test_support;
mod tool_calls;
pub mod tool_compat;
#[cfg(test)]
mod tool_contract_regressions;
mod tool_discovery;
mod tool_envelope;
pub mod tool_spec;
mod tool_validation;
mod vault;
mod verification_observer;
mod verification_tools;
pub mod weather;
pub mod web_search;
mod workspace;
mod workspace_preview;
mod workspace_ui;

use std::sync::Mutex;
use tauri::Manager;

/// The gate between model turns and exclusive operations.
///
/// A model turn holds a **shared** lease, so several conversations can run at
/// once. An exclusive operation — a provider edit, a credential change, a
/// model install — takes the **exclusive** lease, so it cannot run while a
/// turn is in flight, and a turn cannot start while one is.
///
/// The exclusive API is unchanged (`try_lock`), so every existing caller keeps
/// its behaviour and its message. What changes is that a turn no longer takes
/// the exclusive lease: it used to, which serialised the whole application
/// behind a single model operation.
pub struct OperationGate(tokio::sync::RwLock<()>);

/// The error an exclusive or shared lease reports when the gate is busy.
pub type TryLockError = tokio::sync::TryLockError;

impl OperationGate {
    /// Take the exclusive lease, or report that a turn or another exclusive
    /// operation is in progress.
    pub fn try_lock(&self) -> Result<tokio::sync::RwLockWriteGuard<'_, ()>, TryLockError> {
        self.0.try_write()
    }
    /// Take the exclusive lease, waiting for it.
    pub async fn lock(&self) -> tokio::sync::RwLockWriteGuard<'_, ()> {
        self.0.write().await
    }
    /// Take the shared lease a model turn holds for its duration.
    ///
    /// It fails fast while an exclusive operation holds the gate, so a turn
    /// never queues behind one.
    pub fn try_shared(&self) -> Result<tokio::sync::RwLockReadGuard<'_, ()>, TryLockError> {
        self.0.try_read()
    }
}

impl Default for OperationGate {
    fn default() -> Self {
        Self(tokio::sync::RwLock::new(()))
    }
}

/// One model turn at a time per conversation.
///
/// Different conversations run in parallel; the same conversation does not.
/// The per-conversation entry is removed when the turn ends, so the map does
/// not grow with the number of conversations ever used.
pub struct TurnLocks {
    locks:
        std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>,
}

impl Default for TurnLocks {
    fn default() -> Self {
        Self {
            locks: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }
}

/// The held turn lock, released when the guard is dropped.
pub struct TurnGuard<'a> {
    locks: &'a TurnLocks,
    conversation_id: String,
    guard: Option<tokio::sync::OwnedMutexGuard<()>>,
}

impl Drop for TurnGuard<'_> {
    fn drop(&mut self) {
        self.guard.take();
        if let Ok(mut locks) = self.locks.locks.lock() {
            if let Some(lock) = locks.get(&self.conversation_id) {
                // Only the map still holds the lock: no other turn is waiting
                // for it, so the entry can go.
                if std::sync::Arc::strong_count(lock) == 1 {
                    locks.remove(&self.conversation_id);
                }
            }
        }
    }
}

impl TurnLocks {
    /// Take the turn lock for one conversation.
    pub fn acquire(&self, conversation_id: &str) -> Result<TurnGuard<'_>, String> {
        let lock = {
            let mut locks = self
                .locks
                .lock()
                .map_err(|_| "Turn state is unavailable.".to_string())?;
            locks
                .entry(conversation_id.to_string())
                .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
                .clone()
        };
        let guard = lock.try_lock_owned().map_err(|_| {
            "Another model operation is in progress in this conversation.".to_string()
        })?;
        Ok(TurnGuard {
            locks: self,
            conversation_id: conversation_id.to_string(),
            guard: Some(guard),
        })
    }
}

pub struct AppState {
    automation: std::sync::Arc<automation::Manager>,
    installation_operation: tokio::sync::Mutex<()>,
    runtime_installer: model_install::Installer,
    model_installer: model_install::Installer,
    daytona_vault: std::sync::Arc<vault::Vault>,
    daytona_operation: std::sync::Arc<tokio::sync::Mutex<()>>,
    daytona_journal: std::sync::Arc<Mutex<daytona_journal::Journal>>,
    skills: tokio::sync::Mutex<skills::Skills>,
    approvals: approval::Approvals,
    pub ask_user: approval::ChoiceApprovals,
    /// Typed MCP input (form elicitation / MRTR) registry shared by every
    /// connector session; resolved only by the dedicated UI command.
    pub mcp_inputs: std::sync::Arc<mcp_input::Registry>,
    store: Mutex<store::Store>,
    runtime: tokio::sync::Mutex<runtime::Runtime>,
    load_cancel: tokio::sync::watch::Sender<bool>,
    load_status: std::sync::Arc<Mutex<Option<runtime::RuntimeStatus>>>,
    /// The gate between model turns and exclusive operations.
    operation: OperationGate,
    /// One model turn at a time per conversation.
    turns: TurnLocks,
    cancel: tokio::sync::watch::Sender<bool>,
    pub run_control: std::sync::Mutex<agent_run::RunControl>,
    connectors: tokio::sync::Mutex<connectors::McpHub>,
    oauth_operation: tokio::sync::Mutex<()>,
    oauth_cancel: tokio::sync::watch::Sender<bool>,
    pub subagents: std::sync::Arc<subagents::SubagentRegistry>,
    pub terminals: tokio::sync::Mutex<sandbox::TerminalRegistry>,
    pub processes: process_tools::ProcessRegistry,
    pub previews: workspace_preview::Manager,
    pub debuggers: debug_tools::Manager,
    pub http_sessions: api_tools::Manager,
    pub verification_observers: verification_observer::Registry,
    pub language_servers: lsp_client::Manager,
    /// Interactive PTY shells for the inspector terminal panel.
    pub pty_sessions: terminal_ui::PtyRegistry,
    /// Per-run web_search cache: (conversation_id, run_id, normalized question, mode) -> compact result.
    pub web_search_cache: Mutex<harness::WebSearchCache>,
    /// Shared turn budgets by run id. Nested tool dispatch (programmatic
    /// plans, child calls) charges the same counters the parent loop uses,
    /// before running the tool.
    pub turn_budgets: Mutex<
        std::collections::HashMap<String, std::sync::Arc<std::sync::Mutex<continuation::Budgets>>>,
    >,
    /// Non-retryable call signatures by run id. A failed/uncertain/denied
    /// call blocks the same signature for the rest of the turn — including
    /// replays hidden inside later programmatic plans.
    pub turn_blocked: Mutex<
        std::collections::HashMap<
            String,
            std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
        >,
    >,
    pub db_path: std::path::PathBuf,
    pub data_dir: std::path::PathBuf,
}
impl AppState {
    fn database(&self) -> Result<std::sync::MutexGuard<'_, store::Store>, String> {
        self.store
            .lock()
            .map_err(|_| "Database lock is unavailable. Restart the application.".into())
    }
    pub fn cancel_requested(&self) -> bool {
        *self.cancel.borrow()
    }
}

/// Where a panic is recorded, on every supported platform.
fn crash_log_path() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("LOCALAPPDATA")?;
        return Some(
            std::path::PathBuf::from(base)
                .join("com.locallm.app")
                .join("panic.log"),
        );
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")?;
        return Some(
            std::path::PathBuf::from(home)
                .join("Library")
                .join("Logs")
                .join("LocalLM")
                .join("panic.log"),
        );
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let base = std::env::var_os("XDG_STATE_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .map(|home| std::path::PathBuf::from(home).join(".local").join("state"))
            })?;
        return Some(base.join("locallm").join("panic.log"));
    }
    #[allow(unreachable_code)]
    None
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    std::panic::set_hook(Box::new(|info| {
        let message = format!(
            "[LocalLM PANIC] {}\nLocation: {}\nThread: {:?}\nTime: {:?}\n",
            info,
            info.location()
                .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
                .unwrap_or_else(|| "unknown".into()),
            std::thread::current().name().unwrap_or("unnamed"),
            std::time::SystemTime::now(),
        );
        eprintln!("{message}");
        // The crash log must land somewhere a user can find it on every
        // platform. Windows uses `%LOCALAPPDATA%`, macOS uses the user's
        // Library logs directory, and Linux follows XDG_STATE_HOME. The
        // previous version only wrote on Windows, so two of the three
        // supported platforms produced no diagnostics at all.
        if let Some(path) = crash_log_path() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // Keep the log bounded: a repeating panic must not fill the disk.
            let _ = std::fs::write(&path, &message);
        }
    }));
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED,
                )
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            sessions::initialize_activity_events(app.handle().clone());
            let data = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data)?;
            let store =
                store::Store::open(&data.join("locallm.sqlite")).map_err(std::io::Error::other)?;
            let vault = std::sync::Arc::new(vault::Vault::new(data.join("credentials")));
            app.manage(AppState {
                automation: std::sync::Arc::new(automation::Manager::default()),
                installation_operation: tokio::sync::Mutex::new(()),
                runtime_installer: model_install::Installer::default(),
                model_installer: model_install::Installer::default(),
                daytona_vault: vault.clone(),
                daytona_operation: std::sync::Arc::new(tokio::sync::Mutex::new(())),
                daytona_journal: std::sync::Arc::new(Mutex::new(
                    daytona_journal::Journal::open(&data.join("daytona.sqlite"))
                        .map_err(std::io::Error::other)?,
                )),
                skills: tokio::sync::Mutex::new(skills::Skills::new(data.join("skills"))),
                approvals: approval::Approvals::default(),
                ask_user: approval::ChoiceApprovals::default(),
                mcp_inputs: std::sync::Arc::new(mcp_input::Registry::default()),
                store: Mutex::new(store),
                runtime: tokio::sync::Mutex::new(runtime::Runtime::new(data.join("runtime.log"))),
                load_cancel: tokio::sync::watch::channel(false).0,
                load_status: std::sync::Arc::new(Mutex::new(None)),
                operation: OperationGate::default(),
                turns: TurnLocks::default(),
                cancel: tokio::sync::watch::channel(false).0,
                run_control: std::sync::Mutex::new(agent_run::RunControl::default()),
                connectors: tokio::sync::Mutex::new(connectors::McpHub::new(vault)),
                oauth_operation: tokio::sync::Mutex::new(()),
                oauth_cancel: tokio::sync::watch::channel(false).0,
                subagents: subagents::global_registry(),
                terminals: tokio::sync::Mutex::new(sandbox::TerminalRegistry::new()),
                processes: process_tools::ProcessRegistry::default(),
                previews: workspace_preview::Manager::default(),
                debuggers: debug_tools::Manager::default(),
                http_sessions: api_tools::Manager::default(),
                verification_observers: verification_observer::Registry::default(),
                language_servers: lsp_client::Manager::default(),
                pty_sessions: std::sync::Mutex::new(std::collections::HashMap::new()),
                web_search_cache: Mutex::new(harness::WebSearchCache::default()),
                turn_budgets: Mutex::new(std::collections::HashMap::new()),
                turn_blocked: Mutex::new(std::collections::HashMap::new()),
                db_path: data.join("locallm.sqlite"),
                data_dir: data.clone(),
            });
            if let Ok(store) = app.state::<AppState>().database() {
                // Crash recovery runs exactly once, here, on the authoritative
                // connection. `Store::open` deliberately does not recover, so
                // a second connection to the live database can never mark a
                // running tool's audit row as interrupted.
                let _ = store.recover_interrupted();
                let _ = research_tasks::recover_interrupted(&store);
            }
            let handle = app.handle().clone();
            let connector_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = connectors::restore_connections(&connector_handle.state::<AppState>()).await {
                    eprintln!("Could not restore saved connector selections: {error}");
                }
            });
            for (folder, runtime) in [("models", false), ("runtimes", true)] {
                if let Err(error) = install_recovery::clean(&managed_storage::root(&data).map_err(std::io::Error::other)?.join(folder), runtime) {
                    let state = app.state::<AppState>();
                    let installer = if runtime { &state.runtime_installer } else { &state.model_installer };
                    if let Ok(mut inner) = installer.inner.lock() { inner.0.error = Some(format!("Interrupted installation cleanup needs attention: {error}")); };
                }
            }
            // Browser session scratch directories are Chromium user-data
            // directories, tens of megabytes each. A crash leaves them
            // behind, so they are reclaimed at startup too.
            if let Err(error) = automation::sweep_browser_sessions(&data) {
                eprintln!("Could not sweep stale browser sessions: {error}");
            }
            tauri::async_runtime::spawn(async move {
                if daytona_settings::recover_at_startup(&handle.state::<AppState>()).await.is_err() {
                    eprintln!("Startup cloud cleanup could not access its journal; pending ownership is retained.");
                }
            });
            Ok(())
        })
        .invoke_handler({
          let handler: fn(tauri::ipc::Invoke<tauri::Wry>) -> bool = tauri::generate_handler![
            automation::get_automation_config,
            automation::save_automation_config,
            automation::automation_diagnostics,
            automation::reset_automation,
            automation::install_browser_engine,
            daytona_settings::has_daytona_key,
            daytona_settings::save_daytona_key,
            daytona_settings::forget_daytona_key,
            daytona_settings::retry_daytona_cleanup,
            daytona_journal::pending_daytona_operations,
            commands::bootstrap,
            hardware::hardware_status,
            managed_storage::managed_storage_info,
            managed_storage::move_managed_storage,
            execution::get_execution_config,
            profile_tools::get_profile_config,
            profile_tools::save_profile_config,
            remote_database::list_database_profiles,
            remote_database::save_database_profile,
            remote_database::delete_database_profile,
            remote_database::test_database_profile,
            development_credentials::list_http_credentials,
            development_credentials::save_http_credential,
            development_credentials::delete_http_credential,
            development_credentials::get_reference_origins,
            development_credentials::save_reference_origins,
            debug_tools::get_debugger_policy,
            debug_tools::save_debugger_policy,
            lsp_tools::get_lsp_config,
            lsp_tools::save_lsp_config,
            lsp_tools::test_lsp_server,
            artifacts::preview_execution_artifact,
            artifacts::save_artifact_copy,
            compaction::compact_conversation_cmd,
            compaction::compaction_status,
            compaction::set_compaction_auto,
            permission_rules::list_permission_rules,
            permission_rules::save_permission_rules,
            snapshots::list_snapshots,
            snapshots::stage_revert,
            snapshots::discard_revert,
            snapshots::commit_revert,
            snapshots::capture_snapshot,
            plugins::list_plugins,
            plugins::save_plugin,
            plugins::remove_plugin,
            weather::get_weather_config,
            weather::save_weather_config,
            execution::save_execution_config,
            execution::detect_interpreters,
            execution::test_interpreter,
            api_tools::open_workspace_path,
            workspace::get_workspace,
            workspace::set_workspace,
            workspace_ui::set_task_workspace,
            skills::list_skills,
            skills::install_skill,
            skills::remove_skill,
            skills::set_skill_active,
            skills::read_skill_file,
            skills::skill_update_status,
            skills::skill_dependencies,
            commands::create_conversation,
            commands::rename_conversation,
            commands::delete_conversation,
            commands::get_messages,
            export::export_conversation,
            commands::get_conversation_tools,
            commands::save_conversation_tools,
            commands::get_remembered_tools,
            commands::save_remembered_tools,
            commands::save_provider,
            commands::list_providers,
            commands::delete_provider,
            commands::delete_provider_api_key,
            commands::test_provider,
            commands::test_provider_draft,
            commands::list_provider_models,
            commands::list_provider_models_draft,
            commands::preferred_model,
            commands::save_preferred_model,
            commands::save_conversation_model,
            commands::save_runtime_config,
            commands::save_preferences,
            commands::load_model,
            commands::unload_model,
            runtime::runtime_devices,
            commands::runtime_status,
            commands::cancel_model_load,
            commands::test_provider_inference,
            commands::detect_subscription_cli,
            commands::import_subscription_cli,
            commands::get_subscription_status,
            commands::save_manual_subscription_token,
            commands::disconnect_subscription,
            commands::start_subscription_sign_in,
            commands::cancel_subscription_sign_in,
            commands::get_run,
            commands::get_conversation_run,
            commands::get_run_events,
            commands::get_conversation_runs,
            capabilities::list_capabilities,
            capabilities::set_capability_enabled,
            capabilities::dump_config,
            presets::list_presets,
            presets::get_preset,
            presets::set_preset,
            sessions::get_session_events,
            sessions::fork_session,
            sessions::replay_session,
            sessions::search_sessions,
            plans::get_todos,
            plans::get_goal,
            plans::clear_todos,
            subagents::list_subagent_runs,
            subagents::interrupt_subagent,
            subagents::list_subagent_models,
            memory::list_facts,
            memory::teach_fact_cmd,
            memory::forget_fact,
            memory::ingest_repo,
            workspace_ui::workspace_index,
            workspace_ui::save_project,
            workspace_ui::remove_project,
            workspace_ui::save_task_meta,
            workspace_ui::workspace_inspect,
            workspace_ui::workspace_search,
            workspace_ui::workspace_git,
            workspace_ui::workspace_git_action,
            editor::editor_read,
            editor::editor_save,
            editor::editor_mutate,
            terminal_ui::terminal_open,
            terminal_ui::terminal_attach,
            terminal_ui::terminal_ack,
            terminal_ui::terminal_input,
            terminal_ui::terminal_resize,
            terminal_ui::terminal_close,
            inspector_browser::browser_show,
            inspector_browser::browser_hide,
            inspector_browser::browser_close,
            inspector_browser::browser_navigate,
            inspector_browser::browser_reload,
            inspector_browser::browser_go_back,
            inspector_browser::browser_go_forward,
            inspector_browser::browser_url,
            sandbox::sandbox_status,
            sandbox::set_sandbox_provider,
            web_search::web_search_health,
            web_search::get_web_search_config,
            web_search::clear_web_cache,
            local_only::get_offline_mode,
            tool_compat::retest_tool_calling,
            local_only::set_offline_mode,
            web_search::save_web_search_config,
            web_search::web_search,
            web_search::list_research_sessions,
            web_search::get_research_session,
            web_search::delete_research_session,
            commands::get_artifact,
            runtime_log::read_runtime_log,
            model_catalog::model_download_info,
            model_library::search_hugging_face,
            model_library::hugging_face_files,
            model_library::download_hugging_face_model,
            model_library::list_installed_models,
            model_library::delete_installed_model,
            model_library::use_installed_model,
            gguf::read_model_metadata,
            runtime_install_commands::runtime_download_info,
            runtime_inventory::list_installed_runtimes,
            runtime_inventory::delete_installed_runtime,
            runtime_install_commands::runtime_install_status,
            runtime_install_commands::install_runtime,
            runtime_install_commands::cancel_runtime_install,
            model_install::model_install_status,
            model_install::install_model,
            model_install::cancel_model_install,
            chat::send_message,
            presentation::get_presentation_mode, presentation::set_presentation_mode,
            presentation::get_presentations, presentation::save_visual_state, presentation::reject_presentation,
            chat::local_reasoning_capability,
            latency_diagnostics::run_latency_diagnostic,
            document_index::document_index_status,
            document_index::save_document_index,
            document_index::search_document_index,
            chat::context_preflight,
            chat::cancel_generation,
            chat::pause_generation,
            chat::resume_generation,
            chat::restart_generation,
            approval::resolve_tool_approval,
            approval::resolve_ask_user,
            hooks::get_hooks,
            hooks::save_hooks,
            provider_capabilities::probe_model_tools_command,
            mcp_input::resolve_mcp_input,
            connectors::list_connectors,
            connectors::list_local_connectors,
            connectors::read_local_connector,
            connectors::save_local_connector,
            connectors::remove_local_connector,
            connectors::connect_connector,
            connectors::disconnect_connector,
            connectors::sign_in_connector,
            connectors::cancel_connector_sign_in,
          ];
          move |invoke| {
            // Inspector pages are untrusted browsing surfaces. They must not
            // invoke application commands, regardless of their URL or scripts.
            if invoke.message.webview_ref().label() != "main" {
              invoke.resolver.reject("Web pages cannot use LocalLM native commands.");
              true
            } else {
              handler(invoke)
            }
          }
        })
        .build(tauri::generate_context!())
        .expect("Unable to initialize LocalLM")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                let state = app.state::<AppState>();
                state.cancel.send_replace(true);
                if let Ok(mut sessions) = state.pty_sessions.lock() {
                    for (_, session) in sessions.drain() {
                        session.kill();
                    }
                }
                // Terminate every child this app owns. Tauri ends the
                // application with `std::process::exit`, so no destructor runs
                // and a `Drop`-based kill never fires — without this the
                // children outlive the app, reparented to init and unreachable
                // through the tool that started them.
                state.processes.stop_all();
                tauri::async_runtime::block_on(async {
                    state.automation.stop_all().await;
                    let _ = state.runtime.lock().await.stop().await;
                    state.terminals.lock().await.close_all().await;
                    state.debuggers.stop_all().await;
                    state.language_servers.shutdown().await;
                    state.connectors.lock().await.disconnect_all().await;
                });
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two conversations must be able to run at the same time. The gate used
    /// to be exclusive for the whole turn, which serialised the entire
    /// application behind one model operation.
    #[test]
    fn two_turns_can_hold_the_gate_at_once() {
        let gate = OperationGate::default();
        let first = gate
            .try_shared()
            .expect("first turn takes the shared lease");
        let second = gate
            .try_shared()
            .expect("second turn takes the shared lease");
        drop(first);
        drop(second);
    }

    /// An exclusive operation — a provider edit, a credential change — must
    /// still be refused while a turn is in flight, so the selected credential
    /// cannot change under a pending approval.
    #[test]
    fn an_exclusive_operation_is_refused_while_a_turn_holds_the_gate() {
        let gate = OperationGate::default();
        let lease = gate.try_shared().expect("turn takes the shared lease");
        assert!(gate.try_lock().is_err());
        drop(lease);
        assert!(gate.try_lock().is_ok());
    }

    /// A turn in one conversation must not block a turn in another.
    #[test]
    fn turns_in_different_conversations_do_not_block_each_other() {
        let turns = TurnLocks::default();
        let first = turns.acquire("conversation-a").expect("first turn");
        let second = turns.acquire("conversation-b").expect("second turn");
        drop(first);
        drop(second);
    }

    /// The same conversation runs one turn at a time.
    #[test]
    fn one_turn_at_a_time_per_conversation() {
        let turns = TurnLocks::default();
        let first = turns.acquire("conversation-a").expect("first turn");
        assert!(turns.acquire("conversation-a").is_err());
        drop(first);
        assert!(turns.acquire("conversation-a").is_ok());
    }

    /// A finished turn must not leave an entry behind for every conversation
    /// the user ever opened.
    #[test]
    fn a_finished_turn_releases_its_entry() {
        let turns = TurnLocks::default();
        {
            let _guard = turns.acquire("conversation-a").expect("turn");
        }
        let remaining = turns
            .locks
            .lock()
            .map(|locks| locks.len())
            .unwrap_or_default();
        assert_eq!(remaining, 0);
    }
}
