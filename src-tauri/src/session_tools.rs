//! Model-facing session, worktree and model management.
//!
//! OpenCode V2 exposes a namespaced family for managing the harness itself
//! (`opencode_session_rename`, `opencode_session_move`,
//! `opencode_worktree_list`, `opencode_models`). localLM keeps the same
//! capabilities under its own aliases: the model can rename the conversation
//! it is in, move that conversation to another directory, list the
//! repository's worktrees, and look up the exact provider/model reference it
//! needs before delegating to a subagent.
//!
//! Every tool here is a read of app state the model already owns (its own
//! conversation, its own workspace) or a write to the same store row the UI
//! command writes, so a model move and a user move can never disagree.

use serde_json::{json, Value};

/// Aliases owned by this module.
pub const ALIASES: [&str; 4] = ["session_rename", "session_move", "worktree_list", "models"];

/// Whether an alias belongs to this module.
pub fn owns(alias: &str) -> bool {
    ALIASES.contains(&alias)
}

/// Upper bound on the worktrees one listing returns.
const MAX_WORKTREES: usize = 256;
/// Default page size of the model catalog.
const DEFAULT_MODEL_LIMIT: usize = 20;
/// Largest page size the catalog accepts.
const MAX_MODEL_LIMIT: usize = 100;
/// Title limit shared with the store and the UI rename command.
const MAX_TITLE_CHARS: usize = 160;

/// One worktree record parsed from `git worktree list --porcelain`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Worktree {
    /// Absolute path of the worktree.
    pub path: String,
    /// Short branch name; empty for a detached HEAD.
    pub branch: String,
    /// Commit the worktree points at.
    pub head: String,
    /// Whether the worktree is locked.
    pub locked: bool,
}

/// One catalog entry: a model of one provider.
#[derive(Clone, Debug, PartialEq, Eq)]
struct CatalogModel {
    provider_id: String,
    provider_name: String,
    model_id: String,
    name: String,
    released: i64,
    variants: Vec<String>,
}

/// Model-facing tool definitions.
pub fn registry() -> Vec<crate::tool_spec::ToolSpec> {
    let object = |properties: Value, required: &[&str]| {
        json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
    };
    let worktree_item = json!({
        "type":"object",
        "required":["path","branch","head","locked"],
        "properties":{
            "path":{"type":"string"},
            "branch":{"type":"string","description":"Short branch name; empty for a detached HEAD."},
            "head":{"type":"string"},
            "locked":{"type":"boolean"}
        }
    });
    let model_item = json!({
        "type":"object",
        "required":["id","name","released","variants"],
        "properties":{
            "id":{"type":"string","description":"provider/model reference."},
            "name":{"type":"string"},
            "released":{"type":"integer","minimum":0,
                "description":"Release date as a Unix timestamp in seconds, or 0 when unknown."},
            "variants":{"type":"array","items":{"type":"string"},
                "description":"Capability variants known for this model."}
        }
    });
    let provider_item = json!({
        "type":"object",
        "required":["id","name","models"],
        "properties":{
            "id":{"type":"string"},
            "name":{"type":"string"},
            "models":{"type":"array","items":model_item}
        }
    });
    vec![
        crate::tool_spec::ToolSpec::new(
            "session_rename",
            "Rename the current conversation to a short, specific title that summarizes the work. Omit nothing; this always renames the current conversation.",
            object(
                json!({"title":{"type":"string","minLength":1,"maxLength":200,
                    "description":"The new conversation title. Surrounding whitespace is removed."}}),
                &["title"],
            ),
        )
        .with_output(object(json!({"title":{"type":"string"}}), &["title"])),
        crate::tool_spec::ToolSpec::new(
            "session_move",
            "Move the current conversation to another directory so later file and shell work happens there. The directory must already exist and be readable. The current session moves at the next safe boundary; do not run destination-dependent tools in the same turn.",
            object(
                json!({"directory":{"type":"string","minLength":1,"maxLength":4096,
                    "description":"Absolute path of an existing, readable directory."}}),
                &["directory"],
            ),
        )
        .with_output(object(json!({"directory":{"type":"string"}}), &["directory"])),
        crate::tool_spec::ToolSpec::new(
            "worktree_list",
            "List the repository's worktrees for this workspace. Use worktree_enter to switch into one.",
            object(json!({}), &[]),
        )
        .with_output(object(
            json!({"worktrees":{"type":"array","maxItems":256,"items":worktree_item}}),
            &["worktrees"],
        )),
        crate::tool_spec::ToolSpec::new(
            "models",
            "Search the models available to use. Use this to turn a model name the user mentions into an exact reference before running a subagent on it. Returns provider id, model id, name, release date and available variants.",
            object(
                json!({"query":{"type":"string","minLength":1,"maxLength":200,
                    "description":"Text to search for in model names and ids. Every whitespace-separated term must match."},
                    "provider":{"type":"string","minLength":1,"maxLength":120,
                    "description":"Provider id or name to filter by."},
                    "limit":{"type":"integer","minimum":1,"maximum":100,"default":DEFAULT_MODEL_LIMIT},
                    "offset":{"type":"integer","minimum":0,"default":0}}),
                &[],
            ),
        )
        .with_output(object(
            json!({
                "providers":{"type":"array","items":provider_item},
                "total":{"type":"integer","minimum":0},
                "next":{"type":["integer","null"],"minimum":0,
                    "description":"Offset of the next page, or null on the last page."}
            }),
            &["providers","total","next"],
        )),
    ]
}

