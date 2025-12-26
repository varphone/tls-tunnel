use crate::config::ClientFullConfig;
use crate::connection_pool::ConnectionPool;
use anyhow::Result;
use futures::io::{AsyncReadExt as FuturesAsyncReadExt, AsyncWriteExt as FuturesAsyncWriteExt};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{error, info, warn};

use super::stats::ClientStatsTracker;

use super::config::PROTOCOL_VERSION;

#[derive(Serialize)]
pub struct ClientConfigMessage<'a> {
    version: u8,
    proxies: &'a [crate::config::ProxyConfig],
    visitors: &'a [crate::config::VisitorConfig],
}

pub async fn send_client_config<S>(config: &ClientFullConfig, tls_stream: &mut S) -> Result<()>
where
    S: AsyncWriteExt + Unpin,
{
    let msg = ClientConfigMessage {
        version: PROTOCOL_VERSION,
        proxies: &config.proxies,
        visitors: &config.visitors,
    };

    let json = serde_json::to_vec(&msg)?;
    let len = json.len() as u32;
    tls_stream.write_all(&len.to_be_bytes()).await?;
    tls_stream.write_all(&json).await?;
    tls_stream.flush().await?;

    info!(
        "Sent proxy configurations (json) count={} bytes={}",
        config.proxies.len(),
        json.len()
    );
    Ok(())
}

/// 拷贝数据并记录统计
async fn copy_with_stats<R, W>(
    reader: &mut R,
    writer: &mut W,
    tracker: &Option<ClientStatsTracker>,
    is_upload: bool,
) -> std::io::Result<u64>
where
    R: FuturesAsyncReadExt + Unpin,
    W: FuturesAsyncWriteExt + Unpin,
{
    let mut total = 0u64;
    let mut buf = vec![0u8; 8192];

    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }

        writer.write_all(&buf[..n]).await?;

        // 记录统计
        total += n as u64;
        if let Some(ref t) = tracker {
            if is_upload {
                t.record_bytes_sent(n as u64);
            } else {
                t.record_bytes_received(n as u64);
            }
        }
    }

    writer.flush().await?;
    Ok(total)
}

/// 处理yamux流
pub async fn handle_stream(
    stream: yamux::Stream,
    config: ClientFullConfig,
    proxy_pools: Arc<HashMap<u16, Arc<ConnectionPool>>>,
    stats_manager: super::stats::ClientStatsManager,
) -> Result<()> {
    let mut stream = stream;

    // 从 stream 读取 publish_port
    let mut port_buf = [0u8; 2];
    stream.read_exact(&mut port_buf).await?;
    let publish_port = u16::from_be_bytes(port_buf);

    info!(
        "Stream requests connection for publish_port {}",
        publish_port
    );

    // 查找对应的代理配置（使用 publish_port 匹配）
    let proxy = config
        .proxies
        .iter()
        .find(|p| p.publish_port == publish_port)
        .ok_or_else(|| {
            anyhow::anyhow!("No proxy config found for publish_port {}", publish_port)
        })?;

    info!(
        "Found proxy '{}' (local_port: {}) for publish_port {}",
        proxy.name, proxy.local_port, publish_port
    );

    // 获取统计跟踪器
    let tracker = stats_manager.get_tracker(&proxy.name);

    // 连接开始
    if let Some(ref t) = tracker {
        t.connection_started();
    }

    // 确保在函数返回时更新统计
    struct ConnectionGuard {
        tracker: Option<super::stats::ClientStatsTracker>,
    }
    impl Drop for ConnectionGuard {
        fn drop(&mut self) {
            if let Some(ref t) = self.tracker {
                t.connection_ended();
            }
        }
    }
    let _guard = ConnectionGuard {
        tracker: tracker.clone(),
    };

    // 获取该代理对应的连接池（连接池键是 publish_port）
    let pool = proxy_pools
        .get(&publish_port)
        .ok_or_else(|| {
            anyhow::anyhow!("No connection pool found for publish_port {}", publish_port)
        })?
        .clone();

    let local_addr = format!("127.0.0.1:{}", proxy.local_port);

    let (mut stream_read, mut stream_write) = futures::io::AsyncReadExt::split(stream);

    // 尝试一次自动重连（本地转发失败时重建本地连接并重试）
    let mut attempted_retry = false;

    loop {
        let mut local_conn =
            super::connection::connect_local(&local_addr, &pool, proxy.proxy_type).await?;

        let (local_read, local_write) = local_conn.stream.split();
        let mut local_read = local_read.compat();
        let mut local_write = local_write.compat_write();

        // 使用 copy_with_stats 记录流量统计
        let local_to_stream = copy_with_stats(&mut local_read, &mut stream_write, &tracker, true);
        let stream_to_local = copy_with_stats(&mut stream_read, &mut local_write, &tracker, false);

        let result = tokio::select! {
            result = local_to_stream => result,
            result = stream_to_local => result,
        };

        match result {
            Ok(_) => {
                info!("Stream closed for proxy '{}'", proxy.name);
                if local_conn.pooled && proxy.proxy_type.should_reuse_connections() {
                    // 根据代理类型决定是否复用连接
                    pool.return_connection(&local_addr, local_conn.stream).await;
                } else {
                    pool.discard_connection(&local_addr, local_conn.stream)
                        .await;
                }
                return Ok(());
            }
            Err(e) => {
                if local_conn.pooled {
                    // 出错的连接不复用，直接丢弃
                    pool.discard_connection(&local_addr, local_conn.stream)
                        .await;
                }

                if attempted_retry {
                    error!("Stream handling error after retry: {}", e);
                    return Err(anyhow::anyhow!("Stream handling failed after retry: {}", e));
                } else {
                    attempted_retry = true;
                    warn!("Stream error: {}, reconnecting to local service once...", e);
                }
            }
        }
    }
}
