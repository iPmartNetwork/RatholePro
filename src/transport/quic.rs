//! QUIC transport implementation using the quinn crate.
//!
//! QUIC provides:
//! - Built-in TLS 1.3 encryption
//! - Multiplexed streams natively (no need for yamux/custom mux)
//! - Connection migration (survives IP changes)
//! - Lower latency (0-RTT reconnect)
//! - Better performance on lossy networks (no head-of-line blocking)

use anyhow::Result;
use quinn::{ClientConfig, Endpoint, ServerConfig as QuinnServerConfig, Connection};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::fs;
use std::io::BufReader;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use crate::config::QuicConfig;

/// QUIC stream wrapper implementing AsyncRead + AsyncWrite
pub struct QuicStream {
    send: quinn::SendStream,
    recv: quinn::RecvStream,
}

impl QuicStream {
    pub fn new(send: quinn::SendStream, recv: quinn::RecvStream) -> Self {
        Self { send, recv }
    }
}

impl AsyncRead for QuicStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().recv).poll_read(cx, buf)
    }
}

impl AsyncWrite for QuicStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().send).poll_write(cx, buf)
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().send).poll_flush(cx)
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().send).poll_shutdown(cx)
    }
}

/// Create a QUIC client endpoint and connect to server
pub async fn connect(
    remote_addr: &str,
    config: &QuicConfig,
) -> Result<QuicStream> {
    let addr: SocketAddr = remote_addr.parse()
        .map_err(|e| anyhow::anyhow!("Invalid QUIC remote addr '{}': {}", remote_addr, e))?;

    // Build client TLS config
    let mut root_store = rustls::RootCertStore::empty();

    if let Some(ref ca_path) = config.ca {
        let ca_file = fs::File::open(ca_path)
            .map_err(|e| anyhow::anyhow!("Failed to open CA file '{}': {}", ca_path, e))?;
        let mut reader = BufReader::new(ca_file);
        let certs = rustls_pemfile::certs(&mut reader)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!("Failed to parse CA certs: {}", e))?;
        for cert in certs {
            root_store.add(cert)
                .map_err(|e| anyhow::anyhow!("Failed to add CA cert: {}", e))?;
        }
    } else {
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }

    let mut tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    // Set ALPN
    if let Some(ref alpn) = config.alpn {
        tls_config.alpn_protocols = alpn.iter().map(|s| s.as_bytes().to_vec()).collect();
    } else {
        tls_config.alpn_protocols = vec![b"rathole-pro".to_vec()];
    }

    let client_config = ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)
            .map_err(|e| anyhow::anyhow!("QUIC client config error: {}", e))?,
    ));

    // Bind to any local address
    let mut endpoint = Endpoint::client("0.0.0.0:0".parse()?)?;
    endpoint.set_default_client_config(client_config);

    // Extract hostname for SNI
    let hostname = remote_addr.split(':').next().unwrap_or("localhost");

    // Connect
    let connection = endpoint.connect(addr, hostname)?
        .await
        .map_err(|e| anyhow::anyhow!("QUIC connection failed: {}", e))?;

    tracing::info!("QUIC connected to {}", remote_addr);

    // Open a bidirectional stream for the control channel
    let (send, recv) = connection.open_bi().await
        .map_err(|e| anyhow::anyhow!("QUIC open stream failed: {}", e))?;

    Ok(QuicStream::new(send, recv))
}

/// Create a QUIC server endpoint
pub async fn create_server_endpoint(
    bind_addr: &str,
    config: &QuicConfig,
) -> Result<Endpoint> {
    let addr: SocketAddr = bind_addr.parse()
        .map_err(|e| anyhow::anyhow!("Invalid QUIC bind addr '{}': {}", bind_addr, e))?;

    // Load certificate chain
    let cert_path = config.cert.as_ref()
        .ok_or_else(|| anyhow::anyhow!("QUIC server requires 'cert' path"))?;
    let cert_file = fs::File::open(cert_path)
        .map_err(|e| anyhow::anyhow!("Failed to open cert '{}': {}", cert_path, e))?;
    let mut cert_reader = BufReader::new(cert_file);
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("Failed to parse certs: {}", e))?;

    // Load private key
    let key_path = config.key.as_ref()
        .ok_or_else(|| anyhow::anyhow!("QUIC server requires 'key' path"))?;
    let key_file = fs::File::open(key_path)
        .map_err(|e| anyhow::anyhow!("Failed to open key '{}': {}", key_path, e))?;
    let mut key_reader = BufReader::new(key_file);
    let key = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| anyhow::anyhow!("Failed to parse key: {}", e))?
        .ok_or_else(|| anyhow::anyhow!("No private key found in key file"))?;

    // Build server TLS config
    let mut tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| anyhow::anyhow!("TLS server config error: {}", e))?;

    tls_config.alpn_protocols = vec![b"rathole-pro".to_vec()];

    let quic_server_config = QuinnServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)
            .map_err(|e| anyhow::anyhow!("QUIC server config error: {}", e))?,
    ));

    let endpoint = Endpoint::server(quic_server_config, addr)?;
    tracing::info!("QUIC server endpoint bound to {}", bind_addr);

    Ok(endpoint)
}

/// Accept a QUIC connection and return a stream
pub async fn accept(endpoint: &Endpoint) -> Result<QuicStream> {
    let incoming = endpoint.accept().await
        .ok_or_else(|| anyhow::anyhow!("QUIC endpoint closed"))?;

    let connection = incoming.await
        .map_err(|e| anyhow::anyhow!("QUIC accept failed: {}", e))?;

    tracing::info!("QUIC client connected from {}", connection.remote_address());

    let (send, recv) = connection.accept_bi().await
        .map_err(|e| anyhow::anyhow!("QUIC accept stream failed: {}", e))?;

    Ok(QuicStream::new(send, recv))
}
