use crate::console::{migration_issue, schema_name};
use crate::{internal, ApiError, AppState};
use axum::http::StatusCode;
use pg_query::protobuf::node::Node as PgNode;
use pg_query::protobuf::{AlterTableType, Node, RangeVar};
use reactor_auth::{seal, unseal};
use serde_json::{json, Map, Value};
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

const ROW_CAP: usize = 1000;

#[derive(Clone, Default)]
pub struct ProjectPools {
    inner: Arc<Mutex<HashMap<Uuid, PgPool>>>,
}

impl ProjectPools {
    pub async fn forget(&self, project_id: Uuid) {
        if let Some(pool) = self.inner.lock().await.remove(&project_id) {
            pool.close().await;
        }
    }

    async fn get(&self, project_id: Uuid, url: &str) -> Result<PgPool, sqlx::Error> {
        let mut guard = self.inner.lock().await;
        if let Some(pool) = guard.get(&project_id) {
            return Ok(pool.clone());
        }
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect(url)
            .await?;
        guard.insert(project_id, pool.clone());
        Ok(pool)
    }
}

pub fn role_name(pref: &str) -> Result<String, ApiError> {
    let role = format!("proj_{pref}_admin");
    if !crate::console::ident(&role) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid schema"));
    }
    Ok(role)
}

fn qident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

fn qualified(schema: &str, name: &str) -> String {
    if schema.is_empty() {
        qident(name)
    } else {
        format!("{}.{}", qident(schema), qident(name))
    }
}

fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn new_password() -> String {
    let mut bytes = [0u8; 32];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    hex::encode(bytes)
}

fn rel_name(rel: &RangeVar) -> String {
    qualified(&rel.schemaname, &rel.relname)
}

fn stmt_of(raw: &pg_query::protobuf::RawStmt) -> Option<&PgNode> {
    raw.stmt.as_ref()?.node.as_ref()
}

fn node_of(node: &Node) -> Option<&PgNode> {
    node.node.as_ref()
}

pub fn classify(sql: &str) -> Result<Vec<String>, ApiError> {
    let parsed = pg_query::parse(sql)
        .map_err(|err| ApiError::new(StatusCode::BAD_REQUEST, err.to_string()))?;
    let mut warnings = Vec::new();
    let mut created = Vec::new();
    let mut enabled = Vec::new();
    for raw in &parsed.protobuf.stmts {
        let Some(node) = stmt_of(raw) else {
            continue;
        };
        match node {
            PgNode::DropStmt(stmt) => match stmt.remove_type().as_str_name() {
                "OBJECT_TABLE" => push(&mut warnings, "drop_table"),
                "OBJECT_SCHEMA" => push(&mut warnings, "drop_schema"),
                "OBJECT_POLICY" => push(&mut warnings, "drop_policy"),
                _ => {}
            },
            PgNode::TruncateStmt(_) => push(&mut warnings, "truncate"),
            PgNode::DeleteStmt(stmt) => {
                if stmt.where_clause.is_none() {
                    push(&mut warnings, "delete_without_where");
                }
            }
            PgNode::UpdateStmt(stmt) => {
                if stmt.where_clause.is_none() {
                    push(&mut warnings, "update_without_where");
                }
            }
            PgNode::CreateStmt(stmt) => {
                if let Some(rel) = &stmt.relation {
                    created.push(rel.relname.clone());
                }
            }
            PgNode::AlterTableStmt(stmt) => {
                let name = stmt
                    .relation
                    .as_ref()
                    .map(|rel| rel.relname.clone())
                    .unwrap_or_default();
                for cmd in &stmt.cmds {
                    let Some(PgNode::AlterTableCmd(cmd)) = node_of(cmd) else {
                        continue;
                    };
                    match cmd.subtype() {
                        AlterTableType::AtDropColumn => push(&mut warnings, "drop_column"),
                        AlterTableType::AtAlterColumnType => {
                            push(&mut warnings, "alter_column_type")
                        }
                        AlterTableType::AtDisableRowSecurity => push(&mut warnings, "disable_rls"),
                        AlterTableType::AtNoForceRowSecurity => push(&mut warnings, "no_force_rls"),
                        AlterTableType::AtEnableRowSecurity => {
                            if !name.is_empty() {
                                enabled.push(name.clone());
                            }
                        }
                        _ => {}
                    }
                }
            }
            PgNode::GrantStmt(stmt) => {
                if stmt.is_grant && grantee_is_anon(&stmt.grantees) {
                    push(&mut warnings, "grant_anon");
                }
            }
            _ => {}
        }
    }
    for name in &created {
        if !enabled.iter().any(|enabled| enabled == name) {
            push(&mut warnings, "create_table_without_rls");
            break;
        }
    }
    Ok(warnings)
}

