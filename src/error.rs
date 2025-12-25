/// 自定义错误类型
///
/// 使用 thiserror 定义精确的错误类型，替代泛型的 anyhow::Error
/// 这样可以让调用者进行更精确的错误处理和恢复
use std::io;
use thiserror::Error;

/// TLS Tunnel 的主要错误类型
#[derive(Error, Debug)]
pub enum TunnelError {
    /// 连接失败
    #[error("Failed to connect to {addr}: {source}")]
    ConnectionFailed {
        addr: String,
        #[source]
        source: io::Error,
    },

    /// 认证失败
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),

    /// 配置错误
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// 传输层错误
    #[error("Transport error: {0}")]
    TransportError(String),

    /// 协议错误
    #[error("Protocol error: {0}")]
    ProtocolError(String),

    /// 超时错误
    #[error("Operation timeout after {duration:?}")]
    Timeout { duration: std::time::Duration },

    /// 代理未找到
    #[error("Proxy '{name}' with port {port} not found")]
    ProxyNotFound { name: String, port: u16 },

    /// 路由错误
    #[error("Routing error: {0}")]
    RoutingError(String),

    /// 安全错误（SSRF等）
    #[error("Security error: {0}")]
    SecurityError(String),

    /// 资源耗尽
    #[error("Resource exhausted: {0}")]
    ResourceExhausted(String),

    /// I/O 错误
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// 其他错误（保留与 anyhow 的兼容性）
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Result 类型别名
pub type Result<T> = std::result::Result<T, TunnelError>;

impl TunnelError {
    /// 创建连接失败错误
    pub fn connection_failed(addr: impl Into<String>, source: io::Error) -> Self {
        Self::ConnectionFailed {
            addr: addr.into(),
            source,
        }
    }

    /// 创建认证失败错误
    pub fn auth_failed(msg: impl Into<String>) -> Self {
        Self::AuthenticationFailed(msg.into())
    }

    /// 创建配置错误
    pub fn config_error(msg: impl Into<String>) -> Self {
        Self::ConfigError(msg.into())
    }

    /// 创建超时错误
    pub fn timeout(duration: std::time::Duration) -> Self {
        Self::Timeout { duration }
    }

    /// 创建代理未找到错误
    pub fn proxy_not_found(name: impl Into<String>, port: u16) -> Self {
        Self::ProxyNotFound {
            name: name.into(),
            port,
        }
    }

    /// 创建安全错误
    pub fn security_error(msg: impl Into<String>) -> Self {
        Self::SecurityError(msg.into())
    }

    /// 检查是否为超时错误
    pub fn is_timeout(&self) -> bool {
        matches!(self, Self::Timeout { .. })
    }

    /// 检查是否为认证失败
    pub fn is_auth_failed(&self) -> bool {
        matches!(self, Self::AuthenticationFailed(_))
    }

    /// 检查是否为配置错误
    pub fn is_config_error(&self) -> bool {
        matches!(self, Self::ConfigError(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_error_creation() {
        let err = TunnelError::auth_failed("Invalid key");
        assert!(err.is_auth_failed());
        assert_eq!(err.to_string(), "Authentication failed: Invalid key");
    }

    #[test]
    fn test_timeout_error() {
        use std::time::Duration;
        let err = TunnelError::timeout(Duration::from_secs(30));
        assert!(err.is_timeout());
        assert!(err.to_string().contains("timeout"));
    }

    #[test]
    fn test_proxy_not_found() {
        let err = TunnelError::proxy_not_found("test-proxy", 8080);
        assert_eq!(
            err.to_string(),
            "Proxy 'test-proxy' with port 8080 not found"
        );
    }

    #[test]
    fn test_connection_failed() {
        let io_err = io::Error::new(io::ErrorKind::ConnectionRefused, "refused");
        let err = TunnelError::connection_failed("127.0.0.1:8080", io_err);
        assert!(err.to_string().contains("Failed to connect"));
        assert!(err.to_string().contains("127.0.0.1:8080"));
    }

    #[test]
    fn test_error_is_checks() {
        let auth_err = TunnelError::auth_failed("test");
        let config_err = TunnelError::config_error("test");
        let timeout_err = TunnelError::timeout(Duration::from_secs(1));

        assert!(auth_err.is_auth_failed());
        assert!(!auth_err.is_config_error());
        assert!(!auth_err.is_timeout());

        assert!(config_err.is_config_error());
        assert!(!config_err.is_auth_failed());

        assert!(timeout_err.is_timeout());
        assert!(!timeout_err.is_auth_failed());
    }
}
