#!/bin/bash

# ═══════════════════════════════════════════════════════════════
# RatholePro - Professional Installation & Management Script
# Next-generation reverse proxy tunnel with multiplexing,
# multi-transport, load balancing, P2P, and UDP support.
# Developer: iPmart Network (Ali Hassanzadeh)
# Version: 0.1.0
# ═══════════════════════════════════════════════════════════════

set -euo pipefail

# ─── Colors ────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
BOLD='\033[1m'
DIM='\033[2m'
NC='\033[0m'

# ─── Constants ─────────────────────────────────────────────────
readonly RATHOLE_PRO_DIR="/opt/rathole-pro"
readonly CONFIG_DIR="/etc/rathole-pro"
readonly LOG_DIR="/var/log/rathole-pro"
readonly BINARY_NAME="rathole-pro"
readonly GITHUB_REPO="iPmartNetwork/RatholePro"
readonly APP_VERSION="0.3.0"
readonly SERVICE_PREFIX="rathole-pro"
readonly AUTHOR="iPmart Network (Ali Hassanzadeh)"

# ─── Helper Functions ──────────────────────────────────────────

print_banner() {
    clear
    echo -e "${CYAN}"
    echo "╔═══════════════════════════════════════════════════════════╗"
    echo "║                                                           ║"
    echo "║              Rathole Pro v${APP_VERSION}                        ║"
    echo "║     High-Performance Tunnel + Multi-Protocol + Mux        ║"
    echo "║                                                           ║"
    echo "║  Transports: TCP │ TLS │ Noise │ WS │ WSS │ QUIC         ║"
    echo "║  Protocols:  TCP │ UDP │ HTTP                             ║"
    echo "║  Features:   Mux │ Load Balance │ P2P │ IPv6              ║"
    echo "║                                                           ║"
    echo "║  Developer: iPmart Network (Ali Hassanzadeh)              ║"
    echo "║                                                           ║"
    echo "╚═══════════════════════════════════════════════════════════╝"
    echo -e "${NC}"
}

print_info()    { echo -e "  ${BLUE}[INFO]${NC} $1"; }
print_success() { echo -e "  ${GREEN}[✓]${NC} $1"; }
print_error()   { echo -e "  ${RED}[✗]${NC} $1"; }
print_warning() { echo -e "  ${YELLOW}[!]${NC} $1"; }
print_step()    { echo -e "  ${MAGENTA}[→]${NC} $1"; }
print_divider() { echo -e "  ${DIM}───────────────────────────────────────────${NC}"; }

confirm_action() {
    local prompt="${1:-Are you sure?}"
    read -rp "  $prompt (y/n): " answer
    [[ "$answer" =~ ^[Yy]$ ]]
}

check_root() {
    if [[ $EUID -ne 0 ]]; then
        print_error "This script must be run as root (use sudo)"
        exit 1
    fi
}

detect_os() {
    if [[ -f /etc/os-release ]]; then
        OS=$(grep -m1 '^ID=' /etc/os-release | cut -d= -f2 | tr -d '"')
        OS_VERSION=$(grep -m1 '^VERSION_ID=' /etc/os-release | cut -d= -f2 | tr -d '"')
        OS_NAME=$(grep -m1 '^PRETTY_NAME=' /etc/os-release | cut -d= -f2 | tr -d '"')
        OS="${OS:-linux}"
        OS_VERSION="${OS_VERSION:-unknown}"
        OS_NAME="${OS_NAME:-${OS}}"
    elif [[ -f /etc/redhat-release ]]; then
        OS="centos"
        OS_NAME=$(cat /etc/redhat-release)
    else
        OS="unknown"
        OS_NAME="Unknown Linux"
    fi
    print_info "OS: ${OS_NAME}"
}

detect_arch() {
    ARCH=$(uname -m)
    case "${ARCH}" in
        x86_64|amd64)   ARCH="x86_64" ;;
        aarch64|arm64)  ARCH="aarch64" ;;
        armv7l|armhf)   ARCH="armv7" ;;
        i686|i386)      ARCH="i686" ;;
        mips|mipsel)    ARCH="mips" ;;
        *)
            print_error "Unsupported architecture: ${ARCH}"
            exit 1
            ;;
    esac
    print_info "Architecture: ${ARCH}"
}

detect_ipv6() {
    if [[ -f /proc/net/if_inet6 ]] && [[ -s /proc/net/if_inet6 ]]; then
        IPV6_AVAILABLE=true
        print_info "IPv6: Available"
    else
        IPV6_AVAILABLE=false
        print_info "IPv6: Not available"
    fi
}

generate_token() {
    # Generate secure random token
    if command -v openssl &>/dev/null; then
        openssl rand -hex 16
    else
        tr -dc 'a-f0-9' </dev/urandom | head -c 32
    fi
}

generate_noise_keypair() {
    # Generate 32-byte key encoded as base64
    if command -v openssl &>/dev/null; then
        openssl rand -base64 32
    else
        head -c 32 /dev/urandom | base64
    fi
}

# ─── Installation ──────────────────────────────────────────────

