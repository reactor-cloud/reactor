use crate::{bearer, generate_ref, internal, pool_for, reload, store_api_key, ApiError, AppState};
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::{Json, Router};
use reactor_auth::{hash_password, random_token, token_hash, verify_password};
use reactor_identity::ProjectRef;
use reactor_sites::{expected_txt, txt_matches, verification_name};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use uuid::Uuid;

pub fn mount(app: Router<AppState>) -> Router<AppState> {
    let app = crate::agent::mount(app);
    let app = crate::mfa::mount(app);
    let app = crate::email::mount(app);
    let app = crate::project_auth::mount_console(app);
    app.route("/console/v1/setup", get(setup_status).post(setup))
        .route("/console/v1/setup/restart", post(restart_setup))
        .route("/console/v1/login", post(login))
        .route("/console/v1/me", get(me))
        .route("/console/v1/keys", get(list_keys).post(create_key))
        .route("/console/v1/keys/{id}", delete(delete_key))
        .route("/console/v1/cluster", get(cluster).post(rename_cluster))
        .route(
            "/console/v1/operators",
            get(list_operators).post(create_operator),
        )
        .route(
            "/console/v1/operators/{operator_id}",
            post(update_operator).delete(delete_operator),
        )
        .route(
            "/console/v1/operators/{operator_id}/projects/{pref}",
            post(grant_operator).delete(revoke_operator),
        )
        .route(
            "/console/v1/projects",
            get(list_projects).post(create_project),
        )
        .route(
            "/console/v1/projects/{pref}",
            post(rename_project).delete(delete_project),
        )
        .route("/console/v1/projects/{pref}/keys", post(rotate_keys))
        .route(
            "/console/v1/projects/{pref}/members",
            get(list_members).post(add_member),
        )
        .route("/console/v1/projects/{pref}/users", get(list_users))
        .route(
            "/console/v1/projects/{pref}/users/{user_id}",
            get(get_user).delete(delete_user),
        )
        .route(
            "/console/v1/projects/{pref}/users/{user_id}/password",
            post(set_user_password),
        )
        .route("/console/v1/projects/{pref}/schemas", get(schemas))
        .route("/console/v1/projects/{pref}/schema", get(schema))
        .route("/console/v1/projects/{pref}/tables", post(create_table))
        .route(
            "/console/v1/projects/{pref}/migrations",
            post(apply_migration),
        )
        .route(
            "/console/v1/projects/{pref}/tables/{table}",
            get(table_rows).post(insert_row),
        )
        .route(
            "/console/v1/projects/{pref}/tables/{table}/rows",
            post(update_row).delete(delete_row),
        )
        .route(
            "/console/v1/projects/{pref}/objects",
            get(objects).delete(delete_object),
        )
        .route("/console/v1/projects/{pref}/objects/url", post(object_url))
        .route(
            "/console/v1/projects/{pref}/objects/upload",
            post(upload_url),
        )
        .route("/console/v1/projects/{pref}/functions", get(functions))
        .route(
            "/console/v1/projects/{pref}/functions/{name}",
            post(deploy_console_function),
        )
        .route(
            "/console/v1/projects/{pref}/functions/{name}/promote",
            post(promote_function),
        )
        .route(
            "/console/v1/projects/{pref}/functions/{name}/demote",
            post(demote_function),
        )
        .route(
            "/console/v1/projects/{pref}/functions/{name}/invoke",
            post(invoke_console_function),
        )
        .route(
            "/console/v1/projects/{pref}/functions/{name}/stats",
            get(function_stats),
        )
        .route(
            "/console/v1/projects/{pref}/functions/{name}/env",
            get(list_function_env).post(set_function_env),
        )
        .route(
            "/console/v1/projects/{pref}/functions/{name}/env/{key}",
            delete(delete_function_env),
        )
        .route("/console/v1/projects/{pref}/sites", get(sites))
        .route("/console/v1/projects/{pref}/site/stats", get(site_stats))
        .route(
            "/console/v1/projects/{pref}/site/deployments",
            get(site_deployments),
        )
        .route(
            "/console/v1/projects/{pref}/site/deployments/{id}",
            get(site_deployment),
        )
        .route(
            "/console/v1/projects/{pref}/site/env",
            get(list_site_env).post(set_site_env),
        )
        .route(
            "/console/v1/projects/{pref}/site/env/{key}",
            delete(delete_site_env),
        )
        .route(
            "/console/v1/projects/{pref}/site/domains",
            get(site_domains).post(add_site_domain),
        )
        .route(
            "/console/v1/projects/{pref}/site/domains/{host}/verify",
            post(verify_site_domain),
        )
        .route(
            "/console/v1/projects/{pref}/site/domains/{host}",
            delete(remove_site_domain),
        )
        .route("/console/v1/projects/{pref}/logs", get(logs))
        .route("/console/v1/projects/{pref}/overview", get(overview))
        .route("/console", get(spa_index))
        .route("/console/", get(spa_index))
        .route("/console/{*path}", get(spa_file))
}

pub async fn record_log(
    pool: &sqlx::PgPool,
    project_id: Uuid,
    kind: &str,
    name: &str,
    status: i32,
    message: &str,
) {
    let message: String = message.chars().take(500).collect();
    if let Err(err) = sqlx::query(
        "INSERT INTO reactor.logs (project_id, kind, name, status, message) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(project_id)
    .bind(kind)
    .bind(name)
    .bind(status)
    .bind(message)
    .execute(pool)
    .await
    {
        tracing::error!("log write failed: {err}");
    }
}

pub(crate) fn console_key_scope(method: &str, path: &str) -> Option<&'static str> {
    let path = path.split('?').next().unwrap_or(path);
    if path.len() > 1 && path.ends_with('/') {
        return None;
    }
    if method.eq_ignore_ascii_case("POST") && path == "/console/v1/projects" {
        return Some("projects.create");
    }
    let rest = path.strip_prefix("/console/v1/projects/")?;
    let mut parts = rest.split('/');
    let pref = parts.next().unwrap_or("");
    if !project_ref_ok(pref) {
        return None;
    }
    let tail: Vec<&str> = parts.collect();
    let method = method.to_ascii_uppercase();
    match (method.as_str(), tail.as_slice()) {
        ("GET" | "PUT", ["auth"]) => Some("auth.settings"),
        ("GET" | "POST", ["auth", "providers"]) => Some("auth.providers"),
        ("DELETE", ["auth", "providers", name]) if segment(name) => Some("auth.providers"),
        ("GET" | "PUT", ["email"]) => Some("auth.email"),
        ("POST", ["email", "test"]) => Some("auth.email"),
        ("GET", ["email", "templates"]) => Some("auth.email"),
        ("GET" | "PUT", ["email", "templates", name]) if segment(name) => Some("auth.email"),
        ("POST", ["email", "templates", name, "reset"]) if segment(name) => Some("auth.email"),
        ("GET", ["users"]) => Some("auth.users"),
        ("GET" | "DELETE", ["users", id]) if segment(id) => Some("auth.users"),
        ("POST", ["users", id, "password"]) if segment(id) => Some("auth.users"),
        ("POST", ["migrations"]) => Some("projects.migrate"),
        _ => None,
    }
}

fn project_ref_ok(value: &str) -> bool {
    value.len() == 20
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

fn segment(value: &str) -> bool {
    !value.is_empty()
}

fn project_ref_in_console_path(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("/console/v1/projects/")?;
    let pref = rest.split('/').next().unwrap_or("");
    if project_ref_ok(pref) {
        Some(pref)
    } else {
        None
    }
}

pub(crate) async fn console_key_guard(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path().to_string();
    if !path.starts_with("/console/v1") {
        return next.run(request).await;
    }
    let Some(token) = bearer(request.headers()) else {
        return next.run(request).await;
    };
    let Ok(claims) = state.issuer.decode(&token) else {
        return next.run(request).await;
    };
    if claims.aud != "console-key" {
        return next.run(request).await;
    }
    let Ok(operator_id) = Uuid::parse_str(&claims.sub) else {
        return ApiError::unauthorized("console token required").into_response();
    };
    let row = sqlx::query_as::<_, (Uuid, Vec<String>)>(
        "SELECT id, scopes FROM reactor.console_keys WHERE token_hash = $1 AND operator_id = $2",
    )
    .bind(token_hash(&token))
    .bind(operator_id)
    .fetch_optional(&state.pool)
    .await;
    let row = match row {
        Ok(row) => row,
        Err(err) => return internal(err).into_response(),
    };
    let Some((key_id, scopes)) = row else {
        return ApiError::unauthorized("console token required").into_response();
    };
    let Some(scope) = console_key_scope(request.method().as_str(), &path) else {
        return ApiError::forbidden("console key cannot call this").into_response();
    };
    if !scopes.iter().any(|item| item == scope) {
        return ApiError::forbidden("console key cannot call this").into_response();
    }
    if let Some(pref) = project_ref_in_console_path(&path) {
        let stamp = sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT created_by_key FROM reactor.projects WHERE ref = $1",
        )
        .bind(pref)
        .fetch_optional(&state.pool)
        .await;
        let stamp = match stamp {
            Ok(stamp) => stamp,
            Err(err) => return internal(err).into_response(),
        };
        if stamp.flatten() != Some(key_id) {
            return ApiError::forbidden("console key cannot call this").into_response();
        }
    }
    next.run(request).await
}

const KEY_SCOPES: &[&str] = &[
    "projects.create",
    "projects.migrate",
    "auth.settings",
    "auth.providers",
    "auth.email",
    "auth.users",
];

fn clean_scopes(scopes: &[String]) -> Result<Vec<String>, ApiError> {
    if scopes.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "at least one scope is required",
        ));
    }
    let mut out = Vec::new();
    for scope in scopes {
        if !KEY_SCOPES.contains(&scope.as_str()) {
            return Err(ApiError::new(StatusCode::BAD_REQUEST, "unknown scope"));
        }
        if !out.iter().any(|item: &String| item == scope) {
            out.push(scope.clone());
        }
    }
    Ok(out)
}

pub(crate) struct Operator {
    pub(crate) id: Uuid,
    pub(crate) email: String,
    #[allow(dead_code)]
    pub(crate) name: String,
    pub(crate) platform_admin: bool,
    pub(crate) key_id: Option<Uuid>,
    #[allow(dead_code)]
    pub(crate) scopes: Vec<String>,
}

fn session_operator(id: Uuid, email: String, name: String, platform_admin: bool) -> Operator {
    Operator {
        id,
        email,
        name,
        platform_admin,
        key_id: None,
        scopes: Vec::new(),
    }
}

