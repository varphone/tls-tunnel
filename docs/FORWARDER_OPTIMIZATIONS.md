# Forwarder 优化指南

本文档描述了 forwarder 组件的全面优化，包括性能、兼容性和可靠性方面的改进。

## 优化总览

### 第一阶段：性能优化（已完成）

#### 1. 缓冲区优化
- **数据转发缓冲区**：从 8KB 增加到 64KB
  - 减少上下文切换
  - 提高吞吐量（适合高速连接）
  - 适用于 copy_with_stats 函数

- **HTTP 协议解析缓冲区**：设定为 16KB
  - 一次性读取整个 HTTP 请求头
  - 避免逐字节解析的开销
  - 防止超长请求（>16KB 拒绝）

#### 2. HTTP 协议解析优化
- **变更**：从逐字节读取改为缓冲读取
- **性能提升**：减少系统调用次数
- **位置**：`parse_http_request()` 函数

#### 3. 连接空闲超时
- **超时时间**：5 分钟
- **作用**：防止慢速或断开的连接占用资源
- **实现**：在 `copy_with_stats()` 中使用 `tokio::time::timeout`

#### 4. TCP 性能优化
- **TCP_NODELAY**：禁用 Nagle 算法
- **适用**：所有 TCP 连接（本地和远程）
- **效果**：降低 10-20ms RTT 延迟
- **代码**：`stream.set_nodelay(true)?`

### 第二阶段：兼容性优化（已完成）

#### 5. HTTP 直接转发
**功能**：支持非 CONNECT 方法的 HTTP 代理（GET, POST, PUT, DELETE 等）

**工作流程**：
```
客户端 --(HTTP GET/POST)--> Forwarder --(修改后的请求)--> 目标服务器
                                |
                          自动修改请求头：
                          - 移除代理相关 header
                          - 转换绝对 URL 为相对路径
                          - 保留必要的 header
```

**功能细节**：
- 支持绝对 URL：`GET http://example.com/path HTTP/1.1`
- 支持相对路径：`GET /path HTTP/1.1`
- 自动从 Host 头识别目标
- 关闭 Keep-Alive（转换为 Connection: close）

**代码位置**：
- `parse_http_request()` - 完整 HTTP 请求解析
- `handle_http_direct()` - 直接转发请求重建
- `handle_http_connect()` - CONNECT 隧道模式

**使用场景**：
- 传统 HTTP 代理客户端（不支持 CONNECT）
- 旧版浏览器
- 特定的代理工具

#### 6. 连接池缓存
**目的**：减少频繁连接相同目标的握手开销

**实现**：`ConnectionPool` 结构
- 最多缓存 100 个目标的连接
- 连接空闲 5 分钟后自动释放
- 支持异步获取和归还

**使用场景**：
- 多个客户端连接到相同目标
- 长期运行的转发服务
- 连接复用优化

**API**：
```rust
let pool = ConnectionPool::new(100, Duration::from_secs(300));

// 获取或创建连接
let stream = pool.get_or_create("example.com:443").await?;

// 归还连接
pool.return_connection("example.com:443".to_string(), stream).await;

// 清理过期连接
pool.cleanup_expired().await;
```

#### 7. SOCKS5 认证支持
**功能**：RFC 1929 用户名/密码认证

**实现**：`handle_socks5_auth()` 函数

**认证流程**：
1. 客户端发送支持的认证方法
2. 服务器选择认证方法（无认证 0x00 或用户名/密码 0x02）
3. 如果需要认证，进行用户名/密码验证
4. 返回认证结果（0x00 成功，0x01 失败）

**配置**：
```rust
let auth = Socks5Auth {
    username: "user".to_string(),
    password: "pass".to_string(),
};

handle_socks5_auth(stream, Some(&auth)).await?;
```

### 第三阶段：可靠性优化（已完成）

#### 8. 快速失败机制
**功能**：自动黑名单失败的目标

**配置**：
- **失败阈值**：3 次连续失败
- **黑名单超时**：30 分钟
- **清理间隔**：1 分钟

**实现**：`FailedTargetManager` 结构

**工作流程**：
```
连接失败 --> 记录失败 --> 失败计数 >= 3 --> 加入黑名单
                                    |
                            30 分钟后移除
```

**代码位置**：
- `record_failure()` - 记录失败
- `is_blacklisted()` - 检查黑名单
- `cleanup_blacklist()` - 清理过期记录

#### 9. 错误恢复机制
**功能**：指数退避重试

**配置**：`RetryConfig`
- 初始退避：100ms
- 最大退避：10 秒
- 最大重试次数：3

