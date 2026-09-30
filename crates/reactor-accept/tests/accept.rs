use anyhow::Context;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::io::Write;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use uuid::Uuid;

const FN_SOURCE: &str = r#"
const raw = await Bun.stdin.text();
let req = {};
try { req = raw ? JSON.parse(raw) : {}; } catch {}
const caller = JSON.parse(process.env.REACTOR_CALLER || "{}");
if (req.sleep && req.mark) {
  const file = "/tmp/reactor-cron-runs";
  const prev = await Bun.file(file).text().catch(() => "");
  await Bun.write(file, prev + "x");
  await Bun.sleep(req.sleep);
} else if (req.sleep) {
  await Bun.sleep(req.sleep);
}
if (req.read) {
  process.stdout.write(await Bun.file("/tmp/reactor-cron-runs").text().catch(() => ""));
  process.exit(0);
}
process.stdout.write(JSON.stringify({
  caller,
  has_database_url: Boolean(process.env.DATABASE_URL),
  marker: "echo",
}));
"#;

struct LocalDns;

impl Resolve for LocalDns {
    fn resolve(&self, _name: Name) -> Resolving {
        Box::pin(async move {
            let addrs: Addrs = Box::new(std::iter::once(SocketAddr::new(
                IpAddr::V4(Ipv4Addr::LOCALHOST),
                0,
            )));
            Ok(addrs)
        })
    }
}

struct Http {
    client: reqwest::Client,
    operator: String,
}

impl Http {
    fn new(operator: String) -> Self {
        let client = reqwest::Client::builder()
            .dns_resolver(Arc::new(LocalDns))
            .timeout(Duration::from_secs(40))
            .build()
            .unwrap();
        Self { client, operator }
    }

    fn url(port: u16, pref: Option<&str>, path: &str) -> String {
        let path = path.trim_start_matches('/');
        match pref {
            Some(pref) => format!("http://{pref}.apps.localhost:{port}/{path}"),
            None => format!("http://127.0.0.1:{port}/{path}"),
        }
    }

    async fn call(
        &self,
        port: u16,
        pref: Option<&str>,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<Value>,
    ) -> anyhow::Result<(u16, String)> {
        self.call_bytes(
            port,
            pref,
            method,
            path,
            token,
            body.map(|v| ("application/json".into(), v.to_string())),
        )
        .await
    }

    async fn call_bytes(
        &self,
        port: u16,
        pref: Option<&str>,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<(String, String)>,
    ) -> anyhow::Result<(u16, String)> {
        let mut req = self.client.request(
            reqwest::Method::from_bytes(method.as_bytes()).unwrap(),
            Self::url(port, pref, path),
        );
        if let Some(token) = token {
            req = req.bearer_auth(token);
        }
        if let Some((content_type, body)) = body {
            req = req.header("content-type", content_type).body(body);
        }
        let res = req.send().await?;
        let status = res.status().as_u16();
        let text = res.text().await.unwrap_or_default();
        Ok((status, text))
    }
}

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("missing {key}"))
}

