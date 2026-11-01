use crate::agent::{log_line, redact};
use crate::console::{
    ident, issue_keys, load_operator, record_log, require_member, schema_name, user_object,
    write_site_env, Operator,
};
use crate::oauth_as::{challenge_header, principal};
use crate::{generate_ref, reload, ApiError, AppState};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use reactor_auth::hash_password;
use reactor_core::OpenedProject;
use reactor_functions::zip_single;
use reactor_sites::{content_type_for, safe_site_path};
use reactor_storage::object_key;
use serde_json::{json, Value};
use uuid::Uuid;

const FAVICON_PNG: &[u8] = include_bytes!("../assets/favicon.png");
const FAVICON_ICO: &[u8] = include_bytes!("../assets/favicon.ico");
const FAVICON_SVG: &[u8] = include_bytes!("../assets/favicon.svg");

pub fn mount(app: Router<crate::AppCtx>) -> Router<crate::AppCtx> {
    app.route("/mcp", post(mcp).get(mcp_get))
        .route(
            "/favicon.png",
            get(|| async { icon(FAVICON_PNG, "image/png") }),
        )
        .route(
            "/favicon.ico",
            get(|| async { icon(FAVICON_ICO, "image/x-icon") }),
        )
        .route(
            "/favicon.svg",
            get(|| async { icon(FAVICON_SVG, "image/svg+xml") }),
        )
}

fn icon(bytes: &'static [u8], content_type: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "public, max-age=86400"),
        ],
        bytes,
    )
        .into_response()
}

async fn mcp_get(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if principal(&state, &headers).await.is_err() {
        return unauthorized(&state);
    }
    StatusCode::METHOD_NOT_ALLOWED.into_response()
}

async fn mcp(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if principal(&state, &headers).await.is_err() {
        return unauthorized(&state);
    }
    let id = body.get("id").cloned();
    let method = body.get("method").and_then(|v| v.as_str()).unwrap_or("");
    if id.is_none() {
        return StatusCode::ACCEPTED.into_response();
    }
    let result = match method {
        "initialize" => Ok(initialize(&body, &state.config.public_url)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tool_list()})),
        "tools/call" => {
            let (operator_id, scope) = match principal(&state, &headers).await {
                Ok(pair) => pair,
                Err(err) => return tool_http_error(id, err),
            };
            let operator = match load_operator(&state, operator_id).await {
                Ok(operator) => operator,
                Err(err) => return tool_http_error(id, err),
            };
            let params = body.get("params").cloned().unwrap_or(json!({}));
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            match call_tool(&state, &operator, &scope, name, &args).await {
                Ok(value) => Ok(tool_content(value, false)),
                Err(err) => Ok(tool_content(json!({"error": err.text()}), true)),
            }
        }
        _ => Err("method not found"),
    };
    match result {
        Ok(value) => Json(json!({"jsonrpc": "2.0", "id": id, "result": value})).into_response(),
        Err(message) => Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32601, "message": message}
        }))
        .into_response(),
    }
}

fn unauthorized(state: &AppState) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(
            header::WWW_AUTHENTICATE,
            challenge_header(&state.config.public_url),
        )],
        Json(json!({"error": "bearer token required"})),
    )
        .into_response()
}

fn tool_http_error(id: Option<Value>, err: ApiError) -> Response {
    (
        err.status(),
        Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32001, "message": err.text()}
        })),
    )
        .into_response()
}

fn initialize(body: &Value, public_url: &str) -> Value {
    let requested = body
        .pointer("/params/protocolVersion")
        .and_then(|v| v.as_str())
        .unwrap_or("2025-06-18");
    let version = match requested {
        "2024-11-05" | "2025-03-26" | "2025-06-18" | "2025-11-25" | "2026-07-28" => requested,
        _ => "2025-06-18",
    };
    let base = public_url.trim_end_matches('/');
    json!({
        "protocolVersion": version,
        "capabilities": {"tools": {"listChanged": false}},
        "serverInfo": {
            "name": "reactor",
            "version": "1",
            "icons": [
                {
                    "src": format!("{base}/favicon.png"),
                    "mimeType": "image/png",
                    "sizes": ["128x128"]
                },
                {
                    "src": format!("{base}/favicon.svg"),
                    "mimeType": "image/svg+xml",
                    "sizes": ["any"]
                }
            ]
        }
    })
}

fn tool_content(value: Value, is_error: bool) -> Value {
    json!({
        "content": [{"type": "text", "text": value.to_string()}],
        "isError": is_error
    })
}

struct Spec {
    name: &'static str,
    description: &'static str,
    write: bool,
    props: &'static [&'static str],
    required: &'static [&'static str],
}

