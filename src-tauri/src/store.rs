use crate::runtime_config::RuntimeConfig;
use crate::providers::{ModelSelection, ProviderConnection, RemoteModel};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

type Result<T> = std::result::Result<T, String>;
fn db_error(error: rusqlite::Error) -> String {
    format!("Local database: {error}")
}
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub updated_at: i64,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub provider_selection_required: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub error: Option<String>,
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub reasoning: String,
    pub status: String,
    pub created_at: i64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preferences {
    pub runtime_path: String,
    pub model_path: String,
    pub temperature: f64,
    pub top_p: f64,
    pub max_tokens: u32,
    pub system_prompt: String,
}
impl Default for Preferences {
    fn default() -> Self {
        Self { runtime_path: String::new(), model_path: String::new(), temperature: 1.0,
            top_p: 0.95, max_tokens: 2048, system_prompt: "You are a helpful local assistant. Be clear and accurate. If you do not know something, say so.".into() }
    }
}
impl Preferences {
    pub fn validate(&self) -> Result<()> {
        if !self.temperature.is_finite() || !(0.0..=2.0).contains(&self.temperature) {
            return Err("Temperature must be between 0 and 2.".into());
        }
        if !self.top_p.is_finite() || !(0.01..=1.0).contains(&self.top_p) {
            return Err("Top-p must be between 0.01 and 1.".into());
        }
        if !(1..=32768).contains(&self.max_tokens) {
            return Err("Maximum response length must be between 1 and 32768 tokens.".into());
        }
        if self.system_prompt.len() > 32768 {
            return Err("System prompt is too long.".into());
        }
        if self.runtime_path.len() > 32768 || self.model_path.len() > 32768 {
            return Err("File path is too long.".into());
        }
        Ok(())
    }
}

