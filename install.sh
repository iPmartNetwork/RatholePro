#!/bin/bash

# ═══════════════════════════════════════════════════════════════
# RatholePro - Professional Installation & Management Script
# Next-generation reverse proxy tunnel with yamux multiplexing,
# multi-transport, load balancing, P2P, and UDP support.
# Developer: iPmart Network (Ali Hassanzadeh)
# Version: 0.4.0 (Go Core)
# ═══════════════════════════════════════════════════════════════

# Auto-download and re-exec if running from pipe (bash <(curl ...))
if [[ ! -t 0 ]] && [[ -z "${RATHOLE_REEXEC:-}" ]]; then
    tmp="/tmp/rathole-pro-install.sh"
    curl -fsSL -o "$tmp" "https://raw.githubusercontent.com/iPmartNetwork/RatholePro/master/install.sh" 2>/dev/null || \
    wget -q -O "$tmp" "https://raw.githubusercontent.com/iPmartNetwork/RatholePro/master/install.sh" 2>/dev/null
    export RATHOLE_REEXEC=1
    exec bash "$tmp"
fi

set -eo pipefail

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
readonly APP_VERSION="0.4.1"
readonly SERVICE_PREFIX="rathole-pro"
readonly AUTHOR="iPmart Network (Ali Hassanzadeh)"

# ─── Helper Functions ──────────────────────────────────────────

print_banner() {
    clear
    echo -e "${CYAN}"
    echo "╔═══════════════════════════════════════════════════════════╗"
    echo "║                                                           ║"
    echo "║              Rathole Pro v${APP_VERSION} (Go + Yamux)           ║"
    echo "║     High-Performance Tunnel + Multi-Protocol + Mux        ║"
    echo "║                                                           ║"
    echo "║  Transports: TCP │ TLS (auto-cert) │ Noise │ WebSocket   ║"
    echo "║  Protocols:  TCP │ UDP                                    ║"
    echo "║  Features:   Yamux Mux │ Load Balance │ P2P │ IPv6        ║"
    echo "║                                                           ║"
    echo "║  Developer: iPmart Network (Ali Hassanzadeh)              ║"
    echo "║                                                           ║"
    echo "╚═══════════════════════════════════════════════════════════╝"
    echo -e "${NC}"

    if [[ -x "${RATHOLE_PRO_DIR}/${BINARY_NAME}" ]]; then
        local ver
        ver=$("${RATHOLE_PRO_DIR}/${BINARY_NAME}" --version 2>/dev/null | head -1)
        echo -e "  ${GREEN}● Binary: ${ver}${NC}"
    else
        echo -e "  ${RED}● Binary: NOT INSTALLED${NC}"
    fi
    echo ""
}

print_info()    { echo -e "  ${BLUE}[INFO]${NC} $1"; }
print_success() { echo -e "  ${GREEN}[✓]${NC} $1"; }
print_error()   { echo -e "  ${RED}[✗]${NC} $1"; }
print_warning() { echo -e "  ${YELLOW}[!]${NC} $1"; }
print_step()    { echo -e "  ${MAGENTA}[→]${NC} $1"; }
print_divider() { echo -e "  ${DIM}───────────────────────────────────────────${NC}"; }

confirm_action() {
    local prompt="${1:-Are you sure?}"
    echo ""
    echo -n "  ${prompt} (y/n): "; read -r answer
    [[ "$answer" =~ ^[Yy]$ ]]
}

check_root() {
    if [[ $EUID -ne 0 ]]; then
        print_error "This script must be run as root (use sudo)"
        exit 1
    fi
}

detect_os() {
    OS="linux"
    OS_NAME="Linux"
    if [[ -f /etc/os-release ]]; then
        OS=$(grep -m1 '^ID=' /etc/os-release 2>/dev/null | cut -d= -f2 | tr -d '"') || true
        OS_NAME=$(grep -m1 '^PRETTY_NAME=' /etc/os-release 2>/dev/null | cut -d= -f2 | tr -d '"') || true
        OS="${OS:-linux}"
        OS_NAME="${OS_NAME:-Linux}"
    fi
    print_info "OS: ${OS_NAME}"
}

