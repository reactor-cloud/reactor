use crate::console::{load_operator, Operator};
use crate::{bearer, internal, ApiError, AppState};
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::routing::post;
use axum::{Json, Router};
use reactor_auth::{seal, token_hash, unseal};
use serde::Deserialize;
use serde_json::{json, Value};
use totp_rs::{Algorithm, Secret, TOTP};
use uuid::Uuid;
use webauthn_rs::prelude::{
    Passkey, PasskeyAuthentication, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential, Url, Webauthn, WebauthnBuilder,
};

pub fn mount(app: Router<AppState>) -> Router<AppState> {
    app.route("/console/v1/mfa/totp/start", post(totp_start))
        .route("/console/v1/mfa/totp/confirm", post(totp_confirm))
        .route("/console/v1/mfa/totp/reset", post(totp_reset))
        .route(
            "/console/v1/mfa/passkey/register/options",
            post(passkey_register_options),
        )
        .route("/console/v1/mfa/passkey/register", post(passkey_register))
        .route("/console/v1/mfa/finish", post(finish))
        .route("/console/v1/mfa/passkey/options", post(passkey_options))
        .route("/console/v1/mfa/passkey/verify", post(passkey_verify))
        .route("/console/v1/mfa/totp", post(totp_verify))
        .route("/console/v1/mfa/recovery", post(recovery_verify))
}

pub(crate) async fn password_gate(
    state: &AppState,
    id: Uuid,
    email: &str,
    name: &str,
    platform_admin: bool,
) -> Result<Json<Value>, ApiError> {
    let operator = json!({
        "id": id,
        "email": email,
        "name": name,
        "platform_admin": platform_admin,
    });
    if !needs_mfa(state, id, platform_admin).await? {
        let token = state
            .issuer
            .sign_console(&id.to_string(), 60 * 60 * 12)
            .map_err(internal)?;
        return Ok(Json(json!({"access_token": token, "operator": operator})));
    }
    let (totp, passkey) = factors(state, id).await?;
    if !(totp && passkey) {
        let token = state
            .issuer
            .sign_audience(&id.to_string(), "console-setup", 15 * 60)
            .map_err(internal)?;
        return Ok(Json(json!({
            "enrollment_required": true,
            "setup_token": token,
            "operator": operator,
        })));
    }
    let token = state
        .issuer
        .sign_audience(&id.to_string(), "console-mfa", 5 * 60)
        .map_err(internal)?;
    Ok(Json(json!({
        "mfa_required": true,
        "mfa_token": token,
        "operator": operator,
    })))
}

async fn needs_mfa(state: &AppState, id: Uuid, platform_admin: bool) -> Result<bool, ApiError> {
    if platform_admin {
        return Ok(true);
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM reactor.memberships WHERE operator_id = $1 AND role IN ('owner', 'admin')",
    )
    .bind(id)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    Ok(count > 0)
}

async fn factors(state: &AppState, id: Uuid) -> Result<(bool, bool), ApiError> {
    let kinds: Vec<String> =
        sqlx::query_scalar("SELECT kind FROM reactor.operator_factors WHERE operator_id = $1")
            .bind(id)
            .fetch_all(&state.pool)
            .await
            .map_err(internal)?;
    Ok((
        kinds.iter().any(|kind| kind == "totp"),
        kinds.iter().any(|kind| kind == "passkey"),
    ))
}

pub(crate) async fn setup_operator(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<Operator, ApiError> {
    audience_operator(state, headers, "console-setup").await
}

async fn mfa_operator(state: &AppState, headers: &HeaderMap) -> Result<Operator, ApiError> {
    audience_operator(state, headers, "console-mfa").await
}

async fn audience_operator(
    state: &AppState,
    headers: &HeaderMap,
    aud: &str,
) -> Result<Operator, ApiError> {
    let token = bearer(headers).ok_or_else(|| ApiError::unauthorized("console token required"))?;
    let claims = state
        .issuer
        .decode(&token)
        .map_err(|_| ApiError::unauthorized("console token required"))?;
    if claims.aud != aud {
        return Err(ApiError::unauthorized("console token required"));
    }
    let id = Uuid::parse_str(&claims.sub)
        .map_err(|_| ApiError::unauthorized("console token required"))?;
    load_operator(state, id).await
}

fn webauthn(headers: &HeaderMap) -> Result<Webauthn, ApiError> {
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let url = Url::parse(origin).map_err(|_| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            "open the console in a browser to use a passkey",
        )
    })?;
    let Some(rp_id) = url.domain() else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "open the console on a hostname such as localhost to use a passkey",
        ));
    };
    WebauthnBuilder::new(rp_id, &url)
        .and_then(|builder| builder.rp_name("Reactor").allow_any_port(true).build())
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "this origin cannot use passkeys"))
}

