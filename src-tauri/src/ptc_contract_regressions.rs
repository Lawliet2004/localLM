//! Acceptance checks at the public PTC/result boundary, independent of the
//! executor's parser/unit fixtures. Never require a live model or service.
use serde_json::{json, Value};
use std::sync::Arc;

async fn run(
    state: &crate::AppState,
    workspace: &std::path::Path,
    conversation: &str,
    tools: &[crate::connectors::AgentTool],
    plan: Value,
) -> Result<crate::harness::HarnessOutcome, String> {
    let ptc = crate::connectors::AgentTool::harness("ptc_run")?;
    let args = ptc.prepare_arguments(json!({"plan":plan}))?;
    let mut offered = tools.to_vec();
    offered.push(ptc);
    let tools = offered.as_slice();
    let aliases = tools.iter().map(|t| t.alias.as_str()).collect::<Vec<_>>();
    let ctx = crate::harness::HarnessCtx {
        state,
        snapshot: crate::test_support::snapshot(state, workspace, &aliases),
        conversation_id: conversation.into(),
        run_id: "acceptance-plan-run".into(),
        step_id: "acceptance-plan-step".into(),
        access_mode: crate::permissions::AccessMode::FullAccess,
        depth: 0,
        preset_id: crate::presets::CODE.into(),
        inherit_tools: tools,
        emit: Arc::new(|_, _| {}),
    };
    crate::harness::execute(&ctx, "ptc_run", args).await
}

#[tokio::test]
async fn failed_plan_is_a_model_visible_failure_with_no_success_continuation() {
    let dir = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(dir.path());
    let conversation = state.database().unwrap().create_conversation().unwrap().id;
    let tools =
        Arc::new(crate::workspace::Workspace::open(dir.path().to_str().unwrap()).unwrap()).tools();
    let raw = run(
        &state,
        dir.path(),
        &conversation,
        &tools,
        json!({"body":[
            {"call":{"tool":"workspace_read_file","args":{"path":"missing.txt"}}}
        ]}),
    )
    .await
    .unwrap()
    .value;
    let normalized = crate::tool_envelope::normalize(raw, crate::tool_envelope::Family::Harness);
    assert!(
        crate::tool_envelope::failed(&normalized),
        "failed plan must not normalize to success: {normalized}"
    );
    assert!(
        matches!(
            crate::continuation::continuation_after_tool(
                false,
                "ptc_run",
                &normalized,
                &Default::default(),
                0,
                false
            ),
            crate::continuation::Continuation::Stop {
                retryable: false,
                ..
            }
        ),
        "{normalized}"
    );
}

#[tokio::test]
async fn all_parallel_calls_are_reported_and_audited_even_when_first_fails() {
    let dir = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(dir.path());
    let conversation = state.database().unwrap().create_conversation().unwrap().id;
    std::fs::write(dir.path().join("present.txt"), "later member ran").unwrap();
    let tools =
        Arc::new(crate::workspace::Workspace::open(dir.path().to_str().unwrap()).unwrap()).tools();
    let result = run(
        &state,
        dir.path(),
        &conversation,
        &tools,
        json!({"body":[{"parallel":{"body":[
            {"call":{"tool":"workspace_read_file","args":{"path":"missing.txt"}}},
            {"call":{"tool":"workspace_read_file","args":{"path":"present.txt"}}}
        ]}}]}),
    )
    .await
    .unwrap()
    .value;
    let calls = result["calls"].as_array().unwrap();
    assert_eq!(
        calls.len(),
        2,
        "every dispatched member needs an outcome: {result}"
    );
    assert_eq!(calls[0]["status"], "error");
    assert_eq!(calls[1]["status"], "ok");
    let store = state.database().unwrap();
    let rows = store.messages(&conversation).unwrap();
    assert_eq!(
        rows.iter().filter(|m| m.role == "tool").count(),
        2,
        "both calls require durable audit rows"
    );
}

