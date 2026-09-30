use crate::console::{record_log, require_member, require_operator, require_platform_admin};
use crate::{internal, ApiError, AppState};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use reactor_auth::{seal, unseal};
use reactor_email::{auth_link, fill, reserved, reserved_name, Mail};
use reactor_identity::ProjectRef;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn mount(app: Router<AppState>) -> Router<AppState> {
    app.route(
        "/console/v1/cluster/email",
        get(get_cluster).put(put_cluster),
    )
    .route(
        "/console/v1/projects/{pref}/email/cluster",
        put(put_cluster_flag),
    )
    .route(
        "/console/v1/projects/{pref}/email",
        get(get_settings).put(put_settings),
    )
    .route("/console/v1/projects/{pref}/email/test", post(test_send))
    .route(
        "/console/v1/projects/{pref}/email/templates",
        get(list_templates),
    )
    .route(
        "/console/v1/projects/{pref}/email/templates/{name}",
        get(get_template).put(put_template).delete(delete_template),
    )
    .route(
        "/console/v1/projects/{pref}/email/templates/{name}/reset",
        post(reset_template),
    )
}

pub(crate) enum DeliverError {
    Unconfigured,
    Failed,
}

pub(crate) async fn deliver(
    state: &AppState,
    project_id: Uuid,
    template: &str,
    to: &str,
    token: &str,
) -> Result<(), DeliverError> {
    deliver_code(state, project_id, template, to, token, "").await
}

pub(crate) async fn deliver_code(
    state: &AppState,
    project_id: Uuid,
    template: &str,
    to: &str,
    token: &str,
    code: &str,
) -> Result<(), DeliverError> {
    let Some(settings) = resolve_mail(&state.pool, project_id).await.map_err(|err| {
        tracing::error!("{err}");
        DeliverError::Failed
    })?
    else {
        return Err(DeliverError::Unconfigured);
    };
    let (subject, text, html) = template_body(&state.pool, project_id, template)
        .await
        .map_err(|err| {
            tracing::error!("{err}");
            DeliverError::Failed
        })?;
    let link = auth_link(&settings.link_base, token);
    let code = code.to_string();
    let subject = fill(&subject, to, token, &link, &code);
    let text = fill(&text, to, token, &link, &code);
    let html = fill(&html, to, token, &link, &code);
    let mail = open_mail(state.issuer.seal_key(), &settings).map_err(|err| {
        tracing::error!("{err}");
        DeliverError::Failed
    })?;
    let to = to.to_string();
    let sent = tokio::task::spawn_blocking(move || {
        reactor_email::send(&mail, &to, &subject, &text, &html)
    })
    .await
    .map_err(|err| {
        tracing::error!("{err}");
        DeliverError::Failed
    })?
    .map_err(|err| {
        tracing::error!("{err}");
        DeliverError::Failed
    });
    match sent {
        Ok(()) => {
            record_log(&state.pool, project_id, "email", template, 200, "").await;
            Ok(())
        }
        Err(err) => {
            record_log(
                &state.pool,
                project_id,
                "email",
                template,
                502,
                "send failed",
            )
            .await;
            Err(err)
        }
    }
}

struct SettingsRow {
    host: String,
    port: i32,
    username: String,
    password_enc: String,
    from_address: String,
    tls: String,
    link_base: String,
}

struct ClusterRow {
    host: String,
    port: i32,
    username: String,
    password_enc: String,
    from_address: String,
    tls: String,
}

fn open_mail(key: &[u8; 32], settings: &SettingsRow) -> Result<Mail, String> {
    let password = if settings.password_enc.is_empty() {
        String::new()
    } else {
        String::from_utf8(unseal(key, &settings.password_enc).map_err(|err| err.to_string())?)
            .map_err(|err| err.to_string())?
    };
    Ok(Mail {
        host: settings.host.clone(),
        port: settings.port as u16,
        username: settings.username.clone(),
        password,
        from: settings.from_address.clone(),
        tls: settings.tls.clone(),
    })
}

