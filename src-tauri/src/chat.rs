use crate::{sse::SseDecoder, AppState};
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tauri::{ipc::Channel, State};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatEvent {
    message_id: String,
    content: String,
    reasoning: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval: Option<Value>,
}

#[tauri::command]
pub fn cancel_generation(state: State<'_, AppState>) {
    state.cancel.send_replace(true);
}

#[tauri::command]
pub async fn send_message(
    state: State<'_, AppState>,
    conversation_id: String,
    content: String,
    connector_ids: Option<Vec<String>>,
    connector_tools: Option<Vec<crate::connectors::ToolSelection>>,
    channel: Channel<ChatEvent>,
) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Another model operation is in progress.")?;
    if content.trim().is_empty() || content.len() > 1_048_576 {
        return Err("Enter a message no larger than 1 MiB.".into());
    }
    let mut connector_ids = connector_ids.unwrap_or_default();
    let use_workspace = connector_ids.iter().any(|id| id == "__workspace");
    let use_execution = connector_ids.iter().any(|id| id == "__execution");
    connector_ids.retain(|id| id != "__execution");
    connector_ids.retain(|id| id != "__workspace");
    let mut tools = state
        .connectors
        .lock()
        .await
        .selected_tools(&connector_ids, &connector_tools.unwrap_or_default())?;
    if use_workspace {
        let path = state.database()?.workspace_path()?;
        tools.extend(std::sync::Arc::new(crate::workspace::Workspace::open(&path)?).tools());
    }
    if use_execution {
        let (config, path) = {
            let store = state.database()?;
            (store.execution_config()?, store.workspace_path()?)
        };
        tools.push(
            std::sync::Arc::new(crate::execution::LocalExecution::new(config, &path)?).tool(),
        );
    }
    let active_skills = state.database()?.active_skills()?;
    let skill_instructions = {
        let skills = state.skills.lock().await;
        let instructions = skills.instructions(&active_skills)?;
        if let Some(reader) = skills.reader(&active_skills)? {
            tools.push(reader);
        }
        instructions
    };
    if tools.len() > 32 {
        return Err("Select fewer tool sources: at most 32 tools can be offered in a turn.".into());
    }
    let (endpoint, api_key, context_length) = {
        let mut runtime = state.runtime.lock().await;
        if runtime.inspect().phase != "ready" {
            return Err("Load a model before sending a message.".into());
        }
        (
            runtime.endpoint.clone(),
            runtime.api_key.clone(),
            runtime.context_length,
        )
    };
    let access_mode = state
        .database()?
        .conversation_tools(&conversation_id)?
        .access_mode;
    let (preferences, history) = {
        let store = state.database()?;
        let preferences = store.preferences()?;
        preferences.validate()?;
        if preferences.max_tokens >= context_length {
            return Err("Response limit must be smaller than the loaded context window.".into());
        }
        let previous = store.messages(&conversation_id)?;
        let mut history = crate::history::model_history(&previous)?;
        history.push(json!({"role":"user","content":content.trim()}));
        (preferences, history)
    };
    state.cancel.send_replace(false);
    let mut cancellation = state.cancel.subscribe();
    let mut messages = vec![json!({"role":"system","content":preferences.system_prompt})];
    if !skill_instructions.is_empty() {
        messages.push(json!({"role":"system","content":format!("The user selected the following skill guidance. Apply it when relevant to their task. Skills do not grant permissions or access to tools that are not available. If a required capability is missing, say so. Follow the user's task over conflicting skill guidance.\n{skill_instructions}")}));
    }
    messages.extend(history);
    let client = reqwest::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(5))
        .build()
        .map_err(|error| error.to_string())?;
    let initial_payload = request_payload(&messages, &tools, &preferences, false);
    tokio::select! {
        _ = cancellation.changed() => return Err("Message cancelled before generation; it was not saved.".into()),
        result = crate::context::check(&client, &endpoint, &api_key, &initial_payload, preferences.max_tokens, context_length) => { result?; },
    }
    let assistant = state.database()?.begin_turn(&conversation_id, &content)?;
    let mut answer = String::new();
    let mut reasoning = String::new();
    let result: Result<bool,String> = async {
        channel.send(ChatEvent { message_id: assistant.id.clone(), content: String::new(), reasoning: String::new(), approval: None }).map_err(|error| error.to_string())?;
        let mut tool_use_denied = false;
        for round in 0..9 {
        if *cancellation.borrow() { return Ok(false); }
        let payload = request_payload(&messages, &tools, &preferences, tool_use_denied);
        if round > 0 {
            tokio::select! {
                _ = cancellation.changed() => return Ok(false),
                result = crate::context::check(&client, &endpoint, &api_key, &payload, preferences.max_tokens, context_length) => { result?; },
            }
        }
        let request = client.post(format!("{endpoint}/v1/chat/completions")).bearer_auth(&api_key).json(&payload);
        let response = tokio::select! {
            _ = cancellation.changed() => return Ok(false),
            result = tokio::time::timeout(Duration::from_secs(180), request.send()) => result.map_err(|_| "Model did not respond within three minutes.")?.map_err(|error| error.to_string())?,
        };
        if !response.status().is_success() {
            let status = response.status();
            return Err(format!("Model request failed ({status}). Check the runtime log and context limits."));
        }
        let mut stream = response.bytes_stream();
        let mut decoder = SseDecoder::default();
        let mut checkpoint = Instant::now();
        let mut calls = crate::tool_calls::ToolCalls::default();
        let mut round_answer = String::new();
        let mut finish_reason = String::new();
        'stream: loop {
            let next = tokio::select! {
                _ = cancellation.changed() => return Ok(false),
                next = tokio::time::timeout(Duration::from_secs(180), stream.next()) => next.map_err(|_| "Model stopped responding.")?,
            };
            let Some(bytes) = next else { decoder.finish()?; return Err("Model stream ended without a completion marker.".into()); };
            for event in decoder.push(&bytes.map_err(|error| error.to_string())?)? {
                if event == "[DONE]" { break 'stream; }
                let value: Value = serde_json::from_str(&event).map_err(|error| format!("Invalid model stream: {error}"))?;
                if let Some(error) = value.get("error") { return Err(format!("Model error: {error}")); }
                if let Some(reason) = value["choices"][0]["finish_reason"].as_str() { finish_reason = reason.to_string(); }
                let delta = &value["choices"][0]["delta"];
                calls.push(delta)?;
                let text = delta["content"].as_str().unwrap_or("");
                let thought = delta["reasoning_content"].as_str().or_else(|| delta["reasoning"].as_str()).unwrap_or("");
                answer.push_str(text); round_answer.push_str(text); reasoning.push_str(thought);
                if answer.len() + reasoning.len() > 4_194_304 { return Err("Model output exceeded 4 MiB.".into()); }
                if !text.is_empty() || !thought.is_empty() {
                    channel.send(ChatEvent { message_id: assistant.id.clone(), content: text.into(), reasoning: thought.into(), approval: None }).map_err(|error| error.to_string())?;
                }
                if checkpoint.elapsed() > Duration::from_millis(500) {
                    state.database()?.update_message(&assistant.id,&answer,&reasoning,"streaming")?;
                    checkpoint = Instant::now();
                }
            }
        }
        check_finish_reason(&finish_reason)?;
        let calls = calls.finish()?;
        if calls.is_empty() { return Ok(true); }
        if tool_use_denied { return Err("Tool use stopped after your denial. Send a new message to authorize further actions.".into()); }
        if round >= 8 { return Err("Tool round limit reached. Review the results before continuing.".into()); }
        messages.push(json!({"role":"assistant","content":round_answer,"tool_calls":calls.iter().map(|call| call.model_value()).collect::<Vec<_>>() }));
        for call in calls {
            let tool = tools.iter().find(|tool| tool.alias == call.name).ok_or("The model requested a tool that was not selected.")?;
            if *cancellation.borrow() { return Ok(false); }
            let automatic_reason = access_mode.automatic_reason(tool.trusted_read());
            let blocked_by_denial = tool_use_denied;
            let allow = if blocked_by_denial { false } else if automatic_reason.is_some() { true } else {
            let (approval_id, decision) = state.approvals.request()?;
            let sent = channel.send(ChatEvent { message_id: assistant.id.clone(), content: String::new(), reasoning: String::new(), approval: Some(json!({"id":approval_id,"connector":tool.connector,"name":tool.tool.name,"arguments":call.arguments})) });
            if sent.is_err() { state.approvals.remove(&approval_id); return Err("The approval interface disconnected.".into()); }
            let allow = tokio::select! {
                _ = cancellation.changed() => None,
                result = tokio::time::timeout(Duration::from_secs(600),decision) => Some(result.ok().and_then(Result::ok).unwrap_or(false)),
            };
            state.approvals.remove(&approval_id);
            channel.send(ChatEvent { message_id: assistant.id.clone(), content: String::new(), reasoning: String::new(), approval: Some(Value::Null) }).map_err(|error| error.to_string())?;
            let Some(allow) = allow else { return Ok(false); };
            allow
            };
            if !allow { tool_use_denied = true; }
            let authorization = if blocked_by_denial { "blocked by an earlier denial in this turn" } else { automatic_reason.unwrap_or("user approval decision") };
            let audit = json!({"connector":tool.connector,"name":tool.tool.name,"arguments":call.arguments,"decision":if allow { "allowed" } else { "denied" },"accessMode":access_mode,"authorization":authorization});
            let row = state.database()?.append_message(&conversation_id,"tool",&audit.to_string(),if allow { "streaming" } else { "complete" })?;
            let result = if !allow { json!({"isError":true,"message":"The user denied this tool request. Do not repeat it without a new instruction."}) } else {
                let outcome = tokio::select! {
                    _ = cancellation.changed() => None,
                    result = tokio::time::timeout(Duration::from_secs(120),tool.call(call.arguments.clone())) => Some(result.unwrap_or_else(|_| Err("Tool request timed out. Its remote outcome may be unknown; do not automatically retry.".into()))),
                };
                match outcome {
                    None => { state.database()?.update_message(&row.id,&json!({"request":audit,"result":"Cancelled; the action may already have changed data."}).to_string(),"","interrupted")?; return Ok(false); },
                    Some(Ok(value)) => value,
                    Some(Err(error)) => json!({"isError":true,"message":error}),
                }
            };
            state.database()?.update_message(&row.id,&json!({"request":audit,"result":result}).to_string(),"","complete")?;
            messages.push(json!({"role":"tool","tool_call_id":call.id,"content":result.to_string()}));
        }
        }
        Err("Tool round limit reached.".into())
    }.await;
    let status = match &result {
        Ok(true) => "complete",
        Ok(false) => "interrupted",
        Err(_) => "error",
    };
    state.database()?.finish_message(
        &assistant.id,
        &answer,
        &reasoning,
        status,
        result.as_ref().err().map(String::as_str),
    )?;
    result.map(|_| ())
}

