use crate::config::{ClientFullConfig, ProxyType};
use crate::connection_pool::{ConnectionPool, PoolConfig};
use crate::transport::create_transport_client;
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{sleep, Duration};
use tokio_rustls::TlsConnector;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{error, info, warn};
use yamux::{Config as YamuxConfig, Connection as YamuxConnection};

/// 环境变量前缀
const ENV_PREFIX: &str = "TLS_TUNNEL_";

/// 重连延迟（秒）- 可通过环境变量 TLS_TUNNEL_RECONNECT_DELAY_SECS 覆盖
const RECONNECT_DELAY_SECS: u64 = 5;
/// 本地服务连接重试次数 - 可通过环境变量 TLS_TUNNEL_LOCAL_CONNECT_RETRIES 覆盖
const LOCAL_CONNECT_RETRIES: u32 = 3;
/// 本地服务连接重试延迟（毫秒）- 可通过环境变量 TLS_TUNNEL_LOCAL_RETRY_DELAY_MS 覆盖
const LOCAL_RETRY_DELAY_MS: u64 = 1000;
/// 协议版本（JSON 帧）
const PROTOCOL_VERSION: u8 = 1;

fn get_reconnect_delay() -> u64 {
    std::env::var(format!("{}RECONNECT_DELAY_SECS", ENV_PREFIX))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(RECONNECT_DELAY_SECS)
}

fn get_local_retries() -> u32 {
    std::env::var(format!("{}LOCAL_CONNECT_RETRIES", ENV_PREFIX))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(LOCAL_CONNECT_RETRIES)
}

fn get_local_retry_delay() -> u64 {
    std::env::var(format!("{}LOCAL_RETRY_DELAY_MS", ENV_PREFIX))
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(LOCAL_RETRY_DELAY_MS)
}

#[derive(Serialize)]
struct ProxyMessage<'a> {
    version: u8,
    proxies: &'a [crate::config::ProxyConfig],
}

/// 读取服务器返回的错误消息
async fn read_error_message<T>(stream: &mut T) -> Result<String>
where
    T: AsyncReadExt + Unpin,
{
    let mut msg_len_buf = [0u8; 2];
    stream.read_exact(&mut msg_len_buf).await?;
    let msg_len = u16::from_be_bytes(msg_len_buf) as usize;

    if msg_len > 4096 {
        anyhow::bail!("Error message too long");
    }

    let mut msg_buf = vec![0u8; msg_len];
    stream.read_exact(&mut msg_buf).await?;
    let message = String::from_utf8(msg_buf)?;
    Ok(message)
}

/// 运行客户端（带自动重连）
pub async fn run_client(config: ClientFullConfig, tls_connector: TlsConnector) -> Result<()> {
    loop {
        info!("Starting TLS tunnel client...");

        match run_client_session(config.clone(), tls_connector.clone()).await {
            Ok(_) => {
                info!("Client session ended normally");
            }
            Err(e) => {
                error!("Client session error: {}", e);
            }
        }

        let delay = get_reconnect_delay();
        warn!("Connection lost, reconnecting in {} seconds...", delay);
        sleep(Duration::from_secs(delay)).await;
    }
}

