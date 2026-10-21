//! Bounded nbformat 4 notebook reads and hash-pinned, journaled cell edits.
//! Detection is by JSON shape (nbformat 4 + cells array), never extension.
use super::patch::{bytes, require_hash, Desired};
use super::{valid_path, Workspace};
use base64::Engine;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

const MAX_NOTEBOOK: usize = 1024 * 1024;
/// Per-cell output text returned to the model.
const MAX_CELL_OUTPUT: usize = 4 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadRequest {
    path: String,
    #[serde(default = "default_outputs")]
    include_outputs: bool,
}
fn default_outputs() -> bool {
    true
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EditRequest {
    path: String,
    expected_sha256: String,
    edit_mode: String,
    cell_index: usize,
    cell_type: Option<String>,
    source: Option<String>,
}

fn joined(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(lines) => lines.iter().filter_map(Value::as_str).collect(),
        _ => String::new(),
    }
}

fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for code in chars.by_ref() {
                    if code.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// Source is stored the way the existing cell stores it: Jupyter writes a
/// list of lines; a plain string cell keeps the string form.
fn source_value(source: &str, as_list: bool) -> Value {
    if as_list {
        Value::Array(
            source
                .split_inclusive('\n')
                .map(|line| json!(line))
                .collect(),
        )
    } else {
        json!(source)
    }
}

fn notebook_bytes(workspace: &Workspace, path: &str) -> Result<Vec<u8>, String> {
    bytes(&workspace.directory, path, MAX_NOTEBOOK)?
        .ok_or_else(|| "Notebook file does not exist.".to_string())
}

fn parse_notebook(data: &[u8]) -> Result<Value, String> {
    let notebook: Value = serde_json::from_slice(data)
        .map_err(|_| "This file is not a notebook (nbformat 4 JSON required).".to_string())?;
    if notebook["nbformat"].as_u64() != Some(4) || !notebook["cells"].is_array() {
        return Err("This file is not a notebook (nbformat 4 JSON required).".into());
    }
    Ok(notebook)
}

fn cell_id_needed(notebook: &Value) -> bool {
    notebook["nbformat_minor"].as_u64().unwrap_or(0) >= 5
}

fn new_cell_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

fn push_entry(
    items: &mut Vec<Value>,
    budget: &mut usize,
    truncated: &mut bool,
    kind: &str,
    mut text: String,
    extra: Map<String, Value>,
) {
    if text.len() > *budget {
        let mut boundary = *budget;
        while !text.is_char_boundary(boundary) {
            boundary -= 1;
        }
        text.truncate(boundary);
        *truncated = true;
    }
    *budget = budget.saturating_sub(text.len());
    let mut entry = extra;
    entry.insert("type".into(), json!(kind));
    entry.insert("text".into(), json!(text));
    items.push(Value::Object(entry));
}

/// One output becomes one or more model-facing entries: stream text,
/// text/plain, stripped tracebacks; images report mime and byte count only.
fn summarize_outputs(outputs: &Value) -> (Vec<Value>, bool) {
    let mut items = Vec::new();
    let mut budget = MAX_CELL_OUTPUT;
    let mut truncated = false;
    for output in outputs.as_array().into_iter().flatten() {
        match output["output_type"].as_str().unwrap_or("") {
            "stream" => {
                let mut extra = Map::new();
                extra.insert("name".into(), output["name"].clone());
                push_entry(
                    &mut items,
                    &mut budget,
                    &mut truncated,
                    "stream",
                    joined(&output["text"]),
                    extra,
                );
            }
            "error" => {
                let mut extra = Map::new();
                extra.insert("ename".into(), output["ename"].clone());
                extra.insert("evalue".into(), output["evalue"].clone());
                let traceback = output["traceback"]
                    .as_array()
                    .map(|lines| {
                        lines
                            .iter()
                            .map(|line| strip_ansi(line.as_str().unwrap_or("")))
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .unwrap_or_default();
                push_entry(
                    &mut items,
                    &mut budget,
                    &mut truncated,
                    "error",
                    traceback,
                    extra,
                );
            }
            "execute_result" | "display_data" => {
                if let Some(data) = output["data"].as_object() {
                    let mut texts = Vec::new();
                    let mut images = Vec::new();
                    for (mime, payload) in data {
                        if mime.starts_with("image/") {
                            let encoded: String = joined(payload)
                                .chars()
                                .filter(|c| !c.is_whitespace())
                                .collect();
                            let decoded = base64::engine::general_purpose::STANDARD
                                .decode(&encoded)
                                .map(|bytes| bytes.len())
                                .unwrap_or(encoded.len());
                            images.push(json!({"type":"image","mime":mime,"bytes":decoded}));
                        } else if mime.starts_with("text/") {
                            texts.push(joined(payload));
                        }
                    }
                    for text in texts {
                        push_entry(
                            &mut items,
                            &mut budget,
                            &mut truncated,
                            "text",
                            text,
                            Map::new(),
                        );
                    }
                    items.extend(images);
                }
            }
            _ => {}
        }
    }
    (items, truncated)
}

/// Jupyter writes 1-space indents; detect 1 vs 2 and default to 1.
fn detect_indent(text: &str) -> usize {
    for line in text.lines().skip(1) {
        let spaces = line.len() - line.trim_start_matches(' ').len();
        if spaces > 0 && !line.trim_start().is_empty() {
            return spaces.clamp(1, 2);
        }
    }
    1
}

/// serde_json::to_string_pretty indents two spaces and escapes every raw
/// newline inside strings, so rewriting the leading margin is safe.
fn reindent(pretty: &str, indent: usize) -> String {
    if indent == 2 {
        return pretty.to_string();
    }
    let mut out = String::with_capacity(pretty.len());
    for line in pretty.split_inclusive('\n') {
        let spaces = line.len() - line.trim_start_matches(' ').len();
        for _ in 0..spaces / 2 {
            out.push(' ');
        }
        out.push_str(line.trim_start_matches(' '));
    }
    out
}

impl Workspace {
    pub(crate) fn notebook_call(&self, name: &str, arguments: Value) -> Result<Value, String> {
        match name {
            "notebook_read" => {
                let request: ReadRequest =
                    serde_json::from_value(arguments).map_err(|e| e.to_string())?;
                valid_path(&request.path, false)?;
                let data = notebook_bytes(self, &request.path)?;
                let notebook = parse_notebook(&data)?;
                let mut cells = Vec::new();
                for (index, cell) in notebook["cells"].as_array().unwrap().iter().enumerate() {
                    let cell_type = cell["cell_type"].as_str().unwrap_or("");
                    let mut item = json!({"index":index,"cell_type":cell_type,"source":joined(&cell["source"])});
                    if cell_type == "code" {
                        item["execution_count"] = cell["execution_count"].clone();
                        if request.include_outputs {
                            let (outputs, truncated) = summarize_outputs(&cell["outputs"]);
                            item["outputs"] = json!(outputs);
                            item["outputsTruncated"] = json!(truncated);
                        }
                    }
                    cells.push(item);
                }
                Ok(json!({
                    "path":request.path,
                    "sha256":format!("{:x}",Sha256::digest(&data)),
                    "nbformat":notebook["nbformat"].clone(),
                    "nbformat_minor":notebook["nbformat_minor"].clone(),
                    "language":notebook["metadata"]["language_info"]["name"].clone(),
                    "cells":cells,
                }))
            }
            "notebook_edit" => {
                let request: EditRequest =
                    serde_json::from_value(arguments).map_err(|e| e.to_string())?;
                valid_path(&request.path, false)?;
                let data = notebook_bytes(self, &request.path)?;
                require_hash(&request.expected_sha256, &data)?;
                let text = String::from_utf8(data)
                    .map_err(|_| "Notebook must be UTF-8 JSON.".to_string())?;
                let mut notebook = parse_notebook(text.as_bytes())?;
                let needs_id = cell_id_needed(&notebook);
                let cells = notebook["cells"]
                    .as_array_mut()
                    .ok_or("Notebook cells must be an array.")?;
                match request.edit_mode.as_str() {
                    "delete" => {
                        if request.cell_index >= cells.len() {
                            return Err(format!(
                                "cell_index must be below the notebook's {} cells.",
                                cells.len()
                            ));
                        }
                        cells.remove(request.cell_index);
                    }
                    "insert" => {
                        if request.cell_index > cells.len() {
                            return Err(format!(
                                "cell_index may be at most {} to insert.",
                                cells.len()
                            ));
                        }
                        let cell_type = request.cell_type.as_deref().unwrap_or("code");
                        if !matches!(cell_type, "code" | "markdown" | "raw") {
                            return Err("cell_type must be code, markdown or raw.".into());
                        }
                        let mut cell = json!({"cell_type":cell_type,"metadata":{},"source":source_value(request.source.as_deref().unwrap_or(""),true)});
                        if needs_id {
                            cell["id"] = json!(new_cell_id());
                        }
                        if cell_type == "code" {
                            cell["execution_count"] = Value::Null;
                            cell["outputs"] = json!([]);
                        }
                        cells.insert(request.cell_index, cell);
                    }
                    "replace" => {
                        let Some(cell) = cells.get_mut(request.cell_index) else {
                            return Err(format!(
                                "cell_index must be below the notebook's {} cells.",
                                cells.len()
                            ));
                        };
                        if !cell.is_object() {
                            return Err("Notebook cell must be an object.".into());
                        }
                        let before = joined(&cell["source"]);
                        if let Some(cell_type) = request.cell_type.as_deref() {
                            if !matches!(cell_type, "code" | "markdown" | "raw") {
                                return Err("cell_type must be code, markdown or raw.".into());
                            }
                            cell["cell_type"] = json!(cell_type);
                        }
                        if let Some(source) = &request.source {
                            cell["source"] = source_value(source, cell["source"].is_array());
                        }
                        if cell["cell_type"] == "code" {
                            if request
                                .source
                                .as_deref()
                                .is_some_and(|source| source != before)
                            {
                                // New source invalidates recorded outputs.
                                cell["outputs"] = json!([]);
                                cell["execution_count"] = Value::Null;
                            } else {
                                if !cell["outputs"].is_array() {
                                    cell["outputs"] = json!([]);
                                }
                                if cell.get("execution_count").is_none() {
                                    cell["execution_count"] = Value::Null;
                                }
                            }
                        } else {
                            cell.as_object_mut().unwrap().remove("outputs");
                            cell.as_object_mut().unwrap().remove("execution_count");
                        }
                    }
                    _ => return Err("edit_mode must be replace, insert or delete.".into()),
                }
                let indent = detect_indent(&text);
                let mut serialized = reindent(
                    &serde_json::to_string_pretty(&notebook).map_err(|e| e.to_string())?,
                    indent,
                );
                if text.ends_with('\n') {
                    serialized.push('\n');
                }
                self.transact(
                    vec![Desired {
                        path: request.path,
                        expected: Some(request.expected_sha256),
                        after: Some(serialized.into_bytes()),
                        permissions: None,
                    }],
                    &[],
                )
            }
            _ => Err("Unknown notebook tool.".into()),
        }
    }
}

pub(super) fn definitions() -> Vec<(&'static str, &'static str, Value, Vec<&'static str>)> {
    let path = json!({"type":"string","minLength":1,"maxLength":4096});
    let hash = json!({"type":"string","pattern":"^[a-fA-F0-9]{64}$"});
    vec![
        ("notebook_read","Read a Jupyter notebook (.ipynb, nbformat 4, up to 1 MiB) as structured cells: index, cell_type, joined source, execution_count, and per-cell output summaries (stream text, text/plain, errors stripped of ANSI; images as mime and byte count only). Outputs truncate at 4 KiB per cell.",json!({"path":path,"include_outputs":{"type":"boolean","default":true}}),vec!["path"]),
        ("notebook_edit","Replace, insert or delete one cell in a Jupyter notebook pinned by expected_sha256 from notebook_read. insert takes cell_index 0..len with optional cell_type (code default) and source; replace keeps cell id/metadata and clears recorded outputs when code source changes; delete removes the cell. Notebook metadata, other fields and indentation are preserved. Journaled transaction returning status.",json!({"path":path,"expected_sha256":hash,"edit_mode":{"type":"string","enum":["replace","insert","delete"]},"cell_index":{"type":"integer","minimum":0},"cell_type":{"type":"string","enum":["code","markdown","raw"]},"source":{"type":"string"}}),vec!["path","expected_sha256","edit_mode","cell_index"]),
    ]
}

#[cfg(test)]
mod tests {
    use super::super::patch::transaction_ids;
    use super::*;
    use std::path::Path;

    fn workspace(root: &Path) -> Workspace {
        Workspace::open(root.to_str().unwrap()).unwrap()
    }
    fn sha(w: &Workspace, path: &str) -> String {
        w.call("file_info", json!({"path":path})).unwrap()["sha256"]
            .as_str()
            .unwrap()
            .to_string()
    }
    fn fixture() -> String {
        serde_json::to_string_pretty(&json!({
            "nbformat":4,"nbformat_minor":5,
            "metadata":{"language_info":{"name":"python"},"kernelspec":{"name":"python3"}},
            "cells":[
                {"cell_type":"markdown","id":"md-1","metadata":{},"source":["# Title\n","more"]},
                {"cell_type":"code","id":"c-1","metadata":{"tags":["x"]},"source":"print('hi')","execution_count":3,
                 "outputs":[{"output_type":"stream","name":"stdout","text":["hi\n"]},
                            {"output_type":"error","ename":"E","evalue":"v","traceback":["[31mboom[0m"]},
                            {"output_type":"display_data","data":{"image/png":"aGVsbG8="}}]}
            ]
        })).unwrap()
    }

    #[test]
    fn notebook_read_summarizes_cells_and_outputs() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("n.ipynb"), fixture()).unwrap();
        let w = workspace(root.path());
        let result = w.call("notebook_read", json!({"path":"n.ipynb"})).unwrap();
        assert_eq!(result["nbformat"], 4);
        assert_eq!(result["language"], "python");
        let cells = result["cells"].as_array().unwrap();
        assert_eq!(cells[0]["cell_type"], "markdown");
        assert_eq!(cells[0]["source"], "# Title\nmore");
        assert_eq!(cells[1]["execution_count"], 3);
        let outputs = cells[1]["outputs"].as_array().unwrap();
        assert_eq!(outputs[0]["text"], "hi\n");
        assert_eq!(outputs[1]["ename"], "E");
        assert_eq!(outputs[1]["text"], "boom");
        assert_eq!(outputs[2]["mime"], "image/png");
        assert_eq!(outputs[2]["bytes"], 5);
        // Outputs can be skipped entirely.
        let skipped = w
            .call(
                "notebook_read",
                json!({"path":"n.ipynb","include_outputs":false}),
            )
            .unwrap();
        assert!(skipped["cells"][1].get("outputs").is_none());
    }

    #[test]
    fn notebook_rejects_non_notebook_json_and_bounds() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("plain.json"), "{}").unwrap();
        std::fs::write(
            root.path().join("v3.ipynb"),
            "{\"nbformat\":3,\"cells\":[]}",
        )
        .unwrap();
        std::fs::write(root.path().join("big.ipynb"), vec![b' '; MAX_NOTEBOOK + 1]).unwrap();
        let w = workspace(root.path());
        for path in ["plain.json", "v3.ipynb", "big.ipynb", "missing.ipynb"] {
            assert!(
                w.call("notebook_read", json!({"path":path})).is_err(),
                "{path}"
            );
        }
    }

    #[test]
    fn notebook_edit_insert_replace_delete_roundtrip_preserves_fields() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("n.ipynb"), fixture()).unwrap();
        let w = workspace(root.path());
        let hash = sha(&w, "n.ipynb");
        // Insert at index 0 gets a generated id under nbformat_minor >= 5.
        let inserted = w.call("notebook_edit", json!({"path":"n.ipynb","expected_sha256":hash,"edit_mode":"insert","cell_index":0,"cell_type":"markdown","source":"intro"})).unwrap();
        assert_eq!(inserted["status"], "committed", "{inserted}");
        let raw: Value =
            serde_json::from_str(&std::fs::read_to_string(root.path().join("n.ipynb")).unwrap())
                .unwrap();
        assert_eq!(raw["cells"].as_array().unwrap().len(), 3);
        assert_eq!(raw["cells"][0]["cell_type"], "markdown");
        assert_eq!(raw["cells"][0]["source"], json!(["intro"]));
        assert_eq!(raw["cells"][0]["id"].as_str().unwrap().len(), 8);
        // Untouched fields survive: metadata, kernelspec, downstream ids/tags.
        assert_eq!(raw["metadata"]["kernelspec"]["name"], "python3");
        assert_eq!(raw["cells"][2]["id"], "c-1");
        // Replace with changed source clears outputs and execution_count.
        let hash = sha(&w, "n.ipynb");
        let replaced = w.call("notebook_edit", json!({"path":"n.ipynb","expected_sha256":hash,"edit_mode":"replace","cell_index":2,"source":"print('bye')"})).unwrap();
        assert_eq!(replaced["status"], "committed", "{replaced}");
        let raw: Value =
            serde_json::from_str(&std::fs::read_to_string(root.path().join("n.ipynb")).unwrap())
                .unwrap();
        let cell = &raw["cells"][2];
        assert_eq!(cell["source"], json!("print('bye')"));
        assert_eq!(cell["id"], "c-1");
        assert_eq!(cell["metadata"]["tags"], json!(["x"]));
        assert_eq!(cell["outputs"], json!([]));
        assert!(cell["execution_count"].is_null());
        // Bounds, modes and pinning.
        let hash = sha(&w, "n.ipynb");
        for args in [
            json!({"path":"n.ipynb","expected_sha256":"0".repeat(64),"edit_mode":"delete","cell_index":0}),
            json!({"path":"n.ipynb","expected_sha256":hash,"edit_mode":"delete","cell_index":9}),
            json!({"path":"n.ipynb","expected_sha256":hash,"edit_mode":"insert","cell_index":9,"cell_type":"code"}),
            json!({"path":"n.ipynb","expected_sha256":hash,"edit_mode":"replace","cell_index":0,"cell_type":"bogus"}),
            json!({"path":"n.ipynb","expected_sha256":hash,"edit_mode":"bogus","cell_index":0}),
        ] {
            assert!(w.call("notebook_edit", args.clone()).is_err(), "{args}");
        }
        let hash = sha(&w, "n.ipynb");
        let deleted = w.call("notebook_edit", json!({"path":"n.ipynb","expected_sha256":hash,"edit_mode":"delete","cell_index":0})).unwrap();
        assert_eq!(deleted["status"], "committed", "{deleted}");
        let raw: Value =
            serde_json::from_str(&std::fs::read_to_string(root.path().join("n.ipynb")).unwrap())
                .unwrap();
        assert_eq!(raw["cells"].as_array().unwrap().len(), 2);
        assert!(transaction_ids(&w.directory).unwrap().is_empty());
    }

    #[test]
    fn notebook_edit_preserves_one_space_jupyter_indentation() {
        let root = tempfile::tempdir().unwrap();
        // Jupyter-style single-space indent with trailing newline.
        std::fs::write(root.path().join("j.ipynb"), "{\n \"cells\": [],\n \"metadata\": {},\n \"nbformat\": 4,\n \"nbformat_minor\": 5\n}\n").unwrap();
        let w = workspace(root.path());
        let hash = sha(&w, "j.ipynb");
        w.call("notebook_edit", json!({"path":"j.ipynb","expected_sha256":hash,"edit_mode":"insert","cell_index":0,"cell_type":"code","source":"x=1"})).unwrap();
        let text = std::fs::read_to_string(root.path().join("j.ipynb")).unwrap();
        assert!(text.starts_with("{\n \"cells\": [\n  {"), "{text}");
        assert!(text.ends_with("}\n"), "{text}");
    }
}
