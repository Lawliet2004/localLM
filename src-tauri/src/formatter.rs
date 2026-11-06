//! Project-aware formatting: detection, a bounded runner, and the
//! model-facing `format_paths` tool.
//!
//! An agent that writes code and leaves `cargo fmt --check` failing is a
//! defect, not a style choice. This module finds the formatter a project
//! already uses, runs it in place, and reports exactly what it did — the job
//! Claude Code does with a `PostToolUse` hook and OpenCode does with its
//! built-in formatter table.
//!
//! Detection needs no configuration: the file's final extension picks the
//! candidate formatter, the project decides whether that formatter is the one
//! in use, and an installed binary decides whether it can run at all.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use crate::execution::{capture_command, CaptureError};

/// Files larger than this are left alone. Reformatting a huge generated file
/// costs more than it is worth and can stall a tool call.
pub const MAX_FORMAT_BYTES: u64 = 8 * 1024 * 1024;

/// Captured formatter output is bounded so a chatty formatter cannot flood a
/// tool result.
const MAX_CAPTURE_BYTES: usize = 64 * 1024;

/// The timeout used by the automatic post-write path. A slow formatter must
/// never stall a tool call.
pub const AUTO_TIMEOUT_SECONDS: u64 = 10;

/// The timeout used by the explicit `format_paths` tool.
pub const TOOL_TIMEOUT_SECONDS: u64 = 30;

/// A formatter that can be applied to a file.
pub struct Formatter {
    pub name: &'static str,
    pub extensions: &'static [&'static str],
    pub command: &'static str,
    pub args: &'static [&'static str],
}

/// Every supported formatter, in precedence order.
///
/// Precedence matters whenever two formatters claim the same extension:
///
/// * `ruff` wins over `black` for `.py`/`.pyi`, so a project is never
///   formatted by two tools at once. `ruff format` is the formatter and
///   `black` is only the fallback for projects without ruff installed.
/// * `prettier` wins over `biome` for the extensions they share, because a
///   project that declares a prettier dependency asks for prettier.
/// * `gofmt` wins over `gofumpt` for `.go`.
///
/// `ruff-check-fix` is a lint-fix pass rather than a formatter: it shares
/// ruff's extension list, so `detect` never selects it (`ruff format`
/// already claims `.py`). It stays in the table so the runner and the
/// discovery surface know the project supports it.
static FORMATTERS: &[Formatter] = &[
    Formatter {
        name: "rustfmt",
        extensions: &["rs"],
        command: "rustfmt",
        args: &["--edition", "2021"],
    },
    Formatter {
        name: "prettier",
        extensions: &[
            "js", "jsx", "ts", "tsx", "mjs", "cjs", "json", "css", "scss", "less", "html", "vue",
            "yaml", "yml", "md",
        ],
        command: "prettier",
        args: &["--write"],
    },
    Formatter {
        name: "biome",
        extensions: &["js", "jsx", "ts", "tsx", "json", "jsonc", "css", "graphql"],
        command: "biome",
        args: &["format", "--write"],
    },
    Formatter {
        name: "gofmt",
        extensions: &["go"],
        command: "gofmt",
        args: &["-w"],
    },
    Formatter {
        name: "clang-format",
        extensions: &["c", "cc", "cpp", "cxx", "h", "hh", "hpp"],
        command: "clang-format",
        args: &["-i"],
    },
    Formatter {
        name: "ruff",
        extensions: &["py", "pyi"],
        command: "ruff",
        args: &["format"],
    },
    Formatter {
        name: "black",
        extensions: &["py", "pyi"],
        command: "black",
        args: &[],
    },
    Formatter {
        name: "gofumpt",
        extensions: &["go"],
        command: "gofumpt",
        args: &["-w"],
    },
    Formatter {
        name: "shfmt",
        extensions: &["sh", "bash"],
        command: "shfmt",
        args: &["-w"],
    },
    Formatter {
        name: "ruff-check-fix",
        extensions: &["py", "pyi"],
        command: "ruff",
        args: &["check", "--fix"],
    },
];

/// Every supported formatter, in precedence order.
pub fn formatters() -> &'static [Formatter] {
    FORMATTERS
}

