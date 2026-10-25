//! A dependency-aware task graph.
//!
//! The flat todo list says what is on the table; it cannot say what has to
//! happen first. A task graph can: a task may depend on other tasks, and a
//! blocked task cannot be started until its dependencies are done. That is
//! the difference between a checklist and a plan an agent can sequence.
//!
//! The flat `todo_*` tools stay: this graph sits beside them and adds the
//! dependency edges, the owner and the blocked-by relation. Tasks are durable
//! per conversation, so a later turn reads the graph back and continues it.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Aliases owned by this module.
pub const ALIASES: [&str; 5] = [
    "task_create",
    "task_update",
    "task_get",
    "task_list",
    "task_delete",
];

/// Whether an alias belongs to this module.
pub fn owns(alias: &str) -> bool {
    ALIASES.contains(&alias)
}

/// The task has not been started.
pub const PENDING: &str = "pending";
/// The task is being worked on.
pub const IN_PROGRESS: &str = "in_progress";
/// The task is done; it no longer blocks anything.
pub const COMPLETED: &str = "completed";
/// The task is explicitly marked blocked.
pub const BLOCKED: &str = "blocked";
/// The task will not be worked on.
pub const CANCELLED: &str = "cancelled";

/// The states a task can be in.
pub const STATES: [&str; 5] = [PENDING, IN_PROGRESS, COMPLETED, BLOCKED, CANCELLED];

/// Longest task title.
const MAX_TITLE_CHARS: usize = 300;
/// Longest task detail.
const MAX_DETAIL_CHARS: usize = 4000;
/// Longest task owner.
const MAX_OWNER_CHARS: usize = 120;
/// Longest task note.
const MAX_NOTE_CHARS: usize = 1000;
/// Longest task id.
const MAX_ID_CHARS: usize = 128;
/// Most dependencies one task may carry.
const MAX_DEPENDENCIES: usize = 64;

/// One stored task.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    /// Stable id handed back to the model.
    pub id: String,
    /// The conversation the task belongs to.
    pub conversation_id: String,
    /// What the task is.
    pub title: String,
    /// Optional longer description.
    pub detail: Option<String>,
    /// Optional owner of the task.
    pub owner: Option<String>,
    /// One of [`STATES`].
    pub state: String,
    /// Optional note about the last change.
    pub note: Option<String>,
    /// Creation timestamp in milliseconds since the epoch.
    pub created_at: i64,
    /// Last change timestamp in milliseconds since the epoch.
    pub updated_at: i64,
}

