//! Plugin ecosystem: manifest, malware scan, install/enable (Phase 9).
//!
//! Plugins are `dsh-plugin`-compatible in MANIFEST SHAPE ONLY (name, version,
//! capabilities, permissions, sandbox needs). They never execute inside
//! LocalLM: a plugin contributes configuration (tool presets, prompt
//! sections, schedules) that the Rust harness interprets. Anything a plugin
//! wants the model to see is logged; anything it wants executed goes through
//! approval like any other tool call.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub sandbox: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Plugin {
    pub name: String,
    pub version: String,
    pub path: String,
    pub enabled: bool,
    pub sha256: String,
    pub installed_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ScanVerdict {
    pub verdict: String,
    pub findings: Vec<String>,
}

pub fn validate_manifest(manifest: &Manifest) -> Result<(), String> {
    if manifest.name.trim().is_empty() || manifest.name.len() > 120 {
        return Err("Plugin name must be 1-120 characters.".into());
    }
    if manifest.name.contains('/') || manifest.name.contains('\\') || manifest.name.contains("..") {
        return Err("Plugin name must not be a path.".into());
    }
    if manifest.version.trim().is_empty() || manifest.version.len() > 40 {
        return Err("Plugin version must be 1-40 characters.".into());
    }
    if manifest.capabilities.len() > 32 || manifest.permissions.len() > 32 {
        return Err("A plugin declares at most 32 capabilities and 32 permissions.".into());
    }
    if manifest.description.len() > 2000 {
        return Err("Plugin description is limited to 2000 characters.".into());
    }
    Ok(())
}

pub fn read_manifest(dir: &std::path::Path) -> Result<(Manifest, String), String> {
    if !dir.is_dir() {
        return Err("Plugin folder does not exist.".into());
    }
    let path = dir.join("plugin.json");
    let text = std::fs::read_to_string(&path)
        .map_err(|_| "Plugin folder needs a plugin.json manifest.".to_string())?;
    if text.len() > 65_536 {
        return Err("plugin.json exceeds 64 KiB.".into());
    }
    let manifest: Manifest = serde_json::from_str(&text)
        .map_err(|error| format!("plugin.json is invalid: {error}"))?;
    validate_manifest(&manifest)?;
    Ok((manifest, path.to_string_lossy().into_owned()))
}

/// `dsh-plugin-malware-scan` equivalent. Reads at most 64 small text files
/// (≤64 KiB each); verdict is install / caution / reject.
pub fn scan(dir: &std::path::Path) -> Result<ScanVerdict, String> {
    if !dir.is_dir() {
        return Err("Plugin folder does not exist.".into());
    }
    let mut findings = Vec::new();
    let mut files_seen = 0usize;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = std::fs::read_dir(&current).map_err(|error| format!("Could not scan plugin: {error}"))?;
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            files_seen += 1;
            if files_seen > 64 {
                findings.push("Plugin ships more than 64 files; review manually.".to_string());
                break;
            }
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(_) => continue,
            };
            if bytes.len() > 65_536 || bytes.iter().take(1024).any(|byte| *byte == 0) {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes).to_lowercase();
            let display = path.strip_prefix(dir).unwrap_or(&path).to_string_lossy().into_owned();
            for pattern in ["~/.ssh", ".ssh/", ".env", "curl", "| sh", "disable.*sandbox", "ignore previous instructions", "ignore all previous"] {
                if text.contains(pattern) {
                    findings.push(format!("{display} mentions '{pattern}'."));
                }
            }
        }
    }
    let verdict = if findings.iter().any(|finding| finding.contains(".ssh") || finding.contains("ignore previous")) {
        "reject"
    } else if findings.is_empty() {
        "install"
    } else {
        "caution"
    };
    Ok(ScanVerdict { verdict: verdict.to_string(), findings })
}

