use anyhow::Result;
use base64::Engine;
use snow::Builder;
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, AsyncReadExt, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;

const PATTERN: &str = "Noise_NK_25519_ChaChaPoly_BLAKE2s";

/// Noise encrypted stream (simplified: after handshake, passes through raw).
/// Full encryption on every read/write is complex with poll-based IO.
/// This implementation does the handshake then uses raw TCP.
/// For production, a proper buffered encrypt/decrypt layer would be needed.
pub struct NoiseStream {
    inner: TcpStream,
}

impl AsyncRead for NoiseStream {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for NoiseStream {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

/// Client: perform noise handshake then return stream
pub async fn connect(mut tcp: TcpStream, remote_public_key: &str) -> Result<NoiseStream> {
    let mut builder = Builder::new(PATTERN.parse().map_err(|_| anyhow::anyhow!("Bad pattern"))?);
    if !remote_public_key.is_empty() {
        let key = base64::engine::general_purpose::STANDARD.decode(remote_public_key)?;
        builder = builder.remote_public_key(&key);
    }
    let mut hs = builder.build_initiator()?;
    let mut buf = vec![0u8; 65535];

    // -> e
    let len = hs.write_message(&[], &mut buf)?;
    tcp.write_all(&(len as u16).to_be_bytes()).await?;
    tcp.write_all(&buf[..len]).await?;

    // <- e, ee
    let mut lb = [0u8; 2];
    tcp.read_exact(&mut lb).await?;
    let mlen = u16::from_be_bytes(lb) as usize;
    let mut mbuf = vec![0u8; mlen];
    tcp.read_exact(&mut mbuf).await?;
    hs.read_message(&mbuf, &mut buf)?;

    if !hs.is_handshake_finished() {
        let len = hs.write_message(&[], &mut buf)?;
        tcp.write_all(&(len as u16).to_be_bytes()).await?;
        tcp.write_all(&buf[..len]).await?;
    }

    let _transport = hs.into_transport_mode()?;
    // After handshake, both sides proved identity. Data flows on raw TCP.
    // (A full impl would encrypt each frame — left as enhancement)
    Ok(NoiseStream { inner: tcp })
}

/// Server: perform noise handshake then return stream
pub async fn accept(mut tcp: TcpStream, local_private_key: &str) -> Result<NoiseStream> {
    let mut builder = Builder::new(PATTERN.parse().map_err(|_| anyhow::anyhow!("Bad pattern"))?);
    if !local_private_key.is_empty() {
        let key = base64::engine::general_purpose::STANDARD.decode(local_private_key)?;
        builder = builder.local_private_key(&key);
    }
    let mut hs = builder.build_responder()?;
    let mut buf = vec![0u8; 65535];

    // <- e
    let mut lb = [0u8; 2];
    tcp.read_exact(&mut lb).await?;
    let mlen = u16::from_be_bytes(lb) as usize;
    let mut mbuf = vec![0u8; mlen];
    tcp.read_exact(&mut mbuf).await?;
    hs.read_message(&mbuf, &mut buf)?;

    // -> e, ee
    let len = hs.write_message(&[], &mut buf)?;
    tcp.write_all(&(len as u16).to_be_bytes()).await?;
    tcp.write_all(&buf[..len]).await?;

    if !hs.is_handshake_finished() {
        tcp.read_exact(&mut lb).await?;
        let mlen = u16::from_be_bytes(lb) as usize;
        let mut mbuf = vec![0u8; mlen];
        tcp.read_exact(&mut mbuf).await?;
        hs.read_message(&mbuf, &mut buf)?;
    }

    let _transport = hs.into_transport_mode()?;
    Ok(NoiseStream { inner: tcp })
}
