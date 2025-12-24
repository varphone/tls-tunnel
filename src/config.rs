use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 代理配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// 代理名称
    pub name: String,
    /// 服务器发布地址（绑定地址，默认 0.0.0.0）
    #[serde(default = "default_publish_addr")]
    pub publish_addr: String,
    /// 服务器发布端口（外部访问该端口）
    pub publish_port: u16,
    /// 客户端本地服务端口（转发到该端口）
    pub local_port: u16,
}

fn default_publish_addr() -> String {
    "0.0.0.0".to_string()
}

/// 服务器端配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// 服务器监听地址
    pub bind_addr: String,
    /// 服务器监听端口
    pub bind_port: u16,
    /// TLS 证书路径
    #[serde(default)]
    pub cert_path: Option<PathBuf>,
    /// TLS 私钥路径
    #[serde(default)]
    pub key_path: Option<PathBuf>,
    /// 认证密钥（用于客户端认证）
    pub auth_key: String,
}

/// 客户端配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    /// 服务器地址
    pub server_addr: String,
    /// 服务器 TLS 端口
    pub server_port: u16,
    /// 是否跳过证书验证（仅用于测试）
    #[serde(default)]
    pub skip_verify: bool,
    /// CA 证书路径（可选）
    pub ca_cert_path: Option<PathBuf>,
    /// 认证密钥（用于服务器认证）
    pub auth_key: String,
}

/// 客户端完整配置（包含代理列表）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientFullConfig {
    pub client: ClientConfig,
    /// 代理配置列表
    #[serde(default)]
    pub proxies: Vec<ProxyConfig>,
}

impl ServerConfig {
    /// 确保证书路径配置成对出现或同时缺省
    pub fn validate(&self) -> anyhow::Result<()> {
        match (&self.cert_path, &self.key_path) {
            (Some(_), Some(_)) | (None, None) => Ok(()),
            _ => bail!("cert_path and key_path must both be set, or both omitted to auto-generate"),
        }
    }
}

impl ClientFullConfig {
    /// 验证配置的有效性
    pub fn validate(&self) -> anyhow::Result<()> {
        use std::collections::HashSet;

        if self.proxies.is_empty() {
            anyhow::bail!("No proxy configurations defined");
        }

        let mut seen_names = HashSet::new();
        let mut seen_bind = HashSet::new();
        let mut seen_local_ports = HashSet::new();

        for proxy in &self.proxies {
            // 检查 name 唯一性
            if !seen_names.insert(&proxy.name) {
                anyhow::bail!(
                    "Duplicate proxy name '{}': each proxy must have a unique name",
                    proxy.name
                );
            }

            // 检查 (publish_addr, publish_port) 唯一性（服务器端绑定地址+端口）
            if !seen_bind.insert((proxy.publish_addr.clone(), proxy.publish_port)) {
                anyhow::bail!(
                    "Duplicate publish binding {}:{}: each proxy must use a different server bind address/port",
                    proxy.publish_addr,
                    proxy.publish_port
                );
            }

            // 检查 local_port 唯一性（客户端本地服务端口）
            if !seen_local_ports.insert(proxy.local_port) {
                anyhow::bail!(
                    "Duplicate local_port {}: each proxy must connect to a different local service",
                    proxy.local_port
                );
            }

            // 验证端口范围
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

            if proxy.publish_addr.trim().is_empty() {
                anyhow::bail!("Proxy '{}': publish_addr cannot be empty", proxy.name);
            }
        }

        Ok(())
    }
}

/// 应用配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "lowercase")]
pub enum AppConfig {
    Server(ServerConfig),
    Client(ClientFullConfig),
}

impl AppConfig {
    /// 从文件加载配置（自动检测类型）
    #[allow(dead_code)]
    pub fn from_file(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: AppConfig = toml::from_str(&content)?;

        // 验证客户端配置
        if let AppConfig::Client(ref client_config) = config {
            client_config
                .validate()
                .context("Configuration validation failed")?;
        }

        Ok(config)
    }

    /// 从文件加载服务器配置
    pub fn load_server_config(path: &str) -> anyhow::Result<ServerConfig> {
        #[derive(Deserialize)]
        struct ServerConfigWrapper {
            server: ServerConfig,
        }

        let content = std::fs::read_to_string(path)?;
        let wrapper: ServerConfigWrapper =
            toml::from_str(&content).context("Failed to parse server configuration")?;
        wrapper
            .server
            .validate()
            .context("Server configuration validation failed")?;
        Ok(wrapper.server)
    }

    /// 从文件加载客户端配置
    pub fn load_client_config(path: &str) -> anyhow::Result<ClientFullConfig> {
        let content = std::fs::read_to_string(path)?;
        let config: ClientFullConfig =
            toml::from_str(&content).context("Failed to parse client configuration")?;
        config
            .validate()
            .context("Configuration validation failed")?;
        Ok(config)
    }
}
