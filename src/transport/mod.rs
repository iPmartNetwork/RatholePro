pub mod tls;
pub mod noise;

use anyhow::Result;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Type-erased async stream wrapper.
/// Implements AsyncRead + AsyncWrite so it can be used directly with Framed.
pub struct BoxedStream {
    inner: Box<dyn AsyncRead + AsyncWrite + Unpin + Send>,
}

impl BoxedStream {
    pub fn new<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(s: S) -> Self {
        Self { inner: Box::new(s) }
    }
}

impl AsyncRead for BoxedStream {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut *self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for BoxedStream {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut *self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut *self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut *self.inner).poll_shutdown(cx)
    }
}

/// Supported transport types
#[derive(Debug, Clone, PartialEq)]
pub enum TransportType {
    Tcp,
    Tls,
    Noise,
}

impl TransportType {
    pub fn from_str(s: &str) -> Self {
        match s {
            "tls" => Self::Tls,
            "noise" => Self::Noise,
            _ => Self::Tcp,
        }
    }
}

/// Client: connect to remote using the configured transport
pub async fn client_connect(
    addr: &str,
    transport_type: &TransportType,
    tls_hostname: Option<&str>,
    noise_remote_key: Option<&str>,
) -> Result<BoxedStream> {
    let tcp = TcpStream::connect(addr).await?;
    tcp.set_nodelay(true)?;

    match transport_type {
        TransportType::Tcp => Ok(BoxedStream::new(tcp)),
        TransportType::Tls => {
            let hostname = tls_hostname.unwrap_or_else(|| addr.split(':').next().unwrap_or("localhost"));
            let stream = tls::connect(tcp, hostname).await?;
            Ok(BoxedStream::new(stream))
        }
        TransportType::Noise => {
            let remote_key = noise_remote_key.unwrap_or("");
            let stream = noise::connect(tcp, remote_key).await?;
            Ok(BoxedStream::new(stream))
        }
    }
}

/// Server: accept and upgrade a TCP connection
pub async fn server_accept(
    tcp: TcpStream,
    transport_type: &TransportType,
    tls_acceptor: Option<&tokio_rustls::TlsAcceptor>,
    noise_private_key: Option<&str>,
) -> Result<BoxedStream> {
    tcp.set_nodelay(true)?;

    match transport_type {
        TransportType::Tcp => Ok(BoxedStream::new(tcp)),
        TransportType::Tls => {
            let acceptor = tls_acceptor
                .ok_or_else(|| anyhow::anyhow!("TLS acceptor not configured"))?;
            let stream = tls::accept(acceptor, tcp).await?;
            Ok(BoxedStream::new(stream))
        }
        TransportType::Noise => {
            let key = noise_private_key.unwrap_or("");
            let stream = noise::accept(tcp, key).await?;
            Ok(BoxedStream::new(stream))
        }
    }
}
