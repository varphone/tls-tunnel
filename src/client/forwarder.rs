use crate::config::{ForwarderConfig, ProxyType};
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{RwLock, Semaphore};
use tokio::time::{sleep, Duration};
use tokio_util::compat::FuturesAsyncReadCompatExt;
use tracing::{debug, error, info, warn};

use super::config::read_error_message;
use super::geoip::GeoIpRouter;
use super::stats::ClientStatsTracker;
use super::ProxyHandler;

/// 每个 forwarder 的最大并发连接数（防止 DoS 攻击）
const MAX_CONCURRENT_CONNECTIONS: usize = 1000;

/// 协议解析超时时间（防止慢速攻击）
const PROTOCOL_PARSE_TIMEOUT: Duration = Duration::from_secs(30);

/// 连接空闲超时时间（防止资源泄漏）
const CONNECTION_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60); // 5分钟

/// 数据复制缓冲区大小（64KB 适合高吞吐）
const COPY_BUFFER_SIZE: usize = 65536;

/// HTTP 协议解析缓冲区大小
const HTTP_PARSE_BUFFER_SIZE: usize = 16384;

/// 快速失败配置
const FAILED_TARGET_THRESHOLD: u32 = 3; // 失败次数阈值
const FAILED_TARGET_TIMEOUT: Duration = Duration::from_secs(30 * 60); // 黑名单过期时间（30分钟）
const FAILED_TARGET_CLEANUP_INTERVAL: Duration = Duration::from_secs(60); // 清理间隔（1分钟）

/// 失败目标的信息
#[derive(Debug, Clone)]
struct FailedTarget {
    /// 失败次数
    failure_count: u32,
    /// 加入黑名单的时间戳（秒）
    blacklist_time: u64,
}

/// 快速失败管理器
#[derive(Clone)]
pub struct FailedTargetManager {
    /// 失败目标映射表：目标地址 -> 失败信息
    targets: Arc<RwLock<HashMap<String, FailedTarget>>>,
}

impl FailedTargetManager {
    /// 创建新的快速失败管理器
    pub fn new() -> Self {
        Self {
            targets: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// 启动清理任务（删除过期的黑名单条目）
    pub fn start_cleanup_task(self) {
        tokio::spawn(async move {
            loop {
                sleep(FAILED_TARGET_CLEANUP_INTERVAL).await;
                self.cleanup_expired_targets().await;
            }
        });
    }

    /// 检查目标是否在黑名单中
    pub async fn is_blacklisted(&self, target: &str) -> bool {
        let targets = self.targets.read().await;
        if let Some(failed) = targets.get(target) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            // 检查是否还在黑名单期内
            if now < failed.blacklist_time + FAILED_TARGET_TIMEOUT.as_secs() {
                return true;
            }
        }
        false
    }

    /// 记录连接失败
    pub async fn record_failure(&self, target: &str) {
        let mut targets = self.targets.write().await;
        let entry = targets.entry(target.to_string()).or_insert_with(|| {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            FailedTarget {
                failure_count: 0,
                blacklist_time: now,
            }
        });

        entry.failure_count += 1;

        if entry.failure_count >= FAILED_TARGET_THRESHOLD {
            // 更新黑名单时间为当前时间
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            entry.blacklist_time = now;
            warn!(
                "Target '{}' added to blacklist due to {} consecutive failures",
                target, entry.failure_count
            );
        }
    }

    /// 清理过期的黑名单条目
    async fn cleanup_expired_targets(&self) {
        let mut targets = self.targets.write().await;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let expired_targets: Vec<String> = targets
            .iter()
            .filter(|(_, failed)| now >= failed.blacklist_time + FAILED_TARGET_TIMEOUT.as_secs())
            .map(|(target, _)| target.clone())
            .collect();

        for target in expired_targets {
            targets.remove(&target);
            info!("Removed expired target '{}' from blacklist", target);
        }
    }

    /// 获取失败目标数量（用于统计）
    #[allow(dead_code)]
    pub async fn failed_targets_count(&self) -> usize {
        let targets = self.targets.read().await;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        targets
            .iter()
            .filter(|(_, failed)| now < failed.blacklist_time + FAILED_TARGET_TIMEOUT.as_secs())
            .count()
    }
}

impl Default for FailedTargetManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 实时统计的数据复制函数（带超时保护）
/// 相比 tokio::io::copy，这个函数会在每次复制数据后立即更新统计信息
/// 并在连接空闲超过 CONNECTION_IDLE_TIMEOUT 时自动关闭
async fn copy_with_stats<R, W>(
    reader: &mut R,
    writer: &mut W,
    stats_tracker: Option<&ClientStatsTracker>,
    record_fn: impl Fn(&ClientStatsTracker, u64),
) -> std::io::Result<u64>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    use tokio::time::timeout;

    let mut buf = vec![0u8; COPY_BUFFER_SIZE];
    let mut total_copied = 0u64;

    loop {
        // 使用 timeout 防止连接永久挂起（连接空闲超时保护）
        let result = timeout(CONNECTION_IDLE_TIMEOUT, reader.read(&mut buf)).await;

        let n = match result {
            Ok(Ok(n)) => n,
            Ok(Err(e)) => return Err(e),
            Err(_) => {
                // 超时：连接5分钟无数据传输，主动关闭
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "Connection idle timeout",
                ));
            }
        };

        if n == 0 {
            break;
        }

        writer.write_all(&buf[..n]).await?;
        total_copied += n as u64;

        // 实时更新统计信息
        if let Some(tracker) = stats_tracker {
            record_fn(tracker, n as u64);
        }
    }

    Ok(total_copied)
}

