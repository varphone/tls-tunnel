# 传输层抽象重构

## 概述

本次重构为 TLS Tunnel 引入了传输层抽象，为未来支持多种传输协议（TCP+TLS、HTTP/2.0、WebSocket）奠定了基础。

## 架构设计

### 传输层抽象

新增了 `transport` 模块，定义了统一的传输层接口：

```rust
// src/transport.rs

/// 传输层类型
pub enum TransportType {
    Tls,    // TCP + TLS（当前已实现）
    Http2,  // HTTP/2.0 over TLS（待实现）
    Wss,    // WebSocket Secure（待实现）
}

/// 传输层连接抽象
pub trait Transport: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

/// 传输层客户端接口
#[async_trait]
pub trait TransportClient: Send + Sync {
    async fn connect(&self) -> Result<Pin<Box<dyn Transport>>>;
    fn transport_type(&self) -> TransportType;
}

/// 传输层服务器接口
#[async_trait]
pub trait TransportServer: Send + Sync {
    async fn accept(&self) -> Result<Pin<Box<dyn Transport>>>;
    fn transport_type(&self) -> TransportType;
}
```

### 模块结构

```
src/transport/
├── mod.rs        # 传输层接口定义
├── tls.rs        # TLS 传输实现（已完成）
├── http2.rs      # HTTP/2 传输实现（占位）
└── wss.rs        # WebSocket 传输实现（占位）
```

## 实现状态

### ✅ 已完成

1. **传输层接口定义**
   - `Transport` trait：统一的流接口
   - `TransportClient` trait：客户端传输层
   - `TransportServer` trait：服务器传输层
   - `TransportType` enum：传输类型标识

2. **TLS 传输实现** (`src/transport/tls.rs`)
   - `TlsTransportClient`: TLS 客户端传输
   - `TlsTransportServer`: TLS 服务器传输
   - 完全兼容现有的 TCP+TLS 功能

3. **配置支持**
   - `ServerConfig` 新增 `transport` 字段
   - `ClientConfig` 新增 `transport` 字段
   - 默认值为 `TransportType::Tls`（保持向后兼容）

### 🚧 待实现

1. **WebSocket 传输** (`src/transport/wss.rs`)
   - 使用 tokio-tungstenite 实现 WSS
   - WebSocket 协议升级
   - 二进制帧传输隧道数据
   - **状态**: 占位实现，待完成

2. **客户端/服务器集成**
   - 修改 `client.rs` 使用传输层抽象
   - 修改 `server.rs` 使用传输层抽象
   - 根据配置动态选择传输方式

### ✅ 新增完成

**HTTP/2 传输实现** (`src/transport/http2.rs`)
- `Http2TransportClient`: 通过 HTTP/2 CONNECT 建立客户端隧道
- `Http2TransportServer`: 接受 HTTP/2 CONNECT 请求
- `Http2Stream`: 包装 H2 的 SendStream + RecvStream 实现 AsyncRead/AsyncWrite
- 支持 HTTP/2 流量控制和多路复用
- **完全实现并通过编译** ✅

## 配置示例

### 服务器配置

```toml
[server]
bind_addr = "0.0.0.0"
bind_port = 8443
# 传输类型: tls（默认）, http2, wss
transport = "tls"
cert_path = "cert.pem"
key_path = "key.pem"
auth_key = "your-secret-key"
```

### 客户端配置

```toml
[client]
server_addr = "server.com"
server_port = 8443
# 传输类型: tls（默认）, http2, wss
transport = "tls"
skip_verify = false
ca_cert_path = "ca.pem"
auth_key = "your-secret-key"

[[proxies]]
name = "web"
remote_port = 8080
local_port = 80
proxy_type = "http/1.1"
```

## 使用场景

### TCP + TLS（当前支持）
- **优点**: 简单、高效、低延迟
- **适用**: 内网穿透、端口转发
- **特性**: 直接 TCP 连接，TLS 加密

### HTTP/2（计划支持）
- **优点**: 穿越 HTTP 代理、多路复用
- **适用**: 企业网络、有代理的环境
- **特性**: 基于 HTTP/2 CONNECT 隧道

### WebSocket（计划支持）
- **优点**: 穿越严格防火墙、CDN 友好
- **适用**: 高度限制的网络环境
- **特性**: WebSocket 升级，伪装成普通 HTTPS

## 技术细节

### 依赖项

新增的依赖（为未来实现准备）：

```toml
async-trait = "0.1"      # 异步 trait 支持
bytes = "1.0"             # 字节缓冲
h2 = "0.4"                # HTTP/2 实现
http = "1.0"              # HTTP 类型
tokio-tungstenite = "0.24" # WebSocket 实现
```

### 设计原则

1. **抽象统一**: 所有传输方式实现相同接口
2. **可扩展**: 易于添加新的传输协议
3. **向后兼容**: 默认使用 TLS，不影响现有配置
4. **类型安全**: 编译时检查传输类型

## 后续工作

### Phase 1: HTTP/2 实现
- [ ] 实现 HTTP/2 客户端传输
- [ ] 实现 HTTP/2 服务器传输
- [ ] 测试 HTTP/2 隧道功能
- [ ] 性能测试和优化

### Phase 2: WebSocket 实现
- [ ] 实现 WSS 客户端传输
- [ ] 实现 WSS 服务器传输
- [ ] WebSocket 帧处理优化
- [ ] 测试和文档

### Phase 3: 客户端/服务器集成
- [ ] 修改客户端使用传输抽象
- [ ] 修改服务器使用传输抽象
- [ ] 动态传输选择
- [ ] 完整的端到端测试

### Phase 4: 高级特性
- [ ] 传输层自动降级
- [ ] 多传输并行连接
- [ ] 传输层统计和监控
- [ ] 性能调优

## 测试计划

### 单元测试
- 每种传输的连接建立
- 数据收发正确性
- 错误处理

### 集成测试
- 客户端-服务器通信
- 代理功能测试
- 并发连接测试

### 性能测试
- 吞吐量测试
- 延迟测试
- 资源占用测试

## 注意事项

1. **当前版本**: 传输层框架已就绪，但仅 TLS 传输可用
2. **HTTP/2 和 WSS**: 需要额外的开发工作才能启用
3. **配置兼容**: 不设置 `transport` 字段时默认使用 TLS
4. **性能考虑**: HTTP/2 和 WSS 会有额外开销，适用于特定场景

## 文件变更

### 新增文件
- `src/transport.rs` - 传输层接口定义
- `src/transport/tls.rs` - TLS 传输实现
- `src/transport/http2.rs` - HTTP/2 传输占位
- `src/transport/wss.rs` - WebSocket 传输占位

### 修改文件
- `src/config.rs` - 添加 `transport` 字段
- `src/main.rs` - 注册 `transport` 模块
- `Cargo.toml` - 添加新依赖

### 未来需要修改
- `src/client.rs` - 使用传输抽象
- `src/server.rs` - 使用传输抽象

## 总结

本次重构建立了传输层抽象框架，为支持多种传输协议打下了坚实基础。虽然 HTTP/2 和 WebSocket 的完整实现还需要进一步开发，但现有架构已经为扩展做好了准备，且不影响当前的 TLS 传输功能。
