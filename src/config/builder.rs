use anyhow::{Context, Result};
use std::path::PathBuf;

use crate::transport::TransportType;

use super::{
    validator::ConfigValidator, ClientConfig, ClientFullConfig, ForwarderConfig, ProxyConfig,
    ServerConfig, VisitorConfig,
};

/// ServerConfig Builder
#[derive(Debug, Default)]
pub struct ServerConfigBuilder {
    bind_addr: Option<String>,
    bind_port: Option<u16>,
    transport: Option<TransportType>,
    behind_proxy: bool,
    cert_path: Option<PathBuf>,
    key_path: Option<PathBuf>,
    auth_key: Option<String>,
    stats_port: Option<u16>,
    stats_addr: Option<String>,
    allow_forward: bool,
}

impl ServerConfigBuilder {
    /// 创建新的 Builder
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置绑定地址
    pub fn bind_addr(mut self, addr: impl Into<String>) -> Self {
        self.bind_addr = Some(addr.into());
        self
    }

    /// 设置绑定端口
    pub fn bind_port(mut self, port: u16) -> Self {
        self.bind_port = Some(port);
        self
    }

    /// 设置传输类型
    pub fn transport(mut self, transport: TransportType) -> Self {
        self.transport = Some(transport);
        self
    }

    /// 设置是否在反向代理后运行
    pub fn behind_proxy(mut self, behind: bool) -> Self {
        self.behind_proxy = behind;
        self
    }

    /// 设置证书路径
    pub fn cert_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.cert_path = Some(path.into());
        self
    }

    /// 设置私钥路径
    pub fn key_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.key_path = Some(path.into());
        self
    }

    /// 设置认证密钥
    pub fn auth_key(mut self, key: impl Into<String>) -> Self {
        self.auth_key = Some(key.into());
        self
    }

    /// 设置统计端口
    pub fn stats_port(mut self, port: u16) -> Self {
        self.stats_port = Some(port);
        self
    }

    /// 设置统计地址
    pub fn stats_addr(mut self, addr: impl Into<String>) -> Self {
        self.stats_addr = Some(addr.into());
        self
    }

    /// 设置是否允许 forward proxy
    pub fn allow_forward(mut self, allow: bool) -> Self {
        self.allow_forward = allow;
        self
    }

    /// 构建 ServerConfig 并验证
    pub fn build(self) -> Result<ServerConfig> {
        let config = ServerConfig {
            bind_addr: self.bind_addr.context("bind_addr is required")?,
            bind_port: self.bind_port.context("bind_port is required")?,
            transport: self.transport.unwrap_or_default(),
            behind_proxy: self.behind_proxy,
            cert_path: self.cert_path,
            key_path: self.key_path,
            auth_key: self.auth_key.context("auth_key is required")?,
            stats_port: self.stats_port,
            stats_addr: self.stats_addr,
            allow_forward: self.allow_forward,
            rate_limit: None,  // Builder 默认不设置速率限制
            size_limits: None, // Builder 默认不设置大小限制
        };

        // 验证配置
        ConfigValidator::validate_server_config(&config)?;

        Ok(config)
    }
}

/// ClientConfig Builder
#[derive(Debug, Default)]
pub struct ClientConfigBuilder {
    server_addr: Option<String>,
    server_port: Option<u16>,
    server_path: String,
    transport: Option<TransportType>,
    skip_verify: bool,
    ca_cert_path: Option<PathBuf>,
    auth_key: Option<String>,
}

impl ClientConfigBuilder {
    /// 创建新的 Builder
    pub fn new() -> Self {
        Self {
            server_path: "/".to_string(),
            ..Default::default()
        }
    }

    /// 设置服务器地址
    pub fn server_addr(mut self, addr: impl Into<String>) -> Self {
        self.server_addr = Some(addr.into());
        self
    }

    /// 设置服务器端口
    pub fn server_port(mut self, port: u16) -> Self {
        self.server_port = Some(port);
        self
    }

    /// 设置服务器路径
    pub fn server_path(mut self, path: impl Into<String>) -> Self {
        self.server_path = path.into();
        self
    }

    /// 设置传输类型
    pub fn transport(mut self, transport: TransportType) -> Self {
        self.transport = Some(transport);
        self
    }

    /// 设置是否跳过证书验证
    pub fn skip_verify(mut self, skip: bool) -> Self {
        self.skip_verify = skip;
        self
    }

