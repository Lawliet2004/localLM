//! Saved, re-runnable prompts ("recipes") with optional scheduling.
//!
//! A recipe stores a prompt that enqueues into the task queue — manually via
//! "Run now" or automatically every `interval_minutes`. Because a scheduled
//! run is unattended and may write files, it only fires while execution
//! isolation is enforced; if the mode is later turned down, the run is
//! skipped and the recipe is disabled with a note in the task inbox instead
//! of silently running write-capable work unsandboxed.

use serde::{Deserialize, Serialize};
use tauri::Manager;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recipe {
    pub id: String,
    pub name: String,
    pub prompt: String,
    /// Minutes between scheduled runs; `None` means manual only.
    pub interval_minutes: Option<i64>,
    pub enabled: bool,
    /// Unix seconds. `None` until a schedule is enabled.
    pub next_run_at: Option<i64>,
    pub last_run_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecipeInput {
    pub id: Option<String>,
    pub name: String,
    pub prompt: String,
    pub interval_minutes: Option<i64>,
    pub enabled: bool,
}

const MAX_NAME: usize = 120;
const MAX_PROMPT: usize = 65_536;
const MIN_INTERVAL_MINUTES: i64 = 15;

fn enforced(store: &crate::store::Store) -> bool {
    crate::isolation::settings(store).mode == crate::isolation::IsolationMode::Enforced
}

fn validate(input: &RecipeInput) -> Result<(), String> {
    if input.name.trim().is_empty() || input.name.len() > MAX_NAME {
        return Err(format!("Recipe names are up to {MAX_NAME} characters."));
    }
    if input.prompt.trim().is_empty() || input.prompt.len() > MAX_PROMPT {
        return Err(format!("Recipe prompts are up to {MAX_PROMPT} characters."));
    }
    if let Some(interval) = input.interval_minutes {
        if interval < MIN_INTERVAL_MINUTES {
            return Err(format!("Scheduled recipes run at most every {MIN_INTERVAL_MINUTES} minutes."));
        }
    }
    Ok(())
}

/// Enqueue a recipe's prompt as a task in a fresh conversation.
fn enqueue(store: &crate::store::Store, recipe: &Recipe) -> Result<crate::task_queue::Task, String> {
    let conversation = store.create_conversation()?;
    let _ = store.rename_conversation(&conversation.id, &recipe.name);
    crate::task_queue::enqueue_task(store, &conversation.id, &recipe.prompt, Some("act".into()))
}

#[tauri::command]
pub fn recipes_list(state: tauri::State<'_, crate::AppState>) -> Result<Vec<Recipe>, String> {
    state.database()?.recipes_list()
}

#[tauri::command]
pub fn recipe_save(
    state: tauri::State<'_, crate::AppState>,
    input: RecipeInput,
) -> Result<Recipe, String> {
    let store = state.database()?;
    validate(&input)?;
    let scheduled = input.enabled && input.interval_minutes.is_some();
    if scheduled && !enforced(&store) {
        return Err(
            "Scheduled recipes run unattended and can write files, so they need enforced isolation — enable it under Settings → Execution first."
                .into(),
        );
    }
    let now = crate::store::now();
    let existing = input
        .id
        .as_deref()
        .and_then(|id| store.recipe(id).ok().flatten());
    let recipe = Recipe {
        id: input.id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
        name: input.name.trim().to_string(),
        prompt: input.prompt.trim().to_string(),
        interval_minutes: input.interval_minutes,
        enabled: input.enabled,
        next_run_at: if scheduled {
            input.interval_minutes.map(|minutes| now + minutes * 60)
        } else {
            None
        },
        last_run_at: existing.as_ref().and_then(|r| r.last_run_at),
        created_at: existing.map(|r| r.created_at).unwrap_or(now),
    };
    store.recipe_save(&recipe)?;
    Ok(recipe)
}

#[tauri::command]
pub fn recipe_delete(state: tauri::State<'_, crate::AppState>, id: String) -> Result<(), String> {
    state.database()?.recipe_delete(&id)
}

#[tauri::command]
pub fn recipe_run_now(
    state: tauri::State<'_, crate::AppState>,
    id: String,
) -> Result<crate::task_queue::Task, String> {
    let store = state.database()?;
    let recipe = store.recipe(&id)?.ok_or("Unknown recipe.")?;
    enqueue(&store, &recipe)
}

/// Fire due scheduled recipes — called from the task-queue scheduler tick.
/// Skips (and disables) any due recipe if isolation is no longer enforced so
/// unattended writes never run outside the boundary.
pub fn fire_due(app: &tauri::AppHandle) {
    let state = app.state::<crate::AppState>();
    let Ok(store) = state.database() else { return };
    let now = crate::store::now();
    let due = store.recipes_due(now).unwrap_or_default();
    if due.is_empty() {
        return;
    }
    let sandboxed = enforced(&store);
    for mut recipe in due {
        let interval = recipe.interval_minutes.unwrap_or(MIN_INTERVAL_MINUTES);
        if !sandboxed {
            recipe.enabled = false;
            recipe.next_run_at = None;
            let _ = store.recipe_save(&recipe);
            eprintln!("[recipes] '{}' disabled: isolation is no longer enforced", recipe.name);
            continue;
        }
        match enqueue(&store, &recipe) {
            Ok(task) => {
                eprintln!("[recipes] '{}' queued task {}", recipe.name, task.id);
                crate::task_queue::emit_tasks_changed(app);
            }
            Err(error) => {
                eprintln!("[recipes] '{}' failed to queue: {error}", recipe.name);
            }
        }
        let _ = store.recipe_mark_fired(&recipe.id, now, Some(now + interval * 60));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, crate::store::Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&dir.path().join("app.sqlite")).unwrap();
        (dir, store)
    }

    fn input() -> RecipeInput {
        RecipeInput {
            id: None,
            name: "Tidy".into(),
            prompt: "Remove stale lock files".into(),
            interval_minutes: None,
            enabled: true,
        }
    }

    fn save(store: &crate::store::Store, input: RecipeInput) -> Result<Recipe, String> {
        validate(&input)?;
        let now = crate::store::now();
        let scheduled = input.enabled && input.interval_minutes.is_some();
        if scheduled && !enforced(store) {
            return Err("needs enforced isolation".into());
        }
        let recipe = Recipe {
            id: input.id.clone().unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            name: input.name,
            prompt: input.prompt,
            interval_minutes: input.interval_minutes,
            enabled: input.enabled,
            next_run_at: if scheduled { input.interval_minutes.map(|m| now + m * 60) } else { None },
            last_run_at: None,
            created_at: now,
        };
        store.recipe_save(&recipe)?;
        Ok(recipe)
    }

    #[test]
    fn manual_recipes_roundtrip_and_delete() {
        let (_dir, store) = store();
        let recipe = save(&store, input()).unwrap();
        assert!(recipe.next_run_at.is_none());
        let listed = store.recipes_list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "Tidy");
        store.recipe_delete(&recipe.id).unwrap();
        assert!(store.recipes_list().unwrap().is_empty());
    }

    #[test]
    fn scheduled_recipe_requires_enforced_isolation() {
        let (_dir, store) = store();
        let mut scheduled = input();
        scheduled.interval_minutes = Some(30);
        let error = save(&store, scheduled).unwrap_err();
        assert!(error.contains("enforced"), "{error}");
        // Scheduled (enabled) is gated; the same recipe disabled or manual saves fine.
        let mut manual = input();
        manual.interval_minutes = Some(30);
        manual.enabled = false;
        assert!(save(&store, manual).is_ok());
    }

    #[test]
    fn too_short_an_interval_and_empty_fields_fail() {
        let (_dir, store) = store();
        let mut bad = input();
        bad.interval_minutes = Some(5);
        assert!(save(&store, bad).is_err());
        let mut empty = input();
        empty.prompt = "  ".into();
        assert!(save(&store, empty).is_err());
    }

    #[test]
    fn recipes_due_only_lists_enabled_with_elapsed_schedule() {
        let (_dir, store) = store();
        crate::isolation::save(
            &store,
            &crate::isolation::IsolationSettings {
                mode: crate::isolation::IsolationMode::Enforced,
                ..Default::default()
            },
        )
        .unwrap();
        let mut scheduled = input();
        scheduled.interval_minutes = Some(15);
        let recipe = save(&store, scheduled).unwrap();
        let now = crate::store::now();
        assert!(store.recipes_due(now).unwrap().is_empty());
        assert_eq!(store.recipes_due(now + 15 * 60).unwrap().len(), 1);
        store.recipe_mark_fired(&recipe.id, now, Some(now + 15 * 60)).unwrap();
        assert!(store.recipes_due(now + 15 * 60 - 1).unwrap().is_empty());
    }
}
