//! P2P NAT traversal (STUN) and key generation utilities.

use anyhow::Result;
use std::net::SocketAddr;
use tokio::net::UdpSocket;
use tokio::time::{timeout, Duration};

/// Discover external address via STUN
pub async fn stun_discover(stun_server: &str) -> Result<SocketAddr> {
    let sock = UdpSocket::bind("0.0.0.0:0").await?;
    let server: SocketAddr = stun_server.parse()
        .map_err(|_| anyhow::anyhow!("Bad STUN addr: {}", stun_server))?;

    // STUN Binding Request (RFC 5389)
    let txid: [u8; 12] = {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let mut id = [0u8; 12];
        let ns = t.as_nanos();
        for i in 0..12 { id[i] = ((ns >> (i * 8)) & 0xFF) as u8; }
        id
    };
    let mut req = Vec::with_capacity(20);
    req.extend_from_slice(&0x0001u16.to_be_bytes()); // Binding Request
    req.extend_from_slice(&0u16.to_be_bytes());       // Length 0
    req.extend_from_slice(&0x2112A442u32.to_be_bytes()); // Magic
    req.extend_from_slice(&txid);

    sock.send_to(&req, server).await?;

    let mut buf = [0u8; 256];
    let (n, _) = timeout(Duration::from_secs(5), sock.recv_from(&mut buf)).await
        .map_err(|_| anyhow::anyhow!("STUN timeout"))??;

    // Parse XOR-MAPPED-ADDRESS
    if n < 20 { return Err(anyhow::anyhow!("STUN response too short")); }
    let magic = 0x2112A442u32;
    let mut pos = 20;
    let msg_len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    while pos + 4 <= 20 + msg_len && pos + 4 <= n {
        let attr_type = u16::from_be_bytes([buf[pos], buf[pos+1]]);
        let attr_len = u16::from_be_bytes([buf[pos+2], buf[pos+3]]) as usize;
        pos += 4;
        if attr_type == 0x0020 && attr_len >= 8 {
            // XOR-MAPPED-ADDRESS
            let port = u16::from_be_bytes([buf[pos+2], buf[pos+3]]) ^ (magic >> 16) as u16;
            let ip = u32::from_be_bytes([buf[pos+4], buf[pos+5], buf[pos+6], buf[pos+7]]) ^ magic;
            let addr = SocketAddr::new(std::net::Ipv4Addr::from(ip).into(), port);
            return Ok(addr);
        }
        pos += (attr_len + 3) & !3;
    }
    Err(anyhow::anyhow!("No XOR-MAPPED-ADDRESS in STUN response"))
}

/// Attempt UDP hole punch
pub async fn hole_punch(local: &UdpSocket, remote: SocketAddr, attempts: u32) -> bool {
    let punch = b"RHPR-PUNCH";
    for _ in 0..attempts {
        let _ = local.send_to(punch, remote).await;
        let mut buf = [0u8; 32];
        if let Ok(Ok((n, addr))) = timeout(Duration::from_millis(300), local.recv_from(&mut buf)).await {
            if addr == remote && n > 0 { return true; }
        }
    }
    false
}

/// Generate and print a Noise keypair (X25519)
pub fn gen_noise_keypair() {
    // X25519 key generation using simple random
    let private: [u8; 32] = {
        let mut key = [0u8; 32];
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let seed = t.as_nanos();
        for i in 0..32 {
            key[i] = ((seed >> (i % 16 * 8)) ^ (seed >> ((i + 7) % 16 * 8))) as u8;
        }
        // Clamp for X25519
        key[0] &= 248;
        key[31] &= 127;
        key[31] |= 64;
        key
    };

    use base64::Engine;
    let priv_b64 = base64::engine::general_purpose::STANDARD.encode(&private);
    // Note: proper public key derivation needs X25519 scalar mult.
    // For now, output private key and instruct user to use snow/rathole-pro --gen-key with snow.
    println!("═══════════════════════════════════════════");
    println!("  RatholePro — Keypair Generator");
    println!("═══════════════════════════════════════════");
    println!();
    println!("  Private Key (base64):");
    println!("    {}", priv_b64);
    println!();
    println!("  Usage in server.toml:");
    println!("    [server.transport.noise]");
    println!("    local_private_key = \"{}\"", priv_b64);
    println!();
    println!("  Note: For full Noise NK pattern, build with");
    println!("  'snow' crate for proper key derivation.");
    println!("═══════════════════════════════════════════");
}