fn specs() -> &'static [Spec] {
    &[
        Spec { name: "whoami", description: "The signed-in operator and scope. No tokens.", write: false, props: &[], required: &[] },
        Spec { name: "list_projects", description: "Projects this operator can open.", write: false, props: &[], required: &[] },
        Spec { name: "get_project", description: "One project this operator can open.", write: false, props: &["ref"], required: &["ref"] },
        Spec { name: "list_members", description: "Operators on a project.", write: false, props: &["ref"], required: &["ref"] },
        Spec { name: "logs", description: "Recent project logs.", write: false, props: &["ref"], required: &["ref"] },
        Spec { name: "schema", description: "Tables and columns.", write: false, props: &["ref"], required: &["ref"] },
        Spec { name: "list_rows", description: "Up to 50 rows.", write: false, props: &["ref", "table"], required: &["ref", "table"] },
        Spec { name: "execute_sql", description: "Run SQL. Read-only unless write is true. Writes need confirm when the statement warns.", write: false, props: &["ref", "sql", "write", "confirm"], required: &["ref", "sql"] },
        Spec { name: "list_functions", description: "Function deployments.", write: false, props: &["ref"], required: &["ref"] },
        Spec { name: "function_stats", description: "Function log counts.", write: false, props: &["ref", "name"], required: &["ref", "name"] },
        Spec { name: "list_sites", description: "Published site files.", write: false, props: &["ref"], required: &["ref"] },
        Spec { name: "list_site_env", description: "Site variable names. Secret values are never returned.", write: false, props: &["ref"], required: &["ref"] },
        Spec { name: "list_site_deployments", description: "Recent site deployments.", write: false, props: &["ref"], required: &["ref"] },
        Spec { name: "list_buckets", description: "Storage buckets.", write: false, props: &["ref"], required: &["ref"] },
        Spec { name: "list_objects", description: "Stored files.", write: false, props: &["ref"], required: &["ref"] },
        Spec { name: "list_queues", description: "Queues.", write: false, props: &["ref"], required: &["ref"] },
        Spec { name: "peek_queue", description: "Peek a queue.", write: false, props: &["ref", "name"], required: &["ref", "name"] },
        Spec { name: "cluster", description: "Cluster name and database liveness. Platform admin.", write: false, props: &[], required: &[] },
        Spec { name: "create_project", description: "Create a project. Returns ref and anon. The service key stays on the cluster.", write: true, props: &["name"], required: &["name"] },
        Spec { name: "add_member", description: "Add an operator to a project.", write: true, props: &["ref", "email", "name", "role", "password"], required: &["ref", "email", "name", "password"] },
        Spec { name: "apply_migration", description: "Apply a project migration.", write: true, props: &["ref", "version", "sql"], required: &["ref", "version", "sql"] },
        Spec { name: "deploy_function", description: "Deploy function source as index.ts.", write: true, props: &["ref", "name", "source"], required: &["ref", "name", "source"] },
        Spec { name: "promote_function", description: "Pin a function version.", write: true, props: &["ref", "name", "version"], required: &["ref", "name", "version"] },
        Spec { name: "demote_function", description: "Clear a function pin.", write: true, props: &["ref", "name"], required: &["ref", "name"] },
        Spec { name: "set_function_env", description: "Set a function variable. The value is not returned later.", write: true, props: &["ref", "name", "key", "value"], required: &["ref", "name", "key", "value"] },
        Spec { name: "unset_function_env", description: "Remove a function variable.", write: true, props: &["ref", "name", "key"], required: &["ref", "name", "key"] },
        Spec { name: "put_site_file", description: "Publish a site file.", write: true, props: &["ref", "path", "content"], required: &["ref", "path", "content"] },
        Spec { name: "delete_site_file", description: "Remove a site file.", write: true, props: &["ref", "path"], required: &["ref", "path"] },
        Spec { name: "set_site_env", description: "Set a site variable. Secret unless visible is true.", write: true, props: &["ref", "key", "value", "visible"], required: &["ref", "key", "value"] },
        Spec { name: "unset_site_env", description: "Remove a site variable.", write: true, props: &["ref", "key"], required: &["ref", "key"] },
        Spec { name: "create_bucket", description: "Create a storage bucket.", write: true, props: &["ref", "name", "public"], required: &["ref", "name"] },
        Spec { name: "put_object", description: "Store a text object.", write: true, props: &["ref", "bucket", "path", "content"], required: &["ref", "bucket", "path", "content"] },
        Spec { name: "delete_object", description: "Delete a stored object.", write: true, props: &["ref", "bucket", "path"], required: &["ref", "bucket", "path"] },
        Spec { name: "create_queue", description: "Create a queue.", write: true, props: &["ref", "name"], required: &["ref", "name"] },
        Spec { name: "send_queue", description: "Send a queue message.", write: true, props: &["ref", "name", "message"], required: &["ref", "name", "message"] },
        Spec { name: "create_user", description: "Add a project user.", write: true, props: &["ref", "email", "password"], required: &["ref", "email", "password"] },
        Spec { name: "rotate_project_keys", description: "Rotate project keys. The new secrets are not returned.", write: true, props: &["ref"], required: &["ref"] },
        Spec { name: "migrate_all", description: "Apply pending migrations on every project. Platform admin.", write: true, props: &[], required: &[] },
    ]
}

