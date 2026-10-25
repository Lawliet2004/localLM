//! Native-configured PostgreSQL/MySQL connections with server-enforced read-only queries.
use crate::development_tools as dev;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx_core::{
    column::Column, connection::Connection, row::Row, type_info::TypeInfo, value::ValueRef,
};
use sqlx_mysql::{MySql, MySqlConnectOptions, MySqlConnection, MySqlSslMode};
use sqlx_postgres::{PgConnectOptions, PgConnection, PgSslMode, Postgres};
const KEY: &str = "development.database_profiles";
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub engine: String,
    pub host: String,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub tls: String,
    #[serde(default)]
    pub scratch_profile_id: Option<String>,
    #[serde(default)]
    pub dump_executable: String,
    #[serde(default)]
    pub restore_executable: String,
}
impl Profile {
    fn validate(&self) -> Result<(), String> {
        if !["postgres", "mysql"].contains(&self.engine.as_str())
            || self.host.is_empty()
            || self.host.len() > 256
            || self.port == 0
            || self.name.is_empty()
            || self.name.len() > 128
            || self.database.is_empty()
            || self.database.len() > 128
            || self.username.is_empty()
            || self.username.len() > 128
            || !["verify-full", "require", "disable"].contains(&self.tls.as_str())
        {
            return Err(
                "Provide a valid database name, host, port, engine, user and TLS policy.".into(),
            );
        }
        if self.host.contains(['\0', '/', '\\'])
            || self.database.contains('\0')
            || self.username.contains('\0')
        {
            return Err("Database fields contain invalid characters.".into());
        }
        Ok(())
    }
}
#[tauri::command]
pub fn list_database_profiles(
    state: tauri::State<'_, crate::AppState>,
) -> Result<Vec<Profile>, String> {
    state.database()?.setting(KEY)
}
#[tauri::command]
pub async fn save_database_profile(
    state: tauri::State<'_, crate::AppState>,
    mut profile: Profile,
    password: Option<String>,
) -> Result<Profile, String> {
    profile.validate()?;
    let mut items: Vec<Profile> = state.database()?.setting(KEY)?;
    if profile.id.is_empty() {
        if items.len() >= 16 {
            return Err("At most 16 database profiles may be configured.".into());
        }
        profile.id = format!("dev-db-{}", uuid::Uuid::new_v4().simple());
    } else if !items.iter().any(|p| p.id == profile.id) {
        return Err("Cannot update an unknown database profile.".into());
    }
    if profile.scratch_profile_id.as_deref() == Some(&profile.id) {
        return Err("A source profile cannot be its own scratch administration profile.".into());
    }
    if let Some(password) = password {
        if password.len() > 8192 {
            return Err("Database password exceeds its bound.".into());
        }
        state
            .daytona_vault
            .save_blocking(&profile.id, password.into_bytes())
            .await?;
    }
    items.retain(|p| p.id != profile.id);
    items.push(profile.clone());
    state.database()?.save_setting(KEY, &items)?;
    Ok(profile)
}
#[tauri::command]
pub async fn delete_database_profile(
    state: tauri::State<'_, crate::AppState>,
    id: String,
) -> Result<(), String> {
    let mut items: Vec<Profile> = state.database()?.setting(KEY)?;
    if !items.iter().any(|p| p.id == id) {
        return Err("Unknown database profile.".into());
    }
    if items
        .iter()
        .any(|p| p.scratch_profile_id.as_deref() == Some(&id))
    {
        return Err("This profile is still used for scratch administration.".into());
    }
    items.retain(|p| p.id != id);
    state.database()?.save_setting(KEY, &items)?;
    state.daytona_vault.clear_blocking(&id).await
}
pub fn load(state: &crate::AppState, id: &str) -> Result<Profile, String> {
    let items: Vec<Profile> = state.database()?.setting(KEY)?;
    items
        .into_iter()
        .find(|p| p.id == id)
        .ok_or("Unknown database profile.".into())
}
pub(super) async fn password(state: &crate::AppState, p: &Profile) -> Result<String, String> {
    String::from_utf8(
        state
            .daytona_vault
            .load_blocking(&p.id)
            .await?
            .ok_or("Database profile password is unavailable in the vault.")?,
    )
    .map_err(|_| "Database password is invalid.".into())
}
pub(super) fn pg_options(p: &Profile, password: &str) -> PgConnectOptions {
    use sqlx_core::connection::ConnectOptions;
    PgConnectOptions::new()
        .host(&p.host)
        .port(p.port)
        .database(&p.database)
        .username(&p.username)
        .password(password)
        .ssl_mode(match p.tls.as_str() {
            "disable" => PgSslMode::Disable,
            "require" => PgSslMode::Require,
            _ => PgSslMode::VerifyFull,
        })
        .disable_statement_logging()
}
pub(super) fn mysql_options(p: &Profile, password: &str) -> MySqlConnectOptions {
    use sqlx_core::connection::ConnectOptions;
    MySqlConnectOptions::new()
        .host(&p.host)
        .port(p.port)
        .database(&p.database)
        .username(&p.username)
        .password(password)
        .ssl_mode(match p.tls.as_str() {
            "disable" => MySqlSslMode::Disabled,
            "require" => MySqlSslMode::Required,
            _ => MySqlSslMode::VerifyIdentity,
        })
        .disable_statement_logging()
}
fn pg_bind<'a>(
    sql: &'a str,
    parameters: &Value,
) -> Result<sqlx_core::query::Query<'a, Postgres, sqlx_postgres::PgArguments>, String> {
    let mut q = sqlx_core::query::query::<Postgres>(sql);
    for p in parameters.as_array().into_iter().flatten() {
        q = match p {
            Value::Null => q.bind(Option::<String>::None),
            Value::Bool(v) => q.bind(*v),
            Value::Number(v) if v.is_i64() => q.bind(v.as_i64()),
            Value::Number(v) => q.bind(v.as_f64()),
            Value::String(v) => q.bind(v.clone()),
            _ => return Err("SQL parameters must be scalar values.".into()),
        };
    }
    Ok(q)
}
fn mysql_bind<'a>(
    sql: &'a str,
    parameters: &Value,
) -> Result<sqlx_core::query::Query<'a, MySql, sqlx_mysql::MySqlArguments>, String> {
    let mut q = sqlx_core::query::query::<MySql>(sql);
    for p in parameters.as_array().into_iter().flatten() {
        q = match p {
            Value::Null => q.bind(Option::<String>::None),
            Value::Bool(v) => q.bind(*v),
            Value::Number(v) if v.is_i64() => q.bind(v.as_i64()),
            Value::Number(v) => q.bind(v.as_f64()),
            Value::String(v) => q.bind(v.clone()),
            _ => return Err("SQL parameters must be scalar values.".into()),
        };
    }
    Ok(q)
}
fn pg_cell(row: &sqlx_postgres::PgRow, i: usize) -> Value {
    let Ok(raw) = row.try_get_raw(i) else {
        return json!({"unavailable":true});
    };
    if raw.is_null() {
        return Value::Null;
    }
    let kind = row.columns()[i].type_info().name();
    match kind {
        "BOOL" => row.try_get::<bool, _>(i).ok().map(|v| json!(v)),
        "INT2" => row.try_get::<i16, _>(i).ok().map(|v| json!(v)),
        "INT4" => row.try_get::<i32, _>(i).ok().map(|v| json!(v)),
        "INT8" => row.try_get::<i64, _>(i).ok().map(|v| json!(v)),
        "FLOAT4" => row.try_get::<f32, _>(i).ok().map(|v| json!(v)),
        "FLOAT8" => row.try_get::<f64, _>(i).ok().map(|v| json!(v)),
        "JSON" | "JSONB" => row.try_get::<Value, _>(i).ok(),
        _ => row.try_get::<String, _>(i).ok().map(bounded_text),
    }
    .unwrap_or_else(|| json!({"type":kind,"omitted":true}))
}
fn mysql_cell(row: &sqlx_mysql::MySqlRow, i: usize) -> Value {
    let Ok(raw) = row.try_get_raw(i) else {
        return json!({"unavailable":true});
    };
    if raw.is_null() {
        return Value::Null;
    }
    let kind = row.columns()[i].type_info().name();
    if kind == "JSON" {
        if let Ok(v) = row.try_get::<Value, _>(i) {
            return v;
        }
    }
    if let Ok(v) = row.try_get::<i64, _>(i) {
        return json!(v);
    }
    if let Ok(v) = row.try_get::<u64, _>(i) {
        return json!(v);
    }
    if let Ok(v) = row.try_get::<f64, _>(i) {
        return json!(v);
    }
    row.try_get::<String, _>(i)
        .ok()
        .map(bounded_text)
        .unwrap_or_else(|| json!({"type":kind,"omitted":true}))
}
fn bounded_text(v: String) -> Value {
    if v.len() > 65536 {
        json!({"type":"text","bytes":v.len(),"sha256":dev::hash(v.as_bytes()),"omitted":true})
    } else {
        json!(v)
    }
}
pub async fn query(
    state: &crate::AppState,
    p: &Profile,
    alias: &str,
    args: &Value,
) -> Result<Value, String> {
    p.validate()?;
    if state.database()?.setting::<bool>("offline_mode")? {
        return Err("Offline mode blocks remote database access.".into());
    }
    let secret = password(state, p).await?;
    let sql = if alias == "database_schema" {
        if p.engine == "postgres" {
            "SELECT table_schema,table_name,column_name,data_type,is_nullable,column_default FROM information_schema.columns WHERE table_schema NOT IN ('pg_catalog','information_schema') ORDER BY table_schema,table_name,ordinal_position".into()
        } else {
            "SELECT table_schema,table_name,column_name,column_type,is_nullable,column_default FROM information_schema.columns WHERE table_schema=DATABASE() ORDER BY table_name,ordinal_position".into()
        }
    } else {
        let sql = args["sql"].as_str().ok_or("Missing SQL.")?;
        if alias == "database_explain" {
            format!("EXPLAIN {sql}")
        } else {
            sql.into()
        }
    };
    let limit = args["limit"].as_u64().unwrap_or(100) as usize;
    let mut rows = Vec::new();
    let mut columns = Vec::new();
    let mut bytes = 0usize;
    let mut truncated = false;
    if p.engine == "postgres" {
        let mut c = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            PgConnection::connect_with(&pg_options(p, &secret)),
        )
        .await
        .map_err(|_| "PostgreSQL connection timed out.")?
        .map_err(|_| {
            "PostgreSQL connection failed; check TLS, credentials and profile settings."
        })?;
        let mut tx = c
            .begin()
            .await
            .map_err(|_| "Could not begin PostgreSQL read-only transaction.")?;
        sqlx_core::query::query("SET TRANSACTION READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(|_| "Could not enforce PostgreSQL read-only mode.")?;
        sqlx_core::query::query("SET LOCAL statement_timeout='5s'")
            .execute(&mut *tx)
            .await
            .map_err(|_| "Cannot configure query timeout.")?;
        let mut stream = pg_bind(&sql, &args["parameters"])?.fetch(&mut *tx);
        use futures_util::TryStreamExt;
        while let Some(row) = stream
            .try_next()
            .await
            .map_err(|_| "PostgreSQL query failed or was refused by read-only transaction/role.")?
        {
            if rows.len() >= limit {
                truncated = true;
                break;
            }
            if columns.is_empty() {
                columns = row.columns().iter().map(|c| c.name().to_string()).collect();
            }
            let values = (0..row.len()).map(|i| pg_cell(&row, i)).collect::<Vec<_>>();
            bytes += serde_json::to_vec(&values)
                .map_err(|e| e.to_string())?
                .len();
            if bytes > 256 * 1024 {
                truncated = true;
                break;
            }
            rows.push(values);
        }
        drop(stream);
        tx.rollback()
            .await
            .map_err(|_| "PostgreSQL read-only transaction cleanup failed.")?;
    } else {
        let mut c = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            MySqlConnection::connect_with(&mysql_options(p, &secret)),
        )
        .await
        .map_err(|_| "MySQL connection timed out.")?
        .map_err(|_| "MySQL connection failed; check TLS, credentials and profile settings.")?;
        sqlx_core::query::query::<MySql>("SET SESSION TRANSACTION READ ONLY")
            .execute(&mut c)
            .await
            .map_err(|_| "Cannot enforce MySQL read-only transactions.")?;
        sqlx_core::query::query::<MySql>("SET SESSION MAX_EXECUTION_TIME=5000")
            .execute(&mut c)
            .await
            .map_err(|_| "Cannot configure MySQL query timeout.")?;
        let mut tx = c
            .begin()
            .await
            .map_err(|_| "Could not begin MySQL read-only transaction.")?;
        let mut stream = mysql_bind(&sql, &args["parameters"])?.fetch(&mut *tx);
        use futures_util::TryStreamExt;
        while let Some(row) = stream
            .try_next()
            .await
            .map_err(|_| "MySQL query failed or was refused by read-only transaction/role.")?
        {
            if rows.len() >= limit {
                truncated = true;
                break;
            }
            if columns.is_empty() {
                columns = row.columns().iter().map(|c| c.name().to_string()).collect();
            }
            let values = (0..row.len())
                .map(|i| mysql_cell(&row, i))
                .collect::<Vec<_>>();
            bytes += serde_json::to_vec(&values)
                .map_err(|e| e.to_string())?
                .len();
            if bytes > 256 * 1024 {
                truncated = true;
                break;
            }
            rows.push(values);
        }
        drop(stream);
        tx.rollback()
            .await
            .map_err(|_| "MySQL read-only transaction cleanup failed.")?;
    }
    Ok(
        json!({"profileId":p.id,"engine":p.engine,"readOnly":true,"columns":columns,"rows":rows,"truncated":truncated,"rowLimit":limit,"untrustedDatabaseContent":true,"limitation":"Use a database account restricted to read access. Omitted column encodings are explicit; temporary-table privileges and external side effects of privileged server extensions are not OS-isolated."}),
    )
}
#[tauri::command]
pub async fn test_database_profile(
    state: tauri::State<'_, crate::AppState>,
    id: String,
) -> Result<Value, String> {
    let p = load(&state, &id)?;
    query(
        &state,
        &p,
        "database_query",
        &json!({"sql":"SELECT 1","limit":1}),
    )
    .await
}
pub async fn execute(
    ctx: &crate::harness::HarnessCtx<'_>,
    alias: &str,
    args: Value,
) -> Result<Value, String> {
    let id = args["profile_id"]
        .as_str()
        .ok_or("Missing database profile ID.")?;
    let p = load(ctx.state, id)?;
    if alias == "database_rehearse" {
        return crate::database_rehearsal::execute(ctx, &p, args).await;
    }
    tokio::time::timeout(
        std::time::Duration::from_secs(15),
        query(ctx.state, &p, alias, &args),
    )
    .await
    .map_err(|_| {
        "Database request timed out; read-only transaction/connection dropped.".to_string()
    })?
}

