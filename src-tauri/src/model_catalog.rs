//! Pinned installation metadata. User/model input cannot substitute URLs or hashes.
use crate::download::Asset;
use serde::Serialize;
use tauri::Manager;

pub const MODEL_FILENAME: &str = "MiniCPM5-2B.Q6_K.gguf";
pub const MODEL: Asset<'static> = Asset {
    url: "https://huggingface.co/prithivMLmods/MiniCPM5-2B-GGUF/resolve/8b969e82c3ea123d604242f6d97a93b98a452070/MiniCPM5-2B.Q6_K.gguf",
    bytes: 2_070_227_904,
    sha256: "d39e78a06dbb9b28ed9a9118b1370e992caf862003c925264ea33789b3e416bc",
};

/// Development path for the standard runtime. Packaged installs keep the
/// managed copy under the app data `runtimes` directory.
pub const STANDARD_RUNTIME_RELATIVE: &str = ".local/runtime/llama-server.exe";

pub fn model(filename: Option<&str>) -> Result<(&'static str, Asset<'static>), String> {
    match filename.unwrap_or(MODEL_FILENAME) {
        MODEL_FILENAME => Ok((MODEL_FILENAME, MODEL)),
        _ => Err("Choose a model from the managed catalog.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_preserves_default_and_rejects_other_paths() {
        assert_eq!(model(None).unwrap().0, MODEL_FILENAME);
        let (filename, asset) = model(Some(MODEL_FILENAME)).unwrap();
        assert_eq!(filename, "MiniCPM5-2B.Q6_K.gguf");
        assert_eq!(asset.bytes, 2_070_227_904);
        assert_eq!(asset.sha256.len(), 64);
        assert!(model(Some("../../other.gguf")).is_err());
        assert!(model(Some("other.gguf")).is_err());
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDownloadInfo {
    filename: &'static str,
    bytes: u64,
    sha256: &'static str,
    destination: String,
    available_bytes: u64,
    required_bytes: u64,
    destination_exists: bool,
}

#[tauri::command]
pub async fn model_download_info(app: tauri::AppHandle, filename: Option<String>) -> Result<ModelDownloadInfo, String> {
    let (filename, asset) = model(filename.as_deref())?;
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("models");
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| format!("Could not prepare model directory: {error}"))?;
    let destination = directory.join(filename);
    Ok(ModelDownloadInfo {
        filename,
        bytes: asset.bytes,
        sha256: asset.sha256,
        destination: destination.to_string_lossy().into_owned(),
        available_bytes: crate::download::available_space(&directory)?,
        required_bytes: crate::download::required_space(asset.bytes)?,
        destination_exists: tokio::fs::try_exists(&destination)
            .await
            .map_err(|error| error.to_string())?,
    })
}
