//! Complete runtime bundles are published from private staging on the same volume.
use crate::{download, runtime_archive};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::watch;

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
    let assets = runtime_archive::catalog()?;
    tokio::fs::create_dir_all(root)
        .await
        .map_err(|error| error.to_string())?;
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
    let destination = root.join(format!("llama-b10855-cuda12.4-{}", uuid::Uuid::new_v4()));
    tokio::fs::rename(&output, &destination)
        .await
        .map_err(|error| format!("Could not publish complete runtime: {error}"))?;
    Ok(destination.join("llama-server.exe"))
}

#[cfg(test)]
mod tests {
    use super::*;
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
