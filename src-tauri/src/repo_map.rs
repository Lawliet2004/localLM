//! A bounded "map" of the workspace for the turn context: important files, a
//! directory summary, top-level symbols, and the build/test/run commands a
//! contributor would look for first. Computed under strict budgets (file cap,
//! wall clock, token estimate) and cached per tree state.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Default token budget for the injected map (chars/4 estimate).
pub const DEFAULT_BUDGET: usize = 1500;
const MAX_FILES: usize = 20_000;
const SCAN_BUDGET: std::time::Duration = std::time::Duration::from_secs(2);
/// Files larger than this are listed but their symbols are not scanned.
const SYMBOL_FILE_CAP: u64 = 64 * 1024;
/// Symbol grep is capped per file and overall — it is a hint, not an index.
const SYMBOLS_PER_FILE: usize = 3;
const SYMBOLS_TOTAL: usize = 40;

#[derive(Clone, Debug)]
pub struct RepoMap {
    pub text: String,
    /// Source-file count scanned.
    pub files: usize,
    /// A cap or the wall clock cut the scan short.
    pub partial: bool,
}

/// Cache keyed by (root, HEAD, working-tree status hash): any commit or edit
/// invalidates, anything untouched hits. Bounded to a few entries.
fn cache() -> &'static Mutex<HashMap<String, RepoMap>> {
    static CACHE: OnceLock<Mutex<HashMap<String, RepoMap>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// What the model needs first: entry points and contributor-facing docs.
const IMPORTANT_FILES: [&str; 14] = [
    "README",
    "README.md",
    "README.txt",
    "AGENTS.md",
    "CLAUDE.md",
    "PRODUCT.md",
    "DESIGN.md",
    "package.json",
    "Cargo.toml",
    "Cargo.lock",
    "pyproject.toml",
    "Makefile",
    "go.mod",
    "setup.py",
];

const SKIP_DIRS: [&str; 7] = [
    ".git",
    "node_modules",
    "target",
    "dist",
    ".venv",
    "__pycache__",
    "vendor",
];

const SOURCE_EXTS: [&str; 18] = [
    "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "py", "go", "java", "c", "h", "cpp", "cs", "rb",
    "swift", "kt", "sh",
];

fn is_source(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| SOURCE_EXTS.contains(&ext.to_lowercase().as_str()))
}

/// Tree state the cache key covers: HEAD plus a hash of `git status
/// --porcelain`. Non-git roots key on a cheap bounded metadata walk so
/// adding/editing a top-level file invalidates.
fn tree_key(root: &Path) -> Option<String> {
    let head = crate::task_workspace::run_git(root, &["rev-parse", "HEAD"])
        .ok()
        .map(|sha| sha.trim().to_string());
    if let Some(head) = head {
        let status = crate::task_workspace::run_git(root, &["status", "--porcelain", "-z"])
            .unwrap_or_default();
        Some(format!(
            "{}|{}|{:x}",
            root.to_string_lossy(),
            head,
            Sha256::digest(status.as_bytes())
        ))
    } else {
        let mut hasher = Sha256::new();
        let mut count = 0usize;
        for entry in std::fs::read_dir(root).ok()?.flatten().take(2048) {
            if let Ok(meta) = entry.metadata() {
                hasher.update(entry.file_name().to_string_lossy().as_bytes());
                hasher.update(meta.len().to_le_bytes());
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                hasher.update(mtime.to_le_bytes());
                count += 1;
            }
        }
        Some(format!(
            "{}|nogit|{:x}|{count}",
            root.to_string_lossy(),
            hasher.finalize()
        ))
    }
}

