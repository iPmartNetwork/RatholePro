use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: Option<ServerConfig>,
    pub client: Option<ClientConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub bind_addr: String,
    pub default_token: Option<String>,
    pub heartbeat_interval: Option<u64>,
    pub transport: Option<TransportConfig>,
    pub services: HashMap<String, ServerServiceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    pub remote_addr: String,
    pub default_token: Option<String>,
    pub heartbeat_timeout: Option<u64>,
    pub retry_interval: Option<u64>,
    pub mux_connections: Option<u32>,
    pub transport: Option<TransportConfig>,
    pub services: HashMap<String, ClientServiceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportConfig {
    #[serde(rename = "type", default = "default_transport")]
    pub transport_type: String,
    pub tls: Option<TlsConfig>,
    pub noise: Option<NoiseConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    pub cert: Option<String>,
    pub key: Option<String>,
    pub hostname: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoiseConfig {
    pub local_private_key: Option<String>,
    pub remote_public_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerServiceConfig {
    #[serde(rename = "type", default = "default_service_type")]
    pub service_type: String,
    pub token: Option<String>,
    pub bind_addr: String,
    pub nodelay: Option<bool>,
    pub max_mux_streams: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientServiceConfig {
    #[serde(rename = "type", default = "default_service_type")]
    pub service_type: String,
    pub token: Option<String>,
    pub local_addr: String,
    pub nodelay: Option<bool>,
    pub mux_streams: Option<u32>,
    pub retry_interval: Option<u64>,
}

fn default_transport() -> String { "tcp".to_string() }
fn default_service_type() -> String { "tcp".to_string() }

#[derive(Debug, Clone, PartialEq)]
pub enum RunMode { Server, Client }

pub fn load_config(path: &str) -> anyhow::Result<Config> {
    let content = fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Cannot read '{}': {}", path, e))?;
    let config: Config = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Parse error '{}': {}", path, e))?;
    if config.server.is_none() && config.client.is_none() {
        return Err(anyhow::anyhow!("Config needs [server] or [client]"));
    }
    Ok(config)
}

pub fn determine_mode(config: &Config, force_server: bool, force_client: bool) -> RunMode {
    if force_server { return RunMode::Server; }
    if force_client { return RunMode::Client; }
    match (&config.server, &config.client) {
        (Some(_), None) => RunMode::Server,
        (None, Some(_)) => RunMode::Client,
        _ => RunMode::Server,
    }
}
