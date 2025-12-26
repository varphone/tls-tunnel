use super::registry::ProxyRegistry;
use crate::config::ServerConfig;
use anyhow::{Context, Result};
use std::net::{IpAddr, ToSocketAddrs};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::Duration;
use tokio_util::compat::FuturesAsyncReadCompatExt;
use tracing::{error, info, warn};

/// 服务器端读取客户端请求的超时时间（防止慢速攻击）
const CLIENT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// 检查目标地址是否为本地地址（禁止访问）
fn is_local_address(target_addr: &str) -> bool {
    // 解析地址
    let addr_str = if target_addr.contains(':') {
        target_addr.to_string()
    } else {
        format!("{}:80", target_addr) // 添加默认端口以便解析
    };

    // 尝试解析域名/IP
    match addr_str.to_socket_addrs() {
        Ok(addrs) => {
            for addr in addrs {
                let ip = addr.ip();

                // 检查是否为本地地址
                if ip.is_loopback() {
                    warn!("Blocked attempt to access loopback address: {}", ip);
                    return true;
                }

                // 检查是否为私有地址（内网地址）
                match ip {
                    IpAddr::V4(ipv4) => {
                        // 127.0.0.0/8 - Loopback (已被 is_loopback 覆盖)
                        // 10.0.0.0/8 - Private
                        // 172.16.0.0/12 - Private
                        // 192.168.0.0/16 - Private
                        // 169.254.0.0/16 - Link-local
                        // 0.0.0.0/8 - Current network
                        let octets = ipv4.octets();
                        if octets[0] == 10
                            || (octets[0] == 172 && (octets[1] >= 16 && octets[1] <= 31))
                            || (octets[0] == 192 && octets[1] == 168)
                            || (octets[0] == 169 && octets[1] == 254)
                            || octets[0] == 0
                        {
                            warn!("Blocked attempt to access private IPv4 address: {}", ip);
                            return true;
                        }
                    }
                    IpAddr::V6(ipv6) => {
                        // ::1 - Loopback (已被 is_loopback 覆盖)
                        // fc00::/7 - Unique local address (ULA)
                        // fe80::/10 - Link-local
                        if ipv6.is_unique_local() || ipv6.is_unicast_link_local() {
                            warn!("Blocked attempt to access private IPv6 address: {}", ip);
                            return true;
                        }
                    }
                }
            }
            false
        }
        Err(e) => {
            // 无法解析地址，出于安全考虑禁止访问
            warn!(
                "Failed to resolve target address '{}': {}. Blocking for security.",
                target_addr, e
            );
            true
        }
    }
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

/// 处理来自客户端的 visitor stream
/// 客户端发送目标 proxy 名称，服务器通过 yamux 连接到客户端的本地服务并转发数据
pub async fn handle_visitor_stream(
    stream: yamux::Stream,
    proxy_registry: ProxyRegistry,
    server_config: &ServerConfig,
) -> Result<()> {
    use tokio::time::timeout;

    let mut visitor_stream = stream.compat();

    // 使用超时包装读取操作（防止慢速攻击）
    let (proxy_name, publish_port) = timeout(CLIENT_REQUEST_TIMEOUT, async {
        // 读取目标 proxy 名称
        let mut name_len_buf = [0u8; 2];
        visitor_stream
            .read_exact(&mut name_len_buf)
            .await
            .context("Failed to read proxy name length")?;
        let name_len = u16::from_be_bytes(name_len_buf) as usize;

        // 对于 forward 请求，允许更长的名称（包含完整域名+端口）
        // 正常代理名称限制在64字节，forward 请求限制在255字节
        if name_len == 0 || name_len > 255 {
            let error_msg = "Invalid proxy name length (must be 1-255 bytes)";
            error!("{}", error_msg);
            visitor_stream.write_all(&[0]).await.ok();
            send_error_message(&mut visitor_stream, error_msg)
                .await
                .ok();
            return Err(anyhow::anyhow!(error_msg));
        }

        let mut name_buf = vec![0u8; name_len];
        visitor_stream
            .read_exact(&mut name_buf)
            .await
            .context("Failed to read proxy name")?;

        let proxy_name = String::from_utf8(name_buf).context("Invalid UTF-8 in proxy name")?;

        // 读取目标 publish_port
        let mut port_buf = [0u8; 2];
        visitor_stream
            .read_exact(&mut port_buf)
            .await
            .context("Failed to read publish port")?;
        let publish_port = u16::from_be_bytes(port_buf);

        Ok::<(String, u16), anyhow::Error>((proxy_name, publish_port))
    })
    .await
    .map_err(|_| {
        error!(
            "Visitor stream request timeout after {:?}",
            CLIENT_REQUEST_TIMEOUT
        );
        anyhow::anyhow!("Client request timeout")
    })??;

    // 检测是否为 @forward 请求
    if let Some(target_addr) = proxy_name.strip_prefix("@forward:") {
        info!(
            "Visitor stream requesting forward to external target: '{}'",
            target_addr
        );
        return handle_forward_request(visitor_stream, target_addr, server_config).await;
    }

    info!(
        "Visitor stream requesting proxy: '{}' with publish_port {}",
        proxy_name, publish_port
    );

    // 从注册表查找对应的 proxy（按 name 和 publish_port 匹配）
    let proxy_registration = {
        let registry = proxy_registry.read().await;
        registry.get(&(proxy_name.clone(), publish_port)).cloned()
    };

    let (stream_tx, local_port) = match proxy_registration {
        Some(reg) => (reg.stream_tx, reg.proxy_info.local_port),
        None => {
            let error_msg = format!(
                "Proxy '{}' with publish_port {} not found or client not connected",
                proxy_name, publish_port
            );
            error!("{}", error_msg);
            visitor_stream.write_all(&[0]).await.ok();
            send_error_message(&mut visitor_stream, &error_msg)
                .await
                .ok();
            return Err(anyhow::anyhow!(error_msg));
        }
    };

    // 发送确认给visitor客户端
    visitor_stream
        .write_all(&[1])
        .await
        .context("Failed to send confirmation")?;
    visitor_stream.flush().await?;

    info!(
        "Visitor stream confirmed for proxy '{}', requesting connection to target client local port {}",
        proxy_name, local_port
    );

    // 请求目标客户端创建到其本地服务的 yamux stream
    let (response_tx, mut response_rx) = mpsc::channel::<yamux::Stream>(1);

    stream_tx
        .send((response_tx, local_port, proxy_name.clone()))
        .await
        .context("Failed to request yamux stream from target client")?;

    // 等待目标客户端返回 yamux stream
    let mut client_stream = response_rx
        .recv()
        .await
        .ok_or_else(|| anyhow::anyhow!("Failed to receive yamux stream from target client"))?;

    info!(
        "Got yamux stream to target client local port {}, starting bidirectional data transfer",
        local_port
    );

    // 向客户端B的 stream 写入 publish_port（客户端需要通过此端口找到对应的 proxy 配置）
    use futures::io::AsyncWriteExt as FuturesAsyncWriteExt;
    client_stream.write_all(&publish_port.to_be_bytes()).await?;
    client_stream.flush().await?;

    info!("Sent publish_port {} to target client", publish_port);

    let client_stream_tokio = client_stream.compat();

    // 双向转发数据：visitor客户端 ↔ 服务器 ↔ proxy客户端
    let (mut visitor_read, mut visitor_write) = tokio::io::split(visitor_stream);
    let (mut client_read, mut client_write) = tokio::io::split(client_stream_tokio);

    let visitor_to_client = async {
        tokio::io::copy(&mut visitor_read, &mut client_write).await?;
        client_write.shutdown().await?;
        Ok::<_, std::io::Error>(())
    };

    let client_to_visitor = async {
        tokio::io::copy(&mut client_read, &mut visitor_write).await?;
        visitor_write.shutdown().await?;
        Ok::<_, std::io::Error>(())
    };

    tokio::select! {
        result = visitor_to_client => {
            if let Err(e) = result {
                warn!("Visitor '{}': Visitor to target client copy error: {}", proxy_name, e);
            }
        }
        result = client_to_visitor => {
            if let Err(e) = result {
                warn!("Visitor '{}': Target client to visitor copy error: {}", proxy_name, e);
            }
        }
    }

    info!("Visitor stream for proxy '{}' closed", proxy_name);
    Ok(())
}

/// 处理 forward 请求（连接到外部目标）
async fn handle_forward_request<T>(
    mut visitor_stream: T,
    target_addr: &str,
    server_config: &ServerConfig,
) -> Result<()>
where
    T: AsyncReadExt + AsyncWriteExt + Unpin,
{
    // 检查服务器是否允许 forward 功能
    if !server_config.allow_forward {
        let error_msg = "Forward feature is not enabled on server";
        error!("{}", error_msg);
        visitor_stream.write_all(&[0]).await.ok();
        send_error_message(&mut visitor_stream, error_msg)
            .await
            .ok();
        return Err(anyhow::anyhow!(error_msg));
    }

    // 安全检查：禁止访问本地地址和内网地址
    if is_local_address(target_addr) {
        let error_msg = format!(
            "Access denied: cannot forward to local or private address '{}'",
            target_addr
        );
        error!("{}", error_msg);
        visitor_stream.write_all(&[0]).await.ok();
        send_error_message(&mut visitor_stream, &error_msg)
            .await
            .ok();
        return Err(anyhow::anyhow!(error_msg));
    }

    info!("Attempting to connect to external target: {}", target_addr);

    // 连接到外部目标
    let external_stream = match TcpStream::connect(target_addr).await {
        Ok(stream) => stream,
        Err(e) => {
            let error_msg = format!("Failed to connect to {}: {}", target_addr, e);
            error!("{}", error_msg);
            visitor_stream.write_all(&[0]).await.ok();
            send_error_message(&mut visitor_stream, &error_msg)
                .await
                .ok();
            return Err(anyhow::anyhow!(error_msg));
        }
    };

    info!("Successfully connected to external target: {}", target_addr);

    // 发送确认给 visitor 客户端
    visitor_stream
        .write_all(&[1])
        .await
        .context("Failed to send confirmation")?;
    visitor_stream.flush().await?;

    info!(
        "Forward connection confirmed, starting bidirectional data transfer with {}",
        target_addr
    );

    // 双向转发数据：visitor客户端 ↔ 服务器 ↔ 外部目标
    let (mut visitor_read, mut visitor_write) = tokio::io::split(visitor_stream);
    let (mut external_read, mut external_write) = tokio::io::split(external_stream);

    let visitor_to_external = async {
        tokio::io::copy(&mut visitor_read, &mut external_write).await?;
        external_write.shutdown().await?;
        Ok::<_, std::io::Error>(())
    };

    let external_to_visitor = async {
        tokio::io::copy(&mut external_read, &mut visitor_write).await?;
        visitor_write.shutdown().await?;
        Ok::<_, std::io::Error>(())
    };

    tokio::select! {
        result = visitor_to_external => {
            if let Err(e) = result {
                warn!("Forward '{}': Visitor to external copy error: {}", target_addr, e);
            }
        }
        result = external_to_visitor => {
            if let Err(e) = result {
                warn!("Forward '{}': External to visitor copy error: {}", target_addr, e);
            }
        }
    }

    info!("Forward connection to '{}' closed", target_addr);
    Ok(())
}
