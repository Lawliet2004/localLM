//! Bounded OpenAPI 3.0/3.1 resolution and operation-driven request preparation.
use crate::development_tools as dev;
use futures_util::{future::BoxFuture, FutureExt, StreamExt};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
pub fn parse(text: &str) -> Result<Value, String> {
    if text.len() > 1024 * 1024 {
        return Err("OpenAPI document exceeds 1 MiB.".into());
    }
    serde_json::from_str(text)
        .or_else(|_| serde_yaml_ng::from_str(text))
        .map_err(|e| format!("Invalid JSON/YAML document: {e}"))
}
#[derive(Default)]
struct Bundle {
    ids: BTreeMap<String, String>,
    queue: Vec<(String, String, Value)>,
    nodes: usize,
    legacy: bool,
}
fn reject_nonproductive_cycles(definitions: &serde_json::Map<String, Value>) -> Result<(), String> {
    fn links(value: &Value, out: &mut Vec<String>) {
        if let Some(r) = value["$ref"]
            .as_str()
            .and_then(|s| s.strip_prefix("#/$defs/"))
        {
            out.push(r.into());
        }
        for key in ["allOf", "anyOf", "oneOf"] {
            for v in value[key].as_array().into_iter().flatten() {
                links(v, out);
            }
        }
        for key in ["not", "if", "then", "else"] {
            if let Some(v) = value.get(key) {
                links(v, out);
            }
        }
    }
    fn visit(
        id: &str,
        definitions: &serde_json::Map<String, Value>,
        active: &mut BTreeSet<String>,
        done: &mut BTreeSet<String>,
    ) -> Result<(), String> {
        if done.contains(id) {
            return Ok(());
        }
        if !active.insert(id.into()) {
            return Err("Invalid schema reference cycle without instance descent.".into());
        }
        if let Some(value) = definitions.get(id) {
            let mut refs = Vec::new();
            links(value, &mut refs);
            for next in refs {
                visit(&next, definitions, active, done)?;
            }
        }
        active.remove(id);
        done.insert(id.into());
        Ok(())
    }
    let mut done = BTreeSet::new();
    for id in definitions.keys() {
        visit(id, definitions, &mut BTreeSet::new(), &mut done)?;
    }
    Ok(())
}
struct Resolver<'a, 'state> {
    ctx: &'a crate::harness::HarnessCtx<'state>,
    documents: BTreeMap<String, Value>,
    active: BTreeSet<String>,
    references: usize,
    bytes: usize,
    legacy: bool,
}
impl Resolver<'_, '_> {
    // Fetch only under the existing workspace/exact-origin policy. The bundle
    // compiler below rewrites references to local definitions, never to URLs.
    async fn target(
        &mut self,
        reference: &str,
        document: &str,
    ) -> Result<(String, String, Value), String> {
        self.references += 1;
        if self.references > 512 {
            return Err("OpenAPI exceeds 512 references.".into());
        }
        let (location, fragment) = reference.split_once('#').unwrap_or((reference, ""));
        let target_document = if location.is_empty() {
            document.to_string()
        } else if document.starts_with("https://") || document.starts_with("http://") {
            reqwest::Url::parse(document)
                .map_err(|e| e.to_string())?
                .join(location)
                .map_err(|e| e.to_string())?
                .to_string()
        } else if location.starts_with("https://") || location.starts_with("http://") {
            location.to_string()
        } else {
            normalize(
                &std::path::Path::new(document)
                    .parent()
                    .unwrap_or(std::path::Path::new(""))
                    .join(location),
            )?
        };
        if !self.documents.contains_key(&target_document) {
            if self.documents.len() >= 32 {
                return Err("OpenAPI exceeds 32 documents.".into());
            }
            let text = if target_document.starts_with("https://")
                || target_document.starts_with("http://")
            {
                if self.ctx.state.database()?.setting::<bool>("offline_mode")? {
                    return Err("Offline mode blocks remote OpenAPI references.".into());
                }
                let url = reqwest::Url::parse(&target_document).map_err(|e| e.to_string())?;
                let allowed: Vec<String> = self
                    .ctx
                    .state
                    .database()?
                    .setting("development.reference_origins")?;
                if !url.username().is_empty()
                    || url.password().is_some()
                    || !allowed.contains(&url.origin().ascii_serialization())
                {
                    return Err(
                        "Remote reference origin is not authorized in Development tools settings."
                            .into(),
                    );
                }
                // The authorized origin is still vetted: the answer can be a
                // loopback or private address, and the socket must not
                // re-resolve the name to a different one.
                let vetted = crate::sandbox::vet_fetch_target(&url, false)
                    .await
                    .map_err(|error| format!("Remote reference rejected: {error}"))?;
                let mut builder = reqwest::Client::builder()
                    .redirect(reqwest::redirect::Policy::none())
                    .timeout(std::time::Duration::from_secs(15))
                    .no_proxy();
                if let Some(domain) = &vetted.domain {
                    if !vetted.addrs.is_empty() {
                        builder = builder.resolve_to_addrs(domain, &vetted.addrs);
                    }
                }
                let client = builder.build().map_err(|e| e.to_string())?;
                let response = client
                    .get(url)
                    .send()
                    .await
                    .map_err(|e| e.without_url().to_string())?;
                if !response.status().is_success() {
                    return Err("Remote OpenAPI reference request failed.".into());
                }
                let mut data = Vec::new();
                let mut stream = response.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(|e| e.without_url().to_string())?;
                    if data.len() + chunk.len() > 1024 * 1024 {
                        return Err("Remote reference exceeds 1 MiB.".into());
                    }
                    data.extend_from_slice(&chunk);
                }
                String::from_utf8(data).map_err(|_| "Remote reference is not UTF-8.")?
            } else {
                crate::workspace::Workspace::open(&self.ctx.snapshot.workspace_path)?
                    .language_source(&target_document)?
            };
            self.bytes += text.len();
            if self.bytes > 8 * 1024 * 1024 {
                return Err("Referenced OpenAPI documents exceed 8 MiB.".into());
            }
            self.documents
                .insert(target_document.clone(), parse(&text)?);
        }
        let source = self
            .documents
            .get(&target_document)
            .ok_or("Missing reference document.")?;
        let fragment = percent_decode(fragment)?;
        let target = if fragment.is_empty() {
            source.clone()
        } else {
            source
                .pointer(&fragment)
                .cloned()
                .ok_or("OpenAPI reference pointer not found (anchors are unsupported).")?
        };
        Ok((
            format!("{target_document}#{fragment}"),
            target_document,
            target,
        ))
    }
    fn schema_node<'a>(
        &'a mut self,
        value: Value,
        document: String,
        depth: usize,
        bundle: &'a mut Bundle,
    ) -> BoxFuture<'a, Result<Value, String>> {
        async move {
            bundle.nodes += 1;
            if depth > 32 || bundle.nodes > 8000 { return Err("OpenAPI schema exceeds nesting/node budget.".into()); }
            match value {
                Value::Object(mut map) => {
                    if ["$anchor", "$dynamicRef", "$dynamicAnchor", "$recursiveRef", "$recursiveAnchor"].iter().any(|k| map.contains_key(*k)) { return Err("Schema anchors and dynamic/recursive-anchor references are unsupported; use document-relative JSON pointers.".into()); }
                    let mut document = document;
                    if let Some(id) = map.remove("$id") {
                        if bundle.legacy { return Err("Schema $id requires OpenAPI 3.1.".into()); }
                        let id=id.as_str().ok_or("Schema $id must be text.")?;
                        if id.contains('#') { return Err("Schema $id fragments are unsupported.".into()); }
                        let resource = if document.starts_with("http://") || document.starts_with("https://") { reqwest::Url::parse(&document).map_err(|e|e.to_string())?.join(id).map_err(|e|e.to_string())?.to_string() }
                        else if id.starts_with("http://") || id.starts_with("https://") { id.to_string() }
                        else { if id.contains(':') {return Err("Schema $id supports workspace paths and HTTP(S) resource identities only.".into());} normalize(&std::path::Path::new(&document).parent().unwrap_or(std::path::Path::new("")).join(id))? };
                        let stripped=Value::Object(map.clone());
                        if let Some(existing)=self.documents.get(&resource) {
                            let mut existing=existing.clone(); if let Some(map)=existing.as_object_mut() {map.remove("$id");}
                            if existing!=stripped {return Err("Conflicting schema resource IDs.".into());}
                        } else {
                            if self.documents.len()>=32 {return Err("OpenAPI exceeds 32 document/resource identities.".into());}
                            self.bytes+=stripped.to_string().len(); if self.bytes>8*1024*1024 {return Err("Schema resource bundle exceeds 8 MiB.".into());}
                        }
                        // Strip the already-applied ID in the resource cache so a
                        // later reference cannot apply a relative base twice.
                        self.documents.insert(resource.clone(),stripped);
                        document=resource;
                    }
                    if let Some(reference) = map.remove("$ref") {
                        let (key, base, target) = self.target(reference.as_str().ok_or("Schema reference must be text.")?, &document).await?;
                        let id = if let Some(id) = bundle.ids.get(&key) { id.clone() } else {
                            let id = format!("_locallm_ref{}", bundle.ids.len());
                            bundle.ids.insert(key, id.clone()); bundle.queue.push((id.clone(), base, target)); id
                        };
                        map.insert("$ref".into(), json!(format!("#/$defs/{id}")));
                        if bundle.legacy { map.retain(|k, _| k == "$ref"); }
                    }
                    // Traverse schema-bearing keywords only. Examples/defaults/enum
                    // are data and can legitimately contain a property named $ref.
                    for key in ["properties", "patternProperties", "$defs", "definitions", "dependentSchemas"] {
                        if let Some(values) = map.get_mut(key).and_then(Value::as_object_mut) {
                            for value in values.values_mut() { *value = self.schema_node(value.take(), document.clone(), depth + 1, bundle).await?; }
                        }
                    }
                    for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
                        if let Some(values) = map.get_mut(key).and_then(Value::as_array_mut) {
                            for value in values { *value = self.schema_node(value.take(), document.clone(), depth + 1, bundle).await?; }
                        }
                    }
                    for key in ["items", "additionalProperties", "unevaluatedProperties", "unevaluatedItems", "contains", "not", "if", "then", "else", "propertyNames"] {
                        if let Some(value) = map.get_mut(key) { *value = self.schema_node(value.take(), document.clone(), depth + 1, bundle).await?; }
                    }
                    Ok(Value::Object(map))
                }
                Value::Bool(_) => if bundle.legacy { Err("Boolean schemas require OpenAPI 3.1.".into()) } else { Ok(value) },
                _ => Err("Schema must be an object or a 3.1 boolean schema.".into()),
            }
        }.boxed()
    }
    async fn bundle_schema(&mut self, value: Value, document: String) -> Result<Value, String> {
        let legacy = self.legacy;
        let mut bundle = Bundle {
            legacy,
            ..Default::default()
        };
        let mut root = self.schema_node(value, document, 0, &mut bundle).await?;
        let mut definitions = serde_json::Map::new();
        let mut index = 0;
        while index < bundle.queue.len() {
            if index >= 128 {
                return Err("Schema bundle exceeds 128 definitions.".into());
            }
            let (id, base, target) = bundle.queue[index].clone();
            index += 1;
            definitions.insert(id, self.schema_node(target, base, 0, &mut bundle).await?);
        }
        reject_nonproductive_cycles(&definitions)?;
        if !definitions.is_empty() {
            let map = root
                .as_object_mut()
                .ok_or("Referenced boolean root cannot carry definitions.")?;
            let existing = map
                .entry("$defs")
                .or_insert_with(|| json!({}))
                .as_object_mut()
                .ok_or("Invalid schema definitions.")?;
            for (key, value) in definitions {
                if existing.insert(key, value).is_some() {
                    return Err("Reserved schema bundle name collision.".into());
                }
            }
        }
        Ok(root)
    }
    fn resolve<'a>(
        &'a mut self,
        value: Value,
        document: String,
        depth: usize,
    ) -> BoxFuture<'a, Result<Value, String>> {
        async move {
            if depth > 32 {
                return Err("OpenAPI reference nesting exceeds 32 levels.".into());
            }
            match value {
                Value::Object(mut map) => {
                    if let Some(reference) = map.remove("$ref") {
                        let (key, target_document, target) = self
                            .target(
                                reference
                                    .as_str()
                                    .ok_or("OpenAPI reference must be text.")?,
                                &document,
                            )
                            .await?;
                        if !self.active.insert(key.clone()) {
                            return Err(format!("Invalid cyclic OpenAPI Reference Object: {key}"));
                        }
                        let mut resolved = self.resolve(target, target_document, depth + 1).await?;
                        self.active.remove(&key);
                        if let Value::Object(ref mut base) = resolved {
                            for (key, value) in map {
                                base.insert(
                                    key,
                                    self.resolve(value, document.clone(), depth + 1).await?,
                                );
                            }
                        }
                        return Ok(resolved);
                    }
                    for (key, value) in map.iter_mut() {
                        if matches!(key.as_str(), "example" | "default" | "enum" | "value") {
                            continue;
                        }
                        if key == "schema" {
                            *value = self.bundle_schema(value.take(), document.clone()).await?;
                        } else if key == "schemas" {
                            for value in value
                                .as_object_mut()
                                .ok_or("Invalid components schemas.")?
                                .values_mut()
                            {
                                *value = self.bundle_schema(value.take(), document.clone()).await?;
                            }
                        } else {
                            *value = self
                                .resolve(value.take(), document.clone(), depth + 1)
                                .await?;
                        }
                    }
                    Ok(Value::Object(map))
                }
                Value::Array(items) => {
                    let mut resolved = Vec::new();
                    for item in items {
                        resolved.push(self.resolve(item, document.clone(), depth + 1).await?);
                    }
                    Ok(Value::Array(resolved))
                }
                v => Ok(v),
            }
        }
        .boxed()
    }
}
fn normalize(path: &std::path::Path) -> Result<String, String> {
    let mut components = Vec::new();
    for c in path.components() {
        match c {
            std::path::Component::Normal(v) => components.push(v.to_string_lossy().into_owned()),
            std::path::Component::ParentDir => {
                components
                    .pop()
                    .ok_or("OpenAPI reference escapes workspace.")?;
            }
            std::path::Component::CurDir => (),
            _ => return Err("References must be workspace relative.".into()),
        }
    }
    let p = components.join("/");
    crate::sandbox::check_fs_path("", &p, false)?;
    Ok(p)
}
/// Reject an OpenAPI server URL that addresses a private, loopback or
/// cloud-metadata network.
///
/// The OpenAPI document is workspace content the model can write, so its
/// server URL is untrusted input. A literal address is refused here; a name
/// is resolved and refused by the HTTP layer, which vets every hop of the
/// request it makes.
fn reject_private_server(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|_| "OpenAPI server URL must be an absolute HTTP or HTTPS URL.".to_string())?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err("OpenAPI server URL must use HTTP or HTTPS.".into());
    }
    let host = parsed
        .host_str()
        .ok_or("OpenAPI server URL needs a host.")?;
    let literal = host
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
        .unwrap_or(host);
    if let Ok(ip) = literal.parse::<std::net::IpAddr>() {
        if crate::sandbox::is_blocked_ip(&ip) {
            return Err(
                "OpenAPI server URL must not address a loopback, link-local or private network."
                    .into(),
            );
        }
    }
    Ok(())
}
fn percent_decode(s: &str) -> Result<String, String> {
    let mut bytes = Vec::new();
    let mut i = 0;
    while i < s.len() {
        if s.as_bytes()[i] == b'%' {
            if i + 2 >= s.len() {
                return Err("Invalid reference fragment encoding.".into());
            }
            bytes.push(
                u8::from_str_radix(&s[i + 1..i + 3], 16)
                    .map_err(|_| "Invalid reference fragment encoding.")?,
            );
            i += 3;
        } else {
            bytes.push(s.as_bytes()[i]);
            i += 1;
        }
    }
    String::from_utf8(bytes).map_err(|e| e.to_string())
}
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
fn scalar(v: &Value) -> Result<String, String> {
    match v {
        Value::String(s) => Ok(s.clone()),
        Value::Number(_) | Value::Bool(_) => Ok(v.to_string()),
        _ => Err("Parameter must be a scalar or a supported array/object serialization.".into()),
    }
}
fn schema(mut value: Value, legacy: bool) -> Value {
    match &mut value {
        Value::Object(map) => {
            for key in [
                "properties",
                "patternProperties",
                "$defs",
                "definitions",
                "dependentSchemas",
            ] {
                if let Some(values) = map.get_mut(key).and_then(Value::as_object_mut) {
                    for value in values.values_mut() {
                        *value = schema(value.take(), legacy);
                    }
                }
            }
            for key in ["allOf", "anyOf", "oneOf", "prefixItems"] {
                if let Some(values) = map.get_mut(key).and_then(Value::as_array_mut) {
                    for value in values {
                        *value = schema(value.take(), legacy);
                    }
                }
            }
            for key in [
                "items",
                "additionalProperties",
                "unevaluatedProperties",
                "unevaluatedItems",
                "contains",
                "not",
                "if",
                "then",
                "else",
                "propertyNames",
            ] {
                if let Some(value) = map.get_mut(key) {
                    *value = schema(value.take(), legacy);
                }
            }
            if legacy {
                if map.remove("nullable") == Some(json!(true)) {
                    if let Some(t) = map.get_mut("type") {
                        *t = json!([t.clone(), "null"]);
                    }
                    if let Some(items) = map.get_mut("enum").and_then(Value::as_array_mut) {
                        items.push(Value::Null);
                    }
                }
                for (flag, bound) in [
                    ("exclusiveMinimum", "minimum"),
                    ("exclusiveMaximum", "maximum"),
                ] {
                    if map.get(flag).is_some_and(Value::is_boolean) {
                        let exclusive = map.remove(flag) == Some(json!(true));
                        if exclusive {
                            if let Some(v) = map.remove(bound) {
                                map.insert(flag.into(), v);
                            }
                        }
                    }
                }
            }
        }
        Value::Array(items) => {
            for v in items {
                *v = schema(v.take(), legacy)
            }
        }
        _ => (),
    }
    value
}
async fn document(
    ctx: &crate::harness::HarnessCtx<'_>,
    path: &str,
) -> Result<(Value, String), String> {
    let text =
        crate::workspace::Workspace::open(&ctx.snapshot.workspace_path)?.language_source(path)?;
    let parsed = parse(&text)?;
    if !parsed["openapi"]
        .as_str()
        .is_some_and(|v| v.starts_with("3.0.") || v.starts_with("3.1."))
    {
        return Err("Expected OpenAPI 3.0 or 3.1.".into());
    }
    let mut resolver = Resolver {
        ctx,
        documents: BTreeMap::from([(path.into(), parsed.clone())]),
        active: BTreeSet::new(),
        references: 0,
        bytes: text.len(),
        legacy: parsed["openapi"]
            .as_str()
            .is_some_and(|v| v.starts_with("3.0.")),
    };
    Ok((
        tokio::time::timeout(
            std::time::Duration::from_secs(45),
            resolver.resolve(parsed, path.into(), 0),
        )
        .await
        .map_err(|_| "OpenAPI resolution exceeded 45 seconds.")??,
        dev::hash(text.as_bytes()),
    ))
}
pub async fn inspect(ctx: &crate::harness::HarnessCtx<'_>, args: Value) -> Result<Value, String> {
    let path = args["path"].as_str().ok_or("Missing OpenAPI path.")?;
    let (spec, sha) = document(ctx, path).await?;
    let mut operations = Vec::new();
    for (path, item) in spec["paths"].as_object().into_iter().flatten() {
        for method in [
            "get", "post", "put", "patch", "delete", "head", "options", "trace",
        ] {
            if let Some(op) = item.get(method) {
                if args["operation_id"]
                    .as_str()
                    .is_none_or(|id| op["operationId"] == id)
                {
                    operations.push(json!({"method":method.to_uppercase(),"path":path,"operationId":op["operationId"],"operation":op,"pathParameters":item["parameters"]}));
                }
            }
        }
    }
    if args["operation_id"].is_string() && operations.len() != 1 {
        return Err("Operation ID is missing or ambiguous.".into());
    }
    let total = operations.len();
    operations.truncate(100);
    Ok(
        json!({"openapi":spec["openapi"],"sourceSha256":sha,"operations":operations,"total":total,"truncated":total>100,"referencesResolved":true}),
    )
}
fn path_parameter(name: &str, value: &Value, style: &str, explode: bool) -> Result<String, String> {
    let encoded = |v: &Value| scalar(v).map(|v| encode(&v));
    let n = encode(name);
    let parts = match value {
        Value::Array(a) => a.iter().map(encoded).collect::<Result<Vec<_>, _>>()?,
        Value::Object(o) => o
            .iter()
            .map(|(k, v)| Ok((encode(k), encoded(v)?)))
            .collect::<Result<Vec<_>, String>>()?
            .into_iter()
            .flat_map(|(k, v)| {
                if explode {
                    vec![format!("{k}={v}")]
                } else {
                    vec![k, v]
                }
            })
            .collect(),
        v => vec![encoded(v)?],
    };
    match style {
        "simple" => Ok(parts.join(",")),
        "label" => Ok(format!(".{}", parts.join(if explode { "." } else { "," }))),
        "matrix" => {
            if explode {
                match value {
                    Value::Array(_) => Ok(parts.into_iter().map(|v| format!(";{n}={v}")).collect()),
                    Value::Object(_) => Ok(format!(";{}", parts.join(";"))),
                    _ => Ok(format!(";{n}={}", parts.join(","))),
                }
            } else {
                Ok(format!(";{n}={}", parts.join(",")))
            }
        }
        _ => Err("Unsupported path parameter style.".into()),
    }
}
fn simple_parameter(value: &Value, explode: bool) -> Result<String, String> {
    match value {
        Value::Array(a) => a
            .iter()
            .map(scalar)
            .collect::<Result<Vec<_>, _>>()
            .map(|v| v.join(",")),
        Value::Object(o) => {
            let mut parts = Vec::new();
            for (k, v) in o {
                let v = scalar(v)?;
                if explode {
                    parts.push(format!("{k}={v}"));
                } else {
                    parts.extend([k.clone(), v]);
                }
            }
            Ok(parts.join(","))
        }
        v => scalar(v),
    }
}
pub async fn prepare(ctx: &crate::harness::HarnessCtx<'_>, args: Value) -> Result<Value, String> {
    let path = args["path"].as_str().ok_or("Missing OpenAPI path.")?;
    let (spec, _) = document(ctx, path).await?;
    let legacy = spec["openapi"]
        .as_str()
        .is_some_and(|v| v.starts_with("3.0."));
    let id = args["operation_id"]
        .as_str()
        .ok_or("Missing operation ID.")?;
    let mut found = Vec::new();
    for (path, item) in spec["paths"].as_object().into_iter().flatten() {
        for method in ["get", "post", "put", "patch", "delete", "head", "options"] {
            if item[method]["operationId"] == id {
                found.push((path, item, method, &item[method]));
            }
        }
    }
    if found.len() != 1 {
        return Err("Operation ID is missing or ambiguous.".into());
    }
    let (route, item, method, op) = found[0];
    let mut base = args["server_url"]
        .as_str()
        .map(str::to_string)
        .or_else(|| {
            op["servers"][0]["url"]
                .as_str()
                .or_else(|| item["servers"][0]["url"].as_str())
                .or_else(|| spec["servers"][0]["url"].as_str())
                .map(str::to_string)
        })
        .unwrap_or_default();
    let server = op["servers"]
        .as_array()
        .or_else(|| item["servers"].as_array())
        .or_else(|| spec["servers"].as_array())
        .and_then(|s| s.first());
    if let Some(server) = server {
        for (name, var) in server["variables"].as_object().into_iter().flatten() {
            let value = args["server_variables"]
                .get(name)
                .or_else(|| var.get("default"))
                .ok_or("Server variable has no value/default.")?;
            if let Some(values) = var["enum"].as_array() {
                if !values.contains(value) {
                    return Err("Server variable is outside its enum.".into());
                }
            }
            base = base.replace(&format!("{{{name}}}"), &scalar(value)?);
        }
    }
    // The server URL comes from the document, which the model can write.
    if !base.is_empty() {
        reject_private_server(&base)?;
    }
    let mut route = route.to_string();
    let mut query = Vec::new();
    let mut headers = serde_json::Map::new();
    let mut cookie = Vec::new();
    let mut parameters = BTreeMap::new();
    for p in item["parameters"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(op["parameters"].as_array().into_iter().flatten())
    {
        parameters.insert(format!("{}:{}", p["in"], p["name"]), p);
    }
    for p in parameters.values() {
        let name = p["name"].as_str().ok_or("Parameter lacks name.")?;
        let location = p["in"].as_str().ok_or("Parameter lacks location.")?;
        let key = match location {
            "path" => "path_parameters",
            "query" => "query_parameters",
            "header" => "header_parameters",
            "cookie" => "cookie_parameters",
            _ => return Err("Unsupported parameter location.".into()),
        };
        let value = args[key].get(name).or_else(|| p["schema"].get("default"));
        let Some(value) = value else {
            if p["required"] == true || location == "path" {
                return Err(format!("Missing required {location} parameter '{name}'."));
            }
            continue;
        };
        if let Some(s) = p.get("schema") {
            crate::tool_validation::validate_value(&schema(s.clone(), legacy), value)
                .map_err(|e| format!("Invalid parameter '{name}': {e}"))?;
        }
        let style = p["style"]
            .as_str()
            .unwrap_or(if location == "query" || location == "cookie" {
                "form"
            } else {
                "simple"
            });
        let explode = p["explode"].as_bool().unwrap_or(style == "form");
        if location == "path" {
            let encoded = path_parameter(name, value, style, explode)?;
            route = route.replace(&format!("{{{name}}}"), &encoded);
        } else if location == "query" {
            match value {
                Value::Array(values) => {
                    if style == "form" && explode {
                        for v in values {
                            query.push((name.to_string(), scalar(v)?));
                        }
                    } else {
                        let sep = match style {
                            "form" => ",",
                            "spaceDelimited" => " ",
                            "pipeDelimited" => "|",
                            _ => return Err("Unsupported query array style.".into()),
                        };
                        query.push((
                            name.into(),
                            values
                                .iter()
                                .map(scalar)
                                .collect::<Result<Vec<_>, _>>()?
                                .join(sep),
                        ));
                    }
                }
                Value::Object(values) => {
                    if style == "deepObject" {
                        for (k, v) in values {
                            query.push((format!("{name}[{k}]"), scalar(v)?));
                        }
                    } else if style == "form" && explode {
                        for (k, v) in values {
                            query.push((k.clone(), scalar(v)?));
                        }
                    } else if style == "form" {
                        query.push((name.into(), simple_parameter(value, false)?));
                    } else {
                        return Err("Unsupported object parameter serialization.".into());
                    }
                }
                v => query.push((name.into(), scalar(v)?)),
            }
        } else if location == "header" {
            if style != "simple" {
                return Err("Headers require simple serialization.".into());
            }
            headers.insert(name.into(), json!(simple_parameter(value, explode)?));
        } else {
            if style != "form" {
                return Err("Cookies require form serialization.".into());
            }
            match value {
                Value::Array(items) if explode => {
                    for item in items {
                        cookie.push(format!("{}={}", encode(name), encode(&scalar(item)?)));
                    }
                }
                Value::Object(items) if explode => {
                    for (key, value) in items {
                        cookie.push(format!("{}={}", encode(key), encode(&scalar(value)?)));
                    }
                }
                value => cookie.push(format!(
                    "{}={}",
                    encode(name),
                    encode(&simple_parameter(value, false)?)
                )),
            }
        }
    }
    if route.contains('{') || base.contains('{') {
        return Err("Unresolved OpenAPI URL template.".into());
    }
    if !cookie.is_empty() {
        headers.insert("Cookie".into(), json!(cookie.join("; ")));
    }
    let target = if base.is_empty() {
        if args["session_id"].is_null() {
            return Err("Select server_url or an HTTP session for this operation.".into());
        }
        route
    } else {
        format!("{}{}", base.trim_end_matches('/'), route)
    };
    let query_string = reqwest::Url::parse("http://query.invalid/")
        .map(|mut u| {
            u.query_pairs_mut().extend_pairs(query);
            u.query().unwrap_or("").to_string()
        })
        .map_err(|e| e.to_string())?;
    let target = if query_string.is_empty() {
        target
    } else {
        format!(
            "{target}{}{}",
            if target.contains('?') { "&" } else { "?" },
            query_string
        )
    };
    let mut prepared = json!({"url":target,"method":method.to_uppercase(),"headers":headers});
    for key in ["session_id", "credential_id"] {
        if args.get(key).is_some() {
            prepared[key] = args[key].clone();
        }
    }
    if op["requestBody"]["required"] == true
        && args.get("body").is_none()
        && args.get("files").is_none()
    {
        return Err("Operation requires a request body.".into());
    }
    let mut combined_body = args.get("body").cloned();
    if let Some(files) = args["files"].as_array() {
        let mut body = combined_body.take().unwrap_or(json!({}));
        let fields = body
            .as_object_mut()
            .ok_or("Multipart body must be an object.")?;
        for file in files {
            let name = file["field"]
                .as_str()
                .ok_or("Multipart file requires field.")?;
            fields.insert(
                name.into(),
                json!(file["path"]
                    .as_str()
                    .ok_or("Multipart file requires path.")?),
            );
        }
        combined_body = Some(body);
    }
    if let Some(body) = combined_body.as_ref() {
        let content = op["requestBody"]["content"]
            .as_object()
            .ok_or("Operation does not define a request body.")?;
        let kind = if args["files"].is_array() {
            Some("multipart/form-data")
        } else {
            None
        }
        .or_else(|| args["content_type"].as_str())
        .or_else(|| {
            [
                "application/json",
                "text/plain",
                "application/x-www-form-urlencoded",
                "multipart/form-data",
            ]
            .into_iter()
            .find(|k| content.contains_key(*k))
        })
        .ok_or("Select a supported request content type.")?;
        let media = content
            .get(kind)
            .ok_or("Request content type is not defined by this operation.")?;
        if let Some(s) = media.get("schema") {
            crate::tool_validation::validate_value(&schema(s.clone(), legacy), body)
                .map_err(|e| format!("Request body failed schema validation: {e}"))?;
        }
        match kind {
            "application/json" => prepared["json"] = body.clone(),
            "text/plain" => {
                prepared["body"] = json!(body.as_str().ok_or("Text body must be a string.")?);
                prepared["headers"]["Content-Type"] = json!(kind);
            }
            "application/x-www-form-urlencoded" | "multipart/form-data" => {
                let mut fields = args.get("body").cloned().unwrap_or(json!({}));
                let fields = fields
                    .as_object_mut()
                    .ok_or("Form body must be an object.")?;
                for value in fields.values_mut() {
                    *value = json!(scalar(value)?);
                }
                prepared["form"] = json!(fields);
                if kind == "multipart/form-data" {
                    prepared["files"] = args.get("files").cloned().unwrap_or(json!([]));
                } else if args["files"].is_array() {
                    return Err("Files require multipart/form-data.".into());
                }
            }
            _ => return Err("Unsupported request media type.".into()),
        }
    }
    if let Some(files) = args.get("files") {
        prepared["files"] = files.clone();
    }
    let mut responses = serde_json::Map::new();
    for (status, response) in op["responses"].as_object().into_iter().flatten() {
        let mut types = serde_json::Map::new();
        for (kind, media) in response["content"].as_object().into_iter().flatten() {
            if let Some(s) = media.get("schema") {
                types.insert(kind.clone(), schema(s.clone(), legacy));
            }
        }
        responses.insert(status.clone(), Value::Object(types));
    }
    prepared["response_schemas"] = Value::Object(responses);
    Ok(prepared)
}
pub fn definition() -> (String, String, Value) {
    ("openapi_request".into(),"Resolve a workspace OpenAPI 3.0/3.1 JSON/YAML operation, validate inputs, construct a correctly encoded HTTP request and validate the response against status/media-specific schemas. Local references resolve within the workspace; remote reference origins require native settings authorization. The request can have side effects and is never blindly retried.".into(),dev::schema(json!({"path":{"type":"string","minLength":1,"maxLength":4096},"operation_id":{"type":"string","minLength":1,"maxLength":256},"server_url":{"type":"string","maxLength":4096},"server_variables":{"type":"object"},"session_id":{"type":"string","maxLength":128},"credential_id":{"type":"string","maxLength":128},"path_parameters":{"type":"object"},"query_parameters":{"type":"object"},"header_parameters":{"type":"object"},"cookie_parameters":{"type":"object"},"body":{},"content_type":{"type":"string","maxLength":128},"files":{"type":"array","maxItems":16}}),&["path","operation_id"]))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn openapi_server_urls_refuse_private_and_loopback_addresses() {
        for server in [
            "http://127.0.0.1:8080/",
            "http://169.254.169.254/latest/meta-data/",
            "http://10.0.0.5/",
            "http://[::1]/",
            "file:///etc/passwd",
            "not-a-url",
        ] {
            assert!(reject_private_server(server).is_err(), "{server}");
        }
        assert!(reject_private_server("https://example.com/api").is_ok());
    }
    #[tokio::test]
    async fn an_openapi_document_cannot_point_a_request_at_a_private_address() {
        let t = tempfile::tempdir().unwrap();
        let spec = json!({"openapi":"3.1.0","servers":[{"url":"http://127.0.0.1:9/"}],"paths":{"/ok":{"get":{"operationId":"get","responses":{"200":{"description":"ok"}}}}}});
        std::fs::write(t.path().join("api.json"), spec.to_string()).unwrap();
        let state = crate::test_support::app_state(t.path());
        let owner = state.database().unwrap().create_conversation().unwrap().id;
        let ctx = crate::development_tools::tests::context(&state, t.path(), &owner, &[]);
        let error = prepare(&ctx, json!({"path":"api.json","operation_id":"get"}))
            .await
            .unwrap_err();
        assert!(error.contains("private network"), "{error}");
    }
    #[test]
    fn path_styles_follow_explode_rules() {
        assert_eq!(
            path_parameter("id", &json!([1, 2]), "label", true).unwrap(),
            ".1.2"
        );
        assert_eq!(
            path_parameter("id", &json!([1, 2]), "matrix", true).unwrap(),
            ";id=1;id=2"
        );
        assert_eq!(
            path_parameter("id", &json!({"a":"x/y"}), "simple", true).unwrap(),
            "a=x%2Fy"
        );
    }
    #[tokio::test]
    async fn referenced_yaml_operation_validates_and_encodes_before_network() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(
            t.path().join("schema.yaml"),
            "type: object\nrequired: [name]\nproperties:\n  name:\n    type: string\n",
        )
        .unwrap();
        std::fs::write(
            t.path().join("api.yaml"),
            r##"openapi: 3.0.3
servers:
  - url: https://example.com
paths:
  /things/{id}:
    post:
      operationId: createThing
      parameters:
        - in: path
          name: id
          required: true
          schema: {type: string}
      requestBody:
        required: true
        content:
          application/json:
            schema:
              $ref: schema.yaml
      responses:
        '201':
          description: created
          content:
            application/json:
              schema:
                $ref: schema.yaml
"##,
        )
        .unwrap();
        let state = crate::test_support::app_state(t.path());
        let owner = state.database().unwrap().create_conversation().unwrap().id;
        let ctx = crate::development_tools::tests::context(&state, t.path(), &owner, &[]);
        let args = json!({"path":"api.yaml","operation_id":"createThing","path_parameters":{"id":"a/b"},"body":{"name":"thing"}});
        let prepared = prepare(&ctx, args.clone()).await.unwrap();
        assert_eq!(prepared["url"], "https://example.com/things/a%2Fb");
        assert!(crate::tool_validation::validate_value(
            &prepared["response_schemas"]["201"]["application/json"],
            &json!({"name":"thing"})
        )
        .is_ok());
        assert!(crate::tool_validation::validate_value(
            &prepared["response_schemas"]["201"]["application/json"],
            &json!({})
        )
        .is_err());
        let mut invalid = args;
        invalid["body"] = json!({"name":42});
        assert!(prepare(&ctx, invalid).await.is_err());
    }
    #[tokio::test]
    async fn recursive_schemas_bundle_local_refs_and_reject_nonproductive_cycles() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir(t.path().join("schemas")).unwrap();
        std::fs::write(t.path().join("schemas/node.json"), json!({"type":"object","required":["name"],"properties":{"name":{"type":"string"},"children":{"type":"array","items":{"$ref":"node.json"}}},"additionalProperties":false}).to_string()).unwrap();
        let spec = json!({"openapi":"3.1.0","servers":[{"url":"https://example.com"}],"paths":{"/node":{"post":{"operationId":"node","requestBody":{"required":true,"content":{"application/json":{"schema":{"$ref":"schemas/node.json"}}}},"responses":{"200":{"description":"ok","content":{"application/json":{"schema":{"$ref":"schemas/node.json"}}}}}}}}});
        std::fs::write(t.path().join("api.json"), spec.to_string()).unwrap();
        let state = crate::test_support::app_state(t.path());
        let owner = state.database().unwrap().create_conversation().unwrap().id;
        let ctx = crate::development_tools::tests::context(&state, t.path(), &owner, &[]);
        let body = json!({"name":"root","children":[{"name":"leaf","children":[]}]});
        let prepared = prepare(
            &ctx,
            json!({"path":"api.json","operation_id":"node","body":body}),
        )
        .await
        .unwrap();
        let bundled = &prepared["response_schemas"]["200"]["application/json"];
        let validator = crate::tool_validation::compile(bundled).unwrap();
        assert!(validator.is_valid(&body));
        assert!(!validator.is_valid(&json!({"name":"root","children":[{"name":42}]})));
        // A compiled bundle remains self-contained after reference documents vanish.
        std::fs::remove_file(t.path().join("schemas/node.json")).unwrap();
        assert!(validator.is_valid(&body));
        std::fs::write(
            t.path().join("schemas/node.json"),
            r##"{"$ref":"other.json"}"##,
        )
        .unwrap();
        std::fs::write(
            t.path().join("schemas/other.json"),
            r##"{"$ref":"node.json"}"##,
        )
        .unwrap();
        let error = prepare(
            &ctx,
            json!({"path":"api.json","operation_id":"node","body":body}),
        )
        .await
        .unwrap_err();
        assert!(error.contains("without instance descent"), "{error}");
    }
    #[tokio::test]
    async fn nested_schema_ids_preserve_relative_resource_bases() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join("schemas/models")).unwrap();
        std::fs::write(t.path().join("schemas/root.json"),r##"{"$id":"models/tree.json","type":"object","properties":{"leaf":{"$ref":"leaf.json"},"child":{"$ref":"#"}}}"##).unwrap();
        std::fs::write(
            t.path().join("schemas/models/leaf.json"),
            r#"{"type":"integer","minimum":1}"#,
        )
        .unwrap();
        let spec = json!({"openapi":"3.1.0","servers":[{"url":"https://example.com"}],"paths":{"/":{"post":{"operationId":"tree","requestBody":{"content":{"application/json":{"schema":{"$ref":"schemas/root.json"}}}},"responses":{}}}}});
        std::fs::write(t.path().join("api.json"), spec.to_string()).unwrap();
        let state = crate::test_support::app_state(t.path());
        let owner = state.database().unwrap().create_conversation().unwrap().id;
        let ctx = crate::development_tools::tests::context(&state, t.path(), &owner, &[]);
        assert!(prepare(
            &ctx,
            json!({"path":"api.json","operation_id":"tree","body":{"leaf":1,"child":{"leaf":2}}})
        )
        .await
        .is_ok());
        assert!(prepare(
            &ctx,
            json!({"path":"api.json","operation_id":"tree","body":{"child":{"leaf":0}}})
        )
        .await
        .is_err());
    }
    #[tokio::test]
    async fn mutually_recursive_components_preserve_31_ref_siblings() {
        let t = tempfile::tempdir().unwrap();
        let spec = json!({"openapi":"3.1.0","components":{"schemas":{
            "A":{"type":"object","properties":{"b":{"$ref":"#/components/schemas/B"}}},
            "B":{"type":"object","properties":{"a":{"$ref":"#/components/schemas/A"}}}
        }},"paths":{"/":{"get":{"operationId":"get","responses":{"200":{"content":{"application/json":{"schema":{"$ref":"#/components/schemas/A","required":["b"]}}}}}}}},"servers":[{"url":"https://example.com"}]});
        std::fs::write(t.path().join("api.json"), spec.to_string()).unwrap();
        let state = crate::test_support::app_state(t.path());
        let owner = state.database().unwrap().create_conversation().unwrap().id;
        let ctx = crate::development_tools::tests::context(&state, t.path(), &owner, &[]);
        let prepared = prepare(&ctx, json!({"path":"api.json","operation_id":"get"}))
            .await
            .unwrap();
        let schema = &prepared["response_schemas"]["200"]["application/json"];
        assert!(
            crate::tool_validation::validate_value(schema, &json!({"b":{"a":{"b":{}}}})).is_ok()
        );
        assert!(crate::tool_validation::validate_value(schema, &json!({})).is_err());
    }
    #[test]
    fn yaml_schema_and_encoding() {
        let v = parse("openapi: 3.0.3\npaths: {}\n").unwrap();
        assert_eq!(v["openapi"], "3.0.3");
        assert_eq!(encode("a/b &"), "a%2Fb%20%26");
        let s = schema(json!({"type":"string","nullable":true}), true);
        assert!(crate::tool_validation::compile(&s)
            .unwrap()
            .is_valid(&Value::Null));
    }
}
