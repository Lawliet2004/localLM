//! Bounded source/build-input inventory for optional check interpretation.
use serde_json::{json, Value};
pub fn relevant(path: &str) -> bool {
    if path.split('/').any(|p| {
        matches!(
            p,
            "node_modules"
                | "target"
                | "dist"
                | "build"
                | ".git"
                | ".locallm-transactions"
                | "coverage"
                | "__pycache__"
        )
    }) {
        return false;
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    matches!(
        name,
        "Cargo.toml"
            | "Cargo.lock"
            | "package.json"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "Makefile"
            | "Dockerfile"
            | "rust-toolchain.toml"
            | "AGENTS.md"
    ) || [
        "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs", "mts", "cts", "json", "toml", "yaml", "yml",
        "css", "html", "sql",
    ]
    .contains(&path.rsplit('.').next().unwrap_or(""))
}
pub fn inventory(root: &str) -> Result<Value, String> {
    let workspace = crate::workspace::Workspace::open(root)?;
    let mut cursor = None;
    let mut paths = Vec::new();
    let mut warnings = Vec::new();
    loop {
        let mut args = json!({"pattern":"**/*","limit":200,"include_hidden":true});
        if let Some(c) = cursor.take() {
            args["cursor"] = c;
        }
        let page = workspace.call("glob_files", args)?;
        if page["incomplete"] == true {
            warnings.push(json!({"reason":"inventory_limit","details":page["warnings"]}));
        }
        for entry in page["results"].as_array().into_iter().flatten() {
            if let Some(path) = entry["path"].as_str() {
                if relevant(path) {
                    paths.push(json!(path));
                    if paths.len() >= 4096 {
                        warnings.push(json!({"reason":"file_limit"}));
                        break;
                    }
                }
            }
        }
        if paths.len() >= 4096 || page["hasMore"] != true {
            break;
        }
        cursor = page.get("nextCursor").cloned();
        if cursor.is_none() {
            warnings.push(json!({"reason":"missing_cursor"}));
            break;
        }
    }
    paths.sort_by_key(Value::to_string);
    paths.dedup();
    Ok(json!({"paths":paths,"warnings":warnings,"complete":warnings.is_empty()}))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inventory_includes_nested_sources_and_manifests_and_excludes_build_outputs() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(t.path().join("src")).unwrap();
        std::fs::create_dir_all(t.path().join("target")).unwrap();
        std::fs::write(t.path().join("src/main.rs"), "fn main(){}").unwrap();
        std::fs::write(t.path().join("Cargo.toml"), "[package]\nname='test'\n").unwrap();
        std::fs::write(t.path().join("target/ignored.rs"), "ignored").unwrap();
        let inventory = inventory(t.path().to_str().unwrap()).unwrap();
        assert_eq!(inventory["complete"], true, "{inventory}");
        assert_eq!(inventory["paths"], json!(["Cargo.toml", "src/main.rs"]));

    }
}
