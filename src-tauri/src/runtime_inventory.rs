use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledRuntime {
    id: String,
    path: String,
    complete: bool,
    problem: Option<String>,
}

fn inspect(root: &Path) -> Result<Vec<InstalledRuntime>, String> {
    inspect_assets(root, &crate::runtime_archive::catalog()?)
}
fn inspect_assets(
    root: &Path,
    assets: &[crate::runtime_archive::RuntimeAsset],
) -> Result<Vec<InstalledRuntime>, String> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let directory = cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority())
        .map_err(|error| error.to_string())?;
    let mut result = Vec::new();
    for (count, entry) in directory
        .entries()
        .map_err(|error| error.to_string())?
        .enumerate()
    {
        if count >= 1000 {
            return Err("Too many runtime directory entries to inspect. Open the runtime folder to review its contents.".into());
        }
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(suffix) = name.strip_prefix("llama-b10855-cuda12.4-") else {
            continue;
        };
        if uuid::Uuid::parse_str(suffix).is_err()
            || !entry
                .file_type()
                .map_err(|error| error.to_string())?
                .is_dir()
        {
            continue;
        }
        let bundle = directory
            .open_dir(&name)
            .map_err(|error| error.to_string())?;
        let problem = assets
            .iter()
            .flat_map(|asset| &asset.files)
            .find_map(|file| match bundle.symlink_metadata(&file.name) {
                Ok(metadata)
                    if metadata.is_file()
                        && !metadata.file_type().is_symlink()
                        && metadata.len() == file.bytes =>
                {
                    None
                }
                _ => Some(format!("Missing or unexpected file: {}", file.name)),
            });
        result.push(InstalledRuntime {
            id: name.clone(),
            path: root
                .join(name)
                .join("llama-server.exe")
                .to_string_lossy()
                .into_owned(),
            complete: problem.is_none(),
            problem,
        });
    }
    result.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(result)
}

#[tauri::command]
pub async fn list_installed_runtimes(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<InstalledRuntime>, String> {
    let root = crate::managed_storage::root(&state.data_dir)?.join("runtimes");
    tokio::task::spawn_blocking(move || inspect(&root))
        .await
        .map_err(|_| "Runtime inventory worker stopped.")?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reuses_only_complete_bundles_whose_extracted_hashes_still_match() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp
            .path()
            .join(format!("llama-b10855-cuda12.4-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("llama-server.exe"), b"good").unwrap();
        let assets = vec![crate::runtime_archive::RuntimeAsset {
            name: "test.zip".into(),
            url: "https://example.com/test.zip".into(),
            sha256: "a".repeat(64),
            bytes: 1,
            files: vec![crate::runtime_archive::RuntimeFile {
                name: "llama-server.exe".into(),
                bytes: 4,
            }],
        }];
        assert!(reusable_assets(temp.path(), &assets).unwrap().is_none());
        receipt_assets(&dir, &assets).unwrap();
        assert_eq!(
            reusable_assets(temp.path(), &assets).unwrap(),
            Some(dir.join("llama-server.exe"))
        );
        std::fs::write(dir.join("llama-server.exe"), b"evil").unwrap();
        assert!(reusable_assets(temp.path(), &assets).unwrap().is_none());
        std::fs::remove_file(dir.join("llama-server.exe")).unwrap();
        assert!(!inspect_assets(temp.path(), &assets).unwrap()[0].complete);
    }
    #[test]
    fn discovers_published_bundles_and_reports_incomplete_files() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join(".locallm-runtime-partial")).unwrap();
        std::fs::create_dir(temp.path().join("unrelated")).unwrap();
        let name = format!("llama-b10855-cuda12.4-{}", uuid::Uuid::new_v4());
        let path = temp.path().join(&name);
        std::fs::create_dir(&path).unwrap();
        let items = inspect(temp.path()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, name);
        assert!(!items[0].complete);
        assert!(items[0].problem.as_ref().unwrap().contains("ggml-cuda.dll"));
        assert!(items[0].path.ends_with("llama-server.exe"));
    }
}

