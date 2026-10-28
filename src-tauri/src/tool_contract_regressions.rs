//! Independent coordinator regressions for A05. These reflect producer and
//! dispatch contracts, not the implementation choices of the in-flight model
//! pass. Keep ambiguous side effects and domain status fields distinct.
use crate::tool_envelope::{self as envelope, Family, Status};
use serde_json::json;

#[test]
fn unsupported_envelope_versions_are_explicit_failures() {
    let result = envelope::normalize(
        json!({"schemaVersion":"locallm.tool-envelope/999","status":"success","data":{"answer":1},"error":null,"artifacts":[],"metadata":{}}),
        Family::Mcp,
    );
    assert!(
        envelope::failed(&result),
        "Unsupported envelope version was accepted as success: {result}"
    );
    assert!(
        envelope::validate_envelope(&result).is_ok(),
        "The failure itself needs a valid predictable contract: {result}"
    );
}

#[test]
fn malformed_envelope_metadata_never_panics_or_becomes_success() {
    for metadata in [json!("invalid"), json!([]), json!(false), json!(null)] {
        let raw = json!({"schemaVersion":envelope::SCHEMA_VERSION,"status":"success","data":{"answer":1},"error":null,"artifacts":[],"metadata":metadata});
        let outcome = std::panic::catch_unwind(|| envelope::normalize(raw, Family::Mcp));
        assert!(
            outcome.is_ok(),
            "Malformed producer metadata must not panic at dispatch"
        );
        let result = outcome.unwrap();
        assert!(
            envelope::failed(&result),
            "Malformed envelope was accepted as success: {result}"
        );
        assert!(envelope::validate_envelope(&result).is_ok());
    }
}

#[test]
fn generic_connector_errors_cannot_promise_safe_write_replay() {
    for notice in [
        json!({}),
        json!({"denied":true}),
        json!({"paused":true}),
        json!({"code":"denied"}),
    ] {
        let mut raw = json!({"isError":true,"message":"Tool failed after applying some changes."});
        raw.as_object_mut()
            .unwrap()
            .extend(notice.as_object().unwrap().clone());
        let result = envelope::normalize(raw, Family::Mcp);
        assert_eq!(
            result["error"]["retryable"], false,
            "A generic MCP error provides no proof of safe replay: {result}"
        );
        let certainty = result
            .pointer("/error/sideEffects")
            .or_else(|| result.pointer("/metadata/sideEffects"));
        assert_eq!(
            certainty,
            Some(&json!("unknown")),
            "MCP domain flags cannot prove that a partial write had no effects: {result}"
        );
    }
}

#[test]
fn connector_domain_statuses_are_not_native_transaction_statuses() {
    for status in [
        "unknown",
        "rolled_back",
        "not_started",
        "committed",
        "cleaned",
    ] {
        let raw = json!({"status":status,"value":"business state returned by a connector"});
        let result = envelope::normalize(raw.clone(), Family::Mcp);
        assert_eq!(
            result["status"], "success",
            "Domain status={status} is ordinary successful data: {result}"
        );
        assert_eq!(result["data"], raw);
        assert!(!envelope::failed(&result));
    }
}

#[test]
fn actual_successful_patch_recovery_is_a_successful_tool_call() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let id = "a".repeat(32);
    let before = "original";
    std::fs::write(root.path().join("a.txt"), before).unwrap();
    std::fs::write(root.path().join(format!(".locallm-txn-{id}-0-old")), before).unwrap();
    let journal = root.path().join(".locallm-transactions");
    std::fs::create_dir(&journal).unwrap();
    let manifest = json!({"version":1,"id":id,"state":"prepared","entries":[{"path":"a.txt","before":format!("{:x}",Sha256::digest(before.as_bytes())),"after":format!("{:x}",Sha256::digest(b"replacement"))}]});
    std::fs::write(
        journal.join(format!("{id}.json")),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    let workspace = crate::workspace::Workspace::open(root.path().to_str().unwrap()).unwrap();
    let raw = workspace
        .call("recover_patch", json!({"transaction_id":id}))
        .unwrap();
    assert_eq!(raw["status"], "rolled_back");
    assert_eq!(raw["isError"], false);
    assert_eq!(raw["recoveryRequired"], false);
    let result = envelope::normalize(raw, Family::Workspace);
    assert_eq!(
        result["status"], "success",
        "The requested recovery succeeded: {result}"
    );
    assert!(!envelope::failed(&result));
    assert_eq!(result["data"]["status"], "rolled_back");
    assert_eq!(
        envelope::validate_envelope(&result).unwrap(),
        Status::Success
    );
}

