mod agent;
mod buckets;
mod config;
mod console;
mod db;
mod email;
mod factors;
mod flows;
mod http_edge;
mod lambda;
mod mfa;
mod oauth;
mod project_auth;
mod sites_proc;
mod sql;

use async_trait::async_trait;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, FromRef, Path, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use axum::{Json, Router};
use reactor_auth::{
    constant_time_eq, hash_password, random_token, token_hash, verify_password, IdentityProvider,
    InternalIdentity, JwtIssuer,
};
use reactor_core::{Ctx, FnInvoke, Invoker, OpenedProject, ProjectError, Projects, Tasks};
use reactor_functions::{
    scrub_env, zip_hash, AwsLambdaPublisher, BunRuntime, FakePublisher, FunctionPublisher,
};
use reactor_identity::{project_ref_from_host, Identity, ProjectRef, Role};
use reactor_sites::{
    content_type_for, expected_txt, safe_site_path, site_upload_target, txt_from_doh, txt_matches,
    verification_name, SITE_BODY_LIMIT,
};
use reactor_storage::{
    clamp_expires, object_key, reserved_bucket, verify_signature, BlobStore,
    FsStore, S3Store,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sites_proc::{safe_site_command, SiteSupervisor};
use sqlx::PgPool;
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

pub use config::Config;
pub use lambda::{from_apigw_v2, run_lambda, to_apigw_v2};
pub use sql::superware_launch_sql;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub pool: PgPool,
    pub dedicated: Option<PgPool>,
    pub issuer: Arc<JwtIssuer>,
    pub identity: Arc<InternalIdentity>,
    pub blobs: Arc<dyn BlobStore>,
    pub bun: Arc<BunRuntime>,
    pub publisher: Arc<dyn FunctionPublisher>,
    pub http: reqwest::Client,
    pub sites: Arc<SiteSupervisor>,
    pub extension_sql: Arc<Vec<reactor_core::ExtensionSql>>,
    pub extensions: Arc<Vec<reactor_core::ExtensionInfo>>,
    pub project_pools: sql::ProjectPools,
    pub bucket_flags: Arc<buckets::BucketFlags>,
}

pub type AppCtx = Ctx<AppState>;

impl FromRef<AppCtx> for AppState {
    fn from_ref(input: &AppCtx) -> Self {
        input.app.clone()
    }
}

pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }
    pub(crate) fn text(&self) -> &str {
        &self.message
    }
    fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, message)
    }
    fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, message)
    }
    fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, "not found")
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut response = (self.status, Json(json!({"error": self.message}))).into_response();
        if self.status == StatusCode::TOO_MANY_REQUESTS {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, "60".parse().unwrap());
        }
        response
    }
}

pub(crate) async fn limit_auth(
    state: &AppState,
    headers: &HeaderMap,
    project_id: Uuid,
    kind: &str,
    max: i64,
) -> Result<(), ApiError> {
    let ip = headers
        .get("x-reactor-client-ip")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("unknown");
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM reactor.auth_attempts \
         WHERE project_id = $1 AND ip = $2 AND kind = $3 AND created_at > now() - interval '1 minute'",
    )
    .bind(project_id)
    .bind(ip)
    .bind(kind)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    if count >= max {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "too many requests",
        ));
    }
    sqlx::query(
        "INSERT INTO reactor.auth_attempts (id, project_id, ip, kind) VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::new_v4())
    .bind(project_id)
    .bind(ip)
    .bind(kind)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(())
}

fn internal(err: impl std::fmt::Display) -> ApiError {
    tracing::error!("{err}");
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal error")
}

struct Resolved {
    project_id: Uuid,
    pref: String,
    database_url: Option<String>,
    identity: Option<Identity>,
}

pub fn pool_max_connections(mode: &str) -> u32 {
    if mode == "lambda" {
        2
    } else {
        8
    }
}

pub async fn build_app(config: Config) -> anyhow::Result<Router> {
    if config.provider.is_empty() || config.provider != "internal" {
        anyhow::bail!("auth provider must be internal");
    }
    if config.database_url.is_empty() {
        anyhow::bail!("database url is required");
    }
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(pool_max_connections(&config.mode))
        .connect(&config.database_url)
        .await?;
    let sql_dir = PathBuf::from(&config.sql_dir);
    db::apply_control(&pool, &sql_dir, &config.authenticator_password).await?;
    let dedicated = if let Some(url) = &config.dedicated_database_url {
        let dedicated = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect(url)
            .await?;
        db::apply_control(&dedicated, &sql_dir, &config.authenticator_password).await?;
        Some(dedicated)
    } else {
        None
    };
    db::sync_schemas(&pool, dedicated.as_ref()).await?;

    let issuer = Arc::new(match &config.jwt_private_pem_file {
        Some(path) => JwtIssuer::from_private_pem_file(std::path::Path::new(path))?,
        None => {
            let dir = PathBuf::from(&config.jwt_dir);
            if dir.join("private.pem").exists() {
                JwtIssuer::from_private_pem_file(&dir.join("private.pem"))?
            } else {
                JwtIssuer::generate(&dir)?
            }
        }
    });
    sql::ensure_all(
        &pool,
        dedicated.as_ref(),
        config.dedicated_database_url.as_deref(),
        issuer.seal_key(),
    )
    .await?;
    let identity = Arc::new(InternalIdentity {
        pool: pool.clone(),
        issuer: (*issuer).clone(),
    });
    let blobs: Arc<dyn BlobStore> = if config.storage_backend == "s3" {
        let public = if config.storage_public_endpoint.is_empty() {
            config.storage_endpoint.clone()
        } else {
            config.storage_public_endpoint.clone()
        };
        let store = S3Store::new(
            config.storage_bucket.clone(),
            config.storage_endpoint.clone(),
            public,
            config.storage_region.clone(),
            config.storage_access_key.clone(),
            config.storage_secret_key.clone(),
        )
        .await?;
        store.ensure_bucket().await?;
        Arc::new(store)
    } else {
        let root = PathBuf::from(&config.storage_fs_root);
        std::fs::create_dir_all(&root)?;
        Arc::new(FsStore::new(
            root,
            config.public_url.clone(),
            config.storage_sign_secret.clone(),
        ))
    };
    let bun = Arc::new(BunRuntime::new(
        blobs.clone(),
        PathBuf::from(&config.functions_workdir),
        config.functions_bun.clone(),
    ));
    let publisher: Arc<dyn FunctionPublisher> = if config.functions_publisher == "aws" {
        Arc::new(
            AwsLambdaPublisher::new(
                config.lambda_role_arn.clone(),
                config.storage_region.clone(),
                config.lambda_layer_arn.clone(),
            )
            .await?,
        )
    } else {
        Arc::new(FakePublisher::new())
    };
    let sites = Arc::new(SiteSupervisor::new(
        PathBuf::from(&config.sites_workdir),
        std::time::Duration::from_secs(config.sites_idle_secs),
    ));
    let mut kernel = reactor_core::Kernel::<AppState>::new();
    kernel.install_core();
    for name in &config.extensions {
        match name.as_str() {
            "queue" => reactor_ext_queue::register(&mut kernel),
            other => anyhow::bail!("unknown extension {other}"),
        }
    }
    let extension_sql = Arc::new(std::mem::take(&mut kernel.sql));
    let extension_info = Arc::new(std::mem::take(&mut kernel.info));
    let state = AppState {
        config: Arc::new(config),
        pool,
        dedicated,
        issuer,
        identity,
        blobs,
        bun,
        publisher,
        http: reqwest::Client::new(),
        sites: sites.clone(),
        extension_sql,
        extensions: extension_info,
        project_pools: sql::ProjectPools::default(),
        bucket_flags: Arc::new(buckets::BucketFlags::default()),
    };
    let cron_state = state.clone();
    kernel.hook(reactor_core::Hook {
        name: "cron".into(),
        every: std::time::Duration::from_secs(60),
        run: Arc::new(move |_| {
            let state = cron_state.clone();
            Box::pin(async move {
                run_cron(&state)
                    .await
                    .map(|_| ())
                    .map_err(|err| err.text().to_string())
            })
        }),
    });
    kernel.hook(reactor_core::Hook {
        name: "sites".into(),
        every: std::time::Duration::from_secs(1),
        run: Arc::new(move |_| {
            let sites = sites.clone();
            Box::pin(async move {
                sites.sweep().await;
                Ok(())
            })
        }),
    });
    let tasks = Tasks::new(state.pool.clone());
    tasks.register("fn.invoke", Arc::new(FnInvoke)).await;
    for (kind, handler) in std::mem::take(&mut kernel.handlers) {
        tasks.register(kind, handler).await;
    }
    let ctx = AppCtx {
        pool: state.pool.clone(),
        blobs: state.blobs.clone(),
        invoker: Arc::new(ServerInvoker {
            state: state.clone(),
        }),
        tasks,
        http: state.http.clone(),
        extensions: state.extensions.clone(),
        projects: Arc::new(ServerProjects {
            state: state.clone(),
        }),
        log: Arc::new(ServerLog {
            pool: state.pool.clone(),
        }),
        hooks: Arc::new(std::mem::take(&mut kernel.hooks)),
        app: state.clone(),
    };
    if ctx.app.config.mode != "lambda" {
        let clock = ctx.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                if let Err(err) = clock.tick(false).await {
                    tracing::error!("tick: {err}");
                }
            }
        });
    }
    let routes = std::mem::take(&mut kernel.routes);
    Ok(router(ctx, routes))
}

fn site_body_limit(mode: &str) -> usize {
    if mode == "lambda" {
        2 * 1024 * 1024
    } else {
        SITE_BODY_LIMIT
    }
}