fn push(warnings: &mut Vec<String>, code: &str) {
    if !warnings.iter().any(|item| item == code) {
        warnings.push(code.to_string());
    }
}

fn grantee_is_anon(grantees: &[Node]) -> bool {
    grantees.iter().any(|node| match node_of(node) {
        Some(PgNode::RoleSpec(role)) => role.rolename.eq_ignore_ascii_case("anon"),
        _ => false,
    })
}

pub fn down_sql(sql: &str) -> Option<String> {
    let parsed = pg_query::parse(sql).ok()?;
    let mut downs = Vec::new();
    for raw in &parsed.protobuf.stmts {
        let node = stmt_of(raw)?;
        downs.push(statement_down(node)?);
    }
    if downs.is_empty() {
        return None;
    }
    downs.reverse();
    Some(downs.join(";\n"))
}

fn added_column(cmd: &pg_query::protobuf::AlterTableCmd) -> Option<String> {
    if !cmd.name.is_empty() {
        return Some(cmd.name.clone());
    }
    match cmd.def.as_deref().and_then(node_of) {
        Some(PgNode::ColumnDef(column)) if !column.colname.is_empty() => {
            Some(column.colname.clone())
        }
        _ => None,
    }
}

fn statement_down(node: &PgNode) -> Option<String> {
    match node {
        PgNode::CreateStmt(stmt) => {
            let rel = stmt.relation.as_ref()?;
            Some(format!("DROP TABLE IF EXISTS {}", rel_name(rel)))
        }
        PgNode::IndexStmt(stmt) => {
            if stmt.idxname.is_empty() {
                return None;
            }
            let schema = stmt
                .relation
                .as_ref()
                .map(|rel| rel.schemaname.as_str())
                .unwrap_or("");
            Some(format!(
                "DROP INDEX IF EXISTS {}",
                qualified(schema, &stmt.idxname)
            ))
        }
        PgNode::CreatePolicyStmt(stmt) => {
            if stmt.policy_name.is_empty() {
                return None;
            }
            let table = stmt.table.as_ref()?;
            Some(format!(
                "DROP POLICY IF EXISTS {} ON {}",
                qident(&stmt.policy_name),
                rel_name(table)
            ))
        }
        PgNode::AlterTableStmt(stmt) => {
            let rel = stmt.relation.as_ref()?;
            let mut columns = Vec::new();
            for cmd in &stmt.cmds {
                let PgNode::AlterTableCmd(cmd) = node_of(cmd)? else {
                    return None;
                };
                if cmd.subtype() != AlterTableType::AtAddColumn {
                    return None;
                }
                let Some(name) = added_column(cmd) else {
                    return None;
                };
                columns.push(qident(&name));
            }
            if columns.is_empty() {
                return None;
            }
            columns.reverse();
            let drops = columns
                .into_iter()
                .map(|column| format!("DROP COLUMN {column}"))
                .collect::<Vec<_>>()
                .join(", ");
            Some(format!("ALTER TABLE {} {drops}", rel_name(rel)))
        }
        _ => None,
    }
}

pub async fn ensure_all(
    primary: &PgPool,
    dedicated: Option<&PgPool>,
    dedicated_url: Option<&str>,
    seal_key: &[u8; 32],
) -> anyhow::Result<()> {
    let rows: Vec<(Uuid, String, Option<String>)> =
        sqlx::query_as("SELECT id, ref, database_url FROM reactor.projects ORDER BY ref")
            .fetch_all(primary)
            .await?;
    for (id, pref, database_url) in rows {
        if let Some(url) = database_url {
            let data = if dedicated_url == Some(url.as_str()) {
                dedicated
                    .ok_or_else(|| anyhow::anyhow!("dedicated pool missing"))?
                    .clone()
            } else {
                sqlx::postgres::PgPoolOptions::new()
                    .max_connections(1)
                    .connect(&url)
                    .await?
            };
            ensure_project_role(primary, &data, seal_key, id, &pref).await?;
        } else {
            ensure_project_role(primary, primary, seal_key, id, &pref).await?;
        }
    }
    Ok(())
}

