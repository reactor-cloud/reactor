use crate::console::record_log;
use crate::email::{deliver, deliver_code, DeliverError};
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

pub fn mount(app: Router<crate::AppCtx>) -> Router<crate::AppCtx> {
    app.route("/auth/v1/magic-link", post(magic_link))
        .route("/auth/v1/verify", post(verify))
        .route("/auth/v1/recover", post(recover))
        .route("/auth/v1/recover/complete", post(recover_complete))
        .route("/auth/v1/invite", post(invite))
        .route("/auth/v1/invite/accept", post(invite_accept))
        .route("/auth/v1/otp", post(otp_send))
        .route("/auth/v1/otp/verify", post(otp_verify))
}

const OTP_EMAIL_HOUR: i64 = 5;
const OTP_EMAIL_DAY: i64 = 10;
const OTP_PROJECT_HOUR: i64 = 100;
const OTP_MAX_ATTEMPTS: i32 = 5;
const OTP_EMAIL_FAILURES_HOUR: i64 = 10;

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

fn too_many() -> ApiError {
    ApiError::new(StatusCode::TOO_MANY_REQUESTS, "too many requests")
}

fn invalid_code() -> ApiError {
    ApiError::new(StatusCode::BAD_REQUEST, "invalid or expired code")
}

async fn lock_otp(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    project_id: Uuid,
) -> Result<(), ApiError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended('otp:' || $1::text, 0))")
        .bind(project_id)
        .execute(&mut **tx)
        .await
        .map_err(internal)?;
    Ok(())
}

async fn otp_send(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<EmailBody>,
) -> Result<Json<Value>, ApiError> {
    let resolved = resolve_project(&state, &headers, false).await?;
    crate::limit_auth(&state, &headers, resolved.project_id, "otp", 20).await?;
    let email = normalize_email(&body.email)?;
    let Some((id, code)) = hold_otp(&state, resolved.project_id, &email).await? else {
        return Ok(Json(json!({"ok": true})));
    };
    match deliver_code(&state, resolved.project_id, "otp", &email, "", &code).await {
        Ok(()) => {
            sqlx::query(
                "UPDATE reactor.auth_challenges SET consumed_at = now() \
                 WHERE project_id = $1 AND email = $2 AND kind = 'otp' AND consumed_at IS NULL AND id <> $3",
            )
            .bind(resolved.project_id)
            .bind(&email)
            .bind(id)
            .execute(&state.pool)
            .await
            .map_err(internal)?;
            record_log(&state.pool, resolved.project_id, "auth", "otp", 200, "").await;
        }
        Err(err) => {
            let _ = sqlx::query("DELETE FROM reactor.auth_challenges WHERE id = $1")
                .bind(id)
                .execute(&state.pool)
                .await;
            let message = match err {
                DeliverError::Unconfigured => "email is not configured",
                DeliverError::Failed => "send failed",
            };
            record_log(
                &state.pool,
                resolved.project_id,
                "auth",
                "otp",
                200,
                message,
            )
            .await;
        }
    }
    Ok(Json(json!({"ok": true})))
}

async fn hold_otp(
    state: &AppState,
    project_id: Uuid,
    email: &str,
) -> Result<Option<(Uuid, String)>, ApiError> {
    let mut tx = state.pool.begin().await.map_err(internal)?;
    lock_otp(&mut tx, project_id).await?;
    let (minute, hour, day): (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE created_at > now() - interval '60 seconds'), \
         count(*) FILTER (WHERE created_at > now() - interval '1 hour'), \
         count(*) \
         FROM reactor.auth_challenges \
         WHERE project_id = $1 AND email = $2 AND kind = 'otp' AND created_at > now() - interval '1 day'",
    )
    .bind(project_id)
    .bind(email)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    if minute > 0 {
        return Ok(None);
    }
    if hour >= OTP_EMAIL_HOUR || day >= OTP_EMAIL_DAY {
        record_log(&state.pool, project_id, "auth", "otp", 429, "email cap").await;
        return Err(too_many());
    }
    let recent: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM reactor.auth_challenges \
         WHERE project_id = $1 AND kind = 'otp' AND created_at > now() - interval '1 hour'",
    )
    .bind(project_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    if recent >= OTP_PROJECT_HOUR {
        record_log(&state.pool, project_id, "auth", "otp", 429, "project cap").await;
        return Err(too_many());
    }
    let id = Uuid::new_v4();
    let code = format!("{:06}", rand::random::<u32>() % 1_000_000);
    sqlx::query(
        "INSERT INTO reactor.auth_challenges \
         (id, project_id, user_id, email, kind, token_hash, code_hash, expires_at) \
         VALUES ($1, $2, NULL, $3, 'otp', $4, $5, now() + interval '10 minutes')",
    )
    .bind(id)
    .bind(project_id)
    .bind(email)
    .bind(token_hash(&random_token()))
    .bind(token_hash(&code))
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(Some((id, code)))
}

