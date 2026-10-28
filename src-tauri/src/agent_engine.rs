//! Shared scheduling and outcomes for foreground and child sessions.
//! Transport, dispatch authority and presentation belong to the session host.
use futures_util::{stream::BoxStream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub(crate) const OUTPUT_RECOVERY_LIMIT: u32 = 2;
pub(crate) const RETRY_TRUNCATED_CALL: &str = "The previous response hit the output token limit before completing. No tool from that response was executed. Continue the user's task using the available tools. Retry the unfinished call as a fresh complete call; do not assume any file was saved. Keep tool calls small. For a large document or program, create a minimal valid file first, then edit it in smaller steps. Do not ask the user to repeat the request.";

#[derive(Default, Debug, Clone)]
pub struct ThinkFilter {
    pub inside_think: bool,
    pub buffer: String,
}

impl ThinkFilter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_thinking(&self) -> bool {
        self.inside_think
    }

    /// Feeds incremental tokens and returns (answer_chunk, reasoning_chunk)
    pub fn feed(&mut self, text: &str) -> (String, String) {
        let mut answer = String::new();
        let mut reasoning = String::new();

        for ch in text.chars() {
            self.buffer.push(ch);
            let target = if self.inside_think {
                "</think>"
            } else {
                "<think>"
            };

            if target.starts_with(&self.buffer) {
                if self.buffer == target {
                    self.buffer.clear();
                    self.inside_think = !self.inside_think;
                }
            } else {
                while !self.buffer.is_empty() && !target.starts_with(&self.buffer) {
                    let first_char = self.buffer.remove(0);
                    if self.inside_think {
                        reasoning.push(first_char);
                    } else {
                        answer.push(first_char);
                    }
                }
                if self.buffer == target {
                    self.buffer.clear();
                    self.inside_think = !self.inside_think;
                }
            }
        }

        (answer, reasoning)
    }

    pub fn flush(&mut self) -> (String, String) {
        let mut answer = String::new();
        let mut reasoning = String::new();
        if !self.buffer.is_empty() {
            if self.inside_think {
                reasoning.push_str(&self.buffer);
            } else {
                answer.push_str(&self.buffer);
            }
            self.buffer.clear();
        }
        (answer, reasoning)
    }
}

/// Only a native-completed provider response can produce an accepted round.
/// Cutoffs are journaled attempts and carry no dispatchable tool batch.
pub(crate) struct ProviderRoundRecord {
    pub finish: String,
    pub calls: Vec<crate::tool_calls::ToolCall>,
    pub assistant_message: Value,
}

/// Provider deltas have one interpretation regardless of the session host.
/// Rendering and durable storage consume these parts without reparsing them.
#[derive(Default)]
pub(crate) struct RoundContent {
    pub calls: crate::tool_calls::ToolCalls,
    pub usage: crate::inference::InferenceUsage,
    pub finish: String,
    filter: ThinkFilter,
    bytes: usize,
}
pub(crate) struct ContentDelta {
    pub answer: String,
    pub reasoning: String,
    pub has_token: bool,
}
impl RoundContent {
    pub fn observe(&mut self, value: &Value) -> Result<ContentDelta, String> {
        self.usage.observe(value);
        if let Some(reason) = value["choices"][0]["finish_reason"].as_str() {
            self.finish = reason.to_string();
        }
        let delta = &value["choices"][0]["delta"];
        self.calls.push(delta)?;
        let raw = delta["content"].as_str().unwrap_or_default();
        let native_reasoning = delta["reasoning_content"]
            .as_str()
            .or_else(|| delta["reasoning"].as_str())
            .unwrap_or_default();
        self.bytes = self
            .bytes
            .saturating_add(raw.len())
            .saturating_add(native_reasoning.len());
        if self.bytes > 4 * 1024 * 1024 {
            return Err("Model output exceeded 4 MiB.".into());
        }
        let (answer, mut reasoning) = self.filter.feed(raw);
        reasoning.push_str(native_reasoning);
        let tool_token = delta["tool_calls"].as_array().is_some_and(|calls| {
            calls.iter().any(|call| {
                ["arguments", "name"].iter().any(|field| {
                    call["function"][*field]
                        .as_str()
                        .is_some_and(|text| !text.is_empty())
                })
            })
        });
        Ok(ContentDelta {
            answer,
            reasoning,
            has_token: !raw.is_empty() || !native_reasoning.is_empty() || tool_token,
        })
    }
    pub fn flush(&mut self) -> (String, String) {
        self.filter.flush()
    }
    pub fn is_thinking(&self) -> bool {
        self.filter.is_thinking()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StreamFailure {
    pub message: String,
    pub retryable: bool,
}
impl StreamFailure {
    fn transient(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
        }
    }
    fn protocol(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
        }
    }
}
impl From<String> for StreamFailure {
    fn from(message: String) -> Self {
        Self::protocol(message)
    }
}
impl From<&str> for StreamFailure {
    fn from(message: &str) -> Self {
        Self::protocol(message)
    }
}

