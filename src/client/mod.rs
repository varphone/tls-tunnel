mod config;
mod connection;
mod forwarder;
mod stream;
mod visitor;

use crate::config::ClientFullConfig;
use crate::connection_pool::ConnectionPool;
use crate::transport::create_transport_client;
use ::yamux::{Config as YamuxConfig, Connection as YamuxConnection, Mode as YamuxMode};
use anyhow::{Context, Result};
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

        for forwarder in &config.forwarders {
            let forwarder_clone = forwarder.clone();
            let forwarder_name = forwarder.name.clone();
            let stream_tx_clone = visitor_stream_tx.clone(); // 复用同一个 channel

            tokio::spawn(async move {
                if let Err(e) =
                    forwarder::run_forwarder_listener(forwarder_clone, stream_tx_clone).await
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
