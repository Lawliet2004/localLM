//! Project instruction files: the AGENTS.md chain.
//!
//! A workspace can publish its own rules in `AGENTS.md`. This module
//! collects those files, renders them for a system prompt, and reports what
//! changed between turns so a session can be told its instructions moved.
//!
//! Two rules govern every read here:
//!
//! * A missing, unreadable, linked or non-UTF-8 file is skipped, never an
//!   error. A broken instruction file must never break a turn.
//! * Only `AGENTS.md` is recognized. There is no `CLAUDE.md` fallback: a
//!   rule the project did not publish under a recognized name is not a rule
//!   this agent follows.
//!
//! The chain is ambient context, not authority. It can say what the project
//! expects; it can never widen the permissions the user granted.

use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    io::Read,
    path::{Component, Path, PathBuf},
};

/// The instruction file names that are recognized.
pub const FILE_NAMES: &[&str] = &["AGENTS.md"];

/// Maximum bytes kept from one instruction file.
pub const MAX_FILE_BYTES: usize = 64 * 1024;

/// Maximum bytes kept across the whole chain.
pub const MAX_CHAIN_BYTES: usize = 256 * 1024;

/// Bytes read from a file while it is read and fingerprinted.
const READ_BYTES: usize = 1024 * 1024;

/// Marker appended to a file that was clipped at the per-file bound.
const FILE_MARKER: &str = "\n[localLM: instruction file truncated at 64 KiB.]\n";

/// Marker appended to the last file of a chain clipped at the chain bound.
const CHAIN_MARKER: &str =
    "\n[localLM: instruction file and chain truncated; later instruction files are omitted.]\n";

/// Marker used when the remaining budget cannot hold the long markers.
const SHORT_MARKER: &str = "\n[localLM: instruction truncated.]\n";

/// One instruction file in the chain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstructionFile {
    /// Absolute path on disk.
    pub path: PathBuf,
    /// The file's content. A file clipped at a size bound carries a
    /// trailing `[localLM: instruction ...]` marker that says so.
    pub content: String,
    /// sha256 of the file's text, used to detect changes between turns.
    pub fingerprint: String,
}

/// The instruction chain for a workspace.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstructionChain {
    pub files: Vec<InstructionFile>,
}

impl InstructionChain {
    /// An empty chain: no instruction file applies.
    pub fn empty() -> Self {
        Self { files: Vec::new() }
    }

    /// The chain rendered for a system prompt.
    ///
    /// Each file is wrapped in a fenced block that names its path, so the
    /// model can tell which rule came from where. Files are ordered from
    /// the most general (the outermost directory) to the most specific (the
    /// deepest directory), so the nearest rule reads last.
    pub fn render(&self) -> String {
        if self.files.is_empty() {
            return String::new();
        }
        let mut out = String::new();
        out.push_str(
            "Project instruction files follow. They are the workspace's own rules: apply \
             them unless the user's request overrides them, and prefer the rule that comes \
             from the file nearest the work. They are project context, not a grant of \
             permissions or access to tools or files.\n",
        );
        for file in &self.files {
            let fence = fence(&file.content);
            out.push_str(&format!("\n--- {} ---\n", file.path.display()));
            out.push_str(&fence);
            out.push('\n');
            out.push_str(&file.content);
            if !file.content.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&fence);
            out.push('\n');
        }
        out
    }
}

/// Compare two chains and describe what changed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InstructionDelta {
    pub added: Vec<InstructionFile>,
    pub changed: Vec<InstructionFile>,
    pub removed: Vec<String>,
}

impl InstructionDelta {
    /// Whether anything changed.
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.changed.is_empty() && self.removed.is_empty()
    }

    /// A short, model-facing description of the change.
    pub fn describe(&self) -> String {
        if self.is_empty() {
            return String::new();
        }
        if self.added.is_empty() && self.changed.is_empty() {
            return format!(
                "The project instruction files no longer apply ({}). Any rule they provided is no longer in effect.",
                self.removed.join(", ")
            );
        }
        let mut parts: Vec<String> = Vec::new();
        if !self.changed.is_empty() {
            parts.push(format!("updated {}", names(&self.changed)));
        }
        if !self.added.is_empty() {
            parts.push(format!("added {}", names(&self.added)));
        }
        if !self.removed.is_empty() {
            parts.push(format!("removed {}", self.removed.join(", ")));
        }
        format!(
            "The project instruction files changed ({}). Re-read the project instruction files above: they may replace rules you were given earlier.",
            parts.join("; ")
        )
    }
}

