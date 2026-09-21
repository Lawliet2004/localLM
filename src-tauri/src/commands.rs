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
    let store = state.database()?;
    store.delete_conversation(&id)?;
    crate::workspace_ui::remove_task_meta(&store, &id)?;
    drop(store);
    // The saved KV state holds this conversation's content; the index row
    // cascades with the conversation, the file is removed here.
    crate::kv_slots::delete_for_conversation(&state, &id);
    Ok(())
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
        let fitted = crate::context::fit_response_budget(preferences.max_tokens, config.context_length);
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
    let (mut preferences, mut config) = {
        let store = state.database()?;
        (store.preferences()?, store.runtime_config()?)
    };
    // Resolve model-specific runtime requirements before starting a process.
    // Catalog models are not interchangeable: ZAYA needs its custom build,
    // legacy Bonsai 8B Q2_0 needs prism-b9601, Ternary Bonsai 2 needs
    // prism-b10709+, and MiniCPM uses standard llama.cpp. Reading the
    // architecture is a best-effort enhancement; the catalog filename still
    // identifies the managed models when metadata cannot be parsed.
    let architecture = crate::gguf::read_architecture(std::path::Path::new(&preferences.model_path))
        .ok()
        .flatten();
    configure_local_model_runtime(&mut preferences, &mut config, state, architecture.as_deref())?;
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

fn selected_model_filename(model_path: &str) -> Option<&str> {
    Path::new(model_path).file_name().and_then(|value| value.to_str())
}

fn is_prism_runtime(runtime_path: &str) -> bool {
    runtime_path
        .replace('\\', "/")
        .to_ascii_lowercase()
        .contains("runtime-prism-b9601-68faa14")
}

fn is_bonsai2_runtime(runtime_path: &str) -> bool {
    runtime_path
        .replace('\\', "/")
        .to_ascii_lowercase()
        .contains("runtime-prism-b10709-9a9394a")
}

