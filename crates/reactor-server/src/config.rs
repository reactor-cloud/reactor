use serde::Deserialize;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct Config {
    pub mode: String,
    pub place: String,
    pub base_domain: String,
    pub bind: String,
    pub public_url: String,
    pub trusted_proxy: bool,
    pub provider: String,
    pub operator_token: String,
    pub database_url: String,
    pub dedicated_database_url: Option<String>,
    pub authenticator_password: String,
    pub jwt_private_pem_file: Option<String>,
    pub jwt_dir: String,
    pub storage_backend: String,
    pub storage_bucket: String,
    pub storage_endpoint: String,
    pub storage_public_endpoint: String,
    pub storage_access_key: String,
    pub storage_secret_key: String,
    pub storage_region: String,
    pub storage_fs_root: String,
    pub storage_sign_secret: String,
    pub functions_runtime: String,
    pub functions_workdir: String,
    pub functions_bun: String,
    pub functions_publisher: String,
    pub lambda_role_arn: String,
    pub lambda_layer_arn: String,
    pub sites_workdir: String,
    pub sites_idle_secs: u64,
    pub postgrest_url: String,
    pub postgrest_dedicated_url: String,
    pub sql_dir: String,
    pub dns_stub_file: Option<String>,
    pub handler: String,
    pub console_dir: String,
    pub agent_api_key: String,
    pub agent_base_url: String,
    pub agent_model: String,
    pub agent_turn_deadline_secs: u64,
}

#[derive(Debug, Deserialize, Default)]
struct FileConfig {
    #[serde(default)]
    runtime: Section,
    #[serde(default)]
    http: Section,
    #[serde(default)]
    auth: Section,
    #[serde(default)]
    database: Section,
    #[serde(default)]
    storage: Section,
    #[serde(default)]
    functions: Section,
    #[serde(default)]
    postgrest: Section,
}

#[derive(Debug, Deserialize, Default)]
struct Section {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    place: Option<String>,
    #[serde(default)]
    base_domain: Option<String>,
    #[serde(default)]
    bind: Option<String>,
    #[serde(default)]
    public_url: Option<String>,
    #[serde(default)]
    trusted_proxy: Option<String>,
    #[serde(default)]
    operator_token: Option<String>,
    #[serde(default)]
    provider: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    dedicated_url: Option<String>,
    #[serde(default)]
    authenticator_password: Option<String>,
    #[serde(default)]
    jwt_private_pem_file: Option<String>,
    #[serde(default)]
    backend: Option<String>,
    #[serde(default)]
    bucket: Option<String>,
    #[serde(default)]
    endpoint: Option<String>,
    #[serde(default)]
    public_endpoint: Option<String>,
    #[serde(default)]
    access_key: Option<String>,
    #[serde(default)]
    secret_key: Option<String>,
    #[serde(default)]
    region: Option<String>,
    #[serde(default)]
    fs_root: Option<String>,
    #[serde(default)]
    sign_secret: Option<String>,
    #[serde(default)]
    runtime: Option<String>,
    #[serde(default)]
    workdir: Option<String>,
    #[serde(default)]
    bun: Option<String>,
    #[serde(default)]
    publisher: Option<String>,
    #[serde(default)]
    lambda_role_arn: Option<String>,
    #[serde(default)]
    lambda_layer_arn: Option<String>,
    #[serde(default)]
    sites_workdir: Option<String>,
    #[serde(default)]
    sites_idle_secs: Option<u64>,
    #[serde(default)]
    dedicated: Option<String>,
}

