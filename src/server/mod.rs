mod config;
mod connection;
mod registry;
mod stats;
mod visitor;
mod yamux;

pub use registry::ProxyRegistry;

use crate::config::ServerConfig;
use crate::protocol::{AuthRequest, AuthResponse, ConfigStatusResponse, ConfigValidationResponse};
use crate::stats::StatsManager;
use crate::transport::create_transport_server;
use ::yamux::{Config as YamuxConfig, Connection as YamuxConnection, Mode as YamuxMode};
use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::broadcast;
use tokio::sync::{mpsc, RwLock};
use tokio_rustls::TlsAcceptor;
use tokio_util::compat::TokioAsyncReadCompatExt;
use tracing::{error, info, warn};

// 导入 rate_limiter 类型
use crate::rate_limiter::{RateLimiter, RateLimiterConfig};

use config::{read_client_configs, validate_proxy_configs};
use connection::start_proxy_listener;
use stats::start_stats_server;
use yamux::run_yamux_connection;

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

/// 处理客户端传输连接（使用传输抽象）
async fn handle_client_transport(
    transport_stream: std::pin::Pin<Box<dyn crate::transport::Transport>>,
    state: Arc<ServerState>,
) -> Result<()> {
    // 将 Pin<Box<dyn Transport>> 转换为可用的流
    let mut tls_stream = transport_stream;

    info!("Transport connection established");

    // 读取认证请求（JSON 格式，带长度前缀）
    let mut len_buf = [0u8; 4];
    tls_stream.read_exact(&mut len_buf).await?;
    let request_len = u32::from_be_bytes(len_buf) as usize;

    if request_len > 10 * 1024 {
        let error_msg = "Authentication request too large";
        let response = AuthResponse::failed(error_msg.to_string());
        let response_json = serde_json::to_vec(&response)?;
        tls_stream
            .write_all(&(response_json.len() as u32).to_be_bytes())
            .await
            .ok();
        tls_stream.write_all(&response_json).await.ok();
        return Err(anyhow::anyhow!("Request too large"));
    }

    let mut request_buf = vec![0u8; request_len];
    tls_stream.read_exact(&mut request_buf).await?;

    let auth_request: AuthRequest = serde_json::from_slice(&request_buf)
        .context("Failed to parse authentication request JSON")?;

    // 验证认证密钥
    let response = if auth_request.auth_key == state.config.auth_key {
        info!("Client authenticated successfully");
        AuthResponse::success()
    } else {
        tracing::warn!("Authentication failed: invalid key");
        AuthResponse::failed("Invalid authentication key".to_string())
    };

    // 发送认证响应（JSON 格式，带长度前缀）
    let response_json = serde_json::to_vec(&response)?;
    tls_stream
        .write_all(&(response_json.len() as u32).to_be_bytes())
        .await?;
    tls_stream.write_all(&response_json).await?;
    tls_stream.flush().await?;

    // 如果认证失败，断开连接
    if !response.success {
        return Err(anyhow::anyhow!("Authentication failed"));
    }

    let client_configs = read_client_configs(&mut tls_stream).await?;

    // 验证代理配置
    if let Err(e) = validate_proxy_configs(&client_configs.proxies, state.config.bind_port) {
        let error_msg = format!("Proxy configuration validation failed: {}", e);
        error!("{}", error_msg);
        let validation_resp = ConfigValidationResponse::invalid(error_msg);
        let resp_json = serde_json::to_vec(&validation_resp)?;
        tls_stream
            .write_all(&(resp_json.len() as u32).to_be_bytes())
            .await
            .ok();
        tls_stream.write_all(&resp_json).await.ok();
        return Err(e);
    }

    // 预检查哪些代理会被拒绝（提前告知客户端）
    let mut rejected_proxies: Vec<String> = Vec::new();
    {
        let registry = state.proxy_registry.read().await;
        for proxy in &client_configs.proxies {
            let key = (proxy.name.clone(), proxy.publish_port);
            if registry.contains_key(&key) {
                rejected_proxies.push(format!("{}:{}", proxy.name, proxy.publish_port));
            }
        }
    }

    // 如果所有代理都会被拒绝，返回错误
    if !client_configs.proxies.is_empty() && rejected_proxies.len() == client_configs.proxies.len()
    {
        let error_msg = format!(
            "All proxies are already registered by other clients: {}",
            rejected_proxies.join(", ")
        );
        error!("{}", error_msg);
        let validation_resp = ConfigValidationResponse::invalid(error_msg.clone());
        let resp_json = serde_json::to_vec(&validation_resp)?;
        tls_stream
            .write_all(&(resp_json.len() as u32).to_be_bytes())
            .await
            .ok();
        tls_stream.write_all(&resp_json).await.ok();
        return Err(anyhow::anyhow!(error_msg));
    }

    // 发送配置验证成功确认（JSON 格式）
    let validation_resp = ConfigValidationResponse::valid();
    let resp_json = serde_json::to_vec(&validation_resp)?;
    tls_stream
        .write_all(&(resp_json.len() as u32).to_be_bytes())
        .await?;
    tls_stream.write_all(&resp_json).await?;
    tls_stream.flush().await?;

    info!("Client configurations validated");

    // 发送配置状态响应给客户端（JSON 格式）
    let status_response = if rejected_proxies.is_empty() {
        ConfigStatusResponse::accepted()
    } else {
        ConfigStatusResponse::partially_rejected(rejected_proxies.clone())
    };

    let response_json = serde_json::to_vec(&status_response)?;
    let response_len = response_json.len() as u32;
    tls_stream.write_all(&response_len.to_be_bytes()).await?;
    tls_stream.write_all(&response_json).await?;
    tls_stream.flush().await?;

    if !rejected_proxies.is_empty() {
        info!(
            "Client informed of {} rejected proxies: {}",
            rejected_proxies.len(),
            rejected_proxies.join(", ")
        );
    }

    // 建立 yamux 连接（使用兼容层转换tokio的AsyncRead/Write为futures的）
    let yamux_config = YamuxConfig::default();
    let tls_compat = tls_stream.compat();
    let yamux_conn = YamuxConnection::new(tls_compat, yamux_config, YamuxMode::Server);

    info!("Yamux connection established");

    // 创建channel用于请求新的yamux streams
    let (stream_tx, stream_rx) = mpsc::channel::<(mpsc::Sender<::yamux::Stream>, u16, String)>(100);

    // 创建broadcast channel用于监控yamux连接状态
    let (shutdown_tx, _) = broadcast::channel::<()>(1);

    // 注册所有proxy到全局注册表，允许部分成功（跳过已注册的）
    let mut proxy_keys: Vec<(String, u16)> = Vec::new();
    {
        let mut registry = state.proxy_registry.write().await;
        for proxy in &client_configs.proxies {
            let key = (proxy.name.clone(), proxy.publish_port);

            // 检查是否已被其他客户端注册
            if registry.contains_key(&key) {
                warn!(
                    "Proxy '{}' with publish_port {} is already registered by another client, skipping",
                    proxy.name, proxy.publish_port
                );
            } else {
                info!(
                    "Registering proxy '{}' with publish_port {} for visitor access",
                    proxy.name, proxy.publish_port
                );
                registry.insert(
                    key.clone(),
                    registry::ProxyRegistration {
                        stream_tx: stream_tx.clone(),
                        proxy_info: proxy.clone(),
                    },
                );
                proxy_keys.push(key);
            }
        }
    }

    // 确保断开时清理注册表
    let proxy_registry_cleanup = state.proxy_registry.clone();
    let proxy_keys_cleanup = proxy_keys.clone();

    // 在后台运行yamux connection的poll循环
    let shutdown_tx_clone = shutdown_tx.clone();
    let proxy_registry_for_visitor = state.proxy_registry.clone();
    let stream_tx_clone = stream_tx.clone();
    let server_config_ref = Arc::clone(&state.config);
    tokio::spawn(async move {
        let result = run_yamux_connection(
            yamux_conn,
            stream_rx,
            proxy_registry_for_visitor,
            stream_tx_clone,
            &server_config_ref,
        )
        .await;
        if let Err(e) = &result {
            info!("Client disconnected: {}", e);
        } else {
            info!("Client disconnected");
        }

        // 清理注册表
        let mut registry = proxy_registry_cleanup.write().await;
        for key in proxy_keys_cleanup {
            info!("Unregistering proxy '{}' with port {}", key.0, key.1);
            registry.remove(&key);
        }

        // 通知所有监听器关闭
        let _ = shutdown_tx_clone.send(());
    });

    // 使用 JoinSet 管理所有代理监听器任务
    let mut listener_tasks = tokio::task::JoinSet::new();

    // 为每个代理启动监听器
    for proxy in client_configs.proxies {
        // 注册统计追踪器
        let tracker = state.stats_manager.register_proxy(
            proxy.name.clone(),
            proxy.publish_addr.clone(),
            proxy.publish_port,
            proxy.local_port,
        );

        let stream_tx_clone = stream_tx.clone();
        let mut shutdown_rx = shutdown_tx.subscribe();
        let stats_manager = state.stats_manager.clone();
        let proxy_name = proxy.name.clone();

        listener_tasks.spawn(async move {
            tokio::select! {
                result = start_proxy_listener(proxy, stream_tx_clone, tracker) => {
                    if let Err(e) = result {
                        error!("Proxy listener error: {}", e);
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Proxy listener shutting down due to yamux disconnection");
                }
            }
            // 清理统计信息
            stats_manager.unregister_proxy(&proxy_name);
        });
    }

    // 等待所有代理监听器完成
    while let Some(result) = listener_tasks.join_next().await {
        if let Err(e) = result {
            error!("Proxy listener task error: {:?}", e);
        }
    }

    info!("All proxy listeners stopped");
    Ok(())
}