install_dependencies() {
    print_step "Installing dependencies..."
    if command -v apt-get &>/dev/null; then
        apt-get update -qq >/dev/null 2>&1
        apt-get install -y -qq curl wget tar jq openssl >/dev/null 2>&1
    elif command -v dnf &>/dev/null; then
        dnf install -y -q curl wget tar jq openssl >/dev/null 2>&1
    elif command -v yum &>/dev/null; then
        yum install -y -q curl wget tar jq openssl >/dev/null 2>&1
    elif command -v pacman &>/dev/null; then
        pacman -Sy --noconfirm --quiet curl wget tar jq openssl >/dev/null 2>&1
    elif command -v apk &>/dev/null; then
        apk add --quiet curl wget tar jq openssl >/dev/null 2>&1
    else
        print_warning "Unknown package manager. Please install: curl wget tar jq openssl"
    fi
    print_success "Dependencies installed"
}

download_binary() {
    print_step "Downloading RatholePro for ${ARCH}..."
    mkdir -p "${RATHOLE_PRO_DIR}"
    mkdir -p "${CONFIG_DIR}"
    mkdir -p "${LOG_DIR}"

    # Determine the correct asset name based on architecture
    local asset_name=""
    case "${ARCH}" in
        x86_64)   asset_name="${BINARY_NAME}-x86_64-linux" ;;
        aarch64)  asset_name="${BINARY_NAME}-aarch64-linux" ;;
        armv7)    asset_name="${BINARY_NAME}-armv7-linux" ;;
        i686)     asset_name="${BINARY_NAME}-i686-linux" ;;
        mips)     asset_name="${BINARY_NAME}-mips-linux" ;;
        *)        asset_name="${BINARY_NAME}-${ARCH}-linux" ;;
    esac

    # Try to get latest release version from GitHub API
    local latest_version="${APP_VERSION}"
    if command -v curl &>/dev/null && command -v jq &>/dev/null; then
        local api_response
        api_response=$(curl -fsSL "https://api.github.com/repos/${GITHUB_REPO}/releases/latest" 2>/dev/null)
        if [[ -n "${api_response}" ]]; then
            local tag
            tag=$(echo "${api_response}" | jq -r '.tag_name // empty' 2>/dev/null)
            if [[ -n "${tag}" ]]; then
                latest_version="${tag#v}"
                print_info "Latest release: v${latest_version}"
            fi
        fi
    fi

    # Download URL from GitHub Releases
    local download_url="https://github.com/${GITHUB_REPO}/releases/download/v${latest_version}/${asset_name}"
    local tmp_file="/tmp/${asset_name}"

    print_info "Downloading: ${download_url}"

    # Download binary
    local download_ok=false
    if command -v curl &>/dev/null; then
        if curl -fsSL -o "${tmp_file}" "${download_url}" 2>/dev/null; then
            download_ok=true
        fi
    elif command -v wget &>/dev/null; then
        if wget -q -O "${tmp_file}" "${download_url}" 2>/dev/null; then
            download_ok=true
        fi
    fi

    # If direct binary didn't work, try .tar.gz format
    if [[ "${download_ok}" != true ]]; then
        local tar_url="https://github.com/${GITHUB_REPO}/releases/download/v${latest_version}/${asset_name}.tar.gz"
        local tar_file="/tmp/${asset_name}.tar.gz"
        print_info "Trying archive format..."

        if command -v curl &>/dev/null; then
            curl -fsSL -o "${tar_file}" "${tar_url}" 2>/dev/null
        elif command -v wget &>/dev/null; then
            wget -q -O "${tar_file}" "${tar_url}" 2>/dev/null
        fi

        if [[ -f "${tar_file}" ]] && [[ -s "${tar_file}" ]]; then
            tar -xzf "${tar_file}" -C "${RATHOLE_PRO_DIR}/" 2>/dev/null && {
                download_ok=true
                rm -f "${tar_file}"
            }
        fi
    fi

    # Install the binary
    if [[ "${download_ok}" == true ]] && [[ -f "${tmp_file}" ]] && [[ -s "${tmp_file}" ]]; then
        mv "${tmp_file}" "${RATHOLE_PRO_DIR}/${BINARY_NAME}"
        chmod +x "${RATHOLE_PRO_DIR}/${BINARY_NAME}"
        print_success "Binary installed: ${RATHOLE_PRO_DIR}/${BINARY_NAME}"

        # Verify binary works
        if "${RATHOLE_PRO_DIR}/${BINARY_NAME}" --version &>/dev/null; then
            local installed_ver
            installed_ver=$("${RATHOLE_PRO_DIR}/${BINARY_NAME}" --version 2>/dev/null | awk '{print $NF}')
            print_success "Version: ${installed_ver}"
        fi

        # Add to PATH via symlink
        ln -sf "${RATHOLE_PRO_DIR}/${BINARY_NAME}" /usr/local/bin/${BINARY_NAME} 2>/dev/null
        print_info "Symlinked to /usr/local/bin/${BINARY_NAME}"
    elif [[ -x "${RATHOLE_PRO_DIR}/${BINARY_NAME}" ]]; then
        # Already installed from tar extraction
        chmod +x "${RATHOLE_PRO_DIR}/${BINARY_NAME}"
        ln -sf "${RATHOLE_PRO_DIR}/${BINARY_NAME}" /usr/local/bin/${BINARY_NAME} 2>/dev/null
        print_success "Binary installed: ${RATHOLE_PRO_DIR}/${BINARY_NAME}"
    else
        print_error "Download failed!"
        echo ""
        echo -e "  ${YELLOW}No release found for your architecture (${ARCH}).${NC}"
        echo -e "  ${YELLOW}Build from source:${NC}"
        echo ""
        echo "    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
        echo "    source ~/.cargo/env"
        echo "    git clone https://github.com/${GITHUB_REPO}.git"
        echo "    cd RatholePro && cargo build --release"
        echo "    sudo cp target/release/${BINARY_NAME} ${RATHOLE_PRO_DIR}/"
        echo "    sudo ln -sf ${RATHOLE_PRO_DIR}/${BINARY_NAME} /usr/local/bin/${BINARY_NAME}"
        echo ""
        rm -f "${tmp_file}" 2>/dev/null
        exit 1
    fi
}

