# Visitor 模式指南

Visitor 模式允许客户端通过服务器中转访问另一个客户端的服务，实现客户端到客户端的内网穿透。

## 概述

### Proxy vs Visitor

- **Proxy 模式**：外部用户 → 服务器 → 客户端B → 本地服务
- **Visitor 模式**：客户端C → 服务器（中转） → 客户端B → 本地服务

**关键特点**：
- 服务器负责注册proxy和转发visitor请求
- 通过 `(name, publish_port)` 精确匹配目标proxy
- 实现客户端之间的点对点连接（经服务器中转）

## 使用场景

### 1. 访问远程数据库

客户端C需要访问客户端B上的数据库（MySQL、PostgreSQL、MongoDB 等）：

```toml
# 客户端C配置（visitor端）
[[visitors]]
name = "mysql-proxy"
bind_addr = "127.0.0.1"
bind_port = 3306
publish_port = 3306

# 客户端B配置（proxy端）
[[proxies]]
name = "mysql-proxy"
type = "tcp"
local_port = 3306
publish_port = 3306
```

使用：
```bash
# 在客户端C机器上
mysql -h 127.0.0.1 -P 3306 -u user -p
# 实际连接到客户端B的MySQL
```

### 2. 访问内部 API

客户端C需要调用客户端B所在内网的 API：

```toml
# 客户端C配置
[[visitors]]
name = "api-proxy"
bind_addr = "127.0.0.1"
bind_port = 8080
publish_port = 8080

# 客户端B配置
[[proxies]]
name = "api-proxy"
type = "http/1.1"
local_port = 8000
publish_port = 8080
```

### 3. 远程开发调试

```toml
# 开发者本地（客户端C）
[[visitors]]
name = "redis-proxy"
bind_port = 6379
publish_port = 6379

# 远程机器（客户端B）
[[proxies]]
name = "redis-proxy"
type = "tcp"
local_port = 6379
publish_port = 6379
```

### 4. 安全访问敏感服务

不在公网暴露端口，只通过visitor访问：

```toml
# 客户端B配置
[[proxies]]
name = "admin-db"
type = "tcp"
local_port = 5432
publish_port = 5432
# 不配置 publish_addr，服务器不监听端口
```

## 快速开始

### 客户端C配置 (visitor-client.toml)

```toml
[client]
server_addr = "your-server.com"
server_port = 8443
server_name = "your-server.com"
auth_key = "your-secret-key"
transport = "tls"

# Visitor 配置
[[visitors]]
name = "mysql-proxy"
bind_addr = "127.0.0.1"
bind_port = 3306
publish_port = 3306

[[visitors]]
name = "redis-proxy"
bind_addr = "127.0.0.1"
bind_port = 6379
publish_port = 6379
```

### 客户端B配置 (proxy-client.toml)

```toml
[client]
server_addr = "your-server.com"
server_port = 8443
server_name = "your-server.com"
auth_key = "your-secret-key"
transport = "tls"

# Proxy 配置：仅支持 visitor 访问
[[proxies]]
name = "mysql-proxy"
type = "tcp"
local_port = 3306
publish_port = 3306
# 不配置 publish_addr

[[proxies]]
name = "redis-proxy"
type = "tcp"
local_port = 6379
publish_port = 6379

# 混合模式：同时支持外部访问和 visitor
[[proxies]]
name = "web"
type = "http/1.1"
local_port = 3000
publish_addr = "0.0.0.0"
publish_port = 8080
```

### 服务器配置 (server.toml)

```toml
[server]
bind_addr = "0.0.0.0"
bind_port = 8443
cert_path = "certs/cert.pem"
key_path = "certs/key.pem"
auth_key = "your-secret-key"
transport = "tls"
```

## 访问模式

### 模式1：仅 Visitor 访问

不配置 `publish_addr`，其他客户端只能通过 visitor 访问，不会在服务器上开放端口。

```toml
[[proxies]]
name = "mysql-proxy"
local_port = 3306
publish_port = 3306
# 未配置 publish_addr
```

适用于敏感服务（数据库、内部API）和仅团队内部访问的场景。

### 模式2：外部访问

