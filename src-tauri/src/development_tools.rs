//! Immutable workspace evidence shared by edit and command observations.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn freshness(root: &str, snapshot: &Value) -> Value {
    let mut changed = Vec::new();
    let mut unavailable = Vec::new();
    match crate::workspace::Workspace::open(root) {
        Ok(workspace) => {
            for (p, sha) in snapshot["files"].as_object().into_iter().flatten() {
                match workspace.language_optional_source(p) {
                    Ok(t)
                        if t.as_ref()
                            .map(|s| json!(hash(s.as_bytes())))
                            .unwrap_or(Value::Null)
                            == *sha => {}
                    Ok(_) => changed.push(p.clone()),
                    Err(_) => unavailable.push(p.clone()),
                }
            }
        }
        Err(_) => unavailable.push("workspace unavailable".into()),
    }
    json!({"matches":changed.is_empty()&&unavailable.is_empty(),"changed":changed,"unavailable":unavailable,"scope":"explicit_files_only","limitation":"Hashes compare observation endpoints; they do not isolate concurrent external writes or cover unselected dependencies."})
}
