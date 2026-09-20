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

/// Pull numbered or bulleted steps out of a Phase-1 plan so the UI has a
/// checklist even before the model calls `todo_write`.
pub fn parse_plan_steps(plan_text: &str) -> Vec<Todo> {
    let now = crate::store::now();
    let mut todos = Vec::new();
    for line in plan_text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(raw) = step_text(trimmed) else { continue };
        let text = strip_md(raw);
        if text.len() < 3 || text.len() > 500 {
            continue;
        }
        let status = if todos.is_empty() { IN_PROGRESS } else { PENDING };
        todos.push(Todo {
            text,
            status: status.to_string(),
            updated_at: now,
        });
        if todos.len() >= 50 {
            break;
        }
    }
    todos
}

fn step_text(line: &str) -> Option<&str> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() && bytes[i].is_ascii_digit() {
        i += 1;
    }
    if i > 0 && i < bytes.len() && matches!(bytes[i], b'.' | b')' | b':') {
        return Some(line[i + 1..].trim());
    }
    for prefix in ["- ", "* ", "+ ", "– ", "— "] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some(rest.trim());
        }
    }
    None
}

fn strip_md(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '*' || ch == '_' {
            continue;
        }
        if ch == '`' {
            continue;
        }
        out.push(ch);
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One-line "work only on this" reminder injected while a plan is executing.
pub fn current_task_line(todos: &[Todo]) -> Option<String> {
    if todos.is_empty() {
        return None;
    }
    let total = todos.len();
    let done = todos.iter().filter(|todo| todo.status == COMPLETED).count();
    let current = todos
        .iter()
        .enumerate()
        .find(|(_, todo)| todo.status == IN_PROGRESS)
        .or_else(|| todos.iter().enumerate().find(|(_, todo)| todo.status == PENDING));
    let mut line = format!("Checklist progress: {done}/{total} complete.");
    match current {
        Some((index, todo)) => {
            line.push_str(&format!(
                " Current task (index {index}): {}. Work only on this task. When it is done, mark it completed and start the next pending task. Do not skip ahead.",
                todo.text
            ));
        }
        None => {
            line.push_str(" All tasks are complete. Summarize the answer and do not call more tools.");
        }
    }
    Some(line)
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

#[allow(dead_code)]
fn make_todo(text: &str) -> Result<Todo, String> {
    if text.trim().is_empty() || text.len() > 500 {
        return Err("Each todo needs 1-500 characters of text.".into());
    }
    Ok(Todo { text: text.to_string(), status: PENDING.to_string(), updated_at: crate::store::now() })
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
    let total = todos.len();
    let done = todos.iter().filter(|todo| todo.status == COMPLETED).count();
    let mut line = String::from("Tracked plan state (update via todo_add/todo_update; do not edit silently): ");
    if let Some(objective) = goal {
        line.push_str(&format!("goal: {objective}. "));
    }
    if total > 0 {
        line.push_str(&format!("progress: {done}/{total} complete. "));
    }
    if let Some(current) = todos.iter().find(|todo| todo.status == IN_PROGRESS) {
        line.push_str(&format!("current: {}. ", current.text));
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
        assert!(line.contains("todo_add/todo_update"));
        assert!(summary_line(&[], None).is_none());
    }
    #[test]
    fn todo_factory_rejects_blank_and_oversize_text() {
        assert!(make_todo("write tests").is_ok());
        assert!(make_todo("").is_err());
        assert!(make_todo(&"x".repeat(501)).is_err());
    }

    #[test]
    fn parse_plan_steps_takes_numbered_and_bulleted_lines() {
        let plan = "# Plan\n\n1. **Search** fusion yield papers\n2. Open the top source\n3. Calculate the yield\n- skip headings above\n\nThen we are done.";
        let todos = parse_plan_steps(plan);
        assert_eq!(todos.len(), 4);
        assert_eq!(todos[0].text, "Search fusion yield papers");
        assert_eq!(todos[0].status, IN_PROGRESS);
        assert_eq!(todos[1].text, "Open the top source");
        assert_eq!(todos[1].status, PENDING);
        assert_eq!(todos[2].status, PENDING);
        assert_eq!(todos[3].text, "skip headings above");
    }

    #[test]
    fn parse_plan_steps_ignores_prose_without_markers() {
        assert!(parse_plan_steps("I will look this up and then calculate.").is_empty());
        assert!(parse_plan_steps("").is_empty());
    }

    #[test]
    fn current_task_line_names_the_in_progress_item() {
        let todos = vec![
            Todo { text: "done thing".into(), status: COMPLETED.into(), updated_at: 0 },
            Todo { text: "open the paper".into(), status: IN_PROGRESS.into(), updated_at: 0 },
            Todo { text: "verify".into(), status: PENDING.into(), updated_at: 0 },
        ];
        let line = current_task_line(&todos).unwrap();
        assert!(line.contains("1/3 complete"));
        assert!(line.contains("index 1"));
        assert!(line.contains("open the paper"));
        assert!(line.contains("Do not skip ahead"));
        let finished = vec![Todo { text: "done".into(), status: COMPLETED.into(), updated_at: 0 }];
        assert!(current_task_line(&finished).unwrap().contains("All tasks are complete"));
        assert!(current_task_line(&[]).is_none());
    }
}
