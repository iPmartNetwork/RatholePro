use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::net::SocketAddr;

/// Application configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: Option<ServerConfig>,
    pub client: Option<ClientConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Address the server listens for client connections
    pub bind_addr: String,
    /// Default token for services
    pub default_token: Option<String>,
    /// Heartbeat interval in seconds (0 to disable)
    pub heartbeat_interval: Option<u64>,
    /// Prefer IPv6 for outbound connections
    pub prefer_ipv6: Option<bool>,
    /// Transport configuration
    pub transport: Option<TransportConfig>,
    /// Services to expose
    pub services: HashMap<String, ServerServiceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    /// Address of the server (supports IPv4 and IPv6)
    pub remote_addr: String,
    /// Default token for services
    pub default_token: Option<String>,
    /// Heartbeat timeout in seconds
    pub heartbeat_timeout: Option<u64>,
    /// Retry interval in seconds
    pub retry_interval: Option<u64>,
    /// Number of multiplexed connections
    pub mux_connections: Option<u32>,
    /// Prefer IPv6 for outbound connections
    pub prefer_ipv6: Option<bool>,
    /// Transport configuration
    pub transport: Option<TransportConfig>,
    /// HTTP/HTTPS proxy settings
    pub http_proxy: Option<HttpProxyConfig>,
    /// Services to forward
    pub services: HashMap<String, ClientServiceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportConfig {
    /// Transport type: "tcp", "tls", "noise", "ws", "wss", "quic"
    #[serde(rename = "type", default = "default_transport_type")]
    pub transport_type: String,
    /// TCP-specific settings
    pub tcp: Option<TcpConfig>,
    /// TLS settings
    pub tls: Option<TlsConfig>,
    /// Noise protocol settings
    pub noise: Option<NoiseConfig>,
    /// WebSocket settings
    pub websocket: Option<WebSocketConfig>,
    /// QUIC settings
    pub quic: Option<QuicConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketConfig {
    /// WebSocket path (default: "/tunnel")
    pub path: Option<String>,
    /// Enable TLS for WebSocket (wss://)
    pub tls: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TcpConfig {
    pub nodelay: Option<bool>,
    pub keepalive_secs: Option<u64>,
    pub keepalive_interval: Option<u64>,
    pub proxy: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TlsConfig {
    pub trusted_root: Option<String>,
    pub hostname: Option<String>,
    pub pkcs12: Option<String>,
    pub pkcs12_password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NoiseConfig {
    pub pattern: Option<String>,
    pub local_private_key: Option<String>,
    pub remote_public_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuicConfig {
    /// Path to certificate PEM (server)
    pub cert: Option<String>,
    /// Path to private key PEM (server)
    pub key: Option<String>,
    /// CA certificate for verification (client)
    pub ca: Option<String>,
    /// ALPN protocols
    pub alpn: Option<Vec<String>>,
    /// Max concurrent streams per connection
    pub max_streams: Option<u32>,
    /// Keep alive interval in seconds
    pub keep_alive: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpProxyConfig {
    /// HTTP proxy URL (http://host:port or socks5://host:port)
    pub url: String,
    /// Proxy username
    pub username: Option<String>,
    /// Proxy password
    pub password: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerServiceConfig {
    /// Protocol type: "tcp", "udp", or "http"
    #[serde(rename = "type", default = "default_service_type")]
    pub service_type: String,
    /// Authentication token
    pub token: Option<String>,
    /// Address to bind this service
    pub bind_addr: String,
    /// Enable TCP_NODELAY
    pub nodelay: Option<bool>,
    /// Max multiplexed streams for this service
    pub max_mux_streams: Option<u32>,
    /// Load balancing config (multiple backends on client side)
    pub load_balance: Option<LoadBalanceConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientServiceConfig {
    /// Protocol type: "tcp", "udp", or "http"
    #[serde(rename = "type", default = "default_service_type")]
    pub service_type: String,
    /// Authentication token
    pub token: Option<String>,
    /// Local address to forward to (single backend)
    pub local_addr: String,
    /// Multiple backends for load balancing
    pub backends: Option<Vec<String>>,
    /// Load balancing strategy
    pub load_balance: Option<LoadBalanceConfig>,
    /// Enable TCP_NODELAY
    pub nodelay: Option<bool>,
    /// Number of mux streams for this service
    pub mux_streams: Option<u32>,
    /// Retry interval override
    pub retry_interval: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadBalanceConfig {
    /// Strategy: "round_robin", "random", "least_conn"
    pub strategy: Option<String>,
    /// Health check interval in seconds (0 to disable)
    pub health_check_interval: Option<u64>,
}

/// P2P / NAT traversal configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct P2pConfig {
    /// Enable P2P mode (attempt direct connection)
    pub enabled: bool,
    /// STUN server address
    pub stun_server: Option<String>,
    /// TURN server address (fallback relay)
    pub turn_server: Option<String>,
    /// TURN username
    pub turn_username: Option<String>,
    /// TURN password
    pub turn_password: Option<String>,
}

fn default_transport_type() -> String {
    "tcp".to_string()
}

fn default_service_type() -> String {
    "tcp".to_string()
}

/// Running mode determination
#[derive(Debug, Clone, PartialEq)]
pub enum RunMode {
    Server,
    Client,
}

/// Load configuration from a TOML file
pub fn load_config(path: &str) -> anyhow::Result<Config> {
    let content = fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("Failed to read config file '{}': {}", path, e))?;
    let config: Config = toml::from_str(&content)
        .map_err(|e| anyhow::anyhow!("Failed to parse config file '{}': {}", path, e))?;

    // Validate after parsing
    validate_config(&config)?;

    Ok(config)
}

/// Comprehensive config validation
pub fn validate_config(config: &Config) -> anyhow::Result<()> {
    if config.server.is_none() && config.client.is_none() {
        return Err(anyhow::anyhow!(
            "Config must contain at least one of [server] or [client]"
        ));
    }

    if let Some(ref server) = config.server {
        validate_server_config(server)?;
    }

    if let Some(ref client) = config.client {
        validate_client_config(client)?;
    }

    Ok(())
}

fn validate_server_config(config: &ServerConfig) -> anyhow::Result<()> {
    // Validate bind_addr
    validate_addr(&config.bind_addr, "server.bind_addr")?;

    // Must have at least one service
    if config.services.is_empty() {
        return Err(anyhow::anyhow!(
            "Server must have at least one service defined"
        ));
    }

    // Validate each service
    for (name, svc) in &config.services {
        validate_addr(&svc.bind_addr, &format!("server.services.{}.bind_addr", name))?;
        validate_service_type(&svc.service_type, name)?;

        // Validate token exists (either per-service or default)
        if svc.token.is_none() && config.default_token.is_none() {
            return Err(anyhow::anyhow!(
                "Service '{}' has no token and no default_token is set", name
            ));
        }
    }

    // Validate transport config
    if let Some(ref transport) = config.transport {
        validate_transport(transport, "server")?;
    }

    Ok(())
}

fn validate_client_config(config: &ClientConfig) -> anyhow::Result<()> {
    // Validate remote_addr
    validate_addr(&config.remote_addr, "client.remote_addr")?;

    // Must have at least one service
    if config.services.is_empty() {
        return Err(anyhow::anyhow!(
            "Client must have at least one service defined"
        ));
    }

    // Validate each service
    for (name, svc) in &config.services {
        validate_addr(&svc.local_addr, &format!("client.services.{}.local_addr", name))?;
        validate_service_type(&svc.service_type, name)?;

        // Validate token
        if svc.token.is_none() && config.default_token.is_none() {
            return Err(anyhow::anyhow!(
                "Service '{}' has no token and no default_token is set", name
            ));
        }

        // Validate load balance backends
        if let Some(ref backends) = svc.backends {
            if backends.is_empty() {
                return Err(anyhow::anyhow!(
                    "Service '{}' has empty backends list", name
                ));
            }
            for (i, backend) in backends.iter().enumerate() {
                validate_addr(backend, &format!("client.services.{}.backends[{}]", name, i))?;
            }
        }
    }

    // Validate transport
    if let Some(ref transport) = config.transport {
        validate_transport(transport, "client")?;
    }

    // Validate HTTP proxy
    if let Some(ref proxy) = config.http_proxy {
        if proxy.url.is_empty() {
            return Err(anyhow::anyhow!("client.http_proxy.url cannot be empty"));
        }
        if !proxy.url.starts_with("http://")
            && !proxy.url.starts_with("https://")
            && !proxy.url.starts_with("socks5://")
        {
            return Err(anyhow::anyhow!(
                "client.http_proxy.url must start with http://, https://, or socks5://"
            ));
        }
    }

    Ok(())
}

fn validate_addr(addr: &str, field_name: &str) -> anyhow::Result<()> {
    // Try parsing as SocketAddr (supports both IPv4 and IPv6)
    // Format: "host:port" or "[::1]:port"
    if addr.parse::<SocketAddr>().is_err() {
        // Could be hostname:port - check if it has a port
        if let Some(colon_pos) = addr.rfind(':') {
            let port_str = &addr[colon_pos + 1..];
            if port_str.parse::<u16>().is_err() {
                return Err(anyhow::anyhow!(
                    "'{}' = '{}': invalid port number", field_name, addr
                ));
            }
        } else {
            return Err(anyhow::anyhow!(
                "'{}' = '{}': must be in format 'host:port' or '[ipv6]:port'",
                field_name, addr
            ));
        }
    }
    Ok(())
}

fn validate_service_type(service_type: &str, name: &str) -> anyhow::Result<()> {
    match service_type {
        "tcp" | "udp" | "http" => Ok(()),
        other => Err(anyhow::anyhow!(
            "Service '{}': invalid type '{}'. Must be 'tcp', 'udp', or 'http'",
            name, other
        )),
    }
}

fn validate_transport(transport: &TransportConfig, side: &str) -> anyhow::Result<()> {
    let valid_types = ["tcp", "tls", "noise", "ws", "wss", "quic"];
    if !valid_types.contains(&transport.transport_type.as_str()) {
        return Err(anyhow::anyhow!(
            "{}.transport.type '{}' is invalid. Valid: {:?}",
            side, transport.transport_type, valid_types
        ));
    }

    // TLS requires cert config
    if transport.transport_type == "tls" {
        if side == "server" && transport.tls.is_none() {
            return Err(anyhow::anyhow!(
                "server.transport.type = 'tls' requires [server.transport.tls] section"
            ));
        }
    }

    // Noise requires at least pattern or keys
    if transport.transport_type == "noise" {
        if transport.noise.is_none() {
            return Err(anyhow::anyhow!(
                "{}.transport.type = 'noise' requires [{}.transport.noise] section",
                side, side
            ));
        }
    }

    // QUIC requires cert/key on server
    if transport.transport_type == "quic" {
        if transport.quic.is_none() {
            return Err(anyhow::anyhow!(
                "{}.transport.type = 'quic' requires [{}.transport.quic] section",
                side, side
            ));
        }
        if side == "server" {
            if let Some(ref quic) = transport.quic {
                if quic.cert.is_none() || quic.key.is_none() {
                    return Err(anyhow::anyhow!(
                        "server.transport.quic requires 'cert' and 'key' fields"
                    ));
                }
            }
        }
    }

    Ok(())
}

/// Determine whether to run as server or client
pub fn determine_mode(config: &Config, force_server: bool, force_client: bool) -> RunMode {
    if force_server {
        return RunMode::Server;
    }
    if force_client {
        return RunMode::Client;
    }

    match (&config.server, &config.client) {
        (Some(_), None) => RunMode::Server,
        (None, Some(_)) => RunMode::Client,
        (Some(_), Some(_)) => {
            tracing::warn!(
                "Both [server] and [client] found. Use --server or --client to specify."
            );
            RunMode::Server
        }
        (None, None) => {
            unreachable!("Validated above");
        }
    }
}
