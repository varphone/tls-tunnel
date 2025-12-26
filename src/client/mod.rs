mod config;
mod connection;
mod forwarder;
mod geoip;
mod stats;
mod stream;
mod visitor;

use crate::config::{ClientFullConfig, ProxyType};
use crate::connection_pool::ConnectionPool;
use crate::transport::create_transport_client;
use ::yamux::{Config as YamuxConfig, Connection as YamuxConnection, Mode as YamuxMode};
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::future::poll_fn;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::time::{sleep, Duration};
use tokio_rustls::TlsConnector;
use tokio_util::compat::TokioAsyncReadCompatExt;
use tracing::{error, info, warn};

use config::{get_reconnect_delay, read_error_message};
use connection::get_pool_config;
use stream::{handle_stream, send_client_config};
use visitor::run_visitor_listener;

pub use forwarder::ForwarderHandler;
pub use visitor::VisitorHandler;

/// 代理处理器状态
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandlerStatus {
    /// 启动中
    Starting,
    /// 运行中
    Running,
    /// 停止中
    Stopping,
    /// 已停止
    Stopped,
    /// 失败（包含错误信息）
    Failed(String),
}

/// 统一的代理处理器 trait（支持 forwarder 和 visitor 的统一管理）
#[async_trait]
pub trait ProxyHandler: Send + Sync {
    /// 启动代理处理器（监听并处理连接）
    async fn start(&self) -> Result<()>;

    /// 优雅停止代理处理器
    async fn stop(&self) -> Result<()>;

    /// 健康检查
    async fn health_check(&self) -> bool;

    /// 获取当前状态
    fn status(&self) -> HandlerStatus;

    /// 获取代理名称
    fn name(&self) -> &str;

    /// 获取代理类型
    fn proxy_type(&self) -> ProxyType;

    /// 获取监听地址
    fn bind_address(&self) -> String;
}

/// 代理管理器（统一管理所有 ProxyHandler）
pub struct ProxyManager {
    handlers: Vec<Box<dyn ProxyHandler>>,
}

impl ProxyManager {
    /// 创建新的代理管理器
    pub fn new() -> Self {
        Self {
            handlers: Vec::new(),
        }
    }

    /// 添加代理处理器
    pub fn add_handler(&mut self, handler: Box<dyn ProxyHandler>) {
        self.handlers.push(handler);
    }

    /// 启动所有代理处理器
    pub async fn start_all(&self) -> Result<()> {
        info!("Starting {} proxy handler(s)...", self.handlers.len());

        for handler in &self.handlers {
            let name = handler.name().to_string();
            let proxy_type = handler.proxy_type();
            let bind_addr = handler.bind_address();

            info!(
                "Starting proxy handler: '{}' ({:?}) on {}",
                name, proxy_type, bind_addr
            );
        }

        Ok(())
    }

    /// 停止所有代理处理器
    pub async fn stop_all(&self) -> Result<()> {
        info!("Stopping {} proxy handler(s)...", self.handlers.len());

        for handler in &self.handlers {
            info!("Stopping proxy handler: '{}'", handler.name());
            if let Err(e) = handler.stop().await {
                error!("Failed to stop handler '{}': {}", handler.name(), e);
            }
        }

        Ok(())
    }

    /// 列出所有代理处理器信息
    pub fn list_handlers(&self) -> Vec<(&str, ProxyType, String, HandlerStatus)> {
        self.handlers
            .iter()
            .map(|h| (h.name(), h.proxy_type(), h.bind_address(), h.status()))
            .collect()
    }

    /// 获取健康状态
    pub async fn health_check(&self) -> Vec<(String, bool)> {
        let mut results = Vec::new();
        for handler in &self.handlers {
            let name = handler.name().to_string();
            let healthy = handler.health_check().await;
            results.push((name, healthy));
        }
        results
    }

    /// 获取处理器数量
    pub fn handler_count(&self) -> usize {
        self.handlers.len()
    }
}