#[tokio::test]
async fn invalid_final_expression_preserves_completed_call_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(dir.path());
    let conversation = state.database().unwrap().create_conversation().unwrap().id;
    let tools =
        Arc::new(crate::workspace::Workspace::open(dir.path().to_str().unwrap()).unwrap()).tools();
    let result = run(
        &state,
        dir.path(),
        &conversation,
        &tools,
        json!({"body":[
        {"call":{"tool":"workspace_list_files","args":{"path":"."}}}
    ],"return":{"$":"missing_binding"}}),
    )
    .await
    .expect("a post-call expression failure must retain partial outcomes")
    .value;
    assert_eq!(result["calls"].as_array().unwrap().len(), 1, "{result}");
    assert_ne!(result["status"], "ok", "{result}");
    assert_eq!(result["partial"], 1, "{result}");
}

#[tokio::test]
async fn malformed_selected_branch_is_rejected_and_integer_order_is_exact() {
    let dir = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(dir.path());
    let conversation = state.database().unwrap().create_conversation().unwrap().id;
    let tools =
        Arc::new(crate::workspace::Workspace::open(dir.path().to_str().unwrap()).unwrap()).tools();
    let malformed = run(
        &state,
        dir.path(),
        &conversation,
        &tools,
        json!({"body":[{"if":{"cond":true,"then":"not a statement array"}}]}),
    )
    .await;
    assert!(
        match &malformed {
            Err(_) => true,
            Ok(o) => crate::tool_envelope::failed(&crate::tool_envelope::normalize(
                o.value.clone(),
                crate::tool_envelope::Family::Harness
            )),
        },
        "invalid branch must not silently succeed"
    );
    let ordered = run(
        &state,
        dir.path(),
        &conversation,
        &tools,
        json!({"body":[],"return":{"op":"lt","a":9007199254740992_u64,"b":9007199254740993_u64}}),
    )
    .await
    .unwrap()
    .value;
    assert_eq!(
        ordered["value"], true,
        "JSON integers must not collapse through f64: {ordered}"
    );
}

#[tokio::test]
async fn cancellation_interrupts_pending_plan_approval_and_clears_the_request() {
    let dir = tempfile::tempdir().unwrap();
    let state = Arc::new(crate::test_support::app_state(dir.path()));
    let conversation = state.database().unwrap().create_conversation().unwrap().id;
    let mut tools =
        Arc::new(crate::workspace::Workspace::open(dir.path().to_str().unwrap()).unwrap()).tools();
    tools.push(crate::connectors::AgentTool::harness("ptc_run").unwrap());
    let observed = Arc::new(std::sync::Mutex::new(None::<String>));
    let target = observed.clone();
    let cancelling = state.clone();
    let emit = Arc::new(move |_: String, approval: Option<Value>| {
        if let Some(id) = approval.as_ref().and_then(|v| v["id"].as_str()) {
            *target.lock().unwrap() = Some(id.into());
            cancelling.cancel.send_replace(true);
        }
    });
    let aliases = tools.iter().map(|t| t.alias.as_str()).collect::<Vec<_>>();
    let ctx = crate::harness::HarnessCtx {
        state: &state,
        snapshot: crate::test_support::snapshot(&state, dir.path(), &aliases),
        conversation_id: conversation,
        run_id: "approval-cancel-plan".into(),
        step_id: "approval-step".into(),
        access_mode: crate::permissions::AccessMode::Ask,
        depth: 0,
        preset_id: crate::presets::CODE.into(),
        inherit_tools: &tools,
        emit,
    };
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        crate::harness::execute(
            &ctx,
            "ptc_run",
            json!({"plan":{"body":[
                {"call":{"tool":"workspace_list_files","args":{"path":"."}}}
            ]}}),
        ),
    )
    .await
    .expect("Stop must interrupt a pending plan approval instead of waiting600seconds")
    .unwrap()
    .value;
    assert_eq!(result["status"], "cancelled", "{result}");
    let id = observed.lock().unwrap().clone().unwrap();
    assert!(
        state.approvals.resolve(&id, true).is_err(),
        "cancelled approval must not remain actionable"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn cancellation_interrupts_inflight_plan_tool_and_settles_its_audit() {
    let dir = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(dir.path());
    let conversation = state.database().unwrap().create_conversation().unwrap().id;
    let root = dir.path().to_str().unwrap();
    let started = state
        .processes
        .start(
            &conversation,
            root,
            crate::execution::ExecutionConfig::default(),
            json!({"command":"sleep 30","yield_ms":0}),
        )
        .await
        .unwrap();
    let id = started["id"].as_str().unwrap();
    let tools = [
        crate::connectors::AgentTool::harness("process_poll").unwrap(),
        crate::connectors::AgentTool::harness("ptc_run").unwrap(),
    ];
    let aliases = tools.iter().map(|t| t.alias.as_str()).collect::<Vec<_>>();
    let ctx = crate::harness::HarnessCtx {
        state: &state,
        snapshot: crate::test_support::snapshot(&state, dir.path(), &aliases),
        conversation_id: conversation.clone(),
        run_id: "tool-cancel-plan".into(),
        step_id: "tool-step".into(),
        access_mode: crate::permissions::AccessMode::FullAccess,
        depth: 0,
        preset_id: crate::presets::CODE.into(),
        inherit_tools: &tools,
        emit: Arc::new(|_, _| {}),
    };
    let stop = state.cancel.clone();
    let signal = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        stop.send_replace(true);
    });
    let result = tokio::time::timeout(
        std::time::Duration::from_millis(700),
        crate::harness::execute(
            &ctx,
            "ptc_run",
            json!({"plan":{"body":[
                {"call":{"tool":"process_poll","args":{"id":id,"wait_ms":1000}}}
            ]}}),
        ),
    )
    .await;
    state.cancel.send_replace(false);
    state
        .processes
        .stop(&conversation, root, json!({"id":id}))
        .await
        .unwrap();
    signal.await.unwrap();
    let result = result
        .expect("Stop must interrupt an already-running nested tool")
        .unwrap()
        .value;
    assert_eq!(result["status"], "cancelled", "{result}");
    let rows = state.database().unwrap().messages(&conversation).unwrap();
    assert!(!rows.is_empty());
    assert!(
        rows.iter().all(|row| row.status != "streaming"),
        "cancelled nested calls must settle their durable audit"
    );
}

