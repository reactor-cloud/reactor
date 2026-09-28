use async_trait::async_trait;
use bytes::Bytes;
use hex::encode as hex_encode;
use reactor_storage::BlobStore;
use sha2::{Digest, Sha256};
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

#[derive(Debug, Clone)]
pub struct PublishRecord {
    pub name: String,
    pub zip_hash: String,
    pub env: Vec<(String, String)>,
}

#[async_trait]
pub trait FunctionPublisher: Send + Sync {
    async fn publish(&self, name: &str, zip: &[u8], env: &[(&str, &str)])
        -> anyhow::Result<String>;
    async fn invoke(&self, name: &str, payload: &[u8]) -> anyhow::Result<Vec<u8>>;
}

#[derive(Debug, Clone)]
pub struct InvokeRecord {
    pub name: String,
    pub payload: Vec<u8>,
}

pub struct FakePublisher {
    pub records: Mutex<Vec<PublishRecord>>,
    pub invokes: Mutex<Vec<InvokeRecord>>,
}

impl FakePublisher {
    pub fn new() -> Self {
        Self {
            records: Mutex::new(Vec::new()),
            invokes: Mutex::new(Vec::new()),
        }
    }
}

impl Default for FakePublisher {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FunctionPublisher for FakePublisher {
    async fn publish(
        &self,
        name: &str,
        zip: &[u8],
        env: &[(&str, &str)],
    ) -> anyhow::Result<String> {
        let env = scrub_env(env);
        self.records.lock().unwrap().push(PublishRecord {
            name: name.to_string(),
            zip_hash: zip_hash(zip),
            env,
        });
        Ok(name.to_string())
    }

    async fn invoke(&self, name: &str, payload: &[u8]) -> anyhow::Result<Vec<u8>> {
        self.invokes.lock().unwrap().push(InvokeRecord {
            name: name.to_string(),
            payload: payload.to_vec(),
        });
        Ok(payload.to_vec())
    }
}

pub struct AwsLambdaPublisher {
    client: aws_sdk_lambda::Client,
    role_arn: String,
    layer_arn: String,
}

impl AwsLambdaPublisher {
    pub async fn new(role_arn: String, region: String, layer_arn: String) -> anyhow::Result<Self> {
        let shared = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new(region))
            .load()
            .await;
        Ok(Self {
            client: aws_sdk_lambda::Client::new(&shared),
            role_arn,
            layer_arn,
        })
    }

    async fn put_env(&self, name: &str, env: &[(String, String)]) -> anyhow::Result<()> {
        for _ in 0..8 {
            let mut environment = aws_sdk_lambda::types::Environment::builder();
            for (key, value) in env {
                environment = environment.variables(key, value);
            }
            match self
                .client
                .update_function_configuration()
                .function_name(name)
                .environment(environment.build())
                .send()
                .await
            {
                Ok(_) => return Ok(()),
                Err(err) if err.to_string().contains("ResourceConflict") => {
                    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                }
                Err(err) => return Err(err.into()),
            }
        }
        anyhow::bail!("function configuration stayed busy")
    }
}

#[async_trait]
impl FunctionPublisher for AwsLambdaPublisher {
    async fn publish(
        &self,
        name: &str,
        zip: &[u8],
        env: &[(&str, &str)],
    ) -> anyhow::Result<String> {
        let env = scrub_env(env);
        let wrapped = wrap_lambda_zip(zip)?;
        let blob = aws_sdk_lambda::primitives::Blob::new(wrapped);
        if self
            .client
            .get_function()
            .function_name(name)
            .send()
            .await
            .is_ok()
        {
            self.client
                .update_function_code()
                .function_name(name)
                .zip_file(blob)
                .send()
                .await?;
            self.put_env(name, &env).await?;
            return Ok(name.to_string());
        }
        let mut environment = aws_sdk_lambda::types::Environment::builder();
        for (key, value) in &env {
            environment = environment.variables(key, value);
        }
        let mut create = self
            .client
            .create_function()
            .function_name(name)
            .runtime(aws_sdk_lambda::types::Runtime::Providedal2023)
            .role(&self.role_arn)
            .handler("bootstrap")
            .architectures(aws_sdk_lambda::types::Architecture::Arm64)
            .timeout(30)
            .memory_size(256)
            .code(
                aws_sdk_lambda::types::FunctionCode::builder()
                    .zip_file(blob)
                    .build(),
            )
            .environment(environment.build());
        if !self.layer_arn.is_empty() {
            create = create.layers(&self.layer_arn);
        }
        create.send().await?;
        Ok(name.to_string())
    }