pub async fn ensure_project_role(
    control: &PgPool,
    data: &PgPool,
    seal_key: &[u8; 32],
    project_id: Uuid,
    pref: &str,
) -> anyhow::Result<()> {
    let role = role_name(pref).map_err(|err| anyhow::anyhow!(err.text().to_string()))?;
    let schema = schema_name(pref).map_err(|err| anyhow::anyhow!(err.text().to_string()))?;
    let stored: Option<String> =
        sqlx::query_scalar("SELECT password FROM reactor.project_db_roles WHERE project_id = $1")
            .bind(project_id)
            .fetch_optional(control)
            .await?;
    let password = if let Some(stored) = stored {
        String::from_utf8(unseal(seal_key, &stored)?)?
    } else {
        let password = new_password();
        let sealed = seal(seal_key, password.as_bytes())?;
        sqlx::query("INSERT INTO reactor.project_db_roles (project_id, password) VALUES ($1, $2)")
            .bind(project_id)
            .bind(&sealed)
            .execute(control)
            .await?;
        password
    };
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_roles WHERE rolname = $1)")
            .bind(&role)
            .fetch_one(data)
            .await?;
    if exists {
        exec(
            data,
            &format!(
                "ALTER ROLE {} WITH LOGIN NOINHERIT BYPASSRLS CONNECTION LIMIT 2",
                qident(&role)
            ),
        )
        .await?;
    } else {
        exec(
            data,
            &format!(
                "CREATE ROLE {} LOGIN NOINHERIT BYPASSRLS CONNECTION LIMIT 2 PASSWORD {}",
                qident(&role),
                quote_literal(&password)
            ),
        )
        .await?;
    }
    exec(
        data,
        &format!("ALTER ROLE {} SET statement_timeout = '15s'", qident(&role)),
    )
    .await?;
    exec(
        data,
        &format!("ALTER ROLE {} SET lock_timeout = '5s'", qident(&role)),
    )
    .await?;
    exec(
        data,
        &format!(
            "ALTER ROLE {} SET idle_in_transaction_session_timeout = '30s'",
            qident(&role)
        ),
    )
    .await?;
    exec(
        data,
        &format!(
            "ALTER ROLE {} SET search_path TO {}",
            qident(&role),
            qident(&schema)
        ),
    )
    .await?;
    transfer_owned(data, pref, &role).await?;
    Ok(())
}

async fn exec(pool: &PgPool, sql: &str) -> anyhow::Result<()> {
    sqlx::raw_sql(sql).execute(pool).await?;
    Ok(())
}

