use crate::store::{self, QueueError};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use reactor_core::{Ctx, OpenedProject};
use serde::Deserialize;
use serde_json::{json, Value};

pub fn router<S>() -> Router<Ctx<S>>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/queue/v1/queues", post(create::<S>).get(list::<S>))
        .route("/queue/v1/queues/{name}/send", post(send::<S>))
        .route("/queue/v1/queues/{name}/read", post(read::<S>))
        .route("/queue/v1/queues/{name}/delete", post(delete_msg::<S>))
        .route("/queue/v1/queues/{name}/archive", post(archive::<S>))
        .route("/queue/v1/queues/{name}/peek", get(peek::<S>))
        .route(
            "/queue/v1/queues/{name}/subscriptions",
            post(subscribe::<S>).delete(unsubscribe::<S>),
        )
}

async fn open<S>(ctx: &Ctx<S>, headers: &HeaderMap) -> Result<OpenedProject, QueueError>
where
    S: Clone + Send + Sync + 'static,
{
    let authorization = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    ctx.projects
        .open_service(authorization, host)
        .await
        .map_err(QueueError::from)
}

#[derive(Deserialize)]
struct CreateBody {
    name: String,
}

async fn create<S>(
    State(ctx): State<Ctx<S>>,
    headers: HeaderMap,
    Json(body): Json<CreateBody>,
) -> Result<(StatusCode, Json<Value>), QueueError>
where
    S: Clone + Send + Sync + 'static,
{
    let project = open(&ctx, &headers).await?;
    store::create_queue(&project, &body.name).await?;
    Ok((StatusCode::CREATED, Json(json!({"name": body.name}))))
}

async fn list<S>(
    State(ctx): State<Ctx<S>>,
    headers: HeaderMap,
) -> Result<Json<Vec<Value>>, QueueError>
where
    S: Clone + Send + Sync + 'static,
{
    let project = open(&ctx, &headers).await?;
    Ok(Json(store::list_queues(&project).await?))
}

#[derive(Deserialize)]
struct SendBody {
    message: Value,
    #[serde(default)]
    delay_secs: i64,
}

async fn send<S>(
    State(ctx): State<Ctx<S>>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(body): Json<SendBody>,
) -> Result<(StatusCode, Json<Value>), QueueError>
where
    S: Clone + Send + Sync + 'static,
{
    let project = open(&ctx, &headers).await?;
    let msg_id = store::send(&project, &name, body.message, body.delay_secs).await?;
    Ok((StatusCode::CREATED, Json(json!({"msg_id": msg_id}))))
}

#[derive(Deserialize)]
struct ReadBody {
    #[serde(default = "default_vt")]
    vt_secs: i32,
    #[serde(default = "default_qty")]
    qty: i32,
}

fn default_vt() -> i32 {
    30
}

fn default_qty() -> i32 {
    1
}

async fn read<S>(
    State(ctx): State<Ctx<S>>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(body): Json<ReadBody>,
) -> Result<Json<Vec<Value>>, QueueError>
where
    S: Clone + Send + Sync + 'static,
{
    let project = open(&ctx, &headers).await?;
    Ok(Json(
        store::read(&project, &name, body.vt_secs, body.qty).await?,
    ))
}

#[derive(Deserialize)]
struct IdBody {
    msg_id: i64,
}

async fn delete_msg<S>(
    State(ctx): State<Ctx<S>>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(body): Json<IdBody>,
) -> Result<StatusCode, QueueError>
where
    S: Clone + Send + Sync + 'static,
{
    let project = open(&ctx, &headers).await?;
    store::remove(&project, &name, body.msg_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn archive<S>(
    State(ctx): State<Ctx<S>>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(body): Json<IdBody>,
) -> Result<StatusCode, QueueError>
where
    S: Clone + Send + Sync + 'static,
{
    let project = open(&ctx, &headers).await?;
    store::archive(&project, &name, body.msg_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn peek<S>(
    State(ctx): State<Ctx<S>>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<Vec<Value>>, QueueError>
where
    S: Clone + Send + Sync + 'static,
{
    let project = open(&ctx, &headers).await?;
    Ok(Json(store::peek(&project, &name).await?))
}

#[derive(Deserialize)]
struct SubBody {
    function_name: String,
    vt_secs: i32,
    qty: i32,
    max_reads: i32,
}

async fn subscribe<S>(
    State(ctx): State<Ctx<S>>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(body): Json<SubBody>,
) -> Result<StatusCode, QueueError>
where
    S: Clone + Send + Sync + 'static,
{
    let project = open(&ctx, &headers).await?;
    store::subscribe(
        &project,
        &name,
        &body.function_name,
        body.vt_secs,
        body.qty,
        body.max_reads,
    )
    .await?;
    Ok(StatusCode::CREATED)
}

#[derive(Deserialize)]
struct UnsubBody {
    function_name: String,
}

async fn unsubscribe<S>(
    State(ctx): State<Ctx<S>>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(body): Json<UnsubBody>,
) -> Result<StatusCode, QueueError>
where
    S: Clone + Send + Sync + 'static,
{
    let project = open(&ctx, &headers).await?;
    store::unsubscribe(&project, &name, &body.function_name).await?;
    Ok(StatusCode::NO_CONTENT)
}

impl axum::response::IntoResponse for QueueError {
    fn into_response(self) -> axum::response::Response {
        let status = self.status();
        let body = Json(json!({"error": self.message()}));
        (status, body).into_response()
    }
}
