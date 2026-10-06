use crate::console::{require_member, require_operator, site_public_url};
use crate::project_auth::mark_verified;
use crate::{
    internal, issue_session, random_token, resolve_project, token_hash, ApiError, AppState,
};
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::{Form, Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use jsonwebtoken::{encode, EncodingKey, Header};
use reactor_auth::{seal, unseal};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

struct Spec {
    id: &'static str,
    authorize: &'static str,
    token: &'static str,
    userinfo: &'static str,
    scopes: &'static str,
}

fn catalog() -> &'static [Spec] {
    &[
        Spec {
            id: "google",
            authorize: "https://accounts.google.com/o/oauth2/v2/auth",
            token: "https://oauth2.googleapis.com/token",
            userinfo: "https://openidconnect.googleapis.com/v1/userinfo",
            scopes: "openid email",
        },
        Spec {
            id: "microsoft",
            authorize: "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize",
            token: "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token",
            userinfo: "https://graph.microsoft.com/oidc/userinfo",
            scopes: "openid email",
        },
        Spec {
            id: "apple",
            authorize: "https://appleid.apple.com/auth/authorize",
            token: "https://appleid.apple.com/auth/token",
            userinfo: "",
            scopes: "name email",
        },
        Spec {
            id: "github",
            authorize: "https://github.com/login/oauth/authorize",
            token: "https://github.com/login/oauth/access_token",
            userinfo: "https://api.github.com/user",
            scopes: "user:email",
        },
        Spec {
            id: "facebook",
            authorize: "https://www.facebook.com/v19.0/dialog/oauth",
            token: "https://graph.facebook.com/v19.0/oauth/access_token",
            userinfo: "https://graph.facebook.com/me?fields=id,email",
            scopes: "email",
        },
        Spec {
            id: "discord",
            authorize: "https://discord.com/api/oauth2/authorize",
            token: "https://discord.com/api/oauth2/token",
            userinfo: "https://discord.com/api/users/@me",
            scopes: "identify email",
        },
        Spec {
            id: "x",
            authorize: "https://twitter.com/i/oauth2/authorize",
            token: "https://api.twitter.com/2/oauth2/token",
            userinfo: "https://api.twitter.com/2/users/me",
            scopes: "users.read tweet.read",
        },
        Spec {
            id: "linkedin",
            authorize: "https://www.linkedin.com/oauth/v2/authorization",
            token: "https://www.linkedin.com/oauth/v2/accessToken",
            userinfo: "https://api.linkedin.com/v2/userinfo",
            scopes: "openid email",
        },
        Spec {
            id: "slack",
            authorize: "https://slack.com/oauth/v2/authorize",
            token: "https://slack.com/api/oauth.v2.access",
            userinfo: "",
            scopes: "openid email",
        },
        Spec {
            id: "gitlab",
            authorize: "https://gitlab.com/oauth/authorize",
            token: "https://gitlab.com/oauth/token",
            userinfo: "https://gitlab.com/api/v4/user",
            scopes: "read_user",
        },
    ]
}

fn find_spec(id: &str) -> Option<&'static Spec> {
    catalog().iter().find(|spec| spec.id == id)
}

pub fn mount(app: Router<crate::AppCtx>) -> Router<crate::AppCtx> {
    app.route("/auth/v1/authorize", get(authorize)).route(
        "/auth/v1/callback/{provider}",
        get(callback_get).post(callback_post),
    )
}

