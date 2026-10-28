use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const DEFAULT_MAX_RESULT_CHARS: usize = 2048;

#[tauri::command]
pub async fn preview_execution_artifact(
    state: tauri::State<'_, crate::AppState>,
    root: String,
    path: String,
    sha256: String,
) -> Result<Value, String> {
    use base64::Engine;
    let root = crate::workspace_ui::bound_root(&state, &root)?;
    tokio::task::spawn_blocking(move || {
        let (bytes, mime) = crate::workspace::Workspace::open(&root)?.output_artifact(&path)?;
        if format!("{:x}", Sha256::digest(&bytes)) != sha256 {
            return Err(
                "This artifact changed since execution. Inspect the current file in the workspace."
                    .into(),
            );
        }
        let content = if mime.starts_with("image/") {
            format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(&bytes)
            )
        } else {
            String::from_utf8(bytes).map_err(|e| e.to_string())?
        };
        Ok(json!({"mimeType":mime, "content":content}))
    })
    .await
    .map_err(|_| "Artifact preview failed unexpectedly.".to_string())?
}

/// Smallest result budget the bounding envelope can satisfy. Requests below
/// this floor still receive the smallest useful bounded envelope — artifact
/// pointer, size, hash, and truncation flag — rather than unbounded output
/// or a fake-success result.
pub const MIN_RESULT_BUDGET: usize = 512;

/// Serialized UTF-8 length of any JSON value; `usize::MAX` when serialization
/// fails so an unmeasurable value is always treated as over budget.
fn serialized_len<T: serde::Serialize + ?Sized>(value: &T) -> usize {
    serde_json::to_string(value).map_or(usize::MAX, |s| s.len())
}

/// Cut a string to at most `max_bytes` bytes on a UTF-8 boundary.
fn truncate_utf8(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

/// Cap one preserved metadata field so arbitrary tool output cannot push the
/// serialized envelope past the budget. Strings are cut on a UTF-8 boundary,
/// arrays keep items while they fit, and oversized nested objects collapse
/// to a size note. The full value always remains in the saved artifact.
fn bounded_metadata(value: &Value, cap: usize) -> Value {
    match value {
        Value::String(s) => Value::String(truncate_utf8(s, cap)),
        Value::Array(items) => {
            if serialized_len(value) <= cap {
                return value.clone();
            }
            let mut kept: Vec<Value> = Vec::new();
            let mut used = 2usize; // the brackets themselves
            for item in items {
                let item_len = serialized_len(item) + 1;
                if !kept.is_empty() && (kept.len() >= 16 || used + item_len > cap) {
                    break;
                }
                if used + item_len > cap && kept.is_empty() {
                    break;
                }
                used += item_len;
                kept.push(item.clone());
            }
            if kept.len() < items.len() {
                kept.push(Value::String(format!(
                    "… (+{} more)",
                    items.len() - kept.len()
                )));
            }
            Value::Array(kept)
        }
        Value::Object(_) => {
            if serialized_len(value) <= cap {
                value.clone()
            } else {
                json!({"_omittedBytes": serialized_len(value)})
            }
        }
        _ => value.clone(),
    }
}

/// Keep usable research references rather than a prefix of their serialized
/// JSON. Identity fields stay exact; long display titles cannot crowd them out.
fn bounded_research_references(value: &Value, cap: usize) -> Value {
    let slim = |item: &Value| {
        let mut reference = serde_json::Map::new();
        for key in ["id", "url", "title", "publishedAt", "domain"] {
            if let Some(text) = item.get(key).and_then(Value::as_str) {
                reference.insert(
                    key.into(),
                    json!(if key == "title" {
                        truncate_utf8(text, 256)
                    } else {
                        text.to_string()
                    }),
                );
            }
        }
        Value::Object(reference)
    };
    match value {
        Value::Object(entries) => {
            let mut kept = serde_json::Map::new();
            for (key, item) in entries {
                kept.insert(key.clone(), slim(item));
                if serialized_len(&kept) > cap {
                    kept.remove(key);
                }
            }
            Value::Object(kept)
        }
        Value::Array(items) => {
            let mut kept = Vec::new();
            for item in items {
                kept.push(slim(item));
                if serialized_len(&kept) > cap {
                    kept.pop();
                }
            }
            Value::Array(kept)
        }
        _ => Value::Null,
    }
}

/// Smallest honest envelope: always carries the artifact pointer, original
/// size, hash, and truncation flag, plus scalar error/status fields when the
/// caller's result reports a failure — a bounded result must never look like
/// a success. Variable text fields shrink first, then drop entirely if a
/// (below-floor) budget still cannot fit them.
fn minimal_envelope(
    result: &Value,
    id: &str,
    size_bytes: usize,
    sha256: &str,
    budget: usize,
) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("_truncated".into(), Value::Bool(true));
    obj.insert("_artifactId".into(), Value::String(id.into()));
    obj.insert("_originalBytes".into(), json!(size_bytes));
    obj.insert("_sha256".into(), Value::String(sha256.into()));
    if let Some(source) = result.as_object() {
        for key in ["isError", "status", "code"] {
            if let Some(v) = source.get(key) {
                if serialized_len(v) <= 64 {
                    obj.insert(key.to_string(), v.clone());
                }
            }
        }
        if let Some(error) = source.get("error").and_then(Value::as_str) {
            if !error.is_empty() {
                obj.insert(
                    "error".into(),
                    Value::String(truncate_utf8(error, (budget / 4).clamp(48, 160))),
                );
            }
        }
    }
    obj.insert(
        "_notice".into(),
        Value::String(format!(
            "Output exceeded the result budget ({size_bytes} bytes). Complete artifact saved as '{id}'; page it with artifact_read."
        )),
    );
    let mut envelope = Value::Object(obj);
    while serialized_len(&envelope) > budget {
        let Some(map) = envelope.as_object_mut() else {
            break;
        };
        let longest = ["error", "_notice"]
            .iter()
            .filter_map(|key| {
                map.get(*key)
                    .and_then(Value::as_str)
                    .map(|s| (*key, s.len()))
            })
            .max_by_key(|(_, len)| *len);
        match longest {
            Some((key, len)) if len > 40 => {
                let next = truncate_utf8(map[key].as_str().unwrap_or_default(), len / 2);
                map.insert(key.to_string(), Value::String(next));
            }
            Some((key, _)) => {
                map.remove(key);
            }
            None => break,
        }
    }
    envelope
}