#[test]
fn confirmed_cleanup_failures_are_not_contradictory_success_envelopes() {
    for status in ["committed", "cleaned"] {
        let raw = json!({"status":status,"isError":true,"recoveryRequired":true,"cleanupError":"Filesystem refused cleanup."});
        let result = envelope::normalize(raw, Family::Workspace);
        assert_ne!(
            result["status"], "success",
            "Producer reported an error, so success would contradict isError: {result}"
        );
        assert!(envelope::failed(&result));
        assert!(envelope::validate_envelope(&result).is_ok());
    }
}

fn lifecycle_setup(
    root: &std::path::Path,
) -> (crate::AppState, String, Vec<crate::connectors::AgentTool>) {
    let state = crate::test_support::app_state(root);
    let chat = {
        let store = state.database().unwrap();
        let chat = store.create_conversation().unwrap().id;
        store
            .save_run(&crate::agent_run::RunRecord {
                id: "contract-live-run".into(),
                conversation_id: chat.clone(),
                status: crate::agent_run::RunState::ExecutingTools,
                model_provider: None,
                model_id: None,
                checkpoint: None,
                error: None,
                created_at: crate::store::now(),
                updated_at: crate::store::now(),
            outcome: None,
})
            .unwrap();
        let task =
            crate::research_tasks::ResearchTask::new(&chat, "Contract fixture")
                .unwrap();
        store.save_research_task(&task).unwrap();
        chat
    };
    let tools = ["research_pause", "research_resume", "research_cancel"]
        .into_iter()
        .map(|name| crate::connectors::AgentTool::harness(name).unwrap())
        .collect();
    (state, chat, tools)
}

fn lifecycle_ctx<'a>(
    state: &'a crate::AppState,
    root: &std::path::Path,
    chat: &str,
    tools: &'a [crate::connectors::AgentTool],
) -> crate::harness::HarnessCtx<'a> {
    crate::harness::HarnessCtx {
        state,
        snapshot: crate::test_support::snapshot(
            state,
            root,
            &["research_pause", "research_resume", "research_cancel"],
        ),
        conversation_id: chat.into(),
        run_id: "contract-live-run".into(),
        step_id: "contract-step".into(),
        access_mode: crate::permissions::AccessMode::FullAccess,
        depth: 0,
        preset_id: crate::presets::STANDARD.into(),
        inherit_tools: tools,
        emit: std::sync::Arc::new(|_, _| {}),
    }
}

fn assert_loud_control_failure(result: Result<crate::harness::HarnessOutcome, String>) {
    match result {
        Err(message) => assert!(!message.trim().is_empty()),
        Ok(outcome) => {
            assert_ne!(
                outcome.value["applied"], true,
                "Failed control/persistence must not claim it applied durably: {}",
                outcome.value
            );
            assert!(
                envelope::failed(&outcome.value) || outcome.value["reason"].is_string(),
                "Failure needs an explicit error/reason: {}",
                outcome.value
            );
        }
    }
}

#[tokio::test]
async fn lifecycle_lock_failure_never_claims_a_control_was_applied() {
    for alias in ["research_pause", "research_cancel"] {
        let root = tempfile::tempdir().unwrap();
        let (state, chat, tools) = lifecycle_setup(root.path());
        let ctx = lifecycle_ctx(&state, root.path(), &chat, &tools);
        // A poisoned control lock is a real failure mode, not authorization.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = state.run_control.lock().unwrap();
            panic!("fixture: run-control failure");
        }));
        assert_loud_control_failure(crate::harness::execute(&ctx, alias, json!({})).await);
    }
}

#[tokio::test]
async fn lifecycle_persistence_failure_is_not_a_durable_success() {
    let root = tempfile::tempdir().unwrap();
    let (state, chat, tools) = lifecycle_setup(root.path());
    let ctx = lifecycle_ctx(&state, root.path(), &chat, &tools);
    // The table remains readable; only the durable state transition fails.
    let connection = rusqlite::Connection::open(&state.db_path).unwrap();
    connection.execute_batch("CREATE TRIGGER fixture_reject_research_update BEFORE UPDATE ON research_tasks BEGIN SELECT RAISE(ABORT, 'fixture: durable write failed'); END;").unwrap();
    assert_loud_control_failure(crate::harness::execute(&ctx, "research_pause", json!({})).await);
}

