use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "repo-doctor",
    version,
    about = "Explain unmet local setup requirements"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    #[command(about = "Inspect setup and optionally run a bounded startup check")]
    Check(Options),
}

#[derive(clap::Args, Clone)]
pub struct Options {
    #[arg(default_value = ".")]
    pub path: PathBuf,
    #[arg(long, value_enum, default_value = "human")]
    pub format: Format,
    #[arg(long, help = "Reject warnings and incomplete checks")]
    pub strict: bool,
    #[arg(
        long = "static",
        help = "Read files without executing runtimes or opening sockets"
    )]
    pub static_mode: bool,
    #[arg(long, help = "Enable configured loopback TCP connection checks")]
    pub services: bool,
    #[arg(
        long,
        help = "Run the configured startup command after setup passes; executes project code"
    )]
    pub startup: bool,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Format {
    Human,
    Json,
}
