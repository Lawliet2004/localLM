//! Private local document collections with exact-location citations.
//!
//! A collection is a folder the user explicitly adds in Settings → Documents.
//! Indexing reads text files only (bounded count and size, binary skipped),
//! splits them into overlapping line windows, and stores the chunks in a
//! SQLite FTS5 table. The `docs_search` harness tool answers with the file
//! path and line range of each chunk, so the model cites an exact location
//! instead of "the document says". Documents never leave the machine; reads
//! reuse the indexed text, not the live file, so removal is by deleting the
//! collection row — its chunks go with it.

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocCollection {
    pub id: String,
    pub name: String,
    pub root: String,
    pub file_count: i64,
    pub chunk_count: i64,
    pub indexed_at: Option<i64>,
    pub error: Option<String>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocHit {
    pub collection_id: String,
    pub collection: String,
    /// Path relative to the collection root — enough to locate the file.
    pub path: String,
    pub line_start: i64,
    pub line_end: i64,
    pub snippet: String,
}

/// ponytail: single pass, whole file in memory — bounded so documents can't
/// wedge indexing on a giant log.
const MAX_FILES: usize = 2_000;
const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// Chunks carry line windows; 90 lines ≈ a screen and a half of dense prose,
/// with overlap so a sentence spanning a boundary stays findable.
const CHUNK_LINES: usize = 90;
const CHUNK_OVERLAP: usize = 10;
const MAX_CHUNKS_PER_FILE: usize = 200;
const MAX_HITS: usize = 8;
const MAX_SNIPPET_CHARS: usize = 1_200;

/// Text-ish extensions a local model can meaningfully read.
const EXTENSIONS: &[&str] = &[
    "md", "markdown", "txt", "text", "rst", "adoc", "org", "csv", "tsv", "json", "jsonl", "xml",
    "html", "htm", "log", "yaml", "yml", "toml", "ini", "cfg", "rs", "py", "ts", "tsx", "js", "jsx",
    "java", "c", "h", "cpp", "hpp", "cs", "go", "rb", "php", "sh", "ps1", "sql", "css", "scss",
];

fn text_file(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| EXTENSIONS.contains(&extension.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// Split a file into overlapping line windows: `[(start_line, end_line, text)]`.
/// Lines are 1-based, end exclusive — the citation format users expect.
fn chunk_lines(text: &str) -> Vec<(i64, i64, String)> {
    let lines: Vec<&str> = text.lines().collect();
    let step = CHUNK_LINES - CHUNK_OVERLAP;
    let mut chunks = Vec::new();
    let mut start = 0usize;
    while start < lines.len() && chunks.len() < MAX_CHUNKS_PER_FILE {
        let end = (start + CHUNK_LINES).min(lines.len());
        let body = lines[start..end].join("\n");
        let trimmed = body.trim();
        if trimmed.len() > 3 {
            chunks.push(((start + 1) as i64, end as i64, trimmed.to_string()));
        }
        if end == lines.len() {
            break;
        }
        start += step;
    }
    chunks
}

/// Index one collection's root. Returns (files, chunks) after replacing the
/// old rows atomically so a failed scan never leaves a half-indexed set.
pub fn index_collection(
    store: &crate::store::Store,
    id: &str,
) -> Result<DocCollection, String> {
    let collection = store.doc_collection(id)?.ok_or("Unknown collection.")?;
    let mut files = 0i64;
    let mut chunks: Vec<(String, i64, i64, String)> = Vec::new();
    let mut truncated = false;
    let mut stack = vec![std::path::PathBuf::from(&collection.root)];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // Hidden folders are caches or control dirs — never user docs.
            if name.starts_with('.') {
                continue;
            }
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(_) => continue,
            };
            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() && text_file(&path) {
                if files as usize >= MAX_FILES {
                    truncated = true;
                    break;
                }
                let Ok(metadata) = std::fs::metadata(&path) else { continue };
                if metadata.len() > MAX_FILE_BYTES || metadata.len() == 0 {
                    continue;
                }
                // Lossy-read so a stray bad byte can't kill the whole index;
                // binary files still fall out through the extension filter.
                let Ok(text) = std::fs::read(&path).map(|bytes| String::from_utf8_lossy(&bytes).into_owned()) else {
                    continue;
                };
                let relative = path
                    .strip_prefix(&collection.root)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|_| name.to_string());
                for (start, end, body) in chunk_lines(&text) {
                    chunks.push((relative.clone(), start, end, body));
                }
                files += 1;
            }
        }
        if truncated {
            break;
        }
    }
    let error = truncated.then(|| format!("Stopped after {MAX_FILES} files — index is partial."));
    store.doc_chunks_replace(id, &chunks, files, error.as_deref())?;
    store.doc_collection(id)?.ok_or("Collection vanished during indexing.".into())
}

