use bytes::{Buf, BufMut, BytesMut};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::codec::{Decoder, Encoder};

/// Protocol version
pub const PROTOCOL_VERSION: u8 = 1;

/// Magic bytes for Rathole Pro protocol
pub const MAGIC: &[u8; 4] = b"RHPR";

/// Maximum frame size (16 MB)
pub const MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024;

/// Protocol message types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    /// Client authentication request
    Auth(AuthRequest),
    /// Server authentication response
    AuthResp(AuthResponse),
    /// Heartbeat ping
    Ping,
    /// Heartbeat pong
    Pong,
    /// Request to open a new data channel
    OpenChannel(ChannelRequest),
    /// Response to channel open request
    ChannelReady(ChannelResponse),
    /// Service data frame
    Data(DataFrame),
    /// Disconnect notification
    Disconnect(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequest {
    pub service_name: String,
    pub token_hash: String,
    pub service_type: String,  // "tcp" or "udp"
    pub mux_enabled: bool,
    pub mux_streams: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    pub success: bool,
    pub message: String,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelRequest {
    pub session_id: String,
    pub service_name: String,
    pub channel_id: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelResponse {
    pub channel_id: u32,
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataFrame {
    pub channel_id: u32,
    pub payload: Vec<u8>,
}

/// Hash a token for authentication (never send plaintext)
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

/// Message codec for framing
pub struct MessageCodec;

impl Encoder<Message> for MessageCodec {
    type Error = anyhow::Error;

    fn encode(&mut self, item: Message, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let payload = serde_json::to_vec(&item)
            .map_err(|e| anyhow::anyhow!("Failed to serialize message: {}", e))?;

        let len = payload.len() as u32;
        if len > MAX_FRAME_SIZE {
            return Err(anyhow::anyhow!("Message too large: {} bytes", len));
        }

        // Frame format: [MAGIC(4)] [VERSION(1)] [LENGTH(4)] [PAYLOAD(N)]
        dst.put_slice(MAGIC);
        dst.put_u8(PROTOCOL_VERSION);
        dst.put_u32(len);
        dst.put_slice(&payload);

        Ok(())
    }
}

impl Decoder for MessageCodec {
    type Item = Message;
    type Error = anyhow::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        // Need at least header: MAGIC(4) + VERSION(1) + LENGTH(4) = 9 bytes
        if src.len() < 9 {
            return Ok(None);
        }

        // Check magic
        if &src[0..4] != MAGIC {
            return Err(anyhow::anyhow!("Invalid magic bytes"));
        }

        // Check version
        let version = src[4];
        if version != PROTOCOL_VERSION {
            return Err(anyhow::anyhow!(
                "Unsupported protocol version: {}",
                version
            ));
        }

        // Read length
        let len = u32::from_be_bytes([src[5], src[6], src[7], src[8]]) as usize;

        if len > MAX_FRAME_SIZE as usize {
            return Err(anyhow::anyhow!("Frame too large: {} bytes", len));
        }

        // Check if full frame is available
        let total_len = 9 + len;
        if src.len() < total_len {
            src.reserve(total_len - src.len());
            return Ok(None);
        }

        // Consume header
        src.advance(9);

        // Read payload
        let payload = src.split_to(len);
        let message: Message = serde_json::from_slice(&payload)
            .map_err(|e| anyhow::anyhow!("Failed to deserialize message: {}", e))?;

        Ok(Some(message))
    }
}