pub fn router(ctx: AppCtx, extra: Router<AppCtx>) -> Router {
    let state = ctx.app.clone();
    let handler = state.config.handler.clone();
    let all = handler.is_empty();
    let upload_limit = site_body_limit(&state.config.mode);
    let mut app: Router<AppCtx> = Router::new();
    if all || handler == "platform" || handler == "fn" {
        app = app.route("/_internal/tick", post(tick_http));
    }
    if all || handler == "platform" {
        app = app
            .route("/health", get(health))
            .route(
                "/platform/v1/projects",
                get(list_projects).post(create_project),
            )
            .route("/platform/v1/projects/{pref}", patch(patch_project))
            .route("/platform/v1/migrate", post(migrate_http))
            .route("/platform/v1/domains/{host}/verify", post(confirm_domain))
            .route("/platform/v1/extensions", get(list_extensions));
    }
    if all {
        app = app.route("/data/v1/{*path}", axum::routing::any(proxy_data));
    }
    if all || handler == "platform" || handler == "sites" {
        app = app.route("/ask", get(ask));
    }
    if all || handler == "auth" {
        app = app
            .route("/auth/v1/signup", post(signup))
            .route("/auth/v1/token", post(token))
            .route("/auth/v1/logout", post(logout))
            .route("/auth/v1/user", get(user));
        app = flows::mount(app);
        app = project_auth::mount(app);
        app = factors::mount(app);
        app = oauth::mount(app);
    }
    if all || handler == "storage" {
        app = app
            .route("/storage/v1/object/presign", post(presign))
            .route("/storage/v1/object/public/{*path}", get(public_object))
            .route("/storage/v1/bucket", post(create_bucket))
            .route("/storage/v1/bucket/{name}", get(read_bucket))
            .route(
                "/storage/v1/signed",
                get(signed_get)
                    .put(signed_put)
                    .layer(DefaultBodyLimit::max(upload_limit)),
            );
    }
    if all || handler == "fn" {
        app = app
            .route("/fn/v1/_internal/cron", post(cron))
            .route("/fn/v1/_admin/functions/{name}", post(deploy_function))
            .route("/fn/v1/_admin/schedules", post(create_schedule))
            .route("/fn/v1/_admin/tasks/{id}", get(task_status))
            .route("/fn/v1/{name}/enqueue", post(enqueue_function))
            .route("/fn/v1/{name}", post(invoke_function));
    }
    if all || handler == "sites" {
        app = app
            .route(
                "/sites/v1/files/{*path}",
                put(upload_site).layer(DefaultBodyLimit::max(upload_limit)),
            )
            .route("/sites/v1/deployments", post(start_deployment))
            .route(
                "/sites/v1/deployments/{id}/files/{*path}",
                put(upload_deployment_file)
                    .post(post_deployment_file)
                    .layer(DefaultBodyLimit::max(upload_limit)),
            )
            .route("/sites/v1/deployments/{id}/finish", post(finish_deployment))
            .route("/sites/v1/env", get(get_site_env).put(put_site_env))
            .route("/sites/v1/deployments/{id}/fail", post(fail_deployment))
            .route("/sites/v1/routes", post(site_route))
            .route("/sites/v1/domains", post(create_domain))
            .route("/sites/v1/domains/{host}/verify", post(verify_domain))
            .route(
                "/sites/v1/domains/{host}",
                axum::routing::delete(delete_domain),
            )
            .fallback(site_fallback);
    }
    if all || handler == "platform" {
        app = console::mount(app);
    }
    let app = app
        .merge(extra)
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            console::console_key_guard,
        ))
        .with_state(ctx);
    http_edge::wrap(app, state)
}

async fn list_extensions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_operator(&state, &headers)?;
    Ok(Json(json!(state.extensions)))
}

#[derive(Deserialize)]
struct TickQuery {
    #[serde(default)]
    force: i32,
}

async fn tick_http(
    State(ctx): State<AppCtx>,
    headers: HeaderMap,
    Query(query): Query<TickQuery>,
) -> Result<Json<Value>, ApiError> {
    require_operator(&ctx.app, &headers)?;
    let tasks = ctx.tick(query.force == 1).await.map_err(internal)?;
    Ok(Json(json!({"ok": true, "tasks": tasks})))
}

async fn list_projects(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Vec<String>>, ApiError> {
    require_operator(&state, &headers)?;
    let refs: Vec<String> = sqlx::query_scalar("SELECT ref FROM reactor.projects ORDER BY ref")
        .fetch_all(&state.pool)
        .await
        .map_err(internal)?;
    Ok(Json(refs))
}

async fn health(State(state): State<AppState>) -> Response {
    match sqlx::query("SELECT 1").execute(&state.pool).await {
        Ok(_) => StatusCode::OK.into_response(),
        Err(err) => {
            tracing::error!("{err}");
            StatusCode::SERVICE_UNAVAILABLE.into_response()
        }
    }
}

fn require_operator(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(token) = bearer(headers) else {
        return Err(ApiError::unauthorized("operator token required"));
    };
    if !constant_time_eq(&token, &state.config.operator_token)
        || state.config.operator_token.is_empty()
    {
        return Err(ApiError::unauthorized("operator token required"));
    }
    Ok(())
}

pub(crate) fn bearer(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .map(|s| s.to_string())
}

fn host_of(headers: &HeaderMap) -> String {
    headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

async fn resolve_project(
    state: &AppState,
    headers: &HeaderMap,
    require_identity: bool,
) -> Result<Resolved, ApiError> {
    let host_ref = project_ref_from_host(&host_of(headers), &state.config.base_domain);
    let identity = if let Some(token) = bearer(headers) {
        Some(
            state
                .identity
                .resolve(&token)
                .await
                .map_err(|_| ApiError::unauthorized("invalid token"))?,
        )
    } else {
        None
    };
    if let (Some(host_ref), Some(identity)) = (&host_ref, &identity) {
        if host_ref != &identity.project_ref {
            return Err(ApiError::forbidden("host and token project mismatch"));
        }
    }
    let pref = host_ref
        .map(|r| r.as_str().to_string())
        .or_else(|| {
            identity
                .as_ref()
                .map(|i| i.project_ref.as_str().to_string())
        })
        .ok_or_else(|| ApiError::unauthorized("no project"))?;
    if require_identity && identity.is_none() {
        return Err(ApiError::unauthorized("token required"));
    }
    let row: Option<(Uuid, Option<String>)> =
        sqlx::query_as("SELECT id, database_url FROM reactor.projects WHERE ref = $1")
            .bind(&pref)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    let Some((id, database_url)) = row else {
        return Err(ApiError::unauthorized("unknown project"));
    };
    Ok(Resolved {
        project_id: id,
        pref,
        database_url,
        identity,
    })
}

fn require_service(resolved: &Resolved) -> Result<(), ApiError> {
    match &resolved.identity {
        Some(identity) if identity.role == Role::Service => Ok(()),
        _ => Err(ApiError::forbidden("service key required")),
    }
}

#[derive(Deserialize, Default)]
struct CreateProject {
    #[serde(default)]
    name: String,
}

#[derive(Serialize)]
struct ProjectOut {
    id: Uuid,
    #[serde(rename = "ref")]
    pref: String,
    anon_key: String,
    service_key: String,
}

async fn create_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateProject>,
) -> Result<Json<ProjectOut>, ApiError> {
    require_operator(&state, &headers)?;
    let name = body.name;
    let pref = generate_ref();
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO reactor.projects (id, ref, name) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(&pref)
        .bind(&name)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    let schema = format!("proj_{pref}");
    db::apply_project(
        &state.pool,
        std::path::Path::new(&state.config.sql_dir),
        id,
        &schema,
        state.extension_sql.as_slice(),
    )
    .await
    .map_err(internal)?;
    sql::ensure_project_role(&state.pool, &state.pool, state.issuer.seal_key(), id, &pref)
        .await
        .map_err(internal)?;
    let anon = state
        .issuer
        .sign(&pref, "anon", "anon", 60 * 60 * 24 * 365 * 10)
        .map_err(internal)?;
    let service = state
        .issuer
        .sign(&pref, "service", "service", 60 * 60 * 24 * 365 * 10)
        .map_err(internal)?;
    store_api_key(&state.pool, id, "anon", &anon)
        .await
        .map_err(internal)?;
    store_api_key(&state.pool, id, "service", &service)
        .await
        .map_err(internal)?;
    project_auth::insert_settings(&state.pool, id)
        .await
        .map_err(internal)?;
    reload(&state).await?;
    Ok(Json(ProjectOut {
        id,
        pref,
        anon_key: anon,
        service_key: service,
    }))
}

pub(crate) async fn store_api_key(
    pool: &PgPool,
    project_id: Uuid,
    role: &str,
    token: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO reactor.api_keys (id, project_id, role, token_hash) VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::new_v4())
    .bind(project_id)
    .bind(role)
    .bind(token_hash(token))
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) fn generate_ref() -> String {
    const ALPH: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut out = String::with_capacity(20);
    for _ in 0..20 {
        out.push(ALPH[(rand::random::<u8>() as usize) % ALPH.len()] as char);
    }
    out
}

#[derive(Deserialize)]
struct PatchProject {
    database_url: String,
}