fn tool_list() -> Vec<Value> {
    specs()
        .iter()
        .map(|spec| {
            let mut properties = serde_json::Map::new();
            for prop in spec.props {
                let kind = if *prop == "write" || *prop == "confirm" || *prop == "visible" || *prop == "public" {
                    "boolean"
                } else if *prop == "version" && spec.name == "promote_function" {
                    "integer"
                } else if *prop == "message" {
                    "object"
                } else {
                    "string"
                };
                properties.insert((*prop).into(), json!({"type": kind}));
            }
            json!({
                "name": spec.name,
                "description": spec.description,
                "inputSchema": {"type": "object", "properties": properties, "required": spec.required},
                "annotations": {
                    "readOnlyHint": !spec.write && spec.name != "execute_sql",
                    "destructiveHint": spec.write || spec.name == "execute_sql"
                }
            })
        })
        .collect()
}

fn mutates(name: &str, args: &Value) -> bool {
    if name == "execute_sql" {
        return args.get("write").and_then(|v| v.as_bool()).unwrap_or(false);
    }
    specs().iter().any(|spec| spec.name == name && spec.write)
}

async fn call_tool(
    state: &AppState,
    operator: &Operator,
    scope: &str,
    name: &str,
    args: &Value,
) -> Result<Value, ApiError> {
    if !specs().iter().any(|spec| spec.name == name) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "unknown tool"));
    }
    if scope != "operator" && mutates(name, args) {
        return Err(ApiError::forbidden(
            "operator:read cannot change the cluster",
        ));
    }
    let value = dispatch(state, operator, name, args).await?;
    if let Some(pref) = args.get("ref").and_then(|v| v.as_str()) {
        if let Ok(project) = require_member(state, operator, pref, "developer").await {
            let mut copy = args.clone();
            redact(&mut copy);
            record_log(&state.pool, project.id, "mcp", name, 200, &log_line(&copy)).await;
        }
    }
    let mut out = value;
    redact(&mut out);
    Ok(out)
}

