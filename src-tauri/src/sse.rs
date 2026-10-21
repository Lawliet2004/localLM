#[derive(Default)]
pub struct SseDecoder {
    pending: Vec<u8>,
    data: Vec<String>,
    event_size: usize,
    skip_lf: bool,
    started: bool,
}
impl SseDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<String>, String> {
        let mut events = Vec::new();
        for byte in bytes {
            if self.skip_lf {
                self.skip_lf = false;
                if *byte == b'\n' {
                    continue;
                }
            }
            self.event_size += 1;
            if self.event_size > 1_048_576 {
                return Err("Stream event exceeds 1 MiB.".into());
            }
            if *byte != b'\n' && *byte != b'\r' {
                self.pending.push(*byte);
                continue;
            }
            // CR, LF and CRLF are all SSE line endings. Remember CR across
            // network chunks so its optional LF does not become a blank line.
            self.skip_lf = *byte == b'\r';
            let mut line =
                std::str::from_utf8(&self.pending).map_err(|_| "Invalid UTF-8 in event stream.")?;
            // SSE permits one leading BOM, even split across network chunks.
            if !self.started {
                self.started = true;
                line = line.strip_prefix('\u{feff}').unwrap_or(line);
            }
            if line.is_empty() {
                if !self.data.is_empty() {
                    events.push(self.data.join("\n"));
                    self.data.clear();
                }
                self.event_size = 0;
            } else if let ("data", value) = line.split_once(':').unwrap_or((line, "")) {
                self.data
                    .push(value.strip_prefix(' ').unwrap_or(value).to_owned());
            }
            self.pending.clear();
        }
        Ok(events)
    }
    pub fn finish(&self) -> Result<(), String> {
        if !self.pending.is_empty() || !self.data.is_empty() {
            return Err("Model disconnected in the middle of a stream event.".into());
        }
        Ok(())
    }
}

/// One protocol path for foreground chat and child agents. Completion markers
/// are distinct from finish reasons: native Claude ends with message_stop,
/// whereas OpenAI-compatible servers end with [DONE]. Usage-only events after
/// finish_reason must still be consumed before the completion marker.
pub struct ModelStream {
    claude: bool,
    responses: Option<crate::responses_adapter::Stream>,
    pub done: bool,
    finish_seen: bool,
    tool_indices: std::collections::BTreeMap<u64, u64>,
    tool_has_input: std::collections::HashSet<u64>,
    native_content: crate::claude_adapter::StreamContent,
}

impl ModelStream {
    pub fn new(claude: bool) -> Self {
        Self {
            claude,
            responses: None,
            done: false,
            finish_seen: false,
            tool_indices: Default::default(),
            tool_has_input: Default::default(),
            native_content: Default::default(),
        }
    }

    pub fn for_backend(backend: &crate::inference::Backend) -> Self {
        let mut stream = Self::new(matches!(backend, crate::inference::Backend::Claude { .. }));
        if backend.is_responses() {
            stream.responses = Some(Default::default());
        }
        stream
    }

