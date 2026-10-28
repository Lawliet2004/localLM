//! One managed install operation: verify both assets before changing selection.
use crate::{
    model_catalog,
    model_install::{RunGuard, Status},
    runtime_config::{CacheType, RuntimeConfig},
};
use std::path::{Path, PathBuf};

pub fn is_integrated_model(path: &Path) -> bool {
    path.file_name().and_then(|s| s.to_str()) == Some(model_catalog::BONSAI_FILENAME)
}
pub fn requires_prism(path: &Path) -> bool {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_uppercase();
    name.contains("PQ2_0") || name.contains("PTQ1_0")
}
pub fn starting_config(cpu: bool) -> RuntimeConfig {
    RuntimeConfig {
        context_length: 16384,
        gpu_layers: if cpu { 0 } else { -1 },
        offload_kv_cache: !cpu,
        cache_type_k: CacheType::F16,
        cache_type_v: CacheType::F16,
        flash_attention: true,
        ..Default::default()
    }
}

/// Apply only at the local inference boundary, including subagent requests.
/// Keep user temperature/top-p preferences; use the model's remaining defaults.
pub fn apply_sampling(payload: &mut serde_json::Value) {
    let thinking_off = payload["chat_template_kwargs"]["enable_thinking"] == false;
    payload["top_k"] = serde_json::json!(20);
    payload["min_p"] = serde_json::json!(0.0);
    payload["presence_penalty"] = serde_json::json!(if thinking_off { 1.5 } else { 1.0 });
    payload["repeat_penalty"] = serde_json::json!(1.0);
    payload["repeat_last_n"] = serde_json::json!(64);
}

