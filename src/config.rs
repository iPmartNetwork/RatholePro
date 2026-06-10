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
    pub services: HashMap<String, ServerServiceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    pub remote_addr: String,
    pub default_token: Option<String>,
    pub heartbeat_timeout: Option<u64>,
    pub retry_interval: Option<u64>,
    pub mux_connections: Option<u32>,
    pub services: HashMap<String, ClientServiceConfig>,
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

fn default_service_type() -> String {
    "tcp".to_string()
}

#[derive(Debug, Clone, PartialEq)]
pub enum RunMode {
    Server,
    Client,
}

pub fn load_config(path: &str) -> anyhow::Result<Config> {
    let content = fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Failed to read config '{}': {}", path, e))?;
    let config: Config = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to parse config '{}': {}", path, e))?;

    // Basic validation
    if config.server.is_none() && config.client.is_none() {
        return Err(anyhow::anyhow!("Config must have [server] or [client]"));
    }
    if let Some(ref s) = config.server {
        if s.services.is_empty() {
            return Err(anyhow::anyhow!("Server must have at least one service"));
        }
    }
    if let Some(ref c) = config.client {
        if c.services.is_empty() {
            return Err(anyhow::anyhow!("Client must have at least one service"));
        }
    }

    Ok(config)
}

pub fn determine_mode(config: &Config, force_server: bool, force_client: bool) -> RunMode {
    if force_server { return RunMode::Server; }
    if force_client { return RunMode::Client; }
    match (&config.server, &config.client) {
        (Some(_), None) => RunMode::Server,
        (None, Some(_)) => RunMode::Client,
        (Some(_), Some(_)) => RunMode::Server,
        (None, None) => panic!("No server or client config"),
    }
}
