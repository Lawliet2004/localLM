// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--profile") {
        match headless(&args) {
            Ok(()) => return,
            Err(error) => {
                eprintln!("headless: {error}");
                std::process::exit(2);
            }
        }
    }
    tauri_app_lib::run()
}

/// Headless profile: queue a one-shot task into the desktop database, where
/// the in-app scheduler executes it through the normal agent states.
///
/// Usage: locallm --profile headless "task text" [--wait] [--wait-timeout 600]
/// --wait polls the same database the app writes (WAL), so no HTTP, vault,
/// or async runtime is needed in the CLI. Fails loud when the desktop app
/// has never launched (no database) or the scheduler never picks the task up
/// (app not running / webhook profile mismatch).
fn headless(args: &[String]) -> Result<(), String> {
    let profile = args
        .windows(2)
        .find(|pair| pair[0] == "--profile")
        .map(|pair| pair[1].clone())
        .unwrap_or_default();
    if profile != "headless" {
        return Err(format!("Unknown profile '{profile}'. Only 'headless' is supported."));
    }
    let task = args
        .iter()
        .skip_while(|arg| arg.as_str() != "headless")
        .nth(1)
        .filter(|task| !task.starts_with("--"))
        .ok_or("Usage: locallm --profile headless \"task\" [--wait] [--wait-timeout SECS]")?
        .clone();
    if task.trim().is_empty() || task.len() > 4000 {
        return Err("Headless task must be 1-4000 characters.".into());
    }
    let wait = args.iter().any(|arg| arg == "--wait");
    let wait_timeout: u64 = args
        .windows(2)
        .find(|pair| pair[0] == "--wait-timeout")
        .map(|pair| pair[1].parse().unwrap_or(600))
        .unwrap_or(600)
        .clamp(10, 3600);
    let data = std::env::var_os("APPDATA")
        .map(std::path::PathBuf::from)
        .ok_or("APPDATA is not set; cannot locate desktop data.")?
        .join("app.locallm.desktop");
    let db = data.join("locallm.sqlite");
    if !db.is_file() {
        return Err("Desktop data not found. Launch LocalLM once first.".into());
    }
    let store = tauri_app_lib::store::Store::open(&db)?;
    let schedule = tauri_app_lib::scheduling::Schedule {
        id: format!("sched-{}", uuid::Uuid::new_v4()),
        name: "headless-cli".into(),
        cron: "* * * * *".into(),
        task,
        conversation_id: None,
        allow_write: false,
        enabled: true,
        run_once: true,
        last_run_at: None,
        last_result: None,
        created_at: tauri_app_lib::store::now(),
    };
    tauri_app_lib::scheduling::validate_schedule(&schedule)?;
    store.save_schedule(&schedule)?;
    println!("queued {} (one-shot, reads-only pin)", schedule.id);
    println!("Start the desktop app if it is not running; the scheduler picks the task up within ~30s.");
    if !wait {
        return Ok(());
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(wait_timeout);
    loop {
        let store = tauri_app_lib::store::Store::open(&db)?;
        let current = store
            .schedules()?
            .into_iter()
            .find(|item| item.id == schedule.id)
            .ok_or("Schedule vanished from the database.")?;
        if current.last_run_at.is_some() {
            println!("result: {}", current.last_result.unwrap_or_default());
            return Ok(());
        }
        if std::time::Instant::now() > deadline {
            return Err(format!(
                "Timed out after {wait_timeout}s with no result. Is the desktop app running with the scheduler enabled?"
            ));
        }
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
}
