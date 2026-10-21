//! Real WebKitGTK regression check. Run on the isolated Xvfb display owned by
//! scripts/test-browser-layout.mjs; no user workspace or credentials are used.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[cfg(target_os = "linux")]
#[path = "../src/inspector_browser.rs"]
#[allow(dead_code)]
mod inspector_browser;

#[cfg(target_os = "linux")]
mod smoke {
    use enigo::{Button, Coordinate, Direction, Enigo, Mouse, Settings};
    use gtk::prelude::*;
    use serde::Serialize;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tauri::{AppHandle, Manager, Webview, WebviewUrl, WebviewWindowBuilder};
    use webkit2gtk::WebViewExt;

    #[derive(Debug, Serialize)]
    struct ViewState {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: String,
        visible: bool,
        title: String,
        requested_width: i32,
        requested_height: i32,
    }
    async fn sample(view: &Webview) -> Result<ViewState, String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        view.with_webview(move |platform| {
            let widget = platform.inner();
            let allocation = widget.allocation();
            let _ = tx.send(ViewState {
                x: allocation.x(),
                y: allocation.y(),
                width: allocation.width(),
                height: allocation.height(),
                parent: widget
                    .parent()
                    .map(|parent| parent.type_().name().to_string())
                    .unwrap_or_default(),
                visible: widget.is_visible(),
                title: widget.title().unwrap_or_default().to_string(),
                requested_width: widget.width_request(),
                requested_height: widget.height_request(),
            });
        })
        .map_err(|e| e.to_string())?;
        rx.await.map_err(|e| e.to_string())
    }
    async fn settle() {
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    fn click(x: i32, y: i32) -> Result<(), String> {
        let mut input = Enigo::new(&Settings {
            x11_display: Some(std::env::var("DISPLAY").map_err(|e| e.to_string())?),
            wayland_display: Some("locallm-isolated-no-wayland".into()),
            ..Settings::default()
        })
        .map_err(|e| e.to_string())?;
        input
            .move_mouse(x, y, Coordinate::Abs)
            .map_err(|e| e.to_string())?;
        input
            .button(Button::Left, Direction::Click)
            .map_err(|e| e.to_string())
    }
    fn check_main(state: &ViewState, width: i32, height: i32) -> Result<(), String> {
        if state.width != width || state.height != height || state.parent != "GtkOverlay" {
            return Err(format!("The main app was resized or reparented incorrectly: {state:?}, expected {width}x{height}"));
        }
        Ok(())
    }
    fn check_child(
        state: &ViewState,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> Result<(), String> {
        if state.x != x
            || state.y != y
            || state.width != width
            || state.height != height
            || state.parent != "GtkFixed"
            || !state.visible
        {
            return Err(format!(
                "Browser escaped its host rectangle: {state:?}, expected {x},{y} {width}x{height}"
            ));
        }
        Ok(())
    }
    fn fixture() -> Result<String, String> {
        let server = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
        let address = server.local_addr().map_err(|e| e.to_string())?;
        std::thread::spawn(move || {
            for mut stream in server.incoming().flatten() {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut bytes = [0; 4096];
                let count = stream.read(&mut bytes).unwrap_or(0);
                let html = if bytes[..count].starts_with(b"GET /main ") {
                    r#"<!doctype html><title>Main app ready</title><style>html,body{margin:0;height:100%;background:#202123}button{position:absolute;left:20px;top:20px;width:120px;height:40px}</style><button onclick="document.title='Main app click received'">App control</button>"#
                } else {
                    "<!doctype html><title>LocalLM browser layout fixture</title><style>html,body{margin:0;height:100%;background:#276b46;color:white}button{position:absolute;left:20px;top:20px;width:200px;height:40px}</style><button onclick=\"location.hash='clicked'\">Native browser button</button>"
                };
                let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}", html.len());
            }
        });
        Ok(format!("http://{address}/"))
    }
    pub async fn run(app: AppHandle) -> Result<(), String> {
        let window = app.get_window("main").ok_or("Main window missing")?;
        let main = app.get_webview("main").ok_or("Main webview missing")?;
        settle().await;
        let before = sample(&main).await?;
        if before.width != 900 || before.height != 600 {
            return Err(format!("Unexpected initial allocation: {before:?}"));
        }
        for _ in 0..40 {
            if sample(&main).await?.title == "Main app ready" {
                break;
            }
            settle().await;
        }
        if sample(&main).await?.title != "Main app ready" {
            return Err("The primary page fixture did not initialize.".into());
        }
        click(60, 40)?;
        settle().await;
        if sample(&main).await?.title != "Main app click received" {
            return Err(
                "Isolated pointer input did not reach the primary page before opening a browser."
                    .into(),
            );
        }
        main.eval("document.title = 'Main app ready';")
            .map_err(|e| e.to_string())?;
        // Settled samples alone miss a one-frame split during child creation.
        // Record every allocation until the explicit window resize below.
        let allocations = Arc::new(Mutex::new(Vec::new()));
        let observed = allocations.clone();
        let (tx, rx) = tokio::sync::oneshot::channel();
        main.with_webview(move |platform| {
            platform
                .inner()
                .connect_size_allocate(move |_, allocation| {
                    observed
                        .lock()
                        .unwrap()
                        .push((allocation.width(), allocation.height()));
                });
            let _ = tx.send(());
        })
        .map_err(|e| e.to_string())?;
        rx.await.map_err(|e| e.to_string())?;
        let url = fixture()?;
        super::inspector_browser::browser_show(
            app.clone(),
            window.clone(),
            "layout-test".into(),
            180.,
            100.,
            650.,
            450.,
        )
        .await?;
        super::inspector_browser::browser_navigate(app.clone(), "layout-test".into(), url)?;
        let child = app
            .get_webview("inspector-browser-layout-test")
            .ok_or("Child missing")?;
        let mut child_state = sample(&child).await?;
        for _ in 0..40 {
            if child_state.title == "LocalLM browser layout fixture" {
                break;
            }
            settle().await;
            child_state = sample(&child).await?;
        }
        if child_state.title != "LocalLM browser layout fixture" {
            return Err(format!("Native page did not load: {child_state:?}"));
        }
        check_main(&sample(&main).await?, 900, 600)?;
        check_child(&child_state, 180, 100, 650, 450)?;
        click(60, 40)?;
        settle().await;
        if sample(&main).await?.title != "Main app click received" {
            return Err("The browser layer intercepted input outside the panel.".into());
        }
        click(260, 140)?;
        settle().await;
        if child.url().map_err(|e| e.to_string())?.fragment() != Some("clicked") {
            return Err("The browser did not receive pointer input inside its panel.".into());
        }
        // The original bug split the main WebKit allocation vertically here.
        println!(
            "{}",
            serde_json::json!({"case":"open-and-navigate","main":sample(&main).await?,"browser":child_state})
        );
        super::inspector_browser::browser_hide(app.clone(), "layout-test".into())?;
        settle().await;
        if sample(&child).await?.visible {
            return Err("Browser stayed visible after hide".into());
        }
        check_main(&sample(&main).await?, 900, 600)?;
        super::inspector_browser::browser_show(
            app.clone(),
            window.clone(),
            "layout-test".into(),
            20.,
            60.,
            860.,
            520.,
        )
        .await?;
        settle().await;
        check_child(&sample(&child).await?, 20, 60, 860, 520)?;
        super::inspector_browser::browser_hide(app.clone(), "layout-test".into())?;
        super::inspector_browser::browser_show(
            app.clone(),
            window.clone(),
            "second-test".into(),
            200.,
            90.,
            620.,
            430.,
        )
        .await?;
        settle().await;
        check_main(&sample(&main).await?, 900, 600)?;
        if sample(&child).await?.visible {
            return Err("Inactive browser tab became visible".into());
        }
        super::inspector_browser::browser_close(app.clone(), "second-test".into())?;
        if allocations
            .lock()
            .unwrap()
            .iter()
            .any(|size| *size != (900, 600))
        {
            return Err(format!(
                "The main app briefly changed size while switching browser tabs: {:?}",
                allocations.lock().unwrap()
            ));
        }
        super::inspector_browser::browser_show(
            app.clone(),
            window.clone(),
            "layout-test".into(),
            180.,
            100.,
            650.,
            450.,
        )
        .await?;
        window
            .set_size(tauri::LogicalSize::new(1100., 700.))
            .map_err(|e| e.to_string())?;
        settle().await;
        super::inspector_browser::browser_show(
            app.clone(),
            window.clone(),
            "layout-test".into(),
            220.,
            110.,
            800.,
            530.,
        )
        .await?;
        settle().await;
        check_main(&sample(&main).await?, 1100, 700)?;
        check_child(&sample(&child).await?, 220, 110, 800, 530)?;
        if sample(&child).await?.title != "LocalLM browser layout fixture" {
            return Err("Browser lost its page when resized".into());
        }
        super::inspector_browser::browser_close(app.clone(), "layout-test".into())?;
        settle().await;
        check_main(&sample(&main).await?, 1100, 700)?;
        println!(
            "{}",
            serde_json::json!({"case":"hide-restore-tabs-resize-close","passed":true,"main":sample(&main).await?})
        );
        Ok(())
    }
    pub fn start() {
        let exit_code = std::sync::Arc::new(std::sync::atomic::AtomicI32::new(1));
        let result_code = exit_code.clone();
        if std::env::var("LOCALLM_BROWSER_TEST_ISOLATED").as_deref() != Ok("1") {
            eprintln!("Run through scripts/test-browser-layout.mjs to use an isolated display.");
            std::process::exit(2);
        }
        let mut context = tauri::generate_context!();
        context.config_mut().app.windows.clear();
        tauri::Builder::default()
            .setup(move |app| {
                WebviewWindowBuilder::new(
                    app,
                    "main",
                    WebviewUrl::External(
                        format!("{}main", fixture().map_err(std::io::Error::other)?).parse()?,
                    ),
                )
                .title("LocalLM isolated browser layout test")
                .inner_size(900., 600.)
                .position(0., 0.)
                .decorations(false)
                .focused(false)
                .build()?;
                let handle = app.handle().clone();
                let result_code = result_code.clone();
                tauri::async_runtime::spawn(async move {
                    match run(handle.clone()).await {
                        Ok(()) => {
                            result_code.store(0, std::sync::atomic::Ordering::SeqCst);
                            handle.exit(0);
                        }
                        Err(error) => {
                            eprintln!("Browser layout regression: {error}");
                            handle.exit(1);
                        }
                    }
                });
                Ok(())
            })
            .build(context)
            .expect("Cannot start native browser layout test")
            .run_return(|_, _| {});
        std::process::exit(exit_code.load(std::sync::atomic::Ordering::SeqCst));
    }
}

#[cfg(target_os = "linux")]
fn main() {
    smoke::start();
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("This check exercises the Linux GTK browser backend.");
    std::process::exit(2);
}
