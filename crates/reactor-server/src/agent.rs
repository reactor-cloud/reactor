use crate::console::{
    cast_for, checked_table, column_types, ident, issue_keys, json_text, load_operator,
    primary_key, record_log, require_member, require_operator, schema_name, sql_type, user_object,
    Operator,
};
use crate::{bearer, generate_ref, internal, reload, ApiError, AppState};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use bytes::Bytes;
use reactor_auth::{constant_time_eq, hash_password};
use reactor_sites::{content_type_for, safe_site_path};
use reactor_storage::object_key;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use uuid::Uuid;

const SECRET_KEYS: &[&str] = &[
    "password",
    "anon_key",
    "service_key",
    "api_key",
    "authorization",
];

pub fn mount(app: Router<crate::AppCtx>) -> Router<crate::AppCtx> {
    app.route(
        "/console/v1/agent/threads",
        get(list_threads).post(create_thread),
    )
    .route("/console/v1/agent/models", get(list_models))
    .route("/console/v1/agent/latest", get(latest_thread))
    .route("/console/v1/agent/threads/{id}", get(get_thread))
    .route(
        "/console/v1/agent/threads/{id}/messages",
        post(post_message),
    )
    .route("/console/v1/agent/threads/{id}/run", post(run_thread))
}

pub fn redact(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                if SECRET_KEYS
                    .iter()
                    .any(|secret| key.eq_ignore_ascii_case(secret))
                {
                    *child = Value::String("[redacted]".into());
                } else {
                    redact(child);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                redact(item);
            }
        }
        _ => {}
    }
}

pub fn log_line(args: &Value) -> String {
    let mut copy = args.clone();
    redact(&mut copy);
    copy.to_string()
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Default, Debug)]
pub struct StreamAccum {
    pub content: String,
    pub tool_calls: Vec<ToolCall>,
}

pub fn push_sse_line(acc: &mut StreamAccum, line: &str) {
    let line = line.trim();
    if line.is_empty() || line.starts_with(':') {
        return;
    }
    let Some(data) = line.strip_prefix("data:") else {
        return;
    };
    let data = data.trim();
    if data == "[DONE]" {
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return;
    };
    let delta = &value["choices"][0]["delta"];
    if let Some(text) = delta["content"].as_str() {
        acc.content.push_str(text);
    }
    if let Some(calls) = delta["tool_calls"].as_array() {
        for call in calls {
            let index = call["index"].as_u64().unwrap_or(0) as usize;
            while acc.tool_calls.len() <= index {
                acc.tool_calls.push(ToolCall {
                    id: String::new(),
                    name: String::new(),
                    arguments: String::new(),
                });
            }
            let slot = &mut acc.tool_calls[index];
            if let Some(id) = call["id"].as_str() {
                if !id.is_empty() {
                    slot.id = id.to_string();
                }
            }
            if let Some(name) = call["function"]["name"].as_str() {
                if !name.is_empty() {
                    slot.name = name.to_string();
                }
            }
            if let Some(args) = call["function"]["arguments"].as_str() {
                slot.arguments.push_str(args);
            }
        }
    }
}

#[derive(Debug)]
pub enum TurnStart {
    Local,
    Invoke { function_name: String, event: Value },
}

pub fn plan_turn_start(
    mode: &str,
    function_name: &str,
    thread_id: Uuid,
    operator_token: &str,
) -> TurnStart {
    if mode == "lambda" {
        TurnStart::Invoke {
            function_name: function_name.to_string(),
            event: run_event(thread_id, operator_token),
        }
    } else {
        TurnStart::Local
    }
}

pub fn run_event(thread_id: Uuid, operator_token: &str) -> Value {
    let path = format!("/console/v1/agent/threads/{thread_id}/run");
    json!({
        "version": "2.0",
        "rawPath": path,
        "requestContext": {"http": {"method": "POST", "path": path}},
        "headers": {
            "host": "localhost",
            "authorization": format!("Bearer {operator_token}")
        }
    })
}

#[derive(Deserialize)]
struct CreateThread {
    #[serde(default)]
    project_ref: Option<String>,
}

#[derive(Deserialize)]
struct PostBody {
    text: String,
    #[serde(default)]
    model: Option<String>,
}

pub fn clean_model(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 128 {
        return None;
    }
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | ':' | '@' | '+'))
    {
        Some(value.to_string())
    } else {
        None
    }
}

async fn create_thread(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CreateThread>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO reactor.agent_threads (id, operator_id, project_ref) VALUES ($1, $2, $3)",
    )
    .bind(id)
    .bind(operator.id)
    .bind(body.project_ref.filter(|value| !value.is_empty()))
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(json!({ "id": id })))
}