#[derive(Deserialize)]
struct OtpBody {
    email: String,
    code: String,
}

async fn otp_verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<OtpBody>,
) -> Result<Json<crate::TokenOut>, ApiError> {
    let resolved = resolve_project(&state, &headers, false).await?;
    crate::limit_auth(&state, &headers, resolved.project_id, "otp_verify", 30).await?;
    let email = normalize_email(&body.email)?;
    let user_id = match check_otp(&state, resolved.project_id, &email, body.code.trim()).await {
        Ok(user_id) => user_id,
        Err(err) => {
            record_log(
                &state.pool,
                resolved.project_id,
                "auth",
                "otp_verify",
                err.status.as_u16() as i32,
                &err.message,
            )
            .await;
            return Err(err);
        }
    };
    record_log(
        &state.pool,
        resolved.project_id,
        "auth",
        "otp_verify",
        200,
        "",
    )
    .await;
    issue_session(&state, resolved.project_id, &resolved.pref, user_id, email).await
}

async fn check_otp(
    state: &AppState,
    project_id: Uuid,
    email: &str,
    code: &str,
) -> Result<Uuid, ApiError> {
    let mut tx = state.pool.begin().await.map_err(internal)?;
    lock_otp(&mut tx, project_id).await?;
    let failures: i64 = sqlx::query_scalar(
        "SELECT COALESCE(sum(attempts), 0)::bigint FROM reactor.auth_challenges \
         WHERE project_id = $1 AND email = $2 AND kind = 'otp' AND created_at > now() - interval '1 hour'",
    )
    .bind(project_id)
    .bind(email)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    if failures >= OTP_EMAIL_FAILURES_HOUR {
        return Err(too_many());
    }
    let row: Option<(Uuid, Option<String>, i32)> = sqlx::query_as(
        "SELECT id, code_hash, attempts FROM reactor.auth_challenges \
         WHERE project_id = $1 AND email = $2 AND kind = 'otp' \
         AND consumed_at IS NULL AND expires_at > now() \
         ORDER BY created_at DESC LIMIT 1 FOR UPDATE",
    )
    .bind(project_id)
    .bind(email)
    .fetch_optional(&mut *tx)
    .await
    .map_err(internal)?;
    let Some((id, code_hash, attempts)) = row else {
        return Err(invalid_code());
    };
    if code_hash.as_deref() != Some(token_hash(code).as_str()) {
        let next = attempts + 1;
        sqlx::query(
            "UPDATE reactor.auth_challenges SET attempts = $2, \
             consumed_at = CASE WHEN $2 >= $3 THEN now() ELSE consumed_at END WHERE id = $1",
        )
        .bind(id)
        .bind(next)
        .bind(OTP_MAX_ATTEMPTS)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
        tx.commit().await.map_err(internal)?;
        return Err(invalid_code());
    }
    sqlx::query(
        "INSERT INTO reactor.users (id, project_id, email, password_hash, email_verified_at) \
         VALUES ($1, $2, $3, NULL, now()) ON CONFLICT (project_id, email) DO NOTHING",
    )
    .bind(Uuid::new_v4())
    .bind(project_id)
    .bind(email)
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    let user_id: Uuid = sqlx::query_scalar(
        "UPDATE reactor.users SET email_verified_at = COALESCE(email_verified_at, now()) \
         WHERE project_id = $1 AND email = $2 RETURNING id",
    )
    .bind(project_id)
    .bind(email)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;
    sqlx::query(
        "UPDATE reactor.auth_challenges SET consumed_at = now(), user_id = $2 WHERE id = $1",
    )
    .bind(id)
    .bind(user_id)
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(user_id)
}