async fn dispatch(
    state: &AppState,
    operator: &Operator,
    name: &str,
    args: &Value,
) -> Result<Value, ApiError> {
    match name {
        "whoami" => Ok(json!({
            "id": operator.id,
            "email": operator.email,
            "name": operator.name,
            "platform_admin": operator.platform_admin
        })),
        "list_projects" => list_projects(state, operator).await,
        "get_project" => get_project(state, operator, text(args, "ref")?).await,
        "list_members" => list_members(state, operator, text(args, "ref")?).await,
        "logs" => logs(state, operator, text(args, "ref")?).await,
        "schema" => schema(state, operator, text(args, "ref")?).await,
        "list_rows" => list_rows(state, operator, text(args, "ref")?, text(args, "table")?).await,
        "execute_sql" => {
            execute_sql(
                state,
                operator,
                text(args, "ref")?,
                text(args, "sql")?,
                args.get("write").and_then(|v| v.as_bool()).unwrap_or(false),
                args.get("confirm")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            )
            .await
        }
        "list_functions" => list_functions(state, operator, text(args, "ref")?).await,
        "function_stats" => {
            function_stats(state, operator, text(args, "ref")?, text(args, "name")?).await
        }
        "list_sites" => list_sites(state, operator, text(args, "ref")?).await,
        "list_site_env" => list_site_env(state, operator, text(args, "ref")?).await,
        "list_site_deployments" => list_site_deployments(state, operator, text(args, "ref")?).await,
        "list_buckets" => list_buckets(state, operator, text(args, "ref")?).await,
        "list_objects" => list_objects(state, operator, text(args, "ref")?).await,
        "list_queues" => list_queues(state, operator, text(args, "ref")?).await,
        "peek_queue" => peek_queue(state, operator, text(args, "ref")?, text(args, "name")?).await,
        "cluster" => cluster(state, operator).await,
        "create_project" => create_project(state, operator, text(args, "name")?).await,
        "add_member" => {
            add_member(
                state,
                operator,
                text(args, "ref")?,
                text(args, "email")?,
                text(args, "name")?,
                args.get("role")
                    .and_then(|v| v.as_str())
                    .unwrap_or("developer"),
                text(args, "password")?,
            )
            .await
        }
        "apply_migration" => {
            apply_migration(
                state,
                operator,
                text(args, "ref")?,
                text(args, "version")?,
                text(args, "sql")?,
            )
            .await
        }
        "deploy_function" => {
            deploy_function(
                state,
                operator,
                text(args, "ref")?,
                text(args, "name")?,
                text(args, "source")?,
            )
            .await
        }
        "promote_function" => {
            let version = args
                .get("version")
                .and_then(|v| v.as_i64())
                .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "version is required"))?;
            promote_function(
                state,
                operator,
                text(args, "ref")?,
                text(args, "name")?,
                version as i32,
            )
            .await
        }
        "demote_function" => {
            demote_function(state, operator, text(args, "ref")?, text(args, "name")?).await
        }
        "set_function_env" => {
            set_function_env(
                state,
                operator,
                text(args, "ref")?,
                text(args, "name")?,
                text(args, "key")?,
                text(args, "value")?,
            )
            .await
        }
        "unset_function_env" => {
            unset_function_env(
                state,
                operator,
                text(args, "ref")?,
                text(args, "name")?,
                text(args, "key")?,
            )
            .await
        }
        "put_site_file" => {
            put_site_file(
                state,
                operator,
                text(args, "ref")?,
                text(args, "path")?,
                text(args, "content")?,
            )
            .await
        }
        "delete_site_file" => {
            delete_site_file(state, operator, text(args, "ref")?, text(args, "path")?).await
        }
        "set_site_env" => {
            set_site_env(
                state,
                operator,
                text(args, "ref")?,
                text(args, "key")?,
                text(args, "value")?,
                args.get("visible")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            )
            .await
        }
        "unset_site_env" => {
            unset_site_env(state, operator, text(args, "ref")?, text(args, "key")?).await
        }
        "create_bucket" => {
            create_bucket(
                state,
                operator,
                text(args, "ref")?,
                text(args, "name")?,
                args.get("public")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            )
            .await
        }
        "put_object" => {
            put_object(
                state,
                operator,
                text(args, "ref")?,
                text(args, "bucket")?,
                text(args, "path")?,
                text(args, "content")?,
            )
            .await
        }
        "delete_object" => {
            delete_object(
                state,
                operator,
                text(args, "ref")?,
                text(args, "bucket")?,
                text(args, "path")?,
            )
            .await
        }
        "create_queue" => {
            create_queue(state, operator, text(args, "ref")?, text(args, "name")?).await
        }
        "send_queue" => {
            let message = args.get("message").cloned().unwrap_or(Value::Null);
            send_queue(
                state,
                operator,
                text(args, "ref")?,
                text(args, "name")?,
                message,
            )
            .await
        }
        "create_user" => {
            create_user(
                state,
                operator,
                text(args, "ref")?,
                text(args, "email")?,
                text(args, "password")?,
            )
            .await
        }
        "rotate_project_keys" => rotate_project_keys(state, operator, text(args, "ref")?).await,
        "migrate_all" => migrate_all(state, operator).await,
        _ => Err(ApiError::new(StatusCode::BAD_REQUEST, "unknown tool")),
    }
}

fn text<'a>(args: &'a Value, key: &str) -> Result<&'a str, ApiError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, format!("{key} is required")))
}

fn bounded(value: &str, limit: usize, label: &str) -> Result<(), ApiError> {
    if value.len() > limit {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            format!("{label} is too large"),
        ));
    }
    Ok(())
}

async fn list_projects(state: &AppState, operator: &Operator) -> Result<Value, ApiError> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT p.ref, p.name, m.role FROM reactor.projects p \
         JOIN reactor.memberships m ON m.project_id = p.id \
         WHERE m.operator_id = $1 ORDER BY p.name, p.ref",
    )
    .bind(operator.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(json!(rows
        .into_iter()
        .map(|(pref, name, role)| json!({"ref": pref, "name": name, "role": role}))
        .collect::<Vec<_>>()))
}

async fn get_project(state: &AppState, operator: &Operator, pref: &str) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "developer").await?;
    Ok(json!({"ref": project.pref, "name": project.name, "role": project.role}))
}

