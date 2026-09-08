mod approval;
mod chat;
mod commands;
mod connectors;
mod execution;
mod oauth;
mod runtime;
mod runtime_config;
mod skills;
mod sse;
mod store;
mod tool_calls;
mod vault;
mod workspace;

use std::sync::Mutex;
use tauri::Manager;

pub struct AppState {
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
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data)?;
            let store =
                store::Store::open(&data.join("locallm.sqlite")).map_err(std::io::Error::other)?;
            app.manage(AppState {
                skills: tokio::sync::Mutex::new(skills::Skills::new(data.join("skills"))),
                approvals: approval::Approvals::default(),
                store: Mutex::new(store),
                runtime: tokio::sync::Mutex::new(runtime::Runtime::new(data.join("runtime.log"))),
                operation: tokio::sync::Mutex::new(()),
                cancel: tokio::sync::watch::channel(false).0,
                connectors: tokio::sync::Mutex::new(connectors::McpHub::new(std::sync::Arc::new(
                    vault::Vault::new(data.join("credentials")),
                ))),
                oauth_operation: tokio::sync::Mutex::new(()),
                oauth_cancel: tokio::sync::watch::channel(false).0,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::bootstrap,
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
