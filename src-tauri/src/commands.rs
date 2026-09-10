use crate::{
    providers::{self, ModelSelection, ProviderConnection, ProviderDraft, ProviderTestResult},
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
    remembered_tools: crate::store::RememberedTools,
    providers: Vec<ProviderConnection>,
    preferred_model: ModelSelection,
}

#[tauri::command]
pub async fn bootstrap(state: State<'_, AppState>) -> Result<Bootstrap, String> {
    let runtime = state.runtime.lock().await.inspect();
    let store = state.database()?;
    let providers = provider_views(&store, &state.daytona_vault)?;
    Ok(Bootstrap {
        conversations: store.list_conversations()?,
        config: store.runtime_config()?,
        preferences: store.preferences()?,
        runtime,
        remembered_tools: store.remembered_tools()?.unwrap_or_default(),
        providers,
        preferred_model: store.preferred_model()?,
    })
}

fn provider_views(store: &crate::store::Store, vault: &crate::vault::Vault) -> Result<Vec<ProviderConnection>, String> {
    store.providers()?.into_iter().map(|mut provider| {
        provider.has_api_key = vault.load(&providers::credential_id(&provider.id))?.is_some();
        Ok(provider)
    }).collect()
}

#[tauri::command]
pub async fn save_provider(state: State<'_, AppState>, mut draft: ProviderDraft) -> Result<ProviderConnection, String> {
    let _operation = state.operation.try_lock().map_err(|_| "Wait for the active operation before changing providers.")?;
    let existing = draft.id.as_deref().map(|id| state.database().and_then(|store| store.provider(id))).transpose()?.flatten();
    providers::validate_provider_draft(&draft, existing.is_some())?;
    let id = draft.id.take().unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    providers::validate_provider_id(&id)?;
    let base_url = providers::validate_base_url(&draft.base_url)?;
    if let Some(key) = &draft.api_key {
        state.daytona_vault.save(&providers::credential_id(&id), key.as_bytes())?;
    }
    let provider = ProviderConnection { id: id.clone(), name: draft.name, api_format: draft.api_format, base_url, verified: false, last_tested_at: None, models: draft.models, has_api_key: state.daytona_vault.load(&providers::credential_id(&id))?.is_some() };
    state.database()?.save_provider(&provider)?;
    Ok(provider)
}

#[tauri::command]
pub fn list_providers(state: State<'_, AppState>) -> Result<Vec<ProviderConnection>, String> {
    let store = state.database()?;
    provider_views(&store, &state.daytona_vault)
}

