//! Tauri/Wry adds Linux child webviews to the window's vertical GtkBox.
//! That is a split layout, not a positioned child: it shrinks the main view
//! and ignores child bounds. Keep the main view as an overlay's sole content
//! child, with browser views in a separate, non-measuring fixed layer.
use gtk::prelude::*;
use tauri::{AppHandle, Manager, Webview, Window};

const OVERLAY_NAME: &str = "locallm-browser-overlay";
const LAYER_NAME: &str = "locallm-browser-layer";

fn layer(window: &Window) -> Result<gtk::Fixed, String> {
    let root = window.default_vbox().map_err(|e| e.to_string())?;
    let overlay = root
        .children()
        .into_iter()
        .find(|child| child.widget_name() == OVERLAY_NAME)
        .ok_or("The browser overlay is unavailable.")?
        .downcast::<gtk::Overlay>()
        .map_err(|_| "Invalid browser overlay.")?;
    overlay
        .children()
        .into_iter()
        .find(|child| child.widget_name() == LAYER_NAME)
        .ok_or("The browser layer is unavailable.")?
        .downcast::<gtk::Fixed>()
        .map_err(|_| "Invalid browser layer.".into())
}

pub async fn prepare(app: &AppHandle, window: &Window) -> Result<(), String> {
    let main = app
        .get_webview(window.label())
        .ok_or("The application view is closed.")?;
    let window = window.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    main.with_webview(move |platform| {
        let result = (|| {
            if layer(&window).is_ok() {
                return Ok(());
            }
            let primary = platform.inner();
            let root = window.default_vbox().map_err(|e| e.to_string())?;
            if primary.parent().as_ref() != Some(root.upcast_ref()) {
                return Err("Cannot attach the browser to this application layout.".into());
            }
            let overlay = gtk::Overlay::new();
            overlay.set_widget_name(OVERLAY_NAME);
            overlay.set_hexpand(true);
            overlay.set_vexpand(true);
            let fixed = gtk::Fixed::new();
            fixed.set_widget_name(LAYER_NAME);
            fixed.set_hexpand(true);
            fixed.set_vexpand(true);
            // GTK does not measure overlay children when their requested
            // sizes change. Reapply each browser's dimensions after the fixed
            // layer allocates, including restoration of a hidden tab.
            fixed.connect_size_allocate(|fixed, _| {
                for child in fixed.children() {
                    if !child.is_visible() {
                        continue;
                    }
                    let allocation = child.allocation();
                    child.size_allocate(&gtk::Allocation::new(
                        allocation.x(),
                        allocation.y(),
                        child.width_request().max(1),
                        child.height_request().max(1),
                    ));
                }
            });
            // Overlay children do not contribute their minimum sizes to the
            // window, so a browser can never resize the application's content.
            root.remove(&primary);
            overlay.add(&primary);
            overlay.add_overlay(&fixed);
            // The full-window positioning layer is transparent to input.
            // Browser child windows still receive input within their bounds;
            // the application receives clicks everywhere else.
            overlay.set_overlay_pass_through(&fixed, true);
            root.pack_start(&overlay, true, true, 0);
            overlay.show_all();
            Ok(())
        })();
        let _ = tx.send(result);
    })
    .map_err(|e| e.to_string())?;
    rx.await
        .map_err(|_| "The browser layout operation was cancelled.".to_string())?
}

pub async fn place(
    view: &Webview,
    window: &Window,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    let window = window.clone();
    let (tx, rx) = tokio::sync::oneshot::channel();
    view.with_webview(move |platform| {
        let result = (|| {
            let fixed = layer(&window)?;
            let child = platform.inner();
            if child.parent().as_ref() != Some(fixed.upcast_ref()) {
                if let Some(parent) = child.parent() {
                    parent
                        .downcast::<gtk::Container>()
                        .map_err(|_| "Invalid browser parent.")?
                        .remove(&child);
                }
                child.set_hexpand(false);
                child.set_vexpand(false);
                fixed.put(&child, x.round() as i32, y.round() as i32);
            } else {
                fixed.move_(&child, x.round() as i32, y.round() as i32);
            }
            child.set_size_request(
                width.round().max(1.0) as i32,
                height.round().max(1.0) as i32,
            );
            child.show();
            let _ = child.preferred_size();
            // GtkOverlay excludes overlay children from size negotiation, so
            // a changed request alone does not always allocate a new size.
            // Apply the host allocation explicitly, as Wry does for views it
            // originally created under a GtkFixed container.
            child.size_allocate(&gtk::Allocation::new(
                x.round() as i32,
                y.round() as i32,
                width.round().max(1.0) as i32,
                height.round().max(1.0) as i32,
            ));
            fixed.queue_resize();
            fixed.check_resize();
            Ok(())
        })();
        let _ = tx.send(result);
    })
    .map_err(|e| e.to_string())?;
    rx.await
        .map_err(|_| "The browser layout operation was cancelled.".to_string())?
}