pub(crate) async fn exchange_code(
    state: &AppState,
    headers: &HeaderMap,
    code: &str,
) -> Result<Response, ApiError> {
    let resolved = resolve_project(state, headers, false).await?;
    let row: Option<(Uuid, String)> = sqlx::query_as(
        "UPDATE reactor.auth_challenges SET consumed_at = now() \
         WHERE token_hash = $1 AND project_id = $2 AND kind = 'oauth_code' \
         AND consumed_at IS NULL AND expires_at > now() \
         RETURNING user_id, email",
    )
    .bind(token_hash(code))
    .bind(resolved.project_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some((user_id, email)) = row else {
        return Err(ApiError::unauthorized("invalid or expired token"));
    };
    let session = issue_session(state, resolved.project_id, &resolved.pref, user_id, email).await?;
    Ok(session.into_response())
}

fn enc(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn callback_url(state: &AppState, pref: &str, provider: &str) -> String {
    let origin = site_public_url(pref, &state.config.base_domain, &state.config.public_url);
    format!("{origin}/auth/v1/callback/{provider}")
}

#[derive(Deserialize)]
struct AuthorizeQuery {
    provider: String,
    redirect_to: String,
}

async fn authorize(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AuthorizeQuery>,
) -> Result<Redirect, ApiError> {
    let resolved = resolve_project(&state, &headers, false).await?;
    let spec = find_spec(&query.provider)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "unknown provider"))?;
    let client = load_client(&state, resolved.project_id, spec.id).await?;
    let Some(client) = client.filter(|client| client.enabled) else {
        return Err(ApiError::not_found());
    };
    if !client
        .redirects
        .iter()
        .any(|item| item == &query.redirect_to)
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "redirect is not allowed",
        ));
    }
    let verifier = random_token();
    let oauth_state = random_token();
    sqlx::query(
        "INSERT INTO reactor.oauth_transactions (state, project_id, provider, verifier, redirect_to, expires_at) \
         VALUES ($1, $2, $3, $4, $5, now() + interval '10 minutes')",
    )
    .bind(&oauth_state)
    .bind(resolved.project_id)
    .bind(spec.id)
    .bind(&verifier)
    .bind(&query.redirect_to)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    let redirect_uri = callback_url(&state, &resolved.pref, spec.id);
    let authorize = endpoint(&client, spec.authorize, "authorize_url");
    let url = format!(
        "{authorize}?client_id={}&redirect_uri={}&response_type=code&scope={}&state={}&code_challenge={}&code_challenge_method=S256{}",
        enc(&client.client_id),
        enc(&redirect_uri),
        enc(spec.scopes),
        enc(&oauth_state),
        enc(&pkce_challenge(&verifier)),
        if spec.id == "apple" { "&response_mode=form_post" } else { "" }
    );
    Ok(Redirect::temporary(&url))
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

async fn callback_get(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(provider): Path<String>,
    Query(query): Query<CallbackQuery>,
) -> Result<Response, ApiError> {
    finish_callback(
        &state,
        &headers,
        &provider,
        query.code,
        query.state,
        query.error,
    )
    .await
}

async fn callback_post(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(provider): Path<String>,
    Form(query): Form<CallbackQuery>,
) -> Result<Response, ApiError> {
    finish_callback(
        &state,
        &headers,
        &provider,
        query.code,
        query.state,
        query.error,
    )
    .await
}

async fn finish_callback(
    state: &AppState,
    headers: &HeaderMap,
    provider: &str,
    code: Option<String>,
    oauth_state: Option<String>,
    error: Option<String>,
) -> Result<Response, ApiError> {
    let Some(oauth_state) = oauth_state.filter(|value| !value.is_empty()) else {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "state required"));
    };
    let row: Option<(Uuid, String, String, String)> = sqlx::query_as(
        "DELETE FROM reactor.oauth_transactions \
         WHERE state = $1 AND expires_at > now() \
         RETURNING project_id, provider, verifier, redirect_to",
    )
    .bind(&oauth_state)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    let Some((project_id, stored_provider, verifier, redirect_to)) = row else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid or expired state",
        ));
    };
    if stored_provider != provider {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "invalid or expired state",
        ));
    }
    if let Some(error) = error {
        return Ok(
            Redirect::temporary(&format!("{redirect_to}?error={}", enc(&error))).into_response(),
        );
    }
    let Some(code) = code.filter(|value| !value.is_empty()) else {
        return Ok(
            Redirect::temporary(&format!("{redirect_to}?error=missing_code")).into_response(),
        );
    };
    let spec = find_spec(provider)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "unknown provider"))?;
    let Some(client) = load_client(state, project_id, provider).await? else {
        return Ok(
            Redirect::temporary(&format!("{redirect_to}?error=provider_disabled")).into_response(),
        );
    };
    let pref: String = sqlx::query_scalar("SELECT ref FROM reactor.projects WHERE id = $1")
        .bind(project_id)
        .fetch_one(&state.pool)
        .await
        .map_err(internal)?;
    let redirect_uri = callback_url(state, &pref, provider);
    let secret = client_secret(state, &client, spec).map_err(internal)?;
    let token_url = endpoint(&client, spec.token, "token_url");
    let token_body = state
        .http
        .post(&token_url)
        .header("accept", "application/json")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("client_id", client.client_id.as_str()),
            ("client_secret", secret.as_str()),
            ("code_verifier", verifier.as_str()),
        ])
        .send()
        .await
        .map_err(internal)?
        .json::<Value>()
        .await
        .map_err(internal)?;
    let userinfo_url = endpoint(&client, spec.userinfo, "userinfo_url");
    let userinfo = if userinfo_url.is_empty() {
        None
    } else if let Some(access) = token_body
        .get("access_token")
        .and_then(|value| value.as_str())
    {
        match state
            .http
            .get(&userinfo_url)
            .bearer_auth(access)
            .header("accept", "application/json")
            .header("user-agent", "reactor")
            .send()
            .await
        {
            Ok(response) => response.json::<Value>().await.ok(),
            Err(_) => None,
        }
    } else {
        None
    };
    let Some(profile) = read_profile(spec.id, &token_body, userinfo.as_ref()) else {
        return Ok(
            Redirect::temporary(&format!("{redirect_to}?error=email_not_verified")).into_response(),
        );
    };
    if !profile.email_verified {
        return Ok(
            Redirect::temporary(&format!("{redirect_to}?error=email_not_verified")).into_response(),
        );
    }
    let user_id = match link_user(state, project_id, spec.id, &profile).await {
        Ok(id) => id,
        Err(_) => {
            return Ok(
                Redirect::temporary(&format!("{redirect_to}?error=email_not_verified"))
                    .into_response(),
            );
        }
    };
    let one_time = random_token();
    sqlx::query(
        "INSERT INTO reactor.auth_challenges \
         (id, project_id, user_id, email, kind, token_hash, expires_at) \
         VALUES ($1, $2, $3, $4, 'oauth_code', $5, now() + interval '5 minutes')",
    )
    .bind(Uuid::new_v4())
    .bind(project_id)
    .bind(user_id)
    .bind(&profile.email)
    .bind(token_hash(&one_time))
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    let sep = if redirect_to.contains('?') { '&' } else { '?' };
    let _ = headers;
    Ok(Redirect::temporary(&format!("{redirect_to}{sep}code={}", enc(&one_time))).into_response())
}

