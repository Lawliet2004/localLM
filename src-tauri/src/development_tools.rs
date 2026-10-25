//! Shared registration and immutable evidence for development workflows.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn schema(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
pub fn save(db: &Path, owner: &str, run: &str, kind: &str, data: &Value) -> Result<String, String> {
    let content = serde_json::to_string(data).map_err(|e| e.to_string())?;
    if content.len() > 8 * 1024 * 1024 {
        return Err("Development evidence exceeds 8 MiB.".into());
    }
    let id = format!("dev_{}", uuid::Uuid::new_v4().simple());
    let artifact = crate::artifacts::ArtifactRecord {
        id: id.clone(),
        conversation_id: owner.into(),
        run_id: Some(run.into()),
        tool_name: kind.into(),
        mime_type: "application/json".into(),
        size_bytes: content.len(),
        sha256: hash(content.as_bytes()),
        content,
        created_at: crate::store::now(),
    };
    crate::store::Store::open(db)?.save_artifact(&artifact)?;
    Ok(id)
}
pub fn load(db: &Path, owner: &str, root: &str, id: &str, kind: &str) -> Result<Value, String> {
    let a = crate::store::Store::open(db)?
        .artifact(id)?
        .ok_or("Unknown evidence artifact.")?;
    if a.conversation_id != owner || a.tool_name != kind || hash(a.content.as_bytes()) != a.sha256 {
        return Err("Evidence ownership/type/integrity mismatch.".into());
    }
    let v: Value = serde_json::from_str(&a.content).map_err(|e| e.to_string())?;
    if v["root"] != canonical(root)? {
        return Err("Evidence belongs to another workspace.".into());
    }
    Ok(v)
}
pub fn canonical(root: &str) -> Result<String, String> {
    if root.is_empty() {
        return Err("Choose a workspace.".into());
    }
    std::fs::canonicalize(root)
        .map(|p| p.to_string_lossy().into_owned())
        .map_err(|e| e.to_string())
}
pub fn snapshot(root: &str, paths: &[Value]) -> Result<Value, String> {
    if paths.is_empty() || paths.len() > 128 {
        return Err("Select 1–128 source files.".into());
    }
    let workspace = crate::workspace::Workspace::open(root)?;
    let mut files = serde_json::Map::new();
    let mut bytes = 0;
    for path in paths {
        let path = path.as_str().ok_or("Source paths must be strings.")?;
        if files.contains_key(path) {
            return Err("Duplicate source path.".into());
        }
        let text = workspace.language_source(path)?;
        bytes += text.len();
        if bytes > 8 * 1024 * 1024 {
            return Err("Source snapshot exceeds 8 MiB.".into());
        }
        files.insert(path.into(), json!(hash(text.as_bytes())));
    }
    Ok(
        json!({"files":files,"sha256":hash(serde_json::to_vec(&files).map_err(|e|e.to_string())?.as_slice()),"scope":"explicit_files_only"}),
    )
}
pub fn freshness(root: &str, snapshot: &Value) -> Value {
    let mut changed = Vec::new();
    let mut unavailable = Vec::new();
    match crate::workspace::Workspace::open(root) {
        Ok(workspace) => {
            for (p, sha) in snapshot["files"].as_object().into_iter().flatten() {
                match workspace.language_optional_source(p) {
                    Ok(t)
                        if t.as_ref()
                            .map(|s| json!(hash(s.as_bytes())))
                            .unwrap_or(Value::Null)
                            == *sha => {}
                    Ok(_) => changed.push(p.clone()),
                    Err(_) => unavailable.push(p.clone()),
                }
            }
        }
        Err(_) => unavailable.push("workspace unavailable".into()),
    }
    json!({"matches":changed.is_empty()&&unavailable.is_empty(),"changed":changed,"unavailable":unavailable,"scope":"explicit_files_only","limitation":"Hashes compare observation endpoints; they do not isolate concurrent external writes or cover unselected dependencies."})
}
pub fn registry() -> Vec<crate::tool_spec::ToolSpec> {
    let mut tools: Vec<crate::tool_spec::ToolSpec> = crate::verification_tools::registry();
    tools.extend(crate::impact_tools::registry());
    tools.extend(crate::api_tools::registry());
    tools.extend(crate::database_tools::registry());
    tools.extend(crate::performance_tools::registry());
    tools
}
pub fn owns(alias: &str) -> bool {
    matches!(
        alias,
        "verification_run"
            | "verification_report"
            | "verification_compare"
            | "change_impact"
            | "http_session_create"
            | "http_session_close"
            | "http_request"
            | "http_compare"
            | "openapi_inspect"
            | "openapi_request"
            | "database_schema"
            | "database_query"
            | "database_explain"
            | "database_rehearse"
            | "database_cleanup"
            | "benchmark_run"
            | "benchmark_compare"
            | "profile_inspect"
            | "profile_capture"
            | "profile_compare"
    )
}
pub fn read_only(alias: &str) -> bool {
    matches!(
        alias,
        "verification_report"
            | "verification_compare"
            | "change_impact"
            | "openapi_inspect"
            | "database_schema"
            | "database_query"
            | "database_explain"
            | "benchmark_compare"
            | "http_compare"
            | "profile_inspect"
            | "profile_compare"
    )
}
pub fn execution_tool(alias: &str) -> bool {
    matches!(
        alias,
        "verification_run" | "benchmark_run" | "profile_capture" | "database_rehearse"
    )
}
pub async fn execute(
    ctx: &crate::harness::HarnessCtx<'_>,
    alias: &str,
    args: Value,
) -> Result<Value, String> {
    crate::tool_validation::validate(
        &registry()
            .into_iter()
            .find(|x| x.name == alias)
            .ok_or("Unknown development tool.")?
            .input,
        &args,
    )?;
    if execution_tool(alias) {
        let config = ctx.state.database()?.execution_config()?;
        if !config.bash.enabled {
            return Err("Shell execution is disabled.".into());
        }
        if config.bash.always_ask
            && !ctx
                .inherit_tools
                .iter()
                .any(|t| t.alias == alias && t.tool.input_schema["x-locallm-always-ask"] == true)
        {
            return Err("Shell approval policy changed; refresh tools before executing.".into());
        }
    }
    match alias {
        "verification_run" | "verification_report" | "verification_compare" => {
            crate::verification_tools::execute(ctx, alias, args).await
        }
        "http_session_create"
        | "http_session_close"
        | "http_request"
        | "http_compare"
        | "openapi_inspect"
        | "openapi_request" => ctx.state.http_sessions.execute(ctx, alias, args).await,
        "profile_capture" => crate::profile_tools::capture(ctx, args).await,
        "profile_compare" => crate::profile_tools::compare(ctx, args).await,
        "benchmark_run" | "benchmark_compare" => {
            crate::performance_tools::execute(ctx, alias, args).await
        }
        "change_impact" => {
            if args["mode"] == "lexical" {
                return crate::impact_tools::execute(&ctx.snapshot.workspace_path, args);
            }
            let graph = crate::semantic_impact::analyze(ctx, &args).await?;
            if args["mode"].is_null() || args["mode"] == "auto" {
                let mut graph = graph;
                if args["symbols"].is_array() {
                    graph["lexicalFallback"] =
                        crate::impact_tools::execute(&ctx.snapshot.workspace_path, args)?;
                }
                Ok(graph)
            } else {
                Ok(graph)
            }
        }
        "database_cleanup" => crate::database_rehearsal::cleanup_tool(ctx, args).await,
        name
        @ ("database_schema" | "database_query" | "database_explain" | "database_rehearse")
            if args["profile_id"].is_string() =>
        {
            crate::remote_database::execute(ctx, name, args).await
        }
        "database_schema" | "database_query" | "database_explain" | "database_rehearse"
        | "profile_inspect" => {
            let root = ctx.snapshot.workspace_path.clone();
            let alias = alias.to_string();
            tokio::task::spawn_blocking(move || match alias.as_str() {
                "profile_inspect" => crate::performance_tools::inspect(&root, args),
                _ => crate::database_tools::execute(&root, &alias, args),
            })
            .await
            .map_err(|_| {
                "Development worker failed; inspect evidence before retrying.".to_string()
            })?
        }
        _ => Err("Unknown development tool.".into()),
    }
}

