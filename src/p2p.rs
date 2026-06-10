//! P2P NAT traversal module using STUN/TURN.
//!
//! Attempts to establish a direct peer-to-peer connection between
//! client and server, falling back to relay (TURN) if direct fails.
//!
//! Flow:
//! 1. Both peers query STUN server to discover external IP:port
//! 2. Exchange candidates via signaling (control channel)
//! 3. Attempt UDP hole punching for direct connection
//! 4. Fall back to TURN relay if hole punching fails

use anyhow::Result;
use std::net::SocketAddr;
use tokio::net::UdpSocket;
use tokio::time::{timeout, Duration};

/// STUN message type constants
const STUN_BINDING_REQUEST: u16 = 0x0001;
const STUN_BINDING_RESPONSE: u16 = 0x0101;
const STUN_MAGIC_COOKIE: u32 = 0x2112A442;
const STUN_ATTR_MAPPED_ADDRESS: u16 = 0x0001;
const STUN_ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;

/// Result of STUN binding request
#[derive(Debug, Clone)]
pub struct StunResult {
    /// Our external (public) address as seen by STUN server
    pub external_addr: SocketAddr,
    /// Whether we're behind symmetric NAT (harder to traverse)
    pub symmetric_nat: bool,
}

/// NAT traversal candidate
#[derive(Debug, Clone)]
pub struct Candidate {
    /// Local address
    pub local_addr: SocketAddr,
    /// External (STUN-discovered) address
    pub external_addr: Option<SocketAddr>,
    /// TURN relay address (fallback)
    pub relay_addr: Option<SocketAddr>,
}

/// Discover our external address using STUN
pub async fn stun_discover(stun_server: &str) -> Result<StunResult> {
    let socket = UdpSocket::bind("0.0.0.0:0").await?;

    let stun_addr: SocketAddr = stun_server.parse()
        .map_err(|e| anyhow::anyhow!("Invalid STUN server address '{}': {}", stun_server, e))?;

    // Build STUN Binding Request
    let transaction_id: [u8; 12] = rand_transaction_id();
    let request = build_stun_binding_request(&transaction_id);

    // Send request
    socket.send_to(&request, stun_addr).await?;

    // Wait for response (5 second timeout)
    let mut buf = [0u8; 1024];
    let (len, _) = timeout(Duration::from_secs(5), socket.recv_from(&mut buf))
        .await
        .map_err(|_| anyhow::anyhow!("STUN request timed out"))?
        .map_err(|e| anyhow::anyhow!("STUN recv error: {}", e))?;

    // Parse response
    let external_addr = parse_stun_response(&buf[..len], &transaction_id)?;

    // Simple symmetric NAT detection:
    // Send another request from same socket to see if we get same mapping
    let request2 = build_stun_binding_request(&transaction_id);
    socket.send_to(&request2, stun_addr).await?;

    let (len2, _) = timeout(Duration::from_secs(5), socket.recv_from(&mut buf))
        .await
        .map_err(|_| anyhow::anyhow!("STUN second request timed out"))?
        .map_err(|e| anyhow::anyhow!("STUN recv error: {}", e))?;

    let external_addr2 = parse_stun_response(&buf[..len2], &transaction_id)?;

    let symmetric_nat = external_addr != external_addr2;

    tracing::info!(
        "STUN discovered: external={}, symmetric_nat={}",
        external_addr, symmetric_nat
    );

    Ok(StunResult {
        external_addr,
        symmetric_nat,
    })
}

