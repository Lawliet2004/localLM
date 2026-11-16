//! Opt-in entry point for externally graded, real-model harness comparisons.
//! The runner supplies an owned workspace and a loopback-only inference URL.
use serde_json::{json, Value};
use tauri::ipc::Channel;

#[tokio::test]
#[ignore = "Requires LOCALLM_COMPARISON_CONFIG, an owned fixture workspace and a real loopback model. No provider account."]
async fn registered_local_model_comparison() {
    let config_path = std::env::var("LOCALLM_COMPARISON_CONFIG").expect("explicit fixture configuration");
    let config: Value = serde_json::from_slice(&std::fs::read(config_path).unwrap()).unwrap();
    let workspace = std::path::Path::new(config["workspace"].as_str().unwrap()).canonicalize().unwrap();
    let url = reqwest::Url::parse(config["baseUrl"].as_str().unwrap()).unwrap();
    assert_eq!(url.scheme(), "http");
    assert_eq!(url.host_str(), Some("127.0.0.1"));
    assert!(url.username().is_empty() && url.password().is_none());
    let temp = tempfile::tempdir().unwrap();
    let state = crate::test_support::app_state(temp.path());
    crate::test_support::select_fixture_model(&state, &workspace);
    let aliases = [
        "workspace_read_file",
        "workspace_glob_files",
        "workspace_search_content",
        "workspace_create_file",
        "workspace_edit_file",
    ];
    let owner = {
        let store = state.database().unwrap();
        let mut provider = store.provider("fixture-loopback").unwrap().unwrap();
        provider.base_url = url.to_string().trim_end_matches('/').into();
        provider.models[0].id = config["modelId"].as_str().unwrap().into();
        provider.models[0].context_length = Some(32768);
        provider.models[0].max_output_tokens = Some(1024);
        store.save_provider(&provider).unwrap();
        store.save_preferred_model(&crate::providers::ModelSelection { provider_id: Some(provider.id), model_id: provider.models[0].id.clone() }).unwrap();
        let mut preferences = store.preferences().unwrap();
        preferences.temperature = 0.0;
        preferences.top_p = 1.0;
        preferences.max_tokens = 1024;
        preferences.reasoning_mode = crate::store::ReasoningMode::Quick;
        preferences.generative_ui = false;
        store.save_preferences(&preferences).unwrap();
        store.save_setting("offline_mode", &true).unwrap();
        store.set_presentation_mode(None, Some(crate::presentation::Mode::Text)).unwrap();
        use crate::permission_rules::{Action, Effect, Rule, RuleSet};
        let mut rules = vec![Rule { action: Action::Tool, resource: "*".into(), effect: Effect::Deny }];
        rules.extend(aliases.iter().map(|alias| Rule {
            action: Action::Tool,
            resource: (*alias).into(),
            effect: Effect::Allow,
        }));
        crate::permission_rules::save(&store, &RuleSet { rules }).unwrap();
        let owner = store.create_conversation().unwrap().id;
        store.save_conversation_preset(&owner, crate::presets::CHAT).unwrap();
        store.save_conversation_tools(&owner, &crate::store::ConversationTools {
            access_mode: crate::permissions::AccessMode::FullAccess,
            sources: vec!["__workspace".into()], tools: vec![],
        }).unwrap();
        owner
    };
    let started = std::time::Instant::now();
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(240), crate::chat::send_registered_message(
        &state, owner.clone(), config["prompt"].as_str().unwrap().into(),
        Some(vec!["__workspace".into()]), Some(vec![]), None, None,
        Channel::new(|_| Ok(())), None,
    )).await;
    let error = match outcome { Ok(Ok(())) => None, Ok(Err(error)) => Some(error), Err(_) => Some("Comparison exceeded 240 seconds".into()) };
    let store = state.database().unwrap();
    let messages = store.messages(&owner).unwrap();
    let events = store.session_events(&owner, None, Some(5000)).unwrap();
    let history = store.cached_model_history(&owner).unwrap();
    let tool_audits: Vec<Value> = messages.iter().filter(|message| message.role == "tool").map(|message| {
        let audit: Value = serde_json::from_str(&message.content).unwrap();
        let result = &audit["result"];
        json!({"status":message.status,"request":audit.get("request").unwrap_or(&audit),
            "errorCode":result["error"]["code"],"failed":crate::tool_envelope::failed(result)})
    }).collect();
    let result = json!({
        "schemaVersion":"locallm.harness-comparison-result/2", "harness":"LocalLM",
        "elapsedMs":started.elapsed().as_secs_f64()*1000.0, "error":error,
        "messages":messages, "modelHistory":history, "allowedTools":aliases, "toolAudits":tool_audits,
        "recognizedCalls":events.iter().filter(|event|event.event_type=="tool_call").count(),
        "turnEnd":events.iter().rev().find(|event|event.event_type=="turn_end").map(|event|&event.payload),
    });
    std::fs::write(config["output"].as_str().unwrap(), serde_json::to_vec_pretty(&result).unwrap()).unwrap();
}