full_install() {
    echo ""
    print_divider
    print_info "Starting Rathole Pro installation..."
    print_divider
    echo ""
    check_root
    detect_os
    detect_arch
    detect_ipv6
    echo ""
    install_dependencies
    download_binary
    echo ""
    print_success "Installation complete!"
    print_info "Run this script again to configure server/client."
    echo ""
}

# ─── Transport Selection ───────────────────────────────────────

select_transport() {
    echo ""
    echo -e "  ${BOLD}Select Transport Protocol:${NC}"
    echo -e "    ${GREEN}1)${NC} tcp   ${DIM}- Raw TCP (fastest, no encryption)${NC}"
    echo -e "    ${GREEN}2)${NC} tls   ${DIM}- TLS 1.3 (certificate-based encryption)${NC}"
    echo -e "    ${GREEN}3)${NC} noise ${DIM}- Noise Protocol (no certificates needed)${NC}"
    echo -e "    ${GREEN}4)${NC} ws    ${DIM}- WebSocket (bypass firewalls/CDN)${NC}"
    echo -e "    ${GREEN}5)${NC} wss   ${DIM}- WebSocket + TLS (secure + bypass)${NC}"
    echo -e "    ${GREEN}6)${NC} quic  ${DIM}- QUIC (UDP-based, low latency, built-in TLS)${NC}"
    echo ""
    read -rp "  Choice [1]: " transport_choice

    case "${transport_choice:-1}" in
        1) TRANSPORT="tcp" ;;
        2) TRANSPORT="tls" ;;
        3) TRANSPORT="noise" ;;
        4) TRANSPORT="ws" ;;
        5) TRANSPORT="wss" ;;
        6) TRANSPORT="quic" ;;
        *) TRANSPORT="tcp" ;;
    esac
    print_info "Transport: ${TRANSPORT}"
}

configure_transport_server() {
    local config_file="$1"

    cat >> "${config_file}" << EOF

[server.transport]
type = "${TRANSPORT}"
EOF

    case "${TRANSPORT}" in
        tls)
            echo ""
            print_step "TLS Configuration (Server)"
            read -rp "  Certificate PEM path: " cert_path
            read -rp "  Private key PEM path: " key_path

            if [[ ! -f "${cert_path}" ]]; then
                print_warning "Certificate file not found: ${cert_path}"
                print_info "You can generate a self-signed cert with:"
                echo "    openssl req -x509 -newkey rsa:4096 -keyout key.pem -out cert.pem -days 365 -nodes"
            fi

            cat >> "${config_file}" << EOF

[server.transport.tls]
trusted_root = "${cert_path}"
pkcs12 = "${key_path}"
EOF
            ;;
        noise)
            echo ""
            print_step "Noise Protocol Configuration (Server)"
            local private_key
            private_key=$(generate_noise_keypair)
            echo ""
            echo -e "  ${YELLOW}╔══ Generated Server Keypair ══╗${NC}"
            echo -e "  ${YELLOW}║${NC} Private Key (SERVER - keep secret):"
            echo -e "  ${YELLOW}║${NC}   ${BOLD}${private_key}${NC}"
            echo -e "  ${YELLOW}║${NC}"
            echo -e "  ${YELLOW}║${NC} ${DIM}Share the public key with clients after build.${NC}"
            echo -e "  ${YELLOW}║${NC} ${DIM}Use: rathole-pro --gen-key for proper keypair.${NC}"
            echo -e "  ${YELLOW}╚══════════════════════════════╝${NC}"
            echo ""

            cat >> "${config_file}" << EOF

[server.transport.noise]
pattern = "Noise_NK_25519_ChaChaPoly_BLAKE2s"
local_private_key = "${private_key}"
EOF
            ;;
        ws|wss)
            echo ""
            print_step "WebSocket Configuration (Server)"
            read -rp "  WebSocket path [/tunnel]: " ws_path
            ws_path="${ws_path:-/tunnel}"

            cat >> "${config_file}" << EOF

[server.transport.websocket]
path = "${ws_path}"
EOF
            if [[ "${TRANSPORT}" == "wss" ]]; then
                read -rp "  Certificate PEM path: " cert_path
                read -rp "  Private key PEM path: " key_path
                cat >> "${config_file}" << EOF