配置 `publish_addr`，服务器将在 `publish_port` 上监听，支持外部直接访问，同时也支持 visitor 访问。

```toml
[[proxies]]
name = "web-proxy"
local_port = 8000
publish_addr = "0.0.0.0"
publish_port = 8080
```

适用于需要对外提供服务的场景。

## 匹配机制

服务器使用 `(name, publish_port)` 元组作为键查找 proxy。Visitor 连接时发送 name 和 publish_port，服务器根据这两个字段在注册表中精确匹配对应的 proxy。

**多客户端场景示例**：

```toml
# 客户端B1（开发环境）
[[proxies]]
name = "mysql"
local_port = 3306
publish_port = 13306

# 客户端C 访问开发环境
[[visitors]]
name = "mysql"
bind_port = 3306
publish_port = 13306  # 必须匹配
```

## 配置详解

### Visitor 配置项（客户端C）

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `name` | string | 是 | 目标 proxy 的名称（用于匹配） |
| `bind_addr` | string | 否 | 本地监听地址（默认 127.0.0.1） |
| `bind_port` | u16 | 是 | 本地监听端口 |
| `publish_port` | u16 | 是 | 目标 proxy 的 publish_port（用于精确匹配） |

### Proxy 配置项（客户端B）

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `name` | string | 是 | Proxy 名称（对应 visitor 的 name） |
| `type` | string | 否 | 连接类型（tcp/http/1.1/http/2.0） |
| `local_port` | u16 | 是 | 本地服务端口 |
| `publish_port` | u16 | 是 | 端口标识符，用于匹配和监听 |
| `publish_addr` | string | 否 | 服务器监听地址，配置后启用外部访问 |

**关键说明**：

- `publish_port` 必填，与 `name` 组成唯一标识符 `(name, publish_port)`
- `publish_addr` 不配置时为仅 Visitor 模式，配置后为混合模式

## 工作原理

1. **Proxy 端连接**：客户端B连接服务器，注册 proxy 到全局注册表（使用 name 和 publish_port 作为键）
2. **Visitor 端连接**：客户端C连接服务器，启动 visitor 监听本地端口
3. **建立隧道**：本地应用连接到 visitor 端口时，客户端C通过服务器查找匹配的 proxy，建立端到端隧道
4. **数据转发**：通过 Yamux 多路复用实现双向数据转发：`客户端C ↔ 服务器 ↔ 客户端B`

## 最佳实践

- 使用描述性的 name（如 `mysql-dev`、`redis-cache`）
- 仅 Visitor 模式可使用标准端口作为 publish_port（如 3306、6379）
- 外部访问模式建议按环境分配不同端口段避免冲突
- 根据服务类型选择合适的 type（tcp/http/1.1/http/2.0）
- Visitor 默认绑定 127.0.0.1，使用强认证密钥

## 故障排查

### 常见错误

**1. Proxy 不存在或未连接**

错误：`Proxy 'mysql-proxy' with publish_port 3306 not found or client not connected`

解决：确认客户端B已连接，proxy 的 name 和 publish_port 与 visitor 配置一致。

**2. 端口被占用**

错误：`Failed to bind visitor to 127.0.0.1:3306`

解决：更改 `bind_port` 或停止占用端口的程序。

**3. 连接超时**

解决：检查客户端B的目标服务是否运行，检查防火墙和网络连接。

### 调试步骤

1. 查看服务器日志确认 proxy 已注册
2. 启用详细日志：`RUST_LOG=debug ./tls-tunnel client client.toml`
3. 测试连接：`telnet 127.0.0.1 3306` 或 `nc -zv 127.0.0.1 3306`
4. 验证客户端B能连接目标服务

## 相关资源

### 配置示例

- [Visitor 端配置示例](../../examples/visitor-client.toml) - 客户端C配置
- [Proxy 端配置示例](../../examples/visitor-server.toml) - 客户端B配置

### 技术文档

- [架构设计文档](../development/ARCHITECTURE.md) - 系统架构设计
- [协议文档](../development/PROTOCOL.md) - 底层协议细节
- [开发文档](../development/DEVELOPMENT.md) - 开发指南

### 其他

- [README](../../README.md) - 项目概述
- [快速开始](QUICKSTART.md) - 快速入门指南