/// 运行单次客户端会话
async fn run_client_session(config: ClientFullConfig, tls_connector: TlsConnector) -> Result<()> {
    let client_config = &config.client;
    info!(
        "Connecting to {}:{} using {} transport",
        client_config.server_addr, client_config.server_port, client_config.transport
    );

    // 创建传输层客户端
    let transport_client = create_transport_client(client_config, tls_connector)
        .context("Failed to create transport client")?;

    info!("Using transport type: {}", transport_client.transport_type());

    // 通过传输层连接到服务器
    let transport_stream = transport_client
        .connect()
        .await
        .context("Failed to connect to server via transport")?;

    info!(
        "Connected to server via {} transport",
        transport_client.transport_type()
    );

    // 将 Pin<Box<dyn Transport>> 转换为可用的流
    let mut tls_stream = transport_stream;

    info!("Transport connection established");

    // 发送认证密钥
    let key_bytes = client_config.auth_key.as_bytes();
    let key_len = (key_bytes.len() as u32).to_be_bytes();
    tls_stream.write_all(&key_len).await?;
    tls_stream.write_all(key_bytes).await?;
    tls_stream.flush().await?;

    info!("Sent authentication key");

    // 等待认证结果
    let mut auth_result = [0u8; 1];
    tls_stream.read_exact(&mut auth_result).await?;

    if auth_result[0] != 1 {
        // 读取错误消息
        let error_msg = match read_error_message(&mut tls_stream).await {
            Ok(msg) => msg,
            Err(_) => "Unknown error".to_string(),
        };
        error!("Authentication failed: {}", error_msg);
        return Err(anyhow::anyhow!(
            "Server authentication failed: {}",
            error_msg
        ));
    }

    info!("Authentication successful");

    send_proxies_json(&config, &mut tls_stream).await?;

    // 检查服务器是否接受配置（读取一个确认字节）
    let mut config_result = [0u8; 1];
    tls_stream.read_exact(&mut config_result).await?;

    if config_result[0] != 1 {
        // 读取错误消息
        let error_msg = match read_error_message(&mut tls_stream).await {
            Ok(msg) => msg,
            Err(_) => "Unknown validation error".to_string(),
        };
        error!("Server rejected proxy configuration: {}", error_msg);
        return Err(anyhow::anyhow!(
            "Proxy configuration rejected: {}",
            error_msg
        ));
    }

    info!("Server accepted proxy configurations");

    // 创建连接池
    let defaults = PoolConfig::default();
    let pool_config = PoolConfig {
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
    };

    // 为每个代理创建独立的连接池配置
    let proxy_pools: Arc<HashMap<u16, Arc<ConnectionPool>>> = Arc::new(config
        .proxies
        .iter()
        .map(|proxy| {
            let mut pool_cfg = pool_config.clone();
            pool_cfg.reuse_connections = proxy.proxy_type.should_reuse_connections();
            
            // HTTP/2.0 需要单连接多路复用，调整池大小
            if proxy.proxy_type.is_multiplexed() {
                pool_cfg.max_size = 1;
                pool_cfg.min_idle = 1;
            }
            
            let pool = Arc::new(ConnectionPool::new(pool_cfg));
            (proxy.local_port, pool)
        })
        .collect());

    // 预热连接池
    info!("Warming up connection pools for {} proxies...", config.proxies.len());
    for proxy in &config.proxies {
        let local_addr = format!("127.0.0.1:{}", proxy.local_port);
        if let Some(pool) = proxy_pools.get(&proxy.local_port) {
            if let Err(e) = pool.warmup(&local_addr).await {
                warn!("Failed to warm up pool for proxy '{}': {}", proxy.name, e);
            }
        }
    }

    // 启动后台清理任务
    for pool in proxy_pools.values() {
        let pool_clone = pool.clone();
        pool_clone.start_cleanup_task(Duration::from_secs(30));
    }

    // 建立 yamux 连接（使用兼容层）
    let yamux_config = YamuxConfig::default();
    let tls_compat = tls_stream.compat();
    let mut yamux_conn = YamuxConnection::new(tls_compat, yamux_config, yamux::Mode::Client);

    info!("Yamux connection established");

    // 处理来自服务器的流请求
    use futures::future::poll_fn;
    loop {
        let stream_result = poll_fn(|cx| yamux_conn.poll_next_inbound(cx)).await;

        match stream_result {
            Some(Ok(stream)) => {
                info!("Received new stream from server");

                let config = config.clone();
                let proxy_pools = proxy_pools.clone();

                tokio::spawn(async move {
                    if let Err(e) = handle_stream(stream, config, proxy_pools).await {
                        error!("Stream handling error: {}", e);
                    }
                });
            }
            Some(Err(e)) => {
                error!("Yamux error: {}", e);
                break;
            }
            None => {
                info!("Yamux connection closed by server");
                break;
            }
        }
    }

    info!("Client disconnected");
    Ok(())
}

async fn send_proxies_json<S>(
    config: &ClientFullConfig,
    tls_stream: &mut S,
) -> Result<()>
where
    S: AsyncWriteExt + Unpin,
{
    let msg = ProxyMessage {
        version: PROTOCOL_VERSION,
        proxies: &config.proxies,
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
async fn handle_stream(
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
    let pool = proxy_pools.get(&target_port)
        .ok_or_else(|| anyhow::anyhow!("No connection pool found for port {}", target_port))?
        .clone();

    let local_addr = format!("127.0.0.1:{}", target_port);
    let (mut stream_read, mut stream_write) = futures::io::AsyncReadExt::split(stream);

    // 尝试一次自动重连（本地转发失败时重建本地连接并重试）
    let mut attempted_retry = false;

    loop {
        let mut local_conn = connect_local(&local_addr, &pool, proxy.proxy_type).await?;

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
                    pool.discard_connection(&local_addr, local_conn.stream).await;
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

struct LocalConn {
    stream: TcpStream,
    pooled: bool,
}

async fn connect_local(
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
                warn!(
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
                    warn!(
                        "Failed to connect to {} (attempt {}): {}, retrying...",
                        local_addr, attempt, err
                    );
                    sleep(Duration::from_millis(retry_delay)).await;
                } else {
                    error!(
                        "Failed to connect to {} after {} attempts: {}",
                        local_addr, max_retries, err
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