async fn patch_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
    Json(body): Json<PatchProject>,
) -> Result<StatusCode, ApiError> {
    require_operator(&state, &headers)?;
    let pref = ProjectRef::parse(&pref)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid ref"))?;
    let row: Option<(Uuid,)> = sqlx::query_as("SELECT id FROM reactor.projects WHERE ref = $1")
        .bind(pref.as_str())
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?;
    let Some((id,)) = row else {
        return Err(ApiError::not_found());
    };
    let pool = pool_for(&state, Some(&body.database_url))
        .await
        .map_err(internal)?;
    db::apply_control(
        &pool,
        std::path::Path::new(&state.config.sql_dir),
        &state.config.authenticator_password,
    )
    .await
    .map_err(internal)?;
    db::apply_project(
        &pool,
        std::path::Path::new(&state.config.sql_dir),
        id,
        &pref.schema(),
        state.extension_sql.as_slice(),
    )
    .await
    .map_err(internal)?;
    sql::ensure_project_role(
        &state.pool,
        &pool,
        state.issuer.seal_key(),
        id,
        pref.as_str(),
    )
    .await
    .map_err(internal)?;
    state.project_pools.forget(id).await;
    sqlx::query("UPDATE reactor.projects SET database_url = $1 WHERE id = $2")
        .bind(&body.database_url)
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    reload(&state).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn pool_for(state: &AppState, url: Option<&str>) -> anyhow::Result<PgPool> {
    let Some(url) = url else {
        return Ok(state.pool.clone());
    };
    if state.config.dedicated_database_url.as_deref() == Some(url) {
        if let Some(pool) = &state.dedicated {
            return Ok(pool.clone());
        }
    }
    Ok(sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(url)
        .await?)
}

pub(crate) async fn reload(state: &AppState) -> Result<(), ApiError> {
    db::sync_schemas(&state.pool, state.dedicated.as_ref())
        .await
        .map_err(internal)?;
    tokio::time::sleep(std::time::Duration::from_millis(700)).await;
    Ok(())
}

#[derive(Deserialize, Default)]
struct MigrateBody {
    #[serde(default)]
    all: bool,
    #[serde(default)]
    #[serde(rename = "ref")]
    pref: Option<String>,
}

async fn migrate_http(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MigrateBody>,
) -> Result<Json<Value>, ApiError> {
    if body.all {
        if body.pref.is_some() {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "ref and all are exclusive",
            ));
        }
        require_operator(&state, &headers)?;
        db::migrate_all(
            &state.pool,
            state.dedicated.as_ref(),
            std::path::Path::new(&state.config.sql_dir),
            state.config.dedicated_database_url.as_deref(),
            state.extension_sql.as_slice(),
            state.issuer.seal_key(),
        )
        .await
        .map_err(internal)?;
        reload(&state).await?;
        return Ok(Json(json!({"ok": true})));
    }
    let resolved = resolve_project(&state, &headers, true).await?;
    require_service(&resolved)?;
    let pool = pool_for(&state, resolved.database_url.as_deref())
        .await
        .map_err(internal)?;
    db::apply_project(
        &pool,
        std::path::Path::new(&state.config.sql_dir),
        resolved.project_id,
        &format!("proj_{}", resolved.pref),
        state.extension_sql.as_slice(),
    )
    .await
    .map_err(internal)?;
    sql::ensure_project_role(
        &state.pool,
        &pool,
        state.issuer.seal_key(),
        resolved.project_id,
        &resolved.pref,
    )
    .await
    .map_err(internal)?;
    reload(&state).await?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct SignupBody {
    email: String,
    password: String,
}

#[derive(Serialize)]
pub(crate) struct TokenOut {
    access_token: String,
    refresh_token: String,
    user: UserOut,
}

#[derive(Serialize)]
struct UserOut {
    id: Uuid,
    email: String,
}

async fn signup(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SignupBody>,
) -> Result<Response, ApiError> {
    let resolved = resolve_project(&state, &headers, false).await?;
    limit_auth(&state, &headers, resolved.project_id, "signup", 30).await?;
    if body.password.len() < 8 {
        console::record_log(
            &state.pool,
            resolved.project_id,
            "auth",
            "signup",
            400,
            "password too short",
        )
        .await;
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "password too short"));
    }
    let email = body.email.trim().to_ascii_lowercase();
    let user_id = Uuid::new_v4();
    let hash = hash_password(&body.password).map_err(internal)?;
    let inserted = sqlx::query(
        "INSERT INTO reactor.users (id, project_id, email, password_hash) VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(resolved.project_id)
    .bind(&email)
    .bind(&hash)
    .execute(&state.pool)
    .await;
    if let Err(err) = inserted {
        if let sqlx::Error::Database(db) = &err {
            if db.code().as_deref() == Some("23505") {
                console::record_log(
                    &state.pool,
                    resolved.project_id,
                    "auth",
                    "signup",
                    409,
                    "email exists",
                )
                .await;
                return Err(ApiError::new(StatusCode::CONFLICT, "email exists"));
            }
        }
        return Err(internal(err));
    }
    console::record_log(&state.pool, resolved.project_id, "auth", "signup", 201, "").await;
    let (require_email, _) = project_auth::flags(&state, resolved.project_id).await?;
    if require_email {
        match project_auth::send_confirm(&state, resolved.project_id, user_id, &email).await {
            Ok(()) => {
                return Ok(Json(project_auth::pending(user_id, &email)).into_response());
            }
            Err(email::DeliverError::Unconfigured) => {
                let _ = sqlx::query("DELETE FROM reactor.users WHERE id = $1")
                    .bind(user_id)
                    .execute(&state.pool)
                    .await;
                return Err(ApiError::new(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "email is not configured",
                ));
            }
            Err(email::DeliverError::Failed) => {
                let _ = sqlx::query("DELETE FROM reactor.users WHERE id = $1")
                    .bind(user_id)
                    .execute(&state.pool)
                    .await;
                return Err(ApiError::new(
                    StatusCode::BAD_GATEWAY,
                    "email could not be sent",
                ));
            }
        }
    }
    issue_session(&state, resolved.project_id, &resolved.pref, user_id, email)
        .await
        .map(|body| body.into_response())
}

#[derive(Deserialize)]
struct TokenBody {
    email: Option<String>,
    password: Option<String>,
    refresh_token: Option<String>,
    code: Option<String>,
}

async fn token(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<TokenBody>,
) -> Result<Response, ApiError> {
    if let Some(refresh) = body.refresh_token.filter(|token| !token.is_empty()) {
        return refresh_session(&state, &refresh)
            .await
            .map(|body| body.into_response());
    }
    if let Some(code) = body.code.filter(|code| !code.is_empty()) {
        return oauth::exchange_code(&state, &headers, &code).await;
    }
    let resolved = resolve_project(&state, &headers, false).await?;
    limit_auth(&state, &headers, resolved.project_id, "token", 60).await?;
    let email = body.email.unwrap_or_default().trim().to_ascii_lowercase();
    let password = body.password.unwrap_or_default();
    let row: Option<(Uuid, Option<String>, bool)> = sqlx::query_as(
        "SELECT id, password_hash, email_verified_at IS NOT NULL FROM reactor.users WHERE project_id = $1 AND email = $2",
    )
    .bind(resolved.project_id)
    .bind(&email)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some((user_id, Some(password_hash), verified)) = row else {
        console::record_log(
            &state.pool,
            resolved.project_id,
            "auth",
            "token",
            401,
            "invalid credentials",
        )
        .await;
        return Err(ApiError::unauthorized("invalid credentials"));
    };
    if !verify_password(&password, &password_hash) {
        console::record_log(
            &state.pool,
            resolved.project_id,
            "auth",
            "token",
            401,
            "invalid credentials",
        )
        .await;
        return Err(ApiError::unauthorized("invalid credentials"));
    }
    let (require_email, require_mfa) = project_auth::flags(&state, resolved.project_id).await?;
    if require_email && !verified {
        console::record_log(
            &state.pool,
            resolved.project_id,
            "auth",
            "token",
            200,
            "verification required",
        )
        .await;
        return Ok(Json(project_auth::pending(user_id, &email)).into_response());
    }
    if require_mfa {
        console::record_log(
            &state.pool,
            resolved.project_id,
            "auth",
            "token",
            200,
            "mfa",
        )
        .await;
        return factors::after_password(&state, resolved.project_id, user_id).await;
    }
    console::record_log(&state.pool, resolved.project_id, "auth", "token", 200, "").await;
    issue_session(&state, resolved.project_id, &resolved.pref, user_id, email)
        .await
        .map(|body| body.into_response())
}