async fn resolve_mail(
    pool: &sqlx::PgPool,
    project_id: Uuid,
) -> Result<Option<SettingsRow>, sqlx::Error> {
    let project = load_settings(pool, project_id).await?;
    if project
        .as_ref()
        .is_some_and(|row| !row.host.trim().is_empty())
    {
        return Ok(project);
    }
    let allowed: Option<bool> =
        sqlx::query_scalar("SELECT cluster_smtp FROM reactor.projects WHERE id = $1")
            .bind(project_id)
            .fetch_optional(pool)
            .await?;
    if allowed != Some(true) {
        return Ok(None);
    }
    let Some(cluster) = load_cluster(pool).await? else {
        return Ok(None);
    };
    if cluster.host.trim().is_empty() {
        return Ok(None);
    }
    Ok(Some(SettingsRow {
        host: cluster.host,
        port: cluster.port,
        username: cluster.username,
        password_enc: cluster.password_enc,
        from_address: cluster.from_address,
        tls: cluster.tls,
        link_base: project.map(|row| row.link_base).unwrap_or_default(),
    }))
}

async fn load_settings(
    pool: &sqlx::PgPool,
    project_id: Uuid,
) -> Result<Option<SettingsRow>, sqlx::Error> {
    sqlx::query_as::<_, (String, i32, String, String, String, String, String)>(
        "SELECT host, port, username, password_enc, from_address, tls, link_base FROM reactor.email_settings WHERE project_id = $1",
    )
    .bind(project_id)
    .fetch_optional(pool)
    .await
    .map(|row| {
        row.map(|(host, port, username, password_enc, from_address, tls, link_base)| SettingsRow {
            host,
            port,
            username,
            password_enc,
            from_address,
            tls,
            link_base,
        })
    })
}

async fn load_cluster(pool: &sqlx::PgPool) -> Result<Option<ClusterRow>, sqlx::Error> {
    sqlx::query_as::<_, (String, i32, String, String, String, String)>(
        "SELECT host, port, username, password_enc, from_address, tls FROM reactor.cluster_email WHERE id = 1",
    )
    .fetch_optional(pool)
    .await
    .map(|row| {
        row.map(|(host, port, username, password_enc, from_address, tls)| ClusterRow {
            host,
            port,
            username,
            password_enc,
            from_address,
            tls,
        })
    })
}

fn settings_json(row: Option<&SettingsRow>) -> Value {
    let Some(row) = row else {
        return json!({
            "host": "",
            "port": 587,
            "username": "",
            "password_set": false,
            "from_address": "",
            "tls": "starttls",
            "link_base": "",
        });
    };
    json!({
        "host": row.host,
        "port": row.port,
        "username": row.username,
        "password_set": !row.password_enc.is_empty(),
        "from_address": row.from_address,
        "tls": row.tls,
        "link_base": row.link_base,
    })
}

async fn project_mail_json(
    pool: &sqlx::PgPool,
    project_id: Uuid,
    row: Option<&SettingsRow>,
) -> Result<Value, sqlx::Error> {
    let cluster_smtp: bool =
        sqlx::query_scalar("SELECT cluster_smtp FROM reactor.projects WHERE id = $1")
            .bind(project_id)
            .fetch_one(pool)
            .await?;
    let cluster = load_cluster(pool).await?;
    let project_host = row.map(|item| item.host.as_str()).unwrap_or("");
    let cluster_host = cluster
        .as_ref()
        .map(|item| item.host.as_str())
        .unwrap_or("");
    let source = if !project_host.is_empty() {
        "project"
    } else if cluster_smtp && !cluster_host.is_empty() {
        "cluster"
    } else {
        "none"
    };
    let cluster_from = if source == "cluster" {
        cluster
            .as_ref()
            .map(|item| item.from_address.as_str())
            .unwrap_or("")
    } else {
        ""
    };
    let mut value = settings_json(row);
    value["cluster_smtp"] = json!(cluster_smtp);
    value["source"] = json!(source);
    value["cluster_from"] = json!(cluster_from);
    Ok(value)
}