async fn save_challenge(
    state: &AppState,
    operator_id: Uuid,
    kind: &str,
    body: &Value,
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM reactor.operator_challenges WHERE operator_id = $1 AND kind = $2")
        .bind(operator_id)
        .bind(kind)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    sqlx::query(
        "INSERT INTO reactor.operator_challenges (id, operator_id, kind, state, expires_at) \
         VALUES ($1, $2, $3, $4, now() + interval '15 minutes')",
    )
    .bind(Uuid::new_v4())
    .bind(operator_id)
    .bind(kind)
    .bind(body)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    Ok(())
}

async fn read_challenge(
    state: &AppState,
    operator_id: Uuid,
    kind: &str,
) -> Result<Option<Value>, ApiError> {
    sqlx::query_scalar(
        "SELECT state FROM reactor.operator_challenges WHERE operator_id = $1 AND kind = $2 AND expires_at > now()",
    )
    .bind(operator_id)
    .bind(kind)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)
}

async fn take_challenge(
    state: &AppState,
    operator_id: Uuid,
    kind: &str,
) -> Result<Option<Value>, ApiError> {
    let state_json = read_challenge(state, operator_id, kind).await?;
    sqlx::query("DELETE FROM reactor.operator_challenges WHERE operator_id = $1 AND kind = $2")
        .bind(operator_id)
        .bind(kind)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(state_json)
}

fn issuer_label(cluster: &str) -> String {
    let name = cluster
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(':', " ");
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        "Reactor".to_string()
    } else {
        name
    }
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

async fn cluster_issuer(state: &AppState) -> Result<String, ApiError> {
    let name: Option<String> =
        sqlx::query_scalar("SELECT cluster_name FROM reactor.settings WHERE id = 1")
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    Ok(issuer_label(name.as_deref().unwrap_or("")))
}

