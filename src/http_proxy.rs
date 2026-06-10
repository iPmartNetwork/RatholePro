//! HTTP/HTTPS proxy forwarding module.
//!
//! Supports forwarding HTTP traffic with:
//! - Host-based routing (virtual hosts)
//! - Header rewriting (X-Forwarded-For, etc.)
//! - HTTPS termination or passthrough

use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use std::sync::Arc;

/// HTTP request info extracted from the first line
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub version: String,
    pub host: Option<String>,
    pub headers: Vec<(String, String)>,
    pub raw_header: Vec<u8>,
}

/// Parse HTTP request headers from raw bytes
pub fn parse_http_request(data: &[u8]) -> Result<HttpRequest> {
    let header_str = std::str::from_utf8(data)
        .map_err(|e| anyhow::anyhow!("Invalid HTTP header encoding: {}", e))?;

    let mut lines = header_str.lines();

    // Parse request line: "GET /path HTTP/1.1"
    let request_line = lines.next()
        .ok_or_else(|| anyhow::anyhow!("Empty HTTP request"))?;
    let parts: Vec<&str> = request_line.splitn(3, ' ').collect();
    if parts.len() < 3 {
        return Err(anyhow::anyhow!("Invalid HTTP request line: '{}'", request_line));
    }

    let method = parts[0].to_string();
    let path = parts[1].to_string();
    let version = parts[2].to_string();

    // Parse headers
    let mut headers = Vec::new();
    let mut host = None;

    for line in lines {
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_string();
            let value = value.trim().to_string();
            if key.eq_ignore_ascii_case("host") {
                host = Some(value.clone());
            }
            headers.push((key, value));
        }
    }

    Ok(HttpRequest {
        method,
        path,
        version,
        host,
        headers,
        raw_header: data.to_vec(),
    })
}

/// Add proxy headers to the request
pub fn add_proxy_headers(request: &mut HttpRequest, client_addr: &str) {
    // Add X-Forwarded-For
    request.headers.push(("X-Forwarded-For".to_string(), client_addr.to_string()));
    // Add X-Real-IP
    request.headers.push(("X-Real-IP".to_string(), client_addr.to_string()));
}

/// Rebuild HTTP request from parsed data
pub fn rebuild_http_request(request: &HttpRequest) -> Vec<u8> {
    let mut output = format!("{} {} {}\r\n", request.method, request.path, request.version);

    for (key, value) in &request.headers {
        output.push_str(&format!("{}: {}\r\n", key, value));
    }
    output.push_str("\r\n");

    output.into_bytes()
}

/// HTTP CONNECT proxy handler (for HTTPS tunneling)
pub async fn handle_connect(
    mut client_stream: TcpStream,
    target_addr: &str,
) -> Result<()> {
    // Connect to target
    let mut target_stream = TcpStream::connect(target_addr).await?;

    // Send 200 Connection Established
    client_stream
        .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
        .await?;

    // Bidirectional copy (tunnel mode)
    let (mut client_read, mut client_write) = client_stream.into_split();
    let (mut target_read, mut target_write) = target_stream.into_split();

    tokio::select! {
        r = tokio::io::copy(&mut client_read, &mut target_write) => {
            if let Err(e) = r { tracing::debug!("CONNECT tunnel ended: {}", e); }
        }
        r = tokio::io::copy(&mut target_read, &mut client_write) => {
            if let Err(e) = r { tracing::debug!("CONNECT tunnel ended: {}", e); }
        }
    }

    Ok(())
}

/// Forward an HTTP request to a backend, return the response
pub async fn forward_http(
    request: &HttpRequest,
    body: &[u8],
    backend_addr: &str,
) -> Result<Vec<u8>> {
    let mut stream = TcpStream::connect(backend_addr).await?;
    stream.set_nodelay(true)?;

    // Send rebuilt request
    let header_bytes = rebuild_http_request(request);
    stream.write_all(&header_bytes).await?;

    // Send body if any
    if !body.is_empty() {
        stream.write_all(body).await?;
    }

    // Read response
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