detect_arch() {
    ARCH=$(uname -m)
    case "${ARCH}" in
        x86_64|amd64)   ARCH="amd64" ;;
        aarch64|arm64)  ARCH="arm64" ;;
        armv7l|armhf)   ARCH="armv7" ;;
        i686|i386)      ARCH="386" ;;
        mips64)         ARCH="mips64" ;;
        mips)           ARCH="mips" ;;
        *)
            print_error "Unsupported architecture: ${ARCH}"
            exit 1
            ;;
    esac
    print_info "Architecture: ${ARCH}"
}

generate_token() {
    if command -v openssl &>/dev/null; then
        openssl rand -hex 16
    else
        tr -dc 'a-f0-9' </dev/urandom | head -c 32
    fi
}

# ─── Installation ──────────────────────────────────────────────

install_dependencies() {
    print_step "Installing dependencies..."
    if command -v apt-get &>/dev/null; then
        apt-get update -qq >/dev/null 2>&1
        apt-get install -y -qq curl wget tar jq >/dev/null 2>&1
    elif command -v dnf &>/dev/null; then
        dnf install -y -q curl wget tar jq >/dev/null 2>&1
    elif command -v yum &>/dev/null; then
        yum install -y -q curl wget tar jq >/dev/null 2>&1
    elif command -v pacman &>/dev/null; then
        pacman -Sy --noconfirm --quiet curl wget tar jq >/dev/null 2>&1
    elif command -v apk &>/dev/null; then
        apk add --quiet curl wget tar jq >/dev/null 2>&1
    fi
    print_success "Dependencies ready"
}

download_binary() {
    print_step "Downloading RatholePro (Go) for linux/${ARCH}..."
    mkdir -p "${RATHOLE_PRO_DIR}"
    mkdir -p "${CONFIG_DIR}"
    mkdir -p "${LOG_DIR}"

    rm -f "${RATHOLE_PRO_DIR}/${BINARY_NAME}" 2>/dev/null

    local asset_name="${BINARY_NAME}-linux-${ARCH}"

    # Try GitHub releases
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

    local download_url="https://github.com/${GITHUB_REPO}/releases/download/v${latest_version}/${asset_name}"
    local tmp_file="/tmp/${asset_name}"

    print_info "URL: ${download_url}"

    local download_ok=false
    if command -v curl &>/dev/null; then
        curl -fsSL -o "${tmp_file}" "${download_url}" 2>/dev/null && download_ok=true
    elif command -v wget &>/dev/null; then
        wget -q -O "${tmp_file}" "${download_url}" 2>/dev/null && download_ok=true
    fi

    # Try .tar.gz
    if [[ "${download_ok}" != true ]]; then
        local tar_url="${download_url}.tar.gz"
        local tar_file="/tmp/${asset_name}.tar.gz"
        print_info "Trying archive format..."
        if command -v curl &>/dev/null; then
            curl -fsSL -o "${tar_file}" "${tar_url}" 2>/dev/null
        elif command -v wget &>/dev/null; then
            wget -q -O "${tar_file}" "${tar_url}" 2>/dev/null
        fi
        if [[ -f "${tar_file}" ]] && [[ -s "${tar_file}" ]]; then
            tar -xzf "${tar_file}" -C "${RATHOLE_PRO_DIR}/" 2>/dev/null && download_ok=true
            rm -f "${tar_file}"
        fi
    fi

    if [[ "${download_ok}" == true ]] && [[ -f "${tmp_file}" ]] && [[ -s "${tmp_file}" ]]; then
        mv "${tmp_file}" "${RATHOLE_PRO_DIR}/${BINARY_NAME}"
        chmod +x "${RATHOLE_PRO_DIR}/${BINARY_NAME}"
        ln -sf "${RATHOLE_PRO_DIR}/${BINARY_NAME}" /usr/local/bin/${BINARY_NAME} 2>/dev/null
        print_success "Binary installed: ${RATHOLE_PRO_DIR}/${BINARY_NAME}"
        if "${RATHOLE_PRO_DIR}/${BINARY_NAME}" --version &>/dev/null; then
            print_success "Verified: $("${RATHOLE_PRO_DIR}/${BINARY_NAME}" --version 2>/dev/null)"
        fi
    elif [[ -x "${RATHOLE_PRO_DIR}/${BINARY_NAME}" ]]; then
        chmod +x "${RATHOLE_PRO_DIR}/${BINARY_NAME}"
        ln -sf "${RATHOLE_PRO_DIR}/${BINARY_NAME}" /usr/local/bin/${BINARY_NAME} 2>/dev/null
        print_success "Binary installed from archive"
    else
        print_error "Download failed!"
        echo ""
        echo -e "  ${YELLOW}Build from source (requires Go 1.22+):${NC}"
        echo ""
        echo "    git clone https://github.com/${GITHUB_REPO}.git"
        echo "    cd RatholePro/go-core"
        echo "    go build -o ${BINARY_NAME} ."
        echo "    sudo cp ${BINARY_NAME} ${RATHOLE_PRO_DIR}/"
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
    echo -e "  ${BOLD}Select Transport:${NC}"
    echo -e "    ${GREEN}1)${NC} tcp        ${DIM}- Raw TCP (fastest, no encryption)${NC}"
    echo -e "    ${GREEN}2)${NC} tls (auto) ${DIM}- TLS with auto-generated cert (no domain needed!)${NC}"
    echo -e "    ${GREEN}3)${NC} tls (manual) ${DIM}- TLS with your own cert${NC}"
    echo -e "    ${GREEN}4)${NC} noise      ${DIM}- Noise Protocol (no certificates needed)${NC}"
    echo -e "    ${GREEN}5)${NC} ws         ${DIM}- WebSocket (bypass firewalls)${NC}"
    echo ""
    echo -n "  Choice [1]: "; read -r transport_choice

    case "${transport_choice:-1}" in
        1) TRANSPORT="tcp" ;;
        2) TRANSPORT="tls_auto" ;;
        3) TRANSPORT="tls" ;;
        4) TRANSPORT="noise" ;;
        5) TRANSPORT="ws" ;;
        *) TRANSPORT="tcp" ;;
    esac
    print_info "Transport: ${TRANSPORT}"
}

