//! Real fork/model acceptance through LocalLM's existing managed Runtime.
use crate::{
    runtime::Runtime,
    runtime_config::{CacheType, RuntimeConfig},
    store::Preferences,
};
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[tokio::test]
#[ignore = "requires the acquired Bonsai Q1_0 model, pinned CUDA fork, and target NVIDIA GPU"]
async fn managed_chat_acceptance() -> Result<(), String> {
    let project = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let root = project.join(".local/experimental/bonsai-27b");
    let evidence = project.join(".local/bonsai-evidence");
    let stock = std::env::var("BONSAI_ACCEPTANCE_RUNTIME").as_deref() == Ok("stock");
    let prefix = if stock {
        "managed-stock-bonsai"
    } else {
        "managed-bonsai"
    };
    let runtime_dir = if stock {
        project.join(".local/runtime")
    } else {
        std::fs::read_dir(&root)
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .find(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("prism-b10754-2459f68-cuda12.4-")
            })
            .ok_or("Acquire the Bonsai runtime first.")?
            .path()
    };
    let model = root.join("models/Bonsai-27B-Q1_0.gguf");
    if std::fs::metadata(&model).map_err(|e| e.to_string())?.len() != 3_803_452_480 {
        return Err("Acquire and verify the complete Q1_0 model first.".into());
    }
    std::fs::create_dir_all(&evidence).map_err(|e| e.to_string())?;
    let mut runtime = Runtime::new(evidence.join(format!("{prefix}-runtime.log")));
    let preferences = Preferences {
        runtime_path: runtime_dir
            .join("llama-server.exe")
            .to_string_lossy()
            .into_owned(),
        model_path: model.to_string_lossy().into_owned(),
        ..Default::default()
    };
    let config = if std::env::var("BONSAI_ACCEPTANCE_PRESET").as_deref() == Ok("1") {
        let catalog: Value = serde_json::from_str(include_str!("../../../catalog/bonsai-27b.json"))
            .map_err(|e| e.to_string())?;
        serde_json::from_value::<RuntimeConfig>(catalog["preset"].clone())
            .map_err(|e| e.to_string())?
    } else {
        RuntimeConfig {
            context_length: 4096,
            gpu_layers: 32,
            cpu_threads: 6,
            batch_size: 128,
            micro_batch_size: 64,
            flash_attention: true,
            cache_type_k: CacheType::F16,
            cache_type_v: CacheType::F16,
            ..Default::default()
        }
    };
    let started = Instant::now();
    let status = runtime.load(&preferences, &config).await?;
    println!(
        "Managed load {} ms: {}",
        started.elapsed().as_millis(),
        serde_json::to_string(&status).unwrap()
    );
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(300))
        .build()
        .map_err(|e| e.to_string())?;
    let props: Value = client
        .get(format!("{}/props", runtime.endpoint))
        .bearer_auth(&runtime.api_key)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    // No external chat template is passed; preserve the runtime's report of the embedded template.
    std::fs::write(
        evidence.join(format!("{prefix}-props.json")),
        serde_json::to_vec_pretty(&props).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let prompts = [
        "What is the capital of France? Answer briefly.",
        "What is the chemical formula of water? Answer briefly.",
        "Write a short Python function sum_even(numbers) that returns the sum of even integers.",
        "Calculate 17 + 25. Give the number only.",
        "Factor 84 into prime factors. Answer briefly.",
        "Alice is taller than Bob. Bob is taller than Carol. Who is the shortest? Answer briefly.",
        "Responde en español: ¿Cuál es la capital de España?",
        "हिंदी में संक्षेप में बताइए: पानी का रासायनिक सूत्र क्या है?",
        "Return JSON only with a single key answer and integer value 42.",
    ];
    let mut records = Vec::new();
    for prompt in prompts {
        let payload = json!({"messages":[{"role":"user","content":prompt}],"stream":false,
            "temperature":0.7,"top_p":0.95,"top_k":20,"max_tokens":512,
            "chat_template_kwargs":{"enable_thinking":false}});
        let tokens =
            crate::context::count_only(&client, &runtime.endpoint, &runtime.api_key, &payload)
                .await?;
        crate::context::validate_budget(tokens, 512, runtime.context_length)?;
        let response = client
            .post(format!("{}/v1/chat/completions", runtime.endpoint))
            .bearer_auth(&runtime.api_key)
            .json(&payload)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let http_status = response.status();
        let body: Value = response.json().await.map_err(|e| e.to_string())?;
        println!("Prompt: {prompt}\nHTTP {http_status}: {}", body["choices"]);
        records.push(json!({"prompt":prompt,"inputTokens":tokens,"httpStatus":http_status.as_u16(),"response":body}));
        std::fs::write(
            evidence.join(format!("{prefix}-chat.json")),
            serde_json::to_vec_pretty(&records).unwrap(),
        )
        .map_err(|e| e.to_string())?;
        if !http_status.is_success() {
            runtime.stop().await?;
            return Err(format!("Chat failed: HTTP {http_status}"));
        }
    }
    let payload = json!({"messages":[{"role":"user","content":"Remember the code JUNIPER-4721 and say OK."},
        {"role":"assistant","content":"OK"},{"role":"user","content":"What code did I ask you to remember? Give the code only."}],
        "stream":false,"max_tokens":128,"chat_template_kwargs":{"enable_thinking":false}});
    let response: Value = client
        .post(format!("{}/v1/chat/completions", runtime.endpoint))
        .bearer_auth(&runtime.api_key)
        .json(&payload)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    println!("Multi-turn: {}", response["choices"]);
    records.push(json!({"prompt":"multi-turn code recall","response":response}));
    std::fs::write(
        evidence.join(format!("{prefix}-chat.json")),
        serde_json::to_vec_pretty(&records).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let thinking = json!({"messages":[{"role":"user","content":"Calculate 17 + 25 and explain briefly."}],
        "stream":false,"max_tokens":2048,"chat_template_kwargs":{"enable_thinking":true}});
    let response: Value = client
        .post(format!("{}/v1/chat/completions", runtime.endpoint))
        .bearer_auth(&runtime.api_key)
        .json(&thinking)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    println!("Thinking: {}", response["choices"]);
    std::fs::write(
        evidence.join(format!("{prefix}-thinking.json")),
        serde_json::to_vec_pretty(&response).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let definition = crate::system_tools::system_time_tool_definition();
    let tool_payload = json!({"messages":[{"role":"user","content":"Use system_time to read the current time in Asia/Kolkata, then tell me the date. Do not guess the time."}],
        "tools":[{"type":"function","function":{"name":definition.name,"description":definition.description,"parameters":definition.input_schema}}],
        "tool_choice":"auto","stream":false,"max_tokens":512,"chat_template_kwargs":{"enable_thinking":false}});
    let response: Value = client
        .post(format!("{}/v1/chat/completions", runtime.endpoint))
        .bearer_auth(&runtime.api_key)
        .json(&tool_payload)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    println!("Tool request: {}", response["choices"]);
    let mut tool_record = json!({"request":tool_payload,"response":response});
    let message = &tool_record["response"]["choices"][0]["message"];
    if let Some(calls) = message["tool_calls"]
        .as_array()
        .filter(|calls| calls.len() == 1)
    {
        let call = &calls[0];
        if call["function"]["name"] == "system_time" {
            if let Some(arguments) = call["function"]["arguments"]
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
            {
                let result = crate::system_tools::execute_system_time(&arguments)?;
                let mut continuation = tool_record["request"].clone();
                let messages = continuation["messages"].as_array_mut().unwrap();
                messages.push(message.clone());
                messages.push(json!({"role":"tool","tool_call_id":call["id"],"content":serde_json::to_string(&result).unwrap()}));
                let followup: Value = client
                    .post(format!("{}/v1/chat/completions", runtime.endpoint))
                    .bearer_auth(&runtime.api_key)
                    .json(&continuation)
                    .send()
                    .await
                    .map_err(|e| e.to_string())?
                    .json()
                    .await
                    .map_err(|e| e.to_string())?;
                println!("Tool continuation: {}", followup["choices"]);
                tool_record["result"] = result;
                tool_record["continuation"] = followup;
            }
        }
    }
    std::fs::write(
        evidence.join(format!("{prefix}-tool.json")),
        serde_json::to_vec_pretty(&tool_record).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    runtime.stop().await?;
    Ok(())
}