fn is_standard_runtime(runtime_path: &str, data_dir: &Path, project_root: &Path) -> bool {
    if managed_runtime_selected(runtime_path, data_dir) {
        return true;
    }
    let expected = project_root.join(crate::model_catalog::STANDARD_RUNTIME_RELATIVE);
    std::fs::canonicalize(expected)
        .ok()
        .zip(std::fs::canonicalize(runtime_path).ok())
        .is_some_and(|(expected, selected)| expected == selected)
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

fn find_prism_runtime(project_root: &Path) -> Option<std::path::PathBuf> {
    let runtime = project_root.join(crate::model_catalog::BONSAI_RUNTIME_RELATIVE);
    runtime.is_file().then_some(runtime)
}

fn find_bonsai2_runtime(project_root: &Path) -> Option<std::path::PathBuf> {
    let runtime = project_root.join(crate::model_catalog::BONSAI2_RUNTIME_RELATIVE);
    runtime.is_file().then_some(runtime)
}

/// Select the correct local runtime for a managed model and repair settings
/// inherited from a different model. User-selected custom executables remain
/// untouched unless they are one of the app's known incompatible profiles.
fn configure_local_model_runtime(
    preferences: &mut crate::store::Preferences,
    config: &mut crate::runtime_config::RuntimeConfig,
    state: &AppState,
    architecture: Option<&str>,
) -> Result<(), String> {
    let Some(project_root) = Path::new(env!("CARGO_MANIFEST_DIR")).parent() else {
        return Ok(());
    };
    let filename = selected_model_filename(&preferences.model_path);
    let is_zaya = filename == Some(crate::model_catalog::ZAYA1_FILENAME) || architecture == Some("zaya");
    if is_zaya {
        if !is_zaya_runtime(&preferences.runtime_path) {
            zaya_auto_configure(preferences, config, state)?;
        }
        return Ok(());
    }

    let is_bonsai2 = filename.is_some_and(crate::model_catalog::is_bonsai2_filename);
    if is_bonsai2 {
        if !is_bonsai2_runtime(&preferences.runtime_path) {
            bonsai2_auto_configure(preferences, config, state, project_root)?;
        }
        return Ok(());
    }

    let is_bonsai = filename == Some(crate::model_catalog::BONSAI_FILENAME);
    if is_bonsai {
        if normalize_bonsai_context(config) {
            // A previous ZAYA/default profile can exceed Bonsai's hard limit.
            // Keep the user's other tuning choices and move to the model's
            // advertised maximum rather than silently shrinking the context.
            state.database()?.save_runtime_config(config)?;
        }
        let incompatible = preferences.runtime_path.is_empty()
            || is_zaya_runtime(&preferences.runtime_path)
            || is_bonsai2_runtime(&preferences.runtime_path)
            || is_standard_runtime(&preferences.runtime_path, &state.data_dir, project_root);
        if !is_prism_runtime(&preferences.runtime_path) && incompatible {
            let Some(runtime) = find_prism_runtime(project_root) else {
                return Err("Ternary Bonsai Q2_0 requires the Prism prism-b9601-68faa14 runtime. Select its llama-server.exe in Models, then load the model.".into());
            };
            let canonical = std::fs::canonicalize(&runtime)
                .map_err(|error| format!("Prism runtime exists but cannot be resolved: {error}"))?;
            preferences.runtime_path = canonical.to_string_lossy().into_owned();
            state.database()?.save_preferences(preferences)?;
        }
        return Ok(());
    }

    {
        let needs_standard = preferences.runtime_path.is_empty()
            || is_zaya_runtime(&preferences.runtime_path)
            || is_prism_runtime(&preferences.runtime_path)
            || is_bonsai2_runtime(&preferences.runtime_path);
        if needs_standard {
            let Some(runtime) = find_standard_runtime(&state.data_dir, project_root) else {
                return Err("Install a standard llama.cpp runtime or select a compatible llama-server.exe in Models, then load the model.".into());
            };
            let canonical = std::fs::canonicalize(&runtime)
                .map_err(|error| format!("Standard runtime exists but cannot be resolved: {error}"))?;
            preferences.runtime_path = canonical.to_string_lossy().into_owned();
            state.database()?.save_preferences(preferences)?;
        }
    }
    Ok(())
}

fn normalize_bonsai_context(config: &mut crate::runtime_config::RuntimeConfig) -> bool {
    // Preserve deliberate smaller contexts, including the 8K recommendation.
    if config.context_length > crate::model_catalog::BONSAI_CONTEXT_LENGTH
    {
        config.context_length = crate::model_catalog::BONSAI_CONTEXT_LENGTH;
        true
    } else {
        false
    }
}

/// Returns true if the selected runtime path is the dedicated ZAYA build.
fn is_zaya_runtime(runtime_path: &str) -> bool {
    if runtime_path.is_empty() {
        return false;
    }
    let Some(project_root) = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent() else {
        return false;
    };
    let expected = project_root.join(crate::model_catalog::ZAYA1_RUNTIME_RELATIVE);
    let Ok(expected_canonical) = std::fs::canonicalize(&expected) else {
        return false;
    };
    let Ok(selected_canonical) = std::fs::canonicalize(runtime_path) else {
        return false;
    };
    selected_canonical == expected_canonical
}

/// When loading a ZAYA1-8B model, automatically locate the pre-built custom
/// runtime and apply the known-good CPU configuration so the user does not
/// need to manually configure anything.
fn zaya_auto_configure(
    preferences: &mut crate::store::Preferences,
    config: &mut crate::runtime_config::RuntimeConfig,
    state: &AppState,
) -> Result<(), String> {
    // Locate the pre-built custom runtime from llama.cpp PR #23112.
    // The build script (`scripts/prepare-zaya-runtime.ps1`) places it relative
    // to the project root, which is one level above CARGO_MANIFEST_DIR.
    let project_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or("Could not determine project root.")?;
    let custom_runtime = project_root.join(crate::model_catalog::ZAYA1_RUNTIME_RELATIVE);
    if !custom_runtime.is_file() {
        return Err(format!(
            "ZAYA1-8B requires a custom runtime built from llama.cpp PR #23112, \
             but it was not found at:\n  {}\n\n\
             Build it first by running:\n  ./scripts/prepare-zaya-runtime.ps1\n\n\
             Then click Load model again — the app will configure everything automatically.\n\
             Details: docs/ZAYA1-RUNTIME.md",
            custom_runtime.display()
        ));
    }
    let canonical = std::fs::canonicalize(&custom_runtime)
        .map_err(|error| format!("Custom ZAYA runtime exists but cannot be resolved: {error}"))?;
    // Apply the custom runtime path and ZAYA-optimal placement. Keep a
    // user-chosen context unless it is still the generic default, which would
    // allocate a 32k KV cache this CPU-only path does not need.
    preferences.runtime_path = canonical.to_string_lossy().into_owned();
    apply_zaya_recommended(config);
    // Persist so the UI reflects the changes and subsequent loads reuse them.
    let store = state.database()?;
    store.save_preferences(preferences)?;
    store.save_runtime_config(config)?;
    Ok(())
}

fn apply_zaya_recommended(config: &mut crate::runtime_config::RuntimeConfig) {
    let keep_context = config.context_length;
    *config = crate::runtime_config::RuntimeConfig::zaya_recommended();
    if keep_context != crate::runtime_config::RuntimeConfig::default().context_length {
        config.context_length = keep_context;
    }
}

fn bonsai2_auto_configure(
    preferences: &mut crate::store::Preferences,
    config: &mut crate::runtime_config::RuntimeConfig,
    state: &AppState,
    project_root: &Path,
) -> Result<(), String> {
    let Some(runtime) = find_bonsai2_runtime(project_root) else {
        return Err(
            "Ternary Bonsai 2 (PTQ1_0 / PQ2_0) requires the PrismML llama.cpp fork prism-b10709 or newer. \
             Stock llama.cpp refuses these files (invalid ggml type 143). Install it with:\n  \
             powershell -ExecutionPolicy Bypass -File scripts/prepare-runtime.ps1 -Bonsai2\n\
             Then click Use model again. The older prism-b9601 runtime is only for Ternary Bonsai 8B Q2_0."
                .into(),
        );
    };
    let canonical = std::fs::canonicalize(&runtime)
        .map_err(|error| format!("Prism Bonsai 2 runtime exists but cannot be resolved: {error}"))?;
    preferences.runtime_path = canonical.to_string_lossy().into_owned();
    // First switch onto this runtime: drop a leftover projector from another
    // model so a 4 GB GPU is not asked to hold a 0.6 GB vision tower as well.
    preferences.projector_path.clear();
    *config = crate::runtime_config::RuntimeConfig::bonsai2_recommended();
    let store = state.database()?;
    store.save_preferences(preferences)?;
    store.save_runtime_config(config)?;
    Ok(())
}

/// True when the selected runtime executable lives in the app-managed
/// runtimes directory (as opposed to a user-built custom binary).
#[allow(dead_code)]
fn managed_runtime_selected(runtime_path: &str, data_dir: &Path) -> bool {
    if runtime_path.is_empty() {
        return false;
    }
    let runtimes = data_dir.join("runtimes");
    let Ok(managed) = std::fs::canonicalize(&runtimes) else {
        return false;
    };
    std::fs::canonicalize(runtime_path)
        .map(|exe| exe.starts_with(&managed))
        .unwrap_or(false)
}

#[cfg(test)]
mod model_runtime_tests {
    use super::*;

    #[test]
    fn identifies_catalog_models_and_runtime_profiles() {
        assert_eq!(selected_model_filename(r"C:\models\MiniCPM5-2B.Q6_K.gguf"), Some(crate::model_catalog::MODEL_FILENAME));
        assert_eq!(selected_model_filename("/models/Ternary-Bonsai-8B-Q2_0.gguf"), Some(crate::model_catalog::BONSAI_FILENAME));
        assert!(crate::model_catalog::is_bonsai2_filename("Ternary-Bonsai-2-27B-PTQ1_0.gguf"));
        assert!(is_prism_runtime(r"C:\LocalLM\.local\runtime-prism-b9601-68faa14\llama-server.exe"));
        assert!(!is_prism_runtime(r"C:\LocalLM\.local\runtime\llama-server.exe"));
        assert!(is_bonsai2_runtime(r"C:\LocalLM\.local\runtime-prism-b10709-9a9394a\llama-server.exe"));
        assert!(!is_bonsai2_runtime(r"C:\LocalLM\.local\runtime-prism-b9601-68faa14\llama-server.exe"));
        assert!(!is_prism_runtime(r"C:\LocalLM\.local\runtime-prism-b10709-9a9394a\llama-server.exe"));
    }

    #[test]
    fn normalizes_bonsai_to_its_full_model_context() {
        let mut config = RuntimeConfig { context_length: 131_072, ..RuntimeConfig::default() };
        assert!(normalize_bonsai_context(&mut config));
        assert_eq!(config.context_length, crate::model_catalog::BONSAI_CONTEXT_LENGTH);

        config.context_length = 4_096;
        assert!(!normalize_bonsai_context(&mut config));
        assert_eq!(config.context_length, 4_096);

        config.context_length = 8_192;
        assert!(!normalize_bonsai_context(&mut config));
        assert_eq!(config.context_length, 8_192);
    }

    #[test]
    fn zaya_auto_config_keeps_a_saved_context_and_replaces_the_generic_default() {
        let mut custom = RuntimeConfig {
            context_length: 65_536,
            gpu_layers: -1,
            flash_attention: true,
            ..RuntimeConfig::default()
        };
        apply_zaya_recommended(&mut custom);
        assert_eq!(custom.context_length, 65_536);
        assert_eq!(custom.gpu_layers, 0);
        assert!(!custom.flash_attention);

        let mut leftover = RuntimeConfig::default();
        apply_zaya_recommended(&mut leftover);
        assert_eq!(leftover.context_length, 8192);
        assert_eq!(leftover.gpu_layers, 0);
    }

    #[test]
    fn finds_development_standard_runtime_before_managed_copies() {
        let project = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let development = project.path().join(".local/runtime/llama-server.exe");
        std::fs::create_dir_all(development.parent().unwrap()).unwrap();
        std::fs::write(&development, b"runtime").unwrap();
        assert_eq!(find_standard_runtime(data.path(), project.path()), Some(development));
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
    let provider = state.database()?.provider(&id)?.ok_or("Provider no longer exists.")?;
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
pub fn list_conversation_artifacts(
    state: State<'_, AppState>,
    conversation_id: String,
) -> Result<Vec<crate::artifacts::ArtifactRecord>, String> {
    state.database()?.artifacts_for_conversation(&conversation_id)
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
        let provider_id = if provider == "chatgpt" { "chatgpt-subscription" } else { "grok-subscription" };
        if let Ok(Some(prov)) = store.provider(provider_id) {
            let _ = store.update_provider_test(provider_id, true, Some(crate::store::now()), &prov.models);
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
        let provider_id = if provider == "chatgpt" { "chatgpt-subscription" } else { "grok-subscription" };
        if let Ok(Some(prov)) = store.provider(provider_id) {
            let _ = store.update_provider_test(provider_id, true, Some(crate::store::now()), &prov.models);
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
        let provider_id = if provider == "chatgpt" { "chatgpt-subscription" } else { "grok-subscription" };
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
        let provider_id = if provider == "chatgpt" { "chatgpt-subscription" } else { "grok-subscription" };
        if let Ok(Some(prov)) = store.provider(provider_id) {
            let _ = store.update_provider_test(provider_id, true, Some(crate::store::now()), &prov.models);
        }
    }
    crate::subscription_auth::get_subscription_status(&state.daytona_vault, &provider)
}

#[tauri::command]
pub fn cancel_subscription_sign_in(state: State<'_, AppState>) {
    state.oauth_cancel.send_replace(true);
}



#[cfg(test)]
mod runtime_guard_tests {
    use super::*;

    #[test]
    fn only_managed_runtime_paths_are_treated_as_managed() {
        let directory = tempfile::tempdir().unwrap();
        let runtimes = directory.path().join("runtimes");
        std::fs::create_dir_all(runtimes.join("prism")).unwrap();
        let managed_exe = runtimes.join("prism").join("llama-server.exe");
        std::fs::write(&managed_exe, b"stub").unwrap();
        let custom_exe = directory.path().join("llama-server.exe");
        std::fs::write(&custom_exe, b"stub").unwrap();
        assert!(managed_runtime_selected(managed_exe.to_str().unwrap(), directory.path()));
        assert!(!managed_runtime_selected(custom_exe.to_str().unwrap(), directory.path()), "custom runtimes must keep working for patched builds");
        assert!(!managed_runtime_selected("", directory.path()));
        assert!(!managed_runtime_selected("Z:/definitely/missing/llama-server.exe", directory.path()));
    }
}
