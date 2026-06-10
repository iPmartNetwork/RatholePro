pub mod tcp;
pub mod tls;
pub mod websocket;
pub mod noise;
pub mod quic;

use anyhow::Result;
use crate::config::TransportConfig;
use tokio::io::{AsyncRead, AsyncWrite};
use std::pin::Pin;

/// A unified async stream that can be TCP, TLS, WebSocket, or Noise.
/// This enum wraps different transport types into a single type
/// so the rest of the code doesn't need to know the underlying transport.
pub enum TransportStream {
    Tcp(tokio::net::TcpStream),
    Tls(tokio_rustls::client::TlsStream<tokio::net::TcpStream>),
    TlsServer(tokio_rustls::server::TlsStream<tokio::net::TcpStream>),
    WebSocket(websocket::WsStream),
    WebSocketServer(websocket::WsServerStream),
    Noise(noise::NoiseStream<tokio::net::TcpStream>),
    Quic(quic::QuicStream),
}

/// Implement AsyncRead for TransportStream
impl AsyncRead for TransportStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            TransportStream::Tcp(s) => Pin::new(s).poll_read(cx, buf),
            TransportStream::Tls(s) => Pin::new(s).poll_read(cx, buf),
            TransportStream::TlsServer(s) => Pin::new(s).poll_read(cx, buf),
            TransportStream::WebSocket(s) => Pin::new(s).poll_read(cx, buf),
            TransportStream::WebSocketServer(s) => Pin::new(s).poll_read(cx, buf),
            TransportStream::Noise(s) => Pin::new(s).poll_read(cx, buf),
            TransportStream::Quic(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

/// Implement AsyncWrite for TransportStream
impl AsyncWrite for TransportStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.get_mut() {
            TransportStream::Tcp(s) => Pin::new(s).poll_write(cx, buf),
            TransportStream::Tls(s) => Pin::new(s).poll_write(cx, buf),
            TransportStream::TlsServer(s) => Pin::new(s).poll_write(cx, buf),
            TransportStream::WebSocket(s) => Pin::new(s).poll_write(cx, buf),
            TransportStream::WebSocketServer(s) => Pin::new(s).poll_write(cx, buf),
            TransportStream::Noise(s) => Pin::new(s).poll_write(cx, buf),
            TransportStream::Quic(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            TransportStream::Tcp(s) => Pin::new(s).poll_flush(cx),
            TransportStream::Tls(s) => Pin::new(s).poll_flush(cx),
            TransportStream::TlsServer(s) => Pin::new(s).poll_flush(cx),
            TransportStream::WebSocket(s) => Pin::new(s).poll_flush(cx),
            TransportStream::WebSocketServer(s) => Pin::new(s).poll_flush(cx),
            TransportStream::Noise(s) => Pin::new(s).poll_flush(cx),
            TransportStream::Quic(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            TransportStream::Tcp(s) => Pin::new(s).poll_shutdown(cx),
            TransportStream::Tls(s) => Pin::new(s).poll_shutdown(cx),
            TransportStream::TlsServer(s) => Pin::new(s).poll_shutdown(cx),
            TransportStream::WebSocket(s) => Pin::new(s).poll_shutdown(cx),
            TransportStream::WebSocketServer(s) => Pin::new(s).poll_shutdown(cx),
            TransportStream::Noise(s) => Pin::new(s).poll_shutdown(cx),
            TransportStream::Quic(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

/// Determine transport type from config, defaulting to "tcp"
pub fn get_transport_type(config: &Option<TransportConfig>) -> &str {
    match config {
        Some(tc) => tc.transport_type.as_str(),
        None => "tcp",
    }
}

/// Client-side: establish a connection using the configured transport
pub async fn client_connect(
    remote_addr: &str,
    transport_config: &Option<TransportConfig>,
) -> Result<TransportStream> {
    let transport_type = get_transport_type(transport_config);
    let tcp_config = transport_config.as_ref().and_then(|c| c.tcp.clone());

    match transport_type {
        "tcp" => {
            let stream = tcp::connect(remote_addr, &tcp_config).await?;
            Ok(TransportStream::Tcp(stream))
        }
        "tls" => {
            let tls_config = transport_config
                .as_ref()
                .and_then(|c| c.tls.as_ref())
                .ok_or_else(|| anyhow::anyhow!("TLS transport requires [transport.tls] config"))?;
            let tcp_stream = tcp::connect(remote_addr, &tcp_config).await?;
            let tls_stream = tls::connect(tcp_stream, tls_config, remote_addr).await?;
            Ok(TransportStream::Tls(tls_stream))
        }
        "noise" => {
            let noise_config = transport_config
                .as_ref()
                .and_then(|c| c.noise.as_ref())
                .ok_or_else(|| anyhow::anyhow!("Noise transport requires [transport.noise] config"))?;
            let tcp_stream = tcp::connect(remote_addr, &tcp_config).await?;
            let noise_stream = noise::connect(tcp_stream, noise_config).await?;
            Ok(TransportStream::Noise(noise_stream))
        }
        "ws" => {
            let ws_path = transport_config
                .as_ref()
                .and_then(|c| c.websocket.as_ref())
                .and_then(|w| w.path.as_deref())
                .unwrap_or("/tunnel");
            let url = format!("ws://{}{}", remote_addr, ws_path);
            let ws_stream = websocket::connect(&url).await?;
            Ok(TransportStream::WebSocket(ws_stream))
        }
        "wss" => {
            let ws_path = transport_config
                .as_ref()
                .and_then(|c| c.websocket.as_ref())
                .and_then(|w| w.path.as_deref())
                .unwrap_or("/tunnel");
            let url = format!("wss://{}{}", remote_addr, ws_path);
            let ws_stream = websocket::connect(&url).await?;
            Ok(TransportStream::WebSocket(ws_stream))
        }
        "quic" => {
            let quic_config = transport_config
                .as_ref()
                .and_then(|c| c.quic.as_ref())
                .ok_or_else(|| anyhow::anyhow!("QUIC transport requires [transport.quic] config"))?;
            let quic_stream = quic::connect(remote_addr, quic_config).await?;
            Ok(TransportStream::Quic(quic_stream))
        }
        other => Err(anyhow::anyhow!("Unknown transport type: '{}'", other)),
    }
}

/// Server-side: accept and upgrade a raw TCP connection to the configured transport
pub async fn server_accept(
    tcp_stream: tokio::net::TcpStream,
    transport_config: &Option<TransportConfig>,
    tls_acceptor: Option<&tokio_rustls::TlsAcceptor>,
) -> Result<TransportStream> {
    let transport_type = get_transport_type(transport_config);

    match transport_type {
        "tcp" => {
            tcp_stream.set_nodelay(true)?;
            Ok(TransportStream::Tcp(tcp_stream))
        }
        "tls" => {
            let acceptor = tls_acceptor
                .ok_or_else(|| anyhow::anyhow!("TLS acceptor not initialized"))?;
            let tls_stream = tls::accept(acceptor, tcp_stream).await?;
            Ok(TransportStream::TlsServer(tls_stream))
        }
        "noise" => {
            let noise_config = transport_config
                .as_ref()
                .and_then(|c| c.noise.as_ref())
                .ok_or_else(|| anyhow::anyhow!("Noise transport requires [transport.noise] config"))?;
            let noise_stream = noise::accept(tcp_stream, noise_config).await?;
            Ok(TransportStream::Noise(noise_stream))
        }
        "ws" | "wss" => {
            let ws_stream = websocket::accept(tcp_stream).await?;
            Ok(TransportStream::WebSocketServer(ws_stream))
        }
        other => Err(anyhow::anyhow!("Unknown transport type: '{}'", other)),
    }
}
