mod config;
pub mod connection;
mod control_channel;
mod registry;
mod stats;
mod visitor;
mod yamux;

pub use registry::ProxyRegistry;
pub use connection::ExceptionNotification;

use crate::config::ServerConfig;
use crate::stats::StatsManager;
use crate::transport::create_transport_server;
use ::yamux::{Config as YamuxConfig, Connection as YamuxConnection, Mode as YamuxMode};
use anyhow::{Context, Result};
use futures::future::poll_fn;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio_rustls::TlsAcceptor;
use tokio_util::compat::TokioAsyncReadCompatExt;
use tracing::{debug, error, info, warn};

// 导入 rate_limiter 类型
use crate::rate_limiter::{RateLimiter, RateLimiterConfig};

use connection::start_proxy_listener_with_notify;
use stats::start_stats_server;

/// 服务器依赖（用于依赖注入）
pub struct ServerDependencies {
    pub stats_manager: StatsManager,
    pub proxy_registry: ProxyRegistry,
    pub rate_limiter: Option<Arc<RateLimiter>>,
}

impl ServerDependencies {
    /// 创建默认依赖
    pub fn new() -> Self {
        Self {
            stats_manager: StatsManager::new(),
            proxy_registry: Arc::new(RwLock::new(std::collections::HashMap::new())),
            rate_limiter: None,
        }
    }
}

impl Default for ServerDependencies {
    fn default() -> Self {
        Self::new()
    }
}

/// 服务器状态管理（避免过度克隆）
#[derive(Clone)]
pub struct ServerState {
    pub config: Arc<ServerConfig>,
    pub stats_manager: StatsManager,
    pub proxy_registry: ProxyRegistry,
    pub rate_limiter: Option<Arc<RateLimiter>>,
}

impl ServerState {
    /// 从配置创建状态（使用默认依赖）
    pub fn new(config: ServerConfig) -> Self {
        // 根据配置创建速率限制器
        let rate_limiter = config.rate_limit.as_ref().map(|cfg| {
            Arc::new(RateLimiter::new(RateLimiterConfig {
                requests_per_second: cfg.requests_per_second,
                burst_size: cfg.burst_size,
            }))
        });

        let mut deps = ServerDependencies::new();
        deps.rate_limiter = rate_limiter;
        Self::with_dependencies(config, deps)
    }

    /// 从配置和依赖创建状态
    pub fn with_dependencies(config: ServerConfig, deps: ServerDependencies) -> Self {
        Self {
            config: Arc::new(config),
            stats_manager: deps.stats_manager,
            proxy_registry: deps.proxy_registry,
            rate_limiter: deps.rate_limiter,
        }
    }
}

/// 运行服务器（支持依赖注入）
pub async fn run_server(config: ServerConfig, tls_acceptor: TlsAcceptor) -> Result<()> {
    run_server_with_dependencies(config, tls_acceptor, None).await
}

/// 运行服务器（带自定义依赖，用于测试）
pub async fn run_server_with_dependencies(
    config: ServerConfig,
    tls_acceptor: TlsAcceptor,
    deps: Option<ServerDependencies>,
) -> Result<()> {
    info!(
        "Starting TLS tunnel server on {}:{} using {} transport",
        config.bind_addr, config.bind_port, config.transport
    );

    // 创建统一的状态管理（支持依赖注入）
    let state = match deps {
        Some(deps) => Arc::new(ServerState::with_dependencies(config, deps)),
        None => Arc::new(ServerState::new(config)),
    };

    // 如果配置了统计端口，启动HTTP统计服务器
    if let Some(stats_port) = state.config.stats_port {
        // 使用 stats_addr，如果未配置则回退到 bind_addr
        // validate() 已确保 bind_addr 和 stats_addr（如果存在）都不为空
        let stats_addr = state
            .config
            .stats_addr
            .as_ref()
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| state.config.bind_addr.clone());

        info!(
            "Stats server will listen on http://{}:{}",
            stats_addr, stats_port
        );

        let stats_manager = state.stats_manager.clone();
        tokio::spawn(async move {
            if let Err(e) = start_stats_server(stats_addr, stats_port, stats_manager).await {
                error!("Stats server error: {}", e);
            }
        });
    }

    // 创建传输层服务器
    let transport_server = create_transport_server(&state.config, tls_acceptor)
        .await
        .context("Failed to create transport server")?;

    info!(
        "Server listening on {}:{} (transport: {})",
        state.config.bind_addr,
        state.config.bind_port,
        transport_server.transport_type()
    );
    info!("Waiting for client connections... (Press Ctrl+C to stop)");

    // 设置 Ctrl+C 处理
    let shutdown = tokio::signal::ctrl_c();
    tokio::pin!(shutdown);

    // 接受客户端连接
    loop {
        tokio::select! {
            result = transport_server.accept() => {
                match result {
                    Ok(transport_stream) => {
                        info!("Accepted connection via {} transport", transport_server.transport_type());

                        // 应用速率限制
                        if let Some(ref limiter) = state.rate_limiter {
                            match limiter.check() {
                                Ok(_) => {
                                    // 速率限制通过，继续处理
                                }
                                Err(wait_time) => {
                                    warn!(
                                        "Rate limit exceeded, rejecting connection (retry after {:?})",
                                        wait_time
                                    );
                                    // 拒绝连接，不处理
                                    continue;
                                }
                            }
                        }

                        let state = Arc::clone(&state);

                        tokio::spawn(async move {
                            if let Err(e) = handle_client_transport(transport_stream, state).await {
                                error!("Client error: {}", e);
                            }
                        });
                    }
                    Err(e) => {
                        error!("Failed to accept connection: {}", e);
                    }
                }
            }
            _ = &mut shutdown => {
                info!("Received shutdown signal, stopping server...");
                break;
            }
        }
    }

    info!("Server stopped gracefully");
    Ok(())
}

