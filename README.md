# Rathole Pro

**By iPmart Network (Ali Hassanzadeh)**

> **RatholePro** — Next-generation high-performance reverse proxy tunnel with built-in multiplexing, multi-transport encryption, UDP forwarding, load balancing, P2P NAT traversal, and HTTP proxy support. Engineered for raw speed, zero-compromise security, and dead-simple deployment.

A high-performance reverse proxy tunnel with **multiplexing** and **multi-protocol** support, written in Rust.

Built on the concepts of [rathole](https://github.com/rathole-org/rathole) with enhanced features.

[![GitHub](https://img.shields.io/github/stars/iPmartNetwork/RatholePro?style=social)](https://github.com/iPmartNetwork/RatholePro)
[![License](https://img.shields.io/github/license/iPmartNetwork/RatholePro)](https://github.com/iPmartNetwork/RatholePro/blob/main/LICENSE)

## Features

- **Multiplexing (Mux)** - Multiple logical streams over a single TCP connection
- **Multi-Transport** - TCP, TLS, Noise Protocol, WebSocket (ws/wss)
- **High Performance** - Async I/O with Tokio runtime
- **Token Authentication** - SHA-256 hashed tokens, never sent in plaintext
- **Custom Binary Protocol** - Framing with magic bytes and version control
- **Auto Reconnect** - Configurable retry on disconnect
- **Connection Pooling** - Round-robin multiplexer pool
- **Easy Setup** - Interactive menu-driven install script
- **Systemd Integration** - Run as a background service
- **Minimal Binary** - Optimized with LTO and strip

## Supported Transports

| Transport | Description | Use Case |
|-----------|-------------|----------|
| `tcp` | Raw TCP (default) | Local networks, fast |
| `tls` | TLS 1.3 encryption (rustls) | Public internet, certificate-based |
| `noise` | Noise Protocol (ChaCha20-Poly1305) | No certificates needed |
| `ws` | WebSocket | Bypass firewalls, CDN-friendly |
| `wss` | WebSocket over TLS | Secure + firewall bypass |

## Quick Install (Linux)

```bash
bash <(curl -Ls https://raw.githubusercontent.com/iPmartNetwork/RatholePro/main/install.sh)
```

## Build from Source

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
git clone https://github.com/iPmartNetwork/RatholePro.git
cd RatholePro
cargo build --release
```

## Usage Examples

### TCP (default, no encryption)
```toml
# server.toml
[server]
bind_addr = "0.0.0.0:2333"
default_token = "secret"

[server.services.ssh]
bind_addr = "0.0.0.0:5022"
```

### TLS (certificate-based encryption)
```toml
# server.toml
[server]
bind_addr = "0.0.0.0:2333"
default_token = "secret"

[server.transport]
type = "tls"

[server.transport.tls]
trusted_root = "/etc/certs/cert.pem"
pkcs12 = "/etc/certs/key.pem"

[server.services.ssh]
bind_addr = "0.0.0.0:5022"
```

### Noise Protocol (no certificates!)
```toml
# server.toml
[server]
bind_addr = "0.0.0.0:2333"
default_token = "secret"

[server.transport]
type = "noise"

[server.transport.noise]
local_private_key = "BASE64_PRIVATE_KEY"

[server.services.ssh]
bind_addr = "0.0.0.0:5022"
```

### WebSocket (firewall bypass)
```toml
# server.toml
[server]
bind_addr = "0.0.0.0:8080"
default_token = "secret"

[server.transport]
type = "ws"

[server.transport.websocket]
path = "/tunnel"

[server.services.web]
bind_addr = "0.0.0.0:9090"
```

## Architecture

```
                     Transport Layer (TCP/TLS/Noise/WS)
                              │
Visitor ──► [Server:5022] ──► │ ──► Mux Protocol ──► [Client] ──► [127.0.0.1:22]
Visitor ──► [Server:5022] ──► │         (single connection)
Visitor ──► [Server:8080] ──► │
```

Multiple visitor connections are multiplexed over a single transport
connection using a custom mux protocol (SYN/DATA/FIN frames).

## vs Other Tunnels

| Feature | rathole | frp | Rathole Pro |
|---------|---------|-----|-------------|
| Multiplexing | ✗ | ✗ | ✓ |
| TLS | ✓ | ✓ | ✓ |
| Noise Protocol | ✓ | ✗ | ✓ |
| WebSocket | ✓ | ✗ | ✓ |
| Connection Pool | ✗ | ✗ | ✓ |
| Install Script | ✗ | ✗ | ✓ |
| Binary Protocol | ✗ | ✗ | ✓ |
| Auto Reconnect | ✓ | ✓ | ✓ |
| Hot Reload | ✓ | ✓ | Planned |
| UDP Forward | ✓ | ✓ | ✓ |

## Configuration Reference

### Transport Types

```toml
[client.transport]
type = "tcp"    # Raw TCP (default)
type = "tls"    # TLS encryption
type = "noise"  # Noise Protocol
type = "ws"     # WebSocket
type = "wss"    # Secure WebSocket
```

### TLS Options

```toml
[client.transport.tls]
trusted_root = "ca.pem"       # CA certificate (client)
hostname = "example.com"      # SNI hostname (client)

[server.transport.tls]
trusted_root = "cert.pem"     # Server certificate chain
pkcs12 = "key.pem"            # Server private key
```

### Noise Options

```toml
[transport.noise]
pattern = "Noise_NK_25519_ChaChaPoly_BLAKE2s"  # Noise pattern
local_private_key = "base64..."                 # Local private key
remote_public_key = "base64..."                 # Remote public key
```

### WebSocket Options

```toml
[transport.websocket]
path = "/tunnel"              # WebSocket endpoint path
tls = true                    # Use wss:// (set type = "wss" instead)
```

### UDP Forwarding

```toml
# Server
[server.services.wireguard]
type = "udp"
bind_addr = "0.0.0.0:51820"

# Client
[client.services.wireguard]
type = "udp"
local_addr = "127.0.0.1:51820"
```

UDP packets are encapsulated over the TCP/TLS/Noise/WS control channel.
Each unique source address gets its own mux stream. Sessions expire
after 60 seconds of inactivity.

Supported use cases:
- WireGuard VPN
- DNS forwarding
- Game servers (CS2, Minecraft)
- VoIP / SIP
- Any UDP-based protocol

## License

Apache-2.0
