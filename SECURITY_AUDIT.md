# 安全审计报告

**审计日期**: 2025-12-25  
**审计范围**: tls-tunnel 正向代理功能  
**严重等级**: 🔴 高危 | 🟡 中危 | 🟢 低危 | ℹ️ 建议

---

## 🔴 高危漏洞

### 1. ⚠️ 客户端 Forwarder 缺少绑定地址验证（开放代理风险）

**文件**: [src/client/forwarder.rs](src/client/forwarder.rs#L23)

**问题描述**:
```rust
let bind_addr = format!("{}:{}", forwarder.bind_addr, forwarder.bind_port);
let listener = TcpListener::bind(&bind_addr).await?;
```

客户端允许 forwarder 绑定到任意地址（包括 `0.0.0.0`），这会将代理暴露给局域网甚至公网，造成**开放代理滥用**。

**风险**:
- 任何人可以使用你的代理访问互联网
- 可能被滥用进行攻击、刷量、爬虫等非法活动
- IP 地址可能被列入黑名单

**修复建议**:
```rust
// 在配置验证时检查 forwarder 绑定地址
if forwarder.bind_addr != "127.0.0.1" && forwarder.bind_addr != "localhost" {
    warn!(
        "Forwarder '{}': Binding to {} exposes proxy to network! \
         Consider using 127.0.0.1 for localhost-only access.",
        forwarder.name, forwarder.bind_addr
    );
    // 可选：要求用户明确设置 allow_external_bind = true
}
```

**状态**: ❌ 未修复

---

### 2. ⚠️ 缺少连接速率限制（DoS 风险）

**文件**: [src/client/forwarder.rs](src/client/forwarder.rs#L35-L60)

**问题描述**:
```rust
loop {
    match listener.accept().await {
        Ok((local_stream, peer_addr)) => {
            tokio::spawn(async move { ... });
        }
        Err(e) => {
            sleep(Duration::from_millis(100)).await;  // 仅在错误时等待
        }
    }
}
```

接受连接的循环没有速率限制，攻击者可以：
- 发起大量连接耗尽系统资源（文件描述符、内存、CPU）
- 导致合法用户无法使用
- 造成服务器端资源耗尽

**风险**:
- 资源耗尽 DoS 攻击
- 内存泄漏（每个连接会分配缓冲区）
- 进程崩溃

**修复建议**:
```rust
use tokio::sync::Semaphore;
use std::sync::Arc;

// 限制最大并发连接数
let max_connections = Arc::new(Semaphore::new(1000));

loop {
    let permit = max_connections.clone().acquire_owned().await.unwrap();
    match listener.accept().await {
        Ok((local_stream, peer_addr)) => {
            tokio::spawn(async move {
                let _permit = permit; // 持有 permit 直到任务结束
                handle_forwarder_connection(...).await;
            });
        }
        ...
    }
}
```

**状态**: ❌ 未修复

---

### 3. ⚠️ 直连功能缺少安全检查（SSRF 风险）

**文件**: [src/client/forwarder.rs](src/client/forwarder.rs#L417)

**问题描述**:
```rust
async fn handle_direct_connection(
    mut local_stream: TcpStream,
    target: &str,
    forwarder_name: &str,
) -> Result<()> {
    let mut remote_stream = TcpStream::connect(target).await?;  // ❌ 无验证
    ...
}
```

客户端直连功能没有验证目标地址，允许连接到：
- `127.0.0.1`（本机服务）
- `169.254.169.254`（云服务元数据服务器）
- 内网地址（`10.x.x.x`, `192.168.x.x`, `172.16-31.x.x`）

**风险**:
- SSRF（服务器端请求伪造）攻击
- 访问内网服务（数据库、Redis、内部 API）
- 窃取云服务凭证（AWS/Azure/GCP 元数据）
- 端口扫描内网

**修复建议**:
```rust
async fn handle_direct_connection(
    mut local_stream: TcpStream,
    target: &str,
    forwarder_name: &str,
) -> Result<()> {
    // 复用服务器端的安全检查逻辑
    if is_local_or_private_address(target) {
        warn!(
            "Forwarder '{}': Blocked direct connection to local/private address: {}",
            forwarder_name, target
        );
        return Err(anyhow::anyhow!(
            "Direct connection to local/private addresses is not allowed"
        ));
    }
    
    let mut remote_stream = TcpStream::connect(target).await?;
    ...
}
```

**状态**: ❌ 未修复

---

## 🟡 中危问题

### 4. 缺少请求超时机制

**文件**: [src/client/forwarder.rs](src/client/forwarder.rs#L230-L242)

**问题描述**:
```rust
loop {
    stream.read_exact(&mut temp).await?;  // ❌ 无超时
    buffer.push(temp[0]);
    
    if buffer.len() > 8192 {  // ✅ 有长度限制
        anyhow::bail!("HTTP request too long");
    }
}
```

虽然有长度限制，但没有超时机制。慢速攻击者可以：
- 每秒发送 1 字节，保持连接 8192 秒（2.2 小时）
- 耗尽连接池资源

**修复建议**:
```rust
use tokio::time::{timeout, Duration};

let result = timeout(Duration::from_secs(30), async {
    loop {
        stream.read_exact(&mut temp).await?;
        buffer.push(temp[0]);
        if buffer.len() > 8192 {
            anyhow::bail!("HTTP request too long");
        }
    }
}).await??;
```

**状态**: ❌ 未修复

---

### 5. SOCKS5 缺少域名长度验证

**文件**: [src/client/forwarder.rs](src/client/forwarder.rs#L328-L335)

**问题描述**:
```rust
0x03 => {
    // 域名
    let mut len = [0u8; 1];
    stream.read_exact(&mut len).await?;
    let len = len[0] as usize;  // ❌ 未验证长度范围
    
    let mut domain = vec![0u8; len];  // 潜在的大内存分配
    stream.read_exact(&mut domain).await?;
    String::from_utf8(domain)?
}
```

虽然 SOCKS5 协议限制域名长度为 255 字节（u8），但代码未显式验证。

**风险**: 中等（协议本身限制了风险）

**修复建议**:
```rust
0x03 => {
    let mut len = [0u8; 1];
    stream.read_exact(&mut len).await?;
    let len = len[0] as usize;
    
    if len == 0 || len > 255 {  // 显式验证
        anyhow::bail!("Invalid SOCKS5 domain name length: {}", len);
    }
    
    let mut domain = vec![0u8; len];
    stream.read_exact(&mut domain).await?;
    String::from_utf8(domain)?
}
```

**状态**: ⚠️ 部分缓解（协议限制）

---

### 6. 服务器端 visitor 名称长度限制不足

**文件**: [src/server/visitor.rs](src/server/visitor.rs#L107)

**问题描述**:
```rust
if name_len == 0 || name_len > 256 {  // ✅ 有限制但较宽松
    let error_msg = "Invalid proxy name length";
    ...
}
```

256 字节的限制仍可能造成日志注入或缓冲区浪费。

**修复建议**:
```rust
if name_len == 0 || name_len > 64 {  // 更严格的限制
    let error_msg = "Invalid proxy name length (max 64 bytes)";
    ...
}
```

**状态**: ⚠️ 部分缓解

---

## 🟢 低危问题

### 7. 缺少认证密钥强度检查

**文件**: [src/config.rs](src/config.rs#L166)

**问题描述**:
配置中的 `auth_key` 字段没有最小长度要求，用户可能设置弱密码如 `"123"`。

**修复建议**:
```rust
impl ServerConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        // 验证认证密钥强度
        if self.auth_key.len() < 16 {
            bail!("auth_key must be at least 16 characters for security");
        }
        ...
    }
}
```

**状态**: ⚠️ 建议修复

---

### 8. DNS 解析可能泄漏隐私

**文件**: [src/client/geoip.rs](src/client/geoip.rs#L91)

**问题描述**:
```rust
if let Ok(addrs) = (host, 0).to_socket_addrs() {  // 使用系统 DNS
    for addr in addrs {
        let ip = addr.ip();
        ...
    }
}
```

在判断路由策略时会进行 DNS 解析，可能泄漏用户意图。

**建议**: 在文档中说明此行为，建议用户使用 IP 白名单而非域名白名单。

**状态**: ℹ️ 文档改进

---

## ℹ️ 其他建议

### 9. 增加审计日志

建议记录以下事件：
```rust
// 记录敏感操作
info!(
    "Forwarder '{}': Connection from {} to {} (via {}) - User-Agent: {}",
    forwarder.name, peer_addr, target, 
    if direct { "direct" } else { "proxy" },
    user_agent  // 如果是 HTTP
);
```

### 10. 添加配置文件安全检查

在启动时检查配置文件权限：
```bash
# 提示用户保护配置文件
chmod 600 config.toml  # 仅所有者可读写
```

### 11. 支持白名单模式

考虑添加 `allowed_targets` 配置：
```toml
[forwarders.security]
mode = "whitelist"  # 或 "blacklist"
allowed_domains = ["*.example.com", "safe-api.com"]
allowed_ips = ["8.8.8.8", "1.1.1.1"]
```

---

## 修复优先级

| 优先级 | 漏洞编号 | 描述 | 影响 |
|--------|---------|------|------|
| **P0** | #1, #2, #3 | 开放代理、DoS、SSRF | 高危，可被远程利用 |
| **P1** | #4 | 慢速攻击 | 中危，影响可用性 |
| **P2** | #5, #6, #7 | 输入验证、认证强度 | 低-中危，纵深防御 |
| **P3** | #8-11 | 日志、审计、最佳实践 | 安全加固 |

---

## 检查清单

- [ ] 限制 forwarder 默认只能绑定到 localhost
- [ ] 添加并发连接数限制（Semaphore）
- [ ] 在客户端直连功能中添加地址安全检查
- [ ] 为所有网络操作添加超时机制
- [ ] 验证所有用户输入长度和范围
- [ ] 强制认证密钥最小长度
- [ ] 添加速率限制（rate limiting）
- [ ] 记录安全相关的审计日志
- [ ] 编写安全配置文档
- [ ] 添加单元测试覆盖安全场景

---

## 总结

当前实现存在 **3 个高危漏洞**，主要涉及：
1. **开放代理风险** - 可导致 IP 被滥用
2. **DoS 攻击面** - 缺少资源限制
3. **SSRF 漏洞** - 客户端直连未验证

建议**立即修复**高危问题后再发布到生产环境。
