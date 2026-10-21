use futures_util::StreamExt;
use serde_json::Value;
use std::time::Duration;

type CountCell = std::sync::Arc<tokio::sync::Mutex<Option<(std::time::Instant, u64)>>>;
struct TokenCountCache {
    entries: std::collections::HashMap<[u8; 32], (std::time::Instant, CountCell)>,
    capacity: usize,
}

impl TokenCountCache {
    fn cell(&mut self, key: [u8; 32]) -> Option<CountCell> {
        if let Some((used, cell)) = self.entries.get_mut(&key) {
            *used = std::time::Instant::now();
            return Some(cell.clone());
        }
        if self.entries.len() >= self.capacity {
            // In-flight and queued callers keep their entry alive. Evict only
            // idle entries so concurrent identical requests share one count.
            let oldest = self
                .entries
                .iter()
                .filter(|(_, (_, cell))| std::sync::Arc::strong_count(cell) == 1)
                .min_by_key(|(_, (used, _))| *used)
                .map(|(key, _)| *key);
            self.entries.remove(&oldest?);
        }
        let cell = std::sync::Arc::new(tokio::sync::Mutex::new(None));
        self.entries
            .insert(key, (std::time::Instant::now(), cell.clone()));
        Some(cell)
    }
}

fn count_cache_key(endpoint: &str, key: &str, payload: &Value) -> Result<[u8; 32], String> {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    // Endpoint + fresh runtime authentication isolate reloads, even if a port
    // is reused. Store only a digest, never prompts or credentials.
    for part in [
        endpoint.as_bytes(),
        key.as_bytes(),
        &serde_json::to_vec(payload).map_err(|error| error.to_string())?,
    ] {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part);
    }
    Ok(digest.finalize().into())
}

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
    let cap = context
        .saturating_sub(prompt_headroom(context))
        .clamp(1, 32768);
    max_tokens.clamp(1, cap)
}

const COUNT_TTL: Duration = Duration::from_secs(300);

/// Normalize both preflight counts and send-time checks at the shared boundary.
/// Physical slots and streaming controls never change the rendered token count
/// and must not split the same prompt into separate cache entries.
fn normalized_count_payload(payload: &Value) -> Value {
    let mut normalized = payload.clone();
    if let Some(object) = normalized.as_object_mut() {
        for field in ["cache_prompt", "id_slot", "timings", "stream_options"] {
            object.remove(field);
        }
    }
    normalized
}

/// Fetch-or-reuse behind a per-key cell: the mutex is the coalescing point.
/// Only a fresh entry returns without fetching; a queued caller observes
/// whatever the lock holder left — a fresh count, an expired one, or, after a
/// failed or cancelled request, nothing — and refetches under the same lock.
async fn cell_count<F, Fut>(cell: &CountCell, fetch: F) -> Result<u64, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<u64, String>>,
{
    cell_count_with_ttl(cell, COUNT_TTL, fetch).await
}

async fn cell_count_with_ttl<F, Fut>(
    cell: &CountCell,
    ttl: Duration,
    fetch: F,
) -> Result<u64, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<u64, String>>,
{
    let mut cached = cell.lock().await;
    if let Some((created, count)) = *cached {
        if created.elapsed() < ttl {
            return Ok(count);
        }
    }
    let count = fetch().await?;
    *cached = Some((std::time::Instant::now(), count));
    Ok(count)
}

/// Count the payload through the runtime without enforcing the budget, so
/// preflight breakdowns can report an overflow instead of failing a send.
pub async fn count_only(
    client: &reqwest::Client,
    endpoint: &str,
    key: &str,
    payload: &Value,
) -> Result<u64, String> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<TokenCountCache>> =
        std::sync::OnceLock::new();
    let payload = normalized_count_payload(payload);
    let cache_key = count_cache_key(endpoint, key, &payload)?;
    let cache = CACHE.get_or_init(|| {
        std::sync::Mutex::new(TokenCountCache {
            entries: Default::default(),
            capacity: 256,
        })
    });
    let cell = cache
        .lock()
        .ok()
        .and_then(|mut cache| cache.cell(cache_key));
    if let Some(cell) = cell {
        return cell_count(&cell, || count_with_retry(client, endpoint, key, &payload)).await;
    }
    count_with_retry(client, endpoint, key, &payload).await
}

