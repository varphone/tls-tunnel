# 异常处理和重试机制文档

## 概述

本文档详细说明了 TLS Tunnel 在处理代理绑定失败时的异常处理机制和自动重试策略，以确保系统的可靠性。

## 问题场景

当客户端向服务器发送多个代理配置时，服务器可能在绑定其中某些端口时遇到失败情况：

```
客户端配置: proxy1(8001), proxy2(8002), proxy3(8003)
服务器绑定: 
  ✓ proxy1 → 8001 成功
  ✗ proxy2 → 8002 失败（端口被占用）
  ✓ proxy3 → 8003 成功
```

在这种情况下，系统需要：
1. **继续工作**: proxy1 和 proxy3 正常运行
2. **自动重试**: proxy2 持续尝试绑定
3. **通知客户端**: 向客户端报告 proxy2 的状态变化

## 解决方案

### 1. 协议消息扩展

添加了 `ProxyStatusUpdate` 消息用于实时通知代理状态变化：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyStatusUpdate {
    /// 代理名称
    pub proxy_name: String,
    /// 代理状态
    pub status: ProxyHealthStatus,
    /// 错误信息（如果状态为不健康）
    pub error_message: Option<String>,
    /// 下一次重试的秒数
    pub retry_after_seconds: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyHealthStatus {
    /// 正常工作
    Healthy,
    /// 暂时不可用，正在重试
    Unhealthy,
    /// 绑定端口失败
    BindFailed,
}
```

### 2. 重试机制

#### 重试策略

**指数退避重试算法**:
```
第1次失败: 等待 2 秒后重试
第2次失败: 等待 4 秒后重试
第3次失败: 等待 8 秒后重试
...
第N次失败: 等待 min(2^N, 60) 秒后重试
```

**重试配置**:
```rust
const MAX_BIND_RETRIES: u32 = 10;           // 最多重试10次
const INITIAL_RETRY_DELAY_SECS: u64 = 2;   // 初始延迟 2 秒
const MAX_RETRY_DELAY_SECS: u64 = 60;      // 最大延迟 60 秒
```

#### 重试流程

```
绑定端口
  ↓
成功? → YES → 监听客户端连接
  ↓
   NO
  ↓
重试次数 ≤ 10? → YES → 等待 N 秒后重试
  ↓
   NO
  ↓
记录失败，放弃该代理
```

### 3. 错误分类和诊断

#### 常见错误情况

| 错误类型 | 原因 | 诊断信息 |
|---------|------|--------|
| `AddrInUse` | 端口被占用 | "Port 8080 is already in use by another process" |
| `PermissionDenied` | 权限不足 | "Permission denied to bind to 0.0.0.0:80 - may need administrator privileges" |
| 其他 IO 错误 | 系统错误 | "Failed to bind proxy listener on 0.0.0.0:8080: ..." |

#### 诊断步骤

当代理绑定失败时，系统会：

1. **立即通知**
   ```
   Proxy 'proxy2' bind failed (attempt 1/10): 
   Port 8002 is already in use by another process. Retrying in 2 seconds...
   ```

2. **计划重试**
   ```
   Waiting 2 seconds before retry attempt 2/10...
   ```

3. **成功恢复**
   ```
   Proxy 'proxy2' listening on 0.0.0.0:8002 (after 1 retries)
   ```

### 4. 代码实现

#### 服务器侧实现 (src/server/connection.rs)

```rust
pub async fn start_proxy_listener_with_notify(
    proxy: ProxyInfo,
    stream_tx: mpsc::Sender<(mpsc::Sender<yamux::Stream>, u16, String)>,
    tracker: ProxyStatsTracker,
    _status_tx: Option<mpsc::Sender<ProxyStatusUpdate>>,
) -> Result<()> {
    let mut retry_count = 0;
    let mut retry_delay = INITIAL_RETRY_DELAY_SECS;

    loop {
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => {
                // 绑定成功，开始监听
                return handle_listener_loop(listener, proxy, stream_tx, tracker).await;
            }
            Err(e) => {
                retry_count += 1;
                
                if retry_count <= MAX_BIND_RETRIES {
                    // 计算错误信息
                    let error_msg = format_error_message(e);
                    
                    warn!(
                        "Proxy '{}' bind failed (attempt {}/{}): {}. Retrying in {} seconds...",
                        proxy_name, retry_count, MAX_BIND_RETRIES, error_msg, retry_delay
                    );

                    // 等待后重试
                    sleep(Duration::from_secs(retry_delay)).await;
                    
                    // 指数退避
                    retry_delay = std::cmp::min(retry_delay * 2, MAX_RETRY_DELAY_SECS);
                } else {
                    // 重试次数已尽，放弃
                    error!(
                        "Proxy '{}' bind failed after {} retries",
                        proxy_name, MAX_BIND_RETRIES
                    );
                    return Err(anyhow::anyhow!("Failed to bind after {} retries", MAX_BIND_RETRIES));
                }
            }
        }
    }
}
```

### 5. 客户端处理

虽然当前版本中客户端还不接收实时状态更新，但协议已为此预留扩展机制。

未来客户端将能够：
1. 接收代理状态更新消息
2. 实时展示每个代理的绑定状态
3. 提示用户哪些代理正在重试

### 6. 日志示例

#### 成功场景
```
[INFO] Proxy 'proxy1' listening on 0.0.0.0:8001
[INFO] Proxy 'proxy2' listening on 0.0.0.0:8002
[INFO] Proxy 'proxy3' listening on 0.0.0.0:8003
```

#### 失败后恢复场景
```
[WARN] Proxy 'proxy2' bind failed (attempt 1/10): Port 8002 is already in use. Retrying in 2 seconds...
[WARN] Proxy 'proxy2' bind failed (attempt 2/10): Port 8002 is already in use. Retrying in 4 seconds...
[INFO] Proxy 'proxy2' listening on 0.0.0.0:8002 (after 2 retries)
```

#### 最终失败场景
```
[ERROR] Proxy 'proxy2' bind failed (attempt 10/10): Port 8002 is already in use. Retrying in 60 seconds...
[ERROR] Proxy 'proxy2' bind failed after 10 retries: Port 8002 is already in use
[ERROR] Proxy listener error: Failed to bind proxy 'proxy2' after 10 retries
```

## 系统行为

### 部分成功场景

```
客户端请求:
  ✓ Proxy A - 绑定成功 → 立即可用
  ✗ Proxy B - 绑定失败 → 开始重试
  ✓ Proxy C - 绑定成功 → 立即可用