/// 运行 forwarder 监听器
/// 在客户端本地监听端口，接受连接后解析目标地址并通过 yamux 转发到服务器
pub async fn run_forwarder_listener(
    forwarder: ForwarderConfig,
    stream_tx: tokio::sync::mpsc::Sender<tokio::sync::oneshot::Sender<Result<yamux::Stream>>>,
    router: Option<Arc<GeoIpRouter>>,
    stats_tracker: Option<ClientStatsTracker>,
    mut shutdown_rx: tokio::sync::broadcast::Receiver<()>,
) -> Result<()> {
    let bind_addr = format!("{}:{}", forwarder.bind_addr, forwarder.bind_port);

    info!(
        "Forwarder '{}': Binding to {} ({})",
        forwarder.name,
        bind_addr,
        format_proxy_type(forwarder.proxy_type)
    );

    let listener = TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("Failed to bind forwarder to {}", bind_addr))?;

    info!("Forwarder '{}': Listening on {}", forwarder.name, bind_addr);

    // 创建信号量限制并发连接数
    let connection_limiter = Arc::new(Semaphore::new(MAX_CONCURRENT_CONNECTIONS));
    info!(
        "Forwarder '{}': Maximum concurrent connections: {}",
        forwarder.name, MAX_CONCURRENT_CONNECTIONS
    );

    // 创建连接池缓存
    let connection_pool = Arc::new(ConnectionPool::new(
        100,                      // 最多缓存 100 个目标的连接
        Duration::from_secs(300), // 连接空闲 5 分钟后过期
    ));
    info!(
        "Forwarder '{}': Connection pool initialized (max targets: 100, idle timeout: 5min)",
        forwarder.name
    );

    // 启动连接池清理任务（定期清理过期连接）
    let pool_cleanup = connection_pool.clone();
    tokio::spawn(async move {
        loop {
            sleep(Duration::from_secs(60)).await;
            pool_cleanup.cleanup_expired().await;
        }
    });

    // 创建快速失败管理器并启动清理任务
    let failed_target_manager = FailedTargetManager::new();
    failed_target_manager.clone().start_cleanup_task();
    info!(
        "Forwarder '{}': Fast-fail manager initialized (threshold: {}, timeout: {:?})",
        forwarder.name, FAILED_TARGET_THRESHOLD, FAILED_TARGET_TIMEOUT
    );

    loop {
        tokio::select! {
            accept_result = listener.accept() => {
                match accept_result {
                    Ok((local_stream, peer_addr)) => {
                        // 优化 TCP 选项以降低延迟和防止连接断开
                        if let Err(e) = local_stream.set_nodelay(true) {
                            warn!("Failed to set TCP_NODELAY: {}", e);
                        }

                        // 尝试获取连接许可
                        let permit = match connection_limiter.clone().try_acquire_owned() {
                            Ok(permit) => permit,
                            Err(_) => {
                                warn!(
                                    "Forwarder '{}': Connection limit reached ({}), rejecting connection from {}",
                                    forwarder.name, MAX_CONCURRENT_CONNECTIONS, peer_addr
                                );
                                // 直接关闭连接
                                drop(local_stream);
                                continue;
                            }
                        };

                        info!(
                            "Forwarder '{}': Accepted connection from {}",
                            forwarder.name, peer_addr
                        );

                        let forwarder_clone = forwarder.clone();
                        let stream_tx_clone = stream_tx.clone();
                        let router_clone = router.clone();
                        let stats_tracker_clone = stats_tracker.clone();
                        let failed_target_manager_clone = failed_target_manager.clone();
                        let connection_pool_clone = connection_pool.clone();

                        tokio::spawn(async move {
                            // 持有 permit 直到任务结束，自动释放
                            let _permit = permit;
                            if let Err(e) = handle_forwarder_connection(
                                local_stream,
                                &forwarder_clone,
                                stream_tx_clone,
                                router_clone,
                                stats_tracker_clone,
                                failed_target_manager_clone,
                                connection_pool_clone,
                            )
                            .await
                            {
                                error!(
                                    "Forwarder '{}' connection handling error: {}",
                                    forwarder_clone.name, e
                                );
                            }
                        });
                    }
                    Err(e) => {
                        error!("Forwarder '{}': Accept error: {}", forwarder.name, e);
                        sleep(Duration::from_millis(100)).await;
                    }
                }
            }
            // 监听 shutdown 信号
            _ = shutdown_rx.recv() => {
                info!("Forwarder '{}': Shutting down due to connection loss", forwarder.name);
                break Ok(());
            }
        }
    }
}