async fn list_threads(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let rows: Vec<(Uuid, bool, Option<String>, i64)> = sqlx::query_as(
        "SELECT t.id, t.running, left(m.content, 180), \
         (EXTRACT(EPOCH FROM COALESCE(m.created_at, t.created_at)) * 1000)::bigint \
         FROM reactor.agent_threads t \
         LEFT JOIN LATERAL ( \
           SELECT content, created_at FROM reactor.agent_messages \
           WHERE thread_id = t.id AND role = 'user' ORDER BY seq DESC LIMIT 1 \
         ) m ON true \
         WHERE t.operator_id = $1 \
         ORDER BY COALESCE(m.created_at, t.created_at) DESC \
         LIMIT 80",
    )
    .bind(operator.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let threads: Vec<Value> = rows
        .into_iter()
        .map(|(id, running, preview, at)| {
            json!({
                "id": id,
                "running": running,
                "preview": preview,
                "at": at,
            })
        })
        .collect();
    Ok(Json(json!(threads)))
}

async fn list_models(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    require_operator(&state, &headers).await?;
    if state.config.agent_api_key.is_empty() {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "agent is not configured",
        ));
    }
    let url = format!(
        "{}/models",
        state.config.agent_base_url.trim_end_matches('/')
    );
    let response = state
        .http
        .get(url)
        .bearer_auth(&state.config.agent_api_key)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(internal)?;
    if !response.status().is_success() {
        return Err(ApiError::new(StatusCode::BAD_GATEWAY, "model list failed"));
    }
    let body: Value = response.json().await.map_err(internal)?;
    let mut models = Vec::new();
    if let Some(rows) = body["data"].as_array() {
        for row in rows {
            let Some(id) = row["id"].as_str().and_then(clean_model) else {
                continue;
            };
            let name = row["name"]
                .as_str()
                .filter(|value| !value.is_empty())
                .map(|value| value.to_string())
                .unwrap_or_else(|| id.clone());
            models.push(json!({ "id": id, "name": name }));
        }
    }
    models.sort_by(|left, right| {
        left["name"]
            .as_str()
            .unwrap_or("")
            .cmp(right["name"].as_str().unwrap_or(""))
    });
    Ok(Json(json!({
        "default": state.config.agent_model,
        "models": models
    })))
}

async fn latest_thread(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let id: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM reactor.agent_threads WHERE operator_id = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(operator.id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some(id) = id else {
        return Ok(Json(
            json!({ "id": null, "running": false, "messages": [] }),
        ));
    };
    Ok(Json(thread_json(&state, operator.id, id).await?))
}

async fn get_thread(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    Ok(Json(thread_json(&state, operator.id, id).await?))
}

async fn post_message(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<PostBody>,
) -> Result<impl IntoResponse, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    if state.config.agent_api_key.is_empty() {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "agent is not configured",
        ));
    }
    let text = body.text.trim().to_string();
    if text.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "text is required"));
    }
    let model = match body
        .model
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(value) => Some(
            clean_model(value)
                .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "unknown model"))?,
        ),
        None => None,
    };
    owned_thread(&state, operator.id, id).await?;
    if let Some(model) = &model {
        sqlx::query("UPDATE reactor.agent_threads SET model = $1 WHERE id = $2")
            .bind(model)
            .bind(id)
            .execute(&state.pool)
            .await
            .map_err(internal)?;
    }
    append_message(&state, id, "user", &text, None, None, None)
        .await
        .map_err(internal)?;
    let _ = sqlx::query("UPDATE reactor.agent_threads SET running = true WHERE id = $1")
        .bind(id)
        .execute(&state.pool)
        .await;
    start_turn(&state, id).await;
    Ok((StatusCode::ACCEPTED, Json(json!({ "id": id }))))
}

async fn run_thread(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let token = bearer(&headers).unwrap_or_default();
    if state.config.operator_token.is_empty()
        || !constant_time_eq(&token, &state.config.operator_token)
    {
        return Err(ApiError::unauthorized("operator token required"));
    }
    if state.config.agent_api_key.is_empty() {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "agent is not configured",
        ));
    }
    run_turn(state, id).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn start_turn(state: &AppState, thread_id: Uuid) {
    let function_name =
        std::env::var("AWS_LAMBDA_FUNCTION_NAME").unwrap_or_else(|_| "reactor-platform".into());
    match plan_turn_start(
        &state.config.mode,
        &function_name,
        thread_id,
        &state.config.operator_token,
    ) {
        TurnStart::Local => {
            let state = state.clone();
            tokio::spawn(async move {
                run_turn(state, thread_id).await;
            });
        }
        TurnStart::Invoke {
            function_name,
            event,
        } => {
            let region =
                std::env::var("AWS_REGION").unwrap_or_else(|_| state.config.storage_region.clone());
            if let Err(err) = invoke_self(&function_name, &region, event).await {
                tracing::error!("agent invoke failed: {err}");
                let _ =
                    sqlx::query("UPDATE reactor.agent_threads SET running = false WHERE id = $1")
                        .bind(thread_id)
                        .execute(&state.pool)
                        .await;
            }
        }
    }
}

async fn invoke_self(function_name: &str, region: &str, event: Value) -> anyhow::Result<()> {
    let shared = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(aws_config::Region::new(region.to_string()))
        .load()
        .await;
    let client = aws_sdk_lambda::Client::new(&shared);
    client
        .invoke()
        .function_name(function_name)
        .invocation_type(aws_sdk_lambda::types::InvocationType::Event)
        .payload(aws_sdk_lambda::primitives::Blob::new(
            event.to_string().into_bytes(),
        ))
        .send()
        .await?;
    Ok(())
}

