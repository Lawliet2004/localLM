use super::*;
#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    #[test]
    fn policy_is_cheap_and_conservative() {
        for q in [
            "What is TCP?",
            "Explain DNS in simple words.",
            "Fix this TypeScript error",
            "Fix this chart bug",
            "Implement an LRU cache in Python.",
        ] {
            assert_eq!(profile(Mode::Auto, q, 2048), None);
        }
        assert_eq!(
            profile(Mode::Auto, "Show me how TCP handshake works", 2048),
            Some("core")
        );
        assert_eq!(
            profile(Mode::Auto, "Compare PostgreSQL, Redis and FalkorDB", 2048),
            Some("data")
        );
        assert_eq!(
            profile(Mode::Auto, "Teach binary search interactively", 2048),
            Some("education")
        );
        assert_eq!(
            profile(Mode::Auto, "Make a bar chart of sales", 2048),
            Some("chart")
        );
        assert_eq!(profile(Mode::Auto, "Make a flowchart", 2048), Some("core"));
        assert_eq!(profile(Mode::Text, "Plot these sales numbers", 2048), None);
        assert_eq!(profile(Mode::Visual, "Anything", 256), None);
        assert_eq!(
            profile(
                Mode::Visual,
                "Show exact full code from lru.py using workspace_read_file",
                2048
            ),
            Some("code")
        );
        assert_eq!(
            profile(Mode::Visual, "Make a Python LRU quiz", 2048),
            Some("education")
        );
    }
    #[test]
    fn sqlite_persists_modes_state_and_cascades() {
        let store = Store::open_memory().unwrap();
        let c = store.create_conversation().unwrap();
        let other = store.create_conversation().unwrap();
        store.set_presentation_mode(None, Some(Mode::Text)).unwrap();
        store
            .set_presentation_mode(Some(&c.id), Some(Mode::Visual))
            .unwrap();
        assert_eq!(
            store.presentation_mode(Some(&c.id)).unwrap().effective_mode,
            Mode::Visual
        );
        assert_eq!(
            store
                .presentation_mode(Some(&other.id))
                .unwrap()
                .effective_mode,
            Mode::Text
        );
        let m = store
            .append_message(&c.id, "assistant", "Original factual answer", "complete")
            .unwrap();
        let record = Record {
            message_id: m.id.clone(),
            format: "openui".into(),
            format_version: 1,
            library_version: LIBRARY_VERSION.into(),
            source_text: m.content.clone(),
            ui_program: "root = Response([])".into(),
            interactive_state: json!({}),
            generation_status: "complete".into(),
            diagnostic: None,
        };
        store.save_presentation(&record).unwrap();
        assert!(store
            .save_visual_state(&other.id, &m.id, json!({"quiz":1}))
            .is_err());
        store
            .save_visual_state(&c.id, &m.id, json!({"quiz":1}))
            .unwrap();
        assert_eq!(
            store.presentations(&c.id).unwrap()[0].interactive_state,
            json!({"quiz":1})
        );
        assert_eq!(
            store.messages(&c.id).unwrap()[0].content,
            "Original factual answer"
        );
        assert!(store
            .save_visual_state(&c.id, &m.id, json!({"__proto__":1}))
            .is_err());
        store.delete_conversation(&c.id).unwrap();
        assert!(store.presentations(&c.id).unwrap().is_empty());
    }
    #[test]
    fn malformed_metadata_does_not_block_restart_or_lose_original_answer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid.sqlite");
        let conversation;
        {
            let store = Store::open(&path).unwrap();
            conversation = store.create_conversation().unwrap().id;
            let message = store
                .append_message(&conversation, "assistant", "Readable original", "complete")
                .unwrap();
            store
                .connection
                .execute(
                    "INSERT INTO message_presentations VALUES(?1,?2)",
                    [&message.id, "{broken"],
                )
                .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.messages(&conversation).unwrap()[0].content,
            "Readable original"
        );
        let records = store.presentations(&conversation).unwrap();
        assert_eq!(records[0].format, "markdown");
        assert_eq!(records[0].source_text, "Readable original");
    }
    #[test]
    fn restart_preserves_controls_and_marks_partial_programs_interrupted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.sqlite");
        let id;
        {
            let store = Store::open(&path).unwrap();
            let c = store.create_conversation().unwrap();
            id = c.id.clone();
            let m = store
                .append_message(&id, "assistant", "Answer", "complete")
                .unwrap();
            let mut r = Record {
                message_id: m.id,
                format: "openui".into(),
                format_version: 1,
                library_version: LIBRARY_VERSION.into(),
                source_text: "Answer".into(),
                ui_program: "root = Response([])".into(),
                interactive_state: json!({"quiz":{"answer":2}}),
                generation_status: "streaming".into(),
                diagnostic: None,
            };
            store.save_presentation(&r).unwrap();
            r.message_id = store
                .append_message(&id, "assistant", "Second", "complete")
                .unwrap()
                .id;
            r.generation_status = "complete".into();
            store.save_presentation(&r).unwrap();
        }
        let store = Store::open(&path).unwrap();
        store.recover_interrupted().unwrap();
        let records = store.presentations(&id).unwrap();
        assert_eq!(records[0].generation_status, "interrupted");
        assert_eq!(records[1].interactive_state, json!({"quiz":{"answer":2}}));
    }
}