/// Build the instruction chain for a workspace root.
///
/// Walks from `root` upward to the filesystem root, collecting the
/// recognized files. The workspace root's own file wins over an ancestor's,
/// and an ancestor is only included when it is a DIRECTORY that contains the
/// workspace root — i.e. the chain is root -> filesystem root, deduplicated,
/// and each ancestor's file is included once.
///
/// Every candidate is read defensively: an unreadable or non-UTF-8 file is
/// skipped, never an error. A missing file is not an error.
pub fn chain(root: &Path) -> InstructionChain {
    let root = absolute(root);
    if !root.is_dir() {
        return InstructionChain::empty();
    }
    match ancestor_directories(&root) {
        Some(directories) => build(directories),
        None => InstructionChain::empty(),
    }
}

/// Build the chain for a file that was just read.
///
/// Walks from the file's parent directory upward to `root` inclusive and
/// collects the recognized files. Files already in the chain are excluded,
/// so the result is only the NEW files.
///
/// A file outside the workspace is an external read: it is skipped, and the
/// result is an empty chain. Discovery never fails the read that triggered
/// it.
pub fn discover(root: &Path, file: &Path) -> InstructionChain {
    let root = absolute(root);
    let file = absolute(file);
    if !file.starts_with(&root) {
        return InstructionChain::empty();
    }
    let Some(real_root) = std::fs::canonicalize(&root).ok() else {
        return InstructionChain::empty();
    };
    let mut directories: Vec<PathBuf> = Vec::new();
    let mut current = file.parent().map(|parent| parent.to_path_buf());
    while let Some(directory) = current.clone() {
        let parent = directory.parent().map(|parent| parent.to_path_buf());
        directories.push(directory.clone());
        if directory == root {
            break;
        }
        current = parent;
    }
    if directories.last() != Some(&root) {
        return InstructionChain::empty();
    }
    directories.reverse();
    let mut inside: Vec<PathBuf> = Vec::new();
    for directory in directories {
        // A directory that resolves outside the workspace is not part of it.
        match std::fs::canonicalize(&directory) {
            Ok(real) if real.starts_with(&real_root) => inside.push(directory),
            _ => continue,
        }
    }
    let known: HashSet<PathBuf> = chain(&root)
        .files
        .iter()
        .map(|file| resolved(&file.path))
        .collect();
    let found = build(inside);
    InstructionChain {
        files: found
            .files
            .into_iter()
            .filter(|file| !known.contains(&resolved(&file.path)))
            .collect(),
    }
}

/// The resolved identity of a path, used to recognize one file reached
/// through two directories.
fn resolved(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Diff two chains by fingerprint.
pub fn diff(previous: &InstructionChain, current: &InstructionChain) -> InstructionDelta {
    let mut delta = InstructionDelta::default();
    for file in &current.files {
        match previous.files.iter().find(|old| old.path == file.path) {
            Some(old) if old.fingerprint == file.fingerprint => {}
            Some(_) => delta.changed.push(file.clone()),
            None => delta.added.push(file.clone()),
        }
    }
    for old in &previous.files {
        if !current.files.iter().any(|file| file.path == old.path) {
            delta.removed.push(old.path.to_string_lossy().into_owned());
        }
    }
    delta
}

/// A compact record of a chain, small enough to persist between turns.
///
/// The chain itself is rebuilt from disk every turn; what has to survive
/// between turns is only "which files were present, and what did they
/// contain", so the next turn can say what changed.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ChainRecord {
    /// A fingerprint over the whole chain, so an unchanged chain is cheap
    /// to recognise.
    #[serde(default)]
    pub fingerprint: String,
    /// `(path, fingerprint)` pairs.
    pub files: Vec<(String, String)>,
}

impl InstructionChain {
    /// A fingerprint over the whole chain, used to detect changes.
    pub fn fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        for file in &self.files {
            hasher.update(file.path.to_string_lossy().as_bytes());
            hasher.update([0u8]);
            hasher.update(file.fingerprint.as_bytes());
            hasher.update([0u8]);
        }
        format!("{:x}", hasher.finalize())
    }

    /// A compact record of this chain.
    pub fn record(&self) -> ChainRecord {
        ChainRecord {
            fingerprint: self.fingerprint(),
            files: self
                .files
                .iter()
                .map(|file| {
                    (
                        file.path.to_string_lossy().into_owned(),
                        file.fingerprint.clone(),
                    )
                })
                .collect(),
        }
    }
}