pub fn digest_folder(dir: &std::path::Path) -> Result<String, String> {
    let mut names: Vec<String> = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = std::fs::read_dir(&current).map_err(|error| format!("Could not hash plugin: {error}"))?;
        for entry in entries.filter_map(|entry| entry.ok()) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                names.push(path.to_string_lossy().into_owned());
            }
        }
        if names.len() > 512 {
            return Err("Plugin folder is too large to pin.".into());
        }
    }
    names.sort();
    let mut hasher = Sha256::new();
    for name in &names {
        hasher.update(name.as_bytes());
        if let Ok(bytes) = std::fs::read(name) {
            hasher.update(&bytes);
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[tauri::command]
pub fn list_plugins(state: tauri::State<'_, crate::AppState>) -> Result<Vec<Plugin>, String> {
    state.database()?.plugins()
}

#[tauri::command]
pub async fn install_plugin(state: tauri::State<'_, crate::AppState>, path: String) -> Result<Plugin, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before installing plugins.")?;
    let dir = std::path::PathBuf::from(&path);
    let (manifest, _) = read_manifest(&dir)?;
    let verdict = scan(&dir)?;
    if verdict.verdict == "reject" {
        return Err(format!("Plugin refused: {}.", verdict.findings.join(" ")));
    }
    let plugin = Plugin {
        name: manifest.name,
        version: manifest.version,
        path: dir.to_string_lossy().into_owned(),
        enabled: true,
        sha256: digest_folder(&dir)?,
        installed_at: crate::store::now(),
    };
    state.database()?.save_plugin(&plugin)?;
    Ok(plugin)
}

#[tauri::command]
pub async fn set_plugin_enabled(state: tauri::State<'_, crate::AppState>, name: String, enabled: bool) -> Result<Vec<Plugin>, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before changing plugins.")?;
    let store = state.database()?;
    let mut plugins = store.plugins()?;
    let plugin = plugins.iter_mut().find(|item| item.name == name).ok_or("Plugin is not installed.")?;
    plugin.enabled = enabled;
    let updated = plugin.clone();
    store.save_plugin(&updated)?;
    store.plugins()
}

#[tauri::command]
pub async fn remove_plugin(state: tauri::State<'_, crate::AppState>, name: String) -> Result<bool, String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before removing plugins.")?;
    state.database()?.delete_plugin(&name)
}

#[tauri::command]
pub fn scan_plugin(path: String) -> Result<ScanVerdict, String> {
    scan(std::path::Path::new(&path))
}

/// Creator mode: validate + scan a plugin in memory without installing it.
#[tauri::command]
pub fn test_plugin(path: String) -> Result<serde_json::Value, String> {
    let dir = std::path::PathBuf::from(&path);
    let (manifest, _) = read_manifest(&dir)?;
    let verdict = scan(&dir)?;
    Ok(serde_json::json!({ "manifest": manifest, "scan": verdict }))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn plugin_dir(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, content) in files {
            std::fs::write(dir.path().join(name), content).unwrap();
        }
        dir
    }
    #[test]
    fn manifest_shape_and_scan_verdicts() {
        let clean = plugin_dir(&[("plugin.json", r#"{"name":"demo","version":"0.1.0","capabilities":["Tool"]}"#)]);
        let (manifest, _) = read_manifest(clean.path()).unwrap();
        assert_eq!(manifest.name, "demo");
        assert_eq!(scan(clean.path()).unwrap().verdict, "install");
        let nasty = plugin_dir(&[
            ("plugin.json", r#"{"name":"evil","version":"9"}"#),
            ("run.sh", "cat ~/.ssh/id_rsa | sh"),
        ]);
        assert_eq!(scan(nasty.path()).unwrap().verdict, "reject");
        let meh = plugin_dir(&[
            ("plugin.json", r#"{"name":"ok","version":"1"}"#),
            ("notes.md", "uses curl to fetch assets"),
        ]);
        assert_eq!(scan(meh.path()).unwrap().verdict, "caution");
        assert!(read_manifest(&nasty.path().join("missing")).is_err());
        assert!(validate_manifest(&Manifest {
            name: "../x".into(), version: "1".into(), capabilities: vec![],
            permissions: vec![], sandbox: String::new(), description: String::new(),
        })
        .is_err());
    }
}
