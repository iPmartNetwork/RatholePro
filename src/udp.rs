//! UDP forwarding module for Rathole Pro.
//!
//! UDP packets are encapsulated over the TCP control channel using the mux layer.
//! Each unique UDP source address gets its own mux stream ID.
//!
//! Architecture:
//!   Server side: UdpSocket (bind_addr) <-> Mux Stream <-> Client
//!   Client side: Mux Stream <-> UdpSocket (local_addr)
//!
//! Frame format for UDP-over-TCP:
//!   [STREAM_ID: u32] [FLAGS: u8 = 0x00] [LENGTH: u32] [PAYLOAD: UDP packet]
//!
//! Since UDP is connectionless, we track sessions by source address with a timeout.

use anyhow::Result;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UdpSocket;
use tokio::sync::Mutex;

use crate::mux::{self, StreamId};

/// How long to keep a UDP session alive without activity
const UDP_SESSION_TIMEOUT: Duration = Duration::from_secs(60);

/// Maximum UDP packet size
const MAX_UDP_PACKET: usize = 65535;

/// A tracked UDP session (maps source addr to stream ID)
struct UdpSession {
    stream_id: StreamId,
    last_active: Instant,
    source_addr: SocketAddr,
}

/// Run UDP forwarding on the server side.
/// Receives UDP packets on `bind_addr`, forwards them through mux to client.
pub async fn run_server_udp<W: AsyncWriteExt + Unpin + Send + 'static>(
    bind_addr: &str,
    ctrl_write: Arc<Mutex<W>>,
    mux_to_udp_rx: Arc<Mutex<tokio::sync::mpsc::Receiver<(StreamId, Vec<u8>)>>>,
) -> Result<()> {
    let socket = Arc::new(UdpSocket::bind(bind_addr).await?);
    tracing::info!("UDP service listening on {}", bind_addr);

    // Map: stream_id -> source_addr (for sending back responses)
    let sessions: Arc<Mutex<HashMap<u32, UdpSession>>> = Arc::new(Mutex::new(HashMap::new()));
    // Reverse map: source_addr -> stream_id
    let addr_to_stream: Arc<Mutex<HashMap<SocketAddr, u32>>> = Arc::new(Mutex::new(HashMap::new()));

    let next_stream_id = Arc::new(Mutex::new(2u32)); // Even IDs for server

    // Task 1: Read from UDP socket, forward to mux
    let socket_clone = socket.clone();
    let sessions_clone = sessions.clone();
    let addr_to_stream_clone = addr_to_stream.clone();
    let ctrl_write_clone = ctrl_write.clone();
    let next_id = next_stream_id.clone();

    let udp_to_mux = tokio::spawn(async move {
        let mut buf = vec![0u8; MAX_UDP_PACKET];
        loop {
            match socket_clone.recv_from(&mut buf).await {
                Ok((len, src_addr)) => {
                    // Get or create session for this source
                    let stream_id = {
                        let mut addr_map = addr_to_stream_clone.lock().await;
                        if let Some(&sid) = addr_map.get(&src_addr) {
                            // Update last_active
                            let mut sess_map = sessions_clone.lock().await;
                            if let Some(session) = sess_map.get_mut(&sid) {
                                session.last_active = Instant::now();
                            }
                            StreamId(sid)
                        } else {
                            // New session
                            let mut id = next_id.lock().await;
                            let sid = *id;
                            *id += 2;
                            drop(id);

                            let stream_id = StreamId(sid);
                            addr_map.insert(src_addr, sid);

                            let mut sess_map = sessions_clone.lock().await;
                            sess_map.insert(sid, UdpSession {
                                stream_id,
                                last_active: Instant::now(),
                                source_addr: src_addr,
                            });

                            tracing::debug!("New UDP session: {} -> stream {}", src_addr, sid);
                            stream_id
                        }
                    };

                    // Send through mux
                    let mut w = ctrl_write_clone.lock().await;
                    if let Err(e) = mux::write_data_frame(&mut *w, stream_id, &buf[..len]).await {
                        tracing::debug!("UDP->Mux write error: {}", e);
                        break;
                    }
                }
                Err(e) => {
                    tracing::error!("UDP recv error: {}", e);
                    break;
                }
            }
        }
    });

    // Task 2: Read from mux (responses from client), send back to UDP source
    let socket_clone2 = socket.clone();
    let sessions_clone2 = sessions.clone();

    let mux_to_udp = tokio::spawn(async move {
        let mut rx = mux_to_udp_rx.lock().await;
        while let Some((stream_id, data)) = rx.recv().await {
            let sessions = sessions_clone2.lock().await;
            if let Some(session) = sessions.get(&stream_id.0) {
                if let Err(e) = socket_clone2.send_to(&data, session.source_addr).await {
                    tracing::debug!("UDP send_to error: {}", e);
                }
            }
        }
    });

    // Task 3: Cleanup expired sessions
    let sessions_cleanup = sessions.clone();
    let addr_cleanup = addr_to_stream.clone();
    let cleanup_task = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let now = Instant::now();
            let mut sessions = sessions_cleanup.lock().await;
            let mut addr_map = addr_cleanup.lock().await;

            let expired: Vec<u32> = sessions
                .iter()
                .filter(|(_, s)| now.duration_since(s.last_active) > UDP_SESSION_TIMEOUT)
                .map(|(id, _)| *id)
                .collect();

            for id in expired {
                if let Some(session) = sessions.remove(&id) {
                    addr_map.remove(&session.source_addr);
                    tracing::debug!("UDP session expired: {} (stream {})", session.source_addr, id);
                }
            }
        }
    });

    tokio::select! {
        _ = udp_to_mux => {}
        _ = mux_to_udp => {}
        _ = cleanup_task => {}
    }

    Ok(())
}

