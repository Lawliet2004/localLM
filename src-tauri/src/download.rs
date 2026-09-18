//! Streaming verified downloads. Final filenames are published only after verification.
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
    sync::watch,
};

pub async fn verify_file(
    path: &Path,
    asset: &Asset<'_>,
    mut cancel: watch::Receiver<bool>,
    mut progress: impl FnMut(u64),
) -> Result<(), String> {
    use tokio::io::AsyncReadExt;
    if *cancel.borrow() {
        return Err("Download cancelled.".into());
    }
    let work = async {
        let mut file = tokio::fs::File::open(path)
            .await
            .map_err(|error| error.to_string())?;
        let metadata = file.metadata().await.map_err(|error| error.to_string())?;
        if !metadata.is_file() || metadata.len() != asset.bytes {
            return Err("Existing model has an unexpected size. It was not modified.".into());
        }
        let mut hash = Sha256::new();
        let mut read = 0_u64;
        let mut buffer = vec![0; 1024 * 1024];
        loop {
            let count = file
                .read(&mut buffer)
                .await
                .map_err(|error| error.to_string())?;
            if count == 0 {
                break;
            }
            read += count as u64;
            if read > asset.bytes {
                return Err("Model changed during verification.".into());
            }
            hash.update(&buffer[..count]);
            progress(read);
        }
        if read != asset.bytes
            || !format!("{:x}", hash.finalize()).eq_ignore_ascii_case(asset.sha256)
        {
            return Err("Existing model failed SHA-256 verification. It was not modified.".into());
        }
        Ok(())
    };
    tokio::select! { biased; _ = cancel.changed() => Err("Download cancelled.".into()), result = work => result }
}

pub struct Asset<'a> {
    pub url: &'a str,
    pub bytes: u64,
    pub sha256: &'a str,
}

/// Production downloads use HTTPS across every redirect and bounded connection/transfer time.
pub fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::limited(10))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(7200))
        .build()
        .map_err(|_| "Could not initialize the download client.".into())
}

pub fn required_space(bytes: u64) -> Result<u64, String> {
    bytes
        .checked_add(256 * 1024 * 1024)
        .ok_or_else(|| "Download size exceeds supported storage limits.".into())
}

#[cfg(windows)]
pub fn available_space(directory: &Path) -> Result<u64, String> {
    use std::os::windows::ffi::OsStrExt;
    let mut wide: Vec<u16> = directory.as_os_str().encode_wide().collect();
    if wide.contains(&0) {
        return Err("Invalid download directory.".into());
    }
    wide.push(0);
    let mut available = 0;
    // The NUL-terminated directory and output storage outlive this synchronous call.
    unsafe {
        windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
            windows::core::PCWSTR(wide.as_ptr()),
            Some(&mut available),
            None,
            None,
        )
    }
    .map_err(|error| format!("Could not check available disk space: {error}"))?;
    Ok(available)
}

#[cfg(not(windows))]
pub fn available_space(_: &Path) -> Result<u64, String> {
    Err("Managed downloads currently support Windows only.".into())
}

pub fn partial_path(destination: &Path) -> PathBuf {
    let mut name = destination
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".part");
    destination.with_file_name(name)
}

/// The caller supplies a pinned asset and an application-owned destination.
/// Interrupted transfers keep `{name}.part` so a later retry can resume with HTTP Range.
pub async fn fetch(
    client: &reqwest::Client,
    asset: &Asset<'_>,
    destination: &Path,
    cancel: watch::Receiver<bool>,
    progress: impl FnMut(u64),
) -> Result<(), String> {
    if asset.sha256.bytes().all(|b| b == b'0') {
        // A placeholder checksum cannot be verified after download; refuse it
        // up front instead of wasting the transfer (plan step 2: pin the
        // revision and checksum before any large download).
        return Err("This model entry has no pinned checksum yet. Its revision and sha256 must be verified before it can be downloaded.".into());
    }
    fetch_with_header_timeout(
        client,
        asset,
        destination,
        cancel,
        progress,
        Duration::from_secs(60),
    )
    .await
}

