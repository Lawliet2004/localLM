pub mod agent_run;
mod approval;
pub mod artifacts;
pub mod capabilities;
mod chat;
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
mod inference;
mod install_recovery;
pub mod local_mcp_config;
pub mod local_mcp_process;
mod model_catalog;
mod model_install;
mod oauth;
mod permissions;
pub mod memory;
pub mod plans;
pub mod plugins;
pub mod presets;
mod providers;
mod runtime;
pub mod runtime_archive;
mod runtime_config;
pub mod runtime_install;
mod runtime_install_commands;
mod runtime_inventory;
mod runtime_log;
pub mod sandbox;
pub mod scheduling;
pub mod sessions;
mod skills;
mod sse;
pub mod store;
pub mod subagents;
mod tool_calls;
mod tool_discovery;
pub mod system_tools;
mod vault;
mod workspace;

use std::sync::Mutex;
use tauri::Manager;

pub struct AppState {
    installation_operation: tokio::sync::Mutex<()>,
    runtime_installer: model_install::Installer,
    model_installer: model_install::Installer,
    daytona_vault: std::sync::Arc<vault::Vault>,
    daytona_operation: std::sync::Arc<tokio::sync::Mutex<()>>,
    daytona_journal: std::sync::Arc<Mutex<daytona_journal::Journal>>,
    skills: tokio::sync::Mutex<skills::Skills>,
    approvals: approval::Approvals,
    store: Mutex<store::Store>,
    runtime: tokio::sync::Mutex<runtime::Runtime>,
    operation: tokio::sync::Mutex<()>,
    cancel: tokio::sync::watch::Sender<bool>,
    connectors: tokio::sync::Mutex<connectors::McpHub>,
    oauth_operation: tokio::sync::Mutex<()>,
    oauth_cancel: tokio::sync::watch::Sender<bool>,
    pub subagents: std::sync::Arc<subagents::SubagentRegistry>,
    pub terminals: tokio::sync::Mutex<sandbox::TerminalRegistry>,
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
            let vault = std::sync::Arc::new(vault::Vault::new(data.join("credentials")));
            app.manage(AppState {
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
                store: Mutex::new(store),
                runtime: tokio::sync::Mutex::new(runtime::Runtime::new(data.join("runtime.log"))),
                operation: tokio::sync::Mutex::new(()),
                cancel: tokio::sync::watch::channel(false).0,
                connectors: tokio::sync::Mutex::new(connectors::McpHub::new(vault)),
                oauth_operation: tokio::sync::Mutex::new(()),
                oauth_cancel: tokio::sync::watch::channel(false).0,
                subagents: subagents::global_registry(),
                terminals: tokio::sync::Mutex::new(sandbox::TerminalRegistry::new()),
                db_path: data.join("locallm.sqlite"),
                data_dir: data.clone(),
            });
            let handle = app.handle().clone();
            for (folder, runtime) in [("models", false), ("runtimes", true)] {
                if let Err(error) = install_recovery::clean(&data.join(folder), runtime) {
                    let state = app.state::<AppState>();
                    let installer = if runtime { &state.runtime_installer } else { &state.model_installer };
                    if let Ok(mut inner) = installer.inner.lock() { inner.0.error = Some(format!("Interrupted installation cleanup needs attention: {error}")); };
                }
            }
            tauri::async_runtime::spawn(async move {
                if daytona_settings::recover_at_startup(&handle.state::<AppState>()).await.is_err() {
                    eprintln!("Startup cloud cleanup could not access its journal; pending ownership is retained.");
                }
            });
            let scheduler_app = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                scheduling::run_background(scheduler_app).await;
            });
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
            providers::provider_formats,
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
            sessions::fork_session,
            sessions::replay_session,
            sessions::search_sessions,
            plans::get_todos,
            plans::get_goal,
            subagents::list_subagent_runs,
            subagents::interrupt_subagent,
            subagents::list_subagent_models,
            memory::list_facts,
            memory::teach_fact_cmd,
            memory::forget_fact,
            memory::ingest_repo,
            scheduling::list_schedules,
            scheduling::save_schedule,
            scheduling::delete_schedule,
            scheduling::run_schedule_now,
            scheduling::webhook_state,
            scheduling::set_webhook,
            scheduling::rotate_webhook_token,
            sandbox::sandbox_status,
            sandbox::set_sandbox_provider,
            plugins::list_plugins,
            plugins::install_plugin,
            plugins::set_plugin_enabled,
            plugins::remove_plugin,
            plugins::scan_plugin,
            plugins::test_plugin,
            compaction::compact_conversation_cmd,
            compaction::compaction_status,
            compaction::set_compaction_auto,
            commands::get_artifact,
            commands::list_conversation_artifacts,
            runtime_log::read_runtime_log,
            model_catalog::model_download_info,
            runtime_install_commands::runtime_download_info,
            runtime_inventory::list_installed_runtimes,
            runtime_install_commands::runtime_install_status,
            runtime_install_commands::install_runtime,
            runtime_install_commands::cancel_runtime_install,
            model_install::model_install_status,
            model_install::install_model,
            model_install::cancel_model_install,
            chat::send_message,
            chat::cancel_generation,
            approval::resolve_tool_approval,
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
                tauri::async_runtime::block_on(async {
                    let _ = state.runtime.lock().await.stop().await;
                });
            }
        });
}
