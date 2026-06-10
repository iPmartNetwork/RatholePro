use crate::config::{Config, ServerConfig};
use crate::mux::{self, Multiplexer};
use crate::protocol::{self, AuthResponse, Message, MessageCodec};
use crate::transport::{self, TransportStream};
use anyhow::Result;
use futures::{SinkExt, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;
use tokio_util::codec::Framed;
use uuid::Uuid;

/// Run the server
pub async fn run(config: Config) -> Result<()> {
    let server_config = config
        .server
        .ok_or_else(|| anyhow::anyhow!("No [server] block in config"))?;

    // Create TLS acceptor if transport is TLS
    let tls_acceptor = if transport::get_transport_type(&server_config.transport) == "tls" {
        let tls_config = server_config.transport.as_ref()
            .and_then(|t| t.tls.as_ref())
            .ok_or_else(|| anyhow::anyhow!("TLS transport requires [server.transport.tls]"))?;
        Some(crate::transport::tls::create_acceptor(tls_config)?)
    } else {
        None
    };

    tracing::info!("Binding control channel on {}", server_config.bind_addr);
    tracing::info!("Transport: {}", transport::get_transport_type(&server_config.transport));

    let listener = TcpListener::bind(&server_config.bind_addr).await?;
    let server_config = Arc::new(server_config);
    let tls_acceptor = tls_acceptor.map(Arc::new);

    tracing::info!("Rathole Pro server ready on {}", server_config.bind_addr);
    tracing::info!(
        "Services: {:?}",
        server_config.services.keys().collect::<Vec<_>>()
    );

    // Start service listeners
    let svc_listeners = start_service_listeners(&server_config).await?;

    loop {
        let (tcp_stream, addr) = listener.accept().await?;
        tracing::info!("Client connected from {}", addr);

        let cfg = server_config.clone();
        let listeners = svc_listeners.clone();
        let acceptor = tls_acceptor.clone();

        tokio::spawn(async move {
            // Upgrade to configured transport
            let stream = match transport::server_accept(
                tcp_stream,
                &cfg.transport,
                acceptor.as_deref(),
            ).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("Transport accept failed from {}: {}", addr, e);
                    return;
                }
            };

            if let Err(e) = handle_client(stream, cfg, listeners).await {
                tracing::error!("Client {} error: {}", addr, e);
            }
        });
    }
}

async fn start_service_listeners(
    config: &ServerConfig,
) -> Result<Arc<HashMap<String, Arc<TcpListener>>>> {
    let mut listeners = HashMap::new();
    for (name, svc) in &config.services {
        // Only create TCP listeners for TCP services
        // UDP services don't need a TcpListener (they use UdpSocket)
        if svc.service_type == "udp" {
            tracing::info!("Service '{}' (UDP) will bind on {} when client connects", name, svc.bind_addr);
            continue;
        }
        let listener = TcpListener::bind(&svc.bind_addr).await?;
        tracing::info!("Service '{}' (TCP) exposed on {}", name, svc.bind_addr);
        listeners.insert(name.clone(), Arc::new(listener));
    }
    Ok(Arc::new(listeners))
}

async fn handle_client(
    stream: TransportStream,
    config: Arc<ServerConfig>,
    svc_listeners: Arc<HashMap<String, Arc<TcpListener>>>,
) -> Result<()> {
    let mut framed = Framed::new(stream, MessageCodec);

    // Wait for auth
    let auth = match framed.next().await {
        Some(Ok(Message::Auth(a))) => a,
        Some(Ok(_)) => return Err(anyhow::anyhow!("Expected Auth message")),
        Some(Err(e)) => return Err(e),
        None => return Ok(()),
    };

    // Validate service exists
    let service = match config.services.get(&auth.service_name) {
        Some(s) => s,
        None => {
            send_auth_fail(&mut framed, "Service not found").await?;
            return Ok(());
        }
    };

    // Validate token
    let expected_token = service
        .token
        .as_ref()
        .or(config.default_token.as_ref())
        .ok_or_else(|| anyhow::anyhow!("No token for service"))?;

    if auth.token_hash != protocol::hash_token(expected_token) {
        send_auth_fail(&mut framed, "Invalid token").await?;
        return Ok(());
    }

    let session_id = Uuid::new_v4().to_string();
    tracing::info!(
        "Authenticated: service='{}' session={}",
        auth.service_name,
        session_id
    );

    framed
        .send(Message::AuthResp(AuthResponse {
            success: true,
            message: "OK".into(),
            session_id: Some(session_id.clone()),
        }))
        .await?;

    // Get underlying stream
    let control_stream = framed.into_inner();

    // Dispatch based on service type
    if service.service_type == "udp" {
        handle_udp_server(control_stream, &service.bind_addr, &auth.service_name).await
    } else {
        // Get TCP service listener
        let listener = svc_listeners
            .get(&auth.service_name)
            .ok_or_else(|| anyhow::anyhow!("No TCP listener for service"))?
            .clone();

        if auth.mux_enabled {
            handle_mux_server(control_stream, listener, &auth.service_name).await
        } else {
            handle_simple_server(control_stream, listener).await
        }
    }
}

