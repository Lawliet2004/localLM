use super::*;
use serde_json::{json, Value};
use std::time::Duration;
fn note(name: &str, evidence: Value) {
    println!(
        "AUDIT_EVIDENCE {}",
        json!({"name":name,"evidence":evidence})
    );
}
fn request(owner: &str) -> ChildRequest {
    ChildRequest {
        label: "audit child".into(),
        prompt: "Inspect the project and report.".into(),
        depth: 0,
        max_rounds: 4,
        tool_filter: None,
        persona: None,
        output_schema: None,
        policy: ChildPolicy::ReadsOnly,
        parent_run_id: "audit-parent".into(),
        conversation_id: owner.into(),
    }
}
fn remote_snapshot(
    state: &crate::AppState,
    root: &std::path::Path,
    base: String,
) -> BackendSnapshot {
    let mut snapshot = crate::test_support::snapshot(state, root, &[]);
    snapshot.kind = "remote".into();
    snapshot.base_url = base;
    snapshot.context_length = 32768;
    snapshot.api_format = crate::providers::FREETOKEN_OPENAI_COMPAT.into();
    snapshot
}
fn wire(text: &str, reasoning: &str) -> String {
    format!(
        "data: {}\r\n\r\ndata: [DONE]\r\n\r\n",
        json!({"choices":[{"delta":{"content":text,"reasoning_content":reasoning},"finish_reason":"stop"}],"usage":{"prompt_tokens":100,"completion_tokens":10,"prompt_tokens_details":{"cached_tokens":80}}})
    )
}

async fn wait_for_settlement(state: &crate::AppState, owner: &str, count: usize) {
    let result = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let events = state
                .database()
                .unwrap()
                .session_events(owner, None, Some(1000))
                .unwrap();
            if events
                .iter()
                .filter(|e| e.event_type == "subagent_end")
                .count()
                >= count
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    if result.is_err() {
        let store = state.database().unwrap();
        let rows = store.subagent_runs_for_conversation(owner).unwrap();
        let events = store.session_events(owner, None, Some(1000)).unwrap();
        panic!(
            "background children did not settle: rows={rows:?}; ends={:?}",
            events
                .iter()
                .filter(|e| e.event_type == "subagent_end")
                .map(|e| (&e.run_id, &e.payload))
                .collect::<Vec<_>>()
        );
    }
}

#[tokio::test]
async fn multiple_child_rounds_preserve_usage_tool_calls_and_complete_result() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.txt"), "alpha").unwrap();
    std::fs::write(root.path().join("b.txt"), "beta").unwrap();
    let state = crate::test_support::app_state(root.path());
    let owner = state.database().unwrap().create_conversation().unwrap().id;
    let call = |id: &str, path: &str| {
        format!(
            "data: {}\r\n\r\ndata: [DONE]\r\n\r\n",
            json!({
                "choices":[{"delta":{"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":"workspace_read_file","arguments":json!({"path":path}).to_string()}}]},"finish_reason":"tool_calls"}],
                "usage":{"prompt_tokens":100,"completion_tokens":10,"prompt_tokens_details":{"cached_tokens":80}}
            })
        )
    };
    let answer = format!("alpha beta {}", "complete result ".repeat(200));
    let (base, _, server) = crate::test_support::scripted_sse(vec![
        call("read-a", "a.txt"),
        call("read-b", "b.txt"),
        wire(&answer, ""),
    ])
    .await;
    let mut snapshot = remote_snapshot(&state, root.path(), base);
    snapshot.allowed_tool_aliases = vec!["workspace_read_file".into()];
    let (child, result) = run_child_inline(&state, &snapshot, request(&owner), &[], None)
        .await
        .unwrap();
    server.await.unwrap();
    assert_eq!(result, answer);
    let store = state.database().unwrap();
    let events = store.run_events(&child).unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.event_type == "inference_usage")
            .count(),
        3
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.event_type == "tool_call")
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.event_type == "tool_result")
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .find(|e| e.event_type == "subagent_result")
            .unwrap()
            .payload["answer"],
        answer
    );
    assert!(events.windows(2).all(|pair| pair[0].seq < pair[1].seq));
    let children = usage_for_parent(&store, &owner, "audit-parent").unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].rounds.len(), 3);
    assert_eq!(
        children[0]
            .rounds
            .iter()
            .map(|round| round.usage.cached_tokens.unwrap())
            .sum::<u64>(),
        240
    );
}

