//! Opt-in catalog and storage, separate from the standard MiniCPM installation.
use crate::{download, runtime_archive};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const ENABLED_KEY: &str = "experimental_bonsai_enabled";
const ROOT: &str = "experimental/bonsai-27b";
pub(crate) const FILENAME: &str = "Bonsai-27B-Q1_0.gguf";
const RUNTIME_PREFIX: &str = "prism-b10754-2459f68-cuda12.4";
const MODEL: download::Asset<'static> = download::Asset {
    url: "https://huggingface.co/prism-ml/Bonsai-27B-gguf/resolve/f10afb355f104535e3e3e98cf7ab7795c72bd292/Bonsai-27B-Q1_0.gguf",
    bytes: 3_803_452_480,
    sha256: "17ef842e47450caeb8eaa3ebfbbab5d2f2278b62b79be107985fb69a2f819aa0",
};

fn enabled(store: &crate::store::Store) -> Result<bool, String> {
    store.setting(ENABLED_KEY)
}

fn catalog(opted_in: bool) -> Result<Option<Value>, String> {
    if !opted_in {
        return Ok(None);
    }
    serde_json::from_str(include_str!("../../../catalog/bonsai-27b.json"))
        .map(Some)
        .map_err(|e| format!("Invalid experimental Bonsai catalog: {e}"))
}

fn model_path(base: &Path) -> PathBuf {
    base.join(ROOT).join("models").join(FILENAME)
}

fn runtime_assets() -> Result<Vec<runtime_archive::RuntimeAsset>, String> {
    serde_json::from_str(include_str!("../../../catalog/bonsai-runtime-assets.json"))
        .map_err(|e| format!("Invalid experimental runtime catalog: {e}"))
}

fn find_runtime(root: &Path) -> Result<Option<PathBuf>, String> {
    if !root.exists() {
        return Ok(None);
    }
    let assets = runtime_assets()?;
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(suffix) = name.strip_prefix(&format!("{RUNTIME_PREFIX}-")) else {
            continue;
        };
        if uuid::Uuid::parse_str(suffix).is_err()
            || !entry.file_type().map_err(|e| e.to_string())?.is_dir()
        {
            continue;
        }
        if assets.iter().flat_map(|a| &a.files).all(|file| {
            std::fs::symlink_metadata(entry.path().join(&file.name))
                .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink() && m.len() == file.bytes)
        }) {
            candidates.push(entry.path().join("llama-server.exe"));
        }
    }
    candidates.sort();
    Ok(candidates.into_iter().next())
}

