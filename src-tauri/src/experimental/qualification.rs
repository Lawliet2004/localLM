//! Opt-in laptop acceptance; never touches the user's application database.
use super::*;
use crate::{runtime::Runtime, runtime_config::RuntimeConfig, store::Preferences};
use futures_util::StreamExt;
use std::time::{Duration, Instant};

fn evidence() -> PathBuf {
    development().join("bonsai-evidence")
}
fn write(name: &str, value: &Value) -> Result<(), String> {
    std::fs::write(
        evidence().join(name),
        serde_json::to_vec_pretty(value).unwrap(),
    )
    .map_err(|e| e.to_string())
}
fn candidate() -> Result<RuntimeConfig, String> {
    let bytes =
        std::fs::read(evidence().join("candidate-preset.json")).map_err(|e| e.to_string())?;
    let config: RuntimeConfig = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    config.validate()?;
    Ok(config)
}
fn preferences() -> Result<Preferences, String> {
    let runtime =
        find_runtime(&development().join(ROOT))?.ok_or("Acquire the PrismML runtime first.")?;
    Ok(Preferences {
        model_path: model_path(&development()).to_string_lossy().into_owned(),
        runtime_path: runtime.to_string_lossy().into_owned(),
        ..Default::default()
    })
}
fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())
}
async fn completion(
    client: &reqwest::Client,
    runtime: &Runtime,
    payload: &Value,
) -> Result<Value, String> {
    let response = client
        .post(format!("{}/v1/chat/completions", runtime.endpoint))
        .bearer_auth(&runtime.api_key)
        .json(payload)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = response.status();
    let value: Value = response.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {status}: {value}"));
    }
    Ok(value)
}
async fn workload_payload(
    client: &reqwest::Client,
    runtime: &Runtime,
    round: usize,
    cached: bool,
) -> Result<Value, String> {
    let mut lo = 0;
    let mut hi = 3000;
    while lo <= hi {
        let padding = (lo + hi) / 2;
        let mut payload = json!({"messages":[{"role":"user","content":format!("Record {round}. Context: {}\nWrite a long coherent story about a gardener who teaches a child to grow an apple tree. Continue with practical observations and keep writing until the reply limit.", " orchard".repeat(padding))}],
            "stream":false,"temperature":0.7,"top_p":0.95,"top_k":20,"seed":42,"max_tokens":500,"ignore_eos":true,"cache_prompt":cached,"chat_template_kwargs":{"enable_thinking":false}});
        let tokens =
            crate::context::count_only(client, &runtime.endpoint, &runtime.api_key, &payload)
                .await?;
        if tokens == 2000 {
            payload["id_slot"] = json!(0);
            return Ok(payload);
        }
        if tokens < 2000 {
            lo = padding + 1;
        } else if padding == 0 {
            break;
        } else {
            hi = padding - 1;
        }
    }
    Err("Could not construct an exactly 2000-token chat input with the embedded template.".into())
}

#[tokio::test]
#[ignore = "real CUDA laptop qualification; invoke scripts/bonsai-qualification.mjs explicitly"]
async fn laptop_qualification() -> Result<(), String> {
    std::fs::create_dir_all(evidence()).map_err(|e| e.to_string())?;
    match std::env::var("BONSAI_QA_MODE").as_deref() {
        Ok("workload") => workload(false).await,
        Ok("soak") => workload(true).await,
        Ok("integration") => integration().await,
        Ok("max-context") => max_context().await,
        _ => Err("Select workload, soak, integration, or max-context explicitly.".into()),
    }
}