#[tokio::test]
async fn model_pause_cannot_take_ownership_of_a_pending_user_pause() {
    let root = tempfile::tempdir().unwrap();
    let (state, chat, tools) = lifecycle_setup(root.path());
    let ctx = lifecycle_ctx(&state, root.path(), &chat, &tools);
    // Simulate the user pause arriving after the dispatcher's precheck. The
    // handler must preserve ownership under the actual control mutex.
    crate::agent_run::apply_pause(&mut state.run_control.lock().unwrap());
    let _ = crate::harness::execute(&ctx, "research_pause", json!({})).await;
    {
        let control = state.run_control.lock().unwrap();
        assert!(control.pause);
        assert!(
            !control.pause_by_model,
            "A model pause must never relabel a user's pause as its own"
        );
    }
    let _ = crate::harness::execute(&ctx, "research_resume", json!({})).await;
    assert!(
        state.run_control.lock().unwrap().pause,
        "Model resume must preserve the user's pending pause"
    );
}

#[tokio::test]
async fn parent_completion_checkpoint_preserves_durable_lifecycle_state() {
    for (alias, expected) in [
        ("research_pause", crate::research_tasks::TaskStatus::Paused),
        (
            "research_cancel",
            crate::research_tasks::TaskStatus::Cancelled,
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        let (state, chat, tools) = lifecycle_setup(root.path());
        // The real chat loop owns this cached copy before dispatch.
        let mut cached = state
            .database()
            .unwrap()
            .latest_research_task(&chat)
            .unwrap()
            .unwrap();
        let ctx = lifecycle_ctx(&state, root.path(), &chat, &tools);
        let outcome = crate::harness::execute(&ctx, alias, json!({}))
            .await
            .unwrap();
        assert_eq!(outcome.value["applied"], true);
        assert_eq!(
            state
                .database()
                .unwrap()
                .latest_research_task(&chat)
                .unwrap()
                .unwrap()
                .status,
            expected
        );
        // This is the actual completion helper used immediately after the
        // model-visible result is stored. It must not undo the tool's durable
        // pause/cancel with the loop's older Running copy, even before settle.
        crate::chat::persist_completed_tool(
            &state.database().unwrap(),
            &mut cached,
            alias,
            Some("completed call".into()),
        )
        .unwrap();
        assert_eq!(
            state
                .database()
                .unwrap()
                .latest_research_task(&chat)
                .unwrap()
                .unwrap()
                .status,
            expected,
            "Completion checkpoint must not revert lifecycle state after {alias}"
        );
        assert_eq!(
            cached.status, expected,
            "The loop must retain the authoritative lifecycle state too"
        );
    }
}

#[tokio::test]
async fn actual_execution_envelopes_gain_explicit_certainty_at_the_shared_boundary() {
    let root = tempfile::tempdir().unwrap();
    let execution = crate::execution::LocalExecution::new(
        crate::execution::ExecutionConfig::default(),
        root.path().to_str().unwrap(),
    )
    .unwrap();
    let raw = execution
        .run(json!({"code":"print('before failure')\nraise RuntimeError('contract fixture')"}))
        .await
        .unwrap();
    assert!(
        envelope::failed(&raw),
        "The real child process must fail: {raw}"
    );
    let result = envelope::normalize(raw, Family::Execution);
    assert_eq!(result["error"]["retryable"], false);
    assert_eq!(
        result["error"]["sideEffects"], "unknown",
        "A failing executable may have written files before exit: {result}"
    );
    assert_eq!(result["metadata"]["sideEffects"], "unknown");
    assert!(envelope::validate_envelope(&result).is_ok());
}

/// Every command the frontend invokes must be registered in the Tauri
/// handler list. A mismatch is a runtime failure that no Rust test can see,
/// because the IPC boundary is resolved by name at invoke time.
#[test]
fn every_frontend_invoke_resolves_to_a_registered_command() {
    let api = include_str!("../../src/lib/api.ts");
    let lib = include_str!("lib.rs");

    // Registered names: every `module::command` pair in the handler list.
    // Tauri derives the command name from the function name unless the
    // command is renamed with `#[tauri::command(rename_all = ...)]`.
    let mut registered: Vec<String> = Vec::new();
    let handler = lib
        .split_once("generate_handler![")
        .expect("lib.rs must declare a Tauri handler list")
        .1
        .split_once("];")
        .expect("the handler list must be closed")
        .0;
    for entry in handler.split(',') {
        let entry = entry.trim();
        if let Some((module, command)) = entry.split_once("::") {
            registered.push(command.trim().to_string());
            registered.push(format!("{}_{}", module.trim(), command.trim()));
        }
    }

    // Scan `invoke<...>('name'` / `invoke('name'` precisely. The generic
    // argument may itself contain `import('./types')`, so the type parameter
    // list is skipped with a bracket-depth scan instead of a naive search for
    // the next parenthesis.
    let mut invoked: Vec<String> = Vec::new();
    let bytes = api.as_bytes();
    let mut index = 0usize;
    while let Some(found) = api[index..].find("invoke") {
        let start = index + found;
        let after = &api[start + "invoke".len()..];
        let trimmed = after.trim_start();
        let cursor = after.len() - trimmed.len();
        let mut rest = trimmed;
        if let Some(tail) = rest.strip_prefix('<') {
            let mut depth = 1usize;
            let mut end = 0usize;
            for (offset, character) in tail.char_indices() {
                match character {
                    '<' => depth += 1,
                    '>' => {
                        depth -= 1;
                        if depth == 0 {
                            end = offset;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if depth != 0 {
                break;
            }
            rest = tail[end + 1..].trim_start();
        }
        let Some(arguments) = rest.strip_prefix('(') else {
            index = start + "invoke".len() + cursor;
            continue;
        };
        let arguments = arguments.trim_start();
        if let Some(name) = arguments
            .strip_prefix('\'')
            .and_then(|tail| tail.split('\'').next())
        {
            invoked.push(name.to_string());
        }
        index = start + "invoke".len() + cursor + 1;
        let _ = bytes;
    }

    assert!(
        !invoked.is_empty(),
        "The frontend invoke scan must find at least one command"
    );
    let missing: Vec<&String> = invoked
        .iter()
        .filter(|name| !registered.contains(name))
        .collect();
    assert!(
        missing.is_empty(),
        "The frontend invokes commands the backend never registers: {missing:?}"
    );
}

/// A declared result contract must name a tool that exists, and the tool's
/// real result must satisfy the contract. A contract that drifts from the
/// implementation is worse than no contract: it fails the tool at runtime.
#[test]
fn declared_output_contracts_match_real_tool_results() {
    let catalog = crate::harness::registry();
    let contracted: Vec<&crate::tool_spec::ToolSpec> = catalog
        .iter()
        .filter(|spec| spec.has_output_contract())
        .collect();
    assert!(
        !contracted.is_empty(),
        "The harness must declare at least one result contract"
    );
    for spec in contracted {
        let schema = spec.output.clone().unwrap();
        assert!(
            crate::tool_validation::compile(&schema).is_ok(),
            "Tool '{}' declares an uncompilable output contract",
            spec.name
        );
        assert!(
            schema.get("type").is_some(),
            "Tool '{}' declares an output contract without a type",
            spec.name
        );
    }
}

/// Every alias a contract names must exist. A typo would silently disable
/// the contract instead of failing.
#[test]
fn output_contract_aliases_all_exist() {
    let catalog = crate::harness::registry();
    for spec in &catalog {
        if let Some(output) = &spec.output {
            assert!(
                output.is_object(),
                "Tool '{}' has a non-object output contract",
                spec.name
            );
        }
    }
    // The contracts are attached to these aliases; if one is renamed the
    // contract would silently stop being enforced.
    for alias in [
        "list_agents",
        "memory_recall",
        "todo_write",
        "file_search",
        "artifact_read",
        "goal_set",
        "goal_clear",
    ] {
        let spec = crate::tool_spec::find(&catalog, alias)
            .unwrap_or_else(|| panic!("{alias} must be a registered harness tool"));
        assert!(
            spec.has_output_contract(),
            "{alias} must declare a result contract"
        );
    }
}