async fn transfer_owned(pool: &PgPool, pref: &str, role: &str) -> anyhow::Result<()> {
    let own = schema_name(pref).map_err(|err| anyhow::anyhow!(err.text().to_string()))?;
    let schemas: Vec<String> = sqlx::query_scalar(
        "SELECT nspname FROM pg_namespace WHERE nspname = $1 OR starts_with(nspname, $2) ORDER BY nspname",
    )
    .bind(&own)
    .bind(format!("{own}_"))
    .fetch_all(pool)
    .await?;
    for schema in schemas {
        if !crate::console::owned_schema(pref, &schema) {
            continue;
        }
        exec(
            pool,
            &format!("ALTER SCHEMA {} OWNER TO {}", qident(&schema), qident(role)),
        )
        .await?;
        exec(
            pool,
            &format!(
                "GRANT USAGE ON SCHEMA {} TO anon, authenticated, service",
                qident(&schema)
            ),
        )
        .await?;
        let rels: Vec<(String, String)> = sqlx::query_as(
            "SELECT c.relkind::text, c.relname FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE n.nspname = $1 AND c.relkind IN ('r','p','v','m','S','f') \
             AND NOT EXISTS ( \
               SELECT 1 FROM pg_depend d \
               WHERE d.objid = c.oid AND d.classid = 'pg_class'::regclass \
                 AND d.refclassid = 'pg_class'::regclass AND d.deptype IN ('a','i') \
             )",
        )
        .bind(&schema)
        .fetch_all(pool)
        .await?;
        for (kind, name) in rels {
            let keyword = match kind.as_str() {
                "r" | "p" => "TABLE",
                "v" => "VIEW",
                "m" => "MATERIALIZED VIEW",
                "S" => "SEQUENCE",
                "f" => "FOREIGN TABLE",
                _ => continue,
            };
            exec(
                pool,
                &format!(
                    "ALTER {keyword} {}.{} OWNER TO {}",
                    qident(&schema),
                    qident(&name),
                    qident(role)
                ),
            )
            .await?;
        }
        let funcs: Vec<(String, String)> = sqlx::query_as(
            "SELECT p.proname, pg_get_function_identity_arguments(p.oid) \
             FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace WHERE n.nspname = $1",
        )
        .bind(&schema)
        .fetch_all(pool)
        .await?;
        for (name, args) in funcs {
            exec(
                pool,
                &format!(
                    "ALTER FUNCTION {}.{}({args}) OWNER TO {}",
                    qident(&schema),
                    qident(&name),
                    qident(role)
                ),
            )
            .await?;
        }
        let types: Vec<String> = sqlx::query_scalar(
            "SELECT t.typname FROM pg_type t JOIN pg_namespace n ON n.oid = t.typnamespace \
             WHERE n.nspname = $1 AND t.typtype IN ('e','d','c') AND t.typrelid = 0",
        )
        .bind(&schema)
        .fetch_all(pool)
        .await?;
        for name in types {
            exec(
                pool,
                &format!(
                    "ALTER TYPE {}.{} OWNER TO {}",
                    qident(&schema),
                    qident(&name),
                    qident(role)
                ),
            )
            .await?;
        }
    }
    Ok(())
}

pub async fn drop_project_role(pool: &PgPool, pref: &str) -> anyhow::Result<()> {
    let Ok(role) = role_name(pref) else {
        return Ok(());
    };
    sqlx::query(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename = $1 AND pid <> pg_backend_pid()",
    )
    .bind(&role)
    .execute(pool)
    .await?;
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_roles WHERE rolname = $1)")
            .bind(&role)
            .fetch_one(pool)
            .await?;
    if !exists {
        return Ok(());
    }
    exec(pool, &format!("DROP OWNED BY {}", qident(&role))).await?;
    exec(pool, &format!("DROP ROLE {}", qident(&role))).await?;
    Ok(())
}

pub async fn reset_project_role(
    state: &AppState,
    project_id: Uuid,
    pref: &str,
    database_url: Option<&str>,
) -> Result<(), ApiError> {
    let role = role_name(pref)?;
    let password = new_password();
    let data = data_pool(state, database_url).await?;
    exec(
        &data,
        &format!(
            "ALTER ROLE {} PASSWORD {}",
            qident(&role),
            quote_literal(&password)
        ),
    )
    .await
    .map_err(internal)?;
    let sealed = seal(state.issuer.seal_key(), password.as_bytes()).map_err(internal)?;
    sqlx::query(
        "INSERT INTO reactor.project_db_roles (project_id, password) VALUES ($1, $2) \
         ON CONFLICT (project_id) DO UPDATE SET password = EXCLUDED.password",
    )
    .bind(project_id)
    .bind(&sealed)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    sqlx::query(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename = $1 AND pid <> pg_backend_pid()",
    )
    .bind(&role)
    .execute(&data)
    .await
    .map_err(internal)?;
    state.project_pools.forget(project_id).await;
    Ok(())
}

async fn data_pool(state: &AppState, database_url: Option<&str>) -> Result<PgPool, ApiError> {
    crate::pool_for(state, database_url).await.map_err(internal)
}

