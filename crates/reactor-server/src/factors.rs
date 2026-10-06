use crate::console::record_log;
use crate::{
    bearer, internal, issue_session, random_token, resolve_project, token_hash, ApiError, AppState,
};
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use reactor_auth::{seal, unseal};
use serde::Deserialize;
use serde_json::{json, Value};
use totp_rs::{Algorithm, Secret, TOTP};
use uuid::Uuid;
use webauthn_rs::prelude::{
    Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential, Url, Webauthn, WebauthnBuilder,
};

pub fn mount(app: Router<crate::AppCtx>) -> Router<crate::AppCtx> {
    app.route("/auth/v1/factors/totp/start", post(totp_start))
        .route("/auth/v1/factors/totp/confirm", post(totp_confirm))
        .route(
            "/auth/v1/factors/passkey/register/options",
            post(passkey_options),
        )
        .route("/auth/v1/factors/passkey/register", post(passkey_register))
        .route("/auth/v1/factors/recovery/regenerate", post(regenerate))
        .route("/auth/v1/factors/finish", post(finish))
        .route("/auth/v1/factors/totp", post(totp_verify))
        .route(
            "/auth/v1/factors/passkey/options",
            post(passkey_auth_options),
        )
        .route("/auth/v1/factors/passkey/verify", post(passkey_verify))
        .route("/auth/v1/factors/recovery", post(recovery_verify))
}

pub(crate) async fn after_password(
    state: &AppState,
    project_id: Uuid,
    user_id: Uuid,
) -> Result<Response, ApiError> {
    let names = factor_names(state, user_id).await?;
    if names.is_empty() {
        let token = store_login_challenge(state, project_id, user_id, "enroll").await?;
        return Ok(Json(json!({
            "enrollment_required": true,
            "enroll_token": token,
            "factors": names,
        }))
        .into_response());
    }
    let token = store_login_challenge(state, project_id, user_id, "mfa").await?;
    Ok(Json(json!({
        "mfa_required": true,
        "mfa_token": token,
        "factors": names,
    }))
    .into_response())
}

async fn factor_names(state: &AppState, user_id: Uuid) -> Result<Vec<&'static str>, ApiError> {
    let rows: Vec<(String, Option<String>, Option<Value>)> = sqlx::query_as(
        "SELECT kind, secret, credential FROM reactor.user_factors WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let mut names = Vec::new();
    for (kind, secret, credential) in rows {
        if kind == "totp" && secret.is_some() {
            names.push("totp");
        }
        if kind == "passkey" && credential.is_some() {
            names.push("passkey");
        }
    }
    names.sort();
    Ok(names)
}

async fn store_login_challenge(
    state: &AppState,
    project_id: Uuid,
    user_id: Uuid,
    kind: &str,
) -> Result<String, ApiError> {
    let email: String = sqlx::query_scalar("SELECT email FROM reactor.users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&state.pool)
        .await
        .map_err(internal)?;
    let token = random_token();
    sqlx::query(
        "INSERT INTO reactor.auth_challenges \
         (id, project_id, user_id, email, kind, token_hash, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, now() + interval '5 minutes')",
    )
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

enum Actor {
    User,
    Enroll,
}

async fn actor(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(Uuid, Uuid, String, Actor), ApiError> {
    let mut host_only = headers.clone();
    host_only.remove(header::AUTHORIZATION);
    let resolved = resolve_project(state, &host_only, false).await?;
    if let Some(token) = bearer(headers) {
        let row: Option<Uuid> = sqlx::query_scalar(
            "SELECT user_id FROM reactor.auth_challenges \
             WHERE token_hash = $1 AND project_id = $2 AND kind = 'enroll' \
             AND consumed_at IS NULL AND expires_at > now()",
        )
        .bind(token_hash(&token))
        .bind(resolved.project_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(internal)?;
        if let Some(user_id) = row {
            return Ok((resolved.project_id, user_id, resolved.pref, Actor::Enroll));
        }
    }
    let resolved = resolve_project(state, headers, true).await?;
    let user_id = resolved
        .identity
        .as_ref()
        .and_then(|identity| identity.user_id)
        .ok_or_else(|| ApiError::unauthorized("user token required"))?;
    Ok((resolved.project_id, user_id, resolved.pref, Actor::User))
}

fn webauthn(headers: &HeaderMap) -> Result<Webauthn, ApiError> {
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let url = Url::parse(origin)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "origin required for passkeys"))?;
    let Some(rp_id) = url.domain() else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "origin required for passkeys",
        ));
    };
    WebauthnBuilder::new(rp_id, &url)
        .and_then(|builder| builder.rp_name("Reactor").allow_any_port(true).build())
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "this origin cannot use passkeys"))
}

