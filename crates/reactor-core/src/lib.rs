mod kernel;
mod tasks;

use async_trait::async_trait;
use reactor_storage::BlobStore;
use serde_json::Value;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

pub use kernel::{
    hook_lock_key, run_hooks, BoxFuture, ExtensionInfo, ExtensionSql, Hook, Kernel,
};
pub use tasks::{backoff_secs, EnqueueError, FnInvoke, StoredTask, TaskHandler, TaskRow, Tasks};

#[derive(Clone)]
pub struct OpenedProject {
    pub id: Uuid,
    pub pref: String,
    pub schema: String,
    pub pool: PgPool,
}

#[derive(Debug)]
pub enum ProjectError {
    Unauthorized,
    Forbidden,
    Unavailable(String),
}

#[async_trait]
pub trait Invoker: Send + Sync {
    async fn invoke(&self, project_id: Uuid, name: String, body: Vec<u8>) -> Result<Vec<u8>, String>;
}

#[async_trait]
pub trait ProjectLog: Send + Sync {
    async fn record(&self, project_id: Uuid, kind: String, name: String, status: i32, message: String);
}

#[async_trait]
pub trait Projects: Send + Sync {
    async fn open_service(
        &self,
        authorization: Option<String>,
        host: Option<String>,
    ) -> Result<OpenedProject, ProjectError>;

    async fn list(&self) -> Result<Vec<OpenedProject>, ProjectError>;
}

#[derive(Clone)]
pub struct TaskCtx {
    pub pool: PgPool,
    pub blobs: Arc<dyn BlobStore>,
    pub invoker: Arc<dyn Invoker>,
    pub http: reqwest::Client,
    pub projects: Arc<dyn Projects>,
    pub log: Arc<dyn ProjectLog>,
}

#[derive(Clone)]
pub struct Ctx<S> {
    pub pool: PgPool,
    pub blobs: Arc<dyn BlobStore>,
    pub invoker: Arc<dyn Invoker>,
    pub tasks: Tasks,
    pub http: reqwest::Client,
    pub extensions: Arc<Vec<ExtensionInfo>>,
    pub projects: Arc<dyn Projects>,
    pub log: Arc<dyn ProjectLog>,
    pub hooks: Arc<Vec<Hook>>,
    pub app: S,
}

impl<S> Ctx<S> {
    pub fn task_ctx(&self) -> TaskCtx {
        TaskCtx {
            pool: self.pool.clone(),
            blobs: self.blobs.clone(),
            invoker: self.invoker.clone(),
            http: self.http.clone(),
            projects: self.projects.clone(),
            log: self.log.clone(),
        }
    }

    pub async fn tick(&self, force: bool) -> Result<usize, String> {
        let ctx = self.task_ctx();
        let ran = self.tasks.dispatch(&ctx, 16).await?;
        run_hooks(&self.pool, &self.hooks, &ctx, force).await?;
        Ok(ran)
    }
}

pub fn extension_list(info: &[ExtensionInfo]) -> Value {
    serde_json::json!(info)
}