/// Attempt UDP hole punching to establish direct connection
pub async fn hole_punch(
    local_socket: &UdpSocket,
    remote_addr: SocketAddr,
    attempts: u32,
) -> Result<bool> {
    tracing::info!("Attempting hole punch to {}", remote_addr);

    let punch_data = b"RHPR-PUNCH";

    for i in 0..attempts {
        // Send punch packet
        local_socket.send_to(punch_data, remote_addr).await?;

        // Wait briefly for response
        let mut buf = [0u8; 64];
        match timeout(Duration::from_millis(500), local_socket.recv_from(&mut buf)).await {
            Ok(Ok((len, addr))) => {
                if addr == remote_addr && &buf[..len.min(punch_data.len())] == punch_data {
                    tracing::info!("Hole punch successful on attempt {}", i + 1);
                    return Ok(true);
                }
            }
            _ => {
                // Timeout or error, try again
                tracing::debug!("Hole punch attempt {} failed, retrying...", i + 1);
            }
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    tracing::warn!("Hole punch failed after {} attempts", attempts);
    Ok(false)
}

/// Build a minimal STUN Binding Request
fn build_stun_binding_request(transaction_id: &[u8; 12]) -> Vec<u8> {
    let mut msg = Vec::with_capacity(20);

    // Message Type: Binding Request (0x0001)
    msg.extend_from_slice(&STUN_BINDING_REQUEST.to_be_bytes());
    // Message Length: 0 (no attributes)
    msg.extend_from_slice(&0u16.to_be_bytes());
    // Magic Cookie
    msg.extend_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());
    // Transaction ID (12 bytes)
    msg.extend_from_slice(transaction_id);

    msg
}

/// Parse STUN Binding Response to extract mapped address
fn parse_stun_response(data: &[u8], expected_txid: &[u8; 12]) -> Result<SocketAddr> {
    if data.len() < 20 {
        return Err(anyhow::anyhow!("STUN response too short"));
    }

    // Check message type
    let msg_type = u16::from_be_bytes([data[0], data[1]]);
    if msg_type != STUN_BINDING_RESPONSE {
        return Err(anyhow::anyhow!("Not a STUN Binding Response: 0x{:04x}", msg_type));
    }

    // Check magic cookie
    let cookie = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    if cookie != STUN_MAGIC_COOKIE {
        return Err(anyhow::anyhow!("Invalid STUN magic cookie"));
    }

    // Check transaction ID
    if &data[8..20] != expected_txid {
        return Err(anyhow::anyhow!("STUN transaction ID mismatch"));
    }

    let msg_len = u16::from_be_bytes([data[2], data[3]]) as usize;

    // Parse attributes
    let mut pos = 20;
    while pos + 4 <= 20 + msg_len && pos + 4 <= data.len() {
        let attr_type = u16::from_be_bytes([data[pos], data[pos + 1]]);
        let attr_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;

        if pos + attr_len > data.len() {
            break;
        }

        match attr_type {
            STUN_ATTR_XOR_MAPPED_ADDRESS => {
                return parse_xor_mapped_address(&data[pos..pos + attr_len]);
            }
            STUN_ATTR_MAPPED_ADDRESS => {
                return parse_mapped_address(&data[pos..pos + attr_len]);
            }
            _ => {}
        }

        // Align to 4 bytes
        pos += (attr_len + 3) & !3;
    }

    Err(anyhow::anyhow!("No mapped address in STUN response"))
}

fn parse_xor_mapped_address(data: &[u8]) -> Result<SocketAddr> {
    if data.len() < 8 {
        return Err(anyhow::anyhow!("XOR-MAPPED-ADDRESS too short"));
    }

    let family = data[1];
    let xor_port = u16::from_be_bytes([data[2], data[3]]) ^ (STUN_MAGIC_COOKIE >> 16) as u16;

    match family {
        0x01 => {
            // IPv4
            let xor_ip = u32::from_be_bytes([data[4], data[5], data[6], data[7]]) ^ STUN_MAGIC_COOKIE;
            let ip = std::net::Ipv4Addr::from(xor_ip);
            Ok(SocketAddr::new(std::net::IpAddr::V4(ip), xor_port))
        }
        0x02 => {
            // IPv6
            if data.len() < 20 {
                return Err(anyhow::anyhow!("XOR-MAPPED-ADDRESS IPv6 too short"));
            }
            // XOR with magic cookie + transaction ID (simplified)
            let mut ip_bytes = [0u8; 16];
            ip_bytes.copy_from_slice(&data[4..20]);
            // XOR first 4 bytes with magic cookie
            let cookie_bytes = STUN_MAGIC_COOKIE.to_be_bytes();
            for i in 0..4 {
                ip_bytes[i] ^= cookie_bytes[i];
            }
            let ip = std::net::Ipv6Addr::from(ip_bytes);
            Ok(SocketAddr::new(std::net::IpAddr::V6(ip), xor_port))
        }
        _ => Err(anyhow::anyhow!("Unknown address family: {}", family)),
    }
}

fn parse_mapped_address(data: &[u8]) -> Result<SocketAddr> {
    if data.len() < 8 {
        return Err(anyhow::anyhow!("MAPPED-ADDRESS too short"));
    }

    let family = data[1];
    let port = u16::from_be_bytes([data[2], data[3]]);

    match family {
        0x01 => {
            let ip = std::net::Ipv4Addr::new(data[4], data[5], data[6], data[7]);
            Ok(SocketAddr::new(std::net::IpAddr::V4(ip), port))
        }
        0x02 => {
            if data.len() < 20 {
                return Err(anyhow::anyhow!("MAPPED-ADDRESS IPv6 too short"));
            }
            let mut ip_bytes = [0u8; 16];
            ip_bytes.copy_from_slice(&data[4..20]);
            let ip = std::net::Ipv6Addr::from(ip_bytes);
            Ok(SocketAddr::new(std::net::IpAddr::V6(ip), port))
        }
        _ => Err(anyhow::anyhow!("Unknown address family: {}", family)),
    }
}

/// Generate random transaction ID
fn rand_transaction_id() -> [u8; 12] {
    let mut id = [0u8; 12];
    // Use time-based pseudo-random (good enough for STUN)
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let seed = now.as_nanos();
    for (i, byte) in id.iter_mut().enumerate() {
        *byte = ((seed >> (i * 8)) & 0xFF) as u8;
    }
    id
}