pub(crate) async fn issue_session(
    state: &AppState,
    project_id: Uuid,
    pref: &str,
    user_id: Uuid,
    email: String,
) -> Result<Json<TokenOut>, ApiError> {
    let access = state
        .issuer
        .sign(pref, &user_id.to_string(), "authenticated", 900)
        .map_err(internal)?;
    let refresh = random_token();
    sqlx::query(
        "INSERT INTO reactor.sessions (id, user_id, token_hash, expires_at) VALUES ($1, $2, $3, now() + interval '30 days')",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(token_hash(&refresh))
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    let _ = project_id;
    Ok(Json(TokenOut {
        access_token: access,
        refresh_token: refresh,
        user: UserOut { id: user_id, email },
    }))
}

async fn refresh_session(state: &AppState, refresh: &str) -> Result<Json<TokenOut>, ApiError> {
    let hash = token_hash(refresh);
    let row: Option<(Uuid, Uuid, String, String)> = sqlx::query_as(
        "SELECT u.id, u.project_id, u.email, p.ref \
         FROM reactor.sessions s \
         JOIN reactor.users u ON u.id = s.user_id \
         JOIN reactor.projects p ON p.id = u.project_id \
         WHERE s.token_hash = $1 AND s.expires_at > now()",
    )
    .bind(&hash)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some((user_id, project_id, email, pref)) = row else {
        return Err(ApiError::unauthorized("invalid refresh token"));
    };
    let mut tx = state.pool.begin().await.map_err(internal)?;
    let deleted = sqlx::query("DELETE FROM reactor.sessions WHERE token_hash = $1")
        .bind(&hash)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    if deleted.rows_affected() == 0 {
        return Err(ApiError::unauthorized("invalid refresh token"));
    }
    let new_refresh = random_token();
    sqlx::query(
        "INSERT INTO reactor.sessions (id, user_id, token_hash, expires_at) VALUES ($1, $2, $3, now() + interval '30 days')",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(token_hash(&new_refresh))
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    console::record_log(&state.pool, project_id, "auth", "refresh", 200, "").await;
    let access = state
        .issuer
        .sign(&pref, &user_id.to_string(), "authenticated", 900)
        .map_err(internal)?;
    Ok(Json(TokenOut {
        access_token: access,
        refresh_token: new_refresh,
        user: UserOut { id: user_id, email },
    }))
}

#[derive(Deserialize)]
struct LogoutBody {
    refresh_token: String,
}

async fn logout(
    State(state): State<AppState>,
    Json(body): Json<LogoutBody>,
) -> Result<StatusCode, ApiError> {
    let hash = token_hash(&body.refresh_token);
    let project_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT u.project_id FROM reactor.sessions s JOIN reactor.users u ON u.id = s.user_id WHERE s.token_hash = $1",
    )
    .bind(&hash)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    sqlx::query("DELETE FROM reactor.sessions WHERE token_hash = $1")
        .bind(&hash)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    if let Some(project_id) = project_id {
        console::record_log(&state.pool, project_id, "auth", "logout", 204, "").await;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn user(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    let resolved = resolve_project(&state, &headers, true).await?;
    let Some(identity) = resolved.identity else {
        return Err(ApiError::unauthorized("token required"));
    };
    let Some(user_id) = identity.user_id else {
        return Err(ApiError::unauthorized("user token required"));
    };
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT email, to_char(email_verified_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"') \
         FROM reactor.users WHERE id = $1 AND project_id = $2",
    )
    .bind(user_id)
    .bind(resolved.project_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some((email, email_verified_at)) = row else {
        console::record_log(
            &state.pool,
            resolved.project_id,
            "auth",
            "user",
            401,
            "user token required",
        )
        .await;
        return Err(ApiError::unauthorized("user token required"));
    };
    console::record_log(&state.pool, resolved.project_id, "auth", "user", 200, "").await;
    Ok(Json(serde_json::json!({
        "id": user_id,
        "email": email,
        "email_verified_at": email_verified_at,
    })))
}

async fn proxy_data(State(state): State<AppState>, req: Request) -> Result<Response, ApiError> {
    let headers = req.headers().clone();
    let resolved = resolve_project(&state, &headers, true).await?;
    let path = req.uri().path().trim_start_matches("/data/v1").to_string();
    let path = if path.is_empty() {
        "/".to_string()
    } else {
        path
    };
    let query = req
        .uri()
        .query()
        .map(|q| format!("?{q}"))
        .unwrap_or_default();
    let base = if resolved.database_url.is_some() {
        state.config.postgrest_dedicated_url.trim_end_matches('/')
    } else {
        state.config.postgrest_url.trim_end_matches('/')
    };
    let url = format!("{base}{path}{query}");
    let method = reqwest::Method::from_bytes(req.method().as_str().as_bytes())
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "bad method"))?;
    let body = axum::body::to_bytes(req.into_body(), 10_000_000)
        .await
        .map_err(internal)?;
    let mut outbound = state.http.request(method, &url);
    if let Some(token) = bearer(&headers) {
        outbound = outbound.header("authorization", format!("Bearer {token}"));
    }
    for name in ["content-type", "accept", "prefer", "range"] {
        if let Some(value) = headers.get(name) {
            outbound = outbound.header(name, value);
        }
    }
    let schema = format!("proj_{}", resolved.pref);
    outbound = outbound
        .header("accept-profile", &schema)
        .header("content-profile", &schema)
        .body(body.to_vec());
    let response = outbound.send().await.map_err(internal)?;
    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut out = Response::builder().status(status);
    for name in ["content-type", "content-range", "preference-applied"] {
        if let Some(value) = response.headers().get(name) {
            out = out.header(name, value);
        }
    }
    let bytes = response.bytes().await.map_err(internal)?;
    let name: String = path.trim_start_matches('/').chars().take(120).collect();
    console::record_log(
        &state.pool,
        resolved.project_id,
        "database",
        if name.is_empty() { "/" } else { &name },
        status.as_u16() as i32,
        "",
    )
    .await;
    Ok(out.body(Body::from(bytes)).unwrap())
}

#[derive(Deserialize)]
struct PresignBody {
    bucket: String,
    key: String,
    #[serde(default = "default_put")]
    method: String,
    #[serde(default)]
    expires_in: u64,
}

fn default_put() -> String {
    "PUT".into()
}

#[derive(Serialize)]
struct PresignOut {
    url: String,
    key: String,
}

fn storage_public_host(config: &Config, pref: &str) -> String {
    if config.storage_cdn_public_base.is_empty() {
        console::site_public_url(pref, &config.base_domain, &config.public_url)
    } else {
        config.storage_cdn_public_base.clone()
    }
}

async fn presign(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PresignBody>,
) -> Result<Json<PresignOut>, ApiError> {
    let resolved = resolve_project(&state, &headers, true).await?;
    if reserved_bucket(&body.bucket) {
        return Err(ApiError::forbidden("invalid object key"));
    }
    let key = object_key(&resolved.pref, &body.bucket, &body.key)
        .map_err(|_| ApiError::forbidden("invalid object key"))?;
    if !key.starts_with(&format!("{}/", resolved.pref)) {
        return Err(ApiError::forbidden("invalid object key"));
    }
    let identity = resolved
        .identity
        .as_ref()
        .ok_or_else(|| ApiError::unauthorized("token required"))?;
    let pool = pool_for(&state, resolved.database_url.as_deref())
        .await
        .map_err(internal)?;
    let write = !body.method.eq_ignore_ascii_case("GET");
    let allowed = buckets::authorize(
        &pool,
        &resolved.pref,
        identity,
        &body.bucket,
        &body.key,
        write,
    )
    .await?;
    if !allowed {
        return Err(ApiError::forbidden("storage policy denied"));
    }
    let expires = clamp_expires(&body.method, body.expires_in);
    let url = if write {
        state
            .blobs
            .presign_put(&key, expires)
            .await
            .map_err(internal)?
    } else {
        state
            .blobs
            .presign_get(&key, expires)
            .await
            .map_err(internal)?
    };
    Ok(Json(PresignOut { url, key }))
}

#[derive(Deserialize)]
struct CreateBucketBody {
    name: String,
    #[serde(default)]
    public: bool,
}

async fn create_bucket(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateBucketBody>,
) -> Result<Json<Value>, ApiError> {
    let resolved = resolve_project(&state, &headers, true).await?;
    require_service(&resolved)?;
    let pool = pool_for(&state, resolved.database_url.as_deref())
        .await
        .map_err(internal)?;
    buckets::create_bucket(&pool, &resolved.pref, &body.name, body.public).await?;
    state
        .bucket_flags
        .put(buckets::cache_key(&resolved.pref, &body.name), body.public);
    Ok(Json(json!({ "name": body.name, "public": body.public })))
}

async fn read_bucket(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let resolved = resolve_project(&state, &headers, true).await?;
    let pool = pool_for(&state, resolved.database_url.as_deref())
        .await
        .map_err(internal)?;
    let public = buckets::bucket_is_public(&pool, &resolved.pref, &name)
        .await?
        .ok_or_else(ApiError::not_found)?;
    let public_url_base = if public {
        Some(buckets::public_base(
            &state.config.storage_cdn_public_base,
            &storage_public_host(&state.config, &resolved.pref),
            &name,
        ))
    } else {
        None
    };
    Ok(Json(json!({
        "name": name,
        "public": public,
        "public_url_base": public_url_base,
    })))
}

async fn public_object(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(path): Path<String>,
) -> Result<Response, ApiError> {
    let host = host_of(&headers);
    let pref = project_ref_from_host(&host, &state.config.base_domain)
        .map(|pref| pref.as_str().to_string())
        .ok_or_else(ApiError::not_found)?;
    let (bucket, key) = buckets::split_public_path(&path)?;
    let cached = state
        .bucket_flags
        .get(&buckets::cache_key(&pref, &bucket));
    let public = if let Some(flag) = cached {
        flag
    } else {
        let row: Option<(Uuid, Option<String>)> =
            sqlx::query_as("SELECT id, database_url FROM reactor.projects WHERE ref = $1")
                .bind(&pref)
                .fetch_optional(&state.pool)
                .await
                .map_err(internal)?;
        let Some((_, database_url)) = row else {
            return Err(ApiError::not_found());
        };
        let pool = pool_for(&state, database_url.as_deref())
            .await
            .map_err(internal)?;
        let flag = buckets::bucket_is_public(&pool, &pref, &bucket)
            .await?
            .unwrap_or(false);
        state
            .bucket_flags
            .put(buckets::cache_key(&pref, &bucket), flag);
        flag
    };
    if !public {
        return Err(ApiError::not_found());
    }
    let object = object_key(&pref, &bucket, &key).map_err(|_| ApiError::not_found())?;
    let Some((bytes, content_type)) = state.blobs.get(&object).await.map_err(internal)? else {
        return Err(ApiError::not_found());
    };
    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (
                header::CACHE_CONTROL,
                "public, max-age=86400".to_string(),
            ),
        ],
        bytes,
    )
        .into_response())
}

#[derive(Deserialize)]
struct SignedQuery {
    op: String,
    key: String,
    exp: u64,
    sig: String,
}

async fn signed_get(
    State(state): State<AppState>,
    Query(q): Query<SignedQuery>,
) -> Result<Response, ApiError> {
    if q.op != "get"
        || !verify_signature(
            state.config.storage_sign_secret.as_bytes(),
            &q.op,
            &q.key,
            q.exp,
            &q.sig,
        )
    {
        return Err(ApiError::forbidden("bad signature"));
    }
    let Some((bytes, content_type)) = state.blobs.get(&q.key).await.map_err(internal)? else {
        return Err(ApiError::not_found());
    };
    Ok(([(header::CONTENT_TYPE, content_type)], bytes).into_response())
}