/// Model-facing tool definitions.
pub fn registry() -> Vec<crate::tool_spec::ToolSpec> {
    let task_output = json!({
        "type":"object",
        "required":["id","title","state","depends_on","blocked","blocked_by","dependents","created_at","updated_at"],
        "properties":{
            "id":{"type":"string"},
            "title":{"type":"string"},
            "detail":{"type":["string","null"]},
            "owner":{"type":["string","null"]},
            "note":{"type":["string","null"]},
            "state":{"type":"string","enum":STATES},
            "depends_on":{"type":"array","items":{"type":"string"},
                "description":"The tasks this one waits for."},
            "blocked":{"type":"boolean",
                "description":"True while an unfinished dependency keeps this task from starting."},
            "blocked_by":{"type":"array","items":{"type":"string"},
                "description":"The unfinished dependencies that block this task."},
            "dependents":{"type":"array","items":{"type":"string"},
                "description":"The tasks that wait for this one."},
            "created_at":{"type":"integer"},
            "updated_at":{"type":"integer"}
        }
    });
    let id_property = json!({
        "type":"string","minLength":1,"maxLength":MAX_ID_CHARS,
        "description":"The id of the task."
    });
    let depends_on_property = json!({
        "type":"array","maxItems":MAX_DEPENDENCIES,
        "items":{"type":"string","minLength":1,"maxLength":MAX_ID_CHARS},
        "description":"The ids of the tasks this one waits for. Replaces the whole dependency set."
    });
    vec![
        crate::tool_spec::ToolSpec::new(
            "task_create",
            "Create a task in this conversation's task graph. A task may depend on other tasks; a blocked task cannot be started until its dependencies are completed.",
            json!({
                "type":"object",
                "required":["title"],
                "properties":{
                    "title":{"type":"string","minLength":1,"maxLength":MAX_TITLE_CHARS,
                        "description":"What the task is."},
                    "detail":{"type":"string","minLength":1,"maxLength":MAX_DETAIL_CHARS,
                        "description":"Optional longer description of the task."},
                    "depends_on":depends_on_property,
                    "owner":{"type":"string","minLength":1,"maxLength":MAX_OWNER_CHARS,
                        "description":"Optional owner of the task."}
                },
                "additionalProperties":false
            }),
        )
        .with_output(task_output.clone()),
        crate::tool_spec::ToolSpec::new(
            "task_update",
            "Update a task: its state, title, detail, owner or dependencies. A task cannot be set to in_progress while a dependency is unfinished; the tool reports the blocking dependencies instead.",
            json!({
                "type":"object",
                "required":["id"],
                "properties":{
                    "id":id_property,
                    "state":{"type":"string","enum":STATES,
                        "description":"The new state of the task."},
                    "title":{"type":"string","minLength":1,"maxLength":MAX_TITLE_CHARS,
                        "description":"A new title for the task."},
                    "detail":{"type":"string","minLength":1,"maxLength":MAX_DETAIL_CHARS,
                        "description":"A new detail for the task."},
                    "owner":{"type":"string","minLength":1,"maxLength":MAX_OWNER_CHARS,
                        "description":"A new owner for the task."},
                    "depends_on":depends_on_property,
                    "note":{"type":"string","minLength":1,"maxLength":MAX_NOTE_CHARS,
                        "description":"Optional record of what changed."}
                },
                "additionalProperties":false
            }),
        )
        .with_output(task_output.clone()),
        crate::tool_spec::ToolSpec::new(
            "task_get",
            "Read one task with its dependencies, dependents and current state.",
            json!({
                "type":"object",
                "required":["id"],
                "properties":{"id":id_property},
                "additionalProperties":false
            }),
        )
        .with_output(task_output.clone()),
        crate::tool_spec::ToolSpec::new(
            "task_list",
            "List this conversation's tasks with their state and blocking dependencies.",
            json!({
                "type":"object",
                "properties":{
                    "state":{"type":"string","enum":STATES,
                        "description":"Only list tasks in this state. Every task when omitted."}
                },
                "additionalProperties":false
            }),
        )
        .with_output(json!({
            "type":"object",
            "required":["tasks"],
            "properties":{"tasks":{"type":"array","items":task_output}},
            "additionalProperties":false
        })),
        crate::tool_spec::ToolSpec::new(
            "task_delete",
            "Delete a task. Dependencies pointing at it are removed.",
            json!({
                "type":"object",
                "required":["id"],
                "properties":{"id":id_property},
                "additionalProperties":false
            }),
        )
        .with_output(json!({
            "type":"object",
            "required":["id","deleted"],
            "properties":{
                "id":{"type":"string"},
                "deleted":{"type":"boolean"}
            },
            "additionalProperties":false
        })),
    ]
}

/// Execute one of the module's tools.
pub async fn execute(
    ctx: &crate::harness::HarnessCtx<'_>,
    alias: &str,
    args: Value,
) -> Result<Value, String> {
    let store = ctx.state.database()?;
    dispatch(&store, &ctx.conversation_id, alias, args)
}

/// Run one tool against an explicit store.
///
/// The harness path and the tests share this: validation, dispatch and the
/// store call are the same code in both.
fn dispatch(
    store: &crate::store::Store,
    conversation_id: &str,
    alias: &str,
    args: Value,
) -> Result<Value, String> {
    let catalog = registry();
    let spec = crate::tool_spec::find(&catalog, alias)
        .ok_or_else(|| format!("Unknown task tool '{alias}'."))?;
    crate::tool_validation::validate(&spec.input, &args)?;
    match alias {
        "task_create" => create(store, conversation_id, &args),
        "task_update" => update(store, conversation_id, &args),
        "task_get" => get(store, conversation_id, &args),
        "task_list" => list(store, conversation_id, &args),
        "task_delete" => remove(store, conversation_id, &args),
        other => Err(format!("Unknown task tool '{other}'.")),
    }
}

