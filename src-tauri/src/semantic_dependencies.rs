//! Conservative static import resolution. This is lexical/configuration
//! evidence, not proof of runtime module resolution or dynamic dispatch.
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
#[derive(Default, Clone)]
struct Config {
    base: String,
    base_explicit: bool,
    aliases_explicit: bool,
    alias_base: String,
    aliases: BTreeMap<String, Vec<String>>,
    sources: BTreeSet<String>,
}
fn config(
    path: &str,
    values: &BTreeMap<String, Value>,
    stack: &mut BTreeSet<String>,
) -> Result<Config, String> {
    if stack.len() >= 16 || !stack.insert(path.into()) {
        return Err("typescript_config_inheritance_cycle_or_depth".into());
    }
    let value = values
        .get(path)
        .ok_or("typescript_inherited_config_missing")?;
    let parent = Path::new(path).parent().unwrap_or(Path::new(""));
    let mut result = Config {
        base: parent.to_string_lossy().into(),
        ..Default::default()
    };
    if let Some(extends) = value.get("extends") {
        let parents = if let Some(parent) = extends.as_str() {
            vec![parent]
        } else {
            extends
                .as_array()
                .ok_or("invalid_typescript_extends")?
                .iter()
                .map(|v| v.as_str().ok_or("invalid_typescript_extends"))
                .collect::<Result<Vec<_>, _>>()?
        };
        if parents.len() > 8 {
            return Err("typescript_extends_budget".into());
        }
        for extends in parents {
            if !extends.starts_with('.') {
                return Err("external_typescript_config_unresolved".into());
            }
            let target = super::normalize(&parent.join(extends))
                .ok_or("typescript_config_outside_workspace")?;
            let target = if target.ends_with(".json") {
                target
            } else {
                format!("{target}.json")
            };
            let inherited = config(&target, values, stack)?;
            if inherited.base_explicit {
                result.base = inherited.base;
                result.base_explicit = true;
            }
            if inherited.aliases_explicit {
                result.aliases = inherited.aliases;
                result.alias_base = inherited.alias_base;
                result.aliases_explicit = true;
            }
            result.sources.extend(inherited.sources);
        }
    }
    if let Some(base) = value["compilerOptions"]["baseUrl"].as_str() {
        result.base_explicit = true;
        result.base =
            super::normalize(&parent.join(base)).ok_or("typescript_base_outside_workspace")?;
    }
    if let Some(aliases) = value["compilerOptions"]["paths"].as_object() {
        if aliases.len() > 128 {
            return Err("typescript_alias_budget".into());
        }
        result.aliases.clear();
        result.aliases_explicit = true;
        result.alias_base = parent.to_string_lossy().into();
        for (alias, targets) in aliases {
            if alias.matches('*').count() > 1 {
                return Err("typescript_multi_wildcard_alias_unsupported".into());
            }
            let targets = targets.as_array().ok_or("invalid_typescript_alias")?;
            if targets.len() > 16 {
                return Err("typescript_alias_target_budget".into());
            }
            let mut resolved = Vec::new();
            for target in targets {
                let target = target.as_str().ok_or("invalid_typescript_alias_target")?;
                if target.matches('*').count() > 1 {
                    return Err("typescript_multi_wildcard_target_unsupported".into());
                }
                resolved.push(target.to_string());
            }
            result.aliases.insert(alias.clone(), resolved);
        }
    }
    result.sources.insert(path.into());
    stack.remove(path);
    Ok(result)
}
fn candidates(path: &str, files: &BTreeSet<String>) -> Option<String> {
    let mut paths = vec![path.to_string()];
    if let Some(stem) = path.strip_suffix(".js") {
        paths.extend([format!("{stem}.ts"), format!("{stem}.tsx")]);
    }
    for ext in ["ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts", "json"] {
        paths.push(format!("{path}.{ext}"));
        paths.push(format!("{path}/index.{ext}"));
    }
    paths.into_iter().find(|p| files.contains(p))
}
pub(super) fn imports(texts: &BTreeMap<String, String>) -> (Vec<Value>, Vec<Value>) {
    let files = texts.keys().cloned().collect::<BTreeSet<_>>();
    let values = texts
        .iter()
        .filter(|(p, _)| p.ends_with(".json"))
        .filter_map(|(p, t)| super::json_with_comments(t).ok().map(|v| (p.clone(), v)))
        .collect::<BTreeMap<_, _>>();
    let configs = values
        .keys()
        .filter(|p| p.ends_with("tsconfig.json"))
        .map(|p| (p.clone(), config(p, &values, &mut BTreeSet::new())))
        .collect::<BTreeMap<_, _>>();
    let re = regex::Regex::new(
        r#"(?:\bfrom\s*|\bimport\s*\(|\brequire\s*\(|\bimport\s*)["']([^"']+)["']"#,
    )
    .unwrap();
    let mut edges = Vec::new();
    let mut warnings = Vec::new();
    for (path, text) in texts.iter().filter(|(p, _)| {
        ["ts", "tsx", "js", "jsx", "mts", "cts", "mjs", "cjs"]
            .contains(&p.rsplit('.').next().unwrap_or(""))
    }) {
        let selected = configs
            .iter()
            .filter(|(p, _)| {
                super::in_directory(
                    path,
                    Path::new(p)
                        .parent()
                        .unwrap_or(Path::new(""))
                        .to_str()
                        .unwrap_or(""),
                )
            })
            .max_by_key(|(p, _)| p.len());
        let cfg = selected.and_then(|(_, c)| c.as_ref().ok());
        if let Some((p, Err(reason))) = selected {
            warnings.push(json!({"path":p,"reason":reason}));
        }
        for capture in re.captures_iter(text).take(512) {
            let specifier = &capture[1];
            let mut resolved = None;
            if specifier.starts_with('.') {
                resolved = super::normalize(
                    &Path::new(path)
                        .parent()
                        .unwrap_or(Path::new(""))
                        .join(specifier),
                )
                .and_then(|p| candidates(&p, &files));
            } else if let Some(cfg) = cfg {
                let best = cfg
                    .aliases
                    .iter()
                    .filter_map(|(alias, targets)| {
                        if let Some((prefix, suffix)) = alias.split_once('*') {
                            let middle = specifier.strip_prefix(prefix)?.strip_suffix(suffix)?;
                            Some((prefix.len() + suffix.len(), targets, middle))
                        } else if alias == specifier {
                            Some((usize::MAX, targets, ""))
                        } else {
                            None
                        }
                    })
                    .max_by_key(|v| v.0);
                if let Some((_, targets, middle)) = best {
                    resolved = targets.iter().find_map(|target| {
                        let base = if cfg.base_explicit {
                            &cfg.base
                        } else {
                            &cfg.alias_base
                        };
                        super::normalize(&Path::new(base).join(target.replace('*', middle)))
                            .and_then(|path| candidates(&path, &files))
                    });
                    if resolved.is_none() {
                        warnings.push(json!({"path":path,"reason":"unresolved_typescript_alias","specifier":specifier}));
                    }
                } else {
                    resolved = candidates(
                        format!("{}/{specifier}", cfg.base).trim_start_matches('/'),
                        &files,
                    );
                }
            }
            if let Some(target) = resolved {
                edges.push(json!({"from":path,"to":target,"kind":"static_module_relationship","evidence":"lexical_with_config_resolution","evidenceVersion":"current","sourceSha256":crate::development_tools::hash(text.as_bytes()),"configurationSources":cfg.map(|c| c.sources.iter().map(|p| json!({"path":p,"sha256":crate::development_tools::hash(texts[p].as_bytes())})).collect::<Vec<_>>())}));
            } else {
                warnings.push(json!({"path":path,"reason":"external_or_unresolved_module","specifier":specifier}));
            }
        }
        if re.captures_iter(text).nth(512).is_some() {
            warnings.push(json!({"path":path,"reason":"module_relationship_budget"}));
        }
    }
    (edges, warnings)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multiple_extends_and_overridden_base_url_preserve_alias_semantics() {
        let texts=BTreeMap::from([
            ("base.json".into(),r#"{"compilerOptions":{"baseUrl":"old","paths":{"@lib/*":["lib/*"]}}}"#.into()),
            ("strict.json".into(),r#"{"compilerOptions":{"strict":true}}"#.into()),
            ("tsconfig.json".into(),r#"{"extends":["./base.json","./strict.json"],"compilerOptions":{"baseUrl":"new"}}"#.into()),
            ("main.ts".into(),"import {f} from '@lib/api';".into()),
            ("new/lib/api.ts".into(),"export function f() {}".into()),
        ]);
        let (edges, warnings) = imports(&texts);
        assert!(
            edges
                .iter()
                .any(|e| e["from"] == "main.ts" && e["to"] == "new/lib/api.ts"),
            "{warnings:?}"
        );
    }
    #[test]
    fn inherited_aliases_and_sibling_prefixes_resolve_conservatively() {
        let texts = BTreeMap::from([
            ("tsconfig.base.json".into(), r#"{"compilerOptions":{"baseUrl":".","paths":{"@lib/*":["packages/lib/*"]}}}"#.into()),
            ("apps/app/tsconfig.json".into(), r#"{"extends":"../../tsconfig.base.json","references":[{"path":"../../packages/lib"}]}"#.into()),
            ("apps/app/main.ts".into(), "import {f} from '@lib/api'; import './local';".into()),
            ("apps/application/not-app.ts".into(), "import {f} from '@lib/api';".into()),
            ("apps/app/local.ts".into(), "export {};".into()),
            ("packages/lib/api.ts".into(), "export function f() {}".into()),
        ]);
        let (edges, warnings) = imports(&texts);
        assert!(edges
            .iter()
            .any(|e| e["from"] == "apps/app/main.ts" && e["to"] == "packages/lib/api.ts"));
        assert!(edges.iter().any(|e| e["to"] == "apps/app/local.ts"));
        assert!(!edges
            .iter()
            .any(|e| e["from"] == "apps/application/not-app.ts"));
        assert!(warnings
            .iter()
            .any(|w| w["path"] == "apps/application/not-app.ts"));
    }
}
