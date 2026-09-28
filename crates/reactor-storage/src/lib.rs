use async_trait::async_trait;
use bytes::Bytes;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

#[derive(Clone, Debug)]
pub struct ObjectMeta {
    pub key: String,
    pub size: u64,
}

#[async_trait]
pub trait BlobStore: Send + Sync {
    async fn put(&self, key: &str, bytes: Bytes, content_type: &str) -> anyhow::Result<()>;
    async fn get(&self, key: &str) -> anyhow::Result<Option<(Bytes, String)>>;
    async fn delete_prefix(&self, prefix: &str) -> anyhow::Result<()>;
    async fn delete_key(&self, key: &str) -> anyhow::Result<()>;
    async fn list_prefix(&self, prefix: &str) -> anyhow::Result<Vec<String>>;
    async fn list_objects(&self, prefix: &str) -> anyhow::Result<Vec<ObjectMeta>>;
    async fn presign_put(&self, key: &str, expires_secs: u64) -> anyhow::Result<String>;
    async fn presign_get(&self, key: &str, expires_secs: u64) -> anyhow::Result<String>;
}

pub fn object_key(project_ref: &str, bucket: &str, key: &str) -> Result<String, &'static str> {
    if bucket.is_empty()
        || key.is_empty()
        || bucket.contains("..")
        || key.contains("..")
        || bucket.contains('/')
        || key.starts_with('/')
        || bucket.contains('\\')
        || key.contains('\\')
    {
        return Err("invalid object key");
    }
    Ok(format!("{project_ref}/{bucket}/{key}"))
}

pub fn verify_signature(secret: &[u8], op: &str, key: &str, exp: u64, sig: &str) -> bool {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if exp < now || key.contains("..") {
        return false;
    }
    let Ok(expect) = sign(secret, op, key, exp) else {
        return false;
    };
    subtle_eq(&expect, sig)
}

pub struct FsStore {
    root: PathBuf,
    public_base: String,
    secret: Vec<u8>,
}

impl FsStore {
    pub fn new(
        root: impl Into<PathBuf>,
        public_base: impl Into<String>,
        secret: impl AsRef<[u8]>,
    ) -> Self {
        Self {
            root: root.into(),
            public_base: public_base.into().trim_end_matches('/').to_string(),
            secret: secret.as_ref().to_vec(),
        }
    }

    fn path_for(&self, key: &str) -> anyhow::Result<PathBuf> {
        if key.contains("..") {
            anyhow::bail!("invalid key");
        }
        Ok(self.root.join(key))
    }

    pub fn verify(&self, op: &str, key: &str, exp: u64, sig: &str) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if exp < now {
            return false;
        }
        verify_signature(&self.secret, op, key, exp, sig)
    }

    fn signed_url(&self, op: &str, key: &str, expires_secs: u64) -> anyhow::Result<String> {
        let exp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() + expires_secs;
        let sig = sign(&self.secret, op, key, exp)?;
        let key_q = urlencoding(key);
        Ok(format!(
            "{}/storage/v1/signed?op={op}&key={key_q}&exp={exp}&sig={sig}",
            self.public_base
        ))
    }
}

fn urlencoding(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn sign(secret: &[u8], op: &str, key: &str, exp: u64) -> anyhow::Result<String> {
    let mut mac = HmacSha256::new_from_slice(secret)?;
    mac.update(format!("{op}\n{key}\n{exp}").as_bytes());
    Ok(hex::encode(mac.finalize().into_bytes()))
}

fn subtle_eq(a: &str, b: &str) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.bytes()
        .zip(b.bytes())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

#[async_trait]
impl BlobStore for FsStore {
    async fn put(&self, key: &str, bytes: Bytes, content_type: &str) -> anyhow::Result<()> {
        let path = self.path_for(key)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&path, &bytes).await?;
        tokio::fs::write(content_type_path(&path), content_type).await?;
        Ok(())
    }

    async fn get(&self, key: &str) -> anyhow::Result<Option<(Bytes, String)>> {
        let path = self.path_for(key)?;
        if !path.exists() {
            return Ok(None);
        }
        let bytes = tokio::fs::read(&path).await?;
        let ct_path = content_type_path(&path);
        let content_type = tokio::fs::read_to_string(&ct_path)
            .await
            .unwrap_or_else(|_| "application/octet-stream".into());
        Ok(Some((Bytes::from(bytes), content_type)))
    }

    async fn delete_prefix(&self, prefix: &str) -> anyhow::Result<()> {
        let path = self.path_for(prefix)?;
        if path.exists() {
            if path.is_dir() {
                tokio::fs::remove_dir_all(path).await?;
            } else {
                tokio::fs::remove_file(path).await?;
            }
        }
        Ok(())
    }

    async fn delete_key(&self, key: &str) -> anyhow::Result<()> {
        let path = self.path_for(key)?;
        if path.is_file() {
            tokio::fs::remove_file(&path).await?;
            let sidecar = content_type_path(&path);
            if sidecar.is_file() {
                tokio::fs::remove_file(sidecar).await.ok();
            }
        }
        Ok(())
    }

    async fn list_prefix(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        Ok(self
            .list_objects(prefix)
            .await?
            .into_iter()
            .map(|item| item.key)
            .collect())
    }

    async fn list_objects(&self, prefix: &str) -> anyhow::Result<Vec<ObjectMeta>> {
        let path = self.path_for(prefix)?;
        let mut out = Vec::new();
        if path.is_dir() {
            walk_meta(&path, &self.root, &mut out);
        }
        out.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(out)
    }

    async fn presign_put(&self, key: &str, expires_secs: u64) -> anyhow::Result<String> {
        self.signed_url("put", key, expires_secs)
    }

    async fn presign_get(&self, key: &str, expires_secs: u64) -> anyhow::Result<String> {
        self.signed_url("get", key, expires_secs)
    }
}

