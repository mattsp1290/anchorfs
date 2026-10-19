use anchorfs_persistence::{
    meta, migrate, pool,
    settings::{self, DurabilitySettings},
};
use anchorfs_service::{
    app::{AppState, build_router},
    config::Config,
};
use clap::{Parser, Subcommand, ValueEnum};
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
async fn serve() -> Result<(), (i32, String)> {
    let config = Config::from_env().map_err(|err| (err.code, err.message))?;
    let backend =
        anchorfs_backend::s3::S3Backend::from_config(&config.s3).map_err(|e| (2, e.to_string()))?;
    let pool = initialize_metadata(&config).await?;
    let listener = tokio::net::TcpListener::bind(config.listen_addr)
        .await
        .map_err(|_| (2, "listener bind failed".into()))?;
    eprintln!(
        "listening on {}",
        listener
            .local_addr()
            .map_err(|_| (2, "listener address unavailable".into()))?
    );
    let drain = config.shutdown_drain;
    let state = AppState {
        cursor_key: anchorfs_core::decode_hex_32(config.cursor_signing_key.expose())
            .expect("validated cursor signing key"),
        control_permits: std::sync::Arc::new(tokio::sync::Semaphore::new(
            config.db_max_connections.saturating_sub(2).max(1) as usize,
        )),
        pool: pool.clone(),
        backend: std::sync::Arc::new(backend),
        probe_cache: Default::default(),
        config: config.shared(),
    };
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
    let server = axum::serve(listener, build_router(state)).with_graceful_shutdown(async {
        let _ = stop_rx.await;
    });
    let handle = tokio::spawn(async move { server.await });
    shutdown_signal().await;
    let _ = stop_tx.send(());
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
    meta::initialize(&pool, &config.deployment_id, &config.quota_config())
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
