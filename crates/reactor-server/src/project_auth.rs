use crate::console::{require_member, require_operator, site_public_url};
use crate::email::{deliver_code, DeliverError};
use crate::{
    internal, issue_session, random_token, resolve_project, token_hash, ApiError, AppState,
    TokenOut,
};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

pub fn mount(app: Router<AppState>) -> Router<AppState> {
    app.route("/auth/v1/verify-email", post(verify_email))
        .route("/auth/v1/verify-email/send", post(resend))
}

pub fn mount_console(app: Router<AppState>) -> Router<AppState> {
    app.route(
        "/console/v1/projects/{pref}/auth",
        get(get_settings).put(put_settings),
    )
    .route(
        "/console/v1/projects/{pref}/auth/providers",
        get(crate::oauth::list_providers).post(crate::oauth::put_provider),
    )
    .route(
        "/console/v1/projects/{pref}/auth/providers/{provider}",
        axum::routing::delete(crate::oauth::delete_provider),
    )
}

pub(crate) async fn insert_settings(
    pool: &sqlx::PgPool,
    project_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO reactor.auth_settings (project_id) VALUES ($1) ON CONFLICT (project_id) DO NOTHING",
    )
    .bind(project_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn flags(state: &AppState, project_id: Uuid) -> Result<(bool, bool), ApiError> {
    let row: Option<(bool, bool)> = sqlx::query_as(
        "SELECT require_email_verification, require_mfa FROM reactor.auth_settings WHERE project_id = $1",
    )
    .bind(project_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    Ok(row.unwrap_or((true, false)))
}

pub(crate) async fn mark_verified(state: &AppState, user_id: Uuid) -> Result<(), ApiError> {
    sqlx::query(
        "UPDATE reactor.users SET email_verified_at = COALESCE(email_verified_at, now()) WHERE id = $1",
    )
    .bind(user_id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(())
}

pub(crate) fn pending(user_id: Uuid, email: &str) -> Value {
    json!({
        "verification_required": true,
        "user": { "id": user_id, "email": email }
    })
}

pub(crate) async fn send_confirm(
    state: &AppState,
    project_id: Uuid,
    user_id: Uuid,
    email: &str,
) -> Result<(), DeliverError> {
    let _ = sqlx::query(
        "DELETE FROM reactor.auth_challenges \
         WHERE project_id = $1 AND email = $2 AND kind = 'confirm_email' AND consumed_at IS NULL",
    )
    .bind(project_id)
    .bind(email)
    .execute(&state.pool)
    .await
    .map_err(|err| {
        tracing::error!("{err}");
        DeliverError::Failed
    })?;
    let token = random_token();
    let code = format!("{:06}", rand::random::<u32>() % 1_000_000);
    sqlx::query(
        "INSERT INTO reactor.auth_challenges \
         (id, project_id, user_id, email, kind, token_hash, code_hash, expires_at) \
         VALUES ($1, $2, $3, $4, 'confirm_email', $5, $6, now() + interval '24 hours')",
    )
    .bind(Uuid::new_v4())
    .bind(project_id)
    .bind(user_id)
    .bind(email)
    .bind(token_hash(&token))
    .bind(token_hash(&code))
    .execute(&state.pool)
    .await
    .map_err(|err| {
        tracing::error!("{err}");
        DeliverError::Failed
    })?;
    if let Err(err) = deliver_code(state, project_id, "confirm_email", email, &token, &code).await {
        let _ = sqlx::query("DELETE FROM reactor.auth_challenges WHERE token_hash = $1")
            .bind(token_hash(&token))
            .execute(&state.pool)
            .await;
        return Err(err);
    }
    Ok(())
}

async fn recent(state: &AppState, project_id: Uuid, email: &str) -> Result<bool, ApiError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM reactor.auth_challenges \
         WHERE project_id = $1 AND email = $2 AND kind = 'confirm_email' \
         AND created_at > now() - interval '60 seconds'",
    )
    .bind(project_id)
    .bind(email)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    Ok(count > 0)
}

#[derive(Deserialize)]
struct VerifyBody {
    token: Option<String>,
    email: Option<String>,
    code: Option<String>,
}

async fn verify_email(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<VerifyBody>,
) -> Result<Response, ApiError> {
    let resolved = resolve_project(&state, &headers, false).await?;
    if let Some(token) = body.token.filter(|token| !token.is_empty()) {
        let row: Option<(Uuid, String)> = sqlx::query_as(
            "UPDATE reactor.auth_challenges SET consumed_at = now() \
             WHERE token_hash = $1 AND project_id = $2 AND kind = 'confirm_email' \
             AND consumed_at IS NULL AND expires_at > now() \
             RETURNING user_id, email",
        )
        .bind(token_hash(&token))
        .bind(resolved.project_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?;
        let Some((user_id, email)) = row else {
            return Err(ApiError::unauthorized("invalid or expired token"));
        };
        mark_verified(&state, user_id).await?;
        let session =
            issue_session(&state, resolved.project_id, &resolved.pref, user_id, email).await?;
        return Ok(session.into_response());
    }
    let email = body.email.unwrap_or_default().trim().to_ascii_lowercase();
    let code = body.code.unwrap_or_default();
    if email.is_empty() || code.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "token or code required",
        ));
    }
    finish_code(&state, resolved.project_id, &resolved.pref, &email, &code).await
}

