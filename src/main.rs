#![allow(unused_imports, dead_code, unused_variables, unused_mut)]

mod config;
mod server;
mod client;
mod mux;
mod protocol;
mod transport;
mod udp;

use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "rathole-pro")]
#[command(version = "0.2.0")]
#[command(about = "RatholePro v0.2.0 — TCP/TLS/Noise + UDP + Multiplexing\nDeveloper: iPmart Network (Ali Hassanzadeh)")]
struct Cli {
    /// Configuration file path
    #[arg(value_name = "CONFIG")]
    config: Option<String>,

    /// Run as server
    #[arg(long, short = 's')]
    server: bool,

    /// Run as client
    #[arg(long, short = 'c')]
    client: bool,

    /// Validate config and exit
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
        gen_noise_key();
        return Ok(());
    }

    let path = cli.config.as_deref()
        .ok_or_else(|| anyhow::anyhow!("Usage: rathole-pro <CONFIG>"))?;

    tracing::info!("RatholePro v{}", env!("CARGO_PKG_VERSION"));

    let config = config::load_config(path)?;

    if cli.validate {
        println!("✓ Config is valid.");
        return Ok(());
    }

    match config::determine_mode(&config, cli.server, cli.client) {
        config::RunMode::Server => server::run(config).await?,
        config::RunMode::Client => client::run(config).await?,
    }
    Ok(())
}

fn gen_noise_key() {
    use base64::Engine;
    let builder = snow::Builder::new("Noise_NK_25519_ChaChaPoly_BLAKE2s".parse().unwrap());
    let kp = builder.generate_keypair().unwrap();
    let priv_b64 = base64::engine::general_purpose::STANDARD.encode(&kp.private);
    let pub_b64 = base64::engine::general_purpose::STANDARD.encode(&kp.public);
    println!("══════════════════════════════════════════");
    println!("  Noise Protocol Keypair (NK_25519)");
    println!("══════════════════════════════════════════");
    println!();
    println!("  Private (SERVER):");
    println!("    {}", priv_b64);
    println!();
    println!("  Public (CLIENT):");
    println!("    {}", pub_b64);
    println!();
    println!("  Server config:");
    println!("    [server.transport.noise]");
    println!("    local_private_key = \"{}\"", priv_b64);
    println!();
    println!("  Client config:");
    println!("    [client.transport.noise]");
    println!("    remote_public_key = \"{}\"", pub_b64);
    println!("══════════════════════════════════════════");
}
