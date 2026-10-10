//! Local-first memory bank backed by SQLite.
//!
//! Facts are scoped `user`, `project:<id-or-path>`, or `conversation:<id>`.
//! The harness recalls only scopes matching the current conversation and
//! injects them verbatim (logged as `context_injection`); the model teaches
//! via `memory_teach`. Memory text is context — it can never change
//! permissions, interaction mode, or tool availability. No cloud sync;
//! nothing leaves the device except through the provider carrying the prompt.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const SCOPE_USER: &str = "user";
const AUTO_RETAIN_KEY: &str = "memory.autoRetain";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Fact {
    pub id: String,
    pub scope: String,
    pub fact: String,
    /// user | agent | survey — who created it.
    pub origin: String,
    /// Why it was stored (agent rationale or user note), when given.
    #[serde(default)]
    pub reason: Option<String>,
    /// Which turn taught it, when it was agent-written.
    #[serde(default)]
    pub source_conversation_id: Option<String>,
    /// File-derived facts: path + content hash + branch at capture time.
    #[serde(default)]
    pub source_path: Option<String>,
    #[serde(default)]
    pub source_hash: Option<String>,
    #[serde(default)]
    pub source_branch: Option<String>,
    /// Pinned facts recall first; excluded facts are never injected.
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub excluded: bool,
    /// Computed at read time: the source file changed or the project moved
    /// to another branch since the fact was captured. Never stored.
    #[serde(default)]
    pub stale: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Anything one conversation turn may inject: a rule file, the repo map, or
/// a memory fact. `context_preview` and `assemble_turn` share this source, so
/// what the UI lists is exactly what the model saw.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextItem {
    /// rules | repo-map | memory | pinned
    pub kind: String,
    pub label: String,
    pub text: String,
    /// Rough estimate (chars / 4).
    pub tokens: usize,
    pub reason: String,
    pub source: Option<String>,
    pub id: Option<String>,
}

/// Canonical form for a scope argument from a tool call or the UI. Bare
/// `project`/`conversation` resolve against this conversation; a bare path is
/// a project scope keyed by that path (what old `memory_facts` rows used).
pub fn resolve_scope(
    store: &crate::store::Store,
    conversation_id: &str,
    raw: &str,
) -> Result<String, String> {
    let raw = raw.trim();
    match raw {
        // Bare scope = "the workspace" (project) or — with no workspace, as
        // before — the shared user memory.
        "" | "project" => {
            if let Some(key) = project_key(store, conversation_id)? {
                Ok(format!("project:{key}"))
            } else if raw.is_empty() {
                Ok(SCOPE_USER.into())
            } else {
                Err("This conversation has no project or workspace to scope memory to.".into())
            }
        }
        "conversation" | "this" => Ok(format!("conversation:{conversation_id}")),
        "user" | "global" | "me" => Ok(SCOPE_USER.into()),
        scope
            if scope.starts_with("project:")
                || scope.starts_with("conversation:")
                || scope == SCOPE_USER =>
        {
            Ok(scope.into())
        }
        other if other.len() <= 1024 => Ok(format!("project:{}", other)),
        _ => Err("Memory scope must be 'user', 'project', 'conversation', or a path.".into()),
    }
}

/// Validate an already-canonical scope string (`user`, `project:<key>`,
/// `conversation:<id>`). Anything else must go through `resolve_scope`.
pub fn validate_scope(scope: &str) -> Result<&str, String> {
    let ok = scope == SCOPE_USER
        || (scope.starts_with("project:") && scope.len() > "project:".len())
        || (scope.starts_with("conversation:") && scope.len() > "conversation:".len());
    if !ok || scope.len() > 1024 {
        return Err("Memory scope must be 'user', 'project:<key>', or 'conversation:<id>'.".into());
    }
    Ok(scope)
}

/// The conversation's project key — the workspace-index project id when one
/// exists, else the project/workspace path (matches pre-id fact rows).
fn project_key(
    store: &crate::store::Store,
    conversation_id: &str,
) -> Result<Option<String>, String> {
    let data = crate::workspace_ui::index(store)?;
    if let Some(meta) = data.tasks.get(conversation_id) {
        if let Some(id) = &meta.project_id {
            if let Some(project) = data.projects.iter().find(|p| &p.id == id) {
                return Ok(Some(project.id.clone()));
            }
        }
    }
    Ok(
        crate::workspace_ui::project_workspace(store, conversation_id)?
            .filter(|path| !path.trim().is_empty()),
    )
}