[server.transport.tls]
trusted_root = "${cert_path}"
pkcs12 = "${key_path}"
EOF
            fi
            ;;
        quic)
            echo ""
            print_step "QUIC Configuration (Server)"
            read -rp "  Certificate PEM path: " cert_path
            read -rp "  Private key PEM path: " key_path

            cat >> "${config_file}" << EOF

[server.transport.quic]
cert = "${cert_path}"
key = "${key_path}"
max_streams = 100
keep_alive = 15
EOF
            ;;
    esac
}

configure_transport_client() {
    local config_file="$1"
    local remote_addr="$2"

    cat >> "${config_file}" << EOF

[client.transport]
type = "${TRANSPORT}"
EOF

    case "${TRANSPORT}" in
        tls)
            echo ""
            print_step "TLS Configuration (Client)"
            local hostname
            hostname=$(echo "${remote_addr}" | cut -d: -f1)
            read -rp "  TLS hostname [${hostname}]: " tls_host
            tls_host="${tls_host:-${hostname}}"
            read -rp "  CA cert path (empty=system CAs): " ca_path

            cat >> "${config_file}" << EOF

[client.transport.tls]
hostname = "${tls_host}"
EOF
            if [[ -n "${ca_path}" ]]; then
                echo "trusted_root = \"${ca_path}\"" >> "${config_file}"
            fi
            ;;
        noise)
            echo ""
            print_step "Noise Protocol Configuration (Client)"
            read -rp "  Server public key (base64): " server_pub_key

            if [[ -z "${server_pub_key}" ]]; then
                print_error "Server public key is required for Noise transport!"
                return 1
            fi

            cat >> "${config_file}" << EOF

[client.transport.noise]
pattern = "Noise_NK_25519_ChaChaPoly_BLAKE2s"
remote_public_key = "${server_pub_key}"
EOF
            ;;
        ws|wss)
            echo ""
            print_step "WebSocket Configuration (Client)"
            read -rp "  WebSocket path [/tunnel]: " ws_path
            ws_path="${ws_path:-/tunnel}"

            cat >> "${config_file}" << EOF

[client.transport.websocket]
path = "${ws_path}"
EOF
            ;;
        quic)
            echo ""
            print_step "QUIC Configuration (Client)"
            read -rp "  CA cert path (empty=system CAs): " ca_path

            cat >> "${config_file}" << EOF

[client.transport.quic]
max_streams = 100
keep_alive = 15
EOF
            if [[ -n "${ca_path}" ]]; then
                echo "ca = \"${ca_path}\"" >> "${config_file}"
            fi
            ;;
    esac
}

# ─── Server Configuration ──────────────────────────────────────

configure_server() {
    echo ""
    print_divider
    echo -e "  ${BOLD}Server Configuration${NC}"
    print_divider
    echo ""

    # Basic settings
    read -rp "  Bind port [2333]: " server_port
    server_port="${server_port:-2333}"

    # IPv6 support
    read -rp "  Bind on IPv6 too? (y/n) [n]: " use_ipv6
    if [[ "${use_ipv6}" =~ ^[Yy]$ ]]; then
        bind_addr="[::]:${server_port}"
    else
        bind_addr="0.0.0.0:${server_port}"
    fi

    # Token
    read -rp "  Default token (empty=auto-generate): " default_token
    if [[ -z "${default_token}" ]]; then
        default_token=$(generate_token)
        echo -e "  ${GREEN}Generated token:${NC} ${default_token}"
    fi

    read -rp "  Heartbeat interval (seconds) [30]: " heartbeat
    heartbeat="${heartbeat:-30}"

    # Transport
    select_transport

    # Write config
    mkdir -p "${CONFIG_DIR}"
    local config_file="${CONFIG_DIR}/server.toml"

    cat > "${config_file}" << EOF
# Rathole Pro Server Configuration
# Generated: $(date -Iseconds)
# Transport: ${TRANSPORT}

[server]
bind_addr = "${bind_addr}"
default_token = "${default_token}"
heartbeat_interval = ${heartbeat}
prefer_ipv6 = ${use_ipv6:-false}
EOF

    # Transport config
    configure_transport_server "${config_file}"

    # Services
    echo ""
    print_divider
    echo -e "  ${BOLD}Add Services${NC}"
    print_divider
    add_services_server "${config_file}"

    echo ""
    print_success "Server config saved: ${config_file}"
    echo ""
    echo -e "  ${YELLOW}╔══ Important ══╗${NC}"
    echo -e "  ${YELLOW}║${NC} Token: ${BOLD}${default_token}${NC}"
    echo -e "  ${YELLOW}║${NC} Save this! Clients need it to connect."
    echo -e "  ${YELLOW}╚═══════════════╝${NC}"
    echo ""

    # Create systemd service
    create_systemd_service "server"
}