async fn until_ok(port: u16, http: &Http) -> anyhow::Result<()> {
    for _ in 0..40 {
        if let Ok((200, _)) = http.call(port, None, "GET", "/health", None, None).await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    anyhow::bail!("health did not become ready on {port}")
}

fn accept_on() -> bool {
    std::env::var("REACTOR_ACCEPT").ok().as_deref() == Some("1")
}

async fn live() -> anyhow::Result<Http> {
    let http = Http::new(env("REACTOR_OPERATOR_TOKEN"));
    until_ok(18000, &http).await?;
    Ok(http)
}

async fn run_gate(gate: impl std::future::Future<Output = anyhow::Result<()>>) {
    if !accept_on() {
        return;
    }
    if let Err(err) = gate.await {
        panic!("{err:#}");
    }
}

#[tokio::test]
async fn gate_health() {
    run_gate(async {
        in_process_health().await?;
        let http = live().await?;
        until_ok(18001, &http).await?;
        let (status, _) = http.call(18000, None, "GET", "/health", None, None).await?;
        assert_eq!(status, 200);
        let res = http
            .client
            .get(Http::url(18000, None, "/health"))
            .send()
            .await?;
        assert!(res.headers().get("x-request-id").is_some());
        let (status, _) = http
            .call(18000, None, "GET", "/auth/v1/user", None, None)
            .await?;
        assert_eq!(status, 401, "missing project");
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn gate_tenancy() {
    run_gate(async {
        let http = live().await?;
        let a = create_project(&http, 18000).await?;
        let b = create_project(&http, 18000).await?;
        let (status, body) = http
            .call(
                18000,
                Some(&b.pref),
                "GET",
                "/auth/v1/user",
                Some(&a.anon),
                None,
            )
            .await?;
        assert_eq!(status, 403, "host/token mismatch: {body}");
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn gate_migrate() {
    run_gate(async {
        let http = live().await?;
        let a = create_project(&http, 18000).await?;
        let b = create_project(&http, 18000).await?;
        let marker = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sql/project/900_accept_marker.sql");
        let _ = std::fs::remove_file(&marker);
        std::fs::write(&marker, "CREATE TABLE IF NOT EXISTS accept_marker (id int);\n")?;
        let migrate = cli_migrate_all()?;
        let _ = std::fs::remove_file(&marker);
        assert!(migrate.status.success(), "{}", String::from_utf8_lossy(&migrate.stderr));
        let shared = PgPool::connect(&env("REACTOR_ACCEPT_DATABASE_URL")).await?;
        for pref in [&a.pref, &b.pref] {
            let n: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM reactor.schema_migrations m JOIN reactor.projects p ON p.id = m.project_id WHERE p.ref = $1 AND m.version = '900_accept_marker.sql'",
            )
            .bind(pref)
            .fetch_one(&shared)
            .await?;
            assert_eq!(n, 1, "migration skipped {pref}");
        }
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn gate_password_session() {
    run_gate(async {
        let http = live().await?;
        let a = create_project(&http, 18000).await?;
        let user = signup(&http, &a).await?;
        let rotated = refresh(&http, &a, &user).await?;
        let (status, body) = http
            .call(
                18000,
                Some(&a.pref),
                "POST",
                "/auth/v1/token",
                None,
                Some(json!({"refresh_token": user.refresh})),
            )
            .await?;
        assert_eq!(status, 401, "old refresh still worked: {body}");
        logout(&http, &a, &rotated).await?;
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn gate_data() {
    run_gate(async {
        let http = live().await?;
        let a = create_project(&http, 18000).await?;
        let b = create_project(&http, 18000).await?;
        let user = signup(&http, &a).await?;
        let shared = PgPool::connect(&env("REACTOR_ACCEPT_DATABASE_URL")).await?;
        data_gate(&http, &a, &b, &user, &shared).await
    })
    .await;
}

#[tokio::test]
async fn gate_storage() {
    run_gate(async {
        let http = live().await?;
        let a = create_project(&http, 18000).await?;
        let b = create_project(&http, 18000).await?;
        storage_gate(&http, &a, &b).await
    })
    .await;
}

#[tokio::test]
async fn gate_functions() {
    run_gate(async {
        let http = live().await?;
        let a = create_project(&http, 18000).await?;
        let user = signup(&http, &a).await?;
        function_gate(&http, &a, &user).await
    })
    .await;
}

#[tokio::test]
async fn gate_sites() {
    run_gate(async {
        let http = live().await?;
        let a = create_project(&http, 18000).await?;
        let b = create_project(&http, 18000).await?;
        sites_gate(&http, &a, &b).await
    })
    .await;
}

#[tokio::test]
async fn gate_server_site() {
    run_gate(async {
        let http = live().await?;
        let a = create_project(&http, 18000).await?;
        let b = create_project(&http, 18000).await?;
        server_site_gate(&http, &a, &b).await
    })
    .await;
}

#[tokio::test]
async fn gate_builder() {
    run_gate(async {
        let http = live().await?;
        builder_gate(&http).await
    })
    .await;
}

#[tokio::test]
async fn gate_dedicated() {
    run_gate(async {
        let http = live().await?;
        let b = create_project(&http, 18000).await?;
        let user = user_on(&http, &b).await?;
        dedicated_gate(&http, &b, &user).await
    })
    .await;
}

#[tokio::test]
async fn gate_replica() {
    run_gate(async {
        let http = live().await?;
        let a = create_project(&http, 18000).await?;
        replica_gate(&http, &a).await
    })
    .await;
}

#[tokio::test]
async fn gate_cli() {
    run_gate(async {
        let http = live().await?;
        cli_deploy(&http).await
    })
    .await;
}

#[tokio::test]
async fn gate_email() {
    run_gate(email_gate()).await;
}

#[tokio::test]
async fn gate_cluster_email() {
    run_gate(cluster_email_gate()).await;
}

#[tokio::test]
async fn gate_console() {
    run_gate(console_gate()).await;
}

#[tokio::test]
async fn gate_cors() {
    run_gate(cors_gate()).await;
}

#[tokio::test]
async fn gate_rate_limit() {
    run_gate(rate_limit_gate()).await;
}

struct Keys {
    pref: String,
    anon: String,
    service: String,
}

struct Session {
    access: String,
    refresh: String,
    user_id: String,
}

async fn create_project(http: &Http, port: u16) -> anyhow::Result<Keys> {
    let (status, body) = http
        .call(
            port,
            None,
            "POST",
            "/platform/v1/projects",
            Some(&http.operator),
            Some(json!({})),
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body)?;
    Ok(Keys {
        pref: v["ref"].as_str().unwrap().to_string(),
        anon: v["anon_key"].as_str().unwrap().to_string(),
        service: v["service_key"].as_str().unwrap().to_string(),
    })
}

async fn allow_password(http: &Http, project: &Keys) -> anyhow::Result<()> {
    let pool = PgPool::connect(&env("REACTOR_ACCEPT_DATABASE_URL")).await?;
    let required: Option<bool> = sqlx::query_scalar(
        "SELECT s.require_email_verification FROM reactor.auth_settings s \
         JOIN reactor.projects p ON p.id = s.project_id WHERE p.ref = $1",
    )
    .bind(&project.pref)
    .fetch_optional(&pool)
    .await?;
    if required == Some(false) {
        return Ok(());
    }
    let token = console_token(&pool, http, project).await?;
    let (status, body) = http
        .call(
            18000,
            None,
            "PUT",
            &format!("/console/v1/projects/{}/auth", project.pref),
            Some(&token),
            Some(json!({"require_email_verification": false, "require_mfa": false})),
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    Ok(())
}

async fn signup(http: &Http, project: &Keys) -> anyhow::Result<Session> {
    allow_password(http, project).await?;
    let email = format!("{}@example.com", &project.pref[..8]);
    let (status, body) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/auth/v1/signup",
            None,
            Some(json!({"email": email, "password": "password123"})),
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body)?;
    let (ustatus, ubody) = http
        .call(
            18000,
            Some(&project.pref),
            "GET",
            "/auth/v1/user",
            Some(v["access_token"].as_str().unwrap()),
            None,
        )
        .await?;
    assert_eq!(ustatus, 200, "{ubody}");
    Ok(Session {
        access: v["access_token"].as_str().unwrap().to_string(),
        refresh: v["refresh_token"].as_str().unwrap().to_string(),
        user_id: v["user"]["id"].as_str().unwrap().to_string(),
    })
}

async fn user_on(http: &Http, project: &Keys) -> anyhow::Result<Session> {
    signup(http, project).await
}

async fn refresh(http: &Http, project: &Keys, session: &Session) -> anyhow::Result<Session> {
    let (status, body) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/auth/v1/token",
            None,
            Some(json!({"refresh_token": session.refresh})),
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body)?;
    assert_ne!(v["refresh_token"].as_str().unwrap(), session.refresh);
    Ok(Session {
        access: v["access_token"].as_str().unwrap().to_string(),
        refresh: v["refresh_token"].as_str().unwrap().to_string(),
        user_id: session.user_id.clone(),
    })
}

async fn logout(http: &Http, project: &Keys, session: &Session) -> anyhow::Result<()> {
    let (status, body) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/auth/v1/logout",
            None,
            Some(json!({"refresh_token": session.refresh})),
        )
        .await?;
    assert_eq!(status, 204, "{body}");
    let (status, _) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/auth/v1/token",
            None,
            Some(json!({"refresh_token": session.refresh})),
        )
        .await?;
    assert_eq!(status, 401);
    Ok(())
}

async fn data_gate(
    http: &Http,
    a: &Keys,
    b: &Keys,
    user: &Session,
    shared: &PgPool,
) -> anyhow::Result<()> {
    let note = json!({"user_id": user.user_id, "body": "hello-a"});
    let (status, body) =
        retry_data(http, &a.pref, "POST", "/data/v1/notes", &user.access, note).await?;
    assert!(
        status == 201 || status == 200,
        "insert note: {status} {body}"
    );
    let (status, body) = retry_data(
        http,
        &a.pref,
        "GET",
        "/data/v1/notes",
        &user.access,
        json!({}),
    )
    .await?;
    // GET with a json body is odd; call without body below if this fails.
    let _ = (status, body);
    let (status, body) = http
        .call(
            18000,
            Some(&a.pref),
            "GET",
            "/data/v1/notes",
            Some(&user.access),
            None,
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("hello-a"), "{body}");
    let (status, body) = http
        .call(
            18000,
            Some(&b.pref),
            "GET",
            "/data/v1/notes",
            Some(&user.access),
            None,
        )
        .await?;
    assert_eq!(status, 403, "cross project read: {body}");
    let (status, body) = http
        .call(
            18000,
            Some(&a.pref),
            "GET",
            "/data/v1/notes",
            Some(&a.anon),
            None,
        )
        .await?;
    assert!(status != 200 || body == "[]", "anon read: {status} {body}");
    let (status, body) = http
        .call(
            18000,
            Some(&a.pref),
            "GET",
            "/data/v1/notes",
            Some(&a.service),
            None,
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("hello-a"), "{body}");
    let auth = PgPool::connect(&env("REACTOR_ACCEPT_AUTHENTICATOR_URL")).await?;
    assert!(sqlx::query("SELECT * FROM notes")
        .fetch_all(&auth)
        .await
        .is_err());
    let (status, body) = http
        .call(
            18000,
            Some(&a.pref),
            "POST",
            "/data/v1/rpc/create_note_pair",
            Some(&user.access),
            Some(json!({"a": "one", "b": "two"})),
        )
        .await?;
    assert!(status == 200 || status == 204, "rpc: {status} {body}");
    let (status, body) = http
        .call(
            18000,
            Some(&a.pref),
            "GET",
            "/data/v1/note_pairs",
            Some(&user.access),
            None,
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let rows: Vec<Value> = serde_json::from_str(&body)?;
    assert_eq!(rows.len(), 2, "{body}");
    let _ = shared;
    Ok(())
}

async fn retry_data(
    http: &Http,
    pref: &str,
    method: &str,
    path: &str,
    token: &str,
    body: Value,
) -> anyhow::Result<(u16, String)> {
    let mut last = String::new();
    for _ in 0..15 {
        let (status, text) = if method == "GET" {
            http.call(18000, Some(pref), method, path, Some(token), None)
                .await?
        } else {
            http.call(
                18000,
                Some(pref),
                method,
                path,
                Some(token),
                Some(body.clone()),
            )
            .await?
        };
        if status < 500 && !text.contains("PGRST") && text != "{}" {
            return Ok((status, text));
        }
        last = format!("{status} {text}");
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    anyhow::bail!("postgrest not ready: {last}")
}

async fn storage_gate(http: &Http, a: &Keys, b: &Keys) -> anyhow::Result<()> {
    let (status, body) = http
        .call(
            18000,
            Some(&a.pref),
            "POST",
            "/storage/v1/object/presign",
            Some(&a.service),
            Some(json!({"bucket": "../bbbbbbbbbbbbbbbbbbbb", "key": "a.txt", "method": "PUT"})),
        )
        .await?;
    assert_eq!(status, 403, "{body}");
    let (status, body) = http
        .call(
            18000,
            Some(&a.pref),
            "POST",
            "/storage/v1/object/presign",
            Some(&a.service),
            Some(json!({"bucket": "files", "key": "a.txt", "method": "PUT"})),
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body)?;
    let key = v["key"].as_str().unwrap();
    assert!(key.starts_with(&format!("{}/files/", a.pref)), "{key}");
    assert!(!key.contains(&b.pref), "{key}");
    let put = http
        .client
        .put(v["url"].as_str().unwrap())
        .body("hello-object")
        .send()
        .await?;
    assert!(
        put.status().is_success(),
        "{}",
        put.text().await.unwrap_or_default()
    );
    let (status, body) = http
        .call(
            18000,
            Some(&a.pref),
            "POST",
            "/storage/v1/object/presign",
            Some(&a.service),
            Some(json!({"bucket": "files", "key": "a.txt", "method": "GET"})),
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body)?;
    let got = http.client.get(v["url"].as_str().unwrap()).send().await?;
    let text = got.text().await?;
    assert_eq!(text, "hello-object");
    Ok(())
}

async fn function_gate(http: &Http, a: &Keys, user: &Session) -> anyhow::Result<()> {
    let zip = reactor_functions::zip_single("index.ts", FN_SOURCE)?;
    let res = http
        .client
        .post(Http::url(
            18000,
            Some(&a.pref),
            "/fn/v1/_admin/functions/echo",
        ))
        .bearer_auth(&a.service)
        .header("content-type", "application/zip")
        .body(zip)
        .send()
        .await?;
    let status = res.status().as_u16();
    let body = res.text().await?;
    assert_eq!(status, 200, "{body}");
    let rm = Command::new("docker")
        .args([
            "exec",
            "reactor-v2-app",
            "sh",
            "-c",
            "rm -rf /data/functions/*",
        ])
        .status()?;
    assert!(rm.success());
    let echoed = invoke(http, 18000, a, &user.access, "{}").await?;
    assert!(echoed.contains("\"marker\":\"echo\""), "{echoed}");
    assert!(echoed.contains(&user.user_id), "{echoed}");
    assert!(echoed.contains("\"has_database_url\":false"), "{echoed}");
    let client = http.client.clone();
    let url = Http::url(18000, Some(&a.pref), "/fn/v1/echo");
    let token = user.access.clone();
    let invoke_sleep = tokio::spawn(async move {
        client
            .post(url)
            .bearer_auth(token)
            .body(r#"{"sleep":2500}"#)
            .send()
            .await
    });
    let mut env_text = String::new();
    for _ in 0..25 {
        let ps = Command::new("docker")
            .args([
                "exec",
                "reactor-v2-app",
                "sh",
                "-c",
                "for d in /proc/[0-9]*; do comm=$(cat $d/comm 2>/dev/null || true); [ \"$comm\" = bun ] || continue; tr '\\0' '\\n' < $d/environ; done",
            ])
            .output()?;
        env_text = String::from_utf8_lossy(&ps.stdout).into_owned();
        if env_text.contains("REACTOR_CALLER") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        env_text.contains("REACTOR_CALLER"),
        "bun env was not visible: {env_text}"
    );
    assert!(
        !env_text.contains("postgres://"),
        "child env leaked database url: {env_text}"
    );
    let slept = invoke_sleep.await??.text().await?;
    assert!(slept.contains("echo"), "{slept}");
    let (status, _) = http
        .call(
            18000,
            Some(&a.pref),
            "POST",
            "/fn/v1/_admin/schedules",
            Some(&a.service),
            Some(json!({"function_name": "echo", "body": "{\"sleep\":1200,\"mark\":true}"})),
        )
        .await?;
    assert_eq!(status, 201);
    let _ = Command::new("docker")
        .args([
            "exec",
            "reactor-v2-app",
            "rm",
            "-f",
            "/tmp/reactor-cron-runs",
        ])
        .status();
    let one = http.call(
        18000,
        None,
        "POST",
        "/fn/v1/_internal/cron",
        Some(&http.operator),
        Some(json!({})),
    );
    let two = http.call(
        18000,
        None,
        "POST",
        "/fn/v1/_internal/cron",
        Some(&http.operator),
        Some(json!({})),
    );
    let (a1, a2) = tokio::join!(one, two);
    let (s1, b1) = a1?;
    let (s2, b2) = a2?;
    assert_eq!(s1, 200, "{b1}");
    assert_eq!(s2, 200, "{b2}");
    let ran = b1.contains("\"ran\":true") as i32 + b2.contains("\"ran\":true") as i32;
    assert_eq!(ran, 1, "{b1} {b2}");
    let read = invoke(http, 18000, a, &user.access, r#"{"read":true}"#).await?;
    assert_eq!(read, "x", "{read}");
    Ok(())
}

async fn invoke(
    http: &Http,
    port: u16,
    project: &Keys,
    token: &str,
    body: &str,
) -> anyhow::Result<String> {
    let res = http
        .client
        .post(Http::url(port, Some(&project.pref), "/fn/v1/echo"))
        .bearer_auth(token)
        .body(body.to_string())
        .send()
        .await?;
    let status = res.status();
    let text = res.text().await?;
    if !status.is_success() {
        anyhow::bail!("invoke {status}: {text}");
    }
    Ok(text)
}

async fn deploy_echo(http: &Http, project: &Keys) -> anyhow::Result<()> {
    let zip = reactor_functions::zip_single("index.ts", FN_SOURCE)?;
    let res = http
        .client
        .post(Http::url(
            18000,
            Some(&project.pref),
            "/fn/v1/_admin/functions/echo",
        ))
        .bearer_auth(&project.service)
        .header("content-type", "application/zip")
        .body(zip)
        .send()
        .await?;
    let status = res.status().as_u16();
    let body = res.text().await?;
    assert_eq!(status, 200, "{body}");
    Ok(())
}

async fn sites_gate(http: &Http, a: &Keys, b: &Keys) -> anyhow::Result<()> {
    deploy_echo(http, a).await?;
    let res = http
        .client
        .put(Http::url(
            18000,
            Some(&a.pref),
            "/sites/v1/files/index.html",
        ))
        .bearer_auth(&a.service)
        .body("hello-site")
        .send()
        .await?;
    assert!(
        res.status().is_success(),
        "{}",
        res.text().await.unwrap_or_default()
    );
    let (status, body) = http
        .call(
            18000,
            Some(&a.pref),
            "POST",
            "/sites/v1/routes",
            Some(&a.service),
            Some(json!({"path": "/echo", "function": "echo"})),
        )
        .await?;
    assert_eq!(status, 201, "{body}");
    let page = http
        .client
        .get(Http::url(18000, Some(&a.pref), "/"))
        .send()
        .await?;
    let status = page.status();
    let text = page.text().await?;
    assert!(
        status.is_success() && text.contains("hello-site"),
        "{status} {text}"
    );
    let other = http
        .client
        .get(Http::url(18000, Some(&b.pref), "/"))
        .send()
        .await?;
    assert_eq!(other.status().as_u16(), 404);
    let func = http
        .client
        .get(Http::url(18000, Some(&a.pref), "/echo"))
        .send()
        .await?;
    let text = func.text().await?;
    assert!(text.contains("echo"), "{text}");
    let host = format!("custom-{}.example", a.pref);
    let (status, body) = http
        .call(
            18000,
            Some(&a.pref),
            "POST",
            "/sites/v1/domains",
            Some(&a.service),
            Some(json!({"host": host})),
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body)?;
    let stub = PathBuf::from(env("REACTOR_DNS_STUB_HOST"));
    if let Some(parent) = stub.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let name = v["txt_name"].as_str().unwrap();
    let value = v["txt_value"].as_str().unwrap();
    std::fs::write(&stub, serde_json::to_string(&json!({ name: value }))?)?;
    let (status, body) = http
        .call(
            18000,
            Some(&a.pref),
            "POST",
            &format!("/sites/v1/domains/{host}/verify"),
            Some(&a.service),
            None,
        )
        .await?;
    assert_eq!(status, 204, "{body}");
    let (status, _) = http
        .call(
            18000,
            None,
            "GET",
            &format!("/ask?domain={host}"),
            None,
            None,
        )
        .await?;
    assert_eq!(status, 200);
    let (status, _) = http
        .call(
            18000,
            Some(&a.pref),
            "DELETE",
            &format!("/sites/v1/domains/{host}"),
            Some(&a.service),
            None,
        )
        .await?;
    assert_eq!(status, 204);
    let (status, _) = http
        .call(
            18000,
            None,
            "GET",
            &format!("/ask?domain={host}"),
            None,
            None,
        )
        .await?;
    assert_eq!(status, 404);
    Ok(())
}

const NODE_SERVER: &str = r#"
const http = require("http");
const server = http.createServer((req, res) => {
  res.setHeader("content-type", "application/json");
  res.end(JSON.stringify({ token: process.env.TOKEN || "", path: req.url, pid: process.pid }));
});
server.listen(Number(process.env.PORT), "127.0.0.1");
"#;

const BUN_SERVER: &str = r#"
Bun.serve({
  port: Number(process.env.PORT),
  hostname: "127.0.0.1",
  fetch(req) {
    const url = new URL(req.url);
    return Response.json({ token: process.env.TOKEN || "", path: url.pathname, pid: process.pid });
  },
});
"#;

async fn server_site_gate(http: &Http, a: &Keys, b: &Keys) -> anyhow::Result<()> {
    let (status, body) = http
        .call(
            18000,
            Some(&a.pref),
            "PUT",
            "/sites/v1/env",
            Some(&a.service),
            Some(json!({"key": "TOKEN", "value": "alpha"})),
        )
        .await?;
    assert_eq!(status, 204, "{body}");
    finish_files(
        http,
        a,
        &[("index.html", "from-a"), ("server.js", NODE_SERVER)],
        "node server.js",
    )
    .await?;
    let (status, body) = http
        .call_bytes(
            18000,
            Some(&b.pref),
            "PUT",
            "/sites/v1/files/index.html",
            Some(&b.service),
            Some(("text/html".into(), "from-b".into())),
        )
        .await?;
    assert_eq!(status, 201, "{body}");
    let a_page = http
        .client
        .get(Http::url(18000, Some(&a.pref), "/"))
        .send()
        .await?
        .text()
        .await?;
    let b_page = http
        .client
        .get(Http::url(18000, Some(&b.pref), "/"))
        .send()
        .await?
        .text()
        .await?;
    assert!(a_page.contains("from-a"), "{a_page}");
    assert!(b_page.contains("from-b"), "{b_page}");
    assert!(!a_page.contains("from-b"));
    let host = format!("alias-{}.example", a.pref);
    verify_host(http, a, &host).await?;
    let custom = http
        .client
        .get(format!("http://{host}:18000/"))
        .send()
        .await?
        .text()
        .await?;
    assert!(
        custom.contains("from-a") && !custom.contains("from-b"),
        "{custom}"
    );
    let first = site_json(http, &a.pref, "/ping").await?;
    let second = site_json(http, &a.pref, "/ping").await?;
    assert_eq!(first["token"], "alpha");
    assert_eq!(first["path"], "/ping");
    assert_eq!(first["pid"], second["pid"]);
    let pid = first["pid"].as_u64().context("pid")?.to_string();
    tokio::time::sleep(Duration::from_secs(5)).await;
    let gone = Command::new("docker")
        .args([
            "exec",
            "reactor-v2-app",
            "sh",
            "-c",
            &format!("ps -p {pid} >/dev/null 2>&1 && echo alive || echo gone"),
        ])
        .output()?;
    assert!(
        gone.status.success(),
        "{}",
        String::from_utf8_lossy(&gone.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&gone.stdout).trim(), "gone");

    let bun = create_project(http, 18000).await?;
    let (status, body) = http
        .call(
            18000,
            Some(&bun.pref),
            "PUT",
            "/sites/v1/env",
            Some(&bun.service),
            Some(json!({"key": "TOKEN", "value": "beta"})),
        )
        .await?;
    assert_eq!(status, 204, "{body}");
    finish_files(http, &bun, &[("server.ts", BUN_SERVER)], "bun server.ts").await?;
    let echoed = site_json(http, &bun.pref, "/ping").await?;
    let again = site_json(http, &bun.pref, "/ping").await?;
    assert_eq!(echoed["token"], "beta");
    assert_eq!(echoed["path"], "/ping");
    assert_eq!(echoed["pid"], again["pid"]);
    Ok(())
}

async fn builder_gate(http: &Http) -> anyhow::Result<()> {
    let project = create_project(http, 18000).await?;
    let (status, body) = http
        .call(
            18000,
            Some(&project.pref),
            "PUT",
            "/sites/v1/env",
            Some(&project.service),
            Some(json!({"key": "TOKEN", "value": "built-secret"})),
        )
        .await?;
    assert_eq!(status, 204, "{body}");
    let dir = std::env::temp_dir().join(format!("reactor-build-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&dir)?;
    let first = dir.join("first.zip");
    std::fs::write(
        &first,
        zip_files(&[
            (
                "reactor.toml",
                "[build]\ncommand = \"python3 build.py\"\n\n[site]\ndir = \"site\"\n",
            ),
            (
                "build.py",
                "import os, pathlib\npathlib.Path('site').mkdir(exist_ok=True)\npathlib.Path('site/index.html').write_text(os.environ['TOKEN'])\npathlib.Path('site/extra.txt').write_text('extra-file')\n",
            ),
        ]),
    )?;
    run_builder(&project, &first)?;
    let page = http
        .client
        .get(Http::url(18000, Some(&project.pref), "/"))
        .send()
        .await?
        .text()
        .await?;
    assert_eq!(page, "built-secret");
    let second = dir.join("second.zip");
    std::fs::write(
        &second,
        zip_files(&[
            ("reactor.toml", "[site]\ncommand = \"node server.js\"\n"),
            ("site/server.js", NODE_SERVER),
        ]),
    )?;
    run_builder(&project, &second)?;
    let echoed = site_json(http, &project.pref, "/ping").await?;
    assert_eq!(echoed["token"], "built-secret");
    let extra = http
        .client
        .get(Http::url(18000, Some(&project.pref), "/extra.txt"))
        .send()
        .await?
        .text()
        .await?;
    assert!(!extra.contains("extra-file"), "{extra}");
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

fn run_builder(project: &Keys, zip_path: &Path) -> anyhow::Result<()> {
    let out = Command::new(env("REACTOR_BUILDER"))
        .args([
            "--url",
            "http://127.0.0.1:18000",
            "--key",
            &project.service,
            "--zip",
            zip_path.to_str().unwrap(),
        ])
        .output()?;
    if !out.status.success() {
        anyhow::bail!(
            "builder failed: {}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

async fn finish_files(
    http: &Http,
    project: &Keys,
    files: &[(&str, &str)],
    command: &str,
) -> anyhow::Result<()> {
    let (status, body) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/sites/v1/deployments",
            Some(&project.service),
            None,
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let started: Value = serde_json::from_str(&body)?;
    let id = started["id"].as_str().unwrap();
    for (path, contents) in files {
        let (status, body) = http
            .call_bytes(
                18000,
                Some(&project.pref),
                "PUT",
                &format!("/sites/v1/deployments/{id}/files/{path}"),
                Some(&project.service),
                Some(("application/octet-stream".into(), (*contents).into())),
            )
            .await?;
        assert_eq!(status, 201, "{path} {body}");
    }
    let (status, body) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            &format!("/sites/v1/deployments/{id}/finish"),
            Some(&project.service),
            Some(json!({"command": command})),
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    Ok(())
}

async fn verify_host(http: &Http, project: &Keys, host: &str) -> anyhow::Result<()> {
    let (status, body) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/sites/v1/domains",
            Some(&project.service),
            Some(json!({"host": host})),
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body)?;
    let stub = PathBuf::from(env("REACTOR_DNS_STUB_HOST"));
    let name = v["txt_name"].as_str().unwrap();
    let value = v["txt_value"].as_str().unwrap();
    std::fs::write(&stub, serde_json::to_string(&json!({ name: value }))?)?;
    let (status, body) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            &format!("/sites/v1/domains/{host}/verify"),
            Some(&project.service),
            None,
        )
        .await?;
    assert_eq!(status, 204, "{body}");
    Ok(())
}

async fn site_json(http: &Http, pref: &str, path: &str) -> anyhow::Result<Value> {
    let res = http
        .client
        .get(Http::url(18000, Some(pref), path))
        .send()
        .await?;
    let status = res.status();
    let text = res.text().await?;
    if !status.is_success() {
        anyhow::bail!("site {path} {status}: {text}");
    }
    serde_json::from_str(&text).with_context(|| text)
}

fn zip_files(files: &[(&str, &str)]) -> Vec<u8> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let opts = zip::write::SimpleFileOptions::default();
        for (name, body) in files {
            writer.start_file(*name, opts).unwrap();
            writer.write_all(body.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
    }
    cursor.into_inner()
}

async fn dedicated_gate(http: &Http, project: &Keys, user: &Session) -> anyhow::Result<()> {
    let (status, body) = http
        .call(
            18000,
            None,
            "PATCH",
            &format!("/platform/v1/projects/{}", project.pref),
            Some(&http.operator),
            Some(json!({"database_url": env("REACTOR_ACCEPT_PATCH_DATABASE_URL")})),
        )
        .await?;
    assert_eq!(status, 204, "{body}");
    let (status, body) = retry_data(
        http,
        &project.pref,
        "POST",
        "/data/v1/notes",
        &user.access,
        json!({"user_id": user.user_id, "body": "dedicated-note"}),
    )
    .await?;
    assert!(
        status == 201 || status == 200,
        "dedicated insert: {status} {body}"
    );
    let schema = format!("proj_{}", project.pref);
    let shared = PgPool::connect(&env("REACTOR_ACCEPT_DATABASE_URL")).await?;
    let dedicated = PgPool::connect(&env("REACTOR_ACCEPT_DEDICATED_URL")).await?;
    let shared_n: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM {schema}.notes WHERE body = 'dedicated-note'"
    ))
    .fetch_one(&shared)
    .await
    .unwrap_or(0);
    let dedicated_n: i64 = sqlx::query_scalar(&format!(
        "SELECT count(*) FROM {schema}.notes WHERE body = 'dedicated-note'"
    ))
    .fetch_one(&dedicated)
    .await?;
    assert_eq!(shared_n, 0);
    assert_eq!(dedicated_n, 1);
    Ok(())
}

async fn replica_gate(http: &Http, project: &Keys) -> anyhow::Result<()> {
    deploy_echo(http, project).await?;
    let (status, body) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/storage/v1/object/presign",
            Some(&project.service),
            Some(json!({"bucket": "files", "key": "replica.txt", "method": "PUT"})),
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body)?;
    let put = http
        .client
        .put(v["url"].as_str().unwrap())
        .body("from-replica-1")
        .send()
        .await?;
    assert!(
        put.status().is_success(),
        "{}",
        put.text().await.unwrap_or_default()
    );
    let (status, body) = http
        .call(
            18001,
            Some(&project.pref),
            "POST",
            "/storage/v1/object/presign",
            Some(&project.service),
            Some(json!({"bucket": "files", "key": "replica.txt", "method": "GET"})),
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body)?;
    let got = http
        .client
        .get(v["url"].as_str().unwrap())
        .send()
        .await?
        .text()
        .await?;
    assert_eq!(got, "from-replica-1");
    let echoed = invoke(http, 18001, project, &project.service, "{}").await?;
    assert!(echoed.contains("echo"), "{echoed}");
    Ok(())
}

async fn console_token(pool: &PgPool, http: &Http, project: &Keys) -> anyhow::Result<String> {
    let email = format!("ops-{}@example.com", &project.pref[..8]);
    if let Some(id) =
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM reactor.operators WHERE email = $1")
            .bind(&email)
            .fetch_optional(pool)
            .await?
    {
        let code = format!("accept-{id}-{}", Uuid::new_v4());
        sqlx::query(
            "INSERT INTO reactor.operator_recovery_codes (operator_id, code_hash) VALUES ($1, $2)",
        )
        .bind(id)
        .bind(reactor_auth::token_hash(&code))
        .execute(pool)
        .await?;
        return console_login(http, &email, &code).await;
    }
    let hash = reactor_auth::hash_password("scripted-pass")?;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO reactor.operators (id, email, name, password_hash, platform_admin) VALUES ($1, $2, 'Accept', $3, false)",
    )
    .bind(id)
    .bind(&email)
    .bind(&hash)
    .execute(pool)
    .await?;
    let project_id: Uuid = sqlx::query_scalar("SELECT id FROM reactor.projects WHERE ref = $1")
        .bind(&project.pref)
        .fetch_one(pool)
        .await?;
    sqlx::query(
        "INSERT INTO reactor.memberships (operator_id, project_id, role) VALUES ($1, $2, 'owner')",
    )
    .bind(id)
    .bind(project_id)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO reactor.operator_factors (operator_id, kind, secret) VALUES ($1, 'totp', 'test')")
        .bind(id)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO reactor.operator_factors (operator_id, kind, credential) VALUES ($1, 'passkey', $2)")
        .bind(id)
        .bind(json!({}))
        .execute(pool)
        .await?;
    let code = format!("accept-{id}");
    sqlx::query(
        "INSERT INTO reactor.operator_recovery_codes (operator_id, code_hash) VALUES ($1, $2)",
    )
    .bind(id)
    .bind(reactor_auth::token_hash(&code))
    .execute(pool)
    .await?;
    console_login(http, &email, &code).await
}

async fn console_login(http: &Http, email: &str, code: &str) -> anyhow::Result<String> {
    let (status, body) = http
        .call(
            18000,
            None,
            "POST",
            "/console/v1/login",
            None,
            Some(json!({"email": email, "password": "scripted-pass"})),
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let body: Value = serde_json::from_str(&body)?;
    let mfa = body["mfa_token"]
        .as_str()
        .context("console login did not ask for a second factor")?;
    let (status, session) = http
        .call(
            18000,
            None,
            "POST",
            "/console/v1/mfa/recovery",
            Some(mfa),
            Some(json!({"code": code})),
        )
        .await?;
    assert_eq!(status, 200, "{session}");
    let session: Value = serde_json::from_str(&session)?;
    session["access_token"]
        .as_str()
        .map(str::to_string)
        .context("console login did not return a token")
}

async fn console_gate() -> anyhow::Result<()> {
    let http = live().await?;
    let project = create_project(&http, 18000).await?;
    let pool = PgPool::connect(&env("REACTOR_ACCEPT_DATABASE_URL")).await?;
    let token = console_token(&pool, &http, &project).await?;
    for path in [
        "overview",
        "schema",
        "functions",
        "sites",
        "logs",
        "users",
        "email",
    ] {
        let (status, body) = http
            .call(
                18000,
                None,
                "GET",
                &format!("/console/v1/projects/{}/{path}", project.pref),
                Some(&token),
                None,
            )
            .await?;
        assert_eq!(status, 200, "{path}: {body}");
    }
    let (status, body) = http
        .call(
            18000,
            None,
            "POST",
            &format!("/console/v1/projects/{}/keys", project.pref),
            Some(&token),
            Some(json!({})),
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let (status, body) = http
        .call(
            18000,
            None,
            "DELETE",
            &format!("/console/v1/projects/{}", project.pref),
            Some(&token),
            None,
        )
        .await?;
    assert_eq!(status, 204, "{body}");
    Ok(())
}

fn token_from(text: &str) -> Option<String> {
    let rest = text.split("token=").nth(1)?;
    let token: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

async fn mail_bodies(to: &str) -> anyhow::Result<Vec<String>> {
    let res = reqwest::get(format!("http://127.0.0.1:8025/api/v1/search?query=to:{to}")).await?;
    let body: Value = res.json().await?;
    let Some(messages) = body["messages"].as_array() else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for message in messages {
        let id = message["ID"]
            .as_str()
            .or_else(|| message["id"].as_str())
            .unwrap_or("");
        if id.is_empty() {
            continue;
        }
        let detail: Value = reqwest::get(format!("http://127.0.0.1:8025/api/v1/message/{id}"))
            .await?
            .json()
            .await?;
        let text = detail["Text"]
            .as_str()
            .or_else(|| detail["text"].as_str())
            .unwrap_or("");
        let html = detail["HTML"]
            .as_str()
            .or_else(|| detail["html"].as_str())
            .unwrap_or("");
        out.push(format!("{text}\n{html}"));
    }
    Ok(out)
}

async fn mail_token(to: &str) -> anyhow::Result<String> {
    for _ in 0..40 {
        for body in mail_bodies(to).await? {
            if let Some(token) = token_from(&body) {
                return Ok(token);
            }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    anyhow::bail!("no mail for {to}")
}

struct SavedCluster {
    present: bool,
    host: String,
    port: i32,
    username: String,
    password_enc: String,
    from_address: String,
    tls: String,
}

async fn read_cluster(pool: &PgPool) -> anyhow::Result<SavedCluster> {
    let row: Option<(String, i32, String, String, String, String)> = sqlx::query_as(
        "SELECT host, port, username, password_enc, from_address, tls FROM reactor.cluster_email WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(match row {
        Some((host, port, username, password_enc, from_address, tls)) => SavedCluster {
            present: true,
            host,
            port,
            username,
            password_enc,
            from_address,
            tls,
        },
        None => SavedCluster {
            present: false,
            host: String::new(),
            port: 587,
            username: String::new(),
            password_enc: String::new(),
            from_address: String::new(),
            tls: "starttls".into(),
        },
    })
}

async fn write_cluster(pool: &PgPool, saved: &SavedCluster) -> anyhow::Result<()> {
    if !saved.present {
        sqlx::query("DELETE FROM reactor.cluster_email WHERE id = 1")
            .execute(pool)
            .await?;
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO reactor.cluster_email (id, host, port, username, password_enc, from_address, tls) \
         VALUES (1, $1, $2, $3, $4, $5, $6) \
         ON CONFLICT (id) DO UPDATE SET host = EXCLUDED.host, port = EXCLUDED.port, username = EXCLUDED.username, \
         password_enc = EXCLUDED.password_enc, from_address = EXCLUDED.from_address, tls = EXCLUDED.tls",
    )
    .bind(&saved.host)
    .bind(saved.port)
    .bind(&saved.username)
    .bind(&saved.password_enc)
    .bind(&saved.from_address)
    .bind(&saved.tls)
    .execute(pool)
    .await?;
    Ok(())
}

async fn platform_token(pool: &PgPool, http: &Http, project: &Keys) -> anyhow::Result<String> {
    let email = format!("admin-{}@example.com", &project.pref[..8]);
    if let Some(id) =
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM reactor.operators WHERE email = $1")
            .bind(&email)
            .fetch_optional(pool)
            .await?
    {
        let code = format!("accept-{id}-{}", Uuid::new_v4());
        sqlx::query(
            "INSERT INTO reactor.operator_recovery_codes (operator_id, code_hash) VALUES ($1, $2)",
        )
        .bind(id)
        .bind(reactor_auth::token_hash(&code))
        .execute(pool)
        .await?;
        return console_login(http, &email, &code).await;
    }
    let hash = reactor_auth::hash_password("scripted-pass")?;
    let id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO reactor.operators (id, email, name, password_hash, platform_admin) VALUES ($1, $2, 'Accept', $3, true)",
    )
    .bind(id)
    .bind(&email)
    .bind(&hash)
    .execute(pool)
    .await?;
    sqlx::query("INSERT INTO reactor.operator_factors (operator_id, kind, secret) VALUES ($1, 'totp', 'test')")
        .bind(id)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO reactor.operator_factors (operator_id, kind, credential) VALUES ($1, 'passkey', $2)")
        .bind(id)
        .bind(json!({}))
        .execute(pool)
        .await?;
    let code = format!("accept-{id}");
    sqlx::query(
        "INSERT INTO reactor.operator_recovery_codes (operator_id, code_hash) VALUES ($1, $2)",
    )
    .bind(id)
    .bind(reactor_auth::token_hash(&code))
    .execute(pool)
    .await?;
    console_login(http, &email, &code).await
}

fn status_is(status: u16, want: u16, body: &str) -> anyhow::Result<()> {
    if status != want {
        anyhow::bail!("status {status}, want {want}: {body}");
    }
    Ok(())
}

async fn latest_mail(to: &str) -> anyhow::Result<(String, String)> {
    for _ in 0..40 {
        let res =
            reqwest::get(format!("http://127.0.0.1:8025/api/v1/search?query=to:{to}")).await?;
        let body: Value = res.json().await?;
        let Some(messages) = body["messages"].as_array() else {
            tokio::time::sleep(Duration::from_millis(250)).await;
            continue;
        };
        let Some(message) = messages.first() else {
            tokio::time::sleep(Duration::from_millis(250)).await;
            continue;
        };
        let id = message["ID"]
            .as_str()
            .or_else(|| message["id"].as_str())
            .unwrap_or("");
        if id.is_empty() {
            tokio::time::sleep(Duration::from_millis(250)).await;
            continue;
        }
        let detail: Value = reqwest::get(format!("http://127.0.0.1:8025/api/v1/message/{id}"))
            .await?
            .json()
            .await?;
        let from = detail
            .pointer("/From/Address")
            .or_else(|| detail.pointer("/from/Address"))
            .or_else(|| detail.pointer("/From/address"))
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_string();
        let text = detail["Text"]
            .as_str()
            .or_else(|| detail["text"].as_str())
            .unwrap_or("");
        let html = detail["HTML"]
            .as_str()
            .or_else(|| detail["html"].as_str())
            .unwrap_or("");
        if !from.is_empty() {
            return Ok((from, format!("{text}\n{html}")));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    anyhow::bail!("no mail for {to}")
}

async fn cluster_email_gate() -> anyhow::Result<()> {
    let http = live().await?;
    let pool = PgPool::connect(&env("REACTOR_ACCEPT_DATABASE_URL")).await?;
    let saved = read_cluster(&pool).await?;
    let result = cluster_email_steps(&http, &pool).await;
    write_cluster(&pool, &saved).await?;
    result
}

async fn cluster_email_steps(http: &Http, pool: &PgPool) -> anyhow::Result<()> {
    let project = create_project(http, 18000).await?;
    let owner = console_token(pool, http, &project).await?;
    let invitee = format!("invite-c-{}@example.com", &project.pref[..8]);
    let (status, body) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/auth/v1/invite",
            Some(&project.service),
            Some(json!({"email": invitee})),
        )
        .await?;
    status_is(status, 503, &body)?;
    let (status, body) = http
        .call(
            18000,
            None,
            "POST",
            &format!("/console/v1/projects/{}/email/test", project.pref),
            Some(&owner),
            Some(json!({"to": invitee})),
        )
        .await?;
    status_is(status, 503, &body)?;
    let (status, body) = http
        .call(
            18000,
            None,
            "PUT",
            "/console/v1/cluster/email",
            Some(&owner),
            Some(json!({
                "host": "mailpit",
                "port": 1025,
                "username": "",
                "password": "cluster-secret",
                "from_address": "cluster@example.com",
                "tls": "none"
            })),
        )
        .await?;
    status_is(status, 403, &body)?;
    let (status, body) = http
        .call(
            18000,
            None,
            "PUT",
            &format!("/console/v1/projects/{}/email/cluster", project.pref),
            Some(&owner),
            Some(json!({"enabled": true})),
        )
        .await?;
    status_is(status, 403, &body)?;
    let admin = platform_token(pool, http, &project).await?;
    let (status, body) = http
        .call(
            18000,
            None,
            "PUT",
            "/console/v1/cluster/email",
            Some(&admin),
            Some(json!({
                "host": "mailpit",
                "port": 1025,
                "username": "",
                "password": "cluster-secret",
                "from_address": "cluster@example.com",
                "tls": "none"
            })),
        )
        .await?;
    status_is(status, 200, &body)?;
    if body.contains("password_enc") || body.contains("cluster-secret") {
        anyhow::bail!("cluster mail response exposed the password: {body}");
    }
    let saved_mail: Value = serde_json::from_str(&body)?;
    if saved_mail["password_set"] != true {
        anyhow::bail!("password_set: {body}");
    }
    let (status, body) = http
        .call(
            18000,
            None,
            "GET",
            "/console/v1/cluster/email",
            Some(&admin),
            None,
        )
        .await?;
    status_is(status, 200, &body)?;
    if body.contains("password_enc") || body.contains("cluster-secret") {
        anyhow::bail!("cluster mail GET exposed the password: {body}");
    }
    let (status, body) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/auth/v1/invite",
            Some(&project.service),
            Some(json!({"email": format!("invite-off-{}@example.com", &project.pref[..8])})),
        )
        .await?;
    status_is(status, 503, &body)?;
    let (status, body) = http
        .call(
            18000,
            None,
            "PUT",
            &format!("/console/v1/projects/{}/email/cluster", project.pref),
            Some(&admin),
            Some(json!({"enabled": true})),
        )
        .await?;
    status_is(status, 200, &body)?;
    let (status, body) = http
        .call(
            18000,
            None,
            "PUT",
            &format!("/console/v1/projects/{}/email", project.pref),
            Some(&owner),
            Some(json!({
                "host": "",
                "port": 587,
                "username": "",
                "from_address": "",
                "tls": "starttls",
                "link_base": "http://127.0.0.1/cb"
            })),
        )
        .await?;
    status_is(status, 200, &body)?;
    let listed: Value = serde_json::from_str(&body)?;
    if listed["source"] != "cluster" || listed["cluster_from"] != "cluster@example.com" {
        anyhow::bail!("expected cluster source: {body}");
    }
    if body.contains("password_enc") || body.contains("mailpit") {
        anyhow::bail!("project mail exposed the cluster server: {body}");
    }
    let magic = format!("magic-c-{}@example.com", &project.pref[..8]);
    let (status, body) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/auth/v1/magic-link",
            Some(&project.anon),
            Some(json!({"email": magic})),
        )
        .await?;
    status_is(status, 200, &body)?;
    let (from, text) = latest_mail(&magic).await?;
    if from != "cluster@example.com" {
        anyhow::bail!("cluster from was {from}");
    }
    if !text.contains("http://127.0.0.1/cb") {
        anyhow::bail!("link base missing: {text}");
    }
    let (status, body) = http
        .call(
            18000,
            None,
            "PUT",
            &format!("/console/v1/projects/{}/email", project.pref),
            Some(&owner),
            Some(json!({
                "host": "mailpit",
                "port": 1025,
                "username": "",
                "from_address": "project@example.com",
                "tls": "none",
                "link_base": "http://127.0.0.1/cb"
            })),
        )
        .await?;
    status_is(status, 200, &body)?;
    let own: Value = serde_json::from_str(&body)?;
    if own["source"] != "project" {
        anyhow::bail!("expected project source: {body}");
    }
    let owned = format!("own-{}@example.com", &project.pref[..8]);
    let (status, body) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/auth/v1/magic-link",
            Some(&project.anon),
            Some(json!({"email": owned})),
        )
        .await?;
    status_is(status, 200, &body)?;
    let (from, _) = latest_mail(&owned).await?;
    if from != "project@example.com" {
        anyhow::bail!("project from was {from}");
    }
    let (status, body) = http
        .call(
            18000,
            None,
            "PUT",
            &format!("/console/v1/projects/{}/email", project.pref),
            Some(&owner),
            Some(json!({
                "host": "",
                "port": 587,
                "username": "",
                "from_address": "",
                "tls": "starttls",
                "link_base": "http://127.0.0.1/cb"
            })),
        )
        .await?;
    status_is(status, 200, &body)?;
    let again = format!("again-{}@example.com", &project.pref[..8]);
    let (status, body) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/auth/v1/magic-link",
            Some(&project.anon),
            Some(json!({"email": again})),
        )
        .await?;
    status_is(status, 200, &body)?;
    let (from, _) = latest_mail(&again).await?;
    if from != "cluster@example.com" {
        anyhow::bail!("fallback from was {from}");
    }
    let (status, body) = http
        .call(
            18000,
            None,
            "PUT",
            &format!("/console/v1/projects/{}/email/cluster", project.pref),
            Some(&admin),
            Some(json!({"enabled": false})),
        )
        .await?;
    status_is(status, 200, &body)?;
    let (status, body) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/auth/v1/invite",
            Some(&project.service),
            Some(json!({"email": format!("invite-end-{}@example.com", &project.pref[..8])})),
        )
        .await?;
    status_is(status, 503, &body)?;
    Ok(())
}

async fn email_gate() -> anyhow::Result<()> {
    let http = live().await?;
    let project = create_project(&http, 18000).await?;
    let pool = PgPool::connect(&env("REACTOR_ACCEPT_DATABASE_URL")).await?;
    let token = console_token(&pool, &http, &project).await?;
    let invitee = format!("invite-{}@example.com", &project.pref[..8]);
    let (status, body) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/auth/v1/invite",
            Some(&project.service),
            Some(json!({"email": invitee})),
        )
        .await?;
    assert_eq!(status, 503, "invite without smtp: {body}");
    let (status, body) = http
        .call(
            18000,
            None,
            "PUT",
            &format!("/console/v1/projects/{}/email", project.pref),
            Some(&token),
            Some(json!({
                "host": "mailpit",
                "port": 1025,
                "username": "",
                "password": "",
                "from_address": "reactor@example.com",
                "tls": "none",
                "link_base": "http://127.0.0.1/cb"
            })),
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let magic = format!("magic-{}@example.com", &project.pref[..8]);
    let (status, _) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/auth/v1/magic-link",
            Some(&project.anon),
            Some(json!({"email": magic})),
        )
        .await?;
    assert_eq!(status, 200);
    let link = mail_token(&magic).await?;
    let (status, _) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/auth/v1/magic-link",
            Some(&project.anon),
            Some(json!({"email": magic})),
        )
        .await?;
    assert_eq!(status, 200);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert_eq!(
        mail_bodies(&magic).await?.len(),
        1,
        "resend inside 60 seconds sent another message"
    );
    let (status, body) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/auth/v1/verify",
            Some(&project.anon),
            Some(json!({"token": link})),
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let session: Value = serde_json::from_str(&body)?;
    let (status, user) = http
        .call(
            18000,
            Some(&project.pref),
            "GET",
            "/auth/v1/user",
            Some(session["access_token"].as_str().unwrap()),
            None,
        )
        .await?;
    assert_eq!(status, 200, "{user}");
    let user: Value = serde_json::from_str(&user)?;
    assert!(user["email_verified_at"].as_str().is_some(), "{user}");
    let (status, _) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/auth/v1/verify",
            Some(&project.anon),
            Some(json!({"token": link})),
        )
        .await?;
    assert_eq!(status, 401);
    let recover = format!("recover-{}@example.com", &project.pref[..8]);
    signup_as(&http, &project, &recover).await?;
    let (status, _) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/auth/v1/recover",
            Some(&project.anon),
            Some(json!({"email": recover})),
        )
        .await?;
    assert_eq!(status, 200);
    let reset = mail_token(&recover).await?;
    let (status, body) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/auth/v1/recover/complete",
            Some(&project.anon),
            Some(json!({"token": reset, "password": "password456"})),
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let (status, body) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/auth/v1/invite",
            Some(&project.service),
            Some(json!({"email": invitee})),
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let invited = mail_token(&invitee).await?;
    let (status, body) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/auth/v1/invite/accept",
            Some(&project.anon),
            Some(json!({"token": invited, "password": "password456"})),
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let (status, _) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/auth/v1/invite",
            Some(&project.service),
            Some(json!({"email": invitee})),
        )
        .await?;
    assert_eq!(status, 409);
    Ok(())
}

async fn signup_as(http: &Http, project: &Keys, email: &str) -> anyhow::Result<Session> {
    allow_password(http, project).await?;
    let (status, body) = http
        .call(
            18000,
            Some(&project.pref),
            "POST",
            "/auth/v1/signup",
            Some(&project.anon),
            Some(json!({"email": email, "password": "password123"})),
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    let v: Value = serde_json::from_str(&body)?;
    Ok(Session {
        access: v["access_token"].as_str().unwrap().to_string(),
        refresh: v["refresh_token"].as_str().unwrap().to_string(),
        user_id: v["user"]["id"].as_str().unwrap().to_string(),
    })
}

async fn cors_gate() -> anyhow::Result<()> {
    let http = live().await?;
    let project = create_project(&http, 18000).await?;
    let url = Http::url(18000, Some(&project.pref), "/auth/v1/user");
    let ok = http
        .client
        .get(&url)
        .header("origin", "http://localhost:5173")
        .send()
        .await?;
    assert_eq!(
        ok.headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("http://localhost:5173")
    );
    let blocked = http
        .client
        .get(&url)
        .header("origin", "https://evil.example")
        .send()
        .await?;
    assert!(blocked
        .headers()
        .get("access-control-allow-origin")
        .is_none());
    let preflight = http
        .client
        .request(reqwest::Method::OPTIONS, &url)
        .header("origin", "http://127.0.0.1:3000")
        .header("access-control-request-method", "POST")
        .send()
        .await?;
    assert_eq!(preflight.status(), 204);
    assert_eq!(
        preflight
            .headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()),
        Some("http://127.0.0.1:3000")
    );
    Ok(())
}

async fn rate_limit_gate() -> anyhow::Result<()> {
    let http = live().await?;
    let project = create_project(&http, 18000).await?;
    allow_password(&http, &project).await?;
    let mut limited = false;
    for n in 0..31 {
        let (status, body) = http
            .call(
                18000,
                Some(&project.pref),
                "POST",
                "/auth/v1/signup",
                Some(&project.anon),
                Some(json!({"email": format!("limit-{n}@example.com"), "password": "password123"})),
            )
            .await?;
        if status == 429 {
            assert!(body.contains("too many requests"), "{body}");
            limited = true;
            break;
        }
        assert_eq!(status, 200, "{body}");
    }
    assert!(limited, "signup rate limit did not trip");
    Ok(())
}

fn cli_migrate_all() -> anyhow::Result<std::process::Output> {
    let home = std::env::temp_dir().join(format!("reactor-home-{}", std::process::id()));
    std::fs::create_dir_all(&home)?;
    let bin = env("REACTOR_CLI");
    Command::new(&bin)
        .args([
            "login",
            "--url",
            "http://127.0.0.1:18000",
            "--token",
            &env("REACTOR_OPERATOR_TOKEN"),
        ])
        .env("REACTOR_HOME", &home)
        .status()
        .context("cli login")?;
    Ok(Command::new(&bin)
        .args(["db", "migrate", "--all"])
        .env("REACTOR_HOME", &home)
        .output()?)
}

async fn cli_deploy(http: &Http) -> anyhow::Result<()> {
    let project = create_project(http, 18000).await?;
    let dir = std::env::temp_dir().join(format!("reactor-demo-{}", project.pref));
    std::fs::create_dir_all(dir.join("functions/echo"))?;
    std::fs::create_dir_all(dir.join("site"))?;
    std::fs::write(dir.join("functions/echo/index.ts"), FN_SOURCE)?;
    std::fs::write(dir.join("site/index.html"), "cli-site")?;
    let bin = env("REACTOR_CLI");
    let link = Command::new(&bin)
        .args([
            "link",
            "--url",
            "http://127.0.0.1:18000",
            "--ref",
            &project.pref,
            "--service-key",
            &project.service,
        ])
        .current_dir(&dir)
        .status()?;
    assert!(link.success());
    assert!(dir.join(".reactor/service_key").is_file());
    let deploy = Command::new(&bin)
        .arg("deploy")
        .current_dir(&dir)
        .output()?;
    assert!(
        deploy.status.success(),
        "deploy {}\n{}",
        String::from_utf8_lossy(&deploy.stdout),
        String::from_utf8_lossy(&deploy.stderr)
    );
    let page = http
        .client
        .get(Http::url(18000, Some(&project.pref), "/"))
        .send()
        .await?;
    let text = page.text().await?;
    assert!(text.contains("cli-site"), "{text}");
    Ok(())
}

async fn in_process_health() -> anyhow::Result<()> {
    let sql = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sql");
    std::env::set_var("REACTOR_SQL_DIR", &sql);
    std::env::set_var("REACTOR_DATABASE__URL", env("REACTOR_ACCEPT_DATABASE_URL"));
    std::env::remove_var("REACTOR_DATABASE__DEDICATED_URL");
    std::env::remove_var("REACTOR_AUTH__JWT_PRIVATE_PEM_FILE");
    std::env::set_var("REACTOR_AUTH__PROVIDER", "internal");
    std::env::set_var(
        "REACTOR_AUTH__OPERATOR_TOKEN",
        env("REACTOR_OPERATOR_TOKEN"),
    );
    std::env::set_var("REACTOR_STORAGE__BACKEND", "fs");
    std::env::set_var(
        "REACTOR_AUTH__JWT_DIR",
        std::env::temp_dir().join("reactor-v2-accept-keys"),
    );
    std::env::set_var(
        "REACTOR_STORAGE__FS_ROOT",
        std::env::temp_dir().join("reactor-v2-accept-blobs"),
    );
    std::env::set_var(
        "REACTOR_FUNCTIONS__WORKDIR",
        std::env::temp_dir().join("reactor-v2-accept-fn"),
    );
    std::env::set_var("REACTOR_HANDLER", "");
    let app = reactor_server::router_from_env().await?;
    let req = reactor_server::from_apigw_v2(&json!({
        "version": "2.0",
        "rawPath": "/health",
        "requestContext": {"http": {"method": "GET", "path": "/health"}},
        "headers": {"host": "localhost"}
    }));
    let res = app.oneshot(req).await.unwrap_or_else(|_| {
        axum::http::Response::builder()
            .status(500)
            .body(Body::empty())
            .unwrap()
    });
    assert_eq!(res.status(), 200);
    let _ = res.into_body().collect().await;
    Ok(())
}

#[tokio::test]
async fn scripted_agent() {
    if std::env::var("REACTOR_ACCEPT").ok().as_deref() != Some("1") {
        return;
    }
    if let Err(err) = scripted_agent_turn().await {
        panic!("{err:#}");
    }
}

#[tokio::test]
async fn live_agent() {
    if std::env::var("REACTOR_AGENT").ok().as_deref() != Some("1") {
        return;
    }
    if let Err(err) = live_agent_turn().await {
        panic!("{err:#}");
    }
}

async fn scripted_model(axum::Json(body): axum::Json<Value>) -> axum::response::Response {
    let messages = body["messages"].as_array().cloned().unwrap_or_default();
    let tools: Vec<Value> = messages
        .into_iter()
        .filter(|message| message["role"] == "tool")
        .collect();
    let parsed = |index: usize| -> Value {
        tools
            .get(index)
            .and_then(|message| message["content"].as_str())
            .and_then(|text| serde_json::from_str(text).ok())
            .unwrap_or(json!({}))
    };
    let pref = parsed(0)["ref"].as_str().unwrap_or("").to_string();
    let row_id = parsed(2)["id"].as_str().unwrap_or("").to_string();
    let sse = match tools.len() {
        0 => tool_sse(
            "call_project",
            "create_project",
            json!({"name": "scripted-agent-project"}),
        ),
        1 => tool_sse(
            "call_table",
            "create_table",
            json!({"ref": pref, "name": "notes", "columns": [{"name": "body", "type": "text"}]}),
        ),
        2 => tool_sse(
            "call_insert",
            "insert_row",
            json!({"ref": pref, "table": "notes", "values": {"body": "original"}}),
        ),
        3 => tool_sse(
            "call_update",
            "update_row",
            json!({"ref": pref, "table": "notes", "pk": row_id, "column": "body", "value": "scripted-cell"}),
        ),
        4 => tool_sse(
            "call_user",
            "create_user",
            json!({"ref": pref, "email": "scripted-user@example.com", "password": "s3cret-pass"}),
        ),
        5 => tool_sse(
            "call_file",
            "put_object",
            json!({"ref": pref, "path": "notes/hello.txt", "content": "scripted-file-ok"}),
        ),
        6 => tool_sse(
            "call_site",
            "put_site_file",
            json!({"ref": pref, "path": "index.html", "content": "<p>scripted-site-ok</p>"}),
        ),
        _ => text_sse("Done."),
    };
    axum::response::Response::builder()
        .header("content-type", "text/event-stream")
        .body(Body::from(sse))
        .unwrap()
}

fn tool_sse(id: &str, name: &str, args: Value) -> String {
    let payload = json!({
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": id,
                    "function": {"name": name, "arguments": args.to_string()}
                }]
            }
        }]
    });
    format!("data: {payload}\n\ndata: [DONE]\n")
}

fn text_sse(text: &str) -> String {
    format!(
        "data: {}\n\ndata: [DONE]\n",
        json!({"choices":[{"delta":{"content": text}}]})
    )
}

async fn scripted_agent_turn() -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let app =
        axum::Router::new().route("/v1/chat/completions", axum::routing::post(scripted_model));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let root = std::env::temp_dir().join("reactor-agent-scripted");
    let _ = std::fs::remove_dir_all(&root);
    let (router, _env) =
        boot_agent(&format!("http://127.0.0.1:{port}/v1"), "script-key", &root).await?;
    let pool = PgPool::connect(&std::env::var("REACTOR_DATABASE__URL")?).await?;
    let token = console_session(&pool, &router, "scripted-agent@example.com").await?;
    let thread = post_turn(&router, &token, "Set up the scripted project.").await?;
    let body = wait_until_idle(&router, &token, &thread, 40).await?;
    let rendered = body.to_string();
    assert!(!rendered.contains("s3cret-pass"), "{rendered}");
    assert!(!rendered.contains("service_key"), "{rendered}");
    assert!(!rendered.contains("anon_key"), "{rendered}");
    let pref = project_ref(&pool, "scripted-agent-project")
        .await
        .context(body.to_string())?;
    anyhow::ensure!(pref.len() == 20 && pref.chars().all(|c| c.is_ascii_alphanumeric()));
    let cell: String = sqlx::query_scalar(&format!("SELECT body FROM \"proj_{pref}\".notes"))
        .fetch_one(&pool)
        .await?;
    assert_eq!(cell, "scripted-cell");
    let email: String = sqlx::query_scalar(
        "SELECT u.email FROM reactor.users u JOIN reactor.projects p ON p.id = u.project_id WHERE p.ref = $1",
    )
    .bind(&pref)
    .fetch_one(&pool)
    .await?;
    assert_eq!(email, "scripted-user@example.com");
    let logs: Vec<String> = sqlx::query_scalar(
        "SELECT l.message FROM reactor.logs l JOIN reactor.projects p ON p.id = l.project_id WHERE p.ref = $1 AND l.kind = 'agent'",
    )
    .bind(&pref)
    .fetch_all(&pool)
    .await?;
    assert!(
        logs.iter().all(|line| !line.contains("s3cret-pass")),
        "{logs:?}"
    );
    let file = std::fs::read_to_string(
        root.join("blobs")
            .join(&pref)
            .join("notes")
            .join("hello.txt"),
    )?;
    assert_eq!(file, "scripted-file-ok");
    let page = site_body(&router, &pref).await?;
    assert!(page.contains("scripted-site-ok"), "{page}");
    Ok(())
}

async fn live_agent_turn() -> anyhow::Result<()> {
    let key =
        openrouter_key().context("REACTOR_AGENT__API_KEY or console/.env.local is required")?;
    let stamp_full = Uuid::new_v4().simple().to_string();
    let stamp = &stamp_full[..8];
    let name = format!("live-agent-{stamp}");
    let email = format!("live-{stamp}@example.com");
    let cell = format!("live-cell-{stamp}");
    let file_body = format!("live-file-{stamp}");
    let html = format!("live-site-{stamp}");
    let root = std::env::temp_dir().join(format!("reactor-agent-live-{stamp}"));
    let (router, _env) = boot_agent("https://openrouter.ai/api/v1", &key, &root).await?;
    let pool = PgPool::connect(&std::env::var("REACTOR_DATABASE__URL")?).await?;
    let token = console_session(&pool, &router, "live-agent@example.com").await?;
    let prompt = format!(
        "Create a project named {name}. Create a table named notes with one text column named body. Insert a row whose body is original. Change that row's body to {cell}. Add a project user {email} with password live-pass-1. Store a file at notes/hello.txt whose exact contents are {file_body}. Publish index.html whose exact body is {html}."
    );
    let thread = post_turn(&router, &token, &prompt).await?;
    let body = wait_until_idle(&router, &token, &thread, 170).await?;
    let pref = project_ref(&pool, &name).await.context(body.to_string())?;
    let cell_value: String = sqlx::query_scalar(&format!("SELECT body FROM \"proj_{pref}\".notes"))
        .fetch_one(&pool)
        .await
        .context(body.to_string())?;
    assert_eq!(cell_value, cell, "{body}");
    let found: String = sqlx::query_scalar(
        "SELECT u.email FROM reactor.users u JOIN reactor.projects p ON p.id = u.project_id WHERE p.ref = $1",
    )
    .bind(&pref)
    .fetch_one(&pool)
    .await
    .context(body.to_string())?;
    assert_eq!(found, email, "{body}");
    let stored = std::fs::read_to_string(
        root.join("blobs")
            .join(&pref)
            .join("notes")
            .join("hello.txt"),
    )
    .context(body.to_string())?;
    assert_eq!(stored, file_body, "{body}");
    let page = site_body(&router, &pref).await?;
    assert!(page.contains(&html), "{page}\n{body}");
    Ok(())
}

fn openrouter_key() -> Option<String> {
    if let Ok(value) = std::env::var("REACTOR_AGENT__API_KEY") {
        if !value.is_empty() {
            return Some(value);
        }
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../console/.env.local");
    let text = std::fs::read_to_string(path).ok()?;
    for line in text.lines() {
        let line = line.trim();
        let Some(value) = line
            .strip_prefix("REACTOR_AGENT__API_KEY=")
            .or_else(|| line.strip_prefix("OPENROUTER_API_KEY="))
        else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim().to_string();
        if !value.is_empty() {
            return Some(value);
        }
    }
    None
}

struct EnvGuard(Vec<(String, Option<String>)>);

impl EnvGuard {
    fn record(&mut self, key: &str) {
        if self.0.iter().any(|(name, _)| name == key) {
            return;
        }
        self.0.push((key.to_string(), std::env::var(key).ok()));
    }

    fn set(&mut self, key: &str, value: &str) {
        self.record(key);
        std::env::set_var(key, value);
    }

    fn unset(&mut self, key: &str) {
        self.record(key);
        std::env::remove_var(key);
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, prev) in self.0.drain(..).rev() {
            match prev {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

async fn boot_agent(
    base_url: &str,
    api_key: &str,
    root: &Path,
) -> anyhow::Result<(axum::Router, EnvGuard)> {
    std::fs::create_dir_all(root)?;
    let sql = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sql");
    let database = std::env::var("REACTOR_ACCEPT_DATABASE_URL")
        .or_else(|_| std::env::var("REACTOR_DATABASE__URL"))
        .unwrap_or_else(|_| "postgres://reactor:reactor@127.0.0.1:5440/reactor".into());
    let operator =
        std::env::var("REACTOR_OPERATOR_TOKEN").unwrap_or_else(|_| "dev-operator-token".into());
    let mut env = EnvGuard(Vec::new());
    env.set("REACTOR_SQL_DIR", &sql.to_string_lossy());
    env.set("REACTOR_DATABASE__URL", &database);
    env.unset("REACTOR_DATABASE__DEDICATED_URL");
    env.unset("REACTOR_AUTH__JWT_PRIVATE_PEM_FILE");
    env.set("REACTOR_AUTH__PROVIDER", "internal");
    env.set("REACTOR_AUTH__OPERATOR_TOKEN", &operator);
    env.set("REACTOR_STORAGE__BACKEND", "fs");
    env.set(
        "REACTOR_AUTH__JWT_DIR",
        &root.join("keys").to_string_lossy(),
    );
    env.set(
        "REACTOR_STORAGE__FS_ROOT",
        &root.join("blobs").to_string_lossy(),
    );
    env.set(
        "REACTOR_FUNCTIONS__WORKDIR",
        &root.join("fn").to_string_lossy(),
    );
    env.set("REACTOR_HANDLER", "");
    env.set("REACTOR_RUNTIME__MODE", "listen");
    env.set("REACTOR_RUNTIME__BASE_DOMAIN", "apps.localhost");
    env.set("REACTOR_AGENT__BASE_URL", base_url);
    env.set("REACTOR_AGENT__API_KEY", api_key);
    env.set("REACTOR_AGENT__TURN_DEADLINE_SECONDS", "180");
    let app = reactor_server::router_from_env().await?;
    Ok((app, env))
}

async fn console_session(pool: &PgPool, app: &axum::Router, email: &str) -> anyhow::Result<String> {
    let hash = reactor_auth::hash_password("scripted-pass")?;
    sqlx::query(
        "INSERT INTO reactor.operators (id, email, name, password_hash, platform_admin) VALUES ($1, $2, 'Agent', $3, true) \
         ON CONFLICT (email) DO UPDATE SET password_hash = EXCLUDED.password_hash, platform_admin = true",
    )
    .bind(Uuid::new_v4())
    .bind(email)
    .bind(&hash)
    .execute(pool)
    .await?;
    let id: Uuid = sqlx::query_scalar("SELECT id FROM reactor.operators WHERE email = $1")
        .bind(email)
        .fetch_one(pool)
        .await?;
    sqlx::query("DELETE FROM reactor.operator_factors WHERE operator_id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO reactor.operator_factors (operator_id, kind, secret) VALUES ($1, 'totp', 'test')")
        .bind(id)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO reactor.operator_factors (operator_id, kind, credential) VALUES ($1, 'passkey', $2)")
        .bind(id)
        .bind(json!({}))
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM reactor.operator_recovery_codes WHERE operator_id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    sqlx::query(
        "INSERT INTO reactor.operator_recovery_codes (operator_id, code_hash) VALUES ($1, $2)",
    )
    .bind(id)
    .bind(reactor_auth::token_hash("accept-recovery"))
    .execute(pool)
    .await?;
    let (_, body) = app_json(
        app.clone(),
        "POST",
        "/console/v1/login",
        None,
        Some(json!({"email": email, "password": "scripted-pass"})),
    )
    .await?;
    let mfa = body["mfa_token"]
        .as_str()
        .context("console login did not ask for a second factor")?;
    let (_, session) = app_json(
        app.clone(),
        "POST",
        "/console/v1/mfa/recovery",
        Some(mfa),
        Some(json!({"code": "accept-recovery"})),
    )
    .await?;
    session["access_token"]
        .as_str()
        .map(|token| token.to_string())
        .context("console login did not return a token")
}

async fn post_turn(app: &axum::Router, token: &str, text: &str) -> anyhow::Result<String> {
    let (_, created) = app_json(
        app.clone(),
        "POST",
        "/console/v1/agent/threads",
        Some(token),
        Some(json!({})),
    )
    .await?;
    let id = created["id"].as_str().context("thread id")?.to_string();
    let (status, _) = app_json(
        app.clone(),
        "POST",
        &format!("/console/v1/agent/threads/{id}/messages"),
        Some(token),
        Some(json!({"text": text})),
    )
    .await?;
    anyhow::ensure!(status == 202, "message was not accepted: {status}");
    Ok(id)
}

async fn wait_until_idle(
    app: &axum::Router,
    token: &str,
    id: &str,
    seconds: u64,
) -> anyhow::Result<Value> {
    let deadline = std::time::Instant::now() + Duration::from_secs(seconds);
    let mut body = json!({});
    while std::time::Instant::now() < deadline {
        let (_, next) = app_json(
            app.clone(),
            "GET",
            &format!("/console/v1/agent/threads/{id}"),
            Some(token),
            None,
        )
        .await?;
        body = next;
        if body["running"] == false {
            let last = body["messages"]
                .as_array()
                .and_then(|messages| messages.last());
            if last
                .map(|message| message["role"] == "assistant")
                .unwrap_or(false)
            {
                return Ok(body);
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    anyhow::bail!("turn did not finish: {body}")
}

async fn project_ref(pool: &PgPool, name: &str) -> anyhow::Result<String> {
    let pref: Option<String> = sqlx::query_scalar(
        "SELECT ref FROM reactor.projects WHERE name = $1 ORDER BY created_at DESC LIMIT 1",
    )
    .bind(name)
    .fetch_optional(pool)
    .await?;
    let pref = pref.context(format!("project {name} was not created"))?;
    anyhow::ensure!(pref.len() == 20 && pref.chars().all(|c| c.is_ascii_alphanumeric()));
    Ok(pref)
}

async fn site_body(app: &axum::Router, pref: &str) -> anyhow::Result<String> {
    let req = Request::builder()
        .uri("/")
        .header("host", format!("{pref}.apps.localhost"))
        .body(Body::empty())?;
    let res = app
        .clone()
        .oneshot(req)
        .await
        .map_err(|_| anyhow::anyhow!("site request failed"))?;
    let status = res.status();
    let bytes = res.into_body().collect().await?.to_bytes();
    anyhow::ensure!(status == StatusCode::OK, "site status {status}");
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

async fn app_json(
    app: axum::Router,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<Value>,
) -> anyhow::Result<(u16, Value)> {
    let mut builder = Request::builder().method(method).uri(path);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let req = if let Some(body) = body {
        builder
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))?
    } else {
        builder.body(Body::empty())?
    };
    let res = app
        .oneshot(req)
        .await
        .map_err(|_| anyhow::anyhow!("request failed"))?;
    let status = res.status().as_u16();
    let bytes = res.into_body().collect().await?.to_bytes();
    let value = if bytes.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&bytes).unwrap_or(json!({"raw": String::from_utf8_lossy(&bytes)}))
    };
    Ok((status, value))
}

fn code_from(text: &str) -> Option<String> {
    let rest = text.split("Code: ").nth(1)?;
    let code: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if code.len() == 6 {
        Some(code)
    } else {
        None
    }
}

async fn configure_mail(http: &Http, project: &Keys, token: &str) -> anyhow::Result<()> {
    let (status, body) = http
        .call(
            18000,
            None,
            "PUT",
            &format!("/console/v1/projects/{}/email", project.pref),
            Some(token),
            Some(json!({
                "host": "mailpit",
                "port": 1025,
                "username": "",
                "password": "",
                "from_address": "reactor@example.com",
                "tls": "none",
                "link_base": "http://127.0.0.1/cb"
            })),
        )
        .await?;
    assert_eq!(status, 200, "{body}");
    Ok(())
}

#[tokio::test]
async fn gate_confirm_email() {
    run_gate(async {
        let http = live().await?;
        let project = create_project(&http, 18000).await?;
        let pool = PgPool::connect(&env("REACTOR_ACCEPT_DATABASE_URL")).await?;
        let token = console_token(&pool, &http, &project).await?;
        let (status, body) = http
            .call(18000, None, "GET", &format!("/console/v1/projects/{}/auth", project.pref), Some(&token), None)
            .await?;
        assert_eq!(status, 200, "{body}");
        let settings: Value = serde_json::from_str(&body)?;
        assert_eq!(settings["require_email_verification"], true);
        let developer = format!("dev-{}@example.com", &project.pref[..8]);
        let dev_id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO reactor.operators (id, email, name, password_hash, platform_admin) VALUES ($1, $2, 'Dev', $3, false)",
        )
        .bind(dev_id)
        .bind(&developer)
        .bind(reactor_auth::hash_password("scripted-pass")?)
        .execute(&pool)
        .await?;
        let project_id: Uuid = sqlx::query_scalar("SELECT id FROM reactor.projects WHERE ref = $1")
            .bind(&project.pref)
            .fetch_one(&pool)
            .await?;
        sqlx::query(
            "INSERT INTO reactor.memberships (operator_id, project_id, role) VALUES ($1, $2, 'developer')",
        )
        .bind(dev_id)
        .bind(project_id)
        .execute(&pool)
        .await?;
        sqlx::query("INSERT INTO reactor.operator_factors (operator_id, kind, secret) VALUES ($1, 'totp', 'test')")
            .bind(dev_id)
            .execute(&pool)
            .await?;
        sqlx::query("INSERT INTO reactor.operator_factors (operator_id, kind, credential) VALUES ($1, 'passkey', $2)")
            .bind(dev_id)
            .bind(json!({}))
            .execute(&pool)
            .await?;
        let recovery = format!("accept-{dev_id}");
        sqlx::query("INSERT INTO reactor.operator_recovery_codes (operator_id, code_hash) VALUES ($1, $2)")
            .bind(dev_id)
            .bind(reactor_auth::token_hash(&recovery))
            .execute(&pool)
            .await?;
        let (status, body) = http
            .call(18000, None, "POST", "/console/v1/login", None, Some(json!({"email": developer, "password": "scripted-pass"})))
            .await?;
        assert_eq!(status, 200, "{body}");
        let login: Value = serde_json::from_str(&body)?;
        let dev_token = if let Some(access) = login["access_token"].as_str() {
            access.to_string()
        } else {
            let (status, session) = http
                .call(
                    18000,
                    None,
                    "POST",
                    "/console/v1/mfa/recovery",
                    Some(login["mfa_token"].as_str().context("developer mfa")?),
                    Some(json!({"code": recovery})),
                )
                .await?;
            assert_eq!(status, 200, "{session}");
            let session: Value = serde_json::from_str(&session)?;
            session["access_token"].as_str().context("developer token")?.to_string()
        };
        let (status, body) = http
            .call(
                18000,
                None,
                "PUT",
                &format!("/console/v1/projects/{}/auth", project.pref),
                Some(&dev_token),
                Some(json!({"require_email_verification": false, "require_mfa": false})),
            )
            .await?;
        assert_eq!(status, 403, "developer changed auth settings: {body}");
        let bare = format!("bare-{}@example.com", &project.pref[..8]);
        let (status, body) = http
            .call(18000, Some(&project.pref), "POST", "/auth/v1/signup", None, Some(json!({"email": bare, "password": "password123"})))
            .await?;
        assert_eq!(status, 503, "signup without smtp: {body}");
        configure_mail(&http, &project, &token).await?;
        let (status, body) = http
            .call(18000, Some(&project.pref), "POST", "/auth/v1/signup", None, Some(json!({"email": bare, "password": "password123"})))
            .await?;
        assert_eq!(status, 200, "{body}");
        let pending: Value = serde_json::from_str(&body)?;
        assert_eq!(pending["verification_required"], true);
        assert!(pending.get("access_token").is_none());
        let mailed = mail_bodies(&bare).await?;
        let link = token_from(mailed.last().context("no confirm mail")?).context("token")?;
        let code = code_from(mailed.last().unwrap()).context("no code")?;
        let (status, _) = http
            .call(18000, Some(&project.pref), "POST", "/auth/v1/verify-email/send", None, Some(json!({"email": bare})))
            .await?;
        assert_eq!(status, 200);
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert_eq!(mail_bodies(&bare).await?.len(), 1, "resend inside 60 seconds");
        let (status, body) = http
            .call(18000, Some(&project.pref), "POST", "/auth/v1/token", None, Some(json!({"email": bare, "password": "password123"})))
            .await?;
        assert_eq!(status, 200, "{body}");
        let still: Value = serde_json::from_str(&body)?;
        assert_eq!(still["verification_required"], true);
        assert_eq!(mail_bodies(&bare).await?.len(), 1, "login sent another mail");
        let (status, body) = http
            .call(18000, Some(&project.pref), "POST", "/auth/v1/verify-email", None, Some(json!({"token": link})))
            .await?;
        assert_eq!(status, 200, "{body}");
        let (status, _) = http
            .call(18000, Some(&project.pref), "POST", "/auth/v1/verify-email", None, Some(json!({"email": bare, "code": code})))
            .await?;
        assert_eq!(status, 401);
        let coded = format!("code-{}@example.com", &project.pref[..8]);
        let (status, _) = http
            .call(18000, Some(&project.pref), "POST", "/auth/v1/signup", None, Some(json!({"email": coded, "password": "password123"})))
            .await?;
        assert_eq!(status, 200);
        let pasted = code_from(mail_bodies(&coded).await?.last().context("no code mail")?).context("code")?;
        for _ in 0..5 {
            let (status, _) = http
                .call(18000, Some(&project.pref), "POST", "/auth/v1/verify-email", None, Some(json!({"email": coded, "code": "000000"})))
                .await?;
            assert_eq!(status, 401);
        }
        let (status, _) = http
            .call(18000, Some(&project.pref), "POST", "/auth/v1/verify-email", None, Some(json!({"email": coded, "code": pasted})))
            .await?;
        assert_eq!(status, 401, "burned code still worked");
        Ok(())
    })
    .await;
}

fn current_totp(secret: &str) -> anyhow::Result<String> {
    let bytes = totp_rs::Secret::Encoded(secret.to_string()).to_bytes()?;
    let totp = totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        30,
        bytes,
        Some("Reactor".into()),
        "user".into(),
    )?;
    Ok(totp.generate_current()?)
}

#[tokio::test]
async fn gate_project_factors() {
    run_gate(async {
        let http = live().await?;
        let project = create_project(&http, 18000).await?;
        let pool = PgPool::connect(&env("REACTOR_ACCEPT_DATABASE_URL")).await?;
        let token = console_token(&pool, &http, &project).await?;
        let (status, body) = http
            .call(
                18000,
                None,
                "PUT",
                &format!("/console/v1/projects/{}/auth", project.pref),
                Some(&token),
                Some(json!({"require_email_verification": false, "require_mfa": true})),
            )
            .await?;
        assert_eq!(status, 200, "{body}");
        let email = format!("mfa-{}@example.com", &project.pref[..8]);
        let (status, body) = http
            .call(
                18000,
                Some(&project.pref),
                "POST",
                "/auth/v1/signup",
                None,
                Some(json!({"email": email, "password": "password123"})),
            )
            .await?;
        assert_eq!(status, 200, "{body}");
        let (status, body) = http
            .call(
                18000,
                Some(&project.pref),
                "POST",
                "/auth/v1/token",
                None,
                Some(json!({"email": email, "password": "password123"})),
            )
            .await?;
        assert_eq!(status, 200, "{body}");
        let challenge: Value = serde_json::from_str(&body)?;
        assert_eq!(challenge["enrollment_required"], true);
        let enroll = challenge["enroll_token"].as_str().context("enroll token")?;
        let (status, body) = http
            .call(
                18000,
                Some(&project.pref),
                "POST",
                "/auth/v1/factors/totp/start",
                Some(enroll),
                None,
            )
            .await?;
        assert_eq!(status, 200, "{body}");
        let start: Value = serde_json::from_str(&body)?;
        let secret = start["secret"].as_str().context("secret")?;
        let code = current_totp(secret)?;
        let (status, body) = http
            .call(
                18000,
                Some(&project.pref),
                "POST",
                "/auth/v1/factors/totp/confirm",
                Some(enroll),
                Some(json!({"code": code})),
            )
            .await?;
        assert_eq!(status, 200, "{body}");
        let (status, body) = http
            .call(
                18000,
                Some(&project.pref),
                "POST",
                "/auth/v1/factors/finish",
                Some(enroll),
                None,
            )
            .await?;
        assert_eq!(status, 200, "{body}");
        let done: Value = serde_json::from_str(&body)?;
        let codes = done["recovery_codes"].as_array().context("codes")?;
        assert_eq!(codes.len(), 8);
        let backup = codes[0].as_str().unwrap().to_string();
        let (status, body) = http
            .call(
                18000,
                Some(&project.pref),
                "POST",
                "/auth/v1/token",
                None,
                Some(json!({"email": email, "password": "password123"})),
            )
            .await?;
        assert_eq!(status, 200, "{body}");
        let mfa: Value = serde_json::from_str(&body)?;
        assert_eq!(mfa["mfa_required"], true);
        let mfa_token = mfa["mfa_token"].as_str().unwrap();
        let (status, body) = http
            .call(
                18000,
                Some(&project.pref),
                "POST",
                "/auth/v1/factors/totp",
                None,
                Some(json!({"mfa_token": mfa_token, "code": current_totp(secret)?})),
            )
            .await?;
        assert_eq!(status, 200, "{body}");
        let (_, body) = http
            .call(
                18000,
                Some(&project.pref),
                "POST",
                "/auth/v1/token",
                None,
                Some(json!({"email": email, "password": "password123"})),
            )
            .await?;
        let mfa: Value = serde_json::from_str(&body)?;
        let mfa_token = mfa["mfa_token"].as_str().unwrap();
        let (status, body) = http
            .call(
                18000,
                Some(&project.pref),
                "POST",
                "/auth/v1/factors/recovery",
                None,
                Some(json!({"mfa_token": mfa_token, "code": backup})),
            )
            .await?;
        assert_eq!(status, 200, "{body}");
        let (_, body) = http
            .call(
                18000,
                Some(&project.pref),
                "POST",
                "/auth/v1/token",
                None,
                Some(json!({"email": email, "password": "password123"})),
            )
            .await?;
        let mfa: Value = serde_json::from_str(&body)?;
        let burned = mfa["mfa_token"].as_str().unwrap();
        for _ in 0..5 {
            let (status, _) = http
                .call(
                    18000,
                    Some(&project.pref),
                    "POST",
                    "/auth/v1/factors/totp",
                    None,
                    Some(json!({"mfa_token": burned, "code": "000000"})),
                )
                .await?;
            assert_eq!(status, 401);
        }
        let (status, _) = http
            .call(
                18000,
                Some(&project.pref),
                "POST",
                "/auth/v1/factors/totp",
                None,
                Some(json!({"mfa_token": burned, "code": current_totp(secret)?})),
            )
            .await?;
        assert_eq!(status, 401, "burned factor challenge still worked");
        let (status, body) = http
            .call(
                18000,
                None,
                "PUT",
                &format!("/console/v1/projects/{}/auth", project.pref),
                Some(&token),
                Some(json!({"require_email_verification": false, "require_mfa": false})),
            )
            .await?;
        assert_eq!(status, 200, "{body}");
        let (status, body) = http
            .call(
                18000,
                Some(&project.pref),
                "POST",
                "/auth/v1/token",
                None,
                Some(json!({"email": email, "password": "password123"})),
            )
            .await?;
        assert_eq!(status, 200, "{body}");
        let session: Value = serde_json::from_str(&body)?;
        assert!(session["access_token"].as_str().is_some(), "{body}");
        let _ = status;
        Ok(())
    })
    .await;
}

#[tokio::test]
async fn gate_project_passkey() {
    run_gate(async {
        let http = live().await?;
        let project = create_project(&http, 18000).await?;
        allow_password(&http, &project).await?;
        let user = signup(&http, &project).await?;
        let origin = format!("http://{}.apps.localhost:18000", project.pref);
        let client = reqwest::Client::builder()
            .dns_resolver(Arc::new(LocalDns))
            .timeout(Duration::from_secs(40))
            .build()?;
        let res = client
            .post(Http::url(18000, Some(&project.pref), "/auth/v1/factors/passkey/register/options"))
            .bearer_auth(&user.access)
            .header("origin", &origin)
            .send()
            .await?;
        let status = res.status();
        let options: Value = res.json().await?;
        assert_eq!(status, 200, "{options}");
        assert!(options.get("publicKey").is_some() || options.get("rp").is_some(), "{options}");
        let res = client
            .post(Http::url(18000, Some(&project.pref), "/auth/v1/factors/passkey/register"))
            .bearer_auth(&user.access)
            .header("origin", &origin)
            .json(&json!({"id": "aa", "rawId": "aa", "type": "public-key", "response": {"clientDataJSON": "aaaa", "attestationObject": "aaaa"}}))
            .send()
            .await?;
        let status = res.status().as_u16();
        let body = res.text().await.unwrap_or_default();
        assert!(status == 400 || status == 401, "tampered passkey accepted: {status} {body}");
        Ok(())
    })
    .await;
}

async fn mock_authorize(
    axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::response::Redirect {
    let redirect = query.get("redirect_uri").cloned().unwrap_or_default();
    let state = query.get("state").cloned().unwrap_or_default();
    axum::response::Redirect::temporary(&format!("{redirect}?code=from-mock&state={state}"))
}

async fn mock_token() -> axum::Json<Value> {
    axum::Json(json!({"access_token": "mock-access"}))
}

async fn mock_userinfo(
    axum::extract::Query(query): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> axum::Json<Value> {
    let email = query
        .get("email")
        .cloned()
        .unwrap_or_else(|| "oauth@example.com".into());
    let verified = query
        .get("verified")
        .map(|value| value != "false")
        .unwrap_or(true);
    axum::Json(json!({"sub": format!("sub-{email}"), "email": email, "email_verified": verified}))
}

async fn spawn_mock() -> anyhow::Result<String> {
    let listener = tokio::net::TcpListener::bind("0.0.0.0:0").await?;
    let port = listener.local_addr()?.port();
    let app = axum::Router::new()
        .route("/authorize", axum::routing::get(mock_authorize))
        .route("/token", axum::routing::post(mock_token))
        .route("/userinfo", axum::routing::get(mock_userinfo));
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(format!("http://host.docker.internal:{port}"))
}

async fn oauth_code(
    client: &reqwest::Client,
    pref: &str,
    redirect_to: &str,
) -> anyhow::Result<String> {
    let start = client
        .get(Http::url(
            18000,
            Some(pref),
            &format!("/auth/v1/authorize?provider=google&redirect_to={redirect_to}"),
        ))
        .send()
        .await?;
    assert!(start.status().is_redirection(), "{}", start.status());
    let next = start
        .headers()
        .get("location")
        .context("authorize location")?
        .to_str()?
        .to_string();
    let provider = client.get(next).send().await?;
    assert!(provider.status().is_redirection(), "{}", provider.status());
    let callback = provider
        .headers()
        .get("location")
        .context("provider location")?
        .to_str()?
        .to_string();
    let done = client.get(callback).send().await?;
    assert!(done.status().is_redirection(), "{}", done.status());
    let landed = done
        .headers()
        .get("location")
        .context("callback location")?
        .to_str()?;
    let code = landed
        .split("code=")
        .nth(1)
        .context("code")?
        .split('&')
        .next()
        .unwrap();
    Ok(code.to_string())
}

#[tokio::test]
async fn gate_oauth() {
    run_gate(async {
        let http = live().await?;
        let project = create_project(&http, 18000).await?;
        allow_password(&http, &project).await?;
        let pool = PgPool::connect(&env("REACTOR_ACCEPT_DATABASE_URL")).await?;
        let console = console_token(&pool, &http, &project).await?;
        let base = spawn_mock().await?;
        let redirect_to = "http://app.example/done";
        let (status, body) = http
            .call(
                18000,
                None,
                "POST",
                &format!("/console/v1/projects/{}/auth/providers", project.pref),
                Some(&console),
                Some(json!({
                    "provider": "google",
                    "client_id": "client",
                    "client_secret": "secret",
                    "redirects": [redirect_to],
                    "extra": {
                        "authorize_url": format!("{base}/authorize"),
                        "token_url": format!("{base}/token"),
                        "userinfo_url": format!("{base}/userinfo?email=new-oauth@example.com&verified=true")
                    }
                })),
            )
            .await?;
        assert_eq!(status, 200, "{body}");
        let (status, body) = http
            .call(18000, Some(&project.pref), "GET", "/auth/v1/authorize?provider=google&redirect_to=http://evil.example/steal", None, None)
            .await?;
        assert_eq!(status, 400, "{body}");
        let client = reqwest::Client::builder()
            .dns_resolver(Arc::new(LocalDns))
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(40))
            .build()?;
        let code = oauth_code(&client, &project.pref, redirect_to).await?;
        let (status, body) = http
            .call(18000, Some(&project.pref), "POST", "/auth/v1/token", None, Some(json!({"code": code})))
            .await?;
        assert_eq!(status, 200, "{body}");
        let session: Value = serde_json::from_str(&body)?;
        assert_eq!(session["user"]["email"], "new-oauth@example.com");
        let (status, _) = http
            .call(18000, Some(&project.pref), "POST", "/auth/v1/token", None, Some(json!({"code": code})))
            .await?;
        assert_eq!(status, 401);
        let linked = format!("link-{}@example.com", &project.pref[..8]);
        signup_as(&http, &project, &linked).await?;
        let (status, body) = http
            .call(
                18000,
                None,
                "POST",
                &format!("/console/v1/projects/{}/auth/providers", project.pref),
                Some(&console),
                Some(json!({
                    "provider": "google",
                    "client_id": "client",
                    "client_secret": "secret",
                    "redirects": [redirect_to],
                    "extra": {
                        "authorize_url": format!("{base}/authorize"),
                        "token_url": format!("{base}/token"),
                        "userinfo_url": format!("{base}/userinfo?email={linked}&verified=true")
                    }
                })),
            )
            .await?;
        assert_eq!(status, 200, "{body}");
        let code = oauth_code(&client, &project.pref, redirect_to).await?;
        let (status, body) = http
            .call(18000, Some(&project.pref), "POST", "/auth/v1/token", None, Some(json!({"code": code})))
            .await?;
        assert_eq!(status, 200, "{body}");
        let linked_session: Value = serde_json::from_str(&body)?;
        assert_eq!(linked_session["user"]["email"], linked);
        let (status, body) = http
            .call(
                18000,
                None,
                "POST",
                &format!("/console/v1/projects/{}/auth/providers", project.pref),
                Some(&console),
                Some(json!({
                    "provider": "google",
                    "client_id": "client",
                    "redirects": [redirect_to],
                    "extra": {
                        "authorize_url": format!("{base}/authorize"),
                        "token_url": format!("{base}/token"),
                        "userinfo_url": format!("{base}/userinfo?email=nope@example.com&verified=false")
                    }
                })),
            )
            .await?;
        assert_eq!(status, 200, "{body}");
        let start = client
            .get(Http::url(18000, Some(&project.pref), &format!("/auth/v1/authorize?provider=google&redirect_to={redirect_to}")))
            .send()
            .await?;
        let provider = client.get(start.headers().get("location").unwrap().to_str()?).send().await?;
        let done = client.get(provider.headers().get("location").unwrap().to_str()?).send().await?;
        let landed = done.headers().get("location").context("error redirect")?.to_str()?;
        assert!(landed.contains("error=email_not_verified"), "{landed}");
        let (status, body) = http
            .call(
                18000,
                None,
                "POST",
                &format!("/console/v1/projects/{}/auth/providers", project.pref),
                Some(&console),
                Some(json!({
                    "provider": "google",
                    "client_id": "client",
                    "redirects": [redirect_to],
                    "enabled": false,
                    "extra": {"authorize_url": format!("{base}/authorize"), "token_url": format!("{base}/token"), "userinfo_url": format!("{base}/userinfo")}
                })),
            )
            .await?;
        assert_eq!(status, 200, "{body}");
        let (status, body) = http
            .call(18000, Some(&project.pref), "GET", &format!("/auth/v1/authorize?provider=google&redirect_to={redirect_to}"), None, None)
            .await?;
        assert_eq!(status, 404, "{body}");
        Ok(())
    })
    .await;
}
