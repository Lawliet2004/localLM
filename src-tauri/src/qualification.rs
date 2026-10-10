//! Model qualification: a bounded, deterministic suite that proves a loaded
//! model actually does what a profile promises before it is recommended. A
//! model that loads is not a model that works — `readiness()` stays honest
//! about which step is missing, and stored results go stale when the identity
//! (model + runtime config) changes.
//!
//! Everything runs through the same machinery as real turns: the inference
//! lease, the shared cancel channel, the `SseDecoder`/`ToolCalls` parsers,
//! and the local-only URL gate. Nothing here invents a second inference path.

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::watch;

use crate::AppState;
use crate::inference::{Backend, InferenceProvider};
use crate::providers::ModelSelection;
use crate::tool_calls::ToolCalls;

const CASE_TIMEOUT: Duration = Duration::from_secs(60);
const SUITE_TIMEOUT: Duration = Duration::from_secs(300);
const CANCEL_GRACE: Duration = Duration::from_secs(5);

/// Core cases that must all pass for an overall 'passed' — a model that
/// loads and chats but cannot call tools is not qualified for the agent
/// profiles.
const CORE_CASES: &[&str] = &[
    "basic_chat",
    "tool_call",
    "tool_args_parsing",
    "structured_output",
];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseResult {
    pub name: String,
    /// 'passed' | 'failed' | 'unsupported' | 'untested'
    pub status: String,
    pub latency_ms: u64,
    pub tokens_per_second: Option<f64>,
    /// Bounded error excerpt — never a claim of a reason the log did not show.
    pub error: Option<String>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Qualification {
    pub id: String,
    pub identity_hash: String,
    /// True at read time when the stored identity no longer matches the
    /// current model/config — a changed context length is a different model
    /// for capability purposes.
    pub stale: bool,
    pub model_id: String,
    pub model_path: Option<String>,
    pub provider_id: Option<String>,
    pub runtime_version: Option<String>,
    pub config: Value,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub cases: Vec<CaseResult>,
    /// 'passed' | 'partial' | 'failed' | 'cancelled'
    pub status: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum QualificationEvent {
    #[serde(rename = "case")]
    Case {
        name: String,
        status: String,
        result: Option<CaseResult>,
    },
    #[serde(rename = "complete")]
    Complete { record: Qualification },
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Readiness {
    pub step: String,
    pub message: String,
    pub next_action: Option<ReadinessAction>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessAction {
    pub label: String,
    /// Settings section to open ('models', 'providers', …) or empty when the
    /// action is already on screen.
    pub target: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub label: String,
    pub preset: String,
    pub interaction_mode: String,
    pub recommended: bool,
    /// Set when qualification says this profile would mislead — e.g. tool
    /// calls failed qualification — so the label never hides a gap.
    pub note: Option<String>,
}

// ---------------------------------------------------------------------------
// Backend + identity

struct SuiteTarget {
    backend: Backend,
    model_id: String,
    model_path: Option<String>,
    provider_id: Option<String>,
    runtime_version: Option<String>,
    config: Value,
}

impl SuiteTarget {
    fn identity_hash(&self, fingerprint: &str) -> String {
        let config = serde_json::to_string(&self.config).unwrap_or_default();
        let material = format!(
            "{}|{}|{}|{}|{}|{}",
            self.model_id,
            self.model_path.as_deref().unwrap_or(""),
            fingerprint,
            self.runtime_version.as_deref().unwrap_or(""),
            self.provider_id.as_deref().unwrap_or(""),
            config
        );
        crate::task_workspace::sha256_bytes(material.as_bytes())
    }
}

/// sha or (size+mtime) identity for a local GGUF — the file changes under a
/// loaded model are different enough to re-qualify.
fn file_fingerprint(path: Option<&str>) -> String {
    let Some(path) = path else {
        return String::new();
    };
    match std::fs::metadata(path) {
        Ok(meta) => format!("{}:{:?}", meta.len(), meta.modified().ok()),
        Err(_) => "missing".into(),
    }
}

/// Build the qualification target for a selection, on the same
/// local-only/loopback path real turns use.
async fn target_for(state: &AppState, selection: &ModelSelection) -> Result<SuiteTarget, String> {
    if let Some(provider_id) = &selection.provider_id {
        let (provider, model) = {
            let store = state.database()?;
            let provider = store
                .provider(provider_id)?
                .ok_or("The selected provider was deleted.")?;
            let model = provider
                .models
                .iter()
                .find(|model| model.id == selection.model_id)
                .ok_or("That model is not listed for this provider.")?
                .clone();
            (provider, model)
        };
        // The qualification suite obeys the local-only boundary like any
        // inference call — a non-loopback endpoint is rejected before any
        // request is sent.
        crate::local_only::inference_allowed(&provider.base_url)?;
        if crate::providers::is_subscription_format(&provider.api_format)
            || crate::providers::is_claude_format(&provider.api_format)
        {
            return Err("Qualification supports OpenAI-compatible backends; this provider uses a different wire format.".into());
        }
        let key = state
            .daytona_vault
            .load(&crate::providers::credential_id(provider_id))?
            .or_else(|| crate::providers::is_loopback_base_url(&provider.base_url).then(Vec::new))
            .ok_or("Save an API key for the selected provider.")?;
        let key = String::from_utf8(key).map_err(|_| "Saved provider API key is invalid.")?;
        let backend = Backend::openai(&provider, key, &model)?;
        return Ok(SuiteTarget {
            backend,
            model_id: model.id.clone(),
            model_path: None,
            provider_id: Some(provider.id.clone()),
            runtime_version: Some(format!("{} {}", provider.name, provider.api_format)),
            config: json!({
                "contextLength": model.context_length,
                "maxOutputTokens": model.max_output_tokens,
                "temperature": 0,
            }),
        });
    }
    let mut runtime = state.runtime.lock().await;
    let status = runtime.inspect();
    if status.phase != "ready" {
        return Err(format!(
            "No local model is ready to qualify (runtime is {}).",
            status.phase
        ));
    }
    let backend = Backend::local(
        runtime.endpoint.clone(),
        runtime.api_key.clone(),
        runtime.context_length,
    )?;
    let model_path = status.model_path.clone();
    let config = status
        .loaded_config
        .map(|config| serde_json::to_value(config).unwrap_or_default())
        .unwrap_or_default();
    Ok(SuiteTarget {
        backend,
        model_id: model_path
            .as_deref()
            .and_then(|path| std::path::Path::new(path).file_name())
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| "local".into()),
        model_path,
        provider_id: None,
        runtime_version: status
            .launch_config
            .and_then(|cfg| serde_json::to_string(&cfg).ok()),
        config: json!({ "config": config, "temperature": 0 }),
    })
}

// ---------------------------------------------------------------------------
// One bounded completion on the real streaming path

struct Completion {
    text: String,
    calls: Vec<crate::tool_calls::ToolCall>,
    /// Parser error when streamed tool deltas were malformed — a case
    /// reports this instead of a bare "no call".
    calls_error: Option<String>,
    chunks: u32,
    tokens_per_second: Option<f64>,
}

async fn complete(
    backend: &Backend,
    messages: &[Value],
    tools: &[Value],
    cancellation: &mut watch::Receiver<bool>,
    deadline: Duration,
) -> Result<Completion, String> {
    let mut payload = json!({
        "messages": messages,
        "stream": true,
        "stream_options": {"include_usage": true},
        "temperature": 0,
        "max_tokens": 256,
    });
    if !tools.is_empty() {
        payload["tools"] = json!(tools);
        payload["tool_choice"] = json!("auto");
    }
    let payload = backend.payload(&payload);
    let started = Instant::now();
    let response = tokio::select! {
        _ = cancellation.changed() => return Err("cancelled".into()),
        result = tokio::time::timeout(deadline, backend.stream(&payload)) => {
            result.map_err(|_| "Timed out waiting for the model to respond.".to_string())?
                .map_err(|error| error.to_string())?
        }
    };
    let mut stream = response.bytes_stream();
    let mut decoder = crate::sse::SseDecoder::default();
    let mut text = String::new();
    let mut calls = ToolCalls::default();
    let mut chunks = 0u32;
    let mut output_tokens: Option<u64> = None;
    let mut timed_out = false;
    loop {
        let next = tokio::select! {
            _ = cancellation.changed() => return Err("cancelled".into()),
            next = tokio::time::timeout(deadline, stream.next()) => next,
        };
        let bytes = match next {
            Ok(Some(bytes)) => bytes,
            Ok(None) => break,
            Err(_) => {
                timed_out = true;
                break;
            }
        };
        let bytes = bytes.map_err(|error| error.to_string())?;
        for event in decoder.push(&bytes)? {
            if event == "[DONE]" {
                continue;
            }
            let Ok(value) = serde_json::from_str::<Value>(&event) else {
                continue;
            };
            if let Some(usage) = value["usage"]["completion_tokens"].as_u64() {
                output_tokens = Some(usage);
            }
            let delta = &value["choices"][0]["delta"];
            calls.push(delta)?;
            chunks += 1;
            if let Some(content) = delta["content"].as_str() {
                text.push_str(content);
            }
        }
    }
    if timed_out {
        return Err(format!(
            "The model stopped responding within {}s.",
            deadline.as_secs()
        ));
    }
    let latency_ms = started.elapsed().as_millis() as u64;
    let (mut parsed, calls_error) = match calls.finish() {
        Ok(calls) => (calls, None),
        Err(error) => (Vec::new(), Some(error)),
    };
    // Some local models emit text-embedded calls instead of using the
    // tools parameter — parse through the same fallback the turn loop uses.
    if parsed.is_empty() {
        if let Some(text_calls) = crate::tool_calls::parse_text_tool_calls(&text) {
            parsed = text_calls;
        }
    }
    let tokens_per_second = output_tokens
        .filter(|_| latency_ms > 0)
        .map(|tokens| tokens as f64 * 1000.0 / latency_ms as f64);
    Ok(Completion {
        text,
        calls: parsed,
        calls_error,
        chunks,
        tokens_per_second,
    })
}

// ---------------------------------------------------------------------------
// The suite

fn result(
    name: &str,
    status: &str,
    started: Instant,
    detail: Option<String>,
    error: Option<String>,
) -> CaseResult {
    CaseResult {
        name: name.into(),
        status: status.into(),
        latency_ms: started.elapsed().as_millis() as u64,
        tokens_per_second: None,
        error,
        detail,
    }
}

fn add_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "add",
            "description": "Add two integers.",
            "parameters": {
                "type": "object",
                "properties": {
                    "a": {"type": "integer"},
                    "b": {"type": "integer"}
                },
                "required": ["a", "b"],
                "additionalProperties": false
            }
        }
    })
}

