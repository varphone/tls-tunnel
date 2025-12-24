# 项目结构说明

## 目录结构

```
tls-tunnel/
├── src/
│   ├── main.rs          # 主程序入口
│   ├── cli.rs           # 命令行参数解析（Clap）
│   ├── config.rs        # 配置文件结构和加载
│   ├── tls.rs           # TLS 证书管理
│   ├── server.rs        # 服务器端实现
│   └── client.rs        # 客户端实现
├── Cargo.toml           # Rust 项目配置
├── server.toml          # 服务器配置示例
├── client.toml          # 客户端配置示例
├── generate-cert.ps1    # Windows 证书生成脚本
├── generate-cert.sh     # Linux/macOS 证书生成脚本
├── README.md            # 项目说明文档
├── QUICKSTART.md        # 快速开始指南
└── ARCHITECTURE.md      # 本文件 - 架构说明
```

## 模块说明

### 1. main.rs - 主程序入口

负责：
- 解析命令行参数
- 初始化日志系统
- 根据模式（服务器/客户端）加载配置
- 启动相应的服务

```rust
// 流程：
CLI 解析 -> 加载配置 -> 初始化 TLS -> 启动服务器/客户端
```

### 2. cli.rs - 命令行参数解析

使用 Clap 库提供：
- `server` 子命令：启动服务器模式
- `client` 子命令：启动客户端模式
- `--log-level` 全局参数：设置日志级别
- `-c/--config` 参数：指定配置文件路径

### 3. config.rs - 配置管理

定义了三个主要结构：

#### ProxyConfig
```rust
pub struct ProxyConfig {
    pub name: String,        // 代理名称
    pub local_port: u16,     // 本地端口
    pub remote_port: u16,    // 远程端口
}
```

#### ServerConfig
```rust
pub struct ServerConfig {
    pub bind_addr: String,       // 绑定地址
    pub bind_port: u16,          // 监听端口
    pub cert_path: PathBuf,      // 证书路径
    pub key_path: PathBuf,       // 私钥路径
    pub proxies: Vec<ProxyConfig>, // 代理列表
}
```

#### ClientConfig
```rust
pub struct ClientConfig {
    pub server_addr: String,     // 服务器地址
    pub server_port: u16,        // 服务器端口
    pub skip_verify: bool,       // 是否跳过证书验证
    pub ca_cert_path: Option<PathBuf>, // CA 证书路径
    pub proxies: Vec<ProxyConfig>, // 代理列表
}
```

### 4. tls.rs - TLS 管理

提供两个核心函数：

#### load_server_config()
- 加载服务器证书和私钥
- 创建 Rustls ServerConfig
- 返回 TlsAcceptor

#### load_client_config()
- 加载 CA 证书（可选）
- 支持跳过证书验证（测试用）
- 创建 Rustls ClientConfig
- 返回 TlsConnector

### 5. server.rs - 服务器端

#### 主要功能：

1. **run_server()** - 主服务循环
   - 监听 TLS 端口等待客户端连接
   - 为每个代理配置启动监听器（暂未完全实现）
   - 处理客户端的 TLS 连接

2. **handle_tls_connection()** - 处理 TLS 连接
   - 接受 TLS 握手
   - 读取协议头（代理名称）
   - 连接到本地目标服务
   - 双向转发数据

#### 数据流：

```
外部访问 -> 本地端口 -> TLS 连接 -> 协议解析 -> 目标服务
```

### 6. client.rs - 客户端

#### 主要功能：

1. **run_client()** - 主客户端循环
   - 为每个代理配置启动本地监听器
   - 管理所有代理任务

2. **start_local_listener()** - 本地监听
   - 监听本地端口等待连接
   - 每个连接启动一个处理任务

3. **handle_local_connection()** - 处理本地连接
   - 连接到服务器并进行 TLS 握手
   - 发送协议头（代理名称）
   - 双向转发数据

#### 数据流：

```
本地服务 -> 本地监听 -> TLS 连接 -> 服务器 -> 最终目标
```

## 协议设计

### 简单协议格式

客户端连接到服务器后，首先发送：

```
+----------------+------------------+
| 名称长度 (4B)  |  代理名称 (N字节) |
+----------------+------------------+
    u32 (BE)         UTF-8 字符串
```

之后进行双向数据转发。

## 数据流向示意

### 完整流程：

```
1. 服务器启动：
   - 监听 0.0.0.0:8443 (TLS)
   - 监听 0.0.0.0:8080 (代理 "web")

2. 客户端启动：
   - 监听 127.0.0.1:3000 (代理 "web")

3. 用户访问 server:8080
   - 服务器接受连接
   - 等待客户端 TLS 连接

4. 客户端检测到本地 3000 端口连接
   - 连接到 server:8443
   - 发送代理名称 "web"

5. 服务器接收 TLS 连接
   - 读取代理名称 "web"
   - 建立双向转发

6. 数据流动：
   用户 <-> 8080 <-> TLS <-> 3000 <-> 本地服务
```

## 并发模型

使用 Tokio 异步运行时：

- **服务器端**：每个客户端连接一个异步任务
- **客户端**：每个本地连接一个异步任务
- **数据转发**：使用 `tokio::io::copy` 进行零拷贝转发

## 安全考虑

1. **TLS 加密**：所有数据通过 TLS 1.3 加密传输
2. **证书验证**：生产环境强制验证服务器证书
3. **内存安全**：Rust 提供内存安全保证
4. **DoS 防护**：需要添加连接限制（TODO）

## 性能优化

1. **零拷贝**：使用 `tokio::io::copy`
2. **异步 I/O**：基于 Tokio 的高效事件循环
3. **连接复用**：每个连接独立处理，无阻塞

## 未来改进方向

1. **连接池**：维护客户端到服务器的持久连接池
2. **心跳机制**：定期检测连接健康状态
3. **流量统计**：记录和报告数据传输量
4. **认证机制**：添加客户端认证
5. **配置热重载**：支持不重启更新配置
6. **WebSocket 支持**：支持 WebSocket 协议
7. **负载均衡**：多服务器负载分配
8. **访问控制**：IP 白名单/黑名单
9. **压缩**：数据传输压缩
10. **监控面板**：Web 管理界面

## 依赖关系图

```
main.rs
  ├─> cli.rs (Clap)
  ├─> config.rs (Serde, TOML)
  ├─> tls.rs (Rustls, Rustls-pemfile)
  ├─> server.rs (Tokio, Tokio-rustls)
  └─> client.rs (Tokio, Tokio-rustls)
```

## 错误处理

使用 `anyhow` 库进行统一的错误处理：
- 所有错误类型转换为 `anyhow::Error`
- 使用 `.context()` 添加错误上下文
- 顶层统一处理和记录错误

## 日志系统

使用 `tracing` 进行结构化日志：
- `info!()`: 重要操作（连接建立、关闭）
- `warn!()`: 警告信息
- `error!()`: 错误信息
- `debug!()`: 调试信息

日志级别通过命令行参数控制：
```bash
tls-tunnel --log-level debug server -c server.toml
```