pub async fn run_turn(state: AppState, thread_id: Uuid) {
    loop {
        let mut conn = match state.pool.acquire().await {
            Ok(conn) => conn,
            Err(err) => {
                tracing::error!("{err}");
                return;
            }
        };
        let key = lock_key(thread_id);
        let locked: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
            .bind(key)
            .fetch_one(&mut *conn)
            .await
            .unwrap_or(false);
        if !locked {
            return;
        }
        let _ = sqlx::query("UPDATE reactor.agent_threads SET running = true WHERE id = $1")
            .bind(thread_id)
            .execute(&state.pool)
            .await;
        let budget = state
            .config
            .agent_turn_deadline_secs
            .saturating_sub(5)
            .max(1);
        let deadline = Instant::now() + Duration::from_secs(budget);
        if let Err(err) = process_until_idle(&state, thread_id, deadline).await {
            tracing::error!("{err}");
            let detail: String = err.to_string().chars().take(300).collect();
            let _ = append_message(
                &state,
                thread_id,
                "assistant",
                &format!("The turn failed. {detail}"),
                None,
                None,
                None,
            )
            .await;
        }
        let _ = sqlx::query("UPDATE reactor.agent_threads SET running = false WHERE id = $1")
            .bind(thread_id)
            .execute(&state.pool)
            .await;
        let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
            .bind(key)
            .execute(&mut *conn)
            .await;
        drop(conn);
        let Ok(messages) = load_messages(&state.pool, thread_id).await else {
            return;
        };
        if is_idle(&messages) {
            return;
        }
    }
}

fn lock_key(id: Uuid) -> i64 {
    i64::from_be_bytes(id.as_bytes()[..8].try_into().unwrap())
}

async fn process_until_idle(
    state: &AppState,
    thread_id: Uuid,
    deadline: Instant,
) -> anyhow::Result<()> {
    loop {
        if Instant::now() >= deadline {
            append_message(
                state,
                thread_id,
                "assistant",
                "This turn was cut off at the time limit.",
                None,
                None,
                None,
            )
            .await?;
            return Ok(());
        }
        let messages = load_messages(&state.pool, thread_id).await?;
        if is_idle(&messages) {
            return Ok(());
        }
        complete_round(state, thread_id, &messages, deadline).await?;
    }
}

fn is_idle(messages: &[StoredMessage]) -> bool {
    match messages.last() {
        None => true,
        Some(message) => {
            message.role == "assistant"
                && message
                    .tool_calls
                    .as_ref()
                    .map(|v| v.as_array().map(|a| a.is_empty()).unwrap_or(true))
                    .unwrap_or(true)
        }
    }
}

async fn complete_round(
    state: &AppState,
    thread_id: Uuid,
    messages: &[StoredMessage],
    deadline: Instant,
) -> anyhow::Result<()> {
    let operator_id = thread_operator(&state.pool, thread_id).await?;
    let operator = load_operator(state, operator_id)
        .await
        .map_err(|err| anyhow::anyhow!(err.text().to_string()))?;
    let project_ref = thread_project(&state.pool, thread_id).await?;
    let model = thread_model(&state.pool, thread_id)
        .await?
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| state.config.agent_model.clone());
    let request = model_request(&project_ref, messages, &model);
    let url = format!(
        "{}/chat/completions",
        state.config.agent_base_url.trim_end_matches('/')
    );
    let remaining = deadline.saturating_duration_since(Instant::now());
    let response = state
        .http
        .post(url)
        .bearer_auth(&state.config.agent_api_key)
        .timeout(remaining)
        .json(&request)
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!(
            "model request failed: {status} {}",
            body.chars().take(240).collect::<String>()
        );
    }
    let stored = append_message(state, thread_id, "assistant", "", None, None, None).await?;
    let mut acc = StreamAccum::default();
    let mut buf = String::new();
    let mut response = response;
    let mut last_flush = Instant::now();
    while let Some(chunk) = response.chunk().await? {
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buf.find('\n') {
            let line: String = buf.drain(..=pos).collect();
            push_sse_line(&mut acc, &line);
        }
        if last_flush.elapsed() >= Duration::from_secs(1) {
            flush_assistant(&state.pool, stored.id, &acc).await?;
            last_flush = Instant::now();
        }
    }
    if !buf.trim().is_empty() {
        push_sse_line(&mut acc, &buf);
    }
    for (index, call) in acc.tool_calls.iter_mut().enumerate() {
        if call.id.is_empty() {
            call.id = format!("call_{}_{index}", stored.seq);
        }
    }
    let originals = acc.tool_calls.clone();
    for call in &mut acc.tool_calls {
        if let Ok(mut parsed) = serde_json::from_str::<Value>(&call.arguments) {
            redact(&mut parsed);
            call.arguments = parsed.to_string();
        }
    }
    let tool_calls = if acc.tool_calls.is_empty() {
        None
    } else {
        Some(json!(acc
            .tool_calls
            .iter()
            .map(|call| json!({"id": call.id, "name": call.name, "arguments": call.arguments}))
            .collect::<Vec<_>>()))
    };
    sqlx::query("UPDATE reactor.agent_messages SET content = $1, tool_calls = $2 WHERE id = $3")
        .bind(&acc.content)
        .bind(tool_calls.as_ref())
        .bind(stored.id)
        .execute(&state.pool)
        .await?;
    for call in &originals {
        let args = serde_json::from_str::<Value>(&call.arguments).unwrap_or(json!({}));
        let mut result = execute_tool(state, &operator, thread_id, &call.name, &args).await;
        redact(&mut result);
        let text = result.to_string();
        append_message(
            state,
            thread_id,
            "tool",
            &text,
            None,
            Some(&call.id),
            Some(&call.name),
        )
        .await?;
    }
    Ok(())
}