/// Every scope this conversation may draw facts from — user-level, its
/// project (by id and by path so both keyings match), and itself. Different
/// projects never share `project:` facts.
fn fact_scopes(store: &crate::store::Store, conversation_id: &str) -> Vec<String> {
    let mut scopes = vec![
        SCOPE_USER.to_string(),
        format!("conversation:{conversation_id}"),
    ];
    if let Ok(Some(key)) = project_key(store, conversation_id) {
        scopes.push(format!("project:{key}"));
    }
    // The path-keyed form covers facts taught before projects had ids and
    // facts taught while no workspace-index entry exists.
    if let Ok(Some(path)) = crate::workspace_ui::project_workspace(store, conversation_id) {
        let keyed = format!("project:{path}");
        if !scopes.contains(&keyed) {
            scopes.push(keyed);
        }
    }
    scopes
}

pub fn validate_fact(fact: &str) -> Result<&str, String> {
    if fact.trim().is_empty() || fact.len() > 2000 {
        return Err("Memory fact must be 1-2000 characters.".into());
    }
    if let Some(reason) = secret_match(fact) {
        return Err(format!(
            "Not storing that: it looks like a credential ({reason}). Keep secrets out of memory."
        ));
    }
    Ok(fact)
}

/// Cheap credential sniffing for both agent and user memory writes. Not a
/// secrets manager — just refuses the obvious patterns rather than persisting
/// a key into a plain-text fact bank.
fn secret_match(text: &str) -> Option<&'static str> {
    if text.contains("PRIVATE KEY") && text.contains("BEGIN") {
        return Some("private key block");
    }
    let lower = text.to_lowercase();
    for prefix in [
        "sk-", "ghp_", "gho_", "ghu_", "xoxb-", "xoxp-", "xoxa-", "akia",
    ] {
        // A marker followed by a long token run is almost certainly a key.
        if let Some(at) = lower.find(prefix) {
            let tail: String = lower[at..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if tail.len() >= 16 {
                return Some("API key pattern");
            }
        }
    }
    for marker in [
        "password=",
        "passwd=",
        "api_key=",
        "apikey=",
        "secret=",
        "token=",
        "auth_token=",
    ] {
        if let Some(at) = lower.find(marker) {
            let value: String = lower[at + marker.len()..]
                .chars()
                .take_while(|c| !c.is_whitespace())
                .collect();
            if value.len() >= 8 {
                return Some("credential assignment");
            }
        }
    }
    // Bare long high-entropy tokens (mixed alnum, 40+ chars).
    for word in text.split_whitespace() {
        let word: &str =
            word.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-');
        if word.len() >= 40
            && word.chars().any(|c| c.is_ascii_lowercase())
            && word.chars().any(|c| c.is_ascii_uppercase())
            && word.chars().any(|c| c.is_ascii_digit())
        {
            return Some("high-entropy token");
        }
    }
    None
}

/// Whether agent memory writes are stored. Default on — turning it off makes
/// `memory_teach` a no-op so nothing is retained without opting back in.
pub fn auto_retain(store: &crate::store::Store) -> bool {
    store
        .setting::<Value>(AUTO_RETAIN_KEY)
        .ok()
        .and_then(|value| value.as_bool())
        .unwrap_or(true)
}

pub fn set_auto_retain(store: &crate::store::Store, enabled: bool) -> Result<(), String> {
    store.save_setting(AUTO_RETAIN_KEY, &enabled)
}

/// Current branch for branch-aware staleness. `None` outside git.
fn head_branch(root: &str) -> Option<String> {
    crate::task_workspace::run_git(
        std::path::Path::new(root),
        &["rev-parse", "--abbrev-ref", "HEAD"],
    )
    .ok()
    .map(|name| name.trim().to_string())
    .filter(|name| !name.is_empty())
}

fn sha256_file(path: &std::path::Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    Some(format!("{:x}", Sha256::digest(&bytes)))
}