fn transient_provider_error(value: &Value) -> bool {
    let error = &value["error"];
    let codes = [error["code"].as_str(), error["type"].as_str()];
    if codes.iter().flatten().any(|code| {
        matches!(
            *code,
            "insufficient_quota"
                | "invalid_request_error"
                | "authentication_error"
                | "permission_error"
                | "context_length_exceeded"
                | "invalid_api_key"
                | "model_not_found"
                | "unsupported_parameter"
        )
    }) {
        return false;
    }
    codes.iter().flatten().any(|code| {
        matches!(
            *code,
            "rate_limit_exceeded"
                | "rate_limit_error"
                | "overloaded_error"
                | "server_error"
                | "internal_error"
                | "internal_server_error"
                | "timeout"
                | "request_timeout"
                | "temporarily_unavailable"
        )
    })
}
impl From<StreamFailure> for String {
    fn from(error: StreamFailure) -> Self {
        error.message
    }
}
impl std::fmt::Display for StreamFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}
impl std::ops::Deref for StreamFailure {
    type Target = str;
    fn deref(&self) -> &str {
        &self.message
    }
}

/// The same validated provider stream feeds foreground and child sessions.
/// No tool is executable until its native terminal marker has been consumed.
pub struct ProviderRound {
    body: BoxStream<'static, Result<bytes::Bytes, reqwest::Error>>,
    decoder: crate::sse::SseDecoder,
    protocol: crate::sse::ModelStream,
    queued: std::collections::VecDeque<Value>,
}
impl ProviderRound {
    pub fn new(backend: &crate::inference::Backend, response: reqwest::Response) -> Self {
        Self {
            body: response.bytes_stream().boxed(),
            decoder: Default::default(),
            protocol: crate::sse::ModelStream::for_backend(backend),
            queued: Default::default(),
        }
    }
    pub async fn next(&mut self) -> Result<Option<Value>, StreamFailure> {
        loop {
            if let Some(value) = self.queued.pop_front() {
                if transient_provider_error(&value) {
                    // Do not embed raw provider messages: they may echo a key.
                    return Err(StreamFailure::transient(
                        "Provider stream reported a transient failure.",
                    ));
                }
                return Ok(Some(value));
            }
            if self.protocol.done {
                return Ok(None);
            }
            let bytes = tokio::time::timeout(std::time::Duration::from_secs(300), self.body.next())
                .await
                .map_err(|_| StreamFailure::transient("Model stream stopped responding."))?;
            let Some(bytes) = bytes else {
                self.decoder.finish().map_err(StreamFailure::transient)?;
                return Err(StreamFailure::transient(
                    "Model stream ended without a completion marker.",
                ));
            };
            for event in self
                .decoder
                .push(&bytes.map_err(|e| StreamFailure::transient(e.to_string()))?)?
            {
                if let Some(value) = self.protocol.push(&event)? {
                    self.queued.push_back(value);
                }
            }
        }
    }
    pub fn assistant_message(&self, text: &str, calls: &[crate::tool_calls::ToolCall]) -> Value {
        self.protocol.assistant_message(text, calls)
    }
    pub(crate) fn accepted_record(
        &self,
        finish: String,
        text: &str,
        calls: Vec<crate::tool_calls::ToolCall>,
    ) -> Result<ProviderRoundRecord, String> {
        if !self.protocol.done || !matches!(finish.as_str(), "stop" | "tool_calls") {
            return Err("Provider response is incomplete; its tool batch was not accepted.".into());
        }
        crate::arex::validate_batch(&calls)?;
        Ok(ProviderRoundRecord {
            finish,
            assistant_message: self.assistant_message(text, &calls),
            calls,
        })
    }
}