/// The formatter that claims a file's extension, before the project and
/// availability checks.
///
/// This is the extension half of detection on its own: `.rs` belongs to
/// rustfmt, `.ts` to prettier, `.py` to ruff (ahead of black), and `.txt` to
/// nobody.
pub fn candidate(path: &Path) -> Option<&'static Formatter> {
    let extension = path.extension().and_then(|value| value.to_str())?;
    formatters()
        .iter()
        .find(|formatter| formatter.extensions.contains(&extension))
}

/// Whether the project around `path` uses this formatter.
///
/// Only the formatters that need project context answer here: prettier needs
/// a dependency in the nearest `package.json`, biome needs a configuration
/// file. Every other formatter is detected by its presence on PATH, which
/// [`detect`] checks separately.
pub fn project_detection(formatter: &Formatter, path: &Path, root: &Path) -> bool {
    match formatter.name {
        "prettier" => nearest_file(path, root, &["package.json"])
            .map(|package| package_json_has_prettier(&package))
            .unwrap_or(false),
        "biome" => nearest_file(path, root, &["biome.json", "biome.jsonc"]).is_some(),
        _ => true,
    }
}

/// The formatter that applies to `path`, if any.
///
/// A formatter applies when the file's final extension (case-sensitive) is in
/// its extension list, the project uses it, and its binary is installed.
pub fn detect(path: &Path, root: &Path) -> Option<&'static Formatter> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let formatter = candidate(&absolute)?;
    if !project_detection(formatter, &absolute, root) {
        return None;
    }
    if !program_available(formatter.command) {
        return None;
    }
    Some(formatter)
}