configure_transport_server() {
    local config_file="$1"

    case "${TRANSPORT}" in
        tcp)
            # No transport section needed
            ;;
        tls_auto)
            cat >> "${config_file}" << EOF

[server.transport]
type = "tls"

[server.transport.tls]
auto_cert = true
cert_dir = "/etc/rathole-pro/certs"
EOF
            print_success "TLS auto-cert enabled (no domain needed, cert auto-generated)"
            ;;
        tls)
            echo ""
            print_step "TLS Configuration (Server)"
            echo -n "  Certificate PEM path: "; read -r cert_path
            echo -n "  Private key PEM path: "; read -r key_path

            if [[ ! -f "${cert_path}" ]]; then
                print_warning "Cert not found: ${cert_path}"
            fi

            cat >> "${config_file}" << EOF

[server.transport]
type = "tls"

[server.transport.tls]
trusted_root = "${cert_path}"
pkcs12 = "${key_path}"
EOF
            ;;
        noise)
            echo ""
            print_step "Generating Noise keypair..."
            if [[ -x "${RATHOLE_PRO_DIR}/${BINARY_NAME}" ]]; then
                echo ""
                "${RATHOLE_PRO_DIR}/${BINARY_NAME}" --gen-key
                echo ""
                echo -n "  Paste the Private Key here: "; read -r private_key
            else
                private_key=$(head -c 32 /dev/urandom | base64)
                echo -e "  ${YELLOW}Generated Private Key: ${private_key}${NC}"
            fi

            cat >> "${config_file}" << EOF

[server.transport]
type = "noise"

[server.transport.noise]
pattern = "Noise_NK_25519_ChaChaPoly_BLAKE2s"
local_private_key = "${private_key}"
EOF
            ;;
        ws)
            echo ""
            echo -n "  WebSocket path [/tunnel]: "; read -r ws_path
            ws_path="${ws_path:-/tunnel}"

            cat >> "${config_file}" << EOF

[server.transport]
type = "ws"

[server.transport.websocket]
path = "${ws_path}"
EOF
            ;;
    esac
}

configure_transport_client() {
    local config_file="$1"

    case "${TRANSPORT}" in
        tcp)
            ;;
        tls_auto)
            # Client just uses TLS with skip verify (no cert needed)
            cat >> "${config_file}" << EOF

[client.transport]
type = "tls"

