//! Hash-pinned native file transactions. Individual file replacement is atomic;
//! a durable journal supports rollback across files. External applications do
//! not participate in our root lock: hashes provide optimistic conflict checks,
//! not an operating-system compare-and-swap or cross-file isolation guarantee.
use super::{
    is_generated_dir, open_file_no_links, parent_no_links, parse_hash, unified_diff, valid_path,
    Workspace,
};
use cap_fs_ext::DirExt;
use cap_std::fs::{Dir, OpenOptions, Permissions};

/// What one scanned workspace file contributes to a patch.
type ScannedFile = (String, u64, Option<Vec<u8>>, Option<Permissions>);
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, RwLock, Weak},
};

#[path = "workspace_file_identity.rs"]
pub(crate) mod file_identity;

const JOURNAL: &str = ".locallm-transactions";
const MAX_FILE: usize = 1024 * 1024;
const MAX_BATCH: usize = 8 * MAX_FILE;
const MAX_MANIFEST: usize = 128 * 1024;
/// Directory transactions journal before+after bytes for an 8 MiB tree and
/// carry far more entries; recovery bounds every manifest by the largest cap.
const TREE_TARGETS: usize = 2048;
const TREE_BATCH: usize = 16 * MAX_FILE;
const TREE_MANIFEST: usize = 8 * 1024 * 1024;
/// list_tree returns at most this many entries while its full-tree hash scans
/// deeper trees than mutations accept, erroring rather than pinning a
/// partial view. Mutation scans share the delete/copy/move bounds.
const TREE_DISPLAY: usize = 2_000;
const TREE_MUTATE_ENTRIES: usize = 2_000;
const TREE_MUTATE_BYTES: usize = 8 * 1024 * 1024;
const TREE_LIST_ENTRIES: usize = 10_000;
const TREE_LIST_BYTES: usize = 64 * 1024 * 1024;
type Locks = HashMap<PathBuf, Weak<RwLock<()>>>;

/// Shared root coordination for every native workspace/editor entry point.
/// Pure reads take `read()` and may overlap; mutations and recovery take
/// `write()`, which still serializes against every concurrent native reader.
pub(crate) fn root_lock(root: &Path) -> Result<Arc<RwLock<()>>, String> {
    static LOCKS: OnceLock<Mutex<Locks>> = OnceLock::new();
    let root = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    let mut locks = LOCKS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "Workspace lock registry unavailable.")?;
    locks.retain(|_, lock| lock.strong_count() > 0);
    let slot = locks.entry(root).or_default();
    if let Some(lock) = slot.upgrade() {
        return Ok(lock);
    }
    let lock = Arc::new(RwLock::new(()));
    *slot = Arc::downgrade(&lock);
    Ok(lock)
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn io(e: std::io::Error) -> String {
    e.to_string()
}
fn id_valid(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn name(id: &str, index: usize, kind: &str) -> String {
    format!(".locallm-txn-{id}-{index}-{kind}")
}

pub(crate) fn sync_dir(dir: &Dir) -> Result<(), String> {
    #[cfg(unix)]
    // cap-std may hold an O_PATH descriptor, which cannot be fsynced. Reopen
    // the same pinned directory read-only rather than syncing its clone.
    dir.open(".").map_err(io)?.sync_all().map_err(io)?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

pub(super) fn bytes(dir: &Dir, path: &str, limit: usize) -> Result<Option<Vec<u8>>, String> {
    match dir.symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io(e)),
        Ok(meta) if !meta.is_file() || meta.is_symlink() || meta.len() > limit as u64 => {
            return Err(format!(
                "'{path}' must be a regular nonlinked file of at most {limit} bytes."
            ))
        }
        _ => (),
    }
    let file = open_file_no_links(dir, path, OpenOptions::new().read(true)).map_err(io)?;
    let mut data = Vec::new();
    file.take((limit + 1) as u64)
        .read_to_end(&mut data)
        .map_err(io)?;
    if data.len() > limit {
        return Err(format!("'{path}' grew beyond the file limit."));
    }
    Ok(Some(data))
}

fn write_new(
    dir: &Dir,
    path: &str,
    data: &[u8],
    permissions: Option<Permissions>,
) -> Result<(), String> {
    let mut file = open_file_no_links(dir, path, OpenOptions::new().write(true).create_new(true))
        .map_err(io)?;
    file.write_all(data).map_err(io)?;
    if let Some(permissions) = permissions {
        file.set_permissions(permissions).map_err(io)?;
    }
    file.sync_all().map_err(io)
}

fn journal(dir: &Dir, create: bool) -> Result<Option<Dir>, String> {
    if create {
        match dir.create_dir(JOURNAL) {
            Ok(()) => sync_dir(dir)?,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(io(e)),
        }
    }
    match dir.open_dir_nofollow(JOURNAL) {
        Ok(dir) => Ok(Some(dir)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!(
            "Cannot open transaction journal without following links: {e}"
        )),
    }
}

pub(super) fn transaction_ids(dir: &Dir) -> Result<Vec<String>, String> {
    let Some(journal) = journal(dir, false)? else {
        return Ok(Vec::new());
    };
    let mut ids = Vec::new();
    for (count, entry) in journal.entries().map_err(io)?.enumerate() {
        if count >= 128 {
            return Err(
                "Transaction journal exceeds its 128-entry bound; inspect it before continuing."
                    .into(),
            );
        }
        let entry = entry.map_err(io)?;
        let filename = entry.file_name().to_string_lossy().into_owned();
        if let Some(id) = filename.strip_suffix(".json") {
            if !id_valid(id) {
                return Err(
                    "Invalid transaction journal filename; manual inspection required.".into(),
                );
            }
            ids.push(id.to_string());
        }
    }
    ids.sort();
    Ok(ids)
}