/// 处理 forwarder 连接
/// 根据协议类型解析目标地址，然后通过 yamux stream 转发到服务器或直连
async fn handle_forwarder_connection(
    mut local_stream: TcpStream,
    forwarder: &ForwarderConfig,
    stream_tx: tokio::sync::mpsc::Sender<tokio::sync::oneshot::Sender<Result<yamux::Stream>>>,
    router: Option<Arc<GeoIpRouter>>,
    stats_tracker: Option<ClientStatsTracker>,
    failed_target_manager: FailedTargetManager,
    connection_pool: Arc<ConnectionPool>,
) -> Result<()> {
    // 获取客户端地址用于审计日志
    let peer_addr = local_stream
        .peer_addr()
        .map(|addr| addr.to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    // 记录连接开始
    if let Some(ref tracker) = stats_tracker {
        tracker.connection_started();
    }

    // 1. 根据 proxy_type 解析目标地址
    let (target, http_direct_request) = match forwarder.proxy_type {
        ProxyType::HttpProxy => {
            // 解析 HTTP 请求（支持 CONNECT 和直接转发）
            let req = parse_http_request(&mut local_stream).await?;

            match req.method.as_str() {
                "CONNECT" => {
                    // CONNECT 隧道模式
                    handle_http_connect(&mut local_stream, &req.target).await?;
                    (req.target.clone(), None)
                }
                _ => {
                    // HTTP 直接转发（GET, POST 等）
                    let (modified_request, target) =
                        handle_http_direct(&mut local_stream, &req).await?;
                    (target, Some(modified_request))
                }
            }
        }
        ProxyType::Socks5Proxy => {
            let target = parse_socks5(&mut local_stream).await?;
            // SOCKS5 始终是隧道模式
            (target, None)
        }
        _ => anyhow::bail!(
            "Invalid proxy type for forwarder: {:?}",
            forwarder.proxy_type
        ),
    };

    // 如果是 HTTP 直接转发（而非 CONNECT），需要直接转发修改后的请求
    if let Some(request_data) = http_direct_request {
        // 检查目标是否在黑名单中
        if failed_target_manager.is_blacklisted(&target).await {
            warn!(
                "Forwarder '{}': Target '{}' is blacklisted due to previous failures, rejecting immediately",
                forwarder.name, target
            );
            let error_response = b"HTTP/1.1 503 Service Unavailable\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nService temporarily unavailable";
            local_stream.write_all(error_response).await.ok();

            if let Some(ref tracker) = stats_tracker {
                tracker.connection_ended();
            }
            return Err(anyhow::anyhow!("Target blacklisted"));
        }

        // 直接连接目标并转发请求（使用连接池）
        let mut remote_stream = match connection_pool.get_or_create(&target).await {
            Ok(stream) => {
                info!(
                    "Forwarder '{}': Got connection from pool to {}",
                    forwarder.name, target
                );
                ReusableConnection::new(stream, target.clone(), connection_pool.clone())
            }
            Err(e) => {
                failed_target_manager.record_failure(&target).await;
                local_stream.write_all(b"HTTP/1.1 502 Bad Gateway\r\nContent-Type: text/plain\r\nConnection: close\r\n\r\nConnection failed").await.ok();

                if let Some(ref tracker) = stats_tracker {
                    tracker.connection_ended();
                }
                return Err(e);
            }
        };

        // 发送修改后的请求
        if let Some(stream) = remote_stream.get_mut() {
            stream.write_all(&request_data).await?;
        }

        // 双向数据转发
        let (mut local_read, mut local_write) = local_stream.split();

        // 为了支持可复用连接，我们需要手动处理转发
        // 而不是使用 split()（split 会消耗所有权）
        // 因此我们采用循环转发的方式

        if let Some(stream) = remote_stream.get_mut() {
            let (mut remote_read, mut remote_write) = stream.split();

            let stats_c2r = stats_tracker.clone();
            let forwarder_msg_1 = forwarder.name.clone();
            let c2r = async {
                let result = copy_with_stats(
                    &mut local_read,
                    &mut remote_write,
                    stats_c2r.as_ref(),
                    |t, n| t.record_bytes_sent(n),
                )
                .await;

                if let Err(e) = &result {
                    warn!(
                        "Forwarder '{}': Client to remote copy error: {}",
                        forwarder_msg_1, e
                    );
                }

                // 注意：不调用 shutdown()，让连接保持可复用状态
                result
            };

            let stats_r2c = stats_tracker.clone();
            let forwarder_msg_2 = forwarder.name.clone();
            let r2c = async {
                let result = copy_with_stats(
                    &mut remote_read,
                    &mut local_write,
                    stats_r2c.as_ref(),
                    |t, n| t.record_bytes_received(n),
                )
                .await;

                if let Err(e) = &result {
                    warn!(
                        "Forwarder '{}': Remote to client copy error: {}",
                        forwarder_msg_2, e
                    );
                }

                // 注意：不调用 shutdown()，让连接保持可复用状态
                result
            };

            let (c2r_result, r2c_result) = tokio::join!(c2r, r2c);

            // 如果发生错误，标记连接以便不返还到池
            if c2r_result.is_err() || r2c_result.is_err() {
                warn!(
                    "Forwarder '{}': Data transfer completed with some errors",
                    forwarder.name
                );
                remote_stream.mark_error();
            }
        }

        if let Some(ref tracker) = stats_tracker {
            tracker.connection_ended();
        }

        // ReusableConnection 会在 drop 时自动返还连接到池
        drop(remote_stream);

        return Ok(());
    }

    // 检查目标是否在黑名单中（快速失败）
    if failed_target_manager.is_blacklisted(&target).await {
        warn!(
            "Forwarder '{}': Target '{}' is blacklisted due to previous failures, rejecting immediately",
            forwarder.name, target
        );
        let error_response = "HTTP/1.1 503 Service Unavailable\r\n\
             Content-Type: text/plain\r\n\
             Content-Length: 47\r\n\
             Connection: close\r\n\
             \r\n\
             Target is temporarily unavailable (blacklisted)"
            .to_string();
        local_stream.write_all(error_response.as_bytes()).await.ok();

        // 记录连接结束
        if let Some(ref tracker) = stats_tracker {
            tracker.connection_ended();
        }
        return Err(anyhow::anyhow!(
            "Target '{}' is blacklisted due to previous failures",
            target
        ));
    }

    // 2. 判断是否应该直连
    let should_direct = router
        .as_ref()
        .map(|r| r.should_direct_connect(&target))
        .unwrap_or(false);

    // 审计日志：记录连接详情和路由决策
    if should_direct {
        info!(
            "Forwarder '{}': Connection from {} to {} -> DIRECT (bypassing proxy)",
            forwarder.name, peer_addr, target
        );
        let result = handle_direct_connection(
            local_stream,
            &target,
            &forwarder.name,
            stats_tracker.clone(),
            failed_target_manager.clone(),
            connection_pool.clone(),
            true, // 路由规则明确指定直连，跳过安全检查
        )
        .await;
        // 无论成功或失败，都记录连接结束
        if let Some(ref tracker) = stats_tracker {
            tracker.connection_ended();
        }
        return result;
    } else {
        info!(
            "Forwarder '{}': Connection from {} to {} -> PROXY (via server)",
            forwarder.name, peer_addr, target
        );
    }

    // 3. 请求创建新的 yamux stream
    let (response_tx, response_rx) = tokio::sync::oneshot::channel();
    stream_tx
        .send(response_tx)
        .await
        .context("Failed to request yamux stream")?;

    // 等待 yamux stream 创建完成
    let server_stream = response_rx
        .await
        .context("Failed to receive yamux stream")??;

    info!(
        "Forwarder '{}': Opened stream to server for target {}",
        forwarder.name, target
    );

    // 将 yamux stream 转换为兼容的 tokio stream
    let mut server_stream_tokio = server_stream.compat();

    // 3. 发送特殊 name 携带目标地址：@forward:target
    let forward_name = format!("@forward:{}", target);
    let name_bytes = forward_name.as_bytes();
    let name_len = (name_bytes.len() as u16).to_be_bytes();
    server_stream_tokio.write_all(&name_len).await?;
    server_stream_tokio.write_all(name_bytes).await?;

    // 发送 publish_port = 0（占位，不使用）
    let port_bytes = 0u16.to_be_bytes();
    server_stream_tokio.write_all(&port_bytes).await?;
    server_stream_tokio.flush().await?;

    info!(
        "Forwarder '{}': Sent forward request for target {}",
        forwarder.name, target
    );

    // 4. 等待服务器确认（1 字节：1=成功，0=失败）
    let mut confirm = [0u8; 1];
    server_stream_tokio.read_exact(&mut confirm).await?;

    if confirm[0] != 1 {
        // 读取错误消息
        let error_msg = match read_error_message(&mut server_stream_tokio).await {
            Ok(msg) => msg,
            Err(_) => "Unknown error".to_string(),
        };
        error!(
            "Forwarder '{}': Server rejected connection to '{}': {}",
            forwarder.name, target, error_msg
        );

        // 记录连接失败
        failed_target_manager.record_failure(&target).await;

        // 如果是 HTTP 代理，返回错误给客户端
        if forwarder.proxy_type == ProxyType::HttpProxy {
            let error_response = format!(
                "HTTP/1.1 502 Bad Gateway\r\n\
                 Content-Type: text/plain\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\
                 \r\n\
                 {}",
                error_msg.len(),
                error_msg
            );
            local_stream.write_all(error_response.as_bytes()).await.ok();
        }

        // 记录连接结束
        if let Some(ref tracker) = stats_tracker {
            tracker.connection_ended();
        }

        return Err(anyhow::anyhow!(
            "Server rejected forwarder connection: {}",
            error_msg
        ));
    }

    info!(
        "Forwarder '{}': Server accepted connection, starting data transfer",
        forwarder.name
    );

    // 5. 双向转发数据
    let (mut local_read, mut local_write) = local_stream.split();
    let (mut server_read, mut server_write) = tokio::io::split(server_stream_tokio);

    let stats_tracker_c2s = stats_tracker.clone();
    let client_to_server = async move {
        let bytes = copy_with_stats(
            &mut local_read,
            &mut server_write,
            stats_tracker_c2s.as_ref(),
            |tracker, n| tracker.record_bytes_sent(n),
        )
        .await?;
        server_write.shutdown().await?;
        Ok::<_, std::io::Error>(bytes)
    };

    let stats_tracker_s2c = stats_tracker.clone();
    let server_to_client = async move {
        let bytes = copy_with_stats(
            &mut server_read,
            &mut local_write,
            stats_tracker_s2c.as_ref(),
            |tracker, n| tracker.record_bytes_received(n),
        )
        .await?;
        local_write.shutdown().await?;
        Ok::<_, std::io::Error>(bytes)
    };

    // 使用 tokio::join! 确保两个方向的流量都被记录
    let (result_c2s, result_s2c) = tokio::join!(client_to_server, server_to_client);

    if let Err(e) = result_c2s {
        warn!(
            "Forwarder '{}': Client to server copy error: {}",
            forwarder.name, e
        );
    }
    if let Err(e) = result_s2c {
        warn!(
            "Forwarder '{}': Server to client copy error: {}",
            forwarder.name, e
        );
    }

    info!("Forwarder '{}': Connection closed", forwarder.name);

    // 记录连接结束
    if let Some(ref tracker) = stats_tracker {
        tracker.connection_ended();
    }

    Ok(())
}

/// HTTP 请求方法和元数据
#[derive(Debug)]
struct HttpRequest {
    method: String,
    target: String,
    headers: std::collections::HashMap<String, String>,
    #[allow(dead_code)]
    raw_request: Vec<u8>,
}

/// 解析 HTTP 请求（支持 CONNECT 和直接转发）
async fn parse_http_request(stream: &mut TcpStream) -> Result<HttpRequest> {
    use tokio::time::timeout;

    let result = timeout(PROTOCOL_PARSE_TIMEOUT, async {
        let mut buffer = vec![0u8; HTTP_PARSE_BUFFER_SIZE];
        let mut pos = 0;

        // 读取到第一个 \r\n\r\n
        loop {
            let n = stream.read(&mut buffer[pos..]).await?;
            if n == 0 {
                anyhow::bail!("Unexpected EOF while reading HTTP request");
            }
            pos += n;

            // 查找 \r\n\r\n
            if pos >= 4 {
                for i in 0..=pos - 4 {
                    if &buffer[i..i + 4] == b"\r\n\r\n" {
                        let request_buf = buffer[..i + 4].to_vec();
                        let request = String::from_utf8_lossy(&request_buf);
                        let lines: Vec<&str> = request.lines().collect();

                        if lines.is_empty() {
                            anyhow::bail!("Empty HTTP request");
                        }

                        let first_line = lines[0];
                        let parts: Vec<&str> = first_line.split_whitespace().collect();
                        if parts.len() < 2 {
                            anyhow::bail!("Invalid HTTP request line");
                        }

                        let method = parts[0].to_string();
                        let target = parts[1].to_string();

                        // 解析 headers
                        let mut headers = std::collections::HashMap::new();
                        for line in &lines[1..] {
                            if let Some(colon_pos) = line.find(':') {
                                let key = line[..colon_pos].trim().to_lowercase();
                                let value = line[colon_pos + 1..].trim().to_string();
                                headers.insert(key, value);
                            }
                        }

                        return Ok::<HttpRequest, anyhow::Error>(HttpRequest {
                            method,
                            target,
                            headers,
                            raw_request: request_buf,
                        });
                    }
                }
            }

            if pos >= HTTP_PARSE_BUFFER_SIZE {
                anyhow::bail!("HTTP request too long");
            }
        }
    })
    .await
    .map_err(|_| anyhow::anyhow!("HTTP parsing timeout after {:?}", PROTOCOL_PARSE_TIMEOUT))??;

    Ok(result)
}

/// 处理 HTTP CONNECT 请求（隧道模式）
async fn handle_http_connect(stream: &mut TcpStream, target: &str) -> Result<()> {
    if !target.contains(':') {
        anyhow::bail!("Invalid target address: {}", target);
    }

    let response = b"HTTP/1.1 200 Connection Established\r\n\r\n";
    stream.write_all(response).await?;
    stream.flush().await?;

    Ok(())
}

/// 处理 HTTP 直接转发（如 GET, POST 等）
async fn handle_http_direct(
    _stream: &mut TcpStream,
    req: &HttpRequest,
) -> Result<(Vec<u8>, String)> {
    // 解析目标
    let target = if req.target.starts_with("http://") || req.target.starts_with("https://") {
        // 绝对 URL
        let url =
            url::Url::parse(&req.target).map_err(|e| anyhow::anyhow!("Invalid URL: {}", e))?;
        let host = url
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("No host in URL"))?;
        let port = url
            .port()
            .unwrap_or(if url.scheme() == "https" { 443 } else { 80 });
        format!("{}:{}", host, port)
    } else if let Some(host_header) = req.headers.get("host") {
        // 使用 Host header
        host_header.clone()
    } else {
        anyhow::bail!("Cannot determine target from HTTP request");
    };

    // 重建请求（修改为相对路径）
    let path = if req.target.starts_with("http") {
        url::Url::parse(&req.target)
            .ok()
            .map(|u| {
                let mut path = u.path().to_string();
                if let Some(query) = u.query() {
                    path.push('?');
                    path.push_str(query);
                }
                path
            })
            .unwrap_or_else(|| "/".to_string())
    } else {
        req.target.clone()
    };

    let mut modified_request = format!("{} {} HTTP/1.1\r\n", req.method, path).into_bytes();

    // 重建 headers（去除 Proxy-Connection 等代理相关 header）
    for (key, value) in &req.headers {
        if !matches!(
            key.as_str(),
            "proxy-connection" | "connection" | "keep-alive"
        ) {
            modified_request.extend(format!("{}: {}\r\n", key, value).into_bytes());
        }
    }
    modified_request.extend(b"Connection: close\r\n\r\n");

    Ok((modified_request, target))
}