/// Execute one of the module's tools.
pub async fn execute(
    ctx: &crate::harness::HarnessCtx<'_>,
    alias: &str,
    args: Value,
) -> Result<Value, String> {
    let catalog = registry();
    let spec = crate::tool_spec::find(&catalog, alias)
        .ok_or_else(|| format!("Unknown session tool '{alias}'."))?;
    crate::tool_validation::validate(&spec.input, &args)?;
    match alias {
        "session_rename" => {
            let store = ctx.state.database()?;
            rename(&store, &ctx.conversation_id, &args)
        }
        "session_move" => {
            let store = ctx.state.database()?;
            move_conversation(&store, &ctx.conversation_id, &args)
        }
        "worktree_list" => list_worktrees(&ctx.snapshot.workspace_path).await,
        "models" => {
            let store = ctx.state.database()?;
            let providers = store.providers()?;
            let current = store
                .conversation_model(&ctx.conversation_id)
                .ok()
                .and_then(|(selection, _)| selection.provider_id);
            Ok(search_models(&providers, current.as_deref(), &args))
        }
        _ => Err(format!("Unknown session tool '{alias}'.")),
    }
}

/// Rename the conversation the model is currently in.
fn rename(
    store: &crate::store::Store,
    conversation_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let title = args
        .get("title")
        .and_then(Value::as_str)
        .ok_or("session_rename needs a 'title' string.")?;
    let title = title.trim();
    if title.is_empty() {
        return Err("Title must contain at least one visible character.".into());
    }
    // The store and the UI rename command cap the title at 160 characters;
    // the declared schema is wider, so the real limit is reported here.
    if title.chars().count() > MAX_TITLE_CHARS {
        return Err(format!("Title must contain 1–{MAX_TITLE_CHARS} characters."));
    }
    store.rename_conversation(conversation_id, title)?;
    Ok(json!({"title": title}))
}

/// Record a new working directory for the conversation.
///
/// No file is moved: the destination only becomes the directory later file
/// and shell work happens in.
fn move_conversation(
    store: &crate::store::Store,
    conversation_id: &str,
    args: &Value,
) -> Result<Value, String> {
    let directory = args
        .get("directory")
        .and_then(Value::as_str)
        .ok_or("session_move needs a 'directory' string.")?;
    let path = std::path::Path::new(directory);
    if !path.is_absolute() {
        return Err("Directory must be an absolute path.".into());
    }
    let metadata =
        std::fs::metadata(path).map_err(|error| format!("Cannot open directory: {error}"))?;
    if !metadata.is_dir() {
        return Err("Directory must be a directory, not a file.".into());
    }
    std::fs::read_dir(path).map_err(|error| format!("Directory is not readable: {error}"))?;
    // The same store path the UI command uses, so the model and the user
    // share one workspace row instead of two competing truths.
    crate::workspace_ui::select_task_workspace(store, directory, Some(conversation_id))?;
    let canonical = std::fs::canonicalize(path)
        .map(|resolved| resolved.to_string_lossy().into_owned())
        .unwrap_or_else(|_| directory.to_string());
    Ok(json!({"directory": canonical}))
}

