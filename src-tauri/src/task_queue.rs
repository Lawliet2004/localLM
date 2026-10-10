//! Task inbox: persistent queue, steering, pause/cancel, crash recovery.
//!
//! A `tasks` row is the durable record of a unit of work; the scheduler runs
//! queued rows FIFO, one inference lease at a time, through the same
//! [`crate::chat::run_turn`] the interactive `send_message` command uses. The
//! only difference is the sink: interactive sends stream to a Tauri Channel,
//! background tasks broadcast `task-event` so the inbox and any open chat can
//! follow along.

use serde::Serialize;
use serde_json::json;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::{AppState, permissions::InteractionMode, resources};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub conversation_id: String,
    pub content: String,
    pub interaction_mode: String,
    pub status: String,
    pub position: Option<i64>,
    pub run_id: Option<String>,
    pub wait_reason: Option<String>,
    pub error: Option<String>,
    pub next_action: Option<String>,
    pub parent_task_id: Option<String>,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub ended_at: Option<i64>,
}

impl Task {
    pub fn new(
        conversation_id: &str,
        content: &str,
        mode: &str,
        position: Option<i64>,
        parent_task_id: Option<String>,
    ) -> Self {
        Task {
            id: uuid::Uuid::new_v4().to_string(),
            conversation_id: conversation_id.to_string(),
            content: content.to_string(),
            interaction_mode: mode.to_string(),
            status: "queued".to_string(),
            position,
            run_id: None,
            wait_reason: None,
            error: None,
            next_action: None,
            parent_task_id,
            created_at: crate::store::now(),
            started_at: None,
            ended_at: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskTransition {
    pub task_id: String,
    pub from_status: String,
    pub to_status: String,
    pub reason: Option<String>,
    pub at: i64,
}

/// Broadcast every turn event under `task-event` so the sidebar inbox and an
/// open chat can follow tasks that have no interactive Channel attached.
#[derive(Clone)]
pub struct TurnSink {
    channel: Option<tauri::ipc::Channel<crate::chat::ChatEvent>>,
    app: Option<AppHandle>,
    task_id: Option<String>,
    conversation_id: String,
}

impl TurnSink {
    pub fn interactive(
        channel: tauri::ipc::Channel<crate::chat::ChatEvent>,
        app: &AppHandle,
        task_id: Option<String>,
        conversation_id: &str,
    ) -> Self {
        TurnSink {
            channel: Some(channel),
            app: Some(app.clone()),
            task_id,
            conversation_id: conversation_id.to_string(),
        }
    }
    pub fn broadcast(app: &AppHandle, task_id: &str, conversation_id: &str) -> Self {
        TurnSink {
            channel: None,
            app: Some(app.clone()),
            task_id: Some(task_id.to_string()),
            conversation_id: conversation_id.to_string(),
        }
    }
    pub fn send(&self, event: crate::chat::ChatEvent) -> Result<(), tauri::Error> {
        if let Some(app) = &self.app {
            let _ = app.emit(
                "task-event",
                json!({"taskId": self.task_id, "conversationId": self.conversation_id, "event": event}),
            );
        }
        match &self.channel {
            Some(channel) => channel.send(event),
            None => Ok(()),
        }
    }
}

pub fn emit_tasks_changed(app: &AppHandle) {
    let _ = app.emit("tasks-changed", json!({}));
}

fn active(status: &str) -> bool {
    matches!(
        status,
        "queued" | "running" | "waiting_approval" | "waiting_resource"
    )
}

/// Scheduler loop: FIFO by position. Only the head queued task waits on the
/// inference lease — later rows stay `queued`, which is the honest state
/// (they are waiting on this task, not on the model).
pub fn spawn_scheduler(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            crate::recipes::fire_due(&app);
            if let Err(error) = tick(&app).await {
                eprintln!("[tasks] scheduler tick failed: {error}");
            }
            tokio::time::sleep(Duration::from_millis(400)).await;
        }
    });
}

