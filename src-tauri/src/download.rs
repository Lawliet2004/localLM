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

/// Bound connection and idle time, rather than the total time for a multi-GB model.
/// Retries always start from the original URL to refresh expiring CDN redirects.
pub fn client_builder() -> reqwest::ClientBuilder {
    crate::http_client::builder(crate::http_client::Policy::unbounded())
}

pub fn client() -> Result<reqwest::Client, String> {
    client_builder()
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

#[cfg(unix)]
pub fn available_space(directory: &Path) -> Result<u64, String> {
    let stats = rustix::fs::statvfs(directory)
        .map_err(|error| format!("Could not check available disk space: {error}"))?;
    // Available blocks exclude storage reserved for root. Fragment size is the
    // unit used by statvfs block counts, including on non-default mounts.
    stats
        .f_bavail
        .checked_mul(stats.f_frsize)
        .ok_or_else(|| "Available disk space exceeds supported storage limits.".into())
}

#[cfg(not(any(windows, unix)))]
pub fn available_space(_: &Path) -> Result<u64, String> {
    Err("Disk-space checks are unavailable on this platform.".into())
}

pub fn partial_path(destination: &Path) -> PathBuf {
    let mut name = destination
        .file_name()
        .map(|name| name.to_os_string())
        .unwrap_or_default();
    name.push(".part");
    destination.with_file_name(name)
}

#[derive(Clone, Copy)]
struct TransferPolicy {
    header_timeout: Duration,
    idle_timeout: Duration,
    retry_delay: Duration,
    max_delay: Duration,
    max_retries: u32,
}

impl Default for TransferPolicy {
    fn default() -> Self {
        Self {
            header_timeout: Duration::from_secs(60),
            idle_timeout: Duration::from_secs(60),
            retry_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            max_retries: 8,
        }
    }
}

enum TransferError {
    Retry(String, Option<Duration>),
    Fatal(String),
}

impl From<String> for TransferError {
    fn from(message: String) -> Self {
        Self::Fatal(message)
    }
}

impl From<&str> for TransferError {
    fn from(message: &str) -> Self {
        Self::Fatal(message.into())
    }
}

/// Reject incorrect ranges before any saved bytes can be overwritten or appended to.
fn validate_response(response: &reqwest::Response, have: u64, bytes: u64) -> Result<(), String> {
    let expected_length = if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        let range = response
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("bytes "))
            .and_then(|value| value.split_once('/'))
            .and_then(|(range, total)| {
                let (start, end) = range.split_once('-')?;
                Some((
                    start.parse::<u64>().ok()?,
                    end.parse::<u64>().ok()?,
                    total.parse::<u64>().ok()?,
                ))
            });
        match range {
            Some((start, end, total))
                if start == have && total == bytes && end >= start && end < total =>
            {
                end - start + 1
            }
            _ => return Err(
                "Download service returned an invalid Content-Range. Saved bytes were preserved."
                    .into(),
            ),
        }
    } else {
        bytes
    };
    if response
        .content_length()
        .is_some_and(|length| length != expected_length)
    {
        return Err(
            "Download size differs from the pinned asset. Saved bytes were preserved.".into(),
        );
    }
    Ok(())
}

/// The caller supplies a pinned asset and an application-owned destination.
/// Transient failures reconnect automatically, keeping the file and hash open so
/// resuming a large download does not repeatedly hash all previously saved bytes.
pub async fn fetch(
    client: &reqwest::Client,
    asset: &Asset<'_>,
    destination: &Path,
    cancel: watch::Receiver<bool>,
    progress: impl FnMut(u64),
) -> Result<(), String> {
    if asset.sha256.bytes().all(|b| b == b'0') {
        return Err("This model entry has no pinned checksum yet. Its revision and sha256 must be verified before it can be downloaded.".into());
    }
    fetch_with_policy(
        client,
        asset,
        destination,
        cancel,
        progress,
        TransferPolicy::default(),
    )
    .await
}