impl Default for ProxyManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 运行客户端（带自动重连）
pub async fn run_client(config: ClientFullConfig, tls_connector: TlsConnector) -> Result<()> {
    // 创建客户端统计管理器
    let stats_manager = stats::ClientStatsManager::new();

    // 如果配置了统计端口，启动统计 HTTP 服务器
    if let Some(stats_port) = config.client.stats_port {
        let stats_addr = config
            .client
            .stats_addr
            .clone()
            .unwrap_or_else(|| "0.0.0.0".to_string());
        let manager = stats_manager.clone();

        tokio::spawn(async move {
            if let Err(e) = stats::start_client_stats_server(stats_addr, stats_port, manager).await
            {
                error!("Client stats server error: {}", e);
            }
        });
    }

    loop {
        info!("Starting TLS tunnel client...");

        match run_client_session(config.clone(), tls_connector.clone(), stats_manager.clone()).await
        {
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
async fn run_client_session(
    config: ClientFullConfig,
    tls_connector: TlsConnector,
    stats_manager: stats::ClientStatsManager,
) -> Result<()> {
    let client_config = &config.client;
    info!(
        "Connecting to {}:{} using {} transport",
        client_config.server_addr, client_config.server_port, client_config.transport
    );

    // 创建传输层客户端
    let transport_client = create_transport_client(client_config, tls_connector)
        .context("Failed to create transport client")?;

    info!(
        "Using transport type: {}",
        transport_client.transport_type()
    );

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

    send_client_config(&config, &mut tls_stream).await?;

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

    // 为每个代理创建统计跟踪器
    for proxy in &config.proxies {
        let tracker = stats::ClientStatsTracker::new(
            proxy.name.clone(),
            proxy.proxy_type,
            "127.0.0.1".to_string(), // 本地监听地址
            proxy.local_port,
            client_config.server_addr.clone(),
            proxy.publish_port,
        );
        stats_manager.add_tracker(tracker);
    }

    // 创建连接池
    let pool_config = get_pool_config().await;

    // 为每个代理创建独立的连接池配置（使用 publish_port 作为键）
    let proxy_pools: Arc<HashMap<u16, Arc<ConnectionPool>>> = Arc::new(
        config
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
                (proxy.publish_port, pool)
            })
            .collect(),
    );

    // 预热连接池
    info!(
        "Warming up connection pools for {} proxies...",
        config.proxies.len()
    );
    for proxy in &config.proxies {
        let local_addr = format!("127.0.0.1:{}", proxy.local_port);
        if let Some(pool) = proxy_pools.get(&proxy.publish_port) {
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
    let mut yamux_conn = YamuxConnection::new(tls_compat, yamux_config, YamuxMode::Client);

    info!("Yamux connection established");

    // 创建channel用于visitor请求新的yamux stream
    let (visitor_stream_tx, mut visitor_stream_rx) =
        tokio::sync::mpsc::channel::<tokio::sync::oneshot::Sender<Result<yamux::Stream>>>(100);

    // 启动 visitor 监听器
    if !config.visitors.is_empty() {
        info!("Starting {} visitor listeners...", config.visitors.len());

        for visitor in &config.visitors {
            let visitor_clone = visitor.clone();
            let visitor_name = visitor.name.clone();
            let stream_tx_clone = visitor_stream_tx.clone();

            tokio::spawn(async move {
                if let Err(e) = run_visitor_listener(visitor_clone, stream_tx_clone).await {
                    error!("Visitor '{}' listener error: {}", visitor_name, e);
                }
            });
        }
    }

    // 启动 forwarder 监听器
    if !config.forwarders.is_empty() {
        info!(
            "Starting {} forwarder listeners...",
            config.forwarders.len()
        );

        // 为每个 forwarder 创建统计跟踪器
        for forwarder in &config.forwarders {
            let tracker = stats::ClientStatsTracker::new(
                forwarder.name.clone(),
                forwarder.proxy_type,
                forwarder.bind_addr.clone(),
                forwarder.bind_port,
                client_config.server_addr.clone(),
                0, // forwarder 没有固定的 target_port
            );
            stats_manager.add_tracker(tracker);
        }

        // 为每个 forwarder 创建对应的 GeoIP 路由器
        for forwarder in &config.forwarders {
            let forwarder_clone = forwarder.clone();
            let forwarder_name = forwarder.name.clone();
            let stream_tx_clone = visitor_stream_tx.clone(); // 复用同一个 channel
            let stats_tracker = stats_manager.get_tracker(&forwarder.name);

            // 如果配置了路由策略，创建 GeoIP 路由器
            let router = if let Some(ref routing_config) = forwarder.routing {
                match geoip::GeoIpRouter::new(routing_config.clone()) {
                    Ok(router) => {
                        info!(
                            "Forwarder '{}': GeoIP routing enabled (direct_countries: {:?}, default: {:?})",
                            forwarder_name,
                            routing_config.direct_countries,
                            routing_config.default_strategy
                        );
                        Some(Arc::new(router))
                    }
                    Err(e) => {
                        warn!(
                            "Forwarder '{}': Failed to initialize GeoIP router: {}. All traffic will use proxy.",
                            forwarder_name, e
                        );
                        None
                    }
                }
            } else {
                None
            };

            tokio::spawn(async move {
                if let Err(e) = forwarder::run_forwarder_listener(
                    forwarder_clone,
                    stream_tx_clone,
                    router,
                    stats_tracker,
                )
                .await
                {
                    error!("Forwarder '{}' listener error: {}", forwarder_name, e);
                }
            });
        }
    }

    // Handle stream requests from server and visitor stream requests
    loop {
        tokio::select! {
            // 处理服务器发来的 inbound stream 请求（正常的 proxy 模式）
            stream_result = poll_fn(|cx| yamux_conn.poll_next_inbound(cx)) => {
                match stream_result {
                    Some(Ok(stream)) => {
                        info!("Received new stream from server");

                        let config = config.clone();
                        let proxy_pools = proxy_pools.clone();
                        let stats_mgr = stats_manager.clone();

                        tokio::spawn(async move {
                            if let Err(e) = handle_stream(stream, config, proxy_pools, stats_mgr).await {
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

            // 处理 visitor 请求创建新的 outbound stream
            Some(response_tx) = visitor_stream_rx.recv() => {
                let stream_result = poll_fn(|cx| yamux_conn.poll_new_outbound(cx)).await;
                let _ = response_tx.send(stream_result.map_err(|e| anyhow::anyhow!("Failed to create yamux stream: {}", e)));
            }
        }
    }

    info!("Client disconnected");
    Ok(())
}
