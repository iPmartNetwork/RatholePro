use crate::config::{ClientConfig, ClientServiceConfig, Config};
use crate::mux::{self, StreamId};
use crate::protocol::{self, AuthRequest, AuthResponse, Message, MessageCodec};
use anyhow::Result;
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::codec::Framed;

pub async fn run(config: Config) -> Result<()> {
    let cc = config.client.ok_or_else(|| anyhow::anyhow!("No [client] config"))?;
    let cc = Arc::new(cc);
    tracing::info!("Server: {}", cc.remote_addr);

    let mut handles = Vec::new();
    for (name, svc) in cc.services.iter() {
        let n = name.clone();
        let s = svc.clone();
        let c = cc.clone();
        handles.push(tokio::spawn(async move {
            loop {
                if let Err(e) = connect(&n, &s, &c).await {
                    tracing::error!("'{}': {}", n, e);
                }
                let r = s.retry_interval.or(c.retry_interval).unwrap_or(3);
                tokio::time::sleep(Duration::from_secs(r)).await;
            }
        }));
    }
    for h in handles { h.await?; }
    Ok(())
}

async fn connect(name: &str, svc: &ClientServiceConfig, cfg: &ClientConfig) -> Result<()> {
    let stream = TcpStream::connect(&cfg.remote_addr).await?;
    stream.set_nodelay(true)?;
    let mut framed = Framed::new(stream, MessageCodec);

    let token = svc.token.as_ref().or(cfg.default_token.as_ref())
        .ok_or_else(|| anyhow::anyhow!("No token for '{}'", name))?;
    let mux_streams = svc.mux_streams.or(cfg.mux_connections).unwrap_or(4);
    let mux_enabled = mux_streams > 0;

    framed.send(Message::Auth(AuthRequest {
        service_name: name.to_string(),
        token_hash: protocol::hash_token(token),
        service_type: svc.service_type.clone(),
        mux_enabled,
        mux_streams,
    })).await?;

    let resp = match framed.next().await {
        Some(Ok(Message::AuthResp(r))) => r,
        _ => return Err(anyhow::anyhow!("Bad response")),
    };
    if !resp.success { return Err(anyhow::anyhow!("Auth failed: {}", resp.message)); }
    tracing::info!("Auth OK '{}'", name);

    let tcp = framed.into_inner();
    if mux_enabled {
        mux_client(tcp, &svc.local_addr).await
    } else {
        simple_client(tcp, &svc.local_addr).await
    }
}

async fn mux_client(control: TcpStream, local_addr: &str) -> Result<()> {
    let (mut cr, cw) = control.into_split();
    let cw = Arc::new(Mutex::new(cw));
    let la = local_addr.to_string();

    loop {
        let (id, flags, _) = match mux::read_frame(&mut cr).await {
            Ok(f) => f,
            Err(_) => break,
        };
        if flags == mux::FLAG_SYN {
            let la2 = la.clone();
            let cw2 = cw.clone();
            tokio::spawn(async move {
                let _ = forward(id, &la2, cw2).await;
            });
        }
    }
    Ok(())
}

async fn forward(
    id: StreamId,
    local_addr: &str,
    cw: Arc<Mutex<tokio::net::tcp::OwnedWriteHalf>>,
) -> Result<()> {
    let local = TcpStream::connect(local_addr).await?;
    let _ = local.set_nodelay(true);
    let (mut lr, _) = local.into_split();
    let mut buf = vec![0u8; 8192];
    loop {
        match lr.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut w = cw.lock().await;
                if mux::write_data_frame(&mut *w, id, &buf[..n]).await.is_err() { break; }
            }
        }
    }
    let mut w = cw.lock().await;
    let _ = mux::write_fin_frame(&mut *w, id).await;
    Ok(())
}

async fn simple_client(control: TcpStream, local_addr: &str) -> Result<()> {
    let local = TcpStream::connect(local_addr).await?;
    let _ = local.set_nodelay(true);
    let (mut lr, mut lw) = local.into_split();
    let (mut cr, mut cw) = control.into_split();
    tokio::select! {
        _ = tokio::io::copy(&mut cr, &mut lw) => {}
        _ = tokio::io::copy(&mut lr, &mut cw) => {}
    }
    Ok(())
}
