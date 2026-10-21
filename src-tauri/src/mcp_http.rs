//! Bounded `StreamableHttpClient` for remote MCP connectors.
//!
//! The pinned SDK's `post_message` deserializes plain JSON response bodies and
//! reads HTTP error bodies without a byte bound, so a hostile server could
//! allocate unbounded memory before LocalLM's catalog/result limits apply.
//! This wrapper composes the public `StreamableHttpClient` SPI around the same
//! `reqwest::Client` (and nests cleanly under `AuthClient<C>`) while enforcing:
//! - bounded plain-JSON success bodies before deserialization,
//! - bounded non-success error bodies,
//! - the SDK's per-event SSE bound for streamed responses.
//!
//! OAuth challenge/scope handling, reserved-header validation, session-id
//! handling, redirect policy (none — configured on the inner client) and the
//! legacy `server/discover` fallback are preserved verbatim. Requests are
//! never sent twice.
//!
//! Note: the SDK recognizes its own private `BoundedSseStreamError` as a fatal
//! SSE failure; an equivalent custom error is treated as retryable and the
//! transport's default reconnect policy is unbounded. The connector config
//! therefore bounds `retry_config` (see `connectors.rs`) so a persistent GET
//! channel failure — oversized event or dead endpoint — terminates with a real
//! error instead of silently reconnecting forever.

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;
use std::task::{ready, Poll};

use futures_util::stream::{BoxStream, Stream, StreamExt};
use rmcp::model::{
    ClientJsonRpcMessage, ClientRequest, ErrorData, JsonRpcMessage, ServerJsonRpcMessage,
};
use rmcp::transport::streamable_http_client::{
    AuthRequiredError, InsufficientScopeError, SseError, StreamableHttpClient, StreamableHttpError,
    StreamableHttpPostResponse,
};
use sse_stream::{Sse, SseStream};

use reqwest::header::{HeaderName, HeaderValue, ACCEPT, CONTENT_TYPE, WWW_AUTHENTICATE};

/// Mirrors the SDK's default per-event SSE bound (`DEFAULT_MAX_SSE_EVENT_SIZE`
/// is `pub(crate)` and cannot be referenced).
const DEFAULT_MAX_SSE_EVENT_SIZE: usize = 16 * 1024 * 1024;
/// Bound for a plain-JSON response body — aligned with the stdio frame bound
/// (`crate::local_mcp_process::MAX_FRAME_BYTES`) so transports share limits.
pub const MAX_BODY_BYTES: usize = 4 * 1024 * 1024;
/// Bound for non-success error body text.
pub const MAX_ERROR_BODY_BYTES: usize = 256 * 1024;

const EVENT_STREAM_MIME_TYPE: &str = "text/event-stream";
const JSON_MIME_TYPE: &str = "application/json";
const HEADER_SESSION_ID: &str = "Mcp-Session-Id";
const HEADER_LAST_EVENT_ID: &str = "Last-Event-Id";
const HEADER_MCP_PROTOCOL_VERSION: &str = "MCP-Protocol-Version";

/// Reserved headers that user-supplied custom headers must not override
/// (`MCP-Protocol-Version` is injected by the worker and allowed through).
const RESERVED_HEADERS: &[&str] = &[
    "accept",
    HEADER_SESSION_ID,
    HEADER_MCP_PROTOCOL_VERSION,
    HEADER_LAST_EVENT_ID,
];

fn validate_custom_header(name: &HeaderName) -> Result<(), String> {
    if RESERVED_HEADERS
        .iter()
        .any(|reserved| name.as_str().eq_ignore_ascii_case(reserved))
    {
        if name
            .as_str()
            .eq_ignore_ascii_case(HEADER_MCP_PROTOCOL_VERSION)
        {
            return Ok(());
        }
        return Err(name.to_string());
    }
    Ok(())
}

