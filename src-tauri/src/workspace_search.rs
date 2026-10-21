//! Scoped, deterministic, resumable text/glob search over capability handles.
//! Globs are parsed by `glob` but matched with the linear-time regex engine,
//! avoiding recursive wildcard backtracking on model-supplied patterns.
use super::{open_file_no_links, valid_path, Workspace};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use cap_fs_ext::DirExt;
use cap_std::fs::{Dir, OpenOptions};
use regex::{Regex, RegexBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    io::Read,
    time::{Duration, Instant},
};

const MAX_ENTRIES: usize = 20_000;
const MAX_FILES: usize = 5_000;
const MAX_DEPTH: usize = 32;
const MAX_FILE_BYTES: usize = 1024 * 1024;
const READ_BUDGET: usize = 8 * 1024 * 1024;
const PAGE_BYTES: usize = 48 * 1024;

fn yes() -> bool {
    true
}
fn default_limit() -> usize {
    50
}
fn default_context() -> usize {
    2
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ContentArgs {
    query: String,
    path: Option<String>,
    glob: Option<String>,
    #[serde(default)]
    regex: bool,
    #[serde(default = "yes")]
    case_sensitive: bool,
    #[serde(default = "default_context")]
    context_lines: usize,
    #[serde(default = "default_limit")]
    limit: usize,
    cursor: Option<String>,
    #[serde(default)]
    include_hidden: bool,
    #[serde(default)]
    include_generated: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GlobArgs {
    pattern: String,
    path: Option<String>,
    #[serde(default = "yes")]
    case_sensitive: bool,
    #[serde(default = "default_limit")]
    limit: usize,
    cursor: Option<String>,
    #[serde(default)]
    include_hidden: bool,
    #[serde(default)]
    include_generated: bool,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    version: u8,
    key: String,
    path: String,
    next_line: usize,
    sha256: Option<String>,
}

#[derive(Default)]
struct Walk {
    files: Vec<String>,
    entries: usize,
    warnings: Vec<String>,
    excluded: usize,
    started: Option<Instant>,
}

impl Walk {
    fn warning(&mut self, warning: impl Into<String>) {
        let warning = warning.into();
        if self.warnings.len() < 16 && !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }
}

fn engine(pattern: &str, case_sensitive: bool) -> Result<Regex, String> {
    RegexBuilder::new(pattern)
        .case_insensitive(!case_sensitive)
        .size_limit(1024 * 1024)
        .dfa_size_limit(2 * 1024 * 1024)
        .build()
        .map_err(|error| format!("Invalid or oversized search pattern: {error}"))
}

/// Translate the standard glob parser's accepted syntax to a bounded matcher.
/// `*`/`?` and bracket classes never cross `/`; `**/` includes zero directories.
fn glob_engine(pattern: &str, case_sensitive: bool) -> Result<Regex, String> {
    if pattern.is_empty()
        || pattern.len() > 4096
        || pattern.starts_with('/')
        || pattern.contains(['\\', ':'])
        || pattern.chars().any(char::is_control)
        || pattern.split('/').any(|part| part == "..")
    {
        return Err(
            "Use a relative glob of 1–4096 bytes with forward slashes and no parent traversal."
                .into(),
        );
    }
    glob::Pattern::new(pattern).map_err(|error| format!("Invalid glob: {error}"))?;
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut expression = String::from("\\A(?:");
    let mut index = 0;
    while index < chars.len() {
        match chars[index] {
            '*' if chars.get(index + 1) == Some(&'*') => {
                if chars.get(index + 2) == Some(&'/') {
                    expression.push_str("(?:[^/]+/)*");
                    index += 3;
                } else {
                    expression.push_str(".*");
                    index += 2;
                }
            }
            '*' => {
                expression.push_str("[^/]*");
                index += 1;
            }
            '?' => {
                expression.push_str("[^/]");
                index += 1;
            }
            '[' => {
                let negated = chars.get(index + 1) == Some(&'!');
                let start = index + if negated { 2 } else { 1 };
                let end = chars
                    .iter()
                    .enumerate()
                    .skip(start + 1)
                    .find(|(_, value)| **value == ']')
                    .map(|(position, _)| position)
                    .ok_or("Invalid glob character class.")?;
                let mut body = String::new();
                let mut position = start;
                while position < end {
                    if position + 2 < end && chars[position + 1] == '-' {
                        if chars[position] <= chars[position + 2] {
                            body.push_str(&regex::escape(&chars[position].to_string()));
                            body.push('-');
                            body.push_str(&regex::escape(&chars[position + 2].to_string()));
                        }
                        position += 3;
                    } else {
                        body.push_str(&regex::escape(&chars[position].to_string()));
                        position += 1;
                    }
                }
                if body.is_empty() {
                    expression.push_str(if negated { "[^/]" } else { "\\b\\B" });
                } else {
                    expression.push_str(if negated { "[[^" } else { "[[" });
                    expression.push_str(&body);
                    expression.push_str("]&&[^/]]");
                }
                index = end + 1;
            }
            value => {
                expression.push_str(&regex::escape(&value.to_string()));
                index += 1;
            }
        }
    }
    expression.push_str(")\\z");
    engine(&expression, case_sensitive)
}

fn walk(
    directory: &Dir,
    prefix: &str,
    depth: usize,
    args: &ContentArgs,
    filter: Option<&Regex>,
    output: &mut Walk,
) {
    if depth > MAX_DEPTH {
        output.warning("Directory depth limit reached; narrow path to search deeper.");
        return;
    }
    let entries = match directory.entries() {
        Ok(entries) => entries,
        Err(error) => {
            output.warning(format!("Could not enumerate {prefix}: {error}"));
            return;
        }
    };
    let mut names = Vec::new();
    for entry in entries {
        if output.entries >= MAX_ENTRIES
            || output
                .started
                .is_some_and(|start| start.elapsed() > Duration::from_secs(2))
        {
            output.warning("Directory scan limit reached; narrow path or glob. The result does not cover the whole scope.");
            break;
        }
        output.entries += 1;
        match entry {
            Ok(entry) => {
                if let Ok(name) = entry.file_name().into_string() {
                    names.push(name);
                } else {
                    output.excluded += 1;
                }
            }
            Err(error) => output.warning(format!("Directory entry could not be read: {error}")),
        }
    }
    names.sort();
    for name in names {
        if output
            .started
            .is_some_and(|start| start.elapsed() > Duration::from_secs(2))
        {
            output.warning("Directory scan time limit reached; narrow path or glob.");
            break;
        }
        if !args.include_hidden && name.starts_with('.') {
            continue;
        }
        if !args.include_generated && (super::is_generated_dir(&name) || name == ".venv") {
            continue;
        }
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if valid_path(&path, false).is_err() {
            output.excluded += 1;
            continue;
        }
        let metadata = match directory.symlink_metadata(&name) {
            Ok(metadata) => metadata,
            Err(error) => {
                output.warning(format!("Could not inspect {path}: {error}"));
                continue;
            }
        };
        if metadata.is_symlink() {
            output.excluded += 1;
            continue;
        }
        if metadata.is_dir() {
            if output.entries >= MAX_ENTRIES {
                output.warning("Directory entry limit reached; narrow path.");
                continue;
            }
            match directory.open_dir_nofollow(&name) {
                Ok(child) => walk(&child, &path, depth + 1, args, filter, output),
                Err(error) => output.warning(format!("Could not open {path}: {error}")),
            }
        } else if metadata.is_file() {
            if filter.is_some_and(|pattern| !pattern.is_match(&path)) {
                continue;
            }
            if output.files.len() >= MAX_FILES {
                output.warning("File scan limit reached; narrow path or glob.");
                break;
            }
            output.files.push(path);
        }
    }
}

fn encode(cursor: &Cursor) -> Result<String, String> {
    Ok(URL_SAFE_NO_PAD.encode(serde_json::to_vec(cursor).map_err(|error| error.to_string())?))
}

fn decode(value: Option<&str>, key: &str) -> Result<Option<Cursor>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.len() > 8192 {
        return Err("Search cursor is too long.".into());
    }
    let cursor: Cursor = serde_json::from_slice(
        &URL_SAFE_NO_PAD
            .decode(value)
            .map_err(|_| "Search cursor is not valid base64.")?,
    )
    .map_err(|_| "Search cursor is not valid JSON.")?;
    if cursor.version != 1 || cursor.key != key || cursor.next_line > MAX_FILE_BYTES {
        return Err("Search cursor belongs to a different query/workspace, or its version/position is invalid.".into());
    }
    valid_path(&cursor.path, false)?;
    Ok(Some(cursor))
}

fn excerpt(line: &str, column: usize, size: usize) -> (String, usize, bool) {
    let start = column.saturating_sub(size / 4);
    let text = line.chars().skip(start).take(size).collect::<String>();
    let truncated = start > 0 || line.chars().count() > start + size;
    (text, start + 1, truncated)
}

impl Workspace {
    pub(super) fn search_content(&self, arguments: Value) -> Result<Value, String> {
        let args: ContentArgs =
            serde_json::from_value(arguments).map_err(|error| error.to_string())?;
        self.search_page(args, false)
    }

    pub(super) fn glob_files(&self, arguments: Value) -> Result<Value, String> {
        let args: GlobArgs =
            serde_json::from_value(arguments).map_err(|error| error.to_string())?;
        self.search_page(
            ContentArgs {
                query: String::new(),
                path: args.path,
                glob: Some(args.pattern),
                regex: false,
                case_sensitive: args.case_sensitive,
                context_lines: 0,
                limit: args.limit,
                cursor: args.cursor,
                include_hidden: args.include_hidden,
                include_generated: args.include_generated,
            },
            true,
        )
    }

    fn search_page(&self, args: ContentArgs, files_only: bool) -> Result<Value, String> {
        if (!files_only && args.query.is_empty())
            || args.query.len() > 4096
            || !(1..=200).contains(&args.limit)
            || args.context_lines > 10
        {
            return Err(
                "Use a nonempty query up to 4096 bytes, limit 1–200 and context_lines 0–10.".into(),
            );
        }
        let path = args.path.as_deref().unwrap_or(".");
        valid_path(path, true)?;
        let directory = if path == "." {
            self.directory.try_clone()
        } else {
            super::parent_no_links(&self.directory, path)
                .and_then(|(parent, name)| parent.open_dir_nofollow(name))
        }
        .map_err(|error| error.to_string())?;
        let mut signature = serde_json::to_value(&args).map_err(|error| error.to_string())?;
        signature.as_object_mut().unwrap().remove("cursor");
        signature.as_object_mut().unwrap().remove("limit");
        let key = format!(
            "{:x}",
            Sha256::digest(
                format!("{}:{files_only}:{signature}", self.root.to_string_lossy()).as_bytes()
            )
        );
        let cursor = decode(args.cursor.as_deref(), &key)?;
        let pattern = if let Some(glob) = &args.glob {
            Some(glob_engine(glob, args.case_sensitive)?)
        } else {
            None
        };
        let matcher = if files_only {
            None
        } else {
            Some(engine(
                &if args.regex {
                    args.query.clone()
                } else {
                    regex::escape(&args.query)
                },
                args.case_sensitive,
            )?)
        };
        let mut walked = Walk {
            started: Some(Instant::now()),
            ..Default::default()
        };
        walk(
            &directory,
            if path == "." { "" } else { path },
            0,
            &args,
            pattern.as_ref(),
            &mut walked,
        );
        walked.files.sort();
        if cursor
            .as_ref()
            .is_some_and(|cursor| cursor.next_line > 0 && !walked.files.contains(&cursor.path))
        {
            return Err("The cursor's source file disappeared or could not be enumerated. Restart or narrow this query.".into());
        }
        let files = std::mem::take(&mut walked.files);
        let mut matches = Vec::<Value>::new();
        let mut results = Vec::<Value>::new();
        let mut hits = Vec::<String>::new();
        let mut next_cursor = None;
        let mut marker = cursor.unwrap_or_default();
        let mut scanned_files = 0;
        let mut bytes_read = 0;
        let mut excluded_files = walked.excluded;
        let mut page_bytes = 0;
        let started = Instant::now();
        'files: for relative in &files {
            if !marker.path.is_empty()
                && (relative < &marker.path || (relative == &marker.path && marker.next_line == 0))
            {
                continue;
            }
            if pattern
                .as_ref()
                .is_some_and(|pattern| !pattern.is_match(relative))
            {
                continue;
            }
            if scanned_files > 0 && started.elapsed() > Duration::from_secs(2) {
                if !marker.path.is_empty() {
                    next_cursor = Some(encode(&marker)?);
                }
                break;
            }
            if files_only {
                let entry = json!({"path":relative,"kind":"file"});
                let size = serde_json::to_vec(&entry)
                    .map_err(|error| error.to_string())?
                    .len();
                if !results.is_empty() && page_bytes + size > PAGE_BYTES {
                    next_cursor = Some(encode(&marker)?);
                    break;
                }
                page_bytes += size;
                results.push(entry);
                scanned_files += 1;
                marker = Cursor {
                    version: 1,
                    key: key.clone(),
                    path: relative.clone(),
                    next_line: 0,
                    sha256: None,
                };
                if results.len() >= args.limit {
                    next_cursor = Some(encode(&marker)?);
                    break;
                }
                continue;
            }
            let file = match open_file_no_links(
                &self.directory,
                relative,
                OpenOptions::new().read(true),
            ) {
                Ok(file) => file,
                Err(error) => {
                    walked.warning(format!("Could not read {relative}: {error}"));
                    continue;
                }
            };
            let size = file.metadata().map_err(|error| error.to_string())?.len() as usize;
            if size > MAX_FILE_BYTES {
                walked.warning(format!(
                    "Skipped {relative}: text files over 1 MiB need a narrower reader."
                ));
                continue;
            }
            if bytes_read + size > READ_BUDGET {
                if !marker.path.is_empty() {
                    next_cursor = Some(encode(&marker)?);
                }
                break;
            }
            let mut data = Vec::new();
            file.take((MAX_FILE_BYTES + 1).min(READ_BUDGET - bytes_read + 1) as u64)
                .read_to_end(&mut data)
                .map_err(|error| format!("Could not read {relative}: {error}"))?;
            if data.len() > READ_BUDGET - bytes_read {
                if !marker.path.is_empty() {
                    next_cursor = Some(encode(&marker)?);
                }
                break;
            }
            bytes_read += data.len();
            scanned_files += 1;
            let resume_line = if marker.path == *relative {
                marker.next_line
            } else {
                0
            };
            let hash = format!("{:x}", Sha256::digest(&data));
            if resume_line > 0 && marker.sha256.as_deref() != Some(hash.as_str()) {
                return Err(format!(
                    "{relative} changed between search pages. Restart this query without a cursor."
                ));
            }
            marker = Cursor {
                version: 1,
                key: key.clone(),
                path: relative.clone(),
                next_line: 0,
                sha256: None,
            };
            if data.len() > MAX_FILE_BYTES {
                walked.warning(format!("Skipped {relative}: file grew beyond 1 MiB."));
                continue;
            }
            if data.iter().take(1024).any(|byte| *byte == 0) {
                excluded_files += 1;
                continue;
            }
            let text = match std::str::from_utf8(&data) {
                Ok(text) => text,
                Err(_) => {
                    excluded_files += 1;
                    continue;
                }
            };
            let mut lines = text.lines().enumerate();
            let mut before = VecDeque::new();
            while let Some((line_number, line)) = lines.next() {
                if line_number >= resume_line {
                    if let Some(found) = matcher.as_ref().unwrap().find(line) {
                        let column = line[..found.start()].chars().count();
                        let (snippet, start_column, text_truncated) = excerpt(line, column, 1000);
                        let after = lines.clone().take(args.context_lines).map(|(index, text)| json!({"line":index+1,"text":text.chars().take(256).collect::<String>()})).collect::<Vec<_>>();
                        let entry = json!({"path":relative,"line":line_number+1,"column":column+1,"text":snippet,
                            "textStartColumn":start_column,"textTruncated":text_truncated,"before":before,"after":after,"sha256":hash});
                        let hit = format!(
                            "{relative}:{}: {}",
                            line_number + 1,
                            entry["text"].as_str().unwrap_or("")
                        );
                        let size = serde_json::to_vec(&entry)
                            .map_err(|error| error.to_string())?
                            .len()
                            + serde_json::to_vec(&hit)
                                .map_err(|error| error.to_string())?
                                .len()
                            + 32;
                        if !matches.is_empty() && page_bytes + size > PAGE_BYTES {
                            marker.next_line = line_number;
                            marker.sha256 = Some(hash);
                            next_cursor = Some(encode(&marker)?);
                            break 'files;
                        }
                        page_bytes += size;
                        matches.push(entry);
                        hits.push(hit);
                        if matches.len() >= args.limit {
                            marker.next_line = line_number + 1;
                            marker.sha256 = Some(hash);
                            next_cursor = Some(encode(&marker)?);
                            break 'files;
                        }
                    }
                }
                before.push_back(
                    json!({"line":line_number+1,"text":line.chars().take(256).collect::<String>()}),
                );
                while before.len() > args.context_lines {
                    before.pop_front();
                }
            }
        }
        // A cursor points at remaining input, not a promise of another match.
        Ok(
            json!({"matches":matches,"results":results,"hits":hits,"nextCursor":next_cursor,"hasMore":next_cursor.is_some(),
            "incomplete":!walked.warnings.is_empty(),"truncated":next_cursor.is_some() || !walked.warnings.is_empty(),"warnings":walked.warnings,
            "scannedFiles":scanned_files,"scannedEntries":walked.entries,"bytesRead":bytes_read,"excludedFiles":excluded_files,"snapshot":"live"}),
        )
    }
}

