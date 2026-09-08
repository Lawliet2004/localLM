mod approval;
mod chat;
mod commands;
mod connectors;
mod context;
pub mod daytona;
pub mod daytona_cleanup;
pub mod daytona_execution;
pub mod daytona_journal;
mod daytona_settings;
mod execution;
mod export;
mod hardware;
mod history;
mod oauth;
mod permissions;
mod runtime;
mod runtime_config;
mod runtime_log;
mod skills;
mod sse;
mod store;
mod tool_calls;
mod vault;
mod workspace;

use std::sync::Mutex;
use tauri::Manager;

pub struct AppState {
    daytona_vault: std::sync::Arc<vault::Vault>,
    daytona_operation: tokio::sync::Mutex<()>,
    daytona_journal: Mutex<daytona_journal::Journal>,
    skills: tokio::sync::Mutex<skills::Skills>,
    approvals: approval::Approvals,
    store: Mutex<store::Store>,
    runtime: tokio::sync::Mutex<runtime::Runtime>,
    operation: tokio::sync::Mutex<()>,
    cancel: tokio::sync::watch::Sender<bool>,
    connectors: tokio::sync::Mutex<connectors::McpHub>,
    oauth_operation: tokio::sync::Mutex<()>,
    oauth_cancel: tokio::sync::watch::Sender<bool>,
}
impl AppState {
    fn database(&self) -> Result<std::sync::MutexGuard<'_, store::Store>, String> {
        self.store
            .lock()
            .map_err(|_| "Database lock is unavailable. Restart the application.".into())
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
                daytona_vault: vault.clone(),
                daytona_operation: tokio::sync::Mutex::new(()),
                daytona_journal: Mutex::new(
                    daytona_journal::Journal::open(&data.join("daytona.sqlite"))
                        .map_err(std::io::Error::other)?,
                ),
                skills: tokio::sync::Mutex::new(skills::Skills::new(data.join("skills"))),
                approvals: approval::Approvals::default(),
                store: Mutex::new(store),
                runtime: tokio::sync::Mutex::new(runtime::Runtime::new(data.join("runtime.log"))),
                operation: tokio::sync::Mutex::new(()),
                cancel: tokio::sync::watch::channel(false).0,
                connectors: tokio::sync::Mutex::new(connectors::McpHub::new(vault)),
                oauth_operation: tokio::sync::Mutex::new(()),
                oauth_cancel: tokio::sync::watch::channel(false).0,
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
            commands::create_conversation,
            commands::rename_conversation,
            commands::delete_conversation,
            commands::get_messages,
            export::export_conversation,
            commands::get_conversation_tools,
            commands::save_conversation_tools,
            commands::save_runtime_config,
            commands::save_preferences,
            commands::load_model,
            commands::unload_model,
            commands::runtime_status,
            chat::send_message,
            chat::cancel_generation,
            approval::resolve_tool_approval,
            connectors::list_connectors,
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