/// Extracts the `scope=` parameter from a `WWW-Authenticate` value (quoted or
/// unquoted), matching the SDK's behavior for insufficient-scope challenges.
fn extract_scope_from_header(header: &str) -> Option<String> {
    let header_lowercase = header.to_ascii_lowercase();
    let scope_key = "scope=";
    let position = header_lowercase.find(scope_key)?;
    let value_slice = &header[position + scope_key.len()..];
    if let Some(stripped) = value_slice.strip_prefix('"') {
        return stripped.find('"').map(|end| stripped[..end].to_string());
    }
    let end = value_slice
        .find(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .unwrap_or(value_slice.len());
    (end > 0).then(|| value_slice[..end].to_string())
}

fn apply_custom_headers(
    mut builder: reqwest::RequestBuilder,
    custom_headers: HashMap<HeaderName, HeaderValue>,
) -> Result<reqwest::RequestBuilder, StreamableHttpError<reqwest::Error>> {
    for (name, value) in custom_headers {
        validate_custom_header(&name).map_err(StreamableHttpError::ReservedHeaderConflict)?;
        builder = builder.header(name, value);
    }
    Ok(builder)
}

/// Attempts to parse `body` as a JSON-RPC error message.
fn parse_json_rpc_error(body: &[u8]) -> Option<ServerJsonRpcMessage> {
    match serde_json::from_slice::<ServerJsonRpcMessage>(body) {
        Ok(message @ JsonRpcMessage::Error(_)) => Some(message),
        _ => None,
    }
}

/// Legacy `server/discover` fallback: a client error without a session turns
/// into a JSON-RPC invalid-request response instead of a transport failure.
fn legacy_discover_response(
    message: &ClientJsonRpcMessage,
    session_was_attached: bool,
    status: reqwest::StatusCode,
    body: &str,
) -> Option<StreamableHttpPostResponse> {
    if session_was_attached
        || !status.is_client_error()
        || matches!(
            status,
            reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN
        )
    {
        return None;
    }
    let ClientJsonRpcMessage::Request(request) = message else {
        return None;
    };
    if !matches!(request.request, ClientRequest::DiscoverRequest(_)) {
        return None;
    }
    let error = ErrorData::invalid_request(
        format!("server/discover rejected with HTTP {status}: {body}"),
        None,
    );
    Some(StreamableHttpPostResponse::Json(
        ServerJsonRpcMessage::error(error, Some(request.id.clone())),
        None,
    ))
}

#[derive(Debug)]
enum BoundedBodyError {
    Source(Box<dyn std::error::Error + Send + Sync>),
    EventTooLarge { max_size: usize },
}

impl std::fmt::Display for BoundedBodyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Source(error) => error.fmt(f),
            Self::EventTooLarge { max_size } => {
                write!(f, "SSE event exceeded the maximum size of {max_size} bytes")
            }
        }
    }
}

impl std::error::Error for BoundedBodyError {}

/// Per-line event-size accounting identical to the SDK's limiter: comment
/// lines are free; data/field lines count toward the retained event size plus
/// the newline the parser inserts when joining multi-line data fields.
struct SseEventSizeLimiter {
    max_size: usize,
    retained_size: usize,
    line_size: usize,
    line_is_comment: bool,
    previous_was_cr: bool,
}

impl SseEventSizeLimiter {
    fn new(max_size: usize) -> Self {
        Self {
            max_size,
            retained_size: 0,
            line_size: 0,
            line_is_comment: false,
            previous_was_cr: false,
        }
    }

    fn observe(&mut self, chunk: &[u8]) -> Result<(), ()> {
        for &byte in chunk {
            if self.previous_was_cr {
                self.previous_was_cr = false;
                if byte == b'\n' {
                    continue;
                }
            }
            match byte {
                b'\r' => {
                    self.finish_line()?;
                    self.previous_was_cr = true;
                }
                b'\n' => self.finish_line()?,
                _ => {
                    if self.line_size == 0 {
                        self.line_is_comment = byte == b':';
                    }
                    self.line_size = self.line_size.saturating_add(1);
                    self.check_limit()?;
                }
            }
        }
        Ok(())
    }

    fn finish_line(&mut self) -> Result<(), ()> {
        if self.line_size == 0 {
            self.retained_size = 0;
        } else if !self.line_is_comment {
            self.retained_size = self
                .retained_size
                .saturating_add(self.line_size)
                .saturating_add(1);
        }
        self.line_size = 0;
        self.line_is_comment = false;
        self.check_limit()
    }

    fn check_limit(&self) -> Result<(), ()> {
        if self.retained_size.saturating_add(self.line_size) > self.max_size {
            Err(())
        } else {
            Ok(())
        }
    }
}