/// List the worktrees of the repository that owns the workspace.
///
/// A workspace that is not inside a repository has no worktrees: Git is not
/// asked to run there and the listing is empty, not an error.
async fn list_worktrees(root: &str) -> Result<Value, String> {
    if root.is_empty() || !crate::git_tools::available() {
        return Ok(json!({"worktrees": []}));
    }
    if crate::git_tools::repository_root_of(root).await?.is_none() {
        return Ok(json!({"worktrees": []}));
    }
    let output = crate::workspace_ui::git(root, &["worktree", "list", "--porcelain"]).await?;
    let worktrees: Vec<Value> = parse_worktree_porcelain(&output)
        .into_iter()
        .map(|worktree| {
            json!({
                "path": worktree.path,
                "branch": worktree.branch,
                "head": worktree.head,
                "locked": worktree.locked,
            })
        })
        .collect();
    Ok(json!({"worktrees": worktrees}))
}

/// Parse `git worktree list --porcelain` output.
///
/// Records are separated by blank lines; each carries `worktree <path>`,
/// `HEAD <sha>` and either `branch <ref>` or `detached`, plus the optional
/// `bare`, `locked` and `prunable` markers.
fn parse_worktree_porcelain(output: &str) -> Vec<Worktree> {
    let mut worktrees: Vec<Worktree> = Vec::new();
    let mut current: Option<Worktree> = None;
    for line in output.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.trim().is_empty() {
            if let Some(entry) = current.take() {
                worktrees.push(entry);
            }
            continue;
        }
        let (key, value) = match line.split_once(' ') {
            Some((key, value)) => (key, value),
            None => (line, ""),
        };
        match key {
            "worktree" => {
                if let Some(entry) = current.take() {
                    worktrees.push(entry);
                }
                current = Some(Worktree {
                    path: value.to_string(),
                    branch: String::new(),
                    head: String::new(),
                    locked: false,
                });
            }
            "HEAD" => {
                if let Some(entry) = current.as_mut() {
                    entry.head = value.to_string();
                }
            }
            "branch" => {
                if let Some(entry) = current.as_mut() {
                    entry.branch = value
                        .strip_prefix("refs/heads/")
                        .unwrap_or(value)
                        .to_string();
                }
            }
            "locked" => {
                if let Some(entry) = current.as_mut() {
                    entry.locked = true;
                }
            }
            // `detached` (no branch), `bare` and `prunable` carry no field here.
            _ => {}
        }
    }
    if let Some(entry) = current.take() {
        worktrees.push(entry);
    }
    worktrees.truncate(MAX_WORKTREES);
    worktrees
}

