#![allow(unused_imports, dead_code, unused_variables, unused_mut)]

mod config;
mod server;
mod client;
mod mux;
mod protocol;
mod udp;
mod websocket;
mod load_balancer;
mod http_proxy;
mod p2p;

use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "rathole-pro")]
#[command(version = "0.4.0")]
#[command(about = "RatholePro v0.4.0 — Transparent TCP/UDP tunnel\nDeveloper: iPmart Network (Ali Hassanzadeh)")]
struct Cli {
    #[arg(value_name = "CONFIG")]
    config: Option<String>,
    #[arg(long, short = 's')]
    server: bool,
    #[arg(long, short = 'c')]
    client: bool,
    #[arg(long)]
    validate: bool,
    /// Generate Noise keypair
    #[arg(long)]
    gen_key: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    let cli = Cli::parse();

    if cli.gen_key {
        p2p::gen_noise_keypair();
        return Ok(());
    }

    let path = cli.config.as_deref()
        .ok_or_else(|| anyhow::anyhow!("Usage: rathole-pro <CONFIG>\n       rathole-pro --gen-key"))?;
    tracing::info!("RatholePro v{}", env!("CARGO_PKG_VERSION"));
    let config = config::load_config(path)?;
    if cli.validate { println!("✓ Config OK"); return Ok(()); }
    match config::determine_mode(&config, cli.server, cli.client) {
        config::RunMode::Server => server::run(config).await?,
        config::RunMode::Client => client::run(config).await?,
    }
    Ok(())
}
