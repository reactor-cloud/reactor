use sqlx::PgPool;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub async fn apply_control(pool: &PgPool, sql_dir: &Path, password: &str) -> anyhow::Result<()> {
    let dir = sql_dir.join("control");
    let mut names = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".sql"))
        .collect::<Vec<_>>();
    names.sort();
    for name in names {
        let file = std::fs::read_to_string(dir.join(&name))?.replace("__AUTH_PASSWORD__", password);
        let mut body = String::from("BEGIN;\nSELECT pg_advisory_xact_lock(4815162343);\n");
        for stmt in split_sql(&file) {
            body.push_str(&stmt);
            body.push_str(";\n");
        }
        body.push_str("COMMIT;\n");
        if let Err(err) = sqlx::raw_sql(&body).execute(pool).await {
            anyhow::bail!("{name}: {err:#}");
        }
    }
    Ok(())
}

pub async fn apply_project(
    pool: &PgPool,
    sql_dir: &Path,
    project_id: Uuid,
    schema: &str,
) -> anyhow::Result<()> {
    apply_project_owned(
        pool.clone(),
        sql_dir.to_path_buf(),
        project_id,
        schema.to_string(),
    )
    .await
}

async fn apply_project_owned(
    pool: PgPool,
    sql_dir: PathBuf,
    project_id: Uuid,
    schema: String,
) -> anyhow::Result<()> {
    let create_schema = format!("CREATE SCHEMA IF NOT EXISTS \"{schema}\"");
    sqlx::raw_sql(&create_schema).execute(&pool).await?;
    let pref = schema.strip_prefix("proj_").unwrap_or(schema.as_str());
    sqlx::query(
        "INSERT INTO reactor.projects (id, ref, name) VALUES ($1, $2, '') ON CONFLICT (id) DO NOTHING",
    )
    .bind(project_id)
    .bind(pref)
    .execute(&pool)
    .await?;
    let dir = sql_dir.join("project");
    let mut names = std::fs::read_dir(&dir)?
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".sql"))
        .collect::<Vec<_>>();
    names.sort();
    for name in names {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM reactor.schema_migrations WHERE project_id = $1 AND version = $2",
        )
        .bind(project_id)
        .bind(&name)
        .fetch_one(&pool)
        .await?;
        if count > 0 {
            continue;
        }
        let body = std::fs::read_to_string(dir.join(&name))?;
        let script =
            format!("SET search_path TO \"{schema}\";\n{body}\nSET search_path TO public;");
        sqlx::raw_sql(&script).execute(&pool).await?;
        sqlx::query("INSERT INTO reactor.schema_migrations (project_id, version) VALUES ($1, $2)")
            .bind(project_id)
            .bind(&name)
            .execute(&pool)
            .await?;
    }
    let grants = format!(
        "GRANT USAGE ON SCHEMA \"{schema}\" TO anon, authenticated, service; \
         GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA \"{schema}\" TO authenticated, service; \
         GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA \"{schema}\" TO authenticated, service"
    );
    exec_script(&pool, &grants).await?;
    Ok(())
}

pub async fn sync_schemas(primary: &PgPool, dedicated: Option<&PgPool>) -> anyhow::Result<()> {
    let primary = primary.clone();
    let shared: Vec<String> = sqlx::query_scalar(
        "SELECT 'proj_' || ref FROM reactor.projects WHERE database_url IS NULL ORDER BY ref",
    )
    .fetch_all(&primary)
    .await?;
    set_db_schemas(&primary, &shared).await?;
    if let Some(dedicated) = dedicated {
        let dedicated = dedicated.clone();
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT 'proj_' || ref FROM reactor.projects WHERE database_url IS NOT NULL ORDER BY ref",
        )
        .fetch_all(&primary)
        .await?;
        set_db_schemas(&dedicated, &rows).await?;
    }
    Ok(())
}

async fn set_db_schemas(pool: &PgPool, schemas: &[String]) -> anyhow::Result<()> {
    let list = if schemas.is_empty() {
        "reactor_api".to_string()
    } else {
        schemas.join(",")
    };
    if !list
        .split(',')
        .all(|s| s == "reactor_api" || (s.starts_with("proj_") && s.len() == 25))
    {
        anyhow::bail!("refusing to set db schemas");
    }
    let alter = format!("ALTER ROLE authenticator SET pgrst.db_schemas = '{list}'");
    if let Err(err) = sqlx::raw_sql(&alter).execute(pool).await {
        tracing::warn!("role schema setting failed, using postgrest config: {err:#}");
        rewrite_postgrest_schemas(&list)?;
    }
    // Aurora cannot ALTER ROLE, so the schema list lives in the PostgREST config file.
    // Reload still has to run or a project created after boot stays invisible to /data.
    sqlx::raw_sql("NOTIFY pgrst, 'reload config'")
        .execute(pool)
        .await?;
    sqlx::raw_sql("NOTIFY pgrst, 'reload schema'")
        .execute(pool)
        .await?;
    Ok(())
}