/// The model-facing execution boundary for every host. Provider decoding,
/// failure classification, tool accumulation and reasoning filtering advance
/// together. A host only observes already-interpreted deltas and decides where
/// to persist or display them; it cannot forget to flush a partial marker.
pub(crate) struct ModelRound<'a> {
    provider: ProviderRound,
    backend: &'a crate::inference::Backend,
    pub content: RoundContent,
    flushed: bool,
}
impl<'a> ModelRound<'a> {
    pub fn new(backend: &'a crate::inference::Backend, response: reqwest::Response) -> Self {
        Self {
            provider: ProviderRound::new(backend, response),
            backend,
            content: RoundContent::default(),
            flushed: false,
        }
    }
    pub async fn next(&mut self) -> Result<Option<ContentDelta>, StreamFailure> {
        if self.flushed {
            return Ok(None);
        }
        match self.provider.next().await? {
            Some(value) => {
                if let Some(error) = value.get("error") {
                    return Err(self.backend.stream_error(error).into());
                }
                self.content.observe(&value).map(Some).map_err(Into::into)
            }
            None => {
                self.flushed = true;
                let (answer, reasoning) = self.content.flush();
                Ok(
                    (!answer.is_empty() || !reasoning.is_empty()).then_some(ContentDelta {
                        answer,
                        reasoning,
                        has_token: false,
                    }),
                )
            }
        }
    }
}
impl std::ops::Deref for ModelRound<'_> {
    type Target = ProviderRound;
    fn deref(&self) -> &Self::Target {
        &self.provider
    }
}