async fn list_members(
    state: &AppState,
    operator: &Operator,
    pref: &str,
) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "developer").await?;
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT o.email, o.name, m.role FROM reactor.memberships m \
         JOIN reactor.operators o ON o.id = m.operator_id \
         WHERE m.project_id = $1 ORDER BY o.email",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(json!(rows
        .into_iter()
        .map(|(email, name, role)| json!({"email": email, "name": name, "role": role}))
        .collect::<Vec<_>>()))
}

async fn logs(state: &AppState, operator: &Operator, pref: &str) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "developer").await?;
    let rows: Vec<(i64, String, String, i32, String)> = sqlx::query_as(
        "SELECT id, kind, name, status, created_at::text FROM reactor.logs \
         WHERE project_id = $1 ORDER BY id DESC LIMIT 100",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(json!(rows
        .into_iter()
        .map(|(id, kind, name, status, created)| {
            json!({"id": id, "kind": kind, "name": name, "status": status, "created_at": created})
        })
        .collect::<Vec<_>>()))
}

async fn schema(state: &AppState, operator: &Operator, pref: &str) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "developer").await?;
    let pool = project_pool(state, project.id).await?;
    let schema = schema_name(&project.pref)?;
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
        let columns: Vec<(String, String)> = sqlx::query_as(
            "SELECT column_name, data_type FROM information_schema.columns \
             WHERE table_schema = $1 AND table_name = $2 ORDER BY ordinal_position",
        )
        .bind(&schema)
        .bind(&table)
        .fetch_all(&pool)
        .await
        .map_err(internal)?;
        out.push(json!({
            "name": table,
            "columns": columns.into_iter().map(|(n, t)| json!({"name": n, "type": t})).collect::<Vec<_>>()
        }));
    }
    Ok(json!(out))
}

async fn list_rows(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    table: &str,
) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "developer").await?;
    if !ident(table) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid table"));
    }
    let schema = schema_name(&project.pref)?;
    let pool = project_pool(state, project.id).await?;
    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT to_jsonb(t) FROM \"{schema}\".\"{table}\" t LIMIT 50"
    ))
    .fetch_all(&pool)
    .await
    .map_err(internal)?;
    Ok(json!(rows))
}

async fn execute_sql(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    sql: &str,
    write: bool,
    confirm: bool,
) -> Result<Value, ApiError> {
    bounded(sql, 200_000, "sql")?;
    let project = require_member(state, operator, pref, "admin").await?;
    let warnings = crate::sql::classify(sql)?;
    if !warnings.is_empty() && !confirm {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            format!("confirmation required: {}", warnings.join(", ")),
        ));
    }
    let database_url = crate::sql::project_database_url(state, project.id).await?;
    crate::sql::exec_as_project(
        state,
        project.id,
        &project.pref,
        database_url.as_deref(),
        sql,
        !write,
        false,
    )
    .await
}

async fn list_functions(
    state: &AppState,
    operator: &Operator,
    pref: &str,
) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "developer").await?;
    let rows: Vec<(String, i32, String)> = sqlx::query_as(
        "SELECT name, version, created_at::text FROM reactor.deployments \
         WHERE project_id = $1 ORDER BY name, version DESC",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(json!(rows
        .into_iter()
        .map(|(name, version, created)| json!({"name": name, "version": version, "created_at": created}))
        .collect::<Vec<_>>()))
}

async fn function_stats(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    name: &str,
) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "developer").await?;
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM reactor.logs WHERE project_id = $1 AND kind = 'function' AND name = $2",
    )
    .bind(project.id)
    .bind(name)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    Ok(json!({"name": name, "calls": count}))
}

async fn list_sites(state: &AppState, operator: &Operator, pref: &str) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "developer").await?;
    let files: Vec<String> = sqlx::query_scalar(
        "SELECT path FROM reactor.site_files WHERE project_id = $1 ORDER BY path",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(json!({"files": files}))
}

async fn list_site_deployments(
    state: &AppState,
    operator: &Operator,
    pref: &str,
) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "developer").await?;
    let rows: Vec<(Uuid, String, i32, String)> = sqlx::query_as(
        "SELECT id, status, file_count, created_at::text FROM reactor.site_deployments \
         WHERE project_id = $1 ORDER BY created_at DESC LIMIT 50",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(json!(rows
        .into_iter()
        .map(|(id, status, count, created)| {
            json!({"id": id, "status": status, "file_count": count, "created_at": created})
        })
        .collect::<Vec<_>>()))
}

