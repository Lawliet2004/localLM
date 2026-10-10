use serde::Serialize;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::Manager;
use tokio::sync::watch;

static JOB: AtomicU64 = AtomicU64::new(1);

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub busy: bool,
    pub phase: String,
    /// Aggregate bytes for the whole job across every archive or shard.
    pub received: u64,
    pub total: u64,
    pub path: Option<String>,
    pub error: Option<String>,
    /// 0 marks a recovered-on-disk state rather than a job this process ran.
    pub job_id: u64,
    /// Stable asset identity: "runtime:standard",
    /// "model:catalog:<filename>", "model:hf:<repo>@<rev>:<file>".
    pub key: String,
    pub label: String,
    pub asset: Option<String>,
    pub asset_index: u32,
    pub asset_count: u32,
    pub asset_received: u64,
    pub asset_total: u64,
    /// Bytes already on disk when the job started; not network transfer.
    pub saved: u64,
    /// Current transfer attempt, 1-based.
    pub attempt: u32,
    /// Non-fatal explanation: retry reason, a discarded prefix restart, a
    /// reused install, or recovered data.
    pub notice: Option<String>,
    pub updated_at: u64,
}
impl Status {
    pub(crate) fn fail(&mut self, error: String) {
        self.busy = false;
        self.phase = if matches!(
            error.as_str(),
            "Download cancelled." | "Runtime installation cancelled." | "Download paused."
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
    /// One in-flight job per slot; `begin` fails fast instead of queueing.
    pub(crate) run: tokio::sync::Mutex<()>,
}
impl Installer {
    pub(crate) fn status(&self) -> Status {
        self.inner
            .lock()
            .map(|inner| inner.0.clone())
            .unwrap_or_default()
    }
    pub(crate) fn begin(
        &self,
        key: &str,
        label: &str,
    ) -> Result<(RunGuard<'_>, watch::Receiver<bool>, u64), String> {
        let run = self.run.try_lock().map_err(|_| {
            let running = self
                .inner
                .lock()
                .ok()
                .map(|inner| inner.0.label.clone())
                .filter(|label| !label.is_empty());
            match running {
                Some(label) => format!(
                    "A download is already in progress ({label}). Pause it or wait for it to finish."
                ),
                None => {
                    "A download is already in progress. Pause it or wait for it to finish.".into()
                }
            }
        })?;
        let (sender, receiver) = watch::channel(false);
        let job_id = JOB.fetch_add(1, Ordering::Relaxed);
        {
            let mut inner = self.inner.lock().map_err(|_| "Installer unavailable.")?;
            inner.0 = Status {
                busy: true,
                phase: "preparing".into(),
                job_id,
                key: key.into(),
                label: label.into(),
                updated_at: now_ms(),
                ..Default::default()
            };
            inner.1 = Some(sender);
        }
        Ok((
            RunGuard {
                installer: self,
                _run: run,
            },
            receiver,
            job_id,
        ))
    }
    pub(crate) fn update(&self, change: impl FnOnce(&mut Status)) {
        if let Ok(mut inner) = self.inner.lock() {
            change(&mut inner.0);
            inner.0.updated_at = now_ms();
        }
    }
    /// Fold a transfer event into the job status. `prior` is the byte total
    /// of already-finished assets so `received` never moves backwards.
    pub(crate) fn progress(&self, prior: u64, event: crate::download::Event<'_>) {
        match event {
            crate::download::Event::Bytes(bytes) => self.update(|status| {
                status.asset_received = bytes;
                status.received = prior + bytes;
                if status.phase == "retrying" {
                    status.phase = "downloading".into();
                }
            }),
            crate::download::Event::Retry {
                attempt,
                delay,
                reason,
                ..
            } => self.update(|status| {
                status.phase = "retrying".into();
                status.attempt = attempt;
                status.notice = Some(format!(
                    "{reason} Retrying in {} seconds.",
                    delay.as_secs().max(1)
                ));
            }),
            crate::download::Event::Restart(reason) => self.update(|status| {
                status.notice = Some(reason.into());
            }),
        }
    }
    pub(crate) fn finish<T: AsRef<Path>>(&self, result: Result<T, String>) -> Status {
        let mut inner = match self.inner.lock() {
            Ok(inner) => inner,
            Err(poisoned) => poisoned.into_inner(),
        };
        inner.0.busy = false;
        match result {
            Ok(path) => {
                inner.0.phase = "ready".into();
                inner.0.path = Some(path.as_ref().to_string_lossy().into_owned());
            }
            Err(error) => inner.0.fail(error),
        }
        inner.0.updated_at = now_ms();
        inner.0.clone()
    }
    pub(crate) fn cancel(&self, job_id: Option<u64>) -> Result<(), String> {
        let inner = self.inner.lock().map_err(|_| "Installer unavailable.")?;
        if let Some(job_id) = job_id {
            if inner.1.is_none() || inner.0.job_id != job_id {
                return Err("That download is no longer running.".into());
            }
        }
        if let Some(sender) = &inner.1 {
            let _ = sender.send(true);
        }
        Ok(())
    }
}
pub(crate) struct RunGuard<'a> {
    installer: &'a Installer,
    _run: tokio::sync::MutexGuard<'a, ()>,
}
impl Drop for RunGuard<'_> {
    fn drop(&mut self) {
        if let Ok(mut inner) = self.installer.inner.lock() {
            inner.1 = None;
            if inner.0.busy {
                inner.0.busy = false;
                inner.0.phase = "interrupted".into();
                inner.0.error = Some(
                    "Installation was interrupted. Retry to verify or download the files.".into(),
                );
                inner.0.updated_at = now_ms();
            }
        }
    }
}
#[tauri::command]
pub fn model_install_status(state: tauri::State<'_, crate::AppState>) -> Result<Status, String> {
    Ok(state.model_installer.status())
}
#[tauri::command]
pub fn cancel_model_install(
    state: tauri::State<'_, crate::AppState>,
    job_id: Option<u64>,
) -> Result<(), String> {
    state.model_installer.cancel(job_id)
}
/// Every bounded download slot that has ever run a job or recovered state
/// from disk. Reading status is harmless.
#[tauri::command]
pub fn download_jobs(state: tauri::State<'_, crate::AppState>) -> Vec<Status> {
    [
        &state.runtime_installer,
        &state.model_installer,
    ]
    .into_iter()
    .map(Installer::status)
    .filter(|status| status.job_id != 0 || !status.phase.is_empty())
    .collect()
}
#[tauri::command]
pub async fn install_model(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    filename: Option<String>,
) -> Result<Status, String> {
    let (filename, asset) = crate::model_catalog::model(filename.as_deref())?;
    let installer = &state.model_installer;
    let (_guard, receiver, _job_id) =
        installer.begin(&format!("model:catalog:{filename}"), filename)?;
    let result = async {
        let directory = app
            .path()
            .app_data_dir()
            .map_err(|error| error.to_string())?
            .join("models");
        tokio::fs::create_dir_all(&directory)
            .await
            .map_err(|error| error.to_string())?;
        let path = directory.join(filename);
        let exists = tokio::fs::try_exists(&path)
            .await
            .map_err(|error| error.to_string())?;
        let saved = if exists {
            asset.bytes
        } else {
            tokio::fs::metadata(crate::download::partial_path(&path))
                .await
                .ok()
                .filter(|metadata| metadata.is_file())
                .map(|metadata| metadata.len().min(asset.bytes))
                .unwrap_or(0)
        };
        installer.update(|status| {
            status.saved = saved;
            status.received = saved;
            status.total = asset.bytes;
            status.asset = Some(filename.into());
            status.asset_index = 1;
            status.asset_count = 1;
            status.asset_received = saved;
            status.asset_total = asset.bytes;
            status.phase = if exists {
                "verifying"
            } else if saved > 0 {
                "resuming"
            } else {
                "downloading"
            }
            .into();
        });
        if exists {
            crate::download::verify_file(&path, &asset, receiver, |received| {
                installer.update(|status| {
                    status.received = received;
                    status.asset_received = received;
                });
            })
            .await?;
        } else {
            crate::download::fetch(
                &crate::download::client()?,
                &asset,
                &path,
                receiver,
                |event| installer.progress(0, event),
            )
            .await?;
        }
        Ok::<_, String>(path)
    }
    .await;
    Ok(installer.finish(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
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
        drop(RunGuard {
            installer: &installer,
            _run: installer.run.try_lock().unwrap(),
        });
        let status = installer.inner.lock().unwrap().0.clone();
        assert!(!status.busy);
        assert_eq!(status.phase, "interrupted");
        assert!(status.error.is_some());
        installer.inner.lock().unwrap().0 = Status {
            phase: "ready".into(),
            path: Some("verified-model".into()),
            ..Default::default()
        };
        drop(RunGuard {
            installer: &installer,
            _run: installer.run.try_lock().unwrap(),
        });
        assert_eq!(installer.inner.lock().unwrap().0.phase, "ready");
    }
    #[test]
    fn second_job_on_the_same_slot_fails_fast() {
        let installer = Installer::default();
        let (_guard, _receiver, job_id) = installer.begin("model:test", "a.gguf").unwrap();
        assert!(job_id > 0);
        let error = installer.begin("model:test", "b.gguf").err().unwrap();
        assert!(error.contains("a.gguf"), "{error}");
        let status = installer.status();
        assert_eq!(status.job_id, job_id);
        assert_eq!(status.key, "model:test");
        assert_eq!(status.label, "a.gguf");
        assert!(status.busy);
        assert!(status.updated_at > 0);
    }
    #[test]
    fn cancel_only_signals_the_matching_job() {
        let installer = Installer::default();
        let (_guard, receiver, job_id) = installer.begin("model:test", "a.gguf").unwrap();
        assert_eq!(
            installer.cancel(Some(job_id + 1)).unwrap_err(),
            "That download is no longer running."
        );
        assert!(!*receiver.borrow());
        installer.cancel(Some(job_id)).unwrap();
        assert!(*receiver.borrow());
    }
    #[test]
    fn begin_finish_and_update_drive_the_status_shape() {
        let installer = Installer::default();
        let (_guard, _receiver, job_id) = installer.begin("runtime:standard", "runtime").unwrap();
        installer.update(|status| status.total = 42);
        let status = installer.finish(Ok(Path::new("bundle").to_path_buf()));
        assert!(!status.busy);
        assert_eq!(status.phase, "ready");
        assert_eq!(status.path.as_deref(), Some("bundle"));
        assert_eq!(status.job_id, job_id);
        assert_eq!(status.total, 42);
        let status = installer.finish(Err("Download cancelled.".into()) as Result<PathBuf, _>);
        assert_eq!(status.phase, "cancelled");
    }
}
