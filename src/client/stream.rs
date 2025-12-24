use crate::config::ClientFullConfig;
use crate::connection_pool::ConnectionPool;
use anyhow::Result;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{error, info, warn};

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

/// 处理yamux流
pub async fn handle_stream(
    mut stream: yamux::Stream,
    config: ClientFullConfig,
    proxy_pools: Arc<HashMap<u16, Arc<ConnectionPool>>>,
) -> Result<()> {
    use futures::io::AsyncReadExt as FuturesAsyncReadExt;

    // 读取目标端口
    let mut port_buf = [0u8; 2];
    stream.read_exact(&mut port_buf).await?;
    let target_port = u16::from_be_bytes(port_buf);

    info!("Stream requests connection to local port {}", target_port);

    // 查找对应的代理配置
    let proxy = config
        .proxies
        .iter()
        .find(|p| p.local_port == target_port)
        .ok_or_else(|| anyhow::anyhow!("No proxy config found for port {}", target_port))?;

    info!("Found proxy '{}' for port {}", proxy.name, target_port);

    // 获取该端口对应的连接池
    let pool = proxy_pools
        .get(&target_port)
        .ok_or_else(|| anyhow::anyhow!("No connection pool found for port {}", target_port))?
        .clone();

    let local_addr = format!("127.0.0.1:{}", target_port);
    let (mut stream_read, mut stream_write) = futures::io::AsyncReadExt::split(stream);

    // 尝试一次自动重连（本地转发失败时重建本地连接并重试）
    let mut attempted_retry = false;

    loop {
        let mut local_conn =
            super::connection::connect_local(&local_addr, &pool, proxy.proxy_type).await?;

        let (local_read, local_write) = local_conn.stream.split();
        let mut local_read = local_read.compat();
        let mut local_write = local_write.compat_write();

        let local_to_stream = async { futures::io::copy(&mut local_read, &mut stream_write).await };

        let stream_to_local = async { futures::io::copy(&mut stream_read, &mut local_write).await };

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
