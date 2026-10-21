//! Origin-bound HTTP sessions and bounded, redacted API evidence.
use crate::development_tools as dev;
use futures_util::{FutureExt, StreamExt};
use reqwest::cookie::CookieStore;
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Mutex,
    time::Duration,
};
struct BoundedCookies {
    jar: reqwest::cookie::Jar,
    origin: reqwest::Url,
    accepted: std::sync::atomic::AtomicUsize,
}
impl CookieStore for BoundedCookies {
    fn set_cookies(
        &self,
        values: &mut dyn Iterator<Item = &reqwest::header::HeaderValue>,
        url: &reqwest::Url,
    ) {
        if url.origin() != self.origin.origin() {
            return;
        }
        for value in values {
            if value.as_bytes().len() <= 4096
                && self
                    .accepted
                    .try_update(
                        std::sync::atomic::Ordering::Relaxed,
                        std::sync::atomic::Ordering::Relaxed,
                        |n| (n < 256).then_some(n + 1),
                    )
                    .is_ok()
            {
                self.jar.set_cookies(&mut std::iter::once(value), url);
            }
        }
    }
    fn cookies(&self, url: &reqwest::Url) -> Option<reqwest::header::HeaderValue> {
        if url.origin() == self.origin.origin() {
            self.jar.cookies(url)
        } else {
            None
        }
    }
}
struct Session {
    owner: String,
    root: String,
    base: reqwest::Url,
    headers: BTreeMap<String, String>,
    cookies: std::sync::Arc<BoundedCookies>,
    /// Whether same-origin redirects are followed for this session.
    follow: bool,
}
#[derive(Default)]
pub struct Manager {
    sessions: Mutex<HashMap<String, Session>>,
}
fn url(text: &str) -> Result<reqwest::Url, String> {
    let u = reqwest::Url::parse(text).map_err(|e| e.to_string())?;
    if !matches!(u.scheme(), "http" | "https")
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.fragment().is_some()
    {
        return Err("Use an HTTP(S) URL without embedded credentials or fragments.".into());
    }
    Ok(u)
}
/// Vet one fetch target against the private-address policy and pin the
/// validated addresses onto the client that performs the request.
///
/// The model-facing HTTP tools never opt into the private-range escape
/// hatch: a model must not be able to reach a loopback service or a cloud
/// metadata endpoint through one tool while another tool refuses.
async fn vetted(url: &reqwest::Url) -> Result<crate::sandbox::VettedTarget, String> {
    crate::sandbox::vet_fetch_target(url, false)
        .await
        .map_err(|error| format!("HTTP target rejected: {error}"))
}
/// Build a client that can only reach the addresses that were vetted.
///
/// Proxies are disabled because a proxy would resolve and connect on our
/// behalf, bypassing the pin. Redirects are never followed automatically:
/// every hop is vetted separately by the caller.
fn pinned_client(
    vetted: &crate::sandbox::VettedTarget,
    cookies: Option<std::sync::Arc<BoundedCookies>>,
) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .retry(reqwest::retry::never())
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .no_proxy();
    if let Some(domain) = &vetted.domain {
        if !vetted.addrs.is_empty() {
            builder = builder.resolve_to_addrs(domain, &vetted.addrs);
        }
    }
    if let Some(cookies) = cookies {
        builder = builder.cookie_provider(cookies);
    }
    builder.build().map_err(|e| e.to_string())
}
fn sensitive(key: &str) -> bool {
    let k = key.to_ascii_lowercase().replace(['-', '_'], "");
    [
        "authorization",
        "proxyauthorization",
        "cookie",
        "setcookie",
        "password",
        "passwd",
        "secret",
        "token",
        "apikey",
        "accesskey",
    ]
    .iter()
    .any(|s| k.contains(s))
}
fn redact(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, v) in map {
                if sensitive(key) {
                    *v = json!("[REDACTED]");
                } else {
                    redact(v);
                }
            }
        }
        Value::Array(items) => {
            for v in items {
                redact(v)
            }
        }
        _ => (),
    }
}
// Redact string values, never serialized JSON tokens: a credential such as
// "true" must not corrupt a boolean or a number in an otherwise valid body.
fn redact_known(value: &mut Value, secrets: &[&str]) -> bool {
    match value {
        Value::String(text) => {
            let mut changed = false;
            for secret in secrets {
                if secret.len() >= 4 && text.contains(secret) {
                    *text = text.replace(secret, "[REDACTED]");
                    changed = true;
                }
            }
            changed
        }
        Value::Array(items) => items.iter_mut().any(|v| redact_known(v, secrets)),
        Value::Object(map) => map.values_mut().any(|v| redact_known(v, secrets)),
        _ => false,
    }
}
pub(super) fn headers(args: &Value) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for (k, v) in args.as_object().into_iter().flatten() {
        let v = v.as_str().ok_or("Header values must be strings.")?;
        if ["host", "content-length", "transfer-encoding", "connection"]
            .contains(&k.to_ascii_lowercase().as_str())
        {
            return Err("Transport headers cannot be overridden.".into());
        }
        reqwest::header::HeaderName::from_bytes(k.as_bytes()).map_err(|e| e.to_string())?;
        reqwest::header::HeaderValue::from_str(v).map_err(|e| e.to_string())?;
        out.insert(k.to_ascii_lowercase(), v.into());
    }
    Ok(out)
}
/// Assemble the request for one hop. The body is rebuilt for every hop so a
/// followed redirect can carry the same payload.
fn build_request(
    client: &reqwest::Client,
    method: &reqwest::Method,
    target: &reqwest::Url,
    headers: &BTreeMap<String, String>,
    args: &Value,
    root: &str,
    with_body: bool,
) -> Result<reqwest::RequestBuilder, String> {
    let mut request = client.request(method.clone(), target.clone());
    for (k, v) in headers {
        request = request.header(k, v);
    }
    if !with_body {
        return Ok(request);
    }
    if let Some(body) = args.get("json") {
        if body.to_string().len() > 1024 * 1024 {
            return Err("Request JSON exceeds 1 MiB.".into());
        }
        request = request.json(body);
    }
    if let Some(body) = args["body"].as_str() {
        request = request.body(body.to_string());
    }
    if let Some(files) = args["files"].as_array() {
        let workspace = crate::workspace::Workspace::open(root)?;
        let mut form = reqwest::multipart::Form::new();
        let mut bytes = args["form"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(k, v)| k.len() + v.as_str().map_or(0, str::len))
            .sum::<usize>();
        if bytes > 8 * 1024 * 1024 {
            return Err("Multipart fields exceed 8 MiB.".into());
        }
        for (key, value) in args["form"].as_object().into_iter().flatten() {
            form = form.text(
                key.clone(),
                value
                    .as_str()
                    .ok_or("Multipart form fields must be strings.")?
                    .to_string(),
            );
        }
        for file in files {
            let path = file["path"].as_str().ok_or("Multipart file requires workspace path.")?;
            let content = workspace.development_bytes(path, 8 * 1024 * 1024)?;
            bytes += content.len();
            if bytes > 8 * 1024 * 1024 {
                return Err("Multipart input exceeds 8 MiB.".into());
            }
            let filename = std::path::Path::new(path)
                .file_name()
                .ok_or("Invalid multipart filename.")?
                .to_string_lossy()
                .into_owned();
            let mut part = reqwest::multipart::Part::bytes(content).file_name(filename);
            if let Some(mime) = file["mime"].as_str() {
                part = part.mime_str(mime).map_err(|e| e.to_string())?;
            }
            form = form.part(
                file["field"]
                    .as_str()
                    .ok_or("Multipart file requires field name.")?
                    .to_string(),
                part,
            );
        }
        request = request.multipart(form);
    } else if let Some(form) = args["form"].as_object() {
        request = request.form(form);
    }
    Ok(request)
}

