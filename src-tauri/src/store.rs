use crate::runtime_config::RuntimeConfig;
use crate::providers::{ModelSelection, ProviderConnection, RemoteModel};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

type Result<T> = std::result::Result<T, String>;
/// One `session_events` row inside a repair window:
/// (run_id, step_id, tool_call_id, event_type, payload JSON).
type TailEvent = (Option<String>, Option<String>, Option<String>, String, String);
/// A persisted `model_qualifications` row. `stale` is computed at read by
/// comparing identity_hash to the live identity — never stored.
pub struct QualificationRow {
    pub id: String,
    pub identity_hash: String,
    pub model_id: String,
    pub model_path: Option<String>,
    pub model_fingerprint: Option<String>,
    pub runtime_version: Option<String>,
    pub provider_id: Option<String>,
    pub config_json: String,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub results_json: String,
    pub status: String,
}
const LOCAL_TOOL_CALLING_SUPPORT_KEY: &str = "local_tool_calling_support";
const TOOL_COMPATIBILITY_KEY: &str = "tool_compatibility_v2";

fn db_error(error: rusqlite::Error) -> String {
    format!("Local database: {error}")
}
pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedResearch {
    pub id: String,
    pub conversation_id: Option<String>,
    pub question: String,
    pub answer: Option<String>,
    pub sources_json: String,
    pub trace_json: Option<String>,
    pub created_at: i64,
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
    pub metrics: Option<serde_json::Value>,
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
    #[serde(default)]
    pub projector_path: String,
    pub temperature: f64,
    pub top_p: f64,
    pub max_tokens: u32,
    pub system_prompt: String,
}
impl Default for Preferences {
    fn default() -> Self {
        Self { runtime_path: String::new(), model_path: String::new(), projector_path: String::new(), temperature: 1.0,
            top_p: 0.95, max_tokens: 2048, system_prompt: "You are a helpful local assistant. Be clear and accurate. If you do not know something, say so.".into() }
    }
}
impl Preferences {
    /// AREX answers arrive inside a `finish` call — the JSON envelope plus the
    /// evidences list need headroom beyond a chat-sized budget, and report-
    /// style answers visibly splice when they hit a small cap mid-sentence.
    /// Upgrade older saved defaults without lowering an explicit larger budget.
    pub fn apply_model_defaults(mut self) -> Self {
        if crate::arex::is_arex_model(&self.model_path) && self.max_tokens < 8192 {
            self.max_tokens = 8192;
        }
        self
    }

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
        if self.runtime_path.len() > 32768 || self.model_path.len() > 32768 || self.projector_path.len() > 32768 {
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
    /// Ask/Plan/Act. Rows saved before the mode existed default to Act, so
    /// old conversations keep working with their existing permission mode.
    #[serde(default)]
    pub interaction_mode: crate::permissions::InteractionMode,
    pub sources: Vec<String>,
    pub tools: Vec<crate::connectors::ToolSelection>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptFreeze {
    pub conversation_id: String,
    pub selection_hash: String,
    pub tools_json: String,
    pub aliases_json: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RememberedTools {
    #[serde(default)]
    pub access_mode: crate::permissions::AccessMode,
    /// Last explicitly chosen interaction mode — the default for new chats.
    #[serde(default)]
    pub interaction_mode: crate::permissions::InteractionMode,
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
    pub(crate) fn open_memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory().map_err(db_error)?)
    }
    /// Test-only insert with an explicit timestamp; production writes always
    /// stamp `now()`.
    #[cfg(test)]
    pub(crate) fn insert_message_at(
        &self,
        conversation_id: &str,
        role: &str,
        content: &str,
        status: &str,
        created_at: i64,
    ) -> Result<Message> {
        let message = Message {
            error: None,
            metrics: None,
            id: uuid::Uuid::new_v4().to_string(),
            conversation_id: conversation_id.into(),
            role: role.into(),
            content: content.into(),
            reasoning: String::new(),
            status: status.into(),
            created_at,
        };
        self.connection
            .execute(
                "INSERT INTO messages(id,conversation_id,role,content,status,created_at) VALUES (?1,?2,?3,?4,?5,?6)",
                params![message.id, conversation_id, role, content, status, created_at],
            )
            .map_err(db_error)?;
        Ok(message)
    }
    fn initialize(connection: Connection) -> Result<Self> {
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(db_error)?;
        // Refuse data written by a newer session format before any migration
        // or repair touches it: opening anyway could silently misread the log.
        let has_settings: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='settings')",
                [],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if has_settings {
            let stamped: Option<String> = connection
                .query_row(
                    "SELECT value FROM settings WHERE key='session_format_version'",
                    [],
                    |row| row.get(0),
                )
                .optional()
                .map_err(db_error)?;
            if let Some(text) = stamped {
                let version: u32 = serde_json::from_str(&text)
                    .map_err(|_| "Saved session format version is invalid.".to_string())?;
                if version > crate::sessions::SESSION_FORMAT_VERSION {
                    return Err(format!(
                        "This data was written by a newer version of LocalLM (session format {version}); this build understands up to {}. Update LocalLM before opening this data.",
                        crate::sessions::SESSION_FORMAT_VERSION
                    ));
                }
            }
        }
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS conversations(id TEXT PRIMARY KEY, title TEXT NOT NULL, updated_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS messages(id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                role TEXT NOT NULL CHECK(role IN ('user','assistant','tool')), content TEXT NOT NULL, reasoning TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL CHECK(status IN ('complete','streaming','interrupted','error')), created_at INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS messages_conversation ON messages(conversation_id,created_at);
            CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS conversation_tools(conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE, value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS conversation_skills(conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE, value TEXT NOT NULL);
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
            CREATE TABLE IF NOT EXISTS tasks(
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                content TEXT NOT NULL,
                interaction_mode TEXT NOT NULL DEFAULT 'act',
                status TEXT NOT NULL CHECK(status IN ('queued','running','waiting_approval','waiting_resource','paused','interrupted','failed','completed','awaiting_review','cancelled')),
                position INTEGER,
                run_id TEXT,
                wait_reason TEXT,
                error TEXT,
                next_action TEXT,
                parent_task_id TEXT,
                created_at INTEGER NOT NULL,
                started_at INTEGER,
                ended_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS tasks_conversation ON tasks(conversation_id, created_at);
            CREATE INDEX IF NOT EXISTS tasks_queue ON tasks(status, position);
            CREATE TABLE IF NOT EXISTS task_transitions(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                from_status TEXT NOT NULL,
                to_status TEXT NOT NULL,
                reason TEXT,
                at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS task_transitions_task ON task_transitions(task_id, id);
            CREATE TABLE IF NOT EXISTS steering_messages(
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                text TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                applied_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS steering_messages_task ON steering_messages(task_id, applied_at);
            CREATE TABLE IF NOT EXISTS compaction(conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
                cutoff INTEGER NOT NULL, artifact_id TEXT NOT NULL, created_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS conversation_prompt_freeze (
                conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
                selection_hash TEXT NOT NULL,
                tools_json TEXT NOT NULL,
                aliases_json TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS session_events (
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                run_id TEXT REFERENCES runs(id) ON DELETE SET NULL,
                seq INTEGER NOT NULL,
                step_id TEXT,
                tool_call_id TEXT,
                event_type TEXT NOT NULL,
                payload TEXT NOT NULL,
                ignorable INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS research_sessions (
                id TEXT PRIMARY KEY,
                conversation_id TEXT REFERENCES conversations(id) ON DELETE CASCADE,
                question TEXT NOT NULL,
                answer TEXT,
                sources_json TEXT NOT NULL DEFAULT '[]',
                trace_json TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS research_tasks(
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                question TEXT NOT NULL,
                status TEXT NOT NULL,
                budgets_json TEXT NOT NULL,
                operations_json TEXT NOT NULL DEFAULT '[]',
                results_json TEXT NOT NULL DEFAULT '[]',
                requirements_json TEXT NOT NULL DEFAULT '[]',
                findings_json TEXT NOT NULL DEFAULT '[]',
                pending_json TEXT NOT NULL DEFAULT '[]',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS research_tasks_conversation ON research_tasks(conversation_id, created_at);
            CREATE TABLE IF NOT EXISTS task_workspaces(
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                source_root TEXT NOT NULL,
                kind TEXT NOT NULL CHECK(kind IN ('git-worktree','snapshot')),
                isolated_root TEXT NOT NULL,
                branch TEXT,
                baseline_commit TEXT NOT NULL,
                status TEXT NOT NULL CHECK(status IN ('active','applied','discarded')),
                exclusions_json TEXT NOT NULL DEFAULT '[]',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE UNIQUE INDEX IF NOT EXISTS task_workspaces_one_active
                ON task_workspaces(conversation_id) WHERE status='active';
            CREATE TABLE IF NOT EXISTS task_applies(
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL REFERENCES task_workspaces(id) ON DELETE CASCADE,
                created_at INTEGER NOT NULL,
                backup_dir TEXT NOT NULL,
                files_json TEXT NOT NULL,
                undone_at INTEGER
            );
            CREATE INDEX IF NOT EXISTS task_applies_task ON task_applies(task_id, created_at);
            CREATE TABLE IF NOT EXISTS task_criteria(
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                text TEXT NOT NULL,
                origin TEXT NOT NULL CHECK(origin IN ('user','agent')),
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS task_criteria_conversation ON task_criteria(conversation_id, created_at);
            CREATE TABLE IF NOT EXISTS verification_records(
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                criterion_id TEXT REFERENCES task_criteria(id) ON DELETE SET NULL,
                kind TEXT NOT NULL CHECK(kind IN ('command','browser','research','calculation','data','manual')),
                method TEXT NOT NULL,
                command TEXT,
                environment TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                ended_at INTEGER NOT NULL,
                exit_code INTEGER,
                output_excerpt TEXT NOT NULL DEFAULT '',
                output_artifact TEXT,
                output_truncated INTEGER NOT NULL DEFAULT 0,
                artifacts_json TEXT NOT NULL DEFAULT '[]',
                result TEXT NOT NULL CHECK(result IN ('passed','failed','unavailable','inconclusive')),
                limitations TEXT NOT NULL DEFAULT '',
                fingerprint TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS verification_records_conversation ON verification_records(conversation_id, created_at);
            CREATE TABLE IF NOT EXISTS model_qualifications(
                id TEXT PRIMARY KEY,
                identity_hash TEXT NOT NULL,
                model_id TEXT NOT NULL,
                model_path TEXT,
                model_fingerprint TEXT,
                runtime_version TEXT,
                provider_id TEXT,
                config_json TEXT NOT NULL DEFAULT '{}',
                started_at INTEGER NOT NULL,
                ended_at INTEGER,
                results_json TEXT NOT NULL DEFAULT '[]',
                status TEXT NOT NULL CHECK(status IN ('passed','partial','failed','cancelled'))
            );
            CREATE TABLE IF NOT EXISTS recipes(
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                prompt TEXT NOT NULL,
                interval_minutes INTEGER,
                enabled INTEGER NOT NULL DEFAULT 1,
                next_run_at INTEGER,
                last_run_at INTEGER,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS doc_collections(
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                root TEXT NOT NULL UNIQUE,
                file_count INTEGER NOT NULL DEFAULT 0,
                chunk_count INTEGER NOT NULL DEFAULT 0,
                indexed_at INTEGER,
                error TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS doc_chunks USING fts5(
                content,
                path UNINDEXED,
                collection_id UNINDEXED,
                line_start UNINDEXED,
                line_end UNINDEXED
            );
            CREATE TABLE IF NOT EXISTS isolation_grants(
                path TEXT NOT NULL,
                profile TEXT NOT NULL,
                granted_at INTEGER NOT NULL,
                PRIMARY KEY(path, profile)
            );
            CREATE INDEX IF NOT EXISTS model_qualifications_identity ON model_qualifications(identity_hash, started_at);
            CREATE INDEX IF NOT EXISTS session_events_conv_seq ON session_events(conversation_id, seq);
            CREATE INDEX IF NOT EXISTS session_events_run ON session_events(run_id);
            CREATE INDEX IF NOT EXISTS session_events_type ON session_events(event_type);").map_err(db_error)?;
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
        let has_metrics: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM pragma_table_info('messages') WHERE name='metrics')",
                [],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if !has_metrics {
            connection.execute("ALTER TABLE messages ADD COLUMN metrics TEXT", []).map_err(db_error)?;
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
        for (column, definition) in [
            ("reason", "TEXT"),
            ("source_conversation_id", "TEXT"),
            ("source_path", "TEXT"),
            ("source_hash", "TEXT"),
            ("source_branch", "TEXT"),
            ("pinned", "INTEGER NOT NULL DEFAULT 0"),
            ("excluded", "INTEGER NOT NULL DEFAULT 0"),
        ] {
            let exists: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM pragma_table_info('memory_facts') WHERE name=?1)",
                    [column],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            if !exists {
                connection
                    .execute(&format!("ALTER TABLE memory_facts ADD COLUMN {column} {definition}"), [])
                    .map_err(db_error)?;
            }
        }
        let store = Self { connection };
        store.migrate_fact_scopes()?;
        store.seed_remembered_tools()?;
        store.seed_subscription_providers()?;
        Self::migrate_historical_to_session_events(&store.connection)?;
        store.save_setting("session_format_version", &crate::sessions::SESSION_FORMAT_VERSION)?;
        Ok(store)
    }
    /// One-time migration: without a saved preference, seed the remembered
    /// selection from the most recently updated conversation that stored one,
    /// including an explicitly empty selection. Conversation records are never
    /// modified; an unusable stored row leaves the preference unset.
    fn seed_remembered_tools(&self) -> Result<()> {
        let existing: Option<String> = self
            .connection
            .query_row(
                "SELECT value FROM settings WHERE key=?1",
                [REMEMBERED_TOOLS_KEY],
                |row| row.get(0),
            )
            .optional().map_err(db_error)?;
        if let Some(text) = &existing {
            let value: serde_json::Value = serde_json::from_str(text)
                .map_err(|_| "Saved remembered tool selection is invalid.")?;
            if value.get("accessMode").is_some() { return Ok(()); }
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
        if let Some(existing) = existing {
            let mut remembered: RememberedTools = serde_json::from_str(&existing)
                .map_err(|_| "Saved remembered tool selection is invalid.")?;
            // Upgrade pre-permission defaults without replacing the saved tools.
            let upgraded = saved.as_deref()
                .and_then(|text| serde_json::from_str::<ConversationTools>(text).ok());
            remembered.access_mode = upgraded.as_ref().map(|tools| tools.access_mode).unwrap_or_default();
            remembered.interaction_mode = upgraded.map(|tools| tools.interaction_mode).unwrap_or_default();
            self.save_remembered_tools(&remembered)?;
        } else if let Some(text) = saved {
            if let Ok(settings) = serde_json::from_str::<ConversationTools>(&text) {
                let remembered = RememberedTools {
                    access_mode: settings.access_mode,
                    interaction_mode: settings.interaction_mode,
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
    fn seed_subscription_providers(&self) -> Result<()> {
        for provider in default_subscription_providers() {
            let models = serde_json::to_string(&provider.models).map_err(|e| e.to_string())?;
            self.connection.execute(
                "INSERT OR IGNORE INTO providers(id,name,api_format,base_url,verified,last_tested_at,models) VALUES (?1,?2,?3,?4,?5,?6,?7)",
                rusqlite::params![provider.id, provider.name, provider.api_format, provider.base_url, 0, None::<i64>, models],
            ).map_err(db_error)?;
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
    pub fn save_message_metrics(&self, id: &str, metrics: &serde_json::Value) -> Result<()> {
        let encoded = serde_json::to_string(metrics).map_err(|error| error.to_string())?;
        self.connection.execute("UPDATE messages SET metrics=?1 WHERE id=?2", params![encoded, id]).map_err(db_error)?;
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
        // A run waiting on approval puts its task into `waiting_approval`;
        // active run states map back to `running` (a task's own terminal and
        // `waiting_resource`/`paused` states are owned by the task driver).
        if status == crate::agent_run::RunState::AwaitingApproval {
            self.connection
                .execute("UPDATE tasks SET status='waiting_approval' WHERE run_id=?1 AND status='running'", [id])
                .map_err(db_error)?;
        } else if !status.is_terminal() {
            self.connection
                .execute(
                    "UPDATE tasks SET status='running', wait_reason=NULL WHERE run_id=?1 AND status IN ('waiting_approval','running')",
                    [id],
                )
                .map_err(db_error)?;
        }
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
        let seq = if event.seq > 0 {
            event.seq as i64
        } else {
            // Harness-nested child events arrive without a sequence; allocate
            // the next one so the (run_id, seq) primary key never collides.
            self.connection
                .query_row(
                    "SELECT COALESCE(MAX(seq), -1) + 1 FROM run_events WHERE run_id=?1",
                    [&event.run_id],
                    |row| row.get(0),
                )
                .map_err(db_error)?
        };
        let payload = serde_json::to_string(&event.payload).map_err(|e| e.to_string())?;
        self.connection
            .execute(
                "INSERT INTO run_events(run_id, seq, step_id, tool_call_id, event_type, payload, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    event.run_id,
                    seq,
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
        // ---- Harness extensions (presets, subagents, plans, memory) ----
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
        let transaction = self.connection.unchecked_transaction().map_err(db_error)?;
        transaction
            .execute("DELETE FROM todos WHERE conversation_id=?1", [conversation_id])
            .map_err(db_error)?;
        for (index, todo) in todos.iter().enumerate() {
            transaction
                .execute(
                    "INSERT INTO todos(conversation_id, idx, text, status, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![conversation_id, index as i64, todo.text, todo.status, todo.updated_at],
                )
                .map_err(db_error)?;
        }
        transaction.commit().map_err(db_error)?;
        Ok(())
    }
    /// Append one todo item without re-emitting the full list (~30 tokens).
    pub fn add_todo(&self, conversation_id: &str, text: &str) -> Result<crate::plans::Todo> {
        let mut todos = self.todos(conversation_id)?;
        let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
        if let Some(existing) = todos.iter().find(|todo| todo.text.split_whitespace().collect::<Vec<_>>().join(" ") == normalized) {
            return Ok(existing.clone());
        }
        if todos.len() >= 50 {
            return Err("At most 50 todo items are kept per conversation.".into());
        }
        if normalized.is_empty() || normalized.len() > 500 {
            return Err("Each todo needs 1-500 characters of text.".into());
        }
        let todo = crate::plans::Todo {
            text: normalized,
            status: crate::plans::PENDING.to_string(),
            updated_at: now(),
        };
        todos.push(todo.clone());
        self.save_todos(conversation_id, &todos)?;
        Ok(todo)
    }
    /// Update one todo by 0-based index without re-emitting the full list.
    pub fn update_todo(
        &self,
        conversation_id: &str,
        index: usize,
        status: Option<&str>,
        text: Option<&str>,
    ) -> Result<Vec<crate::plans::Todo>> {
        let mut todos = self.todos(conversation_id)?;
        let count = todos.len();
        let todo = todos.get_mut(index).ok_or_else(|| format!("Unknown todo index {index}. Current list has {count} item(s), indexed from 0. Use the latest todo list returned by the tool."))?;
        if let Some(status) = status {
            if ![crate::plans::PENDING, crate::plans::IN_PROGRESS, crate::plans::COMPLETED].contains(&status) {
                return Err(format!("Unknown todo status '{status}'. Use pending, in_progress, or completed."));
            }
            todo.status = status.to_string();
        }
        if let Some(text) = text {
            if text.trim().is_empty() || text.len() > 500 {
                return Err("Each todo needs 1-500 characters of text.".into());
            }
            todo.text = text.to_string();
        }
        todo.updated_at = now();
        self.save_todos(conversation_id, &todos)?;
        Ok(todos)
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
    /// One-time normalization of pre-scoped memory rows. Old scopes were a
    /// workspace path or `global`. `global` maps to `user` (identical reach);
    /// a path matching a saved project maps to `project:<id>`; any other path
    /// maps to `project:<path>` so it recalls only under that same workspace —
    /// never widened to user scope. Origins normalize to user/agent/survey.
    fn migrate_fact_scopes(&self) -> Result<()> {
        self.connection
            .execute_batch(
                "UPDATE memory_facts SET origin='user' WHERE origin='ui';
                 UPDATE memory_facts SET origin='survey' WHERE origin='auto-survey';
                 UPDATE memory_facts SET origin='agent' WHERE origin NOT IN ('user','agent','survey');
                 UPDATE memory_facts SET scope='user' WHERE scope='global';",
            )
            .map_err(db_error)?;
        let paths: Vec<(String, String)> = self
            .setting::<serde_json::Value>("workspace_index_v1")
            .ok()
            .and_then(|value| {
                value.get("projects")?.as_array().map(|projects| {
                    projects
                        .iter()
                        .filter_map(|project| Some((project.get("id")?.as_str()?.to_string(), project.get("path")?.as_str()?.to_string())))
                        .collect()
                })
            })
            .unwrap_or_default();
        let mut statement = self
            .connection
            .prepare("SELECT id, scope FROM memory_facts WHERE scope<>'user' AND scope NOT LIKE 'project:%' AND scope NOT LIKE 'conversation:%'")
            .map_err(db_error)?;
        let rows: Vec<(String, String)> = statement
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(db_error)?
            .flatten()
            .collect();
        for (id, scope) in rows {
            let key = scope.replace('\\', "/").trim_end_matches('/').to_lowercase();
            let project = paths
                .iter()
                .find(|(_, path)| path.replace('\\', "/").trim_end_matches('/').to_lowercase() == key);
            let normalized = match project {
                Some((id, _)) => format!("project:{id}"),
                None => format!("project:{scope}"),
            };
            self.connection
                .execute("UPDATE memory_facts SET scope=?2 WHERE id=?1", params![id, normalized])
                .map_err(db_error)?;
        }
        Ok(())
    }
    const FACT_COLUMNS: &'static str =
        "id, scope, fact, origin, reason, source_conversation_id, source_path, source_hash, source_branch, pinned, excluded, created_at, updated_at";
    fn fact_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<crate::memory::Fact> {
        Ok(crate::memory::Fact {
            id: row.get("id")?,
            scope: row.get("scope")?,
            fact: row.get("fact")?,
            origin: row.get("origin")?,
            reason: row.get("reason")?,
            source_conversation_id: row.get("source_conversation_id")?,
            source_path: row.get("source_path")?,
            source_hash: row.get("source_hash")?,
            source_branch: row.get("source_branch")?,
            pinned: row.get::<_, i64>("pinned")? != 0,
            excluded: row.get::<_, i64>("excluded")? != 0,
            stale: false,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }
    pub fn teach_fact(&self, scope: &str, fact: &str, origin: &str) -> Result<crate::memory::Fact> {
        self.teach_fact_ex(scope, fact, origin, None, None, None, None, None)
    }
    #[allow(clippy::too_many_arguments)]
    pub fn teach_fact_ex(
        &self,
        scope: &str,
        fact: &str,
        origin: &str,
        reason: Option<&str>,
        source_conversation_id: Option<&str>,
        source_path: Option<&str>,
        source_hash: Option<&str>,
        source_branch: Option<&str>,
    ) -> Result<crate::memory::Fact> {
        let record = crate::memory::Fact {
            id: uuid::Uuid::new_v4().to_string(),
            scope: crate::memory::validate_scope(scope)?.to_string(),
            fact: crate::memory::validate_fact(fact)?.to_string(),
            origin: match origin {
                "ui" | "user" => "user",
                "auto-survey" | "survey" => "survey",
                _ => "agent",
            }
            .to_string(),
            reason: reason.map(|r| r.chars().take(300).collect()),
            source_conversation_id: source_conversation_id.map(str::to_string),
            source_path: source_path.map(str::to_string),
            source_hash: source_hash.map(str::to_string),
            source_branch: source_branch.map(str::to_string),
            pinned: false,
            excluded: false,
            stale: false,
            created_at: now(),
            updated_at: now(),
        };
        self.connection
            .execute(
                "INSERT INTO memory_facts(id, scope, fact, origin, reason, source_conversation_id, source_path, source_hash, source_branch, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    record.id, record.scope, record.fact, record.origin, record.reason,
                    record.source_conversation_id, record.source_path, record.source_hash,
                    record.source_branch, record.created_at, record.updated_at
                ],
            )
            .map_err(db_error)?;
        Ok(record)
    }
    pub fn fact(&self, id: &str) -> Result<Option<crate::memory::Fact>> {
        self.connection
            .query_row(
                &format!("SELECT {} FROM memory_facts WHERE id=?1", Self::FACT_COLUMNS),
                [id],
                Self::fact_row,
            )
            .optional()
            .map_err(db_error)
    }
    pub fn update_fact(&self, fact: &crate::memory::Fact) -> Result<()> {
        self.connection
            .execute(
                "UPDATE memory_facts SET fact=?2, reason=?3, updated_at=?4 WHERE id=?1",
                params![fact.id, fact.fact, fact.reason, now()],
            )
            .map_err(db_error)?;
        Ok(())
    }
    /// pinned/excluded toggles — column name comes from the two command
    /// handlers only, never user input.
    pub fn set_fact_flag(&self, id: &str, column: &str, value: bool) -> Result<()> {
        if !["pinned", "excluded"].contains(&column) {
            return Err("Unknown fact flag.".into());
        }
        self.connection
            .execute(&format!("UPDATE memory_facts SET {column}=?2, updated_at=?3 WHERE id=?1"), params![id, value as i64, now()])
            .map_err(db_error)?;
        Ok(())
    }
    /// Facts in any of the given scopes, pinned first, newest otherwise.
    /// Excluded facts never come back — they exist only for the memory list.
    pub fn recall_facts_scoped(&self, scopes: &[String], limit: usize) -> Result<Vec<crate::memory::Fact>> {
        self.recall_facts_in(scopes, limit.min(50), false)
    }
    pub fn recall_facts_scoped_include_excluded(&self, scopes: &[String], limit: usize) -> Result<Vec<crate::memory::Fact>> {
        self.recall_facts_in(scopes, limit.min(200), true)
    }
    fn recall_facts_in(&self, scopes: &[String], limit: usize, include_excluded: bool) -> Result<Vec<crate::memory::Fact>> {
        if scopes.is_empty() {
            return Ok(Vec::new());
        }
        let marks = scopes.iter().map(|_| "?").collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT {} FROM memory_facts WHERE scope IN ({marks}) {} ORDER BY pinned DESC, updated_at DESC LIMIT {}",
            Self::FACT_COLUMNS,
            if include_excluded { "" } else { "AND excluded=0" },
            limit,
        );
        let mut statement = self.connection.prepare(&sql).map_err(db_error)?;
        let rows = statement
            .query_map(rusqlite::params_from_iter(scopes.iter()), Self::fact_row)
            .map_err(db_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(db_error)
    }
    /// Legacy single-scope recall (settings tooling). Also honors the
    /// exclusion flag so hidden facts stay out of every surface.
    pub fn recall_facts(&self, scope: &str, limit: usize) -> Result<Vec<crate::memory::Fact>> {
        let scope = crate::memory::validate_scope(scope)?.to_string();
        self.recall_facts_in(&[scope, "user".to_string()], limit, false)
    }
    pub fn forget_fact(&self, id: &str) -> Result<bool> {
        let changed = self
            .connection
            .execute("DELETE FROM memory_facts WHERE id=?1", [id])
            .map_err(db_error)?;
        Ok(changed > 0)
    }
    const TASK_COLUMNS: &'static str =
        "id, conversation_id, content, interaction_mode, status, position, run_id, wait_reason, error, next_action, parent_task_id, created_at, started_at, ended_at";
    fn task_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<crate::task_queue::Task> {
        Ok(crate::task_queue::Task {
            id: row.get("id")?,
            conversation_id: row.get("conversation_id")?,
            content: row.get("content")?,
            interaction_mode: row.get("interaction_mode")?,
            status: row.get("status")?,
            position: row.get("position")?,
            run_id: row.get("run_id")?,
            wait_reason: row.get("wait_reason")?,
            error: row.get("error")?,
            next_action: row.get("next_action")?,
            parent_task_id: row.get("parent_task_id")?,
            created_at: row.get("created_at")?,
            started_at: row.get("started_at")?,
            ended_at: row.get("ended_at")?,
        })
    }
    pub fn insert_task(&self, task: &crate::task_queue::Task) -> Result<()> {
        self.connection
            .execute(
                &format!(
                    "INSERT INTO tasks({}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                    Self::TASK_COLUMNS
                ),
                params![
                    task.id, task.conversation_id, task.content, task.interaction_mode, task.status,
                    task.position, task.run_id, task.wait_reason, task.error, task.next_action,
                    task.parent_task_id, task.created_at, task.started_at, task.ended_at
                ],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn task(&self, id: &str) -> Result<Option<crate::task_queue::Task>> {
        self.connection
            .query_row(&format!("SELECT {} FROM tasks WHERE id=?1", Self::TASK_COLUMNS), [id], Self::task_row)
            .optional()
            .map_err(db_error)
    }
    /// Next queue position: monotonic, ignores NULL (non-queued rows).
    pub fn next_task_position(&self) -> Result<i64> {
        let position: i64 = self
            .connection
            .query_row("SELECT COALESCE(MAX(position),0)+1 FROM tasks", [], |row| row.get(0))
            .map_err(db_error)?;
        Ok(position)
    }
    pub fn next_queued_task(&self) -> Result<Option<crate::task_queue::Task>> {
        self.connection
            .query_row(
                &format!("SELECT {} FROM tasks WHERE status='queued' ORDER BY position LIMIT 1", Self::TASK_COLUMNS),
                [],
                Self::task_row,
            )
            .optional()
            .map_err(db_error)
    }
    pub fn tasks(&self, statuses: &[&str]) -> Result<Vec<crate::task_queue::Task>> {
        let sql = if statuses.is_empty() {
            format!("SELECT {} FROM tasks ORDER BY created_at DESC LIMIT 200", Self::TASK_COLUMNS)
        } else {
            format!(
                "SELECT {} FROM tasks WHERE status IN ({}) ORDER BY created_at DESC LIMIT 200",
                Self::TASK_COLUMNS,
                statuses.iter().map(|s| format!("'{s}'")).collect::<Vec<_>>().join(",")
            )
        };
        let mut statement = self.connection.prepare(&sql).map_err(db_error)?;
        let rows = statement.query_map([], Self::task_row).map_err(db_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(db_error)
    }
    /// Every status change goes through here so the row and its audit trail
    /// always write together — never one without the other.
    pub fn transition_task(&self, task_id: &str, to: &str, reason: Option<&str>) -> Result<()> {
        let tx = self.connection.unchecked_transaction().map_err(db_error)?;
        let from: String = tx
            .query_row("SELECT status FROM tasks WHERE id=?1", [task_id], |row| row.get(0))
            .optional()
            .map_err(db_error)?
            .ok_or_else(|| "Unknown task.".to_string())?;
        if from == to {
            tx.commit().map_err(db_error)?;
            return Ok(());
        }
        let terminal = matches!(to, "completed" | "failed" | "cancelled" | "interrupted" | "awaiting_review");
        tx.execute(
            "UPDATE tasks SET status=?2, wait_reason=?3, error=COALESCE(?4, error), next_action=COALESCE(?5, next_action), ended_at=CASE WHEN ?6 THEN ?7 ELSE ended_at END, started_at=CASE WHEN ?2='running' AND started_at IS NULL THEN ?7 ELSE started_at END WHERE id=?1",
            params![task_id, to, if to == "waiting_resource" { reason } else { None }, if to == "failed" { reason } else { None }, if to == "awaiting_review" { reason } else { None }, terminal, now()],
        )
        .map_err(db_error)?;
        tx.execute(
            "INSERT INTO task_transitions(task_id, from_status, to_status, reason, at) VALUES (?1,?2,?3,?4,?5)",
            params![task_id, from, to, reason, now()],
        )
        .map_err(db_error)?;
        tx.commit().map_err(db_error)
    }
    /// Clear a wait/error/review note without changing the status (e.g. a
    /// resource wait resolving back to running).
    pub fn clear_task_wait(&self, task_id: &str) -> Result<()> {
        self.connection
            .execute("UPDATE tasks SET wait_reason=NULL, next_action=NULL WHERE id=?1", [task_id])
            .map_err(db_error)?;
        Ok(())
    }
    /// Requeue-position a task (resume/resolve paths) without a status change.
    pub fn move_task_to_queue_tail(&self, task_id: &str, position: i64) -> Result<()> {
        self.connection
            .execute("UPDATE tasks SET position=?2 WHERE id=?1", params![task_id, position])
            .map_err(db_error)?;
        Ok(())
    }
    /// Replace an interrupted task's replay content and clear its old run.
    pub fn reset_task_for_resume(&self, task_id: &str, content: &str, position: i64) -> Result<()> {
        self.connection
            .execute(
                "UPDATE tasks SET content=?2, position=?3, run_id=NULL, error=NULL, next_action=NULL WHERE id=?1",
                params![task_id, content, position],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn set_task_run(&self, task_id: &str, run_id: &str) -> Result<()> {
        self.connection
            .execute("UPDATE tasks SET run_id=?2 WHERE id=?1", params![task_id, run_id])
            .map_err(db_error)?;
        Ok(())
    }
    /// The task (if any) currently holding this conversation — used to queue
    /// follow-ups without touching the running task.
    pub fn running_task_for(&self, conversation_id: &str) -> Result<Option<crate::task_queue::Task>> {
        self.connection
            .query_row(
                &format!(
                    "SELECT {} FROM tasks WHERE conversation_id=?1 AND status IN ('running','waiting_approval','waiting_resource','paused') ORDER BY created_at DESC LIMIT 1",
                    Self::TASK_COLUMNS
                ),
                [conversation_id],
                Self::task_row,
            )
            .optional()
            .map_err(db_error)
    }
    pub fn task_transitions(&self, task_id: &str) -> Result<Vec<crate::task_queue::TaskTransition>> {
        let mut statement = self
            .connection
            .prepare("SELECT task_id, from_status, to_status, reason, at FROM task_transitions WHERE task_id=?1 ORDER BY id")
            .map_err(db_error)?;
        let rows = statement
            .query_map([task_id], |row| {
                Ok(crate::task_queue::TaskTransition {
                    task_id: row.get(0)?,
                    from_status: row.get(1)?,
                    to_status: row.get(2)?,
                    reason: row.get(3)?,
                    at: row.get(4)?,
                })
            })
            .map_err(db_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>().map_err(db_error)
    }
    pub fn add_steering(&self, task_id: &str, text: &str) -> Result<String> {
        let id = uuid::Uuid::new_v4().to_string();
        self.connection
            .execute(
                "INSERT INTO steering_messages(id, task_id, text, created_at) VALUES (?1,?2,?3,?4)",
                params![id, task_id, text, now()],
            )
            .map_err(db_error)?;
        Ok(id)
    }
    /// Claim every unapplied steering note exactly once — the SELECT and the
    /// mark-applied UPDATE commit together, so a restart or a racing read
    /// cannot deliver a note twice.
    pub fn claim_steering(&self, task_id: &str) -> Result<Vec<String>> {
        let tx = self.connection.unchecked_transaction().map_err(db_error)?;
        let claimed: Vec<(String, String)> = {
            let mut statement = tx
                .prepare("SELECT id, text FROM steering_messages WHERE task_id=?1 AND applied_at IS NULL ORDER BY created_at")
                .map_err(db_error)?;
            let rows = statement
                .query_map([task_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
                .map_err(db_error)?;
            rows.collect::<std::result::Result<Vec<_>, _>>().map_err(db_error)?
        };
        if !claimed.is_empty() {
            let ids = claimed.iter().map(|(id, _)| format!("'{id}'")).collect::<Vec<_>>().join(",");
            tx.execute(
                &format!("UPDATE steering_messages SET applied_at=?1 WHERE id IN ({ids}) AND applied_at IS NULL"),
                params![now()],
            )
            .map_err(db_error)?;
        }
        tx.commit().map_err(db_error)?;
        Ok(claimed.into_iter().map(|(_, text)| text).collect())
    }
    pub fn unapplied_steering(&self, task_id: &str) -> Result<i64> {
        self.connection
            .query_row(
                "SELECT COUNT(*) FROM steering_messages WHERE task_id=?1 AND applied_at IS NULL",
                [task_id],
                |row| row.get(0),
            )
            .map_err(db_error)
    }
    /// Startup: tasks left active by an unclean exit.
    pub fn active_tasks(&self) -> Result<Vec<crate::task_queue::Task>> {
        self.tasks(&["running", "waiting_approval", "waiting_resource", "interrupted", "awaiting_review"])
    }

    // -- model qualification runs -------------------------------------------

    pub fn insert_qualification(&self, row: &QualificationRow) -> Result<()> {
        self.connection
            .execute(
                "INSERT INTO model_qualifications(id, identity_hash, model_id, model_path, model_fingerprint, runtime_version, provider_id, config_json, started_at, ended_at, results_json, status)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                params![
                    row.id, row.identity_hash, row.model_id, row.model_path, row.model_fingerprint,
                    row.runtime_version, row.provider_id, row.config_json, row.started_at,
                    row.ended_at, row.results_json, row.status
                ],
            )
            .map_err(db_error)?;
        Ok(())
    }

    pub fn finalize_qualification(&self, id: &str, results_json: &str, status: &str, ended_at: i64) -> Result<()> {
        self.connection
            .execute(
                "UPDATE model_qualifications SET results_json=?2, status=?3, ended_at=?4 WHERE id=?1",
                params![id, results_json, status, ended_at],
            )
            .map_err(db_error)?;
        Ok(())
    }

    fn qualification_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<QualificationRow> {
        Ok(QualificationRow {
            id: row.get(0)?,
            identity_hash: row.get(1)?,
            model_id: row.get(2)?,
            model_path: row.get(3)?,
            model_fingerprint: row.get(4)?,
            runtime_version: row.get(5)?,
            provider_id: row.get(6)?,
            config_json: row.get(7)?,
            started_at: row.get(8)?,
            ended_at: row.get(9)?,
            results_json: row.get(10)?,
            status: row.get(11)?,
        })
    }

    /// Latest qualification for one identity — staleness is computed by the
    /// caller, not stored.
    pub fn latest_qualification(&self, identity_hash: &str) -> Result<Option<QualificationRow>> {
        self.connection
            .query_row(
                "SELECT id, identity_hash, model_id, model_path, model_fingerprint, runtime_version, provider_id, config_json, started_at, ended_at, results_json, status
                 FROM model_qualifications WHERE identity_hash=?1 ORDER BY started_at DESC LIMIT 1",
                [identity_hash],
                Self::qualification_row,
            )
            .optional()
            .map_err(db_error)
    }

    pub fn latest_qualification_any(&self) -> Result<Option<QualificationRow>> {
        self.connection
            .query_row(
                "SELECT id, identity_hash, model_id, model_path, model_fingerprint, runtime_version, provider_id, config_json, started_at, ended_at, results_json, status
                 FROM model_qualifications ORDER BY started_at DESC LIMIT 1",
                [],
                Self::qualification_row,
            )
            .optional()
            .map_err(db_error)
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
    pub fn update_message_created_at(&self, message_id: &str, created_at: i64) -> Result<()> {
        self.connection
            .execute(
                "UPDATE messages SET created_at=?1 WHERE id=?2",
                params![created_at, message_id],
            )
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
        if let Ok(Some(freeze)) = self.prompt_freeze(source_id) {
            let _ = self.save_prompt_freeze(&PromptFreeze { conversation_id: forked.id.clone(), ..freeze });
        }
        for message in prefix {
            let id = uuid::Uuid::new_v4().to_string();
            self.connection
                .execute(
                    "INSERT INTO messages(id, conversation_id, role, content, reasoning, status, created_at, error, metrics)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![id, forked.id, message.role, message.content, message.reasoning, message.status, message.created_at, message.error, message.metrics.map(|value| value.to_string())],
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
    fn migrate_historical_to_session_events(connection: &Connection) -> Result<()> {
        let session_events_empty: bool = connection
            .query_row(
                "SELECT NOT EXISTS(SELECT 1 FROM session_events LIMIT 1)",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);
        if !session_events_empty {
            return Ok(());
        }
        let has_messages: bool = connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM messages LIMIT 1)",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);
        if !has_messages {
            return Ok(());
        }

        let mut stmt = connection
            .prepare("SELECT id FROM conversations ORDER BY updated_at ASC")
            .map_err(db_error)?;
        let conv_ids: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .map_err(db_error)?
            .filter_map(std::result::Result::ok)
            .collect();

        for conv_id in conv_ids {
            let mut msg_stmt = connection
                .prepare(
                    "SELECT id, role, content, reasoning, status, created_at FROM messages WHERE conversation_id = ?1 ORDER BY created_at ASC",
                )
                .map_err(db_error)?;
            let messages: Vec<(String, String, String, String, String, i64)> = msg_stmt
                .query_map([&conv_id], |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                })
                .map_err(db_error)?
                .filter_map(std::result::Result::ok)
                .collect();

            let mut seq: i64 = 1;
            for (_id, role, content, reasoning, _status, created_at) in messages {
                if role == "user" {
                    let ev_id = uuid::Uuid::new_v4().to_string();
                    let payload = serde_json::json!({"role": "user", "content": content}).to_string();
                    let _ = connection.execute(
                        "INSERT INTO session_events (id, conversation_id, run_id, seq, step_id, tool_call_id, event_type, payload, ignorable, created_at)
                         VALUES (?1, ?2, NULL, ?3, 'user_input', NULL, 'user_msg', ?4, 0, ?5)",
                        params![ev_id, conv_id, seq, payload, created_at],
                    );
                    seq += 1;
                } else if role == "assistant" {
                    if !reasoning.is_empty() {
                        let ev_id = uuid::Uuid::new_v4().to_string();
                        let payload = serde_json::json!({"text": reasoning}).to_string();
                        let _ = connection.execute(
                            "INSERT INTO session_events (id, conversation_id, run_id, seq, step_id, tool_call_id, event_type, payload, ignorable, created_at)
                             VALUES (?1, ?2, NULL, ?3, 'thinking', NULL, 'reasoning', ?4, 1, ?5)",
                            params![ev_id, conv_id, seq, payload, created_at],
                        );
                        seq += 1;
                    }
                    let ev_id = uuid::Uuid::new_v4().to_string();
                    let payload = serde_json::json!({"role": "assistant", "content": content}).to_string();
                    let _ = connection.execute(
                        "INSERT INTO session_events (id, conversation_id, run_id, seq, step_id, tool_call_id, event_type, payload, ignorable, created_at)
                         VALUES (?1, ?2, NULL, ?3, 'step_end', NULL, 'step_end', ?4, 0, ?5)",
                        params![ev_id, conv_id, seq, payload, created_at],
                    );
                    seq += 1;
                } else if role == "tool" {
                    let ev_id = uuid::Uuid::new_v4().to_string();
                    let payload = serde_json::json!({"result": content}).to_string();
                    let _ = connection.execute(
                        "INSERT INTO session_events (id, conversation_id, run_id, seq, step_id, tool_call_id, event_type, payload, ignorable, created_at)
                         VALUES (?1, ?2, NULL, ?3, 'tool_result', NULL, 'tool_result', ?4, 0, ?5)",
                        params![ev_id, conv_id, seq, payload, created_at],
                    );
                    seq += 1;
                }
            }
        }
        Ok(())
    }
    pub fn next_session_event_seq(&self, conversation_id: &str) -> Result<u64> {
        let max_seq: Option<i64> = self
            .connection
            .query_row(
                "SELECT MAX(seq) FROM session_events WHERE conversation_id = ?1",
                [conversation_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?
            .flatten();
        Ok((max_seq.unwrap_or(0) + 1) as u64)
    }
    pub fn append_session_event(&self, event: &crate::sessions::SessionEvent) -> Result<u64> {
        if !event.ignorable
            && !crate::sessions::KNOWN_EVENT_TYPES.contains(&event.event_type.as_str())
        {
            return Err(format!(
                "Unknown session event type '{}'.",
                event.event_type
            ));
        }
        let seq = if event.seq > 0 {
            event.seq
        } else {
            self.next_session_event_seq(&event.conversation_id)?
        };
        let payload_str = serde_json::to_string(&event.payload).map_err(|e| e.to_string())?;
        self.connection
            .execute(
                "INSERT INTO session_events (id, conversation_id, run_id, seq, step_id, tool_call_id, event_type, payload, ignorable, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    event.id,
                    event.conversation_id,
                    event.run_id,
                    seq as i64,
                    event.step_id,
                    event.tool_call_id,
                    event.event_type,
                    payload_str,
                    if event.ignorable { 1 } else { 0 },
                    event.created_at,
                ],
            )
            .map_err(db_error)?;
        Ok(seq)
    }
    pub fn session_events(
        &self,
        conversation_id: &str,
        from_seq: Option<u64>,
        limit: Option<u32>,
    ) -> Result<Vec<crate::sessions::SessionEvent>> {
        let from_seq = from_seq.unwrap_or(0) as i64;
        let limit = limit.unwrap_or(1000).min(5000) as i64;
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, conversation_id, run_id, seq, step_id, tool_call_id, event_type, payload, ignorable, created_at
                 FROM session_events
                 WHERE conversation_id = ?1 AND seq >= ?2
                 ORDER BY seq ASC
                 LIMIT ?3",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map(params![conversation_id, from_seq, limit], |row| {
                let payload_text: String = row.get(7)?;
                let payload: serde_json::Value =
                    serde_json::from_str(&payload_text).unwrap_or(serde_json::Value::Null);
                let seq_i64: i64 = row.get(3)?;
                let ignorable_i64: i64 = row.get(8)?;
                Ok(crate::sessions::SessionEvent {
                    id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    run_id: row.get(2)?,
                    seq: seq_i64 as u64,
                    step_id: row.get(4)?,
                    tool_call_id: row.get(5)?,
                    event_type: row.get(6)?,
                    payload,
                    ignorable: ignorable_i64 != 0,
                    created_at: row.get(9)?,
                })
            })
            .map_err(db_error)?;
        let events = rows
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        // Fail loud on a non-ignorable event this build cannot interpret;
        // ignorable unknowns stay tolerated (forward-compatible audit data).
        if let Some(event) = events.iter().find(|event| {
            !event.ignorable
                && !crate::sessions::KNOWN_EVENT_TYPES.contains(&event.event_type.as_str())
        }) {
            return Err(format!(
                "Conversation has an unknown session event type '{}'; it may need a newer version of LocalLM.",
                event.event_type
            ));
        }
        Ok(events)
    }
    pub fn fork_session_events(
        &self,
        source_id: &str,
        up_to_seq: u64,
    ) -> Result<(Conversation, u64)> {
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

        let transaction = self.connection.unchecked_transaction().map_err(db_error)?;
        let mut events = Vec::new();
        let mut cursor = 0;
        loop {
            let batch = self.session_events(source_id, Some(cursor), Some(5000))?;
            let last = batch.last().map(|event| event.seq);
            events.extend(batch.into_iter().filter(|event| event.seq <= up_to_seq));
            match last {
                Some(seq) if seq < up_to_seq => cursor = seq + 1,
                _ => break,
            }
        }
        if events.last().map(|event| event.seq) != Some(up_to_seq) {
            return Err("The selected fork event no longer exists.".into());
        }
        let messages = crate::sessions::fork_messages(&events, &self.messages(source_id)?);
        let forked = Conversation {
            id: uuid::Uuid::new_v4().to_string(), title: format!("Fork of {}", source.title),
            updated_at: now(), ..source
        };
        transaction.execute("INSERT INTO conversations(id,title,updated_at,provider_id,model_id,provider_selection_required) VALUES(?1,?2,?3,?4,?5,?6)", params![forked.id,forked.title,forked.updated_at,forked.provider_id,forked.model_id,forked.provider_selection_required]).map_err(db_error)?;
        // Tool/model preferences are inherited; historical todos and goals are
        // not copied from the future state of the source conversation.
        for table in ["conversation_tools", "conversation_skills"] {
            transaction.execute(&format!("INSERT INTO {table}(conversation_id,value) SELECT ?1,value FROM {table} WHERE conversation_id=?2"), params![forked.id,source_id]).map_err(db_error)?;
        }
        transaction.execute("INSERT INTO conversation_presets(conversation_id,preset) SELECT ?1,preset FROM conversation_presets WHERE conversation_id=?2", params![forked.id,source_id]).map_err(db_error)?;
        transaction.execute("INSERT INTO conversation_prompt_freeze(conversation_id,selection_hash,tools_json,aliases_json,created_at) SELECT ?1,selection_hash,tools_json,aliases_json,created_at FROM conversation_prompt_freeze WHERE conversation_id=?2", params![forked.id,source_id]).map_err(db_error)?;
        for message in messages {
            transaction.execute("INSERT INTO messages(id,conversation_id,role,content,reasoning,status,created_at,error,metrics) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)", params![uuid::Uuid::new_v4().to_string(),forked.id,message.role,message.content,message.reasoning,message.status,message.created_at,message.error,message.metrics.map(|value| value.to_string())]).map_err(db_error)?;
        }
        for event in &events {
            transaction.execute("INSERT INTO session_events(id,conversation_id,run_id,seq,step_id,tool_call_id,event_type,payload,ignorable,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", params![uuid::Uuid::new_v4().to_string(),forked.id,event.run_id,event.seq as i64,event.step_id,event.tool_call_id,event.event_type,event.payload.to_string(),event.ignorable,event.created_at]).map_err(db_error)?;
        }
        crate::workspace_ui::inherit_task_meta(self, source_id, &forked.id)?;
        transaction.commit().map_err(db_error)?;
        Ok((forked, events.len() as u64))
    }

    /// Called once at application startup, never when opening a helper connection.
    pub fn recover_interrupted_runs(&self) -> Result<()> {
        let transaction = self.connection.unchecked_transaction().map_err(db_error)?;
        transaction.execute("UPDATE runs SET status='outcome_unknown' WHERE status IN ('preparing','generating','awaiting_approval','executing_tools','preparing_next_round')", []).map_err(db_error)?;
        transaction.execute("UPDATE messages SET status='interrupted', error=CASE WHEN role='assistant' AND error IS NULL
            THEN 'Stopped: the app closed while this reply was in progress. Send \"continue\" or Retry to resume.' ELSE error END
            WHERE status='streaming'", []).map_err(db_error)?;
        transaction.commit().map_err(db_error)
    }

    /// Startup repair for crash-truncated turns, called once from app setup.
    /// The log is append-only, so an open tail — a `tool_call` with no
    /// `tool_result`, a `step_start` with no `step_end`, a `turn_start` with
    /// no `turn_end` — can only be closed by appending synthetic end events
    /// marked as interrupted. Runs only at startup: `Store::open` is also used
    /// mid-turn by subagent and catalog helpers, where a live tail must stay
    /// open. Returns the number of synthetic events written.
    pub fn repair_open_turn_tails(&self) -> Result<usize> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT conversation_id, run_id, step_id, tool_call_id, event_type,
                        CASE WHEN event_type='tool_call' THEN payload ELSE '' END
                 FROM session_events
                 WHERE event_type IN ('turn_start','turn_end','step_start','step_end','tool_call','tool_result')
                 ORDER BY conversation_id, seq",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;

        // Repair unfinished tails and any calls that were left unpaired even
        // though their turn_end was recorded. A late tool_result still closes
        // its call, so track call pairs across the whole conversation.
        let mut repaired = 0usize;
        let mut conversation: Option<String> = None;
        let mut tail: Vec<TailEvent> = Vec::new();
        let mut unpaired: Vec<(TailEvent, bool)> = Vec::new();
        for row in rows {
            if conversation.as_deref() != Some(row.0.as_str()) {
                let completed = unpaired.drain(..).filter_map(|(event, ended)| ended.then_some(event)).collect();
                repaired += self.repair_open_tail(conversation.as_deref(), completed)?;
                repaired += self.repair_open_tail(conversation.as_deref(), std::mem::take(&mut tail))?;
                conversation = Some(row.0);
            }
            let event = (row.1, row.2, row.3, row.4, row.5);
            match event.3.as_str() {
                "tool_call" if event.2.is_some() => unpaired.push((event.clone(), false)),
                "tool_result" if event.2.is_some() => {
                    unpaired.retain(|(call, _)| call.0 != event.0 || call.2 != event.2);
                }
                "turn_end" => for (_, ended) in &mut unpaired { *ended = true; },
                _ => {}
            }
            if event.3 == "turn_end" {
                tail.clear();
            } else {
                tail.push(event);
            }
        }
        let completed = unpaired.drain(..).filter_map(|(event, ended)| ended.then_some(event)).collect();
        repaired += self.repair_open_tail(conversation.as_deref(), completed)?;
        repaired += self.repair_open_tail(conversation.as_deref(), tail)?;
        Ok(repaired)
    }

    fn repair_open_tail(
        &self,
        conversation_id: Option<&str>,
        tail: Vec<TailEvent>,
    ) -> Result<usize> {
        // Fields: run_id, step_id, tool_call_id, event_type, payload JSON.
        let Some(conversation_id) = conversation_id else {
            return Ok(0);
        };
        let mut calls: Vec<(Option<String>, Option<String>, String, serde_json::Value)> =
            Vec::new();
        let mut steps: Vec<(Option<String>, String)> = Vec::new();
        let mut turns: Vec<Option<String>> = Vec::new();
        for (run_id, step_id, call_id, kind, payload) in &tail {
            match kind.as_str() {
                "turn_start" => turns.push(run_id.clone()),
                "step_start" => {
                    if let Some(step) = step_id {
                        steps.push((run_id.clone(), step.clone()));
                    }
                }
                "tool_call" => {
                    if let Some(id) = call_id {
                        let name = serde_json::from_str::<serde_json::Value>(payload)
                            .ok()
                            .and_then(|value| value.get("name").cloned())
                            .unwrap_or(serde_json::Value::Null);
                        calls.push((run_id.clone(), step_id.clone(), id.clone(), name));
                    }
                }
                "tool_result" => {
                    if let Some(id) = call_id {
                        calls.retain(|(open_run, _, open_id, _)| open_run != run_id || open_id != id);
                    }
                }
                "step_end" => {
                    if let Some(step) = step_id {
                        if let Some(pos) = steps.iter().rposition(|(open_run, open)| open_run == run_id && open == step) {
                            steps.remove(pos);
                        }
                    }
                }
                _ => {}
            }
        }
        let mut repaired = 0usize;
        let now = now();
        for (run_id, step_id, call_id, name) in calls {
            if let Some(id) = run_id.as_deref() {
                if self.run(id)?.is_some_and(|run| run.status == crate::agent_run::RunState::Completed) {
                    self.update_run_status(id, crate::agent_run::RunState::OutcomeUnknown,
                        Some("A tool call has no recorded result; its outcome may be unknown."), None)?;
                }
            }
            self.append_session_event(&crate::sessions::SessionEvent {
                id: uuid::Uuid::new_v4().to_string(),
                conversation_id: conversation_id.to_string(),
                run_id,
                seq: 0,
                step_id,
                tool_call_id: Some(call_id.clone()),
                event_type: "tool_result".into(),
                payload: serde_json::json!({"callId": call_id, "name": name, "decision": "unknown", "result": {"isError": true, "outcome": "unknown", "message": "The app stopped before this action's result was recorded. It may already have changed data. Do not automatically repeat it."}}),
                ignorable: false,
                created_at: now,
            })?;
            repaired += 1;
        }
        for (run_id, step_id) in steps {
            self.append_session_event(&crate::sessions::SessionEvent {
                id: uuid::Uuid::new_v4().to_string(),
                conversation_id: conversation_id.to_string(),
                run_id,
                seq: 0,
                step_id: Some(step_id),
                tool_call_id: None,
                event_type: "step_end".into(),
                payload: serde_json::json!({"interrupted": true}),
                ignorable: false,
                created_at: now,
            })?;
            repaired += 1;
        }
        for run_id in turns {
            self.append_session_event(&crate::sessions::SessionEvent {
                id: uuid::Uuid::new_v4().to_string(),
                conversation_id: conversation_id.to_string(),
                run_id,
                seq: 0,
                step_id: None,
                tool_call_id: None,
                event_type: "turn_end".into(),
                payload: serde_json::json!({"status": "interrupted"}),
                ignorable: false,
                created_at: now,
            })?;
            repaired += 1;
        }
        if repaired > 0 {
            eprintln!(
                "Startup repair closed {repaired} interrupted session event(s) in conversation {conversation_id}."
            );
        }
        Ok(repaired)
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
    pub fn conversation_exists(&self, id: &str) -> Result<bool> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1)",
                [id],
                |row| row.get(0),
            )
            .map_err(db_error)
    }
    pub fn conversation_tools(&self, id: &str) -> Result<ConversationTools> {
        let value: Option<Option<String>> = self.connection.query_row(
            "SELECT t.value FROM conversations c LEFT JOIN conversation_tools t ON t.conversation_id=c.id WHERE c.id=?1", [id], |row| row.get(0),
        ).optional().map_err(db_error)?;        let tools: ConversationTools = match value {
            None => return Err("Conversation no longer exists.".into()),
            Some(None) => ConversationTools::default(),
            Some(Some(value)) => serde_json::from_str(&value)
                .map_err(|_| "Stored conversation tools are invalid.")?,
        };
        tools.validate()?;
        Ok(tools)
    }
    pub fn prompt_freeze(&self, conversation_id: &str) -> Result<Option<PromptFreeze>> {
        self.connection
            .query_row(
                "SELECT conversation_id, selection_hash, tools_json, aliases_json, created_at
                 FROM conversation_prompt_freeze WHERE conversation_id=?1",
                [conversation_id],
                |row| {
                    Ok(PromptFreeze {
                        conversation_id: row.get(0)?,
                        selection_hash: row.get(1)?,
                        tools_json: row.get(2)?,
                        aliases_json: row.get(3)?,
                        created_at: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(db_error)
    }
    pub fn save_prompt_freeze(&self, freeze: &PromptFreeze) -> Result<()> {
        self.connection
            .execute(
                "INSERT INTO conversation_prompt_freeze(conversation_id, selection_hash, tools_json, aliases_json, created_at)
                 VALUES(?1,?2,?3,?4,?5)
                 ON CONFLICT(conversation_id) DO UPDATE SET
                    selection_hash=excluded.selection_hash,
                    tools_json=excluded.tools_json,
                    aliases_json=excluded.aliases_json,
                    created_at=excluded.created_at",
                params![
                    freeze.conversation_id,
                    freeze.selection_hash,
                    freeze.tools_json,
                    freeze.aliases_json,
                    freeze.created_at
                ],
            )
            .map_err(db_error)?;
        Ok(())
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
    pub fn default_subscription_providers() -> Vec<ProviderConnection> {
        default_subscription_providers()
    }
}

pub fn default_subscription_providers() -> Vec<ProviderConnection> {
    vec![
        ProviderConnection {
            id: "chatgpt-subscription".into(),
            name: "ChatGPT (Subscription)".into(),
            api_format: crate::providers::CHATGPT_SUBSCRIPTION.into(),
            base_url: "https://chatgpt.com/backend-api/codex".into(),
            verified: false,
            last_tested_at: None,
            models: vec![
                RemoteModel { id: "gpt-4o".into(), context_length: Some(128_000), max_output_tokens: Some(16_384), supports_images: false, tool_support: crate::providers::ToolSupport::Supported },
                RemoteModel { id: "gpt-4o-mini".into(), context_length: Some(128_000), max_output_tokens: Some(16_384), supports_images: false, tool_support: crate::providers::ToolSupport::Supported },
                RemoteModel { id: "o3-mini".into(), context_length: Some(200_000), max_output_tokens: Some(100_000), supports_images: false, tool_support: crate::providers::ToolSupport::Supported },
                RemoteModel { id: "o1".into(), context_length: Some(200_000), max_output_tokens: Some(100_000), supports_images: false, tool_support: crate::providers::ToolSupport::Supported },
            ],
            has_api_key: false,
        },
        ProviderConnection {
            id: "grok-subscription".into(),
            name: "Grok (Subscription)".into(),
            api_format: crate::providers::GROK_SUBSCRIPTION.into(),
            base_url: "https://cli-chat-proxy.grok.com".into(),
            verified: false,
            last_tested_at: None,
            models: vec![
                RemoteModel { id: "grok-2".into(), context_length: Some(131_072), max_output_tokens: Some(8_192), supports_images: false, tool_support: crate::providers::ToolSupport::Supported },
                RemoteModel { id: "grok-2-latest".into(), context_length: Some(131_072), max_output_tokens: Some(8_192), supports_images: false, tool_support: crate::providers::ToolSupport::Supported },
                RemoteModel { id: "grok-3".into(), context_length: Some(131_072), max_output_tokens: Some(16_384), supports_images: false, tool_support: crate::providers::ToolSupport::Supported },
                RemoteModel { id: "grok-beta".into(), context_length: Some(131_072), max_output_tokens: Some(8_192), supports_images: false, tool_support: crate::providers::ToolSupport::Supported },
            ],
            has_api_key: false,
        },
    ]
}

impl Store {
    /// Research needs a small text window, not the complete chat and tool audits.
    pub fn recent_chat_context(&self, id: &str) -> Result<String> {
        let mut statement = self.connection.prepare_cached(
            "SELECT role,content FROM messages WHERE conversation_id=?1 AND role IN ('user','assistant') ORDER BY created_at DESC,rowid DESC",
        ).map_err(db_error)?;
        let rows = statement.query_map([id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))).map_err(db_error)?;
        let mut recent = Vec::new();
        for row in rows {
            let (role, content) = row.map_err(db_error)?;
            if content.trim().is_empty() { continue; }
            recent.push(format!("{role}: {}", content.chars().take(500).collect::<String>()));
            if recent.len() == 6 { break; }
        }
        recent.reverse();
        Ok(recent.join("\n"))
    }

    pub fn latest_user_content(&self, id: &str) -> Result<Option<String>> {
        self.connection.query_row(
            "SELECT content FROM messages WHERE conversation_id=?1 AND role='user' ORDER BY created_at DESC,rowid DESC LIMIT 1",
            [id], |row| row.get::<_, String>(0),
        ).optional().map(|content| content.map(|text| text.chars().take(8000).collect())).map_err(db_error)
    }

    pub fn messages(&self, id: &str) -> Result<Vec<Message>> {
        let mut statement = self.connection.prepare("SELECT id,conversation_id,role,content,reasoning,status,created_at,error,metrics FROM messages WHERE conversation_id=?1 ORDER BY created_at,rowid").map_err(db_error)?;
        let rows = statement
            .query_map([id], |row| {
                Ok(Message {
                    error: row.get(7)?,
                    metrics: row.get::<_, Option<String>>(8)?.and_then(|text| serde_json::from_str(&text).ok()),
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
            metrics: None,
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
            metrics: None,
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
    /// Commit the three records of a finished turn together.
    pub fn finish_turn(&self, message: &Message, run: &crate::agent_run::RunRecord, task: Option<&crate::research_tasks::ResearchTask>) -> Result<()> {
        let transaction = self.connection.unchecked_transaction().map_err(db_error)?;
        self.finish_message(&message.id, &message.content, &message.reasoning, &message.status, message.error.as_deref())?;
        self.update_run_status(&run.id, run.status, run.error.as_deref(), None)?;
        if let Some(task) = task { self.save_research_task(task)?; }
        transaction.commit().map_err(db_error)
    }

    pub fn stage_research_resume(&self, task: &crate::research_tasks::ResearchTask, checkpoint: &str) -> Result<()> {
        let transaction = self.connection.unchecked_transaction().map_err(db_error)?;
        self.save_research_task(task)?;
        self.save_setting(&format!("pending_resume:{}", task.conversation_id), &checkpoint)?;
        self.save_setting(&format!("pending_resume_task:{}", task.conversation_id), &task.id)?;
        transaction.commit().map_err(db_error)
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
    pub fn save_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
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
    /// (path, profile name) ACEs the sandbox granted — revocable on disable.
    pub fn isolation_grants(&self) -> Result<Vec<(String, String)>> {
        let mut statement = self
            .connection
            .prepare("SELECT path, profile FROM isolation_grants")
            .map_err(db_error)?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .map_err(db_error)?;
        Ok(rows.flatten().collect())
    }
    pub fn record_isolation_grant(&self, path: &str, profile: &str) -> Result<()> {
        self.connection
            .execute(
                "INSERT OR REPLACE INTO isolation_grants(path, profile, granted_at) VALUES (?1, ?2, ?3)",
                rusqlite::params![path, profile, now()],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn clear_isolation_grants(&self) -> Result<()> {
        self.connection
            .execute("DELETE FROM isolation_grants", [])
            .map_err(db_error)?;
        Ok(())
    }

    // -- recipes ---------------------------------------------------------------

    fn recipe_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<crate::recipes::Recipe> {
        Ok(crate::recipes::Recipe {
            id: row.get(0)?,
            name: row.get(1)?,
            prompt: row.get(2)?,
            interval_minutes: row.get(3)?,
            enabled: row.get::<_, i64>(4)? != 0,
            next_run_at: row.get(5)?,
            last_run_at: row.get(6)?,
            created_at: row.get(7)?,
        })
    }
    pub fn recipes_list(&self) -> Result<Vec<crate::recipes::Recipe>> {
        let mut statement = self
            .connection
            .prepare("SELECT id, name, prompt, interval_minutes, enabled, next_run_at, last_run_at, created_at FROM recipes ORDER BY created_at")
            .map_err(db_error)?;
        let rows = statement
            .query_map([], Self::recipe_row)
            .map_err(db_error)?;
        Ok(rows.flatten().collect())
    }
    pub fn recipe(&self, id: &str) -> Result<Option<crate::recipes::Recipe>> {
        self.connection
            .prepare("SELECT id, name, prompt, interval_minutes, enabled, next_run_at, last_run_at, created_at FROM recipes WHERE id = ?1")
            .map_err(db_error)?
            .query_row([id], Self::recipe_row)
            .optional()
            .map_err(db_error)
    }
    pub fn recipe_save(&self, recipe: &crate::recipes::Recipe) -> Result<()> {
        self.connection
            .execute(
                "INSERT OR REPLACE INTO recipes(id, name, prompt, interval_minutes, enabled, next_run_at, last_run_at, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![recipe.id, recipe.name, recipe.prompt, recipe.interval_minutes, recipe.enabled as i64, recipe.next_run_at, recipe.last_run_at, recipe.created_at],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn recipe_mark_fired(&self, id: &str, at: i64, next_run_at: Option<i64>) -> Result<()> {
        self.connection
            .execute(
                "UPDATE recipes SET last_run_at = ?2, next_run_at = ?3 WHERE id = ?1",
                rusqlite::params![id, at, next_run_at],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn recipe_delete(&self, id: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM recipes WHERE id = ?1", [id])
            .map_err(db_error)?;
        Ok(())
    }
    /// Scheduled recipes that are due — enabled, with a past next_run_at.
    pub fn recipes_due(&self, at: i64) -> Result<Vec<crate::recipes::Recipe>> {
        let mut statement = self
            .connection
            .prepare("SELECT id, name, prompt, interval_minutes, enabled, next_run_at, last_run_at, created_at FROM recipes WHERE enabled = 1 AND next_run_at IS NOT NULL AND next_run_at <= ?1 ORDER BY next_run_at")
            .map_err(db_error)?;
        let rows = statement
            .query_map([at], Self::recipe_row)
            .map_err(db_error)?;
        Ok(rows.flatten().collect())
    }

    // -- document collections --------------------------------------------------

    fn doc_collection_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<crate::documents::DocCollection> {
        Ok(crate::documents::DocCollection {
            id: row.get(0)?,
            name: row.get(1)?,
            root: row.get(2)?,
            file_count: row.get(3)?,
            chunk_count: row.get(4)?,
            indexed_at: row.get(5)?,
            error: row.get(6)?,
            created_at: row.get(7)?,
        })
    }
    pub fn doc_collections(&self) -> Result<Vec<crate::documents::DocCollection>> {
        let mut statement = self
            .connection
            .prepare("SELECT id, name, root, file_count, chunk_count, indexed_at, error, created_at FROM doc_collections ORDER BY created_at")
            .map_err(db_error)?;
        let rows = statement.query_map([], Self::doc_collection_row).map_err(db_error)?;
        Ok(rows.flatten().collect())
    }
    pub fn doc_collection(&self, id: &str) -> Result<Option<crate::documents::DocCollection>> {
        self.connection
            .prepare("SELECT id, name, root, file_count, chunk_count, indexed_at, error, created_at FROM doc_collections WHERE id = ?1")
            .map_err(db_error)?
            .query_row([id], Self::doc_collection_row)
            .optional()
            .map_err(db_error)
    }
    pub fn doc_collection_save(&self, collection: &crate::documents::DocCollection) -> Result<()> {
        self.connection
            .execute(
                "INSERT OR REPLACE INTO doc_collections(id, name, root, file_count, chunk_count, indexed_at, error, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![collection.id, collection.name, collection.root, collection.file_count, collection.chunk_count, collection.indexed_at, collection.error, collection.created_at],
            )
            .map_err(db_error)?;
        Ok(())
    }
    /// Atomically replace a collection's chunks and stats — a failed scan never
    /// leaves a half-indexed set behind.
    pub fn doc_chunks_replace(
        &self,
        collection_id: &str,
        chunks: &[(String, i64, i64, String)],
        file_count: i64,
        error: Option<&str>,
    ) -> Result<()> {
        let transaction = self.connection.unchecked_transaction().map_err(db_error)?;
        transaction
            .execute("DELETE FROM doc_chunks WHERE collection_id = ?1", [collection_id])
            .map_err(db_error)?;
        {
            let mut statement = transaction
                .prepare("INSERT INTO doc_chunks(collection_id, path, line_start, line_end, content) VALUES (?1, ?2, ?3, ?4, ?5)")
                .map_err(db_error)?;
            for (path, start, end, body) in chunks {
                statement
                    .execute(rusqlite::params![collection_id, path, start, end, body])
                    .map_err(db_error)?;
            }
        }
        transaction
            .execute(
                "UPDATE doc_collections SET file_count = ?2, chunk_count = ?3, indexed_at = ?4, error = ?5 WHERE id = ?1",
                rusqlite::params![collection_id, file_count, chunks.len() as i64, now(), error],
            )
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)
    }
    pub fn doc_collection_delete(&self, id: &str) -> Result<()> {
        let transaction = self.connection.unchecked_transaction().map_err(db_error)?;
        transaction
            .execute("DELETE FROM doc_chunks WHERE collection_id = ?1", [id])
            .map_err(db_error)?;
        transaction
            .execute("DELETE FROM doc_collections WHERE id = ?1", [id])
            .map_err(db_error)?;
        transaction.commit().map_err(db_error)
    }
    /// (collection_id, collection name, path, line_start, line_end, snippet)
    pub fn doc_chunks_search(
        &self,
        match_query: &str,
        collection_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<(String, String, String, i64, i64, String)>> {
        let sql = match collection_id {
            Some(_) => "SELECT c.id, c.name, doc_chunks.path, doc_chunks.line_start, doc_chunks.line_end, \
                 snippet(doc_chunks, 0, '', '', '…', 40) FROM doc_chunks \
                 JOIN doc_collections c ON c.id = doc_chunks.collection_id \
                 WHERE doc_chunks MATCH ?1 AND doc_chunks.collection_id = ?2 \
                 ORDER BY bm25(doc_chunks) LIMIT ?3",
            None => "SELECT c.id, c.name, doc_chunks.path, doc_chunks.line_start, doc_chunks.line_end, \
                 snippet(doc_chunks, 0, '', '', '…', 40) FROM doc_chunks \
                 JOIN doc_collections c ON c.id = doc_chunks.collection_id \
                 WHERE doc_chunks MATCH ?1 ORDER BY bm25(doc_chunks) LIMIT ?2",
        };
        let mut statement = self.connection.prepare(sql).map_err(db_error)?;
        let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<(String, String, String, i64, i64, String)> {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))
        };
        let rows = match collection_id {
            Some(id) => statement.query_map(rusqlite::params![match_query, id, limit], map_row),
            None => statement.query_map(rusqlite::params![match_query, limit], map_row),
        }
        .map_err(db_error)?;
        Ok(rows.flatten().collect())
    }
    pub fn execution_config(&self) -> Result<crate::execution::ExecutionConfig> {
        self.setting("execution")
    }
    pub fn save_execution_config(&self, config: &crate::execution::ExecutionConfig) -> Result<()> {
        self.save_setting("execution", config)
    }
    pub fn save_local_execution_config(&self, config: &crate::execution::ExecutionConfig) -> Result<()> {
        let transaction = self.connection.unchecked_transaction().map_err(db_error)?;
        self.save_execution_config(config)?;
        self.save_setting(crate::sandbox::PROVIDER_KEY, &"local")?;
        transaction.commit().map_err(db_error)
    }
    pub fn save_workspace_path(&self, path: &str) -> Result<()> {
        self.save_setting("workspace_path", &path)
    }

    // ---- Task workspaces (isolated worktrees/snapshots; see task_workspace.rs) ----
    fn task_workspace_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<crate::task_workspace::TaskWorkspace> {
        let exclusions_json: String = row.get("exclusions_json")?;
        Ok(crate::task_workspace::TaskWorkspace {
            id: row.get("id")?,
            conversation_id: row.get("conversation_id")?,
            source_root: row.get("source_root")?,
            kind: row.get("kind")?,
            isolated_root: row.get("isolated_root")?,
            branch: row.get("branch")?,
            baseline_commit: row.get("baseline_commit")?,
            status: row.get("status")?,
            exclusions: serde_json::from_str(&exclusions_json).unwrap_or_default(),
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }
    const TASK_WORKSPACE_COLUMNS: &'static str =
        "id, conversation_id, source_root, kind, isolated_root, branch, baseline_commit, status, exclusions_json, created_at, updated_at";
    pub fn save_task_workspace(&self, task: &crate::task_workspace::TaskWorkspace) -> Result<()> {
        let exclusions_json = serde_json::to_string(&task.exclusions).map_err(|error| error.to_string())?;
        self.connection
            .execute(
                "INSERT INTO task_workspaces(id, conversation_id, source_root, kind, isolated_root, branch, baseline_commit, status, exclusions_json, created_at, updated_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
                 ON CONFLICT(id) DO UPDATE SET status=excluded.status, exclusions_json=excluded.exclusions_json,
                    baseline_commit=excluded.baseline_commit, branch=excluded.branch, updated_at=excluded.updated_at",
                rusqlite::params![
                    task.id, task.conversation_id, task.source_root, task.kind, task.isolated_root,
                    task.branch, task.baseline_commit, task.status, exclusions_json,
                    task.created_at, task.updated_at
                ],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn task_workspace(&self, id: &str) -> Result<Option<crate::task_workspace::TaskWorkspace>> {
        self.connection
            .query_row(
                &format!("SELECT {} FROM task_workspaces WHERE id=?1", Self::TASK_WORKSPACE_COLUMNS),
                [id],
                Self::task_workspace_row,
            )
            .optional()
            .map_err(db_error)
    }
    /// The conversation's live isolated workspace, if any. At most one row can
    /// be `active` per conversation (partial unique index).
    pub fn active_task_workspace(&self, conversation_id: &str) -> Result<Option<crate::task_workspace::TaskWorkspace>> {
        self.connection
            .query_row(
                &format!("SELECT {} FROM task_workspaces WHERE conversation_id=?1 AND status='active'", Self::TASK_WORKSPACE_COLUMNS),
                [conversation_id],
                Self::task_workspace_row,
            )
            .optional()
            .map_err(db_error)
    }
    /// Same lookup keyed by the isolated root; lets tool/execution paths that
    /// only know their resolved directory find their owning task.
    pub fn active_task_workspace_by_root(&self, isolated_root: &str) -> Result<Option<crate::task_workspace::TaskWorkspace>> {
        self.connection
            .query_row(
                &format!("SELECT {} FROM task_workspaces WHERE isolated_root=?1 AND status='active'", Self::TASK_WORKSPACE_COLUMNS),
                [isolated_root],
                Self::task_workspace_row,
            )
            .optional()
            .map_err(db_error)
    }
    pub fn save_task_apply(&self, apply: &crate::task_workspace::TaskApply) -> Result<()> {
        let files_json = serde_json::to_string(&apply.files).map_err(|error| error.to_string())?;
        self.connection
            .execute(
                "INSERT INTO task_applies(id, task_id, created_at, backup_dir, files_json, undone_at)
                 VALUES(?1,?2,?3,?4,?5,?6)",
                rusqlite::params![apply.id, apply.task_id, apply.created_at, apply.backup_dir, files_json, apply.undone_at],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn task_apply(&self, id: &str) -> Result<Option<crate::task_workspace::TaskApply>> {
        self.connection
            .query_row(
                "SELECT id, task_id, created_at, backup_dir, files_json, undone_at FROM task_applies WHERE id=?1",
                [id],
                Self::task_apply_row,
            )
            .optional()
            .map_err(db_error)
    }
    pub fn task_applies(&self, task_id: &str) -> Result<Vec<crate::task_workspace::TaskApply>> {
        let mut statement = self
            .connection
            .prepare("SELECT id, task_id, created_at, backup_dir, files_json, undone_at FROM task_applies WHERE task_id=?1 ORDER BY created_at")
            .map_err(db_error)?;
        let rows = statement
            .query_map([task_id], Self::task_apply_row)
            .map_err(db_error)?;
        let mut applies = Vec::new();
        for row in rows {
            applies.push(row.map_err(db_error)?);
        }
        Ok(applies)
    }
    pub fn mark_task_apply_undone(&self, id: &str, at: i64) -> Result<()> {
        self.connection
            .execute("UPDATE task_applies SET undone_at=?2 WHERE id=?1", rusqlite::params![id, at])
            .map_err(db_error)?;
        Ok(())
    }
    fn task_apply_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<crate::task_workspace::TaskApply> {
        let files_json: String = row.get("files_json")?;
        Ok(crate::task_workspace::TaskApply {
            id: row.get("id")?,
            task_id: row.get("task_id")?,
            created_at: row.get("created_at")?,
            backup_dir: row.get("backup_dir")?,
            files: serde_json::from_str(&files_json).unwrap_or_default(),
            undone_at: row.get("undone_at")?,
        })
    }
    fn criterion_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<crate::verification::TaskCriterion> {
        Ok(crate::verification::TaskCriterion {
            id: row.get("id")?,
            conversation_id: row.get("conversation_id")?,
            text: row.get("text")?,
            origin: row.get("origin")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
        })
    }
    const CRITERION_COLUMNS: &'static str = "id, conversation_id, text, origin, created_at, updated_at";
    /// Insert or update the text of a completion criterion for a conversation.
    pub fn save_criterion(&self, criterion: &crate::verification::TaskCriterion) -> Result<()> {
        self.connection
            .execute(
                "INSERT INTO task_criteria(id, conversation_id, text, origin, created_at, updated_at)
                 VALUES(?1,?2,?3,?4,?5,?6)
                 ON CONFLICT(id) DO UPDATE SET text=excluded.text, updated_at=excluded.updated_at",
                rusqlite::params![
                    criterion.id,
                    criterion.conversation_id,
                    criterion.text,
                    criterion.origin,
                    criterion.created_at,
                    criterion.updated_at
                ],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn criterion(&self, id: &str) -> Result<Option<crate::verification::TaskCriterion>> {
        self.connection
            .query_row(
                &format!("SELECT {} FROM task_criteria WHERE id=?1", Self::CRITERION_COLUMNS),
                [id],
                Self::criterion_row,
            )
            .optional()
            .map_err(db_error)
    }
    pub fn criteria_for(&self, conversation_id: &str) -> Result<Vec<crate::verification::TaskCriterion>> {
        let mut statement = self
            .connection
            .prepare(&format!("SELECT {} FROM task_criteria WHERE conversation_id=?1 ORDER BY created_at", Self::CRITERION_COLUMNS))
            .map_err(db_error)?;
        let rows = statement.query_map([conversation_id], Self::criterion_row).map_err(db_error)?;
        let mut criteria = Vec::new();
        for row in rows {
            criteria.push(row.map_err(db_error)?);
        }
        Ok(criteria)
    }
    /// Deleting a criterion detaches its records (`ON DELETE SET NULL`) rather
    /// than erasing the evidence they captured.
    pub fn delete_criterion(&self, id: &str) -> Result<()> {
        self.connection
            .execute("DELETE FROM task_criteria WHERE id=?1", [id])
            .map_err(db_error)?;
        Ok(())
    }
    fn verification_record_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<crate::verification::VerificationRecord> {
        let artifacts_json: String = row.get("artifacts_json")?;
        Ok(crate::verification::VerificationRecord {
            id: row.get("id")?,
            conversation_id: row.get("conversation_id")?,
            criterion_id: row.get("criterion_id")?,
            kind: row.get("kind")?,
            method: row.get("method")?,
            command: row.get("command")?,
            environment: row.get("environment")?,
            started_at: row.get("started_at")?,
            ended_at: row.get("ended_at")?,
            exit_code: row.get("exit_code")?,
            output_excerpt: row.get("output_excerpt")?,
            output_artifact: row.get("output_artifact")?,
            output_truncated: row.get::<_, i64>("output_truncated")? != 0,
            artifacts: serde_json::from_str(&artifacts_json).unwrap_or_default(),
            result: row.get("result")?,
            limitations: row.get("limitations")?,
            fingerprint: row.get("fingerprint")?,
            created_at: row.get("created_at")?,
            stale: false,
        })
    }
    const VERIFICATION_COLUMNS: &'static str =
        "id, conversation_id, criterion_id, kind, method, command, environment, started_at, ended_at, exit_code, output_excerpt, output_artifact, output_truncated, artifacts_json, result, limitations, fingerprint, created_at";
    pub fn save_verification_record(&self, record: &crate::verification::VerificationRecord) -> Result<()> {
        let artifacts_json = serde_json::to_string(&record.artifacts).map_err(|error| error.to_string())?;
        self.connection
            .execute(
                "INSERT INTO verification_records(id, conversation_id, criterion_id, kind, method, command, environment, started_at, ended_at, exit_code, output_excerpt, output_artifact, output_truncated, artifacts_json, result, limitations, fingerprint, created_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
                rusqlite::params![
                    record.id, record.conversation_id, record.criterion_id, record.kind, record.method,
                    record.command, record.environment, record.started_at, record.ended_at, record.exit_code,
                    record.output_excerpt, record.output_artifact, record.output_truncated as i64,
                    artifacts_json, record.result, record.limitations, record.fingerprint, record.created_at
                ],
            )
            .map_err(db_error)?;
        Ok(())
    }
    pub fn verification_record(&self, id: &str) -> Result<Option<crate::verification::VerificationRecord>> {
        self.connection
            .query_row(
                &format!("SELECT {} FROM verification_records WHERE id=?1", Self::VERIFICATION_COLUMNS),
                [id],
                Self::verification_record_row,
            )
            .optional()
            .map_err(db_error)
    }
    pub fn verification_records_for(&self, conversation_id: &str) -> Result<Vec<crate::verification::VerificationRecord>> {
        let mut statement = self
            .connection
            .prepare(&format!("SELECT {} FROM verification_records WHERE conversation_id=?1 ORDER BY created_at", Self::VERIFICATION_COLUMNS))
            .map_err(db_error)?;
        let rows = statement.query_map([conversation_id], Self::verification_record_row).map_err(db_error)?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(db_error)?);
        }
        Ok(records)
    }
    pub fn save_active_skills(&self, ids: &[String]) -> Result<()> {
        self.save_setting("active_skills", &ids)
    }
    /// A conversation's own skill selection. A missing `conversation_skills`
    /// row means the conversation never customized skills and inherits the
    /// global list; a saved row is authoritative, even when it is empty.
    /// Unsaved (brand-new or stale) ids resolve to the global list.
    pub fn active_skills_for(&self, conversation_id: &str) -> Result<Vec<String>> {
        let value: Option<Option<String>> = self
            .connection
            .query_row(
                "SELECT s.value FROM conversations c LEFT JOIN conversation_skills s ON s.conversation_id=c.id WHERE c.id=?1",
                [conversation_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        match value {
            Some(Some(text)) => serde_json::from_str(&text)
                .map_err(|_| "Stored conversation skills are invalid.".into()),
            _ => self.active_skills(),
        }
    }
    /// Persist a conversation's own skill selection; writing any row marks
    /// the conversation as customized so it stops inheriting global changes.
    pub fn save_conversation_skills(&self, conversation_id: &str, ids: &[String]) -> Result<()> {
        let value = serde_json::to_string(ids).map_err(|error| error.to_string())?;
        self.connection
            .execute(
                "INSERT INTO conversation_skills(conversation_id, value) VALUES(?1,?2)
                 ON CONFLICT(conversation_id) DO UPDATE SET value=excluded.value",
                params![conversation_id, value],
            )
            .map_err(db_error)?;
        Ok(())
    }
    /// Drop a removed skill from every conversation's own selection so an
    /// uninstalled package can never stay active.
    pub fn forget_conversation_skill(&self, skill_id: &str) -> Result<()> {
        let mut statement = self
            .connection
            .prepare("SELECT conversation_id, value FROM conversation_skills")
            .map_err(db_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)?;
        for (conversation_id, text) in rows {
            let mut ids: Vec<String> = serde_json::from_str(&text)
                .map_err(|_| "Stored conversation skills are invalid.")?;
            if ids.iter().any(|id| id == skill_id) {
                ids.retain(|id| id != skill_id);
                self.save_conversation_skills(&conversation_id, &ids)?;
            }
        }
        Ok(())
    }
    pub fn save_preferences(&self, preferences: &Preferences) -> Result<()> {
        preferences.validate()?;
        self.save_setting("preferences", preferences)
    }
    /// Experimental activation must not publish half a model selection.
    pub(crate) fn save_model_runtime(&self, preferences: &Preferences, config: &RuntimeConfig) -> Result<()> {
        preferences.validate()?;
        config.validate()?;
        let transaction = self.connection.unchecked_transaction().map_err(db_error)?;
        for (key, value) in [("preferences", serde_json::to_string(preferences)), ("runtime", serde_json::to_string(config))] {
            let text = value.map_err(|e| e.to_string())?;
            transaction.execute("INSERT INTO settings VALUES (?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", params![key, text]).map_err(db_error)?;
        }
        transaction.commit().map_err(db_error)
    }

    pub fn local_tool_calling_supported(
        &self,
        runtime_path: &str,
        model_path: &str,
    ) -> Result<Option<bool>> {
        // Path-keyed booleans are the old unreliable signal. They stay on disk
        // and are treated as unknown so one bad parse cannot disable tools.
        let _support: std::collections::BTreeMap<String, bool> = self.setting(LOCAL_TOOL_CALLING_SUPPORT_KEY)?;
        let _ = (runtime_path, model_path);
        Ok(None)
    }
    pub fn tool_compatibility(&self, fingerprint: &str) -> Result<Option<String>> {
        let support: std::collections::BTreeMap<String, String> = self.setting(TOOL_COMPATIBILITY_KEY)?;
        Ok(support.get(fingerprint).cloned())
    }
    pub fn record_tool_compatibility(&self, fingerprint: &str, state: &str) -> Result<()> {
        let mut support: std::collections::BTreeMap<String, String> = self.setting(TOOL_COMPATIBILITY_KEY)?;
        support.insert(fingerprint.to_string(), state.to_string());
        self.save_setting(TOOL_COMPATIBILITY_KEY, &support)
    }
    pub fn record_local_tool_calling_support(
        &self,
        runtime_path: &str,
        model_path: &str,
        supported: bool,
    ) -> Result<()> {
        let mut support: std::collections::BTreeMap<String, bool> = self.setting(LOCAL_TOOL_CALLING_SUPPORT_KEY)?;
        support.insert(tool_calling_key(runtime_path, model_path), supported);
        self.save_setting(LOCAL_TOOL_CALLING_SUPPORT_KEY, &support)
    }

    pub fn save_research_session(
        &self,
        id: &str,
        conversation_id: Option<&str>,
        question: &str,
        answer: &str,
        sources_json: &str,
        trace_json: Option<&str>,
    ) -> Result<()> {
        self.connection
            .execute(
                "INSERT INTO research_sessions(id, conversation_id, question, answer, sources_json, trace_json, created_at) VALUES (?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(id) DO UPDATE SET conversation_id=excluded.conversation_id, question=excluded.question, answer=excluded.answer, sources_json=excluded.sources_json, trace_json=excluded.trace_json",
                params![id, conversation_id, question, answer, sources_json, trace_json, now()],
            )
            .map_err(db_error)?;
        Ok(())
    }

    pub fn list_research_sessions(
        &self,
        conversation_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<PersistedResearch>> {
        let mut stmt = self.connection.prepare(
            "SELECT id, conversation_id, question, answer, sources_json, trace_json, created_at FROM research_sessions WHERE (?1 IS NULL OR conversation_id = ?1) ORDER BY created_at DESC LIMIT ?2"
        ).map_err(db_error)?;
        let rows = stmt.query_map(params![conversation_id, limit], |row| {
            Ok(PersistedResearch {
                id: row.get(0)?,
                conversation_id: row.get::<_, Option<String>>(1)?,
                question: row.get(2)?,
                answer: row.get::<_, Option<String>>(3)?,
                sources_json: row.get(4)?,
                trace_json: row.get::<_, Option<String>>(5)?,
                created_at: row.get(6)?,
            })
        }).map_err(db_error)?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(db_error)
    }

    pub fn research_session(&self, id: &str) -> Result<Option<PersistedResearch>> {
        self.connection
            .query_row(
                "SELECT id, conversation_id, question, answer, sources_json, trace_json, created_at FROM research_sessions WHERE id = ?1",
                [id],
                |row| Ok(PersistedResearch {
                    id: row.get(0)?,
                    conversation_id: row.get::<_, Option<String>>(1)?,
                    question: row.get(2)?,
                    answer: row.get::<_, Option<String>>(3)?,
                    sources_json: row.get(4)?,
                    trace_json: row.get::<_, Option<String>>(5)?,
                    created_at: row.get(6)?,
                }),
            )
            .optional()
            .map_err(db_error)
    }

    pub fn delete_research_session(&self, id: &str) -> Result<bool> {
        let changed = self.connection
            .execute("DELETE FROM research_sessions WHERE id = ?1", [id])
            .map_err(db_error)?;
        Ok(changed > 0)
    }

    pub fn save_research_task(&self, task: &crate::research_tasks::ResearchTask) -> Result<()> {
        crate::research_tasks::ensure_table(&self.connection)?;
        let budgets = serde_json::to_string(&task.budgets).map_err(|error| error.to_string())?;
        let operations = serde_json::to_string(&task.operations).map_err(|error| error.to_string())?;
        let results = serde_json::to_string(&task.completed_results).map_err(|error| error.to_string())?;
        let requirements = serde_json::to_string(&task.requirements).map_err(|error| error.to_string())?;
        let findings = serde_json::to_string(&task.findings).map_err(|error| error.to_string())?;
        let pending = serde_json::to_string(&task.pending_work).map_err(|error| error.to_string())?;
        self.connection
            .execute(
                "INSERT INTO research_tasks(id, conversation_id, question, status, budgets_json, operations_json, results_json, requirements_json, findings_json, pending_json, created_at, updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)
                 ON CONFLICT(id) DO UPDATE SET status=excluded.status, budgets_json=excluded.budgets_json,
                 operations_json=excluded.operations_json, results_json=excluded.results_json,
                 requirements_json=excluded.requirements_json, findings_json=excluded.findings_json,
                 pending_json=excluded.pending_json, updated_at=excluded.updated_at",
                params![task.id, task.conversation_id, task.question, task.status.as_str(), budgets, operations, results, requirements, findings, pending, task.created_at, task.updated_at],
            )
            .map_err(db_error)?;
        Ok(())
    }

    pub fn research_task(&self, id: &str) -> Result<Option<crate::research_tasks::ResearchTask>> {
        crate::research_tasks::ensure_table(&self.connection)?;
        self.connection
            .query_row(
                "SELECT id, conversation_id, question, status, budgets_json, operations_json, results_json, requirements_json, findings_json, pending_json, created_at, updated_at FROM research_tasks WHERE id=?1",
                [id],
                |row| {
                    let status_str: String = row.get(3)?;
                    let budgets_str: String = row.get(4)?;
                    let operations_str: String = row.get(5)?;
                    let results_str: String = row.get(6)?;
                    let requirements_str: String = row.get(7)?;
                    let findings_str: String = row.get(8)?;
                    let pending_str: String = row.get(9)?;
                    Ok(crate::research_tasks::ResearchTask {
                        id: row.get(0)?,
                        conversation_id: row.get(1)?,
                        question: row.get(2)?,
                        status: crate::research_tasks::TaskStatus::parse(&status_str).unwrap_or(crate::research_tasks::TaskStatus::Failed),
                        budgets: serde_json::from_str(&budgets_str).unwrap_or_default(),
                        operations: serde_json::from_str(&operations_str).unwrap_or_default(),
                        completed_results: serde_json::from_str(&results_str).unwrap_or_default(),
                        requirements: serde_json::from_str(&requirements_str).unwrap_or_default(),
                        findings: serde_json::from_str(&findings_str).unwrap_or_default(),
                        pending_work: serde_json::from_str(&pending_str).unwrap_or_default(),
                        created_at: row.get(10)?,
                        updated_at: row.get(11)?,
                    })
                },
            )
            .optional()
            .map_err(db_error)
    }

    pub fn running_research_task_ids(&self) -> Result<Vec<String>> {
        crate::research_tasks::ensure_table(&self.connection)?;
        let mut statement = self
            .connection
            .prepare("SELECT id FROM research_tasks WHERE status IN ('running','paused') ORDER BY updated_at DESC LIMIT 50")
            .map_err(db_error)?;
        let ids = statement
            .query_map([], |row| row.get(0))
            .map_err(db_error)?
            .filter_map(|row| row.ok())
            .collect();
        Ok(ids)
    }

    pub fn recoverable_research_tasks(&self, conversation_id: &str) -> Result<Vec<crate::research_tasks::ResearchTask>> {
        crate::research_tasks::ensure_table(&self.connection)?;
        let mut statement = self
            .connection
            .prepare(
                "SELECT id FROM research_tasks WHERE conversation_id=?1 AND status IN ('running','paused') ORDER BY updated_at DESC LIMIT 20",
            )
            .map_err(db_error)?;
        let ids: Vec<String> = statement
            .query_map([conversation_id], |row| row.get(0))
            .map_err(db_error)?
            .filter_map(|row| row.ok())
            .collect();
        Ok(ids.into_iter().filter_map(|id| self.research_task(&id).ok().flatten()).collect())
    }
}

fn tool_calling_key(runtime_path: &str, model_path: &str) -> String {
    format!("{runtime_path}\u{1f}{model_path}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn research_window_matches_history_without_loading_tool_audits_and_reasoning() {
        let store = Store::open_memory().unwrap();
        let id = store.create_conversation().unwrap().id;
        let other = store.create_conversation().unwrap().id;
        let transaction = store.connection.unchecked_transaction().unwrap();
        for index in 0..1000 {
            let role = ["user", "assistant", "tool"][index % 3];
            store.connection.execute(
                "INSERT INTO messages(id,conversation_id,role,content,reasoning,status,created_at) VALUES(?1,?2,?3,?4,?5,'complete',?6)",
                params![format!("row-{index}"), id, role, format!("message {index}: {}", "界".repeat(600)), "private ".repeat(1000), (index / 3) as i64],
            ).unwrap();
        }
        // Blank messages must not consume the six-message window; timestamps
        // may tie, and another conversation must never contribute context.
        store.insert_message_at(&id, "assistant", "\t\n\u{2003}", "complete", 1000).unwrap();
        store.insert_message_at(&id, "assistant", "last answer", "complete", 1000).unwrap();
        store.insert_message_at(&id, "user", &"界".repeat(8100), "complete", 1000).unwrap();
        store.insert_message_at(&other, "user", "unrelated", "complete", 2000).unwrap();
        transaction.commit().unwrap();
        let legacy = || {
            let mut recent: Vec<_> = store.messages(&id).unwrap().into_iter().rev()
                .filter(|message| matches!(message.role.as_str(), "user" | "assistant") && !message.content.trim().is_empty())
                .take(6).map(|message| format!("{}: {}", message.role, message.content.chars().take(500).collect::<String>())).collect();
            recent.reverse();
            let objective = store.messages(&id).unwrap().into_iter().rev().find(|message| message.role == "user")
                .map(|message| message.content.chars().take(8000).collect::<String>());
            (recent.join("\n"), objective)
        };
        assert_eq!((store.recent_chat_context(&id).unwrap(), store.latest_user_content(&id).unwrap()), legacy());
        assert_eq!(store.recent_chat_context("missing").unwrap(), "");
        assert!(store.latest_user_content("missing").unwrap().is_none());
        let mut before = Vec::new();
        let mut after = Vec::new();
        for _ in 0..5 {
            let start = std::time::Instant::now();
            let _ = legacy();
            before.push(start.elapsed());
            let start = std::time::Instant::now();
            let _ = (store.recent_chat_context(&id).unwrap(), store.latest_user_content(&id).unwrap());
            after.push(start.elapsed());
        }
        before.sort(); after.sort();
        eprintln!("Research history (1,003 messages, debug build): median {:?} full reads -> {:?} projected reads", before[2], after[2]);
    }

    #[test]
    fn experimental_model_selection_rolls_back_both_settings_on_storage_failure() {
        let store = Store::open_memory().unwrap();
        let previous = serde_json::to_value(store.preferences().unwrap()).unwrap();
        let previous_config = store.runtime_config().unwrap();
        store.connection.execute_batch("CREATE TRIGGER reject_runtime BEFORE INSERT ON settings WHEN NEW.key='runtime' BEGIN SELECT RAISE(ABORT,'simulated storage failure'); END;").unwrap();
        let next = Preferences { model_path: "experimental-model.gguf".into(), ..Default::default() };
        assert!(store.save_model_runtime(&next, &RuntimeConfig::default()).is_err());
        assert_eq!(serde_json::to_value(store.preferences().unwrap()).unwrap(), previous);
        assert_eq!(store.runtime_config().unwrap(), previous_config);
    }

    #[test]
    fn response_metrics_survive_reopening_a_conversation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("metrics.sqlite");
        let (conversation_id, message_id);
        {
            let store = Store::open(&path).unwrap();
            conversation_id = store.create_conversation().unwrap().id;
            message_id = store.append_message(&conversation_id, "assistant", "Done", "complete").unwrap().id;
            store.save_message_metrics(&message_id, &serde_json::json!({
                "promptTokPerSec": 125.4, "ttftSecs": 0.42, "genTokPerSec": 38.6
            })).unwrap();
        }
        let store = Store::open(&path).unwrap();
        let message = store.messages(&conversation_id).unwrap().remove(0);
        assert_eq!(message.metrics.unwrap()["ttftSecs"], 0.42);
    }

    #[test]
    fn research_tasks_survive_restart_from_checkpoint() {
        let store = Store::open_memory().unwrap();
        let conversation = store.create_conversation().unwrap();
        let mut task = crate::research_tasks::ResearchTask::new(
            &conversation.id,
            "Long research question",
            crate::research_tasks::TaskBudgets::default(),
        )
        .unwrap();
        task.checkpoint_operation(
            "search",
            crate::research_tasks::OperationStatus::Completed,
            Some("3 hits".into()),
        );
        task.operations.push(crate::research_tasks::TaskOperation {
            id: "op-run".into(),
            kind: "read".into(),
            status: crate::research_tasks::OperationStatus::Running,
            attempts: 1,
            result_summary: None,
            error: None,
            updated_at: now(),
        });
        store.save_research_task(&task).unwrap();
        // Simulate restart: reload, mark interrupted, resume pending only.
        let mut reloaded = store.research_task(&task.id).unwrap().unwrap();
        assert_eq!(reloaded.completed_results.len(), 1);
        reloaded.mark_interrupted_for_recovery();
        store.save_research_task(&reloaded).unwrap();
        let resumed = store.research_task(&task.id).unwrap().unwrap();
        assert_eq!(resumed.operations[1].status, crate::research_tasks::OperationStatus::Unknown);
        assert_eq!(resumed.status, crate::research_tasks::TaskStatus::Paused);
        assert_eq!(resumed.completed_results.len(), 1);
        let recoverable = store.recoverable_research_tasks(&conversation.id).unwrap();
        assert_eq!(recoverable.len(), 1);
    }

    #[test]
    fn arex_gets_a_reasoning_safe_response_budget_without_lowering_explicit_limits() {
        let arex = Preferences { model_path: "C:/models/BAAI_AREX-Turbo-Q4_K_M.gguf".into(), max_tokens: 2048, ..Default::default() };
        assert_eq!(arex.clone().apply_model_defaults().max_tokens, 8192);
        assert_eq!(Preferences { max_tokens: 12000, ..arex.clone() }.apply_model_defaults().max_tokens, 12000);
        let base = Preferences { model_path: "C:/models/model.gguf".into(), max_tokens: 2048, ..Default::default() };
        assert_eq!(base.clone().apply_model_defaults().max_tokens, 2048);
        let fitted = crate::context::fit_response_budget(
            arex.clone().apply_model_defaults().max_tokens,
            8192,
        );
        assert_eq!(fitted, 4096);
        assert!(crate::context::validate_budget(2545, fitted, 8192).is_ok());
    }
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
            interaction_mode: crate::permissions::InteractionMode::Act,
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
    fn local_tool_call_compatibility_is_persisted_per_runtime_and_model() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("compatibility.sqlite");
        {
            let store = Store::open(&path).unwrap();
            assert_eq!(
                store.local_tool_calling_supported("runtime-a", "model-a").unwrap(),
                None,
            );
            store.record_local_tool_calling_support("runtime-a", "model-a", false).unwrap();
            store.record_local_tool_calling_support("runtime-b", "model-a", true).unwrap();
        }
        let store = Store::open(&path).unwrap();
        // The path-keyed false flag is historical and must not disable tools.
        assert_eq!(store.local_tool_calling_supported("runtime-a", "model-a").unwrap(), None);
        assert_eq!(store.local_tool_calling_supported("runtime-b", "model-a").unwrap(), None);
        store.record_tool_compatibility("fp-old", "unsupported").unwrap();
        store.record_tool_compatibility("fp-new", "supported").unwrap();
        assert_eq!(store.tool_compatibility("fp-old").unwrap().as_deref(), Some("unsupported"));
        assert_eq!(store.tool_compatibility("fp-new").unwrap().as_deref(), Some("supported"));
        assert_eq!(store.tool_compatibility("fp-replaced").unwrap(), None);
    }

    #[test]
    fn conversation_tools_reject_duplicates_and_excessive_combined_tools() {
        let mut settings = ConversationTools {
            access_mode: crate::permissions::AccessMode::Ask,
            interaction_mode: crate::permissions::InteractionMode::Act,
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
            access_mode: crate::permissions::AccessMode::Ask,
            interaction_mode: crate::permissions::InteractionMode::Act,
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
    fn remembered_permission_modes_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("remembered-permissions.sqlite");
        for mode in ["fullAccess", "autoApprove", "ask"] {
            // Old rows lack interactionMode; it deserializes to Act and is
            // written back on save.
            let expected = serde_json::json!({
                "accessMode": mode,
                "interactionMode": "act",
                "sources": [],
                "tools": [{ "connectorId": "deepwiki", "toolName": "read_wiki_structure" }]
            });
            {
                let store = Store::open(&path).unwrap();
                let selection: RememberedTools = serde_json::from_value(expected.clone()).unwrap();
                store.save_remembered_tools(&selection).unwrap();
            }
            let store = Store::open(&path).unwrap();
            assert_eq!(serde_json::to_value(store.remembered_tools().unwrap().unwrap()).unwrap(), expected);
        }
    }

    #[test]
    fn active_skill_selection_survives_new_chats_and_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("active-skills.sqlite");
        {
            let store = Store::open(&path).unwrap();
            store.save_active_skills(&["wiki-qa".into()]).unwrap();
            store.create_conversation().unwrap();
            store.create_conversation().unwrap();
            assert_eq!(store.active_skills().unwrap(), vec!["wiki-qa"]);
        }
        {
            let store = Store::open(&path).unwrap();
            assert_eq!(store.active_skills().unwrap(), vec!["wiki-qa"]);
            store.save_active_skills(&[]).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert!(store.active_skills().unwrap().is_empty());
    }

    #[test]
    fn legacy_remembered_tools_default_to_asking() {
        let selection: RememberedTools = serde_json::from_value(serde_json::json!({
            "sources": [], "tools": []
        })).unwrap();
        assert_eq!(serde_json::to_value(selection).unwrap()["accessMode"], "ask");
    }

    #[test]
    fn legacy_defaults_inherit_saved_mode_once_without_overwriting_tools() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("legacy-defaults.sqlite");
        {
            let store = Store::open(&path).unwrap();
            let conversation = store.create_conversation().unwrap();
            store.save_conversation_tools(&conversation.id, &ConversationTools {
                access_mode: crate::permissions::AccessMode::FullAccess,
                ..Default::default()
            }).unwrap();
            store.save_setting("remembered_tools", &serde_json::json!({"sources":["__execution"],"tools":[]})).unwrap();
        }
        {
            let store = Store::open(&path).unwrap();
            let mut remembered = store.remembered_tools().unwrap().unwrap();
            assert_eq!(remembered.access_mode, crate::permissions::AccessMode::FullAccess);
            assert_eq!(remembered.sources, ["__execution"]);
            remembered.access_mode = crate::permissions::AccessMode::Ask;
            store.save_remembered_tools(&remembered).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.remembered_tools().unwrap().unwrap().access_mode, crate::permissions::AccessMode::Ask);
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
                        interaction_mode: crate::permissions::InteractionMode::Act,
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
            access_mode: crate::permissions::AccessMode::Ask,
            interaction_mode: crate::permissions::InteractionMode::Act,
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
                supports_images: false, tool_support: crate::providers::ToolSupport::Supported,
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
            assert!(!store.providers().unwrap().iter().any(|p| p.id == provider.id));
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
    fn adding_the_same_todo_does_not_create_duplicate_work() {
        let store = Store::open_memory().unwrap();
        let id = store.create_conversation().unwrap().id;
        let first = store.add_todo(&id, "Read the paper").unwrap();
        let again = store.add_todo(&id, "  Read   the paper  ").unwrap();
        assert_eq!(first.text, again.text);
        assert_eq!(store.todos(&id).unwrap().len(), 1);
    }
    #[test]
    fn failed_todo_save_preserves_the_previous_list() {
        let store = Store::open_memory().unwrap();
        let id = store.create_conversation().unwrap().id;
        store.add_todo(&id, "Keep this task").unwrap();
        store.connection.execute_batch("CREATE TRIGGER reject_todo BEFORE INSERT ON todos WHEN NEW.text='fail' BEGIN SELECT RAISE(ABORT, 'fail'); END;").unwrap();
        let replacement = vec![
            crate::plans::Todo { text: "New task".into(), status: crate::plans::PENDING.into(), updated_at: now() },
            crate::plans::Todo { text: "fail".into(), status: crate::plans::PENDING.into(), updated_at: now() },
        ];
        assert!(store.save_todos(&id, &replacement).is_err());
        assert_eq!(store.todos(&id).unwrap()[0].text, "Keep this task");
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
        store.recover_interrupted_runs().unwrap();
        let messages = store.messages(&id).unwrap();
        assert_eq!(messages[0].status, "interrupted");
        assert_eq!(messages[0].content, "Partial response");
    }

    #[test]
    fn prompt_freeze_round_trips_and_copies_on_fork() {
        let store = Store::open_memory().unwrap();
        let conv = store.create_conversation().unwrap();
        let freeze = PromptFreeze {
            conversation_id: conv.id.clone(),
            selection_hash: "abc".into(),
            tools_json: "[{\"type\":\"function\"}]".into(),
            aliases_json: "[\"todo_write\"]".into(),
            created_at: 1,
        };
        store.save_prompt_freeze(&freeze).unwrap();
        assert_eq!(store.prompt_freeze(&conv.id).unwrap().unwrap().selection_hash, "abc");
        let user = store.append_message(&conv.id, "user", "hi", "complete").unwrap();
        let forked = store.fork_conversation(&conv.id, &user.id).unwrap();
        assert_eq!(store.prompt_freeze(&forked.id).unwrap().unwrap().aliases_json, "[\"todo_write\"]");
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

    #[test]
    fn audit_secondary_connection_preserves_live_run_and_reply() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("live.sqlite");
        let store = Store::open(&path).unwrap();
        let conversation = store.create_conversation().unwrap();
        let reply = store.append_message(&conversation.id, "assistant", "Working", "streaming").unwrap();
        store.save_run(&crate::agent_run::RunRecord {
            id: "live-run".into(), conversation_id: conversation.id.clone(),
            status: crate::agent_run::RunState::ExecutingTools,
            model_provider: None, model_id: None, checkpoint: None, error: None,
            created_at: now(), updated_at: now(),
        }).unwrap();
        let second = Store::open(&path).unwrap();
        let messages = second.messages(&conversation.id).unwrap();
        assert_eq!(messages[0].id, reply.id);
        assert_eq!(messages[0].status, "streaming");
        assert!(messages[0].error.is_none());
        let status: String = second.connection.query_row("SELECT status FROM runs WHERE id='live-run'", [], |row| row.get(0)).unwrap();
        assert_eq!(status, "executing_tools");
    }

    #[test]
    fn session_events_monotonic_sequence_and_forking() {
        let store = Store::open_memory().unwrap();
        let conv = store.create_conversation().unwrap();
        let run = crate::agent_run::RunRecord {
            id: "run-1".into(),
            conversation_id: conv.id.clone(),
            status: crate::agent_run::RunState::Completed,
            model_provider: None,
            model_id: None,
            checkpoint: None,
            error: None,
            created_at: 1000,
            updated_at: 1000,
        };
        store.save_run(&run).unwrap();

        // 1. Monotonic sequence auto-assignment when seq = 0
        let ev1 = crate::sessions::SessionEvent {
            id: "ev-1".into(),
            conversation_id: conv.id.clone(),
            run_id: Some("run-1".into()),
            seq: 0,
            step_id: Some("turn_start".into()),
            tool_call_id: None,
            event_type: "user_msg".into(),
            payload: serde_json::json!({"content": "Hello world"}),
            ignorable: false,
            created_at: 1000,
        };
        let seq1 = store.append_session_event(&ev1).unwrap();
        assert_eq!(seq1, 1);

        let ev2 = crate::sessions::SessionEvent {
            id: "ev-2".into(),
            conversation_id: conv.id.clone(),
            run_id: Some("run-1".into()),
            seq: 0,
            step_id: Some("step-0".into()),
            tool_call_id: Some("call-1".into()),
            event_type: "tool_call".into(),
            payload: serde_json::json!({"name": "system_time"}),
            ignorable: false,
            created_at: 2000,
        };
        let seq2 = store.append_session_event(&ev2).unwrap();
        assert_eq!(seq2, 2);

        let ev3 = crate::sessions::SessionEvent {
            id: "ev-3".into(),
            conversation_id: conv.id.clone(),
            run_id: Some("run-1".into()),
            seq: 0,
            step_id: Some("step-0".into()),
            tool_call_id: Some("call-1".into()),
            event_type: "tool_result".into(),
            payload: serde_json::json!({"result": "12:00"}),
            ignorable: false,
            created_at: 3000,
        };
        let seq3 = store.append_session_event(&ev3).unwrap();
        assert_eq!(seq3, 3);

        // Fetch events
        let events = store.session_events(&conv.id, None, None).unwrap();
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].seq, 1);
        assert_eq!(events[1].seq, 2);
        assert_eq!(events[2].seq, 3);

        // 2. Fork session up to seq 2
        let (forked, copied_count) = store.fork_session_events(&conv.id, 2).unwrap();
        assert_eq!(copied_count, 2);
        assert_ne!(forked.id, conv.id);

        let forked_events = store.session_events(&forked.id, None, None).unwrap();
        assert_eq!(forked_events.len(), 2);
        assert_eq!(forked_events[0].seq, 1);
        assert_eq!(forked_events[0].event_type, "user_msg");
        assert_eq!(forked_events[1].seq, 2);
        assert_eq!(forked_events[1].event_type, "tool_call");

        // Appending to forked conversation continues sequence from max(seq)
        let ev4 = crate::sessions::SessionEvent {
            id: "ev-4".into(),
            conversation_id: forked.id.clone(),
            run_id: None,
            seq: 0,
            step_id: Some("step-1".into()),
            tool_call_id: None,
            event_type: "user_msg".into(),
            payload: serde_json::json!({"content": "Followup in fork"}),
            ignorable: false,
            created_at: 4000,
        };
        let seq4 = store.append_session_event(&ev4).unwrap();
        assert_eq!(seq4, 3);
    }

    #[test]
    fn audit_fork_excludes_later_turns_and_unseen_assistant_output() {
        let store = Store::open_memory().unwrap();
        let conv = store.create_conversation().unwrap();
        for (role, content) in [("user", "First request"), ("assistant", "First answer"), ("user", "Future request"), ("assistant", "Future answer")] {
            store.append_message(&conv.id, role, content, "complete").unwrap();
        }
        for (seq, event_type, payload) in [
            (1, "user_msg", serde_json::json!({"role":"user","content":"First request"})),
            (2, "system_prompt", serde_json::json!({"content":"System"})),
            (3, "step_start", serde_json::json!({"round":0})),
            (4, "model_response", serde_json::json!({"content":"First answer","final":true})),
            (5, "step_end", serde_json::json!({"content":"First answer"})),
            (6, "turn_end", serde_json::json!({"status":"complete"})),
            (7, "user_msg", serde_json::json!({"content":"Future request"})),
            (8, "model_response", serde_json::json!({"content":"Future answer","final":true})),
        ] {
            store.append_session_event(&crate::sessions::SessionEvent {
                id: format!("audit-{seq}"), conversation_id: conv.id.clone(), run_id: None,
                seq, step_id: None, tool_call_id: None, event_type: event_type.into(),
                payload, ignorable: false, created_at: 1000,
            }).unwrap();
        }
        let (partial, _) = store.fork_session_events(&conv.id, 3).unwrap();
        let messages = store.messages(&partial.id).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].content, "First request");
        let (complete, _) = store.fork_session_events(&conv.id, 6).unwrap();
        let messages = store.messages(&complete.id).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].content, "First answer");
    }

    #[test]
    fn audit_large_fork_is_complete_and_failed_copy_rolls_back() {
        let store = Store::open_memory().unwrap();
        let conv = store.create_conversation().unwrap();
        let transaction = store.connection.unchecked_transaction().unwrap();
        for seq in 1..=5001 {
            store.append_session_event(&crate::sessions::SessionEvent {
                id: format!("large-{seq}"), conversation_id: conv.id.clone(), run_id: None,
                seq, step_id: None, tool_call_id: None, event_type: "note".into(),
                payload: serde_json::json!({}), ignorable: true, created_at: 1000,
            }).unwrap();
        }
        transaction.commit().unwrap();
        let (forked, copied) = store.fork_session_events(&conv.id, 5001).unwrap();
        assert_eq!(copied, 5001);
        assert_eq!(store.session_events(&forked.id, Some(5001), None).unwrap()[0].seq, 5001);
        let count = || store.connection.query_row("SELECT COUNT(*) FROM conversations", [], |row| row.get::<_, i64>(0)).unwrap();
        let before = count();
        store.connection.execute_batch("CREATE TRIGGER reject_fork BEFORE INSERT ON session_events BEGIN SELECT RAISE(ABORT, 'injected copy failure'); END;").unwrap();
        assert!(store.fork_session_events(&conv.id, 5001).is_err());
        assert_eq!(count(), before);
    }

    #[test]
    fn audit_finish_turn_is_atomic_and_survives_restart() {
        use crate::{agent_run::{RunRecord, RunState}, research_tasks::{ResearchTask, TaskBudgets, TaskStatus}};
        let store = Store::open_memory().unwrap();
        let conv = store.create_conversation().unwrap();
        let mut message = store.append_message(&conv.id, "assistant", "", "streaming").unwrap();
        let mut task = ResearchTask::new(&conv.id, "Research", TaskBudgets::default()).unwrap();
        store.save_research_task(&task).unwrap();
        let mut run = RunRecord { id: "atomic-turn".into(), conversation_id: conv.id.clone(), status: RunState::Generating, model_provider: None, model_id: None, checkpoint: None, error: None, created_at: now(), updated_at: now() };
        store.save_run(&run).unwrap();
        message.status = "complete".into(); message.content = "Finished".into();
        run.status = RunState::Completed; task.finish_turn("complete");
        store.connection.execute_batch("CREATE TRIGGER reject_finish BEFORE UPDATE ON research_tasks BEGIN SELECT RAISE(ABORT, 'injected task failure'); END;").unwrap();
        assert!(store.finish_turn(&message, &run, Some(&task)).is_err());
        assert_eq!(store.messages(&conv.id).unwrap()[0].status, "streaming");
        store.connection.execute_batch("DROP TRIGGER reject_finish;").unwrap();
        store.finish_turn(&message, &run, Some(&task)).unwrap();
        store.recover_interrupted_runs().unwrap();
        crate::research_tasks::recover_interrupted(&store).unwrap();
        assert_eq!(store.messages(&conv.id).unwrap()[0].status, "complete");
        assert_eq!(store.research_task(&task.id).unwrap().unwrap().status, TaskStatus::Completed);
    }

    #[test]
    fn audit_failed_resume_does_not_consume_the_paused_checkpoint() {
        use crate::research_tasks::{ResearchTask, TaskBudgets, TaskStatus};
        let store = Store::open_memory().unwrap();
        let conv = store.create_conversation().unwrap();
        let mut task = ResearchTask::new(&conv.id, "Research", TaskBudgets::default()).unwrap();
        task.pause().unwrap(); store.save_research_task(&task).unwrap();
        task.resume().unwrap();
        store.connection.execute_batch("CREATE TRIGGER reject_resume BEFORE INSERT ON settings WHEN NEW.key LIKE 'pending_resume%' BEGIN SELECT RAISE(ABORT, 'injected checkpoint failure'); END;").unwrap();
        assert!(store.stage_research_resume(&task, "Checkpoint").is_err());
        assert_eq!(store.research_task(&task.id).unwrap().unwrap().status, TaskStatus::Paused);
        let marker: String = store.setting(&format!("pending_resume_task:{}", conv.id)).unwrap();
        assert!(marker.is_empty());
    }

    #[test]
    fn forked_conversation_keeps_message_history_and_run_lineage() {
        let store = Store::open_memory().unwrap();
        let conv = store.create_conversation().unwrap();
        store.append_message(&conv.id, "user", "Plan the release", "complete").unwrap();
        store.append_message(&conv.id, "assistant", "Here is the plan", "complete").unwrap();
        // Real run row so session_events.run_id FK lineage is preserved.
        let now = now();
        store.save_run(&crate::agent_run::RunRecord {
            id: "run-1".into(), conversation_id: conv.id.clone(),
            status: crate::agent_run::RunState::Completed,
            model_provider: None, model_id: None, checkpoint: None,
            error: None, created_at: now, updated_at: now,
        }).unwrap();
        for (seq, event_type) in [(1u64, "user_msg"), (2u64, "assistant_msg")] {
            store.append_session_event(&crate::sessions::SessionEvent {
                id: format!("ev-{seq}"),
                conversation_id: conv.id.clone(),
                run_id: Some("run-1".into()),
                seq,
                step_id: Some("step-0".into()),
                tool_call_id: None,
                event_type: event_type.into(),
                payload: serde_json::json!({}),
                ignorable: false,
                created_at: 1000,
            }).unwrap();
        }
        let (forked, _) = store.fork_session_events(&conv.id, 2).unwrap();
        let messages = store.messages(&forked.id).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        let history = crate::history::model_history_with_cutoff(&messages, None).unwrap();
        assert!(history.iter().any(|m| m["role"] == "user"));
        let events = store.session_events(&forked.id, None, None).unwrap();
        assert!(events.iter().all(|e| e.run_id.as_deref() == Some("run-1")));
    }

    #[test]
    fn search_sessions_finds_matching_message_excerpts() {
        let store = Store::open_memory().unwrap();
        let conv = store.create_conversation().unwrap();
        store.rename_conversation(&conv.id, "Knowledge query").unwrap();
        store.append_message(&conv.id, "user", "How does quantum computing work?", "complete").unwrap();
        store.append_message(&conv.id, "assistant", "Quantum computers leverage superposition and entanglement.", "complete").unwrap();

        let hits = store.search_sessions("superposition", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].conversation_id, conv.id);
        assert_eq!(hits[0].role, "assistant");
        assert!(hits[0].excerpt.contains("superposition"));

        let no_hits = store.search_sessions("nonexistent_needle_xyz", 10).unwrap();
        assert!(no_hits.is_empty());
    }

    #[test]
    fn startup_repair_closes_a_crash_truncated_turn_tail_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("repair.sqlite");
        let conversation_id;
        {
            let store = Store::open(&path).unwrap();
            let conversation = store.create_conversation().unwrap();
            conversation_id = conversation.id;
            store
                .save_run(&crate::agent_run::RunRecord {
                    id: "run-1".into(),
                    conversation_id: conversation_id.clone(),
                    status: crate::agent_run::RunState::ExecutingTools,
                    model_provider: None,
                    model_id: None,
                    checkpoint: None,
                    error: None,
                    created_at: 1000,
                    updated_at: 1000,
                })
                .unwrap();
            // Open tail: turn_start + step_start + one answered and one
            // unanswered tool_call, then nothing — the app stopped mid-turn.
            for (event_type, step_id, call_id, payload) in [
                ("turn_start", None, None, serde_json::json!({"model": "m"})),
                ("step_start", Some("step-0"), None, serde_json::json!({"round": 0})),
                ("tool_call", Some("step-0"), Some("call-done"), serde_json::json!({"callId": "call-done", "name": "file_read"})),
                ("tool_result", Some("step-0"), Some("call-done"), serde_json::json!({"callId": "call-done", "name": "file_read", "result": "ok"})),
                ("tool_call", Some("step-0"), Some("call-open"), serde_json::json!({"callId": "call-open", "name": "file_write"})),
            ] {
                store
                    .append_session_event(&crate::sessions::SessionEvent {
                        id: uuid::Uuid::new_v4().to_string(),
                        conversation_id: conversation_id.clone(),
                        run_id: Some("run-1".into()),
                        seq: 0,
                        step_id: step_id.map(str::to_string),
                        tool_call_id: call_id.map(str::to_string),
                        event_type: event_type.into(),
                        payload,
                        ignorable: false,
                        created_at: 1000,
                    })
                    .unwrap();
            }
            // A tool audit whose request was recorded but never got a result.
            store
                .connection
                .execute(
                    "INSERT INTO messages(id,conversation_id,role,content,status,created_at) VALUES('audit-open',?1,'tool','{\"request\":{\"name\":\"file_write\",\"arguments\":{},\"callId\":\"call-open\"}}','streaming',1000)",
                    [&conversation_id],
                )
                .unwrap();
            store
                .connection
                .execute(
                    "INSERT INTO messages(id,conversation_id,role,content,status,created_at) VALUES('reply-open',?1,'assistant','','streaming',2000)",
                    [&conversation_id],
                )
                .unwrap();
        }
        let store = Store::open(&path).unwrap();
        store.recover_interrupted_runs().unwrap();
        let reply = store.messages(&conversation_id).unwrap().remove(1);
        assert_eq!(reply.status, "interrupted");
        assert!(reply.error.as_deref().unwrap_or("").contains("app closed"));
        // Startup recovery flips the stale run to outcome_unknown and the in-flight
        // audit row to interrupted, which replays as an unknown outcome.
        assert_eq!(
            store.run("run-1").unwrap().unwrap().status,
            crate::agent_run::RunState::OutcomeUnknown
        );
        let audit = store.messages(&conversation_id).unwrap().remove(0);
        assert_eq!(audit.status, "interrupted");
        let history = crate::history::model_history_with_cutoff(&[audit], None).unwrap();
        assert!(history[1]["content"]
            .as_str()
            .unwrap()
            .contains("may already have changed data"));

        let before = store.session_events(&conversation_id, None, None).unwrap().len();
        assert_eq!(store.repair_open_turn_tails().unwrap(), 3);
        let events = store.session_events(&conversation_id, None, None).unwrap();
        assert_eq!(events.len(), before + 3);
        let result = events
            .iter()
            .find(|e| {
                e.event_type == "tool_result"
                    && e.tool_call_id.as_deref() == Some("call-open")
            })
            .unwrap();
        assert_eq!(result.payload["decision"], "unknown");
        assert_eq!(result.payload["result"]["outcome"], "unknown");
        assert_eq!(
            result.payload["result"]["message"],
            "The app stopped before this action's result was recorded. It may already have changed data. Do not automatically repeat it."
        );
        assert_eq!(result.run_id.as_deref(), Some("run-1"));
        assert_eq!(result.step_id.as_deref(), Some("step-0"));
        let step_end = events.iter().find(|e| e.event_type == "step_end").unwrap();
        assert_eq!(step_end.payload, serde_json::json!({"interrupted": true}));
        assert_eq!(step_end.step_id.as_deref(), Some("step-0"));
        let turn_end = events.iter().find(|e| e.event_type == "turn_end").unwrap();
        assert_eq!(turn_end.payload, serde_json::json!({"status": "interrupted"}));
        // Synthetic events keep the per-conversation sequence monotonic.
        let seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
        assert!(seqs.windows(2).all(|w| w[0] < w[1]));
        // Idempotent: a second pass and a later restart append nothing.
        assert_eq!(store.repair_open_turn_tails().unwrap(), 0);
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(store.repair_open_turn_tails().unwrap(), 0);
    }

    #[test]
    fn startup_repair_pairs_a_call_left_open_inside_a_completed_turn() {
        let store = Store::open_memory().unwrap();
        let conversation = store.create_conversation().unwrap();
        store.save_run(&crate::agent_run::RunRecord {
            id: "run-complete".into(), conversation_id: conversation.id.clone(),
            status: crate::agent_run::RunState::Completed, model_provider: None, model_id: None,
            checkpoint: None, error: None, created_at: 1000, updated_at: 1000,
        }).unwrap();
        for (event_type, call_id) in [
            ("turn_start", None), ("tool_call", Some("call-orphan")), ("turn_end", None),
            ("turn_start", None), ("tool_call", Some("call-late")), ("turn_end", None),
            ("tool_result", Some("call-late")),
        ] {
            store.append_session_event(&crate::sessions::SessionEvent {
                id: uuid::Uuid::new_v4().to_string(), conversation_id: conversation.id.clone(),
                run_id: Some("run-complete".into()), seq: 0, step_id: Some("step-0".into()),
                tool_call_id: call_id.map(str::to_string), event_type: event_type.into(),
                payload: serde_json::json!({"callId":call_id,"name":"file_write"}),
                ignorable: false, created_at: 1000,
            }).unwrap();
        }
        assert_eq!(store.repair_open_turn_tails().unwrap(), 1);
        let events = store.session_events(&conversation.id, None, None).unwrap();
        let result = events.last().unwrap();
        assert_eq!(result.event_type, "tool_result");
        assert_eq!(result.tool_call_id.as_deref(), Some("call-orphan"));
        assert_eq!(result.payload["result"]["outcome"], "unknown");
        assert_eq!(store.run("run-complete").unwrap().unwrap().status, crate::agent_run::RunState::OutcomeUnknown);
        assert_eq!(store.repair_open_turn_tails().unwrap(), 0);
    }

    #[test]
    fn session_event_types_and_format_version_are_enforced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("versioned.sqlite");
        {
            let store = Store::open(&path).unwrap();
            // Open stamps the current format version.
            let stamped: u32 = store.setting("session_format_version").unwrap();
            assert_eq!(stamped, crate::sessions::SESSION_FORMAT_VERSION);
            let conv = store.create_conversation().unwrap();
            let event = |kind: &str, ignorable: bool| crate::sessions::SessionEvent {
                id: uuid::Uuid::new_v4().to_string(),
                conversation_id: conv.id.clone(),
                run_id: None,
                seq: 0,
                step_id: None,
                tool_call_id: None,
                event_type: kind.into(),
                payload: serde_json::json!({}),
                ignorable,
                created_at: 1000,
            };
            // Unknown non-ignorable types are rejected at write time.
            assert!(store.append_session_event(&event("brand_new_typo", false)).is_err());
            // Ignorable unknowns stay tolerated for forward compatibility.
            store.append_session_event(&event("brand_new_typo", true)).unwrap();
            assert_eq!(store.session_events(&conv.id, None, None).unwrap().len(), 1);
            // A non-ignorable unknown row that bypassed validation fails reads.
            store
                .connection
                .execute(
                    "INSERT INTO session_events(id,conversation_id,seq,event_type,payload,ignorable,created_at) VALUES('mystery',?1,99,'future_type','{}',0,1)",
                    [&conv.id],
                )
                .unwrap();
            assert!(store.session_events(&conv.id, None, None).is_err());
            store
                .save_setting(
                    "session_format_version",
                    &(crate::sessions::SESSION_FORMAT_VERSION + 1),
                )
                .unwrap();
        }
        match Store::open(&path) {
            Ok(_) => panic!("a newer session format must not open"),
            Err(error) => assert!(error.contains("newer version"), "{error}"),
        }
    }

    #[test]
    fn skill_activation_is_per_conversation_and_inherits_global_until_customized() {
        let store = Store::open_memory().unwrap();
        let a = store.create_conversation().unwrap();
        let b = store.create_conversation().unwrap();
        store.save_active_skills(&["wiki-qa".into()]).unwrap();
        // Uncustomized conversations (and unsaved/empty ids) inherit the global set.
        assert_eq!(store.active_skills_for(&a.id).unwrap(), vec!["wiki-qa"]);
        assert_eq!(store.active_skills_for("").unwrap(), vec!["wiki-qa"]);
        // Customizing A leaves B on the inherited global set.
        store.save_conversation_skills(&a.id, &["sentry".into()]).unwrap();
        assert_eq!(store.active_skills_for(&a.id).unwrap(), vec!["sentry"]);
        assert_eq!(store.active_skills_for(&b.id).unwrap(), vec!["wiki-qa"]);
        // An explicitly empty set stays empty rather than re-inheriting.
        store.save_conversation_skills(&a.id, &[]).unwrap();
        assert!(store.active_skills_for(&a.id).unwrap().is_empty());
        // Removing a package scrubs it from every conversation's own set.
        store
            .save_conversation_skills(&b.id, &["sentry".into(), "wiki-qa".into()])
            .unwrap();
        store.forget_conversation_skill("sentry").unwrap();
        assert_eq!(store.active_skills_for(&b.id).unwrap(), vec!["wiki-qa"]);
        // Deleting a conversation cascades its customization row.
        store.delete_conversation(&a.id).unwrap();
        assert_eq!(
            store
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM conversation_skills WHERE conversation_id=?1",
                    [&a.id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn legacy_fact_rows_normalize_scopes_and_origins_without_widening() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("app.db");
        let work = dir.path().join("work-a");
        std::fs::create_dir_all(&work).unwrap();
        {
            let store = Store::open(&db).unwrap();
            // Rows as written by the pre-scoped schema: 'global', raw paths,
            // legacy origins.
            for (scope, origin) in [("global", "ui"), (work.to_str().unwrap(), "model"), ("D:\\elsewhere", "model")] {
                store
                    .connection
                    .execute(
                        "INSERT INTO memory_facts(id, scope, fact, origin, created_at, updated_at) VALUES (?1,?2,'f',?3,0,0)",
                        params![uuid::Uuid::new_v4().to_string(), scope, origin],
                    )
                    .unwrap();
            }
            // A project exists at `work` so its path maps to the id form.
            store
                .save_setting(
                    "workspace_index_v1",
                    &serde_json::json!({
                        "projects": [{"id": "pa", "name": "A", "path": work.to_string_lossy()}],
                        "tasks": {},
                    }),
                )
                .unwrap();
            store.migrate_fact_scopes().unwrap();
            let rows: Vec<(String, String)> = store
                .connection
                .prepare("SELECT scope, origin FROM memory_facts ORDER BY scope")
                .unwrap()
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<std::result::Result<_, _>>()
                .unwrap();
            assert!(rows.contains(&("user".to_string(), "user".to_string())), "{rows:?}");
            assert!(rows.contains(&("project:pa".to_string(), "agent".to_string())), "{rows:?}");
            assert!(
                rows.contains(&(format!("project:{}", "D:\\elsewhere"), "agent".to_string())),
                "{rows:?}"
            );
        }
        // Reopening is additive-safe: columns exist, rows keep their scopes.
        let reopened = Store::open(&db).unwrap();
        assert_eq!(reopened.recall_facts("user", 10).unwrap().len(), 1);
    }
}
