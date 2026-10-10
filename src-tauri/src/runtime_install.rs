//! Complete runtime bundles are published from private staging on the same volume.
//! Archives live in a stable `{root}/.locallm-downloads/{sha256}/` cache so an
//! interrupted job resumes instead of restarting.
use crate::{download, model_install::Installer, runtime_archive};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::watch;

pub(crate) const STANDARD_PREFIX: &str = "llama-b10855-cuda12.4";

/// The stable archive cache location for one runtime asset.
pub(crate) fn cache_path(root: &Path, asset: &runtime_archive::RuntimeAsset) -> PathBuf {
    root.join(".locallm-downloads")
        .join(&asset.sha256)
        .join(&asset.name)
}

/// Bytes already on disk and bytes still to download across the catalog.
/// A complete cached archive counts as saved; a partial cache entry counts
/// its `.part` length.
pub(crate) fn cached_bytes(root: &Path, assets: &[runtime_archive::RuntimeAsset]) -> (u64, u64) {
    let mut saved = 0_u64;
    let mut remaining = 0_u64;
    for asset in assets {
        let path = cache_path(root, asset);
        let complete = std::fs::metadata(&path)
            .ok()
            .is_some_and(|metadata| metadata.is_file() && metadata.len() == asset.bytes);
        if complete {
            saved += asset.bytes;
        } else {
            let part = std::fs::metadata(download::partial_path(&path))
                .ok()
                .filter(|metadata| metadata.is_file())
                .map(|metadata| metadata.len().min(asset.bytes))
                .unwrap_or(0);
            saved += part;
            remaining += asset.bytes - part;
        }
    }
    (saved, remaining)
}

/// Total archive + expanded footprint including the reserve, kept for tests
/// that check the space arithmetic without a volume.
#[cfg(test)]
pub(crate) fn required_bytes(assets: &[runtime_archive::RuntimeAsset]) -> Result<u64, String> {
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
    installer: &Installer,
    cancel: watch::Receiver<bool>,
) -> Result<PathBuf, String> {
    if *cancel.borrow() {
        return Err("Runtime installation cancelled.".into());
    }
    let assets = runtime_archive::catalog()?;
    install_bundle(root, assets, STANDARD_PREFIX, installer, cancel).await
}

pub(crate) async fn install_bundle(
    root: &Path,
    assets: Vec<runtime_archive::RuntimeAsset>,
    prefix: &str,
    installer: &Installer,
    cancel: watch::Receiver<bool>,
) -> Result<PathBuf, String> {
    let client = download::client()?;
    install_bundle_with(
        root,
        &assets,
        prefix,
        installer,
        cancel,
        &client,
        &download::Options::default(),
    )
    .await
}