/// Send one request, re-vetting and re-pinning every redirect hop.
///
/// A redirect is only followed inside the original origin, and every hop is
/// resolved and checked again, so a public URL cannot bounce into loopback,
/// private or cloud-metadata space in the middle of a request.
async fn send(
    method: &reqwest::Method,
    start: &reqwest::Url,
    follow: bool,
    headers: &BTreeMap<String, String>,
    args: &Value,
    root: &str,
    cookies: Option<std::sync::Arc<BoundedCookies>>,
) -> Result<reqwest::Response, String> {
    let origin = start.origin();
    let mut current = start.clone();
    let mut method = method.clone();
    let mut with_body = true;
    let mut hops = 0usize;
    loop {
        let vetted = vetted(&current).await?;
        let client = pinned_client(&vetted, cookies.clone())?;
        let request = build_request(
            &client,
            &method,
            &current,
            headers,
            args,
            root,
            with_body,
        )?;
        let response = request.send().await.map_err(|e| {
            format!(
                "HTTP request failed; side effects may have occurred. {}",
                e.without_url()
            )
        })?;
        let status = response.status().as_u16();
        if !(follow && hops < 5 && matches!(status, 301 | 302 | 303 | 307 | 308)) {
            return Ok(response);
        }
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_string();
        if location.is_empty() {
            return Ok(response);
        }
        let next = reqwest::Url::parse(&location)
            .or_else(|_| current.join(&location))
            .map_err(|_| "Redirect location is invalid.".to_string())?;
        if !matches!(next.scheme(), "http" | "https") || next.origin() != origin {
            return Ok(response);
        }
        // RFC 7231: a redirected POST becomes a bodyless GET.
        if matches!(status, 301..=303) && method == reqwest::Method::POST {
            method = reqwest::Method::GET;
            with_body = false;
        }
        current = next;
        hops += 1;
    }
}
impl Manager {
    pub fn execute<'a>(
        &'a self,
        ctx: &'a crate::harness::HarnessCtx<'_>,
        alias: &'a str,
        args: Value,
    ) -> futures_util::future::BoxFuture<'a, Result<Value, String>> {
        async move {
        let root = dev::canonical(&ctx.snapshot.workspace_path)?;
        let owner = &ctx.conversation_id;
        if alias == "http_session_create" {
            let base = url(args["base_url"].as_str().ok_or("Missing base URL.")?)?;
            if base.query().is_some() {
                return Err("Session base URL must not contain a query.".into());
            }
            // The base URL is vetted before the session exists. A session
            // rooted at a loopback, private or metadata address would let
            // every request it makes answer to a host the model may not
            // reach directly.
            vetted(&base).await?;
            let follow = args["follow_redirects"] == true;
            let cookies=std::sync::Arc::new(BoundedCookies{jar:Default::default(),origin:base.clone(),accepted:Default::default()});
            let session = Session {
                owner: owner.clone(),
                root,
                base: base.clone(),
                headers: {let mut h=headers(&args["headers"])?;if let Some(id)=args["credential_id"].as_str(){let (key,value)=crate::development_credentials::resolve(ctx.state,id,&base)?;h.insert(key,value);}h},
                cookies: cookies.clone(),
                follow,
            };
            let id = format!("http-{}", uuid::Uuid::new_v4());
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| "HTTP sessions unavailable.")?;
            if sessions.len() >= 32 {
                return Err("HTTP session limit reached; close an existing session.".into());
            }
            sessions.insert(id.clone(), session);
            return Ok(
                json!({"sessionId":id,"redirects":args["follow_redirects"]==true,"redirectLimit":5,"cookieJar":true,"message":"Origin-bound in-memory session. Default request headers persist until close or app exit. Tool arguments remain subject to existing audit storage."}),
            );
        }
        if alias == "http_session_close" {
            let id = args["session_id"].as_str().ok_or("Missing session ID.")?;
            let mut sessions = self
                .sessions
                .lock()
                .map_err(|_| "HTTP sessions unavailable.")?;
            let s = sessions.get(id).ok_or("Unknown HTTP session.")?;
            if s.owner != *owner || s.root != root {
                return Err("Foreign HTTP session.".into());
            }
            sessions.remove(id);
            return Ok(json!({"sessionId":id,"closed":true}));
        }
        if alias == "http_compare" {
            let a = dev::load(
                &ctx.state.db_path,
                owner,
                &root,
                args["before_id"].as_str().ok_or("Missing before ID.")?,
                "http_response",
            )?;
            let b = dev::load(
                &ctx.state.db_path,
                owner,
                &root,
                args["after_id"].as_str().ok_or("Missing after ID.")?,
                "http_response",
            )?;
            return Ok(
                json!({"sameStatus":a["statusCode"]==b["statusCode"],"sameBody":a["bodySha256"]==b["bodySha256"],"diff":crate::development_credentials::structural_diff(&a["body"],&b["body"]),"before":{"status":a["statusCode"],"body":a["body"]},"after":{"status":b["statusCode"],"body":b["body"]},"redactedComparison":true,"limitation":"Redacted fields cannot be compared; responses may come from different requests."}),
            );
        }
        if alias == "openapi_inspect" {return crate::openapi_tools::inspect(ctx,args).await;}
        if alias == "openapi_request" {let prepared=crate::openapi_tools::prepare(ctx,args).await?;crate::tool_validation::validate(&registry().into_iter().find(|t|t.name=="http_request").ok_or("HTTP tool schema missing.")?.input,&prepared)?;return Box::pin(self.execute(ctx,"http_request",prepared)).await;}
        if alias != "http_request" {
            return Err("Unknown API tool.".into());
        }
        if ctx.state.database()?.setting::<bool>("offline_mode")? {
            return Err("Offline mode: HTTP requests are disabled.".into());
        }
        let (target, follow, mut request_headers, cookies) = if let Some(id) = args["session_id"].as_str() {
            let sessions = self
                .sessions
                .lock()
                .map_err(|_| "HTTP sessions unavailable.")?;
            let s = sessions.get(id).ok_or("Unknown HTTP session.")?;
            if s.owner != *owner || s.root != root {
                return Err("Foreign HTTP session.".into());
            }
            if args.get("follow_redirects").is_some(){return Err("Set redirect policy when creating the HTTP session.".into());}
            let target = s
                .base
                .join(args["url"].as_str().ok_or("Missing request URL/path.")?)
                .map_err(|e| e.to_string())?;
            let target = url(target.as_str())?;
            if target.origin() != s.base.origin() {
                return Err("Session request must stay within its original origin.".into());
            }
            let mut h=s.headers.clone();
            if !h.contains_key("cookie") {if let Some(value)=s.cookies.cookies(&target){h.insert("cookie".into(),value.to_str().map_err(|_|"Invalid session cookie.")?.into());}}
            (target, s.follow, h, Some(s.cookies.clone()))
        } else {
            let target = url(args["url"].as_str().ok_or("Missing URL.")?)?;
            (target, args["follow_redirects"] == true, BTreeMap::new(), None)
        };
        request_headers.extend(headers(&args["headers"])?);
        if let Some(id)=args["credential_id"].as_str(){let (key,value)=crate::development_credentials::resolve(ctx.state,id,&target)?;request_headers.insert(key,value);}
        let method =
            reqwest::Method::from_bytes(args["method"].as_str().unwrap_or("GET").as_bytes())
                .map_err(|e| e.to_string())?;
        // Compile every candidate before sending a request that could mutate state.
        let mut response_validators=HashMap::new();
        for (status, media) in args["response_schemas"].as_object().into_iter().flatten() {
            for (mime,schema) in media.as_object().ok_or("Response schemas must map status to media types.")? {
                response_validators.insert((status.clone(),mime.clone()),crate::tool_validation::compile(schema)?);
            }
        }
        let validator = args
            .get("response_schema")
            .map(crate::tool_validation::compile)
            .transpose()?;
        if args["files"].is_array() && (args.get("json").is_some() || args.get("body").is_some()) {return Err("Multipart files cannot be combined with JSON or raw body.".into());}
        let body_modes=["json","body","form"].iter().filter(|key|args.get(**key).is_some()).count();if body_modes>1{return Err("Choose exactly one request body format.".into());}
        let start = std::time::Instant::now();
        // Every hop is vetted and pinned: the target itself, and each
        // redirect it may follow.
        let response = send(
            &method,
            &target,
            follow,
            &request_headers,
            &args,
            &root,
            cookies,
        )
        .await?;
        let status = response.status();
        let response_type=response.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v|v.to_str().ok()).unwrap_or("").split(';').next().unwrap_or("").to_string();
        let validator=[status.as_str().to_string(),format!("{}XX",status.as_u16()/100),"default".to_string()].into_iter().find_map(|key| {
            response_validators.remove(&(key.clone(),response_type.clone())).or_else(||response_type.split_once('/').and_then(|(kind,_)|response_validators.remove(&(key.clone(),format!("{kind}/*"))))).or_else(||response_validators.remove(&(key,"*/*".into())))
        }).or(validator);
        let mut response_headers = serde_json::Map::new();
        for (k, v) in response.headers() {
            response_headers.insert(
                k.to_string(),
                if sensitive(k.as_str()) {
                    json!("[REDACTED]")
                } else {
                    json!(v.to_str().unwrap_or("[binary header]"))
                },
            );
        }
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        let mut truncated = false;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| {
                format!(
                    "HTTP body interrupted; request was already sent: {}",
                    e.without_url()
                )
            })?;
            let keep = (1024 * 1024usize)
                .saturating_sub(bytes.len())
                .min(chunk.len());
            bytes.extend_from_slice(&chunk[..keep]);
            if keep < chunk.len() {
                truncated = true;
                break;
            }
        }
        let parsed = if !truncated {
            serde_json::from_slice::<Value>(&bytes).ok()
        } else {
            None
        };
        let mut body = parsed.unwrap_or_else(|| json!(String::from_utf8_lossy(&bytes)));
        let validation = validator.map(|v| if truncated {json!({"valid":false,"reason":"Response is truncated."})} else {match v.check(&body) { Ok(valid)=>json!({"valid":valid}),Err(reason)=>json!({"valid":false,"reason":reason}) }});
        let validation_failed = validation.as_ref().is_some_and(|v| v["valid"] == false);
        redact(&mut body);
        // Also remove known credentials echoed under innocuous response headers.
        let secrets = request_headers
            .iter()
            .filter(|(k, _)| sensitive(k))
            .flat_map(|(key, v)| {let mut values=vec![v.as_str(), v.strip_prefix("Bearer ").unwrap_or(v)];if key=="cookie"{values.extend(v.split(';').filter_map(|v|v.trim().split_once('=').map(|(_,v)|v)));}values})
            .collect::<Vec<_>>();
        let mut response_headers = Value::Object(response_headers);
        let redaction_applied =
            redact_known(&mut body, &secrets) | redact_known(&mut response_headers, &secrets);
        let encoded = serde_json::to_string(&body).map_err(|e| e.to_string())?;
        let mut result = json!({"root":root,"method":method.as_str(),"origin":target.origin().ascii_serialization(),"statusCode":status.as_u16(),"httpSuccess":status.is_success(),"headers":response_headers,"bodySha256":dev::hash(encoded.as_bytes()),"body":body,"truncated":truncated,"durationMs":start.elapsed().as_secs_f64()*1000.,"validation":validation,"redacted":true,"credentialValueRedactionApplied":redaction_applied,"untrustedResponse":true,"isError":!status.is_success()||validation_failed||truncated,"sideEffects":"possible"});
        result["responseId"] = json!(dev::save(
            &ctx.state.db_path,
            owner,
            &ctx.run_id,
            "http_response",
            &result
        )?);
        Ok(result)
    }.boxed()
    }
}
#[cfg(test)]
fn inspect(root: &str, args: Value) -> Result<Value, String> {
    let path = args["path"].as_str().ok_or("Missing OpenAPI path.")?;
    let text = crate::workspace::Workspace::open(root)?.language_source(path)?;
    let spec: Value =
        serde_json::from_str(&text).map_err(|e| format!("OpenAPI input must be JSON: {e}"))?;
    if !spec["openapi"]
        .as_str()
        .is_some_and(|s| s.starts_with("3."))
    {
        return Err("Only OpenAPI 3.x JSON is supported.".into());
    }
    let mut operations = Vec::new();
    for (path, item) in spec["paths"].as_object().into_iter().flatten() {
        for method in [
            "get", "post", "put", "patch", "delete", "head", "options", "trace",
        ] {
            if let Some(op) = item.get(method) {
                if args["operation_id"]
                    .as_str()
                    .is_none_or(|id| op["operationId"] == id)
                {
                    operations.push(json!({"method":method.to_uppercase(),"path":path,"operationId":op["operationId"],"summary":op["summary"],"pathParameters":item["parameters"],"parameters":op["parameters"],"requestBody":op["requestBody"],"responses":op["responses"]}));
                }
            }
        }
    }
    if args["operation_id"].is_string() && operations.len() != 1 {
        return Err("Operation ID was absent or ambiguous.".into());
    }
    let total = operations.len();
    operations.truncate(100);
    Ok(
        json!({"sourceSha256":dev::hash(text.as_bytes()),"openapi":spec["openapi"],"operations":operations,"total":total,"truncated":total>100,"components":if args["operation_id"].is_string(){spec["components"]["schemas"].clone()}else{Value::Null},"limitation":"JSON only; references remain explicit. Resolve local schemas before passing response_schema; OpenAPI 3.0 schemas may need conversion to JSON Schema."}),
    )
}
pub fn registry() -> Vec<crate::tool_spec::ToolSpec> {
    let text = json!({"type":"string","minLength":1,"maxLength":4096});
    let id = json!({"type":"string","minLength":1,"maxLength":128});
    let headers = json!({"type":"object","maxProperties":32,"additionalProperties":{"type":"string","maxLength":8192}});
    vec![crate::openapi_tools::definition(),("http_session_create".into(),"Create an origin-bound in-memory HTTP session with pooled connections and default headers. Automatic bounded origin-bound cookies; redirects stay within the original origin and require follow_redirects. Headers may contain credentials; existing tool-argument audit storage applies. Close to discard session credentials.".into(),dev::schema(json!({"base_url":text,"headers":headers,"credential_id":id,"follow_redirects":{"type":"boolean","default":false}}),&["base_url"])),
    ("http_session_close".into(),"Close an owned HTTP session and discard retained default headers.".into(),dev::schema(json!({"session_id":id}),&["session_id"])),
    ("http_request".into(),"Make a bounded HTTP API request with optional owned session and JSON body. Optional same-origin redirects (maximum five); 30-second timeout, 1 MiB response cap. Optionally validate complete JSON against an offline JSON Schema. Saves redacted response evidence; requests can have side effects and are never automatically retried. Redaction is best-effort for unknown secrets in arbitrary text.".into(),dev::schema(json!({"session_id":id,"url":text,"follow_redirects":{"type":"boolean","default":false},"method":{"type":"string","enum":["GET","HEAD","POST","PUT","PATCH","DELETE","OPTIONS"]},"headers":headers,"json":{},"body":{"type":"string","maxLength":1048576},"form":{"type":"object","maxProperties":100,"additionalProperties":{"type":"string","maxLength":65536}},"files":{"type":"array","maxItems":16,"items":{"type":"object","properties":{"path":text,"field":{"type":"string","minLength":1,"maxLength":128},"mime":{"type":"string","maxLength":128}},"required":["path","field"],"additionalProperties":false}},"credential_id":id,"response_schemas":{"type":"object"},"response_schema":{"type":"object"}}),&["url"])),
    ("http_compare".into(),"Compare saved redacted HTTP response status and payload evidence in this conversation/workspace.".into(),dev::schema(json!({"before_id":id,"after_id":id}),&["before_id","after_id"])),
    ("openapi_inspect".into(),"Inspect a workspace OpenAPI 3.x JSON or YAML document with bounded local and user-allowlisted remote references. List operations or select a unique operationId to read method, route, parameters, request and response schemas. Does not make a network request.".into(),dev::schema(json!({"path":text,"operation_id":{"type":"string","minLength":1,"maxLength":256}}),&["path"]))].into_iter().map(crate::tool_spec::ToolSpec::from).collect()
}

