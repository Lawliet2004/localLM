//! Startup-only migration of the installer's bounded, recognizable staging layout.
use cap_std::fs::Dir;
use std::path::Path;

fn owned(name: &str, prefix: &str, suffix: &str) -> bool {
    name.strip_prefix(prefix)
        .and_then(|name| name.strip_suffix(suffix))
        .is_some_and(|random| {
            random.len() == 32 && random.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}
fn entries(directory: &Dir) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for entry in directory.entries().map_err(|error| error.to_string())? {
        if names.len() >= 1000 {
            return Err("Too many staging entries; automatic cleanup stopped.".into());
        }
        names.push(
            entry
                .map_err(|error| error.to_string())?
                .file_name()
                .to_string_lossy()
                .into_owned(),
        );
    }
    Ok(names)
}
fn regular(directory: &Dir, name: &str) -> bool {
    directory
        .symlink_metadata(name)
        .is_ok_and(|metadata| metadata.is_file() && !metadata.file_type().is_symlink())
}

/// Remove recognizable partial-download leftovers under the models folder.
pub fn clean(root: &Path) -> Result<usize, String> {
    if !root.exists() {
        return Ok(0);
    }
    let directory = Dir::open_ambient_dir(root, cap_std::ambient_authority())
        .map_err(|error| error.to_string())?;
    let mut removed = 0;
    for name in entries(&directory)? {
        if name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit()) {
            let metadata = directory
                .symlink_metadata(&name)
                .map_err(|e| e.to_string())?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                let model = directory.open_dir(&name).map_err(|e| e.to_string())?;
                for file in entries(&model)? {
                    if owned(&file, ".locallm-download-", ".part") && regular(&model, &file) {
                        model.remove_file(&file).map_err(|e| e.to_string())?;
                        removed += 1;
                    }
                }
            }
        }
        if owned(&name, ".locallm-download-", ".part") && regular(&directory, &name) {
            directory
                .remove_file(&name)
                .map_err(|error| error.to_string())?;
            removed += 1;
        }
    }
    Ok(removed)
}

pub struct Recovered {
    /// Bytes already held by the stable archive cache for this catalog.
    pub saved: u64,
    /// Total pinned archive bytes for this catalog.
    pub total: u64,
}