#[tauri::command]
pub async fn delete_provider(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let _operation = state.operation.try_lock().map_err(|_| "Stop the active operation before deleting a provider.")?;
    providers::validate_provider_id(&id)?;
    state.daytona_vault.clear(&providers::credential_id(&id))?;
    if !state.database()?.delete_provider(&id)? {
        return Err("Provider no longer exists.".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn test_provider(state: State<'_, AppState>, id: String) -> Result<ProviderTestResult, String> {
    let _operation = state.operation.try_lock().map_err(|_| "Wait for the active operation before testing a provider.")?;
    let provider = state.database()?.provider(&id)?.ok_or("Provider no longer exists.")?;
    let key = state.daytona_vault.load(&providers::credential_id(&id))?.ok_or("Save an API key before testing this provider.")?;
    let key = String::from_utf8(key).map_err(|_| "Saved provider API key is invalid.")?;
    let result = match providers::test_connection(&provider, &key).await {
        Ok(result) => result,
        Err(error) => {
            state.database()?.update_provider_test(&id, false, Some(crate::store::now()), &provider.models)?;
            return Err(error);
        }
    };
    let models = if result.model_list_supported { providers::merge_listed_models(&provider.models, &result.models) } else { provider.models.clone() };
    state.database()?.update_provider_test(&id, result.verified, Some(crate::store::now()), &models)?;
    Ok(result)
}

#[tauri::command]
pub async fn list_provider_models(state: State<'_, AppState>, id: String) -> Result<ProviderConnection, String> {
    let _operation = state.operation.try_lock().map_err(|_| "Wait for the active operation before listing provider models.")?;
    let provider = state.database()?.provider(&id)?.ok_or("Provider no longer exists.")?;
    let key = state.daytona_vault.load(&providers::credential_id(&id))?.ok_or("Save an API key before listing models.")?;
    let key = String::from_utf8(key).map_err(|_| "Saved provider API key is invalid.")?;
    let result = providers::list_models(&provider, &key).await?;
    if !result.supported {
        return Err("This provider does not expose model listing. Enter a model ID manually.".into());
    }
    let models = providers::merge_listed_models(&provider.models, &result.models);
    state.database()?.update_provider_models(&id, &models)?;
    let mut updated = provider;
    updated.models = models;
    updated.has_api_key = true;
    Ok(updated)
}

#[tauri::command]
pub fn preferred_model(state: State<'_, AppState>) -> Result<ModelSelection, String> {
    state.database()?.preferred_model()
}

#[tauri::command]
pub async fn save_preferred_model(state: State<'_, AppState>, selection: ModelSelection) -> Result<(), String> {
    let _operation = state.operation.try_lock().map_err(|_| "Wait for the active operation before changing models.")?;
    state.database()?.save_preferred_model(&selection)
}

#[tauri::command]
pub async fn save_conversation_model(state: State<'_, AppState>, id: String, selection: ModelSelection) -> Result<(), String> {
    let _operation = state.operation.try_lock().map_err(|_| "Wait for the active operation before changing models.")?;
    state.database()?.save_conversation_model(&id, &selection)
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
pub fn get_remembered_tools(
    state: State<'_, AppState>,
) -> Result<crate::store::RememberedTools, String> {
    Ok(state.database()?.remembered_tools()?.unwrap_or_default())
}
#[tauri::command]
pub fn save_remembered_tools(
    state: State<'_, AppState>,
    tools: crate::store::RememberedTools,
) -> Result<(), String> {
    state.database()?.save_remembered_tools(&tools)
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

#[tauri::command]
pub async fn test_provider_inference(
    state: State<'_, AppState>,
    id: String,
    model_id: String,
) -> Result<ProviderTestResult, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before testing a provider.")?;
    let provider = state.database()?.provider(&id)?.ok_or("Provider no longer exists.")?;
    let key = state.daytona_vault.load(&providers::credential_id(&id))?.ok_or("Save an API key before testing this provider.")?;
    let key = String::from_utf8(key).map_err(|_| "Saved provider API key is invalid.")?;
    let result = providers::test_inference(&provider, &key, &model_id).await?;
    state.database()?.update_provider_test(&id, true, Some(crate::store::now()), &provider.models)?;
    Ok(result)
}

#[tauri::command]
pub fn get_run(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<crate::agent_run::RunRecord>, String> {
    state.database()?.run(&id)
}

#[tauri::command]
pub fn get_conversation_run(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<Option<crate::agent_run::RunRecord>, String> {
    state.database()?.active_run(&conversation_id)
}

#[tauri::command]
pub fn get_run_events(
    state: State<'_, AppState>,
    run_id: String,
) -> Result<Vec<crate::agent_run::RunEvent>, String> {
    state.database()?.run_events(&run_id)
}

#[tauri::command]
pub fn get_conversation_runs(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<Vec<crate::agent_run::RunRecord>, String> {
    state.database()?.runs_for_conversation(&conversation_id)
}

#[tauri::command]
pub fn get_artifact(
    state: State<'_, AppState>,
    id: String,
) -> Result<Option<crate::artifacts::ArtifactRecord>, String> {
    state.database()?.artifact(&id)
}

#[tauri::command]
pub fn list_conversation_artifacts(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<Vec<crate::artifacts::ArtifactRecord>, String> {
    state.database()?.artifacts_for_conversation(&conversation_id)
}

