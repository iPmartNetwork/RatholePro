use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::sync::Arc;
use tokio::sync::Mutex;

pub const FLAG_DATA: u8 = 0x00;
pub const FLAG_SYN: u8 = 0x01;
pub const FLAG_FIN: u8 = 0x02;

const MUX_HEADER_SIZE: usize = 9;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamId(pub u32);

pub struct Multiplexer {
    next_stream_id: Arc<Mutex<u32>>,
}

impl Multiplexer {
    pub fn server() -> Self {
        Self { next_stream_id: Arc::new(Mutex::new(2)) }
    }

    pub fn client() -> Self {
        Self { next_stream_id: Arc::new(Mutex::new(1)) }
    }

    pub async fn next_id(&self) -> StreamId {
        let mut id = self.next_stream_id.lock().await;
        let current = *id;
        *id += 2;
        StreamId(current)
    }
}

pub fn encode_header(stream_id: StreamId, flags: u8, length: u32) -> [u8; MUX_HEADER_SIZE] {
    let mut h = [0u8; MUX_HEADER_SIZE];
    h[0..4].copy_from_slice(&stream_id.0.to_be_bytes());
    h[4] = flags;
    h[5..9].copy_from_slice(&length.to_be_bytes());
    h
}

pub async fn write_data_frame<W: AsyncWriteExt + Unpin>(w: &mut W, id: StreamId, data: &[u8]) -> Result<()> {
    let h = encode_header(id, FLAG_DATA, data.len() as u32);
    w.write_all(&h).await?;
    w.write_all(data).await?;
    w.flush().await?;
    Ok(())
}

pub async fn write_syn_frame<W: AsyncWriteExt + Unpin>(w: &mut W, id: StreamId) -> Result<()> {
    let h = encode_header(id, FLAG_SYN, 0);
    w.write_all(&h).await?;
    w.flush().await?;
    Ok(())
}

pub async fn write_fin_frame<W: AsyncWriteExt + Unpin>(w: &mut W, id: StreamId) -> Result<()> {
    let h = encode_header(id, FLAG_FIN, 0);
    w.write_all(&h).await?;
    w.flush().await?;
    Ok(())
}

pub async fn read_frame<R: AsyncReadExt + Unpin>(r: &mut R) -> Result<(StreamId, u8, Vec<u8>)> {
    let mut h = [0u8; MUX_HEADER_SIZE];
    r.read_exact(&mut h).await?;
    let stream_id = u32::from_be_bytes([h[0], h[1], h[2], h[3]]);
    let flags = h[4];
    let length = u32::from_be_bytes([h[5], h[6], h[7], h[8]]) as usize;
    let mut payload = vec![0u8; length];
    if length > 0 { r.read_exact(&mut payload).await?; }
    Ok((StreamId(stream_id), flags, payload))
}
