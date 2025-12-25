use anyhow::{bail, Result};
use std::collections::HashSet;
use tracing::warn;

use super::{ClientFullConfig, ForwarderConfig, ProxyConfig, ServerConfig, VisitorConfig};

/// 配置验证器 - 负责所有配置验证逻辑
pub struct ConfigValidator;

impl ConfigValidator {
    /// 验证认证密钥强度
    pub fn validate_auth_key(auth_key: &str) -> Result<()> {
        if auth_key.len() < 16 {
            bail!(
                "auth_key must be at least 16 characters for security (current length: {})",
                auth_key.len()
            );
        }
        Ok(())
    }

    /// 验证端口号
    pub fn validate_port(port: u16, context: &str) -> Result<()> {
        if port == 0 {
            bail!("{}: port cannot be 0", context);
        }
        Ok(())
    }

    /// 验证地址不为空
    pub fn validate_address(addr: &str, context: &str) -> Result<()> {
        if addr.trim().is_empty() {
            bail!("{}: address cannot be empty", context);
        }
        Ok(())
    }

    /// 验证名称不为空
    pub fn validate_name(name: &str, context: &str) -> Result<()> {
        if name.trim().is_empty() {
            bail!("{}: name cannot be empty", context);
        }
        Ok(())
    }

    /// 验证服务器配置
    pub fn validate_server_config(config: &ServerConfig) -> Result<()> {
        // 验证绑定地址
        Self::validate_address(&config.bind_addr, "Server bind_addr")?;

        // 验证认证密钥
        Self::validate_auth_key(&config.auth_key)?;

        // 验证统计服务器地址（如果配置了）
        if let Some(ref addr) = config.stats_addr {
            Self::validate_address(addr, "Server stats_addr")?;
        }

        // 验证证书配置
        match (&config.cert_path, &config.key_path) {
            (Some(_), Some(_)) | (None, None) => {}
            _ => bail!("cert_path and key_path must both be set, or both omitted to auto-generate"),
        }

        // 验证反向代理配置
        if config.behind_proxy && config.transport == crate::transport::TransportType::Tls {
            bail!("TLS transport cannot run behind a proxy. Use http2 or wss transport instead.");
        }

        // 当在反向代理后运行时，不需要证书
        if config.behind_proxy && (config.cert_path.is_some() || config.key_path.is_some()) {
            bail!("Certificates are not needed when running behind a proxy (TLS is terminated by the proxy).");
        }

        // 验证速率限制配置
        if let Some(ref rate_limit) = config.rate_limit {
            Self::validate_rate_limit_config(rate_limit)?;
        }

        // 验证请求大小限制配置
        if let Some(ref size_limits) = config.size_limits {
            Self::validate_size_limit_config(size_limits)?;
        }

        Ok(())
    }

    /// 验证速率限制配置
    pub fn validate_rate_limit_config(config: &super::RateLimitConfig) -> Result<()> {
        if config.requests_per_second == 0 {
            bail!("rate_limit.requests_per_second must be greater than 0");
        }
        if config.burst_size == 0 {
            bail!("rate_limit.burst_size must be greater than 0");
        }
        if config.burst_size < config.requests_per_second {
            warn!(
                "rate_limit.burst_size ({}) is less than requests_per_second ({}), \
                 this may cause frequent rate limiting",
                config.burst_size, config.requests_per_second
            );
        }
        Ok(())
    }

    /// 验证请求大小限制配置
    pub fn validate_size_limit_config(config: &super::SizeLimitConfig) -> Result<()> {
        if config.max_request_size == 0 {
            bail!("size_limits.max_request_size must be greater than 0");
        }
        if config.max_header_size == 0 {
            bail!("size_limits.max_header_size must be greater than 0");
        }
        if config.max_header_size > config.max_request_size {
            bail!(
                "size_limits.max_header_size ({}) cannot be greater than max_request_size ({})",
                config.max_header_size,
                config.max_request_size
            );
        }
        // 建议值检查
        if config.max_request_size > 100 * 1024 * 1024 {
            warn!(
                "size_limits.max_request_size is very large ({} bytes = {} MB), \
                 this may lead to memory exhaustion attacks",
                config.max_request_size,
                config.max_request_size / (1024 * 1024)
            );
        }
        Ok(())
    }