pub(crate) async fn require_operator(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Operator, ApiError> {
    let token = bearer(headers).ok_or_else(|| ApiError::unauthorized("console token required"))?;
    let claims = state
        .issuer
        .decode(&token)
        .map_err(|_| ApiError::unauthorized("console token required"))?;
    let id = Uuid::parse_str(&claims.sub)
        .map_err(|_| ApiError::unauthorized("console token required"))?;
    if claims.aud == "console-key" {
        let row: Option<(Uuid, Vec<String>, Uuid, String, String, bool)> = sqlx::query_as(
            "SELECT k.id, k.scopes, o.id, o.email, o.name, o.platform_admin \
             FROM reactor.console_keys k \
             JOIN reactor.operators o ON o.id = k.operator_id \
             WHERE k.token_hash = $1 AND k.operator_id = $2",
        )
        .bind(token_hash(&token))
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?;
        let Some((key_id, scopes, id, email, name, platform_admin)) = row else {
            return Err(ApiError::unauthorized("console token required"));
        };
        return Ok(Operator {
            id,
            email,
            name,
            platform_admin,
            key_id: Some(key_id),
            scopes,
        });
    }
    if claims.aud != "console" {
        return Err(ApiError::unauthorized("console token required"));
    }
    load_operator(state, id).await
}

fn rank(role: &str) -> i32 {
    match role {
        "owner" => 3,
        "admin" => 2,
        "developer" => 1,
        _ => 0,
    }
}

pub(crate) struct ProjectRow {
    pub(crate) id: Uuid,
    pub(crate) pref: String,
    #[allow(dead_code)]
    pub(crate) name: String,
    #[allow(dead_code)]
    pub(crate) role: String,
}

pub(crate) async fn load_operator(state: &AppState, id: Uuid) -> Result<Operator, ApiError> {
    let row: Option<(Uuid, String, String, bool)> = sqlx::query_as(
        "SELECT id, email, name, platform_admin FROM reactor.operators WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some((id, email, name, platform_admin)) = row else {
        return Err(ApiError::unauthorized("console token required"));
    };
    Ok(session_operator(id, email, name, platform_admin))
}

pub(crate) async fn require_member(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    min: &str,
) -> Result<ProjectRow, ApiError> {
    let pref = ProjectRef::parse(pref)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid ref"))?;
    let row: Option<(Uuid, String, String, String)> = sqlx::query_as(
        "SELECT p.id, p.ref, p.name, m.role FROM reactor.projects p \
         JOIN reactor.memberships m ON m.project_id = p.id \
         WHERE p.ref = $1 AND m.operator_id = $2",
    )
    .bind(pref.as_str())
    .bind(operator.id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some((id, pref, name, role)) = row else {
        return Err(ApiError::forbidden("not a member of this project"));
    };
    if rank(&role) < rank(min) {
        return Err(ApiError::forbidden("insufficient role"));
    }
    Ok(ProjectRow {
        id,
        pref,
        name,
        role,
    })
}

pub(crate) fn ident(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 63
        && bytes[0].is_ascii_alphabetic()
        && bytes
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b'_')
}

async fn setup_status(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM reactor.settings")
        .fetch_one(&state.pool)
        .await
        .map_err(internal)?;
    let name: Option<String> =
        sqlx::query_scalar("SELECT cluster_name FROM reactor.settings WHERE id = 1")
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    Ok(Json(json!({
        "needs_setup": count == 0,
        "cluster_name": name,
    })))
}

#[derive(Deserialize)]
struct SetupBody {
    cluster_name: String,
    name: String,
    email: String,
    password: String,
}

async fn setup(
    State(state): State<AppState>,
    Json(body): Json<SetupBody>,
) -> Result<Json<Value>, ApiError> {
    let existing: i64 = sqlx::query_scalar("SELECT count(*) FROM reactor.settings")
        .fetch_one(&state.pool)
        .await
        .map_err(internal)?;
    if existing > 0 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "cluster already set up",
        ));
    }
    if body.cluster_name.trim().is_empty()
        || body.email.trim().is_empty()
        || body.password.len() < 8
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "cluster name, email, and a password of 8 or more characters are required",
        ));
    }
    let id = Uuid::new_v4();
    let hash = hash_password(&body.password).map_err(internal)?;
    sqlx::query("INSERT INTO reactor.settings (id, cluster_name) VALUES (1, $1)")
        .bind(body.cluster_name.trim())
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    sqlx::query(
        "INSERT INTO reactor.operators (id, email, name, password_hash, platform_admin) VALUES ($1, $2, $3, $4, true)",
    )
    .bind(id)
    .bind(body.email.trim().to_lowercase())
    .bind(body.name.trim())
    .bind(hash)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    let email = body.email.trim().to_lowercase();
    let name = body.name.trim().to_string();
    crate::mfa::password_gate(&state, id, &email, &name, true).await
}

async fn restart_setup(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let operator = crate::mfa::setup_operator(&state, &headers).await?;
    if !operator.platform_admin {
        return Err(ApiError::forbidden(
            "only a cluster admin can restart setup",
        ));
    }
    let projects: i64 = sqlx::query_scalar("SELECT count(*) FROM reactor.projects")
        .fetch_one(&state.pool)
        .await
        .map_err(internal)?;
    if projects > 0 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "this cluster already has projects",
        ));
    }
    let mut tx = state.pool.begin().await.map_err(internal)?;
    sqlx::query("DELETE FROM reactor.operators")
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    sqlx::query("DELETE FROM reactor.settings")
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct LoginBody {
    email: String,
    password: String,
}

async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginBody>,
) -> Result<Json<Value>, ApiError> {
    let row: Option<(Uuid, String, String, String, bool)> = sqlx::query_as(
        "SELECT id, email, name, password_hash, platform_admin FROM reactor.operators WHERE email = $1",
    )
    .bind(body.email.trim().to_lowercase())
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some((id, email, name, password_hash, platform_admin)) = row else {
        return Err(ApiError::unauthorized("invalid email or password"));
    };
    if !verify_password(&body.password, &password_hash) {
        return Err(ApiError::unauthorized("invalid email or password"));
    }
    crate::mfa::password_gate(&state, id, &email, &name, platform_admin).await
}

async fn me(State(state): State<AppState>, headers: HeaderMap) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let projects = memberships(&state.pool, operator.id)
        .await
        .map_err(internal)?;
    let cluster: Option<String> =
        sqlx::query_scalar("SELECT cluster_name FROM reactor.settings WHERE id = 1")
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    Ok(Json(json!({
        "operator": {
            "id": operator.id,
            "email": operator.email,
            "name": operator.name,
            "platform_admin": operator.platform_admin,
        },
        "cluster_name": cluster,
        "projects": projects,
    })))
}

async fn memberships(pool: &sqlx::PgPool, operator_id: Uuid) -> Result<Vec<Value>, sqlx::Error> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT p.ref, p.name, m.role FROM reactor.memberships m \
         JOIN reactor.projects p ON p.id = m.project_id \
         WHERE m.operator_id = $1 ORDER BY p.name, p.ref",
    )
    .bind(operator_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(pref, name, role)| json!({"ref": pref, "name": name, "role": role}))
        .collect())
}

pub(crate) async fn require_platform_admin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Operator, ApiError> {
    let operator = require_operator(state, headers).await?;
    if !operator.platform_admin {
        return Err(ApiError::forbidden("platform admin required"));
    }
    Ok(operator)
}

async fn cluster(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let _operator = require_platform_admin(&state, &headers).await?;
    let name: Option<String> =
        sqlx::query_scalar("SELECT cluster_name FROM reactor.settings WHERE id = 1")
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    let catalog: Vec<(String, String)> =
        sqlx::query_as("SELECT ref, name FROM reactor.projects ORDER BY name, ref")
            .fetch_all(&state.pool)
            .await
            .map_err(internal)?;
    let db_ok = sqlx::query("SELECT 1").execute(&state.pool).await.is_ok();
    let size: Option<i64> = sqlx::query_scalar("SELECT pg_database_size(current_database())")
        .fetch_optional(&state.pool)
        .await
        .ok()
        .flatten();
    let connections: Option<i64> = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database()",
    )
    .fetch_optional(&state.pool)
    .await
    .ok()
    .flatten();
    let postgrest = probe(&state, &state.config.postgrest_url).await;
    let dedicated_projects: i64 =
        sqlx::query_scalar("SELECT count(*) FROM reactor.projects WHERE database_url IS NOT NULL")
            .fetch_one(&state.pool)
            .await
            .unwrap_or(0);
    let dedicated = if dedicated_projects == 0 {
        json!({ "active": false })
    } else {
        let ok = !state.config.postgrest_dedicated_url.is_empty()
            && probe(&state, &state.config.postgrest_dedicated_url).await;
        json!({ "active": true, "ok": ok })
    };
    let blobs = state.blobs.list_prefix("").await.is_ok();
    Ok(Json(json!({
        "name": name.unwrap_or_default(),
        "type": state.config.mode,
        "place": state.config.place,
        "functions_runtime": state.config.functions_runtime,
        "projects": catalog.into_iter().map(|(pref, name)| json!({"ref": pref, "name": name})).collect::<Vec<_>>(),
        "reactor": { "ok": true, "bind": state.config.bind },
        "postgres": { "ok": db_ok, "bytes": size, "connections": connections },
        "postgrest": { "ok": postgrest, "url": state.config.postgrest_url },
        "postgrest_dedicated": dedicated,
        "blobs": { "ok": blobs, "backend": state.config.storage_backend },
    })))
}

#[derive(Deserialize)]
struct RenameCluster {
    cluster_name: String,
}

async fn rename_cluster(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RenameCluster>,
) -> Result<Json<Value>, ApiError> {
    let _operator = require_platform_admin(&state, &headers).await?;
    let name = body.cluster_name.trim();
    if name.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "cluster name is required",
        ));
    }
    sqlx::query("UPDATE reactor.settings SET cluster_name = $1 WHERE id = 1")
        .bind(name)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "name": name })))
}

async fn list_operators(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let _operator = require_platform_admin(&state, &headers).await?;
    let people: Vec<(Uuid, String, String, bool, String)> = sqlx::query_as(
        "SELECT id, email, name, platform_admin, created_at::text FROM reactor.operators ORDER BY email",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let links: Vec<(Uuid, String, String, String)> = sqlx::query_as(
        "SELECT m.operator_id, p.ref, p.name, m.role FROM reactor.memberships m \
         JOIN reactor.projects p ON p.id = m.project_id ORDER BY p.name",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let mut access: HashMap<Uuid, Vec<Value>> = HashMap::new();
    for (id, pref, name, role) in links {
        access
            .entry(id)
            .or_default()
            .push(json!({ "ref": pref, "name": name, "role": role }));
    }
    let operators: Vec<Value> = people
        .into_iter()
        .map(|(id, email, name, platform_admin, created)| {
            json!({
                "id": id,
                "email": email,
                "name": name,
                "platform_admin": platform_admin,
                "created_at": created,
                "projects": access.remove(&id).unwrap_or_default(),
            })
        })
        .collect();
    Ok(Json(json!(operators)))
}

#[derive(Deserialize)]
struct OperatorCreate {
    email: String,
    name: String,
    password: String,
    platform_admin: bool,
}

async fn create_operator(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<OperatorCreate>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let _operator = require_platform_admin(&state, &headers).await?;
    let email = body.email.trim().to_lowercase();
    let name = body.name.trim();
    if email.is_empty() || name.is_empty() || body.password.len() < 8 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "name, email, and a password of 8 or more characters are required",
        ));
    }
    let taken: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM reactor.operators WHERE email = $1")
            .bind(&email)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    if taken.is_some() {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "that email already has a console account",
        ));
    }
    let id = Uuid::new_v4();
    let hash = hash_password(&body.password).map_err(internal)?;
    sqlx::query(
        "INSERT INTO reactor.operators (id, email, name, password_hash, platform_admin) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(&email)
    .bind(name)
    .bind(hash)
    .bind(body.platform_admin)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "email": email,
            "name": name,
            "platform_admin": body.platform_admin,
            "projects": [],
        })),
    ))
}

#[derive(Deserialize)]
struct OperatorUpdate {
    platform_admin: bool,
}

