use anyhow::Result;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use std::fs;
use std::io::BufReader;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_rustls::{TlsAcceptor, TlsConnector};
use tokio_rustls::client::TlsStream as ClientTlsStream;
use tokio_rustls::server::TlsStream as ServerTlsStream;
use crate::config::TlsConfig;

/// Create a TLS client connection over an existing TCP stream
pub async fn connect(
    stream: TcpStream,
    config: &TlsConfig,
    remote_addr: &str,
) -> Result<ClientTlsStream<TcpStream>> {
    let mut root_store = rustls::RootCertStore::empty();

    // Load trusted root CA if provided
    if let Some(ref ca_path) = config.trusted_root {
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
        // Use system/webpki root certificates
        root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    }

    let tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let connector = TlsConnector::from(Arc::new(tls_config));

    // Determine server name for SNI
    let hostname = config
        .hostname
        .as_deref()
        .unwrap_or_else(|| {
            // Extract hostname from remote_addr (strip port)
            remote_addr.split(':').next().unwrap_or("localhost")
        });

    let server_name = ServerName::try_from(hostname.to_string())
        .map_err(|e| anyhow::anyhow!("Invalid server name '{}': {}", hostname, e))?;

    let tls_stream = connector.connect(server_name, stream).await
        .map_err(|e| anyhow::anyhow!("TLS handshake failed: {}", e))?;

    Ok(tls_stream)
}

/// Create a TLS acceptor for server-side connections
pub fn create_acceptor(config: &TlsConfig) -> Result<TlsAcceptor> {
    // Load server certificate chain
    let cert_path = config.trusted_root.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Server TLS requires 'trusted_root' pointing to cert PEM"))?;

    let cert_file = fs::File::open(cert_path)
        .map_err(|e| anyhow::anyhow!("Failed to open cert file: {}", e))?;
    let mut cert_reader = BufReader::new(cert_file);
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("Failed to parse certs: {}", e))?;

    if certs.is_empty() {
        return Err(anyhow::anyhow!("No certificates found in cert file"));
    }

    // Load private key
    let key_path = config.pkcs12.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Server TLS requires 'pkcs12' pointing to key PEM file"))?;

    let key_file = fs::File::open(key_path)
        .map_err(|e| anyhow::anyhow!("Failed to open key file: {}", e))?;
    let mut key_reader = BufReader::new(key_file);
    let key = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| anyhow::anyhow!("Failed to parse private key: {}", e))?
        .ok_or_else(|| anyhow::anyhow!("No private key found in key file"))?;

    let tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| anyhow::anyhow!("Failed to build TLS server config: {}", e))?;

    Ok(TlsAcceptor::from(Arc::new(tls_config)))
}

/// Accept a TLS connection on server side
pub async fn accept(
    acceptor: &TlsAcceptor,
    stream: TcpStream,
) -> Result<ServerTlsStream<TcpStream>> {
    let tls_stream = acceptor.accept(stream).await
        .map_err(|e| anyhow::anyhow!("TLS accept failed: {}", e))?;
    Ok(tls_stream)
}