fn content_type_path(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(".content-type");
    PathBuf::from(os)
}

fn walk_meta(dir: &Path, root: &Path, out: &mut Vec<ObjectMeta>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk_meta(&path, root, out);
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.ends_with(".content-type") {
            continue;
        }
        if let Ok(rel) = path.strip_prefix(root) {
            let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
            out.push(ObjectMeta {
                key: rel.to_string_lossy().replace('\\', "/"),
                size,
            });
        }
    }
}

pub struct S3Store {
    client: aws_sdk_s3::Client,
    presign: aws_sdk_s3::Client,
    bucket: String,
}

pub fn uses_static_s3(endpoint: &str, access_key: &str, secret_key: &str) -> bool {
    !endpoint.is_empty() || !access_key.is_empty() || !secret_key.is_empty()
}

impl S3Store {
    pub async fn new(
        bucket: String,
        endpoint: String,
        public_endpoint: String,
        region: String,
        access_key: String,
        secret_key: String,
    ) -> anyhow::Result<Self> {
        let mut loader = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new(region));
        if uses_static_s3(&endpoint, &access_key, &secret_key) {
            let creds = aws_credential_types::Credentials::new(
                access_key, secret_key, None, None, "reactor",
            );
            loader = loader.credentials_provider(creds);
        }
        let shared = loader.load().await;
        let mut s3_conf = aws_sdk_s3::config::Builder::from(&shared);
        if !endpoint.is_empty() {
            s3_conf = s3_conf.endpoint_url(&endpoint).force_path_style(true);
        }
        let sign_endpoint = if public_endpoint.is_empty() {
            endpoint.as_str()
        } else {
            public_endpoint.as_str()
        };
        let mut sign_conf = aws_sdk_s3::config::Builder::from(&shared);
        if !sign_endpoint.is_empty() {
            sign_conf = sign_conf.endpoint_url(sign_endpoint).force_path_style(true);
        }
        Ok(Self {
            client: aws_sdk_s3::Client::from_conf(s3_conf.build()),
            presign: aws_sdk_s3::Client::from_conf(sign_conf.build()),
            bucket,
        })
    }

    pub async fn ensure_bucket(&self) -> anyhow::Result<()> {
        match self
            .client
            .create_bucket()
            .bucket(&self.bucket)
            .send()
            .await
        {
            Ok(_) => {}
            Err(err) => {
                let already = err
                    .as_service_error()
                    .map(|e| e.is_bucket_already_owned_by_you() || e.is_bucket_already_exists())
                    .unwrap_or(false);
                if !already {
                    self.client
                        .head_bucket()
                        .bucket(&self.bucket)
                        .send()
                        .await?;
                }
            }
        }
        let rule = aws_sdk_s3::types::CorsRule::builder()
            .allowed_origins("*")
            .allowed_methods("GET")
            .allowed_methods("PUT")
            .allowed_methods("HEAD")
            .allowed_headers("*")
            .expose_headers("ETag")
            .max_age_seconds(3000)
            .build()?;
        let cors = aws_sdk_s3::types::CorsConfiguration::builder()
            .cors_rules(rule)
            .build()?;
        self.client
            .put_bucket_cors()
            .bucket(&self.bucket)
            .cors_configuration(cors)
            .send()
            .await?;
        Ok(())
    }
}