/// 解析 SOCKS5 请求
async fn parse_socks5(stream: &mut TcpStream) -> Result<String> {
    use tokio::time::timeout;

    // 使用超时包装整个解析过程
    let result = timeout(PROTOCOL_PARSE_TIMEOUT, async {
        // SOCKS5 握手 - 读取客户端方法选择
        let mut header = [0u8; 2];
        stream.read_exact(&mut header).await?;

        if header[0] != 0x05 {
            anyhow::bail!("Unsupported SOCKS version: {}", header[0]);
        }

        let nmethods = header[1] as usize;
        if nmethods == 0 || nmethods > 255 {
            anyhow::bail!("Invalid number of methods: {}", nmethods);
        }

        // 读取方法列表
        let mut methods = vec![0u8; nmethods];
        stream.read_exact(&mut methods).await?;

        // 响应：选择无认证方法 (0x00)
        let response = [0x05, 0x00];
        stream.write_all(&response).await?;
        stream.flush().await?;

        // 读取 SOCKS5 请求
        let mut request = [0u8; 4];
        stream.read_exact(&mut request).await?;

        if request[0] != 0x05 {
            anyhow::bail!("Invalid SOCKS5 request version");
        }

        let cmd = request[1];
        if cmd != 0x01 {
            // 只支持 CONNECT 命令
            let response = [0x05, 0x07, 0x00, 0x01, 0, 0, 0, 0, 0, 0]; // Command not supported
            stream.write_all(&response).await?;
            anyhow::bail!("Unsupported SOCKS5 command: {}", cmd);
        }

        let atyp = request[3];

        // 解析目标地址
        let host = match atyp {
            0x01 => {
                // IPv4
                let mut addr = [0u8; 4];
                stream.read_exact(&mut addr).await?;
                format!("{}.{}.{}.{}", addr[0], addr[1], addr[2], addr[3])
            }
            0x03 => {
                // 域名
                let mut len = [0u8; 1];
                stream.read_exact(&mut len).await?;
                let len = len[0] as usize;

                if len == 0 || len > 255 {
                    anyhow::bail!("Invalid SOCKS5 domain name length: {}", len);
                }

                let mut domain = vec![0u8; len];
                stream.read_exact(&mut domain).await?;
                String::from_utf8(domain)?
            }
            0x04 => {
                // IPv6
                let mut addr = [0u8; 16];
                stream.read_exact(&mut addr).await?;
                format!(
                    "{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}:{:02x}{:02x}",
                    addr[0], addr[1], addr[2], addr[3], addr[4], addr[5], addr[6], addr[7],
                    addr[8], addr[9], addr[10], addr[11], addr[12], addr[13], addr[14], addr[15]
                )
            }
            _ => anyhow::bail!("Unsupported address type: {}", atyp),
        };

        // 读取端口
        let mut port_bytes = [0u8; 2];
        stream.read_exact(&mut port_bytes).await?;
        let port = u16::from_be_bytes(port_bytes);

        let target = format!("{}:{}", host, port);

        // 发送成功响应
        let response = [
            0x05, 0x00, 0x00, 0x01, // VER, REP, RSV, ATYP
            0, 0, 0, 0, // BND.ADDR (0.0.0.0)
            0, 0, // BND.PORT (0)
        ];
        stream.write_all(&response).await?;
        stream.flush().await?;

        Ok::<String, anyhow::Error>(target)
    })
    .await
    .map_err(|_| anyhow::anyhow!("SOCKS5 parsing timeout after {:?}", PROTOCOL_PARSE_TIMEOUT))??;

    Ok(result)
}

