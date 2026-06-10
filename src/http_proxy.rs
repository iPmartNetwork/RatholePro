//! HTTP/HTTPS proxy module.
//! Supports HTTP CONNECT (HTTPS tunneling) and HTTP forwarding.

use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// Parse the first line of an HTTP request: "METHOD PATH VERSION"
pub fn parse_request_line(data: &[u8]) -> Option<(String, String, String)> {
    let s = std::str::from_utf8(data).ok()?;
    let line = s.lines().next()?;
    let parts: Vec<&str> = line.splitn(3, ' ').collect();
    if parts.len() < 3 { return None; }
    Some((parts[0].to_string(), parts[1].to_string(), parts[2].to_string()))
}

/// Extract Host header from raw HTTP request bytes
pub fn extract_host(data: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(data).ok()?;
    for line in s.lines() {
        if line.to_lowercase().starts_with("host:") {
            return Some(line[5..].trim().to_string());
        }
    }
    None
}

/// Handle HTTP CONNECT method (HTTPS tunnel)
pub async fn handle_connect(mut client: TcpStream, target: &str) -> Result<()> {
    let mut target_stream = TcpStream::connect(target).await?;
    client.write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n").await?;
    let (mut cr, mut cw) = client.into_split();
    let (mut tr, mut tw) = target_stream.into_split();
    tokio::select! {
        _ = tokio::io::copy(&mut cr, &mut tw) => {}
        _ = tokio::io::copy(&mut tr, &mut cw) => {}
    }
    Ok(())
}

/// Forward HTTP request to backend and return response
pub async fn forward_request(request_bytes: &[u8], backend: &str) -> Result<Vec<u8>> {
    let mut stream = TcpStream::connect(backend).await?;
    let _ = stream.set_nodelay(true);
    stream.write_all(request_bytes).await?;
    let mut response = Vec::new();
    let mut buf = vec![0u8; 8192];
    loop {
        match stream.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => response.extend_from_slice(&buf[..n]),
            Err(_) => break,
        }
    }
    Ok(response)
}
