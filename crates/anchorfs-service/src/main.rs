use anchorfs_persistence::{
    meta, migrate, pool,
    settings::{self, DurabilitySettings},
};
use anchorfs_service::{
    app::{AppState, build_router},
    config::Config,
};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    Serve,
    BootstrapAdmin {
        #[arg(long)]
        uid: u32,
        #[arg(long)]
        gid: u32,
        #[arg(long, default_value_t = 4194304)]
        max_reservation_bytes: u64,
        #[arg(long)]
        expires_at: Option<String>,
    },
    Openapi {
        #[arg(long, value_enum, default_value = "yaml")]
        format: Format,
    },
    /// One unauthenticated `GET /readyz` against a service origin; never retries.
    /// Exit 0: ready and every requirement met. 1: not ready or a requirement failed.
    /// 2: invalid arguments, unreadable CA, or transport failure before any HTTP response.
    Probe {
        /// Service origin such as `https://host:8080` (no path, query or fragment).
        #[arg(long)]
        url: String,
        #[arg(long)]
        ca_bundle: Option<PathBuf>,
        #[arg(long)]
        require_tls: bool,
        #[arg(long)]
        require_durable: bool,
        #[arg(long, default_value_t = 5000)]
        timeout_ms: u64,
        #[arg(long)]
        allow_plaintext: bool,
    },
}
#[derive(Clone, ValueEnum)]
enum Format {
    Yaml,
    Json,
}
#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    let result = match Cli::parse().command.unwrap_or(Command::Serve) {
        Command::Openapi { format } => {
            let doc = anchorfs_service::api_doc::document();
            match format {
                Format::Yaml => print!("{}", doc.to_yaml().expect("OpenAPI serialization")),
                Format::Json => println!("{}", doc.to_json().expect("OpenAPI serialization")),
            };
            Ok(())
        }
        Command::Serve => serve().await,
        Command::Probe {
            url,
            ca_bundle,
            require_tls,
            require_durable,
            timeout_ms,
            allow_plaintext,
        } => {
            probe(
                &url,
                ca_bundle,
                require_tls,
                require_durable,
                timeout_ms,
                allow_plaintext,
            )
            .await
        }
        Command::BootstrapAdmin {
            uid,
            gid,
            max_reservation_bytes,
            expires_at,
        } => bootstrap_admin(uid, gid, max_reservation_bytes, expires_at).await,
    };
    if let Err((code, message)) = result {
        eprintln!("{message}");
        std::process::exit(code)
    }
}
async fn probe(
    url: &str,
    ca_bundle: Option<PathBuf>,
    require_tls: bool,
    require_durable: bool,
    timeout_ms: u64,
    allow_plaintext: bool,
) -> Result<(), (i32, String)> {
    use anchorfs_client::{endpoint::Endpoint, error::ClientError};
    if timeout_ms == 0 {
        return Err((2, "timeout must be positive".into()));
    }
    let endpoint = Endpoint::parse(url, allow_plaintext).map_err(|e| (2, e.to_string()))?;
    let ca = ca_bundle
        .map(|path| std::fs::read(path).map_err(|_| (2, "CA bundle unreadable".to_string())))
        .transpose()?;
    let report = match anchorfs_client::readiness::readiness(
        &endpoint,
        ca.as_deref(),
        std::time::Duration::from_millis(timeout_ms),
    )
    .await
    {
        Ok(report) => report,
        // A 200 whose body is not a readiness document is an HTTP answer, not a transport failure.
        Err(ClientError::Decode(why)) => return Err((1, format!("not ready: {why}"))),
        Err(error) => return Err((2, error.to_string())),
    };
    let Some(ready) = report.ready else {
        let code = report.error.map(|e| e.code);
        println!(
            "{}",
            serde_json::json!({"status":report.status,"code":code})
        );
        return Err((
            1,
            format!(
                "not ready: status {} code {}",
                report.status,
                code.as_deref().unwrap_or("none")
            ),
        ));
    };
    println!(
        "{}",
        serde_json::json!({"status":report.status,"tls":ready.tls,"durability_profile":ready.durability_profile,"database":ready.database,"backend":ready.backend})
    );
    if require_tls && !ready.tls {
        return Err((1, "requirement failed: tls".into()));
    }
    if require_durable && ready.durability_profile != "durable" {
        return Err((1, "requirement failed: durable profile".into()));
    }
    Ok(())
}
async fn serve() -> Result<(), (i32, String)> {
    let config = Config::from_env().map_err(|err| (err.code, err.message))?;
    let backend =
        anchorfs_backend::s3::S3Backend::from_config(&config.s3).map_err(|e| (2, e.to_string()))?;
    let pool = initialize_metadata(&config).await?;
    let listener = std::net::TcpListener::bind(config.listen_addr)
        .map_err(|_| (2, "listener bind failed".into()))?;
    listener
        .set_nonblocking(true)
        .map_err(|_| (2, "listener bind failed".into()))?;
    eprintln!(
        "listening on {}://{}",
        if config.tls.is_some() {
            "https"
        } else {
            "http"
        },
        listener
            .local_addr()
            .map_err(|_| (2, "listener address unavailable".into()))?
    );
    let drain = config.shutdown_drain;
    let tls = config.tls.clone();
    let state = AppState::production(config, pool.clone(), std::sync::Arc::new(backend));
    let (server_handle, server) = anchorfs_service::tls::serve(
        listener,
        build_router(state),
        tls,
        std::time::Duration::from_secs(5).min(drain / 2),
    )
    .map_err(|_| (2, "listener bind failed".into()))?;
    let handle = tokio::spawn(server);
    shutdown_signal().await;
    server_handle.graceful_shutdown(None);
    let mut handle = handle;
    match tokio::time::timeout(drain, &mut handle).await {
        Ok(Ok(Ok(()))) => {}
        Ok(_) => return Err((2, "HTTP server failed".into())),
        Err(_) => {
            handle.abort();
            return Err((2, "shutdown drain deadline exceeded".into()));
        }
    }
    pool.close().await;
    Ok(())
}
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("SIGTERM handler");
        tokio::select! {_=tokio::signal::ctrl_c()=>{},_=terminate.recv()=>{}}
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

