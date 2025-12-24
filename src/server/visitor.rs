use super::registry::ProxyRegistry;
use anyhow::{Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio_util::compat::FuturesAsyncReadCompatExt;
use tracing::{error, info, warn};

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
pub async fn handle_visitor_stream(stream: yamux::Stream, proxy_registry: ProxyRegistry) -> Result<()> {
    let mut visitor_stream = stream.compat();

    // 读取目标 proxy 名称
    let mut name_len_buf = [0u8; 2];
    visitor_stream
        .read_exact(&mut name_len_buf)
        .await
        .context("Failed to read proxy name length")?;
    let name_len = u16::from_be_bytes(name_len_buf) as usize;

    if name_len == 0 || name_len > 256 {
        let error_msg = "Invalid proxy name length";
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
    let client_stream = response_rx
        .recv()
        .await
        .ok_or_else(|| anyhow::anyhow!("Failed to receive yamux stream from target client"))?;

    info!(
        "Got yamux stream to target client local port {}, starting bidirectional data transfer",
        local_port
    );

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
