//! Read a pinned Git tree through cat-file into an owned directory. No checkout,
//! worktree registration, index, branch, hooks, filters or user files are changed.
use serde_json::{json, Value};
use std::collections::BTreeMap;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
pub(super) struct Baseline {
    pub directory: tempfile::TempDir,
    pub commit: String,
    pub files: BTreeMap<String, String>,
    pub warnings: Vec<Value>,
}
async fn git(root: &str, args: &[&str], cap: usize) -> Result<Vec<u8>, String> {
    let mut command = tokio::process::Command::new("git");
    command
        .args(["-C", root])
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut child = crate::execution::managed_spawn(command).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    child
        .stdout()
        .take()
        .ok_or("Missing Git output.")?
        .take(cap as u64 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    if bytes.len() > cap {
        return Err("Git baseline output exceeds budget.".into());
    }
    if !child.wait().await.map_err(|e| e.to_string())?.success() {
        return Err("Git baseline is unavailable.".into());
    }
    Ok(bytes)
}
pub(super) async fn prepare(root: &str) -> Result<Baseline, String> {
    tokio::time::timeout(std::time::Duration::from_secs(30), async {
        if !git(root, &["rev-parse", "--show-prefix"], 4096)
            .await?
            .iter()
            .all(u8::is_ascii_whitespace)
        {
            return Err(
                "Baseline currently requires the selected workspace to be the Git root.".into(),
            );
        }
        let commit =
            String::from_utf8(git(root, &["rev-parse", "--verify", "HEAD^{commit}"], 128).await?)
                .map_err(|e| e.to_string())?
                .trim()
                .to_string();
        if ![40, 64].contains(&commit.len()) || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("Invalid Git commit identity.".into());
        }
        let tree = git(
            root,
            &["ls-tree", "-rz", "--full-tree", &commit],
            2 * 1024 * 1024,
        )
        .await?;
        let directory = tempfile::Builder::new()
            .prefix("locallm-impact-baseline-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        let mut command = tokio::process::Command::new("git");
        command
            .args(["-C", root, "cat-file", "--batch"])
            .env("GIT_OPTIONAL_LOCKS", "0")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let mut child = crate::execution::managed_spawn(command).map_err(|e| e.to_string())?;
        let mut input = child.stdin().take().ok_or("Missing Git input.")?;
        let mut output =
            tokio::io::BufReader::new(child.stdout().take().ok_or("Missing Git output.")?);
        let mut files = BTreeMap::new();
        let mut warnings = Vec::new();
        let mut total = 0usize;
        for record in tree.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let record = std::str::from_utf8(record).map_err(|_| "Baseline has non-UTF8 paths.")?;
            let (header, path) = record.split_once('\t').ok_or("Malformed Git tree.")?;
            if !crate::source_inventory::relevant(path) {
                continue;
            }
            crate::sandbox::check_fs_path("", path, false)?;
            if std::path::Path::new(path).is_absolute()
                || path
                    .split('/')
                    .any(|p| p.is_empty() || p == "." || p == "..")
            {
                return Err("Invalid baseline path.".into());
            }
            let mut header = header.split_whitespace();
            let mode = header.next();
            let kind = header.next();
            let oid = header.next().ok_or("Missing Git object ID.")?;
            if !matches!(mode, Some("100644" | "100755")) || kind != Some("blob") {
                warnings.push(json!({"path":path,"reason":"baseline_link_or_submodule_excluded"}));
                continue;
            }
            if files.len() >= 4096 {
                return Err("Baseline exceeds 4096 source files.".into());
            }
            input
                .write_all(format!("{oid}\n").as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            let mut reply = Vec::new();
            (&mut output)
                .take(256)
                .read_until(b'\n', &mut reply)
                .await
                .map_err(|e| e.to_string())?;
            let reply = std::str::from_utf8(&reply).map_err(|_| "Invalid Git object header.")?;
            let parts = reply.split_whitespace().collect::<Vec<_>>();
            if parts.len() != 3 || parts[0] != oid || parts[1] != "blob" {
                return Err("Git object identity mismatch.".into());
            }
            let length: usize = parts[2].parse().map_err(|_| "Invalid Git object length.")?;
            total = total.checked_add(length).ok_or("Baseline size overflow.")?;
            if length > 1024 * 1024 || total > 64 * 1024 * 1024 {
                return Err("Baseline exceeds 1 MiB/file or 64 MiB aggregate.".into());
            }
            let mut data = vec![0; length];
            output
                .read_exact(&mut data)
                .await
                .map_err(|e| e.to_string())?;
            if output.read_u8().await.map_err(|e| e.to_string())? != b'\n' {
                return Err("Invalid Git object terminator.".into());
            }
            let text = match String::from_utf8(data) {
                Ok(text) => text,
                Err(_) => {
                    warnings.push(json!({"path":path,"reason":"baseline_binary_excluded"}));
                    continue;
                }
            };
            let target = directory.path().join(path);
            std::fs::create_dir_all(target.parent().ok_or("Missing baseline parent.")?)
                .map_err(|e| e.to_string())?;
            std::fs::write(target, &text).map_err(|e| e.to_string())?;
            files.insert(path.to_string(), text);
        }
        drop(input);
        child.wait().await.map_err(|e| e.to_string())?;
        Ok(Baseline {
            directory,
            commit,
            files,
            warnings,
        })
    })
    .await
    .map_err(|_| "Baseline preparation exceeded 30 seconds.")?
}
#[cfg(test)]
mod tests {
    use super::*;
    pub(super) fn commit(root: &std::path::Path) {
        for args in [
            vec!["init", "-q"],
            vec!["add", "."],
            vec![
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "baseline",
            ],
        ] {
            assert!(std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .status()
                .unwrap()
                .success());
        }
    }
    #[tokio::test]
    async fn pinned_baseline_preserves_worktree_index_and_branch() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("deleted.rs"), "pub fn removed() {}\n").unwrap();
        std::fs::write(t.path().join("survives.rs"), "pub fn removed_symbol() {}\n").unwrap();
        commit(t.path());
        std::fs::remove_file(t.path().join("deleted.rs")).unwrap();
        std::fs::write(t.path().join("survives.rs"), "pub fn replacement() {}\n").unwrap();
        let index = std::fs::read(t.path().join(".git/index")).unwrap();
        let head = std::fs::read(t.path().join(".git/HEAD")).unwrap();
        let baseline = prepare(t.path().to_str().unwrap()).await.unwrap();
        assert!(baseline.files["deleted.rs"].contains("removed"));
        assert!(baseline.files["survives.rs"].contains("removed_symbol"));
        assert_eq!(std::fs::read(t.path().join(".git/index")).unwrap(), index);
        assert_eq!(std::fs::read(t.path().join(".git/HEAD")).unwrap(), head);
        assert!(!t.path().join("deleted.rs").exists());
        assert!(std::fs::read_to_string(t.path().join("survives.rs"))
            .unwrap()
            .contains("replacement"));
        let owned = baseline.directory.path().to_path_buf();
        drop(baseline);
        assert!(!owned.exists());
    }
}
