use crate::{
    runtime::RuntimeStatus,
    runtime_config::RuntimeConfig,
    store::{Conversation, Message, Preferences},
    AppState,
};
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    conversations: Vec<Conversation>,
    config: RuntimeConfig,
    preferences: Preferences,
    runtime: RuntimeStatus,
}

#[tauri::command]
pub async fn bootstrap(state: State<'_, AppState>) -> Result<Bootstrap, String> {
    let runtime = state.runtime.lock().await.inspect();
    let store = state.database()?;
    Ok(Bootstrap {
        conversations: store.list_conversations()?,
        config: store.runtime_config()?,
        preferences: store.preferences()?,
        runtime,
    })
}
#[tauri::command]
pub fn create_conversation(state: State<'_, AppState>) -> Result<Conversation, String> {
    state.database()?.create_conversation()
}
#[tauri::command]
pub fn rename_conversation(
    state: State<'_, AppState>,
    id: String,
    title: String,
) -> Result<(), String> {
    state.database()?.rename_conversation(&id, &title)
}
#[tauri::command]
pub async fn delete_conversation(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Stop the active operation before deleting a conversation.")?;
    state.database()?.delete_conversation(&id)
}
#[tauri::command]
pub fn get_messages(state: State<'_, AppState>, id: String) -> Result<Vec<Message>, String> {
    state.database()?.messages(&id)
}
#[tauri::command]
pub fn get_conversation_tools(
    state: State<'_, AppState>,
    id: String,
) -> Result<crate::store::ConversationTools, String> {
    state.database()?.conversation_tools(&id)
}
#[tauri::command]
pub async fn save_conversation_tools(
    state: State<'_, AppState>,
    id: String,
    tools: crate::store::ConversationTools,
) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before changing conversation tools.")?;
    state.database()?.save_conversation_tools(&id, &tools)
}
#[tauri::command]
pub fn save_runtime_config(
    state: State<'_, AppState>,
    config: RuntimeConfig,
) -> Result<(), String> {
    state.database()?.save_runtime_config(&config)
}
#[tauri::command]
pub fn save_preferences(
    state: State<'_, AppState>,
    preferences: Preferences,
) -> Result<(), String> {
    state.database()?.save_preferences(&preferences)
}
#[tauri::command]
pub async fn load_model(state: State<'_, AppState>) -> Result<RuntimeStatus, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "A model operation is already in progress.")?;
    let (preferences, config) = {
        let store = state.database()?;
        (store.preferences()?, store.runtime_config()?)
    };
    state.runtime.lock().await.load(&preferences, &config).await
}
#[tauri::command]
pub async fn unload_model(state: State<'_, AppState>) -> Result<RuntimeStatus, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Stop generation before unloading the model.")?;
    state.runtime.lock().await.stop().await
}
#[tauri::command]
pub async fn runtime_status(state: State<'_, AppState>) -> Result<RuntimeStatus, String> {
    Ok(state.runtime.lock().await.inspect())
}
