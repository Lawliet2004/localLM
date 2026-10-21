//! Tool-calling compatibility is a fingerprint of the runtime build, the model,
//! the chat template, and the parser. A path-only historical flag is unknown.

pub const PARSER_VERSION: &str = "tool-parser-2";

pub fn selection_fingerprint(runtime: &str, model: &str) -> String {
    fn identity(path: &str) -> String {
        let metadata = std::fs::metadata(path).ok();
        let modified = metadata
            .as_ref()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|t| t.as_nanos())
            .unwrap_or(0);
        format!(
            "{}|{}|{modified}",
            path,
            metadata.map(|m| m.len()).unwrap_or(0)
        )
    }
    fingerprint(
        &identity(runtime),
        &identity(model),
        if model.ends_with(crate::model_catalog::MODEL_FILENAME) {
            "minicpm"
        } else {
            "default"
        },
        PARSER_VERSION,
    )
}

fn probe_supported(value: &serde_json::Value) -> bool {
    let Some(calls) = value["choices"][0]["message"]["tool_calls"].as_array() else {
        return false;
    };
    if calls.len() != 1
        || calls[0]["type"] != "function"
        || calls[0]["id"]
            .as_str()
            .is_none_or(|id| id.is_empty() || id.len() > 128)
        || calls[0]["function"]["name"] != "locallm_probe"
        || !matches!(
            value["choices"][0]["finish_reason"].as_str(),
            Some("tool_calls" | "stop")
        )
    {
        return false;
    }
    let arguments = &calls[0]["function"]["arguments"];
    let parsed = arguments
        .as_str()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .unwrap_or_else(|| arguments.clone());
    parsed == serde_json::json!({"value":42})
}

#[tauri::command]
pub async fn retest_tool_calling(
    state: tauri::State<'_, crate::AppState>,
) -> Result<serde_json::Value, String> {
    use futures_util::StreamExt;
    use serde_json::json;
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before probing tool calling.")?;
    let (endpoint, key, model_path) = {
        let mut runtime = state.runtime.lock().await;
        let status = runtime.inspect();
        if status.phase != "ready" {
            return Err("Load a local model before testing native tool calling.".into());
        }
        (
            runtime.endpoint.clone(),
            runtime.api_key.clone(),
            status.model_path.unwrap_or_default(),
        )
    };
    let preferences = state.database()?.preferences()?;
    let saved_model = std::fs::canonicalize(&preferences.model_path)
        .map_err(|_| "The saved model file is unavailable.")?;
    let loaded_model =
        std::fs::canonicalize(&model_path).map_err(|_| "The loaded model file is unavailable.")?;
    if saved_model != loaded_model {
        return Err(
            "Apply the saved model selection before testing its tool compatibility.".into(),
        );
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let mut cancellation = state.cancel.subscribe();
    let operation = async {
        let response = client.post(format!("{}/v1/chat/completions", endpoint.trim_end_matches('/'))).bearer_auth(&key).json(&json!({
            "messages":[{"role":"user","content":"Call locallm_probe with value 42. Do not answer in prose."}],
            "tools":[{"type":"function","function":{"name":"locallm_probe","description":"Return the supplied test value. This diagnostic never executes a tool.","parameters":{"type":"object","properties":{"value":{"type":"integer","const":42}},"required":["value"],"additionalProperties":false}}}],
            "tool_choice":{"type":"function","function":{"name":"locallm_probe"}},
            "temperature":0,"max_tokens":256,"stream":false,"cache_prompt":false,"chat_template_kwargs":{"enable_thinking":false}
        })).send().await.map_err(|_| "Tool probe could not reach the local runtime.".to_string())?;
        if !response.status().is_success() {
            return Err(format!(
                "Local runtime rejected the probe (HTTP {}). No compatibility state changed.",
                response.status()
            ));
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| "Tool probe response was interrupted.")?;
            if bytes.len() + chunk.len() > 65536 {
                return Err("Tool probe response exceeded 64 KiB.".into());
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| "Tool probe returned invalid JSON.")?;
        if !matches!(
            value["choices"][0]["finish_reason"].as_str(),
            Some("stop" | "tool_calls")
        ) {
            return Err(
                "Tool probe did not finish cleanly. No compatibility state changed.".into(),
            );
        }
        let supported = probe_supported(&value);
        state.database()?.record_tool_compatibility(
            &selection_fingerprint(&preferences.runtime_path, &preferences.model_path),
            if supported {
                "supported"
            } else {
                "unsupported"
            },
        )?;
        Ok(
            json!({"supported":supported,"message":if supported { "Native structured tool call verified. No tool action was executed." } else { "This probe did not produce the requested native tool call. Text chat remains available; tools are disabled for this exact runtime and model until a successful retest or replacement." }}),
        )
    };
    tokio::select! { result = operation => result, _ = cancellation.changed() => Err("Tool probe cancelled. No compatibility state changed.".into()) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompatState {
    Unknown,
    Supported,
    Unsupported,
}

impl CompatState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Supported => "supported",
            Self::Unsupported => "unsupported",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "supported" => Self::Supported,
            "unsupported" => Self::Unsupported,
            _ => Self::Unknown,
        }
    }
}