    pub fn push(&mut self, data: &str) -> Result<Option<serde_json::Value>, String> {
        use serde_json::{json, Value};
        if self.done {
            return Err("Model sent data after stream completion.".into());
        }
        if data.trim() == "[DONE]" {
            return self.complete();
        }
        let mut value: Value =
            serde_json::from_str(data).map_err(|_| "Invalid JSON in model stream.")?;
        // Keep errors as values so the backend can redact its credentials.
        if value.get("error").is_some() {
            return Ok(Some(value));
        }
        let normalized = if let Some(responses) = self.responses.as_mut() {
            let normalized = responses.push(&value)?;
            if responses.completed {
                self.done = true;
                self.finish_seen = true;
            }
            normalized
        } else if self.claude {
            self.native_content.observe(&value)?;
            let kind = value["type"].as_str().unwrap_or("").to_owned();
            if kind == "message_stop" {
                return self.complete();
            }
            if kind == "content_block_start" && value["content_block"]["type"] == "tool_use" {
                let block = value["index"]
                    .as_u64()
                    .ok_or("Claude tool block has no index.")?;
                if self.tool_indices.len() >= 8 || self.tool_indices.contains_key(&block) {
                    return Err("Claude returned too many or duplicate tool blocks.".into());
                }
                let index = self.tool_indices.len() as u64;
                self.tool_indices.insert(block, index);
                let input = &value["content_block"]["input"];
                let input_object = input
                    .as_object()
                    .ok_or("Claude tool input must be a JSON object.")?;
                let arguments = if !input_object.is_empty() {
                    self.tool_has_input.insert(block);
                    input.to_string()
                } else {
                    String::new()
                };
                Some(json!({"choices":[{"delta":{"tool_calls":[{"index":index,
                    "id":value["content_block"]["id"],"type":"function",
                    "function":{"name":value["content_block"]["name"],"arguments":arguments}}]}}]}))
            } else if kind == "content_block_stop" {
                let block = value["index"]
                    .as_u64()
                    .ok_or("Claude content block has no index.")?;
                if let Some(index) = self.tool_indices.get(&block) {
                    if self.tool_has_input.insert(block) {
                        return Ok(Some(
                            json!({"choices":[{"delta":{"tool_calls":[{"index":index,"function":{"arguments":"{}"}}]}}]}),
                        ));
                    }
                }
                None
            } else {
                if kind == "content_block_delta" && value["delta"]["type"] == "input_json_delta" {
                    let block = value["index"]
                        .as_u64()
                        .ok_or("Claude tool delta has no index.")?;
                    let index = self
                        .tool_indices
                        .get(&block)
                        .ok_or("Claude tool delta arrived before its tool block.")?;
                    let fragment = value["delta"]["partial_json"]
                        .as_str()
                        .ok_or("Claude tool arguments fragment must be a string.")?;
                    if !fragment.is_empty() {
                        self.tool_has_input.insert(block);
                    }
                    value["index"] = json!(index);
                }
                crate::claude_adapter::normalize_sse_event(&value.to_string())?
            }
        } else {
            Some(value)
        };
        if let Some(value) = &normalized {
            if value["choices"][0]["finish_reason"]
                .as_str()
                .is_some_and(|reason| !reason.is_empty())
            {
                self.finish_seen = true;
            }
        }
        Ok(normalized)
    }

    fn complete(&mut self) -> Result<Option<serde_json::Value>, String> {
        if self
            .responses
            .as_ref()
            .is_some_and(|responses| !responses.completed)
        {
            return Err("Responses stream ended before its native terminal event; no unfinished tool was executed.".into());
        }
        if !self.finish_seen {
            return Err("Model stream completed without a finish reason; unfinished tools were not executed.".into());
        }
        self.done = true;
        Ok(None)
    }

