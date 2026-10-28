//! Foreground and detached children use the same round and dispatch protocol.
//! A host supplies storage, approval authority and the available tool backend.
use super::*;
use serde_json::{json, Value};

pub(super) enum Host<'a> {
    Foreground {
        state: &'a crate::AppState,
        emit: Option<ChildEmit>,
    },
    Detached {
        store: Arc<Mutex<crate::store::Store>>,
        processes: crate::process_tools::ProcessRegistry,
    },
}
impl Host<'_> {
    fn checkpoint(
        &self,
        id: &str,
        request: &ChildRequest,
        snapshot: &BackendSnapshot,
        tools: &std::collections::BTreeMap<String, String>,
        messages: &[Value],
    ) -> Result<(), String> {
        self.with_store(|store| {
            let mut checkpoint = crate::agent_engine::RunCheckpoint::capture(
                id,
                &request.prompt,
                &snapshot.workspace_path,
                messages,
            );
            checkpoint.child = Some(ChildCheckpoint {
                request: request.clone(),
                tools: tools.clone(),
                model_id: snapshot.model_id.clone(),
                provider_kind: snapshot.kind.clone(),
                api_format: snapshot.api_format.clone(),
                journal_seq: store.latest_run_event_seq(id)?,
            });
            store.save_run_checkpoint(&checkpoint)
        })
    }
    fn with_store<T>(
        &self,
        f: impl FnOnce(&crate::store::Store) -> Result<T, String>,
    ) -> Result<T, String> {
        match self {
            Self::Foreground { state, .. } => {
                let guard = state.database()?;
                f(&guard)
            }
            Self::Detached { store, .. } => {
                let guard = store.lock().map_err(|_| "Store lock poisoned")?;
                f(&guard)
            }
        }
    }
    fn policy(&self) -> crate::capabilities::ToolPolicy {
        self.with_store(|store| Ok(crate::capabilities::ToolPolicy::resolve(store)))
            .unwrap_or_else(crate::capabilities::ToolPolicy::closed)
    }
    fn interrupted(
        &self,
        id: &str,
        round: usize,
        call: &crate::tool_calls::ToolCall,
        seq: &mut u64,
    ) -> String {
        match self {
            Self::Foreground { state, .. } => cancelled_tool_call(state, id, round, call, seq),
            Self::Detached { store, .. } => {
                cancelled_tool_call_detached(store, id, round, call, seq)
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    async fn dispatch(
        &self,
        snapshot: &BackendSnapshot,
        request: &ChildRequest,
        tools: &[&crate::connectors::AgentTool],
        id: &str,
        round: usize,
        tool: &crate::connectors::AgentTool,
        call: &crate::tool_calls::ToolCall,
    ) -> Value {
        match self {
            Self::Foreground { state, emit } => {
                call_child_tool(
                    state,
                    snapshot,
                    request,
                    tools,
                    id,
                    round,
                    tool,
                    call,
                    emit.as_ref(),
                )
                .await
            }
            Self::Detached { processes, .. } => {
                let result = if matches!(
                    tool.harness_name(),
                    Some(
                        "process_start"
                            | "process_poll"
                            | "process_write"
                            | "process_stop"
                            | "shell_run"
                    )
                ) {
                    processes
                        .execute(
                            &request.conversation_id,
                            &snapshot.workspace_path,
                            snapshot.execution_config.clone(),
                            tool.harness_name().unwrap(),
                            call.arguments.clone(),
                        )
                        .await
                } else if tool.harness_name() == Some(crate::tool_discovery::SEARCH) {
                    crate::tool_discovery::search(
                        tools.iter().copied(),
                        &call.arguments,
                        &self.policy(),
                    )
                } else if let Some(alias) = tool.harness_name() {
                    harness_detached(snapshot, request, alias, call.arguments.clone()).await
                } else {
                    let configured = self
                        .with_store(|store| store.setting(crate::sandbox::TOOL_TIMEOUT_KEY))
                        .unwrap_or_default();
                    match crate::agent_engine::await_tool(crate::agent_engine::tool_deadline(tool,&call.arguments,configured),tool.call_in_process_session(call.arguments.clone(),processes,&request.conversation_id)).await {
                        Ok(result)=>result,
                        Err(_)=>return crate::tool_envelope::normalize_error("Subagent tool call timed out. Its outcome may be unknown; do not automatically retry.",crate::tool_envelope::Cause{timeout:true,..Default::default()},tool.envelope_family()),
                    }
                };
                match result {
                    Ok(value) => crate::tool_envelope::normalize(value, tool.envelope_family()),
                    Err(error) => crate::tool_envelope::normalize_error(
                        &error,
                        Default::default(),
                        tool.envelope_family(),
                    ),
                }
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    async fn call(
        &self,
        snapshot: &BackendSnapshot,
        request: &ChildRequest,
        tools: &[&crate::connectors::AgentTool],
        id: &str,
        round: usize,
        tool: &crate::connectors::AgentTool,
        call: &crate::tool_calls::ToolCall,
        cancel: &mut ChildCancel,
        seq: &mut u64,
    ) -> Result<Value, String> {
        let mut allowed =
            request.policy.allows(tool.trusted_read()) && !tool.requires_approval(&call.arguments);
        if !allowed {
            if let Self::Foreground {
                state,
                emit: Some(emit),
            } = self
            {
                allowed = tokio::select! {
                    _=cancel.cancelled()=>{emit("Approval withdrawn (subagent interrupted)".into(),Some(Value::Null));return Err(INTERRUPTED.into());},
                    decision=request_approval_for_child(state,request,id,Some(emit),call)=>decision==Some(true),
                };
            }
        }
        if !allowed {
            return Ok(crate::tool_envelope::failure_notice("denied","Denied by the subagent approval pin (reads-only). Ask the parent for what you need instead.",false,crate::tool_envelope::SIDE_NONE,&json!({"denied":true})));
        }
        tokio::select! {
            _=cancel.cancelled()=>Err(self.interrupted(id,round,call,seq)),
            result=self.dispatch(snapshot,request,tools,id,round,tool,call)=>Ok(result),
        }
    }
    fn event(
        &self,
        id: &str,
        round: usize,
        call: Option<&str>,
        kind: &str,
        payload: Value,
    ) -> Result<(), String> {
        self.with_store(|store| {
            store
                .append_run_event(&crate::agent_run::RunEvent {
                    run_id: id.into(),
                    seq: 0,
                    step_id: format!("round-{round}"),
                    tool_call_id: call.map(str::to_string),
                    event_type: kind.into(),
                    payload,
                    created_at: crate::store::now(),
                })
                .map(|_| ())
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn drive(
    host: Host<'_>,
    snapshot: &BackendSnapshot,
    request: &ChildRequest,
    mut tools: Vec<&crate::connectors::AgentTool>,
    backend: &crate::inference::Backend,
    mut cancel: ChildCancel,
    registry: Option<Arc<SubagentRegistry>>,
    id: &str,
) -> Result<String, String> {
    validate_child_request(request)?;
    if cancel.is_set() {
        return Err("Subagent was interrupted before starting.".into());
    }
    let restored = host.with_store(|store| {
        if store
            .run_checkpoint(id)?
            .is_some_and(|checkpoint| checkpoint.child.is_some())
        {
            restore_child_checkpoint(store, id).map(Some)
        } else {
            Ok(None)
        }
    })?;
    if let Some(checkpoint) = &restored {
        let metadata = checkpoint
            .child
            .as_ref()
            .ok_or("Missing child recovery metadata.")?;
        tools.retain(|tool| {
            metadata
                .tools
                .get(&tool.alias)
                .is_some_and(|fingerprint| *fingerprint == child_tool_fingerprint(tool))
        });
    }
    let pinned_tools: std::collections::BTreeMap<String, String> = tools
        .iter()
        .map(|tool| (tool.alias.clone(), child_tool_fingerprint(tool)))
        .collect();
    let wrappers = crate::tool_discovery::child_wrappers(&tools)?;
    let tools: Vec<_> = tools.into_iter().chain(wrappers.iter()).collect();
    let mut system = child_system_prompt(snapshot, request);
    system.push_str(&format!(
        "\nModel capability metadata (not independent verification): {}",
        backend.capability_report()
    ));
    if matches!(host, Host::Detached { .. }) {
        let limits = detached_tool_limits(snapshot, &tools, &host.policy());
        if !limits.is_empty() {
            let names = limits
                .iter()
                .filter_map(|limit| limit["alias"].as_str())
                .take(24)
                .collect::<Vec<_>>()
                .join(", ");
            system.push_str(&format!("\nThis run executes in the background: these parent-authorized tools are not usable here — {names}. Ask the parent for what you need instead."));
            host.event(id, 0, None, "tool_limits", json!({"unavailable":limits}))?;
        }
    }
    let mut messages = restored
        .map(|checkpoint| checkpoint.messages)
        .unwrap_or_else(|| {
            vec![
                json!({"role":"system","content":system}),
                json!({"role":"user","content":request.prompt}),
            ]
        });
    let mut preferences = crate::store::Preferences {
        system_prompt: snapshot.system_prompt.clone(),
        temperature: snapshot.temperature,
        top_p: snapshot.top_p,
        max_tokens: snapshot.max_tokens,
        ..Default::default()
    };
    backend.prepare_preferences(&mut preferences)?;
    let mut answer = String::new();
    let mut session = ChildSession {
        host: &host,
        snapshot,
        request,
        tools: &tools,
        backend,
        cancel: &mut cancel,
        registry: registry.as_ref(),
        id,
        pinned_tools: &pinned_tools,
        messages: &mut messages,
        preferences: &preferences,
        answer: &mut answer,
        recoveries: 0,
        output_recoveries: 0,
        event_seq: 1,
        failures: Default::default(),
        stop_reason: None,
    };
    let result = crate::agent_engine::drive_session(
        &mut session,
        (request.max_rounds > 0).then_some(request.max_rounds),
    )
    .await;
    let stop_reason = session.stop_reason;
    drop(session);
    let result = result.and_then(|answer| {
        if let Some(schema) = &request.output_schema {
            check_output_schema(schema, &answer)?;
        }
        Ok(answer)
    });
    let reason = if cancel.is_set() {
        crate::agent_engine::StopReason::UserCancelled
    } else if let Some(reason) = stop_reason {
        reason
    } else if result.is_ok() {
        crate::agent_engine::StopReason::Completed
    } else if result.as_ref().err().is_some_and(|error| {
        error.contains("budget exhausted") || error.contains("response token limit")
    }) {
        crate::agent_engine::StopReason::BudgetExhausted
    } else {
        crate::agent_engine::StopReason::ProviderFailure
    };
    let outcome = crate::agent_engine::RunOutcome {
        reason,
        resumable: reason != crate::agent_engine::StopReason::Completed
            && host
                .with_store(|store| restore_child_checkpoint(store, id))
                .is_ok(),
        checkpoint: Some(id.into()),
    };
    let result = result.and_then(|answer| match reason {
        crate::agent_engine::StopReason::PermissionDenied => Err(format!("Subagent could not complete a denied action. Partial answer: {answer}")),
        crate::agent_engine::StopReason::OutcomeUnknown => Err(format!("Subagent has an uncertain tool outcome. Review the saved action before retrying. Partial answer: {answer}")),
        _ => Ok(answer),
    });
    host.with_store(|store| store.save_run_outcome(id, &outcome))?;
    host.event(
        id,
        0,
        None,
        "turn_end",
        json!({"outcome":outcome,"error":result.as_ref().err()}),
    )?;
    result
}

struct ChildSession<'a, 'h> {
    host: &'a Host<'h>,
    snapshot: &'a BackendSnapshot,
    request: &'a ChildRequest,
    tools: &'a [&'a crate::connectors::AgentTool],
    backend: &'a crate::inference::Backend,
    cancel: &'a mut ChildCancel,
    registry: Option<&'a Arc<SubagentRegistry>>,
    id: &'a str,
    pinned_tools: &'a std::collections::BTreeMap<String, String>,
    messages: &'a mut Vec<Value>,
    preferences: &'a crate::store::Preferences,
    answer: &'a mut String,
    recoveries: u32,
    output_recoveries: u32,
    event_seq: u64,
    failures: crate::agent_engine::ActionFailures,
    stop_reason: Option<crate::agent_engine::StopReason>,
}
impl crate::agent_engine::SessionHost for ChildSession<'_, '_> {
    type Output = String;
    fn round(
        &mut self,
        round: usize,
    ) -> futures_util::future::BoxFuture<
        '_,
        Result<crate::agent_engine::RoundControl<String>, String>,
    > {
        Box::pin(async move {
            use crate::inference::InferenceProvider;
            let host = self.host;
            let snapshot = self.snapshot;
            let request = self.request;
            let tools = self.tools;
            let backend = self.backend;
            let cancel = &mut *self.cancel;
            let registry = self.registry;
            let id = self.id;
            let pinned_tools = self.pinned_tools;
            let messages = &mut *self.messages;
            let preferences = self.preferences;
            let answer = &mut *self.answer;

            if cancel.is_set() {
                return Err(INTERRUPTED.into());
            }
            host.checkpoint(id, request, snapshot, pinned_tools, messages)?;
            if let Some(registry) = &registry {
                for note in registry.take_mail(id) {
                    messages
                        .push(json!({"role":"user","content":format!("[parent followup] {note}")}));
                }
            }
            let mut payload = json!({"messages":messages,"temperature":preferences.temperature,"top_p":preferences.top_p,"max_tokens":preferences.max_tokens,"stream":true,"cache_prompt":true});
            if snapshot.inference_slots >= 2 {
                payload["id_slot"] = json!(1);
            }
            if backend.supports_tools() && !tools.is_empty() {
                payload["tools"] = json!(crate::tool_discovery::advertised(tools.iter().copied())
                    .into_iter()
                    .map(|tool| tool.definition())
                    .collect::<Vec<_>>());
            }
            if let Some(schema) = &request.output_schema {
                backend.constrain_output(&mut payload, schema)?;
            }
            let mut payload = backend.payload(&payload)?;
            let checked = tokio::select! {
                _=cancel.cancelled()=>return Err(INTERRUPTED.into()),
                result=tokio::time::timeout(Duration::from_secs(45),backend.check_context(&payload,preferences.max_tokens))=>result.map_err(|_|"Subagent context check timed out.".to_string())?,
            };
            if let Err(error) = checked {
                if let Some(plan) = crate::compaction::plan_compaction(messages, 0) {
                    host.with_store(|store| {
                        crate::compaction::apply_child_compaction(
                            store,
                            &request.conversation_id,
                            messages,
                            plan,
                        )
                    })?;
                    let mut compacted = json!({"messages":messages,"temperature":preferences.temperature,"top_p":preferences.top_p,"max_tokens":preferences.max_tokens,"stream":true});
                    if snapshot.inference_slots >= 2 {
                        compacted["id_slot"] = json!(1);
                    }
                    if backend.supports_tools() && !tools.is_empty() {
                        compacted["tools"] =
                            json!(crate::tool_discovery::advertised(tools.iter().copied())
                                .into_iter()
                                .map(|tool| tool.definition())
                                .collect::<Vec<_>>());
                    }
                    if let Some(schema) = &request.output_schema {
                        backend.constrain_output(&mut compacted, schema)?;
                    }
                    payload = backend.payload(&compacted)?;
                    tokio::select! {_=cancel.cancelled()=>return Err(INTERRUPTED.into()),result=tokio::time::timeout(Duration::from_secs(45),backend.check_context(&payload,preferences.max_tokens))=>result.map_err(|_|"Subagent context check timed out.".to_string())??};
                } else {
                    return Err(error);
                }
            }
            let response = match tokio::select! {
                _=cancel.cancelled()=>return Err(INTERRUPTED.into()),
                result=tokio::time::timeout(Duration::from_secs(600),backend.stream(&payload))=>result,
            } {
                Ok(result) => result?,
                Err(_) => return Err("Subagent model did not respond within ten minutes.".into()),
            };
            let before = answer.len();
            let outcome = match collect_stream(backend, response, answer, cancel).await {
                Ok(outcome) => {
                    self.recoveries = 0;
                    outcome
                }
                Err(error) if error.retryable && !cancel.is_set() && self.recoveries < 2 => {
                    self.recoveries += 1;
                    host.event(id,round,None,"provider_attempt_failed",json!({"error":error,"attempt":self.recoveries,"partialAnswer":&answer[before..],"unfinishedCallsDispatched":false}))?;
                    answer.truncate(before);
                    tokio::select! {_=cancel.cancelled()=>return Err(INTERRUPTED.into()),_=tokio::time::sleep(Duration::from_millis(250*u64::from(self.recoveries)))=>{}}
                    return Ok(crate::agent_engine::RoundControl::Continue);
                }
                Err(error) => return Err(error.into()),
            };
            if outcome.finish == "length" {
                host.event(id, round, None, "provider_attempt_failed", json!({
                    "reason":"response_token_limit", "partialAssistant":outcome.assistant_message,
                    "accepted":false, "unfinishedCallsDispatched":false, "attempt":self.output_recoveries + 1
                }))?;
                answer.truncate(before);
                if self.output_recoveries >= crate::agent_engine::OUTPUT_RECOVERY_LIMIT {
                    return Err(
                        "Subagent hit the response token limit after bounded recovery.".into(),
                    );
                }
                self.output_recoveries += 1;
                messages.push(
                    json!({"role":"user","content":crate::agent_engine::RETRY_TRUNCATED_CALL}),
                );
                return Ok(crate::agent_engine::RoundControl::Continue);
            }
            if outcome.calls.is_empty() {
                return Ok(crate::agent_engine::RoundControl::Complete(
                    answer[before..].to_string(),
                ));
            }
            host.event(
                id,
                round,
                None,
                "provider_round",
                json!({"assistant":outcome.assistant_message}),
            )?;
            messages.push(outcome.assistant_message);
            for mut call in outcome.calls {
                if cancel.is_set() {
                    return Err(INTERRUPTED.into());
                }
                let resolved = if crate::tool_discovery::advertised(tools.iter().copied())
                    .iter()
                    .any(|tool| tool.alias == call.name)
                {
                    crate::tool_discovery::resolve(
                        tools.iter().copied(),
                        &call.name,
                        call.arguments.clone(),
                    )
                } else {
                    Err("Tool is deferred or unavailable. Use tool_search and tool_invoke for this child's authorized catalog.".into())
                };
                let mut control = None;
                let (result, name) = match resolved {
                    Err(error) => (
                        crate::tool_envelope::failure_notice(
                            "unavailable-tool",
                            &error,
                            false,
                            crate::tool_envelope::SIDE_NONE,
                            &json!({}),
                        ),
                        call.name.clone(),
                    ),
                    Ok((tool, arguments)) => {
                        call.name = tool.alias.clone();
                        call.arguments = arguments;
                        let signature = crate::continuation::tool_signature(tool, &call.arguments);
                        let result = if self.failures.blocked(&signature) {
                            crate::tool_envelope::failure_notice("action-blocked","This identical action failed repeatedly. Inspect the previous result or use another action.",false,crate::tool_envelope::SIDE_NONE,&json!({}))
                        } else if let Some(reason) = host.policy().denial_for_tool(tool) {
                            crate::tool_envelope::failure_notice(
                                "capability-disabled",
                                &reason,
                                false,
                                crate::tool_envelope::SIDE_NONE,
                                &json!({}),
                            )
                        } else {
                            match tool.prepare_arguments(call.arguments.clone()) {
                                Err(error) => crate::tool_envelope::failure_notice(
                                    "invalid-arguments",
                                    &error,
                                    false,
                                    crate::tool_envelope::SIDE_NONE,
                                    &json!({}),
                                ),
                                Ok(arguments) => {
                                    call.arguments = arguments;
                                    host.event(
                                        id,
                                        round,
                                        Some(&call.id),
                                        "tool_started",
                                        json!({"tool":call.name,"arguments":call.arguments}),
                                    )?;
                                    host.call(
                                        snapshot,
                                        request,
                                        tools,
                                        id,
                                        round,
                                        tool,
                                        &call,
                                        cancel,
                                        &mut self.event_seq,
                                    )
                                    .await?
                                }
                            }
                        };
                        self.failures.record(&signature, &result);
                        if tool.harness_name().is_some()
                            && matches!(call.name.as_str(), "finish" | "update_context")
                        {
                            control = Some(result.clone());
                        }
                        (result, tool.tool.name.clone())
                    }
                };
                self.stop_reason =
                    crate::agent_engine::termination_after_tool(self.stop_reason, &result);
                let (bounded, artifact) = crate::artifacts::bound_tool_result(
                    result,
                    &name,
                    &request.conversation_id,
                    Some(id),
                    crate::artifacts::DEFAULT_MAX_RESULT_CHARS,
                );
                if let Some(artifact) = artifact {
                    host.with_store(|store| store.save_artifact(&artifact))?;
                }
                host.event(
                    id,
                    round,
                    Some(&call.id),
                    "tool_result",
                    json!({"tool":call.name,"result":bounded}),
                )?;
                messages.push(
                    json!({"role":"tool","tool_call_id":call.id,"content":bounded.to_string()}),
                );
                if let Some(control) = control {
                    let final_answer = host.with_store(|store| {
                        crate::arex::apply_control(
                            &call.name,
                            &control,
                            store,
                            &request.conversation_id,
                            id,
                            messages,
                        )
                    })?;
                    if let Some(final_answer) = final_answer {
                        return Ok(crate::agent_engine::RoundControl::Complete(final_answer));
                    }
                }
            }
            host.checkpoint(id, request, snapshot, pinned_tools, messages)?;
            Ok::<_, String>(crate::agent_engine::RoundControl::Continue)
        })
    }
}
