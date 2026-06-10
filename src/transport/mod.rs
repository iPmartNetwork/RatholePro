pub mod tcp;

#[cfg(feature = "transport-tls")]
pub mod tls;
#[cfg(feature = "transport-noise")]
pub mod noise;
#[cfg(feature = "transport-ws")]
pub mod websocket;
#[cfg(feature = "transport-quic")]
pub mod quic;

use anyhow::Result;
use crate::config::TransportConfig;
use tokio::io::{AsyncRead, AsyncWrite};
use std::pin::Pin;

/// A unified async stream wrapping all transport types.
pub enum TransportStream {
    Tcp(tokio::net::TcpStream),
    #[cfg(feature = "transport-tls")]
    Tls(tokio_rustls::client::TlsStream<tokio::net::TcpStream>),
    #[cfg(feature = "transport-tls")]
    TlsServer(tokio_rustls::server::TlsStream<tokio::net::TcpStream>),
    #[cfg(feature = "transport-ws")]
    WebSocket(websocket::WsStream),
    #[cfg(feature = "transport-ws")]
    WebSocketServer(websocket::WsServerStream),
    #[cfg(feature = "transport-noise")]
    Noise(noise::NoiseStream<tokio::net::TcpStream>),
    #[cfg(feature = "transport-quic")]
    Quic(quic::QuicStream),
}

