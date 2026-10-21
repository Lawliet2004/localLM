//! Evidence graph from language servers and package manifests; no completeness fiction.
use crate::development_tools as dev;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
fn flatten_symbols(items: &[Value], out: &mut Vec<Value>) {
    for item in items {
        out.push(item.clone());
        if out.len() >= 200 {
            break;
        }
        if let Some(children) = item["children"].as_array() {
            flatten_symbols(children, out);
        }
    }
}
#[path = "semantic_baseline.rs"]
mod baseline;
#[path = "semantic_dependencies.rs"]
mod dependencies;
pub async fn analyze(ctx: &crate::harness::HarnessCtx<'_>, args: &Value) -> Result<Value, String> {
    tokio::time::timeout(
        std::time::Duration::from_secs(120),
        analyze_bounded(ctx, args),
    )
    .await
    .map_err(|_| "Impact analysis exceeded 120 seconds; dependency coverage is incomplete.")?
}
async fn analyze_bounded(
    ctx: &crate::harness::HarnessCtx<'_>,
    args: &Value,
) -> Result<Value, String> {
    let root = &ctx.snapshot.workspace_path;
    let paths = args["changed_paths"]
        .as_array()
        .ok_or("Missing changed paths.")?;
    let workspace = crate::workspace::Workspace::open(root)?;
    let inventory = crate::source_inventory::inventory(root)?;
    let snapshot = crate::source_inventory::snapshot(
        root,
        inventory["paths"].as_array().ok_or("Missing inventory.")?,
        false,
    )?;
    let config = crate::lsp_client::load(&*ctx.state.database()?)?;
    let mut nodes = BTreeMap::new();
    let mut edges = Vec::new();
    let mut warnings = vec![json!({"reason":"external_consumers_and_dynamic_dispatch_unresolved"})];
    let baseline = match baseline::prepare(root).await {
        Ok(value) => {
            warnings.extend(value.warnings.clone());
            Some(value)
        }
        Err(error) => {
            warnings.push(json!({"reason":"baseline_unavailable","details":error}));
            None
        }
    };
    let baseline_servers = crate::lsp_client::Manager::default();
    let mut affected = BTreeSet::new();
    let mut tests = BTreeMap::<String, Vec<Value>>::new();
    let budget = std::time::Instant::now() + std::time::Duration::from_secs(45);
    let mut requests = 0usize;
    for path in paths {
        let path = path.as_str().ok_or("Invalid changed path.")?;
        affected.insert(path.to_string());
        let text = workspace.language_optional_source(path)?;
        let previous = baseline.as_ref().and_then(|b| b.files.get(path)).cloned();
        nodes.insert(path.to_string(),json!({"path":path,"kind":"file","sha256":text.as_ref().map(|v|dev::hash(v.as_bytes())),"baselineSha256":previous.as_ref().map(|v|dev::hash(v.as_bytes())),"deleted":text.is_none()}));
        let language = if path.ends_with(".rs") {
            "rust"
        } else if ["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts"]
            .contains(&path.rsplit('.').next().unwrap_or(""))
        {
            "typescript"
        } else {
            warnings.push(json!({"path":path,"reason":"unsupported_language"}));
            continue;
        };
        let server = config.servers.iter().find(|s| {
            s.enabled
                && config.enabled
                && (args["server"].as_str().is_some_and(|id| id == s.id)
                    || args["server"].is_null()
                        && (s.language_id == language
                            || language == "typescript" && s.language_id.starts_with("javascript")))
        });
        let Some(server) = server else {
            warnings.push(json!({"path":path,"reason":"language_server_unavailable"}));
            continue;
        };
        for (evidence_version, analysis_root, text, manager) in [
            (
                "current",
                root.as_str(),
                text.as_ref(),
                &ctx.state.language_servers,
            ),
            (
                "baseline",
                baseline
                    .as_ref()
                    .and_then(|b| b.directory.path().to_str())
                    .unwrap_or(""),
                previous.as_ref(),
                &baseline_servers,
            ),
        ] {
            let Some(text) = text else {
                continue;
            };
            let mut isolated_server = server.clone();
            if evidence_version == "baseline" {
                let target = std::path::Path::new(analysis_root)
                    .join("target")
                    .to_string_lossy()
                    .into_owned();
                isolated_server
                    .environment
                    .insert("CARGO_TARGET_DIR".into(), target.clone());
                isolated_server
                    .environment
                    .insert("CARGO_NET_OFFLINE".into(), "true".into());
                rebase_settings(&mut isolated_server.settings, root, analysis_root);
                rebase_settings(
                    &mut isolated_server.initialization_options,
                    root,
                    analysis_root,
                );
                if language == "typescript" {
                    if isolated_server.initialization_options.is_null() {
                        isolated_server.initialization_options = json!({});
                    }
                    if let Some(options) = isolated_server.initialization_options.as_object_mut() {
                        options.insert("disableAutomaticTypingAcquisition".into(), json!(true));
                    }
                }
            }
            let server = &isolated_server;
            let sha = dev::hash(text.as_bytes());
            let uri = reqwest::Url::from_file_path(std::path::Path::new(analysis_root).join(path))
                .map_err(|_| "Invalid source URI.")?
                .to_string();
            if requests >= 64 || std::time::Instant::now() >= budget {
                warnings.push(json!({"reason":"analysis_budget"}));
                break;
            }
            requests += 1;
            let symbols = manager
                .request(
                    &ctx.conversation_id,
                    analysis_root,
                    server,
                    Some((&uri, text, &sha)),
                    "textDocument/documentSymbol",
                    json!({"textDocument":{"uri":uri}}),
                )
                .await;
            let symbols = match symbols {
                Ok(r) => r.result,
                Err(e) => {
                    warnings.push(json!({"path":path,"reason":"symbol_query_failed","details":e}));
                    continue;
                }
            };
            let mut flat = Vec::new();
            flatten_symbols(
                symbols.as_array().map(Vec::as_slice).unwrap_or(&[]),
                &mut flat,
            );
            if flat.len() >= 200 {
                warnings.push(json!({"path":path,"reason":"symbol_limit"}));
            }
            for symbol in flat {
                if requests >= 64 || std::time::Instant::now() >= budget {
                    warnings.push(json!({"reason":"analysis_budget"}));
                    break;
                }
                if let Some(names) = args["symbols"].as_array() {
                    if !names.is_empty() && !names.iter().any(|n| *n == symbol["name"]) {
                        continue;
                    }
                }
                let position = symbol
                    .get("selectionRange")
                    .unwrap_or(&symbol["location"]["range"])["start"]
                    .clone();
                if !position.is_object() {
                    warnings.push(json!({"reason":"invalid_symbol_position"}));
                    continue;
                }
                requests += 1;
                let refs=manager.request(&ctx.conversation_id,analysis_root,server,Some((&uri,text,&sha)),"textDocument/references",json!({"textDocument":{"uri":uri},"position":position,"context":{"includeDeclaration":false}})).await;
                match refs {
                    Ok(r) => {
                        for loc in r.result.as_array().into_iter().flatten() {
                            if let Some(uri) = loc["uri"].as_str() {
                                if let Ok(target) = crate::lsp_tools::uri_path(
                                    std::path::Path::new(analysis_root),
                                    uri,
                                ) {
                                    affected.insert(target.clone());
                                    edges.push(json!({"from":target,"to":path,"kind":"symbol_reference","symbol":symbol["name"],"range":loc["range"],"evidence":"language_server","evidenceVersion":evidence_version,"baselineCommit":baseline.as_ref().map(|b| &b.commit),"serverFingerprint":crate::lsp_client::fingerprint(server),"sourceSha256":sha}));
                                } else {
                                    warnings.push(json!({"reason":"external_consumer_omitted"}));
                                }
                            }
                        }
                    }
                    Err(e) => warnings
                        .push(json!({"path":path,"reason":"reference_query_failed","details":e})),
                }
                if requests < 60 && std::time::Instant::now() < budget {
                    requests += 1;
                    match manager.request(&ctx.conversation_id,analysis_root,server,Some((&uri,text,&sha)),"textDocument/implementation",json!({"textDocument":{"uri":uri},"position":position})).await {
                    Ok(reply)=>{for location in reply.result.as_array().into_iter().flatten(){let target_uri=location["uri"].as_str().or_else(||location["targetUri"].as_str());if let Some(target_uri)=target_uri {if let Ok(target)=crate::lsp_tools::uri_path(std::path::Path::new(analysis_root),target_uri){affected.insert(target.clone());edges.push(json!({"from":target,"to":path,"kind":"symbol_implementation","symbol":symbol["name"],"evidence":"language_server","evidenceVersion":evidence_version,"baselineCommit":baseline.as_ref().map(|b| &b.commit),"serverFingerprint":crate::lsp_client::fingerprint(server),"sourceSha256":sha}));}}}},
                    Err(error)=>warnings.push(json!({"path":path,"reason":"implementation_analysis_unavailable","details":error}))
                }
                    requests += 1;
                    match manager
                    .request(
                        &ctx.conversation_id,
                        analysis_root,
                        server,
                        Some((&uri, text, &sha)),
                        "textDocument/prepareCallHierarchy",
                        json!({"textDocument":{"uri":uri},"position":position}),
                    )
                    .await
                {
                    Ok(reply) => {
                        let mut queue = reply.result.as_array().into_iter().flatten().take(4).cloned().map(|v| (v, 0usize)).collect::<std::collections::VecDeque<_>>();
                        let mut visited = BTreeSet::new();
                        while let Some((item, depth)) = queue.pop_front() {
                            if !visited.insert(item.to_string()) { continue; }
                            let origin = item["uri"].as_str().and_then(|uri| crate::lsp_tools::uri_path(std::path::Path::new(analysis_root), uri).ok()).unwrap_or_else(|| path.to_string());
                            for (method, field, kind) in [
                                ("callHierarchy/incomingCalls", "from", "caller"),
                                ("callHierarchy/outgoingCalls", "to", "callee"),
                            ] {
                                if requests >= 64 || std::time::Instant::now() >= budget {
                                    warnings.push(json!({"reason":"analysis_budget"}));
                                    break;
                                }
                                requests += 1;
                                match manager.request(&ctx.conversation_id,analysis_root,server,None,method,json!({"item":item})).await {
                            Ok(reply) => for call in reply.result.as_array().into_iter().flatten().take(200) {
                                if let Some(target_uri) = call[field]["uri"].as_str() {
                                    if let Ok(target) = crate::lsp_tools::uri_path(std::path::Path::new(analysis_root), target_uri) {
                                        affected.insert(target.clone());
                                        edges.push(json!({"from":if kind=="caller"{target.as_str()}else{origin.as_str()},"to":if kind=="caller"{origin.as_str()}else{target.as_str()},"kind":kind,"symbol":item["name"],"ranges":call["fromRanges"],"evidence":"language_server","evidenceVersion":evidence_version,"baselineCommit":baseline.as_ref().map(|b| &b.commit),"serverFingerprint":crate::lsp_client::fingerprint(server),"sourceSha256":sha,"transitiveDepth":depth}));
                                        if depth < 3 && queue.len() < 128 { queue.push_back((call[field].clone(), depth + 1)); }
                                        else { warnings.push(json!({"reason":"transitive_expansion_budget"})); }
                                    } else { warnings.push(json!({"reason":"external_consumer_omitted"})); }
                                }
                            },
                            Err(error)=>warnings.push(json!({"path":path,"reason":"call_analysis_failed","details":error}))
                        }
                            }
                        }
                    }
                    Err(error) => warnings.push(
                        json!({"path":path,"reason":"call_hierarchy_unavailable","details":error}),
                    ),
                }
                }
            }
        }
    }
    let mut cargo = BTreeMap::new();
    for path in inventory["paths"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|p| p.ends_with("Cargo.toml"))
    {
        if let Ok(text) = workspace.language_source(path) {
            if let Ok(value) = toml::from_str::<toml::Value>(&text) {
                cargo.insert(path.to_string(), value);
            }
        }
    }
    for (path, value) in &cargo {
        let (dependencies, unresolved) = cargo_dependencies(path, value, &cargo);
        for target in dependencies {
            edges.push(json!({"from":path,"to":target,"kind":"package_dependency","evidence":"manifest_declaration","evidenceVersion":"current","sourceSha256":snapshot["files"][path]}));
        }
        warnings.extend(
            unresolved
                .into_iter()
                .map(|reason| json!({"path":path,"reason":reason})),
        );
    }
    // Manifest relationships are exact declarations; propagation through them is conservative.
    for p in inventory["paths"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|p| {
            p.ends_with("Cargo.toml") || p.ends_with("package.json") || p.ends_with("tsconfig.json")
        })
    {
        let text = workspace.language_source(p)?;
        let manifest_sha = dev::hash(text.as_bytes());
        if p.ends_with("Cargo.toml") {
            if let Ok(value) = toml::from_str::<toml::Value>(&text) {
                for relative in value
                    .get("workspace")
                    .and_then(|v| v.get("members"))
                    .and_then(toml::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(toml::Value::as_str)
                {
                    for target in inventory["paths"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .filter(|v| v.ends_with("Cargo.toml"))
                    {
                        let parent = std::path::Path::new(p)
                            .parent()
                            .unwrap_or(std::path::Path::new(""));
                        let pattern = parent
                            .join(relative)
                            .join("Cargo.toml")
                            .to_string_lossy()
                            .into_owned();
                        if glob::Pattern::new(&pattern).is_ok_and(|g| g.matches(target)) {
                            edges.push(json!({"from":p,"to":target,"kind":"workspace_membership","evidence":"manifest_declaration","sourceSha256":manifest_sha}));
                        }
                    }
                }
            } else {
                warnings.push(json!({"path":p,"reason":"invalid_manifest"}));
            }
        } else if let Ok(value) = json_with_comments(&text) {
            for reference in value["references"].as_array().into_iter().flatten() {
                if let Some(relative) = reference["path"].as_str() {
                    if let Some(target) = normalize(
                        &std::path::Path::new(p)
                            .parent()
                            .unwrap_or(std::path::Path::new(""))
                            .join(if relative.ends_with(".json") {
                                relative.to_string()
                            } else {
                                format!("{relative}/tsconfig.json")
                            }),
                    ) {
                        edges.push(json!({"from":p,"to":target,"kind":"project_reference","evidence":"manifest_declaration","sourceSha256":manifest_sha}));
                    }
                }
            }
        }
        if p.ends_with("tsconfig.json") && json_with_comments(&text).is_err() {
            warnings.push(json!({"path":p,"reason":"invalid_typescript_config"}));
        }
        nodes.insert(
            p.into(),
            json!({"path":p,"kind":"manifest","sha256":manifest_sha}),
        );
    }
    let mut current_texts = BTreeMap::new();
    for path in snapshot["files"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(p, _)| p)
    {
        if let Ok(text) = workspace.language_source(path) {
            current_texts.insert(path.clone(), text);
        }
    }
    let (module_edges, module_warnings) = dependencies::imports(&current_texts);
    edges.extend(module_edges);
    warnings.extend(module_warnings);
    let mut packages = BTreeMap::new();
    for p in nodes.keys().filter(|p| p.ends_with("package.json")) {
        if let Ok(text) = workspace.language_source(p) {
            match serde_json::from_str::<Value>(&text) {
                Ok(value) => {
                    if let Some(name) = value["name"].as_str() {
                        packages.insert(name.to_string(), (p.clone(), value));
                    }
                }
                Err(_) => warnings.push(json!({"path":p,"reason":"invalid_package_manifest"})),
            }
        }
    }
    for (p, value) in packages.values() {
        for section in [
            "dependencies",
            "devDependencies",
            "peerDependencies",
            "optionalDependencies",
        ] {
            for (name, version) in value[section].as_object().into_iter().flatten() {
                if let Some(relative) = version
                    .as_str()
                    .and_then(|v| v.strip_prefix("file:").or_else(|| v.strip_prefix("link:")))
                {
                    if let Some(target) = normalize(
                        &std::path::Path::new(p)
                            .parent()
                            .unwrap_or(std::path::Path::new(""))
                            .join(relative)
                            .join("package.json"),
                    ) {
                        if nodes.contains_key(&target) {
                            edges.push(json!({"from":p,"to":target,"kind":"local_package_dependency","evidence":"manifest_declaration","sourceSha256":nodes[p]["sha256"]}));
                        } else {
                            warnings.push(json!({"path":p,"reason":"local_package_manifest_missing","target":target}));
                        }
                    } else {
                        warnings.push(json!({"path":p,"reason":"external_local_package"}));
                    }
                }
                if let Some((target, _)) = packages.get(name) {
                    edges.push(json!({"from":p,"to":target,"kind":"package_dependency","evidence":"manifest_declaration","sourceSha256":nodes[p]["sha256"]}));
                }
            }
        }
    }
    loop {
        let before = affected.len();
        for edge in &edges {
            let from = edge["from"].as_str().unwrap_or("");
            let to = edge["to"].as_str().unwrap_or("");
            if affected.contains(to) {
                affected.insert(from.into());
            }
        }
        // Any affected file marks its containing package as conservatively affected.
        for node in nodes.values().filter(|v| v["kind"] == "manifest") {
            let p = node["path"].as_str().unwrap_or("");
            let parent = std::path::Path::new(p)
                .parent()
                .unwrap_or(std::path::Path::new(""))
                .to_string_lossy();
            if affected.iter().any(|a| in_directory(a, parent.as_ref())) {
                affected.insert(p.into());
            }
        }
        if affected.len() == before {
            break;
        }
    }
    for p in inventory["paths"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if p.contains(".test.")
            || p.contains(".spec.")
            || p.starts_with("tests/")
            || p.contains("/tests/")
        {
            if affected.contains(p) {
                tests
                    .entry(p.into())
                    .or_default()
                    .push(json!({"reason":"semantic_reference"}));
            } else if nodes
                .values()
                .filter(|n| n["kind"] == "manifest")
                .filter_map(|n| n["path"].as_str())
                .any(|m| {
                    affected.contains(m)
                        && in_directory(
                            p,
                            std::path::Path::new(m)
                                .parent()
                                .unwrap_or(std::path::Path::new(""))
                                .to_string_lossy()
                                .as_ref(),
                        )
                })
            {
                tests
                    .entry(p.into())
                    .or_default()
                    .push(json!({"reason":"affected_package","certainty":"conservative"}));
            }
        }
    }
    let package_roots = nodes
        .values()
        .filter(|n| n["kind"] == "manifest")
        .filter_map(|n| n["path"].as_str())
        .filter(|m| affected.contains(*m))
        .map(|m| {
            std::path::Path::new(m)
                .parent()
                .unwrap_or(std::path::Path::new(""))
                .to_string_lossy()
                .into_owned()
        })
        .collect::<Vec<_>>();
    for p in inventory["paths"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if package_roots.iter().any(|root| in_directory(p, root)) {
            affected.insert(p.into());
        }
    }
    if !warnings.is_empty() || inventory["complete"] != true || snapshot["complete"] != true {
        affected.extend(
            inventory["paths"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_string),
        );
    }
    for edge in &mut edges {
        let version = edge["evidenceVersion"]
            .as_str()
            .unwrap_or("current")
            .to_string();
        edge["evidenceVersion"] = json!(version);
        for (path_key, hash_key) in [("from", "fromSourceSha256"), ("to", "toSourceSha256")] {
            let path = edge[path_key].as_str().unwrap_or("");
            let fingerprint = if version == "baseline" {
                baseline
                    .as_ref()
                    .and_then(|b| b.files.get(path))
                    .map(|s| json!(dev::hash(s.as_bytes())))
                    .unwrap_or(Value::Null)
            } else {
                snapshot["files"][path].clone()
            };
            edge[hash_key] = fingerprint;
        }
    }
    if edges.len() > 4096 {
        edges.truncate(4096);
        warnings.push(json!({"reason":"edge_budget_exhausted","limit":4096}));
    }
    baseline_servers.shutdown().await;
    let baseline_freshness = baseline.as_ref().map(|baseline| {
        let files = baseline
            .files
            .iter()
            .map(|(p, t)| (p.clone(), json!(dev::hash(t.as_bytes()))))
            .collect::<serde_json::Map<_, _>>();
        dev::freshness(
            baseline.directory.path().to_str().unwrap_or(""),
            &json!({"files":files}),
        )
    });
    if baseline_freshness
        .as_ref()
        .is_some_and(|v| v["matches"] != true)
    {
        warnings.push(json!({"reason":"baseline_changed_during_analysis"}));
    }
    let freshness = dev::freshness(root, &snapshot);
    if freshness["matches"] != true {
        warnings.push(json!({"reason":"source_changed_during_analysis"}));
    }
    Ok(
        json!({"analysisKind":"semantic_with_conservative_package_propagation","baselineCommit":baseline.as_ref().map(|b| &b.commit),"baselineFreshness":baseline_freshness,"analysisBudgets":{"baselinePreparationSeconds":30,"totalSeconds":120,"requests":64,"sourceFiles":4096,"sourceBytes":67108864,"transitiveDepth":3,"transitiveQueue":128,"outputEdges":4096},"requestsUsed":requests,"nodes":nodes.values().collect::<Vec<_>>(),"edges":edges,"affectedPaths":affected,"candidateTests":tests,"sourceSnapshot":snapshot,"freshness":freshness,"incomplete":!warnings.is_empty()||inventory["complete"]!=true||snapshot["complete"]!=true,"warnings":warnings,"completeDependencyGraph":false,"limitations":["External consumers and runtime dispatch are not fully observable.","Baseline symbols are analyzed conservatively, including removed symbols and deleted files; absent baselines remain explicit.","Package propagation is conservative; candidate tests are not automatically run."]}),
    )
}
fn rebase_settings(value: &mut Value, current: &str, baseline: &str) {
    match value {
        Value::String(text) => {
            if let Ok(relative) = std::path::Path::new(text).strip_prefix(current) {
                *text = std::path::Path::new(baseline)
                    .join(relative)
                    .to_string_lossy()
                    .into_owned();
            } else if let Ok(url) = reqwest::Url::parse(text) {
                if let Ok(path) = url.to_file_path() {
                    if let Ok(relative) = path.strip_prefix(current) {
                        if let Ok(uri) = reqwest::Url::from_file_path(
                            std::path::Path::new(baseline).join(relative),
                        ) {
                            *text = uri.to_string();
                        }
                    }
                }
            }
        }
        Value::Object(map) => {
            for value in map.values_mut() {
                rebase_settings(value, current, baseline);
            }
        }
        Value::Array(items) => {
            for value in items {
                rebase_settings(value, current, baseline);
            }
        }
        _ => (),
    }
}
fn cargo_dependencies(
    path: &str,
    value: &toml::Value,
    manifests: &BTreeMap<String, toml::Value>,
) -> (BTreeSet<String>, Vec<String>) {
    let parent = std::path::Path::new(path)
        .parent()
        .unwrap_or(std::path::Path::new(""));
    let workspace = manifests
        .iter()
        .filter(|(p, v)| {
            v.get("workspace").is_some()
                && in_directory(
                    path,
                    std::path::Path::new(p)
                        .parent()
                        .unwrap_or(std::path::Path::new(""))
                        .to_str()
                        .unwrap_or(""),
                )
        })
        .max_by_key(|(p, _)| p.len());
    let mut tables = vec![value];
    if let Some(targets) = value.get("target").and_then(toml::Value::as_table) {
        tables.extend(targets.values());
    }
    let mut dependencies = BTreeSet::new();
    let mut warnings = Vec::new();
    for table in tables {
        for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
            for (name, declared) in table
                .get(section)
                .and_then(toml::Value::as_table)
                .into_iter()
                .flatten()
            {
                let (base, dependency) =
                    if declared.get("workspace").and_then(toml::Value::as_bool) == Some(true) {
                        if let Some((manifest, workspace)) = workspace {
                            if let Some(dependency) = workspace
                                .get("workspace")
                                .and_then(|v| v.get("dependencies"))
                                .and_then(|v| v.get(name))
                            {
                                (
                                    std::path::Path::new(manifest)
                                        .parent()
                                        .unwrap_or(std::path::Path::new("")),
                                    dependency,
                                )
                            } else {
                                warnings.push(format!("unresolved_workspace_dependency:{name}"));
                                continue;
                            }
                        } else {
                            warnings.push(format!("workspace_manifest_missing:{name}"));
                            continue;
                        }
                    } else {
                        (parent, declared)
                    };
                if let Some(relative) = dependency.get("path").and_then(toml::Value::as_str) {
                    if let Some(target) = normalize(&base.join(relative).join("Cargo.toml")) {
                        if manifests.contains_key(&target) {
                            dependencies.insert(target);
                        } else {
                            warnings.push(format!("local_dependency_manifest_missing:{target}"));
                        }
                    } else {
                        warnings.push(format!("external_path_dependency:{name}"));
                    }
                }
            }
        }
    }
    (dependencies, warnings)
}
fn normalize(path: &std::path::Path) -> Option<String> {
    let mut parts = Vec::new();
    for c in path.components() {
        match c {
            std::path::Component::Normal(p) => parts.push(p.to_string_lossy().into_owned()),
            std::path::Component::ParentDir => {
                parts.pop()?;
            }
            std::path::Component::CurDir => (),
            _ => return None,
        }
    }
    Some(parts.join("/"))
}

fn in_directory(path: &str, directory: &str) -> bool {
    directory.is_empty()
        || path == directory
        || path
            .strip_prefix(directory)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn json_with_comments(text: &str) -> Result<Value, String> {
    let mut bytes = text.as_bytes().to_vec();
    let mut i = 0;
    let mut quoted = false;
    while i < bytes.len() {
        if quoted {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == b'"' {
                quoted = false;
            }
            i += 1;
            continue;
        }
        if bytes[i] == b'"' {
            quoted = true;
            i += 1;
            continue;
        }
        if bytes.get(i..i + 2) == Some(b"//") {
            while i < bytes.len() && bytes[i] != b'\n' {
                bytes[i] = b' ';
                i += 1;
            }
        } else if bytes.get(i..i + 2) == Some(b"/*") {
            bytes[i] = b' ';
            bytes[i + 1] = b' ';
            i += 2;
            while i + 1 < bytes.len() && bytes.get(i..i + 2) != Some(b"*/") {
                if bytes[i] != b'\n' {
                    bytes[i] = b' ';
                }
                i += 1;
            }
            if i + 1 >= bytes.len() {
                return Err("Unterminated config comment.".into());
            }
            bytes[i] = b' ';
            bytes[i + 1] = b' ';
            i += 2;
        } else {
            i += 1;
        }
    }
    i = 0;
    quoted = false;
    while i < bytes.len() {
        if quoted {
            if bytes[i] == b'\\' {
                i += 2;
                continue;
            }
            if bytes[i] == b'"' {
                quoted = false;
            }
        } else if bytes[i] == b'"' {
            quoted = true;
        } else if bytes[i] == b','
            && bytes[i + 1..]
                .iter()
                .find(|b| !b.is_ascii_whitespace())
                .is_some_and(|b| *b == b'}' || *b == b']')
            {
                bytes[i] = b' ';
            }
        i += 1;
    }
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn deleted_files_and_removed_symbols_use_isolated_baseline_sessions() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("deleted.rs"), "pub fn deleted_symbol() {}\n").unwrap();
        std::fs::write(
            t.path().join("surviving.rs"),
            "pub fn removed_symbol() {}\npub fn survives() {}\n",
        )
        .unwrap();
        std::fs::write(
            t.path().join("consumer.rs"),
            "fn consumer() { deleted_symbol(); removed_symbol(); }\n",
        )
        .unwrap();
        std::fs::write(
            t.path().join("indirect.rs"),
            "fn indirect() { consumer(); }\n",
        )
        .unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "."],
            vec![
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "baseline",
            ],
        ] {
            assert!(std::process::Command::new("git")
                .arg("-C")
                .arg(t.path())
                .args(args)
                .status()
                .unwrap()
                .success());
        }
        std::fs::remove_file(t.path().join("deleted.rs")).unwrap();
        std::fs::write(t.path().join("surviving.rs"), "pub fn survives() {}\n").unwrap();
        let index = std::fs::read(t.path().join(".git/index")).unwrap();
        let state = crate::test_support::app_state(t.path());
        let owner = state.database().unwrap().create_conversation().unwrap().id;
        let server = crate::lsp_client::ServerConfig {
            id: "impact-fixture".into(),
            language_id: "rust".into(),
            enabled: true,
            command: crate::execution::ExecutionConfig::default().python_path,
            arguments: vec![
                format!(
                    "{}/tests/fixtures/lsp_fixture.py",
                    env!("CARGO_MANIFEST_DIR")
                ),
                "impact".into(),
                t.path().join("lsp.log").to_string_lossy().into(),
            ],
            environment: Default::default(),
            initialization_options: Value::Null,
            settings: json!({}),
            timeout_seconds: 2,
            allowed_commands: vec![],
        };
        state
            .database()
            .unwrap()
            .save_setting(
                crate::lsp_client::KEY,
                &crate::lsp_client::Config {
                    enabled: true,
                    always_ask: false,
                    servers: vec![server],
                },
            )
            .unwrap();
        let ctx = crate::development_tools::tests::context(&state, t.path(), &owner, &[]);
        let graph = analyze(
            &ctx,
            &json!({"changed_paths":["deleted.rs","surviving.rs"]}),
        )
        .await
        .unwrap();
        for name in ["deleted_symbol", "removed_symbol"] {
            assert!(
                graph["edges"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|e| e["symbol"] == name
                        && e["from"] == "consumer.rs"
                        && e["evidenceVersion"] == "baseline"
                        && e["serverFingerprint"].is_string()
                        && e["fromSourceSha256"].is_string()),
                "{graph}"
            );
        }
        assert!(
            graph["edges"]
                .as_array()
                .unwrap()
                .iter()
                .any(|e| e["kind"] == "caller"
                    && e["from"] == "indirect.rs"
                    && e["to"] == "consumer.rs"
                    && e["transitiveDepth"].as_u64().is_some_and(|d| d >= 1)),
            "{graph}"
        );
        assert!(graph["baselineCommit"].is_string());
        assert_eq!(graph["completeDependencyGraph"], false);
        assert_eq!(std::fs::read(t.path().join(".git/index")).unwrap(), index);
        assert!(!t.path().join("deleted.rs").exists());
        assert_eq!(
            std::fs::read_to_string(t.path().join("surviving.rs")).unwrap(),
            "pub fn survives() {}\n"
        );
        state.language_servers.shutdown().await;
    }
    #[test]
    fn cargo_workspace_inheritance_includes_target_specific_dependencies() {
        let parse = |s: &str| toml::from_str::<toml::Value>(s).unwrap();
        let manifests = BTreeMap::from([
            ("Cargo.toml".into(), parse("[workspace]\nmembers=['app','lib']\n[workspace.dependencies]\nshared={path='lib',package='shared-lib'}\n")),
            ("app/Cargo.toml".into(), parse("[package]\nname='app'\nversion='0.1.0'\n[target.'cfg(windows)'.dependencies]\nshared.workspace=true\n")),
            ("lib/Cargo.toml".into(), parse("[package]\nname='shared-lib'\nversion='0.1.0'\n")),
        ]);
        let (dependencies, warnings) =
            cargo_dependencies("app/Cargo.toml", &manifests["app/Cargo.toml"], &manifests);
        assert_eq!(dependencies, BTreeSet::from(["lib/Cargo.toml".into()]));
        assert!(warnings.is_empty());
    }
    #[test]
    fn jsonc_and_package_boundaries() {
        let config=json_with_comments("{ // comment\n \"url\": \"https://example.com/x//y\", \"references\": [{\"path\":\"../base\"},], /* block */ }").unwrap();
        assert_eq!(config["url"], "https://example.com/x//y");
        assert_eq!(config["references"][0]["path"], "../base");
        assert!(!in_directory("src-old/a.rs", "src"));
        assert!(in_directory("src/a.rs", "src"));
    }
}

