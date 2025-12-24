use crate::config::ServerConfig;
use crate::transport::create_transport_server;
use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_rustls::TlsAcceptor;
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{error, info, warn};
use yamux::{Config as YamuxConfig, Connection as YamuxConnection};

const SUPPORTED_PROTOCOL_VERSION: u8 = 1;

/// 代理配置信息（从客户端接收）
#[derive(Debug, Clone)]
struct ProxyInfo {
    name: String,
    publish_addr: String,
    publish_port: u16,
    local_port: u16,
}

/// 发送错误消息给客户端
async fn send_error_message<T>(stream: &mut T, message: &str) -> Result<()>
where
    T: AsyncWriteExt + Unpin,
{
    let msg_bytes = message.as_bytes();
    let msg_len = (msg_bytes.len() as u16).to_be_bytes();
    stream.write_all(&msg_len).await?;
    stream.write_all(msg_bytes).await?;
    stream.flush().await?;
    Ok(())
}

/// 验证代理配置的有效性
fn validate_proxy_configs(proxies: &[ProxyInfo], server_bind_port: u16) -> Result<()> {
    use std::collections::HashSet;

    if proxies.is_empty() {
        anyhow::bail!("No proxy configurations received from client");
    }

    let mut seen_names = HashSet::new();
    let mut seen_bind = HashSet::new();
    let mut seen_local_ports = HashSet::new();

    for proxy in proxies {
        // 检查 name 唯一性
        if !seen_names.insert(&proxy.name) {
            anyhow::bail!(
                "Duplicate proxy name '{}': each proxy must have a unique name",
                proxy.name
            );
        }

        // 检查 (publish_addr, publish_port) 唯一性
        if !seen_bind.insert((proxy.publish_addr.clone(), proxy.publish_port)) {
            anyhow::bail!(
                "Duplicate publish binding {}:{}: each proxy must use a different server bind address/port",
                proxy.publish_addr,
                proxy.publish_port
            );
        }

        // 检查 local_port 唯一性
        if !seen_local_ports.insert(proxy.local_port) {
            anyhow::bail!(
                "Duplicate local_port {}: each proxy must connect to a different client port",
                proxy.local_port
            );
        }

        // 检查 publish_port 是否与服务器监听端口冲突
        if proxy.publish_port == server_bind_port {
            anyhow::bail!(
                "Proxy '{}' publish_port {} conflicts with server bind port",
                proxy.name,
                proxy.publish_port
            );
        }

        // 验证地址与端口有效性
        if proxy.publish_addr.trim().is_empty() {
            anyhow::bail!("Proxy '{}': publish_addr cannot be empty", proxy.name);
        }
        if proxy.publish_port == 0 {
            anyhow::bail!("Proxy '{}': publish_port cannot be 0", proxy.name);
        }
        if proxy.local_port == 0 {
            anyhow::bail!("Proxy '{}': local_port cannot be 0", proxy.name);
        }

        // 验证名称不为空
        if proxy.name.trim().is_empty() {
            anyhow::bail!("Proxy name cannot be empty");
        }
    }

    Ok(())
}

