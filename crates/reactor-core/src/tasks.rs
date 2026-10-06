use crate::TaskCtx;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use std::collections::HashMap;
use std::ops::DerefMut;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct TaskRow {
    pub id: Uuid,
    pub kind: String,
    pub project_id: Option<Uuid>,
    pub payload: Value,
    pub attempts: i32,
    pub max_attempts: i32,
}

#[derive(Debug, thiserror::Error)]
pub enum EnqueueError {
    #[error("duplicate task")]
    Conflict,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

#[derive(Clone)]
pub struct Tasks {
    pool: PgPool,
    handlers: Arc<RwLock<HashMap<String, Arc<dyn TaskHandler>>>>,
}

#[async_trait]
pub trait TaskHandler: Send + Sync {
    async fn run(&self, ctx: TaskCtx, task: TaskRow) -> Result<(), String>;
}

pub struct FnInvoke;

#[async_trait]
impl TaskHandler for FnInvoke {
    async fn run(&self, ctx: TaskCtx, task: TaskRow) -> Result<(), String> {
        let name = task
            .payload
            .get("name")
            .and_then(|value| value.as_str())
            .ok_or("missing name")?
            .to_string();
        let project_id = task.project_id.ok_or("missing project")?;
        let body = match task.payload.get("body") {
            Some(value) => serde_json::to_vec(value).map_err(|err| err.to_string())?,
            None => b"{}".to_vec(),
        };
        ctx.invoker.invoke(project_id, name, body).await?;
        Ok(())
    }
}

impl Tasks {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            handlers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn register(&self, kind: impl Into<String>, handler: Arc<dyn TaskHandler>) {
        self.handlers.write().await.insert(kind.into(), handler);
    }

    pub async fn enqueue(
        &self,
        kind: &str,
        project_id: Option<Uuid>,
        payload: Value,
        run_at: DateTime<Utc>,
        max_attempts: i32,
        dedupe_key: Option<&str>,
    ) -> Result<Uuid, EnqueueError> {
        let mut tx = self.pool.begin().await?;
        let id = enqueue_tx(
            &mut tx,
            kind,
            project_id,
            payload,
            run_at,
            max_attempts,
            dedupe_key,
        )
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    pub async fn get(&self, id: Uuid) -> Result<Option<StoredTask>, sqlx::Error> {
        let row: Option<(Uuid, String, Option<Uuid>, String, i32, i32, Option<String>)> =
            sqlx::query_as(
                "SELECT id, kind, project_id, status, attempts, max_attempts, last_error \
                 FROM reactor.tasks WHERE id = $1",
            )
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(
            |(id, kind, project_id, status, attempts, max_attempts, last_error)| StoredTask {
                id,
                kind,
                project_id,
                status,
                attempts,
                max_attempts,
                last_error,
            },
        ))
    }

    pub async fn dispatch(&self, ctx: &TaskCtx, batch: i32) -> Result<usize, String> {
        let mut ran = 0;
        for _ in 0..batch {
            let mut tx = self.pool.begin().await.map_err(|err| err.to_string())?;
            let Some(task) = claim_one(&mut tx).await.map_err(|err| err.to_string())? else {
                break;
            };
            tx.commit().await.map_err(|err| err.to_string())?;
            let handler = self.handlers.read().await.get(&task.kind).cloned();
            let known = handler.is_some();
            let result = match handler {
                Some(handler) => handler.run(ctx.clone(), task.clone()).await,
                None => Err("unknown kind".into()),
            };
            let mut tx = self.pool.begin().await.map_err(|err| err.to_string())?;
            settle(&mut tx, &task, known, result)
                .await
                .map_err(|err| err.to_string())?;
            tx.commit().await.map_err(|err| err.to_string())?;
            ran += 1;
        }
        Ok(ran)
    }
}

#[derive(Debug)]
pub struct StoredTask {
    pub id: Uuid,
    pub kind: String,
    pub project_id: Option<Uuid>,
    pub status: String,
    pub attempts: i32,
    pub max_attempts: i32,
    pub last_error: Option<String>,
}

pub fn backoff_secs(attempts: i32) -> i64 {
    let shift = attempts.max(1) as u32;
    1i64.checked_shl(shift).unwrap_or(60).clamp(2, 60)
}

pub async fn enqueue_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    kind: &str,
    project_id: Option<Uuid>,
    payload: Value,
    run_at: DateTime<Utc>,
    max_attempts: i32,
    dedupe_key: Option<&str>,
) -> Result<Uuid, EnqueueError> {
    let id = Uuid::new_v4();
    let result = sqlx::query(
        "INSERT INTO reactor.tasks \
         (id, kind, project_id, payload, run_at, attempts, max_attempts, status, dedupe_key) \
         VALUES ($1, $2, $3, $4, $5, 0, $6, 'queued', $7)",
    )
    .bind(id)
    .bind(kind)
    .bind(project_id)
    .bind(payload)
    .bind(run_at)
    .bind(max_attempts.max(1))
    .bind(dedupe_key)
    .execute(tx.deref_mut())
    .await;
    match result {
        Ok(_) => Ok(id),
        Err(err) if unique_violation(&err) => Err(EnqueueError::Conflict),
        Err(err) => Err(EnqueueError::Db(err)),
    }
}

