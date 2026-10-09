use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::http::StatusCode;
use reactor_identity::Identity;
use reactor_storage::{public_object_url, reserved_bucket};
use sqlx::PgPool;

use crate::console::schema_name;
use crate::{internal, ApiError};

const FLAG_TTL: Duration = Duration::from_secs(5);

#[derive(Default)]
pub struct BucketFlags {
    inner: Mutex<HashMap<String, (bool, Instant)>>,
}

impl BucketFlags {
    pub fn get(&self, key: &str) -> Option<bool> {
        let guard = self.inner.lock().ok()?;
        let (value, at) = guard.get(key)?;
        if at.elapsed() < FLAG_TTL {
            Some(*value)
        } else {
            None
        }
    }

    pub fn put(&self, key: String, value: bool) {
        if let Ok(mut guard) = self.inner.lock() {
            guard.insert(key, (value, Instant::now()));
        }
    }
}

pub fn cache_key(pref: &str, bucket: &str) -> String {
    format!("{pref}/{bucket}")
}

pub fn public_base(cdn_public_base: &str, project_base: &str, bucket: &str) -> String {
    let host = if cdn_public_base.is_empty() {
        project_base
    } else {
        cdn_public_base
    };
    public_object_url(host, bucket, "").trim_end_matches('/').to_string()
}

pub fn split_public_path(path: &str) -> Result<(String, String), ApiError> {
    let path = path.trim_matches('/');
    let Some((bucket, key)) = path.split_once('/') else {
        return Err(ApiError::not_found());
    };
    if bucket.is_empty() || key.is_empty() || reserved_bucket(bucket) || key.contains("..") {
        return Err(ApiError::not_found());
    }
    Ok((bucket.to_string(), key.to_string()))
}

pub async fn authorize(
    pool: &PgPool,
    pref: &str,
    identity: &Identity,
    bucket: &str,
    name: &str,
    write: bool,
) -> Result<bool, ApiError> {
    let schema = schema_name(pref)?;
    let mut tx = pool.begin().await.map_err(internal)?;
    sqlx::query("SET LOCAL ROLE \"service\"")
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    let found: Option<bool> = match sqlx::query_scalar::<_, bool>(&format!(
        "SELECT \"public\" FROM \"{schema}\".storage_buckets WHERE id = $1"
    ))
    .bind(bucket)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(row) => row,
        Err(err) if missing_table(&err) => return Ok(true),
        Err(err) => return Err(internal(err)),
    };
    if found.is_none() {
        return Ok(true);
    }
    let role = identity.role.as_str();
    sqlx::query("RESET ROLE")
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    sqlx::query("SELECT set_config('request.jwt.claims', $1, true)")
        .bind(identity.claims.to_string())
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    sqlx::query(&format!("SET LOCAL ROLE \"{role}\""))
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    if write {
        let inserted = sqlx::query_scalar::<_, String>(&format!(
            "INSERT INTO \"{schema}\".storage_objects (bucket_id, name, owner) \
             VALUES ($1, $2, $3) ON CONFLICT (bucket_id, name) DO NOTHING RETURNING name"
        ))
        .bind(bucket)
        .bind(name)
        .bind(identity.user_id)
        .fetch_optional(&mut *tx)
        .await;
        match inserted {
            Ok(Some(_)) => {
                tx.commit().await.map_err(internal)?;
                return Ok(true);
            }
            Ok(None) => {}
            Err(err) if denied(&err) => return Ok(false),
            Err(err) => return Err(internal(err)),
        }
    }
    let visible = sqlx::query_scalar::<_, i32>(&format!(
        "SELECT 1 FROM \"{schema}\".storage_objects WHERE bucket_id = $1 AND name = $2"
    ))
    .bind(bucket)
    .bind(name)
    .fetch_optional(&mut *tx)
    .await;
    match visible {
        Ok(row) => {
            let allowed = row.is_some();
            if allowed {
                tx.commit().await.map_err(internal)?;
            }
            Ok(allowed)
        }
        Err(err) if denied(&err) => Ok(false),
        Err(err) => Err(internal(err)),
    }
}

pub async fn bucket_is_public(
    pool: &PgPool,
    pref: &str,
    bucket: &str,
) -> Result<Option<bool>, ApiError> {
    let schema = schema_name(pref)?;
    let mut tx = pool.begin().await.map_err(internal)?;
    sqlx::query("SET LOCAL ROLE \"service\"")
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    let found = sqlx::query_scalar::<_, bool>(&format!(
        "SELECT \"public\" FROM \"{schema}\".storage_buckets WHERE id = $1"
    ))
    .bind(bucket)
    .fetch_optional(&mut *tx)
    .await;
    match found {
        Ok(row) => Ok(row),
        Err(err) if missing_table(&err) => Ok(None),
        Err(err) => Err(internal(err)),
    }
}

