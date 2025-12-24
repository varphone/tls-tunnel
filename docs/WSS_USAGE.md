# WebSocket 传输使用指南

## 概述

WebSocket Secure (WSS) 传输模式使用 WebSocket 协议在客户端和服务器之间建立隧道。相比原生 TLS 传输，WebSocket 传输有以下优势：

- **最佳的防火墙穿透**: WebSocket 是标准的 Web 协议，几乎所有防火墙都允许
- **HTTP/HTTPS 兼容**: 可以通过标准 HTTP 代理
- **与 Web 服务共存**: 可以与普通 Web 服务器共享端口（通过路径区分）
- **实时双向通信**: 原生支持全双工通信

## 配置示例

### 服务器配置 (server-wss.toml)

```toml
[server]
bind_addr = "0.0.0.0"
bind_port = 443  # 使用标准 HTTPS 端口
transport = "wss"  # 使用 WebSocket 传输
auth_key = "your-secret-key-here"

# 可选：指定证书路径（如果不指定会自动生成）
# cert_path = "/path/to/cert.pem"
# key_path = "/path/to/key.pem"
```

### 客户端配置 (client-wss.toml)

```toml
[client]
server_addr = "your-server.com"
server_port = 443
transport = "wss"  # 使用 WebSocket 传输
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
name = "ssh-server"
proxy_type = "tcp"
publish_addr = "0.0.0.0"
publish_port = 2222
local_port = 22
```

## 运行示例

### 1. 启动服务器

```powershell
# 使用 WebSocket 传输的服务器
.\tls-tunnel.exe server server-wss.toml
```

### 2. 启动客户端

```powershell
# 使用 WebSocket 传输的客户端
.\tls-tunnel.exe client client-wss.toml
```

## 工作原理

### 连接建立流程

1. **TCP 连接**: 客户端连接到服务器的 TCP 端口
2. **TLS 握手**: 建立 TLS 加密连接
3. **HTTP 升级**: 客户端发送 WebSocket 升级请求
4. **WebSocket 握手**: 服务器响应 101 Switching Protocols
5. **数据传输**: 通过 WebSocket 二进制帧传输隧道数据

### WebSocket 升级请求示例

客户端发送：

```http
GET / HTTP/1.1
Host: your-server.com
Upgrade: websocket
Connection: Upgrade
Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==
Sec-WebSocket-Version: 13
```

服务器响应：

```http
HTTP/1.1 101 Switching Protocols
Upgrade: websocket
Connection: Upgrade
Sec-WebSocket-Accept: s3pPLMBiTxaQ9kYGzzhZRbK+xOo=
```

之后，连接切换到 WebSocket 协议，使用二进制帧传输数据。

## WebSocket 帧结构

WebSocket 使用二进制帧传输隧道数据：

```
 0                   1                   2                   3
 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
+-+-+-+-+-------+-+-------------+-------------------------------+
|F|R|R|R| opcode|M| Payload len |    Extended payload length    |
|I|S|S|S|  (4)  |A|     (7)     |             (16/64)           |
|N|V|V|V|       |S|             |   (if payload len==126/127)   |
| |1|2|3|       |K|             |                               |
+-+-+-+-+-------+-+-------------+ - - - - - - - - - - - - - - - +
|     Extended payload length continued, if payload len == 127  |
+ - - - - - - - - - - - - - - - +-------------------------------+
|                               |Masking-key, if MASK set to 1  |
+-------------------------------+-------------------------------+
| Masking-key (continued)       |          Payload Data         |
+-------------------------------- - - - - - - - - - - - - - - - +
:                     Payload Data continued ...                :
+ - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - - +
|                     Payload Data continued ...                |
+---------------------------------------------------------------+
```

- **FIN**: 1 表示这是消息的最后一个帧
- **Opcode**: 0x2 表示二进制帧
- **MASK**: 客户端到服务器的帧必须掩码
- **Payload**: 实际的隧道数据

## 与其他传输方式的对比

| 特性 | TLS 传输 | HTTP/2 传输 | WebSocket 传输 |
|------|---------|------------|---------------|
| 防火墙穿透 | 较差 | 较好 | 最好 |
| HTTP 代理支持 | 无 | 有（CONNECT） | 有（WebSocket） |
| 协议开销 | 最小 | 中等 | 中等 |
| 性能 | 最快 | 中等 | 中等 |
| 端口共享 | 否 | 否 | 是（路径区分） |
| 浏览器支持 | 否 | 是（HTTP/2） | 是（WebSocket） |
| 适用场景 | 直连、VPN | 企业网络 | 严格防火墙环境 |

## WebSocket 的优势

### 1. 最佳的防火墙穿透能力

WebSocket 是标准的 Web 协议，几乎所有企业防火墙都允许 WebSocket 连接（通过 443 端口）。

### 2. HTTP 代理兼容

可以通过 HTTP CONNECT 代理建立 WebSocket 连接：

```http
CONNECT your-server.com:443 HTTP/1.1
Host: your-server.com

HTTP/1.1 200 Connection Established
```

然后在建立的 TLS 连接上进行 WebSocket 握手。

### 3. 与 Web 服务共存