fn totp_for(secret: &str, issuer: &str, email: &str) -> anyhow::Result<TOTP> {
    let bytes = Secret::Encoded(secret.to_string())
        .to_bytes()
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        bytes,
        Some(issuer.to_string()),
        email.into(),
    )
    .map_err(|err| anyhow::anyhow!(err.to_string()))
}

fn totp_matches(secret: &str, email: &str, code: &str) -> bool {
    totp_for(secret, "Reactor", email)
        .ok()
        .and_then(|totp| totp.check_current(code).ok())
        .unwrap_or(false)
}

async fn user_email(state: &AppState, user_id: Uuid) -> Result<String, ApiError> {
    sqlx::query_scalar("SELECT email FROM reactor.users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&state.pool)
        .await
        .map_err(internal)
}

async fn project_name(state: &AppState, project_id: Uuid) -> Result<String, ApiError> {
    let name: String = sqlx::query_scalar("SELECT name FROM reactor.projects WHERE id = $1")
        .bind(project_id)
        .fetch_one(&state.pool)
        .await
        .map_err(internal)?;
    if name.trim().is_empty() {
        Ok("Reactor".into())
    } else {
        Ok(name)
    }
}

async fn save_user_challenge(
    state: &AppState,
    user_id: Uuid,
    kind: &str,
    body: &Value,
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM reactor.user_challenges WHERE user_id = $1 AND kind = $2")
        .bind(user_id)
        .bind(kind)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    sqlx::query(
        "INSERT INTO reactor.user_challenges (id, user_id, kind, state, expires_at) \
         VALUES ($1, $2, $3, $4, now() + interval '15 minutes')",
    )
    .bind(Uuid::new_v4())
    .bind(user_id)
    .bind(kind)
    .bind(body)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(())
}

async fn take_user_challenge(
    state: &AppState,
    user_id: Uuid,
    kind: &str,
) -> Result<Option<Value>, ApiError> {
    let row: Option<Value> = sqlx::query_scalar(
        "SELECT state FROM reactor.user_challenges WHERE user_id = $1 AND kind = $2 AND expires_at > now()",
    )
    .bind(user_id)
    .bind(kind)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    sqlx::query("DELETE FROM reactor.user_challenges WHERE user_id = $1 AND kind = $2")
        .bind(user_id)
        .bind(kind)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(row)
}

async fn totp_start(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let (project_id, user_id, _, _) = actor(&state, &headers).await?;
    let email = user_email(&state, user_id).await?;
    let issuer = project_name(&state, project_id).await?;
    let secret = Secret::generate_secret().to_encoded().to_string();
    let sealed = seal(state.issuer.seal_key(), secret.as_bytes()).map_err(internal)?;
    save_user_challenge(&state, user_id, "totp", &json!({"secret": sealed})).await?;
    let totp = totp_for(&secret, &issuer, &email).map_err(internal)?;
    Ok(Json(json!({
        "secret": secret,
        "otpauth": totp.get_url(),
    })))
}

#[derive(Deserialize)]
struct CodeBody {
    code: String,
    mfa_token: Option<String>,
}

async fn totp_confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CodeBody>,
) -> Result<Json<Value>, ApiError> {
    let (_, user_id, _, _) = actor(&state, &headers).await?;
    let email = user_email(&state, user_id).await?;
    let Some(pending) = take_user_challenge(&state, user_id, "totp").await? else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "start authenticator enrollment first",
        ));
    };
    let sealed = pending
        .get("secret")
        .and_then(|value| value.as_str())
        .ok_or_else(|| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "start authenticator enrollment first",
            )
        })?;
    let secret = String::from_utf8(unseal(state.issuer.seal_key(), sealed).map_err(internal)?)
        .map_err(internal)?;
    if !totp_matches(&secret, &email, body.code.trim()) {
        save_user_challenge(&state, user_id, "totp", &json!({"secret": sealed})).await?;
        return Err(ApiError::unauthorized("invalid code"));
    }
    let stored = seal(state.issuer.seal_key(), secret.as_bytes()).map_err(internal)?;
    sqlx::query(
        "INSERT INTO reactor.user_factors (user_id, kind, secret) VALUES ($1, 'totp', $2) \
         ON CONFLICT (user_id, kind) DO UPDATE SET secret = EXCLUDED.secret",
    )
    .bind(user_id)
    .bind(&stored)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(json!({"ok": true, "totp": true})))
}