async fn project_pool(
    state: &AppState,
    project_id: Uuid,
    pref: &str,
    database_url: Option<&str>,
) -> Result<PgPool, ApiError> {
    let role = role_name(pref)?;
    let sealed: Option<String> =
        sqlx::query_scalar("SELECT password FROM reactor.project_db_roles WHERE project_id = $1")
            .bind(project_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    let Some(sealed) = sealed else {
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "project role is missing",
        ));
    };
    let password = String::from_utf8(unseal(state.issuer.seal_key(), &sealed).map_err(internal)?)
        .map_err(internal)?;
    let base = database_url.unwrap_or(state.config.database_url.as_str());
    let mut url = url::Url::parse(base).map_err(internal)?;
    url.set_username(&role)
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "invalid database url"))?;
    url.set_password(Some(&password))
        .map_err(|_| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "invalid database url"))?;
    state
        .project_pools
        .get(project_id, url.as_str())
        .await
        .map_err(internal)
}

pub async fn project_database_url(
    state: &AppState,
    project_id: Uuid,
) -> Result<Option<String>, ApiError> {
    sqlx::query_scalar("SELECT database_url FROM reactor.projects WHERE id = $1")
        .bind(project_id)
        .fetch_one(&state.pool)
        .await
        .map_err(internal)
}

pub async fn exec_as_project(
    state: &AppState,
    project_id: Uuid,
    pref: &str,
    database_url: Option<&str>,
    sql: &str,
    read_only: bool,
    generous_timeout: bool,
) -> Result<Value, ApiError> {
    let schema = schema_name(pref)?;
    let parts = pg_query::split_with_parser(sql)
        .map_err(|err| ApiError::new(StatusCode::BAD_REQUEST, err.to_string()))?;
    let pool = project_pool(state, project_id, pref, database_url).await?;
    let mut tx = pool.begin().await.map_err(internal)?;
    if read_only {
        sqlx::query("SET TRANSACTION READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    if generous_timeout {
        sqlx::query("SET LOCAL statement_timeout = '120s'")
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        sqlx::query("SET LOCAL lock_timeout = '30s'")
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    sqlx::query(&format!("SET LOCAL search_path TO {}", qident(&schema)))
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    let mut results = Vec::new();
    for part in parts {
        let statement = part.trim().trim_end_matches(';').trim();
        if statement.is_empty() {
            continue;
        }
        if is_query(statement)? {
            results.push(fetch_rows(&mut tx, statement).await?);
        } else {
            let done = sqlx::query(statement)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
            results.push(json!({
                "command": command_word(statement),
                "columns": [],
                "rows": [],
                "truncated": false,
                "rows_affected": done.rows_affected(),
            }));
        }
    }
    tx.commit().await.map_err(internal)?;
    Ok(json!({ "results": results }))
}

fn is_query(sql: &str) -> Result<bool, ApiError> {
    let parsed = pg_query::parse(sql)
        .map_err(|err| ApiError::new(StatusCode::BAD_REQUEST, err.to_string()))?;
    let Some(raw) = parsed.protobuf.stmts.first() else {
        return Ok(false);
    };
    Ok(matches!(stmt_of(raw), Some(PgNode::SelectStmt(_))))
}

async fn fetch_rows(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    sql: &str,
) -> Result<Value, ApiError> {
    let wrapped = format!(
        "SELECT row_to_json(q)::json FROM ({sql}) q LIMIT {}",
        ROW_CAP + 1
    );
    let rows: Vec<(Value,)> = sqlx::query_as(&wrapped)
        .fetch_all(&mut **tx)
        .await
        .map_err(db_err)?;
    let truncated = rows.len() > ROW_CAP;
    let kept: Vec<Value> = rows.into_iter().take(ROW_CAP).map(|row| row.0).collect();
    let columns = kept
        .first()
        .and_then(|row| row.as_object())
        .map(|object| object.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default();
    Ok(json!({
        "command": "SELECT",
        "columns": columns,
        "rows": kept,
        "truncated": truncated,
        "rows_affected": kept.len(),
    }))
}

fn command_word(sql: &str) -> String {
    sql.split_whitespace()
        .next()
        .unwrap_or("OK")
        .to_ascii_uppercase()
}

fn db_err(err: sqlx::Error) -> ApiError {
    match &err {
        sqlx::Error::Database(db) => {
            ApiError::new(StatusCode::BAD_REQUEST, db.message().to_string())
        }
        _ => internal(err),
    }
}

pub async fn apply_user_migration(
    state: &AppState,
    project_id: Uuid,
    pref: &str,
    applied_by: Option<Uuid>,
    version: &str,
    sql: &str,
    down_sql: Option<&str>,
    source: &str,
) -> Result<(), ApiError> {
    if let Some(issue) = migration_issue(version, sql) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, issue));
    }
    let inserted = sqlx::query(
        "INSERT INTO reactor.schema_migrations \
         (project_id, version, sql, down_sql, applied_by, source, status) \
         VALUES ($1, $2, $3, $4, $5, $6, 'pending')",
    )
    .bind(project_id)
    .bind(version)
    .bind(sql)
    .bind(down_sql)
    .bind(applied_by)
    .bind(source)
    .execute(&state.pool)
    .await;
    if let Err(err) = inserted {
        if let sqlx::Error::Database(db) = &err {
            if db.code().as_deref() == Some("23505") {
                let status: Option<String> = sqlx::query_scalar(
                    "SELECT status FROM reactor.schema_migrations WHERE project_id = $1 AND version = $2",
                )
                .bind(project_id)
                .bind(version)
                .fetch_optional(&state.pool)
                .await
                .map_err(internal)?;
                let message = if status.as_deref() == Some("pending") {
                    "migration pending"
                } else {
                    "migration already applied"
                };
                return Err(ApiError::new(StatusCode::CONFLICT, message));
            }
        }
        return Err(internal(err));
    }
    let database_url = project_database_url(state, project_id).await?;
    if let Err(err) = exec_as_project(
        state,
        project_id,
        pref,
        database_url.as_deref(),
        sql,
        false,
        true,
    )
    .await
    {
        let _ = sqlx::query(
            "DELETE FROM reactor.schema_migrations WHERE project_id = $1 AND version = $2 AND status = 'pending'",
        )
        .bind(project_id)
        .bind(version)
        .execute(&state.pool)
        .await;
        return Err(err);
    }
    sqlx::query(
        "UPDATE reactor.schema_migrations SET status = 'applied' WHERE project_id = $1 AND version = $2",
    )
    .bind(project_id)
    .bind(version)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(())
}