/// 服务器会话状态
#[derive(Debug, PartialEq, Clone, Copy)]
enum SessionState {
    Authenticating,
    Authenticated,
    ConfiguringProxy,
    Running,
}

/// 服务器世界 - 统一管理所有共享资源
struct ServerWorld {
    yamux_conn: YamuxConnection<
        tokio_util::compat::Compat<std::pin::Pin<Box<dyn crate::transport::Transport>>>,
    >,

    state: Arc<ServerState>,
    session_state: SessionState,
    stream_tx: mpsc::Sender<(mpsc::Sender<::yamux::Stream>, u16, String)>,
    stream_rx: mpsc::Receiver<(mpsc::Sender<::yamux::Stream>, u16, String)>,
    shutdown_tx: broadcast::Sender<()>,
    proxy_keys: Vec<(String, u16)>,
    client_id: Option<String>,
    exception_tx: mpsc::UnboundedSender<connection::ExceptionNotification>,
    exception_rx: mpsc::UnboundedReceiver<connection::ExceptionNotification>,
}

impl ServerWorld {
    /// 清理资源
    async fn cleanup(&mut self) {
        info!("Cleaning up server resources");

        // 清理注册表
        let mut registry = self.state.proxy_registry.write().await;
        for key in &self.proxy_keys {
            info!("Unregistering proxy '{}' with port {}", key.0, key.1);
            registry.remove(key);
        }

        // 通知所有监听器关闭
        let _ = self.shutdown_tx.send(());
    }
}

/// 处理客户端传输连接（使用传输抽象）
async fn handle_client_transport(
    transport_stream: std::pin::Pin<Box<dyn crate::transport::Transport>>,
    state: Arc<crate::server::ServerState>,
) -> Result<()> {
    let tls_stream = transport_stream;

    info!("Transport connection established");

    // 建立 yamux 连接
    let yamux_config = YamuxConfig::default();
    let tls_compat = tls_stream.compat();
    let yamux_conn = YamuxConnection::new(tls_compat, yamux_config, YamuxMode::Server);

    info!("Yamux connection established");

    // 创建控制通道（在获取控制流之前）
    let (control_channel, event_rx) = control_channel::ServerControlChannel::new();

    // 创建channel用于请求新的yamux streams
    let (stream_tx, stream_rx) = mpsc::channel::<(mpsc::Sender<::yamux::Stream>, u16, String)>(100);

    // 创建broadcast channel用于监控yamux连接状态
    let (shutdown_tx, _) = broadcast::channel::<()>(1);

    // 创建异常通知通道
    let (exception_tx, exception_rx) = mpsc::unbounded_channel();

    // 创建服务器世界对象（包含 yamux_conn，用于在事件循环中 poll）
    let world = ServerWorld {
        yamux_conn,
        state,
        session_state: SessionState::Authenticating,
        stream_tx,
        stream_rx,
        shutdown_tx,
        proxy_keys: Vec::new(),
        client_id: None,
        exception_tx,
        exception_rx,
    };

    // 运行统一事件循环
    run_server_event_loop(world, control_channel, event_rx).await?;

    info!("Client disconnected");
    Ok(())
}

