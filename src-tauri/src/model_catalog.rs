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
pub async fn model_download_info(app: tauri::AppHandle) -> Result<ModelDownloadInfo, String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("models");
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|error| format!("Could not prepare model directory: {error}"))?;
    let destination = directory.join(MODEL_FILENAME);
    Ok(ModelDownloadInfo {
        filename: MODEL_FILENAME,
        bytes: MODEL.bytes,
        sha256: MODEL.sha256,
        destination: destination.to_string_lossy().into_owned(),
        available_bytes: crate::download::available_space(&directory)?,
        required_bytes: crate::download::required_space(MODEL.bytes)?,
        destination_exists: tokio::fs::try_exists(&destination)
            .await
            .map_err(|error| error.to_string())?,
    })
}