/// Whether a program is resolvable on PATH.
pub fn program_available(program: &str) -> bool {
    if program.is_empty() {
        return false;
    }
    if program.contains('/') || program.contains('\\') {
        return is_executable(Path::new(program));
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    for directory in std::env::split_paths(&path) {
        if directory.as_os_str().is_empty() {
            continue;
        }
        if is_executable(&directory.join(program)) {
            return true;
        }
        #[cfg(windows)]
        {
            let Some(extensions) = std::env::var_os("PATHEXT") else {
                continue;
            };
            for extension in std::env::split_paths(&extensions) {
                if extension.as_os_str().is_empty() {
                    continue;
                }
                let mut name = program.to_string();
                name.push_str(&extension.to_string_lossy());
                if is_executable(&directory.join(name)) {
                    return true;
                }
            }
        }
    }
    false
}

/// The outcome of formatting one file.
#[derive(Clone, Debug)]
pub struct FormatOutcome {
    pub path: String,
    pub formatter: Option<String>,
    pub changed: bool,
    pub detail: String,
}

/// A formatting outcome plus whether a formatter actually ran.
#[derive(Clone, Debug)]
pub struct DetailedOutcome {
    pub outcome: FormatOutcome,
    /// Whether a formatter actually ran. False when the file was skipped.
    pub applied: bool,
}

/// Why a formatting attempt did not produce a formatted file.
#[derive(Clone, Debug)]
pub enum FormatFailure {
    /// The target path is unusable: outside the workspace, reached through a
    /// symbolic link, or not a regular file. The whole call fails.
    InvalidTarget(String),
    /// The file is not there. Reported per file as skipped.
    MissingFile(String),
    /// The formatter ran and failed. Reported per file as failed.
    FormatterFailed { formatter: String, detail: String },
}

impl FormatFailure {
    /// The message shown to the caller.
    pub fn message(&self) -> String {
        match self {
            Self::InvalidTarget(message) => message.clone(),
            Self::MissingFile(message) => message.clone(),
            Self::FormatterFailed { detail, .. } => detail.clone(),
        }
    }
}

/// Format one workspace-relative file, reporting whether a formatter ran.
async fn format_file_async(
    root: &std::path::Path,
    relative: &str,
    timeout_seconds: u64,
) -> Result<Option<DetailedOutcome>, FormatFailure> {
    let (root, target) = resolve_target(root, relative)?;
    let Some(formatter) = detect(&target, &root) else {
        return Ok(None);
    };
    let metadata = std::fs::metadata(&target)
        .map_err(|error| FormatFailure::InvalidTarget(format!("{relative} cannot be inspected: {error}")))?;
    if metadata.len() > MAX_FORMAT_BYTES {
        return Ok(Some(DetailedOutcome {
            outcome: FormatOutcome {
                path: relative.to_string(),
                formatter: Some(formatter.name.to_string()),
                changed: false,
                detail: format!(
                    "Skipped: the file is larger than the {MAX_FORMAT_BYTES} byte formatting limit."
                ),
            },
            applied: false,
        }));
    }
    let before = file_sha256(&root, relative)?;
    let outcome = run_formatter(
        formatter,
        &root,
        &target,
        Duration::from_secs(timeout_seconds.max(1)),
    ).await;
    match outcome {
        RunOutcome::NotInstalled => Ok(Some(DetailedOutcome {
            outcome: FormatOutcome {
                path: relative.to_string(),
                formatter: Some(formatter.name.to_string()),
                changed: false,
                detail: format!(
                    "Skipped: {} is not installed, so the file was left unchanged.",
                    formatter.command
                ),
            },
            applied: false,
        })),
        RunOutcome::TimedOut => Err(FormatFailure::FormatterFailed {
            formatter: formatter.name.to_string(),
            detail: format!(
                "{} did not finish within {} seconds; the file may be partly formatted.",
                formatter.command, timeout_seconds
            ),
        }),
        RunOutcome::Failed { detail } => Err(FormatFailure::FormatterFailed {
            formatter: formatter.name.to_string(),
            detail: format!("{detail} The file may already have changed."),
        }),
        RunOutcome::Finished { detail } => {
            let after = file_sha256(&root, relative).map_err(|error| FormatFailure::FormatterFailed {
                formatter: formatter.name.to_string(),
                detail: format!("{} finished, but its file changes could not be verified: {}", formatter.name, error.message()),
            })?;
            let changed = before != after;
            Ok(Some(DetailedOutcome {
                outcome: FormatOutcome {
                    path: relative.to_string(),
                    formatter: Some(formatter.name.to_string()),
                    changed,
                    detail: if changed {
                        format!("{} rewrote the file. {detail}", formatter.name)
                            .trim_end()
                            .to_string()
                    } else {
                        format!("{} made no changes. {detail}", formatter.name)
                            .trim_end()
                            .to_string()
                    },
                },
                applied: true,
            }))
        }
    }
}

/// Synchronous entry point for workspace writes, which already run on a
/// blocking worker and hold the workspace transaction guard.
pub fn format_file_detailed(
    root: &Path,
    relative: &str,
    timeout_seconds: u64,
    cancel: Option<&tokio::sync::watch::Receiver<()>>,
) -> Result<Option<DetailedOutcome>, FormatFailure> {
    let run = || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()
            .map_err(|error| FormatFailure::InvalidTarget(format!("Formatter runtime unavailable: {error}")))?;
        runtime.block_on(async {
            if let Some(cancel) = cancel {
                let mut cancel = cancel.clone();
                tokio::select! {
                    biased;
                    _ = cancel.changed() => Err(FormatFailure::InvalidTarget(
                        "Formatting interrupted; the file may already have changed.".into()
                    )),
                    outcome = format_file_async(root, relative, timeout_seconds) => outcome,
                }
            } else {
                format_file_async(root, relative, timeout_seconds).await
            }
        })
    };
    if tokio::runtime::Handle::try_current().is_ok() {
        std::thread::scope(|scope| scope.spawn(run).join())
            .map_err(|_| FormatFailure::InvalidTarget("Formatter worker stopped unexpectedly.".into()))?
    } else {
        run()
    }
}

/// Resolve a workspace-relative path to the file a formatter may rewrite.
///
/// The path must be relative, must not traverse out of the workspace, and must
/// not reach the file through a symbolic link: a formatter rewrites the file
/// in place, so a link could redirect the write outside the workspace.
fn resolve_target(root: &Path, relative: &str) -> Result<(PathBuf, PathBuf), FormatFailure> {
    crate::workspace::valid_path(relative, false).map_err(FormatFailure::InvalidTarget)?;
    let root = std::fs::canonicalize(root).map_err(|error| {
        FormatFailure::InvalidTarget(format!("The workspace is unavailable: {error}"))
    })?;
    let target = root.join(relative);
    if !target.starts_with(&root) {
        return Err(FormatFailure::InvalidTarget(
            "The path leaves the workspace.".into(),
        ));
    }
    // Every parent directory must be a plain directory inside the workspace.
    let mut current = root.clone();
    let parts: Vec<&str> = relative.split('/').collect();
    for part in &parts[..parts.len().saturating_sub(1)] {
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Err(FormatFailure::InvalidTarget(
                    "Refusing to format through a symbolic link.".into(),
                ))
            }
            Err(error) => {
                return Err(FormatFailure::MissingFile(format!(
                    "{relative} does not exist: {error}"
                )))
            }
        }
    }
    match std::fs::symlink_metadata(&target) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(FormatFailure::InvalidTarget(
            "Refusing to format a symbolic link.".into(),
        )),
        Ok(metadata) if !metadata.is_file() => Err(FormatFailure::InvalidTarget(
            "Only regular workspace files can be formatted.".into(),
        )),
        Ok(_) => Ok((root, target)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(
            FormatFailure::MissingFile(format!("{relative} does not exist.")),
        ),
        Err(error) => Err(FormatFailure::InvalidTarget(format!(
            "{relative} cannot be inspected: {error}"
        ))),
    }
}

