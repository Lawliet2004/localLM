//! Local-first memory bank backed by SQLite (Phase 6).
//!
//! Facts are scoped to a workspace path or `global`. The harness recalls at
//! task start and injects verbatim (logged as `context_injection`); the model
//! teaches via `memory_teach`. No cloud sync; nothing leaves the device
//! except through the provider carrying the prompt.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Fact {
    pub id: String,
    pub scope: String,
    pub fact: String,
    pub origin: String,
    pub created_at: i64,
    pub updated_at: i64,
}

pub fn normalize_scope(scope: &str) -> Result<String, String> {
    let scope = scope.trim();
    if scope.is_empty() || scope.len() > 1024 {
        return Err("Memory scope must be 1-1024 characters (workspace path or 'global').".into());
    }
    Ok(scope.to_string())
}

pub fn validate_fact(fact: &str) -> Result<&str, String> {
    if fact.trim().is_empty() || fact.len() > 2000 {
        return Err("Memory fact must be 1-2000 characters.".into());
    }
    Ok(fact)
}

/// Recall payload injected into the turn; bounded and verbatim.
pub fn recall_block(facts: &[Fact]) -> Option<String> {
    if facts.is_empty() {
        return None;
    }
    let mut block = String::from("Remembered facts for this workspace (taught with memory_teach; apply them):\n");
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
    let mut digest = format!("Repo survey of {}: top-level: {}.", path.to_string_lossy(), entries.join(", "));
    if !commits.trim().is_empty() {
        digest.push_str(&format!(" Recent commits: {}.", commits.lines().take(10).collect::<Vec<_>>().join(" | ")));
    }
    if digest.len() > 2000 {
        digest.truncate(2000);
    }
    Ok(digest)
}

#[tauri::command]
pub fn list_facts(state: tauri::State<'_, crate::AppState>, scope: String, limit: Option<usize>) -> Result<Vec<Fact>, String> {
    state.database()?.recall_facts(&scope, limit.unwrap_or(20))
}

#[tauri::command]
pub fn teach_fact_cmd(state: tauri::State<'_, crate::AppState>, scope: String, fact: String) -> Result<Fact, String> {
    state.database()?.teach_fact(&scope, &fact, "ui")
}

#[tauri::command]
pub fn forget_fact(state: tauri::State<'_, crate::AppState>, id: String) -> Result<bool, String> {
    state.database()?.forget_fact(&id)
}

#[tauri::command]
pub async fn ingest_repo(state: tauri::State<'_, crate::AppState>, scope: String) -> Result<Fact, String> {
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
    let previous: Vec<Fact> = store
        .recall_facts(&path.to_string_lossy(), 50)?
        .into_iter()
        .filter(|fact| fact.origin == "auto-survey")
        .collect();
    for fact in previous {
        let _ = store.forget_fact(&fact.id);
    }
    store.teach_fact(&path.to_string_lossy(), &digest, "auto-survey")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recall_block_is_bounded_and_verbatim() {
        assert!(recall_block(&[]).is_none());
        let facts = vec![Fact {
            id: "1".into(),
            scope: "global".into(),
            fact: "use pnpm not npm".into(),
            origin: "ui".into(),
            created_at: 0,
            updated_at: 0,
        }];
        let block = recall_block(&facts).unwrap();
        assert!(block.contains("use pnpm not npm"));
    }
    #[test]
    fn validation_rejects_empty_and_oversize() {
        assert!(normalize_scope("").is_err());
        assert!(validate_fact("").is_err());
        assert!(validate_fact(&"x".repeat(2001)).is_err());
    }
    #[test]
    fn survey_lists_top_level_without_dotfiles() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join(".hidden"), "x").unwrap();
        let digest = survey_repo(dir.path()).unwrap();
        assert!(digest.contains("src") && !digest.contains(".hidden"));
    }
}
