use anchorfs_mount::{config::MountArgs, profile::MountProfile};
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
    let filter = if cfg.args.log_format == "json" {
        tracing_subscriber::EnvFilter::new("info")
    } else {
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())
    };
    let format = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr);
    if cfg.args.log_format == "json" {
        format.json().init();
    } else {
        format.init();
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
        .worker_threads(cfg.args.fuse_threads)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("runtime: {e}");
            return ExitCode::from(4);
        }
    };
    let mount = match rt.block_on(Mount::start(cfg, rt.handle().clone())) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(3);
        }
    };
    let session = match platform::spawn(mount.clone()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            rt.block_on(mount.shutdown());
            return ExitCode::from(4);
        }
    };
    let signals = rt.block_on(async {
        Ok::<_, std::io::Error>((
            signal(SignalKind::terminate())?,
            signal(SignalKind::interrupt())?,
        ))
    });
    let (mut term, mut interrupt) = match signals {
        Ok(s) => s,
        Err(e) => {
            eprintln!("signals: {e}");
            return ExitCode::from(5);
        }
    };
    loop {
        let stop=rt.block_on(async {tokio::select!{_=term.recv()=>true,_=interrupt.recv()=>true,_=tokio::time::sleep(std::time::Duration::from_millis(100))=>false}});
        if session.guard.is_finished() {
            let result = session.join();
            eprintln!("FUSE session ended: {result:?}");
            return ExitCode::from(5);
        }
        if !stop {
            continue;
        }
        // fuser's unmount helper can detach lazily. First require a successful
        // non-lazy unmount so open tool files remain usable on EBUSY.
        let status = std::process::Command::new("fusermount3")
            .arg("-u")
            .arg(&mount.cfg.args.mountpoint)
            .status();
        if !status.is_ok_and(|s| s.success()) {
            tracing::warn!("unmount refused; keeping mount and handles serving");
            continue;
        }
        let mut join = rt.spawn_blocking(move || session.umount_and_join());
        let result=rt.block_on(async {tokio::select! {result=&mut join=>Some(result),_=term.recv()=>None,_=interrupt.recv()=>None}});
        match result {
            Some(Ok(Ok(()))) => {
                rt.block_on(mount.shutdown());
                return ExitCode::SUCCESS;
            }
            _ => {
                tracing::error!("shutdown interrupted or FUSE runtime failed");
                std::process::exit(5);
            }
        }
    }
}