struct Profile {
    subject: String,
    email: String,
    email_verified: bool,
}

fn read_profile(provider: &str, token_body: &Value, userinfo: Option<&Value>) -> Option<Profile> {
    let source = userinfo.unwrap_or(token_body);
    let from_id = token_body
        .get("id_token")
        .and_then(|value| value.as_str())
        .and_then(jwt_payload);
    let body = if source.get("sub").is_some()
        || source.get("id").is_some()
        || source.get("email").is_some()
    {
        source
    } else {
        from_id.as_ref().unwrap_or(source)
    };
    let subject = match provider {
        "github" | "facebook" => body.get("id").and_then(value_string),
        "x" => body
            .pointer("/data/id")
            .and_then(value_string)
            .or_else(|| body.get("id").and_then(value_string)),
        "slack" => token_body
            .pointer("/authed_user/id")
            .and_then(value_string)
            .or_else(|| body.get("sub").and_then(value_string)),
        _ => body.get("sub").and_then(value_string).or_else(|| {
            from_id
                .as_ref()
                .and_then(|claims| claims.get("sub").and_then(value_string))
        }),
    }?;
    let email = body
        .get("email")
        .and_then(|value| value.as_str())
        .map(|email| email.trim().to_ascii_lowercase())
        .filter(|email| email.contains('@'))
        .or_else(|| {
            from_id.as_ref().and_then(|claims| {
                claims
                    .get("email")
                    .and_then(|value| value.as_str())
                    .map(|email| email.trim().to_ascii_lowercase())
            })
        })
        .unwrap_or_default();
    let email_verified = body
        .get("email_verified")
        .and_then(|value| value.as_bool())
        .or_else(|| {
            from_id.as_ref().and_then(|claims| {
                claims
                    .get("email_verified")
                    .and_then(|value| value.as_bool())
            })
        })
        .unwrap_or_else(|| matches!(provider, "github" | "facebook") && email.contains('@'));
    if subject.is_empty() || !email_verified || !email.contains('@') {
        return None;
    }
    Some(Profile {
        subject,
        email,
        email_verified,
    })
}

fn value_string(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::to_string)
        .or_else(|| value.as_i64().map(|number| number.to_string()))
        .or_else(|| value.as_u64().map(|number| number.to_string()))
}

fn jwt_payload(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload)
        .or_else(|_| {
            use base64::engine::general_purpose::URL_SAFE;
            URL_SAFE.decode(payload)
        })
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