/// Migrate recognizable `.locallm-runtime-{32}` staging directories into the
/// stable `{root}/.locallm-downloads/{sha256}/` archive cache so interrupted
/// installs resume. Complete archives and resumable `.part` files move;
/// extracted `bundle/` files and legacy random partials are removed. Unknown
/// content stops the migration with nothing deleted.
pub fn recover_runtime(
    root: &Path,
    assets: &[crate::runtime_archive::RuntimeAsset],
) -> Result<Recovered, String> {
    if root.exists() {
        let directory = Dir::open_ambient_dir(root, cap_std::ambient_authority())
            .map_err(|error| error.to_string())?;
        for name in entries(&directory)? {
            if !owned(&name, ".locallm-runtime-", "") {
                continue;
            }
            let metadata = directory
                .symlink_metadata(&name)
                .map_err(|error| error.to_string())?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                continue;
            }
            let staging = directory
                .open_dir(&name)
                .map_err(|error| error.to_string())?;
            // Validate the entire small layout before touching anything; never
            // recurse through links.
            let mut moves: Vec<(String, &crate::runtime_archive::RuntimeAsset)> = Vec::new();
            let mut removals: Vec<String> = Vec::new();
            let mut has_bundle = false;
            for entry in entries(&staging)? {
                if entry == "bundle" {
                    let metadata = staging
                        .symlink_metadata(&entry)
                        .map_err(|error| error.to_string())?;
                    if !metadata.is_dir() || metadata.file_type().is_symlink() {
                        return Err("Unexpected staging link; files retained.".into());
                    }
                    let bundle = staging
                        .open_dir("bundle")
                        .map_err(|error| error.to_string())?;
                    for file in entries(&bundle)? {
                        if !regular(&bundle, &file)
                            || !assets
                                .iter()
                                .flat_map(|asset| &asset.files)
                                .any(|expected| expected.name == file)
                        {
                            return Err(
                                "Unexpected runtime staging content; files retained.".into()
                            );
                        }
                        removals.push(format!("bundle/{file}"));
                    }
                    has_bundle = true;
                    continue;
                }
                if !regular(&staging, &entry) {
                    return Err("Unexpected runtime staging content; files retained.".into());
                }
                let length = staging
                    .metadata(&entry)
                    .map_err(|error| error.to_string())?
                    .len();
                if let Some(asset) = assets.iter().find(|asset| asset.name == entry) {
                    if length == asset.bytes {
                        moves.push((entry, asset));
                    } else {
                        removals.push(entry);
                    }
                } else if let Some(asset) = assets
                    .iter()
                    .find(|asset| entry == format!("{}.part", asset.name))
                {
                    if length <= asset.bytes {
                        moves.push((entry, asset));
                    } else {
                        removals.push(entry);
                    }
                } else if owned(&entry, ".locallm-download-", ".part") {
                    removals.push(entry);
                } else {
                    return Err("Unexpected runtime staging content; files retained.".into());
                }
            }
            let staging_path = root.join(&name);
            for (entry, asset) in moves {
                let cache = root.join(".locallm-downloads").join(&asset.sha256);
                std::fs::create_dir_all(&cache).map_err(|error| error.to_string())?;
                let source = staging_path.join(&entry);
                let target = cache.join(&entry);
                // A crash can leave the same bytes in both places; keep the
                // larger one.
                if target.exists() {
                    let kept = std::fs::metadata(&target)
                        .map(|metadata| metadata.len())
                        .unwrap_or(0);
                    let incoming = std::fs::metadata(&source)
                        .map(|metadata| metadata.len())
                        .unwrap_or(0);
                    if incoming > kept {
                        let _ = std::fs::remove_file(&target);
                        std::fs::rename(&source, &target)
                            .map_err(|error| error.to_string())?;
                    } else {
                        let _ = std::fs::remove_file(&source);
                    }
                } else {
                    std::fs::rename(&source, &target)
                        .map_err(|error| error.to_string())?;
                }
            }
            for file in removals {
                staging
                    .remove_file(&file)
                    .map_err(|error| error.to_string())?;
            }
            if has_bundle {
                staging
                    .remove_dir("bundle")
                    .map_err(|error| error.to_string())?;
            }
            drop(staging);
            directory
                .remove_dir(&name)
                .map_err(|error| error.to_string())?;
        }
    }
    let (saved, _) = crate::runtime_install::cached_bytes(root, assets);
    let total = assets.iter().map(|asset| asset.bytes).sum();
    Ok(Recovered { saved, total })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_archive::{RuntimeAsset, RuntimeFile};

    fn assets() -> Vec<RuntimeAsset> {
        vec![
            RuntimeAsset {
                name: "one.zip".into(),
                url: "unused".into(),
                sha256: "a".repeat(64),
                bytes: 7,
                files: vec![RuntimeFile {
                    name: "llama-server.exe".into(),
                    bytes: 3,
                }],
            },
            RuntimeAsset {
                name: "two.zip".into(),
                url: "unused".into(),
                sha256: "b".repeat(64),
                bytes: 10,
                files: vec![RuntimeFile {
                    name: "ggml.dll".into(),
                    bytes: 3,
                }],
            },
        ]
    }

    #[cfg(windows)]
    #[test]
    fn does_not_follow_a_bundle_junction() {
        use std::os::windows::process::CommandExt;
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("llama-server.exe"), b"preserved").unwrap();
        let staging = temp
            .path()
            .join(format!(".locallm-runtime-{}", "c".repeat(32)));
        std::fs::create_dir(&staging).unwrap();
        let output = std::process::Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command", "New-Item -ItemType Junction -Path $env:LOCALLM_RECOVERY_LINK -Target $env:LOCALLM_RECOVERY_TARGET -ErrorAction Stop | Out-Null"])
            .env("LOCALLM_RECOVERY_LINK", staging.join("bundle")).env("LOCALLM_RECOVERY_TARGET", outside.path()).creation_flags(0x08000000).output().unwrap();
        assert!(output.status.success());
        assert!(recover_runtime(temp.path(), &assets()).is_err());
        assert_eq!(
            std::fs::read(outside.path().join("llama-server.exe")).unwrap(),
            b"preserved"
        );
    }
    #[test]
    fn removes_only_recognized_partial_layouts() {
        let temp = tempfile::tempdir().unwrap();
        let partial = format!(".locallm-download-{}.part", "a".repeat(32));
        std::fs::write(temp.path().join(&partial), b"partial").unwrap();
        std::fs::write(temp.path().join("model.gguf"), b"keep").unwrap();
        assert_eq!(clean(temp.path()).unwrap(), 1);
        assert_eq!(
            std::fs::read(temp.path().join("model.gguf")).unwrap(),
            b"keep"
        );
        let staging = temp
            .path()
            .join(format!(".locallm-runtime-{}", "b".repeat(32)));
        std::fs::create_dir_all(staging.join("bundle")).unwrap();
        std::fs::write(staging.join("bundle/llama-server.exe"), b"exe").unwrap();
        std::fs::write(staging.join(&partial), b"partial").unwrap();
        let recovered = recover_runtime(temp.path(), &assets()).unwrap();
        assert_eq!(recovered.saved, 0);
        assert_eq!(recovered.total, 17);
        assert!(!staging.exists());
        std::fs::create_dir(&staging).unwrap();
        std::fs::write(staging.join("unknown.txt"), b"keep").unwrap();
        assert!(recover_runtime(temp.path(), &assets()).is_err());
        assert_eq!(std::fs::read(staging.join("unknown.txt")).unwrap(), b"keep");
    }
    #[test]
    fn migrates_recognizable_staging_into_the_stable_cache() {
        let temp = tempfile::tempdir().unwrap();
        let assets = assets();
        let staging = temp
            .path()
            .join(format!(".locallm-runtime-{}", "d".repeat(32)));
        std::fs::create_dir_all(staging.join("bundle")).unwrap();
        std::fs::write(staging.join("bundle/llama-server.exe"), b"exe").unwrap();
        std::fs::write(staging.join("bundle/ggml.dll"), b"dll").unwrap();
        // A finished archive moves whole; a partial one keeps its bytes.
        std::fs::write(staging.join("one.zip"), b"archive").unwrap();
        std::fs::write(staging.join("two.zip.part"), b"half").unwrap();
        let recovered = recover_runtime(temp.path(), &assets).unwrap();
        assert!(!staging.exists());
        let cache = temp.path().join(".locallm-downloads");
        assert_eq!(
            std::fs::read(cache.join("a".repeat(64)).join("one.zip")).unwrap(),
            b"archive"
        );
        assert_eq!(
            std::fs::read(cache.join("b".repeat(64)).join("two.zip.part")).unwrap(),
            b"half"
        );
        assert_eq!(recovered.saved, 7 + 4);
        assert_eq!(recovered.total, 17);
        // A second run is a no-op that still reports the cache.
        let recovered = recover_runtime(temp.path(), &assets).unwrap();
        assert_eq!(recovered.saved, 11);
    }
    #[test]
    fn a_larger_cached_archive_wins_over_the_staged_one() {
        let temp = tempfile::tempdir().unwrap();
        let assets = assets();
        let cache = temp
            .path()
            .join(".locallm-downloads")
            .join("b".repeat(64));
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(cache.join("two.zip.part"), b"lo").unwrap();
        let staging = temp
            .path()
            .join(format!(".locallm-runtime-{}", "e".repeat(32)));
        std::fs::create_dir(&staging).unwrap();
        std::fs::write(staging.join("two.zip.part"), b"longer").unwrap();
        recover_runtime(temp.path(), &assets).unwrap();
        assert_eq!(std::fs::read(cache.join("two.zip.part")).unwrap(), b"longer");
    }
}
