# 开发者指南

本指南面向希望理解、修改或扩展 TLS Tunnel 项目的开发者。

## 目录
- [开发环境设置](#开发环境设置)
- [项目结构](#项目结构)
- [核心概念](#核心概念)
- [代码风格](#代码风格)
- [测试](#测试)
- [调试](#调试)
- [扩展功能](#扩展功能)
- [贡献指南](#贡献指南)

---

## 开发环境设置

### 必需工具
- Rust 1.70+ (推荐使用 rustup)
- Cargo (随 Rust 安装)
- OpenSSL (用于生成测试证书)

### 安装 Rust
```bash
# Linux/macOS
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Windows
# 下载并运行 https://rustup.rs/
```

### 克隆和构建
```bash
git clone <repository>
cd tls-tunnel
cargo build
```

### 开发模式运行
```bash
# 带调试符号的快速编译
cargo run -- server -c server.toml

# 发布模式（优化）
cargo run --release -- server -c server.toml
```

---

## 项目结构

```
src/
├── main.rs      # 程序入口，负责初始化和调度
├── cli.rs       # 命令行参数解析
├── config.rs    # 配置文件结构定义
├── tls.rs       # TLS 证书管理
├── server.rs    # 服务器端核心逻辑
└── client.rs    # 客户端核心逻辑
```

### 依赖关系
```
main.rs
  ├─> cli::Cli::parse()
  ├─> config::AppConfig::from_file()
  ├─> tls::load_server_config() / load_client_config()
  └─> server::run_server() / client::run_client()
```

---

## 核心概念

### 1. 异步运行时 (Tokio)

项目使用 Tokio 作为异步运行时：

```rust
#[tokio::main]
async fn main() -> Result<()> {
    // 异步代码
}
```

**关键点**:
- 所有 I/O 操作都是非阻塞的
- 使用 `async/await` 语法
- `tokio::spawn` 创建并发任务

### 2. TLS 加密 (Rustls)

使用 Rustls 提供 TLS 功能：

```rust
// 服务器端
let tls_acceptor = TlsAcceptor::from(tls_config);
let tls_stream = tls_acceptor.accept(tcp_stream).await?;

// 客户端
let tls_connector = TlsConnector::from(tls_config);
let tls_stream = tls_connector.connect(server_name, tcp_stream).await?;
```

### 3. 数据转发模式

使用 `tokio::io::copy` 进行零拷贝数据转发：

```rust
let (mut read_half, mut write_half) = stream.split();
tokio::io::copy(&mut read_half, &mut write_half).await?;
```

### 4. 协议设计

简单的协议格式：

```
客户端连接后首先发送:
┌────────────────┬──────────────┐
│ 名称长度 (4B)  │ 代理名称     │
└────────────────┴──────────────┘
     u32 BE         UTF-8 字符串
```

---

## 代码风格

### Rust 标准风格
```bash
# 格式化代码
cargo fmt

# 检查代码风格
cargo clippy

# 检查代码（不编译）
cargo check
```

### 命名约定
- **模块**: snake_case (`tls.rs`, `config.rs`)
- **结构体**: PascalCase (`ServerConfig`, `ProxyConfig`)
- **函数**: snake_case (`run_server`, `load_config`)
- **常量**: SCREAMING_SNAKE_CASE

### 错误处理
使用 `anyhow` 进行统一的错误处理：

```rust
use anyhow::{Context, Result};

fn load_file() -> Result<String> {
    let content = std::fs::read_to_string("file.txt")
        .context("Failed to read file")?;
    Ok(content)
}
```

### 日志记录
使用 `tracing` 库：

```rust
use tracing::{info, warn, error, debug};

info!("Server started on port {}", port);
warn!("Connection timeout");
error!("Failed to bind port: {}", err);
debug!("Received {} bytes", len);
```

---

## 测试

### 单元测试
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_parsing() {
        // 测试代码
    }
}
```

运行测试：
```bash
cargo test
```

### 集成测试
在 `tests/` 目录创建集成测试：

```rust
// tests/integration_test.rs
#[tokio::test]
async fn test_full_connection() {
    // 集成测试代码
}
```

### 手动测试
```bash
# 终端 1: 启动测试服务器
python -m http.server 3000

# 终端 2: 启动 TLS 服务器
cargo run -- server -c server.toml

# 终端 3: 启动 TLS 客户端
cargo run -- client -c client.toml

# 终端 4: 测试连接
curl http://localhost:8080
```

---

## 调试

### 启用详细日志
```bash
cargo run -- --log-level debug server -c server.toml
```

### 使用 Rust 调试器
```bash
# 使用 rust-lldb (macOS/Linux)
rust-lldb target/debug/tls-tunnel

# 使用 rust-gdb (Linux)
rust-gdb target/debug/tls-tunnel
```

### VS Code 调试配置
在 `.vscode/launch.json` 添加：

```json
{
    "version": "0.2.0",
    "configurations": [
        {
            "type": "lldb",
            "request": "launch",
            "name": "Debug Server",
            "cargo": {
                "args": ["build", "--bin=tls-tunnel"],
                "filter": {
                    "name": "tls-tunnel",
                    "kind": "bin"
                }
            },
            "args": ["server", "-c", "server.toml"],
            "cwd": "${workspaceFolder}"
        }
    ]
}
```

### 网络调试
```bash
# 查看监听端口
netstat -an | grep LISTEN

# 抓包分析
tcpdump -i any port 8443 -w capture.pcap

# 使用 Wireshark 分析
wireshark capture.pcap
```

---

## 扩展功能

### 1. 添加新的配置选项

#### 步骤 1: 修改配置结构 (`config.rs`)
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    // 现有字段...
    
    /// 新增: 最大连接数
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
}

fn default_max_connections() -> usize {
    100
}
```

#### 步骤 2: 在实现中使用
```rust
// server.rs
pub async fn run_server(config: ServerConfig, tls_acceptor: TlsAcceptor) -> Result<()> {
    let max_connections = config.max_connections;
    // 使用 max_connections 限制并发连接
}
```

### 2. 实现连接池

创建新模块 `src/pool.rs`:

```rust
use std::collections::HashMap;
use tokio::sync::Mutex;
use tokio_rustls::client::TlsStream;

pub struct ConnectionPool {
    connections: Mutex<HashMap<String, Vec<TlsStream<TcpStream>>>>,
}

impl ConnectionPool {
    pub fn new() -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
        }
    }

    pub async fn get_or_create(&self, proxy_name: &str) -> Result<TlsStream<TcpStream>> {
        // 实现连接池逻辑
    }
}
```

在 `main.rs` 中注册模块：
```rust
mod pool;
```

### 3. 添加心跳机制

在 `server.rs` 或 `client.rs` 中：

```rust
async fn heartbeat_task(stream: &mut TlsStream<TcpStream>) -> Result<()> {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    
    loop {
        interval.tick().await;
        stream.write_all(b"PING").await?;
        
        let mut buf = [0u8; 4];
        stream.read_exact(&mut buf).await?;
        
        if &buf != b"PONG" {
            return Err(anyhow!("Heartbeat failed"));
        }
    }
}
```

### 4. 实现流量统计

创建 `src/stats.rs`:

```rust
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Clone)]
pub struct Statistics {
    bytes_sent: Arc<AtomicU64>,
    bytes_received: Arc<AtomicU64>,
    connections_total: Arc<AtomicU64>,
}

impl Statistics {
    pub fn new() -> Self {
        Self {
            bytes_sent: Arc::new(AtomicU64::new(0)),
            bytes_received: Arc::new(AtomicU64::new(0)),
            connections_total: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn add_bytes_sent(&self, bytes: u64) {
        self.bytes_sent.fetch_add(bytes, Ordering::Relaxed);
    }

    pub fn get_bytes_sent(&self) -> u64 {
        self.bytes_sent.load(Ordering::Relaxed)
    }
    
    // 其他统计方法...
}
```

### 5. 添加 Web 管理界面

使用 `axum` 或 `actix-web` 添加 HTTP 服务：

```toml
# Cargo.toml
[dependencies]
axum = "0.7"
```

```rust
// src/admin.rs
use axum::{Router, routing::get};

pub async fn start_admin_server(addr: &str) -> Result<()> {
    let app = Router::new()
        .route("/", get(|| async { "TLS Tunnel Admin" }))
        .route("/stats", get(get_stats));

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn get_stats() -> String {
    // 返回统计信息
    "Statistics".to_string()
}
```

---

## 性能优化

### 1. 使用 Release 模式
```bash
cargo build --release
```

### 2. 启用 LTO (Link Time Optimization)
```toml
# Cargo.toml
[profile.release]
lto = true
codegen-units = 1
```

### 3. 性能分析
```bash
# 使用 perf (Linux)
cargo build --release
perf record ./target/release/tls-tunnel server -c server.toml
perf report

# 使用 flamegraph
cargo install flamegraph
cargo flamegraph -- server -c server.toml
```

---

## 贡献指南

### 提交代码前检查清单
- [ ] 代码通过 `cargo fmt` 格式化
- [ ] 代码通过 `cargo clippy` 检查
- [ ] 添加必要的测试
- [ ] 更新相关文档
- [ ] 提交信息清晰明确

### Pull Request 流程
1. Fork 项目
2. 创建功能分支 (`git checkout -b feature/amazing-feature`)
3. 提交更改 (`git commit -m 'Add amazing feature'`)
4. 推送到分支 (`git push origin feature/amazing-feature`)
5. 创建 Pull Request

### 提交信息格式
```
<type>: <subject>

<body>

<footer>
```

类型:
- `feat`: 新功能
- `fix`: 修复 bug
- `docs`: 文档更新
- `style`: 代码格式调整
- `refactor`: 重构
- `test`: 添加测试
- `chore`: 构建/工具链更改

---

## 常见问题

### Q: 如何添加新的命令行参数？
A: 在 `cli.rs` 中修改 `Cli` 或 `Commands` 结构体，添加新字段。

### Q: 如何更改日志格式？
A: 在 `main.rs` 中修改 `tracing_subscriber` 的初始化代码。

### Q: 如何支持其他配置文件格式（如 JSON/YAML）？
A: 在 `Cargo.toml` 添加相应的 serde 序列化器，修改 `config.rs` 中的加载逻辑。

---

## 参考资源

- [Tokio 文档](https://tokio.rs/)
- [Rustls 文档](https://docs.rs/rustls/)
- [Rust 异步编程](https://rust-lang.github.io/async-book/)
- [Clap 文档](https://docs.rs/clap/)

---

## 联系方式

有问题或建议？欢迎提交 Issue！