async fn update_operator(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(operator_id): Path<String>,
    Json(body): Json<OperatorUpdate>,
) -> Result<StatusCode, ApiError> {
    let _operator = require_platform_admin(&state, &headers).await?;
    let id = Uuid::parse_str(&operator_id)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid operator"))?;
    let current: Option<bool> =
        sqlx::query_scalar("SELECT platform_admin FROM reactor.operators WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    let Some(current) = current else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "operator not found"));
    };
    if current && !body.platform_admin {
        let admins: i64 =
            sqlx::query_scalar("SELECT count(*) FROM reactor.operators WHERE platform_admin")
                .fetch_one(&state.pool)
                .await
                .map_err(internal)?;
        if admins <= 1 {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "the cluster needs a platform admin",
            ));
        }
    }
    sqlx::query("UPDATE reactor.operators SET platform_admin = $1 WHERE id = $2")
        .bind(body.platform_admin)
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_operator(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(operator_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let operator = require_platform_admin(&state, &headers).await?;
    let id = Uuid::parse_str(&operator_id)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid operator"))?;
    if id == operator.id {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "you cannot delete your own account",
        ));
    }
    let current: Option<bool> =
        sqlx::query_scalar("SELECT platform_admin FROM reactor.operators WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    let Some(current) = current else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "operator not found"));
    };
    if current {
        let admins: i64 =
            sqlx::query_scalar("SELECT count(*) FROM reactor.operators WHERE platform_admin")
                .fetch_one(&state.pool)
                .await
                .map_err(internal)?;
        if admins <= 1 {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "the cluster needs a platform admin",
            ));
        }
    }
    sqlx::query("DELETE FROM reactor.operators WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct GrantBody {
    role: String,
}

async fn grant_operator(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((operator_id, pref)): Path<(String, String)>,
    Json(body): Json<GrantBody>,
) -> Result<StatusCode, ApiError> {
    let _operator = require_platform_admin(&state, &headers).await?;
    if rank(&body.role) == 0 {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "role must be owner, admin, or developer",
        ));
    }
    let id = Uuid::parse_str(&operator_id)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid operator"))?;
    let exists: Option<Uuid> = sqlx::query_scalar("SELECT id FROM reactor.operators WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?;
    if exists.is_none() {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "operator not found"));
    }
    let project_id: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM reactor.projects WHERE ref = $1")
            .bind(&pref)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    let Some(project_id) = project_id else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "project not found"));
    };
    sqlx::query(
        "INSERT INTO reactor.memberships (operator_id, project_id, role) VALUES ($1, $2, $3) \
         ON CONFLICT (operator_id, project_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(id)
    .bind(project_id)
    .bind(&body.role)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke_operator(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((operator_id, pref)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let _operator = require_platform_admin(&state, &headers).await?;
    let id = Uuid::parse_str(&operator_id)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid operator"))?;
    let project_id: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM reactor.projects WHERE ref = $1")
            .bind(&pref)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    let Some(project_id) = project_id else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "project not found"));
    };
    sqlx::query("DELETE FROM reactor.memberships WHERE operator_id = $1 AND project_id = $2")
        .bind(id)
        .bind(project_id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn probe(state: &AppState, url: &str) -> bool {
    state
        .http
        .get(url)
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
        .map(|r| r.status().as_u16() < 500)
        .unwrap_or(false)
}

#[derive(Deserialize)]
struct KeyBody {
    name: String,
    scopes: Vec<String>,
}

fn reject_key(operator: &Operator) -> Result<(), ApiError> {
    if operator.key_id.is_some() {
        Err(ApiError::forbidden("console key cannot call this"))
    } else {
        Ok(())
    }
}

async fn list_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    reject_key(&operator)?;
    let rows: Vec<(Uuid, String, Vec<String>, String)> = sqlx::query_as(
        "SELECT id, name, scopes, created_at::text FROM reactor.console_keys \
         WHERE operator_id = $1 ORDER BY created_at",
    )
    .bind(operator.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let keys: Vec<Value> = rows
        .into_iter()
        .map(|(id, name, scopes, created_at)| {
            json!({
                "id": id,
                "name": name,
                "scopes": scopes,
                "created_at": created_at,
            })
        })
        .collect();
    Ok(Json(json!(keys)))
}

async fn create_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<KeyBody>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    reject_key(&operator)?;
    let name = body.name.trim();
    if name.is_empty() || name.len() > 80 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "name is required"));
    }
    let scopes = clean_scopes(&body.scopes)?;
    let id = Uuid::new_v4();
    let token = state
        .issuer
        .sign_audience(
            &operator.id.to_string(),
            "console-key",
            60 * 60 * 24 * 365 * 10,
        )
        .map_err(internal)?;
    sqlx::query(
        "INSERT INTO reactor.console_keys (id, operator_id, name, token_hash, scopes) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(operator.id)
    .bind(name)
    .bind(token_hash(&token))
    .bind(&scopes)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(json!({
        "id": id,
        "name": name,
        "scopes": scopes,
        "token": token,
    })))
}

async fn delete_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    reject_key(&operator)?;
    let deleted =
        sqlx::query("DELETE FROM reactor.console_keys WHERE id = $1 AND operator_id = $2")
            .bind(id)
            .bind(operator.id)
            .execute(&state.pool)
            .await
            .map_err(internal)?;
    if deleted.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn list_projects(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let projects = memberships(&state.pool, operator.id)
        .await
        .map_err(internal)?;
    Ok(Json(json!(projects)))
}

#[derive(Deserialize)]
struct CreateProject {
    name: String,
}

async fn create_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateProject>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "name is required"));
    }
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
    crate::db::apply_project(
        &state.pool,
        std::path::Path::new(&state.config.sql_dir),
        id,
        &schema,
    )
    .await
    .map_err(internal)?;
    crate::project_auth::insert_settings(&state.pool, id)
        .await
        .map_err(internal)?;
    let (anon, service) = issue_keys(&state, id, &pref).await?;
    sqlx::query(
        "INSERT INTO reactor.memberships (operator_id, project_id, role) VALUES ($1, $2, 'owner')",
    )
    .bind(operator.id)
    .bind(id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    if let Some(key_id) = operator.key_id {
        sqlx::query("UPDATE reactor.projects SET created_by_key = $1 WHERE id = $2")
            .bind(key_id)
            .bind(id)
            .execute(&state.pool)
            .await
            .map_err(internal)?;
    }
    reload(&state).await?;
    Ok(Json(json!({
        "id": id,
        "ref": pref,
        "name": name,
        "role": "owner",
        "anon_key": anon,
        "service_key": service,
    })))
}

#[derive(Deserialize)]
struct RenameProject {
    name: String,
}

async fn rename_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
    Json(body): Json<RenameProject>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "owner").await?;
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "name is required"));
    }
    sqlx::query("UPDATE reactor.projects SET name = $1 WHERE id = $2")
        .bind(&name)
        .bind(project.id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "name": name })))
}

async fn delete_project(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
) -> Result<StatusCode, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "owner").await?;
    let url: Option<String> =
        sqlx::query_scalar("SELECT database_url FROM reactor.projects WHERE id = $1")
            .bind(project.id)
            .fetch_one(&state.pool)
            .await
            .map_err(internal)?;
    let keys = state
        .blobs
        .list_objects(&format!("{}/", project.pref))
        .await
        .map_err(internal)?;
    for item in keys {
        state.blobs.delete_key(&item.key).await.map_err(internal)?;
    }
    let pool = crate::pool_for(&state, url.as_deref())
        .await
        .map_err(internal)?;
    let schema = schema_name(&project.pref)?;
    sqlx::raw_sql(&format!("DROP SCHEMA IF EXISTS \"{schema}\" CASCADE"))
        .execute(&pool)
        .await
        .map_err(internal)?;
    sqlx::query("DELETE FROM reactor.projects WHERE id = $1")
        .bind(project.id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    reload(&state).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn issue_keys(
    state: &AppState,
    id: Uuid,
    pref: &str,
) -> Result<(String, String), ApiError> {
    let anon = state
        .issuer
        .sign(pref, "anon", "anon", 60 * 60 * 24 * 365 * 10)
        .map_err(internal)?;
    let service = state
        .issuer
        .sign(pref, "service", "service", 60 * 60 * 24 * 365 * 10)
        .map_err(internal)?;
    store_api_key(&state.pool, id, "anon", &anon)
        .await
        .map_err(internal)?;
    store_api_key(&state.pool, id, "service", &service)
        .await
        .map_err(internal)?;
    Ok((anon, service))
}

async fn rotate_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    sqlx::query("DELETE FROM reactor.api_keys WHERE project_id = $1")
        .bind(project.id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    let (anon, service) = issue_keys(&state, project.id, &project.pref).await?;
    Ok(Json(json!({ "anon_key": anon, "service_key": service })))
}

async fn list_members(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "developer").await?;
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT o.email, o.name, m.role FROM reactor.memberships m \
         JOIN reactor.operators o ON o.id = m.operator_id \
         WHERE m.project_id = $1 ORDER BY o.email",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let members: Vec<Value> = rows
        .into_iter()
        .map(|(email, name, role)| json!({"email": email, "name": name, "role": role}))
        .collect();
    Ok(Json(json!(members)))
}

#[derive(Deserialize)]
struct MemberBody {
    email: String,
    name: String,
    password: String,
    role: String,
}

async fn add_member(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
    Json(body): Json<MemberBody>,
) -> Result<StatusCode, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    if rank(&body.role) == 0 || body.role == "owner" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "role must be admin or developer",
        ));
    }
    if body.password.len() < 8 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "password too short"));
    }
    let email = body.email.trim().to_lowercase();
    let existing: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM reactor.operators WHERE email = $1")
            .bind(&email)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    let operator_id = if let Some(id) = existing {
        id
    } else {
        let id = Uuid::new_v4();
        let hash = hash_password(&body.password).map_err(internal)?;
        sqlx::query(
            "INSERT INTO reactor.operators (id, email, name, password_hash, platform_admin) VALUES ($1, $2, $3, $4, false)",
        )
        .bind(id)
        .bind(&email)
        .bind(body.name.trim())
        .bind(hash)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
        id
    };
    sqlx::query(
        "INSERT INTO reactor.memberships (operator_id, project_id, role) VALUES ($1, $2, $3) \
         ON CONFLICT (operator_id, project_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(operator_id)
    .bind(project.id)
    .bind(&body.role)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(StatusCode::CREATED)
}

async fn list_users(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    let rows: Vec<(Uuid, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, email, created_at::text, email_verified_at::text FROM reactor.users WHERE project_id = $1 ORDER BY created_at",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let users: Vec<Value> = rows
        .into_iter()
        .map(|(id, email, created, verified)| {
            json!({"id": id, "email": email, "created_at": created, "email_verified_at": verified})
        })
        .collect();
    Ok(Json(json!(users)))
}

async fn get_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, user_id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "developer").await?;
    let row: Option<(Uuid, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, email, created_at::text, email_verified_at::text FROM reactor.users WHERE id = $1 AND project_id = $2",
    )
    .bind(user_id)
    .bind(project.id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some((id, email, created, verified)) = row else {
        return Err(ApiError::not_found());
    };
    Ok(Json(
        json!({ "id": id, "email": email, "created_at": created, "email_verified_at": verified }),
    ))
}

#[derive(Deserialize)]
struct PasswordBody {
    password: String,
}

