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

fn provider_views(store: &crate::store::Store, vault: &crate::vault::Vault) -> Result<Vec<ProviderConnection>, String> {
    store.providers()?.into_iter().map(|mut provider| {
        if provider.id == "chatgpt-subscription" || provider.api_format == providers::CHATGPT_SUBSCRIPTION {
            provider.has_api_key = crate::subscription_auth::load_session(vault, "chatgpt")?.is_some();
        } else if provider.id == "grok-subscription" || provider.api_format == providers::GROK_SUBSCRIPTION {
            provider.has_api_key = crate::subscription_auth::load_session(vault, "grok")?.is_some();
        } else {
            provider.has_api_key = vault.load(&providers::credential_id(&provider.id))?.is_some();
        }
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

async fn resolve_provider_key(
    state: &State<'_, AppState>,
    provider: &ProviderConnection,
) -> Result<String, String> {
    if provider.api_format == providers::CHATGPT_SUBSCRIPTION || provider.id == "chatgpt-subscription" {
        crate::subscription_auth::get_valid_access_token(&state.daytona_vault, "chatgpt").await
    } else if provider.api_format == providers::GROK_SUBSCRIPTION || provider.id == "grok-subscription" {
        crate::subscription_auth::get_valid_access_token(&state.daytona_vault, "grok").await
    } else {
        match state
            .daytona_vault
            .load(&providers::credential_id(&provider.id))?
        {
            Some(bytes) => String::from_utf8(bytes).map_err(|_| "Saved provider API key is invalid.".to_string()),
            // Loopback engines may run unauthenticated; the test still proves
            // the endpoint answers.
            None if providers::is_loopback_base_url(&provider.base_url) => Ok(String::new()),
            None => Err("Save an API key before testing this provider.".into()),
        }
    }
}

#[tauri::command]
pub async fn test_provider(state: State<'_, AppState>, id: String) -> Result<ProviderTestResult, String> {
    let _operation = state.operation.try_lock().map_err(|_| "Wait for the active operation before testing a provider.")?;
    let provider = state.database()?.provider(&id)?.ok_or("Provider no longer exists.")?;
    let key = resolve_provider_key(&state, &provider).await?;
    let result = match providers::test_connection(&provider, &key).await {
        Ok(result) => result,
        Err(error) => {
            state.database()?.update_provider_test(&id, false, Some(crate::store::now()), &provider.models)?;
            return Err(error);
        }
    };
    let models = if result.model_list_supported { providers::merge_listed_models(&provider.models, &result.models) } else { provider.models.clone() };
    state.database()?.update_provider_test(&id, result.verified, Some(crate::store::now()), &models)?;
    if result.verified {
        // A live test proves the stored key works — stamp last_verified_at.
        let _ = state.daytona_vault.mark_verified(&providers::credential_id(&provider.id));
    }
    Ok(result)
}

#[tauri::command]
pub async fn list_provider_models(state: State<'_, AppState>, id: String) -> Result<ProviderConnection, String> {
    let _operation = state.operation.try_lock().map_err(|_| "Wait for the active operation before listing provider models.")?;
    let provider = state.database()?.provider(&id)?.ok_or("Provider no longer exists.")?;
    let key = resolve_provider_key(&state, &provider).await?;
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
pub fn create_conversation(state: State<'_, AppState>, project_id: Option<String>) -> Result<Conversation, String> {
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
    persist_runtime_config(&state, &config)
}
/// Preferences (with the fitted response budget) and the configuration are
/// committed in one transaction, then the active model profile records the
/// saved runtime so switching away and back keeps it.
pub(crate) fn persist_runtime_config(state: &AppState, config: &RuntimeConfig) -> Result<(), String> {
    {
        let store = state.database()?;
        let mut preferences = store.preferences()?;
        let fitted = crate::context::fit_response_budget(preferences.max_tokens, config.context_length);
        if fitted != preferences.max_tokens {
            preferences.max_tokens = fitted;
        }
        store.save_model_runtime(&preferences, config)?;
    }
    crate::model_library::save_active_profile(state, config)
}
#[tauri::command]
pub fn save_preferences(
    state: State<'_, AppState>,
    preferences: Preferences,
) -> Result<(), String> {
    persist_preferences(&state, preferences)
}
/// The active profile keeps the executable alongside the model so a later Use
/// restores the newly saved runtime for this model.
pub(crate) fn persist_preferences(state: &AppState, mut preferences: Preferences) -> Result<(), String> {
    let config = {
        let store = state.database()?;
        let config = store.runtime_config()?;
        preferences.max_tokens =
            crate::context::fit_response_budget(preferences.max_tokens, config.context_length);
        store.save_model_runtime(&preferences, &config)?;
        config
    };
    crate::model_library::save_active_profile(state, &config)
}
#[tauri::command]
pub async fn load_model(state: State<'_, AppState>) -> Result<RuntimeStatus, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "A model operation is already in progress.")?;
    match load_selected_model(&state).await {
        Ok(status) => Ok(status),
        Err(error) => {
            let error = preserve_load_failure(&state, &error, &failure_log(&state));
            Err(error)
        }
    }
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
    let assets = crate::runtime_archive::catalog().ok()?;
    crate::runtime_inventory::find_complete(
        &data_dir.join("runtimes"),
        crate::runtime_install::STANDARD_PREFIX,
        &assets,
    )
    .ok()
    .flatten()
}

/// Ensure a llama-server executable is selected before loading. An empty
/// runtime path falls back to the managed standard runtime; user-selected
/// executables are left untouched. The caller persists the updated
/// preferences — this is discovery, not a save.
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
        return Err("Install a standard llama.cpp runtime or select a compatible llama-server.exe in Models, then load the model.".into());
    };
    let canonical = std::fs::canonicalize(&runtime)
        .map_err(|error| format!("Standard runtime exists but cannot be resolved: {error}"))?;
    preferences.runtime_path = canonical.to_string_lossy().into_owned();
    Ok(())
}

fn failure_log(state: &AppState) -> std::path::PathBuf {
    state.data_dir.join("last-load-failure.log")
}

/// Copy the failed load's runtime log aside and translate well-known causes
/// (GPU OOM) into actionable text. Loads use the data-root path.
pub(crate) fn preserve_load_failure(
    state: &AppState,
    error: &str,
    destination: &Path,
) -> String {
    let log = std::fs::read(state.data_dir.join("runtime.log")).unwrap_or_default();
    let saved = std::fs::create_dir_all(destination.parent().unwrap_or(destination))
        .and_then(|()| std::fs::write(destination, &log))
        .is_ok();
    let lower = String::from_utf8_lossy(&log).to_lowercase();
    let details = if saved {
        format!(" See {}", destination.display())
    } else {
        String::new()
    };
    if lower.contains("out of memory")
        || lower.contains("cudamalloc failed")
        || lower.contains("failed to allocate cuda")
    {
        format!("Insufficient GPU memory. Reduce GPU layers, context, or batch size and retry.{details}")
    } else {
        format!("{error}{details}")
    }
}

/// Commit a new model selection and load it as one recoverable operation.
/// Validation happens before anything is persisted; on a load failure the
/// previous selection is restored and the previous model reloaded when it had
/// been running.
pub(crate) async fn switch_and_load(
    state: &AppState,
    next: Preferences,
    config: RuntimeConfig,
) -> Result<RuntimeStatus, String> {
    config.validate()?;
    next.validate()?;
    if !std::fs::canonicalize(&next.model_path)
        .map(|path| path.is_file())
        .unwrap_or(false)
    {
        return Err("Select an existing GGUF model in Models.".into());
    }
    let projector = next.projector_path.trim();
    if !projector.is_empty()
        && !std::fs::canonicalize(projector)
            .map(|path| path.is_file())
            .unwrap_or(false)
    {
        return Err("Select an existing mmproj projector file in Models, or clear it for text-only chat.".into());
    }
    // Resolve the executable on a copy first: an empty path falls back
    // to the managed standard runtime.
    let mut resolved = next;
    configure_local_model_runtime(&mut resolved, state)?;
    if resolved.runtime_path.is_empty()
        || !std::fs::canonicalize(&resolved.runtime_path)
            .map(|path| path.is_file())
            .unwrap_or(false)
    {
        return Err("Select an existing llama-server executable in Models.".into());
    }
    let (previous_preferences, previous_config) = {
        let store = state.database()?;
        (store.preferences()?, store.runtime_config()?)
    };
    let was_ready = state.runtime.lock().await.inspect().phase == "ready";
    state.database()?.save_model_runtime(&resolved, &config)?;
    crate::model_library::save_active_profile(state, &config)?;
    match load_selected_model(state).await {
        Ok(status) => Ok(status),
        Err(error) => {
            let error = preserve_load_failure(state, &error, &failure_log(state));
            {
                let store = state.database()?;
                store.save_model_runtime(&previous_preferences, &previous_config)?;
            }
            let still_ready = state.runtime.lock().await.inspect().phase == "ready";
            // Runtime::load only stops the old process after its own
            // validation; a still-ready phase means it never got that far.
            let outcome = if still_ready {
                "previous model is still running".to_string()
            } else if was_ready {
                match load_selected_model(state).await {
                    Ok(_) => "previous model reloaded".to_string(),
                    Err(reload) => format!("previous model could not be reloaded: {reload}"),
                }
            } else {
                "no model was running".to_string()
            };
            Err(format!("{error}. Previous model settings restored; {outcome}."))
        }
    }
}

#[cfg(test)]
mod model_runtime_tests {
    use super::*;
    use crate::web_search::tests::test_state;

    #[test]
    fn finds_development_standard_runtime_before_managed_copies() {
        let project = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let development = project.path().join(".local/runtime/llama-server.exe");
        std::fs::create_dir_all(development.parent().unwrap()).unwrap();
        std::fs::write(&development, b"runtime").unwrap();
        assert_eq!(find_standard_runtime(data.path(), project.path()), Some(development));
    }

    #[tokio::test]
    async fn switch_load_rejects_a_missing_model_before_committing() {
        let directory = tempfile::tempdir().unwrap();
        let state = test_state(directory.path());
        let (preferences, config) = {
            let store = state.database().unwrap();
            (store.preferences().unwrap(), store.runtime_config().unwrap())
        };
        let next = Preferences {
            model_path: directory.path().join("missing.gguf").to_string_lossy().into_owned(),
            ..preferences.clone()
        };
        let error = switch_and_load(&state, next, config.clone()).await.err().unwrap();
        assert!(error.contains("GGUF"), "{error}");
        let store = state.database().unwrap();
        assert_eq!(store.preferences().unwrap().model_path, preferences.model_path);
        assert_eq!(store.runtime_config().unwrap(), config);
    }

    #[tokio::test]
    async fn switch_load_rejects_a_missing_runtime_before_committing() {
        let directory = tempfile::tempdir().unwrap();
        let state = test_state(directory.path());
        let model = directory.path().join("model.gguf");
        std::fs::write(&model, b"GGUF").unwrap();
        let (preferences, config) = {
            let store = state.database().unwrap();
            (store.preferences().unwrap(), store.runtime_config().unwrap())
        };
        let next = Preferences {
            model_path: model.to_string_lossy().into_owned(),
            runtime_path: directory.path().join("missing.exe").to_string_lossy().into_owned(),
            ..preferences.clone()
        };
        let error = switch_and_load(&state, next, config.clone()).await.err().unwrap();
        assert!(error.contains("llama-server"), "{error}");
        let store = state.database().unwrap();
        assert_eq!(store.preferences().unwrap().model_path, preferences.model_path);
    }

    #[tokio::test]
    async fn switch_load_restores_the_previous_selection_after_a_load_failure() {
        let directory = tempfile::tempdir().unwrap();
        let state = test_state(directory.path());
        let model = directory.path().join("model.gguf");
        std::fs::write(&model, b"GGUF").unwrap();
        // The file exists but cannot run, so the pre-spawn compatibility check fails.
        let exe = directory.path().join("llama-server.exe");
        std::fs::write(&exe, b"not an executable").unwrap();
        let (preferences, config) = {
            let store = state.database().unwrap();
            (store.preferences().unwrap(), store.runtime_config().unwrap())
        };
        let previous_model = preferences.model_path.clone();
        let next = Preferences {
            model_path: model.to_string_lossy().into_owned(),
            runtime_path: exe.to_string_lossy().into_owned(),
            ..preferences
        };
        let error = switch_and_load(&state, next, config.clone()).await.err().unwrap();
        assert!(error.contains("Previous model settings restored"), "{error}");
        let store = state.database().unwrap();
        assert_eq!(store.preferences().unwrap().model_path, previous_model);
        assert_eq!(store.runtime_config().unwrap(), config);
        assert!(directory.path().join("last-load-failure.log").is_file());
    }

    #[tokio::test]
    async fn runtime_snapshot_is_readable_while_the_runtime_is_locked() {
        let directory = tempfile::tempdir().unwrap();
        let state = test_state(directory.path());
        let mut runtime = state.runtime.lock().await;
        runtime.status.phase = "loading".into();
        runtime.status.message = "Loading model into memory".into();
        runtime.inspect();
        let phase = state
            .runtime_snapshot
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .phase
            .clone();
        assert_eq!(phase, "loading");
        assert!(state.runtime.try_lock().is_err());
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
    // A model load holds the runtime lock for its whole duration; status
    // queries must not queue behind it, so fall back to the snapshot the
    // runtime publishes on every mutation.
    if let Ok(mut runtime) = state.runtime.try_lock() {
        return Ok(runtime.inspect());
    }
    Ok(state
        .runtime_snapshot
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone())
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
    crate::local_only::inference_allowed(&provider.base_url)?;
    let key = resolve_provider_key(&state, &provider).await?;
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
pub fn detect_subscription_cli() -> crate::subscription_auth::CliDetectionResult {
    crate::subscription_auth::detect_cli_sessions()
}

/// Record whether the subscription provider currently has working credentials.
fn mark_subscription_tested(state: &AppState, provider: &str, tested: bool) {
    if tested {
        if let Ok(id) = crate::subscription_auth::vault_id_for_provider(provider) {
            let _ = state.daytona_vault.mark_verified(id);
        }
    }
    if let Ok(store) = state.database() {
        let provider_id = if provider == "chatgpt" { "chatgpt-subscription" } else { "grok-subscription" };
        if let Ok(Some(prov)) = store.provider(provider_id) {
            let at = if tested { Some(crate::store::now()) } else { None };
            let _ = store.update_provider_test(provider_id, tested, at, &prov.models);
        }
    }
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
    mark_subscription_tested(&state, &provider, true);
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
    mark_subscription_tested(&state, &provider, true);
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
    mark_subscription_tested(&state, &provider, false);
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
    mark_subscription_tested(&state, &provider, true);
    crate::subscription_auth::get_subscription_status(&state.daytona_vault, &provider)
}

#[tauri::command]
pub fn cancel_subscription_sign_in(state: State<'_, AppState>) {
    state.oauth_cancel.send_replace(true);
}