/// A process wait is cancelled by its session and woken by the supervisor.
/// It has no implicit wall-clock ceiling. Explicit user tool budgets still apply.
pub fn tool_deadline(
    tool: &crate::connectors::AgentTool,
    args: &Value,
    configured: Option<u64>,
) -> Option<std::time::Duration> {
    if let Some(seconds) = configured.filter(|seconds| *seconds > 0) {
        return Some(
            tool.timeout()
                .min(std::time::Duration::from_secs(seconds.clamp(5, 600))),
        );
    }
    if tool.harness_name() == Some("process_poll") && args.get("wait_ms").is_none() {
        None
    } else {
        Some(tool.timeout())
    }
}
pub async fn await_tool<T>(
    deadline: Option<std::time::Duration>,
    operation: impl std::future::Future<Output = T>,
) -> Result<T, tokio::time::error::Elapsed> {
    match deadline {
        Some(deadline) => tokio::time::timeout(deadline, operation).await,
        None => Ok(operation.await),
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    Completed,
    UserCancelled,
    UserPaused,
    AppRestart,
    ProviderFailure,
    BudgetExhausted,
    PermissionDenied,
    OutcomeUnknown,
    Blocked,
}

impl StopReason {
    pub fn run_state(self) -> crate::agent_run::RunState {
        use crate::agent_run::RunState;
        match self {
            Self::Completed => RunState::Completed,
            Self::UserCancelled => RunState::Cancelled,
            Self::ProviderFailure => RunState::Failed,
            Self::OutcomeUnknown => RunState::OutcomeUnknown,
            _ => RunState::Paused,
        }
    }
}

/// Tool denials and uncertain actions have the same terminal meaning for
/// every host. An unrelated provider error must not erase an unknown action.
pub fn termination_after_tool(current: Option<StopReason>, result: &Value) -> Option<StopReason> {
    if current == Some(StopReason::OutcomeUnknown)
        || matches!(result["status"].as_str(), Some("unknown" | "cancelled"))
        || (crate::tool_envelope::failed(result)
            && (result["error"]["sideEffects"] == crate::tool_envelope::SIDE_UNKNOWN
                || result["metadata"]["sideEffects"] == crate::tool_envelope::SIDE_UNKNOWN))
    {
        Some(StopReason::OutcomeUnknown)
    } else if matches!(
        result["error"]["code"].as_str(),
        Some("denied" | "permission-denied" | "capability-disabled")
    ) {
        Some(StopReason::PermissionDenied)
    } else {
        current
    }
}

/// Bound repeated identical failures in every host without disabling tools.
#[derive(Default)]
pub(crate) struct ActionFailures(std::collections::HashMap<String, (Value, u32)>);
impl ActionFailures {
    pub fn blocked(&self, signature: &str) -> bool {
        self.0.get(signature).is_some_and(|(_, count)| *count >= 2)
    }
    pub fn record(&mut self, signature: &str, result: &Value) -> bool {
        if result["error"]["code"] == "action-blocked" {
            return self.blocked(signature);
        }
        if !crate::tool_envelope::failed(result) {
            self.0.remove(signature);
            return false;
        }
        let error = &result["error"];
        let entry = self.0.entry(signature.into()).or_insert((error.clone(), 0));
        if entry.0 != *error {
            *entry = (error.clone(), 0);
        }
        entry.1 = entry.1.saturating_add(1);
        entry.1 >= 2
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunOutcome {
    pub reason: StopReason,
    pub resumable: bool,
    pub checkpoint: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolExecutionRecord {
    pub call_id: String,
    pub name: Option<String>,
    pub arguments: Value,
    pub result: Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunCheckpoint {
    pub id: String,
    pub request: String,
    pub workspace: String,
    pub operations: Vec<ToolExecutionRecord>,
    pub messages: Vec<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child: Option<crate::subagents::ChildCheckpoint>,
}
impl RunCheckpoint {
    pub fn capture(id: &str, request: &str, workspace: &str, messages: &[Value]) -> Self {
        let mut calls = std::collections::HashMap::new();
        let mut operations = Vec::new();
        for message in messages {
            for call in message["tool_calls"].as_array().into_iter().flatten() {
                if let Some(id) = call["id"].as_str() {
                    calls.insert(
                        id.to_string(),
                        (
                            call["function"]["name"].as_str().map(str::to_string),
                            call["function"]["arguments"].clone(),
                        ),
                    );
                }
            }
            if message["role"] == "tool" {
                let id = message["tool_call_id"].as_str().unwrap_or_default();
                let (name, arguments) = calls.get(id).cloned().unwrap_or((None, Value::Null));
                let result = message["content"]
                    .as_str()
                    .and_then(|text| serde_json::from_str(text).ok())
                    .unwrap_or(Value::Null);
                operations.push(ToolExecutionRecord {
                    call_id: id.into(),
                    name,
                    arguments,
                    result,
                });
            }
        }
        Self {
            id: id.into(),
            request: request.into(),
            workspace: workspace.into(),
            operations,
            messages: messages.to_vec(),
            child: None,
        }
    }
    pub fn resume_note(&self) -> String {
        let operations=self.operations.iter().rev().take(80).map(|op| {
            let mut observation=serde_json::Map::new();
            for key in ["id","processId","state","exitCode","path","sha256","bytesWritten","url","sessionId","documentId","proposalId","stagedTree"] {
                if let Some(value)=op.result["data"].get(key) {observation.insert(key.into(),value.clone());}
            }
            serde_json::json!({"callId":op.call_id,"name":op.name,"status":op.result["status"],"observation":observation,"artifacts":op.result["artifacts"]})
        }).collect::<Vec<_>>();
        format!(
            "Paused checkpoint. {}",
            serde_json::json!({"checkpointId":self.id,"originalRequest":self.request,"workspace":self.workspace,"observedOperations":operations,"pendingWork":"Continue the original request. Review the saved tool history and process handles before further actions. Actions whose outcome is unknown must not be blindly repeated. This snapshot may precede later audit rows; the journal is authoritative."})
        )
    }
}

pub enum RoundControl<T> {
    Continue,
    Complete(T),
}

pub struct Engine {
    round: usize,
    limit: Option<usize>,
}
impl Engine {
    pub fn new(limit: Option<usize>) -> Self {
        Self { round: 0, limit }
    }
    pub fn next_round(&mut self) -> Result<usize, String> {
        if self.limit.is_some_and(|limit| self.round >= limit) {
            return Err(
                "Explicit model round budget exhausted; completed work is preserved.".into(),
            );
        }
        let round = self.round;
        self.round = self
            .round
            .checked_add(1)
            .ok_or("Model round counter overflow.")?;
        Ok(round)
    }
}

/// Context, permission policy, storage and delivery are host adapters. One
/// execution core schedules them; no host owns another model-generation loop.
pub(crate) trait SessionHost: Send {
    type Output;
    fn round(
        &mut self,
        round: usize,
    ) -> futures_util::future::BoxFuture<'_, Result<RoundControl<Self::Output>, String>>;
}

pub(crate) async fn drive_session<H: SessionHost>(
    host: &mut H,
    limit: Option<usize>,
) -> Result<H::Output, String> {
    let mut engine = Engine::new(limit);
    loop {
        match host.round(engine.next_round()?).await? {
            RoundControl::Continue => {}
            RoundControl::Complete(value) => return Ok(value),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failure_guard_is_local_to_the_action_and_identical_error() {
        let fail = |message| {
            crate::tool_envelope::failure_notice(
                "workspace-error",
                message,
                false,
                crate::tool_envelope::SIDE_NONE,
                &serde_json::json!({}),
            )
        };
        let mut guard = ActionFailures::default();
        assert!(!guard.record("create a", &fail("missing parent")));
        assert!(!guard.record("create b", &fail("missing parent")));
        assert!(
            !guard.record("create a", &fail("file exists")),
            "A changed failure starts a new streak"
        );
        assert!(guard.record("create a", &fail("file exists")));
        assert!(guard.blocked("create a"));
        assert!(!guard.blocked("create b"));
        let success = crate::tool_envelope::success(serde_json::json!({"created":true}));
        assert!(!guard.record("create b", &success));
        assert!(
            !guard.record("create b", &fail("missing parent")),
            "A successful action clears its old failures"
        );
    }
    #[tokio::test]
    async fn round_content_separates_reasoning_and_flushes_split_markers_before_acceptance() {
        use serde_json::json;
        let events = [
            json!({"choices":[{"delta":{"content":"<thi"}}]}),
            json!({"choices":[{"delta":{"content":"nk>private</think>Visible ","reasoning_content":"native"}}]}),
            json!({"choices":[{"delta":{"content":"<","tool_calls":[{"index":0,"id":"call-one","function":{"name":"read","arguments":"{\"path\":\"a\"}"}}]},"finish_reason":"tool_calls"}]}),
            json!({"choices":[],"usage":{"prompt_tokens":37,"completion_tokens":11}}),
        ];
        let chunks = events
            .into_iter()
            .map(|event| Ok(bytes::Bytes::from(format!("data: {event}\n\n"))))
            .chain([Ok(bytes::Bytes::from_static(b"data: [DONE]\n\n"))]);
        let provider = ProviderRound {
            body: futures_util::stream::iter(chunks).boxed(),
            decoder: Default::default(),
            protocol: crate::sse::ModelStream::new(false),
            queued: Default::default(),
        };
        let backend = crate::inference::Backend::Local {
                    bonsai_profile: false,
            client: reqwest::Client::new(),
            endpoint: String::new(),
            key: String::new(),
            context_length: 4096,
        };
        let mut stream = ModelRound {
            provider,
            backend: &backend,
            content: RoundContent::default(),
            flushed: false,
        };
        let (mut answer, mut reasoning) = (String::new(), String::new());
        while let Some(delta) = stream.next().await.unwrap() {
            answer.push_str(&delta.answer);
            reasoning.push_str(&delta.reasoning);
        }
        assert!(stream.next().await.unwrap().is_none());
        assert_eq!(answer, "Visible <");
        assert_eq!(reasoning, "privatenative");
        assert!(stream.content.usage.reported());
        let finish = stream.content.finish.clone();
        let calls = std::mem::take(&mut stream.content.calls).finish().unwrap();
        let record = stream.accepted_record(finish, &answer, calls).unwrap();
        assert_eq!(record.calls[0].arguments, json!({"path":"a"}));
        assert_eq!(record.assistant_message["reasoning_content"], "native");
        assert_eq!(record.assistant_message["content"], "Visible <");
    }

    #[test]
    fn uncertain_action_remains_unknown_after_later_success_or_denial() {
        let unknown = crate::tool_envelope::failure_notice(
            "timeout",
            "Unknown remote outcome",
            false,
            crate::tool_envelope::SIDE_UNKNOWN,
            &serde_json::json!({}),
        );
        let denied = crate::tool_envelope::failure_notice(
            "denied",
            "Denied",
            false,
            crate::tool_envelope::SIDE_NONE,
            &serde_json::json!({}),
        );
        let success = crate::tool_envelope::normalize(
            serde_json::json!({"ok":true}),
            crate::tool_envelope::Family::Harness,
        );
        assert_eq!(termination_after_tool(None, &success), None);
        let reason = termination_after_tool(None, &unknown);
        assert_eq!(reason, Some(StopReason::OutcomeUnknown));
        assert_eq!(termination_after_tool(reason, &success), reason);
        assert_eq!(termination_after_tool(reason, &denied), reason);
        assert_eq!(
            termination_after_tool(Some(StopReason::PermissionDenied), &success),
            Some(StopReason::PermissionDenied)
        );
        assert_eq!(
            StopReason::OutcomeUnknown.run_state(),
            crate::agent_run::RunState::OutcomeUnknown
        );
        assert_eq!(
            StopReason::PermissionDenied.run_state(),
            crate::agent_run::RunState::Paused
        );
    }

    #[tokio::test]
    async fn productive_work_continues_beyond_the_old_round_ceiling() {
        let mut completed = Vec::new();
        struct Host<'a>(&'a mut Vec<usize>);
        impl SessionHost for Host<'_> {
            type Output = usize;
            fn round(
                &mut self,
                round: usize,
            ) -> futures_util::future::BoxFuture<'_, Result<RoundControl<usize>, String>>
            {
                Box::pin(async move {
                    if round == 100 {
                        return Ok(RoundControl::Complete(self.0.len()));
                    }
                    self.0.push(round);
                    Ok(RoundControl::Continue)
                })
            }
        }
        let result = drive_session(&mut Host(&mut completed), None).await;
        assert_eq!(result.unwrap(), 100);
        assert_eq!(completed, (0..100).collect::<Vec<_>>());
    }
    #[tokio::test(start_paused = true)]
    async fn active_stream_outlives_three_minutes_and_preserves_native_reasoning() {
        let chunks = [
            r#"{"choices":[{"delta":{"reasoning_content":"working"}}]}"#,
            r#"{"choices":[{"delta":{"content":"still "}}]}"#,
            r#"{"choices":[{"delta":{"content":"active"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            "[DONE]",
        ];
        let body = futures_util::stream::unfold(chunks.into_iter(), |mut chunks| async move {
            let value = chunks.next()?;
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            Some((Ok(bytes::Bytes::from(format!("data: {value}\n\n"))), chunks))
        })
        .boxed();
        let mut stream = ProviderRound {
            body,
            decoder: Default::default(),
            protocol: crate::sse::ModelStream::new(false),
            queued: Default::default(),
        };
        let started = tokio::time::Instant::now();
        while stream.next().await.unwrap().is_some() {}
        assert!(started.elapsed() >= std::time::Duration::from_secs(300));
        assert_eq!(
            stream.assistant_message("still active", &[])["reasoning_content"],
            "working"
        );
    }
    #[tokio::test]
    async fn transient_stream_errors_recover_without_leaking_provider_secrets() {
        for (error, transient) in [
            (
                serde_json::json!({"type":"overloaded_error","message":"private-token"}),
                true,
            ),
            (
                serde_json::json!({"code":"server_error","message":"private-token"}),
                true,
            ),
            (
                serde_json::json!({"type":"server_error","code":"insufficient_quota"}),
                false,
            ),
            (serde_json::json!({"type":"permission_error"}), false),
        ] {
            let bytes =
                bytes::Bytes::from(format!("data: {}\n\n", serde_json::json!({"error":error})));
            let mut stream = ProviderRound {
                body: futures_util::stream::iter([Ok(bytes)]).boxed(),
                decoder: Default::default(),
                protocol: crate::sse::ModelStream::new(false),
                queued: Default::default(),
            };
            let result = stream.next().await;
            if transient {
                let error = result.unwrap_err();
                assert!(error.retryable);
                assert!(!error.message.contains("private-token"));
            } else {
                assert!(
                    result.unwrap().unwrap().get("error").is_some(),
                    "permanent errors still reach backend redaction"
                );
            }
        }
    }

    #[tokio::test]
    async fn truncated_batches_remain_unaccepted_and_malformed_protocol_is_not_transient() {
        for (data, transient) in [
            ("data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"one\",\"function\":{\"name\":\"write\",\"arguments\":\"{}\"}}]}}]}\n\n",true),
            ("data: {\"choices\":",true),
            ("data: {bad-json}\n\n",false),
        ] {
            let mut stream = ProviderRound { body:futures_util::stream::iter([Ok(bytes::Bytes::from(data))]).boxed(), decoder:Default::default(), protocol:crate::sse::ModelStream::new(false), queued:Default::default() };
            let error = loop { match stream.next().await { Ok(Some(_))=>{}, Ok(None)=>panic!("unfinished stream accepted"), Err(error)=>break error } };
            assert_eq!(error.retryable, transient, "{error}");
            assert!(!stream.protocol.done);
        }
    }
    #[test]
    fn only_an_explicit_limit_stops_productive_work() {
        let mut engine = Engine::new(Some(2));
        assert_eq!(engine.next_round().unwrap(), 0);
        assert_eq!(engine.next_round().unwrap(), 1);
        assert!(engine.next_round().is_err());
    }

    #[tokio::test]
    async fn shared_driver_respects_budget_and_returns_host_blockers_without_replaying_work() {
        struct Host {
            committed: Vec<usize>,
            blocker: Option<usize>,
        }
        impl SessionHost for Host {
            type Output = ();
            fn round(
                &mut self,
                round: usize,
            ) -> futures_util::future::BoxFuture<'_, Result<RoundControl<()>, String>> {
                Box::pin(async move {
                    if self.blocker == Some(round) {
                        return Err(
                            "Waiting for permission; completed actions stay committed.".into()
                        );
                    }
                    self.committed.push(round);
                    Ok(RoundControl::Continue)
                })
            }
        }
        let mut limited = Host {
            committed: vec![],
            blocker: None,
        };
        assert!(drive_session(&mut limited, Some(3))
            .await
            .unwrap_err()
            .contains("budget exhausted"));
        assert_eq!(limited.committed, vec![0, 1, 2]);
        let mut blocked = Host {
            committed: vec![],
            blocker: Some(2),
        };
        assert!(drive_session(&mut blocked, None)
            .await
            .unwrap_err()
            .contains("Waiting for permission"));
        assert_eq!(blocked.committed, vec![0, 1]);
    }
}
