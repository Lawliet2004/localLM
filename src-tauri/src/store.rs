use crate::providers::{ModelSelection, ProviderConnection, RemoteModel};
use crate::runtime_config::RuntimeConfig;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

type Result<T> = std::result::Result<T, String>;
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
    pub metrics: Option<crate::inference::ResponseMetrics>,
    pub error: Option<String>,
    pub id: String,
    pub conversation_id: String,
    pub role: String,
    pub content: String,
    pub reasoning: String,
    pub status: String,
    pub created_at: i64,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningMode {
    #[default]
    Auto,
    Quick,
    Think,
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
    #[serde(default)]
    pub reasoning_mode: ReasoningMode,
    /// Teach the model to emit `openui` fenced blocks that the chat renders
    /// as structured UI. Frozen into the system prompt when enabled.
    #[serde(default)]
    pub generative_ui: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self { runtime_path: String::new(), model_path: String::new(), projector_path: String::new(), temperature: 1.0,
            top_p: 0.95, max_tokens: 2048, reasoning_mode: ReasoningMode::Auto, generative_ui: false, system_prompt: "You are a helpful local assistant. Be clear and accurate. If you do not know something, say so.".into() }
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
        self.validate_generation_controls()?;
        if !(1..=32768).contains(&self.max_tokens) {
            return Err("Maximum response length must be between 1 and 32768 tokens.".into());
        }
        Ok(())
    }

    /// Shared controls also apply to API requests, whose output budget is
    /// selected from remote model metadata rather than local runtime settings.
    pub fn validate_generation_controls(&self) -> Result<()> {
        if !self.temperature.is_finite() || !(0.0..=2.0).contains(&self.temperature) {
            return Err("Temperature must be between 0 and 2.".into());
        }
        if !self.top_p.is_finite() || !(0.01..=1.0).contains(&self.top_p) {
            return Err("Top-p must be between 0.01 and 1.".into());
        }
        if self.system_prompt.len() > 32768 {
            return Err("System prompt is too long.".into());
        }
        if self.runtime_path.len() > 32768
            || self.model_path.len() > 32768
            || self.projector_path.len() > 32768
        {
            return Err("File path is too long.".into());
        }
        Ok(())
    }
}

