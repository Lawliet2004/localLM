//! Embedded inspector browser: a child webview (WebView2/Chromium on Windows)
//! laid over a placeholder rect inside the workspace panel. Each browser tab
//! owns one labeled webview so switching tabs preserves its page state. The
//! DOM owns the toolbar; this module owns the native surface: position, size,
//! visibility, navigation. Main-webview CSP/iframe rules do not apply here.

use serde_json::Value;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Url};

const HOME: &str = "about:blank";
const LABEL_PREFIX: &str = "inspector-browser";

/// Set once in setup so harness tools (which only see `AppState`) can reach
/// the webviews.
pub static APP: std::sync::OnceLock<AppHandle> = std::sync::OnceLock::new();

pub(crate) fn checked_label(raw: &str) -> Result<String, String> {
    let label = raw.trim();
    if label.len() > 64 || label.is_empty() || !label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("Invalid browser tab label.".into());
    }
    Ok(format!("{LABEL_PREFIX}-{label}"))
}

pub(crate) fn checked_url(raw: &str) -> Result<Url, String> {
    let url: Url = raw.parse().map_err(|_| "Enter a valid address.".to_string())?;
    match url.scheme() {
        "https" | "http" => Ok(url),
        "about" if raw == "about:blank" => Ok(url),
        _ => Err("Only HTTP and HTTPS addresses can be opened.".into()),
    }
}

/// Create-or-move the browser surface over the host rect (logical pixels,
/// window-relative — the same space getBoundingClientRect reports).
#[tauri::command]
pub async fn browser_show(
    app: AppHandle,
    window: tauri::Window,
    label: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let label = checked_label(&label)?;
    let position = LogicalPosition::new(x.max(0.0), y.max(0.0));
    let size = LogicalSize::new(width.max(1.0), height.max(1.0));
    if let Some(view) = app.get_webview(&label) {
        view.set_position(position).map_err(|e| e.to_string())?;
        view.set_size(size).map_err(|e| e.to_string())?;
        view.show().map_err(|e| e.to_string())
    } else {
        create(&app, &window, label, position, size).map(|_| ())
    }
}

pub(crate) fn create(
    app: &AppHandle,
    window: &tauri::Window,
    label: String,
    position: LogicalPosition<f64>,
    size: LogicalSize<f64>,
) -> Result<tauri::Webview, String> {
    let navigator = app.clone();
    let nav_label = label.clone();
    let popup_router = app.clone();
    let popup_label = label.clone();
    let builder = tauri::webview::WebviewBuilder::new(
        label,
        tauri::WebviewUrl::External(checked_url(HOME)?),
    )
    // Keep the owning tab's address bar in sync with in-page navigation.
    .on_navigation(move |url| {
        if checked_url(url.as_str()).is_err() { return false; }
        let _ = navigator.emit(
            "browser-navigated",
            serde_json::json!({"label": nav_label, "url": url.to_string()}),
        );
        true
    })
    // target=_blank / window.open navigates the embedded view instead of
    // spawning an OS window the panel cannot manage.
    .on_new_window(move |url, _features| {
        if let Ok(url) = checked_url(url.as_str()) {
            if let Some(view) = popup_router.get_webview(&popup_label) {
                let _ = view.navigate(url);
            }
        }
        tauri::webview::NewWindowResponse::Deny
    });
    window.add_child(builder, position, size).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_hide(app: AppHandle, label: String) -> Result<(), String> {
    if let Some(view) = app.get_webview(&checked_label(&label)?) {
        view.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn browser_close(app: AppHandle, label: String) -> Result<(), String> {
    if let Some(view) = app.get_webview(&checked_label(&label)?) {
        view.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Recover child webviews left visible after a frontend reload or a tab race.
#[tauri::command]
pub async fn browser_close_all(app: AppHandle) {
    for (label, view) in app.webviews() {
        if label.starts_with(&format!("{LABEL_PREFIX}-")) {
            let _ = view.close();
        }
    }
}

#[tauri::command]
pub async fn browser_navigate(app: AppHandle, label: String, url: String) -> Result<(), String> {
    let url = checked_url(&url)?;
    let view = app.get_webview(&checked_label(&label)?).ok_or("The browser is closed.")?;
    view.navigate(url).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn browser_reload(app: AppHandle, label: String) -> Result<(), String> {
    if let Some(view) = app.get_webview(&checked_label(&label)?) {
        view.reload().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn browser_go_back(app: AppHandle, label: String) -> Result<(), String> {
    if let Some(view) = app.get_webview(&checked_label(&label)?) {
        view.eval("history.back()").map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn browser_go_forward(app: AppHandle, label: String) -> Result<(), String> {
    if let Some(view) = app.get_webview(&checked_label(&label)?) {
        view.eval("history.forward()").map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn browser_url(app: AppHandle, label: String) -> Result<Value, String> {
    match app.get_webview(&checked_label(&label)?) {
        Some(view) => {
            let url = view.url().map_err(|e| e.to_string())?;
            Ok(serde_json::json!({"url": url.to_string()}))
        }
        None => Ok(serde_json::json!({"url": null})),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_navigation_allows_web_pages_and_rejects_local_or_executable_schemes() {
        for url in ["https://example.com/page", "http://localhost:1420", "about:blank"] {
            assert!(checked_url(url).is_ok(), "{url}");
        }
        for url in ["file:///C:/private.txt", "javascript:alert(1)", "data:text/html,hello", "tauri://localhost", "mailto:someone@example.com", "about:settings"] {
            assert!(checked_url(url).is_err(), "{url}");
        }
        assert!(checked_label("agent").is_ok());
        assert!(checked_label("../agent").is_err());
    }
}