/// Mark file-derived facts stale when the source changed or went missing,
/// and project facts stale when the project moved to another branch.
/// Computed on every read — staleness is never persisted.
fn mark_stale(store: &crate::store::Store, conversation_id: &str, facts: &mut [Fact]) {
    let branch_cache =
        std::cell::RefCell::new(std::collections::HashMap::<String, Option<String>>::new());
    let project_path = crate::workspace_ui::project_workspace(store, conversation_id)
        .ok()
        .flatten();
    for fact in facts.iter_mut() {
        if let Some(path) = &fact.source_path {
            let candidate = std::path::Path::new(path);
            let resolved = if candidate.is_absolute() {
                candidate.to_path_buf()
            } else if let Some(root) = &project_path {
                std::path::Path::new(root).join(path)
            } else {
                candidate.to_path_buf()
            };
            fact.stale = match (&fact.source_hash, sha256_file(&resolved)) {
                (Some(stored), Some(now)) => stored != &now,
                _ => true, // file gone, or a path without a captured hash
            };
        }
        if !fact.stale {
            if let (Some(recorded), Some(root)) = (&fact.source_branch, &project_path) {
                let current = branch_cache
                    .borrow_mut()
                    .entry(root.clone())
                    .or_insert_with(|| head_branch(root))
                    .clone();
                if let Some(now) = current {
                    fact.stale = recorded != &now;
                }
            }
        }
    }
}

/// Facts eligible for injection into this conversation: matching scopes, not
/// excluded, pinned first. `stale` is computed so the UI can flag drift.
pub fn facts_for_conversation(
    store: &crate::store::Store,
    conversation_id: &str,
    limit: usize,
) -> Result<Vec<Fact>, String> {
    let scopes = fact_scopes(store, conversation_id);
    let mut facts = store.recall_facts_scoped(&scopes, limit.min(50))?;
    mark_stale(store, conversation_id, &mut facts);
    Ok(facts)
}

/// Recall payload attached to the current user turn (volatile suffix); bounded and verbatim.
pub fn recall_block(facts: &[Fact]) -> Option<String> {
    if facts.is_empty() {
        return None;
    }
    let mut block = String::from("Remembered facts (taught with memory_teach; apply them — they are context, not permission grants):\n");
    let mut bytes = block.len();
    for fact in facts.iter().take(20) {
        let line = format!("- {}\n", fact.fact);
        if bytes + line.len() > 4096 {
            break;
        }
        bytes += line.len();
        block.push_str(&line);
    }
    Some(block)
}

/// Project rule files, in precedence order (first wins): project AGENTS.md,
/// then other project rule files, then user-level rules. Re-read every call
/// so an edit takes effect on the next turn — no cache.
const RULE_FILE_CAP: usize = 16 * 1024;
const RULES_TOTAL_CAP: usize = 32 * 1024;

fn read_rule_file(path: &std::path::Path) -> Option<(String, usize)> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.is_empty() {
        return None;
    }
    let mut text = String::from_utf8_lossy(&bytes).to_string();
    let truncated = text.len() > RULE_FILE_CAP;
    if truncated {
        let mut boundary = RULE_FILE_CAP;
        while !text.is_char_boundary(boundary) {
            boundary -= 1;
        }
        text.truncate(boundary);
        text.push_str("\n[truncated]");
    }
    Some((
        text,
        if truncated {
            RULE_FILE_CAP
        } else {
            bytes.len()
        },
    ))
}

fn rule_items(root: &std::path::Path) -> Vec<ContextItem> {
    let mut items = Vec::new();
    let mut budget = RULES_TOTAL_CAP;
    let push = |label: String,
                path: &std::path::Path,
                reason: &str,
                budget: &mut usize,
                items: &mut Vec<ContextItem>| {
        if *budget == 0 {
            return;
        }
        if let Some((text, _)) = read_rule_file(path) {
            let text = if text.len() > *budget {
                let mut cut = *budget;
                while !text.is_char_boundary(cut) {
                    cut -= 1;
                }
                format!("{}\n[truncated]", &text[..cut])
            } else {
                text
            };
            *budget = budget.saturating_sub(text.len());
            items.push(ContextItem {
                kind: "rules".into(),
                label: label.clone(),
                tokens: text.len() / 4,
                reason: reason.to_string(),
                source: Some(path.to_string_lossy().into_owned()),
                id: None,
                text: format!("{reason} ({label}):\n{text}"),
            });
        }
    };
    // Precedence: project AGENTS.md > other project rule files > user-level.
    push(
        "AGENTS.md".into(),
        &root.join("AGENTS.md"),
        "Project rules",
        &mut budget,
        &mut items,
    );
    for name in ["CLAUDE.md", ".cursorrules"] {
        push(
            name.into(),
            &root.join(name),
            "Project rules",
            &mut budget,
            &mut items,
        );
    }
    if let Ok(entries) = std::fs::read_dir(root.join(".devin").join("rules")) {
        let mut files: Vec<_> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
            .collect();
        files.sort();
        for path in files {
            let label = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            push(
                format!(".devin/rules/{label}"),
                &path,
                "Project rules",
                &mut budget,
                &mut items,
            );
        }
    }
    if let Some(home) = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }) {
        if let Ok(entries) =
            std::fs::read_dir(std::path::Path::new(&home).join(".devin").join("rules"))
        {
            let mut files: Vec<_> = entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
                .collect();
            files.sort();
            for path in files {
                let label = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                push(
                    format!("~/.devin/rules/{label}"),
                    &path,
                    "User-level rules",
                    &mut budget,
                    &mut items,
                );
            }
        }
    }
    items
}

