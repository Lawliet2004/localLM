use crate::{
    providers::{self, ModelSelection, ProviderConnection, ProviderDraft, ProviderTestResult},
    runtime::RuntimeStatus,
    runtime_config::RuntimeConfig,
    store::{Conversation, Message, Preferences},
    AppState,
};
use serde::Serialize;
use std::path::Path;
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
    let saved_preferences = store.preferences()?;
    let config = store.runtime_config()?;
    let mut preferences = saved_preferences.clone().apply_model_defaults();
    preferences.max_tokens =
        crate::context::fit_response_budget(preferences.max_tokens, config.context_length);
    if preferences.max_tokens != saved_preferences.max_tokens {
        store.save_preferences(&preferences)?;
    }
    Ok(Bootstrap {
        conversations: store.list_conversations()?,
        config,
        preferences,
        runtime,
        remembered_tools: store.remembered_tools()?.unwrap_or_default(),
        providers,
        preferred_model: store.preferred_model()?,
    })
}

fn provider_views(
    store: &crate::store::Store,
    vault: &crate::vault::Vault,
) -> Result<Vec<ProviderConnection>, String> {
    store
        .providers()?
        .into_iter()
        .map(|mut provider| {
            if provider.id == "chatgpt-subscription"
                || provider.api_format == providers::CHATGPT_SUBSCRIPTION
            {
                provider.has_api_key =
                    vault.contains(crate::subscription_auth::VAULT_CHATGPT_SUB)?;
            } else if provider.id == "grok-subscription"
                || provider.api_format == providers::GROK_SUBSCRIPTION
            {
                provider.has_api_key = vault.contains(crate::subscription_auth::VAULT_GROK_SUB)?;
            } else {
                provider.has_api_key = vault.contains(&providers::credential_id(&provider.id))?;
            }
            Ok(provider)
        })
        .collect()
}

#[tauri::command]
pub async fn save_provider(
    state: State<'_, AppState>,
    draft: ProviderDraft,
) -> Result<ProviderConnection, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before changing providers.")?;
    providers::save_connection(&*state.database()?, &state.daytona_vault, draft)
}

#[tauri::command]
pub fn list_providers(state: State<'_, AppState>) -> Result<Vec<ProviderConnection>, String> {
    let store = state.database()?;
    provider_views(&store, &state.daytona_vault)
}

#[tauri::command]
pub async fn delete_provider(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Stop the active operation before deleting a provider.")?;
    providers::delete_connection(&*state.database()?, &state.daytona_vault, &id)
}

#[tauri::command]
pub async fn delete_provider_api_key(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Stop the active operation before deleting an API key.")?;
    providers::delete_api_key(&*state.database()?, &state.daytona_vault, &id)
}

async fn resolve_provider_key(
    state: &State<'_, AppState>,
    provider: &ProviderConnection,
) -> Result<String, String> {
    if provider.api_format == providers::CHATGPT_SUBSCRIPTION
        || provider.id == "chatgpt-subscription"
    {
        crate::subscription_auth::get_valid_access_token(&state.daytona_vault, "chatgpt").await
    } else if provider.api_format == providers::GROK_SUBSCRIPTION
        || provider.id == "grok-subscription"
    {
        crate::subscription_auth::get_valid_access_token(&state.daytona_vault, "grok").await
    } else {
        providers::resolve_api_key(
            provider,
            state
                .daytona_vault
                .load(&providers::credential_id(&provider.id))?,
        )
    }
}

#[tauri::command]
pub async fn test_provider(
    state: State<'_, AppState>,
    id: String,
) -> Result<ProviderTestResult, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before testing a provider.")?;
    let provider = state
        .database()?
        .provider(&id)?
        .ok_or("Provider no longer exists.")?;
    let key = resolve_provider_key(&state, &provider).await?;
    let result = match providers::test_connection(&provider, &key).await {
        Ok(result) => result,
        Err(error) => {
            state.database()?.update_provider_test(
                &id,
                false,
                Some(crate::store::now()),
                &provider.models,
            )?;
            return Err(error);
        }
    };
    let models = if result.model_list_supported {
        providers::merge_listed_models(&provider.models, &result.discovered_models)
    } else {
        provider.models.clone()
    };
    state.database()?.update_provider_test(
        &id,
        result.verified,
        Some(crate::store::now()),
        &models,
    )?;
    Ok(result)
}

