//! First-class git tools over the user's workspace repository
//! (docs/EXTENSIONS.md §2.3).
//!
//! `git_status`, `git_diff`, `git_log` and `git_branch` are read-only and
//! trusted reads. `git_commit` always asks, even under Full access, and only
//! the top-level agent may call it. Nothing here pushes, resets, checks out,
//! rebases, amends or deletes branches: those operations are intentionally
//! absent rather than guarded.

use serde_json::{json, Value};
use std::time::Duration;

const OUTPUT_LIMIT: usize = 65_536;

/// Run git in `dir` with output bounded and no pager, prompt or colour.
/// Inherited `GIT_*` variables are removed so the workspace, not the
/// environment, decides which repository is used.
pub(crate) async fn run(dir: &str, args: &[&str], envs: &[(&str, &str)], timeout: Duration) -> Result<String, String> {
    let mut command = tokio::process::Command::new("git");
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .envs(envs.iter().copied())
        .arg("-C")
        .arg(dir)
        .args(["--no-pager", "-c", "color.ui=false", "-c", "core.quotepath=false"])
        .args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let output = tokio::time::timeout(timeout, command.output())
        .await
        .map_err(|_| format!("git {} timed out.", args.first().unwrap_or(&"")))?
        .map_err(|error| format!("Git is not available: {error}"))?;
    if !output.status.success() {
        let stderr: String = String::from_utf8_lossy(&output.stderr).chars().take(2000).collect();
        return Err(format!("git {} failed: {}", args.first().unwrap_or(&""), stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn bounded(text: String) -> (String, bool) {
    if text.len() <= OUTPUT_LIMIT {
        return (text, false);
    }
    let mut cut = OUTPUT_LIMIT;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    (text[..cut].to_string(), true)
}

/// Optional workspace-relative path argument, validated like the file tools.
fn path_arg(args: &Value) -> Result<Option<String>, String> {
    match args.get("path").and_then(Value::as_str) {
        None | Some("") | Some(".") => Ok(None),
        Some(path) => {
            crate::workspace::valid_path(path, false)?;
            Ok(Some(path.to_string()))
        }
    }
}

/// A revision the model may name: plain ref characters, never an option.
fn rev_arg(args: &Value, key: &str) -> Result<Option<String>, String> {
    let Some(rev) = args.get(key).and_then(Value::as_str).filter(|rev| !rev.is_empty()) else {
        return Ok(None);
    };
    let valid = rev.len() <= 200
        && !rev.starts_with('-')
        && !rev.contains("..")
        && rev.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '/' | '~' | '^' | '-'));
    if !valid {
        return Err(format!("'{key}' must be a branch, tag or commit id."));
    }
    Ok(Some(rev.to_string()))
}

async fn repository_root(workspace: &str) -> Result<String, String> {
    if workspace.is_empty() {
        return Err("Choose a workspace folder before using git tools.".into());
    }
    run(workspace, &["rev-parse", "--show-toplevel"], &[], Duration::from_secs(10))
        .await
        .map(|root| root.trim().to_string())
        .map_err(|_| "The workspace is not inside a git repository.".to_string())
}

/// Parse `git status --porcelain=v2 --branch -z`.
pub fn parse_status(raw: &str) -> Value {
    let mut branch = json!({});
    let mut entries = Vec::new();
    let mut records = raw.split('\0').filter(|record| !record.is_empty());
    while let Some(record) = records.next() {
        if let Some(header) = record.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.head" => branch["head"] = json!(value),
                "branch.upstream" => branch["upstream"] = json!(value),
                "branch.oid" => branch["commit"] = json!(value),
                "branch.ab" => {
                    let mut counts = value.split(' ').map(|part| part.trim_start_matches(['+', '-']).parse::<u64>().ok());
                    branch["ahead"] = json!(counts.next().flatten());
                    branch["behind"] = json!(counts.next().flatten());
                }
                _ => {}
            }
            continue;
        }
        let kind = record.chars().next().unwrap_or(' ');
        let entry = match kind {
            '1' => record.splitn(9, ' ').collect::<Vec<_>>().get(1..).and_then(|fields| {
                Some(json!({"path": fields.get(7)?, "staged": &fields[0][..1], "unstaged": &fields[0][1..]}))
            }),
            '2' => {
                let fields: Vec<&str> = record.splitn(10, ' ').collect();
                let original = records.next().unwrap_or("");
                fields.get(9).map(|path| json!({"path": path, "from": original, "staged": &fields[1][..1], "unstaged": &fields[1][1..]}))
            }
            'u' => record.splitn(11, ' ').nth(10).map(|path| json!({"path": path, "conflict": true})),
            '?' => record.get(2..).map(|path| json!({"path": path, "untracked": true})),
            _ => None,
        };
        if let Some(entry) = entry {
            entries.push(entry);
        }
    }
    let truncated = entries.len() > 500;
    entries.truncate(500);
    json!({"branch": branch, "entries": entries, "truncated": truncated})
}