/// Build the context block a turn would inject — rules, repo map, and memory
/// facts — plus the list of fact ids for the `memory_used` event. Shared by
/// `assemble_turn` and the `context_preview` command so they cannot drift.
pub fn turn_context(
    store: &crate::store::Store,
    conversation_id: &str,
    include_memory: bool,
) -> (Vec<ContextItem>, Vec<Fact>) {
    let mut items = Vec::new();
    let workspace = crate::workspace_ui::task_workspace(store, conversation_id)
        .ok()
        .flatten()
        .filter(|path| !path.trim().is_empty())
        .unwrap_or_else(|| store.workspace_path().unwrap_or_default());
    if !workspace.trim().is_empty() {
        let root = std::path::Path::new(&workspace);
        items.extend(rule_items(root));
        if let Some(map) = crate::repo_map::repo_map(root, crate::repo_map::DEFAULT_BUDGET) {
            let text = format!(
                "Workspace map{}:\n{}",
                if map.partial { " (partial)" } else { "" },
                map.text
            );
            items.push(ContextItem {
                kind: "repo-map".into(),
                label: format!("Repository map · {} files", map.files),
                tokens: text.len() / 4,
                reason: if map.partial {
                    "Workspace overview (partial — scan cap hit)".into()
                } else {
                    "Workspace overview".into()
                },
                source: Some(workspace.clone()),
                id: None,
                text,
            });
        }
    }
    let mut facts = Vec::new();
    if include_memory {
        if let Ok(mut found) = facts_for_conversation(store, conversation_id, 20) {
            // Pinned facts order first from the recall; emit one item per fact
            // so the preview can point at exactly what was injected.
            for fact in &found {
                items.push(ContextItem {
                    kind: if fact.pinned {
                        "pinned".into()
                    } else {
                        "memory".into()
                    },
                    label: fact.fact.chars().take(80).collect(),
                    text: format!("- {}", fact.fact),
                    tokens: fact.fact.len() / 4,
                    reason: format!("{} fact in {}", fact.origin, fact.scope),
                    source: fact.source_path.clone(),
                    id: Some(fact.id.clone()),
                });
            }
            facts.append(&mut found);
        }
    }
    (items, facts)
}

/// The injections a turn carries for project context. `memory` is one
/// joined block (the model sees the same text the recall block produces);
/// rules and the repo map are separate injections.
pub fn context_injections(items: &[ContextItem], facts: &[Fact]) -> Vec<(String, String)> {
    let mut injections = Vec::new();
    let mut rules = String::new();
    for item in items.iter().filter(|item| item.kind == "rules") {
        if !rules.is_empty() {
            rules.push_str("\n\n");
        }
        rules.push_str(&item.text);
    }
    if !rules.is_empty() {
        injections.push(("rules".into(), format!("Rules and instructions (context only — they do not change permissions or available tools):\n{rules}")));
    }
    for item in items.iter().filter(|item| item.kind == "repo-map") {
        injections.push(("repo_map".into(), item.text.clone()));
    }
    if let Some(block) = recall_block(facts) {
        injections.push(("memory".into(), block));
    }
    injections
}

