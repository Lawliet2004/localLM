//! Structured code-review findings.
//!
//! A code-review verdict is a claim about one file: what is wrong, and the
//! failure scenario that makes it matter. localLM already computes the
//! evidence — `change_impact` says what a change touches, `verification_tools`
//! says what the tests actually prove — but there was no channel that carried
//! a precise verdict from the model to whoever reviews the work. These tools
//! are that channel.
//!
//! Findings are durable per conversation: a later turn lists them, sees which
//! are still open, and closes them with a note. They are a verdict, not an
//! assertion of truth: the model is told not to report a finding it cannot
//! tie to a concrete failure.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Aliases owned by this module.
pub const ALIASES: [&str; 3] = ["report_findings", "list_findings", "resolve_finding"];

/// Whether an alias belongs to this module.
pub fn owns(alias: &str) -> bool {
    ALIASES.contains(&alias)
}

/// The review categories a finding may declare.
pub const CATEGORIES: [&str; 5] = [
    "correctness",
    "security",
    "performance",
    "test-coverage",
    "maintainability",
];

/// A finding nobody has closed yet.
pub const OPEN: &str = "open";
/// A finding the reviewer closed.
pub const RESOLVED: &str = "resolved";

/// Most findings one call may report.
const MAX_FINDINGS: usize = 32;
/// Longest file path a finding may name.
const MAX_FILE_CHARS: usize = 4096;
/// Longest one-line summary.
const MAX_SUMMARY_CHARS: usize = 500;
/// Longest failure scenario.
const MAX_SCENARIO_CHARS: usize = 2000;
/// Longest resolution note.
const MAX_NOTE_CHARS: usize = 1000;
/// Longest finding id.
const MAX_ID_CHARS: usize = 128;

/// One stored code-review finding.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Finding {
    /// Stable id handed back to the model.
    pub id: String,
    /// The conversation the finding belongs to.
    pub conversation_id: String,
    /// Workspace-relative path of the file the finding is about.
    pub file: String,
    /// Optional 1-based line number in that file.
    pub line: Option<i64>,
    /// One line: what is wrong.
    pub summary: String,
    /// What breaks, and when it breaks.
    pub failure_scenario: String,
    /// Optional review category.
    pub category: Option<String>,
    /// `open` or `resolved`.
    pub state: String,
    /// What was done about the finding, when it was resolved.
    pub note: Option<String>,
    /// Creation timestamp in milliseconds since the epoch.
    pub created_at: i64,
}

