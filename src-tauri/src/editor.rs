//! Direct, user-initiated editor operations. These are deliberately separate
//! from the assistant's file tools and their credential-file policy.
use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::Path,
    sync::Mutex,
};

const MAX_BYTES: usize = 2 * 1024 * 1024;
// Serialize editor mutations, including the hash check and atomic replacement.
static MUTATIONS: Mutex<()> = Mutex::new(());

#[cfg(windows)]
fn rename_no_replace(dir: &Dir, from: &str, to: &str) -> Result<(), String> {
    use std::os::windows::io::AsRawHandle;
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::HANDLE,
            Storage::FileSystem::{
                GetFinalPathNameByHandleW, MoveFileExW, FILE_NAME_NORMALIZED, MOVE_FILE_FLAGS,
            },
        },
    };
    // cap-std pins Windows parent directories without FILE_SHARE_DELETE.
    // Resolve that pinned handle, then use MoveFileEx without REPLACE_EXISTING.
    let handle = HANDLE(dir.as_raw_handle());
    let mut buffer = vec![0u16; 32768];
    let length =
        unsafe { GetFinalPathNameByHandleW(handle, &mut buffer, FILE_NAME_NORMALIZED) } as usize;
    if length == 0 || length >= buffer.len() {
        return Err(format!(
            "Cannot resolve rename folder: {}",
            std::io::Error::last_os_error()
        ));
    }
    buffer.truncate(length);
    let name = |leaf: &str| {
        let mut path = buffer.clone();
        if path.last() != Some(&(b'\\' as u16)) {
            path.push(b'\\' as u16);
        }
        path.extend(leaf.encode_utf16());
        path.push(0);
        path
    };
    let source = name(from);
    let target = name(to);
    unsafe {
        MoveFileExW(
            PCWSTR(source.as_ptr()),
            PCWSTR(target.as_ptr()),
            MOVE_FILE_FLAGS(0),
        )
    }
    .map_err(|e| format!("Cannot rename; the destination may already exist: {e}"))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    content: String,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum Mutation {
    CreateFile {
        path: String,
    },
    CreateDirectory {
        path: String,
    },
    Rename {
        path: String,
        name: String,
    },
    Delete {
        path: String,
        directory: bool,
        recursive: bool,
    },
}

pub fn validate_path(path: &str) -> Result<(), String> {
    if path.split('/').any(|part| {
        part.eq_ignore_ascii_case(".locallm-transactions")
            || part.to_lowercase().starts_with(".locallm-txn-")
    }) {
        return Err(
            "Internal transaction files must be managed through workspace recovery.".into(),
        );
    }
    if path.is_empty()
        || path.len() > 4096
        || Path::new(path).is_absolute()
        || path.contains(['\\', ':', '*', '?', '"', '<', '>', '|'])
        || path.split('/').any(|part| {
            let stem = part
                .split('.')
                .next()
                .unwrap_or_default()
                .to_ascii_uppercase();
            part.is_empty()
                || part == "."
                || part == ".."
                || part.ends_with(['.', ' '])
                || part.chars().any(char::is_control)
                || matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                || (stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && stem.as_bytes()[3].is_ascii_digit())
        })
    {
        return Err(
            "Use a relative workspace path without parent traversal or reserved characters.".into(),
        );
    }
    Ok(())
}

struct Editor {
    dir: Dir,
    transaction_lock: std::sync::Arc<std::sync::RwLock<()>>,
}
impl Editor {
    fn open(root: &str) -> Result<Self, String> {
        if !Path::new(root).is_absolute() {
            return Err("Choose a workspace folder first.".into());
        }
        Ok(Self {
            dir: Dir::open_ambient_dir(root, ambient_authority())
                .map_err(|e| format!("Cannot open workspace: {e}"))?,
            transaction_lock: crate::workspace::patch::root_lock(Path::new(root))?,
        })
    }