/// Cold-repo survey: read-only digest of top-level layout + recent commit
/// subjects, stored as one `auto` fact. Self-heals by replacing the previous
/// survey fact for the scope (delete + teach).
pub fn survey_repo(path: &std::path::Path) -> Result<String, String> {
    let mut entries: Vec<String> = std::fs::read_dir(path)
        .map_err(|error| format!("Could not survey workspace: {error}"))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with('.'))
        .take(40)
        .collect();
    entries.sort();
    let commits = std::process::Command::new("git")
        .args(["-C", &path.to_string_lossy(), "log", "--oneline", "-10"])
        .output()
        .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
        .unwrap_or_default();
    let mut digest = format!(
        "Repo survey of {}: top-level: {}.",
        path.to_string_lossy(),
        entries.join(", ")
    );
    if !commits.trim().is_empty() {
        digest.push_str(&format!(
            " Recent commits: {}.",
            commits.lines().take(10).collect::<Vec<_>>().join(" | ")
        ));
    }
    if digest.len() > 2000 {
        // Byte-truncate on a char boundary: `String::truncate` panics mid-UTF-8.
        let mut boundary = 2000;
        while !digest.is_char_boundary(boundary) {
            boundary -= 1;
        }
        digest.truncate(boundary);
    }
    Ok(digest)
}

#[tauri::command]
pub fn list_facts(
    state: tauri::State<'_, crate::AppState>,
    scope: String,
    limit: Option<usize>,
) -> Result<Vec<Fact>, String> {
    let scope = legacy_scope(&scope)?;
    state.database()?.recall_facts(&scope, limit.unwrap_or(20))
}

/// Standalone scope normalization for callers without a conversation
/// (settings surfaces): canonical forms pass through, `global` upgrades to
/// `user`, anything else is treated as a project path key.
fn legacy_scope(raw: &str) -> Result<String, String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 1024 {
        return Err("Memory scope must be 1-1024 characters.".into());
    }
    Ok(match raw {
        "global" | "user" => SCOPE_USER.to_string(),
        scope if scope.starts_with("project:") || scope.starts_with("conversation:") => {
            scope.to_string()
        }
        path => format!("project:{path}"),
    })
}

#[tauri::command]
pub fn teach_fact_cmd(
    state: tauri::State<'_, crate::AppState>,
    scope: String,
    fact: String,
) -> Result<Fact, String> {
    let scope = legacy_scope(&scope)?;
    state.database()?.teach_fact(&scope, &fact, "user")
}

#[tauri::command]
pub fn forget_fact(state: tauri::State<'_, crate::AppState>, id: String) -> Result<bool, String> {
    state.database()?.forget_fact(&id)
}

#[tauri::command]
pub async fn ingest_repo(
    state: tauri::State<'_, crate::AppState>,
    scope: String,
) -> Result<Fact, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before ingesting memory.")?;
    let path = std::path::PathBuf::from(scope.trim());
    if !path.is_dir() {
        return Err("Memory ingest needs an existing workspace directory.".into());
    }
    let digest = survey_repo(&path)?;
    let store = state.database()?;
    // Self-healing: replace the previous auto survey for this scope.
    let scope_key = format!("project:{}", path.to_string_lossy());
    let previous: Vec<Fact> = store
        .recall_facts_scoped(std::slice::from_ref(&scope_key), 50)?
        .into_iter()
        .filter(|fact| fact.origin == "survey")
        .collect();
    for fact in previous {
        let _ = store.forget_fact(&fact.id);
    }
    let branch = head_branch(&path.to_string_lossy());
    store.teach_fact_ex(
        &scope_key,
        &digest,
        "survey",
        Some("Workspace ingest"),
        None,
        Some(&path.to_string_lossy()),
        Some(&format!("{:x}", Sha256::digest(digest.as_bytes()))),
        branch.as_deref(),
    )
}

/// All facts this conversation can see (user + project + conversation
/// scopes) with staleness computed — the Context tab's memory list.
#[tauri::command]
pub fn memory_facts(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
) -> Result<Vec<Fact>, String> {
    let store = state.database()?;
    // Listing shows excluded facts too (that is where you un-exclude them);
    // only injection filters them out.
    let scopes = fact_scopes(&store, &conversation_id);
    let mut facts = store.recall_facts_scoped_include_excluded(&scopes, 100)?;
    mark_stale(&store, &conversation_id, &mut facts);
    Ok(facts)
}

#[tauri::command]
pub fn memory_create(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
    scope: String,
    fact: String,
    reason: Option<String>,
) -> Result<Fact, String> {
    let store = state.database()?;
    let scope = resolve_scope(&store, &conversation_id, &scope)?;
    store.teach_fact_ex(
        &scope,
        &fact,
        "user",
        reason.as_deref(),
        None,
        None,
        None,
        None,
    )
}

