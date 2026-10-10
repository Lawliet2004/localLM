use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::Manager;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledRuntime {
    id: String,
    path: String,
    complete: bool,
    problem: Option<String>,
}

pub(crate) fn inspect(
    root: &Path,
    prefix: &str,
    assets: &[crate::runtime_archive::RuntimeAsset],
) -> Result<Vec<InstalledRuntime>, String> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let directory = cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority())
        .map_err(|error| error.to_string())?;
    let marker = format!("{prefix}-");
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
        let Some(suffix) = name.strip_prefix(&marker) else {
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

/// The first complete published bundle for this prefix, or None. An
/// incomplete bundle never hides a complete one that sorts after it.
pub(crate) fn find_complete(
    root: &Path,
    prefix: &str,
    assets: &[crate::runtime_archive::RuntimeAsset],
) -> Result<Option<PathBuf>, String> {
    Ok(inspect(root, prefix, assets)?
        .into_iter()
        .find(|runtime| runtime.complete)
        .map(|runtime| PathBuf::from(runtime.path)))
}

#[tauri::command]
pub async fn list_installed_runtimes(
    app: tauri::AppHandle,
) -> Result<Vec<InstalledRuntime>, String> {
    let data = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    tokio::task::spawn_blocking(move || {
        inspect(
            &data.join("runtimes"),
            crate::runtime_install::STANDARD_PREFIX,
            &crate::runtime_archive::catalog()?,
        )
    })
    .await
    .map_err(|_| "Runtime inventory worker stopped.")?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_archive::{RuntimeAsset, RuntimeFile};

    fn fixture_assets() -> Vec<RuntimeAsset> {
        vec![RuntimeAsset {
            name: "a.zip".into(),
            url: "unused".into(),
            sha256: "0".repeat(64),
            bytes: 10,
            files: vec![RuntimeFile {
                name: "llama-server.exe".into(),
                bytes: 7,
            }],
        }]
    }

    #[test]
    fn discovers_published_bundles_and_reports_incomplete_files() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join(".locallm-runtime-partial")).unwrap();
        std::fs::create_dir(temp.path().join("unrelated")).unwrap();
        let assets = crate::runtime_archive::catalog().unwrap();
        let name = format!("{}-{}", crate::runtime_install::STANDARD_PREFIX, uuid::Uuid::new_v4());
        let path = temp.path().join(&name);
        std::fs::create_dir(&path).unwrap();
        let items = inspect(temp.path(), crate::runtime_install::STANDARD_PREFIX, &assets).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].id, name);
        assert!(!items[0].complete);
        assert!(items[0].problem.as_ref().unwrap().contains("ggml-cuda.dll"));
        assert!(items[0].path.ends_with("llama-server.exe"));
    }

    #[test]
    fn find_complete_skips_an_incomplete_bundle_that_sorts_first() {
        let temp = tempfile::tempdir().unwrap();
        let assets = fixture_assets();
        let incomplete = temp
            .path()
            .join("llama-test-00000000-0000-0000-0000-000000000000");
        std::fs::create_dir(&incomplete).unwrap();
        let complete = temp
            .path()
            .join("llama-test-ffffffff-ffff-ffff-ffff-ffffffffffff");
        std::fs::create_dir(&complete).unwrap();
        std::fs::write(complete.join("llama-server.exe"), b"fixture").unwrap();
        let found = find_complete(temp.path(), "llama-test", &assets)
            .unwrap()
            .unwrap();
        assert_eq!(found, complete.join("llama-server.exe"));
        std::fs::remove_file(complete.join("llama-server.exe")).unwrap();
        assert!(find_complete(temp.path(), "llama-test", &assets)
            .unwrap()
            .is_none());
    }
}
