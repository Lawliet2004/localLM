use super::*;
use serde_json::{json, Value};
fn note(name: &str, evidence: Value) {
    println!(
        "AUDIT_EVIDENCE {}",
        json!({"name":name,"evidence":evidence})
    );
}
fn text(text: &str) -> String {
    format!(
        "data: {}\r\n\r\ndata: [DONE]\r\n\r\n",
        json!({"choices":[{"delta":{"content":text},"finish_reason":"stop"}]})
    )
}
fn call(id: &str, name: &str, args: Value) -> String {
    format!(
        "data: {}\r\n\r\ndata: [DONE]\r\n\r\n",
        json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":args.to_string()}}]},"finish_reason":"tool_calls"}]})
    )
}
fn conversation(state: &AppState, base: String) -> String {
    let store = state.database().unwrap();
    let mut provider = store.provider("fixture-loopback").unwrap().unwrap();
    provider.base_url = base;
    store.save_provider(&provider).unwrap();
    let owner = store.create_conversation().unwrap().id;
    store
        .save_conversation_tools(
            &owner,
            &crate::store::ConversationTools {
                access_mode: crate::permissions::AccessMode::FullAccess,
                sources: vec!["__workspace".into()],
                tools: vec![],
            },
        )
        .unwrap();
    store
        .save_conversation_preset(&owner, crate::presets::CODE)
        .unwrap();
    owner
}

