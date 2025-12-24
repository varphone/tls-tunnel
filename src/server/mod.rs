mod registry;
mod config;
mod connection;
mod yamux;
mod visitor;
mod stats;

pub use registry::ProxyRegistry;

use crate::config::ServerConfig;
use crate::stats::StatsManager;
use crate::transport::create_transport_server;
use anyhow::{Context, Result};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, RwLock};
use tokio_rustls::TlsAcceptor;
use tokio::sync::broadcast;
use tokio_util::compat::TokioAsyncReadCompatExt;
use tracing::{error, info};
use ::yamux::{Config as YamuxConfig, Connection as YamuxConnection, Mode as YamuxMode};

use connection::start_proxy_listener;
use config::{validate_proxy_configs, read_client_configs};
use yamux::run_yamux_connection;
use stats::start_stats_server;

/// 运行服务器
pub async fn run_server(config: ServerConfig, tls_acceptor: TlsAcceptor) -> Result<()> {
    info!(
        "Starting TLS tunnel server on {}:{} using {} transport",
        config.bind_addr, config.bind_port, config.transport
    );

    // 创建统计管理器
    let stats_manager = StatsManager::new();

    // 创建全局代理注册表
    let proxy_registry: ProxyRegistry = Arc::new(RwLock::new(std::collections::HashMap::new()));

    // 如果配置了统计端口，启动HTTP统计服务器
    if let Some(stats_port) = config.stats_port {
        let stats_manager_clone = stats_manager.clone();
        tokio::spawn(async move {
            if let Err(e) = start_stats_server(stats_port, stats_manager_clone).await {
                error!("Stats server error: {}", e);
            }
        });
        info!("Stats server listening on http://0.0.0.0:{}", stats_port);
    }

    // 创建传输层服务器
    let transport_server = create_transport_server(&config, tls_acceptor)
        .await
        .context("Failed to create transport server")?;

    info!(
        "Server listening on {}:{} (transport: {})",
        config.bind_addr,
        config.bind_port,
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
                        let config = config.clone();
                        let stats_manager = stats_manager.clone();
                        let proxy_registry = proxy_registry.clone();

                        tokio::spawn(async move {
                            if let Err(e) = handle_client_transport(transport_stream, config, stats_manager, proxy_registry).await {
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
    config: ServerConfig,
    stats_manager: StatsManager,
    proxy_registry: ProxyRegistry,
) -> Result<()> {
    // 将 Pin<Box<dyn Transport>> 转换为可用的流
    let mut tls_stream = transport_stream;

    info!("Transport connection established");

    // 认证
    let mut key_len_buf = [0u8; 4];
    tls_stream.read_exact(&mut key_len_buf).await?;
    let key_len = u32::from_be_bytes(key_len_buf) as usize;

    if key_len > 1024 {
        let error_msg = "Authentication key too long (max 1024 bytes)";
        tracing::warn!("Authentication failed: key too long");
        tls_stream.write_all(&[0]).await.ok();
        send_error_message(&mut tls_stream, error_msg).await.ok();
        return Err(anyhow::anyhow!("Key too long"));
    }

    let mut key_buf = vec![0u8; key_len];
    tls_stream.read_exact(&mut key_buf).await?;
    let client_key = String::from_utf8(key_buf)?;

    if client_key != config.auth_key {
        let error_msg = "Invalid authentication key";
        tracing::warn!("Authentication failed: invalid key");
        tls_stream.write_all(&[0]).await.ok();
        send_error_message(&mut tls_stream, error_msg).await.ok();
        return Err(anyhow::anyhow!("Authentication failed"));
    }

    info!("Client authenticated successfully");
    tls_stream.write_all(&[1]).await?;
    tls_stream.flush().await?;

    let client_configs = read_client_configs(&mut tls_stream).await?;

    // 验证代理配置
    if let Err(e) = validate_proxy_configs(&client_configs.proxies, config.bind_port) {
        let error_msg = format!("Proxy configuration validation failed: {}", e);
        error!("{}", error_msg);
        tls_stream.write_all(&[0]).await.ok();
        send_error_message(&mut tls_stream, &error_msg).await.ok();
        return Err(e);
    }

    // 发送配置验证成功确认
    tls_stream.write_all(&[1]).await?;
    tls_stream.flush().await?;
    info!("Client configurations validated and accepted");

    // 建立 yamux 连接（使用兼容层转换tokio的AsyncRead/Write为futures的）
    let yamux_config = YamuxConfig::default();
    let tls_compat = tls_stream.compat();
    let yamux_conn = YamuxConnection::new(tls_compat, yamux_config, YamuxMode::Server);

    info!("Yamux connection established");

    // 创建channel用于请求新的yamux streams
    let (stream_tx, stream_rx) = mpsc::channel::<(mpsc::Sender<::yamux::Stream>, u16, String)>(100);

    // 创建broadcast channel用于监控yamux连接状态
    let (shutdown_tx, _) = broadcast::channel::<()>(1);

    // 注册所有proxy到全局注册表
    let proxy_keys: Vec<(String, u16)> = client_configs
        .proxies
        .iter()
        .map(|p| (p.name.clone(), p.publish_port))
        .collect();
    {
        let mut registry = proxy_registry.write().await;
        for proxy in &client_configs.proxies {
            info!(
                "Registering proxy '{}' with publish_port {} for visitor access",
                proxy.name, proxy.publish_port
            );
            registry.insert(
                (proxy.name.clone(), proxy.publish_port),
                registry::ProxyRegistration {
                    stream_tx: stream_tx.clone(),
                    proxy_info: proxy.clone(),
                },
            );
        }
    }

    // 确保断开时清理注册表
    let proxy_registry_cleanup = proxy_registry.clone();
    let proxy_keys_cleanup = proxy_keys.clone();

    // 在后台运行yamux connection的poll循环
    let shutdown_tx_clone = shutdown_tx.clone();
    let proxy_registry_for_visitor = proxy_registry.clone();
    let stream_tx_clone = stream_tx.clone();
    tokio::spawn(async move {
        let result = run_yamux_connection(
            yamux_conn,
            stream_rx,
            proxy_registry_for_visitor,
            stream_tx_clone,
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
        let tracker = stats_manager.register_proxy(
            proxy.name.clone(),
            proxy.publish_addr.clone(),
            proxy.publish_port,
            proxy.local_port,
        );

        let stream_tx_clone = stream_tx.clone();
        let mut shutdown_rx = shutdown_tx.subscribe();
        let stats_manager_clone = stats_manager.clone();
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
            stats_manager_clone.unregister_proxy(&proxy_name);
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

/// 发送错误消息给客户端
async fn send_error_message<T>(stream: &mut T, message: &str) -> Result<()>
where
    T: tokio::io::AsyncWriteExt + Unpin,
{
    let msg_bytes = message.as_bytes();
    let msg_len = (msg_bytes.len() as u16).to_be_bytes();
    stream.write_all(&msg_len).await?;
    stream.write_all(msg_bytes).await?;
    stream.flush().await?;
    Ok(())
}
