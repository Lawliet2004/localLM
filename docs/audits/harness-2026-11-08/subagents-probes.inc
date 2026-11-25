#[cfg(test)]
mod audit_harness_20261108 {
    use super::*;
    use serde_json::{json, Value};
    use std::time::Duration;
    fn note(name: &str, evidence: Value) {
        println!("AUDIT_EVIDENCE {}",json!({"name":name,"evidence":evidence}));
    }
    fn request(owner: &str) -> ChildRequest {
        ChildRequest { label:"audit child".into(),prompt:"Inspect the project and report.".into(),depth:0,max_rounds:4,
            tool_filter:None,persona:None,output_schema:None,policy:ChildPolicy::ReadsOnly,
            parent_run_id:"audit-parent".into(),conversation_id:owner.into() }
    }
    fn remote_snapshot(state: &crate::AppState, root: &std::path::Path, base: String) -> BackendSnapshot {
        let mut snapshot = crate::test_support::snapshot(state,root,&[]);
        snapshot.kind = "remote".into(); snapshot.base_url = base;
        snapshot.context_length = 32768;
        snapshot.api_format = crate::providers::FREETOKEN_OPENAI_COMPAT.into();
        snapshot
    }
    fn wire(text: &str, reasoning: &str) -> String {
        format!("data: {}\r\n\r\ndata: [DONE]\r\n\r\n",json!({"choices":[{"delta":{"content":text,"reasoning_content":reasoning},"finish_reason":"stop"}],"usage":{"prompt_tokens":100,"completion_tokens":10,"prompt_tokens_details":{"cached_tokens":80}}}))
    }

    #[tokio::test]
    async fn diag_empty_child_answer_is_marked_completed_and_usage_disappears() {
        let root = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(root.path());
        let owner = state.database().unwrap().create_conversation().unwrap().id;
        let (base,requests,server) = crate::test_support::scripted_sse(vec![wire("","Only reasoning was generated.")]).await;
        let snapshot = remote_snapshot(&state,root.path(),base);
        let (id,answer) = run_child_inline(&state,&snapshot,request(&owner),&[],None).await.unwrap();
        server.await.unwrap();
        assert!(answer.is_empty());
        let row = state.database().unwrap().subagent_runs_for_conversation(&owner).unwrap().remove(0);
        assert_eq!(row.status,"completed");
        let events = state.database().unwrap().run_events(&id).unwrap();
        assert!(!events.iter().any(|e| e.event_type == "inference_usage"));
        assert!(!requests.lock().unwrap()[0].body.to_string().contains("include_usage"));
        note("empty_child_and_unreported_usage",json!({"status":row.status,"answerLength":answer.len(),"reportedFixtureCacheTokens":80,"savedUsageEvents":0,"usageRequested":false}));
    }

    #[tokio::test]
    async fn diag_child_request_omits_project_instruction_file() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("AGENTS.md"),"MUST preserve RULE_SENTINEL and never modify protected.txt.").unwrap();
        let state = crate::test_support::app_state(root.path());
        let owner = state.database().unwrap().create_conversation().unwrap().id;
        let (base,requests,server) = crate::test_support::scripted_sse(vec![wire("Inspected.","")]).await;
        let snapshot = remote_snapshot(&state,root.path(),base);
        run_child_inline(&state,&snapshot,request(&owner),&[],None).await.unwrap();
        server.await.unwrap();
        let payload = requests.lock().unwrap()[0].body.clone();
        assert!(!payload.to_string().contains("RULE_SENTINEL"));
        note("child_project_rules",json!({"instructionFileExists":true,"instructionSentInChildRequest":false,"system":payload["messages"][0]["content"]}));
    }

    #[tokio::test]
    async fn diag_dropped_foreground_child_leaves_running_state_and_live_registry_entry() {
        use tokio::io::AsyncReadExt;
        let root = tempfile::tempdir().unwrap();
        let state = Arc::new(crate::test_support::app_state(root.path()));
        let owner = state.database().unwrap().create_conversation().unwrap().id;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/v1",listener.local_addr().unwrap());
        let snapshot = remote_snapshot(&state,root.path(),base);
        let (seen,accepted) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket,_) = listener.accept().await.unwrap();
            let mut buf=[0;4096]; socket.read(&mut buf).await.unwrap();
            let _=seen.send(()); std::future::pending::<()>().await;
        });
        let req=request(&owner); let worker_state=state.clone();
        let worker=tokio::spawn(async move { run_child_inline(&worker_state,&snapshot,req,&[],None).await });
        tokio::time::timeout(Duration::from_secs(5),accepted).await.unwrap().unwrap();
        worker.abort(); assert!(worker.await.unwrap_err().is_cancelled());
        let row=state.database().unwrap().subagent_runs_for_conversation(&owner).unwrap().remove(0);
        let live=state.subagents.deliver(&row.child_run_id,"Is any worker still here?");
        assert!(live); assert_eq!(row.status,"running");
        state.subagents.retire(&row.child_run_id); server.abort();
        note("dropped_foreground_child",json!({"workerAborted":true,"lineageStatus":row.status,"registryAcceptsMail":live}));
    }

    #[tokio::test]
    async fn diag_background_setup_error_is_discarded_without_settling_run() {
        let root=tempfile::tempdir().unwrap();
        let state=crate::test_support::app_state(root.path());
        let owner=state.database().unwrap().create_conversation().unwrap().id;
        let snapshot=remote_snapshot(&state,root.path(),"not a URL".into());
        assert!(snapshot.backend().is_err());
        let id=spawn_background(snapshot,request(&owner)).unwrap();
        tokio::time::timeout(Duration::from_secs(3),async {
            while state.subagents.deliver(&id,"probe") { tokio::time::sleep(Duration::from_millis(10)).await; }
        }).await.unwrap();
        let row=state.database().unwrap().subagent_runs_for_conversation(&owner).unwrap().remove(0);
        assert_eq!(row.status,"running"); assert!(row.error.is_none());
        note("background_setup_error",json!({"spawnReturnedSuccess":true,"workerRetired":true,"lineageStatus":row.status,"error":row.error}));
    }

    #[tokio::test]
    async fn diag_background_children_bypass_four_parent_run_slots() {
        let root=tempfile::tempdir().unwrap();
        let state=crate::test_support::app_state(root.path());
        let owner=state.database().unwrap().create_conversation().unwrap().id;
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base=format!("http://{}/v1",listener.local_addr().unwrap());
        let snapshot=remote_snapshot(&state,root.path(),base);
        let mut ids=Vec::new();
        for _ in 0..12 { ids.push(spawn_background(snapshot.clone(),request(&owner)).unwrap()); }
        let running=state.database().unwrap().subagent_runs_for_conversation(&owner).unwrap().into_iter().filter(|row|row.status=="running").count();
        assert_eq!(running,12);
        assert_eq!(state.session_runs.active_count(),0);
        for id in &ids { assert!(state.subagents.interrupt(id)); }
        tokio::time::timeout(Duration::from_secs(5),async {
            loop {
                if state.database().unwrap().subagent_runs_for_conversation(&owner).unwrap().iter().all(|row|row.status!="running") {break;}
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }).await.unwrap();
        drop(listener);
        note("background_admission",json!({"acceptedSimultaneousChildren":running,"parentSlotsConsumed":0,"fixtureChildrenInterrupted":true}));
    }
}
