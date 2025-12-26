# TLS Tunnel 协议统一化文档

## 概述

本文档记录了 TLS Tunnel 客户端与服务器之间的控制通信协议的统一化过程。从原来的硬编码二进制协议升级到结构化的 JSON 格式协议。

## 协议设计

### 总体特点

- **格式**: 所有控制消息使用 **4 字节长度前缀 + UTF-8 JSON 载荷** 的格式
- **长度前缀**: 使用大端序（big-endian）的 32 位无符号整数
- **序列化**: 基于 `serde` 和 `serde_json` 库
- **字符编码**: UTF-8

### 消息格式

```
┌─────────────────┬──────────────────┐
│  4 Bytes (u32)  │  JSON Payload    │
│  Length (BE)    │  UTF-8 Encoded   │
└─────────────────┴──────────────────┘
```

## 消息定义

### 1. 认证请求 (AuthRequest)

**发送方**: 客户端  
**接收方**: 服务器  
**时机**: 连接建立后，第一条消息

```json
{
  "auth_key": "your-secret-key"
}
```

**Rust 定义**:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthRequest {
    pub auth_key: String,
}
```

### 2. 认证响应 (AuthResponse)

**发送方**: 服务器  
**接收方**: 客户端  
**时机**: 响应 AuthRequest

```json
{
  "success": true
}
```

或失败时：

```json
{
  "success": false,
  "error": "Invalid authentication key"
}
```

**Rust 定义**:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthResponse {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
```

### 3. 配置验证响应 (ConfigValidationResponse)

**发送方**: 服务器  
**接收方**: 客户端  
**时机**: 验证代理配置后

```json
{
  "valid": true
}
```

或验证失败时：

```json
{
  "valid": false,
  "error": "All proxies are already registered by other clients"
}
```