async fn passkey_options(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let (_, user_id, _, _) = actor(&state, &headers).await?;
    let email = user_email(&state, user_id).await?;
    let webauthn = webauthn(&headers)?;
    let (challenge, reg_state) = webauthn
        .start_passkey_registration(user_id, &email, &email, None)
        .map_err(internal)?;
    save_user_challenge(
        &state,
        user_id,
        "passkey_reg",
        &serde_json::to_value(&reg_state).map_err(internal)?,
    )
    .await?;
    Ok(Json(serde_json::to_value(&challenge).map_err(internal)?))
}

async fn passkey_register(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let (_, user_id, _, _) = actor(&state, &headers).await?;
    let Some(saved) = take_user_challenge(&state, user_id, "passkey_reg").await? else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "start passkey enrollment first",
        ));
    };
    let reg_state: PasskeyRegistration = serde_json::from_value(saved).map_err(internal)?;
    let credential: RegisterPublicKeyCredential = serde_json::from_value(body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid passkey"))?;
    let webauthn = webauthn(&headers)?;
    let passkey = webauthn
        .finish_passkey_registration(&credential, &reg_state)
        .map_err(|_| ApiError::unauthorized("passkey was not accepted"))?;
    sqlx::query(
        "INSERT INTO reactor.user_factors (user_id, kind, credential) VALUES ($1, 'passkey', $2) \
         ON CONFLICT (user_id, kind) DO UPDATE SET credential = EXCLUDED.credential",
    )
    .bind(user_id)
    .bind(serde_json::to_value(&passkey).map_err(internal)?)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(Json(json!({"ok": true, "passkey": true})))
}

async fn regenerate(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let (_, user_id, _, actor) = actor(&state, &headers).await?;
    if !matches!(actor, Actor::User) {
        return Err(ApiError::unauthorized("user token required"));
    }
    let codes = replace_codes(&state, user_id).await?;
    Ok(Json(json!({"recovery_codes": codes})))
}

async fn finish(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let (project_id, user_id, pref, _) = actor(&state, &headers).await?;
    let names = factor_names(&state, user_id).await?;
    if names.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "enroll an authenticator or a passkey before continuing",
        ));
    }
    let token = bearer(&headers).unwrap_or_default();
    let consumed = sqlx::query(
        "UPDATE reactor.auth_challenges SET consumed_at = now() \
         WHERE token_hash = $1 AND project_id = $2 AND kind = 'enroll' AND consumed_at IS NULL",
    )
    .bind(token_hash(&token))
    .bind(project_id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    if consumed.rows_affected() == 0 {
        return Err(ApiError::unauthorized("invalid or expired token"));
    }
    let codes = replace_codes(&state, user_id).await?;
    let email = user_email(&state, user_id).await?;
    let session = issue_session(&state, project_id, &pref, user_id, email).await?;
    let mut body = serde_json::to_value(session.0).map_err(internal)?;
    body["recovery_codes"] = json!(codes);
    record_log(&state.pool, project_id, "auth", "enroll", 200, "").await;
    Ok(Json(body).into_response())
}