    // Open a capability for the parent and operate on a single basename.
    // Reject links at every level; cap-std also confines concurrent resolution.
    fn parent(&self, path: &str) -> Result<(Dir, String), String> {
        validate_path(path)?;
        let mut dir = self.dir.try_clone().map_err(|e| e.to_string())?;
        let mut parts = path.split('/').peekable();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                return Ok((dir, part.into()));
            }
            let meta = dir
                .symlink_metadata(part)
                .map_err(|e| format!("Cannot open parent folder: {e}"))?;
            if meta.is_symlink() || !meta.is_dir() {
                return Err(
                    "Linked paths cannot be edited. Open the actual workspace folder.".into(),
                );
            }
            dir = dir.open_dir(part).map_err(|e| e.to_string())?;
        }
        Err("Select a file or folder inside the workspace.".into())
    }

    fn bytes(dir: &Dir, name: &str) -> Result<Vec<u8>, String> {
        let meta = dir
            .symlink_metadata(name)
            .map_err(|e| format!("Cannot read file: {e}"))?;
        if meta.is_symlink() || !meta.is_file() {
            return Err("The editor supports regular files, not links or folders.".into());
        }
        if meta.len() > MAX_BYTES as u64 {
            return Err("The editor supports UTF-8 text files up to 2 MiB.".into());
        }
        let mut bytes = Vec::new();
        dir.open(name)
            .map_err(|e| e.to_string())?
            .take((MAX_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > MAX_BYTES || bytes.contains(&0) {
            return Err("This file is binary or exceeds the 2 MiB editor limit.".into());
        }
        Ok(bytes)
    }

    fn read(&self, path: &str) -> Result<Document, String> {
        let _transaction = self
            .transaction_lock
            .read()
            .map_err(|_| "Workspace transaction lock unavailable.")?;
        crate::workspace::patch::ensure_clear(&self.dir)?;
        let (dir, name) = self.parent(path)?;
        let bytes = Self::bytes(&dir, &name)?;
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        let content = String::from_utf8(bytes)
            .map_err(|_| "This file is not UTF-8 text. Open it in an external editor.")?;
        Ok(Document { content, sha256 })
    }

    fn save(&self, path: &str, content: &str, expected: &str) -> Result<Document, String> {
        let _transaction = self
            .transaction_lock
            .write()
            .map_err(|_| "Workspace transaction lock unavailable.")?;
        crate::workspace::patch::ensure_clear(&self.dir)?;
        if content.len() > MAX_BYTES || content.contains('\0') {
            return Err("The editor supports UTF-8 text files up to 2 MiB.".into());
        }
        let _guard = MUTATIONS
            .lock()
            .map_err(|_| "Editor operation unavailable.")?;
        let (dir, name) = self.parent(path)?;
        let check = || -> Result<(), String> {
            let bytes = Self::bytes(&dir, &name)?;
            if format!("{:x}", Sha256::digest(bytes)) != expected {
                return Err("This file changed on disk. Your edits are preserved. Reload the file to review the current version before saving.".into());
            }
            Ok(())
        };
        check()?;
        let permissions = dir
            .metadata(&name)
            .map_err(|e| e.to_string())?
            .permissions();
        if permissions.readonly() {
            return Err("This file is read-only. Change its permissions before saving.".into());
        }
        let temp = format!(".locallm-save-{}", uuid::Uuid::new_v4());
        let result = (|| {
            let mut file = dir
                .open_with(&temp, OpenOptions::new().write(true).create_new(true))
                .map_err(|e| e.to_string())?;
            file.write_all(content.as_bytes())
                .map_err(|e| format!("Save failed: {e}"))?;
            file.set_permissions(permissions)
                .map_err(|e| e.to_string())?;
            file.sync_all().map_err(|e| e.to_string())?;
            drop(file);
            check()?;
            dir.rename(&temp, &dir, &name)
                .map_err(|e| format!("Cannot replace file: {e}"))?;
            crate::workspace::patch::sync_dir(&dir)?;
            Ok(Document {
                content: content.into(),
                sha256: format!("{:x}", Sha256::digest(content.as_bytes())),
            })
        })();
        if result.is_err() {
            let _ = dir.remove_file(&temp);
        }
        result
    }

    fn mutate(&self, mutation: Mutation) -> Result<String, String> {
        let _transaction = self
            .transaction_lock
            .write()
            .map_err(|_| "Workspace transaction lock unavailable.")?;
        crate::workspace::patch::ensure_clear(&self.dir)?;
        let _guard = MUTATIONS
            .lock()
            .map_err(|_| "Editor operation unavailable.")?;
        let path = match &mutation {
            Mutation::CreateFile { path }
            | Mutation::CreateDirectory { path }
            | Mutation::Rename { path, .. }
            | Mutation::Delete { path, .. } => path,
        };
        let (dir, name) = self.parent(path)?;
        match &mutation {
            Mutation::CreateFile { .. } => {
                dir.open_with(&name, OpenOptions::new().write(true).create_new(true))
                    .map_err(|e| format!("Cannot create file; existing files are preserved: {e}"))?
                    .sync_all()
                    .map_err(|e| e.to_string())?;
            }
            Mutation::CreateDirectory { .. } => dir
                .create_dir(&name)
                .map_err(|e| format!("Cannot create folder: {e}"))?,
            Mutation::Rename { name: new_name, .. } => {
                validate_path(new_name)?;
                if new_name.contains('/') {
                    return Err("Enter a name, without a folder path.".into());
                }
                if dir
                    .symlink_metadata(&name)
                    .map_err(|e| e.to_string())?
                    .is_symlink()
                {
                    return Err("Linked files and folders cannot be renamed in the editor.".into());
                }
                // NOREPLACE atomically preserves existing destinations on Unix.
                // Windows uses MoveFileEx without REPLACE_EXISTING.
                #[cfg(any(target_os = "linux", target_os = "macos"))]
                rustix::fs::renameat_with(
                    &dir,
                    &name,
                    &dir,
                    new_name,
                    rustix::fs::RenameFlags::NOREPLACE,
                )
                .map_err(|e| format!("Cannot rename; the destination may already exist: {e}"))?;
                #[cfg(windows)]
                rename_no_replace(&dir, &name, new_name)?;
                #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
                {
                    match dir.symlink_metadata(new_name) {
                        Ok(_) => {
                            return Err("A file or folder with that name already exists.".into())
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                        Err(e) => return Err(e.to_string()),
                    }
                    dir.rename(&name, &dir, new_name)
                        .map_err(|e| format!("Cannot rename: {e}"))?;
                }
                crate::workspace::patch::sync_dir(&dir)?;
                let parent = path
                    .rsplit_once('/')
                    .map(|(p, _)| format!("{p}/"))
                    .unwrap_or_default();
                return Ok(format!("{parent}{new_name}"));
            }
            Mutation::Delete {
                directory,
                recursive,
                ..
            } => {
                let meta = dir.symlink_metadata(&name).map_err(|e| e.to_string())?;
                if meta.is_symlink() {
                    return Err("Linked files and folders cannot be deleted in the editor.".into());
                }
                if meta.is_dir() != *directory {
                    return Err(
                        "This entry changed type. Refresh the explorer and try again.".into(),
                    );
                }
                if *directory {
                    if *recursive {
                        dir.remove_dir_all(&name)
                    } else {
                        dir.remove_dir(&name)
                    }
                } else {
                    dir.remove_file(&name)
                }
                .map_err(|e| format!("Cannot delete: {e}"))?;
            }
        }
        crate::workspace::patch::sync_dir(&dir)?;
        Ok(path.clone())
    }
}

pub fn bound_root(state: &crate::AppState, requested: &str) -> Result<String, String> {
    crate::workspace_ui::bound_root(state, requested)
}

#[tauri::command]
pub async fn editor_read(
    state: tauri::State<'_, crate::AppState>,
    root: String,
    path: String,
) -> Result<Document, String> {
    let root = bound_root(&state, &root)?;
    tauri::async_runtime::spawn_blocking(move || Editor::open(&root)?.read(&path))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn editor_save(
    state: tauri::State<'_, crate::AppState>,
    root: String,
    path: String,
    content: String,
    expected_sha256: String,
) -> Result<Document, String> {
    let root = bound_root(&state, &root)?;
    tauri::async_runtime::spawn_blocking(move || {
        Editor::open(&root)?.save(&path, &content, &expected_sha256)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn editor_mutate(
    state: tauri::State<'_, crate::AppState>,
    root: String,
    mutation: Mutation,
) -> Result<String, String> {
    let root = bound_root(&state, &root)?;
    tauri::async_runtime::spawn_blocking(move || Editor::open(&root)?.mutate(mutation))
        .await
        .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pending_workspace_transactions_block_editor_reads_and_mutations_without_auto_recovery() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("a.txt"), "before").unwrap();
        let editor = Editor::open(root.path().to_str().unwrap()).unwrap();
        let doc = editor.read("a.txt").unwrap();
        let journal = root.path().join(".locallm-transactions");
        std::fs::create_dir(&journal).unwrap();
        std::fs::write(
            journal.join(format!("{}.json", "a".repeat(32))),
            "invalid manifest requires inspection",
        )
        .unwrap();
        assert!(editor
            .read("a.txt")
            .err()
            .unwrap()
            .contains("recover_patch"));
        assert!(editor.save("a.txt", "changed", &doc.sha256).is_err());
        assert!(editor
            .mutate(Mutation::Delete {
                path: "a.txt".into(),
                directory: false,
                recursive: false
            })
            .is_err());
        assert_eq!(
            std::fs::read_to_string(root.path().join("a.txt")).unwrap(),
            "before"
        );
        for path in [".locallm-transactions", "sub/.locallm-txn-forged-old"] {
            assert!(validate_path(path).is_err());
        }
        assert!(
            validate_path(".env").is_ok(),
            "Direct user editor access to ordinary credential files retains its separate policy"
        );
    }
    #[test]
    fn edits_full_documents_preserving_bytes_and_rejects_stale_saves() {
        let temp = tempfile::tempdir().unwrap();
        let editor = Editor::open(temp.path().to_str().unwrap()).unwrap();
        let original = format!("\u{feff}{}", "hello\r\n".repeat(900));
        std::fs::write(temp.path().join("code.txt"), &original).unwrap();
        let read = editor.read("code.txt").unwrap();
        assert_eq!(read.content, original);
        let updated = original.replace("hello", "world");
        let saved = editor.save("code.txt", &updated, &read.sha256).unwrap();
        assert_eq!(
            std::fs::read_to_string(temp.path().join("code.txt")).unwrap(),
            updated
        );
        assert!(editor.save("code.txt", "stale", &read.sha256).is_err());
        std::fs::write(temp.path().join("code.txt"), "external edit").unwrap();
        assert!(editor.save("code.txt", "my edit", &saved.sha256).is_err());
        assert_eq!(editor.read("code.txt").unwrap().content, "external edit");
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 1);
    }
    #[test]
    fn file_and_folder_lifecycle_never_clobbers_an_existing_destination() {
        let temp = tempfile::tempdir().unwrap();
        let editor = Editor::open(temp.path().to_str().unwrap()).unwrap();
        editor
            .mutate(Mutation::CreateDirectory { path: "src".into() })
            .unwrap();
        editor
            .mutate(Mutation::CreateFile {
                path: "src/new.rs".into(),
            })
            .unwrap();
        assert!(editor
            .mutate(Mutation::CreateFile {
                path: "src/new.rs".into()
            })
            .is_err());
        editor
            .mutate(Mutation::CreateFile {
                path: "src/other.rs".into(),
            })
            .unwrap();
        assert!(editor
            .mutate(Mutation::Rename {
                path: "src/new.rs".into(),
                name: "other.rs".into()
            })
            .is_err());
        assert_eq!(
            editor
                .mutate(Mutation::Rename {
                    path: "src/new.rs".into(),
                    name: "main.rs".into()
                })
                .unwrap(),
            "src/main.rs"
        );
        assert!(editor
            .mutate(Mutation::Delete {
                path: "src".into(),
                directory: true,
                recursive: false
            })
            .is_err());
        editor
            .mutate(Mutation::Rename {
                path: "src".into(),
                name: "source".into(),
            })
            .unwrap();
        editor
            .mutate(Mutation::Delete {
                path: "source".into(),
                directory: true,
                recursive: true,
            })
            .unwrap();
        assert!(!temp.path().join("source").exists());
    }
    #[test]
    fn rejects_root_traversal_binary_and_oversize_files() {
        for path in [
            "",
            ".",
            "..",
            "../outside",
            "/etc/passwd",
            "C:/file",
            "a\\b",
            "a/../b",
            "a//b",
            "CON.txt",
            "a\0b",
        ] {
            assert!(validate_path(path).is_err(), "{path}");
        }
        let temp = tempfile::tempdir().unwrap();
        let editor = Editor::open(temp.path().to_str().unwrap()).unwrap();
        std::fs::write(temp.path().join("binary"), b"hello\0world").unwrap();
        std::fs::write(temp.path().join("large"), vec![b'x'; MAX_BYTES + 1]).unwrap();
        assert!(editor.read("binary").is_err());
        assert!(editor.read("large").is_err());
        // A user may edit .env; this does not expand the model's file tools.
        editor
            .mutate(Mutation::CreateFile {
                path: ".env".into(),
            })
            .unwrap();
        assert_eq!(editor.read(".env").unwrap().content, "");
    }
    #[cfg(unix)]
    #[test]
    fn rejects_linked_paths_and_preserves_executable_permissions() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret"), "untouched").unwrap();
        symlink(outside.path(), temp.path().join("link")).unwrap();
        symlink(outside.path().join("secret"), temp.path().join("file-link")).unwrap();
        let editor = Editor::open(temp.path().to_str().unwrap()).unwrap();
        assert!(editor.read("link/secret").is_err());
        assert!(editor.save("file-link", "oops", "").is_err());
        assert!(editor
            .mutate(Mutation::Delete {
                path: "link".into(),
                directory: true,
                recursive: true
            })
            .is_err());
        std::fs::create_dir(temp.path().join("container")).unwrap();
        symlink(outside.path(), temp.path().join("container/nested-link")).unwrap();
        editor
            .mutate(Mutation::Delete {
                path: "container".into(),
                directory: true,
                recursive: true,
            })
            .unwrap();
        assert!(outside.path().join("secret").exists());
        std::fs::write(temp.path().join("script"), "old").unwrap();
        std::fs::set_permissions(
            temp.path().join("script"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        let read = editor.read("script").unwrap();
        editor.save("script", "new", &read.sha256).unwrap();
        assert_eq!(
            std::fs::metadata(temp.path().join("script"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        assert_eq!(
            std::fs::read_to_string(outside.path().join("secret")).unwrap(),
            "untouched"
        );
    }
}
