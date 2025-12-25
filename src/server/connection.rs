use super::registry::{ConnectionGuard, ProxyInfo};
use crate::stats::ProxyStatsTracker;
use anyhow::{Context, Result};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{error, info, warn};

/// 启动代理监听器
pub async fn start_proxy_listener(
    proxy: ProxyInfo,
    stream_tx: mpsc::Sender<(mpsc::Sender<yamux::Stream>, u16, String)>,
    tracker: ProxyStatsTracker,
) -> Result<()> {
    let listener =
        tokio::net::TcpListener::bind(format!("{}:{}", proxy.publish_addr, proxy.publish_port))
            .await
            .context("Failed to bind proxy listener")?;

    info!(
        "Proxy '{}' listening on {}:{}",
        proxy.name, proxy.publish_addr, proxy.publish_port
    );

    loop {
        match listener.accept().await {
            Ok((inbound, _peer_addr)) => {
                let proxy_name = proxy.name.clone();
                let stream_tx = stream_tx.clone();
                let tracker_clone = tracker.clone();
                let proxy_type = proxy.proxy_type;

                tokio::spawn(async move {
                    if let Err(e) = handle_proxy_connection(
                        inbound,
                        stream_tx,
                        proxy_name,
                        proxy.publish_port,
                        tracker_clone,
                        proxy_type,
                    )
                    .await
                    {
                        error!("Failed to handle connection: {}", e);
                    }
                });
            }
            Err(e) => {
                error!("Proxy '{}' accept error: {}", proxy.name, e);
            }
        }
    }
}

/// 处理代理连接
pub async fn handle_proxy_connection(
    mut inbound: TcpStream,
    stream_tx: mpsc::Sender<(mpsc::Sender<yamux::Stream>, u16, String)>,
    proxy_name: String,
    publish_port: u16,
    tracker: ProxyStatsTracker,
    proxy_type: crate::config::ProxyType,
) -> Result<()> {
    // 为需要低延迟的代理类型（如 SSH）启用 TCP_NODELAY
    if proxy_type.needs_nodelay() {
        if let Err(e) = inbound.set_nodelay(true) {
            tracing::warn!("Failed to set TCP_NODELAY for inbound connection: {}", e);
        } else {
            tracing::debug!(
                "Enabled TCP_NODELAY for proxy '{}' (type: {:?})",
                proxy_name,
                proxy_type
            );
        }
    }

    // 连接开始，增加计数
    tracker.connection_started();

    // 确保在函数结束时减少活跃连接数
    let _guard = ConnectionGuard::new(tracker.clone());

    info!("Creating yamux stream for proxy '{}'", proxy_name);

    // 请求一个新的yamux stream
    let (response_tx, mut response_rx) = mpsc::channel(1);
    stream_tx
        .send((response_tx, publish_port, proxy_name.clone()))
        .await
        .context("Failed to request yamux stream")?;

    // 等待stream
    let mut stream = response_rx
        .recv()
        .await
        .ok_or_else(|| anyhow::anyhow!("Failed to receive yamux stream"))?;

    info!("Yamux stream created for '{}'", proxy_name);

    // 发送协议头：发布端口（与 visitor 保持一致使用 publish_port）
    use futures::io::AsyncWriteExt;
    stream.write_all(&publish_port.to_be_bytes()).await?;
    stream.flush().await?;

    info!("Sent publish_port {} to client", publish_port);

    // 双向转发数据（使用futures的AsyncRead/Write，需要兼容层）
    let (inbound_read, inbound_write) = inbound.split();
    let (mut stream_read, mut stream_write) = futures::io::AsyncReadExt::split(stream);

    // 转换tokio的split为futures兼容的
    let mut inbound_read = inbound_read.compat();
    let mut inbound_write = inbound_write.compat_write();

    // 跟踪inbound到stream的字节数（外部客户端 → 服务器 → 内网客户端：服务器接收的数据）
    let tracker_clone = tracker.clone();
    let inbound_to_stream = async move {
        let result = futures::io::copy(&mut inbound_read, &mut stream_write).await;
        if let Ok(bytes) = result {
            tracker_clone.add_bytes_received(bytes);
        }
        result
    };

    // 跟踪stream到inbound的字节数（内网客户端 → 服务器 → 外部客户端：服务器发送的数据）
    let stream_to_inbound = async move {
        let result = futures::io::copy(&mut stream_read, &mut inbound_write).await;
        if let Ok(bytes) = result {
            tracker.add_bytes_sent(bytes);
        }
        result
    };

    // 使用 join! 而不是 select!，确保两个方向都完成传输
    let (result1, result2) = tokio::join!(inbound_to_stream, stream_to_inbound);
    
    if let Err(e) = result1 {
        warn!("Error copying inbound to stream: {}", e);
    }
    if let Err(e) = result2 {
        warn!("Error copying stream to inbound: {}", e);
    }

    info!("Connection closed for proxy '{}'", proxy_name);
    Ok(())
}