[client.transport.tls]
EOF
            print_success "TLS enabled (auto-verify skip for self-signed server cert)"
            ;;
        tls)
            echo ""
            print_step "TLS Configuration (Client)"
            echo -n "  CA cert path (empty=skip verify): "; read -r ca_path

            cat >> "${config_file}" << EOF

[client.transport]
type = "tls"

[client.transport.tls]
EOF
            if [[ -n "${ca_path}" ]]; then
                echo "trusted_root = \"${ca_path}\"" >> "${config_file}"
            fi
            ;;
        noise)
            echo ""
            print_step "Noise Configuration (Client)"
            echo -n "  Server public key (base64): "; read -r server_pub_key
            if [[ -z "${server_pub_key}" ]]; then
                print_error "Server public key is required!"
                return 1
            fi

            cat >> "${config_file}" << EOF

[client.transport]
type = "noise"

[client.transport.noise]
pattern = "Noise_NK_25519_ChaChaPoly_BLAKE2s"
remote_public_key = "${server_pub_key}"
EOF
            ;;
        ws)
            echo ""
            echo -n "  WebSocket path [/tunnel]: "; read -r ws_path
            ws_path="${ws_path:-/tunnel}"

            cat >> "${config_file}" << EOF

[client.transport]
type = "ws"

[client.transport.websocket]
path = "${ws_path}"
EOF
            ;;
    esac
}

# ─── Systemd Service ───────────────────────────────────────────

create_systemd_service() {
    local mode="$1"
    local config_file="${CONFIG_DIR}/${mode}.toml"
    local service_name="${SERVICE_PREFIX}-${mode}"

    cat > "/etc/systemd/system/${service_name}.service" << EOF
[Unit]
Description=RatholePro Tunnel (${mode})
Documentation=https://github.com/${GITHUB_REPO}
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
NoNewPrivileges=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=${LOG_DIR} ${CONFIG_DIR}
PrivateTmp=true

[Install]
WantedBy=multi-user.target
EOF

    systemctl daemon-reload
    systemctl enable "${service_name}" >/dev/null 2>&1
    print_success "Systemd service: ${service_name}"
}

# ─── IRAN Server Configuration ─────────────────────────────────

configure_iran() {
    echo ""
    print_divider
    echo -e "  ${BOLD}IRAN Server Setup${NC}"
    echo -e "  ${CYAN}╔═══════════════════════════════════════════════╗${NC}"
    echo -e "  ${CYAN}║  This is your IRAN server (public IP).       ║${NC}"
    echo -e "  ${CYAN}║  Users connect HERE.                         ║${NC}"
    echo -e "  ${CYAN}║  Traffic: User → Iran → Tunnel → Kharej      ║${NC}"
    echo -e "  ${CYAN}╚═══════════════════════════════════════════════╝${NC}"
    print_divider
    echo ""

    echo -n "  Tunnel port [2333]: "; read -r server_port
    server_port="${server_port:-2333}"

    echo ""
    echo -n "  Token (empty=auto-generate): "; read -r default_token
    if [[ -z "${default_token}" ]]; then
        default_token=$(generate_token)
        echo -e "  ${GREEN}Generated:${NC} ${default_token}"
    fi

    echo ""
    select_transport

    mkdir -p "${CONFIG_DIR}"
    local config_file="${CONFIG_DIR}/server.toml"
    cat > "${config_file}" << EOF
[server]
bind_addr = "0.0.0.0:${server_port}"
default_token = "${default_token}"
heartbeat_interval = 30
EOF

    configure_transport_server "${config_file}"

    echo ""
    print_divider
    echo -e "  ${BOLD}Ports to expose (users connect to Iran on these ports)${NC}"
    echo -e "  ${DIM}Comma separated. Example: 2083,443,8080${NC}"
    print_divider
    echo ""
    echo -n "  Ports: "; read -r input_ports

    if [[ -z "${input_ports}" ]]; then
        print_error "At least one port is required!"
        return 1
    fi

    IFS=',' read -ra ports <<< "$(echo "${input_ports}" | tr -d ' ')"
    for port in "${ports[@]}"; do
        if [[ "${port}" =~ ^[0-9]+$ ]] && [ "${port}" -gt 0 ] && [ "${port}" -le 65535 ]; then
            cat >> "${config_file}" << EOF

[server.services.port${port}]
type = "tcp"
bind_addr = "0.0.0.0:${port}"
EOF
            print_success "Port ${port} added"
        else
            print_error "Invalid port: ${port}"
        fi
    done

    echo ""
    print_success "Config: ${config_file}"
    echo ""
    echo -e "  ${YELLOW}════════════════════════════════════════════${NC}"
    echo -e "  ${YELLOW}  COPY THIS FOR KHAREJ SETUP:${NC}"
    echo -e "  ${YELLOW}  Token:       ${default_token}${NC}"
    echo -e "  ${YELLOW}  Tunnel Port: ${server_port}${NC}"
    echo -e "  ${YELLOW}  Transport:   ${TRANSPORT}${NC}"
    echo -e "  ${YELLOW}════════════════════════════════════════════${NC}"
    echo ""

    create_systemd_service "server"
    systemctl start "${SERVICE_PREFIX}-server" 2>/dev/null && \
        print_success "Server started!" || \
        print_error "Failed to start (check: journalctl -u ${SERVICE_PREFIX}-server)"
}