fn rewrite_postgrest_schemas(list: &str) -> anyhow::Result<()> {
    let path = std::env::var("REACTOR_PGRST_CONF").unwrap_or_default();
    if path.is_empty() {
        anyhow::bail!("REACTOR_PGRST_CONF is unset");
    }
    let text = std::fs::read_to_string(&path)?;
    std::fs::write(&path, replace_db_schemas(&text, list))?;
    Ok(())
}

pub fn replace_db_schemas(text: &str, schemas: &str) -> String {
    let mut out = String::new();
    let mut found = false;
    for line in text.lines() {
        if line.starts_with("db-schemas") {
            out.push_str(&format!("db-schemas = \"{schemas}\"\n"));
            found = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !found {
        out.push_str(&format!("db-schemas = \"{schemas}\"\n"));
    }
    out
}

pub async fn migrate_all(
    primary: &PgPool,
    dedicated: Option<&PgPool>,
    sql_dir: &Path,
    dedicated_url: Option<&str>,
) -> anyhow::Result<()> {
    let primary = primary.clone();
    let dedicated = dedicated.cloned();
    let sql_dir = sql_dir.to_path_buf();
    let dedicated_url = dedicated_url.map(|s| s.to_string());
    let rows: Vec<(Uuid, String, Option<String>)> =
        sqlx::query_as("SELECT id, ref, database_url FROM reactor.projects ORDER BY ref")
            .fetch_all(&primary)
            .await?;
    for (id, pref, database_url) in rows {
        let schema = format!("proj_{pref}");
        let pool = match database_url {
            Some(url) => {
                if dedicated_url.as_deref() == Some(url.as_str()) {
                    dedicated.clone().unwrap()
                } else {
                    sqlx::postgres::PgPoolOptions::new()
                        .max_connections(1)
                        .connect(&url)
                        .await?
                }
            }
            None => primary.clone(),
        };
        apply_project_owned(pool, sql_dir.clone(), id, schema).await?;
    }
    sync_schemas(&primary, dedicated.as_ref()).await?;
    Ok(())
}

async fn exec_script(pool: &PgPool, sql: &str) -> anyhow::Result<()> {
    let pool = pool.clone();
    let sql = sql.to_string();
    run_script(pool, sql).await
}

async fn run_script(pool: PgPool, sql: String) -> anyhow::Result<()> {
    sqlx::raw_sql(&sql).execute(&pool).await?;
    Ok(())
}

fn split_sql(sql: &str) -> Vec<String> {
    let bytes = sql.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        if bytes[i] == b'\'' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\'' {
                    if i + 1 < bytes.len() && bytes[i + 1] == b'\'' {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            continue;
        }
        if bytes[i] == b'$' {
            if let Some(tag_end) = dollar_tag_end(bytes, i) {
                let tag = &sql[i..tag_end];
                i = tag_end;
                if let Some(rel) = sql[i..].find(tag) {
                    i += rel + tag.len();
                } else {
                    i = bytes.len();
                }
                continue;
            }
        }
        if bytes[i] == b';' {
            let stmt = sql[start..i].trim();
            if !stmt.is_empty() {
                out.push(stmt.to_string());
            }
            i += 1;
            start = i;
            continue;
        }
        i += 1;
    }
    let tail = sql[start..].trim();
    if !tail.is_empty() {
        out.push(tail.to_string());
    }
    out
}

fn dollar_tag_end(bytes: &[u8], i: usize) -> Option<usize> {
    if bytes.get(i) != Some(&b'$') {
        return None;
    }
    let mut j = i + 1;
    while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
        j += 1;
    }
    if j < bytes.len() && bytes[j] == b'$' {
        Some(j + 1)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{replace_db_schemas, split_sql};

    #[test]
    fn rewrites_only_the_schema_line() {
        let text =
            "db-uri = \"postgres://local\"\ndb-schemas = \"reactor_api\"\nserver-port = 3000\n";
        let next = replace_db_schemas(text, "reactor_api,proj_aaaaaaaaaaaaaaaaaaaa");
        assert!(next.contains("db-schemas = \"reactor_api,proj_aaaaaaaaaaaaaaaaaaaa\""));
        assert!(next.contains("db-uri = \"postgres://local\""));
        assert!(next.contains("server-port = 3000"));
    }

    #[test]
    fn keeps_dollar_quoted_function_together() {
        let sql = "CREATE OR REPLACE FUNCTION reactor.pre_request() RETURNS void AS $$\nBEGIN\n  PERFORM 1;\nEND;\n$$;\nGRANT EXECUTE ON FUNCTION reactor.pre_request() TO anon;";
        let parts = split_sql(sql);
        assert_eq!(parts.len(), 2);
        assert!(parts[0].contains("PERFORM 1;"));
        assert!(parts[1].starts_with("GRANT"));
    }
}
