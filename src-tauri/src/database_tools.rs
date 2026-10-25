//! SQLite inspection and migration rehearsal on private, bounded snapshots.
use crate::development_tools as dev;
use rusqlite::{
    hooks::{AuthAction, AuthContext, Authorization},
    limits::Limit,
    params_from_iter,
    types::{Value as SqlValue, ValueRef},
    Connection,
};
use serde_json::{json, Value};
struct Snapshot {
    connection: Connection,
    _temp: tempfile::TempDir,
    sha: String,
}
fn guard(connection: &Connection, migration: bool) -> Result<(), String> {
    connection
        .set_limit(Limit::SQLITE_LIMIT_LENGTH, 1024 * 1024)
        .map_err(|e| e.to_string())?;
    connection
        .set_limit(Limit::SQLITE_LIMIT_SQL_LENGTH, 65536)
        .map_err(|e| e.to_string())?;
    connection
        .set_limit(Limit::SQLITE_LIMIT_COLUMN, 256)
        .map_err(|e| e.to_string())?;
    connection
        .set_limit(Limit::SQLITE_LIMIT_EXPR_DEPTH, 50)
        .map_err(|e| e.to_string())?;
    connection
        .set_limit(Limit::SQLITE_LIMIT_COMPOUND_SELECT, 50)
        .map_err(|e| e.to_string())?;
    connection
        .set_limit(Limit::SQLITE_LIMIT_VARIABLE_NUMBER, 100)
        .map_err(|e| e.to_string())?;
    connection
        .execute_batch(
            "PRAGMA temp_store=MEMORY; PRAGMA trusted_schema=OFF; PRAGMA foreign_keys=ON;",
        )
        .map_err(|e| e.to_string())?;
    let page_size: i64 = connection
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .map_err(|e| e.to_string())?;
    connection
        .execute_batch(&format!(
            "PRAGMA max_page_count={}; PRAGMA cache_size=-4096;",
            (32 * 1024 * 1024 / page_size.max(1)).max(1)
        ))
        .map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut ticks = 0;
    connection
        .progress_handler(
            1000,
            Some(move || {
                ticks += 1;
                ticks > 10000 || std::time::Instant::now() > deadline
            }),
        )
        .map_err(|e| e.to_string())?;
    connection
        .authorizer(Some(move |ctx: AuthContext<'_>| match ctx.action {
            AuthAction::Attach { .. }
            | AuthAction::Detach { .. }
            | AuthAction::CreateVtable { .. }
            | AuthAction::DropVtable { .. }
            | AuthAction::Unknown { .. } => Authorization::Deny,
            AuthAction::Pragma { pragma_name, .. } => {
                if [
                    "table_info",
                    "table_xinfo",
                    "index_list",
                    "index_info",
                    "foreign_key_list",
                    "foreign_key_check",
                    "integrity_check",
                    "quick_check",
                ]
                .contains(&pragma_name.to_ascii_lowercase().as_str())
                {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }
            AuthAction::Function { function_name }
                if ["load_extension", "readfile", "writefile", "fts3_tokenizer"]
                    .contains(&function_name.to_ascii_lowercase().as_str()) =>
            {
                Authorization::Deny
            }
            AuthAction::Select
            | AuthAction::Read { .. }
            | AuthAction::Function { .. }
            | AuthAction::Recursive => Authorization::Allow,
            _ => {
                if migration {
                    Authorization::Allow
                } else {
                    Authorization::Deny
                }
            }
        }))
        .map_err(|e| e.to_string())?;
    Ok(())
}
fn snapshot(root: &str, path: &str, migration: bool) -> Result<Snapshot, String> {
    let workspace = crate::workspace::Workspace::open(root)?;
    let temp = tempfile::tempdir().map_err(|e| e.to_string())?;
    let target = temp.path().join("snapshot.sqlite");
    workspace.sqlite_backup(path, &target)?;
    let bytes = std::fs::read(&target).map_err(|e| e.to_string())?;
    if bytes.len() > 32 * 1024 * 1024 {
        return Err("SQLite backup exceeds 32 MiB.".into());
    }
    let sha = dev::hash(&bytes);
    let connection = Connection::open_with_flags(
        target,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())?;
    connection
        .busy_timeout(std::time::Duration::from_millis(250))
        .map_err(|e| e.to_string())?;
    guard(&connection, migration)?;
    let checked: String = connection
        .query_row("PRAGMA quick_check(1)", [], |r| r.get(0))
        .map_err(|e| format!("Invalid/inconsistent database snapshot: {e}"))?;
    if checked != "ok" {
        return Err(format!("Database snapshot failed quick_check: {checked}"));
    }
    Ok(Snapshot {
        connection,
        _temp: temp,
        sha,
    })
}
fn parameters(args: &Value) -> Result<Vec<SqlValue>, String> {
    args.as_array()
        .into_iter()
        .flatten()
        .map(|v| match v {
            Value::Null => Ok(SqlValue::Null),
            Value::Bool(v) => Ok(SqlValue::Integer(i64::from(*v))),
            Value::Number(n) => {
                if let Some(v) = n.as_i64() {
                    Ok(SqlValue::Integer(v))
                } else {
                    n.as_f64()
                        .map(SqlValue::Real)
                        .ok_or("Invalid SQL number.".into())
                }
            }
            Value::String(v) => Ok(SqlValue::Text(v.clone())),
            _ => Err("SQL parameters must be scalar JSON values.".into()),
        })
        .collect()
}
fn query(
    connection: &Connection,
    sql: &str,
    params: Vec<SqlValue>,
    limit: usize,
) -> Result<Value, String> {
    let mut statement = connection.prepare(sql).map_err(|e| e.to_string())?;
    if !statement.readonly() {
        return Err("Query must be read-only; rehearse mutations with database_rehearse.".into());
    }
    let columns = statement
        .column_names()
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut cursor = statement
        .query(params_from_iter(params))
        .map_err(|e| e.to_string())?;
    let mut rows = Vec::new();
    let mut truncated = false;
    let mut bytes = 0;
    while let Some(row) = cursor.next().map_err(|e| e.to_string())? {
        if rows.len() >= limit {
            truncated = true;
            break;
        }
        let mut cells = Vec::new();
        for index in 0..columns.len() {
            let value = match row.get_ref(index).map_err(|e| e.to_string())? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(v) => json!(v),
                ValueRef::Real(v) => json!(v),
                ValueRef::Text(t) => {
                    if t.len() > 65536 {
                        truncated = true;
                        json!({"type":"text","bytes":t.len(),"omitted":true})
                    } else {
                        json!(String::from_utf8_lossy(t))
                    }
                }
                ValueRef::Blob(b) => json!({"type":"blob","bytes":b.len(),"sha256":dev::hash(b)}),
            };
            bytes += value.to_string().len();
            cells.push(value);
        }
        if bytes > 256 * 1024 {
            truncated = true;
            break;
        }
        rows.push(cells);
    }
    Ok(json!({"columns":columns,"rows":rows,"truncated":truncated,"rowLimit":limit}))
}
pub fn execute(root: &str, alias: &str, args: Value) -> Result<Value, String> {
    let path = args["path"].as_str().ok_or("Missing database path.")?;
    let is_migration = alias == "database_rehearse";
    let snapshot = snapshot(root, path, is_migration)?;
    let c = &snapshot.connection;
    let limit = args["limit"].as_u64().unwrap_or(100) as usize;
    let result=match alias {
        "database_schema"=>query(c,"SELECT type,name,tbl_name,sql FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%' ORDER BY type,name",vec![],limit)?,
        "database_query"=>query(c,args["sql"].as_str().ok_or("Missing SQL.")?,parameters(&args["parameters"])?,limit)?,
        "database_explain"=>query(c,&format!("EXPLAIN QUERY PLAN {}",args["sql"].as_str().ok_or("Missing SQL.")?),parameters(&args["parameters"])?,limit)?,
        "database_rehearse"=>{
            let before=query(c,"SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name",vec![],200)?;
            // The connection can only reach the private disposable copy. No apply-to-original path exists.
            let sql=args["sql"].as_str().ok_or("Missing migration SQL.")?;
            match c.execute_batch(sql) {
                Ok(())=>{let after=query(c,"SELECT type,name,tbl_name,sql FROM sqlite_schema ORDER BY type,name",vec![],200)?;let integrity=query(c,"PRAGMA integrity_check(10)",vec![],10)?;let foreign=query(c,"PRAGMA foreign_key_check",vec![],100)?;
                    json!({"migrationSucceeded":true,"before":before,"after":after,"integrity":integrity,"foreignKeys":foreign,"originalModified":false,"appliedTo":"disposable_snapshot"})},
                Err(e)=>json!({"migrationSucceeded":false,"isError":true,"errorMessage":e.to_string(),"originalModified":false,"appliedTo":"disposable_snapshot","message":"Partial changes exist only in the disposable copy and will be discarded."})
            }
        },_=>return Err("Unknown database tool.".into())
    };
    Ok(
        json!({"databasePath":path,"sourceSha256":snapshot.sha,"snapshotOnly":true,"result":result,"isError":result["isError"]==true,"untrustedDatabaseContent":true,"limitation":"SQLite online snapshot, 32 MiB input cap. Source identity checks are optimistic against external path races. Query output and execution are bounded."}),
    )
}
pub fn registry() -> Vec<crate::tool_spec::ToolSpec> {
    let path = json!({"type":"string","minLength":1,"maxLength":4096});
    let sql = json!({"type":"string","minLength":1,"maxLength":65536});
    let mut tools: Vec<crate::tool_spec::ToolSpec> = vec![crate::database_rehearsal::definition()]
        .into_iter()
        .map(crate::tool_spec::ToolSpec::from)
        .collect();
    for (alias,description) in [("database_schema","Inspect SQLite schema on a private snapshot; never modifies the original database."),("database_query","Run one read-only parameterized SQLite query on a private snapshot. External file access, extensions, writes and mutable PRAGMAs are denied."),("database_explain","Read EXPLAIN QUERY PLAN for a parameterized SQLite query on a private snapshot."),("database_rehearse","Rehearse SQLite migration SQL on a disposable private snapshot, then inspect schema, integrity and foreign keys. Never applies changes to the original. External attachments and extension/file access are denied.")] {
        let mut properties=json!({"profile_id":{"type":"string","minLength":1,"maxLength":128},"path":path,"limit":{"type":"integer","minimum":1,"maximum":200}});let mut required=vec![];
        if alias!="database_schema" {properties["sql"]=sql.clone();required.push("sql");}
        if matches!(alias,"database_query"|"database_explain"){properties["parameters"]=json!({"type":"array","maxItems":100,"items":{"type":["string","number","boolean","null"],"maxLength":65536}});}
        tools.push(crate::tool_spec::ToolSpec::from((alias.into(),description.into(),dev::schema(properties,&required))));
    }
    tools
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_database_queries_rehearsal_and_escape_refusal() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("test.sqlite");
        let c = Connection::open(&path).unwrap();
        c.execute_batch("CREATE TABLE things(id INTEGER PRIMARY KEY,name TEXT); INSERT INTO things VALUES(1,'one');").unwrap();
        drop(c);
        let root = t.path().to_str().unwrap();
        let before = std::fs::read(&path).unwrap();
        let q=execute(root,"database_query",json!({"path":"test.sqlite","sql":"SELECT name FROM things WHERE id=?1","parameters":[1]})).unwrap();
        assert_eq!(q["result"]["rows"][0][0], "one");
        assert!(execute(
            root,
            "database_query",
            json!({"path":"test.sqlite","sql":"DELETE FROM things"})
        )
        .is_err());
        let r=execute(root,"database_rehearse",json!({"path":"test.sqlite","sql":"ALTER TABLE things ADD COLUMN extra TEXT; INSERT INTO things(id) VALUES(2);"})).unwrap();
        assert_eq!(r["result"]["migrationSucceeded"], true);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let escape = t.path().join("escape.sqlite");
        let r = execute(
            root,
            "database_rehearse",
            json!({"path":"test.sqlite","sql":format!("ATTACH '{}' AS escape;",escape.display())}),
        )
        .unwrap();
        assert_eq!(r["isError"], true);
        assert!(!escape.exists());
        let r = execute(
            root,
            "database_rehearse",
            json!({"path":"test.sqlite","sql":format!("VACUUM INTO '{}';",escape.display())}),
        )
        .unwrap();
        assert_eq!(r["isError"], true);
        assert!(!escape.exists());
    }
    #[test]
    fn live_wal_is_consistently_backed_up_without_changing_journal_mode() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("wal.sqlite");
        let c = Connection::open(&path).unwrap();
        c.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE x(id);")
            .unwrap();
        let s = snapshot(t.path().to_str().unwrap(), "wal.sqlite", false).unwrap();
        let count: i64 = s
            .connection
            .query_row("SELECT count(*) FROM x", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        let mode: String = c
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
    }
}
