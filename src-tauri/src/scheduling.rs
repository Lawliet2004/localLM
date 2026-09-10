//! Scheduling + webhook ingress + headless support (Phase 7).
//!
//! Cron-like tasks persist in SQLite and run inside the desktop app. The
//! webhook/API listener binds loopback only, requires a vault bearer token,
//! and bounds every body. Anything it triggers runs through the same agent
//! states, approval policy, and audit as interactive turns.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use tauri::Manager;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    pub id: String,
    pub name: String,
    /// Five-field cron: minute hour day-of-month month day-of-week.
    /// Supports `*`, `*/n`, `a-b`, `a,b` per field.
    pub cron: String,
    pub task: String,
    pub conversation_id: Option<String>,
    /// Unattended writes need this AND conversation FullAccess; otherwise the
    /// run is pinned to trusted reads with everything else denied.
    pub allow_write: bool,
    pub enabled: bool,
    pub run_once: bool,
    pub last_run_at: Option<i64>,
    pub last_result: Option<String>,
    pub created_at: i64,
}

pub fn validate_schedule(schedule: &Schedule) -> Result<(), String> {
    if schedule.id.is_empty() || schedule.id.len() > 64 {
        return Err("Schedule id must be 1-64 characters.".into());
    }
    if schedule.name.trim().is_empty() || schedule.name.len() > 120 {
        return Err("Schedule name must be 1-120 characters.".into());
    }
    parse_cron(&schedule.cron)?;
    if schedule.task.trim().is_empty() || schedule.task.len() > 4000 {
        return Err("Scheduled task must be 1-4000 characters.".into());
    }
    if let Some(result) = &schedule.last_result {
        if result.len() > 8192 {
            return Err("Stored schedule result exceeds 8 KiB.".into());
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct Field {
    min: i64,
    max: i64,
}

fn parse_field(text: &str, field: Field) -> Result<Vec<i64>, String> {
    let mut values = Vec::new();
    for part in text.split(',') {
        let (range, step) = match part.split_once('/') {
            Some((range, step)) => (range, step.parse::<i64>().map_err(|_| format!("Bad cron step '{part}'."))?),
            None => (part, 1),
        };
        if step < 1 {
            return Err(format!("Bad cron step '{part}'."));
        }
        let (low, high) = if range == "*" {
            (field.min, field.max)
        } else if let Some((low, high)) = range.split_once('-') {
            (
                low.parse::<i64>().map_err(|_| format!("Bad cron value '{part}'."))?,
                high.parse::<i64>().map_err(|_| format!("Bad cron value '{part}'."))?,
            )
        } else {
            let value = range.parse::<i64>().map_err(|_| format!("Bad cron value '{part}'."))?;
            (value, value)
        };
        if low < field.min || high > field.max || low > high {
            return Err(format!("Cron value '{part}' is out of range."));
        }
        let mut value = low;
        while value <= high {
            values.push(value);
            value += step;
        }
    }
    values.sort_unstable();
    values.dedup();
    Ok(values)
}

struct Cron {
    minutes: Vec<i64>,
    hours: Vec<i64>,
    days: Vec<i64>,
    months: Vec<i64>,
    weekdays: Vec<i64>,
}

fn parse_cron(cron: &str) -> Result<Cron, String> {    let parts: Vec<&str> = cron.split_whitespace().collect();
    if parts.len() != 5 {
        return Err("Cron needs five fields: minute hour day-of-month month day-of-week.".into());
    }
    Ok(Cron {
        minutes: parse_field(parts[0], Field { min: 0, max: 59 })?,
        hours: parse_field(parts[1], Field { min: 0, max: 23 })?,
        days: parse_field(parts[2], Field { min: 1, max: 31 })?,
        months: parse_field(parts[3], Field { min: 1, max: 12 })?,
        weekdays: parse_field(parts[4], Field { min: 0, max: 6 })?,
    })
}

/// Next minute-aligned UTC timestamp strictly after `from_secs` matching the
/// schedule, searched at most ~66 hours ahead (covers monthly patterns).
pub fn cron_next(cron: &str, from_secs: i64) -> Result<i64, String> {
    let parsed = parse_cron(cron)?;
    let mut candidate = (from_secs / 60 + 1) * 60;
    for _ in 0..4000 {
        let (hour, minute, day, month, weekday) = utc_fields(candidate);
        if parsed.minutes.contains(&minute)
            && parsed.hours.contains(&hour)
            && parsed.months.contains(&month)
            && (parsed.days.contains(&day) || parsed.weekdays.contains(&weekday))
        {
            return Ok(candidate);
        }
        candidate += 60;
    }
    Err("No matching cron time in the next 4000 minutes.".into())
}

fn utc_fields(secs: i64) -> (i64, i64, i64, i64, i64) {
    // Days since Unix epoch (1970-01-01 was a Thursday).
    let days = secs.div_euclid(86_400);
    let time_of_day = secs.rem_euclid(86_400);
    let (mut year, mut month, mut day) = (1970i64, 1i64, 1 + days);
    loop {
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let lengths = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        if day <= lengths[(month - 1) as usize] {
            break;
        }
        day -= lengths[(month - 1) as usize];
        month += 1;
        if month > 12 {
            month = 1;
            year += 1;
        }
    }
    let weekday = (days + 4).rem_euclid(7); // 0 = Sunday
    (time_of_day / 3600, time_of_day % 3600 / 60, day, month, weekday)
}

pub fn is_due(schedule: &Schedule, now_secs: i64) -> bool {
    if !schedule.enabled {
        return false;
    }
    match schedule.last_run_at {
        None => true, // Never ran: due immediately (also serves one-shot CLI tasks).
        Some(last) if schedule.run_once => {
            let _ = last;
            false
        }
        Some(last) => cron_next(&schedule.cron, last).map(|next| next <= now_secs).unwrap_or(false),
    }
}

fn webhook_token(vault: &crate::vault::Vault) -> Result<Option<String>, String> {
    Ok(vault
        .load("webhook")?
        .and_then(|bytes| String::from_utf8(bytes).ok()))
}

#[tauri::command]
pub fn list_schedules(state: tauri::State<'_, crate::AppState>) -> Result<Vec<Schedule>, String> {
    state.database()?.schedules()
}

#[tauri::command]
pub async fn save_schedule(state: tauri::State<'_, crate::AppState>, mut schedule: Schedule) -> Result<Schedule, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before changing schedules.")?;
    if schedule.id.is_empty() {
        schedule.id = format!("sched-{}", uuid::Uuid::new_v4());
        schedule.created_at = crate::store::now();
    }
    validate_schedule(&schedule)?;
    state.database()?.save_schedule(&schedule)?;
    Ok(schedule)
}

#[tauri::command]
pub async fn delete_schedule(state: tauri::State<'_, crate::AppState>, id: String) -> Result<bool, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before changing schedules.")?;
    state.database()?.delete_schedule(&id)
}

#[tauri::command]
pub async fn run_schedule_now(state: tauri::State<'_, crate::AppState>, id: String) -> Result<String, String> {
    let schedule = state
        .database()?
        .schedules()?
        .into_iter()
        .find(|item| item.id == id)
        .ok_or("Schedule no longer exists.")?;
    execute_schedule(&state, &schedule).await
}

#[tauri::command]
pub fn webhook_state(state: tauri::State<'_, crate::AppState>) -> Result<serde_json::Value, String> {
    let store = state.database()?;
    let enabled: bool = store.setting("webhook.enabled").unwrap_or(false);
    let port: u16 = store.setting("webhook.port").unwrap_or(4317);
    let has_token = webhook_token(&state.daytona_vault)?.is_some();
    Ok(serde_json::json!({ "enabled": enabled, "port": port, "hasToken": has_token }))
}

#[tauri::command]
pub async fn set_webhook(state: tauri::State<'_, crate::AppState>, enabled: bool, port: Option<u16>) -> Result<serde_json::Value, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before changing webhook settings.")?;
    if let Some(port) = port {
        if !(1024..=65535).contains(&port) {
            return Err("Webhook port must be 1024-65535.".into());
        }
        state.database()?.save_setting("webhook.port", &port)?;
    }
    state.database()?.save_setting("webhook.enabled", &enabled)?;
    // Listener (re)starts on next launch; a running listener keeps serving
    // until restart so in-flight webhook calls are never cut off mid-run.
    drop(_operation);
    webhook_state(state)
}

#[tauri::command]
pub async fn rotate_webhook_token(state: tauri::State<'_, crate::AppState>) -> Result<String, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before rotating the webhook token.")?;
    let mut bytes = [0u8; 32];
    #[cfg(windows)]
    {
        // RandomSource-free token: hash process entropy + time twice.
        use sha2::{Digest, Sha256};
        let seed = format!("{:?}{:?}{}", std::process::id(), std::time::SystemTime::now(), uuid::Uuid::new_v4());
        let first = Sha256::digest(seed.as_bytes());
        let second = Sha256::digest(format!("{first:x}{}", uuid::Uuid::new_v4()).as_bytes());
        bytes.copy_from_slice(&second);
    }
    #[cfg(not(windows))]
    {
        use sha2::{Digest, Sha256};
        let seed = format!("{:?}{:?}{}", std::process::id(), std::time::SystemTime::now(), uuid::Uuid::new_v4());
        bytes.copy_from_slice(&Sha256::digest(seed.as_bytes()));
    }
    let token = bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    state.daytona_vault.save("webhook", token.as_bytes())?;
    Ok(token)
}

/// Execute one schedule: resolve (or create) its conversation, run the task
/// as an unattended child agent, save the reply, record a bounded result.
pub async fn execute_schedule(state: &crate::AppState, schedule: &Schedule) -> Result<String, String> {
    let conversation_id = match &schedule.conversation_id {
        Some(id) => id.clone(),
        None => {
            let store = state.database()?;
            let conversation = store.create_conversation()?;
            let _ = store.rename_conversation(&conversation.id, &format!("Scheduled: {}", schedule.name));
            conversation.id
        }
    };
    let user_row = state.database()?.append_message(&conversation_id, "user", &schedule.task, "complete")?;
    let snapshot = crate::subagents::snapshot_for_conversation(state, &conversation_id).await?;
    let policy = crate::subagents::policy_for_schedule(state, &conversation_id, schedule.allow_write)?;
    let outcome = crate::subagents::run_child_inline(
        state,
        &snapshot,
        crate::subagents::ChildRequest {
            label: format!("schedule:{}", schedule.name),
            prompt: schedule.task.clone(),
            depth: 0,
            max_rounds: 6,
            tool_filter: None,
            persona: None,
            output_schema: None,
            policy,
            parent_run_id: String::new(),
            conversation_id: conversation_id.clone(),
        },
        &[],
    )
    .await;
    let summary = match &outcome {
        Ok((answer, _)) => {
            let _ = state.database()?.append_message(&conversation_id, "assistant", answer, "complete");
            truncate_result(answer)
        }
        Err(error) => {
            let _ = state.database()?.append_message(&conversation_id, "assistant", "", "error");
            truncate_result(&format!("scheduled run failed: {error}; user message kept as {user_row}", user_row = user_row.id))
        }
    };
    state.database()?.record_schedule_result(&schedule.id, &summary)?;
    if schedule.run_once {
        // One-shot consumed: disable so CLI --wait sees a stable result.
        if let Ok(store) = state.database() {
            if let Ok(mut schedules) = store.schedules() {
                if let Some(current) = schedules.iter_mut().find(|item| item.id == schedule.id) {
                    current.enabled = false;
                    let _ = store.save_schedule(current);
                }
            }
        }
    }
    Ok(summary)
}

fn truncate_result(text: &str) -> String {
    const LIMIT: usize = 2048;
    if text.len() <= LIMIT {
        return text.to_string();
    }
    format!("{}… (truncated to 2 KiB; full reply in conversation)", &text[..LIMIT])
}

// ---- Loopback HTTP: webhook ingress + local API for SDKs/CLI ----

struct HttpRequest {
    method: String,
    path: String,
    headers: std::collections::HashMap<String, String>,
    body: Vec<u8>,
}

fn parse_http(head: &str, body: &[u8]) -> Result<HttpRequest, String> {
    let mut lines = head.lines();
    let request_line = lines.next().ok_or("Empty HTTP request.")?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or("Bad HTTP request line.")?.to_string();
    let path = parts.next().ok_or("Bad HTTP request line.")?.to_string();
    if parts.next().is_some_and(|version| version != "HTTP/1.1" && version != "HTTP/1.0") {
        return Err("Only HTTP/1.x requests are served.".into());
    }
    let mut headers = std::collections::HashMap::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        let (name, value) = line.split_once(':').ok_or("Bad HTTP header.")?;
        headers.insert(name.trim().to_lowercase(), value.trim().to_string());
    }
    Ok(HttpRequest { method, path, headers, body: body.to_vec() })
}

fn http_response(status: u16, body: &str) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        match status {
            200 => "OK",
            202 => "Accepted",
            400 => "Bad Request",
            403 => "Forbidden",
            404 => "Not Found",
            _ => "Error",
        },
        body.len()
    )
    .into_bytes()
}