/// 格式化代理类型为可读字符串
fn format_proxy_type(proxy_type: ProxyType) -> &'static str {
    match proxy_type {
        ProxyType::HttpProxy => "HTTP proxy",
        ProxyType::Socks5Proxy => "SOCKS5 proxy",
        _ => "Unknown",
    }
}

/// 检查目标地址是否为本地或私有地址（用于客户端直连安全检查）
fn is_unsafe_direct_target(target: &str) -> bool {
    use std::net::{IpAddr, ToSocketAddrs};

    // 提取主机名部分（移除端口）
    let host = if let Some(colon_pos) = target.rfind(':') {
        // 检查是否为 IPv6 地址（包含多个冒号）
        if target.matches(':').count() > 1 {
            // IPv6 地址，查找方括号
            if let Some(bracket_end) = target.find(']') {
                &target[1..bracket_end] // [ipv6]:port -> ipv6
            } else {
                target // 纯 IPv6 地址
            }
        } else {
            &target[..colon_pos] // host:port -> host
        }
    } else {
        target
    };

    // 检查是否为明确的本地主机名（完全匹配）
    if host == "localhost" || host == "127.0.0.1" || host == "::1" {
        return true;
    }

    // 解析地址（添加端口以便解析）
    let addr_str = if target.contains(':') {
        target.to_string()
    } else {
        format!("{}:80", target) // 添加默认端口以便解析
    };

    // 尝试解析域名/IP
    match addr_str.to_socket_addrs() {
        Ok(addrs) => {
            for addr in addrs {
                let ip = addr.ip();

                // 检查是否为本地地址
                if ip.is_loopback() {
                    return true;
                }

                // 检查是否为私有地址（内网地址）
                match ip {
                    IpAddr::V4(ipv4) => {
                        let octets = ipv4.octets();
                        // 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16, 0.0.0.0/8
                        if octets[0] == 10
                            || (octets[0] == 172 && (octets[1] >= 16 && octets[1] <= 31))
                            || (octets[0] == 192 && octets[1] == 168)
                            || (octets[0] == 169 && octets[1] == 254)
                            || octets[0] == 0
                        {
                            return true;
                        }
                    }
                    IpAddr::V6(ipv6) => {
                        // Unique local address (ULA) 或 Link-local
                        if ipv6.is_unique_local() || ipv6.is_unicast_link_local() {
                            return true;
                        }
                    }
                }
            }
            false
        }
        Err(e) => {
            // DNS 解析失败可能是临时问题，不应该直接拒绝
            // 记录警告信息，让连接尝试继续（连接失败会有自己的错误处理）
            debug!(
                "Failed to resolve address '{}' for safety check: {}. Allowing connection attempt.",
                target, e
            );
            false
        }
    }
}