async fn initialize_metadata(config: &Config) -> Result<pool::PgPool, (i32, String)> {
    let pool = pool::connect(&config.pool_config())
        .await
        .map_err(|_| (2, "database connection failed".into()))?;
    let encoding = settings::read_server_encoding(&pool)
        .await
        .map_err(|_| (2, "database encoding check failed".into()))?;
    settings::check_encoding(&encoding).map_err(|err| (2, err.to_string()))?;
    let settings = DurabilitySettings::read(&pool)
        .await
        .map_err(|_| (3, "database settings check failed".into()))?;
    settings
        .check(config.durability_profile)
        .map_err(|err| (3, err.to_string()))?;
    for warning in settings.unsafe_settings() {
        tracing::warn!(setting=%warning,"unsafe development database setting");
    }
    if migrate::database_is_newer(&pool)
        .await
        .map_err(|_| (4, "migration version check failed".into()))?
    {
        return Err((4, "database schema is newer than this binary".into()));
    }
    migrate::apply(&pool)
        .await
        .map_err(|_| (2, "database migration failed".into()))?;
    meta::initialize(
        &pool,
        &config.deployment_id,
        &config.quota_config(),
        config.chunk_size,
    )
    .await
    .map_err(|err| (6, err.to_string()))?;
    let versions = meta::read_versions(&pool)
        .await
        .map_err(|_| (5, "schema versions unavailable".into()))?;
    meta::check_versions(&versions).map_err(|err| (5, err.to_string()))?;
    Ok(pool)
}
async fn bootstrap_admin(
    uid: u32,
    gid: u32,
    max_reservation_bytes: u64,
    expires_at: Option<String>,
) -> Result<(), (i32, String)> {
    let config = Config::bootstrap_from_env().map_err(|error| (error.code, error.message))?;
    let allowance = i64::try_from(max_reservation_bytes)
        .ok()
        .filter(|v| *v > 0)
        .ok_or((2, "invalid max reservation bytes".into()))?;
    let expires_at = expires_at
        .as_deref()
        .map(|s| time::OffsetDateTime::parse(s, &time::format_description::well_known::Rfc3339))
        .transpose()
        .map_err(|_| (2, "invalid expiry".into()))?;
    let pool = initialize_metadata(&config).await?;
    let issued =
        anchorfs_service::bootstrap::create_administrator(&pool, uid, gid, allowance, expires_at)
            .await
            .map_err(|error| {
                use anchorfs_persistence::credentials::BootstrapError;
                match error {
                    BootstrapError::AdministratorExists => {
                        (8, "administrator already exists".into())
                    }
                    BootstrapError::InvalidInput => (2, "invalid administrator parameters".into()),
                    BootstrapError::Database(_) => (2, "administrator bootstrap failed".into()),
                }
            })?;
    println!(
        "{}",
        serde_json::json!({"credential_id":issued.id.0,"secret":issued.secret.expose(),"credential_epoch":issued.epoch})
    );
    pool.close().await;
    Ok(())
}
