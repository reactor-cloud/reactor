use anyhow::Context;
use clap::Parser;
use serde::Deserialize;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Parser)]
struct Cli {
    #[arg(long)]
    url: String,
    #[arg(long)]
    key: String,
    #[arg(long)]
    zip: PathBuf,
}

#[derive(Deserialize, Default)]
struct Manifest {
    #[serde(default)]
    build: Build,
    #[serde(default)]
    site: Site,
}

#[derive(Deserialize, Default)]
struct Build {
    #[serde(default)]
    command: String,
}

#[derive(Deserialize, Default)]
struct Site {
    #[serde(default)]
    command: String,
    #[serde(default)]
    dir: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let client = reqwest::Client::new();
    let base = cli.url.trim_end_matches('/');
    let env: Vec<Value> = client
        .get(format!("{base}/sites/v1/env"))
        .bearer_auth(&cli.key)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let work = std::env::temp_dir().join(format!("reactor-build-{}", std::process::id()));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work)?;
    unzip(&cli.zip, &work)?;
    let manifest = read_manifest(&work);
    if !manifest.build.command.is_empty() {
        run_build(&work, &manifest.build.command, &env)?;
    }
    let dir = if manifest.site.dir.is_empty() {
        "site".to_string()
    } else {
        manifest.site.dir
    };
    let output = work.join(&dir);
    if !output.is_dir() {
        anyhow::bail!("output directory {dir} is missing");
    }
    let started: Value = client
        .post(format!("{base}/sites/v1/deployments"))
        .bearer_auth(&cli.key)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let id = started
        .get("id")
        .and_then(|v| v.as_str())
        .context("deployment id missing")?;
    for file in walkdir::WalkDir::new(&output)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !file.file_type().is_file() {
            continue;
        }
        let rel = file
            .path()
            .strip_prefix(&output)?
            .to_string_lossy()
            .replace('\\', "/");
        let bytes = fs::read(file.path())?;
        let res = client
            .put(format!("{base}/sites/v1/deployments/{id}/files/{rel}"))
            .bearer_auth(&cli.key)
            .body(bytes)
            .send()
            .await?;
        if !res.status().is_success() {
            let _ = client
                .post(format!("{base}/sites/v1/deployments/{id}/fail"))
                .bearer_auth(&cli.key)
                .json(&json!({ "error": format!("upload {rel} failed") }))
                .send()
                .await;
            anyhow::bail!(
                "upload {rel} failed: {}",
                res.text().await.unwrap_or_default()
            );
        }
    }
    let finished = client
        .post(format!("{base}/sites/v1/deployments/{id}/finish"))
        .bearer_auth(&cli.key)
        .json(&json!({ "command": manifest.site.command }))
        .send()
        .await?;
    if !finished.status().is_success() {
        anyhow::bail!(
            "finish failed: {}",
            finished.text().await.unwrap_or_default()
        );
    }
    let _ = fs::remove_dir_all(&work);
    Ok(())
}

fn read_manifest(work: &Path) -> Manifest {
    let path = work.join("reactor.toml");
    let Ok(text) = fs::read_to_string(path) else {
        return Manifest::default();
    };
    toml::from_str(&text).unwrap_or_default()
}

fn run_build(work: &Path, command: &str, env: &[Value]) -> anyhow::Result<()> {
    let mut child = Command::new("sh");
    child
        .arg("-c")
        .arg(command)
        .current_dir(work)
        .env_clear()
        .env(
            "PATH",
            std::env::var("PATH").unwrap_or_else(|_| "/usr/local/bin:/usr/bin:/bin".into()),
        )
        .env("HOME", "/tmp");
    for item in env {
        let Some(key) = item.get("key").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(value) = item.get("value").and_then(|v| v.as_str()) else {
            continue;
        };
        if key.is_empty() || key.starts_with("REACTOR_") || key == "PATH" || key == "HOME" {
            continue;
        }
        child.env(key, value);
    }
    let out = child.output()?;
    if !out.status.success() {
        anyhow::bail!("build failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

fn unzip(zip_path: &Path, dest: &Path) -> anyhow::Result<()> {
    let file = fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file)?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name().to_string();
        if name.contains("..") || name.starts_with('/') {
            anyhow::bail!("unsafe zip path");
        }
        let out = dest.join(&name);
        if file.is_dir() {
            fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut dest_file = fs::File::create(&out)?;
        std::io::copy(&mut file, &mut dest_file)?;
    }
    Ok(())
}