#[cfg(test)]
mod acceptance {
    use super::*;
    #[tokio::test]
    #[ignore = "Requires a disposable PostgreSQL fixture and explicit LOCALLM_ACCEPTANCE_PG_* variables."]
    async fn real_postgres_readonly_and_disposable_rehearsal() {
        let t = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(t.path());
        let secret = std::env::var("LOCALLM_ACCEPTANCE_PG_PASSWORD")
            .expect("Set disposable fixture password; never use production profiles");
        let administrator = Profile {
            id: "dev-db-acceptance-admin".into(),
            name: "Acceptance scratch".into(),
            engine: "postgres".into(),
            host: "127.0.0.1".into(),
            port: std::env::var("LOCALLM_ACCEPTANCE_PG_PORT")
                .expect("Set disposable PostgreSQL port")
                .parse()
                .unwrap(),
            database: "locallm_acceptance".into(),
            username: "postgres".into(),
            tls: "disable".into(),
            scratch_profile_id: None,
            dump_executable: String::new(),
            restore_executable: String::new(),
        };
        let mut source = administrator.clone();
        source.id = "dev-db-acceptance-source".into();
        source.name = "Acceptance source".into();
        source.scratch_profile_id = Some(administrator.id.clone());
        source.dump_executable = std::env::var("LOCALLM_ACCEPTANCE_PG_DUMP")
            .expect("Set installed pg_dump or owned fixture wrapper");
        source.restore_executable = std::env::var("LOCALLM_ACCEPTANCE_PG_RESTORE")
            .expect("Set installed psql or owned fixture wrapper");
        state
            .database()
            .unwrap()
            .save_setting(KEY, &vec![source.clone(), administrator.clone()])
            .unwrap();
        for p in [&source, &administrator] {
            state.daytona_vault.save(&p.id, secret.as_bytes()).unwrap();
        }
        let mut c = PgConnection::connect_with(&pg_options(&administrator, &secret))
            .await
            .unwrap();
        sqlx_core::query::query::<Postgres>(
            "CREATE TABLE source_guard(id INTEGER PRIMARY KEY, name TEXT NOT NULL)",
        )
        .execute(&mut c)
        .await
        .unwrap();
        sqlx_core::query::query::<Postgres>("INSERT INTO source_guard VALUES (1,'original')")
            .execute(&mut c)
            .await
            .unwrap();
        assert_eq!(
            query(
                &state,
                &source,
                "database_query",
                &json!({"sql":"SELECT name FROM source_guard WHERE id=$1","parameters":[1]})
            )
            .await
            .unwrap()["rows"][0][0],
            "original"
        );
        assert!(query(
            &state,
            &source,
            "database_query",
            &json!({"sql":"DELETE FROM source_guard"})
        )
        .await
        .is_err());
        assert!(query(
            &state,
            &source,
            "database_query",
            &json!({"sql":"SELECT 1; DELETE FROM source_guard"})
        )
        .await
        .is_err());
        let owner = state.database().unwrap().create_conversation().unwrap().id;
        let tools = crate::development_tools::registry()
            .iter()
            .map(|t| crate::connectors::AgentTool::harness(&t.name).unwrap())
            .collect::<Vec<_>>();
        let ctx = crate::development_tools::tests::context(&state, t.path(), &owner, &tools);
        let result=crate::development_tools::execute(&ctx,"database_rehearse",json!({"profile_id":source.id,"sql":"ALTER TABLE source_guard ADD COLUMN rehearsed TEXT; INSERT INTO source_guard(id,name) VALUES (2,'scratch');"})).await.unwrap();
        assert_eq!(result["isError"], false, "{result}");
        assert_eq!(result["cleanupComplete"], true, "{result}");
        assert_eq!(result["originalModified"], false);
        assert_ne!(result["cloneSha256"], dev::hash(source.id.as_bytes()));
        let rows = query(
            &state,
            &source,
            "database_query",
            &json!({"sql":"SELECT * FROM source_guard"}),
        )
        .await
        .unwrap();
        assert_eq!(rows["columns"].as_array().unwrap().len(), 2);
        assert_eq!(rows["rows"].as_array().unwrap().len(), 1);
        let remaining = sqlx_core::query::query::<Postgres>(
            "SELECT datname FROM pg_database WHERE datname LIKE 'locallm_rehearsal_%'",
        )
        .fetch_all(&mut c)
        .await
        .unwrap();
        assert!(remaining.is_empty());
        let failed = crate::database_rehearsal::execute(&ctx, &source, json!({"sql":"ALTER TABLE source_guard ADD COLUMN partial TEXT; INSERT INTO source_guard(id,name) VALUES (1,'duplicate');"})).await.unwrap();
        assert_eq!(failed["isError"], true);
        assert_eq!(failed["cleanupComplete"], true);
        assert_eq!(failed["originalModified"], false);
        assert_eq!(
            crate::database_rehearsal::cleanup_tool(
                &ctx,
                json!({"rehearsal_id":failed["rehearsalId"]})
            )
            .await
            .unwrap()["cleanupComplete"],
            true
        );
        let lost = crate::database_rehearsal::execute(
            &ctx,
            &source,
            json!({"sql":"SELECT pg_terminate_backend(pg_backend_pid())"}),
        )
        .await
        .unwrap();
        assert_eq!(lost["isError"], true);
        assert_eq!(lost["cleanupComplete"], true);
        assert_eq!(lost["originalModified"], false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let adapter = t.path().join("partial-dump.py");
            let python = crate::execution::ExecutionConfig::default().python_path;
            std::fs::write(&adapter,format!("#!{python}\nimport subprocess,sys\np=subprocess.run([{}]+sys.argv[1:],stdout=subprocess.PIPE)\nsys.stdout.buffer.write(p.stdout);sys.stdout.buffer.write(b'\\nINVALID_RESTORE_STATEMENT;\\n');sys.exit(p.returncode)\n",serde_json::to_string(&source.dump_executable).unwrap())).unwrap();
            std::fs::set_permissions(&adapter, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut partial = source.clone();
            partial.dump_executable = adapter.to_string_lossy().into();
            let result =
                crate::database_rehearsal::execute(&ctx, &partial, json!({"sql":"SELECT 1"}))
                    .await
                    .unwrap();
            assert_eq!(result["isError"], true);
            assert_eq!(result["cleanupComplete"], true);
            assert_eq!(result["originalModified"], false);
        }
        let metadata=crate::database_rehearsal::execute(&ctx,&source,json!({"sql":"ALTER TABLE source_guard ADD CONSTRAINT unvalidated_guard CHECK(id>0) NOT VALID;"})).await.unwrap();
        assert_eq!(metadata["isError"], false, "{metadata}");
        assert!(metadata["result"]["after"]["constraints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "unvalidated_guard" && c["validated"] == false));
        // Interrupt a live owned rehearsal, discard in-memory state, then
        // recover from durable intent even without creation acknowledgements.
        let mut operation = Box::pin(crate::database_rehearsal::execute(
            &ctx,
            &source,
            json!({"sql":"SELECT pg_sleep(4)"}),
        ));
        let interrupted = tokio::select! {
            result = &mut operation => panic!("Rehearsal unexpectedly completed before interruption: {result:?}"),
            id = tokio::time::timeout(std::time::Duration::from_secs(30), async {
                loop {
                    let ids: Vec<String> = state.database().unwrap().setting("development.rehearsal_index").unwrap();
                    for id in ids {
                        let ledger: Value = state.database().unwrap().setting(&format!("development.rehearsal.{id}")).unwrap();
                        if ledger["status"] == "restored" { return id; }
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }) => id.unwrap(),
        };
        drop(operation);
        let mut ledger: Value = state
            .database()
            .unwrap()
            .setting(&format!("development.rehearsal.{interrupted}"))
            .unwrap();
        ledger["databaseCreated"] = json!(false);
        ledger["roleCreated"] = json!(false);
        ledger["databaseMayExist"] = json!(true);
        ledger["roleMayExist"] = json!(true);
        state
            .database()
            .unwrap()
            .save_rehearsal_ledger(&interrupted, &ledger)
            .unwrap();
        let restarted = crate::test_support::app_state(t.path());
        let restarted_ctx =
            crate::development_tools::tests::context(&restarted, t.path(), &owner, &[]);
        let pending = crate::database_rehearsal::cleanup_tool(&restarted_ctx, json!({}))
            .await
            .unwrap();
        assert!(pending["pendingRehearsals"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["rehearsalId"] == interrupted));
        let marker = ledger["ownershipMarker"].clone();
        ledger["ownershipMarker"] = json!("foreign-marker");
        restarted
            .database()
            .unwrap()
            .save_rehearsal_ledger(&interrupted, &ledger)
            .unwrap();
        let denied = crate::database_rehearsal::cleanup_tool(
            &restarted_ctx,
            json!({"rehearsal_id":interrupted}),
        )
        .await
        .unwrap();
        assert_eq!(
            denied["isError"], true,
            "Unproven remote ownership must never authorize deletion"
        );
        assert_eq!(denied["rehearsalId"], interrupted);
        ledger["ownershipMarker"] = marker;
        restarted
            .database()
            .unwrap()
            .save_rehearsal_ledger(&interrupted, &ledger)
            .unwrap();
        for _ in 0..2 {
            assert_eq!(
                crate::database_rehearsal::cleanup_tool(
                    &restarted_ctx,
                    json!({"rehearsal_id":interrupted})
                )
                .await
                .unwrap()["cleanupComplete"],
                true
            );
        }
        let remaining = sqlx_core::query::query::<Postgres>(
            "SELECT datname FROM pg_database WHERE datname LIKE 'locallm_rehearsal_%'",
        )
        .fetch_all(&mut c)
        .await
        .unwrap();
        assert!(remaining.is_empty());
        sqlx_core::query::query::<Postgres>("DROP TABLE source_guard")
            .execute(&mut c)
            .await
            .unwrap();
    }
    #[tokio::test]
    #[ignore = "Requires an isolated disposable MySQL 8 fixture configured with LOCALLM_ACCEPTANCE_MYSQL_*."]
    async fn real_mysql_readonly_and_disposable_rehearsal() {
        let t = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(t.path());
        let secret = std::env::var("LOCALLM_ACCEPTANCE_MYSQL_PASSWORD")
            .expect("Use only an isolated disposable fixture");
        let admin = Profile {
            id: "dev-db-mysql-admin".into(),
            name: "Isolated fixture admin".into(),
            engine: "mysql".into(),
            host: "127.0.0.1".into(),
            port: std::env::var("LOCALLM_ACCEPTANCE_MYSQL_PORT")
                .expect("Configure disposable fixture port")
                .parse()
                .unwrap(),
            database: "locallm_acceptance".into(),
            username: "root".into(),
            tls: "disable".into(),
            scratch_profile_id: None,
            dump_executable: String::new(),
            restore_executable: String::new(),
        };
        let mut source = admin.clone();
        source.id = "dev-db-mysql-source".into();
        source.scratch_profile_id = Some(admin.id.clone());
        source.dump_executable =
            std::env::var("LOCALLM_ACCEPTANCE_MYSQL_DUMP").expect("Configure installed mysqldump");
        source.restore_executable =
            std::env::var("LOCALLM_ACCEPTANCE_MYSQL_RESTORE").expect("Configure installed mysql");
        state
            .database()
            .unwrap()
            .save_setting(KEY, &vec![source.clone(), admin.clone()])
            .unwrap();
        for p in [&source, &admin] {
            state.daytona_vault.save(&p.id, secret.as_bytes()).unwrap();
        }
        let mut connection = MySqlConnection::connect_with(&mysql_options(&admin, &secret))
            .await
            .unwrap();
        sqlx_core::query::query::<MySql>("CREATE TABLE source_guard(id INTEGER PRIMARY KEY, name VARCHAR(128) NOT NULL, CONSTRAINT positive_id CHECK(id>0))").execute(&mut connection).await.unwrap();
        sqlx_core::query::query::<MySql>("INSERT INTO source_guard VALUES (1,'original')")
            .execute(&mut connection)
            .await
            .unwrap();
        assert_eq!(
            query(
                &state,
                &source,
                "database_query",
                &json!({"sql":"SELECT name FROM source_guard WHERE id=?","parameters":[1]})
            )
            .await
            .unwrap()["rows"][0][0],
            "original"
        );
        assert!(query(
            &state,
            &source,
            "database_query",
            &json!({"sql":"DELETE FROM source_guard"})
        )
        .await
        .is_err());
        let owner = state.database().unwrap().create_conversation().unwrap().id;
        let ctx = crate::development_tools::tests::context(&state, t.path(), &owner, &[]);
        let result=crate::database_rehearsal::execute(&ctx,&source,json!({"sql":"ALTER TABLE source_guard ADD COLUMN rehearsed TEXT; INSERT INTO source_guard(id,name) VALUES(2,'scratch');"})).await.unwrap();
        assert_eq!(result["isError"], false, "{result}");
        assert_eq!(result["cleanupComplete"], true);
        assert_eq!(result["originalModified"], false);
        assert_eq!(result["result"]["ddlImplicitCommit"], true);
        assert!(!result["result"]["after"]["constraints"]
            .as_array()
            .unwrap()
            .is_empty());
        let failed=crate::database_rehearsal::execute(&ctx,&source,json!({"sql":"ALTER TABLE source_guard ADD COLUMN partial TEXT; INSERT INTO source_guard(id,name) VALUES(-1,'constraint failure');"})).await.unwrap();
        assert_eq!(failed["isError"], true);
        assert_eq!(failed["cleanupComplete"], true);
        assert_eq!(failed["originalModified"], false);
        let repeated = crate::database_rehearsal::cleanup_tool(
            &ctx,
            json!({"rehearsal_id":failed["rehearsalId"]}),
        )
        .await
        .unwrap();
        assert_eq!(repeated["cleanupComplete"], true);
        let rows = query(
            &state,
            &source,
            "database_query",
            &json!({"sql":"SELECT * FROM source_guard"}),
        )
        .await
        .unwrap();
        assert_eq!(rows["rows"].as_array().unwrap().len(), 1);
        assert_eq!(rows["columns"].as_array().unwrap().len(), 2);
        sqlx_core::query::query::<MySql>("DROP TABLE source_guard")
            .execute(&mut connection)
            .await
            .unwrap();
    }
}
