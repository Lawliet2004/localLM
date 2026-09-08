use crate::runtime_config::RuntimeConfig;
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

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: String,
    pub title: String,
    pub updated_at: i64,
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
impl ConversationTools {
    pub fn validate(&self) -> Result<()> {
        let mut seen = std::collections::HashSet::new();
        let mut count = self.tools.len();
        for source in &self.sources {
            if !seen.insert(source) {
                return Err("Duplicate tool source.".into());
            }
            count += match source.as_str() {
                "__workspace" => 4,
                "__execution" => 1,
                "__daytona" => 1,
                _ => return Err("Unknown local tool source.".into()),
            };
        }
        if count > 32 {
            return Err("At most 32 tools can be selected.".into());
        }
        let mut seen = std::collections::HashSet::new();
        for tool in &self.tools {
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
}
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
            UPDATE messages SET status='interrupted' WHERE status='streaming';").map_err(db_error)?;
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
        Ok(Self { connection })
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
    pub fn create_conversation(&self) -> Result<Conversation> {
        let conversation = Conversation {
            id: uuid::Uuid::new_v4().to_string(),
            title: "New conversation".into(),
            updated_at: now(),
        };
        self.connection
            .execute(
                "INSERT INTO conversations VALUES (?1,?2,?3)",
                params![conversation.id, conversation.title, conversation.updated_at],
            )
            .map_err(db_error)?;
        Ok(conversation)
    }
    pub fn list_conversations(&self) -> Result<Vec<Conversation>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id,title,updated_at FROM conversations ORDER BY updated_at DESC,rowid DESC",
            )
            .map_err(db_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok(Conversation {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    updated_at: row.get(2)?,
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
    fn setting<T: serde::de::DeserializeOwned + Default>(&self, key: &str) -> Result<T> {
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
    fn save_setting<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
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
            tools: (0..27)
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