add_services_server() {
    local config_file="$1"
    local add_more="y"

    while [[ "${add_more}" =~ ^[Yy]$ ]]; do
        echo ""
        read -rp "  Service name: " svc_name
        if [[ -z "${svc_name}" ]]; then
            print_error "Service name cannot be empty"
            continue
        fi

        echo -e "    Service type:"
        echo -e "      ${GREEN}1)${NC} tcp ${DIM}(default)${NC}"
        echo -e "      ${GREEN}2)${NC} udp ${DIM}(games, VPN, DNS)${NC}"
        echo -e "      ${GREEN}3)${NC} http ${DIM}(web proxy)${NC}"
        read -rp "    Type [1]: " svc_type_choice
        case "${svc_type_choice:-1}" in
            1) svc_type="tcp" ;;
            2) svc_type="udp" ;;
            3) svc_type="http" ;;
            *) svc_type="tcp" ;;
        esac

        read -rp "  Bind port: " svc_port
        if [[ -z "${svc_port}" ]]; then
            print_error "Port cannot be empty"
            continue
        fi

        read -rp "  Max mux streams [8]: " svc_mux
        svc_mux="${svc_mux:-8}"

        read -rp "  Custom token (empty=use default): " svc_token

        cat >> "${config_file}" << EOF

[server.services.${svc_name}]
type = "${svc_type}"
bind_addr = "0.0.0.0:${svc_port}"
max_mux_streams = ${svc_mux}
EOF

        if [[ -n "${svc_token}" ]]; then
            echo "token = \"${svc_token}\"" >> "${config_file}"
        fi

        print_success "Service '${svc_name}' added (${svc_type} on port ${svc_port})"
        echo ""
        read -rp "  Add another service? (y/n): " add_more
    done
}

# ─── Client Configuration ──────────────────────────────────────

configure_client() {
    echo ""
    print_divider
    echo -e "  ${BOLD}Client Configuration${NC}"
    print_divider
    echo ""

    # Server address
    read -rp "  Server address (ip:port or [ipv6]:port): " remote_addr
    if [[ -z "${remote_addr}" ]]; then
        print_error "Server address is required!"
        return 1
    fi

    # Token
    read -rp "  Default token: " default_token
    if [[ -z "${default_token}" ]]; then
        print_error "Token is required!"
        return 1
    fi

    read -rp "  Retry interval (seconds) [3]: " retry
    retry="${retry:-3}"

    read -rp "  Mux connections per service [4]: " mux_conn
    mux_conn="${mux_conn:-4}"

    # IPv6
    read -rp "  Prefer IPv6? (y/n) [n]: " prefer_ipv6
    [[ "${prefer_ipv6}" =~ ^[Yy]$ ]] && prefer_ipv6="true" || prefer_ipv6="false"

    # Transport
    select_transport

    # Write config
    mkdir -p "${CONFIG_DIR}"
    local config_file="${CONFIG_DIR}/client.toml"

    cat > "${config_file}" << EOF
# Rathole Pro Client Configuration
# Generated: $(date -Iseconds)
# Transport: ${TRANSPORT}

[client]
remote_addr = "${remote_addr}"
default_token = "${default_token}"
heartbeat_timeout = 40
retry_interval = ${retry}
mux_connections = ${mux_conn}
prefer_ipv6 = ${prefer_ipv6}
EOF

    # Transport config
    configure_transport_client "${config_file}" "${remote_addr}"

    # HTTP proxy (optional)
    echo ""
    read -rp "  Use HTTP/SOCKS5 proxy? (y/n) [n]: " use_proxy
    if [[ "${use_proxy}" =~ ^[Yy]$ ]]; then
        read -rp "  Proxy URL (http://host:port or socks5://host:port): " proxy_url
        if [[ -n "${proxy_url}" ]]; then
            cat >> "${config_file}" << EOF

[client.http_proxy]
url = "${proxy_url}"
EOF
            read -rp "  Proxy username (empty=none): " proxy_user
            if [[ -n "${proxy_user}" ]]; then
                read -rsp "  Proxy password: " proxy_pass
                echo ""
                cat >> "${config_file}" << EOF
username = "${proxy_user}"
password = "${proxy_pass}"
EOF
            fi
        fi
    fi

    # Services
    echo ""
    print_divider
    echo -e "  ${BOLD}Add Services${NC}"
    print_divider
    add_services_client "${config_file}"

    echo ""
    print_success "Client config saved: ${config_file}"
    echo ""

    # Create systemd service
    create_systemd_service "client"
}

