//! Complete runtime bundles are published from private staging on the same volume.
use crate::{download, runtime_archive};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::watch;

pub const MANAGED_RUNTIME_MESSAGE: &str = "The pinned CUDA installer supports Windows x64 only. For CPU, Vulkan (AMD/Intel), ROCm or macOS Metal, obtain a native llama-server from the official llama.cpp release/build instructions, then use Browse runtime in Model files. Choose a package matching your operating system, CPU architecture and backend.";

pub fn managed_runtime_supported() -> bool {
    cfg!(all(target_os = "windows", target_arch = "x86_64"))
}

pub fn required_bytes(assets: &[runtime_archive::RuntimeAsset]) -> Result<u64, String> {
    let total = assets.iter().try_fold(0_u64, |total, asset| {
        asset.files.iter().try_fold(
            total
                .checked_add(asset.bytes)
                .ok_or("Runtime size overflow.")?,
            |total, file| {
                total
                    .checked_add(file.bytes)
                    .ok_or("Runtime size overflow.")
            },
        )
    })?;
    download::required_space(total)
}

/// The staging owner also travels into blocking extraction, so cancelling the caller
/// cannot delete its directory while that worker is still writing files.
pub async fn install(
    root: &Path,
    cancel: watch::Receiver<bool>,
    progress: impl Fn(&str, u64, u64),
) -> Result<PathBuf, String> {
    if *cancel.borrow() {
        return Err("Runtime installation cancelled.".into());
    }
    if !managed_runtime_supported() {
        return Err(MANAGED_RUNTIME_MESSAGE.into());
    }
    let assets = runtime_archive::catalog()?;
    tokio::fs::create_dir_all(root)
        .await
        .map_err(|error| error.to_string())?;
    let reuse_root = root.to_path_buf();
    progress("verifying", 0, 0);
    if let Some(existing) =
        tokio::task::spawn_blocking(move || crate::runtime_inventory::reusable(&reuse_root))
            .await
            .map_err(|_| "Runtime verification worker stopped")??
    {
        return Ok(existing);
    }
    let required = required_bytes(&assets)?;
    if download::available_space(root)? < required {
        return Err(format!("Runtime installation requires {required} free bytes for archives, expanded files and reserve."));
    }
    if *cancel.borrow() {
        return Err("Runtime installation cancelled.".into());
    }
    let staging = Arc::new(
        tempfile::Builder::new()
            .prefix(".locallm-runtime-")
            .rand_bytes(32)
            .tempdir_in(root)
            .map_err(|error| error.to_string())?,
    );
    let output = staging.path().join("bundle");
    tokio::fs::create_dir(&output)
        .await
        .map_err(|error| error.to_string())?;
    let client = download::client()?;
    for asset in assets {
        if *cancel.borrow() {
            return Err("Runtime installation cancelled.".into());
        }
        let archive = staging.path().join(&asset.name);
        progress("downloading", 0, asset.bytes);
        download::fetch(
            &client,
            &download::Asset {
                url: &asset.url,
                bytes: asset.bytes,
                sha256: &asset.sha256,
            },
            &archive,
            cancel.clone(),
            |received| progress("downloading", received, asset.bytes),
        )
        .await?;
        progress(
            "extracting",
            0,
            asset.files.iter().map(|file| file.bytes).sum(),
        );
        let worker_staging = staging.clone();
        let worker_cancel = cancel.clone();
        tokio::task::spawn_blocking(move || {
            let directory = cap_std::fs::Dir::open_ambient_dir(
                worker_staging.path().join("bundle"),
                cap_std::ambient_authority(),
            )
            .map_err(|error| error.to_string())?;
            runtime_archive::extract(&archive, &directory, &asset, &worker_cancel)
        })
        .await
        .map_err(|_| "Runtime extraction worker stopped.")??;
    }
    if *cancel.borrow() {
        return Err("Runtime installation cancelled.".into());
    }
    let executable = output.join("llama-server.exe");
    if !tokio::fs::try_exists(&executable)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("Runtime bundle is missing llama-server.exe.".into());
    }
    let receipt_root = output.clone();
    tokio::task::spawn_blocking(move || crate::runtime_inventory::receipt(&receipt_root))
        .await
        .map_err(|_| "Runtime receipt worker stopped")??;
    let destination = root.join(format!("llama-b10855-cuda12.4-{}", uuid::Uuid::new_v4()));
    tokio::fs::rename(&output, &destination)
        .await
        .map_err(|error| format!("Could not publish complete runtime: {error}"))?;
    Ok(destination.join("llama-server.exe"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(not(windows))]
    #[tokio::test]
    async fn unsupported_runtime_install_does_not_download_windows_binaries() {
        let root = tempfile::tempdir().unwrap();
        let (_sender, cancel) = watch::channel(false);
        let error = install(root.path(), cancel, |_, _, _| {
            panic!("No Windows runtime download expected")
        })
        .await
        .unwrap_err();
        assert_eq!(error, MANAGED_RUNTIME_MESSAGE);
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
    #[tokio::test]
    async fn cancellation_before_download_leaves_no_installation() {
        let root = tempfile::tempdir().unwrap();
        let (_sender, cancel) = watch::channel(true);
        let result = install(root.path(), cancel, |_, _, _| {
            panic!("No transfer expected")
        })
        .await;
        assert_eq!(result.unwrap_err(), "Runtime installation cancelled.");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        let assets = runtime_archive::catalog().unwrap();
        let archives: u64 = assets.iter().map(|asset| asset.bytes).sum();
        let expanded: u64 = assets
            .iter()
            .flat_map(|asset| &asset.files)
            .map(|file| file.bytes)
            .sum();
        assert_eq!(
            required_bytes(&assets).unwrap(),
            archives + expanded + 268435456
        );
    }
}