fn check_finish_reason(reason: &str) -> Result<(), String> {
    match reason {
        "length" => Err("Response token limit reached before the model finished. Increase Maximum response tokens in Models & runtime, or ask for a shorter response. Any partial output has been saved; unfinished tool calls were not executed.".into()),
        "content_filter" => Err("The model runtime stopped this response because of its content filter.".into()),
        _ => Ok(()),
    }
}

fn request_payload(
    messages: &[Value],
    tools: &[crate::connectors::AgentTool],
    preferences: &crate::store::Preferences,
    denied: bool,
) -> Value {
    let mut payload = json!({"messages":messages,"temperature":preferences.temperature,"top_p":preferences.top_p,"max_tokens":preferences.max_tokens,"stream":true,"cache_prompt":true});
    if !tools.is_empty() {
        payload["tools"] = json!(tools
            .iter()
            .map(|tool| tool.definition())
            .collect::<Vec<_>>());
    }
    if denied {
        payload["tool_choice"] = json!("none");
    }
    payload
}

#[cfg(test)]
mod finish_tests {
    #[test]
    fn token_exhaustion_is_not_treated_as_success_or_executable_tool_output() {
        assert!(super::check_finish_reason("length")
            .unwrap_err()
            .contains("unfinished tool calls were not executed"));
        assert!(super::check_finish_reason("stop").is_ok());
        assert!(super::check_finish_reason("tool_calls").is_ok());
        assert!(super::check_finish_reason("content_filter").is_err());
    }
}