/// Search the configured provider models.
///
/// The catalog is the app's own provider connections: the model id is the
/// exact reference a subagent needs, and nothing here is invented.
fn search_models(
    providers: &[crate::providers::ProviderConnection],
    current_provider: Option<&str>,
    args: &Value,
) -> Value {
    let terms: Vec<String> = args
        .get("query")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .split_whitespace()
        .map(|term| term.to_lowercase())
        .collect();
    let provider_filter = args
        .get("provider")
        .and_then(Value::as_str)
        .map(|value| value.trim().to_lowercase())
        .filter(|value| !value.is_empty());
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .map(|value| value.clamp(1, MAX_MODEL_LIMIT as u64) as usize)
        .unwrap_or(DEFAULT_MODEL_LIMIT);
    let offset = args
        .get("offset")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        .min(usize::MAX as u64) as usize;

    let mut entries: Vec<CatalogModel> = Vec::new();
    for provider in providers {
        let provider_id = provider.id.trim().to_string();
        let provider_name = provider.name.trim().to_string();
        if provider_id.is_empty() {
            continue;
        }
        if let Some(filter) = &provider_filter {
            if !provider_id.to_lowercase().contains(filter.as_str())
                && !provider_name.to_lowercase().contains(filter.as_str())
            {
                continue;
            }
        }
        for model in &provider.models {
            let model_id = model.id.trim().to_string();
            if model_id.is_empty() {
                continue;
            }
            // The searchable text is the provider/model reference and the
            // model name; every term must appear in one of them.
            let haystack = format!("{provider_id}/{model_id} {model_id}").to_lowercase();
            if !terms.iter().all(|term| haystack.contains(term.as_str())) {
                continue;
            }
            entries.push(CatalogModel {
                provider_id: provider_id.clone(),
                provider_name: provider_name.clone(),
                name: model_id.clone(),
                model_id,
                // No release date is recorded for a provider model.
                released: 0,
                variants: variants_of(model),
            });
        }
    }

    // The conversation's own provider comes first, then provider id, then
    // newest release; the model id breaks the remaining ties.
    let current = current_provider.map(|id| id.trim().to_lowercase());
    entries.sort_by(|left, right| {
        let rank = |entry: &CatalogModel| -> u8 {
            match &current {
                Some(id) if entry.provider_id.to_lowercase() == *id => 0,
                _ => 1,
            }
        };
        rank(left)
            .cmp(&rank(right))
            .then_with(|| left.provider_id.cmp(&right.provider_id))
            .then_with(|| right.released.cmp(&left.released))
            .then_with(|| left.model_id.cmp(&right.model_id))
    });

    let total = entries.len();
    let start = offset.min(total);
    let end = start.saturating_add(limit).min(total);
    let page = &entries[start..end];

    // Group the page by provider, keeping the sorted provider order.
    let mut grouped: Vec<(String, String, Vec<&CatalogModel>)> = Vec::new();
    for entry in page {
        match grouped
            .iter_mut()
            .find(|(id, _, _)| *id == entry.provider_id)
        {
            Some((_, _, models)) => models.push(entry),
            None => grouped.push((
                entry.provider_id.clone(),
                entry.provider_name.clone(),
                vec![entry],
            )),
        }
    }
    let providers: Vec<Value> = grouped
        .into_iter()
        .map(|(id, name, models)| {
            json!({
                "id": id,
                "name": name,
                "models": models
                    .into_iter()
                    .map(|model| json!({
                        "id": format!("{}/{}", model.provider_id, model.model_id),
                        "name": model.name,
                        "released": model.released,
                        "variants": model.variants,
                    }))
                    .collect::<Vec<Value>>(),
            })
        })
        .collect();
    let next = if end < total { Some(end as u64) } else { None };
    json!({"providers": providers, "total": total, "next": next})
}

