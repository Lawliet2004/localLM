#[cfg(test)]
mod audit_20261108_subagents {
    use super::*;
    use crate::inference::InferenceProvider;
    use serde_json::json;

    fn backend(base: String, responses: bool) -> crate::inference::Backend {
        crate::inference::Backend::OpenAi { responses,
            client: reqwest::Client::builder().no_proxy().build().unwrap(), base_url: base,
            key: String::new(), model_id:"fixture".into(), context_length:32768,
            max_output_tokens:Some(1024), capabilities:crate::provider_capabilities::ModelCapabilities::legacy(true) }
    }
    fn text_wire(text: &str) -> String {
        format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"delta":{"content":text}}]}),
            json!({"choices":[{"delta":{},"finish_reason":"stop"}]}))
    }

    #[tokio::test]
    async fn child_does_not_separate_inline_thinking_from_answer() {
        let (base, _, server) = crate::test_support::scripted_sse(vec![text_wire("<think>Internal draft</think>Final answer")]).await;
        let backend = backend(base, false);
        let response = backend.stream(&backend.payload(&json!({"messages":[{"role":"user","content":"answer"}],"stream":true})).unwrap()).await.unwrap();
        let mut answer = String::new();
        let outcome = collect_stream(&backend, response, &mut answer, &mut ChildCancel::none()).await.unwrap();
        assert_eq!(outcome.finish, "stop");
        assert_eq!(answer, "<think>Internal draft</think>Final answer");
        server.await.unwrap();
        println!("CONFIRMED: child output includes inline thinking as final answer content");
    }

    #[tokio::test]
    async fn child_valid_final_schema_is_polluted_by_earlier_tool_commentary() {
        let first = format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
            json!({"choices":[{"delta":{"content":"I will inspect the file.","tool_calls":[{"index":0,"id":"c1","type":"function","function":{"name":"workspace_read_file","arguments":"{\"path\":\"a.txt\"}"}}]}}]}),
            json!({"choices":[{"delta":{},"finish_reason":"tool_calls"}]}));
        let (base, _, server) = crate::test_support::scripted_sse(vec![first, text_wire("{\"ok\":true}")]).await;
        let backend = backend(base, false);
        let mut answer = String::new();
        let mut messages = vec![json!({"role":"user","content":"Return JSON after reading."})];
        for round in 0..2 {
            let response = backend.stream(&backend.payload(&json!({"messages":messages,"stream":true})).unwrap()).await.unwrap();
            let outcome = collect_stream(&backend, response, &mut answer, &mut ChildCancel::none()).await.unwrap();
            messages.push(outcome.assistant_message);
            if round == 0 { messages.push(json!({"role":"tool","tool_call_id":"c1","content":"file contents"})); }
        }
        let schema = json!({"type":"object","properties":{"ok":{"type":"boolean"}},"required":["ok"],"additionalProperties":false});
        check_output_schema(&schema, "{\"ok\":true}").unwrap();
        assert!(check_output_schema(&schema, &answer).is_err());
        assert_eq!(answer, "I will inspect the file.{\"ok\":true}");
        server.await.unwrap();
        println!("CONFIRMED: valid final JSON fails child schema validation after preceding tool commentary");
    }

    #[tokio::test]
    async fn child_text_fallback_conflicts_with_native_responses_replay() {
        let output = json!([{"type":"message","id":"msg1","role":"assistant","content":[{"type":"output_text","text":"<function=workspace_read_file>{\"path\":\"a.txt\"}</function>","annotations":[]}]}]);
        let wire = format!("data: {}\n\n", json!({"type":"response.completed","response":{"status":"completed","output":output}}));
        let (base, _, server) = crate::test_support::scripted_sse(vec![wire]).await;
        let backend = backend(base, true);
        let response = backend.stream(&backend.payload(&json!({"messages":[{"role":"user","content":"Read it."}],"stream":true})).unwrap()).await.unwrap();
        let outcome = collect_stream(&backend, response, &mut String::new(), &mut ChildCancel::none()).await.unwrap();
        assert_eq!(outcome.calls.len(), 1);
        let error = backend.payload(&json!({"messages":[outcome.assistant_message,
            {"role":"tool","tool_call_id":outcome.calls[0].id,"content":"file contents"}],"stream":true})).unwrap_err();
        assert!(error.contains("conflict"), "{error}");
        server.await.unwrap();
        println!("CONFIRMED: child's accepted text fallback makes its next native Responses request fail: {error}");
    }
}
