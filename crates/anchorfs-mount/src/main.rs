use anchorfs_mount::profile::MountProfile;
use clap::{Parser, Subcommand};
use std::process::ExitCode;
#[derive(Parser)]
#[command(
    version,
    about = "AnchorFS mount profile and Stage 0 skeleton",
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
    Mount {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        arguments: Vec<String>,
    },
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
    eprintln!("mounting is not implemented in stage 0");
    ExitCode::from(2)
}
