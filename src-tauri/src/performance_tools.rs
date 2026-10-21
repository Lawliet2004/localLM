//! Repeatable shell wall-time measurements and portable profile inspection.
use crate::development_tools as dev;
use serde_json::{json, Value};
struct Running<'a> {
    registry: &'a crate::process_tools::ProcessRegistry,
    owner: &'a str,
    root: &'a str,
    id: String,
    armed: bool,
}
impl Drop for Running<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.registry.request_stop(self.owner, self.root, &self.id);
        }
    }
}
fn statistics(samples: &[f64]) -> Value {
    if samples.is_empty() {
        return Value::Null;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let n = sorted.len();
    let mean = samples.iter().sum::<f64>() / n as f64;
    let median = if n.is_multiple_of(2) {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.
    } else {
        sorted[n / 2]
    };
    json!({"n":n,"minMs":sorted[0],"maxMs":sorted[n-1],"medianMs":median,"meanMs":mean,"p95Ms":sorted[((n as f64*0.95).ceil() as usize).saturating_sub(1)],"sampleStdDevMs":if n>1{Some((samples.iter().map(|v|(v-mean).powi(2)).sum::<f64>()/(n-1)as f64).sqrt())}else{None}})
}
pub async fn execute(
    ctx: &crate::harness::HarnessCtx<'_>,
    alias: &str,
    args: Value,
) -> Result<Value, String> {
    let root = &ctx.snapshot.workspace_path;
    let owner = &ctx.conversation_id;
    if alias == "profile_inspect" {
        return profile(root, args);
    }
    if alias == "benchmark_compare" {
        let a = dev::load(
            &ctx.state.db_path,
            owner,
            root,
            args["before_id"].as_str().ok_or("Missing before ID.")?,
            "benchmark_evidence",
        )?;
        let b = dev::load(
            &ctx.state.db_path,
            owner,
            root,
            args["after_id"].as_str().ok_or("Missing after ID.")?,
            "benchmark_evidence",
        )?;
        let baseline = a["statistics"]["medianMs"].as_f64();
        let after = b["statistics"]["medianMs"].as_f64();
        let samples = |v: &Value| {
            v["samples"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|s| s["warmup"] == false && s["state"] == "exited" && s["exitCode"] == 0)
                .filter_map(|s| s["durationMs"].as_f64())
                .collect::<Vec<_>>()
        };
        let confidence = crate::profile_tools::bootstrap(&samples(&a), &samples(&b));
        return Ok(
            json!({"confidenceInterval":confidence,"before":a["statistics"],"after":b["statistics"],"medianChangePercent":baseline.zip(after).filter(|(a,_)|*a>0.).map(|(a,b)|(b-a)/a*100.),"sameCommand":a["command"]==b["command"],"sameEnvironment":a["configurationSha256"]==b["configurationSha256"]&&a["machine"]==b["machine"],"beforeComplete":a["complete"],"afterComplete":b["complete"],"afterFreshness":dev::freshness(root,&b["sourceSnapshot"]),"causalSpeedupEstablished":false,"limitation":"Wall-clock shell/process timing includes startup. Small or noisy samples and differing workloads do not establish a causal speedup."}),
        );
    }
    if alias != "benchmark_run" {
        return Err("Unknown performance tool.".into());
    }
    let snapshot = dev::snapshot(
        root,
        args["source_paths"]
            .as_array()
            .ok_or("Missing source paths.")?,
    )?;
    let mut config = ctx.state.database()?.execution_config()?;
    config.bash.timeout_seconds = config.bash.timeout_seconds.clamp(1, 30);
    let config_sha = dev::hash(
        serde_json::to_vec(&config)
            .map_err(|e| e.to_string())?
            .as_slice(),
    );
    let samples = args["samples"].as_u64().unwrap_or(5);
    let warmups = args["warmups"].as_u64().unwrap_or(1);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    let mut results = Vec::new();
    let mut times = Vec::new();
    let mut complete = true;
    for index in 0..samples + warmups {
        if std::time::Instant::now() >= deadline {
            complete = false;
            break;
        }
        let mut start = json!({"command":args["command"],"yield_ms":0,"close_stdin":true,"timeout_seconds":config.bash.timeout_seconds});
        if args["cwd"].is_string() {
            start["cwd"] = args["cwd"].clone();
        }
        let launched = ctx
            .state
            .processes
            .start(owner, root, config.clone(), start)
            .await?;
        let mut running = Running {
            registry: &ctx.state.processes,
            owner,
            root,
            id: launched["id"]
                .as_str()
                .ok_or("Process returned no ID.")?
                .into(),
            armed: true,
        };
        let result = loop {
            let result = ctx
                .state
                .processes
                .retained_output(owner, root, &running.id)?;
            if result["state"] != "running" {
                running.armed = false;
                break result;
            }
            if std::time::Instant::now() >= deadline {
                let stopped = ctx
                    .state
                    .processes
                    .stop(owner, root, json!({"id":running.id}))
                    .await?;
                running.armed = false;
                break stopped;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        };
        let success = result["state"] == "exited" && result["exitCode"] == 0;
        if index >= warmups && success {
            if let Some(ms) = result["durationMs"].as_f64() {
                times.push(ms);
            } else {
                complete = false;
            }
        }
        results.push(json!({"index":index,"warmup":index<warmups,"processId":running.id,"state":result["state"],"exitCode":result["exitCode"],"durationMs":result["durationMs"],"outputIncomplete":result["truncated"]}));
        if !success {
            complete = false;
            break;
        }
    }
    complete &= times.len() == samples as usize;
    let mut result = json!({"root":dev::canonical(root)?,"command":args["command"],"cwd":args["cwd"],"configurationSha256":config_sha,"machine":{"os":std::env::consts::OS,"architecture":std::env::consts::ARCH,"logicalCpus":std::thread::available_parallelism().map(|n|n.get()).ok()},"sourceSnapshot":snapshot,"freshness":dev::freshness(root,&snapshot),"samples":results,"statistics":statistics(&times),"requestedSamples":samples,"warmups":warmups,"complete":complete,"isError":!complete,"measurement":"supervised process wall time in milliseconds, including shell/process startup; excludes report polling and output-drain delay","causalSpeedupEstablished":false});
    result["benchmarkId"] = json!(dev::save(
        &ctx.state.db_path,
        owner,
        &ctx.run_id,
        "benchmark_evidence",
        &result
    )?);
    Ok(result)
}
pub(crate) fn profile(root: &str, args: Value) -> Result<Value, String> {
    let text = crate::workspace::Workspace::open(root)?
        .language_source(args["path"].as_str().ok_or("Missing profile path.")?)?;
    let mut profile: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    if profile["nodes"].is_array() && profile["samples"].is_array() {
        profile = crate::profile_tools::normalize_node(&profile)?;
    }
    let profile = crate::profile_tools::normalize_evented(&profile)?;
    let mut hot = std::collections::BTreeMap::<String, f64>::new();
    let mut unit = "microseconds";
    let mut count = 0;
    if let Some(profiles) = profile["profiles"].as_array() {
        let index = args["profile_index"].as_u64().unwrap_or(0) as usize;
        let p = profiles
            .get(index)
            .ok_or("Profile index outside profile list.")?;
        if p["type"] != "sampled" {
            return Err("Speedscope inspection currently supports sampled profiles only.".into());
        }
        unit = p["unit"].as_str().unwrap_or("none");
        let frames = profile["shared"]["frames"]
            .as_array()
            .ok_or("Speedscope frames missing.")?;
        let samples = p["samples"]
            .as_array()
            .ok_or("Speedscope samples missing.")?;
        let weights = p["weights"].as_array();
        if weights.is_none() {
            unit = "samples";
        }
        if weights.is_some_and(|w| w.len() != samples.len()) {
            return Err("Profile weights do not match samples.".into());
        }
        for (i, s) in samples.iter().enumerate() {
            count += 1;
            let stack = s.as_array().ok_or("Invalid profile stack.")?;
            let weight = weights
                .map(|w| w[i].as_f64().ok_or("Invalid sample weight."))
                .transpose()?
                .unwrap_or(1.);
            if !weight.is_finite() || weight < 0. {
                return Err("Invalid sample weight.".into());
            }
            if let Some(leaf) = stack.last() {
                let f = frames
                    .get(leaf.as_u64().ok_or("Invalid frame index.")? as usize)
                    .ok_or("Frame index out of range.")?;
                let key = format!(
                    "{} ({}:{})",
                    f["name"].as_str().unwrap_or("unknown"),
                    f["file"].as_str().unwrap_or("?"),
                    f["line"]
                );
                *hot.entry(key).or_default() += weight;
            }
        }
    } else {
        let events = profile
            .as_array()
            .or_else(|| profile["traceEvents"].as_array())
            .ok_or("Expected Speedscope sampled profile or Chrome trace JSON.")?;
        for e in events {
            if e["ph"] == "X" {
                let d = e["dur"].as_f64().ok_or("Trace duration missing.")?;
                if d < 0. || !d.is_finite() {
                    return Err("Invalid trace duration.".into());
                }
                count += 1;
                *hot.entry(e["name"].as_str().unwrap_or("unknown").into())
                    .or_default() += d;
            }
        }
    }
    let mut hot = hot
        .into_iter()
        .map(|(name, weight)| json!({"name":name,"weight":weight}))
        .collect::<Vec<_>>();
    hot.sort_by(|a, b| {
        b["weight"]
            .as_f64()
            .unwrap_or(0.)
            .total_cmp(&a["weight"].as_f64().unwrap_or(0.))
    });
    let total = hot.len();
    hot.truncate(50);
    Ok(
        json!({"sourceSha256":dev::hash(text.as_bytes()),"unit":unit,"records":count,"hotspots":hot,"truncated":total>50,"limitation":"Speedscope uses leaf sample weights; Chrome trace totals complete-event durations and may double-count nested or concurrent intervals. These totals are not elapsed wall time."}),
    )
}
pub fn registry() -> Vec<crate::tool_spec::ToolSpec> {
    let id = json!({"type":"string","minLength":1,"maxLength":128});
    let mut tools: Vec<crate::tool_spec::ToolSpec> = vec![("benchmark_run".into(),"Repeat an approved shell benchmark sequentially, discard warmups, retain every process ID and failed sample, and save source/environment fingerprints with wall-time statistics. Maximum 60 seconds total, 30 seconds per sample within configured shell limits. Can invoke installed profilers that write workspace traces; inspect them with profile_inspect. Commands may have side effects on every repetition.".into(),dev::schema(json!({"command":{"type":"string","minLength":1,"maxLength":32768},"cwd":{"type":"string","maxLength":4096},"samples":{"type":"integer","minimum":1,"maximum":20},"warmups":{"type":"integer","minimum":0,"maximum":5},"source_paths":{"type":"array","minItems":1,"maxItems":128,"items":{"type":"string","minLength":1,"maxLength":4096}}}),&["command","source_paths"])),
    ("benchmark_compare".into(),"Compare two owned benchmark evidence artifacts. Reports median percentage change and environment/command compatibility without claiming a causal speedup.".into(),dev::schema(json!({"before_id":id,"after_id":id}),&["before_id","after_id"])),
    ("profile_inspect".into(),"Inspect a workspace Speedscope sampled profile or Chrome complete-event trace JSON (up to 1 MiB), returning bounded hotspot aggregates and measurement limitations. Use an installed profiler through benchmark_run or shell tools to capture traces.".into(),dev::schema(json!({"path":{"type":"string","minLength":1,"maxLength":4096},"profile_index":{"type":"integer","minimum":0,"maximum":1000}}),&["path"]))].into_iter().map(crate::tool_spec::ToolSpec::from).collect();
    tools.extend(crate::profile_tools::registry());
    tools
}

pub(crate) fn inspect(root: &str, args: Value) -> Result<Value, String> {
    profile(root, args)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stats_and_profile_units_are_explicit() {
        let s = statistics(&[1., 2., 3., 4.]);
        assert_eq!(s["medianMs"], 2.5);
        assert_eq!(s["p95Ms"], 4.);
        assert!(statistics(&[]).is_null());
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("profile.json"),json!({"shared":{"frames":[{"name":"hot","file":"main.rs","line":1}]},"profiles":[{"type":"sampled","unit":"milliseconds","samples":[[0],[0]],"weights":[2.,3.]}]}).to_string()).unwrap();
        let p = profile(t.path().to_str().unwrap(), json!({"path":"profile.json"})).unwrap();
        assert_eq!(p["hotspots"][0]["weight"], 5.);
        assert_eq!(p["unit"], "milliseconds");
    }
}
