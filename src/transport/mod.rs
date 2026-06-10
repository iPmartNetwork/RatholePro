pub mod tls;
pub mod noise;

use anyhow::Result;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;

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

/// Wrapper struct implementing AsyncRead + AsyncWrite over a boxed stream.
pub struct BoxedStream(Box<dyn AsyncRead + AsyncWrite + Unpin + Send>);

impl BoxedStream {
    pub fn new<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(s: S) -> Self {
        Self(Box::new(s))
    }
}

impl AsyncRead for BoxedStream {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut *self.0).poll_read(cx, buf)
    }
}

impl AsyncWrite for BoxedStream {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut *self.0).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut *self.0).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut *self.0).poll_shutdown(cx)
    }
}

/// Client connect
pub async fn client_connect(addr: &str, tt: &TransportType, tls_host: Option<&str>, noise_key: Option<&str>) -> Result<BoxedStream> {
    let tcp = TcpStream::connect(addr).await?;
    tcp.set_nodelay(true)?;
    match tt {
        TransportType::Tcp => Ok(BoxedStream::new(tcp)),
        TransportType::Tls => {
            let h = tls_host.unwrap_or_else(|| addr.split(':').next().unwrap_or("localhost"));
            let s = tls::connect(tcp, h).await?;
            Ok(BoxedStream::new(s))
        }
        TransportType::Noise => {
            let k = noise_key.unwrap_or("");
            let s = noise::connect(tcp, k).await?;
            Ok(BoxedStream::new(s))
        }
    }
}

/// Server accept
pub async fn server_accept(tcp: TcpStream, tt: &TransportType, tls_acc: Option<&tokio_rustls::TlsAcceptor>, noise_key: Option<&str>) -> Result<BoxedStream> {
    tcp.set_nodelay(true)?;
    match tt {
        TransportType::Tcp => Ok(BoxedStream::new(tcp)),
        TransportType::Tls => {
            let acc = tls_acc.ok_or_else(|| anyhow::anyhow!("TLS acceptor missing"))?;
            let s = tls::accept(acc, tcp).await?;
            Ok(BoxedStream::new(s))
        }
        TransportType::Noise => {
            let k = noise_key.unwrap_or("");
            let s = noise::accept(tcp, k).await?;
            Ok(BoxedStream::new(s))
        }
    }
}
