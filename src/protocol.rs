use bytes::{Buf, BufMut, BytesMut};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio_util::codec::{Decoder, Encoder};

pub const PROTOCOL_VERSION: u8 = 1;
pub const MAGIC: &[u8; 4] = b"RHPR";
pub const MAX_FRAME_SIZE: u32 = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    Auth(AuthRequest),
    AuthResp(AuthResponse),
    Ping,
    Pong,
    Disconnect(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequest {
    pub service_name: String,
    pub token_hash: String,
    pub service_type: String,
    pub mux_enabled: bool,
    pub mux_streams: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    pub success: bool,
    pub message: String,
    pub session_id: Option<String>,
}

pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

pub struct MessageCodec;

impl Encoder<Message> for MessageCodec {
    type Error = anyhow::Error;

    fn encode(&mut self, item: Message, dst: &mut BytesMut) -> Result<(), Self::Error> {
        let payload = serde_json::to_vec(&item)?;
        let len = payload.len() as u32;
        if len > MAX_FRAME_SIZE {
            return Err(anyhow::anyhow!("Message too large"));
        }
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
        if src.len() < 9 { return Ok(None); }
        if &src[0..4] != MAGIC {
            return Err(anyhow::anyhow!("Invalid magic bytes"));
        }
        if src[4] != PROTOCOL_VERSION {
            return Err(anyhow::anyhow!("Unsupported protocol version"));
        }
        let len = u32::from_be_bytes([src[5], src[6], src[7], src[8]]) as usize;
        if len > MAX_FRAME_SIZE as usize {
            return Err(anyhow::anyhow!("Frame too large"));
        }
        if src.len() < 9 + len {
            src.reserve(9 + len - src.len());
            return Ok(None);
        }
        src.advance(9);
        let payload = src.split_to(len);
        let msg: Message = serde_json::from_slice(&payload)?;
        Ok(Some(msg))
    }
}