可以通过路径区分 WebSocket 连接和普通 HTTP 请求：

```
https://your-server.com/         -> Web 服务
wss://your-server.com/tunnel     -> 隧道连接
```

这样可以在同一个端口上同时运行 Web 服务器和隧道服务器。

## 技术实现

### WssStream 泛型包装器

`WssStream<S>` 是一个泛型结构，可以包装任何实现了 `AsyncRead + AsyncWrite` 的流：

```rust
pub struct WssStream<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    ws_stream: WebSocketStream<S>,
    read_buf: Vec<u8>,
    read_pos: usize,
}
```

- **泛型设计**: 支持不同的底层流（TLS、TCP 等）
- **缓冲管理**: 处理 WebSocket 消息和字节流的转换
- **消息过滤**: 自动过滤 Ping/Pong 等控制消息

### 数据帧处理

WebSocket 传输只使用二进制帧（Binary frames）：

```rust
// 写入数据
let msg = Message::Binary(buf.to_vec());
ws_stream.send(msg).await?;

// 读取数据
match ws_stream.next().await {
    Some(Ok(Message::Binary(data))) => {
        // 处理数据
    }
    _ => { /* 忽略其他类型 */ }
}
```

### Ping/Pong 自动处理

tokio-tungstenite 库自动处理 Ping/Pong 消息，保持连接活跃：

- 收到 Ping → 自动回复 Pong
- 定期发送 Ping → 检测连接状态

## 注意事项

1. **证书要求**: WebSocket 传输使用 WSS（WebSocket Secure），需要有效的 TLS 证书
2. **端口选择**: 建议使用 443 端口，以获得最佳的防火墙穿透能力
3. **性能**: WebSocket 有消息帧开销，性能略低于原生 TLS（约 5-15%）
4. **兼容性**: 当前实现与 client.rs/server.rs 的集成尚未完成

## 性能优化

### 1. 帧大小

WebSocket 消息大小影响性能：

- 小消息：开销高，延迟低
- 大消息：开销低，延迟高

当前实现使用应用层提供的缓冲区大小，自动平衡性能。

### 2. 缓冲策略

WssStream 内部维护读缓冲区：

```rust
read_buf: Vec<u8>,  // 未读完的数据
read_pos: usize,    // 读取位置
```

这样可以高效处理跨帧的数据读取。

### 3. 连接复用

虽然当前每个隧道使用单独的 WebSocket 连接，但可以在未来优化为：

- 单个 WebSocket 连接承载多个逻辑流
- 使用自定义的多路复用协议
- 减少连接数和握手开销

## 故障排查

### 连接失败

- 检查服务器是否支持 WebSocket
- 确认 TLS 证书有效
- 验证防火墙规则（允许 443 端口）
- 检查 HTTP 代理配置

### 握手失败

WebSocket 握手可能失败的原因：

1. **证书问题**: 使用 `skip_verify = true` 测试
2. **路径不匹配**: 客户端和服务器使用相同的路径
3. **协议版本**: 确保使用 WebSocket 13

### 性能问题

- WebSocket 比 TLS 慢 5-15% 是正常的
- 大量小消息会增加帧开销
- 考虑调整应用层缓冲区大小

### 调试

启用详细日志：

```powershell
$env:RUST_LOG="debug"
.\tls-tunnel.exe client client-wss.toml
```

查看 WebSocket 握手和消息传输日志。

## 高级用法

### 通过 HTTP 代理连接

WebSocket 可以通过 HTTP CONNECT 代理：

```toml
[client]
server_addr = "your-server.com"
server_port = 443
transport = "wss"
# 未来可能支持的代理配置
# proxy = "http://proxy.company.com:8080"
```

### 自定义 WebSocket 路径

未来可能支持自定义 WebSocket 路径：

```toml
[server]
transport = "wss"
# wss_path = "/tunnel/v1"  # 自定义路径
```

这样可以与其他 Web 服务共存。

### 负载均衡

WebSocket 连接可以通过标准的 HTTP 负载均衡器：

```
Client → Load Balancer (443) → Server 1 (WebSocket)
                             → Server 2 (WebSocket)
                             → Server 3 (WebSocket)
```

负载均衡器需要支持 WebSocket（如 Nginx、HAProxy）。

## 最佳实践

1. **使用 443 端口**: 最大化防火墙穿透能力
2. **启用 TLS**: 始终使用 WSS 而不是 WS
3. **合理的超时**: 设置适当的连接超时和 Ping 间隔
4. **监控连接**: 记录 WebSocket 握手成功率和连接时长
5. **降级策略**: 如果 WebSocket 失败，尝试其他传输方式

## 下一步

- [ ] 集成到 client.rs 和 server.rs
- [ ] 添加端到端测试
- [ ] 实现 HTTP 代理支持
- [ ] 自定义 WebSocket 路径
- [ ] 连接池优化
- [ ] 性能基准测试

## 参考资料

- [RFC 6455 - The WebSocket Protocol](https://tools.ietf.org/html/rfc6455)
- [tokio-tungstenite 文档](https://docs.rs/tokio-tungstenite/)
- [WebSocket 性能优化](https://www.ably.com/topic/websockets)
