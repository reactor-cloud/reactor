use crate::console::{load_operator, require_operator};
use crate::mfa::password_gate;
use crate::{bearer, ApiError, AppState};
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use reactor_auth::{constant_time_eq, random_token, token_hash, verify_password};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;
use uuid::Uuid;

pub fn mount(app: Router<crate::AppCtx>) -> Router<crate::AppCtx> {
    app.route(
        "/.well-known/oauth-protected-resource",
        get(protected_resource),
    )
    .route(
        "/.well-known/oauth-authorization-server",
        get(authorization_server),
    )
    .route("/oauth/register", post(register))
    .route("/oauth/authorize", get(authorize))
    .route("/oauth/login", post(login))
    .route("/oauth/session", post(session))
    .route("/oauth/consent", post(consent))
    .route("/oauth/token", post(token))
}

pub fn mcp_resource(public_url: &str) -> String {
    format!("{}/mcp", public_url.trim_end_matches('/'))
}

pub fn protected_metadata(public_url: &str) -> Value {
    json!({
        "resource": mcp_resource(public_url),
        "authorization_servers": [public_url.trim_end_matches('/')],
        "scopes_supported": ["operator", "operator:read", "offline_access"],
        "bearer_methods_supported": ["header"]
    })
}

pub fn server_metadata(public_url: &str) -> Value {
    let base = public_url.trim_end_matches('/');
    json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth/authorize"),
        "token_endpoint": format!("{base}/oauth/token"),
        "registration_endpoint": format!("{base}/oauth/register"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "scopes_supported": ["operator", "operator:read", "offline_access"]
    })
}

pub fn oauth_error(error: &str, description: &str) -> Value {
    json!({"error": error, "error_description": description})
}

pub fn s256(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

pub fn granted_scope(requested: &str) -> Result<String, &'static str> {
    let mut parts: Vec<&str> = requested
        .split_whitespace()
        .filter(|s| !s.is_empty())
        .collect();
    if parts.is_empty() {
        return Ok("operator offline_access".into());
    }
    for part in &parts {
        if *part != "operator" && *part != "operator:read" && *part != "offline_access" {
            return Err("invalid scope");
        }
    }
    if !parts.contains(&"operator") && !parts.contains(&"operator:read") {
        parts.insert(0, "operator");
    }
    if parts.contains(&"operator") {
        parts.retain(|part| *part != "operator:read");
    }
    let mut out = Vec::new();
    if parts.contains(&"operator") {
        out.push("operator");
    } else {
        out.push("operator:read");
    }
    if parts.contains(&"offline_access") {
        out.push("offline_access");
    }
    Ok(out.join(" "))
}

pub fn access_level(scope: &str) -> &'static str {
    if scope.split_whitespace().any(|part| part == "operator") {
        "operator"
    } else {
        "operator:read"
    }
}

pub fn host_is_blocked(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']);
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    let Ok(ip) = host.parse::<IpAddr>() else {
        return false;
    };
    match ip {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
        }
        IpAddr::V6(v6) => {
            let ula = (v6.segments()[0] & 0xfe00) == 0xfc00;
            v6.is_loopback() || v6.is_unspecified() || v6.is_unicast_link_local() || ula
        }
    }
}

fn host_is_loopback(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']);
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => v4.is_loopback(),
        Ok(IpAddr::V6(v6)) => v6.is_loopback(),
        Err(_) => false,
    }
}

pub fn redirect_allowed(uri: &str) -> bool {
    let Ok(url) = url::Url::parse(uri) else {
        return false;
    };
    if url.fragment().is_some() {
        return false;
    }
    if url.username() != "" || url.password().is_some() {
        return false;
    }
    match url.scheme() {
        "https" => url.host_str().is_some() && !host_is_blocked(url.host_str().unwrap_or("")),
        "http" => host_is_loopback(url.host_str().unwrap_or("")),
        // Cursor registers both paths. /oauth/return is the deep link the desktop
        // listener opens after the loopback callback.
        "cursor" => {
            url.host_str() == Some("anysphere.cursor-mcp")
                && (url.path() == "/oauth/callback" || url.path() == "/oauth/return")
                && url.query().is_none()
        }
        _ => false,
    }
}

/// Cursor's hosted callback lands on cursor.com, which is supposed to forward the
/// browser to a loopback listener. The listener is named by a suffix on `state`
/// (`.tgd` is `http://localhost:8787/callback`). Send the browser there directly.
pub fn cursor_desktop_return(redirect_uri: &str, state: &str) -> Option<&'static str> {
    let Ok(url) = url::Url::parse(redirect_uri) else {
        return None;
    };
    if url.scheme() != "https" {
        return None;
    }
    let Some(host) = url.host_str() else {
        return None;
    };
    if host != "cursor.com" && host != "www.cursor.com" {
        return None;
    }
    if url.path() != "/agents/mcp/oauth/callback" {
        return None;
    }
    let Some((_, code)) = state.rsplit_once(".t") else {
        return None;
    };
    match code {
        "gd" => Some("http://localhost:8787/callback"),
        "gd18787" => Some("http://localhost:18787/callback"),
        "gd28787" => Some("http://localhost:28787/callback"),
        "gd31787" => Some("http://localhost:31787/callback"),
        _ => None,
    }
}

pub fn redirect_matches(registered: &str, actual: &str) -> bool {
    if registered == actual {
        return true;
    }
    let Ok(left) = url::Url::parse(registered) else {
        return false;
    };
    let Ok(right) = url::Url::parse(actual) else {
        return false;
    };
    if left.scheme() != "http" || right.scheme() != "http" {
        return false;
    }
    if !host_is_loopback(left.host_str().unwrap_or(""))
        || !host_is_loopback(right.host_str().unwrap_or(""))
    {
        return false;
    }
    left.host_str() == right.host_str()
        && left.path() == right.path()
        && left.query() == right.query()
}

