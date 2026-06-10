use anyhow::Result;
use futures::{SinkExt, StreamExt, stream::SplitSink, stream::SplitStream};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpStream;
use tokio_tungstenite::{
    connect_async, accept_async,
    tungstenite::protocol::Message as WsMessage,
    WebSocketStream,
};

/// Client-side WebSocket stream (may or may not be TLS)
pub struct WsStream {
    sink: SplitSink<WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>, WsMessage>,
    stream: SplitStream<WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>>,
    read_buf: Vec<u8>,
    read_pos: usize,
}

/// Server-side WebSocket stream (plain TCP, TLS handled at transport layer)
pub struct WsServerStream {
    sink: SplitSink<WebSocketStream<TcpStream>, WsMessage>,
    stream: SplitStream<WebSocketStream<TcpStream>>,
    read_buf: Vec<u8>,
    read_pos: usize,
}

impl WsStream {
    pub fn new(ws: WebSocketStream<tokio_tungstenite::MaybeTlsStream<TcpStream>>) -> Self {
        let (sink, stream) = ws.split();
        Self { sink, stream, read_buf: Vec::new(), read_pos: 0 }
    }
}

impl WsServerStream {
    pub fn new(ws: WebSocketStream<TcpStream>) -> Self {
        let (sink, stream) = ws.split();
        Self { sink, stream, read_buf: Vec::new(), read_pos: 0 }
    }
}

// For now, implement simple polling wrappers
// Note: A full production impl would use a proper adapter layer
impl AsyncRead for WsStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let me = self.get_mut();
        if me.read_pos < me.read_buf.len() {
            let n = (me.read_buf.len() - me.read_pos).min(buf.remaining());
            buf.put_slice(&me.read_buf[me.read_pos..me.read_pos + n]);
            me.read_pos += n;
            if me.read_pos >= me.read_buf.len() { me.read_buf.clear(); me.read_pos = 0; }
            return Poll::Ready(Ok(()));
        }
        match Pin::new(&mut me.stream).poll_next(cx) {
            Poll::Ready(Some(Ok(WsMessage::Binary(data)))) => {
                let n = data.len().min(buf.remaining());
                buf.put_slice(&data[..n]);
                if n < data.len() { me.read_buf = data[n..].to_vec(); me.read_pos = 0; }
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Some(Ok(WsMessage::Close(_)))) | Poll::Ready(None) => Poll::Ready(Ok(())),
            Poll::Ready(Some(Ok(_))) => { cx.waker().wake_by_ref(); Poll::Pending }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for WsStream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        let me = self.get_mut();
        match Pin::new(&mut me.sink).poll_ready(cx) {
            Poll::Ready(Ok(())) => {
                let msg = WsMessage::Binary(buf.to_vec());
                match Pin::new(&mut me.sink).start_send(msg) {
                    Ok(()) => Poll::Ready(Ok(buf.len())),
                    Err(e) => Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))),
                }
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))),
            Poll::Pending => Poll::Pending,
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match Pin::new(&mut self.get_mut().sink).poll_flush(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(e)) => Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))),
            Poll::Pending => Poll::Pending,
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match Pin::new(&mut self.get_mut().sink).poll_close(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(e)) => Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncRead for WsServerStream {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let me = self.get_mut();
        if me.read_pos < me.read_buf.len() {
            let n = (me.read_buf.len() - me.read_pos).min(buf.remaining());
            buf.put_slice(&me.read_buf[me.read_pos..me.read_pos + n]);
            me.read_pos += n;
            if me.read_pos >= me.read_buf.len() { me.read_buf.clear(); me.read_pos = 0; }
            return Poll::Ready(Ok(()));
        }
        match Pin::new(&mut me.stream).poll_next(cx) {
            Poll::Ready(Some(Ok(WsMessage::Binary(data)))) => {
                let n = data.len().min(buf.remaining());
                buf.put_slice(&data[..n]);
                if n < data.len() { me.read_buf = data[n..].to_vec(); me.read_pos = 0; }
                Poll::Ready(Ok(()))
            }
            Poll::Ready(Some(Ok(WsMessage::Close(_)))) | Poll::Ready(None) => Poll::Ready(Ok(())),
            Poll::Ready(Some(Ok(_))) => { cx.waker().wake_by_ref(); Poll::Pending }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for WsServerStream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        let me = self.get_mut();
        match Pin::new(&mut me.sink).poll_ready(cx) {
            Poll::Ready(Ok(())) => {
                match Pin::new(&mut me.sink).start_send(WsMessage::Binary(buf.to_vec())) {
                    Ok(()) => Poll::Ready(Ok(buf.len())),
                    Err(e) => Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))),
                }
            }
            Poll::Ready(Err(e)) => Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))),
            Poll::Pending => Poll::Pending,
        }
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match Pin::new(&mut self.get_mut().sink).poll_flush(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(e)) => Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))),
            Poll::Pending => Poll::Pending,
        }
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        match Pin::new(&mut self.get_mut().sink).poll_close(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(e)) => Poll::Ready(Err(std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))),
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Connect as WebSocket client
pub async fn connect(url: &str) -> Result<WsStream> {
    let (ws, _) = connect_async(url).await
        .map_err(|e| anyhow::anyhow!("WebSocket connect failed: {}", e))?;
    Ok(WsStream::new(ws))
}

/// Accept WebSocket connection on server
pub async fn accept(stream: TcpStream) -> Result<WsServerStream> {
    let ws = accept_async(stream).await
        .map_err(|e| anyhow::anyhow!("WebSocket accept failed: {}", e))?;
    Ok(WsServerStream::new(ws))
}