async fn fetch_with_header_timeout(
    client: &reqwest::Client,
    asset: &Asset<'_>,
    destination: &Path,
    mut cancel: watch::Receiver<bool>,
    mut progress: impl FnMut(u64),
    header_timeout: Duration,
) -> Result<(), String> {
    if asset.bytes == 0
        || asset.sha256.len() != 64
        || !asset.sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("Invalid pinned download metadata.".into());
    }
    if *cancel.borrow() {
        return Err("Download cancelled.".into());
    }
    let parent = destination
        .parent()
        .ok_or("Download destination has no parent directory.")?;
    if destination.exists() {
        return Err(
            "Could not install verified file; an existing destination is preserved.".into(),
        );
    }
    let partial = partial_path(destination);
    let mut have = tokio::fs::metadata(&partial)
        .await
        .ok()
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if have > asset.bytes {
        let _ = tokio::fs::remove_file(&partial).await;
        have = 0;
    }
    let remaining = asset.bytes.saturating_sub(have);
    let needed = required_space(remaining.max(1))?;
    if available_space(parent)? < needed {
        return Err(format!("Not enough disk space. Download requires {needed} free bytes, including a 256 MiB reserve."));
    }
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&partial)
        .await
        .map_err(|error| format!("Cannot create download file: {error}"))?;
    let work = async {
        let mut hash = Sha256::new();
        if have > 0 {
            // These bytes are already saved. Rehashing is not network progress.
            progress(have);
            file.seek(std::io::SeekFrom::Start(0))
                .await
                .map_err(|error| error.to_string())?;
            let mut read = 0_u64;
            let mut buffer = vec![0; 1024 * 1024];
            while read < have {
                let want = std::cmp::min(buffer.len() as u64, have - read) as usize;
                let count = file
                    .read(&mut buffer[..want])
                    .await
                    .map_err(|error| error.to_string())?;
                if count == 0 {
                    return Err("Saved download was truncated. Delete it and retry.".into());
                }
                hash.update(&buffer[..count]);
                read += count as u64;
            }
        }
        if have == asset.bytes {
            if !format!("{:x}", hash.finalize()).eq_ignore_ascii_case(asset.sha256) {
                return Err("Saved download failed SHA-256 verification. It was removed.".into());
            }
            file.sync_all()
                .await
                .map_err(|error| format!("Could not flush verified download: {error}"))?;
            return Ok(());
        }
        let mut request = client
            .get(asset.url)
            .header(reqwest::header::ACCEPT_ENCODING, "identity");
        if have > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let response = tokio::time::timeout(header_timeout, request.send())
            .await
            .map_err(|_| "Download service did not send response headers within 60 seconds.")?
            .map_err(|_| "Could not connect to the download service.")?;
        let status = response.status();
        let resume_ok = have > 0 && status == reqwest::StatusCode::PARTIAL_CONTENT;
        let fresh_ok = status == reqwest::StatusCode::OK;
        if have > 0 && fresh_ok {
            file.set_len(0).await.map_err(|error| error.to_string())?;
            file.seek(std::io::SeekFrom::Start(0))
                .await
                .map_err(|error| error.to_string())?;
            have = 0;
            hash = Sha256::new();
            progress(0);
        } else if !resume_ok && !fresh_ok {
            return Err(format!(
                "Download service returned HTTP {}.",
                status.as_u16()
            ));
        }
        if have == 0
            && response
                .content_length()
                .is_some_and(|length| length != asset.bytes)
        {
            return Err("Download size differs from the pinned asset.".into());
        }
        let mut stream = response.bytes_stream();
        let mut received = have;
        progress(received);
        loop {
            let chunk = tokio::time::timeout(Duration::from_secs(60), stream.next())
                .await
                .map_err(|_| "Download stalled for 60 seconds.")?;
            let Some(chunk) = chunk else { break };
            let chunk = chunk.map_err(|_| "Download was interrupted.")?;
            received = received
                .checked_add(chunk.len() as u64)
                .ok_or("Download is too large.")?;
            if received > asset.bytes {
                return Err("Download exceeds the pinned size.".into());
            }
            file.write_all(&chunk).await.map_err(|error| {
                format!("Could not write download (check free disk space): {error}")
            })?;
            hash.update(&chunk);
            progress(received);
        }
        if received != asset.bytes {
            file.sync_all().await.ok();
            return Err("Download ended before the pinned size was received.".into());
        }
        if !format!("{:x}", hash.finalize()).eq_ignore_ascii_case(asset.sha256) {
            return Err("Download SHA-256 verification failed. File was not installed.".into());
        }
        file.sync_all()
            .await
            .map_err(|error| format!("Could not flush verified download: {error}"))?;
        Ok(())
    };
    let result = tokio::select! {
        biased;
        _ = cancel.changed() => Err("Download cancelled.".into()),
        result = work => result,
    };
    let _ = file.sync_all().await;
    drop(file);
    if let Err(error) = &result {
        let saved = std::fs::metadata(&partial)
            .ok()
            .map(|metadata| metadata.len())
            .unwrap_or(0);
        let discard = saved == 0
            || saved > asset.bytes
            || error.contains("SHA-256")
            || error.contains("truncated");
        if discard {
            let _ = std::fs::remove_file(&partial);
        }
        return result;
    }
    if destination.exists() {
        let _ = std::fs::remove_file(&partial);
        return Err(
            "Could not install verified file; an existing destination is preserved.".into(),
        );
    }
    std::fs::rename(&partial, destination).map_err(|error| {
        format!("Could not install verified file; an existing destination is preserved: {error}")
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    #[tokio::test]
    async fn stalled_headers_release_the_partial_file() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("model.gguf");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let (accepted, received) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            accepted.send(()).unwrap();
            std::future::pending::<()>().await;
        });
        let asset = Asset {
            url: &url,
            bytes: 7,
            sha256: &"0".repeat(64),
        };
        let (_sender, cancel) = watch::channel(false);
        let result = fetch_with_header_timeout(
            &reqwest::Client::new(),
            &asset,
            &destination,
            cancel,
            |_| panic!("No response body received"),
            Duration::from_secs(1),
        )
        .await;
        received.await.unwrap();
        assert!(result.unwrap_err().contains("response headers"));
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        server.abort();
    }
    #[tokio::test]
    async fn existing_file_verification_rejects_corruption_without_modifying_it() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("model");
        tokio::fs::write(&path, b"fixture").await.unwrap();
        let (_sender, receiver) = watch::channel(false);
        let hash = format!("{:x}", Sha256::digest(b"fixture"));
        let asset = Asset {
            url: "unused",
            bytes: 7,
            sha256: &hash,
        };
        let mut progress = 0;
        verify_file(&path, &asset, receiver.clone(), |bytes| progress = bytes)
            .await
            .unwrap();
        assert_eq!(progress, 7);
        tokio::fs::write(&path, b"corrupt").await.unwrap();
        assert!(verify_file(&path, &asset, receiver, |_| {})
            .await
            .unwrap_err()
            .contains("SHA-256"));
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"corrupt");
    }
    #[tokio::test]
    async fn storage_and_transport_preflight_fail_without_creating_files() {
        let temp = tempfile::tempdir().unwrap();
        assert!(available_space(temp.path()).unwrap() > 0);
        assert!(available_space(&temp.path().join("missing")).is_err());
        assert!(required_space(u64::MAX).is_err());
        let (_sender, receiver) = watch::channel(false);
        let asset = Asset {
            url: "https://example.invalid/never-requested",
            bytes: u64::MAX / 2,
            sha256: &"a".repeat(64),
        };
        let error = fetch(
            &client().unwrap(),
            &asset,
            &temp.path().join("model"),
            receiver,
            |_| panic!("no download should start"),
        )
        .await
        .unwrap_err();
        assert!(error.contains("Not enough disk space"));
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
        assert!(client()
            .unwrap()
            .get("http://127.0.0.1:1/")
            .send()
            .await
            .is_err());
    }
    #[tokio::test]
    async fn cancellation_during_transfer_keeps_partial_file() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("model.gguf");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            assert!(socket.read(&mut request).await.unwrap() > 0);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\npartial")
                .await
                .unwrap();
            std::future::pending::<()>().await;
        });
        let (sender, receiver) = watch::channel(false);
        let asset = Asset {
            url: &url,
            bytes: 100,
            sha256: &"b".repeat(64),
        };
        let result = tokio::time::timeout(
            Duration::from_secs(5),
            fetch(
                &reqwest::Client::new(),
                &asset,
                &destination,
                receiver,
                |received| {
                    if received > 0 {
                        sender.send(true).unwrap();
                    }
                },
            ),
        )
        .await
        .unwrap();
        assert_eq!(result.unwrap_err(), "Download cancelled.");
        assert!(!destination.exists());
        let partial = partial_path(&destination);
        assert!(partial.exists());
        assert!(std::fs::metadata(&partial).unwrap().len() > 0);
        server.abort();
    }

    #[tokio::test]
    async fn saved_bytes_do_not_count_as_new_download_progress() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("model.gguf");
        let bytes = vec![42; 2 * 1024 * 1024];
        std::fs::write(partial_path(&destination), &bytes).unwrap();
        let hash = format!("{:x}", Sha256::digest(&bytes));
        let asset = Asset { url: "http://unused.invalid", bytes: bytes.len() as u64, sha256: &hash };
        let (_sender, receiver) = watch::channel(false);
        let mut updates = Vec::new();
        fetch(&reqwest::Client::new(), &asset, &destination, receiver, |n| updates.push(n)).await.unwrap();
        assert!(!updates.is_empty());
        assert!(updates.iter().all(|&n| n == asset.bytes), "Rehashing must not move the download meter backwards: {updates:?}");
    }

    #[tokio::test]
    async fn interrupted_download_resumes_from_saved_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("model.gguf");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for resume in [false, true] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 8192];
                let count = socket.read(&mut request).await.unwrap();
                let header = String::from_utf8_lossy(&request[..count]).to_ascii_lowercase();
                if !resume {
                    socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nfixt")
                        .await
                        .unwrap();
                } else {
                    assert!(header.contains("range: bytes=4-"), "{header}");
                    socket
                        .write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 4-6/7\r\nContent-Length: 3\r\nConnection: close\r\n\r\nure")
                        .await
                        .unwrap();
                }
            }
        });
        let hash = format!("{:x}", Sha256::digest(b"fixture"));
        let asset = Asset {
            url: &url,
            bytes: 7,
            sha256: &hash,
        };
        let (_sender, receiver) = watch::channel(false);
        let first = fetch(&reqwest::Client::new(), &asset, &destination, receiver.clone(), |_| {}).await.unwrap_err();
        assert!(
            first.contains("before the pinned size") || first.contains("interrupted") || first.contains("stalled"),
            "{first}"
        );
        assert_eq!(std::fs::read(partial_path(&destination)).unwrap(), b"fixt");
        fetch(&reqwest::Client::new(), &asset, &destination, receiver, |_| {})
            .await
            .unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"fixture");
        assert!(!partial_path(&destination).exists());
        server.abort();
    }
    #[tokio::test]
    async fn verifies_before_publishing_and_preserves_existing_files() {
        for case in ["success", "hash", "size", "existing", "cancel"] {
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("model.gguf");
            if case == "existing" {
                std::fs::write(&destination, b"original").unwrap();
            }
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/asset", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let _ = socket.read(&mut request).await;
                socket
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nfixture",
                    )
                    .await
                    .unwrap();
            });
            let hash = if case == "hash" {
                format!("{:x}", Sha256::digest(b"different-content"))
            } else {
                format!("{:x}", Sha256::digest(b"fixture"))
            };
            let (sender, receiver) = watch::channel(case == "cancel");
            let asset = Asset {
                url: &url,
                bytes: if case == "size" { 8 } else { 7 },
                sha256: &hash,
            };
            let result = fetch(
                &reqwest::Client::new(),
                &asset,
                &destination,
                receiver,
                |_| {},
            )
            .await;
            drop(sender);
            if case == "cancel" || case == "existing" {
                server.abort();
            } else {
                let _ = tokio::time::timeout(Duration::from_secs(2), server).await;
            }
            assert_eq!(result.is_ok(), case == "success");
            if case == "success" {
                assert_eq!(std::fs::read(&destination).unwrap(), b"fixture");
            } else if case == "existing" {
                assert_eq!(std::fs::read(&destination).unwrap(), b"original");
            } else {
                assert!(!destination.exists());
            }
            assert_eq!(
                std::fs::read_dir(temp.path()).unwrap().count(),
                usize::from(destination.exists())
            );
        }
    }
    #[tokio::test]
    async fn unpinned_placeholder_checksums_are_refused_before_any_transfer() {
        let temp = tempfile::tempdir().unwrap();
        let (_sender, receiver) = watch::channel(false);
        let asset = Asset {
            url: "https://example.invalid/never-requested",
            bytes: 7,
            sha256: &"0".repeat(64),
        };
        let error = fetch(
            &client().unwrap(),
            &asset,
            &temp.path().join("model"),
            receiver,
            |_| panic!("no download should start"),
        )
        .await
        .unwrap_err();
        assert!(error.contains("no pinned checksum"));
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    }
}
