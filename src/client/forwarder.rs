use crate::config::{ForwarderConfig, ProxyType};
use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{sleep, Duration};
use tokio_util::compat::FuturesAsyncReadCompatExt;
use tracing::{error, info, warn};

use super::config::read_error_message;

/// 运行 forwarder 监听器
/// 在客户端本地监听端口，接受连接后解析目标地址并通过 yamux 转发到服务器
pub async fn run_forwarder_listener(
    forwarder: ForwarderConfig,
    stream_tx: tokio::sync::mpsc::Sender<tokio::sync::oneshot::Sender<Result<yamux::Stream>>>,
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

    loop {
        match listener.accept().await {
            Ok((local_stream, peer_addr)) => {
                info!(
                    "Forwarder '{}': Accepted connection from {}",
                    forwarder.name, peer_addr
                );

                let forwarder_clone = forwarder.clone();
                let stream_tx_clone = stream_tx.clone();

                tokio::spawn(async move {
                    if let Err(e) =
                        handle_forwarder_connection(local_stream, &forwarder_clone, stream_tx_clone)
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
/// 根据协议类型解析目标地址，然后通过 yamux stream 转发到服务器
async fn handle_forwarder_connection(
    mut local_stream: TcpStream,
    forwarder: &ForwarderConfig,
    stream_tx: tokio::sync::mpsc::Sender<tokio::sync::oneshot::Sender<Result<yamux::Stream>>>,
) -> Result<()> {
    // 1. 根据 proxy_type 解析目标地址
    let target = match forwarder.proxy_type {
        ProxyType::HttpProxy => parse_http_connect(&mut local_stream).await?,
        ProxyType::Socks5Proxy => parse_socks5(&mut local_stream).await?,
        _ => anyhow::bail!(
            "Invalid proxy type for forwarder: {:?}",
            forwarder.proxy_type
        ),
    };

    info!(
        "Forwarder '{}': Forwarding to target: {}",
        forwarder.name, target
    );

    // 2. 请求创建新的 yamux stream
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
            "Forwarder '{}': Server rejected connection: {}",
            forwarder.name, error_msg
        );

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

    let client_to_server = async {
        tokio::io::copy(&mut local_read, &mut server_write).await?;
        server_write.shutdown().await?;
        Ok::<_, std::io::Error>(())
    };

    let server_to_client = async {
        tokio::io::copy(&mut server_read, &mut local_write).await?;
        local_write.shutdown().await?;
        Ok::<_, std::io::Error>(())
    };

    tokio::select! {
        result = client_to_server => {
            if let Err(e) = result {
                warn!("Forwarder '{}': Client to server copy error: {}", forwarder.name, e);
            }
        }
        result = server_to_client => {
            if let Err(e) = result {
                warn!("Forwarder '{}': Server to client copy error: {}", forwarder.name, e);
            }
        }
    }

    info!("Forwarder '{}': Connection closed", forwarder.name);
    Ok(())
}

/// 解析 HTTP CONNECT 请求
/// 格式：CONNECT example.com:443 HTTP/1.1
async fn parse_http_connect(stream: &mut TcpStream) -> Result<String> {
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

    Ok(target)
}

/// 格式化代理类型为可读字符串
fn format_proxy_type(proxy_type: ProxyType) -> &'static str {
    match proxy_type {
        ProxyType::HttpProxy => "HTTP proxy",
        ProxyType::Socks5Proxy => "SOCKS5 proxy",
        _ => "Unknown",
    }
}
