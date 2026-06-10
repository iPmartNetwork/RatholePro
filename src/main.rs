#![allow(unused_imports, dead_code, unused_variables, unused_mut)]

mod config;
mod server;
mod client;
mod mux;
mod protocol;

use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "rathole-pro")]
#[command(version = "0.1.0")]
#[command(about = "RatholePro — Next-generation high-performance reverse proxy tunnel.\nMulti-transport | Multiplexing | UDP | Load Balancing | P2P\nDeveloper: iPmart Network (Ali Hassanzadeh)")]
struct Cli {
    /// Path to configuration file
    #[arg(value_name = "CONFIG")]
    config: Option<String>,

    /// Run as server explicitly
    #[arg(long, short = 's')]
    server: bool,

    /// Run as client explicitly
    #[arg(long, short = 'c')]
    client: bool,

    /// Validate config file and exit
    #[arg(long)]
    validate: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    let config_path = cli.config.as_deref()
        .ok_or_else(|| anyhow::anyhow!("Config file required. Usage: rathole-pro <CONFIG>"))?;

    tracing::info!("RatholePro v{} starting...", env!("CARGO_PKG_VERSION"));

    let config = config::load_config(config_path)?;

    if cli.validate {
        println!("✓ Configuration is valid.");
        return Ok(());
    }

    match config::determine_mode(&config, cli.server, cli.client) {
        config::RunMode::Server => {
            tracing::info!("Running in SERVER mode");
            server::run(config).await?;
        }
        config::RunMode::Client => {
            tracing::info!("Running in CLIENT mode");
            client::run(config).await?;
        }
    }

    Ok(())
}
