use crate::{inference::{Backend, InferenceProvider}, sse::SseDecoder, AppState};
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tauri::{ipc::Channel, State};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatEvent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seq: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub activity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Value>,
    pub message_id: String,
    pub content: String,
    pub reasoning: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approval: Option<Value>,
}

impl ChatEvent {
    pub fn progress(
        run_id: &str,
        step_id: &str,
        message_id: &str,
        activity: &str,
        approval: Option<Value>,
    ) -> Self {
        // Channel-only progress emission (durable log comes from audit rows).
        Self {
            run_id: Some(run_id.to_string()),
            step_id: Some(step_id.to_string()),
            seq: Some(0),
            state: None,
            activity: Some(activity.to_string()),
            context: None,
            message_id: message_id.to_string(),
            content: String::new(),
            reasoning: String::new(),
            approval,
        }
    }
    pub fn new(
        run: &crate::agent_run::RunRecord,
        step_id: Option<&str>,
        seq: u32,
        activity: &str,
        message_id: &str,
        content: &str,
        reasoning: &str,
        context: Option<Value>,
        approval: Option<Value>,
    ) -> Self {
        Self {
            run_id: Some(run.id.clone()),
            step_id: step_id.map(String::from),
            seq: Some(seq),
            state: Some(run.status.as_str().to_string()),
            activity: Some(activity.to_string()),
            context,
            message_id: message_id.to_string(),
            content: content.to_string(),
            reasoning: reasoning.to_string(),
            approval,
        }
    }
}

#[tauri::command]
pub fn cancel_generation(state: State<'_, AppState>) {
    state.cancel.send_replace(true);
}