pub struct Store {
    connection: Connection,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationTools {
    #[serde(default)]
    pub access_mode: crate::permissions::AccessMode,
    pub sources: Vec<String>,
    pub tools: Vec<crate::connectors::ToolSelection>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RememberedTools {
    pub sources: Vec<String>,
    pub tools: Vec<crate::connectors::ToolSelection>,
}
fn validate_tool_selection(
    sources: &[String],
    tools: &[crate::connectors::ToolSelection],
) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    let mut count = tools.len();
    for source in sources {
        if !seen.insert(source) {
            return Err("Duplicate tool source.".into());
        }
        count += match source.as_str() {
            "__workspace" => 5,
            "__execution" => 1,
            "__daytona" => 1,
            _ => return Err("Unknown local tool source.".into()),
        };
    }
    if count > 32 {
        return Err("At most 32 tools can be selected.".into());
    }
    let mut seen = std::collections::HashSet::new();
    for tool in tools {
        if tool.connector_id.is_empty()
            || tool.connector_id.len() > 120
            || !tool
                .connector_id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || tool.tool_name.is_empty()
            || tool.tool_name.len() > 256
            || tool.tool_name.chars().any(char::is_control)
            || !seen.insert((&tool.connector_id, &tool.tool_name))
        {
            return Err("Invalid or duplicate connector tool selection.".into());
        }
    }
    Ok(())
}
impl ConversationTools {
    pub fn validate(&self) -> Result<()> {
        validate_tool_selection(&self.sources, &self.tools)
    }
}
impl RememberedTools {
    pub fn validate(&self) -> Result<()> {
        validate_tool_selection(&self.sources, &self.tools)
    }
}
const REMEMBERED_TOOLS_KEY: &str = "remembered_tools";
const PREFERRED_MODEL_KEY: &str = "preferred_model";
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        Self::initialize(Connection::open(path).map_err(db_error)?)
    }
    #[cfg(test)]
    fn open_memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory().map_err(db_error)?)
    }
    fn initialize(connection: Connection) -> Result<Self> {
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(db_error)?;
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS conversations(id TEXT PRIMARY KEY, title TEXT NOT NULL, updated_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS messages(id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                role TEXT NOT NULL CHECK(role IN ('user','assistant','tool')), content TEXT NOT NULL, reasoning TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL CHECK(status IN ('complete','streaming','interrupted','error')), created_at INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS messages_conversation ON messages(conversation_id,created_at);
            CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS conversation_tools(conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS providers(id TEXT PRIMARY KEY, name TEXT NOT NULL, api_format TEXT NOT NULL, base_url TEXT NOT NULL, verified INTEGER NOT NULL DEFAULT 0, last_tested_at INTEGER, models TEXT NOT NULL DEFAULT '[]');
            CREATE TABLE IF NOT EXISTS runs(
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                status TEXT NOT NULL,
                model_provider TEXT,
                model_id TEXT,
                checkpoint TEXT,
                error TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS runs_conversation ON runs(conversation_id, created_at);
            CREATE TABLE IF NOT EXISTS run_events(
                run_id TEXT NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
                seq INTEGER NOT NULL,
                step_id TEXT NOT NULL,
                tool_call_id TEXT,
                event_type TEXT NOT NULL,
                payload TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                PRIMARY KEY (run_id, seq)
            );
            CREATE INDEX IF NOT EXISTS run_events_run ON run_events(run_id, seq);
            CREATE TABLE IF NOT EXISTS artifacts(
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                run_id TEXT,
                tool_name TEXT NOT NULL,
                mime_type TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                sha256 TEXT NOT NULL,
                content TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS artifacts_conversation ON artifacts(conversation_id);
            CREATE TABLE IF NOT EXISTS conversation_presets(conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE, preset TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS subagent_runs(id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                parent_run_id TEXT NOT NULL, child_run_id TEXT NOT NULL, depth INTEGER NOT NULL, status TEXT NOT NULL,
                label TEXT NOT NULL, prompt TEXT NOT NULL, error TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS subagent_runs_conversation ON subagent_runs(conversation_id, created_at);
            CREATE TABLE IF NOT EXISTS todos(conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                idx INTEGER NOT NULL, text TEXT NOT NULL, status TEXT NOT NULL, updated_at INTEGER NOT NULL,
                PRIMARY KEY (conversation_id, idx));
            CREATE TABLE IF NOT EXISTS goals(conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
                objective TEXT NOT NULL, updated_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS memory_facts(id TEXT PRIMARY KEY, scope TEXT NOT NULL, fact TEXT NOT NULL,
                origin TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS memory_facts_scope ON memory_facts(scope, updated_at);
            CREATE TABLE IF NOT EXISTS schedules(id TEXT PRIMARY KEY, name TEXT NOT NULL, cron TEXT NOT NULL, task TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1, run_once INTEGER NOT NULL DEFAULT 0,
                last_run_at INTEGER, last_result TEXT, created_at INTEGER NOT NULL,
                conversation_id TEXT, allow_write INTEGER NOT NULL DEFAULT 0);
            CREATE TABLE IF NOT EXISTS plugins(name TEXT PRIMARY KEY, version TEXT NOT NULL, path TEXT NOT NULL,
                enabled INTEGER NOT NULL DEFAULT 1, sha256 TEXT NOT NULL, installed_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS compaction(conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
                cutoff INTEGER NOT NULL, artifact_id TEXT NOT NULL, created_at INTEGER NOT NULL);
            UPDATE messages SET status='interrupted' WHERE status='streaming';
            UPDATE runs SET status='cancelled' WHERE status IN ('preparing','generating','awaiting_approval','executing_tools','preparing_next_round');").map_err(db_error)?;
        let has_error: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('messages') WHERE name='error')",
                [],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if !has_error {
            connection
                .execute("ALTER TABLE messages ADD COLUMN error TEXT", [])
                .map_err(db_error)?;
        }
        for (column, definition) in [
            ("provider_id", "TEXT"),
            ("model_id", "TEXT"),
            ("provider_selection_required", "INTEGER NOT NULL DEFAULT 0"),
        ] {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM pragma_table_info('conversations') WHERE name=?1)",
                    [column],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            if !exists {
                connection
                    .execute(&format!("ALTER TABLE conversations ADD COLUMN {column} {definition}"), [])
                    .map_err(db_error)?;
            }
        }
        let store = Self { connection };
        store.seed_remembered_tools()?;
        Ok(store)
    }
    /// One-time migration: without a saved preference, seed the remembered
    /// selection from the most recently updated conversation that stored one,
    /// including an explicitly empty selection. Conversation records are never
    /// modified; an unusable stored row leaves the preference unset.
    fn seed_remembered_tools(&self) -> Result<()> {
        let exists: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM settings WHERE key=?1)",
                [REMEMBERED_TOOLS_KEY],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if exists {
            return Ok(());
        }
        let saved: Option<String> = self
            .connection
            .query_row(
                "SELECT t.value FROM conversation_tools t JOIN conversations c ON c.id=t.conversation_id
                 ORDER BY c.updated_at DESC, c.rowid DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        if let Some(text) = saved {
            if let Ok(settings) = serde_json::from_str::<ConversationTools>(&text) {
                let remembered = RememberedTools {
                    sources: settings.sources,
                    tools: settings.tools,
                };
                if remembered.validate().is_ok() {
                    self.save_remembered_tools(&remembered)?;
                }
            }
        }
        Ok(())
    }
    pub fn finish_message(
        &self,
        id: &str,
        content: &str,
        reasoning: &str,
        status: &str,
        error: Option<&str>,
    ) -> Result<()> {
        self.connection
            .execute(
                "UPDATE messages SET content=?1,reasoning=?2,status=?3,error=?4 WHERE id=?5",
                params![content, reasoning, status, error, id],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn save_run(&self, run: &crate::agent_run::RunRecord) -> Result<()> {
        self.connection
            .execute(
                "INSERT INTO runs(id, conversation_id, status, model_provider, model_id, checkpoint, error, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(id) DO UPDATE SET
                    status=excluded.status,
                    checkpoint=excluded.checkpoint,
                    error=excluded.error,
                    updated_at=excluded.updated_at",
                params![
                    run.id,
                    run.conversation_id,
                    run.status.as_str(),
                    run.model_provider,
                    run.model_id,
                    run.checkpoint,
                    run.error,
                    run.created_at,
                    run.updated_at
                ],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn update_run_status(
        &self,
        id: &str,
        status: crate::agent_run::RunState,
        error: Option<&str>,
        checkpoint: Option<&str>,
    ) -> Result<()> {
        self.connection
            .execute(
                "UPDATE runs SET status=?1, error=?2, checkpoint=COALESCE(?3, checkpoint), updated_at=?4 WHERE id=?5",
                params![status.as_str(), error, checkpoint, now(), id],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn run(&self, id: &str) -> Result<Option<crate::agent_run::RunRecord>> {
        self.connection
            .query_row(
                "SELECT id, conversation_id, status, model_provider, model_id, checkpoint, error, created_at, updated_at
                 FROM runs WHERE id=?1",
                [id],
                |row| {
                    let status_str: String = row.get(2)?;
                    let status = crate::agent_run::RunState::from_str(&status_str)
                        .unwrap_or(crate::agent_run::RunState::OutcomeUnknown);
                    Ok(crate::agent_run::RunRecord {
                        id: row.get(0)?,
                        conversation_id: row.get(1)?,
                        status,
                        model_provider: row.get(3)?,
                        model_id: row.get(4)?,
                        checkpoint: row.get(5)?,
                        error: row.get(6)?,
                        created_at: row.get(7)?,
                        updated_at: row.get(8)?,
                    })
                },
            )
            .optional()
            .map_err(db_error)
    }
    pub fn active_run(&self, conversation_id: &str) -> Result<Option<crate::agent_run::RunRecord>> {
        // Subagent child runs (checkpoint='subagent') never hijack the
        // user-visible active run, even while still executing.
        self.connection
            .query_row(
                "SELECT id, conversation_id, status, model_provider, model_id, checkpoint, error, created_at, updated_at
                 FROM runs WHERE conversation_id=?1 AND status NOT IN ('completed', 'cancelled', 'failed', 'outcome_unknown')
                 AND (checkpoint IS NULL OR checkpoint != 'subagent')
                 ORDER BY created_at DESC LIMIT 1",
                [conversation_id],
                |row| {
                    let status_str: String = row.get(2)?;
                    let status = crate::agent_run::RunState::from_str(&status_str)
                        .unwrap_or(crate::agent_run::RunState::OutcomeUnknown);
                    Ok(crate::agent_run::RunRecord {
                        id: row.get(0)?,
                        conversation_id: row.get(1)?,
                        status,
                        model_provider: row.get(3)?,
                        model_id: row.get(4)?,
                        checkpoint: row.get(5)?,
                        error: row.get(6)?,
                        created_at: row.get(7)?,
                        updated_at: row.get(8)?,
                    })
                },
            )
            .optional()
            .map_err(db_error)
    }
    pub fn append_run_event(&self, event: &crate::agent_run::RunEvent) -> Result<()> {
        let payload = serde_json::to_string(&event.payload).map_err(|e| e.to_string())?;
        self.connection
            .execute(
                "INSERT INTO run_events(run_id, seq, step_id, tool_call_id, event_type, payload, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    event.run_id,
                    event.seq as i64,
                    event.step_id,
                    event.tool_call_id,
                    event.event_type,
                    payload,
                    event.created_at
                ],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn run_events(&self, run_id: &str) -> Result<Vec<crate::agent_run::RunEvent>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT run_id, seq, step_id, tool_call_id, event_type, payload, created_at
                 FROM run_events WHERE run_id=?1 ORDER BY seq",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map([run_id], |row| {
                let payload_text: String = row.get(5)?;
                let payload: serde_json::Value =
                    serde_json::from_str(&payload_text).unwrap_or(serde_json::Value::Null);
                let seq_i64: i64 = row.get(1)?;
                Ok(crate::agent_run::RunEvent {
                    run_id: row.get(0)?,
                    seq: seq_i64 as u64,
                    step_id: row.get(2)?,
                    tool_call_id: row.get(3)?,
                    event_type: row.get(4)?,
                    payload,
                    created_at: row.get(6)?,
                })
            })
            .map_err(db_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(db_error)
    }
    pub fn save_artifact(&self, artifact: &crate::artifacts::ArtifactRecord) -> Result<()> {
        self.connection
            .execute(
                "INSERT INTO artifacts(id, conversation_id, run_id, tool_name, mime_type, size_bytes, sha256, content, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                 ON CONFLICT(id) DO NOTHING",
                params![
                    artifact.id,
                    artifact.conversation_id,
                    artifact.run_id,
                    artifact.tool_name,
                    artifact.mime_type,
                    artifact.size_bytes as i64,
                    artifact.sha256,
                    artifact.content,
                    artifact.created_at
                ],
            )
            .map_err(db_error)?;
        Ok(())
    }
        // ---- Harness extensions (presets, subagents, plans, memory, schedules, plugins) ----
    pub fn conversation_preset(&self, id: &str) -> Result<String> {
        let preset: Option<String> = self
            .connection
            .query_row(
                "SELECT p.preset FROM conversations c LEFT JOIN conversation_presets p ON p.conversation_id=c.id WHERE c.id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?
            .flatten();
        match preset {
            None => Err("Conversation no longer exists.".into()),
            Some(value) => {
                crate::presets::validate(&value)?;
                Ok(value)
            }
        }
    }
    pub fn save_conversation_preset(&self, id: &str, preset: &str) -> Result<()> {
        crate::presets::validate(preset)?;
        let changed = self
            .connection
            .execute(
                "INSERT INTO conversation_presets(conversation_id, preset) VALUES (?1, ?2)
                 ON CONFLICT(conversation_id) DO UPDATE SET preset=excluded.preset",
                params![id, preset],
            )
            .map_err(db_error)?;
        if changed == 0 {
            return Err("Conversation no longer exists.".into());
        }
        // INSERT..ON CONFLICT on a missing parent would violate FK; verify parent exists.
        let exists: bool = self
            .connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1)",
                [id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if !exists {
            return Err("Conversation no longer exists.".into());
        }
        Ok(())
    }
    pub fn runs_for_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<crate::agent_run::RunRecord>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, conversation_id, status, model_provider, model_id, checkpoint, error, created_at, updated_at
                 FROM runs WHERE conversation_id=?1 ORDER BY created_at",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map([conversation_id], |row| {
                let status_str: String = row.get(2)?;
                let status = crate::agent_run::RunState::from_str(&status_str)
                    .unwrap_or(crate::agent_run::RunState::OutcomeUnknown);
                Ok(crate::agent_run::RunRecord {
                    id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    status,
                    model_provider: row.get(3)?,
                    model_id: row.get(4)?,
                    checkpoint: row.get(5)?,
                    error: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            })
            .map_err(db_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    pub fn insert_subagent_run(&self, run: &crate::subagents::SubagentRun) -> Result<()> {
        self.connection
            .execute(
                "INSERT INTO subagent_runs(id, conversation_id, parent_run_id, child_run_id, depth, status, label, prompt, error, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![run.id, run.conversation_id, run.parent_run_id, run.child_run_id, run.depth, run.status, run.label, run.prompt, run.error, run.created_at, run.updated_at],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn update_subagent_run(&self, id: &str, status: &str, error: Option<&str>) -> Result<()> {
        let changed = self
            .connection
            .execute(
                "UPDATE subagent_runs SET status=?1, error=?2, updated_at=?3 WHERE id=?4",
                params![status, error, now(), id],
            )
            .map_err(db_error)?;
        if changed == 0 {
            return Err("Subagent run no longer exists.".into());
        }
        Ok(())
    }
    pub fn subagent_run(&self, id: &str) -> Result<Option<crate::subagents::SubagentRun>> {
        self.connection
            .query_row(
                "SELECT id, conversation_id, parent_run_id, child_run_id, depth, status, label, prompt, error, created_at, updated_at
                 FROM subagent_runs WHERE id=?1",
                [id],
                |row| {
                    Ok(crate::subagents::SubagentRun {
                        id: row.get(0)?,
                        conversation_id: row.get(1)?,
                        parent_run_id: row.get(2)?,
                        child_run_id: row.get(3)?,
                        depth: row.get(4)?,
                        status: row.get(5)?,
                        label: row.get(6)?,
                        prompt: row.get(7)?,
                        error: row.get(8)?,
                        created_at: row.get(9)?,
                        updated_at: row.get(10)?,
                    })
                },
            )
            .optional()
            .map_err(db_error)
    }
    pub fn subagent_runs_for_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<crate::subagents::SubagentRun>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, conversation_id, parent_run_id, child_run_id, depth, status, label, prompt, error, created_at, updated_at
                 FROM subagent_runs WHERE conversation_id=?1 ORDER BY created_at",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map([conversation_id], |row| {
                Ok(crate::subagents::SubagentRun {
                    id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    parent_run_id: row.get(2)?,
                    child_run_id: row.get(3)?,
                    depth: row.get(4)?,
                    status: row.get(5)?,
                    label: row.get(6)?,
                    prompt: row.get(7)?,
                    error: row.get(8)?,
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                })
            })
            .map_err(db_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    pub fn todos(&self, conversation_id: &str) -> Result<Vec<crate::plans::Todo>> {
        let mut statement = self
            .connection
            .prepare("SELECT idx, text, status, updated_at FROM todos WHERE conversation_id=?1 ORDER BY idx")
            .map_err(db_error)?;
        let rows = statement
            .query_map([conversation_id], |row| {
                Ok(crate::plans::Todo {
                    text: row.get(1)?,
                    status: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })
            .map_err(db_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    pub fn save_todos(&self, conversation_id: &str, todos: &[crate::plans::Todo]) -> Result<()> {
        crate::plans::validate_todos(todos)?;
        self.connection
            .execute("DELETE FROM todos WHERE conversation_id=?1", [conversation_id])
            .map_err(db_error)?;
        for (index, todo) in todos.iter().enumerate() {
            self.connection
                .execute(
                    "INSERT INTO todos(conversation_id, idx, text, status, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![conversation_id, index as i64, todo.text, todo.status, todo.updated_at],
                )
                .map_err(db_error)?;
        }
        Ok(())
    }
    pub fn goal(&self, conversation_id: &str) -> Result<Option<String>> {
        self.connection
            .query_row(
                "SELECT objective FROM goals WHERE conversation_id=?1",
                [conversation_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)
    }
    pub fn save_goal(&self, conversation_id: &str, objective: &str) -> Result<()> {
        crate::plans::validate_objective(objective)?;
        self.connection
            .execute(
                "INSERT INTO goals(conversation_id, objective, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(conversation_id) DO UPDATE SET objective=excluded.objective, updated_at=excluded.updated_at",
                params![conversation_id, objective, now()],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn clear_goal(&self, conversation_id: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM goals WHERE conversation_id=?1", [conversation_id])
            .map_err(db_error)?;
        Ok(())
    }
    pub fn teach_fact(&self, scope: &str, fact: &str, origin: &str) -> Result<crate::memory::Fact> {
        let record = crate::memory::Fact {
            id: uuid::Uuid::new_v4().to_string(),
            scope: crate::memory::normalize_scope(scope)?,
            fact: crate::memory::validate_fact(fact)?.to_string(),
            origin: origin.to_string(),
            created_at: now(),
            updated_at: now(),
        };
        self.connection
            .execute(
                "INSERT INTO memory_facts(id, scope, fact, origin, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![record.id, record.scope, record.fact, record.origin, record.created_at, record.updated_at],
            )
            .map_err(db_error)?;
        Ok(record)
    }
    pub fn recall_facts(&self, scope: &str, limit: usize) -> Result<Vec<crate::memory::Fact>> {
        let scope = crate::memory::normalize_scope(scope)?;
        let limit = limit.min(50) as i64;
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, scope, fact, origin, created_at, updated_at FROM memory_facts
                 WHERE scope=?1 OR scope='global' ORDER BY updated_at DESC LIMIT ?2",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map(params![scope, limit], |row| {
                Ok(crate::memory::Fact {
                    id: row.get(0)?,
                    scope: row.get(1)?,
                    fact: row.get(2)?,
                    origin: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                })
            })
            .map_err(db_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    pub fn forget_fact(&self, id: &str) -> Result<bool> {
        let changed = self
            .connection
            .execute("DELETE FROM memory_facts WHERE id=?1", [id])
            .map_err(db_error)?;
        Ok(changed > 0)
    }
    pub fn save_schedule(&self, schedule: &crate::scheduling::Schedule) -> Result<()> {
        crate::scheduling::validate_schedule(schedule)?;
        self.connection
            .execute(
                "INSERT INTO schedules(id, name, cron, task, enabled, run_once, last_run_at, last_result, created_at, conversation_id, allow_write)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                 ON CONFLICT(id) DO UPDATE SET name=excluded.name, cron=excluded.cron, task=excluded.task,
                 enabled=excluded.enabled, run_once=excluded.run_once, conversation_id=excluded.conversation_id, allow_write=excluded.allow_write",
                params![schedule.id, schedule.name, schedule.cron, schedule.task, schedule.enabled, schedule.run_once, schedule.last_run_at, schedule.last_result, schedule.created_at, schedule.conversation_id, schedule.allow_write as i64],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn schedules(&self) -> Result<Vec<crate::scheduling::Schedule>> {
        let mut statement = self
            .connection
            .prepare("SELECT id, name, cron, task, enabled, run_once, last_run_at, last_result, created_at, conversation_id, allow_write FROM schedules ORDER BY created_at")
            .map_err(db_error)?;
        let rows = statement
            .query_map([], |row| {
                let allow_write_i64: i64 = row.get(10).unwrap_or(0);
                Ok(crate::scheduling::Schedule {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    cron: row.get(2)?,
                    task: row.get(3)?,
                    enabled: row.get(4)?,
                    run_once: row.get(5)?,
                    last_run_at: row.get(6)?,
                    last_result: row.get(7)?,
                    created_at: row.get(8)?,
                    conversation_id: row.get(9)?,
                    allow_write: allow_write_i64 != 0,
                })
            })
            .map_err(db_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    pub fn delete_schedule(&self, id: &str) -> Result<bool> {
        let changed = self
            .connection
            .execute("DELETE FROM schedules WHERE id=?1", [id])
            .map_err(db_error)?;
        Ok(changed > 0)
    }
    pub fn record_schedule_result(&self, id: &str, result: &str) -> Result<()> {
        self.connection
            .execute(
                "UPDATE schedules SET last_run_at=?1, last_result=?2 WHERE id=?3",
                params![now(), result, id],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn save_plugin(&self, plugin: &crate::plugins::Plugin) -> Result<()> {
        self.connection
            .execute(
                "INSERT INTO plugins(name, version, path, enabled, sha256, installed_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(name) DO UPDATE SET version=excluded.version, path=excluded.path,
                 enabled=excluded.enabled, sha256=excluded.sha256",
                params![plugin.name, plugin.version, plugin.path, plugin.enabled, plugin.sha256, plugin.installed_at],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn plugins(&self) -> Result<Vec<crate::plugins::Plugin>> {
        let mut statement = self
            .connection
            .prepare("SELECT name, version, path, enabled, sha256, installed_at FROM plugins ORDER BY name")
            .map_err(db_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok(crate::plugins::Plugin {
                    name: row.get(0)?,
                    version: row.get(1)?,
                    path: row.get(2)?,
                    enabled: row.get(3)?,
                    sha256: row.get(4)?,
                    installed_at: row.get(5)?,
                })
            })
            .map_err(db_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    pub fn delete_plugin(&self, name: &str) -> Result<bool> {
        let changed = self
            .connection
            .execute("DELETE FROM plugins WHERE name=?1", [name])
            .map_err(db_error)?;
        Ok(changed > 0)
    }
    pub fn compaction(&self, conversation_id: &str) -> Result<Option<crate::compaction::Checkpoint>> {
        self.connection
            .query_row(
                "SELECT conversation_id, cutoff, artifact_id, created_at FROM compaction WHERE conversation_id=?1",
                [conversation_id],
                |row| {
                    Ok(crate::compaction::Checkpoint {
                        conversation_id: row.get(0)?,
                        cutoff: row.get(1)?,
                        artifact_id: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                },
            )
            .optional()
            .map_err(db_error)
    }
    pub fn save_compaction(&self, checkpoint: &crate::compaction::Checkpoint) -> Result<()> {
        self.connection
            .execute(
                "INSERT INTO compaction(conversation_id, cutoff, artifact_id, created_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(conversation_id) DO UPDATE SET cutoff=excluded.cutoff, artifact_id=excluded.artifact_id, created_at=excluded.created_at",
                params![checkpoint.conversation_id, checkpoint.cutoff, checkpoint.artifact_id, checkpoint.created_at],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn clear_compaction(&self, conversation_id: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM compaction WHERE conversation_id=?1", [conversation_id])
            .map_err(db_error)?;
        Ok(())
    }
    /// Fork: new conversation with message rows copied up to and including
    /// `through_message_id`, preserving order with fresh ids. Lineage is
    /// recorded by the caller as a run event on the new conversation.
    pub fn fork_conversation(
        &self,
        source_id: &str,
        through_message_id: &str,
    ) -> Result<Conversation> {
        let source: Conversation = self
            .connection
            .query_row(
                "SELECT id, title, updated_at, provider_id, model_id, provider_selection_required FROM conversations WHERE id=?1",
                [source_id],
                |row| {
                    Ok(Conversation {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        updated_at: row.get(2)?,
                        provider_id: row.get(3)?,
                        model_id: row.get(4)?,
                        provider_selection_required: row.get(5)?,
                    })
                },
            )
            .optional()
            .map_err(db_error)?
            .ok_or_else(|| "Source conversation no longer exists.".to_string())?;
        let prefix = self.messages(source_id)?;
        if !prefix.iter().any(|message| message.id == through_message_id) {
            return Err("Fork point message is not in this conversation.".into());
        }
        let mut forked = self.create_conversation()?;
        forked.title = format!("Fork of {}", source.title);
        forked.provider_id = source.provider_id;
        forked.model_id = source.model_id;
        self.connection
            .execute(
                "UPDATE conversations SET title=?1, updated_at=?2, provider_id=?3, model_id=?4 WHERE id=?5",
                params![forked.title, now(), forked.provider_id, forked.model_id, forked.id],
            )
            .map_err(db_error)?;
        if let Ok(tools) = self.conversation_tools(source_id) {
            let _ = self.save_conversation_tools(&forked.id, &tools);
        }
        for message in prefix {
            let id = uuid::Uuid::new_v4().to_string();
            self.connection
                .execute(
                    "INSERT INTO messages(id, conversation_id, role, content, reasoning, status, created_at, error)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![id, forked.id, message.role, message.content, message.reasoning, message.status, message.created_at, message.error],
                )
                .map_err(db_error)?;
            if message.id == through_message_id {
                break;
            }
        }
        Ok(forked)
    }
    /// Bounded cross-conversation search over message content and tool audits.
    pub fn search_sessions(&self, query: &str, limit: usize) -> Result<Vec<crate::sessions::Hit>> {
        let query = query.trim();
        if query.is_empty() || query.len() > 200 {
            return Err("Search query must be 1-200 characters.".into());
        }
        let pattern = format!("%{query}%");
        let limit = limit.min(100) as i64;
        let mut statement = self
            .connection
            .prepare(
                "SELECT c.id, c.title, m.id, m.role, substr(m.content, 1, 280), m.created_at
                 FROM messages m JOIN conversations c ON c.id=m.conversation_id
                 WHERE m.content LIKE ?1 ESCAPE '\\' ORDER BY m.created_at DESC LIMIT ?2",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map(params![pattern, limit], |row| {
                Ok(crate::sessions::Hit {
                    conversation_id: row.get(0)?,
                    conversation_title: row.get(1)?,
                    message_id: row.get(2)?,
                    role: row.get(3)?,
                    excerpt: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })
            .map_err(db_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    pub fn artifact(&self, id: &str) -> Result<Option<crate::artifacts::ArtifactRecord>> {
        self.connection
            .query_row(
                "SELECT id, conversation_id, run_id, tool_name, mime_type, size_bytes, sha256, content, created_at
                 FROM artifacts WHERE id=?1",
                [id],
                |row| {
                    let size_bytes: i64 = row.get(5)?;
                    Ok(crate::artifacts::ArtifactRecord {
                        id: row.get(0)?,
                        conversation_id: row.get(1)?,
                        run_id: row.get(2)?,
                        tool_name: row.get(3)?,
                        mime_type: row.get(4)?,
                        size_bytes: size_bytes as usize,
                        sha256: row.get(6)?,
                        content: row.get(7)?,
                        created_at: row.get(8)?,
                    })
                },
            )
            .optional()
            .map_err(db_error)
    }
    pub fn artifacts_for_conversation(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<crate::artifacts::ArtifactRecord>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, conversation_id, run_id, tool_name, mime_type, size_bytes, sha256, content, created_at
                 FROM artifacts WHERE conversation_id=?1 ORDER BY created_at",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map([conversation_id], |row| {
                let size_bytes: i64 = row.get(5)?;
                Ok(crate::artifacts::ArtifactRecord {
                    id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    run_id: row.get(2)?,
                    tool_name: row.get(3)?,
                    mime_type: row.get(4)?,
                    size_bytes: size_bytes as usize,
                    sha256: row.get(6)?,
                    content: row.get(7)?,
                    created_at: row.get(8)?,
                })
            })
            .map_err(db_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(db_error)
    }
    pub fn conversation_tools(&self, id: &str) -> Result<ConversationTools> {
        let value: Option<Option<String>> = self.connection.query_row(
            "SELECT t.value FROM conversations c LEFT JOIN conversation_tools t ON t.conversation_id=c.id WHERE c.id=?1", [id], |row| row.get(0),
        ).optional().map_err(db_error)?;
        let tools: ConversationTools = match value {
            None => return Err("Conversation no longer exists.".into()),
            Some(None) => ConversationTools::default(),
            Some(Some(value)) => serde_json::from_str(&value)
                .map_err(|_| "Stored conversation tools are invalid.")?,
        };
        tools.validate()?;
        Ok(tools)
    }
    pub fn save_conversation_tools(&self, id: &str, tools: &ConversationTools) -> Result<()> {
        tools.validate()?;
        let value = serde_json::to_string(tools).map_err(|error| error.to_string())?;
        self.connection.execute("INSERT INTO conversation_tools(conversation_id,value) VALUES(?1,?2) ON CONFLICT(conversation_id) DO UPDATE SET value=excluded.value", params![id, value]).map_err(db_error)?;
        Ok(())
    }
    /// The last explicitly chosen selection, applied to new chats. `None`
    /// means no preference was saved yet, not an empty selection.
    pub fn remembered_tools(&self) -> Result<Option<RememberedTools>> {
        let value: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM settings WHERE key=?1",
                [REMEMBERED_TOOLS_KEY],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        value
            .map(|text| {
                serde_json::from_str(&text)
                    .map_err(|_| "Saved remembered tool selection is invalid.".to_string())
            })
            .transpose()
    }
    pub fn save_remembered_tools(&self, tools: &RememberedTools) -> Result<()> {
        tools.validate()?;
        self.save_setting(REMEMBERED_TOOLS_KEY, tools)
    }
    pub fn create_conversation(&self) -> Result<Conversation> {
        let selection: ModelSelection = self.setting(PREFERRED_MODEL_KEY)?;
        selection.validate()?;
        let needs_selection = match selection.provider_id.as_ref() {
            Some(id) => !self.provider_exists(id)?,
            None => false,
        };
        let conversation = Conversation {
            id: uuid::Uuid::new_v4().to_string(),
            title: "New conversation".into(),
            updated_at: now(),
            provider_id: selection.provider_id,
            model_id: (!selection.model_id.is_empty()).then_some(selection.model_id),
            provider_selection_required: needs_selection,
        };
        self.connection
            .execute(
                "INSERT INTO conversations(id,title,updated_at,provider_id,model_id,provider_selection_required) VALUES (?1,?2,?3,?4,?5,?6)",
                params![conversation.id, conversation.title, conversation.updated_at, conversation.provider_id, conversation.model_id, conversation.provider_selection_required],
            )
            .map_err(db_error)?;
        Ok(conversation)
    }
    pub fn list_conversations(&self) -> Result<Vec<Conversation>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id,title,updated_at,provider_id,model_id,provider_selection_required FROM conversations ORDER BY updated_at DESC,rowid DESC",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok(Conversation {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    updated_at: row.get(2)?,
                    provider_id: row.get(3)?,
                    model_id: row.get(4)?,
                    provider_selection_required: row.get::<_, i64>(5)? != 0,
                })
            })
            .map_err(db_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    pub fn rename_conversation(&self, id: &str, title: &str) -> Result<()> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 160 {
            return Err("Title must contain 1–160 characters.".into());
        }
        let changed = self
            .connection
            .execute(
                "UPDATE conversations SET title=?1,updated_at=?2 WHERE id=?3",
                params![title, now(), id],
            )
            .map_err(db_error)?;
        if changed == 0 {
            return Err("Conversation no longer exists.".into());
        }
        Ok(())
    }
    pub fn delete_conversation(&self, id: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM conversations WHERE id=?1", [id])
            .map_err(db_error)?;
        Ok(())
    }
    pub fn conversation_model(&self, id: &str) -> Result<(ModelSelection, bool)> {
        let row: Option<(Option<String>, Option<String>, i64)> = self
            .connection
            .query_row(
                "SELECT provider_id,model_id,provider_selection_required FROM conversations WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(db_error)?;
        let (provider_id, model_id, required) = row.ok_or("Conversation no longer exists.")?;
        let selection = ModelSelection { provider_id, model_id: model_id.unwrap_or_default() };
        selection.validate()?;
        Ok((selection, required != 0))
    }
    pub fn save_conversation_model(&self, id: &str, selection: &ModelSelection) -> Result<()> {
        selection.validate()?;
        if let Some(provider_id) = &selection.provider_id {
            if !self.provider_exists(provider_id)? {
                return Err("Choose an existing provider before saving this conversation.".into());
            }
        }
        let changed = self.connection.execute(
            "UPDATE conversations SET provider_id=?1,model_id=?2,provider_selection_required=0,updated_at=?3 WHERE id=?4",
            params![selection.provider_id, (!selection.model_id.is_empty()).then_some(&selection.model_id), now(), id],
        ).map_err(db_error)?;
        if changed == 0 { return Err("Conversation no longer exists.".into()); }
        Ok(())
    }
    pub fn preferred_model(&self) -> Result<ModelSelection> {
        let selection: ModelSelection = self.setting(PREFERRED_MODEL_KEY)?;
        selection.validate()?;
        Ok(selection)
    }
    pub fn save_preferred_model(&self, selection: &ModelSelection) -> Result<()> {
        selection.validate()?;
        self.save_setting(PREFERRED_MODEL_KEY, selection)
    }
    pub fn provider_exists(&self, id: &str) -> Result<bool> {
        self.connection.query_row("SELECT EXISTS(SELECT 1 FROM providers WHERE id=?1)", [id], |row| row.get(0)).map_err(db_error)
    }
    pub fn provider(&self, id: &str) -> Result<Option<ProviderConnection>> {
        self.connection.query_row(
            "SELECT id,name,api_format,base_url,verified,last_tested_at,models FROM providers WHERE id=?1",
            [id],
            |row| {
                let models: String = row.get(6)?;
                let models = serde_json::from_str::<Vec<RemoteModel>>(&models).map_err(|error| rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(error)))?;
                Ok(ProviderConnection { id: row.get(0)?, name: row.get(1)?, api_format: row.get(2)?, base_url: row.get(3)?, verified: row.get::<_, i64>(4)? != 0, last_tested_at: row.get(5)?, models, has_api_key: false })
            },
        ).optional().map_err(db_error)
    }
    pub fn providers(&self) -> Result<Vec<ProviderConnection>> {
        let mut statement = self.connection.prepare("SELECT id,name,api_format,base_url,verified,last_tested_at,models FROM providers ORDER BY name COLLATE NOCASE,id").map_err(db_error)?;
        let rows = statement.query_map([], |row| {
            let models: String = row.get(6)?;
            let models = serde_json::from_str::<Vec<RemoteModel>>(&models).map_err(|error| rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Text, Box::new(error)))?;
            Ok(ProviderConnection { id: row.get(0)?, name: row.get(1)?, api_format: row.get(2)?, base_url: row.get(3)?, verified: row.get::<_, i64>(4)? != 0, last_tested_at: row.get(5)?, models, has_api_key: false })
        }).map_err(db_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(db_error)
    }
    pub fn save_provider(&self, provider: &ProviderConnection) -> Result<()> {
        let models = serde_json::to_string(&provider.models).map_err(|error| error.to_string())?;
        self.connection.execute(
            "INSERT INTO providers(id,name,api_format,base_url,verified,last_tested_at,models) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET name=excluded.name,api_format=excluded.api_format,base_url=excluded.base_url,verified=excluded.verified,last_tested_at=excluded.last_tested_at,models=excluded.models",
            params![provider.id, provider.name, provider.api_format, provider.base_url, provider.verified, provider.last_tested_at, models],
        ).map_err(db_error)?;
        Ok(())
    }
    pub fn delete_provider(&self, id: &str) -> Result<bool> {
        let changed = self.connection.execute("DELETE FROM providers WHERE id=?1", [id]).map_err(db_error)?;
        if changed > 0 {
            self.connection.execute("UPDATE conversations SET provider_selection_required=1 WHERE provider_id=?1", [id]).map_err(db_error)?;
        }
        Ok(changed > 0)
    }
    pub fn update_provider_test(&self, id: &str, verified: bool, tested_at: Option<i64>, models: &[RemoteModel]) -> Result<()> {
        let models = serde_json::to_string(models).map_err(|error| error.to_string())?;
        let changed = self.connection.execute("UPDATE providers SET verified=?1,last_tested_at=?2,models=?3 WHERE id=?4", params![verified, tested_at, models, id]).map_err(db_error)?;
        if changed == 0 { return Err("Provider no longer exists.".into()); }
        Ok(())
    }
    pub fn update_provider_models(&self, id: &str, models: &[RemoteModel]) -> Result<()> {
        let models = serde_json::to_string(models).map_err(|error| error.to_string())?;
        let changed = self.connection.execute("UPDATE providers SET models=?1 WHERE id=?2", params![models, id]).map_err(db_error)?;
        if changed == 0 { return Err("Provider no longer exists.".into()); }
        Ok(())
    }
    pub fn messages(&self, id: &str) -> Result<Vec<Message>> {
        let mut statement = self.connection.prepare("SELECT id,conversation_id,role,content,reasoning,status,created_at,error FROM messages WHERE conversation_id=?1 ORDER BY created_at,rowid").map_err(db_error)?;
        let rows = statement
            .query_map([id], |row| {
                Ok(Message {
                    error: row.get(7)?,
                    id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    role: row.get(2)?,
                    content: row.get(3)?,
                    reasoning: row.get(4)?,
                    status: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })
            .map_err(db_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    pub fn append_message(
        &self,
        conversation_id: &str,
        role: &str,
        content: &str,
        status: &str,
    ) -> Result<Message> {
        if content.len() > 1_048_576 {
            return Err("Message exceeds the 1 MiB limit.".into());
        }
        let message = Message {
            error: None,
            id: uuid::Uuid::new_v4().to_string(),
            conversation_id: conversation_id.into(),
            role: role.into(),
            content: content.into(),
            reasoning: String::new(),
            status: status.into(),
            created_at: now(),
        };
        let transaction = self.connection.unchecked_transaction().map_err(db_error)?;
        transaction.execute("INSERT INTO messages(id,conversation_id,role,content,status,created_at) VALUES (?1,?2,?3,?4,?5,?6)", params![message.id,conversation_id,role,content,status,message.created_at]).map_err(db_error)?;
        transaction
            .execute(
                "UPDATE conversations SET updated_at=?1 WHERE id=?2",
                params![now(), conversation_id],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)?;
        Ok(message)
    }
    pub fn begin_turn(&self, conversation_id: &str, content: &str) -> Result<Message> {
        let content = content.trim();
        if content.is_empty() || content.len() > 1_048_576 {
            return Err("Enter a message no larger than 1 MiB.".into());
        }
        let assistant = Message {
            id: uuid::Uuid::new_v4().to_string(),
            conversation_id: conversation_id.into(),
            role: "assistant".into(),
            content: String::new(),
            reasoning: String::new(),
            status: "streaming".into(),
            created_at: now(),
            error: None,
        };
        let transaction = self.connection.unchecked_transaction().map_err(db_error)?;
        let existing: bool = transaction
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM messages WHERE conversation_id=?1)",
                [conversation_id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        transaction.execute("INSERT INTO messages(id,conversation_id,role,content,status,created_at) VALUES(?1,?2,'user',?3,'complete',?4)", params![uuid::Uuid::new_v4().to_string(), conversation_id, content, assistant.created_at]).map_err(db_error)?;
        transaction.execute("INSERT INTO messages(id,conversation_id,role,content,status,created_at) VALUES(?1,?2,'assistant','','streaming',?3)", params![assistant.id, conversation_id, assistant.created_at]).map_err(db_error)?;
        if existing {
            transaction
                .execute(
                    "UPDATE conversations SET updated_at=?1 WHERE id=?2",
                    params![assistant.created_at, conversation_id],
                )
                .map_err(db_error)?;
        } else {
            transaction
                .execute(
                    "UPDATE conversations SET title=?1,updated_at=?2 WHERE id=?3",
                    params![
                        content.chars().take(64).collect::<String>(),
                        assistant.created_at,
                        conversation_id
                    ],
                )
                .map_err(db_error)?;
        }
        transaction.commit().map_err(db_error)?;
        Ok(assistant)
    }
    pub fn update_message(
        &self,
        id: &str,
        content: &str,
        reasoning: &str,
        status: &str,
    ) -> Result<()> {
        self.connection
            .execute(
                "UPDATE messages SET content=?1,reasoning=?2,status=?3 WHERE id=?4",
                params![content, reasoning, status, id],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub(crate) fn setting<T: serde::de::DeserializeOwned + Default>(&self, key: &str) -> Result<T> {
        let value: Option<String> = self
            .connection
            .query_row("SELECT value FROM settings WHERE key=?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .map_err(db_error)?;
        value
            .map(|text| {
                serde_json::from_str(&text).map_err(|error| format!("Invalid saved {key}: {error}"))
            })
            .unwrap_or_else(|| Ok(T::default()))
    }
    pub(crate) fn save_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        let text = serde_json::to_string(value).map_err(|error| error.to_string())?;
        self.connection.execute("INSERT INTO settings VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key,text]).map_err(db_error)?;
        Ok(())
    }
    pub fn runtime_config(&self) -> Result<RuntimeConfig> {
        self.setting("runtime")
    }
    pub fn save_runtime_config(&self, config: &RuntimeConfig) -> Result<()> {
        config.validate()?;
        self.save_setting("runtime", config)
    }
    pub fn preferences(&self) -> Result<Preferences> {
        self.setting("preferences")
    }
    pub fn active_skills(&self) -> Result<Vec<String>> {
        self.setting("active_skills")
    }
    pub fn workspace_path(&self) -> Result<String> {
        self.setting("workspace_path")
    }
    pub fn execution_config(&self) -> Result<crate::execution::ExecutionConfig> {
        self.setting("execution")
    }
    pub fn save_execution_config(&self, config: &crate::execution::ExecutionConfig) -> Result<()> {
        self.save_setting("execution", config)
    }
    pub fn save_workspace_path(&self, path: &str) -> Result<()> {
        self.save_setting("workspace_path", &path)
    }
    pub fn save_active_skills(&self, ids: &[String]) -> Result<()> {
        self.save_setting("active_skills", &ids)
    }
    pub fn save_preferences(&self, preferences: &Preferences) -> Result<()> {
        preferences.validate()?;
        self.save_setting("preferences", preferences)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepting_a_turn_is_atomic_even_when_the_assistant_insert_fails() {
        let store = Store::open_memory().unwrap();
        let id = store.create_conversation().unwrap().id;
        store.rename_conversation(&id, "Original title").unwrap();
        let before = store.list_conversations().unwrap().remove(0);
        store.connection.execute_batch("CREATE TRIGGER fail_assistant BEFORE INSERT ON messages WHEN NEW.role='assistant' BEGIN SELECT RAISE(ABORT,'injected write failure'); END;").unwrap();
        assert!(store.begin_turn(&id, "New prompt").is_err());
        assert!(store.messages(&id).unwrap().is_empty());
        let after = store.list_conversations().unwrap().remove(0);
        assert_eq!(after.title, before.title);
        assert_eq!(after.updated_at, before.updated_at);
        store
            .connection
            .execute_batch("DROP TRIGGER fail_assistant;")
            .unwrap();
        let assistant = store.begin_turn(&id, " New prompt ").unwrap();
        let messages = store.messages(&id).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "New prompt");
        assert_eq!(messages[1].id, assistant.id);
        assert_eq!(messages[1].status, "streaming");
        assert_eq!(store.list_conversations().unwrap()[0].title, "New prompt");
        assert!(store.begin_turn("missing", "No orphan records").is_err());
        assert_eq!(store.messages(&id).unwrap().len(), 2);
    }
    #[test]
    fn generation_errors_survive_reopening_with_partial_output() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("errors.sqlite");
        let id;
        {
            let store = Store::open(&path).unwrap();
            id = store.create_conversation().unwrap().id;
            let message = store
                .append_message(&id, "assistant", "", "streaming")
                .unwrap();
            store
                .finish_message(
                    &message.id,
                    "Partial answer",
                    "Partial reasoning",
                    "error",
                    Some("Response token limit reached"),
                )
                .unwrap();
        }
        let store = Store::open(&path).unwrap();
        let messages = store.messages(&id).unwrap();
        assert_eq!(
            messages[0].error.as_deref(),
            Some("Response token limit reached")
        );
        assert_eq!(messages[0].status, "error");
        assert_eq!(messages[0].content, "Partial answer");
        assert_eq!(messages[0].reasoning, "Partial reasoning");
    }
    #[test]
    fn conversation_tools_are_isolated_durable_and_deleted_with_chat() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tools.sqlite");
        let first;
        let second;
        let settings = ConversationTools {
            access_mode: crate::permissions::AccessMode::FullAccess,
            sources: vec!["__workspace".into()],
            tools: vec![crate::connectors::ToolSelection {
                connector_id: "deepwiki".into(),
                tool_name: "read_wiki_structure".into(),
            }],
        };
        {
            let store = Store::open(&path).unwrap();
            first = store.create_conversation().unwrap().id;
            second = store.create_conversation().unwrap().id;
            store.save_conversation_tools(&first, &settings).unwrap();
            assert_eq!(
                store.conversation_tools(&second).unwrap(),
                ConversationTools::default()
            );
            let invalid = ConversationTools {
                sources: vec!["unknown".into()],
                ..settings.clone()
            };
            assert!(store.save_conversation_tools(&first, &invalid).is_err());
            assert_eq!(store.conversation_tools(&first).unwrap(), settings);
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.conversation_tools(&first).unwrap(), settings);
        assert_eq!(
            store.conversation_tools(&second).unwrap(),
            ConversationTools::default()
        );
        store.delete_conversation(&first).unwrap();
        assert!(store.conversation_tools(&first).is_err());
        assert!(store.save_conversation_tools(&first, &settings).is_err());
        assert_eq!(
            store
                .connection
                .query_row("SELECT COUNT(*) FROM conversation_tools", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn conversation_tools_reject_duplicates_and_excessive_combined_tools() {
        let mut settings = ConversationTools {
            access_mode: crate::permissions::AccessMode::Ask,
            sources: vec!["__workspace".into(), "__execution".into()],
            tools: (0..26)
                .map(|index| crate::connectors::ToolSelection {
                    connector_id: "github".into(),
                    tool_name: format!("tool_{index}"),
                })
                .collect(),
        };
        assert!(settings.validate().is_ok());
        settings.tools.push(crate::connectors::ToolSelection {
            connector_id: "github".into(),
            tool_name: "extra".into(),
        });
        assert!(settings.validate().is_err());
        settings.tools.clear();
        settings.sources.push("__execution".into());
        assert!(settings.validate().is_err());
        settings.sources.clear();
        settings.tools = vec![
            crate::connectors::ToolSelection {
                connector_id: "github".into(),
                tool_name: "duplicate".into()
            };
            2
        ];
        assert!(settings.validate().is_err());
    }

    #[test]
    fn remembered_tools_persist_and_explicit_empty_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("remembered.sqlite");
        let selection = RememberedTools {
            sources: vec!["__workspace".into()],
            tools: vec![crate::connectors::ToolSelection {
                connector_id: "deepwiki".into(),
                tool_name: "read_wiki_structure".into(),
            }],
        };
        {
            let store = Store::open(&path).unwrap();
            assert_eq!(store.remembered_tools().unwrap(), None);
            store.save_remembered_tools(&selection).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.remembered_tools().unwrap(), Some(selection.clone()));
        // An explicitly empty selection is a saved preference, not a missing one.
        store.save_remembered_tools(&RememberedTools::default()).unwrap();
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.remembered_tools().unwrap(),
            Some(RememberedTools::default())
        );
        // Invalid selections are rejected without touching the saved value.
        let unknown_source = RememberedTools {
            sources: vec!["unknown".into()],
            ..RememberedTools::default()
        };
        assert!(store.save_remembered_tools(&unknown_source).is_err());
        let excessive = RememberedTools {
            tools: (0..33)
                .map(|index| crate::connectors::ToolSelection {
                    connector_id: "github".into(),
                    tool_name: format!("tool_{index}"),
                })
                .collect(),
            ..RememberedTools::default()
        };
        assert!(store.save_remembered_tools(&excessive).is_err());
        let duplicated = RememberedTools {
            tools: vec![
                crate::connectors::ToolSelection {
                    connector_id: "github".into(),
                    tool_name: "same".into()
                };
                2
            ],
            ..RememberedTools::default()
        };
        assert!(store.save_remembered_tools(&duplicated).is_err());
        assert_eq!(
            store.remembered_tools().unwrap(),
            Some(RememberedTools::default())
        );
    }

    #[test]
    fn first_startup_seeds_remembered_tools_from_the_latest_conversation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("seed.sqlite");
        let first;
        let second;
        {
            let store = Store::open(&path).unwrap();
            first = store.create_conversation().unwrap().id;
            second = store.create_conversation().unwrap().id;
            store
                .save_conversation_tools(
                    &first,
                    &ConversationTools {
                        access_mode: crate::permissions::AccessMode::FullAccess,
                        sources: vec!["__workspace".into()],
                        tools: vec![crate::connectors::ToolSelection {
                            connector_id: "deepwiki".into(),
                            tool_name: "read_wiki_structure".into(),
                        }],
                    },
                )
                .unwrap();
            // Make the second conversation the most recently updated one with an
            // explicitly empty selection; it must win and drop the permission mode.
            store.rename_conversation(&second, "Later conversation").unwrap();
            store
                .save_conversation_tools(&second, &ConversationTools::default())
                .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.remembered_tools().unwrap(),
            Some(RememberedTools::default())
        );
        assert_eq!(store.list_conversations().unwrap().len(), 2);
        // The preference is a snapshot: later restarts must not re-seed it from
        // conversations after the user changed it.
        let replaced = RememberedTools {
            sources: vec!["__daytona".into()],
            tools: vec![],
        };
        store.save_remembered_tools(&replaced).unwrap();
        let store = Store::open(&path).unwrap();
        assert_eq!(store.remembered_tools().unwrap(), Some(replaced));
    }

    #[test]
    fn an_unusable_conversation_selection_does_not_block_startup_or_seeding() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt.sqlite");
        let id;
        {
            let store = Store::open(&path).unwrap();
            id = store.create_conversation().unwrap().id;
            store
                .connection
                .execute(
                    "INSERT INTO conversation_tools(conversation_id,value) VALUES(?1,'not json')",
                    [&id],
                )
                .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.remembered_tools().unwrap(), None);
        assert!(store.conversation_tools(&id).is_err());
    }

    #[test]
    fn provider_connections_and_model_choices_are_durable_and_deletion_marks_chats() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("providers.sqlite");
        let provider = ProviderConnection {
            id: "provider-1".into(),
            name: "Fixture provider".into(),
            api_format: crate::providers::OPENAI_CHAT_COMPLETIONS.into(),
            base_url: "https://api.example.test/v1".into(),
            verified: true,
            last_tested_at: Some(123),
            models: vec![RemoteModel {
                id: "fixture-model".into(),
                context_length: Some(8192),
                max_output_tokens: Some(1024),
                tool_support: crate::providers::ToolSupport::Supported,
            }],
            has_api_key: false,
        };
        let selection = ModelSelection { provider_id: Some(provider.id.clone()), model_id: "fixture-model".into() };
        {
            let store = Store::open(&path).unwrap();
            store.save_provider(&provider).unwrap();
            store.save_preferred_model(&selection).unwrap();
            let conversation = store.create_conversation().unwrap();
            assert_eq!(conversation.provider_id, Some(provider.id.clone()));
            assert_eq!(conversation.model_id, Some("fixture-model".into()));
            assert!(!conversation.provider_selection_required);
            assert_eq!(store.conversation_model(&conversation.id).unwrap(), (selection.clone(), false));
            store.delete_provider(&provider.id).unwrap();
            assert_eq!(store.conversation_model(&conversation.id).unwrap(), (selection, true));
            assert!(store.providers().unwrap().is_empty());
        }
        let store = Store::open(&path).unwrap();
        let conversations = store.list_conversations().unwrap();
        assert!(conversations[0].provider_selection_required);
        assert_eq!(conversations[0].provider_id.as_deref(), Some("provider-1"));
        assert!(store.provider("provider-1").unwrap().is_none());
    }

    #[test]
    fn conversations_and_messages_survive_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.sqlite");
        let id;
        {
            let store = Store::open(&path).unwrap();
            let conversation = store.create_conversation().unwrap();
            id = conversation.id;
            store
                .append_message(&id, "user", "Hello", "complete")
                .unwrap();
            store
                .rename_conversation(&id, " A local conversation ")
                .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.list_conversations().unwrap()[0].title,
            "A local conversation"
        );
        assert_eq!(store.messages(&id).unwrap()[0].content, "Hello");
    }

    #[test]
    fn deletion_removes_messages_and_unknown_conversations_fail() {
        let store = Store::open_memory().unwrap();
        let id = store.create_conversation().unwrap().id;
        store
            .append_message(&id, "user", "message", "complete")
            .unwrap();
        store.delete_conversation(&id).unwrap();
        assert!(store.list_conversations().unwrap().is_empty());
        assert!(store
            .append_message(&id, "user", "orphan", "complete")
            .is_err());
        assert!(store.rename_conversation(&id, "missing").is_err());
    }

    #[test]
    fn startup_recovers_unfinished_responses_without_losing_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.sqlite");
        let id;
        {
            let store = Store::open(&path).unwrap();
            id = store.create_conversation().unwrap().id;
            store
                .append_message(&id, "assistant", "Partial response", "streaming")
                .unwrap();
        }
        let store = Store::open(&path).unwrap();
        let messages = store.messages(&id).unwrap();
        assert_eq!(messages[0].status, "interrupted");
        assert_eq!(messages[0].content, "Partial response");
    }

    #[test]
    fn settings_round_trip_and_reject_invalid_runtime_config() {
        let store = Store::open_memory().unwrap();
        let config = crate::runtime_config::RuntimeConfig {
            cpu_threads: 4,
            ..Default::default()
        };
        store.save_runtime_config(&config).unwrap();
        assert_eq!(store.runtime_config().unwrap(), config);
        let invalid = crate::runtime_config::RuntimeConfig {
            cpu_threads: 0,
            ..config
        };
        assert!(store.save_runtime_config(&invalid).is_err());
        assert_eq!(store.runtime_config().unwrap().cpu_threads, 4);
    }
}