/// FTS5 search across one or all collections, BM25-ranked.
pub fn search(
    store: &crate::store::Store,
    query: &str,
    collection_id: Option<&str>,
) -> Result<Vec<DocHit>, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("Search needs a non-empty query.".into());
    }
    if query.len() > 512 {
        return Err("Keep the search query under 512 characters.".into());
    }
    // Match every token as a prefix — filenames and prose both match; quoting
    // each token keeps user text from being parsed as FTS5 syntax.
    let terms: Vec<String> = query
        .split_whitespace()
        .take(12)
        .map(|word| format!("\"{}\"*", word.replace('"', " ")))
        .collect();
    if terms.is_empty() {
        return Err("Search needs at least one word.".into());
    }
    let match_query = terms.join(" AND ");
    let rows = store.doc_chunks_search(&match_query, collection_id, MAX_HITS as i64)?;
    Ok(rows
        .into_iter()
        .map(|(collection_id, collection, path, line_start, line_end, snippet)| DocHit {
            collection_id,
            collection,
            path,
            line_start,
            line_end,
            snippet,
        })
        .collect())
}

// -- Tauri commands ----------------------------------------------------------

#[tauri::command]
pub fn documents_list_collections(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<DocCollection>, String> {
    state.database()?.doc_collections()
}

#[tauri::command]
pub fn documents_add_collection(
    state: tauri::State<'_, crate::AppState>,
    root: String,
    name: String,
) -> Result<DocCollection, String> {
    let store = state.database()?;
    let root_path = std::path::Path::new(&root);
    if !root_path.is_dir() {
        return Err("Pick an existing folder to index.".into());
    }
    let name = if name.trim().is_empty() {
        root_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Documents".into())
    } else {
        name.trim().to_string()
    };
    let canonical = root_path.canonicalize().map_err(|_| "Cannot resolve that folder.".to_string())?;
    let collection = DocCollection {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        root: canonical.to_string_lossy().into_owned(),
        file_count: 0,
        chunk_count: 0,
        indexed_at: None,
        error: None,
        created_at: crate::store::now(),
    };
    store.doc_collection_save(&collection)?;
    index_collection(&*store, &collection.id)
}

#[tauri::command]
pub fn documents_remove_collection(
    state: tauri::State<'_, crate::AppState>,
    id: String,
) -> Result<(), String> {
    state.database()?.doc_collection_delete(&id)
}

#[tauri::command]
pub fn documents_reindex(
    state: tauri::State<'_, crate::AppState>,
    id: String,
) -> Result<DocCollection, String> {
    index_collection(&*state.database()?, &id)
}

#[tauri::command]
pub fn documents_search(
    state: tauri::State<'_, crate::AppState>,
    query: String,
    collection_id: Option<String>,
) -> Result<Vec<DocHit>, String> {
    search(&*state.database()?, &query, collection_id.as_deref())
}

/// Harness tool result: ranked chunks with exact `path:line` citations.
pub fn harness_search(store: &crate::store::Store, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let query = args
        .get("query")
        .and_then(serde_json::Value::as_str)
        .ok_or("docs_search needs a 'query' string (e.g. {\"query\": \"vacation policy\"}).")?;
    let collection = args.get("collectionId").and_then(serde_json::Value::as_str);
    let hits = search(store, query, collection)?;
    if hits.is_empty() {
        return Ok(serde_json::json!({
            "matches": [],
            "message": "No matches. Documents are indexed on disk only; add a folder under Settings → Documents if the answer is not indexed yet.",
        }));
    }
    Ok(serde_json::json!({
        "matches": hits.iter().map(|hit| serde_json::json!({
            "citation": format!("{}:{}-{}", hit.path, hit.line_start, hit.line_end),
            "collection": hit.collection,
            "path": hit.path,
            "lineStart": hit.line_start,
            "lineEnd": hit.line_end,
            "snippet": hit.snippet.chars().take(MAX_SNIPPET_CHARS).collect::<String>(),
        })).collect::<Vec<_>>(),
        "instruction": "Cite 'path:lineStart-lineEnd' for each claim. The snippet is the indexed text at save time — reindex a collection after editing its files.",
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, crate::store::Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&dir.path().join("app.sqlite")).unwrap();
        (dir, store)
    }

    fn docs_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("policy.md"),
            "Employees get 25 vacation days.\n".repeat(60)
                + "Remote work is allowed twice a week.\n"
                + &"Carry-over expires in March.\n".repeat(60),
        )
        .unwrap();
        std::fs::write(dir.path().join("notes.txt"), "the quick brown fox\njumps over\n".repeat(50)).unwrap();
        std::fs::write(dir.path().join("binary.bin"), [0u8, 159, 146, 150]).unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/config"), "secret".repeat(20)).unwrap();
        dir
    }

    fn add(store: &crate::store::Store, root: &std::path::Path) -> DocCollection {
        let collection = DocCollection {
            id: uuid::Uuid::new_v4().to_string(),
            name: "Test docs".into(),
            root: root.canonicalize().unwrap().to_string_lossy().into_owned(),
            file_count: 0,
            chunk_count: 0,
            indexed_at: None,
            error: None,
            created_at: crate::store::now(),
        };
        store.doc_collection_save(&collection).unwrap();
        index_collection(store, &collection.id).unwrap()
    }

    #[test]
    fn indexing_counts_text_files_and_creates_searchable_chunks() {
        let docs = docs_dir();
        let (_dir, store) = store();
        let collection = add(&store, docs.path());
        assert_eq!(collection.file_count, 2); // .bin skipped, .git skipped
        assert!(collection.chunk_count >= 2);
        assert!(collection.indexed_at.is_some());
    }

    #[test]
    fn search_returns_exact_line_locations() {
        let docs = docs_dir();
        let (_dir, store) = store();
        let collection = add(&store, docs.path());
        let hits = search(&store, "vacation days", None).unwrap();
        assert!(!hits.is_empty());
        assert_eq!(hits[0].collection, "Test docs");
        assert_eq!(hits[0].path, "policy.md");
        assert!(hits[0].line_start >= 1 && hits[0].line_end > hits[0].line_start);
        assert!(hits[0].snippet.contains("vacation"), "{}", hits[0].snippet);
        // Scoping to a different collection returns nothing.
        assert!(search(&store, "vacation days", Some("other-id")).unwrap().is_empty());
        let _ = collection;
    }

    #[test]
    fn reindex_replaces_stale_chunks() {
        let docs = docs_dir();
        let (_dir, store) = store();
        let collection = add(&store, docs.path());
        std::fs::write(docs.path().join("policy.md"), "nothing about vacation now\n").unwrap();
        index_collection(&store, &collection.id).unwrap();
        assert!(search(&store, "vacation days", None).unwrap().is_empty());
    }

    #[test]
    fn fts5_is_available_and_queries_are_quoted_safely() {
        let (_dir, store) = store();
        // No injection through query text: FTS syntax characters are wrapped.
        let result = search(&store, "policy OR index*", None);
        assert!(result.is_ok(), "{result:?}");
    }
}
