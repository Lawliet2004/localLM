use crate::model_install::{RunGuard, Status};
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    bytes: u64,
    required_bytes: u64,
    available_bytes: u64,
    destination: String,
    supported: bool,
    platform: &'static str,
    message: Option<String>,
}

#[tauri::command]
pub async fn runtime_download_info(
    state: tauri::State<'_, crate::AppState>,
    runtime_id: Option<String>,
) -> Result<Info, String> {
    let root = crate::managed_storage::root(&state.data_dir)?.join("runtimes");
    tokio::fs::create_dir_all(&root)
        .await
        .map_err(|error| error.to_string())?;
    let package = crate::runtime_catalog::package(runtime_id.as_deref())?;
    let supported = package.supported();
    let assets = if supported {
        package.assets
    } else {
        Vec::new()
    };
    Ok(Info {
        bytes: assets.iter().map(|asset| asset.bytes).sum(),
        required_bytes: if supported {
            crate::runtime_install::required_bytes(&assets)?
        } else {
            0
        },
        available_bytes: crate::download::available_space(&root)?,
        destination: root.to_string_lossy().into_owned(),
        supported,
        platform: std::env::consts::OS,
        message: (!supported).then(|| crate::runtime_install::MANAGED_RUNTIME_MESSAGE.to_string()),
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
    state: tauri::State<'_, crate::AppState>,
    runtime_id: Option<String>,
) -> Result<Status, String> {
    let package = crate::runtime_catalog::package(runtime_id.as_deref())?;
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
        let root = crate::managed_storage::root(&state.data_dir)?.join("runtimes");
        crate::runtime_install::install_package(
            &root,
            package,
            receiver,
            |phase, received, total| {
                if let Ok(mut inner) = installer.inner.lock() {
                    inner.0.phase = phase.into();
                    inner.0.received = received;
                    inner.0.total = total;
                }
            },
        )
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
            inner.0.fail(error);
        }
    }
    Ok(inner.0.clone())
}
