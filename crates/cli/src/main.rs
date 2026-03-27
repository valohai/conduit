use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use tracing_subscriber::layer::SubscriberExt;

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
    Dashboard {
        #[arg(
            long,
            default_value = "default",
            help = "Color theme (default, qbasic, gorillas, nibbles)"
        )]
        theme: conduit_tui::theme::ThemeName,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    anyhow::ensure!(
        cli.verbose == 0 || cli.quiet == 0,
        "the argument '--verbose...' cannot be used with '--quiet...'"
    );

    let cmd = cli.command.unwrap_or(Commands::Proxy);

    // with the dashboard, only show "error" level logs by default, but allow
    // for up to 4 levels of verbosity; RUST_LOG still overrides this
    let log_level = match (&cmd, cli.verbose, cli.quiet) {
        (Commands::Dashboard { .. }, 4.., _) => "trace",
        (Commands::Dashboard { .. }, 3, _) => "debug",
        (Commands::Dashboard { .. }, 2, _) => "info",
        (Commands::Dashboard { .. }, 1, _) => "warn",
        (Commands::Dashboard { .. }, _, 0) => "error",
        (_, 2.., _) => "trace",
        (_, 1, _) => "debug",
        (_, _, 0) => "info",
        (_, _, 1) => "warn",
        (_, _, 2) => "error",
        _ => "off",
    };

    let log_filter = || {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(log_level))
    };

    if !matches!(cmd, Commands::Dashboard { .. }) {
        // we'll handle logging for dashboard separately to avoid
        // messing the terminal UI output
        tracing_subscriber::fmt()
            .with_env_filter(log_filter())
            .init();
    }

    let config = conduit_core::Config::load(cli.config_path)?;
    tracing::debug!(?config, "config loaded");

    // TODO: make database path configurable; defaulting to "conduit.db"
    let transit_storage = conduit_storage::SqliteTransitStorage::new("sqlite:conduit.db").await?;
    let storages = conduit_core::Storages {
        transit: Arc::new(transit_storage),
    };

    match cmd {
        Commands::Proxy => conduit_proxy::start(config, storages).await?,
        Commands::Dashboard { theme } => {
            let log_buffer = conduit_tui::logging::new_log_buffer();
            let layer = conduit_tui::logging::TuiLogLayer::new(log_buffer.clone());
            let subscriber = tracing_subscriber::Registry::default()
                .with(log_filter())
                .with(layer);
            tracing::subscriber::set_global_default(subscriber)?;

            conduit_tui::start(config, storages, log_buffer, theme.theme())?;
        }
    }

    Ok(())
}
