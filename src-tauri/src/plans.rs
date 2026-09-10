//! Todos, goals, and plan mode as durable logged state (Phase 4, state slice).
//!
//! The model mutates these only through the `todo_write` / `goal_set` /
//! `goal_clear` harness tools (approval + audit apply). Reads are plain IPC
//! for the Trajectory and composer UI.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Todo {
    pub text: String,
    pub status: String,
    pub updated_at: i64,
}

pub const PENDING: &str = "pending";
pub const IN_PROGRESS: &str = "in_progress";
pub const COMPLETED: &str = "completed";

pub fn validate_todos(todos: &[Todo]) -> Result<(), String> {
    if todos.len() > 50 {
        return Err("At most 50 todo items are kept per conversation.".into());
    }
    for todo in todos {
        if todo.text.trim().is_empty() || todo.text.len() > 500 {
            return Err("Each todo needs 1-500 characters of text.".into());
        }
        if ![PENDING, IN_PROGRESS, COMPLETED].contains(&todo.status.as_str()) {
            return Err(format!("Unknown todo status '{}'. Use pending, in_progress, or completed.", todo.status));
        }
    }
    Ok(())
}

pub fn validate_objective(objective: &str) -> Result<(), String> {
    if objective.trim().is_empty() || objective.len() > 4000 {
        return Err("Goal objective must be 1-4000 characters.".into());
    }
    Ok(())
}

/// Parse the `todo_write` tool arguments (full-replacement list).
pub fn parse_todo_write(arguments: &serde_json::Value) -> Result<Vec<Todo>, String> {
    let items = arguments
        .get("todos")
        .and_then(|value| value.as_array())
        .ok_or("todo_write needs a 'todos' array of {text, status}.")?;
    let now = crate::store::now();
    let mut todos = Vec::with_capacity(items.len());
    for item in items {
        let text = item
            .get("text")
            .and_then(|value| value.as_str())
            .ok_or("Each todo needs a 'text' string.")?;
        let status = item.get("status").and_then(|value| value.as_str()).unwrap_or(PENDING);
        todos.push(Todo { text: text.to_string(), status: status.to_string(), updated_at: now });
    }
    validate_todos(&todos)?;
    Ok(todos)
}

/// One-line open-work summary injected into the turn context (logged).
pub fn summary_line(todos: &[Todo], goal: Option<&str>) -> Option<String> {
    let open: Vec<&str> = todos
        .iter()
        .filter(|todo| todo.status != COMPLETED)
        .map(|todo| todo.text.as_str())
        .collect();
    if open.is_empty() && goal.is_none() {
        return None;
    }
    let mut line = String::from("Tracked plan state (update via todo_write; do not edit silently): ");
    if let Some(objective) = goal {
        line.push_str(&format!("goal: {objective}. "));
    }
    if !open.is_empty() {
        line.push_str(&format!("open todos: {}.", open.join("; ")));
    }
    Some(line)
}

#[tauri::command]
pub fn get_todos(state: tauri::State<'_, crate::AppState>, conversation_id: String) -> Result<Vec<Todo>, String> {
    state.database()?.todos(&conversation_id)
}

#[tauri::command]
pub fn get_goal(state: tauri::State<'_, crate::AppState>, conversation_id: String) -> Result<Option<String>, String> {
    state.database()?.goal(&conversation_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_bad_todo_states_and_sizes() {
        assert!(validate_todos(&[]).is_ok());
        assert!(validate_todos(&[Todo { text: "".into(), status: PENDING.into(), updated_at: 0 }]).is_err());
        assert!(validate_todos(&[Todo { text: "x".into(), status: "later".into(), updated_at: 0 }]).is_err());
        assert!(validate_objective("").is_err());
        assert!(validate_objective(&"x".repeat(4001)).is_err());
    }
    #[test]
    fn summary_mentions_goal_and_open_items_only() {
        let todos = vec![
            Todo { text: "done thing".into(), status: COMPLETED.into(), updated_at: 0 },
            Todo { text: "next thing".into(), status: IN_PROGRESS.into(), updated_at: 0 },
        ];
        let line = summary_line(&todos, Some("ship it")).unwrap();
        assert!(line.contains("ship it") && line.contains("next thing") && !line.contains("done thing"));
        assert!(summary_line(&[], None).is_none());
    }
}