// ============= Forwarder 处理器 =============

/// Forwarder 代理处理器
pub struct ForwarderHandler {
    pub config: ForwarderConfig,
    pub stream_tx: tokio::sync::mpsc::Sender<tokio::sync::oneshot::Sender<Result<yamux::Stream>>>,
    pub router: Option<Arc<GeoIpRouter>>,
    pub status: Arc<RwLock<crate::client::HandlerStatus>>,
    pub shutdown_tx: Arc<RwLock<Option<tokio::sync::oneshot::Sender<()>>>>,
    pub stats_tracker: Option<ClientStatsTracker>,
}

impl ForwarderHandler {
    /// 创建新的 ForwarderHandler
    pub fn new(
        config: ForwarderConfig,
        stream_tx: tokio::sync::mpsc::Sender<tokio::sync::oneshot::Sender<Result<yamux::Stream>>>,
        router: Option<Arc<GeoIpRouter>>,
        stats_tracker: Option<ClientStatsTracker>,
    ) -> Self {
        Self {
            config,
            stream_tx,
            router,
            status: Arc::new(RwLock::new(crate::client::HandlerStatus::Stopped)),
            shutdown_tx: Arc::new(RwLock::new(None)),
            stats_tracker,
        }
    }
}

/// 处理直连（不通过服务器）
async fn handle_direct_connection(
    mut local_stream: TcpStream,
    target: &str,
    forwarder_name: &str,
    stats_tracker: Option<ClientStatsTracker>,
    failed_target_manager: FailedTargetManager,
    connection_pool: Arc<ConnectionPool>,
    bypass_safety_check: bool,
) -> Result<()> {
    // 安全检查：禁止访问本地地址和内网地址（防止 SSRF 攻击）
    // 但如果是路由规则明确指定的，则允许（bypass_safety_check = true）
    if !bypass_safety_check && is_unsafe_direct_target(target) {
        warn!(
            "Forwarder '{}': Blocked direct connection to local/private address: {}",
            forwarder_name, target
        );
        return Err(anyhow::anyhow!(
            "Direct connection to local or private addresses is not allowed for security reasons"
        ));
    }

    // 使用连接池获取或创建到目标服务器的连接
    let mut remote_stream = match connection_pool.get_or_create(target).await {
        Ok(stream) => {
            info!(
                "Forwarder '{}': Got connection from pool to {} (or created new)",
                forwarder_name, target
            );
            ReusableConnection::new(stream, target.to_string(), connection_pool.clone())
        }
        Err(e) => {
            error!(
                "Forwarder '{}': Failed to connect directly to '{}': {}",
                forwarder_name, target, e
            );

            // 记录连接失败
            failed_target_manager.record_failure(target).await;

            return Err(anyhow::anyhow!(
                "Failed to connect directly to {}: {}",
                target,
                e
            ));
        }
    };

    // 双向转发数据（使用可复用连接包装器）
    let (mut local_read, mut local_write) = local_stream.split();

    if let Some(stream) = remote_stream.get_mut() {
        let (mut remote_read, mut remote_write) = stream.split();

        let stats_tracker_c2r = stats_tracker.clone();
        let name_msg_c2r = forwarder_name.to_string();
        let client_to_remote = async move {
            let result = copy_with_stats(
                &mut local_read,
                &mut remote_write,
                stats_tracker_c2r.as_ref(),
                |tracker, n| tracker.record_bytes_sent(n),
            )
            .await;
            if let Err(e) = &result {
                warn!(
                    "Forwarder '{}' direct: Client to remote error: {}",
                    name_msg_c2r, e
                );
            }
            // 注意：不调用 shutdown()，让连接保持可复用状态
            result
        };

        let stats_tracker_r2c = stats_tracker.clone();
        let name_msg_r2c = forwarder_name.to_string();
        let remote_to_client = async move {
            let result = copy_with_stats(
                &mut remote_read,
                &mut local_write,
                stats_tracker_r2c.as_ref(),
                |tracker, n| tracker.record_bytes_received(n),
            )
            .await;
            if let Err(e) = &result {
                warn!(
                    "Forwarder '{}' direct: Remote to client error: {}",
                    name_msg_r2c, e
                );
            }
            // 注意：不调用 shutdown()，让连接保持可复用状态
            result
        };

        // 使用 tokio::join! 确保两个方向的流量都被记录
        let (result_c2r, result_r2c) = tokio::join!(client_to_remote, remote_to_client);

        if result_c2r.is_err() || result_r2c.is_err() {
            warn!(
                "Forwarder '{}': Data transfer completed with some errors",
                forwarder_name
            );
            // 如果发生错误，标记连接以便不返还到池
            remote_stream.mark_error();
        }
    }

    info!(
        "Forwarder '{}': Direct connection to {} completed, returning to pool",
        forwarder_name, target
    );

    // ReusableConnection 会在 drop 时自动将连接返还到池或丢弃坏连接
    drop(remote_stream);

    Ok(())
}