fn cluster_json(row: Option<&ClusterRow>) -> Value {
    let Some(row) = row else {
        return json!({
            "host": "",
            "port": 587,
            "username": "",
            "password_set": false,
            "from_address": "",
            "tls": "starttls",
        });
    };
    json!({
        "host": row.host,
        "port": row.port,
        "username": row.username,
        "password_set": !row.password_enc.is_empty(),
        "from_address": row.from_address,
        "tls": row.tls,
    })
}

async fn get_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    let row = load_settings(&state.pool, project.id)
        .await
        .map_err(internal)?;
    Ok(Json(
        project_mail_json(&state.pool, project.id, row.as_ref())
            .await
            .map_err(internal)?,
    ))
}

#[derive(Deserialize)]
struct SettingsBody {
    host: String,
    port: i32,
    username: String,
    password: Option<String>,
    from_address: String,
    tls: String,
    link_base: String,
}

async fn put_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
    Json(body): Json<SettingsBody>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    let host = body.host.trim();
    let link_base = body.link_base.trim();
    if host.is_empty() {
        let port = if (1..=65535).contains(&body.port) {
            body.port
        } else {
            587
        };
        let tls = if matches!(body.tls.as_str(), "starttls" | "tls" | "none") {
            body.tls.as_str()
        } else {
            "starttls"
        };
        sqlx::query(
            "INSERT INTO reactor.email_settings (project_id, host, port, username, password_enc, from_address, tls, link_base) \
             VALUES ($1, '', $2, '', '', '', $3, $4) \
             ON CONFLICT (project_id) DO UPDATE SET host = '', port = EXCLUDED.port, username = '', \
             password_enc = '', from_address = '', tls = EXCLUDED.tls, link_base = EXCLUDED.link_base",
        )
        .bind(project.id)
        .bind(port)
        .bind(tls)
        .bind(link_base)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    } else {
        let from_address = body.from_address.trim();
        if from_address.is_empty() || !(1..=65535).contains(&body.port) {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "host, from address, and a valid port are required",
            ));
        }
        if !matches!(body.tls.as_str(), "starttls" | "tls" | "none") {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "tls must be starttls, tls, or none",
            ));
        }
        let existing = load_settings(&state.pool, project.id)
            .await
            .map_err(internal)?;
        let password_enc = match body
            .password
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(password) => {
                seal(state.issuer.seal_key(), password.as_bytes()).map_err(internal)?
            }
            None => existing
                .as_ref()
                .map(|row| row.password_enc.clone())
                .unwrap_or_default(),
        };
        sqlx::query(
            "INSERT INTO reactor.email_settings (project_id, host, port, username, password_enc, from_address, tls, link_base) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) \
             ON CONFLICT (project_id) DO UPDATE SET host = EXCLUDED.host, port = EXCLUDED.port, username = EXCLUDED.username, \
             password_enc = EXCLUDED.password_enc, from_address = EXCLUDED.from_address, tls = EXCLUDED.tls, link_base = EXCLUDED.link_base",
        )
        .bind(project.id)
        .bind(host)
        .bind(body.port)
        .bind(body.username.trim())
        .bind(&password_enc)
        .bind(from_address)
        .bind(&body.tls)
        .bind(link_base)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    }
    let row = load_settings(&state.pool, project.id)
        .await
        .map_err(internal)?;
    Ok(Json(
        project_mail_json(&state.pool, project.id, row.as_ref())
            .await
            .map_err(internal)?,
    ))
}

#[derive(Deserialize)]
struct TestBody {
    to: String,
}

