use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Multiplexer wrapper providing multiple logical streams over one TCP connection.
/// Uses a lightweight custom multiplexing protocol.
///
/// Frame format per stream:
/// [STREAM_ID: u32] [FLAGS: u8] [LENGTH: u32] [PAYLOAD: N bytes]

pub const FLAG_DATA: u8 = 0x00;
pub const FLAG_SYN: u8 = 0x01;   // Open new stream
pub const FLAG_FIN: u8 = 0x02;   // Close stream
pub const FLAG_ACK: u8 = 0x03;   // Acknowledge stream open

const MUX_HEADER_SIZE: usize = 9; // 4 + 1 + 4

/// A multiplexed stream identifier
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamId(pub u32);

/// Multiplexer that manages multiple streams over a single connection
pub struct Multiplexer {
    next_stream_id: Arc<Mutex<u32>>,
    is_client: bool,
}

impl Multiplexer {
    /// Create a new multiplexer in client mode (odd stream IDs)
    pub fn client() -> Self {
        Self {
            next_stream_id: Arc::new(Mutex::new(1)), // Odd IDs for client
            is_client: true,
        }
    }

    /// Create a new multiplexer in server mode (even stream IDs)
    pub fn server() -> Self {
        Self {
            next_stream_id: Arc::new(Mutex::new(2)), // Even IDs for server
            is_client: false,
        }
    }

    /// Allocate the next stream ID
    pub async fn next_id(&self) -> StreamId {
        let mut id = self.next_stream_id.lock().await;
        let current = *id;
        *id += 2; // Skip by 2 to maintain odd/even separation
        StreamId(current)
    }

    /// Check if this is client-side
    pub fn is_client(&self) -> bool {
        self.is_client
    }
}

/// Encode a mux frame header
pub fn encode_frame_header(stream_id: StreamId, flags: u8, length: u32) -> [u8; MUX_HEADER_SIZE] {
    let mut header = [0u8; MUX_HEADER_SIZE];
    header[0..4].copy_from_slice(&stream_id.0.to_be_bytes());
    header[4] = flags;
    header[5..9].copy_from_slice(&length.to_be_bytes());
    header
}

/// Decode a mux frame header
pub fn decode_frame_header(header: &[u8; MUX_HEADER_SIZE]) -> (StreamId, u8, u32) {
    let stream_id = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
    let flags = header[4];
    let length = u32::from_be_bytes([header[5], header[6], header[7], header[8]]);
    (StreamId(stream_id), flags, length)
}

/// Write a data frame to the underlying connection
pub async fn write_data_frame<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    stream_id: StreamId,
    data: &[u8],
) -> Result<()> {
    let header = encode_frame_header(stream_id, FLAG_DATA, data.len() as u32);
    writer.write_all(&header).await?;
    writer.write_all(data).await?;
    writer.flush().await?;
    Ok(())
}

/// Write a SYN frame (open stream request)
pub async fn write_syn_frame<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    stream_id: StreamId,
) -> Result<()> {
    let header = encode_frame_header(stream_id, FLAG_SYN, 0);
    writer.write_all(&header).await?;
    writer.flush().await?;
    Ok(())
}

/// Write a FIN frame (close stream)
pub async fn write_fin_frame<W: AsyncWriteExt + Unpin>(
    writer: &mut W,
    stream_id: StreamId,
) -> Result<()> {
    let header = encode_frame_header(stream_id, FLAG_FIN, 0);
    writer.write_all(&header).await?;
    writer.flush().await?;
    Ok(())
}

/// Read a frame from the underlying connection
/// Returns (stream_id, flags, payload)
pub async fn read_frame<R: AsyncReadExt + Unpin>(
    reader: &mut R,
) -> Result<(StreamId, u8, Vec<u8>)> {
    let mut header = [0u8; MUX_HEADER_SIZE];
    reader.read_exact(&mut header).await?;

    let (stream_id, flags, length) = decode_frame_header(&header);

    let mut payload = vec![0u8; length as usize];
    if length > 0 {
        reader.read_exact(&mut payload).await?;
    }

    Ok((stream_id, flags, payload))
}

/// Connection pool for round-robin distribution
pub struct MuxPool {
    connections: Vec<Arc<Mutex<tokio::net::TcpStream>>>,
    current: Arc<Mutex<usize>>,
}

impl MuxPool {
    pub fn new() -> Self {
        Self {
            connections: Vec::new(),
            current: Arc::new(Mutex::new(0)),
        }
    }

    pub fn add(&mut self, conn: tokio::net::TcpStream) {
        self.connections.push(Arc::new(Mutex::new(conn)));
    }

    pub async fn next(&self) -> Option<Arc<Mutex<tokio::net::TcpStream>>> {
        if self.connections.is_empty() {
            return None;
        }
        let mut idx = self.current.lock().await;
        let conn = self.connections[*idx].clone();
        *idx = (*idx + 1) % self.connections.len();
        Some(conn)
    }

    pub fn len(&self) -> usize {
        self.connections.len()
    }

    pub fn is_empty(&self) -> bool {
        self.connections.is_empty()
    }
}