pub async fn create_bucket(
    pool: &PgPool,
    pref: &str,
    name: &str,
    public: bool,
) -> Result<(), ApiError> {
    if reserved_bucket(name) || name.is_empty() || name.contains('/') || name.contains("..") {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "invalid bucket"));
    }
    let schema = schema_name(pref)?;
    let mut tx = pool.begin().await.map_err(internal)?;
    sqlx::query("SET LOCAL ROLE \"service\"")
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    let inserted = sqlx::query(&format!(
        "INSERT INTO \"{schema}\".storage_buckets (id, \"public\") VALUES ($1, $2)"
    ))
    .bind(name)
    .bind(public)
    .execute(&mut *tx)
    .await;
    match inserted {
        Ok(_) => {
            tx.commit().await.map_err(internal)?;
            Ok(())
        }
        Err(err) if missing_table(&err) => Err(ApiError::new(
            StatusCode::CONFLICT,
            "storage is not migrated",
        )),
        Err(err) if unique(&err) => Err(ApiError::new(StatusCode::CONFLICT, "bucket exists")),
        Err(err) => Err(internal(err)),
    }
}

pub async fn set_bucket_public(
    pool: &PgPool,
    pref: &str,
    name: &str,
    public: bool,
) -> Result<(), ApiError> {
    let schema = schema_name(pref)?;
    let mut tx = pool.begin().await.map_err(internal)?;
    sqlx::query("SET LOCAL ROLE \"service\"")
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    let updated = sqlx::query(&format!(
        "UPDATE \"{schema}\".storage_buckets SET \"public\" = $2 WHERE id = $1"
    ))
    .bind(name)
    .bind(public)
    .execute(&mut *tx)
    .await;
    match updated {
        Ok(done) if done.rows_affected() == 1 => {
            tx.commit().await.map_err(internal)?;
            Ok(())
        }
        Ok(_) => Err(ApiError::new(StatusCode::NOT_FOUND, "bucket not found")),
        Err(err) if missing_table(&err) => Err(ApiError::new(StatusCode::NOT_FOUND, "bucket not found")),
        Err(err) => Err(internal(err)),
    }
}

pub async fn list_buckets(
    pool: &PgPool,
    pref: &str,
) -> Result<Vec<(String, bool)>, ApiError> {
    let schema = schema_name(pref)?;
    let mut tx = pool.begin().await.map_err(internal)?;
    sqlx::query("SET LOCAL ROLE \"service\"")
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    let rows = sqlx::query_as::<_, (String, bool)>(&format!(
        "SELECT id, \"public\" FROM \"{schema}\".storage_buckets ORDER BY id"
    ))
    .fetch_all(&mut *tx)
    .await;
    match rows {
        Ok(rows) => Ok(rows),
        Err(err) if missing_table(&err) => Ok(Vec::new()),
        Err(err) => Err(internal(err)),
    }
}

pub async fn delete_object_row(pool: &PgPool, pref: &str, bucket: &str, name: &str) {
    let Ok(schema) = schema_name(pref) else {
        return;
    };
    let Ok(mut tx) = pool.begin().await else {
        return;
    };
    if sqlx::query("SET LOCAL ROLE \"service\"")
        .execute(&mut *tx)
        .await
        .is_err()
    {
        return;
    }
    let _ = sqlx::query(&format!(
        "DELETE FROM \"{schema}\".storage_objects WHERE bucket_id = $1 AND name = $2"
    ))
    .bind(bucket)
    .bind(name)
    .execute(&mut *tx)
    .await;
    let _ = tx.commit().await;
}

pub fn bucket_and_name(pref: &str, key: &str) -> Option<(String, String)> {
    let rest = key.strip_prefix(&format!("{pref}/"))?;
    let (bucket, name) = rest.split_once('/')?;
    if bucket.is_empty() || name.is_empty() {
        return None;
    }
    Some((bucket.to_string(), name.to_string()))
}

fn missing_table(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .and_then(|db| db.code().map(|code| code == "42P01"))
        .unwrap_or(false)
}

fn unique(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .and_then(|db| db.code().map(|code| code == "23505"))
        .unwrap_or(false)
}

fn denied(err: &sqlx::Error) -> bool {
    err.as_database_error()
        .and_then(|db| db.code().map(|code| code == "42501" || code == "23505"))
        .unwrap_or(false)
}
