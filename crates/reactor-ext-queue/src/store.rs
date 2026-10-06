use chrono::{DateTime, Utc};
use reactor_core::{OpenedProject, ProjectError, TaskCtx};
use serde_json::{json, Value};
use sqlx::Row;
use std::ops::DerefMut;

#[derive(Debug)]
pub enum QueueError {
    Unauthorized,
    Forbidden,
    InvalidName,
    NotFound,
    Conflict,
    BadRequest(&'static str),
    Db(String),
}

impl QueueError {
    pub fn status(&self) -> axum::http::StatusCode {
        match self {
            Self::Unauthorized => axum::http::StatusCode::UNAUTHORIZED,
            Self::Forbidden => axum::http::StatusCode::FORBIDDEN,
            Self::InvalidName | Self::BadRequest(_) => axum::http::StatusCode::BAD_REQUEST,
            Self::NotFound => axum::http::StatusCode::NOT_FOUND,
            Self::Conflict => axum::http::StatusCode::CONFLICT,
            Self::Db(_) => axum::http::StatusCode::INTERNAL_SERVER_ERROR,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Self::Unauthorized => "token required".into(),
            Self::Forbidden => "service key required".into(),
            Self::InvalidName => "invalid queue name".into(),
            Self::NotFound => "not found".into(),
            Self::Conflict => "queue exists".into(),
            Self::BadRequest(message) => (*message).into(),
            Self::Db(_) => "internal error".into(),
        }
    }
}

impl From<ProjectError> for QueueError {
    fn from(err: ProjectError) -> Self {
        match err {
            ProjectError::Unauthorized => Self::Unauthorized,
            ProjectError::Forbidden => Self::Forbidden,
            ProjectError::Unavailable(message) => Self::Db(message),
        }
    }
}

pub fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 63
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
}

pub async fn create_queue(project: &OpenedProject, name: &str) -> Result<(), QueueError> {
    if !valid_name(name) {
        return Err(QueueError::InvalidName);
    }
    let schema = schema_name(&project.schema)?;
    let mut tx = begin(project).await?;
    let sql = format!("SELECT \"{schema}\".queue_create($1)");
    let result = sqlx::query(&sql).bind(name).execute(tx.deref_mut()).await;
    finish(tx, result, "queue exists")
        .await
        .map(|_| ())
}

pub async fn list_queues(project: &OpenedProject) -> Result<Vec<Value>, QueueError> {
    let schema = schema_name(&project.schema)?;
    let mut tx = begin(project).await?;
    let sql = format!("SELECT name, created_at FROM \"{schema}\".queues ORDER BY name");
    let rows = sqlx::query(&sql).fetch_all(tx.deref_mut()).await.map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            json!({
                "name": row.get::<String, _>("name"),
                "created_at": row.get::<DateTime<Utc>, _>("created_at").to_rfc3339(),
            })
        })
        .collect())
}

pub async fn send(
    project: &OpenedProject,
    name: &str,
    message: Value,
    delay_secs: i64,
) -> Result<i64, QueueError> {
    if !valid_name(name) {
        return Err(QueueError::InvalidName);
    }
    if delay_secs < 0 {
        return Err(QueueError::BadRequest("delay_secs must be >= 0"));
    }
    let schema = schema_name(&project.schema)?;
    let mut tx = begin(project).await?;
    let sql = format!("SELECT \"{schema}\".queue_send($1, $2, $3)");
    let result = sqlx::query_scalar::<_, i64>(&sql)
        .bind(name)
        .bind(message)
        .bind(delay_secs as i32)
        .fetch_one(tx.deref_mut())
        .await;
    let id = match result {
        Ok(id) => id,
        Err(err) if missing_queue(&err) => return Err(QueueError::NotFound),
        Err(err) => return Err(db(err)),
    };
    tx.commit().await.map_err(db)?;
    Ok(id)
}

pub async fn read(
    project: &OpenedProject,
    name: &str,
    vt_secs: i32,
    qty: i32,
) -> Result<Vec<Value>, QueueError> {
    if !valid_name(name) {
        return Err(QueueError::InvalidName);
    }
    if vt_secs < 0 || qty < 1 {
        return Err(QueueError::BadRequest("vt_secs and qty must be positive"));
    }
    let qty = qty.min(100);
    let schema = schema_name(&project.schema)?;
    let mut tx = begin(project).await?;
    let sql = format!("SELECT msg_id, message, read_ct FROM \"{schema}\".queue_read($1, $2, $3)");
    let rows = sqlx::query(&sql)
        .bind(name)
        .bind(vt_secs)
        .bind(qty)
        .fetch_all(tx.deref_mut())
        .await
        .map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            json!({
                "msg_id": row.get::<i64, _>("msg_id"),
                "message": row.get::<Value, _>("message"),
                "read_ct": row.get::<i32, _>("read_ct"),
            })
        })
        .collect())
}

