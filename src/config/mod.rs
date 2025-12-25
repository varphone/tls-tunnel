// 配置管理模块 - 使用模块化设计

mod builder;
mod validator;

// 重新导出 builder 和 validator
pub use builder::{ClientConfigBuilder, ClientFullConfigBuilder, ServerConfigBuilder};
pub use validator::ConfigValidator;

use crate::transport::TransportType;
use anyhow::Context;
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
    /// 直连 IP/CIDR 列表（如 "192.168.0.0/16", "10.0.0.1"）
    #[serde(default)]
    pub direct_ips: Vec<String>,
    /// 代理 IP/CIDR 列表
    #[serde(default)]
    pub proxy_ips: Vec<String>,
    /// 直连域名列表（支持通配符，如 "*.baidu.com", "example.com"）
    #[serde(default)]
    pub direct_domains: Vec<String>,
    /// 代理域名列表（支持通配符）
    #[serde(default)]
    pub proxy_domains: Vec<String>,
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
    /// 速率限制配置（可选）
    #[serde(default)]
    pub rate_limit: Option<RateLimitConfig>,
    /// 请求大小限制配置（可选）
    #[serde(default)]
    pub size_limits: Option<SizeLimitConfig>,
}

/// 速率限制配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitConfig {
    /// 每秒允许的新连接数
    pub requests_per_second: u32,
    /// 突发容量（允许短时间内的峰值连接数）
    pub burst_size: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            requests_per_second: 100,
            burst_size: 200,
        }
    }
}

/// 请求大小限制配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SizeLimitConfig {
    /// 最大请求体大小（字节）
    pub max_request_size: usize,
    /// 最大 HTTP 请求头大小（字节）
    pub max_header_size: usize,
}

impl Default for SizeLimitConfig {
    fn default() -> Self {
        Self {
            max_request_size: 1024 * 1024, // 1MB
            max_header_size: 8 * 1024,     // 8KB
        }
    }
}

impl ServerConfig {
    /// 创建 Builder
    pub fn builder() -> ServerConfigBuilder {
        ServerConfigBuilder::new()
    }

    /// 验证配置（保持向后兼容）
    pub fn validate(&self) -> anyhow::Result<()> {
        ConfigValidator::validate_server_config(self)
    }
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
    /// HTTP 统计信息服务器端口（可选）
    pub stats_port: Option<u16>,
    /// HTTP 统计信息服务器绑定地址（可选，默认为 127.0.0.1）
    pub stats_addr: Option<String>,
}

impl ClientConfig {
    /// 创建 Builder
    pub fn builder() -> ClientConfigBuilder {
        ClientConfigBuilder::new()
    }
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

impl ClientFullConfig {
    /// 创建 Builder
    pub fn builder() -> ClientFullConfigBuilder {
        ClientFullConfigBuilder::new()
    }