async fn stored_totp_secret(state: &AppState, id: Uuid) -> Result<Option<String>, ApiError> {
    let sealed: Option<String> = sqlx::query_scalar(
        "SELECT secret FROM reactor.operator_factors WHERE operator_id = $1 AND kind = 'totp'",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some(sealed) = sealed else {
        return Ok(None);
    };
    let secret = String::from_utf8(unseal(state.issuer.seal_key(), &sealed).map_err(internal)?)
        .map_err(internal)?;
    Ok(Some(secret))
}

async fn can_restart(state: &AppState, operator: &Operator) -> Result<bool, ApiError> {
    if !operator.platform_admin {
        return Ok(false);
    }
    let projects: i64 = sqlx::query_scalar("SELECT count(*) FROM reactor.projects")
        .fetch_one(&state.pool)
        .await
        .map_err(internal)?;
    Ok(projects == 0)
}

fn fresh_secret() -> String {
    Secret::generate_secret().to_encoded().to_string()
}

async fn totp_start(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let operator = setup_operator(&state, &headers).await?;
    let (_, passkey) = factors(&state, operator.id).await?;
    let restart = can_restart(&state, &operator).await?;
    if stored_totp_secret(&state, operator.id).await?.is_some() {
        sqlx::query(
            "DELETE FROM reactor.operator_challenges WHERE operator_id = $1 AND kind = 'totp'",
        )
        .bind(operator.id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
        return Ok(Json(json!({
            "enrolled": true,
            "passkey": passkey,
            "can_restart": restart,
        })));
    }
    let issuer = cluster_issuer(&state).await?;
    if let Some(pending) = read_challenge(&state, operator.id, "totp").await? {
        if let Some(sealed) = pending.get("secret").and_then(|value| value.as_str()) {
            if let Ok(secret) = unseal(state.issuer.seal_key(), sealed) {
                if let Ok(secret) = String::from_utf8(secret) {
                    let totp = totp_for(&secret, &issuer, &operator.email).map_err(internal)?;
                    return Ok(Json(json!({
                        "enrolled": false,
                        "secret": secret,
                        "otpauth": totp.get_url(),
                        "passkey": passkey,
                        "can_restart": restart,
                    })));
                }
            }
        }
    }
    let (secret, otpauth) = new_totp(&state, &operator).await?;
    Ok(Json(json!({
        "enrolled": false,
        "secret": secret,
        "otpauth": otpauth,
        "passkey": passkey,
        "can_restart": restart,
    })))
}

async fn new_totp(state: &AppState, operator: &Operator) -> Result<(String, String), ApiError> {
    let secret = fresh_secret();
    let sealed = seal(state.issuer.seal_key(), secret.as_bytes()).map_err(internal)?;
    save_challenge(state, operator.id, "totp", &json!({"secret": sealed})).await?;
    let issuer = cluster_issuer(state).await?;
    let totp = totp_for(&secret, &issuer, &operator.email).map_err(internal)?;
    Ok((secret, totp.get_url()))
}

async fn totp_reset(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let operator = setup_operator(&state, &headers).await?;
    let (_, passkey) = factors(&state, operator.id).await?;
    if passkey {
        return Err(ApiError::new(
            StatusCode::CONFLICT,
            "the authenticator cannot be reset after a passkey is saved",
        ));
    }
    sqlx::query("DELETE FROM reactor.operator_factors WHERE operator_id = $1 AND kind = 'totp'")
        .bind(operator.id)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    let (secret, otpauth) = new_totp(&state, &operator).await?;
    let restart = can_restart(&state, &operator).await?;
    Ok(Json(json!({
        "enrolled": false,
        "secret": secret,
        "otpauth": otpauth,
        "passkey": false,
        "can_restart": restart,
    })))
}

#[derive(Deserialize)]
struct CodeBody {
    code: String,
}

async fn totp_confirm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CodeBody>,
) -> Result<Json<Value>, ApiError> {
    let operator = setup_operator(&state, &headers).await?;
    if let Some(secret) = stored_totp_secret(&state, operator.id).await? {
        if !totp_matches(&secret, &operator.email, body.code.trim()) {
            return Err(ApiError::unauthorized("invalid code"));
        }
        let (_, passkey) = factors(&state, operator.id).await?;
        return Ok(Json(json!({"ok": true, "totp": true, "passkey": passkey})));
    }
    let Some(pending) = take_challenge(&state, operator.id, "totp").await? else {
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
    if !totp_matches(&secret, &operator.email, body.code.trim()) {
        save_challenge(&state, operator.id, "totp", &json!({"secret": sealed})).await?;
        return Err(ApiError::unauthorized("invalid code"));
    }
    let stored = seal(state.issuer.seal_key(), secret.as_bytes()).map_err(internal)?;
    sqlx::query(
        "INSERT INTO reactor.operator_factors (operator_id, kind, secret) VALUES ($1, 'totp', $2) \
         ON CONFLICT (operator_id, kind) DO UPDATE SET secret = EXCLUDED.secret",
    )
    .bind(operator.id)
    .bind(&stored)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    let (_, passkey) = factors(&state, operator.id).await?;
    Ok(Json(json!({"ok": true, "totp": true, "passkey": passkey})))
}

fn totp_matches(secret: &str, email: &str, code: &str) -> bool {
    totp_for(secret, "Reactor", email)
        .ok()
        .and_then(|totp| totp.check_current(code).ok())
        .unwrap_or(false)
}

async fn passkey_register_options(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let operator = setup_operator(&state, &headers).await?;
    let webauthn = webauthn(&headers)?;
    let display = if operator.name.is_empty() {
        operator.email.as_str()
    } else {
        operator.name.as_str()
    };
    let (challenge, reg_state) = webauthn
        .start_passkey_registration(operator.id, &operator.email, display, None)
        .map_err(internal)?;
    save_challenge(
        &state,
        operator.id,
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
    let operator = setup_operator(&state, &headers).await?;
    let Some(saved) = take_challenge(&state, operator.id, "passkey_reg").await? else {
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
        "INSERT INTO reactor.operator_factors (operator_id, kind, credential) VALUES ($1, 'passkey', $2) \
         ON CONFLICT (operator_id, kind) DO UPDATE SET credential = EXCLUDED.credential",
    )
    .bind(operator.id)
    .bind(serde_json::to_value(&passkey).map_err(internal)?)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    let (totp, _) = factors(&state, operator.id).await?;
    Ok(Json(json!({"ok": true, "totp": totp, "passkey": true})))
}

async fn finish(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let operator = setup_operator(&state, &headers).await?;
    let (totp, passkey) = factors(&state, operator.id).await?;
    if !(totp && passkey) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "enroll a passkey and an authenticator before continuing",
        ));
    }
    let existing: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM reactor.operator_recovery_codes WHERE operator_id = $1",
    )
    .bind(operator.id)
    .fetch_one(&state.pool)
    .await
    .map_err(internal)?;
    let mut codes = Vec::new();
    if existing == 0 {
        for _ in 0..8 {
            let code = recovery_code();
            sqlx::query("INSERT INTO reactor.operator_recovery_codes (operator_id, code_hash) VALUES ($1, $2)")
                .bind(operator.id)
                .bind(token_hash(&code))
                .execute(&state.pool)
                .await
                .map_err(internal)?;
            codes.push(code);
        }
    }
    let token = state
        .issuer
        .sign_console(&operator.id.to_string(), 60 * 60 * 12)
        .map_err(internal)?;
    Ok(Json(json!({
        "access_token": token,
        "recovery_codes": codes,
        "operator": {
            "id": operator.id,
            "email": operator.email,
            "name": operator.name,
            "platform_admin": operator.platform_admin,
        }
    })))
}