add_services_client() {
    local config_file="$1"
    local add_more="y"

    while [[ "${add_more}" =~ ^[Yy]$ ]]; do
        echo ""
        read -rp "  Service name (must match server): " svc_name
        if [[ -z "${svc_name}" ]]; then
            print_error "Service name cannot be empty"
            continue
        fi

        echo -e "    Service type:"
        echo -e "      ${GREEN}1)${NC} tcp ${DIM}(default)${NC}"
        echo -e "      ${GREEN}2)${NC} udp"
        echo -e "      ${GREEN}3)${NC} http"
        read -rp "    Type [1]: " svc_type_choice
        case "${svc_type_choice:-1}" in
            1) svc_type="tcp" ;;
            2) svc_type="udp" ;;
            3) svc_type="http" ;;
            *) svc_type="tcp" ;;
        esac

        read -rp "  Local address (e.g., 127.0.0.1:22): " local_addr
        if [[ -z "${local_addr}" ]]; then
            print_error "Local address is required"
            continue
        fi

        read -rp "  Mux streams [4]: " svc_mux
        svc_mux="${svc_mux:-4}"

        # Load balancing
        read -rp "  Enable load balancing? (y/n) [n]: " use_lb
        local backends_config=""
        local lb_config=""

        if [[ "${use_lb}" =~ ^[Yy]$ ]]; then
            echo -e "    Load balance strategy:"
            echo -e "      ${GREEN}1)${NC} round_robin ${DIM}(default)${NC}"
            echo -e "      ${GREEN}2)${NC} random"
            echo -e "      ${GREEN}3)${NC} least_conn"
            read -rp "    Strategy [1]: " lb_choice
            case "${lb_choice:-1}" in
                1) lb_strategy="round_robin" ;;
                2) lb_strategy="random" ;;
                3) lb_strategy="least_conn" ;;
                *) lb_strategy="round_robin" ;;
            esac

            read -rp "  Health check interval (0=disable) [10]: " hc_interval
            hc_interval="${hc_interval:-10}"

            echo "  Enter backend addresses (one per line, empty to finish):"
            local backends=()
            backends+=("${local_addr}")
            while true; do
                read -rp "    Backend: " backend
                [[ -z "${backend}" ]] && break
                backends+=("${backend}")
            done

            backends_config="backends = [$(printf '"%s", ' "${backends[@]}" | sed 's/, $//')]"
            lb_config="
[client.services.${svc_name}.load_balance]
strategy = \"${lb_strategy}\"
health_check_interval = ${hc_interval}"
        fi

        read -rp "  Custom token (empty=use default): " svc_token

        cat >> "${config_file}" << EOF

[client.services.${svc_name}]
type = "${svc_type}"
local_addr = "${local_addr}"
mux_streams = ${svc_mux}
EOF

        if [[ -n "${backends_config}" ]]; then
            echo "${backends_config}" >> "${config_file}"
        fi
        if [[ -n "${svc_token}" ]]; then
            echo "token = \"${svc_token}\"" >> "${config_file}"
        fi
        if [[ -n "${lb_config}" ]]; then
            echo "${lb_config}" >> "${config_file}"
        fi

        print_success "Service '${svc_name}' added (${svc_type} -> ${local_addr})"
        echo ""
        read -rp "  Add another service? (y/n): " add_more
    done
}

# ─── Systemd Service Management ────────────────────────────────

create_systemd_service() {
    local mode="$1"
    local config_file="${CONFIG_DIR}/${mode}.toml"
    local service_name="${SERVICE_PREFIX}-${mode}"

    cat > "/etc/systemd/system/${service_name}.service" << EOF
[Unit]
Description=Rathole Pro Tunnel (${mode})
Documentation=https://github.com/iPmartNetwork/RatholePro
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=root
ExecStart=${RATHOLE_PRO_DIR}/${BINARY_NAME} ${config_file}
Restart=always
RestartSec=5
LimitNOFILE=1048576
LimitNPROC=512
StandardOutput=journal
StandardError=journal
SyslogIdentifier=${service_name}

# Security hardening
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=${LOG_DIR}
PrivateTmp=true

[Install]
WantedBy=multi-user.target
EOF

    systemctl daemon-reload
    systemctl enable "${service_name}" >/dev/null 2>&1
    print_success "Systemd service created: ${service_name}"
    print_info "Start with: systemctl start ${service_name}"
}

start_service() {
    echo ""
    echo -e "  ${BOLD}Start Service${NC}"
    echo -e "    ${GREEN}1)${NC} Server"
    echo -e "    ${GREEN}2)${NC} Client"
    echo -e "    ${GREEN}3)${NC} Both"
    read -rp "  Choice: " choice

    case "${choice}" in
        1)
            systemctl start "${SERVICE_PREFIX}-server" && \
                print_success "Server started" || \
                print_error "Failed to start server"
            ;;
        2)
            systemctl start "${SERVICE_PREFIX}-client" && \
                print_success "Client started" || \
                print_error "Failed to start client"
            ;;
        3)
            systemctl start "${SERVICE_PREFIX}-server" 2>/dev/null && print_success "Server started"
            systemctl start "${SERVICE_PREFIX}-client" 2>/dev/null && print_success "Client started"
            ;;
        *) print_error "Invalid choice" ;;
    esac
}

stop_service() {
    echo ""
    echo -e "  ${BOLD}Stop Service${NC}"
    echo -e "    ${GREEN}1)${NC} Server"
    echo -e "    ${GREEN}2)${NC} Client"
    echo -e "    ${GREEN}3)${NC} Both"
    read -rp "  Choice: " choice

    case "${choice}" in
        1) systemctl stop "${SERVICE_PREFIX}-server" 2>/dev/null && print_success "Server stopped" ;;
        2) systemctl stop "${SERVICE_PREFIX}-client" 2>/dev/null && print_success "Client stopped" ;;
        3)
            systemctl stop "${SERVICE_PREFIX}-server" 2>/dev/null && print_success "Server stopped"
            systemctl stop "${SERVICE_PREFIX}-client" 2>/dev/null && print_success "Client stopped"
            ;;
        *) print_error "Invalid choice" ;;
    esac
}