    async fn invoke(&self, name: &str, payload: &[u8]) -> anyhow::Result<Vec<u8>> {
        let out = self
            .client
            .invoke()
            .function_name(name)
            .payload(aws_sdk_lambda::primitives::Blob::new(payload.to_vec()))
            .send()
            .await?;
        if out.function_error().is_some() {
            anyhow::bail!(
                "function error: {}",
                out.payload()
                    .map(|p| String::from_utf8_lossy(p.as_ref()).to_string())
                    .unwrap_or_default()
            );
        }
        Ok(out
            .payload()
            .map(|p| p.as_ref().to_vec())
            .unwrap_or_default())
    }
}

pub fn reserved_runtime_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    upper == "PATH"
        || upper == "HOME"
        || (key.starts_with("REACTOR_") && key != "REACTOR_PROJECT_REF")
}

pub fn scrub_env(env: &[(&str, &str)]) -> Vec<(String, String)> {
    env.iter()
        .filter(|(k, _)| !k.is_empty() && !reserved_runtime_key(k))
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

pub fn wrap_lambda_zip(user_zip: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let opts = zip::write::SimpleFileOptions::default();
        let bootstrap_opts = opts.unix_permissions(0o755);
        let mut archive = zip::ZipArchive::new(Cursor::new(user_zip))?;
        let mut has_bootstrap = false;
        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let name = file.name().to_string();
            if name.contains("..") || name.starts_with('/') {
                anyhow::bail!("unsafe zip path");
            }
            if name == "bootstrap" {
                has_bootstrap = true;
            }
            if file.is_dir() {
                writer.add_directory(name, opts)?;
                continue;
            }
            writer.start_file(name, opts)?;
            std::io::copy(&mut file, &mut writer)?;
        }
        if !has_bootstrap {
            writer.start_file("bootstrap", bootstrap_opts)?;
            writer.write_all(LAMBDA_BOOTSTRAP.as_bytes())?;
        }
        writer.finish()?;
    }
    Ok(cursor.into_inner())
}

const LAMBDA_BOOTSTRAP: &str = r#"#!/bin/sh
set -eu
RUNTIME_API="http://${AWS_LAMBDA_RUNTIME_API}/2018-06-01/runtime"
while true; do
  HEADERS=$(mktemp)
  EVENT=$(curl -sS -D "$HEADERS" "$RUNTIME_API/invocation/next")
  REQUEST_ID=$(grep -i '^Lambda-Runtime-Aws-Request-Id:' "$HEADERS" | awk '{print $2}' | tr -d '\r')
  rm -f "$HEADERS"
  if [ -z "$REQUEST_ID" ]; then
    exit 1
  fi
  export REACTOR_CALLER=$(printf '%s' "$EVENT" | /opt/bin/bun -e 'let s="";process.stdin.on("data",d=>s+=d);process.stdin.on("end",()=>{try{const e=JSON.parse(s);process.stdout.write(JSON.stringify(e.caller||{}))}catch{process.stdout.write("{}")}})')
  BODY=$(printf '%s' "$EVENT" | /opt/bin/bun -e 'let s="";process.stdin.on("data",d=>s+=d);process.stdin.on("end",()=>{try{const e=JSON.parse(s);process.stdout.write(typeof e.body==="string"?e.body:s)}catch{process.stdout.write(s)}})')
  if OUT=$(printf '%s' "$BODY" | /opt/bin/bun /var/task/index.ts 2>/tmp/fn.err); then
    curl -sS -X POST "$RUNTIME_API/invocation/$REQUEST_ID/response" --data-binary "$OUT" >/dev/null
  else
    ERR=$(tr '\n' ' ' < /tmp/fn.err 2>/dev/null || echo function failed)
    curl -sS -X POST "$RUNTIME_API/invocation/$REQUEST_ID/error" \
      -H 'Lambda-Runtime-Function-Error-Type: Runtime.ExitError' \
      -d "{\"errorMessage\":\"$ERR\"}" >/dev/null || true
  fi