fn esc(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn form_pairs(body: &str) -> HashMap<String, String> {
    url::form_urlencoded::parse(body.as_bytes())
        .into_owned()
        .collect()
}

async fn read_body(request: axum::extract::Request) -> Result<(HeaderMap, String), ApiError> {
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, 64 * 1024)
        .await
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "body is too large"))?;
    let text = String::from_utf8(bytes.to_vec())
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "body is not utf-8"))?;
    Ok((parts.headers, text))
}

fn pairs_from(headers: &HeaderMap, text: &str) -> Result<HashMap<String, String>, ApiError> {
    let content = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if content.starts_with("application/json") {
        let value: Value = serde_json::from_str(text)
            .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "invalid json"))?;
        let Some(map) = value.as_object() else {
            return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid json"));
        };
        let mut out = HashMap::new();
        for (key, item) in map {
            if let Some(text) = item.as_str() {
                out.insert(key.clone(), text.to_string());
            } else if let Some(items) = item.as_array() {
                let joined = items
                    .iter()
                    .filter_map(|item| item.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                if !joined.is_empty() {
                    out.insert(key.clone(), joined);
                }
            } else if !item.is_null() {
                out.insert(key.clone(), item.to_string());
            }
        }
        return Ok(out);
    }
    Ok(form_pairs(text))
}

#[derive(Deserialize)]
struct AuthorizeQuery {
    response_type: Option<String>,
    client_id: Option<String>,
    redirect_uri: Option<String>,
    code_challenge: Option<String>,
    code_challenge_method: Option<String>,
    scope: Option<String>,
    resource: Option<String>,
    state: Option<String>,
}

struct Authz {
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    scope: String,
    resource: String,
    state: String,
}

const PAGE_STYLE: &str = r#"
:root { color-scheme: light; }
* { box-sizing: border-box; }
body {
  margin: 0;
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 1.5rem;
  background: color-mix(in oklch, oklch(0.97 0 0) 40%, white);
  color: oklch(0.145 0 0);
  font-family: "Geist Variable", Geist, ui-sans-serif, system-ui, sans-serif;
  font-size: 0.875rem;
  line-height: 1.25rem;
}
main {
  width: 100%;
  max-width: 28rem;
  display: flex;
  flex-direction: column;
  gap: 1rem;
  padding: 1rem;
  background: white;
  border-radius: 0.75rem;
  box-shadow: 0 0 0 1px color-mix(in oklch, oklch(0.145 0 0) 10%, transparent);
}
.logo {
  width: 3rem;
  height: 3rem;
  margin-bottom: 0.5rem;
  border-radius: 12px;
  background: #070B15;
  object-fit: cover;
}
h1 { margin: 0; font-size: 1rem; font-weight: 500; line-height: 1.375; }
.sub { margin: 0.25rem 0 0; color: oklch(0.556 0 0); }
form { display: grid; gap: 1rem; }
label { display: grid; gap: 0.5rem; font-weight: 500; }
input {
  width: 100%;
  height: 2rem;
  padding: 0.25rem 0.625rem;
  border: 1px solid oklch(0.922 0 0);
  border-radius: 0.625rem;
  background: transparent;
  color: inherit;
  font: inherit;
}
input:focus {
  outline: none;
  border-color: oklch(0.708 0 0);
  box-shadow: 0 0 0 3px color-mix(in oklch, oklch(0.708 0 0) 50%, transparent);
}
button {
  justify-self: start;
  height: 2rem;
  padding: 0 0.625rem;
  border: 0;
  border-radius: 0.625rem;
  background: oklch(0.205 0 0);
  color: oklch(0.985 0 0);
  font: inherit;
  font-weight: 500;
  cursor: pointer;
}
button:hover { background: color-mix(in oklch, oklch(0.205 0 0) 80%, white); }
button:disabled { opacity: 0.5; }
button.link {
  height: auto;
  padding: 0;
  background: none;
  color: oklch(0.556 0 0);
  font-weight: 400;
  text-decoration: underline;
}
button.link:hover { background: none; }
.error { margin: 0; color: oklch(0.577 0.245 27.325); }
"#;

fn page(title: &str, subtitle: &str, body: &str) -> Html<String> {
    let subtitle = if subtitle.is_empty() {
        String::new()
    } else {
        format!("<p class=\"sub\">{}</p>", esc(subtitle))
    };
    Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{}</title><link rel=\"icon\" href=\"/favicon.ico\" sizes=\"32x32\"><link rel=\"icon\" href=\"/favicon.svg\" type=\"image/svg+xml\"><style>{}</style></head><body><main><img class=\"logo\" src=\"/favicon.svg\" alt=\"\"><div><h1>{}</h1>{}</div>{}</main></body></html>",
        esc(title),
        PAGE_STYLE,
        esc(title),
        subtitle,
        body
    ))
}

fn authz_fields(authz: &Authz) -> String {
    format!(
        "{}{}{}{}{}{}{}",
        hidden("client_id", &authz.client_id),
        hidden("redirect_uri", &authz.redirect_uri),
        hidden("code_challenge", &authz.code_challenge),
        hidden("code_challenge_method", "S256"),
        hidden("scope", &authz.scope),
        hidden("resource", &authz.resource),
        hidden("state", &authz.state),
    )
}

fn sign_in(authz: &Authz, error: &str) -> Html<String> {
    let error = if error.is_empty() {
        String::new()
    } else {
        format!("<p class=\"error\">{}</p>", esc(error))
    };
    page(
        "Log in",
        "Console access. Project users sign in on the project itself.",
        &format!(
            "<form method=\"post\" action=\"/oauth/login\">{}<label>Email<input name=\"email\" type=\"email\" autocomplete=\"username\" required autofocus></label><label>Password<input name=\"password\" type=\"password\" autocomplete=\"current-password\" required></label>{}<button type=\"submit\">Continue</button></form>",
            authz_fields(authz),
            error
        ),
    )
}

