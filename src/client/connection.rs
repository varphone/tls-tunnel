use super::config::{get_local_retries, get_local_retry_delay, ENV_PREFIX};
use crate::config::ProxyType;
use crate::connection_pool::{ConnectionPool, PoolConfig};
use anyhow::Result;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio::time::{sleep, Duration};
use tracing::info;

pub struct LocalConn {
    pub stream: TcpStream,
    pub pooled: bool,
}

pub async fn connect_local(
    local_addr: &str,
    pool: &Arc<ConnectionPool>,
    proxy_type: ProxyType,
) -> Result<LocalConn> {
    // 如果该代理类型应该复用连接，则尝试从池中获取
    if proxy_type.should_reuse_connections() {
        match pool.get(local_addr).await {
            Ok(stream) => {
                info!("Got connection to {} from pool", local_addr);
                return Ok(LocalConn {
                    stream,
                    pooled: true,
                });
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to get connection from pool: {}, falling back to direct connection",
                    e
                );
            }
        }
    }

    // 建立新连接
    let max_retries = get_local_retries();
    let retry_delay = get_local_retry_delay();

    for attempt in 1..=max_retries {
        match TcpStream::connect(local_addr).await {
            Ok(stream) => {
                info!(
                    "Connected to local service: {} (attempt {})",
                    local_addr, attempt
                );
                return Ok(LocalConn {
                    stream,
                    pooled: false,
                });
            }
            Err(err) => {
                if attempt < max_retries {
                    tracing::warn!(
                        "Failed to connect to {} (attempt {}): {}, retrying...",
                        local_addr,
                        attempt,
                        err
                    );
                    sleep(Duration::from_millis(retry_delay)).await;
                } else {
                    tracing::error!(
                        "Failed to connect to {} after {} attempts: {}",
                        local_addr,
                        max_retries,
                        err
                    );
                    return Err(anyhow::anyhow!(
                        "Failed to connect to local service {}: {}",
                        local_addr,
                        err
                    ));
                }
            }
        }
    }

    unreachable!()
}

pub async fn get_pool_config() -> PoolConfig {
    let defaults = PoolConfig::default();
    PoolConfig {
        min_idle: std::env::var(format!("{}POOL_MIN_IDLE", ENV_PREFIX))
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(defaults.min_idle),
        max_size: std::env::var(format!("{}POOL_MAX_SIZE", ENV_PREFIX))
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(defaults.max_size),
        max_idle_time: Duration::from_secs(
            std::env::var(format!("{}POOL_MAX_IDLE_SECS", ENV_PREFIX))
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(defaults.max_idle_time.as_secs()),
        ),
        connect_timeout: Duration::from_millis(
            std::env::var(format!("{}POOL_CONNECT_TIMEOUT_MS", ENV_PREFIX))
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(defaults.connect_timeout.as_millis() as u64),
        ),
        keepalive_time: std::env::var(format!("{}POOL_KEEPALIVE_SECS", ENV_PREFIX))
            .ok()
            .and_then(|v| v.parse().ok())
            .map(Duration::from_secs)
            .or(defaults.keepalive_time),
        keepalive_interval: std::env::var(format!("{}POOL_KEEPALIVE_INTERVAL_SECS", ENV_PREFIX))
            .ok()
            .and_then(|v| v.parse().ok())
            .map(Duration::from_secs)
            .or(defaults.keepalive_interval),
        reuse_connections: true, // 默认值，会在下面根据代理类型调整
    }
}