pub async fn list_migrations(state: &AppState, project_id: Uuid) -> Result<Value, ApiError> {
    let rows: Vec<(
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    )> = sqlx::query_as(
        "SELECT version, status, source, sql, down_sql, applied_at::text \
             FROM reactor.schema_migrations WHERE project_id = $1 ORDER BY applied_at, version",
    )
    .bind(project_id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let items = rows
        .into_iter()
        .map(|(version, status, source, sql, down_sql, applied_at)| {
            let mut item = Map::new();
            item.insert("version".into(), json!(version));
            item.insert("status".into(), json!(status));
            item.insert("source".into(), json!(source));
            item.insert("sql".into(), json!(sql));
            item.insert("down_sql".into(), json!(down_sql));
            item.insert("applied_at".into(), json!(applied_at));
            item.insert(
                "revertable".into(),
                json!(
                    down_sql.as_ref().is_some_and(|sql| !sql.trim().is_empty())
                        || sql
                            .as_ref()
                            .is_some_and(|sql| down_sql_of_stored(sql).is_some())
                ),
            );
            Value::Object(item)
        })
        .collect::<Vec<_>>();
    Ok(json!(items))
}

fn down_sql_of_stored(sql: &str) -> Option<String> {
    down_sql(sql)
}

pub async fn revert_migration(
    state: &AppState,
    project_id: Uuid,
    pref: &str,
    applied_by: Uuid,
    version: &str,
) -> Result<String, ApiError> {
    let row: Option<(Option<String>, Option<String>, String)> = sqlx::query_as(
        "SELECT sql, down_sql, status FROM reactor.schema_migrations WHERE project_id = $1 AND version = $2",
    )
    .bind(project_id)
    .bind(version)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some((sql, stored_down, status)) = row else {
        return Err(ApiError::not_found());
    };
    if status != "applied" {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "migration is not applied",
        ));
    }
    let slug = revert_slug(version)?;
    let versions: Vec<String> =
        sqlx::query_scalar("SELECT version FROM reactor.schema_migrations WHERE project_id = $1")
            .bind(project_id)
            .fetch_all(&state.pool)
            .await
            .map_err(internal)?;
    let marker = format!("_revert_{slug}.sql");
    if versions.iter().any(|item| item.ends_with(&marker)) {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "migration already reverted",
        ));
    }
    let down = match stored_down.filter(|sql| !sql.trim().is_empty()) {
        Some(sql) => sql,
        None => down_sql(sql.as_deref().unwrap_or(""))
            .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "no safe revert"))?,
    };
    let next = next_revert_version(&versions, &slug)?;
    apply_user_migration(
        state,
        project_id,
        pref,
        Some(applied_by),
        &next,
        &down,
        None,
        "console",
    )
    .await?;
    Ok(next)
}

