use sha2::{Digest, Sha256};

/// Runs once at launch. Only journaled resources are inspected; code is never replayed.
pub async fn recover_at_startup(state: &crate::AppState) -> Result<(), String> {
    let _guard = state.daytona_operation.lock().await;
    let operations = pending(state)?;
    if operations.is_empty() {
        return Ok(());
    }
    let credential = state.daytona_vault.load("daytona").and_then(|bytes| {
        let bytes = bytes.ok_or("Save the Daytona credential to resume pending cleanup.")?;
        String::from_utf8(bytes).map_err(|_| "Saved Daytona credential is invalid.".into())
    });
    let (client, scope) = match credential
        .and_then(|key| Ok((crate::daytona::Client::new(&key)?, credential_scope(&key))))
    {
        Ok(value) => value,
        Err(_) => {
            for operation in operations {
                state.daytona_journal.lock().map_err(|_| "Cloud journal unavailable.")?
                    .cleanup_failed(&operation.name, "Startup cleanup needs a valid saved Daytona credential. Open Execution to resolve pending cleanup.")?;
            }
            return Ok(());
        }
    };
    recover_saved(&client, &state.daytona_journal, operations, &scope).await
}

async fn recover_saved(
    client: &impl crate::daytona_cleanup::CleanupTransport,
    journal: &std::sync::Mutex<crate::daytona_journal::Journal>,
    operations: Vec<crate::daytona_journal::PendingOperation>,
    scope: &str,
) -> Result<(), String> {
    for operation in operations {
        if operation.credential_scope != scope {
            journal
                .lock()
                .map_err(|_| "Cloud journal unavailable.")?
                .cleanup_failed(
                    &operation.name,
                    "Saved credential does not match this operation. Nothing was deleted.",
                )?;
            continue;
        }
        // Recovery persists per-resource failures and continues with other owned resources.
        let _ = crate::daytona_cleanup::recover(client, journal, &operation.name, scope).await;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn startup_retains_failures_skips_other_credentials_and_continues() {
        struct Remote;
        #[async_trait::async_trait]
        impl crate::daytona_cleanup::CleanupTransport for Remote {
            async fn inspect(&self, id: &str) -> Result<Option<crate::daytona::Sandbox>, String> {
                match id {
                    "failed" => Err("Daytona returned HTTP 503.".into()),
                    "absent" => Ok(None),
                    _ => panic!("must not inspect resources under another credential"),
                }
            }
            async fn delete(&self, _: &str) -> Result<(), String> {
                panic!("absent or inaccessible resources must not be deleted")
            }
        }
        let temp = tempfile::tempdir().unwrap();
        let journal = std::sync::Mutex::new(
            crate::daytona_journal::Journal::open(&temp.path().join("journal")).unwrap(),
        );
        let scope = "a".repeat(64);
        let failed = journal.lock().unwrap().begin(&scope).unwrap();
        journal
            .lock()
            .unwrap()
            .associate(&failed, "failed")
            .unwrap();
        let other = journal.lock().unwrap().begin(&"b".repeat(64)).unwrap();
        let absent = journal.lock().unwrap().begin(&scope).unwrap();
        journal
            .lock()
            .unwrap()
            .associate(&absent, "absent")
            .unwrap();
        let mut operations = journal.lock().unwrap().pending().unwrap();
        operations.sort_by_key(|item| if item.name == failed { 0 } else { 1 });
        recover_saved(&Remote, &journal, operations, &scope)
            .await
            .unwrap();
        let remaining = journal.lock().unwrap().pending().unwrap();
        assert_eq!(remaining.len(), 2);
        assert!(remaining.iter().all(|item| item.cleanup_error.is_some()));
        assert!(remaining.iter().any(|item| item.name == failed));
        assert!(remaining.iter().any(|item| item.name == other));
    }
}
pub fn credential_scope(key: &str) -> String {
    format!("{:x}", Sha256::digest(key.as_bytes()))
}
fn pending(
    state: &crate::AppState,
) -> Result<Vec<crate::daytona_journal::PendingOperation>, String> {
    state
        .daytona_journal
        .lock()
        .map_err(|_| "Cloud journal unavailable.")?
        .pending()
}
#[tauri::command]
pub fn has_daytona_key(state: tauri::State<'_, crate::AppState>) -> Result<bool, String> {
    Ok(state.daytona_vault.load("daytona")?.is_some())
}
#[tauri::command]
pub async fn save_daytona_key(
    state: tauri::State<'_, crate::AppState>,
    key: String,
) -> Result<(), String> {
    let _model_operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the current model operation before changing cloud credentials.")?;
    let _guard = state
        .daytona_operation
        .try_lock()
        .map_err(|_| "Wait for the cloud operation to finish.")?;
    crate::daytona::Client::new(&key)?;
    let scope = credential_scope(&key);
    if pending(&state)?
        .iter()
        .any(|item| item.credential_scope != scope)
    {
        return Err("Resolve pending cloud cleanup before replacing its credential.".into());
    }
    state.daytona_vault.save("daytona", key.as_bytes())
}
#[tauri::command]
pub async fn forget_daytona_key(state: tauri::State<'_, crate::AppState>) -> Result<(), String> {
    let _model_operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the current model operation before changing cloud credentials.")?;
    let _guard = state
        .daytona_operation
        .try_lock()
        .map_err(|_| "Wait for the cloud operation to finish.")?;
    if !pending(&state)?.is_empty() {
        return Err("Resolve pending cloud cleanup before forgetting its credential.".into());
    }
    state.daytona_vault.clear("daytona")
}
#[tauri::command]
pub async fn retry_daytona_cleanup(
    state: tauri::State<'_, crate::AppState>,
    name: String,
) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the current model operation to finish.")?;
    let _guard = state
        .daytona_operation
        .try_lock()
        .map_err(|_| "Another cloud operation is in progress.")?;
    let bytes = state
        .daytona_vault
        .load("daytona")?
        .ok_or("Save the Daytona credential for this operation first.")?;
    let key = String::from_utf8(bytes).map_err(|_| "Saved Daytona credential is invalid.")?;
    let client = crate::daytona::Client::new(&key)?;
    crate::daytona_cleanup::recover(
        &client,
        &state.daytona_journal,
        &name,
        &credential_scope(&key),
    )
    .await
}
