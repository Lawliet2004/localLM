//! Managed storage pointer switches only after a complete, verified copy.
//! The source remains intact, so interruption never destroys the only copy.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Location {
    root: PathBuf,
}

pub fn root(data: &Path) -> Result<PathBuf, String> {
    match std::fs::read(data.join("managed-storage.json")) {
        Ok(bytes) => {
            let config: Location = serde_json::from_slice(&bytes)
                .map_err(|e| format!("Managed storage setting is damaged: {e}"))?;
            if !config.root.is_absolute() {
                return Err("Managed storage path must be absolute.".into());
            }
            if !config.root.is_dir() {
                return Err(
                    "Managed storage is unavailable. Reconnect its volume before continuing."
                        .into(),
                );
            }
            Ok(config.root)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(data.to_path_buf()),
        Err(e) => Err(e.to_string()),
    }
}
fn regular_files(root: &Path, paths: &mut Vec<PathBuf>) -> Result<u64, String> {
    match root.symlink_metadata() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e.to_string()),
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(
                "Storage migration refuses symbolic links. Review the source folder first.".into(),
            );
        }
        Ok(_) => {}
    }
    let mut total = 0u64;
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            return Err(
                "Storage migration refuses symbolic links. Review the source folder first.".into(),
            );
        }
        if kind.is_dir() {
            total = total
                .checked_add(regular_files(&entry.path(), paths)?)
                .ok_or("Storage size overflow")?;
        } else if kind.is_file() {
            total = total
                .checked_add(entry.metadata().map_err(|e| e.to_string())?.len())
                .ok_or("Storage size overflow")?;
            paths.push(entry.path());
        } else {
            return Err("Storage contains an unsupported file type.".into());
        }
    }
    Ok(total)
}
fn hash(path: &Path) -> Result<Vec<u8>, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut reader = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 65536];
    loop {
        let n = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    Ok(digest.finalize().to_vec())
}
fn copy_storage(source: &Path, target: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target).map_err(|e| e.to_string())?;
    let target = std::fs::canonicalize(target).map_err(|e| e.to_string())?;
    let source = std::fs::canonicalize(source).map_err(|e| e.to_string())?;
    if target == source {
        return Ok(());
    }
    if target.starts_with(&source) || source.starts_with(&target) {
        return Err("Choose a storage folder outside the current storage folder.".into());
    }
    let marker = target.join(".locallm-migration.json");
    let owner = source.to_string_lossy().into_owned();
    if marker.exists() {
        if std::fs::read_to_string(&marker).map_err(|e| e.to_string())? != owner {
            return Err("This folder belongs to another migration.".into());
        }
    } else {
        if std::fs::read_dir(&target)
            .map_err(|e| e.to_string())?
            .next()
            .is_some()
        {
            return Err("Choose an empty destination folder.".into());
        }
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&marker)
            .map_err(|e| e.to_string())?;
        use std::io::Write;
        file.write_all(owner.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
    }
    let mut existing = Vec::new();
    regular_files(&target, &mut existing)?;
    let mut files = Vec::new();
    regular_files(&source.join("models"), &mut files)?;
    regular_files(&source.join("runtimes"), &mut files)?;
    let needed = files.iter().try_fold(0u64, |total, from| {
        let to = target.join(from.strip_prefix(&source).map_err(|e| e.to_string())?);
        if to.symlink_metadata().is_ok() {
            if to
                .symlink_metadata()
                .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
                && hash(from)? == hash(&to)?
            {
                return Ok(total);
            }
            return Err(
                "Existing migration files differ; source retained. Choose a new empty folder."
                    .to_string(),
            );
        }
        total
            .checked_add(from.metadata().map_err(|e| e.to_string())?.len())
            .ok_or_else(|| "Storage size overflow".to_string())
    })?;
    if crate::download::available_space(&target)? < crate::download::required_space(needed)? {
        return Err(
            "Not enough destination space to copy managed storage and retain recovery reserve."
                .into(),
        );
    }
    for from in files {
        let to = target.join(from.strip_prefix(&source).map_err(|e| e.to_string())?);
        let parent = to.parent().ok_or("Missing destination parent")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        // A retry may reuse verified completed copies, but never an unexpected file.
        if to.symlink_metadata().is_ok() {
            if to
                .symlink_metadata()
                .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
                && hash(&from)? == hash(&to)?
            {
                continue;
            }
            return Err(format!(
                "Migration destination differs: {}. Choose an empty folder.",
                to.display()
            ));
        }
        let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
        let mut input = std::fs::File::open(&from).map_err(|e| e.to_string())?;
        std::io::copy(&mut input, &mut tmp).map_err(|e| e.to_string())?;
        tmp.as_file()
            .set_permissions(input.metadata().map_err(|e| e.to_string())?.permissions())
            .map_err(|e| e.to_string())?;
        tmp.as_file().sync_all().map_err(|e| e.to_string())?;
        if hash(&from)? != hash(tmp.path())? {
            return Err("Storage copy verification failed; source retained.".into());
        }
        tmp.persist_noclobber(to).map_err(|e| e.to_string())?;
    }
    Ok(())
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Info {
    root: String,
    available_bytes: u64,
}
#[tauri::command]
pub fn managed_storage_info(state: tauri::State<'_, crate::AppState>) -> Result<Info, String> {
    let root = root(&state.data_dir)?;
    Ok(Info {
        root: root.to_string_lossy().into_owned(),
        available_bytes: crate::download::available_space(&root)?,
    })
}
#[tauri::command]
pub async fn move_managed_storage(
    state: tauri::State<'_, crate::AppState>,
    destination: String,
) -> Result<Info, String> {
    move_storage(&state, destination).await
}
async fn move_storage(state: &crate::AppState, destination: String) -> Result<Info, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Stop active operations before moving storage.")?;
    let _installation = state
        .installation_operation
        .try_lock()
        .map_err(|_| "Finish or cancel downloads before moving storage.")?;
    if state.runtime.lock().await.inspect().phase != "stopped" {
        return Err("Unload the model before moving storage.".into());
    }
    let source = root(&state.data_dir)?;
    let target = PathBuf::from(destination);
    if !target.is_absolute() {
        return Err("Choose an absolute storage folder.".into());
    }
    let worker_target = target.clone();
    let worker_source = source.clone();
    tokio::task::spawn_blocking(move || copy_storage(&worker_source, &worker_target))
        .await
        .map_err(|_| "Storage migration worker stopped; source retained.")??;
    crate::model_library::migrate_storage_profiles(state, &source, &target)?;
    // All references point to existing verified copies before the pointer switch.
    let config = Location {
        root: std::fs::canonicalize(&target).map_err(|e| e.to_string())?,
    };
    use std::io::Write;
    let mut tmp = tempfile::NamedTempFile::new_in(&state.data_dir).map_err(|e| e.to_string())?;
    tmp.write_all(&serde_json::to_vec(&config).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    tmp.as_file().sync_all().map_err(|e| e.to_string())?;
    tmp.persist(state.data_dir.join("managed-storage.json"))
        .map_err(|e| e.to_string())?;
    #[cfg(unix)]
    std::fs::File::open(&state.data_dir)
        .and_then(|f| f.sync_all())
        .map_err(|e| e.to_string())?;
    let root = root(&state.data_dir)?;
    Ok(Info {
        root: root.to_string_lossy().into_owned(),
        available_bytes: crate::download::available_space(&root)?,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn migration_persists_location_and_remaps_settings_and_profiles() {
        let temp = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(temp.path());
        let target = tempfile::tempdir().unwrap();
        let dir = state.data_dir.join("models").join("a".repeat(64));
        std::fs::create_dir_all(&dir).unwrap();
        let model = dir.join("model.gguf");
        std::fs::write(&model, b"GGUF").unwrap();
        let mut prefs = state.database().unwrap().preferences().unwrap();
        prefs.model_path = model.to_string_lossy().into_owned();
        state.database().unwrap().save_preferences(&prefs).unwrap();
        crate::model_library::save_active_profile(
            &state,
            &crate::runtime_config::RuntimeConfig::default(),
        )
        .unwrap();
        let result = move_storage(&state, target.path().to_string_lossy().into_owned())
            .await
            .unwrap();
        assert_eq!(Path::new(&result.root), target.path());
        assert_eq!(root(&state.data_dir).unwrap(), target.path());
        let expected = target
            .path()
            .join("models")
            .join("a".repeat(64))
            .join("model.gguf");
        assert_eq!(
            Path::new(&state.database().unwrap().preferences().unwrap().model_path),
            expected
        );
        assert!(model.exists());
        assert!(expected.exists());
        assert_eq!(
            std::fs::read_dir(state.data_dir.join("model-profiles"))
                .unwrap()
                .count(),
            2
        );
        std::fs::write(state.data_dir.join("managed-storage.json"), b"{bad").unwrap();
        assert!(root(&state.data_dir).unwrap_err().contains("damaged"));
    }
    #[cfg(unix)]
    #[test]
    fn migration_rejects_links_without_touching_their_targets() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        std::fs::create_dir(source.path().join("models")).unwrap();
        std::os::unix::fs::symlink(outside.path(), source.path().join("models/escape.gguf"))
            .unwrap();
        assert!(copy_storage(source.path(), target.path())
            .unwrap_err()
            .contains("symbolic links"));
        assert!(outside.path().exists());
        std::fs::remove_file(source.path().join("models/escape.gguf")).unwrap();
        std::fs::remove_dir(source.path().join("models")).unwrap();
        std::os::unix::fs::symlink(target.path(), source.path().join("models")).unwrap();
        assert!(copy_storage(source.path(), target.path())
            .unwrap_err()
            .contains("symbolic links"));
    }
    #[test]
    fn copies_partials_and_runtimes_and_retries_without_touching_source() {
        let source = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        std::fs::create_dir(source.path().join("models")).unwrap();
        std::fs::write(source.path().join("models/a.gguf.part"), b"saved prefix").unwrap();
        copy_storage(source.path(), target.path()).unwrap();
        copy_storage(source.path(), target.path()).unwrap();
        assert_eq!(
            std::fs::read(target.path().join("models/a.gguf.part")).unwrap(),
            b"saved prefix"
        );
        assert!(source.path().join("models/a.gguf.part").exists());
        std::fs::write(target.path().join("models/a.gguf.part"), b"unexpected").unwrap();
        assert!(copy_storage(source.path(), target.path()).is_err());
    }
}
