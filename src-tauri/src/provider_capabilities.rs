//! Persisted model controls and their provenance. An endpoint reporting a
//! capability is distinct from a user override and from a successful inference
//! probe (which cannot establish all optional capabilities).
use crate::inference::InferenceProvider;
use crate::providers::{RemoteModel, ToolSupport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Source {
    #[default]
    Configured,
    ProviderReported,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum TokenField {
    #[default]
    ProtocolDefault,
    MaxTokens,
    MaxCompletionTokens,
    MaxOutputTokens,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Controls {
    pub source: Source,
    pub endpoint_fingerprint: Option<String>,
    pub temperature: ToolSupport,
    pub top_p: ToolSupport,
    pub reasoning_effort: ToolSupport,
    pub tool_choice: ToolSupport,
    pub parallel_tool_calls: ToolSupport,
    pub structured_output: ToolSupport,
    pub token_limit_field: TokenField,
}

impl Controls {
    pub fn bind_endpoint(&mut self, format: &str, base: &str, model_id: &str) {
        self.endpoint_fingerprint = Some(endpoint_fingerprint(format, base, model_id));
    }
    pub fn validate_endpoint(
        &self,
        format: &str,
        base: &str,
        model_id: &str,
    ) -> Result<(), String> {
        self.validate_protocol(format)?;
        if self
            .endpoint_fingerprint
            .as_ref()
            .is_some_and(|pin| pin.len() != 64 || !pin.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err("Invalid model capability endpoint fingerprint.".into());
        }
        if self.source == Source::ProviderReported
            && self.endpoint_fingerprint.as_deref()
                != Some(endpoint_fingerprint(format, base, model_id).as_str())
        {
            return Err("Reported capabilities belong to another endpoint, API format or model ID. Fetch models again, or configure explicit controls for this model.".into());
        }
        Ok(())
    }
    pub fn validate_protocol(&self, format: &str) -> Result<(), String> {
        use crate::providers::{CLAUDE_MESSAGES, OPENAI_RESPONSES};
        let valid = match format {
            OPENAI_RESPONSES => matches!(
                self.token_limit_field,
                TokenField::ProtocolDefault | TokenField::MaxOutputTokens
            ),
            CLAUDE_MESSAGES => matches!(
                self.token_limit_field,
                TokenField::ProtocolDefault | TokenField::MaxTokens
            ),
            _ => !matches!(self.token_limit_field, TokenField::MaxOutputTokens),
        };
        if valid {
            Ok(())
        } else {
            Err(
                "Configured output-token field is incompatible with the selected API format."
                    .into(),
            )
        }
    }
}
fn endpoint_fingerprint(format: &str, base: &str, model_id: &str) -> String {
    use sha2::{Digest, Sha256};
    format!(
        "{:x}",
        Sha256::digest(
            json!([format, base.trim().trim_end_matches('/'), model_id])
                .to_string()
                .as_bytes()
        )
    )
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCapabilities {
    pub tool_support: ToolSupport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<Controls>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<ToolProbe>,
}

/// One recorded tool-call probe for a (provider, model) pair. The stored
/// fingerprint fields make the record self-invalidating: when the saved
/// model's tool-related settings change the record stops matching.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ToolProbe {
    pub verified: bool,
    pub supported: bool,
    pub detail: String,
    pub probed_at: String,
    pub base_url: String,
    pub model_id: String,
    pub tool_support: ToolSupport,
}

pub fn probe_key(provider_id: &str, model_id: &str) -> String {
    format!("tool_probe:{provider_id}:{model_id}")
}

/// The stored probe only counts while the saved settings that shape tool
/// calling (base URL, model id, support override) still match.
/// Capabilities for a saved remote model: persisted controls plus a valid
/// probe. A verified negative probe demotes support to Unsupported, which
/// drops tools everywhere apply_tool_support / supports_tools is consulted.
#[cfg(test)]
/// The stored probe only counts while the saved settings that shape tool
/// calling (base URL, model id, support override) still match.
pub fn load_probe(
    store: &crate::store::Store,
    provider: &crate::providers::ProviderConnection,
    model: &crate::providers::RemoteModel,
) -> Option<ToolProbe> {
    let probe: Option<ToolProbe> = store.setting(&probe_key(&provider.id, &model.id)).ok();
    let probe = probe?;
    (probe.base_url == provider.base_url
        && probe.model_id == model.id
        && probe.tool_support == model.tool_support)
        .then_some(probe)
}

#[cfg(test)]
/// Capabilities for a saved remote model: persisted controls plus a valid
/// probe. A verified negative probe demotes support to Unsupported, which
/// drops tools everywhere apply_tool_support / supports_tools is consulted.
pub fn capabilities_for(
    store: &crate::store::Store,
    provider: &crate::providers::ProviderConnection,
    model: &crate::providers::RemoteModel,
) -> ModelCapabilities {
    let mut capabilities = ModelCapabilities::from_model(model);
    if let Some(probe) = load_probe(store, provider, model) {
        if probe.verified && !probe.supported {
            capabilities.tool_support = ToolSupport::Unsupported;
        }
        capabilities.probe = Some(probe);
    }
    capabilities
}

impl Default for ToolProbe {
    fn default() -> Self {
        ToolProbe {
            verified: false,
            supported: false,
            detail: String::new(),
            probed_at: String::new(),
            base_url: String::new(),
            model_id: String::new(),
            tool_support: ToolSupport::Unknown,
        }
    }
}

/// The probe function every model is asked to call once.
const PROBE_TOOL: &str = "report_probe";
const PROBE_VALUE: &str = "ok";

fn probe_tools_schema() -> Value {
    json!([{"type":"function","function":{"name":PROBE_TOOL,
        "description":"Report a probe value back to the caller.",
        "parameters":{"type":"object","properties":{"value":{"type":"string","enum":["ok"]}},"required":["value"],"additionalProperties":false}}}])
}

/// One bounded inference round-trip asking the model to call report_probe
/// with value "ok". Uses the same backend/payload/stream path as live turns
/// so OpenAI-compatible, Responses and Claude formats all exercise their
/// real tool plumbing. A supported=true records a parsed call; text-parsed
/// calls count and are flagged in the detail. Failures record verified=false
/// and change nothing else.
pub async fn probe_model_tools(
    state: &crate::AppState,
    provider_id: &str,
    model_id: &str,
) -> Result<ToolProbe, String> {
    // Store guards are not Send; keep every database/vault read synchronous
    // and scoped so nothing non-Send crosses an await.
    let (provider, model, key) = {
        let store = state.database()?;
        let provider = store.provider(provider_id)?.ok_or("Unknown provider.")?;
        let model = provider
            .models
            .iter()
            .find(|model| model.id == model_id)
            .cloned()
            .unwrap_or(crate::providers::RemoteModel {
                controls: None,
                id: model_id.to_string(),
                context_length: Some(8192),
                max_output_tokens: None,
                supports_images: false,
                tool_support: ToolSupport::Unknown,
            });
        let key = crate::providers::resolve_api_key(
            &provider,
            state
                .daytona_vault
                .load(&crate::providers::credential_id(&provider.id))?,
        )
        .unwrap_or_default();
        (provider, model, key)
    };
    let backend = if crate::providers::is_subscription_format(&provider.api_format) {
        let kind = if provider.api_format == crate::providers::CHATGPT_SUBSCRIPTION
            || provider.id == "chatgpt-subscription"
        {
            "chatgpt"
        } else {
            "grok"
        };
        let token =
            crate::subscription_auth::get_valid_access_token(&state.daytona_vault, kind).await?;
        let account_id = crate::subscription_auth::load_session(&state.daytona_vault, kind)?
            .and_then(|s| s.account_id);
        crate::inference::Backend::subscription(
            "subscription",
            &provider.base_url,
            token,
            account_id,
            &model,
        )?
    } else {
        crate::inference::Backend::openai(&provider, key, &model)?
    };
    let request = json!({
        "messages":[{"role":"user","content":"Call the function report_probe exactly once with argument \"value\" set to \"ok\". Do not answer in plain text."}],
        "tools":probe_tools_schema(),"stream":true,
    });
    let result = tokio::time::timeout(std::time::Duration::from_secs(60), async {
        let payload = backend.payload(&request)?;
        let response = backend.stream(&payload).await?;
        response.text().await.map_err(|e| e.to_string())
    })
    .await;
    let detail: String;
    let mut supported = false;
    let parse = |body: String| -> Result<(bool, String), String> {
        let mut decoder = crate::sse::SseDecoder::default();
        let mut protocol = crate::sse::ModelStream::for_backend(&backend);
        let mut calls = crate::tool_calls::ToolCalls::default();
        let mut answer = String::new();
        for event in decoder.push(&body.into_bytes())? {
            let Some(value) = protocol.push(&event)? else {
                if protocol.done {
                    break;
                }
                continue;
            };
            if let Some(error) = value.get("error") {
                return Err(format!("Probe request failed: {error}"));
            }
            let delta = &value["choices"][0]["delta"];
            calls.push(delta)?;
            answer.push_str(delta["content"].as_str().unwrap_or(""));
        }
        let mut calls = calls.finish()?;
        let mut text_parsed = false;
        if calls.is_empty() {
            if let Some(parsed) = crate::tool_calls::parse_text_tool_calls(&answer) {
                calls = parsed;
                text_parsed = true;
            }
        }
        let mut detail = "Model produced no recognized tool call.".to_string();
        let mut supported = false;
        for call in &calls {
            if call.name == PROBE_TOOL {
                let args = call.arguments["value"]
                    .as_str()
                    .unwrap_or_else(|| call.arguments["arguments"]["value"].as_str().unwrap_or(""));
                if args == PROBE_VALUE || call.arguments.to_string().contains("\"ok\"") {
                    supported = true;
                    detail = if text_parsed {
                        "Model emitted report_probe(value=ok) as text-parsed markup.".into()
                    } else {
                        "Model emitted report_probe(value=ok) as a tool call.".into()
                    };
                } else {
                    detail = format!("Model called {PROBE_TOOL} without value=ok.");
                }
            }
        }
        if !supported && detail.starts_with("Model produced no") {
            detail = format!("Model produced {} non-matching call(s).", calls.len());
        }
        Ok((supported, detail))
    };
    match result {
        Ok(Ok(body)) => match parse(body) {
            Ok(pair) => {
                supported = pair.0;
                detail = pair.1;
            }
            Err(error) => {
                detail = error;
            }
        },
        Ok(Err(error)) => {
            detail = error;
        }
        Err(_) => {
            detail = "Probe timed out after 60 seconds.".to_string();
        }
    }
    let probe = ToolProbe {
        verified: supported
            || detail.contains("value=ok")
            || detail.starts_with("Model produced")
            || detail.starts_with("Model emitted"),
        supported,
        detail,
        probed_at: crate::store::now().to_string(),
        base_url: provider.base_url.clone(),
        model_id: model.id.clone(),
        tool_support: model.tool_support.clone(),
    };
    state
        .database()?
        .save_setting(&probe_key(&provider.id, &model.id), &probe)?;
    Ok(probe)
}

#[tauri::command]
pub async fn probe_model_tools_command(
    state: tauri::State<'_, crate::AppState>,
    provider_id: String,
    model_id: String,
) -> Result<ToolProbe, String> {
    probe_model_tools(&state, &provider_id, &model_id).await
}
impl ModelCapabilities {
    pub fn from_model(model: &RemoteModel) -> Self {
        Self {
            tool_support: model.tool_support.clone(),
            controls: model.controls.clone(),
            probe: None,
        }
    }
    pub fn legacy(supported: bool) -> Self {
        Self {
            tool_support: if supported {
                ToolSupport::Supported
            } else {
                ToolSupport::Unsupported
            },
            controls: None,
            probe: None,
        }
    }
    pub fn allows_tools(&self) -> bool {
        self.tool_support != ToolSupport::Unsupported
    }
    pub fn report(&self) -> Value {
        json!({"tools":self.tool_support,"toolsVerified":self.probe.as_ref().map(|p|p.verified).unwrap_or(false),
            "toolsSource":if self.probe.is_some(){"probe"}else{"saved-model-setting"},
            "probedAt":self.probe.as_ref().map(|p|json!(p.probed_at)).unwrap_or(Value::Null),
            "controls":self.controls,
            "controlSource":self.controls.as_ref().map(|c|json!(c.source)).unwrap_or(json!("legacy-family-heuristic")),
            "unknownToolsMayBeAttempted":self.tool_support==ToolSupport::Unknown})
    }
}

/// Apply only explicitly configured controls. None preserves the documented
/// legacy mapping for existing saved connections; Some(unknown) does not infer
/// a model family's optional capabilities.
pub fn apply(payload: &mut Value, controls: &Controls, format: &str) -> Result<(), String> {
    controls.validate_protocol(format)?;
    let object = payload
        .as_object_mut()
        .ok_or("Model request must be an object.")?;
    if controls.temperature == ToolSupport::Unsupported {
        object.remove("temperature");
    }
    if controls.top_p == ToolSupport::Unsupported {
        object.remove("top_p");
    }
    if controls.tool_choice == ToolSupport::Unsupported {
        if let Some(choice) = object.remove("tool_choice") {
            if choice != "auto" {
                return Err("This model is configured without tool-choice support; the requested tool policy cannot be enforced.".into());
            }
        }
    }
    if controls.parallel_tool_calls == ToolSupport::Unsupported
        && object.contains_key("parallel_tool_calls")
    {
        return Err("This model is configured without parallel-tool-call control support.".into());
    }
    let mode = object.remove("locallm_reasoning_mode");
    if controls.reasoning_effort == ToolSupport::Supported {
        if let Some(effort) = mode
            .as_ref()
            .and_then(Value::as_str)
            .and_then(|mode| match mode {
                "quick" => Some("low"),
                "think" => Some("high"),
                _ => None,
            })
        {
            match format {
                crate::providers::OPENAI_RESPONSES => {
                    object
                        .entry("reasoning")
                        .or_insert(json!({"effort":effort}));
                }
                crate::providers::CLAUDE_MESSAGES => {
                    let config = object
                        .entry("output_config")
                        .or_insert(json!({}))
                        .as_object_mut()
                        .ok_or("Claude output_config must be an object.")?;
                    config.entry("effort").or_insert(json!(effort));
                }
                _ => {
                    object.entry("reasoning_effort").or_insert(json!(effort));
                }
            }
        }
    } else if object.contains_key("reasoning")
        || object.contains_key("reasoning_effort")
        || object
            .get("output_config")
            .is_some_and(|v| v.get("effort").is_some())
    {
        return Err(
            "Reasoning effort requires configured supported capability for this model.".into(),
        );
    }
    if object.contains_key("response_format")
        && controls.structured_output != ToolSupport::Supported
    {
        return Err(
            "Structured output requires configured supported capability for this model.".into(),
        );
    }
    let field = match controls.token_limit_field {
        TokenField::ProtocolDefault => None,
        TokenField::MaxTokens => Some("max_tokens"),
        TokenField::MaxCompletionTokens => Some("max_completion_tokens"),
        TokenField::MaxOutputTokens => Some("max_output_tokens"),
    };
    if let Some(field) = field {
        let mut limit = None;
        for key in ["max_tokens", "max_completion_tokens", "max_output_tokens"] {
            if let Some(value) = object.remove(key) {
                if !value
                    .as_u64()
                    .is_some_and(|v| v > 0 && v <= u64::from(u32::MAX))
                {
                    return Err("Invalid configured output-token limit.".into());
                }
                if limit.as_ref().is_some_and(|old| old != &value) {
                    return Err("Conflicting configured output-token limits.".into());
                }
                limit = Some(value);
            }
        }
        if let Some(limit) = limit {
            object.insert(field.into(), limit);
        }
    }
    Ok(())
}

/// Enforce strict-mode structural requirements without changing the user's
/// schema (making optional properties required would change its semantics).
/// Provider-specific keyword restrictions are still reported by the endpoint.
pub fn validate_strict_schema(schema: &Value) -> Result<(), String> {
    if schema["type"] != "object" || schema.get("anyOf").is_some() {
        return Err("Strict provider output requires an object root without a root anyOf. Adjust outputSchema or use post-validation mode.".into());
    }
    fn visit(schema: &Value) -> Result<(), String> {
        if schema["type"] == "object"
            || schema["type"]
                .as_array()
                .is_some_and(|types| types.iter().any(|ty| ty == "object"))
            || schema.get("properties").is_some()
        {
            if schema["additionalProperties"] != false {
                return Err("Strict provider output requires additionalProperties:false on every object; the schema was not modified.".into());
            }
            let properties = schema["properties"].as_object();
            let required = schema["required"].as_array();
            if properties.is_some_and(|props| {
                props.keys().any(|key| {
                    !required.is_some_and(|values| {
                        values
                            .iter()
                            .any(|value| value.as_str() == Some(key.as_str()))
                    })
                })
            }) {
                return Err("Strict provider output requires every property to be required; optional properties were not silently changed.".into());
            }
        }
        for field in ["properties", "$defs", "definitions"] {
            if let Some(map) = schema[field].as_object() {
                for child in map.values() {
                    visit(child)?;
                }
            }
        }
        for field in ["anyOf", "oneOf", "allOf", "prefixItems"] {
            if let Some(items) = schema[field].as_array() {
                for child in items {
                    visit(child)?;
                }
            }
        }
        for field in ["items", "additionalProperties", "not", "if", "then", "else"] {
            if schema[field].is_object() {
                visit(&schema[field])?;
            }
        }
        Ok(())
    }
    visit(schema)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn probe_records_report_and_invalidate_on_setting_change() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        let store = state.database().unwrap();
        let mut provider = crate::providers::ProviderConnection {
            id: "p".into(),
            name: "P".into(),
            api_format: crate::providers::OPENAI_CHAT_COMPLETIONS.into(),
            base_url: "https://a".into(),
            verified: true,
            last_tested_at: None,
            models: vec![],
            has_api_key: true,
        };
        let model = crate::providers::RemoteModel {
            controls: None,
            id: "m".into(),
            context_length: Some(8192),
            max_output_tokens: None,
            supports_images: false,
            tool_support: ToolSupport::Supported,
        };
        assert!(load_probe(&store, &provider, &model).is_none());
        let probe = ToolProbe {
            verified: true,
            supported: true,
            detail: "ok".into(),
            probed_at: "1".into(),
            base_url: provider.base_url.clone(),
            model_id: model.id.clone(),
            tool_support: model.tool_support.clone(),
        };
        store
            .save_setting(&probe_key(&provider.id, &model.id), &probe)
            .unwrap();
        let report = capabilities_for(&store, &provider, &model).report();
        assert_eq!(report["toolsVerified"], json!(true));
        assert_eq!(report["toolsSource"], json!("probe"));
        assert_eq!(report["probedAt"], json!("1"));
        // A changed endpoint invalidates the stored probe.
        provider.base_url = "https://b".into();
        assert!(load_probe(&store, &provider, &model).is_none());
        // A verified-negative probe demotes tool support to Unsupported.
        provider.base_url = "https://a".into();
        let negative = ToolProbe {
            verified: true,
            supported: false,
            ..probe.clone()
        };
        store
            .save_setting(&probe_key(&provider.id, &model.id), &negative)
            .unwrap();
        let capabilities = capabilities_for(&store, &provider, &model);
        assert_eq!(capabilities.tool_support, ToolSupport::Unsupported);
        assert_eq!(capabilities.report()["toolsVerified"], json!(true));
        // An unverified probe changes nothing about advertised support.
        let failed = ToolProbe {
            verified: false,
            supported: false,
            ..probe.clone()
        };
        store
            .save_setting(&probe_key(&provider.id, &model.id), &failed)
            .unwrap();
        assert_eq!(
            capabilities_for(&store, &provider, &model).tool_support,
            ToolSupport::Supported
        );
    }

    #[tokio::test]
    async fn live_probe_detects_a_report_probe_tool_call() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        crate::test_support::select_fixture_model(&state, dir.path());
        let call = json!({"type":"function_call","id":"fc","call_id":"c","name":"report_probe","arguments":"{\"value\":\"ok\"}"});
        let (base,_requests,server) = crate::test_support::scripted_sse(vec![
            format!("data: {}\r\n\r\n", json!({"type":"response.completed","response":{"status":"completed","output":[call],"usage":{"input_tokens":5,"output_tokens":3}}})),
        ]).await;
        {
            let store = state.database().unwrap();
            let mut provider = store.provider("fixture-loopback").unwrap().unwrap();
            provider.api_format = crate::providers::OPENAI_RESPONSES.into();
            provider.base_url = base;
            provider.models = vec![crate::providers::RemoteModel {
                controls: None,
                id: "fixture".into(),
                context_length: Some(8192),
                max_output_tokens: None,
                supports_images: false,
                tool_support: ToolSupport::Supported,
            }];
            store.save_provider(&provider).unwrap();
        }
        let probe = probe_model_tools(&state, "fixture-loopback", "fixture")
            .await
            .unwrap();
        server.await.unwrap();
        assert!(probe.supported, "{probe:?}");
        assert!(probe.verified);
        assert!(probe.detail.contains("report_probe"));
        // Persisted + loaded into capabilities.
        let store = state.database().unwrap();
        let provider = store.provider("fixture-loopback").unwrap().unwrap();
        let model = crate::providers::RemoteModel {
            controls: None,
            id: "fixture".into(),
            context_length: Some(8192),
            max_output_tokens: None,
            supports_images: false,
            tool_support: ToolSupport::Supported,
        };
        let report = capabilities_for(&store, &provider, &model).report();
        assert_eq!(report["toolsSource"], json!("probe"));
        assert_eq!(report["toolsVerified"], json!(true));
    }

    use super::*;
    use crate::inference::{Backend, InferenceProvider};
    use crate::providers::{CLAUDE_MESSAGES, OPENAI_CHAT_COMPLETIONS, OPENAI_RESPONSES};
    fn backend(format: &str, support: ToolSupport, controls: Option<Controls>) -> Backend {
        let provider = crate::providers::ProviderConnection {
            id: "fixture".into(),
            name: "Fixture".into(),
            api_format: format.into(),
            base_url: "https://api.example.test/v1".into(),
            verified: true,
            last_tested_at: None,
            models: vec![],
            has_api_key: true,
        };
        let model = RemoteModel {
            id: "gpt-5".into(),
            context_length: Some(8192),
            max_output_tokens: Some(2048),
            tool_support: support,
            supports_images: false,
            controls,
        };
        Backend::openai(&provider, "fixture-key".into(), &model).unwrap()
    }
    fn supported() -> Controls {
        Controls {
            temperature: ToolSupport::Unsupported,
            top_p: ToolSupport::Unsupported,
            reasoning_effort: ToolSupport::Supported,
            tool_choice: ToolSupport::Supported,
            parallel_tool_calls: ToolSupport::Supported,
            structured_output: ToolSupport::Supported,
            ..Default::default()
        }
    }
    fn schema() -> Value {
        json!({"type":"object","properties":{"result":{"type":"string"}},"required":["result"],"additionalProperties":false})
    }
    #[test]
    fn persisted_controls_preserve_unknown_and_reject_incompatible_fields() {
        let legacy: RemoteModel = serde_json::from_value(
            json!({"id":"custom","contextLength":8192,"maxOutputTokens":null}),
        )
        .unwrap();
        assert!(legacy.controls.is_none());
        assert_eq!(legacy.tool_support, ToolSupport::Unknown);
        assert!(ModelCapabilities::from_model(&legacy).allows_tools());
        assert_eq!(
            ModelCapabilities::from_model(&legacy).report()["tools"],
            "unknown"
        );
        assert_eq!(
            ModelCapabilities::from_model(&legacy).report()["toolsVerified"],
            false
        );
        let configured: Controls =
            serde_json::from_value(json!({"structuredOutput":"supported"})).unwrap();
        assert_eq!(configured.reasoning_effort, ToolSupport::Unknown);
        assert!(serde_json::from_value::<Controls>(json!({"reasoningEffort":true})).is_err());
        assert!(serde_json::from_value::<Controls>(json!({"unknownField":true})).is_err());
        let controls = Controls {
            token_limit_field: TokenField::MaxCompletionTokens,
            ..Default::default()
        };
        assert!(controls.validate_protocol(OPENAI_CHAT_COMPLETIONS).is_ok());
        assert!(controls.validate_protocol(OPENAI_RESPONSES).is_err());
        assert!(controls.validate_protocol(CLAUDE_MESSAGES).is_err());
        let mut reported = Controls {
            source: Source::ProviderReported,
            ..Default::default()
        };
        assert!(reported
            .validate_endpoint(OPENAI_RESPONSES, "https://example.test/v1", "model")
            .is_err());
        reported.bind_endpoint(OPENAI_RESPONSES, "https://example.test/v1", "model");
        assert!(reported
            .validate_endpoint(OPENAI_RESPONSES, "https://example.test/v1/", "model")
            .is_ok());
        assert!(reported
            .validate_endpoint(OPENAI_CHAT_COMPLETIONS, "https://example.test/v1", "model")
            .is_err());
        assert!(reported
            .validate_endpoint(OPENAI_RESPONSES, "https://other.test/v1", "model")
            .is_err());
        assert!(reported
            .validate_endpoint(OPENAI_RESPONSES, "https://example.test/v1", "another-model")
            .is_err());
    }
    #[test]
    fn configured_controls_replace_family_guesses_and_map_each_protocol() {
        let input = json!({"messages":[{"role":"user","content":"Return a result"}],"max_tokens":1000,"temperature":0.4,"top_p":0.8,"locallm_reasoning_mode":"think"});
        for format in [OPENAI_CHAT_COMPLETIONS, OPENAI_RESPONSES, CLAUDE_MESSAGES] {
            let backend = backend(format, ToolSupport::Unknown, Some(supported()));
            assert!(backend.supports_tools());
            assert_eq!(backend.capabilities().tool_support, ToolSupport::Unknown);
            let payload = backend.payload(&input).unwrap();
            assert!(payload.get("temperature").is_none());
            assert!(payload.get("top_p").is_none());
            assert!(payload.get("locallm_reasoning_mode").is_none());
            match format {
                OPENAI_RESPONSES => assert_eq!(payload["reasoning"]["effort"], "high"),
                CLAUDE_MESSAGES => assert_eq!(payload["output_config"]["effort"], "high"),
                _ => assert_eq!(payload["reasoning_effort"], "high"),
            }
        }
        let controls = Controls {
            token_limit_field: TokenField::MaxCompletionTokens,
            ..Default::default()
        };
        let payload = backend(
            OPENAI_CHAT_COMPLETIONS,
            ToolSupport::Supported,
            Some(controls),
        )
        .payload(&input)
        .unwrap();
        assert_eq!(payload["max_completion_tokens"], 1000);
        assert!(payload.get("max_tokens").is_none());
        assert_eq!(
            payload["temperature"], 0.4,
            "unknown sampling is an attempt, not guessed unsupported"
        );
        assert!(
            payload.get("reasoning_effort").is_none(),
            "explicit unknown must suppress family guesses"
        );
    }
    #[test]
    fn constraints_are_native_only_when_supported_and_original_schema_is_unchanged() {
        let schema = schema();
        for format in [OPENAI_CHAT_COMPLETIONS, OPENAI_RESPONSES, CLAUDE_MESSAGES] {
            let backend = backend(format, ToolSupport::Supported, Some(supported()));
            let mut payload =
                json!({"messages":[],"max_tokens":1000,"locallm_reasoning_mode":"quick"});
            assert!(backend.constrain_output(&mut payload, &schema).unwrap());
            let native = backend.payload(&payload).unwrap();
            let actual = match format {
                OPENAI_RESPONSES => &native["text"]["format"]["schema"],
                CLAUDE_MESSAGES => &native["output_config"]["format"]["schema"],
                _ => &native["response_format"]["json_schema"]["schema"],
            };
            assert_eq!(actual, &schema);
        }
        for controls in [
            None,
            Some(Controls::default()),
            Some(Controls {
                structured_output: ToolSupport::Unsupported,
                ..Default::default()
            }),
        ] {
            let backend = backend(OPENAI_RESPONSES, ToolSupport::Supported, controls);
            let mut payload = json!({"messages":[]});
            assert!(!backend.constrain_output(&mut payload, &schema).unwrap());
            assert!(payload.get("response_format").is_none());
        }
        let local = Backend::local("http://127.0.0.1:1".into(), String::new(), 8192).unwrap();
        let mut payload = json!({"messages":[]});
        assert!(local.constrain_output(&mut payload, &schema).unwrap());
        assert_eq!(
            local.payload(&payload).unwrap()["response_format"],
            json!({"type":"json_object","schema":schema})
        );
    }
    #[test]
    fn unsupported_controls_and_incompatible_strict_schema_fail_before_send() {
        let backend = backend(
            OPENAI_CHAT_COMPLETIONS,
            ToolSupport::Supported,
            Some(Controls {
                tool_choice: ToolSupport::Unsupported,
                parallel_tool_calls: ToolSupport::Unsupported,
                structured_output: ToolSupport::Supported,
                ..Default::default()
            }),
        );
        for field in [
            json!({"tool_choice":"none"}),
            json!({"parallel_tool_calls":false}),
            json!({"reasoning_effort":"high"}),
        ] {
            assert!(backend.payload(&field).is_err());
        }
        let mut optional = schema();
        optional["required"] = json!([]);
        let original = optional.clone();
        let mut payload = json!({});
        assert!(backend.constrain_output(&mut payload, &optional).is_err());
        assert_eq!(optional, original);
        assert!(payload.get("response_format").is_none());
        let mut open = schema();
        open["additionalProperties"] = json!(true);
        assert!(backend.constrain_output(&mut payload, &open).is_err());
        let mut nested = schema();
        nested["properties"]["result"] = json!({"type":["object","null"]});
        assert!(backend.constrain_output(&mut payload, &nested).is_err());
        for invalid in [Value::Null, json!("text"), json!([])] {
            assert!(backend.payload(&invalid).is_err());
        }
        let claude =
            super::tests::backend(CLAUDE_MESSAGES, ToolSupport::Supported, Some(supported()));
        assert!(claude
            .payload(&json!({"output_config":42,"locallm_reasoning_mode":"think"}))
            .is_err());
        assert!(claude.payload(&json!({"output_config":42})).is_err());
    }
    #[tokio::test]
    async fn sqlite_parent_and_child_snapshots_preserve_unknown_tools_and_explicit_controls() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        crate::test_support::select_fixture_model(&state, dir.path());
        let conversation = {
            let store = state.database().unwrap();
            let mut provider = store.provider("fixture-loopback").unwrap().unwrap();
            provider.api_format = OPENAI_RESPONSES.into();
            provider.models[0].tool_support = ToolSupport::Unknown;
            provider.models[0].controls = Some(supported());
            store.save_provider(&provider).unwrap();
            store.create_conversation().unwrap().id
        };
        let plan = crate::chat::assemble_turn(
            &state,
            &conversation,
            "Return a result",
            vec![],
            Some(vec![]),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            plan.backend.capabilities().tool_support,
            ToolSupport::Unknown
        );
        assert_eq!(plan.backend.capabilities().controls, Some(supported()));
        assert!(plan
            .injections
            .iter()
            .any(|(kind, value)| kind == "model_capabilities" && value.contains("unknown")));
        let snapshot = crate::subagents::snapshot_for_conversation(&state, &conversation)
            .await
            .unwrap();
        let child = snapshot.backend().unwrap();
        assert_eq!(child.capabilities(), plan.backend.capabilities());
        let store = crate::store::Store::open(&state.db_path).unwrap();
        let provider = store.provider("fixture-loopback").unwrap().unwrap();
        assert_eq!(provider.models[0].controls, Some(supported()));
        assert_eq!(provider.models[0].tool_support, ToolSupport::Unknown);
        let mut payload = json!({"messages":[]});
        assert!(child.constrain_output(&mut payload, &schema()).unwrap());
        assert_eq!(
            child.payload(&payload).unwrap()["text"]["format"]["schema"],
            schema()
        );
    }
    fn child_request(conversation_id: &str, schema: Value) -> crate::subagents::ChildRequest {
        crate::subagents::ChildRequest {
            label: "Constrained fixture".into(),
            prompt: "Return the requested JSON object.".into(),
            depth: 0,
            max_rounds: 1,
            tool_filter: None,
            persona: None,
            output_schema: Some(schema),
            policy: crate::subagents::ChildPolicy::ReadsOnly,
            parent_run_id: "parent-fixture".into(),
            conversation_id: conversation_id.into(),
        }
    }
    #[tokio::test]
    async fn both_child_drivers_reject_incompatible_strict_schema_before_any_network_request() {
        use crate::subagents::{drive_child, drive_detached, ChildCancel};
        use std::sync::{Arc, Mutex};
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        let conversation = state.database().unwrap().create_conversation().unwrap().id;
        let snapshot = crate::test_support::snapshot(&state, dir.path(), &[]);
        let backend = backend(OPENAI_RESPONSES, ToolSupport::Supported, Some(supported()));
        let request = child_request(
            &conversation,
            json!({"type":"object","properties":{"result":{"type":"string"}},"additionalProperties":false}),
        );
        let foreground = drive_child(
            &state,
            &snapshot,
            &request,
            vec![],
            &backend,
            ChildCancel::none(),
            None,
            "child-foreground",
            None,
        )
        .await
        .unwrap_err();
        assert!(
            foreground.contains("every property to be required"),
            "{foreground}"
        );
        let store = Arc::new(Mutex::new(
            crate::store::Store::open(&state.db_path).unwrap(),
        ));
        let background = drive_detached(
            &snapshot,
            &request,
            vec![],
            &backend,
            ChildCancel::none(),
            None,
            "child-background",
            store,
        )
        .await
        .unwrap_err();
        assert!(
            background.contains("every property to be required"),
            "{background}"
        );
    }
    #[tokio::test]
    async fn both_child_drivers_send_native_schema_and_still_reject_invalid_answers() {
        use crate::subagents::{drive_child, drive_detached, ChildCancel};
        use std::sync::{Arc, Mutex};
        for format in [
            "local",
            OPENAI_CHAT_COMPLETIONS,
            OPENAI_RESPONSES,
            CLAUDE_MESSAGES,
        ] {
            for background in [false, true] {
                for valid in [true, false] {
                    let dir = tempfile::tempdir().unwrap();
                    let state = crate::test_support::app_state(dir.path());
                    let conversation = state.database().unwrap().create_conversation().unwrap().id;
                    let snapshot = crate::test_support::snapshot(&state, dir.path(), &[]);
                    let answer = if valid {
                        r#"{"result":"ok"}"#
                    } else {
                        r#"{"wrong":"not schema-valid"}"#
                    };
                    let wire=match format {
                OPENAI_RESPONSES=>format!("data: {}\r\n\r\n",json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"message","id":"msg-fixture","role":"assistant","content":[{"type":"output_text","text":answer,"annotations":[]}]}]}})),
                CLAUDE_MESSAGES=>[json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":answer}}),json!({"type":"content_block_stop","index":0}),json!({"type":"message_delta","delta":{"stop_reason":"end_turn"}}),json!({"type":"message_stop"})].iter().map(|event|format!("data: {event}\r\n\r\n")).collect(),
                _=>format!("data: {}\r\n\r\ndata: [DONE]\r\n\r\n",json!({"choices":[{"delta":{"content":answer},"finish_reason":"stop"}]})),
            };
                    let responses = if format == "local" {
                        vec![json!({"input_tokens":10}).to_string(), wire]
                    } else {
                        vec![wire]
                    };
                    let (base, requests, server) =
                        crate::test_support::scripted_sse(responses).await;
                    let mut backend = if format == "local" {
                        Backend::local(
                            base.trim_end_matches("/v1").into(),
                            "fixture-key".into(),
                            8192,
                        )
                        .unwrap()
                    } else {
                        backend(format, ToolSupport::Supported, Some(supported()))
                    };
                    if let Backend::OpenAi { base_url, .. } | Backend::Claude { base_url, .. } =
                        &mut backend
                    {
                        *base_url = base;
                    }
                    let request = child_request(&conversation, schema());
                    let result = if background {
                        let store = Arc::new(Mutex::new(
                            crate::store::Store::open(&state.db_path).unwrap(),
                        ));
                        drive_detached(
                            &snapshot,
                            &request,
                            vec![],
                            &backend,
                            ChildCancel::none(),
                            None,
                            "child-background",
                            store,
                        )
                        .await
                    } else {
                        drive_child(
                            &state,
                            &snapshot,
                            &request,
                            vec![],
                            &backend,
                            ChildCancel::none(),
                            None,
                            "child-foreground",
                            None,
                        )
                        .await
                    };
                    if valid {
                        assert_eq!(result.unwrap(), answer);
                    } else {
                        assert!(
                            result.is_err(),
                            "provider-side constraints must never replace answer validation"
                        );
                    }
                    server.await.unwrap();
                    let requests = requests.lock().unwrap();
                    assert_eq!(requests.len(), if format == "local" { 2 } else { 1 });
                    if format == "local" {
                        assert!(requests[0]
                            .headers
                            .starts_with("POST /v1/chat/completions/input_tokens "));
                        assert_eq!(requests[0].body["response_format"]["schema"], schema());
                    }
                    let body = &requests.last().unwrap().body;
                    let actual = match format {
                        "local" => &body["response_format"]["schema"],
                        OPENAI_RESPONSES => &body["text"]["format"]["schema"],
                        CLAUDE_MESSAGES => &body["output_config"]["format"]["schema"],
                        _ => &body["response_format"]["json_schema"]["schema"],
                    };
                    assert_eq!(actual, &schema());
                }
            }
        }
    }
}
