use anyhow::Result;
use base64::Engine;
use snow::{Builder, TransportState};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, AsyncReadExt, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use crate::config::NoiseConfig;

/// Default Noise pattern
const DEFAULT_PATTERN: &str = "Noise_NK_25519_ChaChaPoly_BLAKE2s";

/// Maximum Noise message size (65535 bytes per spec)
const MAX_NOISE_MSG_SIZE: usize = 65535;

/// Noise-encrypted stream wrapper
pub struct NoiseStream<S> {
    inner: S,
    transport: TransportState,
    read_buf: Vec<u8>,
    read_pos: usize,
    read_len: usize,
}

impl<S: AsyncRead + AsyncWrite + Unpin> NoiseStream<S> {
    /// Wrap a stream with Noise encryption after handshake
    fn new(inner: S, transport: TransportState) -> Self {
        Self {
            inner,
            transport,
            read_buf: vec![0u8; MAX_NOISE_MSG_SIZE],
            read_pos: 0,
            read_len: 0,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for NoiseStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let me = self.get_mut();

        // If we have buffered decrypted data, return that first
        if me.read_pos < me.read_len {
            let available = me.read_len - me.read_pos;
            let to_copy = available.min(buf.remaining());
            buf.put_slice(&me.read_buf[me.read_pos..me.read_pos + to_copy]);
            me.read_pos += to_copy;
            return Poll::Ready(Ok(()));
        }

        // Read length prefix (2 bytes big-endian)
        let mut len_buf = [0u8; 2];
        let mut len_read_buf = ReadBuf::new(&mut len_buf);
        match Pin::new(&mut me.inner).poll_read(cx, &mut len_read_buf) {
            Poll::Ready(Ok(())) => {
                if len_read_buf.filled().len() < 2 {
                    return Poll::Ready(Ok(()));
                }
            }
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            Poll::Pending => return Poll::Pending,
        }

        let msg_len = u16::from_be_bytes(len_buf) as usize;
        if msg_len == 0 {
            return Poll::Ready(Ok(()));
        }

        // Read encrypted message
        let mut enc_buf = vec![0u8; msg_len];
        let mut enc_read_buf = ReadBuf::new(&mut enc_buf);
        match Pin::new(&mut me.inner).poll_read(cx, &mut enc_read_buf) {
            Poll::Ready(Ok(())) => {}
            Poll::Ready(Err(e)) => return Poll::Ready(Err(e)),
            Poll::Pending => return Poll::Pending,
        }

        // Decrypt
        let decrypted_len = me.transport.read_message(&enc_buf[..msg_len], &mut me.read_buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        me.read_pos = 0;
        me.read_len = decrypted_len;

        let to_copy = decrypted_len.min(buf.remaining());
        buf.put_slice(&me.read_buf[..to_copy]);
        me.read_pos = to_copy;

        Poll::Ready(Ok(()))
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for NoiseStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let me = self.get_mut();

        // Encrypt the data
        // Noise max payload is ~65517 bytes (65535 - 16 for AEAD tag)
        let max_payload = MAX_NOISE_MSG_SIZE - 16;
        let to_encrypt = buf.len().min(max_payload);

        let mut enc_buf = vec![0u8; to_encrypt + 16]; // payload + AEAD overhead
        let enc_len = me.transport.write_message(&buf[..to_encrypt], &mut enc_buf)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;

        // Write length prefix + encrypted data
        let len_bytes = (enc_len as u16).to_be_bytes();

        // We need to write both length and data atomically-ish
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

/// Perform Noise handshake as initiator (client) and return encrypted stream
pub async fn connect(
    mut stream: TcpStream,
    config: &NoiseConfig,
) -> Result<NoiseStream<TcpStream>> {
    let pattern = config.pattern.as_deref().unwrap_or(DEFAULT_PATTERN);

    let mut builder = Builder::new(pattern.parse()
        .map_err(|e| anyhow::anyhow!("Invalid noise pattern '{}': {}", pattern, e))?);

    // Set local private key if provided
    if let Some(ref key_b64) = config.local_private_key {
        let key = base64::engine::general_purpose::STANDARD.decode(key_b64)
            .map_err(|e| anyhow::anyhow!("Invalid local_private_key base64: {}", e))?;
        builder = builder.local_private_key(&key);
    }

    // Set remote public key if provided
    if let Some(ref key_b64) = config.remote_public_key {
        let key = base64::engine::general_purpose::STANDARD.decode(key_b64)
            .map_err(|e| anyhow::anyhow!("Invalid remote_public_key base64: {}", e))?;
        builder = builder.remote_public_key(&key);
    }

    let mut handshake = builder.build_initiator()
        .map_err(|e| anyhow::anyhow!("Failed to build noise initiator: {}", e))?;

    // Perform handshake
    let mut buf = vec![0u8; MAX_NOISE_MSG_SIZE];

    // -> e (client sends ephemeral key)
    let len = handshake.write_message(&[], &mut buf)
        .map_err(|e| anyhow::anyhow!("Noise handshake write failed: {}", e))?;
    stream.write_all(&(len as u16).to_be_bytes()).await?;
    stream.write_all(&buf[..len]).await?;

    // <- e, ee (server responds)
    let mut len_buf = [0u8; 2];
    stream.read_exact(&mut len_buf).await?;
    let msg_len = u16::from_be_bytes(len_buf) as usize;
    let mut msg_buf = vec![0u8; msg_len];
    stream.read_exact(&mut msg_buf).await?;
    handshake.read_message(&msg_buf, &mut buf)
        .map_err(|e| anyhow::anyhow!("Noise handshake read failed: {}", e))?;

    // Check if handshake is complete (for NK pattern, 2 messages suffice)
    if !handshake.is_handshake_finished() {
        // -> s, se (if needed for pattern)
        let len = handshake.write_message(&[], &mut buf)
            .map_err(|e| anyhow::anyhow!("Noise handshake write 2 failed: {}", e))?;
        stream.write_all(&(len as u16).to_be_bytes()).await?;
        stream.write_all(&buf[..len]).await?;
    }

    let transport = handshake.into_transport_mode()
        .map_err(|e| anyhow::anyhow!("Failed to enter transport mode: {}", e))?;

    Ok(NoiseStream::new(stream, transport))
}

/// Perform Noise handshake as responder (server) and return encrypted stream
pub async fn accept(
    mut stream: TcpStream,
    config: &NoiseConfig,
) -> Result<NoiseStream<TcpStream>> {
    let pattern = config.pattern.as_deref().unwrap_or(DEFAULT_PATTERN);

    let mut builder = Builder::new(pattern.parse()
        .map_err(|e| anyhow::anyhow!("Invalid noise pattern '{}': {}", pattern, e))?);

    // Set local private key if provided
    if let Some(ref key_b64) = config.local_private_key {
        let key = base64::engine::general_purpose::STANDARD.decode(key_b64)
            .map_err(|e| anyhow::anyhow!("Invalid local_private_key base64: {}", e))?;
        builder = builder.local_private_key(&key);
    }

    // Set remote public key if provided (for mutual auth)
    if let Some(ref key_b64) = config.remote_public_key {
        let key = base64::engine::general_purpose::STANDARD.decode(key_b64)
            .map_err(|e| anyhow::anyhow!("Invalid remote_public_key base64: {}", e))?;
        builder = builder.remote_public_key(&key);
    }

    let mut handshake = builder.build_responder()
        .map_err(|e| anyhow::anyhow!("Failed to build noise responder: {}", e))?;

    let mut buf = vec![0u8; MAX_NOISE_MSG_SIZE];

    // <- e (read client's ephemeral)
    let mut len_buf = [0u8; 2];
    stream.read_exact(&mut len_buf).await?;
    let msg_len = u16::from_be_bytes(len_buf) as usize;
    let mut msg_buf = vec![0u8; msg_len];
    stream.read_exact(&mut msg_buf).await?;
    handshake.read_message(&msg_buf, &mut buf)
        .map_err(|e| anyhow::anyhow!("Noise handshake read failed: {}", e))?;

    // -> e, ee (respond with our ephemeral)
    let len = handshake.write_message(&[], &mut buf)
        .map_err(|e| anyhow::anyhow!("Noise handshake write failed: {}", e))?;
    stream.write_all(&(len as u16).to_be_bytes()).await?;
    stream.write_all(&buf[..len]).await?;

    // Check if more messages needed
    if !handshake.is_handshake_finished() {
        // <- s, se
        stream.read_exact(&mut len_buf).await?;
        let msg_len = u16::from_be_bytes(len_buf) as usize;
        let mut msg_buf = vec![0u8; msg_len];
        stream.read_exact(&mut msg_buf).await?;
        handshake.read_message(&msg_buf, &mut buf)
            .map_err(|e| anyhow::anyhow!("Noise handshake read 2 failed: {}", e))?;
    }

    let transport = handshake.into_transport_mode()
        .map_err(|e| anyhow::anyhow!("Failed to enter transport mode: {}", e))?;

    Ok(NoiseStream::new(stream, transport))
}
