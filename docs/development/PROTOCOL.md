# TLS Tunnel 协议说明

## 协议版本

当前版本: 2.0 (基于 Yamux 多路复用)

## 架构概述

本程序使用 **Yamux** 多路复用协议，通过单个 TLS 连接传输多个独立的数据流。这样设计的优势：

- 减少 TLS 握手开销
- 降低服务器端口占用
- 更好的连接复用
- 支持动态代理配置

## 连接流程

### 阶段 1: TLS 连接建立

客户端连接到服务器的监听端口（默认 8443）

```
客户端 ----TCP连接----> 服务器:8443
客户端 <---TLS 1.3---> 服务器
```

### 阶段 2: 身份认证

**步骤 1：客户端发送认证密钥**

```
+------------------+------------------+
| 密钥长度 (4字节) | 认证密钥 (N字节) |
+------------------+------------------+
   u32 (大端序)      UTF-8 字符串
```

**步骤 2：服务器验证并响应**

```
+------------------+
| 认证结果 (1字节) |
+------------------+
   1 = 成功
   0 = 失败
```

**如果认证失败**，服务器发送错误消息：

```
+------------------+------------------+
| 消息长度 (2字节) | 错误消息 (N字节) |
+------------------+------------------+
   u16 (大端序)      UTF-8 字符串
```

示例错误消息：
- "Invalid authentication key"
- "Authentication key too long (max 1024 bytes)"

### 阶段 3: 代理配置传输

客户端发送所有代理配置给服务器。

**步骤 1：发送代理数量**

```
+------------------+
| 代理数量 (2字节) |
+------------------+
   u16 (大端序)
```

**步骤 2：对每个代理发送配置**

```
+------------------+------------------+
| 名称长度 (2字节) | 代理名称 (N字节) |
+------------------+------------------+
   u16 (大端序)      UTF-8 字符串

+------------------+------------------+
| 发布端口 (2字节) | 本地端口 (2字节) |
+------------------+------------------+
   u16 (大端序)      u16 (大端序)
```

**字段说明：**
- `名称`: 代理的唯一标识符（如 "web", "ssh"）
- `发布端口` (publish_port): 服务器监听的端口（外部访问）
- `本地端口` (local_port): 客户端本地服务端口（转发目标）

**步骤 3：服务器验证配置**

服务器验证代理配置的有效性：
- 检查名称唯一性
- 检查 publish_port 唯一性
- 检查 local_port 唯一性
- 检查 publish_port 不与服务器监听端口冲突
- 检查端口非零
- 检查名称非空

如果验证失败，发送：

```
+------------------+
| 验证结果: 0      |
+------------------+
| 错误消息长度     |
+------------------+
| 错误消息内容     |
+------------------+
```

如果验证成功，发送：

```
+------------------+
| 验证结果: 1      |
+------------------+
```

### 阶段 4: Yamux 多路复用建立

在同一个 TLS 连接上建立 Yamux 多路复用会话：

```
┌─────────────────────────────────────┐
│        TLS 1.3 加密连接              │
├─────────────────────────────────────┤
│        Yamux 多路复用层              │
├─────────────────────────────────────┤
│  Stream 1 │ Stream 2 │ Stream 3 ... │
└─────────────────────────────────────┘
```

### 阶段 5: 数据转发

当外部用户访问服务器的某个监听端口时：

**步骤 1：服务器创建 Yamux Stream**

服务器通过 Yamux 创建新的出站流（outbound stream）

**步骤 2：发送目标端口**

```
+------------------+
| 目标端口 (2字节) |
+------------------+
   u16 (大端序)
```

这是客户端应该连接的本地服务端口。

**步骤 3：客户端连接本地服务**

客户端收到目标端口后，连接到本地服务（如 127.0.0.1:3000）

**步骤 4：双向数据转发**

```
外部用户 <-> 服务器监听端口 <-> Yamux Stream <-> 客户端 <-> 本地服务
```

数据在这个 Yamux Stream 中透明转发，不做任何修改。

## 完整协议流程示例