fn schedule_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "schedule_reminder",
            "description": "Schedule a reminder.",
            "parameters": {
                "type": "object",
                "properties": {
                    "task": {
                        "type": "object",
                        "properties": {"title": {"type": "string"}, "when": {"type": "string"}},
                        "required": ["title", "when"]
                    },
                    "note": {"type": "string"}
                },
                "required": ["task"],
                "additionalProperties": false
            }
        }
    })
}

struct Runner<'a> {
    backend: &'a Backend,
    cancel_tx: watch::Sender<bool>,
    cancel: watch::Receiver<bool>,
    emit: Box<dyn Fn(QualificationEvent) + Send + Sync + 'a>,
}

impl<'a> Runner<'a> {
    fn cancelled(&self) -> bool {
        *self.cancel.borrow()
    }
}

async fn case_basic_chat(runner: &mut Runner<'_>) -> CaseResult {
    let name = "basic_chat";
    let started = Instant::now();
    let messages =
        [json!({"role": "user", "content": "Reply with exactly the word READY and nothing else."})];
    match complete(
        runner.backend,
        &messages,
        &[],
        &mut runner.cancel,
        CASE_TIMEOUT,
    )
    .await
    {
        Ok(done) => {
            let streamed = done.chunks >= 2;
            let ok = !done.text.trim().is_empty();
            let mut result = result(
                name,
                if ok && streamed { "passed" } else { "failed" },
                started,
                Some(format!("{} chunks streamed", done.chunks)),
                None,
            );
            result.tokens_per_second = done.tokens_per_second;
            if !ok {
                result.error = Some("The model returned an empty answer.".into());
            } else if !streamed {
                result.detail = Some(
                    "Answer arrived but streaming could not be verified (fewer than 2 chunks)."
                        .into(),
                );
            }
            result
        }
        Err(error) if error == "cancelled" => {
            result(name, "untested", started, None, Some("cancelled".into()))
        }
        Err(error) => result(name, "failed", started, None, Some(error)),
    }
}