async fn handle_api(app: &tauri::AppHandle, request: HttpRequest) -> Vec<u8> {
    let respond = |status: u16, body: String| http_response(status, &body);
    let state: tauri::State<'_, crate::AppState> = app.state();
    let token = match webhook_token(&state.daytona_vault) {
        Ok(Some(token)) => token,
        _ => return respond(403, r#"{"error":"Webhook token is not configured. Rotate one in Settings."}"#.into()),
    };
    let authorized = request
        .headers
        .get("authorization")
        .is_some_and(|value| value == &format!("Bearer {token}"));
    if !authorized {
        return respond(403, r#"{"error":"Bad or missing bearer token."}"#.into());
    }
    if request.body.len() > 65_536 {
        return respond(400, r#"{"error":"Body exceeds 64 KiB."}"#.into());
    }
    if request.method == "GET" && request.path == "/api/config" {
        let store = match state.database() {
            Ok(store) => store,
            Err(error) => return respond(500, format!(r#"{{"error":"{error}"}}"#)),
        };
        let capabilities = crate::capabilities::list(&store, None).unwrap_or_default();
        let schedules = store.schedules().unwrap_or_default().len();
        return respond(200, serde_json::json!({"preset": "standard", "capabilities": capabilities, "schedules": schedules}).to_string());
    }
    if request.method == "GET" && request.path == "/api/schedules" {
        let schedules = state.database().map(|store| store.schedules().unwrap_or_default());
        let mut schedules = match schedules {
            Ok(schedules) => schedules,
            Err(error) => return respond(500, format!(r#"{{"error":"{error}"}}"#)),
        };
        for schedule in &mut schedules {
            if let Some(result) = &schedule.last_result {
                schedule.last_result = Some(truncate_result(result));
            }
            schedule.task = truncate_result(&schedule.task);
        }
        return respond(200, serde_json::to_string(&schedules).unwrap_or_default());
    }
    if request.method == "POST" {
        let schedule_id = request.path.strip_prefix("/webhook/").map(str::to_string).or_else(|| {
            serde_json::from_slice::<serde_json::Value>(&request.body)
                .ok()
                .and_then(|body| {
                    (request.path == "/api/schedules/run")
                        .then(|| body.get("id")?.as_str().map(str::to_string))
                        .flatten()
                })
        });
        if let Some(id) = schedule_id {
            let store = match state.database() {
                Ok(store) => store,
                Err(error) => return respond(500, format!(r#"{{"error":"{error}"}}"#)),
            };
            let schedule = match store.schedules() {
                Ok(schedules) => schedules.into_iter().find(|item| item.id == id),
                Err(error) => return respond(500, format!(r#"{{"error":"{error}"}}"#)),
            };
            let Some(schedule) = schedule else {
                return respond(404, r#"{"error":"Unknown schedule id."}"#.into());
            };
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let state: tauri::State<'_, crate::AppState> = app.state();
                let _ = execute_schedule(&state, &schedule).await;
            });
            return respond(202, serde_json::json!({"accepted": id}).to_string());
        }
    }
    respond(404, r#"{"error":"Unknown route."}"#.into())
}

async fn serve_connection(app: tauri::AppHandle, mut stream: tokio::net::TcpStream) {
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    // Read until end of headers, bounded at 16 KiB.
    while head.len() < 16_384 {
        match stream.read(&mut byte).await {
            Ok(0) => return,
            Ok(_) => {
                head.extend_from_slice(&byte);
                if head.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => return,
        }
    }
    let split = head.windows(4).position(|window| window == b"\r\n\r\n").map(|index| index + 4);
    let Some(split) = split else {
        let _ = stream.write_all(&http_response(400, r#"{"error":"Headers exceed 16 KiB."}"#)).await;
        return;
    };
    let head_text = String::from_utf8_lossy(&head[..split]).into_owned();
    let content_length: usize = head_text
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.trim().eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse().ok())
        .unwrap_or(0);
    if content_length > 65_536 {
        let _ = stream.write_all(&http_response(400, r#"{"error":"Body exceeds 64 KiB."}"#)).await;
        return;
    }
    let mut body = head[split..].to_vec();
    while body.len() < content_length {
        let mut chunk = vec![0u8; content_length - body.len()];
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(count) => body.extend_from_slice(&chunk[..count]),
        }
    }
    body.truncate(content_length);
    let request = match parse_http(&head_text, &body) {
        Ok(request) => request,
        Err(error) => {
            let _ = stream.write_all(&http_response(400, &format!(r#"{{"error":"{error}"}}"#))).await;
            return;
        }
    };
    let response = handle_api(&app, request).await;
    let _ = stream.write_all(&response).await;
}

/// Serve due schedules every 30s plus the loopback webhook/API listener.
/// Runs for the app lifetime; never panics the host on listener errors.
pub async fn run_background(app: tauri::AppHandle) {
    let mut ticker = tokio::time::interval(Duration::from_secs(30));
    // Start the webhook listener once if enabled at boot.
    {
        let state: tauri::State<'_, crate::AppState> = app.state();
        let (enabled, port) = if let Ok(store) = state.database() {
            (store.setting("webhook.enabled").unwrap_or(false), store.setting("webhook.port").unwrap_or(4317))
        } else {
            (false, 4317)
        };
        if enabled {
            let listener_app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                    while let Ok((stream, _)) = listener.accept().await {
                        serve_connection(listener_app.clone(), stream).await;
                    }
                }
            });
        }
    }
    loop {
        ticker.tick().await;
        let state: tauri::State<'_, crate::AppState> = app.state();
        let due: Vec<Schedule> = state
            .database()
            .map(|store| {
                store
                    .schedules()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|schedule| is_due(schedule, crate::store::now() / 1000))
                    .collect()
            })
            .unwrap_or_default();
        for schedule in due {
            let result = execute_schedule(&state, &schedule).await;
            if result.is_err() {
                let _ = state.database().map(|store| {
                    store.record_schedule_result(&schedule.id, &format!("scheduled run failed: {}", result.unwrap_err()))
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cron_fields_and_next_match() {
        assert!(parse_cron("* * * * *").is_ok());
        assert!(parse_cron("*/15 9-17 * * 1-5").is_ok());
        assert!(parse_cron("nope").is_err());
        assert!(parse_cron("61 * * * *").is_err());
        // 2026-01-01T00:00:00Z = 1767225600; next minute matches * * * * *.
        assert_eq!(cron_next("* * * * *", 1_767_225_600).unwrap(), 1_767_225_660);
        // Weekday-only 09:30 skips the weekend correctly.
        let friday = 1_767_278_400; // 2026-01-02 (Friday) 00:00 UTC
        let next = cron_next("30 9 * * 1-5", friday).unwrap();
        let fields = utc_fields(next);
        assert_eq!((fields.0, fields.1), (9, 30));
    }
    #[test]
    fn due_logic_covers_never_run_disabled_and_one_shot() {
        let base = Schedule {
            id: "s".into(), name: "n".into(), cron: "* * * * *".into(), task: "t".into(),
            conversation_id: None, allow_write: false, enabled: true, run_once: false,
            last_run_at: None, last_result: None, created_at: 0,
        };
        assert!(is_due(&base, 1_767_225_600));
        assert!(!is_due(&Schedule { enabled: false, ..base.clone() }, 1_767_225_600));
        assert!(!is_due(&Schedule { run_once: true, last_run_at: Some(1), ..base.clone() }, 2));
        assert!(validate_schedule(&base).is_ok());
        assert!(validate_schedule(&Schedule { task: "".into(), ..base }).is_err());
    }
    #[test]
    fn http_request_parsing_rejects_garbage() {
        assert!(parse_http("GET /api/config HTTP/1.1\r\nAuthorization: Bearer x\r\n", &[]).is_ok());
        assert!(parse_http("GARBAGE", &[]).is_err());
    }
}