**退避计算**：
```
第 1 次重试：等待 100ms 后重试
第 2 次重试：等待 200ms 后重试
第 3 次重试：等待 400ms 后重试
（最大不超过 10 秒）
```

**使用示例**：
```rust
let config = RetryConfig::default();
let result = retry_with_backoff(
    || async {
        TcpStream::connect("example.com:443").await
    },
    &config,
).await?;
```

**适用场景**：
- 网络波动
- 服务器临时不可用
- DNS 解析失败

## 配置常数汇总

```rust
// 性能相关
const COPY_BUFFER_SIZE: usize = 65536;           // 64KB 数据转发缓冲
const HTTP_PARSE_BUFFER_SIZE: usize = 16384;     // 16KB HTTP 解析缓冲
const CONNECTION_IDLE_TIMEOUT: Duration = Duration::from_secs(5 * 60);  // 5 分钟空闲超时

// 协议相关
const PROTOCOL_PARSE_TIMEOUT: Duration = Duration::from_secs(30);       // 30 秒协议解析超时
const MAX_CONCURRENT_CONNECTIONS: usize = 1000;  // 最大并发连接数

// 可靠性相关
const FAILED_TARGET_THRESHOLD: u32 = 3;                    // 失败阈值
const FAILED_TARGET_TIMEOUT: Duration = Duration::from_secs(30 * 60);  // 黑名单 30 分钟
const FAILED_TARGET_CLEANUP_INTERVAL: Duration = Duration::from_secs(60); // 清理间隔 1 分钟
```

## 性能基准

基于典型使用场景的性能改进估计：

| 优化项 | 改进效果 | 适用场景 |
|--------|--------|--------|
| 缓冲区扩大 | +30-50% 吞吐量 | 高速连接（>100Mbps） |
| HTTP 解析优化 | -20-30% CPU | 频繁 HTTP 请求 |
| 连接池复用 | -50-80% 延迟 | 频繁连接同一目标 |
| TCP_NODELAY | -10-20ms RTT | 所有场景 |
| 连接超时 | 节省内存 | 长期运行服务 |

## 调试和监控

### 统计信息
每个 forwarder 连接的统计：
- `bytes_sent` - 发送字节数
- `bytes_received` - 接收字节数
- `connections_count` - 当前连接数
- `proxy_type` - 代理类型（HttpProxy/Socks5Proxy）

### 日志级别
- `INFO` - 正常连接、成功转发
- `WARN` - 黑名单触发、超时、认证失败
- `ERROR` - 连接失败、协议错误

### 日志示例
```
INFO: Forwarder 'proxy1': Connection from 127.0.0.1 to example.com:443 -> DIRECT
WARN: Forwarder 'proxy1': Target 'unreachable.com:80' is blacklisted due to previous failures
ERROR: Forwarder 'proxy1': Failed to connect directly to 'failed.com:80': connection refused
```

## 最佳实践

### 1. 缓冲区大小选择
- **高速链路**（>100Mbps）：使用 64KB（默认）
- **低速链路**（<10Mbps）：降低至 32KB 以降低延迟
- **内存受限**：最小 8KB

### 2. 超时配置
- **局域网**：协议解析超时可降至 5 秒
- **广域网**：保持 30 秒
- **慢速链路**：增加至 60 秒

### 3. 失败处理
- **关键服务**：黑名单超时设为 5 分钟
- **非关键服务**：可设为 30 分钟
- **自动恢复**：建议启用指数退避重试

### 4. 连接池配置
- **小型部署**：池大小 50，空闲超时 5 分钟
- **中型部署**：池大小 100，空闲超时 10 分钟
- **大型部署**：池大小 500+，定期清理

## 故障排查

### 连接频繁中断
- 检查 `CONNECTION_IDLE_TIMEOUT` 是否过短
- 查看是否频繁触发黑名单

### 内存占用持续增加
- 启用连接池清理：`pool.cleanup_expired().await`
- 检查是否有泄漏的连接

### HTTP 直接转发失败
- 验证目标 URL 格式
- 检查 Host 头是否正确识别
- 查看日志中的 URL 解析错误

### SOCKS5 认证问题
- 验证用户名/密码
- 检查认证协议支持（RFC 1929）
- 查看认证失败日志

## 相关文档

- [ARCHITECTURE.md](ARCHITECTURE.md) - 整体架构设计
- [TRANSPORT_REFACTORING.md](TRANSPORT_REFACTORING.md) - 传输层优化
- [TRANSPORT_COMPARISON.md](TRANSPORT_COMPARISON.md) - 传输方式对比
