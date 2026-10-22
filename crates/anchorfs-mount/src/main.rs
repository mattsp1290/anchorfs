use anchorfs_mount::{
    config::{LogFormat, MountArgs},
    profile::MountProfile,
};
use clap::{Parser, Subcommand};
use std::process::ExitCode;
#[derive(Parser)]
#[command(
    version,
    about = "Mount an AnchorFS workspace using the strict Linux FUSE profile",
    args_conflicts_with_subcommands = true
)]
struct Cli {
    #[arg(long)]
    print_profile: bool,
    #[command(subcommand)]
    command: Option<Command>,
}
#[derive(Subcommand)]
enum Command {
    Mount(Box<MountArgs>),
}
fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.print_profile {
        println!(
            "{}",
            serde_json::to_string(&MountProfile::strict()).expect("profile serializes")
        );
        return ExitCode::SUCCESS;
    }
    let Some(Command::Mount(args)) = cli.command else {
        eprintln!("use mount or --print-profile");
        return ExitCode::from(2);
    };
    let cfg = match args.config() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    // Dependency debug events can contain kernel-supplied names. JSON logs
    // stay at info; local text logs may opt into RUST_LOG debug.
    let filter = match cfg.log_format() {
        LogFormat::Json => tracing_subscriber::EnvFilter::new("info"),
        LogFormat::Text => {
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())
        }
    };
    let format = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr);
    match cfg.log_format() {
        LogFormat::Json => format.json().init(),
        LogFormat::Text => format.init(),
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = cfg;
        eprintln!("mounting is supported only on Linux");
        ExitCode::from(2)
    }
    #[cfg(target_os = "linux")]
    {
        linux(cfg)
    }
}
#[cfg(target_os = "linux")]
fn linux(cfg: anchorfs_mount::config::MountConfig) -> ExitCode {
    use anchorfs_mount::{platform, runtime::Mount};
    use tokio::signal::unix::{SignalKind, signal};
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(cfg.fuse_threads())
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("runtime: {e}");
            return ExitCode::from(4);
        }
    };
    let signals = rt.block_on(async {
        Ok::<_, std::io::Error>((
            signal(SignalKind::terminate())?,
            signal(SignalKind::interrupt())?,
        ))
    });
    let (term, interrupt) = match signals {
        Ok(s) => s,
        Err(e) => {
            eprintln!("signals: {e}");
            return ExitCode::from(5);
        }
    };
    let mount = match rt.block_on(Mount::start(cfg, rt.handle().clone())) {
        Ok(mount) => mount,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(e.exit_code());
        }
    };
    let session = match platform::spawn(mount.clone()) {
        Ok(session) => session,
        Err(e) => {
            eprintln!("{e}");
            rt.block_on(mount.shutdown());
            return ExitCode::from(e.exit_code());
        }
    };
    rt.block_on(serve(mount, session, term, interrupt))
}

#[cfg(target_os = "linux")]
async fn next_signal(
    term: &mut tokio::signal::unix::Signal,
    interrupt: &mut tokio::signal::unix::Signal,
) {
    tokio::select! { _ = term.recv() => {}, _ = interrupt.recv() => {} }
}

#[cfg(target_os = "linux")]
async fn serve(
    mount: std::sync::Arc<anchorfs_mount::runtime::Mount>,
    session: fuser::BackgroundSession,
    mut term: tokio::signal::unix::Signal,
    mut interrupt: tokio::signal::unix::Signal,
) -> ExitCode {
    // join waits without unmounting. A refusal by our non-lazy helper therefore
    // leaves the FUSE thread serving and every tool handle intact.
    let mut joined = tokio::task::spawn_blocking(move || session.join());
    loop {
        tokio::select! {
            result = &mut joined => return finished(&mount, result).await,
            _ = next_signal(&mut term, &mut interrupt) => {}
        }
        let mut helper = tokio::process::Command::new("fusermount3");
        helper
            .arg("-u")
            .arg(mount.config().mountpoint())
            .kill_on_drop(true);
        let status = tokio::select! {
            status = helper.status() => status,
            result = &mut joined => return finished(&mount, result).await,
            _ = next_signal(&mut term, &mut interrupt) => std::process::exit(5),
        };
        if !status.is_ok_and(|status| status.success()) {
            tracing::warn!("unmount refused; keeping mount and handles serving");
            continue;
        }
        return tokio::select! {
            result = &mut joined => finished(&mount, result).await,
            _ = next_signal(&mut term, &mut interrupt) => std::process::exit(5),
        };
    }
}

#[cfg(target_os = "linux")]
async fn finished(
    mount: &anchorfs_mount::runtime::Mount,
    result: Result<std::io::Result<()>, tokio::task::JoinError>,
) -> ExitCode {
    mount.shutdown().await;
    match result {
        Ok(Ok(())) => ExitCode::SUCCESS,
        error => {
            tracing::error!(?error, "FUSE session ended with an error");
            ExitCode::from(5)
        }
    }
}