/// Create one task in the graph.
fn create(
    store: &crate::store::Store,
    conversation_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let title = required(args.get("title"), "title", MAX_TITLE_CHARS)?;
    let detail = optional(args.get("detail"), "detail", MAX_DETAIL_CHARS)?;
    let owner = optional(args.get("owner"), "owner", MAX_OWNER_CHARS)?;
    let depends_on = dependencies(args.get("depends_on"))?;
    for dependency in &depends_on {
        if store.task(conversation_id, dependency)?.is_none() {
            return Err(format!(
                "Unknown dependency '{dependency}' for this conversation."
            ));
        }
    }
    let now = crate::store::now();
    let task = Task {
        id: new_id(),
        conversation_id: conversation_id.to_string(),
        title,
        detail,
        owner,
        state: PENDING.to_string(),
        note: None,
        created_at: now,
        updated_at: now,
    };
    store.insert_task(&task)?;
    // A new task cannot be part of a cycle: nothing depends on it yet.
    store.set_task_dependencies(&task.id, &depends_on)?;
    view(store, &task)
}

/// Update one task of the graph.
fn update(
    store: &crate::store::Store,
    conversation_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let id = task_id(args)?;
    let mut task = store
        .task(conversation_id, id)?
        .ok_or("Unknown task for this conversation.")?;
    if present(args, "depends_on") {
        let depends_on = dependencies(args.get("depends_on"))?;
        if depends_on.iter().any(|dependency| dependency == id) {
            return Err("A task cannot depend on itself.".into());
        }
        for dependency in &depends_on {
            if store.task(conversation_id, dependency)?.is_none() {
                return Err(format!(
                    "Unknown dependency '{dependency}' for this conversation."
                ));
            }
        }
        let mut graph = dependency_graph(store, conversation_id)?;
        graph.insert(id.to_string(), depends_on.clone());
        if let Some(cycle) = find_cycle(&graph, id) {
            return Err(format!(
                "Dependency cycle detected: {}. A task cannot wait for itself.",
                cycle.join(" -> ")
            ));
        }
        store.set_task_dependencies(id, &depends_on)?;
    }
    if present(args, "state") {
        let state = args
            .get("state")
            .and_then(Value::as_str)
            .ok_or("task_update 'state' must be a string.")?;
        if !STATES.contains(&state) {
            return Err(format!(
                "Unknown task state '{state}'. Use one of: {}.",
                STATES.join(", ")
            ));
        }
        if state == IN_PROGRESS {
            let depends_on = store.task_dependencies(id)?;
            let blocking = unfinished_dependencies(store, &task, &depends_on)?;
            if !blocking.is_empty() {
                return Err(format!(
                    "Task {id} cannot start: it is blocked by unfinished dependencies: {}.",
                    blocking.join(", ")
                ));
            }
        }
        task.state = state.to_string();
    }
    if present(args, "title") {
        task.title = required(args.get("title"), "title", MAX_TITLE_CHARS)?;
    }
    if present(args, "detail") {
        task.detail = optional(args.get("detail"), "detail", MAX_DETAIL_CHARS)?;
    }
    if present(args, "owner") {
        task.owner = optional(args.get("owner"), "owner", MAX_OWNER_CHARS)?;
    }
    if present(args, "note") {
        task.note = optional(args.get("note"), "note", MAX_NOTE_CHARS)?;
    }
    task.updated_at = crate::store::now();
    store.save_task(&task)?;
    view(store, &task)
}

/// Read one task of the graph.
fn get(store: &crate::store::Store, conversation_id: &str, args: &Value) -> Result<Value, String> {
    let id = task_id(args)?;
    let task = store
        .task(conversation_id, id)?
        .ok_or("Unknown task for this conversation.")?;
    view(store, &task)
}

/// List the tasks of the graph, optionally filtered by state.
fn list(store: &crate::store::Store, conversation_id: &str, args: &Value) -> Result<Value, String> {
    let wanted = match args.get("state") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let state = value
                .as_str()
                .ok_or("task_list 'state' must be a string.")?;
            if !STATES.contains(&state) {
                return Err(format!(
                    "Unknown task state '{state}'. Use one of: {}.",
                    STATES.join(", ")
                ));
            }
            Some(state)
        }
    };
    let mut tasks = Vec::new();
    for task in store.tasks(conversation_id)? {
        if wanted.is_some_and(|state| task.state != state) {
            continue;
        }
        tasks.push(view(store, &task)?);
    }
    Ok(json!({ "tasks": tasks }))
}

