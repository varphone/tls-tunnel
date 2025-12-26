mod config;
mod connection;
mod control_channel;
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
use tokio::time::{interval, sleep, Duration};
use tokio_rustls::TlsConnector;
use tokio_util::compat::TokioAsyncReadCompatExt;
use tracing::{debug, error, info, warn};

use config::get_reconnect_delay;
use connection::get_pool_config;
use stream::handle_stream;
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
    let tls_stream = transport_stream;

    info!("Transport connection established");

    // 建立 yamux 连接（使用兼容层）
    let yamux_config = YamuxConfig::default();
    let tls_compat = tls_stream.compat();
    let mut yamux_conn = YamuxConnection::new(tls_compat, yamux_config, YamuxMode::Client);

    info!("Yamux connection established");

    // 创建控制流（yamux 的第一个流）
    let control_stream = poll_fn(|cx| yamux_conn.poll_new_outbound(cx))
        .await
        .context("Failed to create control stream")?;

    info!("Control stream created");

    // 创建控制通道
    let (control_channel, event_rx) = control_channel::ClientControlChannel::new(config.clone());
    let config = Arc::new(config);

    // 创建channel用于visitor请求新的yamux stream
    let (visitor_stream_tx, visitor_stream_rx) =
        tokio::sync::mpsc::channel::<tokio::sync::oneshot::Sender<Result<yamux::Stream>>>(100);

    // 创建广播channel用于通知所有监听器连接已断开
    let (shutdown_tx, _shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);

    // 创建心跳定时器
    let mut heartbeat_interval = interval(Duration::from_secs(30));
    heartbeat_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // 创建客户端世界对象
    let world = ClientWorld {
        yamux_conn,
        event_rx,
        visitor_stream_rx,
        config,
        stats_manager,
        visitor_stream_tx,
        shutdown_tx,
        state: ClientState::Authenticating,
        heartbeat_interval,
        proxy_pools: None,
    };

    // 运行统一事件循环
    run_client_event_loop(world, control_stream, control_channel).await?;

    info!("Client disconnected");
    Ok(())
}

/// 客户端状态
#[derive(Debug, PartialEq, Clone, Copy)]
enum ClientState {
    Authenticating,
    Authenticated,
    ConfiguringProxy,
    Running,
}

/// 客户端世界 - 统一管理所有共享资源
struct ClientWorld {
    yamux_conn: YamuxConnection<
        tokio_util::compat::Compat<std::pin::Pin<Box<dyn crate::transport::Transport>>>,
    >,
    event_rx: tokio::sync::mpsc::UnboundedReceiver<control_channel::ControlEvent>,
    visitor_stream_rx:
        tokio::sync::mpsc::Receiver<tokio::sync::oneshot::Sender<Result<yamux::Stream>>>,
    config: Arc<ClientFullConfig>,
    stats_manager: stats::ClientStatsManager,
    visitor_stream_tx:
        tokio::sync::mpsc::Sender<tokio::sync::oneshot::Sender<Result<yamux::Stream>>>,
    shutdown_tx: tokio::sync::broadcast::Sender<()>,
    state: ClientState,
    heartbeat_interval: tokio::time::Interval,
    proxy_pools: Option<Arc<HashMap<u16, Arc<ConnectionPool>>>>,
}

impl ClientWorld {
    /// 处理控制通道事件
    async fn handle_control_event(
        &mut self,
        event: control_channel::ControlEvent,
        control_channel: &mut control_channel::ClientControlChannel,
        control_stream: &mut yamux::Stream,
    ) -> Result<bool> {
        match event {
            control_channel::ControlEvent::AuthenticationSuccess { client_id } => {
                info!("✓ Authentication successful: {}", client_id);
                self.state = ClientState::Authenticated;
                self.initialize_resources().await?;
                self.submit_config(control_channel, control_stream).await?;
                Ok(true)
            }

            control_channel::ControlEvent::AuthenticationFailed { reason } => {
                error!("✗ Authentication failed: {}", reason);
                Err(anyhow::anyhow!("Authentication failed: {}", reason))
            }

            control_channel::ControlEvent::ConfigAccepted => {
                info!("✓ Configuration accepted by server");
                self.state = ClientState::Running;
                self.start_listeners().await?;
                Ok(true)
            }

            control_channel::ControlEvent::ConfigPartiallyRejected { rejected_proxies } => {
                warn!("⚠ Some proxies rejected: {}", rejected_proxies.join(", "));
                self.state = ClientState::Running;
                self.start_listeners().await?;
                Ok(true)
            }

            control_channel::ControlEvent::ConfigRejected { rejected_proxies } => {
                error!("✗ All proxies rejected: {}", rejected_proxies.join(", "));
                Err(anyhow::anyhow!("All proxies rejected"))
            }

            control_channel::ControlEvent::ConnectionClosed => {
                warn!("Control channel closed by server");
                let _ = self.shutdown_tx.send(());
                Ok(false)
            }
        }
    }