#[async_trait]
impl ProxyHandler for ForwarderHandler {
    async fn start(&self) -> Result<()> {
        {
            let mut status = self.status.write().await;
            *status = crate::client::HandlerStatus::Starting;
        }

        let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel();
        {
            let mut tx = self.shutdown_tx.write().await;
            *tx = Some(shutdown_tx);
        }

        {
            let mut status = self.status.write().await;
            *status = crate::client::HandlerStatus::Running;
        }

        let config = self.config.clone();
        let stream_tx = self.stream_tx.clone();
        let router = self.router.clone();
        let status = self.status.clone();
        let stats_tracker = self.stats_tracker.clone();

        // 创建内部的 shutdown channel 用于 listener
        let (listener_shutdown_tx, listener_shutdown_rx) = tokio::sync::broadcast::channel::<()>(1);

        tokio::select! {
            result = run_forwarder_listener(config, stream_tx, router, stats_tracker, listener_shutdown_rx) => {
                if let Err(e) = &result {
                    let mut s = status.write().await;
                    *s = crate::client::HandlerStatus::Failed(e.to_string());
                }
                result
            }
            _ = &mut shutdown_rx => {
                let mut s = status.write().await;
                *s = crate::client::HandlerStatus::Stopped;
                // 通知 listener 关闭
                let _ = listener_shutdown_tx.send(());
                Ok(())
            }
        }
    }

    async fn stop(&self) -> Result<()> {
        {
            let mut status = self.status.write().await;
            *status = crate::client::HandlerStatus::Stopping;
        }

        let mut tx_lock = self.shutdown_tx.write().await;
        if let Some(tx) = tx_lock.take() {
            let _ = tx.send(());
        }

        {
            let mut status = self.status.write().await;
            *status = crate::client::HandlerStatus::Stopped;
        }

        Ok(())
    }

    async fn health_check(&self) -> bool {
        let status = self.status.read().await;
        matches!(*status, crate::client::HandlerStatus::Running)
    }

    fn status(&self) -> crate::client::HandlerStatus {
        // 注意：这里使用 blocking read，在实际应用中可能需要改为异步
        match self.status.try_read() {
            Ok(s) => s.clone(),
            Err(_) => crate::client::HandlerStatus::Running, // 默认返回 Running
        }
    }

    fn name(&self) -> &str {
        &self.config.name
    }

    fn proxy_type(&self) -> ProxyType {
        self.config.proxy_type
    }

    fn bind_address(&self) -> String {
        format!("{}:{}", self.config.bind_addr, self.config.bind_port)
    }
}
/// 检查连接是否还活着（简单的健康检查）
/// 通过尝试设置 TCP_NODELAY 来验证连接是否仍然有效
async fn is_connection_alive(stream: &TcpStream) -> bool {
    // 尝试获取当前的 TCP_NODELAY 状态
    // 如果连接已断开，这个操作会失败
    match stream.nodelay() {
        Ok(_) => true, // 连接仍然有效
        Err(_) => {
            warn!("Connection health check failed: connection is no longer alive");
            false // 连接已断开
        }
    }
}

/// 可复用的连接包装器（RAII 模式）
/// 在 drop 时自动将连接返还到池，或丢弃已关闭的连接
struct ReusableConnection {
    stream: Option<TcpStream>,
    target: String,
    pool: Arc<ConnectionPool>,
    had_error: bool, // 标记是否发生过错误
}

impl ReusableConnection {
    fn new(stream: TcpStream, target: String, pool: Arc<ConnectionPool>) -> Self {
        Self {
            stream: Some(stream),
            target,
            pool,
            had_error: false,
        }
    }

    /// 标记连接发生过错误
    fn mark_error(&mut self) {
        self.had_error = true;
    }

    /// 获取可变引用用于读写
    fn get_mut(&mut self) -> Option<&mut TcpStream> {
        self.stream.as_mut()
    }

    /// 获取不可变引用用于读
    #[allow(dead_code)]
    fn get(&self) -> Option<&TcpStream> {
        self.stream.as_ref()
    }

    /// 获取所有权（消费连接）
    #[allow(dead_code)]
    fn into_inner(mut self) -> Option<TcpStream> {
        self.stream.take()
    }
}

impl Drop for ReusableConnection {
    fn drop(&mut self) {
        if let Some(stream) = self.stream.take() {
            // 如果连接发生过错误，直接丢弃而不是返还到池
            if self.had_error {
                debug!(
                    "Discarding connection to {} due to previous errors",
                    self.target
                );
                return;
            }

            // 异步任务中将连接返还到池
            let target = self.target.clone();
            let pool = self.pool.clone();
            tokio::spawn(async move {
                pool.return_connection(target, stream).await;
            });
        }
    }
}