/// Delete one task of the graph.
fn remove(
    store: &crate::store::Store,
    conversation_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let id = task_id(args)?;
    let deleted = store.delete_task(conversation_id, id)?;
    if !deleted {
        return Err("Unknown task for this conversation.".into());
    }
    Ok(json!({ "id": id, "deleted": true }))
}

/// The model-visible shape of one task, with its dependency edges.
fn view(store: &crate::store::Store, task: &Task) -> Result<Value, String> {
    let depends_on = store.task_dependencies(&task.id)?;
    let dependents = store.task_dependents(&task.id)?;
    let blocked_by = unfinished_dependencies(store, task, &depends_on)?;
    // A task the model marked blocked stays blocked; otherwise the flag
    // follows the dependencies that are still unfinished.
    let blocked = task.state == BLOCKED || (!finished(&task.state) && !blocked_by.is_empty());
    Ok(json!({
        "id": task.id,
        "title": task.title,
        "detail": task.detail,
        "owner": task.owner,
        "note": task.note,
        "state": task.state,
        "depends_on": depends_on,
        "blocked": blocked,
        "blocked_by": blocked_by,
        "dependents": dependents,
        "created_at": task.created_at,
        "updated_at": task.updated_at,
    }))
}

/// Whether a dependency in this state will ever be worked on again.
fn finished(state: &str) -> bool {
    state == COMPLETED || state == CANCELLED
}

/// The dependencies of a task that still have to be finished.
fn unfinished_dependencies(
    store: &crate::store::Store,
    task: &Task,
    depends_on: &[String],
) -> Result<Vec<String>, String> {
    let mut blocking = Vec::new();
    for dependency in depends_on {
        let Some(record) = store.task(&task.conversation_id, dependency)? else {
            continue;
        };
        if !finished(&record.state) {
            blocking.push(record.id);
        }
    }
    Ok(blocking)
}

/// The dependency edges of every task of one conversation.
fn dependency_graph(
    store: &crate::store::Store,
    conversation_id: &str,
) -> Result<std::collections::BTreeMap<String, Vec<String>>, String> {
    let mut graph = std::collections::BTreeMap::new();
    for task in store.tasks(conversation_id)? {
        graph.insert(task.id.clone(), store.task_dependencies(&task.id)?);
    }
    Ok(graph)
}

/// The dependency cycle reachable from `start`, if there is one.
///
/// The reported cycle is the closed path itself, so the model can see which
/// edges have to change.
fn find_cycle(
    graph: &std::collections::BTreeMap<String, Vec<String>>,
    start: &str,
) -> Option<Vec<String>> {
    let mut stack: Vec<String> = Vec::new();
    let mut on_stack: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut done: std::collections::HashSet<String> = std::collections::HashSet::new();
    let repeated = walk(graph, start, &mut stack, &mut on_stack, &mut done)?;
    let position = stack.iter().position(|id| *id == repeated).unwrap_or(0);
    let mut cycle = stack[position..].to_vec();
    cycle.push(repeated);
    Some(cycle)
}

/// Depth-first walk that reports the first node reached twice.
fn walk(
    graph: &std::collections::BTreeMap<String, Vec<String>>,
    node: &str,
    stack: &mut Vec<String>,
    on_stack: &mut std::collections::HashSet<String>,
    done: &mut std::collections::HashSet<String>,
) -> Option<String> {
    if on_stack.contains(node) {
        return Some(node.to_string());
    }
    if done.contains(node) {
        return None;
    }
    stack.push(node.to_string());
    on_stack.insert(node.to_string());
    if let Some(children) = graph.get(node) {
        for child in children {
            if let Some(repeated) = walk(graph, child, stack, on_stack, done) {
                return Some(repeated);
            }
        }
    }
    stack.pop();
    on_stack.remove(node);
    done.insert(node.to_string());
    None
}

/// The id of the task an update addresses.
fn task_id(args: &Value) -> Result<&str, String> {
    args.get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty() && id.chars().count() <= MAX_ID_CHARS)
        .ok_or_else(|| "This task tool needs an 'id' string.".to_string())
}

/// Whether an argument was given at all.
fn present(args: &Value, key: &str) -> bool {
    matches!(args.get(key), Some(value) if !value.is_null())
}