#[cfg(test)]
mod stream_tests {
    use super::*;
    #[tokio::test]
    async fn isolated_presentation_stream_has_no_tools_history_or_state_and_preserves_answer() {
        let root = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(root.path());
        crate::test_support::select_fixture_model(&state, root.path());
        let ui = "root = Response([Text(\"Observed result: 2\")])";
        let sse = format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"delta":{"content":ui}}]}),
            json!({"choices":[{"delta":{},"finish_reason":"stop"}]})
        );
        let (base, requests, server) = crate::test_support::scripted_sse(vec![sse]).await;
        let (conversation, message) = {
            let store = state.database().unwrap();
            let mut provider = store.provider("fixture-loopback").unwrap().unwrap();
            provider.base_url = base;
            store.save_provider(&provider).unwrap();
            let c = store.create_conversation().unwrap();
            let m = store
                .append_message(&c.id, "assistant", "Observed result: 2", "complete")
                .unwrap();
            (c.id, m.id)
        };
        state
            .database()
            .unwrap()
            .save_run(&crate::agent_run::RunRecord {
                id: "fixture-presentation".into(),
                conversation_id: conversation.clone(),
                status: crate::agent_run::RunState::Generating,
                model_provider: Some("fixture-loopback".into()),
                model_id: Some("fixture-model".into()),
                checkpoint: None,
                error: None,
                created_at: crate::store::now(),
                updated_at: crate::store::now(),
            })
            .unwrap();
        let plan = crate::chat::assemble_turn(
            &state,
            &conversation,
            "Plot result",
            vec![],
            Some(vec![]),
            None,
            None,
        )
        .await
        .unwrap();
        let channel = Channel::new(|_| Ok(()));
        let (_cancel, mut cancellation) = tokio::sync::watch::channel(false);
        generate(
            &state,
            &plan,
            FinalAnswer {
                conversation: &conversation,
                message: &message,
                question: "Plot result",
                answer: "Observed result: 2",
                mode: Mode::Visual,
                run_id: "fixture-presentation",
            },
            &channel,
            &mut cancellation,
        )
        .await
        .unwrap();
        server.await.unwrap();
        let request = &requests.lock().unwrap()[0].body;
        assert!(request.get("tools").is_none());
        assert_eq!(request["messages"].as_array().unwrap().len(), 2);
        assert!(!request.to_string().contains("interactiveState"));
        let store = state.database().unwrap();
        let record = &store.presentations(&conversation).unwrap()[0];
        assert_eq!(record.ui_program, ui);
        assert_eq!(record.generation_status, "complete");
        assert_eq!(
            store.messages(&conversation).unwrap()[0].content,
            "Observed result: 2"
        );
    }
}

