use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpStream;
use tokio::process::Command;
use tokio::sync::Mutex;
use uuid::Uuid;

struct Running {
    child: tokio::process::Child,
    port: u16,
    last: Instant,
}

pub struct SiteSupervisor {
    inner: Mutex<HashMap<Uuid, Running>>,
    locks: Mutex<HashMap<Uuid, std::sync::Arc<Mutex<()>>>>,
    idle: Duration,
    workdir: PathBuf,
}

impl SiteSupervisor {
    pub fn new(workdir: PathBuf, idle: Duration) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            locks: Mutex::new(HashMap::new()),
            idle,
            workdir,
        }
    }

    pub async fn stop(&self, project_id: Uuid) {
        if let Some(mut running) = self.inner.lock().await.remove(&project_id) {
            let _ = running.child.kill().await;
            let _ = running.child.wait().await;
        }
    }

    pub async fn sweep(&self) {
        let mut inner = self.inner.lock().await;
        let stale: Vec<Uuid> = inner
            .iter()
            .filter(|(_, running)| running.last.elapsed() >= self.idle)
            .map(|(id, _)| *id)
            .collect();
        for id in stale {
            if let Some(mut running) = inner.remove(&id) {
                let _ = running.child.kill().await;
                let _ = running.child.wait().await;
            }
        }
    }

    pub async fn ensure<F, Fut>(
        &self,
        project_id: Uuid,
        command: &str,
        dir: &Path,
        env: &[(String, String)],
        pool: &sqlx::PgPool,
        prepare: F,
    ) -> anyhow::Result<u16>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<()>>,
    {
        let gate = self.gate(project_id).await;
        let _hold = gate.lock().await;
        {
            let mut inner = self.inner.lock().await;
            if let Some(running) = inner.get_mut(&project_id) {
                if running.child.try_wait()?.is_none() {
                    running.last = Instant::now();
                    return Ok(running.port);
                }
            }
            inner.remove(&project_id);
        }
        prepare().await?;
        let port = free_port()?;
        let child = spawn_site(command, dir, port, env, pool, project_id)?;
        if !wait_port(port).await {
            anyhow::bail!("site did not listen on {port}");
        }
        self.inner.lock().await.insert(
            project_id,
            Running {
                child,
                port,
                last: Instant::now(),
            },
        );
        Ok(port)
    }

    pub fn project_dir(&self, project_id: Uuid) -> PathBuf {
        self.workdir.join(project_id.to_string())
    }

    async fn gate(&self, project_id: Uuid) -> std::sync::Arc<Mutex<()>> {
        let mut locks = self.locks.lock().await;
        locks
            .entry(project_id)
            .or_insert_with(|| std::sync::Arc::new(Mutex::new(())))
            .clone()
    }
}

fn free_port() -> anyhow::Result<u16> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn spawn_site(
    command: &str,
    dir: &Path,
    port: u16,
    env: &[(String, String)],
    pool: &sqlx::PgPool,
    project_id: Uuid,
) -> anyhow::Result<tokio::process::Child> {
    let mut parts = command.split_whitespace();
    let Some(program) = parts.next() else {
        anyhow::bail!("empty site command");
    };
    let args: Vec<&str> = parts.collect();
    let mut child = Command::new(program);
    child
        .args(args)
        .current_dir(dir)
        .env_clear()
        .env("PATH", "/usr/local/bin:/usr/bin:/bin")
        .env("HOME", "/tmp")
        .env("PORT", port.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (key, value) in env {
        if crate::console::reserved_env_key(key) {
            continue;
        }
        child.env(key, value);
    }
    let mut child = child.spawn()?;
    if let Some(stdout) = child.stdout.take() {
        pump(pool.clone(), project_id, stdout);
    }
    if let Some(stderr) = child.stderr.take() {
        pump(pool.clone(), project_id, stderr);
    }
    Ok(child)
}

fn pump<R>(pool: sqlx::PgPool, project_id: Uuid, reader: R)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            crate::console::record_log(&pool, project_id, "site", "server", 0, line).await;
        }
    });
}

async fn wait_port(port: u16) -> bool {
    for _ in 0..50 {
        if TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

pub fn safe_site_command(command: &str) -> bool {
    !command.is_empty()
        && command.len() <= 200
        && command
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '.' | '_' | '/' | '-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_rejects_a_shell() {
        assert!(safe_site_command("node server.js"));
        assert!(safe_site_command("bun src/index.ts"));
        assert!(!safe_site_command("node server.js; rm -rf /"));
        assert!(!safe_site_command(""));
    }

    #[tokio::test]
    async fn idle_sweep_stops_the_child() {
        if std::process::Command::new("python3")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let root = std::env::temp_dir().join(format!("reactor-site-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join("app");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("server.py"),
            r#"import http.server, os
port = int(os.environ["PORT"])
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"ok")
    def log_message(self, fmt, *args):
        return
http.server.HTTPServer(("127.0.0.1", port), H).serve_forever()
"#,
        )
        .unwrap();
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://reactor:reactor@127.0.0.1:1/reactor")
            .unwrap();
        let sites = SiteSupervisor::new(root.clone(), Duration::from_millis(1));
        let id = Uuid::new_v4();
        let port = sites
            .ensure(
                id,
                "python3 server.py",
                &dir,
                &[("TOKEN".into(), "alpha".into())],
                &pool,
                || async { Ok(()) },
            )
            .await
            .unwrap();
        assert!(port > 0);
        tokio::time::sleep(Duration::from_millis(20)).await;
        sites.sweep().await;
        assert!(sites.inner.lock().await.get(&id).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