#[tokio::test]
async fn nested_call_ids_are_unique_across_parallel_blocks_and_plan_invocations() {
    let dir = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(dir.path());
    let conversation = state.database().unwrap().create_conversation().unwrap().id;
    let tools =
        Arc::new(crate::workspace::Workspace::open(dir.path().to_str().unwrap()).unwrap()).tools();
    for name in ["a.txt", "b.txt", "c.txt", "d.txt"] {
        std::fs::write(dir.path().join(name), name).unwrap();
    }
    let plan = json!({"body":[
        {"parallel":{"body":[
            {"call":{"tool":"workspace_read_file","args":{"path":"a.txt"}}},
            {"call":{"tool":"workspace_read_file","args":{"path":"b.txt"}}}
        ]}},
        {"parallel":{"body":[
            {"call":{"tool":"workspace_read_file","args":{"path":"c.txt"}}},
            {"call":{"tool":"workspace_read_file","args":{"path":"d.txt"}}}
        ]}}
    ]});
    let mut seen = std::collections::HashSet::new();
    for _ in 0..2 {
        let outcome = run(&state, dir.path(), &conversation, &tools, plan.clone())
            .await
            .unwrap()
            .value;
        assert!(
            !crate::tool_envelope::failed(&crate::tool_envelope::normalize(
                outcome.clone(),
                crate::tool_envelope::Family::Harness
            )),
            "{outcome}"
        );
        let calls = outcome["calls"].as_array().unwrap();
        assert_eq!(calls.len(), 4);
        for call in calls {
            let id = call["callId"].as_str().unwrap();
            assert!(
                seen.insert(id.to_string()),
                "a completed nested call reused an existing audit/result ID: {id}"
            );
        }
    }
}