fn revert_slug(version: &str) -> Result<String, ApiError> {
    let Some(name) = version.strip_suffix(".sql") else {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid version"));
    };
    let Some((_, rest)) = name.split_once('_') else {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid version"));
    };
    if rest.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid version"));
    }
    Ok(rest.to_string())
}

fn next_revert_version(versions: &[String], slug: &str) -> Result<String, ApiError> {
    let mut n = 1i32;
    for version in versions {
        let digits: String = version.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(parsed) = digits.parse::<i32>() {
            n = n.max(parsed + 1);
        }
    }
    if !(1..=9999).contains(&n) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid version"));
    }
    let version = format!("{n:04}_revert_{slug}.sql");
    if version.len() > 80 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid version"));
    }
    Ok(version)
}

pub fn superware_launch_sql() -> &'static str {
    r#"CREATE TABLE sw_members (
  user_id text PRIMARY KEY,
  role text NOT NULL
);
ALTER TABLE sw_members ENABLE ROW LEVEL SECURITY;
ALTER TABLE sw_members FORCE ROW LEVEL SECURITY;

CREATE TABLE sw_grants (
  role text NOT NULL,
  permission text NOT NULL,
  PRIMARY KEY (role, permission)
);
INSERT INTO sw_grants (role, permission) VALUES ('owner', 'jobs.access');
ALTER TABLE sw_grants ENABLE ROW LEVEL SECURITY;
ALTER TABLE sw_grants FORCE ROW LEVEL SECURITY;

CREATE FUNCTION auth_sub() RETURNS text
LANGUAGE sql STABLE AS $sw$
  SELECT coalesce(current_setting('request.jwt.claims', true)::json->>'sub', '');
$sw$;

CREATE FUNCTION sw_role() RETURNS text
LANGUAGE sql STABLE SECURITY DEFINER SET search_path FROM CURRENT AS $sw$
  SELECT role FROM sw_members WHERE user_id = auth_sub() LIMIT 1;
$sw$;

CREATE FUNCTION sw_can(actor text, permission text, entity text, row_id uuid) RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path FROM CURRENT AS $sw$
  SELECT EXISTS (
    SELECT 1 FROM sw_members WHERE user_id = actor AND role = 'owner'
  ) OR EXISTS (
    SELECT 1 FROM sw_members AS member
    JOIN sw_grants AS allowed ON allowed.role = member.role
    WHERE member.user_id = actor AND allowed.permission = sw_can.permission
  );
$sw$;

CREATE POLICY sw_members_read ON sw_members FOR SELECT TO authenticated USING (sw_role() IS NOT NULL);
GRANT SELECT ON sw_members TO authenticated;
GRANT SELECT, INSERT, DELETE ON sw_members TO service;

CREATE POLICY sw_grants_read ON sw_grants FOR SELECT TO authenticated USING (true);
GRANT SELECT ON sw_grants TO authenticated, service;

CREATE TABLE jobs (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  title text
);
ALTER TABLE jobs ENABLE ROW LEVEL SECURITY;
ALTER TABLE jobs FORCE ROW LEVEL SECURITY;
CREATE POLICY jobs_access ON jobs FOR ALL TO authenticated
  USING (sw_can(auth_sub(), 'jobs.access', 'jobs', id))
  WITH CHECK (sw_can(auth_sub(), 'jobs.access', 'jobs', id));
GRANT SELECT, INSERT, UPDATE, DELETE ON jobs TO authenticated, service;