async fn set_user_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, user_id)): Path<(String, Uuid)>,
    Json(body): Json<PasswordBody>,
) -> Result<StatusCode, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    if body.password.len() < 8 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "password too short"));
    }
    let hash = hash_password(&body.password).map_err(internal)?;
    let updated = sqlx::query(
        "UPDATE reactor.users SET password_hash = $1 WHERE id = $2 AND project_id = $3",
    )
    .bind(hash)
    .bind(user_id)
    .bind(project.id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_user(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, user_id)): Path<(String, Uuid)>,
) -> Result<StatusCode, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    let deleted = sqlx::query("DELETE FROM reactor.users WHERE id = $1 AND project_id = $2")
        .bind(user_id)
        .bind(project.id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    if deleted.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, Default)]
struct SchemaQuery {
    schema: Option<String>,
}

async fn data_pool(state: &AppState, project_id: Uuid) -> Result<sqlx::PgPool, ApiError> {
    let url: Option<String> =
        sqlx::query_scalar("SELECT database_url FROM reactor.projects WHERE id = $1")
            .bind(project_id)
            .fetch_one(&state.pool)
            .await
            .map_err(internal)?;
    pool_for(state, url.as_deref()).await.map_err(internal)
}

pub(crate) fn owned_schema(pref: &str, name: &str) -> bool {
    let Ok(own) = schema_name(pref) else {
        return false;
    };
    ident(name) && (name == own || name.starts_with(&format!("{own}_")))
}

async fn list_owned_schemas(pool: &sqlx::PgPool, pref: &str) -> Result<Vec<String>, ApiError> {
    let own = schema_name(pref)?;
    let names: Vec<String> = sqlx::query_scalar(
        "SELECT nspname FROM pg_namespace \
         WHERE nspname = $1 OR starts_with(nspname, $2) \
         ORDER BY nspname",
    )
    .bind(&own)
    .bind(format!("{own}_"))
    .fetch_all(pool)
    .await
    .map_err(internal)?;
    Ok(names
        .into_iter()
        .filter(|name| owned_schema(pref, name))
        .collect())
}

async fn resolve_schema(
    pool: &sqlx::PgPool,
    pref: &str,
    requested: Option<&str>,
) -> Result<String, ApiError> {
    let own = schema_name(pref)?;
    let name = requested.filter(|value| !value.is_empty()).unwrap_or(&own);
    if !owned_schema(pref, name) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid schema"));
    }
    let found: Option<String> = sqlx::query_scalar("SELECT nspname FROM pg_namespace WHERE nspname = $1")
        .bind(name)
        .fetch_optional(pool)
        .await
        .map_err(internal)?;
    found.ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "schema not found"))
}

async fn schemas(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "developer").await?;
    let pool = data_pool(&state, project.id).await?;
    let names = list_owned_schemas(&pool, &project.pref).await?;
    let mut out = Vec::new();
    for name in names {
        let tables: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM information_schema.tables \
             WHERE table_schema = $1 AND table_type = 'BASE TABLE'",
        )
        .bind(&name)
        .fetch_one(&pool)
        .await
        .map_err(internal)?;
        out.push(json!({ "name": name, "tables": tables }));
    }
    Ok(Json(json!(out)))
}

async fn schema(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
    Query(query): Query<SchemaQuery>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "developer").await?;
    let pool = data_pool(&state, project.id).await?;
    let schema = resolve_schema(&pool, &project.pref, query.schema.as_deref()).await?;
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name FROM information_schema.tables \
         WHERE table_schema = $1 AND table_type = 'BASE TABLE' ORDER BY table_name",
    )
    .bind(&schema)
    .fetch_all(&pool)
    .await
    .map_err(internal)?;
    let mut out = Vec::new();
    for table in tables {
        let columns = column_types(&pool, &schema, &table).await?;
        let pk = primary_key(&pool, &schema, &table).await?;
        out.push(json!({
            "name": table,
            "primary_key": pk.as_ref().map(|(name, _)| name),
            "columns": columns.into_iter().map(|(n, t)| json!({"name": n, "type": t})).collect::<Vec<_>>(),
        }));
    }
    Ok(Json(json!(out)))
}

#[derive(Deserialize)]
struct CreateTable {
    name: String,
    columns: Vec<ColumnSpec>,
}

#[derive(Deserialize)]
struct ColumnSpec {
    name: String,
    #[serde(rename = "type")]
    ty: String,
}

async fn create_table(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
    Query(query): Query<SchemaQuery>,
    Json(body): Json<CreateTable>,
) -> Result<StatusCode, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    let pool = data_pool(&state, project.id).await?;
    let schema = resolve_schema(&pool, &project.pref, query.schema.as_deref()).await?;
    if !ident(&body.name) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid table"));
    }
    let mut defs = Vec::new();
    let mut has_id = false;
    let mut has_user = false;
    for column in &body.columns {
        if !ident(&column.name) {
            return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid column"));
        }
        let ty = sql_type(&column.ty)
            .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "unsupported column type"))?;
        if column.name == "id" {
            has_id = true;
        }
        if column.name == "user_id" {
            has_user = true;
        }
        defs.push(format!("\"{}\" {ty}", column.name));
    }
    if !has_id {
        defs.insert(0, "id uuid PRIMARY KEY DEFAULT gen_random_uuid()".into());
    }
    let using = if has_user {
        "user_id::text = current_setting('request.jwt.claims', true)::json->>'sub'"
    } else {
        "true"
    };
    let sql = format!(
        "CREATE TABLE \"{schema}\".\"{table}\" ({defs}); \
         ALTER TABLE \"{schema}\".\"{table}\" ENABLE ROW LEVEL SECURITY; \
         ALTER TABLE \"{schema}\".\"{table}\" FORCE ROW LEVEL SECURITY; \
         CREATE POLICY \"{table}_access\" ON \"{schema}\".\"{table}\" FOR ALL TO authenticated USING ({using}) WITH CHECK ({using}); \
         GRANT SELECT, INSERT, UPDATE, DELETE ON \"{schema}\".\"{table}\" TO authenticated, service;",
        table = body.name,
        defs = defs.join(", "),
    );
    sqlx::raw_sql(&sql)
        .execute(&pool)
        .await
        .map_err(internal)?;
    reload(&state).await?;
    Ok(StatusCode::CREATED)
}

#[derive(Deserialize)]
struct MigrationBody {
    version: String,
    sql: String,
}

pub(crate) fn migration_issue(version: &str, sql: &str) -> Option<&'static str> {
    let bytes = version.as_bytes();
    let Some(name) = version.strip_suffix(".sql") else {
        return Some("invalid version");
    };
    let Some((number, rest)) = name.split_once('_') else {
        return Some("invalid version");
    };
    if number.len() != 4
        || !number.bytes().all(|c| c.is_ascii_digit())
        || rest.is_empty()
        || !rest
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        || bytes.len() > 80
    {
        return Some("invalid version");
    }
    if sql.len() > 200_000 || sql.trim().is_empty() {
        return Some("invalid sql");
    }
    let lower = sql.to_ascii_lowercase();
    if lower.contains("reactor.")
        || lower.contains("proj_")
        || lower.contains("public.")
        || sql.contains("--")
        || sql.contains("/*")
    {
        return Some("sql is not allowed");
    }
    for statement in sql_statements(sql) {
        let trimmed = statement.trim();
        if trimmed.is_empty() {
            continue;
        }
        let word = trimmed
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_ascii_uppercase();
        if word != "CREATE"
            && word != "ALTER"
            && word != "GRANT"
            && word != "REVOKE"
            && word != "INSERT"
            && word != "COMMENT"
        {
            return Some("sql is not allowed");
        }
    }
    None
}

pub(crate) fn sql_statements(sql: &str) -> Vec<&str> {
    let bytes = sql.as_bytes();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut parts = Vec::new();
    while i < bytes.len() {
        match bytes[i] {
            b'\'' | b'"' => {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() {
                    if bytes[i] == quote {
                        if i + 1 < bytes.len() && bytes[i + 1] == quote {
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            b'$' => {
                let tag_at = i;
                let mut j = i + 1;
                while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'$' {
                    let tag = &sql[tag_at..=j];
                    let after = j + 1;
                    if let Some(found) = sql[after..].find(tag) {
                        i = after + found + tag.len();
                        continue;
                    }
                }
                i += 1;
            }
            b';' => {
                parts.push(&sql[start..i]);
                i += 1;
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < sql.len() {
        parts.push(&sql[start..]);
    }
    parts
}

fn pg_text(err: sqlx::Error) -> String {
    match &err {
        sqlx::Error::Database(db) => db.message().to_string(),
        _ => err.to_string(),
    }
}

async fn apply_migration(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
    Json(body): Json<MigrationBody>,
) -> Result<Json<Value>, ApiError> {
    run_migration(state, headers, pref, body).await
}

async fn run_migration(
    state: AppState,
    headers: HeaderMap,
    pref: String,
    body: MigrationBody,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    let schema = schema_name(&project.pref)?;
    if let Some(issue) = migration_issue(&body.version, &body.sql) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, issue));
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM reactor.schema_migrations WHERE project_id = $1 AND version = $2",
    )
    .bind(project.id)
    .bind(&body.version)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    if count > 0 {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "migration already applied",
        ));
    }
    let mut tx = state.pool.begin().await.map_err(internal)?;
    if let Err(err) = sqlx::query(&format!("SET LOCAL search_path TO \"{schema}\""))
        .execute(&mut *tx)
        .await
    {
        return Err(internal(err));
    }
    for statement in sql_statements(&body.sql) {
        let trimmed = statement.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Err(err) = sqlx::query(trimmed).execute(&mut *tx).await {
            return Err(ApiError::new(StatusCode::BAD_REQUEST, pg_text(err)));
        }
    }
    sqlx::query("INSERT INTO reactor.schema_migrations (project_id, version) VALUES ($1, $2)")
        .bind(project.id)
        .bind(&body.version)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    reload(&state).await?;
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
    kind: Option<String>,
    name: Option<String>,
    schema: Option<String>,
}

async fn table_rows(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, table)): Path<(String, String)>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "developer").await?;
    let pool = data_pool(&state, project.id).await?;
    let schema = resolve_schema(&pool, &project.pref, query.schema.as_deref()).await?;
    if !ident(&table) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid table"));
    }
    let q = query.q.unwrap_or_default();
    let limit = query.limit.unwrap_or(100).clamp(1, 100);
    let offset = query.offset.unwrap_or(0).max(0);
    let order = match primary_key(&pool, &schema, &table).await? {
        Some((pk, _)) if !pk.contains('"') => format!(" ORDER BY t.\"{pk}\""),
        _ => String::new(),
    };
    let filter = "WHERE ($1 = '' OR to_jsonb(t)::text ILIKE '%' || $1 || '%')";
    let total: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM \"{schema}\".\"{table}\" t {filter}"
    ))
    .bind(&q)
    .fetch_one(&pool)
    .await
    .map_err(internal)?;
    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT to_jsonb(t) FROM \"{schema}\".\"{table}\" t {filter}{order} LIMIT $2 OFFSET $3"
    ))
    .bind(&q)
    .bind(limit)
    .bind(offset)
    .fetch_all(&pool)
    .await
    .map_err(internal)?;
    Ok(Json(json!({ "rows": rows, "total": total })))
}

#[derive(Deserialize)]
struct RowValues {
    values: std::collections::BTreeMap<String, Value>,
}