async fn replace_codes(state: &AppState, user_id: Uuid) -> Result<Vec<String>, ApiError> {
    sqlx::query("DELETE FROM reactor.user_recovery_codes WHERE user_id = $1")
        .bind(user_id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    let mut codes = Vec::new();
    for _ in 0..8 {
        let code = recovery_code();
        sqlx::query("INSERT INTO reactor.user_recovery_codes (user_id, code_hash) VALUES ($1, $2)")
            .bind(user_id)
            .bind(token_hash(&code))
            .execute(&state.pool)
            .await
            .map_err(internal)?;
        codes.push(code);
    }
    Ok(codes)
}

fn recovery_code() -> String {
    const ALPH: &[u8] = b"abcdefghijkmnpqrstuvwxyz23456789";
    let mut out = String::with_capacity(10);
    for index in 0..8 {
        if index == 4 {
            out.push('-');
        }
        out.push(ALPH[(rand::random::<u8>() as usize) % ALPH.len()] as char);
    }
    out
}

async fn load_mfa(
    state: &AppState,
    project_id: Uuid,
    token: &str,
) -> Result<(Uuid, Uuid, String), ApiError> {
    let row: Option<(Uuid, Uuid, String, i32)> = sqlx::query_as(
        "SELECT id, user_id, email, attempts FROM reactor.auth_challenges \
         WHERE token_hash = $1 AND project_id = $2 AND kind = 'mfa' \
         AND consumed_at IS NULL AND expires_at > now()",
    )
    .bind(token_hash(token))
    .bind(project_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some((id, user_id, email, _)) = row else {
        return Err(ApiError::unauthorized("invalid or expired token"));
    };
    Ok((id, user_id, email))
}

async fn burn(state: &AppState, challenge_id: Uuid) -> Result<(), ApiError> {
    let attempts: i32 = sqlx::query_scalar(
        "UPDATE reactor.auth_challenges SET attempts = attempts + 1 WHERE id = $1 RETURNING attempts",
    )
    .bind(challenge_id)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    if attempts >= 5 {
        sqlx::query("UPDATE reactor.auth_challenges SET consumed_at = now() WHERE id = $1")
            .bind(challenge_id)
            .execute(&state.pool)
            .await
            .map_err(internal)?;
    }
    Ok(())
}

async fn consume_mfa(state: &AppState, challenge_id: Uuid) -> Result<(), ApiError> {
    let updated = sqlx::query(
        "UPDATE reactor.auth_challenges SET consumed_at = now() WHERE id = $1 AND consumed_at IS NULL",
    )
    .bind(challenge_id)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    if updated.rows_affected() == 0 {
        return Err(ApiError::unauthorized("invalid or expired token"));
    }
    Ok(())
}

async fn session_for(
    state: &AppState,
    headers: &HeaderMap,
    user_id: Uuid,
    email: String,
) -> Result<Response, ApiError> {
    let resolved = resolve_project(state, headers, false).await?;
    let session = issue_session(state, resolved.project_id, &resolved.pref, user_id, email).await?;
    Ok(session.into_response())
}

async fn totp_verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CodeBody>,
) -> Result<Response, ApiError> {
    let resolved = resolve_project(&state, &headers, false).await?;
    let token = body.mfa_token.unwrap_or_default();
    let (id, user_id, email) = load_mfa(&state, resolved.project_id, &token).await?;
    let sealed: Option<String> = sqlx::query_scalar(
        "SELECT secret FROM reactor.user_factors WHERE user_id = $1 AND kind = 'totp'",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some(sealed) = sealed else {
        burn(&state, id).await?;
        return Err(ApiError::unauthorized("invalid code"));
    };
    let secret = String::from_utf8(unseal(state.issuer.seal_key(), &sealed).map_err(internal)?)
        .map_err(internal)?;
    if !totp_matches(&secret, &email, body.code.trim()) {
        burn(&state, id).await?;
        return Err(ApiError::unauthorized("invalid code"));
    }
    consume_mfa(&state, id).await?;
    record_log(&state.pool, resolved.project_id, "auth", "mfa", 200, "").await;
    session_for(&state, &headers, user_id, email).await
}

#[derive(Deserialize)]
struct MfaBody {
    mfa_token: String,
    #[serde(flatten)]
    credential: Value,
}

async fn passkey_auth_options(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MfaBody>,
) -> Result<Json<Value>, ApiError> {
    let resolved = resolve_project(&state, &headers, false).await?;
    let (id, user_id, _) = load_mfa(&state, resolved.project_id, &body.mfa_token).await?;
    let stored: Option<Value> = sqlx::query_scalar(
        "SELECT credential FROM reactor.user_factors WHERE user_id = $1 AND kind = 'passkey'",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some(stored) = stored else {
        burn(&state, id).await?;
        return Err(ApiError::unauthorized("invalid passkey"));
    };
    let passkey: Passkey = serde_json::from_value(stored).map_err(internal)?;
    let webauthn = webauthn(&headers)?;
    let (challenge, auth_state) = webauthn
        .start_passkey_authentication(&[passkey])
        .map_err(internal)?;
    save_user_challenge(
        &state,
        user_id,
        "passkey_auth",
        &serde_json::to_value(&auth_state).map_err(internal)?,
    )
    .await?;
    Ok(Json(serde_json::to_value(&challenge).map_err(internal)?))
}

async fn passkey_verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MfaBody>,
) -> Result<Response, ApiError> {
    let resolved = resolve_project(&state, &headers, false).await?;
    let (id, user_id, email) = load_mfa(&state, resolved.project_id, &body.mfa_token).await?;
    let Some(saved) = take_user_challenge(&state, user_id, "passkey_auth").await? else {
        burn(&state, id).await?;
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "start passkey verification first",
        ));
    };
    let auth_state: PasskeyAuthentication = serde_json::from_value(saved).map_err(internal)?;
    let mut credential_json = body.credential;
    if let Some(map) = credential_json.as_object_mut() {
        map.remove("mfa_token");
    }
    let credential: PublicKeyCredential = serde_json::from_value(credential_json)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid passkey"))?;
    let webauthn = webauthn(&headers)?;
    let result = match webauthn.finish_passkey_authentication(&credential, &auth_state) {
        Ok(result) => result,
        Err(_) => {
            burn(&state, id).await?;
            return Err(ApiError::unauthorized("passkey was not accepted"));
        }
    };
    if let Some(stored) = sqlx::query_scalar::<_, Value>(
        "SELECT credential FROM reactor.user_factors WHERE user_id = $1 AND kind = 'passkey'",
    )
    .bind(user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    {
        if let Ok(mut passkey) = serde_json::from_value::<Passkey>(stored) {
            if passkey.update_credential(&result) == Some(true) {
                let _ = sqlx::query(
                    "UPDATE reactor.user_factors SET credential = $2 WHERE user_id = $1 AND kind = 'passkey'",
                )
                .bind(user_id)
                .bind(serde_json::to_value(&passkey).unwrap_or(Value::Null))
                .execute(&state.pool)
                .await;
            }
        }
    }
    consume_mfa(&state, id).await?;
    record_log(&state.pool, resolved.project_id, "auth", "mfa", 200, "").await;
    session_for(&state, &headers, user_id, email).await
}

async fn recovery_verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CodeBody>,
) -> Result<Response, ApiError> {
    let resolved = resolve_project(&state, &headers, false).await?;
    let token = body.mfa_token.unwrap_or_default();
    let (id, user_id, email) = load_mfa(&state, resolved.project_id, &token).await?;
    let updated = sqlx::query(
        "UPDATE reactor.user_recovery_codes SET used_at = now() \
         WHERE user_id = $1 AND code_hash = $2 AND used_at IS NULL",
    )
    .bind(user_id)
    .bind(token_hash(body.code.trim()))
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    if updated.rows_affected() == 0 {
        burn(&state, id).await?;
        return Err(ApiError::unauthorized("invalid code"));
    }
    consume_mfa(&state, id).await?;
    record_log(&state.pool, resolved.project_id, "auth", "mfa", 200, "").await;
    session_for(&state, &headers, user_id, email).await
}