async fn case_structured_output(runner: &mut Runner<'_>) -> CaseResult {
    let name = "structured_output";
    let started = Instant::now();
    let messages = [
        json!({"role": "user", "content": "Reply with ONLY a JSON object like {\"name\": \"ada\", \"score\": 7}. No prose."}),
    ];
    match complete(
        runner.backend,
        &messages,
        &[],
        &mut runner.cancel,
        CASE_TIMEOUT,
    )
    .await
    {
        Ok(done) => {
            let trimmed = done
                .text
                .trim()
                .trim_start_matches("```json")
                .trim_start_matches("```")
                .trim_end_matches("```")
                .trim()
                .to_string();
            let parsed = serde_json::from_str::<Value>(&trimmed);
            let ok = parsed.as_ref().is_ok_and(|value| {
                value.is_object() && value["name"].is_string() && value["score"].is_number()
            });
            let mut result = result(
                name,
                if ok { "passed" } else { "failed" },
                started,
                None,
                parsed
                    .err()
                    .map(|error| format!("Not valid JSON: {error}"))
                    .or(if !ok {
                        Some(
                            "Reply was not an object with string 'name' and number 'score'.".into(),
                        )
                    } else {
                        None
                    }),
            );
            result.tokens_per_second = done.tokens_per_second;
            result
        }
        Err(error) if error == "cancelled" => {
            result(name, "untested", started, None, Some("cancelled".into()))
        }
        Err(error) => result(name, "failed", started, None, Some(error)),
    }
}

async fn case_tool_call(runner: &mut Runner<'_>) -> CaseResult {
    let name = "tool_call";
    let started = Instant::now();
    if !runner.backend.supports_tools() {
        return result(
            name,
            "unsupported",
            started,
            Some("The backend reports this model cannot use tools.".into()),
            None,
        );
    }
    let messages = [
        json!({"role": "user", "content": "Use the add tool to add 2 and 3. Do not answer in prose."}),
    ];
    match complete(
        runner.backend,
        &messages,
        &[add_tool()],
        &mut runner.cancel,
        CASE_TIMEOUT,
    )
    .await
    {
        Ok(done) => {
            let call = done.calls.iter().find(|call| call.name == "add");
            match call {
                Some(call) => {
                    let args = &call.arguments;
                    let ok = args["a"].as_i64() == Some(2) && args["b"].as_i64() == Some(3);
                    result(
                        name,
                        if ok { "passed" } else { "failed" },
                        started,
                        None,
                        if ok {
                            None
                        } else {
                            Some(format!("Called 'add' with wrong arguments: {}", args))
                        },
                    )
                }
                None => result(
                    name,
                    "failed",
                    started,
                    None,
                    Some(done.calls_error.unwrap_or_else(|| {
                        format!(
                            "No tool call emitted; reply was: {}",
                            done.text.chars().take(160).collect::<String>()
                        )
                    })),
                ),
            }
        }
        Err(error) if error == "cancelled" => {
            result(name, "untested", started, None, Some("cancelled".into()))
        }
        Err(error) => result(name, "failed", started, None, Some(error)),
    }
}

async fn case_tool_args_parsing(runner: &mut Runner<'_>) -> CaseResult {
    let name = "tool_args_parsing";
    let started = Instant::now();
    if !runner.backend.supports_tools() {
        return result(
            name,
            "unsupported",
            started,
            Some("The backend reports this model cannot use tools.".into()),
            None,
        );
    }
    let messages = [
        json!({"role": "user", "content": "Call schedule_reminder with task {title: \"water plants\", when: \"tomorrow 8am\"} and note \"it's raining\". Do not answer in prose."}),
    ];
    match complete(
        runner.backend,
        &messages,
        &[schedule_tool()],
        &mut runner.cancel,
        CASE_TIMEOUT,
    )
    .await
    {
        Ok(done) => {
            let call = done
                .calls
                .iter()
                .find(|call| call.name == "schedule_reminder");
            match call {
                Some(call) => {
                    let task = &call.arguments["task"];
                    let ok = task["title"].is_string()
                        && task["when"].is_string()
                        && call.arguments["note"].is_string();
                    result(
                        name,
                        if ok { "passed" } else { "failed" },
                        started,
                        None,
                        if ok {
                            None
                        } else {
                            Some(format!("Nested arguments incomplete: {}", call.arguments))
                        },
                    )
                }
                None => result(
                    name,
                    "failed",
                    started,
                    None,
                    Some("No tool call emitted for nested arguments.".into()),
                ),
            }
        }
        Err(error) if error == "cancelled" => {
            result(name, "untested", started, None, Some("cancelled".into()))
        }
        Err(error) => result(name, "failed", started, None, Some(error)),
    }
}

/// Feed the model a malformed tool reply exactly like the turn loop does and
/// require it to recover on one retry.
async fn case_malformed_recovery(runner: &mut Runner<'_>) -> CaseResult {
    let name = "malformed_recovery";
    let started = Instant::now();
    if !runner.backend.supports_tools() {
        return result(
            name,
            "unsupported",
            started,
            Some("The backend reports this model cannot use tools.".into()),
            None,
        );
    }
    // Round 1: the tool result is corrupt (what the loop records when a call
    // is malformed). Round 2 must still produce a valid call or clean answer.
    let messages = [
        json!({"role": "user", "content": "Use the add tool to add 4 and 5."}),
        json!({"role": "assistant", "content": "add(4,5) please", "tool_calls": [{"id": "call-1", "type": "function", "function": {"name": "add", "arguments": "{\"a\":4,"}}]}),
        json!({"role": "tool", "tool_call_id": "call-1", "content": "{\"isError\":true,\"message\":\"Tool arguments are not valid JSON. The generation may have been truncated; unfinished tool calls are not executed.\"}"}),
    ];
    match complete(
        runner.backend,
        &messages,
        &[add_tool()],
        &mut runner.cancel,
        CASE_TIMEOUT,
    )
    .await
    {
        Ok(done) => {
            let recovered = !done.calls.is_empty() || !done.text.trim().is_empty();
            let malformed_again = done.calls.iter().any(|call| call.arguments.is_null());
            result(
                name,
                if recovered && !malformed_again {
                    "passed"
                } else {
                    "failed"
                },
                started,
                Some("After a malformed tool result the model was given one retry.".into()),
                if recovered && !malformed_again {
                    None
                } else {
                    Some("The model could not recover from a malformed tool result.".into())
                },
            )
        }
        Err(error) if error == "cancelled" => {
            result(name, "untested", started, None, Some("cancelled".into()))
        }
        Err(error) => result(name, "failed", started, None, Some(error)),
    }
}