async fn insert_row(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, table)): Path<(String, String)>,
    Query(query): Query<SchemaQuery>,
    Json(body): Json<RowValues>,
) -> Result<StatusCode, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    let pool = data_pool(&state, project.id).await?;
    let schema = resolve_schema(&pool, &project.pref, query.schema.as_deref()).await?;
    if !ident(&table) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid table"));
    }
    let columns = column_types(&pool, &schema, &table).await?;
    let mut names = Vec::new();
    let mut casts = Vec::new();
    let mut values = Vec::new();
    for (name, ty) in &columns {
        let Some(value) = body.values.get(name) else {
            continue;
        };
        let text = json_text(value);
        if text.is_empty() {
            continue;
        }
        let cast = cast_for(ty)
            .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "column cannot be written"))?;
        names.push(format!("\"{name}\""));
        casts.push(format!("${}::{cast}", names.len()));
        values.push(text);
    }
    let sql = if names.is_empty() {
        format!("INSERT INTO \"{schema}\".\"{table}\" DEFAULT VALUES")
    } else {
        format!(
            "INSERT INTO \"{schema}\".\"{table}\" ({}) VALUES ({})",
            names.join(", "),
            casts.join(", ")
        )
    };
    exec_in_schema(&pool, &schema, &sql, &values).await?;
    Ok(StatusCode::CREATED)
}

#[derive(Deserialize)]
struct CellUpdate {
    pk: String,
    column: String,
    value: Value,
}

async fn update_row(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, table)): Path<(String, String)>,
    Query(query): Query<SchemaQuery>,
    Json(body): Json<CellUpdate>,
) -> Result<StatusCode, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    let pool = data_pool(&state, project.id).await?;
    let schema = resolve_schema(&pool, &project.pref, query.schema.as_deref()).await?;
    if !ident(&table) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid table"));
    }
    let (pk, pk_ty) = primary_key(&pool, &schema, &table)
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "table has no single primary key"))?;
    if !ident(&body.column) || body.column == pk {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid column"));
    }
    let columns = column_types(&pool, &schema, &table).await?;
    let ty = columns
        .iter()
        .find(|(name, _)| name == &body.column)
        .map(|(_, ty)| ty.as_str())
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "invalid column"))?;
    let cast = cast_for(ty)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "column cannot be written"))?;
    let pk_cast = cast_for(&pk_ty).unwrap_or("text");
    let sql = format!(
        "UPDATE \"{schema}\".\"{table}\" SET \"{}\" = $1::{cast} WHERE \"{pk}\" = $2::{pk_cast}",
        body.column
    );
    let updated = exec_in_schema(
        &pool,
        &schema,
        &sql,
        &[json_text(&body.value), body.pk.clone()],
    )
    .await?;
    if updated == 0 {
        return Err(ApiError::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct RowKey {
    pk: String,
}

async fn delete_row(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, table)): Path<(String, String)>,
    Query(query): Query<SchemaQuery>,
    Json(body): Json<RowKey>,
) -> Result<StatusCode, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    let pool = data_pool(&state, project.id).await?;
    let schema = resolve_schema(&pool, &project.pref, query.schema.as_deref()).await?;
    if !ident(&table) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid table"));
    }
    let (pk, pk_ty) = primary_key(&pool, &schema, &table)
        .await?
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "table has no single primary key"))?;
    let pk_cast = cast_for(&pk_ty).unwrap_or("text");
    let sql = format!("DELETE FROM \"{schema}\".\"{table}\" WHERE \"{pk}\" = $1::{pk_cast}");
    let deleted = exec_in_schema(&pool, &schema, &sql, &[body.pk.clone()]).await?;
    if deleted == 0 {
        return Err(ApiError::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) fn schema_name(pref: &str) -> Result<String, ApiError> {
    let schema = format!("proj_{pref}");
    if !ident(&schema) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid schema"));
    }
    Ok(schema)
}

pub(crate) fn checked_table(pref: &str, table: &str) -> Result<String, ApiError> {
    if !ident(table) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid table"));
    }
    schema_name(pref)
}

pub(crate) fn sql_type(value: &str) -> Option<&'static str> {
    match value {
        "text" => Some("text"),
        "uuid" => Some("uuid"),
        "int" | "integer" => Some("integer"),
        "bigint" => Some("bigint"),
        "bool" | "boolean" => Some("boolean"),
        "date" => Some("date"),
        "timestamptz" => Some("timestamptz"),
        "jsonb" => Some("jsonb"),
        "numeric" => Some("numeric"),
        _ => None,
    }
}

pub(crate) fn cast_for(data_type: &str) -> Option<&'static str> {
    match data_type {
        "uuid" => Some("uuid"),
        "text" | "character varying" => Some("text"),
        "integer" => Some("integer"),
        "bigint" => Some("bigint"),
        "smallint" => Some("smallint"),
        "boolean" => Some("boolean"),
        "date" => Some("date"),
        "timestamp with time zone" | "timestamptz" => Some("timestamptz"),
        "jsonb" => Some("jsonb"),
        "json" => Some("json"),
        "numeric" => Some("numeric"),
        _ => None,
    }
}

async fn exec_in_schema(
    pool: &sqlx::PgPool,
    schema: &str,
    sql: &str,
    values: &[String],
) -> Result<u64, ApiError> {
    let mut tx = pool.begin().await.map_err(internal)?;
    if let Err(err) = sqlx::query(&format!("SET LOCAL search_path TO \"{schema}\""))
        .execute(&mut *tx)
        .await
    {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, pg_text(err)));
    }
    let mut query = sqlx::query(sql);
    for value in values {
        query = query.bind(value);
    }
    let result = match query.execute(&mut *tx).await {
        Ok(result) => result,
        Err(err) => return Err(ApiError::new(StatusCode::BAD_REQUEST, pg_text(err))),
    };
    tx.commit().await.map_err(internal)?;
    Ok(result.rows_affected())
}

pub(crate) fn json_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

pub(crate) async fn column_types(
    pool: &sqlx::PgPool,
    schema: &str,
    table: &str,
) -> Result<Vec<(String, String)>, ApiError> {
    sqlx::query_as(
        "SELECT column_name, data_type FROM information_schema.columns \
         WHERE table_schema = $1 AND table_name = $2 ORDER BY ordinal_position",
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(internal)
}

pub(crate) async fn primary_key(
    pool: &sqlx::PgPool,
    schema: &str,
    table: &str,
) -> Result<Option<(String, String)>, ApiError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT kcu.column_name, c.data_type \
         FROM information_schema.table_constraints tc \
         JOIN information_schema.key_column_usage kcu \
           ON tc.constraint_name = kcu.constraint_name AND tc.table_schema = kcu.table_schema \
         JOIN information_schema.columns c \
           ON c.table_schema = kcu.table_schema AND c.table_name = kcu.table_name AND c.column_name = kcu.column_name \
         WHERE tc.constraint_type = 'PRIMARY KEY' AND tc.table_schema = $1 AND tc.table_name = $2 \
         ORDER BY kcu.ordinal_position",
    )
    .bind(schema)
    .bind(table)
    .fetch_all(pool)
    .await
    .map_err(internal)?;
    if rows.len() == 1 {
        Ok(Some(rows.into_iter().next().unwrap()))
    } else {
        Ok(None)
    }
}

async fn objects(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "developer").await?;
    let keys = state
        .blobs
        .list_objects(&format!("{}/", project.pref))
        .await
        .map_err(internal)?;
    let keys: Vec<Value> = keys
        .into_iter()
        .filter(|item| user_object(&project.pref, &item.key))
        .map(|item| json!({"key": item.key, "size": item.size}))
        .collect();
    Ok(Json(json!(keys)))
}

#[derive(Deserialize)]
struct ObjectBody {
    key: String,
}

async fn object_url(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
    Json(body): Json<ObjectBody>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "developer").await?;
    if !user_object(&project.pref, &body.key) {
        return Err(ApiError::forbidden("object is outside this project"));
    }
    let url = state
        .blobs
        .presign_get(&body.key, 300)
        .await
        .map_err(internal)?;
    Ok(Json(json!({ "url": url })))
}

#[derive(Deserialize)]
struct UploadBody {
    path: String,
}

async fn upload_url(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
    Json(body): Json<UploadBody>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    let path = body.path.trim().trim_start_matches('/');
    let ok = !path.is_empty()
        && !path.contains("..")
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..");
    if !ok {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid path"));
    }
    let key = format!("{}/{}", project.pref, path);
    if !user_object(&project.pref, &key) {
        return Err(ApiError::forbidden("object is outside this project"));
    }
    let url = state.blobs.presign_put(&key, 300).await.map_err(internal)?;
    Ok(Json(json!({ "url": url, "key": key })))
}

async fn delete_object(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
    Json(body): Json<ObjectBody>,
) -> Result<StatusCode, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    if !user_object(&project.pref, &body.key) {
        return Err(ApiError::forbidden("object is outside this project"));
    }
    state.blobs.delete_key(&body.key).await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) fn user_object(pref: &str, key: &str) -> bool {
    let Some(rest) = key.strip_prefix(&format!("{pref}/")) else {
        return false;
    };
    !key.contains("..")
        && !rest.starts_with("_functions/")
        && !rest.starts_with("_sites/")
        && !rest.is_empty()
}

async fn overview(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "developer").await?;
    let schema = format!("proj_{}", project.pref);
    let users: i64 = sqlx::query_scalar("SELECT count(*) FROM reactor.users WHERE project_id = $1")
        .bind(project.id)
        .fetch_one(&state.pool)
        .await
        .map_err(internal)?;
    let tables: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.tables WHERE table_schema = $1 AND table_type = 'BASE TABLE'",
    )
    .bind(&schema)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    let functions: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT name) FROM reactor.deployments WHERE project_id = $1",
    )
    .bind(project.id)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    let sites: i64 =
        sqlx::query_scalar("SELECT count(*) FROM reactor.site_files WHERE project_id = $1")
            .bind(project.id)
            .fetch_one(&state.pool)
            .await
            .map_err(internal)?;
    let stored = state
        .blobs
        .list_objects(&format!("{}/", project.pref))
        .await
        .map_err(internal)?;
    let files = stored
        .iter()
        .filter(|item| user_object(&project.pref, &item.key))
        .count();
    let hours: Vec<i64> = sqlx::query_scalar(
        "SELECT (extract(epoch FROM date_trunc('hour', now()) - make_interval(hours => n)))::bigint \
         FROM generate_series(23, 0, -1) AS n",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let rows: Vec<(String, i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT kind, \
                (extract(epoch FROM date_trunc('hour', created_at)))::bigint, \
                count(*) FILTER (WHERE status < 400), \
                count(*) FILTER (WHERE status >= 400 AND status < 500), \
                count(*) FILTER (WHERE status >= 500) \
         FROM reactor.logs \
         WHERE project_id = $1 \
           AND created_at >= date_trunc('hour', now()) - interval '23 hours' \
           AND kind IN ('auth', 'database', 'function', 'site') \
         GROUP BY 1, 2",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let mut counts: HashMap<(String, i64), (i64, i64, i64)> = HashMap::new();
    for (kind, hour, ok, warn, error) in rows {
        counts.insert((kind, hour), (ok, warn, error));
    }
    let series = |kind: &str| {
        hours
            .iter()
            .map(|hour| {
                let (ok, warn, error) = counts
                    .get(&(kind.to_string(), *hour))
                    .copied()
                    .unwrap_or((0, 0, 0));
                json!({ "ok": ok, "warn": warn, "error": error })
            })
            .collect::<Vec<_>>()
    };
    Ok(Json(json!({
        "name": project.name,
        "counts": { "users": users, "tables": tables, "files": files, "functions": functions, "sites": sites },
        "hours": hours,
        "series": {
            "auth": series("auth"),
            "database": series("database"),
            "function": series("function"),
            "site": series("site"),
        }
    })))
}

async fn functions(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "developer").await?;
    let rows: Vec<(String, i32, bool, bool, String)> = sqlx::query_as(
        "SELECT d.name, d.version, \
                (d.version = COALESCE( \
                    (SELECT p.version FROM reactor.function_pins p WHERE p.project_id = d.project_id AND p.name = d.name), \
                    (SELECT MAX(m.version) FROM reactor.deployments m WHERE m.project_id = d.project_id AND m.name = d.name) \
                )) AS live, \
                EXISTS ( \
                    SELECT 1 FROM reactor.function_pins p \
                    WHERE p.project_id = d.project_id AND p.name = d.name AND p.version = d.version \
                ) AS pinned, \
                d.created_at::text \
         FROM reactor.deployments d WHERE d.project_id = $1 ORDER BY d.name, d.version DESC",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(name, version, live, pinned, created)| {
            json!({"name": name, "version": version, "live": live, "pinned": pinned, "created_at": created})
        })
        .collect();
    Ok(Json(json!(items)))
}

