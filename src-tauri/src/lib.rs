pub mod agent_run;
mod arex;
pub mod continuation;
pub mod local_only;
pub mod tool_compat;
pub mod approval;
pub mod artifacts;
pub mod capabilities;
mod chat;
mod attachments;
mod workspace_ui;
mod claude_adapter;
mod commands;
pub mod compaction;
mod connectors;
mod context;
pub mod daytona;
pub mod daytona_cleanup;
pub mod daytona_execution;
pub mod daytona_journal;
mod daytona_settings;
pub mod download;
mod execution;
mod export;
mod hardware;
pub mod harness;
mod history;
pub mod gguf;
mod inference;
mod install_recovery;
pub mod isolation;
pub mod local_mcp_config;
pub mod local_mcp_process;
mod model_catalog;
mod model_install;
mod model_library;
mod oauth;
mod permissions;
pub mod memory;
pub mod plans;
mod prompt;
mod qualification;
mod repo_map;
mod resources;
pub mod documents;
pub mod presets;
pub mod recipes;
mod providers;
mod runtime;
pub mod runtime_archive;
mod runtime_config;
pub mod runtime_install;
mod runtime_install_commands;
mod runtime_inventory;
mod runtime_log;
pub mod sandbox;
pub mod sessions;
pub mod skills;
mod sse;
mod store;
mod inspector_browser;
mod browser_agent;
mod terminal_ui;
mod tool_envelope;
mod research_tasks;
pub mod web_search;
mod tool_calls;
mod tool_discovery;
pub mod subscription_auth;
pub mod subagents;
pub mod system_tools;
mod task_queue;
mod task_workspace;
mod vault;
mod verification;
mod weather;
mod workspace;

use std::sync::Mutex;
use tauri::Manager;