/// The capability variants the app knows for one provider model.
///
/// localLM stores no release date and no variant catalog, so a variant is
/// only reported when the stored model actually carries that capability.
fn variants_of(model: &crate::providers::RemoteModel) -> Vec<String> {
    let mut variants: Vec<String> = Vec::new();
    if model.tool_support == crate::providers::ToolSupport::Supported {
        variants.push("tools".to_string());
    }
    if model.supports_images {
        variants.push("vision".to_string());
    }
    if model
        .controls
        .as_ref()
        .is_some_and(|controls| controls.reasoning_effort == crate::providers::ToolSupport::Supported)
    {
        variants.push("reasoning".to_string());
    }
    variants
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::{ProviderConnection, RemoteModel, ToolSupport};

    fn remote_model(id: &str) -> RemoteModel {
        RemoteModel {
            controls: None,
            id: id.to_string(),
            context_length: Some(128_000),
            max_output_tokens: Some(16_384),
            supports_images: false,
            tool_support: ToolSupport::Supported,
        }
    }

    fn provider(id: &str, name: &str, models: &[&str]) -> ProviderConnection {
        ProviderConnection {
            id: id.to_string(),
            name: name.to_string(),
            api_format: crate::providers::OPENAI_RESPONSES.to_string(),
            base_url: "https://example.invalid/v1".to_string(),
            verified: true,
            last_tested_at: None,
            has_api_key: false,
            models: models
                .iter()
                .map(|model| remote_model(model))
                .collect::<Vec<_>>(),
        }
    }

    #[test]
    fn registry_declares_the_four_tools_with_compilable_contracts() {
        let catalog = registry();
        assert_eq!(catalog.len(), 4);
        for alias in ALIASES {
            let spec = crate::tool_spec::find(&catalog, alias)
                .unwrap_or_else(|| panic!("{alias} must be a registered tool"));
            assert!(!spec.description.trim().is_empty(), "{alias}");
            assert!(spec.has_output_contract(), "{alias}");
            assert!(
                crate::tool_validation::compile(&spec.input).is_ok(),
                "{alias} input schema must compile"
            );
            assert!(
                crate::tool_validation::compile(spec.output.as_ref().unwrap()).is_ok(),
                "{alias} output schema must compile"
            );
        }
        let rename = crate::tool_spec::find(&catalog, "session_rename").unwrap();
        assert_eq!(rename.input["required"], json!(["title"]));
        assert_eq!(rename.input["additionalProperties"], false);
        let list = crate::tool_spec::find(&catalog, "worktree_list").unwrap();
        assert_eq!(list.input["required"], json!([]));
        let models = crate::tool_spec::find(&catalog, "models").unwrap();
        assert_eq!(models.input["required"], json!([]));
        assert_eq!(models.input["properties"]["limit"]["maximum"], 100);
    }

    #[test]
    fn session_rename_sets_the_title_and_rejects_an_empty_one() {
        let store = crate::store::Store::open_memory().unwrap();
        let conversation = store.create_conversation().unwrap().id;
        let renamed = rename(
            &store,
            &conversation,
            &json!({"title": "  Wire the model catalog  "}),
        )
        .unwrap();
        assert_eq!(renamed, json!({"title": "Wire the model catalog"}));
        assert_eq!(
            store
                .list_conversations()
                .unwrap()
                .into_iter()
                .find(|row| row.id == conversation)
                .unwrap()
                .title,
            "Wire the model catalog"
        );
        assert!(rename(&store, &conversation, &json!({"title": "   "})).is_err());
        assert!(rename(&store, &conversation, &json!({"title": ""})).is_err());
        assert!(rename(&store, &conversation, &json!({})).is_err());
    }

    #[test]
    fn session_rename_on_an_unknown_conversation_errors() {
        let store = crate::store::Store::open_memory().unwrap();
        assert!(rename(&store, "missing-conversation", &json!({"title": "Nope"})).is_err());
    }

    #[test]
    fn session_move_requires_an_existing_readable_directory() {
        let store = crate::store::Store::open_memory().unwrap();
        let conversation = store.create_conversation().unwrap().id;
        let base = tempfile::tempdir().unwrap();
        let destination = base.path().join("elsewhere");
        std::fs::create_dir_all(&destination).unwrap();
        let file = base.path().join("file.txt");
        std::fs::write(&file, "not a directory").unwrap();

        assert!(move_conversation(
            &store,
            &conversation,
            &json!({"directory": base.path().join("missing").to_string_lossy()})
        )
        .is_err());
        assert!(move_conversation(
            &store,
            &conversation,
            &json!({"directory": file.to_string_lossy()})
        )
        .is_err());
        assert!(move_conversation(
            &store,
            &conversation,
            &json!({"directory": "relative/path"})
        )
        .is_err());

        let moved = move_conversation(
            &store,
            &conversation,
            &json!({"directory": destination.to_string_lossy()}),
        )
        .unwrap();
        let expected = std::fs::canonicalize(&destination).unwrap();
        assert_eq!(
            moved["directory"],
            json!(expected.to_string_lossy().as_ref())
        );
        assert_eq!(
            crate::workspace_ui::task_workspace(&store, &conversation).unwrap(),
            Some(expected.to_string_lossy().into_owned())
        );
        assert_eq!(store.workspace_path().unwrap(), expected.to_string_lossy());
    }

    #[tokio::test]
    async fn worktree_list_in_a_plain_directory_is_empty_not_an_error() {
        let base = tempfile::tempdir().unwrap();
        let listed = list_worktrees(base.path().to_str().unwrap()).await.unwrap();
        assert_eq!(listed, json!({"worktrees": []}));
        assert!(list_worktrees("").await.is_ok());
    }

    #[tokio::test]
    async fn worktree_list_reports_the_repository_worktrees() {
        assert!(
            crate::git_tools::available(),
            "Git is required for the worktree fixture"
        );
        let base = tempfile::tempdir().unwrap();
        let root = base.path().to_str().unwrap();
        crate::workspace_ui::git(root, &["init", "-b", "main"])
            .await
            .unwrap();
        let listed = list_worktrees(root).await.unwrap();
        let worktrees = listed["worktrees"].as_array().unwrap();
        assert_eq!(worktrees.len(), 1);
        assert_eq!(
            std::fs::canonicalize(worktrees[0]["path"].as_str().unwrap()).unwrap(),
            std::fs::canonicalize(root).unwrap()
        );
        assert_eq!(worktrees[0]["branch"], "main");
        assert_eq!(worktrees[0]["locked"], false);
        assert!(!worktrees[0]["head"].as_str().unwrap().is_empty());
    }

    #[test]
    fn worktree_porcelain_parser_reads_paths_branches_detached_and_locked() {
        let porcelain = "worktree /repo\nHEAD 1111111111111111111111111111111111111111\nbranch refs/heads/main\n\nworktree /repo/wt\nHEAD 2222222222222222222222222222222222222222\ndetached\n\nworktree /repo/locked\nHEAD 3333333333333333333333333333333333333333\nbranch refs/heads/feature/one\nlocked\n\n";
        let parsed = parse_worktree_porcelain(porcelain);
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].path, "/repo");
        assert_eq!(parsed[0].branch, "main");
        assert_eq!(parsed[0].head, "1111111111111111111111111111111111111111");
        assert!(!parsed[0].locked);
        assert_eq!(parsed[1].path, "/repo/wt");
        assert_eq!(parsed[1].branch, "");
        assert_eq!(parsed[1].head, "2222222222222222222222222222222222222222");
        assert!(!parsed[1].locked);
        assert_eq!(parsed[2].path, "/repo/locked");
        assert_eq!(parsed[2].branch, "feature/one");
        assert!(parsed[2].locked);
    }

    #[test]
    fn models_catalog_pages_filters_and_reports_an_empty_match() {
        let providers = vec![
            provider("openai", "OpenAI", &["gpt-4o", "gpt-4o-mini"]),
            provider("anthropic", "Anthropic", &["claude-sonnet"]),
        ];
        let all = search_models(&providers, None, &json!({}));
        assert_eq!(all["total"], 3);
        assert_eq!(all["next"], json!(null));
        let listed = all["providers"].as_array().unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0]["id"], "anthropic");
        assert_eq!(listed[0]["models"][0]["id"], "anthropic/claude-sonnet");
        assert_eq!(listed[0]["models"][0]["released"], 0);
        assert!(listed[0]["models"][0]["variants"].is_array());
        assert_eq!(listed[1]["id"], "openai");
        assert_eq!(listed[1]["models"].as_array().unwrap().len(), 2);

        let limited = search_models(&providers, None, &json!({"limit": 2}));
        assert_eq!(limited["total"], 3);
        assert_eq!(limited["next"], 2);
        let models: Vec<&Value> = limited["providers"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|provider| provider["models"].as_array().unwrap().iter())
            .collect();
        assert_eq!(models.len(), 2);

        let first = search_models(&providers, None, &json!({"limit": 1, "offset": 0}));
        let second = search_models(&providers, None, &json!({"limit": 1, "offset": 1}));
        let id = |page: &Value| -> String {
            page["providers"][0]["models"][0]["id"].as_str().unwrap().to_string()
        };
        assert_ne!(id(&first), id(&second));
        assert_eq!(first["next"], 1);
        assert_eq!(second["next"], 2);

        let filtered = search_models(&providers, None, &json!({"provider": "OpenAI"}));
        assert_eq!(filtered["total"], 2);
        assert_eq!(filtered["providers"].as_array().unwrap().len(), 1);
        assert_eq!(filtered["providers"][0]["id"], "openai");

        let empty = search_models(&providers, None, &json!({"query": "no-such-model"}));
        assert_eq!(empty["total"], 0);
        assert_eq!(empty["next"], json!(null));
        assert_eq!(empty["providers"], json!([]));

        let queried = search_models(&providers, None, &json!({"query": "sonnet"}));
        assert_eq!(queried["total"], 1);
        assert_eq!(queried["providers"][0]["models"][0]["id"], "anthropic/claude-sonnet");

        // The conversation's own provider is listed first.
        let current = search_models(&providers, Some("openai"), &json!({}));
        assert_eq!(current["providers"][0]["id"], "openai");
    }

    #[test]
    fn models_catalog_reads_the_configured_providers_from_the_store() {
        let store = crate::store::Store::open_memory().unwrap();
        store
            .save_provider(&provider("openai", "OpenAI", &["gpt-4o"]))
            .unwrap();
        let providers = store.providers().unwrap();
        let result = search_models(&providers, None, &json!({}));
        // The store seeds the subscription providers, so the catalog carries
        // every configured provider, not just the one saved here.
        assert!(result["total"].as_u64().unwrap() >= 1);
        let listed = result["providers"].as_array().unwrap();
        let openai = listed
            .iter()
            .find(|entry| entry["id"] == "openai")
            .expect("the saved provider must be listed");
        assert_eq!(openai["name"], "OpenAI");
        assert_eq!(openai["models"][0]["id"], "openai/gpt-4o");
        assert_eq!(openai["models"][0]["name"], "gpt-4o");
        assert_eq!(openai["models"][0]["released"], 0);
        assert_eq!(openai["models"][0]["variants"], json!(["tools"]));
    }

    #[tokio::test]
    // The database guard is explicitly dropped before the await below;
    // `await_holding_lock` does not model an explicit `drop`.
    #[allow(clippy::await_holding_lock)]
    async fn execute_dispatches_the_module_tools() {
        let base = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(base.path());
        let store = state.database().unwrap();
        let conversation = store.create_conversation().unwrap().id;
        store
            .save_provider(&provider("openai", "OpenAI", &["gpt-4o"]))
            .unwrap();
        drop(store);
        let ctx = crate::harness::HarnessCtx {
            state: &state,
            snapshot: crate::test_support::snapshot(&state, base.path(), &ALIASES),
            conversation_id: conversation.clone(),
            run_id: "session-tools-fixture".into(),
            step_id: "session-tools-step".into(),
            access_mode: crate::permissions::AccessMode::FullAccess,
            depth: 0,
            preset_id: crate::presets::STANDARD.into(),
            inherit_tools: &[],
            emit: std::sync::Arc::new(|_, _| {}),
        };
        let renamed = execute(&ctx, "session_rename", json!({"title": "Harness contract"}))
            .await
            .unwrap();
        assert_eq!(renamed, json!({"title": "Harness contract"}));
        let listed = execute(&ctx, "worktree_list", json!({})).await.unwrap();
        assert!(listed["worktrees"].is_array());
        let catalog = execute(&ctx, "models", json!({"provider": "openai"}))
            .await
            .unwrap();
        assert_eq!(catalog["total"], 1);
        assert_eq!(catalog["providers"][0]["models"][0]["id"], "openai/gpt-4o");
        assert!(execute(&ctx, "session_rename", json!({"title": " "}))
            .await
            .is_err());
        assert!(execute(&ctx, "not_a_tool", json!({})).await.is_err());
        // The declared result contracts hold for every tool.
        let catalog = registry();
        let moved = base.path().join("moved");
        std::fs::create_dir_all(&moved).unwrap();
        for spec in &catalog {
            let value = match spec.name.as_str() {
                "session_rename" => execute(&ctx, "session_rename", json!({"title": "Contract"}))
                    .await
                    .unwrap(),
                "session_move" => execute(
                    &ctx,
                    "session_move",
                    json!({"directory": moved.to_string_lossy()}),
                )
                .await
                .unwrap(),
                "worktree_list" => execute(&ctx, "worktree_list", json!({})).await.unwrap(),
                "models" => execute(&ctx, "models", json!({"provider": "openai"}))
                    .await
                    .unwrap(),
                other => panic!("{other} must be a registered tool"),
            };
            spec.validate_output(&value)
                .unwrap_or_else(|error| panic!("{}: {error}", spec.name));
        }
    }
}