fn hidden(name: &str, value: &str) -> String {
    format!(
        "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
        esc(name),
        esc(value)
    )
}

async fn protected_resource(State(state): State<AppState>) -> Json<Value> {
    Json(protected_metadata(&state.config.public_url))
}

async fn authorization_server(State(state): State<AppState>) -> Json<Value> {
    Json(server_metadata(&state.config.public_url))
}

async fn register(State(state): State<AppState>, request: axum::extract::Request) -> Response {
    let (headers, text) = match read_body(request).await {
        Ok(pair) => pair,
        Err(err) => return err.into_response(),
    };
    let pairs = match pairs_from(&headers, &text) {
        Ok(pairs) => pairs,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(oauth_error("invalid_client_metadata", err.text())),
            )
                .into_response()
        }
    };
    let method = pairs
        .get("token_endpoint_auth_method")
        .map(String::as_str)
        .unwrap_or("none");
    if method != "none" {
        return (
            StatusCode::BAD_REQUEST,
            Json(oauth_error(
                "invalid_client_metadata",
                "public clients only",
            )),
        )
            .into_response();
    }
    if pairs.contains_key("client_secret") {
        return (
            StatusCode::BAD_REQUEST,
            Json(oauth_error(
                "invalid_client_metadata",
                "client_secret is not used",
            )),
        )
            .into_response();
    }
    let name = pairs.get("client_name").cloned().unwrap_or_default();
    let redirects = redirect_list(pairs.get("redirect_uris").map(String::as_str).unwrap_or(""));
    if redirects.is_empty() || redirects.iter().any(|uri| !redirect_allowed(uri)) {
        return (
            StatusCode::BAD_REQUEST,
            Json(oauth_error(
                "invalid_redirect_uri",
                "redirect_uri is not allowed",
            )),
        )
            .into_response();
    }
    let id = random_token();
    if let Err(err) = insert_client(&state, &id, &name, &redirects).await {
        return err.into_response();
    }
    (
        StatusCode::CREATED,
        Json(json!({
            "client_id": id,
            "client_name": name,
            "redirect_uris": redirects,
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"]
        })),
    )
        .into_response()
}

fn redirect_list(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.starts_with('[') {
        serde_json::from_str::<Vec<String>>(trimmed).unwrap_or_default()
    } else {
        trimmed
            .split_whitespace()
            .filter(|item| !item.is_empty())
            .map(|item| item.to_string())
            .collect()
    }
}

async fn insert_client(
    state: &AppState,
    id: &str,
    name: &str,
    redirects: &[String],
) -> Result<(), ApiError> {
    sqlx::query(
        "INSERT INTO reactor.mcp_clients (id, name, redirect_uris) VALUES ($1, $2, $3) \
         ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, redirect_uris = EXCLUDED.redirect_uris",
    )
    .bind(id)
    .bind(name)
    .bind(redirects)
    .execute(&state.pool)
    .await
    .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(())
}

async fn authorize(State(state): State<AppState>, Query(query): Query<AuthorizeQuery>) -> Response {
    match prepare(&state, &query_map(&query)).await {
        Ok(authz) => sign_in(&authz, "").into_response(),
        Err(err) => (
            StatusCode::BAD_REQUEST,
            page(
                "Authorization error",
                "",
                &format!("<p class=\"error\">{}</p>", esc(err.text())),
            ),
        )
            .into_response(),
    }
}

fn query_map(query: &AuthorizeQuery) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for (key, value) in [
        ("response_type", query.response_type.clone()),
        ("client_id", query.client_id.clone()),
        ("redirect_uri", query.redirect_uri.clone()),
        ("code_challenge", query.code_challenge.clone()),
        ("code_challenge_method", query.code_challenge_method.clone()),
        ("scope", query.scope.clone()),
        ("resource", query.resource.clone()),
        ("state", query.state.clone()),
    ] {
        if let Some(value) = value {
            map.insert(key.to_string(), value);
        }
    }
    map
}

async fn login(State(state): State<AppState>, request: axum::extract::Request) -> Response {
    let (headers, text) = match read_body(request).await {
        Ok(pair) => pair,
        Err(err) => return err.into_response(),
    };
    let pairs = match pairs_from(&headers, &text) {
        Ok(pairs) => pairs,
        Err(err) => return err.into_response(),
    };
    let authz = match prepare(&state, &pairs).await {
        Ok(authz) => authz,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                page(
                    "Authorization error",
                    "",
                    &format!("<p class=\"error\">{}</p>", esc(err.text())),
                ),
            )
                .into_response()
        }
    };
    let email = pairs.get("email").cloned().unwrap_or_default();
    let password = pairs.get("password").cloned().unwrap_or_default();
    let gate = match operator_password(&state, &email, &password).await {
        Ok(body) => body.0,
        Err(err) => return (err.status(), sign_in(&authz, err.text())).into_response(),
    };
    if gate.get("access_token").and_then(|v| v.as_str()).is_some() {
        let operator_id = gate["operator"]["id"].as_str().unwrap_or("");
        return allow_page(&state, operator_id, &authz);
    }
    if let Some(mfa_token) = gate.get("mfa_token").and_then(|v| v.as_str()) {
        return second_factor(&authz, mfa_token).into_response();
    }
    page(
        "Second factor required",
        "This account still needs a passkey and an authenticator. Finish that in the console, then sign in here again. No authorization code was issued.",
        "",
    )
    .into_response()
}

fn allow_page(state: &AppState, operator_id: &str, authz: &Authz) -> Response {
    let consent = match state
        .issuer
        .sign_audience(operator_id, "oauth-consent", 10 * 60)
    {
        Ok(token) => token,
        Err(err) => {
            return ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())
                .into_response()
        }
    };
    page(
        "Allow access",
        "This client wants access to this cluster.",
        &format!(
            "<form method=\"post\" action=\"/oauth/consent\">{}{}<button type=\"submit\" id=\"approve\">Approve</button></form>",
            hidden("consent", &consent),
            authz_fields(authz),
        ),
    )
    .into_response()
}

