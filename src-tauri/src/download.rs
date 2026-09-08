//! Streaming verified downloads. Final filenames are published only after verification.
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::{path::Path, time::Duration};
use tokio::{io::AsyncWriteExt, sync::watch};

pub struct Asset<'a> {
    pub url: &'a str,
    pub bytes: u64,
    pub sha256: &'a str,
}

/// The caller supplies a pinned asset and an application-owned destination.
/// Dropping this future or cancellation removes the unpublished temporary file.
pub async fn fetch(
    client: &reqwest::Client,
    asset: &Asset<'_>,
    destination: &Path,
    mut cancel: watch::Receiver<bool>,
    mut progress: impl FnMut(u64),
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
    let temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("Cannot create download file: {error}"))?;
    // Keep the tempfile owner alive until the asynchronous file handle is closed.
    let mut file =
        tokio::fs::File::from_std(temporary.reopen().map_err(|error| error.to_string())?);
    let work = async {
        let response = client
            .get(asset.url)
            .header(reqwest::header::ACCEPT_ENCODING, "identity")
            .send()
            .await
            .map_err(|_| "Could not connect to the download service.")?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(format!(
                "Download service returned HTTP {}.",
                response.status().as_u16()
            ));
        }
        if response
            .content_length()
            .is_some_and(|length| length != asset.bytes)
        {
            return Err("Download size differs from the pinned asset.".into());
        }
        let mut stream = response.bytes_stream();
        let mut received = 0_u64;
        let mut hash = Sha256::new();
        progress(0);
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
    drop(file);
    result?;
    temporary.persist_noclobber(destination).map_err(|error| {
        format!(
            "Could not install verified file; an existing destination is preserved: {}",
            error.error
        )
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;
    #[tokio::test]
    async fn cancellation_during_transfer_removes_partial_file() {
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
            sha256: &"0".repeat(64),
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
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
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
                "0".repeat(64)
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
            if case == "cancel" {
                server.abort();
            } else {
                server.await.unwrap();
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
}
