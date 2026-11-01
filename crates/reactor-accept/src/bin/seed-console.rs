use anyhow::Context;
use reactor_auth::{hash_password, token_hash};
use sqlx::PgPool;
use uuid::Uuid;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let database = std::env::var("REACTOR_ACCEPT_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://reactor:reactor@127.0.0.1:5440/reactor".to_string());
    let email = std::env::var("REACTOR_CONSOLE_EMAIL")
        .unwrap_or_else(|_| "console-contract@example.com".to_string());
    let password =
        std::env::var("REACTOR_CONSOLE_PASSWORD").unwrap_or_else(|_| "scripted-pass".to_string());
    let recovery = std::env::var("REACTOR_CONSOLE_RECOVERY")
        .unwrap_or_else(|_| "console-contract-recovery".to_string());
    let console_url = std::env::var("REACTOR_CONSOLE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:18000".to_string());
    let env_file =
        std::env::var("CONSOLE_ENV_FILE").unwrap_or_else(|_| ".data/console.env".to_string());

    let pool = PgPool::connect(&database).await?;
    let hash = hash_password(&password)?;
    let existing: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM reactor.operators WHERE email = $1")
            .bind(&email)
            .fetch_optional(&pool)
            .await?;
    let id = if let Some(id) = existing {
        sqlx::query(
            "UPDATE reactor.operators SET password_hash = $2, platform_admin = true, name = 'Contract' WHERE id = $1",
        )
        .bind(id)
        .bind(&hash)
        .execute(&pool)
        .await?;
        id
    } else {
        let id = Uuid::new_v4();
        sqlx::query(
            "INSERT INTO reactor.operators (id, email, name, password_hash, platform_admin) VALUES ($1, $2, 'Contract', $3, true)",
        )
        .bind(id)
        .bind(&email)
        .bind(&hash)
        .execute(&pool)
        .await?;
        id
    };
    sqlx::query("DELETE FROM reactor.operator_factors WHERE operator_id = $1")
        .bind(id)
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO reactor.operator_factors (operator_id, kind, secret) VALUES ($1, 'totp', 'test')",
    )
    .bind(id)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO reactor.operator_factors (operator_id, kind, credential) VALUES ($1, 'passkey', '{}'::jsonb)",
    )
    .bind(id)
    .execute(&pool)
    .await?;
    sqlx::query("DELETE FROM reactor.operator_recovery_codes WHERE operator_id = $1")
        .bind(id)
        .execute(&pool)
        .await?;
    sqlx::query(
        "INSERT INTO reactor.operator_recovery_codes (operator_id, code_hash) VALUES ($1, $2)",
    )
    .bind(id)
    .bind(token_hash(&recovery))
    .execute(&pool)
    .await?;

    let body = format!(
        "REACTOR_CONSOLE_URL={console_url}\nREACTOR_CONSOLE_EMAIL={email}\nREACTOR_CONSOLE_PASSWORD={password}\nREACTOR_CONSOLE_RECOVERY={recovery}\n"
    );
    if let Some(parent) = std::path::Path::new(&env_file).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
    }
    std::fs::write(&env_file, &body).with_context(|| format!("write {env_file}"))?;
    print!("{body}");
    Ok(())
}
