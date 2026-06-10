use crate::config::{ClientConfig, ClientServiceConfig, Config};
use crate::mux::{self, StreamId};
use crate::protocol::{self, AuthRequest, AuthResponse, Message, MessageCodec};
use crate::transport::{self, BoxedStream, TransportType};
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

    let transport_type = cc.transport.as_ref()
        .map(|t| TransportType::from_str(&t.transport_type))
        .unwrap_or(TransportType::Tcp);
    tracing::info!("Server: {} [transport={:?}]", cc.remote_addr, transport_type);

    let mut handles = Vec::new();
    for (name, svc) in cc.services.iter() {
        let n = name.clone();
        let s = svc.clone();
        let c = cc.clone();
        let tt = transport_type.clone();
        handles.push(tokio::spawn(async move {
            loop {
                if let Err(e) = connect_svc(&n, &s, &c, &tt).await {
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

async fn connect_svc(name: &str, svc: &ClientServiceConfig, cfg: &ClientConfig, tt: &TransportType) -> Result<()> {
    let tls_hostname = cfg.transport.as_ref()
        .and_then(|t| t.tls.as_ref())
        .and_then(|t| t.hostname.as_deref());
    let noise_key = cfg.transport.as_ref()
        .and_then(|t| t.noise.as_ref())
        .and_then(|n| n.remote_public_key.as_deref());

    let stream = transport::client_connect(&cfg.remote_addr, tt, tls_hostname, noise_key).await?;
    tracing::info!("Connected for '{}'", name);

    let mut framed = Framed::new(stream, MessageCodec);

    let token = svc.token.as_ref().or(cfg.default_token.as_ref())
        .ok_or_else(|| anyhow::anyhow!("No token for '{}'", name))?;
    let mux_streams = svc.mux_streams.or(cfg.mux_connections).unwrap_or(4);
    let mux_enabled = mux_streams > 0 && svc.service_type != "udp";

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
    tracing::info!("Auth OK '{}' [{}]", name, svc.service_type);

    let control = framed.into_inner();

    if svc.service_type == "udp" {
        handle_udp(control, &svc.local_addr).await
    } else if mux_enabled {
        mux_client(control, &svc.local_addr).await
    } else {
        simple_client(control, &svc.local_addr).await
    }
}

async fn handle_udp(control: BoxedStream, local_addr: &str) -> Result<()> {
    let (cr, cw) = tokio::io::split(control);
    let cw = Arc::new(Mutex::new(cw));
    let (tx, rx) = tokio::sync::mpsc::channel(256);

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

    let result = crate::udp::client_udp(local_addr, cw, rx).await;
    read_task.abort();
    result
}

async fn mux_client(control: BoxedStream, local_addr: &str) -> Result<()> {
    let (cr, cw) = tokio::io::split(control);
    let cw = Arc::new(Mutex::new(cw));
    let la = local_addr.to_string();
    let mut cr = cr;

    loop {
        let (id, flags, _) = match mux::read_frame(&mut cr).await {
            Ok(f) => f,
            Err(_) => break,
        };
        if flags == mux::FLAG_SYN {
            let la2 = la.clone();
            let cw2 = cw.clone();
            tokio::spawn(async move { let _ = forward(id, &la2, cw2).await; });
        }
    }
    Ok(())
}

async fn forward<W: AsyncWriteExt + Unpin + Send>(
    id: StreamId,
    local_addr: &str,
    cw: Arc<Mutex<W>>,
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

async fn simple_client(control: BoxedStream, local_addr: &str) -> Result<()> {
    let local = TcpStream::connect(local_addr).await?;
    let _ = local.set_nodelay(true);
    let (mut lr, mut lw) = local.into_split();
    let (mut cr, mut cw) = tokio::io::split(control);
    tokio::select! {
        _ = tokio::io::copy(&mut cr, &mut lw) => {}
        _ = tokio::io::copy(&mut lr, &mut cw) => {}
    }
    Ok(())
}