async fn flush_assistant(pool: &sqlx::PgPool, id: i64, acc: &StreamAccum) -> anyhow::Result<()> {
    sqlx::query("UPDATE reactor.agent_messages SET content = $1 WHERE id = $2")
        .bind(&acc.content)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

fn model_request(project_ref: &Option<String>, messages: &[StoredMessage], model: &str) -> Value {
    let mut out = vec![json!({"role": "system", "content": system_prompt(project_ref)})];
    for message in messages {
        match message.role.as_str() {
            "user" => out.push(json!({"role": "user", "content": message.content})),
            "tool" => out.push(json!({
                "role": "tool",
                "tool_call_id": message.tool_call_id,
                "content": message.content
            })),
            "assistant" => {
                let mut row = json!({"role": "assistant", "content": message.content});
                if message.content.is_empty() {
                    row["content"] = Value::Null;
                }
                if let Some(calls) = &message.tool_calls {
                    if let Some(list) = calls.as_array() {
                        row["tool_calls"] = Value::Array(
                            list.iter()
                                .map(|call| {
                                    json!({
                                        "id": call["id"],
                                        "type": "function",
                                        "function": {
                                            "name": call["name"],
                                            "arguments": call["arguments"]
                                        }
                                    })
                                })
                                .collect(),
                        );
                    }
                }
                out.push(row);
            }
            _ => {}
        }
    }
    json!({
        "model": model,
        "temperature": 0,
        "stream": true,
        "messages": out,
        "tools": tool_schemas()
    })
}

fn system_prompt(project_ref: &Option<String>) -> String {
    let open = project_ref
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(|pref| format!("The open project ref is {pref}."))
        .unwrap_or_else(|| "No project is open.".into());
    format!(
        "You are Reactor Operator. Do the user's task with the tools. Take refs and ids from tool results; do not invent them. Never repeat API keys or passwords. {open} \
create_project takes a name and returns a ref. create_table columns use type text, uuid, int, bool, date, timestamptz, numeric, or jsonb. \
insert_row returns the primary key as id. update_row writes one cell. create_user adds a project user and needs a password of at least 8 characters. \
put_object stores a file at a path such as notes/hello.txt. put_site_file with path index.html publishes the site."
    )
}

fn tool_schemas() -> Value {
    json!([
        tool(
            "create_project",
            "Create a project and return its ref.",
            json!({"name": {"type": "string"}}),
            &["name"]
        ),
        tool(
            "list_projects",
            "List projects this operator can open.",
            json!({}),
            &[]
        ),
        tool(
            "schema",
            "List tables and columns.",
            json!({"ref": {"type": "string"}}),
            &["ref"]
        ),
        tool(
            "create_table",
            "Create a table.",
            json!({
                "ref": {"type": "string"},
                "name": {"type": "string"},
                "columns": {"type": "array", "items": {"type": "object", "properties": {"name": {"type": "string"}, "type": {"type": "string"}}, "required": ["name", "type"]}}
            }),
            &["ref", "name", "columns"]
        ),
        tool(
            "insert_row",
            "Insert a row. Returns id.",
            json!({
                "ref": {"type": "string"},
                "table": {"type": "string"},
                "values": {"type": "object"}
            }),
            &["ref", "table", "values"]
        ),
        tool(
            "list_rows",
            "List up to 20 rows.",
            json!({"ref": {"type": "string"}, "table": {"type": "string"}}),
            &["ref", "table"]
        ),
        tool(
            "update_row",
            "Set one cell identified by its primary key.",
            json!({
                "ref": {"type": "string"},
                "table": {"type": "string"},
                "pk": {"type": "string"},
                "column": {"type": "string"},
                "value": {}
            }),
            &["ref", "table", "pk", "column", "value"]
        ),
        tool(
            "list_users",
            "List project users.",
            json!({"ref": {"type": "string"}}),
            &["ref"]
        ),
        tool(
            "create_user",
            "Add a project user.",
            json!({
                "ref": {"type": "string"},
                "email": {"type": "string"},
                "password": {"type": "string"}
            }),
            &["ref", "email", "password"]
        ),
        tool(
            "list_objects",
            "List stored files.",
            json!({"ref": {"type": "string"}}),
            &["ref"]
        ),
        tool(
            "put_object",
            "Store a text file in the project.",
            json!({
                "ref": {"type": "string"},
                "path": {"type": "string"},
                "content": {"type": "string"}
            }),
            &["ref", "path", "content"]
        ),
        tool(
            "list_sites",
            "List published site files.",
            json!({"ref": {"type": "string"}}),
            &["ref"]
        ),
        tool(
            "put_site_file",
            "Publish a site file. Use path index.html for the home page.",
            json!({
                "ref": {"type": "string"},
                "path": {"type": "string"},
                "content": {"type": "string"}
            }),
            &["ref", "path", "content"]
        )
    ])
}

fn tool(name: &str, description: &str, properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": {"type": "object", "properties": properties, "required": required}
        }
    })
}

async fn execute_tool(
    state: &AppState,
    operator: &Operator,
    thread_id: Uuid,
    name: &str,
    args: &Value,
) -> Value {
    let outcome = match name {
        "create_project" => create_project(state, operator, thread_id, args).await,
        "list_projects" => list_projects(state, operator).await,
        "schema" => list_schema(state, operator, args).await,
        "create_table" => create_table(state, operator, args).await,
        "insert_row" => insert_row(state, operator, args).await,
        "list_rows" => list_rows(state, operator, args).await,
        "update_row" => update_row(state, operator, args).await,
        "list_users" => list_users(state, operator, args).await,
        "create_user" => create_user(state, operator, args).await,
        "list_objects" => list_objects(state, operator, args).await,
        "put_object" => put_object(state, operator, args).await,
        "list_sites" => list_sites(state, operator, args).await,
        "put_site_file" => put_site_file(state, operator, args).await,
        _ => Err(format!("unknown tool {name}")),
    };
    match outcome {
        Ok(value) => value,
        Err(err) => json!({"error": err}),
    }
}

