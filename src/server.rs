use crate::config::Config;
use crate::mux::{self, Multiplexer};
use crate::protocol::{self, AuthResponse, Message, MessageCodec};
use anyhow::Result;
use futures::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio_util::codec::Framed;
use uuid::Uuid;

pub async fn run(config: Config) -> Result<()> {
    let sc = config.server.ok_or_else(|| anyhow::anyhow!("No [server]"))?;
    let listener = TcpListener::bind(&sc.bind_addr).await?;
    tracing::info!("Server on {}", sc.bind_addr);

    // Start TCP service listeners
    let mut svc_map: HashMap<String, Arc<TcpListener>> = HashMap::new();
    for (name, svc) in &sc.services {
        if svc.service_type == "udp" {
            tracing::info!("  [{}] UDP on {}", name, svc.bind_addr.as_deref().unwrap_or("?"));
            continue;
        }
        if let Some(ref addr) = svc.bind_addr {
            let l = TcpListener::bind(addr).await?;
            tracing::info!("  [{}] TCP on {}", name, addr);
            svc_map.insert(name.clone(), Arc::new(l));
        }
    }
    let svc_map = Arc::new(svc_map);
    let sc = Arc::new(sc);

    loop {
        let (stream, addr) = listener.accept().await?;
        let cfg = sc.clone();
        let svcs = svc_map.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(stream, cfg, svcs).await {
                tracing::error!("{}: {}", addr, e);
            }
        });
    }
}

async fn handle(stream: TcpStream, config: Arc<crate::config::ServerConfig>, svcs: Arc<HashMap<String, Arc<TcpListener>>>) -> Result<()> {
    let _ = stream.set_nodelay(true);
    let mut framed = Framed::new(stream, MessageCodec);
    let auth = match framed.next().await {
        Some(Ok(Message::Auth(a))) => a,
        _ => return Err(anyhow::anyhow!("Expected Auth")),
    };
    let svc = config.services.get(&auth.service_name)
        .ok_or_else(|| anyhow::anyhow!("Unknown service"))?;
    let token = svc.token.as_ref().or(config.default_token.as_ref())
        .ok_or_else(|| anyhow::anyhow!("No token"))?;
    if auth.token_hash != protocol::hash_token(token) {
        framed.send(Message::AuthResp(AuthResponse { success: false, message: "Bad token".into(), session_id: None })).await?;
        return Ok(());
    }
    let sid = Uuid::new_v4().to_string();
    framed.send(Message::AuthResp(AuthResponse { success: true, message: "OK".into(), session_id: Some(sid) })).await?;
    let tcp = framed.into_inner();

    if svc.service_type == "udp" {
        let addr = svc.bind_addr.as_deref().ok_or_else(|| anyhow::anyhow!("UDP needs bind_addr"))?;
        return udp_server(tcp, addr).await;
    }

    let listener = svcs.get(&auth.service_name).ok_or_else(|| anyhow::anyhow!("No listener"))?.clone();
    if auth.mux_enabled { mux_srv(tcp, listener).await } else { simple_srv(tcp, listener).await }
}

async fn udp_server(tcp: TcpStream, bind_addr: &str) -> Result<()> {
    let (mut cr, cw) = tcp.into_split();
    let cw = Arc::new(Mutex::new(cw));
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let read_task = tokio::spawn(async move {
        loop {
            match mux::read_frame(&mut cr).await {
                Ok((sid, _f, data)) => { let _ = tx.send((sid, data)).await; }
                Err(_) => break,
            }
        }
    });
    let _ = crate::udp::server_udp(bind_addr, cw, rx).await;
    read_task.abort();
    Ok(())
}

async fn mux_srv(tcp: TcpStream, listener: Arc<TcpListener>) -> Result<()> {
    let (mut cr, cw) = tcp.into_split();
    let cw = Arc::new(Mutex::new(cw));
    let muxer = Multiplexer::server();
    let streams: Arc<Mutex<HashMap<u32, tokio::sync::mpsc::Sender<Vec<u8>>>>> = Arc::new(Mutex::new(HashMap::new()));
    let sc = streams.clone();
    tokio::spawn(async move {
        loop {
            match mux::read_frame(&mut cr).await {
                Ok((id, f, data)) => {
                    if f == mux::FLAG_FIN { sc.lock().await.remove(&id.0); continue; }
                    if let Some(tx) = sc.lock().await.get(&id.0) { let _ = tx.send(data).await; }
                }
                Err(_) => break,
            }
        }
    });
    loop {
        let (v, _) = listener.accept().await?;
        let _ = v.set_nodelay(true);
        let id = muxer.next_id().await;
        let cw2 = cw.clone();
        { let mut w = cw2.lock().await; mux::write_syn_frame(&mut *w, id).await?; }
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
        streams.lock().await.insert(id.0, tx);
        let cw3 = cw.clone();
        tokio::spawn(async move {
            let (mut vr, mut vw) = v.into_split();
            let w = cw3; let sid = id;
            let t1 = tokio::spawn(async move {
                let mut buf = [0u8; 8192];
                loop { match vr.read(&mut buf).await { Ok(0) | Err(_) => break, Ok(n) => { let mut ww = w.lock().await; if mux::write_data_frame(&mut *ww, sid, &buf[..n]).await.is_err() { break; } } } }
            });
            let t2 = tokio::spawn(async move { while let Some(d) = rx.recv().await { if vw.write_all(&d).await.is_err() { break; } } });
            let _ = tokio::join!(t1, t2);
        });
    }
}

async fn simple_srv(tcp: TcpStream, listener: Arc<TcpListener>) -> Result<()> {
    let (v, _) = listener.accept().await?;
    let _ = v.set_nodelay(true);
    let (mut vr, mut vw) = v.into_split();
    let (mut cr, mut cw) = tcp.into_split();
    tokio::select! { _ = tokio::io::copy(&mut vr, &mut cw) => {} _ = tokio::io::copy(&mut cr, &mut vw) => {} }
    Ok(())
}
