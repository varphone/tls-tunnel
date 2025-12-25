use crate::config::{ProxyType, VisitorConfig};
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{sleep, Duration};
use tokio_util::compat::FuturesAsyncReadCompatExt;
use tracing::{error, info, warn};

use super::config::read_error_message;
use super::ProxyHandler;

/// 运行 visitor 监听器
/// 在客户端本地监听端口，接受连接后通过 yamux 连接到服务器
pub async fn run_visitor_listener(
    visitor: VisitorConfig,
    stream_tx: tokio::sync::mpsc::Sender<tokio::sync::oneshot::Sender<Result<yamux::Stream>>>,
) -> Result<()> {
    let bind_addr = format!("{}:{}", visitor.bind_addr, visitor.bind_port);

    info!(
        "Visitor '{}': Binding to {} -> proxy name '{}' port {}",
        visitor.name, visitor.bind_addr, visitor.name, visitor.publish_port
    );

    let listener = TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("Failed to bind visitor to {}", bind_addr))?;

    info!("Visitor '{}': Listening on {}", visitor.name, bind_addr);

    loop {
        match listener.accept().await {
            Ok((local_stream, peer_addr)) => {
                info!(
                    "Visitor '{}': Accepted connection from {}",
                    visitor.name, peer_addr
                );

                let visitor_clone = visitor.clone();
                let stream_tx_clone = stream_tx.clone();

                tokio::spawn(async move {
                    if let Err(e) =
                        handle_visitor_connection(local_stream, &visitor_clone, stream_tx_clone)
                            .await
                    {
                        error!(
                            "Visitor '{}' connection handling error: {}",
                            visitor_clone.name, e
                        );
                    }
                });
            }
            Err(e) => {
                error!("Visitor '{}': Accept error: {}", visitor.name, e);
                sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

/// 处理 visitor 连接
/// 创建 yamux stream 到服务器，发送目标 proxy 名称，然后双向转发数据
pub async fn handle_visitor_connection(
    mut local_stream: tokio::net::TcpStream,
    visitor: &VisitorConfig,
    stream_tx: tokio::sync::mpsc::Sender<tokio::sync::oneshot::Sender<Result<yamux::Stream>>>,
) -> Result<()> {
    // 为需要低延迟的代理类型（如 SSH）启用 TCP_NODELAY
    if visitor.proxy_type.needs_nodelay() {
        if let Err(e) = local_stream.set_nodelay(true) {
            tracing::warn!(
                "Visitor '{}': Failed to set TCP_NODELAY: {}",
                visitor.name,
                e
            );
        } else {
            tracing::debug!(
                "Visitor '{}': Enabled TCP_NODELAY (proxy type: {:?})",
                visitor.name,
                visitor.proxy_type
            );
        }
    }

    // 请求创建新的 yamux stream
    let (response_tx, response_rx) = tokio::sync::oneshot::channel();
    stream_tx
        .send(response_tx)
        .await
        .context("Failed to request yamux stream")?;

    // 等待 yamux stream 创建完成
    let server_stream = response_rx
        .await
        .context("Failed to receive yamux stream")??;

    info!(
        "Visitor '{}': Opened stream to server for proxy '{}' port {}",
        visitor.name, visitor.name, visitor.publish_port
    );

    // 将 yamux stream 转换为兼容的 tokio stream
    let mut server_stream_tokio = server_stream.compat();

    // 发送目标 proxy 名称长度、名称和 publish_port
    let name_bytes = visitor.name.as_bytes();
    let name_len = (name_bytes.len() as u16).to_be_bytes();
    server_stream_tokio.write_all(&name_len).await?;
    server_stream_tokio.write_all(name_bytes).await?;

    let port_bytes = visitor.publish_port.to_be_bytes();
    server_stream_tokio.write_all(&port_bytes).await?;
    server_stream_tokio.flush().await?;

    info!(
        "Visitor '{}': Sent target proxy name '{}' port {}",
        visitor.name, visitor.name, visitor.publish_port
    );

    // 等待服务器确认（1 字节：1=成功，0=失败）
    let mut confirm = [0u8; 1];
    server_stream_tokio.read_exact(&mut confirm).await?;

    if confirm[0] != 1 {
        // 读取错误消息
        let error_msg = match read_error_message(&mut server_stream_tokio).await {
            Ok(msg) => msg,
            Err(_) => "Unknown error".to_string(),
        };
        error!(
            "Visitor '{}': Server rejected connection: {}",
            visitor.name, error_msg
        );
        return Err(anyhow::anyhow!(
            "Server rejected visitor connection: {}",
            error_msg
        ));
    }

    info!(
        "Visitor '{}': Server accepted connection, starting data transfer",
        visitor.name
    );

    // 双向转发数据
    let (mut local_read, mut local_write) = local_stream.split();
    let (mut server_read, mut server_write) = tokio::io::split(server_stream_tokio);

    let client_to_server = async {
        tokio::io::copy(&mut local_read, &mut server_write).await?;
        server_write.shutdown().await?;
        Ok::<_, std::io::Error>(())
    };

    let server_to_client = async {
        tokio::io::copy(&mut server_read, &mut local_write).await?;
        local_write.shutdown().await?;
        Ok::<_, std::io::Error>(())
    };

    tokio::select! {
        result = client_to_server => {
            if let Err(e) = result {
                warn!("Visitor '{}': Client to server copy error: {}", visitor.name, e);
            }
        }
        result = server_to_client => {
            if let Err(e) = result {
                warn!("Visitor '{}': Server to client copy error: {}", visitor.name, e);
            }
        }
    }

    info!("Visitor '{}': Connection closed", visitor.name);
    Ok(())
}

/// Visitor 处理器（实现 ProxyHandler trait）
pub struct VisitorHandler {
    config: VisitorConfig,
    stream_tx: tokio::sync::mpsc::Sender<tokio::sync::oneshot::Sender<Result<yamux::Stream>>>,
    status: Arc<tokio::sync::RwLock<crate::client::HandlerStatus>>,
    shutdown_tx: Arc<tokio::sync::RwLock<Option<tokio::sync::oneshot::Sender<()>>>>,
}

impl VisitorHandler {
    pub fn new(
        config: VisitorConfig,
        stream_tx: tokio::sync::mpsc::Sender<tokio::sync::oneshot::Sender<Result<yamux::Stream>>>,
    ) -> Self {
        Self {
            config,
            stream_tx,
            status: Arc::new(tokio::sync::RwLock::new(
                crate::client::HandlerStatus::Stopped,
            )),
            shutdown_tx: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }
}

#[async_trait]
impl ProxyHandler for VisitorHandler {
    async fn start(&self) -> Result<()> {
        {
            let mut status = self.status.write().await;
            *status = crate::client::HandlerStatus::Starting;
        }

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        {
            let mut tx = self.shutdown_tx.write().await;
            *tx = Some(shutdown_tx);
        }

        {
            let mut status = self.status.write().await;
            *status = crate::client::HandlerStatus::Running;
        }

        let config = self.config.clone();
        let stream_tx = self.stream_tx.clone();
        let status = self.status.clone();

        tokio::select! {
            result = run_visitor_listener(config, stream_tx) => {
                if let Err(e) = &result {
                    let mut s = status.write().await;
                    *s = crate::client::HandlerStatus::Failed(e.to_string());
                }
                result
            }
            _ = &mut shutdown_rx => {
                let mut s = status.write().await;
                *s = crate::client::HandlerStatus::Stopped;
                Ok(())
            }
        }
    }

    async fn stop(&self) -> Result<()> {
        {
            let mut status = self.status.write().await;
            *status = crate::client::HandlerStatus::Stopping;
        }

        let mut tx_lock = self.shutdown_tx.write().await;
        if let Some(tx) = tx_lock.take() {
            let _ = tx.send(());
        }

        {
            let mut status = self.status.write().await;
            *status = crate::client::HandlerStatus::Stopped;
        }

        Ok(())
    }

    async fn health_check(&self) -> bool {
        let status = self.status.read().await;
        matches!(*status, crate::client::HandlerStatus::Running)
    }

    fn status(&self) -> crate::client::HandlerStatus {
        match self.status.try_read() {
            Ok(s) => s.clone(),
            Err(_) => crate::client::HandlerStatus::Running,
        }
    }

    fn name(&self) -> &str {
        &self.config.name
    }

    fn proxy_type(&self) -> ProxyType {
        self.config.proxy_type
    }

    fn bind_address(&self) -> String {
        format!("{}:{}", self.config.bind_addr, self.config.bind_port)
    }
}