# ─── KHAREJ Server Configuration ───────────────────────────────

configure_kharej() {
    echo ""
    print_divider
    echo -e "  ${BOLD}KHAREJ Server Setup${NC}"
    echo -e "  ${CYAN}╔═══════════════════════════════════════════════╗${NC}"
    echo -e "  ${CYAN}║  This is your KHAREJ (abroad) server.        ║${NC}"
    echo -e "  ${CYAN}║  Your panel/service runs HERE.               ║${NC}"
    echo -e "  ${CYAN}║  Traffic: Iran → Tunnel → Kharej:local       ║${NC}"
    echo -e "  ${CYAN}╚═══════════════════════════════════════════════╝${NC}"
    print_divider
    echo ""

    echo -n "  Iran server IP: "; read -r iran_ip
    if [[ -z "${iran_ip}" ]]; then
        print_error "Iran IP is required!"
        return 1
    fi

    echo -n "  Tunnel port (same as Iran): "; read -r tunnel_port
    if [[ -z "${tunnel_port}" ]]; then
        print_error "Tunnel port is required!"
        return 1
    fi

    echo -n "  Token (same as Iran): "; read -r default_token
    if [[ -z "${default_token}" ]]; then
        print_error "Token is required!"
        return 1
    fi

    echo ""
    select_transport

    mkdir -p "${CONFIG_DIR}"
    local config_file="${CONFIG_DIR}/client.toml"
    cat > "${config_file}" << EOF
[client]
remote_addr = "${iran_ip}:${tunnel_port}"
default_token = "${default_token}"
heartbeat_timeout = 40
retry_interval = 1
mux_connections = 4
EOF

    configure_transport_client "${config_file}"

    echo ""
    print_divider
    echo -e "  ${BOLD}Ports to forward (local services on this machine)${NC}"
    echo -e "  ${DIM}Comma separated. Example: 2083,443,8080${NC}"
    print_divider
    echo ""
    echo -n "  Ports: "; read -r input_ports

    if [[ -z "${input_ports}" ]]; then
        print_error "At least one port is required!"
        return 1
    fi

    IFS=',' read -ra ports <<< "$(echo "${input_ports}" | tr -d ' ')"
    for port in "${ports[@]}"; do
        if [[ "${port}" =~ ^[0-9]+$ ]] && [ "${port}" -gt 0 ] && [ "${port}" -le 65535 ]; then
            cat >> "${config_file}" << EOF

[client.services.port${port}]
type = "tcp"
local_addr = "127.0.0.1:${port}"
mux_streams = 4
EOF
            print_success "Port ${port} → 127.0.0.1:${port}"
        else
            print_error "Invalid port: ${port}"
        fi
    done

    echo ""
    print_success "Config: ${config_file}"

    create_systemd_service "client"
    systemctl start "${SERVICE_PREFIX}-client" 2>/dev/null && \
        print_success "Client started!" || \
        print_error "Failed to start (check: journalctl -u ${SERVICE_PREFIX}-client)"
}

# ─── Service Management ────────────────────────────────────────