pub struct AppState {
    runtime_installer: model_install::Installer,
    model_installer: model_install::Installer,
    daytona_vault: std::sync::Arc<vault::Vault>,
    daytona_operation: std::sync::Arc<tokio::sync::Mutex<()>>,
    daytona_journal: std::sync::Arc<Mutex<daytona_journal::Journal>>,
    skills: tokio::sync::Mutex<skills::Skills>,
    approvals: approval::Approvals,
    pub ask_user: approval::ChoiceApprovals,
    store: Mutex<store::Store>,
    runtime: tokio::sync::Mutex<runtime::Runtime>,
    /// Last published runtime status; readable while a load holds `runtime`.
    pub runtime_snapshot: std::sync::Arc<std::sync::Mutex<runtime::RuntimeStatus>>,
    /// The inference lease: capacity 1. `Arc` so queued tasks can wait on an
    /// owned guard while direct callers keep `try_lock` semantics.
    operation: std::sync::Arc<tokio::sync::Mutex<()>>,
    /// Named resource slots (browser, workspace-write:<root>, background);
    /// `inference` records its holder here for wait-reason display.
    pub resources: std::sync::Arc<resources::LeaseRegistry>,
    cancel: tokio::sync::watch::Sender<bool>,
    pub run_control: std::sync::Mutex<agent_run::RunControl>,
    connectors: tokio::sync::Mutex<connectors::McpHub>,
    oauth_operation: tokio::sync::Mutex<()>,
    oauth_cancel: tokio::sync::watch::Sender<bool>,
    pub subagents: std::sync::Arc<subagents::SubagentRegistry>,
    pub terminals: tokio::sync::Mutex<sandbox::TerminalRegistry>,
    /// Interactive PTY shells for the inspector terminal panel.
    pub pty_sessions: terminal_ui::PtyRegistry,
    /// Per-run web_search cache: (conversation_id, run_id, normalized question, mode) -> compact result.
    pub web_search_cache: Mutex<std::collections::HashMap<String, serde_json::Value>>,
    /// Windows only: keeps the configured WSL distro (and its SearXNG unit)
    /// alive for the app's lifetime; released on exit.
    pub searxng_wsl: Mutex<Option<(String, Box<dyn process_wrap::tokio::ChildWrapper>)>>,
    /// The in-flight qualification run's cancel channel — separate from
    /// `cancel` so a leftover `true` from a cancelled turn cannot wedge it.
    pub qualifying: Mutex<Option<tokio::sync::watch::Sender<bool>>>,
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    std::panic::set_hook(Box::new(|info| {
        let message = format!(
            "[LocalLM PANIC] {}\nLocation: {}\nThread: {:?}\nTime: {:?}\n",
            info,
            info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_else(|| "unknown".into()),
            std::thread::current().name().unwrap_or("unnamed"),
            std::time::SystemTime::now(),
        );
        eprintln!("{message}");
        if let Ok(appdata) = std::env::var("LOCALAPPDATA") {
            let path = std::path::Path::new(&appdata).join("com.locallm.app").join("panic.log");
            let _ = std::fs::create_dir_all(path.parent().unwrap_or(std::path::Path::new(".")));
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
            let data = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data)?;
            let store =
                store::Store::open(&data.join("locallm.sqlite")).map_err(std::io::Error::other)?;
            let runtime = runtime::Runtime::new(data.join("runtime.log"));
            let runtime_snapshot = runtime.published();
            let vault = std::sync::Arc::new(vault::Vault::new(data.join("credentials")));
            app.manage(AppState {
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
                store: Mutex::new(store),
                runtime: tokio::sync::Mutex::new(runtime),
                runtime_snapshot,
                operation: std::sync::Arc::new(tokio::sync::Mutex::new(())),
                resources: std::sync::Arc::new(resources::LeaseRegistry::default()),
                cancel: tokio::sync::watch::channel(false).0,
                run_control: std::sync::Mutex::new(agent_run::RunControl::default()),
                connectors: tokio::sync::Mutex::new(connectors::McpHub::new(vault)),
                oauth_operation: tokio::sync::Mutex::new(()),
                oauth_cancel: tokio::sync::watch::channel(false).0,
                subagents: subagents::global_registry(),
                terminals: tokio::sync::Mutex::new(sandbox::TerminalRegistry::new()),
                pty_sessions: std::sync::Mutex::new(std::collections::HashMap::new()),
                web_search_cache: Mutex::new(std::collections::HashMap::new()),
                searxng_wsl: Mutex::new(None),
                qualifying: Mutex::new(None),
                db_path: data.join("locallm.sqlite"),
                data_dir: data.clone(),
            });
            if let Ok(store) = app.state::<AppState>().database() {
                if let Err(error) = store.recover_interrupted_runs() {
                    eprintln!("Startup run recovery could not complete: {error}");
                }
                let _ = research_tasks::recover_interrupted(&store);
                if let Err(error) = task_queue::recover_tasks(&store) {
                    eprintln!("Startup task recovery could not complete: {error}");
                }
                if let Err(error) = store.repair_open_turn_tails() {
                    eprintln!("Startup session log repair could not complete: {error}");
                }
            }
            let handle = app.handle().clone();
            task_queue::spawn_scheduler(&handle);
            let _ = inspector_browser::APP.set(handle.clone());
            let connector_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(error) = connectors::restore_connections(&connector_handle.state::<AppState>()).await {
                    eprintln!("Could not restore saved connector selections: {error}");
                }
            });
            {
                let state = app.state::<AppState>();
                if let Err(error) = install_recovery::clean(&data.join("models")) {
                    state.model_installer.update(|status| {
                        status.error =
                            Some(format!("Interrupted installation needs attention: {error}"));
                    });
                }
                // Recognizable runtime staging migrates into the stable archive
                // cache so interrupted installs resume instead of restarting.
                if let Ok(assets) = runtime_archive::catalog() {
                    let root = data.join("runtimes");
                    match install_recovery::recover_runtime(&root, &assets) {
                        Ok(recovered) => {
                            let complete = runtime_inventory::find_complete(&root, runtime_install::STANDARD_PREFIX, &assets)
                                .ok()
                                .flatten()
                                .is_some();
                            if recovered.saved > 0 && !complete {
                                state.runtime_installer.update(|status| {
                                    status.phase = "interrupted".into();
                                    status.key = "runtime:standard".into();
                                    status.label = "llama.cpp CUDA runtime".into();
                                    status.saved = recovered.saved;
                                    status.received = recovered.saved;
                                    status.total = recovered.total;
                                    status.notice = Some(format!(
                                        "Download paused at {} of {} bytes. Resume to continue — saved archives will be reused.",
                                        recovered.saved, recovered.total
                                    ));
                                });
                            }
                        }
                        Err(error) => state.runtime_installer.update(|status| {
                            status.phase = "interrupted".into();
                            status.key = "runtime:standard".into();
                            status.label = "llama.cpp CUDA runtime".into();
                            status.error =
                                Some(format!("Interrupted installation needs attention: {error}"));
                        }),
                    }
                }
                // A leftover model .part marks its slot as resumable too.
                let bytes = model_catalog::MODEL.bytes;
                let partial = download::partial_path(&data.join("models").join(model_catalog::MODEL_FILENAME));
                let saved = std::fs::metadata(&partial).ok()
                    .filter(|metadata| metadata.is_file())
                    .map(|metadata| metadata.len().min(bytes)).unwrap_or(0);
                if saved > 0 {
                    state.model_installer.update(|status| {
                        status.phase = "interrupted".into();
                        status.key = format!("model:catalog:{}", model_catalog::MODEL_FILENAME);
                        status.label = model_catalog::MODEL_FILENAME.into();
                        status.saved = saved;
                        status.received = saved;
                        status.total = bytes;
                        status.notice = Some(format!(
                            "Download paused at {saved} of {bytes} bytes. Resume to continue — saved archives will be reused."
                        ));
                    });
                }
            }
            tauri::async_runtime::spawn(async move {
                if daytona_settings::recover_at_startup(&handle.state::<AppState>()).await.is_err() {
                    eprintln!("Startup cloud cleanup could not access its journal; pending ownership is retained.");
                }
            });
            web_search::start_searxng_wsl(&app.state::<AppState>());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            daytona_settings::has_daytona_key,
            daytona_settings::save_daytona_key,
            daytona_settings::forget_daytona_key,
            daytona_settings::retry_daytona_cleanup,
            daytona_journal::pending_daytona_operations,
            commands::bootstrap,
            hardware::hardware_status,
            execution::get_execution_config,
            execution::save_execution_config,
            execution::detect_interpreters,
            execution::test_interpreter,
            isolation::isolation_status,
            isolation::isolation_get,
            isolation::isolation_save,
            recipes::recipes_list,
            recipes::recipe_save,
            recipes::recipe_delete,
            recipes::recipe_run_now,
            documents::documents_list_collections,
            documents::documents_add_collection,
            documents::documents_remove_collection,
            documents::documents_reindex,
            documents::documents_search,
            workspace::get_workspace,
            workspace::set_workspace,
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
            commands::test_provider,
            commands::list_provider_models,
            commands::preferred_model,
            commands::save_preferred_model,
            commands::save_conversation_model,
            commands::save_runtime_config,
            commands::save_preferences,
            commands::load_model,
            commands::unload_model,
            commands::runtime_status,
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
            memory::memory_facts,
            memory::memory_create,
            memory::memory_update,
            memory::memory_delete,
            memory::memory_set_pinned,
            memory::memory_set_excluded,
            memory::memory_auto_retain,
            memory::memory_set_auto_retain,
            memory::context_preview,
            workspace_ui::workspace_index,
            workspace_ui::save_project,
            workspace_ui::remove_project,
            workspace_ui::save_task_meta,
            workspace_ui::workspace_inspect,
            workspace_ui::workspace_search,
            workspace_ui::workspace_git,
            task_workspace::task_workspace_create,
            task_workspace::task_workspace_status,
            task_workspace::task_review,
            task_workspace::task_file_diff,
            task_workspace::task_checkpoints,
            task_workspace::task_restore_checkpoint,
            task_workspace::task_apply,
            task_workspace::task_undo_apply,
            task_workspace::task_discard,
            verification::verification_summary,
            verification::criteria_upsert,
            verification::criteria_delete,
            verification::verification_run_user,
            verification::verification_record_output,
            terminal_ui::terminal_open,
            terminal_ui::terminal_attach,
            terminal_ui::terminal_input,
            terminal_ui::terminal_resize,
            terminal_ui::terminal_close,
            inspector_browser::browser_show,
            inspector_browser::browser_hide,
            inspector_browser::browser_close,
            inspector_browser::browser_close_all,
            inspector_browser::browser_navigate,
            inspector_browser::browser_reload,
            inspector_browser::browser_go_back,
            inspector_browser::browser_go_forward,
            inspector_browser::browser_url,
            browser_agent::get_browser_config,
            browser_agent::save_browser_config,
            sandbox::sandbox_status,
            sandbox::set_sandbox_provider,
            web_search::web_search_health,
            web_search::get_web_search_config,
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
            runtime_install_commands::runtime_install_status,
            runtime_install_commands::install_runtime,
            runtime_install_commands::cancel_runtime_install,
            model_install::model_install_status,
            model_install::install_model,
            model_install::cancel_model_install,
            model_install::download_jobs,
            task_queue::tasks_list,
            task_queue::task_enqueue,
            task_queue::task_steer,
            task_queue::task_pause,
            task_queue::task_resume,
            task_queue::task_cancel,
            task_queue::task_retry,
            task_queue::task_branch,
            task_queue::task_resolve_uncertain,
            task_queue::task_transitions,
            qualification::qualification_run,
            qualification::qualification_cancel,
            qualification::qualification_results,
            qualification::readiness,
            qualification::profiles,
            chat::send_message,
            chat::context_preflight,
            chat::effective_capabilities,
            compaction::compaction_status,
            compaction::set_compaction_auto,
            compaction::compact_conversation_cmd,
            chat::cancel_generation,
            chat::pause_generation,
            chat::resume_generation,
            chat::restart_generation,
            approval::resolve_tool_approval,
            approval::resolve_ask_user,
            connectors::list_connectors,
            connectors::list_local_connectors,
            connectors::read_local_connector,
            connectors::save_local_connector,
            connectors::remove_local_connector,
            connectors::connect_connector,
            connectors::disconnect_connector,
            connectors::sign_in_connector,
            connectors::cancel_connector_sign_in,
        ])
        .build(tauri::generate_context!())
        .expect("Unable to initialize LocalLM")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                let state = app.state::<AppState>();
                state.cancel.send_replace(true);
                web_search::stop_searxng_wsl(&state);
                if let Ok(mut sessions) = state.pty_sessions.lock() {
                    for (_, session) in sessions.drain() {
                        session.kill();
                    }
                }
                tauri::async_runtime::block_on(async {
                    let _ = state.runtime.lock().await.stop().await;
                    state.terminals.lock().await.kill_all().await;
                });
            }
        });
}