/// 处理代理配置提交（独立函数避免借用冲突）
async fn handle_proxy_config_submission(
    world: &mut ServerWorld,
    control_channel: &control_channel::ServerControlChannel,
    control_stream: &mut ::yamux::Stream,
    id: serde_json::Value,
    proxies: Vec<crate::config::ProxyConfig>,
    visitors: Vec<crate::config::VisitorConfig>,
) -> Result<bool> {
    use std::collections::HashSet;

    // 验证代理配置
    let mut seen_names = HashSet::new();
    let mut seen_bind = HashSet::new();

    for proxy in &proxies {
        // 检查 name 唯一性
        if !seen_names.insert(&proxy.name) {
            error!("Duplicate proxy name '{}'", proxy.name);
            control_channel
                .send_config_rejected(
                    control_stream,
                    id,
                    vec![format!("Duplicate proxy name: {}", proxy.name)],
                )
                .await?;
            return Ok(false);
        }

        // 检查 (publish_addr, publish_port) 唯一性
        if !seen_bind.insert((proxy.publish_addr.clone(), proxy.publish_port)) {
            error!(
                "Duplicate publish binding {}:{}",
                proxy.publish_addr, proxy.publish_port
            );
            control_channel
                .send_config_rejected(
                    control_stream,
                    id,
                    vec![format!(
                        "Duplicate binding: {}:{}",
                        proxy.publish_addr, proxy.publish_port
                    )],
                )
                .await?;
            return Ok(false);
        }

        // 验证端口和地址有效性
        if proxy.publish_port == 0
            || proxy.local_port == 0
            || proxy.publish_addr.trim().is_empty()
            || proxy.name.trim().is_empty()
        {
            error!("Invalid proxy configuration: {}", proxy.name);
            control_channel
                .send_config_rejected(
                    control_stream,
                    id,
                    vec![format!("Invalid proxy: {}", proxy.name)],
                )
                .await?;
            return Ok(false);
        }

        // 检查是否与服务器端口冲突
        if proxy.publish_port == world.state.config.bind_port {
            error!("Proxy '{}' port conflicts with server port", proxy.name);
            control_channel
                .send_config_rejected(
                    control_stream,
                    id,
                    vec![format!("Port conflict: {}", proxy.name)],
                )
                .await?;
            return Ok(false);
        }
    }

    // 预检查哪些代理会被拒绝
    let mut rejected_proxies: Vec<String> = Vec::new();
    {
        let registry = world.state.proxy_registry.read().await;
        for proxy in &proxies {
            let key = (proxy.name.clone(), proxy.publish_port);
            if registry.contains_key(&key) {
                rejected_proxies.push(format!("{}:{}", proxy.name, proxy.publish_port));
            }
        }
    }

    // 如果所有代理都会被拒绝
    if !proxies.is_empty() && rejected_proxies.len() == proxies.len() {
        error!("All proxies rejected: {}", rejected_proxies.join(", "));
        
        // 发送异常通知给客户端
        let _ = control_channel
            .send_exception_notification(
                control_stream,
                "error",
                format!("所有代理配置被拒绝：{}", rejected_proxies.join(", ")),
                Some("ALL_PROXIES_REJECTED".to_string()),
                Some(serde_json::json!({
                    "rejected_proxies": &rejected_proxies,
                    "reason": "端口或名称冲突"
                }))
            )
            .await;
        
        control_channel
            .send_config_rejected(control_stream, id, rejected_proxies)
            .await?;
        return Ok(false);
    }

    // 注册代理
    {
        let mut registry = world.state.proxy_registry.write().await;
        for proxy in &proxies {
            let key = (proxy.name.clone(), proxy.publish_port);

            if registry.contains_key(&key) {
                warn!(
                    "Proxy '{}' with publish_port {} is already registered, skipping",
                    proxy.name, proxy.publish_port
                );
            } else {
                info!(
                    "Registering proxy '{}' with publish_port {}",
                    proxy.name, proxy.publish_port
                );

                let proxy_info = registry::ProxyInfo {
                    name: proxy.name.clone(),
                    proxy_type: proxy.proxy_type,
                    publish_addr: proxy.publish_addr.clone(),
                    publish_port: proxy.publish_port,
                    local_port: proxy.local_port,
                };

                registry.insert(
                    key.clone(),
                    registry::ProxyRegistration {
                        stream_tx: world.stream_tx.clone(),
                        proxy_info,
                    },
                );
                world.proxy_keys.push(key);
            }
        }
    }

    // 验证 visitor 配置：检查对应的 proxy 是否存在
    let mut rejected_visitors: Vec<String> = Vec::new();
    if !visitors.is_empty() {
        let registry = world.state.proxy_registry.read().await;
        for visitor in &visitors {
            // visitor 通过 name 和 publish_port 查找对应的 proxy
            let key = (visitor.name.clone(), visitor.publish_port);
            if !registry.contains_key(&key) {
                warn!(
                    "Visitor '{}' references non-existent proxy '{}:{}', will be unavailable",
                    visitor.name, visitor.name, visitor.publish_port
                );
                rejected_visitors.push(format!("{}:{}", visitor.name, visitor.publish_port));
            } else {
                info!(
                    "Visitor '{}' validated: proxy '{}:{}' exists",
                    visitor.name, visitor.name, visitor.publish_port
                );
            }
        }
    }

    // 合并被拒绝的 proxies 和 visitors
    let mut all_rejected = rejected_proxies.clone();
    all_rejected.extend(rejected_visitors.clone());

    // 发送响应
    if all_rejected.is_empty() {
        info!("All proxies and visitors accepted");
        control_channel
            .send_config_accepted(control_stream, id)
            .await?;
    } else {
        info!(
            "Partially accepted: {} item(s) rejected",
            all_rejected.len()
        );
        
        // 发送警告通知
        let _ = control_channel
            .send_exception_notification(
                control_stream,
                "warning",
                format!("部分配置被拒绝：{} 项", all_rejected.len()),
                Some("PARTIAL_CONFIG_REJECTION".to_string()),
                Some(serde_json::json!({
                    "rejected_items": &all_rejected,
                    "rejected_proxies": &rejected_proxies,
                    "rejected_visitors": &rejected_visitors
                }))
            )
            .await;
        
        control_channel
            .send_config_partially_rejected(control_stream, id, all_rejected)
            .await?;
    }

    world.session_state = SessionState::Running;

    // 启动代理监听器
    start_proxy_listeners_for_world(world, proxies).await?;

    Ok(true)
}