    pub fn assistant_message(
        &self,
        text: &str,
        calls: &[crate::tool_calls::ToolCall],
    ) -> serde_json::Value {
        let mut message = serde_json::json!({"role":"assistant","content":text,
            "tool_calls":calls.iter().map(|call| call.model_value()).collect::<Vec<_>>()});
        if let Some(content) = self
            .responses
            .as_ref()
            .and_then(|responses| responses.replay())
        {
            message["locallm_responses_output"] = content;
        }
        if let Some(content) = self.native_content.replay(calls) {
            message["locallm_claude_content"] = content;
        }
        message
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leading_bom_does_not_drop_the_first_event_at_any_chunk_boundary() {
        let wire = "\u{feff}data: first\n\ndata: 日本\n\ndata: [DONE]\n\n".as_bytes();
        for boundary in 0..=wire.len() {
            let mut decoder = SseDecoder::default();
            let mut events = decoder.push(&wire[..boundary]).unwrap();
            events.extend(decoder.push(&wire[boundary..]).unwrap());
            assert_eq!(events, ["first", "日本", "[DONE]"], "boundary {boundary}");
            decoder.finish().unwrap();
        }
        let mut decoder = SseDecoder::default();
        assert_eq!(decoder.push(b"data\n\n").unwrap(), [""]);
    }

    #[test]
    fn claude_initial_block_content_and_subsequent_deltas_are_preserved() {
        let mut protocol = ModelStream::new(true);
        let events = [
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":"café "}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"日本"}}"#,
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"thinking","thinking":"reason"}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"thinking_delta","thinking":"ing"}}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
            r#"{"type":"message_stop"}"#,
        ];
        let mut text = String::new();
        let mut reasoning = String::new();
        for event in events {
            if let Some(value) = protocol.push(event).unwrap() {
                text.push_str(
                    value["choices"][0]["delta"]["content"]
                        .as_str()
                        .unwrap_or(""),
                );
                reasoning.push_str(
                    value["choices"][0]["delta"]["reasoning_content"]
                        .as_str()
                        .unwrap_or(""),
                );
            }
        }
        assert!(protocol.done);
        assert_eq!(text, "café 日本");
        assert_eq!(reasoning, "reasoning");
    }

    #[test]
    fn claude_thinking_signatures_and_block_order_survive_the_tool_followup() {
        use serde_json::json;
        let events = [
            json!({"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"Think ","signature":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"carefully"}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"opaque-"}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"signature"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"content_block_start","index":1,"content_block":{"type":"text","text":"Checking"}}),
            json!({"type":"content_block_stop","index":1}),
            json!({"type":"content_block_start","index":2,"content_block":{"type":"tool_use","id":"a","name":"lookup","input":{}}}),
            json!({"type":"content_block_delta","index":2,"delta":{"type":"input_json_delta","partial_json":"{\"q\":\"日本\"}"}}),
            json!({"type":"content_block_stop","index":2}),
            json!({"type":"content_block_start","index":3,"content_block":{"type":"redacted_thinking","data":"opaque-data"}}),
            json!({"type":"content_block_stop","index":3}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"}}),
            json!({"type":"message_stop"}),
        ];
        let mut protocol = ModelStream::new(true);
        let mut calls = crate::tool_calls::ToolCalls::default();
        for event in events {
            if let Some(value) = protocol.push(&event.to_string()).unwrap() {
                calls.push(&value["choices"][0]["delta"]).unwrap();
            }
        }
        assert!(protocol.done);
        let assistant = protocol.assistant_message("Checking", &calls.finish().unwrap());
        let request = crate::claude_adapter::to_messages_request(
            &json!({"messages":[
                {"role":"user","content":"Find it"}, assistant,
                {"role":"tool","tool_call_id":"a","content":"Found"}
            ]}),
            "claude-sonnet-4-6",
            None,
        );
        assert_eq!(
            request["messages"][1]["content"],
            json!([
                {"type":"thinking","thinking":"Think carefully","signature":"opaque-signature"},
                {"type":"text","text":"Checking"},
                {"type":"tool_use","id":"a","name":"lookup","input":{"q":"日本"}},
                {"type":"redacted_thinking","data":"opaque-data"}
            ])
        );
        assert_eq!(request["messages"][2]["content"][0]["tool_use_id"], "a");
    }

    #[test]
    fn malformed_native_content_fragments_fail_without_panicking() {
        let mut protocol = ModelStream::new(true);
        assert!(protocol.push(r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"x"}}"#).is_err());
        protocol.push(r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}"#).unwrap();
        assert!(protocol.push(r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":[]}}"#).is_err());
        assert!(protocol.push(r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"wrong block"}}"#).is_err());
    }

    #[test]
    fn bare_carriage_returns_and_mixed_line_endings_work_bytewise() {
        let mut decoder = SseDecoder::default();
        let wire = ": heartbeat\rdata: café\rdata: 日本\r\rdata: tail\r\n\r\ndata: [DONE]\n\n";
        let mut events = Vec::new();
        for byte in wire.as_bytes() {
            events.extend(decoder.push(&[*byte]).unwrap());
        }
        assert_eq!(events, ["café\n日本", "tail", "[DONE]"]);
        decoder.finish().unwrap();
    }

    #[test]
    fn claude_parallel_tools_use_dense_indices_and_empty_arguments_are_valid() {
        use serde_json::json;
        let mut protocol = ModelStream::new(true);
        let mut calls = crate::tool_calls::ToolCalls::default();
        for offset in 0..8 {
            let index = offset * 3 + 1; // text/thinking blocks are not tool indices
            let event = json!({"type":"content_block_start","index":index,"content_block":{"type":"tool_use","id":format!("call_{offset}"),"name":"clock","input":{}}});
            let normalized = protocol.push(&event.to_string()).unwrap().unwrap();
            calls.push(&normalized["choices"][0]["delta"]).unwrap();
            let normalized = protocol
                .push(&json!({"type":"content_block_stop","index":index}).to_string())
                .unwrap()
                .unwrap();
            calls.push(&normalized["choices"][0]["delta"]).unwrap();
        }
        protocol
            .push(r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#)
            .unwrap();
        protocol.push(r#"{"type":"message_stop"}"#).unwrap();
        assert!(protocol.done);
        let calls = calls.finish().unwrap();
        assert_eq!(calls.len(), 8);
        assert!(calls.iter().all(|call| call.arguments == json!({})));
        assert_eq!(calls[7].id, "call_7");
    }

    #[test]
    fn claude_fragments_survive_bytewise_sse_and_preserve_usage_after_finish() {
        use serde_json::json;
        let events = [
            json!({"type":"message_start","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":90}}}),
            json!({"type":"content_block_start","index":11,"content_block":{"type":"tool_use","id":"a","name":"lookup","input":{}}}),
            json!({"type":"content_block_delta","index":11,"delta":{"type":"input_json_delta","partial_json":"{\"q\":"}}),
            json!({"type":"content_block_delta","index":11,"delta":{"type":"input_json_delta","partial_json":"\"café 日本\"}"}}),
            json!({"type":"content_block_stop","index":11}),
            json!({"type":"message_delta","delta":{"stop_reason":"tool_use"},"usage":{"output_tokens":12}}),
            json!({"type":"message_stop"}),
        ];
        let wire = events
            .iter()
            .map(|event| format!("data: {event}\r\n\r\n"))
            .collect::<String>();
        let mut decoder = SseDecoder::default();
        let mut protocol = ModelStream::new(true);
        let mut calls = crate::tool_calls::ToolCalls::default();
        let mut usage = crate::inference::InferenceUsage::default();
        for byte in wire.as_bytes() {
            for event in decoder.push(&[*byte]).unwrap() {
                if let Some(value) = protocol.push(&event).unwrap() {
                    calls.push(&value["choices"][0]["delta"]).unwrap();
                    usage.observe(&value);
                }
            }
        }
        decoder.finish().unwrap();
        assert!(protocol.done);
        assert_eq!(calls.finish().unwrap()[0].arguments["q"], "café 日本");
        assert_eq!(usage.input_tokens, Some(100));
        assert_eq!(usage.cached_tokens, Some(90));
        assert_eq!(usage.output_tokens, Some(12));
    }

    #[test]
    fn openai_usage_only_tail_does_not_replace_finish_reason() {
        let mut protocol = ModelStream::new(false);
        protocol
            .push(r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#)
            .unwrap();
        let usage = protocol.push(r#"{"choices":[],"usage":{"prompt_tokens":100,"prompt_tokens_details":{"cached_tokens":80}}}"#).unwrap().unwrap();
        assert_eq!(usage["usage"]["prompt_tokens_details"]["cached_tokens"], 80);
        assert!(!protocol.done);
        protocol.push("[DONE]").unwrap();
        assert!(protocol.done);
        assert!(protocol.push("[DONE]").is_err());
    }

    #[test]
    fn premature_completion_and_orphan_tool_deltas_fail_closed() {
        assert!(ModelStream::new(false).push("[DONE]").is_err());
        assert!(ModelStream::new(true)
            .push(r#"{"type":"message_stop"}"#)
            .is_err());
        assert!(ModelStream::new(true).push(r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{}"}}"#).is_err());
        assert!(ModelStream::new(true).push(r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"a","name":"clock","input":[]}}"#).is_err());
        let mut malformed = ModelStream::new(true);
        malformed.push(r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"a","name":"clock","input":{}}}"#).unwrap();
        assert!(malformed.push(r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":null}}"#).is_err());
        let mut protocol = ModelStream::new(true);
        // A usage-only message_delta is not a terminal finish reason.
        protocol
            .push(r#"{"type":"message_delta","delta":{},"usage":{"output_tokens":12}}"#)
            .unwrap();
        assert!(protocol.push(r#"{"type":"message_stop"}"#).is_err());
    }

    #[test]
    fn provider_errors_remain_available_for_backend_redaction() {
        for claude in [true, false] {
            let value = ModelStream::new(claude)
                .push(r#"{"type":"error","error":{"message":"private-token"}}"#)
                .unwrap()
                .unwrap();
            assert_eq!(value["error"]["message"], "private-token");
        }
    }

    #[test]
    fn preserves_unicode_across_network_boundaries_and_accepts_crlf() {
        let mut decoder = SseDecoder::default();
        let bytes = "data: {\"text\":\"你好\"}\r\n\r\ndata: [DONE]\n\n".as_bytes();
        let mut events = Vec::new();
        for byte in bytes {
            events.extend(decoder.push(&[*byte]).unwrap());
        }
        assert_eq!(events, ["{\"text\":\"你好\"}", "[DONE]"]);
        assert!(decoder.finish().is_ok());
    }

    #[test]
    fn ignores_comments_and_joins_multiple_data_lines() {
        let mut decoder = SseDecoder::default();
        assert_eq!(
            decoder
                .push(b": heartbeat\nevent: message\ndata: one\ndata: two\n\n")
                .unwrap(),
            ["one\ntwo"]
        );
    }

    #[test]
    fn rejects_invalid_utf8_oversized_events_and_truncated_streams() {
        assert!(SseDecoder::default().push(b"data: \xff\n\n").is_err());
        assert!(SseDecoder::default().push(&vec![b'x'; 1_048_577]).is_err());
        let mut decoder = SseDecoder::default();
        decoder.push(b"data: incomplete").unwrap();
        assert!(decoder.finish().is_err());
    }
}

#[cfg(test)]
mod responses_tests {
    use super::*;
    use crate::inference::InferenceProvider;
    use serde_json::{json, Value};
    fn backend() -> crate::inference::Backend {
        let provider = crate::providers::ProviderConnection {
            id: "fixture".into(),
            name: "Fixture".into(),
            api_format: crate::providers::OPENAI_RESPONSES.into(),
            base_url: "http://127.0.0.1:1919/v1".into(),
            verified: true,
            last_tested_at: None,
            models: vec![],
            has_api_key: false,
        };
        crate::inference::Backend::openai(
            &provider,
            String::new(),
            &crate::providers::RemoteModel {
                controls: None,
                id: "fixture-model".into(),
                context_length: Some(8192),
                max_output_tokens: None,
                tool_support: crate::providers::ToolSupport::Supported,
                supports_images: false,
            },
        )
        .unwrap()
    }
    #[test]
    fn responses_backend_stream_finishes_on_native_event_with_terminal_usage_and_call() {
        let backend = backend();
        assert!(backend.is_responses());
        let payload = backend
            .payload(&json!({"messages":[{"role":"user","content":"Hello"}],"max_tokens":64}))
            .unwrap();
        assert_eq!(payload["model"], "fixture-model");
        assert!(payload.get("messages").is_none());
        let completed = json!({"type":"response.completed","response":{"status":"completed","output":[{"type":"function_call","id":"fc-item","call_id":"call-owned","name":"echo","arguments":"{}"}],"usage":{"input_tokens":12,"output_tokens":4,"input_tokens_details":{"cached_tokens":2}}}});
        let wire = format!("data: {completed}\r\n\r\n");
        for boundary in 0..=wire.len() {
            let mut decoder = SseDecoder::default();
            let mut events = decoder.push(&wire.as_bytes()[..boundary]).unwrap();
            events.extend(decoder.push(&wire.as_bytes()[boundary..]).unwrap());
            decoder.finish().unwrap();
            let mut stream = ModelStream::for_backend(&backend);
            let mut calls = crate::tool_calls::ToolCalls::default();
            let mut usage = crate::inference::InferenceUsage::default();
            for event in events {
                let normalized = stream.push(&event).unwrap().unwrap();
                calls.push(&normalized["choices"][0]["delta"]).unwrap();
                usage.observe(&normalized);
                assert_eq!(normalized["choices"][0]["finish_reason"], "tool_calls");
            }
            assert!(stream.done);
            let calls = calls.finish().unwrap();
            assert_eq!(calls[0].id, "call-owned");
            assert_eq!(usage.input_tokens, Some(12));
            assert_eq!(usage.cached_tokens, Some(2));
            let replay = stream.assistant_message("", &calls);
            assert_eq!(replay["locallm_responses_output"][0]["id"], "fc-item");
        }
    }
    #[test]
    fn done_marker_cannot_complete_an_unfinished_native_response() {
        let backend = backend();
        let mut stream = ModelStream::for_backend(&backend);
        assert!(stream.push("[DONE]").is_err());
        assert!(!stream.done);
        let failed: Value =
            json!({"type":"response.failed","response":{"error":{"message":"failed"}}});
        assert!(stream
            .push(&failed.to_string())
            .unwrap()
            .unwrap()
            .get("error")
            .is_some());
        assert!(!stream.done);
    }
}