/// Run UDP forwarding on the client side.
/// Receives mux frames from server, sends as UDP to local service.
/// Receives UDP responses, sends back through mux.
pub async fn run_client_udp<W: AsyncWriteExt + Unpin + Send + 'static>(
    local_addr: &str,
    ctrl_write: Arc<Mutex<W>>,
    mux_to_udp_rx: Arc<Mutex<tokio::sync::mpsc::Receiver<(StreamId, Vec<u8>)>>>,
) -> Result<()> {
    // We create a single UDP socket to communicate with the local service
    // Bind to 0.0.0.0:0 to get an ephemeral port
    let socket = Arc::new(UdpSocket::bind("0.0.0.0:0").await?);
    let target_addr: SocketAddr = local_addr.parse()
        .map_err(|e| anyhow::anyhow!("Invalid UDP local_addr '{}': {}", local_addr, e))?;

    tracing::info!("UDP client forwarding to {}", local_addr);

    // Connect socket to target (allows using send/recv instead of send_to/recv_from)
    socket.connect(target_addr).await?;

    let socket_clone = socket.clone();
    let ctrl_write_clone = ctrl_write.clone();

    // Task 1: Receive from mux, send to local UDP service
    let mux_to_local = tokio::spawn(async move {
        let mut rx = mux_to_udp_rx.lock().await;
        while let Some((_stream_id, data)) = rx.recv().await {
            if let Err(e) = socket_clone.send(&data).await {
                tracing::debug!("UDP send to local error: {}", e);
                break;
            }
        }
    });

    // Task 2: Receive from local UDP service, send back through mux
    let local_to_mux = tokio::spawn(async move {
        let mut buf = vec![0u8; MAX_UDP_PACKET];
        // Use a fixed stream ID for client->server UDP responses
        let response_stream_id = StreamId(1); // Odd = client-originated

        loop {
            match socket.recv(&mut buf).await {
                Ok(len) => {
                    let mut w = ctrl_write_clone.lock().await;
                    if let Err(e) = mux::write_data_frame(&mut *w, response_stream_id, &buf[..len]).await {
                        tracing::debug!("UDP local->mux write error: {}", e);
                        break;
                    }
                }
                Err(e) => {
                    tracing::debug!("UDP recv from local error: {}", e);
                    break;
                }
            }
        }
    });

    tokio::select! {
        _ = mux_to_local => {}
        _ = local_to_mux => {}
    }

    Ok(())
}
