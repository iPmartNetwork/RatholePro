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
    pub services: HashMap<String, ServiceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    pub remote_addr: String,
    pub default_token: Option<String>,
    pub heartbeat_timeout: Option<u64>,
    pub retry_interval: Option<u64>,
    pub mux_connections: Option<u32>,
    pub services: HashMap<String, ServiceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceConfig {
    #[serde(rename = "type", default = "default_tcp")]
    pub service_type: String,
    pub token: Option<String>,
    #[serde(default)]
    pub bind_addr: Option<String>,
    #[serde(default)]
    pub local_addr: Option<String>,
    pub nodelay: Option<bool>,
    pub mux_streams: Option<u32>,
    pub max_mux_streams: Option<u32>,
    pub retry_interval: Option<u64>,
}

fn default_tcp() -> String { "tcp".to_string() }

#[derive(Debug, Clone, PartialEq)]
pub enum RunMode { Server, Client }

pub fn load_config(path: &str) -> anyhow::Result<Config> {
    let content = fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Cannot read '{}': {}", path, e))?;
    let config: Config = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Parse error '{}': {}", path, e))?;
    if config.server.is_none() && config.client.is_none() {
        return Err(anyhow::anyhow!("Need [server] or [client]"));
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