/// Write a function, then run a deterministic check — only where local
/// execution is actually configured; otherwise honestly 'untested'.
async fn case_simple_coding(state: &AppState, runner: &mut Runner<'_>) -> CaseResult {
    let name = "simple_coding";
    let started = Instant::now();
    let messages = [
        json!({"role": "user", "content": "Write a Python function `def add(a, b)` that returns their sum, then on the next line print(add(2, 3)). Output ONLY runnable Python, no fences, no prose."}),
    ];
    let done = match complete(
        runner.backend,
        &messages,
        &[],
        &mut runner.cancel,
        CASE_TIMEOUT,
    )
    .await
    {
        Ok(done) => done,
        Err(error) if error == "cancelled" => {
            return result(name, "untested", started, None, Some("cancelled".into()));
        }
        Err(error) => return result(name, "failed", started, None, Some(error)),
    };
    let python = state
        .database()
        .ok()
        .and_then(|store| store.execution_config().ok())
        .map(|config| config.python_path)
        .filter(|path| !path.trim().is_empty());
    let Some(python) = python else {
        return result(
            name,
            "untested",
            started,
            Some("No local Python interpreter is configured.".into()),
            None,
        );
    };
    let code = done
        .text
        .trim_start_matches("```python")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim()
        .to_string();
    let output = tokio::process::Command::new(&python)
        .arg("-c")
        .arg(&code)
        .output()
        .await;
    match output {
        Ok(out) if out.status.success() && String::from_utf8_lossy(&out.stdout).contains('5') => {
            result(
                name,
                "passed",
                started,
                Some("Generated code ran and printed the expected answer.".into()),
                None,
            )
        }
        Ok(out) => result(
            name,
            "failed",
            started,
            None,
            Some(format!(
                "Generated code failed to run or printed wrong output: {}",
                String::from_utf8_lossy(&out.stderr)
                    .chars()
                    .take(160)
                    .collect::<String>()
            )),
        ),
        Err(error) => result(
            name,
            "untested",
            started,
            Some(format!("Python could not be executed: {error}")),
            None,
        ),
    }
}

/// Only meaningful when web search is configured; otherwise 'untested'.
async fn case_grounded_research(state: &AppState, runner: &mut Runner<'_>) -> CaseResult {
    let name = "grounded_research";
    let started = Instant::now();
    let configured = state
        .database()
        .ok()
        .and_then(|store| store.setting::<String>("searxng_base_url").ok())
        .is_some_and(|url| !url.trim().is_empty());
    if !configured {
        return result(
            name,
            "untested",
            started,
            Some("Web search is not configured; grounding cannot be tested.".into()),
            None,
        );
    }
    let evidence = "Source: usgs.gov — Mount Rainier rises 4,392 m above sea level.";
    let messages = [
        json!({"role": "user", "content": format!("Using ONLY this source: {evidence}\nHow tall is Mount Rainier? Reply with the height and nothing else.")}),
    ];
    match complete(
        runner.backend,
        &messages,
        &[],
        &mut runner.cancel,
        CASE_TIMEOUT,
    )
    .await
    {
        Ok(done) => {
            let ok = done.text.contains("4,392") || done.text.contains("4392");
            result(
                name,
                if ok { "passed" } else { "failed" },
                started,
                None,
                if ok {
                    None
                } else {
                    Some(format!(
                        "Answer did not reflect the supplied source: {}",
                        done.text.chars().take(120).collect::<String>()
                    ))
                },
            )
        }
        Err(error) if error == "cancelled" => {
            result(name, "untested", started, None, Some("cancelled".into()))
        }
        Err(error) => result(name, "failed", started, None, Some(error)),
    }
}

/// Cancellation must actually stop a long generation and leave the runtime
/// usable — a hang here means the model cannot be trusted behind a queue.
async fn case_cancellation(runner: &mut Runner<'_>) -> CaseResult {
    let name = "cancellation";
    let started = Instant::now();
    let messages =
        [json!({"role": "user", "content": "Count from 1 to 500, one number per line."})];
    // The case cancels on a case-local channel; a user cancel on the suite
    // channel is forwarded so both reach the in-flight generation — and the
    // suite flag still reads as the USER's cancel afterward.
    let (case_tx, mut case_rx) = watch::channel(false);
    {
        let case_tx = case_tx.clone();
        let mut suite_rx = runner.cancel_tx.subscribe();
        tokio::spawn(async move {
            let _ = suite_rx.changed().await;
            case_tx.send_replace(true);
        });
    }
    let mut generation = std::pin::pin!(complete(
        runner.backend,
        &messages,
        &[],
        &mut case_rx,
        Duration::from_secs(60)
    ));
    // Give the request a moment to get in flight, then cancel — polling in
    // slices keeps the borrow on the same task (no spawn, no 'static).
    let mut outcome = None;
    let mut triggered = false;
    while outcome.is_none() {
        match tokio::time::timeout(Duration::from_millis(50), &mut generation).await {
            Ok(result) => outcome = Some(result),
            Err(_) => {
                if !triggered && started.elapsed() >= Duration::from_millis(400) {
                    triggered = true;
                    case_tx.send_replace(true);
                }
                if started.elapsed() > Duration::from_millis(400) + CANCEL_GRACE {
                    break;
                }
            }
        }
    }
    match outcome {
        Some(Err(error)) if error == "cancelled" => {
            // The model must still answer a trivial ping — a cancel that
            // wedges the runtime is a fail, not a pass.
            let messages = [json!({"role": "user", "content": "Reply with OK."})];
            let mut cancel = runner.cancel_tx.subscribe();
            match complete(
                runner.backend,
                &messages,
                &[],
                &mut cancel,
                Duration::from_secs(15),
            )
            .await
            {
                Ok(done) if !done.text.trim().is_empty() => result(
                    name,
                    "passed",
                    started,
                    Some("Generation stopped promptly and the model stayed responsive.".into()),
                    None,
                ),
                _ => result(
                    name,
                    "failed",
                    started,
                    None,
                    Some("The model stopped but did not answer a follow-up ping.".into()),
                ),
            }
        }
        _ => result(
            name,
            "failed",
            started,
            None,
            Some(format!(
                "A running generation did not stop within {}s of cancel.",
                CANCEL_GRACE.as_secs()
            )),
        ),
    }
}