#[tauri::command]
pub async fn send_message(
    state: State<'_, AppState>,
    conversation_id: String,
    content: String,
    connector_ids: Option<Vec<String>>,
    connector_tools: Option<Vec<crate::connectors::ToolSelection>>,
    channel: Channel<ChatEvent>,
) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Another model operation is in progress.")?;
    if content.trim().is_empty() || content.len() > 1_048_576 {
        return Err("Enter a message no larger than 1 MiB.".into());
    }
    let mut connector_ids = connector_ids.unwrap_or_default();
    // Runtime preset intersects the requested sources (config patch, no fork).
    let preset = {
        let store = state.database()?;
        let id = store.conversation_preset(&conversation_id).unwrap_or_else(|_| crate::presets::STANDARD.to_string());
        crate::presets::get(&id)?
    };
    let use_daytona = connector_ids.iter().any(|id| id == "__daytona") && preset.sources.iter().any(|source| source == "__daytona");
    connector_ids.retain(|id| id != "__daytona");
    let use_workspace = connector_ids.iter().any(|id| id == "__workspace") && preset.sources.iter().any(|source| source == "__workspace");
    let use_execution = connector_ids.iter().any(|id| id == "__execution") && preset.sources.iter().any(|source| source == "__execution");
    connector_ids.retain(|id| id != "__execution");
    connector_ids.retain(|id| id != "__workspace");
    if !preset.mcp {
        connector_ids.clear();
    }
    let connector_tools = if preset.mcp { connector_tools.unwrap_or_default() } else { Vec::new() };
    let mut tools = state
        .connectors
        .lock()
        .await
        .selected_tools(&connector_ids, &connector_tools)?;
    if use_workspace {
        let path = state.database()?.workspace_path()?;
        let mut workspace_tools = std::sync::Arc::new(crate::workspace::Workspace::open(&path)?).tools();
        if preset.id == crate::presets::MINIMAL {
            // Minimal benchmark surface: editing only.
            workspace_tools.retain(|tool| crate::presets::minimal_workspace_tools().contains(&tool.tool.name.as_str()));
        }
        tools.extend(workspace_tools);
    }
    if use_daytona {
        let bytes = state
            .daytona_vault
            .load("daytona")?
            .ok_or("Save a Daytona API key in Execution first.")?;
        let key = String::from_utf8(bytes).map_err(|_| "Saved Daytona key is invalid.")?;
        tools.push(crate::connectors::AgentTool::daytona(
            crate::daytona_execution::Executor::new(
                &key,
                state.daytona_journal.clone(),
                state.daytona_operation.clone(),
            )?,
        ));
    }
    if use_execution {
        let (config, path) = {
            let store = state.database()?;
            (store.execution_config()?, store.workspace_path()?)
        };
        tools.push(
            std::sync::Arc::new(crate::execution::LocalExecution::new(config, &path)?).tool(),
        );
    }
    let active_skills = if preset.skills { state.database()?.active_skills()? } else { Vec::new() };
    let skill_instructions = {
        let skills = state.skills.lock().await;
        let instructions = skills.instructions(&active_skills)?;
        if preset.skills {
            if let Some(reader) = skills.reader(&active_skills)? {
                tools.push(reader);
            }
        }
        instructions
    };
    for alias in &preset.harness {
        if !tools.iter().any(|tool| &tool.alias == alias) {
            tools.push(crate::connectors::AgentTool::harness(alias)?);
        }
    }
    if tools.len() > 64 {
        return Err("Select fewer tool sources: at most 64 tools can be offered in a turn.".into());
    }
    let (selection, selection_required) = state.database()?.conversation_model(&conversation_id)?;
    if selection_required {
        return Err("This conversation needs another provider selection before it can continue. Its history is preserved.".into());
    }
    let backend = if let Some(provider_id) = selection.provider_id.clone() {
        let provider = state.database()?.provider(&provider_id)?.ok_or("The selected provider was deleted. Choose another provider before sending a message.")?;
        if !provider.verified {
            return Err("Test the selected provider connection successfully before sending a message.".into());
        }
        let model = provider.models.iter().find(|model| model.id == selection.model_id).ok_or("Configure the selected remote model's context capacity before sending a message.")?;
        let key = state.daytona_vault.load(&crate::providers::credential_id(&provider_id))?.ok_or("Save an API key for the selected provider before sending a message.")?;
        let key = String::from_utf8(key).map_err(|_| "Saved provider API key is invalid.")?;
        match provider.api_format.as_str() {
            crate::providers::OPENAI_CHAT_COMPLETIONS => Backend::openai(&provider, key, model)?,
            crate::providers::ANTHROPIC_MESSAGES => Backend::anthropic(&provider, key, model)?,
            _ => return Err("This provider format is not supported by the current chat adapter.".into()),
        }
    } else {
        let mut runtime = state.runtime.lock().await;
        if runtime.inspect().phase != "ready" {
            return Err("Load a local model before sending a message, or choose a tested API provider.".into());
        }
        Backend::local(runtime.endpoint.clone(), runtime.api_key.clone(), runtime.context_length)?
    };
    let context_length = backend.context_capacity();
    let access_mode = state
        .database()?
        .conversation_tools(&conversation_id)?
        .access_mode;
    // Logged turn injections: everything model-visible below is recorded as a
    // context_injection run event after the run row exists.
    let mut injections: Vec<(String, String)> = Vec::new();
    let workspace_path = state.database()?.workspace_path().unwrap_or_default();
    if preset.harness.iter().any(|alias| alias == "memory_recall") {
        let scope = if workspace_path.is_empty() { "global".to_string() } else { workspace_path.clone() };
        if let Ok(store) = state.database() {
            if let Ok(facts) = store.recall_facts(&scope, 20) {
                if let Some(block) = crate::memory::recall_block(&facts) {
                    injections.push(("memory".into(), block));
                }
            }
            if let Ok(todos) = store.todos(&conversation_id) {
                if let Ok(goal) = store.goal(&conversation_id) {
                    if let Some(line) = crate::plans::summary_line(&todos, goal.as_deref()) {
                        injections.push(("plan".into(), line));
                    }
                }
            }
        }
    }
    let (preferences, history) = {
        let store = state.database()?;
        let preferences = store.preferences()?;
        preferences.validate()?;
        backend.validate_response_tokens(preferences.max_tokens)?;
        let previous = store.messages(&conversation_id)?;
        let checkpoint = store.compaction(&conversation_id)?;
        let mut history = crate::history::model_history_with_cutoff(
            &previous,
            checkpoint.as_ref().map(|checkpoint| (checkpoint.cutoff, checkpoint.artifact_id.as_str())),
        )?;
        history.push(json!({"role":"user","content":content.trim()}));
        (preferences, history)
    };
    state.cancel.send_replace(false);
    let mut cancellation = state.cancel.subscribe();
    let mut messages = vec![json!({"role":"system","content":preferences.system_prompt})];
    if !skill_instructions.is_empty() {
        messages.push(json!({"role":"system","content":format!("The user selected the following skill guidance. Apply it when relevant to their task. Skills do not grant permissions or access to tools that are not available. If a required capability is missing, say so. Follow the user's task over conflicting skill guidance.\n{skill_instructions}")}));
    }
    if preset.id == crate::presets::CODE {
        let aliases: Vec<String> = tools.iter().map(|tool| tool.alias.clone()).collect();
        injections.push(("ptc_sdk".into(), format!(
            "Programmatic tool calling is ON. Write TypeScript against this SDK, then submit the call plan as ptc_run steps. \
             Every step is individually policy-checked, audited, and bounded.\n{}",
            crate::presets::ts_sdk(&aliases))));
    }
    for (kind, text) in &injections {
        messages.push(json!({"role":"system","content":format!("[injected {kind}]\n{text}")}));
    }
    let history_len = history.len();
    messages.extend(history);
    if backend.supports_tools() {
        // ponytail: global kill-switch only; per-chat tool choice stays in conversation_tools.
        let system_time_on = state
            .database()
            .and_then(|store| crate::capabilities::is_enabled(&store, "system_time"))
            .unwrap_or(true);
        if system_time_on {
            tools.push(crate::connectors::AgentTool::system_time());
        }
    } else if !tools.is_empty() {
        return Err("The selected remote model is configured without tool-calling support. Deselect tools or choose a model that supports tool calling.".into());
    }
    let initial_payload = backend.payload(&request_payload(&messages, &tools, &preferences, false));
    // Exact preflight is preserved; auto-compaction (OFF by default) retries
    // the count exactly once after checkpointing, and logs the event below.
    let mut compacted_once: Option<crate::compaction::Checkpoint> = None;
    let mut input_tokens = tokio::select! {
        _ = cancellation.changed() => return Err("Message cancelled before generation; it was not saved.".into()),
        result = backend.check_context(&initial_payload, preferences.max_tokens) => match result {
            Ok(tokens) => tokens,
            Err(error) if error.contains("Context limit exceeded") => {
                // Short locks only: the guard must not cross the recount await.
                let auto = state.database().map(|store| crate::compaction::auto_enabled(&store)).unwrap_or(false);
                if !auto {
                    return Err(error);
                }
                let checkpoint = {
                    let store = state.database()?;
                    crate::compaction::compact_now(&store, &conversation_id, crate::compaction::keep_last(&store))?
                };
                messages.truncate(messages.len() - history_len);
                let previous = state.database()?.messages(&conversation_id)?;
                let mut fresh = crate::history::model_history_with_cutoff(
                    &previous,
                    Some((checkpoint.cutoff, checkpoint.artifact_id.as_str())),
                )?;
                fresh.push(json!({"role":"user","content":content.trim()}));
                messages.extend(fresh);
                let retry_payload = backend.payload(&request_payload(&messages, &tools, &preferences, false));
                let retry = tokio::select! {
                    _ = cancellation.changed() => return Err("Message cancelled before generation; it was not saved.".into()),
                    result = backend.check_context(&retry_payload, preferences.max_tokens) => result?,
                };
                compacted_once = Some(checkpoint);
                retry
            }
            Err(error) => return Err(error),
        },
    };
    let assistant = state.database()?.begin_turn(&conversation_id, &content)?;
    let run_id = format!("run-{}", uuid::Uuid::new_v4());
    let now = crate::store::now();
    let mut run = crate::agent_run::RunRecord {
        id: run_id.clone(),
        conversation_id: conversation_id.clone(),
        status: crate::agent_run::RunState::Preparing,
        model_provider: selection.provider_id.clone(),
        model_id: Some(selection.model_id.clone()),
        checkpoint: None,
        error: None,
        created_at: now,
        updated_at: now,
    };
    state.database()?.save_run(&run)?;
    let mut seq: u32 = 0;
    // Model-visible ⟺ logged: every injected context block lands in the log.
    for (kind, text) in &injections {
        seq += 1;
        let _ = state.database()?.append_run_event(&crate::agent_run::RunEvent {
            run_id: run.id.clone(),
            seq: seq as u64,
            step_id: "turn_start".into(),
            tool_call_id: None,
            event_type: "context_injection".into(),
            payload: json!({"kind": kind, "chars": text.len()}),
            created_at: crate::store::now(),
        });
    }
    if let Some(checkpoint) = &compacted_once {
        seq += 1;
        let _ = state.database()?.append_run_event(&crate::agent_run::RunEvent {
            run_id: run.id.clone(),
            seq: seq as u64,
            step_id: "turn_start".into(),
            tool_call_id: None,
            event_type: "compaction".into(),
            payload: json!({"artifact": checkpoint.artifact_id, "cutoff": checkpoint.cutoff, "auto": true}),
            created_at: crate::store::now(),
        });
    }
    // Snapshot once for harness children (subagents, schedules, workflows).
    let snapshot = crate::subagents::snapshot_for_conversation(&state, &conversation_id).await?;
    let preset_id = preset.id.clone();
    let mut answer = String::new();
    let mut reasoning = String::new();
    let result: Result<bool,String> = async {
        seq += 1;
        channel.send(ChatEvent::new(
            &run,
            None,
            seq,
            "Preparing turn",
            &assistant.id,
            "",
            "",
            None,
            None,
        )).map_err(|error| error.to_string())?;

        let mut tool_use_denied = false;
        // Loop-hygiene streaks: consecutive identical calls, guarded centrally.
        let mut recent_calls: Vec<(String, String)> = Vec::new();
        for round in 0..9 {
            if *cancellation.borrow() { return Ok(false); }
            let step_id = format!("step-{round}");
            let _ = run.transition_to(crate::agent_run::RunState::Generating);
            let _ = state.database()?.update_run_status(&run.id, run.status, None, Some(&step_id));
            seq += 1;
            let _ = state.database()?.append_run_event(&crate::agent_run::RunEvent {
                run_id: run.id.clone(),
                seq: seq as u64,
                step_id: step_id.clone(),
                tool_call_id: None,
                event_type: "step_start".into(),
                payload: json!({"round": round}),
                created_at: crate::store::now(),
            });

            let payload = backend.payload(&request_payload(&messages, &tools, &preferences, tool_use_denied));
            if round > 0 {
                input_tokens = tokio::select! {
                    _ = cancellation.changed() => return Ok(false),
                    result = backend.check_context(&payload, preferences.max_tokens) => result?,
                };
            }
            seq += 1;
            channel.send(ChatEvent::new(
                &run,
                Some(&step_id),
                seq,
                "Generating response…",
                &assistant.id,
                "",
                "",
                Some(json!({"inputTokens":input_tokens,"responseReserve":preferences.max_tokens,"contextLength":context_length,"estimated":backend.context_is_estimate()})),
                None,
            )).map_err(|error| error.to_string())?;

            let response = tokio::select! {
                _ = cancellation.changed() => return Ok(false),
                result = tokio::time::timeout(Duration::from_secs(180), backend.stream(&payload)) => result.map_err(|_| "Model did not respond within three minutes.")??,
            };
            let mut stream = response;
            let mut decoder = SseDecoder::default();
            let mut checkpoint = Instant::now();
            let mut calls = crate::tool_calls::ToolCalls::default();
            let mut round_answer = String::new();
            let mut finish_reason = String::new();
            'stream: loop {
                let next = tokio::select! {
                    _ = cancellation.changed() => return Ok(false),
                    next = tokio::time::timeout(Duration::from_secs(180), stream.next()) => next.map_err(|_| "Model stopped responding.")?,
                };
                let Some(bytes) = next else { decoder.finish()?; return Err("Model stream ended without a completion marker.".into()); };
                for event in decoder.push(&bytes?)? {
                    if event == "[DONE]" { break 'stream; }
                    let value: Value = serde_json::from_str(&event).map_err(|error| format!("Invalid model stream: {error}"))?;
                    if let Some(error) = value.get("error") { return Err(backend.stream_error(error)); }
                    if let Some(reason) = value["choices"][0]["finish_reason"].as_str() { finish_reason = reason.to_string(); }
                    let delta = &value["choices"][0]["delta"];
                    calls.push(delta)?;
                    let text = delta["content"].as_str().unwrap_or("");
                    let thought = delta["reasoning_content"].as_str().or_else(|| delta["reasoning"].as_str()).unwrap_or("");
                    answer.push_str(text); round_answer.push_str(text); reasoning.push_str(thought);
                    if answer.len() + reasoning.len() > 4_194_304 { return Err("Model output exceeded 4 MiB.".into()); }
                    if !text.is_empty() || !thought.is_empty() {
                        seq += 1;
                        channel.send(ChatEvent::new(
                            &run,
                            Some(&step_id),
                            seq,
                            "Streaming response…",
                            &assistant.id,
                            text,
                            thought,
                            None,
                            None,
                        )).map_err(|error| error.to_string())?;
                    }
                    if checkpoint.elapsed() > Duration::from_millis(500) {
                        state.database()?.update_message(&assistant.id,&answer,&reasoning,"streaming")?;
                        checkpoint = Instant::now();
                    }
                }
            }
            check_finish_reason(&finish_reason)?;
            let calls = calls.finish()?;
            if calls.is_empty() {
                let _ = run.transition_to(crate::agent_run::RunState::Completed);
                let _ = state.database()?.update_run_status(&run.id, run.status, None, Some(&step_id));
                return Ok(true);
            }
            if tool_use_denied { return Err("Tool use stopped after your denial. Send a new message to authorize further actions.".into()); }
            if round >= 8 { return Err("Tool round limit reached. Review the results before continuing.".into()); }
            messages.push(json!({"role":"assistant","content":round_answer,"tool_calls":calls.iter().map(|call| call.model_value()).collect::<Vec<_>>() }));
            for call in calls {
                let tool = tools.iter().find(|tool| tool.alias == call.name).ok_or("The model requested a tool that was not selected.")?;
                if *cancellation.borrow() { return Ok(false); }
                let automatic_reason = access_mode.automatic_reason(tool.trusted_read());
                let blocked_by_denial = tool_use_denied;
                let allow = if blocked_by_denial { false } else if automatic_reason.is_some() { true } else {
                    let _ = run.transition_to(crate::agent_run::RunState::AwaitingApproval);
                    let _ = state.database()?.update_run_status(&run.id, run.status, None, Some(&step_id));
                    let (approval_id, decision) = state.approvals.request()?;
                    seq += 1;
                    let sent = channel.send(ChatEvent::new(
                        &run,
                        Some(&step_id),
                        seq,
                        &format!("Awaiting approval: {}", tool.tool.name),
                        &assistant.id,
                        "",
                        "",
                        None,
                        Some(json!({"id":approval_id,"connector":tool.connector,"localServerName":tool.local_server_name(),"name":tool.tool.name,"arguments":call.arguments})),
                    ));
                    if sent.is_err() { state.approvals.remove(&approval_id); return Err("The approval interface disconnected.".into()); }
                    let allow = tokio::select! {
                        _ = cancellation.changed() => None,
                        result = tokio::time::timeout(Duration::from_secs(600),decision) => Some(result.ok().and_then(Result::ok).unwrap_or(false)),
                    };
                    state.approvals.remove(&approval_id);
                    seq += 1;
                    channel.send(ChatEvent::new(
                        &run,
                        Some(&step_id),
                        seq,
                        "Approval decision recorded",
                        &assistant.id,
                        "",
                        "",
                        None,
                        Some(Value::Null),
                    )).map_err(|error| error.to_string())?;
                    let Some(allow) = allow else { return Ok(false); };
                    allow
                };
                if !allow { tool_use_denied = true; }
                let authorization = if blocked_by_denial { "blocked by an earlier denial in this turn" } else { automatic_reason.unwrap_or("user approval decision") };
                let audit = json!({"connector":tool.connector,"localServerName":tool.local_server_name(),"name":tool.tool.name,"arguments":call.arguments,"decision":if allow { "allowed" } else { "denied" },"accessMode":access_mode,"authorization":authorization});
                let row = state.database()?.append_message(&conversation_id,"tool",&audit.to_string(),if allow { "streaming" } else { "complete" })?;

                let _ = run.transition_to(crate::agent_run::RunState::ExecutingTools);
                let _ = state.database()?.update_run_status(&run.id, run.status, None, Some(&step_id));
                seq += 1;
                channel.send(ChatEvent::new(
                    &run,
                    Some(&step_id),
                    seq,
                    &format!("Executing {}", tool.tool.name),
                    &assistant.id,
                    "",
                    "",
                    None,
                    None,
                )).map_err(|error| error.to_string())?;

                let result = if !allow { json!({"isError":true,"message":"The user denied this tool request. Do not repeat it without a new instruction."}) } else {
                    let args_text = call.arguments.to_string();
                    // Guard: three identical calls in a row end loudly, not in a loop.
                    if let Some(note) = crate::sandbox::check_repetition(&recent_calls, &call.name, &args_text) {
                        recent_calls.push((call.name.clone(), args_text));
                        json!({"isError":true,"message":note})
                    } else {
                        recent_calls.push((call.name.clone(), args_text));
                        let timeout_secs = state
                            .database()
                            .map(|store| crate::sandbox::effective_timeout_secs(&store, tool.timeout().as_secs()))
                            .unwrap_or(tool.timeout().as_secs());
                        let app_state: &AppState = &state;
                        let mut pending_events: Vec<crate::harness::PendingEvent> = Vec::new();
                        let outcome = tokio::select! {
                            _ = cancellation.changed() => None,
                            result = tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), async {
                                if crate::harness::is_harness_tool(&call.name) {
                                    let emit_run = run.id.clone();
                                    let emit_step = step_id.clone();
                                    let emit_message = assistant.id.clone();
                                    let emit_channel = channel.clone();
                                    let emit: std::sync::Arc<dyn Fn(String, Option<Value>) + Send + Sync> =
                                        std::sync::Arc::new(move |activity, approval| {
                                            // Channel-only progress (seq 0); durability comes from audit rows.
                                            let _ = emit_channel.send(ChatEvent::progress(
                                                &emit_run,
                                                &emit_step,
                                                &emit_message,
                                                &activity,
                                                approval,
                                            ));
                                        });
                                    let ctx = crate::harness::HarnessCtx {
                                        state: app_state,
                                        snapshot: snapshot.clone(),
                                        conversation_id: conversation_id.clone(),
                                        run_id: run.id.clone(),
                                        step_id: step_id.clone(),
                                        access_mode,
                                        depth: 0,
                                        preset_id: preset_id.clone(),
                                        inherit_tools: &tools,
                                        emit,
                                    };
                                    match crate::harness::execute(&ctx, &call.name, call.arguments.clone()).await {
                                        Ok(outcome) => {
                                            pending_events.extend(outcome.events);
                                            Ok(outcome.value)
                                        }
                                        Err(error) => Err(error),
                                    }
                                } else {
                                    tool.call(call.arguments.clone()).await
                                }
                            }) => Some(result.unwrap_or_else(|_| Err("Tool request timed out. Its remote outcome may be unknown; do not automatically retry.".into()))),
                        };
                        for event in pending_events.drain(..) {
                            seq += 1;
                            let _ = state.database()?.append_run_event(&crate::agent_run::RunEvent {
                                run_id: run.id.clone(),
                                seq: seq as u64,
                                step_id: step_id.clone(),
                                tool_call_id: Some(call.id.clone()),
                                event_type: event.event_type,
                                payload: event.payload,
                                created_at: crate::store::now(),
                            });
                        }
                        match outcome {
                            None => {
                                let _ = run.transition_to(crate::agent_run::RunState::Cancelled);
                                let _ = state.database()?.update_run_status(&run.id, run.status, Some("Cancelled; the action may already have changed data."), Some(&step_id));
                                state.database()?.update_message(&row.id,&json!({"request":audit,"result":"Cancelled; the action may already have changed data."}).to_string(),"","interrupted")?;
                                return Ok(false);
                            },
                            Some(Ok(value)) => value,
                            Some(Err(error)) => json!({"isError":true,"message":error}),
                        }
                    }
                };

                let (bounded_val, maybe_artifact) = crate::artifacts::bound_tool_result(
                    result,
                    &tool.tool.name,
                    &conversation_id,
                    Some(&run.id),
                    crate::artifacts::DEFAULT_MAX_RESULT_CHARS,
                );
                if let Some(artifact) = &maybe_artifact {
                    let _ = state.database()?.save_artifact(artifact);
                }
                state.database()?.update_message(&row.id,&json!({"request":audit,"result":bounded_val}).to_string(),"","complete")?;
                messages.push(json!({"role":"tool","tool_call_id":call.id,"content":bounded_val.to_string()}));
            }
            let _ = run.transition_to(crate::agent_run::RunState::PreparingNextRound);
            let _ = state.database()?.update_run_status(&run.id, run.status, None, Some(&step_id));
        }
        Err("Tool round limit reached.".into())
    }.await;
    let status = match &result {
        Ok(true) => "complete",
        Ok(false) => "interrupted",
        Err(_) => "error",
    };
    state.database()?.finish_message(
        &assistant.id,
        &answer,
        &reasoning,
        status,
        result.as_ref().err().map(String::as_str),
    )?;
    let final_run_state = match &result {
        Ok(true) => crate::agent_run::RunState::Completed,
        Ok(false) => crate::agent_run::RunState::Cancelled,
        Err(_) => crate::agent_run::RunState::Failed,
    };
    let _ = run.transition_to(final_run_state);
    let _ = state.database()?.update_run_status(
        &run.id,
        run.status,
        result.as_ref().err().map(String::as_str),
        None,
    );
    result.map(|_| ())
}

