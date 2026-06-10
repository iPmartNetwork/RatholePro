# RatholePro

**By iPmart Network (Ali Hassanzadeh)**

> **RatholePro** — Next-generation high-performance reverse proxy tunnel with built-in multiplexing, multi-transport encryption, UDP forwarding, load balancing, P2P NAT traversal, and HTTP proxy support. Engineered for raw speed, zero-compromise security, and dead-simple deployment.

[![GitHub Stars](https://img.shields.io/github/stars/iPmartNetwork/RatholePro?style=flat-square)](https://github.com/iPmartNetwork/RatholePro/stargazers)
[![License](https://img.shields.io/github/license/iPmartNetwork/RatholePro?style=flat-square)](https://github.com/iPmartNetwork/RatholePro/blob/master/LICENSE)
[![Release](https://img.shields.io/github/v/release/iPmartNetwork/RatholePro?style=flat-square)](https://github.com/iPmartNetwork/RatholePro/releases)

---

## Features

| Category | Features |
|----------|----------|
| **Transports** | TCP, TLS 1.3, Noise Protocol, WebSocket, WSS, QUIC |
| **Protocols** | TCP forwarding, UDP forwarding, HTTP/HTTPS proxy |
| **Performance** | Multiplexing, Connection pooling, Async I/O (Tokio) |
| **Security** | SHA-256 token auth, Noise encryption, TLS 1.3, Config validation |
| **Networking** | IPv6 support, Load balancing, P2P NAT traversal (STUN) |
| **Operations** | Auto reconnect, Systemd integration, Menu-driven installer |

---

## Quick Install (Linux)

One command — auto-detects architecture and downloads from [GitHub Releases](https://github.com/iPmartNetwork/RatholePro/releases):

```bash
bash <(curl -Ls https://raw.githubusercontent.com/iPmartNetwork/RatholePro/master/install.sh)
```

Supported architectures: `x86_64`, `aarch64 (ARM64)`, `armv7`

---

## Manual Download

Download the binary for your platform from [Releases](https://github.com/iPmartNetwork/RatholePro/releases):

| Platform | Architecture | File |
|----------|-------------|------|
| Linux | x86_64 (AMD/Intel) | `rathole-pro-x86_64-linux` |
| Linux | aarch64 (ARM64) | `rathole-pro-aarch64-linux` |
| Linux | armv7 (ARM32) | `rathole-pro-armv7-linux` |
| Windows | x86_64 | `rathole-pro-x86_64-windows.exe` |
| macOS | x86_64 | `rathole-pro-x86_64-macos` |

```bash
# Example: download and install on x86_64 Linux
wget https://github.com/iPmartNetwork/RatholePro/releases/latest/download/rathole-pro-x86_64-linux
chmod +x rathole-pro-x86_64-linux
sudo mv rathole-pro-x86_64-linux /usr/local/bin/rathole-pro
```

---

## Build from Source

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
git clone https://github.com/iPmartNetwork/RatholePro.git
cd RatholePro
cargo build --release
# Binary: target/release/rathole-pro
```

---

## Usage

```bash
# Run as server
rathole-pro server.toml

# Run as client
rathole-pro client.toml

# Validate config without running
rathole-pro --validate server.toml

# Generate Noise Protocol keypair
rathole-pro --gen-key
```

---

## Configuration Examples

### Basic TCP Tunnel

**Server** (public IP):
```toml
[server]
bind_addr = "0.0.0.0:2333"
default_token = "my_secret_token"

[server.services.ssh]
bind_addr = "0.0.0.0:5022"
```

**Client** (behind NAT):
```toml
[client]
remote_addr = "your-server.com:2333"
default_token = "my_secret_token"
mux_connections = 4

[client.services.ssh]
local_addr = "127.0.0.1:22"
mux_streams = 8
```

### TLS Encryption
```toml
[server.transport]
type = "tls"

[server.transport.tls]
trusted_root = "/etc/certs/cert.pem"
pkcs12 = "/etc/certs/key.pem"
```

### Noise Protocol (no certificates!)
```toml
[server.transport]
type = "noise"

[server.transport.noise]
local_private_key = "BASE64_SERVER_PRIVATE_KEY"
```

### WebSocket (bypass firewalls)
```toml
[server.transport]
type = "ws"

[server.transport.websocket]
path = "/tunnel"
```

### QUIC (low latency, built-in TLS)
```toml
[server.transport]
type = "quic"

[server.transport.quic]
cert = "/etc/certs/cert.pem"
key = "/etc/certs/key.pem"
```

### UDP Forwarding (WireGuard, games, DNS)
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

### Load Balancing (multiple backends)
```toml
[client.services.web]
local_addr = "127.0.0.1:8080"
backends = ["127.0.0.1:8080", "127.0.0.1:8081", "127.0.0.1:8082"]

[client.services.web.load_balance]
strategy = "round_robin"
health_check_interval = 10
```

### IPv6
```toml
[server]
bind_addr = "[::]:2333"
prefer_ipv6 = true
```

---

## Architecture

```
                     Transport Layer
              (TCP / TLS / Noise / WS / WSS / QUIC)
                              │
Visitor ──► [Server:5022] ──► │ ──► Mux Protocol ──► [Client] ──► [127.0.0.1:22]
Visitor ──► [Server:5022] ──► │    (single connection)
Visitor ──► [Server:8080] ──► │
UDP pkt ──► [Server:51820] ─► │ ──► UDP-over-TCP ──► [Client] ──► [127.0.0.1:51820]
```

---

## Comparison

| Feature | rathole | frp | Backhaul | **RatholePro** |
|---------|---------|-----|----------|----------------|
| Multiplexing | ✗ | ✗ | ✓ | ✓ |
| TLS | ✓ | ✓ | ✗ | ✓ |
| Noise Protocol | ✓ | ✗ | ✗ | ✓ |
| WebSocket | ✓ | ✗ | ✓ | ✓ |
| QUIC | ✗ | ✗ | ✗ | ✓ |
| UDP Forward | ✓ | ✓ | ✓ | ✓ |
| HTTP Proxy | ✗ | ✓ | ✗ | ✓ |
| Load Balance | ✗ | ✓ | ✗ | ✓ |
| P2P/STUN | ✗ | ✗ | ✗ | ✓ |
| Config Validation | ✗ | ✗ | ✗ | ✓ |
| IPv6 | Partial | ✓ | ✗ | ✓ |
| Install Script | ✗ | ✗ | ✗ | ✓ |

---

## Install Script Menu

```
╔═══════════════════════════════════════════════════════════╗
║              Rathole Pro v0.1.0                          ║
║     High-Performance Tunnel + Multi-Protocol + Mux      ║
║  Developer: iPmart Network (Ali Hassanzadeh)            ║
╚═══════════════════════════════════════════════════════════╝

  1) Install Rathole Pro
  2) Configure Server
  3) Configure Client
  4) Start Service
  5) Stop Service
  6) Restart Service
  7) View Status
  8) View Logs
  9) View Config
 10) Update Binary
 11) Uninstall
  0) Exit
```

---

## License

Apache-2.0 — [iPmart Network (Ali Hassanzadeh)](https://github.com/iPmartNetwork)