async fn workload(soak: bool) -> Result<(), String> {
    let config = candidate()?;
    let name = if soak { "soak" } else { "workload" };
    let mut runtime = Runtime::new(evidence().join(format!("{name}-runtime.log")));
    let loaded = runtime.load(&preferences()?, &config).await?;
    println!("{name} loaded: {}", serde_json::to_string(&loaded).unwrap());
    let client = client()?;
    let started = Instant::now();
    let mut records = Vec::new();
    while if soak {
        started.elapsed() < Duration::from_secs(1800)
    } else {
        records.len() < 5
    } {
        let payload = workload_payload(&client, &runtime, records.len(), soak).await?;
        let round = Instant::now();
        let response = completion(&client, &runtime, &payload).await?;
        let content = response["choices"][0]["message"]["content"]
            .as_str()
            .ok_or("Missing final content")?;
        if content.trim().is_empty() || content.contains('\u{fffd}') {
            return Err("Empty or malformed workload output.".into());
        }
        if response["usage"]["prompt_tokens"] != 2000
            || response["usage"]["completion_tokens"] != 500
        {
            write(&format!("{name}-unexpected-response.json"), &response)?;
            return Err(format!(
                "Expected 2000 input + 500 output, got {}",
                response["usage"]
            ));
        }
        records.push(json!({"round":records.len()+1,"elapsedSeconds":started.elapsed().as_secs_f64(),"requestSeconds":round.elapsed().as_secs_f64(),"response":response}));
        write(
            &format!("{name}.json"),
            &json!({"config":config,"elapsedSeconds":started.elapsed().as_secs_f64(),"records":records}),
        )?;
        let last = records.last().unwrap();
        println!(
            "{name} round {}: {}",
            records.len(),
            last["response"]["timings"]
        );
        if runtime.inspect().phase != "ready" {
            return Err("Server died during workload.".into());
        }
    }
    runtime.stop().await?;
    println!(
        "{name} complete: {} rounds, {:.3} seconds",
        records.len(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

async fn short_chat(client: &reqwest::Client, runtime: &Runtime) -> Result<Value, String> {
    let value = completion(client, runtime, &json!({"messages":[{"role":"user","content":"What is the capital of France? Answer briefly."}],"max_tokens":128,"chat_template_kwargs":{"enable_thinking":false}})).await?;
    if !value["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .to_lowercase()
        .contains("paris")
    {
        return Err(format!("Incoherent factual answer: {value}"));
    }
    Ok(value)
}

async fn integration() -> Result<(), String> {
    let directory = tempfile::tempdir().map_err(|e| e.to_string())?;
    let state = crate::web_search::tests::test_state(directory.path());
    let mini = Preferences {
        model_path: development()
            .join("models/MiniCPM5-2B.Q6_K.gguf")
            .to_string_lossy()
            .into_owned(),
        runtime_path: development()
            .join("runtime/llama-server.exe")
            .to_string_lossy()
            .into_owned(),
        ..Default::default()
    };
    let config = RuntimeConfig::default();
    state.database()?.save_model_runtime(&mini, &config)?;
    let client = client()?;
    let before = crate::commands::load_selected_model(&state).await?;
    let mini_answer = short_chat(&client, &*state.runtime.lock().await).await?;
    let missing_flag_error = activate_inner(&state, false)
        .await
        .err()
        .ok_or("Off flag allowed activation")?;
    assert!(missing_flag_error.contains("Enable Experimental"));
    assert_eq!(state.runtime.lock().await.inspect().phase, "ready");
    set_enabled_inner(&state, true).await?;
    let bonsai = activate_inner(&state, true).await?;
    assert_eq!(
        state.database()?.preferences()?.runtime_path,
        preferences()?.runtime_path
    );
    let bonsai_answer = short_chat(&client, &*state.runtime.lock().await).await?;
    let previous: Option<Previous> = state.database()?.setting(PREVIOUS_KEY)?;
    restore(
        &state,
        previous.as_ref().ok_or("No saved MiniCPM selection")?,
        true,
    )
    .await?;
    assert_eq!(state.database()?.preferences()?.model_path, mini.model_path);
    assert_eq!(state.database()?.runtime_config()?, config);
    let mini_after = short_chat(&client, &*state.runtime.lock().await).await?;
    let bonsai_again = activate_inner(&state, false).await?;
    let mut record = json!({"miniBefore":before,"miniAnswer":mini_answer,"offError":missing_flag_error,"bonsai":bonsai,"bonsaiAnswer":bonsai_answer,"miniAfter":mini_after,"bonsaiAgain":bonsai_again});
    write("integration.json", &record)?;

    // Real SSE includes reasoning deltas; the existing parser already accepts this field.
    let runtime = state.runtime.lock().await;
    let unauthorized = client
        .get(format!("{}/v1/models", runtime.endpoint))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .status();
    assert_eq!(unauthorized.as_u16(), 401);
    let mut stream = client.post(format!("{}/v1/chat/completions", runtime.endpoint)).bearer_auth(&runtime.api_key)
        .json(&json!({"messages":[{"role":"user","content":"Calculate 17+25 and explain briefly."}],"stream":true,"max_tokens":512,"chat_template_kwargs":{"enable_thinking":true}}))
        .send().await.map_err(|e| e.to_string())?.bytes_stream();
    let mut sse = String::new();
    while let Some(bytes) = stream.next().await {
        sse.push_str(
            std::str::from_utf8(&bytes.map_err(|e| e.to_string())?).map_err(|e| e.to_string())?,
        );
    }
    let mut thought = String::new();
    let mut answer = String::new();
    for line in sse.lines().filter_map(|l| l.strip_prefix("data: ")) {
        if let Ok(value) = serde_json::from_str::<Value>(line) {
            let delta = &value["choices"][0]["delta"];
            thought.push_str(delta["reasoning_content"].as_str().unwrap_or(""));
            answer.push_str(delta["content"].as_str().unwrap_or(""));
        }
    }
    assert!(!thought.is_empty());
    assert!(answer.contains("42"));
    assert!(!answer.contains("<think>"));
    record["streaming"] = json!({"unauthorizedStatus":401,"reasoning":thought,"content":answer});
    std::fs::write(evidence().join("thinking.sse.txt"), sse).map_err(|e| e.to_string())?;
    drop(runtime);
    write("integration.json", &record)?;

    // Simulate insufficient VRAM with the maximum model context and full F16 cache.
    let oom = RuntimeConfig {
        context_length: 262144,
        cache_type_k: crate::runtime_config::CacheType::F16,
        cache_type_v: crate::runtime_config::CacheType::F16,
        ..candidate()?
    };
    state.database()?.save_runtime_config(&oom)?;
    let error = crate::commands::load_selected_model(&state)
        .await
        .err()
        .ok_or("Expected insufficient VRAM to fail")?;
    let oom_error = recover_selected_failure(&state, error).await;
    assert_eq!(state.database()?.preferences()?.model_path, mini.model_path);
    assert_eq!(state.database()?.runtime_config()?, config);
    assert_eq!(state.runtime.lock().await.inspect().phase, "ready");
    record["oomError"] = json!(oom_error);
    record["miniAfterOom"] = short_chat(&client, &*state.runtime.lock().await).await?;
    write("integration.json", &record)?;

    // Hide only the isolated runtime executable, restoring it even if the assertion fails.
    let executable = PathBuf::from(preferences()?.runtime_path);
    let hidden = executable.with_extension("exe.qa-missing");
    std::fs::rename(&executable, &hidden).map_err(|e| e.to_string())?;
    let missing = activate_inner(&state, true).await;
    std::fs::rename(&hidden, &executable).map_err(|e| e.to_string())?;
    let missing = missing.err().ok_or("Missing runtime was accepted")?;
    assert!(missing.contains("missing or incomplete"));
    assert_eq!(state.runtime.lock().await.inspect().phase, "ready");
    record["missingRuntimeError"] = json!(missing);

    let _ = activate_inner(&state, true).await?;
    // The QA driver kills this exact isolated process after the first streamed token.
    let runtime = state.runtime.lock().await;
    let mut stream = client.post(format!("{}/v1/chat/completions", runtime.endpoint)).bearer_auth(&runtime.api_key)
        .json(&json!({"messages":[{"role":"user","content":"Write a very long story about an orchard."}],"stream":true,"max_tokens":1000,"ignore_eos":true,"chat_template_kwargs":{"enable_thinking":false}}))
        .send().await.map_err(|e| e.to_string())?.bytes_stream();
    let mut first = Vec::new();
    loop {
        first.extend_from_slice(
            &stream
                .next()
                .await
                .ok_or("Stream ended before first text token")?
                .map_err(|e| e.to_string())?,
        );
        if String::from_utf8_lossy(&first)
            .lines()
            .filter_map(|l| l.strip_prefix("data: "))
            .filter_map(|l| serde_json::from_str::<Value>(l).ok())
            .any(|v| {
                v["choices"][0]["delta"]["content"]
                    .as_str()
                    .is_some_and(|s| !s.is_empty())
            })
        {
            break;
        }
    }
    println!("BONSAI_QA_KILL_READY");
    let mut interrupted = false;
    while let Some(chunk) = stream.next().await {
        if chunk.is_err() {
            interrupted = true;
            break;
        }
    }
    drop(runtime);
    if !interrupted {
        return Err("QA driver did not interrupt the real generation.".into());
    }
    let status = state.runtime.lock().await.inspect();
    assert_eq!(status.phase, "error");
    record["killedMidGeneration"] = json!({"firstChunkBytes":first.len(),"status":status});
    set_enabled_inner(&state, false).await?;
    crate::commands::load_selected_model(&state).await?;
    record["miniAfterKill"] = short_chat(&client, &*state.runtime.lock().await).await?;
    state.runtime.lock().await.stop().await?;
    write("integration.json", &record)?;
    Ok(())
}

async fn max_context() -> Result<(), String> {
    let mut records = Vec::new();
    for context in [262144, 131072, 65536] {
        let config = RuntimeConfig {
            context_length: context,
            ..candidate()?
        };
        let mut runtime = Runtime::new(evidence().join(format!("max-context-{context}.log")));
        let result = async {
            let status = runtime.load(&preferences()?, &config).await?;
            let client = client()?;
            let response = completion(
                &client,
                &runtime,
                &workload_payload(&client, &runtime, 0, false).await?,
            )
            .await?;
            Ok::<_, String>(json!({"status":status,"response":response}))
        }
        .await;
        runtime.stop().await?;
        println!(
            "Context {context}: {}",
            if result.is_ok() { "passed" } else { "failed" }
        );
        records.push(
            json!({"context":context,"result":result.as_ref().ok(),"error":result.as_ref().err()}),
        );
        write("max-context.json", &json!(records))?;
        if result.is_ok() {
            break;
        }
    }
    if records.last().unwrap()["result"].is_null() {
        return Err("No extended context passed.".into());
    }
    Ok(())
}
