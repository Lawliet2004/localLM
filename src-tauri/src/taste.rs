//! Implicit working-style learning from recorded decisions.
//!
//! `memory_teach` / `memory_recall` carry the preferences the user states
//! out loud. This module carries the preferences the user *shows*: every
//! approval decision (accepted, rejected, edited-then-accepted) is a signal
//! about how the user wants work proposed. localLM already produces that
//! signal — the approval modal, the tool audit row and the run events all
//! record it — and then throws it away.
//!
//! The signal is captured once, in the durable `taste_signals` table, and
//! derived into a small, inspectable profile the model can read back with
//! `taste_recall`. Nothing here infers a preference the recorded decisions
//! do not support: every observation is a sentence built from the counts.
//!
//! Boundaries that matter:
//!
//! * Secrets are redacted before anything is written: a key that looks like
//!   a credential (`api_key`, `authorization`, `session`, ...) has its value
//!   replaced with `[redacted]`.
//! * Each stored payload is bounded to 8 KiB and the log keeps at most
//!   [`MAX_SIGNALS`] signals per conversation.
//! * The profile is an observation, not an instruction. The model is told
//!   what the user tends to accept, never told to do anything.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

/// Aliases owned by this module.
pub const ALIASES: [&str; 2] = ["taste_recall", "taste_teach"];

/// Whether an alias belongs to this module.
pub fn owns(alias: &str) -> bool {
    ALIASES.contains(&alias)
}

/// Maximum signals kept per conversation. The oldest are dropped first.
pub const MAX_SIGNALS: usize = 500;

/// Maximum bytes of one stored payload (arguments or result).
pub const MAX_PAYLOAD_BYTES: usize = 8 * 1024;

/// Scope prefix for explicit taste notes.
///
/// The notes live in the same durable bank as `memory_teach` uses, but under
/// their own scope so an explicit memory fact and a taste note never collide.
pub const NOTE_SCOPE_PREFIX: &str = "taste:";

/// Origin recorded for a note taught with `taste_teach`.
pub const NOTE_ORIGIN: &str = "taste";

/// Keys whose values are replaced with `[redacted]` before storage.
const SECRET_KEYS: [&str; 12] = [
    "token",
    "secret",
    "password",
    "passwd",
    "api_key",
    "apikey",
    "authorization",
    "auth",
    "credential",
    "private_key",
    "session",
    "cookie",
];

/// What the user decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Decision {
    /// The proposal was accepted as-is.
    Accepted,
    /// The proposal was rejected.
    Rejected,
    /// The user edited the proposal before accepting it.
    Edited,
}

impl Decision {
    /// The stored spelling of a decision.
    pub fn as_str(&self) -> &'static str {
        match self {
            Decision::Accepted => "accepted",
            Decision::Rejected => "rejected",
            Decision::Edited => "edited",
        }
    }

    /// Parse a stored decision. Unknown spellings are not accepted: a
    /// decision the log does not understand is treated as an acceptance so a
    /// corrupt row can never inflate the rejection counts.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(text: &str) -> Option<Decision> {
        match text {
            "accepted" => Some(Decision::Accepted),
            "rejected" => Some(Decision::Rejected),
            "edited" => Some(Decision::Edited),
            _ => None,
        }
    }
}

/// One observed approval decision.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TasteSignal {
    /// Row id. Storage detail, never shown to the model.
    pub id: String,
    /// The conversation the decision belongs to.
    pub conversation_id: String,
    /// The tool alias that was called.
    pub tool: String,
    /// What the user decided.
    pub decision: Decision,
    /// The arguments the model proposed (bounded, redacted).
    pub arguments: Value,
    /// The result the model produced (bounded, redacted).
    pub result: Value,
    /// When it happened (unix seconds).
    pub created_at: i64,
}

/// What the taste profile says about the user's preferences.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TasteProfile {
    /// Preferred tools, most-used first.
    pub preferred_tools: Vec<(String, u64)>,
    /// Tools the user rejects most often, with their rejection counts.
    pub rejected_tools: Vec<(String, u64)>,
    /// The ratio of edits to acceptances, 0.0..=1.0.
    pub edit_rate: f64,
    /// The number of signals the profile was derived from.
    pub sample_size: u64,
    /// Plain-language observations, e.g. "rejects 3 of 4 shell_run proposals".
    pub observations: Vec<String>,
}