async fn signed_put(
    State(state): State<AppState>,
    Query(q): Query<SignedQuery>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<StatusCode, ApiError> {
    if q.op != "put"
        || !verify_signature(
            state.config.storage_sign_secret.as_bytes(),
            &q.op,
            &q.key,
            q.exp,
            &q.sig,
        )
    {
        return Err(ApiError::forbidden("bad signature"));
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream");
    state
        .blobs
        .put(&q.key, body, content_type)
        .await
        .map_err(internal)?;
    Ok(StatusCode::OK)
}

#[derive(Deserialize)]
pub(crate) struct PromoteFlag {
    #[serde(default = "promote_by_default")]
    pub promote: bool,
}

fn promote_by_default() -> bool {
    true
}

async fn deploy_function(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Query(flag): Query<PromoteFlag>,
    body: axum::body::Bytes,
) -> Result<Json<Value>, ApiError> {
    let resolved = resolve_project(&state, &headers, true).await?;
    require_service(&resolved)?;
    let version = deploy_zip(
        &state,
        resolved.project_id,
        &resolved.pref,
        &name,
        body.clone(),
        flag.promote,
    )
    .await?;
    Ok(Json(
        json!({ "name": name, "version": version, "sha256": zip_hash(&body), "promoted": flag.promote }),
    ))
}

pub(crate) async fn deploy_zip(
    state: &AppState,
    project_id: Uuid,
    pref: &str,
    name: &str,
    body: axum::body::Bytes,
    promote: bool,
) -> Result<i32, ApiError> {
    if !valid_name(name) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid function name",
        ));
    }
    let previous: Option<i32> = sqlx::query_scalar(
        "SELECT MAX(version) FROM reactor.deployments WHERE project_id = $1 AND name = $2",
    )
    .bind(project_id)
    .bind(name)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    let version = previous.unwrap_or(0) + 1;
    let key = object_key(pref, "_functions", &format!("{name}/{version}.zip"))
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid function name"))?;
    state
        .blobs
        .put(&key, body.clone(), "application/zip")
        .await
        .map_err(internal)?;
    let mut tx = state.pool.begin().await.map_err(internal)?;
    sqlx::query(
        "INSERT INTO reactor.deployments (id, project_id, name, version, blob_key) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(Uuid::new_v4())
    .bind(project_id)
    .bind(name)
    .bind(version)
    .bind(&key)
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    if promote {
        sqlx::query("DELETE FROM reactor.function_pins WHERE project_id = $1 AND name = $2")
            .bind(project_id)
            .bind(name)
            .execute(&mut *tx)
            .await
            .map_err(internal)?;
    } else if let Some(previous) = previous {
        sqlx::query(
            "INSERT INTO reactor.function_pins (project_id, name, version) VALUES ($1, $2, $3) \
             ON CONFLICT (project_id, name) DO NOTHING",
        )
        .bind(project_id)
        .bind(name)
        .bind(previous)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    }
    tx.commit().await.map_err(internal)?;
    if promote {
        publish_lambda(state, pref, name, &body).await?;
    }
    Ok(version)
}

pub(crate) async fn pin_function(
    state: &AppState,
    project_id: Uuid,
    pref: &str,
    name: &str,
    version: i32,
) -> Result<(), ApiError> {
    if !valid_name(name) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid function name",
        ));
    }
    let key: Option<String> = sqlx::query_scalar(
        "SELECT blob_key FROM reactor.deployments WHERE project_id = $1 AND name = $2 AND version = $3",
    )
    .bind(project_id)
    .bind(name)
    .bind(version)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some(key) = key else {
        return Err(ApiError::not_found());
    };
    sqlx::query(
        "INSERT INTO reactor.function_pins (project_id, name, version) VALUES ($1, $2, $3) \
         ON CONFLICT (project_id, name) DO UPDATE SET version = EXCLUDED.version",
    )
    .bind(project_id)
    .bind(name)
    .bind(version)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    if state.config.functions_runtime == "lambda" {
        let stored = state.blobs.get(&key).await.map_err(internal)?;
        let Some((bytes, _)) = stored else {
            return Err(ApiError::not_found());
        };
        publish_lambda(state, pref, name, &bytes).await?;
    }
    Ok(())
}

pub(crate) async fn unpin_function(
    state: &AppState,
    project_id: Uuid,
    pref: &str,
    name: &str,
) -> Result<(), ApiError> {
    if !valid_name(name) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid function name",
        ));
    }
    sqlx::query("DELETE FROM reactor.function_pins WHERE project_id = $1 AND name = $2")
        .bind(project_id)
        .bind(name)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    if state.config.functions_runtime == "lambda" {
        let key: Option<String> = sqlx::query_scalar(
            "SELECT blob_key FROM reactor.deployments WHERE project_id = $1 AND name = $2 ORDER BY version DESC LIMIT 1",
        )
        .bind(project_id)
        .bind(name)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?;
        if let Some(key) = key {
            if let Some((bytes, _)) = state.blobs.get(&key).await.map_err(internal)? {
                publish_lambda(state, pref, name, &bytes).await?;
            }
        }
    }
    Ok(())
}

pub(crate) fn lambda_env(pref: &str, function_env: &[(String, String)]) -> Vec<(String, String)> {
    let mut owned = vec![("REACTOR_PROJECT_REF".to_string(), pref.to_string())];
    for (key, value) in function_env {
        if console::reserved_env_key(key) {
            continue;
        }
        owned.push((key.clone(), value.clone()));
    }
    let refs: Vec<(&str, &str)> = owned
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    scrub_env(&refs)
}

async fn publish_lambda(
    state: &AppState,
    pref: &str,
    name: &str,
    body: &[u8],
) -> Result<(), ApiError> {
    if state.config.functions_runtime != "lambda" {
        return Ok(());
    }
    let project_id: Uuid = sqlx::query_scalar("SELECT id FROM reactor.projects WHERE ref = $1")
        .bind(pref)
        .fetch_one(&state.pool)
        .await
        .map_err(internal)?;
    let function_env = console::load_function_env(state, project_id, name).await?;
    let env = lambda_env(pref, &function_env);
    let env_ref: Vec<(&str, &str)> = env
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    state
        .publisher
        .publish(&format!("{pref}-{name}"), body, &env_ref)
        .await
        .map_err(internal)?;
    Ok(())
}

pub(crate) async fn republish_function(
    state: &AppState,
    project_id: Uuid,
    pref: &str,
    name: &str,
) -> Result<(), ApiError> {
    if state.config.functions_runtime != "lambda" {
        return Ok(());
    }
    let key: Option<String> = sqlx::query_scalar(
        "SELECT d.blob_key FROM reactor.deployments d \
         WHERE d.project_id = $1 AND d.name = $2 AND d.version = COALESCE( \
           (SELECT p.version FROM reactor.function_pins p WHERE p.project_id = d.project_id AND p.name = d.name), \
           (SELECT MAX(m.version) FROM reactor.deployments m WHERE m.project_id = d.project_id AND m.name = d.name) \
         )",
    )
    .bind(project_id)
    .bind(name)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some(key) = key else {
        return Ok(());
    };
    let Some((bytes, _)) = state.blobs.get(&key).await.map_err(internal)? else {
        return Err(ApiError::not_found());
    };
    publish_lambda(state, pref, name, &bytes).await
}

pub(crate) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

#[derive(Deserialize)]
struct EnqueueBody {
    #[serde(default)]
    body: Value,
    #[serde(default)]
    delay_secs: i64,
    #[serde(default = "default_attempts")]
    max_attempts: i32,
}

fn default_attempts() -> i32 {
    3
}

async fn enqueue_function(
    State(ctx): State<AppCtx>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Json(body): Json<EnqueueBody>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    if !valid_name(&name) || name.starts_with('_') {
        return Err(ApiError::not_found());
    }
    if body.delay_secs < 0 || body.max_attempts < 1 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "delay_secs and max_attempts must be positive",
        ));
    }
    let resolved = resolve_project(&ctx.app, &headers, true).await?;
    require_service(&resolved)?;
    let run_at = chrono::Utc::now() + chrono::Duration::seconds(body.delay_secs)
        - chrono::Duration::seconds(if body.delay_secs == 0 { 1 } else { 0 });
    let id = ctx
        .tasks
        .enqueue(
            "fn.invoke",
            Some(resolved.project_id),
            json!({"name": name, "body": body.body}),
            run_at,
            body.max_attempts,
            None,
        )
        .await
        .map_err(|err| match err {
            reactor_core::EnqueueError::Conflict => {
                ApiError::new(StatusCode::CONFLICT, "duplicate task")
            }
            reactor_core::EnqueueError::Db(err) => internal(err),
        })?;
    Ok((StatusCode::CREATED, Json(json!({"id": id}))))
}

async fn task_status(
    State(ctx): State<AppCtx>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let resolved = resolve_project(&ctx.app, &headers, true).await?;
    require_service(&resolved)?;
    let Some(task) = ctx.tasks.get(id).await.map_err(internal)? else {
        return Err(ApiError::not_found());
    };
    if task.project_id != Some(resolved.project_id) {
        return Err(ApiError::not_found());
    }
    Ok(Json(json!({
        "id": task.id,
        "kind": task.kind,
        "status": task.status,
        "attempts": task.attempts,
        "max_attempts": task.max_attempts,
        "last_error": task.last_error,
    })))
}

async fn invoke_function(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    body: axum::body::Bytes,
) -> Result<Response, ApiError> {
    if name.starts_with('_') {
        return Err(ApiError::not_found());
    }
    let resolved = resolve_project(&state, &headers, true).await?;
    let caller = caller_json(&resolved);
    let bytes = run_function(&state, &resolved, &name, &caller, &body, None).await?;
    Ok(([(header::CONTENT_TYPE, "application/json")], bytes).into_response())
}

fn caller_json(resolved: &Resolved) -> Value {
    let identity = resolved.identity.as_ref();
    json!({
        "sub": identity.and_then(|i| i.user_id.map(|id| id.to_string())).unwrap_or_else(|| {
            identity.map(|i| i.role.as_str().to_string()).unwrap_or_else(|| "anon".into())
        }),
        "ref": resolved.pref,
        "role": identity.map(|i| i.role.as_str()).unwrap_or("anon"),
    })
}