start_service() {
    echo ""
    echo -e "    ${GREEN}1)${NC} Server  ${GREEN}2)${NC} Client  ${GREEN}3)${NC} Both"
    echo -n "  Choice: "; read -r c
    case "${c}" in
        1) systemctl start "${SERVICE_PREFIX}-server" && print_success "Server started" ;;
        2) systemctl start "${SERVICE_PREFIX}-client" && print_success "Client started" ;;
        3) systemctl start "${SERVICE_PREFIX}-server" 2>/dev/null; systemctl start "${SERVICE_PREFIX}-client" 2>/dev/null; print_success "Both started" ;;
    esac
}

stop_service() {
    echo ""
    echo -e "    ${GREEN}1)${NC} Server  ${GREEN}2)${NC} Client  ${GREEN}3)${NC} Both"
    echo -n "  Choice: "; read -r c
    case "${c}" in
        1) systemctl stop "${SERVICE_PREFIX}-server" && print_success "Server stopped" ;;
        2) systemctl stop "${SERVICE_PREFIX}-client" && print_success "Client stopped" ;;
        3) systemctl stop "${SERVICE_PREFIX}-server" 2>/dev/null; systemctl stop "${SERVICE_PREFIX}-client" 2>/dev/null; print_success "Both stopped" ;;
    esac
}

restart_service() {
    echo ""
    echo -e "    ${GREEN}1)${NC} Server  ${GREEN}2)${NC} Client  ${GREEN}3)${NC} Both"
    echo -n "  Choice: "; read -r c
    case "${c}" in
        1) systemctl restart "${SERVICE_PREFIX}-server" && print_success "Server restarted" ;;
        2) systemctl restart "${SERVICE_PREFIX}-client" && print_success "Client restarted" ;;
        3) systemctl restart "${SERVICE_PREFIX}-server" 2>/dev/null; systemctl restart "${SERVICE_PREFIX}-client" 2>/dev/null; print_success "Both restarted" ;;
    esac
}

show_status() {
    echo ""
    print_divider
    echo -e "  ${BOLD}Status${NC}"
    print_divider
    echo ""

    if systemctl is-active "${SERVICE_PREFIX}-server" &>/dev/null; then
        echo -e "  Server: ${GREEN}● Running${NC}"
    elif systemctl is-enabled "${SERVICE_PREFIX}-server" &>/dev/null 2>&1; then
        echo -e "  Server: ${YELLOW}● Stopped${NC}"
    else
        echo -e "  Server: ${DIM}○ Not configured${NC}"
    fi

    if systemctl is-active "${SERVICE_PREFIX}-client" &>/dev/null; then
        echo -e "  Client: ${GREEN}● Running${NC}"
    elif systemctl is-enabled "${SERVICE_PREFIX}-client" &>/dev/null 2>&1; then
        echo -e "  Client: ${YELLOW}● Stopped${NC}"
    else
        echo -e "  Client: ${DIM}○ Not configured${NC}"
    fi

    echo ""
    [[ -f "${CONFIG_DIR}/server.toml" ]] && echo -e "  Server config: ${CONFIG_DIR}/server.toml"
    [[ -f "${CONFIG_DIR}/client.toml" ]] && echo -e "  Client config: ${CONFIG_DIR}/client.toml"
    echo ""
}

view_logs() {
    echo ""
    echo -e "    ${GREEN}1)${NC} Server  ${GREEN}2)${NC} Client  ${GREEN}3)${NC} Server (live)  ${GREEN}4)${NC} Client (live)"
    echo -n "  Choice: "; read -r c
    case "${c}" in
        1) journalctl -u "${SERVICE_PREFIX}-server" --no-pager -n 50 ;;
        2) journalctl -u "${SERVICE_PREFIX}-client" --no-pager -n 50 ;;
        3) journalctl -u "${SERVICE_PREFIX}-server" -f ;;
        4) journalctl -u "${SERVICE_PREFIX}-client" -f ;;
    esac
}

view_config() {
    echo ""
    echo -e "    ${GREEN}1)${NC} Server  ${GREEN}2)${NC} Client"
    echo -n "  Choice: "; read -r c
    case "${c}" in
        1) [[ -f "${CONFIG_DIR}/server.toml" ]] && cat "${CONFIG_DIR}/server.toml" || print_warning "No server config" ;;
        2) [[ -f "${CONFIG_DIR}/client.toml" ]] && cat "${CONFIG_DIR}/client.toml" || print_warning "No client config" ;;
    esac
}

