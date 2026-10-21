//! Stage ordered LSP resource/text edits into one hash-pinned transaction.
use crate::development_tools::hash;
use crate::lsp_tools::{edited_text, uri_path, ChangedFile};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
pub fn prepare(
    root: &str,
    edit: &Value,
    documents: &[crate::lsp_client::Document],
) -> Result<(Vec<ChangedFile>, Vec<Value>), String> {
    let canonical = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    let workspace = crate::workspace::Workspace::open(root)?;
    let mut before = BTreeMap::<String, Option<String>>::new();
    let mut after = before.clone();
    let mut used = BTreeSet::new();
    let annotations = edit["changeAnnotations"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    let mut operations = edit["documentChanges"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if edit.get("documentChanges").is_some() && edit.get("changes").is_some() {
        return Err("Ambiguous WorkspaceEdit contains both edit representations.".into());
    }
    if let Some(changes) = edit["changes"].as_object() {
        for (uri, edits) in changes {
            operations.push(json!({"textDocument":{"uri":uri,"version":null},"edits":edits}));
        }
    }
    if operations.len() > 1024 {
        return Err("Semantic edit exceeds 1024 operations.".into());
    }
    for op in operations {
        if let Some(id) = op["annotationId"].as_str() {
            used.insert(id.to_string());
        }
        let uris = if op["kind"] == "rename" {
            vec!["oldUri", "newUri"]
        } else if op.get("kind").is_some() {
            vec!["uri"]
        } else {
            vec![]
        };
        let mut paths = Vec::new();
        for key in uris {
            let p = uri_path(
                &canonical,
                op[key].as_str().ok_or("Resource operation lacks URI.")?,
            )?;
            if !before.contains_key(&p) {
                let content = workspace.language_optional_source(&p)?;
                before.insert(p.clone(), content.clone());
                after.insert(p.clone(), content);
            }
            paths.push(p);
        }
        match op["kind"].as_str() {
            Some("create") => {
                let p = &paths[0];
                if after[p].is_some() {
                    if op["options"]["ignoreIfExists"] == true {
                        continue;
                    }
                    if op["options"]["overwrite"] != true {
                        return Err("Create destination already exists.".into());
                    }
                }
                after.insert(p.clone(), Some(String::new()));
            }
            Some("delete") => {
                let p = &paths[0];
                if after[p].is_none() && op["options"]["ignoreIfNotExists"] != true {
                    return Err("Delete target is absent.".into());
                }
                after.insert(p.clone(), None);
            }
            Some("rename") => {
                let old = &paths[0];
                let new = &paths[1];
                if old == new {
                    continue;
                }
                if old.to_lowercase() == new.to_lowercase() {
                    if !workspace.case_rename_destination(old, new)? {
                        return Err(
                            "Case-only rename collides with a distinct existing file.".into()
                        );
                    }
                    // On insensitive filesystems the destination lookup aliases
                    // the original. Its logical pre-transaction state is absent.
                    before.insert(new.clone(), None);
                    after.insert(new.clone(), None);
                }
                let content = after[old].clone().ok_or("Rename source is absent.")?;
                if after[new].is_some() {
                    if op["options"]["ignoreIfExists"] == true {
                        continue;
                    }
                    if op["options"]["overwrite"] != true {
                        return Err("Rename destination already exists.".into());
                    }
                }
                after.insert(old.clone(), None);
                after.insert(new.clone(), Some(content));
            }
            Some(_) => return Err("Unsupported LSP resource operation.".into()),
            None => {
                let p = uri_path(
                    &canonical,
                    op["textDocument"]["uri"]
                        .as_str()
                        .ok_or("Text edit lacks URI.")?,
                )?;
                if !before.contains_key(&p) {
                    let content = workspace.language_optional_source(&p)?;
                    before.insert(p.clone(), content.clone());
                    after.insert(p.clone(), content);
                }
                if let Some(version) = op["textDocument"]["version"].as_i64() {
                    let d = documents
                        .iter()
                        .find(|d| uri_path(&canonical, &d.uri).ok().as_ref() == Some(&p))
                        .ok_or("Versioned edit targets an unopened document.")?;
                    if i64::from(d.version) != version
                        || before[&p].as_ref().map(|s| hash(s.as_bytes())) != Some(d.sha256.clone())
                    {
                        return Err("Versioned semantic target is stale.".into());
                    }
                }
                let edits = op["edits"].as_array().ok_or("Text edit lacks edit list.")?;
                for e in edits {
                    if let Some(id) = e["annotationId"].as_str() {
                        used.insert(id.into());
                    }
                }
                let current = after[&p]
                    .as_deref()
                    .ok_or("Text edits require an existing or explicitly created file.")?;
                let text = edited_text(current, edits)?;
                after.insert(p, Some(text));
            }
        }
        if before.len() > 16 {
            return Err("Semantic proposal exceeds 16 files.".into());
        }
    }
    let mut confirmations = Vec::new();
    for (id, value) in &annotations {
        if !value["label"].is_string()
            || value
                .get("needsConfirmation")
                .is_some_and(|v| !v.is_boolean())
        {
            return Err("Malformed semantic change annotation.".into());
        }
        let mut v = value.clone();
        v["id"] = json!(id);
        confirmations.push(v);
    }
    if used.iter().any(|id| !annotations.contains_key(id)) {
        return Err("Semantic edit uses unknown annotation.".into());
    }
    let mut files = Vec::new();
    let mut bytes = 0;
    for (path, content) in after {
        let original = before.remove(&path).ok_or("Missing original snapshot.")?;
        if original == content {
            continue;
        }
        bytes += original.as_ref().map_or(0, String::len) + content.as_ref().map_or(0, String::len);
        if bytes > 8 * 1024 * 1024 {
            return Err("Semantic edit exceeds 8 MiB.".into());
        }
        let diff = crate::workspace::language_diff(
            &path,
            original.as_deref().unwrap_or(""),
            content.as_deref().unwrap_or(""),
        );
        files.push(ChangedFile {
            path,
            expected_sha256: original.as_ref().map(|s| hash(s.as_bytes())),
            sha256: content.as_ref().map(|s| hash(s.as_bytes())),
            content,
            diff,
        });
    }
    Ok((files, confirmations))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn case_only_rename_plus_edit_preserves_requested_spelling() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("Name.rs"), "old").unwrap();
        let uri = |p: &str| {
            reqwest::Url::from_file_path(t.path().join(p))
                .unwrap()
                .to_string()
        };
        let (files, _) = prepare(t.path().to_str().unwrap(), &json!({"documentChanges":[
            {"kind":"rename","oldUri":uri("Name.rs"),"newUri":uri("name.rs")},
            {"textDocument":{"uri":uri("name.rs")},"edits":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}},"newText":"new"}]}
        ]}), &[]).unwrap();
        let result = crate::workspace::Workspace::open(t.path().to_str().unwrap())
            .unwrap()
            .apply_language_resources(
                files
                    .into_iter()
                    .map(|f| (f.path, f.expected_sha256, f.content))
                    .collect(),
                vec![],
            )
            .unwrap();
        assert_eq!(result["status"], "committed", "{result}");
        assert_eq!(
            std::fs::read_to_string(t.path().join("name.rs")).unwrap(),
            "new"
        );
    }
    #[test]
    fn ordered_resources_are_prepared_without_mutation_then_applied() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path().to_str().unwrap();
        std::fs::write(t.path().join("old.rs"), "old").unwrap();
        let uri = |p: &str| {
            reqwest::Url::from_file_path(t.path().join(p))
                .unwrap()
                .to_string()
        };
        let (files,annotations)=prepare(root,&json!({"documentChanges":[{"kind":"rename","oldUri":uri("old.rs"),"newUri":uri("new.rs"),"annotationId":"move"},{"textDocument":{"uri":uri("new.rs"),"version":null},"edits":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":3}},"newText":"new"}]}],"changeAnnotations":{"move":{"label":"Move source","needsConfirmation":true}}}),&[]).unwrap();
        assert!(t.path().join("old.rs").exists());
        assert_eq!(annotations[0]["id"], "move");
        crate::workspace::Workspace::open(root)
            .unwrap()
            .apply_language_resources(
                files
                    .into_iter()
                    .map(|f| (f.path, f.expected_sha256, f.content))
                    .collect(),
                vec![],
            )
            .unwrap();
        assert!(!t.path().join("old.rs").exists());
        assert_eq!(
            std::fs::read_to_string(t.path().join("new.rs")).unwrap(),
            "new"
        );
    }
}
