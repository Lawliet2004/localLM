//! Isolated native state for tool/catalog integration tests. No user settings,
//! credentials, model process, OS window or external service is accessed.
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

pub(crate) fn app_state(root: &Path) -> crate::AppState {
    let data = root.join("fixture-state");
    std::fs::create_dir_all(&data).unwrap();
    let db_path = data.join("locallm.sqlite");
    let vault = Arc::new(crate::vault::Vault::new(data.join("credentials")));
    crate::AppState {
        automation: Arc::new(crate::automation::Manager::default()),
        installation_operation: tokio::sync::Mutex::new(()),
        runtime_installer: crate::model_install::Installer::default(),
        model_installer: crate::model_install::Installer::default(),
        daytona_vault: vault.clone(),
        daytona_operation: Arc::new(tokio::sync::Mutex::new(())),
        daytona_journal: Arc::new(Mutex::new(
            crate::daytona_journal::Journal::open(&data.join("daytona.sqlite")).unwrap(),
        )),
        skills: tokio::sync::Mutex::new(crate::skills::Skills::new(data.join("skills"))),
        approvals: crate::approval::Approvals::default(),
        ask_user: crate::approval::ChoiceApprovals::default(),
        mcp_inputs: Arc::new(crate::mcp_input::Registry::default()),
        store: Mutex::new(crate::store::Store::open(&db_path).unwrap()),
        runtime: tokio::sync::Mutex::new(crate::runtime::Runtime::new(data.join("runtime.log"))),
        load_cancel: tokio::sync::watch::channel(false).0,
        load_status: Arc::new(Mutex::new(None)),
        operation: crate::OperationGate::default(),
        turns: crate::TurnLocks::default(),
        cancel: tokio::sync::watch::channel(false).0,
        run_control: Mutex::new(crate::agent_run::RunControl::default()),
        connectors: tokio::sync::Mutex::new(crate::connectors::McpHub::new(vault)),
        oauth_operation: tokio::sync::Mutex::new(()),
        oauth_cancel: tokio::sync::watch::channel(false).0,
        subagents: crate::subagents::global_registry(),
        processes: crate::process_tools::ProcessRegistry::persistent(db_path.clone()),
        previews: crate::workspace_preview::Manager::default(),
        language_servers: crate::lsp_client::Manager::default(),
        verification_observers: crate::verification_observer::Registry::default(),
        pty_sessions: Mutex::new(std::collections::HashMap::new()),
        web_search_cache: Mutex::new(crate::harness::WebSearchCache::default()),
        turn_budgets: Mutex::new(std::collections::HashMap::new()),
        turn_blocked: Mutex::new(std::collections::HashMap::new()),
        db_path,
        data_dir: data,
    }
}

pub(crate) fn snapshot(
    state: &crate::AppState,
    workspace: &Path,
    aliases: &[&str],
) -> crate::subagents::BackendSnapshot {
    crate::subagents::BackendSnapshot {
        processes: state.processes.clone(),
        model_capabilities: None,
        kind: "local".into(),
        endpoint: String::new(),
        key: String::new(),
        base_url: String::new(),
        api_format: String::new(),
        model_id: "fixture".into(),
        context_length: 8192,
        max_output_tokens: Some(2048),
        supports_tools: true,
        allowed_tool_aliases: aliases.iter().map(|name| name.to_string()).collect(),
        system_prompt: String::new(),
        temperature: 0.0,
        top_p: 1.0,
        max_tokens: 2048,
        workspace_path: workspace.to_string_lossy().into_owned(),
        execution_config: state.database().unwrap().execution_config().unwrap(),
        active_skills: Vec::new(),
        db_path: state.db_path.clone(),
        data_dir: state.data_dir.clone(),
        account_id: None,
        inference_slots: 1,
    }
}

/// Catalog assembly needs a selected tool-capable model, but does not need
/// to contact it. This verified keyless loopback fixture never starts a server.
pub(crate) fn select_fixture_model(state: &crate::AppState, workspace: &Path) {
    let store = state.database().unwrap();
    store
        .save_workspace_path(workspace.to_str().unwrap())
        .unwrap();
    store
        .save_provider(&crate::providers::ProviderConnection {
            id: "fixture-loopback".into(),
            name: "Fixture".into(),
            api_format: crate::providers::FREETOKEN_OPENAI_COMPAT.into(),
            base_url: "http://127.0.0.1:1919/v1".into(),
            verified: true,
            last_tested_at: None,
            has_api_key: false,
            models: vec![crate::providers::RemoteModel {
                controls: None,
                id: "fixture-model".into(),
                context_length: Some(32768),
                max_output_tokens: Some(8192),
                tool_support: crate::providers::ToolSupport::Supported,
                supports_images: false,
            }],
        })
        .unwrap();
    store
        .save_preferred_model(&crate::providers::ModelSelection {
            provider_id: Some("fixture-loopback".into()),
            model_id: "fixture-model".into(),
        })
        .unwrap();
}

/// Bounded loopback SSE fixture shared by protocol integration tests. No
/// provider credentials or external network destination are involved.
#[derive(Clone)]
pub(crate) struct FixtureHttpRequest {
    pub headers: String,
    pub body: serde_json::Value,
}
pub(crate) async fn scripted_sse(
    responses: Vec<String>,
) -> (
    String,
    Arc<Mutex<Vec<FixtureHttpRequest>>>,
    tokio::task::JoinHandle<()>,
) {
    let count = responses.len();
    let mut responses = responses.into_iter();
    scripted_sse_with(count, move |_| responses.next().unwrap()).await
}

/// A request-aware fixture lets later model rounds inspect real process IDs
/// and reports returned by the preceding round, without bypassing dispatch.
pub(crate) async fn scripted_sse_with(
    count: usize,
    mut respond: impl FnMut(&serde_json::Value) -> String + Send + 'static,
) -> (
    String,
    Arc<Mutex<Vec<FixtureHttpRequest>>>,
    tokio::task::JoinHandle<()>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Protocol fixture requires loopback socket binding.");
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let records = requests.clone();
    let task = tokio::spawn(async move {
        for _ in 0..count {
            let (mut stream, _) =
                tokio::time::timeout(std::time::Duration::from_secs(10), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buffer = vec![0; 8192];
                let n = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    stream.read(&mut buffer),
                )
                .await
                .unwrap()
                .unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                assert!(bytes.len() <= 4 * 1024 * 1024);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    assert!(end <= 8192);
                    let headers = String::from_utf8(bytes[..end].to_vec()).unwrap();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(str::trim)
                                .map(str::to_owned)
                        })
                        .unwrap()
                        .parse::<usize>()
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        records.lock().unwrap().push(FixtureHttpRequest {
                            headers,
                            body: serde_json::from_slice(&bytes[end + 4..end + 4 + length])
                                .unwrap(),
                        });
                        break;
                    }
                }
            }
            let body = respond(&records.lock().unwrap().last().unwrap().body);
            let response=format!("HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
            stream.write_all(response.as_bytes()).await.unwrap();
        }
    });
    (base, requests, task)
}