    /// 验证 Proxy 配置列表
    pub fn validate_proxies(proxies: &[ProxyConfig]) -> Result<()> {
        let mut seen_names = HashSet::new();
        let mut seen_bind = HashSet::new();
        let mut seen_local_ports = HashSet::new();

        for proxy in proxies {
            // 验证名称
            Self::validate_name(&proxy.name, "Proxy name")?;

            // 检查 name 唯一性
            if !seen_names.insert(&proxy.name) {
                bail!(
                    "Duplicate proxy name '{}': each proxy must have a unique name",
                    proxy.name
                );
            }

            // 检查 (publish_addr, publish_port) 唯一性
            if !seen_bind.insert((proxy.publish_addr.clone(), proxy.publish_port)) {
                bail!(
                    "Duplicate publish binding {}:{}: each proxy must use a different server bind address/port",
                    proxy.publish_addr,
                    proxy.publish_port
                );
            }

            // 检查 local_port 唯一性
            if !seen_local_ports.insert(proxy.local_port) {
                bail!(
                    "Duplicate local_port {}: each proxy must connect to a different local service",
                    proxy.local_port
                );
            }

            // 验证端口
            Self::validate_port(proxy.publish_port, &format!("Proxy '{}'", proxy.name))?;
            Self::validate_port(proxy.local_port, &format!("Proxy '{}'", proxy.name))?;

            // 验证地址
            Self::validate_address(&proxy.publish_addr, &format!("Proxy '{}'", proxy.name))?;
        }

        Ok(())
    }

    /// 验证 Visitor 配置列表
    pub fn validate_visitors(visitors: &[VisitorConfig]) -> Result<()> {
        let mut seen_names = HashSet::new();
        let mut seen_binds = HashSet::new();

        for visitor in visitors {
            // 验证名称
            Self::validate_name(&visitor.name, "Visitor name")?;

            // 检查 name 唯一性
            if !seen_names.insert(&visitor.name) {
                bail!(
                    "Duplicate visitor name '{}': each visitor must have a unique name",
                    visitor.name
                );
            }

            // 检查 (bind_addr, bind_port) 唯一性
            if !seen_binds.insert((visitor.bind_addr.clone(), visitor.bind_port)) {
                bail!(
                    "Duplicate visitor binding {}:{}: each visitor must use a different local bind address/port",
                    visitor.bind_addr,
                    visitor.bind_port
                );
            }

            // 验证端口
            Self::validate_port(visitor.bind_port, &format!("Visitor '{}'", visitor.name))?;
            Self::validate_port(visitor.publish_port, &format!("Visitor '{}'", visitor.name))?;

            // 验证地址
            Self::validate_address(&visitor.bind_addr, &format!("Visitor '{}'", visitor.name))?;
        }

        Ok(())
    }

    /// 验证 Forwarder 配置列表
    pub fn validate_forwarders(forwarders: &[ForwarderConfig]) -> Result<()> {
        let mut seen_names = HashSet::new();
        let mut seen_binds = HashSet::new();

        for forwarder in forwarders {
            // 验证名称
            Self::validate_name(&forwarder.name, "Forwarder name")?;

            // 检查 name 唯一性
            if !seen_names.insert(&forwarder.name) {
                bail!(
                    "Duplicate forwarder name '{}': each forwarder must have a unique name",
                    forwarder.name
                );
            }

            // 检查 (bind_addr, bind_port) 唯一性
            if !seen_binds.insert((forwarder.bind_addr.clone(), forwarder.bind_port)) {
                bail!(
                    "Duplicate forwarder binding {}:{}: each forwarder must use a different local bind address/port",
                    forwarder.bind_addr,
                    forwarder.bind_port
                );
            }

            // 安全检查：警告绑定到非本地地址
            Self::check_forwarder_security(&forwarder.name, &forwarder.bind_addr);

            // 验证端口
            Self::validate_port(
                forwarder.bind_port,
                &format!("Forwarder '{}'", forwarder.name),
            )?;

            // 验证地址
            Self::validate_address(
                &forwarder.bind_addr,
                &format!("Forwarder '{}'", forwarder.name),
            )?;
        }

        Ok(())
    }

    /// 检查 forwarder 安全性（绑定地址）
    fn check_forwarder_security(name: &str, bind_addr: &str) {
        if bind_addr != "127.0.0.1" && bind_addr != "localhost" && bind_addr != "::1" {
            warn!(
                "⚠️  SECURITY WARNING: Forwarder '{}' is binding to '{}' which exposes the proxy to your network!\n\
                 This allows anyone on the network to use your proxy, which may lead to:\n\
                 - Abuse of your IP address\n\
                 - Your IP being blacklisted\n\
                 - Unauthorized use of your bandwidth\n\
                 RECOMMENDATION: Use bind_addr = '127.0.0.1' for localhost-only access.",
                name, bind_addr
            );
        }
    }

