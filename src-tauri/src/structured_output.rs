//! Grammar-constrained final answers (docs/EXTENSIONS.md §1.1).
//!
//! Tool calls are already constrained by the runtime: `llama-server --jinja`
//! compiles the request's `tools` schemas into a lazy grammar for templates
//! it supports. The harness therefore never sends its own `grammar` or
//! `response_format` alongside `tools`, which would duplicate or conflict
//! with that grammar.
//!
//! What the runtime cannot know is a *final-answer* schema such as a
//! subagent's `output_schema`. For that, a tool-free repair request carries
//! `response_format: json_schema`, which llama-server turns into a grammar,
//! so a local model cannot emit off-schema JSON. Remote providers get a plain
//! retry instead, and every repair records which path ran.

use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decoding {
    /// `response_format: json_schema` compiled to a grammar by llama-server.
    Grammar,
    /// Instruction-only retry; the answer is validated after the fact.
    Unconstrained,
}

impl Decoding {
    /// Only the managed local runtime is known to honour `json_schema`.
    /// OpenAI-compatible servers vary, and the Claude and subscription APIs
    /// have no equivalent, so they are not sent a field they may reject.
    pub fn for_backend(backend: &crate::inference::Backend) -> Self {
        match backend {
            crate::inference::Backend::Local { .. } => Self::Grammar,
            _ => Self::Unconstrained,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Grammar => "grammar",
            Self::Unconstrained => "unconstrained",
        }
    }
}

/// The user turn that asks for the schema-conforming answer. It is
/// model-visible, so callers must log it with the repair event.
pub fn repair_prompt(schema: &Value, problem: &str) -> String {
    format!(
        "Your previous reply could not be used: {problem} Reply again with ONLY the final answer as a JSON object matching this schema, with no prose and no code fences: {schema}"
    )
}

/// Internal (OpenAI-style) payload for the repair round. It never carries
/// tools, so a grammar here cannot collide with the runtime's tool grammar.
pub fn repair_payload(
    messages: &[Value],
    preferences: &crate::store::Preferences,
    schema: &Value,
    decoding: Decoding,
    id_slot: Option<i64>,
) -> Value {
    let mut payload = json!({
        "messages": messages,
        "temperature": preferences.temperature,
        "top_p": preferences.top_p,
        "max_tokens": preferences.max_tokens,
        "stream": true,
        "cache_prompt": true,
    });
    preferences.sampling.apply(&mut payload);
    if let Some(slot) = id_slot {
        payload["id_slot"] = json!(slot);
    }
    if decoding == Decoding::Grammar {
        payload["response_format"] = json!({
            "type": "json_schema",
            "json_schema": {"name": "final_answer", "schema": schema},
        });
    }
    payload
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grammar_payload_carries_schema_and_never_tools() {
        let schema = json!({"type": "object", "required": ["verdict"], "properties": {"verdict": {"type": "string"}}});
        let messages = vec![json!({"role": "user", "content": "go"})];
        let mut preferences = crate::store::Preferences::default();
        preferences.sampling.seed = Some(3);
        let payload = repair_payload(&messages, &preferences, &schema, Decoding::Grammar, Some(1));
        assert_eq!(payload["response_format"]["type"], "json_schema");
        assert_eq!(payload["response_format"]["json_schema"]["schema"], schema);
        assert_eq!(payload["seed"], 3);
        assert_eq!(payload["id_slot"], 1);
        assert!(payload.get("tools").is_none());
        assert!(payload.get("grammar").is_none());
    }

    #[test]
    fn unconstrained_payload_sends_no_format() {
        let payload = repair_payload(&[], &crate::store::Preferences::default(), &json!({}), Decoding::Unconstrained, None);
        assert!(payload.get("response_format").is_none());
        assert!(payload.get("id_slot").is_none());
    }

    #[test]
    fn only_the_local_runtime_gets_grammar_decoding() {
        let local = crate::inference::Backend::local("http://127.0.0.1:1".into(), "k".into(), 4096).unwrap();
        assert_eq!(Decoding::for_backend(&local), Decoding::Grammar);
        let model = crate::providers::RemoteModel { id: "m".into(), context_length: Some(4096), max_output_tokens: None, supports_images: false, tool_support: crate::providers::ToolSupport::Unknown };
        let remote = crate::inference::Backend::subscription("ChatGPT", "https://chatgpt.com/backend-api/codex", "t".into(), None, &model).unwrap();
        assert_eq!(Decoding::for_backend(&remote), Decoding::Unconstrained);
    }

    #[test]
    fn repair_prompt_names_the_problem_and_schema() {
        let prompt = repair_prompt(&json!({"required": ["a"]}), "Subagent output is missing required key 'a'.");
        assert!(prompt.contains("missing required key 'a'"));
        assert!(prompt.contains(r#"{"required":["a"]}"#));
    }
}