async fn tick(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    let Some(task) = state
        .database()
        .ok()
        .and_then(|store| store.next_queued_task().ok().flatten())
    else {
        return Ok(());
    };
    // The lease wait re-checks the row each tick so a queued pause/cancel
    // stops the wait instead of starting a cancelled task.
    let task_id = task.id.clone();
    let reason = format!(
        "task {} · {}",
        &task.id[..8.min(task.id.len())],
        task.content.chars().take(40).collect::<String>()
    );
    let lease = {
        let app_still = app.clone();
        let still = move || {
            app_still
                .state::<AppState>()
                .database()
                .ok()
                .and_then(|store| store.task(&task_id).ok().flatten())
                .is_some_and(|task| task.status == "queued")
        };
        let app_wait = app.clone();
        let task_for_wait = task.clone();
        resources::acquire_inference(&state, &task.id, &reason, None, still, move |wait_reason| {
            let state = app_wait.state::<AppState>();
            if let Ok(store) = state.database() {
                let _ =
                    store.transition_task(&task_for_wait.id, "waiting_resource", Some(wait_reason));
            }
            let _ = app_wait.emit("tasks-changed", json!({}));
        })
        .await
    };
    let lease = match lease {
        Ok(lease) => lease,
        Err(_) => return Ok(()), // cancelled while waiting — leave the row alone
    };
    // Someone else may have cancelled between the last poll and lease grant.
    let still_queued = state
        .database()
        .and_then(|store| store.task(&task.id))
        .ok()
        .flatten()
        .is_some_and(|task| task.status == "queued" || task.status == "waiting_resource");
    if !still_queued {
        return Ok(());
    }
    {
        let store = state.database()?;
        store.clear_task_wait(&task.id)?;
        store.transition_task(&task.id, "running", None)?;
    }
    emit_tasks_changed(app);
    drive(app, task, lease).await;
    Ok(())
}

async fn drive(app: &AppHandle, task: Task, lease: resources::InferenceLease) {
    let state = app.state::<AppState>();
    let sink = TurnSink::broadcast(app, &task.id, &task.conversation_id);
    let request = crate::chat::TurnRequest {
        conversation_id: task.conversation_id.clone(),
        content: task.content.clone(),
        connector_ids: None,
        connector_tools: None,
        plan_mode: None,
        interaction_mode: Some(task.interaction_mode.clone()),
        sink,
        task_id: Some(task.id.clone()),
        lease: Some(lease),
    };
    let outcome = crate::chat::run_turn(&state, request).await;
    let (to, error) = match &outcome {
        Ok(status) => (status_for_turn(status), None),
        Err(error) => ("failed", Some(error.clone())),
    };
    if let Ok(store) = state.database() {
        let _ = store.transition_task(&task.id, to, error.as_deref());
    }
    emit_tasks_changed(app);
}

/// A queued task row for an interactive `send_message`: it goes straight to
/// `running` inside `run_turn`, but lives in the inbox so the user can pause,
/// steer, or cancel it like any other task.
pub fn register_running_task(
    store: &crate::store::Store,
    conversation_id: &str,
    content: &str,
    mode: &str,
) -> Result<Task, String> {
    let task = Task::new(conversation_id, content, mode, None, None);
    store.insert_task(&task)?;
    // Row inserts as queued; the running transition goes through the same
    // transactional path as every other state change.
    store.transition_task(&task.id, "running", Some("started by send_message"))?;
    Ok(task)
}

/// Crash recovery: a task left mid-flight is interrupted unless its last step
/// has an uncertain write, in which case it must be reviewed before resume.
pub fn recover_tasks(store: &crate::store::Store) -> Result<u32, String> {
    let mut recovered = 0;
    for task in store.active_tasks()? {
        if !matches!(
            task.status.as_str(),
            "running" | "waiting_approval" | "waiting_resource"
        ) {
            continue;
        }
        let calls = classify_run(store, &task)?;
        if let Some((call_id, name)) = calls
            .uncertain
            .iter()
            .find(|(_, name)| !crate::permissions::trusted_read_alias(name))
        {
            store.transition_task(
                &task.id,
                "awaiting_review",
                Some(&format!("Call {call_id} ({name}) may have run — its result is unknown. Mark it done or retry before resuming.")),
            )?;
        } else {
            store.transition_task(
                &task.id,
                "interrupted",
                Some("LocalLM exited while this task was running."),
            )?;
        }
        recovered += 1;
    }
    Ok(recovered)
}

