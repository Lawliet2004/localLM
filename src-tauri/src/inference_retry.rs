//! Bounded retries of inference HTTP responses, before any stream is handed
//! to a consumer. Connection, request and timeout failures are retried only
//! before accepting a response. Partial 2xx streams are recovered by the shared
//! engine from committed history; tool execution is never replayed here.
use chrono::{DateTime, Months, NaiveDateTime, TimeZone, Utc};
use reqwest::{RequestBuilder, Response};
use std::time::Duration;

const MAX_ATTEMPTS: u8 = 3;
const WAIT_BUDGET: Duration = Duration::from_secs(8);
const BASE_DELAY: Duration = Duration::from_millis(250);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Stop {
    Success,
    NonTransient,
    Exhausted,
    ServerDelay,
    Uncloneable,
    Transport,
}

#[derive(Clone, Debug)]
pub(crate) struct RetryTrace {
    pub attempts: u8,
    pub waited: Duration,
    pub stop: Stop,
}

impl RetryTrace {
    pub fn note(&self) -> String {
        let retried = self.attempts.saturating_sub(1);
        let prefix = if retried > 0 {
            format!(
                " {retried} bounded pre-stream HTTP {} ({} ms backoff).",
                if retried == 1 {
                    "retry was attempted"
                } else {
                    "retries were attempted"
                },
                self.waited.as_millis()
            )
        } else {
            String::new()
        };
        let reason = match self.stop {
            Stop::ServerDelay => " Provider Retry-After or required backoff exceeds the remaining 8-second wait budget; no early retry was sent.",
            Stop::Uncloneable => " The request body cannot be replayed; no automatic retry was sent.",
            _ => "",
        };
        format!("{prefix}{reason}")
    }
}

pub(crate) struct SendError {
    error: reqwest::Error,
    trace: RetryTrace,
}
impl SendError {
    pub fn is_timeout(&self) -> bool {
        self.error.is_timeout()
    }
    pub fn note(&self) -> String {
        self.trace.note()
    }
}

pub(crate) fn response_note(response: &Response) -> String {
    response
        .extensions()
        .get::<RetryTrace>()
        .map(RetryTrace::note)
        .unwrap_or_default()
}

#[derive(Clone)]
pub(crate) struct ErrorBody(pub String);

async fn capture_error_body(response: &mut Response) -> bool {
    const LIMIT: usize = 16_384;
    let mut bytes = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(1), async {
        while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
            if bytes.len().saturating_add(chunk.len()) > LIMIT {
                bytes.extend_from_slice(&chunk[..LIMIT.saturating_sub(bytes.len())]);
                return Err(());
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok(())
    })
    .await;
    let incomplete = !matches!(read, Ok(Ok(())));
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    let permanent = permanent_error(&text);
    let marker = if incomplete {
        "\n[Provider error body incomplete or exceeded its bound.]"
    } else {
        ""
    };
    let limit = LIMIT - marker.len();
    if text.len() > limit {
        let mut boundary = limit;
        while !text.is_char_boundary(boundary) {
            boundary -= 1;
        }
        text.truncate(boundary);
    }
    text.push_str(marker);
    response.extensions_mut().insert(ErrorBody(text.clone()));
    permanent
}

fn permanent_error(body: &str) -> bool {
    let value = serde_json::from_str::<serde_json::Value>(body).unwrap_or_default();
    let error = value.get("error").unwrap_or(&value);
    for key in ["code", "type"] {
        if matches!(
            error[key].as_str(),
            Some(
                "invalid_request_error"
                    | "authentication_error"
                    | "permission_error"
                    | "not_found_error"
                    | "insufficient_quota"
                    | "context_length_exceeded"
                    | "invalid_api_key"
                    | "model_not_found"
                    | "unsupported_parameter"
            )
        ) {
            return true;
        }
    }
    let lower = body.to_lowercase();
    [
        "chat template",
        "chat_template",
        "system message must be at the beginning",
        "roles must alternate",
        "maximum context length",
        "context length exceeded",
        "invalid role",
    ]
    .iter()
    .any(|text| lower.contains(text))
}