async fn run_suite(
    state: &AppState,
    target: &SuiteTarget,
    identity_hash: &str,
    emit: impl Fn(QualificationEvent) + Send + Sync,
    cancel_tx: watch::Sender<bool>,
) -> Qualification {
    let started_at = crate::store::now();
    let mut runner = Runner {
        backend: &target.backend,
        cancel: cancel_tx.subscribe(),
        cancel_tx,
        emit: Box::new(emit),
    };
    let deadline = Instant::now() + SUITE_TIMEOUT;
    let mut cases = Vec::new();
    macro_rules! case {
        ($name:literal, $future:expr) => {{
            let outcome = if runner.cancelled() || Instant::now() > deadline {
                CaseResult {
                    name: $name.into(),
                    status: "untested".into(),
                    latency_ms: 0,
                    tokens_per_second: None,
                    error: Some(if runner.cancelled() {
                        "cancelled".into()
                    } else {
                        "Suite time budget exhausted.".into()
                    }),
                    detail: None,
                }
            } else {
                $future.await
            };
            (runner.emit)(QualificationEvent::Case {
                name: outcome.name.clone(),
                status: outcome.status.clone(),
                result: Some(outcome.clone()),
            });
            cases.push(outcome);
        }};
    }
    case!("basic_chat", case_basic_chat(&mut runner));
    case!("structured_output", case_structured_output(&mut runner));
    case!("tool_call", case_tool_call(&mut runner));
    case!("tool_args_parsing", case_tool_args_parsing(&mut runner));
    case!("malformed_recovery", case_malformed_recovery(&mut runner));
    case!("simple_coding", case_simple_coding(state, &mut runner));
    case!(
        "grounded_research",
        case_grounded_research(state, &mut runner)
    );
    case!("cancellation", case_cancellation(&mut runner));
    let cancelled = runner.cancelled();
    let status = if cancelled {
        "cancelled"
    } else {
        let core: Vec<&CaseResult> = cases
            .iter()
            .filter(|case| CORE_CASES.contains(&case.name.as_str()))
            .collect();
        let passing = core.iter().filter(|case| case.status == "passed").count();
        if passing == core.len() {
            "passed"
        } else if passing == 0 {
            "failed"
        } else {
            "partial"
        }
    };
    Qualification {
        id: uuid::Uuid::new_v4().to_string(),
        identity_hash: identity_hash.to_string(),
        stale: false,
        model_id: target.model_id.clone(),
        model_path: target.model_path.clone(),
        provider_id: target.provider_id.clone(),
        runtime_version: target.runtime_version.clone(),
        config: target.config.clone(),
        started_at,
        ended_at: Some(crate::store::now()),
        cases,
        status: status.into(),
    }
}

// ---------------------------------------------------------------------------
// Readiness + profiles

fn action(label: &str, target: &str) -> Option<ReadinessAction> {
    Some(ReadinessAction {
        label: label.into(),
        target: target.into(),
    })
}

/// Detect llama.cpp-class memory failures from the runtime log tail and, for
/// small-memory machines, suggest the lever that actually helps.
fn oom_suggestion(log_tail: &str, memory_bytes: u64) -> Option<String> {
    let lowered = log_tail.to_lowercase();
    let looks_oom = [
        "out of memory",
        "oom",
        "failed to allocate",
        "vk_error_out_of_device_memory",
        "insufficient memory",
    ]
    .iter()
    .any(|pattern| lowered.contains(pattern));
    if !looks_oom {
        return None;
    }
    let suggestion = if memory_bytes <= 18 * 1024 * 1024 * 1024 {
        "Try fewer GPU layers (or CPU only) and a smaller context — 4 GB VRAM / 16 GB RAM machines usually need both lowered."
    } else {
        "Try fewer GPU layers or a smaller context."
    };
    Some(suggestion.into())
}

pub async fn readiness_state(state: &AppState) -> Result<Readiness, String> {
    let preferences = state
        .database()
        .and_then(|store| store.preferences())
        .unwrap_or_default();
    if preferences.runtime_path.trim().is_empty()
        || !std::path::Path::new(&preferences.runtime_path).exists()
    {
        return Ok(Readiness {
            step: "runtime-missing".into(),
            message: "No local runtime is installed.".into(),
            next_action: action("Install a runtime in Models", "models"),
        });
    }
    let mut runtime = state.runtime.lock().await;
    let status = runtime.inspect();
    let log_tail = runtime.log_tail(64 * 1024);
    drop(runtime);
    let model_path = status
        .model_path
        .clone()
        .filter(|path| !path.trim().is_empty())
        .unwrap_or(preferences.model_path.clone());
    if model_path.trim().is_empty() {
        return Ok(Readiness {
            step: "model-missing".into(),
            message: "No model is selected.".into(),
            next_action: action("Choose a model in Models", "models"),
        });
    }
    if status.phase == "loading" {
        return Ok(Readiness {
            step: "loading".into(),
            message: status.message.clone(),
            next_action: None,
        });
    }
    if status.phase == "error" {
        // OOM shows in the runtime's stderr log, not always in the status —
        // check both before naming a cause.
        let memory = crate::hardware::hardware_status()
            .await
            .memory_total_bytes
            .unwrap_or(u64::MAX);
        let detail = format!("{}\n{}", status.message, log_tail);
        let message = oom_suggestion(&detail, memory).unwrap_or(status.message.clone());
        return Ok(Readiness {
            step: "load-failed".into(),
            message,
            next_action: action("Adjust the runtime config in Models", "models"),
        });
    }
    if status.phase != "ready" {
        return Ok(Readiness {
            step: "not-loaded".into(),
            message: "A model is selected but not loaded.".into(),
            next_action: action("Load the model in Models", "models"),
        });
    }
    // A stored qualification is only evidence for the identity it was
    // measured on — a different config is a different answer.
    let selection = state
        .database()
        .and_then(|store| store.preferred_model())
        .unwrap_or_default();
    let identity = current_identity(state, &selection).await;
    let store = state.database()?;
    if let Some(latest) = store.latest_qualification(&identity)? {
        return Ok(match latest.status.as_str() {
            "passed" | "partial" => Readiness {
                step: "ready".into(),
                message: "Model is loaded and qualified.".into(),
                next_action: None,
            },
            "cancelled" => Readiness {
                step: "not-qualified".into(),
                message: "The last qualification run was cancelled.".into(),
                next_action: action("Run qualification in Models", "models"),
            },
            _ => Readiness {
                step: "not-qualified".into(),
                message: format!("The last qualification run {}.", latest.status),
                next_action: action("Run qualification in Models", "models"),
            },
        });
    }
    // Nothing stored for this exact identity — a record for a different
    // identity means something changed since the model was last measured.
    if store
        .latest_qualification_any()?
        .is_some_and(|record| record.identity_hash != identity)
    {
        return Ok(Readiness {
            step: "qualification-stale".into(),
            message: "The model or its configuration changed since it was last qualified.".into(),
            next_action: action("Re-run qualification in Models", "models"),
        });
    }
    Ok(Readiness {
        step: "not-qualified".into(),
        message: "This model has never been qualified.".into(),
        next_action: action("Run qualification in Models", "models"),
    })
}

/// The identity hash the current selection would qualify under — model file
/// fingerprint, runtime config, and provider are all part of what makes a
/// stored result meaningful.
async fn current_identity(state: &AppState, selection: &ModelSelection) -> String {
    match target_for(state, selection).await {
        Ok(target) => target.identity_hash(&file_fingerprint(target.model_path.as_deref())),
        Err(_) => String::new(),
    }
}