# ─── Update ────────────────────────────────────────────────────

update_binary() {
    echo ""
    print_step "Updating RatholePro..."
    detect_arch

    local server_was_running=false client_was_running=false
    systemctl is-active "${SERVICE_PREFIX}-server" &>/dev/null && server_was_running=true
    systemctl is-active "${SERVICE_PREFIX}-client" &>/dev/null && client_was_running=true

    [[ "${server_was_running}" == true ]] && systemctl stop "${SERVICE_PREFIX}-server"
    [[ "${client_was_running}" == true ]] && systemctl stop "${SERVICE_PREFIX}-client"

    download_binary

    [[ "${server_was_running}" == true ]] && systemctl start "${SERVICE_PREFIX}-server" && print_success "Server restarted"
    [[ "${client_was_running}" == true ]] && systemctl start "${SERVICE_PREFIX}-client" && print_success "Client restarted"

    print_success "Update complete!"
}

# ─── Uninstall ─────────────────────────────────────────────────

uninstall() {
    echo ""
    print_warning "This will remove RatholePro completely."
    echo -n "  Type 'yes' to confirm: "; read -r answer
    [[ "$answer" != "yes" ]] && return

    systemctl stop "${SERVICE_PREFIX}-server" 2>/dev/null || true
    systemctl stop "${SERVICE_PREFIX}-client" 2>/dev/null || true
    systemctl disable "${SERVICE_PREFIX}-server" 2>/dev/null || true
    systemctl disable "${SERVICE_PREFIX}-client" 2>/dev/null || true
    rm -f "/etc/systemd/system/${SERVICE_PREFIX}-server.service"
    rm -f "/etc/systemd/system/${SERVICE_PREFIX}-client.service"
    systemctl daemon-reload 2>/dev/null || true
    rm -rf "${RATHOLE_PRO_DIR}"
    rm -rf "${CONFIG_DIR}"
    rm -rf "${LOG_DIR}"
    rm -f "/usr/local/bin/${BINARY_NAME}"

    print_success "RatholePro completely uninstalled."
}

# ─── Main Menu ─────────────────────────────────────────────────

main_menu() {
    while true; do
        print_banner
        echo -e "  ${BOLD}Main Menu${NC}"
        echo ""
        echo -e "    ${GREEN} 1)${NC} Install Binary"
        echo ""
        echo -e "    ${CYAN}── Setup Tunnel ──${NC}"
        echo -e "    ${GREEN} 2)${NC} Configure IRAN Server   ${DIM}(users connect here)${NC}"
        echo -e "    ${GREEN} 3)${NC} Configure KHAREJ Server ${DIM}(services run here)${NC}"
        echo ""
        echo -e "    ${CYAN}── Manage ──${NC}"
        echo -e "    ${GREEN} 4)${NC} Start"
        echo -e "    ${GREEN} 5)${NC} Stop"
        echo -e "    ${GREEN} 6)${NC} Restart"
        echo -e "    ${GREEN} 7)${NC} Status"
        echo -e "    ${GREEN} 8)${NC} Logs"
        echo -e "    ${GREEN} 9)${NC} View Config"
        echo -e "    ${GREEN}10)${NC} Update"
        echo -e "    ${GREEN}11)${NC} Uninstall"
        echo ""
        echo -e "    ${RED} 0)${NC} Exit"
        echo ""
        echo -n "  Select: "; read -r choice

        case "${choice}" in
            1)  full_install ;;
            2)  check_root; configure_iran ;;
            3)  check_root; configure_kharej ;;
            4)  check_root; start_service ;;
            5)  check_root; stop_service ;;
            6)  check_root; restart_service ;;
            7)  show_status ;;
            8)  view_logs ;;
            9)  view_config ;;
            10) check_root; update_binary ;;
            11) check_root; uninstall ;;
            0)  echo -e "\n  ${DIM}Goodbye!${NC}\n"; exit 0 ;;
            *)  print_error "Invalid option" ;;
        esac

        echo ""
        echo -n "  Press Enter..."; read -r _
    done
}

# ─── Entry ─────────────────────────────────────────────────────
main_menu
