use anyhow::Result;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;

use crate::mux::{self, StreamId};

const SESSION_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_UDP: usize = 65535;

struct Session {
    stream_id: StreamId,
    last_active: Instant,
    addr: SocketAddr,
}

/// Server-side UDP: listen on bind_addr, forward packets via mux
pub async fn server_udp<W: AsyncWriteExt + Unpin + Send + 'static>(
    bind_addr: &str,
    ctrl_write: Arc<Mutex<W>>,
    mut data_rx: tokio::sync::mpsc::Receiver<(StreamId, Vec<u8>)>,
) -> Result<()> {
    let socket = Arc::new(UdpSocket::bind(bind_addr).await?);
    tracing::info!("UDP listening on {}", bind_addr);

    let sessions: Arc<Mutex<HashMap<u32, Session>>> = Arc::new(Mutex::new(HashMap::new()));
    let addr_map: Arc<Mutex<HashMap<SocketAddr, u32>>> = Arc::new(Mutex::new(HashMap::new()));
    let next_id = Arc::new(Mutex::new(2u32));

    // UDP recv -> mux
    let s1 = socket.clone();
    let sess1 = sessions.clone();
    let addr1 = addr_map.clone();
    let cw = ctrl_write.clone();
    let nid = next_id.clone();
    let recv_task = tokio::spawn(async move {
        let mut buf = vec![0u8; MAX_UDP];
        loop {
            let (len, src) = match s1.recv_from(&mut buf).await {
                Ok(r) => r,
                Err(_) => break,
            };
            let sid = {
                let mut am = addr1.lock().await;
                if let Some(&id) = am.get(&src) {
                    sess1.lock().await.get_mut(&id).map(|s| s.last_active = Instant::now());
                    StreamId(id)
                } else {
                    let mut id = nid.lock().await;
                    let s = *id; *id += 2;
                    am.insert(src, s);
                    sess1.lock().await.insert(s, Session { stream_id: StreamId(s), last_active: Instant::now(), addr: src });
                    StreamId(s)
                }
            };
            let mut w = cw.lock().await;
            let _ = mux::write_data_frame(&mut *w, sid, &buf[..len]).await;
        }
    });

    // Mux -> UDP send back
    let s2 = socket.clone();
    let sess2 = sessions.clone();
    let send_task = tokio::spawn(async move {
        while let Some((sid, data)) = data_rx.recv().await {
            let sess = sess2.lock().await;
            if let Some(s) = sess.get(&sid.0) {
                let _ = s2.send_to(&data, s.addr).await;
            }
        }
    });

    // Cleanup expired sessions
    let sess3 = sessions.clone();
    let addr3 = addr_map.clone();
    let cleanup = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let now = Instant::now();
            let mut sess = sess3.lock().await;
            let mut am = addr3.lock().await;
            let expired: Vec<u32> = sess.iter()
                .filter(|(_, s)| now.duration_since(s.last_active) > SESSION_TIMEOUT)
                .map(|(id, _)| *id).collect();
            for id in expired {
                if let Some(s) = sess.remove(&id) { am.remove(&s.addr); }
            }
        }
    });

    tokio::select! {
        _ = recv_task => {}
        _ = send_task => {}
        _ = cleanup => {}
    }
    Ok(())
}

/// Client-side UDP: forward mux data to local UDP service
pub async fn client_udp<W: AsyncWriteExt + Unpin + Send + 'static>(
    local_addr: &str,
    ctrl_write: Arc<Mutex<W>>,
    mut data_rx: tokio::sync::mpsc::Receiver<(StreamId, Vec<u8>)>,
) -> Result<()> {
    let socket = Arc::new(UdpSocket::bind("0.0.0.0:0").await?);
    let target: SocketAddr = local_addr.parse()
        .map_err(|e| anyhow::anyhow!("Invalid UDP addr '{}': {}", local_addr, e))?;
    socket.connect(target).await?;
    tracing::info!("UDP forwarding to {}", local_addr);

    // Mux -> local UDP
    let s1 = socket.clone();
    let fwd_task = tokio::spawn(async move {
        while let Some((_, data)) = data_rx.recv().await {
            let _ = s1.send(&data).await;
        }
    });

    // Local UDP -> mux
    let s2 = socket.clone();
    let reply_task = tokio::spawn(async move {
        let mut buf = vec![0u8; MAX_UDP];
        let resp_id = StreamId(1);
        loop {
            let len = match s2.recv(&mut buf).await {
                Ok(n) => n,
                Err(_) => break,
            };
            let mut w = ctrl_write.lock().await;
            let _ = mux::write_data_frame(&mut *w, resp_id, &buf[..len]).await;
        }
    });

    tokio::select! {
        _ = fwd_task => {}
        _ = reply_task => {}
    }
    Ok(())
}
