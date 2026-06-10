//! WebSocket transport helpers.
//! Provides connect/accept for tunneling over WebSocket (ws/wss).
//! Useful for bypassing firewalls that only allow HTTP/HTTPS.

use anyhow::Result;
use base64::Engine;
use sha2::Digest;
use tokio::net::TcpStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Simple WebSocket frame (binary opcode 0x82)
/// This is a minimal implementation — enough for tunneling binary data.
const WS_FIN_BINARY: u8 = 0x82;

/// Wrap data in a WebSocket binary frame
pub fn ws_frame(data: &[u8]) -> Vec<u8> {
    let len = data.len();
    let mut frame = Vec::with_capacity(10 + len);
    frame.push(WS_FIN_BINARY);
    if len < 126 {
        frame.push(len as u8);
    } else if len < 65536 {
        frame.push(126);
        frame.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        frame.push(127);
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }
    frame.extend_from_slice(data);
    frame
}

/// Perform a minimal WebSocket client upgrade handshake
pub async fn client_upgrade(stream: &mut TcpStream, host: &str, path: &str) -> Result<()> {
    let key = "dGhlIHNhbXBsZSBub25jZQ=="; // static for simplicity
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: {}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {}\r\nSec-WebSocket-Version: 13\r\n\r\n",
        path, host, key
    );
    stream.write_all(req.as_bytes()).await?;
    // Read response (just consume headers)
    let mut buf = vec![0u8; 1024];
    let n = stream.read(&mut buf).await?;
    let resp = String::from_utf8_lossy(&buf[..n]);
    if !resp.contains("101") {
        return Err(anyhow::anyhow!("WebSocket upgrade failed: {}", resp.lines().next().unwrap_or("")));
    }
    Ok(())
}

/// Perform a minimal WebSocket server upgrade handshake
pub async fn server_upgrade(stream: &mut TcpStream) -> Result<()> {
    let mut buf = vec![0u8; 2048];
    let n = stream.read(&mut buf).await?;
    let req = String::from_utf8_lossy(&buf[..n]);
    if !req.contains("Upgrade: websocket") && !req.contains("upgrade: websocket") {
        return Err(anyhow::anyhow!("Not a WebSocket upgrade request"));
    }
    // Extract Sec-WebSocket-Key
    let key = req.lines()
        .find(|l| l.to_lowercase().starts_with("sec-websocket-key"))
        .and_then(|l| l.split(':').nth(1))
        .map(|k| k.trim().to_string())
        .unwrap_or_default();

    // Compute accept key
    let magic = format!("{}258EAFA5-E914-47DA-95CA-C5AB0DC85B11", key);
    let hash = sha2::Sha256::digest(magic.as_bytes());
    let accept = base64::engine::general_purpose::STANDARD.encode(&hash);

    let resp = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
        accept
    );
    stream.write_all(resp.as_bytes()).await?;
    Ok(())
}