    /// 初始化资源（连接池、统计跟踪器）
    async fn initialize_resources(&mut self) -> Result<()> {
        info!("Initializing connection pools");

        // 创建连接池
        let pool_config = get_pool_config().await;
        let pools: HashMap<u16, Arc<ConnectionPool>> = self
            .config
            .proxies
            .iter()
            .map(|proxy| {
                let mut pool_cfg = pool_config.clone();
                pool_cfg.reuse_connections = proxy.proxy_type.should_reuse_connections();
                if proxy.proxy_type.is_multiplexed() {
                    pool_cfg.max_size = 1;
                    pool_cfg.min_idle = 1;
                }
                let pool = Arc::new(ConnectionPool::new(pool_cfg));
                (proxy.publish_port, pool)
            })
            .collect();

        // 预热连接池
        for proxy in &self.config.proxies {
            let local_addr = format!("127.0.0.1:{}", proxy.local_port);
            if let Some(pool) = pools.get(&proxy.publish_port) {
                if let Err(e) = pool.warmup(&local_addr).await {
                    warn!("Failed to warm up pool for '{}': {}", proxy.name, e);
                }
            }
        }

        // 启动清理任务
        for pool in pools.values() {
            pool.clone().start_cleanup_task(Duration::from_secs(30));
        }

        self.proxy_pools = Some(Arc::new(pools));

        // 为每个代理创建统计跟踪器
        for proxy in &self.config.proxies {
            let tracker = stats::ClientStatsTracker::new(
                proxy.name.clone(),
                proxy.proxy_type,
                "127.0.0.1".to_string(),
                proxy.local_port,
                self.config.client.server_addr.clone(),
                proxy.publish_port,
            );
            self.stats_manager.add_or_update_tracker(tracker);
        }

        Ok(())
    }

    /// 提交配置
    async fn submit_config(
        &mut self,
        control_channel: &mut control_channel::ClientControlChannel,
        control_stream: &mut yamux::Stream,
    ) -> Result<()> {
        self.state = ClientState::ConfiguringProxy;
        info!("Submitting proxy configuration");
        control_channel.send_submit_config(control_stream).await
    }

