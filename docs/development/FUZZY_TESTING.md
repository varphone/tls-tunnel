# Fuzzy 测试文档

## 概述

Fuzzy 测试（模糊测试）是一种软件测试技术，通过向程序提供随机、畸形或意外的输入来发现程序的健壮性问题、安全漏洞和崩溃。本文档描述了 tls-tunnel 项目中的 fuzzy 测试用例及其目的。

## 测试用例

### 1. test_malformed_auth_message

**目的**：测试服务器对畸形认证消息的处理能力。

**测试内容**：
- 不完整的 JSON 消息
- 缺少必需参数的 JSON-RPC 请求
- 完全不是 JSON 的数据
- 空认证密钥
- 大量空字节

**预期行为**：服务器应该优雅地拒绝这些消息，记录错误，但不应崩溃。

---

### 2. test_oversized_messages

**目的**：测试服务器对超大消息的处理。

**测试内容**：
- 发送超过配置的 `max_request_size` 限制的消息（2MB vs 1MB 限制）

**预期行为**：
- 服务器应该检测到消息过大
- 拒绝或关闭连接
- 不应该因为内存耗尽而崩溃

---

### 3. test_rapid_connect_disconnect

**目的**：测试服务器处理快速连接和断开的能力。

**测试内容**：
- 在短时间内连接和断开 50 次
- 不发送任何数据就立即断开

**预期行为**：
- 服务器应该正确清理每个连接
- 不应该出现资源泄漏
- 不应该崩溃或变得不稳定

---

### 4. test_concurrent_connections

**目的**：测试服务器的并发处理能力。

**测试内容**：
- 同时创建 30 个并发连接
- 每个连接保持不同的时间
- 测试速率限制功能

**预期行为**：
- 服务器应该能够处理多个并发连接
- 速率限制应该正常工作
- 所有连接应该被正确处理和清理

---

### 5. test_incomplete_handshake

**目的**：测试服务器对不完整协议握手的处理。

**测试内容**：
- 建立 TLS 连接但不发送认证消息
- 立即关闭连接
- 重复 10 次

**预期行为**：
- 服务器应该检测到超时或连接关闭
- 正确清理未完成的握手
- 不应该泄漏资源

---

### 6. test_random_data_injection

**目的**：测试服务器对随机数据的鲁棒性。

**测试内容**：
- 发送 20 组随机生成的数据
- 每组数据长度在 1-1000 字节之间
- 数据内容完全随机

**预期行为**：
- 服务器应该能够解析并处理或拒绝随机数据
- 不应该因为意外输入而崩溃
- 应该记录解析错误

---

### 7. test_idle_connections

**目的**：测试服务器对空闲连接的处理。

**测试内容**：
- 创建 5 个连接
- 保持空闲状态 2 秒
- 不发送任何数据

**预期行为**：
- 服务器应该保持连接或在超时后关闭
- 不应该因为空闲连接而占用过多资源
- 应该有适当的超时机制

---

### 8. test_mixed_valid_invalid_messages

**目的**：测试服务器对混合有效和无效消息的处理。

**测试内容**：
- 先发送一个有效的认证请求
- 然后发送多个无效消息
- 测试状态管理和错误恢复

**预期行为**：
- 服务器应该正确处理有效消息
- 优雅地拒绝无效消息
- 不应该因为无效消息而影响整体功能

---

## 运行测试

### 运行所有 fuzzy 测试

```bash
cargo test --test fuzzy_tests -- --test-threads=1 --nocapture
```

### 运行单个测试

```bash
cargo test --test fuzzy_tests test_malformed_auth_message -- --nocapture
```

### 运行所有测试（包括 fuzzy 测试）

```bash
cargo test -- --test-threads=1
```

## 测试覆盖的安全方面

1. **输入验证**
   - JSON 解析
   - 协议格式验证
   - 参数检查

2. **资源管理**
   - 连接清理
   - 内存使用
   - 文件描述符管理

3. **DoS 防护**
   - 速率限制
   - 大小限制
   - 并发限制

4. **错误处理**
   - 异常输入处理
   - 协议错误恢复
   - 日志记录

5. **状态管理**
   - 连接状态跟踪
   - 认证状态
   - 错误状态恢复

## 添加新的 Fuzzy 测试

创建新的 fuzzy 测试时，应该考虑：

1. **明确测试目的**：每个测试应该针对特定的失败场景
2. **可重现性**：测试应该是确定性的（或使用固定的随机种子）
3. **资源清理**：确保测试后正确清理资源
4. **超时处理**：使用超时避免测试挂起
5. **断言服务器状态**：验证服务器在测试后仍然正常运行

### 示例模板

```rust
#[tokio::test]
async fn test_your_scenario() {
    let server_port = common::get_available_port();
    let auth_key = "test-fuzzy-your-case";

    let (cert_path, key_path) = common::generate_test_certs();
    let _cleanup = common::TestCleanup::new(cert_path.clone(), key_path.clone());

    // 启动服务器
    let server_config = ServerConfig {
        // ... 配置
    };
    
    let server_handle = tokio::spawn(async move {
        tls_tunnel::server::run_server(server_config, acceptor).await.ok();
    });

    sleep(Duration::from_millis(300)).await;

    // 执行测试逻辑
    // ...

    // 验证服务器仍在运行
    sleep(Duration::from_millis(500)).await;
    server_handle.abort();
}
```

## 持续改进

Fuzzy 测试应该持续更新以覆盖：
- 新发现的边界情况
- 用户报告的问题
- 安全研究发现的漏洞模式
- 新增功能的健壮性测试

## 相关资源

- [OWASP Fuzzing Guide](https://owasp.org/www-community/Fuzzing)
- [AFL Fuzzing](https://github.com/google/AFL)
- [Cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz)

## 未来计划

- [ ] 集成专业的 fuzzing 工具（如 cargo-fuzz）
- [ ] 添加更多协议层面的 fuzzy 测试
- [ ] 测试 yamux 多路复用的健壮性
- [ ] 添加性能基准测试
- [ ] 测试极端网络条件（延迟、丢包等）