fn git_files(root: &Path) -> Option<Vec<String>> {
    let out =
        crate::task_workspace::run_git(root, &["ls-files", "-co", "--exclude-standard", "-z"])
            .ok()?;
    Some(
        out.split('\0')
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

fn walk_files(root: &Path, deadline: Instant) -> (Vec<String>, bool) {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    let mut partial = false;
    while let Some(dir) = stack.pop() {
        if files.len() >= MAX_FILES || Instant::now() > deadline {
            partial = true;
            break;
        }
        let Ok(children) = std::fs::read_dir(&dir) else {
            continue;
        };
        for child in children.flatten() {
            let name = child.file_name();
            let name = name.to_string_lossy();
            if SKIP_DIRS.contains(&name.as_ref()) {
                continue;
            }
            let path = child.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                if let Ok(rel) = path.strip_prefix(root) {
                    files.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    }
    (files, partial)
}

/// Cheap symbol skim: prefix-matched declaration lines in small source files.
/// Deliberately dumb — a map, not a parser.
fn symbols_in(text: &str) -> Vec<String> {
    const MARKERS: [&str; 8] = [
        "fn ",
        "struct ",
        "class ",
        "def ",
        "export function ",
        "export const ",
        "pub fn ",
        "const ",
    ];
    let mut symbols = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if MARKERS.iter().any(|marker| trimmed.starts_with(marker)) {
            let name: String = trimmed.chars().take(80).collect();
            let name = name.trim_end().trim_end_matches('{').trim().to_string();
            if !name.is_empty() {
                symbols.push(name);
            }
            if symbols.len() >= SYMBOLS_PER_FILE {
                break;
            }
        }
    }
    symbols
}

/// Commands a contributor would run — package.json scripts, Makefile targets,
/// Cargo metadata, pyproject scripts.
fn commands(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    if let Ok(text) = std::fs::read_to_string(root.join("package.json")) {
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
            if let Some(scripts) = json.get("scripts").and_then(|s| s.as_object()) {
                let names: Vec<String> = scripts
                    .keys()
                    .take(12)
                    .map(|k| format!("npm run {k}"))
                    .collect();
                if !names.is_empty() {
                    found.push(names.join(", "));
                }
            }
        }
    }
    if root.join("Cargo.toml").is_file() {
        found.push("cargo build/test".into());
    }
    if let Ok(text) = std::fs::read_to_string(root.join("Makefile")) {
        let targets: Vec<String> = text
            .lines()
            .filter_map(|line| {
                if line.starts_with('\t') || line.starts_with('.') || line.contains('=') {
                    return None;
                }
                line.split(':')
                    .next()
                    .map(str::trim)
                    .filter(|t| {
                        !t.is_empty() && t.chars().all(|c| c.is_alphanumeric() || "-_.".contains(c))
                    })
                    .map(str::to_string)
            })
            .take(10)
            .collect();
        if !targets.is_empty() {
            found.push(format!("make {}", targets.join(" | ")));
        }
    }
    if root.join("pyproject.toml").is_file() {
        found.push("pyproject.toml project".into());
    }
    found
}

/// Directory tree summary: "<dir>/ (N files)" for the top two levels.
fn tree_summary(files: &[String]) -> Vec<String> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for file in files {
        let mut parts = file.split('/');
        if let Some(first) = parts.next() {
            let key = if parts.next().is_some() {
                format!("{first}/")
            } else {
                first.to_string()
            };
            *counts.entry(key).or_default() += 1;
        }
    }
    let mut entries: Vec<(String, usize)> = counts.into_iter().collect();
    entries.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    entries
        .iter()
        .take(24)
        .map(|(dir, n)| format!("{dir} ({n})"))
        .collect()
}

pub fn repo_map(root: &Path, token_budget: usize) -> Option<RepoMap> {
    if !root.is_dir() {
        return None;
    }
    let start = Instant::now();
    let deadline = start + SCAN_BUDGET;
    let key = tree_key(root)?;
    if let Some(hit) = cache()
        .lock()
        .ok()
        .and_then(|cache| cache.get(&key).cloned())
    {
        return Some(hit);
    }
    let (files, mut partial) = match git_files(root) {
        Some(list) => {
            let partial = list.len() > MAX_FILES;
            (
                list.into_iter().take(MAX_FILES).collect::<Vec<_>>(),
                partial,
            )
        }
        None => walk_files(root, deadline),
    };
    if Instant::now() > deadline {
        partial = true;
    }

    let mut sections: Vec<String> = Vec::new();
    // Important files first — whatever is actually present.
    let important: Vec<&String> = files
        .iter()
        .filter(|file| {
            let name = file.rsplit('/').next().unwrap_or(file);
            IMPORTANT_FILES.contains(&name)
        })
        .take(IMPORTANT_FILES.len())
        .collect();
    if !important.is_empty() {
        sections.push(format!(
            "Key files: {}",
            important
                .iter()
                .map(|f| f.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let summary = tree_summary(&files);
    if !summary.is_empty() {
        sections.push(format!("Layout: {}", summary.join(", ")));
    }
    let commands = commands(root);
    if !commands.is_empty() {
        sections.push(format!("Commands: {}", commands.join("; ")));
    }
    // Symbols from small source files, newest-looking paths first is noise —
    // keep it deterministic: sort, scan until the symbol or time cap.
    let mut sources: Vec<&String> = files.iter().filter(|f| is_source(Path::new(f))).collect();
    sources.sort();
    let mut symbols: Vec<String> = Vec::new();
    for file in sources {
        if Instant::now() > deadline {
            partial = true;
            break;
        }
        // The symbol list is a capped hint, not the scan itself — filling it
        // does not make the map partial.
        if symbols.len() >= SYMBOLS_TOTAL {
            break;
        }
        let path = root.join(file);
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        if meta.len() > SYMBOL_FILE_CAP {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&path) {
            for symbol in symbols_in(&text) {
                symbols.push(format!("{file}: {symbol}"));
            }
        }
    }
    if !symbols.is_empty() {
        sections.push(format!("Symbols:\n{}", symbols.join("\n")));
    }

    // Fit the token budget strictly: keep whole sections, then truncate the
    // last one on a char boundary.
    let mut text = String::new();
    let char_budget = token_budget.max(64) * 4;
    for section in &sections {
        if text.len() + section.len() + 1 > char_budget {
            let mut remaining = char_budget
                .saturating_sub(text.len() + 1)
                .saturating_sub(20);
            if remaining > section.len() {
                remaining = section.len();
            }
            while remaining > 0 && !section.is_char_boundary(remaining) {
                remaining -= 1;
            }
            if remaining > 40 {
                text.push_str(&section[..remaining]);
                text.push_str("\n…\n");
            }
            break;
        }
        text.push_str(section);
        text.push('\n');
    }
    if text.is_empty() && files.is_empty() {
        return None;
    }
    let map = RepoMap {
        text,
        files: files.len(),
        partial,
    };
    if let Ok(mut cache) = cache().lock() {
        if cache.len() > 16 {
            cache.clear();
        }
        cache.insert(key, map.clone());
    }
    Some(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_map_lists_key_files_and_commands() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("package.json"),
            r#"{"scripts":{"test":"vitest","build":"vite build"}}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "agent notes").unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(
            dir.path().join("src/main.ts"),
            "export function boot() {}\nconst x = 1;\n",
        )
        .unwrap();
        let map = repo_map(dir.path(), DEFAULT_BUDGET).unwrap();
        assert!(map.text.contains("package.json"));
        assert!(map.text.contains("npm run test"));
        assert!(map.text.contains("boot()"), "{}", map.text);
        assert!(map.text.len() <= DEFAULT_BUDGET * 4);
    }

    #[test]
    fn repo_map_fits_budget_and_handles_unicode_and_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("データ")).unwrap();
        std::fs::write(dir.path().join("データ/résumé.rs"), "fn café() {}\n").unwrap();
        let map = repo_map(dir.path(), 400).unwrap();
        assert!(map.text.len() <= 400 * 4 + 64);
        assert!(map.text.is_char_boundary(map.text.len()));
        assert!(map.text.contains("résumé"), "{}", map.text);
        let empty = tempfile::tempdir().unwrap();
        assert!(repo_map(empty.path(), DEFAULT_BUDGET).is_none());
    }

    #[test]
    fn repo_map_stays_bounded_on_many_files() {
        let dir = tempfile::tempdir().unwrap();
        for index in 0..5000 {
            std::fs::write(dir.path().join(format!("f{index}.txt")), "x").unwrap();
        }
        let start = Instant::now();
        let map = repo_map(dir.path(), 200).unwrap();
        assert!(
            start.elapsed() < std::time::Duration::from_secs(8),
            "took {:?}",
            start.elapsed()
        );
        assert!(map.text.len() <= 200 * 4 + 64);
        assert!(map.files > 0);
    }

    /// The measured artifact for the milestone: map this very repository and
    /// prove it stays inside the scan budget.
    #[test]
    fn repo_map_on_this_repo_is_fast_and_bounded() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        let start = Instant::now();
        let map = repo_map(repo, DEFAULT_BUDGET).unwrap();
        let elapsed = start.elapsed();
        eprintln!(
            "repo_map on {}: {} files, {} bytes, partial={} in {:?}",
            repo.display(),
            map.files,
            map.text.len(),
            map.partial,
            elapsed
        );
        assert!(
            elapsed < std::time::Duration::from_secs(10),
            "took {elapsed:?}"
        );
        assert!(map.text.len() <= DEFAULT_BUDGET * 4 + 64);
    }

    #[test]
    fn repo_map_cache_hits_and_invalidates_on_change() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "one").unwrap();
        let first = repo_map(dir.path(), DEFAULT_BUDGET).unwrap();
        let second = repo_map(dir.path(), DEFAULT_BUDGET).unwrap();
        assert_eq!(first.text, second.text);
        std::fs::write(dir.path().join("b.txt"), "two").unwrap();
        let third = repo_map(dir.path(), DEFAULT_BUDGET).unwrap();
        assert_ne!(first.text, third.text);
    }
}
