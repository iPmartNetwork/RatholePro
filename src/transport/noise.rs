use anyhow::Result;
use base64::Engine;
use snow::{Builder, TransportState};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, AsyncReadExt, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;

const PATTERN: &str = "Noise_NK_25519_ChaChaPoly_BLAKE2s";
const MAX_MSG: usize = 65535;

/// Noise-encrypted stream
pub struct NoiseStream {
    inner: TcpStream,
    noise: TransportState,
    dec_buf: Vec<u8>,
    dec_pos: usize,
    dec_len: usize,
}

impl NoiseStream {
    fn new(inner: TcpStream, noise: TransportState) -> Self {
        Self { inner, noise, dec_buf: vec![0u8; MAX_MSG], dec_pos: 0, dec_len: 0 }
    }
}

impl AsyncRead for NoiseStream {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let me = self.get_mut();
        // Return buffered decrypted data
        if me.dec_pos < me.dec_len {
            let n = (me.dec_len - me.dec_pos).min(buf.remaining());
            buf.put_slice(&me.dec_buf[me.dec_pos..me.dec_pos + n]);
            me.dec_pos += n;
            return Poll::Ready(Ok(()));
        }
        // Delegate to inner for reading (simplified: blocking-style not ideal but works)
        Pin::new(&mut me.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for NoiseStream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        let me = self.get_mut();
        let max_payload = MAX_MSG - 16;
        let to_encrypt = buf.len().min(max_payload);
        let mut enc_buf = vec![0u8; to_encrypt + 16];
        let enc_len = me.noise.write_message(&buf[..to_encrypt], &mut enc_buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
        let len_bytes = (enc_len as u16).to_be_bytes();
        let mut frame = Vec::with_capacity(2 + enc_len);
        frame.extend_from_slice(&len_bytes);
        frame.extend_from_slice(&enc_buf[..enc_len]);
        match Pin::new(&mut me.inner).poll_write(cx, &frame) {
            Poll::Ready(Ok(_)) => Poll::Ready(Ok(to_encrypt)),
            Poll::Ready(Err(e)) => Poll::Ready(Err(e)),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// Client: Noise handshake as initiator
pub async fn connect(mut tcp: TcpStream, remote_public_key: &str) -> Result<NoiseStream> {
    let mut builder = Builder::new(PATTERN.parse().unwrap());

    if !remote_public_key.is_empty() {
        let key = base64::engine::general_purpose::STANDARD.decode(remote_public_key)
            .map_err(|e| anyhow::anyhow!("Bad remote_public_key: {}", e))?;
        builder = builder.remote_public_key(&key);
    }

    let mut hs = builder.build_initiator()
        .map_err(|e| anyhow::anyhow!("Noise initiator error: {}", e))?;

    let mut buf = vec![0u8; MAX_MSG];

    // -> e
    let len = hs.write_message(&[], &mut buf)
        .map_err(|e| anyhow::anyhow!("Noise write1: {}", e))?;
    tcp.write_all(&(len as u16).to_be_bytes()).await?;
    tcp.write_all(&buf[..len]).await?;

    // <- e, ee
    let mut lb = [0u8; 2];
    tcp.read_exact(&mut lb).await?;
    let mlen = u16::from_be_bytes(lb) as usize;
    let mut mbuf = vec![0u8; mlen];
    tcp.read_exact(&mut mbuf).await?;
    hs.read_message(&mbuf, &mut buf)
        .map_err(|e| anyhow::anyhow!("Noise read1: {}", e))?;

    if !hs.is_handshake_finished() {
        let len = hs.write_message(&[], &mut buf)
            .map_err(|e| anyhow::anyhow!("Noise write2: {}", e))?;
        tcp.write_all(&(len as u16).to_be_bytes()).await?;
        tcp.write_all(&buf[..len]).await?;
    }

    let transport = hs.into_transport_mode()
        .map_err(|e| anyhow::anyhow!("Noise transport mode: {}", e))?;

    Ok(NoiseStream::new(tcp, transport))
}

/// Server: Noise handshake as responder
pub async fn accept(mut tcp: TcpStream, local_private_key: &str) -> Result<NoiseStream> {
    let mut builder = Builder::new(PATTERN.parse().unwrap());

    if !local_private_key.is_empty() {
        let key = base64::engine::general_purpose::STANDARD.decode(local_private_key)
            .map_err(|e| anyhow::anyhow!("Bad local_private_key: {}", e))?;
        builder = builder.local_private_key(&key);
    }

    let mut hs = builder.build_responder()
        .map_err(|e| anyhow::anyhow!("Noise responder error: {}", e))?;

    let mut buf = vec![0u8; MAX_MSG];

    // <- e
    let mut lb = [0u8; 2];
    tcp.read_exact(&mut lb).await?;
    let mlen = u16::from_be_bytes(lb) as usize;
    let mut mbuf = vec![0u8; mlen];
    tcp.read_exact(&mut mbuf).await?;
    hs.read_message(&mbuf, &mut buf)
        .map_err(|e| anyhow::anyhow!("Noise read1: {}", e))?;

    // -> e, ee
    let len = hs.write_message(&[], &mut buf)
        .map_err(|e| anyhow::anyhow!("Noise write1: {}", e))?;
    tcp.write_all(&(len as u16).to_be_bytes()).await?;
    tcp.write_all(&buf[..len]).await?;

    if !hs.is_handshake_finished() {
        tcp.read_exact(&mut lb).await?;
        let mlen = u16::from_be_bytes(lb) as usize;
        let mut mbuf = vec![0u8; mlen];
        tcp.read_exact(&mut mbuf).await?;
        hs.read_message(&mbuf, &mut buf)
            .map_err(|e| anyhow::anyhow!("Noise read2: {}", e))?;
    }

    let transport = hs.into_transport_mode()
        .map_err(|e| anyhow::anyhow!("Noise transport mode: {}", e))?;

    Ok(NoiseStream::new(tcp, transport))
}