// ============= 优化 5: 连接池缓存 =============

/// 连接池项
#[allow(dead_code)]
struct PooledConnection {
    stream: TcpStream,
    created_at: std::time::Instant,
}

/// 连接池缓存
#[allow(dead_code)]
pub struct ConnectionPool {
    pools: Arc<RwLock<HashMap<String, Vec<PooledConnection>>>>,
    max_pool_size: usize,
    max_idle_time: Duration,
}

impl ConnectionPool {
    pub fn new(max_pool_size: usize, max_idle_time: Duration) -> Self {
        Self {
            pools: Arc::new(RwLock::new(HashMap::new())),
            max_pool_size,
            max_idle_time,
        }
    }

    /// 从池中获取或创建连接
    pub async fn get_or_create(&self, target: &str) -> Result<TcpStream> {
        // 尝试从池中获取可用连接
        {
            let mut pools = self.pools.write().await;
            if let Some(pool) = pools.get_mut(target) {
                while let Some(pooled) = pool.pop() {
                    // 检查连接是否过期
                    if pooled.created_at.elapsed() < self.max_idle_time {
                        // 验证连接是否仍然有效（简单的健康检查）
                        if is_connection_alive(&pooled.stream).await {
                            return Ok(pooled.stream);
                        }
                        // 连接已断开，继续尝试下一个或创建新连接
                    }
                    // 连接已过期或已断开，丢弃
                }
            }
        }

        // 创建新连接
        let stream = TcpStream::connect(target)
            .await
            .context(format!("Failed to connect to {}", target))?;

        stream.set_nodelay(true)?;
        Ok(stream)
    }

    /// 将连接返还到池
    pub async fn return_connection(&self, target: String, stream: TcpStream) {
        let mut pools = self.pools.write().await;
        let pool = pools.entry(target).or_insert_with(Vec::new);

        if pool.len() < self.max_pool_size {
            pool.push(PooledConnection {
                stream,
                created_at: std::time::Instant::now(),
            });
        }
        // 如果池已满，连接被丢弃
    }

    /// 清理过期连接
    pub async fn cleanup_expired(&self) {
        let mut pools = self.pools.write().await;
        for pool in pools.values_mut() {
            pool.retain(|p| p.created_at.elapsed() < self.max_idle_time);
        }
    }
}

// ============= 优化 6: SOCKS5 认证支持 =============

/// SOCKS5 认证配置
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct Socks5Auth {
    pub username: String,
    pub password: String,
}

/// SOCKS5 认证处理
#[allow(dead_code)]
async fn handle_socks5_auth(stream: &mut TcpStream, auth: Option<&Socks5Auth>) -> Result<()> {
    // 1. 服务器读取客户端支持的认证方法
    let mut buffer = [0u8; 2];
    stream.read_exact(&mut buffer).await?;

    let nmethods = buffer[1] as usize;
    let mut methods = vec![0u8; nmethods];
    stream.read_exact(&mut methods).await?;

    // 2. 选择认证方法
    let selected_method = if auth.is_some() && methods.contains(&0x02) {
        // 使用用户名/密码认证
        0x02u8
    } else if methods.contains(&0x00) {
        // 不需要认证
        0x00u8
    } else {
        // 不支持的认证方法
        stream.write_all(&[0x05, 0xFF]).await?;
        anyhow::bail!("No supported authentication method");
    };

    // 3. 发送选定的认证方法
    stream.write_all(&[0x05, selected_method]).await?;
    stream.flush().await?;

    // 4. 如果需要认证，进行用户名/密码验证
    if selected_method == 0x02 {
        if let Some(auth_config) = auth {
            // 读取认证请求
            let mut auth_buf = [0u8; 2];
            stream.read_exact(&mut auth_buf).await?;

            let username_len = auth_buf[1] as usize;
            let mut username = vec![0u8; username_len];
            stream.read_exact(&mut username).await?;

            let mut password_len = [0u8; 1];
            stream.read_exact(&mut password_len).await?;
            let password_len = password_len[0] as usize;
            let mut password = vec![0u8; password_len];
            stream.read_exact(&mut password).await?;

            let username_str = String::from_utf8(username)?;
            let password_str = String::from_utf8(password)?;

            if username_str == auth_config.username && password_str == auth_config.password {
                // 认证成功
                stream.write_all(&[0x01, 0x00]).await?;
            } else {
                // 认证失败
                stream.write_all(&[0x01, 0x01]).await?;
                anyhow::bail!("Authentication failed");
            }
            stream.flush().await?;
        }
    }

    Ok(())
}

// ============= 优化 7: 错误恢复机制 =============

/// 重试配置
#[allow(dead_code)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub initial_backoff: Duration,
    pub max_backoff: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(10),
        }
    }
}

/// 带指数退避的重试执行
#[allow(dead_code)]
pub async fn retry_with_backoff<F, Fut, T>(mut f: F, config: &RetryConfig) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: futures::Future<Output = Result<T>>,
{
    let mut attempt = 0;
    let mut backoff = config.initial_backoff;

    loop {
        match f().await {
            Ok(result) => return Ok(result),
            Err(e) => {
                attempt += 1;
                if attempt >= config.max_retries {
                    return Err(e).context(format!("Failed after {} retries", config.max_retries));
                }

                warn!(
                    "Attempt {} failed: {}. Retrying in {:?}...",
                    attempt, e, backoff
                );

                sleep(backoff).await;

                // 指数退避：每次翻倍，但不超过 max_backoff
                backoff = std::cmp::min(backoff * 2, config.max_backoff);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connection_pool() {
        let _pool = ConnectionPool::new(5, Duration::from_secs(60));
        // 测试池的基本功能
        // 注意：这里需要实际的连接才能完全测试
    }

    #[test]
    fn test_retry_config() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.initial_backoff, Duration::from_millis(100));
    }
}
