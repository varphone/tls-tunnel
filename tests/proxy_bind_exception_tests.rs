// 代理绑定异常通知功能测试

use tls_tunnel::server::connection::ExceptionNotification;

#[test]
fn test_exception_notification_request_structure() {
    let request = ExceptionNotification {
        level: "warning".to_string(),
        message: "代理绑定失败".to_string(),
        code: Some("PROXY_BIND_RETRY".to_string()),
        data: Some(serde_json::json!({
            "proxy_name": "web-proxy",
            "retry_count": 1
        })),
    };

    assert_eq!(request.level, "warning");
    assert_eq!(request.message, "代理绑定失败");
    assert_eq!(request.code, Some("PROXY_BIND_RETRY".to_string()));
    assert!(request.data.is_some());
}

#[test]
fn test_exception_notification_request_optional_fields() {
    let request = ExceptionNotification {
        level: "error".to_string(),
        message: "绑定失败".to_string(),
        code: None,
        data: None,
    };

    assert_eq!(request.level, "error");
    assert!(request.code.is_none());
    assert!(request.data.is_none());
}

#[test]
fn test_bind_retry_notification_data() {
    let request = ExceptionNotification {
        level: "warning".to_string(),
        message: "代理 'web-proxy' 绑定失败 (尝试 1/10)".to_string(),
        code: Some("PROXY_BIND_RETRY".to_string()),
        data: Some(serde_json::json!({
            "proxy_name": "web-proxy",
            "publish_addr": "0.0.0.0",
            "publish_port": 8080,
            "retry_count": 1,
            "max_retries": 10,
            "retry_delay_secs": 2,
            "error": "Address already in use"
        })),
    };

    let data = request.data.unwrap();
    assert_eq!(data["proxy_name"], "web-proxy");
    assert_eq!(data["publish_port"], 8080);
    assert_eq!(data["retry_count"], 1);
    assert_eq!(data["max_retries"], 10);
}

#[test]
fn test_bind_failed_notification_data() {
    let request = ExceptionNotification {
        level: "error".to_string(),
        message: "代理 'web-proxy' 绑定失败，已达到最大重试次数".to_string(),
        code: Some("PROXY_BIND_FAILED".to_string()),
        data: Some(serde_json::json!({
            "proxy_name": "web-proxy",
            "publish_addr": "0.0.0.0",
            "publish_port": 8080,
            "max_retries": 10,
            "final_error": "Address already in use"
        })),
    };

    assert_eq!(request.level, "error");
    assert_eq!(request.code.unwrap(), "PROXY_BIND_FAILED");
    
    let data = request.data.unwrap();
    assert_eq!(data["max_retries"], 10);
    assert_eq!(data["final_error"], "Address already in use");
}