/// One required text field.
fn required(value: Option<&Value>, label: &str, max: usize) -> Result<String, String> {
    let text = value
        .and_then(Value::as_str)
        .ok_or_else(|| format!("'{label}' must be a string."))?;
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.chars().count() > max {
        return Err(format!("'{label}' must be 1-{max} characters."));
    }
    Ok(trimmed.to_string())
}

/// One optional text field.
fn optional(value: Option<&Value>, label: &str, max: usize) -> Result<Option<String>, String> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(value) => {
            let text = value
                .as_str()
                .ok_or_else(|| format!("'{label}' must be a string."))?;
            let trimmed = text.trim();
            if trimmed.is_empty() || trimmed.chars().count() > max {
                return Err(format!("'{label}' must be 1-{max} characters."));
            }
            Ok(Some(trimmed.to_string()))
        }
    }
}

/// The dependency set of a task.
fn dependencies(value: Option<&Value>) -> Result<Vec<String>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let items = value
        .as_array()
        .ok_or("'depends_on' must be an array of task ids.")?;
    if items.len() > MAX_DEPENDENCIES {
        return Err(format!(
            "A task may depend on at most {MAX_DEPENDENCIES} other tasks."
        ));
    }
    let mut dependencies = Vec::new();
    for item in items {
        let id = item
            .as_str()
            .ok_or("'depends_on' entries must be task id strings.")?;
        let trimmed = id.trim();
        if trimmed.is_empty() || trimmed.chars().count() > MAX_ID_CHARS {
            return Err(format!(
                "'depends_on' entries must be 1-{MAX_ID_CHARS} characters."
            ));
        }
        if !dependencies.iter().any(|existing| existing == trimmed) {
            dependencies.push(trimmed.to_string());
        }
    }
    Ok(dependencies)
}

