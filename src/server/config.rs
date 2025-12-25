use super::registry::{ProxyInfo, SUPPORTED_PROTOCOL_VERSION};
use anyhow::{Context, Result};
use tokio::io::AsyncReadExt;
use tracing::info;

use super::registry::ClientConfigs;
use super::registry::VisitorInfo;

/// 验证代理配置的有效性
pub fn validate_proxy_configs(proxies: &[ProxyInfo], server_bind_port: u16) -> Result<()> {
    use std::collections::HashSet;

    // 允许空的 proxies（客户端可能只有 forwarders 配置）
    if proxies.is_empty() {
        return Ok(());
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

/// 读取客户端配置
pub async fn read_client_configs<S>(tls_stream: &mut S) -> Result<ClientConfigs>
where
    S: AsyncReadExt + Unpin,
{
    // 读取长度前缀的 JSON
    let mut len_buf = [0u8; 4];
    tls_stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 {
        anyhow::bail!("Client config message length cannot be 0");
    }
    let mut buf = vec![0u8; len];
    tls_stream.read_exact(&mut buf).await?;

    #[derive(serde::Deserialize)]
    struct ClientConfigMessage {
        version: u8,
        proxies: Vec<crate::config::ProxyConfig>,
        #[serde(default)]
        visitors: Vec<crate::config::VisitorConfig>,
    }

    let msg: ClientConfigMessage =
        serde_json::from_slice(&buf).context("Failed to parse client config message JSON")?;
    if msg.version != SUPPORTED_PROTOCOL_VERSION {
        anyhow::bail!("Unsupported protocol version {}", msg.version);
    }

    // 注意：客户端可能只有 forwarders 配置（不需要发送给服务端），
    // 此时 proxies 和 visitors 都为空是正常的
    // if msg.proxies.is_empty() && msg.visitors.is_empty() {
    //     anyhow::bail!("No proxy or visitor configurations provided");
    // }

    let mut proxies = Vec::with_capacity(msg.proxies.len());
    for p in msg.proxies {
        proxies.push(ProxyInfo {
            name: p.name,
            proxy_type: p.proxy_type,
            publish_addr: p.publish_addr,
            publish_port: p.publish_port,
            local_port: p.local_port,
        });
    }

    let mut visitors = Vec::with_capacity(msg.visitors.len());
    for v in msg.visitors {
        visitors.push(VisitorInfo {
            name: v.name,
            bind_addr: v.bind_addr,
            bind_port: v.bind_port,
            publish_port: v.publish_port,
        });
    }

    info!(
        "Client (json v{}) has {} proxy and {} visitor configurations",
        msg.version,
        proxies.len(),
        visitors.len()
    );

    Ok(ClientConfigs { proxies, visitors })
}
