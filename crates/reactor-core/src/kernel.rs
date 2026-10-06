use crate::tasks::prune_done;
use crate::TaskCtx;
use axum::Router;
use chrono::{DateTime, Utc};
use std::ops::DerefMut;
use serde::Serialize;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

pub type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

#[derive(Clone, Debug, Serialize)]
pub struct ExtensionInfo {
    pub name: String,
    pub routes: Vec<String>,
}

#[derive(Clone)]
pub struct ExtensionSql {
    pub version: String,
    pub body: String,
}

pub struct Hook {
    pub name: String,
    pub every: Duration,
    pub run: Arc<dyn Fn(TaskCtx) -> BoxFuture<Result<(), String>> + Send + Sync>,
}

pub struct Kernel<S> {
    pub routes: Router<crate::Ctx<S>>,
    pub handlers: Vec<(String, Arc<dyn crate::TaskHandler>)>,
    pub hooks: Vec<Hook>,
    pub sql: Vec<ExtensionSql>,
    pub info: Vec<ExtensionInfo>,
}

impl<S> Kernel<S>
where
    S: Clone + Send + Sync + 'static,
{
    pub fn new() -> Self {
        Self {
            routes: Router::new(),
            handlers: Vec::new(),
            hooks: Vec::new(),
            sql: Vec::new(),
            info: Vec::new(),
        }
    }

    pub fn merge(&mut self, router: Router<crate::Ctx<S>>) {
        self.routes = std::mem::take(&mut self.routes).merge(router);
    }

    pub fn handle(&mut self, kind: impl Into<String>, handler: Arc<dyn crate::TaskHandler>) {
        self.handlers.push((kind.into(), handler));
    }

    pub fn hook(&mut self, hook: Hook) {
        self.hooks.push(hook);
    }

    pub fn install_core(&mut self) {
        self.hook(Hook {
            name: "sessions".into(),
            every: Duration::from_secs(60),
            run: Arc::new(|ctx| {
                Box::pin(async move {
                    sqlx::query("DELETE FROM reactor.sessions WHERE expires_at < now()")
                        .execute(&ctx.pool)
                        .await
                        .map(|_| ())
                        .map_err(|err| err.to_string())
                })
            }),
        });
        self.hook(Hook {
            name: "prune".into(),
            every: Duration::from_secs(60),
            run: Arc::new(|ctx| {
                Box::pin(async move {
                    let mut tx = ctx.pool.begin().await.map_err(|err| err.to_string())?;
                    prune_done(&mut tx).await.map_err(|err| err.to_string())?;
                    tx.commit().await.map_err(|err| err.to_string())?;
                    Ok(())
                })
            }),
        });
    }
}

impl<S> Default for Kernel<S>
where
    S: Clone + Send + Sync + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

pub fn hook_lock_key(name: &str) -> i64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in name.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let mut key = (hash as i64) & 0x7fff_ffff_ffff_ff00;
    if key == 0 || key == 4_815_162_342 || key == 4_815_162_343 {
        key = key.wrapping_add(64);
    }
    key
}

pub async fn run_hooks(pool: &sqlx::PgPool, hooks: &[Hook], ctx: &TaskCtx, force: bool) -> Result<(), String> {
    for hook in hooks {
        let mut tx = pool.begin().await.map_err(|err| err.to_string())?;
        let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1)")
            .bind(hook_lock_key(&hook.name))
            .fetch_one(tx.deref_mut())
            .await
            .map_err(|err| err.to_string())?;
        if !locked {
            continue;
        }
        let last: Option<DateTime<Utc>> =
            sqlx::query_scalar("SELECT last_run FROM reactor.ticks WHERE name = $1")
                .bind(&hook.name)
                .fetch_optional(tx.deref_mut())
                .await
                .map_err(|err| err.to_string())?;
        let due = force
            || last
                .map(|stamp| Utc::now().signed_duration_since(stamp) >= chrono::Duration::from_std(hook.every).unwrap_or(chrono::Duration::seconds(1)))
                .unwrap_or(true);
        if !due {
            tx.commit().await.map_err(|err| err.to_string())?;
            continue;
        }
        sqlx::query(
            "INSERT INTO reactor.ticks (name, last_run) VALUES ($1, now()) \
             ON CONFLICT (name) DO UPDATE SET last_run = now()",
        )
        .bind(&hook.name)
        .execute(tx.deref_mut())
        .await
        .map_err(|err| err.to_string())?;
        tx.commit().await.map_err(|err| err.to_string())?;
        (hook.run)(ctx.clone()).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::hook_lock_key;

    #[test]
    fn hook_locks_stay_off_the_cron_and_migrate_keys() {
        for name in ["cron", "sites", "sessions", "prune", "queue"] {
            let key = hook_lock_key(name);
            assert_ne!(key, 4_815_162_342, "{name}");
            assert_ne!(key, 4_815_162_343, "{name}");
            assert_ne!(key, 0, "{name}");
        }
    }
}