async fn create_project(
    state: &AppState,
    operator: &Operator,
    thread_id: Uuid,
    args: &Value,
) -> Result<Value, String> {
    let name = args["name"].as_str().unwrap_or("").trim().to_string();
    if name.is_empty() {
        return Err("name is required".into());
    }
    let pref = generate_ref();
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO reactor.projects (id, ref, name) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(&pref)
        .bind(&name)
        .execute(&state.pool)
        .await
        .map_err(|err| err.to_string())?;
    crate::db::apply_project(
        &state.pool,
        std::path::Path::new(&state.config.sql_dir),
        id,
        &format!("proj_{pref}"),
        state.extension_sql.as_slice(),
    )
    .await
    .map_err(|err| err.to_string())?;
    crate::project_auth::insert_settings(&state.pool, id)
        .await
        .map_err(|err| err.to_string())?;
    let _keys = issue_keys(state, id, &pref)
        .await
        .map_err(|err| err.text().to_string())?;
    sqlx::query(
        "INSERT INTO reactor.memberships (operator_id, project_id, role) VALUES ($1, $2, 'owner')",
    )
    .bind(operator.id)
    .bind(id)
    .execute(&state.pool)
    .await
    .map_err(|err| err.to_string())?;
    let _ = sqlx::query("UPDATE reactor.agent_threads SET project_ref = $1 WHERE id = $2")
        .bind(&pref)
        .bind(thread_id)
        .execute(&state.pool)
        .await;
    reload(state).await.map_err(|err| err.text().to_string())?;
    let result = json!({"id": id, "ref": pref, "name": name, "role": "owner"});
    note(state, id, "create_project", args, 201).await;
    Ok(result)
}

async fn list_projects(state: &AppState, operator: &Operator) -> Result<Value, String> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT p.ref, p.name, m.role FROM reactor.projects p \
         JOIN reactor.memberships m ON m.project_id = p.id \
         WHERE m.operator_id = $1 ORDER BY p.name",
    )
    .bind(operator.id)
    .fetch_all(&state.pool)
    .await
    .map_err(|err| err.to_string())?;
    Ok(json!(rows
        .into_iter()
        .map(|(pref, name, role)| json!({"ref": pref, "name": name, "role": role}))
        .collect::<Vec<_>>()))
}

async fn list_schema(state: &AppState, operator: &Operator, args: &Value) -> Result<Value, String> {
    let project = member(state, operator, arg(args, "ref")?, "developer").await?;
    let schema = schema_name(&project.pref).map_err(|err| err.text().to_string())?;
    let tables: Vec<String> = sqlx::query_scalar(
        "SELECT table_name FROM information_schema.tables WHERE table_schema = $1 AND table_type = 'BASE TABLE' ORDER BY table_name",
    )
    .bind(&schema)
    .fetch_all(&state.pool)
    .await
    .map_err(|err| err.to_string())?;
    let mut out = Vec::new();
    for table in tables {
        let columns = column_types(&state.pool, &schema, &table)
            .await
            .map_err(|err| err.text().to_string())?;
        out.push(json!({
            "name": table,
            "columns": columns.into_iter().map(|(n, t)| json!({"name": n, "type": t})).collect::<Vec<_>>()
        }));
    }
    Ok(json!(out))
}

async fn create_table(
    state: &AppState,
    operator: &Operator,
    args: &Value,
) -> Result<Value, String> {
    let project = member(state, operator, arg(args, "ref")?, "admin").await?;
    let name = arg(args, "name")?;
    if !ident(&name) {
        return Err("invalid table".into());
    }
    let schema = schema_name(&project.pref).map_err(|err| err.text().to_string())?;
    let columns = args["columns"].as_array().ok_or("columns are required")?;
    let mut defs = Vec::new();
    let mut has_id = false;
    let mut has_user = false;
    for column in columns {
        let column_name = column["name"].as_str().unwrap_or("");
        if !ident(column_name) {
            return Err("invalid column".into());
        }
        let ty =
            sql_type(column["type"].as_str().unwrap_or("")).ok_or("unsupported column type")?;
        if column_name == "id" {
            has_id = true;
        }
        if column_name == "user_id" {
            has_user = true;
        }
        defs.push(format!("\"{column_name}\" {ty}"));
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
        "CREATE TABLE \"{schema}\".\"{name}\" ({defs}); \
         ALTER TABLE \"{schema}\".\"{name}\" ENABLE ROW LEVEL SECURITY; \
         ALTER TABLE \"{schema}\".\"{name}\" FORCE ROW LEVEL SECURITY; \
         CREATE POLICY \"{name}_access\" ON \"{schema}\".\"{name}\" FOR ALL TO authenticated USING ({using}) WITH CHECK ({using}); \
         GRANT SELECT, INSERT, UPDATE, DELETE ON \"{schema}\".\"{name}\" TO authenticated, service;",
        defs = defs.join(", ")
    );
    sqlx::raw_sql(&sql)
        .execute(&state.pool)
        .await
        .map_err(|err| err.to_string())?;
    reload(state).await.map_err(|err| err.text().to_string())?;
    note(state, project.id, "create_table", args, 201).await;
    Ok(json!({"name": name}))
}