#[tauri::command]
pub fn memory_update(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
    id: String,
    text: String,
    reason: Option<String>,
) -> Result<Fact, String> {
    let store = state.database()?;
    let scopes = fact_scopes(&store, &conversation_id);
    let mut fact = store
        .fact(&id)?
        .filter(|fact| scopes.contains(&fact.scope))
        .ok_or("Unknown memory fact for this conversation.")?;
    fact.fact = validate_fact(&text)?.to_string();
    if let Some(reason) = reason {
        fact.reason = Some(reason.chars().take(300).collect());
    }
    store.update_fact(&fact)?;
    Ok(fact)
}

#[tauri::command]
pub fn memory_delete(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
    id: String,
) -> Result<(), String> {
    let store = state.database()?;
    let scopes = fact_scopes(&store, &conversation_id);
    match store.fact(&id)? {
        Some(fact) if scopes.contains(&fact.scope) => {
            store.forget_fact(&id)?;
            Ok(())
        }
        _ => Err("Unknown memory fact for this conversation.".into()),
    }
}

#[tauri::command]
pub fn memory_set_pinned(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
    id: String,
    pinned: bool,
) -> Result<(), String> {
    set_flag(state, conversation_id, id, "pinned", pinned)
}

#[tauri::command]
pub fn memory_set_excluded(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
    id: String,
    excluded: bool,
) -> Result<(), String> {
    set_flag(state, conversation_id, id, "excluded", excluded)
}

fn set_flag(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
    id: String,
    column: &str,
    value: bool,
) -> Result<(), String> {
    let store = state.database()?;
    let scopes = fact_scopes(&store, &conversation_id);
    match store.fact(&id)? {
        Some(fact) if scopes.contains(&fact.scope) => store.set_fact_flag(&id, column, value),
        _ => Err("Unknown memory fact for this conversation.".into()),
    }
}

#[tauri::command]
pub fn memory_auto_retain(state: tauri::State<'_, crate::AppState>) -> Result<bool, String> {
    let store = state.database()?;
    Ok(auto_retain(&store))
}

#[tauri::command]
pub fn memory_set_auto_retain(
    state: tauri::State<'_, crate::AppState>,
    enabled: bool,
) -> Result<(), String> {
    let store = state.database()?;
    set_auto_retain(&store, enabled)
}

