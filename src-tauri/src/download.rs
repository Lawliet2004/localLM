//! Streaming verified downloads. Final filenames are published only after verification.
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
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

pub fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::limited(10))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(7200))
        .no_proxy()
        .build()
        .map_err(|_| "Could not initialize secure downloader.".into())
}

pub struct Asset<'a> {
    pub url: &'a str,
    pub bytes: u64,
    pub sha256: &'a str,
}

/// Per-file transfer notifications. `Bytes` carries the byte count on disk
/// for this file, including a saved prefix reported once before it is
/// rehashed. `Retry` fires before each backoff sleep. `Restart` means a
/// saved prefix was discarded and the file starts over.
pub enum Event<'a> {
    Bytes(u64),
    Retry {
        attempt: u32,
        max: u32,
        delay: Duration,
        reason: &'a str,
    },
    Restart(&'a str),
}

/// Retry and timeout bounds for `fetch`.
pub struct Options {
    pub header_timeout: Duration,
    pub stall_timeout: Duration,
    /// Consecutive attempts that may fail without writing new bytes.
    pub max_attempts: u32,
    /// Absolute cap on request attempts for one file.
    pub max_total_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            header_timeout: Duration::from_secs(60),
            stall_timeout: Duration::from_secs(60),
            max_attempts: 5,
            max_total_attempts: 20,
            base_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
        }
    }
}

/// Headroom kept free on the volume while downloads and extraction run.
pub const RESERVE: u64 = 256 * 1024 * 1024;

static RESERVED: Mutex<u64> = Mutex::new(0);

fn reserved() -> u64 {
    RESERVED.lock().map(|total| *total).unwrap_or(0)
}

/// Bytes held against a volume for the lifetime of the guard.
pub struct Reservation {
    held: Mutex<u64>,
}
impl Reservation {
    /// Shrink the held amount as bytes land on disk; it never grows.
    pub fn set_remaining(&self, remaining: u64) {
        if let Ok(mut held) = self.held.lock() {
            if remaining < *held {
                let released = *held - remaining;
                *held = remaining;
                if let Ok(mut total) = RESERVED.lock() {
                    *total = total.saturating_sub(released);
                }
            }
        }
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        let held = self.held.lock().map(|held| *held).unwrap_or(0);
        if let Ok(mut total) = RESERVED.lock() {
            *total = total.saturating_sub(held);
        }
    }
}

/// Promise `bytes` of this volume's free space to the caller. Fails when the
/// request plus the fixed reserve does not fit into space that is not already
/// promised to other in-flight downloads.
pub fn reserve(directory: &Path, bytes: u64) -> Result<Reservation, String> {
    let needed = bytes
        .checked_add(RESERVE)
        .ok_or("Download size exceeds supported storage limits.")?;
    let mut total = RESERVED
        .lock()
        .map_err(|_| "Storage accounting is unavailable.")?;
    let available = available_space(directory)?.saturating_sub(*total);
    if available < needed {
        return Err(format!(
            "Not enough disk space: needs {} more bytes (including a 256 MiB reserve); {available} available.",
            needed - available
        ));
    }
    *total += bytes;
    Ok(Reservation {
        held: Mutex::new(bytes),
    })
}

pub struct Storage {
    /// Free bytes minus space promised to in-flight downloads.
    pub available: u64,
    /// Bytes that must stay available; 0 when no transfer is needed.
    pub required: u64,
}

/// The one disk-space calculation shared by info endpoints and execution.
pub fn storage(directory: &Path, download_remaining: u64, extra: u64) -> Result<Storage, String> {
    let available = available_space(directory)?.saturating_sub(reserved());
    let bytes = download_remaining
        .checked_add(extra)
        .ok_or("Download size exceeds supported storage limits.")?;
    let required = if bytes == 0 { 0 } else { required_space(bytes)? };
    Ok(Storage {
        available,
        required,
    })
}

