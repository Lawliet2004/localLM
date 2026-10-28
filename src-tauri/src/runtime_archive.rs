//! Extract pinned runtime archives only into a private staging directory.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    io::{Read, Seek, Write},
    path::Path,
};

#[derive(Clone, Deserialize)]
pub struct RuntimeAsset {
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
    /// Legacy bundles pin individual sizes as well. Prism pins the complete
    /// archive digest, then bounds flat extraction and receipts every output.
    pub files: Vec<RuntimeFile>,
}
#[derive(Clone, Deserialize)]
pub struct RuntimeFile {
    pub name: String,
    pub bytes: u64,
}
pub fn catalog() -> Result<Vec<RuntimeAsset>, String> {
    serde_json::from_str(include_str!("../../catalog/runtime-assets.json"))
        .map_err(|error| error.to_string())
}

pub fn extract(
    path: &Path,
    staging: &cap_std::fs::Dir,
    asset: &RuntimeAsset,
    cancel: &tokio::sync::watch::Receiver<bool>,
) -> Result<(), String> {
    let check_cancel = || {
        if *cancel.borrow() {
            Err("Runtime installation cancelled.".to_string())
        } else {
            Ok(())
        }
    };
    check_cancel()?;
    let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    if file.metadata().map_err(|error| error.to_string())?.len() != asset.bytes {
        return Err("Runtime archive size mismatch.".into());
    }
    let mut hash = Sha256::new();
    let mut hashed = 0_u64;
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        check_cancel()?;
        let count = file.read(&mut buffer).map_err(|error| error.to_string())?;
        if count == 0 {
            break;
        }
        hashed += count as u64;
        if hashed > asset.bytes {
            return Err("Runtime archive changed during verification.".into());
        }
        hash.update(&buffer[..count]);
    }
    if format!("{:x}", hash.finalize()) != asset.sha256 {
        return Err("Runtime archive SHA-256 mismatch.".into());
    }
    file.rewind().map_err(|error| error.to_string())?;
    if asset.name.ends_with(".tar.gz") {
        return extract_tar(file, staging, asset, cancel);
    }
    let mut archive = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    if archive.len() > 1000 || (!asset.files.is_empty() && archive.len() != asset.files.len()) {
        return Err("Runtime archive file count mismatch.".into());
    }
    let mut seen = HashSet::new();
    let mut expanded = 0u64;
    for index in 0..archive.len() {
        check_cancel()?;
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        let inferred = RuntimeFile {
            name: entry.name().to_string(),
            bytes: entry.size(),
        };
        let expected = asset
            .files
            .iter()
            .find(|item| item.name == entry.name())
            .or_else(|| asset.files.is_empty().then_some(&inferred))
            .ok_or("Unexpected runtime archive entry.")?;
        expanded = expanded
            .checked_add(expected.bytes)
            .ok_or("Runtime size overflow.")?;
        if expanded > MAX_EXPANDED_BYTES {
            return Err("Runtime archive exceeds extraction limit.".into());
        }
        if entry.is_dir()
            || entry.is_symlink()
            || entry.size() != expected.bytes
            || !seen.insert(expected.name.clone())
            || expected.name.is_empty()
            || expected.name.contains(['/', '\\', ':'])
            || expected.name.starts_with('.')
            || expected.name.ends_with(['.', ' '])
            || expected.name.chars().any(char::is_control)
        {
            return Err("Invalid runtime archive entry.".into());
        }
        let mut output = staging
            .open_with(
                &expected.name,
                cap_std::fs::OpenOptions::new().write(true).create_new(true),
            )
            .map_err(|error| error.to_string())?;
        let mut written = 0_u64;
        loop {
            check_cancel()?;
            let count = entry.read(&mut buffer).map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            written += count as u64;
            if written > expected.bytes {
                return Err("Runtime entry exceeds its pinned size.".into());
            }
            output
                .write_all(&buffer[..count])
                .map_err(|error| error.to_string())?;
        }
        if written != expected.bytes {
            return Err("Runtime entry ended early.".into());
        }
        output.sync_all().map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(crate) const MAX_EXPANDED_BYTES: u64 = 2 * 1024 * 1024 * 1024;
pub(crate) fn safe_leaf(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && !name.ends_with(['.', ' '])
        && !name.contains(['/', '\\', ':'])
        && !name.chars().any(char::is_control)
}

/// Release tarballs contain one flat version directory. Materialize library aliases
/// as ordinary files; no archive link is ever created or followed on disk.
fn extract_tar(
    file: std::fs::File,
    staging: &cap_std::fs::Dir,
    asset: &RuntimeAsset,
    cancel: &tokio::sync::watch::Receiver<bool>,
) -> Result<(), String> {
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let mut seen = HashSet::new();
    let mut links = Vec::new();
    let mut expanded = 0u64;
    for item in archive.entries().map_err(|e| e.to_string())? {
        if *cancel.borrow() {
            return Err("Runtime installation cancelled.".into());
        }
        let mut entry = item.map_err(|e| e.to_string())?;
        let raw = entry.path().map_err(|e| e.to_string())?.into_owned();
        let components: Vec<_> = raw.components().collect();
        if components.iter().any(|p| {
            !matches!(
                p,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        }) {
            return Err("Invalid runtime archive path.".into());
        }
        if entry.header().entry_type().is_dir() {
            continue;
        }
        let name = raw
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("Invalid runtime filename.")?
            .to_owned();
        if !safe_leaf(&name) || !seen.insert(name.clone()) {
            return Err("Invalid or duplicate runtime entry.".into());
        }
        if seen.len() > 1000 {
            return Err("Too many runtime archive entries.".into());
        }
        let inferred = RuntimeFile {
            name: name.clone(),
            bytes: entry.size(),
        };
        let expected = asset
            .files
            .iter()
            .find(|f| f.name == name)
            .or_else(|| asset.files.is_empty().then_some(&inferred))
            .ok_or("Unexpected runtime archive entry.")?;
        expanded = expanded
            .checked_add(expected.bytes)
            .ok_or("Runtime size overflow.")?;
        if expanded > MAX_EXPANDED_BYTES {
            return Err("Runtime archive exceeds extraction limit.".into());
        }
        if entry.header().entry_type().is_symlink() {
            let target = entry
                .link_name()
                .map_err(|e| e.to_string())?
                .ok_or("Missing runtime alias target.")?;
            let target = target
                .to_str()
                .filter(|n| safe_leaf(n))
                .ok_or("Invalid runtime alias target.")?
                .to_owned();
            links.push((name, target, expected.bytes));
            continue;
        }
        if !entry.header().entry_type().is_file() || entry.size() != expected.bytes {
            return Err("Invalid runtime archive entry type or size.".into());
        }
        let mut output = staging
            .open_with(
                &name,
                cap_std::fs::OpenOptions::new().write(true).create_new(true),
            )
            .map_err(|e| e.to_string())?;
        let mut buffer = [0; 65536];
        let mut written = 0;
        loop {
            if *cancel.borrow() {
                return Err("Runtime installation cancelled.".into());
            }
            let n = entry.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            written += n as u64;
            if written > expected.bytes {
                return Err("Runtime entry exceeds its pinned size.".into());
            }
            output.write_all(&buffer[..n]).map_err(|e| e.to_string())?;
        }
        if written != expected.bytes {
            return Err("Runtime entry ended early.".into());
        }
        output.sync_all().map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use cap_std::fs::PermissionsExt;
            output
                .set_permissions(cap_std::fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
        }
    }
    if !asset.files.is_empty() && seen.len() != asset.files.len() {
        return Err("Runtime archive file count mismatch.".into());
    }
    for _ in 0..links.len() {
        let mut remaining = Vec::new();
        for (name, target, bytes) in links {
            if *cancel.borrow() {
                return Err("Runtime installation cancelled.".into());
            }
            if !seen.contains(&target)
                || (!asset.files.is_empty()
                    && !asset
                        .files
                        .iter()
                        .any(|f| f.name == target && f.bytes == bytes))
            {
                return Err("Runtime alias does not match a pinned file.".into());
            }
            if staging.try_exists(&target).map_err(|e| e.to_string())? {
                let size = staging.metadata(&target).map_err(|e| e.to_string())?.len();
                expanded = expanded.checked_add(size).ok_or("Runtime size overflow.")?;
                if expanded > MAX_EXPANDED_BYTES {
                    return Err("Runtime aliases exceed extraction limit.".into());
                }
                staging
                    .copy(&target, staging, &name)
                    .map_err(|e| e.to_string())?;
            } else {
                remaining.push((name, target, bytes));
            }
        }
        links = remaining;
        if links.is_empty() {
            break;
        }
    }
    if !links.is_empty() {
        return Err("Unresolved runtime aliases.".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture_tar(link: &str) -> (tempfile::TempDir, std::path::PathBuf, RuntimeAsset) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runtime.tar.gz");
        let encoder = flate2::write::GzEncoder::new(
            std::fs::File::create(&path).unwrap(),
            flate2::Compression::default(),
        );
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, "release/llama-server", &b"good"[..])
            .unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_link_name(link).unwrap();
        header.set_cksum();
        builder
            .append_data(&mut header, "release/libalias.so", std::io::empty())
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let asset = RuntimeAsset {
            name: "runtime.tar.gz".into(),
            url: "https://example.com/pinned".into(),
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            bytes: bytes.len() as u64,
            files: vec![],
        };
        (temp, path, asset)
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    #[ignore = "requires LOCALLM_PRISM_ARCHIVE pointing to the pinned Linux CPU archive"]
    fn cached_prism_bundle_extracts_and_exposes_mtp_controls() {
        let package = crate::runtime_catalog::package(Some("prism-linux-cpu")).unwrap();
        let archive = std::env::var("LOCALLM_PRISM_ARCHIVE").unwrap();
        let output = tempfile::tempdir().unwrap();
        let directory =
            cap_std::fs::Dir::open_ambient_dir(output.path(), cap_std::ambient_authority())
                .unwrap();
        let (_sender, cancel) = tokio::sync::watch::channel(false);
        extract(Path::new(&archive), &directory, &package.assets[0], &cancel).unwrap();
        crate::runtime_inventory::receipt_assets(output.path(), &package.assets).unwrap();
        let help = std::process::Command::new(output.path().join("llama-server"))
            .arg("--help")
            .output()
            .unwrap();
        assert!(
            help.status.success(),
            "{}",
            String::from_utf8_lossy(&help.stderr)
        );
        let help = format!(
            "{}{}",
            String::from_utf8_lossy(&help.stdout),
            String::from_utf8_lossy(&help.stderr)
        );
        assert!(help.contains("draft-mtp"));
        assert!(help.contains("--spec-draft-n-max"));
    }

    #[test]
    fn verified_tar_materializes_aliases_without_creating_links() {
        let (_archive, path, asset) = fixture_tar("llama-server");
        let output = tempfile::tempdir().unwrap();
        let dir = cap_std::fs::Dir::open_ambient_dir(output.path(), cap_std::ambient_authority())
            .unwrap();
        let (_sender, cancel) = tokio::sync::watch::channel(false);
        extract(&path, &dir, &asset, &cancel).unwrap();
        assert_eq!(
            std::fs::read(output.path().join("libalias.so")).unwrap(),
            b"good"
        );
        assert!(
            !std::fs::symlink_metadata(output.path().join("libalias.so"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_ne!(
                std::fs::metadata(output.path().join("llama-server"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o111,
                0
            );
        }
    }
    #[test]
    fn verified_tar_still_rejects_links_outside_its_bundle() {
        let (_archive, path, asset) = fixture_tar("../outside");
        let output = tempfile::tempdir().unwrap();
        let dir = cap_std::fs::Dir::open_ambient_dir(output.path(), cap_std::ambient_authority())
            .unwrap();
        let (_sender, cancel) = tokio::sync::watch::channel(false);
        assert!(extract(&path, &dir, &asset, &cancel)
            .unwrap_err()
            .contains("alias target"));
        assert!(!output.path().join("libalias.so").exists());
    }
    #[test]
    fn archive_checksum_and_cancellation_are_checked_before_extraction() {
        let (_archive, path, mut asset) = fixture_tar("llama-server");
        let output = tempfile::tempdir().unwrap();
        let dir = cap_std::fs::Dir::open_ambient_dir(output.path(), cap_std::ambient_authority())
            .unwrap();
        let (sender, cancel) = tokio::sync::watch::channel(false);
        asset.sha256 = "0".repeat(64);
        assert!(extract(&path, &dir, &asset, &cancel)
            .unwrap_err()
            .contains("SHA-256"));
        sender.send(true).unwrap();
        assert_eq!(
            extract(&path, &dir, &asset, &cancel).unwrap_err(),
            "Runtime installation cancelled."
        );
        assert_eq!(std::fs::read_dir(output.path()).unwrap().count(), 0);
    }

    #[test]
    #[ignore = "requires the pinned archives in .local/downloads and space for the expanded runtime"]
    fn extracts_cached_pinned_cuda_archives() {
        let assets = catalog().unwrap();
        let directory = tempfile::tempdir().unwrap();
        let needed = crate::runtime_install::required_bytes(&assets).unwrap();
        assert!(crate::download::available_space(directory.path()).unwrap() >= needed);
        let staging =
            cap_std::fs::Dir::open_ambient_dir(directory.path(), cap_std::ambient_authority())
                .unwrap();
        let (_sender, cancel) = tokio::sync::watch::channel(false);
        let archive_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(".local/downloads");
        for asset in &assets {
            extract(&archive_root.join(&asset.name), &staging, asset, &cancel).unwrap();
            for file in &asset.files {
                assert_eq!(staging.metadata(&file.name).unwrap().len(), file.bytes);
            }
        }
        assert_eq!(
            staging.entries().unwrap().count(),
            assets.iter().map(|asset| asset.files.len()).sum::<usize>()
        );
    }
    #[test]
    fn extraction_checks_hash_manifest_and_existing_files() {
        for case in ["valid", "hash", "unexpected", "size", "existing", "cancel"] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("archive.zip");
            let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
            writer
                .start_file("llama-server.exe", zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"fixture").unwrap();
            writer.finish().unwrap();
            let bytes = std::fs::read(&path).unwrap();
            let asset = RuntimeAsset {
                name: "archive.zip".into(),
                url: "unused".into(),
                bytes: bytes.len() as u64,
                sha256: if case == "hash" {
                    "0".repeat(64)
                } else {
                    format!("{:x}", Sha256::digest(&bytes))
                },
                files: vec![RuntimeFile {
                    name: if case == "unexpected" {
                        "different.exe"
                    } else {
                        "llama-server.exe"
                    }
                    .into(),
                    bytes: if case == "size" { 8 } else { 7 },
                }],
            };
            let directory = tempfile::tempdir().unwrap();
            let staging =
                cap_std::fs::Dir::open_ambient_dir(directory.path(), cap_std::ambient_authority())
                    .unwrap();
            if case == "existing" {
                staging.write("llama-server.exe", b"preserved").unwrap();
            }
            let (_sender, cancel) = tokio::sync::watch::channel(case == "cancel");
            let result = extract(&path, &staging, &asset, &cancel);
            assert_eq!(result.is_ok(), case == "valid");
            if case == "valid" {
                assert_eq!(staging.read("llama-server.exe").unwrap(), b"fixture");
            } else if case == "existing" {
                assert_eq!(staging.read("llama-server.exe").unwrap(), b"preserved");
            } else {
                assert_eq!(staging.entries().unwrap().count(), 0);
            }
        }
        let assets = catalog().unwrap();
        assert_eq!(assets.len(), 2);
        assert!(assets
            .iter()
            .flat_map(|asset| &asset.files)
            .any(|file| file.name == "llama-server.exe"));
    }
}