pub async fn claim_one(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<Option<TaskRow>, sqlx::Error> {
    let row: Option<(Uuid, String, Option<Uuid>, Value, i32, i32)> = sqlx::query_as(
        "WITH picked AS ( \
            SELECT id FROM reactor.tasks \
            WHERE (status = 'queued' AND run_at <= now()) \
               OR (status = 'running' AND lease_until < now()) \
            ORDER BY run_at \
            FOR UPDATE SKIP LOCKED \
            LIMIT 1 \
         ) \
         UPDATE reactor.tasks AS t \
         SET status = 'running', \
             lease_until = now() + interval '30 seconds', \
             attempts = t.attempts + 1 \
         FROM picked \
         WHERE t.id = picked.id \
         RETURNING t.id, t.kind, t.project_id, t.payload, t.attempts, t.max_attempts",
    )
    .fetch_optional(tx.deref_mut())
    .await?;
    Ok(row.map(
        |(id, kind, project_id, payload, attempts, max_attempts)| TaskRow {
            id,
            kind,
            project_id,
            payload,
            attempts,
            max_attempts,
        },
    ))
}

pub async fn settle(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    task: &TaskRow,
    known: bool,
    result: Result<(), String>,
) -> Result<(), sqlx::Error> {
    if !known {
        sqlx::query(
            "UPDATE reactor.tasks SET status = 'dead', lease_until = NULL, last_error = $2 WHERE id = $1",
        )
        .bind(task.id)
        .bind("unknown kind")
        .execute(tx.deref_mut())
        .await?;
        return Ok(());
    }
    match result {
        Ok(()) => {
            sqlx::query(
                "UPDATE reactor.tasks SET status = 'done', lease_until = NULL, last_error = NULL WHERE id = $1",
            )
            .bind(task.id)
            .execute(tx.deref_mut())
            .await?;
        }
        Err(err) if task.attempts < task.max_attempts => {
            sqlx::query(
                "UPDATE reactor.tasks \
                 SET status = 'queued', run_at = now() + make_interval(secs => $2), \
                     lease_until = NULL, last_error = $3 \
                 WHERE id = $1",
            )
            .bind(task.id)
            .bind(backoff_secs(task.attempts))
            .bind(err)
            .execute(tx.deref_mut())
            .await?;
        }
        Err(err) => {
            sqlx::query(
                "UPDATE reactor.tasks SET status = 'dead', lease_until = NULL, last_error = $2 WHERE id = $1",
            )
            .bind(task.id)
            .bind(err)
            .execute(tx.deref_mut())
            .await?;
        }
    }
    Ok(())
}

pub async fn prune_done(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> Result<u64, sqlx::Error> {
    let result = sqlx::query(
        "DELETE FROM reactor.tasks WHERE status = 'done' AND created_at < now() - interval '24 hours'",
    )
    .execute(tx.deref_mut())
    .await?;
    Ok(result.rows_affected())
}

fn unique_violation(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .and_then(|db| db.code().map(|code| code == "23505"))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_until_the_cap() {
        assert_eq!(backoff_secs(1), 2);
        assert_eq!(backoff_secs(2), 4);
        assert_eq!(backoff_secs(3), 8);
        assert_eq!(backoff_secs(6), 60);
        assert_eq!(backoff_secs(7), 60);
    }

    async fn pool() -> Option<PgPool> {
        let Ok(pool) = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_secs(2))
            .connect("postgres://reactor:reactor@127.0.0.1:5440/reactor")
            .await
        else {
            return None;
        };
        let mut conn = pool.acquire().await.unwrap();
        sqlx::query("SELECT pg_advisory_lock(4815162344)")
            .execute(conn.deref_mut())
            .await
            .unwrap();
        let applied = sqlx::raw_sql(include_str!("../../../sql/control/013_tasks.sql"))
            .execute(conn.deref_mut())
            .await;
        let _ = sqlx::query("SELECT pg_advisory_unlock(4815162344)")
            .execute(conn.deref_mut())
            .await;
        match applied {
            Ok(_) => {}
            Err(err) if unique_violation(&err) => {}
            Err(err) => panic!("{err}"),
        }
        Some(pool)
    }

    async fn isolate(
        pool: &PgPool,
    ) -> sqlx::Transaction<'static, sqlx::Postgres> {
        let mut tx = pool.begin().await.unwrap();
        sqlx::query("LOCK TABLE reactor.tasks IN ACCESS EXCLUSIVE MODE")
            .execute(tx.deref_mut())
            .await
            .unwrap();
        sqlx::query(
            "UPDATE reactor.tasks \
             SET run_at = now() + interval '100 years', \
                 lease_until = now() + interval '100 years' \
             WHERE status IN ('queued', 'running')",
        )
        .execute(tx.deref_mut())
        .await
        .unwrap();
        tx
    }

    #[tokio::test]
    async fn claim_keeps_a_live_lease_and_reclaims_an_expired_one() {
        let Some(pool) = pool().await else {
            return;
        };
        let mut tx = isolate(&pool).await;
        let leased = Uuid::new_v4();
        let expired = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO reactor.tasks (id, kind, payload, run_at, attempts, max_attempts, status, lease_until) \
             VALUES ($1, 'claim-test', '{}', now(), 1, 3, 'running', now() + interval '1 hour')",
        )
        .bind(leased)
        .execute(tx.deref_mut())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO reactor.tasks (id, kind, payload, run_at, attempts, max_attempts, status, lease_until) \
             VALUES ($1, 'claim-test', '{}', '-infinity', 1, 3, 'running', now() - interval '1 hour')",
        )
        .bind(expired)
        .execute(tx.deref_mut())
        .await
        .unwrap();
        let claimed = claim_one(&mut tx).await.unwrap().unwrap();
        assert_eq!(claimed.id, expired);
        assert_eq!(claimed.attempts, 2);
        let again = claim_one(&mut tx).await.unwrap();
        assert!(again.is_none());
        let leased_attempts: i32 =
            sqlx::query_scalar("SELECT attempts FROM reactor.tasks WHERE id = $1")
                .bind(leased)
                .fetch_one(tx.deref_mut())
                .await
                .unwrap();
        assert_eq!(leased_attempts, 1);
        tx.rollback().await.unwrap();
    }

    #[tokio::test]
    async fn duplicate_dedupe_key_conflicts_while_queued() {
        let Some(pool) = pool().await else {
            return;
        };
        let mut tx = isolate(&pool).await;
        let key = format!("dedupe-{}", Uuid::new_v4());
        enqueue_tx(
            &mut tx,
            "dedupe-test",
            None,
            serde_json::json!({}),
            Utc::now() - chrono::Duration::hours(1),
            3,
            Some(&key),
        )
        .await
        .unwrap();
        let err = enqueue_tx(
            &mut tx,
            "dedupe-test",
            None,
            serde_json::json!({}),
            Utc::now() - chrono::Duration::hours(1),
            3,
            Some(&key),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, EnqueueError::Conflict));
        tx.rollback().await.unwrap();
    }

    #[tokio::test]
    async fn unknown_kind_is_dead_immediately() {
        let Some(pool) = pool().await else {
            return;
        };
        let mut tx = isolate(&pool).await;
        let id = enqueue_tx(
            &mut tx,
            "no-such-kind",
            None,
            serde_json::json!({}),
            Utc::now() - chrono::Duration::hours(1),
            5,
            None,
        )
        .await
        .unwrap();
        let task = claim_one(&mut tx).await.unwrap().unwrap();
        assert_eq!(task.id, id);
        settle(&mut tx, &task, false, Err("unused".into()))
            .await
            .unwrap();
        let status: String = sqlx::query_scalar("SELECT status FROM reactor.tasks WHERE id = $1")
            .bind(id)
            .fetch_one(tx.deref_mut())
            .await
            .unwrap();
        assert_eq!(status, "dead");
        tx.rollback().await.unwrap();
    }

    #[tokio::test]
    async fn prune_removes_done_rows_older_than_a_day() {
        let Some(pool) = pool().await else {
            return;
        };
        let mut tx = isolate(&pool).await;
        let old = Uuid::new_v4();
        let fresh = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO reactor.tasks (id, kind, payload, run_at, attempts, max_attempts, status, created_at) \
             VALUES ($1, 'prune-test', '{}', now(), 1, 1, 'done', now() - interval '25 hours')",
        )
        .bind(old)
        .execute(tx.deref_mut())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO reactor.tasks (id, kind, payload, run_at, attempts, max_attempts, status, created_at) \
             VALUES ($1, 'prune-test', '{}', now(), 1, 1, 'done', now())",
        )
        .bind(fresh)
        .execute(tx.deref_mut())
        .await
        .unwrap();
        prune_done(&mut tx).await.unwrap();
        let old_count: i64 = sqlx::query_scalar("SELECT count(*) FROM reactor.tasks WHERE id = $1")
            .bind(old)
            .fetch_one(tx.deref_mut())
            .await
            .unwrap();
        let fresh_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM reactor.tasks WHERE id = $1")
                .bind(fresh)
                .fetch_one(tx.deref_mut())
                .await
                .unwrap();
        assert_eq!(old_count, 0);
        assert_eq!(fresh_count, 1);
        tx.rollback().await.unwrap();
    }
}