pub(crate) async fn run_function(
    state: &AppState,
    resolved: &Resolved,
    name: &str,
    caller: &Value,
    body: &[u8],
    version: Option<i32>,
) -> Result<Vec<u8>, ApiError> {
    let row: Option<(String, i32)> = match version {
        Some(version) => {
            sqlx::query_as(
                "SELECT blob_key, version FROM reactor.deployments WHERE project_id = $1 AND name = $2 AND version = $3",
            )
            .bind(resolved.project_id)
            .bind(name)
            .bind(version)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?
        }
        None => {
            sqlx::query_as(
                "SELECT d.blob_key, d.version FROM reactor.deployments d \
                 WHERE d.project_id = $1 AND d.name = $2 AND d.version = COALESCE( \
                   (SELECT p.version FROM reactor.function_pins p WHERE p.project_id = d.project_id AND p.name = d.name), \
                   (SELECT MAX(m.version) FROM reactor.deployments m WHERE m.project_id = d.project_id AND m.name = d.name) \
                 )",
            )
            .bind(resolved.project_id)
            .bind(name)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?
        }
    };
    let Some((blob_key, version)) = row else {
        return Err(ApiError::not_found());
    };
    let extra = console::load_function_env(state, resolved.project_id, name).await?;
    if state.config.functions_runtime == "lambda" {
        let event = json!({
            "caller": caller,
            "body": String::from_utf8_lossy(body),
        });
        return match state
            .publisher
            .invoke(
                &format!("{}-{name}", resolved.pref),
                event.to_string().as_bytes(),
            )
            .await
        {
            Ok(bytes) => {
                log_function(state, resolved.project_id, name, 200, "").await;
                Ok(bytes)
            }
            Err(err) => {
                log_function(state, resolved.project_id, name, 500, &err.to_string()).await;
                Err(internal(err))
            }
        };
    }
    let dest = PathBuf::from(&state.config.functions_workdir)
        .join(&resolved.pref)
        .join(name)
        .join(version.to_string());
    state.bun.ensure(&blob_key, &dest).await.map_err(internal)?;
    let pairs: Vec<(&str, &str)> = extra
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    match state.bun.invoke(&dest, caller, body, &pairs).await {
        Ok(bytes) => {
            log_function(state, resolved.project_id, name, 200, "").await;
            Ok(bytes)
        }
        Err(err) => {
            log_function(state, resolved.project_id, name, 500, &err.to_string()).await;
            Err(internal(err))
        }
    }
}

async fn log_function(state: &AppState, project_id: Uuid, name: &str, status: i32, message: &str) {
    console::record_log(&state.pool, project_id, "function", name, status, message).await;
}

#[derive(Deserialize)]
struct ScheduleBody {
    function_name: String,
    #[serde(default)]
    body: String,
}

async fn create_schedule(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ScheduleBody>,
) -> Result<StatusCode, ApiError> {
    let resolved = resolve_project(&state, &headers, true).await?;
    require_service(&resolved)?;
    let payload = if body.body.is_empty() {
        "{}".to_string()
    } else {
        body.body
    };
    sqlx::query(
        "INSERT INTO reactor.schedules (id, project_id, function_name, body, next_run) VALUES ($1, $2, $3, $4, now())",
    )
    .bind(Uuid::new_v4())
    .bind(resolved.project_id)
    .bind(body.function_name)
    .bind(payload)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(StatusCode::CREATED)
}

async fn cron(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    require_operator(&state, &headers)?;
    Ok(Json(run_cron(&state).await?))
}

pub(crate) async fn run_cron(state: &AppState) -> Result<Value, ApiError> {
    let mut tx = state.pool.begin().await.map_err(internal)?;
    let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_xact_lock($1)")
        .bind(4_815_162_342_i64)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
    if !locked {
        return Ok(json!({"ran": false}));
    }
    let due: Vec<(Uuid, Uuid, String, String)> = sqlx::query_as(
        "SELECT id, project_id, function_name, body FROM reactor.schedules WHERE next_run <= now()",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(internal)?;
    let mut ran = 0;
    for (id, project_id, name, body) in due {
        let pref: String = sqlx::query_scalar("SELECT ref FROM reactor.projects WHERE id = $1")
            .bind(project_id)
            .fetch_one(&state.pool)
            .await
            .map_err(internal)?;
        let resolved = Resolved {
            project_id,
            pref: pref.clone(),
            database_url: None,
            identity: Some(Identity {
                project_id,
                project_ref: ProjectRef::parse(&pref).map_err(internal)?,
                user_id: None,
                role: Role::Service,
                claims: json!({"sub": "cron", "role": "service"}),
            }),
        };
        let caller = json!({"sub": "cron", "ref": pref, "role": "service"});
        run_function(&state, &resolved, &name, &caller, body.as_bytes(), None).await?;
        sqlx::query(
            "UPDATE reactor.schedules SET next_run = now() + interval '1 day' WHERE id = $1",
        )
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
        ran += 1;
    }
    tx.commit().await.map_err(internal)?;
    Ok(json!({"ran": true, "count": ran}))
}

async fn upload_site(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(path): Path<String>,
    body: axum::body::Bytes,
) -> Result<StatusCode, ApiError> {
    let resolved = resolve_project(&state, &headers, true).await?;
    require_service(&resolved)?;
    let path = safe_site_path(&path).ok_or_else(|| ApiError::forbidden("invalid path"))?;
    let key = object_key(&resolved.pref, "_sites", &path)
        .map_err(|_| ApiError::forbidden("invalid path"))?;
    let content_type = content_type_for(&path);
    state
        .blobs
        .put(&key, body, content_type)
        .await
        .map_err(internal)?;
    sqlx::query(
        "INSERT INTO reactor.site_files (project_id, path, blob_key, content_type) VALUES ($1, $2, $3, $4) \
         ON CONFLICT (project_id, path) DO UPDATE SET blob_key = EXCLUDED.blob_key, content_type = EXCLUDED.content_type",
    )
    .bind(resolved.project_id)
    .bind(&path)
    .bind(&key)
    .bind(content_type)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(StatusCode::CREATED)
}

async fn start_deployment(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let resolved = resolve_project(&state, &headers, true).await?;
    require_service(&resolved)?;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO reactor.site_deployments (id, project_id, status) VALUES ($1, $2, 'building')",
    )
    .bind(id)
    .bind(resolved.project_id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(json!({ "id": id })))
}

async fn upload_deployment_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, path)): Path<(Uuid, String)>,
    body: axum::body::Bytes,
) -> Result<StatusCode, ApiError> {
    let resolved = resolve_project(&state, &headers, true).await?;
    require_service(&resolved)?;
    let path = safe_site_path(&path).ok_or_else(|| ApiError::forbidden("invalid path"))?;
    require_building(&state, id, resolved.project_id).await?;
    let key = deployment_blob_key(&resolved.pref, id, &path)?;
    state
        .blobs
        .put(&key, body, content_type_for(&path))
        .await
        .map_err(internal)?;
    record_deployment_file(&state, id, &path, &key).await?;
    Ok(StatusCode::CREATED)
}

async fn post_deployment_file(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((id, path)): Path<(Uuid, String)>,
) -> Result<Response, ApiError> {
    let resolved = resolve_project(&state, &headers, true).await?;
    require_service(&resolved)?;
    let (path, confirm) =
        site_upload_target(&path).ok_or_else(|| ApiError::forbidden("invalid path"))?;
    require_building(&state, id, resolved.project_id).await?;
    let key = deployment_blob_key(&resolved.pref, id, &path)?;
    if confirm {
        if state.blobs.head(&key).await.map_err(internal)?.is_none() {
            return Err(ApiError::not_found());
        }
        record_deployment_file(&state, id, &path, &key).await?;
        return Ok(StatusCode::CREATED.into_response());
    }
    let url = state.blobs.presign_put(&key, 300).await.map_err(internal)?;
    let fallback = format!(
        "{}/sites/v1/deployments/{id}/files/{path}",
        state.config.public_url.trim_end_matches('/')
    );
    Ok(Json(json!({ "url": url, "fallback": fallback })).into_response())
}

fn deployment_blob_key(pref: &str, id: Uuid, path: &str) -> Result<String, ApiError> {
    object_key(pref, "_sites", &format!("{id}/{path}"))
        .map_err(|_| ApiError::forbidden("invalid path"))
}

async fn require_building(state: &AppState, id: Uuid, project_id: Uuid) -> Result<(), ApiError> {
    let building: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM reactor.site_deployments WHERE id = $1 AND project_id = $2 AND status = 'building'",
    )
    .bind(id)
    .bind(project_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    if building.is_none() {
        return Err(ApiError::not_found());
    }
    Ok(())
}

async fn record_deployment_file(
    state: &AppState,
    id: Uuid,
    path: &str,
    key: &str,
) -> Result<(), ApiError> {
    let content_type = content_type_for(path);
    sqlx::query(
        "INSERT INTO reactor.site_deployment_files (deployment_id, path, blob_key, content_type) VALUES ($1, $2, $3, $4) \
         ON CONFLICT (deployment_id, path) DO UPDATE SET blob_key = EXCLUDED.blob_key, content_type = EXCLUDED.content_type",
    )
    .bind(id)
    .bind(path)
    .bind(key)
    .bind(content_type)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(())
}

#[derive(Deserialize, Default)]
struct FinishBody {
    #[serde(default)]
    command: String,
}

async fn finish_deployment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    body: axum::body::Bytes,
) -> Result<Json<Value>, ApiError> {
    let resolved = resolve_project(&state, &headers, true).await?;
    require_service(&resolved)?;
    let files: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT f.path, f.blob_key, f.content_type FROM reactor.site_deployment_files f \
         JOIN reactor.site_deployments d ON d.id = f.deployment_id \
         WHERE d.id = $1 AND d.project_id = $2 AND d.status = 'building'",
    )
    .bind(id)
    .bind(resolved.project_id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let exists: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM reactor.site_deployments WHERE id = $1 AND project_id = $2 AND status = 'building'",
    )
    .bind(id)
    .bind(resolved.project_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    if exists.is_none() {
        return Err(ApiError::not_found());
    }
    let command = if body.is_empty() {
        String::new()
    } else {
        serde_json::from_slice::<FinishBody>(&body)
            .map(|body| body.command)
            .unwrap_or_default()
    };
    if !command.is_empty() && !safe_site_command(&command) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid site command",
        ));
    }
    let paths: Vec<String> = files.iter().map(|(path, _, _)| path.clone()).collect();
    let mut tx = state.pool.begin().await.map_err(internal)?;
    sqlx::query("DELETE FROM reactor.site_files WHERE project_id = $1 AND NOT (path = ANY($2))")
        .bind(resolved.project_id)
        .bind(&paths)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    for (path, blob_key, content_type) in &files {
        sqlx::query(
            "INSERT INTO reactor.site_files (project_id, path, blob_key, content_type) VALUES ($1, $2, $3, $4) \
             ON CONFLICT (project_id, path) DO UPDATE SET blob_key = EXCLUDED.blob_key, content_type = EXCLUDED.content_type",
        )
        .bind(resolved.project_id)
        .bind(path)
        .bind(blob_key)
        .bind(content_type)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    }
    sqlx::query(
        "UPDATE reactor.site_deployments SET status = 'ready', file_count = $3, error = '', command = $4 WHERE id = $1 AND project_id = $2",
    )
    .bind(id)
    .bind(resolved.project_id)
    .bind(files.len() as i32)
    .bind(&command)
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    state.sites.stop(resolved.project_id).await;
    Ok(Json(
        json!({ "id": id, "status": "ready", "file_count": files.len(), "command": command }),
    ))
}