/// One tool's decision counts.
#[derive(Clone, Copy, Debug, Default)]
struct ToolCounts {
    accepted: u64,
    edited: u64,
    rejected: u64,
}

impl ToolCounts {
    fn proposals(&self) -> u64 {
        self.accepted + self.edited + self.rejected
    }
    fn kept(&self) -> u64 {
        self.accepted + self.edited
    }
}

/// Unix time in seconds.
fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Record one approval decision into the taste signal log.
///
/// The arguments and the result are redacted and bounded before they are
/// written, so a credential that was passed to a tool never reaches the
/// store. Recording is best effort: a failure here must never break the
/// approval flow that produced it, so callers should ignore the error.
pub fn record(
    store: &crate::store::Store,
    conversation_id: &str,
    tool: &str,
    decision: Decision,
    arguments: &Value,
    result: &Value,
) -> Result<(), String> {
    let conversation_id = conversation_id.trim();
    if conversation_id.is_empty() || conversation_id.len() > 256 {
        return Err("A taste signal needs a conversation id of 1-256 characters.".into());
    }
    let signal = TasteSignal {
        id: uuid::Uuid::new_v4().to_string(),
        conversation_id: conversation_id.to_string(),
        tool: tool.chars().take(128).collect(),
        decision,
        arguments: bound_payload(&redact(arguments)),
        result: bound_payload(&redact(result)),
        created_at: now_seconds(),
    };
    store.insert_taste_signal(&signal)
}

/// The profile derived from one conversation's signals.
pub fn profile_for(
    store: &crate::store::Store,
    conversation_id: &str,
) -> Result<TasteProfile, String> {
    Ok(profile(&store.taste_signals(conversation_id)?))
}

/// Derive the profile from a signal log.
///
/// Every number in the result comes from the signals; nothing is inferred
/// beyond the counts.
pub fn profile(signals: &[TasteSignal]) -> TasteProfile {
    let mut counts: std::collections::BTreeMap<String, ToolCounts> = Default::default();
    let mut accepted = 0u64;
    let mut edited = 0u64;
    for signal in signals {
        let entry = counts.entry(signal.tool.clone()).or_default();
        match signal.decision {
            Decision::Accepted => {
                entry.accepted += 1;
                accepted += 1;
            }
            Decision::Rejected => entry.rejected += 1,
            Decision::Edited => {
                entry.edited += 1;
                edited += 1;
            }
        }
    }
    let kept = accepted + edited;
    let edit_rate = if kept == 0 {
        0.0
    } else {
        (edited as f64 / kept as f64).clamp(0.0, 1.0)
    };

    let mut preferred: Vec<(String, u64)> = counts
        .iter()
        .filter(|(_, entry)| entry.kept() > 0)
        .map(|(tool, entry)| (tool.clone(), entry.kept()))
        .collect();
    preferred.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));

    let mut rejected: Vec<(String, u64)> = counts
        .iter()
        .filter(|(_, entry)| entry.rejected > 0)
        .map(|(tool, entry)| (tool.clone(), entry.rejected))
        .collect();
    rejected.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));

    // Observations are sentences built from the counts above. A tool with no
    // rejections is reported as accepted; the edit rate is reported only
    // when there was something to keep.
    let mut ranked: Vec<(&String, &ToolCounts)> = counts.iter().collect();
    ranked.sort_by(|left, right| {
        right
            .1
            .proposals()
            .cmp(&left.1.proposals())
            .then_with(|| left.0.cmp(right.0))
    });
    let mut observations: Vec<String> = Vec::new();
    for (tool, entry) in ranked.iter().take(5) {
        let proposals = entry.proposals();
        if entry.rejected > 0 {
            observations.push(format!(
                "rejects {} of {} {} proposals",
                entry.rejected, proposals, tool
            ));
        } else {
            observations.push(format!(
                "accepts {} of {} {} proposals",
                entry.kept(),
                proposals,
                tool
            ));
        }
    }
    if kept > 0 {
        observations.push(format!(
            "edits {} of {} accepted proposals (edit rate {:.2})",
            edited, kept, edit_rate
        ));
    }

    TasteProfile {
        preferred_tools: preferred,
        rejected_tools: rejected,
        edit_rate,
        sample_size: signals.len() as u64,
        observations,
    }
}

