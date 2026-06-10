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

    let transport_type = transport::get_transport_type(&server_config.transport).to_string();

    // Create TLS acceptor if needed
    #[cfg(feature = "transport-tls")]
    let tls_acceptor = if transport_type == "tls" {
        let tls_config = server_config.transport.as_ref()
            .and_then(|t| t.tls.as_ref())
            .ok_or_else(|| anyhow::anyhow!("TLS requires [server.transport.tls]"))?;
        Some(Arc::new(crate::transport::tls::create_acceptor(tls_config)?))
    } else {
        None
    };

    tracing::info!("Binding on {}", server_config.bind_addr);
    tracing::info!("Transport: {}", transport_type);

    let listener = TcpListener::bind(&server_config.bind_addr).await?;
    let server_config = Arc::new(server_config);

    tracing::info!("RatholePro server ready");
    tracing::info!("Services: {:?}", server_config.services.keys().collect::<Vec<_>>());

    let svc_listeners = start_service_listeners(&server_config).await?;

    loop {
        let (tcp_stream, addr) = listener.accept().await?;
        tracing::info!("Client from {}", addr);

        let cfg = server_config.clone();
        let listeners = svc_listeners.clone();
        #[cfg(feature = "transport-tls")]
        let acceptor = tls_acceptor.clone();

        tokio::spawn(async move {
            let stream = {
                #[cfg(feature = "transport-tls")]
                {
                    match transport::server_accept(tcp_stream, &cfg.transport, acceptor.as_deref()).await {
                        Ok(s) => s,
                        Err(e) => { tracing::error!("Transport error {}: {}", addr, e); return; }
                    }
                }
                #[cfg(not(feature = "transport-tls"))]
                {
                    match transport::server_accept(tcp_stream, &cfg.transport, None).await {
                        Ok(s) => s,
                        Err(e) => { tracing::error!("Transport error {}: {}", addr, e); return; }
                    }
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
        if svc.service_type == "udp" {
            tracing::info!("Service '{}' (UDP) on {}", name, svc.bind_addr);
            continue;
        }
        let listener = TcpListener::bind(&svc.bind_addr).await?;
        tracing::info!("Service '{}' ({}) on {}", name, svc.service_type, svc.bind_addr);
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

    let auth = match framed.next().await {
        Some(Ok(Message::Auth(a))) => a,
        Some(Ok(_)) => return Err(anyhow::anyhow!("Expected Auth")),
        Some(Err(e)) => return Err(e),
        None => return Ok(()),
    };

    let service = match config.services.get(&auth.service_name) {
        Some(s) => s,
        None => {
            let _ = framed.send(Message::AuthResp(AuthResponse {
                success: false, message: "Service not found".into(), session_id: None,
            })).await;
            return Ok(());
        }
    };

    let expected_token = service.token.as_ref()
        .or(config.default_token.as_ref())
        .ok_or_else(|| anyhow::anyhow!("No token configured"))?;

    if auth.token_hash != protocol::hash_token(expected_token) {
        let _ = framed.send(Message::AuthResp(AuthResponse {
            success: false, message: "Invalid token".into(), session_id: None,
        })).await;
        return Ok(());
    }

    let session_id = Uuid::new_v4().to_string();
    tracing::info!("Auth OK: '{}' session={}", auth.service_name, session_id);

    framed.send(Message::AuthResp(AuthResponse {
        success: true, message: "OK".into(), session_id: Some(session_id),
    })).await?;

    let control = framed.into_inner();

    if service.service_type == "udp" {
        handle_udp_server(control, &service.bind_addr, &auth.service_name).await
    } else {
        let listener = svc_listeners.get(&auth.service_name)
            .ok_or_else(|| anyhow::anyhow!("No listener for service"))?
            .clone();
        if auth.mux_enabled {
            handle_mux_server(control, listener, &auth.service_name).await
        } else {
            handle_simple_server(control, listener).await
        }
    }
}

async fn handle_mux_server(
    control: TransportStream,
    listener: Arc<TcpListener>,
    service_name: &str,
) -> Result<()> {
    tracing::info!("Mux mode for '{}'", service_name);
    let (mut ctrl_read, ctrl_write) = tokio::io::split(control);
    let ctrl_write = Arc::new(Mutex::new(ctrl_write));
    let muxer = Multiplexer::server();
    let streams: Arc<Mutex<HashMap<u32, tokio::sync::mpsc::Sender<Vec<u8>>>>> =
        Arc::new(Mutex::new(HashMap::new()));

    let streams_clone = streams.clone();
    let read_task = tokio::spawn(async move {
        loop {
            match mux::read_frame(&mut ctrl_read).await {
                Ok((sid, flags, payload)) => {
                    if flags == mux::FLAG_FIN {
                        streams_clone.lock().await.remove(&sid.0);
                        continue;
                    }
                    if let Some(tx) = streams_clone.lock().await.get(&sid.0) {
                        let _ = tx.send(payload).await;
                    }
                }
                Err(_) => break,
            }
        }
    });

    loop {
        tokio::select! {
            result = listener.accept() => {
                let (visitor, addr) = result?;
                let _ = visitor.set_nodelay(true);
                tracing::debug!("Visitor {}", addr);

                let stream_id = muxer.next_id().await;
                let cw = ctrl_write.clone();
                let streams = streams.clone();

                { let mut w = cw.lock().await; mux::write_syn_frame(&mut *w, stream_id).await?; }

                let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(256);
                streams.lock().await.insert(stream_id.0, tx);

                tokio::spawn(async move {
                    let (mut vr, mut vw) = visitor.into_split();
                    let cw2 = cw.clone();
                    let sid = stream_id;

                    let v2c = tokio::spawn(async move {
                        let mut buf = vec![0u8; 8192];
                        loop {
                            match vr.read(&mut buf).await {
                                Ok(0) | Err(_) => break,
                                Ok(n) => {
                                    let mut w = cw2.lock().await;
                                    if mux::write_data_frame(&mut *w, sid, &buf[..n]).await.is_err() { break; }
                                }
                            }
                        }
                    });
                    let c2v = tokio::spawn(async move {
                        while let Some(data) = rx.recv().await {
                            if vw.write_all(&data).await.is_err() { break; }
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

async fn handle_simple_server(control: TransportStream, listener: Arc<TcpListener>) -> Result<()> {
    let (visitor, _) = listener.accept().await?;
    let _ = visitor.set_nodelay(true);
    let (mut vr, mut vw) = visitor.into_split();
    let (mut cr, mut cw) = tokio::io::split(control);
    tokio::select! {
        r = tokio::io::copy(&mut vr, &mut cw) => { if let Err(e) = r { tracing::debug!("ended: {}", e); } }
        r = tokio::io::copy(&mut cr, &mut vw) => { if let Err(e) = r { tracing::debug!("ended: {}", e); } }
    }
    Ok(())
}

async fn handle_udp_server(
    control: TransportStream,
    bind_addr: &str,
    service_name: &str,
) -> Result<()> {
    tracing::info!("UDP mode for '{}' on {}", service_name, bind_addr);
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
                    let _ = tx_clone.send((sid, payload)).await;
                }
                Err(_) => break,
            }
        }
    });

    let result = crate::udp::run_server_udp(bind_addr, ctrl_write, rx).await;
    read_task.abort();
    result
}