/// Parse `git log --format=%H%x1f%an%x1f%aI%x1f%s%x1e`.
pub fn parse_log(raw: &str) -> Vec<Value> {
    raw.split('\u{1e}')
        .map(str::trim)
        .filter(|record| !record.is_empty())
        .filter_map(|record| {
            let fields: Vec<&str> = record.split('\u{1f}').collect();
            (fields.len() == 4).then(|| json!({"commit": fields[0], "author": fields[1], "date": fields[2], "subject": fields[3]}))
        })
        .collect()
}

/// Parse `git for-each-ref --format=%(refname:short)%1f%(HEAD) refs/heads/`.
pub fn parse_branches(raw: &str) -> Value {
    let mut current = Value::Null;
    let mut branches = Vec::new();
    for line in raw.lines().filter(|line| !line.is_empty()) {
        let (name, head) = line.split_once('\u{1f}').unwrap_or((line, ""));
        if head.trim() == "*" {
            current = json!(name);
        }
        branches.push(json!(name));
    }
    json!({"current": current, "branches": branches})
}

fn commit_message(args: &Value) -> Result<String, String> {
    let message = args.get("message").and_then(Value::as_str).unwrap_or("").trim().to_string();
    if message.is_empty() || message.len() > 2000 {
        return Err("git_commit needs a 1-2000 character message.".into());
    }
    Ok(message)
}

fn commit_paths(args: &Value) -> Result<Vec<String>, String> {
    let Some(paths) = args.get("paths") else { return Ok(Vec::new()) };
    let paths = paths.as_array().ok_or("git_commit 'paths' must be a list of workspace paths.")?;
    if paths.len() > 100 {
        return Err("git_commit accepts at most 100 paths.".into());
    }
    paths
        .iter()
        .map(|path| {
            let path = path.as_str().ok_or("git_commit paths must be strings.")?;
            crate::workspace::valid_path(path, false)?;
            Ok(path.to_string())
        })
        .collect()
}

