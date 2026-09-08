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
    channel: Channel<ChatEvent>,
) -> Result<(), String> {
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Another model operation is in progress.")?;
    if content.trim().is_empty() || content.len() > 1_048_576 {
        return Err("Enter a message no larger than 1 MiB.".into());
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
    let (preferences, history, assistant) = {
        let store = state.database()?;
        let preferences = store.preferences()?;
        preferences.validate()?;
        if preferences.max_tokens >= context_length {
            return Err("Response limit must be smaller than the loaded context window.".into());
        }
        let previous = store.messages(&conversation_id)?;
        store.append_message(&conversation_id, "user", content.trim(), "complete")?;
        if previous.is_empty() {
            store.rename_conversation(
                &conversation_id,
                &content.trim().chars().take(64).collect::<String>(),
            )?;
        }
        let history = store.messages(&conversation_id)?;
        let assistant = store.append_message(&conversation_id, "assistant", "", "streaming")?;
        (preferences, history, assistant)
    };
    state.cancel.send_replace(false);
    let mut cancellation = state.cancel.subscribe();
    let mut messages = vec![json!({"role":"system","content":preferences.system_prompt})];
    for message in history {
        if message.role == "user"
            || (message.role == "assistant"
                && message.status == "complete"
                && !message.content.is_empty())
        {
            messages.push(json!({"role":message.role,"content":message.content}));
        }
    }
    let mut answer = String::new();
    let mut reasoning = String::new();
    let result: Result<bool,String> = async {
        channel.send(ChatEvent { message_id: assistant.id.clone(), content: String::new(), reasoning: String::new() }).map_err(|error| error.to_string())?;
        let client = reqwest::Client::builder().no_proxy().connect_timeout(Duration::from_secs(5)).build().map_err(|error| error.to_string())?;
        let request = client.post(format!("{endpoint}/v1/chat/completions")).bearer_auth(api_key).json(&json!({
            "messages":messages,"temperature":preferences.temperature,"top_p":preferences.top_p,
            "max_tokens":preferences.max_tokens,"stream":true,"cache_prompt":true,
        }));
        let response = tokio::select! {
            _ = cancellation.changed() => return Ok(false),
            result = tokio::time::timeout(Duration::from_secs(180), request.send()) => result.map_err(|_| "Model did not respond within three minutes.")?.map_err(|error| error.to_string())?,
        };
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.map_err(|error| error.to_string())?;
            return Err(format!("Model request failed ({status}): {}", body.chars().take(1000).collect::<String>()));
        }
        let mut stream = response.bytes_stream();
        let mut decoder = SseDecoder::default();
        let mut checkpoint = Instant::now();
        loop {
            let next = tokio::select! {
                _ = cancellation.changed() => return Ok(false),
                next = tokio::time::timeout(Duration::from_secs(180), stream.next()) => next.map_err(|_| "Model stopped responding.")?,
            };
            let Some(bytes) = next else { decoder.finish()?; return Err("Model stream ended without a completion marker.".into()); };
            for event in decoder.push(&bytes.map_err(|error| error.to_string())?)? {
                if event == "[DONE]" { return Ok(true); }
                let value: Value = serde_json::from_str(&event).map_err(|error| format!("Invalid model stream: {error}"))?;
                if let Some(error) = value.get("error") { return Err(format!("Model error: {error}")); }
                let delta = &value["choices"][0]["delta"];
                let text = delta["content"].as_str().unwrap_or("");
                let thought = delta["reasoning_content"].as_str().or_else(|| delta["reasoning"].as_str()).unwrap_or("");
                answer.push_str(text); reasoning.push_str(thought);
                if answer.len() + reasoning.len() > 4_194_304 { return Err("Model output exceeded 4 MiB.".into()); }
                if !text.is_empty() || !thought.is_empty() {
                    channel.send(ChatEvent { message_id: assistant.id.clone(), content: text.into(), reasoning: thought.into() }).map_err(|error| error.to_string())?;
                }
                if checkpoint.elapsed() > Duration::from_millis(500) {
                    state.database()?.update_message(&assistant.id,&answer,&reasoning,"streaming")?;
                    checkpoint = Instant::now();
                }
            }
        }
    }.await;
    let status = match &result {
        Ok(true) => "complete",
        Ok(false) => "interrupted",
        Err(_) => "error",
    };
    state
        .database()?
        .update_message(&assistant.id, &answer, &reasoning, status)?;
    result.map(|_| ())
}
