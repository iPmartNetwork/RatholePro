use crate::config::{ClientConfig, ClientServiceConfig, Config};
use crate::mux::{self, StreamId};
use crate::protocol::{self, AuthRequest, AuthResponse, Message, MessageCodec};
use crate::transport::{self, TransportStream};
use anyhow::Result;
use futures::{SinkExt, StreamExt};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_util::codec::Framed;

pub async fn run(config: Config) -> Result<()> {
    let client_config = config
        .client
        .ok_or_else(|| anyhow::anyhow!("No [client] block in config"))?;

    let client_config = Arc::new(client_config);
    tracing::info!("Server: {}", client_config.remote_addr);
    tracing::info!("Transport: {}", transport::get_transport_type(&client_config.transport));
    tracing::info!("Services: {:?}", client_config.services.keys().collect::<Vec<_>>());

    let mut handles = Vec::new();
    for (name, service) in client_config.services.iter() {
        let name = name.clone();
        let service = service.clone();
        let cfg = client_config.clone();
        handles.push(tokio::spawn(async move {
            loop {
                match connect_service(&name, &service, &cfg).await {
                    Ok(_) => tracing::info!("'{}' disconnected", name),
                    Err(e) => tracing::error!("'{}' error: {}", name, e),
                }
                let retry = service.retry_interval.or(cfg.retry_interval).unwrap_or(3);
                tracing::info!("Retry '{}' in {}s", name, retry);
                tokio::time::sleep(Duration::from_secs(retry)).await;
            }
        }));
    }
    for h in handles { h.await?; }
    Ok(())
}

async fn connect_service(name: &str, service: &ClientServiceConfig, config: &ClientConfig) -> Result<()> {
    let stream = transport::client_connect(&config.remote_addr, &config.transport).await?;
    tracing::info!("Connected for '{}'", name);

    let mut framed = Framed::new(stream, MessageCodec);
    let token = service.token.as_ref().or(config.default_token.as_ref())
        .ok_or_else(|| anyhow::anyhow!("No token for '{}'", name))?;

    let mux_streams = service.mux_streams.or(config.mux_connections).unwrap_or(4);
    let mux_enabled = mux_streams > 0 && service.service_type != "udp";

    framed.send(Message::Auth(AuthRequest {
        service_name: name.to_string(),
        token_hash: protocol::hash_token(token),
        service_type: service.service_type.clone(),
        mux_enabled,
        mux_streams,
    })).await?;

    let resp = match framed.next().await {
        Some(Ok(Message::AuthResp(r))) => r,
        Some(Ok(_)) => return Err(anyhow::anyhow!("Unexpected response")),
        Some(Err(e)) => return Err(e),
        None => return Err(anyhow::anyhow!("Connection closed")),
    };
    if !resp.success { return Err(anyhow::anyhow!("Auth failed: {}", resp.message)); }

    tracing::info!("Auth OK '{}' (type={}, mux={})", name, service.service_type, mux_enabled);
    let control = framed.into_inner();

    if service.service_type == "udp" {
        handle_udp_client(control, &service.local_addr, name).await
    } else if mux_enabled {
        handle_mux_client(control, &service.local_addr, name).await
    } else {
        handle_simple_client(control, &service.local_addr).await
    }
}

async fn handle_mux_client(control: TransportStream, local_addr: &str, service_name: &str) -> Result<()> {
    tracing::info!("Mux client for '{}'", service_name);
    let (mut ctrl_read, ctrl_write) = tokio::io::split(control);
    let ctrl_write = Arc::new(Mutex::new(ctrl_write));
    let local_addr = local_addr.to_string();

    loop {
        let (stream_id, flags, _) = match mux::read_frame(&mut ctrl_read).await {
            Ok(f) => f,
            Err(_) => break,
        };
        if flags == mux::FLAG_SYN {
            let la = local_addr.clone();
            let cw = ctrl_write.clone();
            tokio::spawn(async move {
                if let Err(e) = forward_to_local(stream_id, &la, cw).await {
                    tracing::debug!("Stream {} err: {}", stream_id.0, e);
                }
            });
        }
    }
    Ok(())
}

async fn forward_to_local(
    stream_id: StreamId,
    local_addr: &str,
    ctrl_write: Arc<Mutex<tokio::io::WriteHalf<TransportStream>>>,
) -> Result<()> {
    let local = TcpStream::connect(local_addr).await?;
    let _ = local.set_nodelay(true);
    let (mut lr, _lw) = local.into_split();

    let mut buf = vec![0u8; 8192];
    loop {
        match lr.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut w = ctrl_write.lock().await;
                if mux::write_data_frame(&mut *w, stream_id, &buf[..n]).await.is_err() { break; }
            }
        }
    }
    let mut w = ctrl_write.lock().await;
    let _ = mux::write_fin_frame(&mut *w, stream_id).await;
    Ok(())
}

async fn handle_simple_client(control: TransportStream, local_addr: &str) -> Result<()> {
    let local = TcpStream::connect(local_addr).await?;
    let _ = local.set_nodelay(true);
    let (mut lr, mut lw) = local.into_split();
    let (mut cr, mut cw) = tokio::io::split(control);
    tokio::select! {
        r = tokio::io::copy(&mut cr, &mut lw) => { if let Err(e) = r { tracing::debug!("ended: {}", e); } }
        r = tokio::io::copy(&mut lr, &mut cw) => { if let Err(e) = r { tracing::debug!("ended: {}", e); } }
    }
    Ok(())
}

async fn handle_udp_client(control: TransportStream, local_addr: &str, service_name: &str) -> Result<()> {
    tracing::info!("UDP client for '{}'", service_name);
    let (mut ctrl_read, ctrl_write) = tokio::io::split(control);
    let ctrl_write = Arc::new(Mutex::new(ctrl_write));
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let rx = Arc::new(Mutex::new(rx));

    let tx_clone = tx;
    let read_task = tokio::spawn(async move {
        loop {
            match mux::read_frame(&mut ctrl_read).await {
                Ok((sid, flags, payload)) => {
                    if flags == mux::FLAG_FIN { continue; }
                    if tx_clone.send((sid, payload)).await.is_err() { break; }
                }
                Err(_) => break,
            }
        }
    });

    let result = crate::udp::run_client_udp(local_addr, ctrl_write, rx).await;
    read_task.abort();
    result
}