const MFA_SCRIPT: &str = r#"
const cfg = JSON.parse(document.getElementById("oauth-mfa").textContent)
const error = document.getElementById("mfa-error")
const name = document.getElementById("code-name")
const input = document.getElementById("otp")
let recovery = false
function show(err) {
  error.hidden = false
  error.textContent = err && err.message ? err.message : "could not verify"
}
function b64urlToBytes(value) {
  const raw = atob(String(value).replace(/-/g, "+").replace(/_/g, "/"))
  const out = new Uint8Array(raw.length)
  for (let i = 0; i < raw.length; i++) out[i] = raw.charCodeAt(i)
  return out
}
function bytesToB64url(buffer) {
  const view = new Uint8Array(buffer)
  let raw = ""
  for (const byte of view) raw += String.fromCharCode(byte)
  return btoa(raw).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "")
}
async function mfa(path, body) {
  const res = await fetch(path, {
    method: "POST",
    headers: { "content-type": "application/json", authorization: "Bearer " + cfg.mfa_token },
    body: JSON.stringify(body || {})
  })
  const data = await res.json().catch(() => ({}))
  if (!res.ok) throw new Error(data.error || "request failed")
  return data
}
async function finish(access) {
  const res = await fetch("/oauth/session", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      access_token: access,
      client_id: cfg.client_id,
      redirect_uri: cfg.redirect_uri,
      code_challenge: cfg.code_challenge,
      code_challenge_method: "S256",
      scope: cfg.scope,
      resource: cfg.resource,
      state: cfg.state
    })
  })
  const text = await res.text()
  if (!res.ok) {
    let message = text
    try { message = JSON.parse(text).error || text } catch (e) {}
    throw new Error(message)
  }
  document.open()
  document.write(text)
  document.close()
}
document.getElementById("passkey").addEventListener("click", async () => {
  error.hidden = true
  try {
    const options = await mfa("/console/v1/mfa/passkey/options")
    const publicKey = options.publicKey
    publicKey.challenge = b64urlToBytes(publicKey.challenge)
    if (publicKey.allowCredentials) {
      publicKey.allowCredentials = publicKey.allowCredentials.map((item) => ({ ...item, id: b64urlToBytes(item.id) }))
    }
    const cred = await navigator.credentials.get({ publicKey })
    if (!(cred instanceof PublicKeyCredential)) throw new Error("passkey was not used")
    const response = cred.response
    const verified = await mfa("/console/v1/mfa/passkey/verify", {
      id: cred.id,
      rawId: bytesToB64url(cred.rawId),
      type: cred.type,
      response: {
        authenticatorData: bytesToB64url(response.authenticatorData),
        clientDataJSON: bytesToB64url(response.clientDataJSON),
        signature: bytesToB64url(response.signature),
        userHandle: response.userHandle ? bytesToB64url(response.userHandle) : null
      },
      extensions: cred.getClientExtensionResults()
    })
    await finish(verified.access_token)
  } catch (err) {
    show(err)
  }
})
document.getElementById("switch").addEventListener("click", () => {
  recovery = !recovery
  name.textContent = recovery ? "Backup code" : "Authenticator code"
  input.inputMode = recovery ? "text" : "numeric"
  document.getElementById("switch").textContent = recovery ? "Use an authenticator code" : "Use a backup code"
  input.value = ""
})
document.getElementById("code").addEventListener("submit", async (event) => {
  event.preventDefault()
  error.hidden = true
  try {
    const path = recovery ? "/console/v1/mfa/recovery" : "/console/v1/mfa/totp"
    const verified = await mfa(path, { code: input.value.trim() })
    await finish(verified.access_token)
  } catch (err) {
    show(err)
  }
})
"#;

fn json_for_script(value: &Value) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "{}".into())
        .replace('<', "\\u003c")
}

fn second_factor(authz: &Authz, mfa_token: &str) -> Html<String> {
    let cfg = json_for_script(&json!({
        "mfa_token": mfa_token,
        "client_id": authz.client_id,
        "redirect_uri": authz.redirect_uri,
        "code_challenge": authz.code_challenge,
        "scope": authz.scope,
        "resource": authz.resource,
        "state": authz.state,
    }));
    let mut body = String::from(
        r#"<button type="button" id="passkey">Use passkey</button><form id="code"><label><span id="code-name">Authenticator code</span><input id="otp" autocomplete="one-time-code" inputmode="numeric" required></label><button type="submit">Continue</button></form><button type="button" class="link" id="switch">Use a backup code</button><p class="error" id="mfa-error" hidden></p><script type="application/json" id="oauth-mfa">"#,
    );
    body.push_str(&cfg);
    body.push_str("</script><script>");
    body.push_str(MFA_SCRIPT);
    body.push_str("</script>");
    page(
        "Second factor",
        "Use the passkey or the authenticator code for this account.",
        &body,
    )
}

async fn session(State(state): State<AppState>, request: axum::extract::Request) -> Response {
    let (headers, text) = match read_body(request).await {
        Ok(pair) => pair,
        Err(err) => return err.into_response(),
    };
    let pairs = match pairs_from(&headers, &text) {
        Ok(pairs) => pairs,
        Err(err) => return err.into_response(),
    };
    let authz = match prepare(&state, &pairs).await {
        Ok(authz) => authz,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(oauth_error("invalid_request", err.text())),
            )
                .into_response()
        }
    };
    let Some(access) = pairs.get("access_token") else {
        return (
            StatusCode::BAD_REQUEST,
            Json(oauth_error("invalid_request", "access_token is required")),
        )
            .into_response();
    };
    let claims = match state.issuer.decode(access) {
        Ok(claims) if claims.aud == "console" => claims,
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(oauth_error("invalid_request", "sign in again")),
            )
                .into_response()
        }
    };
    let Ok(operator_id) = Uuid::parse_str(&claims.sub) else {
        return (
            StatusCode::UNAUTHORIZED,
            Json(oauth_error("invalid_request", "sign in again")),
        )
            .into_response();
    };
    if load_operator(&state, operator_id).await.is_err() {
        return (
            StatusCode::UNAUTHORIZED,
            Json(oauth_error("invalid_request", "sign in again")),
        )
            .into_response();
    }
    allow_page(&state, &operator_id.to_string(), &authz)
}

