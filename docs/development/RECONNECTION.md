# 客户端重连机制

本文档说明 TLS Tunnel 如何处理客户端断开和重连。

## 工作流程

### 1. 初次连接

```
客户端                       服务器
  │                           │
  ├──────TLS连接─────────────►│
  ├──────认证密钥────────────►│
  │◄─────认证成功─────────────┤
  ├──────代理配置────────────►│
  │◄─────配置确认─────────────┤
  ├──────Yamux连接───────────►│
  │                           │
  │                     [启动代理监听器]
  │                       - 8080 → 3000
  │                       - 22 → 22
  │                       - 5432 → 5432
```

**服务器端行为：**
- 为每个代理启动独立的监听器任务
- 所有监听器通过 `JoinSet` 统一管理
- 每个监听器订阅 shutdown 信号

### 2. 客户端断开

当客户端断开连接时（网络故障、进程终止、Ctrl+C等）：

```
客户端                       服务器
  │                           │
  X  断开连接                 │
                              │
                        [检测到连接断开]
                              │
                        [发送shutdown信号]
                              │
                      ┌───────┴───────┐
                      │ 代理监听器1    │
                      │ (8080→3000)   │
                      │ ✓ 收到信号     │
                      │ ✓ 停止监听     │
                      │ ✓ 释放端口     │
                      └───────────────┘
                      ┌───────────────┐
                      │ 代理监听器2    │
                      │ (22→22)       │
                      │ ✓ 收到信号     │
                      │ ✓ 停止监听     │
                      │ ✓ 释放端口     │
                      └───────────────┘
                      ┌───────────────┐
                      │ 代理监听器3    │
                      │ (5432→5432)   │
                      │ ✓ 收到信号     │
                      │ ✓ 停止监听     │
                      │ ✓ 释放端口     │
                      └───────────────┘
                              │
                      [等待所有任务完成]
                              │
                      [清理连接资源]
```

**关键行为：**
1. Yamux 连接检测到断开
2. 通过 `broadcast channel` 发送 shutdown 信号
3. 所有代理监听器收到信号后停止 `accept()` 循环
4. 释放所有绑定的端口
5. `JoinSet` 等待所有任务完成
6. 清理客户端相关的所有资源

### 3. 客户端重连

```
客户端                       服务器
  │                           │
  ├──────TLS连接─────────────►│ [端口已释放]
  ├──────认证密钥────────────►│
  │◄─────认证成功─────────────┤
  ├──────代理配置────────────►│ [重新验证配置]
  │◄─────配置确认─────────────┤
  ├──────Yamux连接───────────►│
  │                           │
  │                     [重新启动监听器]
  │                       ✓ 8080 → 3000
  │                       ✓ 22 → 22
  │                       ✓ 5432 → 5432
```

**重连成功的原因：**
- 之前的监听器已经完全关闭
- 端口已经释放
- 可以重新绑定相同的端口

## 技术实现

### 使用 `JoinSet` 管理任务

```rust
let mut listener_tasks = tokio::task::JoinSet::new();

for proxy in proxies {
    listener_tasks.spawn(async move {
        // 监听器任务
    });
}

// 等待所有任务完成或收到关闭信号
listener_tasks.shutdown().await;
```

**优势：**
- 统一管理所有代理监听器任务
- 可以批量取消所有任务
- 等待所有任务完成后再清理

### 使用 `broadcast channel` 发送关闭信号

```rust
// 创建 broadcast channel
let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);

// 每个监听器订阅信号
let mut shutdown_signal = shutdown_tx.subscribe();

// 监听器使用 select! 等待信号
tokio::select! {
    result = start_proxy_listener(...) => { }
    _ = shutdown_signal.recv() => {
        info!("Shutting down...");
    }
}
```

**优势：**
- 一个信号可以通知多个接收者
- 非阻塞式通知
- 可靠的关闭机制

### 监控 Yamux 连接状态

```rust
tokio::spawn(async move {
    let result = run_yamux_connection(yamux_conn, stream_rx).await;
    // 连接断开时发送关闭信号
    let _ = shutdown_tx.send(());
});
```