系统状态:
  A: Running
  B: Retrying... (attempt 1/10, next retry in 2s)
  C: Running

用户可以:
  - 通过 Proxy A 和 C 访问内网资源
  - 等待 Proxy B 绑定成功（若端口最终被释放）
  - 手动解决端口占用问题
```

### 最大重试次数设计

为什么选择 10 次重试？

- **下限**: 至少给予足够的重试机会（如系统临时资源占用）
- **上限**: 防止无限等待（最终超时 = 2+4+8+16+32+60+60+60+60+60 = 402 秒 ≈ 7 分钟）
- **权衡**: 给用户足够时间解决问题，但不会永久阻挂系统

## 客户端重新连接处理

如果客户端在某个代理绑定失败期间断开连接，重新连接时：

1. **新连接请求**: 客户端重新发送所有代理配置
2. **全新绑定**: 服务器对所有代理进行绑定
3. **独立重试**: 每条新连接的重试计数独立
4. **状态重置**: 之前的重试历史不保留

## 未来改进

### 短期计划
- [ ] 向客户端发送实时代理状态更新
- [ ] 在客户端显示代理绑定状态
- [ ] 支持配置自定义重试策略

### 中期计划
- [ ] 支持故障通知（邮件/Webhook）
- [ ] 统计重试次数和成功率
- [ ] 自动清理僵尸代理

### 长期计划
- [ ] 代理健康检查机制
- [ ] 动态端口分配
- [ ] 负载均衡和故障转移

## 测试场景

### 测试 1: 端口暂时被占用

```bash
# 终端1: 占用端口
nc -l 0.0.0.0 8080

# 终端2: 启动服务器（会重试）
tls-tunnel server

# 终端3: 在几秒后释放端口
^C  # 按 Ctrl+C 停止 nc

# 预期: 服务器继续重试，最终绑定成功
```

### 测试 2: 权限不足

```bash
# 尝试绑定特权端口（非root/admin）
# 配置端口号 < 1024（如 80, 443）

# 预期: 错误提示权限问题，重试但最终失败
```

### 测试 3: 多个代理混合成功/失败

```toml
# 配置文件
[[proxies]]
name = "proxy1"
publish_addr = "0.0.0.0"
publish_port = 8001  # 可用

[[proxies]]
name = "proxy2"
publish_addr = "0.0.0.0"
publish_port = 22    # 可能被占用（SSH）

[[proxies]]
name = "proxy3"
publish_addr = "0.0.0.0"
publish_port = 8003  # 可用
```

## 参考资源

- [Protocol.rs](../../src/protocol.rs) - ProxyStatusUpdate 定义
- [Connection.rs](../../src/server/connection.rs) - 重试机制实现
- [Server/mod.rs](../../src/server/mod.rs) - 代理启动逻辑