async fn consent(State(state): State<AppState>, request: axum::extract::Request) -> Response {
    let (headers, text) = match read_body(request).await {
        Ok(pair) => pair,
        Err(err) => return err.into_response(),
    };
    let pairs = match pairs_from(&headers, &text) {
        Ok(pairs) => pairs,
        Err(err) => return err.into_response(),
    };
    let authz = match prepare(&state, &pairs).await {
        Ok(authz) => authz,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(oauth_error("invalid_request", err.text())),
            )
                .into_response()
        }
    };
    let Some(consent) = pairs.get("consent") else {
        return (
            StatusCode::BAD_REQUEST,
            Json(oauth_error("invalid_request", "consent is required")),
        )
            .into_response();
    };
    let claims = match state.issuer.decode(consent) {
        Ok(claims) if claims.aud == "oauth-consent" => claims,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(oauth_error("invalid_request", "consent is not valid")),
            )
                .into_response()
        }
    };
    let Ok(operator_id) = Uuid::parse_str(&claims.sub) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(oauth_error("invalid_request", "consent is not valid")),
        )
            .into_response();
    };
    if load_operator(&state, operator_id).await.is_err() {
        return (
            StatusCode::BAD_REQUEST,
            Json(oauth_error("invalid_request", "operator is not valid")),
        )
            .into_response();
    }
    let code = match issue_code(&state, &authz, operator_id).await {
        Ok(code) => code,
        Err(err) => return err.into_response(),
    };
    let mut extra = format!("code={}", urlencoding(&code));
    if !authz.state.is_empty() {
        extra.push_str("&state=");
        extra.push_str(&urlencoding(&authz.state));
    }
    let target = cursor_desktop_return(&authz.redirect_uri, &authz.state)
        .unwrap_or(authz.redirect_uri.as_str());
    let location = if target.contains('?') {
        format!("{target}&{extra}")
    } else {
        format!("{target}?{extra}")
    };
    Redirect::to(&location).into_response()
}

async fn operator_password(
    state: &AppState,
    email: &str,
    password: &str,
) -> Result<Json<Value>, ApiError> {
    let row: Option<(Uuid, String, String, String, bool)> = sqlx::query_as(
        "SELECT id, email, name, password_hash, platform_admin FROM reactor.operators WHERE email = $1",
    )
    .bind(email.trim().to_lowercase())
    .fetch_optional(&state.pool)
    .await
    .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let Some((id, email, name, password_hash, platform_admin)) = row else {
        return Err(ApiError::unauthorized("invalid email or password"));
    };
    if !verify_password(password, &password_hash) {
        return Err(ApiError::unauthorized("invalid email or password"));
    }
    password_gate(state, id, &email, &name, platform_admin).await
}

fn urlencoding(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

async fn prepare(state: &AppState, pairs: &HashMap<String, String>) -> Result<Authz, ApiError> {
    let response_type = pairs
        .get("response_type")
        .map(String::as_str)
        .unwrap_or("code");
    if response_type != "code" {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "response_type must be code",
        ));
    }
    let method = pairs
        .get("code_challenge_method")
        .map(String::as_str)
        .unwrap_or("S256");
    if method != "S256" {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "S256 is required"));
    }
    let client_id = pairs
        .get("client_id")
        .cloned()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "client_id is required"))?;
    let redirect_uri = pairs
        .get("redirect_uri")
        .cloned()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "redirect_uri is required"))?;
    if !redirect_allowed(&redirect_uri) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "redirect_uri is not allowed",
        ));
    }
    let challenge = pairs
        .get("code_challenge")
        .cloned()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "code_challenge is required"))?;
    let scope = granted_scope(pairs.get("scope").map(String::as_str).unwrap_or(""))
        .map_err(|err| ApiError::new(StatusCode::BAD_REQUEST, err))?;
    let resource = pairs
        .get("resource")
        .cloned()
        .unwrap_or_else(|| mcp_resource(&state.config.public_url));
    if resource != mcp_resource(&state.config.public_url) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "resource is not this MCP",
        ));
    }
    let redirects = client_redirects(state, &client_id).await?;
    if !redirects
        .iter()
        .any(|item| redirect_matches(item, &redirect_uri))
    {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "redirect_uri is not registered",
        ));
    }
    Ok(Authz {
        client_id,
        redirect_uri,
        code_challenge: challenge,
        scope,
        resource,
        state: pairs.get("state").cloned().unwrap_or_default(),
    })
}

async fn client_redirects(state: &AppState, client_id: &str) -> Result<Vec<String>, ApiError> {
    let row: Option<Vec<String>> =
        sqlx::query_scalar("SELECT redirect_uris FROM reactor.mcp_clients WHERE id = $1")
            .bind(client_id)
            .fetch_optional(&state.pool)
            .await
            .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    if let Some(uris) = row {
        return Ok(uris);
    }
    if let Some(uris) = fetch_metadata(state, client_id).await? {
        insert_client(state, client_id, client_id, &uris).await?;
        return Ok(uris);
    }
    Err(ApiError::new(StatusCode::BAD_REQUEST, "unknown client"))
}