    /// 验证配置（保持向后兼容）
    pub fn validate(&self) -> anyhow::Result<()> {
        ConfigValidator::validate_client_full_config(self)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection_pool::PoolConfig as ConnPoolConfig;

    #[test]
    fn test_proxy_type_should_reuse_connections() {
        assert!(!ProxyType::Tcp.should_reuse_connections());
        assert!(ProxyType::Http11.should_reuse_connections());
        assert!(ProxyType::Http2.should_reuse_connections());
        assert!(!ProxyType::HttpProxy.should_reuse_connections());
        assert!(!ProxyType::Socks5Proxy.should_reuse_connections());
    }

    #[test]
    fn test_proxy_type_is_multiplexed() {
        assert!(!ProxyType::Tcp.is_multiplexed());
        assert!(!ProxyType::Http11.is_multiplexed());
        assert!(ProxyType::Http2.is_multiplexed());
        assert!(!ProxyType::HttpProxy.is_multiplexed());
        assert!(!ProxyType::Socks5Proxy.is_multiplexed());
    }

    #[test]
    fn test_proxy_type_serde() {
        // 测试序列化
        let tcp = ProxyType::Tcp;
        let json = serde_json::to_string(&tcp).unwrap();
        assert_eq!(json, "\"tcp\"");

        let http2 = ProxyType::Http2;
        let json = serde_json::to_string(&http2).unwrap();
        assert_eq!(json, "\"http/2.0\"");

        // 测试反序列化
        let tcp: ProxyType = serde_json::from_str("\"tcp\"").unwrap();
        assert_eq!(tcp, ProxyType::Tcp);

        let http11: ProxyType = serde_json::from_str("\"http/1.1\"").unwrap();
        assert_eq!(http11, ProxyType::Http11);
    }

    #[test]
    fn test_connection_pool_config_default() {
        let config = ConnPoolConfig::default();
        assert_eq!(config.min_idle, 2);
        assert_eq!(config.max_size, 10);
    }

    #[test]
    fn test_server_config_validation() {
        let mut config = ServerConfig {
            bind_addr: "0.0.0.0".to_string(),
            bind_port: 8443,
            transport: TransportType::Tls,
            behind_proxy: false,
            cert_path: Some(PathBuf::from("/path/to/cert.pem")),
            key_path: Some(PathBuf::from("/path/to/key.pem")),
            auth_key: "a".repeat(20),
            stats_port: None,
            stats_addr: None,
            allow_forward: false,
            rate_limit: None,
            size_limits: None,
        };

        // 有效配置
        assert!(config.validate().is_ok());

        // 无效：弱密钥
        config.auth_key = "weak".to_string();
        assert!(config.validate().is_err());

        // 修复密钥后继续测试
        config.auth_key = "a".repeat(20);

        // 无效：证书路径不匹配
        config.cert_path = Some(PathBuf::from("/cert.pem"));
        config.key_path = None;
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_client_config_basic() {
        let config = ClientConfig {
            server_addr: "example.com".to_string(),
            server_port: 8443,
            server_path: "/".to_string(),
            transport: TransportType::Tls,
            skip_verify: false,
            ca_cert_path: Some(PathBuf::from("/path/to/ca.pem")),
            auth_key: "a".repeat(20),
            stats_port: None,
            stats_addr: None,
        };

        assert_eq!(config.server_port, 8443);
        assert_eq!(config.transport, TransportType::Tls);
    }

    #[test]
    fn test_server_config_builder() {
        let config = ServerConfig::builder()
            .bind_addr("0.0.0.0")
            .bind_port(8443)
            .auth_key("1234567890123456")
            .build();

        assert!(config.is_ok());
    }

    #[test]
    fn test_client_config_builder() {
        let config = ClientConfig::builder()
            .server_addr("example.com")
            .server_port(8443)
            .auth_key("1234567890123456")
            .build();

        assert!(config.is_ok());
    }

    #[test]
    fn test_rate_limit_config_default() {
        let config = RateLimitConfig::default();
        assert_eq!(config.requests_per_second, 100);
        assert_eq!(config.burst_size, 200);
    }

    #[test]
    fn test_size_limit_config_default() {
        let config = SizeLimitConfig::default();
        assert_eq!(config.max_request_size, 1024 * 1024); // 1 MB
        assert_eq!(config.max_header_size, 8 * 1024); // 8 KB
    }

    #[test]
    fn test_server_config_with_rate_limit() {
        let rate_limit = RateLimitConfig {
            requests_per_second: 50,
            burst_size: 100,
        };

        let config = ServerConfig {
            bind_addr: "0.0.0.0".to_string(),
            bind_port: 8443,
            transport: TransportType::Tls,
            behind_proxy: false,
            cert_path: Some(PathBuf::from("/path/to/cert.pem")),
            key_path: Some(PathBuf::from("/path/to/key.pem")),
            auth_key: "a".repeat(20),
            stats_port: None,
            stats_addr: None,
            allow_forward: false,
            rate_limit: Some(rate_limit),
            size_limits: None,
        };

        assert!(config.validate().is_ok());
        assert_eq!(config.rate_limit.as_ref().unwrap().requests_per_second, 50);
    }

    #[test]
    fn test_server_config_with_size_limits() {
        let size_limits = SizeLimitConfig {
            max_request_size: 2 * 1024 * 1024, // 2 MB
            max_header_size: 16 * 1024,        // 16 KB
        };

        let config = ServerConfig {
            bind_addr: "0.0.0.0".to_string(),
            bind_port: 8443,
            transport: TransportType::Tls,
            behind_proxy: false,
            cert_path: Some(PathBuf::from("/path/to/cert.pem")),
            key_path: Some(PathBuf::from("/path/to/key.pem")),
            auth_key: "a".repeat(20),
            stats_port: None,
            stats_addr: None,
            allow_forward: false,
            rate_limit: None,
            size_limits: Some(size_limits),
        };

        assert!(config.validate().is_ok());
        assert_eq!(
            config.size_limits.as_ref().unwrap().max_request_size,
            2 * 1024 * 1024
        );
    }

    #[test]
    fn test_toml_deserialization_with_rate_limit() {
        // 测试 TOML 反序列化是否正确处理 rate_limit 和 size_limits 字段
        let toml_str = r#"
            bind_addr = "127.0.0.1"
            bind_port = 3080
            auth_key = "test-key-12345678"
            behind_proxy = true
            transport = "http2"
            
            [rate_limit]
            requests_per_second = 50
            burst_size = 100
            
            [size_limits]
            max_request_size = 2097152
            max_header_size = 16384
        "#;

        let config: ServerConfig = toml::from_str(toml_str).unwrap();

        // 验证速率限制配置
        assert!(config.rate_limit.is_some());
        let rate_limit = config.rate_limit.unwrap();
        assert_eq!(rate_limit.requests_per_second, 50);
        assert_eq!(rate_limit.burst_size, 100);

        // 验证大小限制配置
        assert!(config.size_limits.is_some());
        let size_limits = config.size_limits.unwrap();
        assert_eq!(size_limits.max_request_size, 2097152);
        assert_eq!(size_limits.max_header_size, 16384);
    }
}
