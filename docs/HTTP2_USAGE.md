# HTTP/2 传输使用指南

## 概述

HTTP/2 传输模式使用 HTTP/2 CONNECT 方法在客户端和服务器之间建立隧道。相比原生 TLS 传输，HTTP/2 传输有以下优势：

- **更好的防火墙穿透**: 使用标准 HTTP/2 协议，更容易通过企业防火墙
- **多路复用**: 单个 TCP 连接可承载多个逻辑流
- **流量控制**: HTTP/2 内置流量控制机制
- **头部压缩**: HPACK 压缩减少开销

## 配置示例

### 服务器配置 (server-http2.toml)

```toml
[server]
bind_addr = "0.0.0.0"
bind_port = 8443
transport = "http2"  # 使用 HTTP/2 传输
auth_key = "your-secret-key-here"

# 可选：指定证书路径（如果不指定会自动生成）
# cert_path = "/path/to/cert.pem"
# key_path = "/path/to/key.pem"
```

### 客户端配置 (client-http2.toml)

```toml
[client]
server_addr = "your-server.com"
server_port = 8443
transport = "http2"  # 使用 HTTP/2 传输
auth_key = "your-secret-key-here"
skip_verify = false  # 生产环境应该验证证书

# 可选：指定 CA 证书
# ca_cert_path = "/path/to/ca.pem"

# 代理配置示例
[[proxies]]
name = "web-server"
proxy_type = "http/1.1"
publish_addr = "0.0.0.0"
publish_port = 8080
local_port = 80

[[proxies]]
name = "api-server"
proxy_type = "http/2.0"
publish_addr = "0.0.0.0"
publish_port = 8081
local_port = 3000
```

## 运行示例

### 1. 启动服务器

```powershell
# 使用 HTTP/2 传输的服务器
.\tls-tunnel.exe server server-http2.toml
```

### 2. 启动客户端

```powershell
# 使用 HTTP/2 传输的客户端
.\tls-tunnel.exe client client-http2.toml
```

## 工作原理

### 连接建立流程

1. **TCP 连接**: 客户端连接到服务器的 TCP 端口
2. **TLS 握手**: 建立 TLS 加密连接
3. **HTTP/2 握手**: 升级到 HTTP/2 协议
4. **CONNECT 请求**: 客户端发送 HTTP/2 CONNECT 请求
5. **200 OK 响应**: 服务器响应 200 OK 建立隧道
6. **数据传输**: 通过 HTTP/2 流传输隧道数据

### HTTP/2 CONNECT 请求示例

```http
CONNECT / HTTP/2
Host: your-server.com
```

服务器响应：

```http
HTTP/2 200 OK
```

之后，HTTP/2 流就成为了透明的数据传输通道。

## 与 TLS 传输的对比

| 特性 | TLS 传输 | HTTP/2 传输 |
|------|---------|------------|
| 防火墙穿透 | 较差 | 较好 |
| 多路复用 | 需要 Yamux | HTTP/2 原生支持 |
| 性能 | 稍快 | 稍慢（额外协议开销） |
| 兼容性 | 需要直接 TCP | 标准 HTTP/2 |
| 适用场景 | 直连、VPN | 企业网络、代理环境 |

## 注意事项

1. **证书要求**: HTTP/2 传输仍然需要 TLS，因此需要有效的 TLS 证书
2. **端口选择**: 建议使用 443 或 8443 等标准 HTTPS 端口
3. **性能**: HTTP/2 有额外的协议开销，性能略低于原生 TLS
4. **兼容性**: 当前实现与 client.rs/server.rs 的集成尚未完成

## 技术实现

### Http2Stream

`Http2Stream` 封装了 H2 crate 的 `SendStream` 和 `RecvStream`，实现了 Tokio 的 `AsyncRead` 和 `AsyncWrite` trait：

```rust
pub struct Http2Stream {
    send_stream: SendStream<Bytes>,
    recv_stream: RecvStream,
    read_buf: Option<Bytes>,
}
```

- **AsyncRead**: 从 `RecvStream` 读取数据，自动处理流量控制
- **AsyncWrite**: 向 `SendStream` 写入数据，处理容量预留
- **缓冲管理**: 内部缓冲未读完的数据块

### 流量控制

HTTP/2 的流量控制通过窗口更新机制实现：

```rust
// 读取数据后释放流量控制窗口
let _ = self.recv_stream.flow_control().release_capacity(bytes_read);
```

### 多路复用

虽然当前实现每个隧道使用一个 HTTP/2 连接，但 HTTP/2 的多路复用特性已经内置在协议中，未来可以优化为共享连接。

## 故障排查

### 连接失败

- 检查服务器是否支持 HTTP/2
- 确认 TLS 证书有效
- 验证防火墙规则

### 性能问题

- HTTP/2 传输比 TLS 传输慢 5-10% 是正常的
- 调整 TCP 窗口大小可能有帮助
- 考虑使用原生 TLS 传输以获得最佳性能

### 调试

启用详细日志：

```powershell
$env:RUST_LOG="debug"
.\tls-tunnel.exe client client-http2.toml
```

## 下一步

- [ ] 集成到 client.rs 和 server.rs
- [ ] 添加端到端测试
- [ ] 性能基准测试
- [ ] 连接池优化（共享 HTTP/2 连接）