fn transient(status: reqwest::StatusCode) -> bool {
    matches!(status.as_u16(), 408 | 429)
        || (status.is_server_error()
            && !matches!(status.as_u16(), 501 | 505 | 506 | 508 | 510 | 511))
}

/// HTTP Retry-After accepts delta-seconds or an HTTP-date. Modern IMF-fixdate
/// and the two legacy date spellings are accepted; malformed values use the
/// normal bounded backoff. Overflowing delta-seconds remain a *long* delay,
/// never an accidental zero-second retry.
fn retry_after(value: &str, now: DateTime<Utc>) -> Option<Duration> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if value.bytes().all(|b| b.is_ascii_digit()) {
        return Some(Duration::from_secs(
            value.parse::<u64>().unwrap_or(u64::MAX),
        ));
    }
    if value.len() > 128 {
        return None;
    }
    let date = DateTime::parse_from_rfc2822(value)
        .map(|date| date.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            let (weekday, tail) = value.split_once(", ")?;
            if ![
                "Monday",
                "Tuesday",
                "Wednesday",
                "Thursday",
                "Friday",
                "Saturday",
                "Sunday",
            ]
            .contains(&weekday)
            {
                return None;
            }
            let (date, time) = tail.split_once(' ')?;
            let (prefix, year) = date.rsplit_once('-')?;
            if year.len() != 2 || !year.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            // RFC5322 obsolete years:00–49 =>2000–2049,50–99 =>1950–1999;
            // HTTP additionally moves a result >50 years ahead into the past.
            let short = year.parse::<i32>().ok()?;
            let mut year = short + if short <= 49 { 2000 } else { 1900 };
            let parse = |year| {
                NaiveDateTime::parse_from_str(
                    &format!("{prefix}-{year} {time}"),
                    "%d-%b-%Y %H:%M:%S GMT",
                )
                .ok()
                .map(|date| Utc.from_utc_datetime(&date))
            };
            let candidate = parse(year)?;
            if candidate > now.checked_add_months(Months::new(600))? {
                year -= 100;
            }
            parse(year)
        })
        .or_else(|| {
            NaiveDateTime::parse_from_str(value, "%a %b %e %H:%M:%S %Y")
                .ok()
                .map(|date| Utc.from_utc_datetime(&date))
        })?;
    Some(date.signed_duration_since(now).to_std().unwrap_or_default())
}

fn delay(response: &Response, retry: u8, waited: Duration) -> Option<Duration> {
    let remaining = WAIT_BUDGET.checked_sub(waited)?;
    let jitter = Duration::from_millis((uuid::Uuid::new_v4().as_u128() % 126) as u64);
    let backoff = BASE_DELAY
        .saturating_mul(1u32 << retry.min(8))
        .saturating_add(jitter);
    let mut requested = Duration::ZERO;
    // Multiple valid Retry-After headers are conservatively combined by max.
    // Do not cap a server's minimum to a shorter, more aggressive delay.
    for value in response.headers().get_all(reqwest::header::RETRY_AFTER) {
        if let Some(after) = value.to_str().ok().and_then(|s| retry_after(s, Utc::now())) {
            requested = requested.max(after);
        }
    }
    let delay = backoff.max(requested);
    if delay <= remaining {
        Some(delay)
    } else {
        None
    }
}

