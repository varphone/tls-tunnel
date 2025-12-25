use crate::transport::TransportType;
use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 代理类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ProxyType {
    /// 原始 TCP 连接（不复用）
    #[default]
    Tcp,
    /// HTTP/1.1（支持长连接复用）
    #[serde(rename = "http/1.1")]
    Http11,
    /// HTTP/2.0（单连接多路复用）
    #[serde(rename = "http/2.0")]
    Http2,
    /// HTTP 代理（用于 forwarder）
    #[serde(rename = "http")]
    HttpProxy,
    /// SOCKS5 代理（用于 forwarder）
    #[serde(rename = "socks5")]
    Socks5Proxy,
}

impl ProxyType {
    /// 是否应该复用连接
    pub fn should_reuse_connections(self) -> bool {
        match self {
            ProxyType::Tcp => false,
            ProxyType::Http11 => true,
            ProxyType::Http2 => true,
            ProxyType::HttpProxy | ProxyType::Socks5Proxy => false,
        }
    }

    /// 是否需要单一长连接多路复用
    pub fn is_multiplexed(self) -> bool {
        matches!(self, ProxyType::Http2)
    }
}

/// 代理配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyConfig {
    /// 代理名称
    pub name: String,
    /// 代理类型
    #[serde(default)]
    pub proxy_type: ProxyType,
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

fn default_bind_addr() -> String {
    "127.0.0.1".to_string()
}

/// Visitor 配置（客户端主动访问其他客户端的服务）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VisitorConfig {
    /// Visitor 名称（对应目标 proxy 的 name）
    pub name: String,
    /// 代理类型
    #[serde(default)]
    pub proxy_type: ProxyType,
    /// 客户端本地绑定地址（默认 127.0.0.1）
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,
    /// 客户端本地绑定端口（本地应用连接此端口）
    pub bind_port: u16,
    /// 目标 proxy 的 publish_port（用于精确匹配，当有多个同名 proxy 时）
    pub publish_port: u16,
}

/// Forwarder 配置（客户端转发到外部网络）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForwarderConfig {
    /// Forwarder 名称
    pub name: String,
    /// 代理类型（http 或 socks5）
    pub proxy_type: ProxyType,
    /// 客户端本地绑定地址（默认 127.0.0.1）
    #[serde(default = "default_bind_addr")]
    pub bind_addr: String,
    /// 客户端本地绑定端口（本地应用连接此端口）
    pub bind_port: u16,
    /// 路由策略（可选）
    #[serde(default)]
    pub routing: Option<RoutingConfig>,
}

/// 路由策略配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingConfig {
    /// GeoIP 数据库路径（.mmdb 文件）
    pub geoip_db: Option<String>,
    /// 直连国家列表（ISO 3166-1 alpha-2 代码，如 "CN", "US"）
    #[serde(default)]
    pub direct_countries: Vec<String>,
    /// 通过代理的国家列表（为空表示其他所有国家）
    #[serde(default)]
    pub proxy_countries: Vec<String>,
    /// 默认策略：direct（直连）或 proxy（代理），默认 proxy
    #[serde(default = "default_routing_strategy")]
    pub default_strategy: RoutingStrategy,
}

/// 路由策略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoutingStrategy {
    /// 直接连接
    Direct,
    /// 通过代理
    Proxy,
}

fn default_routing_strategy() -> RoutingStrategy {
    RoutingStrategy::Proxy
}

/// 服务器端配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// 服务器监听地址
    pub bind_addr: String,
    /// 服务器监听端口
    pub bind_port: u16,
    /// 传输类型（tls, http2, wss）
    #[serde(default)]
    pub transport: TransportType,
    /// 是否运行在反向代理后（如 Nginx）
    /// 当为 true 时，HTTP/2 和 WebSocket 将使用非 TLS 模式
    #[serde(default)]
    pub behind_proxy: bool,
    /// TLS 证书路径
    #[serde(default)]
    pub cert_path: Option<PathBuf>,
    /// TLS 私钥路径
    #[serde(default)]
    pub key_path: Option<PathBuf>,
    /// 认证密钥（用于客户端认证）
    pub auth_key: String,
    /// 统计信息 HTTP 服务器端口（可选）
    #[serde(default)]
    pub stats_port: Option<u16>,
    /// 统计信息服务器绑定地址（可选，默认使用 bind_addr）
    #[serde(default)]
    pub stats_addr: Option<String>,
    /// 是否允许 forward proxy 功能（默认 false）
    #[serde(default)]
    pub allow_forward: bool,
}

