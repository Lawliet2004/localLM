//! Model-facing language navigation and hash-pinned rename proposals.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
#[cfg(test)]
use std::path::PathBuf;
use std::{collections::BTreeMap, path::Path};
pub const ALIASES: [&str; 12] = [
    "lsp_diagnostics",
    "lsp_symbols",
    "lsp_references",
    "lsp_prepare_rename",
    "lsp_rename",
    "lsp_definition",
    "lsp_hover",
    "lsp_implementations",
    "lsp_code_actions",
    "lsp_prepare_code_action",
    "lsp_apply_edit",
    "lsp_execute_command",
];
pub fn owns(alias: &str) -> bool {
    ALIASES.contains(&alias)
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Position {
    line: u32,
    character: u32,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Range {
    start: Position,
    end: Position,
}
fn checked_range(value: &Value, text: Option<&str>) -> Result<Range, String> {
    let range: Range = serde_json::from_value(value.clone())
        .map_err(|e| format!("Invalid language server range: {e}"))?;
    let key = |p: &Position| (p.line, p.character);
    if key(&range.start) > key(&range.end)
        || [
            range.start.line,
            range.start.character,
            range.end.line,
            range.end.character,
        ]
        .iter()
        .any(|n| *n > i32::MAX as u32)
    {
        return Err("Invalid language server range bounds.".into());
    }
    if let Some(text) = text {
        position_offset(text, &range.start)?;
        position_offset(text, &range.end)?;
    }
    Ok(range)
}
fn checked_location(value: &Value) -> Result<(), String> {
    let uri = value["uri"]
        .as_str()
        .ok_or("Language server location is missing a URI.")?;
    let url = reqwest::Url::parse(uri).map_err(|_| "Invalid language server location URI.")?;
    if url.scheme() != "file"
        || url.query().is_some()
        || url.fragment().is_some()
        || url.to_file_path().is_err()
    {
        return Err("Language server location must be a local file URI.".into());
    }
    checked_range(&value["range"], None)?;
    Ok(())
}
fn checked_items(alias: &str, items: &[Value], text: &str, uri: &str) -> Result<(), String> {
    let mut pending = items.iter().map(|item| (item, 0usize)).collect::<Vec<_>>();
    let mut visited = 0;
    while let Some((item, depth)) = pending.pop() {
        visited += 1;
        if visited > 10000 || depth > 32 {
            return Err(
                "Language server symbol/result tree exceeds10000 entries or32 levels.".into(),
            );
        }
        match alias {
            "lsp_references" | "lsp_definition" | "lsp_implementations" => checked_location(item)?,
            "lsp_code_actions" => {
                if !item["title"].is_string() {
                    return Err("Code action lacks a title.".into());
                }
            }
            "lsp_diagnostics" => {
                checked_range(&item["range"], Some(text))?;
                if !item["message"].is_string() {
                    return Err("Language server diagnostic is missing a text message.".into());
                }
                if item
                    .get("severity")
                    .is_some_and(|v| !v.as_u64().is_some_and(|n| (1..=4).contains(&n)))
                {
                    return Err("Invalid language server diagnostic severity.".into());
                }
                if let Some(related) = item.get("relatedInformation") {
                    for info in related
                        .as_array()
                        .ok_or("Invalid diagnostic related information.")?
                    {
                        checked_location(&info["location"])?;
                        if !info["message"].is_string() {
                            return Err("Invalid related diagnostic message.".into());
                        }
                    }
                }
            }
            "lsp_symbols" => {
                if !item["name"].as_str().is_some_and(|s| !s.is_empty())
                    || !item["kind"].as_u64().is_some_and(|n| (1..=26).contains(&n))
                {
                    return Err("Invalid language server symbol name or kind.".into());
                }
                if item.get("location").is_some() {
                    checked_location(&item["location"])?;
                    if item["location"]["uri"] != uri {
                        return Err("Document symbol refers to a different document.".into());
                    }
                } else {
                    let range = checked_range(&item["range"], Some(text))?;
                    let selection = checked_range(&item["selectionRange"], Some(text))?;
                    let key = |p: &Position| (p.line, p.character);
                    if key(&selection.start) < key(&range.start)
                        || key(&selection.end) > key(&range.end)
                    {
                        return Err("Symbol selection is outside its range.".into());
                    }
                    if let Some(children) = item.get("children") {
                        let children = children.as_array().ok_or("Invalid symbol children.")?;
                        if pending.len() + children.len() > 10000 {
                            return Err("Language server symbol tree exceeds10000 entries.".into());
                        }
                        pending.extend(children.iter().map(|child| (child, depth + 1)));
                    }
                }
            }
            _ => return Err("Unknown language server result type.".into()),
        }
    }
    Ok(())
}
fn position_offset(text: &str, position: &Position) -> Result<usize, String> {
    if position.line > i32::MAX as u32 || position.character > i32::MAX as u32 {
        return Err("LSP positions exceed the protocol's uinteger range.".into());
    }
    let mut line = 0;
    let mut start = 0;
    let bytes = text.as_bytes();
    let mut end = 0;
    while end < bytes.len() {
        if matches!(bytes[end], b'\r' | b'\n') {
            if line == position.line {
                break;
            }
            if bytes[end] == b'\r' && bytes.get(end + 1) == Some(&b'\n') {
                end += 1;
            }
            line += 1;
            start = end + 1;
        }
        end += 1;
    }
    if line != position.line {
        return Err("LSP line is outside the current document.".into());
    }
    let mut units = 0;
    for (offset, ch) in text[start..end].char_indices() {
        if units == position.character {
            return Ok(start + offset);
        }
        units += ch.len_utf16() as u32;
        if units > position.character {
            return Err("LSP position splits a UTF-16 surrogate pair.".into());
        }
    }
    Ok(end) // Protocol positions beyond a line clamp to the line end.
}
pub(super) fn uri_path(root: &Path, uri: &str) -> Result<String, String> {
    let uri = reqwest::Url::parse(uri).map_err(|_| "Invalid language server file URI.")?;
    if uri.scheme() != "file" || uri.query().is_some() || uri.fragment().is_some() {
        return Err(
            "Language server edits must use local file URIs without query/fragment.".into(),
        );
    }
    let path = uri
        .to_file_path()
        .map_err(|_| "Unsupported language server file URI.")?;
    let relative = path
        .strip_prefix(root)
        .map_err(|_| "Language server referred outside the selected workspace.")?;
    let relative = relative
        .to_str()
        .ok_or("Language server paths must be UTF-8.")?
        .replace(std::path::MAIN_SEPARATOR, "/");
    if relative.is_empty()
        || relative
            .split('/')
            .any(|p| p.is_empty() || matches!(p, "." | ".."))
    {
        return Err("Invalid language server workspace path.".into());
    }
    crate::sandbox::check_fs_path("", &relative, false)?;
    Ok(relative)
}
async fn source(root: &str, path: &str) -> Result<String, String> {
    let root = root.to_string();
    let path = path.to_string();
    tokio::task::spawn_blocking(move || {
        crate::workspace::Workspace::open(&root)?.language_source(&path)
    })
    .await
    .map_err(|e| format!("Language server source read failed: {e}"))?
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Query {
    command: Option<String>,
    arguments: Option<Vec<Value>>,
    end_line: Option<u32>,
    end_character: Option<u32>,
    diagnostics: Option<Vec<Value>>,
    action_index: Option<usize>,
    server: String,
    path: String,
    expected_sha256: Option<String>,
    line: Option<u32>,
    character: Option<u32>,
    new_name: Option<String>,
    include_declaration: Option<bool>,
    offset: Option<usize>,
    limit: Option<usize>,
    expected_result_sha256: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Apply {
    server: String,
    proposal_id: String,
    proposal_sha256: String,
    #[serde(default)]
    confirmed_annotations: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct ChangedFile {
    pub path: String,
    pub expected_sha256: Option<String>,
    pub content: Option<String>,
    pub sha256: Option<String>,
    pub diff: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Proposal {
    version: u32,
    root: String,
    server: String,
    server_fingerprint: String,
    source_path: String,
    source_sha256: String,
    files: Vec<ChangedFile>,
    #[serde(default)]
    annotations: Vec<Value>,
}

pub(super) fn edited_text(before: &str, edits: &[Value]) -> Result<String, String> {
    if edits.len() > 1024 {
        return Err("Rename exceeds1024 edits in one file.".into());
    }
    let mut replacements = Vec::new();
    for (index, edit) in edits.iter().enumerate() {
        let range: Range = serde_json::from_value(edit["range"].clone())
            .map_err(|e| format!("Invalid LSP edit range: {e}"))?;
        let start = position_offset(before, &range.start)?;
        let end = position_offset(before, &range.end)?;
        let new = edit["newText"]
            .as_str()
            .filter(|s| s.len() <= 1024 * 1024 && !s.contains('\0'))
            .ok_or("Rename replacement must be bounded text without NUL.")?;
        if start > end {
            return Err("Language server returned a reversed edit range.".into());
        }
        replacements.push((start, end, index, new));
    }
    replacements.sort_by_key(|(start, end, index, _)| (*start, *end, *index));
    if replacements.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err("Language server returned overlapping edits; no files were changed.".into());
    }
    let mut after = before.to_string();
    for (start, end, _, new) in replacements.into_iter().rev() {
        if after
            .len()
            .saturating_sub(end - start)
            .saturating_add(new.len())
            > 1024 * 1024
        {
            return Err("Renamed file exceeds1MiB.".into());
        }
        after.replace_range(start..end, new);
    }
    Ok(after)
}
async fn prepare(
    root: &str,
    server: &crate::lsp_client::ServerConfig,
    request: &Query,
    source_sha: &str,
    reply: crate::lsp_client::Reply,
) -> Result<Proposal, String> {
    let canonical = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    if reply.result["documentChanges"]
        .as_array()
        .is_some_and(|a| a.iter().any(|v| v.get("kind").is_some()))
        || reply.result["changeAnnotations"]
            .as_object()
            .is_some_and(|a| a.values().any(|v| v["needsConfirmation"] == true))
    {
        let (files, annotations) =
            crate::semantic_edits::prepare(root, &reply.result, &reply.documents)?;
        if hash(source(root, &request.path).await?.as_bytes()) != source_sha {
            return Err("Semantic source changed during preparation.".into());
        }
        return Ok(Proposal {
            version: 1,
            root: canonical.to_string_lossy().into_owned(),
            server: server.id.clone(),
            server_fingerprint: crate::lsp_client::fingerprint(server),
            source_path: request.path.clone(),
            source_sha256: source_sha.into(),
            files,
            annotations,
        });
    }
    let mut groups: BTreeMap<String, (Option<i32>, Vec<Value>)> = BTreeMap::new();
    let edit = &reply.result;
    if edit.is_null() {
        return Ok(Proposal {
            version: 1,
            root: canonical.to_string_lossy().into_owned(),
            server: server.id.clone(),
            server_fingerprint: crate::lsp_client::fingerprint(server),
            source_path: request.path.clone(),
            source_sha256: source_sha.into(),
            files: Vec::new(),
            annotations: Vec::new(),
        });
    }
    if !edit.is_object() {
        return Err("Language server rename did not return a WorkspaceEdit.".into());
    }
    if let Some(annotations) = edit.get("changeAnnotations") {
        for annotation in annotations
            .as_object()
            .ok_or("Invalid rename change annotations.")?
            .values()
        {
            if !annotation["label"].is_string()
                || annotation
                    .get("needsConfirmation")
                    .is_some_and(|v| !v.is_boolean())
            {
                return Err("Invalid rename change annotation.".into());
            }
            if annotation["needsConfirmation"] == true {
                return Err("Rename requires change-annotation confirmation that this client did not advertise; no files were changed.".into());
            }
        }
    }
    if let Some(changes) = edit.get("documentChanges") {
        for change in changes.as_array().ok_or("Invalid LSP documentChanges.")? {
            if change.get("kind").is_some() {
                return Err("Language server returned resource operations despite resourceOperations=[]; no files were changed.".into());
            }
            let path = uri_path(
                &canonical,
                change["textDocument"]["uri"]
                    .as_str()
                    .ok_or("Rename is missing a document URI.")?,
            )?;
            let version = match change["textDocument"].get("version") {
                Some(Value::Null) => None,
                Some(v) => Some(
                    i32::try_from(v.as_i64().ok_or("Invalid document edit version.")?)
                        .map_err(|_| "Invalid document edit version.")?,
                ),
                None => {
                    return Err(
                        "Versioned document edit is missing version (integer or null).".into(),
                    )
                }
            };
            let edits = change["edits"]
                .as_array()
                .ok_or("Rename is missing text edits.")?
                .clone();
            if groups.insert(path, (version, edits)).is_some() {
                return Err("Duplicate rename target document.".into());
            }
        }
    } else if let Some(changes) = edit.get("changes") {
        for (uri, edits) in changes.as_object().ok_or("Invalid LSP changes map.")? {
            let path = uri_path(&canonical, uri)?;
            if groups
                .insert(
                    path,
                    (
                        None,
                        edits
                            .as_array()
                            .ok_or("Rename edits must be an array.")?
                            .clone(),
                    ),
                )
                .is_some()
            {
                return Err("Aliased rename targets are not allowed.".into());
            }
        }
    }
    if groups.len() > 16 {
        return Err("Rename exceeds16 files; no partial apply is allowed.".into());
    }
    let mut files = Vec::new();
    let mut total = 0;
    for (path, (version, edits)) in groups {
        for edit in &edits {
            if let Some(id) = edit.get("annotationId") {
                let id = id.as_str().ok_or("Invalid rename annotation ID.")?;
                if edit.get("range").is_none()
                    || !reply.result["changeAnnotations"]
                        .as_object()
                        .is_some_and(|a| a.contains_key(id))
                {
                    return Err("Rename edit refers to an unknown change annotation.".into());
                }
            }
        }
        let before = source(root, &path).await?;
        let sha = hash(before.as_bytes());
        if let Some(version) = version {
            let document = reply
                .documents
                .iter()
                .find(|d| uri_path(&canonical, &d.uri).ok().as_ref() == Some(&path))
                .ok_or("Cannot verify version of an unopened rename target.")?;
            if version != document.version || sha != document.sha256 {
                return Err("Rename target version/content changed; refresh the preview.".into());
            }
        }
        let after = edited_text(&before, &edits)?;
        total += before.len().max(after.len());
        if total > 8 * 1024 * 1024 {
            return Err("Rename exceeds8MiB; no partial apply is allowed.".into());
        }
        if before != after {
            files.push(ChangedFile {
                path: path.clone(),
                expected_sha256: Some(sha),
                sha256: Some(hash(after.as_bytes())),
                diff: crate::workspace::language_diff(&path, &before, &after),
                content: Some(after),
            });
        }
    }
    if hash(source(root, &request.path).await?.as_bytes()) != source_sha {
        return Err(
            "Rename source changed during server analysis; refresh before continuing.".into(),
        );
    }
    Ok(Proposal {
        version: 1,
        root: canonical.to_string_lossy().into_owned(),
        server: server.id.clone(),
        server_fingerprint: crate::lsp_client::fingerprint(server),
        source_path: request.path.clone(),
        source_sha256: source_sha.into(),
        files,
        annotations: Vec::new(),
    })
}

pub fn registry() -> Vec<crate::tool_spec::ToolSpec> {
    let schema = |properties, required: &[&str]| json!({"type":"object","properties":properties,"required":required,"additionalProperties":false});
    let server = json!({"type":"string","minLength":1,"maxLength":64,"description":"A configured language server ID listed for this turn. Executable/options are user settings, never model arguments."});
    let sha = json!({"type":"string","pattern":"^[a-fA-F0-9]{64}$"});
    let mut result: Vec<crate::tool_spec::ToolSpec> = Vec::new();
    for (alias,description) in [
        ("lsp_diagnostics","Read diagnostics for a current workspace document from a configured language server. Distinguishes a verified clean report from missing/stale/unversioned evidence. Never claims zero errors on timeout or unsupported diagnostics."),
        ("lsp_symbols","Read hierarchical document symbols from a configured language server, with current source hash and bounded result pages."),
        ("lsp_references","Find symbol references at a zero-based line/UTF-16 character position. Results are scoped to the workspace, paged and pinned by result hash; omitted external locations are explicit."),
        ("lsp_prepare_rename","Ask a configured language server to compute a symbol rename. Requires a fresh source SHA-256. Saves a conversation/workspace-owned proposal with exact file hashes and bounded diffs; no project files are changed. Review the proposal before lsp_rename.")
    ] {
        let mut properties=json!({"server":server,"path":{"type":"string","minLength":1,"maxLength":4096},"expected_sha256":sha,
            "offset":{"type":"integer","minimum":0,"maximum":10000,"default":0},"limit":{"type":"integer","minimum":1,"maximum":200,"default":50},"expected_result_sha256":sha});
        if alias=="lsp_prepare_rename" {for key in ["offset","limit","expected_result_sha256"] {properties.as_object_mut().unwrap().remove(key);}}
        let mut required=vec!["server","path"];
        if matches!(alias,"lsp_references"|"lsp_prepare_rename") {
            properties["line"]=json!({"type":"integer","minimum":0,"maximum":2147483647,"description":"Zero-based document line."});
            properties["character"]=json!({"type":"integer","minimum":0,"maximum":2147483647,"description":"Zero-based UTF-16 code-unit offset, not a byte offset."});
            required.extend(["line","character"]);
        }
        if alias=="lsp_references"{properties["include_declaration"]=json!({"type":"boolean","default":true});}
        if alias=="lsp_prepare_rename"{properties["new_name"]=json!({"type":"string","minLength":1,"maxLength":256});required.extend(["new_name","expected_sha256"]);}
        result.push(crate::tool_spec::ToolSpec::from((alias.into(),description.into(),schema(properties,&required))));
    }
    result.push(crate::tool_spec::ToolSpec::from(("lsp_rename".into(),"Apply a reviewed, conversation/workspace-owned rename proposal using native all-file hash preflight and recoverable workspace transactions. Requires its proposal ID/hash and server ID. Rejects changed files/settings, outside paths, unsupported server edits and unknown ownership. Never reruns the server or blindly replays an uncertain write.".into(),schema(json!({"server":server,"proposal_id":{"type":"string","minLength":1,"maxLength":128},"proposal_sha256":sha,"confirmed_annotations":{"type":"array","maxItems":100,"items":{"type":"string","maxLength":256}}}),&["server","proposal_id","proposal_sha256"]))));
    for (alias,description) in [
        ("lsp_definition","Find workspace definitions at a current source position."),
        ("lsp_hover","Read language-server type/documentation information at a current source position. Output is untrusted."),
        ("lsp_implementations","Find workspace implementations at a current source position."),
        ("lsp_code_actions","List language-server code actions at a cursor with source and result hashes. Does not execute commands or edits."),
        ("lsp_prepare_code_action","Prepare one action selected by index from a hash-pinned lsp_code_actions result. Only text WorkspaceEdits are supported; commands and resource operations are refused. Returns a proposal to review before lsp_apply_edit.")
    ] {
        let mut properties=json!({"server":server,"path":{"type":"string","minLength":1,"maxLength":4096},"expected_sha256":sha,
            "line":{"type":"integer","minimum":0,"maximum":2147483647},"character":{"type":"integer","minimum":0,"maximum":2147483647},
            "offset":{"type":"integer","minimum":0,"maximum":10000},"limit":{"type":"integer","minimum":1,"maximum":200},"expected_result_sha256":sha});
        let mut required=vec!["server","path","line","character","expected_sha256"];
    if matches!(alias,"lsp_code_actions"|"lsp_prepare_code_action") {
            properties["end_line"]=json!({"type":"integer","minimum":0,"maximum":2147483647});properties["end_character"]=json!({"type":"integer","minimum":0,"maximum":2147483647});
            properties["diagnostics"]=json!({"type":"array","maxItems":100,"items":{"type":"object"}});
        }
        if alias=="lsp_prepare_code_action" {properties["action_index"]=json!({"type":"integer","minimum":0,"maximum":9999});required.extend(["action_index","expected_result_sha256"]);}
        result.push(crate::tool_spec::ToolSpec::from((alias.into(),description.into(),schema(properties,&required))));
    }
    let command_schema = schema(
        json!({"server":server,"path":{"type":"string","minLength":1,"maxLength":4096},"expected_sha256":sha,"command":{"type":"string","minLength":1,"maxLength":256},"arguments":{"type":"array","maxItems":64}}),
        &["server", "path", "expected_sha256", "command"],
    );
    let mut command_schema = command_schema;
    command_schema["x-locallm-always-ask"] = json!(true);
    result.push(crate::tool_spec::ToolSpec::from(("lsp_execute_command".into(),"Execute a user-allowlisted, server-advertised LSP command after explicit approval. Commands may directly modify files or have other side effects. Client-mediated edits are staged as proposals requiring separate review/application; never automatically replay an uncertain command.".into(),command_schema)));
    let apply = result
        .iter()
        .find(|x| x.name == "lsp_rename")
        .unwrap()
        .input
        .clone();
    result.push(crate::tool_spec::ToolSpec::from(("lsp_apply_edit".into(),"Apply a reviewed semantic-edit proposal with ownership, settings and file-hash checks through recoverable workspace transactions. Does not execute server commands.".into(),apply)));
    result
}

#[allow(clippy::too_many_arguments)]
pub async fn execute(
    manager: &crate::lsp_client::Manager,
    db_path: &Path,
    owner: &str,
    run_id: &str,
    root: &str,
    alias: &str,
    args: Value,
    offered_always_ask: bool,
) -> Result<Value, String> {
    let schema = registry()
        .into_iter()
        .find(|spec| spec.name == alias)
        .ok_or("Unknown LSP tool.")?
        .input;
    crate::tool_validation::validate(&schema, &args)?;
    let revision = manager.revision();
    let config = {
        let store = crate::store::Store::open(db_path)?;
        if let Some(reason) =
            crate::capabilities::ToolPolicy::resolve(&store).denial_for_alias(alias)
        {
            return Err(reason);
        }
        crate::lsp_client::load(&store)?
    };
    if config.always_ask && !offered_always_ask {
        return Err("Language server approval settings changed. Start a new turn to obtain explicit approval; no server or rename was started.".into());
    }
    let id = args["server"]
        .as_str()
        .ok_or("Missing language server ID.")?;
    let server = config.server(id)?;
    if matches!(alias, "lsp_rename" | "lsp_apply_edit") {
        let request: Apply = serde_json::from_value(args).map_err(|e| e.to_string())?;
        let artifact = crate::store::Store::open(db_path)?
            .artifact(&request.proposal_id)?
            .ok_or("Unknown rename proposal.")?;
        if artifact.conversation_id != owner
            || artifact.tool_name != "lsp_rename_proposal"
            || !artifact
                .sha256
                .eq_ignore_ascii_case(&request.proposal_sha256)
            || hash(artifact.content.as_bytes()) != artifact.sha256
        {
            return Err("Rename proposal ownership/hash mismatch.".into());
        }
        let proposal: Proposal =
            serde_json::from_str(&artifact.content).map_err(|e| e.to_string())?;
        if proposal.version != 1
            || proposal.server != request.server
            || proposal.server_fingerprint != crate::lsp_client::fingerprint(server)
            || proposal.root != std::fs::canonicalize(root).map_err(|e| e.to_string())?
        {
            return Err("Rename proposal belongs to different workspace/settings; prepare a fresh proposal.".into());
        }
        let required = proposal
            .annotations
            .iter()
            .filter(|v| v["needsConfirmation"] == true)
            .filter_map(|v| v["id"].as_str())
            .collect::<Vec<_>>();
        if required
            .iter()
            .any(|id| !request.confirmed_annotations.iter().any(|v| v == id))
        {
            return Err("This proposal requires explicit change-annotation confirmation before application.".into());
        }
        let root = root.to_string();
        let value = tokio::task::spawn_blocking(move || {
            crate::workspace::Workspace::open(&root)?.apply_language_resources(
                proposal
                    .files
                    .into_iter()
                    .map(|f| (f.path, f.expected_sha256, f.content))
                    .collect(),
                vec![(proposal.source_path, proposal.source_sha256)],
            )
        })
        .await
        .map_err(|e| format!("Rename transaction failed: {e}"))??;
        return Ok(crate::tool_envelope::normalize(
            value,
            crate::tool_envelope::Family::Workspace,
        ));
    }
    let request: Query = serde_json::from_value(args).map_err(|e| e.to_string())?;
    let text = source(root, &request.path).await?;
    let sha = hash(text.as_bytes());
    if request
        .expected_sha256
        .as_ref()
        .is_some_and(|s| !s.eq_ignore_ascii_case(&sha))
    {
        return Err(
            "Document hash changed; read the current source before using its positions.".into(),
        );
    }
    let canonical = std::fs::canonicalize(root).map_err(|e| e.to_string())?;
    let uri = reqwest::Url::from_file_path(canonical.join(&request.path))
        .map_err(|_| "Invalid document URI.")?
        .to_string();
    let mut params = json!({"textDocument":{"uri":uri}});
    if let (Some(line), Some(character)) = (request.line, request.character) {
        let position = Position { line, character };
        position_offset(&text, &position)?;
        params["position"] = serde_json::to_value(position).map_err(|e| e.to_string())?;
    }
    let method = match alias {
        "lsp_execute_command" => {
            let command = request.command.as_deref().ok_or("Missing command.")?;
            if !server.allowed_commands.iter().any(|v| v == command) {
                return Err("LSP command is not user-allowlisted.".into());
            }
            params = json!({"command":command,"arguments":request.arguments.clone().unwrap_or_default()});
            "workspace/executeCommand"
        }
        "lsp_definition" => "textDocument/definition",
        "lsp_hover" => "textDocument/hover",
        "lsp_implementations" => "textDocument/implementation",
        "lsp_code_actions" | "lsp_prepare_code_action" => {
            let p = params["position"].clone();
            let end = match (request.end_line, request.end_character) {
                (Some(line), Some(character)) => json!({"line":line,"character":character}),
                (None, None) => p.clone(),
                _ => {
                    return Err(
                        "Provide both end_line and end_character for an action selection.".into(),
                    )
                }
            };
            let range = json!({"start":p,"end":end});
            checked_range(&range, Some(&text))?;
            let diagnostics = request.diagnostics.clone().unwrap_or_default();
            checked_items("lsp_diagnostics", &diagnostics, &text, &uri)?;
            params["range"] = range;
            params["context"] = json!({"diagnostics":diagnostics,"triggerKind":1});
            "textDocument/codeAction"
        }
        "lsp_diagnostics" => "__diagnostics",
        "lsp_symbols" => "textDocument/documentSymbol",
        "lsp_references" => {
            params["context"] =
                json!({"includeDeclaration":request.include_declaration.unwrap_or(true)});
            "textDocument/references"
        }
        "lsp_prepare_rename" => {
            let name = request
                .new_name
                .as_deref()
                .ok_or("Missing new symbol name.")?;
            if name.chars().any(char::is_control) {
                return Err("New symbol name contains control characters.".into());
            }
            params["newName"] = json!(name);
            "textDocument/rename"
        }
        _ => unreachable!(),
    };
    let mut reply = manager
        .request_guarded(
            owner,
            root,
            server,
            Some((&uri, &text, &sha)),
            method,
            params,
            revision.clone(),
        )
        .await?;
    if hash(source(root, &request.path).await?.as_bytes()) != sha {
        return Err(
            "Document changed during language server analysis; results were not published.".into(),
        );
    }
    if alias == "lsp_execute_command" {
        let mut edits = reply.staged_edits.clone();
        if reply.result.get("changes").is_some() || reply.result.get("documentChanges").is_some() {
            edits.push(reply.result.clone());
        }
        if edits.is_empty() {
            return Ok(
                json!({"commandExecuted":true,"sideEffects":"unknown","result":reply.result,"message":"Command completed without a staged edit. It may have changed files directly; inspect the workspace before continuing."}),
            );
        }
        let mut operations = Vec::new();
        let mut annotations = serde_json::Map::new();
        for edit in edits {
            if let Some(items) = edit["documentChanges"].as_array() {
                operations.extend(items.clone());
            }
            if let Some(changes) = edit["changes"].as_object() {
                for (uri, edits) in changes {
                    operations
                        .push(json!({"textDocument":{"uri":uri,"version":null},"edits":edits}));
                }
            }
            if let Some(items) = edit["changeAnnotations"].as_object() {
                for (key, value) in items {
                    if annotations.insert(key.clone(), value.clone()).is_some() {
                        return Err("Command returned conflicting annotation IDs; side effects are unknown.".into());
                    }
                }
            }
        }
        reply.result = json!({"documentChanges":operations,"changeAnnotations":annotations});
    }
    if matches!(alias, "lsp_code_actions" | "lsp_prepare_code_action") {
        let actions = reply.result.as_array().cloned().unwrap_or_default();
        if !reply.result.is_null() && !reply.result.is_array() {
            return Err("Invalid code action result.".into());
        }
        if actions.len() > 10000 || actions.iter().any(|a| !a["title"].is_string()) {
            return Err("Code actions must be a bounded list with text titles.".into());
        }
        let pin=hash(serde_json::to_vec(&json!({"server":crate::lsp_client::fingerprint(server),"root":canonical,"path":request.path,"source":sha,"line":request.line,"character":request.character,"endLine":request.end_line,"endCharacter":request.end_character,"diagnostics":request.diagnostics,"actions":actions})).map_err(|e|e.to_string())?.as_slice());
        if request
            .expected_result_sha256
            .as_ref()
            .is_some_and(|v| v != &pin)
        {
            return Err("Code actions changed; refresh the action list before selection.".into());
        }
        if alias == "lsp_code_actions" {
            let offset = request.offset.unwrap_or(0);
            let limit = request.limit.unwrap_or(50);
            return Ok(
                json!({"sourceSha256":sha,"resultSha256":pin,"total":actions.len(),"hasMore":offset+limit<actions.len(),"items":actions.iter().enumerate().skip(offset).take(limit).map(|(i,a)|json!({"index":i,"title":a["title"],"kind":a["kind"],"disabled":a["disabled"],"hasEdit":a.get("edit").is_some(),"hasCommand":a.get("command").is_some()})).collect::<Vec<_>>(),"untrustedServerOutput":true}),
            );
        }
        let mut action = actions
            .get(request.action_index.ok_or("Missing action index.")?)
            .cloned()
            .ok_or("Action index outside result.")?;
        if action.get("disabled").is_some() {
            return Err("Selected code action is disabled.".into());
        }
        if action.get("edit").is_none()
            && action.get("data").is_some()
            && reply.capabilities["codeActionProvider"]["resolveProvider"] == true
        {
            let resolved = manager
                .request_guarded(
                    owner,
                    root,
                    server,
                    Some((&uri, &text, &sha)),
                    "codeAction/resolve",
                    action,
                    revision.clone(),
                )
                .await?;
            action = resolved.result;
            reply.documents = resolved.documents;
        }
        if action.get("command").is_some() {
            return Err(
                "Code action requires a server command; no edit or command was executed.".into(),
            );
        }
        reply.result = action
            .get("edit")
            .cloned()
            .ok_or("Selected action has no supported text edit.")?;
    }
    if matches!(
        alias,
        "lsp_prepare_rename" | "lsp_prepare_code_action" | "lsp_execute_command"
    ) {
        let proposal = prepare(root, server, &request, &sha, reply).await?;
        if proposal.files.is_empty() {
            return Ok(json!({"noChanges":true,"fileCount":0,"sourceSha256":sha,"isError":false}));
        }
        let content = serde_json::to_string(&proposal).map_err(|e| e.to_string())?;
        if content.len() > 16 * 1024 * 1024 {
            return Err("Rename proposal exceeds16MiB; no files were changed.".into());
        }
        let artifact = crate::artifacts::ArtifactRecord {
            id: format!("lsp_rename_{}", uuid::Uuid::new_v4().simple()),
            conversation_id: owner.into(),
            run_id: Some(run_id.into()),
            tool_name: "lsp_rename_proposal".into(),
            mime_type: "application/json".into(),
            size_bytes: content.len(),
            sha256: hash(content.as_bytes()),
            content,
            created_at: crate::store::now(),
        };
        crate::store::Store::open(db_path)?.save_artifact(&artifact)?;
        return Ok(
            json!({"server":server.id,"proposalId":artifact.id,"proposalSha256":artifact.sha256,"sourceSha256":sha,"fileCount":proposal.files.len(),"annotations":proposal.annotations,"isError":false,
            "reviewIncomplete":proposal.files.iter().any(|f|f.diff.len()>16384),
            "files":proposal.files.iter().map(|f|{let mut end=f.diff.len().min(16384);while !f.diff.is_char_boundary(end){end-=1;}json!({"path":f.path,"beforeSha256":f.expected_sha256,"sha256":f.sha256,"diff":&f.diff[..end],"diffTruncated":end<f.diff.len()})}).collect::<Vec<_>>(),
            "message":"No project files changed. Review this proposal (artifact_read) before lsp_apply_edit or lsp_rename; current file hashes are rechecked together by native transaction preflight. Full diffs and proposed content are in the proposal artifact."}),
        );
    }
    if alias == "lsp_hover" {
        return Ok(json!({"sourceSha256":sha,"hover":reply.result,"untrustedServerOutput":true}));
    }
    if matches!(alias, "lsp_definition" | "lsp_implementations") {
        let locations = match reply.result {
            Value::Null => vec![],
            Value::Array(v) => v,
            v if v.is_object() => vec![v],
            _ => return Err("Invalid location result.".into()),
        };
        reply.result = Value::Array(
            locations
                .into_iter()
                .map(|v| {
                    if v.get("targetUri").is_some() {
                        json!({"uri":v["targetUri"],"range":v["targetSelectionRange"]})
                    } else {
                        v
                    }
                })
                .collect(),
        );
    }
    let mut incomplete = false;
    let mut omitted = 0;
    let mut items = if alias == "lsp_diagnostics" {
        incomplete = reply.result["source"] == "publishDiagnostics"
            && reply.result["versionVerified"] != true;
        reply.result["items"]
            .as_array()
            .ok_or("Invalid diagnostic items.")?
            .clone()
    } else {
        match reply.result {
            Value::Null => Vec::new(),
            Value::Array(items) => items,
            _ => return Err("Language server returned a non-array result.".into()),
        }
    };
    if items.len() > 10000 {
        return Err("Language server result exceeds10000 entries.".into());
    }
    checked_items(alias, &items, &text, &uri)?;
    if matches!(
        alias,
        "lsp_references" | "lsp_definition" | "lsp_implementations"
    ) {
        items.retain_mut(|item| {
            let uri = item["uri"].as_str().unwrap_or("");
            match uri_path(&canonical, uri) {
                Ok(path) => {
                    item["path"] = json!(path);
                    true
                }
                Err(_) => {
                    omitted += 1;
                    false
                }
            }
        });
    }
    items.sort_by_cached_key(Value::to_string);
    let result_sha=hash(serde_json::to_vec(&json!({"server":crate::lsp_client::fingerprint(server),"root":canonical,"path":request.path,"sourceSha256":sha,
        "method":method,"line":request.line,"character":request.character,"includeDeclaration":request.include_declaration.unwrap_or(true),"items":items})).map_err(|e|e.to_string())?.as_slice());
    if request
        .expected_result_sha256
        .as_ref()
        .is_some_and(|pin| !pin.eq_ignore_ascii_case(&result_sha))
    {
        return Err("Language server result set changed; refresh its hash before paging.".into());
    }
    let offset = request.offset.unwrap_or(0);
    let limit = request.limit.unwrap_or(50);
    let total = items.len();
    Ok(
        json!({"server":request.server,"path":request.path,"sourceSha256":sha,"resultSha256":result_sha,"offset":offset,"limit":limit,"total":total,
        "hasMore":offset.saturating_add(limit)<total,"nextOffset":if offset.saturating_add(limit)<total{Some(offset+limit)}else{None},
        "items":items.into_iter().skip(offset).take(limit).collect::<Vec<_>>(),"omittedExternalLocations":omitted,"incomplete":incomplete,
        "clean":if alias=="lsp_diagnostics"&&!incomplete{Some(total==0)}else{None},"isError":false,"untrustedServerOutput":true}),
    )
}

pub fn configure_tool(
    tool: &mut crate::connectors::AgentTool,
    config: &crate::lsp_client::Config,
) -> bool {
    if !config.enabled {
        return false;
    }
    let ids = config
        .servers
        .iter()
        .filter(|s| s.enabled && crate::execution::executable(&s.command))
        .map(|s| s.id.clone())
        .collect::<Vec<_>>();
    if ids.is_empty() {
        return false;
    }
    tool.tool.input_schema["properties"]["server"]["enum"] = json!(ids);
    if tool.alias == "lsp_execute_command" {
        tool.tool.input_schema["properties"]["command"]["description"] = json!(format!(
            "User-allowlisted commands by server: {}",
            serde_json::to_string(
                &config
                    .servers
                    .iter()
                    .filter(|s| s.enabled)
                    .map(|s| json!({"server":s.id,"commands":s.allowed_commands}))
                    .collect::<Vec<_>>()
            )
            .unwrap_or_default()
        ));
    }
    if config.always_ask {
        tool.tool.input_schema["x-locallm-always-ask"] = json!(true);
    }
    true
}
#[tauri::command]
pub fn get_lsp_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<crate::lsp_client::Config, String> {
    crate::lsp_client::load(&*state.database()?)
}
#[tauri::command]
pub async fn save_lsp_config(
    state: tauri::State<'_, crate::AppState>,
    config: crate::lsp_client::Config,
) -> Result<(), String> {
    config.validate()?;
    state
        .database()?
        .save_setting(crate::lsp_client::KEY, &config)?;
    state.language_servers.shutdown().await;
    Ok(())
}
#[tauri::command]
pub async fn test_lsp_server(
    state: tauri::State<'_, crate::AppState>,
    server: String,
    root: String,
) -> Result<Value, String> {
    let root = crate::workspace_ui::bound_root(&state, &root)?;
    let config = {
        let store = state.database()?;
        if let Some(reason) = crate::capabilities::ToolPolicy::resolve(&store).denial("lsp_tools") {
            return Err(reason);
        }
        crate::lsp_client::load(&store)?
    };
    let configured = config.server(&server)?;
    let reply = state
        .language_servers
        .request(
            "settings-probe",
            &root,
            configured,
            None,
            "__initialize",
            json!({}),
        )
        .await?;
    Ok(
        json!({"server":server,"ready":true,"capabilities":reply.capabilities,"message":"Language server initialized."}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        dir: tempfile::TempDir,
        state: crate::AppState,
        owner: String,
        config: crate::lsp_client::Config,
        log: PathBuf,
    }
    impl Fixture {
        fn new(mode: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let state = crate::test_support::app_state(dir.path());
            crate::test_support::select_fixture_model(&state, dir.path());
            let owner = state.database().unwrap().create_conversation().unwrap().id;
            let python = crate::execution::ExecutionConfig::default().python_path;
            assert!(
                !python.is_empty(),
                "The standard-library LSP fixture requires the already installed Python runtime."
            );
            let log = dir.path().join("server-events.jsonl");
            let server = crate::lsp_client::ServerConfig {
                id: "fixture".into(),
                language_id: "rust".into(),
                command: python,
                allowed_commands: vec![],
                enabled: true,
                arguments: vec![
                    format!(
                        "{}/tests/fixtures/lsp_fixture.py",
                        env!("CARGO_MANIFEST_DIR")
                    ),
                    mode.into(),
                    log.to_string_lossy().into_owned(),
                ],
                environment: BTreeMap::new(),
                initialization_options: Value::Null,
                settings: json!({"fixture":{"answer":42}}),
                timeout_seconds: 2,
            };
            let config = crate::lsp_client::Config {
                enabled: true,
                always_ask: false,
                servers: vec![server],
            };
            config.validate().unwrap();
            state
                .database()
                .unwrap()
                .save_setting(crate::lsp_client::KEY, &config)
                .unwrap();
            std::fs::write(dir.path().join("main.rs"), "😀 old_name = old_name;\r\n").unwrap();
            std::fs::write(dir.path().join("other.rs"), "old_name();\n").unwrap();
            Self {
                dir,
                state,
                owner,
                config,
                log,
            }
        }
        fn args(&self) -> Value {
            json!({"server":"fixture","path":"main.rs"})
        }
        fn rename_args(&self) -> Value {
            json!({"server":"fixture","path":"main.rs","line":0,"character":3,"new_name":"renamed","expected_sha256":hash(&std::fs::read(self.dir.path().join("main.rs")).unwrap())})
        }
        async fn call(&self, alias: &str, args: Value) -> Result<Value, String> {
            execute(
                &self.state.language_servers,
                &self.state.db_path,
                &self.owner,
                "fixture-run",
                self.dir.path().to_str().unwrap(),
                alias,
                args,
                false,
            )
            .await
        }
        async fn close(&self) {
            self.state.language_servers.shutdown().await;
        }
        fn events(&self) -> Vec<Value> {
            std::fs::read_to_string(&self.log)
                .unwrap_or_default()
                .lines()
                .map(|l| serde_json::from_str(l).unwrap())
                .collect()
        }
    }
    #[tokio::test]
    async fn semantic_actions_are_pinned_resolved_reviewable_and_transactional() {
        let f = Fixture::new("full");
        let mut args = f.rename_args();
        args.as_object_mut().unwrap().remove("new_name");
        for alias in ["lsp_definition", "lsp_hover", "lsp_implementations"] {
            let r = f.call(alias, args.clone()).await.unwrap();
            assert!(r["sourceSha256"].is_string());
        }
        let list = f.call("lsp_code_actions", args.clone()).await.unwrap();
        assert_eq!(list["total"], 3);
        let mut selected = args.clone();
        selected["action_index"] = json!(0);
        selected["expected_result_sha256"] = list["resultSha256"].clone();
        let mut stale = selected.clone();
        stale["expected_result_sha256"] = json!("0".repeat(64));
        assert!(f.call("lsp_prepare_code_action", stale).await.is_err());
        let mut command = selected.clone();
        command["action_index"] = json!(1);
        assert!(f
            .call("lsp_prepare_code_action", command)
            .await
            .unwrap_err()
            .contains("server command"));
        let mut deferred = selected.clone();
        deferred["action_index"] = json!(2);
        let resolved = f.call("lsp_prepare_code_action", deferred).await.unwrap();
        assert!(resolved["proposalId"].is_string());
        let preview = f.call("lsp_prepare_code_action", selected).await.unwrap();
        assert!(std::fs::read_to_string(f.dir.path().join("main.rs"))
            .unwrap()
            .contains("old_name"));
        f.call("lsp_apply_edit",json!({"server":"fixture","proposal_id":preview["proposalId"],"proposal_sha256":preview["proposalSha256"]})).await.unwrap();
        assert!(std::fs::read_to_string(f.dir.path().join("main.rs"))
            .unwrap()
            .contains("action_name"));
        f.close().await;
    }
    #[test]
    fn utf16_ranges_line_endings_insert_order_overlap_and_boundaries() {
        let text = "😀 old\r\nx\ry\n";
        assert_eq!(
            position_offset(
                text,
                &Position {
                    line: 0,
                    character: 3
                }
            )
            .unwrap(),
            5
        );
        assert!(position_offset(
            text,
            &Position {
                line: 0,
                character: 1
            }
        )
        .is_err());
        assert_eq!(
            position_offset(
                text,
                &Position {
                    line: 1,
                    character: 99
                }
            )
            .unwrap(),
            11
        );
        assert_eq!(
            position_offset(
                text,
                &Position {
                    line: 2,
                    character: 0
                }
            )
            .unwrap(),
            12
        );
        let edited=edited_text("ab",&[
            json!({"range":{"start":{"line":0,"character":1},"end":{"line":0,"character":1}},"newText":"X"}),
            json!({"range":{"start":{"line":0,"character":1},"end":{"line":0,"character":1}},"newText":"Y"}),
        ]).unwrap();
        assert_eq!(edited, "aXYb");
        assert!(edited_text("abc",&[
            json!({"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":2}},"newText":"x"}),
            json!({"range":{"start":{"line":0,"character":1},"end":{"line":0,"character":3}},"newText":"y"}),
        ]).is_err());
    }
    #[tokio::test]
    async fn protocol_navigation_pooling_configuration_and_server_edit_refusal() {
        let f = Fixture::new("configuration");
        let diagnostics = f.call("lsp_diagnostics", f.args()).await.unwrap();
        assert_eq!(diagnostics["clean"], true);
        let symbols = f.call("lsp_symbols", f.args()).await.unwrap();
        assert_eq!(symbols["items"][0]["name"], "old_name");
        let refs = f
            .call(
                "lsp_references",
                json!({"server":"fixture","path":"main.rs","line":0,"character":3,"limit":1}),
            )
            .await
            .unwrap();
        assert_eq!(refs["total"], 1);
        assert_eq!(refs["omittedExternalLocations"], 1);
        let events = f.events();
        assert_eq!(
            events.iter().filter(|e| e["method"] == "started").count(),
            1,
            "reuse the configured session"
        );
        assert_eq!(
            events
                .iter()
                .find(|e| e["method"] == "configuration_result")
                .unwrap()["result"],
            json!([42])
        );
        assert_eq!(
            events
                .iter()
                .find(|e| e["method"] == "apply_result")
                .unwrap()["result"]["applied"],
            false
        );
        let mut pin = f.args();
        pin["expected_result_sha256"] = json!("0".repeat(64));
        assert!(f
            .call("lsp_symbols", pin)
            .await
            .unwrap_err()
            .contains("result set changed"));
        f.close().await;
    }
    #[tokio::test]
    async fn real_workspace_rename_is_previewed_pinned_transactional_and_not_replayed() {
        let f = Fixture::new("full");
        let preview = f.call("lsp_prepare_rename", f.rename_args()).await.unwrap();
        assert_eq!(preview["fileCount"], 2);
        assert!(std::fs::read_to_string(f.dir.path().join("main.rs"))
            .unwrap()
            .contains("old_name"));
        let apply = json!({"server":"fixture","proposal_id":preview["proposalId"],"proposal_sha256":preview["proposalSha256"]});
        std::fs::write(f.dir.path().join("other.rs"), "user changed this\n").unwrap();
        assert!(f.call("lsp_rename", apply.clone()).await.is_err());
        assert!(std::fs::read_to_string(f.dir.path().join("main.rs"))
            .unwrap()
            .contains("old_name"));
        assert_eq!(
            std::fs::read_to_string(f.dir.path().join("other.rs")).unwrap(),
            "user changed this\n"
        );
        let fresh = f.call("lsp_prepare_rename", f.rename_args()).await.unwrap();
        let apply = json!({"server":"fixture","proposal_id":fresh["proposalId"],"proposal_sha256":fresh["proposalSha256"]});
        let committed = f.call("lsp_rename", apply.clone()).await.unwrap();
        assert_eq!(committed["status"], "success", "{committed}");
        assert_eq!(
            std::fs::read_to_string(f.dir.path().join("main.rs")).unwrap(),
            "😀 renamed = renamed;\r\n"
        );
        assert!(
            f.call("lsp_rename", apply).await.is_err(),
            "never silently replay a completed rename"
        );
        f.close().await;
    }
    #[tokio::test]
    async fn resource_deletion_is_only_applied_after_review() {
        let f = Fixture::new("resource");
        let proposal = f.call("lsp_prepare_rename", f.rename_args()).await.unwrap();
        assert!(f.dir.path().join("main.rs").exists());
        let result=f.call("lsp_apply_edit",json!({"server":"fixture","proposal_id":proposal["proposalId"],"proposal_sha256":proposal["proposalSha256"]})).await.unwrap();
        assert_eq!(result["status"], "success");
        assert!(!f.dir.path().join("main.rs").exists());
        f.close().await;
    }
    #[tokio::test]
    async fn allowlisted_commands_stage_server_edits_without_writing() {
        let mut f = Fixture::new("normal");
        f.config.servers[0].allowed_commands = vec!["fixture.refactor".into()];
        f.state
            .database()
            .unwrap()
            .save_setting(crate::lsp_client::KEY, &f.config)
            .unwrap();
        let mut args = f.args();
        args["command"] = json!("fixture.refactor");
        args["expected_sha256"] =
            json!(hash(&std::fs::read(f.dir.path().join("main.rs")).unwrap()));
        let proposal = f.call("lsp_execute_command", args).await.unwrap();
        assert!(proposal["proposalId"].is_string());
        assert!(std::fs::read_to_string(f.dir.path().join("main.rs"))
            .unwrap()
            .contains("old_name"));
        let result=f.call("lsp_apply_edit",json!({"server":"fixture","proposal_id":proposal["proposalId"],"proposal_sha256":proposal["proposalSha256"]})).await.unwrap();
        assert_eq!(result["status"], "success");
        assert!(std::fs::read_to_string(f.dir.path().join("main.rs"))
            .unwrap()
            .contains("command_name"));
        f.close().await;
        assert!(!crate::subagents::harness_detached_supported(
            "lsp_execute_command"
        ));
    }
    #[tokio::test]
    async fn missing_stale_or_malformed_server_evidence_never_claims_clean_or_applies_edits() {
        for mode in [
            "bad_header",
            "utf8",
            "unsupported",
            "wrong_version",
            "outside",
        ] {
            let f = Fixture::new(mode);
            let result = if mode == "unsupported" {
                f.call("lsp_symbols", f.args()).await
            } else {
                f.call("lsp_prepare_rename", f.rename_args()).await
            };
            assert!(result.is_err(), "{mode}: {result:?}");
            assert!(std::fs::read_to_string(f.dir.path().join("main.rs"))
                .unwrap()
                .contains("old_name"));
            f.close().await;
        }
        let push = Fixture::new("push");
        assert_eq!(
            push.call("lsp_diagnostics", push.args()).await.unwrap()["clean"],
            true
        );
        assert_eq!(
            push.call("lsp_diagnostics", push.args()).await.unwrap()["clean"],
            true
        );
        assert_eq!(
            push.events()
                .iter()
                .filter(|e| e["method"] == "opened")
                .count(),
            2,
            "legacy diagnostics must refresh rather than reuse a cached clean report"
        );
        push.close().await;
        let unversioned = Fixture::new("unversioned");
        let data = unversioned
            .call("lsp_diagnostics", unversioned.args())
            .await
            .unwrap();
        assert!(data["clean"].is_null());
        assert_eq!(data["incomplete"], true);
        unversioned.close().await;
        let stale = Fixture::new("stale");
        assert!(stale.call("lsp_diagnostics", stale.args()).await.is_err());
        stale.close().await;
    }

    #[tokio::test]
    async fn malformed_protocol_records_and_transferred_page_pins_are_rejected() {
        for (mode, alias) in [
            ("bad_diagnostic", "lsp_diagnostics"),
            ("bad_symbol", "lsp_symbols"),
            ("bad_reference", "lsp_references"),
        ] {
            let f = Fixture::new(mode);
            let mut args = f.args();
            if alias == "lsp_references" {
                args["line"] = json!(0);
                args["character"] = json!(3);
            }
            assert!(
                f.call(alias, args).await.is_err(),
                "{mode} must fail loudly rather than become empty/clean evidence"
            );
            f.close().await;
        }
        let f = Fixture::new("full");
        let a = f
            .call(
                "lsp_references",
                json!({"server":"fixture","path":"main.rs","line":0,"character":3}),
            )
            .await
            .unwrap();
        let changed=f.call("lsp_references",json!({"server":"fixture","path":"main.rs","line":0,"character":3,"include_declaration":false,"expected_result_sha256":a["resultSha256"]})).await;
        assert!(
            changed.unwrap_err().contains("result set changed"),
            "pins bind the request, not just identical response items"
        );
        f.close().await;
    }

    #[tokio::test]
    async fn oversized_preview_retains_exact_usable_proposal_and_full_review_bytes() {
        let f = Fixture::new("full");
        let before = "old_name();\r\n".repeat(4000);
        std::fs::write(f.dir.path().join("other.rs"), &before).unwrap();
        let preview = f.call("lsp_prepare_rename", f.rename_args()).await;
        // The server fixture correctly returns >1024 edits for that file; bounds
        // reject the entire proposal instead of silently applying a prefix.
        assert!(preview.unwrap_err().contains("1024 edits"));
        let before = format!("old_name();\r\n{}", "unchanged long content ".repeat(3000));
        std::fs::write(f.dir.path().join("other.rs"), &before).unwrap();
        let preview = f.call("lsp_prepare_rename", f.rename_args()).await.unwrap();
        assert_eq!(preview["reviewIncomplete"], true);
        assert!(preview["files"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["diffTruncated"] == true));
        let native =
            crate::tool_envelope::normalize(preview.clone(), crate::tool_envelope::Family::Harness);
        let (bounded, saved) =
            crate::artifacts::bound_tool_result(native, "lsp_prepare_rename", &f.owner, None, 512);
        assert_eq!(bounded["data"]["proposalId"], preview["proposalId"]);
        assert_eq!(bounded["data"]["proposalSha256"], preview["proposalSha256"]);
        assert_eq!(bounded["data"]["reviewIncomplete"], true);
        assert!(bounded.to_string().len() <= 512);
        assert!(saved.is_some());
        let artifact = f
            .state
            .database()
            .unwrap()
            .artifact(preview["proposalId"].as_str().unwrap())
            .unwrap()
            .unwrap();
        let proposal: Proposal = serde_json::from_str(&artifact.content).unwrap();
        let other = proposal
            .files
            .iter()
            .find(|f| f.path == "other.rs")
            .unwrap();
        assert!(other.diff.len() > 16384);
        assert!(other.diff.contains("No newline at end of file"));
        assert!(other.diff.ends_with("\n"));
        let applied=f.call("lsp_rename",json!({"server":"fixture","proposal_id":bounded["data"]["proposalId"],"proposal_sha256":bounded["data"]["proposalSha256"]})).await.unwrap();
        assert_eq!(applied["status"], "success");
        assert_eq!(
            std::fs::read_to_string(f.dir.path().join("other.rs")).unwrap(),
            before.replacen("old_name", "renamed", 1)
        );
        f.close().await;
    }

    #[tokio::test]
    async fn actual_catalog_schema_source_approval_capabilities_and_ownership_are_enforced() {
        let mut f = Fixture::new("full");
        crate::test_support::select_fixture_model(&f.state, f.dir.path());
        let plan = crate::chat::assemble_turn(
            &f.state,
            &f.owner,
            "Navigate code",
            vec!["__workspace".into()],
            Some(vec![]),
            Some(crate::presets::STANDARD),
            None,
        )
        .await
        .unwrap();
        for alias in ALIASES {
            let tool = plan.tools.iter().find(|t| t.alias == alias).unwrap();
            assert_eq!(
                crate::capabilities::tool_capability(tool),
                Some("lsp_tools")
            );
            assert!(!tool.concurrent_read());
            assert_eq!(tool.tool.input_schema["additionalProperties"], false);
            assert_eq!(
                tool.tool.input_schema["properties"]["server"]["enum"],
                json!(["fixture"])
            );
            assert!(tool
                .prepare_arguments(
                    json!({"server":"fixture","path":"main.rs","command":"malicious"})
                )
                .is_err());
            assert!(!crate::presets::harness_source_allowed(alias, false, true));
        }
        let selected = plan
            .tools
            .iter()
            .find(|t| t.alias == "lsp_symbols")
            .unwrap();
        assert!(selected.trusted_read());
        let mut calls = crate::tool_calls::ToolCalls::default();
        calls.push(&json!({"tool_calls":[{"index":0,"id":"lsp-model-call","type":"function","function":{"name":"lsp_symbols","arguments":f.args().to_string()}}]})).unwrap();
        let call = calls.finish().unwrap().remove(0);
        let aliases = plan
            .tools
            .iter()
            .map(|t| t.alias.as_str())
            .collect::<Vec<_>>();
        let ctx = crate::harness::HarnessCtx {
            state: &f.state,
            snapshot: crate::test_support::snapshot(&f.state, f.dir.path(), &aliases),
            conversation_id: f.owner.clone(),
            run_id: "lsp-model-run".into(),
            step_id: "lsp-step".into(),
            access_mode: crate::permissions::AccessMode::FullAccess,
            depth: 0,
            preset_id: crate::presets::STANDARD.into(),
            inherit_tools: &plan.tools,
            emit: std::sync::Arc::new(|_, _| {}),
        };
        let result = crate::harness::execute(
            &ctx,
            &call.name,
            selected.prepare_arguments(call.arguments).unwrap(),
        )
        .await
        .unwrap()
        .value;
        assert_eq!(result["items"][0]["name"], "old_name");
        f.config.always_ask = true;
        f.state
            .database()
            .unwrap()
            .save_setting(crate::lsp_client::KEY, &f.config)
            .unwrap();
        assert!(crate::harness::execute(&ctx, "lsp_symbols", f.args())
            .await
            .err()
            .unwrap()
            .contains("approval settings changed"));
        let refreshed = crate::chat::assemble_turn(
            &f.state,
            &f.owner,
            "Navigate code",
            vec!["__workspace".into()],
            Some(vec![]),
            Some(crate::presets::STANDARD),
            None,
        )
        .await
        .unwrap();
        assert!(refreshed
            .tools
            .iter()
            .find(|t| t.alias == "lsp_symbols")
            .unwrap()
            .requires_approval(&f.args()));
        for capability in ["lsp_tools", "workspace_files", "local_execution"] {
            f.state
                .database()
                .unwrap()
                .save_setting(
                    crate::capabilities::DISABLED_KEY,
                    &vec![capability.to_string()],
                )
                .unwrap();
            assert!(
                crate::harness::execute(&ctx, "lsp_symbols", f.args())
                    .await
                    .err()
                    .unwrap()
                    .contains("disabled"),
                "{capability}"
            );
        }
        f.state
            .database()
            .unwrap()
            .save_setting(crate::capabilities::DISABLED_KEY, &Vec::<String>::new())
            .unwrap();
        f.config.always_ask = false;
        f.state
            .database()
            .unwrap()
            .save_setting(crate::lsp_client::KEY, &f.config)
            .unwrap();
        let preview = f.call("lsp_prepare_rename", f.rename_args()).await.unwrap();
        let another = f
            .state
            .database()
            .unwrap()
            .create_conversation()
            .unwrap()
            .id;
        let wrong=execute(&f.state.language_servers,&f.state.db_path,&another,"wrong-owner",f.dir.path().to_str().unwrap(),"lsp_rename",json!({"server":"fixture","proposal_id":preview["proposalId"],"proposal_sha256":preview["proposalSha256"]}),false).await;
        assert!(wrong.unwrap_err().contains("ownership"));
        let no_source = crate::chat::assemble_turn(
            &f.state,
            "",
            "No files",
            vec!["__execution".into()],
            Some(vec![]),
            Some(crate::presets::STANDARD),
            None,
        )
        .await
        .unwrap();
        assert!(!no_source.tools.iter().any(|t| owns(&t.alias)));
        f.close().await;
    }

    #[tokio::test]
    async fn missing_executable_and_changed_settings_remove_stale_authority_without_launch() {
        let mut f = Fixture::new("full");
        crate::test_support::select_fixture_model(&f.state, f.dir.path());
        f.config.servers[0].command = f
            .dir
            .path()
            .join("does-not-exist")
            .to_string_lossy()
            .into_owned();
        f.state
            .database()
            .unwrap()
            .save_setting(crate::lsp_client::KEY, &f.config)
            .unwrap();
        let plan = crate::chat::assemble_turn(
            &f.state,
            &f.owner,
            "Read code",
            vec!["__workspace".into()],
            Some(vec![]),
            Some(crate::presets::STANDARD),
            None,
        )
        .await
        .unwrap();
        assert!(!plan.tools.iter().any(|t| owns(&t.alias)));
        assert!(plan
            .tool_notice
            .as_deref()
            .unwrap_or("")
            .contains("Language server"));
        assert!(f
            .call("lsp_symbols", f.args())
            .await
            .unwrap_err()
            .contains("unavailable"));
        assert!(f.events().is_empty());
        let disabled = crate::lsp_client::Config::default();
        assert!(disabled.validate().is_ok());
        let revision = f.state.language_servers.revision();
        f.state.language_servers.clear();
        let mut server = f.config.servers[0].clone();
        server.command = crate::execution::ExecutionConfig::default().python_path;
        assert!(f
            .state
            .language_servers
            .request_guarded(
                &f.owner,
                f.dir.path().to_str().unwrap(),
                &server,
                None,
                "__initialize",
                json!({}),
                revision
            )
            .await
            .err()
            .unwrap()
            .contains("settings changed"));
        assert!(
            f.events().is_empty(),
            "a stale settings generation must not start the executable"
        );
        f.close().await;
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn timeout_stop_graceful_shutdown_and_descendant_cleanup_are_real() {
        let f = Fixture::new("descendant");
        f.call("lsp_symbols", f.args()).await.unwrap();
        let events = f.events();
        let parent = events.iter().find(|e| e["method"] == "started").unwrap()["pid"]
            .as_u64()
            .unwrap();
        let child = events.iter().find(|e| e["method"] == "descendant").unwrap()["child"]
            .as_u64()
            .unwrap();
        f.close().await;
        let alive = |pid: u64| {
            std::fs::read_to_string(format!("/proc/{pid}/stat"))
                .ok()
                .is_some_and(|s| {
                    s.rsplit_once(')')
                        .is_none_or(|(_, tail)| !tail.trim_start().starts_with('Z'))
                })
        };
        for _ in 0..100 {
            if !alive(parent) && !alive(child) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let lingering = alive(child);
        if lingering {
            let _ = std::process::Command::new("kill")
                .arg("-KILL")
                .arg(child.to_string())
                .status();
        }
        assert!(!alive(parent));
        assert!(
            !lingering,
            "the configured server's descendant escaped cleanup"
        );
        let events = f.events();
        assert!(events.iter().any(|e| e["method"] == "shutdown"));
        assert!(events.iter().any(|e| e["method"] == "exit"));
        let mut f = Fixture::new("hang");
        f.config.servers[0].timeout_seconds = 1;
        f.state
            .database()
            .unwrap()
            .save_setting(crate::lsp_client::KEY, &f.config)
            .unwrap();
        let args = json!({"server":"fixture","path":"main.rs","line":0,"character":3});
        let error = f.call("lsp_references", args.clone()).await.unwrap_err();
        assert!(error.contains("timed out"));
        let mut tools = ALIASES
            .iter()
            .map(|a| crate::connectors::AgentTool::harness(a).unwrap())
            .collect::<Vec<_>>();
        for tool in &mut tools {
            configure_tool(tool, &f.config);
        }
        let ctx = crate::harness::HarnessCtx {
            state: &f.state,
            snapshot: crate::test_support::snapshot(&f.state, f.dir.path(), &ALIASES),
            conversation_id: f.owner.clone(),
            run_id: "stop-lsp".into(),
            step_id: "stop-step".into(),
            access_mode: crate::permissions::AccessMode::FullAccess,
            depth: 0,
            preset_id: crate::presets::STANDARD.into(),
            inherit_tools: &tools,
            emit: std::sync::Arc::new(|_, _| {}),
        };
        let signal = f.state.cancel.clone();
        let stop = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            signal.send_replace(true);
        });
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(700),
            crate::harness::execute(&ctx, "lsp_references", args),
        )
        .await
        .expect("Stop must interrupt a pending LSP response");
        assert!(result.err().unwrap().contains("cancelled"));
        stop.await.unwrap();
        f.state.cancel.send_replace(false);
        f.close().await;
    }
}

#[cfg(test)]
mod acceptance {
    use super::*;
    async fn language(engine: &str, executable: &str) {
        let t = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(t.path());
        let owner = state.database().unwrap().create_conversation().unwrap().id;
        let (path, text, line, character, args) = if engine == "rust" {
            std::fs::create_dir_all(t.path().join("src")).unwrap();
            std::fs::write(
                t.path().join("Cargo.toml"),
                "[package]\nname='locallm_acceptance'\nversion='0.1.0'\nedition='2021'\n",
            )
            .unwrap();
            ("src/main.rs","fn add(a: i32, b: i32) -> i32 { a + b }\nfn main() { println!(\"{}\", add(1, 2)); }\n",0,3,vec![])
        } else {
            std::fs::write(
                t.path().join("tsconfig.json"),
                "{\"compilerOptions\":{\"strict\":true},\"files\":[\"index.ts\"]}",
            )
            .unwrap();
            ("index.ts","export function add(a: number, b: number) { return a + b; }\nconsole.log(add(1, 2));\n",0,16,{let mut args=Vec::new();if let Ok(script)=std::env::var("LOCALLM_TYPESCRIPT_LANGUAGE_SERVER_SCRIPT"){args.push(script);}args.push("--stdio".into());args})
        };
        std::fs::write(t.path().join(path), text).unwrap();
        let server = crate::lsp_client::ServerConfig {
            id: "acceptance".into(),
            language_id: engine.into(),
            enabled: true,
            command: executable.into(),
            arguments: args,
            environment: Default::default(),
            initialization_options: Value::Null,
            settings: json!({}),
            timeout_seconds: 60,
            allowed_commands: vec![],
        };
        let config = crate::lsp_client::Config {
            enabled: true,
            always_ask: false,
            servers: vec![server.clone()],
        };
        state
            .database()
            .unwrap()
            .save_setting(crate::lsp_client::KEY, &config)
            .unwrap();
        let uri = reqwest::Url::from_file_path(t.path().join(path))
            .unwrap()
            .to_string();
        let sha = hash(text.as_bytes());
        let mut references = Value::Null;
        for _ in 0..30 {
            let reply=state.language_servers.request(&owner,t.path().to_str().unwrap(),&server,Some((&uri,text,&sha)),"textDocument/references",json!({"textDocument":{"uri":uri},"position":{"line":line,"character":character},"context":{"includeDeclaration":true}})).await.unwrap();
            references = reply.result;
            if references.as_array().is_some_and(|v| v.len() >= 2) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        assert!(
            references.as_array().is_some_and(|v| v.len() >= 2),
            "Expected real cross-symbol reference evidence: {references}"
        );
        state.language_servers.shutdown().await;
    }
    #[tokio::test]
    #[ignore = "Requires an installed rust-analyzer in LOCALLM_RUST_ANALYZER."]
    async fn real_rust_analyzer() {
        language(
            "rust",
            &std::env::var("LOCALLM_RUST_ANALYZER")
                .expect("Configure installed rust-analyzer path"),
        )
        .await;
    }
    #[tokio::test]
    #[ignore = "Requires installed typescript-language-server in LOCALLM_TYPESCRIPT_LANGUAGE_SERVER."]
    async fn real_typescript_language_server() {
        language(
            "typescript",
            &std::env::var("LOCALLM_TYPESCRIPT_LANGUAGE_SERVER")
                .expect("Configure installed TypeScript server path"),
        )
        .await;
    }
}