#[tauri::command]
pub async fn install_bonsai(
    state: tauri::State<'_, crate::AppState>,
    runtime_id: String,
    mtp_mode: Option<crate::runtime_config::MtpMode>,
) -> Result<crate::runtime::RuntimeStatus, String> {
    let package = crate::runtime_catalog::package(Some(&runtime_id))?;
    if !package.id.starts_with("prism-") || !package.supported() {
        return Err("Choose a Prism runtime for this operating system and architecture.".into());
    }
    let _installation = state
        .installation_operation
        .try_lock()
        .map_err(|_| "Another model or runtime installation is running.")?;
    let (sender, receiver) = tokio::sync::watch::channel(false);
    for installer in [&state.model_installer, &state.runtime_installer] {
        let mut inner = installer
            .inner
            .lock()
            .map_err(|_| "Installer unavailable.")?;
        inner.0 = Status {
            profile_id: Some("bonsai-2-9b".into()),
            busy: true,
            phase: "preparing".into(),
            ..Default::default()
        };
        inner.1 = Some(sender.clone());
    }
    let _model_guard = RunGuard(&state.model_installer);
    let _runtime_guard = RunGuard(&state.runtime_installer);
    let cpu = package.backend == "CPU";
    let result = async {
        let root = crate::managed_storage::root(&state.data_dir)?;
        // Check combined peak requirements before transferring either component.
        tokio::fs::create_dir_all(&root).await.map_err(|e| e.to_string())?;
        let required = crate::runtime_install::required_bytes(&package.assets)?
            .checked_add(model_catalog::BONSAI.bytes).ok_or("Installation size overflow.")?;
        if crate::download::available_space(&root)? < required {
            return Err(format!("Bonsai and Prism require {required} free bytes for download, extraction and reserve."));
        }
        let runtime = crate::runtime_install::install_package(&root.join("runtimes"), package, receiver.clone(), |phase, received, total| {
            if let Ok(mut inner) = state.runtime_installer.inner.lock() {
                inner.0.phase = phase.into(); inner.0.received = received; inner.0.total = total;
            }
        }).await?;
        {
            let mut inner = state.runtime_installer.inner.lock().map_err(|_| "Installer unavailable.")?;
            inner.0.phase = "ready".into();
            inner.0.path = Some(runtime.to_string_lossy().into_owned());
        }
        let models = root.join("models");
        tokio::fs::create_dir_all(&models).await.map_err(|e| e.to_string())?;
        let model = models.join(model_catalog::BONSAI_FILENAME);
        let exists = tokio::fs::try_exists(&model).await.map_err(|e| e.to_string())?;
        {
            let mut inner = state.model_installer.inner.lock().map_err(|_| "Installer unavailable.")?;
            inner.0.phase = if exists { "verifying" } else { "downloading" }.into();
            inner.0.total = model_catalog::BONSAI.bytes;
        }
        let progress = |received| {
            if let Ok(mut inner) = state.model_installer.inner.lock() { inner.0.received = received; }
        };
        if exists {
            crate::download::verify_file(&model, &model_catalog::BONSAI, receiver.clone(), progress).await?;
        } else {
            crate::download::fetch(&crate::download::client()?, &model_catalog::BONSAI, &model, receiver.clone(), progress).await?;
        }
        if *receiver.borrow() { return Err("Download cancelled.".into()); }
        Ok::<(PathBuf, PathBuf), String>((runtime, model))
    }.await;
    let (runtime, model) = match result {
        Ok(paths) => paths,
        Err(error) => {
            for installer in [&state.model_installer, &state.runtime_installer] {
                installer
                    .inner
                    .lock()
                    .map_err(|_| "Installer unavailable.")?
                    .0
                    .fail(error.clone());
            }
            return Err(error);
        }
    };
    {
        let mut inner = state
            .model_installer
            .inner
            .lock()
            .map_err(|_| "Installer unavailable.")?;
        inner.0.path = Some(model.to_string_lossy().into_owned());
        inner.0.phase = "waiting for active generation".into();
    }
    let mut config = starting_config(cpu);
    config.mtp_mode = mtp_mode.unwrap_or_default();
    // Preflight before persisting the candidate or stopping a working process.
    if let Err(error) = crate::runtime::preflight_bonsai(&runtime, &model, &config).await {
        for installer in [&state.model_installer, &state.runtime_installer] {
            installer
                .inner
                .lock()
                .map_err(|_| "Installer unavailable.")?
                .0
                .fail(error.clone());
        }
        return Err(error);
    }
    // Downloads do not block the current conversation. Queue only the activation
    // behind generation/loading, and allow cancellation while waiting.
    let mut waiting_cancel = receiver.clone();
    let _operation = tokio::select! {
        biased;
        _ = waiting_cancel.changed() => {
            for installer in [&state.model_installer, &state.runtime_installer] {
                installer.inner.lock().map_err(|_| "Installer unavailable.")?.0.fail("Download cancelled.".into());
            }
            return Err("Download cancelled.".into());
        },
        guard = state.operation.lock() => guard,
    };
    if *receiver.borrow() {
        return Err("Download cancelled.".into());
    }
    for installer in [&state.model_installer, &state.runtime_installer] {
        let mut inner = installer
            .inner
            .lock()
            .map_err(|_| "Installer unavailable.")?;
        inner.0.busy = true;
        inner.0.phase = "loading".into();
    }
    let (previous_preferences, previous_config) = {
        let store = state.database()?;
        (store.preferences()?, store.runtime_config()?)
    };
    crate::model_library::save_active_profile(&state, &previous_config)?;
    let mut preferences = previous_preferences.clone();
    preferences.runtime_path = runtime.to_string_lossy().into_owned();
    preferences.model_path = model.to_string_lossy().into_owned();
    preferences.projector_path.clear();
    preferences.temperature = 1.0;
    preferences.top_p = 0.95;
    preferences.max_tokens = 8192;
    {
        let store = state.database()?;
        store.save_preferences(&preferences)?;
        if let Err(error) = store.save_runtime_config(&config) {
            store.save_preferences(&previous_preferences)?;
            return Err(error);
        }
    }
    match crate::commands::load_selected_model_with_install_cancel(&state, Some(receiver.clone()))
        .await
    {
        Ok(status) => {
            for installer in [&state.model_installer, &state.runtime_installer] {
                let mut inner = installer
                    .inner
                    .lock()
                    .map_err(|_| "Installer unavailable.")?;
                inner.0.busy = false;
                inner.0.phase = "ready".into();
            }
            Ok(status)
        }
        Err(error) => {
            // Keep verified downloads and restore the prior selection. The old
            // runtime bundle is never overwritten or removed.
            for installer in [&state.model_installer, &state.runtime_installer] {
                installer
                    .inner
                    .lock()
                    .map_err(|_| "Installer unavailable.")?
                    .0
                    .fail(error.clone());
            }
            let store = state.database()?;
            store.save_preferences(&previous_preferences)?;
            store.save_runtime_config(&previous_config)?;
            Err(format!("Bonsai was installed but could not start: {error} Previous selection restored. Use Install and run without MTP to retry, or inspect Diagnostics for the runtime error."))
        }
    }
}

