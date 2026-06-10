# RatholePro

**By iPmart Network (Ali Hassanzadeh)**

> **RatholePro** — Next-generation high-performance reverse proxy tunnel built in Go with real yamux multiplexing, multi-transport encryption, auto-TLS, UDP forwarding, load balancing, P2P NAT traversal, and HTTP proxy. Single binary, zero config headache.

[![GitHub Stars](https://img.shields.io/github/stars/iPmartNetwork/RatholePro?style=flat-square)](https://github.com/iPmartNetwork/RatholePro/stargazers)
[![License](https://img.shields.io/github/license/iPmartNetwork/RatholePro?style=flat-square)](https://github.com/iPmartNetwork/RatholePro/blob/master/LICENSE)
[![Release](https://img.shields.io/github/v/release/iPmartNetwork/RatholePro?style=flat-square)](https://github.com/iPmartNetwork/RatholePro/releases)

---

## Features

| Category | Features |
|----------|----------|
| **Core** | Real yamux multiplexing (single TCP, many streams) |
| **Transports** | TCP, TLS (auto-cert!), Noise Protocol, WebSocket |
| **Protocols** | TCP forwarding, UDP forwarding, HTTP/HTTPS proxy |
| **Security** | SHA-256 token auth, TLS 1.2+ (auto-generated cert), Noise encryption |
| **Networking** | IPv6, Load balancing (round-robin/random/least-conn), P2P STUN |
| **Operations** | Auto reconnect, Heartbeat, Systemd service, Menu-driven installer |

---

## Quick Install (Linux)

```bash
bash <(curl -Ls https://raw.githubusercontent.com/iPmartNetwork/RatholePro/master/install.sh)
```

---

## Manual Download

Download from [Releases](https://github.com/iPmartNetwork/RatholePro/releases/latest):

| Platform | Architecture | File |
|----------|-------------|------|
| Linux | x86_64 (AMD/Intel) | `rathole-pro-linux-amd64` |
| Linux | ARM64 (aarch64) | `rathole-pro-linux-arm64` |
| Linux | ARMv7 (32-bit) | `rathole-pro-linux-armv7` |
| Linux | MIPS64 | `rathole-pro-linux-mips64` |
| Linux | MIPS | `rathole-pro-linux-mips` |
| Windows | x86_64 | `rathole-pro-windows-amd64.exe` |
| macOS | x86_64 (Intel) | `rathole-pro-darwin-amd64` |
| macOS | ARM64 (Apple Silicon) | `rathole-pro-darwin-arm64` |

```bash
wget https://github.com/iPmartNetwork/RatholePro/releases/latest/download/rathole-pro-linux-amd64
chmod +x rathole-pro-linux-amd64
sudo mv rathole-pro-linux-amd64 /usr/local/bin/rathole-pro
```

---

## Build from Source

Requires Go 1.22+:

```bash
git clone https://github.com/iPmartNetwork/RatholePro.git
cd RatholePro/go-core
go build -ldflags="-s -w" -o rathole-pro .
sudo mv rathole-pro /usr/local/bin/
```

---

## Usage

```bash
# Server mode
rathole-pro server.toml

# Client mode
rathole-pro client.toml

# Validate config
rathole-pro --validate server.toml

# Generate Noise keypair
rathole-pro --gen-key

# Show version
rathole-pro --version
```

---

## Configuration Examples

### Basic TCP Tunnel

**Server** (public IP — Iran):
```toml
[server]
bind_addr = "0.0.0.0:2333"
default_token = "my_secret_token"
heartbeat_interval = 30

[server.services.web]
bind_addr = "0.0.0.0:8080"
```

**Client** (behind NAT — Kharej):
```toml
[client]
remote_addr = "IRAN_IP:2333"
default_token = "my_secret_token"
retry_interval = 3
mux_connections = 4

[client.services.web]
local_addr = "127.0.0.1:8080"
mux_streams = 8
```

---

### TLS with Auto-Cert (no domain needed!)

**Server:**
```toml
[server.transport]
type = "tls"

[server.transport.tls]
auto_cert = true
```

**Client:**
```toml
[client.transport]
type = "tls"

[client.transport.tls]
# Nothing needed! Encryption active, no cert file required.
```

> Certificate is auto-generated on first run. No domain, no manual setup.

---

### TLS with Custom Cert

```toml
[server.transport]
type = "tls"

[server.transport.tls]
trusted_root = "/etc/certs/cert.pem"
pkcs12 = "/etc/certs/key.pem"
```

---

### Noise Protocol (no certificates!)

```toml
# Server
[server.transport]
type = "noise"

[server.transport.noise]
local_private_key = "BASE64_PRIVATE_KEY"

# Client
[client.transport]
type = "noise"

[client.transport.noise]
remote_public_key = "BASE64_PUBLIC_KEY"
```

Generate keys: `rathole-pro --gen-key`

---

### WebSocket (bypass firewalls)

```toml
[server.transport]
type = "ws"

[server.transport.websocket]
path = "/tunnel"
```

---

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

---

## Architecture

```
                     Transport Layer
              (TCP / TLS / Noise / WebSocket)
                           │
                      ┌────┴────┐
                      │  yamux  │  ← single TCP connection, many streams
                      └────┬────┘
                           │
Visitor ─► [Server:8080] ──┼──► stream 1 ──► [Client] ──► [127.0.0.1:8080]
Visitor ─► [Server:8080] ──┼──► stream 2 ──► [Client] ──► [127.0.0.1:8080]
Visitor ─► [Server:5022] ──┼──► stream 3 ──► [Client] ──► [127.0.0.1:22]
UDP pkt ─► [Server:51820] ─┼──► stream 4 ──► [Client] ──► [127.0.0.1:51820]
```

---

## Install Script Menu

```
╔═══════════════════════════════════════════════════════════╗
║         Rathole Pro v0.4.0 (Go + Yamux)                  ║
║  Transports: TCP │ TLS (auto-cert) │ Noise │ WebSocket  ║
║  Developer: iPmart Network (Ali Hassanzadeh)             ║
╚═══════════════════════════════════════════════════════════╝

  1) Install Binary
  2) Configure IRAN Server (users connect here)
  3) Configure KHAREJ Server (services run here)
  4) Start / 5) Stop / 6) Restart
  7) Status / 8) Logs / 9) View Config
 10) Update / 11) Uninstall
  0) Exit
```

---

## Comparison

| Feature | rathole | frp | Backhaul | **RatholePro** |
|---------|---------|-----|----------|----------------|
| Yamux Multiplexing | ✗ | ✗ | ✓ | ✓ |
| TLS Auto-Cert | ✗ | ✗ | ✗ | ✓ |
| Noise Protocol | ✓ | ✗ | ✗ | ✓ |
| WebSocket | ✓ | ✗ | ✓ | ✓ |
| UDP Forward | ✓ | ✓ | ✓ | ✓ |
| HTTP Proxy | ✗ | ✓ | ✗ | ✓ |
| Load Balance | ✗ | ✓ | ✗ | ✓ |
| P2P/STUN | ✗ | ✗ | ✗ | ✓ |
| IPv6 | Partial | ✓ | ✗ | ✓ |
| Install Script | ✗ | ✗ | ✗ | ✓ |
| Single Binary | ✓ | ✓ | ✓ | ✓ |

---

## License

Apache-2.0 — [iPmart Network (Ali Hassanzadeh)](https://github.com/iPmartNetwork)