/// Compare a stored record with the chain on disk now.
pub fn diff_record(record: &ChainRecord, current: &InstructionChain) -> InstructionDelta {
    let previous = InstructionChain {
        files: record
            .files
            .iter()
            .map(|(path, fingerprint)| InstructionFile {
                path: std::path::PathBuf::from(path),
                content: String::new(),
                fingerprint: fingerprint.clone(),
            })
            .collect(),
    };
    diff(&previous, current)
}

/// Read one instruction file without following links.
///
/// Returns the file's text and its fingerprint. `None` means the file is not
/// usable as an instruction file: it is missing, linked, unreadable, or not
/// UTF-8 text.
fn read(directory: &Path, name: &str) -> Option<(String, String)> {
    let handle = Dir::open_ambient_dir(directory, ambient_authority()).ok()?;
    let mut options = OpenOptions::new();
    options.read(true);
    let mut file = crate::workspace::open_file_no_links(&handle, name, &mut options).ok()?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take(READ_BYTES as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let fingerprint = format!("{:x}", Sha256::digest(text.as_bytes()));
    Some((text, fingerprint))
}

/// Build a chain from a list of directories, outermost first.
fn build(directories: Vec<PathBuf>) -> InstructionChain {
    let mut files: Vec<InstructionFile> = Vec::new();
    let mut positions: HashMap<PathBuf, usize> = HashMap::new();
    let mut total: usize = 0;
    'outer: for directory in directories {
        for name in FILE_NAMES {
            let path = directory.join(name);
            let key = resolved(&path);
            let Some((text, fingerprint)) = read(&directory, name) else {
                continue;
            };
            let remaining = MAX_CHAIN_BYTES.saturating_sub(total);
            if remaining < SHORT_MARKER.len() {
                break 'outer;
            }
            let limit = MAX_FILE_BYTES.min(remaining);
            let chain_bound = remaining < MAX_FILE_BYTES;
            let Some(content) = clip(&text, limit, chain_bound) else {
                break 'outer;
            };
            let length = content.len();
            if let Some(position) = positions.get(&key).copied() {
                // The same file reached through two directories: the nearest
                // rule is the one that applies.
                let previous = files[position].content.len();
                if total - previous + length <= MAX_CHAIN_BYTES {
                    total = total - previous + length;
                    files[position] = InstructionFile {
                        path,
                        content,
                        fingerprint,
                    };
                }
                continue;
            }
            total += length;
            positions.insert(key, files.len());
            files.push(InstructionFile {
                path,
                content,
                fingerprint,
            });
            if chain_bound {
                break 'outer;
            }
        }
    }
    files.sort_by(|left, right| {
        depth(&left.path)
            .cmp(&depth(&right.path))
            .then_with(|| left.path.cmp(&right.path))
    });
    InstructionChain { files }
}

/// Clip text to a byte budget and mark it as truncated.
fn clip(text: &str, limit: usize, chain_bound: bool) -> Option<String> {
    if text.len() <= limit {
        return Some(text.to_string());
    }
    let long = if chain_bound {
        CHAIN_MARKER
    } else {
        FILE_MARKER
    };
    let marker = if long.len() + 16 <= limit {
        long
    } else {
        SHORT_MARKER
    };
    if marker.len() >= limit {
        return None;
    }
    let keep = limit - marker.len();
    let mut end = keep.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut content = String::with_capacity(end + marker.len());
    content.push_str(&text[..end]);
    content.push_str(marker);
    Some(content)
}

/// The directories from the filesystem root down to `root`.
///
/// An ancestor is kept only when it really contains the workspace root: the
/// ancestor's resolved path must be a prefix of the root's resolved path.
fn ancestor_directories(root: &Path) -> Option<Vec<PathBuf>> {
    let real_root = std::fs::canonicalize(root).ok()?;
    let mut chain: Vec<PathBuf> = Vec::new();
    let mut current = Some(root.to_path_buf());
    while let Some(directory) = current.clone() {
        let parent = directory.parent().map(|parent| parent.to_path_buf());
        chain.push(directory);
        current = parent;
    }
    chain.reverse();
    let mut kept: Vec<PathBuf> = Vec::new();
    for directory in chain {
        match std::fs::canonicalize(&directory) {
            Ok(real) if real_root.starts_with(&real) => kept.push(directory),
            _ => continue,
        }
    }
    Some(kept)
}

