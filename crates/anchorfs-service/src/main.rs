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
