use futures_util::StreamExt;
use serde_json::Value;
use std::time::Duration;

pub fn validate_budget(input: u64, response: u32, context: u32) -> Result<(), String> {
    let required = input.saturating_add(u64::from(response));
    if required > u64::from(context) {
        return Err(format!("Context limit exceeded: the prompt uses {input} tokens and the response reserves {response}, but the loaded context holds {context}. Shorten the message, select fewer tools or skills, start a new conversation, or increase the context and reload the model."));
    }
    Ok(())
}

/// Room kept for system prompt, tools, and a first user message so a response
/// reserve cannot consume the entire window. Typical first local turns are
/// about 2.5k tokens; 4k covers that with margin on 8k+ contexts.
pub fn prompt_headroom(context: u32) -> u32 {
    if context <= 1 {
        return 0;
    }
    if context <= 4096 {
        return context / 2;
    }
    4096.max(context / 4).min(context / 2)
}

/// Cap a configured response budget so prompt plus reserve can still fit.
pub fn fit_response_budget(max_tokens: u32, context: u32) -> u32 {
    let cap = context.saturating_sub(prompt_headroom(context)).clamp(1, 32768);
    max_tokens.clamp(1, cap)
}

/// Count the payload through the runtime without enforcing the budget, so
/// preflight breakdowns can report an overflow instead of failing a send.
pub async fn count_only(
    client: &reqwest::Client,
    endpoint: &str,
    key: &str,
    payload: &Value,
) -> Result<u64, String> {
    match count_only_inner(client, endpoint, key, payload).await {
        Ok(count) => Ok(count),
        Err(first_error) => {
            tokio::time::sleep(Duration::from_millis(500)).await;
            count_only_inner(client, endpoint, key, payload)
                .await
                .map_err(|_| first_error)
        }
    }
}

async fn count_only_inner(
    client: &reqwest::Client,
    endpoint: &str,
    key: &str,
    payload: &Value,
) -> Result<u64, String> {
    let response = client
        .post(format!("{endpoint}/v1/chat/completions/input_tokens"))
        .bearer_auth(key)
        .json(payload)
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|_| {
            "Could not count context tokens. Check that the model runtime is responding."
                .to_string()
        })?;
    let status = response.status();
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "Token-count response was interrupted.")?;
        if bytes.len() + chunk.len() > 4096 {
            return Err("Token-count response exceeds its size limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    if !status.is_success() {
        return Err(runtime_prompt_error(status, &String::from_utf8_lossy(&bytes), key));
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid token-count response.")?;
    value["input_tokens"]
        .as_u64()
        .ok_or_else(|| "Runtime did not return a valid input token count.".to_string())
}

/// Prefer the runtime's chat-template or tokenizer message over a bare HTTP status.
pub fn runtime_prompt_error(status: reqwest::StatusCode, body: &str, key: &str) -> String {
    let detail = crate::providers::sanitize_text(&runtime_error_detail(body), key);
    if detail.is_empty() {
        format!(
            "The runtime rejected this prompt ({status}). Check the chat template and the runtime log."
        )
    } else {
        format!("The runtime rejected this prompt ({status}): {detail}")
    }
}

fn runtime_error_detail(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        if let Some(message) = value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .or_else(|| value.pointer("/error").and_then(Value::as_str))
            .or_else(|| value.get("message").and_then(Value::as_str))
        {
            return message.trim().to_string();
        }
    }
    trimmed.to_string()
}