#[tauri::command]
pub fn cancel_bonsai_install(state: tauri::State<'_, crate::AppState>) -> Result<(), String> {
    cancel_install(&state)
}
fn cancel_install(state: &crate::AppState) -> Result<(), String> {
    let inner = state
        .model_installer
        .inner
        .lock()
        .map_err(|_| "Installer unavailable.")?;
    if inner.0.profile_id.as_deref() == Some("bonsai-2-9b") && inner.0.busy {
        if let Some(sender) = &inner.1 {
            let _ = sender.send(true);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn old_saved_configs_default_to_auto_and_explicit_off_round_trips() {
        let mut saved = serde_json::to_value(RuntimeConfig::default()).unwrap();
        saved.as_object_mut().unwrap().remove("mtpMode");
        let config: RuntimeConfig = serde_json::from_value(saved.clone()).unwrap();
        assert_eq!(config.mtp_mode, crate::runtime_config::MtpMode::Auto);
        saved["mtpMode"] = "off".into();
        let config: RuntimeConfig = serde_json::from_value(saved).unwrap();
        assert_eq!(config.mtp_mode, crate::runtime_config::MtpMode::Off);
        assert_eq!(serde_json::to_value(config).unwrap()["mtpMode"], "off");
    }

    #[test]
    fn cancellation_is_owned_and_does_not_cancel_an_unrelated_model_load() {
        let temp = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(temp.path());
        let (sender, receiver) = tokio::sync::watch::channel(false);
        {
            let mut inner = state.model_installer.inner.lock().unwrap();
            inner.0.busy = true;
            inner.1 = Some(sender);
        }
        cancel_install(&state).unwrap();
        assert!(!*receiver.borrow());
        {
            let mut inner = state.model_installer.inner.lock().unwrap();
            inner.0.profile_id = Some("bonsai-2-9b".into());
            inner.0.phase = "loading".into();
        }
        cancel_install(&state).unwrap();
        assert!(*receiver.borrow());
        assert!(!*state.load_cancel.borrow());
    }
    #[tokio::test]
    async fn already_cancelled_activation_keeps_selection_and_runtime_untouched() {
        let temp = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(temp.path());
        let before = state.database().unwrap().preferences().unwrap();
        let (_sender, receiver) = tokio::sync::watch::channel(true);
        let error =
            crate::commands::load_selected_model_with_install_cancel(&state, Some(receiver))
                .await
                .err()
                .unwrap();
        assert_eq!(error, "Model loading cancelled.");
        assert_eq!(
            state.database().unwrap().preferences().unwrap().model_path,
            before.model_path
        );
        assert_eq!(state.runtime.lock().await.inspect().phase, "stopped");
        assert!(state.load_status.lock().unwrap().is_none());
    }

    #[test]
    fn integrated_head_is_a_model_and_external_heads_are_not_auto_enabled() {
        assert!(is_integrated_model(Path::new(
            model_catalog::BONSAI_FILENAME
        )));
        assert!(!is_integrated_model(Path::new("mtp-head.gguf")));
        assert!(!is_integrated_model(Path::new(
            "Ternary-Bonsai-2-9B-PQ2_0.gguf"
        )));
        assert!(requires_prism(Path::new("Ternary-Bonsai-2-9B-PQ2_0.gguf")));
    }
    #[test]
    fn local_profile_resets_prose_penalties_and_respects_thinking_control() {
        let mut payload = serde_json::json!({"temperature":0.7,"top_p":0.8,"repeat_penalty":1.1,"chat_template_kwargs":{"enable_thinking":false}});
        apply_sampling(&mut payload);
        assert_eq!(payload["presence_penalty"], 1.5);
        assert_eq!(payload["repeat_penalty"], 1.0);
        assert_eq!(payload["repeat_last_n"], 64);
        assert_eq!(payload["min_p"], 0.0);
        assert_eq!(payload["temperature"], 0.7);
        payload["chat_template_kwargs"]["enable_thinking"] = true.into();
        apply_sampling(&mut payload);
        assert_eq!(payload["presence_penalty"], 1.0);
    }
    #[test]
    fn conservative_first_load_keeps_response_and_prompt_budgets() {
        let cpu = starting_config(true);
        assert_eq!(cpu.gpu_layers, 0);
        assert!(!cpu.offload_kv_cache);
        assert_eq!(cpu.cache_type_v, CacheType::F16);
        assert!(cpu.context_length > 8192);
        assert_eq!(starting_config(false).gpu_layers, -1);
        cpu.validate().unwrap();
    }
}
