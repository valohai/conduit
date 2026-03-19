use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};

#[derive(Parser)]
struct Cli {
    #[arg(short = 'v', long, action = clap::ArgAction::Count, global = true, help = "Increase log verbosity (-v for debug, -vv for trace)")]
    verbose: u8,
    #[arg(short = 'q', long, action = clap::ArgAction::Count, global = true, help = "Decrease log verbosity (-q for warn, -qq for error, -qqq for off)")]
    quiet: u8,
    #[arg(short, long = "config", global = true, help = format!("Path to config TOML file [env: {}]", conduit_core::CONFIG_ENV_VAR))]
    config_path: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Run the proxy server [default command]")]
    Proxy,
    #[command(about = "Open the terminal-based dashboard")]
    Dashboard,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    anyhow::ensure!(
        cli.verbose == 0 || cli.quiet == 0,
        "the argument '--verbose...' cannot be used with '--quiet...'"
    );

    let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        let level = match (cli.verbose, cli.quiet) {
            (2.., _) => "trace",
            (1, _) => "debug",
            (_, 0) => "info",
            (_, 1) => "warn",
            (_, 2) => "error",
            _ => "off",
        };
        tracing_subscriber::EnvFilter::new(level)
    });
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let config = conduit_core::Config::load(cli.config_path)?;
    tracing::debug!(?config, "config loaded");

    // TODO: make database path configurable; defaulting to "conduit.db"
    let transit_storage = conduit_storage::SqliteTransitStorage::new("sqlite:conduit.db").await?;
    let storages = conduit_core::Storages {
        transit: Arc::new(transit_storage),
    };

    let cmd = cli.command.unwrap_or(Commands::Proxy);
    match cmd {
        Commands::Proxy => conduit_proxy::start(config, storages).await?,
        Commands::Dashboard => conduit_tui::start(config, storages)?,
    }

    Ok(())
}