/// Historical `local_tool_calling_support` booleans came from malformed text
/// calls. Both true and false are unknown until a fingerprint probe runs.
pub fn interpret_historical_flag(_stored: Option<bool>) -> CompatState {
    CompatState::Unknown
}

pub fn fingerprint(
    runtime_identity: &str,
    model_identity: &str,
    template: &str,
    parser: &str,
) -> String {
    format!("{runtime_identity}\u{1f}{model_identity}\u{1f}{template}\u{1f}{parser}")
}

/// Identity is the build label plus size and mtime. The directory path is not
/// sufficient: replacing the binary at the same path changes this string.
pub fn runtime_identity(build_label: &str, size: u64, modified_secs: u64) -> String {
    format!("{build_label}|{size}|{modified_secs}")
}

pub fn build_label_from_path(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    if let Some(rest) = name.strip_prefix("llama-") {
        let label = rest.split('-').take(2).collect::<Vec<_>>().join("-");
        if !label.is_empty() {
            return label;
        }
    }
    if let Some(index) = name.find("prism-") {
        return name[index..]
            .split(['/', '\\'])
            .next()
            .unwrap_or(name)
            .to_string();
    }
    "unknown-build".into()
}

/// Tools stay enabled unless this exact fingerprint was probed unsupported.
pub fn tools_enabled(state: CompatState) -> bool {
    !matches!(state, CompatState::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn probe_requires_an_actual_completed_structured_call() {
        use serde_json::json;
        assert!(probe_supported(
            &json!({"choices":[{"finish_reason":"tool_calls","message":{"tool_calls":[{"id":"probe-call", "type":"function", "function":{"name":"locallm_probe","arguments":"{\"value\":42}"}}]}}]})
        ));
        for value in [
            json!({"choices":[{"finish_reason":"stop","message":{"content":"locallm_probe({value:42})"}}]}),
            json!({"choices":[{"finish_reason":"length","message":{"tool_calls":[{"id":"probe-call", "type":"function", "function":{"name":"locallm_probe","arguments":"{\"value\":42}"}}]}}]}),
        ] {
            assert!(!probe_supported(&value));
        }
    }

    #[test]
    fn historical_flags_are_unknown_and_do_not_disable_tools() {
        assert_eq!(interpret_historical_flag(Some(false)), CompatState::Unknown);
        assert_eq!(interpret_historical_flag(Some(true)), CompatState::Unknown);
        assert_eq!(interpret_historical_flag(None), CompatState::Unknown);
        assert!(tools_enabled(CompatState::Unknown));
        assert!(!tools_enabled(CompatState::Unsupported));
    }

    #[test]
    fn replacing_a_runtime_at_the_same_path_invalidates_compatibility() {
        let path = r"C:\runtime\llama-b10855-cuda12.4-aaaa";
        let model = "MiniCPM5-2B.Q6_K.gguf";
        let template = "minicpm";
        let before = fingerprint(
            &runtime_identity(&build_label_from_path(path), 100, 1),
            model,
            template,
            PARSER_VERSION,
        );
        let after = fingerprint(
            &runtime_identity(&build_label_from_path(path), 200, 2),
            model,
            template,
            PARSER_VERSION,
        );
        assert_ne!(before, after);
        assert_eq!(build_label_from_path(path), "b10855-cuda12.4");
        // A stale unsupported result for `before` does not apply to `after`.
        assert!(tools_enabled(interpret_historical_flag(Some(false))));
        assert_eq!(CompatState::parse("nope"), CompatState::Unknown);
    }
}