pub struct Store {
    pub(crate) connection: Connection,
    /// Directory that holds the database file. Private snapshot blobs live in
    /// `<data dir>/snapshots`, next to the database and never in a workspace.
    data_dir: PathBuf,
    /// Keeps the scratch directory of an in-memory store alive for as long as
    /// the store exists. It is only held, never read.
    #[allow(dead_code)]
    scratch: Option<tempfile::TempDir>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationTools {
    #[serde(default)]
    pub access_mode: crate::permissions::AccessMode,
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
            "__workspace" => crate::workspace::TOOL_COUNT,
            "__execution" => 1,
            "__daytona" => 1,
            "__browser" => crate::automation::BROWSER_TOOL_COUNT,
            "__computer" => crate::automation::COMPUTER_TOOL_COUNT,
            _ => return Err("Unknown local tool source.".into()),
        };
    }
    if count > crate::tool_discovery::MAX_CATALOG {
        return Err(
            "At most4096 tools can be selected; tool discovery keeps each request bounded.".into(),
        );
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
    /// Open the live database.
    ///
    /// This constructor performs **schema setup only**. It never runs crash
    /// recovery, because recovery is a startup action, not a connection
    /// action: recovery marks every `streaming` message as interrupted and
    /// every live run as cancelled, and running it on a second connection
    /// would flip the audit row of a tool that is executing *right now* out of
    /// `'streaming'`. That defeats the `OLD.status!='streaming'` guard on the
    /// `native_tool_round_edit` trigger, which then deletes the native
    /// provider context anchored on that row mid-turn.
    ///
    /// The authoritative connection is recovered exactly once at startup via
    /// [`Store::recover_interrupted`]. Every other connection to the same
    /// file is safe to open at any time.
    pub fn open(path: &Path) -> Result<Self> {
        let data_dir = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .map(|parent| parent.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        Self::initialize(Connection::open(path).map_err(db_error)?, data_dir, None)
    }
    #[cfg(test)]
    pub(crate) fn open_memory() -> Result<Self> {
        // An in-memory database still owns a private directory so private
        // sidecar storage (snapshot blobs) has a home that is cleaned up with
        // the store.
        let scratch = tempfile::tempdir().map_err(|error| error.to_string())?;
        let data_dir = scratch.path().to_path_buf();
        Self::initialize(
            Connection::open_in_memory().map_err(db_error)?,
            data_dir,
            Some(scratch),
        )
    }
    /// Directory that holds the database file. Private snapshot blobs are
    /// stored in `<data dir>/snapshots`.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
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
            metrics: None,
            error: None,
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
    /// The schema version this build expects.
    ///
    /// The version lives in SQLite's `PRAGMA user_version`, so the store can
    /// tell which additive steps have already run and can refuse to open a
    /// database written by a **newer** build instead of corrupting it.
    const SCHEMA_VERSION: u32 = 1;

    /// The schema version recorded in the database.
    fn schema_version(connection: &Connection) -> Result<u32> {
        let version: i64 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .map_err(db_error)?;
        Ok(version.max(0) as u32)
    }

    /// Record the schema version after every additive step has succeeded.
    fn set_schema_version(connection: &Connection, version: u32) -> Result<()> {
        connection
            .execute_batch(&format!("PRAGMA user_version = {version}"))
            .map_err(db_error)?;
        Ok(())
    }

    fn initialize(
        connection: Connection,
        data_dir: PathBuf,
        scratch: Option<tempfile::TempDir>,
    ) -> Result<Self> {
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(db_error)?;
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS conversations(id TEXT PRIMARY KEY, title TEXT NOT NULL, updated_at INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS messages(id TEXT PRIMARY KEY, conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                role TEXT NOT NULL CHECK(role IN ('user','assistant','tool')), content TEXT NOT NULL, reasoning TEXT NOT NULL DEFAULT '',
                status TEXT NOT NULL CHECK(status IN ('complete','streaming','interrupted','error')), created_at INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS messages_conversation ON messages(conversation_id,created_at);
            CREATE TABLE IF NOT EXISTS model_message_contents(
                message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
                content_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS native_tool_rounds(
                message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
                content_json TEXT NOT NULL
            );
            CREATE TRIGGER IF NOT EXISTS native_tool_round_edit AFTER UPDATE OF content ON messages
            WHEN OLD.content!=NEW.content AND OLD.status!='streaming' BEGIN
                DELETE FROM native_tool_rounds WHERE message_id=NEW.id;
            END;
            CREATE TABLE IF NOT EXISTS message_inference(
                message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
                metrics_json TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS conversation_model_history(
                conversation_id TEXT PRIMARY KEY REFERENCES conversations(id) ON DELETE CASCADE,
                through_message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
                checkpoint_id TEXT,
                messages_json TEXT NOT NULL
            );
            CREATE TRIGGER IF NOT EXISTS invalidate_model_history_on_message_update
                AFTER UPDATE OF content,reasoning,status ON messages BEGIN
                DELETE FROM conversation_model_history WHERE conversation_id=NEW.conversation_id;
                DELETE FROM model_message_contents WHERE message_id=NEW.id AND OLD.content!=NEW.content;
            END;
            CREATE TRIGGER IF NOT EXISTS invalidate_model_history_on_message_delete
                AFTER DELETE ON messages BEGIN
                DELETE FROM conversation_model_history WHERE conversation_id=OLD.conversation_id;
            END;
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
            CREATE TABLE IF NOT EXISTS run_checkpoints(
                id TEXT PRIMARY KEY REFERENCES runs(id) ON DELETE CASCADE,
                payload TEXT NOT NULL, updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS process_results(
                id TEXT PRIMARY KEY, owner TEXT NOT NULL, workspace TEXT NOT NULL,
                snapshot TEXT NOT NULL, updated_at INTEGER NOT NULL
            );
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
            CREATE TABLE IF NOT EXISTS findings(
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                file TEXT NOT NULL,
                line INTEGER,
                summary TEXT NOT NULL,
                failure_scenario TEXT NOT NULL,
                category TEXT,
                state TEXT NOT NULL,
                note TEXT,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS findings_conversation ON findings(conversation_id, created_at);
            CREATE TABLE IF NOT EXISTS tasks(
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                title TEXT NOT NULL,
                detail TEXT,
                owner TEXT,
                state TEXT NOT NULL,
                note TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS task_dependencies(
                task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                depends_on TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
                PRIMARY KEY(task_id, depends_on)
            );
            CREATE INDEX IF NOT EXISTS tasks_conversation ON tasks(conversation_id, created_at);
            CREATE TABLE IF NOT EXISTS memory_facts(id TEXT PRIMARY KEY, scope TEXT NOT NULL, fact TEXT NOT NULL,
                origin TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS memory_facts_scope ON memory_facts(scope, updated_at);
            CREATE TABLE IF NOT EXISTS taste_signals(
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                tool TEXT NOT NULL,
                decision TEXT NOT NULL,
                arguments TEXT NOT NULL,
                result TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS taste_signals_conversation ON taste_signals(conversation_id, created_at);
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
            CREATE TABLE IF NOT EXISTS snapshots(
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                step INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS snapshots_conversation ON snapshots(conversation_id, created_at);
            CREATE TABLE IF NOT EXISTS snapshot_files(
                snapshot_id TEXT NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
                path TEXT NOT NULL,
                sha256 TEXT NOT NULL,
                size INTEGER NOT NULL,
                deleted INTEGER NOT NULL,
                PRIMARY KEY (snapshot_id, path)
            );
            CREATE TABLE IF NOT EXISTS staged_reverts(
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                snapshot_id TEXT NOT NULL REFERENCES snapshots(id) ON DELETE CASCADE,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS staged_reverts_conversation ON staged_reverts(conversation_id);
            CREATE INDEX IF NOT EXISTS session_events_conv_seq ON session_events(conversation_id, seq);
            CREATE INDEX IF NOT EXISTS session_events_run ON session_events(run_id);
            CREATE INDEX IF NOT EXISTS session_events_type ON session_events(event_type);").map_err(db_error)?;
        // A database written by a newer build must not be opened here: the
        // older schema would silently drop columns the newer build relies on.
        let version = Self::schema_version(&connection)?;
        if version > Self::SCHEMA_VERSION {
            return Err(format!(
                "This conversation database was written by a newer version of LocalLM (schema {version}, this build supports {}). Update the app, or start with a fresh data directory.",
                Self::SCHEMA_VERSION
            ));
        }
        let has_outcome: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('runs') WHERE name='outcome_json')", [], |row| row.get(0)
        ).map_err(db_error)?;
        if !has_outcome { connection.execute("ALTER TABLE runs ADD COLUMN outcome_json TEXT", []).map_err(db_error)?; }
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
                    .execute(
                        &format!("ALTER TABLE conversations ADD COLUMN {column} {definition}"),
                        [],
                    )
                    .map_err(db_error)?;
            }
        }
        crate::presentation::initialize(&connection)?;
        // Every additive step above has succeeded; record the version so the
        // next open knows what has already run.
        Self::set_schema_version(&connection, Self::SCHEMA_VERSION)?;
        let store = Self {
            connection,
            data_dir,
            scratch,
        };
        store.seed_remembered_tools()?;
        store.seed_subscription_providers()?;
        Self::migrate_historical_to_session_events(&store.connection)?;
        Ok(store)
    }
    /// One-shot crash recovery. Call this **once**, at startup, on the
    /// authoritative connection.
    ///
    /// It marks every message still flagged `streaming` as interrupted and
    /// every run still in a live state as cancelled — the state a crash
    /// leaves behind. It must not run on any later connection: a tool that is
    /// executing right now has a `streaming` audit row, and flipping it to
    /// `interrupted` defeats the `OLD.status!='streaming'` guard on the
    /// `native_tool_round_edit` trigger, which then deletes the native
    /// provider context anchored on that row.
    pub fn recover_interrupted(&self) -> Result<()> {
        let interrupted = {
            let mut query = self.connection.prepare("SELECT id,conversation_id,checkpoint FROM runs WHERE status IN ('preparing','generating','awaiting_approval','executing_tools','preparing_next_round')").map_err(db_error)?;
            let rows = query.query_map([], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,Option<String>>(2)?))).map_err(db_error)?;
            rows.collect::<std::result::Result<Vec<_>,_>>().map_err(db_error)?
        };
        let transaction = rusqlite::Transaction::new_unchecked(&self.connection, rusqlite::TransactionBehavior::Immediate).map_err(db_error)?;
        self.connection
            .execute_batch(
                "UPDATE messages SET status='interrupted' WHERE status='streaming';",
            )
            .map_err(db_error)?;
        for (id, conversation, checkpoint) in interrupted {
            let child = checkpoint.as_deref() == Some("subagent");
            let child_recoverable = child && crate::subagents::restore_child_checkpoint(self, &id).is_ok();
            let outcome = crate::agent_engine::RunOutcome { reason:crate::agent_engine::StopReason::AppRestart,
                resumable:!child || child_recoverable, checkpoint:if self.run_checkpoint(&id)?.is_some() {Some(id.clone())} else {checkpoint} };
            self.update_run_status(&id, if child && !child_recoverable { crate::agent_run::RunState::OutcomeUnknown } else { crate::agent_run::RunState::Paused },
                Some("Application restarted. Completed work is preserved; in-flight actions have unknown outcomes and were not repeated."), None)?;
            self.save_run_outcome(&id, &outcome)?;
            if child {
                self.connection.execute("UPDATE subagent_runs SET status='interrupted', error=?1, updated_at=?2 WHERE child_run_id=?3 AND status='running'",
                    params!["Application restarted; saved work requires an explicit resume.", now(), id]).map_err(db_error)?;
            }
            let seq = self.next_session_event_seq(&conversation)?;
            self.insert_session_event(&crate::sessions::SessionEvent {
                id:uuid::Uuid::new_v4().to_string(), conversation_id:conversation,
                run_id:Some(id), seq:0, step_id:None, tool_call_id:None,
                event_type:"turn_end".into(), payload:serde_json::json!({"status":"interrupted","outcome":outcome}),
                ignorable:false, created_at:now(),
            }, seq)?;
        }
        transaction.commit().map_err(db_error)?;
        crate::presentation_store::recover_interrupted(&self.connection)?;
        Ok(())
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
            .optional()
            .map_err(db_error)?;
        if let Some(text) = &existing {
            let value: serde_json::Value = serde_json::from_str(text)
                .map_err(|_| "Saved remembered tool selection is invalid.")?;
            if value.get("accessMode").is_some() {
                return Ok(());
            }
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
            remembered.access_mode = saved
                .as_deref()
                .and_then(|text| serde_json::from_str::<ConversationTools>(text).ok())
                .map(|tools| tools.access_mode)
                .unwrap_or_default();
            self.save_remembered_tools(&remembered)?;
        } else if let Some(text) = saved {
            if let Ok(settings) = serde_json::from_str::<ConversationTools>(&text) {
                let remembered = RememberedTools {
                    access_mode: settings.access_mode,
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
    pub fn save_run_checkpoint(&self, checkpoint:&crate::agent_engine::RunCheckpoint)->Result<()> {
        let payload=serde_json::to_string(checkpoint).map_err(|e|e.to_string())?;
        self.connection.execute("INSERT INTO run_checkpoints(id,payload,updated_at) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload,updated_at=excluded.updated_at",
            params![checkpoint.id,payload,now()]).map_err(db_error)?;
        Ok(())
    }
    pub fn run_checkpoint(&self,id:&str)->Result<Option<crate::agent_engine::RunCheckpoint>> {
        let payload:Option<String>=self.connection.query_row("SELECT payload FROM run_checkpoints WHERE id=?1",[id],|row|row.get(0)).optional().map_err(db_error)?;
        payload.map(|payload|serde_json::from_str(&payload).map_err(|e|e.to_string())).transpose()
    }
    pub fn save_run_outcome(&self, id: &str, outcome: &crate::agent_engine::RunOutcome) -> Result<()> {
        self.connection.execute("UPDATE runs SET outcome_json=?1 WHERE id=?2",
            params![serde_json::to_string(outcome).map_err(|e| e.to_string())?, id]).map_err(db_error)?;
        Ok(())
    }
    pub fn run(&self, id: &str) -> Result<Option<crate::agent_run::RunRecord>> {
        self.connection
            .query_row(
                "SELECT id, conversation_id, status, model_provider, model_id, checkpoint, error, created_at, updated_at, outcome_json
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
                    outcome: row.get::<_, Option<String>>(9)?.and_then(|text| serde_json::from_str(&text).ok()),
})
                },
            )
            .optional()
            .map_err(db_error)
    }
    pub fn latest_run(&self, conversation_id: &str) -> Result<Option<crate::agent_run::RunRecord>> {
        // Subagent child runs (checkpoint='subagent') never hijack the
        // user-visible active run, even while still executing.
        let mut latest = self.connection
            .query_row(
                "SELECT id, conversation_id, status, model_provider, model_id, checkpoint, error, created_at, updated_at, outcome_json
                 FROM runs WHERE conversation_id=?1
                 AND (checkpoint IS NULL OR checkpoint != 'subagent')
                 ORDER BY created_at DESC, rowid DESC LIMIT 1",
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
                    outcome: row.get::<_, Option<String>>(9)?.and_then(|text| serde_json::from_str(&text).ok()),
})
                },
            )
            .optional()
            .map_err(db_error)?;
        // Older builds represented a settled pause as preparing_next_round.
        // Only expose Resume when its durable task checkpoint still exists.
        if let Some(run) = latest.as_mut() {
            if run.status == crate::agent_run::RunState::PreparingNextRound
                && run.error.as_deref() == Some(crate::agent_run::pause_notice(false))
                && self
                    .recoverable_research_tasks(conversation_id)?
                    .iter()
                    .any(|task| task.status == crate::research_tasks::TaskStatus::Paused)
            {
                run.status = crate::agent_run::RunState::Paused;
            }
        }
        Ok(latest)
    }
    pub fn append_run_event(&self, event: &crate::agent_run::RunEvent) -> Result<()> {
        let payload = serde_json::to_string(&event.payload).map_err(|e| e.to_string())?;
        // Allocation and insertion are one SQLite write statement. Child
        // dispatch and interrupt delivery can use different connections.
        self.connection
            .execute(
                "INSERT INTO run_events(run_id, seq, step_id, tool_call_id, event_type, payload, created_at)
                 SELECT ?1, CASE WHEN ?2 > 0 THEN ?2 ELSE COALESCE(MAX(seq), -1) + 1 END, ?3, ?4, ?5, ?6, ?7 FROM run_events WHERE run_id=?1",
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
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }

    pub fn latest_run_event_seq(&self, run_id: &str) -> Result<Option<u64>> {
        self.connection.query_row("SELECT MAX(seq) FROM run_events WHERE run_id=?1", [run_id],
            |row| row.get::<_, Option<i64>>(0)).map(|seq| seq.map(|seq| seq as u64)).map_err(db_error)
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
        // Artifacts hold the **full** tool output, so an unbounded table grows
        // the database for the lifetime of the conversation. Evict the oldest
        // rows of this conversation once its share passes the budget.
        self.evict_artifacts_over_budget(&artifact.conversation_id)?;
        Ok(())
    }
    /// Total artifact bytes a single conversation may keep.
    const ARTIFACT_BUDGET_BYTES: i64 = 256 * 1024 * 1024;

    /// Drop the oldest artifacts of a conversation until it fits its budget.
    ///
    /// The newest artifact is never evicted: it is the one the model was just
    /// told to read back.
    fn evict_artifacts_over_budget(&self, conversation_id: &str) -> Result<()> {
        let total: i64 = self
            .connection
            .query_row(
                "SELECT COALESCE(SUM(size_bytes),0) FROM artifacts WHERE conversation_id=?1",
                [conversation_id],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        if total <= Self::ARTIFACT_BUDGET_BYTES {
            return Ok(());
        }
        // Evict the oldest artifacts until the conversation fits its budget.
        // The newest artifact is never evicted: it is the one the model was
        // just told to read back.
        let newest: Option<String> = self
            .connection
            .query_row(
                "SELECT id FROM artifacts WHERE conversation_id=?1
                 ORDER BY created_at DESC, rowid DESC LIMIT 1",
                [conversation_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        loop {
            let total: i64 = self
                .connection
                .query_row(
                    "SELECT COALESCE(SUM(size_bytes),0) FROM artifacts WHERE conversation_id=?1",
                    [conversation_id],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            if total <= Self::ARTIFACT_BUDGET_BYTES {
                break;
            }
            let removed = self
                .connection
                .execute(
                    "DELETE FROM artifacts WHERE id = (
                         SELECT id FROM artifacts
                         WHERE conversation_id=?1 AND id IS NOT ?2
                         ORDER BY created_at ASC, rowid ASC LIMIT 1
                     )",
                    params![conversation_id, newest],
                )
                .map_err(db_error)?;
            if removed == 0 {
                // Only the newest artifact is left, and it alone is over the
                // budget. Keep it: the transcript must stay readable.
                break;
            }
        }
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
                "SELECT id, conversation_id, status, model_provider, model_id, checkpoint, error, created_at, updated_at, outcome_json
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
                outcome: row.get::<_, Option<String>>(9)?.and_then(|text| serde_json::from_str(&text).ok()),
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
        // The replace is atomic: a failure part-way through the insert would
        // otherwise leave the conversation with a half-written checklist.
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(db_error)?;
        transaction
            .execute(
                "DELETE FROM todos WHERE conversation_id=?1",
                [conversation_id],
            )
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
        if todos.len() >= 50 {
            return Err("At most 50 todo items are kept per conversation.".into());
        }
        if text.trim().is_empty() || text.len() > 500 {
            return Err("Each todo needs 1-500 characters of text.".into());
        }
        let todo = crate::plans::Todo {
            text: text.to_string(),
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
        let todo = todos.get_mut(index).ok_or("Unknown todo index.")?;
        if let Some(status) = status {
            if ![
                crate::plans::PENDING,
                crate::plans::IN_PROGRESS,
                crate::plans::COMPLETED,
            ]
            .contains(&status)
            {
                return Err(format!(
                    "Unknown todo status '{status}'. Use pending, in_progress, or completed."
                ));
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
            .execute(
                "DELETE FROM goals WHERE conversation_id=?1",
                [conversation_id],
            )
            .map_err(db_error)?;
        Ok(())
    }
    /// Store one code-review finding. Findings are durable per conversation,
    /// so a later turn can read back the verdict the model handed over.
    pub fn insert_finding(&self, finding: &crate::findings::Finding) -> Result<()> {
        self.connection
            .execute(
                "INSERT INTO findings(id,conversation_id,file,line,summary,failure_scenario,category,state,note,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    finding.id,
                    finding.conversation_id,
                    finding.file,
                    finding.line,
                    finding.summary,
                    finding.failure_scenario,
                    finding.category,
                    finding.state,
                    finding.note,
                    finding.created_at
                ],
            )
            .map_err(db_error)?;
        Ok(())
    }
    /// The findings of one conversation, oldest first. A `None` state lists
    /// every finding; otherwise only the findings in that state.
    pub fn list_findings(
        &self,
        conversation_id: &str,
        state: Option<&str>,
    ) -> Result<Vec<crate::findings::Finding>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, conversation_id, file, line, summary, failure_scenario, category, state, note, created_at
                 FROM findings WHERE conversation_id=?1 AND (?2 IS NULL OR state=?2) ORDER BY created_at, rowid",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map(params![conversation_id, state], |row| {
                Ok(crate::findings::Finding {
                    id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    file: row.get(2)?,
                    line: row.get(3)?,
                    summary: row.get(4)?,
                    failure_scenario: row.get(5)?,
                    category: row.get(6)?,
                    state: row.get(7)?,
                    note: row.get(8)?,
                    created_at: row.get(9)?,
                })
            })
            .map_err(db_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    /// One finding of one conversation.
    pub fn finding(
        &self,
        conversation_id: &str,
        id: &str,
    ) -> Result<Option<crate::findings::Finding>> {
        self.connection
            .query_row(
                "SELECT id, conversation_id, file, line, summary, failure_scenario, category, state, note, created_at
                 FROM findings WHERE id=?1 AND conversation_id=?2",
                params![id, conversation_id],
                |row| {
                    Ok(crate::findings::Finding {
                        id: row.get(0)?,
                        conversation_id: row.get(1)?,
                        file: row.get(2)?,
                        line: row.get(3)?,
                        summary: row.get(4)?,
                        failure_scenario: row.get(5)?,
                        category: row.get(6)?,
                        state: row.get(7)?,
                        note: row.get(8)?,
                        created_at: row.get(9)?,
                    })
                },
            )
            .optional()
            .map_err(db_error)
    }
    /// Mark one finding resolved and record what was done about it.
    pub fn resolve_finding(
        &self,
        conversation_id: &str,
        id: &str,
        note: Option<&str>,
    ) -> Result<crate::findings::Finding> {
        // The row has to belong to this conversation: a finding of another
        // conversation is not addressable from here.
        self.finding(conversation_id, id)?
            .ok_or("Unknown finding for this conversation.")?;
        self.connection
            .execute(
                "UPDATE findings SET state='resolved', note=?2 WHERE id=?1 AND conversation_id=?3",
                params![id, note, conversation_id],
            )
            .map_err(db_error)?;
        self.finding(conversation_id, id)?
            .ok_or("Unknown finding for this conversation.".into())
    }
    /// Store one task of a conversation's task graph.
    pub fn insert_task(&self, task: &crate::tasks::Task) -> Result<()> {
        self.connection
            .execute(
                "INSERT INTO tasks(id,conversation_id,title,detail,owner,state,note,created_at,updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    task.id,
                    task.conversation_id,
                    task.title,
                    task.detail,
                    task.owner,
                    task.state,
                    task.note,
                    task.created_at,
                    task.updated_at
                ],
            )
            .map_err(db_error)?;
        Ok(())
    }
    /// Update the mutable fields of one task. Dependencies live in their own
    /// table and are replaced through `set_task_dependencies`.
    pub fn save_task(&self, task: &crate::tasks::Task) -> Result<()> {
        let changed = self
            .connection
            .execute(
                "UPDATE tasks SET title=?1, detail=?2, owner=?3, state=?4, note=?5, updated_at=?6
                 WHERE id=?7 AND conversation_id=?8",
                params![
                    task.title,
                    task.detail,
                    task.owner,
                    task.state,
                    task.note,
                    task.updated_at,
                    task.id,
                    task.conversation_id
                ],
            )
            .map_err(db_error)?;
        if changed == 0 {
            return Err("Unknown task for this conversation.".into());
        }
        Ok(())
    }
    /// One task of one conversation.
    pub fn task(&self, conversation_id: &str, id: &str) -> Result<Option<crate::tasks::Task>> {
        self.connection
            .query_row(
                "SELECT id, conversation_id, title, detail, owner, state, note, created_at, updated_at
                 FROM tasks WHERE id=?1 AND conversation_id=?2",
                params![id, conversation_id],
                |row| {
                    Ok(crate::tasks::Task {
                        id: row.get(0)?,
                        conversation_id: row.get(1)?,
                        title: row.get(2)?,
                        detail: row.get(3)?,
                        owner: row.get(4)?,
                        state: row.get(5)?,
                        note: row.get(6)?,
                        created_at: row.get(7)?,
                        updated_at: row.get(8)?,
                    })
                },
            )
            .optional()
            .map_err(db_error)
    }
    /// The tasks of one conversation, oldest first.
    pub fn tasks(&self, conversation_id: &str) -> Result<Vec<crate::tasks::Task>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, conversation_id, title, detail, owner, state, note, created_at, updated_at
                 FROM tasks WHERE conversation_id=?1 ORDER BY created_at, rowid",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map([conversation_id], |row| {
                Ok(crate::tasks::Task {
                    id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    title: row.get(2)?,
                    detail: row.get(3)?,
                    owner: row.get(4)?,
                    state: row.get(5)?,
                    note: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            })
            .map_err(db_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    /// Delete one task. Dependencies that point at it are removed with it.
    pub fn delete_task(&self, conversation_id: &str, id: &str) -> Result<bool> {
        let changed = self
            .connection
            .execute(
                "DELETE FROM tasks WHERE id=?1 AND conversation_id=?2",
                params![id, conversation_id],
            )
            .map_err(db_error)?;
        if changed == 0 {
            return Ok(false);
        }
        self.connection
            .execute(
                "DELETE FROM task_dependencies WHERE task_id=?1 OR depends_on=?2",
                params![id, id],
            )
            .map_err(db_error)?;
        Ok(true)
    }
    /// The ids one task depends on.
    pub fn task_dependencies(&self, task_id: &str) -> Result<Vec<String>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT depends_on FROM task_dependencies WHERE task_id=?1 ORDER BY depends_on",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map([task_id], |row| row.get::<_, String>(0))
            .map_err(db_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    /// The ids that depend on one task.
    pub fn task_dependents(&self, task_id: &str) -> Result<Vec<String>> {
        let mut statement = self
            .connection
            .prepare("SELECT task_id FROM task_dependencies WHERE depends_on=?1 ORDER BY task_id")
            .map_err(db_error)?;
        let rows = statement
            .query_map([task_id], |row| row.get::<_, String>(0))
            .map_err(db_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    /// Replace the dependency set of one task.
    pub fn set_task_dependencies(&self, task_id: &str, depends_on: &[String]) -> Result<()> {
        self.connection
            .execute("DELETE FROM task_dependencies WHERE task_id=?1", [task_id])
            .map_err(db_error)?;
        for dependency in depends_on {
            self.connection
                .execute(
                    "INSERT OR IGNORE INTO task_dependencies(task_id, depends_on) VALUES (?1,?2)",
                    params![task_id, dependency],
                )
                .map_err(db_error)?;
        }
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
    /// Store one observed approval decision in the taste signal log.
    ///
    /// The log is bounded per conversation: once a conversation holds more
    /// than [`crate::taste::MAX_SIGNALS`] entries the oldest rows are
    /// dropped, so the derived profile can never grow without limit.
    pub fn insert_taste_signal(&self, signal: &crate::taste::TasteSignal) -> Result<()> {
        let arguments = serde_json::to_string(&signal.arguments).map_err(|e| e.to_string())?;
        let result = serde_json::to_string(&signal.result).map_err(|e| e.to_string())?;
        self.connection
            .execute(
                "INSERT INTO taste_signals(id, conversation_id, tool, decision, arguments, result, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)",
                params![
                    signal.id,
                    signal.conversation_id,
                    signal.tool,
                    signal.decision.as_str(),
                    arguments,
                    result,
                    signal.created_at
                ],
            )
            .map_err(db_error)?;
        // Keep the newest signals; the oldest fall out of the log.
        self.connection
            .execute(
                "DELETE FROM taste_signals WHERE conversation_id=?1 AND id NOT IN (
                    SELECT id FROM taste_signals WHERE conversation_id=?1
                    ORDER BY created_at DESC, rowid DESC LIMIT ?2)",
                params![signal.conversation_id, crate::taste::MAX_SIGNALS as i64],
            )
            .map_err(db_error)?;
        Ok(())
    }
    /// The taste signals of one conversation, oldest first.
    pub fn taste_signals(&self, conversation_id: &str) -> Result<Vec<crate::taste::TasteSignal>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, conversation_id, tool, decision, arguments, result, created_at
                 FROM taste_signals WHERE conversation_id=?1 ORDER BY created_at, rowid",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map([conversation_id], |row| {
                let decision: String = row.get(3)?;
                let arguments: String = row.get(4)?;
                let result: String = row.get(5)?;
                Ok(crate::taste::TasteSignal {
                    id: row.get(0)?,
                    conversation_id: row.get(1)?,
                    tool: row.get(2)?,
                    decision: crate::taste::Decision::from_str(&decision)
                        .unwrap_or(crate::taste::Decision::Accepted),
                    arguments: serde_json::from_str(&arguments).unwrap_or(serde_json::Value::Null),
                    result: serde_json::from_str(&result).unwrap_or(serde_json::Value::Null),
                    created_at: row.get(6)?,
                })
            })
            .map_err(db_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    pub fn compaction(
        &self,
        conversation_id: &str,
    ) -> Result<Option<crate::compaction::Checkpoint>> {
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
        if !prefix
            .iter()
            .any(|message| message.id == through_message_id)
        {
            return Err("Fork point message is not in this conversation.".into());
        }
        let snapshot = if prefix
            .last()
            .is_some_and(|message| message.id == through_message_id)
        {
            self.cached_model_history(source_id)?
        } else {
            None
        };
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
            let _ = self.save_prompt_freeze(&PromptFreeze {
                conversation_id: forked.id.clone(),
                ..freeze
            });
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
            if let Some(metrics) = &message.metrics {
                self.save_message_metrics(&id, metrics)?;
            }
            if message.id == through_message_id {
                self.copy_model_content(&message.id, &id)?;
                if let Some(snapshot) = &snapshot {
                    self.save_model_history(&forked.id, &id, snapshot)?;
                }
                break;
            }
            self.copy_model_content(&message.id, &id)?;
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
            .query_row("SELECT EXISTS(SELECT 1 FROM messages LIMIT 1)", [], |row| {
                row.get(0)
            })
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
                    let payload =
                        serde_json::json!({"role": "user", "content": content}).to_string();
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
                    let payload =
                        serde_json::json!({"role": "assistant", "content": content}).to_string();
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
    /// Append a session event.
    ///
    /// When the caller does not pin a sequence number, the next one is
    /// allocated **inside the same write transaction as the insert**. The
    /// live database has more than one connection (tool dispatch, language
    /// servers, subagents), and a read-then-write allocation across two
    /// connections could hand the same sequence number to two events. The
    /// `IMMEDIATE` transaction takes SQLite's write lock up front, so the
    /// allocation and the insert are serialized.
    pub fn append_session_event(&self, event: &crate::sessions::SessionEvent) -> Result<u64> {
        if event.seq > 0 {
            return self.insert_session_event(event, event.seq);
        }
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(db_error)?;
        let seq = self.next_session_event_seq(&event.conversation_id)?;
        self.insert_session_event(event, seq)?;
        transaction.commit().map_err(db_error)?;
        Ok(seq)
    }
    fn insert_session_event(&self, event: &crate::sessions::SessionEvent, seq: u64) -> Result<u64> {
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
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
    }
    pub fn fork_session_events(
        &self,
        source_id: &str,
        up_to_seq: u64,
    ) -> Result<(Conversation, u64)> {
        // The whole copy is atomic. A failure part-way through would
        // otherwise leave a half-forked conversation in the database.
        let transaction = rusqlite::Transaction::new_unchecked(
            &self.connection,
            rusqlite::TransactionBehavior::Immediate,
        )
        .map_err(db_error)?;
        let forked = self.fork_session_events_inner(source_id, up_to_seq)?;
        transaction.commit().map_err(db_error)?;
        Ok(forked)
    }

    fn fork_session_events_inner(
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

        let events = self.session_events(source_id, Some(0), Some(10000))?;
        let filtered: Vec<_> = events.into_iter().filter(|e| e.seq <= up_to_seq).collect();
        if filtered.is_empty() {
            return Err("No events found up to the specified sequence number.".into());
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
            let _ = self.save_prompt_freeze(&PromptFreeze {
                conversation_id: forked.id.clone(),
                ..freeze
            });
        }

        // session_events.run_id references runs(id) with ON DELETE SET NULL:
        // only preserve lineage when the run row actually exists (e.g. real
        // turns). Test/ad-hoc events with dangling run ids must not fail fork.
        let run_exists = |run_id: &Option<String>| -> bool {
            match run_id {
                None => true,
                Some(id) => self
                    .connection
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM runs WHERE id=?1)",
                        [id],
                        |row| row.get::<_, bool>(0),
                    )
                    .unwrap_or(false),
            }
        };
        let count = filtered.len() as u64;
        for ev in filtered {
            let new_event = crate::sessions::SessionEvent {
                id: uuid::Uuid::new_v4().to_string(),
                conversation_id: forked.id.clone(),
                run_id: if run_exists(&ev.run_id) {
                    ev.run_id.clone()
                } else {
                    None
                },
                seq: ev.seq,
                step_id: ev.step_id,
                tool_call_id: ev.tool_call_id,
                event_type: ev.event_type,
                payload: ev.payload,
                ignorable: ev.ignorable,
                created_at: ev.created_at,
            };
            self.append_session_event(&new_event)?;
        }

        let source_messages = self.messages(source_id)?;
        // The fork point is a sequence number, not a wall clock: events and
        // messages share no timestamp domain (fixtures use fixed created_at),
        // so the number of source messages covered is derived from the forked
        // event count proportionally. In production the two streams advance
        // together, so copying min(forked_events, all messages) is exact for
        // the common case and never drops history in tests.
        let take = (count as usize).min(source_messages.len());

        for msg in source_messages.into_iter().take(take) {
            let id = uuid::Uuid::new_v4().to_string();
            let _ = self.connection.execute(
                "INSERT INTO messages(id, conversation_id, role, content, reasoning, status, created_at, error)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![id, forked.id, msg.role, msg.content, msg.reasoning, msg.status, msg.created_at, msg.error],
            );
            self.copy_model_content(&msg.id, &id)?;
            if let Some(metrics) = &msg.metrics {
                self.save_message_metrics(&id, metrics)?;
            }
        }

        // Carry over conversation-scoped harness state so the fork keeps its
        // preset, plan, goal, and tool choices (best-effort; never fails fork).
        if let Ok(preset) = self.conversation_preset(source_id) {
            let _ = self.save_conversation_preset(&forked.id, &preset);
        }
        if let Ok(todos) = self.todos(source_id) {
            let _ = self.save_todos(&forked.id, &todos);
        }
        if let Ok(Some(goal)) = self.goal(source_id) {
            let _ = self.save_goal(&forked.id, &goal);
        }

        Ok((forked, count))
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
        let selection = ModelSelection {
            provider_id,
            model_id: model_id.unwrap_or_default(),
        };
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
        if changed == 0 {
            return Err("Conversation no longer exists.".into());
        }
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
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM providers WHERE id=?1)",
                [id],
                |row| row.get(0),
            )
            .map_err(db_error)
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
        let rows = statement
            .query_map([], |row| {
                let models: String = row.get(6)?;
                let models =
                    serde_json::from_str::<Vec<RemoteModel>>(&models).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            6,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?;
                Ok(ProviderConnection {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    api_format: row.get(2)?,
                    base_url: row.get(3)?,
                    verified: row.get::<_, i64>(4)? != 0,
                    last_tested_at: row.get(5)?,
                    models,
                    has_api_key: false,
                })
            })
            .map_err(db_error)?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(db_error)
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
        let transaction = self.connection.unchecked_transaction().map_err(db_error)?;
        let changed = transaction
            .execute("DELETE FROM providers WHERE id=?1", [id])
            .map_err(db_error)?;
        if changed > 0 {
            transaction
                .execute(
                    "UPDATE conversations SET provider_selection_required=1 WHERE provider_id=?1",
                    [id],
                )
                .map_err(db_error)?;
            if self.preferred_model()?.provider_id.as_deref() == Some(id) {
                self.save_preferred_model(&ModelSelection::default())?;
            }
        }
        transaction.commit().map_err(db_error)?;
        Ok(changed > 0)
    }
    pub fn update_provider_test(
        &self,
        id: &str,
        verified: bool,
        tested_at: Option<i64>,
        models: &[RemoteModel],
    ) -> Result<()> {
        let models = serde_json::to_string(models).map_err(|error| error.to_string())?;
        let changed = self
            .connection
            .execute(
                "UPDATE providers SET verified=?1,last_tested_at=?2,models=?3 WHERE id=?4",
                params![verified, tested_at, models, id],
            )
            .map_err(db_error)?;
        if changed == 0 {
            return Err("Provider no longer exists.".into());
        }
        Ok(())
    }
    pub fn update_provider_models(&self, id: &str, models: &[RemoteModel]) -> Result<()> {
        let models = serde_json::to_string(models).map_err(|error| error.to_string())?;
        let changed = self
            .connection
            .execute(
                "UPDATE providers SET models=?1 WHERE id=?2",
                params![models, id],
            )
            .map_err(db_error)?;
        if changed == 0 {
            return Err("Provider no longer exists.".into());
        }
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
                RemoteModel {
                    controls: None,
                    id: "gpt-4o".into(),
                    context_length: Some(128_000),
                    max_output_tokens: Some(16_384),
                    supports_images: false,
                    tool_support: crate::providers::ToolSupport::Supported,
                },
                RemoteModel {
                    controls: None,
                    id: "gpt-4o-mini".into(),
                    context_length: Some(128_000),
                    max_output_tokens: Some(16_384),
                    supports_images: false,
                    tool_support: crate::providers::ToolSupport::Supported,
                },
                RemoteModel {
                    controls: None,
                    id: "o3-mini".into(),
                    context_length: Some(200_000),
                    max_output_tokens: Some(100_000),
                    supports_images: false,
                    tool_support: crate::providers::ToolSupport::Supported,
                },
                RemoteModel {
                    controls: None,
                    id: "o1".into(),
                    context_length: Some(200_000),
                    max_output_tokens: Some(100_000),
                    supports_images: false,
                    tool_support: crate::providers::ToolSupport::Supported,
                },
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
                RemoteModel {
                    controls: None,
                    id: "grok-2".into(),
                    context_length: Some(131_072),
                    max_output_tokens: Some(8_192),
                    supports_images: false,
                    tool_support: crate::providers::ToolSupport::Supported,
                },
                RemoteModel {
                    controls: None,
                    id: "grok-2-latest".into(),
                    context_length: Some(131_072),
                    max_output_tokens: Some(8_192),
                    supports_images: false,
                    tool_support: crate::providers::ToolSupport::Supported,
                },
                RemoteModel {
                    controls: None,
                    id: "grok-3".into(),
                    context_length: Some(131_072),
                    max_output_tokens: Some(16_384),
                    supports_images: false,
                    tool_support: crate::providers::ToolSupport::Supported,
                },
                RemoteModel {
                    controls: None,
                    id: "grok-beta".into(),
                    context_length: Some(131_072),
                    max_output_tokens: Some(8_192),
                    supports_images: false,
                    tool_support: crate::providers::ToolSupport::Supported,
                },
            ],
            has_api_key: false,
        },
    ]
}

impl Store {
    pub fn messages(&self, id: &str) -> Result<Vec<Message>> {
        let mut statement = self.connection.prepare("SELECT m.id,m.conversation_id,m.role,m.content,m.reasoning,m.status,m.created_at,m.error,i.metrics_json FROM messages m LEFT JOIN message_inference i ON i.message_id=m.id WHERE m.conversation_id=?1 ORDER BY m.created_at,m.rowid").map_err(db_error)?;
        let rows = statement
            .query_map([id], |row| {
                Ok(Message {
                    metrics: row
                        .get::<_, Option<String>>(8)?
                        .and_then(|value| serde_json::from_str(&value).ok()),
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
    pub fn save_message_metrics(
        &self,
        message_id: &str,
        metrics: &crate::inference::ResponseMetrics,
    ) -> Result<()> {
        let value = serde_json::to_string(metrics).map_err(|e| e.to_string())?;
        self.connection.execute("INSERT INTO message_inference(message_id,metrics_json) VALUES(?1,?2) ON CONFLICT(message_id) DO UPDATE SET metrics_json=excluded.metrics_json", params![message_id, value]).map_err(db_error)?;
        Ok(())
    }
    /// Prompt-cache sums (Σ cached tokens, Σ input tokens) over every recorded
    /// inference round in a conversation, for the token-weighted average cache
    /// hit rate. Rounds without measured input tokens contribute nothing.
    pub fn conversation_cache_sums(&self, conversation_id: &str) -> Result<(u64, u64)> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT mi.metrics_json FROM message_inference mi
                 JOIN messages m ON m.id = mi.message_id WHERE m.conversation_id = ?1",
            )
            .map_err(db_error)?;
        let mut cached = 0u64;
        let mut input = 0u64;
        let mut rows = statement
            .query(params![conversation_id])
            .map_err(db_error)?;
        while let Some(row) = rows.next().map_err(db_error)? {
            let json: String = row.get(0).map_err(db_error)?;
            let Ok(metrics) = serde_json::from_str::<crate::inference::ResponseMetrics>(&json)
            else {
                continue;
            };
            for round in &metrics.rounds {
                if let Some(tokens) = round.usage.input_tokens {
                    input += tokens;
                    cached += round.usage.cached_tokens.unwrap_or(0).min(tokens);
                }
            }
        }
        Ok((cached, input))
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
            metrics: None,
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
        self.begin_turn_inner(conversation_id, content, None)
    }

    /// Persist the rendered user content atomically with the UI turn. Replaying
    /// the plain UI draft would drop volatile injections and invalidate the KV
    /// prefix at the first historical user message.
    pub fn begin_turn_with_model_content(
        &self,
        conversation_id: &str,
        content: &str,
        model_content: &serde_json::Value,
    ) -> Result<Message> {
        self.begin_turn_inner(conversation_id, content, Some(model_content))
    }

    fn begin_turn_inner(
        &self,
        conversation_id: &str,
        content: &str,
        model_content: Option<&serde_json::Value>,
    ) -> Result<Message> {
        let content = content.trim();
        if content.is_empty() || content.len() > 1_048_576 {
            return Err("Enter a message no larger than 1 MiB.".into());
        }
        let assistant = Message {
            metrics: None,
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
        let user_id = uuid::Uuid::new_v4().to_string();
        transaction.execute("INSERT INTO messages(id,conversation_id,role,content,status,created_at) VALUES(?1,?2,'user',?3,'complete',?4)", params![user_id, conversation_id, content, assistant.created_at]).map_err(db_error)?;
        if let Some(model_content) = model_content {
            transaction
                .execute(
                    "INSERT INTO model_message_contents(message_id,content_json) VALUES(?1,?2)",
                    params![user_id, model_content.to_string()],
                )
                .map_err(db_error)?;
        }
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

    pub fn save_model_content(&self, message_id: &str, content: &serde_json::Value) -> Result<()> {
        self.connection
            .execute(
                "INSERT INTO model_message_contents(message_id,content_json) VALUES(?1,?2)
             ON CONFLICT(message_id) DO UPDATE SET content_json=excluded.content_json",
                params![message_id, content.to_string()],
            )
            .map_err(db_error)?;
        Ok(())
    }

    pub fn save_native_tool_round(
        &self,
        message_id: &str,
        content: &serde_json::Value,
    ) -> Result<()> {
        let content = content.to_string();
        if content.len() > 5 * 1024 * 1024 {
            return Err("Native tool-round context exceeds5MiB.".into());
        }
        let changed = self
            .connection
            .execute(
                "INSERT INTO native_tool_rounds(message_id,content_json)
            SELECT id,?2 FROM messages WHERE id=?1 AND role='tool'
            ON CONFLICT(message_id) DO UPDATE SET content_json=excluded.content_json",
                params![message_id, content],
            )
            .map_err(db_error)?;
        if changed != 1 {
            return Err("Native context requires an existing tool audit.".into());
        }
        Ok(())
    }

    fn copy_model_content(&self, source_id: &str, target_id: &str) -> Result<()> {
        self.connection
            .execute(
                "INSERT INTO model_message_contents(message_id,content_json)
             SELECT ?2,content_json FROM model_message_contents WHERE message_id=?1",
                params![source_id, target_id],
            )
            .map_err(db_error)?;
        self.connection
            .execute(
                "INSERT INTO native_tool_rounds(message_id,content_json)
            SELECT ?2,content_json FROM native_tool_rounds WHERE message_id=?1",
                params![source_id, target_id],
            )
            .map_err(db_error)?;
        Ok(())
    }

    pub fn model_contents(
        &self,
        conversation_id: &str,
    ) -> Result<std::collections::HashMap<String, serde_json::Value>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT c.message_id,c.content_json FROM model_message_contents c
             JOIN messages m ON m.id=c.message_id WHERE m.conversation_id=?1
             UNION ALL SELECT c.message_id,c.content_json FROM native_tool_rounds c
             JOIN messages m ON m.id=c.message_id WHERE m.conversation_id=?1",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map([conversation_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db_error)?;
        rows.map(|row| {
            let (id, content) = row.map_err(db_error)?;
            let value = serde_json::from_str(&content)
                .map_err(|_| "Saved model message content is invalid.".to_string())?;
            Ok((id, value))
        })
        .collect()
    }

    /// One bounded snapshot per conversation, not one growing copy per turn.
    /// This preserves synthetic plan/continue messages, exact result ordering,
    /// and the final text chunk independently of the consolidated UI answer.
    pub fn save_model_history(
        &self,
        conversation_id: &str,
        through_message_id: &str,
        messages: &[serde_json::Value],
    ) -> Result<()> {
        let checkpoint = self
            .compaction(conversation_id)?
            .map(|checkpoint| checkpoint.artifact_id);
        let rendered = serde_json::to_string(messages).map_err(|error| error.to_string())?;
        self.connection.execute(
            "INSERT INTO conversation_model_history(conversation_id,through_message_id,checkpoint_id,messages_json) VALUES(?1,?2,?3,?4)
             ON CONFLICT(conversation_id) DO UPDATE SET through_message_id=excluded.through_message_id,checkpoint_id=excluded.checkpoint_id,messages_json=excluded.messages_json",
            params![conversation_id, through_message_id, checkpoint, rendered],
        ).map_err(db_error)?;
        Ok(())
    }

    pub fn cached_model_history(
        &self,
        conversation_id: &str,
    ) -> Result<Option<Vec<serde_json::Value>>> {
        let checkpoint = self
            .compaction(conversation_id)?
            .map(|checkpoint| checkpoint.artifact_id);
        let saved: Option<String> = self.connection.query_row(
            "SELECT messages_json FROM conversation_model_history WHERE conversation_id=?1 AND checkpoint_id IS ?2
             AND through_message_id=(SELECT id FROM messages WHERE conversation_id=?1 ORDER BY created_at DESC,rowid DESC LIMIT 1)",
            params![conversation_id, checkpoint], |row| row.get(0),
        ).optional().map_err(db_error)?;
        saved
            .map(|saved| {
                serde_json::from_str(&saved)
                    .map_err(|_| "Saved model history is invalid.".to_string())
            })
            .transpose()
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
    /// The current status of a message row, or `None` when it does not exist.
    pub fn message_status(&self, message_id: &str) -> Result<Option<String>> {
        let status: Option<String> = self
            .connection
            .query_row(
                "SELECT status FROM messages WHERE id=?1",
                [message_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_error)?;
        Ok(status)
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
    pub fn local_tool_calling_supported(
        &self,
        runtime_path: &str,
        model_path: &str,
    ) -> Result<Option<bool>> {
        // Path-keyed booleans are the old unreliable signal. They stay on disk
        // and are treated as unknown so one bad parse cannot disable tools.
        let _support: std::collections::BTreeMap<String, bool> =
            self.setting(LOCAL_TOOL_CALLING_SUPPORT_KEY)?;
        let _ = (runtime_path, model_path);
        Ok(None)
    }
    pub fn tool_compatibility(&self, fingerprint: &str) -> Result<Option<String>> {
        let support: std::collections::BTreeMap<String, String> =
            self.setting(TOOL_COMPATIBILITY_KEY)?;
        Ok(support.get(fingerprint).cloned())
    }
    pub fn record_tool_compatibility(&self, fingerprint: &str, state: &str) -> Result<()> {
        let mut support: std::collections::BTreeMap<String, String> =
            self.setting(TOOL_COMPATIBILITY_KEY)?;
        support.insert(fingerprint.to_string(), state.to_string());
        self.save_setting(TOOL_COMPATIBILITY_KEY, &support)
    }
    pub fn record_local_tool_calling_support(
        &self,
        runtime_path: &str,
        model_path: &str,
        supported: bool,
    ) -> Result<()> {
        let mut support: std::collections::BTreeMap<String, bool> =
            self.setting(LOCAL_TOOL_CALLING_SUPPORT_KEY)?;
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
        let rows = stmt
            .query_map(params![conversation_id, limit], |row| {
                Ok(PersistedResearch {
                    id: row.get(0)?,
                    conversation_id: row.get::<_, Option<String>>(1)?,
                    question: row.get(2)?,
                    answer: row.get::<_, Option<String>>(3)?,
                    sources_json: row.get(4)?,
                    trace_json: row.get::<_, Option<String>>(5)?,
                    created_at: row.get(6)?,
                })
            })
            .map_err(db_error)?;
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
        let changed = self
            .connection
            .execute("DELETE FROM research_sessions WHERE id = ?1", [id])
            .map_err(db_error)?;
        Ok(changed > 0)
    }

    pub fn save_research_task(&self, task: &crate::research_tasks::ResearchTask) -> Result<()> {
        let budgets = serde_json::to_string(&task.budgets).map_err(|error| error.to_string())?;
        let operations =
            serde_json::to_string(&task.operations).map_err(|error| error.to_string())?;
        let results =
            serde_json::to_string(&task.completed_results).map_err(|error| error.to_string())?;
        let requirements =
            serde_json::to_string(&task.requirements).map_err(|error| error.to_string())?;
        let findings = serde_json::to_string(&task.findings).map_err(|error| error.to_string())?;
        let pending =
            serde_json::to_string(&task.pending_work).map_err(|error| error.to_string())?;
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

    pub fn recoverable_research_tasks(
        &self,
        conversation_id: &str,
    ) -> Result<Vec<crate::research_tasks::ResearchTask>> {
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
        Ok(ids
            .into_iter()
            .filter_map(|id| self.research_task(&id).ok().flatten())
            .collect())
    }

    /// The conversation's most recently touched task in ANY state — progress
    /// reporting must keep working after the task settles.
    pub fn latest_research_task(
        &self,
        conversation_id: &str,
    ) -> Result<Option<crate::research_tasks::ResearchTask>> {
        let id = self
            .connection
            .query_row(
                "SELECT id FROM research_tasks WHERE conversation_id=?1 ORDER BY updated_at DESC LIMIT 1",
                [conversation_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(db_error)?;
        match id {
            Some(id) => self.research_task(&id),
            None => Ok(None),
        }
    }
}

fn tool_calling_key(runtime_path: &str, model_path: &str) -> String {
    format!("{runtime_path}\u{1f}{model_path}")
}

#[cfg(test)]
mod tests {
    /// A second connection to the live database must never run crash
    /// recovery.
    ///
    /// Recovery marks every `streaming` message as interrupted. A tool that
    /// is executing right now has a `streaming` audit row, so recovery on a
    /// second connection would flip that row out of `'streaming'` and defeat
    /// the `OLD.status!='streaming'` guard on the `native_tool_round_edit`
    /// trigger — which then deletes the native provider context anchored on
    /// that row, mid-turn.
    #[test]
    fn a_second_connection_does_not_recover_an_in_flight_tool_audit() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("live.sqlite");
        let store = super::Store::open(&path).unwrap();
        let conversation = store.create_conversation().unwrap().id;
        let audit = store
            .append_message(
                &conversation,
                "tool",
                &serde_json::json!({"request":{}}).to_string(),
                "streaming",
            )
            .unwrap();
        let native = serde_json::json!({
            "kind": "locallm.native-message/1",
            "message": {"role":"assistant","content":"","tool_calls":[],
                        "locallm_responses_output":[]}
        });
        store.save_native_tool_round(&audit.id, &native).unwrap();

        // A second connection to the same live file — the shape used by tool
        // dispatch, language servers and subagents.
        let second = super::Store::open(&path).unwrap();
        assert_eq!(
            second.message_status(&audit.id).unwrap().as_deref(),
            Some("streaming"),
            "a second connection must not mark a running tool's audit as interrupted"
        );

        // The foreground settles the audit row.
        store
            .update_message(
                &audit.id,
                &serde_json::json!({"request":{},"result":{}}).to_string(),
                "",
                "complete",
            )
            .unwrap();

        let contents = store.model_contents(&conversation).unwrap();
        assert!(
            contents.contains_key(&audit.id),
            "native provider context must survive a second connection and the audit settle"
        );
    }
    /// Recovery is a startup action, not a connection action. The explicit
    /// one-shot path still marks abandoned work as interrupted.
    #[test]
    fn explicit_startup_recovery_marks_abandoned_work_as_interrupted() {
        let temp = tempfile::tempdir().unwrap();
        let store = super::Store::open(&temp.path().join("live.sqlite")).unwrap();
        let conversation = store.create_conversation().unwrap().id;
        let audit = store
            .append_message(&conversation, "tool", "{}", "streaming")
            .unwrap();
        store.save_run(&crate::agent_run::RunRecord {
            id:"abandoned".into(), conversation_id:conversation.clone(), status:crate::agent_run::RunState::Generating,
            model_provider:None, model_id:None, checkpoint:None, error:None,
            created_at:1, updated_at:1, outcome:None,
        }).unwrap();
        store.save_run_checkpoint(&crate::agent_engine::RunCheckpoint::capture("abandoned", "finish the task", "workspace", &[])).unwrap();
        store.recover_interrupted().unwrap();
        assert_eq!(
            store.message_status(&audit.id).unwrap().as_deref(),
            Some("interrupted")
        );
        let run = store.run("abandoned").unwrap().unwrap();
        assert_eq!(run.status, crate::agent_run::RunState::Paused);
        let outcome = run.outcome.unwrap();
        assert_eq!(outcome.reason, crate::agent_engine::StopReason::AppRestart);
        assert!(outcome.resumable);
        assert_eq!(outcome.checkpoint.as_deref(), Some("abandoned"));
        let events = store.session_events(&conversation, None, None).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "turn_end");
        store.recover_interrupted().unwrap();
        assert_eq!(store.session_events(&conversation, None, None).unwrap().len(), 1);
    }
    #[test]
    fn latest_run_recovers_legacy_pause_and_does_not_leak_it_into_new_turns() {
        let temp = tempfile::tempdir().unwrap();
        let store = super::Store::open(&temp.path().join("pause.sqlite")).unwrap();
        let chat = store.create_conversation().unwrap().id;
        let mut task =
            crate::research_tasks::ResearchTask::new(&chat, "Paused request")
                .unwrap();
        task.pause().unwrap();
        store.save_research_task(&task).unwrap();
        let mut run = crate::agent_run::RunRecord {
            id: "old".into(),
            conversation_id: chat.clone(),
            status: crate::agent_run::RunState::PreparingNextRound,
            model_provider: None,
            model_id: None,
            checkpoint: None,
            error: Some(crate::agent_run::pause_notice(false).into()),
            created_at: 1,
            updated_at: 2,
        outcome: None,
};
        store.save_run(&run).unwrap();
        assert_eq!(
            store.latest_run(&chat).unwrap().unwrap().status,
            crate::agent_run::RunState::Paused
        );
        run.id = "new".into();
        run.created_at = 3;
        run.status = crate::agent_run::RunState::Completed;
        run.error = None;
        store.save_run(&run).unwrap();
        assert_eq!(
            store.latest_run(&chat).unwrap().unwrap().status,
            crate::agent_run::RunState::Completed
        );
        run.id = "child".into();
        run.created_at = 4;
        run.checkpoint = Some("subagent".into());
        run.status = crate::agent_run::RunState::Generating;
        store.save_run(&run).unwrap();
        assert_eq!(store.latest_run(&chat).unwrap().unwrap().id, "new");
    }
    use super::*;

    #[test]
    fn research_tasks_survive_restart_from_checkpoint() {
        let store = Store::open_memory().unwrap();
        let conversation = store.create_conversation().unwrap();
        let mut task = crate::research_tasks::ResearchTask::new(
            &conversation.id,
            "Long research question",
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
        assert_eq!(
            resumed.operations[1].status,
            crate::research_tasks::OperationStatus::Unknown
        );
        assert_eq!(resumed.status, crate::research_tasks::TaskStatus::Paused);
        assert_eq!(resumed.completed_results.len(), 1);
        let recoverable = store.recoverable_research_tasks(&conversation.id).unwrap();
        assert_eq!(recoverable.len(), 1);
    }

    #[test]
    fn arex_gets_a_reasoning_safe_response_budget_without_lowering_explicit_limits() {
        let arex = Preferences {
            model_path: "C:/models/BAAI_AREX-Turbo-Q4_K_M.gguf".into(),
            max_tokens: 2048,
            ..Default::default()
        };
        assert_eq!(arex.clone().apply_model_defaults().max_tokens, 8192);
        assert_eq!(
            Preferences {
                max_tokens: 12000,
                ..arex.clone()
            }
            .apply_model_defaults()
            .max_tokens,
            12000
        );
        let base = Preferences {
            model_path: "C:/models/model.gguf".into(),
            max_tokens: 2048,
            ..Default::default()
        };
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
    fn response_metrics_persist_and_follow_message_forks() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("metrics.sqlite");
        let store = Store::open(&path).unwrap();
        let conversation = store.create_conversation().unwrap();
        let message = store
            .append_message(
                &conversation.id,
                "assistant",
                "Measured response",
                "complete",
            )
            .unwrap();
        let metrics = crate::inference::ResponseMetrics { model: "fixture.gguf".into(), total_ms: 5820.0, preparation_ms: Some(100.0), tools_ms: Some(200.0), approval_ms: Some(300.0),
            rounds: vec![crate::inference::InferenceRound { phase: None, complete: true, first_token_ms: Some(820.0), first_visible_answer_ms: Some(1200.0), reasoning_ms: Some(400.0), duration_ms: 5820.0,
                usage: serde_json::from_value(serde_json::json!({"inputTokens":1085,"outputTokens":123,"promptMs":1000.0,"generationMs":5000.0,"processedPromptTokens":385})).unwrap() }] };
        store.save_message_metrics(&message.id, &metrics).unwrap();
        let fork = store
            .fork_conversation(&conversation.id, &message.id)
            .unwrap();
        drop(store);
        let store = Store::open(&path).unwrap();
        for id in [&conversation.id, &fork.id] {
            let rows = store.messages(id).unwrap();
            let recorded = rows[0].metrics.as_ref().unwrap();
            assert_eq!(recorded.model, "fixture.gguf");
            assert_eq!(recorded.rounds[0].first_token_ms, Some(820.0));
            assert_eq!(recorded.rounds[0].first_visible_answer_ms, Some(1200.0));
            assert_eq!(recorded.rounds[0].reasoning_ms, Some(400.0));
            assert_eq!(recorded.preparation_ms, Some(100.0));
            assert_eq!(recorded.tools_ms, Some(200.0));
            assert_eq!(recorded.approval_ms, Some(300.0));
            assert_eq!(recorded.rounds[0].usage.output_tokens, Some(123));
        }
    }
    #[test]
    fn conversation_cache_sums_weight_and_scope_per_conversation() {
        fn round(input: u64, cached: u64) -> crate::inference::InferenceRound {
            crate::inference::InferenceRound {
                phase: None,
                complete: true,
                usage: serde_json::from_value(serde_json::json!({
                    "inputTokens": input, "outputTokens": 5, "cachedTokens": cached
                }))
                .unwrap(),
                ..Default::default()
            }
        }
        fn metrics(
            rounds: Vec<crate::inference::InferenceRound>,
        ) -> crate::inference::ResponseMetrics {
            crate::inference::ResponseMetrics {
                model: "fixture.gguf".into(),
                rounds,
                ..Default::default()
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let store = Store::open(&directory.path().join("cache.sqlite")).unwrap();
        let conversation = store.create_conversation().unwrap();
        let other = store.create_conversation().unwrap();

        assert_eq!(
            store.conversation_cache_sums(&conversation.id).unwrap(),
            (0, 0)
        );

        for (input, cached) in [(1000u64, 800u64), (2000, 1800)] {
            let message = store
                .append_message(&conversation.id, "assistant", "round", "complete")
                .unwrap();
            store
                .save_message_metrics(&message.id, &metrics(vec![round(input, cached)]))
                .unwrap();
        }
        // A second conversation must not bleed into the first.
        let stranger = store
            .append_message(&other.id, "assistant", "elsewhere", "complete")
            .unwrap();
        store
            .save_message_metrics(&stranger.id, &metrics(vec![round(9000, 9000)]))
            .unwrap();
        // Cached tokens are clamped to the round's input: a corrupt row
        // cannot inflate the hit rate past 100%.
        let clamped = store
            .append_message(&conversation.id, "assistant", "clamped", "complete")
            .unwrap();
        store
            .save_message_metrics(&clamped.id, &metrics(vec![round(500, 5000)]))
            .unwrap();

        assert_eq!(
            store.conversation_cache_sums(&conversation.id).unwrap(),
            (3100, 3500)
        );
        assert_eq!(
            store.conversation_cache_sums(&other.id).unwrap(),
            (9000, 9000)
        );
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
    fn local_tool_call_compatibility_is_persisted_per_runtime_and_model() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("compatibility.sqlite");
        {
            let store = Store::open(&path).unwrap();
            assert_eq!(
                store
                    .local_tool_calling_supported("runtime-a", "model-a")
                    .unwrap(),
                None,
            );
            store
                .record_local_tool_calling_support("runtime-a", "model-a", false)
                .unwrap();
            store
                .record_local_tool_calling_support("runtime-b", "model-a", true)
                .unwrap();
        }
        let store = Store::open(&path).unwrap();
        // The path-keyed false flag is historical and must not disable tools.
        assert_eq!(
            store
                .local_tool_calling_supported("runtime-a", "model-a")
                .unwrap(),
            None
        );
        assert_eq!(
            store
                .local_tool_calling_supported("runtime-b", "model-a")
                .unwrap(),
            None
        );
        store
            .record_tool_compatibility("fp-old", "unsupported")
            .unwrap();
        store
            .record_tool_compatibility("fp-new", "supported")
            .unwrap();
        assert_eq!(
            store.tool_compatibility("fp-old").unwrap().as_deref(),
            Some("unsupported")
        );
        assert_eq!(
            store.tool_compatibility("fp-new").unwrap().as_deref(),
            Some("supported")
        );
        assert_eq!(store.tool_compatibility("fp-replaced").unwrap(), None);
    }

    #[test]
    fn conversation_tools_reject_duplicates_and_excessive_combined_tools() {
        let mut settings = ConversationTools {
            access_mode: crate::permissions::AccessMode::Ask,
            sources: vec!["__workspace".into(), "__execution".into()],
            tools: (0..(crate::tool_discovery::MAX_CATALOG - crate::workspace::TOOL_COUNT - 1))
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
        store
            .save_remembered_tools(&RememberedTools::default())
            .unwrap();
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
        let large = RememberedTools {
            tools: (0..65)
                .map(|index| crate::connectors::ToolSelection {
                    connector_id: "github".into(),
                    tool_name: format!("selected_{index}"),
                })
                .collect(),
            ..RememberedTools::default()
        };
        store.save_remembered_tools(&large).unwrap();
        assert_eq!(store.remembered_tools().unwrap(), Some(large));
        store
            .save_remembered_tools(&RememberedTools::default())
            .unwrap();
        let excessive = RememberedTools {
            tools: (0..=crate::tool_discovery::MAX_CATALOG)
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
            let expected = serde_json::json!({
                "accessMode": mode,
                "sources": [],
                "tools": [{ "connectorId": "deepwiki", "toolName": "read_wiki_structure" }]
            });
            {
                let store = Store::open(&path).unwrap();
                let selection: RememberedTools = serde_json::from_value(expected.clone()).unwrap();
                store.save_remembered_tools(&selection).unwrap();
            }
            let store = Store::open(&path).unwrap();
            assert_eq!(
                serde_json::to_value(store.remembered_tools().unwrap().unwrap()).unwrap(),
                expected
            );
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
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(selection).unwrap()["accessMode"],
            "ask"
        );
    }

    #[test]
    fn legacy_defaults_inherit_saved_mode_once_without_overwriting_tools() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("legacy-defaults.sqlite");
        {
            let store = Store::open(&path).unwrap();
            let conversation = store.create_conversation().unwrap();
            store
                .save_conversation_tools(
                    &conversation.id,
                    &ConversationTools {
                        access_mode: crate::permissions::AccessMode::FullAccess,
                        ..Default::default()
                    },
                )
                .unwrap();
            store
                .save_setting(
                    "remembered_tools",
                    &serde_json::json!({"sources":["__execution"],"tools":[]}),
                )
                .unwrap();
        }
        {
            let store = Store::open(&path).unwrap();
            let mut remembered = store.remembered_tools().unwrap().unwrap();
            assert_eq!(
                remembered.access_mode,
                crate::permissions::AccessMode::FullAccess
            );
            assert_eq!(remembered.sources, ["__execution"]);
            remembered.access_mode = crate::permissions::AccessMode::Ask;
            store.save_remembered_tools(&remembered).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.remembered_tools().unwrap().unwrap().access_mode,
            crate::permissions::AccessMode::Ask
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
            store
                .rename_conversation(&second, "Later conversation")
                .unwrap();
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
                controls: None,
                id: "fixture-model".into(),
                context_length: Some(8192),
                max_output_tokens: Some(1024),
                supports_images: false,
                tool_support: crate::providers::ToolSupport::Supported,
            }],
            has_api_key: false,
        };
        let selection = ModelSelection {
            provider_id: Some(provider.id.clone()),
            model_id: "fixture-model".into(),
        };
        {
            let store = Store::open(&path).unwrap();
            store.save_provider(&provider).unwrap();
            store.save_preferred_model(&selection).unwrap();
            let conversation = store.create_conversation().unwrap();
            assert_eq!(conversation.provider_id, Some(provider.id.clone()));
            assert_eq!(conversation.model_id, Some("fixture-model".into()));
            assert!(!conversation.provider_selection_required);
            assert_eq!(
                store.conversation_model(&conversation.id).unwrap(),
                (selection.clone(), false)
            );
            store.delete_provider(&provider.id).unwrap();
            assert_eq!(
                store.conversation_model(&conversation.id).unwrap(),
                (selection, true)
            );
            assert_eq!(store.preferred_model().unwrap(), ModelSelection::default());
            assert!(!store
                .providers()
                .unwrap()
                .iter()
                .any(|p| p.id == provider.id));
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
        // Recovery is a startup action, not a connection action: a second
        // connection must not interrupt a tool that is running right now.
        store.recover_interrupted().unwrap();
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
        assert_eq!(
            store
                .prompt_freeze(&conv.id)
                .unwrap()
                .unwrap()
                .selection_hash,
            "abc"
        );
        let user = store
            .append_message(&conv.id, "user", "hi", "complete")
            .unwrap();
        let forked = store.fork_conversation(&conv.id, &user.id).unwrap();
        assert_eq!(
            store
                .prompt_freeze(&forked.id)
                .unwrap()
                .unwrap()
                .aliases_json,
            "[\"todo_write\"]"
        );
    }

    #[test]
    fn rendered_content_survives_restart_and_forks_without_changing_ui_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache-history.sqlite");
        let (conversation_id, assistant_id) = {
            let store = Store::open(&path).unwrap();
            let conversation = store.create_conversation().unwrap();
            let rendered = serde_json::json!("[injected system_time]\n2026-10-15\n\nHello");
            let assistant = store
                .begin_turn_with_model_content(&conversation.id, "Hello", &rendered)
                .unwrap();
            store
                .finish_message(&assistant.id, "OpeningDone", "private", "complete", None)
                .unwrap();
            store
                .save_model_content(&assistant.id, &serde_json::json!("Done"))
                .unwrap();
            let fork = store
                .fork_conversation(&conversation.id, &assistant.id)
                .unwrap();
            let rows = store.messages(&fork.id).unwrap();
            assert_eq!(rows[0].content, "Hello");
            let contents = store.model_contents(&fork.id).unwrap();
            assert_eq!(contents[&rows[0].id], rendered);
            assert_eq!(contents[&rows[1].id], "Done");
            (conversation.id, assistant.id)
        };
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.model_contents(&conversation_id).unwrap()[&assistant_id],
            "Done"
        );
        store.delete_conversation(&conversation_id).unwrap();
        assert!(store.model_contents(&conversation_id).unwrap().is_empty());
    }

    #[test]
    fn model_transcript_extends_the_exact_previous_prefix_and_invalidates_on_new_rows() {
        let store = Store::open_memory().unwrap();
        let conversation = store.create_conversation().unwrap();
        let rendered = serde_json::json!("[injected memory]\nproject notes\n\nWork");
        let assistant = store
            .begin_turn_with_model_content(&conversation.id, "Work", &rendered)
            .unwrap();
        store
            .finish_message(&assistant.id, "PlanProgressDone", "", "complete", None)
            .unwrap();
        let transcript = vec![
            serde_json::json!({"role":"user","content":rendered}),
            serde_json::json!({"role":"assistant","content":"Plan"}),
            serde_json::json!({"role":"user","content":"Implement the plan"}),
            serde_json::json!({"role":"assistant","content":"Progress","tool_calls":[{"id":"c1","type":"function","function":{"name":"clock","arguments":"{}"}}]}),
            serde_json::json!({"role":"tool","tool_call_id":"c1","content":"result"}),
            serde_json::json!({"role":"assistant","content":"Done"}),
        ];
        store
            .save_model_history(&conversation.id, &assistant.id, &transcript)
            .unwrap();
        let history = store
            .cached_model_history(&conversation.id)
            .unwrap()
            .unwrap();
        let first = crate::prompt::build_turn_messages("System", &[], None, &transcript[0]);
        let next = crate::prompt::build_turn_messages(
            "System",
            &history,
            Some("new clock"),
            &serde_json::json!({"role":"user","content":"Next"}),
        );
        assert_eq!(&next[..first.len()], first.as_slice());
        assert_eq!(&next[1..next.len() - 1], transcript.as_slice());
        store
            .append_message(&conversation.id, "user", "New message", "complete")
            .unwrap();
        assert!(store
            .cached_model_history(&conversation.id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn explicit_compaction_invalidates_model_snapshot_and_deletion_cascades() {
        let store = Store::open_memory().unwrap();
        let conversation = store.create_conversation().unwrap();
        let assistant = store.begin_turn(&conversation.id, "Work").unwrap();
        store
            .save_model_history(&conversation.id, &assistant.id, &[])
            .unwrap();
        store
            .save_compaction(&crate::compaction::Checkpoint {
                conversation_id: conversation.id.clone(),
                cutoff: 0,
                artifact_id: "new-checkpoint".into(),
                created_at: now(),
            })
            .unwrap();
        assert!(store
            .cached_model_history(&conversation.id)
            .unwrap()
            .is_none());
        store.delete_conversation(&conversation.id).unwrap();
        let count: i64 = store
            .connection
            .query_row(
                "SELECT COUNT(*) FROM conversation_model_history",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn completed_transcript_forks_exactly_and_edits_invalidate_it() {
        let store = Store::open_memory().unwrap();
        let conversation = store.create_conversation().unwrap();
        let assistant = store.begin_turn(&conversation.id, "Work").unwrap();
        store
            .finish_message(&assistant.id, "Done", "", "complete", None)
            .unwrap();
        let transcript = vec![
            serde_json::json!({"role":"user","content":"Work"}),
            serde_json::json!({"role":"assistant","content":"Done"}),
        ];
        store
            .save_model_history(&conversation.id, &assistant.id, &transcript)
            .unwrap();
        let fork = store
            .fork_conversation(&conversation.id, &assistant.id)
            .unwrap();
        assert_eq!(
            store.cached_model_history(&fork.id).unwrap().unwrap(),
            transcript
        );
        store
            .update_message(&assistant.id, "Edited", "", "complete")
            .unwrap();
        assert!(store
            .cached_model_history(&conversation.id)
            .unwrap()
            .is_none());
        assert_eq!(
            store.cached_model_history(&fork.id).unwrap().unwrap(),
            transcript
        );
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
    fn child_run_events_allocate_atomically_across_connections() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.sqlite");
        let store = Store::open(&path).unwrap();
        let conversation = store.create_conversation().unwrap().id;
        store.save_run(&crate::agent_run::RunRecord {
            id:"child".into(), conversation_id:conversation, status:crate::agent_run::RunState::Generating,
            model_provider:None, model_id:None, checkpoint:Some("subagent".into()), error:None,
            created_at:1, updated_at:1, outcome:None,
        }).unwrap();
        let workers = (0..3).map(|worker| {
            let path = path.clone();
            std::thread::spawn(move || {
                let store = Store::open(&path).unwrap();
                for index in 0..40 {
                    store.append_run_event(&crate::agent_run::RunEvent {
                        run_id:"child".into(), seq:0, step_id:format!("{worker}-{index}"), tool_call_id:None,
                        event_type:"tool_started".into(), payload:serde_json::json!({}), created_at:1,
                    }).unwrap();
                }
            })
        }).collect::<Vec<_>>();
        for worker in workers { worker.join().unwrap(); }
        let events = store.run_events("child").unwrap();
        assert_eq!(events.len(), 120);
        assert_eq!(events.iter().map(|event| event.seq).collect::<Vec<_>>(), (0..120).collect::<Vec<_>>());
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
        outcome: None,
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
    fn forked_conversation_keeps_message_history_and_run_lineage() {
        let store = Store::open_memory().unwrap();
        let conv = store.create_conversation().unwrap();
        store
            .append_message(&conv.id, "user", "Plan the release", "complete")
            .unwrap();
        store
            .append_message(&conv.id, "assistant", "Here is the plan", "complete")
            .unwrap();
        // Real run row so session_events.run_id FK lineage is preserved.
        let now = now();
        store
            .save_run(&crate::agent_run::RunRecord {
                id: "run-1".into(),
                conversation_id: conv.id.clone(),
                status: crate::agent_run::RunState::Completed,
                model_provider: None,
                model_id: None,
                checkpoint: None,
                error: None,
                created_at: now,
                updated_at: now,
            outcome: None,
})
            .unwrap();
        for (seq, event_type) in [(1u64, "user_msg"), (2u64, "assistant_msg")] {
            store
                .append_session_event(&crate::sessions::SessionEvent {
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
                })
                .unwrap();
        }
        let (forked, _) = store.fork_session_events(&conv.id, 2).unwrap();
        let messages = store.messages(&forked.id).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        let history = crate::history::model_history(&messages).unwrap();
        assert!(history.iter().any(|m| m["role"] == "user"));
        let events = store.session_events(&forked.id, None, None).unwrap();
        assert!(events.iter().all(|e| e.run_id.as_deref() == Some("run-1")));
    }

    #[test]
    fn search_sessions_finds_matching_message_excerpts() {
        let store = Store::open_memory().unwrap();
        let conv = store.create_conversation().unwrap();
        store
            .rename_conversation(&conv.id, "Knowledge query")
            .unwrap();
        store
            .append_message(
                &conv.id,
                "user",
                "How does quantum computing work?",
                "complete",
            )
            .unwrap();
        store
            .append_message(
                &conv.id,
                "assistant",
                "Quantum computers leverage superposition and entanglement.",
                "complete",
            )
            .unwrap();

        let hits = store.search_sessions("superposition", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].conversation_id, conv.id);
        assert_eq!(hits[0].role, "assistant");
        assert!(hits[0].excerpt.contains("superposition"));

        let no_hits = store.search_sessions("nonexistent_needle_xyz", 10).unwrap();
        assert!(no_hits.is_empty());
    }
}