#[derive(Clone, Debug, Default)]
pub struct CallRecovery {
    pub completed: Vec<String>,
    pub not_started: Vec<String>,
    /// call id + tool name: dispatched (event exists) but no result recorded.
    pub uncertain: Vec<(String, String)>,
}

/// Classify a run's tool calls from session events. `tool_call` events are
/// emitted for every model call up front; the first without a `tool_result`
/// is the crash point (uncertain — it may already have run), and later
/// resultless calls never started.
pub fn classify_run(store: &crate::store::Store, task: &Task) -> Result<CallRecovery, String> {
    let Some(run_id) = task.run_id.clone() else {
        return Ok(CallRecovery::default());
    };
    let events = store.session_events(&task.conversation_id, None, None)?;
    let mut calls: Vec<(String, String)> = Vec::new();
    let mut results = std::collections::HashSet::<String>::new();
    for event in events
        .iter()
        .filter(|event| event.run_id.as_deref() == Some(run_id.as_str()))
    {
        match event.event_type.as_str() {
            "tool_call" => {
                let call_id = event
                    .tool_call_id
                    .clone()
                    .or_else(|| event.payload["callId"].as_str().map(String::from))
                    .unwrap_or_default();
                if !call_id.is_empty() {
                    let name = event.payload["name"].as_str().unwrap_or("").to_string();
                    calls.push((call_id, name));
                }
            }
            "tool_result" => {
                if let Some(call_id) = event
                    .tool_call_id
                    .clone()
                    .or_else(|| event.payload["callId"].as_str().map(String::from))
                {
                    results.insert(call_id);
                }
            }
            _ => {}
        }
    }
    let mut recovery = CallRecovery::default();
    let mut seen_uncertain = false;
    for (call_id, name) in calls {
        if results.contains(&call_id) {
            recovery.completed.push(call_id);
        } else if !seen_uncertain {
            seen_uncertain = true;
            recovery.uncertain.push((call_id, name));
        } else {
            recovery.not_started.push(call_id);
        }
    }
    Ok(recovery)
}

fn parse_mode(raw: Option<String>) -> Result<InteractionMode, String> {
    match raw.as_deref() {
        None | Some("act") => Ok(InteractionMode::Act),
        Some("ask") => Ok(InteractionMode::Ask),
        Some("plan") => Ok(InteractionMode::Plan),
        Some(other) => Err(format!("Unknown interaction mode: {other}")),
    }
}

/// Turn status string → task status for the driver's terminal transition.
pub fn status_for_turn(turn_status: &str) -> &'static str {
    match turn_status {
        "complete" => "completed",
        "paused" => "paused",
        "interrupted" => "cancelled",
        _ => "failed",
    }
}

// Command bodies live here as plain fns so tests can drive them without a
// Tauri runtime; the #[tauri::command] wrappers only add event emission.

pub fn enqueue_task(
    store: &crate::store::Store,
    conversation_id: &str,
    content: &str,
    interaction_mode: Option<String>,
) -> Result<Task, String> {
    if content.trim().is_empty() || content.len() > 1_048_576 {
        return Err("Enter a task no larger than 1 MiB.".into());
    }
    let mode = parse_mode(interaction_mode)?;
    if !store.conversation_exists(conversation_id)? {
        return Err("Unknown conversation.".into());
    }
    let position = store.next_task_position()?;
    let task = Task::new(
        conversation_id,
        content.trim(),
        mode.as_str(),
        Some(position),
        None,
    );
    store.insert_task(&task)?;
    Ok(task)
}

pub fn steer_task(store: &crate::store::Store, task_id: &str, text: &str) -> Result<(), String> {
    let text = text.trim();
    if text.is_empty() || text.len() > 65_536 {
        return Err("Enter a steering note no larger than 64 KiB.".into());
    }
    let task = store.task(task_id)?.ok_or("Unknown task.")?;
    if !active(&task.status) && task.status != "paused" {
        return Err("Only a running or paused task can be steered.".into());
    }
    store.add_steering(task_id, text).map(|_| ())
}

