use anyhow::Context;
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "reactor")]
struct Cli {
    /// Context for this command. Does not change the current context.
    #[arg(long, global = true)]
    context: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Setup {
        #[arg(long)]
        url: String,
        #[arg(long)]
        cluster: String,
        #[arg(long)]
        email: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        password: Option<String>,
    },
    Login {
        #[arg(long)]
        url: String,
        #[arg(long, conflicts_with = "email")]
        token: Option<String>,
        #[arg(long, conflicts_with = "token")]
        email: Option<String>,
        #[arg(long)]
        password: Option<String>,
    },
    Logout,
    Context {
        #[command(subcommand)]
        cmd: Option<ContextCmd>,
    },
    Link {
        #[arg(long)]
        url: String,
        #[arg(long = "ref")]
        pref: String,
        #[arg(long)]
        service_key: String,
    },
    Deploy,
    Projects {
        #[command(subcommand)]
        cmd: Option<ProjectsCmd>,
    },
    Keys {
        #[command(subcommand)]
        cmd: KeysCmd,
    },
    /// Console service keys for the signed-in operator.
    ServiceKeys {
        #[command(subcommand)]
        cmd: Option<ServiceKeysCmd>,
    },
    Logs,
    Users,
    Members {
        #[command(subcommand)]
        cmd: Option<MembersCmd>,
    },
    Cluster,
    Functions {
        #[command(subcommand)]
        cmd: Option<FunctionsCmd>,
    },
    Sites {
        #[command(subcommand)]
        cmd: Option<SitesCmd>,
    },
    Storage,
    Db {
        #[command(subcommand)]
        cmd: DbCmd,
    },
}

#[derive(Subcommand)]
enum ContextCmd {
    Use { name: String },
}

#[derive(Subcommand)]
enum ProjectsCmd {
    Create {
        name: String,
        #[arg(long)]
        link: bool,
    },
}

#[derive(Subcommand)]
enum KeysCmd {
    Rotate,
}

#[derive(Subcommand)]
enum ServiceKeysCmd {
    /// Mint a console service key. The token is printed once.
    Create {
        name: String,
        /// Scope to grant. Repeat the flag for each one.
        #[arg(
            long = "scope",
            required = true,
            value_parser = [
                "projects.create",
                "projects.migrate",
                "auth.settings",
                "auth.providers",
                "auth.email",
                "auth.users",
            ]
        )]
        scope: Vec<String>,
    },
    /// Delete a console service key.
    Revoke { id: String },
}

#[derive(Subcommand)]
enum MembersCmd {
    Add {
        #[arg(long)]
        email: String,
        #[arg(long)]
        name: String,
        #[arg(long, default_value = "developer")]
        role: String,
        #[arg(long)]
        password: Option<String>,
    },
}

#[derive(Subcommand)]
enum SitesCmd {
    Env {
        #[command(subcommand)]
        cmd: Option<EnvCmd>,
    },
}

#[derive(Subcommand)]
enum EnvCmd {
    Set {
        key: String,
        value: String,
        #[arg(long)]
        visible: bool,
    },
    Unset {
        key: String,
    },
}

#[derive(Subcommand)]
enum FunctionsCmd {
    Promote { name: String, version: i32 },
    Demote { name: String },
}

#[derive(Subcommand)]
enum DbCmd {
    Migrate {
        #[arg(long)]
        all: bool,
        #[arg(long)]
        dry_run: bool,
    },
    Tables,
    Rows {
        table: String,
    },
}

