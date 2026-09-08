use sha2::{Digest, Sha256};
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
