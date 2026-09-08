use crate::{
    daytona::{Client, CodeResult, Sandbox},
    daytona_cleanup::{self, CleanupTransport},
    daytona_journal::Journal,
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{watch, Mutex as AsyncMutex};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodeRequest {
    pub language: String,
    pub code: String,
    pub timeout_seconds: u32,
}
impl CodeRequest {
    pub fn validate(&self) -> Result<(), String> {
        if !matches!(
            self.language.as_str(),
            "python" | "javascript" | "typescript"
        ) || self.code.trim().is_empty()
            || self.code.len() > 32768
            || !(1..=90).contains(&self.timeout_seconds)
        {
            return Err("Use Python, JavaScript or TypeScript, at most 32 KiB of code and a 1–90 second timeout.".into());
        }
        Ok(())
    }
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub operation_name: String,
    pub result: Option<CodeResult>,
    pub error: Option<String>,
    pub cleanup_error: Option<String>,
    pub is_error: bool,
}
#[async_trait::async_trait]
pub trait ExecutionTransport: CleanupTransport {
    async fn create(&self, name: &str) -> Result<Sandbox, String>;
    async fn execute(&self, sandbox: &Sandbox, request: &CodeRequest)
        -> Result<CodeResult, String>;
}
#[async_trait::async_trait]
impl ExecutionTransport for Client {
    async fn create(&self, name: &str) -> Result<Sandbox, String> {
        Client::create(self, name).await
    }
    async fn execute(
        &self,
        sandbox: &Sandbox,
        request: &CodeRequest,
    ) -> Result<CodeResult, String> {
        self.run_code(
            sandbox,
            &request.language,
            &request.code,
            request.timeout_seconds,
        )
        .await
    }
}
struct CancelOnDrop(watch::Sender<bool>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}
fn journal(store: &Mutex<Journal>) -> Result<std::sync::MutexGuard<'_, Journal>, String> {
    store
        .lock()
        .map_err(|_| "Cloud journal unavailable.".into())
}
fn owned(sandbox: &Sandbox, name: &str) -> bool {
    sandbox.name == name
        && sandbox.labels.get("locallm-operation").map(String::as_str) == Some(name)
}

