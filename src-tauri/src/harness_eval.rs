//! Opt-in headless evaluation of the production harness in isolated state.
//! Never opens the user's database for writing. No benchmark runs at startup.
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Request {
    root: PathBuf,
    workspace: PathBuf,
    build_identity: String,
    prompt: String,
    base_url: String,
    model: String,
    #[serde(default = "context")]
    context_length: u32,
    #[serde(default = "output")]
    max_output_tokens: u32,
    #[serde(default = "deadline")]
    deadline_seconds: u64,
}
fn context() -> u32 {
    32768
}
fn output() -> u32 {
    4096
}
fn deadline() -> u64 {
    600
}

pub fn run() -> Result<(), String> {
    let argument = std::env::args()
        .nth(2)
        .ok_or("Evaluation requires a request JSON path.")?;
    if argument == "provider-config" {
        // Parent evaluation coordinator consumes this private pipe. It never
        // places credentials in argv, artifacts, request logs or the report.
        let data = PathBuf::from(std::env::args().nth(3).ok_or("Missing data directory")?);
        let db = rusqlite::Connection::open_with_flags(
            data.join("locallm.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| e.to_string())?;
        let (id,base,model):(String,String,String)=db.query_row(
            "SELECT p.id,p.base_url,json_extract(p.models,'$[0].id') FROM providers p WHERE lower(p.name) LIKE '%river%' LIMIT 1",[],|row|Ok((row.get(0)?,row.get(1)?,row.get(2)?))).map_err(|e|e.to_string())?;
        let vault = crate::vault::Vault::new(data.join("credentials"));
        let key = vault
            .load(&crate::providers::credential_id(&id))?
            .ok_or("Configured River credential is unavailable.")?;
        let key = String::from_utf8(key).map_err(|_| "Credential is not UTF-8")?;
        println!("{}", json!({"baseUrl":base,"model":model,"apiKey":key}));
        return Ok(());
    }
    let request: Request =
        serde_json::from_slice(&std::fs::read(argument).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?
        .block_on(evaluate(request))
}
async fn evaluate(request: Request) -> Result<(), String> {
    std::fs::create_dir_all(&request.root).map_err(|e| e.to_string())?;
    if request.root.join("fixture-state/locallm.sqlite").exists() {
        return Err("Evaluation root already contains a run. Use a fresh trial directory.".into());
    }
    let workspace = std::fs::canonicalize(&request.workspace).map_err(|e| e.to_string())?;
    if !workspace.starts_with(std::fs::canonicalize(&request.root).map_err(|e| e.to_string())?) {
        return Err("Evaluation workspace must be inside its isolated trial directory.".into());
    }
    let state = crate::test_support::app_state(&request.root);
    let conversation = {
        let store = state.database()?;
        store.save_workspace_path(workspace.to_str().ok_or("Non-UTF8 workspace")?)?;
        let provider = crate::providers::ProviderConnection {
            id: "eval-loopback".into(),
            name: "Evaluation proxy".into(),
            api_format: crate::providers::FREETOKEN_OPENAI_COMPAT.into(),
            base_url: request.base_url.clone(),
            verified: true,
            last_tested_at: None,
            has_api_key: false,
            models: vec![crate::providers::RemoteModel {
                id: request.model.clone(),
                context_length: Some(request.context_length),
                max_output_tokens: Some(request.max_output_tokens),
                tool_support: crate::providers::ToolSupport::Supported,
                supports_images: false,
                controls: None,
            }],
        };
        store.save_provider(&provider)?;
        store.save_preferred_model(&crate::providers::ModelSelection {
            provider_id: Some(provider.id),
            model_id: request.model.clone(),
        })?;
        let mut prefs = store.preferences()?;
        prefs.temperature = 0.2;
        prefs.max_tokens = request.max_output_tokens;
        prefs.system_prompt="You are a capable general-purpose assistant. Complete the user's task accurately. Use available tools to create requested files and retrieve sources. Distinguish what you verified from what remains uncertain. Continue useful work until the request is complete or a real blocker prevents it.".into();
        store.save_preferences(&prefs)?;
        store.set_presentation_mode(None, Some(crate::presentation::Mode::Text))?;
        let mut config = store.execution_config()?;
        config.bash.timeout_seconds = 300;
        config.python.timeout_seconds = 90;
        config.validate()?;
        store.save_execution_config(&config)?;
        let conversation = store.create_conversation()?;
        store.save_conversation_tools(
            &conversation.id,
            &crate::store::ConversationTools {
                access_mode: crate::permissions::AccessMode::FullAccess,
                sources: vec!["__workspace".into(), "__execution".into()],
                tools: vec![],
            },
        )?;
        conversation.id
    };
    let plan = crate::chat::assemble_turn(
        &state,
        &conversation,
        &request.prompt,
        vec!["__workspace".into(), "__execution".into()],
        Some(vec![]),
        None,
        None,
    )
    .await?;
    write_json(
        &request.root.join("configuration.json"),
        &json!({"buildIdentity":request.build_identity,"model":request.model,"contextLength":request.context_length,"maxOutputTokens":request.max_output_tokens,"prompt":request.prompt,"catalog":plan.tool_definitions,"permissions":"fullAccess in isolated workspace","deadlineSeconds":request.deadline_seconds}),
    )?;
    let started = std::time::Instant::now();
    let operation = crate::chat::send_message_inner(
        &state,
        conversation.clone(),
        request.prompt,
        Some(vec!["__workspace".into(), "__execution".into()]),
        Some(vec![]),
        None,
        None,
        tauri::ipc::Channel::new(|_| Ok(())),
    );
    tokio::pin!(operation);
    let result = tokio::select! {
        result=&mut operation=>result,
        _=tokio::time::sleep(std::time::Duration::from_secs(request.deadline_seconds))=>{
            if let Ok(mut control)=state.run_control.lock() {crate::agent_run::request_cancel(&mut control);}
            state.cancel.send_replace(true);
            let _=tokio::time::timeout(std::time::Duration::from_secs(10),&mut operation).await;
            Err("Explicit evaluation wall-clock budget exhausted.".into())
        }
    };
    state.processes.stop_owned(&conversation);
    let store = state.database()?;
    write_json(
        &request.root.join("result.json"),
        &json!({"buildIdentity":request.build_identity,"model":request.model,"conversationId":conversation,"elapsedMs":started.elapsed().as_millis(),"error":result.err(),"run":store.latest_run(&conversation)?,"messages":store.messages(&conversation)?}),
    )?;
    println!("{}", json!({"result":request.root.join("result.json")}));
    Ok(())
}
fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    std::fs::write(
        path,
        serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}