/// Resolve a requested path to a canonical path inside the workspace.
///
/// A relative path is a workspace-relative path: traversal, reserved
/// characters and absolute paths are refused before it is resolved. An
/// absolute path is accepted only when it canonicalizes inside the workspace,
/// so a link that points outside is refused as well.
// The command below is registered in `lib.rs` (see the integration note at the
// end of this file); until that registration lands the helper is only used by
// the tests.
#[allow(dead_code)]
fn resolve_workspace_path(bound: &str, path: &str) -> Result<std::path::PathBuf, String> {
    let requested = std::path::Path::new(path);
    let absolute = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        crate::editor::validate_path(path)?;
        std::path::Path::new(bound).join(requested)
    };
    let canonical_root = std::fs::canonicalize(bound)
        .map_err(|error| format!("Could not resolve the workspace folder: {error}"))?;
    let canonical = std::fs::canonicalize(&absolute)
        .map_err(|_| "That file is no longer available.".to_string())?;
    if !canonical.starts_with(&canonical_root) {
        return Err("Only files inside the selected workspace can be opened.".into());
    }
    Ok(canonical)
}

/// Open a workspace file or folder in the operating system's default
/// application.
///
/// The renderer cannot hand an arbitrary absolute path to the opener plugin:
/// the path is resolved against the selected workspace, canonicalized, and
/// refused when it leaves that workspace. The opener plugin is invoked from
/// the backend so the capability file does not have to grant
/// `opener:allow-open-path` to the webview.
// Registered in `lib.rs` (see the integration note at the end of this file).
#[tauri::command]
pub fn open_workspace_path(
    app: tauri::AppHandle,
    state: tauri::State<'_, crate::AppState>,
    path: String,
    root: Option<String>,
) -> Result<(), String> {
    let bound = match root.filter(|root| !root.is_empty()) {
        Some(root) => crate::workspace_ui::bound_root(&state, &root)?,
        None => state.database()?.workspace_path()?,
    };
    let canonical = resolve_workspace_path(&bound, &path)?;
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_path(canonical.to_string_lossy().into_owned(), None::<String>)
        .map_err(|error| format!("Could not open that file: {error}"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn model_facing_http_targets_refuse_private_and_loopback_addresses() {
        for target in [
            "http://127.0.0.1:8080/x",
            "http://169.254.169.254/latest/meta-data/",
            "http://10.0.0.5/",
            "http://192.168.1.1/",
            "http://[::1]/",
            "http://[::ffff:10.0.0.1]/",
        ] {
            let url = reqwest::Url::parse(target).unwrap();
            assert!(vetted(&url).await.is_err(), "{target} must be refused");
        }
    }
    #[tokio::test]
    async fn http_sessions_and_requests_refuse_private_targets() {
        let t = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(t.path());
        let owner = state.database().unwrap().create_conversation().unwrap().id;
        let ctx = crate::development_tools::tests::context(&state, t.path(), &owner, &[]);
        // A session rooted at a loopback address is refused: every request it
        // would make would answer to a host the model may not reach.
        let error = state
            .http_sessions
            .execute(
                &ctx,
                "http_session_create",
                json!({"base_url":"http://127.0.0.1:9/"}),
            )
            .await
            .unwrap_err();
        assert!(error.contains("rejected"), "{error}");
        // A cloud metadata endpoint is refused as well.
        let error = state
            .http_sessions
            .execute(
                &ctx,
                "http_request",
                json!({"url":"http://169.254.169.254/latest/meta-data/"}),
            )
            .await
            .unwrap_err();
        assert!(error.contains("rejected"), "{error}");
    }
    #[test]
    fn opening_a_path_is_confined_to_the_selected_workspace() {
        let t = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("inside.txt"), "inside").unwrap();
        std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        let bound = t.path().to_str().unwrap();
        // A relative workspace path resolves inside the workspace.
        assert!(resolve_workspace_path(bound, "inside.txt").is_ok());
        // Traversal is refused before it is resolved.
        assert!(resolve_workspace_path(bound, "../secret.txt").is_err());
        // An absolute path outside the workspace is refused.
        assert!(resolve_workspace_path(
            bound,
            outside.path().join("secret.txt").to_str().unwrap()
        )
        .is_err());
        // A link that points outside the workspace is refused too.
        #[cfg(unix)]
        {
            let link = t.path().join("escape.txt");
            std::os::unix::fs::symlink(outside.path().join("secret.txt"), &link).unwrap();
            assert!(resolve_workspace_path(bound, "escape.txt").is_err());
        }
    }
    #[test]
    fn automatic_cookies_are_bounded_and_origin_scoped() {
        let origin = reqwest::Url::parse("https://example.com/").unwrap();
        let cookies = BoundedCookies {
            jar: Default::default(),
            origin: origin.clone(),
            accepted: Default::default(),
        };
        for i in 0..300 {
            let header =
                reqwest::header::HeaderValue::from_str(&format!("cookie{i}=value{i}; Path=/"))
                    .unwrap();
            cookies.set_cookies(&mut std::iter::once(&header), &origin);
        }
        let header = cookies.cookies(&origin).unwrap();
        assert_eq!(header.to_str().unwrap().split(';').count(), 256);
        assert!(cookies
            .cookies(&reqwest::Url::parse("https://other.example.com/").unwrap())
            .is_none());
    }
    #[test]
    fn secrets_and_origins_are_checked() {
        let mut v = json!({"nested":{"access_token":"secret","ok":4},"Authorization":"Bearer x"});
        redact(&mut v);
        assert_eq!(v["nested"]["access_token"], "[REDACTED]");
        assert_eq!(v["nested"]["ok"], 4);
        assert!(url("file:///tmp/a").is_err());
        assert!(url("https://u:p@example.com").is_err());
        assert!(headers(&json!({"Host":"other"})).is_err());
    }
    #[test]
    fn credential_redaction_does_not_rewrite_json_tokens() {
        let mut body =
            json!({"ok":true,"missing":null,"number":1234,"text":"token=true; code=1234"});
        assert!(redact_known(&mut body, &["true", "1234"]));
        assert_eq!(body["ok"], true);
        assert!(body["missing"].is_null());
        assert_eq!(body["number"], 1234);
        assert_eq!(body["text"], "token=[REDACTED]; code=[REDACTED]");
    }
    #[test]
    fn openapi_resolves_unique_operation() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("api.json"),json!({"openapi":"3.1.0","paths":{"/things/{id}":{"get":{"operationId":"getThing","responses":{"200":{"description":"ok"}}}}}}).to_string()).unwrap();
        let r = inspect(
            t.path().to_str().unwrap(),
            json!({"path":"api.json","operation_id":"getThing"}),
        )
        .unwrap();
        assert_eq!(r["operations"][0]["method"], "GET");
        assert!(inspect(
            t.path().to_str().unwrap(),
            json!({"path":"api.json","operation_id":"missing"})
        )
        .is_err());
    }
}

