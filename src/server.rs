use crate::config::{Config, ServerConfig};
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
    let sc = config.server.ok_or_else(|| anyhow::anyhow!("No [server] config"))?;
    let listener = TcpListener::bind(&sc.bind_addr).await?;
    tracing::info!("Server listening on {}", sc.bind_addr);

    let sc = Arc::new(sc);
    let svc_listeners = start_svc_listeners(&sc).await?;

    loop {
        let (stream, addr) = listener.accept().await?;
        tracing::info!("Client from {}", addr);
        let cfg = sc.clone();
        let svcs = svc_listeners.clone();
        tokio::spawn(async move {
            if let Err(e) = handle(stream, cfg, svcs).await {
                tracing::error!("{}: {}", addr, e);
            }
        });
    }
}

async fn start_svc_listeners(c: &ServerConfig) -> Result<Arc<HashMap<String, Arc<TcpListener>>>> {
    let mut m = HashMap::new();
    for (name, svc) in &c.services {
        if svc.service_type == "udp" { continue; }
        let l = TcpListener::bind(&svc.bind_addr).await?;
        tracing::info!("Service '{}' on {}", name, svc.bind_addr);
        m.insert(name.clone(), Arc::new(l));
    }
    Ok(Arc::new(m))
}

async fn handle(
    stream: TcpStream,
    config: Arc<ServerConfig>,
    svc_listeners: Arc<HashMap<String, Arc<TcpListener>>>,
) -> Result<()> {
    stream.set_nodelay(true)?;
    let mut framed = Framed::new(stream, MessageCodec);

    let auth = match framed.next().await {
        Some(Ok(Message::Auth(a))) => a,
        _ => return Err(anyhow::anyhow!("Expected Auth")),
    };

    let svc = match config.services.get(&auth.service_name) {
        Some(s) => s,
        None => {
            framed.send(Message::AuthResp(AuthResponse { success: false, message: "Not found".into(), session_id: None })).await?;
            return Ok(());
        }
    };

    let token = svc.token.as_ref().or(config.default_token.as_ref())
        .ok_or_else(|| anyhow::anyhow!("No token"))?;
    if auth.token_hash != protocol::hash_token(token) {
        framed.send(Message::AuthResp(AuthResponse { success: false, message: "Bad token".into(), session_id: None })).await?;
        return Ok(());
    }

    let sid = Uuid::new_v4().to_string();
    tracing::info!("Auth OK '{}' session={}", auth.service_name, sid);
    framed.send(Message::AuthResp(AuthResponse { success: true, message: "OK".into(), session_id: Some(sid) })).await?;

    let tcp = framed.into_inner();
    let listener = svc_listeners.get(&auth.service_name)
        .ok_or_else(|| anyhow::anyhow!("No listener"))?.clone();

    if auth.mux_enabled {
        mux_server(tcp, listener).await
    } else {
        simple_server(tcp, listener).await
    }
}

async fn mux_server(control: TcpStream, listener: Arc<TcpListener>) -> Result<()> {
    let (mut cr, cw) = control.into_split();
    let cw = Arc::new(Mutex::new(cw));
    let muxer = Multiplexer::server();
    let streams: Arc<Mutex<HashMap<u32, tokio::sync::mpsc::Sender<Vec<u8>>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let sc = streams.clone();
    let _reader = tokio::spawn(async move {
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
        let (visitor, _) = listener.accept().await?;
        let _ = visitor.set_nodelay(true);
        let id = muxer.next_id().await;
        let cw2 = cw.clone();
        { let mut w = cw2.lock().await; mux::write_syn_frame(&mut *w, id).await?; }

        let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
        streams.lock().await.insert(id.0, tx);

        let cw3 = cw.clone();
        tokio::spawn(async move {
            let (mut vr, mut vw) = visitor.into_split();
            let sid = id;
            let w = cw3;
            let t1 = tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                loop {
                    match vr.read(&mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => { let mut ww = w.lock().await; if mux::write_data_frame(&mut *ww, sid, &buf[..n]).await.is_err() { break; } }
                    }
                }
            });
            let t2 = tokio::spawn(async move {
                while let Some(d) = rx.recv().await { if vw.write_all(&d).await.is_err() { break; } }
            });
            let _ = tokio::join!(t1, t2);
        });
    }
}

async fn simple_server(control: TcpStream, listener: Arc<TcpListener>) -> Result<()> {
    let (visitor, _) = listener.accept().await?;
    let _ = visitor.set_nodelay(true);
    let (mut vr, mut vw) = visitor.into_split();
    let (mut cr, mut cw) = control.into_split();
    tokio::select! {
        _ = tokio::io::copy(&mut vr, &mut cw) => {}
        _ = tokio::io::copy(&mut cr, &mut vw) => {}
    }
    Ok(())
}