pub(crate) async fn install_bundle_with(
    root: &Path,
    assets: &[runtime_archive::RuntimeAsset],
    prefix: &str,
    installer: &Installer,
    cancel: watch::Receiver<bool>,
    client: &reqwest::Client,
    options: &download::Options,
) -> Result<PathBuf, String> {
    if *cancel.borrow() {
        return Err("Runtime installation cancelled.".into());
    }
    tokio::fs::create_dir_all(root)
        .await
        .map_err(|error| error.to_string())?;
    if let Some(existing) = crate::runtime_inventory::find_complete(root, prefix, assets)? {
        installer.update(|status| {
            status.notice =
                Some("This pinned runtime is already installed; reusing it.".into());
        });
        return Ok(existing);
    }
    let total: u64 = assets
        .iter()
        .try_fold(0_u64, |total, asset| {
            total.checked_add(asset.bytes).ok_or("Runtime size overflow.")
        })?;
    let expanded: u64 = assets
        .iter()
        .flat_map(|asset| &asset.files)
        .try_fold(0_u64, |total, file| {
            total.checked_add(file.bytes).ok_or("Runtime size overflow.")
        })?;
    let (saved, remaining) = cached_bytes(root, assets);
    let space = download::storage(root, remaining, expanded)?;
    if space.available < space.required {
        return Err(format!(
            "Runtime installation requires {} free bytes for archives, expanded files and reserve.",
            space.required
        ));
    }
    // Extraction space is held until publication; archives reserve their own
    // remaining bytes inside fetch.
    let _extraction = download::reserve(root, expanded)?;
    installer.update(|status| {
        status.saved = saved;
        status.received = saved;
        status.total = total;
        status.asset_count = assets.len() as u32;
    });
    let mut completed = 0_u64;
    for (index, asset) in assets.iter().enumerate() {
        if *cancel.borrow() {
            return Err("Runtime installation cancelled.".into());
        }
        let archive = cache_path(root, asset);
        tokio::fs::create_dir_all(archive.parent().unwrap_or(root))
            .await
            .map_err(|error| error.to_string())?;
        let cached = tokio::fs::metadata(&archive)
            .await
            .ok()
            .is_some_and(|metadata| metadata.is_file() && metadata.len() == asset.bytes);
        let asset_pin = download::Asset {
            url: &asset.url,
            bytes: asset.bytes,
            sha256: &asset.sha256,
        };
        installer.update(|status| {
            status.asset = Some(asset.name.clone());
            status.asset_index = index as u32 + 1;
            status.asset_total = asset.bytes;
            status.asset_received = 0;
        });
        let mut verified = false;
        if cached {
            installer.update(|status| {
                status.phase = "verifying".into();
                status.attempt = 0;
            });
            match download::verify_file(&archive, &asset_pin, cancel.clone(), |bytes| {
                installer.update(|status| status.asset_received = bytes);
            })
            .await
            {
                Ok(()) => verified = true,
                Err(error) => {
                    if *cancel.borrow() {
                        return Err(error);
                    }
                    let _ = tokio::fs::remove_file(&archive).await;
                    installer.update(|status| {
                        status.notice = Some(
                            "The cached archive failed verification; downloading it again.".into(),
                        );
                        // Its bytes no longer count as on-disk.
                        status.saved = status.saved.saturating_sub(asset.bytes);
                        status.received = completed;
                    });
                }
            }
        }
        if !verified {
            let part = tokio::fs::metadata(download::partial_path(&archive))
                .await
                .ok()
                .filter(|metadata| metadata.is_file())
                .map(|metadata| metadata.len())
                .unwrap_or(0);
            installer.update(|status| {
                status.phase = if part > 0 { "resuming" } else { "downloading" }.into();
            });
            download::fetch_with(
                client,
                &asset_pin,
                &archive,
                cancel.clone(),
                |event| installer.progress(completed, event),
                options,
            )
            .await?;
        }
        completed += asset.bytes;
        installer.update(|status| status.received = completed);
    }
    // Extraction staging is still throwaway: cancel or failure discards it,
    // while the archive cache stays for the next attempt.
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
    for (index, asset) in assets.iter().enumerate() {
        if *cancel.borrow() {
            return Err("Runtime installation cancelled.".into());
        }
        installer.update(|status| {
            status.phase = "extracting".into();
            status.asset = Some(asset.name.clone());
            status.asset_index = index as u32 + 1;
        });
        let archive = cache_path(root, asset);
        let worker_staging = staging.clone();
        let worker_cancel = cancel.clone();
        let worker_asset = asset.clone();
        tokio::task::spawn_blocking(move || {
            let directory = cap_std::fs::Dir::open_ambient_dir(
                worker_staging.path().join("bundle"),
                cap_std::ambient_authority(),
            )
            .map_err(|error| error.to_string())?;
            runtime_archive::extract(&archive, &directory, &worker_asset, &worker_cancel)
        })
        .await
        .map_err(|_| "Runtime extraction worker stopped.")??;
    }
    if *cancel.borrow() {
        return Err("Runtime installation cancelled.".into());
    }
    installer.update(|status| status.phase = "finalizing".into());
    let executable = output.join("llama-server.exe");
    if !tokio::fs::try_exists(&executable)
        .await
        .map_err(|error| error.to_string())?
    {
        return Err("Runtime bundle is missing llama-server.exe.".into());
    }
    let destination = root.join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
    tokio::fs::rename(&output, &destination)
        .await
        .map_err(|error| format!("Could not publish complete runtime: {error}"))?;
    // A published bundle makes the archive cache redundant; step 0 reuse
    // covers reinstalls. Cached bytes are removed best effort only.
    for asset in assets {
        let archive = cache_path(root, asset);
        let _ = std::fs::remove_file(&archive);
        let _ = std::fs::remove_file(download::partial_path(&archive));
        if let Some(parent) = archive.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
    let _ = std::fs::remove_dir(root.join(".locallm-downloads"));
    Ok(destination.join("llama-server.exe"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_install::Installer;

    #[tokio::test]
    async fn cached_verified_archive_publishes_a_complete_reusable_runtime() {
        use sha2::{Digest, Sha256};
        use std::io::Write;
        let root = tempfile::tempdir().unwrap();
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        for name in ["llama-server.exe", "ggml.dll"] {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(b"fixture").unwrap();
        }
        let body = zip.finish().unwrap().into_inner();
        let assets = vec![runtime_archive::RuntimeAsset {
            name: "fixture.zip".into(), url: "http://127.0.0.1:1/unreachable".into(),
            bytes: body.len() as u64, sha256: format!("{:x}", Sha256::digest(&body)),
            files: ["llama-server.exe", "ggml.dll"].into_iter().map(|name| runtime_archive::RuntimeFile {
                name: name.into(), bytes: 7,
            }).collect(),
        }];
        let cache = cache_path(root.path(), &assets[0]);
        std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
        std::fs::write(&cache, body).unwrap();
        let installer = Installer::default();
        let (_guard, cancel, _job) = installer.begin("runtime:test", "fixture").unwrap();
        let client = download::client().unwrap();
        let installed = install_bundle_with(root.path(), &assets, "llama-test", &installer, cancel.clone(), &client, &download::Options::default()).await.unwrap();
        assert_eq!(std::fs::read(&installed).unwrap(), b"fixture");
        assert_eq!(std::fs::read(installed.with_file_name("ggml.dll")).unwrap(), b"fixture");
        assert_eq!(crate::runtime_inventory::find_complete(root.path(), "llama-test", &assets).unwrap(), Some(installed.clone()));
        assert!(!cache.exists(), "published archives must release their download cache");
        let reused = install_bundle_with(root.path(), &assets, "llama-test", &installer, cancel, &client, &download::Options::default()).await.unwrap();
        assert_eq!(reused, installed);
        assert!(installer.status().notice.unwrap().contains("reusing"));
    }

    #[tokio::test]
    async fn cancellation_before_download_leaves_no_installation() {
        let root = tempfile::tempdir().unwrap();
        let (_sender, cancel) = watch::channel(true);
        let installer = Installer::default();
        let (_guard, _receiver, _job) = installer.begin("runtime:test", "runtime").unwrap();
        let result = install(root.path(), &installer, cancel).await;
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
        assert_eq!(
            download::storage(root.path(), archives, expanded)
                .unwrap()
                .required,
            archives + expanded + download::RESERVE
        );
    }
}