async fn insert_row(state: &AppState, operator: &Operator, args: &Value) -> Result<Value, String> {
    let project = member(state, operator, arg(args, "ref")?, "admin").await?;
    let table = arg(args, "table")?;
    let schema = checked_table(&project.pref, &table).map_err(|err| err.text().to_string())?;
    let values = args["values"].as_object().ok_or("values are required")?;
    let columns = column_types(&state.pool, &schema, &table)
        .await
        .map_err(|err| err.text().to_string())?;
    let mut names = Vec::new();
    let mut casts = Vec::new();
    let mut bound = Vec::new();
    for (name, ty) in &columns {
        let Some(value) = values.get(name) else {
            continue;
        };
        let text = json_text(value);
        if text.is_empty() {
            continue;
        }
        let cast = cast_for(ty).ok_or("column cannot be written")?;
        names.push(format!("\"{name}\""));
        casts.push(format!("${}::{cast}", names.len()));
        bound.push(text);
    }
    let (pk, _) = primary_key(&state.pool, &schema, &table)
        .await
        .map_err(|err| err.text().to_string())?
        .ok_or("table has no single primary key")?;
    let sql = if names.is_empty() {
        format!("INSERT INTO \"{schema}\".\"{table}\" DEFAULT VALUES RETURNING \"{pk}\"::text")
    } else {
        format!(
            "INSERT INTO \"{schema}\".\"{table}\" ({}) VALUES ({}) RETURNING \"{pk}\"::text",
            names.join(", "),
            casts.join(", ")
        )
    };
    let mut query = sqlx::query_scalar::<_, String>(&sql);
    for value in &bound {
        query = query.bind(value);
    }
    let id = query
        .fetch_one(&state.pool)
        .await
        .map_err(|err| err.to_string())?;
    note(state, project.id, "insert_row", args, 201).await;
    Ok(json!({"id": id}))
}

async fn list_rows(state: &AppState, operator: &Operator, args: &Value) -> Result<Value, String> {
    let project = member(state, operator, arg(args, "ref")?, "developer").await?;
    let table = arg(args, "table")?;
    let schema = checked_table(&project.pref, &table).map_err(|err| err.text().to_string())?;
    let rows: Vec<Value> = sqlx::query_scalar(&format!(
        "SELECT to_jsonb(t) FROM \"{schema}\".\"{table}\" t LIMIT 20"
    ))
    .fetch_all(&state.pool)
    .await
    .map_err(|err| err.to_string())?;
    Ok(json!(rows))
}

async fn update_row(state: &AppState, operator: &Operator, args: &Value) -> Result<Value, String> {
    let project = member(state, operator, arg(args, "ref")?, "admin").await?;
    let table = arg(args, "table")?;
    let column = arg(args, "column")?;
    let pk_value = json_text(&args["pk"]);
    if pk_value.is_empty() {
        return Err("pk is required".into());
    }
    let schema = checked_table(&project.pref, &table).map_err(|err| err.text().to_string())?;
    let (pk, pk_ty) = primary_key(&state.pool, &schema, &table)
        .await
        .map_err(|err| err.text().to_string())?
        .ok_or("table has no single primary key")?;
    if !ident(&column) || column == pk {
        return Err("invalid column".into());
    }
    let columns = column_types(&state.pool, &schema, &table)
        .await
        .map_err(|err| err.text().to_string())?;
    let ty = columns
        .iter()
        .find(|(name, _)| name == &column)
        .map(|(_, ty)| ty.as_str())
        .ok_or("invalid column")?;
    let cast = cast_for(ty).ok_or("column cannot be written")?;
    let pk_cast = cast_for(&pk_ty).unwrap_or("text");
    let sql = format!(
        "UPDATE \"{schema}\".\"{table}\" SET \"{column}\" = $1::{cast} WHERE \"{pk}\" = $2::{pk_cast}"
    );
    let updated = sqlx::query(&sql)
        .bind(json_text(&args["value"]))
        .bind(&pk_value)
        .execute(&state.pool)
        .await
        .map_err(|err| err.to_string())?;
    if updated.rows_affected() == 0 {
        return Err("row not found".into());
    }
    note(state, project.id, "update_row", args, 200).await;
    Ok(json!({"updated": true}))
}

async fn list_users(state: &AppState, operator: &Operator, args: &Value) -> Result<Value, String> {
    let project = member(state, operator, arg(args, "ref")?, "admin").await?;
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, email FROM reactor.users WHERE project_id = $1 ORDER BY created_at",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await
    .map_err(|err| err.to_string())?;
    Ok(json!(rows
        .into_iter()
        .map(|(id, email)| json!({"id": id, "email": email}))
        .collect::<Vec<_>>()))
}