async fn passkey_options(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let operator = mfa_operator(&state, &headers).await?;
    let stored: Option<Value> = sqlx::query_scalar(
        "SELECT credential FROM reactor.operator_factors WHERE operator_id = $1 AND kind = 'passkey'",
    )
    .bind(operator.id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some(stored) = stored else {
        return Err(ApiError::unauthorized("passkey is not enrolled"));
    };
    let passkey: Passkey = serde_json::from_value(stored).map_err(internal)?;
    let webauthn = webauthn(&headers)?;
    let (challenge, auth_state) = webauthn
        .start_passkey_authentication(&[passkey])
        .map_err(internal)?;
    save_challenge(
        &state,
        operator.id,
        "passkey_auth",
        &serde_json::to_value(&auth_state).map_err(internal)?,
    )
    .await?;
    Ok(Json(serde_json::to_value(&challenge).map_err(internal)?))
}

async fn passkey_verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    let operator = mfa_operator(&state, &headers).await?;
    let Some(saved) = take_challenge(&state, operator.id, "passkey_auth").await? else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "start passkey sign-in first",
        ));
    };
    let auth_state: PasskeyAuthentication = serde_json::from_value(saved).map_err(internal)?;
    let credential: PublicKeyCredential = serde_json::from_value(body)
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid passkey"))?;
    let webauthn = webauthn(&headers)?;
    let result = webauthn
        .finish_passkey_authentication(&credential, &auth_state)
        .map_err(|_| ApiError::unauthorized("passkey was not accepted"))?;
    if let Some(stored) = sqlx::query_scalar::<_, Value>(
        "SELECT credential FROM reactor.operator_factors WHERE operator_id = $1 AND kind = 'passkey'",
    )
    .bind(operator.id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?
    {
        if let Ok(mut passkey) = serde_json::from_value::<Passkey>(stored) {
            if passkey.update_credential(&result) == Some(true) {
                let _ = sqlx::query(
                    "UPDATE reactor.operator_factors SET credential = $1 WHERE operator_id = $2 AND kind = 'passkey'",
                )
                .bind(serde_json::to_value(&passkey).unwrap_or(Value::Null))
                .bind(operator.id)
                .execute(&state.pool)
                .await;
            }
        }
    }
    session_json(&state, &operator)
}

async fn totp_verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CodeBody>,
) -> Result<Json<Value>, ApiError> {
    let operator = mfa_operator(&state, &headers).await?;
    let Some(secret) = stored_totp_secret(&state, operator.id).await? else {
        return Err(ApiError::unauthorized("invalid code"));
    };
    if !totp_matches(&secret, &operator.email, body.code.trim()) {
        return Err(ApiError::unauthorized("invalid code"));
    }
    session_json(&state, &operator)
}

async fn recovery_verify(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<CodeBody>,
) -> Result<Json<Value>, ApiError> {
    let operator = mfa_operator(&state, &headers).await?;
    let hashes: Vec<String> = sqlx::query_scalar(
        "SELECT code_hash FROM reactor.operator_recovery_codes WHERE operator_id = $1 AND used_at IS NULL",
    )
    .bind(operator.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let presented = token_hash(body.code.trim());
    let mut matched = false;
    for hash in &hashes {
        if reactor_auth::constant_time_eq(hash, &presented) {
            matched = true;
        }
    }
    if !matched {
        return Err(ApiError::unauthorized("invalid code"));
    }
    sqlx::query(
        "UPDATE reactor.operator_recovery_codes SET used_at = now() WHERE operator_id = $1 AND code_hash = $2 AND used_at IS NULL",
    )
    .bind(operator.id)
    .bind(&presented)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    session_json(&state, &operator)
}

fn session_json(state: &AppState, operator: &Operator) -> Result<Json<Value>, ApiError> {
    let token = state
        .issuer
        .sign_console(&operator.id.to_string(), 60 * 60 * 12)
        .map_err(internal)?;
    Ok(Json(json!({
        "access_token": token,
        "operator": {
            "id": operator.id,
            "email": operator.email,
            "name": operator.name,
            "platform_admin": operator.platform_admin,
        }
    })))
}

fn recovery_code() -> String {
    reactor_auth::random_token()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(12)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{fresh_secret, issuer_label, totp_for, totp_matches};

    #[test]
    fn authenticator_code_matches_its_secret() {
        let secret = fresh_secret();
        let totp = totp_for(&secret, "Lab", "a@b.co").unwrap();
        let url = totp.get_url();
        assert!(url.contains("issuer=Lab"), "{url}");
        assert!(url.contains("otpauth://totp/Lab:"), "{url}");
        let code = totp.generate_current().unwrap();
        assert!(totp_matches(&secret, "a@b.co", &code));
    }

    #[test]
    fn cluster_label_has_no_colon() {
        assert_eq!(issuer_label("Lab: West"), "Lab West");
        assert_eq!(issuer_label("   "), "Reactor");
    }
}