#[async_trait]
impl BlobStore for S3Store {
    async fn put(&self, key: &str, bytes: Bytes, content_type: &str) -> anyhow::Result<()> {
        self.client
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .body(bytes.to_vec().into())
            .content_type(content_type)
            .send()
            .await?;
        Ok(())
    }

    async fn get(&self, key: &str) -> anyhow::Result<Option<(Bytes, String)>> {
        match self
            .client
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
        {
            Ok(out) => {
                let ct = out
                    .content_type()
                    .unwrap_or("application/octet-stream")
                    .to_string();
                let data = out.body.collect().await?.into_bytes();
                Ok(Some((data, ct)))
            }
            Err(err) => {
                if err
                    .as_service_error()
                    .map(|se| se.is_no_such_key())
                    .unwrap_or(false)
                {
                    Ok(None)
                } else {
                    Err(err.into())
                }
            }
        }
    }

    async fn delete_prefix(&self, prefix: &str) -> anyhow::Result<()> {
        let listed = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(prefix)
            .send()
            .await?;
        for obj in listed.contents() {
            if let Some(key) = obj.key() {
                self.client
                    .delete_object()
                    .bucket(&self.bucket)
                    .key(key)
                    .send()
                    .await?;
            }
        }
        Ok(())
    }

    async fn delete_key(&self, key: &str) -> anyhow::Result<()> {
        self.client
            .delete_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await?;
        Ok(())
    }

    async fn list_prefix(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        Ok(self
            .list_objects(prefix)
            .await?
            .into_iter()
            .map(|item| item.key)
            .collect())
    }

    async fn list_objects(&self, prefix: &str) -> anyhow::Result<Vec<ObjectMeta>> {
        let listed = self
            .client
            .list_objects_v2()
            .bucket(&self.bucket)
            .prefix(prefix)
            .send()
            .await?;
        let mut out = listed
            .contents()
            .iter()
            .filter_map(|obj| {
                obj.key().map(|key| ObjectMeta {
                    key: key.to_string(),
                    size: obj.size().unwrap_or(0).max(0) as u64,
                })
            })
            .collect::<Vec<_>>();
        out.sort_by(|a, b| a.key.cmp(&b.key));
        Ok(out)
    }

    async fn presign_put(&self, key: &str, expires_secs: u64) -> anyhow::Result<String> {
        let presigned = self
            .presign
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(aws_sdk_s3::presigning::PresigningConfig::expires_in(
                std::time::Duration::from_secs(expires_secs),
            )?)
            .await?;
        Ok(presigned.uri().to_string())
    }

    async fn presign_get(&self, key: &str, expires_secs: u64) -> anyhow::Result<String> {
        let presigned = self
            .presign
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(aws_sdk_s3::presigning::PresigningConfig::expires_in(
                std::time::Duration::from_secs(expires_secs),
            )?)
            .await?;
        Ok(presigned.uri().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_s3_settings_use_the_execution_role() {
        assert!(!uses_static_s3("", "", ""));
    }

    #[test]
    fn minio_keeps_static_credentials() {
        assert!(uses_static_s3("http://minio:9000", "minio", "minioadmin"));
        assert!(uses_static_s3("http://minio:9000", "", ""));
    }

    #[tokio::test]
    async fn fs_prefix_isolation() {
        let dir = std::env::temp_dir().join(format!("reactor-fs-{}", uuid_like()));
        let store = FsStore::new(&dir, "http://127.0.0.1:18000", b"secret");
        let a = object_key("aaaaaaaaaaaaaaaaaaaa", "files", "a.txt").unwrap();
        store
            .put(&a, Bytes::from_static(b"hello"), "text/plain")
            .await
            .unwrap();
        let got = store.get(&a).await.unwrap().unwrap();
        assert_eq!(got.0.as_ref(), b"hello");
        let b = object_key("bbbbbbbbbbbbbbbbbbbb", "files", "a.txt").unwrap();
        assert!(store.get(&b).await.unwrap().is_none());
        assert!(object_key("aaaaaaaaaaaaaaaaaaaa", "../bbbbbbbbbbbbbbbbbbbb", "a.txt").is_err());
        let url = store.presign_get(&a, 60).await.unwrap();
        assert!(url.contains("aaaaaaaaaaaaaaaaaaaa"));
        assert!(!url.contains("bbbbbbbbbbbbbbbbbbbb"));
        let _ = tokio::fs::remove_dir_all(dir).await;
    }

    fn uuid_like() -> String {
        format!("{}", std::process::id())
    }
}