async fn fetch_metadata(
    state: &AppState,
    client_id: &str,
) -> Result<Option<Vec<String>>, ApiError> {
    if !client_id.starts_with("https://") {
        return Ok(None);
    }
    let Ok(url) = url::Url::parse(client_id) else {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid client_id"));
    };
    if host_is_blocked(url.host_str().unwrap_or("")) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "client metadata host is not allowed",
        ));
    }
    let response = state
        .http
        .get(client_id)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "client metadata was not fetched"))?;
    if !response.status().is_success() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "client metadata was not fetched",
        ));
    }
    let body: Value = response
        .json()
        .await
        .map_err(|_| ApiError::new(StatusCode::BAD_REQUEST, "client metadata is not json"))?;
    if let Some(id) = body.get("client_id").and_then(|v| v.as_str()) {
        if id != client_id {
            return Err(ApiError::new(StatusCode::BAD_REQUEST, "client_id mismatch"));
        }
    }
    let uris = body
        .get("redirect_uris")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if uris.is_empty() || uris.iter().any(|uri| !redirect_allowed(uri)) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "redirect_uri is not allowed",
        ));
    }
    Ok(Some(uris))
}

async fn issue_code(
    state: &AppState,
    authz: &Authz,
    operator_id: Uuid,
) -> Result<String, ApiError> {
    let code = random_token();
    sqlx::query(
        "INSERT INTO reactor.oauth_codes \
         (code_hash, client_id, operator_id, redirect_uri, code_challenge, scope, resource, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, now() + interval '60 seconds')",
    )
    .bind(token_hash(&code))
    .bind(&authz.client_id)
    .bind(operator_id)
    .bind(&authz.redirect_uri)
    .bind(&authz.code_challenge)
    .bind(&authz.scope)
    .bind(&authz.resource)
    .execute(&state.pool)
    .await
    .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(code)
}

async fn token(State(state): State<AppState>, request: axum::extract::Request) -> Response {
    let (headers, text) = match read_body(request).await {
        Ok(pair) => pair,
        Err(err) => return err.into_response(),
    };
    let pairs = match pairs_from(&headers, &text) {
        Ok(pairs) => pairs,
        Err(err) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(oauth_error("invalid_request", err.text())),
            )
                .into_response()
        }
    };
    if pairs.contains_key("client_secret") {
        return (
            StatusCode::BAD_REQUEST,
            Json(oauth_error("invalid_client", "client_secret is not used")),
        )
            .into_response();
    }
    let grant = pairs.get("grant_type").map(String::as_str).unwrap_or("");
    let result = match grant {
        "authorization_code" => exchange_code(&state, &pairs).await,
        "refresh_token" => exchange_refresh(&state, &pairs).await,
        _ => Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "unsupported grant_type",
        )),
    };
    match result {
        Ok(body) => (
            StatusCode::OK,
            [(header::CACHE_CONTROL, "no-store")],
            Json(body),
        )
            .into_response(),
        Err(err) => {
            let error = if err.text().contains("client") {
                "invalid_client"
            } else {
                "invalid_grant"
            };
            (err.status(), Json(oauth_error(error, err.text()))).into_response()
        }
    }
}

async fn exchange_code(
    state: &AppState,
    pairs: &HashMap<String, String>,
) -> Result<Value, ApiError> {
    let code = pairs
        .get("code")
        .cloned()
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "code is required"))?;
    let verifier = pairs
        .get("code_verifier")
        .cloned()
        .filter(|value| (43..=128).contains(&value.len()))
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "code_verifier is required"))?;
    let client_id = pairs
        .get("client_id")
        .cloned()
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "client_id is required"))?;
    let redirect_uri = pairs
        .get("redirect_uri")
        .cloned()
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "redirect_uri is required"))?;
    let resource = pairs
        .get("resource")
        .cloned()
        .unwrap_or_else(|| mcp_resource(&state.config.public_url));
    if resource != mcp_resource(&state.config.public_url) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "resource is not this MCP",
        ));
    }
    let hash = token_hash(&code);
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let row: Option<(String, Uuid, String, String, String, String, bool)> = sqlx::query_as(
        "SELECT client_id, operator_id, redirect_uri, code_challenge, scope, resource, consumed_at IS NOT NULL \
         FROM reactor.oauth_codes WHERE code_hash = $1 AND expires_at > now() FOR UPDATE",
    )
    .bind(&hash)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let Some((row_client, operator_id, row_redirect, challenge, scope, row_resource, consumed)) =
        row
    else {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "code is not valid"));
    };
    if consumed
        || row_client != client_id
        || row_redirect != redirect_uri
        || row_resource != resource
    {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "code is not valid"));
    }
    if !constant_time_eq(&s256(&verifier), &challenge) {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "code_verifier does not match",
        ));
    }
    sqlx::query("UPDATE reactor.oauth_codes SET consumed_at = now() WHERE code_hash = $1")
        .bind(&hash)
        .execute(&mut *tx)
        .await
        .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let refresh = insert_refresh(&mut tx, &client_id, operator_id, &scope, &resource).await?;
    tx.commit()
        .await
        .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let access = issue_access(state, operator_id, &resource, &scope).await?;
    Ok(token_body(&access, &refresh, &scope))
}

