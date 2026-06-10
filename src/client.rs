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

/// Run the client
pub async fn run(config: Config) -> Result<()> {
    let client_config = config
        .client
        .ok_or_else(|| anyhow::anyhow!("No [client] block in config"))?;

    let client_config = Arc::new(client_config);
    tracing::info!("Server: {}", client_config.remote_addr);
    tracing::info!(
        "Transport: {}",
        transport::get_transport_type(&client_config.transport)
    );
    tracing::info!(
        "Services: {:?}",
        client_config.services.keys().collect::<Vec<_>>()
    );

    let mut handles = Vec::new();
    for (name, service) in client_config.services.iter() {
        let name = name.clone();
        let service = service.clone();
        let cfg = client_config.clone();

        handles.push(tokio::spawn(async move {
            loop {
                match connect_service(&name, &service, &cfg).await {
                    Ok(_) => tracing::info!("Service '{}' disconnected", name),
                    Err(e) => tracing::error!("Service '{}' error: {}", name, e),
                }
                let retry = service.retry_interval.or(cfg.retry_interval).unwrap_or(3);
                tracing::info!("Retry '{}' in {}s...", name, retry);
                tokio::time::sleep(Duration::from_secs(retry)).await;
            }
        }));
    }

    for h in handles {
        h.await?;
    }
    Ok(())
}

async fn connect_service(
    name: &str,
    service: &ClientServiceConfig,
    config: &ClientConfig,
) -> Result<()> {
    // Establish transport connection (TCP/TLS/Noise/WS)
    let stream = transport::client_connect(&config.remote_addr, &config.transport).await?;
    tracing::info!("Connected to {} for '{}'", config.remote_addr, name);

    let mut framed = Framed::new(stream, MessageCodec);

    // Get token
    let token = service
        .token
        .as_ref()
        .or(config.default_token.as_ref())
        .ok_or_else(|| anyhow::anyhow!("No token for '{}'", name))?;

    let mux_streams = service.mux_streams.or(config.mux_connections).unwrap_or(4);
    let mux_enabled = mux_streams > 0;

    // Authenticate
    framed
        .send(Message::Auth(AuthRequest {
            service_name: name.to_string(),
            token_hash: protocol::hash_token(token),
            service_type: service.service_type.clone(),
            mux_enabled,
            mux_streams,
        }))
        .await?;

    // Wait for response
    let resp = match framed.next().await {
        Some(Ok(Message::AuthResp(r))) => r,
        Some(Ok(_)) => return Err(anyhow::anyhow!("Unexpected response")),
        Some(Err(e)) => return Err(e),
        None => return Err(anyhow::anyhow!("Connection closed")),
    };

    if !resp.success {
        return Err(anyhow::anyhow!("Auth failed: {}", resp.message));
    }

    tracing::info!("Authenticated for '{}' (mux={}, type={})", name, mux_enabled, service.service_type);

    let control = framed.into_inner();

    // Dispatch based on service type
    if service.service_type == "udp" {
        handle_udp_client(control, &service.local_addr, name).await
    } else if mux_enabled {
        handle_mux_client(control, &service.local_addr, name).await
    } else {
        handle_simple_client(control, &service.local_addr).await
    }
}

/// Mux mode: accept SYN frames from server, forward to local service
async fn handle_mux_client(
    control: TransportStream,
    local_addr: &str,
    service_name: &str,
) -> Result<()> {
    tracing::info!("Mux client active for '{}'", service_name);

    let (mut ctrl_read, mut ctrl_write) = tokio::io::split(control);
    let ctrl_write = Arc::new(Mutex::new(ctrl_write));
    let local_addr = local_addr.to_string();

    loop {
        let (stream_id, flags, _payload) = match mux::read_frame(&mut ctrl_read).await {
            Ok(f) => f,
            Err(e) => {
                tracing::debug!("Mux read ended: {}", e);
                break;
            }
        };

        if flags == 0x01 {
            // SYN - new visitor connection from server
            let local_addr = local_addr.clone();
            let ctrl_write = ctrl_write.clone();

            tokio::spawn(async move {
                if let Err(e) = handle_local_forward(stream_id, &local_addr, ctrl_write).await {
                    tracing::debug!("Stream {} error: {}", stream_id.0, e);
                }
            });
        }
        // DATA frames for existing streams are handled within their tasks
        // via a shared routing mechanism (simplified here)
    }

    Ok(())
}

/// Forward a single mux stream to local service
async fn handle_local_forward(
    stream_id: StreamId,
    local_addr: &str,
    ctrl_write: Arc<Mutex<tokio::io::WriteHalf<TransportStream>>>,
) -> Result<()> {
    let local = TcpStream::connect(local_addr).await?;
    local.set_nodelay(true)?;

    let (mut local_read, _local_write) = local.into_split();

    // Local -> Server (via mux data frame)
    let cw = ctrl_write.clone();
    let sid = stream_id;
    let mut buf = vec![0u8; 8192];
    loop {
        match local_read.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                let mut w = cw.lock().await;
                if mux::write_data_frame(&mut *w, sid, &buf[..n]).await.is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    // Send FIN when done
    let mut w = ctrl_write.lock().await;
    let _ = mux::write_fin_frame(&mut *w, stream_id).await;

    Ok(())
}

/// Simple mode: direct TCP relay
async fn handle_simple_client(control: TransportStream, local_addr: &str) -> Result<()> {
    let local = TcpStream::connect(local_addr).await?;
    local.set_nodelay(true)?;

    let (mut l_read, mut l_write) = local.into_split();
    let (mut c_read, mut c_write) = tokio::io::split(control);

    tokio::select! {
        r = tokio::io::copy(&mut c_read, &mut l_write) => {
            if let Err(e) = r { tracing::debug!("Simple ended: {}", e); }
        }
        r = tokio::io::copy(&mut l_read, &mut c_write) => {
            if let Err(e) = r { tracing::debug!("Simple ended: {}", e); }
        }
    }
    Ok(())
}

/// UDP mode: forward UDP packets between mux and local UDP service
async fn handle_udp_client(
    control: TransportStream,
    local_addr: &str,
    service_name: &str,
) -> Result<()> {
    tracing::info!("UDP client active for '{}'", service_name);

    let (mut ctrl_read, ctrl_write) = tokio::io::split(control);
    let ctrl_write = Arc::new(Mutex::new(ctrl_write));

    // Channel for mux frames -> UDP sender
    let (tx, rx) = tokio::sync::mpsc::channel::<(StreamId, Vec<u8>)>(256);
    let rx = Arc::new(Mutex::new(rx));

    // Task: read mux frames from server, put into channel
    let tx_clone = tx.clone();
    let read_task = tokio::spawn(async move {
        loop {
            match mux::read_frame(&mut ctrl_read).await {
                Ok((stream_id, flags, payload)) => {
                    if flags == 0x02 {
                        continue; // FIN, ignore for UDP
                    }
                    if tx_clone.send((stream_id, payload)).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Run UDP client forwarder
    let result = crate::udp::run_client_udp(local_addr, ctrl_write, rx).await;

    read_task.abort();
    result
}