async fn test_send(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
    Json(body): Json<TestBody>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    let to = body.to.trim();
    if to.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "recipient is required",
        ));
    }
    let Some(settings) = resolve_mail(&state.pool, project.id)
        .await
        .map_err(internal)?
    else {
        return Err(ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "email is not configured",
        ));
    };
    let mail = open_mail(state.issuer.seal_key(), &settings).map_err(internal)?;
    let to = to.to_string();
    let result = tokio::task::spawn_blocking(move || {
        reactor_email::send(
            &mail,
            &to,
            "Reactor test",
            "This is a test message from Reactor.\n",
            "<p>This is a test message from Reactor.</p>",
        )
    })
    .await
    .map_err(internal)?;
    if let Err(err) = result {
        tracing::error!("{err}");
        record_log(&state.pool, project.id, "email", "test", 502, "send failed").await;
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "email could not be sent",
        ));
    }
    record_log(&state.pool, project.id, "email", "test", 200, "").await;
    Ok(Json(json!({"ok": true})))
}

async fn get_cluster(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let _operator = require_platform_admin(&state, &headers).await?;
    let row = load_cluster(&state.pool).await.map_err(internal)?;
    Ok(Json(cluster_json(row.as_ref())))
}

#[derive(Deserialize)]
struct ClusterBody {
    host: String,
    port: i32,
    username: String,
    password: Option<String>,
    from_address: String,
    tls: String,
}

async fn put_cluster(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ClusterBody>,
) -> Result<Json<Value>, ApiError> {
    let _operator = require_platform_admin(&state, &headers).await?;
    let host = body.host.trim();
    let from_address = body.from_address.trim();
    if host.is_empty() || from_address.is_empty() || !(1..=65535).contains(&body.port) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "host, from address, and a valid port are required",
        ));
    }
    if !matches!(body.tls.as_str(), "starttls" | "tls" | "none") {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "tls must be starttls, tls, or none",
        ));
    }
    let existing = load_cluster(&state.pool).await.map_err(internal)?;
    let password_enc = match body
        .password
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(password) => seal(state.issuer.seal_key(), password.as_bytes()).map_err(internal)?,
        None => existing
            .as_ref()
            .map(|row| row.password_enc.clone())
            .unwrap_or_default(),
    };
    sqlx::query(
        "INSERT INTO reactor.cluster_email (id, host, port, username, password_enc, from_address, tls) \
         VALUES (1, $1, $2, $3, $4, $5, $6) \
         ON CONFLICT (id) DO UPDATE SET host = EXCLUDED.host, port = EXCLUDED.port, username = EXCLUDED.username, \
         password_enc = EXCLUDED.password_enc, from_address = EXCLUDED.from_address, tls = EXCLUDED.tls",
    )
    .bind(host)
    .bind(body.port)
    .bind(body.username.trim())
    .bind(&password_enc)
    .bind(from_address)
    .bind(&body.tls)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    let row = load_cluster(&state.pool).await.map_err(internal)?;
    Ok(Json(cluster_json(row.as_ref())))
}

#[derive(Deserialize)]
struct ClusterFlag {
    enabled: bool,
}

async fn put_cluster_flag(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
    Json(body): Json<ClusterFlag>,
) -> Result<Json<Value>, ApiError> {
    let _operator = require_platform_admin(&state, &headers).await?;
    let pref = ProjectRef::parse(&pref)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid ref"))?;
    let id: Option<Uuid> = sqlx::query_scalar("SELECT id FROM reactor.projects WHERE ref = $1")
        .bind(pref.as_str())
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?;
    let Some(id) = id else {
        return Err(ApiError::not_found());
    };
    sqlx::query("UPDATE reactor.projects SET cluster_smtp = $1 WHERE id = $2")
        .bind(body.enabled)
        .bind(id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(Json(json!({"enabled": body.enabled})))
}

async fn seed_reserved(pool: &sqlx::PgPool, project_id: Uuid) -> Result<(), sqlx::Error> {
    for template in reserved() {
        sqlx::query(
            "INSERT INTO reactor.email_templates (project_id, name, subject, body_html, body_text) \
             VALUES ($1, $2, $3, $4, $5) ON CONFLICT (project_id, name) DO NOTHING",
        )
        .bind(project_id)
        .bind(template.name)
        .bind(template.subject)
        .bind(template.body_html)
        .bind(template.body_text)
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn template_body(
    pool: &sqlx::PgPool,
    project_id: Uuid,
    name: &str,
) -> Result<(String, String, String), sqlx::Error> {
    seed_reserved(pool, project_id).await?;
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT subject, body_text, body_html FROM reactor.email_templates WHERE project_id = $1 AND name = $2",
    )
    .bind(project_id)
    .bind(name)
    .fetch_optional(pool)
    .await?;
    row.ok_or(sqlx::Error::RowNotFound)
}

fn valid_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && bytes[0].is_ascii_lowercase()
        && bytes
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'_')
}