/// Execute one git tool. `workspace` is the configured workspace folder.
pub async fn execute(alias: &str, workspace: &str, args: &Value) -> Result<Value, String> {
    let root = repository_root(workspace).await?;
    let quick = Duration::from_secs(30);
    match alias {
        "git_status" => {
            let raw = run(workspace, &["status", "--porcelain=v2", "--branch", "-z", "--untracked-files=all"], &[], quick).await?;
            let mut status = parse_status(&raw);
            status["repositoryRoot"] = json!(root);
            Ok(status)
        }
        "git_diff" => {
            let staged = args.get("staged").and_then(Value::as_bool).unwrap_or(false);
            let base = rev_arg(args, "base")?;
            let path = path_arg(args)?;
            let mut command: Vec<&str> = vec!["diff", "--no-ext-diff", "--no-textconv", "--stat", "--patch"];
            if staged {
                command.push("--cached");
            }
            if let Some(base) = &base {
                command.push(base);
            }
            command.push("--");
            if let Some(path) = &path {
                command.push(path);
            }
            let (diff, truncated) = bounded(run(workspace, &command, &[], quick).await?);
            Ok(json!({"staged": staged, "base": base, "path": path, "diff": diff, "truncated": truncated, "empty": diff.trim().is_empty()}))
        }
        "git_log" => {
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(10).clamp(1, 50).to_string();
            let path = path_arg(args)?;
            let mut command = vec!["log", "-n", limit.as_str(), "--format=%H%x1f%an%x1f%aI%x1f%s%x1e", "--"];
            if let Some(path) = &path {
                command.push(path);
            }
            Ok(json!({"commits": parse_log(&run(workspace, &command, &[], quick).await?)}))
        }
        "git_branch" => {
            let raw = run(workspace, &["for-each-ref", "--format=%(refname:short)%1f%(HEAD)", "refs/heads/"], &[], quick).await?;
            Ok(parse_branches(&raw))
        }
        "git_commit" => {
            let message = commit_message(args)?;
            let paths = commit_paths(args)?;
            let slow = Duration::from_secs(120);
            if !paths.is_empty() {
                let mut add = vec!["add", "--"];
                add.extend(paths.iter().map(String::as_str));
                run(workspace, &add, &[], slow).await?;
            }
            let staged = run(workspace, &["diff", "--cached", "--name-only", "-z"], &[], quick).await?;
            if staged.trim_matches('\0').is_empty() {
                return Err("Nothing is staged to commit. Pass the changed paths in 'paths'.".into());
            }
            // Only what is staged is committed: never `-a`, `--amend` or a push.
            // The repository's own hooks and identity apply as for any commit.
            run(workspace, &["commit", "--quiet", "-m", &message], &[], slow).await?;
            let head = run(workspace, &["rev-parse", "HEAD"], &[], quick).await?.trim().to_string();
            let (stat, _) = bounded(run(workspace, &["show", "--stat", "--format=%s", "HEAD"], &[], quick).await?);
            Ok(json!({"commit": head, "summary": stat}))
        }
        _ => Err(format!("Unknown git tool '{alias}'.")),
    }
}

/// Harness registry entries for the git tools.
pub fn registry() -> Vec<(String, String, Value)> {
    let schema = |properties: Value, required: &[&str]| json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false});
    vec![
        ("git_status".into(), "Workspace repository status: branch, upstream ahead/behind, and staged, unstaged, untracked and conflicted paths (read-only).".into(),
            schema(json!({}), &[])),
        ("git_diff".into(), "Unified diff with a --stat summary (read-only). Unstaged by default; staged=true for the index; base compares the working tree with a branch, tag or commit; path limits to one workspace path. Output is capped at 64 KiB.".into(),
            schema(json!({"staged": {"type": "boolean"}, "base": {"type": "string"}, "path": {"type": "string"}}), &[])),
        ("git_log".into(), "Recent commits (hash, author, ISO date, subject), newest first (read-only).".into(),
            schema(json!({"limit": {"type": "integer", "minimum": 1, "maximum": 50}, "path": {"type": "string"}}), &[])),
        ("git_branch".into(), "Local branches and the current branch (read-only).".into(),
            schema(json!({}), &[])),
        ("git_commit".into(), "Create a commit in the workspace repository. Always asks the user. Stages the given workspace paths first; without paths, commits what is already staged. Never amends, pushes or uses -a.".into(),
            schema(json!({"message": {"type": "string"}, "paths": {"type": "array", "maxItems": 100, "items": {"type": "string"}}}), &["message"])),
    ]
}