pub async fn check(
    client: &reqwest::Client,
    endpoint: &str,
    key: &str,
    payload: &Value,
    response_tokens: u32,
    context: u32,
) -> Result<u64, String> {
    let input = count_only(client, endpoint, key, payload).await?;
    validate_budget(input, response_tokens, context)?;
    Ok(input)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn counts_the_authenticated_payload_and_rejects_invalid_responses() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (body, valid) in [
            (r#"{"input_tokens":123}"#.to_string(), true),
            (r#"{"input_tokens":-1}"#.to_string(), false),
            ("{}".into(), false),
            ("x".repeat(5000), false),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let endpoint = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut buffer = [0; 2048];
                    let count = stream.read(&mut buffer).await.unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                    assert!(request.len() < 16384);
                    if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length: "))
                            .unwrap()
                            .parse()
                            .unwrap();
                        if request.len() >= end + 4 + length {
                            assert!(headers.contains("authorization: bearer test-key"));
                            assert!(headers.starts_with("post /v1/chat/completions/input_tokens "));
                            let payload: Value =
                                serde_json::from_slice(&request[end + 4..end + 4 + length])
                                    .unwrap();
                            let response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                            let _ = stream.write_all(response.as_bytes()).await;
                            return payload;
                        }
                    }
                }
            });
            let payload = serde_json::json!({"messages":[{"role":"user","content":"日本語"}],"tools":[{"type":"function","function":{"name":"read","parameters":{"type":"object"}}}]});
            let client = reqwest::Client::builder().no_proxy().build().unwrap();
            let result = check(&client, &endpoint, "test-key", &payload, 512, 8192).await;
            assert_eq!(result.is_ok(), valid);
            if valid {
                assert_eq!(result.unwrap(), 123);
            }
            assert_eq!(server.await.unwrap(), payload);
        }
    }
    #[test]
    fn reserves_response_tokens_and_rejects_overflow_without_truncating() {
        assert!(validate_budget(7000, 1192, 8192).is_ok());
        let error = validate_budget(7001, 1192, 8192).unwrap_err();
        assert!(error.contains("7001") && error.contains("1192") && error.contains("8192"));
        assert!(validate_budget(u64::MAX, 1, 8192).is_err());
        assert!(validate_budget(8193, 1, 8192).is_err());
    }

    #[test]
    fn response_budget_leaves_room_for_a_typical_first_prompt() {
        // 8192 context with an 8192 reserve is the failure mode that blocked
        // every first ZAYA (and leftover-budget) turn: 2545 + 8192 > 8192.
        assert_eq!(fit_response_budget(8192, 8192), 4096);
        assert!(validate_budget(2545, fit_response_budget(8192, 8192), 8192).is_ok());
        assert_eq!(fit_response_budget(2048, 8192), 2048);
        assert_eq!(fit_response_budget(8192, 65536), 8192);
        assert_eq!(fit_response_budget(12000, 2048), 1024);
        assert_eq!(fit_response_budget(1, 128), 1);
    }

    #[test]
    fn runtime_prompt_error_surfaces_chat_template_detail() {
        let body = r#"{"error":{"code":500,"message":"Failed to apply chat template from model's metadata: System message must be at the beginning.","type":"server_error"}}"#;
        let error = runtime_prompt_error(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            body,
            "secret-key",
        );
        assert!(error.contains("System message must be at the beginning"));
        assert!(error.contains("500"));
        assert!(!error.contains("secret-key"));
        let missing = runtime_prompt_error(reqwest::StatusCode::INTERNAL_SERVER_ERROR, "", "");
        assert!(missing.contains("chat template"));
        let query = runtime_prompt_error(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            r#"{"error":{"message":"No user query found in messages."}}"#,
            "",
        );
        assert!(query.contains("No user query found in messages"));
    }

    #[tokio::test]
    async fn count_failure_includes_runtime_error_body() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            loop {
                let mut buffer = [0; 2048];
                let count = stream.read(&mut buffer).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&buffer[..count]);
                if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                    let length: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    if request.len() >= end + 4 + length {
                        let body = r#"{"error":{"message":"No user query found in messages."}}"#;
                        let response = format!(
                            "HTTP/1.1 500 Internal Server Error\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(response.as_bytes()).await;
                        return;
                    }
                }
            }
        });
        let payload = serde_json::json!({"messages":[{"role":"system","content":"only"}]});
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let error = count_only(&client, &endpoint, "test-key", &payload)
            .await
            .unwrap_err();
        assert!(error.contains("No user query found in messages"));
        server.await.unwrap();
    }
}