async fn list_buckets(
    state: &AppState,
    operator: &Operator,
    pref: &str,
) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "developer").await?;
    let pool = project_pool(state, project.id).await?;
    let rows = crate::buckets::list_buckets(&pool, &project.pref).await?;
    Ok(json!(rows
        .into_iter()
        .map(|(name, public)| json!({"name": name, "public": public}))
        .collect::<Vec<_>>()))
}

async fn list_objects(
    state: &AppState,
    operator: &Operator,
    pref: &str,
) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "developer").await?;
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
    Ok(json!(keys))
}

async fn list_queues(state: &AppState, operator: &Operator, pref: &str) -> Result<Value, ApiError> {
    let opened = queue_project(state, operator, pref).await?;
    let rows = reactor_ext_queue::list_queues(&opened)
        .await
        .map_err(queue_err)?;
    Ok(json!(rows))
}

async fn peek_queue(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    name: &str,
) -> Result<Value, ApiError> {
    let opened = queue_project(state, operator, pref).await?;
    let rows = reactor_ext_queue::peek(&opened, name)
        .await
        .map_err(queue_err)?;
    Ok(json!(rows))
}

async fn cluster(state: &AppState, operator: &Operator) -> Result<Value, ApiError> {
    if !operator.platform_admin {
        return Err(ApiError::forbidden("platform admin required"));
    }
    let name: Option<String> =
        sqlx::query_scalar("SELECT cluster_name FROM reactor.settings WHERE id = 1")
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    let db_ok = sqlx::query("SELECT 1").execute(&state.pool).await.is_ok();
    Ok(json!({"name": name.unwrap_or_default(), "postgres_ok": db_ok}))
}

async fn create_project(
    state: &AppState,
    operator: &Operator,
    name: &str,
) -> Result<Value, ApiError> {
    let pref = generate_ref();
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO reactor.projects (id, ref, name) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(&pref)
        .bind(name)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    crate::db::apply_project(
        &state.pool,
        std::path::Path::new(&state.config.sql_dir),
        id,
        &format!("proj_{pref}"),
        state.extension_sql.as_slice(),
    )
    .await
    .map_err(internal)?;
    crate::sql::ensure_project_role(&state.pool, &state.pool, state.issuer.seal_key(), id, &pref)
        .await
        .map_err(internal)?;
    crate::project_auth::insert_settings(&state.pool, id)
        .await
        .map_err(internal)?;
    let (anon, _service) = issue_keys(state, id, &pref).await?;
    sqlx::query(
        "INSERT INTO reactor.memberships (operator_id, project_id, role) VALUES ($1, $2, 'owner')",
    )
    .bind(operator.id)
    .bind(id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    reload(state).await?;
    Ok(json!({"id": id, "ref": pref, "name": name, "role": "owner", "anon": anon}))
}

async fn add_member(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    email: &str,
    name: &str,
    role: &str,
    password: &str,
) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "admin").await?;
    if role != "admin" && role != "developer" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "role must be admin or developer",
        ));
    }
    if password.len() < 8 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "password too short"));
    }
    let email = email.trim().to_lowercase();
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
        let hash = hash_password(password).map_err(internal)?;
        sqlx::query(
            "INSERT INTO reactor.operators (id, email, name, password_hash, platform_admin) \
             VALUES ($1, $2, $3, $4, false)",
        )
        .bind(id)
        .bind(&email)
        .bind(name.trim())
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
    .bind(role)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(json!({"email": email, "role": role}))
}

async fn apply_migration(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    version: &str,
    sql: &str,
) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "admin").await?;
    crate::sql::apply_user_migration(
        state,
        project.id,
        &project.pref,
        Some(operator.id),
        version,
        sql,
        None,
        "mcp",
    )
    .await?;
    reload(state).await?;
    Ok(json!({"ok": true, "version": version}))
}

async fn deploy_function(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    name: &str,
    source: &str,
) -> Result<Value, ApiError> {
    bounded(source, 256_000, "source")?;
    let project = require_member(state, operator, pref, "admin").await?;
    let zip = zip_single("index.ts", source).map_err(internal)?;
    let version = crate::deploy_zip(state, project.id, &project.pref, name, zip, false).await?;
    Ok(json!({"name": name, "version": version}))
}

async fn promote_function(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    name: &str,
    version: i32,
) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "admin").await?;
    crate::pin_function(state, project.id, &project.pref, name, version).await?;
    Ok(json!({"name": name, "version": version, "pinned": true}))
}

async fn demote_function(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    name: &str,
) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "admin").await?;
    crate::unpin_function(state, project.id, &project.pref, name).await?;
    Ok(json!({"name": name, "pinned": false}))
}

