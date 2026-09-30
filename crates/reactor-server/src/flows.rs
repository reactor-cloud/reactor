use crate::console::record_log;
use crate::email::{deliver, DeliverError};
use crate::{
    internal, issue_session, random_token, require_service, resolve_project, token_hash, ApiError,
    AppState,
};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn mount(app: Router<AppState>) -> Router<AppState> {
    app.route("/auth/v1/magic-link", post(magic_link))
        .route("/auth/v1/verify", post(verify))
        .route("/auth/v1/recover", post(recover))
        .route("/auth/v1/recover/complete", post(recover_complete))
        .route("/auth/v1/invite", post(invite))
        .route("/auth/v1/invite/accept", post(invite_accept))
}

fn normalize_email(email: &str) -> Result<String, ApiError> {
    let email = email.trim().to_ascii_lowercase();
    if email.len() < 3 || email.len() > 320 || email.contains(' ') || !email.contains('@') {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid email"));
    }
    Ok(email)
}

async fn recent(
    state: &AppState,
    project_id: Uuid,
    email: &str,
    kind: &str,
) -> Result<bool, ApiError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM reactor.auth_challenges \
         WHERE project_id = $1 AND email = $2 AND kind = $3 AND consumed_at IS NULL \
         AND created_at > now() - interval '60 seconds'",
    )
    .bind(project_id)
    .bind(email)
    .bind(kind)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    Ok(count > 0)
}

async fn insert_challenge(
    state: &AppState,
    project_id: Uuid,
    user_id: Uuid,
    email: &str,
    kind: &str,
    days: bool,
) -> Result<String, ApiError> {
    let token = random_token();
    let expires = if days {
        "now() + interval '7 days'"
    } else {
        "now() + interval '15 minutes'"
    };
    sqlx::query(&format!(
        "INSERT INTO reactor.auth_challenges (id, project_id, user_id, email, kind, token_hash, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, {expires})"
    ))
    .bind(Uuid::new_v4())
    .bind(project_id)
    .bind(user_id)
    .bind(email)
    .bind(kind)
    .bind(token_hash(&token))
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(token)
}

async fn find_user(
    state: &AppState,
    project_id: Uuid,
    email: &str,
) -> Result<Option<Uuid>, ApiError> {
    sqlx::query_scalar("SELECT id FROM reactor.users WHERE project_id = $1 AND email = $2")
        .bind(project_id)
        .bind(email)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)
}

async fn create_user(state: &AppState, project_id: Uuid, email: &str) -> Result<Uuid, ApiError> {
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO reactor.users (id, project_id, email, password_hash) VALUES ($1, $2, $3, NULL)")
        .bind(id)
        .bind(project_id)
        .bind(email)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(id)
}

async fn delete_user(state: &AppState, user_id: Uuid) {
    let _ = sqlx::query("DELETE FROM reactor.users WHERE id = $1")
        .bind(user_id)
        .execute(&state.pool)
        .await;
}

async fn delete_token(state: &AppState, token: &str) {
    let _ = sqlx::query("DELETE FROM reactor.auth_challenges WHERE token_hash = $1")
        .bind(token_hash(token))
        .execute(&state.pool)
        .await;
}

#[derive(Deserialize)]
struct EmailBody {
    email: String,
}

async fn magic_link(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<EmailBody>,
) -> Result<Json<Value>, ApiError> {
    let resolved = resolve_project(&state, &headers, false).await?;
    crate::limit_auth(&state, &headers, resolved.project_id, "magic_link", 30).await?;
    let email = normalize_email(&body.email)?;
    if recent(&state, resolved.project_id, &email, "magic_link").await? {
        return Ok(Json(json!({"ok": true})));
    }
    let existing = find_user(&state, resolved.project_id, &email).await?;
    let (user_id, created) = match existing {
        Some(id) => (id, false),
        None => (
            create_user(&state, resolved.project_id, &email).await?,
            true,
        ),
    };
    let token = match insert_challenge(
        &state,
        resolved.project_id,
        user_id,
        &email,
        "magic_link",
        false,
    )
    .await
    {
        Ok(token) => token,
        Err(err) => {
            if created {
                delete_user(&state, user_id).await;
            }
            return Err(err);
        }
    };
    if let Err(err) = deliver(&state, resolved.project_id, "magic_link", &email, &token).await {
        delete_token(&state, &token).await;
        if created {
            delete_user(&state, user_id).await;
        }
        if matches!(err, DeliverError::Failed) {
            record_log(
                &state.pool,
                resolved.project_id,
                "auth",
                "magic_link",
                200,
                "send failed",
            )
            .await;
        }
    } else {
        record_log(
            &state.pool,
            resolved.project_id,
            "auth",
            "magic_link",
            200,
            "",
        )
        .await;
    }
    Ok(Json(json!({"ok": true})))
}