/// Re-wrap an excerpt of an envelope result so bounded model output keeps the
/// `locallm.tool-envelope/1` contract: truncation downgrades success to
/// `partial`, the original error/metadata survive, and the artifact pointer
/// rides `metadata`. Non-envelope results pass through unchanged.
fn rewrap_envelope(
    result: &Value,
    excerpt: Value,
    artifact_id: &str,
    size_bytes: usize,
    sha256: &str,
) -> Value {
    if result.get("schemaVersion").and_then(Value::as_str)
        != Some(crate::tool_envelope::SCHEMA_VERSION)
    {
        return excerpt;
    }
    let status = crate::tool_envelope::Status::parse(
        result.get("status").and_then(Value::as_str).unwrap_or(""),
    )
    .unwrap_or(crate::tool_envelope::Status::Partial);
    let status = if status == crate::tool_envelope::Status::Success {
        crate::tool_envelope::Status::Partial
    } else {
        status
    };
    let mut metadata = result.get("metadata").cloned().unwrap_or_else(|| json!({}));
    metadata["truncated"] = json!(true);
    metadata["artifactId"] = json!(artifact_id);
    metadata["originalBytes"] = json!(size_bytes);
    metadata["sha256"] = json!(sha256);
    let error = result.get("error").cloned().unwrap_or(Value::Null);
    let mut envelope = crate::tool_envelope::make(
        status,
        excerpt,
        result
            .get("artifacts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        if error.is_null() { None } else { Some(error) },
        metadata,
    );
    // Keep the legacy flat failure markers and truncation pointers beside the
    // envelope — the same keys non-enveloped bounded results carry.
    for key in ["isError", "message", "denied", "paused"] {
        if let Some(value) = result.get(key) {
            envelope[key.to_string()] = value.clone();
        }
    }
    envelope["_truncated"] = json!(true);
    envelope["_artifactId"] = json!(artifact_id);
    envelope["_originalBytes"] = json!(size_bytes);
    envelope["_sha256"] = json!(sha256);
    envelope
}

/// Excerpt budget in serialized UTF-8 bytes for one tool result, derived from the tokens
/// still free after the counted input and the response reserve. At most half
/// of the remaining allowance is spent, so the next model round keeps
/// headroom; the budget stays within [512, 8192] bytes either way.
pub fn excerpt_budget(context_length: u32, input_tokens: u64, response_reserve: u32) -> usize {
    let used = input_tokens.min(u64::from(context_length)) as u32;
    let remaining = context_length
        .saturating_sub(used)
        .saturating_sub(response_reserve);
    let chars = (remaining as usize).saturating_mul(4) / 2;
    chars.clamp(512, 8192)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ArtifactRecord {
    pub id: String,
    pub conversation_id: String,
    pub run_id: Option<String>,
    pub tool_name: String,
    pub mime_type: String,
    pub size_bytes: usize,
    pub sha256: String,
    pub content: String,
    pub created_at: i64,
}

// Compact canonical scaffolding shared by native continuation summaries.
// Source family and fixed fields stay bounded before optional display content.
fn native_summary(
    result: &Value,
    data: Value,
    artifact: &ArtifactRecord,
    budget: usize,
) -> Option<Value> {
    let enveloped = result.get("schemaVersion").and_then(Value::as_str)
        == Some(crate::tool_envelope::SCHEMA_VERSION);
    if enveloped && !matches!(result.pointer("/metadata/family").and_then(Value::as_str), Some("harness" | "workspace")) {
        return None;
    }
    let mut out = if enveloped {
        let status = result["status"].as_str()?;
        let error = if result["error"].is_null() {
            Value::Null
        } else {
            json!({"code":truncate_utf8(result["error"]["code"].as_str().unwrap_or("tool-error"),32),
                "message":"See artifact.","retryable":false,
                "sideEffects":result.pointer("/error/sideEffects").and_then(Value::as_str).unwrap_or("unknown")})
        };
        crate::tool_envelope::make(
            crate::tool_envelope::Status::parse(if status == "success" {
                "partial"
            } else {
                status
            })?,
            data,
            Vec::new(),
            if error.is_null() { None } else { Some(error) },
            json!({"family":result["metadata"]["family"],"truncated":true,"sideEffects":result.pointer("/metadata/sideEffects").and_then(Value::as_str).unwrap_or("unknown")}),
        )
    } else {
        data
    };
    out["_truncated"] = json!(true);
    out["_artifactId"] = json!(artifact.id);
    out["_originalBytes"] = json!(artifact.size_bytes);
    out["_sha256"] = json!(artifact.sha256);
    // The fixed native envelope fits the supported floor. A malformed result
    // takes the generic bounding path instead of evading the byte cap.
    if serialized_len(&out) > budget {
        return None;
    }
    Some(out)
}

/// Native lifecycle results need a usable handle even when their logs or case
/// diagnostics overflow. Build a small, typed summary before spending space on
/// optional fields. Never interpret an arbitrary connector's `id` as a process.
fn lifecycle_excerpt(
    result: &Value,
    tool: &str,
    artifact: &ArtifactRecord,
    budget: usize,
) -> Option<Value> {
    let checks = matches!(tool, "check_run" | "check_report");
    let debug = false;
    // `shell_run` returns a live process handle when the command outlives its
    // window. It has to keep the handle and the "still running" state, or the
    // model loses the only way to reach a child that is still running.
    let shell = tool == "shell_run";
    if !checks
        && !debug
        && !shell
        && !matches!(
            tool,
            "process_start" | "process_poll" | "process_write" | "process_stop"
        )
    {
        return None;
    }
    let enveloped = result.get("schemaVersion").and_then(Value::as_str)
        == Some(crate::tool_envelope::SCHEMA_VERSION);
    if enveloped && result.pointer("/metadata/family").and_then(Value::as_str) != Some("harness") {
        return None;
    }
    let source = if enveloped { &result["data"] } else { result };
    let key = if debug {
        "sessionId"
    } else if checks || shell {
        "processId"
    } else {
        "id"
    };
    let id = source.get(key)?.as_str()?;
    // These are host-generated UUID handles, not clipped display strings.
    uuid::Uuid::parse_str(id.strip_prefix(if debug { "debug-" } else { "proc-" })?).ok()?;
    let mut out = native_summary(result, json!({key:id}), artifact, budget)?;
    // Optional summary fields enter atomically only if the result still fits.
    let mut insert = |path: &[&str], value: &Value| {
        let counts = path == ["report", "counts"]
            && value.as_object().is_some_and(|obj| {
                obj.len() == 5
                    && ["total", "passed", "failed", "skipped", "todo"]
                        .iter()
                        .all(|key| obj.get(*key).is_some_and(Value::is_u64))
            });
        if !(counts
            || value.is_null()
            || value.is_boolean()
            || value.is_number()
            || value.as_str().is_some_and(|s| s.len() <= 128))
        {
            return;
        }
        let mut candidate = out.clone();
        let mut target = if enveloped {
            &mut candidate["data"]
        } else {
            &mut candidate
        };
        for key in &path[..path.len() - 1] {
            if target.get(*key).is_none() {
                target[*key] = json!({});
            }
            target = &mut target[*key];
        }
        target[path[path.len() - 1]] = value.clone();
        if serialized_len(&candidate) <= budget {
            out = candidate;
        }
    };
    if checks {
        insert(&["report", "caseDetailsOmitted"], &json!(true));
        for field in [
            "verdict",
            "testsPassed",
            "incomplete",
            "commandSucceeded",
            "sourceSha256",
            "countsScope",
            "hasMoreCases",
            "nextCaseOffset",
            "caseOffset",
            "availableCases",
            "caseDetailsComplete",
            "format",
            "requestedFormat",
            "sourceStream",
            "runnerFailed",
            "suites",
        ] {
            if let Some(value) = source["report"].get(field) {
                insert(&["report", field], value);
            }
        }
        if let Some(value) = source["report"].get("counts") {
            insert(&["report", "counts"], value);
        }
        for field in [
            "state",
            "exitCode",
            "nextCursor",
            "hasMore",
            "truncated",
            "firstAvailableCursor",
        ] {
            if let Some(value) = source["process"].get(field) {
                insert(&["process", field], value);
            }
        }
        if let Some(value) = source.get("started") {
            insert(&["started"], value);
        }
    } else {
        for field in [
            "state",
            "exitCode",
            "nextCursor",
            "hasMore",
            "truncated",
            "firstAvailableCursor",
            "written",
            "stdinClosed",
        ] {
            if let Some(value) = source.get(field) {
                insert(&[field], value);
            }
        }
    }
    if shell {
        // The tool's own `status` says whether the PROCESS is still running.
        // The envelope's `status` describes the call, so the process state
        // needs its own field to survive bounding.
        if let Some(value) = source.get("status").and_then(Value::as_str) {
            insert(&["processStatus"], &json!(value));
        }
        if let Some(value) = source.get("exitCode") {
            insert(&["exitCode"], value);
        }
    }
    // Compatibility fields are optional; the canonical data handle is exact.
    for (key, value) in [
        (key, json!(id)),
        ("isError", json!(crate::tool_envelope::failed(result))),
    ] {
        let mut candidate = out.clone();
        candidate[key] = value;
        if serialized_len(&candidate) <= budget {
            out = candidate;
        }
    }
    Some(out)
}

fn append_text_excerpt(mut out: Value, key: &str, text: &str, enveloped: bool, budget: usize) -> Value {
    let (mut low, mut high) = (0, text.len().min(budget));
    while low <= high {
        let mid = low + (high - low) / 2;
        let mut candidate = out.clone();
        let target = if enveloped { &mut candidate["data"] } else { &mut candidate };
        target[key] = json!(truncate_utf8(text, mid));
        if serialized_len(&candidate) <= budget {
            out = candidate;
            low = mid + 1;
        } else if mid == 0 { break; } else { high = mid - 1; }
    }
    out
}

/// Keep file hashes separate from the saved artifact's checksum. Dropping a
/// committed file's hash made agents reuse the artifact checksum for edits.
fn workspace_excerpt(result: &Value, tool: &str, artifact: &ArtifactRecord, budget: usize) -> Option<Value> {
    if result.pointer("/metadata/family").and_then(Value::as_str) != Some("workspace")
        || crate::tool_envelope::failed(result)
        || !matches!(tool, "apply_patch" | "workspace_apply_patch" | "edit_file" | "workspace_edit_file" | "read_file" | "workspace_read_file")
    {
        return None;
    }
    let source = &result["data"];
    let mut data = serde_json::Map::new();
    for key in ["path", "sha256", "totalLines", "bytesWritten", "reformatted", "transactionId", "recoveryRequired", "status"] {
        if let Some(value) = source.get(key) { data.insert(key.into(), value.clone()); }
    }
    if source["changes"].is_array() {
        data.insert("changes".into(), json!([]));
        data.insert("changesOmitted".into(), json!(source["changes"].as_array()?.len()));
    }
    let mut out = native_summary(result, Value::Object(data), artifact, usize::MAX)?;
    for key in ["_originalBytes", "_sha256"] {
        if serialized_len(&out) > budget { out.as_object_mut()?.remove(key); }
    }
    if serialized_len(&out) > budget { return None; }
    for change in source["changes"].as_array().into_iter().flatten() {
        let mut slim = serde_json::Map::new();
        for key in ["path", "destination", "sha256", "kind"] {
            if let Some(value) = change.get(key) { slim.insert(key.into(), value.clone()); }
        }
        let mut candidate = out.clone();
        candidate["data"]["changes"].as_array_mut()?.push(Value::Object(slim));
        candidate["data"]["changesOmitted"] = json!(source["changes"].as_array()?.len() - candidate["data"]["changes"].as_array()?.len());
        if serialized_len(&candidate) > budget { break; }
        out = candidate;
    }
    if let Some(text) = source["content"].as_str() {
        out = append_text_excerpt(out, "content", text, true, budget);
    }
    Some(out)
}

/// A fetched page lives in the document store. Its two-part reference must
/// survive excerpting so the agent can search or open the remaining text.
fn web_document_excerpt(
    result: &Value,
    tool: &str,
    artifact: &ArtifactRecord,
    budget: usize,
) -> Option<Value> {
    if !matches!(tool, "web_read" | "web_open" | "web_fetch_url" | "web_find") {
        return None;
    }
    let enveloped = result["schemaVersion"] == crate::tool_envelope::SCHEMA_VERSION;
    let source = if enveloped { &result["data"] } else { result };
    let session = source["sessionId"].as_str().filter(|s| s.len() <= 128)?;
    let document = source["documentId"].as_str().filter(|s| s.len() <= 128)?;
    let mut out = native_summary(
        result,
        json!({"sessionId":session,"documentId":document}),
        artifact,
        usize::MAX,
    )?;
    // Leave space for initial-page text and its cursor before optional labels.
    // Artifact byte counts are never document-text offsets.
    let label_budget = budget.saturating_sub(if source["text"].is_string() { 160 } else { 0 });
    for key in ["_originalBytes", "_sha256"] {
        if serialized_len(&out) > label_budget {
            out.as_object_mut()?.remove(key);
        }
    }
    if serialized_len(&out) > budget {
        return None;
    }
    if let Some(passages) = source["passages"].as_array() {
        let mut kept = Vec::new();
        for passage in passages {
            let text = passage["text"].as_str().unwrap_or_default();
            let mut candidate_passage = passage.clone();
            // Spend the available space on complete passages first. A clipped
            // passage advances only past the text actually shown, not its tail.
            let mut best = None;
            let (mut low, mut high) = (0, text.len());
            while low <= high {
                let mid = low + (high - low) / 2;
                let excerpt = truncate_utf8(text, mid);
                candidate_passage["text"] = json!(excerpt);
                candidate_passage["chars"] = json!(excerpt.encode_utf16().count());
                if excerpt.len() < text.len() {
                    candidate_passage["textTruncated"] = json!(true);
                    if let Some(start) = passage["textOffset"].as_u64() {
                        candidate_passage["nextTextOffset"] =
                            json!(start + excerpt.encode_utf16().count() as u64);
                    }
                } else {
                    candidate_passage.as_object_mut()?.remove("textTruncated");
                    candidate_passage["nextTextOffset"] = passage["nextTextOffset"].clone();
                }
                let mut candidate = out.clone();
                let data = if enveloped {
                    &mut candidate["data"]
                } else {
                    &mut candidate
                };
                let mut combined = kept.clone();
                combined.push(candidate_passage.clone());
                data["passages"] = json!(combined);
                data["passagesOmitted"] = json!(passages.len() - combined.len());
                if serialized_len(&candidate) <= budget {
                    best = Some((candidate, candidate_passage.clone(), excerpt.len()));
                    low = mid + 1;
                } else if mid == 0 {
                    break;
                } else {
                    high = mid - 1;
                }
            }
            let Some((candidate, passage, shown)) = best else {
                break;
            };
            out = candidate;
            kept.push(passage);
            if shown < text.len() {
                break;
            }
        }
    }
    for key in [
        "url",
        "title",
        "textTotalChars",
        "textTruncated",
        "extractionMethod",
        "snippetOnly",
    ] {
        if let Some(value) = source.get(key) {
            let mut candidate = out.clone();
            let target = if enveloped {
                &mut candidate["data"]
            } else {
                &mut candidate
            };
            target[key] = value.clone();
            if serialized_len(&candidate) <= label_budget {
                out = candidate;
            }
        }
    }
    if let Some(text) = source["text"].as_str() {
        let start = source["textOffset"].as_u64().unwrap_or(0);
        let mut best = None;
        let (mut low, mut high) = (0, text.len().min(budget));
        while low <= high {
            let mid = low + (high - low) / 2;
            let visible = truncate_utf8(text, mid);
            let next = start + visible.encode_utf16().count() as u64;
            let more = visible.len() < text.len()
                || source["nextTextOffset"].is_number()
                || source["textTruncated"] == true
                || source["textTotalChars"]
                    .as_u64()
                    .is_some_and(|total| next < total);
            let mut candidate = out.clone();
            let target = if enveloped {
                &mut candidate["data"]
            } else {
                &mut candidate
            };
            target["text"] = json!(visible);
            target["textOffset"] = json!(start);
            target["nextTextOffset"] = if more { json!(next) } else { Value::Null };
            target["textTruncated"] = json!(more);
            if serialized_len(&candidate) <= budget {
                best = Some(candidate);
                low = mid + 1;
            } else if mid == 0 {
                break;
            } else {
                high = mid - 1;
            }
        }
        out = best?;
    }
    Some(out)
}

/// A staged-tree pin is a continuation handle too. Keep it exact and flag
/// that the excerpt is not a complete review; full diff bytes remain saved.
fn git_diff_excerpt(
    result: &Value,
    tool: &str,
    artifact: &ArtifactRecord,
    budget: usize,
) -> Option<Value> {
    if tool != "git_diff" {
        return None;
    }
    let enveloped = result.get("schemaVersion").and_then(Value::as_str)
        == Some(crate::tool_envelope::SCHEMA_VERSION);
    let source = if enveloped { &result["data"] } else { result };
    let diff = source.get("diff")?.as_str()?;
    let pin = source.get("stagedTree")?;
    if !pin.is_null()
        && !pin.as_str().is_some_and(|s| {
            matches!(s.len(), 40 | 64)
                && s.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        })
    {
        return None;
    }
    let data = json!({"stagedTree":pin,"reviewIncomplete":true,"pathCount":source["paths"].as_array().map(Vec::len),"truncated":source["truncated"]==true});
    let mut best = native_summary(result, data, artifact, budget)?;
    let (mut low, mut high) = (0, diff.len().min(budget));
    while low <= high {
        let mid = low + (high - low) / 2;
        let mut candidate = best.clone();
        let target = if enveloped {
            &mut candidate["data"]
        } else {
            &mut candidate
        };
        target["diff_excerpt"] = json!(truncate_utf8(diff, mid));
        if serialized_len(&candidate) <= budget {
            best = candidate;
            low = mid + 1;
        } else {
            if mid == 0 {
                break;
            }
            high = mid - 1;
        }
    }
    Some(best)
}

/// Rename continuation pins must survive large previews without implying that
/// an excerpt constitutes a complete review of the stored proposal.
fn lsp_rename_excerpt(
    result: &Value,
    tool: &str,
    artifact: &ArtifactRecord,
    budget: usize,
) -> Option<Value> {
    if !matches!(
        tool,
        "lsp_prepare_rename" | "lsp_prepare_code_action" | "lsp_execute_command"
    ) {
        return None;
    }
    let enveloped = result.get("schemaVersion").and_then(Value::as_str)
        == Some(crate::tool_envelope::SCHEMA_VERSION);
    let source = if enveloped { &result["data"] } else { result };
    let id = source.get("proposalId")?.as_str()?;
    uuid::Uuid::parse_str(id.strip_prefix("lsp_rename_")?).ok()?;
    let sha = source.get("proposalSha256")?.as_str()?;
    if sha.len() != 64 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let data = json!({"proposalId":id,"proposalSha256":sha,"reviewIncomplete":true});
    // Two exact pins cost more than a process ID. Keep the saved-result ID;
    // its optional display checksum/byte count yield to the continuation pins.
    let mut out = native_summary(result, data, artifact, usize::MAX)?;
    for key in ["_originalBytes", "_sha256"] {
        if serialized_len(&out) > budget {
            out.as_object_mut()?.remove(key);
        }
    }
    if serialized_len(&out) > budget {
        return None;
    }
    for key in ["server", "fileCount", "sourceSha256"] {
        if let Some(value) = source.get(key) {
            let bounded = value.is_u64() || value.as_str().is_some_and(|s| s.len() <= 64);
            if !bounded {
                continue;
            }
            let mut candidate = out.clone();
            let target = if enveloped {
                &mut candidate["data"]
            } else {
                &mut candidate
            };
            target[key] = value.clone();
            if serialized_len(&candidate) <= budget {
                out = candidate;
            }
        }
    }
    Some(out)
}

/// Bound a tool output so huge outputs (e.g. web pages, dumps, large files) do not blow up model context.
/// When serialized output exceeds `max_chars` bytes, full output is captured in an `ArtifactRecord`, and a structured excerpt
/// with truncation indicators and artifact ID is returned to the model.
pub fn bound_tool_result(
    result: Value,
    tool_name: &str,
    conversation_id: &str,
    run_id: Option<&str>,
    max_chars: usize,
) -> (Value, Option<ArtifactRecord>) {
    // A page is a window into an existing artifact, not a new artifact. Size
    // the final wire envelope (including escaping) before returning a cursor.
    let page = result.get("data").filter(|v| v.get("excerpt").is_some()).unwrap_or(&result);
    if tool_name == "artifact_read" && page["excerpt"].is_string() && page["id"].is_string() {
        return (bound_artifact_page(&result, page, max_chars.max(MIN_RESULT_BUDGET)), None);
    }
    let wire_size = serialized_len(&result);
    if wire_size <= max_chars {
        return (result, None);
    }
    // The enforced cap is a serialized UTF-8 byte bound. Below the supported
    // floor, fall back to the smallest useful envelope instead of emitting
    // an unbounded or dishonest result.
    let budget = max_chars.max(MIN_RESULT_BUDGET);
    if wire_size <= budget {
        return (result, None);
    }

    // Persist the same versioned contract the provider receives. Internal
    // legacy aliases otherwise duplicate document text in the saved artifact.
    let raw = crate::tool_envelope::model_result(&result).to_string();
    let size_bytes = raw.len();

    let id = format!("art_{}", uuid::Uuid::new_v4().simple());
    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    let sha256 = format!("{:x}", hasher.finalize());

    let artifact = ArtifactRecord {
        id: id.clone(),
        conversation_id: conversation_id.into(),
        run_id: run_id.map(String::from),
        tool_name: tool_name.into(),
        mime_type: if result.is_object() || result.is_array() {
            "application/json".into()
        } else {
            "text/plain".into()
        },
        size_bytes,
        sha256: sha256.clone(),
        content: raw.clone(),
        created_at: crate::store::now(),
    };

    if let Some(excerpt) = lifecycle_excerpt(&result, tool_name, &artifact, budget)
        .or_else(|| workspace_excerpt(&result, tool_name, &artifact, budget))
        .or_else(|| web_document_excerpt(&result, tool_name, &artifact, budget))
        .or_else(|| git_diff_excerpt(&result, tool_name, &artifact, budget))
        .or_else(|| lsp_rename_excerpt(&result, tool_name, &artifact, budget))
    {
        return (excerpt, Some(artifact));
    }

    // Create a bounded representation preserving key error or status indicators
    let excerpt = if let Some(obj) = result.as_object() {
        let mut bounded_obj = serde_json::Map::new();
        // Preserve critical metadata fields, each bounded so a huge preserved
        // value cannot blow through the final byte cap on its own.
        let field_cap = (budget / 4).clamp(64, 512);
        for (k, v) in obj {
            if matches!(
                k.as_str(),
                "isError"
                    | "status"
                    | "code"
                    | "error"
                    | "title"
                    | "url"
                    | "source"
                    | "failures"
                    | "failureCount"
                    | "rawCount"
                    | "goal"
                    | "instruction"
                    | "_screenshotArtifactIds"
                    | "width"
                    | "height"
                    | "coordinateSpace"
            ) {
                bounded_obj.insert(k.clone(), bounded_metadata(v, field_cap));
            }
        }
        bounded_obj.insert("_truncated".into(), Value::Bool(true));
        bounded_obj.insert("_artifactId".into(), Value::String(id.clone()));
        bounded_obj.insert("_originalBytes".into(), json!(size_bytes));
        bounded_obj.insert("_sha256".into(), Value::String(sha256.clone()));

        // The research worker returns evidence and document handles, not a
        // generic text/content field. Preserve both so a model can use the
        // evidence or read more without rerunning the search.
        let mut handled = false;
        if obj.get("sessionId").is_some_and(Value::is_string) {
            if let Some(evidence) = obj
                .get("evidence")
                .or_else(|| obj.get("answer"))
                .and_then(Value::as_str)
            {
                for key in ["sessionId", "synthesis"] {
                    if let Some(value) = obj.get(key) {
                        bounded_obj.insert(key.into(), bounded_metadata(value, 128));
                    }
                }
                for key in ["sources", "documents"] {
                    if let Some(value) = obj.get(key) {
                        bounded_obj
                            .insert(key.into(), bounded_research_references(value, budget / 5));
                    }
                }
                if let Some(diagnostics) = obj.get("diagnostics") {
                    bounded_obj.insert(
                        "diagnostics".into(),
                        bounded_metadata(diagnostics, (budget / 8).clamp(64, 512)),
                    );
                }
                bounded_obj.insert("evidence".into(), json!(truncate_utf8(evidence, budget)));
                handled = true;
            }
        }

        // artifact_read itself passes through this limiter. Keep the original
        // page identity and cursor, rather than producing an artifact of an
        // unreadable JSON preview of another artifact.
        let artifact_page =
            tool_name == "artifact_read" && obj.get("excerpt").is_some_and(Value::is_string);
        if artifact_page {
            for key in ["id", "tool", "offset", "limit", "totalChars", "hasMore"] {
                if let Some(value) = obj.get(key) {
                    bounded_obj.insert(key.into(), value.clone());
                }
            }
            bounded_obj.insert(
                "excerpt".into(),
                json!(truncate_utf8(
                    obj["excerpt"].as_str().unwrap_or_default(),
                    budget
                )),
            );
            handled = true;
        }

        // Find primary content field or provide truncated overview.
        if let Some(results) = obj
            .get("structuredContent")
            .filter(|_| !handled)
            .and_then(|sc| sc.get("results"))
            .and_then(Value::as_array)
        {
            let mut snippets = Vec::new();
            for r in results.iter().take(5) {
                let title = r.get("title").and_then(Value::as_str).unwrap_or("");
                let url = r.get("url").and_then(Value::as_str).unwrap_or("");
                let excerpt = if let Some(excerpts) = r.get("excerpts").and_then(Value::as_array) {
                    excerpts
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(" ")
                } else if let Some(snippet) = r.get("snippet").and_then(Value::as_str) {
                    snippet.to_string()
                } else {
                    String::new()
                };
                let mut item = serde_json::Map::new();
                if !title.is_empty() {
                    item.insert("title".into(), json!(title));
                }
                if !url.is_empty() {
                    item.insert("url".into(), json!(url));
                }
                if !excerpt.is_empty() {
                    let clipped: String = excerpt.chars().take(280).collect();
                    item.insert("snippet".into(), json!(clipped));
                }
                if !item.is_empty() {
                    snippets.push(Value::Object(item));
                }
            }
            if !snippets.is_empty() {
                bounded_obj.insert("search_results".into(), Value::Array(snippets));
                handled = true;
            }
        }

        if !handled {
            if let Some(arr) = obj.get("content").and_then(Value::as_array) {
                let mut combined = String::new();
                for item in arr {
                    if let Some(t) = item.get("text").and_then(Value::as_str) {
                        if !combined.is_empty() {
                            combined.push_str("\n\n");
                        }
                        combined.push_str(t);
                    }
                }
                if !combined.is_empty() {
                    let take_len = budget.min(1500);
                    let slice: String = combined.chars().take(take_len).collect();
                    bounded_obj.insert("content_excerpt".into(), Value::String(slice));
                    handled = true;
                }
            }
        }

        if !handled {
            if let Some(results) = obj.get("results").and_then(Value::as_array) {
                // Search-style hit lists ({results:[{title,url,snippet,…}]}):
                // keep every item's identity inside the budget instead of a
                // raw-JSON prefix that only ever shows the first result.
                let total = results.len();
                let mut kept: Vec<Value> = Vec::new();
                let mut list_budget = budget.saturating_sub(700);
                for item in results {
                    let mut slim = serde_json::Map::new();
                    for key in ["title", "url", "publishedAt", "domain", "engine"] {
                        if let Some(v) = item.get(key) {
                            slim.insert(key.into(), v.clone());
                        }
                    }
                    let snippet = item
                        .get("snippet")
                        .or_else(|| item.get("text"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    let collapsed: String =
                        snippet.split_whitespace().collect::<Vec<_>>().join(" ");
                    let clipped: String = collapsed.chars().take(280).collect();
                    if !clipped.is_empty() {
                        slim.insert("snippet".into(), Value::String(clipped));
                    }
                    if slim.is_empty() {
                        continue;
                    }
                    let size = serde_json::to_string(&slim).map_or(0, |s| s.len());
                    if size > list_budget {
                        break;
                    }
                    list_budget -= size;
                    kept.push(Value::Object(slim));
                }
                if !kept.is_empty() {
                    bounded_obj.insert(
                        "_resultsKept".into(),
                        json!(format!("{} of {}", kept.len(), total)),
                    );
                    bounded_obj.insert("results".into(), Value::Array(kept));
                    handled = true;
                }
            }
        }

        if !handled {
            if let Some(pages) = obj.get("pages").and_then(Value::as_array) {
                // Visit-style page lists: per-page text gets a share of the
                // budget; failed pages stay visible as small error entries.
                let total = pages.len();
                let per_page = (budget.saturating_sub(700) / total.max(1)).clamp(400, 4000);
                let mut kept: Vec<Value> = Vec::new();
                let mut list_budget = budget.saturating_sub(700);
                for page in pages {
                    let mut slim = serde_json::Map::new();
                    if page.get("isError") == Some(&Value::Bool(true)) {
                        for key in ["isError", "message", "url"] {
                            if let Some(v) = page.get(key) {
                                slim.insert(key.into(), v.clone());
                            }
                        }
                    } else {
                        for key in [
                            "url",
                            "title",
                            "publishedAt",
                            "extractionMethod",
                            "archivedAt",
                        ] {
                            if let Some(v) = page.get(key) {
                                slim.insert(key.into(), v.clone());
                            }
                        }
                        if let Some(headings) = page.get("headings").and_then(Value::as_array) {
                            slim.insert(
                                "headings".into(),
                                Value::Array(headings.iter().take(12).cloned().collect()),
                            );
                        }
                        if let Some(text) = page.get("text").and_then(Value::as_str) {
                            let clipped: String = text.chars().take(per_page).collect();
                            if !clipped.is_empty() {
                                slim.insert("text".into(), Value::String(clipped));
                            }
                        }
                    }
                    if slim.is_empty() {
                        continue;
                    }
                    let size = serde_json::to_string(&slim).map_or(0, |s| s.len());
                    if size > list_budget {
                        break;
                    }
                    list_budget -= size;
                    kept.push(Value::Object(slim));
                }
                if !kept.is_empty() {
                    bounded_obj.insert(
                        "_pagesKept".into(),
                        json!(format!("{} of {}", kept.len(), total)),
                    );
                    bounded_obj.insert("pages".into(), Value::Array(kept));
                    handled = true;
                }
            }
        }

        if !handled {
            if let Some(content_str) = obj.get("content").and_then(Value::as_str) {
                let take_len = budget.min(1500);
                let slice: String = content_str.chars().take(take_len).collect();
                bounded_obj.insert("content_excerpt".into(), Value::String(slice));
            } else if let Some(text_str) = obj.get("text").and_then(Value::as_str) {
                let take_len = budget.min(1500);
                let slice: String = text_str.chars().take(take_len).collect();
                bounded_obj.insert("text_excerpt".into(), Value::String(slice));
            } else if let Some(body_str) = obj.get("body").and_then(Value::as_str) {
                // web_fetch returns {url, status, body}: excerpt the cleaned text.
                let take_len = budget.min(1500);
                let slice: String = body_str.chars().take(take_len).collect();
                bounded_obj.insert("body_excerpt".into(), Value::String(slice));
            } else {
                let fallback: String = result.to_string().chars().take(1200).collect();
                bounded_obj.insert("preview".into(), Value::String(fallback));
            }
        }

        bounded_obj.insert(
            "_notice".into(),
            Value::String(if artifact_page {
                "Next page: use the original id and offset + limit.".into()
            } else { format!(
                "Output excerpted ({size_bytes} bytes). Complete result saved as '{id}'. Read more with artifact_read using this id and an offset/limit before concluding information is unavailable. Do not repeat the original tool call."
            ) }),
        );

        // Enforce the serialized byte cap on the final envelope, whichever
        // fields were populated: shrink list fields first (they carry the
        // most weight), then the longest excerpt/metadata strings; if the
        // object still overflows, degrade to the minimal pointer envelope.
        let mut cur_len = serialized_len(&bounded_obj);
        while cur_len > budget {
            let mut shrunk = false;
            for key in ["results", "pages", "search_results", "failures"] {
                let over = bounded_obj
                    .get(key)
                    .and_then(Value::as_array)
                    .is_some_and(|a| !a.is_empty());
                if over {
                    if let Some(Value::Array(arr)) = bounded_obj.get_mut(key) {
                        arr.pop();
                    }
                    shrunk = true;
                    break;
                }
            }
            if !shrunk {
                let longest = [
                    "content_excerpt",
                    "text_excerpt",
                    "body_excerpt",
                    "preview",
                    "evidence",
                    "excerpt",
                    "goal",
                    "instruction",
                    "error",
                    "title",
                    "status",
                    "url",
                    "source",
                ]
                .iter()
                .filter_map(|key| {
                    bounded_obj
                        .get(*key)
                        .and_then(Value::as_str)
                        .map(|s| (*key, s.len()))
                })
                .max_by_key(|(_, len)| *len);
                if let Some((key, len)) = longest {
                    let excess = cur_len.saturating_sub(budget) + 32;
                    let keep = len.saturating_sub(excess);
                    if keep == 0 {
                        bounded_obj.remove(key);
                    } else {
                        let trimmed =
                            truncate_utf8(bounded_obj[key].as_str().unwrap_or_default(), keep);
                        bounded_obj.insert(key.to_string(), Value::String(trimmed));
                    }
                    shrunk = true;
                }
            }
            if !shrunk {
                break;
            }
            cur_len = serialized_len(&bounded_obj);
        }
        if artifact_page {
            let kept = bounded_obj
                .get("excerpt")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .chars()
                .count();
            let original = obj["excerpt"].as_str().unwrap_or_default().chars().count();
            bounded_obj.insert("limit".into(), json!(kept));
            bounded_obj.insert(
                "hasMore".into(),
                json!(kept < original || obj["hasMore"] == true),
            );
            cur_len = serialized_len(&bounded_obj);
        }
        let excerpt = if cur_len > budget {
            minimal_envelope(&result, &id, size_bytes, &sha256, budget)
        } else {
            Value::Object(bounded_obj)
        };
        let wrapped = rewrap_envelope(&result, excerpt, &id, size_bytes, &sha256);
        if serialized_len(&wrapped) > budget {
            // Envelope overhead pushed the excerpt over budget — degrade to
            // the minimal pointer, still wrapped in a valid envelope.
            let minimal = rewrap_envelope(
                &result,
                minimal_envelope(&result, &id, size_bytes, &sha256, budget),
                &id,
                size_bytes,
                &sha256,
            );
            if serialized_len(&minimal) > budget {
                // The envelope overhead alone is larger than the budget. Shrink
                // the *inner* object until the whole envelope fits, so the
                // model-visible value never exceeds the documented bound.
                let mut inner = minimal_envelope(&result, &id, size_bytes, &sha256, budget);
                let mut wrapped_minimal = minimal.clone();
                while serialized_len(&wrapped_minimal) > budget {
                    let Some(data) = inner.get("data").and_then(Value::as_object).cloned() else {
                        break;
                    };
                    if data.is_empty() {
                        break;
                    }
                    let mut shrunk = data;
                    shrunk.remove("excerpt");
                    shrunk.remove("preview");
                    for key in ["stdout", "stderr", "content", "text", "output"] {
                        if let Some(value) = shrunk.get(key).and_then(Value::as_str) {
                            let keep = value.chars().count() / 2;
                            shrunk.insert(
                                key.into(),
                                Value::String(value.chars().take(keep).collect()),
                            );
                        }
                    }
                    inner = json!({"data": shrunk});
                    wrapped_minimal =
                        rewrap_envelope(&result, inner.clone(), &id, size_bytes, &sha256);
                }
                wrapped_minimal
            } else {
                minimal
            }
        } else {
            wrapped
        }
    } else {
        let text = result.as_str().unwrap_or(&raw);
        // Reserve room for the fixed pointer fields before excerpting.
        let preview_cap = budget.saturating_sub(400).min(1500);
        let preview: String = text.chars().take(preview_cap).collect();
        let mut scalar = json!({
            "_truncated": true,
            "_artifactId": id,
            "_originalBytes": size_bytes,
            "_sha256": sha256,
            "preview": preview,
            "_notice": format!("Output was truncated ({size_bytes} bytes). Complete artifact saved as '{id}'. To read more, call artifact_read with id '{id}' and an offset/limit.")
        });
        if serialized_len(&scalar) > budget {
            let excess = serialized_len(&scalar).saturating_sub(budget) + 32;
            let trimmed = truncate_utf8(
                scalar["preview"].as_str().unwrap_or_default(),
                scalar["preview"]
                    .as_str()
                    .unwrap_or_default()
                    .len()
                    .saturating_sub(excess),
            );
            scalar["preview"] = Value::String(trimmed);
            if serialized_len(&scalar) > budget {
                scalar = minimal_envelope(&result, &id, size_bytes, &sha256, budget);
            }
        }
        scalar
    };

    (excerpt, Some(artifact))
}

fn bound_artifact_page(original: &Value, page: &Value, budget: usize) -> Value {
    let text = page["excerpt"].as_str().unwrap_or_default();
    let chars: Vec<char> = text.chars().collect();
    let enveloped = crate::tool_envelope::is_envelope(original);
    let render = |count: usize| {
        let data = json!({"id":page["id"],"offset":page["offset"],"limit":count,
            "totalChars":page["totalChars"],"hasMore":count < chars.len() || page["hasMore"] == true,
            "excerpt":chars[..count].iter().collect::<String>()});
        if enveloped {
            crate::tool_envelope::make(crate::tool_envelope::Status::Success, data, vec![], None,
                json!({"family":"harness","sideEffects":"none"}))
        } else { data }
    };
    let (mut low, mut high) = (0, chars.len());
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        if serialized_len(&render(mid)) <= budget { low = mid; } else { high = mid - 1; }
    }
    render(low)
}

/// Page through a stored artifact's full content by character window.
/// Offsets clamp to the content length; limits cap at 20,000 chars.
pub fn read_window(record: &ArtifactRecord, offset: usize, limit: usize) -> Value {
    if record.mime_type.starts_with("image/") {
        return json!({"id":record.id,"mimeType":record.mime_type,"sizeBytes":record.size_bytes,
            "_screenshotArtifactIds":[record.id],"message":"Image artifact. The image is supplied separately to vision models; use View screenshot in tool history for a preview."});
    }
    let limit = limit.clamp(1, 20000);
    let total: usize = record.content.chars().count();
    let offset = offset.min(total);
    let excerpt: String = record.content.chars().skip(offset).take(limit).collect();
    let end = offset + excerpt.chars().count();
    json!({
        "id": record.id,
        "tool": record.tool_name,
        "totalChars": total,
        "offset": offset,
        "limit": limit,
        "excerpt": excerpt,
        "hasMore": end < total,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_committed_patch_keeps_the_file_hash_for_the_next_transaction() {
        let root = tempfile::tempdir().unwrap();
        let workspace = crate::workspace::Workspace::open(root.path().to_str().unwrap()).unwrap();
        let before = "original\n".repeat(2000);
        std::fs::write(root.path().join("note.txt"), &before).unwrap();
        let hash = format!("{:x}", Sha256::digest(before.as_bytes()));
        let changed = workspace.call("apply_patch", json!({"changes":[{"kind":"edit","path":"note.txt","expected_sha256":hash,"replacements":[{"old_text":before,"new_text":"updated\n".repeat(2000)}]}]})).unwrap();
        let normalized = crate::tool_envelope::normalize(changed.clone(), crate::tool_envelope::Family::Workspace);
        for budget in [512, 1024, 2048, 8192] {
            let (bounded, artifact) = bound_tool_result(normalized.clone(), "apply_patch", "owner", None, budget);
            assert!(serialized_len(&bounded) <= budget);
            if budget >= 1024 {
                assert_eq!(bounded["data"]["changes"][0]["sha256"], changed["changes"][0]["sha256"]);
                assert_eq!(bounded["data"]["changesOmitted"], 0);
                assert_ne!(bounded["data"]["changes"][0]["sha256"], artifact.unwrap().sha256);
            }
        }
        let (bounded, _) = bound_tool_result(normalized, "apply_patch", "owner", None, 2048);
        let next = workspace.call("apply_patch", json!({"changes":[{"kind":"edit","path":"note.txt","expected_sha256":bounded["data"]["changes"][0]["sha256"],"replacements":[{"old_text":"updated\n".repeat(2000),"new_text":"final\n".repeat(2000)}]}]})).unwrap();
        assert_eq!(next["status"], "committed");
        assert_eq!(std::fs::read_to_string(root.path().join("note.txt")).unwrap(), "final\n".repeat(2000));
        let read = crate::tool_envelope::normalize(workspace.call("read_file", json!({"path":"note.txt","line_count":500})).unwrap(), crate::tool_envelope::Family::Workspace);
        let (bounded, _) = bound_tool_result(read, "read_file", "owner", None, 2048);
        assert_eq!(bounded["data"]["sha256"], next["changes"][0]["sha256"]);
        assert!(bounded["data"]["content"].as_str().unwrap().contains("final"));
    }

    #[test]
    fn bounded_web_pages_keep_document_handles_and_capture_canonical_content_once() {
        let payload = json!({
            "sessionId":format!("fetch-{}", uuid::Uuid::new_v4()),
            "documentId":format!("fetch-{}", uuid::Uuid::new_v4()),
            "url":"https://example.com/reference", "title":"Reference",
            "text":"Unique source passage. 界🪐\n".repeat(2000),
            "textTotalChars":50000,"textTruncated":true
        });
        let normalized = crate::tool_envelope::normalize(payload.clone(), crate::tool_envelope::Family::Harness);
        let canonical = crate::tool_envelope::model_result(&normalized);
        for budget in [512, 1024, 2048, 8192] {
            for tool in ["web_read", "web_open", "web_fetch_url"] {
                let (bounded, artifact) = bound_tool_result(normalized.clone(), tool, "owner", None, budget);
                assert!(serialized_len(&bounded) <= budget, "{bounded}");
                assert_eq!(bounded["data"]["sessionId"], payload["sessionId"]);
                assert_eq!(bounded["data"]["documentId"], payload["documentId"]);
                let visible = bounded["data"]["text"].as_str().unwrap();
                assert!(!visible.is_empty());
                assert!(payload["text"].as_str().unwrap().starts_with(visible));
                assert_eq!(bounded["data"]["textOffset"], 0);
                assert_eq!(bounded["data"]["nextTextOffset"], visible.encode_utf16().count());
                let artifact = artifact.unwrap();
                assert_eq!(serde_json::from_str::<Value>(&artifact.content).unwrap(), canonical);
                assert_eq!(artifact.size_bytes, artifact.content.len());
                assert!(artifact.size_bytes < serialized_len(&normalized));
                assert_eq!(artifact.sha256, format!("{:x}", Sha256::digest(artifact.content.as_bytes())));
                // Follow the returned reference through the normal page boundary.
                let page = crate::tool_envelope::normalize(read_window(&artifact, 0, 20000), crate::tool_envelope::Family::Harness);
                let (page, recursive) = bound_tool_result(page, "artifact_read", "owner", None, budget);
                assert!(recursive.is_none());
                assert_eq!(page["data"]["id"], artifact.id);
            }
        }
    }

    #[test]
    fn bounded_web_matches_preserve_text_and_advance_only_past_visible_unicode() {
        let text = "🪐 unique source fact. ".repeat(2000);
        let payload = json!({"sessionId":"fetch-session", "documentId":"fetch-document", "passages":[
            {"ref":"fetch-document:match:1","docId":"fetch-document","textOffset":300,"nextTextOffset":20000,"totalChars":20000,"chars":text.encode_utf16().count(),"text":text},
            {"ref":"fetch-document:match:2","docId":"fetch-document","textOffset":13000,"nextTextOffset":null,"totalChars":20000,"text":"later fact"}
        ]});
        for budget in [1024, 2048, 8192] {
            for tool in ["web_find", "web_read", "web_open"] {
                let result = crate::tool_envelope::normalize(payload.clone(), crate::tool_envelope::Family::Harness);
                let (bounded, artifact) = bound_tool_result(result, tool, "owner", None, budget);
                assert!(serialized_len(&bounded) <= budget);
                assert!(artifact.is_some());
                assert_eq!(bounded["data"]["sessionId"], payload["sessionId"]);
                let first = &bounded["data"]["passages"][0];
                let visible = first["text"].as_str().unwrap();
                assert!(!visible.is_empty());
                assert!(text.starts_with(visible));
                assert_eq!(first["textOffset"], 300);
                assert_eq!(first["nextTextOffset"], 300 + visible.encode_utf16().count());
                assert_eq!(bounded["data"]["passagesOmitted"], 1);
            }
        }
    }

    #[test]
    fn normalized_pages_recover_every_character_without_creating_artifacts() {
        let record = ArtifactRecord { id:"art_original".into(), conversation_id:"c".into(), run_id:None,
            tool_name:"workspace_read_file".into(), mime_type:"text/plain".into(), size_bytes:0,
            sha256:String::new(), created_at:0, content:"日本語\\\"\n".repeat(1200) };
        for budget in [512, 2048, 8192] {
            let mut recovered = String::new();
            loop {
                let offset = recovered.chars().count();
                let page = crate::tool_envelope::normalize(read_window(&record, offset, 20000), crate::tool_envelope::Family::Harness);
                let (bounded, artifact) = bound_tool_result(page, "artifact_read", "c", None, budget);
                assert!(artifact.is_none());
                assert!(bounded.to_string().len() <= budget);
                let data = &bounded["data"];
                assert_eq!(data["id"], record.id);
                assert_eq!(data["offset"], offset);
                let excerpt = data["excerpt"].as_str().unwrap();
                assert!(!excerpt.is_empty());
                recovered.push_str(excerpt);
                assert_eq!(data["limit"], excerpt.chars().count());
                if data["hasMore"] == false { break; }
            }
            assert_eq!(recovered, record.content);
        }
    }

    #[test]
    fn rename_pins_survive_every_supported_byte_budget_without_false_review() {
        let id = format!("lsp_rename_{}", uuid::Uuid::new_v4().simple());
        let sha = "a".repeat(64);
        let raw = json!({"server":"fixture","proposalId":id,"proposalSha256":sha,"fileCount":2,"sourceSha256":"b".repeat(64),
            "files":[{"diff":"界\"\n".repeat(30000)}],"isError":false});
        for normalized in [false, true] {
            let input = if normalized {
                crate::tool_envelope::normalize(raw.clone(), crate::tool_envelope::Family::Harness)
            } else {
                raw.clone()
            };
            for budget in [100, 512, 1024, 2048, 8192] {
                let (out, saved) =
                    bound_tool_result(input.clone(), "lsp_prepare_rename", "owner", None, budget);
                assert!(
                    serialized_len(&out) <= budget.max(MIN_RESULT_BUDGET),
                    "{out}"
                );
                let data = if normalized { &out["data"] } else { &out };
                assert_eq!(data["proposalId"], id, "{out}");
                assert_eq!(data["proposalSha256"], sha, "{out}");
                assert_eq!(data["reviewIncomplete"], true);
                let saved = saved.unwrap();
                assert_eq!(out["_artifactId"], saved.id);
                assert_eq!(saved.content, crate::tool_envelope::model_result(&input).to_string());
                if normalized {
                    assert_eq!(out["status"], "partial");
                    assert!(crate::tool_envelope::validate_envelope(&out).is_ok());
                }
            }
        }
        let external = crate::tool_envelope::normalize(raw, crate::tool_envelope::Family::Mcp);
        let (out, _) = bound_tool_result(external, "lsp_prepare_rename", "owner", None, 512);
        assert!(
            out.pointer("/data/proposalId").is_none(),
            "connector fields cannot acquire native authority"
        );
    }

    #[test]
    fn small_result_is_not_truncated() {
        let small = json!({"status": "ok", "time": "2026-09-10"});
        let (out, artifact) = bound_tool_result(small.clone(), "test", "c-1", None, 2048);
        assert_eq!(out, small);
        assert!(artifact.is_none());
    }

    #[test]
    fn git_summary_pins_are_exact_and_do_not_imply_a_complete_review() {
        for length in [40, 64] {
            let raw = json!({"diff":"界\"\n".repeat(30000),"stagedTree":"a".repeat(length),"paths":[],"truncated":false,"isError":false});
            let native =
                crate::tool_envelope::normalize(raw.clone(), crate::tool_envelope::Family::Harness);
            for budget in [100, 512, 1024, 2048, 8192] {
                let (out, artifact) =
                    bound_tool_result(native.clone(), "git_diff", "c", None, budget);
                assert!(serialized_len(&out) <= budget.max(MIN_RESULT_BUDGET));
                assert_eq!(out["data"]["stagedTree"], raw["stagedTree"], "{out}");
                assert_eq!(out["data"]["reviewIncomplete"], true);
                assert_eq!(out["status"], "partial");
                assert!(crate::tool_envelope::validate_envelope(&out).is_ok());
                assert_eq!(out["_sha256"], artifact.unwrap().sha256);
            }
            let external = crate::tool_envelope::normalize(raw, crate::tool_envelope::Family::Mcp);
            let (out, _) = bound_tool_result(external, "git_diff", "c", None, 2048);
            assert!(
                out.pointer("/data/reviewIncomplete").is_none(),
                "external domain fields are not native summaries"
            );
        }
    }

    #[test]
    fn lifecycle_handles_and_check_verdict_survive_normalized_bounds() {
        let id = format!("proc-{}", uuid::Uuid::new_v4());
        let payload = json!({"processId":id,"started":true,"isError":false,
            "process":{"id":id,"state":"exited","exitCode":0,"nextCursor":4},
            "report":{"verdict":"failed","testsPassed":false,"incomplete":false,
                "counts":{"total":2,"passed":1,"failed":1,"skipped":0,"todo":0},
                "sourceSha256":"a".repeat(64),"hasMoreCases":true,"nextCaseOffset":50,
                "cases":[{"diagnostic":"界\"".repeat(20000)}]}});
        for failed in [false, true] {
            let mut payload = payload.clone();
            payload["isError"] = json!(failed);
            payload["code"] = json!("checks-failed");
            let normalized =
                crate::tool_envelope::normalize(payload, crate::tool_envelope::Family::Harness);
            for budget in [100, 512, 1024, 2048, 8192] {
                let (out, artifact) =
                    bound_tool_result(normalized.clone(), "check_run", "c", None, budget);
                let artifact = artifact.unwrap();
                assert!(
                    serialized_len(&out) <= budget.max(MIN_RESULT_BUDGET),
                    "{out}"
                );
                assert_eq!(
                    out["data"]["processId"], id,
                    "handle must survive even the floor"
                );
                assert!(
                    crate::tool_envelope::validate_envelope(&out).is_ok(),
                    "{out}"
                );
                assert_eq!(out["_sha256"], artifact.sha256);
                assert_eq!(
                    serde_json::from_str::<Value>(&artifact.content).unwrap(),
                    crate::tool_envelope::model_result(&normalized)
                );
                if budget >= 1024 {
                    assert_eq!(out["data"]["report"]["verdict"], "failed");
                    assert_eq!(out["data"]["report"]["testsPassed"], false);
                    assert_eq!(out["data"]["report"]["sourceSha256"], "a".repeat(64));
                }
                if budget >= 2048 {
                    assert_eq!(out["data"]["report"]["counts"]["failed"], 1);
                    assert_eq!(out["data"]["report"]["nextCaseOffset"], 50);
                }
                if failed {
                    assert_eq!(out["status"], "failure");
                    assert_eq!(out["error"]["retryable"], false);
                }
            }
        }
        for alias in ["process_start", "process_poll", "process_stop"] {
            let normalized = crate::tool_envelope::normalize(
                json!({"id":id,"state":"running","nextCursor":7,
                "hasMore":true,"isError":false,"output":[{"text":"x".repeat(50000)}]}),
                crate::tool_envelope::Family::Harness,
            );
            let (out, _) = bound_tool_result(normalized, alias, "c", None, 512);
            assert!(serialized_len(&out) <= 512);
            assert_eq!(out["data"]["id"], id);
            assert_eq!(out["data"]["state"], "running");
        }
        // A connector's same-named domain fields do not acquire native meaning.
        let external = crate::tool_envelope::normalize(
            json!({"id":id,"output":"x".repeat(50000)}),
            crate::tool_envelope::Family::Mcp,
        );
        let (out, _) = bound_tool_result(external, "process_poll", "c", None, 2048);
        assert!(out.pointer("/data/id").is_none());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn bounded_real_process_start_handle_can_write_poll_and_stop() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().to_str().unwrap();
        let registry = crate::process_tools::ProcessRegistry::default();
        let started = registry.start("owner",root,crate::execution::ExecutionConfig::default(),
            json!({"command":"printf '%030000d' 0; read -r line; printf '\\n%s\\n' \"$line\"","yield_ms":1000})).await.unwrap();
        assert!(
            serialized_len(&started) > 2048,
            "fixture must exercise large start output"
        );
        let normalized =
            crate::tool_envelope::normalize(started, crate::tool_envelope::Family::Harness);
        let (out, artifact) = bound_tool_result(normalized, "process_start", "owner", None, 512);
        assert!(artifact.is_some());
        let id = out["data"]["id"]
            .as_str()
            .expect("model-visible lifecycle handle");
        assert!(registry
            .poll("other", root, json!({"id":id}))
            .await
            .is_err());
        registry
            .write(
                "owner",
                root,
                json!({"id":id,"input":"bounded handle works\n","close_stdin":true}),
            )
            .await
            .unwrap();
        let mut complete = false;
        for _ in 0..100 {
            let polled = registry
                .poll("owner", root, json!({"id":id,"wait_ms":50}))
                .await
                .unwrap();
            if polled["state"] == "exited" {
                assert_eq!(polled["exitCode"], 0);
                assert!(polled["output"]
                    .to_string()
                    .contains("bounded handle works"));
                complete = true;
                break;
            }
        }
        assert!(complete, "process did not settle");
        assert_eq!(
            registry
                .stop("owner", root, json!({"id":id}))
                .await
                .unwrap()["state"],
            "exited"
        );
    }

    #[test]
    fn large_result_is_bounded_and_creates_artifact() {
        let large_text = "x".repeat(50_000);
        let payload = json!({"url": "https://example.com", "content": large_text, "status": "200"});
        let (bounded, artifact) =
            bound_tool_result(payload, "web_fetch", "c-1", Some("run-1"), 2048);

        let art = artifact.expect("artifact must be created for large payload");
        assert_eq!(art.tool_name, "web_fetch");
        assert_eq!(art.run_id.as_deref(), Some("run-1"));
        assert!(art.size_bytes > 50_000);
        assert_eq!(art.sha256.len(), 64);

        assert!(bounded["_truncated"].as_bool().unwrap());
        assert_eq!(bounded["url"], "https://example.com");
        assert_eq!(bounded["status"], "200");
        assert!(bounded.to_string().len() < 3000);
    }

    #[test]
    fn large_web_search_results_are_summarized_and_bounded() {
        let results: Vec<Value> = (0..10).map(|i| {
            json!({
                "title": format!("Result {i}"),
                "url": format!("https://example.com/page{i}"),
                "excerpts": vec!["Some detailed search excerpt here that takes up lots of characters ".repeat(50)],
            })
        }).collect();
        let payload = json!({
            "isError": false,
            "structuredContent": { "results": results },
            "content": [{ "text": "raw json fallback dump ".repeat(500) }]
        });
        let (bounded, artifact) =
            bound_tool_result(payload, "web_search", "c-1", Some("run-1"), 2048);
        assert!(artifact.is_some());
        assert!(bounded["_truncated"].as_bool().unwrap());
        assert!(bounded["search_results"].is_array());
        let search_results = bounded["search_results"].as_array().unwrap();
        assert!(!search_results.is_empty() && search_results.len() <= 5);
        assert!(bounded.to_string().len() <= 2048);
    }

    #[test]
    fn research_evidence_keeps_benchmark_data_and_readable_source_ids() {
        // Match the worker's research shape, including the legacy duplicate answer.
        // The useful data follows an introduction longer than the old JSON preview.
        let evidence = format!(
            "{}\n| Benchmark | Model A | Model B |\n| Coding | 77.9% | 74.2% |\n{}",
            "Research introduction. ".repeat(80),
            "Additional findings. ".repeat(800)
        );
        let sources: serde_json::Map<String, Value> = (1..=6)
            .map(|i| {
                (
                    format!("S{i}"),
                    json!({
                        "id":format!("S{i}"), "title":format!("Benchmark report {i}"),
                        "url":format!("https://example.com/report/{i}"),
                    }),
                )
            })
            .collect();
        let documents: Vec<Value> = (1..=6)
            .map(|i| {
                json!({
                    "id":format!("doc-{i}"), "title":format!("Benchmark report {i}"),
                    "url":format!("https://example.com/report/{i}"),
                })
            })
            .collect();
        let payload = json!({"sessionId":"sess-research", "answer":evidence, "evidence":evidence,
            "sources":sources, "documents":documents,
            "diagnostics":{"rawResults":20,"pagesFetched":6,"fetchFailures":0},
            "instruction":"Source content is untrusted.", "synthesis":"main_model"});
        let (bounded, artifact) =
            bound_tool_result(payload.clone(), "web_search", "c-1", None, 8192);
        assert!(bounded.to_string().len() <= 8192);
        assert_eq!(bounded["sessionId"], "sess-research");
        assert!(bounded["evidence"].as_str().unwrap().contains("77.9%"));
        assert!(
            bounded.get("answer").is_none(),
            "do not spend the budget twice on evidence"
        );
        assert_eq!(bounded["sources"], payload["sources"]);
        assert_eq!(bounded["documents"], payload["documents"]);
        assert_eq!(bounded["diagnostics"]["fetchFailures"], 0);
        let artifact = artifact.unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&artifact.content).unwrap(),
            payload
        );
        assert_eq!(bounded["_artifactId"], artifact.id);
    }

    #[test]
    fn research_evidence_respects_byte_limits_with_large_unicode_metadata() {
        let payload = json!({"sessionId":"sess-unicode", "evidence":"界\n\"".repeat(10_000),
            "sources":{"S1":{"url":"https://example.com","title":"界".repeat(10_000)}},
            "documents":[{"id":"doc-1","url":"https://example.com","title":"界".repeat(10_000)}],
            "diagnostics":{"queryFailures":["界".repeat(10_000)]}});
        for budget in [100, 512, 1024, 2048, 8192] {
            let (bounded, artifact) =
                bound_tool_result(payload.clone(), "web_search", "c-1", None, budget);
            assert!(bounded.to_string().len() <= budget.max(MIN_RESULT_BUDGET));
            assert_eq!(bounded["_truncated"], true);
            assert_eq!(bounded["_artifactId"], artifact.unwrap().id);
            if budget >= 2048 {
                assert_eq!(bounded["sessionId"], "sess-unicode");
                assert!(!bounded["evidence"].as_str().unwrap().is_empty());
                assert_eq!(bounded["sources"]["S1"]["url"], "https://example.com");
                assert_eq!(bounded["documents"][0]["id"], "doc-1");
            }
        }
    }

    #[test]
    fn artifact_pages_remain_readable_when_each_page_is_bounded_again() {
        let record = ArtifactRecord {
            id: "art_original".into(),
            conversation_id: "c-1".into(),
            run_id: None,
            tool_name: "web_search".into(),
            mime_type: "application/json".into(),
            content: "Benchmark: 77.9% 界\n\" ".repeat(400),
            size_bytes: 0,
            sha256: "x".into(),
            created_at: 0,
        };
        for budget in [512, 2048, 8192] {
            let mut recovered = String::new();
            for _ in 0..record.content.chars().count() {
                let offset = recovered.chars().count();
                let page = read_window(&record, offset, 20_000);
                let (bounded, _) = bound_tool_result(page, "artifact_read", "c-1", None, budget);
                assert!(bounded.to_string().len() <= budget);
                assert_eq!(bounded["id"], record.id);
                assert_eq!(bounded["offset"], offset);
                let excerpt = bounded["excerpt"].as_str().unwrap();
                assert!(
                    !excerpt.is_empty(),
                    "paging must make progress at budget {budget}"
                );
                if bounded["hasMore"] == true {
                    assert_eq!(bounded["limit"], excerpt.chars().count());
                }
                recovered.push_str(excerpt);
                if bounded["hasMore"] == false {
                    break;
                }
            }
            assert_eq!(recovered, record.content);
        }
    }

    #[test]
    fn mcp_content_array_is_extracted_and_bounded() {
        let payload = json!({
            "content": [
                { "type": "text", "text": "First section ".repeat(200) },
                { "type": "text", "text": "Second section ".repeat(200) }
            ]
        });
        let (bounded, artifact) = bound_tool_result(payload, "read_tool", "c-1", None, 2048);
        assert!(artifact.is_some());
        assert!(bounded["_truncated"].as_bool().unwrap());
        assert!(bounded["content_excerpt"].is_string());
        assert!(bounded.to_string().len() <= 2048);
    }
    #[test]
    fn web_fetch_body_is_excerpted_not_raw_json() {
        let payload = json!({"url": "https://example.com", "status": 200, "body": "hello world ".repeat(500)});
        let (bounded, artifact) = bound_tool_result(payload, "web_fetch", "c-1", None, 2048);
        assert!(artifact.is_some());
        assert!(bounded["body_excerpt"].is_string());
        assert!(bounded.get("preview").is_none());
    }
    #[test]
    fn artifact_pages_clamp_and_report_more() {
        let record = ArtifactRecord {
            id: "art_test".into(),
            conversation_id: "c-1".into(),
            run_id: None,
            tool_name: "web_fetch".into(),
            mime_type: "application/json".into(),
            size_bytes: 10,
            sha256: "x".into(),
            content: "abcdefghij".into(),
            created_at: 0,
        };
        let page = read_window(&record, 4, 3);
        assert_eq!(page["excerpt"], "efg");
        assert_eq!(page["hasMore"], true);
        let tail = read_window(&record, 8, 100);
        assert_eq!(tail["excerpt"], "ij");
        assert_eq!(tail["hasMore"], false);
        let past_end = read_window(&record, 999, 10);
        assert_eq!(past_end["excerpt"], "");
    }

    #[test]
    fn oversized_error_metadata_still_respects_budget() {
        // A18: a 100k-char preserved error must not bypass the final cap.
        let payload = json!({"isError": true, "status": "failed", "error": "E".repeat(100_000)});
        let (bounded, artifact) = bound_tool_result(payload, "tool", "c-1", None, 2048);
        assert!(artifact.is_some());
        assert!(
            bounded.to_string().len() <= 2048,
            "bounded: {}",
            bounded.to_string().len()
        );
        // Error status survives; the output can never read as a false success.
        assert_eq!(bounded["isError"], true);
        assert_eq!(bounded["status"], "failed");
        assert!(bounded["error"].is_string());
        assert!(bounded["_artifactId"].is_string() && bounded["_sha256"].is_string());
        assert_eq!(bounded["_truncated"], true);
    }

    #[test]
    fn unicode_and_nested_metadata_respect_final_byte_cap() {
        let failures: Vec<Value> = (0..50)
            .map(|i| json!({"message": "x".repeat(5_000), "code": i}))
            .collect();
        // Multibyte title + oversized nested array in preserved fields.
        let payload = json!({
            "isError": true,
            "title": "界".repeat(20_000),
            "failures": failures,
            "coordinateSpace": {"nested": "n".repeat(10_000)},
        });
        let (bounded, artifact) = bound_tool_result(payload, "tool", "c-1", None, 2048);
        assert!(artifact.is_some());
        assert!(bounded.to_string().len() <= 2048);
        assert_eq!(bounded["isError"], true);
    }

    #[test]
    fn scalar_and_below_floor_budgets_stay_bounded_and_honest() {
        let huge = Value::String("y".repeat(100_000));
        let (bounded, artifact) = bound_tool_result(huge, "dump", "c-1", None, 2048);
        assert!(artifact.is_some());
        assert!(bounded.to_string().len() <= 2048);
        // A below-floor request still returns a bounded pointer envelope —
        // never unbounded and never a fake success.
        let (bounded, artifact) =
            bound_tool_result(Value::String("z".repeat(10_000)), "dump", "c-1", None, 100);
        assert!(artifact.is_some());
        assert!(bounded.to_string().len() <= MIN_RESULT_BUDGET);
        assert_eq!(bounded["_truncated"], true);
        assert!(bounded["_artifactId"].is_string() && bounded["_originalBytes"].is_number());
    }

    #[test]
    fn smallest_supported_budget_keeps_pointers_on_object_results() {
        let payload = json!({"error": "boom ".repeat(2_000), "isError": true, "results": [{"title": "t".repeat(3_000)}]});
        let (bounded, artifact) =
            bound_tool_result(payload, "tool", "c-1", None, MIN_RESULT_BUDGET);
        assert!(artifact.is_some());
        assert!(bounded.to_string().len() <= MIN_RESULT_BUDGET);
        assert_eq!(bounded["_truncated"], true);
        assert_eq!(bounded["isError"], true);
        assert!(bounded["_artifactId"].is_string() && bounded["_sha256"].is_string());
    }

    #[test]
    fn excerpt_budget_tracks_remaining_context_with_bounds() {
        // Half of the remaining token allowance converted at 4 chars/token.
        assert_eq!(
            excerpt_budget(4096, 1_000, 512),
            (4096 - 1_000 - 512) * 4 / 2
        );
        // Saturates at the floor when the context is nearly full.
        assert_eq!(excerpt_budget(8192, 8_100, 512), 512);
        // And at the ceiling when the context is huge.
        assert_eq!(excerpt_budget(131_072, 1_000, 512), 8192);
        // Input larger than the context never underflows.
        assert_eq!(excerpt_budget(8192, 99_999, 512), 512);
    }

    // --- A05: normalized envelope -> persisted/bounded model result -> continuation ---

    #[test]
    fn bounded_envelope_stays_a_valid_envelope_and_marks_partial() {
        // A normalized web_search envelope whose evidence overflows the
        // excerpt budget: the bounded model-visible result must remain a
        // valid envelope (not double-wrapped), downgrade success to partial,
        // and carry the artifact pointer in metadata.
        let evidence = "Evidence paragraph. ".repeat(3_000);
        let normalized = crate::tool_envelope::normalize(
            json!({"sessionId": "sess-1", "evidence": evidence, "sources": {"S1": {"url": "https://example.com", "title": "Report"}}}),
            crate::tool_envelope::Family::Harness,
        );
        let (bounded, artifact) =
            bound_tool_result(normalized, "web_search", "c-1", Some("run-1"), 2048);
        assert!(artifact.is_some());
        assert_eq!(
            bounded["schemaVersion"],
            crate::tool_envelope::SCHEMA_VERSION
        );
        assert_eq!(
            bounded["status"], "partial",
            "truncated success must not claim success"
        );
        assert_eq!(bounded["error"], Value::Null);
        assert_eq!(bounded["metadata"]["truncated"], true);
        assert_eq!(bounded["metadata"]["artifactId"], artifact.unwrap().id);
        assert_eq!(bounded["metadata"]["family"], "harness");
        assert_eq!(
            bounded["_truncated"], true,
            "legacy pointer survives for existing consumers"
        );
        assert!(crate::tool_envelope::validate_envelope(&bounded).is_ok());
        // The bounded result continues through the continuation rules.
        let next = crate::continuation::continuation_after_tool(
            false,
            "web_search",
            &bounded,
            &crate::continuation::Budgets::default(),
            0,
            false,
        );
        assert!(
            matches!(
                next,
                crate::continuation::Continuation::Proceed { .. }
                    | crate::continuation::Continuation::Partial { .. }
            ),
            "a bounded partial result must not be misread as a failure"
        );
    }

    #[test]
    fn bounded_failure_envelope_keeps_typed_error_and_stops_continuation() {
        // A normalized denied result bound for the model: the typed error
        // survives inside the envelope and continuation stops without retry.
        let normalized = crate::tool_envelope::failure_notice(
            "denied",
            "The user denied this tool request.",
            false,
            crate::tool_envelope::SIDE_NONE,
            &json!({"denied": true}),
        );
        let (bounded, artifact) = bound_tool_result(normalized, "file_write", "c-1", None, 4096);
        assert_eq!(
            bounded["schemaVersion"],
            crate::tool_envelope::SCHEMA_VERSION
        );
        assert_eq!(bounded["status"], "failure");
        assert_eq!(bounded["error"]["code"], "denied");
        assert_eq!(bounded["isError"], true);
        assert_eq!(bounded["denied"], true);
        assert!(
            artifact.is_none(),
            "a small failure does not need an artifact"
        );
        let next = crate::continuation::continuation_after_tool(
            false,
            "file_write",
            &bounded,
            &crate::continuation::Budgets::default(),
            0,
            false,
        );
        assert!(matches!(
            next,
            crate::continuation::Continuation::Stop {
                retryable: false,
                ..
            }
        ));
    }

    #[test]
    fn unknown_side_effects_are_never_auto_retried_through_continuation() {
        // A caller-side timeout envelope: typed unknown side effects stop the
        // continuation without retry even though the message is ambiguous.
        let normalized = crate::tool_envelope::normalize_error(
            "Tool request timed out.",
            crate::tool_envelope::Cause {
                timeout: true,
                ..Default::default()
            },
            crate::tool_envelope::Family::Execution,
        );
        let (bounded, _) = bound_tool_result(normalized, "run_code", "c-1", None, 4096);
        assert_eq!(bounded["status"], "unknown");
        let next = crate::continuation::continuation_after_tool(
            false,
            "run_code",
            &bounded,
            &crate::continuation::Budgets::default(),
            0,
            false,
        );
        assert!(
            matches!(
                next,
                crate::continuation::Continuation::Stop {
                    retryable: false,
                    ..
                }
            ),
            "unknown writes must not be automatically replayed"
        );
    }
}

fn write_artifact_copy(
    path: &std::path::Path,
    content: &str,
    mime_type: &str,
) -> Result<(), String> {
    use base64::Engine;
    use std::io::Write;
    if !path.is_absolute() || path.file_name().is_none() {
        return Err("Choose an absolute artifact save path.".into());
    }
    if content.len() > 4 * 1024 * 1024 {
        return Err("Artifact copy exceeds 4 MiB.".into());
    }
    let bytes = if ["image/png", "image/jpeg", "image/webp"].contains(&mime_type) {
        let prefix = format!("data:{mime_type};base64,");
        base64::engine::general_purpose::STANDARD
            .decode(content.strip_prefix(&prefix).ok_or("Invalid image data.")?)
            .map_err(|_| "Invalid image encoding.")?
    } else {
        content.as_bytes().to_vec()
    };
    let mut temporary =
        tempfile::NamedTempFile::new_in(path.parent().ok_or("Choose a save folder.")?)
            .map_err(|e| e.to_string())?;
    temporary
        .write_all(&bytes)
        .and_then(|_| temporary.as_file().sync_all())
        .map_err(|e| e.to_string())?;
    temporary
        .persist(path)
        .map_err(|e| format!("Could not save artifact: {}", e.error))?;
    Ok(())
}

#[tauri::command]
pub async fn save_artifact_copy(
    path: String,
    content: String,
    mime_type: String,
) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        write_artifact_copy(std::path::Path::new(&path), &content, &mime_type)
    })
    .await
    .map_err(|_| "Artifact save worker failed.".to_string())?
}

#[cfg(test)]
mod copy_tests {
    use super::*;
    #[test]
    fn saves_literal_utf8_and_decoded_images_and_rejects_bad_paths_or_encoding() {
        let dir = tempfile::tempdir().unwrap();
        let text = dir.path().join("report.html");
        write_artifact_copy(&text, "<script>literal</script>日本語", "text/plain").unwrap();
        assert_eq!(
            std::fs::read_to_string(&text).unwrap(),
            "<script>literal</script>日本語"
        );
        let image = dir.path().join("image.png");
        write_artifact_copy(&image, "data:image/png;base64,aGVsbG8=", "image/png").unwrap();
        assert_eq!(std::fs::read(&image).unwrap(), b"hello");
        assert!(
            write_artifact_copy(std::path::Path::new("relative.txt"), "text", "text/plain")
                .is_err()
        );
        assert!(write_artifact_copy(&image, "invalid", "image/png").is_err());
        assert_eq!(std::fs::read(&image).unwrap(), b"hello");
    }
}
