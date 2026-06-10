pub mod tls;
pub mod noise;

use anyhow::Result;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

/// Type-erased async stream — works for TCP, TLS, and Noise uniformly
pub type BoxedStream = Box<dyn AsyncRead + AsyncWrite + Unpin + Send>;

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
        TransportType::Tcp => Ok(Box::new(tcp)),
        TransportType::Tls => {
            let hostname = tls_hostname.unwrap_or_else(|| addr.split(':').next().unwrap_or("localhost"));
            let stream = tls::connect(tcp, hostname).await?;
            Ok(Box::new(stream))
        }
        TransportType::Noise => {
            let remote_key = noise_remote_key.unwrap_or("");
            let stream = noise::connect(tcp, remote_key).await?;
            Ok(Box::new(stream))
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
        TransportType::Tcp => Ok(Box::new(tcp)),
        TransportType::Tls => {
            let acceptor = tls_acceptor
                .ok_or_else(|| anyhow::anyhow!("TLS acceptor not configured"))?;
            let stream = tls::accept(acceptor, tcp).await?;
            Ok(Box::new(stream))
        }
        TransportType::Noise => {
            let key = noise_private_key.unwrap_or("");
            let stream = noise::accept(tcp, key).await?;
            Ok(Box::new(stream))
        }
    }
}