pub async fn remove(project: &OpenedProject, name: &str, msg_id: i64) -> Result<(), QueueError> {
    let schema = schema_name(&project.schema)?;
    ensure_queue(project, &schema, name).await?;
    let mut tx = begin(project).await?;
    let sql = format!("SELECT \"{schema}\".queue_delete($1)");
    sqlx::query(&sql)
        .bind(msg_id)
        .execute(tx.deref_mut())
        .await
        .map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(())
}

pub async fn archive(project: &OpenedProject, name: &str, msg_id: i64) -> Result<(), QueueError> {
    let schema = schema_name(&project.schema)?;
    ensure_queue(project, &schema, name).await?;
    let mut tx = begin(project).await?;
    let sql = format!("SELECT \"{schema}\".queue_archive($1)");
    sqlx::query(&sql)
        .bind(msg_id)
        .execute(tx.deref_mut())
        .await
        .map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(())
}

pub async fn peek(project: &OpenedProject, name: &str) -> Result<Vec<Value>, QueueError> {
    if !valid_name(name) {
        return Err(QueueError::InvalidName);
    }
    let schema = schema_name(&project.schema)?;
    ensure_queue(project, &schema, name).await?;
    let mut tx = begin(project).await?;
    let sql = format!(
        "SELECT msg_id, message, vt, read_ct, enqueued_at FROM \"{schema}\".queue_peek($1)"
    );
    let rows = sqlx::query(&sql)
        .bind(name)
        .fetch_all(tx.deref_mut())
        .await
        .map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            json!({
                "msg_id": row.get::<i64, _>("msg_id"),
                "message": row.get::<Value, _>("message"),
                "vt": row.get::<DateTime<Utc>, _>("vt").to_rfc3339(),
                "read_ct": row.get::<i32, _>("read_ct"),
                "enqueued_at": row.get::<DateTime<Utc>, _>("enqueued_at").to_rfc3339(),
            })
        })
        .collect())
}

pub async fn subscribe(
    project: &OpenedProject,
    name: &str,
    function_name: &str,
    vt_secs: i32,
    qty: i32,
    max_reads: i32,
) -> Result<(), QueueError> {
    if !valid_name(name) || !valid_function(function_name) {
        return Err(QueueError::InvalidName);
    }
    if vt_secs < 0 || qty < 1 || max_reads < 1 {
        return Err(QueueError::BadRequest("vt_secs, qty, and max_reads must be positive"));
    }
    let schema = schema_name(&project.schema)?;
    ensure_queue(project, &schema, name).await?;
    let mut tx = begin(project).await?;
    let sql = format!(
        "INSERT INTO \"{schema}\".queue_subscriptions (queue_name, function_name, vt_secs, qty, max_reads) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (queue_name, function_name) DO UPDATE \
         SET vt_secs = EXCLUDED.vt_secs, qty = EXCLUDED.qty, max_reads = EXCLUDED.max_reads"
    );
    sqlx::query(&sql)
        .bind(name)
        .bind(function_name)
        .bind(vt_secs)
        .bind(qty.min(100))
        .bind(max_reads)
        .execute(tx.deref_mut())
        .await
        .map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(())
}

pub async fn unsubscribe(
    project: &OpenedProject,
    name: &str,
    function_name: &str,
) -> Result<(), QueueError> {
    let schema = schema_name(&project.schema)?;
    let mut tx = begin(project).await?;
    let sql = format!(
        "DELETE FROM \"{schema}\".queue_subscriptions WHERE queue_name = $1 AND function_name = $2"
    );
    sqlx::query(&sql)
        .bind(name)
        .bind(function_name)
        .execute(tx.deref_mut())
        .await
        .map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(())
}

pub async fn list_subscriptions(project: &OpenedProject, name: &str) -> Result<Vec<Value>, QueueError> {
    let schema = schema_name(&project.schema)?;
    ensure_queue(project, &schema, name).await?;
    let mut tx = begin(project).await?;
    let sql = format!(
        "SELECT function_name, vt_secs, qty, max_reads FROM \"{schema}\".queue_subscriptions \
         WHERE queue_name = $1 ORDER BY function_name"
    );
    let rows = sqlx::query(&sql)
        .bind(name)
        .fetch_all(tx.deref_mut())
        .await
        .map_err(db)?;
    tx.commit().await.map_err(db)?;
    Ok(rows
        .into_iter()
        .map(|row| {
            json!({
                "function_name": row.get::<String, _>("function_name"),
                "vt_secs": row.get::<i32, _>("vt_secs"),
                "qty": row.get::<i32, _>("qty"),
                "max_reads": row.get::<i32, _>("max_reads"),
            })
        })
        .collect())
}

pub async fn drain(ctx: TaskCtx) -> Result<(), String> {
    let projects = ctx.projects.list().await.map_err(|err| match err {
        ProjectError::Unavailable(message) => message,
        ProjectError::Unauthorized => "unauthorized".into(),
        ProjectError::Forbidden => "forbidden".into(),
    })?;
    for project in projects {
        drain_project(&ctx, &project).await?;
    }
    Ok(())
}