/// Pause at the next boundary; also cancel the in-flight generation step —
/// the discarded partial step stays in the transcript as interrupted output.
pub fn pause_task(state: &AppState, task_id: &str) -> Result<(), String> {
    let store = state.database()?;
    let task = store.task(task_id)?.ok_or("Unknown task.")?;
    match task.status.as_str() {
        "queued" | "waiting_resource" => store.transition_task(task_id, "paused", None),
        "paused" => Ok(()),
        status if active(status) => {
            if let Ok(mut control) = state.run_control.lock() {
                crate::agent_run::apply_pause(&mut control);
                crate::agent_run::request_cancel(&mut control);
            }
            state.cancel.send_replace(true);
            Ok(())
        }
        _ => Err("Only an active task can be paused.".into()),
    }
}

pub fn resume_task(store: &crate::store::Store, task_id: &str) -> Result<(), String> {
    let task = store.task(task_id)?.ok_or("Unknown task.")?;
    match task.status.as_str() {
        "paused" => {
            // Stage the checkpoint the next turn must carry; the paused run
            // stays where the turn epilogue left it.
            let _ = crate::chat::stage_paused_resume(store, &task.conversation_id);
            let position = store.next_task_position()?;
            store.move_task_to_queue_tail(task_id, position)?;
            store.transition_task(task_id, "queued", None)
        }
        "interrupted" => {
            // Resume by replaying the original request with an explicit note
            // about what the recovery pass found, so the model does not
            // silently repeat completed or uncertain calls.
            let calls = classify_run(store, &task)?;
            if !calls.uncertain.is_empty() {
                return Err("Review the uncertain call before resuming this task.".into());
            }
            let mut content = task.content.clone();
            if !calls.completed.is_empty() || !calls.not_started.is_empty() {
                content.push_str(&format!(
                    "\n\n[Recovery: the previous attempt was interrupted. Completed calls: {}. Do not repeat them. Never-started calls: {}.]",
                    if calls.completed.is_empty() { "none".into() } else { calls.completed.join(", ") },
                    if calls.not_started.is_empty() { "none".into() } else { calls.not_started.join(", ") }
                ));
            }
            let position = store.next_task_position()?;
            store.reset_task_for_resume(task_id, &content, position)?;
            store.transition_task(task_id, "queued", None)
        }
        _ => Err("Only paused or interrupted tasks can be resumed.".into()),
    }
}

pub fn cancel_task(state: &AppState, task_id: &str) -> Result<(), String> {
    let store = state.database()?;
    let task = store.task(task_id)?.ok_or("Unknown task.")?;
    if !active(&task.status) && task.status != "paused" {
        return Err("This task is already finished.".into());
    }
    if task.status != "queued" && task.status != "paused" {
        if let Ok(mut control) = state.run_control.lock() {
            crate::agent_run::request_cancel(&mut control);
        }
        state.cancel.send_replace(true);
    }
    store.transition_task(task_id, "cancelled", None)
}

pub fn retry_task(store: &crate::store::Store, task_id: &str) -> Result<Task, String> {
    let task = store.task(task_id)?.ok_or("Unknown task.")?;
    if active(&task.status) {
        return Err("This task is still running.".into());
    }
    let position = store.next_task_position()?;
    let retry = Task::new(
        &task.conversation_id,
        &task.content,
        &task.interaction_mode,
        Some(position),
        Some(task.id.clone()),
    );
    store.insert_task(&retry)?;
    Ok(retry)
}

pub fn branch_task(
    store: &crate::store::Store,
    task_id: &str,
    from_seq: u64,
) -> Result<Task, String> {
    let task = store.task(task_id)?.ok_or("Unknown task.")?;
    // Reuse the session fork so the branched task keeps parent provenance.
    let (forked, _) = store.fork_session_events(&task.conversation_id, from_seq)?;
    let position = store.next_task_position()?;
    let branch = Task::new(
        &forked.id,
        &task.content,
        &task.interaction_mode,
        Some(position),
        Some(task.id.clone()),
    );
    store.insert_task(&branch)?;
    Ok(branch)
}

