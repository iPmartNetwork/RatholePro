mod config;
mod server;
mod client;
mod mux;
mod protocol;
mod transport;
mod udp;
mod http_proxy;
mod load_balancer;
mod p2p;

use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "rathole-pro")]
#[command(version = "0.1.0")]
#[command(about = "RatholePro — Next-generation high-performance reverse proxy tunnel.\nMulti-transport (TCP/TLS/Noise/WS/WSS/QUIC) | Multiplexing | UDP | Load Balancing | P2P\nDeveloper: iPmart Network (Ali Hassanzadeh)")]
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

    /// Generate Noise Protocol keypair
    #[arg(long)]
    gen_key: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    // Handle --gen-key
    if cli.gen_key {
        #[cfg(feature = "transport-noise")]
        {
            generate_noise_keypair();
            return Ok(());
        }
        #[cfg(not(feature = "transport-noise"))]
        {
            eprintln!("Noise transport feature not enabled. Build with --features transport-noise");
            return Ok(());
        }
    }

    // Config file is required for all other operations
    let config_path = cli.config.as_deref()
        .ok_or_else(|| anyhow::anyhow!("Config file path required. Usage: rathole-pro <CONFIG>"))?;

    tracing::info!("Rathole Pro v{} starting...", env!("CARGO_PKG_VERSION"));

    let config = config::load_config(config_path)?;

    // Handle --validate
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

/// Generate and print a Noise Protocol keypair
#[cfg(feature = "transport-noise")]
fn generate_noise_keypair() {
    use snow::Builder;
    use base64::Engine;

    let builder = Builder::new("Noise_NK_25519_ChaChaPoly_BLAKE2s".parse().unwrap());
    let keypair = builder.generate_keypair().unwrap();

    let private_b64 = base64::engine::general_purpose::STANDARD.encode(&keypair.private);
    let public_b64 = base64::engine::general_purpose::STANDARD.encode(&keypair.public);

    println!("╔══════════════════════════════════════════════════════╗");
    println!("║         Noise Protocol Keypair Generated            ║");
    println!("╠══════════════════════════════════════════════════════╣");
    println!("║ Pattern: Noise_NK_25519_ChaChaPoly_BLAKE2s          ║");
    println!("╚══════════════════════════════════════════════════════╝");
    println!();
    println!("Private Key (keep secret - use on SERVER):");
    println!("  {}", private_b64);
    println!();
    println!("Public Key (share with clients):");
    println!("  {}", public_b64);
    println!();
    println!("Server config:");
    println!("  [server.transport.noise]");
    println!("  local_private_key = \"{}\"", private_b64);
    println!();
    println!("Client config:");
    println!("  [client.transport.noise]");
    println!("  remote_public_key = \"{}\"", public_b64);
}