async fn recover(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<EmailBody>,
) -> Result<Json<Value>, ApiError> {
    let resolved = resolve_project(&state, &headers, false).await?;
    crate::limit_auth(&state, &headers, resolved.project_id, "recover", 30).await?;
    let email = normalize_email(&body.email)?;
    if recent(&state, resolved.project_id, &email, "recovery").await? {
        return Ok(Json(json!({"ok": true})));
    }
    let Some(user_id) = find_user(&state, resolved.project_id, &email).await? else {
        return Ok(Json(json!({"ok": true})));
    };
    let token = insert_challenge(
        &state,
        resolved.project_id,
        user_id,
        &email,
        "recovery",
        false,
    )
    .await?;
    if deliver(&state, resolved.project_id, "recovery", &email, &token)
        .await
        .is_err()
    {
        delete_token(&state, &token).await;
    }
    record_log(&state.pool, resolved.project_id, "auth", "recover", 200, "").await;
    Ok(Json(json!({"ok": true})))
}

async fn invite(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<EmailBody>,
) -> Result<Json<Value>, ApiError> {
    let resolved = resolve_project(&state, &headers, true).await?;
    require_service(&resolved)?;
    crate::limit_auth(&state, &headers, resolved.project_id, "invite", 30).await?;
    let email = normalize_email(&body.email)?;
    if find_user(&state, resolved.project_id, &email)
        .await?
        .is_some()
    {
        return Err(ApiError::new(StatusCode::CONFLICT, "email exists"));
    }
    let user_id = create_user(&state, resolved.project_id, &email).await?;
    let token = match insert_challenge(&state, resolved.project_id, user_id, &email, "invite", true)
        .await
    {
        Ok(token) => token,
        Err(err) => {
            delete_user(&state, user_id).await;
            return Err(err);
        }
    };
    match deliver(&state, resolved.project_id, "invite", &email, &token).await {
        Ok(()) => {
            record_log(&state.pool, resolved.project_id, "auth", "invite", 200, "").await;
            Ok(Json(json!({"ok": true})))
        }
        Err(DeliverError::Unconfigured) => {
            delete_token(&state, &token).await;
            delete_user(&state, user_id).await;
            Err(ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "email is not configured",
            ))
        }
        Err(DeliverError::Failed) => {
            delete_token(&state, &token).await;
            delete_user(&state, user_id).await;
            Err(ApiError::new(
                StatusCode::BAD_GATEWAY,
                "email could not be sent",
            ))
        }
    }
}

#[derive(Deserialize)]
struct TokenBody {
    token: String,
}

async fn verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<TokenBody>,
) -> Result<Json<crate::TokenOut>, ApiError> {
    let resolved = resolve_project(&state, &headers, false).await?;
    let row = consume(&state, resolved.project_id, &body.token, "magic_link").await?;
    crate::project_auth::mark_verified(&state, row.0).await?;
    record_log(&state.pool, resolved.project_id, "auth", "verify", 200, "").await;
    issue_session(&state, resolved.project_id, &resolved.pref, row.0, row.1).await
}

#[derive(Deserialize)]
struct PasswordTokenBody {
    token: String,
    password: String,
}

async fn recover_complete(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PasswordTokenBody>,
) -> Result<Json<crate::TokenOut>, ApiError> {
    complete(&state, &headers, body, "recovery").await
}

async fn invite_accept(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PasswordTokenBody>,
) -> Result<Json<crate::TokenOut>, ApiError> {
    complete(&state, &headers, body, "invite").await
}

async fn complete(
    state: &AppState,
    headers: &HeaderMap,
    body: PasswordTokenBody,
    kind: &str,
) -> Result<Json<crate::TokenOut>, ApiError> {
    if body.password.len() < 8 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "password too short"));
    }
    let resolved = resolve_project(state, headers, false).await?;
    let (user_id, email) = consume(state, resolved.project_id, &body.token, kind).await?;
    let hash = crate::hash_password(&body.password).map_err(internal)?;
    sqlx::query("UPDATE reactor.users SET password_hash = $1 WHERE id = $2 AND project_id = $3")
        .bind(&hash)
        .bind(user_id)
        .bind(resolved.project_id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    crate::project_auth::mark_verified(state, user_id).await?;
    record_log(&state.pool, resolved.project_id, "auth", kind, 200, "").await;
    issue_session(state, resolved.project_id, &resolved.pref, user_id, email).await
}

async fn consume(
    state: &AppState,
    project_id: Uuid,
    token: &str,
    kind: &str,
) -> Result<(Uuid, String), ApiError> {
    let row: Option<(Uuid, String)> = sqlx::query_as(
        "UPDATE reactor.auth_challenges SET consumed_at = now() \
         WHERE token_hash = $1 AND project_id = $2 AND kind = $3 AND consumed_at IS NULL AND expires_at > now() \
         RETURNING user_id, email",
    )
    .bind(token_hash(token))
    .bind(project_id)
    .bind(kind)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    row.ok_or_else(|| ApiError::unauthorized("invalid or expired token"))
}