    /// 启动监听器（visitor 和 forwarder）
    async fn start_listeners(&mut self) -> Result<()> {
        // 启动 visitor 监听器
        if !self.config.visitors.is_empty() {
            info!(
                "Starting {} visitor listeners...",
                self.config.visitors.len()
            );

            for visitor in &self.config.visitors {
                let visitor_clone = visitor.clone();
                let visitor_name = visitor.name.clone();
                let stream_tx_clone = self.visitor_stream_tx.clone();
                let shutdown_rx = self.shutdown_tx.subscribe();

                tokio::spawn(async move {
                    if let Err(e) =
                        run_visitor_listener(visitor_clone, stream_tx_clone, shutdown_rx).await
                    {
                        error!("Visitor '{}' listener error: {}", visitor_name, e);
                    }
                });
            }
        }

        // 启动 forwarder 监听器
        if !self.config.forwarders.is_empty() {
            info!(
                "Starting {} forwarder listeners...",
                self.config.forwarders.len()
            );

            for forwarder in &self.config.forwarders {
                let tracker = stats::ClientStatsTracker::new(
                    forwarder.name.clone(),
                    forwarder.proxy_type,
                    forwarder.bind_addr.clone(),
                    forwarder.bind_port,
                    self.config.client.server_addr.clone(),
                    0,
                );
                self.stats_manager.add_or_update_tracker(tracker);
            }

            for forwarder in &self.config.forwarders {
                let forwarder_clone = forwarder.clone();
                let forwarder_name = forwarder.name.clone();
                let stream_tx_clone = self.visitor_stream_tx.clone();
                let shutdown_rx = self.shutdown_tx.subscribe();
                let stats_tracker = self.stats_manager.get_tracker(&forwarder.name);

                let router = if let Some(ref routing_config) = forwarder.routing {
                    match geoip::GeoIpRouter::new(routing_config.clone()) {
                        Ok(router) => {
                            info!("Forwarder '{}': GeoIP routing enabled", forwarder_name);
                            Some(Arc::new(router))
                        }
                        Err(e) => {
                            warn!(
                                "Forwarder '{}': Failed to initialize GeoIP router: {}",
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
                        shutdown_rx,
                    )
                    .await
                    {
                        error!("Forwarder '{}' listener error: {}", forwarder_name, e);
                    }
                });
            }
        }

        Ok(())
    }
}

/// 统一的客户端事件循环
/// 集中处理：yamux I/O、控制通道事件、visitor 请求、心跳等
async fn run_client_event_loop(
    mut world: ClientWorld,
    mut control_stream: yamux::Stream,
    mut control_channel: control_channel::ClientControlChannel,
) -> Result<()> {
    info!("Starting unified client event loop");

    // 开始认证
    info!("Starting authentication");
    if let Err(e) = control_channel.send_authenticate(&mut control_stream).await {
        error!("Failed to send authentication: {}", e);
        return Err(e);
    }

    // 主事件循环
    loop {
        tokio::select! {
            // 1. 驱动 yamux 连接并处理 inbound streams（始终运行以处理 ping/pong）
            stream_result = poll_fn(|cx| world.yamux_conn.poll_next_inbound(cx)) => {
                match stream_result {
                    Some(Ok(stream)) if world.state == ClientState::Running => {
                        debug!("Received new stream from server");

                        if let Some(ref pools) = world.proxy_pools {
                            let config_clone = (*world.config).clone();
                            let pools_clone = pools.clone();
                            let mgr_clone = world.stats_manager.clone();

                            tokio::spawn(async move {
                                if let Err(e) = handle_stream(stream, config_clone, pools_clone, mgr_clone).await {
                                    error!("Stream handling error: {}", e);
                                }
                            });
                        }
                    }
                    Some(Ok(_stream)) => {
                        // 在非 Running 状态收到 stream，忽略
                        debug!("Ignoring inbound stream in non-running state");
                    }
                    Some(Err(e)) => {
                        error!("Yamux error: {}", e);
                        let _ = world.shutdown_tx.send(());
                        break;
                    }
                    None => {
                        info!("Yamux connection closed by server");
                        let _ = world.shutdown_tx.send(());
                        break;
                    }
                }
            }

            // 2. 处理控制流的读取
            read_result = control_channel.read_message(&mut control_stream) => {
                match read_result {
                    Ok(Some(message)) => {
                        if let Err(e) = control_channel.handle_notification(message).await {
                            error!("Failed to handle control message: {}", e);
                        }
                    }
                    Ok(None) => {
                        info!("Control stream closed by server");
                        let _ = world.shutdown_tx.send(());
                        break;
                    }
                    Err(e) => {
                        error!("Control stream read error: {}", e);
                        let _ = world.shutdown_tx.send(());
                        break;
                    }
                }
            }

            // 3. 处理控制通道事件
            event = world.event_rx.recv() => {
                if let Some(event) = event {
                    if !world.handle_control_event(event, &mut control_channel, &mut control_stream).await? {
                        break;
                    }
                } else {
                    error!("Control event stream closed");
                    let _ = world.shutdown_tx.send(());
                    break;
                }
            }

            // 4. 处理 visitor outbound stream 请求
            Some(response_tx) = world.visitor_stream_rx.recv() => {
                let stream_result = poll_fn(|cx| world.yamux_conn.poll_new_outbound(cx)).await;
                let _ = response_tx.send(
                    stream_result.map_err(|e| anyhow::anyhow!("Failed to create yamux stream: {}", e))
                );
            }

            // 5. 定时心跳（仅在 Running 状态）
            _ = world.heartbeat_interval.tick(), if world.state == ClientState::Running => {
                debug!("Sending heartbeat");
                if let Err(e) = control_channel.send_heartbeat(&mut control_stream).await {
                    warn!("Failed to send heartbeat: {}", e);
                }
            }
        }
    }

    info!("Client event loop ended");
    Ok(())
}

/// 旧版本的主循环（待删除）
#[allow(dead_code)]
async fn run_old_main_loop(
    config: Arc<ClientFullConfig>,
    _proxy_pools: Arc<HashMap<u16, Arc<ConnectionPool>>>,
) -> Result<()> {
    // 已重构到 run_client_event_loop
    warn!("run_old_main_loop is deprecated");
    Ok(())
}
