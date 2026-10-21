//! Test evidence explicitly bound to selected source snapshots.
use crate::development_tools as dev;
use serde_json::{json, Value};
pub fn registry() -> Vec<crate::tool_spec::ToolSpec> {
    let paths = json!({"type":"array","minItems":1,"maxItems":128,"items":{"type":"string","minLength":1,"maxLength":4096}});
    let id = json!({"type":"string","minLength":1,"maxLength":128});
    let mut run = crate::check_tools::registry()
        .into_iter()
        .find(|x| x.name == "check_run")
        .unwrap()
        .input
        .clone();
    run["properties"]["source_paths"] = paths;
    run["properties"]["mode"] =
        json!({"type":"string","enum":["explicit","affected","workspace"],"default":"explicit"});
    run["properties"]["server"] = json!({"type":"string","minLength":1,"maxLength":64});
    run["required"] = json!(["command"]);
    vec![("verification_run".into(),"Run checks against an explicit set of source files, saving immutable source hashes, command and execution-configuration fingerprint. Returns evidenceId and processId; inspect verification_report. Coverage excludes unselected files and dependencies.".into(),run),
    ("verification_report".into(),"Read an owned check and compare selected source hashes with its pre-run snapshot. verifiedCurrent requires a recognized passing suite and unchanged selected files. No command is rerun. Returns reportId for immutable comparisons.".into(),dev::schema(json!({"evidence_id":id,"wait_ms":{"type":"integer","minimum":0,"maximum":1000}}),&["evidence_id"])),
    ("verification_compare".into(),"Compare two immutable verification reports from this conversation/workspace. Reports test counts and freshness separately; does not imply equal test selection or complete coverage.".into(),dev::schema(json!({"before_id":id,"after_id":id}),&["before_id","after_id"]))].into_iter().map(crate::tool_spec::ToolSpec::from).collect()
}
pub async fn execute(
    ctx: &crate::harness::HarnessCtx<'_>,
    alias: &str,
    args: Value,
) -> Result<Value, String> {
    let owner = &ctx.conversation_id;
    let root = &ctx.snapshot.workspace_path;
    let db = &ctx.state.db_path;
    let run = &ctx.run_id;
    match alias {
        "verification_run" => {
            let mode = args["mode"].as_str().unwrap_or("explicit");
            let pending = crate::verification_observer::Registry::begin(root);
            let (snap, selection) = match mode {
                "explicit" => {
                    let snapshot = dev::snapshot(
                        root,
                        args["source_paths"]
                            .as_array()
                            .ok_or("Explicit verification requires source_paths.")?,
                    )?;
                    (snapshot, json!({"complete":true,"mode":"explicit"}))
                }
                "workspace" => {
                    let inventory = crate::source_inventory::inventory(root)?;
                    let snap = crate::source_inventory::snapshot(
                        root,
                        inventory["paths"]
                            .as_array()
                            .ok_or("Invalid workspace inventory.")?,
                        false,
                    )?;
                    (snap, inventory)
                }
                "affected" => {
                    let graph = crate::semantic_impact::analyze(
                        ctx,
                        &json!({"changed_paths":args["source_paths"],"server":args["server"]}),
                    )
                    .await?;
                    let mut paths = graph["affectedPaths"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                    let inventory = crate::source_inventory::inventory(root)?;
                    paths.extend(
                        inventory["paths"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|p| {
                                p.as_str().is_some_and(|p| {
                                    p.ends_with(".toml")
                                        || p.ends_with(".json")
                                        || p.ends_with(".lock")
                                        || p.ends_with(".yaml")
                                })
                            })
                            .cloned(),
                    );
                    paths.sort_by_key(Value::to_string);
                    paths.dedup();
                    let snap = crate::source_inventory::snapshot(root, &paths, true)?;
                    (
                        snap,
                        json!({"complete":graph["incomplete"]==false,"graph":graph}),
                    )
                }
                _ => return Err("Unknown verification selection mode.".into()),
            };
            let config = crate::check_tools::effective_config(
                ctx.state.database()?.execution_config()?,
                args["cwd"].as_str(),
            );
            let config_hash = dev::hash(
                serde_json::to_vec(&config)
                    .map_err(|e| e.to_string())?
                    .as_slice(),
            );
            let runtime_identity = dev::runtime_identity(&config);
            let mut call = args.clone();
            for key in ["source_paths", "mode", "server"] {
                call.as_object_mut().unwrap().remove(key);
            }
            let observation_id = format!("observation_{}", uuid::Uuid::new_v4().simple());
            let observation_available = pending.is_ok();
            let completion = pending
                .ok()
                .map(|pending| crate::verification_observer::Completion {
                    pending,
                    root: root.clone(),
                    sources: snap.clone(),
                    membership: if mode == "workspace" {
                        Some(selection.clone())
                    } else {
                        None
                    },
                    db: db.clone(),
                    owner: owner.clone(),
                    run: run.clone(),
                    id: observation_id.clone(),
                });
            let result = crate::check_tools::execute_observed(
                &ctx.state.processes,
                owner,
                root,
                config,
                "check_run",
                call,
                completion,
            )
            .await?;
            let evidence = json!({"root":dev::canonical(root)?,"sources":snap,"selection":selection,"mode":mode,"observationId":observation_id,"observationAvailable":observation_available,"runtimeIdentity":runtime_identity,"toolVersion":env!("CARGO_PKG_VERSION"),"command":args["command"],"cwd":args["cwd"],"format":args.get("format").cloned().unwrap_or(json!("auto")),"configurationSha256":config_hash,"processId":result["processId"],"createdAt":crate::store::now()});
            let id =
                dev::save(db, owner, run, "verification_evidence", &evidence).map_err(|e| {
                    format!(
                        "Check started as {}; evidence save failed: {e}",
                        result["processId"]
                    )
                })?;
            Ok(
                json!({"evidenceId":id,"processId":result["processId"],"check":result,"sourceSnapshot":snap,"verifiedCurrent":false,"message":"Check launch is not a passing result; read verification_report."}),
            )
        }
        "verification_report" => {
            let evidence = dev::load(
                db,
                owner,
                root,
                args["evidence_id"].as_str().ok_or("Missing evidence ID.")?,
                "verification_evidence",
            )?;
            let config = crate::check_tools::effective_config(
                ctx.state.database()?.execution_config()?,
                evidence["cwd"].as_str(),
            );
            let config_matches = evidence["configurationSha256"]
                == dev::hash(
                    serde_json::to_vec(&config)
                        .map_err(|e| e.to_string())?
                        .as_slice(),
                );
            let runtime_matches = evidence["runtimeIdentity"]["complete"] == true
                && evidence["runtimeIdentity"] == dev::runtime_identity(&config);
            let check=crate::check_tools::execute(&ctx.state.processes,owner,root,config,"check_report",json!({"id":evidence["processId"],"format":evidence["format"],"wait_ms":args["wait_ms"].as_u64().unwrap_or(0)})).await?;
            let mut freshness = dev::freshness(root, &evidence["sources"]);
            if evidence["mode"] == "workspace" {
                let inventory = crate::source_inventory::inventory(root)?;
                let previous = evidence["sources"]["files"]
                    .as_object()
                    .into_iter()
                    .flatten()
                    .map(|(p, _)| p.clone())
                    .collect::<std::collections::BTreeSet<_>>();
                let current = inventory["paths"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect::<std::collections::BTreeSet<_>>();
                let membership = previous == current && inventory["complete"] == true;
                freshness["membershipMatches"] = json!(membership);
                if !membership {
                    freshness["matches"] = json!(false);
                }
            }
            let observation = evidence["observationId"].as_str()
                .and_then(|id| dev::load(db, owner, root, id, "verification_observation").ok())
                .unwrap_or(json!({"complete":false,"warnings":["completion_evidence_missing_running_or_restarted"],"changed":[]}));

            let changed_selected = observation["changed"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .any(|p| {
                    evidence["sources"]["files"].get(p).is_some()
                        || (evidence["mode"] == "workspace" && crate::source_inventory::relevant(p))
                });
            let coverage_complete = evidence["selection"]["complete"] == true
                && evidence["sources"]
                    .get("complete")
                    .is_none_or(|v| v == true);

            let mut result = json!({"root":dev::canonical(root)?,"evidenceId":args["evidence_id"],"processId":evidence["processId"],"command":evidence["command"],"sourceSnapshot":evidence["sources"],"freshness":freshness,"runtimeMatches":runtime_matches,"configurationMatches":config_matches,"configurationSha256":evidence["configurationSha256"],"cwd":evidence["cwd"],"isError":check["isError"]==true,
                "selection":evidence["selection"],"observation":observation,"coverageComplete":coverage_complete,"changedDuringObservation":changed_selected,
                "verifiedCurrent":check["report"]["testsPassed"]==true&&freshness["matches"]==true&&config_matches&&runtime_matches&&coverage_complete&&!changed_selected&&observation["complete"]==true,"check":check,"observedAt":crate::store::now()});
            result["reportId"] = json!(dev::save(db, owner, run, "verification_report", &result)?);
            Ok(result)
        }
        "verification_compare" => {
            let before = dev::load(
                db,
                owner,
                root,
                args["before_id"].as_str().ok_or("Missing before ID.")?,
                "verification_report",
            )?;
            let after = dev::load(
                db,
                owner,
                root,
                args["after_id"].as_str().ok_or("Missing after ID.")?,
                "verification_report",
            )?;
            Ok(
                json!({"before":{"counts":before["check"]["report"]["counts"],"verdict":before["check"]["report"]["verdict"],"verifiedAtObservation":before["verifiedCurrent"]},"after":{"counts":after["check"]["report"]["counts"],"verdict":after["check"]["report"]["verdict"],"verifiedAtObservation":after["verifiedCurrent"]},"sameCommand":before["command"]==after["command"]&&before["cwd"]==after["cwd"],"sameEnvironment":before["configurationSha256"]==after["configurationSha256"],"sameSelectedSources":before["sourceSnapshot"]==after["sourceSnapshot"],"currentAfterFreshness":dev::freshness(root,&after["sourceSnapshot"]),"limitation":"Different test counts may reflect changed test selection; this comparison does not establish causality."}),
            )
        }
        _ => Err("Unknown verification tool.".into()),
    }
}
