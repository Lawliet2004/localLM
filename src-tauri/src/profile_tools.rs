//! Managed first-class CPU capture with original traces and normalized evidence.
use crate::development_tools as dev;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt};
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub native_executable: String,
    pub export_executable: String,
    pub export_profile: String,
}
#[tauri::command]
pub fn get_profile_config(state: tauri::State<'_, crate::AppState>) -> Result<Config, String> {
    state.database()?.setting("development.profilers")
}
#[tauri::command]
pub fn save_profile_config(
    state: tauri::State<'_, crate::AppState>,
    config: Config,
) -> Result<(), String> {
    for p in [&config.native_executable, &config.export_executable] {
        if !p.is_empty() && !crate::execution::executable(p) {
            return Err("Profiler executables must be installed absolute executable paths.".into());
        }
    }
    if !config.export_profile.is_empty()
        && !std::path::Path::new(&config.export_profile).is_absolute()
    {
        return Err("Select an absolute WPA export profile path.".into());
    }
    state
        .database()?
        .save_setting("development.profilers", &config)
}
fn detected(name: &str) -> Result<String, String> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|v| std::env::split_paths(&v).collect::<Vec<_>>())
        .map(|p| p.join(name))
        .find(|p| crate::execution::executable(&p.to_string_lossy()))
        .map(|p| p.to_string_lossy().into_owned())
        .ok_or_else(|| {
            format!("Required profiler executable '{name}' is not installed or configured.")
        })
}
struct Tasks(Vec<tokio::task::JoinHandle<()>>);
impl Drop for Tasks {
    fn drop(&mut self) {
        for t in &self.0 {
            t.abort();
        }
    }
}
fn command(
    executable: &str,
    cwd: &std::path::Path,
    config: &crate::execution::ExecutionConfig,
) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(executable);
    c.current_dir(cwd)
        .env_clear()
        .envs(&config.bash.environment)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    for k in ["PATH", "SystemRoot", "WINDIR", "HOME", "TEMP", "TMP"] {
        if !config.bash.environment.contains_key(k) {
            if let Some(v) = std::env::var_os(k) {
                c.env(k, v);
            }
        }
    }
    crate::execution::memory_limit(&mut c, &config.bash);
    c
}
async fn run(cmd: tokio::process::Command, seconds: u64) -> Result<Vec<u8>, String> {
    let mut child = crate::execution::managed_spawn(cmd)
        .map_err(|_| "Cannot launch profiler/exporter.".to_string())?;
    let out = child
        .stdout()
        .take()
        .ok_or("Profiler stdout unavailable.")?;
    let mut err = child
        .stderr()
        .take()
        .ok_or("Profiler stderr unavailable.")?;
    let _tasks = Tasks(vec![tokio::spawn(async move {
        let mut b = [0; 8192];
        while let Ok(n) = err.read(&mut b).await {
            if n == 0 {
                break;
            }
        }
    })]);
    let result=tokio::time::timeout(std::time::Duration::from_secs(seconds),async{let mut bytes=Vec::new();out.take(8*1024*1024+1).read_to_end(&mut bytes).await.map_err(|_|"Profiler output could not be read.")?;if bytes.len()>8*1024*1024{return Err("Profiler export exceeds 8 MiB.".into());}let status=child.wait().await.map_err(|_|"Profiler process failed.")?;if !status.success(){return Err("Profiler/exporter failed. Check installed tool compatibility, symbols and native profiling permissions; no permissions were changed.".into());}Ok(bytes)}).await.map_err(|_|"Profiler timed out; owned process tree stopped.".to_string())?;
    result
}
struct GracefulChild {
    child: Option<crate::execution::ExecutionChild>,
    armed: bool,
}
impl std::ops::Deref for GracefulChild {
    type Target = crate::execution::ExecutionChild;
    fn deref(&self) -> &Self::Target {
        self.child.as_ref().expect("live owned recorder")
    }
}
impl std::ops::DerefMut for GracefulChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.child.as_mut().expect("live owned recorder")
    }
}
impl Drop for GracefulChild {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        if let (Some(mut child), Ok(runtime)) =
            (self.child.take(), tokio::runtime::Handle::try_current())
        {
            runtime.spawn(async move {
                #[cfg(unix)]
                if let Some(pid) = child
                    .id()
                    .and_then(|pid| rustix::process::Pid::from_raw(pid as i32))
                {
                    let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::INT);
                }
                #[cfg(not(unix))]
                let _ = child.start_kill();
                let _ =
                    tokio::time::timeout(std::time::Duration::from_secs(10), child.wait()).await;
                // The owned managed child forces cleanup if graceful stop failed.
            });
        }
    }
}
struct CancelledTrace {
    temp: std::sync::Arc<tempfile::TempDir>,
    db: std::path::PathBuf,
    root: String,
    owner: String,
    run: String,
    emit: std::sync::Arc<dyn Fn(String, Option<Value>) + Send + Sync>,
    armed: bool,
}
impl Drop for CancelledTrace {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let (temp, db, root, owner, run, emit) = (
            self.temp.clone(),
            self.db.clone(),
            self.root.clone(),
            self.owner.clone(),
            self.run.clone(),
            self.emit.clone(),
        );
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                // Keep trace storage alive through the owned recorder's bounded
                // graceful stop (and Windows' named WPR stop).
                tokio::time::sleep(std::time::Duration::from_secs(if cfg!(windows) {31} else {11})).await;
                let retained = retain_trace_owned(&db,&owner,&run,&root,temp.path());
                let mut evidence = json!({"root":root,"complete":false,"isError":true,"cancelled":true,"originalTraceId":retained.as_ref().ok().cloned().flatten(),"traceRetentionComplete":false,"traceRetentionError":retained.err(),"message":"Capture cancelled; retained trace may be partial and is not accepted for comparison."});
                if let Ok(id) = dev::save(&db,&owner,&run,"profile_capture",&evidence) { evidence["profileId"]=json!(id); }
                emit("tool_progress".into(),Some(evidence));
            });
        }
    }
}
// A capture duration is a normal stop condition, not a failed exporter timeout.
// SIGINT lets perf flush its owned recording before bounded forced cleanup.
async fn record_for(cmd: tokio::process::Command, seconds: u64) -> Result<Value, String> {
    let child = crate::execution::managed_spawn(cmd)
        .map_err(|_| "Cannot launch owned recording workload.")?;
    let mut child = GracefulChild {
        child: Some(child),
        armed: true,
    };
    let started = std::time::Instant::now();
    let mut tasks = Tasks(Vec::new());
    for pipe in [
        child
            .stdout()
            .take()
            .map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        child
            .stderr()
            .take()
            .map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
    ]
    .into_iter()
    .flatten()
    {
        tasks.0.push(tokio::spawn(async move {
            let mut pipe = pipe;
            let mut bytes = [0; 8192];
            while let Ok(n) = pipe.read(&mut bytes).await {
                if n == 0 {
                    break;
                }
            }
        }));
    }
    let (status, duration_reached) =
        match tokio::time::timeout(std::time::Duration::from_secs(seconds), child.wait()).await {
            Ok(status) => (status.map_err(|_| "Recorder wait failed.")?, false),
            Err(_) => {
                #[cfg(unix)]
                {
                    let pid = child.id().ok_or("Owned recorder PID unavailable.")?;
                    // Managed spawn creates a private process group; the unreaped
                    // child still owns this PID. Never signal a named/global session.
                    if rustix::process::kill_process_group(
                        rustix::process::Pid::from_raw(pid as i32)
                            .ok_or("Invalid owned recorder PID.")?,
                        rustix::process::Signal::INT,
                    )
                    .is_err()
                    {
                        return Err(
                            "Cannot gracefully stop owned recorder; trace may be partial.".into(),
                        );
                    }
                }
                #[cfg(not(unix))]
                child
                    .start_kill()
                    .map_err(|_| "Cannot stop owned workload.")?;
                let status = tokio::time::timeout(std::time::Duration::from_secs(10), child.wait())
                    .await
                    .map_err(|_| {
                        "Owned recorder did not flush within ten seconds; trace is incomplete."
                    })?
                    .map_err(|_| "Owned recorder stop failed.")?;
                (status, true)
            }
        };
    child.armed = false;
    #[allow(unused_assignments)]
    let mut expected_signal = false;
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        expected_signal =
            duration_reached && (status.signal() == Some(2) || status.code() == Some(130));
    }
    #[cfg(windows)]
    {
        expected_signal = duration_reached;
    }
    if !status.success() && !expected_signal {
        return Err("Native recording/workload failed; permissions and trace completeness were not established.".into());
    }
    Ok(
        json!({"actualDurationMs":started.elapsed().as_secs_f64()*1000.,"durationLimitReached":duration_reached,"earlyWorkloadExit":!duration_reached,"gracefulStopConfirmed":true,"exitCode":status.code()}),
    )
}
async fn node_capture(
    config: &crate::execution::ExecutionConfig,
    cwd: &std::path::Path,
    program: &str,
    args: &[String],
    seconds: u64,
) -> Result<Value, String> {
    let executable = &config.node_path;
    if !crate::execution::executable(executable) {
        return Err("Configure an installed Node executable.".into());
    }
    let mut cmd = command(executable, cwd, config);
    cmd.args(["--inspect-brk=127.0.0.1:0", "--", program])
        .args(args);
    let mut child = crate::execution::managed_spawn(cmd)
        .map_err(|_| "Cannot launch Node inspector.".to_string())?;
    let stdout = child.stdout().take().ok_or("Node stdout unavailable.")?;
    let stderr = child.stderr().take().ok_or("Node stderr unavailable.")?;
    let mut stderr = tokio::io::BufReader::new(stderr);
    let mut line = String::new();
    let mut endpoint = None;
    for _ in 0..16 {
        line.clear();
        let n = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            stderr.read_line(&mut line),
        )
        .await
        .map_err(|_| "Node inspector initialization timed out.")?
        .map_err(|_| "Node inspector closed.")?;
        if n == 0 {
            break;
        }
        if let Some((_, url)) = line.split_once("Debugger listening on ") {
            endpoint = Some(url.trim().to_string());
            break;
        }
        if line.len() > 8192 {
            return Err("Node inspector diagnostics exceed bounds.".into());
        }
    }
    let endpoint = endpoint.ok_or("Node did not expose an inspector endpoint.")?;
    let u = reqwest::Url::parse(&endpoint).map_err(|_| "Invalid Node inspector endpoint.")?;
    if u.scheme() != "ws" || !matches!(u.host_str(), Some("127.0.0.1") | Some("localhost")) {
        return Err("Node inspector must remain on loopback.".into());
    }
    let _tasks = Tasks(vec![
        tokio::spawn(async move {
            let mut stdout = stdout;
            let mut b = [0; 8192];
            while let Ok(n) = stdout.read(&mut b).await {
                if n == 0 {
                    break;
                }
            }
        }),
        tokio::spawn(async move {
            let mut b = [0; 8192];
            while let Ok(n) = stderr.read(&mut b).await {
                if n == 0 {
                    break;
                }
            }
        }),
    ]);
    let (mut socket, _) = tokio_tungstenite::connect_async(&endpoint)
        .await
        .map_err(|_| "Cannot connect to owned Node inspector.")?;
    async fn request<S>(
        socket: &mut tokio_tungstenite::WebSocketStream<S>,
        id: u64,
        method: &str,
    ) -> Result<Value, String>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
    {
        socket
            .send(tokio_tungstenite::tungstenite::Message::Text(
                json!({"id":id,"method":method}).to_string().into(),
            ))
            .await
            .map_err(|_| "Node profiler request failed.")?;
        for _ in 0..1024 {
            let message = tokio::time::timeout(std::time::Duration::from_secs(10), socket.next())
                .await
                .map_err(|_| "Node profiler request timed out.")?
                .ok_or("Node inspector closed.")?
                .map_err(|_| "Node inspector transport failed.")?;
            if let tokio_tungstenite::tungstenite::Message::Text(text) = message {
                if text.len() > 8 * 1024 * 1024 {
                    return Err("Node profile exceeds 8 MiB.".into());
                }
                let value: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
                if value["id"] == id {
                    if value.get("error").is_some() {
                        return Err("Node profiler protocol request failed.".into());
                    }
                    return Ok(value["result"].clone());
                }
            }
        }
        Err("Node inspector event budget exceeded.".into())
    }
    request(&mut socket, 1, "Profiler.enable").await?;
    request(&mut socket, 2, "Profiler.start").await?;
    request(&mut socket, 3, "Runtime.runIfWaitingForDebugger").await?;
    tokio::time::sleep(std::time::Duration::from_secs(seconds)).await;
    let result = request(&mut socket, 4, "Profiler.stop").await?;
    let _ = socket.close(None).await;
    let _ = child.start_kill();
    let _ = child.wait().await;
    Ok(result["profile"].clone())
}
pub fn normalize_node(profile: &Value) -> Result<Value, String> {
    let nodes = profile["nodes"]
        .as_array()
        .ok_or("Node profile lacks nodes.")?;
    if nodes.len() > 100000 {
        return Err("Node frame budget exceeded.".into());
    }
    let mut index = std::collections::BTreeMap::new();
    let mut parents = std::collections::BTreeMap::new();
    let mut frames = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let id = node["id"].as_u64().ok_or("Invalid Node frame ID.")?;
        if index.insert(id, i).is_some() {
            return Err("Duplicate Node frame ID.".into());
        }
        for child in node["children"].as_array().into_iter().flatten() {
            if parents
                .insert(child.as_u64().ok_or("Invalid Node child ID.")?, id)
                .is_some()
            {
                return Err("Node frame has multiple parents.".into());
            }
        }
        frames.push(json!({"name":node["callFrame"]["functionName"],"file":node["callFrame"]["url"],"line":node["callFrame"]["lineNumber"].as_i64().map(|v|v+1)}));
    }
    let samples = profile["samples"]
        .as_array()
        .ok_or("Node profile has no samples.")?;
    if samples.len() > 100000 {
        return Err("Node sample budget exceeded.".into());
    }
    let mut visits = 0;
    let mut stacks = Vec::new();
    for sample in samples {
        let mut id = sample.as_u64().ok_or("Invalid Node sample.")?;
        let mut stack = Vec::new();
        for _ in 0..256 {
            visits += 1;
            if visits > 1000000 {
                return Err("Node stack expansion budget exceeded.".into());
            }
            stack.push(
                *index
                    .get(&id)
                    .ok_or("Node sample references unknown frame.")?,
            );
            if let Some(parent) = parents.get(&id) {
                id = *parent;
            } else {
                break;
            }
        }
        if stack.len() >= 256 {
            return Err("Node frame cycle/depth limit.".into());
        }
        stack.reverse();
        stacks.push(stack);
    }
    let weights = profile["timeDeltas"]
        .as_array()
        .filter(|w| w.len() == samples.len())
        .cloned();
    Ok(
        json!({"shared":{"frames":frames},"profiles":[{"type":"sampled","name":"Node CPU","unit":if weights.is_some(){"microseconds"}else{"none"},"samples":stacks,"weights":weights}]}),
    )
}
fn sampled_stacks(
    stacks: Vec<Vec<String>>,
    weights: Option<Vec<f64>>,
    unit: &str,
) -> Result<Value, String> {
    if stacks.is_empty()
        || weights
            .as_ref()
            .is_some_and(|v| v.len() != stacks.len() || v.iter().any(|n| !n.is_finite() || *n < 0.))
    {
        return Err("No valid weighted CPU samples.".into());
    }
    let mut indices = std::collections::BTreeMap::new();
    let mut frames = Vec::new();
    let mut samples = Vec::new();
    for stack in stacks {
        if stack.is_empty() {
            continue;
        }
        let mut sample = Vec::new();
        for name in stack {
            let i = *indices.entry(name.clone()).or_insert_with(|| {
                let i = frames.len();
                frames.push(json!({"name":name}));
                i
            });
            sample.push(i);
        }
        samples.push(sample);
    }
    Ok(
        json!({"shared":{"frames":frames},"profiles":[{"type":"sampled","name":"Native CPU","unit":unit,"samples":samples,"weights":weights}]}),
    )
}
fn normalize_perf(text: &str) -> Result<Value, String> {
    let mut stacks = Vec::new();
    let mut stack = Vec::new();
    let mut weights = Vec::new();
    let mut period = None;
    let mut weighted = None;
    let period_pattern = regex::Regex::new(r":\s+(\d+)\s+cpu-clock(?::[a-z]+)?:").unwrap();
    for line in text.lines().chain(std::iter::once("")) {
        if line.starts_with('#') {
            continue;
        }
        if line.starts_with('\t') || line.starts_with("        ") {
            let mut parts = line.split_whitespace();
            let address = parts.next().unwrap_or("");
            if !address.is_empty() && address.bytes().all(|b| b.is_ascii_hexdigit()) {
                let rest = parts.collect::<Vec<_>>().join(" ");
                let symbol = rest
                    .rsplit_once(" (")
                    .map(|(name, _)| name)
                    .unwrap_or(&rest);
                let name = symbol
                    .rsplit_once("+0x")
                    .map(|(name, _)| name)
                    .unwrap_or(symbol);
                let identity = if let Some((_, module)) = rest.rsplit_once(" (") {
                    format!("{name} ({module}")
                } else {
                    name.to_string()
                };
                stack.push(identity);
            }
        } else {
            if !stack.is_empty() {
                stack.reverse();
                stacks.push(std::mem::take(&mut stack));
                let has_period = period.is_some();
                if weighted.is_some_and(|previous| previous != has_period) {
                    return Err("Mixed weighted/unweighted perf samples are unsupported.".into());
                }
                weighted = Some(has_period);
                weights.push(period.unwrap_or(1.));
            }
            period = period_pattern
                .captures(line)
                .and_then(|c| c[1].parse::<f64>().ok());
            if line.contains("cpu-clock") && period.is_none() {
                return Err("perf CPU-clock export is missing its sample period.".into());
            }
            if line.contains("PERF_RECORD_LOST") || line.contains("LOST_SAMPLES") {
                return Err("perf reported lost samples; capture is incomplete.".into());
            }
        }
    }
    sampled_stacks(
        stacks,
        Some(weights),
        if weighted == Some(true) {
            "nanoseconds"
        } else {
            "none"
        },
    )
}
#[cfg(any(test, target_os = "windows"))]
fn normalize_wpa(bytes: &[u8]) -> Result<Value, String> {
    let mut reader = csv::Reader::from_reader(bytes);
    let headers = reader.headers().map_err(|e| e.to_string())?.clone();
    let function = headers
        .iter()
        .position(|v| v.eq_ignore_ascii_case("Function"))
        .ok_or("WPA export needs Function and Weight (ms) columns.")?;
    let weight=headers.iter().position(|v|v.eq_ignore_ascii_case("Weight (ms)")).ok_or("WPA CPU export must include an explicit Weight (ms) column; grouped rows cannot be counted as samples.")?;
    let chain = headers
        .iter()
        .position(|v| v.eq_ignore_ascii_case("Stack") || v.eq_ignore_ascii_case("Stack (JSON)"));
    let mut stacks = Vec::new();
    let mut weights = Vec::new();
    for row in reader.records() {
        let row = row.map_err(|e| e.to_string())?;
        let function = row.get(function).ok_or("Missing WPA function.")?;
        let stack = if let Some(index) = chain {
            let stack:Vec<String>=serde_json::from_str(row.get(index).ok_or("Missing WPA stack.")?).map_err(|_| "Unsupported WPA stack encoding; export root-to-leaf JSON symbol arrays, or omit Stack and explicitly use leaf-only evidence.")?;
            if stack.is_empty()
                || stack.len() > 256
                || stack.last().map(String::as_str) != Some(function)
            {
                return Err("WPA stack is empty, oversized or disagrees with Function.".into());
            }
            stack
        } else {
            vec![function.to_string()]
        };
        stacks.push(stack);
        weights.push(
            row.get(weight)
                .ok_or("Missing WPA weight.")?
                .parse::<f64>()
                .map_err(|_| "Invalid WPA weight.")?,
        );
    }
    let mut result = sampled_stacks(stacks, Some(weights), "milliseconds")?;
    result["callchainEvidence"] = json!(if chain.is_some() {
        "root_to_leaf_export"
    } else {
        "leaf_only_aggregation"
    });
    Ok(result)
}
#[cfg(any(test, target_os = "macos"))]
fn normalize_xctrace(xml: &str) -> Result<Value, String> {
    let doc = roxmltree::Document::parse(xml).map_err(|_| "Invalid xctrace XML.")?;
    let ids = doc
        .descendants()
        .filter_map(|n| n.attribute("id").map(|id| (id, n)))
        .collect::<std::collections::HashMap<_, _>>();
    fn resolve<'a, 'input>(
        node: roxmltree::Node<'a, 'input>,
        ids: &std::collections::HashMap<&'a str, roxmltree::Node<'a, 'input>>,
    ) -> Result<roxmltree::Node<'a, 'input>, &'static str> {
        match node.attribute("ref") {
            Some(id) => ids
                .get(id)
                .copied()
                .ok_or("xctrace contains a missing exported reference."),
            None => Ok(node),
        }
    }
    let mut stacks = Vec::new();
    let mut weights = Vec::new();
    let mut unit = None;
    for row in doc.descendants().filter(|n| n.has_tag_name("row")) {
        let backtrace = resolve(
            row.children()
                .find(|n| n.has_tag_name("backtrace"))
                .ok_or("xctrace CPU row has no backtrace; capture is incomplete.")?,
            &ids,
        )?;
        let mut names = Vec::new();
        for frame in backtrace.children().filter(|n| n.has_tag_name("frame")) {
            let frame = resolve(frame, &ids)?;
            names.push(
                frame
                    .attribute("name")
                    .or_else(|| frame.attribute("fmt"))
                    .unwrap_or("[unknown]")
                    .to_string(),
            );
        }
        if names.is_empty() {
            return Err("xctrace CPU row has an empty backtrace.".into());
        }
        names.reverse();
        stacks.push(names);
        let (sample_unit, weight) =
            if let Some(weight) = row.children().find(|n| n.has_tag_name("weight")) {
                let weight = resolve(weight, &ids)?;
                let unit = weight
                    .attribute("unit")
                    .ok_or("xctrace weight has no explicit unit; refusing to infer CPU time.")?;
                let unit = match unit {
                    "ns" | "nanoseconds" => "nanoseconds",
                    "us" | "microseconds" => "microseconds",
                    "ms" | "milliseconds" => "milliseconds",
                    "s" | "seconds" => "seconds",
                    "count" | "none" => "none",
                    _ => return Err("Unsupported xctrace weight unit.".into()),
                };
                (
                    unit,
                    weight
                        .text()
                        .ok_or("Missing xctrace weight.")?
                        .parse::<f64>()
                        .map_err(|_| "Invalid xctrace weight.")?,
                )
            } else {
                ("none", 1.)
            };
        if unit.is_some_and(|unit| unit != sample_unit) {
            return Err("Mixed xctrace weight units are unsupported.".into());
        }
        unit = Some(sample_unit);
        weights.push(weight);
    }
    sampled_stacks(stacks, Some(weights), unit.unwrap_or("none"))
}
async fn native_identity(
    config: &crate::execution::ExecutionConfig,
    settings: &Config,
    kind: &str,
    cwd: &std::path::Path,
) -> Value {
    let native = if kind == "node" {
        Ok(config.node_path.clone())
    } else if !settings.native_executable.is_empty() {
        Ok(settings.native_executable.clone())
    } else {
        detected(if cfg!(target_os = "linux") {
            "perf"
        } else if cfg!(target_os = "macos") {
            "xctrace"
        } else {
            "wpr.exe"
        })
    };
    let Ok(native) = native else {
        return json!({"complete":false,"reason":"profiler_executable_unavailable"});
    };
    let exporter = if kind == "node" || settings.export_executable.is_empty() {
        native.clone()
    } else {
        settings.export_executable.clone()
    };
    let mut versions = Vec::new();
    for executable in [&native, &exporter] {
        let identity = dev::executable_identity(executable);
        let mut cmd = command(executable, cwd, config);
        cmd.arg(if cfg!(target_os = "macos") && kind != "node" {
            "version"
        } else if cfg!(windows) && kind != "node" {
            "-help"
        } else {
            "--version"
        });
        let version = run(cmd, 5)
            .await
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .map(|text| text.chars().take(2000).collect::<String>());
        versions.push(json!({"executable":identity,"version":version}));
    }
    json!({"complete":versions.iter().all(|v| v["executable"]["unavailable"]!=true && v["version"].as_str().is_some_and(|v| !v.trim().is_empty())),"tools":versions})
}
async fn native_capture(
    ctx: &crate::harness::HarnessCtx<'_>,
    config: &crate::execution::ExecutionConfig,
    cwd: &std::path::Path,
    program: &str,
    args: &[String],
    seconds: u64,
    temp: &std::path::Path,
) -> Result<(Value, Value), String> {
    let settings: Config = ctx.state.database()?.setting("development.profilers")?;
    #[cfg(target_os = "linux")]
    {
        let exe = if settings.native_executable.is_empty() {
            detected("perf")?
        } else {
            settings.native_executable
        };
        let output = temp.join("capture.perf.data");
        let mut cmd = command(&exe, cwd, config);
        cmd.args(["record", "-e", "cpu-clock:u", "--call-graph", "dwarf", "-o"])
            .arg(&output)
            .arg("--")
            .arg(program)
            .args(args);
        let lifecycle = record_for(cmd, seconds).await?;
        let exporter = if settings.export_executable.is_empty() {
            exe
        } else {
            settings.export_executable
        };
        let mut cmd = command(&exporter, cwd, config);
        cmd.args(["script", "-F", "+period", "-i"]).arg(output);
        let text = String::from_utf8(run(cmd, 30).await?)
            .map_err(|_| "perf export is not UTF-8.".to_string())?;
        return Ok((
            normalize_perf(&text)?,
            json!({"format":"perf_script","text":text,"lifecycle":lifecycle,"normalization":"one root-to-leaf callchain per CPU-clock sample; exported period weights in nanoseconds"}),
        ));
    }
    #[cfg(target_os = "macos")]
    {
        let exe = if settings.native_executable.is_empty() {
            detected("xctrace")?
        } else {
            settings.native_executable
        };
        let output = temp.join("capture.trace");
        let mut cmd = command(&exe, cwd, config);
        cmd.args(["record", "--template", "Time Profiler", "--output"])
            .arg(&output)
            .args(["--time-limit", &format!("{seconds}s"), "--launch", "--"])
            .arg(program)
            .args(args);
        record_for(cmd, seconds + 15).await?;
        let mut cmd = command(&exe, cwd, config);
        cmd.arg("export").arg("--input").arg(&output).args([
            "--xpath",
            "/trace-toc/run[@number='1']/data/table[@schema='time-profile']",
        ]);
        let xml = String::from_utf8(run(cmd, 30).await?)
            .map_err(|_| "xctrace export is not UTF-8.".to_string())?;
        return Ok((
            normalize_xctrace(&xml)?,
            json!({"format":"xctrace_xml","text":xml,"normalization":"one root-to-leaf backtrace per exported CPU row; sample counts"}),
        ));
    }
    #[cfg(target_os = "windows")]
    {
        let exe = if settings.native_executable.is_empty() {
            detected("wpr.exe")?
        } else {
            settings.native_executable
        };
        if settings.export_profile.is_empty() {
            return Err(
                "Configure a WPA export profile selecting sampled CPU Function/Weight columns."
                    .into(),
            );
        }
        let exporter = if settings.export_executable.is_empty() {
            detected("wpaexporter.exe")?
        } else {
            settings.export_executable
        };
        let instance = format!("locallm_{}", uuid::Uuid::new_v4().simple());
        let trace = temp.join("capture.etl");
        let mut start = command(&exe, cwd, config);
        start.args(["-start", "CPU", "-filemode", "-instancename", &instance]);
        struct Stop {
            exe: String,
            instance: String,
            trace: std::path::PathBuf,
            armed: bool,
        }
        impl Drop for Stop {
            fn drop(&mut self) {
                if self.armed {
                    let exe = self.exe.clone();
                    let instance = self.instance.clone();
                    let trace = self.trace.clone();
                    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                        runtime.spawn(async move {
                            let mut command = tokio::process::Command::new(exe);
                            command
                                .arg("-stop")
                                .arg(trace)
                                .args(["-instancename", &instance])
                                .kill_on_drop(true);
                            let _ = tokio::time::timeout(
                                std::time::Duration::from_secs(30),
                                command.status(),
                            )
                            .await;
                        });
                    }
                }
            }
        }
        let mut guard = Stop {
            exe: exe.clone(),
            instance: instance.clone(),
            trace: trace.clone(),
            armed: true,
        };
        run(start, 15).await?;
        let mut target = command(program, cwd, config);
        target.args(args);
        let result = record_for(target, seconds).await;
        let mut stop = command(&exe, cwd, config);
        stop.arg("-stop")
            .arg(trace.as_os_str())
            .args(["-instancename", &instance]);
        run(stop, 30).await?;
        guard.armed = false;
        result?;
        let mut export = command(&exporter, cwd, config);
        export
            .arg("-i")
            .arg(trace)
            .arg("-profile")
            .arg(settings.export_profile)
            .arg("-outputfolder")
            .arg(temp);
        run(export, 30).await?;
        let mut normalized = None;
        let mut traces = Vec::new();
        for entry in std::fs::read_dir(temp).map_err(|e| e.to_string())? {
            let p = entry.map_err(|e| e.to_string())?.path();
            if p.extension().is_some_and(|v| v == "csv") {
                let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
                if bytes.len() > 8 * 1024 * 1024 {
                    return Err("WPA export exceeds 8 MiB.".into());
                }
                if normalized.is_some() {
                    return Err("WPA export must select exactly one CPU table.".into());
                }
                normalized = Some(normalize_wpa(&bytes)?);
                traces.push(String::from_utf8_lossy(&bytes).into_owned());
            }
        }
        return Ok((
            normalized.ok_or("WPA export contained no CPU table.")?,
            json!({"format":"wpa_csv","tables":traces,"normalization":"exclusive CPU Function weights in milliseconds"}),
        ));
    }
    #[allow(unreachable_code)]
    Err("Native profiling is unavailable on this platform.".into())
}
fn retain_trace(
    ctx: &crate::harness::HarnessCtx<'_>,
    temp: &std::path::Path,
) -> Result<Option<String>, String> {
    retain_trace_owned(
        &ctx.state.db_path,
        &ctx.conversation_id,
        &ctx.run_id,
        &dev::canonical(&ctx.snapshot.workspace_path)?,
        temp,
    )
}
fn retain_trace_owned(
    db: &std::path::Path,
    owner: &str,
    run: &str,
    root: &str,
    temp: &std::path::Path,
) -> Result<Option<String>, String> {
    use base64::Engine;
    let workspace =
        crate::workspace::Workspace::open(temp.to_str().ok_or("Trace path is not UTF-8.")?)?;
    let mut pending = vec![std::path::PathBuf::new()];
    let mut files = Vec::new();
    let mut total = 0usize;
    while let Some(relative) = pending.pop() {
        for entry in std::fs::read_dir(temp.join(&relative)).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let path = relative.join(entry.file_name());
            let meta = entry.file_type().map_err(|e| e.to_string())?;
            if meta.is_symlink() {
                return Err("Profiler trace contains an unsafe symlink.".into());
            }
            if meta.is_dir() {
                pending.push(path);
                if pending.len() > 128 {
                    return Err("Profiler trace directory limit exceeded.".into());
                }
            } else if meta.is_file() {
                let data = workspace.development_bytes(
                    path.to_str().ok_or("Non-UTF-8 trace filename.")?,
                    5 * 1024 * 1024,
                )?;
                total += data.len();
                if total > 5 * 1024 * 1024 || files.len() >= 128 {
                    return Err("Original profiler trace exceeds the 5 MiB/128-file artifact budget; select a shorter capture.".into());
                }
                files.push(json!({"path":path.to_string_lossy(),"sha256":dev::hash(&data),"bytes":data.len(),"base64":base64::engine::general_purpose::STANDARD.encode(data)}));
            }
        }
    }
    if files.is_empty() {
        return Ok(None);
    }
    dev::save(
        db,
        owner,
        run,
        "profile_original_trace",
        &json!({"root":root,"encoding":"base64_per_file","files":files}),
    )
    .map(Some)
}
pub async fn capture(ctx: &crate::harness::HarnessCtx<'_>, args: Value) -> Result<Value, String> {
    let mut config = ctx.state.database()?.execution_config()?;
    if !config.bash.enabled {
        return Err("Profiling requires enabled local execution.".into());
    }
    if let Some(cwd) = args["cwd"].as_str() {
        config.bash.working_directory = cwd.into();
    }
    let execution =
        crate::execution::LocalExecution::new(config.clone(), &ctx.snapshot.workspace_path)?;
    let cwd = execution.shell_directory()?;
    let seconds = args["duration_seconds"].as_u64().unwrap_or(5);
    if seconds > config.bash.timeout_seconds {
        return Err("Capture duration exceeds configured execution timeout.".into());
    }
    let paths = args["source_paths"]
        .as_array()
        .ok_or("Choose profile source files.")?;
    let snapshot = dev::snapshot(&ctx.snapshot.workspace_path, paths)?;
    let program = args["program"].as_str().ok_or("Missing profile program.")?;
    let kind = args["kind"].as_str().unwrap_or("node");
    let arguments = args["arguments"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    let workload_path = if kind == "node" {
        config.node_path.as_str()
    } else {
        program
    };
    let workload_identity = dev::executable_identity(workload_path);
    let profiler_config: Config = ctx.state.database()?.setting("development.profilers")?;
    let capture_identity = native_identity(&config, &profiler_config, kind, &cwd).await;
    let temp = std::sync::Arc::new(tempfile::tempdir().map_err(|e| e.to_string())?);
    let mut cancelled_trace = CancelledTrace {
        temp: temp.clone(),
        db: ctx.state.db_path.clone(),
        root: dev::canonical(&ctx.snapshot.workspace_path)?,
        owner: ctx.conversation_id.clone(),
        run: ctx.run_id.clone(),
        emit: ctx.emit.clone(),
        armed: true,
    };
    let (profile, original, profiler) = if kind == "node" {
        let program = if std::path::Path::new(program).is_absolute() {
            let root =
                std::fs::canonicalize(&ctx.snapshot.workspace_path).map_err(|e| e.to_string())?;
            let relative = std::path::Path::new(program)
                .strip_prefix(&root)
                .map_err(|_| "Node program must be inside the selected workspace.")?;
            crate::workspace::Workspace::open(&ctx.snapshot.workspace_path)?
                .language_source(&relative.to_string_lossy())?;
            program.into()
        } else {
            crate::workspace::Workspace::open(&ctx.snapshot.workspace_path)?
                .language_source(program)?;
            std::path::Path::new(&ctx.snapshot.workspace_path)
                .join(program)
                .to_string_lossy()
                .into_owned()
        };
        let original = node_capture(&config, &cwd, &program, &arguments, seconds).await?;
        (normalize_node(&original)?, original, "node-inspector")
    } else {
        if !crate::execution::executable(program) {
            return Err("Native program must be an installed absolute executable.".into());
        }
        let captured = native_capture(
            ctx,
            &config,
            &cwd,
            program,
            &arguments,
            seconds,
            temp.path(),
        )
        .await;
        let (profile, original) = match captured {
            Ok(value) => value,
            Err(error) => {
                let retained = retain_trace(ctx, temp.path());
                let mut result = json!({"root":dev::canonical(&ctx.snapshot.workspace_path)?,"isError":true,"complete":false,"message":error,"originalTraceId":retained.as_ref().ok().cloned().flatten(),"traceRetentionComplete":false,"traceRetentionError":retained.err(),"sourceSnapshot":snapshot,"workloadIdentity":workload_identity,"captureIdentity":capture_identity});
                result["profileId"] = json!(dev::save(
                    &ctx.state.db_path,
                    &ctx.conversation_id,
                    &ctx.run_id,
                    "profile_capture",
                    &result
                )?);
                cancelled_trace.armed = false;
                return Ok(result);
            }
        };
        (profile, original, "native")
    };
    let retention = retain_trace(ctx, temp.path());
    let original_trace_id = retention.as_ref().ok().cloned().flatten();
    let retention_complete = retention.is_ok() && (kind == "node" || original_trace_id.is_some());
    let workload_unchanged = workload_identity == dev::executable_identity(workload_path);
    let mut result = json!({"workloadIdentity":workload_identity,"captureIdentity":capture_identity,"cwd":cwd,"root":dev::canonical(&ctx.snapshot.workspace_path)?,"originalTraceId":original_trace_id,"runtimeIdentity":dev::runtime_identity(&config),"profilerConfigurationSha256":dev::hash(&serde_json::to_vec(&profiler_config).map_err(|e|e.to_string())?),"profile":profile,"original":original,"profiler":profiler,"program":program,"arguments":arguments,"durationSeconds":seconds,"configurationSha256":dev::hash(serde_json::to_vec(&config).map_err(|e|e.to_string())?.as_slice()),"sourceSnapshot":snapshot,"freshness":dev::freshness(&ctx.snapshot.workspace_path,&snapshot),"machine":{"os":std::env::consts::OS,"architecture":std::env::consts::ARCH,"logicalCpus":std::thread::available_parallelism().ok().map(|n|n.get()),"hostFingerprint":std::env::var("COMPUTERNAME").or_else(|_|std::env::var("HOSTNAME")).or_else(|_|std::env::var("HOST")).ok().filter(|v|!v.is_empty()).map(|v|dev::hash(v.as_bytes()))},"workloadUnchanged":workload_unchanged,"traceRetentionComplete":retention_complete,"traceRetentionError":retention.err(),"complete":retention_complete&&workload_unchanged,"symbolization":"as_exported","capturedAt":crate::store::now()});
    result["profileId"] = json!(dev::save(
        &ctx.state.db_path,
        &ctx.conversation_id,
        &ctx.run_id,
        "profile_capture",
        &result
    )?);
    cancelled_trace.armed = false;
    Ok(result)
}
fn hotspots(profile: &Value) -> Result<(String, std::collections::BTreeMap<String, f64>), String> {
    let frames = profile["shared"]["frames"]
        .as_array()
        .ok_or("Profile lacks frame table.")?;
    let p = &profile["profiles"][0];
    if p["type"] != "sampled" {
        return Err("Capture comparison requires normalized sampled profiles.".into());
    }
    let unit = if p["weights"].is_array() {
        p["unit"].as_str().unwrap_or("none")
    } else {
        "samples"
    };
    let mut hot = std::collections::BTreeMap::new();
    for (i, sample) in p["samples"]
        .as_array()
        .ok_or("Profile lacks samples.")?
        .iter()
        .enumerate()
    {
        if let Some(index) = sample
            .as_array()
            .and_then(|v| v.last())
            .and_then(Value::as_u64)
        {
            let frame = frames
                .get(index as usize)
                .ok_or("Invalid profile frame index.")?;
            let name = format!(
                "{}@{}",
                frame["name"].as_str().unwrap_or("unknown"),
                frame["file"].as_str().unwrap_or("?")
            );
            let weight = if p["weights"].is_array() {
                p["weights"][i].as_f64().ok_or("Invalid profile weight.")?
            } else {
                1.
            };
            if !weight.is_finite() || weight < 0. {
                return Err("Invalid profile weight.".into());
            }
            *hot.entry(name).or_insert(0.) += weight;
        }
    }
    Ok((unit.into(), hot))
}
pub async fn compare(ctx: &crate::harness::HarnessCtx<'_>, args: Value) -> Result<Value, String> {
    let a = dev::load(
        &ctx.state.db_path,
        &ctx.conversation_id,
        &ctx.snapshot.workspace_path,
        args["before_id"]
            .as_str()
            .ok_or("Missing before capture ID.")?,
        "profile_capture",
    )?;
    let b = dev::load(
        &ctx.state.db_path,
        &ctx.conversation_id,
        &ctx.snapshot.workspace_path,
        args["after_id"]
            .as_str()
            .ok_or("Missing after capture ID.")?,
        "profile_capture",
    )?;
    for field in [
        "profiler",
        "profilerConfigurationSha256",
        "runtimeIdentity",
        "program",
        "arguments",
        "durationSeconds",
        "configurationSha256",
        "machine",
        "captureIdentity",
        "cwd",
    ] {
        if a[field] != b[field] {
            return Err(format!(
                "Profile comparison is incompatible: {field} differs."
            ));
        }
    }
    if a["machine"]["hostFingerprint"].is_null() || b["machine"]["hostFingerprint"].is_null() {
        return Err(
            "Host identity unavailable; capture comparison cannot establish compatible machines."
                .into(),
        );
    }
    if a["captureIdentity"]["complete"] != true
        || b["captureIdentity"]["complete"] != true
        || a["workloadIdentity"]["unavailable"] == true
        || b["workloadIdentity"]["unavailable"] == true
    {
        return Err(
            "Incomplete executable/version provenance cannot establish compatible captures.".into(),
        );
    }
    if a["complete"] != true || b["complete"] != true {
        return Err("Incomplete captures cannot establish a profile comparison.".into());
    }
    let (unit, before) = hotspots(&a["profile"])?;
    let (after_unit, after) = hotspots(&b["profile"])?;
    if unit != after_unit {
        return Err("Profile measurement units differ.".into());
    }
    let keys = before
        .keys()
        .chain(after.keys())
        .collect::<std::collections::BTreeSet<_>>();
    let mut changes=keys.into_iter().map(|key|{let a=before.get(key).copied().unwrap_or(0.);let b=after.get(key).copied().unwrap_or(0.);json!({"symbol":key,"before":a,"after":b,"delta":b-a,"change":if !before.contains_key(key){"added"}else if !after.contains_key(key){"removed"}else{"changed"}})}).collect::<Vec<_>>();
    changes.sort_by(|a, b| {
        b["delta"]
            .as_f64()
            .unwrap_or(0.)
            .abs()
            .total_cmp(&a["delta"].as_f64().unwrap_or(0.).abs())
    });
    let total = changes.len();
    changes.truncate(100);
    Ok(
        json!({"unit":unit,"hotspots":changes,"sameWorkloadBinary":a["workloadIdentity"]==b["workloadIdentity"],"truncated":total>100,"identity":"symbol_and_file","afterFreshness":dev::freshness(&ctx.snapshot.workspace_path,&b["sourceSnapshot"]),"symbolization":"as_exported","causalSpeedupEstablished":false,"limitation":"Missing symbols, idle samples and exporter configuration can affect interpretation. Sample deltas alone do not establish causal speedup."}),
    )
}
pub fn normalize_evented(profile: &Value) -> Result<Value, String> {
    let frame_count = profile["shared"]["frames"].as_array().map_or(0, Vec::len);
    let mut normalized = profile.clone();
    let Some(profiles) = normalized["profiles"].as_array_mut() else {
        return Ok(normalized);
    };
    for p in profiles {
        if p["type"] != "evented" {
            continue;
        }
        let mut stack = Vec::<usize>::new();
        let mut samples = Vec::new();
        let mut weights = Vec::new();
        let mut previous = p["startValue"]
            .as_f64()
            .ok_or("Evented profile lacks startValue.")?;
        let events = p["events"]
            .as_array()
            .ok_or("Evented profile lacks events.")?;
        if events.len() > 100000 {
            return Err("Evented sample budget exceeded.".into());
        }
        for event in events {
            let time = event["at"].as_f64().ok_or("Invalid event time.")?;
            if time < previous || !time.is_finite() {
                return Err("Evented timestamps are not monotonic.".into());
            }
            if !stack.is_empty() && time > previous {
                samples.push(stack.clone());
                weights.push(time - previous);
            }
            previous = time;
            let frame = event["frame"].as_u64().ok_or("Invalid evented frame.")? as usize;
            if frame >= frame_count {
                return Err("Evented frame index is outside the frame table.".into());
            }
            match event["type"].as_str() {
                Some("O") => stack.push(frame),
                Some("C") if stack.pop() == Some(frame) => (),
                _ => return Err("Unbalanced evented profile stack.".into()),
            }
        }
        if !stack.is_empty() {
            return Err("Evented profile ended with open frames.".into());
        }
        if p["endValue"]
            .as_f64()
            .is_some_and(|end| !end.is_finite() || end < previous)
        {
            return Err("Evented endValue precedes its events.".into());
        }
        let unit = p["unit"].clone();
        let name = p["name"].clone();
        *p = json!({"type":"sampled","unit":unit,"name":name,"samples":samples,"weights":weights});
    }
    Ok(normalized)
}
pub fn bootstrap(before: &[f64], after: &[f64]) -> Value {
    if before.len() < 2 || after.len() < 2 {
        return json!({"available":false,"reason":"at_least_two_samples_per_group_required"});
    }
    fn median(v: &mut [f64]) -> f64 {
        v.sort_by(f64::total_cmp);
        let n = v.len();
        if n.is_multiple_of(2) {
            (v[n / 2 - 1] + v[n / 2]) / 2.
        } else {
            v[n / 2]
        }
    }
    let mut seed = 0x4c6f63616c4c4d31u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut changes = Vec::new();
    for _ in 0..2000 {
        let mut a = (0..before.len())
            .map(|_| before[next() as usize % before.len()])
            .collect::<Vec<_>>();
        let mut b = (0..after.len())
            .map(|_| after[next() as usize % after.len()])
            .collect::<Vec<_>>();
        let baseline = median(&mut a);
        if baseline > 0. {
            changes.push((median(&mut b) - baseline) / baseline * 100.);
        }
    }
    if changes.is_empty() {
        return json!({"available":false,"reason":"zero_baseline"});
    }
    changes.sort_by(f64::total_cmp);
    let n = changes.len();
    json!({"available":true,"method":"deterministic_percentile_bootstrap","resamples":2000,"confidence":0.95,"lowerPercent":changes[(n as f64*0.025).floor()as usize],"upperPercent":changes[((n as f64*0.975).ceil()as usize).min(n-1)],"causalityEstablished":false})
}
pub fn registry() -> Vec<crate::tool_spec::ToolSpec> {
    let id = json!({"type":"string","minLength":1,"maxLength":128});
    vec![("profile_capture".into(),"Capture an approved workspace Node program through its owned loopback inspector, or an installed native program using configured perf/WPR/xctrace tooling. Returns original export plus normalized profile evidence. Does not install tools or change profiling permissions. Native export weighting and symbolization are disclosed.".into(),dev::schema(json!({"kind":{"type":"string","enum":["node","native"]},"program":{"type":"string","minLength":1,"maxLength":4096},"arguments":{"type":"array","maxItems":64,"items":{"type":"string","maxLength":8192}},"cwd":{"type":"string","maxLength":4096},"duration_seconds":{"type":"integer","minimum":1,"maximum":120},"source_paths":{"type":"array","minItems":1,"maxItems":128,"items":{"type":"string","maxLength":4096}}}),&["program","source_paths"])),("profile_compare".into(),"Compare two owned captures by symbol/file identity. Reject incompatible profiler units/settings/workloads and qualify missing symbols or stale source. Profile evidence is not proof of causal speedup.".into(),dev::schema(json!({"before_id":id,"after_id":id}),&["before_id","after_id"]))].into_iter().map(crate::tool_spec::ToolSpec::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[tokio::test]
    async fn requested_duration_flushes_owned_recorder_and_accepts_early_exit() {
        let t = tempfile::tempdir().unwrap();
        let config = crate::execution::ExecutionConfig::default();
        let mut cmd = command(&config.python_path, t.path(), &config);
        cmd.args(["-c", "import signal,time,pathlib\ndef stop(*args):\n pathlib.Path('flushed').write_text('complete');raise SystemExit(0)\nsignal.signal(signal.SIGINT,stop)\nwhile True:time.sleep(.01)"]);
        let lifecycle = record_for(cmd, 1).await.unwrap();
        assert_eq!(lifecycle["durationLimitReached"], true);
        assert_eq!(
            std::fs::read_to_string(t.path().join("flushed")).unwrap(),
            "complete"
        );
        let mut cmd = command(&config.python_path, t.path(), &config);
        cmd.args(["-c", "pass"]);
        assert_eq!(record_for(cmd, 2).await.unwrap()["earlyWorkloadExit"], true);
        let mut cmd = command(&config.python_path, t.path(), &config);
        cmd.args(["-c", "raise SystemExit(7)"]);
        assert!(record_for(cmd, 2).await.is_err());
        let mut cmd = command(&config.python_path, t.path(), &config);
        cmd.args(["-c", "import signal,time,pathlib\ndef stop(*args):\n pathlib.Path('cancel-flushed').write_text('complete');raise SystemExit(0)\nsignal.signal(signal.SIGINT,stop)\npathlib.Path('ready').write_text('ready')\nwhile True:time.sleep(.01)"]);
        let task = tokio::spawn(record_for(cmd, 30));
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !t.path().join("ready").exists() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        task.abort();
        let _ = task.await;
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while !t.path().join("cancel-flushed").exists() {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    #[ignore = "Requires explicitly configured authorized native profiler host and compiler; never changes permissions."]
    async fn real_native_profiler_capture() {
        let t = tempfile::tempdir().unwrap();
        let source = t.path().join("cpu.c");
        std::fs::write(&source, "int main(void) { volatile unsigned long long x=0; for(;;) { x=x*1664525+1013904223; } }\n").unwrap();
        let binary = t.path().join(if cfg!(windows) { "cpu.exe" } else { "cpu" });
        let cc = std::env::var("LOCALLM_CC").expect("Configure actual installed C compiler");
        assert!(std::process::Command::new(cc)
            .args(["-g", "-O0"])
            .arg(&source)
            .arg("-o")
            .arg(&binary)
            .status()
            .unwrap()
            .success());
        let state = crate::test_support::app_state(t.path());
        let owner = state.database().unwrap().create_conversation().unwrap().id;
        let settings = Config {
            native_executable: std::env::var("LOCALLM_NATIVE_PROFILER")
                .expect("Configure native profiler"),
            export_executable: std::env::var("LOCALLM_PROFILE_EXPORTER").unwrap_or_default(),
            export_profile: std::env::var("LOCALLM_WPA_PROFILE").unwrap_or_default(),
        };
        state
            .database()
            .unwrap()
            .save_setting("development.profilers", &settings)
            .unwrap();
        let ctx = crate::development_tools::tests::context(&state, t.path(), &owner, &[]);
        let result = capture(
            &ctx,
            json!({"kind":"native","program":binary,"source_paths":["cpu.c"],"duration_seconds":1}),
        )
        .await
        .unwrap();
        assert_eq!(result["complete"], true, "{result}");
        assert!(result["originalTraceId"].is_string());
        assert!(!result["profile"]["profiles"][0]["samples"]
            .as_array()
            .unwrap()
            .is_empty());
    }
    #[test]
    fn native_normalizers_preserve_samples_weights_and_references() {
        let perf=normalize_perf("app 1 123: cycles:\n\t100 leaf+0x2 (/app)\n\t200 root+0x1 (/app)\n\napp 1 124: cycles:\n\t100 leaf+0x2 (/app)\n\t200 root+0x1 (/app)\n\n").unwrap();
        assert_eq!(perf["profiles"][0]["samples"].as_array().unwrap().len(), 2);
        assert_eq!(perf["shared"]["frames"][0]["name"], "root (/app)");
        assert_eq!(hotspots(&perf).unwrap().1["leaf (/app)@?"], 2.);
        let weighted = normalize_perf(
            "app 1 123.0: 1000000 cpu-clock:\n\t100 leaf+0x1 (/app)\n\t200 root+0x1 (/app)\n\n",
        )
        .unwrap();
        assert_eq!(weighted["profiles"][0]["unit"], "nanoseconds");
        assert_eq!(hotspots(&weighted).unwrap().1["leaf (/app)@?"], 1000000.);
        let wpa = normalize_wpa(b"Function,Weight (ms)\nhot,3.5\ncold,0.5\n").unwrap();
        assert_eq!(hotspots(&wpa).unwrap().1["hot@?"], 3.5);
        assert!(normalize_wpa(b"Function,Count\nhot,500\n").is_err());
        let xml = r#"<trace><row><backtrace id="b"><frame id="f" name="leaf"/><frame name="root"/></backtrace></row><row><backtrace ref="b"/></row></trace>"#;
        let trace = normalize_xctrace(xml).unwrap();
        assert_eq!(trace["profiles"][0]["samples"].as_array().unwrap().len(), 2);
        assert_eq!(hotspots(&trace).unwrap().1["leaf@?"], 2.);
        let weighted=normalize_xctrace(r#"<trace><row><backtrace><frame name="leaf"/><frame name="root"/></backtrace><weight unit="nanoseconds">2500000</weight></row></trace>"#).unwrap();
        assert_eq!(hotspots(&weighted).unwrap().1["leaf@?"], 2500000.);
        assert_eq!(weighted["profiles"][0]["unit"], "nanoseconds");
        assert!(
            normalize_xctrace(r#"<trace><row><backtrace ref="missing"/></row></trace>"#).is_err()
        );
        let wpa = normalize_wpa(
            b"Function,Weight (ms),Stack\nleaf,2.5,\"[\"\"root\"\",\"\"leaf\"\"]\"\n",
        )
        .unwrap();
        assert_eq!(
            wpa["profiles"][0]["samples"][0].as_array().unwrap().len(),
            2
        );
        assert_eq!(hotspots(&wpa).unwrap().1["leaf@?"], 2.5);
    }
    #[test]
    fn evented_profiles_and_bootstrap_reject_invalid_evidence() {
        assert!(normalize_evented(&json!({"profiles":[{"type":"evented","startValue":0,"events":[{"type":"C","at":1,"frame":0}]}]})).is_err());
        let v = bootstrap(&[10., 10., 10.], &[8., 8., 8.]);
        assert_eq!(v["lowerPercent"], -20.);
        assert_eq!(v["upperPercent"], -20.);
        assert_eq!(bootstrap(&[0., 0.], &[1., 1.])["available"], false);
    }
    #[tokio::test]
    #[ignore = "Requires installed Node and permitted loopback sockets; run explicitly."]
    async fn real_node_inspector_capture() {
        let t = tempfile::tempdir().unwrap();
        let program = t.path().join("cpu.cjs");
        std::fs::write(&program,"function hot(){let x=0;for(let i=0;i<100000;i++)x+=Math.sqrt(i);return x;}setInterval(hot,1);\n").unwrap();
        let config = crate::execution::ExecutionConfig::default();
        let original = node_capture(&config, t.path(), program.to_str().unwrap(), &[], 1)
            .await
            .unwrap();
        let profile = normalize_node(&original).unwrap();
        assert!(!profile["profiles"][0]["samples"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(profile["shared"]["frames"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["name"] == "hot"));
    }
}
