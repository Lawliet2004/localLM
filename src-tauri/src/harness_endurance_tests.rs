//! Opt-in, real elapsed-time acceptance. Uses the production registered turn,
//! SQLite store and file tools with a paced loopback provider, never an account.
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn request(socket: &mut tokio::net::TcpStream) -> Value {
    let mut bytes = Vec::new();
    loop {
        let mut buffer = [0; 8192];
        let n = tokio::time::timeout(Duration::from_secs(30), socket.read(&mut buffer))
            .await
            .unwrap()
            .unwrap();
        assert!(n > 0, "provider request ended before its body");
        bytes.extend_from_slice(&buffer[..n]);
        assert!(
            bytes.len() <= 4 * 1024 * 1024,
            "fixture request exceeds 4 MiB"
        );
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            assert!(end <= 8192);
            let headers = std::str::from_utf8(&bytes[..end]).unwrap();
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
                return serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
            }
        }
    }
}

fn resident_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        line.strip_prefix("VmRSS:")?
            .split_whitespace()
            .next()?
            .parse::<u64>()
            .ok()
            .map(|kb| kb * 1024)
    })
}

#[tokio::test]
#[ignore = "Real elapsed-time soak: LOCALLM_SOAK_SECONDS=3600 (2–7200), LOCALLM_SOAK_READS=24 (24–60). No real model or browser."]
async fn wall_clock_endurance() {
    let seconds = std::env::var("LOCALLM_SOAK_SECONDS")
        .ok()
        .map(|v| v.parse::<u64>().unwrap())
        .unwrap_or(3600);
    let reads = std::env::var("LOCALLM_SOAK_READS")
        .ok()
        .map(|v| v.parse::<usize>().unwrap())
        .unwrap_or(24);
    assert!((2..=7200).contains(&seconds));
    assert!((24..=60).contains(&reads));
    let root = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(root.path());
    crate::test_support::select_fixture_model(&state, root.path());
    std::fs::write(
        root.path().join("AGENTS.md"),
        "Never modify soak-protected.txt. Retain SOAK_RULE_SENTINEL.",
    )
    .unwrap();
    std::fs::write(root.path().join("soak-protected.txt"), "immutable").unwrap();
    for index in 0..reads {
        let content = (0..160)
            .map(|line| format!("file {index}, line {line}: {}\n", "evidence ".repeat(8)))
            .collect::<String>();
        std::fs::write(root.path().join(format!("evidence-{index}.txt")), content).unwrap();
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/v1", listener.local_addr().unwrap());
    let owner = {
        let store = state.database().unwrap();
        let mut provider = store.provider("fixture-loopback").unwrap().unwrap();
        provider.base_url = base;
        provider.models[0].context_length = Some(16384);
        provider.models[0].max_output_tokens = Some(1024);
        store.save_provider(&provider).unwrap();
        let owner = store.create_conversation().unwrap().id;
        store
            .save_conversation_preset(&owner, crate::presets::CODE)
            .unwrap();
        store
            .save_conversation_tools(
                &owner,
                &crate::store::ConversationTools {
                    access_mode: crate::permissions::AccessMode::FullAccess,
                    sources: vec!["__workspace".into()],
                    tools: vec![],
                },
            )
            .unwrap();
        owner
    };
    let seen = Arc::new(Mutex::new(Vec::new()));
    let records = seen.clone();
    let start = Instant::now();
    let rss_start = resident_bytes();
    let server = tokio::spawn(async move {
        for index in 0..=reads {
            let (mut socket, _) = tokio::time::timeout(Duration::from_secs(90), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let body = request(&mut socket).await;
            assert!(
                !body.to_string().contains(crate::compaction::HANDOFF_FIELD),
                "internal metadata reached the provider"
            );
            records.lock().unwrap().push(body);
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").await.unwrap();
            let target = start
                + Duration::from_secs_f64(seconds as f64 * (index + 1) as f64 / (reads + 1) as f64);
            while Instant::now() < target {
                let wait = target
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(30));
                tokio::time::sleep(wait).await;
                socket.write_all(b": fixture-heartbeat\n\n").await.unwrap();
                println!(
                    "SOAK_PROGRESS {}",
                    json!({"elapsedSeconds":start.elapsed().as_secs_f64(),"request":index+1,"requests":reads+1,"settledPriorReads":index,"residentBytes":resident_bytes()})
                );
            }
            let delta = if index < reads {
                let context = (0..64)
                    .map(|part| {
                        use sha2::Digest;
                        format!(
                            "{:x}\n",
                            sha2::Sha256::digest(format!("fixture-context-{index}-{part}"))
                        )
                    })
                    .collect::<String>();
                json!({"content":format!("Unverified fixture context references for evidence-{index}:\n{context}"),"tool_calls":[{"index":0,"id":format!("soak-read-{index}"),"type":"function","function":{
                    "name":"workspace_read_file","arguments":json!({"path":format!("evidence-{index}.txt")}).to_string()
                }}]})
            } else {
                json!({"content":"Completed the paced read-only fixture; protected content retained."})
            };
            let reason = if index < reads { "tool_calls" } else { "stop" };
            let wire = format!(
                "data: {}\r\n\r\ndata: [DONE]\r\n\r\n",
                json!({"choices":[{"delta":delta,"finish_reason":reason}]})
            );
            socket.write_all(wire.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
    });
    println!(
        "SOAK_START {}",
        json!({"seconds":seconds,"reads":reads,"provider":"scripted-loopback","residentBytes":rss_start})
    );
    tokio::time::timeout(Duration::from_secs(seconds + 180), crate::chat::send_registered_message(
        &state, owner.clone(), format!("Read all {reads} evidence files. Never modify soak-protected.txt. Retain SOAK_REQUEST_SENTINEL throughout the task."),
        Some(vec!["__workspace".into()]), Some(vec![]), None, None, Channel::new(|_| Ok(())), None,
    )).await.expect("soak exceeded its absolute deadline").unwrap();
    server.await.unwrap();
    assert!(
        start.elapsed() >= Duration::from_secs(seconds),
        "clock was accelerated instead of actually waiting"
    );
    assert_eq!(seen.lock().unwrap().len(), reads + 1);
    assert_eq!(state.session_runs.active_count(), 0);
    assert_eq!(
        std::fs::read_to_string(root.path().join("soak-protected.txt")).unwrap(),
        "immutable"
    );
    let (message_id, run_id, archives) = {
        let store = state.database().unwrap();
        let messages = store.messages(&owner).unwrap();
        assert_eq!(messages.iter().filter(|m| m.role == "tool").count(), reads);
        for (index, message) in messages.iter().filter(|m| m.role == "tool").enumerate() {
            let audit: Value = serde_json::from_str(&message.content).unwrap();
            let result = &audit["result"];
            assert!(
                !crate::tool_envelope::failed(result) && result["isError"] != true,
                "read {index} failed: {result}"
            );
            assert!(
                result
                    .to_string()
                    .contains(&format!("file {index}, line 0")),
                "read {index} did not return its actual file data: {result}"
            );
        }
        let answer = messages.iter().find(|m| m.role == "assistant").unwrap();
        assert_eq!(answer.status, "complete");
        assert!(answer.content.contains("protected content retained"));
        assert_eq!(
            store.latest_research_task(&owner).unwrap().unwrap().status,
            crate::research_tasks::TaskStatus::Completed
        );
        let events = store.session_events(&owner, None, Some(5000)).unwrap();
        let end = events.iter().rev().find(|e| e.event_type == "turn_end").unwrap();
        assert_eq!(end.payload["partial"], false);
        assert_eq!(end.payload["status"], "complete");
        let run_id = end.run_id.as_ref().unwrap();
        assert_eq!(store.run(run_id).unwrap().unwrap().status, crate::agent_run::RunState::Completed);
        let compactions: Vec<_> = events.iter().filter(|event| event.event_type == "compaction").collect();
        {
            let requests = seen.lock().unwrap();
            println!(
                "SOAK_CONTEXT {}",
                json!({"firstMessageBytes":requests.first().unwrap()["messages"].to_string().len(),"lastMessageBytes":requests.last().unwrap()["messages"].to_string().len(),"maxCompactedInputTokens":compactions.iter().filter_map(|event| event.payload["inputTokens"].as_u64()).max()})
            );
        }
        // Active-turn compaction has no user-boundary cutoff yet. Its durable
        // publication is the event/artifact plus the exact canonical buffer.
        let archives: Vec<String> = compactions.iter()
            .map(|event| event.payload["artifact"].as_str().unwrap().into()).collect();
        let archive_id = archives.last().expect("large evidence should exercise automatic compaction");
        let archive = store.artifact(archive_id).unwrap().unwrap();
        assert!(archive.content.contains("SOAK_REQUEST_SENTINEL"));
        assert!(seen.lock().unwrap().last().unwrap()["messages"]
            .to_string()
            .contains("soak-protected.txt"));
        (answer.id.clone(), run_id.clone(), archives)
    };
    let db = state.db_path.clone();
    drop(state);
    let store = crate::store::Store::open(&db).unwrap();
    store.recover_interrupted().unwrap();
    crate::research_tasks::recover_interrupted(&store).unwrap();
    assert_eq!(
        store.latest_research_task(&owner).unwrap().unwrap().status,
        crate::research_tasks::TaskStatus::Completed
    );
    assert!(store
        .messages(&owner)
        .unwrap()
        .iter()
        .any(|m| m.id == message_id && m.status == "complete"));
    assert_eq!(store.run(&run_id).unwrap().unwrap().status, crate::agent_run::RunState::Completed);
    assert!(archives.iter().all(|id| store.artifact(id).unwrap().is_some()));
    let canonical = store.cached_model_history(&owner).unwrap().expect("canonical compacted history must survive reopen");
    assert!(serde_json::to_string(&canonical).unwrap().contains(archives.last().unwrap()));
    println!(
        "SOAK_RESULT {}",
        json!({"elapsedSeconds":start.elapsed().as_secs_f64(),"requests":reads+1,"realFileReads":reads,"compactions":archives.len(),"reopenPreservedCompletion":true,"runStatus":"completed","ledgerStatus":"completed","turnStatus":"complete","partial":false,"residentStartBytes":rss_start,"residentEndBytes":resident_bytes(),"scope":"One registered read-only turn with paced scripted inference."})
    );
}