async fn drain_project(ctx: &TaskCtx, project: &OpenedProject) -> Result<(), String> {
    let Ok(schema) = schema_name(&project.schema) else {
        return Ok(());
    };
    let mut tx = project.pool.begin().await.map_err(|err| err.to_string())?;
    set_service(&mut tx).await.map_err(|err| err.message())?;
    let sql = format!(
        "SELECT queue_name, function_name, vt_secs, qty, max_reads FROM \"{schema}\".queue_subscriptions"
    );
    let subs = match sqlx::query(&sql).fetch_all(tx.deref_mut()).await {
        Ok(rows) => rows,
        Err(err) if undefined_table(&err) => return Ok(()),
        Err(err) => return Err(err.to_string()),
    };
    tx.commit().await.map_err(|err| err.to_string())?;
    for row in subs {
        let queue: String = row.get("queue_name");
        let function: String = row.get("function_name");
        let vt_secs: i32 = row.get("vt_secs");
        let qty: i32 = row.get("qty");
        let max_reads: i32 = row.get("max_reads");
        let messages = read(project, &queue, vt_secs, qty)
            .await
            .map_err(|err| err.message())?;
        for message in messages {
            let msg_id = message["msg_id"].as_i64().unwrap_or(0);
            let read_ct = message["read_ct"].as_i64().unwrap_or(0) as i32;
            let body = serde_json::to_vec(&message["message"]).unwrap_or_else(|_| b"{}".to_vec());
            let delivery = format!("{queue}:{function}");
            match ctx
                .invoker
                .invoke(project.id, function.clone(), body)
                .await
            {
                Ok(_) => {
                    ctx.log
                        .record(project.id, "queue".into(), delivery, 200, String::new())
                        .await;
                    remove(project, &queue, msg_id)
                        .await
                        .map_err(|err| err.message())?;
                }
                Err(err) => {
                    ctx.log
                        .record(project.id, "queue".into(), delivery, 500, err)
                        .await;
                    if read_ct >= max_reads {
                        archive(project, &queue, msg_id)
                            .await
                            .map_err(|err| err.message())?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn schema_name(schema: &str) -> Result<String, QueueError> {
    let rest = schema.strip_prefix("proj_").unwrap_or("");
    if schema.len() == 25
        && rest.len() == 20
        && rest
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
    {
        Ok(schema.to_string())
    } else {
        Err(QueueError::Db("invalid schema".into()))
    }
}

fn valid_function(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.contains('/')
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

async fn begin(
    project: &OpenedProject,
) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, QueueError> {
    let schema = schema_name(&project.schema)?;
    let mut tx = project.pool.begin().await.map_err(db)?;
    set_service(&mut tx).await?;
    sqlx::query("SELECT set_config('search_path', $1, true)")
        .bind(schema)
        .execute(tx.deref_mut())
        .await
        .map_err(db)?;
    Ok(tx)
}

async fn set_service(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> Result<(), QueueError> {
    sqlx::query("SELECT set_config('role', 'service', true)")
        .execute(tx.deref_mut())
        .await
        .map_err(db)?;
    Ok(())
}

async fn ensure_queue(project: &OpenedProject, schema: &str, name: &str) -> Result<(), QueueError> {
    if !valid_name(name) {
        return Err(QueueError::InvalidName);
    }
    let mut tx = begin(project).await?;
    let sql = format!("SELECT 1 FROM \"{schema}\".queues WHERE name = $1");
    let found: Option<i32> = sqlx::query_scalar(&sql)
        .bind(name)
        .fetch_optional(tx.deref_mut())
        .await
        .map_err(db)?;
    tx.commit().await.map_err(db)?;
    if found.is_none() {
        return Err(QueueError::NotFound);
    }
    Ok(())
}

async fn finish(
    tx: sqlx::Transaction<'static, sqlx::Postgres>,
    result: Result<sqlx::postgres::PgQueryResult, sqlx::Error>,
    conflict: &str,
) -> Result<(), QueueError> {
    match result {
        Ok(_) => {
            tx.commit().await.map_err(db)?;
            Ok(())
        }
        Err(err) if unique_violation(&err) => Err(QueueError::Conflict),
        Err(err) if err.to_string().contains("invalid queue name") => Err(QueueError::InvalidName),
        Err(err) if err.to_string().contains(conflict) => Err(QueueError::Conflict),
        Err(err) => Err(db(err)),
    }
}

fn db(err: sqlx::Error) -> QueueError {
    QueueError::Db(err.to_string())
}

fn unique_violation(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .and_then(|db| db.code().map(|code| code == "23505"))
        .unwrap_or(false)
}

fn missing_queue(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .and_then(|db| db.code().map(|code| code == "P0002"))
        .unwrap_or(false)
        || err.to_string().contains("queue not found")
}

fn undefined_table(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .and_then(|db| db.code().map(|code| code == "42P01"))
        .unwrap_or(false)
}
