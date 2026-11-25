#[cfg(test)]
mod audit_20261108 {
    use serde_json::{json, Value};

    #[test]
    fn quoted_code_example_becomes_an_executable_call() {
        let sample = "Example syntax, do not execute:\n```xml\n<tool_call>{\"name\":\"workspace_create_file\",\"arguments\":{\"path\":\"example.txt\",\"content\":\"example\"}}</tool_call>\n```";
        let parsed = crate::tool_calls::parse_text_tool_calls(sample).unwrap();
        assert_eq!(parsed[0].name, "workspace_create_file");
        assert!(matches!(crate::chat::decide_text_tool_call(sample, 0), crate::chat::TextCallDecision::Execute(_)));
        assert!(!crate::tool_calls::strip_text_tool_calls(sample).contains("example.txt"));
        println!("CONFIRMED: fenced explanatory XML becomes Execute and is removed from answer text");
    }

    #[test]
    fn chat_completion_reasoning_is_absent_from_tool_round_replay() {
        let mut stream = crate::sse::ModelStream::new(false);
        let mut calls = crate::tool_calls::ToolCalls::default();
        for value in [
            json!({"choices":[{"delta":{"reasoning_content":"Use the exact required algorithm."}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","type":"function","function":{"name":"workspace_read_file","arguments":"{\"path\":\"a.txt\"}"}}]}}]}),
            json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}),
        ] {
            if let Some(event) = stream.push(&value.to_string()).unwrap() {
                calls.push(&event["choices"][0]["delta"]).unwrap();
            }
        }
        stream.push("[DONE]").unwrap();
        let message = stream.assistant_message("", &calls.finish().unwrap());
        assert_eq!(message["tool_calls"].as_array().unwrap().len(), 1);
        assert!(message.get("reasoning_content").is_none());
        assert!(!message.to_string().contains("exact required algorithm"));
        println!("CONFIRMED: received reasoning_content is absent from the next tool-round assistant message");
    }

    #[test]
    fn discovered_write_rejects_input_accepted_by_the_direct_tool() {
        let root = tempfile::tempdir().unwrap();
        let workspace = crate::workspace::Workspace::open(root.path().to_str().unwrap()).unwrap();
        let write = std::sync::Arc::new(workspace).tools().into_iter()
            .find(|tool| tool.alias == "workspace_write_file").unwrap();
        let invoke = crate::connectors::AgentTool::harness("tool_invoke").unwrap();
        let args = json!({"path":"large.txt","content":"x".repeat(40_000)});
        write.prepare_arguments(args.clone()).unwrap();
        crate::tool_spec::check_argument_bytes("workspace_write_file", args.to_string().len()).unwrap();
        let tools = [write, invoke];
        let error = crate::tool_discovery::resolve(tools.iter(), "tool_invoke",
            json!({"name":"workspace_write_file","arguments":args})).err().unwrap();
        assert!(error.contains("32KiB"), "{error}");
        println!("CONFIRMED: 40,000-byte valid direct write fails discovery wrapper: {error}");
    }

    #[test]
    fn local_execution_aliases_do_not_invalidate_passed_verification() {
        for alias in ["local_run_code", "local_execute_command"] {
            let mut progress = crate::verification_progress::Progress::default();
            progress.observe("workspace_apply_patch", &json!({}), &json!({"ok":true}), true);
            progress.observe("check_run", &json!({"command":"test"}), &json!({
                "processId":"p1","process":{"state":"exited","exitCode":0},
                "report":{"verdict":"passed","commandSucceeded":true,"incomplete":false}
            }), true);
            assert_eq!(progress.outcome()["status"], "verified");
            progress.observe(alias, &json!({"command":"touch changed-after-test","code":"open('changed-after-test', 'w').write('changed')"}),
                &json!({"exitCode":0}), true);
            assert_eq!(progress.outcome()["status"], "verified");
            assert!(!progress.needs_verification());
            println!("CONFIRMED: {alias} after checks leaves prior changes marked verified");
        }
    }

    #[test]
    fn compaction_keeps_an_archive_but_no_semantic_record_of_old_requirements() {
        let requirement = "Never change the customer's legacy API contract: SENTINEL_REQUIREMENT";
        let mut messages = vec![json!({"role":"system","content":"Assist the user."}), json!({"role":"user","content":requirement})];
        messages.push(json!({"role":"assistant","content":"Acknowledged."}));
        for n in 0..12 {
            messages.push(json!({"role":"user","content":format!("Followup {n}")}));
            messages.push(json!({"role":"assistant","content":format!("Result {n}")}));
        }
        messages.push(json!({"role":"user","content":"Continue implementing."}));
        let plan = crate::compaction::plan_compaction(&messages, 1).unwrap();
        assert!(Value::Array(plan.dropped).to_string().contains("SENTINEL_REQUIREMENT"));
        assert!(!Value::Array(plan.kept).to_string().contains("SENTINEL_REQUIREMENT"));
        println!("CONFIRMED: old explicit requirement survives only in the archive, absent from the model-visible checkpoint");
    }

    #[tokio::test]
    async fn nested_project_rules_depend_on_which_file_read_alias_is_used() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("nested")).unwrap();
        std::fs::write(root.path().join("nested/AGENTS.md"), "Always retain the SENTINEL_NESTED_RULE contract.").unwrap();
        std::fs::write(root.path().join("nested/a.txt"), "file contents").unwrap();
        let workspace = std::sync::Arc::new(crate::workspace::Workspace::open(root.path().to_str().unwrap()).unwrap());
        let read = workspace.tools().into_iter().find(|tool| tool.alias == "workspace_read_file").unwrap();
        let direct = read.call(json!({"path":"nested/a.txt"})).await.unwrap();
        assert!(!direct.to_string().contains("SENTINEL_NESTED_RULE"));
        let catalog = crate::harness::execute_workspace_tool(root.path().to_str().unwrap(),
            crate::execution::ExecutionConfig::default(), "file_read", json!({"path":"nested/a.txt"})).await.unwrap();
        assert!(catalog["instructions"].as_str().unwrap().contains("SENTINEL_NESTED_RULE"));
        println!("CONFIRMED: file_read supplies applicable nested AGENTS.md; workspace_read_file omits it");
    }

    #[tokio::test]
    async fn native_catalog_file_read_still_loses_file_identity_when_bounded() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("large.txt"), "contents\n".repeat(4000)).unwrap();
        let read = crate::harness::execute_workspace_tool(root.path().to_str().unwrap(),
            crate::execution::ExecutionConfig::default(), "file_read", json!({"path":"large.txt"})).await.unwrap();
        assert!(read["sha256"].is_string());
        let normalized = crate::tool_envelope::normalize(read, crate::tool_envelope::Family::Harness);
        let (bounded, artifact) = crate::artifacts::bound_tool_result(normalized, "file_read", "audit", None, 2048);
        assert!(artifact.is_some());
        assert!(bounded.get("sha256").is_none());
        assert!(bounded.pointer("/data/sha256").is_none());
        assert!(bounded["_artifactId"].is_string());
        println!("CONFIRMED: first-party file_read still loses file SHA-256 at the 2 KiB bound");
    }
}
