use serde::Serialize;
use std::sync::Mutex;
use tauri::Manager;
use tokio::sync::watch;

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub busy: bool,
    pub phase: String,
    pub received: u64,
    pub total: u64,
    pub path: Option<String>,
    pub error: Option<String>,
}
impl Status {
    pub(crate) fn fail(&mut self, error: String) {
        self.busy = false;
        self.phase = if matches!(
            error.as_str(),
            "Download cancelled." | "Runtime installation cancelled."
        ) {
            "cancelled"
        } else {
            "failed"
        }
        .into();
        self.error = Some(error);
    }
}
#[derive(Default)]
pub struct Installer {
    pub(crate) inner: Mutex<(Status, Option<watch::Sender<bool>>)>,
}
pub(crate) struct RunGuard<'a>(pub(crate) &'a Installer);
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_is_distinct_from_installation_failure() {
        let mut status = Status::default();
        for error in ["Download cancelled.", "Runtime installation cancelled."] {
            status.fail(error.into());
            assert_eq!(status.phase, "cancelled");
            assert!(!status.busy);
        }
        status.fail("Download SHA-256 verification failed.".into());
        assert_eq!(status.phase, "failed");
    }
    #[test]
    fn interrupted_run_releases_busy_state_but_preserves_finished_result() {
        let installer = Installer::default();
        installer.inner.lock().unwrap().0.busy = true;
        drop(RunGuard(&installer));
        let status = installer.inner.lock().unwrap().0.clone();
        assert!(!status.busy);
        assert_eq!(status.phase, "interrupted");
        assert!(status.error.is_some());
        installer.inner.lock().unwrap().0 = Status {
            phase: "ready".into(),
            path: Some("verified-model".into()),
            ..Default::default()
        };
        drop(RunGuard(&installer));
        assert_eq!(installer.inner.lock().unwrap().0.phase, "ready");
    }
}
impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.0.inner.lock() {
            inner.1 = None;
            if inner.0.busy {
                inner.0.busy = false;
                inner.0.phase = "interrupted".into();
                inner.0.error = Some(
                    "Installation was interrupted. Retry to verify or download the files.".into(),
                );
            }
        }
    }
}
#[tauri::command]
pub fn model_install_status(state: tauri::State<'_, crate::AppState>) -> Result<Status, String> {
    Ok(state
        .model_installer
        .inner
        .lock()
        .map_err(|_| "Installer unavailable.")?
        .0
        .clone())
}
#[tauri::command]
pub fn cancel_model_install(state: tauri::State<'_, crate::AppState>) -> Result<(), String> {
    let inner = state
        .model_installer
        .inner
        .lock()
        .map_err(|_| "Installer unavailable.")?;
    if let Some(sender) = &inner.1 {
        let _ = sender.send(true);
    }
    Ok(())
}
#[tauri::command]
pub async fn install_model(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
) -> Result<Status, String> {
    let _installation = state
        .installation_operation
        .try_lock()
        .map_err(|_| "Another model or runtime installation is running.")?;
    let installer = &state.model_installer;
    let receiver = {
        let mut inner = installer
            .inner
            .lock()
            .map_err(|_| "Installer unavailable.")?;
        if inner.0.busy {
            return Err("A model installation is already running.".into());
        }
        let (sender, receiver) = watch::channel(false);
        inner.0 = Status {
            busy: true,
            phase: "preparing".into(),
            total: crate::model_catalog::MODEL.bytes,
            ..Default::default()
        };
        inner.1 = Some(sender);
        receiver
    };
    let _guard = RunGuard(installer);
    let result = async {
        let directory = app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join("models");
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(|error| error.to_string())?;
        let path = directory.join(crate::model_catalog::MODEL_FILENAME);
        let exists = tokio::fs::try_exists(&path)
            .await
            .map_err(|error| error.to_string())?;
        installer
            .inner
            .lock()
            .map_err(|_| "Installer unavailable.")?
            .0
            .phase = if exists { "verifying" } else { "downloading" }.into();
        let progress = |received| {
            if let Ok(mut inner) = installer.inner.lock() {
                inner.0.received = received;
            }
        };
        if exists {
            crate::download::verify_file(&path, &crate::model_catalog::MODEL, receiver, progress)
                .await?;
        } else {
            crate::download::fetch(
                &crate::download::client()?,
                &crate::model_catalog::MODEL,
                &path,
                receiver,
                progress,
            )
            .await?;
        }
        Ok::<_, String>(path.to_string_lossy().into_owned())
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
            inner.0.path = Some(path);
        }
        Err(error) => {
            inner.0.fail(error);
        }
    }
    Ok(inner.0.clone())
}
