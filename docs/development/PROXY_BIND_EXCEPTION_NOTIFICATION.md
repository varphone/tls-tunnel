# 代理监听器绑定异常通知功能

## 概述

重构了代理监听器的绑定逻辑，在绑定失败时通过控制通道向客户端发送实时异常通知。

## 实现时间
2025-12-27

## 功能说明

### 1. 异常通知机制

当服务端尝试绑定代理监听端口失败时（如端口被占用、权限不足等），会通过控制通道向对应的客户端发送异常通知：

- **警告通知**：在重试期间发送，告知客户端当前的重试状态
- **错误通知**：达到最大重试次数后发送，告知客户端绑定最终失败

### 2. 架构设计

#### 异常通知流程

```
代理监听器 (Tokio Task)
    ↓ 绑定失败
    ↓ 发送异常通知 (ExceptionNotification)
    ↓ 通过 mpsc::unbounded_channel
    ↓
服务器事件循环 (run_server_event_loop)
    ↓ 接收异常通知
    ↓ 调用 control_channel.send_exception_notification()
    ↓
控制流 (yamux::Stream)
    ↓ JSON-RPC 通知
    ↓
客户端 (ClientControlChannel)
    ↓ 接收并处理通知
    ↓ 输出日志 (error!/warn!)
```

#### 关键组件

1. **ExceptionNotification** (src/server/connection.rs)
   - 异常通知结构
   - 包含级别、消息、代码和附加数据

2. **ServerWorld.exception_tx/rx** (src/server/mod.rs)
   - 异常通知通道，连接代理监听器和事件循环
   - 使用 unbounded channel 避免阻塞

3. **事件循环集成** (src/server/mod.rs)
   - 在 `tokio::select!` 中添加异常通知分支
   - 自动转发异常通知到控制通道

## 代码变更

### 1. src/server/connection.rs

#### 新增结构体
```rust
pub struct ExceptionNotification {
    pub level: String,
    pub message: String,
    pub code: Option<String>,
    pub data: Option<serde_json::Value>,
}
```

#### 修改函数签名
```rust
pub async fn start_proxy_listener_with_notify(
    proxy: ProxyInfo,
    stream_tx: mpsc::Sender<(mpsc::Sender<yamux::Stream>, u16, String)>,
    tracker: ProxyStatsTracker,
    exception_tx: Option<mpsc::UnboundedSender<ExceptionNotification>>,
) -> Result<()>
```

#### 绑定失败时发送通知

**重试期间 - 警告通知**：
```rust
if let Some(ref tx) = exception_tx {
    let _ = tx.send(ExceptionNotification {
        level: "warning".to_string(),
        message: format!("代理 '{}' 绑定失败 (尝试 {}/{}): {}. 将在 {} 秒后重试",
            proxy_name, retry_count, MAX_BIND_RETRIES, error_msg, retry_delay),
        code: Some("PROXY_BIND_RETRY".to_string()),
        data: Some(serde_json::json!({
            "proxy_name": proxy_name,
            "publish_addr": proxy.publish_addr,
            "publish_port": proxy.publish_port,
            "retry_count": retry_count,
            "max_retries": MAX_BIND_RETRIES,
            "retry_delay_secs": retry_delay,
            "error": error_msg
        })),
    });
}
```

**最终失败 - 错误通知**：
```rust
if let Some(ref tx) = exception_tx {
    let _ = tx.send(ExceptionNotification {
        level: "error".to_string(),
        message: format!("代理 '{}' 绑定失败，已达到最大重试次数 ({})",
            proxy_name, MAX_BIND_RETRIES),
        code: Some("PROXY_BIND_FAILED".to_string()),
        data: Some(serde_json::json!({
            "proxy_name": proxy_name,
            "publish_addr": proxy.publish_addr,
            "publish_port": proxy.publish_port,
            "max_retries": MAX_BIND_RETRIES,
            "final_error": error_msg
        })),
    });
}
```

### 2. src/server/mod.rs

#### ServerWorld 结构体扩展
```rust
struct ServerWorld {
    // ... 现有字段 ...
    exception_tx: mpsc::UnboundedSender<connection::ExceptionNotification>,
    exception_rx: mpsc::UnboundedReceiver<connection::ExceptionNotification>,
}
```

