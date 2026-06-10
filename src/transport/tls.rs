use anyhow::Result;
use rustls::pki_types::ServerName;
use std::io::BufReader;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_rustls::TlsAcceptor;
use tokio_rustls::TlsConnector;

pub async fn connect(tcp: TcpStream, hostname: &str) -> Result<tokio_rustls::client::TlsStream<TcpStream>> {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();
    let connector = TlsConnector::from(Arc::new(config));
    let name = ServerName::try_from(hostname.to_string())
        .map_err(|_| anyhow::anyhow!("Bad hostname: {}", hostname))?;
    let stream = connector.connect(name, tcp).await?;
    Ok(stream)
}

pub fn create_acceptor(cert_path: &str, key_path: &str) -> Result<TlsAcceptor> {
    let cf = std::fs::File::open(cert_path)?;
    let mut cr = BufReader::new(cf);
    let certs: Vec<_> = rustls_pemfile::certs(&mut cr).collect::<std::result::Result<Vec<_>, _>>()?;
    let kf = std::fs::File::open(key_path)?;
    let mut kr = BufReader::new(kf);
    let key = rustls_pemfile::private_key(&mut kr)?.ok_or_else(|| anyhow::anyhow!("No key"))?;
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;
    Ok(TlsAcceptor::from(Arc::new(config)))
}

pub async fn accept(acceptor: &TlsAcceptor, tcp: TcpStream) -> Result<tokio_rustls::server::TlsStream<TcpStream>> {
    let stream = acceptor.accept(tcp).await?;
    Ok(stream)
}
