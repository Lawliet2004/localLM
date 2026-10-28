use super::*;

/// The mutable state one model turn's round loop carries.
///
/// The round loop was previously an inline async block inside
/// `send_message_inner`, which made the turn orchestration a single
/// 1,700-line function. Holding the state in one struct lets the loop be a
/// named, testable unit and keeps the call site readable.
pub(super) struct TurnRounds<'a> {
    pub(super) state: &'a AppState,
    pub(super) channel: &'a Channel<ChatEvent>,
    pub(super) run: &'a mut crate::agent_run::RunRecord,
    pub(super) assistant: &'a crate::store::Message,
    pub(super) conversation_id: String,
    pub(super) plan: &'a TurnPlan,
    pub(super) messages: &'a mut Vec<serde_json::Value>,
    pub(super) answer: &'a mut String,
    pub(super) reasoning: &'a mut String,
    pub(super) research_fallback: Option<String>,
    pub(super) stop_reason: Option<crate::agent_engine::StopReason>,
    pub(super) web_lookup_retried: bool,
    pub(super) finalization_retried: bool,
    pub(super) replay_answer: Option<serde_json::Value>,
    pub(super) last_model_message_id: String,
    pub(super) response_metrics: &'a mut crate::inference::ResponseMetrics,
    pub(super) active_measurement: &'a mut Option<(Instant, crate::inference::InferenceRound)>,
    pub(super) input_tokens: u64,
    pub(super) breakdown: ContextBreakdown,
    pub(super) compacted_once: Option<String>,
    /// The event sequence counter for this turn.
    pub(super) seq: u32,
    pub(super) plan_mode: bool,
    pub(super) presentation_mode: crate::presentation::Mode,
    pub(super) cancellation: tokio::sync::watch::Receiver<bool>,
    pub(super) run_generation: u64,
    pub(super) access_mode: crate::permissions::AccessMode,
    pub(super) preset_id: String,
    pub(super) content: &'a str,
    pub(super) snapshot: &'a mut crate::subagents::BackendSnapshot,
    pub(super) response_started: Instant,
}

impl<'a> TurnRounds<'a> {
    pub(super) async fn run(&mut self) -> Result<bool, String> {
        // Bind the turn state to locals with the names the loop body uses, so
        // the body reads exactly as it did when it was an inline block.
        let state = self.state;
        let channel = self.channel;
        let run = &mut *self.run;
        let assistant = self.assistant;
        let conversation_id = self.conversation_id.clone();
        let plan = self.plan;
        let _presentation_mode = self.presentation_mode;
        let _access_mode = self.access_mode;
        let content = self.content;
        self.seq += 1;
        channel
            .send(ChatEvent::new(
                run,
                None,
                self.seq,
                "Preparing turn",
                &assistant.id,
                "",
                "",
                None,
                None,
            ))
            .map_err(|error| error.to_string())?;

        // Loop-hygiene streaks: consecutive identical calls, guarded centrally.
        let recent_calls: Vec<(String, String)> = Vec::new();
        let research_evidence: Vec<(String, String)> = Vec::new();
        // The turn's budget cell is shared: nested dispatch paths (plan
        // steps, parallel reads) charge searches/fetches against the same
        // counters before they run, so loops cannot overspend the turn.
        let budgets = std::sync::Arc::new(std::sync::Mutex::new(
            crate::continuation::Budgets::default(),
        ));
        if let Ok(mut map) = state.turn_budgets.lock() {
            map.insert(run.id.clone(), budgets.clone());
        }
        let _budget_registration = TurnBudgetRegistration {
            map: &state.turn_budgets,
            run_id: run.id.clone(),
        };
        let corrections_used = 0u32;
        // Plan mode: mutating tools refuse until plan_submit is approved.
        let plan_approved = false;
        // Record available evidence without forcing a second execution path.
        let verification = crate::continuation::VerificationProgress::default();
        let repairs_used = 0u32;
        let failures = crate::agent_engine::ActionFailures::default();
        let output_recoveries = 0u32;
        let stream_recoveries = 0u32;
        let round_preferences = plan.preferences.clone();
        // Non-retryable signatures are shared for the run: nested dispatch
        // paths refuse to replay them even inside a later plan in this turn.
        let blocked_calls = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::HashSet::<String>::new(),
        ));
        if let Ok(mut map) = state.turn_blocked.lock() {
            map.insert(run.id.clone(), blocked_calls.clone());
        }
        let _blocked_registration = TurnBlockedRegistration {
            map: &state.turn_blocked,
            run_id: run.id.clone(),
        };
        let paused_exit = false;
        let evidence_note: Option<String> = None;
        let question = content.chars().take(4000).collect::<String>();
        let ledger = crate::research_tasks::ResearchTask::new(
            &conversation_id,
            if question.trim().is_empty() {
                "continue"
            } else {
                &question
            },
        )
        .ok();
        if let Some(task) = ledger.as_ref() {
            save_ledger(state, task);
        }
        let mut host = ForegroundSession {
            turn: self,
            recent_calls,
            research_evidence,
            budgets,
            corrections_used,
            plan_approved,
            verification,
            repairs_used,
            failures,
            output_recoveries,
            stream_recoveries,
            round_preferences,
            blocked_calls,
            paused_exit,
            evidence_note,
            ledger,
        };
        crate::agent_engine::drive_session(&mut host, None).await
    }
}

struct ForegroundSession<'s, 'a> {
    turn: &'s mut TurnRounds<'a>,
    recent_calls: Vec<(String, String)>,
    research_evidence: Vec<(String, String)>,
    budgets: std::sync::Arc<std::sync::Mutex<crate::continuation::Budgets>>,
    corrections_used: u32,
    plan_approved: bool,
    verification: crate::continuation::VerificationProgress,
    repairs_used: u32,
    failures: crate::agent_engine::ActionFailures,
    output_recoveries: u32,
    stream_recoveries: u32,
    round_preferences: crate::store::Preferences,
    blocked_calls: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<String>>>,
    paused_exit: bool,
    evidence_note: Option<String>,
    ledger: Option<crate::research_tasks::ResearchTask>,
}