**Rust 定义**:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigValidationResponse {
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
```

### 4. 配置状态响应 (ConfigStatusResponse)

**发送方**: 服务器  
**接收方**: 客户端  
**时机**: 认证后，报告代理注册状态

```json
{
  "accepted": true,
  "rejected_proxies": []
}
```

或部分拒绝时：

```json
{
  "accepted": true,
  "rejected_proxies": ["proxy1:8080", "proxy2:8081"]
}
```

或全部拒绝时：

```json
{
  "accepted": false,
  "rejected_proxies": ["proxy1:8080", "proxy2:8081"]
}
```

**Rust 定义**:
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigStatusResponse {
    pub accepted: bool,
    #[serde(default)]
    pub rejected_proxies: Vec<String>,
}
```

## 客户端-服务器交互流程

### 完整的连接流程

```
Client                                    Server
   |                                         |
   |--- 建立 TLS/Transport 连接 ------------>|
   |                                         |
   |--- [4字节长度][AuthRequest JSON] ------>|
   |                                         |
   |<---- [4字节长度][AuthResponse JSON] ----|
   |                                         |
   |--- [4字节长度][ClientConfigs JSON] ---->|  (已有，格式不变)
   |                                         |
   |<---- [4字节长度][ConfigValidation] -----|
   |                                         |
   |<---- [4字节长度][ConfigStatus JSON] ----|
   |                                         |
   |--- 建立 Yamux 多路复用连接 ------------>|
   |                                         |
   |<----- Inbound Streams (Proxies) -------|
   |                                         |
```

### 详细步骤

#### 1. 认证阶段

1. **客户端发送认证请求**
   ```rust
   let auth_request = AuthRequest {
       auth_key: client_config.auth_key.clone(),
   };
   let request_json = serde_json::to_vec(&auth_request)?;
   tls_stream.write_all(&(request_json.len() as u32).to_be_bytes()).await?;
   tls_stream.write_all(&request_json).await?;
   ```

2. **服务器验证并响应**
   ```rust
   let response = if auth_request.auth_key == state.config.auth_key {
       AuthResponse::success()
   } else {
       AuthResponse::failed("Invalid authentication key".to_string())
   };
   let response_json = serde_json::to_vec(&response)?;
   tls_stream.write_all(&(response_json.len() as u32).to_be_bytes()).await?;
   tls_stream.write_all(&response_json).await?;
   ```

3. **客户端接收认证响应**
   ```rust
   let mut len_buf = [0u8; 4];
   tls_stream.read_exact(&mut len_buf).await?;
   let response_len = u32::from_be_bytes(len_buf) as usize;
   let mut response_buf = vec![0u8; response_len];
   tls_stream.read_exact(&mut response_buf).await?;
   let auth_response: AuthResponse = serde_json::from_slice(&response_buf)?;
   ```

#### 2. 配置验证阶段

1. **客户端发送代理配置**（格式保持不变）

2. **服务器验证配置并响应**
   ```rust
   let validation_resp = ConfigValidationResponse::valid();
   let resp_json = serde_json::to_vec(&validation_resp)?;
   tls_stream.write_all(&(resp_json.len() as u32).to_be_bytes()).await?;
   tls_stream.write_all(&resp_json).await?;
   ```

3. **服务器发送配置状态**
   ```rust
   let status_response = if rejected_proxies.is_empty() {
       ConfigStatusResponse::accepted()
   } else {
       ConfigStatusResponse::partially_rejected(rejected_proxies)
   };
   let response_json = serde_json::to_vec(&status_response)?;
   tls_stream.write_all(&(response_json.len() as u32).to_be_bytes()).await?;
   tls_stream.write_all(&response_json).await?;
   ```

4. **客户端接收状态响应**
   ```rust
   let mut len_buf = [0u8; 4];
   tls_stream.read_exact(&mut len_buf).await?;
   let response_len = u32::from_be_bytes(len_buf) as usize;
   let mut response_buf = vec![0u8; response_len];
   tls_stream.read_exact(&mut response_buf).await?;
   let status_response: ConfigStatusResponse = serde_json::from_slice(&response_buf)?;
   ```

## 错误处理

### 大小限制

- **认证请求**: 最大 10 KB
- **一般消息**: 根据 JSON 结构动态确定

### 常见错误消息

| 错误 | 含义 |
|------|------|
| `Invalid authentication key` | 提供的认证密钥不正确 |
| `Proxy configuration validation failed: ...` | 代理配置验证失败 |
| `All proxies are already registered by other clients: ...` | 所有代理都被其他客户端占用 |

### 错误处理流程

1. **认证失败**
   - 服务器发送 `AuthResponse { success: false, error: "..." }`
   - 断开连接

2. **配置验证失败**
   - 服务器发送 `ConfigValidationResponse { valid: false, error: "..." }`
   - 断开连接

3. **部分代理拒绝**
   - 服务器发送 `ConfigStatusResponse { accepted: true, rejected_proxies: [...] }`
   - 继续连接，但列出的代理不可用

## 向后兼容性

**注意**: 本次更改不向后兼容。任何使用旧二进制协议的客户端或服务器都无法与新版本通信。

建议在升级时确保客户端和服务器版本同步。

## 好处

1. **可读性**: JSON 格式易于理解和调试
2. **可扩展性**: 添加新字段无需更改二进制格式
3. **互操作性**: 支持使用任何语言/框架的客户端
4. **类型安全**: 使用 `serde` 确保类型正确性
5. **易于维护**: 集中定义在 `protocol.rs`，避免散落在各处

## 实现位置

| 组件 | 文件 | 说明 |
|------|------|------|
| 协议定义 | `src/protocol.rs` | 所有消息结构定义 |
| 服务器实现 | `src/server/mod.rs` | 服务器端发送/接收逻辑 |
| 客户端实现 | `src/client/mod.rs` | 客户端端发送/接收逻辑 |

## 测试建议

### 单元测试

```rust
#[test]
fn test_auth_request_serialization() {
    let req = AuthRequest {
        auth_key: "test-key".to_string(),
    };
    let json = serde_json::to_vec(&req).unwrap();
    let deserialized: AuthRequest = serde_json::from_slice(&json).unwrap();
    assert_eq!(deserialized.auth_key, "test-key");
}
```

### 集成测试

1. 启动服务器和客户端
2. 验证认证成功
3. 验证配置验证成功
4. 验证代理状态正确报告
5. 验证部分拒绝时的行为

## 版本号

- **实现版本**: v1.4.1+
- **协议版本**: 1.0
- **发布日期**: 2025-12-26

## 相关更改

- Commit: `3a96882` - 统一控制通信协议为 JSON 格式
- Commit: `6b622a4` - 协议消息结构定义
- Commit: `4114dc6` - 被拒绝代理信息通信