/// The SHA-256 of a workspace file, read through the same no-follow path the
/// read tool uses.
fn file_sha256(root: &Path, relative: &str) -> Result<String, FormatFailure> {
    let directory = cap_std::fs::Dir::open_ambient_dir(root, cap_std::ambient_authority())
        .map_err(|error| {
            FormatFailure::InvalidTarget(format!("The workspace is unavailable: {error}"))
        })?;
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true);
    let file = crate::workspace::open_file_no_links(&directory, relative, &mut options)
        .map_err(|error| {
            FormatFailure::InvalidTarget(format!("{relative} cannot be read: {error}"))
        })?;
    let mut bytes = Vec::new();
    file.take(MAX_FORMAT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            FormatFailure::InvalidTarget(format!("{relative} cannot be read: {error}"))
        })?;
    if bytes.len() as u64 > MAX_FORMAT_BYTES {
        return Err(FormatFailure::InvalidTarget(format!(
            "{relative} grew beyond the formatting limit."
        )));
    }
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

/// What a formatter run produced.
enum RunOutcome {
    /// The formatter ran and exited successfully.
    Finished { detail: String },
    /// The formatter could not be started because it is not installed.
    NotInstalled,
    /// The formatter ran and failed.
    Failed { detail: String },
    /// The formatter did not finish in time.
    TimedOut,
}

/// Run one formatter over one file, with a hard timeout.
async fn run_formatter(formatter: &Formatter, root: &Path, target: &Path, timeout: Duration) -> RunOutcome {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut command = tokio::process::Command::new(formatter.command);
    command.args(formatter.args).arg(target).current_dir(root)
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let (status, stdout, stderr) = match capture_command(command, None, deadline, MAX_CAPTURE_BYTES).await {
        Ok(output) => output,
        Err(CaptureError::Spawn(error)) if error.kind() == std::io::ErrorKind::NotFound => {
            return RunOutcome::NotInstalled;
        }
        Err(CaptureError::Spawn(error)) => return RunOutcome::Failed {
            detail: format!("The formatter could not be started: {error}"),
        },
        Err(CaptureError::Io(error)) => return RunOutcome::Failed {
            detail: format!("Could not collect the formatter result: {error}"),
        },
        Err(CaptureError::Timeout) => return RunOutcome::TimedOut,
    };
    if status.success() {
        let detail = if stderr.trim().is_empty() {
            stdout.trim().to_string()
        } else {
            stderr.trim().to_string()
        };
        RunOutcome::Finished { detail }
    } else {
        let detail = if stderr.trim().is_empty() {
            format!(
                "{} exited with status {}: {}",
                formatter.command,
                status.code().unwrap_or(-1),
                stdout.trim()
            )
        } else {
            format!(
                "{} exited with status {}: {}",
                formatter.command,
                status.code().unwrap_or(-1),
                stderr.trim()
            )
        };
        RunOutcome::Failed { detail }
    }
}

/// Whether a path is a file that can be executed.
fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// The nearest file with one of `names`, searching from the file's directory
/// up to the workspace root (inclusive).
fn nearest_file(path: &Path, root: &Path, names: &[&str]) -> Option<PathBuf> {
    let directory = path.parent().unwrap_or(root);
    for candidate in directory_chain(directory, root) {
        for name in names {
            let file = candidate.join(name);
            if file.is_file() {
                return Some(file);
            }
        }
    }
    None
}