async fn list_templates(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    seed_reserved(&state.pool, project.id)
        .await
        .map_err(internal)?;
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, subject FROM reactor.email_templates WHERE project_id = $1 ORDER BY name",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let templates: Vec<Value> = rows
        .into_iter()
        .map(|(name, subject)| json!({"name": name, "subject": subject, "reserved": reserved_name(&name)}))
        .collect();
    Ok(Json(json!(templates)))
}

async fn get_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    if !valid_name(&name) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid template name",
        ));
    }
    let row = template_body(&state.pool, project.id, &name).await;
    let Ok((subject, body_text, body_html)) = row else {
        return Err(ApiError::not_found());
    };
    Ok(Json(json!({
        "name": name,
        "subject": subject,
        "body_text": body_text,
        "body_html": body_html,
        "reserved": reserved_name(&name),
    })))
}

#[derive(Deserialize)]
struct TemplateBody {
    subject: String,
    body_text: String,
    body_html: String,
}

async fn put_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, name)): Path<(String, String)>,
    Json(body): Json<TemplateBody>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    if !valid_name(&name) || body.subject.trim().is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "a template name and subject are required",
        ));
    }
    sqlx::query(
        "INSERT INTO reactor.email_templates (project_id, name, subject, body_html, body_text, updated_at) \
         VALUES ($1, $2, $3, $4, $5, now()) \
         ON CONFLICT (project_id, name) DO UPDATE SET subject = EXCLUDED.subject, body_html = EXCLUDED.body_html, \
         body_text = EXCLUDED.body_text, updated_at = now()",
    )
    .bind(project.id)
    .bind(&name)
    .bind(body.subject.trim())
    .bind(&body.body_html)
    .bind(&body.body_text)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(json!({
        "name": name,
        "subject": body.subject.trim(),
        "body_text": body.body_text,
        "body_html": body.body_html,
        "reserved": reserved_name(&name),
    })))
}

async fn delete_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, name)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    if reserved_name(&name) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "this template is used by auth and cannot be deleted",
        ));
    }
    sqlx::query("DELETE FROM reactor.email_templates WHERE project_id = $1 AND name = $2")
        .bind(project.id)
        .bind(&name)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn reset_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, name)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    let Some(template) = reserved().iter().find(|template| template.name == name) else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "only auth templates can be reset",
        ));
    };
    sqlx::query(
        "INSERT INTO reactor.email_templates (project_id, name, subject, body_html, body_text, updated_at) \
         VALUES ($1, $2, $3, $4, $5, now()) \
         ON CONFLICT (project_id, name) DO UPDATE SET subject = EXCLUDED.subject, body_html = EXCLUDED.body_html, \
         body_text = EXCLUDED.body_text, updated_at = now()",
    )
    .bind(project.id)
    .bind(template.name)
    .bind(template.subject)
    .bind(template.body_html)
    .bind(template.body_text)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(json!({
        "name": template.name,
        "subject": template.subject,
        "body_text": template.body_text,
        "body_html": template.body_html,
        "reserved": true,
    })))
}