/// Test an unsaved provider draft — same code path as `test_provider` but the
/// draft's key is used directly and nothing is written to the store or vault.
#[tauri::command]
pub async fn test_provider_draft(
    state: State<'_, AppState>,
    draft: ProviderDraft,
) -> Result<ProviderTestResult, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before testing a provider.")?;
    let key = draft.api_key.clone().unwrap_or_default();
    let provider = providers::connection_from_draft(draft)?;
    providers::test_connection(&provider, &key).await
}

/// List models for an unsaved provider draft without persisting anything.
#[tauri::command]
pub async fn list_provider_models_draft(
    state: State<'_, AppState>,
    draft: ProviderDraft,
) -> Result<Vec<providers::RemoteModel>, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before listing provider models.")?;
    let key = draft.api_key.clone().unwrap_or_default();
    let provider = providers::connection_from_draft(draft)?;
    let result = providers::list_models(&provider, &key).await?;
    if !result.supported {
        return Err(
            "This provider does not expose model listing. Enter a model ID manually.".into(),
        );
    }
    Ok(providers::merge_listed_models(
        &provider.models,
        &result.details,
    ))
}

#[tauri::command]
pub async fn list_provider_models(
    state: State<'_, AppState>,
    id: String,
) -> Result<ProviderConnection, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before listing provider models.")?;
    let provider = state
        .database()?
        .provider(&id)?
        .ok_or("Provider no longer exists.")?;
    let key = resolve_provider_key(&state, &provider).await?;
    let result = providers::list_models(&provider, &key).await?;
    if !result.supported {
        return Err(
            "This provider does not expose model listing. Enter a model ID manually.".into(),
        );
    }
    let models = providers::merge_listed_models(&provider.models, &result.details);
    state.database()?.update_provider_models(&id, &models)?;
    let mut updated = provider;
    updated.models = models;
    updated.has_api_key = state
        .daytona_vault
        .contains(&providers::credential_id(&id))?;
    Ok(updated)
}

#[tauri::command]
pub fn preferred_model(state: State<'_, AppState>) -> Result<ModelSelection, String> {
    state.database()?.preferred_model()
}

#[tauri::command]
pub async fn save_preferred_model(
    state: State<'_, AppState>,
    selection: ModelSelection,
) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before changing models.")?;
    state.database()?.save_preferred_model(&selection)
}

#[tauri::command]
pub async fn save_conversation_model(
    state: State<'_, AppState>,
    id: String,
    selection: ModelSelection,
) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before changing models.")?;
    state.database()?.save_conversation_model(&id, &selection)
}
#[tauri::command]
pub fn create_conversation(
    state: State<'_, AppState>,
    project_id: Option<String>,
) -> Result<Conversation, String> {
    let store = state.database()?;
    let conversation = store.create_conversation()?;
    // Chats always file under a folder: assign one in the same call so the row
    // is never unfiled, even when project_id is absent (falls back to the first
    // project) or stale.
    crate::workspace_ui::file_task(&store, &conversation.id, project_id.as_deref())?;
    Ok(conversation)
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
    state.automation.stop_conversation(&id).await;
    state.previews.stop_conversation(&id);
    // Stop every child this conversation owns before its rows disappear.
    // `process_stop` and `terminal_close` need the id, and the conversation
    // that produced it no longer exists, so a running child would otherwise
    // be left with no reachable handle.
    state.processes.stop_owned(&id);
    state.terminals.lock().await.close_owned(&id).await;
    let store = state.database()?;
    store.delete_conversation(&id)?;
    crate::workspace_ui::remove_task_meta(&store, &id)
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
    {
        let store = state.database()?;
        store.save_runtime_config(&config)?;
        let mut preferences = store.preferences()?;
        let fitted =
            crate::context::fit_response_budget(preferences.max_tokens, config.context_length);
        if fitted != preferences.max_tokens {
            preferences.max_tokens = fitted;
            store.save_preferences(&preferences)?;
        }
    }
    crate::model_library::save_active_profile(&state, &config)?;
    Ok(())
}
#[tauri::command]
pub fn save_preferences(
    state: State<'_, AppState>,
    mut preferences: Preferences,
) -> Result<(), String> {
    let store = state.database()?;
    let config = store.runtime_config()?;
    preferences.max_tokens =
        crate::context::fit_response_budget(preferences.max_tokens, config.context_length);
    store.save_preferences(&preferences)
}
#[tauri::command]
pub async fn load_model(state: State<'_, AppState>) -> Result<RuntimeStatus, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "A model operation is already in progress.")?;
    load_selected_model(&state).await
}