#[derive(Serialize, Deserialize, Default)]
struct Credentials {
    url: String,
    #[serde(default)]
    token: String,
    #[serde(default)]
    console_token: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
struct Session {
    #[serde(default)]
    url: String,
    #[serde(default)]
    token: String,
    #[serde(default)]
    console_token: String,
    #[serde(default)]
    email: String,
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct Store {
    #[serde(default)]
    current: String,
    #[serde(default)]
    contexts: BTreeMap<String, Session>,
}

#[derive(Clone, Copy)]
enum Scope {
    Operator,
    Project,
}

#[derive(Default)]
struct LoginUpdate {
    console_token: Option<String>,
    token: Option<String>,
    email: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct ProjectFile {
    url: String,
    #[serde(rename = "ref")]
    pref: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let context = explicit_context(cli.context);
    let context = context.as_deref();
    match cli.cmd {
        Cmd::Setup {
            url,
            cluster,
            email,
            name,
            password,
        } => setup(context, &url, &cluster, &email, &name, password).await,
        Cmd::Login {
            url,
            token,
            email,
            password,
        } => login(context, &url, token, email, password).await,
        Cmd::Logout => logout(context),
        Cmd::Context { cmd } => match cmd {
            None => list_contexts(),
            Some(ContextCmd::Use { name }) => use_context(&name),
        },
        Cmd::Link {
            url,
            pref,
            service_key,
        } => {
            write_link(&url, &pref, &service_key)?;
            Ok(())
        }
        Cmd::Deploy => deploy().await,
        Cmd::Projects { cmd } => match cmd {
            None => list_projects(context).await,
            Some(ProjectsCmd::Create { name, link }) => create_project(context, &name, link).await,
        },
        Cmd::Keys { cmd } => match cmd {
            KeysCmd::Rotate => rotate_keys(context).await,
        },
        Cmd::ServiceKeys { cmd } => match cmd {
            None => list_service_keys(context).await,
            Some(ServiceKeysCmd::Create { name, scope }) => {
                create_service_key(context, &name, &scope).await
            }
            Some(ServiceKeysCmd::Revoke { id }) => revoke_service_key(context, &id).await,
        },
        Cmd::Logs => logs(context).await,
        Cmd::Users => users(context).await,
        Cmd::Members { cmd } => match cmd {
            None => members(context).await,
            Some(MembersCmd::Add {
                email,
                name,
                role,
                password,
            }) => add_member(context, &email, &name, &role, password).await,
        },
        Cmd::Cluster => cluster(context).await,
        Cmd::Functions { cmd } => match cmd {
            None => functions(context).await,
            Some(FunctionsCmd::Promote { name, version }) => {
                promote_function(context, &name, version).await
            }
            Some(FunctionsCmd::Demote { name }) => demote_function(context, &name).await,
        },
        Cmd::Sites { cmd } => match cmd {
            None => sites(context).await,
            Some(SitesCmd::Env { cmd }) => match cmd {
                None => site_env(context).await,
                Some(EnvCmd::Set {
                    key,
                    value,
                    visible,
                }) => set_site_env(context, &key, &value, visible).await,
                Some(EnvCmd::Unset { key }) => unset_site_env(context, &key).await,
            },
        },
        Cmd::Storage => storage(context).await,
        Cmd::Db { cmd } => match cmd {
            DbCmd::Migrate { all, dry_run } => migrate(context, all, dry_run).await,
            DbCmd::Tables => tables(context).await,
            DbCmd::Rows { table } => rows(context, &table).await,
        },
    }
}

fn home() -> PathBuf {
    std::env::var("REACTOR_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let base = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(base).join(".config/reactor")
        })
}

fn explicit_context(flag: Option<String>) -> Option<String> {
    flag.filter(|value| !value.is_empty()).or_else(|| {
        std::env::var("REACTOR_CONTEXT")
            .ok()
            .filter(|value| !value.is_empty())
    })
}

fn normalize_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

fn host_of(url: &str) -> String {
    let rest = url.trim().trim_end_matches('/');
    let rest = rest.split("://").nth(1).unwrap_or(rest);
    let rest = rest.split('/').next().unwrap_or(rest);
    if let Some(stripped) = rest.strip_prefix('[') {
        return stripped.split(']').next().unwrap_or(stripped).to_string();
    }
    rest.split(':').next().unwrap_or(rest).to_string()
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if (c == '-' || c == '_' || c.is_whitespace())
            && !out.ends_with('-')
            && !out.is_empty()
        {
            out.push('-');
        }
    }
    out.trim_matches('-').to_string()
}

fn context_name_from_url(url: &str) -> String {
    let host = host_of(url).to_ascii_lowercase();
    if host == "127.0.0.1" || host == "localhost" || host == "::1" || host == "0.0.0.0" {
        return "local".into();
    }
    let name = slug(host.split('.').next().unwrap_or("cluster"));
    if name.is_empty() {
        "cluster".into()
    } else {
        name
    }
}

fn load_store(dir: &Path) -> Store {
    let path = dir.join("contexts.json");
    if path.is_file() {
        return fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
    }
    let legacy = dir.join("credentials.json");
    let Some(creds) = fs::read_to_string(&legacy)
        .ok()
        .and_then(|text| serde_json::from_str::<Credentials>(&text).ok())
    else {
        return Store::default();
    };
    if creds.url.is_empty() && creds.token.is_empty() && creds.console_token.is_empty() {
        return Store::default();
    }
    let url = normalize_url(&creds.url);
    let name = if url.is_empty() {
        "default".into()
    } else {
        context_name_from_url(&url)
    };
    let mut store = Store {
        current: name.clone(),
        ..Store::default()
    };
    store.contexts.insert(
        name,
        Session {
            url,
            token: creds.token,
            console_token: creds.console_token,
            email: String::new(),
        },
    );
    if save_store(dir, &store).is_ok() {
        let _ = fs::remove_file(legacy);
    }
    store
}

fn save_store(dir: &Path, store: &Store) -> anyhow::Result<()> {
    fs::create_dir_all(dir)?;
    fs::write(
        dir.join("contexts.json"),
        serde_json::to_string_pretty(store)?,
    )?;
    Ok(())
}

fn merge_session(session: &mut Session, url: &str, update: LoginUpdate) {
    session.url = url.to_string();
    if let Some(token) = update.console_token {
        session.console_token = token;
    }
    if let Some(token) = update.token {
        session.token = token;
    }
    if let Some(email) = update.email {
        session.email = email;
    }
}

fn apply_login(
    store: &mut Store,
    name: Option<&str>,
    url: &str,
    update: LoginUpdate,
) -> anyhow::Result<String> {
    let url = normalize_url(url);
    if url.is_empty() {
        anyhow::bail!("url missing");
    }
    if let Some(existing_name) = store
        .contexts
        .iter()
        .find(|(_, session)| normalize_url(&session.url) == url)
        .map(|(name, _)| name.clone())
    {
        if let Some(name) = name.filter(|value| !value.is_empty()) {
            if name != existing_name {
                anyhow::bail!("{url} is already context {existing_name}");
            }
        }
        merge_session(
            store.contexts.get_mut(&existing_name).unwrap(),
            &url,
            update,
        );
        store.current = existing_name.clone();
        return Ok(existing_name);
    }
    let name = match name.filter(|value| !value.is_empty()) {
        Some(name) => name.to_string(),
        None => context_name_from_url(&url),
    };
    if let Some(existing) = store.contexts.get(&name) {
        anyhow::bail!(
            "context {name} already points at {}, pass --context",
            existing.url
        );
    }
    let mut session = Session {
        url: url.clone(),
        ..Session::default()
    };
    merge_session(&mut session, &url, update);
    store.contexts.insert(name.clone(), session);
    store.current = name.clone();
    Ok(name)
}

fn select_session<'a>(
    store: &'a Store,
    explicit: Option<&str>,
    project_url: Option<&str>,
    scope: Scope,
) -> anyhow::Result<&'a Session> {
    if let Some(name) = explicit.filter(|value| !value.is_empty()) {
        return store
            .contexts
            .get(name)
            .context(format!("context {name} not found"));
    }
    match scope {
        Scope::Project => {
            let url = normalize_url(project_url.context("reactor.toml missing, run link")?);
            store
                .contexts
                .values()
                .find(|session| normalize_url(&session.url) == url)
                .context(format!("no session for {url}, run login"))
        }
        Scope::Operator => store
            .contexts
            .get(&store.current)
            .context("no current context, run login"),
    }
}