```
客户端                                              服务器
  |                                                    |
  |------------ TCP + TLS 连接建立 ------------------->|
  |<----------- TLS 1.3 握手 ------------------------->|
  |                                                    |
  |--- 认证密钥长度: 4字节 --------------------------->|
  |--- 认证密钥: "your-secret-key" -------------------->|
  |                                                    | (验证密钥)
  |<-- 认证结果: 1 (成功) -----------------------------|
  |                                                    |
  |--- 代理数量: 2 ------------------------------------>|
  |--- 代理1: "web", local=8080, remote=3000 ---------->|
  |--- 代理2: "ssh", local=2222, remote=22 ------------>|
  |                                                    | (验证配置)
  |<-- 验证结果: 1 (成功) -----------------------------|
  |                                                    |
  |<========== Yamux 多路复用会话建立 ================>|
  |                                                    | (服务器监听 8080, 2222)
  |                                                    |
  |                                                    | <-- 外部用户访问 :8080
  |                                                    | (创建 Yamux Stream #1)
  |<-- [Stream #1] 目标端口: 3000 --------------------|
  | (连接到 127.0.0.1:3000)                            |
  |                                                    |
  |<====== [Stream #1] 双向数据转发 ==================>|
  |                                                    |
  |                                                    | <-- 另一个用户访问 :2222
  |                                                    | (创建 Yamux Stream #2)
  |<-- [Stream #2] 目标端口: 22 ----------------------|
  | (连接到 127.0.0.1:22)                              |
  |                                                    |
  |<====== [Stream #2] 双向数据转发 ==================>|
  |                                                    |
```

## 错误处理

### 认证失败
- 服务器发送 `0x00` 作为认证结果
- 服务器发送错误消息（2字节长度 + 消息内容）
- 服务器记录警告日志（包含客户端 IP）
- 服务器关闭连接
- 客户端显示错误消息并退出（或重连）

### 配置验证失败
- 服务器发送 `0x00` 作为验证结果
- 服务器发送详细错误消息
- 示例：
  - "Duplicate proxy name 'web': each proxy must have a unique name"
  - "Proxy 'web' local_port 8443 conflicts with server bind port"
  - "Duplicate local_port 8080: each proxy must use a different server port"

### 本地服务连接失败
- 客户端自动重试（默认 3 次，间隔 1 秒）
- 如果全部失败，关闭 Yamux stream
- 记录错误日志

### Yamux 连接错误
- 任一方检测到 Yamux 错误时关闭连接
- 客户端自动重连（默认延迟 5 秒）
- 记录错误日志

### 数据传输错误
- 关闭当前 Yamux stream
- 保持主连接和其他 stream 继续工作
- 记录错误日志

## 自动重连机制

### 客户端重连
- 与服务器连接断开后，等待 5 秒自动重连
- 无限循环重试，直到连接成功
- 每次重连都需要重新认证和发送配置

### 本地服务重试
- 连接本地服务失败时，重试 3 次
- 每次重试间隔 1 秒
- 重试全部失败后放弃该连接

## 安全特性

1. **TLS 1.3 加密**：所有数据通过 TLS 1.3 加密传输
2. **密钥认证**：防止未授权客户端连接
3. **长度限制**：
   - 认证密钥：最大 1024 字节
   - 代理名称：最大 65535 字节
   - 错误消息：最大 4096 字节
4. **配置验证**：
   - 客户端验证：防止配置错误
   - 服务器验证：防止恶意配置
5. **端口冲突检测**：防止绑定冲突
6. **日志记录**：记录所有认证失败和异常行为

## 字节序

所有多字节数值使用**大端序**（Big-Endian / Network Byte Order）

## 数据类型

- `u8`: 无符号 8 位整数
- `u16`: 无符号 16 位整数（大端序）
- `u32`: 无符号 32 位整数（大端序）
- `String`: UTF-8 编码的字符串

## 协议限制

1. **密钥长度**：最大 1024 字节
2. **代理名称长度**：最大 65535 字节
3. **错误消息长度**：最大 4096 字节
4. **端口范围**：1-65535（端口 0 无效）
5. **代理数量**：最大 65535 个
6. **并发 Stream**：理论上无限制（受 Yamux 和系统资源限制）

## 配置约束

### 客户端验证
- 代理名称必须唯一
- local_port 必须唯一（每个代理占用不同的服务器端口）
- remote_port 必须唯一（每个代理连接不同的本地服务）
- 端口不能为 0
- 名称不能为空

### 服务器验证
- 所有客户端验证规则
- local_port 不能与服务器监听端口冲突
- 防止重复的名称和端口绑定

## Yamux 多路复用

### 为什么使用 Yamux？