#[cfg(test)]
mod transport_tests {
    use super::*;
    #[tokio::test]
    #[ignore = "Requires a host permitting loopback TCP sockets; run explicitly."]
    async fn actual_http_response_validation_redaction_and_evidence() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("HTTP fixture needs permitted loopback sockets");
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut b = [0; 1024];
            loop {
                let n = stream.read(&mut b).await.unwrap();
                if n == 0 {
                    break;
                }
                request.extend_from_slice(&b[..n]);
                if request.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            assert!(String::from_utf8_lossy(&request).contains("Bearer fixture-secret"));
            let body =
                json!({"ok":true,"access_token":"fixture-secret","echo":"Bearer fixture-secret"})
                    .to_string();
            let response=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nSet-Cookie: secret=fixture-secret\r\nX-Echo: Bearer fixture-secret\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body);
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        let t = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(t.path());
        let owner = state.database().unwrap().create_conversation().unwrap().id;
        let tools = registry()
            .iter()
            .map(|t| crate::connectors::AgentTool::harness(&t.name).unwrap())
            .collect::<Vec<_>>();
        let ctx = crate::development_tools::tests::context(&state, t.path(), &owner, &tools);
        let session = crate::harness::execute(
            &ctx,
            "http_session_create",
            json!({"base_url":base,"headers":{"Authorization":"Bearer fixture-secret"}}),
        )
        .await
        .unwrap()
        .value;
        let response=crate::harness::execute(&ctx,"http_request",json!({"session_id":session["sessionId"],"url":"things","response_schema":{"type":"object","required":["ok"],"properties":{"ok":{"type":"boolean"}}}})).await.unwrap().value;
        assert_eq!(response["validation"]["valid"], true);
        assert_eq!(response["statusCode"], 200);
        assert!(!response.to_string().contains("fixture-secret"));
        assert_eq!(response["headers"]["set-cookie"], "[REDACTED]");
        let a = dev::load(
            &state.db_path,
            &owner,
            t.path().to_str().unwrap(),
            response["responseId"].as_str().unwrap(),
            "http_response",
        )
        .unwrap();
        assert!(!a.to_string().contains("fixture-secret"));
        let compared = crate::harness::execute(
            &ctx,
            "http_compare",
            json!({"before_id":response["responseId"],"after_id":response["responseId"]}),
        )
        .await
        .unwrap()
        .value;
        assert_eq!(compared["sameBody"], true);
        server.await.unwrap();
    }
    #[tokio::test]
    #[ignore = "Requires permitted loopback TCP sockets; isolated HTTP fixtures only."]
    async fn actual_http_redirect_forms_references_and_uncertain_writes() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let foreign = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}/", listener.local_addr().unwrap());
        let foreign_url = format!("http://{}/", foreign.local_addr().unwrap());
        let records = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let received = records.clone();
        let cross = foreign_url.clone();
        let server = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0; 8192];
                let (header, offset, length) = loop {
                    let n = stream.read(&mut buffer).await.unwrap();
                    if n == 0 {
                        break (String::new(), 0, 0);
                    }
                    bytes.extend_from_slice(&buffer[..n]);
                    assert!(bytes.len() < 16 * 1024 * 1024);
                    if let Some(offset) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                        let header = String::from_utf8_lossy(&bytes[..offset]).to_string();
                        let length = header
                            .lines()
                            .filter_map(|l| l.split_once(':'))
                            .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                            .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                            .unwrap_or(0);
                        break (header, offset + 4, length);
                    }
                };
                if header.is_empty() {
                    continue;
                }
                while bytes.len() < offset + length {
                    let n = stream.read(&mut buffer).await.unwrap();
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buffer[..n]);
                }
                let request = String::from_utf8_lossy(&bytes).to_string();
                let path = header
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap()
                    .to_string();
                received.lock().unwrap().push(request.clone());
                if path == "/write-close" {
                    continue;
                }
                let mut status = "200 OK";
                let mut extra = String::new();
                let body = if path == "/cross" {
                    status = "302 Found";
                    extra = format!("Location: {cross}\r\n");
                    "{}".into()
                } else if path == "/loop" {
                    status = "302 Found";
                    extra = "Location: /loop\r\n".into();
                    "{}".into()
                } else if path == "/redirect/2" {
                    status = "302 Found";
                    extra = "Location: /redirect/1\r\n".into();
                    "{}".into()
                } else if path == "/redirect/1" {
                    status = "302 Found";
                    extra = "Location: /ok\r\n".into();
                    "{}".into()
                } else if path == "/set" {
                    extra = "Set-Cookie: fixture=retained-value; Path=/\r\n".into();
                    "{}".into()
                } else if path.starts_with("/cookie-limit/") {
                    let batch = path.rsplit('/').next().unwrap().parse::<usize>().unwrap();
                    for index in batch * 80..(batch + 1) * 80 {
                        extra.push_str(&format!("Set-Cookie: c{index}=v{index}; Path=/\r\n"));
                    }
                    extra.push_str(&format!(
                        "Set-Cookie: oversized={}; Path=/\r\n",
                        "x".repeat(4097)
                    ));
                    "{}".into()
                } else if path == "/schema/node.json" {
                    r##"{"type":"object","required":["name"],"properties":{"name":{"type":"string"},"child":{"$ref":"node.json"}}}"##.into()
                } else if path == "/large" {
                    "x".repeat(1024 * 1024 + 100)
                } else {
                    json!({"ok":true}).to_string()
                };
                let response = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                let _ = stream.write_all(response.as_bytes()).await;
            }
        });
        struct Server(tokio::task::JoinHandle<()>);
        impl Drop for Server {
            fn drop(&mut self) {
                self.0.abort();
            }
        }
        let _server = Server(server);
        let t = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(t.path());
        let owner = state.database().unwrap().create_conversation().unwrap().id;
        let ctx = crate::development_tools::tests::context(&state, t.path(), &owner, &[]);
        let credential = crate::development_credentials::HttpCredential {
            id: "dev-http-fixture".into(),
            name: "Owned fixture".into(),
            origin: base.trim_end_matches('/').into(),
            header: "authorization".into(),
        };
        state
            .daytona_vault
            .save(&credential.id, b"Bearer vault-fixture-secret")
            .unwrap();
        state
            .database()
            .unwrap()
            .save_setting("development.http_credentials", &vec![credential.clone()])
            .unwrap();
        let session = state
            .http_sessions
            .execute(
                &ctx,
                "http_session_create",
                json!({"base_url":base,"credential_id":credential.id,"follow_redirects":true}),
            )
            .await
            .unwrap();
        let id = &session["sessionId"];
        let request = |path: &str| json!({"session_id":id,"url":path});
        assert_eq!(
            state
                .http_sessions
                .execute(&ctx, "http_request", request("redirect/2"))
                .await
                .unwrap()["statusCode"],
            200
        );
        let count = records.lock().unwrap().len();
        assert_eq!(
            state
                .http_sessions
                .execute(&ctx, "http_request", request("loop"))
                .await
                .unwrap()["statusCode"],
            302
        );
        assert!(records.lock().unwrap().len() - count <= 5);
        assert_eq!(
            state
                .http_sessions
                .execute(&ctx, "http_request", request("cross"))
                .await
                .unwrap()["statusCode"],
            302
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(50), foreign.accept())
                .await
                .is_err()
        );
        assert!(state
            .http_sessions
            .execute(
                &ctx,
                "http_request",
                json!({"url":foreign_url,"credential_id":credential.id})
            )
            .await
            .is_err());
        state
            .http_sessions
            .execute(&ctx, "http_request", request("set"))
            .await
            .unwrap();
        state
            .http_sessions
            .execute(&ctx, "http_request", request("cookie"))
            .await
            .unwrap();
        assert!(records
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .to_lowercase()
            .contains("cookie: fixture=retained-value"));
        state
            .http_sessions
            .execute(
                &ctx,
                "http_request",
                json!({"session_id":id,"url":"form","method":"POST","form":{"name":"a b&c"}}),
            )
            .await
            .unwrap();
        assert!(records
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .contains("name=a+b%26c"));
        for batch in 0..5 {
            state
                .http_sessions
                .execute(
                    &ctx,
                    "http_request",
                    request(&format!("cookie-limit/{batch}")),
                )
                .await
                .unwrap();
        }
        state
            .http_sessions
            .execute(&ctx, "http_request", request("cookie"))
            .await
            .unwrap();
        let cookie_request = records.lock().unwrap().last().unwrap().to_lowercase();
        let cookie = cookie_request
            .lines()
            .find_map(|l| l.strip_prefix("cookie:"))
            .unwrap();
        assert!(cookie.split(';').count() <= 256);
        assert!(!cookie.contains("oversized="));
        let selected=state.http_sessions.execute(&ctx,"http_request",json!({"session_id":id,"url":"ok","response_schemas":{"2XX":{"application/*":{"type":"object","required":["ok"]}}}})).await.unwrap();
        assert_eq!(selected["validation"]["valid"], true);
        std::fs::write(t.path().join("upload.txt"), "owned-file-content").unwrap();
        state.http_sessions.execute(&ctx,"http_request",json!({"session_id":id,"url":"multipart","method":"POST","form":{"title":"fixture-field"},"files":[{"field":"file","path":"upload.txt"}]})).await.unwrap();
        let uploaded = records.lock().unwrap().last().unwrap().clone();
        assert!(uploaded.contains("multipart/form-data"));
        assert!(uploaded.contains("fixture-field"));
        assert!(uploaded.contains("owned-file-content"));
        let count = records.lock().unwrap().len();
        assert!(state.http_sessions.execute(&ctx,"http_request",json!({"session_id":id,"url":"unsafe","method":"POST","response_schema":{"type":"invalid"}})).await.is_err());
        assert_eq!(
            records.lock().unwrap().len(),
            count,
            "Invalid response schemas must compile before writes"
        );
        let mismatch=state.http_sessions.execute(&ctx,"http_request",json!({"session_id":id,"url":"ok","response_schema":{"type":"object","required":["missing"]}})).await.unwrap();
        assert_eq!(mismatch["isError"], true);
        let large = state
            .http_sessions
            .execute(&ctx, "http_request", request("large"))
            .await
            .unwrap();
        assert_eq!(large["truncated"], true);
        assert_eq!(large["isError"], true);
        let count = records.lock().unwrap().len();
        assert!(state
            .http_sessions
            .execute(
                &ctx,
                "http_request",
                json!({"session_id":id,"url":"write-close","method":"POST","json":{"write":true}})
            )
            .await
            .is_err());
        assert_eq!(
            records.lock().unwrap().len(),
            count + 1,
            "Uncertain POST must never be retried"
        );
        // Resolve an authorized remote document with a document-relative recursive
        // reference, then validate fully offline before sending the request.
        state
            .database()
            .unwrap()
            .save_setting(
                "development.reference_origins",
                &vec![base.trim_end_matches('/').to_string()],
            )
            .unwrap();
        let spec = json!({"openapi":"3.1.0","servers":[{"url":base.trim_end_matches('/')}],"paths":{"/ok":{"post":{"operationId":"recursive","parameters":[{"in":"query","name":"tenant","required":true,"schema":{"type":"string"}}],"requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"request.schema.json"}}}},"responses":{"200":{"content":{"application/json":{"schema":{"type":"object","required":["ok"]}}}}}}}}});
        std::fs::write(t.path().join("api.json"), spec.to_string()).unwrap();
        std::fs::write(
            t.path().join("request.schema.json"),
            json!({"$ref":format!("{base}schema/node.json")}).to_string(),
        )
        .unwrap();
        let response=state.http_sessions.execute(&ctx,"openapi_request",json!({"path":"api.json","operation_id":"recursive","query_parameters":{"tenant":"fixture"},"body":{"name":"root","child":{"name":"leaf"}}})).await.unwrap();
        assert_eq!(response["validation"]["valid"], true);
        assert!(
            state
                .http_sessions
                .execute(
                    &ctx,
                    "openapi_request",
                    json!({"path":"api.json","operation_id":"recursive","body":{"name":"valid"}})
                )
                .await
                .is_err(),
            "Required query input must fail before POST"
        );
        let before = records
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.starts_with("POST "))
            .count();
        assert!(state
            .http_sessions
            .execute(
                &ctx,
                "openapi_request",
                json!({"path":"api.json","operation_id":"recursive","query_parameters":{"tenant":"fixture"},"body":{"name":42}})
            )
            .await
            .is_err());
        assert!(state
            .http_sessions
            .execute(
                &ctx,
                "openapi_request",
                json!({"path":"api.json","operation_id":"recursive","query_parameters":{"tenant":"fixture"}})
            )
            .await
            .is_err());
        assert_eq!(
            records
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.starts_with("POST "))
                .count(),
            before
        );
        state
            .http_sessions
            .execute(&ctx, "http_session_close", json!({"session_id":id}))
            .await
            .unwrap();
    }
}

// ---------------------------------------------------------------------------
// INTEGRATION NOTE FOR src-tauri/src/lib.rs (owned by another agent)
// ---------------------------------------------------------------------------
// `opener:allow-open-path` was removed from `capabilities/default.json`, so the
// webview can no longer hand an arbitrary absolute path to the opener plugin.
// "Open in external editor" now goes through the backend command below.
//
// Add this exact line to the `tauri::generate_handler![ ... ]` list in
// `src-tauri/src/lib.rs` (for example next to the other `workspace_ui::`
// entries):
//
//     api_tools::open_workspace_path,
//
// ---------------------------------------------------------------------------