#[derive(Deserialize)]
struct PinBody {
    version: i32,
}

#[derive(Deserialize)]
struct VersionQuery {
    version: Option<i32>,
}

async fn deploy_console_function(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, name)): Path<(String, String)>,
    Query(flag): Query<crate::PromoteFlag>,
    body: axum::body::Bytes,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    let version =
        crate::deploy_zip(&state, project.id, &project.pref, &name, body, flag.promote).await?;
    Ok(Json(
        json!({ "name": name, "version": version, "promoted": flag.promote }),
    ))
}

async fn promote_function(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, name)): Path<(String, String)>,
    Json(body): Json<PinBody>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    crate::pin_function(&state, project.id, &project.pref, &name, body.version).await?;
    Ok(Json(
        json!({ "name": name, "version": body.version, "pinned": true }),
    ))
}

async fn demote_function(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    crate::unpin_function(&state, project.id, &project.pref, &name).await?;
    Ok(Json(json!({ "name": name, "pinned": false })))
}

async fn invoke_console_function(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, name)): Path<(String, String)>,
    Query(query): Query<VersionQuery>,
    body: axum::body::Bytes,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "developer").await?;
    let resolved = crate::Resolved {
        project_id: project.id,
        pref: project.pref.clone(),
        database_url: None,
        identity: None,
    };
    let caller = json!({ "sub": operator.id, "ref": project.pref, "role": "console" });
    match crate::run_function(&state, &resolved, &name, &caller, &body, query.version).await {
        Ok(bytes) => {
            record_log(&state.pool, project.id, "function", &name, 200, "").await;
            Ok(Json(
                json!({ "status": 200, "body": String::from_utf8_lossy(&bytes) }),
            ))
        }
        Err(err) if err.status == StatusCode::NOT_FOUND => Err(err),
        Err(err) => {
            record_log(
                &state.pool,
                project.id,
                "function",
                &name,
                500,
                "invoke failed",
            )
            .await;
            Ok(Json(json!({ "status": 500, "body": err.text() })))
        }
    }
}

async fn function_stats(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "developer").await?;
    let hours: Vec<i64> = sqlx::query_scalar(
        "SELECT (extract(epoch FROM date_trunc('hour', now()) - make_interval(hours => n)))::bigint \
         FROM generate_series(23, 0, -1) AS n",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let rows: Vec<(i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT (extract(epoch FROM date_trunc('hour', created_at)))::bigint, \
                count(*) FILTER (WHERE status < 400), \
                count(*) FILTER (WHERE status >= 400 AND status < 500), \
                count(*) FILTER (WHERE status >= 500) \
         FROM reactor.logs \
         WHERE project_id = $1 AND kind = 'function' AND name = $2 \
           AND created_at >= date_trunc('hour', now()) - interval '23 hours' \
         GROUP BY 1",
    )
    .bind(project.id)
    .bind(&name)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let mut counts: HashMap<i64, (i64, i64, i64)> = HashMap::new();
    for (hour, ok, warn, error) in rows {
        counts.insert(hour, (ok, warn, error));
    }
    let points: Vec<Value> = hours
        .iter()
        .map(|hour| {
            let (ok, warn, error) = counts.get(hour).copied().unwrap_or((0, 0, 0));
            json!({ "ok": ok, "warn": warn, "error": error })
        })
        .collect();
    Ok(Json(json!({ "hours": hours, "points": points })))
}

async fn site_stats(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "developer").await?;
    let hours: Vec<i64> = sqlx::query_scalar(
        "SELECT (extract(epoch FROM date_trunc('hour', now()) - make_interval(hours => n)))::bigint \
         FROM generate_series(23, 0, -1) AS n",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let rows: Vec<(i64, i64, i64, i64)> = sqlx::query_as(
        "SELECT (extract(epoch FROM date_trunc('hour', created_at)))::bigint, \
                count(*) FILTER (WHERE status < 400), \
                count(*) FILTER (WHERE status >= 400 AND status < 500), \
                count(*) FILTER (WHERE status >= 500) \
         FROM reactor.logs \
         WHERE project_id = $1 AND kind = 'site' \
           AND created_at >= date_trunc('hour', now()) - interval '23 hours' \
         GROUP BY 1",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let mut counts: HashMap<i64, (i64, i64, i64)> = HashMap::new();
    for (hour, ok, warn, error) in rows {
        counts.insert(hour, (ok, warn, error));
    }
    let points: Vec<Value> = hours
        .iter()
        .map(|hour| {
            let (ok, warn, error) = counts.get(hour).copied().unwrap_or((0, 0, 0));
            json!({ "ok": ok, "warn": warn, "error": error })
        })
        .collect();
    Ok(Json(json!({ "hours": hours, "points": points })))
}

async fn sites(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "developer").await?;
    let files: Vec<String> = sqlx::query_scalar(
        "SELECT path FROM reactor.site_files WHERE project_id = $1 ORDER BY path",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let domains: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT host, verified_at::text FROM reactor.domains WHERE project_id = $1 ORDER BY host",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(json!({
        "files": files,
        "domains": domains.into_iter().map(|(host, verified)| json!({"host": host, "verified_at": verified})).collect::<Vec<_>>(),
        "url": site_public_url(&project.pref, &state.config.base_domain, &state.config.public_url),
    })))
}

async fn site_deployments(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "developer").await?;
    let rows: Vec<(Uuid, String, String, i32, String)> = sqlx::query_as(
        "SELECT id, status, error, file_count, created_at::text FROM reactor.site_deployments \
         WHERE project_id = $1 ORDER BY created_at DESC LIMIT 50",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(id, status, error, file_count, created)| {
            json!({"id": id, "status": status, "error": error, "file_count": file_count, "created_at": created})
        })
        .collect();
    Ok(Json(json!(items)))
}

async fn site_deployment(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, id)): Path<(String, Uuid)>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "developer").await?;
    let row: Option<(String, String, i32, String)> = sqlx::query_as(
        "SELECT status, error, file_count, created_at::text FROM reactor.site_deployments WHERE id = $1 AND project_id = $2",
    )
    .bind(id)
    .bind(project.id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some((status, error, file_count, created)) = row else {
        return Err(ApiError::not_found());
    };
    let files: Vec<String> = sqlx::query_scalar(
        "SELECT path FROM reactor.site_deployment_files WHERE deployment_id = $1 ORDER BY path",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(json!({
        "id": id,
        "status": status,
        "error": error,
        "file_count": file_count,
        "created_at": created,
        "files": files,
    })))
}

#[derive(Deserialize)]
struct EnvBody {
    key: String,
    value: String,
    #[serde(default)]
    visibility: String,
}

async fn list_site_env(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "developer").await?;
    let reveal = rank(&project.role) >= rank("admin");
    type EnvRow = (String, String, Vec<u8>, Vec<u8>, String);
    let rows: Vec<EnvRow> = sqlx::query_as(
        "SELECT key, visibility, nonce, ciphertext, updated_at::text FROM reactor.site_env WHERE project_id = $1 ORDER BY key",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let mut items = Vec::new();
    for (key, visibility, nonce, ciphertext, updated) in rows {
        let opened = if visibility == "visible" && reveal {
            Some(open_env(
                &state.config.storage_sign_secret,
                &nonce,
                &ciphertext,
            )?)
        } else {
            None
        };
        let mut item = site_env_public(&key, &visibility, reveal, opened.as_deref());
        item["updated_at"] = json!(updated);
        items.push(item);
    }
    Ok(Json(json!(items)))
}

async fn set_site_env(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
    Json(body): Json<EnvBody>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    if !acceptable_function_env_key(&body.key) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid key"));
    }
    if body.value.len() > 8192 || body.value.contains('\0') {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid value"));
    }
    let visibility = if body.visibility == "visible" {
        "visible"
    } else {
        "secret"
    };
    let (nonce, ciphertext) = seal_env(&state.config.storage_sign_secret, &body.value)?;
    sqlx::query(
        "INSERT INTO reactor.site_env (project_id, key, visibility, nonce, ciphertext) VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (project_id, key) DO UPDATE SET visibility = EXCLUDED.visibility, nonce = EXCLUDED.nonce, \
         ciphertext = EXCLUDED.ciphertext, updated_at = now()",
    )
    .bind(project.id)
    .bind(&body.key)
    .bind(visibility)
    .bind(&nonce)
    .bind(&ciphertext)
    .execute(&state.pool)
        .await
        .map_err(internal)?;
    state.sites.stop(project.id).await;
    Ok(Json(json!({ "key": body.key, "visibility": visibility })))
}

async fn delete_site_env(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, key)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    sqlx::query("DELETE FROM reactor.site_env WHERE project_id = $1 AND key = $2")
        .bind(project.id)
        .bind(&key)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    state.sites.stop(project.id).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn site_domains(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "developer").await?;
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT host, token, verified_at::text FROM reactor.domains WHERE project_id = $1 ORDER BY host",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(host, token, verified)| {
            let mut item = json!({
                "host": host,
                "verified_at": verified,
                "url": if verified.is_some() { format!("https://{host}") } else { String::new() },
            });
            if verified.is_none() {
                item["txt_name"] = json!(verification_name(&host));
                item["txt_value"] = json!(expected_txt(&token));
            }
            item
        })
        .collect();
    Ok(Json(json!(items)))
}

#[derive(Deserialize)]
struct DomainBody {
    host: String,
}

async fn add_site_domain(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
    Json(body): Json<DomainBody>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    let host = body.host.trim().trim_end_matches('.').to_ascii_lowercase();
    if !valid_host(&host) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid host"));
    }
    let platform = format!("{}.{}", project.pref, state.config.base_domain);
    if host == platform || host.ends_with(&format!(".{}", state.config.base_domain)) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "platform host"));
    }
    let token = random_token();
    let inserted =
        sqlx::query("INSERT INTO reactor.domains (host, project_id, token) VALUES ($1, $2, $3)")
            .bind(&host)
            .bind(project.id)
            .bind(&token)
            .execute(&state.pool)
            .await;
    if let Err(err) = inserted {
        if let sqlx::Error::Database(db) = &err {
            if db.code().as_deref() == Some("23505") {
                return Err(ApiError::new(StatusCode::CONFLICT, "domain already exists"));
            }
        }
        return Err(internal(err));
    }
    Ok(Json(json!({
        "host": host,
        "txt_name": verification_name(&host),
        "txt_value": expected_txt(&token),
    })))
}

