use crate::config::Config;
use crate::mux::{self, StreamId};
use crate::protocol::{self, AuthRequest, Message, MessageCodec};
use anyhow::Result;
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::codec::Framed;

pub async fn run(config: Config) -> Result<()> {
    let cc = config.client.ok_or_else(|| anyhow::anyhow!("No [client]"))?;
    let cc = Arc::new(cc);
    tracing::info!("Server: {}", cc.remote_addr);
    let mut handles = Vec::new();
    for (name, svc) in cc.services.iter() {
        let n = name.clone(); let s = svc.clone(); let c = cc.clone();
        handles.push(tokio::spawn(async move {
            loop {
                if let Err(e) = do_connect(&n, &s, &c).await { tracing::error!("'{}': {}", n, e); }
                let r = s.retry_interval.or(c.retry_interval).unwrap_or(3);
                tokio::time::sleep(Duration::from_secs(r)).await;
            }
        }));
    }
    for h in handles { h.await?; }
    Ok(())
}

async fn do_connect(name: &str, svc: &crate::config::ServiceConfig, cfg: &crate::config::ClientConfig) -> Result<()> {
    let tcp = TcpStream::connect(&cfg.remote_addr).await?;
    let _ = tcp.set_nodelay(true);
    let mut framed = Framed::new(tcp, MessageCodec);
    let token = svc.token.as_ref().or(cfg.default_token.as_ref())
        .ok_or_else(|| anyhow::anyhow!("No token"))?;
    let mux_streams = svc.mux_streams.or(cfg.mux_connections).unwrap_or(4);
    let mux_enabled = mux_streams > 0 && svc.service_type != "udp";
    framed.send(Message::Auth(AuthRequest {
        service_name: name.to_string(),
        token_hash: protocol::hash_token(token),
        service_type: svc.service_type.clone(),
        mux_enabled, mux_streams,
    })).await?;
    let resp = match framed.next().await {
        Some(Ok(Message::AuthResp(r))) => r,
        _ => return Err(anyhow::anyhow!("Bad response")),
    };
    if !resp.success { return Err(anyhow::anyhow!("Auth: {}", resp.message)); }
    tracing::info!("OK '{}'", name);
    let tcp = framed.into_inner();

    if svc.service_type == "udp" {
        let addr = svc.local_addr.as_deref().ok_or_else(|| anyhow::anyhow!("UDP needs local_addr"))?;
        return udp_client(tcp, addr).await;
    }
    let addr = svc.local_addr.as_deref().ok_or_else(|| anyhow::anyhow!("Need local_addr"))?;
    if mux_enabled { mux_cli(tcp, addr).await } else { simple_cli(tcp, addr).await }
}

async fn udp_client(tcp: TcpStream, local_addr: &str) -> Result<()> {
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
    let _ = crate::udp::client_udp(local_addr, cw, rx).await;
    read_task.abort();
    Ok(())
}

async fn mux_cli(tcp: TcpStream, local_addr: &str) -> Result<()> {
    let (mut cr, cw) = tcp.into_split();
    let cw = Arc::new(Mutex::new(cw));
    let la = local_addr.to_string();
    loop {
        let (id, flags, _) = match mux::read_frame(&mut cr).await { Ok(f) => f, Err(_) => break };
        if flags == mux::FLAG_SYN {
            let la2 = la.clone(); let cw2 = cw.clone();
            tokio::spawn(async move { let _ = fwd(id, &la2, cw2).await; });
        }
    }
    Ok(())
}

async fn fwd(id: StreamId, addr: &str, cw: Arc<Mutex<tokio::net::tcp::OwnedWriteHalf>>) -> Result<()> {
    let local = TcpStream::connect(addr).await?;
    let _ = local.set_nodelay(true);
    let (mut lr, _) = local.into_split();
    let mut buf = [0u8; 8192];
    loop { match lr.read(&mut buf).await { Ok(0) | Err(_) => break, Ok(n) => { let mut w = cw.lock().await; if mux::write_data_frame(&mut *w, id, &buf[..n]).await.is_err() { break; } } } }
    let mut w = cw.lock().await; let _ = mux::write_fin_frame(&mut *w, id).await;
    Ok(())
}

async fn simple_cli(tcp: TcpStream, local_addr: &str) -> Result<()> {
    let local = TcpStream::connect(local_addr).await?;
    let _ = local.set_nodelay(true);
    let (mut lr, mut lw) = local.into_split();
    let (mut cr, mut cw) = tcp.into_split();
    tokio::select! { _ = tokio::io::copy(&mut cr, &mut lw) => {} _ = tokio::io::copy(&mut lr, &mut cw) => {} }
    Ok(())
}