/// Model-facing tool definitions.
pub fn registry() -> Vec<crate::tool_spec::ToolSpec> {
    let finding_input = json!({
        "type":"object",
        "required":["file","summary","failure_scenario"],
        "properties":{
            "file":{"type":"string","minLength":1,"maxLength":MAX_FILE_CHARS,
                "description":"Workspace-relative path of the file the finding is about."},
            "summary":{"type":"string","minLength":1,"maxLength":MAX_SUMMARY_CHARS,
                "description":"One line: what is wrong."},
            "failure_scenario":{"type":"string","minLength":1,"maxLength":MAX_SCENARIO_CHARS,
                "description":"What breaks, and when it breaks."},
            "category":{"type":"string","enum":CATEGORIES,
                "description":"Optional review category."},
            "line":{"type":"integer","minimum":1,
                "description":"Optional 1-based line number in the file."}
        }
    });
    let finding_output = json!({
        "type":"object",
        "required":["id","file","summary","failure_scenario","state","created_at"],
        "properties":{
            "id":{"type":"string"},
            "file":{"type":"string"},
            "line":{"type":["integer","null"]},
            "summary":{"type":"string"},
            "failure_scenario":{"type":"string"},
            "category":{"type":["string","null"]},
            "state":{"type":"string"},
            "created_at":{"type":"integer"}
        }
    });
    vec![
        crate::tool_spec::ToolSpec::new(
            "report_findings",
            "Report structured code-review findings for this workspace. Each finding names the file, what is wrong, and the failure scenario that makes it matter. Use this to hand a precise verdict; do not report a finding you cannot tie to a concrete failure.",
            json!({
                "type":"object",
                "required":["findings"],
                "properties":{
                    "findings":{
                        "type":"array",
                        "minItems":1,
                        "maxItems":MAX_FINDINGS,
                        "items":finding_input,
                        "description":"The findings of this review, one verdict each."
                    }
                },
                "additionalProperties":false
            }),
        )
        .with_output(json!({
            "type":"object",
            "required":["reported","findings"],
            "properties":{
                "reported":{"type":"integer","minimum":0},
                "findings":{"type":"array","items":finding_output}
            },
            "additionalProperties":false
        })),
        crate::tool_spec::ToolSpec::new(
            "list_findings",
            "List the code-review findings reported for this conversation. Use this before reporting a duplicate.",
            json!({
                "type":"object",
                "properties":{
                    "state":{"type":"string","enum":["open","resolved","all"],"default":"open",
                        "description":"Which findings to list. Open findings by default."}
                },
                "additionalProperties":false
            }),
        )
        .with_output(json!({
            "type":"object",
            "required":["findings"],
            "properties":{"findings":{"type":"array","items":finding_output}},
            "additionalProperties":false
        })),
        crate::tool_spec::ToolSpec::new(
            "resolve_finding",
            "Mark a reported finding as resolved, optionally recording what was done.",
            json!({
                "type":"object",
                "required":["id"],
                "properties":{
                    "id":{"type":"string","minLength":1,"maxLength":MAX_ID_CHARS,
                        "description":"The id of the finding to close."},
                    "note":{"type":"string","minLength":1,"maxLength":MAX_NOTE_CHARS,
                        "description":"Optional record of what was done about the finding."}
                },
                "additionalProperties":false
            }),
        )
        .with_output(json!({
            "type":"object",
            "required":["id","state"],
            "properties":{
                "id":{"type":"string"},
                "state":{"type":"string"},
                "note":{"type":["string","null"]}
            },
            "additionalProperties":false
        })),
    ]
}

/// Execute one of the module's tools.
pub async fn execute(
    ctx: &crate::harness::HarnessCtx<'_>,
    alias: &str,
    args: Value,
) -> Result<Value, String> {
    let store = ctx.state.database()?;
    dispatch(&store, &ctx.conversation_id, alias, args)
}

/// Run one tool against an explicit store.
///
/// The harness path and the tests share this: validation, dispatch and the
/// store call are the same code in both.
fn dispatch(
    store: &crate::store::Store,
    conversation_id: &str,
    alias: &str,
    args: Value,
) -> Result<Value, String> {
    let catalog = registry();
    let spec = crate::tool_spec::find(&catalog, alias)
        .ok_or_else(|| format!("Unknown findings tool '{alias}'."))?;
    crate::tool_validation::validate(&spec.input, &args)?;
    match alias {
        "report_findings" => report(store, conversation_id, &args),
        "list_findings" => list(store, conversation_id, &args),
        "resolve_finding" => resolve(store, conversation_id, &args),
        other => Err(format!("Unknown findings tool '{other}'.")),
    }
}

/// Report one batch of findings for this conversation.
fn report(
    store: &crate::store::Store,
    conversation_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let items = args
        .get("findings")
        .and_then(Value::as_array)
        .ok_or("report_findings needs a 'findings' array.")?;
    if items.is_empty() || items.len() > MAX_FINDINGS {
        return Err(format!(
            "report_findings accepts 1-{MAX_FINDINGS} findings per call."
        ));
    }
    let mut stored = Vec::with_capacity(items.len());
    for item in items {
        let finding = parse_finding(conversation_id, item)?;
        store.insert_finding(&finding)?;
        stored.push(finding);
    }
    Ok(json!({
        "reported": stored.len(),
        "findings": stored.iter().map(view).collect::<Vec<Value>>(),
    }))
}