/// Dropping the caller requests cancellation, but the worker still records creation and attempts cleanup.
pub async fn run<T: ExecutionTransport + 'static>(
    transport: Arc<T>,
    store: Arc<Mutex<Journal>>,
    operation: Arc<AsyncMutex<()>>,
    scope: String,
    request: CodeRequest,
) -> Result<Outcome, String> {
    request.validate()?;
    let (sender, mut cancelled) = watch::channel(false);
    let _cancel = CancelOnDrop(sender);
    tokio::spawn(async move {
        let _guard=operation.lock_owned().await;
        if *cancelled.borrow(){return Err("Cloud execution cancelled before creation.".into());}
        let name=journal(&store)?.begin(&scope)?;
        // Creation is allowed to finish within its transport deadline even if the caller disappears,
        // so its returned resource identity can still be captured for cleanup.
        let execution=async {
            let sandbox=transport.create(&name).await?;
            if !owned(&sandbox,&name){return Err("Created sandbox ownership did not match the operation.".into());}
            journal(&store)?.associate(&name,&sandbox.id)?;
            if *cancelled.borrow(){return Err("Cloud execution cancelled after creation; code was not submitted.".into());}
            let execute=async {
                let mut ready=sandbox;
                let deadline=tokio::time::Instant::now()+Duration::from_secs(60);
                loop {
                    if ready.state.as_deref()==Some("started"){break;}
                    if matches!(ready.state.as_deref(),Some("error"|"destroyed"|"destroying")){return Err("Daytona sandbox did not become ready.".into());}
                    if tokio::time::Instant::now()>=deadline{return Err("Daytona sandbox readiness timed out.".into());}
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    let next=transport.inspect(&ready.id).await?.ok_or("Daytona sandbox disappeared before execution.")?;
                    if next.id!=ready.id || !owned(&next,&name){return Err("Sandbox identity changed before execution.".into());}
                    ready=next;
                }
                transport.execute(&ready,&request).await
            };
            tokio::select! {
                _=cancelled.changed()=>Err("Cloud execution cancelled; code may have run. Cleanup was requested.".into()),
                value=tokio::time::timeout(Duration::from_secs(160),execute)=>value.unwrap_or_else(|_|Err("Cloud execution deadline exceeded; code may have run.".into())),
            }
        }.await;
        let cleanup_error=daytona_cleanup::recover(transport.as_ref(),&store,&name,&scope).await.err();
        let is_error=execution.as_ref().map_or(true,|value|value.exit_code!=0) || cleanup_error.is_some();
        let (result,error)=match execution {Ok(value)=>(Some(value),None),Err(error)=>(None,Some(error))};
        Ok(Outcome{operation_name:name,result,error,cleanup_error,is_error})
    }).await.map_err(|_|"Cloud worker failed; inspect pending cleanup before retrying.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    struct Remote {
        store: Arc<Mutex<Journal>>,
        name: Mutex<String>,
        deleted: AtomicBool,
        runs: AtomicUsize,
        entered: tokio::sync::Notify,
        block: bool,
        exit_code: i32,
    }
    impl Remote {
        fn sandbox(&self) -> Sandbox {
            let name = self.name.lock().unwrap().clone();
            Sandbox {
                id: "sandbox-1".into(),
                labels: [("locallm-operation".into(), name.clone())].into(),
                name,
                state: Some("started".into()),
                toolbox_proxy_url: None,
            }
        }
    }
    #[async_trait::async_trait]
    impl CleanupTransport for Remote {
        async fn inspect(&self, _: &str) -> Result<Option<Sandbox>, String> {
            Ok((!self.deleted.load(Ordering::SeqCst)).then(|| self.sandbox()))
        }
        async fn delete(&self, _: &str) -> Result<(), String> {
            self.deleted.store(true, Ordering::SeqCst);
            Ok(())
        }
    }
    #[async_trait::async_trait]
    impl ExecutionTransport for Remote {
        async fn create(&self, name: &str) -> Result<Sandbox, String> {
            assert_eq!(journal(&self.store)?.pending()?[0].name, name);
            *self.name.lock().unwrap() = name.into();
            Ok(self.sandbox())
        }
        async fn execute(&self, _: &Sandbox, _: &CodeRequest) -> Result<CodeResult, String> {
            self.runs.fetch_add(1, Ordering::SeqCst);
            self.entered.notify_one();
            if self.block {
                std::future::pending().await
            } else {
                Ok(CodeResult {
                    exit_code: self.exit_code,
                    result: "42".into(),
                    artifacts: None,
                })
            }
        }
    }
    #[tokio::test]
    async fn execution_and_caller_cancellation_both_cleanup_owned_resources() {
        for (block, exit_code) in [(false, 0), (true, 0), (false, 7)] {
            let temp = tempfile::tempdir().unwrap();
            let store = Arc::new(Mutex::new(
                Journal::open(&temp.path().join("journal")).unwrap(),
            ));
            let remote = Arc::new(Remote {
                store: store.clone(),
                name: Mutex::new(String::new()),
                deleted: AtomicBool::new(false),
                runs: AtomicUsize::new(0),
                entered: tokio::sync::Notify::new(),
                block,
                exit_code,
            });
            let task = tokio::spawn(run(
                remote.clone(),
                store.clone(),
                Arc::new(AsyncMutex::new(())),
                "a".repeat(64),
                CodeRequest {
                    language: "python".into(),
                    code: "print(42)".into(),
                    timeout_seconds: 30,
                },
            ));
            tokio::time::timeout(Duration::from_secs(2), remote.entered.notified())
                .await
                .unwrap();
            if block {
                task.abort();
                assert!(matches!(task.await, Err(error) if error.is_cancelled()));
            } else {
                let outcome = task.await.unwrap().unwrap();
                assert_eq!(outcome.is_error, exit_code != 0);
                assert_eq!(outcome.result.unwrap().result, "42");
            }
            tokio::time::timeout(Duration::from_secs(2), async {
                while !journal(&store).unwrap().pending().unwrap().is_empty() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            assert!(remote.deleted.load(Ordering::SeqCst));
            assert_eq!(remote.runs.load(Ordering::SeqCst), 1);
        }
    }
}