restart_service() {
    echo ""
    echo -e "  ${BOLD}Restart Service${NC}"
    echo -e "    ${GREEN}1)${NC} Server"
    echo -e "    ${GREEN}2)${NC} Client"
    echo -e "    ${GREEN}3)${NC} Both"
    read -rp "  Choice: " choice

    case "${choice}" in
        1) systemctl restart "${SERVICE_PREFIX}-server" && print_success "Server restarted" ;;
        2) systemctl restart "${SERVICE_PREFIX}-client" && print_success "Client restarted" ;;
        3)
            systemctl restart "${SERVICE_PREFIX}-server" 2>/dev/null && print_success "Server restarted"
            systemctl restart "${SERVICE_PREFIX}-client" 2>/dev/null && print_success "Client restarted"
            ;;
        *) print_error "Invalid choice" ;;
    esac
}

show_status() {
    echo ""
    print_divider
    echo -e "  ${BOLD}Rathole Pro Status${NC}"
    print_divider
    echo ""

    # Server status
    if systemctl is-active "${SERVICE_PREFIX}-server" &>/dev/null; then
        echo -e "  Server:  ${GREEN}● Running${NC}"
        local server_pid
        server_pid=$(systemctl show -p MainPID "${SERVICE_PREFIX}-server" 2>/dev/null | cut -d= -f2)
        if [[ "${server_pid}" != "0" ]] && [[ -n "${server_pid}" ]]; then
            echo -e "           PID: ${server_pid}"
        fi
    elif systemctl is-enabled "${SERVICE_PREFIX}-server" &>/dev/null 2>&1; then
        echo -e "  Server:  ${YELLOW}● Stopped (enabled)${NC}"
    else
        echo -e "  Server:  ${DIM}○ Not configured${NC}"
    fi

    # Client status
    if systemctl is-active "${SERVICE_PREFIX}-client" &>/dev/null; then
        echo -e "  Client:  ${GREEN}● Running${NC}"
        local client_pid
        client_pid=$(systemctl show -p MainPID "${SERVICE_PREFIX}-client" 2>/dev/null | cut -d= -f2)
        if [[ "${client_pid}" != "0" ]] && [[ -n "${client_pid}" ]]; then
            echo -e "           PID: ${client_pid}"
        fi
    elif systemctl is-enabled "${SERVICE_PREFIX}-client" &>/dev/null 2>&1; then
        echo -e "  Client:  ${YELLOW}● Stopped (enabled)${NC}"
    else
        echo -e "  Client:  ${DIM}○ Not configured${NC}"
    fi

    echo ""

    # Config files
    if [[ -f "${CONFIG_DIR}/server.toml" ]]; then
        local transport
        transport=$(grep -m1 'type' "${CONFIG_DIR}/server.toml" 2>/dev/null | head -1 | cut -d'"' -f2)
        echo -e "  Server config:  ${GREEN}${CONFIG_DIR}/server.toml${NC} [${transport:-tcp}]"
    fi
    if [[ -f "${CONFIG_DIR}/client.toml" ]]; then
        local transport
        transport=$(grep -m1 'type' "${CONFIG_DIR}/client.toml" 2>/dev/null | head -1 | cut -d'"' -f2)
        echo -e "  Client config:  ${GREEN}${CONFIG_DIR}/client.toml${NC} [${transport:-tcp}]"
    fi

    # Binary info
    if [[ -x "${RATHOLE_PRO_DIR}/${BINARY_NAME}" ]]; then
        echo -e "  Binary:         ${GREEN}${RATHOLE_PRO_DIR}/${BINARY_NAME}${NC}"
    else
        echo -e "  Binary:         ${RED}Not installed${NC}"
    fi

    echo ""
}

view_logs() {
    echo ""
    echo -e "  ${BOLD}View Logs${NC}"
    echo -e "    ${GREEN}1)${NC} Server logs"
    echo -e "    ${GREEN}2)${NC} Client logs"
    echo -e "    ${GREEN}3)${NC} Live server logs (follow)"
    echo -e "    ${GREEN}4)${NC} Live client logs (follow)"
    read -rp "  Choice: " choice

    case "${choice}" in
        1) journalctl -u "${SERVICE_PREFIX}-server" --no-pager -n 50 ;;
        2) journalctl -u "${SERVICE_PREFIX}-client" --no-pager -n 50 ;;
        3) journalctl -u "${SERVICE_PREFIX}-server" -f ;;
        4) journalctl -u "${SERVICE_PREFIX}-client" -f ;;
        *) print_error "Invalid choice" ;;
    esac
}

view_config() {
    echo ""
    echo -e "  ${BOLD}View Configuration${NC}"
    echo -e "    ${GREEN}1)${NC} Server config"
    echo -e "    ${GREEN}2)${NC} Client config"
    read -rp "  Choice: " choice

    case "${choice}" in
        1)
            if [[ -f "${CONFIG_DIR}/server.toml" ]]; then
                echo ""
                print_divider
                cat "${CONFIG_DIR}/server.toml"
                print_divider
            else
                print_warning "No server config found"
            fi
            ;;
        2)
            if [[ -f "${CONFIG_DIR}/client.toml" ]]; then
                echo ""
                print_divider
                cat "${CONFIG_DIR}/client.toml"
                print_divider
            else
                print_warning "No client config found"
            fi
            ;;
        *) print_error "Invalid choice" ;;
    esac
}