pub const READ_ONLY: [&str; 4] = ["git_status", "git_diff", "git_log", "git_branch"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_porcelain_v2_with_renames_and_untracked_paths() {
        // Captured from git 2.49 (`status --porcelain=v2 --branch -z`).
        let raw = "# branch.oid 0e4cadb3524fee0989b560fee48db35a5a8ba9d9\0# branch.head master\0# branch.upstream origin/master\0# branch.ab +2 -1\0\
2 RM N... 100644 100644 100644 567609b1234a9b8806c5a05da6c866e480aa148d 567609b1234a9b8806c5a05da6c866e480aa148d R100 .gi2\0.gitignore\0\
1 A. N... 000000 100644 100644 0000000000000000000000000000000000000000 78981922613b2afb6025042ff6bd878ac1994e85 src/a b.txt\0\
? b [x].txt\0? nested/\0";
        let status = parse_status(raw);
        assert_eq!(status["branch"]["head"], "master");
        assert_eq!(status["branch"]["upstream"], "origin/master");
        assert_eq!(status["branch"]["ahead"], 2);
        assert_eq!(status["branch"]["behind"], 1);
        let entries = status["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0], json!({"path": ".gi2", "from": ".gitignore", "staged": "R", "unstaged": "M"}));
        assert_eq!(entries[1], json!({"path": "src/a b.txt", "staged": "A", "unstaged": "."}));
        assert_eq!(entries[2], json!({"path": "b [x].txt", "untracked": true}));
        assert_eq!(status["truncated"], false);
    }

    #[test]
    fn parses_log_and_branch_records() {
        let log = parse_log("abc\u{1f}Ada\u{1f}2026-09-21T10:00:00+05:30\u{1f}Fix add\u{1e}\ndef\u{1f}Bo\u{1f}2026-09-20T09:00:00+05:30\u{1f}Init\u{1e}\n");
        assert_eq!(log.len(), 2);
        assert_eq!(log[0]["subject"], "Fix add");
        let branches = parse_branches("feature\u{1f} \nmaster\u{1f}*\n");
        assert_eq!(branches["current"], "master");
        assert_eq!(branches["branches"], json!(["feature", "master"]));
    }

    #[test]
    fn arguments_cannot_become_options_or_escape_the_workspace() {
        assert!(rev_arg(&json!({"base": "--output=x"}), "base").is_err());
        assert!(rev_arg(&json!({"base": "main..evil"}), "base").is_err());
        assert_eq!(rev_arg(&json!({"base": "origin/main~2"}), "base").unwrap().as_deref(), Some("origin/main~2"));
        assert!(path_arg(&json!({"path": "../secret"})).is_err());
        assert!(path_arg(&json!({"path": "C:/x"})).is_err());
        assert!(commit_paths(&json!({"paths": ["src/a.rs", "../b"]})).is_err());
        assert!(commit_message(&json!({"message": "  "})).is_err());
        assert_eq!(commit_message(&json!({"message": "-n looks like a flag"})).unwrap(), "-n looks like a flag");
    }

    #[tokio::test]
    async fn read_tools_and_commit_work_on_a_real_repository() {
        if run(".", &["--version"], &[], Duration::from_secs(10)).await.is_err() {
            return; // Git is optional on build machines; the parsers are covered above.
        }
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_string_lossy().to_string();
        let identity = [("GIT_AUTHOR_NAME", "t"), ("GIT_AUTHOR_EMAIL", "t@x"), ("GIT_COMMITTER_NAME", "t"), ("GIT_COMMITTER_EMAIL", "t@x")];
        run(&ws, &["init", "-q"], &[], Duration::from_secs(10)).await.unwrap();
        run(&ws, &["config", "user.name", "t"], &identity, Duration::from_secs(10)).await.unwrap();
        run(&ws, &["config", "user.email", "t@x"], &identity, Duration::from_secs(10)).await.unwrap();
        run(&ws, &["config", "commit.gpgsign", "false"], &[], Duration::from_secs(10)).await.unwrap();
        std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
        let status = execute("git_status", &ws, &json!({})).await.unwrap();
        assert_eq!(status["entries"][0]["path"], "a.txt");
        assert!(execute("git_commit", &ws, &json!({"message": "first"})).await.unwrap_err().contains("Nothing is staged"));
        let commit = execute("git_commit", &ws, &json!({"message": "first", "paths": ["a.txt"]})).await.unwrap();
        assert_eq!(commit["commit"].as_str().unwrap().len(), 40);
        std::fs::write(dir.path().join("a.txt"), "two\n").unwrap();
        let diff = execute("git_diff", &ws, &json!({})).await.unwrap();
        assert!(diff["diff"].as_str().unwrap().contains("+two"));
        let log = execute("git_log", &ws, &json!({"limit": 5})).await.unwrap();
        assert_eq!(log["commits"][0]["subject"], "first");
    }
}