async fn create_user(state: &AppState, operator: &Operator, args: &Value) -> Result<Value, String> {
    let project = member(state, operator, arg(args, "ref")?, "admin").await?;
    let email = arg(args, "email")?.trim().to_ascii_lowercase();
    let password = arg(args, "password")?;
    if password.len() < 8 {
        return Err("password too short".into());
    }
    let id = Uuid::new_v4();
    let hash = hash_password(&password).map_err(|err| err.to_string())?;
    sqlx::query(
        "INSERT INTO reactor.users (id, project_id, email, password_hash) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(project.id)
    .bind(&email)
    .bind(hash)
    .execute(&state.pool)
    .await
    .map_err(|err| err.to_string())?;
    note(state, project.id, "create_user", args, 201).await;
    Ok(json!({"id": id, "email": email}))
}

async fn list_objects(
    state: &AppState,
    operator: &Operator,
    args: &Value,
) -> Result<Value, String> {
    let project = member(state, operator, arg(args, "ref")?, "developer").await?;
    let keys = state
        .blobs
        .list_objects(&format!("{}/", project.pref))
        .await
        .map_err(|err| err.to_string())?;
    let keys: Vec<Value> = keys
        .into_iter()
        .filter(|item| user_object(&project.pref, &item.key))
        .map(|item| json!({"key": item.key, "size": item.size}))
        .collect();
    Ok(json!(keys))
}

async fn put_object(state: &AppState, operator: &Operator, args: &Value) -> Result<Value, String> {
    let project = member(state, operator, arg(args, "ref")?, "admin").await?;
    let path = arg(args, "path")?;
    let content = arg(args, "content")?;
    if content.len() > 256_000 {
        return Err("content is too large".into());
    }
    let key = format!("{}/{}", project.pref, path.trim_start_matches('/'));
    if !user_object(&project.pref, &key) {
        return Err("object is outside this project".into());
    }
    state
        .blobs
        .put(
            &key,
            Bytes::from(content.into_bytes()),
            "text/plain; charset=utf-8",
        )
        .await
        .map_err(|err| err.to_string())?;
    note(state, project.id, "put_object", args, 201).await;
    Ok(json!({"key": key}))
}

async fn list_sites(state: &AppState, operator: &Operator, args: &Value) -> Result<Value, String> {
    let project = member(state, operator, arg(args, "ref")?, "developer").await?;
    let files: Vec<String> = sqlx::query_scalar(
        "SELECT path FROM reactor.site_files WHERE project_id = $1 ORDER BY path",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await
    .map_err(|err| err.to_string())?;
    Ok(json!({"files": files}))
}

async fn put_site_file(
    state: &AppState,
    operator: &Operator,
    args: &Value,
) -> Result<Value, String> {
    let project = member(state, operator, arg(args, "ref")?, "admin").await?;
    let path = safe_site_path(&arg(args, "path")?).ok_or("invalid path")?;
    let content = arg(args, "content")?;
    if content.len() > 256_000 {
        return Err("content is too large".into());
    }
    let key = object_key(&project.pref, "_sites", &path).map_err(|_| "invalid path")?;
    let content_type = content_type_for(&path);
    state
        .blobs
        .put(&key, Bytes::from(content.into_bytes()), content_type)
        .await
        .map_err(|err| err.to_string())?;
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
    .map_err(|err| err.to_string())?;
    note(state, project.id, "put_site_file", args, 201).await;
    Ok(json!({"path": path}))
}

async fn note(state: &AppState, project_id: Uuid, tool: &str, args: &Value, status: i32) {
    record_log(
        &state.pool,
        project_id,
        "agent",
        tool,
        status,
        &log_line(args),
    )
    .await;
}

async fn member(
    state: &AppState,
    operator: &Operator,
    pref: String,
    min: &str,
) -> Result<crate::console::ProjectRow, String> {
    require_member(state, operator, &pref, min)
        .await
        .map_err(|err| err.text().to_string())
}

fn arg(args: &Value, key: &str) -> Result<String, String> {
    args[key]
        .as_str()
        .map(|value| value.to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{key} is required"))
}

struct StoredMessage {
    #[allow(dead_code)]
    id: i64,
    #[allow(dead_code)]
    seq: i32,
    role: String,
    content: String,
    tool_calls: Option<Value>,
    tool_call_id: Option<String>,
    name: Option<String>,
}

struct Inserted {
    id: i64,
    seq: i32,
}

async fn append_message(
    state: &AppState,
    thread_id: Uuid,
    role: &str,
    content: &str,
    tool_calls: Option<&Value>,
    tool_call_id: Option<&str>,
    name: Option<&str>,
) -> anyhow::Result<Inserted> {
    for _ in 0..5 {
        let seq: i32 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(seq), -1) + 1 FROM reactor.agent_messages WHERE thread_id = $1",
        )
        .bind(thread_id)
        .fetch_one(&state.pool)
        .await?;
        let inserted: Result<(i64,), sqlx::Error> = sqlx::query_as(
            "INSERT INTO reactor.agent_messages (thread_id, seq, role, content, tool_calls, tool_call_id, name) \
             VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id",
        )
        .bind(thread_id)
        .bind(seq)
        .bind(role)
        .bind(content)
        .bind(tool_calls)
        .bind(tool_call_id)
        .bind(name)
        .fetch_one(&state.pool)
        .await;
        match inserted {
            Ok((id,)) => return Ok(Inserted { id, seq }),
            Err(sqlx::Error::Database(err)) if err.code().as_deref() == Some("23505") => continue,
            Err(err) => return Err(err.into()),
        }
    }
    anyhow::bail!("could not append message")
}

async fn load_messages(pool: &sqlx::PgPool, thread_id: Uuid) -> anyhow::Result<Vec<StoredMessage>> {
    type StoredRow = (
        i64,
        i32,
        String,
        String,
        Option<Value>,
        Option<String>,
        Option<String>,
    );
    let rows: Vec<StoredRow> = sqlx::query_as(
        "SELECT id, seq, role, content, tool_calls, tool_call_id, name FROM reactor.agent_messages \
         WHERE thread_id = $1 ORDER BY seq",
    )
    .bind(thread_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(id, seq, role, content, tool_calls, tool_call_id, name)| StoredMessage {
                id,
                seq,
                role,
                content,
                tool_calls,
                tool_call_id,
                name,
            },
        )
        .collect())
}