async fn link_user(
    state: &AppState,
    project_id: Uuid,
    provider: &str,
    profile: &Profile,
) -> Result<Uuid, ApiError> {
    let existing: Option<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM reactor.user_identities WHERE project_id = $1 AND provider = $2 AND subject = $3",
    )
    .bind(project_id)
    .bind(provider)
    .bind(&profile.subject)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    if let Some(user_id) = existing {
        mark_verified(state, user_id).await?;
        return Ok(user_id);
    }
    let by_email: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM reactor.users WHERE project_id = $1 AND email = $2")
            .bind(project_id)
            .bind(&profile.email)
            .fetch_optional(&state.pool)
            .await
            .map_err(internal)?;
    let user_id = if let Some(user_id) = by_email {
        user_id
    } else {
        let user_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO reactor.users (id, project_id, email, password_hash, email_verified_at) \
             VALUES ($1, $2, $3, NULL, now())",
        )
        .bind(user_id)
        .bind(project_id)
        .bind(&profile.email)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
        user_id
    };
    sqlx::query(
        "INSERT INTO reactor.user_identities (id, project_id, user_id, provider, subject, email) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(project_id)
    .bind(user_id)
    .bind(provider)
    .bind(&profile.subject)
    .bind(&profile.email)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    mark_verified(state, user_id).await?;
    Ok(user_id)
}

struct ClientRow {
    client_id: String,
    secret_enc: String,
    extra: Value,
    enabled: bool,
    redirects: Vec<String>,
}

async fn load_client(
    state: &AppState,
    project_id: Uuid,
    provider: &str,
) -> Result<Option<ClientRow>, ApiError> {
    let row: Option<(String, String, Value, bool, Vec<String>)> = sqlx::query_as(
        "SELECT client_id, secret_enc, extra, enabled, redirects FROM reactor.oauth_clients \
         WHERE project_id = $1 AND provider = $2",
    )
    .bind(project_id)
    .bind(provider)
    .fetch_optional(&state.pool)
    .await
    .map_err(internal)?;
    Ok(row.map(
        |(client_id, secret_enc, extra, enabled, redirects)| ClientRow {
            client_id,
            secret_enc,
            extra,
            enabled,
            redirects,
        },
    ))
}

fn endpoint(client: &ClientRow, fallback: &str, key: &str) -> String {
    client
        .extra
        .get(key)
        .and_then(|value| value.as_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let tenant = client
                .extra
                .get("tenant")
                .and_then(|value| value.as_str())
                .filter(|value| !value.is_empty())
                .unwrap_or("common");
            fallback.replace("{tenant}", tenant)
        })
}

fn client_secret(state: &AppState, client: &ClientRow, spec: &Spec) -> anyhow::Result<String> {
    if spec.id == "apple" {
        let key = unseal(state.issuer.seal_key(), &client.secret_enc)?;
        let pem = String::from_utf8(key)?;
        let team = client
            .extra
            .get("team_id")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let key_id = client
            .extra
            .get("key_id")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        return apple_client_secret(team, key_id, &client.client_id, &pem);
    }
    if client.secret_enc.is_empty() {
        return Ok(String::new());
    }
    Ok(String::from_utf8(unseal(
        state.issuer.seal_key(),
        &client.secret_enc,
    )?)?)
}

#[derive(Serialize, Deserialize)]
struct AppleClaims {
    iss: String,
    sub: String,
    aud: String,
    iat: u64,
    exp: u64,
}

pub(crate) fn apple_client_secret(
    team_id: &str,
    key_id: &str,
    client_id: &str,
    pem: &str,
) -> anyhow::Result<String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let mut header = Header::new(jsonwebtoken::Algorithm::ES256);
    header.kid = Some(key_id.to_string());
    let claims = AppleClaims {
        iss: team_id.to_string(),
        sub: client_id.to_string(),
        aud: "https://appleid.apple.com".into(),
        iat: now,
        exp: now + 300,
    };
    let key = EncodingKey::from_ec_pem(pem.as_bytes())?;
    Ok(encode(&header, &claims, &key)?)
}

#[derive(Deserialize)]
pub(crate) struct ProviderBody {
    provider: String,
    client_id: String,
    client_secret: Option<String>,
    redirects: Vec<String>,
    enabled: Option<bool>,
    extra: Option<Value>,
}

pub(crate) async fn list_providers(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    let rows: Vec<(String, String, Value, bool, Vec<String>)> = sqlx::query_as(
        "SELECT provider, client_id, extra, enabled, redirects FROM reactor.oauth_clients \
         WHERE project_id = $1 ORDER BY provider",
    )
    .bind(project.id)
    .fetch_all(&state.pool)
    .await
    .map_err(internal)?;
    let origin = site_public_url(
        &project.pref,
        &state.config.base_domain,
        &state.config.public_url,
    );
    let providers: Vec<Value> = rows
        .into_iter()
        .map(|(provider, client_id, extra, enabled, redirects)| {
            json!({
                "provider": provider,
                "client_id": client_id,
                "enabled": enabled,
                "redirects": redirects,
                "extra": extra,
                "callback": format!("{origin}/auth/v1/callback/{provider}"),
            })
        })
        .collect();
    Ok(Json(json!(providers)))
}

