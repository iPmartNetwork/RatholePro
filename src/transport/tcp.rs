use anyhow::Result;
use tokio::net::{TcpListener, TcpStream};
use crate::config::TcpConfig;

/// Connect to a remote address over TCP
pub async fn connect(addr: &str, config: &Option<TcpConfig>) -> Result<TcpStream> {
    let stream = TcpStream::connect(addr).await?;
    apply_tcp_options(&stream, config)?;
    Ok(stream)
}

/// Create a TCP listener
pub async fn listen(addr: &str) -> Result<TcpListener> {
    let listener = TcpListener::bind(addr).await?;
    Ok(listener)
}

/// Accept a connection and apply TCP options
pub async fn accept(listener: &TcpListener, config: &Option<TcpConfig>) -> Result<(TcpStream, std::net::SocketAddr)> {
    let (stream, addr) = listener.accept().await?;
    apply_tcp_options(&stream, config)?;
    Ok((stream, addr))
}

/// Apply TCP options from config
fn apply_tcp_options(stream: &TcpStream, config: &Option<TcpConfig>) -> Result<()> {
    let nodelay = config
        .as_ref()
        .and_then(|c| c.nodelay)
        .unwrap_or(true);
    stream.set_nodelay(nodelay)?;

    // Note: keepalive requires platform-specific socket options.
    // Tokio's TcpStream doesn't expose them directly on all platforms.
    // For production, use socket2 crate. For now, nodelay is sufficient.

    Ok(())
}