async fn fetch_with_policy(
    client: &reqwest::Client,
    asset: &Asset<'_>,
    destination: &Path,
    mut cancel: watch::Receiver<bool>,
    mut progress: impl FnMut(u64),
    policy: TransferPolicy,
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
        tokio::fs::remove_file(&partial)
            .await
            .map_err(|error| error.to_string())?;
        have = 0;
    }
    let needed = required_space(asset.bytes.saturating_sub(have).max(1))?;
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
        let mut failures = 0_u32;
        let mut high_water = have;
        while have < asset.bytes {
            let attempt = async {
                let mut request = client.get(asset.url).header(reqwest::header::ACCEPT_ENCODING, "identity");
                if have > 0 {
                    request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
                }
                let response = tokio::time::timeout(policy.header_timeout, request.send()).await
                    .map_err(|_| TransferError::Retry("Download service did not send response headers in time.".into(), None))?
                    .map_err(|_| TransferError::Retry("Could not connect to the download service.".into(), None))?;
                let status = response.status();
                if status == reqwest::StatusCode::REQUEST_TIMEOUT
                    || status == reqwest::StatusCode::TOO_MANY_REQUESTS
                    || status.is_server_error()
                {
                    let retry_after = response.headers().get(reqwest::header::RETRY_AFTER)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.parse::<u64>().ok()).map(Duration::from_secs);
                    return Err(TransferError::Retry(format!("Download service returned HTTP {}.", status.as_u16()), retry_after));
                }
                if status != reqwest::StatusCode::OK && status != reqwest::StatusCode::PARTIAL_CONTENT {
                    return Err(TransferError::Fatal(format!("Download service returned HTTP {}.", status.as_u16())));
                }
                validate_response(&response, have, asset.bytes)?;
                if have > 0 && status == reqwest::StatusCode::OK {
                    // The service ignored Range. Validate its response and storage
                    // before discarding the saved prefix and starting again.
                    let needed = required_space(asset.bytes - have)?;
                    if available_space(parent)? < needed {
                        return Err("Not enough disk space to restart this download. Saved bytes were preserved.".into());
                    }
                    file.set_len(0).await.map_err(|error| error.to_string())?;
                    file.seek(std::io::SeekFrom::Start(0)).await.map_err(|error| error.to_string())?;
                    have = 0;
                    hash = Sha256::new();
                }
                progress(have);
                let mut stream = response.bytes_stream();
                loop {
                    let chunk = tokio::time::timeout(policy.idle_timeout, stream.next()).await
                        .map_err(|_| TransferError::Retry("Download stalled while waiting for data.".into(), None))?;
                    let Some(chunk) = chunk else { break };
                    let chunk = chunk.map_err(|_| TransferError::Retry("Download was interrupted.".into(), None))?;
                    let received = have.checked_add(chunk.len() as u64).ok_or("Download is too large.")?;
                    if received > asset.bytes {
                        return Err("Download exceeds the pinned size.".into());
                    }
                    file.write_all(&chunk).await.map_err(|error| {
                        format!("Could not write download (check free disk space): {error}")
                    })?;
                    hash.update(&chunk);
                    have = received;
                    progress(have);
                }
                if have < asset.bytes {
                    return Err(TransferError::Retry("Download ended before the pinned size was received.".into(), None));
                }
                Ok::<_, TransferError>(())
            }.await;
            match attempt {
                Ok(()) => break,
                Err(TransferError::Fatal(error)) => return Err(error),
                Err(TransferError::Retry(error, retry_after)) => {
                    // A completed body can still end with a transport error. The
                    // pinned checksum below decides whether those bytes are valid.
                    if have == asset.bytes {
                        break;
                    }
                    if have > high_water {
                        failures = 0;
                        high_water = have;
                    }
                    failures += 1;
                    if failures > policy.max_retries {
                        return Err(format!("{error} Automatic retries exhausted. Saved partial files can be resumed."));
                    }
                    file.flush()
                        .await
                        .map_err(|error| format!("Could not save partial download: {error}"))?;
                    let backoff = policy
                        .retry_delay
                        .saturating_mul(1_u32 << (failures - 1).min(16))
                        .min(policy.max_delay);
                    // Respect server throttling hints while bounding untrusted values.
                    let delay = retry_after
                        .map(|delay| delay.min(Duration::from_secs(300)))
                        .unwrap_or(backoff);
                    tokio::time::sleep(delay).await;
                }
            }
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
    install_verified(&partial, destination)?;
    Ok(())
}