async fn verify_site_domain(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, host)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    let token: Option<String> =
        sqlx::query_scalar("SELECT token FROM reactor.domains WHERE host = $1 AND project_id = $2")
            .bind(&host)
            .bind(project.id)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    let Some(token) = token else {
        return Err(ApiError::not_found());
    };
    let records = crate::lookup_txt(
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

async fn remove_site_domain(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, host)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    sqlx::query("DELETE FROM reactor.domains WHERE host = $1 AND project_id = $2")
        .bind(&host)
        .bind(project.id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct FunctionEnvBody {
    key: String,
    value: String,
}

async fn list_function_env(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "developer").await?;
    if !crate::valid_name(&name) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid function name",
        ));
    }
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, updated_at::text FROM reactor.function_env WHERE project_id = $1 AND name = $2 ORDER BY key",
    )
    .bind(project.id)
    .bind(&name)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(key, updated)| function_env_public(&key, &updated))
        .collect();
    Ok(Json(json!(items)))
}

async fn set_function_env(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, name)): Path<(String, String)>,
    Json(body): Json<FunctionEnvBody>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    if !crate::valid_name(&name) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid function name",
        ));
    }
    if !acceptable_function_env_key(&body.key) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid key"));
    }
    if body.value.len() > 8192 || body.value.contains('\0') {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid value"));
    }
    let (nonce, ciphertext) = seal_env(&state.config.storage_sign_secret, &body.value)?;
    sqlx::query(
        "INSERT INTO reactor.function_env (project_id, name, key, nonce, ciphertext) VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (project_id, name, key) DO UPDATE SET nonce = EXCLUDED.nonce, ciphertext = EXCLUDED.ciphertext, updated_at = now()",
    )
    .bind(project.id)
    .bind(&name)
    .bind(&body.key)
    .bind(&nonce)
    .bind(&ciphertext)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    crate::republish_function(&state, project.id, &project.pref, &name).await?;
    Ok(Json(json!({ "key": body.key })))
}

async fn delete_function_env(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, name, key)): Path<(String, String, String)>,
) -> Result<StatusCode, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    sqlx::query(
        "DELETE FROM reactor.function_env WHERE project_id = $1 AND name = $2 AND key = $3",
    )
    .bind(project.id)
    .bind(&name)
    .bind(&key)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    crate::republish_function(&state, project.id, &project.pref, &name).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn function_env_public(key: &str, updated: &str) -> Value {
    json!({"key": key, "updated_at": updated})
}

pub(crate) async fn load_function_env(
    state: &AppState,
    project_id: Uuid,
    name: &str,
) -> Result<Vec<(String, String)>, ApiError> {
    load_function_env_from(
        &state.pool,
        &state.config.storage_sign_secret,
        project_id,
        name,
    )
    .await
}

async fn load_function_env_from(
    pool: &sqlx::PgPool,
    secret: &str,
    project_id: Uuid,
    name: &str,
) -> Result<Vec<(String, String)>, ApiError> {
    let rows: Vec<(String, Vec<u8>, Vec<u8>)> = sqlx::query_as(
        "SELECT key, nonce, ciphertext FROM reactor.function_env WHERE project_id = $1 AND name = $2",
    )
    .bind(project_id)
    .bind(name)
    .fetch_all(pool)
    .await
    .map_err(internal)?;
    let mut out = Vec::new();
    for (key, nonce, ciphertext) in rows {
        out.push((key, open_env(secret, &nonce, &ciphertext)?));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

pub(crate) async fn load_site_env(
    state: &AppState,
    project_id: Uuid,
) -> Result<Vec<(String, String)>, ApiError> {
    load_site_env_from(&state.pool, &state.config.storage_sign_secret, project_id).await
}

pub(crate) async fn load_site_env_from(
    pool: &sqlx::PgPool,
    secret: &str,
    project_id: Uuid,
) -> Result<Vec<(String, String)>, ApiError> {
    let rows: Vec<(String, Vec<u8>, Vec<u8>)> = sqlx::query_as(
        "SELECT key, nonce, ciphertext FROM reactor.site_env WHERE project_id = $1 ORDER BY key",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await
    .map_err(internal)?;
    let mut out = Vec::new();
    for (key, nonce, ciphertext) in rows {
        out.push((key, open_env(secret, &nonce, &ciphertext)?));
    }
    Ok(out)
}

#[derive(serde::Deserialize)]
pub(crate) struct SiteEnvWrite {
    pub key: String,
    pub value: String,
    #[serde(default)]
    pub visibility: String,
}

pub(crate) async fn write_site_env(
    state: &AppState,
    project_id: Uuid,
    key: &str,
    value: &str,
    visibility: &str,
) -> Result<(), ApiError> {
    if !acceptable_function_env_key(key) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid key"));
    }
    if value.len() > 8192 || value.contains('\0') {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid value"));
    }
    let visibility = if visibility == "visible" {
        "visible"
    } else {
        "secret"
    };
    let (nonce, ciphertext) = seal_env(&state.config.storage_sign_secret, value)?;
    sqlx::query(
        "INSERT INTO reactor.site_env (project_id, key, visibility, nonce, ciphertext) VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (project_id, key) DO UPDATE SET visibility = EXCLUDED.visibility, nonce = EXCLUDED.nonce, \
         ciphertext = EXCLUDED.ciphertext, updated_at = now()",
    )
    .bind(project_id)
    .bind(key)
    .bind(visibility)
    .bind(&nonce)
    .bind(&ciphertext)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(())
}

pub(crate) fn site_env_public(
    key: &str,
    visibility: &str,
    reveal: bool,
    value: Option<&str>,
) -> Value {
    let mut item = json!({"key": key, "visibility": visibility});
    if visibility == "visible" && reveal {
        if let Some(value) = value {
            item["value"] = json!(value);
        }
    }
    item
}

fn env_key(secret: &str) -> [u8; 32] {
    let digest = Sha256::digest(secret.as_bytes());
    let mut key = [0u8; 32];
    key.copy_from_slice(&digest);
    key
}

fn seal_env(secret: &str, plain: &str) -> Result<(Vec<u8>, Vec<u8>), ApiError> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};
    let cipher = Aes256Gcm::new_from_slice(&env_key(secret)).map_err(internal)?;
    let mut nonce = [0u8; 12];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut nonce);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), plain.as_bytes())
        .map_err(internal)?;
    Ok((nonce.to_vec(), ciphertext))
}

fn open_env(secret: &str, nonce: &[u8], ciphertext: &[u8]) -> Result<String, ApiError> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};
    if nonce.len() != 12 {
        return Err(ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "stored value cannot be read",
        ));
    }
    let cipher = Aes256Gcm::new_from_slice(&env_key(secret)).map_err(internal)?;
    let plain = cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| {
            ApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "stored value cannot be read",
            )
        })?;
    String::from_utf8(plain).map_err(internal)
}

pub(crate) fn reserved_env_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    upper == "PATH" || upper == "HOME" || key.starts_with("REACTOR_")
}

fn acceptable_function_env_key(key: &str) -> bool {
    valid_env_key(key) && !reserved_env_key(key)
}

fn valid_env_key(key: &str) -> bool {
    let bytes = key.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && (bytes[0].is_ascii_alphabetic() || bytes[0] == b'_')
        && bytes
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b'_')
}

fn valid_host(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && host.contains('.')
        && !host.starts_with('.')
        && !host.ends_with('.')
        && !host.contains("..")
        && host
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

pub(crate) fn site_public_url(pref: &str, base_domain: &str, public_url: &str) -> String {
    let https = public_url.starts_with("https://");
    let scheme = if https { "https" } else { "http" };
    let hostport = public_url
        .split("://")
        .nth(1)
        .unwrap_or(public_url)
        .split('/')
        .next()
        .unwrap_or("");
    let port = hostport
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse::<u16>().ok());
    match port {
        Some(port) if !((https && port == 443) || (!https && port == 80)) => {
            format!("{scheme}://{pref}.{base_domain}:{port}")
        }
        _ => format!("{scheme}://{pref}.{base_domain}"),
    }
}

async fn logs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "developer").await?;
    let q = query.q.unwrap_or_default();
    let kind = query.kind.unwrap_or_default();
    let name = query.name.unwrap_or_default();
    let rows: Vec<(i64, String, String, i32, String, String)> = sqlx::query_as(
        "SELECT id, kind, name, status, message, created_at::text FROM reactor.logs \
         WHERE project_id = $1 AND ($2 = '' OR kind ILIKE '%' || $2 || '%' OR name ILIKE '%' || $2 || '%' \
           OR message ILIKE '%' || $2 || '%' OR status::text ILIKE '%' || $2 || '%' OR id::text = $2) \
           AND ($3 = '' OR kind = $3) AND ($4 = '' OR name = $4) \
         ORDER BY id DESC LIMIT 100",
    )
    .bind(project.id)
    .bind(&q)
    .bind(&kind)
    .bind(&name)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(id, kind, name, status, message, created)| {
            json!({"id": id, "kind": kind, "name": name, "status": status, "message": message, "created_at": created})
        })
        .collect();
    Ok(Json(json!(items)))
}

async fn spa_index(State(state): State<AppState>) -> Response {
    spa_bytes(&state, "index.html")
}

async fn spa_file(State(state): State<AppState>, Path(path): Path<String>) -> Response {
    if path.starts_with("v1") || path.contains("..") {
        return StatusCode::NOT_FOUND.into_response();
    }
    let root = std::path::PathBuf::from(&state.config.console_dir);
    let file = root.join(&path);
    if file.is_file() {
        return file_response(&file);
    }
    spa_bytes(&state, "index.html")
}

fn spa_bytes(state: &AppState, name: &str) -> Response {
    let path = std::path::PathBuf::from(&state.config.console_dir).join(name);
    if path.is_file() {
        file_response(&path)
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

fn file_response(path: &std::path::Path) -> Response {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(_) => return StatusCode::NOT_FOUND.into_response(),
    };
    let kind = match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    };
    ([(header::CONTENT_TYPE, kind)], bytes).into_response()
}

#[cfg(test)]
mod tests {
    use super::{
        acceptable_function_env_key, console_key_scope, function_env_public,
        load_function_env_from, migration_issue, open_env, owned_schema, seal_env, site_env_public,
        site_public_url,
    };
    use uuid::Uuid;

    const REF: &str = "abcdefghijklmnopqrst";

    #[test]
    fn project_schema_stays_on_that_project() {
        let pref = "abcdefghijklmnopqrst";
        assert!(owned_schema(pref, "proj_abcdefghijklmnopqrst"));
        assert!(owned_schema(pref, "proj_abcdefghijklmnopqrst_billing"));
        assert!(!owned_schema(pref, "proj_zzzzzzzzzzzzzzzzzzzz"));
        assert!(!owned_schema(pref, "public"));
        assert!(!owned_schema(pref, "reactor"));
        assert!(!owned_schema(pref, "reactor_api"));
        assert!(!owned_schema(pref, "proj_abcdefghijklmnopqrst;drop"));
    }