async fn finish_code(
    state: &AppState,
    project_id: Uuid,
    pref: &str,
    email: &str,
    code: &str,
) -> Result<Response, ApiError> {
    let row: Option<(Uuid, Uuid, i32)> = sqlx::query_as(
        "SELECT id, user_id, attempts FROM reactor.auth_challenges \
         WHERE project_id = $1 AND email = $2 AND kind = 'confirm_email' \
         AND consumed_at IS NULL AND expires_at > now() \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(project_id)
    .bind(email)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some((id, user_id, attempts)) = row else {
        return Err(ApiError::unauthorized("invalid or expired token"));
    };
    let matched: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM reactor.auth_challenges WHERE id = $1 AND code_hash = $2",
    )
    .bind(id)
    .bind(token_hash(code))
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    if matched.is_none() {
        let next = attempts + 1;
        if next >= 5 {
            sqlx::query("UPDATE reactor.auth_challenges SET attempts = $2, consumed_at = now() WHERE id = $1")
                .bind(id)
                .bind(next)
                .execute(&state.pool)
                .await
                .map_err(internal)?;
        } else {
            sqlx::query("UPDATE reactor.auth_challenges SET attempts = $2 WHERE id = $1")
                .bind(id)
                .bind(next)
                .execute(&state.pool)
                .await
                .map_err(internal)?;
        }
        return Err(ApiError::unauthorized("invalid or expired token"));
    }
    let consumed = sqlx::query(
        "UPDATE reactor.auth_challenges SET consumed_at = now() WHERE id = $1 AND consumed_at IS NULL",
    )
    .bind(id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    if consumed.rows_affected() == 0 {
        return Err(ApiError::unauthorized("invalid or expired token"));
    }
    mark_verified(state, user_id).await?;
    let session: Json<TokenOut> =
        issue_session(state, project_id, pref, user_id, email.to_string()).await?;
    Ok(session.into_response())
}

#[derive(Deserialize)]
struct EmailBody {
    email: String,
}

async fn resend(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<EmailBody>,
) -> Result<Json<Value>, ApiError> {
    let resolved = resolve_project(&state, &headers, false).await?;
    crate::limit_auth(&state, &headers, resolved.project_id, "confirm_email", 30).await?;
    let email = body.email.trim().to_ascii_lowercase();
    if recent(&state, resolved.project_id, &email).await? {
        return Ok(Json(json!({"ok": true})));
    }
    let row: Option<(Uuid, bool)> = sqlx::query_as(
        "SELECT id, email_verified_at IS NOT NULL FROM reactor.users WHERE project_id = $1 AND email = $2",
    )
    .bind(resolved.project_id)
    .bind(&email)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some((user_id, verified)) = row else {
        return Ok(Json(json!({"ok": true})));
    };
    if verified {
        return Ok(Json(json!({"ok": true})));
    }
    if send_confirm(&state, resolved.project_id, user_id, &email)
        .await
        .is_err()
    {
        return Ok(Json(json!({"ok": true})));
    }
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct SettingsBody {
    require_email_verification: bool,
    require_mfa: bool,
}

async fn get_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    let (require_email_verification, require_mfa) = flags(&state, project.id).await?;
    let origin = site_public_url(
        &project.pref,
        &state.config.base_domain,
        &state.config.public_url,
    );
    Ok(Json(json!({
        "require_email_verification": require_email_verification,
        "require_mfa": require_mfa,
        "callback_base": format!("{origin}/auth/v1/callback"),
    })))
}

async fn put_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
    Json(body): Json<SettingsBody>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    sqlx::query(
        "INSERT INTO reactor.auth_settings (project_id, require_email_verification, require_mfa) \
         VALUES ($1, $2, $3) \
         ON CONFLICT (project_id) DO UPDATE SET \
         require_email_verification = EXCLUDED.require_email_verification, \
         require_mfa = EXCLUDED.require_mfa",
    )
    .bind(project.id)
    .bind(body.require_email_verification)
    .bind(body.require_mfa)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    get_settings(State(state), headers, Path(pref)).await
}
