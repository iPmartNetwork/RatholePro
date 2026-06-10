use crate::config::{Config, ServerConfig};
use crate::mux::{self, Multiplexer};
use crate::protocol::{self, AuthResponse, Message, MessageCodec};
use crate::transport::{self, BoxedStream, TransportType};
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

    let transport_type = sc.transport.as_ref()
        .map(|t| TransportType::from_str(&t.transport_type))
        .unwrap_or(TransportType::Tcp);

    // Create TLS acceptor if needed
    let tls_acceptor = if transport_type == TransportType::Tls {
        let tls_cfg = sc.transport.as_ref().and_then(|t| t.tls.as_ref())
            .ok_or_else(|| anyhow::anyhow!("TLS needs [server.transport.tls]"))?;
        let cert = tls_cfg.cert.as_deref().ok_or_else(|| anyhow::anyhow!("TLS needs cert"))?;
        let key = tls_cfg.key.as_deref().ok_or_else(|| anyhow::anyhow!("TLS needs key"))?;
        Some(transport::tls::create_acceptor(cert, key)?)
    } else { None };

    let noise_key = if transport_type == TransportType::Noise {
        sc.transport.as_ref().and_then(|t| t.noise.as_ref())
            .and_then(|n| n.local_private_key.clone())
    } else { None };

    let listener = TcpListener::bind(&sc.bind_addr).await?;
    tracing::info!("Server on {} [transport={:?}]", sc.bind_addr, transport_type);

    let sc = Arc::new(sc);
    let svc_listeners = start_svc_listeners(&sc).await?;

    loop {
        let (tcp, addr) = listener.accept().await?;
        let cfg = sc.clone();
        let svcs = svc_listeners.clone();
        let tt = transport_type.clone();
        let acc = tls_acceptor.clone();
        let nk = noise_key.clone();

        tokio::spawn(async move {
            let stream = match transport::server_accept(
                tcp, &tt, acc.as_ref(), nk.as_deref()
            ).await {
                Ok(s) => s,
                Err(e) => { tracing::error!("{}: transport: {}", addr, e); return; }
            };
            if let Err(e) = handle(stream, cfg, svcs).await {
                tracing::error!("{}: {}", addr, e);
            }
        });
    }
}

async fn start_svc_listeners(c: &ServerConfig) -> Result<Arc<HashMap<String, Arc<TcpListener>>>> {
    let mut m = HashMap::new();
    for (name, svc) in &c.services {
        if svc.service_type == "udp" {
            tracing::info!("Service '{}' (UDP) on {}", name, svc.bind_addr);
            continue;
        }
        let l = TcpListener::bind(&svc.bind_addr).await?;
        tracing::info!("Service '{}' (TCP) on {}", name, svc.bind_addr);
        m.insert(name.clone(), Arc::new(l));
    }
    Ok(Arc::new(m))
}

async fn handle(
    stream: BoxedStream,
    config: Arc<ServerConfig>,
    svc_listeners: Arc<HashMap<String, Arc<TcpListener>>>,
) -> Result<()> {
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
    tracing::info!("Auth OK '{}' [{}] session={}", auth.service_name, auth.service_type, sid);
    framed.send(Message::AuthResp(AuthResponse { success: true, message: "OK".into(), session_id: Some(sid) })).await?;

    let control = framed.into_inner();

    // UDP service
    if svc.service_type == "udp" {
        return handle_udp(control, &svc.bind_addr).await;
    }

    // TCP service
    let listener = svc_listeners.get(&auth.service_name)
        .ok_or_else(|| anyhow::anyhow!("No listener"))?.clone();

    if auth.mux_enabled {
        mux_server(control, listener).await
    } else {
        simple_server(control, listener).await
    }
}

async fn handle_udp(mut control: BoxedStream, bind_addr: &str) -> Result<()> {
    let (cr, cw) = tokio::io::split(control);
    let cw = Arc::new(Mutex::new(cw));
    let (tx, rx) = tokio::sync::mpsc::channel(256);

    // Read mux frames -> channel
    let mut cr = cr;
    let read_task = tokio::spawn(async move {
        loop {
            match mux::read_frame(&mut cr).await {
                Ok((sid, f, data)) => {
                    if f == mux::FLAG_FIN { continue; }
                    let _ = tx.send((sid, data)).await;
                }
                Err(_) => break,
            }
        }
    });

    let result = crate::udp::server_udp(bind_addr, cw, rx).await;
    read_task.abort();
    result
}

async fn mux_server(mut control: BoxedStream, listener: Arc<TcpListener>) -> Result<()> {
    let (cr, cw) = tokio::io::split(control);
    let cw = Arc::new(Mutex::new(cw));
    let mut cr = cr;
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

async fn simple_server(mut control: BoxedStream, listener: Arc<TcpListener>) -> Result<()> {
    let (visitor, _) = listener.accept().await?;
    let _ = visitor.set_nodelay(true);
    let (mut vr, mut vw) = visitor.into_split();
    let (mut cr, mut cw) = tokio::io::split(control);
    tokio::select! {
        _ = tokio::io::copy(&mut vr, &mut cw) => {}
        _ = tokio::io::copy(&mut cr, &mut vw) => {}
    }
    Ok(())
}
