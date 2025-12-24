// HTTP/2传输实现
// 使用 HTTP/2 CONNECT 方法建立隧道

use super::{Transport, TransportClient, TransportServer, TransportType};
use anyhow::{Context, Result};
use async_trait::async_trait;
use bytes::{Buf, Bytes};
use h2::{RecvStream, SendStream};
use rustls::pki_types::ServerName;
use std::io;
use std::pin::Pin;
use std::task::{Context as TaskContext, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::{TlsAcceptor, TlsConnector};

/// 服务器端流类型枚举，用于统一处理 TLS 和 plain TCP
enum ServerStreamType {
    Tls(Box<tokio_rustls::server::TlsStream<TcpStream>>),
    Plain(TcpStream),
}

impl AsyncRead for ServerStreamType {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            ServerStreamType::Tls(s) => Pin::new(s).poll_read(cx, buf),
            ServerStreamType::Plain(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for ServerStreamType {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            ServerStreamType::Tls(s) => Pin::new(s).poll_write(cx, buf),
            ServerStreamType::Plain(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            ServerStreamType::Tls(s) => Pin::new(s).poll_flush(cx),
            ServerStreamType::Plain(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            ServerStreamType::Tls(s) => Pin::new(s).poll_shutdown(cx),
            ServerStreamType::Plain(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

/// HTTP/2 流包装器，实现 AsyncRead + AsyncWrite
pub struct Http2Stream {
    send_stream: SendStream<Bytes>,
    recv_stream: RecvStream,
    read_buf: Option<Bytes>,
}

impl Http2Stream {
    pub fn new(send_stream: SendStream<Bytes>, recv_stream: RecvStream) -> Self {
        Self {
            send_stream,
            recv_stream,
            read_buf: None,
        }
    }
}

impl AsyncRead for Http2Stream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        // 如果有缓冲的数据，先读取缓冲数据
        if let Some(data) = self.read_buf.as_mut() {
            if !data.is_empty() {
                let to_read = std::cmp::min(data.len(), buf.remaining());
                buf.put_slice(&data[..to_read]);
                data.advance(to_read);

                if data.is_empty() {
                    self.read_buf = None;
                }
                return Poll::Ready(Ok(()));
            }
        }

        // 从 HTTP/2 流读取新数据
        match self.recv_stream.poll_data(cx) {
            Poll::Ready(Some(Ok(data))) => {
                let to_read = std::cmp::min(data.len(), buf.remaining());
                buf.put_slice(&data[..to_read]);

                // 如果还有剩余数据，保存到缓冲区
                if to_read < data.len() {
                    let mut remaining = data;
                    remaining.advance(to_read);
                    self.read_buf = Some(remaining);
                }

                // 释放流量控制窗口
                let _ = self.recv_stream.flow_control().release_capacity(to_read);

                Poll::Ready(Ok(()))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Err(io::Error::other(e))),
            Poll::Ready(None) => {
                // 流结束
                Poll::Ready(Ok(()))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for Http2Stream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        // 检查发送流是否有足够的容量
        self.send_stream.reserve_capacity(buf.len());

        match self.send_stream.poll_capacity(cx) {
            Poll::Ready(Some(Ok(available))) => {
                let to_write = std::cmp::min(available, buf.len());
                let data = Bytes::copy_from_slice(&buf[..to_write]);

                self.send_stream
                    .send_data(data, false)
                    .map_err(io::Error::other)?;

                Poll::Ready(Ok(to_write))
            }
            Poll::Ready(Some(Err(e))) => Poll::Ready(Err(io::Error::other(e))),
            Poll::Ready(None) => {
                // 流已关闭
                Poll::Ready(Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "stream closed",
                )))
            }
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        // HTTP/2 没有显式的 flush 操作，数据会自动发送
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, _cx: &mut TaskContext<'_>) -> Poll<io::Result<()>> {
        // 发送 END_STREAM 标志
        self.send_stream
            .send_data(Bytes::new(), true)
            .map_err(io::Error::other)?;
        Poll::Ready(Ok(()))
    }
}

pub struct Http2TransportClient {
    server_addr: String,
    server_port: u16,
    #[allow(dead_code)]
    server_path: String,
    connector: TlsConnector,
}

impl Http2TransportClient {
    pub fn new(
        server_addr: String,
        server_port: u16,
        server_path: String,
        connector: TlsConnector,
    ) -> Self {
        Self {
            server_addr,
            server_port,
            server_path,
            connector,
        }
    }
}

#[async_trait]
impl TransportClient for Http2TransportClient {
    async fn connect(&self) -> Result<Pin<Box<dyn Transport>>> {
        // 1. 建立 TCP + TLS 连接
        tracing::debug!(
            "HTTP/2 client: Connecting to {}:{}",
            self.server_addr,
            self.server_port
        );
        let tcp = TcpStream::connect((&self.server_addr as &str, self.server_port))
            .await
            .context("Failed to connect to server")?;
        tracing::debug!("HTTP/2 client: TCP connected");

        let domain = ServerName::try_from(self.server_addr.clone())
            .map_err(|_| anyhow::anyhow!("Invalid DNS name"))?
            .to_owned();

        let tls_stream = self
            .connector
            .connect(domain, tcp)
            .await
            .context("TLS handshake failed")?;

        // 检查 ALPN 协商结果
        let (_, tls_conn) = tls_stream.get_ref();
        tracing::debug!(
            "HTTP/2 client: TLS handshake completed, ALPN: {:?}",
            tls_conn.alpn_protocol()
        );

        // 2. Establish HTTP/2 connection
        let (send_request, connection) = h2::client::handshake(tls_stream)
            .await
            .context("HTTP/2 handshake failed")?;
        tracing::debug!("HTTP/2 client: HTTP/2 handshake completed");

        // Run HTTP/2 connection driver in background
        // This is required because the connection needs to be continuously polled to process frames
        tokio::spawn(async move {
            tracing::debug!("HTTP/2 client: Connection driver started");
            if let Err(e) = connection.await {
                tracing::error!("HTTP/2 connection error: {:?}", e);
            } else {
                tracing::debug!("HTTP/2 client: Connection closed normally");
            }
        });

        // Note: The send_request is already ready after handshake completes
        // No need for additional delay as the driver runs concurrently

        // 3. Send CONNECT request to establish tunnel
        // Note: CONNECT request URI should be the target address (authority), not a path
        tracing::debug!("HTTP/2 client: Sending CONNECT request");
        let request = http::Request::builder()
            .method(http::Method::CONNECT)
            .uri(&self.server_addr) // CONNECT uses authority, not path
            .version(http::Version::HTTP_2)
            .body(())
            .context("Failed to build CONNECT request")?;

        // Wait for sender to be ready
        let mut send_request = send_request
            .ready()
            .await
            .context("send_request ready() failed")?;

        // false indicates this is not the last frame, more data will be sent
        let (response_fut, send_stream) = send_request
            .send_request(request, false)
            .context("Failed to send CONNECT request")?;
        tracing::debug!("HTTP/2 client: CONNECT request sent, waiting for response");

        // Wait for server response
        let response = response_fut
            .await
            .context("Failed to receive CONNECT response")?;
        tracing::debug!(
            "HTTP/2 client: Received response with status: {}",
            response.status()
        );

        if response.status() != http::StatusCode::OK {
            anyhow::bail!("CONNECT failed with status: {}", response.status());
        }

        let recv_stream = response.into_body();

        // 4. 返回包装的 HTTP/2 流，现在可以用于双向通信
        tracing::debug!("HTTP/2 client: Connection established successfully");
        Ok(Box::pin(Http2Stream::new(send_stream, recv_stream)))
    }

    fn transport_type(&self) -> TransportType {
        TransportType::Http2
    }
}

pub struct Http2TransportServer {
    listener: TcpListener,
    acceptor: Option<TlsAcceptor>,
}

impl Http2TransportServer {
    pub async fn bind(
        bind_addr: String,
        bind_port: u16,
        acceptor: TlsAcceptor,
        behind_proxy: bool,
    ) -> Result<Self> {
        let listener = TcpListener::bind((bind_addr.as_str(), bind_port))
            .await
            .context("Failed to bind HTTP/2 server")?;

        Ok(Self {
            listener,
            acceptor: if behind_proxy { None } else { Some(acceptor) },
        })
    }
}

#[async_trait]
impl TransportServer for Http2TransportServer {
    async fn accept(&self) -> Result<Pin<Box<dyn Transport>>> {
        // 1. 接受 TCP 连接
        tracing::debug!("HTTP/2 server: Waiting for TCP connection");
        let (tcp_stream, peer_addr) = self
            .listener
            .accept()
            .await
            .context("Failed to accept TCP")?;
        tracing::debug!("HTTP/2 server: Accepted TCP connection from {}", peer_addr);

        // 2. 创建统一的流类型
        let stream = if let Some(ref acceptor) = self.acceptor {
            // 标准 TLS 模式
            tracing::debug!("HTTP/2 server: Starting TLS handshake");
            let tls_stream = acceptor
                .accept(tcp_stream)
                .await
                .context("TLS handshake failed")?;
            tracing::debug!("HTTP/2 server: TLS handshake completed");
            Box::new(ServerStreamType::Tls(Box::new(tls_stream)))
        } else {
            // 反向代理模式 - 直接使用 TCP（TLS 由前端代理处理）
            tracing::debug!("HTTP/2 server: Using plain TCP (behind proxy)");
            Box::new(ServerStreamType::Plain(tcp_stream))
        };

        // 3. HTTP/2 握手
        tracing::debug!("HTTP/2 server: Starting HTTP/2 handshake");
        let mut connection = h2::server::handshake(stream)
            .await
            .context("HTTP/2 handshake failed")?;
        tracing::debug!("HTTP/2 server: HTTP/2 handshake completed");

        // 4. 接受第一个 HTTP/2 流（应该是 CONNECT 请求）
        // 直接在主流程中 accept，因为 accept 本身会驱动 connection
        tracing::debug!("HTTP/2 server: Waiting for first HTTP/2 stream");

        let (request, mut response_stream) = match connection.accept().await {
            Some(Ok(stream)) => {
                tracing::debug!("HTTP/2 server: Received first stream");
                stream
            }
            Some(Err(e)) => {
                tracing::error!("HTTP/2 server: Failed to accept stream: {:?}", e);
                return Err(e).context("Failed to accept HTTP/2 stream");
            }
            None => {
                tracing::error!("HTTP/2 server: Connection closed before receiving request");
                anyhow::bail!("Connection closed before receiving request");
            }
        };

        // 在后台继续运行 HTTP/2 连接处理
        // 注意：对于 tls-tunnel，我们只使用第一个 stream
        tokio::spawn(async move {
            tracing::debug!("HTTP/2 server: Connection driver started");
            while let Some(result) = connection.accept().await {
                if let Err(e) = result {
                    tracing::error!("HTTP/2 server: Accept error: {:?}", e);
                    break;
                }
                tracing::debug!("HTTP/2 server: Accepted additional stream (will be ignored)");
            }
            tracing::debug!("HTTP/2 server: Connection driver stopped");
        });

        // 5. 验证是 CONNECT 请求
        tracing::debug!(
            "HTTP/2 server: Request method: {}, URI: {}",
            request.method(),
            request.uri()
        );
        if request.method() != http::Method::CONNECT {
            tracing::error!("HTTP/2 server: Expected CONNECT, got {}", request.method());
            let response = http::Response::builder()
                .status(http::StatusCode::METHOD_NOT_ALLOWED)
                .body(())
                .unwrap();
            response_stream.send_response(response, true)?;
            anyhow::bail!("Expected CONNECT method, got {}", request.method());
        }

        // 6. 发送 200 OK 响应，表示建立隧道
        tracing::debug!("HTTP/2 server: Sending 200 OK response");
        let response = http::Response::builder()
            .status(http::StatusCode::OK)
            .body(())
            .context("Failed to build response")?;

        // false 表示连接保持打开，用于后续数据传输
        let send_stream = response_stream
            .send_response(response, false)
            .context("Failed to send response")?;
        tracing::debug!("HTTP/2 server: Response sent");

        let recv_stream = request.into_body();

        // 7. 返回包装的 HTTP/2 流，用于双向通信
        tracing::debug!("HTTP/2 server: Connection established successfully");
        Ok(Box::pin(Http2Stream::new(send_stream, recv_stream)))
    }

    fn transport_type(&self) -> TransportType {
        TransportType::Http2
    }
}