/// Only bundles with a receipt from verified extraction can be reused.
pub(crate) fn reusable(root: &Path) -> Result<Option<std::path::PathBuf>, String> {
    reusable_assets(root, &crate::runtime_archive::catalog()?)
}
fn reusable_assets(
    root: &Path,
    assets: &[crate::runtime_archive::RuntimeAsset],
) -> Result<Option<std::path::PathBuf>, String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    for item in inspect_assets(root, assets)? {
        if !item.complete {
            continue;
        }
        let parent = Path::new(&item.path)
            .parent()
            .ok_or("Runtime parent missing")?;
        let receipt_path = parent.join("verified-files.json");
        if !receipt_path
            .symlink_metadata()
            .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink() && m.len() < 65536)
        {
            continue;
        }
        let Ok(receipt) = std::fs::read(&receipt_path) else {
            continue;
        };
        let Ok(hashes) =
            serde_json::from_slice::<std::collections::BTreeMap<String, String>>(&receipt)
        else {
            continue;
        };
        let mut valid = hashes.len() == assets.iter().map(|a| a.files.len()).sum::<usize>();
        for file in assets.iter().flat_map(|a| &a.files) {
            let mut input =
                std::fs::File::open(parent.join(&file.name)).map_err(|e| e.to_string())?;
            let mut hash = Sha256::new();
            let mut buffer = [0u8; 65536];
            loop {
                let n = input.read(&mut buffer).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                hash.update(&buffer[..n]);
            }
            valid &= hashes.get(&file.name) == Some(&format!("{:x}", hash.finalize()));
        }
        if valid {
            return Ok(Some(item.path.into()));
        }
    }
    Ok(None)
}
pub(crate) fn receipt(bundle: &Path) -> Result<(), String> {
    receipt_assets(bundle, &crate::runtime_archive::catalog()?)
}
fn receipt_assets(
    bundle: &Path,
    assets: &[crate::runtime_archive::RuntimeAsset],
) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    let mut hashes = std::collections::BTreeMap::new();
    for file in assets.iter().flat_map(|a| &a.files) {
        let mut input = std::fs::File::open(bundle.join(&file.name)).map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut buffer = [0u8; 65536];
        loop {
            let n = input.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        hashes.insert(file.name.clone(), format!("{:x}", hash.finalize()));
    }
    let mut output =
        std::fs::File::create(bundle.join("verified-files.json")).map_err(|e| e.to_string())?;
    output
        .write_all(&serde_json::to_vec(&hashes).map_err(|e| e.to_string())?)
        .and_then(|_| output.sync_all())
        .map_err(|e| e.to_string())
}
#[tauri::command]
pub async fn delete_installed_runtime(
    state: tauri::State<'_, crate::AppState>,
    id: String,
) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Stop active operations before removing runtimes.")?;
    let _installation = state
        .installation_operation
        .try_lock()
        .map_err(|_| "Wait for installation before removing runtimes.")?;
    if state.runtime.lock().await.inspect().phase != "stopped" {
        return Err("Unload the model before removing a runtime.".into());
    }
    let root = crate::managed_storage::root(&state.data_dir)?.join("runtimes");
    let items = inspect(&root)?;
    let Some(item) = items.into_iter().find(|i| i.id == id) else {
        return Ok(());
    };
    let path = Path::new(&item.path)
        .parent()
        .ok_or("Runtime folder unavailable")?;
    let bundle = cap_std::fs::Dir::open_ambient_dir(path, cap_std::ambient_authority())
        .map_err(|e| e.to_string())?;
    let expected: Vec<_> = crate::runtime_archive::catalog()?
        .into_iter()
        .flat_map(|a| a.files.into_iter().map(|f| f.name))
        .chain(std::iter::once("verified-files.json".into()))
        .collect();
    let entries = bundle
        .entries()
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    for entry in &entries {
        if !expected
            .iter()
            .any(|n| entry.file_name() == std::ffi::OsStr::new(n))
            || !entry
                .file_type()
                .is_ok_and(|t| t.is_file() && !t.is_symlink())
        {
            return Err(
                "Runtime contains unexpected files or links; review it manually before removal."
                    .into(),
            );
        }
    }
    for entry in entries {
        bundle
            .remove_file(entry.file_name())
            .map_err(|e| e.to_string())?;
    }
    drop(bundle);
    std::fs::remove_dir(path).map_err(|e| e.to_string())?;
    let mut preferences = state.database()?.preferences()?;
    if Path::new(&preferences.runtime_path).parent() == Some(path) {
        preferences.runtime_path.clear();
        state.database()?.save_preferences(&preferences)?;
    }
    Ok(())
}