pub fn resolve_uncertain(
    store: &crate::store::Store,
    task_id: &str,
    _call_id: &str,
    resolution: &str,
) -> Result<(), String> {
    let task = store.task(task_id)?.ok_or("Unknown task.")?;
    if task.status != "awaiting_review" {
        return Err("This task is not waiting for review.".into());
    }
    match resolution {
        // 'done' accepts the call as executed; the task may then be resumed.
        "done" => store.transition_task(
            task_id,
            "interrupted",
            Some("Uncertain call marked done by the user."),
        ),
        // 'retry' clears review so resume re-queues the task; the user
        // explicitly chose to let the uncertain call be attempted again.
        "retry" => store.transition_task(
            task_id,
            "interrupted",
            Some("Uncertain call marked for retry by the user."),
        ),
        _ => Err("Resolution must be 'done' or 'retry'.".into()),
    }
}

#[tauri::command]
pub fn tasks_list(
    state: State<'_, AppState>,
    statuses: Option<Vec<String>>,
) -> Result<Vec<Task>, String> {
    let store = state.database()?;
    let filters: Vec<String> = statuses
        .unwrap_or_default()
        .into_iter()
        .filter(|status| !status.is_empty())
        .collect();
    store.tasks(&filters.iter().map(String::as_str).collect::<Vec<_>>())
}

#[tauri::command]
pub fn task_enqueue(
    app: AppHandle,
    state: State<'_, AppState>,
    conversation_id: String,
    content: String,
    interaction_mode: Option<String>,
) -> Result<Task, String> {
    let task = enqueue_task(
        &*state.database()?,
        &conversation_id,
        &content,
        interaction_mode,
    )?;
    emit_tasks_changed(&app);
    Ok(task)
}

#[tauri::command]
pub fn task_steer(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
    text: String,
) -> Result<(), String> {
    steer_task(&*state.database()?, &task_id, &text)?;
    emit_tasks_changed(&app);
    Ok(())
}