pub(crate) async fn send(request: RequestBuilder) -> Result<Response, SendError> {
    let template = request.try_clone();
    let mut pending = request;
    let mut trace = RetryTrace {
        attempts: 0,
        waited: Duration::ZERO,
        stop: Stop::NonTransient,
    };
    loop {
        // Every pending request is produced by the original builder or clone;
        // the request cannot change endpoint/auth/body between attempts.
        trace.attempts += 1;
        trace.stop = Stop::Transport;
        let mut response = match pending.send().await {
            Ok(response) => response,
            Err(error) => {
                // This is a model request before a response body exists. Tool
                // execution is outside this retry boundary and is never replayed.
                if (error.is_connect() || error.is_timeout() || error.is_request())
                    && !error.is_builder() && trace.attempts < MAX_ATTEMPTS {
                    if let Some(next) = template.as_ref().and_then(RequestBuilder::try_clone) {
                        let wait = BASE_DELAY.saturating_mul(1 << (trace.attempts - 1));
                        tokio::time::sleep(wait).await;
                        trace.waited += wait;
                        pending = next;
                        continue;
                    }
                }
                return Err(SendError { error, trace });
            }
        };
        // Some runtimes report permanent prompt/template/validation failures as
        // HTTP500, and quota exhaustion as429. Inspect a bounded error body and
        // retain it for the existing diagnostic boundary before deciding.
        let permanent = transient(response.status()) && capture_error_body(&mut response).await;
        trace.stop = if response.status().is_success() {
            Stop::Success
        } else if !transient(response.status()) || permanent {
            Stop::NonTransient
        } else if trace.attempts >= MAX_ATTEMPTS {
            Stop::Exhausted
        } else if template.is_none() {
            Stop::Uncloneable
        } else {
            Stop::ServerDelay
        };
        if transient(response.status())
            && !permanent
            && trace.attempts < MAX_ATTEMPTS
            && template.is_some()
        {
            if let Some(wait) = delay(&response, trace.attempts - 1, trace.waited) {
                if let Some(next) = template.as_ref().and_then(RequestBuilder::try_clone) {
                    pending = next;
                    // A bounded error body is retained only while deciding;
                    // release the failed response before the cancellable wait.
                    drop(response);
                    tokio::time::sleep(wait).await;
                    trace.waited = trace.waited.saturating_add(wait);
                    continue;
                }
                trace.stop = Stop::Uncloneable;
            }
        }
        response.extensions_mut().insert(trace);
        return Ok(response);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inference::{Backend, InferenceProvider};
    use serde_json::{json, Value};
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[derive(Clone)]
    struct Reply {
        status: u16,
        headers: String,
        body: String,
        claimed: Option<usize>,
        hold_headers: bool,
        hold_body: bool,
    }
    impl Reply {
        fn new(status: u16, headers: &str, body: &str) -> Self {
            Self {
                status,
                headers: headers.into(),
                body: body.into(),
                claimed: None,
                hold_headers: false,
                hold_body: false,
            }
        }
    }
    struct Server {
        url: String,
        seen: Arc<Mutex<Vec<(std::time::Instant, String, Value)>>>,
        task: tokio::task::JoinHandle<()>,
    }
    impl Drop for Server {
        fn drop(&mut self) {
            self.task.abort();
        }
    }
    impl Server {
        fn count(&self) -> usize {
            self.seen.lock().unwrap().len()
        }
        async fn wait(&self, count: usize) {
            tokio::time::timeout(Duration::from_secs(3), async {
                while self.count() < count {
                    tokio::time::sleep(Duration::from_millis(2)).await;
                }
            })
            .await
            .unwrap();
        }
    }
    async fn server(replies: Vec<Reply>) -> Server {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let records = seen.clone();
        let task = tokio::spawn(async move {
            let mut children = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((mut socket,_)) = accepted else { break; };
                        let records = records.clone();
                        let replies = replies.clone();
                        children.spawn(async move {
                            let mut request = Vec::new();
                            let (headers,payload) = loop {
                                let mut buffer = [0u8;4096];
                                let Ok(count) = socket.read(&mut buffer).await else { return; };
                                if count == 0 || request.len() > 1024*1024 { return; }
                                request.extend_from_slice(&buffer[..count]);
                                if let Some(end) = request.windows(4).position(|p| p == b"\r\n\r\n") {
                                    let headers = String::from_utf8_lossy(&request[..end]).to_string();
                                    let length = headers.lines().find_map(|line| line.to_lowercase().strip_prefix("content-length:").map(str::trim).map(str::to_string)).and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
                                    if request.len() >= end + 4 + length { break (headers,serde_json::from_slice(&request[end+4..end+4+length]).unwrap_or(Value::Null)); }
                                }
                            };
                            let index = { let mut records = records.lock().unwrap(); let i = records.len(); records.push((std::time::Instant::now(),headers,payload)); i };
                            let reply = &replies[index.min(replies.len()-1)];
                            if reply.status == 0 { return; }
                            if reply.hold_headers { tokio::time::sleep(Duration::from_secs(60)).await; return; }
                            let header = format!("HTTP/1.1 {} Fixture\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n",reply.status,reply.claimed.unwrap_or(reply.body.len()),reply.headers);
                            let _ = socket.write_all(header.as_bytes()).await;
                            if reply.hold_body { tokio::time::sleep(Duration::from_secs(60)).await; return; }
                            let _ = socket.write_all(reply.body.as_bytes()).await;
                        });
                    }
                    _ = children.join_next(), if !children.is_empty() => (),
                }
            }
        });
        Server { url, seen, task }
    }

    fn backend(kind: &str, url: &str) -> Backend {
        let client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(4))
            .build()
            .unwrap();
        match kind {
            "local" => Backend::Local {
                    bonsai_profile: false,
                client,
                endpoint: url.into(),
                key: "fixture-secret".into(),
                context_length: 8192,
            },
            "claude" => Backend::Claude {
                client,
                base_url: url.into(),
                key: "fixture-secret".into(),
                model_id: "fixture".into(),
                context_length: 8192,
                max_output_tokens: None,
                capabilities: crate::provider_capabilities::ModelCapabilities::legacy(true),
            },
            "subscription" => Backend::Subscription {
                client,
                provider_name: "Fixture".into(),
                base_url: url.into(),
                access_token: "fixture-secret".into(),
                account_id: Some("fixture-account".into()),
                model_id: "fixture".into(),
                context_length: 8192,
                max_output_tokens: None,
                capabilities: crate::provider_capabilities::ModelCapabilities::legacy(true),
            },
            _ => Backend::OpenAi {
                responses: false,
                client,
                base_url: url.into(),
                key: "fixture-secret".into(),
                model_id: "fixture".into(),
                context_length: 8192,
                max_output_tokens: None,
                capabilities: crate::provider_capabilities::ModelCapabilities::legacy(true),
            },
        }
    }

    #[test]
    fn retry_after_dates_overflow_and_nontransient_classification_are_explicit() {
        let now = Utc.with_ymd_and_hms(1994, 11, 6, 8, 49, 30).unwrap();
        for value in [
            "Sun, 06 Nov 1994 08:49:37 GMT",
            "Sunday, 06-Nov-94 08:49:37 GMT",
            "Sun Nov  6 08:49:37 1994",
        ] {
            assert_eq!(
                retry_after(value, now),
                Some(Duration::from_secs(7)),
                "{value}"
            );
        }
        assert_eq!(retry_after("7", now), Some(Duration::from_secs(7)));
        assert_eq!(retry_after("0", now), Some(Duration::ZERO));
        assert_eq!(
            retry_after(&"9".repeat(200), now),
            Some(Duration::from_secs(u64::MAX))
        );
        for value in ["", "tomorrow", "-1", "1.5"] {
            assert_eq!(retry_after(value, now), None);
        }
        assert_eq!(
            retry_after("Sun, 06 Nov 1994 08:49:29 GMT", now),
            Some(Duration::ZERO)
        );
        let current = Utc.with_ymd_and_hms(2026, 10, 19, 0, 0, 0).unwrap();
        assert_eq!(
            retry_after("Thursday, 01-Jan-70 00:00:00 GMT", current),
            Some(Duration::ZERO)
        );
        assert_eq!(
            retry_after("Wednesday, 01-Jan-85 00:00:00 GMT", current),
            Some(Duration::ZERO)
        );
        let historic = Utc.with_ymd_and_hms(1999, 12, 31, 23, 59, 59).unwrap();
        assert_eq!(
            retry_after("Saturday, 01-Jan-00 00:00:00 GMT", historic),
            Some(Duration::from_secs(1))
        );
        let historic = Utc.with_ymd_and_hms(1990, 1, 1, 0, 0, 0).unwrap();
        assert_eq!(
            retry_after("Sunday, 01-Jan-45 00:00:00 GMT", historic),
            Some(Duration::ZERO)
        );
        for code in [400, 401, 403, 404, 409, 422, 501, 505, 506, 508, 510, 511] {
            assert!(!transient(reqwest::StatusCode::from_u16(code).unwrap()));
        }
        for code in [408, 429, 500, 502, 503, 504, 529, 599] {
            assert!(transient(reqwest::StatusCode::from_u16(code).unwrap()));
        }
        for body in [
            r#"{"error":{"code":"insufficient_quota"}}"#,
            r#"{"error":{"type":"invalid_request_error"}}"#,
            r#"{"error":{"message":"System message must be at the beginning."}}"#,
        ] {
            assert!(permanent_error(body));
        }
        assert!(!permanent_error(
            r#"{"error":{"type":"server_error","message":"temporarily overloaded"}}"#
        ));
    }

    #[tokio::test]
    async fn every_backend_retries_before_stream_and_preserves_request_protocol_and_payload() {
        for kind in ["local", "openai", "claude", "subscription"] {
            let fixture = server(vec![
                Reply::new(if kind == "claude" { 529 } else { 503 }, "", "busy"),
                Reply::new(
                    200,
                    "Content-Type: text/event-stream\r\n",
                    "data: fixture\n\n",
                ),
            ])
            .await;
            let backend = backend(kind, &fixture.url);
            let payload =
                json!({"messages":[{"role":"user","content":"fixture input"}],"stream":true});
            let response = backend.stream(&payload).await.unwrap();
            let trace = response.extensions().get::<RetryTrace>().unwrap();
            assert_eq!(trace.attempts, 2);
            assert_eq!(trace.stop, Stop::Success);
            assert!(trace.waited >= BASE_DELAY);
            assert_eq!(response.text().await.unwrap(), "data: fixture\n\n");
            let records = fixture.seen.lock().unwrap();
            assert_eq!(records.len(), 2, "{kind}");
            assert_eq!(records[0].1, records[1].1);
            assert_eq!(records[0].2, payload);
            assert_eq!(records[1].2, payload);
            assert!(records[1].0.duration_since(records[0].0) >= BASE_DELAY);
            if kind == "claude" {
                assert!(records[0].1.starts_with("POST /v1/messages "));
                assert!(records[0]
                    .1
                    .to_lowercase()
                    .contains("x-api-key: fixture-secret"));
            } else {
                assert!(records[0].1.starts_with("POST /v1/chat/completions "));
            }
        }
    }

    #[tokio::test]
    async fn retries_are_bounded_and_error_reporting_retains_attempts_without_credentials() {
        let fixture = server(vec![Reply::new(503, "", "fixture-secret temporarily busy")]).await;
        let error = backend("openai", &fixture.url)
            .stream(&json!({"stream":true}))
            .await
            .unwrap_err();
        assert_eq!(fixture.count(), 3);
        assert!(
            error.contains("2 bounded pre-stream HTTP retries"),
            "{error}"
        );
        assert!(!error.contains("fixture-secret"));
        assert!(!error.contains("No automatic retry was attempted"));
        {
            let records = fixture.seen.lock().unwrap();
            assert!(records[2].0.duration_since(records[1].0) >= BASE_DELAY * 2);
        }
        let fixture = server(vec![
            Reply::new(503, "", "busy"),
            Reply::new(401, "", "fixture-secret"),
        ])
        .await;
        let error = backend("subscription", &fixture.url)
            .stream(&json!({"stream":true}))
            .await
            .unwrap_err();
        assert_eq!(fixture.count(), 2);
        assert!(error.contains("1 bounded pre-stream HTTP retry"), "{error}");
        assert!(error.contains("Re-authenticate"));
        assert!(!error.contains("fixture-secret"));
    }

    #[tokio::test]
    async fn auth_validation_quota_and_template_errors_are_never_replayed() {
        for (status, body) in [
            (401, "unauthorized"),
            (403, "forbidden"),
            (400, "invalid"),
            (422, "invalid"),
            (
                500,
                r#"{"error":{"message":"System message must be at the beginning."}}"#,
            ),
            (500, r#"{"error":{"type":"invalid_request_error"}}"#),
            (429, r#"{"error":{"code":"insufficient_quota"}}"#),
        ] {
            let fixture = server(vec![Reply::new(status, "", body)]).await;
            assert!(backend("local", &fixture.url)
                .stream(&json!({"stream":true}))
                .await
                .is_err());
            assert_eq!(fixture.count(), 1, "{status}: {body}");
        }
    }

    #[tokio::test]
    // The database guard is explicitly dropped before the await below;
    // `await_holding_lock` does not model an explicit `drop`.
    #[allow(clippy::await_holding_lock)]
    async fn retry_after_is_honored_and_long_server_delays_are_not_shortened() {
        let fixture = server(vec![
            Reply::new(429, "Retry-After: 1\r\n", "busy"),
            Reply::new(200, "", "ok"),
        ])
        .await;
        let response = backend("openai", &fixture.url)
            .stream(&json!({"stream":true}))
            .await
            .unwrap();
        assert_eq!(
            response.extensions().get::<RetryTrace>().unwrap().waited,
            Duration::from_secs(1)
        );
        let records = fixture.seen.lock().unwrap();
        assert!(records[1].0.duration_since(records[0].0) >= Duration::from_secs(1));
        drop(records);
        for value in ["9", "999999999999999999999999999999999999999"] {
            let fixture = server(vec![Reply::new(
                429,
                &format!("Retry-After: {value}\r\n"),
                "busy",
            )])
            .await;
            let error = backend("openai", &fixture.url)
                .stream(&json!({"stream":true}))
                .await
                .unwrap_err();
            assert_eq!(fixture.count(), 1);
            assert!(error.contains("no early retry was sent"), "{error}");
        }
    }

    #[tokio::test]
    async fn cancellation_during_backoff_headers_and_error_body_never_sends_another_request() {
        for mode in ["backoff", "headers", "error_body"] {
            let mut reply = Reply::new(503, "Retry-After: 3\r\n", "busy");
            reply.hold_headers = mode == "headers";
            reply.hold_body = mode == "error_body";
            if reply.hold_body {
                reply.claimed = Some(1000);
            }
            let fixture = server(vec![reply]).await;
            let backend = backend("openai", &fixture.url);
            let request =
                tokio::spawn(async move { backend.stream(&json!({"stream":true})).await });
            fixture.wait(1).await;
            tokio::time::sleep(Duration::from_millis(30)).await;
            let started = std::time::Instant::now();
            request.abort();
            assert!(request.await.unwrap_err().is_cancelled());
            assert!(started.elapsed() < Duration::from_millis(300));
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert_eq!(fixture.count(), 1, "{mode}");
        }
    }

    #[tokio::test]
    async fn partial_success_stream_is_not_retried_but_pre_response_disconnect_recovers() {
        let mut reply = Reply::new(
            200,
            "Content-Type: text/event-stream\r\n",
            "data: partial\n\n",
        );
        reply.claimed = Some(10000);
        let fixture = server(vec![reply]).await;
        let response = backend("openai", &fixture.url)
            .stream(&json!({"stream":true}))
            .await
            .unwrap();
        assert_eq!(
            response.extensions().get::<RetryTrace>().unwrap().attempts,
            1
        );
        assert!(response.text().await.is_err());
        assert_eq!(fixture.count(), 1);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (_socket, _) = listener.accept().await.unwrap();
        });
        assert!(backend("openai", &url)
            .stream(&json!({"stream":true}))
            .await
            .is_err());
        task.await.unwrap();
        let fixture = server(vec![Reply::new(503, "", "busy"), Reply::new(0, "", ""), Reply::new(200, "", "done")]).await;
        let response = backend("openai", &fixture.url)
            .stream(&json!({"stream":true}))
            .await
            .unwrap();
        assert_eq!(
            fixture.count(),
            3,
            "A disconnected model request may retry without rerunning tools"
        );
        assert_eq!(response.extensions().get::<RetryTrace>().unwrap().attempts, 3);
    }
}
