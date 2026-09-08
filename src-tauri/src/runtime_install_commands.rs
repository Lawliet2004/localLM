use crate::model_install::{RunGuard, Status};
use serde::Serialize;
use tauri::Manager;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    bytes: u64,
    required_bytes: u64,
    available_bytes: u64,
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
    Ok(Info {
        bytes: assets.iter().map(|asset| asset.bytes).sum(),
        required_bytes: crate::runtime_install::required_bytes(&assets)?,
        available_bytes: crate::download::available_space(&root)?,
        destination: root.to_string_lossy().into_owned(),
    })
}

#[tauri::command]
pub fn runtime_install_status(state: tauri::State<'_, crate::AppState>) -> Result<Status, String> {
    Ok(state
        .runtime_installer
        .inner
        .lock()
        .map_err(|_| "Installer unavailable.")?
        .0
        .clone())
}
#[tauri::command]
pub fn cancel_runtime_install(state: tauri::State<'_, crate::AppState>) -> Result<(), String> {
    let inner = state
        .runtime_installer
        .inner
        .lock()
        .map_err(|_| "Installer unavailable.")?;
    if let Some(sender) = &inner.1 {
        let _ = sender.send(true);
    }
    Ok(())
}
#[tauri::command]
pub async fn install_runtime(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
) -> Result<Status, String> {
    let _installation = state
        .installation_operation
        .try_lock()
        .map_err(|_| "Another model or runtime installation is running.")?;
    let installer = &state.runtime_installer;
    let receiver = {
        let mut inner = installer
            .inner
            .lock()
            .map_err(|_| "Installer unavailable.")?;
        let (sender, receiver) = tokio::sync::watch::channel(false);
        inner.0 = Status {
            busy: true,
            phase: "preparing".into(),
            ..Default::default()
        };
        inner.1 = Some(sender);
        receiver
    };
    let _guard = RunGuard(installer);
    let result = async {
        let root = app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join("runtimes");
        crate::runtime_install::install(&root, receiver, |phase, received, total| {
            if let Ok(mut inner) = installer.inner.lock() {
                inner.0.phase = phase.into();
                inner.0.received = received;
                inner.0.total = total;
            }
        })
        .await
    }
    .await;
    let mut inner = installer
        .inner
        .lock()
        .map_err(|_| "Installer unavailable.")?;
    inner.0.busy = false;
    match result {
        Ok(path) => {
            inner.0.phase = "ready".into();
            inner.0.path = Some(path.to_string_lossy().into_owned());
        }
        Err(error) => {
            inner.0.phase = "failed".into();
            inner.0.error = Some(error);
        }
    }
    Ok(inner.0.clone())
}
