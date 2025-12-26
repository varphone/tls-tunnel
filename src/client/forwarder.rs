use crate::config::{ForwarderConfig, ProxyType};
use anyhow::{Context, Result};
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Semaphore, RwLock};
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

/// 数据复制缓冲区大小
const COPY_BUFFER_SIZE: usize = 8192;

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
    pub async fn failed_targets_count(&self) -> usize {
        let targets = self.targets.read().await;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        targets
            .iter()
            .filter(|(_, failed)| {
                now < failed.blacklist_time + FAILED_TARGET_TIMEOUT.as_secs()
            })
            .count()
    }
}

impl Default for FailedTargetManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 实时统计的数据复制函数
/// 相比 tokio::io::copy，这个函数会在每次复制数据后立即更新统计信息
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
    let mut buf = vec![0u8; COPY_BUFFER_SIZE];
    let mut total_copied = 0u64;

    loop {
        let n = reader.read(&mut buf).await?;
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

    // 创建快速失败管理器并启动清理任务
    let failed_target_manager = FailedTargetManager::new();
    failed_target_manager.clone().start_cleanup_task();
    info!(
        "Forwarder '{}': Fast-fail manager initialized (threshold: {}, timeout: {:?})",
        forwarder.name, FAILED_TARGET_THRESHOLD, FAILED_TARGET_TIMEOUT
    );

    loop {
        match listener.accept().await {
            Ok((local_stream, peer_addr)) => {
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
    let target = match forwarder.proxy_type {
        ProxyType::HttpProxy => parse_http_connect(&mut local_stream).await?,
        ProxyType::Socks5Proxy => parse_socks5(&mut local_stream).await?,
        _ => anyhow::bail!(
            "Invalid proxy type for forwarder: {:?}",
            forwarder.proxy_type
        ),
    };

    // 检查目标是否在黑名单中（快速失败）
    if failed_target_manager.is_blacklisted(&target).await {
        warn!(
            "Forwarder '{}': Target '{}' is blacklisted due to previous failures, rejecting immediately",
            forwarder.name, target
        );
        let error_response = format!(
            "HTTP/1.1 503 Service Unavailable\r\n\
             Content-Type: text/plain\r\n\
             Content-Length: 47\r\n\
             Connection: close\r\n\
             \r\n\
             Target is temporarily unavailable (blacklisted)"
        );
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
        warn!("Forwarder '{}': Client to server copy error: {}", forwarder.name, e);
    }
    if let Err(e) = result_s2c {
        warn!("Forwarder '{}': Server to client copy error: {}", forwarder.name, e);
    }

    info!("Forwarder '{}': Connection closed", forwarder.name);
    
    // 记录连接结束
    if let Some(ref tracker) = stats_tracker {
        tracker.connection_ended();
    }
    
    Ok(())
}

/// 解析 HTTP CONNECT 请求
/// 格式：CONNECT example.com:443 HTTP/1.1
async fn parse_http_connect(stream: &mut TcpStream) -> Result<String> {
    use tokio::time::timeout;

    // 使用超时包装整个解析过程
    let result = timeout(PROTOCOL_PARSE_TIMEOUT, async {
        let mut buffer = Vec::new();
        let mut temp = [0u8; 1];

        // 读取到第一个 \r\n\r\n
        loop {
            stream.read_exact(&mut temp).await?;
            buffer.push(temp[0]);

            if buffer.len() >= 4 && &buffer[buffer.len() - 4..] == b"\r\n\r\n" {
                break;
            }

            // 防止超长请求
            if buffer.len() > 8192 {
                anyhow::bail!("HTTP request too long");
            }
        }

        let request = String::from_utf8_lossy(&buffer);
        let first_line = request
            .lines()
            .next()
            .ok_or_else(|| anyhow::anyhow!("Empty HTTP request"))?;

        // 解析：CONNECT example.com:443 HTTP/1.1
        let parts: Vec<&str> = first_line.split_whitespace().collect();
        if parts.len() < 2 {
            anyhow::bail!("Invalid HTTP CONNECT request");
        }

        if parts[0] != "CONNECT" {
            anyhow::bail!("Only CONNECT method is supported");
        }

        let target = parts[1].to_string();

        Ok::<String, anyhow::Error>(target)
    })
    .await
    .map_err(|_| {
        anyhow::anyhow!(
            "HTTP CONNECT parsing timeout after {:?}",
            PROTOCOL_PARSE_TIMEOUT
        )
    })??;

    let target = result;

    // 验证目标地址格式
    if !target.contains(':') {
        anyhow::bail!("Invalid target address: {}", target);
    }

    // 发送 200 Connection Established 响应
    let response = b"HTTP/1.1 200 Connection Established\r\n\r\n";
    stream.write_all(response).await?;
    stream.flush().await?;

    Ok(target)
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

/// 处理直连（不通过服务器）
async fn handle_direct_connection(
    mut local_stream: TcpStream,
    target: &str,
    forwarder_name: &str,
    stats_tracker: Option<ClientStatsTracker>,
    failed_target_manager: FailedTargetManager,
) -> Result<()> {
    // 安全检查：禁止访问本地地址和内网地址（防止 SSRF 攻击）
    if is_unsafe_direct_target(target) {
        warn!(
            "Forwarder '{}': Blocked direct connection to local/private address: {}",
            forwarder_name, target
        );
        return Err(anyhow::anyhow!(
            "Direct connection to local or private addresses is not allowed for security reasons"
        ));
    }

    // 直接连接目标服务器
    let mut remote_stream = match TcpStream::connect(target).await {
        Ok(stream) => {
            info!(
                "Forwarder '{}': Successfully connected directly to {}",
                forwarder_name, target
            );
            stream
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

    // 双向转发数据
    let (mut local_read, mut local_write) = local_stream.split();
    let (mut remote_read, mut remote_write) = remote_stream.split();

    let stats_tracker_c2r = stats_tracker.clone();
    let client_to_remote = async move {
        let bytes = copy_with_stats(
            &mut local_read,
            &mut remote_write,
            stats_tracker_c2r.as_ref(),
            |tracker, n| tracker.record_bytes_sent(n),
        )
        .await?;
        remote_write.shutdown().await?;
        Ok::<_, std::io::Error>(bytes)
    };

    let stats_tracker_r2c = stats_tracker.clone();
    let remote_to_client = async move {
        let bytes = copy_with_stats(
            &mut remote_read,
            &mut local_write,
            stats_tracker_r2c.as_ref(),
            |tracker, n| tracker.record_bytes_received(n),
        )
        .await?;
        local_write.shutdown().await?;
        Ok::<_, std::io::Error>(bytes)
    };

    // 使用 tokio::join! 确保两个方向的流量都被记录
    let (result_c2r, result_r2c) = tokio::join!(client_to_remote, remote_to_client);
    
    if let Err(e) = result_c2r {
        warn!("Forwarder '{}' direct: Client to remote error: {}", forwarder_name, e);
    }
    if let Err(e) = result_r2c {
        warn!("Forwarder '{}' direct: Remote to client error: {}", forwarder_name, e);
    }

    info!(
        "Forwarder '{}': Direct connection to {} closed",
        forwarder_name, target
    );
    Ok(())
}

/// Forwarder 处理器（实现 ProxyHandler trait）
pub struct ForwarderHandler {
    config: ForwarderConfig,
    stream_tx: tokio::sync::mpsc::Sender<tokio::sync::oneshot::Sender<Result<yamux::Stream>>>,
    router: Option<Arc<GeoIpRouter>>,
    stats_tracker: Option<ClientStatsTracker>,
    status: Arc<tokio::sync::RwLock<crate::client::HandlerStatus>>,
    shutdown_tx: Arc<tokio::sync::RwLock<Option<tokio::sync::oneshot::Sender<()>>>>,
}

impl ForwarderHandler {
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
            stats_tracker,
            status: Arc::new(tokio::sync::RwLock::new(
                crate::client::HandlerStatus::Stopped,
            )),
            shutdown_tx: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }
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

        tokio::select! {
            result = run_forwarder_listener(config, stream_tx, router, stats_tracker) => {
                if let Err(e) = &result {
                    let mut s = status.write().await;
                    *s = crate::client::HandlerStatus::Failed(e.to_string());
                }
                result
            }
            _ = &mut shutdown_rx => {
                let mut s = status.write().await;
                *s = crate::client::HandlerStatus::Stopped;
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