pub fn required_space(bytes: u64) -> Result<u64, String> {
    bytes
        .checked_add(RESERVE)
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

/// `bytes start-end/total` or `bytes */total` from a Content-Range header.
/// The `*` form returns `u64::MAX` for start and end.
fn parse_content_range(value: Option<&reqwest::header::HeaderValue>) -> Option<(u64, u64, u64)> {
    let text = value?.to_str().ok()?.strip_prefix("bytes ")?;
    if let Some(total) = text.strip_prefix("*/") {
        return Some((u64::MAX, u64::MAX, total.parse().ok()?));
    }
    let (range, total) = text.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    Some((start.parse().ok()?, end.parse().ok()?, total.parse().ok()?))
}

struct Failure {
    message: String,
    retryable: bool,
}
impl Failure {
    fn retry(message: String) -> Self {
        Self {
            message,
            retryable: true,
        }
    }
    fn fatal(message: String) -> Self {
        Self {
            message,
            retryable: false,
        }
    }
}

/// Discard a saved prefix the host will not honour and stream the file again.
/// Checks that the second download still fits before truncating.
async fn restart_transfer(
    file: &mut tokio::fs::File,
    hash: &mut Sha256,
    have: &mut u64,
    parent: &Path,
    on_event: &mut impl FnMut(Event<'_>),
) -> Result<(), Failure> {
    let free = available_space(parent)
        .map_err(Failure::fatal)?
        .saturating_sub(reserved());
    let extra = have
        .checked_add(RESERVE)
        .ok_or_else(|| Failure::fatal("Download size exceeds supported storage limits.".into()))?;
    if free < extra {
        return Err(Failure::fatal(format!(
            "Not enough disk space: needs {} more bytes (including a 256 MiB reserve); {free} available.",
            extra - free
        )));
    }
    file.set_len(0)
        .await
        .map_err(|error| Failure::fatal(error.to_string()))?;
    file.seek(std::io::SeekFrom::Start(0))
        .await
        .map_err(|error| Failure::fatal(error.to_string()))?;
    *have = 0;
    *hash = Sha256::new();
    on_event(Event::Restart(
        "The download host does not support resuming; restarting this file from the beginning.",
    ));
    on_event(Event::Bytes(0));
    Ok(())
}

/// The mutable state a response body streams into.
struct Sink<'a> {
    file: &'a mut tokio::fs::File,
    hash: &'a mut Sha256,
    have: &'a mut u64,
    wrote: &'a mut bool,
}

/// Stream one response body into the partial file. `wrote` records whether
/// this attempt put new bytes on disk.
async fn stream_body(
    response: reqwest::Response,
    sink: &mut Sink<'_>,
    asset: &Asset<'_>,
    options: &Options,
    reservation: &Reservation,
    on_event: &mut impl FnMut(Event<'_>),
) -> Result<(), Failure> {
    let mut stream = response.bytes_stream();
    loop {
        let chunk = match tokio::time::timeout(options.stall_timeout, stream.next()).await {
            Err(_) => {
                return Err(Failure::retry(format!(
                    "Download stalled for {} seconds.",
                    options.stall_timeout.as_secs()
                )))
            }
            Ok(None) => break,
            Ok(Some(Err(error))) => {
                return Err(Failure::retry(format!(
                    "Download was interrupted: {}",
                    error.without_url()
                )))
            }
            Ok(Some(Ok(chunk))) => chunk,
        };
        let next = sink
            .have
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| Failure::fatal("Download is too large.".into()))?;
        if next > asset.bytes {
            return Err(Failure::fatal("Download exceeds the pinned size.".into()));
        }
        sink.file.write_all(&chunk).await.map_err(|error| {
            Failure::fatal(format!(
                "Could not write the download to disk (disk full or not writable): {error}"
            ))
        })?;
        sink.hash.update(&chunk);
        *sink.have = next;
        *sink.wrote = true;
        reservation.set_remaining(asset.bytes - next);
        on_event(Event::Bytes(next));
    }
    if *sink.have != asset.bytes {
        let _ = sink.file.sync_all().await;
        return Err(Failure::retry(
            "Download ended before the pinned size was received.".into(),
        ));
    }
    if !format!("{:x}", sink.hash.clone().finalize()).eq_ignore_ascii_case(asset.sha256) {
        return Err(Failure::fatal(
            "Download failed SHA-256 verification; the corrupt partial file was removed. Retry to download it again.".into(),
        ));
    }
    sink.file.sync_all().await.map_err(|error| {
        Failure::fatal(format!(
            "Could not write the download to disk (disk full or not writable): {error}"
        ))
    })?;
    Ok(())
}

/// The caller supplies a pinned asset and an application-owned destination.
/// Interrupted transfers keep `{name}.part` so a later retry can resume with HTTP Range.
pub async fn fetch(
    client: &reqwest::Client,
    asset: &Asset<'_>,
    destination: &Path,
    cancel: watch::Receiver<bool>,
    on_event: impl FnMut(Event<'_>),
) -> Result<(), String> {
    fetch_with(client, asset, destination, cancel, on_event, &Options::default()).await
}

pub(crate) async fn fetch_with(
    client: &reqwest::Client,
    asset: &Asset<'_>,
    destination: &Path,
    mut cancel: watch::Receiver<bool>,
    mut on_event: impl FnMut(Event<'_>),
    options: &Options,
) -> Result<(), String> {
    if asset.bytes == 0
        || asset.sha256.len() != 64
        || !asset.sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("Invalid pinned download metadata.".into());
    }
    if asset.sha256.bytes().all(|b| b == b'0') {
        // A placeholder checksum cannot be verified after download; refuse it
        // up front instead of wasting the transfer (plan step 2: pin the
        // revision and checksum before any large download).
        return Err("This model entry has no pinned checksum yet. Its revision and sha256 must be verified before it can be downloaded.".into());
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
    // Hold the remaining bytes for the whole transfer so concurrent jobs
    // cannot overcommit the volume; released chunk by chunk and on drop.
    let reservation = reserve(parent, asset.bytes - have)?;
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
            on_event(Event::Bytes(have));
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
                return Err("Download failed SHA-256 verification; the corrupt partial file was removed. Retry to download it again.".into());
            }
            file.sync_all().await.map_err(|error| {
                format!("Could not write the download to disk (disk full or not writable): {error}")
            })?;
            return Ok(());
        }
        let mut attempt = 0_u32;
        let mut idle = 0_u32;
        'attempts: loop {
            attempt += 1;
            if attempt > options.max_total_attempts {
                return Err(
                    "Download host kept failing; saved bytes were kept for a later retry.".into(),
                );
            }
            let mut wrote = false;
            // Re-request the original URL on every attempt so expiring signed
            // redirect targets are refreshed.
            let mut request = client
                .get(asset.url)
                .header(reqwest::header::ACCEPT_ENCODING, "identity");
            if have > 0 {
                request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
            }
            let mut retry_after = None;
            let failure = match tokio::time::timeout(options.header_timeout, request.send()).await
            {
                Err(_) => Failure::retry(format!(
                    "Download host did not send response headers within {} seconds.",
                    options.header_timeout.as_secs()
                )),
                Ok(Err(error)) => {
                    let transient = error.is_connect() || error.is_timeout();
                    let detail = error.without_url();
                    if transient {
                        Failure::retry(format!("Could not connect to the download host: {detail}"))
                    } else {
                        return Err(format!("Download request failed: {detail}"));
                    }
                }
                Ok(Ok(response)) => {
                    let status = response.status().as_u16();
                    match status {
                        200 => {
                            if have > 0 {
                                // The host ignored Range; a restart needs room
                                // for the bytes that must be fetched twice.
                                restart_transfer(
                                    &mut file,
                                    &mut hash,
                                    &mut have,
                                    parent,
                                    &mut on_event,
                                )
                                .await
                                .map_err(|failure| failure.message)?;
                            }
                            if response
                                .content_length()
                                .is_some_and(|length| length != asset.bytes)
                            {
                                return Err("Download size differs from the pinned asset.".into());
                            }
                            let mut sink = Sink {
                                file: &mut file,
                                hash: &mut hash,
                                have: &mut have,
                                wrote: &mut wrote,
                            };
                            match stream_body(
                                response,
                                &mut sink,
                                asset,
                                options,
                                &reservation,
                                &mut on_event,
                            )
                            .await
                            {
                                Ok(()) => break 'attempts Ok(()),
                                Err(failure) => failure,
                            }
                        }
                        206 if have > 0 => {
                            let range = parse_content_range(
                                response.headers().get(reqwest::header::CONTENT_RANGE),
                            );
                            let valid = range.is_some_and(|(start, end, total)| {
                                start == have && end + 1 == asset.bytes && total == asset.bytes
                            }) && response
                                .headers()
                                .get(reqwest::header::CONTENT_LENGTH)
                                .and_then(|value| value.to_str().ok())
                                .and_then(|value| value.parse::<u64>().ok())
                                .is_none_or(|length| length == asset.bytes - have);
                            if !valid {
                                Failure::retry(
                                    "Download host returned an invalid resume range; saved bytes were kept.".into(),
                                )
                            } else {
                                let mut sink = Sink {
                                    file: &mut file,
                                    hash: &mut hash,
                                    have: &mut have,
                                    wrote: &mut wrote,
                                };
                                match stream_body(
                                    response,
                                    &mut sink,
                                    asset,
                                    options,
                                    &reservation,
                                    &mut on_event,
                                )
                                .await
                                {
                                    Ok(()) => break 'attempts Ok(()),
                                    Err(failure) => failure,
                                }
                            }
                        }
                        416 => {
                            match parse_content_range(
                                response.headers().get(reqwest::header::CONTENT_RANGE),
                            ) {
                                Some((u64::MAX, _, total)) if total != asset.bytes => {
                                    return Err("Remote file size differs from the pinned size; saved bytes were kept.".into());
                                }
                                _ if have > 0 => {
                                    // Contradictory refusal: discard the saved
                                    // prefix and restart without a Range.
                                    restart_transfer(
                                        &mut file,
                                        &mut hash,
                                        &mut have,
                                        parent,
                                        &mut on_event,
                                    )
                                    .await
                                    .map_err(|failure| failure.message)?;
                                    continue 'attempts;
                                }
                                _ => {
                                    return Err(
                                        "Download host rejected the request (HTTP 416).".into()
                                    )
                                }
                            }
                        }
                        401 | 403 => {
                            return Err(format!(
                                "Access denied by the download host (HTTP {status}). Accept the model's license and use a read token with access."
                            ))
                        }
                        404 | 410 => {
                            return Err(format!(
                                "The pinned file is no longer available at its source (HTTP {status})."
                            ))
                        }
                        408 | 429 | 500 | 502 | 503 | 504 => {
                            retry_after = response
                                .headers()
                                .get(reqwest::header::RETRY_AFTER)
                                .and_then(|value| value.to_str().ok())
                                .and_then(|value| value.trim().parse::<u64>().ok())
                                .map(|seconds| Duration::from_secs(seconds.min(60)));
                            Failure::retry(format!("Download host returned HTTP {status}."))
                        }
                        _ => {
                            return Err(format!(
                                "Download host rejected the request (HTTP {status})."
                            ))
                        }
                    }
                }
            };
            idle = if wrote { 0 } else { idle + 1 };
            if !failure.retryable || idle >= options.max_attempts {
                return Err(failure.message);
            }
            let backoff = options
                .base_delay
                .saturating_mul(1_u32 << (attempt - 1).min(20))
                .min(options.max_delay);
            let delay = retry_after.map_or(backoff, |hint| backoff.max(hint));
            on_event(Event::Retry {
                attempt,
                max: options.max_attempts,
                delay,
                reason: &failure.message,
            });
            // The outer select drops this sleep the moment cancel fires.
            tokio::time::sleep(delay).await;
        }
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

    fn fast() -> Options {
        Options {
            header_timeout: Duration::from_secs(2),
            stall_timeout: Duration::from_millis(300),
            max_attempts: 3,
            max_total_attempts: 20,
            base_delay: Duration::from_millis(5),
            max_delay: Duration::from_millis(20),
        }
    }

    fn sha256(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    /// Read one HTTP request and answer it with the given head + body chunks.
    async fn serve_once(
        listener: &tokio::net::TcpListener,
        respond: impl FnOnce(&str) -> (String, Vec<Vec<u8>>),
    ) -> String {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 8192];
        let count = socket.read(&mut request).await.unwrap();
        let header = String::from_utf8_lossy(&request[..count]).into_owned();
        let (head, chunks) = respond(&header);
        socket.write_all(head.as_bytes()).await.unwrap();
        for chunk in chunks {
            socket.write_all(&chunk).await.unwrap();
        }
        header
    }

    #[tokio::test]
    async fn stalled_headers_release_the_partial_file() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("model.gguf");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                assert!(socket.read(&mut request).await.unwrap() > 0);
                std::future::pending::<()>().await;
            }
        });
        let asset = Asset {
            url: &url,
            bytes: 7,
            sha256: &"a".repeat(64),
        };
        let (_sender, cancel) = watch::channel(false);
        let result = fetch_with(
            &reqwest::Client::new(),
            &asset,
            &destination,
            cancel,
            |_| panic!("No response body received"),
            &Options {
                header_timeout: Duration::from_secs(1),
                max_attempts: 1,
                ..fast()
            },
        )
        .await;
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
        let hash = sha256(b"fixture");
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
                |event| {
                    if let Event::Bytes(received) = event {
                        if received > 0 {
                            let _ = sender.send(true);
                        }
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
        let hash = sha256(&bytes);
        let asset = Asset { url: "http://unused.invalid", bytes: bytes.len() as u64, sha256: &hash };
        let (_sender, receiver) = watch::channel(false);
        let mut updates = Vec::new();
        fetch(&reqwest::Client::new(), &asset, &destination, receiver, |event| {
            if let Event::Bytes(n) = event {
                updates.push(n);
            }
        })
        .await
        .unwrap();
        assert!(!updates.is_empty());
        assert!(updates.iter().all(|&n| n == asset.bytes), "Rehashing must not move the download meter backwards: {updates:?}");
    }

    #[tokio::test]
    async fn interrupted_body_retries_with_the_saved_range_in_one_call() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("model.gguf");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let server_hits = hits.clone();
        let server = tokio::spawn(async move {
            loop {
                let request = serve_once(&listener, |header| {
                    server_hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if header.to_ascii_lowercase().contains("range: bytes=4-") {
                        (
                            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 4-6/7\r\nContent-Length: 3\r\nConnection: close\r\n\r\n".into(),
                            vec![b"ure".to_vec()],
                        )
                    } else {
                        // Close the body early to force a resumable retry.
                        (
                            "HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\n".into(),
                            vec![b"fixt".to_vec()],
                        )
                    }
                })
                .await;
                assert!(!request.is_empty());
            }
        });
        let hash = sha256(b"fixture");
        let asset = Asset {
            url: &url,
            bytes: 7,
            sha256: &hash,
        };
        let (_sender, receiver) = watch::channel(false);
        let mut retries = 0;
        fetch_with(&reqwest::Client::new(), &asset, &destination, receiver, |event| {
            if let Event::Retry { .. } = event {
                retries += 1;
            }
        }, &fast())
        .await
        .unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"fixture");
        assert!(!partial_path(&destination).exists());
        assert!(retries >= 1);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2);
        server.abort();
    }

    #[tokio::test]
    async fn resumes_with_a_valid_206_range_response() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("model.gguf");
        std::fs::write(partial_path(&destination), b"fixt").unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let request = serve_once(&listener, |header| {
                assert!(header.to_ascii_lowercase().contains("range: bytes=4-"), "{header}");
                (
                    "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 4-6/7\r\nContent-Length: 3\r\nConnection: close\r\n\r\n".into(),
                    vec![b"ure".to_vec()],
                )
            })
            .await;
            assert!(request.contains("GET"));
        });
        let hash = sha256(b"fixture");
        let asset = Asset { url: &url, bytes: 7, sha256: &hash };
        let (_sender, receiver) = watch::channel(false);
        let mut events = Vec::new();
        fetch_with(&reqwest::Client::new(), &asset, &destination, receiver, |event| {
            if let Event::Bytes(n) = event {
                events.push(n);
            }
        }, &fast())
        .await
        .unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"fixture");
        assert_eq!(events[0], 4, "the saved prefix reports once before rehash");
        assert_eq!(*events.last().unwrap(), 7);
        server.abort();
    }

    #[tokio::test]
    async fn invalid_206_never_touches_the_saved_prefix() {
        for head in [
            "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 2-6/7\r\nContent-Length: 5\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\nConnection: close\r\n\r\n",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("model.gguf");
            std::fs::write(partial_path(&destination), b"fixt").unwrap();
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/asset", listener.local_addr().unwrap());
            let head = head.to_string();
            let server = tokio::spawn(async move {
                loop {
                    let answer = head.clone();
                    serve_once(&listener, |_| (answer.clone(), Vec::new())).await;
                }
            });
            let hash = sha256(b"fixture");
            let asset = Asset { url: &url, bytes: 7, sha256: &hash };
            let (_sender, receiver) = watch::channel(false);
            let error = fetch_with(&reqwest::Client::new(), &asset, &destination, receiver, |_| {}, &fast())
                .await
                .unwrap_err();
            assert!(error.contains("invalid resume range"), "{error}");
            assert_eq!(std::fs::read(partial_path(&destination)).unwrap(), b"fixt");
            server.abort();
        }
    }

    #[tokio::test]
    async fn a_200_to_a_range_request_restarts_the_file() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("model.gguf");
        std::fs::write(partial_path(&destination), b"fixt").unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            serve_once(&listener, |header| {
                assert!(header.to_ascii_lowercase().contains("range: bytes=4-"), "{header}");
                (
                    "HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\n".into(),
                    vec![b"fixture".to_vec()],
                )
            })
            .await;
        });
        let hash = sha256(b"fixture");
        let asset = Asset { url: &url, bytes: 7, sha256: &hash };
        let (_sender, receiver) = watch::channel(false);
        let mut restarted = false;
        fetch_with(&reqwest::Client::new(), &asset, &destination, receiver, |event| {
            if let Event::Restart(reason) = event {
                restarted = true;
                assert!(reason.contains("does not support resuming"));
            }
        }, &fast())
        .await
        .unwrap();
        assert!(restarted);
        assert_eq!(std::fs::read(&destination).unwrap(), b"fixture");
        server.abort();
    }

    #[tokio::test]
    async fn a_416_with_a_mismatched_total_keeps_the_partial() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("model.gguf");
        std::fs::write(partial_path(&destination), b"fixt").unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let server_hits = hits.clone();
        let server = tokio::spawn(async move {
            loop {
                serve_once(&listener, |_| {
                    server_hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    (
                        "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */99\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
                        Vec::new(),
                    )
                })
                .await;
            }
        });
        let hash = sha256(b"fixture");
        let asset = Asset { url: &url, bytes: 7, sha256: &hash };
        let (_sender, receiver) = watch::channel(false);
        let error = fetch_with(&reqwest::Client::new(), &asset, &destination, receiver, |_| {}, &fast())
            .await
            .unwrap_err();
        assert!(error.contains("differs from the pinned size"), "{error}");
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(std::fs::read(partial_path(&destination)).unwrap(), b"fixt");
        server.abort();
    }

    #[tokio::test]
    async fn server_failures_retry_and_honour_retry_after() {
        for (status, extra) in [(503, "Retry-After: 1\r\n"), (429, "")] {
            let temp = tempfile::tempdir().unwrap();
            let destination = temp.path().join("model.gguf");
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/asset", listener.local_addr().unwrap());
            let hits = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
            let server_hits = hits.clone();
            let server = tokio::spawn(async move {
                loop {
                    serve_once(&listener, |_| {
                        if server_hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                            (
                                format!("HTTP/1.1 {status} Retry\r\n{extra}Content-Length: 0\r\nConnection: close\r\n\r\n"),
                                Vec::new(),
                            )
                        } else {
                            (
                                "HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\n".into(),
                                vec![b"fixture".to_vec()],
                            )
                        }
                    })
                    .await;
                }
            });
            let hash = sha256(b"fixture");
            let asset = Asset { url: &url, bytes: 7, sha256: &hash };
            let (_sender, receiver) = watch::channel(false);
            let mut retry_delays = Vec::new();
            fetch_with(&reqwest::Client::new(), &asset, &destination, receiver, |event| {
                if let Event::Retry { delay, .. } = event {
                    retry_delays.push(delay);
                }
            }, &fast())
            .await
            .unwrap();
            assert_eq!(std::fs::read(&destination).unwrap(), b"fixture");
            assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 2);
            assert_eq!(retry_delays.len(), 1);
            if status == 503 {
                assert_eq!(retry_delays[0], Duration::from_secs(1));
            }
            server.abort();
        }
    }

    #[tokio::test]
    async fn missing_files_fail_without_a_retry() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("model.gguf");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let server_hits = hits.clone();
        let server = tokio::spawn(async move {
            loop {
                serve_once(&listener, |_| {
                    server_hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    (
                        "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
                        Vec::new(),
                    )
                })
                .await;
            }
        });
        let hash = sha256(b"fixture");
        let asset = Asset { url: &url, bytes: 7, sha256: &hash };
        let (_sender, receiver) = watch::channel(false);
        let error = fetch_with(&reqwest::Client::new(), &asset, &destination, receiver, |_| {}, &fast())
            .await
            .unwrap_err();
        assert!(error.contains("no longer available"), "{error}");
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
        server.abort();
    }

    #[tokio::test]
    async fn a_stalled_body_retries_and_resumes() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("model.gguf");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut first = true;
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 8192];
                let count = socket.read(&mut request).await.unwrap();
                let header = String::from_utf8_lossy(&request[..count]).into_owned();
                if first {
                    first = false;
                    socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\n\r\nfixt")
                        .await
                        .unwrap();
                    // Hold the connection open without finishing the body.
                    tokio::time::sleep(Duration::from_secs(2)).await;
                } else {
                    assert!(header.to_ascii_lowercase().contains("range: bytes=4-"), "{header}");
                    socket
                        .write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 4-6/7\r\nContent-Length: 3\r\nConnection: close\r\n\r\nure")
                        .await
                        .unwrap();
                }
            }
        });
        let hash = sha256(b"fixture");
        let asset = Asset { url: &url, bytes: 7, sha256: &hash };
        let (_sender, receiver) = watch::channel(false);
        let mut stalled = false;
        fetch_with(&reqwest::Client::new(), &asset, &destination, receiver, |event| {
            if let Event::Retry { reason, .. } = event {
                stalled = reason.contains("stalled");
            }
        }, &fast())
        .await
        .unwrap();
        assert!(stalled);
        assert_eq!(std::fs::read(&destination).unwrap(), b"fixture");
        server.abort();
    }

    #[tokio::test]
    async fn cancel_during_backoff_returns_immediately_and_keeps_the_partial() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("model.gguf");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            loop {
                serve_once(&listener, |_| {
                    (
                        "HTTP/1.1 503 Busy\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
                        Vec::new(),
                    )
                })
                .await;
            }
        });
        let hash = sha256(b"fixture");
        let asset = Asset { url: &url, bytes: 7, sha256: &hash };
        let (sender, receiver) = watch::channel(false);
        let start = std::time::Instant::now();
        let result = fetch_with(
            &reqwest::Client::new(),
            &asset,
            &destination,
            receiver,
            |event| {
                if let Event::Retry { .. } = event {
                    let _ = sender.send(true);
                }
            },
            // A long backoff proves the cancel preempts the sleep.
            &Options {
                base_delay: Duration::from_secs(30),
                max_delay: Duration::from_secs(30),
                ..fast()
            },
        )
        .await;
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(result.unwrap_err(), "Download cancelled.");
        assert!(!destination.exists());
        server.abort();
    }

    #[tokio::test]
    async fn a_corrupt_download_removes_the_partial_and_publishes_nothing() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("model.gguf");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/asset", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            loop {
                serve_once(&listener, |_| {
                    (
                        "HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\n".into(),
                        vec![b"corrupt".to_vec()],
                    )
                })
                .await;
            }
        });
        let hash = sha256(b"fixture");
        let asset = Asset { url: &url, bytes: 7, sha256: &hash };
        let (_sender, receiver) = watch::channel(false);
        let error = fetch_with(&reqwest::Client::new(), &asset, &destination, receiver, |_| {}, &fast())
            .await
            .unwrap_err();
        assert!(error.contains("SHA-256"), "{error}");
        assert!(error.contains("corrupt partial file was removed"), "{error}");
        assert!(!destination.exists());
        assert!(!partial_path(&destination).exists());
        server.abort();
    }

    #[tokio::test]
    async fn retries_re_request_the_original_url_after_a_redirect() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("model.gguf");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let url = format!("http://{address}/asset");
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let server_hits = hits.clone();
        let redirected = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        let server_redirected = redirected.clone();
        let server = tokio::spawn(async move {
            loop {
                server_hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                serve_once(&listener, |header| {
                    if header.starts_with("GET /asset") {
                        (
                            format!("HTTP/1.1 302 Found\r\nLocation: http://{address}/redirected\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                            Vec::new(),
                        )
                    } else if server_redirected
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                        == 0
                    {
                        // End the redirected body early so the fetch retries.
                        (
                            "HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\n".into(),
                            vec![b"fix".to_vec()],
                        )
                    } else {
                        (
                            "HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\n".into(),
                            vec![b"fixture".to_vec()],
                        )
                    }
                })
                .await;
            }
        });
        let hash = sha256(b"fixture");
        let asset = Asset { url: &url, bytes: 7, sha256: &hash };
        let (_sender, receiver) = watch::channel(false);
        fetch_with(&reqwest::Client::new(), &asset, &destination, receiver, |_| {}, &fast())
            .await
            .unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), b"fixture");
        // The retry re-requested the original /asset URL rather than the
        // (expiring) redirect target — at least once more than the first try.
        let requests = hits.load(std::sync::atomic::Ordering::SeqCst);
        let redirected_hits = redirected.load(std::sync::atomic::Ordering::SeqCst);
        assert!(requests - redirected_hits >= 2, "original URL re-requested");
        assert!(redirected_hits >= 1);
        server.abort();
    }

    #[tokio::test]
    async fn a_bearer_token_is_not_forwarded_to_a_cross_host_redirect() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("model.gguf");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let url = format!("http://127.0.0.1:{port}/asset");
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let server_seen = seen.clone();
        let server = tokio::spawn(async move {
            loop {
                let header = serve_once(&listener, |header| {
                    if header.to_ascii_lowercase().starts_with("get /asset") {
                        (
                            format!("HTTP/1.1 302 Found\r\nLocation: http://localhost:{port}/other\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
                            Vec::new(),
                        )
                    } else {
                        (
                            "HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\n".into(),
                            vec![b"fixture".to_vec()],
                        )
                    }
                })
                .await;
                server_seen.lock().unwrap().push(header);
            }
        });
        let hash = sha256(b"fixture");
        let asset = Asset { url: &url, bytes: 7, sha256: &hash };
        let client = reqwest::Client::builder()
            .default_headers({
                let mut headers = reqwest::header::HeaderMap::new();
                headers.insert(
                    reqwest::header::AUTHORIZATION,
                    reqwest::header::HeaderValue::from_static("Bearer test-token"),
                );
                headers
            })
            .build()
            .unwrap();
        let (_sender, receiver) = watch::channel(false);
        fetch_with(&client, &asset, &destination, receiver, |_| {}, &fast())
            .await
            .unwrap();
        let requests = seen.lock().unwrap();
        let redirect_host = requests
            .iter()
            .find(|h| h.to_ascii_lowercase().contains("host: localhost"))
            .expect("the redirect target was never requested");
        assert!(
            !redirect_host.to_ascii_lowercase().contains("authorization"),
            "credentials must not leak to another host: {redirect_host}"
        );
        assert!(requests[0].to_ascii_lowercase().contains("authorization"));
        server.abort();
    }

    #[tokio::test]
    async fn reservations_share_the_volume_and_verification_needs_no_reserve() {
        let temp = tempfile::tempdir().unwrap();
        let free = available_space(temp.path()).unwrap();
        let storage = storage(temp.path(), 0, 0).unwrap();
        assert_eq!(storage.required, 0);
        // Free space on a shared volume moves between samples.
        assert!(storage.available <= free);
        assert!(free - storage.available < 128 * 1024 * 1024);
        let held = free / 2;
        let first = reserve(temp.path(), held).unwrap();
        let second = reserve(temp.path(), held).err().unwrap();
        assert!(second.contains("Not enough disk space"), "{second}");
        first.set_remaining(held / 2);
        let second = reserve(temp.path(), held).unwrap();
        drop(second);
        let third = reserve(temp.path(), held).unwrap();
        drop(third);
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
                loop {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    let mut request = [0; 4096];
                    let _ = socket.read(&mut request).await;
                    let _ = socket
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nfixture",
                        )
                        .await;
                }
            });
            let hash = if case == "hash" {
                sha256(b"different-content")
            } else {
                sha256(b"fixture")
            };
            let (sender, receiver) = watch::channel(case == "cancel");
            let asset = Asset {
                url: &url,
                bytes: if case == "size" { 8 } else { 7 },
                sha256: &hash,
            };
            let result = fetch_with(
                &reqwest::Client::new(),
                &asset,
                &destination,
                receiver,
                |_| {},
                &fast(),
            )
            .await;
            drop(sender);
            server.abort();
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