/// Expose native-configured opaque references without exposing vault contents.
pub fn configure_tool(
    tool: &mut crate::connectors::AgentTool,
    store: &crate::store::Store,
) -> Result<(), String> {
    if !owns(&tool.alias) {
        return Ok(());
    }
    if let Some(property) = tool.tool.input_schema["properties"].get_mut("profile_id") {
        let profiles: Vec<crate::remote_database::Profile> =
            store.setting("development.database_profiles")?;
        property["description"] = json!(format!(
            "Native database profiles: {}",
            serde_json::to_string(
                &profiles
                    .iter()
                    .map(|p| json!({"id":p.id,"name":p.name,"engine":p.engine}))
                    .collect::<Vec<_>>()
            )
            .map_err(|e| e.to_string())?
        ));
    }
    if let Some(property) = tool.tool.input_schema["properties"].get_mut("credential_id") {
        let items: Vec<crate::development_credentials::HttpCredential> =
            store.setting("development.http_credentials")?;
        property["description"] = json!(format!(
            "Native origin-bound credential references: {}",
            serde_json::to_string(&items).map_err(|e| e.to_string())?
        ));
    }
    Ok(())
}

/// Fingerprint configured runtime bytes as well as paths; in-place upgrades
/// invalidate evidence even when the saved execution settings did not change.
pub fn runtime_identity(config: &crate::execution::ExecutionConfig) -> Value {
    let mut runtimes = serde_json::Map::new();
    for (name, path) in [
        ("python", &config.python_path),
        ("node", &config.node_path),
        ("powershell", &config.powershell_path),
        ("shell", &config.bash_path),
    ] {
        let result = executable_identity_result(path);
        runtimes.insert(
            name.into(),
            match result {
                Ok(value) => value,
                Err(_) if path.is_empty() => Value::Null,
                Err(_) => json!({"path":path,"unavailable":true}),
            },
        );
    }
    json!({"complete":!runtimes.values().any(|v|v["unavailable"]==true),"applicationVersion":env!("CARGO_PKG_VERSION"),"executables":runtimes})
}

