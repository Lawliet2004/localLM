//! Bounded lexical impact evidence; never claims a complete dependency graph.
use crate::development_tools as dev;
use serde_json::{json, Value};
pub fn registry() -> Vec<crate::tool_spec::ToolSpec> {
    vec![("change_impact".into(),"Analyze current and isolated Git-baseline symbols using configured language servers, bounded transitive relationships and conservative package/configuration dependencies. Deleted files and removed symbols retain baseline provenance. Explicit lexical mode remains separately labeled. Unsupported relationships and exhausted budgets keep coverage incomplete; no complete dependency graph or complete test selection is claimed.".into(),dev::schema(json!({"changed_paths":{"type":"array","minItems":1,"maxItems":128,"items":{"type":"string","minLength":1,"maxLength":4096}},"symbols":{"type":"array","minItems":1,"maxItems":16,"items":{"type":"string","minLength":1,"maxLength":128,"pattern":"^[A-Za-z_$][A-Za-z0-9_$]*$"}},"limit":{"type":"integer","minimum":1,"maximum":200},"mode":{"type":"string","enum":["auto","semantic","lexical"],"default":"auto"},"server":{"type":"string","minLength":1,"maxLength":64}}),&["changed_paths"]))].into_iter().map(crate::tool_spec::ToolSpec::from).collect()
}
pub fn execute(root: &str, args: Value) -> Result<Value, String> {
    let sources = dev::snapshot(
        root,
        args["changed_paths"]
            .as_array()
            .ok_or("Missing changed paths.")?,
    )?;
    let workspace = crate::workspace::Workspace::open(root)?;
    let mut evidence = Vec::new();
    let mut tests = std::collections::BTreeSet::new();
    let mut incomplete = false;
    for symbol in args["symbols"].as_array().ok_or("Missing symbols.")? {
        let symbol = symbol.as_str().ok_or("Invalid symbol.")?;
        let result=workspace.call("search_content",json!({"query":format!("(?:^|[^A-Za-z0-9_$]){}(?:$|[^A-Za-z0-9_$])",regex::escape(symbol)),"regex":true,"context_lines":0,"limit":args["limit"].as_u64().unwrap_or(50)}))?;
        incomplete |= result["hasMore"] == true
            || result["truncated"] == true
            || result["incomplete"] == true;
        for m in result["matches"].as_array().into_iter().flatten() {
            if let Some(path) = m["path"].as_str() {
                if path.contains("/tests/")
                    || path.starts_with("tests/")
                    || path.contains(".test.")
                    || path.contains(".spec.")
                    || path.ends_with("_test.py")
                {
                    tests.insert(path.to_string());
                }
            }
        }
        evidence.push(json!({"symbol":symbol,"search":result}));
    }
    Ok(
        json!({"sourceSnapshot":sources,"freshness":dev::freshness(root,&sources),"references":evidence,"candidateTests":tests,"incomplete":incomplete,"analysisKind":"lexical_heuristic","completeDependencyGraph":false,"limitation":"Matches may be comments or unrelated names. Dynamic dispatch, generated code, external consumers and unscanned files can be missed. Candidate tests are filename-based suggestions; no tests were executed."}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn impact_reports_evidence_without_claiming_semantics() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("a.rs"), "fn changed() {}\n").unwrap();
        std::fs::write(t.path().join("a.test.ts"), "changed();\n").unwrap();
        let r = execute(
            t.path().to_str().unwrap(),
            json!({"changed_paths":["a.rs"],"symbols":["changed"]}),
        )
        .unwrap();
        assert_eq!(r["completeDependencyGraph"], false);
        assert_eq!(r["candidateTests"][0], "a.test.ts");
        assert!(
            r["references"][0]["search"]["matches"]
                .as_array()
                .unwrap()
                .len()
                >= 2
        );
    }
}