fn activate(store: &mut Store, name: &str) -> anyhow::Result<()> {
    if !store.contexts.contains_key(name) {
        anyhow::bail!("context {name} not found");
    }
    store.current = name.to_string();
    Ok(())
}

fn clear_session(store: &mut Store, explicit: Option<&str>) -> anyhow::Result<String> {
    let name = if let Some(name) = explicit.filter(|value| !value.is_empty()) {
        if !store.contexts.contains_key(name) {
            anyhow::bail!("context {name} not found");
        }
        name.to_string()
    } else if store.current.is_empty() || !store.contexts.contains_key(&store.current) {
        anyhow::bail!("no current context, run login");
    } else {
        store.current.clone()
    };
    let session = store.contexts.get_mut(&name).unwrap();
    session.token.clear();
    session.console_token.clear();
    session.email.clear();
    Ok(name)
}

fn remember(context: Option<&str>, url: &str, update: LoginUpdate) -> anyhow::Result<String> {
    let dir = home();
    let mut store = load_store(&dir);
    let name = apply_login(&mut store, context, url, update)?;
    save_store(&dir, &store)?;
    Ok(name)
}

fn list_contexts() -> anyhow::Result<()> {
    let store = load_store(&home());
    if store.contexts.is_empty() {
        println!("no contexts");
        return Ok(());
    }
    for (name, session) in &store.contexts {
        let mark = if name == &store.current { "*" } else { " " };
        println!("{mark} {name}\t{}\t{}", session.url, session.email);
    }
    Ok(())
}

fn use_context(name: &str) -> anyhow::Result<()> {
    let dir = home();
    let mut store = load_store(&dir);
    activate(&mut store, name)?;
    save_store(&dir, &store)?;
    println!("{name}");
    Ok(())
}

fn logout(context: Option<&str>) -> anyhow::Result<()> {
    let dir = home();
    let mut store = load_store(&dir);
    let name = clear_session(&mut store, context)?;
    save_store(&dir, &store)?;
    println!("{name}");
    Ok(())
}

fn project_file() -> anyhow::Result<ProjectFile> {
    let text = fs::read_to_string("reactor.toml").context("reactor.toml missing, run link")?;
    Ok(toml::from_str(&text)?)
}

fn service_key() -> anyhow::Result<String> {
    Ok(fs::read_to_string(".reactor/service_key")?
        .trim()
        .to_string())
}

fn write_link(url: &str, pref: &str, service_key: &str) -> anyhow::Result<()> {
    fs::create_dir_all(".reactor")?;
    fs::write(".reactor/service_key", service_key.trim())?;
    fs::write(
        "reactor.toml",
        toml::to_string(&ProjectFile {
            url: url.trim_end_matches('/').to_string(),
            pref: pref.to_string(),
        })?,
    )?;
    Ok(())
}

fn read_password(flag: Option<String>) -> anyhow::Result<String> {
    if let Some(value) = flag.filter(|value| !value.is_empty()) {
        return Ok(value);
    }
    if let Ok(value) = std::env::var("REACTOR_PASSWORD") {
        if !value.is_empty() {
            return Ok(value);
        }
    }
    Ok(rpassword::prompt_password("Password: ")?)
}

fn read_code() -> anyhow::Result<String> {
    if let Ok(value) = std::env::var("REACTOR_TOTP") {
        if !value.is_empty() {
            return Ok(value);
        }
    }
    Ok(rpassword::prompt_password("Authenticator code: ")?)
}