/// List the findings of this conversation.
fn list(store: &crate::store::Store, conversation_id: &str, args: &Value) -> Result<Value, String> {
    let state = args
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or(OPEN)
        .trim();
    let filter = match state {
        OPEN => Some(OPEN),
        RESOLVED => Some(RESOLVED),
        "all" => None,
        other => {
            return Err(format!(
                "Unknown finding state '{other}'. Use open, resolved, or all."
            ))
        }
    };
    let rows = store.list_findings(conversation_id, filter)?;
    Ok(json!({
        "findings": rows.iter().map(view).collect::<Vec<Value>>(),
    }))
}

/// Close one finding and record what was done about it.
fn resolve(
    store: &crate::store::Store,
    conversation_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let id = args
        .get("id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty() && id.chars().count() <= MAX_ID_CHARS)
        .ok_or("resolve_finding needs an 'id' string.")?;
    let note = match args.get("note") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let text = value
                .as_str()
                .ok_or("resolve_finding 'note' must be a string.")?;
            let trimmed = text.trim();
            if trimmed.is_empty() || trimmed.chars().count() > MAX_NOTE_CHARS {
                return Err(format!(
                    "resolve_finding 'note' must be 1-{MAX_NOTE_CHARS} characters."
                ));
            }
            Some(trimmed.to_string())
        }
    };
    let finding = store.resolve_finding(conversation_id, id, note.as_deref())?;
    Ok(json!({
        "id": finding.id,
        "state": finding.state,
        "note": finding.note,
    }))
}

/// Read one reported finding into a stored row.
fn parse_finding(conversation_id: &str, item: &Value) -> Result<Finding, String> {
    let file = bounded(item.get("file"), "file", MAX_FILE_CHARS)?;
    let summary = bounded(item.get("summary"), "summary", MAX_SUMMARY_CHARS)?;
    let failure_scenario = bounded(
        item.get("failure_scenario"),
        "failure_scenario",
        MAX_SCENARIO_CHARS,
    )?;
    let category = match item.get("category") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let slug = value
                .as_str()
                .ok_or("Finding 'category' must be a string.")?;
            if !CATEGORIES.contains(&slug) {
                return Err(format!(
                    "Unknown finding category '{slug}'. Use one of: {}.",
                    CATEGORIES.join(", ")
                ));
            }
            Some(slug.to_string())
        }
    };
    let line = match item.get("line") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let number = value.as_i64().ok_or("Finding 'line' must be an integer.")?;
            if number < 1 {
                return Err("Finding 'line' must be 1 or greater.".into());
            }
            Some(number)
        }
    };
    Ok(Finding {
        id: new_id(),
        conversation_id: conversation_id.to_string(),
        file,
        line,
        summary,
        failure_scenario,
        category,
        state: OPEN.to_string(),
        note: None,
        created_at: crate::store::now(),
    })
}

/// The model-visible shape of one finding.
fn view(finding: &Finding) -> Value {
    json!({
        "id": finding.id,
        "file": finding.file,
        "line": finding.line,
        "summary": finding.summary,
        "failure_scenario": finding.failure_scenario,
        "category": finding.category,
        "state": finding.state,
        "created_at": finding.created_at,
    })
}

/// A short, stable, human-quotable id.
fn new_id() -> String {
    let raw = uuid::Uuid::new_v4().simple().to_string();
    format!("fnd_{}", &raw[..8])
}

