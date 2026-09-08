use crate::{
    daytona::{Client, Sandbox},
    daytona_journal::Journal,
};
use std::{sync::Mutex, time::Duration};

#[async_trait::async_trait]
pub trait CleanupTransport: Send + Sync {
    async fn inspect(&self, id: &str) -> Result<Option<Sandbox>, String>;
    async fn delete(&self, id: &str) -> Result<(), String>;
}
#[async_trait::async_trait]
impl CleanupTransport for Client {
    async fn inspect(&self, id: &str) -> Result<Option<Sandbox>, String> {
        Client::inspect(self, id).await
    }
    async fn delete(&self, id: &str) -> Result<(), String> {
        Client::delete(self, id).await
    }
}
fn journal(store: &Mutex<Journal>) -> Result<std::sync::MutexGuard<'_, Journal>, String> {
    store
        .lock()
        .map_err(|_| "Cloud journal unavailable.".into())
}

/// Cancellation leaves ownership durable. A DELETE acknowledgement alone never clears the record.
pub async fn recover(
    transport: &impl CleanupTransport,
    store: &Mutex<Journal>,
    name: &str,
    scope: &str,
) -> Result<(), String> {
    let operation = journal(store)?
        .pending()?
        .into_iter()
        .find(|item| item.name == name)
        .ok_or("Unknown cloud operation.")?;
    if operation.credential_scope != scope {
        return Err("Select the credentials associated with this cloud operation.".into());
    }
    let result=tokio::time::timeout(Duration::from_secs(60),async {
        let found=transport.inspect(operation.sandbox_id.as_deref().unwrap_or(name)).await?;
        let Some(sandbox)=found else {
            if operation.sandbox_id.is_none() {return Err("Creation outcome is still unknown. No sandbox is visible yet; the ownership record has been retained.".into());}
            return journal(store)?.acknowledge_absent(name,scope);
        };
        if sandbox.name!=name || sandbox.labels.get("locallm-operation").map(String::as_str)!=Some(name)
            || operation.sandbox_id.as_ref().is_some_and(|id|id!=&sandbox.id) {
            return Err("Remote sandbox ownership does not match. Nothing was deleted.".into());
        }
        journal(store)?.associate(name,&sandbox.id)?;
        transport.delete(&sandbox.id).await?;
        for attempt in 0..15 {
            if transport.inspect(&sandbox.id).await?.is_none() {return journal(store)?.acknowledge_absent(name,scope);}
            if attempt<14 {tokio::time::sleep(Duration::from_secs(1)).await;}
        }
        Err("Deletion was requested but remote absence is not confirmed. Retry cleanup later.".into())
    }).await.unwrap_or_else(|_| Err("Cleanup timed out. Remote absence is not confirmed; the ownership record is retained.".into()));
    if let Err(error) = &result {
        journal(store)?.cleanup_failed(name, error)?;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    #[tokio::test]
    async fn cancellation_during_delete_keeps_associated_ownership() {
        struct Blocking {
            name: String,
            entered: tokio::sync::Notify,
        }
        #[async_trait::async_trait]
        impl CleanupTransport for Blocking {
            async fn inspect(&self, _: &str) -> Result<Option<Sandbox>, String> {
                Ok(Some(sandbox(&self.name)))
            }
            async fn delete(&self, _: &str) -> Result<(), String> {
                self.entered.notify_one();
                std::future::pending().await
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("journal");
        let store = std::sync::Arc::new(Mutex::new(Journal::open(&path).unwrap()));
        let scope = "a".repeat(64);
        let name = journal(&store).unwrap().begin(&scope).unwrap();
        let remote = std::sync::Arc::new(Blocking {
            name: name.clone(),
            entered: tokio::sync::Notify::new(),
        });
        let task = tokio::spawn({
            let store = store.clone();
            let remote = remote.clone();
            async move { recover(remote.as_ref(), &store, &name, &scope).await }
        });
        tokio::time::timeout(Duration::from_secs(2), remote.entered.notified())
            .await
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        drop(store);
        let reopened = Journal::open(&path).unwrap();
        let pending = reopened.pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].sandbox_id.as_deref(), Some("sandbox-1"));
    }
    struct Remote {
        answers: Mutex<VecDeque<Result<Option<Sandbox>, String>>>,
        deleted: Mutex<Vec<String>>,
        fail_delete: bool,
    }
    #[async_trait::async_trait]
    impl CleanupTransport for Remote {
        async fn inspect(&self, _: &str) -> Result<Option<Sandbox>, String> {
            self.answers
                .lock()
                .unwrap()
                .pop_front()
                .expect("unexpected inspection")
        }
        async fn delete(&self, id: &str) -> Result<(), String> {
            self.deleted.lock().unwrap().push(id.into());
            if self.fail_delete {
                Err("Daytona returned HTTP 503.".into())
            } else {
                Ok(())
            }
        }
    }
    fn sandbox(name: &str) -> Sandbox {
        Sandbox {
            id: "sandbox-1".into(),
            name: name.into(),
            labels: [("locallm-operation".into(), name.into())].into(),
            state: Some("started".into()),
            toolbox_proxy_url: None,
        }
    }
    #[tokio::test]
    async fn recovery_requires_ownership_and_confirmed_absence() {
        for case in ["success", "mismatch", "failure", "unknown"] {
            let temp = tempfile::tempdir().unwrap();
            let store = Mutex::new(Journal::open(&temp.path().join("journal")).unwrap());
            let scope = "a".repeat(64);
            let name = journal(&store).unwrap().begin(&scope).unwrap();
            let mut value = sandbox(&name);
            if case == "mismatch" {
                value.labels.clear();
            }
            let answers = if case == "unknown" {
                vec![Ok(None)]
            } else {
                vec![Ok(Some(value)), Ok(None)]
            };
            let remote = Remote {
                answers: Mutex::new(answers.into()),
                deleted: Mutex::new(Vec::new()),
                fail_delete: case == "failure",
            };
            assert!(recover(&remote, &store, &name, &"b".repeat(64))
                .await
                .is_err());
            let result = recover(&remote, &store, &name, &scope).await;
            let pending = journal(&store).unwrap().pending().unwrap();
            if case == "success" {
                assert!(result.is_ok());
                assert!(pending.is_empty());
                assert_eq!(*remote.deleted.lock().unwrap(), vec!["sandbox-1"]);
            } else {
                assert!(result.is_err());
                assert_eq!(pending.len(), 1);
                assert!(pending[0].cleanup_error.is_some());
                if case != "failure" {
                    assert!(remote.deleted.lock().unwrap().is_empty());
                }
            }
        }
    }
}