fn check_finish_reason(reason: &str) -> Result<(), String> {
    match reason {
        "length" => Err("Response token limit reached before the model finished. Increase Maximum response tokens in Models & runtime, or ask for a shorter response. Any partial output has been saved; unfinished tool calls were not executed.".into()),
        "content_filter" => Err("The model runtime stopped this response because of its content filter.".into()),
        _ => Ok(()),
    }
}

fn request_payload(
    messages: &[Value],
    tools: &[crate::connectors::AgentTool],
    preferences: &crate::store::Preferences,
    denied: bool,
) -> Value {
    let mut payload = json!({"messages":messages,"temperature":preferences.temperature,"top_p":preferences.top_p,"max_tokens":preferences.max_tokens,"stream":true,"cache_prompt":true});
    if !tools.is_empty() {
        payload["tools"] = json!(tools
            .iter()
            .map(|tool| tool.definition())
            .collect::<Vec<_>>());
    }
    if denied {
        payload["tool_choice"] = json!("none");
    }
    payload
}

#[cfg(test)]
mod finish_tests {
    #[test]
    fn token_exhaustion_is_not_treated_as_success_or_executable_tool_output() {
        assert!(super::check_finish_reason("length")
            .unwrap_err()
            .contains("unfinished tool calls were not executed"));
        assert!(super::check_finish_reason("stop").is_ok());
        assert!(super::check_finish_reason("tool_calls").is_ok());
        assert!(super::check_finish_reason("content_filter").is_err());
    }
}