#[tauri::command]
pub async fn context_preview(
    state: tauri::State<'_, crate::AppState>,
    conversation_id: String,
) -> Result<Value, String> {
    let db_path = state.db_path.clone();
    tokio::task::spawn_blocking(move || {
        let store = crate::store::Store::open(&db_path)?;
        // Mirror the turn's gate: memory facts are only injected when the
        // preset exposes memory_recall, so the preview must apply it too.
        let preset_id = store
            .conversation_preset(&conversation_id)
            .unwrap_or_else(|_| crate::presets::STANDARD.to_string());
        let include_memory = crate::presets::get(&preset_id)
            .map(|preset| preset.harness.iter().any(|alias| alias == "memory_recall"))
            .unwrap_or(false);
        let (items, _facts) = turn_context(&store, &conversation_id, include_memory);
        let total: usize = items.iter().map(|item| item.tokens).sum();
        Ok(json!({
            "items": items.iter().map(|item| json!({
                "kind": item.kind, "label": item.label, "tokens": item.tokens,
                "reason": item.reason, "source": item.source, "id": item.id,
            })).collect::<Vec<_>>(),
            "totalTokens": total,
            // The context block is a slice of the turn budget, not all of it.
            "budgetTokens": crate::repo_map::DEFAULT_BUDGET + 4096 / 4 + RULES_TOTAL_CAP / 4,
        }))
    })
    .await
    .map_err(|_| "Context preview stopped.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recall_block_is_bounded_and_verbatim() {
        assert!(recall_block(&[]).is_none());
        let facts = vec![Fact {
            id: "1".into(),
            scope: "user".into(),
            fact: "use pnpm not npm".into(),
            origin: "user".into(),
            reason: None,
            source_conversation_id: None,
            source_path: None,
            source_hash: None,
            source_branch: None,
            pinned: false,
            excluded: false,
            stale: false,
            created_at: 0,
            updated_at: 0,
        }];
        let block = recall_block(&facts).unwrap();
        assert!(block.contains("use pnpm not npm"));
    }
    #[test]
    fn validation_rejects_empty_oversize_and_secrets() {
        assert!(validate_fact("").is_err());
        assert!(validate_fact(&"x".repeat(2001)).is_err());
        assert!(validate_fact("my key is sk-abcdefghijklmnopqrstuvwxyz012345").is_err());
        assert!(validate_fact("-----BEGIN PRIVATE KEY-----\nMII").is_err());
        assert!(validate_fact("password=hunter2hunter2").is_err());
        assert!(validate_fact("AKIAIOSFODNN7EXAMPLE lives here").is_err());
        assert!(validate_fact("uses bearer ").is_ok());
        assert!(validate_fact("the token must expire").is_ok());
    }
    #[test]
    fn secret_guard_does_not_reject_ordinary_text() {
        assert!(validate_fact("API keys go in the vault, never in memory").is_ok());
        assert!(validate_fact("set NODE_ENV=production for builds").is_ok());
    }
    #[test]
    fn survey_lists_top_level_without_dotfiles() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join(".hidden"), "x").unwrap();
        let digest = survey_repo(dir.path()).unwrap();
        assert!(digest.contains("src") && !digest.contains(".hidden"));
    }
    #[test]
    fn survey_truncates_on_a_utf8_boundary() {
        let dir = tempfile::tempdir().unwrap();
        // Long multi-byte filenames push the digest past the 2000-byte cap
        // while guaranteeing the cut would land inside a character.
        for index in 0..40 {
            let name = format!(
                "résumé_日本語_{}_ファイル{:02}.md",
                "名前".repeat(20),
                index
            );
            std::fs::write(dir.path().join(name), "x").unwrap();
        }
        let digest = survey_repo(dir.path()).unwrap();
        assert!(digest.len() <= 2000);
        assert!(digest.is_char_boundary(digest.len()));
    }
    #[test]
    fn survey_empty_directory_still_works() {
        let dir = tempfile::tempdir().unwrap();
        let digest = survey_repo(dir.path()).unwrap();
        assert!(digest.len() <= 2000);
        assert!(digest.contains("top-level:"));
    }

    /// Two conversations filed under two projects.
    fn fixture() -> (tempfile::TempDir, crate::store::Store, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&dir.path().join("app.db")).unwrap();
        let c1 = store.create_conversation().unwrap().id;
        let c2 = store.create_conversation().unwrap().id;
        let pa = dir.path().join("proj-a");
        let pb = dir.path().join("proj-b");
        std::fs::create_dir_all(&pa).unwrap();
        std::fs::create_dir_all(&pb).unwrap();
        store
            .save_setting(
                "workspace_index_v1",
                &serde_json::json!({
                    "projects": [
                        {"id": "p1", "name": "A", "path": pa.to_string_lossy()},
                        {"id": "p2", "name": "B", "path": pb.to_string_lossy()},
                    ],
                    "tasks": {
                        &c1: {"projectId": "p1", "archived": false, "pinned": false},
                        &c2: {"projectId": "p2", "archived": false, "pinned": false},
                    },
                }),
            )
            .unwrap();
        (dir, store, c1, c2)
    }

    #[test]
    fn project_facts_never_leak_between_projects() {
        let (_dir, store, c1, c2) = fixture();
        let scope = resolve_scope(&store, &c1, "project").unwrap();
        assert_eq!(scope, "project:p1");
        store
            .teach_fact(&scope, "project A uses pnpm", "user")
            .unwrap();
        store
            .teach_fact("user", "dark mode everywhere", "user")
            .unwrap();
        let a = facts_for_conversation(&store, &c1, 20).unwrap();
        let b = facts_for_conversation(&store, &c2, 20).unwrap();
        assert!(a.iter().any(|fact| fact.fact.contains("pnpm")), "{a:?}");
        assert!(!b.iter().any(|fact| fact.fact.contains("pnpm")), "{b:?}");
        assert!(a.iter().any(|fact| fact.fact.contains("dark mode")));
        assert!(b.iter().any(|fact| fact.fact.contains("dark mode")));
        // A fact scoped to another conversation stays invisible.
        let theirs = format!("conversation:{c2}");
        store
            .teach_fact(&theirs, "c2 secret context", "agent")
            .unwrap();
        assert!(!facts_for_conversation(&store, &c1, 20)
            .unwrap()
            .iter()
            .any(|fact| fact.fact.contains("secret context")));
    }

    #[test]
    fn excluded_and_deleted_facts_never_inject() {
        let (_dir, store, c1, _c2) = fixture();
        let scope = resolve_scope(&store, &c1, "conversation").unwrap();
        let fact = store.teach_fact(&scope, "temporary note", "user").unwrap();
        assert!(facts_for_conversation(&store, &c1, 20)
            .unwrap()
            .iter()
            .any(|f| f.id == fact.id));
        store.set_fact_flag(&fact.id, "excluded", true).unwrap();
        assert!(!facts_for_conversation(&store, &c1, 20)
            .unwrap()
            .iter()
            .any(|f| f.id == fact.id));
        // Still listed for the UI (include_excluded listing) so it can be restored.
        let scopes = fact_scopes(&store, &c1);
        assert!(store
            .recall_facts_scoped_include_excluded(&scopes, 50)
            .unwrap()
            .iter()
            .any(|f| f.id == fact.id));
        store.forget_fact(&fact.id).unwrap();
        assert!(!store
            .recall_facts_scoped_include_excluded(&scopes, 50)
            .unwrap()
            .iter()
            .any(|f| f.id == fact.id));
    }

    #[test]
    fn auto_retain_flag_round_trips_and_secrets_never_persist() {
        let (_dir, store, c1, _c2) = fixture();
        assert!(auto_retain(&store));
        set_auto_retain(&store, false).unwrap();
        assert!(!auto_retain(&store));
        let result = store.teach_fact("user", "ghp_0123456789abcdefghijklmnop", "agent");
        assert!(result.is_err(), "credential-looking fact must be refused");
        assert!(facts_for_conversation(&store, &c1, 20).unwrap().is_empty());
    }

    #[test]
    fn edited_rule_files_reach_the_next_turn() {
        let (dir, store, c1, _c2) = fixture();
        let root = dir.path().join("proj-a");
        std::fs::write(root.join("AGENTS.md"), "first rules").unwrap();
        let (items, _) = turn_context(&store, &c1, false);
        let rules = items.iter().find(|item| item.kind == "rules").unwrap();
        assert!(rules.text.contains("first rules"));
        std::fs::write(root.join("AGENTS.md"), "updated rules").unwrap();
        let (items, _) = turn_context(&store, &c1, false);
        let rules = items.iter().find(|item| item.kind == "rules").unwrap();
        assert!(rules.text.contains("updated rules"), "{}", rules.text);
        assert!(!rules.text.contains("first rules"));
    }

    #[test]
    fn context_preview_items_match_what_turns_inject() {
        let (dir, store, c1, _c2) = fixture();
        std::fs::write(dir.path().join("proj-a").join("AGENTS.md"), "project rules").unwrap();
        store
            .teach_fact(
                &resolve_scope(&store, &c1, "project").unwrap(),
                "remember this",
                "user",
            )
            .unwrap();
        let (items, facts) = turn_context(&store, &c1, true);
        let injections = context_injections(&items, &facts);
        let rules = injections.iter().find(|(kind, _)| kind == "rules").unwrap();
        assert!(rules.1.contains("project rules"));
        let memory = injections
            .iter()
            .find(|(kind, _)| kind == "memory")
            .unwrap();
        assert!(memory.1.contains("remember this"));
        // The preview items are the injection's sources — every injected rule
        // file and every fact id appears.
        assert!(items
            .iter()
            .any(|item| item.kind == "rules" && item.label == "AGENTS.md"));
        assert!(items
            .iter()
            .any(|item| item.id.as_deref() == Some(facts[0].id.as_str())));
    }

    #[test]
    fn file_derived_fact_turns_stale_when_source_changes() {
        let (dir, store, c1, _c2) = fixture();
        let doc = dir.path().join("proj-a").join("notes.md");
        std::fs::write(&doc, "v1").unwrap();
        let hash = format!("{:x}", Sha256::digest(b"v1"));
        let scope = resolve_scope(&store, &c1, "project").unwrap();
        store
            .teach_fact_ex(
                &scope,
                "notes describe v1",
                "user",
                None,
                None,
                Some(&doc.to_string_lossy()),
                Some(&hash),
                None,
            )
            .unwrap();
        assert!(!facts_for_conversation(&store, &c1, 20).unwrap()[0].stale);
        std::fs::write(&doc, "v2 changed").unwrap();
        assert!(facts_for_conversation(&store, &c1, 20).unwrap()[0].stale);
        std::fs::remove_file(&doc).unwrap();
        assert!(facts_for_conversation(&store, &c1, 20).unwrap()[0].stale);
    }
}