impl AsyncRead for TransportStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            TransportStream::Tcp(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "transport-tls")]
            TransportStream::Tls(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "transport-tls")]
            TransportStream::TlsServer(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "transport-ws")]
            TransportStream::WebSocket(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "transport-ws")]
            TransportStream::WebSocketServer(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "transport-noise")]
            TransportStream::Noise(s) => Pin::new(s).poll_read(cx, buf),
            #[cfg(feature = "transport-quic")]
            TransportStream::Quic(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for TransportStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.get_mut() {
            TransportStream::Tcp(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "transport-tls")]
            TransportStream::Tls(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "transport-tls")]
            TransportStream::TlsServer(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "transport-ws")]
            TransportStream::WebSocket(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "transport-ws")]
            TransportStream::WebSocketServer(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "transport-noise")]
            TransportStream::Noise(s) => Pin::new(s).poll_write(cx, buf),
            #[cfg(feature = "transport-quic")]
            TransportStream::Quic(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            TransportStream::Tcp(s) => Pin::new(s).poll_flush(cx),
            #[cfg(feature = "transport-tls")]
            TransportStream::Tls(s) => Pin::new(s).poll_flush(cx),
            #[cfg(feature = "transport-tls")]
            TransportStream::TlsServer(s) => Pin::new(s).poll_flush(cx),
            #[cfg(feature = "transport-ws")]
            TransportStream::WebSocket(s) => Pin::new(s).poll_flush(cx),
            #[cfg(feature = "transport-ws")]
            TransportStream::WebSocketServer(s) => Pin::new(s).poll_flush(cx),
            #[cfg(feature = "transport-noise")]
            TransportStream::Noise(s) => Pin::new(s).poll_flush(cx),
            #[cfg(feature = "transport-quic")]
            TransportStream::Quic(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match self.get_mut() {
            TransportStream::Tcp(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(feature = "transport-tls")]
            TransportStream::Tls(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(feature = "transport-tls")]
            TransportStream::TlsServer(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(feature = "transport-ws")]
            TransportStream::WebSocket(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(feature = "transport-ws")]
            TransportStream::WebSocketServer(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(feature = "transport-noise")]
            TransportStream::Noise(s) => Pin::new(s).poll_shutdown(cx),
            #[cfg(feature = "transport-quic")]
            TransportStream::Quic(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

pub fn get_transport_type(config: &Option<TransportConfig>) -> &str {
    match config {
        Some(tc) => tc.transport_type.as_str(),
        None => "tcp",
    }
}

/// Client-side: connect using configured transport
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
        #[cfg(feature = "transport-tls")]
        "tls" => {
            let tls_cfg = transport_config.as_ref()
                .and_then(|c| c.tls.as_ref())
                .ok_or_else(|| anyhow::anyhow!("TLS requires [transport.tls] config"))?;
            let tcp_stream = tcp::connect(remote_addr, &tcp_config).await?;
            let tls_stream = tls::connect(tcp_stream, tls_cfg, remote_addr).await?;
            Ok(TransportStream::Tls(tls_stream))
        }
        #[cfg(feature = "transport-noise")]
        "noise" => {
            let noise_cfg = transport_config.as_ref()
                .and_then(|c| c.noise.as_ref())
                .ok_or_else(|| anyhow::anyhow!("Noise requires [transport.noise] config"))?;
            let tcp_stream = tcp::connect(remote_addr, &tcp_config).await?;
            let noise_stream = noise::connect(tcp_stream, noise_cfg).await?;
            Ok(TransportStream::Noise(noise_stream))
        }
        #[cfg(feature = "transport-ws")]
        "ws" => {
            let path = transport_config.as_ref()
                .and_then(|c| c.websocket.as_ref())
                .and_then(|w| w.path.as_deref())
                .unwrap_or("/tunnel");
            let url = format!("ws://{}{}", remote_addr, path);
            let ws = websocket::connect(&url).await?;
            Ok(TransportStream::WebSocket(ws))
        }
        #[cfg(feature = "transport-ws")]
        "wss" => {
            let path = transport_config.as_ref()
                .and_then(|c| c.websocket.as_ref())
                .and_then(|w| w.path.as_deref())
                .unwrap_or("/tunnel");
            let url = format!("wss://{}{}", remote_addr, path);
            let ws = websocket::connect(&url).await?;
            Ok(TransportStream::WebSocket(ws))
        }
        #[cfg(feature = "transport-quic")]
        "quic" => {
            let quic_cfg = transport_config.as_ref()
                .and_then(|c| c.quic.as_ref())
                .ok_or_else(|| anyhow::anyhow!("QUIC requires [transport.quic] config"))?;
            let qs = quic::connect(remote_addr, quic_cfg).await?;
            Ok(TransportStream::Quic(qs))
        }
        other => Err(anyhow::anyhow!("Unsupported transport: '{}'. Check enabled features.", other)),
    }
}

/// Server-side: accept and upgrade connection
#[cfg(feature = "transport-tls")]
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
            let s = tls::accept(acceptor, tcp_stream).await?;
            Ok(TransportStream::TlsServer(s))
        }
        #[cfg(feature = "transport-noise")]
        "noise" => {
            let noise_cfg = transport_config.as_ref()
                .and_then(|c| c.noise.as_ref())
                .ok_or_else(|| anyhow::anyhow!("Noise requires config"))?;
            let s = noise::accept(tcp_stream, noise_cfg).await?;
            Ok(TransportStream::Noise(s))
        }
        #[cfg(feature = "transport-ws")]
        "ws" | "wss" => {
            let s = websocket::accept(tcp_stream).await?;
            Ok(TransportStream::WebSocketServer(s))
        }
        other => Err(anyhow::anyhow!("Unsupported server transport: '{}'", other)),
    }
}

/// Server-side accept without TLS feature (fallback)
#[cfg(not(feature = "transport-tls"))]
pub async fn server_accept(
    tcp_stream: tokio::net::TcpStream,
    transport_config: &Option<TransportConfig>,
    _tls_acceptor: Option<&()>,
) -> Result<TransportStream> {
    let transport_type = get_transport_type(transport_config);

    match transport_type {
        "tcp" => {
            tcp_stream.set_nodelay(true)?;
            Ok(TransportStream::Tcp(tcp_stream))
        }
        #[cfg(feature = "transport-noise")]
        "noise" => {
            let noise_cfg = transport_config.as_ref()
                .and_then(|c| c.noise.as_ref())
                .ok_or_else(|| anyhow::anyhow!("Noise requires config"))?;
            let s = noise::accept(tcp_stream, noise_cfg).await?;
            Ok(TransportStream::Noise(s))
        }
        #[cfg(feature = "transport-ws")]
        "ws" | "wss" => {
            let s = websocket::accept(tcp_stream).await?;
            Ok(TransportStream::WebSocketServer(s))
        }
        other => Err(anyhow::anyhow!("Unsupported transport: '{}'", other)),
    }
}
