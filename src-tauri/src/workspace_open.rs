fn resolve_workspace_path(bound: &str, path: &str) -> Result<std::path::PathBuf, String> {
    let requested = std::path::Path::new(path);
    let absolute = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        crate::editor::validate_path(path)?;
        std::path::Path::new(bound).join(requested)
    };
    let canonical_root = std::fs::canonicalize(bound)
        .map_err(|error| format!("Could not resolve the workspace folder: {error}"))?;
    let canonical = std::fs::canonicalize(&absolute)
        .map_err(|_| "That file is no longer available.".to_string())?;
    if !canonical.starts_with(&canonical_root) {
        return Err("Only files inside the selected workspace can be opened.".into());
    }
    Ok(canonical)
}

/// Open a workspace file or folder in the operating system's default
/// application.
///
/// The renderer cannot hand an arbitrary absolute path to the opener plugin:
/// the path is resolved against the selected workspace, canonicalized, and
/// refused when it leaves that workspace. The opener plugin is invoked from
/// the backend so the capability file does not have to grant
/// `opener:allow-open-path` to the webview.
// Registered in `lib.rs` (see the integration note at the end of this file).
#[tauri::command]
pub fn open_workspace_path(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    path: String,
    root: Option<String>,
) -> Result<(), String> {
    let bound = match root.filter(|root| !root.is_empty()) {
        Some(root) => crate::workspace_ui::bound_root(&state, &root)?,
        None => state.database()?.workspace_path()?,
    };
    let canonical = resolve_workspace_path(&bound, &path)?;
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(canonical.to_string_lossy().into_owned(), None::<String>)
        .map_err(|error| format!("Could not open that file: {error}"))
}
