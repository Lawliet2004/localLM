//! Explicit per-project, bounded UTF-8 document retrieval. No embedding service.
use crate::{store::Store, workspace::Workspace, AppState};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentIndex {
    pub enabled: bool,
    pub paths: Vec<String>,
    pub updated_at: i64,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentHit {
    pub path: String,
    pub line: usize,
    pub excerpt: String,
}
fn key(root: &str) -> Result<String, String> {
    let path = std::fs::canonicalize(root).map_err(|e| format!("Cannot open project: {e}"))?;
    Ok(path.to_string_lossy().into_owned())
}
fn settings(store: &Store) -> Result<BTreeMap<String, DocumentIndex>, String> {
    store.setting("document_indexes_v1")
}
fn validate_paths(paths: &[String]) -> Result<(), String> {
    if paths.is_empty() || paths.len() > 16 {
        return Err("Choose one to 16 documents.".into());
    }
    for path in paths {
        let extension = std::path::Path::new(path)
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if !["md", "txt", "rst", "csv", "tsv"].contains(&extension.as_str()) {
            return Err("Only Markdown, text, RST, CSV and TSV documents are supported.".into());
        }
    }
    Ok(())
}
pub fn retrieve(store: &Store, root: &str, query: &str) -> Result<Vec<DocumentHit>, String> {
    let root = key(root)?;
    let index = settings(store)?.remove(&root).unwrap_or_default();
    if !index.enabled {
        return Ok(Vec::new());
    }
    let workspace = Workspace::open(&root)?;
    let terms: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|term| term.len() > 2)
        .take(40)
        .map(str::to_lowercase)
        .collect();
    let mut hits: Vec<(usize, DocumentHit)> = Vec::new();
    for path in index.paths {
        // Re-read scoped files on every retrieval: edits are current, deleted
        // files disappear, and no stale private content remains in an index.
        let Ok(text) = workspace.read_script(&path) else {
            continue;
        };
        for (line, chunk) in text.lines().collect::<Vec<_>>().chunks(12).enumerate() {
            let excerpt = chunk.join("\n");
            let lower = excerpt.to_lowercase();
            let score = terms
                .iter()
                .filter(|term| lower.contains(term.as_str()))
                .count();
            if score > 0 {
                hits.push((
                    score,
                    DocumentHit {
                        path: path.clone(),
                        line: line * 12 + 1,
                        excerpt: excerpt.chars().take(1800).collect(),
                    },
                ));
            }
        }
    }
    hits.sort_by_key(|hit| std::cmp::Reverse(hit.0));
    Ok(hits.into_iter().take(3).map(|(_, hit)| hit).collect())
}
pub fn forget_project(store: &Store, root: &str) -> Result<(), String> {
    let mut indexes = settings(store)?;
    if indexes.remove(root).is_some() {
        store.save_setting("document_indexes_v1", &indexes)?;
    }
    Ok(())
}

#[tauri::command]
pub fn document_index_status(
    state: tauri::State<'_, AppState>,
    root: String,
) -> Result<DocumentIndex, String> {
    let root = crate::workspace_ui::bound_root(&state, &root)?;
    Ok(settings(&*state.database()?)?
        .remove(&key(&root)?)
        .unwrap_or_default())
}
#[tauri::command]
pub fn save_document_index(
    state: tauri::State<'_, AppState>,
    root: String,
    enabled: bool,
    paths: Vec<String>,
) -> Result<DocumentIndex, String> {
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation.")?;
    let root = key(&crate::workspace_ui::bound_root(&state, &root)?)?;
    if enabled {
        validate_paths(&paths)?;
    }
    let mut paths = paths;
    paths.sort();
    paths.dedup();
    if enabled {
        let workspace = Workspace::open(&root)?;
        for path in &paths {
            workspace.read_script(path).map_err(|error| format!("{path}: cannot index UTF-8 documents over 32 KiB or inaccessible files ({error})"))?;
        }
    }
    let store = state.database()?;
    let mut indexes = settings(&store)?;
    let value = DocumentIndex {
        enabled,
        paths: if enabled { paths } else { Vec::new() },
        updated_at: crate::store::now(),
    };
    indexes.insert(root, value.clone());
    store.save_setting("document_indexes_v1", &indexes)?;
    Ok(value)
}
#[tauri::command]
pub fn search_document_index(
    state: tauri::State<'_, AppState>,
    root: String,
    query: String,
) -> Result<Vec<DocumentHit>, String> {
    if query.len() > 4096 {
        return Err("Search query is too long.".into());
    }
    let root = crate::workspace_ui::bound_root(&state, &root)?;
    retrieve(&*state.database()?, &root, &query)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_scope_is_off_by_default_and_reads_current_documents() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().to_str().unwrap();
        let store = Store::open_memory().unwrap();
        std::fs::write(
            directory.path().join("guide.md"),
            "The falcon project uses SQLite.",
        )
        .unwrap();
        assert!(retrieve(&store, root, "falcon").unwrap().is_empty());
        let mut indexes = BTreeMap::new();
        indexes.insert(
            key(root).unwrap(),
            DocumentIndex {
                enabled: true,
                paths: vec!["guide.md".into()],
                updated_at: 1,
            },
        );
        store.save_setting("document_indexes_v1", &indexes).unwrap();
        let hits = retrieve(&store, root, "falcon").unwrap();
        assert_eq!(hits[0].path, "guide.md");
        assert_eq!(hits[0].line, 1);
        std::fs::write(
            directory.path().join("guide.md"),
            "The falcon project uses PostgreSQL.",
        )
        .unwrap();
        assert!(retrieve(&store, root, "falcon").unwrap()[0]
            .excerpt
            .contains("PostgreSQL"));
        std::fs::remove_file(directory.path().join("guide.md")).unwrap();
        assert!(retrieve(&store, root, "falcon").unwrap().is_empty());
    }
    #[test]
    fn limits_file_types_and_refuses_outside_links() {
        assert!(validate_paths(&["secret.env".into()]).is_err());
        assert!(validate_paths(&vec!["readme.md".into(); 17]).is_err());
        let directory = tempfile::tempdir().unwrap();
        assert!(Workspace::open(directory.path().to_str().unwrap())
            .unwrap()
            .read_script("../outside.txt")
            .is_err());
    }
}