    #[test]
    fn console_key_scope_maps_the_allowlist() {
        assert_eq!(
            console_key_scope("POST", "/console/v1/projects"),
            Some("projects.create")
        );
        assert_eq!(
            console_key_scope("POST", &format!("/console/v1/projects/{REF}")),
            None
        );
        assert_eq!(
            console_key_scope("GET", &format!("/console/v1/projects/{REF}/auth")),
            Some("auth.settings")
        );
        assert_eq!(
            console_key_scope("PUT", &format!("/console/v1/projects/{REF}/auth")),
            Some("auth.settings")
        );
        assert_eq!(
            console_key_scope("GET", &format!("/console/v1/projects/{REF}/auth/providers")),
            Some("auth.providers")
        );
        assert_eq!(
            console_key_scope(
                "POST",
                &format!("/console/v1/projects/{REF}/auth/providers")
            ),
            Some("auth.providers")
        );
        assert_eq!(
            console_key_scope(
                "DELETE",
                &format!("/console/v1/projects/{REF}/auth/providers/github")
            ),
            Some("auth.providers")
        );
        assert_eq!(
            console_key_scope("GET", &format!("/console/v1/projects/{REF}/email")),
            Some("auth.email")
        );
        assert_eq!(
            console_key_scope("PUT", &format!("/console/v1/projects/{REF}/email")),
            Some("auth.email")
        );
        assert_eq!(
            console_key_scope("POST", &format!("/console/v1/projects/{REF}/email/test")),
            Some("auth.email")
        );
        assert_eq!(
            console_key_scope(
                "GET",
                &format!("/console/v1/projects/{REF}/email/templates")
            ),
            Some("auth.email")
        );
        assert_eq!(
            console_key_scope(
                "GET",
                &format!("/console/v1/projects/{REF}/email/templates/invite")
            ),
            Some("auth.email")
        );
        assert_eq!(
            console_key_scope(
                "PUT",
                &format!("/console/v1/projects/{REF}/email/templates/invite")
            ),
            Some("auth.email")
        );
        assert_eq!(
            console_key_scope(
                "POST",
                &format!("/console/v1/projects/{REF}/email/templates/invite/reset")
            ),
            Some("auth.email")
        );
        assert_eq!(
            console_key_scope("GET", &format!("/console/v1/projects/{REF}/users")),
            Some("auth.users")
        );
        let user = "6b1c1a4e-2f0a-4d3b-9c11-0a9e8d7c6b5a";
        assert_eq!(
            console_key_scope("GET", &format!("/console/v1/projects/{REF}/users/{user}")),
            Some("auth.users")
        );
        assert_eq!(
            console_key_scope(
                "DELETE",
                &format!("/console/v1/projects/{REF}/users/{user}")
            ),
            Some("auth.users")
        );
        assert_eq!(
            console_key_scope(
                "POST",
                &format!("/console/v1/projects/{REF}/users/{user}/password")
            ),
            Some("auth.users")
        );
        assert_eq!(
            console_key_scope("POST", &format!("/console/v1/projects/{REF}/migrations")),
            Some("projects.migrate")
        );
        assert_eq!(
            console_key_scope("GET", &format!("/console/v1/projects/{REF}/migrations")),
            None
        );
        for (method, path) in [
            ("POST", format!("/console/v1/projects/{REF}/email/cluster")),
            ("DELETE", format!("/console/v1/projects/{REF}")),
            ("POST", format!("/console/v1/projects/{REF}/members")),
            ("POST", format!("/console/v1/projects/{REF}/keys")),
            ("POST", "/console/v1/keys".to_string()),
            ("GET", "/console/v1/me".to_string()),
            ("GET", format!("/console/v1/projects/SHORT/auth")),
            (
                "PUT",
                format!("/console/v1/projects/ABCDEFGHIJ1234567890/auth"),
            ),
        ] {
            assert_eq!(console_key_scope(method, &path), None, "{method} {path}");
        }
    }

    #[test]
    fn migration_sql_stays_inside_the_project() {
        assert!(migration_issue(
            "0001_app.sql",
            "CREATE TABLE \"cafe\" (id uuid PRIMARY KEY); GRANT SELECT ON \"cafe\" TO authenticated;"
        )
        .is_none());
        assert_eq!(
            migration_issue("app.sql", "CREATE TABLE t (id int);"),
            Some("invalid version")
        );
        assert_eq!(
            migration_issue("0001_app.sql", "DROP TABLE cafe;"),
            Some("sql is not allowed")
        );
        assert_eq!(
            migration_issue("0001_app.sql", "CREATE TABLE reactor.notes (id int);"),
            Some("sql is not allowed")
        );
        assert!(migration_issue(
            "0001_app.sql",
            "CREATE FUNCTION sw() RETURNS void LANGUAGE plpgsql AS $sw$ BEGIN RETURN; END; $sw$; GRANT EXECUTE ON FUNCTION sw() TO authenticated;"
        )
        .is_none());
        assert!(migration_issue(
            "0001_app.sql",
            "CREATE TABLE sw_settings (id int); INSERT INTO sw_settings (id) VALUES (1); REVOKE ALL ON FUNCTION sw() FROM PUBLIC;"
        )
        .is_none());
        assert_eq!(
            migration_issue("0001_app.sql", "DELETE FROM sw_settings;"),
            Some("sql is not allowed")
        );
    }

    #[test]
    fn site_url_uses_https_without_the_default_port() {
        assert_eq!(
            site_public_url("abc", "apps.example", "https://apps.example"),
            "https://abc.apps.example"
        );
        assert_eq!(
            site_public_url("abc", "apps.localhost", "http://127.0.0.1:18000"),
            "http://abc.apps.localhost:18000"
        );
    }

    #[test]
    fn site_env_list_omits_secrets() {
        let secret = site_env_public("API_SECRET", "secret", true, Some("sekret"));
        assert!(secret.get("value").is_none());
        let visible = site_env_public("TOKEN", "visible", true, Some("alpha"));
        assert_eq!(visible["value"], "alpha");
        let hidden = site_env_public("TOKEN", "visible", false, Some("alpha"));
        assert!(hidden.get("value").is_none());
    }

    #[tokio::test]
    async fn site_env_round_trip_returns_the_sealed_value() {
        let Ok(pool) = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_secs(2))
            .connect("postgres://reactor:reactor@127.0.0.1:5440/reactor")
            .await
        else {
            return;
        };
        let project_id: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM reactor.projects LIMIT 1")
                .fetch_optional(&pool)
                .await
                .ok()
                .flatten();
        let Some(project_id) = project_id else {
            return;
        };
        let secret = "site-env-test";
        let (nonce, ciphertext) =
            seal_env(secret, "alpha-value").unwrap_or_else(|err| panic!("{}", err.text()));
        let key = format!("ZZ_{}", Uuid::new_v4().simple());
        sqlx::query(
            "INSERT INTO reactor.site_env (project_id, key, visibility, nonce, ciphertext) VALUES ($1, $2, 'secret', $3, $4)",
        )
        .bind(project_id)
        .bind(&key)
        .bind(&nonce)
        .bind(&ciphertext)
        .execute(&pool)
        .await
        .unwrap();
        let (nonce, ciphertext): (Vec<u8>, Vec<u8>) = sqlx::query_as(
            "SELECT nonce, ciphertext FROM reactor.site_env WHERE project_id = $1 AND key = $2",
        )
        .bind(project_id)
        .bind(&key)
        .fetch_one(&pool)
        .await
        .unwrap();
        let value =
            open_env(secret, &nonce, &ciphertext).unwrap_or_else(|err| panic!("{}", err.text()));
        assert_eq!(value, "alpha-value");
        sqlx::query("DELETE FROM reactor.site_env WHERE project_id = $1 AND key = $2")
            .bind(project_id)
            .bind(&key)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[test]
    fn function_env_list_omits_the_value() {
        let item = function_env_public("TOKEN", "2026-09-27");
        assert!(item.get("value").is_none());
        assert_eq!(item["key"], "TOKEN");
    }

    #[test]
    fn function_env_rejects_reserved_keys() {
        assert!(acceptable_function_env_key("TOKEN"));
        assert!(!acceptable_function_env_key("PATH"));
        assert!(!acceptable_function_env_key("HOME"));
        assert!(!acceptable_function_env_key("REACTOR_CALLER"));
        assert!(!acceptable_function_env_key("1TOKEN"));
    }

    #[test]
    fn function_env_ciphertext_does_not_contain_the_value() {
        let (nonce, ciphertext) =
            seal_env("sign-secret", "alpha-value").unwrap_or_else(|err| panic!("{}", err.text()));
        assert_eq!(
            open_env("sign-secret", &nonce, &ciphertext)
                .unwrap_or_else(|err| panic!("{}", err.text())),
            "alpha-value"
        );
        assert!(!ciphertext
            .windows(b"alpha-value".len())
            .any(|window| window == b"alpha-value"));
        assert!(open_env("other-secret", &nonce, &ciphertext).is_err());
    }

    #[tokio::test]
    async fn function_env_is_isolated_per_function() {
        let Ok(pool) = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_secs(2))
            .connect("postgres://reactor:reactor@127.0.0.1:5440/reactor")
            .await
        else {
            return;
        };
        let project_id: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM reactor.projects LIMIT 1")
                .fetch_optional(&pool)
                .await
                .ok()
                .flatten();
        let Some(project_id) = project_id else {
            return;
        };
        sqlx::query(include_str!("../../../sql/control/004_function_env.sql"))
            .execute(&pool)
            .await
            .unwrap();
        let left = format!("zz_env_{}", Uuid::new_v4().simple());
        let right = format!("zz_env_{}", Uuid::new_v4().simple());
        let secret = "function-env-test";
        let insert = |name: String, value: String| {
            let pool = pool.clone();
            let left_or_right = name;
            async move {
                let (nonce, ciphertext) =
                    seal_env(secret, &value).unwrap_or_else(|err| panic!("{}", err.text()));
                sqlx::query(
                    "INSERT INTO reactor.function_env (project_id, name, key, nonce, ciphertext) VALUES ($1, $2, 'TOKEN', $3, $4)",
                )
                .bind(project_id)
                .bind(left_or_right)
                .bind(nonce)
                .bind(ciphertext)
                .execute(&pool)
                .await
                .unwrap();
            }
        };
        insert(left.clone(), "alpha".into()).await;
        insert(right.clone(), "beta".into()).await;
        let loaded_left = load_function_env_from(&pool, secret, project_id, &left)
            .await
            .unwrap_or_else(|err| panic!("{}", err.text()));
        let loaded_right = load_function_env_from(&pool, secret, project_id, &right)
            .await
            .unwrap_or_else(|err| panic!("{}", err.text()));
        let loaded_other = load_function_env_from(&pool, secret, project_id, "zz_env_missing")
            .await
            .unwrap_or_else(|err| panic!("{}", err.text()));
        sqlx::query("DELETE FROM reactor.function_env WHERE project_id = $1 AND name = ANY($2)")
            .bind(project_id)
            .bind(vec![left, right])
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(loaded_left, vec![("TOKEN".into(), "alpha".into())]);
        assert_eq!(loaded_right, vec![("TOKEN".into(), "beta".into())]);
        assert!(loaded_other.is_empty());
    }
}