/// The scope under which explicit taste notes are stored for one scope.
pub fn note_scope(scope: &str) -> String {
    let trimmed: String = scope.trim().chars().take(900).collect();
    format!("{NOTE_SCOPE_PREFIX}{trimmed}")
}

/// Replace credential-looking values with `[redacted]`.
///
/// The rule is key based: a key that names a credential is redacted at every
/// depth. Compound spellings (`auth_token`, `x-api-key`, `db_password`) are
/// recognized as well, because that is how credentials are actually written.
pub fn redact(value: &Value) -> Value {
    match value {
        Value::Object(fields) => {
            let mut out = Map::new();
            for (key, item) in fields {
                if is_secret_key(key) {
                    out.insert(key.clone(), Value::String("[redacted]".into()));
                } else {
                    out.insert(key.clone(), redact(item));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(redact).collect()),
        other => other.clone(),
    }
}

/// Whether one key names a credential.
fn is_secret_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .map(|character| match character {
            '-' | ' ' => '_',
            other => other.to_ascii_lowercase(),
        })
        .collect();
    if SECRET_KEYS.contains(&normalized.as_str()) {
        return true;
    }
    // Compound spellings: `auth_token`, `db_password`, `x-api-key`.
    normalized
        .split('_')
        .any(|segment| SECRET_KEYS.contains(&segment))
        || SECRET_KEYS
            .iter()
            .filter(|term| term.contains('_'))
            .any(|term| normalized.contains(term))
}

/// Bound one payload to [`MAX_PAYLOAD_BYTES`].
///
/// The shape is kept where possible: strings are clipped and long lists are
/// shortened, so the stored payload still reads as the object it was.
fn bound_payload(value: &Value) -> Value {
    if serialized_len(value) <= MAX_PAYLOAD_BYTES {
        return value.clone();
    }
    let shrunk = shrink(value, MAX_PAYLOAD_BYTES);
    if serialized_len(&shrunk) <= MAX_PAYLOAD_BYTES {
        return shrunk;
    }
    // Last resort: keep the text, clipped, so nothing oversized is stored.
    let text = serde_json::to_string(value).unwrap_or_default();
    Value::String(clip(&text, MAX_PAYLOAD_BYTES))
}

fn serialized_len(value: &Value) -> usize {
    serde_json::to_string(value)
        .map(|text| text.len())
        .unwrap_or(usize::MAX)
}

/// Clip a string to a byte budget without splitting a character.
fn clip(text: &str, limit: usize) -> String {
    text[..floor_boundary(text, limit)].to_string()
}

/// Shrink a value until it fits the budget.
///
/// The budget is measured in serialized bytes, so an escaped string never
/// pushes the result past the limit.
fn shrink(value: &Value, budget: usize) -> Value {
    match value {
        Value::Object(fields) => {
            let mut out = Map::new();
            let mut used = 2usize;
            for (key, item) in fields {
                let remaining = budget.saturating_sub(used + key.len() + 4);
                let shrunk = shrink(item, remaining);
                let cost = key.len() + 4 + serialized_len(&shrunk);
                if used + cost > budget && !out.is_empty() {
                    break;
                }
                out.insert(key.clone(), shrunk);
                used += cost;
                if used > budget {
                    break;
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => {
            let mut out = Vec::new();
            let mut used = 2usize;
            for item in items {
                let remaining = budget.saturating_sub(used + 1);
                let shrunk = shrink(item, remaining);
                let cost = 1 + serialized_len(&shrunk);
                if used + cost > budget && !out.is_empty() {
                    break;
                }
                out.push(shrunk);
                used += cost;
                if used > budget {
                    break;
                }
            }
            Value::Array(out)
        }
        Value::String(text) => shrink_string(text, budget),
        other => other.clone(),
    }
}

/// Clip a string so its serialized form fits the budget.
fn shrink_string(text: &str, budget: usize) -> Value {
    let mut end = text.len().min(budget.saturating_sub(2));
    loop {
        end = floor_boundary(text, end);
        let candidate = Value::String(text[..end].to_string());
        if serialized_len(&candidate) <= budget || end == 0 {
            return candidate;
        }
        end /= 2;
    }
}

/// The largest boundary at or below `end`.
fn floor_boundary(text: &str, end: usize) -> usize {
    let mut end = end.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}

/// One tool and how often it was kept.
#[derive(Clone, Debug, Serialize)]
struct ToolCount {
    tool: String,
    count: u64,
}

/// The profile as the model sees it.
///
/// The field names are the ones the profile is documented with, so the
/// model-facing payload and the Rust struct agree.
#[derive(Clone, Debug, Serialize)]
struct ProfileView {
    preferred_tools: Vec<ToolCount>,
    rejected_tools: Vec<ToolCount>,
    edit_rate: f64,
    sample_size: u64,
    observations: Vec<String>,
}

impl From<&TasteProfile> for ProfileView {
    fn from(profile: &TasteProfile) -> Self {
        Self {
            preferred_tools: profile
                .preferred_tools
                .iter()
                .map(|(tool, count)| ToolCount {
                    tool: tool.clone(),
                    count: *count,
                })
                .collect(),
            rejected_tools: profile
                .rejected_tools
                .iter()
                .map(|(tool, count)| ToolCount {
                    tool: tool.clone(),
                    count: *count,
                })
                .collect(),
            edit_rate: profile.edit_rate,
            sample_size: profile.sample_size,
            observations: profile.observations.clone(),
        }
    }
}

/// Model-facing tool definitions.
pub fn registry() -> Vec<crate::tool_spec::ToolSpec> {
    vec![
        crate::tool_spec::ToolSpec::new(
            "taste_recall",
            "Recall the learned working preferences for this workspace: which tools are accepted, which are rejected, and how often proposals are edited. Use it to shape how you propose work. The profile is derived from recorded decisions and may be empty.",
            json!({
                "type":"object",
                "properties":{"scope":{"type":"string","minLength":1,"maxLength":256,
                    "description":"Which taste scope to recall. Defaults to the current conversation."}},
                "required":[],
                "additionalProperties":false
            }),
        )
        .with_output(json!({
            "type":"object",
            "required":["profile","signals"],
            "properties":{
                "profile":{"type":"object"},
                "signals":{"type":"integer","minimum":0},
                "notes":{"type":"array","items":{"type":"string"}}
            },
            "additionalProperties":false
        })),
        crate::tool_spec::ToolSpec::new(
            "taste_teach",
            "Record an explicit working preference for this workspace, in the user's own words. Prefer taste_recall for what has already been learned; use this only for something the user states outright.",
            json!({
                "type":"object",
                "properties":{"note":{"type":"string","minLength":1,"maxLength":2000,
                    "description":"The preference, in the user's own words."}},
                "required":["note"],
                "additionalProperties":false
            }),
        )
        .with_output(json!({
            "type":"object",
            "required":["recorded","note"],
            "properties":{
                "recorded":{"type":"boolean"},
                "note":{"type":"string"}
            },
            "additionalProperties":false
        })),
    ]
}

/// Execute a taste tool.
pub async fn execute(
    ctx: &crate::harness::HarnessCtx<'_>,
    alias: &str,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    match alias {
        "taste_recall" => recall(ctx, args),
        "taste_teach" => teach(ctx, args),
        other => Err(format!("Unknown taste tool '{other}'.")),
    }
}

/// The scope a recall reads: the caller's scope, or the conversation.
fn scope_of(args: &Value, conversation_id: &str) -> Result<String, String> {
    match args.get("scope") {
        Some(value) => {
            let text = value
                .as_str()
                .ok_or("taste_recall 'scope' must be a string.")?;
            if text.trim().is_empty() || text.len() > 256 {
                return Err("taste_recall 'scope' must be 1-256 characters.".into());
            }
            Ok(text.trim().to_string())
        }
        None => Ok(conversation_id.to_string()),
    }
}

/// Recall the learned preferences.
fn recall(
    ctx: &crate::harness::HarnessCtx<'_>,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let scope = scope_of(&args, &ctx.conversation_id)?;
    let store = ctx.state.database()?;
    let signals = store.taste_signals(&scope)?;
    let profile = profile(&signals);
    let notes: Vec<String> = store
        .recall_facts(&note_scope(&scope), 50)?
        .into_iter()
        .filter(|fact| fact.origin == NOTE_ORIGIN)
        .map(|fact| fact.fact)
        .collect();
    Ok(json!({
        "profile": ProfileView::from(&profile),
        "signals": signals.len(),
        "notes": notes,
    }))
}

/// Record one explicit preference.
fn teach(
    ctx: &crate::harness::HarnessCtx<'_>,
    args: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let note = args
        .get("note")
        .and_then(|value| value.as_str())
        .ok_or("taste_teach needs a 'note' string.")?;
    if note.trim().is_empty() || note.len() > 2000 {
        return Err("taste_teach 'note' must be 1-2000 characters.".into());
    }
    let store = ctx.state.database()?;
    store.teach_fact(&note_scope(&ctx.conversation_id), note, NOTE_ORIGIN)?;
    Ok(json!({"recorded": true, "note": note}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store() -> (tempfile::TempDir, crate::store::Store) {
        let directory = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(&directory.path().join("taste.sqlite")).unwrap();
        (directory, store)
    }

    fn context<'a>(
        state: &'a crate::AppState,
        workspace: &std::path::Path,
    ) -> crate::harness::HarnessCtx<'a> {
        crate::harness::HarnessCtx {
            state,
            snapshot: crate::test_support::snapshot(
                state,
                workspace,
                &["taste_recall", "taste_teach"],
            ),
            conversation_id: "taste-fixture".into(),
            run_id: "taste-run".into(),
            step_id: "step0".into(),
            access_mode: crate::permissions::AccessMode::FullAccess,
            depth: 0,
            preset_id: crate::presets::STANDARD.into(),
            inherit_tools: &[],
            emit: std::sync::Arc::new(|_, _| {}),
        }
    }

    #[test]
    fn recording_a_signal_stores_it_and_it_appears_in_the_profile() {
        let (_directory, store) = store();
        let conversation = store.create_conversation().unwrap();
        record(
            &store,
            &conversation.id,
            "file_write",
            Decision::Accepted,
            &json!({"path":"main.rs"}),
            &json!({"ok":true}),
        )
        .unwrap();
        let signals = store.taste_signals(&conversation.id).unwrap();
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].tool, "file_write");
        assert_eq!(signals[0].decision, Decision::Accepted);
        let profile = profile_for(&store, &conversation.id).unwrap();
        assert_eq!(profile.sample_size, 1);
        assert_eq!(profile.preferred_tools, vec![("file_write".to_string(), 1)]);
        assert!(profile.rejected_tools.is_empty());
        assert_eq!(profile.edit_rate, 0.0);
        assert!(!profile.observations.is_empty());
    }

    #[test]
    fn secrets_are_redacted_before_storage() {
        let (_directory, store) = store();
        let conversation = store.create_conversation().unwrap();
        record(
            &store,
            &conversation.id,
            "shell_run",
            Decision::Accepted,
            &json!({"api_key":"sk-secret-value","command":"ls"}),
            &json!({"authorization":"Bearer abc","nested":{"session":"xyz"}}),
        )
        .unwrap();
        let signals = store.taste_signals(&conversation.id).unwrap();
        assert_eq!(signals[0].arguments["api_key"], json!("[redacted]"));
        assert_eq!(signals[0].arguments["command"], json!("ls"));
        assert_eq!(signals[0].result["authorization"], json!("[redacted]"));
        assert_eq!(signals[0].result["nested"]["session"], json!("[redacted]"));
        let text = serde_json::to_string(&signals[0]).unwrap();
        assert!(!text.contains("sk-secret-value"), "{text}");
        assert!(!text.contains("Bearer abc"), "{text}");
    }

    #[test]
    fn compound_secret_keys_are_redacted() {
        let redacted = redact(&json!({
            "auth_token":"a",
            "x-api-key":"b",
            "db_password":"c",
            "harmless":"d"
        }));
        assert_eq!(redacted["auth_token"], json!("[redacted]"));
        assert_eq!(redacted["x-api-key"], json!("[redacted]"));
        assert_eq!(redacted["db_password"], json!("[redacted]"));
        assert_eq!(redacted["harmless"], json!("d"));
    }

    #[test]
    fn oversized_payloads_are_truncated() {
        let (_directory, store) = store();
        let conversation = store.create_conversation().unwrap();
        let huge = "x".repeat(64 * 1024);
        record(
            &store,
            &conversation.id,
            "file_read",
            Decision::Accepted,
            &json!({"path": huge}),
            &json!({"content": huge}),
        )
        .unwrap();
        let signals = store.taste_signals(&conversation.id).unwrap();
        assert!(signals[0].arguments["path"].as_str().unwrap().len() <= MAX_PAYLOAD_BYTES);
        assert!(signals[0].result["content"].as_str().unwrap().len() <= MAX_PAYLOAD_BYTES);
        assert!(serialized_len(&signals[0].arguments) <= MAX_PAYLOAD_BYTES);
        assert!(serialized_len(&signals[0].result) <= MAX_PAYLOAD_BYTES);
    }

    #[test]
    fn the_signal_log_is_capped_and_drops_the_oldest() {
        let (_directory, store) = store();
        let conversation = store.create_conversation().unwrap();
        for index in 0..(MAX_SIGNALS + 25) {
            record(
                &store,
                &conversation.id,
                "shell_run",
                Decision::Accepted,
                &json!({"index": index}),
                &json!({}),
            )
            .unwrap();
        }
        let signals = store.taste_signals(&conversation.id).unwrap();
        assert_eq!(signals.len(), MAX_SIGNALS);
        // The oldest entries are the ones that fell out.
        assert_eq!(signals[0].arguments["index"], json!(25));
        assert_eq!(
            signals.last().unwrap().arguments["index"],
            json!(MAX_SIGNALS + 24)
        );
    }

    #[test]
    fn the_profile_counts_per_tool_and_computes_the_edit_rate() {
        let edited = TasteSignal {
            id: "1".into(),
            conversation_id: "c".into(),
            tool: "file_write".into(),
            decision: Decision::Edited,
            arguments: json!({}),
            result: json!({}),
            created_at: 1,
        };
        let accepted = TasteSignal {
            id: "2".into(),
            conversation_id: "c".into(),
            tool: "file_read".into(),
            decision: Decision::Accepted,
            arguments: json!({}),
            result: json!({}),
            created_at: 2,
        };
        let rejected = TasteSignal {
            id: "3".into(),
            conversation_id: "c".into(),
            tool: "shell_run".into(),
            decision: Decision::Rejected,
            arguments: json!({}),
            result: json!({}),
            created_at: 3,
        };

        let one_edit = profile(std::slice::from_ref(&edited));
        assert_eq!(one_edit.edit_rate, 1.0);
        assert_eq!(one_edit.sample_size, 1);
        assert_eq!(
            one_edit.preferred_tools,
            vec![("file_write".to_string(), 1)]
        );

        let one_accepted = profile(std::slice::from_ref(&accepted));
        assert_eq!(one_accepted.edit_rate, 0.0);
        assert_eq!(
            one_accepted.preferred_tools,
            vec![("file_read".to_string(), 1)]
        );

        let one_rejected = profile(std::slice::from_ref(&rejected));
        assert_eq!(one_rejected.edit_rate, 0.0);
        assert_eq!(
            one_rejected.rejected_tools,
            vec![("shell_run".to_string(), 1)]
        );
        assert!(one_rejected.preferred_tools.is_empty());

        let both = profile(&[accepted.clone(), rejected.clone()]);
        assert_eq!(both.edit_rate, 0.0);
        assert_eq!(both.sample_size, 2);
        assert!(both
            .observations
            .iter()
            .any(|line| line.contains("shell_run") && line.starts_with("rejects")));
    }

    #[test]
    fn observations_are_empty_without_signals_and_present_with_them() {
        let empty = profile(&[]);
        assert!(empty.observations.is_empty());
        assert_eq!(empty.sample_size, 0);
        assert_eq!(empty.edit_rate, 0.0);

        let signal = TasteSignal {
            id: "1".into(),
            conversation_id: "c".into(),
            tool: "shell_run".into(),
            decision: Decision::Rejected,
            arguments: json!({}),
            result: json!({}),
            created_at: 1,
        };
        let filled = profile(&[signal]);
        assert!(!filled.observations.is_empty());
        assert!(filled.observations[0].contains("shell_run"));
    }

    #[tokio::test]
    async fn taste_recall_on_a_fresh_conversation_returns_an_empty_profile() {
        let directory = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(directory.path());
        let ctx = context(&state, directory.path());
        let recalled = execute(&ctx, "taste_recall", json!({})).await.unwrap();
        assert_eq!(recalled["signals"], json!(0));
        assert_eq!(recalled["profile"]["sample_size"], json!(0));
        assert_eq!(recalled["profile"]["edit_rate"], json!(0.0));
        assert_eq!(recalled["profile"]["preferred_tools"], json!([]));
        assert_eq!(recalled["profile"]["observations"], json!([]));
        assert!(recalled["notes"].as_array().unwrap().is_empty());
        // The declared result contract holds.
        let contract = registry()
            .into_iter()
            .find(|spec| spec.name == "taste_recall")
            .unwrap()
            .output
            .unwrap();
        crate::tool_validation::validate_value(&contract, &recalled).unwrap();
    }

    #[tokio::test]
    async fn taste_teach_stores_a_note_and_taste_recall_surfaces_it() {
        let directory = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(directory.path());
        let ctx = context(&state, directory.path());
        let taught = execute(&ctx, "taste_teach", json!({"note":"prefer pnpm over npm"}))
            .await
            .unwrap();
        assert_eq!(taught["recorded"], json!(true));
        assert_eq!(taught["note"], json!("prefer pnpm over npm"));
        let contract = registry()
            .into_iter()
            .find(|spec| spec.name == "taste_teach")
            .unwrap()
            .output
            .unwrap();
        crate::tool_validation::validate_value(&contract, &taught).unwrap();

        let recalled = execute(&ctx, "taste_recall", json!({})).await.unwrap();
        let notes = recalled["notes"].as_array().unwrap();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0], json!("prefer pnpm over npm"));

        // The note lives in the memory bank under its own scope, so an
        // explicit memory fact is never mistaken for a taste note.
        let store = state.database().unwrap();
        let facts = store.recall_facts("global", 50).unwrap();
        assert!(facts.iter().all(|fact| fact.origin != NOTE_ORIGIN));
    }

    #[tokio::test]
    async fn taste_recall_rejects_an_oversized_scope() {
        let directory = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(directory.path());
        let ctx = context(&state, directory.path());
        assert!(
            execute(&ctx, "taste_recall", json!({"scope":"x".repeat(257)}))
                .await
                .is_err()
        );
        assert!(execute(&ctx, "taste_teach", json!({"note":""}))
            .await
            .is_err());
        assert!(execute(&ctx, "taste_teach", json!({})).await.is_err());
    }

    #[test]
    fn the_registry_declares_both_tools_with_their_contracts() {
        let specs = registry();
        assert_eq!(specs.len(), 2);
        let recall = specs
            .iter()
            .find(|spec| spec.name == "taste_recall")
            .unwrap();
        assert!(recall.has_output_contract());
        let teach = specs
            .iter()
            .find(|spec| spec.name == "taste_teach")
            .unwrap();
        assert!(teach.has_output_contract());
        assert!(owns("taste_recall") && owns("taste_teach"));
        assert!(!owns("memory_recall"));
        // A contract that does not compile would fail the tool at runtime.
        for spec in &specs {
            let contract = spec.output.clone().unwrap();
            crate::tool_validation::compile(&contract)
                .unwrap_or_else(|error| panic!("{}: {error}", spec.name));
        }
    }
}
