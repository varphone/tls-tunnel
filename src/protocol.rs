/// 客户端与服务器之间的协议消息定义
use serde::{Deserialize, Serialize};

/// 认证请求消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequest {
    /// 认证密钥
    pub auth_key: String,
}

/// 认证响应消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    /// 认证是否成功
    pub success: bool,
    /// 如果失败，错误信息
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl AuthResponse {
    /// 创建成功响应
    pub fn success() -> Self {
        Self {
            success: true,
            error: None,
        }
    }

    /// 创建失败响应
    pub fn failed(error: String) -> Self {
        Self {
            success: false,
            error: Some(error),
        }
    }
}

/// 配置验证响应消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigValidationResponse {
    /// 配置是否有效
    pub valid: bool,
    /// 如果无效，错误信息
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ConfigValidationResponse {
    /// 创建有效响应
    pub fn valid() -> Self {
        Self {
            valid: true,
            error: None,
        }
    }

    /// 创建无效响应
    pub fn invalid(error: String) -> Self {
        Self {
            valid: false,
            error: Some(error),
        }
    }
}

/// 服务器发送给客户端的配置状态响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigStatusResponse {
    /// 是否所有配置都被接受
    pub accepted: bool,
    /// 被拒绝的代理列表（已被其他客户端占用）
    #[serde(default)]
    pub rejected_proxies: Vec<String>,
}

impl ConfigStatusResponse {
    /// 创建接受响应
    pub fn accepted() -> Self {
        Self {
            accepted: true,
            rejected_proxies: Vec::new(),
        }
    }

    /// 创建部分拒绝响应
    pub fn partially_rejected(rejected: Vec<String>) -> Self {
        Self {
            accepted: true,
            rejected_proxies: rejected,
        }
    }

    /// 创建全部拒绝响应
    pub fn all_rejected(rejected: Vec<String>) -> Self {
        Self {
            accepted: false,
            rejected_proxies: rejected,
        }
    }

    /// 是否有被拒绝的代理
    pub fn has_rejected(&self) -> bool {
        !self.rejected_proxies.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_status_response_accepted() {
        let resp = ConfigStatusResponse::accepted();
        assert!(resp.accepted);
        assert!(resp.rejected_proxies.is_empty());
    }

    #[test]
    fn test_config_status_response_partially_rejected() {
        let resp = ConfigStatusResponse::partially_rejected(vec!["proxy1".to_string()]);
        assert!(resp.accepted);
        assert_eq!(resp.rejected_proxies.len(), 1);
        assert!(resp.has_rejected());
    }

    #[test]
    fn test_config_status_response_all_rejected() {
        let resp = ConfigStatusResponse::all_rejected(vec!["proxy1".to_string()]);
        assert!(!resp.accepted);
        assert_eq!(resp.rejected_proxies.len(), 1);
    }
}