async fn thread_operator(pool: &sqlx::PgPool, thread_id: Uuid) -> anyhow::Result<Uuid> {
    let id: Option<Uuid> =
        sqlx::query_scalar("SELECT operator_id FROM reactor.agent_threads WHERE id = $1")
            .bind(thread_id)
            .fetch_optional(pool)
            .await?;
    id.ok_or_else(|| anyhow::anyhow!("thread not found"))
}

async fn thread_model(pool: &sqlx::PgPool, thread_id: Uuid) -> anyhow::Result<Option<String>> {
    let model: Option<String> =
        sqlx::query_scalar("SELECT model FROM reactor.agent_threads WHERE id = $1")
            .bind(thread_id)
            .fetch_one(pool)
            .await?;
    Ok(model)
}

async fn thread_project(pool: &sqlx::PgPool, thread_id: Uuid) -> anyhow::Result<Option<String>> {
    let pref: Option<String> =
        sqlx::query_scalar("SELECT project_ref FROM reactor.agent_threads WHERE id = $1")
            .bind(thread_id)
            .fetch_one(pool)
            .await?;
    Ok(pref)
}

async fn owned_thread(
    state: &AppState,
    operator_id: Uuid,
    thread_id: Uuid,
) -> Result<(), ApiError> {
    let found: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM reactor.agent_threads WHERE id = $1 AND operator_id = $2",
    )
    .bind(thread_id)
    .bind(operator_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    if found.is_none() {
        return Err(ApiError::not_found());
    }
    Ok(())
}

async fn thread_json(
    state: &AppState,
    operator_id: Uuid,
    thread_id: Uuid,
) -> Result<Value, ApiError> {
    let row: Option<(Uuid, bool, Option<String>)> = sqlx::query_as(
        "SELECT id, running, project_ref FROM reactor.agent_threads WHERE id = $1 AND operator_id = $2",
    )
    .bind(thread_id)
    .bind(operator_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some((id, running, project_ref)) = row else {
        return Err(ApiError::not_found());
    };
    let messages = load_messages(&state.pool, thread_id)
        .await
        .map_err(internal)?;
    let messages: Vec<Value> = messages
        .into_iter()
        .map(|message| {
            json!({
                "role": message.role,
                "content": message.content,
                "name": message.name,
                "tool_calls": message.tool_calls,
            })
        })
        .collect();
    Ok(json!({
        "id": id,
        "running": running,
        "project_ref": project_ref,
        "messages": messages
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secrets_from_tool_results_and_logs() {
        let mut value = json!({
            "password": "s3cret-pass",
            "anon_key": "anon-value",
            "service_key": "service-value",
            "nested": {"password": "s3cret-pass"},
            "name": "notes"
        });
        redact(&mut value);
        let line = log_line(&value);
        assert_eq!(value["password"], "[redacted]");
        assert_eq!(value["anon_key"], "[redacted]");
        assert_eq!(value["service_key"], "[redacted]");
        assert_eq!(value["nested"]["password"], "[redacted]");
        assert_eq!(value["name"], "notes");
        assert!(!line.contains("s3cret-pass"));
        assert!(!line.contains("anon-value"));
        assert!(!line.contains("service-value"));
        assert!(!line.contains("service_key") || line.contains("[redacted]"));
    }

    #[test]
    fn parses_split_tool_call_stream() {
        let mut acc = StreamAccum::default();
        let body = "\
data: {\"choices\":[{\"delta\":{\"content\":\"Working.\"}}]}\n\
\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"function\":{\"name\":\"create_project\",\"arguments\":\"\"}}]}}]}\n\
data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"name\\\":\\\"Notes\\\"}\"}}]}}]}\n\
data: [DONE]\n";
        for line in body.lines() {
            push_sse_line(&mut acc, line);
        }
        assert_eq!(acc.content, "Working.");
        assert_eq!(acc.tool_calls.len(), 1);
        assert_eq!(acc.tool_calls[0].id, "call_1");
        assert_eq!(acc.tool_calls[0].name, "create_project");
        assert_eq!(acc.tool_calls[0].arguments, "{\"name\":\"Notes\"}");
    }

    #[test]
    fn accepts_openrouter_model_ids() {
        assert_eq!(
            clean_model(" anthropic/claude-sonnet-4.6 "),
            Some("anthropic/claude-sonnet-4.6".into())
        );
        assert_eq!(clean_model(""), None);
        assert_eq!(clean_model("has space"), None);
        assert_eq!(clean_model("javascript:alert(1)"), None);
    }

    #[test]
    fn lambda_plans_an_invoke_and_listen_does_not() {
        let id = Uuid::nil();
        match plan_turn_start("lambda", "reactor-platform", id, "tok") {
            TurnStart::Invoke {
                function_name,
                event,
            } => {
                assert_eq!(function_name, "reactor-platform");
                let path = event["rawPath"].as_str().unwrap();
                assert!(path.ends_with("/run"));
                assert!(event["headers"]["authorization"]
                    .as_str()
                    .unwrap()
                    .contains("tok"));
            }
            TurnStart::Local => panic!("lambda must build a self-invoke"),
        }
        assert!(matches!(
            plan_turn_start("listen", "reactor-platform", id, "tok"),
            TurnStart::Local
        ));
    }
}