/// Byte-stream wrapper enforcing the per-event bound before `sse-stream`
/// allocates event data — the same enforcement the SDK applies internally.
fn bounded_sse_stream<S, E>(
    stream: S,
    max_event_size: usize,
) -> BoxStream<'static, Result<Sse, SseError>>
where
    S: Stream<Item = Result<bytes::Bytes, E>> + Send + 'static,
    E: std::error::Error + Send + Sync + 'static,
{
    let mut limiter = SseEventSizeLimiter::new(max_event_size);
    let mut stream = Box::pin(stream);
    let mut failed = false;
    let stream = futures_util::stream::poll_fn(move |cx| {
        if failed {
            return Poll::Ready(None);
        }
        match ready!(stream.as_mut().poll_next(cx)) {
            Some(Ok(chunk)) => {
                if limiter.observe(&chunk).is_err() {
                    failed = true;
                    Poll::Ready(Some(Err(BoundedBodyError::EventTooLarge {
                        max_size: limiter.max_size,
                    })))
                } else {
                    Poll::Ready(Some(Ok(chunk)))
                }
            }
            Some(Err(error)) => {
                failed = true;
                Poll::Ready(Some(Err(BoundedBodyError::Source(Box::new(error)))))
            }
            None => Poll::Ready(None),
        }
    });
    SseStream::from_bytes_stream(stream).boxed()
}

