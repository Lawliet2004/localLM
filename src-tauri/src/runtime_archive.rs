//! Extract pinned runtime archives only into a private staging directory.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    io::{Read, Seek, Write},
    path::Path,
};

#[derive(Deserialize)]
pub struct RuntimeAsset {
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
    pub files: Vec<RuntimeFile>,
}
#[derive(Deserialize)]
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
    let mut archive = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
    if archive.len() != asset.files.len() {
        return Err("Runtime archive file count mismatch.".into());
    }
    let mut seen = HashSet::new();
    for index in 0..archive.len() {
        check_cancel()?;
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        let expected = asset
            .files
            .iter()
            .find(|item| item.name == entry.name())
            .ok_or("Unexpected runtime archive entry.")?;
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

#[cfg(test)]
mod tests {
    use super::*;
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
