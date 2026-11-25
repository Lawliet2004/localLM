#[cfg(test)]
mod audit_harness_20261108 {
    use super::*;
    use serde_json::{json,Value};
    fn note(name:&str,evidence:Value) { println!("AUDIT_EVIDENCE {}",json!({"name":name,"evidence":evidence})); }
    fn text(text:&str) -> String {
        format!("data: {}\r\n\r\ndata: [DONE]\r\n\r\n",json!({"choices":[{"delta":{"content":text},"finish_reason":"stop"}]}))
    }
    fn call(id:&str,name:&str,args:Value) -> String {
        format!("data: {}\r\n\r\ndata: [DONE]\r\n\r\n",json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":name,"arguments":args.to_string()}}]},"finish_reason":"tool_calls"}]}))
    }
    fn conversation(state:&AppState,base:String) -> String {
        let store=state.database().unwrap();
        let mut provider=store.provider("fixture-loopback").unwrap().unwrap();
        provider.base_url=base; store.save_provider(&provider).unwrap();
        let owner=store.create_conversation().unwrap().id;
        store.save_conversation_tools(&owner,&crate::store::ConversationTools {access_mode:crate::permissions::AccessMode::FullAccess,sources:vec!["__workspace".into()],tools:vec![]}).unwrap();
        store.save_conversation_preset(&owner,crate::presets::CODE).unwrap();
        owner
    }

    #[tokio::test]
    async fn diag_parent_stops_at_64_model_rounds_with_unfinished_work() {
        let root=tempfile::tempdir().unwrap();
        let state=crate::test_support::app_state(root.path());
        crate::test_support::select_fixture_model(&state,root.path());
        let mut responses=Vec::new();
        for index in 0..63 {
            let path=format!("file-{index}.txt"); std::fs::write(root.path().join(&path),"fixture").unwrap();
            responses.push(call(&format!("read-{index}"),"workspace_read_file",json!({"path":path})));
        }
        responses.push(text("Partial result: additional work remains."));
        let (base,requests,server)=crate::test_support::scripted_sse(responses).await;
        let owner=conversation(&state,base);
        send_registered_message(&state,owner.clone(),"Inspect all 100 numbered files and complete all requested work.".into(),Some(vec!["__workspace".into()]),Some(vec![]),None,None,Channel::new(|_|Ok(())),None).await.unwrap();
        server.await.unwrap();
        let requests=requests.lock().unwrap();
        assert_eq!(requests.len(),64);
        assert_eq!(requests[63].body["tool_choice"],"none");
        assert_eq!(requests[0].body["tools"],requests[63].body["tools"]);
        let store=state.database().unwrap();
        let events=store.session_events(&owner,None,Some(5000)).unwrap();
        let end=events.iter().rev().find(|e|e.event_type=="turn_end").unwrap();
        // The continuation code installed a partial-result fallback before
        // the final round, so the separate partial_work flag was never set.
        assert_eq!(end.payload["partial"],false);
        assert!(requests[63].body["messages"].as_array().unwrap().iter().any(|m|m["content"].as_str().is_some_and(|s|s.contains("Budget or checklist limit reached"))));
        assert_eq!(store.messages(&owner).unwrap().iter().filter(|m|m.role=="tool").count(),63);
        note("parent_round_cap",json!({"modelRequests":requests.len(),"toolCalls":63,"partial":end.payload["partial"],"finalToolChoice":requests[63].body["tool_choice"],"stableToolSchemas":true}));
    }

    #[tokio::test]
    async fn diag_task_graph_pending_work_is_missing_from_turn_partial_outcome() {
        let root=tempfile::tempdir().unwrap();
        let state=crate::test_support::app_state(root.path());
        crate::test_support::select_fixture_model(&state,root.path());
        let (base,requests,server)=crate::test_support::scripted_sse(vec![
            call("create-task","task_create",json!({"title":"Implement the requested feature"})),
            text("I have recorded the task; implementation is still pending."),
        ]).await;
        let owner=conversation(&state,base);
        send_registered_message(&state,owner.clone(),"Track the feature in the task graph and implement it.".into(),Some(vec!["__workspace".into()]),Some(vec![]),None,None,Channel::new(|_|Ok(())),None).await.unwrap();
        server.await.unwrap();
        assert_eq!(requests.lock().unwrap().len(),2);
        let store=state.database().unwrap();
        let pending:i64=store.connection.query_row("SELECT COUNT(*) FROM tasks WHERE conversation_id=?1 AND state='pending'",[&owner],|r|r.get(0)).unwrap();
        assert_eq!(pending,1);
        let events=store.session_events(&owner,None,None).unwrap();
        let end=events.iter().rev().find(|e|e.event_type=="turn_end").unwrap();
        assert_eq!(end.payload["partial"],false);
        note("task_graph_completion",json!({"pendingGraphTasks":pending,"turnEndPartial":end.payload["partial"],"unfinishedTodos":end.payload["unfinishedTodos"],"delivered":end.payload["delivered"]}));
    }

    #[tokio::test]
    async fn diag_completed_turn_research_ledger_reopens_as_paused_work() {
        let root=tempfile::tempdir().unwrap();
        let state=crate::test_support::app_state(root.path());
        crate::test_support::select_fixture_model(&state,root.path());
        let (base,_,server)=crate::test_support::scripted_sse(vec![text("4")]).await;
        let owner=conversation(&state,base);
        send_registered_message(&state,owner.clone(),"What is 2 + 2?".into(),Some(vec![]),Some(vec![]),None,None,Channel::new(|_|Ok(())),None).await.unwrap();
        server.await.unwrap();
        let store=state.database().unwrap();
        assert!(store.messages(&owner).unwrap().iter().filter(|m|m.role=="assistant").all(|m|m.status=="complete"));
        let before=store.latest_research_task(&owner).unwrap().unwrap();
        assert_eq!(before.status,crate::research_tasks::TaskStatus::Running);
        store.recover_interrupted().unwrap();
        crate::research_tasks::recover_interrupted(&store).unwrap();
        let after=store.latest_research_task(&owner).unwrap().unwrap();
        assert_eq!(after.status,crate::research_tasks::TaskStatus::Paused);
        note("completed_turn_research_ledger",json!({"answerCompleted":true,"ledgerBeforeRestart":before.status,"ledgerAfterRestart":after.status,"pending":after.pending_work}));
    }

    #[tokio::test]
    async fn diag_read_only_ptc_uses_an_extra_verification_round() {
        let mut observations=Vec::new();
        for batched in [false,true] {
            let root=tempfile::tempdir().unwrap();
            std::fs::write(root.path().join("AGENTS.md"),"MUST preserve PARENT_RULE_SENTINEL.").unwrap();
            std::fs::write(root.path().join("a.txt"),"alpha").unwrap();
            std::fs::write(root.path().join("b.txt"),"beta").unwrap();
            let state=crate::test_support::app_state(root.path());
            crate::test_support::select_fixture_model(&state,root.path());
            let responses=if batched {vec![
                call("batch","ptc_run",json!({"steps":[{"tool":"workspace_read_file","args":{"path":"a.txt"}},{"tool":"workspace_read_file","args":{"path":"b.txt"}}]})),
                text("alpha beta"),
                text("alpha beta"),
            ]} else {vec![
                call("read-a","workspace_read_file",json!({"path":"a.txt"})),
                call("read-b","workspace_read_file",json!({"path":"b.txt"})),
                text("alpha beta"),
            ]};
            let (base,requests,server)=crate::test_support::scripted_sse(responses).await;
            let owner=conversation(&state,base);
            send_registered_message(&state,owner.clone(),"Read a.txt and b.txt and report both values.".into(),Some(vec!["__workspace".into(),"__execution".into()]),Some(vec![]),None,None,Channel::new(|_|Ok(())),None).await.unwrap();
            server.await.unwrap();
            let requests=requests.lock().unwrap();
            assert!(requests[0].body["messages"][0]["content"].as_str().unwrap().contains("PARENT_RULE_SENTINEL"));
            let last=requests.last().unwrap().body["messages"].to_string();
            assert!(last.contains("alpha")&&last.contains("beta"),"batched={batched}; messages={}",serde_json::to_string(&requests.last().unwrap().body["messages"].as_array().unwrap().iter().filter(|m|m["role"]=="tool").collect::<Vec<_>>()).unwrap());
            let messages=state.database().unwrap().messages(&owner).unwrap();
            let answer=messages.iter().find(|m|m.role=="assistant").unwrap();
            assert_eq!(answer.content,"alpha beta");
            let events=state.database().unwrap().session_events(&owner,None,None).unwrap();
            let nudges=events.iter().filter(|e|e.event_type=="verify_before_finish").count();
            assert_eq!(nudges,usize::from(batched));
            let end=events.iter().rev().find(|e|e.event_type=="turn_end").unwrap();
            observations.push(json!({"batched":batched,"modelRequests":requests.len(),"outerToolCalls":if batched{1}else{2},"underlyingReads":2,"durableToolAuditRows":messages.iter().filter(|m|m.role=="tool").count(),"answer":answer.content,"parentRulesPresent":true,"verificationNudges":nudges,"verification":end.payload["verification"],"partial":end.payload["partial"]}));
        }
        assert_eq!(observations[0]["modelRequests"],3);
        assert_eq!(observations[1]["modelRequests"],3);
        note("scripted_ptc_equivalence",json!({"cases":observations,"limitation":"Scripted model chooses the plan. This tests orchestration and result parity, not real-model task accuracy or comparative harness performance."}));
    }
}
