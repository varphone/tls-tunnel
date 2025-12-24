// WebSocket 传输实现
// 使用 WebSocket Secure (WSS) 协议建立隧道

use super::{Transport, TransportClient, TransportServer, TransportType};
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use rustls::pki_types::ServerName;
use std::io;
use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector};
use tokio_tungstenite::tungstenite::protocol::Message;
use tokio_tungstenite::WebSocketStream;

/// WebSocket 流包装器，实现 AsyncRead + AsyncWrite
pub struct WssStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    ws_stream: WebSocketStream<S>,
    read_buf: Vec<u8>,
    read_pos: usize,
}

impl<S> WssStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    pub fn new(ws_stream: WebSocketStream<S>) -> Self {
        Self {
            ws_stream,
            read_buf: Vec::new(),
            read_pos: 0,
        }
    }
}

impl<S> AsyncRead for WssStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // 如果有缓冲数据，先读取缓冲数据
        if self.read_pos < self.read_buf.len() {
            let to_read = std::cmp::min(self.read_buf.len() - self.read_pos, buf.remaining());
            buf.put_slice(&self.read_buf[self.read_pos..self.read_pos + to_read]);
            self.read_pos += to_read;

            // 如果缓冲区读完了，清空
            if self.read_pos >= self.read_buf.len() {
                self.read_buf.clear();
                self.read_pos = 0;
            }

            return Poll::Ready(Ok(()));
        }

        // 从 WebSocket 读取新消息
        match self.ws_stream.poll_next_unpin(cx) {
            Poll::Ready(Some(Ok(msg))) => match msg {
                Message::Binary(data) => {
                    let to_read = std::cmp::min(data.len(), buf.remaining());
                    buf.put_slice(&data[..to_read]);

                    // 如果还有剩余数据，保存到缓冲区
                    if to_read < data.len() {
                        self.read_buf = data[to_read..].to_vec();
                        self.read_pos = 0;
                    }

                    Poll::Ready(Ok(()))
                }
                Message::Close(_) => {
                    // WebSocket 关闭
                    Poll::Ready(Ok(()))
                }
                Message::Ping(_) | Message::Pong(_) => {
                    // Ping/Pong 由库自动处理，继续读取
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
                _ => {
                    // 忽略其他消息类型（Text 等）
                    cx.waker().wake_by_ref();
                    Poll::Pending
                }
            },
            Poll::Ready(Some(Err(e))) => {
                Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e)))
            }
            Poll::Ready(None) => {
                // 流结束
                Poll::Ready(Ok(()))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S> AsyncWrite for WssStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        // 将数据作为二进制消息发送
        let msg = Message::Binary(buf.to_vec());

        match self.ws_stream.poll_ready_unpin(cx) {
            Poll::Ready(Ok(())) => {
                match self.ws_stream.start_send_unpin(msg) {
                    Ok(()) => Poll::Ready(Ok(buf.len())),
                    Err(e) => Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e))),
                }
            }
            Poll::Ready(Err(e)) => {
                Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e)))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        match self.ws_stream.poll_flush_unpin(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(e)) => Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e))),
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        match self.ws_stream.poll_close_unpin(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(Ok(())),
            Poll::Ready(Err(e)) => Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e))),
            Poll::Pending => Poll::Pending,
        }
    }
}

pub struct WssTransportClient {
    server_addr: String,
    server_port: u16,
    connector: TlsConnector,
}

impl WssTransportClient {
    pub fn new(server_addr: String, server_port: u16, connector: TlsConnector) -> Self {
        Self {
            server_addr,
            server_port,
            connector,
        }
    }
}

#[async_trait]
impl TransportClient for WssTransportClient {
    async fn connect(&self) -> Result<Pin<Box<dyn Transport>>> {
        // 1. 建立 TCP 连接
        let tcp = TcpStream::connect((&self.server_addr as &str, self.server_port))
            .await
            .context("Failed to connect to server")?;

        // 2. TLS 握手
        let domain = ServerName::try_from(self.server_addr.clone())
            .map_err(|_| anyhow::anyhow!("Invalid DNS name"))?
            .to_owned();

        let tls_stream = self
            .connector
            .connect(domain, tcp)
            .await
            .context("TLS handshake failed")?;

        // 3. WebSocket 握手
        let ws_url = format!("wss://{}/", self.server_addr);
        let (ws_stream, _response) = tokio_tungstenite::client_async(ws_url, tls_stream)
            .await
            .context("WebSocket handshake failed")?;

        // 4. 返回包装的 WebSocket 流
        Ok(Box::pin(WssStream::new(ws_stream)))
    }

    fn transport_type(&self) -> TransportType {
        TransportType::Wss
    }
}

pub struct WssTransportServer {
    listener: TcpListener,
    acceptor: TlsAcceptor,
}

impl WssTransportServer {
    pub async fn bind(bind_addr: String, bind_port: u16, acceptor: TlsAcceptor) -> Result<Self> {
        let listener = TcpListener::bind((bind_addr.as_str(), bind_port))
            .await
            .context("Failed to bind WebSocket server")?;

        Ok(Self { listener, acceptor })
    }
}

#[async_trait]
impl TransportServer for WssTransportServer {
    async fn accept(&self) -> Result<Pin<Box<dyn Transport>>> {
        // 1. 接受 TCP 连接
        let (tcp_stream, _) = self
            .listener
            .accept()
            .await
            .context("Failed to accept TCP")?;

        // 2. TLS 握手
        let tls_stream = self
            .acceptor
            .accept(tcp_stream)
            .await
            .context("TLS handshake failed")?;

        // 3. WebSocket 握手
        let ws_stream = tokio_tungstenite::accept_async(tls_stream)
            .await
            .context("WebSocket handshake failed")?;

        // 4. 返回包装的 WebSocket 流
        Ok(Box::pin(WssStream::new(ws_stream)))
    }

    fn transport_type(&self) -> TransportType {
        TransportType::Wss
    }
}