fn field(value: &Value, key: &str) -> String {
    match value.get(key) {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}

fn rows_of(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}

fn scopes_of(value: &Value) -> String {
    value
        .get("scopes")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default()
}

async fn read_body(res: reqwest::Response) -> anyhow::Result<Value> {
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("{status}: {text}");
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

async fn open_post(url: &str, path: &str, body: Value) -> anyhow::Result<Value> {
    let base = url.trim_end_matches('/');
    let res = reqwest::Client::new()
        .post(format!("{base}{path}"))
        .json(&body)
        .send()
        .await?;
    read_body(res).await
}

async fn open_bearer(url: &str, path: &str, token: &str, body: Value) -> anyhow::Result<Value> {
    let base = url.trim_end_matches('/');
    let res = reqwest::Client::new()
        .post(format!("{base}{path}"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await?;
    read_body(res).await
}

fn setup_context_name(context: Option<&str>, cluster: &str) -> Option<String> {
    if let Some(name) = context.filter(|value| !value.is_empty()) {
        return Some(name.to_string());
    }
    let name = slug(cluster);
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

async fn setup(
    context: Option<&str>,
    url: &str,
    cluster: &str,
    email: &str,
    name: &str,
    password: Option<String>,
) -> anyhow::Result<()> {
    let password = read_password(password)?;
    let body = open_post(
        url,
        "/console/v1/setup",
        json!({
            "cluster_name": cluster,
            "name": name,
            "email": email,
            "password": password,
        }),
    )
    .await?;
    let token = field(&body, "access_token");
    if token.is_empty() {
        println!("cluster {cluster} created");
        println!(
            "finish passkey and authenticator enrollment at {}/console/login",
            url.trim_end_matches('/')
        );
        return Ok(());
    }
    let named = setup_context_name(context, cluster);
    remember(
        named.as_deref(),
        url,
        LoginUpdate {
            console_token: Some(token),
            email: Some(email.to_string()),
            ..LoginUpdate::default()
        },
    )?;
    println!("cluster {cluster} ready");
    Ok(())
}

async fn login(
    context: Option<&str>,
    url: &str,
    token: Option<String>,
    email: Option<String>,
    password: Option<String>,
) -> anyhow::Result<()> {
    if let Some(token) = token {
        let name = remember(
            context,
            url,
            LoginUpdate {
                token: Some(token),
                ..LoginUpdate::default()
            },
        )?;
        println!("{name}");
        return Ok(());
    }
    let Some(email) = email else {
        anyhow::bail!("pass --token or --email");
    };
    let password = read_password(password)?;
    let body = open_post(
        url,
        "/console/v1/login",
        json!({ "email": email, "password": password }),
    )
    .await?;
    let mut token = field(&body, "access_token");
    if token.is_empty()
        && body
            .get("enrollment_required")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        anyhow::bail!(
            "finish passkey and authenticator enrollment at {}/console/login",
            url.trim_end_matches('/')
        );
    }
    if token.is_empty() {
        let mfa = field(&body, "mfa_token");
        if mfa.is_empty() {
            anyhow::bail!("login did not return a token");
        }
        let code = read_code()?;
        let done = open_bearer(url, "/console/v1/mfa/totp", &mfa, json!({ "code": code })).await?;
        token = field(&done, "access_token");
    }
    if token.is_empty() {
        anyhow::bail!("login did not return a token");
    }
    let operator = body.get("operator").cloned().unwrap_or(Value::Null);
    let who = field(&operator, "email");
    let stored = if who.is_empty() {
        email.to_string()
    } else {
        who
    };
    let name = remember(
        context,
        url,
        LoginUpdate {
            console_token: Some(token),
            email: Some(stored.clone()),
            ..LoginUpdate::default()
        },
    )?;
    println!("logged in as {stored} on {name}");
    Ok(())
}

async fn console(
    context: Option<&str>,
    scope: Scope,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> anyhow::Result<Value> {
    let store = load_store(&home());
    let project_url = match scope {
        Scope::Project => Some(project_file()?.url),
        Scope::Operator => None,
    };
    let session = select_session(&store, context, project_url.as_deref(), scope)?;
    if session.console_token.is_empty() {
        anyhow::bail!("console session missing, run login --email");
    }
    let url = session.url.clone();
    let token = session.console_token.clone();
    let method = reqwest::Method::from_bytes(method.as_bytes())?;
    let mut req = reqwest::Client::new()
        .request(method, format!("{}{path}", url.trim_end_matches('/')))
        .bearer_auth(&token);
    if let Some(body) = body {
        req = req.json(&body);
    }
    read_body(req.send().await?).await
}

fn pref() -> anyhow::Result<String> {
    Ok(project_file()?.pref)
}

async fn list_projects(context: Option<&str>) -> anyhow::Result<()> {
    let body = console(
        context,
        Scope::Operator,
        "GET",
        "/console/v1/projects",
        None,
    )
    .await?;
    for project in rows_of(&body) {
        println!(
            "{}\t{}\t{}",
            field(project, "ref"),
            field(project, "name"),
            field(project, "role")
        );
    }
    Ok(())
}

async fn create_project(context: Option<&str>, name: &str, link: bool) -> anyhow::Result<()> {
    let body = console(
        context,
        Scope::Operator,
        "POST",
        "/console/v1/projects",
        Some(json!({ "name": name })),
    )
    .await?;
    let pref = field(&body, "ref");
    let service = field(&body, "service_key");
    if link {
        let store = load_store(&home());
        let session = select_session(&store, context, None, Scope::Operator)?;
        write_link(&session.url, &pref, &service)?;
    }
    println!("ref\t{pref}");
    println!("name\t{}", field(&body, "name"));
    println!("anon\t{}", field(&body, "anon_key"));
    println!("service\t{service}");
    Ok(())
}

async fn rotate_keys(context: Option<&str>) -> anyhow::Result<()> {
    let pref = pref()?;
    let body = console(
        context,
        Scope::Project,
        "POST",
        &format!("/console/v1/projects/{pref}/keys"),
        Some(json!({})),
    )
    .await?;
    let service = field(&body, "service_key");
    if Path::new(".reactor/service_key").is_file() {
        fs::write(".reactor/service_key", &service)?;
    }
    println!("anon\t{}", field(&body, "anon_key"));
    println!("service\t{service}");
    Ok(())
}

async fn list_service_keys(context: Option<&str>) -> anyhow::Result<()> {
    let body = console(context, Scope::Operator, "GET", "/console/v1/keys", None).await?;
    for key in rows_of(&body) {
        println!(
            "{}\t{}\t{}\t{}",
            field(key, "id"),
            field(key, "name"),
            scopes_of(key),
            field(key, "created_at")
        );
    }
    Ok(())
}

async fn create_service_key(
    context: Option<&str>,
    name: &str,
    scopes: &[String],
) -> anyhow::Result<()> {
    let body = console(
        context,
        Scope::Operator,
        "POST",
        "/console/v1/keys",
        Some(json!({ "name": name, "scopes": scopes })),
    )
    .await?;
    println!("id\t{}", field(&body, "id"));
    println!("name\t{}", field(&body, "name"));
    println!("scopes\t{}", scopes_of(&body));
    println!("token\t{}", field(&body, "token"));
    Ok(())
}

async fn revoke_service_key(context: Option<&str>, id: &str) -> anyhow::Result<()> {
    console(
        context,
        Scope::Operator,
        "DELETE",
        &format!("/console/v1/keys/{id}"),
        None,
    )
    .await?;
    println!("revoked\t{id}");
    Ok(())
}

async fn logs(context: Option<&str>) -> anyhow::Result<()> {
    let pref = pref()?;
    let body = console(
        context,
        Scope::Project,
        "GET",
        &format!("/console/v1/projects/{pref}/logs"),
        None,
    )
    .await?;
    for row in rows_of(&body) {
        println!(
            "{}\t{}\t{}\t{}\t{}",
            field(row, "created_at"),
            field(row, "kind"),
            field(row, "name"),
            field(row, "status"),
            field(row, "message")
        );
    }
    Ok(())
}

async fn users(context: Option<&str>) -> anyhow::Result<()> {
    let pref = pref()?;
    let body = console(
        context,
        Scope::Project,
        "GET",
        &format!("/console/v1/projects/{pref}/users"),
        None,
    )
    .await?;
    for row in rows_of(&body) {
        println!(
            "{}\t{}\t{}",
            field(row, "email"),
            field(row, "id"),
            field(row, "created_at")
        );
    }
    Ok(())
}

async fn members(context: Option<&str>) -> anyhow::Result<()> {
    let pref = pref()?;
    let body = console(
        context,
        Scope::Project,
        "GET",
        &format!("/console/v1/projects/{pref}/members"),
        None,
    )
    .await?;
    for row in rows_of(&body) {
        println!(
            "{}\t{}\t{}",
            field(row, "email"),
            field(row, "name"),
            field(row, "role")
        );
    }
    Ok(())
}

async fn add_member(
    context: Option<&str>,
    email: &str,
    name: &str,
    role: &str,
    password: Option<String>,
) -> anyhow::Result<()> {
    let pref = pref()?;
    let password = read_password(password)?;
    console(
        context,
        Scope::Project,
        "POST",
        &format!("/console/v1/projects/{pref}/members"),
        Some(json!({ "email": email, "name": name, "role": role, "password": password })),
    )
    .await?;
    println!("{email}\t{role}");
    Ok(())
}

async fn cluster(context: Option<&str>) -> anyhow::Result<()> {
    let body = console(context, Scope::Operator, "GET", "/console/v1/cluster", None).await?;
    if let Some(name) = body.get("name").and_then(|v| v.as_str()) {
        println!("name\t{name}");
    }
    if let Some(place) = body.get("place").and_then(|v| v.as_str()) {
        println!("place\t{place}");
    }
    if let Some(kind) = body.get("type").and_then(|v| v.as_str()) {
        println!("type\t{kind}");
    }
    println!("reactor\t{}", up(body.get("reactor")));
    let pg = body.get("postgres");
    println!(
        "postgres\t{}\t{} connections\t{} bytes",
        up(pg),
        pg.and_then(|v| v.get("connections"))
            .map(value_text)
            .unwrap_or_default(),
        pg.and_then(|v| v.get("bytes"))
            .map(value_text)
            .unwrap_or_default()
    );
    println!("postgrest\t{}", up(body.get("postgrest")));
    if let Some(dedicated) = body.get("postgrest_dedicated").filter(|v| !v.is_null()) {
        let state = if dedicated.get("active").and_then(|v| v.as_bool()) == Some(false) {
            "inactive"
        } else {
            up(Some(dedicated))
        };
        println!("postgrest_dedicated\t{state}");
    }
    let blobs = body.get("blobs");
    println!(
        "blobs\t{}\t{}",
        up(blobs),
        blobs
            .and_then(|v| v.get("backend"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
    );
    Ok(())
}

fn up(part: Option<&Value>) -> &'static str {
    if part
        .and_then(|v| v.get("ok"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        "up"
    } else {
        "down"
    }
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Number(n) => n.to_string(),
        other => other.to_string(),
    }
}

async fn functions(context: Option<&str>) -> anyhow::Result<()> {
    let pref = pref()?;
    let body = console(
        context,
        Scope::Project,
        "GET",
        &format!("/console/v1/projects/{pref}/functions"),
        None,
    )
    .await?;
    for row in rows_of(&body) {
        let mark = if field(row, "pinned") == "true" {
            "pinned"
        } else if field(row, "live") == "true" {
            "live"
        } else {
            ""
        };
        println!(
            "{}\t{}\t{mark}\t{}",
            field(row, "name"),
            field(row, "version"),
            field(row, "created_at")
        );
    }
    Ok(())
}

async fn promote_function(context: Option<&str>, name: &str, version: i32) -> anyhow::Result<()> {
    let pref = pref()?;
    let body = console(
        context,
        Scope::Project,
        "POST",
        &format!("/console/v1/projects/{pref}/functions/{name}/promote"),
        Some(json!({ "version": version })),
    )
    .await?;
    println!("{}", serde_json::to_string(&body)?);
    Ok(())
}

async fn demote_function(context: Option<&str>, name: &str) -> anyhow::Result<()> {
    let pref = pref()?;
    let body = console(
        context,
        Scope::Project,
        "POST",
        &format!("/console/v1/projects/{pref}/functions/{name}/demote"),
        None,
    )
    .await?;
    println!("{}", serde_json::to_string(&body)?);
    Ok(())
}

async fn site_env(context: Option<&str>) -> anyhow::Result<()> {
    let pref = pref()?;
    let body = console(
        context,
        Scope::Project,
        "GET",
        &format!("/console/v1/projects/{pref}/site/env"),
        None,
    )
    .await?;
    for row in rows_of(&body) {
        if field(row, "visibility") == "visible" && !field(row, "value").is_empty() {
            println!("{}\tvisible\t{}", field(row, "key"), field(row, "value"));
        } else {
            println!("{}\t{}", field(row, "key"), field(row, "visibility"));
        }
    }
    Ok(())
}

async fn set_site_env(
    context: Option<&str>,
    key: &str,
    value: &str,
    visible: bool,
) -> anyhow::Result<()> {
    let pref = pref()?;
    let body = console(
        context,
        Scope::Project,
        "POST",
        &format!("/console/v1/projects/{pref}/site/env"),
        Some(json!({
            "key": key,
            "value": value,
            "visibility": if visible { "visible" } else { "secret" },
        })),
    )
    .await?;
    println!("{}", serde_json::to_string(&body)?);
    Ok(())
}

async fn unset_site_env(context: Option<&str>, key: &str) -> anyhow::Result<()> {
    let pref = pref()?;
    console(
        context,
        Scope::Project,
        "DELETE",
        &format!("/console/v1/projects/{pref}/site/env/{key}"),
        None,
    )
    .await?;
    Ok(())
}

async fn sites(context: Option<&str>) -> anyhow::Result<()> {
    let pref = pref()?;
    let body = console(
        context,
        Scope::Project,
        "GET",
        &format!("/console/v1/projects/{pref}/sites"),
        None,
    )
    .await?;
    println!("{}", field(&body, "url"));
    for file in body
        .get("files")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
    {
        if let Some(path) = file.as_str() {
            println!("{path}");
        }
    }
    Ok(())
}

async fn storage(context: Option<&str>) -> anyhow::Result<()> {
    let pref = pref()?;
    let body = console(
        context,
        Scope::Project,
        "GET",
        &format!("/console/v1/projects/{pref}/objects"),
        None,
    )
    .await?;
    for key in rows_of(&body) {
        if let Some(key) = key.as_str() {
            println!("{key}");
        } else if !field(key, "key").is_empty() {
            println!("{}\t{}", field(key, "key"), field(key, "size"));
        }
    }
    Ok(())
}

async fn tables(context: Option<&str>) -> anyhow::Result<()> {
    let pref = pref()?;
    let body = console(
        context,
        Scope::Project,
        "GET",
        &format!("/console/v1/projects/{pref}/schema"),
        None,
    )
    .await?;
    for table in rows_of(&body) {
        let columns = table
            .get("columns")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .map(|column| format!("{} {}", field(column, "name"), field(column, "type")))
            .collect::<Vec<_>>()
            .join(", ");
        println!("{}\t{columns}", field(table, "name"));
    }
    Ok(())
}

async fn rows(context: Option<&str>, table: &str) -> anyhow::Result<()> {
    let pref = pref()?;
    let body = console(
        context,
        Scope::Project,
        "GET",
        &format!("/console/v1/projects/{pref}/tables/{table}"),
        None,
    )
    .await?;
    let list = body.get("rows").unwrap_or(&body);
    for row in rows_of(list) {
        println!("{}", serde_json::to_string(row)?);
    }
    Ok(())
}

async fn deploy() -> anyhow::Result<()> {
    let project = project_file()?;
    let key = service_key()?;
    let client = reqwest::Client::new();
    let migrate = client
        .post(format!(
            "{}/platform/v1/migrate",
            project.url.trim_end_matches('/')
        ))
        .bearer_auth(&key)
        .json(&json!({"all": false}))
        .send()
        .await?;
    if !migrate.status().is_success() {
        anyhow::bail!(
            "migrate failed: {}",
            migrate.text().await.unwrap_or_default()
        );
    }
    let functions = Path::new("functions");
    if functions.is_dir() {
        for entry in fs::read_dir(functions)? {
            let entry = entry?;
            if !entry.path().is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            let zip = zip_dir(&entry.path())?;
            let res = client
                .post(format!(
                    "{}/fn/v1/_admin/functions/{name}",
                    project.url.trim_end_matches('/')
                ))
                .bearer_auth(&key)
                .header("content-type", "application/zip")
                .body(zip)
                .send()
                .await?;
            if !res.status().is_success() {
                anyhow::bail!(
                    "function {name} failed: {}",
                    res.text().await.unwrap_or_default()
                );
            }
        }
    }
    let site = Path::new("site");
    if site.is_dir() {
        let started = client
            .post(format!(
                "{}/sites/v1/deployments",
                project.url.trim_end_matches('/')
            ))
            .bearer_auth(&key)
            .send()
            .await?;
        if !started.status().is_success() {
            anyhow::bail!(
                "site deploy failed: {}",
                started.text().await.unwrap_or_default()
            );
        }
        let started: Value = started.json().await?;
        let id = started
            .get("id")
            .and_then(|v| v.as_str())
            .context("deployment id missing")?;
        for file in walkdir::WalkDir::new(site)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !file.file_type().is_file() {
                continue;
            }
            let rel = file
                .path()
                .strip_prefix(site)?
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = fs::read(file.path())?;
            let res = client
                .put(format!(
                    "{}/sites/v1/deployments/{id}/files/{rel}",
                    project.url.trim_end_matches('/')
                ))
                .bearer_auth(&key)
                .body(bytes)
                .send()
                .await?;
            if !res.status().is_success() {
                let message = format!("upload {rel} failed");
                let _ = client
                    .post(format!(
                        "{}/sites/v1/deployments/{id}/fail",
                        project.url.trim_end_matches('/')
                    ))
                    .bearer_auth(&key)
                    .json(&json!({ "error": message }))
                    .send()
                    .await;
                anyhow::bail!(
                    "site {rel} failed: {}",
                    res.text().await.unwrap_or_default()
                );
            }
        }
        let finished = client
            .post(format!(
                "{}/sites/v1/deployments/{id}/finish",
                project.url.trim_end_matches('/')
            ))
            .bearer_auth(&key)
            .send()
            .await?;
        if !finished.status().is_success() {
            anyhow::bail!(
                "site finish failed: {}",
                finished.text().await.unwrap_or_default()
            );
        }
    }
    Ok(())
}

fn zip_dir(dir: &Path) -> anyhow::Result<Vec<u8>> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let opts = zip::write::SimpleFileOptions::default();
        for file in walkdir::WalkDir::new(dir)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !file.file_type().is_file() {
                continue;
            }
            let rel = file
                .path()
                .strip_prefix(dir)?
                .to_string_lossy()
                .replace('\\', "/");
            writer.start_file(rel, opts)?;
            writer.write_all(&fs::read(file.path())?)?;
        }
        writer.finish()?;
    }
    Ok(cursor.into_inner())
}

async fn migrate(context: Option<&str>, all: bool, dry_run: bool) -> anyhow::Result<()> {
    let store = load_store(&home());
    let project_url = project_file().ok().map(|project| project.url);
    let scope = if project_url.is_some() {
        Scope::Project
    } else {
        Scope::Operator
    };
    let session = select_session(&store, context, project_url.as_deref(), scope)?;
    if session.token.is_empty() {
        anyhow::bail!("operator token missing, run login --token");
    }
    let client = reqwest::Client::new();
    let base = session.url.trim_end_matches('/');
    let token = session.token.clone();
    if dry_run {
        let res = client
            .get(format!("{base}/platform/v1/projects"))
            .bearer_auth(&token)
            .send()
            .await?;
        let refs: Vec<String> = res.json().await?;
        for pref in refs {
            println!("{pref}");
        }
        return Ok(());
    }
    if !all {
        anyhow::bail!("pass --all");
    }
    let res = client
        .post(format!("{base}/platform/v1/migrate"))
        .bearer_auth(&token)
        .json(&json!({"all": true}))
        .send()
        .await?;
    if !res.status().is_success() {
        anyhow::bail!("migrate failed: {}", res.text().await.unwrap_or_default());
    }
    Ok(())
}

#[cfg(test)]
mod context_tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn scratch(name: &str) -> PathBuf {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("reactor-ctx-{name}-{n}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn names_local_and_first_label() {
        assert_eq!(context_name_from_url("http://127.0.0.1:18000"), "local");
        assert_eq!(context_name_from_url("http://localhost:18000/"), "local");
        assert_eq!(context_name_from_url("https://fly1.reactor.cloud"), "fly1");
    }

    #[test]
    fn login_keeps_each_cluster_and_switches_current() {
        let dir = scratch("login");
        let mut store = Store::default();
        apply_login(
            &mut store,
            None,
            "http://127.0.0.1:18000",
            LoginUpdate {
                console_token: Some("local-console".into()),
                ..LoginUpdate::default()
            },
        )
        .unwrap();
        apply_login(
            &mut store,
            Some("fly"),
            "https://fly1.reactor.cloud/",
            LoginUpdate {
                console_token: Some("fly-console".into()),
                email: Some("ada@example.com".into()),
                ..LoginUpdate::default()
            },
        )
        .unwrap();
        save_store(&dir, &store).unwrap();
        let store = load_store(&dir);
        assert_eq!(store.current, "fly");
        assert_eq!(store.contexts["local"].console_token, "local-console");
        assert_eq!(store.contexts["local"].url, "http://127.0.0.1:18000");
        assert_eq!(store.contexts["fly"].email, "ada@example.com");
        assert_eq!(store.contexts["fly"].url, "https://fly1.reactor.cloud");
    }

    #[test]
    fn second_login_merges_tokens_on_the_same_url() {
        let mut store = Store::default();
        apply_login(
            &mut store,
            None,
            "http://127.0.0.1:18000",
            LoginUpdate {
                token: Some("operator".into()),
                ..LoginUpdate::default()
            },
        )
        .unwrap();
        apply_login(
            &mut store,
            None,
            "http://127.0.0.1:18000/",
            LoginUpdate {
                console_token: Some("console".into()),
                email: Some("ada@example.com".into()),
                ..LoginUpdate::default()
            },
        )
        .unwrap();
        let session = &store.contexts["local"];
        assert_eq!(session.token, "operator");
        assert_eq!(session.console_token, "console");
        assert_eq!(store.contexts.len(), 1);
    }

    #[test]
    fn derived_name_collision_asks_for_context() {
        let mut store = Store::default();
        apply_login(
            &mut store,
            Some("fly1"),
            "https://fly1.reactor.cloud",
            LoginUpdate::default(),
        )
        .unwrap();
        let err = apply_login(
            &mut store,
            None,
            "https://fly1.other.example",
            LoginUpdate::default(),
        )
        .unwrap_err();
        assert!(err.to_string().contains("--context"), "{err}");
    }

    #[test]
    fn explicit_context_does_not_follow_the_project_url() {
        let mut store = Store::default();
        apply_login(
            &mut store,
            Some("local"),
            "http://127.0.0.1:18000",
            LoginUpdate::default(),
        )
        .unwrap();
        apply_login(
            &mut store,
            Some("fly"),
            "https://fly1.reactor.cloud",
            LoginUpdate::default(),
        )
        .unwrap();
        let session = select_session(
            &store,
            Some("local"),
            Some("https://fly1.reactor.cloud"),
            Scope::Project,
        )
        .unwrap();
        assert_eq!(session.url, "http://127.0.0.1:18000");
        assert_eq!(store.current, "fly");
    }

    #[test]
    fn project_scope_uses_the_checkout_url() {
        let mut store = Store::default();
        apply_login(
            &mut store,
            Some("local"),
            "http://127.0.0.1:18000",
            LoginUpdate::default(),
        )
        .unwrap();
        apply_login(
            &mut store,
            Some("fly"),
            "https://fly1.reactor.cloud",
            LoginUpdate::default(),
        )
        .unwrap();
        let session =
            select_session(&store, None, Some("http://127.0.0.1:18000"), Scope::Project).unwrap();
        assert_eq!(session.url, "http://127.0.0.1:18000");
        let err =
            select_session(&store, None, Some("https://aws.example"), Scope::Project).unwrap_err();
        assert!(
            err.to_string()
                .contains("no session for https://aws.example"),
            "{err}"
        );
    }

    #[test]
    fn operator_scope_uses_current() {
        let mut store = Store::default();
        apply_login(
            &mut store,
            Some("local"),
            "http://127.0.0.1:18000",
            LoginUpdate::default(),
        )
        .unwrap();
        apply_login(
            &mut store,
            Some("fly"),
            "https://fly1.reactor.cloud",
            LoginUpdate::default(),
        )
        .unwrap();
        let session = select_session(&store, None, None, Scope::Operator).unwrap();
        assert_eq!(session.url, "https://fly1.reactor.cloud");
    }

    #[test]
    fn legacy_credentials_become_one_context() {
        let dir = scratch("legacy");
        fs::write(
            dir.join("credentials.json"),
            r#"{"url":"http://127.0.0.1:18000","token":"op","console_token":"con"}"#,
        )
        .unwrap();
        let store = load_store(&dir);
        assert_eq!(store.current, "local");
        assert_eq!(store.contexts["local"].token, "op");
        assert_eq!(store.contexts["local"].console_token, "con");
        assert!(!dir.join("credentials.json").exists());
        assert!(dir.join("contexts.json").is_file());
    }

    #[test]
    fn logout_clears_tokens_and_use_switches_current() {
        let mut store = Store::default();
        apply_login(
            &mut store,
            Some("local"),
            "http://127.0.0.1:18000",
            LoginUpdate {
                token: Some("op".into()),
                console_token: Some("con".into()),
                email: Some("ada@example.com".into()),
            },
        )
        .unwrap();
        apply_login(
            &mut store,
            Some("fly"),
            "https://fly1.reactor.cloud",
            LoginUpdate::default(),
        )
        .unwrap();
        activate(&mut store, "local").unwrap();
        let name = clear_session(&mut store, None).unwrap();
        assert_eq!(name, "local");
        assert!(store.contexts["local"].console_token.is_empty());
        assert!(store.contexts["local"].token.is_empty());
        assert!(store.contexts["local"].email.is_empty());
        assert_eq!(store.contexts["local"].url, "http://127.0.0.1:18000");
        assert_eq!(store.current, "local");
    }
}
