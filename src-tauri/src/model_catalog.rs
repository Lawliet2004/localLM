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
pub const BONSAI_FILENAME: &str = "Ternary-Bonsai-8B-Q2_0.gguf";
pub const BONSAI_CONTEXT_LENGTH: u32 = 65_536;
pub const BONSAI: Asset<'static> = Asset {
    url: "https://huggingface.co/prism-ml/Ternary-Bonsai-8B-gguf/resolve/c2aefbeb4b24469cd11579c3384b990404c17a30/Ternary-Bonsai-8B-Q2_0.gguf",
    bytes: 2_182_184_672,
    sha256: "3c8d70470a5d97e5a2b9410ddd899cb740116591462626c60cb2fead6448f60b",
};

/// ZAYA1-8B community checkpoint. Zyphra publishes only BF16 safetensors
/// (~17.7 GiB), so the only loadable quantization is the community GGUF.
/// Pinned to the exact revision, byte size, and LFS sha256 verified through
/// the Hugging Face API on 2026-09-12. It was produced with llama.cpp draft
/// PR #23112 and loads only in a runtime built from that branch; the load
/// path refuses it for the managed runtime (see commands::load_model and
/// docs/ZAYA1-FREETOKEN-PLAN.md).
pub const ZAYA1_FILENAME: &str = "ZAYA1-8B-Q4_K_M.gguf";
pub const ZAYA1: Asset<'static> = Asset {
    url: "https://huggingface.co/Abiray/ZAYA1-8B-GGUF/resolve/e16067cfd1f73cc688ec4004573f33de76aa88bf/ZAYA1-8B-Q4_K_M.gguf",
    bytes: 5_567_581_549,
    sha256: "330ad2b15a6dabc9d955e7f10f4f7ee220180f06ee3bee4d06062218966f2c74",
};

/// Relative path from the project root to the pre-built ZAYA runtime from
/// llama.cpp PR #23112 (the only implementation that supports the `zaya`
/// architecture). Used by auto-configuration to locate the custom runtime.
pub const ZAYA1_RUNTIME_RELATIVE: &str =
    ".local/llama.cpp-3750f9ce7ac20f7a905b43d9f20ad1050884f6c7/build/bin/Release/llama-server.exe";
/// Development paths for the two model-specific runtimes used by the pinned
/// catalog. Packaged installs use the app-managed b10855 directory for the
/// standard runtime and require the user to select a Prism build for Bonsai.
pub const STANDARD_RUNTIME_RELATIVE: &str = ".local/runtime/llama-server.exe";
pub const BONSAI_RUNTIME_RELATIVE: &str = ".local/runtime-prism-b9601-68faa14/llama-server.exe";

pub fn model(filename: Option<&str>) -> Result<(&'static str, Asset<'static>), String> {
    match filename.unwrap_or(MODEL_FILENAME) {
        MODEL_FILENAME => Ok((MODEL_FILENAME, MODEL)),
        BONSAI_FILENAME => Ok((BONSAI_FILENAME, BONSAI)),
        ZAYA1_FILENAME => Ok((ZAYA1_FILENAME, ZAYA1)),
        _ => Err("Choose a model from the managed catalog.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn catalog_preserves_default_and_pins_exact_bonsai_quantization() {
        assert_eq!(model(None).unwrap().0, MODEL_FILENAME);
        let (filename, asset) = model(Some(BONSAI_FILENAME)).unwrap();
        assert_eq!(filename, "Ternary-Bonsai-8B-Q2_0.gguf");
        assert_eq!(asset.bytes, 2_182_184_672);
        assert_eq!(asset.sha256.len(), 64);
        assert!(model(Some("../../other.gguf")).is_err());
    }
    #[test]
    fn catalog_includes_zaya1_reference_entry() {
        let (filename, asset) = model(Some(ZAYA1_FILENAME)).unwrap();
        assert_eq!(filename, "ZAYA1-8B-Q4_K_M.gguf");
        assert_eq!(asset.bytes, 5_567_581_549);
        assert_eq!(asset.sha256, "330ad2b15a6dabc9d955e7f10f4f7ee220180f06ee3bee4d06062218966f2c74");
        assert!(asset.url.contains("/resolve/e16067cfd1f73cc688ec4004573f33de76aa88bf/"), "the revision must stay pinned, not /main/");
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
