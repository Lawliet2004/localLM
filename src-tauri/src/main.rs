// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(feature = "harness-eval")]
    if std::env::args().nth(1).as_deref() == Some("--harness-eval") {
        if let Err(error) = tauri_app_lib::run_harness_eval() { eprintln!("{error}"); std::process::exit(1); }
        return;
    }
    tauri_app_lib::run()
}
