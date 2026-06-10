#![allow(unused_imports, dead_code, unused_variables, unused_mut)]

mod config;
mod server;
mod client;
mod mux;
mod protocol;
mod udp;

use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "rathole-pro")]
#[command(version = "0.2.0")]
#[command(about = "RatholePro v0.2.0 — TCP + UDP + Multiplexing tunnel\nDeveloper: iPmart Network (Ali Hassanzadeh)")]
struct Cli {
    #[arg(value_name = "CONFIG")]
    config: Option<String>,
    #[arg(long, short = 's')]
    server: bool,
    #[arg(long, short = 'c')]
    client: bool,
    #[arg(long)]
    validate: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    let cli = Cli::parse();
    let path = cli.config.as_deref()
        .ok_or_else(|| anyhow::anyhow!("Usage: rathole-pro <CONFIG>"))?;
    tracing::info!("RatholePro v{}", env!("CARGO_PKG_VERSION"));
    let config = config::load_config(path)?;
    if cli.validate { println!("OK"); return Ok(()); }
    match config::determine_mode(&config, cli.server, cli.client) {
        config::RunMode::Server => server::run(config).await?,
        config::RunMode::Client => client::run(config).await?,
    }
    Ok(())
}
