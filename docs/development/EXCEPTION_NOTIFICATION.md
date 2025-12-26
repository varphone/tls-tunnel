# 异常通知功能

## 概述

服务端可以通过控制通道向客户端发送异常通知，客户端会根据异常级别输出相应的日志。

## 功能特性

- **实时推送**：服务端可以随时向客户端推送异常信息
- **分级处理**：支持三个级别的通知（error、warning、info）
- **结构化数据**：支持附加错误代码和额外数据
- **自动日志**：客户端自动根据级别输出对应的日志

## 异常级别

| 级别 | 说明 | 客户端日志级别 |
|------|------|--------------|
| `error` | 严重错误 | ERROR |
| `warning` | 警告信息 | WARN |
| `info` | 一般通知 | INFO |

## 使用方法

### 服务端发送异常通知

在服务端代码中，通过 `ServerControlChannel` 的 `send_exception_notification` 方法发送通知：

```rust
use crate::server::control_channel::ServerControlChannel;

// 发送错误通知
control_channel.send_exception_notification(
    stream,
    "error",
    "配置验证失败：端口冲突".to_string(),
    Some("CONFIG_VALIDATION_ERROR".to_string()),
    Some(json!({
        "conflicting_ports": [8080, 8081],
        "proxy_name": "web-proxy"
    }))
).await?;

// 发送警告通知
control_channel.send_exception_notification(
    stream,
    "warning",
    "代理连接数接近限制".to_string(),
    Some("CONN_LIMIT_WARNING".to_string()),
    Some(json!({
        "current": 950,
        "limit": 1000
    }))
).await?;

// 发送信息通知
control_channel.send_exception_notification(
    stream,
    "info",
    "配置已更新".to_string(),
    None,
    None
).await?;
```

### 客户端接收处理

客户端的 `ClientControlChannel::handle_notification` 方法会自动处理 `push_exception` 通知：

- **错误级别**：输出 `error!` 日志
- **警告级别**：输出 `warn!` 日志
- **其他级别**：输出 `info!` 日志

日志格式示例：

```
[ERROR] Server exception: CONFIG_VALIDATION_ERROR 配置验证失败：端口冲突
[ERROR] Exception data: {"conflicting_ports":[8080,8081],"proxy_name":"web-proxy"}

[WARN] Server warning: CONN_LIMIT_WARNING 代理连接数接近限制
[WARN] Warning data: {"current":950,"limit":1000}

[INFO] Server notification:  配置已更新
```

## 使用场景

### 1. 配置错误

```rust
// 当客户端提交的配置有问题时
control_channel.send_exception_notification(
    stream,
    "error",
    format!("代理配置 '{}' 无效：{}", proxy_name, error_msg),
    Some("INVALID_PROXY_CONFIG".to_string()),
    Some(json!({"proxy_name": proxy_name}))
).await?;
```

### 2. 资源限制警告

```rust
// 当资源使用接近限制时
control_channel.send_exception_notification(
    stream,
    "warning",
    "带宽使用率超过 80%".to_string(),
    Some("BANDWIDTH_WARNING".to_string()),
    Some(json!({
        "usage_mbps": 800,
        "limit_mbps": 1000,
        "percentage": 80
    }))
).await?;
```

### 3. 连接异常

```rust
// 当检测到连接异常时
control_channel.send_exception_notification(
    stream,
    "error",
    "目标服务器连接失败".to_string(),
    Some("TARGET_CONNECTION_FAILED".to_string()),
    Some(json!({
        "target_host": "backend.example.com",
        "target_port": 8080,
        "error": "Connection refused"
    }))
).await?;
```

### 4. 状态更新通知

```rust
// 通知客户端状态变化
control_channel.send_exception_notification(
    stream,
    "info",
    "代理服务已启动".to_string(),
    Some("PROXY_STARTED".to_string()),
    Some(json!({"proxy_name": proxy_name}))
).await?;
```

## 协议细节

### 通知格式

异常通知使用 JSON-RPC 2.0 通知格式（无 `id` 字段）：

```json
{
  "jsonrpc": "2.0",
  "method": "push_exception",
  "params": {
    "level": "error",
    "message": "配置验证失败：端口冲突",
    "code": "CONFIG_VALIDATION_ERROR",
    "data": {
      "conflicting_ports": [8080, 8081],
      "proxy_name": "web-proxy"
    }
  }
}
```

### 参数说明

| 字段 | 类型 | 必需 | 说明 |
|------|------|------|------|
| `level` | string | 是 | 异常级别：error/warning/info |
| `message` | string | 是 | 异常消息 |
| `code` | string | 否 | 异常代码（用于程序化处理） |
| `data` | object | 否 | 附加数据（任意 JSON 对象） |

## 最佳实践

1. **使用错误代码**：为常见错误定义标准错误代码，便于客户端程序化处理
2. **提供详细信息**：在 `data` 字段中包含足够的上下文信息用于调试
3. **合理分级**：
   - `error`：需要用户立即关注的问题
   - `warning`：潜在问题或需要注意的状态
   - `info`：一般性通知或状态更新
4. **避免滥用**：不要发送过于频繁的通知，避免日志刷屏

## 示例：在连接失败时发送通知

```rust
// 在服务端连接处理代码中
async fn handle_proxy_connection(
    control_channel: &ServerControlChannel,
    control_stream: &mut yamux::Stream,
    proxy_config: &ProxyConfig,
) -> Result<()> {
    // 尝试连接目标服务器
    match connect_to_target(&proxy_config.local_addr, proxy_config.local_port).await {
        Ok(stream) => {
            // 连接成功，继续处理...
            Ok(())
        }
        Err(e) => {
            // 连接失败，发送异常通知
            control_channel.send_exception_notification(
                control_stream,
                "error",
                format!(
                    "无法连接到目标服务器 {}:{}",
                    proxy_config.local_addr,
                    proxy_config.local_port
                ),
                Some("TARGET_CONNECTION_FAILED".to_string()),
                Some(json!({
                    "proxy_name": &proxy_config.name,
                    "target_addr": &proxy_config.local_addr,
                    "target_port": proxy_config.local_port,
                    "error": e.to_string()
                }))
            ).await?;
            
            Err(e)
        }
    }
}
```

## 客户端日志示例

当上述异常发送后，客户端会输出：

```
2025-12-27T10:30:45.123Z ERROR tls_tunnel::client::control_channel: Server exception: TARGET_CONNECTION_FAILED 无法连接到目标服务器 127.0.0.1:8080
2025-12-27T10:30:45.123Z ERROR tls_tunnel::client::control_channel: Exception data: {"proxy_name":"web-proxy","target_addr":"127.0.0.1","target_port":8080,"error":"Connection refused (os error 111)"}
```