#[derive(Default)]
pub(crate) struct Installer {
    model: crate::model_install::Installer,
    runtime: crate::model_install::Installer,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AssetKind {
    Model,
    Runtime,
}
impl Installer {
    fn for_asset(&self, asset: AssetKind) -> &crate::model_install::Installer {
        match asset {
            AssetKind::Model => &self.model,
            AssetKind::Runtime => &self.runtime,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Previous {
    preferences: crate::store::Preferences,
    config: crate::runtime_config::RuntimeConfig,
    was_ready: bool,
}
const PREVIOUS_KEY: &str = "experimental_bonsai_previous";

fn development() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".local")
}
fn selected_model(state: &crate::AppState) -> PathBuf {
    let managed = model_path(&state.data_dir);
    let developer = model_path(&development());
    if !managed.exists() && developer.is_file() {
        developer
    } else {
        managed
    }
}
fn selected_runtime(state: &crate::AppState) -> Result<Option<PathBuf>, String> {
    if let Some(path) = find_runtime(&state.data_dir.join(ROOT).join("runtimes"))? {
        return Ok(Some(path));
    }
    find_runtime(&development().join(ROOT))
}
fn same_file(left: &Path, right: &Path) -> bool {
    left == right
        || std::fs::canonicalize(left)
            .ok()
            .zip(std::fs::canonicalize(right).ok())
            .is_some_and(|(a, b)| a == b)
}
fn owns_model(state: &crate::AppState, path: &str) -> bool {
    !path.is_empty()
        && [model_path(&state.data_dir), model_path(&development())]
            .iter()
            .any(|p| same_file(Path::new(path), p))
}
fn require_enabled(state: &crate::AppState) -> Result<(), String> {
    let store = state.database()?;
    if enabled(&store)? {
        Ok(())
    } else {
        Err("Enable Experimental models in Models & runtime before using Bonsai.".into())
    }
}
fn preset() -> Result<Option<crate::runtime_config::RuntimeConfig>, String> {
    let entry = catalog(true)?.unwrap();
    if entry["preset"].is_null() {
        return Ok(None);
    }
    let value: crate::runtime_config::RuntimeConfig =
        serde_json::from_value(entry["preset"].clone())
            .map_err(|e| format!("Invalid Bonsai preset: {e}"))?;
    value.validate()?;
    Ok(Some(value))
}

pub(crate) fn configure_runtime(
    state: &crate::AppState,
    preferences: &mut crate::store::Preferences,
) -> Result<(), String> {
    if !owns_model(state, &preferences.model_path) {
        return Ok(());
    }
    require_enabled(state)?;
    preferences.runtime_path = selected_runtime(state)?.ok_or("Bonsai's pinned PrismML runtime is missing or incomplete. Install it in Experimental models, or choose MiniCPM.")?.to_string_lossy().into_owned();
    preferences.projector_path.clear();
    Ok(())
}

/// Bound the fork's host prompt cache; stock model/runtime arguments are unchanged.
pub(crate) fn launch_arguments(model: &Path, executable: &Path) -> Option<[&'static str; 2]> {
    let owned = model.file_name().is_some_and(|n| n == FILENAME)
        && model
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .is_some_and(|n| n == "bonsai-27b")
        && model
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .is_some_and(|n| n == "experimental");
    let pinned = executable
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|n| n.to_string_lossy().starts_with(RUNTIME_PREFIX));
    (owned && pinned).then_some(["--cache-ram", "512"])
}

#[tauri::command]
pub fn bonsai_enabled(state: tauri::State<'_, crate::AppState>) -> Result<bool, String> {
    let store = state.database()?;
    enabled(&store)
}

async fn restore(state: &crate::AppState, previous: &Previous, reload: bool) -> Result<(), String> {
    state.runtime.lock().await.stop().await?;
    {
        let store = state.database()?;
        store.save_model_runtime(&previous.preferences, &previous.config)?;
    }
    if reload && previous.was_ready {
        crate::commands::load_selected_model(state).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn set_bonsai_enabled(
    state: tauri::State<'_, crate::AppState>,
    enabled: bool,
) -> Result<bool, String> {
    set_enabled_inner(&state, enabled).await
}
async fn set_enabled_inner(state: &crate::AppState, enabled: bool) -> Result<bool, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Stop generation or loading before changing experimental models.")?;
    let _installation = state
        .installation_operation
        .try_lock()
        .map_err(|_| "Finish or cancel the installation before disabling experimental models.")?;
    if !enabled {
        let preferences = state.database()?.preferences()?;
        if owns_model(state, &preferences.model_path) {
            let previous: Option<Previous> = state.database()?.setting(PREVIOUS_KEY)?;
            let previous = previous.ok_or("Choose another model before disabling experimental models; no previous selection was saved.")?;
            let config = state.database()?.runtime_config()?;
            crate::model_library::save_active_profile(state, &config)?;
            restore(state, &previous, false).await?;
        }
    }
    state.database()?.save_setting(ENABLED_KEY, &enabled)?;
    Ok(enabled)
}

#[tauri::command]
pub async fn bonsai_info(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Option<Value>, String> {
    info_inner(&state).await
}
async fn info_inner(state: &crate::AppState) -> Result<Option<Value>, String> {
    let opted_in = {
        let store = state.database()?;
        enabled(&store)?
    };
    let Some(entry) = catalog(opted_in)? else {
        return Ok(None);
    };
    let root = state.data_dir.join(ROOT);
    tokio::fs::create_dir_all(root.join("models"))
        .await
        .map_err(|e| e.to_string())?;
    tokio::fs::create_dir_all(root.join("runtimes"))
        .await
        .map_err(|e| e.to_string())?;
    let model = selected_model(state);
    let assets = runtime_assets()?;
    Ok(Some(json!({
        "catalog": entry,
        "model": {"filename": FILENAME, "bytes": MODEL.bytes, "sha256": MODEL.sha256,
            "destination": model, "destinationExists": model.is_file(),
            "availableBytes": download::available_space(model.parent().unwrap())?, "requiredBytes": download::required_space(MODEL.bytes)?},
        "runtime": {"bytes": assets.iter().map(|a| a.bytes).sum::<u64>(),
            "requiredBytes": crate::runtime_install::required_bytes(&assets)?, "availableBytes": download::available_space(&root)?, "destination": root.join("runtimes")},
        "runtimePath": selected_runtime(state)?, "preset": preset()?
    })))
}

#[tauri::command]
pub fn bonsai_install_status(
    state: tauri::State<'_, crate::AppState>,
    asset: AssetKind,
) -> Result<crate::model_install::Status, String> {
    require_enabled(&state)?;
    Ok(state
        .bonsai_installer
        .for_asset(asset)
        .inner
        .lock()
        .map_err(|_| "Experimental installer unavailable.")?
        .0
        .clone())
}
#[tauri::command]
pub fn cancel_bonsai_install(
    state: tauri::State<'_, crate::AppState>,
    asset: AssetKind,
) -> Result<(), String> {
    require_enabled(&state)?;
    let inner = state
        .bonsai_installer
        .for_asset(asset)
        .inner
        .lock()
        .map_err(|_| "Experimental installer unavailable.")?;
    if let Some(sender) = &inner.1 {
        let _ = sender.send(true);
    }
    Ok(())
}
#[tauri::command]
pub async fn install_bonsai(
    state: tauri::State<'_, crate::AppState>,
    asset: AssetKind,
) -> Result<crate::model_install::Status, String> {
    require_enabled(&state)?;
    let _installation = state
        .installation_operation
        .try_lock()
        .map_err(|_| "Another model or runtime installation is running.")?;
    let installer = state.bonsai_installer.for_asset(asset);
    let (sender, cancel) = tokio::sync::watch::channel(false);
    {
        let mut inner = installer
            .inner
            .lock()
            .map_err(|_| "Experimental installer unavailable.")?;
        inner.0 = crate::model_install::Status {
            busy: true,
            phase: "preparing".into(),
            ..Default::default()
        };
        inner.1 = Some(sender);
    }
    let _guard = crate::model_install::RunGuard(installer);
    let progress = |phase: &str, received, total| {
        if let Ok(mut inner) = installer.inner.lock() {
            inner.0.phase = phase.into();
            inner.0.received = received;
            inner.0.total = total;
        }
    };
    let result = async {
        match asset {
            AssetKind::Runtime => {
                crate::runtime_install::install_bundle(
                    &state.data_dir.join(ROOT).join("runtimes"),
                    runtime_assets()?,
                    RUNTIME_PREFIX,
                    cancel,
                    progress,
                )
                .await
            }
            AssetKind::Model => {
                let path = selected_model(&state);
                tokio::fs::create_dir_all(path.parent().unwrap())
                    .await
                    .map_err(|e| e.to_string())?;
                if path.is_file() {
                    download::verify_file(&path, &MODEL, cancel, |n| {
                        progress("verifying", n, MODEL.bytes)
                    })
                    .await?;
                } else {
                    download::fetch(&download::client()?, &MODEL, &path, cancel, |n| {
                        progress("downloading", n, MODEL.bytes)
                    })
                    .await?;
                }
                Ok(path)
            }
        }
    }
    .await;
    let mut inner = installer
        .inner
        .lock()
        .map_err(|_| "Experimental installer unavailable.")?;
    inner.0.busy = false;
    match result {
        Ok(path) => {
            inner.0.phase = "ready".into();
            inner.0.path = Some(path.to_string_lossy().into_owned());
        }
        Err(error) => inner.0.fail(error),
    }
    Ok(inner.0.clone())
}

#[tauri::command]
pub async fn activate_bonsai(
    state: tauri::State<'_, crate::AppState>,
    use_preset: Option<bool>,
) -> Result<crate::runtime::RuntimeStatus, String> {
    activate_inner(&state, use_preset.unwrap_or(false)).await
}
async fn activate_inner(
    state: &crate::AppState,
    use_preset: bool,
) -> Result<crate::runtime::RuntimeStatus, String> {
    require_enabled(state)?;
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Stop generation or loading before switching models.")?;
    let _installation = state
        .installation_operation
        .try_lock()
        .map_err(|_| "Finish the installation before loading Bonsai.")?;
    let model = selected_model(state);
    let runtime = selected_runtime(state)?
        .ok_or("Bonsai's PrismML runtime is missing or incomplete. Install it before loading.")?;
    let config = preset()?.ok_or(
        "Bonsai performance acceptance is not complete; no validated preset is available.",
    )?;
    let (preferences, current_config) = {
        let store = state.database()?;
        (store.preferences()?, store.runtime_config()?)
    };
    let selected = owns_model(state, &preferences.model_path);
    let config = choose_config(
        state,
        &preferences,
        &current_config,
        &model,
        config,
        use_preset,
    )?;
    config.validate()?;
    let (_sender, cancel) = tokio::sync::watch::channel(false);
    download::verify_file(&model, &MODEL, cancel, |_| {}).await?;
    let previous = Previous {
        preferences: preferences.clone(),
        config: current_config.clone(),
        was_ready: state.runtime.lock().await.inspect().phase == "ready",
    };
    crate::model_library::save_active_profile(state, &current_config)?;
    if !selected {
        state
            .database()?
            .save_setting(PREVIOUS_KEY, &Some(previous.clone()))?;
    }
    let mut next = preferences;
    next.model_path = model.to_string_lossy().into_owned();
    next.runtime_path = runtime.to_string_lossy().into_owned();
    next.projector_path.clear();
    if !selected {
        next.temperature = 0.7;
        next.top_p = 0.95;
        next.max_tokens = 512;
    }
    state.database()?.save_model_runtime(&next, &config)?;
    match crate::commands::load_selected_model(state).await {
        Ok(status) => Ok(status),
        Err(error) => {
            state.runtime.lock().await.stop().await?;
            let error = preserve_load_error(state, &error);
            let fallback: Option<Previous> = state.database()?.setting(PREVIOUS_KEY)?;
            if let Err(recovery) =
                restore(state, fallback.as_ref().unwrap_or(&previous), true).await
            {
                return Err(format!(
                    "Bonsai failed: {error}. Previous settings restored; reload failed: {recovery}"
                ));
            }
            Err(format!(
                "Bonsai failed: {error}. Previous model settings restored."
            ))
        }
    }
}

fn preserve_load_error(state: &crate::AppState, error: &str) -> String {
    let log = std::fs::read(state.data_dir.join("runtime.log")).unwrap_or_default();
    let destination = state.data_dir.join(ROOT).join("last-load-failure.log");
    let saved = std::fs::create_dir_all(destination.parent().unwrap())
        .and_then(|()| std::fs::write(&destination, &log))
        .is_ok();
    let lower = String::from_utf8_lossy(&log).to_lowercase();
    let details = if saved {
        format!(" See {}.", destination.display())
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

fn choose_config(
    state: &crate::AppState,
    preferences: &crate::store::Preferences,
    current: &crate::runtime_config::RuntimeConfig,
    model: &Path,
    measured: crate::runtime_config::RuntimeConfig,
    use_preset: bool,
) -> Result<crate::runtime_config::RuntimeConfig, String> {
    if use_preset {
        return Ok(measured);
    }
    if owns_model(state, &preferences.model_path) {
        return Ok(current.clone());
    }
    Ok(
        crate::model_library::saved_profile_config(state, &model.to_string_lossy())?
            .unwrap_or(measured),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn off_info_does_not_create_install_directories_or_change_standard_runtime() {
        let directory = tempfile::tempdir().unwrap();
        let state = crate::web_search::tests::test_state(directory.path());
        assert!(info_inner(&state).await.unwrap().is_none());
        assert!(!directory.path().join(ROOT).exists());
        assert!(activate_inner(&state, false)
            .await
            .err()
            .unwrap()
            .contains("Enable Experimental"));
        let mut preferences = crate::store::Preferences {
            model_path: directory
                .path()
                .join("models/MiniCPM5-2B.Q6_K.gguf")
                .to_string_lossy()
                .into_owned(),
            runtime_path: "original/llama-server.exe".into(),
            ..Default::default()
        };
        configure_runtime(&state, &mut preferences).unwrap();
        assert_eq!(preferences.runtime_path, "original/llama-server.exe");
        assert!(launch_arguments(
            Path::new(&preferences.model_path),
            Path::new(&preferences.runtime_path)
        )
        .is_none());
    }

    #[test]
    fn measured_preset_is_optional_and_user_overrides_survive_selection() {
        let directory = tempfile::tempdir().unwrap();
        let state = crate::web_search::tests::test_state(directory.path());
        let model = model_path(directory.path());
        let preferences = crate::store::Preferences {
            model_path: model.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let current = crate::runtime_config::RuntimeConfig {
            context_length: 8192,
            gpu_layers: 32,
            ..Default::default()
        };
        let measured = crate::runtime_config::RuntimeConfig {
            context_length: 4096,
            gpu_layers: 65,
            ..Default::default()
        };
        assert_eq!(
            choose_config(
                &state,
                &preferences,
                &current,
                &model,
                measured.clone(),
                false
            )
            .unwrap(),
            current
        );
        assert_eq!(
            choose_config(
                &state,
                &preferences,
                &current,
                &model,
                measured.clone(),
                true
            )
            .unwrap(),
            measured
        );
        state
            .database()
            .unwrap()
            .save_preferences(&preferences)
            .unwrap();
        crate::model_library::save_active_profile(&state, &current).unwrap();
        let other = crate::store::Preferences::default();
        assert_eq!(
            choose_config(&state, &other, &measured, &model, measured.clone(), false).unwrap(),
            current
        );
        if let Some(preset) = preset().unwrap() {
            preset.validate().unwrap();
        }
    }

    #[tokio::test]
    async fn disabling_restores_previous_model_and_runtime_configuration() {
        let directory = tempfile::tempdir().unwrap();
        let state = crate::web_search::tests::test_state(directory.path());
        let previous = Previous {
            preferences: crate::store::Preferences::default(),
            config: crate::runtime_config::RuntimeConfig::default(),
            was_ready: false,
        };
        let experimental = crate::store::Preferences {
            model_path: model_path(directory.path()).to_string_lossy().into_owned(),
            runtime_path: "PrismML/llama-server.exe".into(),
            ..previous.preferences.clone()
        };
        {
            let store = state.database().unwrap();
            store.save_setting(ENABLED_KEY, &true).unwrap();
            store
                .save_setting(PREVIOUS_KEY, &Some(previous.clone()))
                .unwrap();
            store
                .save_model_runtime(
                    &experimental,
                    &crate::runtime_config::RuntimeConfig {
                        context_length: 4096,
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        assert!(!set_enabled_inner(&state, false).await.unwrap());
        let store = state.database().unwrap();
        assert_eq!(
            store.preferences().unwrap().model_path,
            previous.preferences.model_path
        );
        assert_eq!(
            store.preferences().unwrap().runtime_path,
            previous.preferences.runtime_path
        );
        assert_eq!(store.runtime_config().unwrap(), previous.config);
        assert!(!enabled(&store).unwrap());
    }

    #[test]
    fn off_keeps_existing_settings_and_catalog_unchanged() {
        let store = crate::store::Store::open_memory().unwrap();
        let previous = store.preferences().unwrap();
        assert!(!enabled(&store).unwrap());
        assert!(catalog(false).unwrap().is_none());
        assert_eq!(
            crate::model_catalog::model(None).unwrap().0,
            "MiniCPM5-2B.Q6_K.gguf"
        );
        assert_eq!(store.preferences().unwrap().model_path, previous.model_path);
        store.save_setting(ENABLED_KEY, &true).unwrap();
        assert!(enabled(&store).unwrap());
        assert!(catalog(true).unwrap().unwrap()["experimental"]
            .as_bool()
            .unwrap());
    }

    #[test]
    fn catalog_matches_verified_asset_and_keeps_paths_separate() {
        let entry = catalog(true).unwrap().unwrap();
        assert_eq!(entry["filename"], FILENAME);
        assert_eq!(entry["bytes"], MODEL.bytes);
        assert_eq!(entry["sha256"], MODEL.sha256);
        assert_eq!(entry["url"], MODEL.url);
        assert_eq!(entry["license"], "Apache-2.0");
        let root = tempfile::tempdir().unwrap();
        let experimental = model_path(root.path());
        assert!(experimental.starts_with(root.path().join("experimental/bonsai-27b")));
        assert_ne!(experimental.parent().unwrap(), root.path().join("models"));
        assert!(runtime_assets()
            .unwrap()
            .iter()
            .all(|a| a.url.contains("prism-b10754-2459f68")));
    }

    #[test]
    fn incomplete_runtime_is_not_selected() {
        let root = tempfile::tempdir().unwrap();
        let bundle = root
            .path()
            .join(format!("{RUNTIME_PREFIX}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&bundle).unwrap();
        std::fs::write(bundle.join("llama-server.exe"), b"incomplete").unwrap();
        assert!(find_runtime(root.path()).unwrap().is_none());
    }

    #[tokio::test]
    async fn sha_failure_does_not_make_model_usable_or_change_preferences() {
        let store = crate::store::Store::open_memory().unwrap();
        let before = store.preferences().unwrap();
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join(FILENAME);
        std::fs::write(&file, b"x").unwrap();
        let (_sender, cancel) = tokio::sync::watch::channel(false);
        let error = crate::download::verify_file(
            &file,
            &crate::download::Asset { bytes: 1, ..MODEL },
            cancel,
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(error.contains("SHA-256"));
        let mut status = crate::model_install::Status::default();
        status.fail(error);
        assert_eq!(status.phase, "failed");
        assert!(status.path.is_none());
        assert_eq!(store.preferences().unwrap().model_path, before.model_path);
    }
}