/// A short, stable, human-quotable id.
fn new_id() -> String {
    let raw = uuid::Uuid::new_v4().simple().to_string();
    format!("tsk_{}", &raw[..8])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn conversation_store() -> (crate::store::Store, String) {
        let store = crate::store::Store::open_memory().unwrap();
        let conversation = store.create_conversation().unwrap().id;
        (store, conversation)
    }

    fn create(
        store: &crate::store::Store,
        conversation: &str,
        args: Value,
    ) -> Result<Value, String> {
        dispatch(store, conversation, "task_create", args)
    }

    fn update(
        store: &crate::store::Store,
        conversation: &str,
        args: Value,
    ) -> Result<Value, String> {
        dispatch(store, conversation, "task_update", args)
    }

    fn id_of(task: &Value) -> String {
        task["id"].as_str().unwrap().to_string()
    }

    #[test]
    fn a_new_task_is_pending_and_unblocked() {
        let (store, conversation) = conversation_store();
        let task = create(&store, &conversation, json!({"title":"Write the parser"})).unwrap();
        assert!(id_of(&task).starts_with("tsk_"), "{task}");
        assert_eq!(task["title"], "Write the parser");
        assert_eq!(task["state"], "pending");
        assert_eq!(task["blocked"], false);
        assert_eq!(task["depends_on"].as_array().unwrap().len(), 0);
        assert_eq!(task["owner"], Value::Null);
        assert_eq!(store.tasks(&conversation).unwrap().len(), 1);
    }

    #[test]
    fn a_task_with_an_unfinished_dependency_is_blocked() {
        let (store, conversation) = conversation_store();
        let first = create(&store, &conversation, json!({"title":"First"})).unwrap();
        let second = create(
            &store,
            &conversation,
            json!({"title":"Second", "depends_on":[id_of(&first)]}),
        )
        .unwrap();
        assert_eq!(second["blocked"], true);
        assert_eq!(second["blocked_by"].as_array().unwrap().len(), 1);
        assert_eq!(second["blocked_by"][0], id_of(&first).as_str());
    }

    #[test]
    fn a_blocked_task_cannot_be_started_and_can_be_after_the_dependency_completes() {
        let (store, conversation) = conversation_store();
        let first = create(&store, &conversation, json!({"title":"First"})).unwrap();
        let second = create(
            &store,
            &conversation,
            json!({"title":"Second", "depends_on":[id_of(&first)]}),
        )
        .unwrap();
        let error = update(
            &store,
            &conversation,
            json!({"id": id_of(&second), "state":"in_progress"}),
        )
        .unwrap_err();
        assert!(error.contains(&id_of(&first)), "{error}");

        update(
            &store,
            &conversation,
            json!({"id": id_of(&first), "state":"completed"}),
        )
        .unwrap();
        let started = update(
            &store,
            &conversation,
            json!({"id": id_of(&second), "state":"in_progress"}),
        )
        .unwrap();
        assert_eq!(started["state"], "in_progress");
        assert_eq!(started["blocked"], false);
    }

    #[test]
    fn a_dependency_cycle_is_rejected_and_reported() {
        let (store, conversation) = conversation_store();
        let first = create(&store, &conversation, json!({"title":"First"})).unwrap();
        let second = create(&store, &conversation, json!({"title":"Second"})).unwrap();
        update(
            &store,
            &conversation,
            json!({"id": id_of(&first), "depends_on":[id_of(&second)]}),
        )
        .unwrap();
        let error = update(
            &store,
            &conversation,
            json!({"id": id_of(&second), "depends_on":[id_of(&first)]}),
        )
        .unwrap_err();
        assert!(error.contains("cycle"), "{error}");
        assert!(error.contains(&id_of(&first)), "{error}");
        // The rejected edge was not written.
        let stored = store.task(&conversation, &id_of(&second)).unwrap().unwrap();
        assert!(store.task_dependencies(&stored.id).unwrap().is_empty());
    }

    #[test]
    fn a_task_cannot_depend_on_itself() {
        let (store, conversation) = conversation_store();
        let task = create(&store, &conversation, json!({"title":"Only"})).unwrap();
        let error = update(
            &store,
            &conversation,
            json!({"id": id_of(&task), "depends_on":[id_of(&task)]}),
        )
        .unwrap_err();
        assert!(error.contains("itself"), "{error}");
    }

    #[test]
    fn a_dependency_on_an_unknown_task_is_rejected() {
        let (store, conversation) = conversation_store();
        let error = create(
            &store,
            &conversation,
            json!({"title":"Orphan", "depends_on":["tsk_00000000"]}),
        )
        .unwrap_err();
        assert!(error.contains("tsk_00000000"), "{error}");
        let task = create(&store, &conversation, json!({"title":"Real"})).unwrap();
        let error = update(
            &store,
            &conversation,
            json!({"id": id_of(&task), "depends_on":["tsk_00000000"]}),
        )
        .unwrap_err();
        assert!(error.contains("tsk_00000000"), "{error}");
    }

    #[test]
    fn task_get_returns_dependents_and_blocked_by() {
        let (store, conversation) = conversation_store();
        let first = create(&store, &conversation, json!({"title":"First"})).unwrap();
        let second = create(
            &store,
            &conversation,
            json!({"title":"Second", "depends_on":[id_of(&first)]}),
        )
        .unwrap();
        let read = dispatch(
            &store,
            &conversation,
            "task_get",
            json!({"id": id_of(&first)}),
        )
        .unwrap();
        assert_eq!(read["dependents"].as_array().unwrap().len(), 1);
        assert_eq!(read["dependents"][0], id_of(&second).as_str());
        assert_eq!(read["blocked_by"].as_array().unwrap().len(), 0);
        let read = dispatch(
            &store,
            &conversation,
            "task_get",
            json!({"id": id_of(&second)}),
        )
        .unwrap();
        assert_eq!(read["blocked_by"].as_array().unwrap().len(), 1);
        assert_eq!(read["blocked_by"][0], id_of(&first).as_str());
    }

    #[test]
    fn deleting_a_task_cleans_up_the_dependencies_that_point_at_it() {
        let (store, conversation) = conversation_store();
        let first = create(&store, &conversation, json!({"title":"First"})).unwrap();
        let second = create(
            &store,
            &conversation,
            json!({"title":"Second", "depends_on":[id_of(&first)]}),
        )
        .unwrap();
        let deleted = dispatch(
            &store,
            &conversation,
            "task_delete",
            json!({"id": id_of(&first)}),
        )
        .unwrap();
        assert_eq!(deleted["deleted"], true);
        assert_eq!(deleted["id"], id_of(&first).as_str());
        assert!(store.task(&conversation, &id_of(&first)).unwrap().is_none());
        assert!(store.task_dependencies(&id_of(&second)).unwrap().is_empty());
        let error = dispatch(
            &store,
            &conversation,
            "task_delete",
            json!({"id": id_of(&first)}),
        )
        .unwrap_err();
        assert!(!error.trim().is_empty());
    }

    #[test]
    fn task_list_filters_by_state() {
        let (store, conversation) = conversation_store();
        let first = create(&store, &conversation, json!({"title":"First"})).unwrap();
        create(&store, &conversation, json!({"title":"Second"})).unwrap();
        update(
            &store,
            &conversation,
            json!({"id": id_of(&first), "state":"completed"}),
        )
        .unwrap();
        let all = dispatch(&store, &conversation, "task_list", json!({})).unwrap();
        assert_eq!(all["tasks"].as_array().unwrap().len(), 2);
        let completed = dispatch(
            &store,
            &conversation,
            "task_list",
            json!({"state":"completed"}),
        )
        .unwrap();
        assert_eq!(completed["tasks"].as_array().unwrap().len(), 1);
        assert_eq!(completed["tasks"][0]["id"], id_of(&first).as_str());
        let pending = dispatch(
            &store,
            &conversation,
            "task_list",
            json!({"state":"pending"}),
        )
        .unwrap();
        assert_eq!(pending["tasks"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn tasks_are_scoped_to_their_conversation() {
        let (store, first) = conversation_store();
        let second = store.create_conversation().unwrap().id;
        create(&store, &first, json!({"title":"Mine"})).unwrap();
        let listed = dispatch(&store, &second, "task_list", json!({})).unwrap();
        assert!(listed["tasks"].as_array().unwrap().is_empty());
    }

    #[test]
    fn the_registry_declares_a_result_contract_for_every_tool() {
        let catalog = registry();
        assert_eq!(catalog.len(), 5);
        for spec in &catalog {
            assert!(spec.has_output_contract(), "{}", spec.name);
        }
        assert!(owns("task_create") && owns("task_update") && owns("task_get"));
        assert!(owns("task_list") && owns("task_delete"));
        assert!(!owns("todo_write"));
    }

    #[tokio::test]
    async fn the_harness_path_dispatches_and_satisfies_the_declared_contract() {
        let directory = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(directory.path());
        let conversation = state.database().unwrap().create_conversation().unwrap().id;
        let ctx = crate::harness::HarnessCtx {
            state: &state,
            snapshot: crate::test_support::snapshot(&state, directory.path(), &[]),
            conversation_id: conversation,
            run_id: "tasks-fixture".into(),
            step_id: "tasks-step".into(),
            access_mode: crate::permissions::AccessMode::FullAccess,
            depth: 0,
            preset_id: crate::presets::STANDARD.into(),
            inherit_tools: &[],
            emit: std::sync::Arc::new(|_, _| {}),
        };
        let catalog = registry();
        let contract = |alias: &str| {
            crate::tool_spec::find(&catalog, alias)
                .unwrap()
                .output
                .clone()
                .unwrap()
        };
        let created = execute(&ctx, "task_create", json!({ "title": "Parse the log" }))
            .await
            .unwrap();
        crate::tool_validation::validate_value(&contract("task_create"), &created).unwrap();
        let id = id_of(&created);
        let updated = execute(
            &ctx,
            "task_update",
            json!({ "id": id, "state": "in_progress" }),
        )
        .await
        .unwrap();
        crate::tool_validation::validate_value(&contract("task_update"), &updated).unwrap();
        assert_eq!(updated["state"], "in_progress");
        let read = execute(&ctx, "task_get", json!({ "id": id }))
            .await
            .unwrap();
        crate::tool_validation::validate_value(&contract("task_get"), &read).unwrap();
        assert_eq!(read["id"], id.as_str());
        let listed = execute(&ctx, "task_list", json!({})).await.unwrap();
        crate::tool_validation::validate_value(&contract("task_list"), &listed).unwrap();
        assert_eq!(listed["tasks"].as_array().unwrap().len(), 1);
        let deleted = execute(&ctx, "task_delete", json!({ "id": id }))
            .await
            .unwrap();
        crate::tool_validation::validate_value(&contract("task_delete"), &deleted).unwrap();
        assert_eq!(deleted["deleted"], true);
    }
}