#[tokio::test]
async fn uncertain_nested_write_is_not_replayed_by_a_second_plan_in_the_same_turn() {
    let dir = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(dir.path());
    let conversation = state.database().unwrap().create_conversation().unwrap().id;
    let execution = Arc::new(
        crate::execution::LocalExecution::new(
            crate::execution::ExecutionConfig::default(),
            dir.path().to_str().unwrap(),
        )
        .unwrap(),
    );
    assert!(execution.code_available());
    let mut tool = execution.tool();
    tool.alias = "run_code".into();
    let mut alias = tool.clone();
    alias.alias = "local_run_code".into();
    let tools = vec![tool, alias];
    let args = json!({"language":"python","code":"with open('uncertain-counter.txt', 'a', encoding='utf-8') as out:\n out.write('x')\nraise RuntimeError('after the write')"});
    let first = run(
        &state,
        dir.path(),
        &conversation,
        &tools,
        json!({"body":[{"call":{"tool":"run_code","args":args,"as":"first"}}]}),
    )
    .await
    .unwrap()
    .value;
    assert!(
        crate::tool_envelope::failed(&crate::tool_envelope::normalize(
            first.clone(),
            crate::tool_envelope::Family::Harness
        )),
        "first plan must report its actual subprocess failure: {first}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("uncertain-counter.txt")).unwrap(),
        "x"
    );
    let second = run(
        &state,
        dir.path(),
        &conversation,
        &tools,
        json!({"body":[{"call":{"tool":"run_code","args":args,"as":"second"}}]}),
    )
    .await
    .unwrap()
    .value;
    assert!(
        crate::tool_envelope::failed(&crate::tool_envelope::normalize(
            second.clone(),
            crate::tool_envelope::Family::Harness
        )),
        "the subsequent plan must explain why the uncertain operation is blocked: {second}"
    );
    assert_eq!(std::fs::read_to_string(dir.path().join("uncertain-counter.txt")).unwrap(),"x","a second plan must not bypass the parent turn's uncertain-write protection by changing plan bindings");
    let aliased = run(
        &state,
        dir.path(),
        &conversation,
        &tools,
        json!({"body":[{"call":{"tool":"local_run_code","args":args,"as":"third"}}]}),
    )
    .await
    .unwrap()
    .value;
    assert!(crate::tool_envelope::failed(
        &crate::tool_envelope::normalize(aliased, crate::tool_envelope::Family::Harness)
    ));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("uncertain-counter.txt")).unwrap(),
        "x",
        "a namespaced alias for the same backend must not bypass uncertain-write protection"
    );
}

#[tokio::test]
async fn unreadable_shared_turn_budget_never_fails_open_into_a_write() {
    for poison_registry in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        let conversation = state.database().unwrap().create_conversation().unwrap().id;
        let tools =
            Arc::new(crate::workspace::Workspace::open(dir.path().to_str().unwrap()).unwrap())
                .tools();
        let args = json!({"path":"blocked-write.txt","content":"must not be written"});
        tools
            .iter()
            .find(|t| t.alias == "workspace_create_file")
            .unwrap()
            .prepare_arguments(args.clone())
            .unwrap();
        let cell = Arc::new(std::sync::Mutex::new(
            crate::continuation::Budgets::default(),
        ));
        state
            .turn_budgets
            .lock()
            .unwrap()
            .insert("acceptance-plan-run".into(), cell.clone());
        if poison_registry {
            let registry = &state.turn_budgets;
            std::thread::scope(|scope| {
                assert!(scope
                    .spawn(move || {
                        let _guard = registry.lock().unwrap();
                        panic!("fixture poisons budget registry");
                    })
                    .join()
                    .is_err());
            });
        } else {
            assert!(std::thread::spawn(move || {
                let _guard = cell.lock().unwrap();
                panic!("fixture poisons budget cell");
            })
            .join()
            .is_err());
        }
        let outcome = run(
            &state,
            dir.path(),
            &conversation,
            &tools,
            json!({"body":[{"call":{"tool":"workspace_create_file","args":args}}]}),
        )
        .await;
        assert!(!dir.path().join("blocked-write.txt").exists(),"an unreadable active parent budget must deny before physical write (registry={poison_registry})");
        assert!(match outcome {
            Err(_) => true,
            Ok(o) => crate::tool_envelope::failed(&crate::tool_envelope::normalize(
                o.value,
                crate::tool_envelope::Family::Harness
            )),
        });
        assert!(state
            .database()
            .unwrap()
            .messages(&conversation)
            .unwrap()
            .iter()
            .all(|row| row.status != "streaming"));
    }
}
