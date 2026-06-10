use crate::config::Config;
use crate::protocol::{self, AuthRequest, Message, MessageCodec};
use anyhow::Result;
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;

pub async fn run(config: Config) -> Result<()> {
    let cc = config.client.ok_or_else(|| anyhow::anyhow!("No [client]"))?;
    let cc = Arc::new(cc);
    tracing::info!("Server: {}", cc.remote_addr);

    let mut handles = Vec::new();
    for (name, svc) in cc.services.iter() {
        let pool_size = svc.mux_streams.or(cc.mux_connections).unwrap_or(4) as usize;
        let pool_size = pool_size.max(1);

        for i in 0..pool_size {
            let n = name.clone();
            let s = svc.clone();
            let c = cc.clone();
            handles.push(tokio::spawn(async move {
                loop {
                    match do_connect(&n, &s, &c).await {
                        Ok(_) => tracing::debug!("[{}#{}] done, reconnecting", n, i),
                        Err(e) => tracing::debug!("[{}#{}] {}, retrying", n, i, e),
                    }
                    let r = s.retry_interval.or(c.retry_interval).unwrap_or(1);
                    tokio::time::sleep(Duration::from_secs(r)).await;
                }
            }));
        }
        tracing::info!("[{}] pool={} to {}", name, pool_size, cc.remote_addr);
    }

    for h in handles { h.await?; }
    Ok(())
}

async fn do_connect(
    name: &str,
    svc: &crate::config::ServiceConfig,
    cfg: &crate::config::ClientConfig,
) -> Result<()> {
    let mut tcp = TcpStream::connect(&cfg.remote_addr).await?;
    let _ = tcp.set_nodelay(true);
    let mut framed = Framed::new(tcp, MessageCodec);

    let token = svc.token.as_ref().or(cfg.default_token.as_ref())
        .ok_or_else(|| anyhow::anyhow!("No token"))?;

    framed.send(Message::Auth(AuthRequest {
        service_name: name.to_string(),
        token_hash: protocol::hash_token(token),
        service_type: svc.service_type.clone(),
        mux_enabled: false,
        mux_streams: 0,
    })).await?;

    let resp = match framed.next().await {
        Some(Ok(Message::AuthResp(r))) => r,
        _ => return Err(anyhow::anyhow!("Bad response")),
    };
    if !resp.success {
        return Err(anyhow::anyhow!("Auth failed: {}", resp.message));
    }

    // Auth done. Connection is now in server's pool.
    // Wait until server sends us data (meaning a visitor connected).
    // Then connect to local and relay.
    let mut tcp = framed.into_inner();
    let local_addr = svc.local_addr.as_deref()
        .ok_or_else(|| anyhow::anyhow!("No local_addr"))?;

    // Wait for first byte from server (visitor data)
    let mut first_byte = [0u8; 1];
    let n = tcp.read(&mut first_byte).await?;
    if n == 0 {
        return Err(anyhow::anyhow!("Server closed connection (no visitor)"));
    }

    // Now we know a visitor is connected — open local
    let mut local = TcpStream::connect(local_addr).await?;
    let _ = local.set_nodelay(true);

    // Send the first byte we already read
    local.write_all(&first_byte[..n]).await?;

    // Transparent relay
    let (mut sr, mut sw) = tcp.into_split();
    let (mut lr, mut lw) = local.into_split();

    tokio::select! {
        r = tokio::io::copy(&mut sr, &mut lw) => { let _ = r; }
        r = tokio::io::copy(&mut lr, &mut sw) => { let _ = r; }
    }

    Ok(())
}