/// 运行服务器
pub async fn run_server(config: ServerConfig, tls_acceptor: TlsAcceptor) -> Result<()> {
    info!(
        "Starting TLS tunnel server on {}:{} using {} transport",
        config.bind_addr, config.bind_port, config.transport
    );

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

                        tokio::spawn(async move {
                            if let Err(e) = handle_client_transport(transport_stream, config).await {
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
        warn!("Authentication failed: key too long");
        tls_stream.write_all(&[0]).await.ok();
        send_error_message(&mut tls_stream, error_msg).await.ok();
        return Err(anyhow::anyhow!("Key too long"));
    }

    let mut key_buf = vec![0u8; key_len];
    tls_stream.read_exact(&mut key_buf).await?;
    let client_key = String::from_utf8(key_buf)?;

    if client_key != config.auth_key {
        let error_msg = "Invalid authentication key";
        warn!("Authentication failed: invalid key");
        tls_stream.write_all(&[0]).await.ok();
        send_error_message(&mut tls_stream, error_msg).await.ok();
        return Err(anyhow::anyhow!("Authentication failed"));
    }

    info!("Client authenticated successfully");
    tls_stream.write_all(&[1]).await?;
    tls_stream.flush().await?;

    let proxies = read_proxy_configs(&mut tls_stream).await?;

    // 验证代理配置
    if let Err(e) = validate_proxy_configs(&proxies, config.bind_port) {
        let error_msg = format!("Proxy configuration validation failed: {}", e);
        error!("{}", error_msg);
        tls_stream.write_all(&[0]).await.ok();
        send_error_message(&mut tls_stream, &error_msg).await.ok();
        return Err(e);
    }

    // 发送配置验证成功确认
    tls_stream.write_all(&[1]).await?;
    tls_stream.flush().await?;
    info!("Proxy configurations validated and accepted");

    // 建立 yamux 连接（使用兼容层转换tokio的AsyncRead/Write为futures的）
    let yamux_config = YamuxConfig::default();
    let tls_compat = tls_stream.compat();
    let yamux_conn = YamuxConnection::new(tls_compat, yamux_config, yamux::Mode::Server);

    info!("Yamux connection established");

    // 创建channel用于请求新的yamux streams
    let (stream_tx, stream_rx) = mpsc::channel::<(mpsc::Sender<yamux::Stream>, u16, String)>(100);

    // 创建broadcast channel用于监控yamux连接状态
    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);

    // 在后台运行yamux connection的poll循环
    let shutdown_tx_clone = shutdown_tx.clone();
    tokio::spawn(async move {
        let result = run_yamux_connection(yamux_conn, stream_rx).await;
        if let Err(e) = &result {
            info!("Client disconnected: {}", e);
        } else {
            info!("Client disconnected");
        }
        // 通知所有监听器关闭
        let _ = shutdown_tx_clone.send(());
    });

    // 使用 JoinSet 管理所有代理监听器任务
    let mut listener_tasks = tokio::task::JoinSet::new();

    // 为每个代理启动监听器
    for proxy in proxies {
        let stream_tx_clone = stream_tx.clone();
        let mut shutdown_rx = shutdown_tx.subscribe();

        listener_tasks.spawn(async move {
            tokio::select! {
                result = start_proxy_listener(proxy, stream_tx_clone) => {
                    if let Err(e) = result {
                        error!("Proxy listener error: {}", e);
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Proxy listener shutting down due to yamux disconnection");
                }
            }
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

/// 处理客户端连接（旧版本 - 保留以防万一）
#[allow(dead_code)]
async fn handle_client(
    stream: TcpStream,
    acceptor: TlsAcceptor,
    config: ServerConfig,
) -> Result<()> {
    let peer_addr = stream.peer_addr()?;

    // TLS 握手
    let mut tls_stream = match acceptor.accept(stream).await {
        Ok(s) => s,
        Err(e) => {
            error!("TLS handshake failed with {}: {:?}", peer_addr, e);
            return Err(anyhow::anyhow!("TLS handshake failed: {}", e));
        }
    };

    info!("TLS handshake completed with {}", peer_addr);

    // 认证
    let mut key_len_buf = [0u8; 4];
    tls_stream.read_exact(&mut key_len_buf).await?;
    let key_len = u32::from_be_bytes(key_len_buf) as usize;

    if key_len > 1024 {
        let error_msg = "Authentication key too long (max 1024 bytes)";
        warn!("Authentication failed from {}: key too long", peer_addr);
        tls_stream.write_all(&[0]).await.ok();
        send_error_message(&mut tls_stream, error_msg).await.ok();
        return Err(anyhow::anyhow!("Key too long"));
    }

    let mut key_buf = vec![0u8; key_len];
    tls_stream.read_exact(&mut key_buf).await?;
    let client_key = String::from_utf8(key_buf)?;

    if client_key != config.auth_key {
        let error_msg = "Invalid authentication key";
        warn!("Authentication failed from {}: invalid key", peer_addr);
        tls_stream.write_all(&[0]).await.ok();
        send_error_message(&mut tls_stream, error_msg).await.ok();
        return Err(anyhow::anyhow!("Authentication failed"));
    }

    info!("Client {} authenticated successfully", peer_addr);
    tls_stream.write_all(&[1]).await?;
    tls_stream.flush().await?;

    let proxies = read_proxy_configs(&mut tls_stream).await?;

    // 验证代理配置
    if let Err(e) = validate_proxy_configs(&proxies, config.bind_port) {
        let error_msg = format!("Proxy configuration validation failed: {}", e);
        error!("{}", error_msg);
        tls_stream.write_all(&[0]).await.ok();
        send_error_message(&mut tls_stream, &error_msg).await.ok();
        return Err(e);
    }

    // 发送配置验证成功确认
    tls_stream.write_all(&[1]).await?;
    tls_stream.flush().await?;
    info!("Proxy configurations validated and accepted");

    // 建立 yamux 连接（使用兼容层转换tokio的AsyncRead/Write为futures的）
    let yamux_config = YamuxConfig::default();
    let tls_compat = tls_stream.compat();
    let yamux_conn = YamuxConnection::new(tls_compat, yamux_config, yamux::Mode::Server);

    info!("Yamux connection established with {}", peer_addr);

    // 创建channel用于请求新的yamux streams
    let (stream_tx, stream_rx) = mpsc::channel::<(mpsc::Sender<yamux::Stream>, u16, String)>(100);

    // 创建broadcast channel用于监控yamux连接状态
    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);

    // 在后台运行yamux connection的poll循环
    let shutdown_tx_clone = shutdown_tx.clone();
    tokio::spawn(async move {
        let result = run_yamux_connection(yamux_conn, stream_rx).await;
        if let Err(e) = &result {
            info!("Client {} disconnected: {}", peer_addr, e);
        } else {
            info!("Client {} disconnected", peer_addr);
        }
        // 通知所有监听器关闭
        let _ = shutdown_tx_clone.send(());
    });

    // 使用 JoinSet 管理所有代理监听器任务
    let mut listener_tasks = tokio::task::JoinSet::new();

    // 为每个代理启动监听器
    for proxy in proxies {
        let stream_tx = stream_tx.clone();
        let mut shutdown_signal = shutdown_tx.subscribe();

        listener_tasks.spawn(async move {
            tokio::select! {
                result = start_proxy_listener(proxy.clone(), stream_tx) => {
                    if let Err(e) = result {
                        error!("Proxy '{}' listener error: {}", proxy.name, e);
                    }
                }
                _ = shutdown_signal.recv() => {
                    info!("Proxy '{}' listener shutting down due to client disconnect", proxy.name);
                }
            }
        });
    }

    // 等待关闭信号或所有监听器完成
    let mut main_shutdown = shutdown_tx.subscribe();
    tokio::select! {
        _ = main_shutdown.recv() => {
            info!("Shutting down all proxy listeners for client {}", peer_addr);
        }
        _ = async {
            while listener_tasks.join_next().await.is_some() {}
        } => {
            info!("All proxy listeners completed for client {}", peer_addr);
        }
    }

    // 中止所有剩余的监听器任务
    listener_tasks.shutdown().await;
    info!("All resources cleaned up for client {}", peer_addr);

    Ok(())
}

async fn read_proxy_configs<S>(tls_stream: &mut S) -> Result<Vec<ProxyInfo>>
where
    S: AsyncReadExt + Unpin,
{
    // 读取长度前缀的 JSON
    let mut len_buf = [0u8; 4];
    tls_stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 {
        anyhow::bail!("Proxy message length cannot be 0");
    }
    let mut buf = vec![0u8; len];
    tls_stream.read_exact(&mut buf).await?;

    #[derive(serde::Deserialize)]
    struct ProxyMessage {
        version: u8,
        proxies: Vec<crate::config::ProxyConfig>,
    }

    let msg: ProxyMessage =
        serde_json::from_slice(&buf).context("Failed to parse proxy message JSON")?;
    if msg.version != SUPPORTED_PROTOCOL_VERSION {
        anyhow::bail!("Unsupported protocol version {}", msg.version);
    }

    if msg.proxies.is_empty() {
        anyhow::bail!("No proxy configurations provided");
    }

    let mut proxies = Vec::with_capacity(msg.proxies.len());
    for p in msg.proxies {
        proxies.push(ProxyInfo {
            name: p.name,
            publish_addr: p.publish_addr,
            publish_port: p.publish_port,
            local_port: p.local_port,
        });
    }

    info!(
        "Client (json v{}) has {} proxy configurations",
        msg.version,
        proxies.len()
    );
    Ok(proxies)
}

/// 运行yamux连接的poll循环
async fn run_yamux_connection<T>(
    mut yamux_conn: YamuxConnection<T>,
    mut stream_rx: mpsc::Receiver<(mpsc::Sender<yamux::Stream>, u16, String)>,
) -> Result<()>
where
    T: futures::io::AsyncRead + futures::io::AsyncWrite + Unpin,
{
    use futures::future::poll_fn;

    loop {
        // Poll yamux连接和stream请求
        tokio::select! {
            // 处理新的stream请求
            req = stream_rx.recv() => {
                if let Some((response_tx, _remote_port, proxy_name)) = req {
                    // 创建新的outbound stream
                    let stream = poll_fn(|cx| yamux_conn.poll_new_outbound(cx)).await
                        .context("Failed to create yamux stream")?;

                    info!("Created yamux stream for proxy '{}'", proxy_name);

                    if response_tx.send(stream).await.is_err() {
                        warn!("Failed to send stream back to handler");
                    }
                } else {
                    info!("Stream request channel closed");
                    break;
                }
            }
            // Poll yamux连接以处理incoming streams（服务器端不应该收到）
            stream_result = poll_fn(|cx| yamux_conn.poll_next_inbound(cx)) => {
                match stream_result {
                    Some(Ok(stream)) => {
                        warn!("Received unexpected inbound stream from client");
                        drop(stream); // 关闭意外的流
                    }
                    Some(Err(e)) => {
                        error!("Yamux poll error: {}", e);
                        break;
                    }
                    None => {
                        info!("Yamux connection closed by client");
                        break;
                    }
                }
            }
        }
    }

    info!("Yamux connection loop ended");
    Ok(())
}

/// 启动代理监听器
async fn start_proxy_listener(
    proxy: ProxyInfo,
    stream_tx: mpsc::Sender<(mpsc::Sender<yamux::Stream>, u16, String)>,
) -> Result<()> {
    let listener = TcpListener::bind(format!("{}:{}", proxy.publish_addr, proxy.publish_port))
        .await
        .with_context(|| format!("Failed to bind port {}", proxy.publish_port))?;

    info!(
        "Proxy '{}' listening on {}:{} (forwarding to client local port {})",
        proxy.name, proxy.publish_addr, proxy.publish_port, proxy.local_port
    );

    loop {
        match listener.accept().await {
            Ok((inbound, addr)) => {
                info!("Proxy '{}' accepted connection from {}", proxy.name, addr);

                let stream_tx = stream_tx.clone();
                let proxy_name = proxy.name.clone();
                let local_port = proxy.local_port;

                tokio::spawn(async move {
                    if let Err(e) =
                        handle_proxy_connection(inbound, stream_tx, proxy_name, local_port).await
                    {
                        error!("Failed to handle connection: {}", e);
                    }
                });
            }
            Err(e) => {
                error!("Accept error: {}", e);
            }
        }
    }
}

/// 处理代理连接
async fn handle_proxy_connection(
    mut inbound: TcpStream,
    stream_tx: mpsc::Sender<(mpsc::Sender<yamux::Stream>, u16, String)>,
    proxy_name: String,
    remote_port: u16,
) -> Result<()> {
    info!("Creating yamux stream for proxy '{}'", proxy_name);

    // 请求一个新的yamux stream
    let (response_tx, mut response_rx) = mpsc::channel(1);
    stream_tx
        .send((response_tx, remote_port, proxy_name.clone()))
        .await
        .context("Failed to request yamux stream")?;

    // 等待stream
    let mut stream = response_rx
        .recv()
        .await
        .ok_or_else(|| anyhow::anyhow!("Failed to receive yamux stream"))?;

    info!("Yamux stream created for '{}'", proxy_name);

    // 发送协议头：目标端口
    use futures::io::AsyncWriteExt;
    stream.write_all(&remote_port.to_be_bytes()).await?;
    stream.flush().await?;

    info!("Sent target port {} to client", remote_port);

    // 双向转发数据（使用futures的AsyncRead/Write，需要兼容层）
    let (inbound_read, inbound_write) = inbound.split();
    let (mut stream_read, mut stream_write) = futures::io::AsyncReadExt::split(stream);

    // 转换tokio的split为futures兼容的
    let mut inbound_read = inbound_read.compat();
    let mut inbound_write = inbound_write.compat_write();

    let inbound_to_stream = async { futures::io::copy(&mut inbound_read, &mut stream_write).await };

    let stream_to_inbound = async { futures::io::copy(&mut stream_read, &mut inbound_write).await };

    tokio::select! {
        result = inbound_to_stream => {
            if let Err(e) = result {
                warn!("Error copying inbound to stream: {}", e);
            }
        }
        result = stream_to_inbound => {
            if let Err(e) = result {
                warn!("Error copying stream to inbound: {}", e);
            }
        }
    }

    info!("Connection closed for proxy '{}'", proxy_name);
    Ok(())
}