mod regressions {
    use super::*;
    async fn fixture(
        base: &str,
    ) -> (
        tempfile::TempDir,
        AppState,
        crate::chat::TurnPlan,
        String,
        String,
    ) {
        fixture_protocol(base, crate::providers::FREETOKEN_OPENAI_COMPAT).await
    }
    async fn fixture_protocol(
        base: &str,
        format: &str,
    ) -> (
        tempfile::TempDir,
        AppState,
        crate::chat::TurnPlan,
        String,
        String,
    ) {
        let root = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(root.path());
        crate::test_support::select_fixture_model(&state, root.path());
        state
            .daytona_vault
            .save(
                &crate::providers::credential_id("fixture-loopback"),
                b"fixture-only-not-a-real-key",
            )
            .unwrap();
        let (conversation, message) = {
            let store = state.database().unwrap();
            let mut provider = store.provider("fixture-loopback").unwrap().unwrap();
            provider.base_url = base.into();
            provider.api_format = format.into();
            store.save_provider(&provider).unwrap();
            let c = store.create_conversation().unwrap();
            let m = store
                .append_message(&c.id, "assistant", "Authoritative answer", "complete")
                .unwrap();
            store
                .save_run(&crate::agent_run::RunRecord {
                    id: "presentation-regression".into(),
                    conversation_id: c.id.clone(),
                    status: crate::agent_run::RunState::Generating,
                    model_provider: Some("fixture-loopback".into()),
                    model_id: Some("fixture-model".into()),
                    checkpoint: None,
                    error: None,
                    created_at: crate::store::now(),
                    updated_at: crate::store::now(),
                })
                .unwrap();
            (c.id, m.id)
        };
        let plan = crate::chat::assemble_turn(
            &state,
            &conversation,
            "Chart",
            vec![],
            Some(vec![]),
            None,
            None,
        )
        .await
        .unwrap();
        (root, state, plan, conversation, message)
    }
    fn final_answer<'a>(conversation: &'a str, message: &'a str) -> FinalAnswer<'a> {
        FinalAnswer {
            conversation,
            message,
            question: "Chart",
            answer: "Authoritative answer",
            mode: Mode::Visual,
            run_id: "presentation-regression",
        }
    }
    #[tokio::test]
    async fn transport_failures_and_usage_are_distinct_from_render_success() {
        for (delta, finish, expected) in [
            (
                json!({"content":"root = Response([])"}),
                Some("stop"),
                "complete",
            ),
            (
                json!({"content":"root = Response([Text(\"partial"}),
                Some("length"),
                "error",
            ),
            (
                json!({"tool_calls":[{"function":{"name":"shell","arguments":"{}"}}]}),
                Some("stop"),
                "error",
            ),
            (json!({"content":"root = Response([])"}), None, "error"),
        ] {
            let mut sse = format!("data: {}\n\n", json!({"choices":[{"delta":delta}]}));
            if let Some(reason) = finish {
                sse.push_str(&format!("data: {}\n\ndata: [DONE]\n\n",json!({"choices":[{"delta":{},"finish_reason":reason}],"usage":{"prompt_tokens":30,"completion_tokens":8}})));
            }
            let (base, requests, server) = crate::test_support::scripted_sse(vec![sse]).await;
            let (_root, state, plan, c, m) = fixture(&base).await;
            let (_cancel, mut cancellation) = tokio::sync::watch::channel(false);
            let measurement = generate(
                &state,
                &plan,
                final_answer(&c, &m),
                &Channel::new(|_| Ok(())),
                &mut cancellation,
            )
            .await
            .unwrap()
            .unwrap();
            server.await.unwrap();
            assert_eq!(measurement.phase.as_deref(), Some("presentation"));
            assert_eq!(measurement.complete, expected == "complete");
            if expected == "complete" {
                assert_eq!(measurement.usage.input_tokens, Some(30));
                assert_eq!(measurement.usage.output_tokens, Some(8));
            }
            let store = state.database().unwrap();
            assert_eq!(
                store.presentations(&c).unwrap()[0].generation_status,
                expected
            );
            assert_eq!(
                store.messages(&c).unwrap()[0].content,
                "Authoritative answer"
            );
            assert!(requests.lock().unwrap()[0].body.get("tools").is_none());
        }
    }
    #[tokio::test]
    async fn context_exhaustion_and_cancel_before_generation_preserve_answer_without_request() {
        let (_root, state, mut plan, c, m) = fixture("http://127.0.0.1:1/v1").await;
        if let crate::inference::Backend::OpenAi { context_length, .. } = &mut plan.backend {
            *context_length = 128;
        }
        let (_cancel, mut cancellation) = tokio::sync::watch::channel(false);
        assert!(generate(
            &state,
            &plan,
            final_answer(&c, &m),
            &Channel::new(|_| Ok(())),
            &mut cancellation
        )
        .await
        .unwrap()
        .is_none());
        assert_eq!(
            state.database().unwrap().presentations(&c).unwrap()[0].format,
            "markdown"
        );
        let (_cancel, mut cancellation) = tokio::sync::watch::channel(true);
        assert!(generate(
            &state,
            &plan,
            final_answer(&c, &m),
            &Channel::new(|_| Ok(())),
            &mut cancellation
        )
        .await
        .unwrap()
        .is_none());
    }
    #[tokio::test]
    async fn cancellation_and_idle_timeout_set_failure_metadata_and_stop_work() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for cancel in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}/v1", listener.local_addr().unwrap());
            let (_root, state, plan, c, m) = fixture(&base).await;
            let (sender, mut receiver) = tokio::sync::watch::channel(false);
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buffer = [0; 65536];
                let _ = socket.read(&mut buffer).await.unwrap();
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"root = Response([Text(\\\"First\\\")\"}}]}\n\n").await.unwrap();
                if cancel {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    sender.send(true).unwrap();
                }
                std::future::pending::<()>().await;
            });
            let result = generate_with_limits(
                &state,
                &plan,
                final_answer(&c, &m),
                &Channel::new(|_| Ok(())),
                &mut receiver,
                Limits {
                    request: Duration::from_secs(2),
                    idle: Duration::from_millis(100),
                    total: Duration::from_secs(2),
                },
            )
            .await
            .unwrap()
            .unwrap();
            server.abort();
            assert!(!result.complete);
            let records = state.database().unwrap().presentations(&c).unwrap();
            assert_eq!(
                records[0].generation_status,
                if cancel { "interrupted" } else { "error" }
            );
            assert!(records[0].diagnostic.as_ref().unwrap().contains(if cancel {
                "interrupted"
            } else {
                "timed out"
            }));
        }
    }
    #[tokio::test]
    async fn rejection_is_owned_idempotent_and_auto_backs_off_for_exact_model() {
        let (_root, state, plan, c, m) = fixture("http://127.0.0.1:1/v1").await;
        let store = state.database().unwrap();
        store
            .save_presentation(&Record {
                message_id: m.clone(),
                format: "openui".into(),
                format_version: 1,
                library_version: LIBRARY_VERSION.into(),
                source_text: "Authoritative answer".into(),
                ui_program: "root = Response([])".into(),
                interactive_state: json!({}),
                generation_status: "complete".into(),
                diagnostic: None,
            })
            .unwrap();
        store
            .reject_presentation("other", &m, "root = Response([])")
            .unwrap();
        assert_eq!(
            store.presentations(&c).unwrap()[0].generation_status,
            "complete"
        );
        store
            .reject_presentation(&c, &m, "different program")
            .unwrap();
        assert_eq!(
            store.presentations(&c).unwrap()[0].generation_status,
            "complete"
        );
        store
            .reject_presentation(&c, &m, "root = Response([])")
            .unwrap();
        store
            .reject_presentation(&c, &m, "root = Response([])")
            .unwrap();
        let count: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM session_events WHERE event_type='presentation_rejected'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        for i in 0..3 {
            store.append_session_event(&crate::sessions::SessionEvent{id:uuid::Uuid::new_v4().to_string(),tool_call_id:None,ignorable:false,conversation_id:c.clone(),seq:0,run_id:None,step_id:Some("presentation".into()),event_type:"presentation_end".into(),payload:json!({"messageId":format!("failed-{i}"),"modelKey":model_key(&plan),"transportStatus":"error"}),created_at:crate::store::now()}).unwrap();
        }
        drop(store);
        assert!(!eligible(&state, &plan, Mode::Auto));
        assert!(eligible(&state, &plan, Mode::Visual));
    }
    #[tokio::test]
    async fn formatter_handles_native_responses_and_claude_protocols_without_tools() {
        let ui = "root = Response([Text(\"Authoritative answer\")])";
        let responses = format!(
            "data: {}\n\n",
            json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"message","id":"m","role":"assistant","content":[{"type":"output_text","text":ui,"annotations":[]}]}],"usage":{"input_tokens":30,"output_tokens":8}}})
        );
        let claude=[json!({"type":"message_start","message":{"id":"m","role":"assistant","usage":{"input_tokens":30,"output_tokens":0}}}),json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":ui}}),json!({"type":"content_block_stop","index":0}),json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":8}}),json!({"type":"message_stop"})].iter().map(|event|format!("data: {event}\n\n")).collect::<String>();
        for (format, sse) in [
            (crate::providers::OPENAI_RESPONSES, responses),
            (crate::providers::CLAUDE_MESSAGES, claude),
        ] {
            let (base, requests, server) = crate::test_support::scripted_sse(vec![sse]).await;
            let (_root, state, plan, c, m) = fixture_protocol(&base, format).await;
            let (_sender, mut cancellation) = tokio::sync::watch::channel(false);
            let result = generate(
                &state,
                &plan,
                final_answer(&c, &m),
                &Channel::new(|_| Ok(())),
                &mut cancellation,
            )
            .await
            .unwrap()
            .unwrap();
            server.await.unwrap();
            assert!(result.complete, "{format}");
            assert_eq!(result.usage.input_tokens, Some(30));
            assert_eq!(result.usage.output_tokens, Some(8));
            assert_eq!(
                state.database().unwrap().presentations(&c).unwrap()[0].ui_program,
                ui
            );
            assert!(requests.lock().unwrap()[0].body.get("tools").is_none());
        }
    }
    #[tokio::test]
    async fn rich_profile_downgrades_to_core_without_expanding_the_context_budget() {
        let sse = format!(
            "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"delta":{"content":"root = Response([Text(\"Authoritative answer\")])"}}]}),
            json!({"choices":[{"delta":{},"finish_reason":"stop"}]})
        );
        let (base, requests, server) = crate::test_support::scripted_sse(vec![sse]).await;
        let (_root, state, mut plan, c, m) = fixture(&base).await;
        plan.preferences.max_tokens = 512;
        let core_minimum = plan
            .backend
            .count_tokens(&json!({"messages":[{"role":"user","content":spec("core")}]}))
            .await
            .unwrap();
        let capacity = core_minimum as u32 + 1024;
        if let crate::inference::Backend::OpenAi { context_length, .. } = &mut plan.backend {
            *context_length = capacity;
        }
        let (_sender, mut cancellation) = tokio::sync::watch::channel(false);
        let mut input = final_answer(&c, &m);
        input.question = "Compare the values";
        let result = generate(
            &state,
            &plan,
            input,
            &Channel::new(|_| Ok(())),
            &mut cancellation,
        )
        .await
        .unwrap()
        .unwrap();
        server.await.unwrap();
        assert!(result.complete);
        assert_eq!(plan.backend.context_capacity(), capacity);
        let store = state.database().unwrap();
        let profile:String=store.connection.query_row("SELECT json_extract(payload,'$.profile') FROM session_events WHERE event_type='presentation_start'",[],|row|row.get(0)).unwrap();
        assert_eq!(profile, "core");
        assert_eq!(requests.lock().unwrap().len(), 1);
    }
}
