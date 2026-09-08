//! Durable cloud resource ownership, independent of conversation deletion.
use rusqlite::{params, Connection};
use serde::Serialize;
use std::path::Path;

pub struct Journal {
    connection: Connection,
}
#[tauri::command]
pub fn pending_daytona_operations(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<PendingOperation>, String> {
    state
        .daytona_journal
        .lock()
        .map_err(|_| "Cloud operation journal is unavailable.")?
        .pending()
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingOperation {
    pub name: String,
    pub credential_scope: String,
    pub sandbox_id: Option<String>,
    pub created_at: i64,
    pub cleanup_error: Option<String>,
}
impl Journal {
    pub fn open(path: &Path) -> Result<Self, String> {
        let connection = Connection::open(path).map_err(|error| error.to_string())?;
        connection
            .busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|error| error.to_string())?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; CREATE TABLE IF NOT EXISTS operations(name TEXT PRIMARY KEY,credential_scope TEXT NOT NULL,sandbox_id TEXT,created_at INTEGER NOT NULL,cleanup_error TEXT);").map_err(|error|error.to_string())?;
        Ok(Self { connection })
    }
    /// Commit before issuing any creation request. Never automatically repeat creation for this name.
    pub fn begin(&self, credential_scope: &str) -> Result<String, String> {
        if credential_scope.len() != 64
            || !credential_scope
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("Invalid credential scope for cloud resource ownership.".into());
        }
        let name = format!("locallm-{}", uuid::Uuid::new_v4());
        self.connection
            .execute(
                "INSERT INTO operations(name,credential_scope,created_at) VALUES(?1,?2,?3)",
                params![name, credential_scope, crate::store::now()],
            )
            .map_err(|error| error.to_string())?;
        Ok(name)
    }
    /// An operation cannot be reassigned to a different sandbox after association.
    pub fn associate(&self, name: &str, sandbox_id: &str) -> Result<(), String> {
        if sandbox_id.is_empty()
            || sandbox_id.len() > 128
            || !sandbox_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err("Invalid Daytona sandbox identifier.".into());
        }
        let count=self.connection.execute("UPDATE operations SET sandbox_id=?1 WHERE name=?2 AND (sandbox_id IS NULL OR sandbox_id=?1)",params![sandbox_id,name]).map_err(|error|error.to_string())?;
        if count != 1 {
            return Err("Cloud operation is missing or already belongs to another sandbox.".into());
        }
        Ok(())
    }
    /// Only pass sanitized transport errors, never remote response bodies or credentials.
    pub fn cleanup_failed(&self, name: &str, error: &str) -> Result<(), String> {
        if error.len() > 4096 {
            return Err("Cleanup error exceeds its storage limit.".into());
        }
        if self
            .connection
            .execute(
                "UPDATE operations SET cleanup_error=?1 WHERE name=?2",
                params![error, name],
            )
            .map_err(|error| error.to_string())?
            != 1
        {
            return Err("Unknown cloud operation.".into());
        }
        Ok(())
    }
    /// Remove only after remote absence has been verified, not merely after DELETE was accepted.
    pub fn acknowledge_absent(&self, name: &str, credential_scope: &str) -> Result<(), String> {
        if self
            .connection
            .execute(
                "DELETE FROM operations WHERE name=?1 AND credential_scope=?2",
                params![name, credential_scope],
            )
            .map_err(|error| error.to_string())?
            != 1
        {
            return Err("Cloud operation or credential scope does not match.".into());
        }
        Ok(())
    }
    pub fn pending(&self) -> Result<Vec<PendingOperation>, String> {
        let mut statement=self.connection.prepare("SELECT name,credential_scope,sandbox_id,created_at,cleanup_error FROM operations ORDER BY created_at,name").map_err(|error|error.to_string())?;
        let records = statement
            .query_map([], |row| {
                Ok(PendingOperation {
                    name: row.get(0)?,
                    credential_scope: row.get(1)?,
                    sandbox_id: row.get(2)?,
                    created_at: row.get(3)?,
                    cleanup_error: row.get(4)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string());
        records
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ambiguous_creation_and_failed_cleanup_survive_reopening() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("cloud.sqlite");
        let scope = "a".repeat(64);
        let journal = Journal::open(&path).unwrap();
        let name = journal.begin(&scope).unwrap();
        drop(journal);
        let journal = Journal::open(&path).unwrap();
        let pending = journal.pending().unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].name, name);
        assert!(pending[0].sandbox_id.is_none());
        journal.associate(&name, "sandbox-1").unwrap();
        journal.associate(&name, "sandbox-1").unwrap();
        assert!(journal.associate(&name, "sandbox-2").is_err());
        journal
            .cleanup_failed(&name, "Daytona returned HTTP 503.")
            .unwrap();
        drop(journal);
        let journal = Journal::open(&path).unwrap();
        let pending = journal.pending().unwrap();
        assert_eq!(pending[0].sandbox_id.as_deref(), Some("sandbox-1"));
        assert!(pending[0].cleanup_error.as_ref().unwrap().contains("503"));
        assert!(journal.acknowledge_absent(&name, &"b".repeat(64)).is_err());
        assert_eq!(journal.pending().unwrap().len(), 1);
        journal.acknowledge_absent(&name, &scope).unwrap();
        assert!(journal.pending().unwrap().is_empty());
    }
    #[test]
    fn invalid_and_unknown_operations_do_not_create_records() {
        let temp = tempfile::tempdir().unwrap();
        let journal = Journal::open(&temp.path().join("cloud.sqlite")).unwrap();
        assert!(journal.begin("secret").is_err());
        assert!(journal.associate("missing", "id").is_err());
        assert!(journal.cleanup_failed("missing", "failure").is_err());
        assert!(journal.pending().unwrap().is_empty());
    }
}
