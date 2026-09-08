use std::{io, path::Path, sync::Arc};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};

const LIMIT: usize = 8 * 1024 * 1024;
const TRUNCATED: &[u8] = b"\n[LocalLM: runtime log limit reached; further output is discarded until the next model load.]\n";

pub struct RuntimeLog {
    file: tokio::fs::File,
    remaining: usize,
    truncated: bool,
}
impl RuntimeLog {
    pub async fn create(path: &Path) -> io::Result<Arc<Mutex<Self>>> {
        Ok(Arc::new(Mutex::new(Self {
            file: tokio::fs::File::create(path).await?,
            remaining: LIMIT - TRUNCATED.len(),
            truncated: false,
        })))
    }
    async fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.truncated {
            return Ok(());
        }
        let count = bytes.len().min(self.remaining);
        self.file.write_all(&bytes[..count]).await?;
        self.remaining -= count;
        if count < bytes.len() {
            self.file.write_all(TRUNCATED).await?;
            self.truncated = true;
        }
        self.file.flush().await
    }
}
pub async fn drain(
    mut source: impl AsyncRead + Unpin,
    log: Arc<Mutex<RuntimeLog>>,
) -> io::Result<()> {
    let mut buffer = [0u8; 8192];
    let mut failure = None;
    loop {
        let count = source.read(&mut buffer).await?;
        if count == 0 {
            return failure.map_or(Ok(()), Err);
        }
        // Even a full disk or a capped log must not leave the child's pipe blocked.
        if failure.is_none() {
            if let Err(error) = log.lock().await.append(&buffer[..count]).await {
                failure = Some(error);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn write_failure_does_not_stop_pipe_drain() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("readonly");
        tokio::fs::write(&path, b"unchanged").await.unwrap();
        let log = Arc::new(Mutex::new(RuntimeLog {
            file: tokio::fs::File::open(&path).await.unwrap(),
            remaining: LIMIT,
            truncated: false,
        }));
        let (mut writer, reader) = tokio::io::duplex(32);
        let task = tokio::spawn(drain(reader, log));
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            writer.write_all(&[b'x'; 100_000]).await.unwrap();
            drop(writer);
            assert!(task.await.unwrap().is_err());
        })
        .await
        .unwrap();
        assert_eq!(tokio::fs::read(path).await.unwrap(), b"unchanged");
    }
    #[tokio::test]
    async fn capped_logs_keep_draining_both_streams_without_growing() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("runtime.log");
        let log = RuntimeLog::create(&path).await.unwrap();
        log.lock().await.remaining = 100;
        let (mut writer_a, reader_a) = tokio::io::duplex(32);
        let (mut writer_b, reader_b) = tokio::io::duplex(32);
        let a = tokio::spawn(drain(reader_a, log.clone()));
        let b = tokio::spawn(drain(reader_b, log.clone()));
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let writes = tokio::join!(
                writer_a.write_all(&[b'a'; 100_000]),
                writer_b.write_all(&[b'b'; 100_000])
            );
            writes.0.unwrap();
            writes.1.unwrap();
            drop(writer_a);
            drop(writer_b);
            a.await.unwrap().unwrap();
            b.await.unwrap().unwrap();
        })
        .await
        .unwrap();
        let bytes = tokio::fs::read(&path).await.unwrap();
        assert_eq!(bytes.len(), 100 + TRUNCATED.len());
        assert!(bytes.ends_with(TRUNCATED));
        assert!(bytes[..100].iter().all(|byte| matches!(byte, b'a' | b'b')));
        drop(log);
        let fresh = RuntimeLog::create(&path).await.unwrap();
        fresh.lock().await.append(b"new load").await.unwrap();
        assert_eq!(tokio::fs::read(path).await.unwrap(), b"new load");
    }
}
