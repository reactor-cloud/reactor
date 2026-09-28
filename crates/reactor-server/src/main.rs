use reactor_auth::JwtIssuer;
use std::net::SocketAddr;
use std::path::PathBuf;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("jwt-init") => {
            let mut out = PathBuf::from(".data/keys");
            while let Some(flag) = args.next() {
                if flag == "--out" {
                    out = PathBuf::from(args.next().unwrap_or_else(|| ".data/keys".into()));
                }
            }
            if !out.join("private.pem").exists() {
                JwtIssuer::generate(&out)?;
            }
            println!("{}", out.display());
            Ok(())
        }
        Some("migrate") => {
            let config = reactor_server::Config::load();
            let _ = reactor_server::build_app(config).await?;
            Ok(())
        }
        _ => {
            let config = reactor_server::Config::load();
            let lambda =
                config.mode == "lambda" || std::env::var_os("AWS_LAMBDA_RUNTIME_API").is_some();
            let bind: SocketAddr = config.bind.parse()?;
            let app = reactor_server::build_app(config).await?;
            if lambda {
                reactor_server::run_lambda(app).await?;
                Ok(())
            } else {
                let listener = tokio::net::TcpListener::bind(bind).await?;
                tracing::info!("listening on {bind}");
                axum::serve(
                    listener,
                    app.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await?;
                Ok(())
            }
        }
    }
}
