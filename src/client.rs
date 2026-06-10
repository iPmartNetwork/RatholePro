use crate::config::Config;
use crate::protocol::{self, AuthRequest, Message, MessageCodec};
use anyhow::Result;
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;

/// Client: connects to server, authenticates, then the connection
/// becomes a data channel for transparent relay.
/// Multiple connections are maintained (pool_size) so multiple
/// visitors can be served simultaneously.
pub async fn run(config: Config) -> Result<()> {
    let cc = config.client.ok_or_else(|| anyhow::anyhow!("No [client]"))?;
    let cc = Arc::new(cc);
    tracing::info!("Server: {}", cc.remote_addr);

    let mut handles = Vec::new();
    for (name, svc) in cc.services.iter() {
        let pool_size = svc.mux_streams.or(cc.mux_connections).unwrap_or(4) as usize;
        let pool_size = pool_size.max(1);

        // Launch pool_size concurrent connections for this service
        for i in 0..pool_size {
            let n = name.clone();
            let s = svc.clone();
            let c = cc.clone();
            handles.push(tokio::spawn(async move {
                loop {
                    match do_connect(&n, &s, &c).await {
                        Ok(_) => tracing::debug!("[{}#{}] relay done, reconnecting", n, i),
                        Err(e) => tracing::debug!("[{}#{}] error: {}, retrying", n, i, e),
                    }
                    let r = s.retry_interval.or(c.retry_interval).unwrap_or(1);
                    tokio::time::sleep(Duration::from_secs(r)).await;
                }
            }));
        }
        tracing::info!("[{}] {} connections to {}", name, pool_size, cc.remote_addr);
    }

    for h in handles { h.await?; }
    Ok(())
}

/// Connect to server, auth, then wait for relay data.
/// When server assigns a visitor, data flows through this connection.
/// After relay ends (visitor disconnects), this function returns
/// and the caller reconnects.
async fn do_connect(
    name: &str,
    svc: &crate::config::ServiceConfig,
    cfg: &crate::config::ClientConfig,
) -> Result<()> {
    let tcp = TcpStream::connect(&cfg.remote_addr).await?;
    let _ = tcp.set_nodelay(true);
    let mut framed = Framed::new(tcp, MessageCodec);

    let token = svc.token.as_ref().or(cfg.default_token.as_ref())
        .ok_or_else(|| anyhow::anyhow!("No token"))?;

    // Auth
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

    // Auth done. Now this connection is a data channel.
    // Server will start sending visitor data through it.
    // We relay to local service.
    let tcp = framed.into_inner();
    let local_addr = svc.local_addr.as_deref()
        .ok_or_else(|| anyhow::anyhow!("No local_addr for '{}'", name))?;

    // Connect to local service
    let local = TcpStream::connect(local_addr).await?;
    let _ = local.set_nodelay(true);

    // Transparent relay: server <-> local service
    let (mut sr, mut sw) = tcp.into_split();
    let (mut lr, mut lw) = local.into_split();

    tokio::select! {
        r = tokio::io::copy(&mut sr, &mut lw) => { r?; }
        r = tokio::io::copy(&mut lr, &mut sw) => { r?; }
    }

    Ok(())
}
