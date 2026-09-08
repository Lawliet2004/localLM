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

pub async fn check(
    client: &reqwest::Client,
    endpoint: &str,
    key: &str,
    payload: &Value,
    response_tokens: u32,
    context: u32,
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
    if !response.status().is_success() {
        return Err(format!("The runtime could not count this prompt ({}). Use the supported llama.cpp runtime and check the selected tools and chat template.", response.status()));
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| "Token-count response was interrupted.")?;
        if bytes.len() + chunk.len() > 4096 {
            return Err("Token-count response exceeds its size limit.".into());
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid token-count response.")?;
    let input = value["input_tokens"]
        .as_u64()
        .ok_or("Runtime did not return a valid input token count.")?;
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
}
