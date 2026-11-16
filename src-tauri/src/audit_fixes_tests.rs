#[cfg(test)]
mod audit_fixes {
    use serde_json::json;

    #[test]
    fn quoted_examples_stay_inert() {
        let sample = "Example syntax, do not execute:\n```xml\n<tool_call>{\"name\":\"workspace_create_file\",\"arguments\":{\"path\":\"example.txt\",\"content\":\"example\"}}</tool_call>\n```";
        assert!(crate::tool_calls::parse_text_tool_calls(sample).is_none());
        assert!(matches!(crate::chat::decide_text_tool_call(sample, 0), crate::chat::TextCallDecision::NotACall));
        assert_eq!(crate::tool_calls::strip_text_tool_calls(sample), sample);

    }

    #[test]
    fn compatible_reasoning_survives_round_replay() {
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
        assert_eq!(message["reasoning_content"], "Use the exact required algorithm.");
        let saved = crate::history::saved_native_message(&message).unwrap();
        assert_eq!(crate::history::assistant_from_saved(&saved).unwrap(), message);
        
    }

    #[test]
    fn discovery_obeys_target_write_budget() {
        let root = tempfile::tempdir().unwrap();
        let workspace = crate::workspace::Workspace::open(root.path().to_str().unwrap()).unwrap();
        let write = std::sync::Arc::new(workspace).tools().into_iter()
            .find(|tool| tool.alias == "workspace_write_file").unwrap();
        let invoke = crate::connectors::AgentTool::harness("tool_invoke").unwrap();
        let args = json!({"path":"large.txt","content":"x".repeat(40_000)});
        write.prepare_arguments(args.clone()).unwrap();
        crate::tool_spec::check_argument_bytes("workspace_write_file", args.to_string().len()).unwrap();
        let tools = [write, invoke];
        let (target, resolved) = crate::tool_discovery::resolve(tools.iter(), "tool_invoke",
            json!({"name":"workspace_write_file","arguments":args.clone()})).unwrap();
        assert_eq!(target.alias, "workspace_write_file");
        assert_eq!(resolved, args);
        assert!(crate::tool_discovery::resolve(tools.iter(), "tool_invoke",
            json!({"name":"workspace_write_file","arguments":{"path":"a","content":"x".repeat(7 * 1024 * 1024)}})).is_err());

    }

    #[test]
    fn local_execution_invalidates_passed_checks() {
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
            assert_eq!(progress.outcome()["status"], "unverified");
            assert!(progress.needs_verification());
        }
    }

    #[tokio::test]
    async fn workspace_aliases_share_nested_rules() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("nested")).unwrap();
        std::fs::write(root.path().join("nested/AGENTS.md"), "Always retain the SENTINEL_NESTED_RULE contract.").unwrap();
        std::fs::write(root.path().join("nested/a.txt"), "file contents").unwrap();
        let workspace = std::sync::Arc::new(crate::workspace::Workspace::open(root.path().to_str().unwrap()).unwrap());
        let read = workspace.tools().into_iter().find(|tool| tool.alias == "workspace_read_file").unwrap();
        let direct = read.call(json!({"path":"nested/a.txt"})).await.unwrap();
        assert!(direct.to_string().contains("SENTINEL_NESTED_RULE"));
        let catalog = crate::harness::execute_workspace_tool(root.path().to_str().unwrap(),
            crate::execution::ExecutionConfig::default(), "file_read", json!({"path":"nested/a.txt"})).await.unwrap();
        assert!(catalog["instructions"].as_str().unwrap().contains("SENTINEL_NESTED_RULE"));
    }

    #[tokio::test]
    async fn bounded_harness_read_preserves_file_identity() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("large.txt"), "contents\n".repeat(4000)).unwrap();
        let read = crate::harness::execute_workspace_tool(root.path().to_str().unwrap(),
            crate::execution::ExecutionConfig::default(), "file_read", json!({"path":"large.txt"})).await.unwrap();
        assert!(read["sha256"].is_string());
        let normalized = crate::tool_envelope::normalize(read, crate::tool_envelope::Family::Harness);
        let (bounded, artifact) = crate::artifacts::bound_tool_result(normalized, "file_read", "audit", None, 2048);
        assert!(artifact.is_some());
        assert_eq!(bounded["sha256"], bounded["data"]["sha256"]);
        assert!(bounded["sha256"].as_str().is_some_and(|sha| sha.len() == 64));
        assert!(bounded["_artifactId"].is_string());
    }
    #[test]
    fn bounded_rules_and_long_paths_stay_inside_the_output_budget() {
        for rules in ["rule\n".repeat(160), "rule\n".repeat(4000)] {
            let result = crate::tool_envelope::normalize(json!({"sha256":"a".repeat(64),"path":"long/".repeat(300),"totalLines":4000,"content":"contents\n".repeat(4000),"instructions":rules}), crate::tool_envelope::Family::Harness);
            let (bounded, artifact) = crate::artifacts::bound_tool_result(result, "file_read", "test", None, 2048);
            assert!(artifact.is_some());
            assert!(bounded.to_string().len() <= 2048);
            assert!(bounded["data"]["instructions"].is_string() || bounded["data"]["instructionsIncomplete"] == true);
            assert_eq!(bounded["data"]["sha256"], "a".repeat(64));
        }
    }

}