async fn set_function_env(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    name: &str,
    key: &str,
    value: &str,
) -> Result<Value, ApiError> {
    bounded(value, 8192, "value")?;
    let project = require_member(state, operator, pref, "admin").await?;
    if !crate::valid_name(name) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid function name",
        ));
    }
    if !crate::console::acceptable_function_env_key(key) {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid key"));
    }
    let (nonce, ciphertext) = crate::console::seal_env(&state.config.storage_sign_secret, value)?;
    sqlx::query(
        "INSERT INTO reactor.function_env (project_id, name, key, nonce, ciphertext) VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (project_id, name, key) DO UPDATE SET nonce = EXCLUDED.nonce, ciphertext = EXCLUDED.ciphertext, updated_at = now()",
    )
    .bind(project.id)
    .bind(name)
    .bind(key)
    .bind(&nonce)
    .bind(&ciphertext)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    crate::republish_function(state, project.id, &project.pref, name).await?;
    Ok(json!({"key": key}))
}

async fn unset_function_env(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    name: &str,
    key: &str,
) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "admin").await?;
    sqlx::query(
        "DELETE FROM reactor.function_env WHERE project_id = $1 AND name = $2 AND key = $3",
    )
    .bind(project.id)
    .bind(name)
    .bind(key)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    crate::republish_function(state, project.id, &project.pref, name).await?;
    Ok(json!({"key": key}))
}

async fn list_site_env(
    state: &AppState,
    operator: &Operator,
    pref: &str,
) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "developer").await?;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT key, visibility FROM reactor.site_env WHERE project_id = $1 ORDER BY key",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    Ok(json!(rows
        .into_iter()
        .map(|(key, visibility)| crate::console::site_env_public(&key, &visibility, false, None))
        .collect::<Vec<_>>()))
}

async fn put_site_file(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    path: &str,
    content: &str,
) -> Result<Value, ApiError> {
    bounded(content, 256_000, "content")?;
    let project = require_member(state, operator, pref, "admin").await?;
    let path = safe_site_path(path)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "invalid path"))?;
    let key = object_key(&project.pref, "_sites", &path)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid path"))?;
    let content_type = content_type_for(&path);
    state
        .blobs
        .put(&key, Bytes::from(content.to_string()), content_type)
        .await
        .map_err(internal)?;
    sqlx::query(
        "INSERT INTO reactor.site_files (project_id, path, blob_key, content_type) VALUES ($1, $2, $3, $4) \
         ON CONFLICT (project_id, path) DO UPDATE SET blob_key = EXCLUDED.blob_key, content_type = EXCLUDED.content_type",
    )
    .bind(project.id)
    .bind(&path)
    .bind(&key)
    .bind(content_type)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(json!({"path": path}))
}

async fn delete_site_file(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    path: &str,
) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "admin").await?;
    let path = safe_site_path(path)
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "invalid path"))?;
    let key: Option<String> = sqlx::query_scalar(
        "DELETE FROM reactor.site_files WHERE project_id = $1 AND path = $2 RETURNING blob_key",
    )
    .bind(project.id)
    .bind(&path)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    if let Some(key) = key {
        state.blobs.delete_key(&key).await.map_err(internal)?;
    }
    Ok(json!({"path": path}))
}

async fn set_site_env(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    key: &str,
    value: &str,
    visible: bool,
) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "admin").await?;
    let visibility = if visible { "visible" } else { "secret" };
    write_site_env(state, project.id, key, value, visibility).await?;
    Ok(json!({"key": key, "visibility": visibility}))
}

async fn unset_site_env(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    key: &str,
) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "admin").await?;
    sqlx::query("DELETE FROM reactor.site_env WHERE project_id = $1 AND key = $2")
        .bind(project.id)
        .bind(key)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(json!({"key": key}))
}

async fn create_bucket(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    name: &str,
    public: bool,
) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "admin").await?;
    let pool = project_pool(state, project.id).await?;
    crate::buckets::create_bucket(&pool, &project.pref, name, public).await?;
    state
        .bucket_flags
        .put(crate::buckets::cache_key(&project.pref, name), public);
    Ok(json!({"name": name, "public": public}))
}

async fn put_object(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    bucket: &str,
    path: &str,
    content: &str,
) -> Result<Value, ApiError> {
    bounded(content, 256_000, "content")?;
    let project = require_member(state, operator, pref, "admin").await?;
    let key = object_key(&project.pref, bucket, path)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid object key"))?;
    if !user_object(&project.pref, &key) {
        return Err(ApiError::forbidden("object is outside this project"));
    }
    state
        .blobs
        .put(
            &key,
            Bytes::from(content.to_string()),
            "text/plain; charset=utf-8",
        )
        .await
        .map_err(internal)?;
    Ok(json!({"key": key}))
}