async fn send_auth_fail(
    framed: &mut Framed<TransportStream, MessageCodec>,
    msg: &str,
) -> Result<()> {
    framed
        .send(Message::AuthResp(AuthResponse {
            success: false,
            message: msg.to_string(),
            session_id: None,
        }))
        .await?;
    Ok(())
}

/// Multiplexed mode: multiple visitors share one connection to client
async fn handle_mux_server(
    control: TransportStream,
    listener: Arc<TcpListener>,
    service_name: &str,
) -> Result<()> {
    tracing::info!("Mux mode active for '{}'", service_name);

    let (mut ctrl_read, mut ctrl_write) = tokio::io::split(control);
    let ctrl_write = Arc::new(Mutex::new(ctrl_write));
    let muxer = Multiplexer::server();

    // Map of active streams -> visitor data senders
    let streams: Arc<Mutex<HashMap<u32, tokio::sync::mpsc::Sender<Vec<u8>>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    // Task: read mux frames from client and dispatch to visitor channels
    let streams_clone = streams.clone();
    let read_task = tokio::spawn(async move {
        loop {
            match mux::read_frame(&mut ctrl_read).await {
                Ok((stream_id, flags, payload)) => {
                    if flags == 0x02 {
                        // FIN
                        streams_clone.lock().await.remove(&stream_id.0);
                        continue;
                    }
                    let map = streams_clone.lock().await;
                    if let Some(tx) = map.get(&stream_id.0) {
                        let _ = tx.send(payload).await;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Accept visitors, assign stream IDs, forward traffic
    loop {
        tokio::select! {
            result = listener.accept() => {
                let (visitor, addr) = result?;
                visitor.set_nodelay(true)?;
                tracing::debug!("Visitor {} for '{}'", addr, service_name);

                let stream_id = muxer.next_id().await;
                let ctrl_write = ctrl_write.clone();
                let streams = streams.clone();

                // Send SYN to client
                {
                    let mut w = ctrl_write.lock().await;
                    mux::write_syn_frame(&mut *w, stream_id).await?;
                }

                // Channel for data from client -> visitor
                let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
                streams.lock().await.insert(stream_id.0, tx);

                tokio::spawn(async move {
                    let (mut v_read, mut v_write) = visitor.into_split();

                    // Visitor -> Client (mux frame)
                    let cw = ctrl_write.clone();
                    let sid = stream_id;
                    let v2c = tokio::spawn(async move {
                        let mut buf = vec![0u8; 8192];
                        loop {
                            match v_read.read(&mut buf).await {
                                Ok(0) => break,
                                Ok(n) => {
                                    let mut w = cw.lock().await;
                                    if mux::write_data_frame(&mut *w, sid, &buf[..n])
                                        .await
                                        .is_err()
                                    {
                                        break;
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                    });

                    // Client -> Visitor (from channel)
                    let c2v = tokio::spawn(async move {
                        while let Some(data) = rx.recv().await {
                            if v_write.write_all(&data).await.is_err() {
                                break;
                            }
                        }
                    });

                    let _ = tokio::join!(v2c, c2v);
                });
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }

    read_task.abort();
    Ok(())
}

/// Simple mode: one visitor per control connection
async fn handle_simple_server(
    control: TransportStream,
    listener: Arc<TcpListener>,
) -> Result<()> {
    let (visitor, _) = listener.accept().await?;
    visitor.set_nodelay(true)?;

    let (mut v_read, mut v_write) = visitor.into_split();
    let (mut c_read, mut c_write) = tokio::io::split(control);

    tokio::select! {
        r = tokio::io::copy(&mut v_read, &mut c_write) => {
            if let Err(e) = r { tracing::debug!("Simple ended: {}", e); }
        }
        r = tokio::io::copy(&mut c_read, &mut v_write) => {
            if let Err(e) = r { tracing::debug!("Simple ended: {}", e); }
        }
    }
    Ok(())
}

/// UDP mode: forward UDP packets between visitors and mux
async fn handle_udp_server(
    control: TransportStream,
    bind_addr: &str,
    service_name: &str,
) -> Result<()> {
    tracing::info!("UDP server active for '{}' on {}", service_name, bind_addr);

    let (mut ctrl_read, ctrl_write) = tokio::io::split(control);
    let ctrl_write = Arc::new(Mutex::new(ctrl_write));

    // Channel for mux frames -> UDP sender
    let (tx, rx) = tokio::sync::mpsc::channel::<(crate::mux::StreamId, Vec<u8>)>(256);
    let rx = Arc::new(Mutex::new(rx));

    // Task: read mux frames from client (responses), put into channel
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

    // Run UDP server forwarder
    let result = crate::udp::run_server_udp(bind_addr, ctrl_write, rx).await;

    read_task.abort();
    result
}
