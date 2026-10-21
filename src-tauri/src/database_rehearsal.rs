//! Disposable remote database rehearsals with durable resource ownership.
use crate::{development_tools as dev, remote_database::Profile};
use futures_util::TryStreamExt;
use serde_json::{json, Value};
use sqlx_core::{connection::Connection, executor::Executor};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
async fn execute_pg(
    c: &mut sqlx_postgres::PgConnection,
    sql: &str,
) -> Result<(), sqlx_core::error::Error> {
    let mut results = c.execute_many(sql);
    while results.try_next().await?.is_some() {}
    Ok(())
}
async fn execute_mysql(
    c: &mut sqlx_mysql::MySqlConnection,
    sql: &str,
) -> Result<(), sqlx_core::error::Error> {
    let mut results = c.execute_many(sql);
    while results.try_next().await?.is_some() {}
    Ok(())
}
async fn admin(state: &crate::AppState, p: &Profile, sql: &str) -> Result<(), String> {
    tokio::time::timeout(std::time::Duration::from_secs(30), admin_bounded(state, p, sql)).await.map_err(|_| "Scratch administration timed out; acknowledgement is uncertain; discover the rehearsal with database_cleanup.")?
}
async fn admin_bounded(state: &crate::AppState, p: &Profile, sql: &str) -> Result<(), String> {
    let secret = crate::remote_database::password(state, p)?;
    if p.engine == "postgres" {
        let mut c = sqlx_postgres::PgConnection::connect_with(&crate::remote_database::pg_options(
            p, &secret,
        ))
        .await
        .map_err(|_| "Scratch PostgreSQL administration connection failed.")?;
        execute_pg(&mut c, sql).await.map_err(|_| {
            "Scratch PostgreSQL administration operation failed; check configured privileges."
        })?;
    } else {
        let mut c = sqlx_mysql::MySqlConnection::connect_with(
            &crate::remote_database::mysql_options(p, &secret),
        )
        .await
        .map_err(|_| "Scratch MySQL administration connection failed.")?;
        execute_mysql(&mut c, sql).await.map_err(|_| {
            "Scratch MySQL administration operation failed; check configured privileges."
        })?;
    }
    Ok(())
}
fn key(id: &str) -> Result<String, String> {
    uuid::Uuid::parse_str(id).map_err(|_| "Invalid rehearsal ID.")?;
    Ok(format!("development.rehearsal.{id}"))
}
fn persist(state: &crate::AppState, id: &str, ledger: &Value) -> Result<(), String> {
    state.database()?.save_rehearsal_ledger(id, ledger)
}
// Remote ownership markers are created atomically with the restricted role.
// Cleanup rechecks them even when an earlier creation acknowledgement was lost.
async fn owned_resources(
    state: &crate::AppState,
    p: &Profile,
    ledger: &Value,
) -> Result<(bool, bool), String> {
    use sqlx_core::row::Row;
    let database = ledger["database"]
        .as_str()
        .ok_or("Missing generated database.")?;
    let role = ledger["role"].as_str().ok_or("Missing generated role.")?;
    let marker = ledger["ownershipMarker"].as_str().ok_or("Legacy rehearsal lacks a remote ownership marker; automatic destructive cleanup is refused.")?;
    let secret = crate::remote_database::password(state, p)?;
    let operation = async {
        if p.engine == "postgres" {
            let mut c = sqlx_postgres::PgConnection::connect_with(
                &crate::remote_database::pg_options(p, &secret),
            )
            .await
            .map_err(|_| "Cannot verify remote PostgreSQL ownership.")?;
            let role_row = sqlx_core::query::query::<sqlx_postgres::Postgres>("SELECT shobj_description(oid,'pg_authid') AS marker FROM pg_roles WHERE rolname=$1").bind(role).fetch_optional(&mut c).await.map_err(|_| "Cannot inspect restricted role ownership marker.")?;
            if let Some(row) = &role_row {
                if row
                    .try_get::<Option<String>, _>("marker")
                    .ok()
                    .flatten()
                    .as_deref()
                    != Some(marker)
                {
                    return Err("Remote role ownership marker mismatch; resource preserved.".into());
                }
            }
            let db_row = sqlx_core::query::query::<sqlx_postgres::Postgres>(
                "SELECT pg_get_userbyid(datdba) AS owner FROM pg_database WHERE datname=$1",
            )
            .bind(database)
            .fetch_optional(&mut c)
            .await
            .map_err(|_| "Cannot inspect scratch database ownership.")?;
            if let Some(row) = &db_row {
                if role_row.is_none()
                    || row.try_get::<String, _>("owner").ok().as_deref() != Some(role)
                    || ledger["databaseAbsentBeforeCreate"] != true
                {
                    return Err(
                        "Remote database ownership cannot be established; resource preserved."
                            .into(),
                    );
                }
            }
            Ok((db_row.is_some(), role_row.is_some()))
        } else {
            let mut c = sqlx_mysql::MySqlConnection::connect_with(
                &crate::remote_database::mysql_options(p, &secret),
            )
            .await
            .map_err(|_| "Cannot verify remote MySQL ownership.")?;
            let role_row = sqlx_core::query::query::<sqlx_mysql::MySql>("SELECT JSON_UNQUOTE(JSON_EXTRACT(ATTRIBUTE,'$.locallmOwner')) AS marker FROM information_schema.USER_ATTRIBUTES WHERE USER=? AND HOST='%'").bind(role).fetch_optional(&mut c).await.map_err(|_| "MySQL 8.0.21+ user ownership metadata is required for cleanup.")?;
            if let Some(row) = &role_row {
                if row
                    .try_get::<Option<String>, _>("marker")
                    .ok()
                    .flatten()
                    .as_deref()
                    != Some(marker)
                {
                    return Err(
                        "Remote MySQL role ownership marker mismatch; resource preserved.".into(),
                    );
                }
            }
            let exists = sqlx_core::query::query::<sqlx_mysql::MySql>(
                "SELECT schema_name FROM information_schema.schemata WHERE schema_name=?",
            )
            .bind(database)
            .fetch_optional(&mut c)
            .await
            .map_err(|_| "Cannot inspect scratch MySQL database.")?
            .is_some();
            if exists && (role_row.is_none() || ledger["databaseAbsentBeforeCreate"] != true) {
                return Err(
                    "Remote MySQL database ownership cannot be established; resource preserved."
                        .into(),
                );
            }
            Ok((exists, role_row.is_some()))
        }
    };
    tokio::time::timeout(std::time::Duration::from_secs(15), operation)
        .await
        .map_err(|_| "Remote ownership inspection timed out; no cleanup authorized.")?
}
async fn cleanup(ctx: &crate::harness::HarnessCtx<'_>, id: &str) -> Result<Value, String> {
    if ctx.state.database()?.setting::<bool>("offline_mode")? {
        return Err("Offline mode blocks remote cleanup.".into());
    }
    let mut ledger: Value = ctx.state.database()?.setting(&key(id)?)?;
    if ledger["owner"] != ctx.conversation_id
        || ledger["root"] != dev::canonical(&ctx.snapshot.workspace_path)?
    {
        return Err("Rehearsal belongs to another conversation/workspace.".into());
    }
    let p = crate::remote_database::load(
        ctx.state,
        ledger["adminProfileId"]
            .as_str()
            .ok_or("Rehearsal lacks administration profile.")?,
    )?;
    if dev::hash(
        serde_json::to_vec(&p)
            .map_err(|e| e.to_string())?
            .as_slice(),
    ) != ledger["adminFingerprint"]
    {
        return Err(
            "Scratch administration settings changed; restore the recorded profile before cleanup."
                .into(),
        );
    }
    let uuid = uuid::Uuid::parse_str(id)
        .map_err(|e| e.to_string())?
        .simple()
        .to_string();
    let database = format!("locallm_rehearsal_{uuid}");
    let role = format!("llm_{}", &uuid[..24]);
    if ledger["database"] != database || ledger["role"] != role {
        return Err("Invalid scratch resource ownership record.".into());
    }
    if ledger["status"] == "cleaned" {
        return Ok(json!({"rehearsalId":id,"cleanupComplete":true}));
    }
    let (database_exists, role_exists) = owned_resources(ctx.state, &p, &ledger).await?;
    if database_exists && ledger["databaseCreated"] != true && ledger["databaseMayExist"] != true {
        return Err("Database creation intent is absent; resource preserved.".into());
    }
    if role_exists && ledger["roleCreated"] != true && ledger["roleMayExist"] != true {
        return Err("Role creation intent is absent; resource preserved.".into());
    }
    if (ledger["databaseCreated"] == true || ledger["databaseMayExist"] == true) && database_exists
    {
        let sql = if p.engine == "postgres" {
            format!("DROP DATABASE IF EXISTS \"{database}\" WITH (FORCE)")
        } else {
            format!("DROP DATABASE IF EXISTS `{database}`")
        };
        admin(ctx.state, &p, &sql).await?;
        ledger["databaseCreated"] = json!(false);
        ledger["databaseMayExist"] = json!(false);
        persist(ctx.state, id, &ledger)?;
    }
    if (ledger["roleCreated"] == true || ledger["roleMayExist"] == true) && role_exists {
        let sql = if p.engine == "postgres" {
            format!("DROP ROLE IF EXISTS \"{role}\"")
        } else {
            format!("DROP USER IF EXISTS '{role}'@'%'")
        };
        admin(ctx.state, &p, &sql).await?;
        ledger["roleCreated"] = json!(false);
        ledger["roleMayExist"] = json!(false);
        persist(ctx.state, id, &ledger)?;
    }
    if database_exists && ledger["databaseCreated"] != false && ledger["databaseMayExist"] != false
    {
        return Err("Scratch database cleanup is not confirmed.".into());
    }
    ledger["databaseCreated"] = json!(false);
    ledger["databaseMayExist"] = json!(false);
    ledger["roleCreated"] = json!(false);
    ledger["roleMayExist"] = json!(false);
    ledger["status"] = json!("cleaned");
    persist(ctx.state, id, &ledger)?;
    Ok(json!({"rehearsalId":id,"cleanupComplete":true}))
}
async fn cli(
    ctx: &crate::harness::HarnessCtx<'_>,
    p: &Profile,
    executable: &str,
    input: Option<Vec<u8>>,
    dump: bool,
    override_secret: Option<&str>,
) -> Result<Vec<u8>, String> {
    if !crate::execution::executable(executable) {
        return Err("Configure an installed absolute database dump/restore executable in Development tools settings.".into());
    }
    let config = ctx.state.database()?.execution_config()?;
    if !config.bash.enabled {
        return Err("Database clone utilities require enabled shell execution.".into());
    }
    let password = match override_secret {
        Some(value) => value.to_string(),
        None => crate::remote_database::password(ctx.state, p)?,
    };
    let mut cmd = tokio::process::Command::new(executable);
    cmd.env_clear()
        .current_dir(&ctx.snapshot.workspace_path)
        .kill_on_drop(true)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    for k in ["PATH", "SystemRoot", "WINDIR", "HOME", "TEMP", "TMP"] {
        if let Some(v) = std::env::var_os(k) {
            cmd.env(k, v);
        }
    }
    if p.engine == "postgres" {
        cmd.env("PGPASSWORD", &password).env(
            "PGSSLMODE",
            if p.tls == "disable" {
                "disable"
            } else if p.tls == "require" {
                "require"
            } else {
                "verify-full"
            },
        );
        cmd.args([
            "--host",
            &p.host,
            "--port",
            &p.port.to_string(),
            "--username",
            &p.username,
            "--dbname",
            &p.database,
            "--no-password",
        ]);
        if dump {
            cmd.args([
                "--format=plain",
                "--no-owner",
                "--no-privileges",
                "--no-comments",
                "--no-security-labels",
                "--no-tablespaces",
            ]);
        } else {
            cmd.args(["--no-psqlrc", "--set=ON_ERROR_STOP=1"]);
        }
    } else {
        cmd.env("MYSQL_PWD", &password);
        cmd.args([
            format!("--host={}", p.host),
            format!("--port={}", p.port),
            format!("--user={}", p.username),
            format!(
                "--ssl-mode={}",
                if p.tls == "disable" {
                    "DISABLED"
                } else if p.tls == "require" {
                    "REQUIRED"
                } else {
                    "VERIFY_IDENTITY"
                }
            ),
        ]);
        if dump {
            cmd.args([
                "--single-transaction",
                "--skip-lock-tables",
                "--skip-comments",
                "--no-tablespaces",
                "--set-gtid-purged=OFF",
            ]);
            cmd.arg(&p.database);
        } else {
            cmd.arg(format!("--database={}", p.database));
        }
    }
    crate::execution::memory_limit(&mut cmd, &config.bash);
    let mut child = crate::execution::managed_spawn(cmd)
        .map_err(|_| "Could not start database clone utility.".to_string())?;
    let mut stdin = child
        .stdin()
        .take()
        .ok_or("Database clone utility has no stdin.")?;
    let stdout = child
        .stdout()
        .take()
        .ok_or("Database clone utility has no stdout.")?;
    let stderr = child
        .stderr()
        .take()
        .ok_or("Database clone utility has no stderr.")?;
    let operation = async {
        let write = async {
            if let Some(input) = input {
                stdin
                    .write_all(&input)
                    .await
                    .map_err(|_| "Database clone utility input failed.")?;
            }
            drop(stdin);
            Ok::<_, String>(())
        };
        let read = async {
            let mut output = Vec::new();
            stdout
                .take(32 * 1024 * 1024 + 1)
                .read_to_end(&mut output)
                .await
                .map_err(|_| "Database clone output failed.")?;
            Ok::<_, String>(output)
        };
        let errors = async {
            let mut bytes = Vec::new();
            stderr
                .take(65537)
                .read_to_end(&mut bytes)
                .await
                .map_err(|_| "Database clone diagnostics failed.")?;
            Ok::<_, String>(bytes)
        };
        let (_, output, errors) = tokio::try_join!(write, read, errors)?;
        if output.len() > 32 * 1024 * 1024 || errors.len() > 65536 {
            return Err("Database clone utility exceeded output limits.".into());
        }
        let status = child
            .wait()
            .await
            .map_err(|_| "Database clone utility wait failed.")?;
        if !status.success() {
            return Err("Database clone utility failed; verify executable compatibility, TLS, privileges and scratch-only object references.".into());
        }
        Ok(output)
    };
    tokio::time::timeout(
        std::time::Duration::from_secs(config.bash.timeout_seconds.min(120)),
        operation,
    )
    .await
    .map_err(|_| "Database clone utility timed out; resource cleanup may be needed.")?
}
async fn pg_schema(connection: &mut sqlx_postgres::PgConnection) -> Result<Value, String> {
    use sqlx_core::row::Row;
    let columns=sqlx_core::query::query::<sqlx_postgres::Postgres>("SELECT table_schema,table_name,column_name,data_type,is_nullable,column_default FROM information_schema.columns WHERE table_schema NOT IN ('pg_catalog','information_schema') ORDER BY table_schema,table_name,ordinal_position LIMIT 4097").fetch_all(&mut *connection).await.map_err(|_|"Cannot inspect scratch PostgreSQL columns.")?;
    if columns.len() > 4096 {
        return Err("Schema inspection exceeds 4096 columns.".into());
    }
    let columns = columns
        .iter()
        .map(|row| {
            let mut value = serde_json::Map::new();
            for key in [
                "table_schema",
                "table_name",
                "column_name",
                "data_type",
                "is_nullable",
                "column_default",
            ] {
                value.insert(
                    key.into(),
                    row.try_get::<Option<String>, _>(key)
                        .ok()
                        .flatten()
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                );
            }
            Value::Object(value)
        })
        .collect::<Vec<_>>();
    let constraints=sqlx_core::query::query::<sqlx_postgres::Postgres>("SELECT n.nspname AS schema,c.conrelid::regclass::text AS relation,c.conname,c.convalidated,c.condeferrable,c.condeferred,pg_get_constraintdef(c.oid) AS definition FROM pg_constraint c JOIN pg_namespace n ON n.oid=c.connamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') ORDER BY n.nspname,c.conrelid,c.conname LIMIT 4097").fetch_all(&mut *connection).await.map_err(|_|"Cannot inspect scratch PostgreSQL constraints.")?;
    if constraints.len() > 4096 {
        return Err("Schema inspection exceeds 4096 constraints.".into());
    }
    let constraints=constraints.iter().map(|row|json!({"schema":row.try_get::<String,_>("schema").ok(),"relation":row.try_get::<String,_>("relation").ok(),"deferrable":row.try_get::<bool,_>("condeferrable").ok(),"initiallyDeferred":row.try_get::<bool,_>("condeferred").ok(),"name":row.try_get::<String,_>("conname").ok(),"validated":row.try_get::<bool,_>("convalidated").ok(),"definition":row.try_get::<String,_>("definition").ok()})).collect::<Vec<_>>();
    let disabled = sqlx_core::query::query::<sqlx_postgres::Postgres>("SELECT n.nspname AS schema,c.relname AS relation,t.tgname AS name,t.tgenabled::text AS enabled FROM pg_trigger t JOIN pg_class c ON c.oid=t.tgrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') AND t.tgenabled <> 'O' ORDER BY n.nspname,c.relname,t.tgname LIMIT 4097").fetch_all(&mut *connection).await.map_err(|_| "Cannot inspect PostgreSQL trigger enforcement.")?;
    if disabled.len() > 4096 {
        return Err("Trigger metadata exceeds budget.".into());
    }
    let disabled = disabled.iter().map(|r| json!({"schema":r.try_get::<String,_>("schema").ok(),"relation":r.try_get::<String,_>("relation").ok(),"name":r.try_get::<String,_>("name").ok(),"enabled":r.try_get::<String,_>("enabled").ok()})).collect::<Vec<_>>();
    let indexes=sqlx_core::query::query::<sqlx_postgres::Postgres>("SELECT n.nspname AS schema,c.relname AS relation,i.indisvalid,i.indisready,pg_get_indexdef(i.indexrelid) AS definition FROM pg_index i JOIN pg_class c ON c.oid=i.indrelid JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname NOT IN ('pg_catalog','information_schema') ORDER BY n.nspname,c.relname,i.indexrelid LIMIT 4097").fetch_all(&mut *connection).await.map_err(|_| "Cannot inspect PostgreSQL index validity.")?;
    if indexes.len() > 4096 {
        return Err("PostgreSQL index metadata exceeds budget.".into());
    }
    let indexes=indexes.iter().map(|r|json!({"schema":r.try_get::<String,_>("schema").ok(),"relation":r.try_get::<String,_>("relation").ok(),"valid":r.try_get::<bool,_>("indisvalid").ok(),"ready":r.try_get::<bool,_>("indisready").ok(),"definition":r.try_get::<String,_>("definition").ok()})).collect::<Vec<_>>();
    Ok(
        json!({"columns":columns,"constraints":constraints,"indexes":indexes,"nonDefaultTriggerEnforcement":disabled,"integrityScope":"bounded constraint validation/enforcement metadata; no physical corruption scan or exhaustive row scan"}),
    )
}
async fn mysql_schema(connection: &mut sqlx_mysql::MySqlConnection) -> Result<Value, String> {
    use sqlx_core::row::Row;
    let columns=sqlx_core::query::query::<sqlx_mysql::MySql>("SELECT table_name,column_name,column_type,is_nullable,column_default FROM information_schema.columns WHERE table_schema=DATABASE() ORDER BY table_name,ordinal_position LIMIT 4097").fetch_all(&mut *connection).await.map_err(|_|"Cannot inspect scratch MySQL schema.")?;
    if columns.len() > 4096 {
        return Err("MySQL schema exceeds 4096 columns.".into());
    }
    let constraints = sqlx_core::query::query::<sqlx_mysql::MySql>("SELECT table_name,constraint_name,constraint_type,enforced FROM information_schema.table_constraints WHERE constraint_schema=DATABASE() ORDER BY table_name,constraint_name LIMIT 4097").fetch_all(&mut *connection).await.map_err(|_| "Cannot inspect MySQL constraint enforcement; MySQL 8.0 metadata is required.")?;
    if constraints.len() > 4096 {
        return Err("MySQL schema exceeds 4096 constraints.".into());
    }
    let constraints = constraints.iter().map(|r| json!({"table":r.try_get::<String,_>("table_name").ok(),"name":r.try_get::<String,_>("constraint_name").ok(),"type":r.try_get::<String,_>("constraint_type").ok(),"enforced":r.try_get::<String,_>("enforced").ok()})).collect::<Vec<_>>();
    let enforcement=sqlx_core::query::query::<sqlx_mysql::MySql>("SELECT @@SESSION.foreign_key_checks AS foreign_key_checks,@@SESSION.unique_checks AS unique_checks,@@SESSION.sql_mode AS sql_mode").fetch_one(&mut *connection).await.map_err(|_| "Cannot inspect MySQL session constraint enforcement.")?;
    let enforcement = json!({"foreignKeyChecks":enforcement.try_get::<u64,_>("foreign_key_checks").ok(),"uniqueChecks":enforcement.try_get::<u64,_>("unique_checks").ok(),"sqlMode":enforcement.try_get::<String,_>("sql_mode").ok()});
    async fn metadata(
        connection: &mut sqlx_mysql::MySqlConnection,
        sql: &str,
    ) -> Result<Vec<Value>, String> {
        use sqlx_core::{column::Column, row::Row};
        let rows = sqlx_core::query::query::<sqlx_mysql::MySql>(sql)
            .fetch_all(connection)
            .await
            .map_err(|_| "Cannot inspect bounded MySQL constraint details.")?;
        if rows.len() > 4096 {
            return Err("MySQL constraint metadata exceeds 4096 rows.".into());
        }
        Ok(rows
            .iter()
            .map(|row| {
                Value::Object(
                    row.columns()
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            (
                                c.name().to_string(),
                                row.try_get::<Option<String>, _>(i)
                                    .ok()
                                    .flatten()
                                    .map(Value::String)
                                    .unwrap_or(Value::Null),
                            )
                        })
                        .collect(),
                )
            })
            .collect())
    }
    let keys=metadata(connection,"SELECT table_name,column_name,constraint_name,CAST(ordinal_position AS CHAR) AS ordinal_position,referenced_table_name,referenced_column_name FROM information_schema.key_column_usage WHERE constraint_schema=DATABASE() ORDER BY table_name,constraint_name,ordinal_position LIMIT 4097").await?;
    let foreign_keys=metadata(connection,"SELECT table_name,constraint_name,referenced_table_name,match_option,update_rule,delete_rule FROM information_schema.referential_constraints WHERE constraint_schema=DATABASE() ORDER BY table_name,constraint_name LIMIT 4097").await?;
    let checks=metadata(connection,"SELECT constraint_name,check_clause FROM information_schema.check_constraints WHERE constraint_schema=DATABASE() ORDER BY constraint_name LIMIT 4097").await?;
    let engines=metadata(connection,"SELECT table_name,engine FROM information_schema.tables WHERE table_schema=DATABASE() AND table_type='BASE TABLE' ORDER BY table_name LIMIT 4097").await?;
    let mut integrity = Vec::new();
    for table in engines.iter().take(16) {
        let table = table["table_name"]
            .as_str()
            .or_else(|| table["TABLE_NAME"].as_str())
            .ok_or("MySQL table name metadata unavailable.")?;
        let sql = format!("CHECK TABLE `{}` QUICK", table.replace('`', "``"));
        let rows = sqlx_core::query::query::<sqlx_mysql::MySql>(&sql)
            .fetch_all(&mut *connection)
            .await
            .map_err(|_| "Bounded MySQL QUICK integrity check failed.")?;
        if rows.len() > 16 {
            return Err("MySQL QUICK check exceeds result bound.".into());
        }
        for row in rows {
            integrity.push(json!({"table":table,"operation":row.try_get::<String,_>(1).ok(),"messageType":row.try_get::<String,_>(2).ok(),"message":row.try_get::<String,_>(3).ok().map(|s|s.chars().take(1000).collect::<String>())}));
        }
    }
    Ok(
        json!({"sessionEnforcement":enforcement,"constraints":constraints,"keyColumns":keys,"foreignKeys":foreign_keys,"checks":checks,"tableEngines":engines,"quickIntegrity":integrity,"quickIntegrityCoverageComplete":engines.len()<=16,"integrityScope":"constraint metadata and engine QUICK checks on at most 16 scratch tables; not exhaustive physical corruption scanning or complete row validation","columns":columns.iter().map(|row|{let mut value=serde_json::Map::new();for key in ["table_name","column_name","column_type","is_nullable","column_default"]{value.insert(key.into(),row.try_get::<Option<String>,_>(key).ok().flatten().map(Value::String).unwrap_or(Value::Null));}Value::Object(value)}).collect::<Vec<_>>()}),
    )
}
async fn source_evidence(state: &crate::AppState, source: &Profile) -> Result<Value, String> {
    let operation = async {
        let schema =
            crate::remote_database::query(state, source, "database_schema", &json!({"limit":200}))
                .await?;
        let mut tables = std::collections::BTreeSet::new();
        for row in schema["rows"].as_array().into_iter().flatten() {
            let namespace = row[0].as_str().ok_or("Schema namespace unavailable.")?;
            let table = row[1].as_str().ok_or("Schema table unavailable.")?;
            tables.insert((namespace.to_string(), table.to_string()));
        }
        let mut complete = schema["truncated"] != true && tables.len() <= 16;
        let mut hashes = serde_json::Map::new();
        for (namespace, table) in tables.into_iter().take(16) {
            let sql = if source.engine == "postgres" {
                format!(
                    "SELECT * FROM \"{}\".\"{}\"",
                    namespace.replace('"', "\"\""),
                    table.replace('"', "\"\"")
                )
            } else {
                format!(
                    "SELECT * FROM `{}`.`{}`",
                    namespace.replace('`', "``"),
                    table.replace('`', "``")
                )
            };
            let result = crate::remote_database::query(
                state,
                source,
                "database_query",
                &json!({"sql":sql,"limit":200}),
            )
            .await?;
            let mut rows = result["rows"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .map(|r| r.to_string())
                .collect::<Vec<_>>();
            rows.sort();
            complete &= result["truncated"] != true
                && !rows
                    .iter()
                    .any(|r| r.contains("\"omitted\":true") || r.contains("\"unavailable\":true"));
            hashes.insert(
                format!("{namespace}.{table}"),
                json!(dev::hash(rows.join("\n").as_bytes())),
            );
        }
        Ok::<_, String>(
            json!({"schema":schema,"rowHashes":hashes,"complete":complete,"scope":"read-only endpoint comparison, at most 16 tables, 200 columns/rows per table; external writers are not isolated"}),
        )
    };
    tokio::time::timeout(std::time::Duration::from_secs(30), operation)
        .await
        .map_err(|_| "Source preservation observation exceeded 30 seconds.")?
}
pub async fn execute(
    ctx: &crate::harness::HarnessCtx<'_>,
    source: &Profile,
    args: Value,
) -> Result<Value, String> {
    if ctx.state.database()?.setting::<bool>("offline_mode")? {
        return Err("Offline mode blocks remote database rehearsals.".into());
    }
    let admin_id = source
        .scratch_profile_id
        .as_deref()
        .ok_or("Configure a separate scratch-administration profile before migration rehearsal.")?;
    let administrator = crate::remote_database::load(ctx.state, admin_id)?;
    if administrator.engine != source.engine || administrator.id == source.id {
        return Err(
            "Source and scratch administration must be separate profiles using the same engine."
                .into(),
        );
    }
    let source_before = source_evidence(ctx.state, source).await?;
    let dump = cli(ctx, source, &source.dump_executable, None, true, None).await?;
    let clone_hash = dev::hash(&dump);
    let id = uuid::Uuid::new_v4().simple().to_string();
    let database = format!("locallm_rehearsal_{id}");
    let role = format!("llm_{}", &id[..24]);
    let secret = uuid::Uuid::new_v4().simple().to_string();
    let marker = format!("locallm-rehearsal:{}", uuid::Uuid::new_v4().simple());
    let mut ledger = json!({"ownershipMarker":marker,"owner":ctx.conversation_id,"root":dev::canonical(&ctx.snapshot.workspace_path)?,"adminProfileId":administrator.id,"adminFingerprint":dev::hash(serde_json::to_vec(&administrator).map_err(|e|e.to_string())?.as_slice()),"sourceProfileId":source.id,"database":database,"role":role,"roleCreated":false,"databaseCreated":false,"status":"prepared","createdAt":crate::store::now()});
    persist(ctx.state, &id, &ledger)?;
    (ctx.emit)(
        "tool_progress".into(),
        Some(
            json!({"rehearsalId":id,"message":"Disposable resource ownership saved. Use database_cleanup without an ID to discover interrupted rehearsals."}),
        ),
    );
    let outcome = async {
        let (database_exists, role_exists) =
            owned_resources(ctx.state, &administrator, &ledger).await?;
        if database_exists || role_exists {
            return Err(
                "Generated remote resource name already exists; nothing was created or adopted."
                    .to_string(),
            );
        }
        ledger["databaseAbsentBeforeCreate"] = json!(true);
        persist(ctx.state, &id, &ledger)?;
        let create_role = if source.engine == "postgres" {
            format!("BEGIN; CREATE ROLE \"{role}\" LOGIN PASSWORD '{secret}' NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION; COMMENT ON ROLE \"{role}\" IS '{marker}'; COMMIT")
        } else {
            format!("CREATE USER '{role}'@'%' IDENTIFIED BY '{secret}' ATTRIBUTE '{{\"locallmOwner\":\"{marker}\"}}'")
        };
        ledger["roleMayExist"] = json!(true);
        persist(ctx.state, &id, &ledger)?;
        admin(ctx.state, &administrator, &create_role).await?;
        ledger["roleCreated"] = json!(true);
        persist(ctx.state, &id, &ledger)?;
        let create_db = if source.engine == "postgres" {
            format!("CREATE DATABASE \"{database}\" OWNER \"{role}\"")
        } else {
            format!("CREATE DATABASE `{database}`")
        };
        ledger["databaseMayExist"] = json!(true);
        persist(ctx.state, &id, &ledger)?;
        admin(ctx.state, &administrator, &create_db).await?;
        ledger["databaseCreated"] = json!(true);
        persist(ctx.state, &id, &ledger)?;
        if source.engine == "mysql" {
            admin(
                ctx.state,
                &administrator,
                &format!("GRANT ALL PRIVILEGES ON `{database}`.* TO '{role}'@'%'"),
            )
            .await?;
        }
        let mut scratch = administrator.clone();
        scratch.database = database.clone();
        scratch.username = role.clone();
        cli(
            ctx,
            &scratch,
            &source.restore_executable,
            Some(dump),
            false,
            Some(&secret),
        )
        .await?;
        ledger["status"] = json!("restored");
        persist(ctx.state, &id, &ledger)?;
        let migration = args["sql"].as_str().ok_or("Missing migration SQL.")?;
        let result = if source.engine == "postgres" {
            let mut c = sqlx_postgres::PgConnection::connect_with(
                &crate::remote_database::pg_options(&scratch, &secret),
            )
            .await
            .map_err(|_| "Cannot connect to disposable PostgreSQL database.")?;
            execute_pg(&mut c, "SET statement_timeout='5s'; SET lock_timeout='3s'")
                .await
                .map_err(|_| "Cannot bound scratch PostgreSQL statements.")?;
            let before = pg_schema(&mut c).await?;
            execute_pg(&mut c, migration)
                .await
                .map_err(|_| "PostgreSQL migration failed in the disposable database.")?;
            execute_pg(&mut c,"COMMIT").await.map_err(|_| "PostgreSQL migration could not commit deferred constraints inside scratch database.")?;
            let after = pg_schema(&mut c).await?;
            json!({"before":before,"after":after,"schemaDiff":crate::development_credentials::structural_diff(&before,&after),"constraintEnforcement":"server","migrationSucceeded":true,"integrityScope":"server constraint metadata; not a physical corruption scan"})
        } else {
            let mut c = sqlx_mysql::MySqlConnection::connect_with(
                &crate::remote_database::mysql_options(&scratch, &secret),
            )
            .await
            .map_err(|_| "Cannot connect to disposable MySQL database.")?;
            execute_mysql(
                &mut c,
                "SET SESSION MAX_EXECUTION_TIME=5000; SET SESSION lock_wait_timeout=5",
            )
            .await
            .map_err(|_| "Cannot bound scratch MySQL statements.")?;
            let before = mysql_schema(&mut c).await?;
            execute_mysql(&mut c,migration).await.map_err(|_|"MySQL migration failed in the disposable database; DDL may have committed only inside that disposable database.")?;
            {
                execute_mysql(&mut c,"COMMIT").await.map_err(|_| "MySQL scratch migration commit failed; side effects remain limited to the disposable database.")?;
                let after = mysql_schema(&mut c).await?;
                json!({"before":before,"after":after,"schemaDiff":crate::development_credentials::structural_diff(&before,&after),"migrationSucceeded":true,"ddlImplicitCommit":true,"constraintEnforcement":"server","integrityScope":"column metadata and server enforcement; not a physical corruption scan"})
            }
        };
        Ok::<_, String>(result)
    };
    let result = tokio::time::timeout(std::time::Duration::from_secs(180), outcome)
        .await
        .map_err(|_| format!("Rehearsal {id} timed out; call database_cleanup with this ID."));
    let outcome = match result {
        Ok(v) => v,
        Err(e) => Err(e),
    };
    let cleanup_result = cleanup(ctx, &id).await;
    let source_after = source_evidence(ctx.state, source).await;
    let source_unchanged = source_after
        .as_ref()
        .ok()
        .filter(|v| v["complete"] == true && source_before["complete"] == true)
        .map(|v| v == &source_before);
    let mut result = json!({"root":dev::canonical(&ctx.snapshot.workspace_path)?,"rehearsalId":id,"engine":source.engine,"sourceProfileId":source.id,"originalModified":source_unchanged.map(|v| !v),"sourcePreservation":{"before":source_before,"after":source_after.as_ref().ok(),"error":source_after.as_ref().err(),"unchanged":source_unchanged},"result":outcome.as_ref().ok(),"message":outcome.as_ref().err(),"isError":outcome.is_err()||cleanup_result.is_err(),"cleanupComplete":cleanup_result.is_ok(),"cleanupError":cleanup_result.err(),"cloneSha256":clone_hash,"limitation":"Only successfully restored, privilege-restricted scratch databases are migrated. Native dump/restore utilities must be explicitly configured. External effects of privileged extensions are not OS-isolated."});
    result["evidenceId"] = json!(dev::save(
        &ctx.state.db_path,
        &ctx.conversation_id,
        &ctx.run_id,
        "database_rehearsal",
        &result
    )?);
    Ok(result)
}
pub async fn cleanup_tool(
    ctx: &crate::harness::HarnessCtx<'_>,
    args: Value,
) -> Result<Value, String> {
    if let Some(id) = args["rehearsal_id"].as_str() {
        return Ok(match cleanup(ctx, id).await {
            Ok(value) => value,
            Err(message) => {
                json!({"rehearsalId":id,"isError":true,"cleanupComplete":false,"recoveryRequired":true,"message":message})
            }
        });
    }
    let store = ctx.state.database()?;
    let ids: Vec<String> = store.setting("development.rehearsal_index")?;
    let root = dev::canonical(&ctx.snapshot.workspace_path)?;
    let mut pending = Vec::new();
    for id in ids {
        let ledger: Value = store.setting(&key(&id)?)?;
        if ledger["owner"] == ctx.conversation_id
            && ledger["root"] == root
            && ledger["status"] != "cleaned"
        {
            pending.push(
                json!({"rehearsalId":id,"status":ledger["status"],"createdAt":ledger["createdAt"]}),
            );
        }
    }
    Ok(json!({"pendingRehearsals":pending}))
}
pub fn definition() -> (String, String, Value) {
    ("database_cleanup".into(),"Clean up a disposable database and temporary role owned by a recorded rehearsal in this conversation/workspace. Omit rehearsal_id to list interrupted owned rehearsals. Never accepts arbitrary resource names. Requires the original scratch-administration profile fingerprint.".into(),{let mut schema=dev::schema(json!({"rehearsal_id":{"type":"string","minLength":32,"maxLength":36}}),&[]);schema["x-locallm-always-ask"]=json!(true);schema})
}
