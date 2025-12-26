// 异常通知功能测试示例
//
// 本文件演示如何在实际场景中使用异常通知功能

use serde_json::json;
use tls_tunnel::control_protocol::*;

#[test]
fn test_exception_notification_structure() {
    // 创建错误级别通知
    let error_notification = ExceptionNotification {
        level: "error".to_string(),
        message: "配置验证失败".to_string(),
        code: Some("CONFIG_ERROR".to_string()),
        data: Some(json!({
            "field": "port",
            "value": -1
        })),
    };

    assert_eq!(error_notification.level, "error");
    assert_eq!(error_notification.message, "配置验证失败");
    assert!(error_notification.code.is_some());
    assert!(error_notification.data.is_some());
}

#[test]
fn test_exception_notification_serialization() {
    let notification = ExceptionNotification {
        level: "warning".to_string(),
        message: "连接数接近限制".to_string(),
        code: Some("CONN_LIMIT".to_string()),
        data: Some(json!({"current": 900, "limit": 1000})),
    };

    // 序列化为 JSON
    let json_str = serde_json::to_string(&notification).unwrap();
    assert!(json_str.contains("warning"));
    assert!(json_str.contains("连接数接近限制"));
    assert!(json_str.contains("CONN_LIMIT"));

    // 反序列化
    let deserialized: ExceptionNotification = serde_json::from_str(&json_str).unwrap();
    assert_eq!(deserialized.level, "warning");
    assert_eq!(deserialized.message, "连接数接近限制");
}

#[test]
fn test_exception_notification_optional_fields() {
    // 创建只包含必需字段的通知
    let minimal_notification = ExceptionNotification {
        level: "info".to_string(),
        message: "配置已更新".to_string(),
        code: None,
        data: None,
    };

    // 序列化时应该跳过 None 字段
    let json_str = serde_json::to_string(&minimal_notification).unwrap();
    assert!(!json_str.contains("code"));
    assert!(!json_str.contains("data"));
}

#[test]
fn test_jsonrpc_notification_format() {
    let notification = ExceptionNotification {
        level: "error".to_string(),
        message: "测试错误".to_string(),
        code: Some("TEST_ERROR".to_string()),
        data: None,
    };

    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        method: "push_exception".to_string(),
        params: serde_json::to_value(&notification).unwrap(),
        id: None, // 通知没有 ID
    };

    assert!(request.is_notification());
    assert_eq!(request.method, "push_exception");
}

// 使用示例（集成到实际代码中）
mod usage_examples {
    use tls_tunnel::control_protocol::ExceptionNotification;

    /// 示例1：配置错误通知
    #[allow(dead_code)]
    fn example_config_error() -> ExceptionNotification {
        ExceptionNotification {
            level: "error".to_string(),
            message: "代理配置无效：端口号必须在 1-65535 之间".to_string(),
            code: Some("INVALID_PORT".to_string()),
            data: Some(serde_json::json!({
                "proxy_name": "web-proxy",
                "invalid_port": -1,
                "valid_range": "1-65535"
            })),
        }
    }

    /// 示例2：连接警告通知
    #[allow(dead_code)]
    fn example_connection_warning() -> ExceptionNotification {
        ExceptionNotification {
            level: "warning".to_string(),
            message: "目标服务器响应缓慢".to_string(),
            code: Some("SLOW_RESPONSE".to_string()),
            data: Some(serde_json::json!({
                "target": "backend.example.com:8080",
                "response_time_ms": 5000,
                "threshold_ms": 1000
            })),
        }
    }

    /// 示例3：状态信息通知
    #[allow(dead_code)]
    fn example_status_info() -> ExceptionNotification {
        ExceptionNotification {
            level: "info".to_string(),
            message: "代理服务已启动".to_string(),
            code: Some("PROXY_STARTED".to_string()),
            data: Some(serde_json::json!({
                "proxy_name": "web-proxy",
                "publish_port": 8080,
                "started_at": "2025-12-27T10:30:00Z"
            })),
        }
    }

    /// 示例4：资源限制警告
    #[allow(dead_code)]
    fn example_resource_warning() -> ExceptionNotification {
        ExceptionNotification {
            level: "warning".to_string(),
            message: "带宽使用率超过 80%".to_string(),
            code: Some("BANDWIDTH_WARNING".to_string()),
            data: Some(serde_json::json!({
                "usage_mbps": 800,
                "limit_mbps": 1000,
                "percentage": 80,
                "recommendation": "考虑升级带宽或限制连接数"
            })),
        }
    }

    /// 示例5：认证失败通知
    #[allow(dead_code)]
    fn example_auth_error() -> ExceptionNotification {
        ExceptionNotification {
            level: "error".to_string(),
            message: "认证失败：密钥不匹配".to_string(),
            code: Some("AUTH_FAILED".to_string()),
            data: Some(serde_json::json!({
                "client_id": "unknown",
                "attempt_count": 3,
                "locked_until": "2025-12-27T10:35:00Z"
            })),
        }
    }
}