**工作原理：**
- Yamux 连接的 poll 循环检测底层 TCP 断开
- 当检测到断开时，任务结束
- 发送 broadcast 信号通知所有监听器

## 客户端自动重连

客户端具有自动重连功能：

```rust
loop {
    match connect_and_run(&config).await {
        Ok(_) => {
            info!("Connection closed normally");
        }
        Err(e) => {
            error!("Connection error: {}", e);
        }
    }
    
    let delay = get_reconnect_delay();
    info!("Reconnecting in {} seconds...", delay);
    tokio::time::sleep(Duration::from_secs(delay)).await;
}
```

**重连策略：**
- 默认延迟：5秒（可通过环境变量 `TLS_TUNNEL_RECONNECT_DELAY_SECS` 配置）
- 无限重试
- 每次重连都会重新建立完整的握手流程

## 常见场景

### 场景 1：网络暂时中断

1. 客户端网络断开
2. 服务器检测到连接断开，清理资源
3. 客户端网络恢复
4. 客户端自动重连（5秒后）
5. 重新建立连接和代理

### 场景 2：客户端重启

1. 客户端进程终止（Ctrl+C 或崩溃）
2. 服务器检测到断开，清理资源
3. 手动重启客户端
4. 立即重新连接成功

### 场景 3：修改代理配置后重启

1. 修改 `client.toml` 添加新代理
2. 重启客户端
3. 服务器清理旧的监听器
4. 启动新的监听器（包括新增的代理）

### 场景 4：多客户端场景

如果有多个客户端连接到同一服务器：

- 每个客户端有独立的代理配置
- 代理端口不能冲突（由验证机制确保）
- 客户端 A 断开不影响客户端 B
- 每个客户端的资源独立清理

## 故障排除

### 问题：重连后提示端口已被占用

**可能原因：**
- 监听器未正常关闭
- 系统端口释放延迟

**解决方法：**
```bash
# 检查端口占用（Linux）
sudo netstat -tulpn | grep :8080

# 检查端口占用（Windows）
netstat -ano | findstr :8080

# 如果确认是僵尸进程，手动终止
kill <PID>  # Linux
taskkill /PID <PID> /F  # Windows
```

### 问题：客户端一直重连失败

**可能原因：**
- 服务器未运行
- 网络不通
- 认证密钥不匹配
- 代理配置验证失败

**检查步骤：**
1. 确认服务器正在运行：`ps aux | grep tls-tunnel`
2. 测试网络连通性：`telnet server-ip 8443`
3. 检查日志：查看服务器和客户端的错误信息
4. 验证配置：`tls-tunnel check -c client.toml`

## 最佳实践

### 1. 设置合理的重连延迟

```bash
# 开发环境：快速重连
export TLS_TUNNEL_RECONNECT_DELAY_SECS=2

# 生产环境：避免过度重连
export TLS_TUNNEL_RECONNECT_DELAY_SECS=10
```

### 2. 监控连接状态

- 观察服务器日志中的 "Client connected" 和 "Client disconnected"
- 观察客户端日志中的重连尝试

### 3. 优雅关闭

使用 Ctrl+C 而不是 `kill -9`，确保资源正常清理。

### 4. 配置健康检查

在关键应用中，可以添加健康检查脚本：

```bash
#!/bin/bash
while true; do
    if ! curl -s http://localhost:8080 > /dev/null; then
        echo "Tunnel down, checking client..."
        # 重启客户端或发送告警
    fi
    sleep 30
done
```

## 性能考虑

### 资源清理时间

- 典型清理时间：< 100ms
- 包含：关闭所有监听器、等待任务完成、释放端口

### 重连开销

- TLS 握手：~50-200ms
- 认证和配置交换：~10-50ms
- Yamux 连接建立：~10ms
- 总计：~100-300ms

### 内存占用

- 每个监听器：~8KB
- Yamux 连接：~16KB
- 每个活动流：~4KB

## 相关文档

- [架构设计](ARCHITECTURE.md)
- [协议说明](PROTOCOL.md)
- [测试指南](TESTING.md)