pub(super) fn definitions() -> Vec<(&'static str, &'static str, Value, Vec<&'static str>)> {
    let shared = json!({"path":{"type":"string","maxLength":4096,"description":"Relative directory scope; defaults to workspace root."},
        "limit":{"type":"integer","minimum":1,"maximum":200,"default":50},"cursor":{"type":"string","maxLength":8192,"description":"Opaque nextCursor from the same query/workspace; omit to restart."},
        "case_sensitive":{"type":"boolean","default":true},"include_hidden":{"type":"boolean","default":false},"include_generated":{"type":"boolean","default":false}});
    let mut content = shared.clone();
    content["query"] = json!({"type":"string","minLength":1,"maxLength":1024,"description":"Literal text or Rust regex, matched per line."});
    content["regex"] = json!({"type":"boolean","default":false});
    content["glob"] = json!({"type":"string","maxLength":1024,"description":"Workspace-relative glob, e.g. src/**/*.rs; * and ? do not cross directories."});
    content["context_lines"] = json!({"type":"integer","minimum":0,"maximum":10,"default":2});
    let mut files = shared;
    files["pattern"] = json!({"type":"string","minLength":1,"maxLength":1024});
    vec![
        ("search_files", "Find workspace filenames by a literal case-insensitive substring. This legacy filename search is bounded; use glob_files for patterns and pagination.", json!({"query":{"type":"string","maxLength":200},"include_generated":{"type":"boolean","default":false}}), vec!["query"]),
        ("search_content", "Search scoped workspace UTF-8 files with literal/regex, case control, glob filtering and line context. Returns multiple matching lines per file with source hashes. Continue nextCursor until hasMore is false; hasMore means input remains, not guaranteed matches. Inspect incomplete/warnings before concluding there are no matches. Hidden/generated paths are excluded by default; links and credential paths are never followed.", content, vec!["query"]),
        ("glob_files", "Find workspace files by a standard relative glob, including **, *, ? and bracket classes. Returns bounded sorted pages. Continue nextCursor until hasMore is false; incomplete/warnings identify hard scan limits requiring a narrower path. Uses the workspace path and credential policies; never follows links.", files, vec!["pattern"]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_glob_engine_matches_standard_glob_syntax() {
        let options = glob::MatchOptions {
            case_sensitive: true,
            require_literal_separator: true,
            require_literal_leading_dot: false,
        };
        for pattern in [
            "**/*.rs",
            "src/**/file?.[rt]s",
            "**/a[!0-3].txt",
            "[]].txt",
            "[[]a.txt",
            "**",
            "*",
            "[z-a].rs",
            "[!z-a].rs",
            "**/**/a*",
        ] {
            let expected = glob::Pattern::new(pattern).unwrap();
            let actual = glob_engine(pattern, true).unwrap();
            for path in [
                "a.rs",
                "src/a.rs",
                "src/file1.rs",
                "src/nested/file2.ts",
                "a5.txt",
                "dir/a1.txt",
                "].txt",
                "[a.txt",
                "z.rs",
                "A.rs",
                "nested/.hidden/a.rs",
            ] {
                assert_eq!(
                    actual.is_match(path),
                    expected.matches_with(path, options),
                    "{pattern}: {path}"
                );
            }
        }
        assert!(glob_engine("**/*.RS", false)
            .unwrap()
            .is_match("src/code.rs"));
        for pattern in ["", "../*", "/absolute/*", "src\\*", "[bad", "a**b", "***"] {
            assert!(glob_engine(pattern, true).is_err(), "{pattern}");
        }
        // Standard parsing plus linear matching avoids exponential wildcard recursion.
        let adversarial = format!("{}z", "a*".repeat(200));
        assert!(!glob_engine(&adversarial, true)
            .unwrap()
            .is_match(&"a".repeat(4000)));
    }

    fn fixture() -> tempfile::TempDir {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(directory.path().join("src/nested")).unwrap();
        std::fs::write(
            directory.path().join("src/a.rs"),
            "alpha\nneedle one\nmiddle\nNEEDLE two\nneedle three\nend\n",
        )
        .unwrap();
        std::fs::write(
            directory.path().join("src/nested/b.rs"),
            "needle foo\nneedle bar\n[x] literal\n",
        )
        .unwrap();
        std::fs::write(directory.path().join("README.md"), "needle outside scope").unwrap();
        directory
    }

    #[test]
    fn content_search_paginates_multiple_lines_deterministically_with_context_and_hashes() {
        let directory = fixture();
        let workspace = Workspace::open(directory.path().to_str().unwrap()).unwrap();
        let mut args = json!({"query":"needle","path":"src","glob":"src/**/*.rs","case_sensitive":false,"context_lines":1,"limit":1});
        let first = workspace.call("search_content", args.clone()).unwrap();
        assert_eq!(first["matches"][0]["path"], "src/a.rs");
        assert_eq!(first["matches"][0]["line"], 2);
        assert_eq!(first["matches"][0]["before"][0]["text"], "alpha");
        assert_eq!(first["matches"][0]["after"][0]["text"], "middle");
        assert_eq!(first["matches"][0]["sha256"].as_str().unwrap().len(), 64);
        let mut positions = vec![("src/a.rs".to_string(), 2)];
        let mut cursor = first["nextCursor"].clone();
        for _ in 0..10 {
            args["cursor"] = cursor;
            let page = workspace.call("search_content", args.clone()).unwrap();
            assert_eq!(page["incomplete"], false, "{page}");
            for found in page["matches"].as_array().unwrap() {
                positions.push((
                    found["path"].as_str().unwrap().to_string(),
                    found["line"].as_u64().unwrap(),
                ));
            }
            if page["hasMore"] == false {
                break;
            }
            cursor = page["nextCursor"].clone();
        }
        assert_eq!(
            positions,
            vec![
                ("src/a.rs".into(), 2),
                ("src/a.rs".into(), 4),
                ("src/a.rs".into(), 5),
                ("src/nested/b.rs".into(), 1),
                ("src/nested/b.rs".into(), 2)
            ]
        );
        let regex = workspace
            .call(
                "search_content",
                json!({"query":"needle (one|foo)","regex":true}),
            )
            .unwrap();
        assert_eq!(regex["matches"].as_array().unwrap().len(), 2);
        let literal = workspace
            .call("search_content", json!({"query":"[x]"}))
            .unwrap();
        assert_eq!(literal["matches"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn cursors_reject_query_workspace_and_source_changes() {
        let directory = fixture();
        let workspace = Workspace::open(directory.path().to_str().unwrap()).unwrap();
        let page = workspace
            .call("search_content", json!({"query":"needle","limit":1}))
            .unwrap();
        let cursor = page["nextCursor"].clone();
        assert!(workspace
            .call(
                "search_content",
                json!({"query":"different","cursor":cursor})
            )
            .unwrap_err()
            .contains("different query/workspace"));
        let other = fixture();
        let other = Workspace::open(other.path().to_str().unwrap()).unwrap();
        assert!(other
            .call("search_content", json!({"query":"needle","cursor":cursor}))
            .is_err());
        std::fs::write(directory.path().join("README.md"), "needle changed source").unwrap();
        assert!(workspace
            .call("search_content", json!({"query":"needle","cursor":cursor}))
            .unwrap_err()
            .contains("changed between search pages"));
        for args in [
            json!({"query":"needle","cursor":"bad cursor"}),
            json!({"query":"[","regex":true}),
            json!({"query":"needle","glob":"[bad"}),
            json!({"query":"needle","path":"../"}),
            json!({"query":"needle","limit":0}),
            json!({"query":"needle","context_lines":11}),
            json!({"query":"needle","unknown":true}),
        ] {
            assert!(
                workspace.call("search_content", args.clone()).is_err(),
                "{args}"
            );
        }
    }

    #[test]
    fn glob_pages_are_sorted_query_bound_and_legacy_filename_search_remains_compatible() {
        let directory = fixture();
        let workspace = Workspace::open(directory.path().to_str().unwrap()).unwrap();
        let first = workspace
            .call("glob_files", json!({"pattern":"**/*.rs","limit":1}))
            .unwrap();
        assert_eq!(first["results"], json!([{"path":"src/a.rs","kind":"file"}]));
        let next = workspace
            .call(
                "glob_files",
                json!({"pattern":"**/*.rs","cursor":first["nextCursor"]}),
            )
            .unwrap();
        assert_eq!(
            next["results"],
            json!([{"path":"src/nested/b.rs","kind":"file"}])
        );
        assert_eq!(next["hasMore"], false);
        let legacy = workspace
            .call("search_files", json!({"query":"README"}))
            .unwrap();
        assert_eq!(
            legacy["results"],
            json!([{"path":"README.md","kind":"file"}])
        );
        for tool in std::sync::Arc::new(workspace)
            .tools()
            .iter()
            .filter(|tool| {
                matches!(
                    tool.tool.name.as_str(),
                    "search_files" | "search_content" | "glob_files"
                )
            })
        {
            assert!(tool.trusted_read());
            assert_eq!(tool.tool.input_schema["additionalProperties"], false);
            assert!(tool.prepare_arguments(json!({"unknown":true})).is_err());
        }
    }

    #[test]
    fn read_budget_pages_progress_and_oversized_text_is_explicitly_incomplete() {
        let directory = tempfile::tempdir().unwrap();
        let mut text = "needle\n".to_string();
        text.push_str(&"x".repeat(MAX_FILE_BYTES - text.len()));
        for index in 0..9 {
            std::fs::write(directory.path().join(format!("{index:02}.txt")), &text).unwrap();
        }
        let workspace = Workspace::open(directory.path().to_str().unwrap()).unwrap();
        let first = workspace
            .call(
                "search_content",
                json!({"query":"needle","context_lines":0,"limit":200}),
            )
            .unwrap();
        assert_eq!(first["matches"].as_array().unwrap().len(), 8, "{first}");
        assert_eq!(first["hasMore"], true);
        let next = workspace.call("search_content", json!({"query":"needle","context_lines":0,"limit":200,"cursor":first["nextCursor"]})).unwrap();
        assert_eq!(next["matches"].as_array().unwrap().len(), 1);
        assert_eq!(next["hasMore"], false);
        std::fs::write(
            directory.path().join("large.txt"),
            "x".repeat(MAX_FILE_BYTES + 1),
        )
        .unwrap();
        let large = workspace
            .call(
                "search_content",
                json!({"query":"needle","glob":"large.txt"}),
            )
            .unwrap();
        assert_eq!(large["incomplete"], true);
        assert!(large["warnings"].to_string().contains("over 1 MiB"));
    }

    #[cfg(unix)]
    #[test]
    fn search_refuses_links_and_credentials_and_controls_hidden_generated_scope() {
        use std::os::unix::fs::symlink;
        let directory = fixture();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "needle super-secret").unwrap();
        symlink(outside.path(), directory.path().join("linked")).unwrap();
        symlink(
            outside.path().join("secret.txt"),
            directory.path().join("link.txt"),
        )
        .unwrap();
        std::fs::write(directory.path().join(".env"), "needle super-secret").unwrap();
        std::fs::write(directory.path().join(".hidden"), "needle hidden fixture").unwrap();
        std::fs::create_dir(directory.path().join("node_modules")).unwrap();
        std::fs::write(
            directory.path().join("node_modules/module.js"),
            "needle generated fixture",
        )
        .unwrap();
        let workspace = Workspace::open(directory.path().to_str().unwrap()).unwrap();
        let default = workspace
            .call("search_content", json!({"query":"needle","limit":200}))
            .unwrap();
        assert!(!default["matches"].to_string().contains("fixture"));
        let all = workspace.call("search_content", json!({"query":"needle","include_hidden":true,"include_generated":true,"limit":200})).unwrap();
        assert!(all["matches"].to_string().contains("hidden fixture"));
        assert!(all["matches"].to_string().contains("generated fixture"));
        assert!(!all.to_string().contains("super-secret"));
        assert!(workspace
            .call("search_content", json!({"query":"needle","path":"linked"}))
            .is_err());
    }

    #[tokio::test]
    async fn model_facing_search_runs_through_schema_dispatch_and_catalog_policy() {
        let directory = fixture();
        let state = crate::test_support::app_state(directory.path());
        crate::test_support::select_fixture_model(&state, directory.path());
        let plan = crate::chat::assemble_turn(
            &state,
            "",
            "Search source",
            vec!["__workspace".into()],
            Some(vec![]),
            Some(crate::presets::STANDARD),
            None,
        )
        .await
        .unwrap();
        for alias in [
            "workspace_search_files",
            "workspace_search_content",
            "workspace_glob_files",
        ] {
            let tool = plan
                .tools
                .iter()
                .find(|tool| tool.alias == alias)
                .expect("Workspace must advertise the new read tools");
            assert!(tool.trusted_read());
            assert!(!tool.requires_approval(&json!({})));
            assert_eq!(
                tool.definition()["function"]["parameters"]["additionalProperties"],
                false
            );
        }
        let mut calls = crate::tool_calls::ToolCalls::default();
        let arguments = json!({"query":"needle","path":"src","case_sensitive":false});
        calls.push(&json!({"tool_calls":[{"index":0,"id":"search-fixture","type":"function","function":{"name":"workspace_search_content","arguments":arguments.to_string()}}]})).unwrap();
        let call = calls.finish().unwrap().remove(0);
        let tool = plan
            .tools
            .iter()
            .find(|tool| tool.alias == call.name)
            .unwrap();
        let prepared = tool.prepare_arguments(call.arguments).unwrap();
        let result = tool.call(prepared).await.unwrap();
        assert_eq!(result["matches"].as_array().unwrap().len(), 5, "{result}");
        assert!(tool
            .prepare_arguments(json!({"query":"needle","unexpected":true}))
            .is_err());
        state
            .database()
            .unwrap()
            .save_setting(crate::capabilities::DISABLED_KEY, &vec!["workspace_files"])
            .unwrap();
        let disabled = crate::chat::assemble_turn(
            &state,
            "",
            "Search source",
            vec!["__workspace".into()],
            Some(vec![]),
            Some(crate::presets::STANDARD),
            None,
        )
        .await
        .unwrap();
        assert!(!disabled
            .tools
            .iter()
            .any(|tool| tool.connector == "Workspace"));
        let unselected = crate::chat::assemble_turn(
            &state,
            "",
            "Search source",
            vec![],
            Some(vec![]),
            Some(crate::presets::STANDARD),
            None,
        )
        .await
        .unwrap();
        assert!(!unselected
            .tools
            .iter()
            .any(|tool| tool.alias.starts_with("workspace_")));
    }

    #[test]
    fn unicode_columns_long_lines_and_escaped_result_pages_are_bounded_and_resumable() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("unicode.txt"),
            format!("{}needle\n", "🦀".repeat(1200)),
        )
        .unwrap();
        let workspace = Workspace::open(directory.path().to_str().unwrap()).unwrap();
        let unicode = workspace
            .call("search_content", json!({"query":"needle"}))
            .unwrap();
        assert_eq!(unicode["matches"][0]["column"], 1201);
        assert_eq!(unicode["matches"][0]["textTruncated"], true);
        assert!(unicode["matches"][0]["text"]
            .as_str()
            .unwrap()
            .contains("needle"));
        let line = format!("needle{}\n", "\\\"".repeat(600));
        std::fs::write(directory.path().join("escaped.txt"), line.repeat(40)).unwrap();
        let mut args =
            json!({"query":"needle","glob":"escaped.txt","context_lines":10,"limit":200});
        let mut found = Vec::new();
        for _ in 0..100 {
            let page = workspace.call("search_content", args.clone()).unwrap();
            assert!(serde_json::to_vec(&page).unwrap().len() <= PAGE_BYTES + 4096);
            for entry in page["matches"].as_array().unwrap() {
                found.push(entry["line"].as_u64().unwrap());
            }
            if page["hasMore"] == false {
                break;
            }
            args["cursor"] = page["nextCursor"].clone();
        }
        assert_eq!(found, (1..=40).collect::<Vec<_>>());
    }
}