impl crate::agent_engine::SessionHost for ForegroundSession<'_, '_> {
    type Output = bool;
    fn round(
        &mut self,
        round: usize,
    ) -> futures_util::future::BoxFuture<'_, Result<crate::agent_engine::RoundControl<bool>, String>>
    {
        Box::pin(async move {
            let turn = &mut *self.turn;
            let state = turn.state;
            let channel = turn.channel;
            let run = &mut *turn.run;
            let assistant = turn.assistant;
            let conversation_id = turn.conversation_id.clone();
            let plan = turn.plan;
            let messages = &mut *turn.messages;
            let answer = &mut *turn.answer;
            let reasoning = &mut *turn.reasoning;
            let response_metrics = &mut *turn.response_metrics;
            let mut active_measurement = &mut *turn.active_measurement;
            let plan_mode = turn.plan_mode;
            let mut cancellation = turn.cancellation.clone();
            let run_generation = turn.run_generation;
            let preset_id = turn.preset_id.clone();
            let content = turn.content;
            let snapshot = &mut *turn.snapshot;
            let response_started = turn.response_started;
            let tool_use_denied = false;

            state
                .database()?
                .save_run_checkpoint(&crate::agent_engine::RunCheckpoint::capture(
                    &run.id,
                    content,
                    &snapshot.workspace_path,
                    messages,
                ))?;
            // Only the last executed round may provide the saved model content;
            // an earlier round's value must not survive into a finishing exit.
            turn.replay_answer = None;
            if cancel_applies(state, run_generation) {
                return Ok(crate::agent_engine::RoundControl::Complete(false));
            }
            if state
                .run_control
                .lock()
                .map(|control| control.pause)
                .unwrap_or(false)
            {
                if let Some(task) = self.ledger.as_mut() {
                    if task.status == crate::research_tasks::TaskStatus::Running {
                        task.pending_work.push(
                            "Continue the interrupted request from the saved checkpoint.".into(),
                        );
                        let _ = task.pause();
                        save_ledger(state, task);
                    }
                }
                return Ok(crate::agent_engine::RoundControl::Complete(false));
            }
            let step_id = format!("step-{round}");
            // Capture the workspace before the step so a bad edit can be
            // reverted to the state the user left it in. The capture is
            // best-effort: a failed snapshot must never stop the turn.
            let snapshot_paths = match state.database() {
                Ok(store) => crate::snapshots::tracked_paths(&store, &conversation_id),
                Err(_) => Vec::new(),
            };
            if !snapshot_paths.is_empty() && !snapshot.workspace_path.is_empty() {
                if let Ok(store) = state.database() {
                    let _ = crate::snapshots::capture(
                        &store,
                        &conversation_id,
                        round as i64,
                        std::path::Path::new(&snapshot.workspace_path),
                        &snapshot_paths,
                    );
                }
            }
            // Plan mode gates mutating tools behind plan_submit approval;
            // read-only tools stay offered for investigation, so there is no
            // tool-free planning pass anymore.
            if plan_mode && round > 0 {
                if let Ok(todos) = state.database()?.todos(&conversation_id) {
                    refresh_plan_focus(messages, &todos);
                }
            }
            let finalizing = turn.research_fallback.is_some()
                || turn.finalization_retried
                || self
                    .budgets
                    .lock()
                    .map(|b| b.reserve_final_response())
                    .unwrap_or(true);
            if self
                .budgets
                .lock()
                .map(|b| b.reserve_final_response())
                .unwrap_or(true)
                && turn.research_fallback.is_none()
            {
                turn.stop_reason = Some(crate::agent_engine::StopReason::BudgetExhausted);
                turn.research_fallback = Some("Partial result. The model round budget is exhausted. State the completed work and name unresolved steps without claiming completion.".into());
                messages.push(json!({"role":"user","content":turn.research_fallback.as_deref().unwrap_or_default()}));
            }
            if let Ok(mut b) = self.budgets.lock() {
                b.note_round();
            }
            let _ = run.transition_to(crate::agent_run::RunState::Generating);
            let _ = state
                .database()?
                .update_run_status(&run.id, run.status, None, Some(&step_id));
            turn.seq += 1;
            let _ = state
                .database()?
                .append_run_event(&crate::agent_run::RunEvent {
                    run_id: run.id.clone(),
                    seq: turn.seq as u64,
                    step_id: step_id.clone(),
                    tool_call_id: None,
                    event_type: "step_start".into(),
                    payload: json!({"round": round}),
                    created_at: crate::store::now(),
                });
            if let Ok(store) = state.database() {
                emit_session_event(
                    &store,
                    &conversation_id,
                    Some(&run.id),
                    Some(&step_id),
                    None,
                    "step_start",
                    json!({"round": round}),
                    false,
                );
            }

            if round > 0 {
                let (tokens, compacted, breakdown) = tokio::select! {
                    _ = cancellation.changed() => return Ok(crate::agent_engine::RoundControl::Complete(false)),
                    result = prepare_context(state, plan, &self.round_preferences, &conversation_id, messages, tool_use_denied, finalizing) => result?,
                };
                turn.input_tokens = tokens;
                turn.breakdown = breakdown;
                if let Some(artifact) = compacted {
                    let store = state.database()?;
                    emit_session_event(
                        &store,
                        &conversation_id,
                        Some(&run.id),
                        Some(&step_id),
                        None,
                        "compaction",
                        json!({"artifact":artifact,"auto":true,"thresholdPercent":80,"inputTokens":tokens}),
                        false,
                    );
                    let mut event = ChatEvent::progress(
                        &run.id,
                        &step_id,
                        &assistant.id,
                        "Context compacted · continuing…",
                        None,
                    );
                    event.notice = Some("Context reached 80%. Older activity was archived; continuing this response.".into());
                    channel.send(event).map_err(|e| e.to_string())?;
                }
            } else if turn.compacted_once.is_some() {
                let mut event = ChatEvent::progress(
                    &run.id,
                    &step_id,
                    &assistant.id,
                    "Context compacted · continuing…",
                    None,
                );
                event.notice = Some(
                    "Context reached 80%. Older activity was archived; continuing this response."
                        .into(),
                );
                let _ = channel.send(event);
            }
            let payload = plan.backend.payload(&request_payload(
                messages,
                &plan.tool_definitions,
                &self.round_preferences,
                tool_use_denied,
                finalizing,
                false,
                plan.id_slot,
                plan.local_arex(),
            ))?;
            turn.seq += 1;
            let _ = channel.send(ChatEvent::new(
            run,
            Some(&step_id),
            turn.seq,
            "Generating response…",
            &assistant.id,
            "",
            "",
            Some(json!({"inputTokens":turn.input_tokens,"responseReserve":self.round_preferences.max_tokens,"contextLength":plan.context_length,"estimated":plan.backend.context_is_estimate(),"breakdown":turn.breakdown,"avgCacheHitRate":conversation_cache_hit_rate(state, &conversation_id, None)})),
            None,
        ));

            if response_metrics.preparation_ms.is_none() {
                response_metrics.preparation_ms =
                    Some(response_started.elapsed().as_secs_f64() * 1000.0);
            }
            let round_start_time = Instant::now();
            *active_measurement = Some((
                round_start_time,
                crate::inference::InferenceRound {
                    phase: None,
                    usage: Default::default(),
                    first_token_ms: None,
                    first_visible_answer_ms: None,
                    reasoning_ms: None,
                    duration_ms: 0.0,
                    complete: false,
                },
            ));
            let response = tokio::select! {
                _ = cancellation.changed() => return Ok(crate::agent_engine::RoundControl::Complete(false)),
                result = tokio::time::timeout(Duration::from_secs(600), plan.backend.stream(&payload)) => result.map_err(|_| "Model did not respond within ten minutes.".to_string()).and_then(|response| response),
            };
            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    if let Ok(store) = state.database() {
                        emit_session_event(
                            &store,
                            &conversation_id,
                            Some(&run.id),
                            Some(&step_id),
                            None,
                            "model_step_failed",
                            json!({"round":round,"error":error,"toolWorkPreserved":true,"automaticallyRetried":false}),
                            false,
                        );
                    }
                    return Err(format!("{error} Failed at model round {}. Earlier tool results are preserved; review their outcomes before retrying the request.", round + 1));
                }
            };
            let mut stream = crate::agent_engine::ModelRound::new(&plan.backend, response);
            let mut checkpoint = Instant::now();
            let mut round_answer = String::new();
            let mut round_reasoning = String::new();
            let mut reasoning_started: Option<Instant> = None;
            let mut reasoning_last: Option<Instant> = None;
            // A call attempt on a no-tool round is not a visible answer — but
            // its markup is stripped from the answer text all the same.
            let mut suppressed_call = false;
            // A finalizing round whose calls were suppressed is already
            // recorded in the transcript. The retry path must not record the
            // same round a second time.
            let mut suppressed_round_recorded = false;
            let answer_len_before_round = answer.len();
            let reasoning_len_before_round = reasoning.len();
            // The verbatim-loop detector needs enough output to compare; it
            // then runs once per further ~1 KiB of generated text.
            let mut next_rep_check = 1_200usize;
            let mut repetition_stopped = false;
            'stream: loop {
                let next = tokio::select! {
                    _ = cancellation.changed() => return Ok(crate::agent_engine::RoundControl::Complete(false)),
                    next = stream.next() => next,
                };
                let next = match next {
                    Ok(next) => next,
                    Err(error) if error.retryable && self.stream_recoveries < 2 => {
                        self.stream_recoveries += 1;
                        // This attempt never reached native completion. Preserve
                        // it for inspection, discard every unfinished call, and
                        // regenerate from the last committed tool exchange.
                        {
                            let store = state.database()?;
                            emit_session_event(
                                &store,
                                &conversation_id,
                                Some(&run.id),
                                Some(&step_id),
                                None,
                                "provider_attempt_failed",
                                json!({"error":error,"attempt":self.stream_recoveries,
                                "partialAnswer":&answer[answer_len_before_round..],
                                "partialReasoning":&reasoning[reasoning_len_before_round..],
                                "toolWorkPreserved":true,"unfinishedCallsDispatched":false}),
                                false,
                            );
                        }
                        answer.truncate(answer_len_before_round);
                        reasoning.truncate(reasoning_len_before_round);
                        if let Some((start, mut measurement)) = active_measurement.take() {
                            measurement.duration_ms = start.elapsed().as_secs_f64() * 1000.0;
                            measurement.complete = false;
                            response_metrics.rounds.push(measurement);
                        }
                        let mut event = ChatEvent::progress(
                            &run.id,
                            &step_id,
                            &assistant.id,
                            "Recovering provider stream…",
                            None,
                        );
                        event.discard_step = Some(step_id.clone());
                        event.notice = Some("Connection interrupted. Retrying this model response; completed tool actions are preserved.".into());
                        channel.send(event).map_err(|e| e.to_string())?;
                        tokio::select! {
                            _ = cancellation.changed() => return Ok(crate::agent_engine::RoundControl::Complete(false)),
                            _ = tokio::time::sleep(Duration::from_millis(250 * u64::from(self.stream_recoveries))) => {},
                        }
                        let _ = run.transition_to(crate::agent_run::RunState::PreparingNextRound);
                        return Ok(crate::agent_engine::RoundControl::Continue);
                    }
                    Err(error) => return Err(error.into()),
                };
                let Some(delta) = next else {
                    break 'stream;
                };
                if let Some((_, measurement)) = &mut active_measurement {
                    measurement.usage = stream.content.usage.clone();
                    if measurement.first_token_ms.is_none() && delta.has_token {
                        measurement.first_token_ms =
                            Some(round_start_time.elapsed().as_secs_f64() * 1000.0);
                    }
                }

                let ans_chunk = delta.answer;
                let thought_chunk = delta.reasoning;

                if !thought_chunk.is_empty() {
                    let now = Instant::now();
                    reasoning_started.get_or_insert(now);
                    reasoning_last = Some(now);
                }
                if let Some((_, measurement)) = &mut active_measurement {
                    if !ans_chunk.trim().is_empty() && measurement.first_visible_answer_ms.is_none()
                    {
                        measurement.first_visible_answer_ms =
                            Some(response_started.elapsed().as_secs_f64() * 1000.0);
                    }
                    measurement.reasoning_ms = reasoning_started
                        .zip(reasoning_last)
                        .map(|(start, end)| end.duration_since(start).as_secs_f64() * 1000.0);
                }
                answer.push_str(&ans_chunk);
                round_answer.push_str(&ans_chunk);
                reasoning.push_str(&thought_chunk);
                round_reasoning.push_str(&thought_chunk);
                if answer.len() + reasoning.len() > 4_194_304 {
                    return Err("Model output exceeded 4 MiB.".into());
                }
                if round_answer.len() >= next_rep_check {
                    next_rep_check = round_answer.len() + 1024;
                    // A loop may have re-emitted the block more than once;
                    // keep cutting until only the first copy remains.
                    let mut removed = 0usize;
                    while let Some(cut) = repetition_cut(&round_answer) {
                        removed += round_answer.len() - cut;
                        round_answer.truncate(cut);
                    }
                    if removed > 0 {
                        repetition_stopped = true;
                        answer.truncate(answer.len().saturating_sub(removed));
                        let mut event = ChatEvent::progress(
                            &run.id,
                            &step_id,
                            &assistant.id,
                            "Stopping repeated output…",
                            None,
                        );
                        event.notice = Some("The model began repeating its earlier output verbatim; the repeated text was removed.".into());
                        let _ = channel.send(event);
                        break 'stream;
                    }
                }
                if !ans_chunk.is_empty() || !thought_chunk.is_empty() {
                    turn.seq += 1;
                    let elapsed_secs = round_start_time.elapsed().as_secs_f64();
                    let mut evt = ChatEvent::new(
                        run,
                        Some(&step_id),
                        turn.seq,
                        if stream.content.is_thinking() || !thought_chunk.is_empty() {
                            "Thinking…"
                        } else {
                            "Streaming response…"
                        },
                        &assistant.id,
                        &ans_chunk,
                        &thought_chunk,
                        None,
                        None,
                    );
                    evt.elapsed_secs = Some(elapsed_secs);
                    evt.round_index = Some(round);
                    channel.send(evt).map_err(|error| error.to_string())?;
                }
                if checkpoint.elapsed() > Duration::from_millis(500) {
                    state.database()?.update_message(
                        &assistant.id,
                        answer,
                        reasoning,
                        "streaming",
                    )?;
                    checkpoint = Instant::now();
                }
            }
            if repetition_stopped {
                turn.stop_reason = Some(crate::agent_engine::StopReason::Blocked);
                return Err("Model output repeated before the response completed. Partial output was saved; unfinished tool calls were not executed.".into());
            }
            self.stream_recoveries = 0;
            let usage = stream.content.usage.clone();
            let finish_reason = stream.content.finish.clone();
            let mut calls = std::mem::take(&mut stream.content.calls);
            if let Some((start, mut measurement)) = active_measurement.take() {
                measurement.duration_ms = start.elapsed().as_secs_f64() * 1000.0;
                measurement.complete = !repetition_stopped;
                response_metrics.rounds.push(measurement);
            }
            if usage.reported() {
                let mut context = usage.context(
                    turn.input_tokens,
                    self.round_preferences.max_tokens,
                    plan.context_length,
                    plan.backend.context_is_estimate(),
                );
                context["breakdown"] = serde_json::to_value(turn.breakdown).unwrap_or_default();
                context["avgCacheHitRate"] = json!(conversation_cache_hit_rate(
                    state,
                    &conversation_id,
                    Some(&usage)
                ));
                turn.seq += 1;
                let _ = channel.send(ChatEvent::new(
                    run,
                    Some(&step_id),
                    turn.seq,
                    "Model usage measured",
                    &assistant.id,
                    "",
                    "",
                    Some(context),
                    None,
                ));
                if let Ok(store) = state.database() {
                    emit_session_event(
                        &store,
                        &conversation_id,
                        Some(&run.id),
                        Some(&step_id),
                        None,
                        "inference_usage",
                        json!({"round":round,"usage":usage}),
                        false,
                    );
                }
            }
            // Persist exposed reasoning before finalization can return or
            // tool validation can fail; each completed model round owns its row.
            if let Ok(store) = state.database() {
                if !round_reasoning.is_empty() {
                    emit_session_event(
                        &store,
                        &conversation_id,
                        Some(&run.id),
                        Some(&step_id),
                        None,
                        "reasoning",
                        json!({"text": round_reasoning, "durationMs": reasoning_started.zip(reasoning_last).map(|(start, end)| end.duration_since(start).as_secs_f64() * 1000.0)}),
                        true,
                    );
                }
            }
            // Handle length BEFORE empty-answer finalization and text-call repair.
            // Otherwise a thinking-only cutoff disables tools on the next round,
            // and an unfinished file call can never become a real write.
            if finish_reason == "length" {
                if finalizing || tool_use_denied || self.output_recoveries >= OUTPUT_RECOVERY_LIMIT
                {
                    turn.stop_reason = Some(crate::agent_engine::StopReason::BudgetExhausted);
                    check_finish_reason(&finish_reason)?;
                }
                self.output_recoveries += 1;
                let unfinished_call = !calls.is_empty() || is_text_tool_call(&round_answer);
                if unfinished_call {
                    // Do not replay invalid provider items or malformed JSON, and
                    // never dispatch any sibling calls from this truncated batch.
                    answer.truncate(answer_len_before_round);
                } else if !round_answer.trim().is_empty() {
                    let continued = stream.assistant_message(&round_answer, &[]);
                    messages.push(
                        if crate::history::saved_native_message(&continued).is_some() {
                            continued
                        } else {
                            json!({"role":"assistant","content":round_answer})
                        },
                    );
                }
                messages.push(json!({"role":"user","content":if unfinished_call || round_answer.trim().is_empty() {
                RETRY_TRUNCATED_CALL
            } else {
                crate::prompt::CONTINUE_INSTRUCTION
            }}));
                // API rounds already request the configured model maximum (or
                // omit the limit for a provider-managed default). Recovery must
                // neither override that choice nor impose the local 32K ceiling.
                let previous_budget = self.round_preferences.max_tokens;
                if matches!(&plan.backend, Backend::Local { .. }) {
                    self.round_preferences.max_tokens = recovery_response_budget(
                        previous_budget,
                        turn.input_tokens,
                        plan.context_length,
                        None,
                    );
                }
                let store = state.database()?;
                emit_session_event(
                    &store,
                    &conversation_id,
                    Some(&run.id),
                    Some(&step_id),
                    None,
                    "step_end",
                    json!({"continuing":true,"reason":"response_token_limit",
                    "retry":self.output_recoveries,"unfinishedToolCall":unfinished_call,
                    "responseReserve":self.round_preferences.max_tokens}),
                    false,
                );
                let mut notice = ChatEvent::progress(
                    &run.id,
                    &step_id,
                    &assistant.id,
                    "Output limit reached · retrying…",
                    None,
                );
                let continuation = if self.round_preferences.max_tokens > previous_budget {
                    "Continuing with a larger response budget."
                } else {
                    "Continuing in another response."
                };
                notice.notice = Some(format!(
                    "The model reached this response's output limit. {continuation}{}",
                    if unfinished_call {
                        " Unfinished tool calls were not executed."
                    } else {
                        ""
                    }
                ));
                let _ = channel.send(notice);
                let _ = run.transition_to(crate::agent_run::RunState::PreparingNextRound);
                store.update_run_status(&run.id, run.status, None, Some(&step_id))?;
                return Ok(crate::agent_engine::RoundControl::Continue);
            }
            let mut text_parsed_calls = false;
            if !finalizing && calls.is_empty() {
                match decide_text_tool_call(&round_answer, self.repairs_used) {
                    // Some templates (AREX-family) emit literal tool-call text;
                    // execute it through the normal permissioned dispatch.
                    TextCallDecision::Execute(parsed) => {
                        calls = parsed;
                        text_parsed_calls = true;
                        let stripped = crate::tool_calls::strip_text_tool_calls(&round_answer);
                        if answer.ends_with(&round_answer) {
                            answer.truncate(answer.len() - round_answer.len());
                            answer.push_str(&stripped);
                        }
                        round_answer = stripped;
                    }
                    TextCallDecision::Repair { error } => {
                        self.repairs_used += 1;
                        answer.truncate(answer_len_before_round);
                        let repaired = stream.assistant_message(&round_answer, &[]);
                        if crate::history::saved_native_message(&repaired).is_some() {
                            messages.push(repaired);
                        }
                        messages.push(json!({"role":"user","content":format!("The previous tool call was malformed and was not executed. Validation error: {error}. Reply with one valid tool call or a final answer.")}));
                        let _ = run.transition_to(crate::agent_run::RunState::PreparingNextRound);
                        state.database()?.update_run_status(
                            &run.id,
                            run.status,
                            None,
                            Some(&step_id),
                        )?;
                        return Ok(crate::agent_engine::RoundControl::Continue);
                    }
                    TextCallDecision::Fail { error } => {
                        return Err(format!(
                            "Tool call repair failed: {error}. Later turns can still call tools."
                        ));
                    }
                    TextCallDecision::NotACall => {}
                }
            }
            check_finish_reason(&finish_reason)?;
            if !finalizing
                && !tool_use_denied
                && calls.is_empty()
                && finish_reason == "stop"
                && crate::continuation::retry_missing_web_lookup(
                    &round_answer,
                    content,
                    plan.tools.iter().any(|tool| {
                        tool.connector == "Harness"
                            && matches!(tool.alias.as_str(), "web_search" | "search")
                    }),
                    self.budgets
                        .lock()
                        .map(|b| b.searches_used > 0 || b.fetches_used > 0)
                        .unwrap_or(false),
                    turn.web_lookup_retried,
                )
            {
                turn.web_lookup_retried = true;
                answer.truncate(answer_len_before_round);
                let retry_reply = stream.assistant_message(&round_answer, &[]);
                if crate::history::saved_native_message(&retry_reply).is_some() {
                    messages.push(retry_reply);
                } else {
                    messages.push(json!({"role":"assistant","content":round_answer}));
                }
                messages
                    .push(json!({"role":"user","content":crate::continuation::WEB_LOOKUP_RETRY}));
                if let Ok(store) = state.database() {
                    emit_session_event(
                        &store,
                        &conversation_id,
                        Some(&run.id),
                        Some(&step_id),
                        None,
                        "web_lookup_retry",
                        json!({"round":round,"reason":"claimed_no_live_access_without_lookup"}),
                        false,
                    );
                }
                channel
                    .send(ChatEvent::progress(
                        &run.id,
                        &step_id,
                        &assistant.id,
                        "Checking available web sources…",
                        None,
                    ))
                    .map_err(|e| e.to_string())?;
                let _ = run.transition_to(crate::agent_run::RunState::PreparingNextRound);
                state
                    .database()?
                    .update_run_status(&run.id, run.status, None, Some(&step_id))?;
                return Ok(crate::agent_engine::RoundControl::Continue);
            }
            if finalizing {
                // AREX answers through its finish tool; the finalizing payload
                // keeps that channel open for local AREX, but a call can still
                // arrive as literal markup or a structured delta. A finish call
                // runs through the same control path as the tool loop; any
                // other call is suppressed — stripped from the answer and still
                // judged "no visible answer" for the retry/fallback decision.
                let has_markup = is_text_tool_call(&round_answer)
                    || round_answer.contains("<tool_call")
                    || round_answer.contains("<function=");
                if has_markup {
                    let stripped = strip_call_markup(&round_answer);
                    if answer.ends_with(&round_answer) {
                        answer.truncate(answer.len() - round_answer.len());
                        answer.push_str(&stripped);
                    }
                    if let Some(parsed) = crate::tool_calls::parse_text_tool_calls(&round_answer) {
                        if let Some(call) = parsed.iter().find(|call| call.name == "finish") {
                            if let Some((row, result_text)) = run_finish_call(
                                state,
                                channel,
                                run,
                                &step_id,
                                &conversation_id,
                                &assistant.id,
                                &mut turn.seq,
                                answer,
                                &self.research_evidence,
                                plan.access_mode,
                                call,
                                &round_answer,
                                None,
                            )
                            .await?
                            {
                                turn.last_model_message_id = row.id.clone();
                                // Text-parsed calls are not in the native output, so this
                                // round replays canonically — the same shape rebuild emits.
                                messages.push(json!({"role":"assistant","content":round_answer,"tool_calls":[call.model_value()]}));
                                messages.push(json!({"role":"tool","tool_call_id":call.id,"content":result_text}));
                                return Ok(crate::agent_engine::RoundControl::Complete(true));
                            }
                        }
                    }
                    round_answer = stripped;
                    suppressed_call = true;
                }
                if !calls.is_empty() {
                    suppressed_call = true;
                    // Validate without consuming: a malformed call is a real
                    // error, not an empty round, and the caller still needs to
                    // see that this round produced calls.
                    let finished = calls.validated()?;
                    if let Some(call) = finished.iter().find(|call| call.name == "finish") {
                        let finish_call_id = call.id.clone();
                        let finish_message = {
                            let message = stream.assistant_message(&round_answer, &finished);
                            // The rebuild guard rejects native rounds with more
                            // calls than the replay cap; larger rounds keep the
                            // canonical transcript instead of a durable native one.
                            if finished.len() <= 8
                                && crate::history::saved_native_message(&message).is_some()
                            {
                                Some(message)
                            } else {
                                None
                            }
                        };
                        if let Some((row, result_text)) = run_finish_call(
                            state,
                            channel,
                            run,
                            &step_id,
                            &conversation_id,
                            &assistant.id,
                            &mut turn.seq,
                            answer,
                            &self.research_evidence,
                            plan.access_mode,
                            call,
                            &round_answer,
                            finish_message.as_ref(),
                        )
                        .await?
                        {
                            turn.last_model_message_id = row.id.clone();
                            // Record the round exactly as rebuild will replay it: the
                            // model's own output plus one result per emitted call —
                            // suppressed calls keep an explicit unknown outcome rather
                            // than a synthetic result or a missing entry.
                            messages.push(finish_message.unwrap_or_else(|| json!({"role":"assistant","content":round_answer,"tool_calls":finished.iter().map(|finished_call| finished_call.model_value()).collect::<Vec<_>>()})));
                            for finished_call in &finished {
                                let content = if finished_call.id == finish_call_id {
                                    result_text.clone()
                                } else {
                                    crate::tool_envelope::failure_notice("suppressed-tool-call","This call was superseded by the finish call in the same round and was not dispatched. Its outcome is unknown; do not automatically repeat it.",false,crate::tool_envelope::SIDE_UNKNOWN,&json!({})).to_string()
                                };
                                messages.push(json!({"role":"tool","tool_call_id":finished_call.id,"content":content}));
                            }
                            return Ok(crate::agent_engine::RoundControl::Complete(true));
                        }
                    } else {
                        // The round produced calls but no finish call, so they are
                        // suppressed rather than dispatched. The round is still
                        // recorded: the model emitted calls, and the transcript
                        // must carry them together with an explicit
                        // unknown-outcome result for each. Without this the next
                        // provider request would carry a native function_call
                        // with no matching canonical tool_call, which the
                        // Responses adapter rejects outright and which leaves a
                        // Claude `tool_use` without its `tool_result`.
                        let message = stream.assistant_message(&round_answer, &finished);
                        messages.push(if crate::history::saved_native_message(&message).is_some() {
                        message
                    } else {
                        json!({"role":"assistant","content":round_answer,"tool_calls":finished.iter().map(|finished_call| finished_call.model_value()).collect::<Vec<_>>()})
                    });
                        for finished_call in &finished {
                            let content = crate::tool_envelope::failure_notice("suppressed-tool-call","This call was suppressed on a finalizing round and was not dispatched. Its outcome is unknown; do not automatically repeat it.",false,crate::tool_envelope::SIDE_UNKNOWN,&json!({})).to_string();
                            messages.push(json!({"role":"tool","tool_call_id":finished_call.id,"content":content}));
                        }
                        suppressed_round_recorded = true;
                        // The calls are recorded, not dispatched.
                        calls = crate::tool_calls::ToolCalls::default();
                    }
                }
            }
            if needs_final_answer(finalizing, !calls.is_empty(), &round_answer) {
                // On a finalizing round a suppressed call attempt still counts
                // as no visible answer; retry once, then use the grounded result.
                let final_answer = if calls.is_empty() && !suppressed_call {
                    &round_answer
                } else {
                    ""
                };
                match finalization_action(
                    final_answer,
                    turn.research_fallback.as_deref(),
                    turn.finalization_retried
                        || self.budgets.lock().map(|b| b.exhausted()).unwrap_or(true),
                    false,
                ) {
                    FinalizationAction::Complete => {}
                    FinalizationAction::Retry => {
                        turn.finalization_retried = true;
                        // The superseded draft stays in the transcript for
                        // context but leaves the visible answer, so a
                        // regenerated attempt does not stack a second copy.
                        answer.truncate(answer_len_before_round);
                        // Preserve provider-owned replay context for a completed
                        // draft; suppressed calls never acquire synthetic results.
                        // A round whose suppressed calls were already recorded
                        // must not be recorded again.
                        if !suppressed_round_recorded {
                            let native_reply = stream.assistant_message(&round_answer, &[]);
                            if calls.is_empty()
                                && crate::history::saved_native_message(&native_reply).is_some()
                            {
                                messages.push(native_reply);
                            } else if !round_answer.trim().is_empty()
                                && !is_text_tool_call(&round_answer)
                            {
                                messages.push(json!({"role":"assistant","content":round_answer}));
                            }
                        }
                        messages.push(json!({"role":"user","content":FINAL_ANSWER_INSTRUCTION}));
                        if let Ok(store) = state.database() {
                            emit_session_event(
                                &store,
                                &conversation_id,
                                Some(&run.id),
                                Some(&step_id),
                                None,
                                "finalization_retry",
                                json!({"round": round, "reason": "no_visible_answer", "finishReason": finish_reason, "reasoningLength": round_reasoning.len()}),
                                false,
                            );
                        }
                        let _ = run.transition_to(crate::agent_run::RunState::PreparingNextRound);
                        state.database()?.update_run_status(
                            &run.id,
                            run.status,
                            None,
                            Some(&step_id),
                        )?;
                        return Ok(crate::agent_engine::RoundControl::Continue);
                    }
                    FinalizationAction::Fallback(fallback) => {
                        turn.replay_answer = Some(json!(fallback));
                        *answer = strip_call_markup(answer);
                        if !answer.trim().is_empty() {
                            answer.push_str("\n\n");
                        }
                        answer.push_str(&fallback);
                        turn.seq += 1;
                        channel
                            .send(ChatEvent::new(
                                run,
                                Some(&step_id),
                                turn.seq,
                                "Using completed research answer…",
                                &assistant.id,
                                &fallback,
                                "",
                                None,
                                None,
                            ))
                            .map_err(|error| error.to_string())?;
                        if let Ok(store) = state.database() {
                            emit_session_event(
                                &store,
                                &conversation_id,
                                Some(&run.id),
                                Some(&step_id),
                                None,
                                "research_answer_fallback",
                                json!({"round": round, "answerLength": fallback.len()}),
                                false,
                            );
                        }
                        return Ok(crate::agent_engine::RoundControl::Complete(true));
                    }
                    FinalizationAction::Error => return Err(EMPTY_FINAL_ANSWER_ERROR.into()),
                }
            }
            let calls = calls.finish()?;
            let accepted = stream.accepted_record(finish_reason.clone(), &round_answer, calls)?;
            let calls = accepted.calls;
            turn.replay_answer = Some(if calls.is_empty() {
                crate::history::saved_native_message(&accepted.assistant_message)
                    .unwrap_or_else(|| json!(round_answer))
            } else {
                Value::Null
            });
            if !calls.is_empty() && matches!(&plan.backend, Backend::Local { .. }) {
                state.database()?.record_local_tool_calling_support(
                    &plan.preferences.runtime_path,
                    &plan.preferences.model_path,
                    true,
                )?;
            }
            {
                let store = state.database()?;
                emit_session_event(
                    &store,
                    &conversation_id,
                    Some(&run.id),
                    Some(&step_id),
                    None,
                    "model_response",
                    json!({"content":round_answer,"final":calls.is_empty()}),
                    false,
                );
            }
            if calls.is_empty() {
                if round_answer.trim().is_empty() {
                    return Err(EMPTY_FINAL_ANSWER_ERROR.into());
                }
                // Plain-text completions skip `finish`, so the claim audit
                // would never run — apply it here when the turn did research.
                if plan.local_arex() && !self.research_evidence.is_empty() {
                    let audit_args = json!({"answer": answer, "evidences": []});
                    if let Some(footer) = claim_audit_footer(
                        state,
                        &audit_args,
                        &self.research_evidence,
                        run,
                        &step_id,
                        &conversation_id,
                        &mut turn.seq,
                    )
                    .await
                    {
                        answer.push_str(&footer);
                        turn.seq += 1;
                        channel
                            .send(ChatEvent::new(
                                run,
                                Some(&step_id),
                                turn.seq,
                                "Claim audit",
                                &assistant.id,
                                &footer,
                                "",
                                None,
                                None,
                            ))
                            .map_err(|error| error.to_string())?;
                    }
                }
                let final_answer = answer[answer_len_before_round..].to_string();
                commit_final_answer(
                    state,
                    run,
                    &step_id,
                    &conversation_id,
                    &assistant.id,
                    answer,
                    &final_answer,
                )?;
                if let Ok(store) = state.database() {
                    emit_session_event(
                        &store,
                        &conversation_id,
                        Some(&run.id),
                        Some(&step_id),
                        None,
                        "step_end",
                        json!({"round": round, "content": round_answer, "toolCallsCount": 0}),
                        false,
                    );
                }
                return Ok(crate::agent_engine::RoundControl::Complete(true));
            }
            if tool_use_denied {
                return Err("Tool use stopped after your denial. Send a new message to authorize further actions.".into());
            }
            let mut round_model_message = accepted.assistant_message;
            if text_parsed_calls {
                // Calls parsed from literal text are absent from the native
                // output; keeping it would fail replay parity and, for Claude,
                // fabricate tool_use blocks the provider never emitted.
                if let Some(object) = round_model_message.as_object_mut() {
                    object.remove("locallm_responses_output");
                    object.remove("locallm_claude_content");
                }
            }
            let mut round_model_saved = false;
            messages.push(round_model_message.clone());
            if let Ok(store) = state.database() {
                for call in &calls {
                    emit_session_event(
                        &store,
                        &conversation_id,
                        Some(&run.id),
                        Some(&step_id),
                        Some(&call.id),
                        "tool_call",
                        json!({
                            "callId": call.id,
                            "name": call.name,
                            "arguments": call.arguments,
                        }),
                        false,
                    );
                }
            }
            let tool_calls_count = calls.len();
            let mut explicit_finish = None;
            let mut finish_args: Option<Value> = None;
            // Bounded concurrency for independent reads: consecutive
            // auto-allowed trusted reads are dispatched as a group; every
            // other call is an ordered barrier on the sequential path.
            let mut prefetched: std::collections::HashMap<String, Prefetched> =
                std::collections::HashMap::new();
            let mut call_index = 0usize;
            while call_index < calls.len() {
                let mut call = calls[call_index].clone();
                let resolved = if crate::tool_discovery::visible(&plan.tool_definitions, &call.name)
                {
                    crate::tool_discovery::resolve(
                        plan.tools.iter(),
                        &call.name,
                        call.arguments.clone(),
                    )
                } else {
                    Err("Tool schema is deferred or unavailable. Use tool_search and tool_invoke for authorized deferred tools.".into())
                };
                let (tool, arguments) = match resolved {
                    Ok(resolved) => resolved,
                    Err(error) => {
                        let result = crate::tool_envelope::failure_notice(
                            "unavailable-tool",
                            &error,
                            false,
                            crate::tool_envelope::SIDE_NONE,
                            &json!({}),
                        );
                        let store = state.database()?;
                        let row=store.append_message(&conversation_id,"tool",&json!({"request":{"connector":"Unavailable","name":call.name,"arguments":call.arguments,"callId":call.id,"stepId":step_id,"decision":"unavailable"},"result":result}).to_string(),"complete")?;
                        crate::history::remember_native_round(
                            &store,
                            &row,
                            &round_model_message,
                            &mut round_model_saved,
                        )?;
                        messages.push(json!({"role":"tool","tool_call_id":call.id,"content":result.to_string()}));
                        call_index += 1;
                        continue;
                    }
                };
                let discovery_invocation = call.name != tool.alias;
                call.name = tool.alias.clone();
                call.arguments = arguments;
                // Group dispatch honours every pre-dispatch gate the sequential
                // path applies: an already-recorded cancellation, a pending
                // pause, a call the repetition guard would suppress, and the
                // per-call eligibility check all decide BEFORE any dispatch.
                let group_end = if !discovery_invocation
                    && !prefetched.contains_key(&call.id)
                    && !(*cancellation.borrow() && cancel_applies(state, run_generation))
                    && crate::sandbox::check_repetition(
                        &self.recent_calls,
                        &call.name,
                        &call.arguments.to_string(),
                    )
                    .is_none()
                {
                    self.blocked_calls.lock().ok().and_then(|blocked| {
                        if concurrent_read_ok(&call, plan, state, &blocked, tool_use_denied) {
                            Some(read_group_end(
                                &calls,
                                call_index,
                                plan,
                                state,
                                &blocked,
                                tool_use_denied,
                                &self.recent_calls,
                            ))
                        } else {
                            None
                        }
                    })
                } else {
                    None
                };
                if let Some(end) = group_end.filter(|end| *end > call_index + 1) {
                    let reserved = crate::history::reserve_native_read_round(
                        &*state.database()?,
                        &conversation_id,
                        &json!({"connector":tool.connector,"localServerName":tool.local_server_name(),"name":tool.tool.name,"arguments":call.arguments,"decision":"allowed","accessMode":plan.access_mode,"authorization":"auto-approved read group (reserved before dispatch)","callId":call.id,"stepId":step_id,"assistantContent":round_answer}),
                        &round_model_message,
                        &mut round_model_saved,
                    )?;
                    prefetch_read_group(
                        state,
                        plan,
                        snapshot,
                        &conversation_id,
                        run,
                        &step_id,
                        &assistant.id,
                        round,
                        channel,
                        &cancellation,
                        &mut self.ledger,
                        &calls[call_index..end],
                        &mut prefetched,
                    )
                    .await;
                    if let Some(entry) = prefetched.get_mut(&call.id) {
                        entry.audit_row = reserved;
                    }
                }
                let was_prefetched = prefetched.contains_key(&call.id);
                call_index += 1;

                // Capability kills are re-checked at dispatch: a frozen catalog
                // entry offered before the kill must still deny visibly. A
                // prefetched member passed this exact gate at dispatch — its
                // real outcome is already recorded, so it is never re-labelled.
                let capability_denial = if was_prefetched {
                    None
                } else {
                    crate::capabilities::tool_capability(tool).and_then(|capability| {
                        crate::capabilities::ToolPolicy::guard(state.database()).denial(capability)
                    })
                };
                if let Some(reason) = capability_denial {
                    let result = crate::tool_envelope::failure_notice(
                        "capability-disabled",
                        &reason,
                        false,
                        crate::tool_envelope::SIDE_NONE,
                        &json!({}),
                    );
                    turn.stop_reason =
                        crate::agent_engine::termination_after_tool(turn.stop_reason, &result);
                    let store = state.database()?;
                    let recorded=store.append_message(&conversation_id, "tool", &json!({"request":{"connector":tool.connector,"name":tool.tool.name,"arguments":call.arguments,"callId":call.id,"stepId":step_id,"decision":"denied","authorization":"capability disabled"},"result":result}).to_string(), "complete")?;
                    crate::history::remember_native_round(
                        &store,
                        &recorded,
                        &round_model_message,
                        &mut round_model_saved,
                    )?;
                    emit_session_event(
                        &store,
                        &conversation_id,
                        Some(&run.id),
                        Some(&step_id),
                        Some(&call.id),
                        "tool_result",
                        result.clone(),
                        false,
                    );
                    messages.push(
                        json!({"role":"tool","tool_call_id":call.id,"content":result.to_string()}),
                    );
                    continue;
                }
                if plan_mode
                    && !self.plan_approved
                    && !tool.trusted_read()
                    && !matches!(
                        call.name.as_str(),
                        "plan_submit"
                            | "ask_user"
                            | "todo_write"
                            | "todo_add"
                            | "todo_update"
                            | "finish"
                    )
                {
                    let result = crate::tool_envelope::failure_notice("plan-approval-required", "Plan mode: call plan_submit and get the plan approved before running mutating tools. Read-only tools, todos and plan_submit stay available.", true, crate::tool_envelope::SIDE_NONE, &json!({}));
                    let store = state.database()?;
                    let recorded = store.append_message(&conversation_id, "tool", &json!({"request":{"connector":tool.connector,"name":tool.tool.name,"arguments":call.arguments,"callId":call.id,"stepId":step_id,"decision":"denied","authorization":"plan approval pending"},"result":result}).to_string(), "complete")?;
                    crate::history::remember_native_round(
                        &store,
                        &recorded,
                        &round_model_message,
                        &mut round_model_saved,
                    )?;
                    emit_session_event(
                        &store,
                        &conversation_id,
                        Some(&run.id),
                        Some(&step_id),
                        Some(&call.id),
                        "tool_result",
                        result.clone(),
                        false,
                    );
                    messages.push(
                        json!({"role":"tool","tool_call_id":call.id,"content":result.to_string()}),
                    );
                    continue;
                }
                if state
                    .run_control
                    .lock()
                    .map(|control| crate::agent_run::pause_blocks_new_tool(control.pause))
                    .unwrap_or(false)
                {
                    let notice = crate::agent_run::pause_notice(false);
                    if let Some(task) = self.ledger.as_mut() {
                        let _ = task.pause_before_new_tool(&call.id, &call.name);
                        save_ledger(state, task);
                    }
                    let paused = crate::tool_envelope::failure_notice(
                        "paused",
                        notice,
                        false,
                        crate::tool_envelope::SIDE_NONE,
                        &json!({"paused":true}),
                    );
                    messages.push(
                        json!({"role":"tool","tool_call_id":call.id,"content":paused.to_string()}),
                    );
                    self.paused_exit = true;
                    settle_unconsumed_reads(
                        state,
                        plan,
                        &conversation_id,
                        run,
                        &step_id,
                        &mut self.ledger,
                        &calls,
                        &mut prefetched,
                        messages,
                        &mut turn.seq,
                    );
                    break;
                }
                let signature = crate::continuation::tool_signature(tool, &call.arguments);
                // A prefetched member is never "re-dispatched": its outcome is
                // real, so an uncertain sibling cannot retroactively suppress it.
                if !was_prefetched
                    && self
                        .blocked_calls
                        .lock()
                        .map(|set| set.contains(&signature))
                        .unwrap_or(true)
                {
                    let skipped = crate::tool_envelope::failure_notice(
                        "action-blocked",
                        "Not dispatched: this identical action was blocked by an earlier failure, denial or uncertain outcome. Inspect the original result; do not automatically repeat it.",
                        false,
                        crate::tool_envelope::SIDE_NONE,
                        &json!({}),
                    );
                    let store = state.database()?;
                    let row=store.append_message(&conversation_id,"tool",&json!({"request":{"connector":tool.connector,"name":tool.tool.name,"arguments":call.arguments,"callId":call.id,"stepId":step_id,"decision":"denied","authorization":"turn replay protection"},"result":skipped}).to_string(),"complete")?;
                    crate::history::remember_native_round(
                        &store,
                        &row,
                        &round_model_message,
                        &mut round_model_saved,
                    )?;
                    messages.push(
                        json!({"role":"tool","tool_call_id":call.id,"content":skipped.to_string()}),
                    );
                    continue;
                }
                match tool.prepare_arguments(call.arguments.clone()) {
                    Ok(arguments) => call.arguments = arguments,
                    Err(error) => {
                        let result = crate::tool_envelope::failure_notice(
                            "invalid-arguments",
                            &error,
                            false,
                            crate::tool_envelope::SIDE_NONE,
                            &json!({}),
                        );
                        let signature = crate::continuation::tool_signature(tool, &call.arguments);
                        if self.failures.record(&signature, &result) {
                            self.blocked_calls
                                .lock()
                                .map_err(|_| "Replay protection unavailable.")?
                                .insert(signature);
                        }
                        let store = state.database()?;
                        let recorded=store.append_message(&conversation_id, "tool", &json!({"request":{"connector":tool.connector,"name":tool.tool.name,"arguments":call.arguments,"callId":call.id,"stepId":step_id,"decision":"invalid"},"result":result}).to_string(), "complete")?;
                        crate::history::remember_native_round(
                            &store,
                            &recorded,
                            &round_model_message,
                            &mut round_model_saved,
                        )?;
                        emit_session_event(
                            &store,
                            &conversation_id,
                            Some(&run.id),
                            Some(&step_id),
                            Some(&call.id),
                            "tool_result",
                            result.clone(),
                            false,
                        );
                        messages.push(json!({"role":"tool","tool_call_id":call.id,"content":result.to_string()}));
                        continue;
                    }
                }
                // Permission rules are evaluated before the access mode: a
                // user's explicit rule is authoritative, and the access mode
                // only decides the calls no rule matched. Rules come from
                // persisted settings only — never from model output.
                let rule_set = crate::permission_rules::load(&*state.database()?);
                let (rule_action, rule_resource) =
                    crate::permission_rules::classify(&call.name, &call.arguments);
                let rule_decision = crate::permission_rules::decide_for_tool(
                    &rule_set,
                    plan.access_mode,
                    rule_action,
                    &rule_resource,
                    tool.trusted_read(),
                    tool.requires_approval(&call.arguments),
                );
                let automatic_reason = match rule_decision {
                    crate::permission_rules::Decision::Allow(reason) => Some(reason),
                    crate::permission_rules::Decision::Ask => None,
                    crate::permission_rules::Decision::Deny(reason) => {
                        let skipped = crate::tool_envelope::failure_notice(
                            "permission-denied",
                            &reason,
                            false,
                            crate::tool_envelope::SIDE_NONE,
                            &json!({}),
                        );
                        turn.stop_reason =
                            crate::agent_engine::termination_after_tool(turn.stop_reason, &skipped);
                        let store = state.database()?;
                        let row = store.append_message(&conversation_id, "tool", &json!({"request":{"connector":tool.connector,"name":tool.tool.name,"arguments":call.arguments,"callId":call.id,"stepId":step_id,"decision":"denied","authorization":"permission rule"},"result":skipped}).to_string(), "complete")?;
                        crate::history::remember_native_round(
                            &store,
                            &row,
                            &round_model_message,
                            &mut round_model_saved,
                        )?;
                        messages.push(json!({"role":"tool","tool_call_id":call.id,"content":skipped.to_string()}));
                        emit_session_event(
                            &store,
                            &conversation_id,
                            Some(&run.id),
                            Some(&step_id),
                            Some(&call.id),
                            "tool_result",
                            skipped.clone(),
                            false,
                        );
                        if let Ok(mut blocked) = self.blocked_calls.lock() {
                            blocked
                                .insert(crate::continuation::tool_signature(tool, &call.arguments));
                        }
                        continue;
                    }
                };
                let blocked_by_denial = tool_use_denied;
                let allow = if blocked_by_denial {
                    false
                } else if automatic_reason.is_some() {
                    true
                } else {
                    let _ = run.transition_to(crate::agent_run::RunState::AwaitingApproval);
                    let _ = state.database()?.update_run_status(
                        &run.id,
                        run.status,
                        None,
                        Some(&step_id),
                    );
                    let purpose = state
                        .database()?
                        .todos(&conversation_id)
                        .unwrap_or_default()
                        .into_iter()
                        .find(|todo| todo.status == crate::plans::IN_PROGRESS)
                        .map(|todo| todo.text);
                    let (approval_id, decision) = state.approvals.request()?;
                    let _pending = state.approvals.guard(&approval_id);
                    turn.seq += 1;
                    let sent = channel.send(ChatEvent::new(
                    run,
                    Some(&step_id),
                    turn.seq,
                    &format!("Awaiting approval: {}", tool.tool.name),
                    &assistant.id,
                    "",
                    "",
                    None,
                    Some(json!({"id":approval_id,"connector":tool.connector,"localServerName":tool.local_server_name(),"name":tool.tool.name,"arguments":call.arguments,"purpose":purpose})),
                ));
                    if sent.is_err() {
                        state.approvals.remove(&approval_id);
                        return Err("The approval interface disconnected.".into());
                    }
                    let approval_started = Instant::now();
                    let allow = tokio::select! {
                        _ = cancellation.changed() => None,
                        result = tokio::time::timeout(Duration::from_secs(600),decision) => Some(result.ok().and_then(Result::ok).unwrap_or(false)),
                    };
                    response_metrics.approval_ms = Some(
                        response_metrics.approval_ms.unwrap_or(0.0)
                            + approval_started.elapsed().as_secs_f64() * 1000.0,
                    );
                    state.approvals.remove(&approval_id);
                    turn.seq += 1;
                    channel
                        .send(ChatEvent::new(
                            run,
                            Some(&step_id),
                            turn.seq,
                            "Approval decision recorded",
                            &assistant.id,
                            "",
                            "",
                            None,
                            Some(Value::Null),
                        ))
                        .map_err(|error| error.to_string())?;
                    let Some(allow) = allow else {
                        settle_unconsumed_reads(
                            state,
                            plan,
                            &conversation_id,
                            run,
                            &step_id,
                            &mut self.ledger,
                            &calls,
                            &mut prefetched,
                            messages,
                            &mut turn.seq,
                        );
                        return Ok(crate::agent_engine::RoundControl::Complete(false));
                    };
                    allow
                };
                if !allow {
                    if let Ok(mut blocked) = self.blocked_calls.lock() {
                        blocked.insert(crate::continuation::tool_signature(tool, &call.arguments));
                    }
                }
                let budget_error = if allow && !was_prefetched {
                    crate::continuation::admit_tool(state, &run.id, &call.name, &call.arguments)
                        .err()
                } else {
                    None
                };
                let authorization = if budget_error.is_some() {
                    "turn tool-budget admission denied"
                } else if blocked_by_denial {
                    "blocked by an earlier denial in this turn"
                } else {
                    automatic_reason.unwrap_or("user approval decision")
                };
                let audit = json!({"connector":tool.connector,"localServerName":tool.local_server_name(),"name":tool.tool.name,"arguments":call.arguments,"decision":if allow&&budget_error.is_none() { "allowed" } else { "denied" },"accessMode":plan.access_mode,"authorization":authorization,"callId":call.id,"stepId":step_id,"assistantContent":round_answer});
                let row = if let Some(row) = prefetched
                    .get_mut(&call.id)
                    .and_then(|entry| entry.audit_row.take())
                {
                    state.database()?.update_message(
                        &row.id,
                        &audit.to_string(),
                        "",
                        "streaming",
                    )?;
                    row
                } else {
                    state.database()?.append_message(
                        &conversation_id,
                        "tool",
                        &audit.to_string(),
                        if allow && budget_error.is_none() {
                            "streaming"
                        } else {
                            "complete"
                        },
                    )?
                };
                turn.last_model_message_id = row.id.clone();
                crate::history::remember_native_round(
                    &*state.database()?,
                    &row,
                    &round_model_message,
                    &mut round_model_saved,
                )?;

                let _ = run.transition_to(crate::agent_run::RunState::ExecutingTools);
                let _ =
                    state
                        .database()?
                        .update_run_status(&run.id, run.status, None, Some(&step_id));
                turn.seq += 1;
                channel
                    .send(ChatEvent::new(
                        run,
                        Some(&step_id),
                        turn.seq,
                        &format!(
                            "{} {}",
                            if budget_error.is_some() {
                                "Blocked"
                            } else {
                                "Executing"
                            },
                            tool.tool.name
                        ),
                        &assistant.id,
                        "",
                        "",
                        None,
                        None,
                    ))
                    .map_err(|error| error.to_string())?;

                let tool_exec_start = Instant::now();
                if allow && budget_error.is_none() {
                    let cmd_arg = if tool.tool.name == "execute_command" {
                        call.arguments
                            .get("command")
                            .and_then(|v| v.as_str())
                            .map(String::from)
                    } else {
                        call.arguments
                            .get("code")
                            .and_then(|v| v.as_str())
                            .map(String::from)
                    };
                    let lang_arg = call
                        .arguments
                        .get("language")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    let cwd_arg = call
                        .arguments
                        .get("cwd")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    turn.seq += 1;
                    let _ = channel.send(ChatEvent {
                        run_id: Some(run.id.clone()),
                        step_id: Some(step_id.clone()),
                        seq: Some(turn.seq),
                        state: Some(run.status.as_str().to_string()),
                        activity: Some(format!("Running {}", tool.tool.name)),
                        context: None,
                        notice: None,
                        message_id: assistant.id.clone(),
                        content: String::new(),
                        reasoning: String::new(),
                        approval: None,
                        elapsed_secs: Some(0.0),
                        round_index: Some(round),
                        presentation: None,
                        discard_step: None,
                        outcome: None,
                        tool_stream: Some(ToolStreamEvent::Started {
                            tool_call_id: call.id.clone(),
                            tool_name: tool.tool.name.clone(),
                            command: cmd_arg,
                            language: lang_arg,
                            cwd: cwd_arg,
                        }),
                    });
                }

                let mut result = if let Some(error) = budget_error {
                    crate::tool_envelope::failure_notice(
                        "tool-budget-blocked",
                        &error,
                        false,
                        crate::tool_envelope::SIDE_NONE,
                        &json!({"budget":true}),
                    )
                } else if !allow {
                    crate::tool_envelope::failure_notice("denied", "The user denied this tool request. Do not repeat it without a new instruction.", false, crate::tool_envelope::SIDE_NONE, &json!({"denied":true}))
                } else {
                    let args_text = call.arguments.to_string();
                    // User pre_tool_use hooks may veto before dispatch; a block
                    // is a failure envelope, other failures are notices.
                    let hook_verdict =
                        crate::hooks::pre_use(state, &conversation_id, &call.name, &call.arguments)
                            .await;
                    for notice in &hook_verdict.notices {
                        let _ = channel.send(ChatEvent::progress(
                            &run.id,
                            &step_id,
                            &assistant.id,
                            notice,
                            None,
                        ));
                    }
                    if let Some(blocked) = hook_verdict.blocked {
                        crate::tool_envelope::failure_notice(
                            "hook-blocked",
                            &blocked,
                            false,
                            crate::tool_envelope::SIDE_NONE,
                            &json!({"hookBlocked":true}),
                        )
                    // Guard: three identical calls in a row end loudly, not in a loop.
                    } else if let Some(note) =
                        crate::sandbox::check_repetition(&self.recent_calls, &call.name, &args_text)
                    {
                        self.recent_calls.push((call.name.clone(), args_text));
                        json!({"isError":true,"message":note})
                    } else {
                        self.recent_calls.push((call.name.clone(), args_text));
                        let configured = state
                            .database()?
                            .setting::<Option<u64>>(crate::sandbox::TOOL_TIMEOUT_KEY)?;
                        let deadline =
                            crate::agent_engine::tool_deadline(tool, &call.arguments, configured);
                        let app_state: &AppState = state;
                        let mut pending_events: Vec<crate::harness::PendingEvent> = Vec::new();
                        let emit_tool_channel = channel.clone();
                        let emit_tool_call_id = call.id.clone();
                        let emit_tool_run_id = run.id.clone();
                        let emit_tool_step_id = step_id.clone();
                        let emit_tool_msg_id = assistant.id.clone();
                        let emit_tool_state = run.status.as_str().to_string();
                        let emit_tool_act = format!("Executing {}", tool.tool.name);
                        let tool_chunk_cb: crate::execution::ChunkCallback =
                            std::sync::Arc::new(move |stream_name, chunk_text| {
                                let _ = emit_tool_channel.send(ChatEvent {
                                    run_id: Some(emit_tool_run_id.clone()),
                                    step_id: Some(emit_tool_step_id.clone()),
                                    seq: Some(0),
                                    state: Some(emit_tool_state.clone()),
                                    activity: Some(emit_tool_act.clone()),
                                    context: None,
                                    notice: None,
                                    message_id: emit_tool_msg_id.clone(),
                                    content: String::new(),
                                    reasoning: String::new(),
                                    approval: None,
                                    elapsed_secs: Some(tool_exec_start.elapsed().as_secs_f64()),
                                    round_index: Some(round),
                                    presentation: None,
                                    discard_step: None,
                                    outcome: None,
                                    tool_stream: Some(ToolStreamEvent::OutputChunk {
                                        tool_call_id: emit_tool_call_id.clone(),
                                        stream: stream_name.to_string(),
                                        chunk: chunk_text.to_string(),
                                    }),
                                });
                            });
                        let catalog_name =
                            matches!(call.name.as_str(), "run_code" | "file_write" | "file_read");
                        if allow && catalog_name {
                            if let Some(task) = self.ledger.as_mut() {
                                if let Ok(store) = state.database() {
                                    begin_catalog_op(&store, task, &call.id, &call.name);
                                }
                            }
                        }
                        let catalog_db = state.db_path.clone();
                        let catalog_task = self
                            .ledger
                            .as_ref()
                            .map(|task| task.id.clone())
                            .unwrap_or_default();
                        // The outcome keeps the Elapsed variant so timeout is
                        // normalized with a typed cause, not a magic string.
                        // A prefetched group member consumes its already
                        // dispatched outcome; the durable catalog op settles
                        // here on the shared ledger in call order.
                        let outcome = if let Some(pre) = prefetched.remove(&call.id) {
                            pending_events.extend(pre.events);
                            if pre.catalog {
                                if let Some(task) = self.ledger.as_mut() {
                                    if let Ok(store) = state.database() {
                                        if let Ok(Some(fresh)) = store.research_task(&task.id) {
                                            *task = fresh;
                                        }
                                        match &pre.outcome {
                                            Some(Ok(Ok(value))) => {
                                                settle_catalog_op(&store, task, &call.id, value)
                                            }
                                            _ => {
                                                let _ = store.save_research_task(task);
                                            }
                                        }
                                    }
                                }
                            }
                            pre.outcome
                        } else {
                            tokio::select! {
                                _ = cancellation.changed() => None,
                                result = crate::agent_engine::await_tool(deadline, async {
                                    if tool.is_harness() {
                                        let emit_run = run.id.clone();
                                        let emit_step = step_id.clone();
                                        let emit_message = assistant.id.clone();
                                        let emit_channel = channel.clone();
                                        let emit: std::sync::Arc<dyn Fn(String, Option<Value>) + Send + Sync> =
                                            std::sync::Arc::new(move |activity, approval| {
                                                // Channel-only progress (turn.seq 0); durability comes from audit rows.
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
                                            access_mode: plan.access_mode,
                                            depth: 0,
                                            preset_id: preset_id.clone(),
                                            inherit_tools: &plan.tools,
                                            emit,
                                        };
                                        let catalog = matches!(call.name.as_str(), "run_code" | "file_write" | "file_read");
                                        if catalog {
                                            let config = state.database()?.execution_config()?;
                                            let path = snapshot.workspace_path.clone();
                                            let value = run_catalog_and_record(&catalog_db, &catalog_task, &path, config, &call.id, &call.name, call.arguments.clone()).await?;
                                            if let Some(task) = self.ledger.as_mut() {
                                                if let Ok(store) = state.database() {
                                                    if let Ok(Some(fresh)) = store.research_task(&task.id) {
                                                        *task = fresh;
                                                    }
                                                }
                                            }
                                            Ok(value)
                                        } else {
                                            match crate::harness::execute(&ctx, &call.name, call.arguments.clone()).await {
                                                Ok(outcome) => {
                                                    pending_events.extend(outcome.events);
                                                    Ok(outcome.value)
                                                }
                                                Err(error) => Err(error),
                                            }
                                        }
                                    } else {
                                        let mcp_ui = tool.is_mcp().then(|| {
                                            crate::mcp_input::CallUi::new(
                                                state.mcp_inputs.clone(),
                                                progress_emit(channel, &run.id, &step_id, &assistant.id),
                                            )
                                        });
                                        tool.call_with_input(call.arguments.clone(), Some(tool_chunk_cb), mcp_ui).await
                                    }
                                }) => Some(result),
                            }
                        };
                        for event in pending_events.drain(..) {
                            turn.seq += 1;
                            let _ =
                                state
                                    .database()?
                                    .append_run_event(&crate::agent_run::RunEvent {
                                        run_id: run.id.clone(),
                                        seq: turn.seq as u64,
                                        step_id: step_id.clone(),
                                        tool_call_id: Some(call.id.clone()),
                                        event_type: event.event_type,
                                        payload: event.payload,
                                        created_at: crate::store::now(),
                                    });
                        }
                        let duration_ms = tool_exec_start.elapsed().as_millis();
                        response_metrics.tools_ms = Some(
                            response_metrics.tools_ms.unwrap_or(0.0)
                                + tool_exec_start.elapsed().as_secs_f64() * 1000.0,
                        );
                        let exit_code = match &outcome {
                            Some(Ok(Ok(val))) => val
                                .get("exitCode")
                                .and_then(|v| v.as_i64())
                                .map(|c| c as i32),
                            _ => None,
                        };
                        let error_info = match &outcome {
                            Some(Ok(Ok(val))) => tool_execution_error(val),
                            Some(Ok(Err(err))) => Some(err.clone()),
                            Some(Err(_)) => Some("Tool request timed out.".into()),
                            None => Some("Cancelled".into()),
                        };
                        turn.seq += 1;
                        let _ = channel.send(ChatEvent {
                            run_id: Some(run.id.clone()),
                            step_id: Some(step_id.clone()),
                            seq: Some(turn.seq),
                            state: Some(run.status.as_str().to_string()),
                            activity: Some(format!("Finished {}", tool.tool.name)),
                            context: None,
                            notice: None,
                            message_id: assistant.id.clone(),
                            content: String::new(),
                            reasoning: String::new(),
                            approval: None,
                            elapsed_secs: Some(tool_exec_start.elapsed().as_secs_f64()),
                            round_index: Some(round),
                            presentation: None,
                            discard_step: None,
                            outcome: None,
                            tool_stream: Some(ToolStreamEvent::Finished {
                                tool_call_id: call.id.clone(),
                                exit_code,
                                duration_ms,
                                error: error_info,
                            }),
                        });
                        match outcome {
                        None => {
                            let _ = run.transition_to(crate::agent_run::RunState::Cancelled);
                            let _ = state.database()?.update_run_status(&run.id, run.status, Some("Cancelled; the action may already have changed data."), Some(&step_id));
                            state.database()?.update_message(&row.id,&json!({"request":audit,"result":"Cancelled; the action may already have changed data."}).to_string(),"","interrupted")?;
                            settle_unconsumed_reads(state, plan, &conversation_id, run, &step_id, &mut self.ledger, &calls, &mut prefetched, messages, &mut turn.seq);
                            return Ok(crate::agent_engine::RoundControl::Complete(false));
                        },
                        // Model-visible results normalize once at this
                        // boundary: backend values keep their typed data
                        // (envelopes pass through), dispatch errors become
                        // failure envelopes, and caller-side timeouts are
                        // explicitly unknown-side-effect outcomes.
                        Some(Ok(Ok(value))) => crate::tool_envelope::normalize(value, tool.envelope_family()),
                        Some(Ok(Err(error))) => crate::tool_envelope::normalize_error(&error, crate::tool_envelope::Cause::default(), tool.envelope_family()),
                        Some(Err(_)) => crate::tool_envelope::normalize_error("Tool request timed out. Its remote outcome may be unknown; do not automatically retry.", crate::tool_envelope::Cause { timeout: true, ..Default::default() }, tool.envelope_family()),
                    }
                    }
                };

                // The taste profile learns from DECISIONS the user made, not
                // from calls the access mode approved on their own. An
                // auto-approved call carries no signal about the user's
                // taste, so it is deliberately not recorded.
                if !matches!(rule_decision, crate::permission_rules::Decision::Allow(_)) {
                    let decision = if allow {
                        crate::taste::Decision::Accepted
                    } else {
                        crate::taste::Decision::Rejected
                    };
                    if let Ok(store) = state.database() {
                        let _ = crate::taste::record(
                            &store,
                            &conversation_id,
                            &tool.tool.name,
                            decision,
                            &call.arguments,
                            &result,
                        );
                    }
                }
                if matches!(
                    tool.tool.name.as_str(),
                    "weather"
                        | "get_weather"
                        | "search"
                        | "visit"
                        | "web_search"
                        | "web_open"
                        | "web_find"
                        | "web_fetch"
                        | "web_fetch_url"
                ) {
                    crate::arex::collect_evidence(&result, &mut self.research_evidence);
                }
                if plan_mode
                    && !self.plan_approved
                    && call.name == "plan_submit"
                    && result["data"]["approved"] == Value::Bool(true)
                {
                    self.plan_approved = true;
                    let submitted = call.arguments["plan"]
                        .as_str()
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    let _ = begin_plan_implementation(messages, &submitted);
                    if let Ok(store) = state.database() {
                        if store.todos(&conversation_id).unwrap_or_default().is_empty() {
                            let seeded = crate::plans::parse_plan_steps(&submitted);
                            if !seeded.is_empty() {
                                let _ = store.save_todos(&conversation_id, &seeded);
                            }
                        }
                        if let Ok(todos) = store.todos(&conversation_id) {
                            refresh_plan_focus(messages, &todos);
                        }
                    }
                }
                let signature = crate::continuation::tool_signature(tool, &call.arguments);
                if self.failures.record(&signature, &result) {
                    self.blocked_calls
                        .lock()
                        .map_err(|_| "Replay protection unavailable.")?
                        .insert(signature);
                }
                self.verification.observe(&call.name, &result);
                if let Some(summary) = self.verification.summary() {
                    if let Ok(store) = state.database() {
                        emit_session_event(
                            &store,
                            &conversation_id,
                            Some(&run.id),
                            Some(&step_id),
                            Some(&call.id),
                            "verification_status",
                            summary,
                            false,
                        );
                    }
                }
                if matches!(call.name.as_str(), "worktree_enter" | "worktree_exit")
                    && !crate::tool_envelope::failed(&result)
                {
                    if let Ok(store) = state.database() {
                        if let Ok(Some(path)) =
                            crate::workspace_ui::task_workspace(&store, &conversation_id)
                        {
                            snapshot.workspace_path = path;
                        }
                    }
                }
                let control = if tool.connector == "Harness"
                    && matches!(call.name.as_str(), "update_context" | "finish")
                {
                    Some(result.clone())
                } else {
                    None
                };
                let deliverable_missing = matches!(
                    call.name.as_str(),
                    "file_write" | "create_file" | "run_code"
                ) && result["isError"] == Value::Bool(true);
                let mut follow_up: Option<String> = None;
                let budget_snapshot = self.budgets.lock().map(|b| b.clone()).unwrap_or_default();
                match crate::continuation::continuation_after_tool(
                    plan_mode,
                    &call.name,
                    &result,
                    &budget_snapshot,
                    self.corrections_used,
                    deliverable_missing,
                ) {
                    crate::continuation::Continuation::Proceed { evidence } => {
                        if self.evidence_note.is_none() {
                            self.evidence_note = evidence;
                        }
                    }
                    crate::continuation::Continuation::CorrectOnce { reason } => {
                        self.corrections_used = self.corrections_used.saturating_add(1);
                        follow_up = Some(format!("{reason} Fix it once with the tools still available, then stop. Do not start an open-ended review."));
                    }
                    crate::continuation::Continuation::Partial { unresolved } => {
                        turn.stop_reason = Some(if budget_snapshot.reserve_final_response() {
                            crate::agent_engine::StopReason::BudgetExhausted
                        } else {
                            crate::agent_engine::StopReason::Blocked
                        });
                        let named = unresolved.join("; ");
                        turn.research_fallback =
                            Some(format!("Partial result. Unresolved: {named}"));
                        turn.finalization_retried = false;
                        follow_up = Some(format!("Budget or checklist limit reached. Answer from the results already collected and name this unresolved work: {named}. Do not call tools."));
                    }
                    crate::continuation::Continuation::Stop { message, retryable } => {
                        if !retryable {
                            if let Ok(mut set) = self.blocked_calls.lock() {
                                set.insert(crate::continuation::tool_signature(
                                    tool,
                                    &call.arguments,
                                ));
                            }
                        }
                        if result["denied"] == Value::Bool(true) {
                            if let Ok(mut blocked) = self.blocked_calls.lock() {
                                blocked.insert(crate::continuation::tool_signature(
                                    tool,
                                    &call.arguments,
                                ));
                            }
                        }
                        let _ = message;
                    }
                }

                // Image bytes never enter the audit row or the model transcript:
                // automation captures and workspace document reads both strip
                // image parts into conversation-owned artifacts that
                // add_model_images rehydrates only for vision-capable models.
                if matches!(tool.connector.as_str(), "Browser" | "Computer")
                    || (tool.connector == "Workspace"
                        && matches!(tool.tool.name.as_str(), "document_read" | "view_image"))
                {
                    if let Err(error) = crate::automation::capture_images(
                        &mut result,
                        &*state.database()?,
                        &conversation_id,
                        &run.id,
                        &tool.tool.name,
                    ) {
                        result = crate::tool_envelope::normalize_error(
                            &error,
                            crate::tool_envelope::Cause::default(),
                            tool.envelope_family(),
                        );
                    }
                }

                turn.stop_reason =
                    crate::agent_engine::termination_after_tool(turn.stop_reason, &result);

                // Excerpt size follows the tokens still free after the counted
                // input and response reserve; full results stay in artifacts.
                let catalog_recorded =
                    matches!(call.name.as_str(), "run_code" | "file_write" | "file_read") && allow;
                let excerpt_chars = crate::artifacts::excerpt_budget(
                    plan.context_length,
                    turn.input_tokens,
                    self.round_preferences.max_tokens,
                );
                let (bounded_val, maybe_artifact) = crate::artifacts::bound_tool_result(
                    result,
                    &tool.tool.name,
                    &conversation_id,
                    Some(&run.id),
                    excerpt_chars,
                );
                if let Some(artifact) = &maybe_artifact {
                    state.database()?.save_artifact(artifact)?;
                }
                let mut bounded_val = bounded_val;
                if let Some(note) = crate::hooks::post_use(
                    state,
                    &conversation_id,
                    &call.name,
                    &call.arguments,
                    &bounded_val,
                    "approved",
                )
                .await
                {
                    // Survives artifact bounding: applied after it, separate field.
                    bounded_val["hookNote"] = json!(note);
                }
                state.database()?.update_message(
                    &row.id,
                    &json!({"request":audit,"result":bounded_val}).to_string(),
                    "",
                    "complete",
                )?;
                messages.push(
                    json!({"role":"tool","tool_call_id":call.id,"content":bounded_val.to_string()}),
                );
                if !catalog_recorded {
                    if let Some(task) = self.ledger.as_mut() {
                        let store = state.database()?;
                        persist_completed_tool(&store, task, &call.name, Some(call.id.clone()))?;
                    }
                }
                if let Some(follow_up) = follow_up {
                    messages.push(json!({"role":"user","content":follow_up}));
                }
                if let Some(control) = control {
                    explicit_finish = crate::arex::apply_control(
                        &call.name,
                        &control,
                        &*state.database()?,
                        &conversation_id,
                        &run.id,
                        messages,
                    )?;
                    if explicit_finish.is_some() {
                        finish_args = Some(call.arguments.clone());
                    }
                }
                if let Ok(store) = state.database() {
                    emit_session_event(
                        &store,
                        &conversation_id,
                        Some(&run.id),
                        Some(&step_id),
                        Some(&call.id),
                        "tool_result",
                        json!({
                            "callId": call.id,
                            "name": tool.tool.name,
                            "decision": if allow { "allowed" } else { "denied" },
                            "result": bounded_val,
                            "artifactId": maybe_artifact.as_ref().map(|a| a.id.clone()),
                        }),
                        false,
                    );
                }
            }
            if plan.supports_images {
                crate::automation::add_model_images(
                    messages,
                    &*state.database()?,
                    &conversation_id,
                )?;
            }
            if let Some(final_answer) = explicit_finish {
                let finish_args = finish_args.take();
                present_research_answer(
                    state,
                    channel,
                    run,
                    &step_id,
                    &conversation_id,
                    &assistant.id,
                    &mut turn.seq,
                    answer,
                    &final_answer,
                    finish_args.as_ref(),
                    &self.research_evidence,
                )
                .await?;
                return Ok(crate::agent_engine::RoundControl::Complete(true));
            }
            if self.paused_exit {
                return Ok(crate::agent_engine::RoundControl::Complete(false));
            }
            let _ = run.transition_to(crate::agent_run::RunState::PreparingNextRound);
            let _ = state
                .database()?
                .update_run_status(&run.id, run.status, None, Some(&step_id));
            if let Ok(store) = state.database() {
                emit_session_event(
                    &store,
                    &conversation_id,
                    Some(&run.id),
                    Some(&step_id),
                    None,
                    "step_end",
                    json!({"round": round, "content": round_answer, "toolCallsCount": tool_calls_count}),
                    false,
                );
            }
            Ok::<_, String>(crate::agent_engine::RoundControl::Continue)
        })
    }
}