pub(crate) fn ensure_clear(dir: &Dir) -> Result<(), String> {
    let ids = transaction_ids(dir)?;
    if ids.is_empty() {
        Ok(())
    } else {
        Err(format!("Workspace has pending transaction(s): {}. Use workspace_recover_patch with transaction_id to roll back prepared changes or clean committed changes before continuing. Reads never perform recovery automatically.", ids.join(", ")))
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Change {
    Edit {
        path: String,
        expected_sha256: String,
        replacements: Vec<Replacement>,
    },
    Create {
        path: String,
        content: String,
    },
    Delete {
        path: String,
        expected_sha256: String,
    },
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Replacement {
    old_text: String,
    new_text: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Patch {
    changes: Vec<Change>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Transfer {
    path: String,
    destination: String,
    expected_sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Delete {
    path: String,
    expected_sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recover {
    transaction_id: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteFile {
    path: String,
    content: String,
    expected_sha256: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteBase64 {
    path: String,
    content_base64: String,
    expected_sha256: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Append {
    path: String,
    content: String,
    expected_sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MultiEdit {
    path: String,
    expected_sha256: String,
    edits: Vec<Replacement>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditLines {
    path: String,
    expected_sha256: String,
    start_line: usize,
    end_line: usize,
    new_text: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TreeRequest {
    path: String,
    #[serde(default = "tree_depth")]
    max_depth: usize,
    #[serde(default)]
    include_generated: bool,
}
fn tree_depth() -> usize {
    3
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeleteDir {
    path: String,
    recursive: bool,
    expected_tree_sha256: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TransferDir {
    path: String,
    destination: String,
    expected_tree_sha256: String,
}

pub(super) struct Desired {
    pub(super) path: String,
    pub(super) expected: Option<String>,
    pub(super) after: Option<Vec<u8>>,
    pub(super) permissions: Option<Permissions>,
}
struct Target {
    rename_to: Option<String>,
    identity: Option<(u64, u64)>,
    parent: Dir,
    leaf: String,
    before: Option<Vec<u8>>,
    after: Option<Vec<u8>>,
    permissions: Option<Permissions>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rename_to: Option<String>,
    path: String,
    before: Option<String>,
    after: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u8,
    id: String,
    state: String,
    entries: Vec<Entry>,
}

pub(super) fn require_hash(expected: &str, data: &[u8]) -> Result<(), String> {
    parse_hash(expected)?;
    if hash(data).eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err("File hash changed. Read the current file or file_info and retry; no project files were changed.".into())
    }
}

fn unique_paths(paths: impl Iterator<Item = String>) -> Result<(), String> {
    let mut seen = HashSet::new();
    for path in paths {
        valid_path(&path, false)?;
        if !seen.insert(path.to_lowercase()) {
            return Err("Duplicate or case-aliased transaction targets are not allowed.".into());
        }
    }
    Ok(())
}

// Exact directory spelling is essential on case-insensitive filesystems:
// looking up the destination alone can open the original source instead.
fn exact_leaf(parent: &Dir, leaf: &str) -> Result<bool, String> {
    for entry in parent.entries().map_err(io)? {
        if entry.map_err(io)?.file_name() == std::ffi::OsStr::new(leaf) {
            return Ok(true);
        }
    }
    Ok(false)
}
pub(super) fn case_destination(root: &Dir, old: &str, new: &str) -> Result<bool, String> {
    valid_path(old, false)?;
    valid_path(new, false)?;
    if old == new
        || old.to_lowercase() != new.to_lowercase()
        || Path::new(old).parent() != Path::new(new).parent()
    {
        return Err("Case-only file renames require the same parent spelling.".into());
    }
    let (parent, leaf) = parent_no_links(root, old).map_err(io)?;
    if !exact_leaf(&parent, &leaf)? {
        return Err("Rename source spelling changed.".into());
    }
    let source = open_file_no_links(&parent, &leaf, OpenOptions::new().read(true)).map_err(io)?;
    let source_id = file_identity::single_link_identity(&source)?;
    let new_leaf = Path::new(new)
        .file_name()
        .ok_or("Missing rename destination.")?
        .to_str()
        .ok_or("Non-UTF8 destination.")?;
    if exact_leaf(&parent, new_leaf)? {
        return Ok(false);
    }
    if bytes(&parent, new_leaf, MAX_FILE)?.is_some() {
        let dest =
            open_file_no_links(&parent, new_leaf, OpenOptions::new().read(true)).map_err(io)?;
        if file_identity::single_link_identity(&dest)? != source_id {
            return Err("Rename destination resolves to a different file.".into());
        }
    }
    Ok(true)
}
fn preflight(
    root: &Dir,
    desired: Vec<Desired>,
    max_targets: usize,
    max_batch: usize,
) -> Result<(Vec<Target>, Vec<Entry>), String> {
    if desired.is_empty() || desired.len() > max_targets {
        return Err(format!(
            "A transaction must change between 1 and {max_targets} files."
        ));
    }
    let mut desired = desired;
    let mut renames = HashMap::new();
    let mut i = 0;
    while i < desired.len() {
        if desired[i].after.is_none() && desired[i].expected.is_some() {
            let old = desired[i].path.clone();
            if let Some(j) = (0..desired.len()).find(|&j| {
                j != i
                    && desired[j].path != old
                    && desired[j].path.to_lowercase() == old.to_lowercase()
            }) {
                if desired[j].expected.is_some()
                    || desired[j].after.is_none()
                    || !case_destination(root, &old, &desired[j].path)?
                {
                    return Err(
                        "Case-only rename destination collides with an existing file.".into(),
                    );
                }
                let dest = desired.remove(j);
                if j < i {
                    i -= 1;
                }
                desired[i].after = dest.after;
                renames.insert(old, dest.path);
            }
        }
        i += 1;
    }
    unique_paths(desired.iter().map(|d| d.path.clone()))?;
    let mut targets = Vec::new();
    let mut entries = Vec::new();
    let mut total = 0;
    let mut identities = HashSet::new();
    for item in desired {
        let (parent, leaf) = parent_no_links(root, &item.path).map_err(io)?;
        let before = bytes(&parent, &leaf, MAX_FILE)?;
        match (&item.expected, &before) {
            (Some(expected), Some(before)) => require_hash(expected, before)?,
            (None, None) => (),
            (None, Some(_)) => {
                return Err(format!(
                    "Destination '{}' already exists; it was preserved.",
                    item.path
                ))
            }
            _ => return Err(format!("Expected file '{}' is missing.", item.path)),
        }
        let permissions = if before.is_some() {
            let file =
                open_file_no_links(&parent, &leaf, OpenOptions::new().read(true)).map_err(io)?;
            let meta = file.metadata().map_err(io)?;
            if !identities.insert(file_identity::single_link_identity(&file)?) {
                return Err("Two patch paths resolve to the same file.".into());
            }
            if meta.permissions().readonly() {
                return Err(format!("'{}' is read-only.", item.path));
            }
            Some(meta.permissions())
        } else {
            item.permissions
        };
        total += before.as_ref().map_or(0, Vec::len) + item.after.as_ref().map_or(0, Vec::len);
        if total > max_batch || item.after.as_ref().is_some_and(|b| b.len() > MAX_FILE) {
            return Err(format!(
                "Patch exceeds the 1 MiB per-file or {max_batch} byte aggregate bound."
            ));
        }
        let rename_to = renames.remove(&item.path);
        if before == item.after && rename_to.is_none() {
            return Err(format!("'{}' has no change.", item.path));
        }
        entries.push(Entry {
            rename_to: rename_to.clone(),
            path: item.path,
            before: before.as_ref().map(|b| hash(b)),
            after: item.after.as_ref().map(|b| hash(b)),
        });
        let identity = if before.is_some() {
            Some(file_identity::single_link_identity(
                &open_file_no_links(&parent, &leaf, OpenOptions::new().read(true)).map_err(io)?,
            )?)
        } else {
            None
        };
        targets.push(Target {
            rename_to,
            identity,
            parent,
            leaf,
            before,
            after: item.after,
            permissions,
        });
    }
    Ok((targets, entries))
}

fn save_manifest(journal: &Dir, manifest: &Manifest, limit: usize) -> Result<(), String> {
    let data = serde_json::to_vec(manifest).map_err(|e| e.to_string())?;
    if data.len() > limit {
        return Err(format!(
            "Transaction manifest exceeds the {limit} byte bound."
        ));
    }
    let temp = format!("{}.next", manifest.id);
    // Only our own ID is reused while holding the root lock.
    match journal.remove_file(&temp) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
        Err(e) => return Err(io(e)),
    }
    write_new(journal, &temp, &data, None)?;
    journal
        .rename(&temp, journal, format!("{}.json", manifest.id))
        .map_err(io)?;
    sync_dir(journal)
}

fn check_target(target: &Target, expected: &Option<String>) -> Result<(), String> {
    if let Some(identity) = target.identity {
        let file = open_file_no_links(&target.parent, &target.leaf, OpenOptions::new().read(true))
            .map_err(io)?;
        if file.metadata().map_err(io)?.permissions().readonly() {
            return Err("Transaction target became read-only.".into());
        }
        if file_identity::single_link_identity(&file)? != identity {
            return Err("Transaction target identity changed.".into());
        }
    }
    if let Some(destination) = &target.rename_to {
        let leaf = Path::new(destination)
            .file_name()
            .ok_or("Missing rename leaf.")?
            .to_str()
            .ok_or("Invalid rename leaf.")?;
        if !exact_leaf(&target.parent, &target.leaf)? || exact_leaf(&target.parent, leaf)? {
            return Err("Case-only rename spelling/collision changed.".into());
        }
    }
    let current = bytes(&target.parent, &target.leaf, MAX_FILE)?
        .as_ref()
        .map(|b| hash(b));
    if &current != expected {
        return Err(format!("'{}' changed during the transaction.", target.leaf));
    }
    Ok(())
}

fn cleanup(root: &Dir, journal: &Dir, manifest: &Manifest) -> Result<(), String> {
    for (index, entry) in manifest.entries.iter().enumerate() {
        let (parent, _) = parent_no_links(root, &entry.path).map_err(io)?;
        for kind in ["new", "old", "restore", "probe", "park"] {
            match parent.remove_file(name(&manifest.id, index, kind)) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(io(e)),
            }
        }
        sync_dir(&parent)?;
    }
    journal
        .remove_file(format!("{}.json", manifest.id))
        .map_err(io)?;
    sync_dir(journal)
}

fn rollback(root: &Dir, manifest: &Manifest) -> Result<Vec<String>, String> {
    // Validate all recovery material before changing even the first target.
    let mut originals = Vec::new();
    for (index, entry) in manifest.entries.iter().enumerate() {
        let (parent, leaf) = parent_no_links(root, &entry.path).map_err(io)?;
        let original = bytes(&parent, &name(&manifest.id, index, "old"), MAX_FILE)?;
        let current = bytes(&parent, &leaf, MAX_FILE)?.as_ref().map(|b| hash(b));
        let original_hash = original.as_ref().map(|b| hash(b));
        if original_hash != entry.before && !(original.is_none() && current == entry.before) {
            return Err(format!("Recovery backup is missing or corrupt for '{}'; original files were not overwritten.", entry.path));
        }
        originals.push((parent, leaf, original));
    }
    let mut conflicts = Vec::new();
    for (index, ((parent, leaf, original), entry)) in originals
        .into_iter()
        .zip(&manifest.entries)
        .enumerate()
        .rev()
    {
        let result = (|| -> Result<(), String> {
            if let Some(destination) = &entry.rename_to {
                let dest = Path::new(destination)
                    .file_name()
                    .ok_or("Missing recovery destination.")?
                    .to_str()
                    .ok_or("Invalid recovery destination.")?;
                let old_exists = exact_leaf(&parent, &leaf)?;
                let new_exists = exact_leaf(&parent, dest)?;
                if old_exists && new_exists {
                    return Err("Concurrent destination preserved".into());
                }
                if old_exists {
                    require_hash(
                        entry.before.as_deref().ok_or("Missing original hash.")?,
                        &bytes(&parent, &leaf, MAX_FILE)?.ok_or("Missing original.")?,
                    )?;
                    return Ok(());
                }
                if new_exists {
                    require_hash(
                        entry.after.as_deref().ok_or("Missing destination hash.")?,
                        &bytes(&parent, dest, MAX_FILE)?.ok_or("Missing destination.")?,
                    )?;
                }
                let park = name(&manifest.id, index, "park");
                let parked = bytes(&parent, &park, MAX_FILE)?
                    .ok_or("Parked rename original missing; recovery required.")?;
                require_hash(
                    entry.before.as_deref().ok_or("Missing original hash.")?,
                    &parked,
                )?;
                // Preserve writers holding a handle to the original parked inode.
                file_identity::single_link_identity(
                    &open_file_no_links(&parent, &park, OpenOptions::new().read(true))
                        .map_err(io)?,
                )?;
                if new_exists {
                    parent.remove_file(dest).map_err(io)?;
                }
                parent.hard_link(&park, &parent, &leaf).map_err(io)?;
                parent.remove_file(&park).map_err(io)?;
                return sync_dir(&parent);
            }
            let current = bytes(&parent, &leaf, MAX_FILE)?.as_ref().map(|b| hash(b));
            if current == entry.before {
                return Ok(());
            }
            if current != entry.after {
                return Err("Concurrent edit preserved".into());
            }
            if let Some(original) = original {
                let restore = name(&manifest.id, index, "restore");
                // A previous interrupted rollback may have left this exact controlled name.
                match parent.remove_file(&restore) {
                    Ok(()) => (),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                    Err(e) => return Err(io(e)),
                }
                let permissions = parent
                    .symlink_metadata(name(&manifest.id, index, "old"))
                    .map_err(io)?
                    .permissions();
                write_new(&parent, &restore, &original, Some(permissions))?;
                // Recheck immediately before replacing; external writers do not share our lock.
                let fresh = bytes(&parent, &leaf, MAX_FILE)?.as_ref().map(|b| hash(b));
                if fresh != current {
                    return Err("Concurrent edit preserved".into());
                }
                if current.is_some() {
                    parent.rename(&restore, &parent, &leaf).map_err(io)?;
                } else {
                    parent.hard_link(&restore, &parent, &leaf).map_err(io)?;
                    parent.remove_file(&restore).map_err(io)?;
                }
            } else {
                parent.remove_file(&leaf).map_err(io)?;
            }
            sync_dir(&parent)
        })();
        if let Err(reason) = result {
            conflicts.push(format!("{}: {reason}", entry.path));
        }
    }
    Ok(conflicts)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TreeKind {
    File,
    Directory,
    Link,
    Other,
}
impl TreeKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Directory => "directory",
            Self::Link => "link",
            Self::Other => "other",
        }
    }
}
struct TreeEntry {
    path: String,
    depth: usize,
    kind: TreeKind,
    size: u64,
    sha: String,
    content: Option<Vec<u8>>,
    permissions: Option<Permissions>,
}
struct ScanBounds {
    entries: usize,
    bytes: usize,
}

/// Stream one file's SHA-256 without a per-file size cap; callers bound the
/// tree totals, and mutations re-verify every byte through the journal.
fn hash_file(
    dir: &Dir,
    name: &str,
    collect: bool,
    bounds: &ScanBounds,
    hashed: &mut usize,
) -> Result<ScannedFile, String> {
    let file = open_file_no_links(dir, name, OpenOptions::new().read(true)).map_err(io)?;
    let meta = file.metadata().map_err(io)?;
    let mut reader = &file;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 65_536];
    let mut content = collect.then(Vec::new);
    loop {
        let read = reader.read(&mut buffer).map_err(io)?;
        if read == 0 {
            break;
        }
        *hashed += read;
        if *hashed > bounds.bytes {
            return Err("Directory tree exceeds its byte bound.".into());
        }
        hasher.update(&buffer[..read]);
        if let Some(content) = &mut content {
            content.extend_from_slice(&buffer[..read]);
        }
    }
    Ok((
        format!("{:x}", hasher.finalize()),
        meta.len(),
        content,
        Some(meta.permissions()),
    ))
}

/// Depth-first tree walk recording every entry. Links are reported but never
/// followed; generated directories are skipped like search unless asked.
#[allow(clippy::too_many_arguments)] // Recursive traversal shares one bounded scan state.
fn scan_tree(
    dir: &Dir,
    prefix: &str,
    depth: usize,
    include_generated: bool,
    collect: bool,
    bounds: &ScanBounds,
    hashed: &mut usize,
    out: &mut Vec<TreeEntry>,
) -> Result<(), String> {
    for entry in dir.entries().map_err(io)? {
        if out.len() >= bounds.entries {
            return Err(format!(
                "Directory tree exceeds the {} entry bound.",
                bounds.entries
            ));
        }
        let entry = entry.map_err(io)?;
        let kind = entry.file_type().map_err(io)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // Journal internals are never model-visible tree entries, matching
        // the check_fs_path part-level deny policy.
        if name == JOURNAL || name.starts_with(".locallm-txn-") {
            continue;
        }
        let path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        // Same sensitive-file policy as the workspace tools: credential paths
        // are never enumerated, hashed, moved or deleted.
        if crate::sandbox::check_fs_path("", &path, false).is_err() {
            continue;
        }
        if kind.is_symlink() {
            out.push(TreeEntry {
                path,
                depth,
                kind: TreeKind::Link,
                size: entry.metadata().map_err(io)?.len(),
                sha: String::new(),
                content: None,
                permissions: None,
            });
            continue;
        }
        if kind.is_dir() {
            if !include_generated && is_generated_dir(&name) {
                continue;
            }
            out.push(TreeEntry {
                path: path.clone(),
                depth,
                kind: TreeKind::Directory,
                size: entry.metadata().map_err(io)?.len(),
                sha: String::new(),
                content: None,
                permissions: None,
            });
            let child = dir.open_dir_nofollow(&name).map_err(io)?;
            scan_tree(
                &child,
                &path,
                depth + 1,
                include_generated,
                collect,
                bounds,
                hashed,
                out,
            )?;
            continue;
        }
        if !kind.is_file() {
            out.push(TreeEntry {
                path,
                depth,
                kind: TreeKind::Other,
                size: entry.metadata().map_err(io)?.len(),
                sha: String::new(),
                content: None,
                permissions: None,
            });
            continue;
        }
        let (sha, size, content, permissions) = hash_file(dir, &name, collect, bounds, hashed)?;
        out.push(TreeEntry {
            path,
            depth,
            kind: TreeKind::File,
            size,
            sha,
            content,
            permissions,
        });
    }
    Ok(())
}

/// Canonical subtree pin: SHA-256 over sorted "kind\tpath\tsha" lines, so a
/// list_tree result commits delete/move/copy to exactly that file set.
fn tree_sha256(entries: &[TreeEntry]) -> String {
    let mut lines: Vec<String> = entries
        .iter()
        .map(|entry| format!("{}\t{}\t{}", entry.kind.as_str(), entry.path, entry.sha))
        .collect();
    lines.sort();
    hash(lines.join("\n").as_bytes())
}

fn open_tree_dir(directory: &Dir, path: &str) -> Result<Dir, String> {
    if path == "." {
        return directory.try_clone().map_err(io);
    }
    let (parent, name) = parent_no_links(directory, path).map_err(io)?;
    parent.open_dir_nofollow(&name).map_err(io)
}

fn subtree_relative<'a>(path: &'a str, root: &str) -> &'a str {
    path.strip_prefix(&format!("{root}/")).unwrap_or(path)
}

impl Workspace {
    pub(crate) fn case_rename_destination(&self, old: &str, new: &str) -> Result<bool, String> {
        let _guard = self
            .transaction_lock
            .read()
            .map_err(|_| "Workspace lock unavailable.")?;
        ensure_clear(&self.directory)?;
        case_destination(&self.directory, old, new)
    }

    /// Apply approved language-server text edits through the existing journal
    /// and all-file preflight. No model-controlled paths bypass native policy.
    pub(crate) fn apply_language_resources(
        &self,
        changes: Vec<(String, Option<String>, Option<String>)>,
        guards: Vec<(String, String)>,
    ) -> Result<Value, String> {
        let _guard = self
            .transaction_lock
            .write()
            .map_err(|_| "Workspace transaction lock unavailable.")?;
        ensure_clear(&self.directory)?;
        self.transact(
            changes
                .into_iter()
                .map(|(path, expected, after)| Desired {
                    path,
                    expected,
                    after: after.map(String::into_bytes),
                    permissions: None,
                })
                .collect(),
            &guards,
        )
    }
    fn patch_desired(&self, arguments: Value) -> Result<Vec<Desired>, String> {
        let request: Patch = serde_json::from_value(arguments).map_err(|e| e.to_string())?;
        if request.changes.is_empty() || request.changes.len() > 16 {
            return Err("A patch must change 1–16 files.".into());
        }
        let mut desired = Vec::new();
        for change in request.changes {
            match change {
                Change::Create { path, content } => desired.push(Desired {
                    path,
                    expected: None,
                    after: Some(content.into_bytes()),
                    permissions: None,
                }),
                Change::Delete {
                    path,
                    expected_sha256,
                } => desired.push(Desired {
                    path,
                    expected: Some(expected_sha256),
                    after: None,
                    permissions: None,
                }),
                Change::Edit {
                    path,
                    expected_sha256,
                    replacements,
                } => desired.push(self.edit_desired(&path, &expected_sha256, replacements, 64)?),
            }
        }
        Ok(desired)
    }

    /// Unique, nonoverlapping replacements resolved against the original text,
    /// shared by apply_patch edits and multi_edit.
    ///
    /// The shared matcher locates every region, so a smart quote, a dash
    /// variant, an exotic space, trailing whitespace or CRLF no longer fails
    /// an otherwise exact edit. Every `old_text` must still resolve to
    /// exactly one region of the original file, and the regions must not
    /// overlap.
    fn edit_desired(
        &self,
        path: &str,
        expected_sha256: &str,
        replacements: Vec<Replacement>,
        limit: usize,
    ) -> Result<Desired, String> {
        valid_path(path, false)?;
        let data = bytes(&self.directory, path, MAX_FILE)?.ok_or("Edit target is missing.")?;
        require_hash(expected_sha256, &data)?;
        let text = String::from_utf8(data).map_err(|_| "Patch edits require UTF-8 text.")?;
        if replacements.is_empty() || replacements.len() > limit {
            return Err(format!(
                "Use 1–{limit} unique nonoverlapping replacements per edit."
            ));
        }
        let mut ranges = Vec::new();
        for replacement in replacements {
            if replacement.old_text.is_empty()
                || replacement.old_text == replacement.new_text
                || replacement.old_text.len() > 65_536
                || replacement.new_text.len() > 65_536
            {
                return Err("Each replacement needs distinct old/new text, nonempty old text, and at most 64 KiB per string.".into());
            }
            let report = crate::edit_match::find_matches(&text, &replacement.old_text);
            if report.count() != 1 {
                if !report.found() {
                    return Err(format!(
                        "The text to replace was not found ({}). Read the current file and retry.",
                        report.describe()
                    ));
                }
                return Err("Each old_text must occur exactly once in the original file.".into());
            }
            let located = report.matches[0];
            // An overlapping occurrence counts as a second one: "aa" appears
            // twice in "aaa". The step is measured in the source, whose first
            // character may be wider than the one the search text spelled.
            let first = text[located.start..].chars().next().unwrap();
            let next = located.start + first.len_utf8();
            if next < text.len() && text[next..].contains(&replacement.old_text) {
                return Err("Each old_text must occur exactly once in the original file.".into());
            }
            ranges.push((located, replacement.new_text));
        }
        ranges.sort_by_key(|range| range.0.start);
        if ranges
            .windows(2)
            .any(|pair| pair[0].0.end > pair[1].0.start)
        {
            return Err("Patch replacements overlap in the original file.".into());
        }
        // The regions are resolved against the original text, so they are
        // applied back to front; the line ending style comes from the
        // original file, not from the partially rewritten text.
        let dominant = crate::edit_match::dominant_terminator(&text);
        let mut updated = text.clone();
        for (located, replacement) in ranges.into_iter().rev() {
            updated = crate::edit_match::replace_at(&updated, &located, &replacement, dominant);
        }
        Ok(Desired {
            path: path.into(),
            expected: Some(expected_sha256.into()),
            after: Some(updated.into_bytes()),
            permissions: None,
        })
    }

    /// Replace an inclusive 1-based line range; end_line = start_line - 1
    /// inserts before start_line (which may be one past the last line).
    /// Existing line endings are normalized to the file's own style.
    fn edit_lines_desired(&self, request: &EditLines) -> Result<Desired, String> {
        valid_path(&request.path, false)?;
        if request.start_line == 0 {
            return Err("start_line is 1-based; use end_line = 0 to insert at the top.".into());
        }
        if request.new_text.len() > MAX_FILE {
            return Err("Replacement text exceeds 1 MiB.".into());
        }
        let data =
            bytes(&self.directory, &request.path, MAX_FILE)?.ok_or("Edit target is missing.")?;
        require_hash(&request.expected_sha256, &data)?;
        let text = String::from_utf8(data).map_err(|_| "Line edits require UTF-8 text.")?;
        let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
        let trailing = text.ends_with('\n');
        let mut lines: Vec<String> = if text.is_empty() {
            Vec::new()
        } else {
            text.split('\n')
                .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
                .collect()
        };
        if trailing {
            lines.pop();
        }
        let total = lines.len();
        if request.end_line < request.start_line - 1 {
            return Err("end_line must be at least start_line - 1 for insertion.".into());
        }
        if request.end_line == request.start_line - 1 {
            if request.start_line > total + 1 {
                return Err(format!(
                    "To insert, start_line may be at most one past the file's {total} lines."
                ));
            }
        } else if request.end_line > total {
            return Err(format!("end_line exceeds the file's {total} lines."));
        }
        let replacement: Vec<String> = if request.new_text.is_empty() {
            Vec::new()
        } else {
            let mut new_lines: Vec<String> = request
                .new_text
                .split('\n')
                .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
                .collect();
            if request.new_text.ends_with('\n') {
                new_lines.pop();
            }
            new_lines
        };
        lines.splice(request.start_line - 1..request.end_line, replacement);
        let mut after = lines.join(eol);
        if trailing && !after.is_empty() {
            after.push_str(eol);
        }
        Ok(Desired {
            path: request.path.clone(),
            expected: Some(request.expected_sha256.clone()),
            after: Some(after.into_bytes()),
            permissions: None,
        })
    }

    /// Scan a directory subtree until the expected hash from list_tree matches
    /// under either generated-directory policy, then refuse anything the
    /// journal cannot safely reproduce or restore.
    fn pinned_tree(&self, path: &str, expected: &str) -> Result<Vec<TreeEntry>, String> {
        parse_hash(expected)?;
        let bounds = ScanBounds {
            entries: TREE_MUTATE_ENTRIES,
            bytes: TREE_MUTATE_BYTES,
        };
        for include_generated in [false, true] {
            let dir = open_tree_dir(&self.directory, path)?;
            let mut hashed = 0usize;
            let mut entries = Vec::new();
            scan_tree(
                &dir,
                path,
                1,
                include_generated,
                true,
                &bounds,
                &mut hashed,
                &mut entries,
            )?;
            if tree_sha256(&entries).eq_ignore_ascii_case(expected) {
                if entries
                    .iter()
                    .any(|entry| matches!(entry.kind, TreeKind::Link | TreeKind::Other))
                {
                    return Err(
                        "Directory contains links or special files; refusing to change it.".into(),
                    );
                }
                if entries
                    .iter()
                    .any(|entry| entry.kind == TreeKind::File && entry.size > MAX_FILE as u64)
                {
                    return Err("Directory contains a file over the 1 MiB per-file bound.".into());
                }
                return Ok(entries);
            }
        }
        Err("Directory tree changed since list_tree. Re-run list_tree and retry with the fresh treeSha256.".into())
    }

    /// Create every destination directory shallowest-first. The directories
    /// themselves are not journaled: they are new, empty and cheap to remove.
    fn create_dirs_top_down(&self, mut dirs: Vec<String>) -> Result<(), String> {
        dirs.sort_by_key(|dir| dir.matches('/').count());
        for dir in dirs {
            let (parent, name) = parent_no_links(&self.directory, &dir).map_err(io)?;
            parent
                .create_dir(&name)
                .map_err(|e| format!("Could not create destination directory '{dir}': {e}"))?;
            sync_dir(&parent)?;
        }
        Ok(())
    }

    /// Remove directories deepest-first once their files are gone. Returns
    /// paths that could not be removed (foreign content or remove failure);
    /// callers surface them without undoing the committed file transaction.
    fn remove_dirs_bottom_up(&self, mut dirs: Vec<String>) -> Vec<String> {
        dirs.sort_by_key(|dir| std::cmp::Reverse(dir.matches('/').count()));
        let mut remaining = Vec::new();
        for dir in dirs {
            let removed = parent_no_links(&self.directory, &dir).and_then(|(parent, name)| {
                let result = parent.remove_dir(&name);
                if result.is_ok() {
                    let _ = sync_dir(&parent);
                }
                result
            });
            if removed.is_err() {
                remaining.push(dir);
            }
        }
        remaining
    }

    fn list_tree(&self, arguments: Value) -> Result<Value, String> {
        let request: TreeRequest = serde_json::from_value(arguments).map_err(|e| e.to_string())?;
        valid_path(&request.path, true)?;
        if request.max_depth == 0 || request.max_depth > 10 {
            return Err("max_depth must be between 1 and 10.".into());
        }
        let dir = open_tree_dir(&self.directory, &request.path)?;
        let prefix = if request.path == "." {
            String::new()
        } else {
            request.path.clone()
        };
        let bounds = ScanBounds {
            entries: TREE_LIST_ENTRIES,
            bytes: TREE_LIST_BYTES,
        };
        let mut hashed = 0usize;
        let mut entries = Vec::new();
        scan_tree(
            &dir,
            &prefix,
            1,
            request.include_generated,
            false,
            &bounds,
            &mut hashed,
            &mut entries,
        )?;
        let total = entries.len();
        let mut shown: Vec<&TreeEntry> = entries
            .iter()
            .filter(|entry| entry.depth <= request.max_depth)
            .collect();
        let truncated = shown.len() > TREE_DISPLAY;
        shown.truncate(TREE_DISPLAY);
        Ok(json!({
            "path": request.path,
            "entries": shown.iter().map(|entry| {
                let mut item = json!({"path":entry.path,"kind":entry.kind.as_str(),"size":entry.size});
                if entry.kind == TreeKind::File { item["sha256"] = json!(entry.sha); }
                item
            }).collect::<Vec<_>>(),
            "truncated": truncated,
            "totalEntries": total,
            "treeSha256": tree_sha256(&entries),
        }))
    }

    fn delete_directory(&self, arguments: Value) -> Result<Value, String> {
        let request: DeleteDir = serde_json::from_value(arguments).map_err(|e| e.to_string())?;
        valid_path(&request.path, false)?;
        let metadata = self
            .directory
            .symlink_metadata(&request.path)
            .map_err(|e| format!("Directory is missing: {e}"))?;
        if !metadata.file_type().is_dir() {
            return Err("Path is not a directory.".into());
        }
        if !request.recursive {
            let (parent, name) = parent_no_links(&self.directory, &request.path).map_err(io)?;
            parent.remove_dir(&name).map_err(|e| {
                format!("Non-recursive delete only removes an empty directory: {e}")
            })?;
            sync_dir(&parent)?;
            return Ok(json!({"path":request.path,"deleted":true,"recursive":false}));
        }
        let expected = request
            .expected_tree_sha256
            .ok_or("Recursive delete requires expected_tree_sha256 from list_tree.")?;
        let entries = self.pinned_tree(&request.path, &expected)?;
        let desired: Vec<Desired> = entries
            .iter()
            .filter(|entry| entry.kind == TreeKind::File)
            .map(|entry| Desired {
                path: entry.path.clone(),
                expected: Some(entry.sha.clone()),
                after: None,
                permissions: None,
            })
            .collect();
        let mut result = if desired.is_empty() {
            json!({"status":"committed","changes":[]})
        } else {
            self.transact_tree(desired, &[])?
        };
        if result["isError"] == true {
            return Ok(result);
        }
        let mut dirs: Vec<String> = entries
            .iter()
            .filter(|entry| entry.kind == TreeKind::Directory)
            .map(|entry| entry.path.clone())
            .collect();
        let dir_count = dirs.len() + 1;
        dirs.push(request.path.clone());
        let remaining = self.remove_dirs_bottom_up(dirs);
        result["removedDirectories"] = json!(dir_count - remaining.len());
        result["remainingDirectories"] = json!(remaining);
        Ok(result)
    }

    fn transfer_directory(&self, tool: &str, arguments: Value) -> Result<Value, String> {
        let request: TransferDir = serde_json::from_value(arguments).map_err(|e| e.to_string())?;
        valid_path(&request.path, false)?;
        valid_path(&request.destination, false)?;
        if request.destination == request.path
            || request
                .destination
                .starts_with(&format!("{}/", request.path))
        {
            return Err("Destination cannot be the directory itself or inside it.".into());
        }
        let metadata = self
            .directory
            .symlink_metadata(&request.path)
            .map_err(|e| format!("Source directory is missing: {e}"))?;
        if !metadata.file_type().is_dir() {
            return Err("Source is not a directory.".into());
        }
        let (dest_parent, dest_name) = parent_no_links(&self.directory, &request.destination)
            .map_err(|e| format!("Destination parent must exist: {e}"))?;
        match dest_parent.symlink_metadata(&dest_name) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
            _ => return Err("Destination already exists; it was preserved.".into()),
        }
        let entries = self.pinned_tree(&request.path, &request.expected_tree_sha256)?;
        // Directories are created first so journaled file creates land in a
        // finished tree; on any failure they are removed empty, bottom-up.
        let mut dir_paths: Vec<String> = entries
            .iter()
            .filter(|entry| entry.kind == TreeKind::Directory)
            .map(|entry| {
                format!(
                    "{}/{}",
                    request.destination,
                    subtree_relative(&entry.path, &request.path)
                )
            })
            .collect();
        dir_paths.insert(0, request.destination.clone());
        if let Err(reason) = self.create_dirs_top_down(dir_paths.clone()) {
            self.remove_dirs_bottom_up(dir_paths);
            return Err(reason);
        }
        let mut desired = Vec::new();
        let mut guards = Vec::new();
        for entry in entries.iter().filter(|entry| entry.kind == TreeKind::File) {
            desired.push(Desired {
                path: format!(
                    "{}/{}",
                    request.destination,
                    subtree_relative(&entry.path, &request.path)
                ),
                expected: None,
                after: entry.content.clone(),
                permissions: entry.permissions.clone(),
            });
            if tool == "copy_directory" {
                guards.push((entry.path.clone(), entry.sha.clone()));
            }
        }
        if tool == "move_directory" {
            for entry in entries.iter().filter(|entry| entry.kind == TreeKind::File) {
                desired.push(Desired {
                    path: entry.path.clone(),
                    expected: Some(entry.sha.clone()),
                    after: None,
                    permissions: None,
                });
            }
        }
        let mut result = if desired.is_empty() {
            json!({"status":"committed","changes":[]})
        } else {
            match self.transact_tree(desired, &guards) {
                Ok(result) => result,
                Err(reason) => {
                    self.remove_dirs_bottom_up(dir_paths);
                    return Err(reason);
                }
            }
        };
        if result["isError"] == true {
            self.remove_dirs_bottom_up(dir_paths);
            return Ok(result);
        }
        if tool == "move_directory" {
            let mut dirs: Vec<String> = entries
                .iter()
                .filter(|entry| entry.kind == TreeKind::Directory)
                .map(|entry| entry.path.clone())
                .collect();
            dirs.push(request.path.clone());
            result["remainingDirectories"] = json!(self.remove_dirs_bottom_up(dirs));
        }
        Ok(result)
    }

    pub(super) fn transact(
        &self,
        desired: Vec<Desired>,
        read_guards: &[(String, String)],
    ) -> Result<Value, String> {
        self.transact_with_commit_hook(desired, read_guards, |_, _, _| {})
    }

    /// Directory operations journal whole trees: more targets, and the
    /// aggregate covers before+after bytes of a bounded source tree.
    fn transact_tree(
        &self,
        desired: Vec<Desired>,
        read_guards: &[(String, String)],
    ) -> Result<Value, String> {
        self.transact_with_commit_hook_limits(
            desired,
            read_guards,
            TREE_TARGETS,
            TREE_BATCH,
            TREE_MANIFEST,
            |_, _, _| {},
        )
    }

    fn transact_with_commit_hook(
        &self,
        desired: Vec<Desired>,
        read_guards: &[(String, String)],
        before_commit: impl FnMut(usize, &Target, &Manifest),
    ) -> Result<Value, String> {
        self.transact_with_commit_hook_limits(
            desired,
            read_guards,
            16,
            MAX_BATCH,
            MAX_MANIFEST,
            before_commit,
        )
    }

    fn transact_with_commit_hook_limits(
        &self,
        desired: Vec<Desired>,
        read_guards: &[(String, String)],
        max_targets: usize,
        max_batch: usize,
        max_manifest: usize,
        mut before_commit: impl FnMut(usize, &Target, &Manifest),
    ) -> Result<Value, String> {
        let (targets, entries) = preflight(&self.directory, desired, max_targets, max_batch)?;
        // Establish that the journal is usable before creating recovery blobs.
        let journal = journal(&self.directory, true)?.ok_or("Could not create journal.")?;
        let id = uuid::Uuid::new_v4().simple().to_string();
        let mut manifest = Manifest {
            version: 1,
            id,
            state: "prepared".into(),
            entries,
        };
        // Prepare all same-parent backups/stages before the first project mutation.
        let staged = (|| -> Result<(), String> {
            for (index, target) in targets.iter().enumerate() {
                if let Some(before) = &target.before {
                    write_new(
                        &target.parent,
                        &name(&manifest.id, index, "old"),
                        before,
                        target.permissions.clone(),
                    )?;
                }
                if let Some(after) = &target.after {
                    let stage = name(&manifest.id, index, "new");
                    write_new(&target.parent, &stage, after, target.permissions.clone())?;
                    // Prove no-clobber hard-link creation is available before changing project files.
                    if target.before.is_none() || target.rename_to.is_some() {
                        let probe = name(&manifest.id, index, "probe");
                        target
                            .parent
                            .hard_link(&stage, &target.parent, &probe)
                            .map_err(io)?;
                        target.parent.remove_file(&probe).map_err(io)?;
                    }
                }
                sync_dir(&target.parent)?;
            }
            for (target, entry) in targets.iter().zip(&manifest.entries) {
                check_target(target, &entry.before)?;
            }
            for (path, expected) in read_guards {
                let current =
                    bytes(&self.directory, path, MAX_FILE)?.ok_or("Copy source disappeared.")?;
                require_hash(expected, &current)?;
            }
            Ok(())
        })();
        if let Err(reason) = staged {
            let mut cleanup_errors = Vec::new();
            for (index, target) in targets.iter().enumerate() {
                for kind in ["old", "new", "probe"] {
                    if let Err(e) = target.parent.remove_file(name(&manifest.id, index, kind)) {
                        if e.kind() != std::io::ErrorKind::NotFound {
                            cleanup_errors.push(io(e));
                        }
                    }
                }
            }
            return Err(format!("Patch preparation failed; no project files were changed: {reason}. Staging cleanup errors: {cleanup_errors:?}"));
        }
        if let Err(reason) = save_manifest(&journal, &manifest, max_manifest) {
            let recorded = journal
                .symlink_metadata(format!("{}.json", manifest.id))
                .is_ok();
            let mut cleanup_errors = Vec::new();
            if !recorded {
                // A failed initial journal write cannot authorize any commit.
                // Remove unjournaled preparation blobs rather than accumulating
                // backups that the recovery API cannot discover.
                for (index, target) in targets.iter().enumerate() {
                    for kind in ["old", "new", "probe"] {
                        if let Err(error) =
                            target.parent.remove_file(name(&manifest.id, index, kind))
                        {
                            if error.kind() != std::io::ErrorKind::NotFound {
                                cleanup_errors.push(io(error));
                            }
                        }
                    }
                }
                if let Err(error) = journal.remove_file(format!("{}.next", manifest.id)) {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        cleanup_errors.push(io(error));
                    }
                }
            }
            return Ok(
                json!({"isError":true,"status":"not_started","message":format!("No project files changed; journal preparation failed: {reason}"),"transactionId":manifest.id,"recoveryRequired":recorded || !cleanup_errors.is_empty(),"cleanupErrors":cleanup_errors}),
            );
        }
        let commit = (|| -> Result<(), String> {
            for (index, (target, entry)) in targets.iter().zip(&manifest.entries).enumerate() {
                check_target(target, &entry.before)?;
                if let Some(after) = &entry.after {
                    let staged =
                        bytes(&target.parent, &name(&manifest.id, index, "new"), MAX_FILE)?
                            .ok_or("Staged file is missing.")?;
                    if hash(&staged) != *after {
                        return Err("Staged file changed during transaction preparation.".into());
                    }
                }
                before_commit(index, target, &manifest);
                // The prepared journal covers the entire spelling transition,
                // including termination between parking and destination creation.
                if let Some(destination) = &target.rename_to {
                    check_target(target, &entry.before)?;
                    let leaf = Path::new(destination)
                        .file_name()
                        .ok_or("Missing rename leaf.")?
                        .to_str()
                        .ok_or("Invalid rename leaf.")?;
                    target
                        .parent
                        .rename(
                            &target.leaf,
                            &target.parent,
                            name(&manifest.id, index, "park"),
                        )
                        .map_err(io)?;
                    sync_dir(&target.parent)?;
                    target
                        .parent
                        .hard_link(name(&manifest.id, index, "new"), &target.parent, leaf)
                        .map_err(io)?;
                    sync_dir(&target.parent)?;
                    continue;
                }
                match (&target.before, &target.after) {
                    (Some(_), Some(_)) => target
                        .parent
                        .rename(
                            name(&manifest.id, index, "new"),
                            &target.parent,
                            &target.leaf,
                        )
                        .map_err(io)?,
                    (None, Some(_)) => target
                        .parent
                        .hard_link(
                            name(&manifest.id, index, "new"),
                            &target.parent,
                            &target.leaf,
                        )
                        .map_err(io)?,
                    (Some(_), None) => target.parent.remove_file(&target.leaf).map_err(io)?,
                    _ => return Err("Empty patch entry.".into()),
                }
                sync_dir(&target.parent)?;
            }
            Ok(())
        })();
        if let Err(reason) = commit {
            let recovery = rollback(&self.directory, &manifest);
            let (status, conflicts) = match recovery {
                Ok(ref conflicts) if conflicts.is_empty() => ("rolled_back", Vec::new()),
                Ok(conflicts) => ("unknown", conflicts),
                Err(error) => ("unknown", vec![error]),
            };
            let cleanup_error = if status == "rolled_back" {
                cleanup(&self.directory, &journal, &manifest).err()
            } else {
                None
            };
            return Ok(
                json!({"isError":true,"status":status,"message":format!("Patch commit failed: {reason}"),"transactionId":manifest.id,"recoveryRequired": status == "unknown" || cleanup_error.is_some(),"conflicts":conflicts,"cleanupError":cleanup_error}),
            );
        }
        manifest.state = "committed".into();
        if let Err(reason) = save_manifest(&journal, &manifest, max_manifest) {
            // Do not report success without a durable commit marker. A prepared
            // or committed marker may remain, depending on the failed step.
            return Ok(
                json!({"isError":true,"status":"unknown","message":format!("Files changed, but durable commit confirmation failed: {reason}"),"transactionId":manifest.id,"recoveryRequired":true}),
            );
        }
        let changes: Vec<_> = targets.iter().zip(&manifest.entries).map(|(target, entry)| {
            let diff = match (
                std::str::from_utf8(target.before.as_deref().unwrap_or_default()),
                std::str::from_utf8(target.after.as_deref().unwrap_or_default()),
            ) {
                (Ok(before), Ok(after)) => Some(unified_diff(&entry.path, before, after)),
                _ => None,
            };
            json!({"path":entry.path,"destination":entry.rename_to,"beforeSha256":entry.before,"sha256":entry.after,"kind":if entry.before.is_none() {"create"} else if entry.after.is_none() {"delete"} else {"edit"},"diff":diff})
        }).collect();
        let cleanup_error = cleanup(&self.directory, &journal, &manifest).err();
        Ok(
            json!({"status":"committed","transactionId":manifest.id,"changes":changes,"recoveryRequired":cleanup_error.is_some(),"cleanupError":cleanup_error,"isolation":"native workspace operations serialized; external writers use optimistic hash checks","directorySync":cfg!(unix)}),
        )
    }

    pub(super) fn patch_call(&self, tool: &str, arguments: Value) -> Result<Value, String> {
        match tool {
            "file_info" => {
                let request: super::PathRequest =
                    serde_json::from_value(arguments).map_err(|e| e.to_string())?;
                valid_path(&request.path, false)?;
                let data = bytes(&self.directory, &request.path, MAX_FILE)?
                    .ok_or("File does not exist.")?;
                Ok(
                    json!({"path":request.path,"bytes":data.len(),"sha256":hash(&data),"text":std::str::from_utf8(&data).is_ok() && !data.contains(&0)}),
                )
            }
            "apply_patch" => self.transact(self.patch_desired(arguments)?, &[]),
            "write_file" => {
                let request: WriteFile =
                    serde_json::from_value(arguments).map_err(|e| e.to_string())?;
                valid_path(&request.path, false)?;
                if request.content.len() > MAX_FILE {
                    return Err("File content exceeds 1 MiB.".into());
                }
                self.transact(
                    vec![Desired {
                        path: request.path,
                        expected: request.expected_sha256,
                        after: Some(request.content.into_bytes()),
                        permissions: None,
                    }],
                    &[],
                )
            }
            "write_file_base64" => {
                let request: WriteBase64 =
                    serde_json::from_value(arguments).map_err(|e| e.to_string())?;
                valid_path(&request.path, false)?;
                use base64::Engine;
                let data = base64::engine::general_purpose::STANDARD
                    .decode(request.content_base64)
                    .map_err(|_| "Invalid base64 content.".to_string())?;
                if data.len() > MAX_FILE {
                    return Err("Decoded content exceeds 1 MiB.".into());
                }
                self.transact(
                    vec![Desired {
                        path: request.path,
                        expected: request.expected_sha256,
                        after: Some(data),
                        permissions: None,
                    }],
                    &[],
                )
            }
            "append_file" => {
                let request: Append =
                    serde_json::from_value(arguments).map_err(|e| e.to_string())?;
                valid_path(&request.path, false)?;
                if request.content.len() > MAX_FILE {
                    return Err("Append content exceeds 1 MiB.".into());
                }
                let mut data = bytes(&self.directory, &request.path, MAX_FILE)?
                    .ok_or("Append target is missing.")?;
                require_hash(&request.expected_sha256, &data)?;
                data.extend_from_slice(request.content.as_bytes());
                if data.len() > MAX_FILE {
                    return Err("Appended file would exceed 1 MiB.".into());
                }
                self.transact(
                    vec![Desired {
                        path: request.path,
                        expected: Some(request.expected_sha256),
                        after: Some(data),
                        permissions: None,
                    }],
                    &[],
                )
            }
            "multi_edit" => {
                let request: MultiEdit =
                    serde_json::from_value(arguments).map_err(|e| e.to_string())?;
                self.transact(
                    vec![self.edit_desired(
                        &request.path,
                        &request.expected_sha256,
                        request.edits,
                        32,
                    )?],
                    &[],
                )
            }
            "edit_lines" => {
                let request: EditLines =
                    serde_json::from_value(arguments).map_err(|e| e.to_string())?;
                self.transact(vec![self.edit_lines_desired(&request)?], &[])
            }
            "list_tree" => self.list_tree(arguments),
            "delete_directory" => self.delete_directory(arguments),
            "copy_directory" | "move_directory" => self.transfer_directory(tool, arguments),
            "delete_file" => {
                let request: Delete =
                    serde_json::from_value(arguments).map_err(|e| e.to_string())?;
                self.transact(
                    vec![Desired {
                        path: request.path,
                        expected: Some(request.expected_sha256),
                        after: None,
                        permissions: None,
                    }],
                    &[],
                )
            }
            "copy_file" | "move_file" => {
                let request: Transfer =
                    serde_json::from_value(arguments).map_err(|e| e.to_string())?;
                unique_paths([request.path.clone(), request.destination.clone()].into_iter())?;
                let data = bytes(&self.directory, &request.path, MAX_FILE)?
                    .ok_or("Source file is missing.")?;
                require_hash(&request.expected_sha256, &data)?;
                let permissions = self
                    .directory
                    .symlink_metadata(&request.path)
                    .map_err(io)?
                    .permissions();
                let guards = vec![(request.path.clone(), request.expected_sha256.clone())];
                let mut desired = vec![Desired {
                    path: request.destination,
                    expected: None,
                    after: Some(data),
                    permissions: Some(permissions),
                }];
                if tool == "move_file" {
                    desired.push(Desired {
                        path: request.path,
                        expected: Some(request.expected_sha256),
                        after: None,
                        permissions: None,
                    });
                }
                self.transact(desired, &guards)
            }
            "recover_patch" => self.recover_patch(arguments),
            _ => Err("Unknown patch tool.".into()),
        }
    }

    fn recover_patch(&self, arguments: Value) -> Result<Value, String> {
        let request: Recover = serde_json::from_value(arguments).map_err(|e| e.to_string())?;
        let Some(id) = request.transaction_id else {
            return Ok(
                json!({"transactions":transaction_ids(&self.directory)?,"message":"Specify transaction_id to recover. Prepared transactions roll back; committed transactions finish cleanup."}),
            );
        };
        if !id_valid(&id) {
            return Err("transaction_id must be 32 lowercase hexadecimal characters.".into());
        }
        let journal = journal(&self.directory, false)?.ok_or("No transaction journal exists.")?;
        let data = bytes(&journal, &format!("{id}.json"), TREE_MANIFEST)?
            .ok_or("Transaction was not found.")?;
        let manifest: Manifest = serde_json::from_slice(&data)
            .map_err(|e| format!("Invalid transaction manifest: {e}"))?;
        if manifest.version != 1
            || manifest.id != id
            || !matches!(manifest.state.as_str(), "prepared" | "committed")
            || manifest.entries.is_empty()
            || manifest.entries.len() > TREE_TARGETS
        {
            return Err("Invalid transaction manifest version, ID, state or entry count.".into());
        }
        unique_paths(manifest.entries.iter().map(|e| e.path.clone()))?;
        for entry in &manifest.entries {
            if let Some(destination) = &entry.rename_to {
                valid_path(destination, false)?;
                if destination == &entry.path
                    || destination.to_lowercase() != entry.path.to_lowercase()
                    || Path::new(destination).parent() != Path::new(&entry.path).parent()
                    || entry.before.is_none()
                    || entry.after.is_none()
                {
                    return Err("Invalid case-only rename journal entry.".into());
                }
            }
            if entry.before.is_none() && entry.after.is_none() {
                return Err("Invalid empty transaction entry.".into());
            }
            for hash in [&entry.before, &entry.after].into_iter().flatten() {
                parse_hash(hash)?;
            }
        }
        if manifest.state == "prepared" {
            let conflicts = rollback(&self.directory, &manifest)?;
            if !conflicts.is_empty() {
                return Ok(
                    json!({"isError":true,"status":"unknown","transactionId":id,"recoveryRequired":true,"conflicts":conflicts,"message":"Recovery preserved divergent files; original backups remain."}),
                );
            }
        }
        let cleanup_error = cleanup(&self.directory, &journal, &manifest).err();
        Ok(
            json!({"isError":cleanup_error.is_some(),"status":if manifest.state == "prepared" {"rolled_back"} else {"cleaned"},"transactionId":id,"recoveryRequired":cleanup_error.is_some(),"cleanupError":cleanup_error}),
        )
    }
}

pub(super) fn definitions() -> Vec<(&'static str, &'static str, Value, Vec<&'static str>)> {
    let path = json!({"type":"string","minLength":1,"maxLength":4096});
    let hash = json!({"type":"string","pattern":"^[a-fA-F0-9]{64}$"});
    let replacement = json!({"type":"object","properties":{"old_text":{"type":"string","minLength":1,"maxLength":65536},"new_text":{"type":"string","maxLength":65536}},"required":["old_text","new_text"],"additionalProperties":false});
    let edit = json!({"type":"object","properties":{"kind":{"const":"edit"},"path":path,"expected_sha256":hash,"replacements":{"type":"array","minItems":1,"maxItems":64,"items":replacement}},"required":["kind","path","expected_sha256","replacements"],"additionalProperties":false});
    let create = json!({"type":"object","properties":{"kind":{"const":"create"},"path":path,"content":{"type":"string","maxLength":1048576}},"required":["kind","path","content"],"additionalProperties":false});
    let delete = json!({"type":"object","properties":{"kind":{"const":"delete"},"path":path,"expected_sha256":hash},"required":["kind","path","expected_sha256"],"additionalProperties":false});
    vec![
        ("apply_patch", "Apply 1–16 hash-pinned file edits/creates/deletes after all-file preflight. Edits use unique, nonoverlapping replacements against original text; smart quotes, dash variants, exotic spaces, trailing whitespace and CRLF are tolerated when matching and the line endings of every replaced region are preserved. Atomic per-file replacement with durable rollback journal; external applications do not share cross-file isolation. Returns diffs and explicit committed/rolled_back/unknown status. Parents must exist; 1 MiB/file, 8 MiB total. Recover an interrupted transaction with recover_patch.", json!({"changes":{"type":"array","minItems":1,"maxItems":16,"items":{"oneOf":[edit,create,delete]}}}), vec!["changes"]),
        ("copy_file", "Copy a regular workspace file to a new destination without overwriting. Requires source SHA-256 from file_info/read_file. Parents must exist. Maximum 1 MiB; returns journaled transaction status.", json!({"path":path,"destination":path,"expected_sha256":hash}), vec!["path","destination","expected_sha256"]),
        ("move_file", "Move a hash-pinned regular workspace file to a new destination using a recoverable copy/delete transaction. Never overwrites a destination. Parents must exist. Maximum 1 MiB.", json!({"path":path,"destination":path,"expected_sha256":hash}), vec!["path","destination","expected_sha256"]),
        ("delete_file", "Delete one regular workspace file after verifying expected_sha256. Uses durable rollback backup until commit. Does not delete directories. Maximum 1 MiB.", json!({"path":path,"expected_sha256":hash}), vec!["path","expected_sha256"]),
        ("recover_patch", "Inspect or recover pending workspace patch transactions. Omit transaction_id to list. Supply the ID to roll back a prepared transaction or clean a committed one. Divergent external edits are preserved and unresolved recovery is reported. This is a write operation subject to normal approval.", json!({"transaction_id":{"type":"string","pattern":"^[a-f0-9]{32}$"}}), vec![]),
        ("file_info", "Read bounded metadata and SHA-256 for a regular workspace text or binary file up to 1 MiB. Use the hash for copy/move/delete or patches. Does not return file contents.", json!({"path":path}), vec!["path"]),
        ("write_file", "Create or overwrite one whole UTF-8 workspace file up to 1 MiB. Overwriting requires expected_sha256 from a fresh read_file/file_info; creating requires omitting it and never overwrites. Journaled transaction returning status and a diff when overwriting.", json!({"path":path,"content":{"type":"string","maxLength":1048576},"expected_sha256":hash}), vec!["path","content"]),
        ("write_file_base64", "Create or overwrite one workspace file with base64-decoded binary content, up to 1 MiB decoded. Same existence and expected_sha256 rules as write_file. Journaled transaction returning status.", json!({"path":path,"content_base64":{"type":"string","maxLength":1400000},"expected_sha256":hash}), vec!["path","content_base64"]),
        ("append_file", "Append UTF-8 text to an existing workspace file after verifying expected_sha256. The resulting file must stay under 1 MiB. Journaled transaction returning status and a diff.", json!({"path":path,"content":{"type":"string","maxLength":1048576},"expected_sha256":hash}), vec!["path","content","expected_sha256"]),
        ("multi_edit", "Apply 1–32 unique nonoverlapping text replacements to one existing UTF-8 workspace file pinned by expected_sha256. Each old_text must occur exactly once in the original file. Smart quotes, dash variants, exotic spaces, trailing whitespace and CRLF are tolerated when matching, and the line endings of every replaced region are preserved. Journaled transaction returning status and a diff.", json!({"path":path,"expected_sha256":hash,"edits":{"type":"array","minItems":1,"maxItems":32,"items":replacement}}), vec!["path","expected_sha256","edits"]),
        ("edit_lines", "Replace an inclusive 1-based line range in an existing UTF-8 workspace file pinned by expected_sha256. Use end_line = start_line - 1 to insert before start_line; start_line may be totalLines + 1 to append. The file's line ending style is preserved. Maximum file size 1 MiB.", json!({"path":path,"expected_sha256":hash,"start_line":{"type":"integer","minimum":1},"end_line":{"type":"integer","minimum":0},"new_text":{"type":"string","maxLength":1048576}}), vec!["path","expected_sha256","start_line","end_line","new_text"]),
        ("list_tree", "List a workspace directory recursively down to max_depth (1–10, default 3), skipping generated directories unless include_generated. Returns up to 2000 entries with path, kind, size and file SHA-256 plus treeSha256 over the full tree, the pin delete/move/copy_directory require.", json!({"path":path,"max_depth":{"type":"integer","minimum":1,"maximum":10,"default":3},"include_generated":{"type":"boolean","default":false}}), vec!["path"]),
        ("delete_directory", "Delete a workspace directory. Non-recursive removes only an empty directory. Recursive requires expected_tree_sha256 from list_tree for that path, refuses linked or special entries, and is capped at 2000 entries / 8 MiB. File contents are journaled for rollback before emptied directories are removed bottom-up.", json!({"path":path,"recursive":{"type":"boolean"},"expected_tree_sha256":hash}), vec!["path","recursive"]),
        ("move_directory", "Move a workspace directory to a new destination whose parent exists. Never overwrites; requires expected_tree_sha256 from list_tree; refuses linked or special entries; capped at 2000 entries / 8 MiB. Journaled file moves, then emptied source directories are removed.", json!({"path":path,"destination":path,"expected_tree_sha256":hash}), vec!["path","destination","expected_tree_sha256"]),
        ("copy_directory", "Copy a workspace directory tree to a new destination whose parent exists. Never overwrites; requires expected_tree_sha256 from list_tree; refuses linked or special entries; capped at 2000 entries / 8 MiB. Journaled file creates.", json!({"path":path,"destination":path,"expected_tree_sha256":hash}), vec!["path","destination","expected_tree_sha256"]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(root: &Path) -> Workspace {
        Workspace::open(root.to_str().unwrap()).unwrap()
    }
    fn edit(path: &str, before: &str, old: &str, new: &str) -> Value {
        json!({"kind":"edit","path":path,"expected_sha256":hash(before.as_bytes()),"replacements":[{"old_text":old,"new_text":new}]})
    }
    fn desired(path: &str, before: Option<&str>, after: Option<&str>) -> Desired {
        Desired {
            path: path.into(),
            expected: before.map(|s| hash(s.as_bytes())),
            after: after.map(|s| s.as_bytes().to_vec()),
            permissions: None,
        }
    }

    #[test]
    fn batch_preflight_preserves_every_original_on_late_conflict_and_rejects_ambiguous_edits() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.txt"), "alpha").unwrap();
        std::fs::write(root.path().join("b.txt"), "beta").unwrap();
        let w = workspace(root.path());
        let result = w.call("apply_patch", json!({"changes":[edit("a.txt","alpha","alpha","updated"),edit("b.txt","stale","stale","x")]}));
        assert!(result.is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("a.txt")).unwrap(),
            "alpha"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("b.txt")).unwrap(),
            "beta"
        );
        assert!(transaction_ids(&w.directory).unwrap().is_empty());
        for content in ["aaa", "ααα"] {
            std::fs::write(root.path().join("a.txt"), content).unwrap();
            let old = if content.is_ascii() { "aa" } else { "αα" };
            assert!(w
                .call(
                    "apply_patch",
                    json!({"changes":[edit("a.txt",content,old,"x")]})
                )
                .is_err());
        }
        std::fs::write(root.path().join("a.txt"), "abcdef").unwrap();
        assert!(w.call("apply_patch", json!({"changes":[{"kind":"edit","path":"a.txt","expected_sha256":hash(b"abcdef"),"replacements":[{"old_text":"abc","new_text":"x"},{"old_text":"cde","new_text":"y"}]}]})).is_err());
        assert!(w.call("apply_patch", json!({"changes":[edit("a.txt","abcdef","abc","x"),edit("a.txt","abcdef","def","y")]})).is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("a.txt")).unwrap(),
            "abcdef"
        );
    }

    #[test]
    fn multi_file_patch_preserves_unicode_crlf_permissions_and_returns_diffs() {
        let root = tempfile::tempdir().unwrap();
        let before = "α one\r\nβ two\r\n";
        std::fs::write(root.path().join("a.txt"), before).unwrap();
        std::fs::write(root.path().join("obsolete.txt"), "old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(
                root.path().join("a.txt"),
                std::fs::Permissions::from_mode(0o750),
            )
            .unwrap();
        }
        let w = workspace(root.path());
        let result = w.call("apply_patch", json!({"changes":[
            {"kind":"edit","path":"a.txt","expected_sha256":hash(before.as_bytes()),"replacements":[{"old_text":"α one","new_text":"α first"},{"old_text":"β two","new_text":"β second"}]},
            {"kind":"create","path":"new.txt","content":"new 🌍\r\n"},
            {"kind":"delete","path":"obsolete.txt","expected_sha256":hash(b"old")}
        ]})).unwrap();
        assert_eq!(result["status"], "committed", "{result}");
        assert_eq!(result["recoveryRequired"], false);
        assert_eq!(result["changes"].as_array().unwrap().len(), 3);
        assert!(result["changes"][0]["diff"]
            .as_str()
            .unwrap()
            .contains("α first"));
        assert_eq!(
            std::fs::read(root.path().join("a.txt")).unwrap(),
            "α first\r\nβ second\r\n".as_bytes()
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("new.txt")).unwrap(),
            "new 🌍\r\n"
        );
        assert!(!root.path().join("obsolete.txt").exists());
        assert!(transaction_ids(&w.directory).unwrap().is_empty());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(root.path().join("a.txt"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o750
            );
        }
        // Large multibyte diffs must truncate at a UTF-8 boundary.
        assert!(unified_diff("unicode.txt", &"🦀".repeat(10_000), "replacement").len() < 17_000);
    }

    #[test]
    fn actual_commit_syscall_failure_restores_earlier_changes() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.txt"), "before a").unwrap();
        std::fs::write(root.path().join("b.txt"), "before b").unwrap();
        let w = workspace(root.path());
        let result = w
            .transact_with_commit_hook(
                vec![
                    desired("a.txt", Some("before a"), Some("after a")),
                    desired("b.txt", Some("before b"), Some("after b")),
                ],
                &[],
                |index, target, manifest| {
                    if index == 1 {
                        // Remove the verified second stage immediately before rename.
                        // The actual OS rename fails, after a.txt has been replaced.
                        target
                            .parent
                            .remove_file(name(&manifest.id, index, "new"))
                            .unwrap();
                    }
                },
            )
            .unwrap();
        assert_eq!(result["status"], "rolled_back", "{result}");
        assert_eq!(result["isError"], true);
        assert_eq!(result["recoveryRequired"], false);
        for (path, expected) in [("a.txt", "before a"), ("b.txt", "before b")] {
            assert_eq!(
                std::fs::read_to_string(root.path().join(path)).unwrap(),
                expected
            );
        }
        assert!(transaction_ids(&w.directory).unwrap().is_empty());
    }

    #[test]
    fn case_rename_commit_preflight_and_interrupted_recovery() {
        for phase in ["commit", "prepared", "parked", "installed", "external"] {
            let root = tempfile::tempdir().unwrap();
            std::fs::write(root.path().join("Name.rs"), "before").unwrap();
            let w = workspace(root.path());
            let changes = || {
                vec![
                    desired("Name.rs", Some("before"), None),
                    desired("name.rs", None, Some("after")),
                ]
            };
            if phase == "commit" {
                let result = w.transact(changes(), &[]).unwrap();
                assert_eq!(result["status"], "committed", "{result}");
                let names = std::fs::read_dir(root.path())
                    .unwrap()
                    .map(|e| e.unwrap().file_name())
                    .collect::<Vec<_>>();
                assert!(names.contains(&"name.rs".into()));
                assert!(!names.contains(&"Name.rs".into()));
                assert_eq!(
                    std::fs::read_to_string(root.path().join("name.rs")).unwrap(),
                    "after"
                );
                let reversed = w
                    .transact(
                        vec![
                            desired("name.rs", Some("after"), None),
                            desired("Name.rs", None, Some("restored")),
                        ],
                        &[],
                    )
                    .unwrap();
                assert_eq!(reversed["status"], "committed", "{reversed}");
                assert_eq!(
                    std::fs::read_to_string(root.path().join("Name.rs")).unwrap(),
                    "restored"
                );
                continue;
            }
            let (targets, entries) = preflight(&w.directory, changes(), 16, MAX_BATCH).unwrap();
            assert_eq!(entries.len(), 1);
            let manifest = Manifest {
                version: 1,
                id: uuid::Uuid::new_v4().simple().to_string(),
                state: "prepared".into(),
                entries,
            };
            let target = &targets[0];
            write_new(
                &target.parent,
                &name(&manifest.id, 0, "old"),
                b"before",
                target.permissions.clone(),
            )
            .unwrap();
            write_new(
                &target.parent,
                &name(&manifest.id, 0, "new"),
                b"after",
                target.permissions.clone(),
            )
            .unwrap();
            let journal = journal(&w.directory, true).unwrap().unwrap();
            save_manifest(&journal, &manifest, MAX_MANIFEST).unwrap();
            if phase != "prepared" {
                target
                    .parent
                    .rename("Name.rs", &target.parent, name(&manifest.id, 0, "park"))
                    .unwrap();
            }
            if matches!(phase, "installed" | "external") {
                target
                    .parent
                    .rename(name(&manifest.id, 0, "new"), &target.parent, "name.rs")
                    .unwrap();
            }
            if phase == "external" {
                std::fs::write(root.path().join("name.rs"), "external").unwrap();
            }
            drop(w);
            let reopened = workspace(root.path());
            let result = reopened
                .recover_patch(json!({"transaction_id":manifest.id}))
                .unwrap();
            if phase == "external" {
                assert_eq!(result["status"], "unknown");
                assert_eq!(
                    std::fs::read_to_string(root.path().join("name.rs")).unwrap(),
                    "external"
                );
                // Resolve the simulated external conflict to exercise repeated recovery.
                std::fs::write(root.path().join("name.rs"), "after").unwrap();
                assert_eq!(
                    reopened
                        .recover_patch(json!({"transaction_id":manifest.id}))
                        .unwrap()["status"],
                    "rolled_back"
                );
            } else {
                assert_eq!(result["status"], "rolled_back", "{result}");
            }
            assert_eq!(
                std::fs::read_to_string(root.path().join("Name.rs")).unwrap(),
                "before"
            );
        }
    }
    #[test]
    fn case_rename_stale_collision_and_commit_failure_preserve_sources() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("Name.rs"), "before").unwrap();
        std::fs::write(root.path().join("other.rs"), "other").unwrap();
        let w = workspace(root.path());
        assert!(w
            .transact(
                vec![
                    desired("Name.rs", Some("stale"), None),
                    desired("name.rs", None, Some("after"))
                ],
                &[]
            )
            .is_err());
        if !root.path().join("name.rs").exists() {
            std::fs::write(root.path().join("name.rs"), "distinct").unwrap();
            assert!(w
                .transact(
                    vec![
                        desired("Name.rs", Some("before"), None),
                        desired("name.rs", None, Some("after"))
                    ],
                    &[]
                )
                .is_err());
            assert_eq!(
                std::fs::read_to_string(root.path().join("name.rs")).unwrap(),
                "distinct"
            );
            std::fs::remove_file(root.path().join("name.rs")).unwrap();
        }
        let result = w
            .transact_with_commit_hook(
                vec![
                    desired("Name.rs", Some("before"), None),
                    desired("name.rs", None, Some("after")),
                    desired("other.rs", Some("other"), Some("changed")),
                ],
                &[],
                |index, target, manifest| {
                    if index == 1 {
                        target.parent.remove_file(&target.leaf).unwrap();
                        target
                            .parent
                            .remove_file(name(&manifest.id, index, "new"))
                            .unwrap();
                    }
                },
            )
            .unwrap();
        assert_eq!(
            result["status"], "unknown",
            "External deletion is preserved: {result}"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("Name.rs")).unwrap(),
            "before"
        );
    }

    // Construct the durable on-disk state left by termination after the first
    // file replacement. Recovery runs through a freshly opened Workspace.
    fn interrupted(root: &Path, state: &str) -> String {
        std::fs::write(root.join("a.txt"), "before a").unwrap();
        std::fs::write(root.join("b.txt"), "before b").unwrap();
        let w = workspace(root);
        let (targets, entries) = preflight(
            &w.directory,
            vec![
                desired("a.txt", Some("before a"), Some("after a")),
                desired("b.txt", Some("before b"), Some("after b")),
            ],
            16,
            MAX_BATCH,
        )
        .unwrap();
        let manifest = Manifest {
            version: 1,
            id: uuid::Uuid::new_v4().simple().to_string(),
            state: state.into(),
            entries,
        };
        for (index, target) in targets.iter().enumerate() {
            write_new(
                &target.parent,
                &name(&manifest.id, index, "old"),
                target.before.as_ref().unwrap(),
                target.permissions.clone(),
            )
            .unwrap();
            write_new(
                &target.parent,
                &name(&manifest.id, index, "new"),
                target.after.as_ref().unwrap(),
                target.permissions.clone(),
            )
            .unwrap();
            sync_dir(&target.parent).unwrap();
        }
        let journal = journal(&w.directory, true).unwrap().unwrap();
        save_manifest(&journal, &manifest, MAX_MANIFEST).unwrap();
        let count = if state == "committed" { 2 } else { 1 };
        for (index, target) in targets.iter().enumerate().take(count) {
            target
                .parent
                .rename(
                    name(&manifest.id, index, "new"),
                    &target.parent,
                    &target.leaf,
                )
                .unwrap();
        }
        manifest.id
    }

    #[test]
    fn prepared_recovery_is_explicit_restart_safe_and_committed_recovery_only_cleans() {
        for state in ["prepared", "committed"] {
            let root = tempfile::tempdir().unwrap();
            let id = interrupted(root.path(), state);
            let w = workspace(root.path());
            assert!(w
                .call("read_file", json!({"path":"a.txt"}))
                .unwrap_err()
                .contains("recover_patch"));
            assert!(w.read_script("a.txt").is_err());
            assert!(w.output_artifact("a.txt").is_err());
            assert_eq!(
                std::fs::read_to_string(root.path().join("a.txt")).unwrap(),
                "after a"
            );
            let listed = w.call("recover_patch", json!({})).unwrap();
            assert_eq!(listed["transactions"], json!([id.clone()]));
            let result = w
                .call("recover_patch", json!({"transaction_id":id}))
                .unwrap();
            assert_eq!(result["recoveryRequired"], false, "{result}");
            for (path, expected) in if state == "prepared" {
                [("a.txt", "before a"), ("b.txt", "before b")]
            } else {
                [("a.txt", "after a"), ("b.txt", "after b")]
            } {
                assert_eq!(
                    std::fs::read_to_string(root.path().join(path)).unwrap(),
                    expected
                );
            }
            assert!(w.call("read_file", json!({"path":"a.txt"})).is_ok());
            assert!(transaction_ids(&w.directory).unwrap().is_empty());
        }
    }

    #[test]
    fn recovery_preserves_divergent_edits_and_validates_all_backups_before_mutation() {
        let root = tempfile::tempdir().unwrap();
        let id = interrupted(root.path(), "prepared");
        let w = workspace(root.path());
        std::fs::write(root.path().join("a.txt"), "user's concurrent edit").unwrap();
        let result = w
            .call("recover_patch", json!({"transaction_id":id}))
            .unwrap();
        assert_eq!(result["status"], "unknown", "{result}");
        assert_eq!(result["recoveryRequired"], true);
        assert_eq!(
            std::fs::read_to_string(root.path().join("a.txt")).unwrap(),
            "user's concurrent edit"
        );
        assert!(root.path().join(name(&id, 0, "old")).exists());
        // Even a later backup's corruption must be caught before rolling back a.txt.
        std::fs::write(root.path().join("a.txt"), "after a").unwrap();
        std::fs::write(root.path().join(name(&id, 1, "old")), "tampered").unwrap();
        assert!(w
            .call("recover_patch", json!({"transaction_id":id}))
            .unwrap_err()
            .contains("corrupt"));
        assert_eq!(
            std::fs::read_to_string(root.path().join("a.txt")).unwrap(),
            "after a"
        );
        let journal = root.path().join(JOURNAL).join(format!("{id}.json"));
        let mut manifest: Value =
            serde_json::from_slice(&std::fs::read(&journal).unwrap()).unwrap();
        manifest["entries"][0]["path"] = json!("../escape.txt");
        std::fs::write(&journal, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(w
            .call("recover_patch", json!({"transaction_id":id}))
            .is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("a.txt")).unwrap(),
            "after a"
        );
    }

    #[test]
    fn recovery_can_finish_after_interrupted_rollback_cleanup() {
        let root = tempfile::tempdir().unwrap();
        let id = interrupted(root.path(), "prepared");
        let w = workspace(root.path());
        std::fs::write(root.path().join("a.txt"), "before a").unwrap();
        std::fs::remove_file(root.path().join(name(&id, 0, "old"))).unwrap();
        let result = w
            .call("recover_patch", json!({"transaction_id":id}))
            .unwrap();
        assert_eq!(result["status"], "rolled_back", "{result}");
        assert_eq!(result["recoveryRequired"], false);
    }

    #[test]
    fn interrupted_creations_and_deletions_recover_without_clobbering_new_user_content() {
        for conflict in [false, true] {
            let root = tempfile::tempdir().unwrap();
            std::fs::write(root.path().join("old.txt"), "old content").unwrap();
            let w = workspace(root.path());
            let id = uuid::Uuid::new_v4().simple().to_string();
            let (targets, entries) = preflight(
                &w.directory,
                vec![
                    desired("new.txt", None, Some("created")),
                    desired("old.txt", Some("old content"), None),
                ],
                16,
                MAX_BATCH,
            )
            .unwrap();
            let manifest = Manifest {
                version: 1,
                id: id.clone(),
                state: "prepared".into(),
                entries,
            };
            write_new(&targets[0].parent, &name(&id, 0, "new"), b"created", None).unwrap();
            write_new(
                &targets[1].parent,
                &name(&id, 1, "old"),
                b"old content",
                None,
            )
            .unwrap();
            let journal = journal(&w.directory, true).unwrap().unwrap();
            save_manifest(&journal, &manifest, MAX_MANIFEST).unwrap();
            targets[0]
                .parent
                .hard_link(name(&id, 0, "new"), &targets[0].parent, &targets[0].leaf)
                .unwrap();
            targets[1].parent.remove_file(&targets[1].leaf).unwrap();
            if conflict {
                std::fs::write(root.path().join("new.txt"), "user content").unwrap();
            }
            let result = w
                .call("recover_patch", json!({"transaction_id":id}))
                .unwrap();
            assert_eq!(
                std::fs::read_to_string(root.path().join("old.txt")).unwrap(),
                "old content"
            );
            if conflict {
                assert_eq!(result["status"], "unknown", "{result}");
                assert_eq!(
                    std::fs::read_to_string(root.path().join("new.txt")).unwrap(),
                    "user content"
                );
            } else {
                assert_eq!(result["status"], "rolled_back", "{result}");
                assert!(!root.path().join("new.txt").exists());
            }
        }
    }

    #[test]
    fn copy_move_delete_info_support_binary_files_and_never_overwrite() {
        let root = tempfile::tempdir().unwrap();
        let data = b"binary\0\xff";
        std::fs::write(root.path().join("source.bin"), data).unwrap();
        let w = workspace(root.path());
        let info = w.call("file_info", json!({"path":"source.bin"})).unwrap();
        assert_eq!(info["text"], false);
        assert_eq!(info["sha256"], hash(data));
        let transfer =
            json!({"path":"source.bin","destination":"copy.bin","expected_sha256":info["sha256"]});
        assert_eq!(
            w.call("copy_file", transfer.clone()).unwrap()["status"],
            "committed"
        );
        assert!(w.call("copy_file", transfer).is_err());
        assert_eq!(w.call("move_file",json!({"path":"copy.bin","destination":"moved.bin","expected_sha256":info["sha256"]})).unwrap()["status"],"committed");
        assert!(!root.path().join("copy.bin").exists());
        assert_eq!(std::fs::read(root.path().join("source.bin")).unwrap(), data);
        assert_eq!(std::fs::read(root.path().join("moved.bin")).unwrap(), data);
        assert!(w
            .call(
                "delete_file",
                json!({"path":"moved.bin","expected_sha256":"0".repeat(64)})
            )
            .is_err());
        assert_eq!(
            w.call(
                "delete_file",
                json!({"path":"moved.bin","expected_sha256":info["sha256"]})
            )
            .unwrap()["status"],
            "committed"
        );
        assert!(!root.path().join("moved.bin").exists());
    }

    #[test]
    fn containment_internal_paths_bounds_and_links_are_enforced() {
        let root = tempfile::tempdir().unwrap();
        let w = workspace(root.path());
        for path in [
            "../escape.txt",
            ".env",
            "nested/.ssh/key",
            ".locallm-transactions/forged.json",
            ".locallm-txn-forged-old",
            "missing/child.txt",
        ] {
            assert!(
                w.call(
                    "apply_patch",
                    json!({"changes":[{"kind":"create","path":path,"content":"never"}]})
                )
                .is_err(),
                "{path}"
            );
        }
        assert!(w.call("apply_patch", json!({"changes":[]})).is_err());
        assert!(w.call("apply_patch",json!({"changes":[{"kind":"create","path":"large.txt","content":"x".repeat(MAX_FILE+1)}]})).is_err());
        assert!(!root.path().join("large.txt").exists());
        std::fs::write(root.path().join("too-big.bin"), vec![0; MAX_FILE + 1]).unwrap();
        assert!(w.call("file_info", json!({"path":"too-big.bin"})).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let outside = tempfile::tempdir().unwrap();
            std::fs::write(outside.path().join("outside.txt"), "untouched").unwrap();
            symlink(outside.path(), root.path().join("linked")).unwrap();
            assert!(w.call("apply_patch",json!({"changes":[edit("linked/outside.txt","untouched","untouched","changed")]})).is_err());
            std::fs::hard_link(
                outside.path().join("outside.txt"),
                root.path().join("hard.txt"),
            )
            .unwrap();
            assert!(w
                .call(
                    "delete_file",
                    json!({"path":"hard.txt","expected_sha256":hash(b"untouched")})
                )
                .is_err());
            assert_eq!(
                std::fs::read_to_string(outside.path().join("outside.txt")).unwrap(),
                "untouched"
            );
        }
        assert!(w
            .call("recover_patch", json!({"transaction_id":"../escape"}))
            .is_err());
    }

    #[test]
    fn workspace_instances_share_the_root_lock() {
        let root = tempfile::tempdir().unwrap();
        let a = workspace(root.path());
        let b = workspace(root.path());
        assert!(Arc::ptr_eq(&a.transaction_lock, &b.transaction_lock));
        let other = tempfile::tempdir().unwrap();
        assert!(!Arc::ptr_eq(
            &a.transaction_lock,
            &workspace(other.path()).transaction_lock
        ));
        let guard = a.transaction_lock.write().unwrap();
        let (started, ready) = std::sync::mpsc::channel();
        let (done, completed) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            started.send(()).unwrap();
            done.send(b.call("list_files", json!({"path":"."})))
                .unwrap();
        });
        ready
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(completed
            .recv_timeout(std::time::Duration::from_millis(30))
            .is_err());
        drop(guard);
        assert!(completed
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap()
            .is_ok());
        thread.join().unwrap();
    }

    #[tokio::test]
    async fn model_schema_dispatch_source_and_approval_boundaries_are_preserved() {
        let root = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(root.path());
        crate::test_support::select_fixture_model(&state, root.path());
        let plan = crate::chat::assemble_turn(
            &state,
            "",
            "Patch files",
            vec!["__workspace".into()],
            Some(vec![]),
            Some(crate::presets::STANDARD),
            None,
        )
        .await
        .unwrap();
        for name in [
            "apply_patch",
            "copy_file",
            "move_file",
            "delete_file",
            "recover_patch",
            "file_info",
            "write_file",
            "write_file_base64",
            "append_file",
            "multi_edit",
            "edit_lines",
            "list_tree",
            "delete_directory",
            "copy_directory",
            "move_directory",
            "notebook_read",
            "notebook_edit",
            "view_image",
        ] {
            let tool = plan
                .tools
                .iter()
                .find(|t| t.alias == format!("workspace_{name}"))
                .unwrap();
            assert_eq!(
                tool.trusted_read(),
                matches!(
                    name,
                    "file_info" | "list_tree" | "notebook_read" | "view_image"
                )
            );
            assert_eq!(
                tool.definition()["function"]["parameters"]["additionalProperties"],
                false
            );
            for access in [
                crate::permissions::AccessMode::Ask,
                crate::permissions::AccessMode::AutoApprove,
                crate::permissions::AccessMode::FullAccess,
            ] {
                assert_eq!(
                    access.automatic_reason(tool.trusted_read()).is_some()
                        && !tool.requires_approval(&json!({})),
                    access == crate::permissions::AccessMode::FullAccess
                        || (access == crate::permissions::AccessMode::AutoApprove
                            && matches!(
                                name,
                                "file_info" | "list_tree" | "notebook_read" | "view_image"
                            ))
                );
            }
        }
        let args = json!({"changes":[{"kind":"create","path":"model.txt","content":"model dispatch fixture"}]});
        let mut calls = crate::tool_calls::ToolCalls::default();
        calls.push(&json!({"tool_calls":[{"index":0,"id":"patch-fixture","type":"function","function":{"name":"workspace_apply_patch","arguments":args.to_string()}}]})).unwrap();
        let call = calls.finish().unwrap().remove(0);
        let tool = plan.tools.iter().find(|t| t.alias == call.name).unwrap();
        let result = tool
            .call(tool.prepare_arguments(call.arguments).unwrap())
            .await
            .unwrap();
        assert_eq!(result["status"], "committed", "{result}");
        assert_eq!(
            std::fs::read_to_string(root.path().join("model.txt")).unwrap(),
            "model dispatch fixture"
        );
        for invalid in [
            json!({"changes":[],"extra":true}),
            json!({"changes":[{"kind":"delete","path":"model.txt"}]}),
            json!({"changes":[{"kind":"create","path":"a","content":1}]}),
        ] {
            assert!(tool.prepare_arguments(invalid).is_err());
        }
        state
            .database()
            .unwrap()
            .save_setting(crate::capabilities::DISABLED_KEY, &vec!["workspace_files"])
            .unwrap();
        let policy = crate::capabilities::ToolPolicy::guard(state.database());
        assert!(
            policy.denial_for_tool(tool).is_some(),
            "A stale patch tool remains subject to the common dispatch capability gate"
        );
        let disabled = crate::chat::assemble_turn(
            &state,
            "",
            "Patch files",
            vec!["__workspace".into()],
            Some(vec![]),
            Some(crate::presets::STANDARD),
            None,
        )
        .await
        .unwrap();
        assert!(!disabled.tools.iter().any(|t| t.connector == "Workspace"));
        let unselected = crate::chat::assemble_turn(
            &state,
            "",
            "Patch files",
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
            .any(|t| t.alias == "workspace_apply_patch"));
    }

    fn file_sha(w: &Workspace, path: &str) -> String {
        w.call("file_info", json!({"path":path})).unwrap()["sha256"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn write_file_creates_and_overwrites_only_with_the_current_hash() {
        let root = tempfile::tempdir().unwrap();
        let w = workspace(root.path());
        // Creating requires an omitted hash; supplying one fails on a missing file.
        assert!(w
            .call(
                "write_file",
                json!({"path":"a.txt","content":"one","expected_sha256":"0".repeat(64)})
            )
            .is_err());
        let created = w
            .call("write_file", json!({"path":"a.txt","content":"one"}))
            .unwrap();
        assert_eq!(created["status"], "committed", "{created}");
        // Existing files refuse a missing or stale hash and return a diff.
        assert!(w
            .call("write_file", json!({"path":"a.txt","content":"two"}))
            .is_err());
        assert!(w
            .call(
                "write_file",
                json!({"path":"a.txt","content":"two","expected_sha256":"0".repeat(64)})
            )
            .is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("a.txt")).unwrap(),
            "one"
        );
        let overwritten = w
            .call(
                "write_file",
                json!({"path":"a.txt","content":"two","expected_sha256":file_sha(&w,"a.txt")}),
            )
            .unwrap();
        assert_eq!(overwritten["status"], "committed", "{overwritten}");
        assert!(overwritten["changes"][0]["diff"]
            .as_str()
            .unwrap()
            .contains("+two"));
        assert_eq!(
            std::fs::read_to_string(root.path().join("a.txt")).unwrap(),
            "two"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            std::fs::write(root.path().join("real.txt"), "real").unwrap();
            symlink("real.txt", root.path().join("link.txt")).unwrap();
            assert!(w
                .call(
                    "write_file",
                    json!({"path":"link.txt","content":"x","expected_sha256":hash(b"real")})
                )
                .is_err());
            assert_eq!(
                std::fs::read_to_string(root.path().join("real.txt")).unwrap(),
                "real"
            );
        }
    }

    #[test]
    fn write_file_base64_round_trips_binary_and_pins_overwrites() {
        use base64::Engine;
        let root = tempfile::tempdir().unwrap();
        let w = workspace(root.path());
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"\x00\xffbinary");
        assert!(w
            .call(
                "write_file_base64",
                json!({"path":"b.bin","content_base64":"not base64!!"})
            )
            .is_err());
        let created = w
            .call(
                "write_file_base64",
                json!({"path":"b.bin","content_base64":encoded}),
            )
            .unwrap();
        assert_eq!(created["status"], "committed", "{created}");
        assert_eq!(
            std::fs::read(root.path().join("b.bin")).unwrap(),
            b"\x00\xffbinary"
        );
        assert!(w
            .call(
                "write_file_base64",
                json!({"path":"b.bin","content_base64":encoded})
            )
            .is_err());
        assert!(w
            .call(
                "write_file_base64",
                json!({"path":"b.bin","content_base64":encoded,"expected_sha256":"0".repeat(64)})
            )
            .is_err());
        let updated = base64::engine::general_purpose::STANDARD.encode(b"updated\x01");
        assert_eq!(w.call("write_file_base64",json!({"path":"b.bin","content_base64":updated,"expected_sha256":file_sha(&w,"b.bin")})).unwrap()["status"],"committed");
        assert_eq!(
            std::fs::read(root.path().join("b.bin")).unwrap(),
            b"updated\x01"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            std::fs::write(root.path().join("real.bin"), "real").unwrap();
            symlink("real.bin", root.path().join("link.bin")).unwrap();
            assert!(w.call("write_file_base64",json!({"path":"link.bin","content_base64":encoded,"expected_sha256":hash(b"real")})).is_err());
            assert_eq!(
                std::fs::read(root.path().join("real.bin")).unwrap(),
                b"real"
            );
        }
    }

    #[test]
    fn append_file_requires_an_existing_file_and_a_fresh_hash() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("log.txt"), "first\n").unwrap();
        let w = workspace(root.path());
        let sha = file_sha(&w, "log.txt");
        assert!(w
            .call(
                "append_file",
                json!({"path":"log.txt","content":"x","expected_sha256":"0".repeat(64)})
            )
            .is_err());
        assert!(w
            .call(
                "append_file",
                json!({"path":"missing.txt","content":"x","expected_sha256":sha})
            )
            .is_err());
        let appended = w
            .call(
                "append_file",
                json!({"path":"log.txt","content":"second\n","expected_sha256":sha}),
            )
            .unwrap();
        assert_eq!(appended["status"], "committed", "{appended}");
        assert!(appended["changes"][0]["diff"]
            .as_str()
            .unwrap()
            .contains("+second"));
        assert_eq!(
            std::fs::read_to_string(root.path().join("log.txt")).unwrap(),
            "first\nsecond\n"
        );
        // The resulting file must stay under the 1 MiB bound.
        std::fs::write(root.path().join("full.txt"), vec![b'a'; MAX_FILE]).unwrap();
        assert!(w
            .call(
                "append_file",
                json!({"path":"full.txt","content":"b","expected_sha256":file_sha(&w,"full.txt")})
            )
            .is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            std::fs::write(root.path().join("real.txt"), "real").unwrap();
            symlink("real.txt", root.path().join("link.txt")).unwrap();
            assert!(w
                .call(
                    "append_file",
                    json!({"path":"link.txt","content":"x","expected_sha256":hash(b"real")})
                )
                .is_err());
            assert_eq!(
                std::fs::read_to_string(root.path().join("real.txt")).unwrap(),
                "real"
            );
        }
    }

    #[test]
    fn multi_edit_reuses_patch_replacement_rules_on_one_file() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("m.txt"), "alpha beta gamma").unwrap();
        let w = workspace(root.path());
        let sha = file_sha(&w, "m.txt");
        // Same rules as apply_patch edits: nonempty, unique, nonoverlapping.
        for edits in [
            json!([]),
            json!([{"old_text":"a","new_text":"x"}]),
            json!([{"old_text":"alpha beta","new_text":"x"},{"old_text":"beta gamma","new_text":"y"}]),
            json!([{"old_text":"alpha","new_text":"alpha"}]),
            json!([{"old_text":"absent","new_text":"x"}]),
        ] {
            assert!(
                w.call(
                    "multi_edit",
                    json!({"path":"m.txt","expected_sha256":sha,"edits":edits.clone()})
                )
                .is_err(),
                "{edits}"
            );
        }
        // The 1–32 bound applies before any match lookup.
        let many: Vec<Value> = (0..33)
            .map(|index| json!({"old_text":format!("w{index}"),"new_text":"x"}))
            .collect();
        std::fs::write(
            root.path().join("many.txt"),
            (0..33)
                .map(|index| format!("w{index}"))
                .collect::<Vec<_>>()
                .join(" "),
        )
        .unwrap();
        assert!(w
            .call(
                "multi_edit",
                json!({"path":"many.txt","expected_sha256":file_sha(&w,"many.txt"),"edits":many})
            )
            .is_err());
        assert!(w.call("multi_edit",json!({"path":"m.txt","expected_sha256":"0".repeat(64),"edits":[{"old_text":"beta","new_text":"B"}]})).is_err());
        let edited = w.call("multi_edit",json!({"path":"m.txt","expected_sha256":sha,"edits":[{"old_text":"alpha","new_text":"ALPHA"},{"old_text":"gamma","new_text":"GAMMA"}]})).unwrap();
        assert_eq!(edited["status"], "committed", "{edited}");
        assert_eq!(
            std::fs::read_to_string(root.path().join("m.txt")).unwrap(),
            "ALPHA beta GAMMA"
        );
        assert!(w.call("multi_edit",json!({"path":"m.txt","expected_sha256":sha,"edits":[{"old_text":"beta","new_text":"B"}]})).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink("m.txt", root.path().join("link.txt")).unwrap();
            assert!(w.call("multi_edit",json!({"path":"link.txt","expected_sha256":file_sha(&w,"m.txt"),"edits":[{"old_text":"beta","new_text":"B"}]})).is_err());
            assert_eq!(
                std::fs::read_to_string(root.path().join("m.txt")).unwrap(),
                "ALPHA beta GAMMA"
            );
        }
    }

    #[test]
    fn multi_edit_tolerates_typographic_differences_and_keeps_line_endings() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("t.txt"),
            "let s = \u{201C}hi\u{201D};\r\nlet n = 1;\r\n",
        )
        .unwrap();
        let w = workspace(root.path());
        let sha = file_sha(&w, "t.txt");
        let edited = w
            .call(
                "multi_edit",
                json!({"path":"t.txt","expected_sha256":sha,"edits":[{"old_text":"\"hi\"","new_text":"\"bye\""},{"old_text":"let n = 1;","new_text":"let n = 2;"}]}),
            )
            .unwrap();
        assert_eq!(edited["status"], "committed", "{edited}");
        assert_eq!(
            std::fs::read_to_string(root.path().join("t.txt")).unwrap(),
            "let s = \"bye\";\r\nlet n = 2;\r\n"
        );
    }

    #[test]
    fn edit_lines_replaces_inserts_appends_and_preserves_line_endings() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("l.txt"), "one\ntwo\nthree\n").unwrap();
        let w = workspace(root.path());
        let sha = file_sha(&w, "l.txt");
        let replaced = w.call("edit_lines",json!({"path":"l.txt","expected_sha256":sha,"start_line":2,"end_line":3,"new_text":"TWO\nTHREE"})).unwrap();
        assert_eq!(replaced["status"], "committed", "{replaced}");
        assert_eq!(
            std::fs::read_to_string(root.path().join("l.txt")).unwrap(),
            "one\nTWO\nTHREE\n"
        );
        assert!(w.call("edit_lines",json!({"path":"l.txt","expected_sha256":sha,"start_line":1,"end_line":1,"new_text":"x"})).is_err());
        // Insert before line 1 and append one past the last line.
        let sha = file_sha(&w, "l.txt");
        w.call("edit_lines",json!({"path":"l.txt","expected_sha256":sha,"start_line":1,"end_line":0,"new_text":"zero"})).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("l.txt")).unwrap(),
            "zero\none\nTWO\nTHREE\n"
        );
        let sha = file_sha(&w, "l.txt");
        w.call("edit_lines",json!({"path":"l.txt","expected_sha256":sha,"start_line":5,"end_line":4,"new_text":"four"})).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("l.txt")).unwrap(),
            "zero\none\nTWO\nTHREE\nfour\n"
        );
        let sha = file_sha(&w, "l.txt");
        for (start, end) in [(0usize, 0usize), (2, 0), (7, 6), (1, 99)] {
            assert!(w.call("edit_lines",json!({"path":"l.txt","expected_sha256":sha,"start_line":start,"end_line":end,"new_text":"x"})).is_err(),"{start}..{end}");
        }
        // Deleting a range passes empty new_text.
        w.call(
            "edit_lines",
            json!({"path":"l.txt","expected_sha256":sha,"start_line":1,"end_line":1,"new_text":""}),
        )
        .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.path().join("l.txt")).unwrap(),
            "one\nTWO\nTHREE\nfour\n"
        );
        // A CRLF file keeps its own line endings in the replacement text.
        std::fs::write(root.path().join("crlf.txt"), "a\r\nb\r\n").unwrap();
        let crlf = w.call("edit_lines",json!({"path":"crlf.txt","expected_sha256":file_sha(&w,"crlf.txt"),"start_line":1,"end_line":1,"new_text":"x\ny"})).unwrap();
        assert_eq!(crlf["status"], "committed", "{crlf}");
        assert_eq!(
            std::fs::read(root.path().join("crlf.txt")).unwrap(),
            b"x\r\ny\r\nb\r\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink("l.txt", root.path().join("link.txt")).unwrap();
            assert!(w.call("edit_lines",json!({"path":"link.txt","expected_sha256":file_sha(&w,"l.txt"),"start_line":1,"end_line":1,"new_text":"x"})).is_err());
        }
    }

    #[test]
    fn list_tree_reports_kinds_hashes_depth_and_generated_policy() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src/d1/d2/d3/d4")).unwrap();
        std::fs::write(root.path().join("src/a.txt"), "alpha").unwrap();
        std::fs::write(root.path().join("src/d1/d2/d3/d4/deep.rs"), "deep").unwrap();
        std::fs::create_dir_all(root.path().join("src/node_modules/pkg")).unwrap();
        std::fs::write(root.path().join("src/node_modules/pkg/x.js"), "x").unwrap();
        let w = workspace(root.path());
        let listed = w.call("list_tree", json!({"path":"src"})).unwrap();
        assert_eq!(listed["truncated"], false);
        let entries = listed["entries"].as_array().unwrap();
        let by_path = |path: &str| entries.iter().find(|entry| entry["path"] == path).cloned();
        assert_eq!(by_path("src/a.txt").unwrap()["kind"], "file");
        assert_eq!(by_path("src/a.txt").unwrap()["sha256"], hash(b"alpha"));
        assert_eq!(by_path("src/d1").unwrap()["kind"], "directory");
        assert!(by_path("src/d1").unwrap().get("sha256").is_none());
        // Past max_depth, the file still counts toward the full-tree hash.
        assert!(by_path("src/d1/d2/d3/d4/deep.rs").is_none());
        assert!(listed["totalEntries"].as_u64().unwrap() > entries.len() as u64);
        assert!(by_path("src/node_modules").is_none());
        let deep = w
            .call(
                "list_tree",
                json!({"path":"src","max_depth":10,"include_generated":true}),
            )
            .unwrap();
        let paths: Vec<&str> = deep["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["path"].as_str().unwrap())
            .collect();
        assert!(
            paths.contains(&"src/d1/d2/d3/d4/deep.rs")
                && paths.contains(&"src/node_modules/pkg/x.js")
        );
        assert_ne!(listed["treeSha256"], deep["treeSha256"]);
        // The hash pins the tree: any content change invalidates it.
        let sha_before = listed["treeSha256"].as_str().unwrap().to_string();
        assert_eq!(
            w.call("list_tree", json!({"path":"src"})).unwrap()["treeSha256"],
            sha_before
        );
        std::fs::write(root.path().join("src/a.txt"), "changed").unwrap();
        assert_ne!(
            w.call("list_tree", json!({"path":"src"})).unwrap()["treeSha256"],
            sha_before
        );
        assert!(w
            .call("list_tree", json!({"path":"src","max_depth":0}))
            .is_err());
        assert!(w
            .call("list_tree", json!({"path":"src","max_depth":11}))
            .is_err());
        assert!(w.call("list_tree", json!({"path":"src/a.txt"})).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink("a.txt", root.path().join("src/linked.txt")).unwrap();
            let with_link = w
                .call("list_tree", json!({"path":"src","max_depth":1}))
                .unwrap();
            assert_eq!(
                with_link["entries"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|entry| entry["path"] == "src/linked.txt")
                    .unwrap()["kind"],
                "link"
            );
            assert!(w
                .call("list_tree", json!({"path":"src/linked.txt"}))
                .is_err());
        }
    }

    #[test]
    fn delete_directory_bounds_paths_and_pins_the_whole_tree() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("tree/sub/deep")).unwrap();
        std::fs::write(root.path().join("tree/a.txt"), "a").unwrap();
        std::fs::write(root.path().join("tree/sub/b.txt"), "b").unwrap();
        std::fs::write(root.path().join("tree/sub/deep/c.txt"), "c").unwrap();
        let w = workspace(root.path());
        assert!(w
            .call("delete_directory", json!({"path":"tree","recursive":false}))
            .is_err());
        assert!(w
            .call("delete_directory", json!({"path":"tree","recursive":true}))
            .is_err());
        assert!(w
            .call(
                "delete_directory",
                json!({"path":"tree","recursive":true,"expected_tree_sha256":"0".repeat(64)})
            )
            .is_err());
        assert!(w
            .call(
                "delete_directory",
                json!({"path":".","recursive":true,"expected_tree_sha256":"0".repeat(64)})
            )
            .is_err());
        assert!(w
            .call(
                "delete_directory",
                json!({"path":"tree/a.txt","recursive":false})
            )
            .is_err());
        let listed = w.call("list_tree", json!({"path":"tree"})).unwrap();
        let deleted = w
            .call(
                "delete_directory",
                json!({"path":"tree","recursive":true,"expected_tree_sha256":listed["treeSha256"]}),
            )
            .unwrap();
        assert_eq!(deleted["status"], "committed", "{deleted}");
        assert_eq!(
            deleted["remainingDirectories"].as_array().unwrap().len(),
            0,
            "{deleted}"
        );
        assert!(!root.path().join("tree").exists());
        assert!(transaction_ids(&w.directory).unwrap().is_empty());
        // A file too large for the per-file bound rejects the transaction.
        std::fs::create_dir(root.path().join("big")).unwrap();
        std::fs::write(root.path().join("big/blob.bin"), vec![0; MAX_FILE + 1]).unwrap();
        let listed = w.call("list_tree", json!({"path":"big"})).unwrap();
        assert!(w
            .call(
                "delete_directory",
                json!({"path":"big","recursive":true,"expected_tree_sha256":listed["treeSha256"]})
            )
            .is_err());
        assert_eq!(
            std::fs::read(root.path().join("big/blob.bin"))
                .unwrap()
                .len(),
            MAX_FILE + 1
        );
        // Non-recursive deletes only an empty directory.
        std::fs::create_dir(root.path().join("empty")).unwrap();
        assert_eq!(
            w.call(
                "delete_directory",
                json!({"path":"empty","recursive":false})
            )
            .unwrap()["deleted"],
            true
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            std::fs::create_dir_all(root.path().join("victim/sub")).unwrap();
            std::fs::write(root.path().join("victim/keep.txt"), "keep").unwrap();
            std::fs::write(root.path().join("outside.txt"), "outside").unwrap();
            symlink("../outside.txt", root.path().join("victim/sub/link.txt")).unwrap();
            let listed = w.call("list_tree", json!({"path":"victim"})).unwrap();
            assert!(w.call("delete_directory",json!({"path":"victim","recursive":true,"expected_tree_sha256":listed["treeSha256"]})).is_err());
            assert_eq!(
                std::fs::read_to_string(root.path().join("victim/keep.txt")).unwrap(),
                "keep"
            );
            assert_eq!(
                std::fs::read_to_string(root.path().join("outside.txt")).unwrap(),
                "outside"
            );
        }
    }

    #[test]
    fn delete_directory_preserves_generated_content_it_never_listed() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("tree/node_modules/pkg")).unwrap();
        std::fs::write(root.path().join("tree/a.txt"), "a").unwrap();
        std::fs::write(root.path().join("tree/node_modules/pkg/keep.js"), "keep").unwrap();
        let w = workspace(root.path());
        // The default listing skips generated dirs; that pinned view is what
        // the delete covers, so the untouched content is reported, not lost.
        let listed = w.call("list_tree", json!({"path":"tree"})).unwrap();
        let deleted = w
            .call(
                "delete_directory",
                json!({"path":"tree","recursive":true,"expected_tree_sha256":listed["treeSha256"]}),
            )
            .unwrap();
        assert_eq!(deleted["status"], "committed", "{deleted}");
        assert!(!root.path().join("tree/a.txt").exists());
        assert_eq!(
            std::fs::read_to_string(root.path().join("tree/node_modules/pkg/keep.js")).unwrap(),
            "keep"
        );
        let remaining = deleted["remainingDirectories"].as_array().unwrap();
        assert_eq!(remaining, &vec![json!("tree")]);
    }

    #[test]
    fn copy_and_move_directory_journal_the_whole_tree() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("src/sub/empty_sub")).unwrap();
        std::fs::write(root.path().join("src/a.txt"), "a").unwrap();
        std::fs::write(root.path().join("src/sub/b.txt"), "b").unwrap();
        let w = workspace(root.path());
        let sha = w.call("list_tree", json!({"path":"src"})).unwrap()["treeSha256"]
            .as_str()
            .unwrap()
            .to_string();
        for bad in [
            json!({"path":"src","destination":"dst","expected_tree_sha256":"0".repeat(64)}),
            json!({"path":"src","destination":"src/inner","expected_tree_sha256":sha.clone()}),
            json!({"path":"src","destination":"missing-parent/dst","expected_tree_sha256":sha.clone()}),
        ] {
            assert!(w.call("copy_directory", bad.clone()).is_err(), "{bad}");
            assert!(w.call("move_directory", bad).is_err());
        }
        let copied = w
            .call(
                "copy_directory",
                json!({"path":"src","destination":"dst","expected_tree_sha256":sha}),
            )
            .unwrap();
        assert_eq!(copied["status"], "committed", "{copied}");
        assert_eq!(
            std::fs::read_to_string(root.path().join("dst/a.txt")).unwrap(),
            "a"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("dst/sub/b.txt")).unwrap(),
            "b"
        );
        assert!(root.path().join("dst/sub/empty_sub").is_dir());
        assert!(root.path().join("src/a.txt").exists());
        // The destination is never overwritten, even by a second copy.
        assert!(w
            .call(
                "copy_directory",
                json!({"path":"src","destination":"dst","expected_tree_sha256":sha})
            )
            .is_err());
        let moved = w
            .call(
                "move_directory",
                json!({"path":"src","destination":"moved","expected_tree_sha256":sha}),
            )
            .unwrap();
        assert_eq!(moved["status"], "committed", "{moved}");
        assert_eq!(
            moved["remainingDirectories"].as_array().unwrap().len(),
            0,
            "{moved}"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("moved/a.txt")).unwrap(),
            "a"
        );
        assert_eq!(
            std::fs::read_to_string(root.path().join("moved/sub/b.txt")).unwrap(),
            "b"
        );
        assert!(root.path().join("moved/sub/empty_sub").is_dir());
        assert!(!root.path().join("src").exists());
        assert!(transaction_ids(&w.directory).unwrap().is_empty());
        // A file over the per-file bound rejects before any destination appears.
        std::fs::create_dir(root.path().join("big")).unwrap();
        std::fs::write(root.path().join("big/blob.bin"), vec![0; MAX_FILE + 1]).unwrap();
        let big = w.call("list_tree", json!({"path":"big"})).unwrap()["treeSha256"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(w
            .call(
                "move_directory",
                json!({"path":"big","destination":"big-moved","expected_tree_sha256":big})
            )
            .is_err());
        assert!(!root.path().join("big-moved").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            std::fs::create_dir_all(root.path().join("victim/sub")).unwrap();
            std::fs::write(root.path().join("victim/keep.txt"), "keep").unwrap();
            std::fs::write(root.path().join("outside.txt"), "outside").unwrap();
            symlink("../../outside.txt", root.path().join("victim/sub/link.txt")).unwrap();
            let victim = w.call("list_tree", json!({"path":"victim"})).unwrap()["treeSha256"]
                .as_str()
                .unwrap()
                .to_string();
            assert!(w
                .call(
                    "copy_directory",
                    json!({"path":"victim","destination":"v-copy","expected_tree_sha256":victim})
                )
                .is_err());
            assert!(w
                .call(
                    "move_directory",
                    json!({"path":"victim","destination":"v-moved","expected_tree_sha256":victim})
                )
                .is_err());
            assert!(!root.path().join("v-copy").exists());
            assert!(!root.path().join("v-moved").exists());
            assert_eq!(
                std::fs::read_to_string(root.path().join("victim/keep.txt")).unwrap(),
                "keep"
            );
        }
    }
}