/// The directories from `directory` up to `root`, nearest first.
fn directory_chain(directory: &Path, root: &Path) -> Vec<PathBuf> {
    let Ok(relative) = directory.strip_prefix(root) else {
        return vec![directory.to_path_buf()];
    };
    let mut current = root.to_path_buf();
    let mut directories = vec![current.clone()];
    for component in relative.components() {
        current = current.join(component);
        directories.push(current.clone());
    }
    directories.reverse();
    directories
}

/// Whether a `package.json` declares a prettier dependency.
fn package_json_has_prettier(path: &Path) -> bool {
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    if bytes.len() > 1024 * 1024 {
        return false;
    }
    let Ok(package) = serde_json::from_slice::<Value>(&bytes) else {
        return false;
    };
    ["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"]
        .iter()
        .any(|section| package[section].get("prettier").is_some())
}

/// Model-facing tool definitions.
pub fn registry() -> Vec<crate::tool_spec::ToolSpec> {
    let input = json!({
        "type":"object",
        "properties":{
            "paths":{
                "type":"array",
                "minItems":1,
                "maxItems":64,
                "items":{"type":"string","minLength":1,"maxLength":4096,
                    "description":"Workspace-relative path of a file to format."}
            }
        },
        "required":["paths"],
        "additionalProperties":false
    });
    let output = json!({
        "type":"object",
        "required":["schemaVersion","status","data","error","metadata"],
        "properties":{
            "schemaVersion":{"const":crate::tool_envelope::SCHEMA_VERSION},
            "status":{"enum":["success","partial","failure"]},
            "error":{"type":["object","null"]},
            "metadata":{"type":"object","required":["sideEffects"]},
            "data":{
                "type":"object","required":["formatted","skipped","failed"],
                "properties":{
                    "formatted":{"type":"array","items":{"$ref":"#/$defs/outcome"}},
                    "skipped":{"type":"array","items":{"$ref":"#/$defs/outcome"}},
                    "failed":{"type":"array","items":{"$ref":"#/$defs/outcome"}}
                },
                "additionalProperties":false
            }
        },
        "$defs":{
            "outcome":{
                "type":"object",
                "required":["path","formatter","changed","detail"],
                "properties":{
                    "path":{"type":"string"},
                    "formatter":{"type":["string","null"]},
                    "changed":{"type":"boolean"},
                    "detail":{"type":"string"}
                }
            }
        }
    });
    vec![crate::tool_spec::ToolSpec::new(
        "format_paths",
        "Format workspace files in place with the formatter the project already uses \
         (rustfmt, prettier, biome, gofmt, clang-format, ruff, black, gofumpt, shfmt). \
         Use it after writing or editing code so the result passes the project's own \
         format check. Files are rewritten only when the formatter changes them; files \
         with no applicable formatter are reported as skipped and left untouched. \
         Paths are workspace-relative and files outside the workspace are never touched. \
         Returns one outcome per path: which formatter ran, whether the file changed, \
         and what happened.",
        input,
    )
    .with_output(output)]
}

/// Execute the format_paths tool.
pub async fn execute(
    ctx: &crate::harness::HarnessCtx<'_>,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let schema = registry()
        .into_iter()
        .find(|spec| spec.name == "format_paths")
        .map(|spec| spec.input)
        .ok_or("The formatter tool is not registered.")?;
    crate::tool_validation::validate(&schema, &args)?;
    let paths = args["paths"]
        .as_array()
        .ok_or("format_paths needs a 'paths' array.")?;
    let root = PathBuf::from(ctx.snapshot.workspace_path.clone());
    // Every path is resolved before anything is formatted: a path outside the
    // workspace fails the whole call rather than half-formatting a batch.
    let mut targets = Vec::with_capacity(paths.len());
    for path in paths {
        let path = path
            .as_str()
            .ok_or("Every path must be a string.")?;
        if path.is_empty() || path.len() > 4096 {
            return Err("Every path must be 1–4096 bytes.".into());
        }
        match resolve_target(&root, path) {
            Ok(_) => targets.push(path.to_string()),
            // A path that leaves the workspace, reaches through a link, or is
            // not a regular file is a hard failure of the whole call.
            Err(FormatFailure::InvalidTarget(message)) => return Err(message),
            // A missing file is reported per file as skipped.
            Err(_) => targets.push(path.to_string()),
        }
    }
    let mut formatted = Vec::new();
    let mut skipped = Vec::new();
    let mut failed = Vec::new();
    for path in targets {
        let outcome = format_file_async(&root, &path, TOOL_TIMEOUT_SECONDS).await;
        match outcome {
            Ok(None) => skipped.push(outcome_json(&path, None, false, "No formatter applies to this file.")),
            Ok(Some(detailed)) if detailed.applied => {
                formatted.push(outcome_json(
                    &path,
                    detailed.outcome.formatter.as_deref(),
                    detailed.outcome.changed,
                    &detailed.outcome.detail,
                ));
            }
            Ok(Some(detailed)) => skipped.push(outcome_json(
                &path,
                detailed.outcome.formatter.as_deref(),
                false,
                &detailed.outcome.detail,
            )),
            Err(FormatFailure::InvalidTarget(message)) => {
                failed.push(outcome_json(&path, None, false, &message));
            }
            Err(FormatFailure::MissingFile(message)) => {
                skipped.push(outcome_json(&path, None, false, &message))
            }
            Err(FormatFailure::FormatterFailed { formatter, detail }) => failed.push(outcome_json(
                &path,
                Some(formatter.as_str()),
                false,
                &detail,
            )),
        }
    }
    Ok(format_result(formatted, skipped, failed))
}