done
"#;

pub fn zip_hash(bytes: &[u8]) -> String {
    hex_encode(Sha256::digest(bytes))
}

pub struct BunRuntime {
    pub blobs: std::sync::Arc<dyn BlobStore>,
    pub workdir: PathBuf,
    pub bun_bin: String,
}

impl BunRuntime {
    pub fn new(blobs: std::sync::Arc<dyn BlobStore>, workdir: PathBuf, bun_bin: String) -> Self {
        Self {
            blobs,
            workdir,
            bun_bin,
        }
    }

    pub async fn ensure(&self, blob_key: &str, dest: &Path) -> anyhow::Result<()> {
        let marker = dest.join("index.ts");
        if marker.exists() {
            return Ok(());
        }
        let Some((bytes, _)) = self.blobs.get(blob_key).await? else {
            anyhow::bail!("function blob missing");
        };
        if dest.exists() {
            tokio::fs::remove_dir_all(dest).await.ok();
        }
        tokio::fs::create_dir_all(dest).await?;
        extract_zip(&bytes, dest)?;
        Ok(())
    }

    pub async fn invoke(
        &self,
        dest: &Path,
        caller: &serde_json::Value,
        body: &[u8],
        env: &[(&str, &str)],
    ) -> anyhow::Result<Vec<u8>> {
        let mut child = Command::new(&self.bun_bin);
        child
            .arg("index.ts")
            .current_dir(dest)
            .env_clear()
            .env("PATH", bun_path())
            .env("HOME", "/tmp")
            .env("REACTOR_CALLER", caller.to_string())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (key, value) in env {
            if key.is_empty()
                || *key == "PATH"
                || *key == "HOME"
                || *key == "REACTOR_CALLER"
                || key.starts_with("REACTOR_")
            {
                continue;
            }
            child.env(key, value);
        }
        let mut child = child.spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(body).await?;
        }
        let out =
            tokio::time::timeout(std::time::Duration::from_secs(30), child.wait_with_output())
                .await??;
        if !out.status.success() {
            anyhow::bail!("function failed: {}", String::from_utf8_lossy(&out.stderr));
        }
        Ok(out.stdout)
    }
}

fn bun_path() -> String {
    if let Ok(path) = std::env::var("PATH") {
        if path.contains("bun") || std::path::Path::new("/usr/local/bin/bun").exists() {
            return format!("/usr/local/bin:/usr/bin:/bin:{path}");
        }
        return path;
    }
    "/usr/local/bin:/usr/bin:/bin".into()
}

pub fn extract_zip(bytes: &[u8], dest: &Path) -> anyhow::Result<()> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_string();
        if name.contains("..") || name.starts_with('/') {
            anyhow::bail!("unsafe zip path");
        }
        let out = dest.join(&name);
        if file.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut dest_file = std::fs::File::create(&out)?;
        std::io::copy(&mut file, &mut dest_file)?;
    }
    Ok(())
}