#[tokio::test]
async fn resume_is_bound_to_the_saved_request_and_intake_rolls_back_as_one_unit() {
    use crate::research_tasks::{ResearchTask, TaskBudgets, TaskStatus};
    let root = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(root.path());
    crate::test_support::select_fixture_model(&state, root.path());
    let owner = state.database().unwrap().create_conversation().unwrap().id;
    let store = state.database().unwrap();
    let mut task = ResearchTask::new(&owner, "Previous release", TaskBudgets::for_parent()).unwrap();
    task.status = TaskStatus::Paused;
    store.save_run(&crate::agent_run::RunRecord { id:"prior-run".into(), conversation_id:owner.clone(),
        status:crate::agent_run::RunState::Paused, model_provider:None, model_id:None,
        checkpoint:None, error:None, created_at:1, updated_at:1 }).unwrap();
    store.start_research_task(&task, "prior-run").unwrap();
    let resume = prepare_task_resume(&store, &owner, Some(&task.id)).unwrap();
    assert!(store.pending_turn(&owner).unwrap().is_none());
    let pending = store.stage_pending_turn_with_resume(&owner, "Continue", None, Some(&resume.task_id)).unwrap();
    let recovered = store.stage_pending_turn(&owner, "Continue", Some(&pending.id)).unwrap();
    assert_eq!(recovered.resume_task_id.as_deref(), Some(task.id.as_str()));
    store.discard_pending_turn(&owner, &pending.id).unwrap();
    store.connection.execute_batch("CREATE TRIGGER fail_intake BEFORE INSERT ON session_events
        WHEN NEW.event_type='turn_start' BEGIN SELECT RAISE(ABORT, 'fixture intake failure'); END;").unwrap();
    drop(store);
    let error = send_registered_message(&state, owner.clone(), "Unrelated question".into(), Some(vec![]), Some(vec![]),
        None, None, Channel::new(|_| Ok(())), None).await.unwrap_err();
    assert!(error.contains("fixture intake failure"), "{error}");
    let store = state.database().unwrap();
    let next = store.pending_turn(&owner).unwrap().unwrap();
    assert_eq!(next.content, "Unrelated question");
    assert!(next.resume_task_id.is_none());
    assert!(store.messages(&owner).unwrap().is_empty());
    assert_eq!(store.latest_run(&owner).unwrap().unwrap().id, "prior-run");
    assert!(store.session_events(&owner, None, None).unwrap().is_empty());
    assert_eq!(store.research_task(&task.id).unwrap().unwrap().status, TaskStatus::Paused);
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn goal_tool_flow_keeps_independent_release_evidence_and_rechecks_after_external_changes() {
    use crate::research_tasks::{TaskStatus, settle_turn};
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("workspace");
    std::fs::create_dir(&root).unwrap();
    let workspace = root.to_str().unwrap();
    let state = crate::test_support::app_state(directory.path());
    crate::test_support::select_fixture_model(&state, &root);
    let owner = conversation(&state, "http://127.0.0.1:1919/v1".into());
    let goal_file = "## 1. Create the release source\n## Verification commands\n```sh\ntest -f app.rs\ngrep -q release app.rs\n```\n";
    std::fs::write(root.join("GOAL.md"), goal_file).unwrap();
    let plan = assemble_turn(&state, &owner, "/goal GOAL.md", vec!["__workspace".into(), "__execution".into()], None, None, None).await.unwrap();
    let wire = request_payload(&build_turn_messages(&plan), &plan.tool_definitions, &plan.preferences, false, false, None, false);
    assert_eq!(wire["messages"][0]["role"], "system");
    assert_eq!(wire["messages"].to_string().matches("Create the release source").count(), 1);
    assert!(plan.tool_definitions.len() <= 38, "got {} tools", plan.tool_definitions.len());
    assert!(plan.tool_definitions.len() < plan.tools.len());
    let run = crate::agent_run::RunRecord { id:"goal-fixture".into(), conversation_id:owner.clone(),
        status:crate::agent_run::RunState::ExecutingTools, model_provider:None, model_id:None,
        checkpoint:None, error:None, created_at:1, updated_at:1 };
    let mut task = {
        let store = state.database().unwrap();
        store.save_run(&run).unwrap();
        start_turn_ledger(&store, &owner, "/goal GOAL.md", &run.id, &plan, None).unwrap().0
    };
    let mut ctx = crate::development_tools::tests::context(&state, &root, &owner, &plan.tools);
    ctx.run_id = run.id.clone();
    let edit = plan.tools.iter().find(|tool| tool.alias == "workspace_create_file").unwrap();
    let args = json!({"path":"app.rs","content":"// release\n"});
    let result = edit.call(args.clone()).await.unwrap();
    let result = crate::tool_envelope::normalize(result, edit.envelope_family());
    assert!(!crate::tool_envelope::failed(&result), "{result}");
    task.context.verification.observe(&edit.alias, &args, &result, true);
    let audit_id = {
        let store = state.database().unwrap();
        store.save_research_task(&task).unwrap();
        store.insert_message_row(&owner, "tool", &json!({"request":{"runId":run.id,"name":edit.tool.name,"decision":"allowed"},"result":result}).to_string(), "complete", crate::store::now()).unwrap().id
    };
    let marked = crate::harness::execute(&ctx, "goal_update", json!({"id":"G1","state":"passed","note":"Created app.rs","audit_id":audit_id})).await.unwrap();
    assert_eq!(marked.value["requirements"][0]["state"], "passed");
    task = state.database().unwrap().research_task(&task.id).unwrap().unwrap();
    let args = json!({"command":"test -f app.rs","mode":"workspace","yield_ms":0});
    let launch = crate::harness::execute(&ctx, "verification_run", args.clone()).await.unwrap().value;
    task.context.verification.observe("verification_run", &args, &launch, true);
    let report_args = json!({"evidence_id":launch["evidenceId"],"wait_ms":1000});
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let report = crate::harness::execute(&ctx, "verification_report", report_args.clone()).await.unwrap().value;
        task.context.verification.observe("verification_report", &report_args, &report, true);
        if task.context.verification.evidence_for("test -f app.rs").is_some() { break; }
        assert!(tokio::time::Instant::now() < deadline, "{report}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let store = state.database().unwrap();
    store.save_research_task(&task).unwrap();
    let pending = task.context.goal.as_ref().unwrap().pending(&task.context.verification);
    assert_eq!(pending.len(), 1);
    assert!(pending[0].contains("grep -q release app.rs"));
    settle_turn(&store, &task.id, crate::agent_run::RunState::Paused, true, pending).unwrap();
    let mut saved = store.research_task(&task.id).unwrap().unwrap();
    assert_eq!(saved.status, TaskStatus::BudgetExhausted);
    saved.context.verification.refresh_goal_evidence(&store, &owner, workspace);
    assert!(saved.context.verification.evidence_for("test -f app.rs").is_some());
    std::fs::write(root.join("app.rs"), "// external edit\n").unwrap();
    saved.context.verification.refresh_goal_evidence(&store, &owner, workspace);
    assert_eq!(saved.context.goal.as_ref().unwrap().pending(&saved.context.verification).len(), 2);
}

async fn assert_rejects_write_round(format: &str, wire: String, expected_error: &str) {
    let root = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(root.path());
    crate::test_support::select_fixture_model(&state, root.path());
    let (base, requests, server) = crate::test_support::scripted_sse(vec![wire]).await;
    let owner = conversation(&state, base);
    {
        let store = state.database().unwrap();
        let mut provider = store.provider("fixture-loopback").unwrap().unwrap();
        provider.api_format = format.into();
        if format == crate::providers::CLAUDE_MESSAGES {
            provider.has_api_key = true;
            state.daytona_vault.save(&crate::providers::credential_id(&provider.id), b"fixture-only").unwrap();
        }
        store.save_provider(&provider).unwrap();
    }
    let outcome = send_registered_message(
        &state, owner.clone(), "Create unexpected.txt.".into(),
        Some(vec!["__workspace".into()]), Some(vec![]), None, None,
        Channel::new(|_| Ok(())), None,
    ).await;
    server.await.unwrap();
    assert!(!root.path().join("unexpected.txt").exists(), "{outcome:?}");
    let error = outcome.unwrap_err();
    assert!(error.contains(expected_error), "{error}");
    let route = match format {
        crate::providers::CLAUDE_MESSAGES => "POST /v1/messages ",
        crate::providers::OPENAI_RESPONSES => "POST /v1/responses ",
        _ => "POST /v1/chat/completions ",
    };
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(requests[0].headers.starts_with(route));
    let messages = state.database().unwrap().messages(&owner).unwrap();
    assert!(messages.iter().all(|message| message.role != "tool"));
    assert!(messages.iter().any(|message| message.role == "assistant" && message.status == "error"));
}

fn sse_frames(events: impl IntoIterator<Item = Value>) -> String {
    events.into_iter().map(|event| format!("data: {event}\r\n\r\n")).collect()
}

#[tokio::test]
async fn registered_turn_rejects_native_claude_write_without_a_closed_tool_block() {
    let wire = sse_frames([
        json!({"type":"content_block_start","index":0,"content_block":{
            "type":"tool_use","id":"write-unclosed","name":"workspace_create_file","input":{}
        }}),
        json!({"type":"content_block_delta","index":0,"delta":{
            "type":"input_json_delta","partial_json":"{\"path\":\"unexpected.txt\",\"content\":\"fixture\"}"
        }}),
        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"}}),
        json!({"type":"message_stop"}),
    ]);
    assert_rejects_write_round(crate::providers::CLAUDE_MESSAGES, wire, "before closing a tool block").await;
}

#[tokio::test]
async fn registered_turn_rejects_fragments_from_a_second_compatible_choice() {
    let mut wire = sse_frames([
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"write","type":"function",
            "function":{"name":"workspace_create_file","arguments":r#"{"path":"unexpected.txt","#}}]}}]}),
        json!({"choices":[{"index":1,"delta":{"tool_calls":[{"index":0,
            "function":{"arguments":r#""content":"combined"}"#}}]},"finish_reason":"tool_calls"}]}),
    ]);
    wire.push_str("data: [DONE]\r\n\r\n");
    assert_rejects_write_round(crate::providers::OPENAI_CHAT_COMPLETIONS, wire, "single model choice").await;
}

#[tokio::test]
async fn registered_turn_rejects_native_claude_write_with_unclosed_thinking() {
    let wire = sse_frames([
        json!({"type":"content_block_start","index":0,"content_block":{
            "type":"thinking","thinking":"Checking ownership","signature":""
        }}),
        json!({"type":"content_block_start","index":1,"content_block":{
            "type":"tool_use","id":"write","name":"workspace_create_file","input":{
                "path":"unexpected.txt","content":"fixture"
            }
        }}),
        json!({"type":"content_block_stop","index":1}),
        json!({"type":"message_delta","delta":{"stop_reason":"tool_use"}}),
        json!({"type":"message_stop"}),
    ]);
    assert_rejects_write_round(crate::providers::CLAUDE_MESSAGES, wire, "before closing a content block").await;
}

#[tokio::test]
async fn registered_turn_rejects_responses_refusal_relabelled_as_output_text() {
    let message = json!({"type":"message","id":"msg","role":"assistant","status":"completed",
        "content":[{"type":"output_text","text":"I cannot make that change.","annotations":[]}]});
    let write = json!({"type":"function_call","id":"fc-write","call_id":"call-write",
        "name":"workspace_create_file","status":"completed",
        "arguments":r#"{"path":"unexpected.txt","content":"fixture"}"#});
    let wire = sse_frames([
        json!({"type":"response.output_item.added","output_index":0,
            "item":{"type":"message","id":"msg","role":"assistant","status":"in_progress","content":[]}}),
        json!({"type":"response.refusal.delta","output_index":0,"item_id":"msg","content_index":0,
            "delta":"I cannot make that change."}),
        json!({"type":"response.completed","response":{"status":"completed","output":[message,write]}}),
    ]);
    assert_rejects_write_round(crate::providers::OPENAI_RESPONSES, wire, "text part changed kind").await;
}

#[tokio::test]
async fn registered_turn_rejects_responses_arguments_extended_after_done() {
    let mut write = json!({"type":"function_call","id":"fc-write","call_id":"call-write",
        "name":"workspace_create_file","status":"in_progress","arguments":""});
    let start = write.clone();
    let prefix = r#"{"path":"unexpected.txt","#;
    let suffix = r#""content":"late argument"}"#;
    write["status"] = json!("completed");
    write["arguments"] = json!(format!("{prefix}{suffix}"));
    let wire = sse_frames([
        json!({"type":"response.output_item.added","output_index":0,"item":start}),
        json!({"type":"response.function_call_arguments.done","output_index":0,"item_id":"fc-write",
            "arguments":prefix}),
        json!({"type":"response.function_call_arguments.delta","output_index":0,"item_id":"fc-write",
            "delta":suffix}),
        json!({"type":"response.completed","response":{"status":"completed","output":[write]}}),
    ]);
    assert_rejects_write_round(crate::providers::OPENAI_RESPONSES, wire, "arguments changed after completion").await;
}

#[tokio::test]
async fn registered_chat_completions_turn_displays_and_saves_a_refusal() {
    let root = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(root.path());
    crate::test_support::select_fixture_model(&state, root.path());
    let wire = concat!(
        "data: {\"choices\":[{\"delta\":{\"refusal\":\"I cannot \"}}]}\r\n\r\n",
        "data: {\"choices\":[{\"delta\":{\"refusal\":\"fulfil that request.\"},\"finish_reason\":\"stop\"}]}\r\n\r\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":6}}\r\n\r\n",
        "data: [DONE]\r\n\r\n"
    );
    let (base, requests, server) = crate::test_support::scripted_sse(vec![wire.into()]).await;
    let owner = conversation(&state, base);
    send_registered_message(
        &state, owner.clone(), "Make a decision.".into(), Some(vec![]), Some(vec![]),
        None, None, Channel::new(|_| Ok(())), None,
    ).await.unwrap();
    server.await.unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1);
    let store = state.database().unwrap();
    let messages = store.messages(&owner).unwrap();
    let reply = messages.iter().find(|message| message.role == "assistant").unwrap();
    assert_eq!(reply.status, "complete");
    assert_eq!(reply.content, "I cannot fulfil that request.");
}

#[tokio::test]
async fn productive_turn_crosses_the_round_budget_and_finishes_in_the_same_task() {
    let root = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(root.path());
    crate::test_support::select_fixture_model(&state, root.path());
    let mut responses = Vec::new();
    for index in 0..65 {
        let path = format!("file-{index}.txt");
        std::fs::write(root.path().join(&path), "fixture").unwrap();
        responses.push(call(
            &format!("read-{index}"),
            "workspace_read_file",
            json!({"path":path}),
        ));
    }
    responses.push(text("I reviewed all 65 files."));
    let (base, requests, server) = crate::test_support::scripted_sse(responses).await;
    let owner = conversation(&state, base);
    send_registered_message(
        &state,
        owner.clone(),
        "Inspect all 65 numbered files and report what you found.".into(),
        Some(vec!["__workspace".into()]),
        Some(vec![]),
        None,
        None,
        Channel::new(|_| Ok(())),
        None,
    )
    .await
    .unwrap();
    server.await.unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 66);
    assert_ne!(requests[64].body["tool_choice"], "none");
    assert_eq!(requests[0].body["tools"], requests[64].body["tools"]);
    assert_eq!(requests[0].body["messages"][0], requests[64].body["messages"][0]);
    assert!(requests[64].body["messages"].as_array().unwrap().iter()
        .any(|message| message["tool_call_id"] == "read-63"));
    let store = state.database().unwrap();
    let events = store.session_events(&owner, None, Some(5000)).unwrap();
    let end = events
        .iter()
        .rev()
        .find(|e| e.event_type == "turn_end")
        .unwrap();
    let continued = events.iter().find(|event| event.event_type == "work_segment_continued").unwrap();
    assert_eq!(continued.payload, json!({"segment":2,"reason":"model_rounds"}));
    assert_eq!(continued.run_id, end.run_id);
    assert_eq!(end.payload["partial"], false);
    assert_eq!(end.payload["taskStatus"], "completed");
    assert!(end.payload["resumeTaskId"].is_null());
    assert_eq!(events.iter().filter(|event| event.event_type == "turn_start").count(), 1);
    assert_eq!(
        store
            .messages(&owner)
            .unwrap()
            .iter()
            .filter(|m| m.role == "tool")
            .count(),
        65
    );
    note(
        "parent_segment_continuation",
        json!({"modelRequests":requests.len(),"toolCalls":65,"partial":end.payload["partial"],"stableToolSchemas":true}),
    );
}

#[tokio::test]
async fn a_segment_without_progress_preserves_the_task_for_resume() {
    use crate::research_tasks::{ResearchTask, TaskBudgets, TaskStatus};
    let root = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(root.path());
    crate::test_support::select_fixture_model(&state, root.path());
    let (base, requests, server) = crate::test_support::scripted_sse(vec![
        call("missing-1", "workspace_read_file", json!({"path":"missing-first.txt"})),
        call("missing-2", "workspace_read_file", json!({"path":"missing-second.txt"})),
        text("Both files are missing. Their contents are still needed."),
    ]).await;
    let owner = conversation(&state, base);
    let mut task = ResearchTask::new(&owner, "Read both files.",
        TaskBudgets { max_rounds: 2, ..TaskBudgets::for_parent() }).unwrap();
    task.status = TaskStatus::Paused;
    state.database().unwrap().save_research_task(&task).unwrap();
    send_registered_turn(&state, owner.clone(), "Continue from the saved checkpoint.".into(),
        Some(vec!["__workspace".into()]), Some(vec![]), None, None, Channel::new(|_| Ok(())),
        None, None, Some(task.id.clone())).await.unwrap();
    server.await.unwrap();

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[2].body["tool_choice"], "none");
    let store = state.database().unwrap();
    let events = store.session_events(&owner, None, None).unwrap();
    assert!(!events.iter().any(|event| event.event_type == "work_segment_continued"));
    let end = events.last().unwrap();
    assert_eq!(end.payload["stopReason"], "budget_exhausted");
    assert_eq!(end.payload["resumeTaskId"], task.id);
    assert_eq!(end.payload["partial"], true);
}

#[tokio::test]
async fn interrupted_stream_retries_only_the_model_round() {
    let root = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(root.path());
    crate::test_support::select_fixture_model(&state, root.path());
    std::fs::write(root.path().join("recorded.txt"), "settled result").unwrap();
    let draft = json!({"choices":[{"delta":{"content":"Interrupted draft",
        "tool_calls":[{"index":0,"id":"unfinished-write","type":"function","function":{
            "name":"workspace_create_file","arguments":r#"{"path":"unexpected.txt","content":"partial"}"#
        }}]},"finish_reason":"tool_calls"}]});
    let (base, requests, server) = crate::test_support::scripted_sse(vec![
        call("read", "workspace_read_file", json!({"path":"recorded.txt"})),
        format!("data: {draft}\r\n\r\n"),
        text("Finished using the recorded result."),
    ]).await;
    let owner = conversation(&state, base);
    let outgoing = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let channel_events = outgoing.clone();
    let channel = Channel::new(move |body| {
        if let tauri::ipc::InvokeResponseBody::Json(body) = body {
            channel_events.lock().unwrap().push(serde_json::from_str(&body).unwrap());
        }
        Ok(())
    });
    send_registered_message(&state, owner.clone(), "Read recorded.txt.".into(),
        Some(vec!["__workspace".into()]), Some(vec![]), None, None, channel, None).await.unwrap();
    server.await.unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests[1].body, requests[2].body);
    assert!(requests[2].body.to_string().contains("settled result"));
    assert!(!root.path().join("unexpected.txt").exists());
    let store = state.database().unwrap();
    let rows = store.messages(&owner).unwrap();
    assert_eq!(rows.iter().filter(|row| row.role == "tool").count(), 1);
    let reply = rows.iter().find(|row| row.role == "assistant").unwrap();
    assert_eq!(reply.content, "Finished using the recorded result.");
    let events = store.session_events(&owner, None, None).unwrap();
    assert!(events.iter().any(|event| event.event_type == "model_round_retry"
        && event.payload["reason"] == "stream_interrupted"));
    assert!(outgoing.lock().unwrap().iter().any(|event| event["replaceDraft"] == true && event["content"] == ""));
}

#[cfg(unix)]
#[tokio::test]
async fn a_finishing_model_waits_for_its_running_check() {
    let root = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(root.path());
    crate::test_support::select_fixture_model(&state, root.path());
    let (base, requests, server) = crate::test_support::scripted_sse(vec![
        call("start-check", "check_run", json!({"command":"while [ ! -f release ]; do sleep 0.1; done; echo finished", "yield_ms":0,"timeout_seconds":10})),
        text("The check is done."),
        text("The command has finished and the structured report remains to be collected."),
    ]).await;
    let owner = conversation(&state, base);
    let waiting = std::sync::Arc::new(tokio::sync::Notify::new());
    let notify = waiting.clone();
    let channel = Channel::new(move |body| {
        if let tauri::ipc::InvokeResponseBody::Json(body) = body {
            let event: Value = serde_json::from_str(&body).unwrap();
            if event["activity"] == "Waiting for checks to finish…" { notify.notify_one(); }
        }
        Ok(())
    });
    let request = send_registered_message(&state, owner.clone(), "Run the build check.".into(),
        Some(vec!["__workspace".into(), "__execution".into()]), Some(vec![]),
        None, None, channel, None);
    let drive_check = async {
        tokio::time::timeout(Duration::from_secs(5), waiting.notified()).await.unwrap();
        assert_eq!(requests.lock().unwrap().len(), 2);
        std::fs::write(root.path().join("release"), "").unwrap();
    };
    let (result, ()) = tokio::join!(Box::pin(request), drive_check);
    result.unwrap();
    server.await.unwrap();
    let requests = requests.lock().unwrap();
    assert!(requests[2].body.to_string().contains("Collect the terminal check_report"));
    let store = state.database().unwrap();
    let run = store.latest_run(&owner).unwrap().unwrap();
    let events = store.session_events(&owner, None, None).unwrap();
    assert_eq!(run.status, crate::agent_run::RunState::Paused);
    let end = events.last().unwrap();
    assert_eq!(end.event_type, "turn_end");
    assert_eq!(end.payload["verification"]["status"], "unverified");
}

#[tokio::test]
async fn pending_task_graph_work_is_included_in_partial_outcome() {
    let root = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(root.path());
    crate::test_support::select_fixture_model(&state, root.path());
    let (base, requests, server) = crate::test_support::scripted_sse(vec![
        call(
            "create-task",
            "task_create",
            json!({"title":"Implement the requested feature"}),
        ),
        text("I have recorded the task."),
        text("The implementation is still pending."),
    ])
    .await;
    let owner = conversation(&state, base);
    send_registered_message(
        &state,
        owner.clone(),
        "Track the feature in the task graph and implement it.".into(),
        Some(vec!["__workspace".into()]),
        Some(vec![]),
        None,
        None,
        Channel::new(|_| Ok(())),
        None,
    )
    .await
    .unwrap();
    server.await.unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[2].body.to_string().contains("active checklist has unfinished work"));
    let store = state.database().unwrap();
    let pending: i64 = store
        .connection
        .query_row(
            "SELECT COUNT(*) FROM tasks WHERE conversation_id=?1 AND state='pending'",
            [&owner],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(pending, 1);
    let events = store.session_events(&owner, None, None).unwrap();
    let end = events
        .iter()
        .rev()
        .find(|e| e.event_type == "turn_end")
        .unwrap();
    assert_eq!(end.payload["partial"], true);
    assert_eq!(end.payload["unfinishedTasks"].as_array().unwrap().len(), 1);
    note(
        "task_graph_completion",
        json!({"pendingGraphTasks":pending,"turnEndPartial":end.payload["partial"],"unfinishedTodos":end.payload["unfinishedTodos"],"delivered":end.payload["delivered"]}),
    );
}

#[tokio::test]
async fn completed_turn_research_stays_completed_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(root.path());
    crate::test_support::select_fixture_model(&state, root.path());
    let (base, _, server) = crate::test_support::scripted_sse(vec![text("4")]).await;
    let owner = conversation(&state, base);
    send_registered_message(
        &state,
        owner.clone(),
        "What is 2 + 2?".into(),
        Some(vec![]),
        Some(vec![]),
        None,
        None,
        Channel::new(|_| Ok(())),
        None,
    )
    .await
    .unwrap();
    server.await.unwrap();
    let store = state.database().unwrap();
    assert!(store
        .messages(&owner)
        .unwrap()
        .iter()
        .filter(|m| m.role == "assistant")
        .all(|m| m.status == "complete"));
    let before = store.latest_research_task(&owner).unwrap().unwrap();
    assert_eq!(before.status, crate::research_tasks::TaskStatus::Completed);
    store.recover_interrupted().unwrap();
    crate::research_tasks::recover_interrupted(&store).unwrap();
    let after = store.latest_research_task(&owner).unwrap().unwrap();
    assert_eq!(after.status, crate::research_tasks::TaskStatus::Completed);
    note(
        "completed_turn_research_ledger",
        json!({"answerCompleted":true,"ledgerBeforeRestart":before.status,"ledgerAfterRestart":after.status,"pending":after.pending_work}),
    );
}

#[tokio::test]
async fn resumed_turn_reuses_and_settles_its_original_ledger() {
    use crate::research_tasks::{ResearchTask, TaskBudgets, TaskStatus};
    for status in [TaskStatus::Paused, TaskStatus::BudgetExhausted] {
        let root = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(root.path());
        crate::test_support::select_fixture_model(&state, root.path());
        let (base, requests, server) =
            crate::test_support::scripted_sse(vec![text("Finished the remaining work.")]).await;
        let owner = conversation(&state, base);
        let id = {
            let store = state.database().unwrap();
            store
                .save_run(&crate::agent_run::RunRecord {
                    id: "prior-budget-run".into(),
                    conversation_id: owner.clone(),
                    status: crate::agent_run::RunState::Completed,
                    model_provider: None,
                    model_id: None,
                    checkpoint: None,
                    error: None,
                    created_at: 1,
                    updated_at: 2,
                })
                .unwrap();
            let mut task = ResearchTask::new(
                &owner,
                "Original request with RESUME_SENTINEL.",
                TaskBudgets::default(),
            )
            .unwrap();
            task.status = status.clone();
            task.pending_work = vec!["Finish the remaining work".into()];
            store
                .start_research_task(&task, "prior-budget-run")
                .unwrap();
            prepare_task_resume(&store, &owner, Some(&task.id)).unwrap();
            assert_eq!(
                store.research_task(&task.id).unwrap().unwrap().status,
                status,
                "staging must retain a durable pause"
            );
            task.id
        };
        send_registered_turn(
            &state,
            owner.clone(),
            "Continue from the saved checkpoint.".into(),
            Some(vec![]),
            Some(vec![]),
            None,
            None,
            Channel::new(|_| Ok(())),
            None,
            None,
            Some(id.clone()),
        )
        .await
        .unwrap();
        server.await.unwrap();
        assert!(requests.lock().unwrap()[0]
            .body
            .to_string()
            .contains("RESUME_SENTINEL"));
        let store = state.database().unwrap();
        let task = store.research_task(&id).unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Completed);
        assert!(task.pending_work.is_empty());
        assert_ne!(
            store.research_task_run(&task).unwrap().unwrap().id,
            "prior-budget-run"
        );
        let count: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM research_tasks WHERE conversation_id=?1",
                [&owner],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "resuming must not leave a second phantom ledger");
    }
}

#[tokio::test]
async fn three_day_old_checkpoint_preserves_completed_write_and_unknown_outcome() {
    use crate::research_tasks::{OperationStatus, ResearchTask, TaskBudgets, TaskStatus};
    let root = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(root.path());
    crate::test_support::select_fixture_model(&state, root.path());
    let owner = conversation(&state, "http://127.0.0.1:1919/v1".into());
    let output = root.path().join("completed.txt");
    let completed = json!({"path":"completed.txt","content":"COMPLETE_ONCE_SENTINEL"});
    let uncertain = json!({"path":"unacknowledged.txt","content":"do not create on resume"});
    let raw = crate::workspace::Workspace::open(root.path().to_str().unwrap()).unwrap()
        .call("create_file", completed.clone()).unwrap();
    let completed_result = crate::tool_envelope::normalize(raw, crate::tool_envelope::Family::Workspace);
    let modified = std::fs::metadata(&output).unwrap().modified().unwrap();
    let aged = crate::store::now() - 3 * 24 * 60 * 60 * 1000;
    let id = {
        let store = state.database().unwrap();
        store.save_run(&crate::agent_run::RunRecord {
            id: "aged-paused-run".into(), conversation_id: owner.clone(), status: crate::agent_run::RunState::ExecutingTools,
            model_provider: None, model_id: None, checkpoint: None, error: None, created_at: aged, updated_at: aged,
        }).unwrap();
        let mut task = ResearchTask::new(&owner, "Preserve completed.txt; review the uncertain external action without repeating it.", TaskBudgets::default()).unwrap();
        task.checkpoint_operation("file_write", OperationStatus::Completed, Some("Created completed.txt with COMPLETE_ONCE_SENTINEL".into()));
        task.checkpoint_operation("external_action", OperationStatus::Unknown, None);
        task.status = TaskStatus::Paused;
        task.pending_work = vec!["Review the external action; do not repeat it automatically".into()];
        task.created_at = aged;
        task.updated_at = aged;
        for op in &mut task.operations { op.updated_at = aged; }
        store.start_research_task(&task, "aged-paused-run").unwrap();
        let prior_request = store.append_message(&owner, "user", &task.question, "complete").unwrap();
        store.update_message_created_at(&prior_request.id, aged).unwrap();
        let request = |id: &str, args: &Value| json!({"connector":"Workspace","name":"create_file",
            "arguments":args,"decision":"allowed","runId":"aged-paused-run","stepId":"step-0","callId":id
        });
        let settled = store.append_message(&owner, "tool", &json!({
            "request":request("completed-write", &completed),"result":completed_result
        }).to_string(), "complete").unwrap();
        store.update_message_created_at(&settled.id, aged + 1).unwrap();
        let unacknowledged = store.append_message(&owner, "tool",
            &request("uncertain-write", &uncertain).to_string(), "streaming").unwrap();
        store.update_message_created_at(&unacknowledged.id, aged + 2).unwrap();
        task.id
    };
    drop(state);
    let state = crate::test_support::app_state(root.path());
    {
        let store = state.database().unwrap();
        store.recover_interrupted().unwrap();
        crate::research_tasks::recover_interrupted(&store).unwrap();
        assert_eq!(store.run("aged-paused-run").unwrap().unwrap().status, crate::agent_run::RunState::OutcomeUnknown);
    }
    for segment in 0..2 {
        let (base, requests, server) = crate::test_support::scripted_sse(vec![
            call(&format!("repeat-completed-{segment}"), "workspace_create_file", completed.clone()),
            call(&format!("repeat-uncertain-{segment}"), "workspace_create_file", uncertain.clone()),
            call(&format!("inspect-completed-{segment}"), "workspace_read_file", json!({"path":"completed.txt"})),
            text("The saved file is intact. The external action still needs review; I did not repeat it."),
        ]).await;
        {
            let store = state.database().unwrap();
            let staged = prepare_task_resume(&store, &owner, Some(&id)).unwrap();
            assert!(staged.completed.len() >= 1);
            let mut provider = store.provider("fixture-loopback").unwrap().unwrap();
            provider.base_url = base;
            store.save_provider(&provider).unwrap();
        }
        send_registered_turn(&state, owner.clone(), "Continue from the saved checkpoint.".into(), Some(vec!["__workspace".into()]), Some(vec![]), None, None, Channel::new(|_| Ok(())), None, None, Some(id.clone())).await.unwrap();
        server.await.unwrap();
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 4);
        assert!(requests[0].body["messages"].to_string().contains("Paused checkpoint."));
        for reply in &requests[1..=2] {
            assert!(reply.body["messages"].as_array().unwrap().last().unwrap()["content"]
                .as_str().unwrap().contains("uncertain-outcome-not-repeated"));
        }
    }
    let store = state.database().unwrap();
    let task = store.research_task(&id).unwrap().unwrap();
    assert_eq!(task.status, TaskStatus::Paused);
    assert_eq!(task.operations.iter().filter(|op| op.kind == "file_write").count(), 1);
    assert!(task.operations.iter().any(|op| op.status == OperationStatus::Unknown));
    assert!(task.pending_work.iter().any(|work| work.contains("outcome may be unknown")));
    assert_eq!(std::fs::read_to_string(&output).unwrap(), "COMPLETE_ONCE_SENTINEL");
    assert_eq!(std::fs::metadata(&output).unwrap().modified().unwrap(), modified);
    assert!(!root.path().join("unacknowledged.txt").exists());
    assert!(store.setting::<String>(&format!("pending_resume:{owner}")).unwrap().is_empty());
    assert_eq!(task.operations.iter().filter(|op| op.id.starts_with("resume-audit-")).count(), 1);
    let events = store.session_events(&owner, None, None).unwrap();
    let end = events.iter().rev().find(|event| event.event_type == "turn_end").unwrap();
    assert_eq!(end.payload["partial"], true, "unknown operations must remain visible in the turn outcome");
    note("aged_checkpoint_resume", json!({"checkpointAgeDays":3,"realElapsedDays":0,"completedWritePreserved":true,"unknownOutcomeRetained":true,"resumedSegments":2,"blockedExactWriteAttempts":4,"partial":end.payload["partial"]}));
}

#[tokio::test]
async fn read_only_ptc_saves_a_model_round_without_false_verification() {
    let mut observations = Vec::new();
    for batched in [false, true] {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("AGENTS.md"),
            "MUST preserve PARENT_RULE_SENTINEL.",
        )
        .unwrap();
        std::fs::write(root.path().join("a.txt"), "alpha").unwrap();
        std::fs::write(root.path().join("b.txt"), "beta").unwrap();
        let state = crate::test_support::app_state(root.path());
        crate::test_support::select_fixture_model(&state, root.path());
        let responses = if batched {
            vec![
                call(
                    "batch",
                    "ptc_run",
                    json!({"steps":[{"tool":"workspace_read_file","args":{"path":"a.txt"}},{"tool":"workspace_read_file","args":{"path":"b.txt"}}]}),
                ),
                text("alpha beta"),
            ]
        } else {
            vec![
                call("read-a", "workspace_read_file", json!({"path":"a.txt"})),
                call("read-b", "workspace_read_file", json!({"path":"b.txt"})),
                text("alpha beta"),
            ]
        };
        let (base, requests, server) = crate::test_support::scripted_sse(responses).await;
        let owner = conversation(&state, base);
        send_registered_message(
            &state,
            owner.clone(),
            "Read a.txt and b.txt and report both values.".into(),
            Some(vec!["__workspace".into(), "__execution".into()]),
            Some(vec![]),
            None,
            None,
            Channel::new(|_| Ok(())),
            None,
        )
        .await
        .unwrap();
        server.await.unwrap();
        let requests = requests.lock().unwrap();
        assert!(requests[0].body["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("PARENT_RULE_SENTINEL"));
        let last = requests.last().unwrap().body["messages"].to_string();
        assert!(
            last.contains("alpha") && last.contains("beta"),
            "batched={batched}; messages={}",
            serde_json::to_string(
                &requests.last().unwrap().body["messages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|m| m["role"] == "tool")
                    .collect::<Vec<_>>()
            )
            .unwrap()
        );
        let messages = state.database().unwrap().messages(&owner).unwrap();
        let answer = messages.iter().find(|m| m.role == "assistant").unwrap();
        assert_eq!(answer.content, "alpha beta");
        let events = state
            .database()
            .unwrap()
            .session_events(&owner, None, None)
            .unwrap();
        let nudges = events
            .iter()
            .filter(|e| e.event_type == "verify_before_finish")
            .count();
        assert_eq!(nudges, 0);
        assert_eq!(
            events
                .iter()
                .rev()
                .find(|e| e.event_type == "turn_end")
                .unwrap()
                .payload["partial"],
            false
        );
        let end = events
            .iter()
            .rev()
            .find(|e| e.event_type == "turn_end")
            .unwrap();
        observations.push(json!({"batched":batched,"modelRequests":requests.len(),"outerToolCalls":if batched{1}else{2},"underlyingReads":2,"durableToolAuditRows":messages.iter().filter(|m|m.role=="tool").count(),"answer":answer.content,"parentRulesPresent":true,"verificationNudges":nudges,"verification":end.payload["verification"],"partial":end.payload["partial"]}));
    }
    assert_eq!(observations[0]["modelRequests"], 3);
    assert_eq!(observations[1]["modelRequests"], 2);
    note(
        "scripted_ptc_equivalence",
        json!({"cases":observations,"limitation":"Scripted model chooses the plan. This tests orchestration and result parity, not real-model task accuracy or comparative harness performance."}),
    );
}

#[tokio::test]
async fn provider_context_overflow_auto_compacts_and_retries_cleanly() {
    let root = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(root.path());
    crate::test_support::select_fixture_model(&state, root.path());
    std::fs::write(root.path().join("big.txt"), "payload-line\n".repeat(400)).unwrap();
    let overflow_event = format!(
        "data: {}\r\n\r\n",
        json!({"error":{"code":"context_length_exceeded","message":"This model's maximum context length is 128000 tokens."}})
    );
    let (base, requests, server) = crate::test_support::scripted_sse(vec![
        call("read-1", "workspace_read_file", json!({"path":"big.txt"})),
        call("read-2", "workspace_read_file", json!({"path":"big.txt"})),
        call("read-3", "workspace_read_file", json!({"path":"big.txt"})),
        overflow_event,
        text("Recovered cleanly after automatic emergency compaction."),
    ])
    .await;
    let owner = conversation(&state, base);
    send_registered_message(
        &state,
        owner.clone(),
        "Read big.txt and summarize.".into(),
        Some(vec!["__workspace".into()]),
        Some(vec![]),
        None,
        None,
        Channel::new(|_| Ok(())),
        None,
    )
    .await
    .unwrap();
    server.await.unwrap();
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 5);
    assert!(
        serde_json::to_vec(&requests[4].body).unwrap().len()
            < serde_json::to_vec(&requests[3].body).unwrap().len(),
        "retry after context overflow must carry a compacted smaller payload"
    );
    let store = state.database().unwrap();
    let reply = store
        .messages(&owner)
        .unwrap()
        .into_iter()
        .find(|m| m.role == "assistant")
        .unwrap();
    assert_eq!(
        reply.content,
        "Recovered cleanly after automatic emergency compaction."
    );
    let events = store.session_events(&owner, None, None).unwrap();
    assert!(events.iter().any(|e| e.event_type == "compaction"
        && e.payload["reason"] == "provider_context_overflow"));
}

#[test]
fn transient_and_overflow_error_classifiers_distinguish_permanent_failures() {
    assert!(is_context_overflow_error("context_length_exceeded: too many tokens"));
    assert!(is_context_overflow_error("Prompt is too long for this model."));
    assert!(!is_context_overflow_error("HTTP 401 unauthorized"));

    assert!(is_transient_stream_error("Provider returned HTTP 429 rate limit exceeded"));
    assert!(is_transient_stream_error("Provider returned HTTP 529 overloaded"));
    assert!(is_transient_stream_error("error sending request: connection reset by peer"));
    assert!(!is_transient_stream_error("HTTP 401 unauthorized: invalid_api_key"));
    assert!(!is_transient_stream_error("HTTP 429: insufficient_quota"));
    assert!(!is_transient_stream_error("HTTP 400 invalid_request_error"));
}
