use crate::{
    chat::ThinkFilter,
    inference::{Backend, InferenceProvider},
    sse::SseDecoder,
    AppState,
};
use futures_util::StreamExt;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

fn fixture_correct(task: &str, answer: &str) -> bool {
    match task {
        "simple" => answer.trim() == "4",
        "coding" => {
            let code = answer
                .lines()
                .filter(|line| !line.trim_start().starts_with("```"))
                .collect::<Vec<_>>()
                .join("\n");
            let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
            matches!(
                compact.as_str(),
                "defadd(a,b):returna+b" | "defadd(a,b):return(a+b)"
            )
        }
        "research" => {
            let compact: String = answer
                .to_lowercase()
                .chars()
                .filter(|c| c.is_alphanumeric() || matches!(c, '[' | ']'))
                .collect();
            matches!(
                compact.as_str(),
                "alphais6msfaster[a][b]"
                    | "alphais6msfasterthanbeta[a][b]"
                    | "alphaisfasterby6ms[a][b]"
            )
        }
        _ => false,
    }
}

#[tauri::command]
pub async fn run_latency_diagnostic(
    state: tauri::State<'_, AppState>,
    task: String,
    mode: crate::store::ReasoningMode,
    samples: u32,
) -> Result<Value, String> {
    if !(1..=3).contains(&samples) {
        return Err("Choose one to three samples.".into());
    }
    let prompt = match task.as_str() {
        "simple" => "What is 2 + 2? Answer with the number only.",
        "coding" => "Return a Python function named add(a, b) that returns a + b. No explanation.",
        "research" => "Use only these supplied fictional records: [A] Alpha latency is 12 ms. [B] Beta latency is 18 ms. Reply exactly: Alpha is 6 ms faster. [A] [B]",
        _ => return Err("Unknown diagnostic task.".into()),
    };
    let _operation = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active model operation.")?;
    let (_, mut cancellation) = {
        let mut control = state
            .run_control
            .lock()
            .map_err(|_| "Run control unavailable.")?;
        crate::agent_run::begin_generation(&mut control, &state.cancel)
    };
    let (backend, model) = {
        let mut runtime = state.runtime.lock().await;
        let status = runtime.inspect();
        if status.phase != "ready" {
            return Err("Load a local model before running this diagnostic.".into());
        }
        (
            Backend::local(
                runtime.endpoint.clone(),
                runtime.api_key.clone(),
                runtime.context_length,
            )?,
            status.model_path,
        )
    };
    let Backend::Local {
        client,
        endpoint,
        key,
        ..
    } = &backend
    else {
        unreachable!()
    };
    let props: Value = client
        .get(format!("{}/props", endpoint.trim_end_matches('/')))
        .bearer_auth(key)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let template = props["chat_template"].as_str().unwrap_or("");
    let mut control = json!({});
    if mode != crate::store::ReasoningMode::Auto {
        if template.contains("enable_thinking") {
            control = json!({"enable_thinking":mode == crate::store::ReasoningMode::Think});
        } else if template.contains("reasoning_effort") {
            control = json!({"reasoning_effort":if mode == crate::store::ReasoningMode::Quick {"low"} else {"high"}});
        } else {
            return Err(
                "The loaded runtime template exposes no verified reasoning control. Choose Auto."
                    .into(),
            );
        }
    }
    let mut records = Vec::new();
    for tools in [false, true] {
        for sample in 0..samples {
            for cache in ["cold", "prime", "warm"] {
                let mut payload = json!({"messages":[{"role":"user","content":prompt}], "stream":true, "max_tokens":160, "temperature":0, "cache_prompt":cache != "cold", "id_slot":0});
                if control != json!({}) {
                    payload["chat_template_kwargs"] = control.clone();
                }
                if tools {
                    payload["tools"] = json!([{"type":"function","function":{"name":"diagnostic_fixture","description":"Synthetic diagnostic schema. No tool is executed.","parameters":{"type":"object","properties":{}}}}]);
                }
                let observe = async {
                    let started = Instant::now();
                    let response = backend.stream(&backend.payload(&payload)?).await?;
                    let mut stream = response.bytes_stream();
                    let mut decoder = SseDecoder::default();
                    let mut filter = ThinkFilter::new();
                    let mut answer = String::new();
                    let mut first_answer = None;
                    let mut first_token = None;
                    let mut done = false;
                    let mut finish_reason = String::new();
                    let mut tool_calls = false;
                    while let Some(bytes) = stream.next().await {
                        for event in decoder.push(&bytes.map_err(|e| e.to_string())?)? {
                            if event == "[DONE]" {
                                done = true;
                                break;
                            }
                            let value: Value =
                                serde_json::from_str(&event).map_err(|e| e.to_string())?;
                            if value.get("error").is_some() {
                                return Err(backend.stream_error(&value));
                            }
                            if let Some(reason) = value["choices"][0]["finish_reason"].as_str() {
                                finish_reason = reason.to_string();
                            }
                            let delta = &value["choices"][0]["delta"];
                            tool_calls |= delta["tool_calls"]
                                .as_array()
                                .is_some_and(|calls| !calls.is_empty());
                            let raw = delta["content"].as_str().unwrap_or("");
                            if first_token.is_none()
                                && (!raw.is_empty()
                                    || delta.get("reasoning_content").is_some()
                                    || delta.get("tool_calls").is_some())
                            {
                                first_token = Some(started.elapsed().as_secs_f64() * 1000.0);
                            }
                            let (visible, _) = filter.feed(raw);
                            if first_answer.is_none() && !visible.trim().is_empty() {
                                first_answer = Some(started.elapsed().as_secs_f64() * 1000.0);
                            }
                            answer.push_str(&visible);
                            if answer.len() > 65536 {
                                return Err("Diagnostic response exceeded its limit.".into());
                            }
                        }
                        if done {
                            break;
                        }
                    }
                    let (flush, _) = filter.flush();
                    if first_answer.is_none() && !flush.trim().is_empty() {
                        first_answer = Some(started.elapsed().as_secs_f64() * 1000.0);
                    }
                    answer.push_str(&flush);
                    let complete =
                        done && finish_reason == "stop" && !tool_calls && !answer.trim().is_empty();
                    let correct = fixture_correct(&task, &answer);
                    Ok::<Value, String>(
                        json!({"task":task,"mode":mode,"tools":tools,"cache":cache,"sample":sample+1,"firstTokenMs":first_token,"firstAnswerMs":first_answer,"totalMs":started.elapsed().as_secs_f64()*1000.0,"answer":answer,"finishReason":finish_reason,"toolCallsRequested":tool_calls,"complete":complete,"correct":correct && complete}),
                    )
                };
                let record = tokio::select! {
                    _ = cancellation.changed() => return Err("Diagnostic cancelled. No tools were executed.".into()),
                    result = tokio::time::timeout(Duration::from_secs(90), observe) => result.map_err(|_| "Diagnostic request timed out.")??,
                };
                records.push(record);
            }
        }
    }
    Ok(
        json!({"schemaVersion":1,"model":model,"task":task,"mode":mode,"records":records,"boundaries":["Native loopback HTTP client, before UI rendering.","Cold requests cache_prompt=false; warm follows an identical prime. Not cold disk or process.","Tool schemas are synthetic; no tool or generated code is executed.","Research checks supplied fictional evidence, not live web search.","Correctness is a fixture check; small samples establish no speedup."]}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restrictive_nonexecuting_fixtures_reject_comments_and_reversed_evidence() {
        assert!(fixture_correct(
            "coding",
            "```python\ndef add(a, b):\n    return a + b\n```"
        ));
        assert!(!fixture_correct("coding", "# def add(a, b): return a + b"));
        assert!(!fixture_correct(
            "coding",
            "def add(a, b): return a + b\nprint('side effect')"
        ));
        assert!(fixture_correct("research", "Alpha is 6 ms faster. [A] [B]"));
        assert!(!fixture_correct(
            "research",
            "Beta is 6 ms faster than Alpha [A][B]"
        ));
        assert!(!fixture_correct(
            "research",
            "Alpha is slower. Beta is faster by 6 ms. [A][B]"
        ));
    }
}