fn row_to_qualification(row: crate::store::QualificationRow, stale: bool) -> Qualification {
    Qualification {
        id: row.id,
        identity_hash: row.identity_hash,
        stale,
        model_id: row.model_id,
        model_path: row.model_path,
        provider_id: row.provider_id,
        runtime_version: row.runtime_version,
        config: serde_json::from_str(&row.config_json).unwrap_or_default(),
        started_at: row.started_at,
        ended_at: row.ended_at,
        cases: serde_json::from_str(&row.results_json).unwrap_or_default(),
        status: row.status,
    }
}

/// Profile → preset + interaction mode mapping. Recommended derives from the
/// latest qualification for the current identity, never from a label.
pub async fn profiles_state(state: &AppState) -> Vec<Profile> {
    let selection = state
        .database()
        .and_then(|store| store.preferred_model())
        .unwrap_or_default();
    let identity = current_identity(state, &selection).await;
    let latest = state
        .database()
        .ok()
        .and_then(|store| store.latest_qualification(&identity).ok().flatten())
        .map(|row| row_to_qualification(row, false));
    let tool_ok = latest
        .as_ref()
        .map(|record| {
            record
                .cases
                .iter()
                .filter(|case| case.name == "tool_call" || case.name == "tool_args_parsing")
                .all(|case| case.status == "passed")
        })
        .unwrap_or(false);
    let research_ok = latest
        .as_ref()
        .map(|record| {
            record
                .cases
                .iter()
                .find(|case| case.name == "grounded_research")
                .is_none_or(|case| case.status != "failed")
        })
        .unwrap_or(false);
    let qualified = latest
        .as_ref()
        .is_some_and(|record| record.status == "passed" || record.status == "partial");
    let tool_note = (!tool_ok)
        .then(|| "Tool calls failed qualification — this profile may mislead.".to_string());
    let research_note = (!research_ok).then(|| "Grounded research was not verified.".to_string());
    vec![
        Profile {
            id: "chat".into(),
            label: "Chat".into(),
            preset: crate::presets::CHAT.into(),
            interaction_mode: "act".into(),
            recommended: !tool_ok || !qualified,
            note: None,
        },
        Profile {
            id: "coding".into(),
            label: "Coding".into(),
            preset: crate::presets::CODING.into(),
            interaction_mode: "act".into(),
            recommended: tool_ok,
            note: tool_note.clone(),
        },
        Profile {
            id: "research".into(),
            label: "Research".into(),
            preset: crate::presets::RESEARCH.into(),
            interaction_mode: "act".into(),
            recommended: tool_ok && research_ok,
            note: tool_note.clone().or(research_note),
        },
        Profile {
            id: "advanced".into(),
            label: "Advanced".into(),
            preset: crate::presets::CREATOR.into(),
            interaction_mode: "act".into(),
            recommended: false,
            note: tool_note,
        },
    ]
}

// ---------------------------------------------------------------------------
// Commands

#[tauri::command]
pub async fn qualification_run(
    app: AppHandle,
    state: State<'_, AppState>,
    selection: ModelSelection,
    channel: tauri::ipc::Channel<QualificationEvent>,
) -> Result<(), String> {
    let lease =
        crate::resources::try_acquire_inference(&state, "qualification", "model qualification")
            .map_err(|busy| format!("Qualification cannot run while {}.", busy.reason))?;
    let target = target_for(&state, &selection).await?;
    let fingerprint = file_fingerprint(target.model_path.as_deref());
    let identity = target.identity_hash(&fingerprint);
    let id = uuid::Uuid::new_v4().to_string();
    // Scope the store guard: a std MutexGuard held across the suite's awaits
    // would make this command's future non-Send.
    state
        .database()?
        .insert_qualification(&crate::store::QualificationRow {
            id: id.clone(),
            identity_hash: identity.clone(),
            model_id: target.model_id.clone(),
            model_path: target.model_path.clone(),
            model_fingerprint: Some(fingerprint),
            runtime_version: target.runtime_version.clone(),
            provider_id: target.provider_id.clone(),
            config_json: target.config.to_string(),
            started_at: crate::store::now(),
            ended_at: None,
            results_json: "[]".into(),
            status: "cancelled".into(),
        })?;
    // Its own watch channel so a stale `true` left by a previous turn cancel
    // cannot wedge the suite.
    let (cancel_tx, _) = watch::channel(false);
    if let Ok(mut qualifying) = state.qualifying.lock() {
        *qualifying = Some(cancel_tx.clone());
    }
    let _ = app.emit("tasks-changed", json!({}));
    let emit = {
        let channel = channel.clone();
        move |event: QualificationEvent| {
            let _ = channel.send(event);
        }
    };
    let mut record = run_suite(&state, &target, &identity, emit, cancel_tx).await;
    record.id = id;
    drop(lease);
    let _ = state.database().and_then(|store| {
        store.finalize_qualification(
            &record.id,
            &serde_json::to_string(&record.cases).unwrap_or_default(),
            &record.status,
            crate::store::now(),
        )
    });
    if let Ok(mut qualifying) = state.qualifying.lock() {
        *qualifying = None;
    }
    let _ = channel.send(QualificationEvent::Complete {
        record: record.clone(),
    });
    let _ = app.emit("tasks-changed", json!({}));
    Ok(())
}

#[tauri::command]
pub fn qualification_cancel(state: State<'_, AppState>) -> Result<(), String> {
    let cancel = state
        .qualifying
        .lock()
        .ok()
        .and_then(|guard| guard.clone())
        .ok_or("No qualification is running.")?;
    cancel.send_replace(true);
    Ok(())
}

async fn results_state(
    state: &AppState,
    selection: Option<ModelSelection>,
) -> Result<Option<Qualification>, String> {
    let selection = match selection {
        Some(selection) => selection,
        None => state
            .database()
            .and_then(|store| store.preferred_model())
            .unwrap_or_default(),
    };
    let identity = current_identity(state, &selection).await;
    let store = state.database()?;
    Ok(store
        .latest_qualification(&identity)?
        .map(|row| row_to_qualification(row, false))
        .or_else(|| {
            // No record for this identity — surface the latest record with a
            // stale flag so the UI can say exactly what expired.
            store
                .latest_qualification_any()
                .ok()
                .flatten()
                .filter(|row| row.identity_hash != identity)
                .map(|row| row_to_qualification(row, true))
        }))
}

#[tauri::command]
pub async fn qualification_results(
    state: State<'_, AppState>,
    selection: Option<ModelSelection>,
) -> Result<Option<Qualification>, String> {
    results_state(&state, selection).await
}