async fn exchange_refresh(
    state: &AppState,
    pairs: &HashMap<String, String>,
) -> Result<Value, ApiError> {
    let refresh = pairs
        .get("refresh_token")
        .cloned()
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "refresh_token is required"))?;
    let client_id = pairs
        .get("client_id")
        .cloned()
        .ok_or_else(|| ApiError::new(StatusCode::BAD_REQUEST, "client_id is required"))?;
    let hash = token_hash(&refresh);
    let mut tx = state
        .pool
        .begin()
        .await
        .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let row: Option<(Uuid, String, Uuid, String, String, bool, bool)> = sqlx::query_as(
        "SELECT family_id, client_id, operator_id, scope, resource, rotated_at IS NOT NULL, expires_at > now() \
         FROM reactor.oauth_refresh WHERE token_hash = $1 FOR UPDATE",
    )
    .bind(&hash)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let Some((family_id, row_client, operator_id, scope, resource, rotated, live)) = row else {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "refresh_token is not valid",
        ));
    };
    if row_client != client_id {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "client_id does not match",
        ));
    }
    if rotated {
        sqlx::query("DELETE FROM reactor.oauth_refresh WHERE family_id = $1")
            .bind(family_id)
            .execute(&mut *tx)
            .await
            .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        tx.commit()
            .await
            .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "refresh_token was already used",
        ));
    }
    if !live {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "refresh_token is expired",
        ));
    }
    sqlx::query("UPDATE reactor.oauth_refresh SET rotated_at = now() WHERE token_hash = $1")
        .bind(&hash)
        .execute(&mut *tx)
        .await
        .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let refresh = insert_refresh_family(
        &mut tx,
        family_id,
        &client_id,
        operator_id,
        &scope,
        &resource,
    )
    .await?;
    tx.commit()
        .await
        .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let access = issue_access(state, operator_id, &resource, &scope).await?;
    Ok(token_body(&access, &refresh, &scope))
}

async fn insert_refresh(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    client_id: &str,
    operator_id: Uuid,
    scope: &str,
    resource: &str,
) -> Result<String, ApiError> {
    insert_refresh_family(tx, Uuid::new_v4(), client_id, operator_id, scope, resource).await
}

async fn insert_refresh_family(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    family_id: Uuid,
    client_id: &str,
    operator_id: Uuid,
    scope: &str,
    resource: &str,
) -> Result<String, ApiError> {
    let refresh = random_token();
    sqlx::query(
        "INSERT INTO reactor.oauth_refresh \
         (id, family_id, token_hash, client_id, operator_id, scope, resource, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, now() + interval '30 days')",
    )
    .bind(Uuid::new_v4())
    .bind(family_id)
    .bind(token_hash(&refresh))
    .bind(client_id)
    .bind(operator_id)
    .bind(scope)
    .bind(resource)
    .execute(&mut **tx)
    .await
    .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(refresh)
}

pub(crate) async fn issue_access(
    state: &AppState,
    operator_id: Uuid,
    resource: &str,
    scope: &str,
) -> Result<String, ApiError> {
    state
        .issuer
        .sign_scoped(
            &operator_id.to_string(),
            resource,
            state.config.public_url.trim_end_matches('/'),
            scope,
            3600,
        )
        .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))
}

fn token_body(access: &str, refresh: &str, scope: &str) -> Value {
    json!({
        "access_token": access,
        "token_type": "Bearer",
        "expires_in": 3600,
        "refresh_token": refresh,
        "scope": scope
    })
}

pub(crate) async fn list_sessions(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let rows: Vec<(Uuid, String, String, String, String)> = sqlx::query_as(
        "SELECT family_id, client_id, scope, created_at::text, expires_at::text \
         FROM reactor.oauth_refresh \
         WHERE operator_id = $1 AND rotated_at IS NULL AND expires_at > now() \
         ORDER BY created_at DESC",
    )
    .bind(operator.id)
    .fetch_all(&state.pool)
    .await
    .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let items = rows
        .into_iter()
        .map(|(family, client, scope, created, expires)| {
            json!({
                "family_id": family,
                "client_id": client,
                "scope": scope,
                "created_at": created,
                "expires_at": expires
            })
        })
        .collect::<Vec<_>>();
    Ok(Json(json!(items)))
}

pub(crate) async fn revoke_session(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(family): axum::extract::Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let done =
        sqlx::query("DELETE FROM reactor.oauth_refresh WHERE family_id = $1 AND operator_id = $2")
            .bind(family)
            .bind(operator.id)
            .execute(&state.pool)
            .await
            .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub(crate) struct McpKeyBody {
    name: String,
    scope: String,
}

pub(crate) async fn list_mcp_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let rows: Vec<(Uuid, String, String, String)> = sqlx::query_as(
        "SELECT id, name, scope, created_at::text FROM reactor.mcp_keys \
         WHERE operator_id = $1 ORDER BY created_at",
    )
    .bind(operator.id)
    .fetch_all(&state.pool)
    .await
    .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    let items = rows
        .into_iter()
        .map(|(id, name, scope, created)| {
            json!({"id": id, "name": name, "scope": scope, "created_at": created})
        })
        .collect::<Vec<_>>();
    Ok(Json(json!(items)))
}

pub(crate) async fn create_mcp_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<McpKeyBody>,
) -> Result<Json<Value>, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let name = body.name.trim();
    if name.is_empty() || name.len() > 80 {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "name is required"));
    }
    if body.scope != "operator" && body.scope != "operator:read" {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid scope"));
    }
    let id = Uuid::new_v4();
    let token = format!("rtr_mcp_{}", random_token());
    sqlx::query(
        "INSERT INTO reactor.mcp_keys (id, operator_id, name, token_hash, scope) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(id)
    .bind(operator.id)
    .bind(name)
    .bind(token_hash(&token))
    .bind(&body.scope)
    .execute(&state.pool)
    .await
    .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(Json(json!({
        "id": id,
        "name": name,
        "scope": body.scope,
        "token": token
    })))
}