/// 客户端配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    /// 服务器地址
    pub server_addr: String,
    /// 服务器端口
    pub server_port: u16,
    /// 服务器路径（用于反向代理子目录，默认为 "/"）
    #[serde(default = "default_server_path")]
    pub server_path: String,
    /// 传输类型（tls, http2, wss）
    #[serde(default)]
    pub transport: TransportType,
    /// 是否跳过证书验证（仅用于测试）
    #[serde(default)]
    pub skip_verify: bool,
    /// CA 证书路径（可选）
    pub ca_cert_path: Option<PathBuf>,
    /// 认证密钥（用于服务器认证）
    pub auth_key: String,
}

fn default_server_path() -> String {
    "/".to_string()
}

/// 客户端完整配置（包含代理列表）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientFullConfig {
    pub client: ClientConfig,
    /// 代理配置列表（提供给外部访问的服务）
    #[serde(default)]
    pub proxies: Vec<ProxyConfig>,
    /// Visitor 配置列表（访问服务器端的服务）
    #[serde(default)]
    pub visitors: Vec<VisitorConfig>,
    /// Forwarder 配置列表（转发到外部网络）
    #[serde(default)]
    pub forwarders: Vec<ForwarderConfig>,
}

impl ServerConfig {
    /// 确保证书路径配置成对出现或同时缺省
    pub fn validate(&self) -> anyhow::Result<()> {
        // 验证绑定地址不为空
        if self.bind_addr.trim().is_empty() {
            bail!("bind_addr cannot be empty");
        }

        // 验证统计服务器地址（如果配置了）
        if let Some(ref addr) = self.stats_addr {
            if addr.trim().is_empty() {
                bail!("stats_addr cannot be empty string, either set a valid address or omit it to use bind_addr");
            }
        }

        // 验证证书配置
        match (&self.cert_path, &self.key_path) {
            (Some(_), Some(_)) | (None, None) => {}
            _ => bail!("cert_path and key_path must both be set, or both omitted to auto-generate"),
        }

        // 当在反向代理后运行时，只有 HTTP/2 和 WebSocket 支持
        if self.behind_proxy && self.transport == TransportType::Tls {
            bail!("TLS transport cannot run behind a proxy. Use http2 or wss transport instead.");
        }

        // 当在反向代理后运行时，不需要证书
        if self.behind_proxy && (self.cert_path.is_some() || self.key_path.is_some()) {
            bail!("Certificates are not needed when running behind a proxy (TLS is terminated by the proxy).");
        }

        Ok(())
    }
}

impl ClientFullConfig {
    /// 验证配置的有效性
    pub fn validate(&self) -> anyhow::Result<()> {
        use std::collections::HashSet;

        if self.proxies.is_empty() && self.visitors.is_empty() && self.forwarders.is_empty() {
            anyhow::bail!("No proxy, visitor, or forwarder configurations defined");
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

        // 验证 visitors 配置
        let mut seen_visitor_names = HashSet::new();
        let mut seen_visitor_binds = HashSet::new();

        for visitor in &self.visitors {
            // 检查 name 唯一性
            if !seen_visitor_names.insert(&visitor.name) {
                anyhow::bail!(
                    "Duplicate visitor name '{}': each visitor must have a unique name",
                    visitor.name
                );
            }

            // 检查 (bind_addr, bind_port) 唯一性
            if !seen_visitor_binds.insert((visitor.bind_addr.clone(), visitor.bind_port)) {
                anyhow::bail!(
                    "Duplicate visitor binding {}:{}: each visitor must use a different local bind address/port",
                    visitor.bind_addr,
                    visitor.bind_port
                );
            }

            // 验证端口范围
            if visitor.bind_port == 0 {
                anyhow::bail!("Visitor '{}': bind_port cannot be 0", visitor.name);
            }

            if visitor.publish_port == 0 {
                anyhow::bail!("Visitor '{}': publish_port cannot be 0", visitor.name);
            }

            // 验证名称不为空
            if visitor.name.trim().is_empty() {
                anyhow::bail!("Visitor name cannot be empty");
            }

            if visitor.bind_addr.trim().is_empty() {
                anyhow::bail!("Visitor '{}': bind_addr cannot be empty", visitor.name);
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
