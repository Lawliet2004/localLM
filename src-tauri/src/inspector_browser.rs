//! Embedded inspector browser: a child webview (WebView2/Chromium on Windows)
//! laid over a placeholder rect inside the workspace panel. Each browser tab
//! owns one labeled webview so switching tabs preserves its page state. The
//! DOM owns the toolbar; this module owns the native surface: position, size,
//! visibility, navigation. Main-webview CSP/iframe rules do not apply here.

use serde_json::Value;
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Url};

const HOME: &str = "about:blank";
const LABEL_PREFIX: &str = "inspector-browser";

#[cfg(target_os = "linux")]
#[path = "inspector_browser_linux.rs"]
mod linux;

fn checked_label(raw: &str) -> Result<String, String> {
    let label = raw.trim();
    if label.len() > 64
        || label.is_empty()
        || !label
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err("Invalid browser tab label.".into());
    }
    Ok(format!("{LABEL_PREFIX}-{label}"))
}

fn checked_url(raw: &str) -> Result<Url, String> {
    let url: Url = raw
        .parse()
        .map_err(|_| "Enter a valid address.".to_string())?;
    match url.scheme() {
        "https" | "http"
            if url.host_str().is_some()
                && url.username().is_empty()
                && url.password().is_none() =>
        {
            Ok(url)
        }
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
    if ![x, y, width, height].iter().all(|value| value.is_finite()) {
        return Err("Invalid browser dimensions.".into());
    }
    let label = checked_label(&label)?;
    let position = LogicalPosition::new(x.max(0.0), y.max(0.0));
    let size = LogicalSize::new(width.max(1.0), height.max(1.0));
    #[cfg(target_os = "linux")]
    linux::prepare(&app, &window).await?;
    let view = if let Some(view) = app.get_webview(&label) {
        view
    } else {
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
            if checked_url(url.as_str()).is_err() {
                return false;
            }
            let _ = navigator.emit(
                "browser-navigated",
                serde_json::json!({"label": nav_label, "url": url.to_string()}),
            );
            true
        })
        // target=_blank / window.open navigates the embedded view instead of
        // spawning an OS window the panel cannot manage.
        .on_new_window(move |url, _features| {
            if let Some(view) = popup_router
                .get_webview(&popup_label)
                .filter(|_| checked_url(url.as_str()).is_ok())
            {
                let _ = view.navigate(url);
            }
            tauri::webview::NewWindowResponse::Deny
        });
        window
            .add_child(builder, position, size)
            .map_err(|e| e.to_string())?
    };
    #[cfg(target_os = "linux")]
    {
        linux::place(
            &view,
            &window,
            position.x,
            position.y,
            size.width,
            size.height,
        )
        .await
    }
    #[cfg(not(target_os = "linux"))]
    {
        view.set_bounds(tauri::Rect {
            position: position.into(),
            size: size.into(),
        })
        .map_err(|e| e.to_string())?;
        view.show().map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_navigation_rejects_local_files_scripts_and_credentials() {
        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:text/html,hello",
            "about:config",
            "https://user:secret@example.com",
        ] {
            assert!(checked_url(url).is_err(), "{url}");
        }
        for url in [
            "https://example.com",
            "http://localhost:1420",
            "about:blank",
        ] {
            assert!(checked_url(url).is_ok(), "{url}");
        }
        for label in ["", "../main", "space name", "<script>"] {
            assert!(checked_label(label).is_err());
        }
    }
}

#[tauri::command]
pub fn browser_hide(app: AppHandle, label: String) -> Result<(), String> {
    if let Some(view) = app.get_webview(&checked_label(&label)?) {
        view.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn browser_close(app: AppHandle, label: String) -> Result<(), String> {
    if let Some(view) = app.get_webview(&checked_label(&label)?) {
        view.close().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn browser_navigate(app: AppHandle, label: String, url: String) -> Result<(), String> {
    let url = checked_url(&url)?;
    let view = app
        .get_webview(&checked_label(&label)?)
        .ok_or("The browser is closed.")?;
    view.navigate(url).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn browser_reload(app: AppHandle, label: String) -> Result<(), String> {
    if let Some(view) = app.get_webview(&checked_label(&label)?) {
        view.reload().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn browser_go_back(app: AppHandle, label: String) -> Result<(), String> {
    if let Some(view) = app.get_webview(&checked_label(&label)?) {
        view.eval("history.back()").map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn browser_go_forward(app: AppHandle, label: String) -> Result<(), String> {
    if let Some(view) = app.get_webview(&checked_label(&label)?) {
        view.eval("history.forward()").map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn browser_url(app: AppHandle, label: String) -> Result<Value, String> {
    match app.get_webview(&checked_label(&label)?) {
        Some(view) => {
            let url = view.url().map_err(|e| e.to_string())?;
            Ok(serde_json::json!({"url": url.to_string()}))
        }
        None => Ok(serde_json::json!({"url": null})),
    }
}