# ─── Uninstall ─────────────────────────────────────────────────

uninstall() {
    echo ""
    print_divider
    echo -e "  ${RED}${BOLD}Uninstall Rathole Pro${NC}"
    print_divider
    echo ""
    print_warning "This will remove:"
    echo "    - Binary: ${RATHOLE_PRO_DIR}/"
    echo "    - Config: ${CONFIG_DIR}/"
    echo "    - Logs:   ${LOG_DIR}/"
    echo "    - Systemd services"
    echo ""

    if ! confirm_action "Proceed with uninstall?"; then
        print_info "Cancelled."
        return
    fi

    # Stop services
    systemctl stop "${SERVICE_PREFIX}-server" 2>/dev/null
    systemctl stop "${SERVICE_PREFIX}-client" 2>/dev/null
    systemctl disable "${SERVICE_PREFIX}-server" 2>/dev/null
    systemctl disable "${SERVICE_PREFIX}-client" 2>/dev/null

    # Remove service files
    rm -f "/etc/systemd/system/${SERVICE_PREFIX}-server.service"
    rm -f "/etc/systemd/system/${SERVICE_PREFIX}-client.service"
    systemctl daemon-reload

    # Remove files
    rm -rf "${RATHOLE_PRO_DIR}"
    rm -rf "${CONFIG_DIR}"
    rm -rf "${LOG_DIR}"

    echo ""
    print_success "Rathole Pro completely uninstalled."
}

# ─── Update ────────────────────────────────────────────────────

update_binary() {
    echo ""
    print_divider
    echo -e "  ${BOLD}Update Rathole Pro${NC}"
    print_divider
    echo ""

    detect_arch

    local current_version="unknown"
    if [[ -x "${RATHOLE_PRO_DIR}/${BINARY_NAME}" ]]; then
        current_version=$(${RATHOLE_PRO_DIR}/${BINARY_NAME} --version 2>/dev/null | awk '{print $NF}' || echo "unknown")
    fi
    print_info "Current version: ${current_version}"
    print_info "Latest version: ${APP_VERSION}"

    if ! confirm_action "Download and install v${APP_VERSION}?"; then
        return
    fi

    # Stop services temporarily
    local server_was_running=false
    local client_was_running=false

    if systemctl is-active "${SERVICE_PREFIX}-server" &>/dev/null; then
        server_was_running=true
        systemctl stop "${SERVICE_PREFIX}-server"
    fi
    if systemctl is-active "${SERVICE_PREFIX}-client" &>/dev/null; then
        client_was_running=true
        systemctl stop "${SERVICE_PREFIX}-client"
    fi

    # Download new binary
    download_binary

    # Restart services
    if [[ "${server_was_running}" == true ]]; then
        systemctl start "${SERVICE_PREFIX}-server" && print_success "Server restarted"
    fi
    if [[ "${client_was_running}" == true ]]; then
        systemctl start "${SERVICE_PREFIX}-client" && print_success "Client restarted"
    fi

    print_success "Update complete!"
}

# ─── Main Menu ─────────────────────────────────────────────────

main_menu() {
    while true; do
        print_banner
        echo -e "  ${BOLD}Main Menu${NC}"
        echo ""
        echo -e "    ${GREEN} 1)${NC} Install Rathole Pro"
        echo -e "    ${GREEN} 2)${NC} Configure Server"
        echo -e "    ${GREEN} 3)${NC} Configure Client"
        echo ""
        echo -e "    ${GREEN} 4)${NC} Start Service"
        echo -e "    ${GREEN} 5)${NC} Stop Service"
        echo -e "    ${GREEN} 6)${NC} Restart Service"
        echo -e "    ${GREEN} 7)${NC} View Status"
        echo ""
        echo -e "    ${GREEN} 8)${NC} View Logs"
        echo -e "    ${GREEN} 9)${NC} View Config"
        echo -e "    ${GREEN}10)${NC} Update Binary"
        echo -e "    ${GREEN}11)${NC} Uninstall"
        echo ""
        echo -e "    ${RED} 0)${NC} Exit"
        echo ""
        read -rp "  Select option: " choice

        case "${choice}" in
            1)  full_install ;;
            2)  check_root; configure_server ;;
            3)  check_root; configure_client ;;
            4)  check_root; start_service ;;
            5)  check_root; stop_service ;;
            6)  check_root; restart_service ;;
            7)  show_status ;;
            8)  view_logs ;;
            9)  view_config ;;
            10) check_root; update_binary ;;
            11) check_root; uninstall ;;
            0)
                echo ""
                echo -e "  ${DIM}Goodbye!${NC}"
                echo ""
                exit 0
                ;;
            *)  print_error "Invalid option" ;;
        esac

        echo ""
        read -rp "  Press Enter to continue..." _
    done
}

# ─── Entry Point ───────────────────────────────────────────────
main_menu