#[tokio::test]
async fn background_completion_refreshes_parent_metrics_and_inherits_rules() {
    for before_parent_end in [false, true] {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("AGENTS.md"),
            "MUST retain BACKGROUND_RULE_SENTINEL.",
        )
        .unwrap();
        let state = crate::test_support::app_state(root.path());
        let owner = state.database().unwrap().create_conversation().unwrap().id;
        let message = {
            let store = state.database().unwrap();
            let now = crate::store::now();
            store
                .save_run(&crate::agent_run::RunRecord {
                    id: "audit-parent".into(),
                    conversation_id: owner.clone(),
                    status: crate::agent_run::RunState::Completed,
                    model_provider: None,
                    model_id: None,
                    checkpoint: None,
                    error: None,
                    created_at: now,
                    updated_at: now,
                })
                .unwrap();
            let message = store
                .append_message(&owner, "assistant", "Child is working", "complete")
                .unwrap();
            store
                .save_message_metrics(&message.id, &crate::inference::ResponseMetrics::default())
                .unwrap();
            if !before_parent_end {
                crate::chat::emit_session_event(
                    &store,
                    &owner,
                    Some("audit-parent"),
                    None,
                    None,
                    "turn_end",
                    json!({"messageId":message.id}),
                    false,
                );
            }
            message.id
        };
        let (base, requests, server) =
            crate::test_support::scripted_sse(vec![wire("Background answer", "")]).await;
        let snapshot = remote_snapshot(&state, root.path(), base);
        let child = spawn_background(snapshot, request(&owner)).unwrap();
        server.await.unwrap();
        wait_for_settlement(&state, &owner, 1).await;
        if before_parent_end {
            let store = state.database().unwrap();
            crate::chat::emit_session_event(
                &store,
                &owner,
                Some("audit-parent"),
                None,
                None,
                "turn_end",
                json!({"messageId":message}),
                false,
            );
            store
                .refresh_delegated_metrics(&owner, "audit-parent")
                .unwrap();
        }
        assert!(requests.lock().unwrap()[0]
            .body
            .to_string()
            .contains("BACKGROUND_RULE_SENTINEL"));
        let store = state.database().unwrap();
        let messages = store.messages(&owner).unwrap();
        let metrics = messages
            .iter()
            .find(|m| m.id == message)
            .unwrap()
            .metrics
            .as_ref()
            .unwrap();
        assert_eq!(metrics.children.len(), 1);
        assert_eq!(metrics.children[0].run_id, child);
        assert_eq!(metrics.children[0].status, "completed");
        assert_eq!(metrics.children[0].rounds[0].usage.cached_tokens, Some(80));
    }
}

#[tokio::test]
async fn asynchronous_background_setup_error_is_durably_failed() {
    let root = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(root.path());
    let owner = state.database().unwrap().create_conversation().unwrap().id;
    let mut snapshot = remote_snapshot(&state, root.path(), "http://127.0.0.1:9/v1".into());
    snapshot.context_length = 1; // Valid transport; asynchronous context preparation fails before dispatch.
    let child = spawn_background(snapshot, request(&owner)).unwrap();
    wait_for_settlement(&state, &owner, 1).await;
    let store = state.database().unwrap();
    let row = store
        .subagent_runs_for_conversation(&owner)
        .unwrap()
        .remove(0);
    assert_eq!(row.status, "failed");
    assert!(
        row.error
            .as_ref()
            .is_some_and(|error| error.contains("Context limit")),
        "{:?}",
        row.error
    );
    assert_eq!(
        store.run(&child).unwrap().unwrap().status,
        crate::agent_run::RunState::Failed
    );
    assert!(!state.subagents.deliver(&child, "no worker remains"));
}

#[tokio::test]
async fn empty_child_answer_fails_and_preserves_reported_usage() {
    let root = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(root.path());
    let owner = state.database().unwrap().create_conversation().unwrap().id;
    let (base, requests, server) =
        crate::test_support::scripted_sse(vec![wire("", "Only reasoning was generated.")]).await;
    let snapshot = remote_snapshot(&state, root.path(), base);
    let error = run_child_inline(&state, &snapshot, request(&owner), &[], None)
        .await
        .unwrap_err();
    server.await.unwrap();
    assert!(error.contains("no visible final answer"));
    let row = state
        .database()
        .unwrap()
        .subagent_runs_for_conversation(&owner)
        .unwrap()
        .remove(0);
    assert_eq!(row.status, "failed");
    let id = row.child_run_id.clone();
    let events = state.database().unwrap().run_events(&id).unwrap();
    assert!(events
        .iter()
        .any(|e| e.event_type == "inference_usage" && e.payload["usage"]["cachedTokens"] == 80));
    assert!(requests.lock().unwrap()[0]
        .body
        .to_string()
        .contains("include_usage"));
    note(
        "empty_child_and_unreported_usage",
        json!({"status":row.status,"error":error,"reportedFixtureCacheTokens":80,"savedUsageEvents":1,"usageRequested":true}),
    );
}