async fn delete_object(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    bucket: &str,
    path: &str,
) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "admin").await?;
    let key = object_key(&project.pref, bucket, path)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid object key"))?;
    if !user_object(&project.pref, &key) {
        return Err(ApiError::forbidden("object is outside this project"));
    }
    state.blobs.delete_key(&key).await.map_err(internal)?;
    Ok(json!({"key": key}))
}

async fn create_queue(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    name: &str,
) -> Result<Value, ApiError> {
    let opened = queue_project(state, operator, pref).await?;
    reactor_ext_queue::create_queue(&opened, name)
        .await
        .map_err(queue_err)?;
    Ok(json!({"name": name}))
}

async fn send_queue(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    name: &str,
    message: Value,
) -> Result<Value, ApiError> {
    let opened = queue_project(state, operator, pref).await?;
    let id = reactor_ext_queue::send(&opened, name, message, 0)
        .await
        .map_err(queue_err)?;
    Ok(json!({"id": id}))
}

async fn create_user(
    state: &AppState,
    operator: &Operator,
    pref: &str,
    email: &str,
    password: &str,
) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "admin").await?;
    if password.len() < 8 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "password too short"));
    }
    let email = email.trim().to_ascii_lowercase();
    let id = Uuid::new_v4();
    let hash = hash_password(password).map_err(internal)?;
    sqlx::query(
        "INSERT INTO reactor.users (id, project_id, email, password_hash) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(project.id)
    .bind(&email)
    .bind(hash)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(json!({"id": id, "email": email}))
}

async fn rotate_project_keys(
    state: &AppState,
    operator: &Operator,
    pref: &str,
) -> Result<Value, ApiError> {
    let project = require_member(state, operator, pref, "admin").await?;
    sqlx::query("DELETE FROM reactor.api_keys WHERE project_id = $1")
        .bind(project.id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    let _keys = issue_keys(state, project.id, &project.pref).await?;
    Ok(json!({"ref": project.pref, "rotated": true}))
}

async fn migrate_all(state: &AppState, operator: &Operator) -> Result<Value, ApiError> {
    if !operator.platform_admin {
        return Err(ApiError::forbidden("platform admin required"));
    }
    crate::db::migrate_all(
        &state.pool,
        state.dedicated.as_ref(),
        std::path::Path::new(&state.config.sql_dir),
        state.config.dedicated_database_url.as_deref(),
        state.extension_sql.as_slice(),
        state.issuer.seal_key(),
    )
    .await
    .map_err(internal)?;
    reload(state).await?;
    Ok(json!({"ok": true}))
}

async fn queue_project(
    state: &AppState,
    operator: &Operator,
    pref: &str,
) -> Result<OpenedProject, ApiError> {
    if !state.extensions.iter().any(|item| item.name == "queue") {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "queue extension is off",
        ));
    }
    let project = require_member(state, operator, pref, "developer").await?;
    let url = crate::sql::project_database_url(state, project.id).await?;
    let pool = crate::pool_for(state, url.as_deref())
        .await
        .map_err(internal)?;
    Ok(OpenedProject {
        id: project.id,
        pref: project.pref,
        schema: format!("proj_{pref}"),
        pool,
    })
}

async fn project_pool(state: &AppState, id: Uuid) -> Result<sqlx::PgPool, ApiError> {
    let url = crate::sql::project_database_url(state, id).await?;
    crate::pool_for(state, url.as_deref())
        .await
        .map_err(internal)
}

fn internal(err: impl std::fmt::Display) -> ApiError {
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
}

fn queue_err(err: reactor_ext_queue::QueueError) -> ApiError {
    ApiError::new(err.status(), err.message())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_advertises_png_and_svg_icons() {
        let body = json!({"params": {"protocolVersion": "2025-06-18"}});
        let result = initialize(&body, "http://127.0.0.1:18000/");
        let icons = &result["serverInfo"]["icons"];
        assert_eq!(icons[0]["src"], "http://127.0.0.1:18000/favicon.png");
        assert_eq!(icons[0]["mimeType"], "image/png");
        assert_eq!(icons[0]["sizes"][0], "128x128");
        assert_eq!(icons[1]["src"], "http://127.0.0.1:18000/favicon.svg");
        assert_eq!(icons[1]["mimeType"], "image/svg+xml");
        assert_eq!(icons[1]["sizes"][0], "any");
        assert!(FAVICON_PNG.starts_with(b"\x89PNG"));
        assert!(FAVICON_ICO.starts_with(&[0, 0, 1, 0]));
    }
}