#[cfg(test)]
mod acceptance {
    use super::*;
    async fn real_impact(language: &str, executable: String) {
        let t = tempfile::tempdir().unwrap();
        let (changed, consumer, survivor) = if language == "rust" {
            std::fs::create_dir(t.path().join("src")).unwrap();
            std::fs::write(
                t.path().join("Cargo.toml"),
                "[package]\nname='impact_acceptance'\nversion='0.1.0'\nedition='2021'\n",
            )
            .unwrap();
            std::fs::write(t.path().join("src/lib.rs"),"pub mod removed; pub mod consumer; pub fn removed_symbol() {} pub fn survives() {}\n").unwrap();
            std::fs::write(t.path().join("src/removed.rs"), "pub fn erased() {}\n").unwrap();
            std::fs::write(
                t.path().join("src/consumer.rs"),
                "pub fn caller() { crate::removed::erased(); crate::removed_symbol(); }\n",
            )
            .unwrap();
            ("src/removed.rs", "src/consumer.rs", "src/lib.rs")
        } else {
            std::fs::write(
                t.path().join("tsconfig.json"),
                r#"{"compilerOptions":{"strict":true},"include":["*.ts"]}"#,
            )
            .unwrap();
            std::fs::write(t.path().join("removed.ts"), "export function erased() {}\n").unwrap();
            std::fs::write(
                t.path().join("surviving.ts"),
                "export function removed_symbol() {} export function survives() {}\n",
            )
            .unwrap();
            std::fs::write(t.path().join("consumer.ts"),"import {erased} from './removed'; import {removed_symbol} from './surviving'; export function caller() { erased(); removed_symbol(); }\n").unwrap();
            ("removed.ts", "consumer.ts", "surviving.ts")
        };
        for args in [
            vec!["init", "-q"],
            vec!["add", "."],
            vec![
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "commit",
                "-qm",
                "baseline",
            ],
        ] {
            assert!(std::process::Command::new("git")
                .arg("-C")
                .arg(t.path())
                .args(args)
                .status()
                .unwrap()
                .success());
        }
        std::fs::remove_file(t.path().join(changed)).unwrap();
        std::fs::write(
            t.path().join(survivor),
            if language == "rust" {
                "pub mod consumer; pub fn survives() {}\n"
            } else {
                "export function survives() {}\n"
            },
        )
        .unwrap();
        let state = crate::test_support::app_state(t.path());
        let owner = state.database().unwrap().create_conversation().unwrap().id;
        let mut arguments = Vec::new();
        if language != "rust" {
            if let Ok(script) = std::env::var("LOCALLM_TYPESCRIPT_LANGUAGE_SERVER_SCRIPT") {
                arguments.push(script);
            }
            arguments.push("--stdio".into());
        }
        let server = crate::lsp_client::ServerConfig {
            id: "real-impact".into(),
            language_id: language.into(),
            enabled: true,
            command: executable,
            arguments,
            environment: Default::default(),
            initialization_options: Value::Null,
            settings: json!({}),
            timeout_seconds: 30,
            allowed_commands: vec![],
        };
        state
            .database()
            .unwrap()
            .save_setting(
                crate::lsp_client::KEY,
                &crate::lsp_client::Config {
                    enabled: true,
                    always_ask: false,
                    servers: vec![server],
                },
            )
            .unwrap();
        let ctx = crate::development_tools::tests::context(&state, t.path(), &owner, &[]);
        let graph = analyze(&ctx, &json!({"changed_paths":[changed,survivor]}))
            .await
            .unwrap();
        for name in ["erased", "removed_symbol"] {
            assert!(
                graph["edges"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|e| e["from"] == consumer
                        && e["evidenceVersion"] == "baseline"
                        && e["symbol"] == name),
                "Real server did not establish removed-symbol consumer evidence: {graph}"
            );
        }
        assert!(!t.path().join(changed).exists());
        state.language_servers.shutdown().await;
    }
    #[tokio::test]
    #[ignore = "Requires installed LOCALLM_RUST_ANALYZER; validates Git baseline impact, not just navigation."]
    async fn real_rust_impact() {
        real_impact("rust", std::env::var("LOCALLM_RUST_ANALYZER").unwrap()).await;
    }
    #[tokio::test]
    #[ignore = "Requires installed LOCALLM_TYPESCRIPT_LANGUAGE_SERVER; validates Git baseline impact."]
    async fn real_typescript_impact() {
        real_impact(
            "typescript",
            std::env::var("LOCALLM_TYPESCRIPT_LANGUAGE_SERVER").unwrap(),
        )
        .await;
    }
}