fn executable_identity_result(path: &str) -> Result<Value, String> {
    use std::io::Read;

    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let before = file.metadata().map_err(|e| e.to_string())?;
    if before.len() > 128 * 1024 * 1024 {
        return Err("executable_exceeds_fingerprint_budget".into());
    }
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 65536];
    let mut total = 0;
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        total += n;
        if total > 128 * 1024 * 1024 {
            return Err("executable_changed_or_exceeds_budget".into());
        }
        hash.update(&buffer[..n]);
    }
    let after = file.metadata().map_err(|e| e.to_string())?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err("executable_changed_during_fingerprinting".into());
    }
    Ok(json!({"path":path,"bytes":total,"sha256":format!("{:x}",hash.finalize())}))
}
pub fn executable_identity(path: &str) -> Value {
    executable_identity_result(path).unwrap_or_else(|_| json!({"path":path,"unavailable":true}))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn context<'a>(
        state: &'a crate::AppState,
        root: &Path,
        owner: &str,
        tools: &'a [crate::connectors::AgentTool],
    ) -> crate::harness::HarnessCtx<'a> {
        let aliases = tools.iter().map(|t| t.alias.as_str()).collect::<Vec<_>>();
        crate::harness::HarnessCtx {
            state,
            snapshot: crate::test_support::snapshot(state, root, &aliases),
            conversation_id: owner.into(),
            run_id: "development-fixture".into(),
            step_id: "development-step".into(),
            access_mode: crate::permissions::AccessMode::FullAccess,
            depth: 0,
            preset_id: crate::presets::CODING.into(),
            inherit_tools: tools,
            emit: std::sync::Arc::new(|_, _| {}),
        }
    }
    fn setup(root: &Path) -> (crate::AppState, String, Vec<crate::connectors::AgentTool>) {
        let state = crate::test_support::app_state(root);
        let owner = state.database().unwrap().create_conversation().unwrap().id;
        let tools = registry()
            .iter()
            .map(|t| crate::connectors::AgentTool::harness(&t.name).unwrap())
            .collect();
        (state, owner, tools)
    }
    #[test]
    fn new_tools_have_strict_registered_schemas_sources_and_presets() {
        let mut tools = registry();
        tools.extend(crate::debug_tools::registry());
        for spec in tools {
            let alias = spec.name;
            let schema = spec.input;
            assert_eq!(schema["additionalProperties"], false, "{alias}");
            crate::tool_validation::compile(&schema).unwrap();
            assert!(crate::harness::definition(&alias).is_some());
            assert!(crate::capabilities::harness_alias_capability(&alias).is_some());
            assert!(
                !crate::presets::harness_source_allowed(&alias, false, false),
                "{alias}"
            );
            for preset in [
                crate::presets::STANDARD,
                crate::presets::CODE,
                crate::presets::CODING,
                crate::presets::CREATOR,
            ] {
                assert!(
                    crate::presets::get(preset)
                        .unwrap()
                        .harness
                        .contains(&alias),
                    "{preset}: {alias}"
                );
            }
            for preset in [crate::presets::MINIMAL, crate::presets::CHAT] {
                assert!(!crate::presets::get(preset)
                    .unwrap()
                    .harness
                    .contains(&alias));
            }
        }
    }
    #[tokio::test]
    async fn observations_finish_without_report_polling_and_remain_historical() {
        for change_during in [false, true] {
            let t = tempfile::tempdir().unwrap();
            let (state, owner, tools) = setup(t.path());
            std::fs::write(t.path().join("main.rs"), "original").unwrap();
            let ctx = context(&state, t.path(), &owner, &tools);
            let command = if cfg!(windows) {
                "Start-Sleep -Milliseconds 400; Write-Output 'TAP version 13'; Write-Output 'ok 1 - one'; Write-Output '1..1'"
            } else {
                "sleep 0.4; printf 'TAP version 13\nok 1 - one\n1..1\n'"
            };
            let launched = crate::harness::execute(
                &ctx,
                "verification_run",
                json!({"command":command,"source_paths":["main.rs"],"yield_ms":0}),
            )
            .await
            .unwrap()
            .value;
            if change_during {
                std::fs::write(t.path().join("main.rs"), "modified").unwrap();
                std::fs::write(t.path().join("main.rs"), "original").unwrap();
            }
            // Wait on immutable completion evidence itself, never verification_report.
            let evidence = load(
                &state.db_path,
                &owner,
                t.path().to_str().unwrap(),
                launched["evidenceId"].as_str().unwrap(),
                "verification_evidence",
            )
            .unwrap();
            let observation_id = evidence["observationId"].as_str().unwrap();
            let observation = tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    if let Ok(value) = load(
                        &state.db_path,
                        &owner,
                        t.path().to_str().unwrap(),
                        observation_id,
                        "verification_observation",
                    ) {
                        break value;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                }
            })
            .await
            .unwrap();
            std::fs::write(t.path().join("main.rs"), "after completion").unwrap();
            let args = json!({"evidence_id":launched["evidenceId"]});
            let report = crate::harness::execute(&ctx, "verification_report", args.clone())
                .await
                .unwrap()
                .value;
            assert_eq!(
                report["changedDuringObservation"], change_during,
                "{report}"
            );
            assert_eq!(report["freshness"]["matches"], false);
            assert_eq!(report["observation"], observation);
            std::fs::write(t.path().join("main.rs"), "original").unwrap();
            let report = crate::harness::execute(&ctx, "verification_report", args)
                .await
                .unwrap()
                .value;
            assert_eq!(report["observation"], observation);
            assert_eq!(
                report["verifiedCurrent"],
                !change_during && cfg!(target_os = "linux"),
                "{report}"
            );
        }
    }
    #[tokio::test]
    async fn source_bound_checks_become_stale_and_enforce_ownership() {
        let t = tempfile::tempdir().unwrap();
        let (state, owner, tools) = setup(t.path());
        std::fs::write(t.path().join("main.rs"), "fn main(){}\n").unwrap();
        let ctx = context(&state, t.path(), &owner, &tools);
        let command = if cfg!(windows) {
            "Write-Output 'running 1 test'; Write-Output 'test one ... ok'; Write-Output 'test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s'"
        } else {
            "printf 'running 1 test\ntest one ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n'"
        };
        let result = crate::harness::execute(
            &ctx,
            "verification_run",
            json!({"command":command,"source_paths":["main.rs"],"format":"cargo"}),
        )
        .await
        .unwrap()
        .value;
        let args = json!({"evidence_id":result["evidenceId"],"wait_ms":100});
        let mut report = Value::Null;
        for _ in 0..20 {
            report = crate::harness::execute(&ctx, "verification_report", args.clone())
                .await
                .unwrap()
                .value;
            if report["check"]["report"]["verdict"] != "running" {
                break;
            }
        }
        assert_eq!(
            report["verifiedCurrent"],
            cfg!(target_os = "linux"),
            "{report}"
        );
        std::fs::write(t.path().join("main.rs"), "fn changed(){}\n").unwrap();
        let stale = crate::harness::execute(&ctx, "verification_report", args.clone())
            .await
            .unwrap()
            .value;
        assert_eq!(stale["verifiedCurrent"], false);
        assert_eq!(stale["freshness"]["changed"][0], "main.rs");
        let compared = crate::harness::execute(
            &ctx,
            "verification_compare",
            json!({"before_id":report["reportId"],"after_id":stale["reportId"]}),
        )
        .await
        .unwrap()
        .value;
        assert_eq!(compared["after"]["verifiedAtObservation"], false);
        let foreign = context(&state, t.path(), "foreign-owner", &tools);
        assert!(
            crate::harness::execute(&foreign, "verification_report", args)
                .await
                .is_err()
        );
        state
            .database()
            .unwrap()
            .save_setting(crate::capabilities::DISABLED_KEY, &vec!["workspace_files"])
            .unwrap();
        assert!(crate::harness::execute(
            &ctx,
            "change_impact",
            json!({"changed_paths":["main.rs"],"symbols":["changed"]})
        )
        .await
        .is_err());
    }
    #[tokio::test]
    async fn benchmark_records_successes_failures_and_environment_comparisons() {
        let t = tempfile::tempdir().unwrap();
        let (state, owner, tools) = setup(t.path());
        std::fs::write(t.path().join("source.txt"), "source").unwrap();
        let ctx = context(&state, t.path(), &owner, &tools);
        let result = crate::harness::execute(
            &ctx,
            "benchmark_run",
            json!({"command":"exit 0","source_paths":["source.txt"],"samples":2,"warmups":1}),
        )
        .await
        .unwrap()
        .value;
        assert_eq!(result["complete"], true, "{result}");
        assert_eq!(result["statistics"]["n"], 2);
        assert_eq!(result["samples"].as_array().unwrap().len(), 3);
        let failed = crate::harness::execute(
            &ctx,
            "benchmark_run",
            json!({"command":"exit 3","source_paths":["source.txt"],"samples":2,"warmups":0}),
        )
        .await
        .unwrap()
        .value;
        assert_eq!(failed["complete"], false);
        assert!(failed["statistics"].is_null());
        assert_eq!(failed["samples"][0]["exitCode"], 3);
        let comparison = crate::harness::execute(
            &ctx,
            "benchmark_compare",
            json!({"before_id":result["benchmarkId"],"after_id":failed["benchmarkId"]}),
        )
        .await
        .unwrap()
        .value;
        assert_eq!(comparison["causalSpeedupEstablished"], false);
        assert_eq!(comparison["sameCommand"], false);
    }
    #[test]
    fn bounded_native_evidence_handles_survive_and_mcp_cannot_spoof_them() {
        for (alias, key, prefix) in [
            ("verification_run", "evidenceId", "dev_"),
            ("verification_report", "reportId", "dev_"),
            ("benchmark_run", "benchmarkId", "dev_"),
            ("http_request", "responseId", "dev_"),
            ("http_session_create", "sessionId", "http-"),
            ("debugger_start", "sessionId", "debug-"),
        ] {
            let id = format!("{prefix}{}", uuid::Uuid::new_v4());
            let raw = json!({key:id,"large":"x".repeat(20000)});
            let normalized =
                crate::tool_envelope::normalize(raw.clone(), crate::tool_envelope::Family::Harness);
            let (bounded, saved) =
                crate::artifacts::bound_tool_result(normalized, alias, "owner", None, 512);
            assert!(saved.is_some());
            assert_eq!(bounded["data"][key], id, "{alias}: {bounded}");
            assert!(bounded.to_string().len() <= 512);
            let external = crate::tool_envelope::normalize(raw, crate::tool_envelope::Family::Mcp);
            let (bounded, _) =
                crate::artifacts::bound_tool_result(external, alias, "owner", None, 512);
            assert!(bounded.pointer(&format!("/data/{key}")).is_none());
        }
    }
    #[test]
    fn bounded_profile_keeps_capture_and_original_trace_handles() {
        let profile = format!("dev_{}", uuid::Uuid::new_v4().simple());
        let trace = format!("dev_{}", uuid::Uuid::new_v4().simple());
        for failed in [false, true] {
            let raw = json!({"profileId":profile,"originalTraceId":trace,"isError":failed,"large":"x".repeat(20000)});
            let normalized =
                crate::tool_envelope::normalize(raw, crate::tool_envelope::Family::Harness);
            let (bounded, _) = crate::artifacts::bound_tool_result(
                normalized,
                "profile_capture",
                "owner",
                None,
                512,
            );
            assert_eq!(bounded["data"]["profileId"], profile, "{bounded}");
            assert_eq!(bounded["data"]["originalTraceId"], trace, "{bounded}");
            assert!(bounded.to_string().len() <= 512);
        }
    }
    #[test]
    fn bounded_verification_preserves_multiple_continuation_handles() {
        for alias in ["verification_run", "verification_report"] {
            for failed in [false, true] {
                let process = format!("proc-{}", uuid::Uuid::new_v4());
                let evidence = format!("dev_{}", uuid::Uuid::new_v4().simple());
                let report = format!("dev_{}", uuid::Uuid::new_v4().simple());
                let mut raw = json!({"processId":process,"evidenceId":evidence,"large":"x".repeat(20000),"isError":failed});
                if alias == "verification_report" {
                    raw["reportId"] = json!(report);
                }
                let normalized =
                    crate::tool_envelope::normalize(raw, crate::tool_envelope::Family::Harness);
                let (bounded, _) =
                    crate::artifacts::bound_tool_result(normalized, alias, "owner", None, 512);
                assert_eq!(bounded["data"]["processId"], process, "{alias}: {bounded}");
                let key = if alias == "verification_run" {
                    "evidenceId"
                } else {
                    "reportId"
                };
                let id = if alias == "verification_run" {
                    &evidence
                } else {
                    &report
                };
                assert_eq!(bounded["data"][key], *id, "{alias}: {bounded}");
                assert!(bounded.to_string().len() <= 512);
                if failed {
                    assert!(crate::tool_envelope::failed(&bounded));
                }
            }
        }
    }
    #[tokio::test]
    async fn http_sessions_are_origin_owned_and_offline_is_enforced() {
        let t = tempfile::tempdir().unwrap();
        let (state, owner, tools) = setup(t.path());
        let ctx = context(&state, t.path(), &owner, &tools);
        // Origin ownership/offline enforcement never sends a request. Use a
        // public literal address so this regression does not depend on DNS.
        let session=crate::harness::execute(&ctx,"http_session_create",json!({"base_url":"https://93.184.216.34/api/","headers":{"Authorization":"Bearer fixture-secret"}})).await.unwrap().value;
        let id = session["sessionId"].clone();
        let foreign = context(&state, t.path(), "other", &tools);
        assert!(
            crate::harness::execute(&foreign, "http_session_close", json!({"session_id":id}))
                .await
                .is_err()
        );
        assert!(crate::harness::execute(
            &ctx,
            "http_request",
            json!({"session_id":id,"url":"https://other.invalid/"})
        )
        .await
        .unwrap_err()
        .contains("origin"));
        state
            .database()
            .unwrap()
            .save_setting("offline_mode", &true)
            .unwrap();
        assert!(crate::harness::execute(
            &ctx,
            "http_request",
            json!({"session_id":id,"url":"things"})
        )
        .await
        .unwrap_err()
        .contains("Offline"));
        crate::harness::execute(&ctx, "http_session_close", json!({"session_id":id}))
            .await
            .unwrap();
    }
}