pub(crate) async fn load_selected_model(state: &AppState) -> Result<RuntimeStatus, String> {
    let (mut preferences, config) = {
        let store = state.database()?;
        (store.preferences()?, store.runtime_config()?)
    };
    configure_local_model_runtime(&mut preferences, state)?;
    let mut preferences = preferences.apply_model_defaults();
    preferences.max_tokens =
        crate::context::fit_response_budget(preferences.max_tokens, config.context_length);
    {
        let store = state.database()?;
        store.save_preferences(&preferences)?;
        store.save_runtime_config(&config)?;
    }
    crate::model_library::save_active_profile(state, &config)?;
    state.runtime.lock().await.load(&preferences, &config).await
}

fn find_standard_runtime(data_dir: &Path, project_root: &Path) -> Option<std::path::PathBuf> {
    let development = project_root.join(crate::model_catalog::STANDARD_RUNTIME_RELATIVE);
    if development.is_file() {
        return Some(development);
    }
    let root = data_dir.join("runtimes");
    let mut candidates = std::fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            name.starts_with("llama-b10855-cuda12.4-")
                .then(|| entry.path().join("llama-server.exe"))
        })
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    candidates.sort();
    candidates.into_iter().next()
}

/// Ensure a llama-server executable is selected before loading. An empty
/// runtime path falls back to the managed standard runtime; user-selected
/// executables are left untouched.
fn configure_local_model_runtime(
    preferences: &mut crate::store::Preferences,
    state: &AppState,
) -> Result<(), String> {
    if !preferences.runtime_path.is_empty() {
        return Ok(());
    }
    let Some(project_root) = Path::new(env!("CARGO_MANIFEST_DIR")).parent() else {
        return Ok(());
    };
    let Some(runtime) = find_standard_runtime(&state.data_dir, project_root) else {
        return Err("Select a native llama-server executable for your operating system using Browse runtime in Model files, save settings, then load the model.".into());
    };
    let canonical = std::fs::canonicalize(&runtime)
        .map_err(|error| format!("Standard runtime exists but cannot be resolved: {error}"))?;
    preferences.runtime_path = canonical.to_string_lossy().into_owned();
    state.database()?.save_preferences(preferences)?;
    Ok(())
}

#[cfg(test)]
mod model_runtime_tests {
    use super::*;

