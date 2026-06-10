use anyhow::Result;
use futures::{SinkExt, StreamExt};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async, accept_async,
    tungstenite::protocol::Message as WsMessage,
    WebSocketStream, MaybeTlsStream,
};

/// WebSocket stream wrapper that implements AsyncRead + AsyncWrite.
/// This allows the rest of the code to treat WebSocket like any other byte stream.
pub struct WsStream {
    inner: WebSocketStream<MaybeTlsStream<TcpStream>>,
    read_buf: Vec<u8>,
    read_pos: usize,
}

/// Server-side WebSocket (no TLS wrapper needed at this level)
pub struct WsServerStream {
    inner: WebSocketStream<TcpStream>,
    read_buf: Vec<u8>,
    read_pos: usize,
}

impl WsStream {
    pub fn new(inner: WebSocketStream<MaybeTlsStream<TcpStream>>) -> Self {
        Self {
            inner,
            read_buf: Vec::new(),
            read_pos: 0,
        }
    }
}

impl WsServerStream {
    pub fn new(inner: WebSocketStream<TcpStream>) -> Self {
        Self {
            inner,
            read_buf: Vec::new(),
            read_pos: 0,
        }
    }
}

impl AsyncRead for WsStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let me = self.get_mut();

        // Return buffered data first
        if me.read_pos < me.read_buf.len() {
            let available = me.read_buf.len() - me.read_pos;
            let to_copy = available.min(buf.remaining());
            buf.put_slice(&me.read_buf[me.read_pos..me.read_pos + to_copy]);
            me.read_pos += to_copy;
            if me.read_pos >= me.read_buf.len() {
                me.read_buf.clear();
                me.read_pos = 0;
            }
            return Poll::Ready(Ok(()));
        }

        // Poll for next WebSocket message
        match Pin::new(&mut me.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(msg))) => {
                match msg {
                    WsMessage::Binary(data) => {
                        let to_copy = data.len().min(buf.remaining());
                        buf.put_slice(&data[..to_copy]);
                        if to_copy < data.len() {
                            me.read_buf = data[to_copy..].to_vec();
                            me.read_pos = 0;
                        }
                        Poll::Ready(Ok(()))
                    }
                    WsMessage::Close(_) => Poll::Ready(Ok(())),
                    WsMessage::Ping(_) | WsMessage::Pong(_) => {
                        // Ignore control frames, wake to poll again
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                    _ => {
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                }
            }
            Poll::Ready(Some(Err(e))) => {
                Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))
            }
            Poll::Ready(None) => Poll::Ready(Ok(())),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for WsStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let me = self.get_mut();
        let msg = WsMessage::Binary(buf.to_vec());

        match Pin::new(&mut me.inner).poll_ready(cx) {
            Poll::Ready(Ok(())) => {
                match Pin::new(&mut me.inner).start_send(msg) {
                    Ok(()) => Poll::Ready(Ok(buf.len())),
                    Err(e) => Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::Other, e.to_string()
                    ))),
                }
            }
            Poll::Ready(Err(e)) => {
                Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let me = self.get_mut();
        match Pin::new(&mut me.inner).poll_flush(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(e)) => {
                Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let me = self.get_mut();
        match Pin::new(&mut me.inner).poll_close(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(e)) => {
                Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

// Same implementation for server-side stream
impl AsyncRead for WsServerStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let me = self.get_mut();

        if me.read_pos < me.read_buf.len() {
            let available = me.read_buf.len() - me.read_pos;
            let to_copy = available.min(buf.remaining());
            buf.put_slice(&me.read_buf[me.read_pos..me.read_pos + to_copy]);
            me.read_pos += to_copy;
            if me.read_pos >= me.read_buf.len() {
                me.read_buf.clear();
                me.read_pos = 0;
            }
            return Poll::Ready(Ok(()));
        }

        match Pin::new(&mut me.inner).poll_next(cx) {
            Poll::Ready(Some(Ok(msg))) => {
                match msg {
                    WsMessage::Binary(data) => {
                        let to_copy = data.len().min(buf.remaining());
                        buf.put_slice(&data[..to_copy]);
                        if to_copy < data.len() {
                            me.read_buf = data[to_copy..].to_vec();
                            me.read_pos = 0;
                        }
                        Poll::Ready(Ok(()))
                    }
                    WsMessage::Close(_) => Poll::Ready(Ok(())),
                    _ => {
                        cx.waker().wake_by_ref();
                        Poll::Pending
                    }
                }
            }
            Poll::Ready(Some(Err(e))) => {
                Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))
            }
            Poll::Ready(None) => Poll::Ready(Ok(())),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for WsServerStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let me = self.get_mut();
        let msg = WsMessage::Binary(buf.to_vec());

        match Pin::new(&mut me.inner).poll_ready(cx) {
            Poll::Ready(Ok(())) => {
                match Pin::new(&mut me.inner).start_send(msg) {
                    Ok(()) => Poll::Ready(Ok(buf.len())),
                    Err(e) => Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::Other, e.to_string()
                    ))),
                }
            }
            Poll::Ready(Err(e)) => {
                Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match Pin::new(&mut self.get_mut().inner).poll_flush(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(e)) => {
                Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match Pin::new(&mut self.get_mut().inner).poll_close(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(e)) => {
                Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string())))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Connect to a WebSocket server (client side)
pub async fn connect(url: &str) -> Result<WsStream> {
    let (ws_stream, _) = connect_async(url).await
        .map_err(|e| anyhow::anyhow!("WebSocket connection failed: {}", e))?;
    Ok(WsStream::new(ws_stream))
}

/// Accept a WebSocket connection (server side)
pub async fn accept(stream: TcpStream) -> Result<WsServerStream> {
    let ws_stream = accept_async(stream).await
        .map_err(|e| anyhow::anyhow!("WebSocket accept failed: {}", e))?;
    Ok(WsServerStream::new(ws_stream))
}