#[derive(Deserialize)]
struct FailBody {
    error: String,
}

async fn fail_deployment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<FailBody>,
) -> Result<StatusCode, ApiError> {
    let resolved = resolve_project(&state, &headers, true).await?;
    require_service(&resolved)?;
    let error: String = body.error.chars().take(500).collect();
    let updated = sqlx::query(
        "UPDATE reactor.site_deployments SET status = 'error', error = $3 WHERE id = $1 AND project_id = $2 AND status = 'building'",
    )
    .bind(id)
    .bind(resolved.project_id)
    .bind(&error)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct SiteRouteBody {
    path: String,
    function: String,
}

async fn site_route(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<SiteRouteBody>,
) -> Result<StatusCode, ApiError> {
    let resolved = resolve_project(&state, &headers, true).await?;
    require_service(&resolved)?;
    let path = safe_site_path(&body.path).ok_or_else(|| ApiError::forbidden("invalid path"))?;
    sqlx::query(
        "INSERT INTO reactor.site_routes (project_id, path, function_name) VALUES ($1, $2, $3) \
         ON CONFLICT (project_id, path) DO UPDATE SET function_name = EXCLUDED.function_name",
    )
    .bind(resolved.project_id)
    .bind(path)
    .bind(body.function)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(StatusCode::CREATED)
}

#[derive(Deserialize)]
struct DomainBody {
    host: String,
}

async fn create_domain(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<DomainBody>,
) -> Result<Json<Value>, ApiError> {
    let resolved = resolve_project(&state, &headers, true).await?;
    require_service(&resolved)?;
    let token = random_token();
    sqlx::query("INSERT INTO reactor.domains (host, project_id, token) VALUES ($1, $2, $3)")
        .bind(&body.host)
        .bind(resolved.project_id)
        .bind(&token)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(Json(json!({
        "host": body.host,
        "token": token,
        "txt_name": verification_name(&body.host),
        "txt_value": expected_txt(&token),
    })))
}

async fn verify_domain(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(host): Path<String>,
) -> Result<StatusCode, ApiError> {
    let resolved = resolve_project(&state, &headers, true).await?;
    require_service(&resolved)?;
    let token: Option<String> =
        sqlx::query_scalar("SELECT token FROM reactor.domains WHERE host = $1 AND project_id = $2")
            .bind(&host)
            .bind(resolved.project_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    let Some(token) = token else {
        return Err(ApiError::not_found());
    };
    let records = lookup_txt(
        &state.http,
        &state.config.dns_stub_file,
        &verification_name(&host),
    )
    .await;
    if !txt_matches(&records, &token) {
        return Err(ApiError::forbidden("verification failed"));
    }
    sqlx::query("UPDATE reactor.domains SET verified_at = now() WHERE host = $1")
        .bind(&host)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

const DOH_BASES: &[&str] = &[
    "https://cloudflare-dns.com/dns-query",
    "https://dns.google/resolve",
];

pub(crate) async fn lookup_txt(
    http: &reqwest::Client,
    stub: &Option<String>,
    name: &str,
) -> Vec<String> {
    lookup_txt_from(http, stub, name, DOH_BASES).await
}

async fn lookup_txt_from(
    http: &reqwest::Client,
    stub: &Option<String>,
    name: &str,
    bases: &[&str],
) -> Vec<String> {
    if stub.is_some() {
        return read_txt_stub(stub, name);
    }
    for base in bases {
        match fetch_doh_txt(http, base, name).await {
            Ok(records) if !records.is_empty() => return records,
            Ok(_) => {}
            Err(err) => {
                tracing::warn!("txt lookup failed for {name} via {base}: {err:#}");
            }
        }
    }
    Vec::new()
}

async fn fetch_doh_txt(
    http: &reqwest::Client,
    base: &str,
    name: &str,
) -> anyhow::Result<Vec<String>> {
    let response = http
        .get(base)
        .header(header::ACCEPT, "application/dns-json")
        .query(&[("name", name), ("type", "TXT")])
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await?
        .error_for_status()?;
    let body = response.text().await?;
    txt_from_doh(&body).ok_or_else(|| anyhow::anyhow!("dns response was not json"))
}

fn read_txt_stub(path: &Option<String>, name: &str) -> Vec<String> {
    let Some(path) = path else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(map) = serde_json::from_str::<serde_json::Map<String, Value>>(&text) else {
        return Vec::new();
    };
    map.get(name)
        .and_then(|v| v.as_str())
        .map(|s| vec![s.to_string()])
        .unwrap_or_default()
}

async fn delete_domain(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(host): Path<String>,
) -> Result<StatusCode, ApiError> {
    let resolved = resolve_project(&state, &headers, true).await?;
    require_service(&resolved)?;
    sqlx::query("DELETE FROM reactor.domains WHERE host = $1 AND project_id = $2")
        .bind(&host)
        .bind(resolved.project_id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct AskQuery {
    domain: String,
}

async fn confirm_domain(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(host): Path<String>,
) -> Result<StatusCode, ApiError> {
    require_operator(&state, &headers)?;
    let updated = sqlx::query("UPDATE reactor.domains SET verified_at = now() WHERE host = $1")
        .bind(&host)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn ask(
    State(state): State<AppState>,
    Query(q): Query<AskQuery>,
) -> Result<StatusCode, ApiError> {
    let found: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM reactor.domains WHERE host = $1 AND verified_at IS NOT NULL",
    )
    .bind(&q.domain)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    if found == 0 {
        return Err(ApiError::not_found());
    }
    Ok(StatusCode::OK)
}

async fn site_fallback(State(state): State<AppState>, req: Request) -> Result<Response, ApiError> {
    let headers = req.headers().clone();
    let host = host_of(&headers);
    let host_only = host.split(':').next().unwrap_or("").to_string();
    let project = if let Some(pref) = project_ref_from_host(&host, &state.config.base_domain) {
        load_project(&state, pref.as_str()).await?
    } else {
        let row: Option<(Uuid, String)> = sqlx::query_as(
            "SELECT p.id, p.ref FROM reactor.domains d JOIN reactor.projects p ON p.id = d.project_id \
             WHERE d.host = $1 AND d.verified_at IS NOT NULL",
        )
        .bind(&host_only)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?;
        let Some((id, pref)) = row else {
            return Err(ApiError::not_found());
        };
        Resolved {
            project_id: id,
            pref,
            database_url: None,
            identity: None,
        }
    };
    let path =
        safe_site_path(req.uri().path()).ok_or_else(|| ApiError::forbidden("invalid path"))?;
    let command: String = sqlx::query_scalar(
        "SELECT command FROM reactor.site_deployments WHERE project_id = $1 AND status = 'ready' ORDER BY created_at DESC LIMIT 1",
    )
    .bind(project.project_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    .unwrap_or_default();
    let mut candidates = vec![path.clone()];
    if path.ends_with('/') {
        candidates.push(format!("{path}index.html"));
    } else if !path.contains('.') {
        candidates.push(format!("{path}/index.html"));
    }
    let mut file: Option<(String, String)> = None;
    for candidate in &candidates {
        file = sqlx::query_as(
            "SELECT blob_key, content_type FROM reactor.site_files WHERE project_id = $1 AND path = $2",
        )
        .bind(project.project_id)
        .bind(candidate)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?;
        if file.is_some() {
            break;
        }
    }
    if let Some((blob_key, content_type)) = file {
        let Some((bytes, _)) = state.blobs.get(&blob_key).await.map_err(internal)? else {
            console::record_log(
                &state.pool,
                project.project_id,
                "site",
                &path,
                404,
                "missing blob",
            )
            .await;
            return Err(ApiError::not_found());
        };
        console::record_log(&state.pool, project.project_id, "site", &path, 200, "").await;
        return Ok(([(header::CONTENT_TYPE, content_type)], bytes).into_response());
    }
    if !command.is_empty() {
        return proxy_site(&state, &project, &command, req).await;
    }
    let function: Option<String> = sqlx::query_scalar(
        "SELECT function_name FROM reactor.site_routes WHERE project_id = $1 AND path = $2",
    )
    .bind(project.project_id)
    .bind(&path)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    if let Some(name) = function {
        let caller = json!({"sub": "site", "ref": project.pref, "role": "anon"});
        let bytes = run_function(&state, &project, &name, &caller, b"", None).await?;
        return Ok(([(header::CONTENT_TYPE, "application/json")], bytes).into_response());
    }
    console::record_log(&state.pool, project.project_id, "site", &path, 404, "").await;
    Err(ApiError::not_found())
}

async fn proxy_site(
    state: &AppState,
    project: &Resolved,
    command: &str,
    req: Request,
) -> Result<Response, ApiError> {
    let dir = state.sites.project_dir(project.project_id);
    let env = console::load_site_env(state, project.project_id).await?;
    let port = state
        .sites
        .ensure(project.project_id, command, &dir, &env, &state.pool, || {
            let state = state.clone();
            let dir = dir.clone();
            let project_id = project.project_id;
            async move {
                materialize_site(&state, project_id, &dir)
                    .await
                    .map_err(|err| anyhow::anyhow!(err.text().to_string()))
            }
        })
        .await
        .map_err(internal)?;
    let (parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, 8 * 1024 * 1024)
        .await
        .map_err(internal)?;
    let url = format!("http://127.0.0.1:{port}{}", parts.uri);
    let method = reqwest::Method::from_bytes(parts.method.as_str().as_bytes()).map_err(internal)?;
    let mut outbound = state.http.request(method, url);
    for (name, value) in parts.headers.iter() {
        if name == header::HOST || name == header::CONNECTION || name == header::CONTENT_LENGTH {
            continue;
        }
        outbound = outbound.header(name, value);
    }
    let response = outbound
        .body(bytes.to_vec())
        .send()
        .await
        .map_err(internal)?;
    let status =
        StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut headers = HeaderMap::new();
    for (name, value) in response.headers() {
        if name == reqwest::header::CONNECTION || name == reqwest::header::TRANSFER_ENCODING {
            continue;
        }
        if let (Ok(name), Ok(value)) = (
            axum::http::HeaderName::from_bytes(name.as_str().as_bytes()),
            axum::http::HeaderValue::from_bytes(value.as_bytes()),
        ) {
            headers.insert(name, value);
        }
    }
    let body = response.bytes().await.map_err(internal)?;
    console::record_log(
        &state.pool,
        project.project_id,
        "site",
        "server",
        status.as_u16() as i32,
        "",
    )
    .await;
    Ok((status, headers, body).into_response())
}

async fn materialize_site(
    state: &AppState,
    project_id: Uuid,
    dir: &std::path::Path,
) -> Result<(), ApiError> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT path, blob_key FROM reactor.site_files WHERE project_id = $1")
            .bind(project_id)
            .fetch_all(&state.pool)
            .await
            .map_err(internal)?;
    if dir.exists() {
        tokio::fs::remove_dir_all(dir).await.ok();
    }
    tokio::fs::create_dir_all(dir).await.map_err(internal)?;
    for (path, blob_key) in rows {
        let Some((bytes, _)) = state.blobs.get(&blob_key).await.map_err(internal)? else {
            continue;
        };
        let dest = dir.join(&path);
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(internal)?;
        }
        tokio::fs::write(dest, &bytes).await.map_err(internal)?;
    }
    Ok(())
}

async fn get_site_env(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let resolved = resolve_project(&state, &headers, true).await?;
    require_service(&resolved)?;
    let env = console::load_site_env(&state, resolved.project_id).await?;
    let items: Vec<Value> = env
        .into_iter()
        .map(|(key, value)| json!({"key": key, "value": value}))
        .collect();
    Ok(Json(json!(items)))
}

async fn put_site_env(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<console::SiteEnvWrite>,
) -> Result<StatusCode, ApiError> {
    let resolved = resolve_project(&state, &headers, true).await?;
    require_service(&resolved)?;
    console::write_site_env(
        &state,
        resolved.project_id,
        &body.key,
        &body.value,
        &body.visibility,
    )
    .await?;
    state.sites.stop(resolved.project_id).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn load_project(state: &AppState, pref: &str) -> Result<Resolved, ApiError> {
    let row: Option<(Uuid, Option<String>)> =
        sqlx::query_as("SELECT id, database_url FROM reactor.projects WHERE ref = $1")
            .bind(pref)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    let Some((id, database_url)) = row else {
        return Err(ApiError::not_found());
    };
    Ok(Resolved {
        project_id: id,
        pref: pref.to_string(),
        database_url,
        identity: None,
    })
}

struct ServerInvoker {
    state: AppState,
}

#[async_trait]
impl Invoker for ServerInvoker {
    async fn invoke(
        &self,
        project_id: Uuid,
        name: String,
        body: Vec<u8>,
    ) -> Result<Vec<u8>, String> {
        let (pref, database_url): (String, Option<String>) =
            sqlx::query_as("SELECT ref, database_url FROM reactor.projects WHERE id = $1")
                .bind(project_id)
                .fetch_one(&self.state.pool)
                .await
                .map_err(|err| err.to_string())?;
        let resolved = Resolved {
            project_id,
            pref: pref.clone(),
            database_url,
            identity: Some(Identity {
                project_id,
                project_ref: ProjectRef::parse(&pref).map_err(|err| err.to_string())?,
                user_id: None,
                role: Role::Service,
                claims: json!({"sub": "service", "role": "service"}),
            }),
        };
        let caller = json!({"sub": "service", "ref": pref, "role": "service"});
        run_function(&self.state, &resolved, &name, &caller, &body, None)
            .await
            .map_err(|err| err.text().to_string())
    }
}

struct ServerLog {
    pool: sqlx::PgPool,
}

#[async_trait]
impl reactor_core::ProjectLog for ServerLog {
    async fn record(
        &self,
        project_id: Uuid,
        kind: String,
        name: String,
        status: i32,
        message: String,
    ) {
        console::record_log(&self.pool, project_id, &kind, &name, status, &message).await;
    }
}

struct ServerProjects {
    state: AppState,
}

#[async_trait]
impl Projects for ServerProjects {
    async fn open_service(
        &self,
        authorization: Option<String>,
        host: Option<String>,
    ) -> Result<OpenedProject, ProjectError> {
        let mut headers = HeaderMap::new();
        if let Some(value) = authorization {
            let Ok(parsed) = value.parse() else {
                return Err(ProjectError::Unauthorized);
            };
            headers.insert(header::AUTHORIZATION, parsed);
        }
        if let Some(value) = host {
            let Ok(parsed) = value.parse() else {
                return Err(ProjectError::Unauthorized);
            };
            headers.insert(header::HOST, parsed);
        }
        let resolved = match resolve_project(&self.state, &headers, true).await {
            Ok(resolved) => resolved,
            Err(err) if err.status == StatusCode::FORBIDDEN => return Err(ProjectError::Forbidden),
            Err(_) => return Err(ProjectError::Unauthorized),
        };
        if require_service(&resolved).is_err() {
            return Err(ProjectError::Forbidden);
        }
        let pool = pool_for(&self.state, resolved.database_url.as_deref())
            .await
            .map_err(|err| ProjectError::Unavailable(err.to_string()))?;
        Ok(OpenedProject {
            id: resolved.project_id,
            pref: resolved.pref.clone(),
            schema: format!("proj_{}", resolved.pref),
            pool,
        })
    }

    async fn list(&self) -> Result<Vec<OpenedProject>, ProjectError> {
        let rows: Vec<(Uuid, String, Option<String>)> =
            sqlx::query_as("SELECT id, ref, database_url FROM reactor.projects ORDER BY ref")
                .fetch_all(&self.state.pool)
                .await
                .map_err(|err| ProjectError::Unavailable(err.to_string()))?;
        let mut out = Vec::new();
        for (id, pref, url) in rows {
            let pool = pool_for(&self.state, url.as_deref())
                .await
                .map_err(|err| ProjectError::Unavailable(err.to_string()))?;
            out.push(OpenedProject {
                id,
                pref: pref.clone(),
                schema: format!("proj_{pref}"),
                pool,
            });
        }
        Ok(out)
    }
}

pub async fn router_from_env() -> anyhow::Result<Router> {
    build_app(Config::load()).await
}

#[cfg(test)]
mod tests {
    use super::{lambda_env, lookup_txt_from, pool_max_connections};
    use reactor_sites::txt_matches;

    fn local_doh(status: u16, body: &'static str) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let mut stream = listener.incoming().next().unwrap().unwrap();
            let mut buf = [0u8; 4096];
            let _ = std::io::Read::read(&mut stream, &mut buf);
            let response = format!(
                "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
        });
        format!("http://{addr}/dns-query")
    }

    #[tokio::test]
    async fn stub_wins_and_a_miss_does_not_query_dns() {
        let dir = std::env::temp_dir().join(format!("reactor-dns-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("stub.json");
        let name = "_reactor-verify.app.example.com";
        std::fs::write(
            &path,
            format!(r#"{{"{name}":"reactor-site-verification=from-stub"}}"#),
        )
        .unwrap();
        let http = reqwest::Client::new();
        let live = local_doh(
            200,
            r#"{"Answer":[{"type":16,"data":"\"reactor-site-verification=from-dns\""}]}"#,
        );
        let stub = Some(path.display().to_string());
        let records = lookup_txt_from(&http, &stub, name, &[live.as_str()]).await;
        assert_eq!(
            records,
            vec!["reactor-site-verification=from-stub".to_string()]
        );
        let missing = lookup_txt_from(
            &http,
            &stub,
            "_reactor-verify.other.example",
            &[live.as_str()],
        )
        .await;
        assert!(missing.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn unset_stub_reads_public_txt_and_falls_back() {
        let fail = local_doh(500, "nope");
        let ok = local_doh(
            200,
            r#"{"Answer":[{"type":16,"data":"\"reactor-site-verification=from-dns\""}]}"#,
        );
        let http = reqwest::Client::new();
        let records = lookup_txt_from(
            &http,
            &None,
            "_reactor-verify.app.example.com",
            &[fail.as_str(), ok.as_str()],
        )
        .await;
        assert!(txt_matches(&records, "from-dns"));
        let empty = local_doh(200, r#"{"Status":3}"#);
        let none = lookup_txt_from(
            &http,
            &None,
            "_reactor-verify.missing.example",
            &[empty.as_str()],
        )
        .await;
        assert!(none.is_empty());
    }

    #[test]
    fn lambda_pool_leaves_room_for_the_agent_lock() {
        assert_eq!(pool_max_connections("lambda"), 2);
        assert_eq!(pool_max_connections("listen"), 8);
    }

    #[test]
    fn lambda_env_keeps_user_secrets_and_drops_reserved_names() {
        let env = lambda_env(
            "pref",
            &[
                ("TOKEN".into(), "alpha".into()),
                ("API_SECRET".into(), "sekret".into()),
                ("REACTOR_CALLER".into(), "hijack".into()),
                ("PATH".into(), "/bin".into()),
            ],
        );
        assert!(env
            .iter()
            .any(|(key, value)| key == "TOKEN" && value == "alpha"));
        assert!(env
            .iter()
            .any(|(key, value)| key == "API_SECRET" && value == "sekret"));
        assert!(env
            .iter()
            .any(|(key, value)| key == "REACTOR_PROJECT_REF" && value == "pref"));
        assert!(env
            .iter()
            .all(|(key, _)| key != "DATABASE_URL" && key != "REACTOR_CALLER" && key != "PATH"));
    }
}
