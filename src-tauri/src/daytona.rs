//! Low-level Daytona transport. Callers must persist ownership before creation and arrange cleanup.
use futures_util::StreamExt;
use reqwest::{
    header::{HeaderMap, HeaderValue, AUTHORIZATION},
    Method, StatusCode, Url,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

const RESPONSE_LIMIT: usize = 1024 * 1024;
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: Url,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sandbox {
    #[serde(default)]
    pub labels: std::collections::HashMap<String, String>,
    pub id: String,
    pub name: String,
    pub state: Option<String>,
    pub toolbox_proxy_url: Option<String>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodeResult {
    pub exit_code: i32,
    pub result: String,
    pub artifacts: Option<Value>,
}

fn identifier(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("Invalid Daytona sandbox identifier.".into());
    }
    Ok(())
}
fn toolbox_url(base: &str, id: &str) -> Result<Url, String> {
    identifier(id)?;
    let mut url = Url::parse(base).map_err(|_| "Invalid Daytona toolbox URL.")?;
    if url.scheme() != "https"
        || url.host_str() != Some("proxy.app.daytona.io")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "/toolbox" | "/toolbox/")
    {
        return Err(
            "Unrecognized Daytona toolbox origin or path. No credentials were sent.".into(),
        );
    }
    url.set_path(&format!("/toolbox/{id}/process/code-run"));
    Ok(url)
}
impl Client {
    pub fn new(api_key: &str) -> Result<Self, String> {
        if api_key.trim() != api_key
            || api_key.is_empty()
            || api_key.len() > 8192
            || api_key.chars().any(char::is_control)
        {
            return Err("Enter a valid Daytona API key.".into());
        }
        let mut value = HeaderValue::from_str(&format!("Bearer {api_key}"))
            .map_err(|_| "Invalid Daytona API key.")?;
        value.set_sensitive(true);
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, value);
        let http = reqwest::Client::builder()
            .default_headers(headers)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| "Could not initialize Daytona networking.")?;
        Ok(Self {
            http,
            base: Url::parse("https://app.daytona.io/api/").expect("fixed URL"),
        })
    }
    async fn request(
        &self,
        method: Method,
        url: Url,
        body: Option<Value>,
        timeout: Duration,
    ) -> Result<Option<Value>, String> {
        let mut request = self.http.request(method, url).timeout(timeout);
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response=request.send().await.map_err(|_| "Daytona request failed. A remote action may have completed; inspect its operation record before retrying.")?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(format!("Daytona returned HTTP {}. Response details were omitted to avoid exposing credentials or remote data.",response.status().as_u16()));
        }
        if response.status() == StatusCode::NO_CONTENT {
            return Ok(Some(Value::Null));
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| {
                "Daytona response was interrupted; the remote outcome may be unknown."
            })?;
            if bytes.len() + chunk.len() > RESPONSE_LIMIT {
                return Err(
                    "Daytona response exceeds 1 MiB. The remote action may have completed.".into(),
                );
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() {
            return Ok(Some(Value::Null));
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| "Daytona returned invalid JSON; the remote outcome may be unknown.".into())
    }
    fn sandbox_url(&self, name: &str) -> Result<Url, String> {
        identifier(name)?;
        self.base
            .join(&format!("sandbox/{name}"))
            .map_err(|_| "Invalid sandbox URL.".into())
    }
    pub async fn inspect(&self, name: &str) -> Result<Option<Sandbox>, String> {
        self.request(
            Method::GET,
            self.sandbox_url(name)?,
            None,
            Duration::from_secs(30),
        )
        .await?
        .map(|value| {
            serde_json::from_value(value)
                .map_err(|_| "Daytona sandbox response is incomplete.".into())
        })
        .transpose()
    }
    pub async fn create(&self, operation_name: &str) -> Result<Sandbox, String> {
        identifier(operation_name)?;
        if !operation_name.starts_with("locallm-") {
            return Err("Daytona operations require a LocalLM ownership name.".into());
        }
        let value=self.request(Method::POST,self.base.join("sandbox").expect("fixed relative URL"),Some(json!({
            "name":operation_name,"labels":{"locallm-operation":operation_name},"public":false,
            "autoStopInterval":5,"autoDeleteInterval":0,"ttlMinutes":10,"cpu":1,"memory":1,"disk":3,"gpu":0
        })),Duration::from_secs(60)).await?.ok_or("Daytona creation endpoint was not found.")?;
        serde_json::from_value(value).map_err(|_| "Daytona creation response is incomplete. Recover by the saved operation name before retrying.".into())
    }
    pub async fn delete(&self, id: &str) -> Result<(), String> {
        self.request(
            Method::DELETE,
            self.sandbox_url(id)?,
            None,
            Duration::from_secs(30),
        )
        .await?;
        Ok(())
    }
    pub async fn run_code(
        &self,
        sandbox: &Sandbox,
        language: &str,
        code: &str,
        seconds: u32,
    ) -> Result<CodeResult, String> {
        if !matches!(language, "python" | "javascript" | "typescript")
            || code.trim().is_empty()
            || code.len() > 32768
            || !(1..=90).contains(&seconds)
        {
            return Err("Use Python, JavaScript or TypeScript, at most 32 KiB of code, and a timeout of 1–90 seconds.".into());
        }
        let url = toolbox_url(
            sandbox
                .toolbox_proxy_url
                .as_deref()
                .ok_or("Daytona did not provide a toolbox endpoint.")?,
            &sandbox.id,
        )?;
        let value = self
            .request(
                Method::POST,
                url,
                Some(json!({"code":code,"language":language,"timeout":seconds})),
                Duration::from_secs(seconds as u64 + 10),
            )
            .await?
            .ok_or("Daytona toolbox was not found.")?;
        serde_json::from_value(value).map_err(|_| {
            "Daytona execution response is incomplete; do not automatically repeat the code.".into()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn creation_sends_owned_private_time_limited_sandbox_request() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut header = Vec::new();
            loop {
                let mut byte = [0];
                socket.read_exact(&mut byte).await.unwrap();
                header.push(byte[0]);
                if header.ends_with(b"\r\n\r\n") {
                    break;
                }
                assert!(header.len() < 8192);
            }
            let header = String::from_utf8(header).unwrap().to_lowercase();
            assert!(header.starts_with("post /api/sandbox "));
            assert!(header.contains("authorization: bearer fixture-key\r\n"));
            let count: usize = header
                .lines()
                .find_map(|line| line.strip_prefix("content-length: "))
                .unwrap()
                .parse()
                .unwrap();
            assert!(count < 8192);
            let mut body = vec![0; count];
            socket.read_exact(&mut body).await.unwrap();
            let response = r#"{"id":"sandbox-1","name":"locallm-test","state":"creating"}"#;
            socket.write_all(format!("HTTP/1.1 201 Created\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",response.len()).as_bytes()).await.unwrap();
            serde_json::from_slice::<Value>(&body).unwrap()
        });
        let mut client = Client::new("fixture-key").unwrap();
        client.base = Url::parse(&format!("http://{address}/api/")).unwrap();
        assert!(client.create("unowned-name").await.is_err());
        let created = client.create("locallm-test").await.unwrap();
        assert_eq!(created.id, "sandbox-1");
        let body = server.await.unwrap();
        assert_eq!(body["name"], "locallm-test");
        assert_eq!(body["labels"]["locallm-operation"], "locallm-test");
        assert_eq!(body["public"], false);
        assert_eq!(body["ttlMinutes"], 10);
        assert_eq!(body["autoStopInterval"], 5);
        assert_eq!(body["autoDeleteInterval"], 0);
        assert_eq!(body["gpu"], 0);
        assert!(!body.to_string().contains("fixture-key"));
    }
    #[test]
    fn keys_and_toolbox_destinations_are_validated_before_network_access() {
        for key in ["", " key", "key\r\nInjected: true"] {
            assert!(Client::new(key).is_err());
        }
        for url in [
            "http://proxy.app.daytona.io/toolbox",
            "https://proxy.app.daytona.io.evil.test/toolbox",
            "https://user@proxy.app.daytona.io/toolbox",
            "https://proxy.app.daytona.io/toolbox?key=x",
            "https://proxy.app.daytona.io/other",
            "https://proxy.app.daytona.io:444/toolbox",
        ] {
            assert!(toolbox_url(url, "sandbox-1").is_err());
        }
        assert!(toolbox_url("https://proxy.app.daytona.io/toolbox", "../escape").is_err());
        assert_eq!(
            toolbox_url("https://proxy.app.daytona.io/toolbox/", "sandbox-1")
                .unwrap()
                .as_str(),
            "https://proxy.app.daytona.io/toolbox/sandbox-1/process/code-run"
        );
    }
    #[tokio::test]
    async fn authenticated_requests_are_bounded_and_do_not_follow_redirects() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (status, body, expected) in [
            (
                "200 OK",
                r#"{"id":"s1","name":"locallm-test","state":"started"}"#.to_string(),
                "ok",
            ),
            ("404 Not Found", String::new(), "missing"),
            ("302 Found", String::new(), "error"),
            ("200 OK", "x".repeat(RESPONSE_LIMIT + 1), "error"),
            ("401 Unauthorized", "private credential".into(), "error"),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut byte = [0];
                    socket.read_exact(&mut byte).await.unwrap();
                    request.push(byte[0]);
                    if request.ends_with(b"\r\n\r\n") {
                        break;
                    }
                    assert!(request.len() < 8192);
                }
                let request = String::from_utf8(request).unwrap().to_lowercase();
                assert!(request.starts_with("get /api/sandbox/locallm-test "));
                assert!(request.contains("authorization: bearer fixture-key\r\n"));
                let response=format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nLocation: https://example.invalid/\r\nConnection: close\r\n\r\n{body}",body.len());
                let _ = socket.write_all(response.as_bytes()).await;
            });
            let mut client = Client::new("fixture-key").unwrap();
            client.base = Url::parse(&format!("http://{address}/api/")).unwrap();
            let result = client.inspect("locallm-test").await;
            match expected {
                "ok" => assert_eq!(result.unwrap().unwrap().id, "s1"),
                "missing" => assert!(result.unwrap().is_none()),
                _ => {
                    let error = result.unwrap_err();
                    assert!(!error.contains("private credential"));
                    assert!(!error.contains("fixture-key"));
                }
            }
            server.await.unwrap();
        }
    }
}