/// Read a response body into memory with a hard byte bound. `Content-Length`
/// is checked up front; chunked bodies are cut off mid-stream.
async fn collect_bounded<S, E>(
    stream: S,
    max: usize,
) -> Result<Vec<u8>, StreamableHttpError<reqwest::Error>>
where
    S: Stream<Item = Result<bytes::Bytes, E>> + Send + 'static,
    E: std::error::Error + Send + Sync + 'static,
{
    let mut stream = Box::pin(stream);
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            StreamableHttpError::UnexpectedServerResponse(Cow::Owned(format!(
                "response body read failed: {error}"
            )))
        })?;
        if bytes.len().saturating_add(chunk.len()) > max {
            return Err(StreamableHttpError::UnexpectedServerResponse(Cow::Owned(
                format!("response body exceeds the {max} byte limit"),
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn bounded_body(
    response: reqwest::Response,
    max: usize,
) -> Result<Vec<u8>, StreamableHttpError<reqwest::Error>> {
    if response
        .content_length()
        .is_some_and(|length| length > max as u64)
    {
        return Err(StreamableHttpError::UnexpectedServerResponse(Cow::Owned(
            format!("response body exceeds the {max} byte limit"),
        )));
    }
    collect_bounded(response.bytes_stream(), max).await
}

/// `reqwest::Client` wrapper enforcing bounded reads on the response paths the
/// SDK leaves unbounded. GET/DELETE delegate to the SDK implementation, which
/// already enforces the SSE event bound; POST is reimplemented with bounded
/// plain-JSON and error-body reads.
#[derive(Clone)]
pub struct BoundedHttpClient {
    inner: reqwest::Client,
}

impl BoundedHttpClient {
    pub fn new(inner: reqwest::Client) -> Self {
        Self { inner }
    }
}

impl StreamableHttpClient for BoundedHttpClient {
    type Error = reqwest::Error;

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_token: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        self.get_stream_with_max_sse_event_size(
            uri,
            session_id,
            last_event_id,
            auth_token,
            custom_headers,
            DEFAULT_MAX_SSE_EVENT_SIZE,
        )
        .await
    }

    async fn get_stream_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_token: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        let mut request_builder = self
            .inner
            .get(uri.as_ref())
            .header(ACCEPT, [EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE].join(", "));
        if let Some(session_id) = session_id {
            request_builder = request_builder.header(HEADER_SESSION_ID, session_id.as_ref());
        }
        if let Some(last_event_id) = last_event_id {
            request_builder = request_builder.header(HEADER_LAST_EVENT_ID, last_event_id);
        }
        if let Some(auth_header) = auth_token {
            request_builder = request_builder.bearer_auth(auth_header);
        }
        request_builder = apply_custom_headers(request_builder, custom_headers)?;
        let response = request_builder.send().await?;
        if response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            return Err(StreamableHttpError::ServerDoesNotSupportSse);
        }
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            if let Some(header) = response.headers().get(WWW_AUTHENTICATE) {
                let header = header
                    .to_str()
                    .map_err(|_| {
                        StreamableHttpError::UnexpectedServerResponse(Cow::from(
                            "invalid www-authenticate header value",
                        ))
                    })?
                    .to_string();
                return Err(StreamableHttpError::AuthRequired(AuthRequiredError::new(
                    header,
                )));
            }
        }
        if response.status() == reqwest::StatusCode::FORBIDDEN {
            if let Some(header) = response.headers().get(WWW_AUTHENTICATE) {
                let header_str = header.to_str().map_err(|_| {
                    StreamableHttpError::UnexpectedServerResponse(Cow::from(
                        "invalid www-authenticate header value",
                    ))
                })?;
                let scope = extract_scope_from_header(header_str);
                return Err(StreamableHttpError::InsufficientScope(
                    InsufficientScopeError::new(header_str.to_string(), scope),
                ));
            }
        }
        let response = response.error_for_status()?;
        match response.headers().get(CONTENT_TYPE) {
            Some(ct) => {
                if !ct.as_bytes().starts_with(EVENT_STREAM_MIME_TYPE.as_bytes())
                    && !ct.as_bytes().starts_with(JSON_MIME_TYPE.as_bytes())
                {
                    return Err(StreamableHttpError::UnexpectedContentType(Some(
                        String::from_utf8_lossy(ct.as_bytes()).to_string(),
                    )));
                }
            }
            None => {
                return Err(StreamableHttpError::UnexpectedContentType(None));
            }
        }
        Ok(bounded_sse_stream(
            response.bytes_stream(),
            max_sse_event_size,
        ))
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session: Arc<str>,
        auth_token: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        let mut request_builder = self.inner.delete(uri.as_ref());
        if let Some(auth_header) = auth_token {
            request_builder = request_builder.bearer_auth(auth_header);
        }
        request_builder = request_builder.header(HEADER_SESSION_ID, session.as_ref());
        request_builder = apply_custom_headers(request_builder, custom_headers)?;
        let response = request_builder.send().await?;
        if response.status() == reqwest::StatusCode::METHOD_NOT_ALLOWED {
            return Ok(());
        }
        let _response = response.error_for_status()?;
        Ok(())
    }

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_token: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        self.post_message_with_max_sse_event_size(
            uri,
            message,
            session_id,
            auth_token,
            custom_headers,
            DEFAULT_MAX_SSE_EVENT_SIZE,
        )
        .await
    }

    async fn post_message_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_token: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        let mut request = self
            .inner
            .post(uri.as_ref())
            .header(ACCEPT, [EVENT_STREAM_MIME_TYPE, JSON_MIME_TYPE].join(", "));
        if let Some(auth_header) = auth_token {
            request = request.bearer_auth(auth_header);
        }
        request = apply_custom_headers(request, custom_headers)?;
        let session_was_attached = session_id.is_some();
        if let Some(session_id) = session_id {
            request = request.header(HEADER_SESSION_ID, session_id.as_ref());
        }
        let response = request.json(&message).send().await?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED {
            if let Some(header) = response.headers().get(WWW_AUTHENTICATE) {
                let header = header
                    .to_str()
                    .map_err(|_| {
                        StreamableHttpError::UnexpectedServerResponse(Cow::from(
                            "invalid www-authenticate header value",
                        ))
                    })?
                    .to_string();
                return Err(StreamableHttpError::AuthRequired(AuthRequiredError::new(
                    header,
                )));
            }
        }
        if response.status() == reqwest::StatusCode::FORBIDDEN {
            if let Some(header) = response.headers().get(WWW_AUTHENTICATE) {
                let header_str = header.to_str().map_err(|_| {
                    StreamableHttpError::UnexpectedServerResponse(Cow::from(
                        "invalid www-authenticate header value",
                    ))
                })?;
                let scope = extract_scope_from_header(header_str);
                return Err(StreamableHttpError::InsufficientScope(
                    InsufficientScopeError::new(header_str.to_string(), scope),
                ));
            }
        }
        let status = response.status();
        if matches!(
            status,
            reqwest::StatusCode::ACCEPTED | reqwest::StatusCode::NO_CONTENT
        ) {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        if status == reqwest::StatusCode::NOT_FOUND && session_was_attached {
            return Err(StreamableHttpError::SessionExpired);
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .map(|ct| String::from_utf8_lossy(ct.as_bytes()).to_string());
        let content_length = response.content_length();
        let session_id = response
            .headers()
            .get(HEADER_SESSION_ID)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_string());
        // Spec requires 202 Accepted for these, but some servers return an
        // empty 200; treat it as equivalent to Accepted.
        if status.is_success()
            && content_length == Some(0)
            && matches!(
                message,
                ClientJsonRpcMessage::Notification(_)
                    | ClientJsonRpcMessage::Response(_)
                    | ClientJsonRpcMessage::Error(_)
            )
        {
            return Ok(StreamableHttpPostResponse::Accepted);
        }
        // Non-success responses may carry valid JSON-RPC error payloads that
        // should surface as McpError rather than a transport failure. The body
        // read is bounded; malformed/oversized bodies fail fast instead of
        // leaving the request pending until timeout.
        if !status.is_success() {
            let body_bytes = bounded_body(response, MAX_ERROR_BODY_BYTES).await?;
            let body = String::from_utf8_lossy(&body_bytes).into_owned();
            if content_type
                .as_deref()
                .is_some_and(|ct| ct.as_bytes().starts_with(JSON_MIME_TYPE.as_bytes()))
            {
                if let Some(message) = parse_json_rpc_error(&body_bytes) {
                    return Ok(StreamableHttpPostResponse::Json(message, session_id));
                }
            }
            if let Some(response) =
                legacy_discover_response(&message, session_was_attached, status, &body)
            {
                return Ok(response);
            }
            return Err(StreamableHttpError::UnexpectedServerResponse(Cow::Owned(
                format!("HTTP {status}: {body}"),
            )));
        }
        match content_type.as_deref() {
            Some(ct) if ct.as_bytes().starts_with(EVENT_STREAM_MIME_TYPE.as_bytes()) => {
                Ok(StreamableHttpPostResponse::Sse(
                    bounded_sse_stream(response.bytes_stream(), max_sse_event_size),
                    session_id,
                ))
            }
            Some(ct) if ct.as_bytes().starts_with(JSON_MIME_TYPE.as_bytes()) => {
                // Bounded read before deserialization; malformed bodies keep
                // the SDK's lenient behavior (treated as accepted rather than
                // failing), oversized bodies are rejected outright.
                let body = bounded_body(response, MAX_BODY_BYTES).await?;
                match serde_json::from_slice::<ServerJsonRpcMessage>(&body) {
                    Ok(message) => Ok(StreamableHttpPostResponse::Json(message, session_id)),
                    Err(_) => Ok(StreamableHttpPostResponse::Accepted),
                }
            }
            _ => Err(StreamableHttpError::UnexpectedContentType(content_type)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::stream;

    fn chunks(
        parts: Vec<&'static [u8]>,
    ) -> impl Stream<Item = Result<bytes::Bytes, std::io::Error>> {
        stream::iter(
            parts
                .into_iter()
                .map(|part| Ok(bytes::Bytes::from_static(part))),
        )
    }

    #[tokio::test]
    async fn collect_bounded_accepts_within_limit() {
        let body = collect_bounded(chunks(vec![b"hello ", b"world"]), 16)
            .await
            .unwrap();
        assert_eq!(body, b"hello world");
    }

    #[tokio::test]
    async fn collect_bounded_rejects_oversized_stream() {
        let error = collect_bounded(chunks(vec![b"abc", b"defg"]), 4)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("byte limit"), "{error}");
    }

    #[tokio::test]
    async fn bounded_sse_stream_enforces_event_limit() {
        // 20-byte data line exceeds a 16-byte event budget.
        let stream = bounded_sse_stream(chunks(vec![b"data: 0123456789abcdefgh\n\n"]), 16);
        futures_util::pin_mut!(stream);
        let error = stream.next().await.unwrap().unwrap_err();
        assert!(error.to_string().contains("maximum size"), "{error}");
    }

    #[tokio::test]
    async fn bounded_sse_stream_passes_small_events() {
        let stream = bounded_sse_stream(chunks(vec![b"data: ok\n\n"]), 1024);
        futures_util::pin_mut!(stream);
        let event = stream.next().await.unwrap().unwrap();
        assert_eq!(event.data.as_deref(), Some("ok"));
    }

    #[test]
    fn json_rpc_error_parsing_only_accepts_errors() {
        assert!(parse_json_rpc_error(
            br#"{"jsonrpc":"2.0","id":1,"error":{"code":-32600,"message":"bad"}}"#
        )
        .is_some());
        assert!(parse_json_rpc_error(br#"{"jsonrpc":"2.0","id":1,"result":{}}"#).is_none());
        assert!(parse_json_rpc_error(b"not json").is_none());
    }

    #[test]
    fn scope_extraction_handles_quoted_and_bare() {
        assert_eq!(
            extract_scope_from_header(r#"Bearer realm="x", scope="files:read""#),
            Some("files:read".to_string())
        );
        assert_eq!(
            extract_scope_from_header("Bearer scope=read:data, error=x"),
            Some("read:data".to_string())
        );
        assert_eq!(extract_scope_from_header("Bearer realm=\"x\""), None);
    }

    #[test]
    fn reserved_headers_rejected_except_protocol_version() {
        assert!(validate_custom_header(&HeaderName::from_static("mcp-session-id")).is_err());
        assert!(validate_custom_header(&HeaderName::from_static("accept")).is_err());
        assert!(validate_custom_header(&HeaderName::from_static("mcp-protocol-version")).is_ok());
        assert!(validate_custom_header(&HeaderName::from_static("x-custom")).is_ok());
    }

    /// Scripted loopback HTTP server: answers one canned raw reply per
    /// request in order (Content-Length is the caller's to declare) and
    /// records (request line, lowercased headers, body).
    async fn wire_server(
        replies: Vec<String>,
    ) -> (
        String,
        Arc<std::sync::Mutex<Vec<(String, String, String)>>>,
        tokio::task::JoinHandle<()>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let recorded = Arc::new(std::sync::Mutex::new(Vec::new()));
        let records = recorded.clone();
        let task = tokio::spawn(async move {
            for reply in replies {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let mut request = Vec::new();
                let (line, heads, payload) = loop {
                    let mut buffer = [0u8; 8192];
                    let Ok(count) = socket.read(&mut buffer).await else {
                        return;
                    };
                    if count == 0 || request.len() > 1024 * 1024 {
                        return;
                    }
                    request.extend_from_slice(&buffer[..count]);
                    if let Some(end) = request.windows(4).position(|p| p == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&request[..end]).to_string();
                        let length = head
                            .lines()
                            .find_map(|line| {
                                line.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(str::trim)
                                    .map(str::to_string)
                            })
                            .and_then(|s| s.parse::<usize>().ok())
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            let mut lines = head.lines();
                            let line = lines.next().unwrap_or_default().to_string();
                            let heads = lines.map(str::to_lowercase).collect::<Vec<_>>().join("\n");
                            break (
                                line,
                                heads,
                                String::from_utf8_lossy(&request[end + 4..end + 4 + length])
                                    .into_owned(),
                            );
                        }
                    }
                };
                records.lock().unwrap().push((line, heads, payload));
                let _ = socket.write_all(reply.as_bytes()).await;
            }
        });
        (url, recorded, task)
    }

    fn ping() -> ClientJsonRpcMessage {
        ClientJsonRpcMessage::request(
            ClientRequest::PingRequest(rmcp::model::PingRequest::default()),
            rmcp::model::RequestId::Number(1),
        )
    }

    fn reply(status: u16, headers: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status} Fixture\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    #[tokio::test]
    async fn bounded_http_client_enforces_wire_limits_and_header_parity() {
        let big_json = reply(200, "Content-Type: application/json\r\n", "")
            // Declared body exceeds MAX_BODY_BYTES; the bound fires on the header
            // before any body bytes are read.
            .replacen(
                "Content-Length: 0",
                &format!("Content-Length: {}", MAX_BODY_BYTES + 1),
                1,
            );
        let big_error = reply(500, "Content-Type: application/json\r\n", "").replacen(
            "Content-Length: 0",
            &format!("Content-Length: {}", MAX_ERROR_BODY_BYTES + 1),
            1,
        );
        let sse_event = format!("data: {}\n\n", "x".repeat(200));
        let (url, records, server) = wire_server(vec![
            reply(
                200,
                "Content-Type: application/json\r\nMcp-Session-Id: fixture-session\r\n",
                r#"{"jsonrpc":"2.0","id":1,"result":{}}"#,
            ),
            big_json,
            big_error,
            reply(
                400,
                "Content-Type: application/json\r\n",
                r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32600,"message":"bad request"}}"#,
            ),
            reply(200, "Content-Type: text/event-stream\r\n", &sse_event),
            reply(200, "Content-Type: text/event-stream\r\n", &sse_event),
            reply(
                200,
                "Content-Type: text/event-stream\r\n",
                "data: hello\n\n",
            ),
            reply(405, "", ""),
        ])
        .await;
        let uri: Arc<str> = url.as_str().into();
        let client = BoundedHttpClient::new(reqwest::Client::new());
        let mut headers = HashMap::new();
        headers.insert(
            HeaderName::from_static("x-custom"),
            HeaderValue::from_static("yes"),
        );

        // JSON success: parsed, session id surfaced, request headers sent.
        let response = client
            .post_message(
                uri.clone(),
                ping(),
                Some("s-1".into()),
                Some("token-1".into()),
                headers.clone(),
            )
            .await
            .unwrap();
        match response {
            StreamableHttpPostResponse::Json(_, session) => {
                assert_eq!(session.as_deref(), Some("fixture-session"));
            }
            other => panic!("expected Json response, got {other:?}"),
        }

        // Oversized declared JSON success body: rejected before reading.
        let error = client
            .post_message(uri.clone(), ping(), None, None, HashMap::new())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("byte limit"), "{error}");

        // Oversized declared error body: rejected before reading.
        let error = client
            .post_message(uri.clone(), ping(), None, None, HashMap::new())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("byte limit"), "{error}");

        // Real JSON-RPC error body surfaces as a message, not transport failure.
        let response = client
            .post_message(uri.clone(), ping(), None, None, HashMap::new())
            .await
            .unwrap();
        assert!(
            matches!(response, StreamableHttpPostResponse::Json(..)),
            "{response:?}"
        );

        // POST SSE reply with an event over the caller's bound errors in-stream.
        let response = client
            .post_message_with_max_sse_event_size(
                uri.clone(),
                ping(),
                None,
                None,
                HashMap::new(),
                64,
            )
            .await
            .unwrap();
        match response {
            StreamableHttpPostResponse::Sse(mut stream, _) => {
                let error = stream.next().await.unwrap().unwrap_err();
                assert!(error.to_string().contains("maximum size"), "{error}");
            }
            other => panic!("expected Sse response, got {other:?}"),
        }

        // GET stream: oversized event errors; the small event passes.
        let mut stream = client
            .get_stream_with_max_sse_event_size(
                uri.clone(),
                Some("s-1".into()),
                None,
                None,
                HashMap::new(),
                64,
            )
            .await
            .unwrap();
        let error = stream.next().await.unwrap().unwrap_err();
        assert!(error.to_string().contains("maximum size"), "{error}");

        let mut stream = client
            .get_stream(uri.clone(), None, None, None, HashMap::new())
            .await
            .unwrap();
        assert_eq!(
            stream.next().await.unwrap().unwrap().data.as_deref(),
            Some("hello")
        );

        // delete_session tolerates 405 and sends the session header.
        client
            .delete_session(uri.clone(), "s-9".into(), None, HashMap::new())
            .await
            .unwrap();

        server.await.unwrap();
        let requests = records.lock().unwrap();
        assert_eq!(requests.len(), 8);
        let (line, heads, _) = &requests[0];
        assert!(line.starts_with("POST "), "{line}");
        assert!(heads.contains("mcp-session-id: s-1"), "{heads}");
        assert!(heads.contains("authorization: bearer token-1"), "{heads}");
        assert!(heads.contains("x-custom: yes"), "{heads}");
        assert!(
            heads.contains("accept: text/event-stream, application/json"),
            "{heads}"
        );
        let (line, heads, _) = &requests[7];
        assert!(line.starts_with("DELETE "), "{line}");
        assert!(heads.contains("mcp-session-id: s-9"), "{heads}");
    }
}