async fn count_with_retry(
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
        return Err(runtime_prompt_error(
            status,
            &String::from_utf8_lossy(&bytes),
            key,
        ));
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

/// Conservative remote preflight estimate. Image data travels as image input,
/// so its base64 bytes must not be counted as text. Reserve 8192 tokens per
/// image; provider-reported usage remains authoritative.
pub fn estimate_remote_tokens(payload: &serde_json::Value) -> u64 {
    fn strip(value: &mut serde_json::Value, images: &mut u64) {
        if value["type"] == "image_url"
            || value["type"] == "input_image"
            || (value["type"] == "image" && value["source"]["type"] == "base64")
        {
            *images += 1;
            *value = serde_json::json!({"type":"text","text":"[image input]"});
            return;
        }
        match value {
            serde_json::Value::Array(items) => {
                for item in items {
                    strip(item, images);
                }
            }
            serde_json::Value::Object(object) => {
                for item in object.values_mut() {
                    strip(item, images);
                }
            }
            _ => {}
        }
    }
    let mut bounded = payload.clone();
    let mut images = 0;
    strip(&mut bounded, &mut images);
    (bounded.to_string().len() as u64)
        .div_ceil(4)
        .max(1)
        .saturating_add(images * 8192)
}

#[cfg(test)]
mod tests {
    #[test]
    fn image_preflight_reserves_image_tokens_without_counting_base64_as_text() {
        let make = |bytes: usize| serde_json::json!({"messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":format!("data:image/png;base64,{}","A".repeat(bytes))}}]}]});
        let small = super::estimate_remote_tokens(&make(10));
        assert_eq!(small, super::estimate_remote_tokens(&make(1_000_000)));
        assert!((8192..8500).contains(&small));
        let native = |bytes: usize| serde_json::json!({"input":[{"role":"user","content":[{"type":"input_image","image_url":format!("data:image/png;base64,{}","A".repeat(bytes))}]}]});
        let small = super::estimate_remote_tokens(&native(10));
        assert_eq!(small, super::estimate_remote_tokens(&native(1_000_000)));
        assert!((8192..8500).contains(&small));
        assert!(
            super::estimate_remote_tokens(
                &serde_json::json!({"input":[{"role":"user","content":"long text".repeat(10000)}]})
            ) > small
        );
    }

    use super::*;
    #[test]
    fn token_count_keys_isolate_runtime_reloads_and_every_payload_change() {
        let payload = serde_json::json!({"messages":[{"role":"user","content":"Hello"}]});
        let first = count_cache_key("http://127.0.0.1:1", "runtime-one", &payload).unwrap();
        assert_ne!(
            first,
            count_cache_key("http://127.0.0.1:1", "runtime-two", &payload).unwrap()
        );
        assert_ne!(
            first,
            count_cache_key("http://127.0.0.1:2", "runtime-one", &payload).unwrap()
        );
        assert_ne!(
            first,
            count_cache_key(
                "http://127.0.0.1:1",
                "runtime-one",
                &serde_json::json!({"messages":[],"tools":[]})
            )
            .unwrap()
        );
        assert_eq!(
            first,
            count_cache_key("http://127.0.0.1:1", "runtime-one", &payload).unwrap()
        );
    }

    #[test]
    fn count_cache_is_bounded_and_never_evicts_an_active_request() {
        let mut cache = TokenCountCache {
            entries: Default::default(),
            capacity: 1,
        };
        let active = cache.cell([1; 32]).unwrap();
        assert!(cache.cell([2; 32]).is_none());
        assert!(std::sync::Arc::ptr_eq(
            &active,
            &cache.cell([1; 32]).unwrap()
        ));
        drop(active);
        assert!(cache.cell([2; 32]).is_some());
        assert_eq!(cache.entries.len(), 1);
        assert!(!cache.entries.contains_key(&[1; 32]));
    }

    #[tokio::test]
    async fn concurrent_and_repeated_preflights_share_one_exact_token_count() {
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
                        break;
                    }
                }
            }
            let body = r#"{"input_tokens":123}"#;
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            // Listener closes here: any extra network request would fail.
        });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let payload = serde_json::json!({"messages":[{"role":"user","content":"日本語"}]});
        let (a, b, c) = tokio::join!(
            count_only(&client, &endpoint, "coalescing-key", &payload),
            count_only(&client, &endpoint, "coalescing-key", &payload),
            count_only(&client, &endpoint, "coalescing-key", &payload),
        );
        assert_eq!((a.unwrap(), b.unwrap(), c.unwrap()), (123, 123, 123));
        server.await.unwrap();
        assert_eq!(
            count_only(&client, &endpoint, "coalescing-key", &payload)
                .await
                .unwrap(),
            123
        );
    }
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
        // every first reasoning-model turn: 2545 + 8192 > 8192.
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

    /// An `Instant` that already looks `age` old so TTL tests never sleep.
    fn empty_cell() -> CountCell {
        std::sync::Arc::new(tokio::sync::Mutex::new(None))
    }

    #[tokio::test]
    async fn expired_entries_are_refetched_and_fresh_entries_are_not() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let cell = empty_cell();
        *cell.lock().await = Some((std::time::Instant::now(), 111));
        let fetches = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = fetches.clone();
        // A zero TTL makes this entry expired on every platform, including a
        // freshly booted Windows host whose Instant cannot subtract 5 minutes.
        let count = cell_count_with_ttl(&cell, Duration::ZERO, move || {
            counter.fetch_add(1, Ordering::SeqCst);
            async { Ok(222u64) }
        })
        .await
        .unwrap();
        assert_eq!(count, 222);
        assert_eq!(fetches.load(Ordering::SeqCst), 1);
        {
            let cached = cell.lock().await;
            let (created, stored) = *cached.as_ref().unwrap();
            assert_eq!(stored, 222);
            assert!(created.elapsed() < COUNT_TTL);
        }
        // The refreshed entry is served without another fetch.
        let counter = fetches.clone();
        let hit = cell_count(&cell, move || {
            counter.fetch_add(1, Ordering::SeqCst);
            async { Ok(999u64) }
        })
        .await
        .unwrap();
        assert_eq!(hit, 222);
        assert_eq!(fetches.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_cancelled_first_request_releases_the_cell_for_queued_callers() {
        let cell = empty_cell();
        let (started, ready) = tokio::sync::oneshot::channel::<()>();
        let leader = {
            let cell = cell.clone();
            tokio::spawn(async move {
                cell_count(&cell, move || async move {
                    let _ = started.send(());
                    std::future::pending::<Result<u64, String>>().await
                })
                .await
            })
        };
        ready.await.unwrap();
        // Queue a caller on the same cell before the leader is cancelled.
        let queued = {
            let cell = cell.clone();
            tokio::spawn(async move { cell_count(&cell, || async { Ok(7u64) }).await })
        };
        tokio::task::yield_now().await;
        leader.abort();
        assert!(leader.await.unwrap_err().is_cancelled());
        let count = tokio::time::timeout(Duration::from_secs(30), queued)
            .await
            .expect("queued caller starved behind a cancelled request")
            .unwrap()
            .unwrap();
        assert_eq!(count, 7);
        assert_eq!(cell.lock().await.as_ref().unwrap().1, 7);
    }

    #[tokio::test]
    async fn failed_counts_never_become_cached_successes() {
        let cell = empty_cell();
        let error = cell_count(&cell, || async { Err("runtime rejected".to_string()) })
            .await
            .unwrap_err();
        assert_eq!(error, "runtime rejected");
        assert!(cell.lock().await.is_none());
        // The next caller runs its own fetch instead of inheriting a failure.
        assert_eq!(cell_count(&cell, || async { Ok(9u64) }).await.unwrap(), 9);
        // An expired entry whose refetch fails surfaces the error, not stale data.
        *cell.lock().await = Some((std::time::Instant::now(), 4));
        let stale = cell_count_with_ttl(&cell, Duration::ZERO, || async {
            Err("still down".to_string())
        })
        .await
        .unwrap_err();
        assert_eq!(stale, "still down");
    }

    #[test]
    fn envelope_flags_do_not_split_count_cache_keys() {
        let bare = serde_json::json!({"messages":[{"role":"user","content":"hi"}],"temperature":0.2,"top_p":0.9,"max_tokens":256,"stream":true,"tools":[]});
        let mut enveloped = bare.clone();
        enveloped["cache_prompt"] = serde_json::json!(true);
        enveloped["id_slot"] = serde_json::json!(1);
        enveloped["timings"] = serde_json::json!(true);
        enveloped["stream_options"] = serde_json::json!({"include_usage":true});
        assert_eq!(
            count_cache_key("http://e", "k", &normalized_count_payload(&enveloped)).unwrap(),
            count_cache_key("http://e", "k", &normalized_count_payload(&bare)).unwrap(),
        );
        // Only the request envelope is normalized; semantic fields still isolate.
        let mut other = bare.clone();
        other["max_tokens"] = serde_json::json!(512);
        assert_ne!(
            count_cache_key("http://e", "k", &normalized_count_payload(&other)).unwrap(),
            count_cache_key("http://e", "k", &normalized_count_payload(&bare)).unwrap(),
        );
    }
}