#[tauri::command]
pub fn task_pause(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<(), String> {
    pause_task(&state, &task_id)?;
    emit_tasks_changed(&app);
    Ok(())
}

#[tauri::command]
pub fn task_resume(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<(), String> {
    resume_task(&*state.database()?, &task_id)?;
    emit_tasks_changed(&app);
    Ok(())
}

#[tauri::command]
pub fn task_cancel(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<(), String> {
    cancel_task(&state, &task_id)?;
    emit_tasks_changed(&app);
    Ok(())
}

#[tauri::command]
pub fn task_retry(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Task, String> {
    let task = retry_task(&*state.database()?, &task_id)?;
    emit_tasks_changed(&app);
    Ok(task)
}

#[tauri::command]
pub fn task_branch(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
    from_seq: u64,
) -> Result<Task, String> {
    let task = branch_task(&*state.database()?, &task_id, from_seq)?;
    emit_tasks_changed(&app);
    Ok(task)
}

#[tauri::command]
pub fn task_resolve_uncertain(
    app: AppHandle,
    state: State<'_, AppState>,
    task_id: String,
    call_id: String,
    resolution: String,
) -> Result<(), String> {
    resolve_uncertain(&*state.database()?, &task_id, &call_id, &resolution)?;
    emit_tasks_changed(&app);
    Ok(())
}

#[tauri::command]
pub fn task_transitions(
    state: State<'_, AppState>,
    task_id: String,
) -> Result<Vec<TaskTransition>, String> {
    state.database()?.task_transitions(&task_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, crate::store::Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&dir.path().join("app.sqlite")).unwrap();
        (dir, store)
    }

    #[test]
    fn queue_is_fifo_and_never_touches_the_running_task() {
        let (_dir, store) = store();
        let conv = store.create_conversation().unwrap();
        let running = register_running_task(&store, &conv.id, "running work", "act").unwrap();
        let first = enqueue_task(&store, &conv.id, "first", None).unwrap();
        let second = enqueue_task(&store, &conv.id, "second", None).unwrap();
        let third = enqueue_task(&store, &conv.id, "third", None).unwrap();
        assert!(first.position < second.position && second.position < third.position);
        // The head of the queue is the earliest follow-up; the running task
        // is untouched — new work never alters an active row.
        assert_eq!(store.next_queued_task().unwrap().unwrap().id, first.id);
        assert_eq!(
            store.running_task_for(&conv.id).unwrap().unwrap().id,
            running.id
        );
        store.transition_task(&first.id, "running", None).unwrap();
        assert_eq!(store.next_queued_task().unwrap().unwrap().id, second.id);
    }

    #[test]
    fn transitions_are_persisted_in_order() {
        let (_dir, store) = store();
        let conv = store.create_conversation().unwrap();
        let task = enqueue_task(&store, &conv.id, "work", None).unwrap();
        store.transition_task(&task.id, "running", None).unwrap();
        store.transition_task(&task.id, "completed", None).unwrap();
        let history = store.task_transitions(&task.id).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(
            (
                history[0].from_status.as_str(),
                history[0].to_status.as_str()
            ),
            ("queued", "running")
        );
        assert_eq!(
            (
                history[1].from_status.as_str(),
                history[1].to_status.as_str()
            ),
            ("running", "completed")
        );
        assert!(store.task(&task.id).unwrap().unwrap().ended_at.is_some());
    }

    #[test]
    fn steering_is_claimed_exactly_once_across_reopen() {
        let (dir, store) = store();
        let conv = store.create_conversation().unwrap();
        let task = register_running_task(&store, &conv.id, "work", "act").unwrap();
        steer_task(&store, &task.id, "first note").unwrap();
        steer_task(&store, &task.id, "second note").unwrap();
        assert_eq!(
            store.claim_steering(&task.id).unwrap(),
            vec!["first note".to_string(), "second note".to_string()]
        );
        assert!(store.claim_steering(&task.id).unwrap().is_empty());
        // A restart cannot deliver them again: applied_at is durably written.
        let reopened = crate::store::Store::open(&dir.path().join("app.sqlite")).unwrap();
        assert!(reopened.claim_steering(&task.id).unwrap().is_empty());
        assert_eq!(reopened.unapplied_steering(&task.id).unwrap(), 0);
    }

    #[test]
    fn steering_rejects_finished_and_missing_tasks() {
        let (_dir, store) = store();
        let conv = store.create_conversation().unwrap();
        let task = enqueue_task(&store, &conv.id, "work", None).unwrap();
        store.transition_task(&task.id, "cancelled", None).unwrap();
        assert!(steer_task(&store, &task.id, "late").is_err());
        assert!(steer_task(&store, "missing", "note").is_err());
    }

    fn test_app_state(dir: &std::path::Path) -> AppState {
        crate::web_search::tests::test_state(dir)
    }

    #[test]
    fn pause_marks_queued_tasks_and_cancels_in_flight_generation() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_app_state(dir.path());
        let conv = state.database().unwrap().create_conversation().unwrap();
        let queued = enqueue_task(&state.database().unwrap(), &conv.id, "queued", None).unwrap();
        pause_task(&state, &queued.id).unwrap();
        assert_eq!(
            state
                .database()
                .unwrap()
                .task(&queued.id)
                .unwrap()
                .unwrap()
                .status,
            "paused"
        );
        resume_task(&state.database().unwrap(), &queued.id).unwrap();
        assert_eq!(
            state
                .database()
                .unwrap()
                .task(&queued.id)
                .unwrap()
                .unwrap()
                .status,
            "queued"
        );
        // A running task: pause asks for the next boundary AND cancels the
        // in-flight generation step (the partial step is discarded).
        let running =
            register_running_task(&state.database().unwrap(), &conv.id, "running", "act").unwrap();
        let generation = crate::agent_run::begin_run(&mut state.run_control.lock().unwrap());
        pause_task(&state, &running.id).unwrap();
        let control = state.run_control.lock().unwrap();
        assert!(control.pause);
        assert!(crate::agent_run::cancel_matches(
            control.cancel_generation,
            generation
        ));
        drop(control);
        assert!(*state.cancel.borrow());
    }

    #[test]
    fn cancel_queued_and_running_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_app_state(dir.path());
        let conv = state.database().unwrap().create_conversation().unwrap();
        let queued = enqueue_task(&state.database().unwrap(), &conv.id, "queued", None).unwrap();
        cancel_task(&state, &queued.id).unwrap();
        assert_eq!(
            state
                .database()
                .unwrap()
                .task(&queued.id)
                .unwrap()
                .unwrap()
                .status,
            "cancelled"
        );
        let running =
            register_running_task(&state.database().unwrap(), &conv.id, "running", "act").unwrap();
        let generation = crate::agent_run::begin_run(&mut state.run_control.lock().unwrap());
        cancel_task(&state, &running.id).unwrap();
        assert_eq!(
            state
                .database()
                .unwrap()
                .task(&running.id)
                .unwrap()
                .unwrap()
                .status,
            "cancelled"
        );
        assert!(crate::agent_run::cancel_matches(
            state.run_control.lock().unwrap().cancel_generation,
            generation
        ));
        assert!(cancel_task(&state, &queued.id).is_err()); // already finished
    }

    #[test]
    fn retry_creates_a_new_task_and_branch_keeps_provenance() {
        let (_dir, store) = store();
        let conv = store.create_conversation().unwrap();
        let task = enqueue_task(&store, &conv.id, "work", None).unwrap();
        store
            .transition_task(&task.id, "failed", Some("boom"))
            .unwrap();
        let retry = retry_task(&store, &task.id).unwrap();
        assert_eq!(retry.status, "queued");
        assert_eq!(retry.parent_task_id.as_deref(), Some(task.id.as_str()));
        let seq = store
            .append_session_event(&crate::sessions::SessionEvent {
                id: uuid::Uuid::new_v4().to_string(),
                conversation_id: conv.id.clone(),
                run_id: None,
                seq: 0,
                step_id: None,
                tool_call_id: None,
                event_type: "user_msg".into(),
                payload: serde_json::json!({"role":"user","content":"hi"}),
                ignorable: false,
                created_at: crate::store::now(),
            })
            .unwrap();
        let branch = branch_task(&store, &task.id, seq).unwrap();
        assert_ne!(branch.conversation_id, conv.id);
        assert_eq!(branch.parent_task_id.as_deref(), Some(task.id.as_str()));
    }

    #[test]
    fn recovery_marks_uncertain_writes_for_review_and_reads_done() {
        let (_dir, store) = store();
        let conv = store.create_conversation().unwrap();
        let run_id = "run-recover".to_string();
        store
            .save_run(&crate::agent_run::RunRecord {
                id: run_id.clone(),
                conversation_id: conv.id.clone(),
                status: crate::agent_run::RunState::ExecutingTools,
                model_provider: None,
                model_id: None,
                checkpoint: None,
                error: None,
                created_at: crate::store::now(),
                updated_at: crate::store::now(),
            })
            .unwrap();
        let mut task = register_running_task(&store, &conv.id, "work", "act").unwrap();
        store.set_task_run(&task.id, &run_id).unwrap();
        task.run_id = Some(run_id.clone());
        // call-1 completed, call-2 started a WRITE and vanished (uncertain),
        // call-3 was emitted but never started.
        crate::chat::emit_session_event(
            &store,
            &conv.id,
            Some(&run_id),
            Some("step-0"),
            Some("call-1"),
            "tool_call",
            serde_json::json!({"callId":"call-1","name":"file_read","arguments":{}}),
            false,
        );
        crate::chat::emit_session_event(
            &store,
            &conv.id,
            Some(&run_id),
            Some("step-0"),
            Some("call-1"),
            "tool_result",
            serde_json::json!({"callId":"call-1","name":"file_read","decision":"allowed","result":{}}),
            false,
        );
        crate::chat::emit_session_event(
            &store,
            &conv.id,
            Some(&run_id),
            Some("step-0"),
            Some("call-2"),
            "tool_call",
            serde_json::json!({"callId":"call-2","name":"file_write","arguments":{}}),
            false,
        );
        crate::chat::emit_session_event(
            &store,
            &conv.id,
            Some(&run_id),
            Some("step-0"),
            Some("call-3"),
            "tool_call",
            serde_json::json!({"callId":"call-3","name":"file_read","arguments":{}}),
            false,
        );
        let calls = classify_run(&store, &task).unwrap();
        assert_eq!(calls.completed, vec!["call-1".to_string()]);
        assert_eq!(
            calls.uncertain,
            vec![("call-2".to_string(), "file_write".to_string())]
        );
        assert_eq!(calls.not_started, vec!["call-3".to_string()]);
        recover_tasks(&store).unwrap();
        let recovered = store.task(&task.id).unwrap().unwrap();
        assert_eq!(recovered.status, "awaiting_review");
        assert!(recovered.next_action.unwrap().contains("call-2"));
        // The user resolves the review before resume is allowed.
        resolve_uncertain(&store, &task.id, "call-2", "done").unwrap();
        assert_eq!(store.task(&task.id).unwrap().unwrap().status, "interrupted");
        // Read-only uncertainty is a plain interruption — review is for
        // calls that could have mutated state.
        let task2 = register_running_task(&store, &conv.id, "reads only", "act").unwrap();
        let run2 = "run-recover-2";
        store
            .save_run(&crate::agent_run::RunRecord {
                id: run2.into(),
                conversation_id: conv.id.clone(),
                status: crate::agent_run::RunState::ExecutingTools,
                model_provider: None,
                model_id: None,
                checkpoint: None,
                error: None,
                created_at: crate::store::now(),
                updated_at: crate::store::now(),
            })
            .unwrap();
        store.set_task_run(&task2.id, run2).unwrap();
        crate::chat::emit_session_event(
            &store,
            &conv.id,
            Some(run2),
            Some("step-0"),
            Some("call-r"),
            "tool_call",
            serde_json::json!({"callId":"call-r","name":"file_read","arguments":{}}),
            false,
        );
        recover_tasks(&store).unwrap();
        assert_eq!(
            store.task(&task2.id).unwrap().unwrap().status,
            "interrupted"
        );
    }

    #[test]
    fn resume_interrupted_replays_content_with_completed_calls_noted() {
        let (_dir, store) = store();
        let conv = store.create_conversation().unwrap();
        let run_id = "run-interrupted";
        store
            .save_run(&crate::agent_run::RunRecord {
                id: run_id.into(),
                conversation_id: conv.id.clone(),
                status: crate::agent_run::RunState::ExecutingTools,
                model_provider: None,
                model_id: None,
                checkpoint: None,
                error: None,
                created_at: crate::store::now(),
                updated_at: crate::store::now(),
            })
            .unwrap();
        let task = register_running_task(&store, &conv.id, "original request", "act").unwrap();
        store.set_task_run(&task.id, run_id).unwrap();
        crate::chat::emit_session_event(
            &store,
            &conv.id,
            Some(run_id),
            Some("step-0"),
            Some("call-1"),
            "tool_call",
            serde_json::json!({"callId":"call-1","name":"file_read","arguments":{}}),
            false,
        );
        crate::chat::emit_session_event(
            &store,
            &conv.id,
            Some(run_id),
            Some("step-0"),
            Some("call-1"),
            "tool_result",
            serde_json::json!({"callId":"call-1","name":"file_read","decision":"allowed","result":{}}),
            false,
        );
        store
            .transition_task(&task.id, "interrupted", None)
            .unwrap();
        resume_task(&store, &task.id).unwrap();
        let resumed = store.task(&task.id).unwrap().unwrap();
        assert_eq!(resumed.status, "queued");
        assert!(resumed.content.contains("original request"));
        assert!(resumed.content.contains("call-1"));
        assert!(resumed.content.contains("Do not repeat them"));
        assert!(resumed.run_id.is_none());
    }

    #[test]
    fn unresolved_uncertain_blocks_resume() {
        let (_dir, store) = store();
        let conv = store.create_conversation().unwrap();
        let run_id = "run-uncertain";
        store
            .save_run(&crate::agent_run::RunRecord {
                id: run_id.into(),
                conversation_id: conv.id.clone(),
                status: crate::agent_run::RunState::ExecutingTools,
                model_provider: None,
                model_id: None,
                checkpoint: None,
                error: None,
                created_at: crate::store::now(),
                updated_at: crate::store::now(),
            })
            .unwrap();
        let task = register_running_task(&store, &conv.id, "work", "act").unwrap();
        store.set_task_run(&task.id, run_id).unwrap();
        crate::chat::emit_session_event(
            &store,
            &conv.id,
            Some(run_id),
            Some("step-0"),
            Some("call-1"),
            "tool_call",
            serde_json::json!({"callId":"call-1","name":"file_write","arguments":{}}),
            false,
        );
        store
            .transition_task(&task.id, "interrupted", None)
            .unwrap();
        assert!(resume_task(&store, &task.id).is_err());
    }
}