impl Config {
    pub fn load() -> Self {
        let file = Path::new("Reactor.toml")
            .exists()
            .then(|| std::fs::read_to_string("Reactor.toml").ok())
            .flatten()
            .and_then(|text| toml::from_str::<FileConfig>(&text).ok())
            .unwrap_or_default();

        let operator = pick(
            "REACTOR_AUTH__OPERATOR_TOKEN",
            file.auth.operator_token.clone(),
            "",
        );
        let sign = pick(
            "REACTOR_STORAGE__SIGN_SECRET",
            file.storage.sign_secret.clone(),
            if operator.is_empty() {
                "dev-sign"
            } else {
                &operator
            },
        );
        Self {
            mode: pick("REACTOR_RUNTIME__MODE", file.runtime.mode.clone(), "listen"),
            place: place(&file),
            base_domain: pick(
                "REACTOR_RUNTIME__BASE_DOMAIN",
                file.runtime.base_domain.clone(),
                "apps.localhost",
            ),
            bind: pick("REACTOR_HTTP__BIND", file.http.bind.clone(), "0.0.0.0:8000"),
            public_url: pick(
                "REACTOR_HTTP__PUBLIC_URL",
                file.http.public_url.clone(),
                "http://127.0.0.1:18000",
            ),
            trusted_proxy: matches!(
                pick(
                    "REACTOR_HTTP__TRUSTED_PROXY",
                    file.http.trusted_proxy.clone(),
                    "false",
                )
                .as_str(),
                "1" | "true" | "yes"
            ),
            provider: match std::env::var("REACTOR_AUTH__PROVIDER") {
                Ok(value) => value,
                Err(_) => file
                    .auth
                    .provider
                    .clone()
                    .unwrap_or_else(|| "internal".to_string()),
            },
            operator_token: operator,
            database_url: pick("REACTOR_DATABASE__URL", file.database.url.clone(), ""),
            dedicated_database_url: opt_pick(
                "REACTOR_DATABASE__DEDICATED_URL",
                file.database.dedicated_url.clone(),
            ),
            authenticator_password: pick(
                "REACTOR_DATABASE__AUTHENTICATOR_PASSWORD",
                file.database.authenticator_password.clone(),
                "authenticator",
            ),
            jwt_private_pem_file: opt_pick(
                "REACTOR_AUTH__JWT_PRIVATE_PEM_FILE",
                file.auth.jwt_private_pem_file.clone(),
            ),
            jwt_dir: pick("REACTOR_AUTH__JWT_DIR", None, ".data/keys"),
            storage_backend: pick(
                "REACTOR_STORAGE__BACKEND",
                file.storage.backend.clone(),
                "fs",
            ),
            storage_bucket: pick(
                "REACTOR_STORAGE__BUCKET",
                file.storage.bucket.clone(),
                "reactor",
            ),
            storage_endpoint: pick(
                "REACTOR_STORAGE__ENDPOINT",
                file.storage.endpoint.clone(),
                "",
            ),
            storage_public_endpoint: pick(
                "REACTOR_STORAGE__PUBLIC_ENDPOINT",
                file.storage.public_endpoint.clone(),
                "",
            ),
            storage_access_key: pick(
                "REACTOR_STORAGE__ACCESS_KEY",
                file.storage.access_key.clone(),
                "",
            ),
            storage_secret_key: pick(
                "REACTOR_STORAGE__SECRET_KEY",
                file.storage.secret_key.clone(),
                "",
            ),
            storage_region: pick(
                "REACTOR_STORAGE__REGION",
                file.storage.region.clone(),
                "us-east-1",
            ),
            storage_fs_root: pick(
                "REACTOR_STORAGE__FS_ROOT",
                file.storage.fs_root.clone(),
                ".data/blobs",
            ),
            storage_sign_secret: sign,
            functions_runtime: pick(
                "REACTOR_FUNCTIONS__RUNTIME",
                file.functions.runtime.clone(),
                "bun",
            ),
            functions_workdir: pick(
                "REACTOR_FUNCTIONS__WORKDIR",
                file.functions.workdir.clone(),
                ".data/functions",
            ),
            functions_bun: pick("REACTOR_FUNCTIONS__BUN", file.functions.bun.clone(), "bun"),
            functions_publisher: pick(
                "REACTOR_FUNCTIONS__PUBLISHER",
                file.functions.publisher.clone(),
                "fake",
            ),
            lambda_role_arn: pick(
                "REACTOR_FUNCTIONS__LAMBDA_ROLE_ARN",
                file.functions.lambda_role_arn.clone(),
                "arn:aws:iam::000000000000:role/reactor",
            ),
            lambda_layer_arn: pick(
                "REACTOR_FUNCTIONS__LAYER_ARN",
                file.functions.lambda_layer_arn.clone(),
                "",
            ),
            sites_workdir: pick(
                "REACTOR_SITES__WORKDIR",
                file.functions.sites_workdir.clone(),
                "/tmp/sites",
            ),
            sites_idle_secs: pick(
                "REACTOR_SITES__IDLE_SECS",
                file.functions.sites_idle_secs.map(|secs| secs.to_string()),
                "300",
            )
            .parse()
            .unwrap_or(300),
            postgrest_url: pick(
                "REACTOR_POSTGREST__URL",
                file.postgrest.url.clone(),
                "http://127.0.0.1:3000",
            ),
            postgrest_dedicated_url: pick(
                "REACTOR_POSTGREST__DEDICATED_URL",
                file.postgrest.dedicated.clone(),
                "http://127.0.0.1:3001",
            ),
            sql_dir: pick("REACTOR_SQL_DIR", None, &default_sql_dir()),
            dns_stub_file: opt_pick("REACTOR_DNS_STUB_FILE", None),
            handler: pick("REACTOR_HANDLER", None, ""),
            console_dir: pick("REACTOR_CONSOLE_DIR", None, "console/dist"),
            agent_api_key: pick("REACTOR_AGENT__API_KEY", None, ""),
            agent_base_url: pick(
                "REACTOR_AGENT__BASE_URL",
                None,
                "https://openrouter.ai/api/v1",
            ),
            agent_model: pick("REACTOR_AGENT__MODEL", None, "anthropic/claude-sonnet-4.6"),
            agent_turn_deadline_secs: pick("REACTOR_AGENT__TURN_DEADLINE_SECONDS", None, "180")
                .parse()
                .unwrap_or(180),
        }
    }
}

fn default_sql_dir() -> String {
    if Path::new("sql/control/001_init.sql").exists() {
        "sql".into()
    } else if Path::new("/app/sql/control/001_init.sql").exists() {
        "/app/sql".into()
    } else {
        "sql".into()
    }
}

fn pick(env_key: &str, file: Option<String>, default: &str) -> String {
    if let Ok(value) = std::env::var(env_key) {
        if !value.is_empty() {
            return value;
        }
    }
    file.unwrap_or_else(|| default.to_string())
}

fn place(file: &FileConfig) -> String {
    if let Some(value) = opt_pick("REACTOR_RUNTIME__PLACE", file.runtime.place.clone()) {
        return value;
    }
    if std::env::var_os("FLY_APP_NAME").is_some() {
        return "fly".into();
    }
    if std::env::var_os("AWS_LAMBDA_FUNCTION_NAME").is_some()
        || std::env::var_os("AWS_EXECUTION_ENV").is_some()
    {
        return "aws".into();
    }
    "docker".into()
}

fn opt_pick(env_key: &str, file: Option<String>) -> Option<String> {
    if let Ok(value) = std::env::var(env_key) {
        if !value.is_empty() {
            return Some(value);
        }
    }
    file.filter(|v| !v.is_empty())
}