CREATE TABLE sw_connections (
  key text PRIMARY KEY,
  secret text NOT NULL DEFAULT ''
);
ALTER TABLE sw_connections ENABLE ROW LEVEL SECURITY;
ALTER TABLE sw_connections FORCE ROW LEVEL SECURITY;
GRANT SELECT, INSERT, UPDATE, DELETE ON sw_connections TO service;
CREATE VIEW sw_connection_status WITH (security_invoker = false) AS
  SELECT key FROM sw_connections;
GRANT SELECT ON sw_connection_status TO authenticated;

CREATE FUNCTION sw_apply_data(entity text, op text, input jsonb) RETURNS jsonb
LANGUAGE plpgsql AS $sw$
BEGIN
  IF current_setting('sw.apply', true) IS DISTINCT FROM '1' THEN
    RAISE EXCEPTION 'not allowed';
  END IF;
  RETURN input;
END;
$sw$;
REVOKE ALL ON FUNCTION sw_apply_data(text, text, jsonb) FROM PUBLIC;

CREATE TABLE scratch (
  id int
);
ALTER TABLE scratch ENABLE ROW LEVEL SECURITY;
ALTER TABLE scratch FORCE ROW LEVEL SECURITY;"#
}

#[cfg(test)]
mod tests {
    use super::{classify, down_sql, superware_launch_sql};

    fn codes(sql: &str) -> Vec<String> {
        classify(sql).unwrap_or_else(|err| panic!("{}", err.text()))
    }

    #[test]
    fn warns_on_destructive_sql() {
        assert!(codes("DROP TABLE cafe;").contains(&"drop_table".into()));
        assert!(codes("DROP SCHEMA extra;").contains(&"drop_schema".into()));
        assert!(codes("TRUNCATE notes;").contains(&"truncate".into()));
        assert!(codes("DELETE FROM notes;").contains(&"delete_without_where".into()));
        assert!(codes("DELETE FROM notes WHERE id = 1;").is_empty());
        assert!(codes("UPDATE notes SET body = 'x';").contains(&"update_without_where".into()));
        assert!(codes("UPDATE notes SET body = 'x' WHERE id = 1;").is_empty());
        assert!(codes("ALTER TABLE notes DROP COLUMN body;").contains(&"drop_column".into()));
        assert!(codes("ALTER TABLE notes ALTER COLUMN body TYPE int;")
            .contains(&"alter_column_type".into()));
        assert!(
            codes("ALTER TABLE notes DISABLE ROW LEVEL SECURITY;").contains(&"disable_rls".into())
        );
        assert!(codes("ALTER TABLE notes NO FORCE ROW LEVEL SECURITY;")
            .contains(&"no_force_rls".into()));
        assert!(codes("DROP POLICY notes_owner ON notes;").contains(&"drop_policy".into()));
        assert!(codes("CREATE TABLE bare (id int);").contains(&"create_table_without_rls".into()));
        assert!(
            codes("CREATE TABLE bare (id int); ALTER TABLE bare ENABLE ROW LEVEL SECURITY;")
                .is_empty()
        );
        assert!(codes("GRANT SELECT ON notes TO anon;").contains(&"grant_anon".into()));
        assert!(codes("GRANT SELECT ON notes TO service;").is_empty());
    }

    #[test]
    fn superware_launch_sql_has_no_warnings() {
        assert!(codes(superware_launch_sql()).is_empty());
    }

    #[test]
    fn down_sql_covers_mechanical_ddl_only() {
        let table = down_sql("CREATE TABLE t (id int);").unwrap();
        assert!(table.contains("DROP TABLE"));
        assert!(table.contains("t"));
        let index = down_sql("CREATE INDEX idx_t ON t (id);").unwrap();
        assert!(index.contains("DROP INDEX"));
        assert!(index.contains("idx_t"));
        let policy = down_sql("CREATE POLICY p ON t FOR SELECT USING (true);").unwrap();
        assert!(policy.contains("DROP POLICY"));
        assert!(policy.contains("p"));
        let column = down_sql("ALTER TABLE t ADD COLUMN note text;").unwrap();
        assert!(column.contains("DROP COLUMN"));
        assert!(column.contains("note"));
        assert!(down_sql("ALTER TABLE t ALTER COLUMN note TYPE int;").is_none());
        assert!(down_sql("UPDATE t SET note = 'x';").is_none());
        assert!(down_sql("DELETE FROM t;").is_none());
    }
}
