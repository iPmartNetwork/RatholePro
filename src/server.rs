use crate::config::Config;
use crate::protocol::{self, AuthResponse, Message, MessageCodec};
use anyhow::Result;
use futures::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex};
use tokio_util::codec::Framed;
use uuid::Uuid;

/// Server: listens for clients and visitors.
/// For each visitor on service port, picks an idle client data channel
/// and does transparent TCP relay (no framing, no mux).
pub async fn run(config: Config) -> Result<()> {
    let sc = config.server.ok_or_else(|| anyhow::anyhow!("No [server]"))?;
    let control_listener = TcpListener::bind(&sc.bind_addr).await?;
    tracing::info!("Server control on {}", sc.bind_addr);

    let sc = Arc::new(sc);

    // For each service, create a channel of idle data connections (pool)
    // When client connects and auths, its TcpStream goes into the pool.
    // When visitor connects, we take one from the pool and relay.
    let pools: Arc<HashMap<String, Arc<Mutex<Vec<TcpStream>>>>> = {
        let mut m = HashMap::new();
        for name in sc.services.keys() {
            m.insert(name.clone(), Arc::new(Mutex::new(Vec::new())));
        }
        Arc::new(m)
    };

    // Start visitor listeners for each service
    for (name, svc) in &sc.services {
        if let Some(ref addr) = svc.bind_addr {
            let listener = TcpListener::bind(addr).await?;
            tracing::info!("  [{}] visitors on {}", name, addr);
            let pool = pools.get(name).unwrap().clone();
            let svc_name = name.clone();
            tokio::spawn(async move {
                accept_visitors(listener, pool, svc_name).await;
            });
        }
    }

    // Accept client (data channel) connections on control port
    let cfg = sc.clone();
    loop {
        let (stream, addr) = control_listener.accept().await?;
        let pools = pools.clone();
        let config = cfg.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_client_connection(stream, config, pools).await {
                tracing::debug!("{}: {}", addr, e);
            }
        });
    }
}

/// Accept a client connection: auth, then put the raw TcpStream into the pool.
/// The client keeps reconnecting (one connection per relay).
async fn handle_client_connection(
    stream: TcpStream,
    config: Arc<crate::config::ServerConfig>,
    pools: Arc<HashMap<String, Arc<Mutex<Vec<TcpStream>>>>>,
) -> Result<()> {
    let _ = stream.set_nodelay(true);
    let mut framed = Framed::new(stream, MessageCodec);

    // Auth
    let auth = match framed.next().await {
        Some(Ok(Message::Auth(a))) => a,
        _ => return Err(anyhow::anyhow!("Expected Auth")),
    };

    let svc = config.services.get(&auth.service_name)
        .ok_or_else(|| anyhow::anyhow!("Unknown service: {}", auth.service_name))?;
    let token = svc.token.as_ref().or(config.default_token.as_ref())
        .ok_or_else(|| anyhow::anyhow!("No token"))?;
    if auth.token_hash != protocol::hash_token(token) {
        framed.send(Message::AuthResp(AuthResponse {
            success: false, message: "Bad token".into(), session_id: None,
        })).await?;
        return Ok(());
    }

    framed.send(Message::AuthResp(AuthResponse {
        success: true, message: "OK".into(), session_id: Some(Uuid::new_v4().to_string()),
    })).await?;

    tracing::info!("Client ready for '{}'", auth.service_name);

    // Get the raw TcpStream back (no more framing from here)
    let tcp = framed.into_inner();

    // Put into the pool — waiting for a visitor to use it
    let pool = pools.get(&auth.service_name)
        .ok_or_else(|| anyhow::anyhow!("No pool for '{}'", auth.service_name))?;
    pool.lock().await.push(tcp);

    Ok(())
}

/// Accept visitors on a service port. For each visitor,
/// take an idle client connection from pool and relay transparently.
async fn accept_visitors(
    listener: TcpListener,
    pool: Arc<Mutex<Vec<TcpStream>>>,
    service_name: String,
) {
    loop {
        let (visitor, vaddr) = match listener.accept().await {
            Ok(v) => v,
            Err(_) => continue,
        };
        let _ = visitor.set_nodelay(true);

        // Take a client data channel from pool
        let client_stream = {
            let mut p = pool.lock().await;
            if p.is_empty() {
                tracing::warn!("[{}] No client available for visitor {}", service_name, vaddr);
                drop(visitor);
                continue;
            }
            p.remove(0)
        };

        tracing::info!("[{}] Relay: visitor {} <-> client", service_name, vaddr);

        // Transparent relay — raw TCP copy, no framing
        tokio::spawn(async move {
            let _ = relay(visitor, client_stream).await;
        });
    }
}

/// Transparent bidirectional TCP relay (zero modification to bytes)
async fn relay(mut a: TcpStream, mut b: TcpStream) -> Result<()> {
    let (mut ar, mut aw) = a.split();
    let (mut br, mut bw) = b.split();
    tokio::select! {
        r = tokio::io::copy(&mut ar, &mut bw) => { r?; }
        r = tokio::io::copy(&mut br, &mut aw) => { r?; }
    }
    Ok(())
}