pub(crate) async fn put_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pref): Path<String>,
    Json(body): Json<ProviderBody>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    if find_spec(&body.provider).is_none() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "unknown provider"));
    }
    if body.client_id.trim().is_empty() || body.redirects.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "client id and a redirect are required",
        ));
    }
    let mut extra = body.extra.unwrap_or_else(|| json!({}));
    let mut secret = body.client_secret.unwrap_or_default();
    if body.provider == "apple" {
        if let Some(key) = extra.get("private_key").and_then(|value| value.as_str()) {
            secret = key.to_string();
            if let Some(map) = extra.as_object_mut() {
                map.remove("private_key");
            }
        }
    }
    let secret_enc = if secret.is_empty() {
        String::new()
    } else {
        seal(state.issuer.seal_key(), secret.as_bytes()).map_err(internal)?
    };
    sqlx::query(
        "INSERT INTO reactor.oauth_clients \
         (project_id, provider, client_id, secret_enc, extra, enabled, redirects) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) \
         ON CONFLICT (project_id, provider) DO UPDATE SET \
         client_id = EXCLUDED.client_id, \
         secret_enc = CASE WHEN EXCLUDED.secret_enc = '' THEN reactor.oauth_clients.secret_enc ELSE EXCLUDED.secret_enc END, \
         extra = EXCLUDED.extra, enabled = EXCLUDED.enabled, redirects = EXCLUDED.redirects",
    )
    .bind(project.id)
    .bind(&body.provider)
    .bind(body.client_id.trim())
    .bind(&secret_enc)
    .bind(&extra)
    .bind(body.enabled.unwrap_or(true))
    .bind(&body.redirects)
    .execute(&state.pool)
    .await
    .map_err(internal)?;
    list_providers(State(state), headers, Path(pref)).await
}

pub(crate) async fn delete_provider(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((pref, provider)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let project = require_member(&state, &operator, &pref, "admin").await?;
    sqlx::query("DELETE FROM reactor.oauth_clients WHERE project_id = $1 AND provider = $2")
        .bind(project.id)
        .bind(&provider)
        .execute(&state.pool)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::{apple_client_secret, catalog, read_profile};
    use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
    use serde_json::json;

    #[test]
    fn catalog_has_ten_providers() {
        assert_eq!(catalog().len(), 10);
        for spec in catalog() {
            assert!(spec.authorize.starts_with("https://"), "{}", spec.id);
            assert!(spec.token.starts_with("https://"), "{}", spec.id);
            assert!(!spec.id.is_empty());
        }
    }

    #[test]
    fn unverified_email_is_refused() {
        let body = json!({"sub": "1", "email": "a@b.co", "email_verified": false});
        assert!(read_profile("google", &json!({}), Some(&body)).is_none());
    }

    #[test]
    fn apple_client_secret_round_trip() {
        let pem = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgXyMduG1rZb8u0VKW\nfL3+jV+nrBxMXaDqUU46lKUCazShRANCAATWDuVf6qzCC9ojpfd/UHDi026wU+np\n8Ah4uvVNipOVFylrKpmeMWxa9cm52ltlqyfWWBEVRuSeuXvgU5eS61AW\n-----END PRIVATE KEY-----\n";
        let token = apple_client_secret("TEAM", "KEY", "svc.example", pem).expect("sign");
        use p256::pkcs8::{DecodePrivateKey, EncodePublicKey};
        let public = p256::SecretKey::from_pkcs8_pem(pem)
            .unwrap()
            .public_key()
            .to_public_key_pem(p256::pkcs8::LineEnding::LF)
            .unwrap();
        let mut validation = Validation::new(Algorithm::ES256);
        validation.set_required_spec_claims(&["exp"]);
        validation.validate_aud = false;
        let data = decode::<super::AppleClaims>(
            &token,
            &DecodingKey::from_ec_pem(public.as_bytes()).unwrap(),
            &validation,
        )
        .unwrap();
        assert_eq!(data.claims.iss, "TEAM");
        assert_eq!(data.claims.sub, "svc.example");
        assert_eq!(data.claims.aud, "https://appleid.apple.com");
        assert_eq!(data.header.kid.as_deref(), Some("KEY"));
    }
}
