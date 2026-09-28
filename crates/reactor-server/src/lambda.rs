use axum::body::Body;
use axum::http::{Request, Response};
use axum::Router;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use tower::{Service, ServiceExt};

pub async fn run_lambda(app: Router) -> anyhow::Result<()> {
    lambda_http::run(LambdaService { app })
        .await
        .map_err(|err| anyhow::anyhow!(err.to_string()))?;
    Ok(())
}

struct LambdaService {
    app: Router,
}

impl Service<lambda_http::Request> for LambdaService {
    type Response = Response<Body>;
    type Error = std::convert::Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: lambda_http::Request) -> Self::Future {
        let app = self.app.clone();
        Box::pin(async move {
            let (parts, body) = req.into_parts();
            let bytes = match body {
                lambda_http::Body::Empty => Vec::new(),
                lambda_http::Body::Text(text) => text.into_bytes(),
                lambda_http::Body::Binary(bytes) => bytes,
            };
            let request = Request::from_parts(parts, Body::from(bytes));
            Ok(app.oneshot(request).await.unwrap_or_else(|_| {
                Response::builder()
                    .status(500)
                    .body(Body::from("internal error"))
                    .unwrap()
            }))
        })
    }
}

pub fn from_apigw_v2(value: &Value) -> Request<Body> {
    let method = value["requestContext"]["http"]["method"]
        .as_str()
        .unwrap_or("GET");
    let path = value["rawPath"].as_str().unwrap_or("/");
    let query = value["rawQueryString"].as_str().unwrap_or("");
    let uri = if query.is_empty() {
        path.to_string()
    } else {
        format!("{path}?{query}")
    };
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(headers) = value.get("headers").and_then(|h| h.as_object()) {
        for (key, val) in headers {
            if let Some(text) = val.as_str() {
                builder = builder.header(key, text);
            }
        }
    }
    let raw = value.get("body").and_then(|b| b.as_str()).unwrap_or("");
    builder.body(Body::from(raw.to_string())).expect("request")
}

pub fn to_apigw_v2(response: Response<Body>, body: &[u8]) -> Value {
    let mut headers = serde_json::Map::new();
    for (key, value) in response.headers() {
        if let Ok(text) = value.to_str() {
            headers.insert(key.to_string(), Value::String(text.to_string()));
        }
    }
    serde_json::json!({
        "statusCode": response.status().as_u16(),
        "headers": headers,
        "body": String::from_utf8_lossy(body),
        "isBase64Encoded": false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_apigw_path_and_method() {
        let req = from_apigw_v2(&serde_json::json!({
            "version": "2.0",
            "rawPath": "/health",
            "rawQueryString": "",
            "headers": {"host": "localhost"},
            "requestContext": {"http": {"method": "GET", "path": "/health"}}
        }));
        assert_eq!(req.method(), "GET");
        assert_eq!(req.uri().path(), "/health");
    }
}