1. **单连接多流**：避免为每个代理建立单独的 TLS 连接
2. **降低开销**：减少 TLS 握手次数
3. **连接复用**：更高效的资源利用
4. **动态扩展**：可以随时创建新的 stream

### Yamux 配置

使用默认 Yamux 配置：
- 模式：客户端为 Client，服务器为 Server
- 窗口大小：默认值
- 最大 Stream 数：无限制

### Stream 生命周期

1. **创建**：服务器接受外部连接时创建新 stream
2. **使用**：双向转发数据
3. **关闭**：任一端关闭或发生错误时关闭 stream
4. **清理**：Yamux 自动清理已关闭的 stream

## 与旧版本的区别

### 旧版本（1.0）
- 每个代理一个 TLS 连接
- 服务器需要预配置代理列表
- 无动态配置支持
- 无配置验证
- 无错误消息返回

### 新版本（2.0）
- 单个 TLS 连接 + Yamux 多路复用
- 服务器无需配置代理，由客户端动态提供
- 完整的配置验证（客户端 + 服务器）
- 详细的错误消息反馈
- 自动重连机制

## 实现注意事项

### 客户端实现要点

```rust
// 1. TLS 握手
let mut tls_stream = connector.connect(server_name, tcp_stream).await?;

// 2. 发送认证密钥
let key_bytes = auth_key.as_bytes();
let key_len = (key_bytes.len() as u32).to_be_bytes();
tls_stream.write_all(&key_len).await?;
tls_stream.write_all(key_bytes).await?;
tls_stream.flush().await?;

// 3. 等待认证结果
let mut auth_result = [0u8; 1];
tls_stream.read_exact(&mut auth_result).await?;
if auth_result[0] != 1 {
    // 读取错误消息
    let error_msg = read_error_message(&mut tls_stream).await?;
    return Err(anyhow!("Authentication failed: {}", error_msg));
}

// 4. 发送代理配置列表
let proxy_count = (config.proxies.len() as u16).to_be_bytes();
tls_stream.write_all(&proxy_count).await?;

for proxy in &config.proxies {
    // 发送名称
    let name_bytes = proxy.name.as_bytes();
    let name_len = (name_bytes.len() as u16).to_be_bytes();
    tls_stream.write_all(&name_len).await?;
    tls_stream.write_all(name_bytes).await?;
    
    // 发送端口
    tls_stream.write_all(&proxy.local_port.to_be_bytes()).await?;
    tls_stream.write_all(&proxy.remote_port.to_be_bytes()).await?;
}
tls_stream.flush().await?;

// 5. 等待配置验证结果
let mut config_result = [0u8; 1];
tls_stream.read_exact(&mut config_result).await?;
if config_result[0] != 1 {
    let error_msg = read_error_message(&mut tls_stream).await?;
    return Err(anyhow!("Configuration rejected: {}", error_msg));
}

// 6. 建立 Yamux 连接
let tls_compat = tls_stream.compat();  // tokio -> futures 兼容层
let mut yamux_conn = YamuxConnection::new(
    tls_compat, 
    YamuxConfig::default(), 
    yamux::Mode::Client
);

// 7. 处理入站 streams
loop {
    match yamux_conn.poll_next_inbound(cx).await {
        Some(Ok(stream)) => {
            // 读取目标端口并连接本地服务
            handle_stream(stream, config).await?;
        }
        Some(Err(e)) => break,
        None => break,
    }
}
```

### 服务器实现要点