    /// 验证客户端完整配置
    pub fn validate_client_full_config(config: &ClientFullConfig) -> Result<()> {
        // 验证认证密钥
        Self::validate_auth_key(&config.client.auth_key)?;

        // 至少要有一个配置
        if config.proxies.is_empty() && config.visitors.is_empty() && config.forwarders.is_empty() {
            bail!("No proxy, visitor, or forwarder configurations defined");
        }

        // 验证各个配置列表
        Self::validate_proxies(&config.proxies)?;
        Self::validate_visitors(&config.visitors)?;
        Self::validate_forwarders(&config.forwarders)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_auth_key() {
        // 太短的密钥应该失败
        assert!(ConfigValidator::validate_auth_key("short").is_err());

        // 长度够的密钥应该成功
        assert!(ConfigValidator::validate_auth_key("1234567890123456").is_ok());
        assert!(ConfigValidator::validate_auth_key("very-long-secure-key-12345678").is_ok());
    }

    #[test]
    fn test_validate_port() {
        // 端口 0 应该失败
        assert!(ConfigValidator::validate_port(0, "test").is_err());

        // 有效端口应该成功
        assert!(ConfigValidator::validate_port(8080, "test").is_ok());
        assert!(ConfigValidator::validate_port(443, "test").is_ok());
        assert!(ConfigValidator::validate_port(65535, "test").is_ok());
    }

    #[test]
    fn test_validate_address() {
        // 空地址应该失败
        assert!(ConfigValidator::validate_address("", "test").is_err());
        assert!(ConfigValidator::validate_address("   ", "test").is_err());

        // 有效地址应该成功
        assert!(ConfigValidator::validate_address("127.0.0.1", "test").is_ok());
        assert!(ConfigValidator::validate_address("0.0.0.0", "test").is_ok());
        assert!(ConfigValidator::validate_address("example.com", "test").is_ok());
    }

    #[test]
    fn test_validate_name() {
        // 空名称应该失败
        assert!(ConfigValidator::validate_name("", "test").is_err());
        assert!(ConfigValidator::validate_name("   ", "test").is_err());

        // 有效名称应该成功
        assert!(ConfigValidator::validate_name("proxy1", "test").is_ok());
        assert!(ConfigValidator::validate_name("my-proxy", "test").is_ok());
    }

    #[test]
    fn test_validate_rate_limit_config() {
        use super::super::RateLimitConfig;

        // requests_per_second = 0 应该失败
        let invalid_config = RateLimitConfig {
            requests_per_second: 0,
            burst_size: 200,
        };
        assert!(ConfigValidator::validate_rate_limit_config(&invalid_config).is_err());

        // burst_size = 0 应该失败
        let invalid_config = RateLimitConfig {
            requests_per_second: 100,
            burst_size: 0,
        };
        assert!(ConfigValidator::validate_rate_limit_config(&invalid_config).is_err());

        // 有效配置应该成功
        let valid_config = RateLimitConfig {
            requests_per_second: 100,
            burst_size: 200,
        };
        assert!(ConfigValidator::validate_rate_limit_config(&valid_config).is_ok());
    }

    #[test]
    fn test_validate_size_limit_config() {
        use super::super::SizeLimitConfig;

        // max_request_size = 0 应该失败
        let invalid_config = SizeLimitConfig {
            max_request_size: 0,
            max_header_size: 8192,
        };
        assert!(ConfigValidator::validate_size_limit_config(&invalid_config).is_err());

        // max_header_size = 0 应该失败
        let invalid_config = SizeLimitConfig {
            max_request_size: 1048576,
            max_header_size: 0,
        };
        assert!(ConfigValidator::validate_size_limit_config(&invalid_config).is_err());

        // max_header_size > max_request_size 应该失败
        let invalid_config = SizeLimitConfig {
            max_request_size: 8192,
            max_header_size: 16384,
        };
        assert!(ConfigValidator::validate_size_limit_config(&invalid_config).is_err());

        // 有效配置应该成功
        let valid_config = SizeLimitConfig {
            max_request_size: 1048576,
            max_header_size: 8192,
        };
        assert!(ConfigValidator::validate_size_limit_config(&valid_config).is_ok());
    }
}
