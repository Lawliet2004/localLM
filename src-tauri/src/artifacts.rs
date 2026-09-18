use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const DEFAULT_MAX_RESULT_CHARS: usize = 2048;

/// Excerpt budget in characters for one tool result, derived from the tokens
/// still free after the counted input and the response reserve. At most half
/// of the remaining allowance is spent, so the next model round keeps
/// headroom; the budget stays within [512, 8192] characters either way.
pub fn excerpt_budget(context_length: u32, input_tokens: u64, response_reserve: u32) -> usize {
    let used = input_tokens.min(u64::from(context_length)) as u32;
    let remaining = context_length.saturating_sub(used).saturating_sub(response_reserve);
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

/// Bound a tool output so huge outputs (e.g. web pages, dumps, large files) do not blow up model context.
/// When output exceeds `max_chars`, full output is captured in an `ArtifactRecord`, and a structured excerpt
/// with truncation indicators and artifact ID is returned to the model.
pub fn bound_tool_result(
    result: Value,
    tool_name: &str,
    conversation_id: &str,
    run_id: Option<&str>,
    max_chars: usize,
) -> (Value, Option<ArtifactRecord>) {
    let raw = result.to_string();
    let size_bytes = raw.len();
    if size_bytes <= max_chars {
        return (result, None);
    }

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

    // Create a bounded representation preserving key error or status indicators
    let excerpt = if let Some(obj) = result.as_object() {
        let mut bounded_obj = serde_json::Map::new();
        // Preserve critical metadata fields
        for (k, v) in obj {
            if matches!(k.as_str(), "isError" | "status" | "code" | "error" | "title" | "url" | "source") {
                bounded_obj.insert(k.clone(), v.clone());
            }
        }
        bounded_obj.insert("_truncated".into(), Value::Bool(true));
        bounded_obj.insert("_artifactId".into(), Value::String(id.clone()));
        bounded_obj.insert("_originalBytes".into(), json!(size_bytes));
        bounded_obj.insert("_sha256".into(), Value::String(sha256));

        // Find primary content field or provide truncated overview
        let mut handled = false;
        if let Some(results) = obj.get("structuredContent").and_then(|sc| sc.get("results")).and_then(Value::as_array) {
            let mut snippets = Vec::new();
            for r in results.iter().take(5) {
                let title = r.get("title").and_then(Value::as_str).unwrap_or("");
                let url = r.get("url").and_then(Value::as_str).unwrap_or("");
                let excerpt = if let Some(excerpts) = r.get("excerpts").and_then(Value::as_array) {
                    excerpts.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")
                } else if let Some(snippet) = r.get("snippet").and_then(Value::as_str) {
                    snippet.to_string()
                } else {
                    String::new()
                };
                let mut item = serde_json::Map::new();
                if !title.is_empty() { item.insert("title".into(), json!(title)); }
                if !url.is_empty() { item.insert("url".into(), json!(url)); }
                if !excerpt.is_empty() {
                    let clipped: String = excerpt.chars().take(280).collect();
                    item.insert("snippet".into(), json!(clipped));
                }
                if !item.is_empty() { snippets.push(Value::Object(item)); }
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
                        if !combined.is_empty() { combined.push_str("\n\n"); }
                        combined.push_str(t);
                    }
                }
                if !combined.is_empty() {
                    let take_len = max_chars.min(1500);
                    let slice: String = combined.chars().take(take_len).collect();
                    bounded_obj.insert("content_excerpt".into(), Value::String(slice));
                    handled = true;
                }
            }
        }

        if !handled {
            if let Some(content_str) = obj.get("content").and_then(Value::as_str) {
                let take_len = max_chars.min(1500);
                let slice: String = content_str.chars().take(take_len).collect();
                bounded_obj.insert("content_excerpt".into(), Value::String(slice));
            } else if let Some(text_str) = obj.get("text").and_then(Value::as_str) {
                let take_len = max_chars.min(1500);
                let slice: String = text_str.chars().take(take_len).collect();
                bounded_obj.insert("text_excerpt".into(), Value::String(slice));
            } else if let Some(body_str) = obj.get("body").and_then(Value::as_str) {
                // web_fetch returns {url, status, body}: excerpt the cleaned text.
                let take_len = max_chars.min(1500);
                let slice: String = body_str.chars().take(take_len).collect();
                bounded_obj.insert("body_excerpt".into(), Value::String(slice));
            } else {
                let fallback: String = result.to_string().chars().take(1200).collect();
                bounded_obj.insert("preview".into(), Value::String(fallback));
            }
        }

        bounded_obj.insert(
            "_notice".into(),
            Value::String(format!(
                "Output was truncated (original size: {size_bytes} bytes) to protect context budget. Complete artifact saved as '{id}'."
            )),
        );

        // Enforce strict max_chars boundary on the final serialized object
        let mut cur_len = serde_json::to_string(&bounded_obj).map_or(0, |s| s.len());
        while cur_len > max_chars {
            let has_search_results = bounded_obj.get("search_results")
                .and_then(Value::as_array)
                .is_some_and(|a| a.len() > 1);
            if has_search_results {
                if let Some(Value::Array(arr)) = bounded_obj.get_mut("search_results") {
                    arr.pop();
                }
                cur_len = serde_json::to_string(&bounded_obj).map_or(0, |s| s.len());
            } else {
                break;
            }
        }
        if cur_len > max_chars {
            for key in ["content_excerpt", "text_excerpt", "body_excerpt", "preview"] {
                if let Some(Value::String(s)) = bounded_obj.get(key) {
                    let excess = cur_len.saturating_sub(max_chars);
                    let new_take = s.len().saturating_sub(excess + 32);
                    let trimmed: String = s.chars().take(new_take).collect();
                    bounded_obj.insert(key.into(), Value::String(trimmed));
                    break;
                }
            }
        }

        Value::Object(bounded_obj)
    } else {
        let text = result.as_str().unwrap_or(&raw);
        let preview: String = text.chars().take(1500).collect();
        json!({
            "_truncated": true,
            "_artifactId": id,
            "_originalBytes": size_bytes,
            "_sha256": sha256,
            "preview": preview,
            "_notice": format!("Output was truncated ({size_bytes} bytes). Complete artifact saved as '{id}'.")
        })
    };

    (excerpt, Some(artifact))
}