```rust
// 1. TLS 握手
let mut tls_stream = acceptor.accept(tcp_stream).await?;

// 2. 读取并验证认证密钥
let mut key_len_buf = [0u8; 4];
tls_stream.read_exact(&mut key_len_buf).await?;
let key_len = u32::from_be_bytes(key_len_buf) as usize;

if key_len > 1024 {
    tls_stream.write_all(&[0]).await?;
    send_error_message(&mut tls_stream, "Authentication key too long").await?;
    return Err(anyhow!("Key too long"));
}

let mut key_buf = vec![0u8; key_len];
tls_stream.read_exact(&mut key_buf).await?;
let client_key = String::from_utf8(key_buf)?;

if client_key != config.auth_key {
    tls_stream.write_all(&[0]).await?;
    send_error_message(&mut tls_stream, "Invalid authentication key").await?;
    return Err(anyhow!("Authentication failed"));
}

tls_stream.write_all(&[1]).await?;  // 认证成功
tls_stream.flush().await?;

// 3. 读取代理配置
let mut proxy_count_buf = [0u8; 2];
tls_stream.read_exact(&mut proxy_count_buf).await?;
let proxy_count = u16::from_be_bytes(proxy_count_buf);

let mut proxies = Vec::new();
for _ in 0..proxy_count {
    // 读取名称
    let mut name_len_buf = [0u8; 2];
    tls_stream.read_exact(&mut name_len_buf).await?;
    let name_len = u16::from_be_bytes(name_len_buf) as usize;
    
    let mut name_buf = vec![0u8; name_len];
    tls_stream.read_exact(&mut name_buf).await?;
    let name = String::from_utf8(name_buf)?;
    
    // 读取端口
    let mut local_port_buf = [0u8; 2];
    tls_stream.read_exact(&mut local_port_buf).await?;
    let local_port = u16::from_be_bytes(local_port_buf);
    
    let mut remote_port_buf = [0u8; 2];
    tls_stream.read_exact(&mut remote_port_buf).await?;
    let remote_port = u16::from_be_bytes(remote_port_buf);
    
    proxies.push(ProxyInfo { name, local_port, remote_port });
}

// 4. 验证配置
if let Err(e) = validate_proxy_configs(&proxies, config.bind_port) {
    tls_stream.write_all(&[0]).await?;
    send_error_message(&mut tls_stream, &format!("{}", e)).await?;
    return Err(e);
}

tls_stream.write_all(&[1]).await?;  // 配置验证成功
tls_stream.flush().await?;

// 5. 建立 Yamux 连接
let tls_compat = tls_stream.compat();
let yamux_conn = YamuxConnection::new(
    tls_compat,
    YamuxConfig::default(),
    yamux::Mode::Server
);

// 6. 为每个代理启动监听器
for proxy in proxies {
    let listener = TcpListener::bind((config.bind_addr, proxy.local_port)).await?;
    tokio::spawn(async move {
        loop {
            let (inbound, _) = listener.accept().await?;
            // 创建 Yamux outbound stream
            let mut stream = yamux_conn.poll_new_outbound(cx).await?;
            // 发送目标端口
            stream.write_all(&proxy.remote_port.to_be_bytes()).await?;
            // 双向转发
            forward_data(inbound, stream).await?;
        }
    });
}
```

## 测试建议

1. **正常流程测试**：
   - 验证完整的连接建立和数据转发
   - 测试多个代理同时工作
   - 测试多个并发连接

2. **认证测试**：
   - 使用错误的密钥
   - 使用超长的密钥（> 1024 字节）
   - 不发送密钥直接发送数据

3. **配置验证测试**：
   - 重复的代理名称
   - 重复的 local_port
   - 重复的 remote_port
   - local_port 与服务器监听端口冲突
   - 端口为 0
   - 空的代理名称

4. **错误处理测试**：
   - 本地服务未启动
   - 在各个阶段断开连接
   - 网络中断后重连

5. **并发和负载测试**：
   - 多个客户端同时连接
   - 大量并发 streams
   - 大文件传输测试
   - 长时间运行稳定性测试

6. **Yamux 测试**：
   - Stream 创建和关闭
   - 多个 stream 并发数据传输
   - Stream 错误处理

## 性能考虑

### 优化点
1. **零拷贝**：使用高效的 I/O 操作
2. **异步处理**：基于 Tokio 的异步 I/O
3. **连接复用**：Yamux 避免重复 TLS 握手
4. **缓冲区管理**：合理的缓冲区大小

### 性能指标
- TLS 握手延迟：< 100ms
- Stream 创建延迟：< 10ms
- 数据转发延迟：< 5ms
- 吞吐量：取决于网络带宽

## 兼容性

- **Rust 版本**：最低 1.70
- **TLS 版本**：TLS 1.3
- **Yamux 版本**：0.13.x
- **支持平台**：Windows、Linux、macOS、BSD

## 相关资源

- [Yamux 规范](https://github.com/hashicorp/yamux/blob/master/spec.md)
- [TLS 1.3 RFC 8446](https://datatracker.ietf.org/doc/html/rfc8446)
- [Rustls 文档](https://docs.rs/rustls/)
- [Tokio 文档](https://tokio.rs/)