    #[test]
    fn finds_development_standard_runtime_before_managed_copies() {
        let project = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let development = project.path().join(".local/runtime/llama-server.exe");
        std::fs::create_dir_all(development.parent().unwrap()).unwrap();
        std::fs::write(&development, b"runtime").unwrap();
        assert_eq!(
            find_standard_runtime(data.path(), project.path()),
            Some(development)
        );
    }
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
    let provider = state
        .database()?
        .provider(&id)?
        .ok_or("Provider no longer exists.")?;
    // This is the explicit manual-verification path for providers without
    // model listing — user-configured endpoints may answer over HTTPS, the
    // same rules chat inference follows.
    if providers::is_subscription_format(&provider.api_format) {
        crate::local_only::inference_allowed(&provider.base_url)?;
    }
    let key = resolve_provider_key(&state, &provider).await?;
    let result = match providers::test_inference(&provider, &key, &model_id).await {
        Ok(result) => result,
        Err(error) => {
            // A failed inference check must not leave a stale verified flag.
            state.database()?.update_provider_test(
                &id,
                false,
                Some(crate::store::now()),
                &provider.models,
            )?;
            return Err(error);
        }
    };
    state.database()?.update_provider_test(
        &id,
        true,
        Some(crate::store::now()),
        &provider.models,
    )?;
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
    state.database()?.latest_run(&conversation_id)
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
pub fn detect_subscription_cli() -> crate::subscription_auth::CliDetectionResult {
    crate::subscription_auth::detect_cli_sessions()
}

#[tauri::command]
pub async fn import_subscription_cli(
    state: State<'_, AppState>,
    provider: String,
) -> Result<crate::subscription_auth::SubscriptionStatus, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before syncing credentials.")?;
    crate::subscription_auth::import_cli_session(&state.daytona_vault, &provider)?;
    if let Ok(store) = state.database() {
        let provider_id = if provider == "chatgpt" {
            "chatgpt-subscription"
        } else {
            "grok-subscription"
        };
        if let Ok(Some(prov)) = store.provider(provider_id) {
            let _ = store.update_provider_test(
                provider_id,
                true,
                Some(crate::store::now()),
                &prov.models,
            );
        }
    }
    crate::subscription_auth::get_subscription_status(&state.daytona_vault, &provider)
}

#[tauri::command]
pub fn get_subscription_status(
    state: State<'_, AppState>,
    provider: String,
) -> Result<crate::subscription_auth::SubscriptionStatus, String> {
    crate::subscription_auth::get_subscription_status(&state.daytona_vault, &provider)
}

#[tauri::command]
pub async fn save_manual_subscription_token(
    state: State<'_, AppState>,
    provider: String,
    token: String,
    refresh_token: Option<String>,
    account_id: Option<String>,
) -> Result<crate::subscription_auth::SubscriptionStatus, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before saving credentials.")?;
    crate::subscription_auth::save_manual_token(
        &state.daytona_vault,
        &provider,
        &token,
        refresh_token.as_deref(),
        account_id.as_deref(),
    )?;
    if let Ok(store) = state.database() {
        let provider_id = if provider == "chatgpt" {
            "chatgpt-subscription"
        } else {
            "grok-subscription"
        };
        if let Ok(Some(prov)) = store.provider(provider_id) {
            let _ = store.update_provider_test(
                provider_id,
                true,
                Some(crate::store::now()),
                &prov.models,
            );
        }
    }
    crate::subscription_auth::get_subscription_status(&state.daytona_vault, &provider)
}

#[tauri::command]
pub async fn disconnect_subscription(
    state: State<'_, AppState>,
    provider: String,
) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before disconnecting.")?;
    crate::subscription_auth::clear_session(&state.daytona_vault, &provider)?;
    if let Ok(store) = state.database() {
        let provider_id = if provider == "chatgpt" {
            "chatgpt-subscription"
        } else {
            "grok-subscription"
        };
        if let Ok(Some(prov)) = store.provider(provider_id) {
            let _ = store.update_provider_test(provider_id, false, None, &prov.models);
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn start_subscription_sign_in(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    provider: String,
) -> Result<crate::subscription_auth::SubscriptionStatus, String> {
    let _operation = state
        .oauth_operation
        .try_lock()
        .map_err(|_| "Another account sign-in is already in progress.")?;
    state.oauth_cancel.send_replace(false);
    crate::subscription_auth::sign_in_subscription(
        &app,
        &provider,
        state.daytona_vault.clone(),
        state.oauth_cancel.subscribe(),
    )
    .await?;
    if let Ok(store) = state.database() {
        let provider_id = if provider == "chatgpt" {
            "chatgpt-subscription"
        } else {
            "grok-subscription"
        };
        if let Ok(Some(prov)) = store.provider(provider_id) {
            let _ = store.update_provider_test(
                provider_id,
                true,
                Some(crate::store::now()),
                &prov.models,
            );
        }
    }
    crate::subscription_auth::get_subscription_status(&state.daytona_vault, &provider)
}

#[tauri::command]
pub fn cancel_subscription_sign_in(state: State<'_, AppState>) {
    state.oauth_cancel.send_replace(true);
}
