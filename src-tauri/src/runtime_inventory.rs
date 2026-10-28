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
    let mut items = Vec::new();
    for package in crate::runtime_catalog::packages()? {
        items.extend(inspect_package(root, &package)?);
    }
    Ok(items)
}
#[cfg(test)]
fn inspect_assets(
    root: &Path,
    assets: &[crate::runtime_archive::RuntimeAsset],
) -> Result<Vec<InstalledRuntime>, String> {
    let mut package = crate::runtime_catalog::package(None)?;
    package.assets = assets.to_vec();
    inspect_package(root, &package)
}
fn inspect_package(
    root: &Path,
    package: &crate::runtime_catalog::Package,
) -> Result<Vec<InstalledRuntime>, String> {
    let assets = &package.assets;
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
        let Some(suffix) = name.strip_prefix(&package.prefix()) else {
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
        let files = bundle_files(&root.join(&name), assets);
        let problem = files.as_ref().err().cloned().or_else(|| {
            files.as_ref().ok().and_then(|files| {
                files
                    .iter()
                    .find_map(|file| match bundle.symlink_metadata(&file.name) {
                        Ok(metadata)
                            if metadata.is_file()
                                && !metadata.file_type().is_symlink()
                                && metadata.len() == file.bytes =>
                        {
                            None
                        }
                        _ => Some(format!("Missing or unexpected file: {}", file.name)),
                    })
            })
        });
        result.push(InstalledRuntime {
            id: name.clone(),
            path: root
                .join(name)
                .join(&package.executable)
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
    fn prism_receipts_are_isolated_by_package_and_reuse_rehashes_every_file() {
        let root = tempfile::tempdir().unwrap();
        let package = crate::runtime_catalog::package(Some("prism-linux-cpu")).unwrap();
        let directory = root
            .path()
            .join(format!("{}{}", package.prefix(), uuid::Uuid::new_v4()));
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("llama-server"), b"good").unwrap();
        std::fs::write(directory.join("libllama.so"), b"library").unwrap();
        assert!(!inspect_package(root.path(), &package).unwrap()[0].complete);
        receipt_assets(&directory, &package.assets).unwrap();
        assert_eq!(
            reusable_package(root.path(), &package).unwrap(),
            Some(directory.join("llama-server"))
        );
        let other = crate::runtime_catalog::package(Some("prism-linux-cuda")).unwrap();
        assert!(reusable_package(root.path(), &other).unwrap().is_none());
        std::fs::write(directory.join("libllama.so"), b"changed").unwrap();
        assert!(reusable_package(root.path(), &package).unwrap().is_none());
        std::fs::remove_file(directory.join("libllama.so")).unwrap();
        assert!(!inspect_package(root.path(), &package).unwrap()[0].complete);
    }

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
pub(crate) fn reusable_package(
    root: &Path,
    package: &crate::runtime_catalog::Package,
) -> Result<Option<std::path::PathBuf>, String> {
    reusable_checked(root, package)
}
#[cfg(test)]
fn reusable_assets(
    root: &Path,
    assets: &[crate::runtime_archive::RuntimeAsset],
) -> Result<Option<std::path::PathBuf>, String> {
    let mut package = crate::runtime_catalog::package(None)?;
    package.assets = assets.to_vec();
    reusable_checked(root, &package)
}
fn reusable_checked(
    root: &Path,
    package: &crate::runtime_catalog::Package,
) -> Result<Option<std::path::PathBuf>, String> {
    let assets = &package.assets;
    use sha2::{Digest, Sha256};
    use std::io::Read;
    for item in inspect_package(root, package)? {
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
        let files = bundle_files(parent, assets)?;
        let mut valid = hashes.len() == files.len();
        for file in &files {
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

// Whole-archive SHA-256 pins Prism bundles. Record bounded, flat extracted
// files in the receipt, then rehash them before any installation is reused.
fn extracted_files(
    bundle: &Path,
    assets: &[crate::runtime_archive::RuntimeAsset],
) -> Result<Vec<crate::runtime_archive::RuntimeFile>, String> {
    if assets.iter().all(|a| !a.files.is_empty()) {
        return Ok(assets.iter().flat_map(|a| a.files.clone()).collect());
    }
    let mut files = Vec::new();
    for entry in std::fs::read_dir(bundle).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "verified-files.json" {
            continue;
        }
        if files.len() >= 1000
            || !crate::runtime_archive::safe_leaf(&name)
            || !entry
                .file_type()
                .is_ok_and(|t| t.is_file() && !t.is_symlink())
        {
            return Err("Unexpected extracted runtime entry.".into());
        }
        files.push(crate::runtime_archive::RuntimeFile {
            name,
            bytes: entry.metadata().map_err(|e| e.to_string())?.len(),
        });
    }
    Ok(files)
}
fn bundle_files(
    bundle: &Path,
    assets: &[crate::runtime_archive::RuntimeAsset],
) -> Result<Vec<crate::runtime_archive::RuntimeFile>, String> {
    if assets.iter().all(|a| !a.files.is_empty()) {
        return Ok(assets.iter().flat_map(|a| a.files.clone()).collect());
    }
    let receipt = bundle.join("verified-files.json");
    if !receipt
        .symlink_metadata()
        .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink() && m.len() < 65536)
    {
        return Err("Runtime verification receipt missing or invalid.".into());
    }
    let hashes: std::collections::BTreeMap<String, String> =
        serde_json::from_slice(&std::fs::read(receipt).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if hashes.is_empty()
        || hashes.len() > 1000
        || !hashes
            .keys()
            .any(|n| n == "llama-server" || n == "llama-server.exe")
    {
        return Err("Runtime receipt has no server executable.".into());
    }
    hashes
        .into_iter()
        .map(|(name, hash)| {
            if !crate::runtime_archive::safe_leaf(&name)
                || hash.len() != 64
                || !hash.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err("Invalid runtime receipt entry.".into());
            }
            let bytes = bundle
                .join(&name)
                .symlink_metadata()
                .map(|m| m.len())
                .unwrap_or(0);
            Ok(crate::runtime_archive::RuntimeFile { name, bytes })
        })
        .collect()
}
pub(crate) fn receipt_assets(
    bundle: &Path,
    assets: &[crate::runtime_archive::RuntimeAsset],
) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    use std::io::{Read, Write};
    let mut hashes = std::collections::BTreeMap::new();
    for file in extracted_files(bundle, assets)? {
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
    let package = crate::runtime_catalog::packages()?
        .into_iter()
        .find(|p| id.starts_with(&p.prefix()))
        .ok_or("Unknown runtime package.")?;
    let expected: Vec<_> = bundle_files(path, &package.assets)?
        .into_iter()
        .map(|f| f.name)
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
