//! QUIC transport implementation.
//!
//! QUIC provides built-in TLS 1.3, multiplexed streams, connection migration,
//! and better performance on lossy networks.

#[cfg(feature = "transport-quic")]
use anyhow::Result;
#[cfg(feature = "transport-quic")]
use std::pin::Pin;
#[cfg(feature = "transport-quic")]
use std::task::{Context, Poll};
#[cfg(feature = "transport-quic")]
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
#[cfg(feature = "transport-quic")]
use crate::config::QuicConfig;

/// QUIC stream wrapper implementing AsyncRead + AsyncWrite
#[cfg(feature = "transport-quic")]
pub struct QuicStream {
    send: quinn::SendStream,
    recv: quinn::RecvStream,
}

#[cfg(feature = "transport-quic")]
impl QuicStream {
    pub fn new(send: quinn::SendStream, recv: quinn::RecvStream) -> Self {
        Self { send, recv }
    }
}

#[cfg(feature = "transport-quic")]
impl AsyncRead for QuicStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.recv).poll_read(cx, buf)
    }
}

#[cfg(feature = "transport-quic")]
impl AsyncWrite for QuicStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.send).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.send).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.send).poll_shutdown(cx)
    }
}

#[cfg(feature = "transport-quic")]
pub async fn connect(remote_addr: &str, config: &QuicConfig) -> Result<QuicStream> {
    use std::sync::Arc;
    use std::net::SocketAddr;

    let addr: SocketAddr = remote_addr.parse()
        .map_err(|e| anyhow::anyhow!("Invalid QUIC addr '{}': {}", remote_addr, e))?;

    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let mut tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    tls_config.alpn_protocols = vec![b"rathole-pro".to_vec()];

    let client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)
            .map_err(|e| anyhow::anyhow!("QUIC config error: {}", e))?,
    ));

    let mut endpoint = quinn::Endpoint::client("0.0.0.0:0".parse()?)?;
    endpoint.set_default_client_config(client_config);

    let hostname = remote_addr.split(':').next().unwrap_or("localhost");
    let connection = endpoint.connect(addr, hostname)?.await
        .map_err(|e| anyhow::anyhow!("QUIC connect failed: {}", e))?;

    let (send, recv) = connection.open_bi().await
        .map_err(|e| anyhow::anyhow!("QUIC open_bi failed: {}", e))?;

    Ok(QuicStream::new(send, recv))
}

/// Placeholder when QUIC feature is disabled
#[cfg(not(feature = "transport-quic"))]
pub struct QuicStream;

#[cfg(not(feature = "transport-quic"))]
impl QuicStream {
    // Placeholder - never constructed when feature disabled
}
