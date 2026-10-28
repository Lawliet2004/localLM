//! Startup-only cleanup of the installer's bounded, recognizable staging layout.
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

/// Model partials are durable recovery data. Only explicit model deletion may
/// remove them; unknown/orphan files are retained for manual recovery.
pub fn clean(root: &Path, runtime: bool) -> Result<usize, String> {
    if !root.exists() {
        return Ok(0);
    }
    let directory = Dir::open_ambient_dir(root, cap_std::ambient_authority())
        .map_err(|error| error.to_string())?;
    let mut removed = 0;
    for name in entries(&directory)? {
        if !runtime {
            continue;
        }
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
        let staged_entries = entries(&staging)?;
        let assets: Vec<_> = crate::runtime_catalog::packages()?
            .into_iter()
            .flat_map(|package| package.assets)
            .collect();
        let prism_archive = assets.iter().any(|asset| {
            asset.files.is_empty() && staged_entries.iter().any(|name| name == &asset.name)
        });
        let mut files = Vec::new();
        let mut has_bundle = false;
        // Validate the entire small layout before removing anything; never recurse through links.
        for entry in staged_entries {
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
                            && !(prism_archive && crate::runtime_archive::safe_leaf(&file))
                            && file != "verified-files.json"
                    {
                        return Err("Unexpected runtime staging content; files retained.".into());
                    }
                    files.push(format!("bundle/{file}"));
                }
                has_bundle = true;
            } else if regular(&staging, &entry)
                && (assets
                    .iter()
                    .any(|asset| asset.name == entry || format!("{}.part", asset.name) == entry)
                    || owned(&entry, ".locallm-download-", ".part"))
            {
                files.push(entry);
            } else {
                return Err("Unexpected runtime staging content; files retained.".into());
            }
        }
        for file in files {
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
        removed += 1;
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recognizes_runtime_archive_partial_but_refuses_links() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp
            .path()
            .join(format!(".locallm-runtime-{}", "a".repeat(32)));
        std::fs::create_dir(&dir).unwrap();
        let name = format!(
            "{}.part",
            crate::runtime_archive::catalog().unwrap()[0].name
        );
        std::fs::write(dir.join(&name), b"prefix").unwrap();
        assert_eq!(clean(temp.path(), true).unwrap(), 1);
        assert!(!dir.exists());
        #[cfg(unix)]
        {
            std::fs::create_dir(&dir).unwrap();
            let outside = tempfile::NamedTempFile::new().unwrap();
            std::os::unix::fs::symlink(outside.path(), dir.join(name)).unwrap();
            assert!(clean(temp.path(), true).is_err());
            assert!(outside.path().exists());
        }
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
        assert!(clean(temp.path(), true).is_err());
        assert_eq!(
            std::fs::read(outside.path().join("llama-server.exe")).unwrap(),
            b"preserved"
        );
    }
    /// The downloader writes `<name>.part`; the collector must reclaim those
    /// or an interrupted multi-gigabyte download is never reclaimed.
    #[test]
    fn preserves_model_partials_across_restart() {
        let temp = tempfile::tempdir().unwrap();
        let digest = "a".repeat(64);
        let model_dir = temp.path().join(&digest);
        std::fs::create_dir_all(&model_dir).unwrap();
        std::fs::write(model_dir.join("model.gguf.part"), b"partial").unwrap();
        std::fs::write(model_dir.join("model.gguf"), b"keep").unwrap();
        std::fs::write(temp.path().join("catalog-model.gguf.part"), b"partial").unwrap();
        std::fs::write(temp.path().join("notes.txt"), b"keep").unwrap();

        assert_eq!(clean(temp.path(), false).unwrap(), 0);
        assert!(model_dir.join("model.gguf.part").exists());
        assert!(temp.path().join("catalog-model.gguf.part").exists());
        assert_eq!(
            std::fs::read(model_dir.join("model.gguf")).unwrap(),
            b"keep"
        );
        assert_eq!(
            std::fs::read(temp.path().join("notes.txt")).unwrap(),
            b"keep"
        );
    }
    #[test]
    fn removes_only_recognized_partial_layouts() {
        let temp = tempfile::tempdir().unwrap();
        let partial = format!(".locallm-download-{}.part", "a".repeat(32));
        std::fs::write(temp.path().join(&partial), b"partial").unwrap();
        std::fs::write(temp.path().join("model.gguf"), b"keep").unwrap();
        assert_eq!(clean(temp.path(), false).unwrap(), 0);
        assert_eq!(
            std::fs::read(temp.path().join("model.gguf")).unwrap(),
            b"keep"
        );
        let staging = temp
            .path()
            .join(format!(".locallm-runtime-{}", "b".repeat(32)));
        std::fs::create_dir_all(staging.join("bundle")).unwrap();
        std::fs::write(staging.join("bundle/llama-server.exe"), b"partial").unwrap();
        std::fs::write(staging.join(&partial), b"partial").unwrap();
        assert_eq!(clean(temp.path(), true).unwrap(), 1);
        assert!(!staging.exists());
        std::fs::create_dir(&staging).unwrap();
        std::fs::write(staging.join("unknown.txt"), b"keep").unwrap();
        assert!(clean(temp.path(), true).is_err());
        assert_eq!(std::fs::read(staging.join("unknown.txt")).unwrap(), b"keep");
    }
}