    /// 设置 CA 证书路径
    pub fn ca_cert_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.ca_cert_path = Some(path.into());
        self
    }

    /// 设置认证密钥
    pub fn auth_key(mut self, key: impl Into<String>) -> Self {
        self.auth_key = Some(key.into());
        self
    }

    /// 构建 ClientConfig
    pub fn build(self) -> Result<ClientConfig> {
        let config = ClientConfig {
            server_addr: self.server_addr.context("server_addr is required")?,
            server_port: self.server_port.context("server_port is required")?,
            server_path: self.server_path,
            transport: self.transport.unwrap_or_default(),
            skip_verify: self.skip_verify,
            ca_cert_path: self.ca_cert_path,
            auth_key: self.auth_key.context("auth_key is required")?,
            stats_port: None,
            stats_addr: None,
        };

        // 验证认证密钥
        ConfigValidator::validate_auth_key(&config.auth_key)?;

        Ok(config)
    }
}

/// ClientFullConfig Builder
#[derive(Debug, Default)]
pub struct ClientFullConfigBuilder {
    client: Option<ClientConfig>,
    proxies: Vec<ProxyConfig>,
    visitors: Vec<VisitorConfig>,
    forwarders: Vec<ForwarderConfig>,
}

impl ClientFullConfigBuilder {
    /// 创建新的 Builder
    pub fn new() -> Self {
        Self::default()
    }

    /// 设置客户端配置
    pub fn client(mut self, client: ClientConfig) -> Self {
        self.client = Some(client);
        self
    }

    /// 添加 proxy 配置
    pub fn add_proxy(mut self, proxy: ProxyConfig) -> Self {
        self.proxies.push(proxy);
        self
    }

    /// 批量添加 proxy 配置
    pub fn proxies(mut self, proxies: Vec<ProxyConfig>) -> Self {
        self.proxies = proxies;
        self
    }

    /// 添加 visitor 配置
    pub fn add_visitor(mut self, visitor: VisitorConfig) -> Self {
        self.visitors.push(visitor);
        self
    }

    /// 批量添加 visitor 配置
    pub fn visitors(mut self, visitors: Vec<VisitorConfig>) -> Self {
        self.visitors = visitors;
        self
    }

    /// 添加 forwarder 配置
    pub fn add_forwarder(mut self, forwarder: ForwarderConfig) -> Self {
        self.forwarders.push(forwarder);
        self
    }

    /// 批量添加 forwarder 配置
    pub fn forwarders(mut self, forwarders: Vec<ForwarderConfig>) -> Self {
        self.forwarders = forwarders;
        self
    }

    /// 构建 ClientFullConfig 并验证
    pub fn build(self) -> Result<ClientFullConfig> {
        let config = ClientFullConfig {
            client: self.client.context("client config is required")?,
            proxies: self.proxies,
            visitors: self.visitors,
            forwarders: self.forwarders,
        };

        // 验证配置
        ConfigValidator::validate_client_full_config(&config)?;

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_config_builder() {
        let config = ServerConfigBuilder::new()
            .bind_addr("0.0.0.0")
            .bind_port(8443)
            .auth_key("1234567890123456")
            .build();

        assert!(config.is_ok());
        let config = config.unwrap();
        assert_eq!(config.bind_addr, "0.0.0.0");
        assert_eq!(config.bind_port, 8443);
        assert_eq!(config.auth_key, "1234567890123456");
    }

    #[test]
    fn test_server_config_builder_missing_required() {
        // 缺少必需字段应该失败
        let result = ServerConfigBuilder::new().bind_addr("0.0.0.0").build();

        assert!(result.is_err());
    }

    #[test]
    fn test_server_config_builder_invalid_auth_key() {
        // 认证密钥太短应该失败
        let result = ServerConfigBuilder::new()
            .bind_addr("0.0.0.0")
            .bind_port(8443)
            .auth_key("short")
            .build();

        assert!(result.is_err());
    }

    #[test]
    fn test_client_config_builder() {
        let config = ClientConfigBuilder::new()
            .server_addr("example.com")
            .server_port(8443)
            .auth_key("1234567890123456")
            .build();

        assert!(config.is_ok());
        let config = config.unwrap();
        assert_eq!(config.server_addr, "example.com");
        assert_eq!(config.server_port, 8443);
        assert_eq!(config.auth_key, "1234567890123456");
    }

    #[test]
    fn test_client_full_config_builder() {
        let client = ClientConfigBuilder::new()
            .server_addr("example.com")
            .server_port(8443)
            .auth_key("1234567890123456")
            .build()
            .unwrap();

        let proxy = ProxyConfig {
            name: "test-proxy".to_string(),
            proxy_type: super::super::ProxyType::Tcp,
            publish_addr: "0.0.0.0".to_string(),
            publish_port: 9000,
            local_port: 8080,
        };

        let config = ClientFullConfigBuilder::new()
            .client(client)
            .add_proxy(proxy)
            .build();

        assert!(config.is_ok());
        let config = config.unwrap();
        assert_eq!(config.proxies.len(), 1);
    }
}
