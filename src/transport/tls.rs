use anyhow::Result;
use rustls::pki_types::ServerName;
use std::io::BufReader;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_rustls::{TlsAcceptor, TlsConnector};
use tokio_rustls::client::TlsStream as ClientTlsStream;
use tokio_rustls::server::TlsStream as ServerTlsStream;

/// Connect as TLS client (uses system/webpki root CAs)
pub async fn connect(tcp: TcpStream, hostname: &str) -> Result<ClientTlsStream<TcpStream>> {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let connector = TlsConnector::from(Arc::new(tls_config));
    let server_name = ServerName::try_from(hostname.to_string())
        .map_err(|e| anyhow::anyhow!("Invalid hostname '{}': {}", hostname, e))?;

    let stream = connector.connect(server_name, tcp).await
        .map_err(|e| anyhow::anyhow!("TLS handshake failed: {}", e))?;

    Ok(stream)
}

/// Create TLS acceptor from cert + key PEM files
pub fn create_acceptor(cert_path: &str, key_path: &str) -> Result<TlsAcceptor> {
    let cert_file = std::fs::File::open(cert_path)
        .map_err(|e| anyhow::anyhow!("Cannot open cert '{}': {}", cert_path, e))?;
    let mut cert_reader = BufReader::new(cert_file);
    let certs: Vec<_> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("Parse certs failed: {}", e))?;

    let key_file = std::fs::File::open(key_path)
        .map_err(|e| anyhow::anyhow!("Cannot open key '{}': {}", key_path, e))?;
    let mut key_reader = BufReader::new(key_file);
    let key = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| anyhow::anyhow!("Parse key failed: {}", e))?
        .ok_or_else(|| anyhow::anyhow!("No private key found"))?;

    let tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| anyhow::anyhow!("TLS config error: {}", e))?;

    Ok(TlsAcceptor::from(Arc::new(tls_config)))
}

/// Accept a TLS connection
pub async fn accept(acceptor: &TlsAcceptor, tcp: TcpStream) -> Result<ServerTlsStream<TcpStream>> {
    let stream = acceptor.accept(tcp).await
        .map_err(|e| anyhow::anyhow!("TLS accept failed: {}", e))?;
    Ok(stream)
}
