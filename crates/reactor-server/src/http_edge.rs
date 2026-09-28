use crate::AppState;
use axum::extract::{ConnectInfo, Request, State};
use axum::http::{header, HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use std::net::SocketAddr;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::trace::TraceLayer;

pub fn wrap(app: Router, state: AppState) -> Router {
    let request_id = HeaderName::from_static("x-request-id");
    app.layer(middleware::from_fn_with_state(state, edge))
        .layer(PropagateRequestIdLayer::new(request_id.clone()))
        .layer(TraceLayer::new_for_http())
        .layer(SetRequestIdLayer::new(request_id, MakeRequestUuid))
}

async fn edge(State(state): State<AppState>, mut request: Request, next: Next) -> Response {
    let ip = client_ip(&state, &request);
    request.headers_mut().remove("x-reactor-client-ip");
    if let Ok(value) = HeaderValue::from_str(&ip) {
        request.headers_mut().insert("x-reactor-client-ip", value);
    }
    let path = request.uri().path().to_string();
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let allowed = match origin.as_deref() {
        Some(origin) if public_api(&path) && origin_allowed(&state, origin).await => {
            Some(origin.to_string())
        }
        _ => None,
    };
    if request.method() == Method::OPTIONS && public_api(&path) {
        let mut response = StatusCode::NO_CONTENT.into_response();
        apply_cors(response.headers_mut(), allowed.as_deref());
        return response;
    }
    let mut response = next.run(request).await;
    apply_cors(response.headers_mut(), allowed.as_deref());
    response
}

fn public_api(path: &str) -> bool {
    path.starts_with("/auth/v1")
        || path.starts_with("/data/v1")
        || path.starts_with("/storage/v1")
        || path.starts_with("/fn/v1")
}

fn client_ip(state: &AppState, request: &Request) -> String {
    if state.config.trusted_proxy {
        if let Some(forwarded) = request
            .headers()
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
        {
            if let Some(ip) = forwarded.split(',').next() {
                let ip = ip.trim();
                if !ip.is_empty() {
                    return ip.to_string();
                }
            }
        }
    }
    request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

async fn origin_allowed(state: &AppState, origin: &str) -> bool {
    let Ok(url) = url::Url::parse(origin) else {
        return false;
    };
    let Some(host) = url.host_str() else {
        return false;
    };
    if host == "localhost" || host == "127.0.0.1" {
        return true;
    }
    let base = &state.config.base_domain;
    if host == base || host.ends_with(&format!(".{base}")) {
        return true;
    }
    let verified: Option<bool> =
        sqlx::query_scalar("SELECT verified_at IS NOT NULL FROM reactor.domains WHERE host = $1")
            .bind(host)
            .fetch_optional(&state.pool)
            .await
            .ok()
            .flatten();
    verified.unwrap_or(false)
}

fn apply_cors(headers: &mut axum::http::HeaderMap, origin: Option<&str>) {
    let Some(origin) = origin else {
        return;
    };
    let Ok(origin) = HeaderValue::from_str(origin) else {
        return;
    };
    headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, PUT, PATCH, DELETE, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Authorization, Content-Type, apikey, Prefer, Range, Accept"),
    );
    headers.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("600"),
    );
    headers.insert(header::VARY, HeaderValue::from_static("Origin"));
}