pub(crate) async fn revoke_mcp_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let operator = require_operator(&state, &headers).await?;
    let done = sqlx::query("DELETE FROM reactor.mcp_keys WHERE id = $1 AND operator_id = $2")
        .bind(id)
        .bind(operator.id)
        .execute(&state.pool)
        .await
        .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    if done.rows_affected() == 0 {
        return Err(ApiError::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn mcp_key(
    state: &AppState,
    token: &str,
) -> Result<Option<(Uuid, String)>, ApiError> {
    if !token.starts_with("rtr_mcp_") {
        return Ok(None);
    }
    let row: Option<(Uuid, String)> =
        sqlx::query_as("SELECT operator_id, scope FROM reactor.mcp_keys WHERE token_hash = $1")
            .bind(token_hash(token))
            .fetch_optional(&state.pool)
            .await
            .map_err(|err| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, err.to_string()))?;
    Ok(row)
}

pub(crate) fn challenge_header(public_url: &str) -> String {
    format!(
        "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource\"",
        public_url.trim_end_matches('/')
    )
}

pub(crate) async fn principal(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(Uuid, String), ApiError> {
    let token = bearer(headers).ok_or_else(|| ApiError::unauthorized("bearer token required"))?;
    if let Some((operator_id, scope)) = mcp_key(state, &token).await? {
        let _ = load_operator(state, operator_id).await?;
        return Ok((operator_id, scope));
    }
    let claims = state
        .issuer
        .decode(&token)
        .map_err(|_| ApiError::unauthorized("bearer token required"))?;
    let resource = mcp_resource(&state.config.public_url);
    if claims.aud != resource {
        return Err(ApiError::unauthorized("token audience is not this MCP"));
    }
    let issuer = state.config.public_url.trim_end_matches('/');
    if !claims.iss.is_empty() && claims.iss != issuer {
        return Err(ApiError::unauthorized("token issuer is not this cluster"));
    }
    let operator_id = Uuid::parse_str(&claims.sub)
        .map_err(|_| ApiError::unauthorized("bearer token required"))?;
    let _ = load_operator(state, operator_id).await?;
    let scope = if claims.scope.is_empty() {
        "operator".to_string()
    } else {
        claims.scope
    };
    Ok((operator_id, access_level(&scope).to_string()))
}

#[allow(dead_code)]
fn _bearer_used(headers: &HeaderMap) -> Option<String> {
    bearer(headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_the_rfc_vector() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            s256(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn offline_access_is_a_known_scope_and_public_clients_have_no_secret() {
        assert_eq!(
            granted_scope("openid offline_access").unwrap_err(),
            "invalid scope"
        );
        assert_eq!(
            granted_scope("operator:read offline_access").unwrap(),
            "operator:read offline_access"
        );
        assert_eq!(
            granted_scope("offline_access").unwrap(),
            "operator offline_access"
        );
        let body = oauth_error("invalid_client", "client_secret is not used");
        assert_eq!(body["error"], "invalid_client");
        assert!(body["error_description"]
            .as_str()
            .unwrap()
            .contains("client_secret"));
        let meta = server_metadata("http://127.0.0.1:18000");
        assert!(meta["scopes_supported"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item == "offline_access"));
        assert_eq!(meta["token_endpoint_auth_methods_supported"][0], "none");
    }

    #[test]
    fn loopback_http_may_change_port_and_other_http_may_not() {
        assert!(redirect_allowed("http://127.0.0.1:43111/callback"));
        assert!(redirect_allowed("http://localhost/callback"));
        assert!(!redirect_allowed("http://example.com/callback"));
        assert!(!redirect_allowed("http://127.0.0.1/callback#frag"));
        assert!(redirect_matches(
            "http://127.0.0.1/callback",
            "http://127.0.0.1:54321/callback"
        ));
        assert!(!redirect_matches(
            "https://example.com/callback",
            "https://example.com:8443/callback"
        ));
        assert!(redirect_allowed("https://example.com/callback"));
        assert!(redirect_allowed("http://localhost:8787/callback"));
        assert!(redirect_allowed(
            "https://www.cursor.com/agents/mcp/oauth/callback"
        ));
        assert!(redirect_allowed(
            "cursor://anysphere.cursor-mcp/oauth/callback"
        ));
        assert!(redirect_allowed(
            "cursor://anysphere.cursor-mcp/oauth/return"
        ));
        assert!(!redirect_allowed("cursor://evil.example/oauth/callback"));
        assert!(!redirect_allowed(
            "cursor://anysphere.cursor-mcp/oauth/callback?next=1"
        ));
        let state = "eyJzdXJmYWNlIjoibWNwX3Byb2Nlc3MifQ.tgd";
        assert_eq!(
            cursor_desktop_return("https://www.cursor.com/agents/mcp/oauth/callback", state),
            Some("http://localhost:8787/callback")
        );
        assert_eq!(
            cursor_desktop_return("https://cursor.com/agents/mcp/oauth/callback", state),
            Some("http://localhost:8787/callback")
        );
        assert_eq!(
            cursor_desktop_return(
                "https://cursor.com/agents/mcp/oauth/callback",
                "eyJzdXJmYWNlIjoibWNwX3Byb2Nlc3MifQ.tgd18787"
            ),
            Some("http://localhost:18787/callback")
        );
        assert_eq!(
            cursor_desktop_return("https://example.com/agents/mcp/oauth/callback", state),
            None
        );
    }

    #[test]
    fn metadata_urls_on_link_local_and_private_addresses_are_blocked() {
        assert!(host_is_blocked("169.254.169.254"));
        assert!(host_is_blocked("10.1.2.3"));
        assert!(host_is_blocked("127.0.0.1"));
        assert!(!host_is_blocked("example.com"));
    }

    #[test]
    fn enrolled_operator_is_asked_for_a_passkey_or_code() {
        let html = second_factor(
            &Authz {
                client_id: "client".into(),
                redirect_uri: "http://127.0.0.1:9/cb".into(),
                code_challenge: "challenge".into(),
                scope: "operator".into(),
                resource: "http://127.0.0.1:18000/mcp".into(),
                state: "s".into(),
            },
            "mfa-token.example",
        )
        .0;
        assert!(html.contains("Use passkey"), "{html}");
        assert!(html.contains("Authenticator code"), "{html}");
        assert!(html.contains("mfa-token.example"), "{html}");
        assert!(!html.contains("action=\"/oauth/consent\""), "{html}");
    }
}