/// Make a path absolute without resolving links.
fn absolute(path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(path)
    };
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::Prefix(prefix) => out.push(prefix.as_os_str()),
            Component::RootDir => out.push(Component::RootDir.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(part) => out.push(part),
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// How deep a path sits below the filesystem root.
fn depth(path: &Path) -> usize {
    path.components()
        .filter(|component| matches!(component, Component::Normal(_)))
        .count()
}

/// A fence long enough to contain the content it wraps.
fn fence(content: &str) -> String {
    let mut longest = 0usize;
    let mut run = 0usize;
    for character in content.chars() {
        if character == '`' {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    "`".repeat(longest.max(2) + 1)
}

/// The paths named in a set of files.
fn names(files: &[InstructionFile]) -> String {
    files
        .iter()
        .map(|file| file.path.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether any directory at or above `root` publishes an instruction file.
    fn ambient_has_file(root: &Path) -> bool {
        let mut current = Some(root.to_path_buf());
        while let Some(directory) = current {
            if FILE_NAMES.iter().any(|name| directory.join(name).is_file()) {
                return true;
            }
            current = directory.parent().map(|parent| parent.to_path_buf());
        }
        false
    }

    fn file_in<'a>(chain: &'a InstructionChain, path: &Path) -> Option<&'a InstructionFile> {
        chain.files.iter().find(|file| file.path == path)
    }

    #[test]
    fn only_agents_md_is_recognized() {
        assert!(FILE_NAMES.contains(&"AGENTS.md"));
        assert!(!FILE_NAMES.contains(&"CLAUDE.md"));
    }

    #[test]
    fn chain_returns_the_workspace_file() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("AGENTS.md");
        std::fs::write(&file, "root rule").unwrap();
        let chain = chain(temp.path());
        let found = file_in(&chain, &file).expect("workspace file");
        assert_eq!(found.content, "root rule");
        assert!(!found.fingerprint.is_empty());
    }

    #[test]
    fn chain_finds_an_ancestor_file() {
        let temp = tempfile::tempdir().unwrap();
        let ancestor = temp.path().join("AGENTS.md");
        std::fs::write(&ancestor, "ancestor rule").unwrap();
        let child = temp.path().join("child");
        std::fs::create_dir_all(&child).unwrap();
        let chain = chain(&child);
        let found = file_in(&chain, &ancestor).expect("ancestor file");
        assert_eq!(found.content, "ancestor rule");
    }

    #[test]
    fn chain_without_a_file_is_empty_and_never_errors() {
        let temp = tempfile::tempdir().unwrap();
        let chain = chain(temp.path());
        assert!(chain
            .files
            .iter()
            .all(|file| !file.path.starts_with(temp.path())));
        assert_eq!(chain.files.is_empty(), !ambient_has_file(temp.path()));
        assert_eq!(chain.render(), "");
    }

    #[test]
    fn chain_skips_a_non_utf8_file() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("AGENTS.md"), [0xff, 0xfe, 0x00]).unwrap();
        let chain = chain(temp.path());
        assert!(chain
            .files
            .iter()
            .all(|file| !file.path.starts_with(temp.path())));
    }

    #[test]
    fn chain_ignores_claude_md() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("CLAUDE.md"), "not a rule").unwrap();
        let chain = chain(temp.path());
        assert!(chain
            .files
            .iter()
            .all(|file| file.path.file_name().unwrap() != "CLAUDE.md"));
    }

    #[test]
    fn discover_reports_a_nested_file_as_new() {
        let temp = tempfile::tempdir().unwrap();
        let root_file = temp.path().join("AGENTS.md");
        std::fs::write(&root_file, "root rule").unwrap();
        let nested = temp.path().join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        let nested_file = nested.join("AGENTS.md");
        std::fs::write(&nested_file, "nested rule").unwrap();
        let target = nested.join("target.txt");
        std::fs::write(&target, "content").unwrap();
        let found = discover(temp.path(), &target);
        let discovered = file_in(&found, &nested_file).expect("nested file");
        assert_eq!(discovered.content, "nested rule");
        // The workspace file was already in the chain: it is not new.
        assert!(file_in(&found, &root_file).is_none());
    }

    #[test]
    fn discover_skips_files_outside_the_workspace() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        std::fs::create_dir_all(&root).unwrap();
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("AGENTS.md"), "outside rule").unwrap();
        let target = outside.join("target.txt");
        std::fs::write(&target, "content").unwrap();
        let found = discover(&root, &target);
        assert!(found.files.is_empty());
    }

    #[test]
    fn render_names_every_file() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let outer = temp.path().join("AGENTS.md");
        std::fs::write(&outer, "outer rule").unwrap();
        let inner = workspace.join("AGENTS.md");
        std::fs::write(&inner, "inner rule").unwrap();
        let rendered = chain(&workspace).render();
        assert!(rendered.contains(&outer.to_string_lossy().into_owned()));
        assert!(rendered.contains(&inner.to_string_lossy().into_owned()));
        assert!(rendered.contains("outer rule"));
        assert!(rendered.contains("inner rule"));
        // The nearest rule reads last.
        assert!(rendered.find("outer rule").unwrap() < rendered.find("inner rule").unwrap());
    }

    #[test]
    fn diff_reports_added_changed_and_removed_files() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let outer = temp.path().join("AGENTS.md");
        std::fs::write(&outer, "one").unwrap();
        let first = chain(&workspace);
        std::fs::write(&outer, "two").unwrap();
        let inner = workspace.join("AGENTS.md");
        std::fs::write(&inner, "inner").unwrap();
        let second = chain(&workspace);
        let delta = diff(&first, &second);
        assert!(!delta.is_empty());
        assert!(file_in(
            &InstructionChain {
                files: delta.changed.clone()
            },
            &outer
        )
        .is_some());
        assert!(file_in(
            &InstructionChain {
                files: delta.added.clone()
            },
            &inner
        )
        .is_some());
        assert!(delta.removed.is_empty());
        let described = delta.describe();
        assert!(!described.is_empty());
        assert!(described.contains("instruction"));
        assert!(described.contains(&inner.to_string_lossy().into_owned()));
        // Everything gone: the session is told the rules no longer apply.
        std::fs::remove_file(&outer).unwrap();
        std::fs::remove_file(&inner).unwrap();
        let third = chain(&workspace);
        let removed = diff(&second, &third);
        assert!(!removed.removed.is_empty());
        assert!(removed
            .removed
            .iter()
            .any(|path| path == &outer.to_string_lossy().into_owned()));
        assert!(removed.describe().contains("no longer apply"));
    }

    #[test]
    fn fingerprint_is_stable_until_the_content_changes() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("AGENTS.md");
        std::fs::write(&file, "one").unwrap();
        let first = chain(temp.path());
        let second = chain(temp.path());
        assert_eq!(first.fingerprint(), second.fingerprint());
        assert_eq!(
            file_in(&first, &file).unwrap().fingerprint,
            file_in(&second, &file).unwrap().fingerprint
        );
        std::fs::write(&file, "two").unwrap();
        let third = chain(temp.path());
        assert_ne!(first.fingerprint(), third.fingerprint());
        assert_ne!(
            file_in(&first, &file).unwrap().fingerprint,
            file_in(&third, &file).unwrap().fingerprint
        );
    }

    #[test]
    fn a_large_file_is_clipped_and_the_clip_is_reported() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("AGENTS.md");
        std::fs::write(&file, "a".repeat(MAX_FILE_BYTES + 4096)).unwrap();
        let chain = chain(temp.path());
        let found = file_in(&chain, &file).expect("workspace file");
        assert!(found.content.len() <= MAX_FILE_BYTES);
        assert!(found
            .content
            .contains("[localLM: instruction file truncated"));
        assert!(chain.render().contains("truncated"));
    }

    #[test]
    fn the_chain_is_ordered_from_the_outermost_directory_inwards() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("AGENTS.md"), "outer").unwrap();
        let child = temp.path().join("child");
        std::fs::create_dir_all(&child).unwrap();
        std::fs::write(child.join("AGENTS.md"), "inner").unwrap();
        let chain = chain(&child);
        let outer = chain
            .files
            .iter()
            .position(|file| file.path == temp.path().join("AGENTS.md"))
            .expect("outer file");
        let inner = chain
            .files
            .iter()
            .position(|file| file.path == child.join("AGENTS.md"))
            .expect("inner file");
        assert!(outer < inner, "the nearest rule must read last");
    }

    #[cfg(unix)]
    #[test]
    fn the_nearest_directory_wins_when_one_file_is_reachable_twice() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("parent");
        std::fs::create_dir_all(&parent).unwrap();
        std::fs::write(parent.join("AGENTS.md"), "shared rule").unwrap();
        // A directory that resolves to its own parent: the same file is
        // reachable through the workspace root and through its ancestor.
        let link = parent.join("self");
        std::os::unix::fs::symlink(".", &link).unwrap();
        let chain = chain(&link);
        let shared: Vec<&InstructionFile> = chain
            .files
            .iter()
            .filter(|file| file.path.ends_with("AGENTS.md"))
            .collect();
        assert_eq!(shared.len(), 1, "{shared:?}");
        assert_eq!(shared[0].path, link.join("AGENTS.md"));
        assert_eq!(shared[0].content, "shared rule");
    }
}