/// Page through a stored artifact's full content by character window.
/// Offsets clamp to the content length; limits cap at 20,000 chars.
pub fn read_window(record: &ArtifactRecord, offset: usize, limit: usize) -> Value {
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
    fn small_result_is_not_truncated() {
        let small = json!({"status": "ok", "time": "2026-09-10"});
        let (out, artifact) = bound_tool_result(small.clone(), "test", "c-1", None, 2048);
        assert_eq!(out, small);
        assert!(artifact.is_none());
    }

    #[test]
    fn large_result_is_bounded_and_creates_artifact() {
        let large_text = "x".repeat(50_000);
        let payload = json!({"url": "https://example.com", "content": large_text, "status": "200"});
        let (bounded, artifact) = bound_tool_result(payload, "web_fetch", "c-1", Some("run-1"), 2048);
        
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
        let (bounded, artifact) = bound_tool_result(payload, "web_search", "c-1", Some("run-1"), 2048);
        assert!(artifact.is_some());
        assert!(bounded["_truncated"].as_bool().unwrap());
        assert!(bounded["search_results"].is_array());
        let search_results = bounded["search_results"].as_array().unwrap();
        assert!(!search_results.is_empty() && search_results.len() <= 5);
        assert!(bounded.to_string().len() <= 2048);
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
            id: "art_test".into(), conversation_id: "c-1".into(), run_id: None,
            tool_name: "web_fetch".into(), mime_type: "application/json".into(),
            size_bytes: 10, sha256: "x".into(), content: "abcdefghij".into(), created_at: 0,
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
    fn excerpt_budget_tracks_remaining_context_with_bounds() {
        // Half of the remaining token allowance converted at 4 chars/token.
        assert_eq!(excerpt_budget(4096, 1_000, 512), (4096 - 1_000 - 512) * 4 / 2);
        // Saturates at the floor when the context is nearly full.
        assert_eq!(excerpt_budget(8192, 8_100, 512), 512);
        // And at the ceiling when the context is huge.
        assert_eq!(excerpt_budget(131_072, 1_000, 512), 8192);
        // Input larger than the context never underflows.
        assert_eq!(excerpt_budget(8192, 99_999, 512), 512);
    }
}