pub fn zip_single(name: &str, contents: &str) -> anyhow::Result<Bytes> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let opts = zip::write::SimpleFileOptions::default();
        writer.start_file(name, opts)?;
        writer.write_all(contents.as_bytes())?;
        writer.finish()?;
    }
    Ok(Bytes::from(cursor.into_inner()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_keeps_user_secrets_and_drops_reserved_names() {
        let env = scrub_env(&[
            ("API_SECRET", "sekret"),
            ("DATABASE_URL", "postgres://user"),
            ("PATH", "/bin"),
            ("HOME", "/root"),
            ("REACTOR_CALLER", "{}"),
            ("REACTOR_PROJECT_REF", "pref"),
        ]);
        assert_eq!(
            env,
            vec![
                ("API_SECRET".into(), "sekret".into()),
                ("DATABASE_URL".into(), "postgres://user".into()),
                ("REACTOR_PROJECT_REF".into(), "pref".into()),
            ]
        );
    }

    #[tokio::test]
    async fn invoke_receives_only_the_function_env() {
        if std::process::Command::new("bun")
            .arg("--version")
            .output()
            .is_err()
        {
            return;
        }
        let dest = std::env::temp_dir().join(format!("reactor-fn-env-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&dest).await;
        tokio::fs::create_dir_all(&dest).await.unwrap();
        tokio::fs::write(
            dest.join("index.ts"),
            r#"process.stdout.write(JSON.stringify({
  mine: process.env.MINE ?? null,
  secret: process.env.API_SECRET ?? null,
  other: process.env.OTHER ?? null,
  database: process.env.DATABASE_URL ?? null,
  caller: process.env.REACTOR_CALLER ?? null,
}))"#,
        )
        .await
        .unwrap();
        std::env::set_var("DATABASE_URL", "postgres://parent");
        let store = reactor_storage::FsStore::new(&dest, "http://localhost", b"secret");
        let runtime = BunRuntime::new(std::sync::Arc::new(store), dest.clone(), "bun".into());
        let out = runtime
            .invoke(
                &dest,
                &serde_json::json!({"sub": "user"}),
                b"{}",
                &[
                    ("MINE", "alpha"),
                    ("API_SECRET", "sekret"),
                    ("REACTOR_CALLER", "hijack"),
                ],
            )
            .await
            .unwrap();
        let _ = tokio::fs::remove_dir_all(&dest).await;
        let body: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(body["mine"], "alpha");
        assert_eq!(body["secret"], "sekret");
        assert_eq!(body["other"], serde_json::Value::Null);
        assert_eq!(body["database"], serde_json::Value::Null);
        assert_eq!(body["caller"], r#"{"sub":"user"}"#);
    }

    #[tokio::test]
    async fn fake_publisher_records_publish_and_invoke() {
        let fake = FakePublisher::new();
        let zip = b"zip-bytes";
        fake.publish(
            "fn",
            zip,
            &[("API_SECRET", "sekret"), ("PATH", "/bin"), ("NAME", "fn")],
        )
        .await
        .unwrap();
        let out = fake.invoke("fn", br#"{"ok":true}"#).await.unwrap();
        assert_eq!(out, br#"{"ok":true}"#);
        let records = fake.records.lock().unwrap();
        assert_eq!(records[0].name, "fn");
        assert_eq!(records[0].zip_hash, zip_hash(zip));
        assert_eq!(
            records[0].env,
            vec![
                ("API_SECRET".into(), "sekret".into()),
                ("NAME".into(), "fn".into()),
            ]
        );
        drop(records);
        let invokes = fake.invokes.lock().unwrap();
        assert_eq!(invokes[0].name, "fn");
        assert_eq!(invokes[0].payload, br#"{"ok":true}"#);
    }

    #[test]
    fn lambda_zip_includes_bootstrap_and_user_code() {
        let user = zip_single("index.ts", "process.stdout.write('ok')").unwrap();
        let wrapped = wrap_lambda_zip(&user).unwrap();
        let mut archive = zip::ZipArchive::new(Cursor::new(wrapped)).unwrap();
        let mut names = Vec::new();
        for i in 0..archive.len() {
            names.push(archive.by_index(i).unwrap().name().to_string());
        }
        assert!(names.iter().any(|name| name == "index.ts"));
        assert!(names.iter().any(|name| name == "bootstrap"));
    }
}
