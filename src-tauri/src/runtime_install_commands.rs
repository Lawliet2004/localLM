use crate::model_install::Status;
use serde::Serialize;
use tauri::Manager;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    bytes: u64,
    required_bytes: u64,
    available_bytes: u64,
    saved_bytes: u64,
    reusable: bool,
    destination: String,
}

#[tauri::command]
pub async fn runtime_download_info(app: tauri::AppHandle) -> Result<Info, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("runtimes");
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|error| error.to_string())?;
    let assets = crate::runtime_archive::catalog()?;
    let (saved, remaining) = crate::runtime_install::cached_bytes(&root, &assets);
    let expanded: u64 = assets
        .iter()
        .flat_map(|asset| &asset.files)
        .map(|file| file.bytes)
        .sum();
    let storage = crate::download::storage(&root, remaining, expanded)?;
    let reusable = crate::runtime_inventory::find_complete(
        &root,
        crate::runtime_install::STANDARD_PREFIX,
        &assets,
    )?
    .is_some();
    Ok(Info {
        bytes: assets.iter().map(|asset| asset.bytes).sum(),
        required_bytes: storage.required,
        available_bytes: storage.available,
        saved_bytes: saved,
        reusable,
        destination: root.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub fn runtime_install_status(state: tauri::State<'_, crate::AppState>) -> Result<Status, String> {
    Ok(state.runtime_installer.status())
}
#[tauri::command]
pub fn cancel_runtime_install(
    state: tauri::State<'_, crate::AppState>,
    job_id: Option<u64>,
) -> Result<(), String> {
    state.runtime_installer.cancel(job_id)
}
#[tauri::command]
pub async fn install_runtime(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
) -> Result<Status, String> {
    let installer = &state.runtime_installer;
    let (_guard, receiver, _job_id) =
        installer.begin("runtime:standard", "llama.cpp CUDA runtime")?;
    let result = async {
        let root = app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join("runtimes");
        crate::runtime_install::install(&root, installer, receiver).await
    }
    .await;
    Ok(installer.finish(result))
}
