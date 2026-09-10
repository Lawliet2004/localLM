use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub const DEFAULT_MAX_RESULT_CHARS: usize = 2048;

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
        if let Some(content_str) = obj.get("content").and_then(Value::as_str) {
            let take_len = max_chars.min(1500);
            let slice: String = content_str.chars().take(take_len).collect();
            bounded_obj.insert("content_excerpt".into(), Value::String(slice));
        } else if let Some(text_str) = obj.get("text").and_then(Value::as_str) {
            let take_len = max_chars.min(1500);
            let slice: String = text_str.chars().take(take_len).collect();
            bounded_obj.insert("text_excerpt".into(), Value::String(slice));
        } else {
            let fallback: String = result.to_string().chars().take(1200).collect();
            bounded_obj.insert("preview".into(), Value::String(fallback));
        }
        bounded_obj.insert(
            "_notice".into(),
            Value::String(format!(
                "Output was truncated (original size: {size_bytes} bytes) to protect context budget. Complete artifact saved as '{id}'."
            )),
        );
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
}
