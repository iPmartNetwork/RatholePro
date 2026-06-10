use anyhow::Result;
use crate::mux::{self, StreamId};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;

const TIMEOUT: Duration = Duration::from_secs(60);

struct Sess { id: StreamId, addr: SocketAddr, ts: Instant }

pub async fn server_udp<W: AsyncWriteExt + Unpin + Send + 'static>(
    bind: &str, cw: Arc<Mutex<W>>, mut rx: tokio::sync::mpsc::Receiver<(StreamId, Vec<u8>)>,
) -> Result<()> {
    let sock = Arc::new(UdpSocket::bind(bind).await?);
    tracing::info!("UDP on {}", bind);
    let sessions: Arc<Mutex<HashMap<u32, Sess>>> = Arc::new(Mutex::new(HashMap::new()));
    let addrs: Arc<Mutex<HashMap<SocketAddr, u32>>> = Arc::new(Mutex::new(HashMap::new()));
    let nid = Arc::new(Mutex::new(2u32));

    let s1 = sock.clone(); let sess1 = sessions.clone(); let a1 = addrs.clone(); let cw1 = cw.clone(); let nid1 = nid.clone();
    let t1 = tokio::spawn(async move {
        let mut buf = vec![0u8; 65535];
        loop {
            let (n, src) = match s1.recv_from(&mut buf).await { Ok(r) => r, Err(_) => break };
            let sid = { let mut am = a1.lock().await;
                if let Some(&id) = am.get(&src) { sess1.lock().await.get_mut(&id).map(|s| s.ts = Instant::now()); StreamId(id) }
                else { let mut i = nid1.lock().await; let s = *i; *i += 2; am.insert(src, s);
                    sess1.lock().await.insert(s, Sess { id: StreamId(s), addr: src, ts: Instant::now() }); StreamId(s) }
            };
            let mut w = cw1.lock().await; let _ = mux::write_data_frame(&mut *w, sid, &buf[..n]).await;
        }
    });

    let s2 = sock.clone(); let sess2 = sessions.clone();
    let t2 = tokio::spawn(async move {
        while let Some((sid, data)) = rx.recv().await {
            if let Some(s) = sess2.lock().await.get(&sid.0) { let _ = s2.send_to(&data, s.addr).await; }
        }
    });

    let sess3 = sessions.clone(); let a3 = addrs.clone();
    let t3 = tokio::spawn(async move {
        loop { tokio::time::sleep(Duration::from_secs(30)).await;
            let now = Instant::now(); let mut ss = sess3.lock().await; let mut am = a3.lock().await;
            let exp: Vec<u32> = ss.iter().filter(|(_, s)| now.duration_since(s.ts) > TIMEOUT).map(|(k, _)| *k).collect();
            for k in exp { if let Some(s) = ss.remove(&k) { am.remove(&s.addr); } }
        }
    });

    tokio::select! { _ = t1 => {} _ = t2 => {} _ = t3 => {} }
    Ok(())
}

pub async fn client_udp<W: AsyncWriteExt + Unpin + Send + 'static>(
    local: &str, cw: Arc<Mutex<W>>, mut rx: tokio::sync::mpsc::Receiver<(StreamId, Vec<u8>)>,
) -> Result<()> {
    let sock = Arc::new(UdpSocket::bind("0.0.0.0:0").await?);
    let target: SocketAddr = local.parse()?;
    sock.connect(target).await?;
    tracing::info!("UDP -> {}", local);

    let s1 = sock.clone();
    let t1 = tokio::spawn(async move { while let Some((_, d)) = rx.recv().await { let _ = s1.send(&d).await; } });

    let s2 = sock.clone();
    let t2 = tokio::spawn(async move {
        let mut buf = vec![0u8; 65535]; let rid = StreamId(1);
        loop { match s2.recv(&mut buf).await { Ok(n) => { let mut w = cw.lock().await; let _ = mux::write_data_frame(&mut *w, rid, &buf[..n]).await; } Err(_) => break } }
    });

    tokio::select! { _ = t1 => {} _ = t2 => {} }
    Ok(())
}