/// One bounded text field of a finding.
fn bounded(value: Option<&Value>, label: &str, max: usize) -> Result<String, String> {
    let text = value
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Finding '{label}' must be a string."))?;
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.chars().count() > max {
        return Err(format!("Finding '{label}' must be 1-{max} characters."));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn conversation_store() -> (crate::store::Store, String) {
        let store = crate::store::Store::open_memory().unwrap();
        let conversation = store.create_conversation().unwrap().id;
        (store, conversation)
    }

    fn report(
        store: &crate::store::Store,
        conversation: &str,
        findings: Value,
    ) -> Result<Value, String> {
        dispatch(
            store,
            conversation,
            "report_findings",
            json!({ "findings": findings }),
        )
    }

    fn one_finding() -> Value {
        json!({
            "file":"src/lib.rs",
            "summary":"Off-by-one in the slice window",
            "failure_scenario":"The last element is dropped when the window ends exactly at the end of the slice.",
            "category":"correctness",
            "line":42
        })
    }

    #[test]
    fn reporting_one_finding_stores_it_with_an_id() {
        let (store, conversation) = conversation_store();
        let result = report(&store, &conversation, json!([one_finding()])).unwrap();
        assert_eq!(result["reported"], 1);
        let id = result["findings"][0]["id"].as_str().unwrap().to_string();
        assert!(id.starts_with("fnd_"), "{id}");
        assert_eq!(result["findings"][0]["state"], "open");
        assert_eq!(result["findings"][0]["file"], "src/lib.rs");
        assert_eq!(result["findings"][0]["line"], 42);
        let stored = store.list_findings(&conversation, None).unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, id);
        assert_eq!(stored[0].summary, "Off-by-one in the slice window");
        assert_eq!(stored[0].category.as_deref(), Some("correctness"));
    }

    #[test]
    fn thirty_two_findings_are_accepted_and_thirty_three_are_not() {
        let (store, conversation) = conversation_store();
        let build = |count: usize| {
            Value::Array(
                (0..count)
                    .map(|index| {
                        json!({
                            "file":"src/lib.rs",
                            "summary":format!("finding {index}"),
                            "failure_scenario":"It breaks on the next call."
                        })
                    })
                    .collect(),
            )
        };
        let accepted = report(&store, &conversation, build(32));
        assert!(accepted.is_ok(), "{accepted:?}");
        let rejected = report(&store, &conversation, build(33));
        assert!(rejected.is_err(), "{rejected:?}");
        assert_eq!(store.list_findings(&conversation, None).unwrap().len(), 32);
    }

    #[test]
    fn a_finding_without_file_summary_or_scenario_is_rejected() {
        let (store, conversation) = conversation_store();
        for missing in ["file", "summary", "failure_scenario"] {
            let mut item = one_finding();
            item.as_object_mut().unwrap().remove(missing);
            let error = report(&store, &conversation, json!([item])).unwrap_err();
            assert!(!error.trim().is_empty(), "{missing}");
        }
        assert!(store.list_findings(&conversation, None).unwrap().is_empty());
    }

    #[test]
    fn an_unknown_category_is_rejected() {
        let (store, conversation) = conversation_store();
        let mut item = one_finding();
        item["category"] = json!("style");
        let error = report(&store, &conversation, json!([item])).unwrap_err();
        assert!(!error.trim().is_empty());
        assert!(store.list_findings(&conversation, None).unwrap().is_empty());
    }

    #[test]
    fn list_findings_returns_open_by_default_and_all_or_resolved_on_request() {
        let (store, conversation) = conversation_store();
        let first = report(&store, &conversation, json!([one_finding()])).unwrap();
        let first_id = first["findings"][0]["id"].as_str().unwrap().to_string();
        let second = report(
            &store,
            &conversation,
            json!([{
                "file":"src/other.rs",
                "summary":"Another issue",
                "failure_scenario":"It panics on empty input."
            }]),
        )
        .unwrap();
        let second_id = second["findings"][0]["id"].as_str().unwrap().to_string();

        let open = dispatch(&store, &conversation, "list_findings", json!({})).unwrap();
        assert_eq!(open["findings"].as_array().unwrap().len(), 2);

        dispatch(
            &store,
            &conversation,
            "resolve_finding",
            json!({"id": first_id}),
        )
        .unwrap();

        let open = dispatch(&store, &conversation, "list_findings", json!({})).unwrap();
        let open_ids: Vec<&str> = open["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|finding| finding["id"].as_str().unwrap())
            .collect();
        assert_eq!(open_ids, vec![second_id]);

        let resolved = dispatch(
            &store,
            &conversation,
            "list_findings",
            json!({"state":"resolved"}),
        )
        .unwrap();
        let resolved_ids: Vec<&str> = resolved["findings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|finding| finding["id"].as_str().unwrap())
            .collect();
        assert_eq!(resolved_ids, vec![first_id]);

        let all = dispatch(
            &store,
            &conversation,
            "list_findings",
            json!({"state":"all"}),
        )
        .unwrap();
        assert_eq!(all["findings"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn resolving_a_finding_marks_it_resolved_and_records_the_note() {
        let (store, conversation) = conversation_store();
        let reported = report(&store, &conversation, json!([one_finding()])).unwrap();
        let id = reported["findings"][0]["id"].as_str().unwrap().to_string();
        let resolved = dispatch(
            &store,
            &conversation,
            "resolve_finding",
            json!({"id": id, "note":"Guarded the window end."}),
        )
        .unwrap();
        assert_eq!(resolved["id"], id.as_str());
        assert_eq!(resolved["state"], "resolved");
        assert_eq!(resolved["note"], "Guarded the window end.");
        let stored = store.finding(&conversation, &id).unwrap().unwrap();
        assert_eq!(stored.state, "resolved");
        assert_eq!(stored.note.as_deref(), Some("Guarded the window end."));
    }

    #[test]
    fn resolving_an_unknown_finding_errors() {
        let (store, conversation) = conversation_store();
        let error = dispatch(
            &store,
            &conversation,
            "resolve_finding",
            json!({"id":"fnd_00000000"}),
        )
        .unwrap_err();
        assert!(
            error.contains("fnd_00000000") || !error.trim().is_empty(),
            "{error}"
        );
    }

    #[test]
    fn findings_are_scoped_to_their_conversation() {
        let (store, first) = conversation_store();
        let second = store.create_conversation().unwrap().id;
        report(&store, &first, json!([one_finding()])).unwrap();
        let listed = dispatch(&store, &second, "list_findings", json!({})).unwrap();
        assert!(listed["findings"].as_array().unwrap().is_empty());
        let all = dispatch(&store, &first, "list_findings", json!({})).unwrap();
        assert_eq!(all["findings"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn the_registry_declares_a_result_contract_for_every_tool() {
        let catalog = registry();
        assert_eq!(catalog.len(), 3);
        for spec in &catalog {
            assert!(spec.has_output_contract(), "{}", spec.name);
            assert!(spec.validate_output(&json!({})).is_err(), "{}", spec.name);
        }
        assert!(owns("report_findings") && owns("list_findings") && owns("resolve_finding"));
        assert!(!owns("todo_write"));
    }

    #[tokio::test]
    async fn the_harness_path_dispatches_and_satisfies_the_declared_contract() {
        let directory = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(directory.path());
        let conversation = state.database().unwrap().create_conversation().unwrap().id;
        let ctx = crate::harness::HarnessCtx {
            state: &state,
            snapshot: crate::test_support::snapshot(&state, directory.path(), &[]),
            conversation_id: conversation,
            run_id: "findings-fixture".into(),
            step_id: "findings-step".into(),
            access_mode: crate::permissions::AccessMode::FullAccess,
            depth: 0,
            preset_id: crate::presets::STANDARD.into(),
            inherit_tools: &[],
            emit: std::sync::Arc::new(|_, _| {}),
        };
        let catalog = registry();
        let contract = |alias: &str| {
            crate::tool_spec::find(&catalog, alias)
                .unwrap()
                .output
                .clone()
                .unwrap()
        };
        let reported = execute(
            &ctx,
            "report_findings",
            json!({ "findings": [one_finding()] }),
        )
        .await
        .unwrap();
        crate::tool_validation::validate_value(&contract("report_findings"), &reported).unwrap();
        assert_eq!(reported["reported"], 1);
        let id = reported["findings"][0]["id"].as_str().unwrap().to_string();
        let listed = execute(&ctx, "list_findings", json!({})).await.unwrap();
        crate::tool_validation::validate_value(&contract("list_findings"), &listed).unwrap();
        assert_eq!(listed["findings"].as_array().unwrap().len(), 1);
        let resolved = execute(&ctx, "resolve_finding", json!({ "id": id }))
            .await
            .unwrap();
        crate::tool_validation::validate_value(&contract("resolve_finding"), &resolved).unwrap();
        assert_eq!(resolved["state"], "resolved");
        // An empty batch is not a verdict: it is refused.
        let error = execute(&ctx, "report_findings", json!({ "findings": [] }))
            .await
            .unwrap_err();
        assert!(!error.trim().is_empty());
    }
}