#[tokio::test]
async fn child_request_inherits_project_instruction_files() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("AGENTS.md"),
        "MUST preserve RULE_SENTINEL and never modify protected.txt.",
    )
    .unwrap();
    let state = crate::test_support::app_state(root.path());
    let owner = state.database().unwrap().create_conversation().unwrap().id;
    let (base, requests, server) =
        crate::test_support::scripted_sse(vec![wire("Inspected.", "")]).await;
    let snapshot = remote_snapshot(&state, root.path(), base);
    run_child_inline(&state, &snapshot, request(&owner), &[], None)
        .await
        .unwrap();
    server.await.unwrap();
    let payload = requests.lock().unwrap()[0].body.clone();
    assert!(payload.to_string().contains("RULE_SENTINEL"));
    note(
        "child_project_rules",
        json!({"instructionFileExists":true,"instructionSentInChildRequest":true,"system":payload["messages"][0]["content"]}),
    );
}

#[tokio::test]
async fn dropped_foreground_child_retires_registry_and_settles_lineage() {
    use tokio::io::AsyncReadExt;
    let root = tempfile::tempdir().unwrap();
    let state = Arc::new(crate::test_support::app_state(root.path()));
    let owner = state.database().unwrap().create_conversation().unwrap().id;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let snapshot = remote_snapshot(&state, root.path(), base);
    let (seen, accepted) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0; 4096];
        socket.read(&mut buf).await.unwrap();
        let _ = seen.send(());
        std::future::pending::<()>().await;
    });
    let req = request(&owner);
    let worker_state = state.clone();
    let worker =
        tokio::spawn(
            async move { run_child_inline(&worker_state, &snapshot, req, &[], None).await },
        );
    tokio::time::timeout(Duration::from_secs(5), accepted)
        .await
        .unwrap()
        .unwrap();
    worker.abort();
    assert!(worker.await.unwrap_err().is_cancelled());
    let row = state
        .database()
        .unwrap()
        .subagent_runs_for_conversation(&owner)
        .unwrap()
        .remove(0);
    let live = state
        .subagents
        .deliver(&row.child_run_id, "Is any worker still here?");
    assert!(!live);
    assert_eq!(row.status, "interrupted");
    assert_eq!(
        state
            .database()
            .unwrap()
            .run(&row.child_run_id)
            .unwrap()
            .unwrap()
            .status,
        crate::agent_run::RunState::OutcomeUnknown
    );
    let metrics = usage_for_parent(&*state.database().unwrap(), &owner, "audit-parent").unwrap();
    assert_eq!(metrics[0].rounds.len(), 1);
    assert!(!metrics[0].rounds[0].complete);
    state.subagents.retire(&row.child_run_id);
    server.abort();
    note(
        "dropped_foreground_child",
        json!({"workerAborted":true,"lineageStatus":row.status,"registryAcceptsMail":live}),
    );
}

#[tokio::test]
async fn invalid_background_snapshot_is_rejected_before_publishing() {
    let root = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(root.path());
    let owner = state.database().unwrap().create_conversation().unwrap().id;
    let snapshot = remote_snapshot(&state, root.path(), "not a URL".into());
    assert!(spawn_background(snapshot, request(&owner)).is_err());
    assert!(state
        .database()
        .unwrap()
        .subagent_runs_for_conversation(&owner)
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn background_child_admission_is_bounded_and_released() {
    let root = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(root.path());
    let owner = state.database().unwrap().create_conversation().unwrap().id;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let snapshot = remote_snapshot(&state, root.path(), base);
    let mut ids = Vec::new();
    for _ in 0..8 {
        ids.push(spawn_background(snapshot.clone(), request(&owner)).unwrap());
    }
    let overflow = spawn_background(snapshot.clone(), request(&owner)).unwrap_err();
    assert!(overflow.contains("At most eight"));
    let running = state
        .database()
        .unwrap()
        .subagent_runs_for_conversation(&owner)
        .unwrap()
        .into_iter()
        .filter(|row| row.status == "running")
        .count();
    assert_eq!(running, 8);
    assert_eq!(state.session_runs.active_count(), 0);
    for id in &ids {
        assert!(state.subagents.interrupt(id));
    }
    wait_for_settlement(&state, &owner, 8).await;
    let replacement =
        spawn_background(snapshot, request(&owner)).expect("settled children release admission");
    assert!(state.subagents.interrupt(&replacement));
    wait_for_settlement(&state, &owner, 9).await;
    drop(listener);
    note(
        "background_admission",
        json!({"acceptedSimultaneousChildren":running,"parentSlotsConsumed":0,"fixtureChildrenInterrupted":true}),
    );
}