#[tauri::command]
pub async fn readiness(state: State<'_, AppState>) -> Result<Readiness, String> {
    readiness_state(&state).await
}

#[tauri::command]
pub async fn profiles(state: State<'_, AppState>) -> Result<Vec<Profile>, String> {
    Ok(profiles_state(&state).await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{OPENAI_CHAT_COMPLETIONS, ProviderConnection, RemoteModel, ToolSupport};
    use serde_json::json;
    use std::io::{Read, Write};

    /// A loopback OpenAI-compatible mock. `respond` sees the request JSON and
    /// returns SSE data lines (values, not "data:" prefixed). Events trickle
    /// so cancellation tests have an in-flight stream to cut.
    fn spawn_mock_model(respond: impl Fn(&Value) -> Vec<Value> + Send + Sync + 'static) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let respond = std::sync::Arc::new(respond);
        std::thread::spawn(move || {
            for connection in listener.incoming() {
                let Ok(mut stream) = connection else { continue };
                let respond = respond.clone();
                std::thread::spawn(move || {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                    let mut request = Vec::new();
                    let mut buffer = [0u8; 4096];
                    let request_json = loop {
                        let Ok(count) = stream.read(&mut buffer) else {
                            return;
                        };
                        if count == 0 {
                            return;
                        };
                        request.extend_from_slice(&buffer[..count]);
                        if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                            let headers =
                                String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                            let length: usize = headers
                                .lines()
                                .find_map(|line| line.strip_prefix("content-length: "))
                                .unwrap_or("0")
                                .parse()
                                .unwrap_or(0);
                            if request.len() >= end + 4 + length {
                                break serde_json::from_slice::<Value>(
                                    &request[end + 4..end + 4 + length],
                                )
                                .unwrap_or_default();
                            }
                        }
                    };
                    let mut sse = String::new();
                    for event in respond(&request_json) {
                        sse.push_str(&format!("data: {}\n\n", event));
                    }
                    sse.push_str("data: [DONE]\n\n");
                    if stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n")
                        .is_err()
                    {
                        return;
                    }
                    // Trickle so an in-flight cancellation is observable.
                    let mut rest = sse.as_str();
                    while let Some(end) = rest.find("\n\n") {
                        if stream
                            .write_all(format!("{}\n\n", &rest[..end]).as_bytes())
                            .is_err()
                        {
                            return;
                        }
                        let _ = stream.flush();
                        rest = &rest[end + 2..];
                        std::thread::sleep(Duration::from_millis(10));
                    }
                });
            }
        });
        format!("http://{address}")
    }

    fn sse_content(text: &str) -> Vec<Value> {
        text.split_inclusive(char::is_whitespace)
            .map(|chunk| json!({"choices":[{"delta":{"content": chunk}}]}))
            .chain(std::iter::once(json!({"choices":[{"delta":{},"finish_reason":"stop"}],"usage":{"completion_tokens":8,"prompt_tokens":4}})))
            .collect()
    }

    fn provider_with_model(
        base_url: String,
        tool_support: ToolSupport,
    ) -> (ProviderConnection, RemoteModel) {
        (
            ProviderConnection {
                id: "qualify-test".into(),
                name: "Fixture".into(),
                api_format: OPENAI_CHAT_COMPLETIONS.into(),
                base_url,
                verified: true,
                last_tested_at: None,
                models: Vec::new(),
                has_api_key: true,
            },
            RemoteModel {
                id: "qualify-model".into(),
                context_length: Some(8192),
                max_output_tokens: Some(2048),
                supports_images: false,
                tool_support,
            },
        )
    }

    #[tokio::test]
    async fn non_loopback_remote_is_rejected_before_any_request() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::web_search::tests::test_state(dir.path());
        let (mut provider, model) =
            provider_with_model("https://api.example.com/v1".into(), ToolSupport::Supported);
        provider.models = vec![model];
        state.database().unwrap().save_provider(&provider).unwrap();
        let error = target_for(
            &state,
            &ModelSelection {
                provider_id: Some("qualify-test".into()),
                model_id: "qualify-model".into(),
            },
        )
        .await
        .map(|_| ())
        .unwrap_err();
        assert!(error.contains("local model"), "{error}");
    }

    #[tokio::test]
    async fn tool_call_failure_fails_qualification_and_empowers_chat_only() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::web_search::tests::test_state(dir.path());
        let base = spawn_mock_model(|request| {
            let messages = request["messages"].as_array().cloned().unwrap_or_default();
            let last = messages
                .last()
                .and_then(|m| m["content"].as_str())
                .unwrap_or("");
            if request.get("tools").is_some() {
                // Tool-capable prompts only get prose back.
                sse_content("I can't call tools.")
            } else if last.contains("READY") {
                sse_content("READY")
            } else if last.contains("JSON") {
                sse_content("{\"name\":\"ada\",\"score\":7}")
            } else if last.contains("Count") {
                (1..=50)
                    .map(|n| json!({"choices":[{"delta":{"content": format!("{n}\n")}}]}))
                    .collect()
            } else {
                sse_content("OK")
            }
        });
        let (mut provider, model) = provider_with_model(base, ToolSupport::Supported);
        provider.models = vec![model];
        state.database().unwrap().save_provider(&provider).unwrap();
        let selection = ModelSelection {
            provider_id: Some("qualify-test".into()),
            model_id: "qualify-model".into(),
        };
        state
            .database()
            .unwrap()
            .save_preferred_model(&selection)
            .unwrap();
        let target = target_for(&state, &selection).await.unwrap();
        let record = run_suite(
            &state,
            &target,
            &target.identity_hash("fp"),
            |_| {},
            watch::channel(false).0,
        )
        .await;
        assert_eq!(record.status, "partial", "{:?}", record.cases);
        assert_eq!(
            record
                .cases
                .iter()
                .find(|c| c.name == "basic_chat")
                .unwrap()
                .status,
            "passed"
        );
        assert_eq!(
            record
                .cases
                .iter()
                .find(|c| c.name == "tool_call")
                .unwrap()
                .status,
            "failed"
        );
        let profiles = profiles_state(&state).await;
        assert!(
            profiles
                .iter()
                .find(|p| p.id == "chat")
                .unwrap()
                .recommended
        );
        let coding = profiles.iter().find(|p| p.id == "coding").unwrap();
        assert!(!coding.recommended);
        assert!(
            coding
                .note
                .as_deref()
                .unwrap_or("")
                .contains("Tool calls failed")
        );
    }

    #[tokio::test]
    async fn a_streaming_generation_cancels_and_the_model_stays_healthy() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::web_search::tests::test_state(dir.path());
        let base = spawn_mock_model(|request| {
            let last = request["messages"]
                .as_array()
                .and_then(|m| m.last())
                .and_then(|m| m["content"].as_str())
                .unwrap_or("");
            if last.contains("Count") {
                (1..=200)
                    .map(|n| json!({"choices":[{"delta":{"content": format!("{n}\n")}}]}))
                    .collect()
            } else {
                sse_content("OK")
            }
        });
        let (mut provider, model) = provider_with_model(base, ToolSupport::Supported);
        provider.models = vec![model];
        state.database().unwrap().save_provider(&provider).unwrap();
        let selection = ModelSelection {
            provider_id: Some("qualify-test".into()),
            model_id: "qualify-model".into(),
        };
        let target = target_for(&state, &selection).await.unwrap();
        let cancel_tx = watch::channel(false).0;
        let mut runner = Runner {
            backend: &target.backend,
            cancel: cancel_tx.subscribe(),
            cancel_tx,
            emit: Box::new(|_| {}),
        };
        let result = case_cancellation(&mut runner).await;
        assert_eq!(result.status, "passed", "{:?}", result.error);
        assert!(result.latency_ms < 5000);
    }

    #[tokio::test]
    async fn stored_results_go_stale_when_the_identity_changes() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::web_search::tests::test_state(dir.path());
        {
            let store = state.database().unwrap();
            store
                .insert_qualification(&crate::store::QualificationRow {
                    id: "q1".into(),
                    identity_hash: "hash-a".into(),
                    model_id: "m".into(),
                    model_path: None,
                    model_fingerprint: None,
                    runtime_version: None,
                    provider_id: None,
                    config_json: "{}".into(),
                    started_at: 1,
                    ended_at: Some(2),
                    results_json: "[]".into(),
                    status: "passed".into(),
                })
                .unwrap();
            assert!(store.latest_qualification("hash-a").unwrap().is_some());
            assert!(store.latest_qualification("hash-b").unwrap().is_none());
        }
        // The stored row is for a different identity than the current
        // selection would produce — it must come back flagged stale.
        let result = results_state(&state, None).await.unwrap().unwrap();
        assert!(result.stale);
    }

    #[tokio::test]
    async fn readiness_orders_the_setup_steps() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::web_search::tests::test_state(dir.path());
        let first = readiness_state(&state).await.unwrap();
        assert_eq!(first.step, "runtime-missing");
        let runtime_dir = dir.path().join("runtime");
        std::fs::create_dir_all(&runtime_dir).unwrap();
        let runtime_bin = runtime_dir.join("server.exe");
        std::fs::write(&runtime_bin, "stub").unwrap();
        {
            let store = state.database().unwrap();
            let mut prefs = store.preferences().unwrap();
            prefs.runtime_path = runtime_bin.display().to_string();
            store.save_preferences(&prefs).unwrap();
        }
        let second = readiness_state(&state).await.unwrap();
        assert_eq!(second.step, "model-missing");
        {
            let store = state.database().unwrap();
            let mut prefs = store.preferences().unwrap();
            prefs.model_path = "model.gguf".into();
            store.save_preferences(&prefs).unwrap();
        }
        let third = readiness_state(&state).await.unwrap();
        assert_eq!(third.step, "not-loaded");
    }

    #[test]
    fn oom_suggestion_scales_to_small_memory() {
        let big = oom_suggestion("CUDA out of memory", 64 * 1024 * 1024 * 1024).unwrap();
        assert!(big.contains("fewer GPU layers") && !big.contains("4 GB VRAM"));
        let small = oom_suggestion("failed to allocate buffer", 16 * 1024 * 1024 * 1024).unwrap();
        assert!(small.contains("4 GB VRAM") && small.contains("smaller context"));
        assert!(oom_suggestion("some other failure", 16 * 1024 * 1024 * 1024).is_none());
    }
    /// Live qualification against a real llama.cpp + GGUF on this machine.
    /// Run:
    ///   $env:LOCALLM_LIVE_SERVER="...\llama-server.exe"
    ///   $env:LOCALLM_LIVE_MODEL="...\model.gguf"
    ///   cargo test --lib live_qualification -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_qualification_against_llama_cpp() {
        let server = std::env::var("LOCALLM_LIVE_SERVER").unwrap_or_default();
        let model = std::env::var("LOCALLM_LIVE_MODEL").unwrap_or_default();
        if server.is_empty()
            || model.is_empty()
            || !std::path::Path::new(&server).exists()
            || !std::path::Path::new(&model).exists()
        {
            eprintln!(
                "live qualification skipped: set LOCALLM_LIVE_SERVER + LOCALLM_LIVE_MODEL to existing files"
            );
            return;
        }
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut child = tokio::process::Command::new(&server)
            .args([
                "--host",
                "127.0.0.1",
                "--port",
                &port.to_string(),
                "--model",
                &model,
                "--ctx-size",
                "2048",
            ])
            .kill_on_drop(true)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let base = format!("http://127.0.0.1:{port}");
        let client = reqwest::Client::new();
        let mut healthy = false;
        for _ in 0..120 {
            if client
                .get(format!("{base}/health"))
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false)
            {
                healthy = true;
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        assert!(
            healthy,
            "llama.cpp server did not become healthy within 120s"
        );

        let dir = tempfile::tempdir().unwrap();
        let state = crate::web_search::tests::test_state(dir.path());
        let (mut provider, model_row) = provider_with_model(base, ToolSupport::Supported);
        provider.models = vec![model_row];
        state.database().unwrap().save_provider(&provider).unwrap();
        let selection = ModelSelection {
            provider_id: Some("qualify-test".into()),
            model_id: "qualify-model".into(),
        };
        let target = target_for(&state, &selection).await.unwrap();
        let started = Instant::now();
        let record = run_suite(
            &state,
            &target,
            &target.identity_hash("live"),
            |event| {
                if let QualificationEvent::Case {
                    name,
                    status,
                    result,
                } = &event
                {
                    eprintln!(
                        "[case] {name} -> {status} {:?}",
                        result.as_ref().map(|r| r.latency_ms)
                    );
                }
            },
            watch::channel(false).0,
        )
        .await;
        eprintln!(
            "\n=== live qualification: {} in {:.1}s ===",
            record.status,
            started.elapsed().as_secs_f32()
        );
        for case in &record.cases {
            eprintln!(
                "  {:<20} {:<12} {:>6}ms  tok/s={:?}  err={:?}",
                case.name,
                case.status,
                case.latency_ms,
                case.tokens_per_second.map(|v| (v * 10.0).round() / 10.0),
                case.error
            );
        }
        child.kill().await.ok();
        assert_ne!(record.status, "failed", "model produced no usable output");
    }
}