fn format_result(formatted: Vec<Value>, skipped: Vec<Value>, failed: Vec<Value>) -> Value {
    use crate::tool_envelope::{error_object, make, merge_compat, Status, SIDE_APPLIED, SIDE_NONE, SIDE_UNKNOWN};
    let (status, side_effects) = if failed.is_empty() {
        (Status::Success, if formatted.iter().any(|file| file["changed"] == true) { SIDE_APPLIED } else { SIDE_NONE })
    } else if formatted.is_empty() {
        (Status::Failure, SIDE_UNKNOWN)
    } else {
        (Status::Partial, SIDE_UNKNOWN)
    };
    let message = "Some paths could not be formatted. Inspect the per-file outcomes before retrying.";
    let error = (status == Status::Failure).then(|| error_object("format-failed", message, false, side_effects));
    let data = json!({"formatted":formatted,"skipped":skipped,"failed":failed});
    let mut result = make(status, data.clone(), Vec::new(), error, json!({"sideEffects":side_effects}));
    merge_compat(&mut result, &data);
    if status == Status::Failure {
        result["isError"] = json!(true);
    }
    result
}

/// One per-file outcome in the tool result.
fn outcome_json(path: &str, formatter: Option<&str>, changed: bool, detail: &str) -> Value {
    json!({
        "path": path,
        "formatter": formatter,
        "changed": changed,
        "detail": detail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn formatter(name: &str) -> &'static Formatter {
        formatters()
            .iter()
            .find(|formatter| formatter.name == name)
            .unwrap_or_else(|| panic!("{name} is not in the formatter table"))
    }

    #[test]
    fn the_formatter_table_is_declared_with_commands() {
        let table = formatters();
        assert!(!table.is_empty());
        for formatter in table {
            assert!(!formatter.command.is_empty(), "{}", formatter.name);
            assert!(!formatter.name.is_empty());
            assert!(!formatter.extensions.is_empty(), "{}", formatter.name);
        }
        // The documented table is complete.
        for name in [
            "rustfmt",
            "prettier",
            "biome",
            "gofmt",
            "clang-format",
            "ruff",
            "black",
            "gofumpt",
            "shfmt",
            "ruff-check-fix",
        ] {
            assert!(table.iter().any(|formatter| formatter.name == name), "{name}");
        }
    }

    #[test]
    fn extensions_pick_the_expected_formatter() {
        assert_eq!(candidate(Path::new("main.rs")).map(|f| f.name), Some("rustfmt"));
        assert_eq!(candidate(Path::new("app.ts")).map(|f| f.name), Some("prettier"));
        assert_eq!(candidate(Path::new("app.tsx")).map(|f| f.name), Some("prettier"));
        // ruff wins over black for python files.
        assert_eq!(candidate(Path::new("a.py")).map(|f| f.name), Some("ruff"));
        assert_eq!(candidate(Path::new("a.pyi")).map(|f| f.name), Some("ruff"));
        assert_eq!(candidate(Path::new("a.go")).map(|f| f.name), Some("gofmt"));
        assert_eq!(candidate(Path::new("a.sh")).map(|f| f.name), Some("shfmt"));
        assert_eq!(candidate(Path::new("a.c")).map(|f| f.name), Some("clang-format"));
        // Nothing claims a plain text file, and the match is case-sensitive.
        assert!(candidate(Path::new("notes.txt")).is_none());
        assert!(candidate(Path::new("main.RS")).is_none());
        assert!(candidate(Path::new("Makefile")).is_none());
        // The final extension decides.
        assert_eq!(
            candidate(Path::new("vendor.min.js")).map(|f| f.name),
            Some("prettier")
        );
    }

    #[test]
    fn detect_never_selects_a_formatter_outside_the_extension_list() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("main.rs");
        std::fs::write(&source, "fn main() {}\n").unwrap();
        let script = root.path().join("app.ts");
        std::fs::write(&script, "const a = 1;\n").unwrap();
        if let Some(selected) = detect(&source, root.path()) {
            assert_eq!(selected.name, "rustfmt");
            assert!(selected.extensions.contains(&"rs"));
        }
        if let Some(selected) = detect(&script, root.path()) {
            assert_eq!(selected.name, "prettier");
            assert!(selected.extensions.contains(&"ts"));
        }
    }

    #[test]
    fn detection_walks_up_to_the_nearest_package_json() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("pkg");
        std::fs::create_dir_all(package.join("sub")).unwrap();
        let manifest = package.join("package.json");
        std::fs::write(&manifest, r#"{"devDependencies":{"prettier":"^3.0.0"}}"#).unwrap();
        let file = package.join("sub/file.ts");
        std::fs::write(&file, "const a = 1;\n").unwrap();
        let prettier = formatter("prettier");
        // The nearest package.json is found by walking up from the file.
        assert!(project_detection(prettier, &file, root.path()));
        // Without it the project does not use prettier.
        std::fs::remove_file(&manifest).unwrap();
        assert!(!project_detection(prettier, &file, root.path()));
        // A package.json without a prettier dependency does not count either.
        std::fs::write(&manifest, r#"{"dependencies":{"typescript":"^5.0.0"}}"#).unwrap();
        assert!(!project_detection(prettier, &file, root.path()));
        std::fs::write(&manifest, r#"{"devDependencies":{"prettier":"^3.0.0"}}"#).unwrap();
        // detect() only selects a formatter whose binary is installed.
        assert_eq!(
            detect(&file, root.path()).map(|found| found.name),
            program_available("prettier").then_some("prettier")
        );
    }

    #[test]
    fn biome_needs_a_configuration_file() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("src/app.ts");
        std::fs::create_dir_all(root.path().join("src")).unwrap();
        std::fs::write(&file, "const a = 1;\n").unwrap();
        let biome = formatter("biome");
        assert!(!project_detection(biome, &file, root.path()));
        std::fs::write(root.path().join("biome.json"), "{}\n").unwrap();
        assert!(project_detection(biome, &file, root.path()));
    }

    #[test]
    fn a_nonsense_program_is_not_available() {
        assert!(!program_available(""));
        assert!(!program_available(
            "locallm-definitely-not-a-real-program-9f2c"
        ));
    }

    #[test]
    fn a_file_without_a_formatter_is_not_an_error() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("notes.txt"), "hello\n").unwrap();
        let outcome = format_file_detailed(root.path(), "notes.txt", 10, None).unwrap();
        assert!(outcome.is_none());
    }

    #[test]
    fn a_path_outside_the_root_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.rs"), "fn main() {}\n").unwrap();
        for path in [
            "../escape.rs",
            "/etc/passwd",
            "sub/../../escape.rs",
        ] {
            assert!(
                format_file_detailed(root.path(), path, 10, None).is_err(),
                "{path} was not refused"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_out_of_the_root_is_refused() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("secret.rs");
        std::fs::write(&target, "fn main() {}\n").unwrap();
        symlink(&target, root.path().join("link.rs")).unwrap();
        let error = format_file_detailed(root.path(), "link.rs", 10, None).unwrap_err();
        let message = error.message();
        assert!(message.contains("link") || message.contains("symbolic"), "{message}");
        // The file outside the workspace is untouched.
        assert_eq!(
            std::fs::read_to_string(&target).unwrap(),
            "fn main() {}\n"
        );
    }

    #[test]
    fn the_tool_validates_its_arguments_and_its_result_contract() {        let spec = registry()
            .into_iter()
            .find(|spec| spec.name == "format_paths")
            .unwrap();
        assert!(crate::tool_validation::validate(&spec.input, &json!({"paths":["a.rs"]})).is_ok());
        assert!(crate::tool_validation::validate(&spec.input, &json!({"paths":[]})).is_err());
        assert!(crate::tool_validation::validate(&spec.input, &json!({})).is_err());
        assert!(crate::tool_validation::validate(&spec.input, &json!({"paths":"a.rs"})).is_err());
        assert!(crate::tool_validation::validate(&spec.input, &json!({"paths":[""],"x":1})).is_err());
        let result = format_result(
            vec![outcome_json("a.rs", Some("rustfmt"), true, "rewrote")],
            vec![outcome_json("b.txt", None, false, "no formatter")],
            vec![],
        );
        assert!(spec.validate_output(&result).is_ok());
        assert!(spec
            .validate_output(&json!({"formatted":[],"skipped":[]}))
            .is_err());
    }

    #[tokio::test]
    async fn the_format_paths_tool_reports_one_outcome_per_path() {
        let directory = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(directory.path());
        let workspace = directory.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::write(workspace.join("main.rs"), "fn  main( ) {}\n").unwrap();
        std::fs::write(workspace.join("notes.txt"), "hello\n").unwrap();
        let tools: Vec<crate::connectors::AgentTool> = Vec::new();
        let ctx = crate::harness::HarnessCtx {
            state: &state,
            snapshot: crate::test_support::snapshot(&state, &workspace, &["format_paths"]),
            conversation_id: "conversation".into(),
            run_id: "run".into(),
            step_id: "step-0".into(),
            access_mode: crate::permissions::AccessMode::FullAccess,
            depth: 0,
            preset_id: crate::presets::STANDARD.into(),
            inherit_tools: &tools,
            emit: std::sync::Arc::new(|_, _| {}),
        };
        let result = execute(&ctx, json!({"paths":["main.rs","notes.txt"]}))
            .await
            .unwrap();
        // Every path is accounted for exactly once.
        let reported = result["formatted"].as_array().unwrap().len()
            + result["skipped"].as_array().unwrap().len()
            + result["failed"].as_array().unwrap().len();
        assert_eq!(reported, 2);
        assert!(result["failed"].as_array().unwrap().is_empty());
        // The plain text file has no formatter and is never rewritten.
        let skipped = result["skipped"].as_array().unwrap();
        assert!(skipped
            .iter()
            .any(|entry| entry["path"] == "notes.txt" && entry["formatter"].is_null()));
        assert_eq!(
            std::fs::read_to_string(workspace.join("notes.txt")).unwrap(),
            "hello\n"
        );
        // The declared result contract holds for what the tool returned.
        let spec = registry()
            .into_iter()
            .find(|spec| spec.name == "format_paths")
            .unwrap();
        assert!(spec.validate_output(&result).is_ok());
        // A path outside the workspace fails the whole call.
        assert!(execute(&ctx, json!({"paths":["../escape.rs"]})).await.is_err());
        assert!(execute(&ctx, json!({"paths":[]})).await.is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn inherited_formatter_pipes_obey_the_deadline() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("fixture.rs");
        let formatter = Formatter { name: "fixture", extensions: &["rs"], command: "sh", args: &["-c", "(sleep 2) & exit 0"] };
        let start = std::time::Instant::now();
        let outcome = run_formatter(&formatter, root.path(), &target, Duration::from_millis(100)).await;
        assert!(matches!(outcome, RunOutcome::TimedOut));
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn failed_and_partial_formatter_outcomes_stay_typed_after_normalization() {
        let failed = outcome_json("a.rs", Some("rustfmt"), false, "formatter failed");
        let formatted = outcome_json("b.rs", Some("rustfmt"), true, "formatted");
        for (files, expected) in [(vec![], "failure"), (vec![formatted], "partial")] {
            let value = format_result(files, vec![], vec![failed.clone()]);
            let normalized = crate::tool_envelope::normalize(value, crate::tool_envelope::Family::Harness);
            assert_eq!(normalized["status"], expected);
            assert_eq!(normalized["metadata"]["sideEffects"], "unknown");
            assert_eq!(normalized["data"]["failed"][0]["path"], "a.rs");
        }
    }

}