/// Publish a verified download under its final name without ever replacing an
/// existing file.
///
/// `rename(2)` silently replaces the destination on Unix, which would break
/// the "an existing destination is preserved" guarantee between the existence
/// check and the rename. A hard link creates the destination atomically and
/// fails with `AlreadyExists` when the name is taken, so the verified bytes
/// are published exactly once. The partial file is only unlinked after the
/// link succeeded.
fn install_verified(partial: &Path, destination: &Path) -> Result<(), String> {
    match std::fs::hard_link(partial, destination) {
        Ok(()) => {
            let _ = std::fs::remove_file(partial);
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(partial);
            Err("Could not install verified file; an existing destination is preserved.".into())
        }
        Err(_error) => {
            // Some volumes (FAT/exFAT, some network shares) cannot create hard
            // links. Fall back to a rename there, but only when the
            // destination is still absent.
            if destination.exists() {
                let _ = std::fs::remove_file(partial);
                return Err(
                    "Could not install verified file; an existing destination is preserved.".into(),
                );
            }
            std::fs::rename(partial, destination).map_err(|error| {
                format!(
                    "Could not install verified file; an existing destination is preserved: {error}"
                )
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[test]
    fn installing_a_verified_file_never_replaces_an_existing_destination() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("model.gguf");
        let partial = partial_path(&destination);
        std::fs::write(&partial, b"verified").unwrap();
        std::fs::write(&destination, b"existing").unwrap();
        let error = install_verified(&partial, &destination).unwrap_err();
        assert!(error.contains("existing destination is preserved"), "{error}");
        // The installed file is untouched: the verified bytes never replaced
        // it, and the partial download is discarded.
        assert_eq!(std::fs::read(&destination).unwrap(), b"existing");
        assert!(!partial.exists());
    }

    #[test]
    fn the_install_is_atomic_and_needs_no_prior_existence_check() {
        // The publish step is a create-if-absent link: it refuses a name that
        // is already taken, whatever the caller checked before. A rename
        // would have silently replaced the destination on Unix.
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("model.gguf");
        let partial = partial_path(&destination);
        std::fs::write(&partial, b"verified").unwrap();
        std::fs::write(&destination, b"existing").unwrap();
        let error = std::fs::hard_link(&partial, &destination).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&destination).unwrap(), b"existing");
    }

    #[test]
    fn installing_a_verified_file_publishes_it_once_and_clears_the_partial() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("model.gguf");
        let partial = partial_path(&destination);
        std::fs::write(&partial, b"verified").unwrap();
        install_verified(&partial, &destination).unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"verified");
        assert!(
            !partial.exists(),
            "the partial download must not survive the install"
        );
    }
    fn fast_policy() -> TransferPolicy {
        TransferPolicy {
            header_timeout: Duration::from_millis(100),
            idle_timeout: Duration::from_millis(100),
            retry_delay: Duration::from_millis(1),
            max_delay: Duration::from_millis(5),
            max_retries: 2,
        }
    }

    async fn read_request(socket: &mut tokio::net::TcpStream) -> String {
        let mut request = Vec::new();
        while !request.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            assert_eq!(socket.read(&mut byte).await.unwrap(), 1);
            request.push(byte[0]);
            assert!(request.len() < 16384);
        }
        String::from_utf8(request).unwrap().to_ascii_lowercase()
    }

    async fn scripted_server(
        responses: Vec<(Option<&'static str>, &'static [u8])>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for (range, response) in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_request(&mut socket).await;
                if let Some(range) = range {
                    assert!(
                        request.contains(&format!("range: bytes={range}-\r\n")),
                        "{request}"
                    );
                } else {
                    assert!(!request.contains("range:"), "{request}");
                }
                socket.write_all(response).await.unwrap();
            }
        });
        (url, server)
    }

    async fn finish_server(server: tokio::task::JoinHandle<()>) {
        tokio::time::timeout(Duration::from_secs(5), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn repeated_disconnects_resume_without_manual_retry_and_reset_retry_budget() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("model.gguf");
        std::fs::write(partial_path(&destination), b"f").unwrap();
        let (url, server) = scripted_server(vec![
            (Some("1"), b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
            (Some("1"), b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 1-6/7\r\nContent-Length: 6\r\nConnection: close\r\n\r\ni"),
            (Some("2"), b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 2-6/7\r\nContent-Length: 5\r\nConnection: close\r\n\r\nxt"),
            (Some("4"), b"HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
            (Some("4"), b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 4-6/7\r\nContent-Length: 3\r\nConnection: close\r\n\r\nure"),
        ]).await;
        let hash = format!("{:x}", Sha256::digest(b"fixture"));
        let asset = Asset {
            url: &url,
            bytes: 7,
            sha256: &hash,
        };
        let (_sender, receiver) = watch::channel(false);
        let mut updates = Vec::new();
        fetch_with_policy(
            &reqwest::Client::new(),
            &asset,
            &destination,
            receiver,
            |n| updates.push(n),
            fast_policy(),
        )
        .await
        .unwrap();
        finish_server(server).await;
        assert_eq!(std::fs::read(&destination).unwrap(), b"fixture");
        assert!(!partial_path(&destination).exists());
        assert!(
            updates.windows(2).all(|pair| pair[0] <= pair[1]),
            "{updates:?}"
        );
    }

    #[tokio::test]
    async fn invalid_resume_headers_and_permanent_http_errors_preserve_saved_bytes() {
        for response in [
            &b"HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nConnection: close\r\n\r\nure"[..],
            &b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-2/7\r\nContent-Length: 3\r\nConnection: close\r\n\r\nfix"[..],
            &b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 4-6/8\r\nContent-Length: 3\r\nConnection: close\r\n\r\nure"[..],
            &b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 4-6/7\r\nContent-Length: 2\r\nConnection: close\r\n\r\nur"[..],
            &b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nno"[..],
            &b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"[..],
            &b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"[..],
        ] {
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("model.gguf");
            std::fs::write(partial_path(&destination), b"fixt").unwrap();
            let (url, server) = scripted_server(vec![(Some("4"), response)]).await;
            let hash = format!("{:x}", Sha256::digest(b"fixture"));
            let asset = Asset { url: &url, bytes: 7, sha256: &hash };
            let (_sender, receiver) = watch::channel(false);
            let error = fetch_with_policy(&reqwest::Client::new(), &asset, &destination,
                receiver, |_| {}, fast_policy()).await.unwrap_err();
            finish_server(server).await;
            assert!(!error.contains("Automatic retries exhausted"), "{error}");
            assert_eq!(std::fs::read(partial_path(&destination)).unwrap(), b"fixt");
            assert!(!destination.exists());
        }
    }

    #[tokio::test]
    async fn ignored_range_restarts_and_verifies_the_full_response() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("model.gguf");
        std::fs::write(partial_path(&destination), b"fixt").unwrap();
        let (url, server) = scripted_server(vec![(
            Some("4"),
            b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nfixture",
        )])
        .await;
        let hash = format!("{:x}", Sha256::digest(b"fixture"));
        let asset = Asset {
            url: &url,
            bytes: 7,
            sha256: &hash,
        };
        let (_sender, receiver) = watch::channel(false);
        fetch_with_policy(
            &reqwest::Client::new(),
            &asset,
            &destination,
            receiver,
            |_| {},
            fast_policy(),
        )
        .await
        .unwrap();
        finish_server(server).await;
        assert_eq!(std::fs::read(&destination).unwrap(), b"fixture");
    }

    #[tokio::test]
    async fn exhausted_retries_keep_partial_bytes_for_a_later_resume() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("model.gguf");
        std::fs::write(partial_path(&destination), b"fixt").unwrap();
        let (url, server) = scripted_server(vec![
            (Some("4"), &b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"[..]); 3
        ]).await;
        let hash = format!("{:x}", Sha256::digest(b"fixture"));
        let asset = Asset {
            url: &url,
            bytes: 7,
            sha256: &hash,
        };
        let (_sender, receiver) = watch::channel(false);
        let error = fetch_with_policy(
            &reqwest::Client::new(),
            &asset,
            &destination,
            receiver,
            |_| {},
            fast_policy(),
        )
        .await
        .unwrap_err();
        finish_server(server).await;
        assert!(error.contains("Automatic retries exhausted"), "{error}");
        assert_eq!(std::fs::read(partial_path(&destination)).unwrap(), b"fixt");
        assert!(!destination.exists());
    }

    #[tokio::test]
    async fn cancellation_interrupts_retry_backoff_immediately() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("model.gguf");
        std::fs::write(partial_path(&destination), b"fixt").unwrap();
        let (url, server) = scripted_server(vec![(Some("4"), b"HTTP/1.1 503 Service Unavailable\r\nRetry-After: 30\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")]).await;
        let (sender, receiver) = watch::channel(false);
        let cancel = tokio::spawn(async move {
            finish_server(server).await;
            tokio::time::sleep(Duration::from_millis(20)).await;
            sender.send(true).unwrap();
        });
        let hash = format!("{:x}", Sha256::digest(b"fixture"));
        let asset = Asset {
            url: &url,
            bytes: 7,
            sha256: &hash,
        };
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            fetch_with_policy(
                &reqwest::Client::new(),
                &asset,
                &destination,
                receiver,
                |_| {},
                TransferPolicy {
                    max_delay: Duration::from_secs(30),
                    ..fast_policy()
                },
            ),
        )
        .await
        .unwrap();
        assert_eq!(result.unwrap_err(), "Download cancelled.");
        cancel.await.unwrap();
        assert_eq!(std::fs::read(partial_path(&destination)).unwrap(), b"fixt");
    }

    #[tokio::test]
    async fn stalled_headers_and_body_reconnect_from_saved_offset() {
        for headers in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("model.gguf");
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/asset", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut stalled, _) = listener.accept().await.unwrap();
                let _ = read_request(&mut stalled).await;
                if !headers {
                    stalled
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\nfixt")
                        .await
                        .unwrap();
                }
                // Retain the first socket while accepting the automatic reconnect.
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = read_request(&mut socket).await;
                if headers {
                    assert!(!request.contains("range:"), "{request}");
                    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nfixture").await.unwrap();
                } else {
                    assert!(request.contains("range: bytes=4-"), "{request}");
                    socket.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 4-6/7\r\nContent-Length: 3\r\nConnection: close\r\n\r\nure").await.unwrap();
                }
            });
            let hash = format!("{:x}", Sha256::digest(b"fixture"));
            let asset = Asset {
                url: &url,
                bytes: 7,
                sha256: &hash,
            };
            let (_sender, receiver) = watch::channel(false);
            tokio::time::timeout(
                Duration::from_secs(5),
                fetch_with_policy(
                    &reqwest::Client::new(),
                    &asset,
                    &destination,
                    receiver,
                    |_| {},
                    fast_policy(),
                ),
            )
            .await
            .unwrap()
            .unwrap();
            finish_server(server).await;
            assert_eq!(std::fs::read(&destination).unwrap(), b"fixture");
        }
    }

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
        let result = fetch_with_policy(
            &reqwest::Client::new(),
            &asset,
            &destination,
            cancel,
            |_| panic!("No response body received"),
            TransferPolicy {
                header_timeout: Duration::from_millis(50),
                max_retries: 0,
                ..TransferPolicy::default()
            },
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
        let asset = Asset {
            url: "http://unused.invalid",
            bytes: bytes.len() as u64,
            sha256: &hash,
        };
        let (_sender, receiver) = watch::channel(false);
        let mut updates = Vec::new();
        fetch(
            &reqwest::Client::new(),
            &asset,
            &destination,
            receiver,
            |n| updates.push(n),
        )
        .await
        .unwrap();
        assert!(!updates.is_empty());
        assert!(
            updates.iter().all(|&n| n == asset.bytes),
            "Rehashing must not move the download meter backwards: {updates:?}"
        );
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
        fetch(
            &reqwest::Client::new(),
            &asset,
            &destination,
            receiver,
            |_| {},
        )
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
