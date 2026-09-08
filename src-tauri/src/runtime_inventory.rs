use serde::Serialize;
use std::path::Path;
use tauri::Manager;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledRuntime {
    id: String,
    path: String,
    complete: bool,
    problem: Option<String>,
}

fn inspect(root: &Path) -> Result<Vec<InstalledRuntime>, String> {
    if !root.exists() {
        return Ok(Vec::new());
    }
    let directory = cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority())
        .map_err(|error| error.to_string())?;
    let assets = crate::runtime_archive::catalog()?;
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
    app: tauri::AppHandle,
) -> Result<Vec<InstalledRuntime>, String> {
    let root = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("runtimes");
    tokio::task::spawn_blocking(move || inspect(&root))
        .await
        .map_err(|_| "Runtime inventory worker stopped.")?
}

#[cfg(test)]
mod tests {
    use super::*;
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