#### 创建异常通知通道
```rust
let (exception_tx, exception_rx) = mpsc::unbounded_channel();

let world = ServerWorld {
    // ... 现有字段 ...
    exception_tx,
    exception_rx,
};
```

#### 启动代理监听器时传递通道
```rust
let exception_tx = world.exception_tx.clone();

tokio::spawn(async move {
    tokio::select! {
        result = start_proxy_listener_with_notify(
            proxy_info,
            stream_tx_clone,
            tracker,
            Some(exception_tx)
        ) => {
            // ...
        }
    }
});
```

#### 事件循环中处理异常通知
```rust
loop {
    tokio::select! {
        // ... 现有分支 ...

        // 5. 处理异常通知（从代理监听器发送过来的）
        Some(exception_req) = world.exception_rx.recv() => {
            if let Err(e) = control_channel
                .send_exception_notification(
                    &mut control_stream,
                    &exception_req.level,
                    exception_req.message,
                    exception_req.code,
                    exception_req.data,
                )
                .await
            {
                warn!("Failed to send exception notification: {}", e);
            }
        }
    }
}
```

## 使用场景

### 场景 1：端口被占用

服务端日志：
```
WARN Proxy 'web-proxy' bind failed (attempt 1/10): Port 8080 is already in use. Retrying in 2 seconds...
```

客户端日志：
```
WARN Server warning: PROXY_BIND_RETRY 代理 'web-proxy' 绑定失败 (尝试 1/10): Port 8080 is already in use. 将在 2 秒后重试
WARN Warning data: {"proxy_name":"web-proxy","publish_addr":"0.0.0.0","publish_port":8080,"retry_count":1,"max_retries":10,"retry_delay_secs":2,"error":"Port 8080 is already in use"}
```

### 场景 2：权限不足

服务端日志：
```
WARN Proxy 'privileged-proxy' bind failed (attempt 1/10): Permission denied to bind to 0.0.0.0:80. Retrying in 2 seconds...
```

客户端日志：
```
WARN Server warning: PROXY_BIND_RETRY 代理 'privileged-proxy' 绑定失败 (尝试 1/10): Permission denied to bind to 0.0.0.0:80. 将在 2 秒后重试
WARN Warning data: {"proxy_name":"privileged-proxy","publish_addr":"0.0.0.0","publish_port":80,...}
```

### 场景 3：达到最大重试次数

服务端日志：
```
ERROR Proxy 'web-proxy' bind failed after 10 retries: Port 8080 is already in use
```

客户端日志：
```
ERROR Server exception: PROXY_BIND_FAILED 代理 'web-proxy' 绑定失败，已达到最大重试次数 (10)
ERROR Exception data: {"proxy_name":"web-proxy","publish_addr":"0.0.0.0","publish_port":8080,"max_retries":10,"final_error":"Port 8080 is already in use"}
```

## 技术优势

### 1. 解耦设计
- 代理监听器和控制通道完全解耦
- 通过消息通道通信，避免直接依赖
- 易于测试和维护

### 2. 非阻塞
- 使用 unbounded channel，不会阻塞代理监听器
- 发送失败时静默丢弃，不影响重试逻辑

### 3. 实时反馈
- 客户端可以立即看到服务端的绑定状态
- 无需轮询或等待超时
- 提升用户体验

### 4. 详细诊断信息
- 包含完整的错误上下文
- 重试次数、延迟时间等元数据
- 便于问题排查

## 测试验证

### 编译检查
```bash
cargo check
✓ 编译通过
```

### Clippy 检查
```bash
cargo clippy --all-targets --all-features
✓ 无警告无错误
```

### 单元测试
```bash
cargo test --lib
✓ 68 tests passed
```

## 后续改进建议

1. **重试策略可配置**
   - 最大重试次数
   - 初始延迟时间
   - 退避策略

2. **更细粒度的错误分类**
   - 临时错误（可重试）
   - 永久错误（不可重试）
   - 根据错误类型调整策略

3. **通知合并**
   - 短时间内多次失败时合并通知
   - 避免日志刷屏

4. **通知优先级**
   - 关键错误立即发送
   - 一般警告批量发送
   - 优化网络使用

## 相关文档

- [异常通知功能文档](EXCEPTION_NOTIFICATION.md)
- [异常通知实现总结](EXCEPTION_NOTIFICATION_IMPLEMENTATION.md)
- [协议兼容性分析](../PROTOCOL_COMPATIBILITY_ANALYSIS.md)
