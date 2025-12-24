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
                    .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
                
                Poll::Ready(Ok(to_write))
            }
            Poll::Ready(Some(Err(e))) => {
                Poll::Ready(Err(io::Error::new(io::ErrorKind::Other, e)))
            }
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
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Poll::Ready(Ok(()))
    }
}

pub struct Http2TransportClient {
    server_addr: String,
    server_port: u16,
    connector: TlsConnector,
}

impl Http2TransportClient {
    pub fn new(server_addr: String, server_port: u16, connector: TlsConnector) -> Self {
        Self {
            server_addr,
            server_port,
            connector,
        }
    }
}

#[async_trait]
impl TransportClient for Http2TransportClient {
    async fn connect(&self) -> Result<Pin<Box<dyn Transport>>> {
        // 1. 建立 TCP + TLS 连接
        let tcp = TcpStream::connect((&self.server_addr as &str, self.server_port))
            .await
            .context("Failed to connect to server")?;
        
        let domain = ServerName::try_from(self.server_addr.clone())
            .map_err(|_| anyhow::anyhow!("Invalid DNS name"))?
            .to_owned();
        
        let tls_stream = self
            .connector
            .connect(domain, tcp)
            .await
            .context("TLS handshake failed")?;

        // 2. 建立 HTTP/2 连接
        let (mut send_request, connection) = h2::client::handshake(tls_stream)
            .await
            .context("HTTP/2 handshake failed")?;

        // 在后台运行 HTTP/2 连接
        tokio::spawn(async move {
            if let Err(e) = connection.await {
                eprintln!("HTTP/2 connection error: {:?}", e);
            }
        });

        // 3. 发送 CONNECT 请求建立隧道
        let request = http::Request::builder()
            .method(http::Method::CONNECT)
            .uri("/")
            .header(http::header::HOST, self.server_addr.as_str())
            .body(())
            .context("Failed to build CONNECT request")?;

        let (response_fut, send_stream) = send_request
            .send_request(request, false)
            .context("Failed to send CONNECT request")?;

        // 等待服务器响应
        let response = response_fut.await.context("Failed to receive CONNECT response")?;
        
        if response.status() != http::StatusCode::OK {
            anyhow::bail!("CONNECT failed with status: {}", response.status());
        }

        let recv_stream = response.into_body();

        // 4. 返回包装的 HTTP/2 流
        Ok(Box::pin(Http2Stream::new(send_stream, recv_stream)))
    }
    
    fn transport_type(&self) -> TransportType {
        TransportType::Http2
    }
}

pub struct Http2TransportServer {
    listener: TcpListener,
    acceptor: TlsAcceptor,
}

impl Http2TransportServer {
    pub async fn bind(bind_addr: String, bind_port: u16, acceptor: TlsAcceptor) -> Result<Self> {
        let listener = TcpListener::bind((bind_addr.as_str(), bind_port))
            .await
            .context("Failed to bind HTTP/2 server")?;
        
        Ok(Self { listener, acceptor })
    }
}

#[async_trait]
impl TransportServer for Http2TransportServer {
    async fn accept(&self) -> Result<Pin<Box<dyn Transport>>> {
        // 1. 接受 TCP 连接
        let (tcp_stream, _) = self.listener.accept().await.context("Failed to accept TCP")?;

        // 2. TLS 握手
        let tls_stream = self
            .acceptor
            .accept(tcp_stream)
            .await
            .context("TLS handshake failed")?;

        // 3. HTTP/2 握手
        let mut connection = h2::server::handshake(tls_stream)
            .await
            .context("HTTP/2 handshake failed")?;

        // 4. 接受第一个 HTTP/2 流（应该是 CONNECT 请求）
        let (request, mut stream) = match connection.accept().await {
            Some(Ok(stream)) => stream,
            Some(Err(e)) => return Err(e).context("Failed to accept HTTP/2 stream"),
            None => anyhow::bail!("Connection closed before receiving request"),
        };

        // 在后台继续处理其他可能的流
        tokio::spawn(async move {
            while let Some(result) = connection.accept().await {
                if let Err(e) = result {
                    eprintln!("HTTP/2 accept error: {:?}", e);
                    break;
                }
            }
        });

        // 5. 验证是 CONNECT 请求
        if request.method() != http::Method::CONNECT {
            let response = http::Response::builder()
                .status(http::StatusCode::METHOD_NOT_ALLOWED)
                .body(())
                .unwrap();
            stream.send_response(response, true)?;
            anyhow::bail!("Expected CONNECT method, got {}", request.method());
        }

        // 6. 发送 200 OK 响应
        let response = http::Response::builder()
            .status(http::StatusCode::OK)
            .body(())
            .context("Failed to build response")?;
        
        let send_stream = stream
            .send_response(response, false)
            .context("Failed to send response")?;

        let recv_stream = request.into_body();

        // 7. 返回包装的 HTTP/2 流
        Ok(Box::pin(Http2Stream::new(send_stream, recv_stream)))
    }
    
    fn transport_type(&self) -> TransportType {
        TransportType::Http2
    }
}