/// 启动代理监听器（独立函数）
async fn start_proxy_listeners_for_world(
    world: &mut ServerWorld,
    proxies: Vec<crate::config::ProxyConfig>,
) -> Result<()> {
    for proxy in proxies {
        // 将 ProxyConfig 转换为 ProxyInfo
        let proxy_info = registry::ProxyInfo {
            name: proxy.name.clone(),
            proxy_type: proxy.proxy_type,
            publish_addr: proxy.publish_addr.clone(),
            publish_port: proxy.publish_port,
            local_port: proxy.local_port,
        };

        // 注册统计追踪器
        let tracker = world.state.stats_manager.register_proxy(
            proxy_info.name.clone(),
            proxy_info.publish_addr.clone(),
            proxy_info.publish_port,
            proxy_info.local_port,
        );

        let stream_tx_clone = world.stream_tx.clone();
        let mut shutdown_rx = world.shutdown_tx.subscribe();
        let stats_manager = world.state.stats_manager.clone();
        let proxy_name = proxy_info.name.clone();
        let exception_tx = world.exception_tx.clone();

        tokio::spawn(async move {
            tokio::select! {
                result = start_proxy_listener_with_notify(proxy_info, stream_tx_clone, tracker, Some(exception_tx)) => {
                    if let Err(e) = result {
                        error!("Proxy listener error: {}", e);
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Proxy listener shutting down due to disconnection");
                }
            }
            stats_manager.unregister_proxy(&proxy_name);
        });
    }

    Ok(())
}

/// 统一的服务器事件循环
/// 集中处理：yamux I/O、控制通道事件、stream 请求等
async fn run_server_event_loop(
    mut world: ServerWorld,
    control_channel: control_channel::ServerControlChannel,
    mut event_rx: tokio::sync::mpsc::UnboundedReceiver<control_channel::ControlEvent>,
) -> Result<()> {
    info!("Starting unified server event loop");

    // 首先等待客户端创建控制流
    info!("Waiting for control stream from client");
    let stream_result = poll_fn(|cx| world.yamux_conn.poll_next_inbound(cx)).await;
    let mut control_stream = match stream_result {
        Some(Ok(stream)) => {
            info!("Control stream established");
            stream
        }
        Some(Err(e)) => {
            error!("Failed to accept control stream: {}", e);
            return Err(anyhow::anyhow!("Failed to accept control stream: {}", e));
        }
        None => {
            error!("Yamux connection closed before control stream was established");
            return Err(anyhow::anyhow!("Connection closed"));
        }
    };

    info!("Control stream established");

    // 主事件循环
    loop {
        tokio::select! {
            // 1. 持续驱动 yamux 连接（处理 ping/pong 和 inbound streams）
            stream_result = poll_fn(|cx| world.yamux_conn.poll_next_inbound(cx)) => {
                match stream_result {
                    Some(Ok(stream)) => {
                        if world.session_state == SessionState::Running {
                            debug!("Received new inbound stream from client (visitor or forwarder)");
                            // 处理 visitor 和 forwarder 的 inbound stream
                            let proxy_registry = world.state.proxy_registry.clone();
                            let server_config = world.state.config.clone();
                            tokio::spawn(async move {
                                if let Err(e) = visitor::handle_visitor_stream(stream, proxy_registry, &server_config).await {
                                    error!("Failed to handle inbound stream: {}", e);
                                }
                            });
                        } else {
                            warn!("Received inbound stream before running state, dropping");
                            drop(stream);
                        }
                    }
                    Some(Err(e)) => {
                        error!("Yamux error: {}", e);
                        break;
                    }
                    None => {
                        info!("Yamux connection closed by client");
                        break;
                    }
                }
            }

            // 2. 处理控制流的读取
            read_result = control_channel.read_message(&mut control_stream) => {
                match read_result {
                    Ok(Some(_request)) => {
                        // 请求已被处理并触发了事件
                    }
                    Ok(None) => {
                        info!("Control stream closed by client");
                        break;
                    }
                    Err(e) => {
                        error!("Control stream read error: {}", e);
                        break;
                    }
                }
            }

            // 3. 处理控制通道事件
            event = event_rx.recv() => {
                if let Some(event) = event {
                    let continue_loop = match event {
                        control_channel::ControlEvent::AuthenticateRequest { id, auth_key } => {
                            if auth_key == world.state.config.auth_key {
                                let client_id = format!("client_{}", uuid::Uuid::new_v4());
                                info!("Client authenticated successfully: {}", client_id);

                                if let Err(e) = control_channel
                                    .send_auth_success(&mut control_stream, id, client_id.clone())
                                    .await {
                                    error!("Failed to send auth success: {}", e);
                                    false
                                } else {
                                    world.client_id = Some(client_id);
                                    world.session_state = SessionState::Authenticated;
                                    true
                                }
                            } else {
                                warn!("Authentication failed: invalid key");
                                if let Err(e) = control_channel
                                    .send_auth_failure(&mut control_stream, id, "Invalid authentication key".to_string())
                                    .await {
                                    error!("Failed to send auth failure: {}", e);
                                }
                                false
                            }
                        }

                        control_channel::ControlEvent::SubmitConfigRequest { id, proxies, visitors } => {
                            // 处理配置请求
                            if world.session_state != SessionState::Authenticated {
                                warn!("Received config before authentication");
                                false
                            } else {
                                world.session_state = SessionState::ConfiguringProxy;
                                info!("Processing proxy configuration: {} proxies, {} visitors", proxies.len(), visitors.len());

                                // 验证并注册代理配置
                                let result = handle_proxy_config_submission(&mut world, &control_channel, &mut control_stream, id, proxies, visitors).await;
                                result.unwrap_or(false)
                            }
                        }

                        control_channel::ControlEvent::Heartbeat => {
                            debug!("Received heartbeat from client");
                            true
                        }

                        control_channel::ControlEvent::ConnectionClosed => {
                            info!("Control channel closed by client");
                            false
                        }
                    };

                    if !continue_loop {
                        break;
                    }
                } else {
                    error!("Control event stream closed");
                    break;
                }
            }

            // 4. 处理 stream 请求（visitor 需要新的 yamux stream）
            Some((response_tx, port, visitor_addr)) = world.stream_rx.recv(), if world.session_state == SessionState::Running => {
                debug!("Creating new yamux stream for visitor: {}:{}", visitor_addr, port);

                match poll_fn(|cx| world.yamux_conn.poll_new_outbound(cx)).await {
                    Ok(stream) => {
                        if response_tx.send(stream).await.is_err() {
                            warn!("Failed to send yamux stream to visitor handler");
                        }
                    }
                    Err(e) => {
                        error!("Failed to create yamux stream: {}", e);
                        // visitor handler 会超时处理
                    }
                }
            }

            // 5. 处理异常通知（从代理监听器发送过来的）
            Some(exception_req) = world.exception_rx.recv() => {
                if let Err(e) = control_channel
                    .send_exception_notification(
                        &mut control_stream,
                        &exception_req.level,
                        exception_req.message,
                        exception_req.code,
                        exception_req.data,
                    )
                    .await
                {
                    warn!("Failed to send exception notification: {}", e);
                }
            }
        }
    }

    // 清理资源
    world.cleanup().await;

    info!("Server event loop ended");
    Ok(())
}
