# 变更日志

## [1.5.1] - 2025-12-27

### 修复
- 🐛 **编译修复**
  * 添加缺失的 anyhow::Context 导入到 service.rs
  * 添加缺失的 std::path::Path 导入到 service.rs
  * 修复 Linux 平台 systemd 服务注册/注销功能的编译问题

## [1.5.0] - 2025-12-27

### 新增
- ✨ **全面的集成测试套件**
  * 添加完整的集成测试用例，覆盖主要功能场景
  * visitor 和 forwarder 模式的集成测试
  * 代理绑定异常通知测试

- 🧪 **Fuzzy 测试框架**
  * 创建 8 个综合性 fuzzy 测试用例，验证服务器可靠性
  * test_malformed_auth_message - 测试畸形消息处理
  * test_oversized_messages - 测试超大消息限制
  * test_rapid_connect_disconnect - 测试快速连接/断开
  * test_concurrent_connections - 测试并发连接
  * test_incomplete_handshake - 测试不完整握手
  * test_random_data_injection - 测试随机数据注入
  * test_idle_connections - 测试空闲连接
  * test_mixed_valid_invalid_messages - 测试混合有效/无效消息
  * 添加详细的 fuzzy 测试文档（docs/development/FUZZY_TESTING.md）

- 📢 **异常通知功能**
  * 实现异常通知和代理绑定失败通知功能
  * 增强系统可观测性和问题诊断能力

### 改进
- 🔧 **架构重构**
  * 重构客户端架构，使用统一事件循环和 ClientWorld 结构体
  * 重构服务端架构，使用统一事件循环和 ServerWorld 结构体
  * 重构 CLI 模块，创建 cli 子模块结构
  * 重组文档结构，将开发文档移至 docs/development 目录

- 🛠️ **错误消息优化**
  * 改进 TLS 连接错误消息，提供更好的故障排查信息
  * 增强证书验证失败的错误提示
  * 添加详细的错误原因和解决建议

- 🔒 **安全性改进**
  * 改进 forwarder 直接连接的安全检查
  * 支持路由规则覆盖 SSRF 保护（bypass_safety_check）
  * 修正 test_forwarder_http_proxy 和 test_forwarder_socks5_proxy 测试用例

- 🌐 **协议兼容性**
  * 增强协议向前和向后兼容性
  * 修复 visitor 配置验证问题
  * 修复服务端未处理 inbound stream 导致 forwarder 功能失效

### 修复
- 🐛 **Bug 修复**
  * 修复 visitor 模式测试 - 服务器向客户端B发送 publish_port
  * 修复客户端和服务端 yamux 连接未持续 poll 的问题
  * 修复所有 clippy 警告和错误
  * 删除客户端和服务器中的无用代码

### 文档
- 📝 **文档更新**
  * 更新示例配置文件
  * 添加 fuzzy 测试文档
  * 改进开发文档组织结构

### 测试
- ✅ **测试覆盖**
  * 总计 96 个测试用例全部通过
  * 68 个单元测试
  * 10 个集成测试
  * 8 个 fuzzy 测试
  * 4 个控制协议测试
  * 4 个代理绑定异常测试
  * 2 个文档测试

## [1.4.1] - 2025-12-26

### 改进
- 📊 **流量统计实现优化**
  * 在 stream 处理层实现流量统计，避免代码重复
  * 使用 copy_with_stats 函数在数据拷贝时自动记录统计信息
  * 支持上传和下载流量分别统计
  * 删除未使用的 stats_stream 模块

## [1.4.0] - 2025-12-26

### 新增
- ✨ **ForwarderHandler 重构**
  * 新增 ForwarderHandler 结构体，统一管理 Forwarder 代理处理
  * 实现 ProxyHandler trait，支持统一的处理器管理接口
  * 完整的生命周期管理（Start、Stop、HealthCheck）

### 改进
- 📝 **代码质量改进**
  * 清理编译警告，提高代码质量
  * 优化代码格式和结构
  * 增强代码可维护性

## [1.3.3] - 2025-12-25

### 修复
- 🐛 **服务器统计准确性修复**
  * 修正发送/接收方向标记错误（数据方向完全反了）
  * 修复严重的数据统计不完整问题：
    - 原因：使用 `tokio::select!` 导致一个方向完成时取消另一个方向
    - 后果：只统计先完成方向的数据，显示几十 KB 实际传输几十 MB
    - 解决：改用 `tokio::join!` 确保两个方向完整传输
  * 现在可以准确统计带宽使用和性能分析

## [1.3.2] - 2025-12-25

### 新增
- 🔌 **SSH 代理类型支持**
  * 新增 `ProxyType::Ssh` 代理类型
  * 为 SSH 连接启用 TCP_NODELAY，降低交互延迟
  * 在客户端、服务端和访客模式下全面支持
  * 禁用 Nagle 算法，优先降低延迟而非提高吞吐量

### 修复
- 🐛 **maxminddb 0.27 API 适配**
  * 适配 maxminddb 0.27 版本的 API 变更
  * 修正 `LookupResult.decode()` 方法调用
  * 更新 Country 数据结构访问方式
- 🛡️ **Forwarder 安全检查改进**
  * 修正域名安全检查的误判问题
  * 正确处理包含 "localhost" 字符串的合法公网域名（如 localhost.weixin.qq.com）
  * 改进 DNS 解析失败的处理逻辑，避免误拦截
  * 增强 IPv6 地址和端口号的解析逻辑

## [1.3.1] - 2025-12-25

### 改进
- 📝 **日志格式优化**
  * 优化 systemd 环境下的日志输出格式
  * 自动检测 systemd 环境（通过 `INVOCATION_ID` 或 `JOURNAL_STREAM` 环境变量）
  * systemd 环境下禁用应用时间戳（避免与 systemd journal 的时间戳重复）
  * 更简洁的日志输出，符合 systemd 最佳实践
- 📊 **完成度统计更新**
  * 更新 IMPROVEMENTS.md 项目完成度到 80%
  * 配置管理完成度从 33% 提升到 67%（配置验证已实现）
  * 修正各类别完成度统计

## [1.3.0] - 2025-12-25

### 新增
- ✨ **客户端统计功能**
  * 添加 `stats_port` 和 `stats_addr` 配置选项到客户端配置
  * 实现客户端 HTTP 统计服务器（与服务端架构一致）
  * 提供 `/stats` JSON API 和 `/` HTML 可视化页面
  * 支持使用 `tls-tunnel top --url http://localhost:9091` 查看客户端统计
  * 实时跟踪每个代理的连接数、流量、运行时长等指标
- 🌍 **GeoIP 智能路由**
  * 基于地理位置的智能路由功能
  * 支持按国家/地区选择转发策略
  * 内置 GeoIP 数据库支持
- 🔀 **HTTP/SOCKS5 转发代理**
  * 支持 HTTP CONNECT 代理
  * 支持 SOCKS5 代理协议
  * 包含安全防护机制
- 🎯 **高级路由策略**
  * IP/CIDR 范围匹配
  * 域名通配符支持
  * 灵活的路由规则配置
- 🔧 **增强日志控制**
  * 支持 `RUST_LOG` 环境变量控制日志级别
  * 支持更精细的模块级别日志控制
  * 优先级：`RUST_LOG` 环境变量 > `--verbose` 标志

### 改进
- 🔒 **安全加固**（Phase 3）
  * 修复所有已知安全漏洞（P0-P3）
  * 增强配置验证机制
  * 添加速率限制支持
  * 改进错误处理
- 📊 **统计功能整合**
  * 移除独立的 `client-dashboard` 命令
  * 统一使用 `top` 命令查看服务端/客户端统计
  * 整合统计文档到 `docs/STATISTICS.md`（中文版）
- 📚 **文档改进**
  * 新增客户端统计快速入门文档
  * 新增示例配置 `examples/client-with-stats.toml`
  * 统一中文文档

### 删除
- 🗑️ 移除独立的统计实现文档（已整合）

## [1.2.3] - 2025-12-25

### 修复
- 🐛 修复 top 命令的统计 API 路径错误
  * 将请求路径从 /api/stats 改为 /stats
  * 与服务器端实际的 API 路径保持一致

## [1.2.2] - 2025-12-25

### 修复
- 🐛 修复 WebSocket 传输在 Nginx 反向代理后的连接问题
  * 使用实际服务器地址作为 Host header 而不是 localhost
  * 确保 Nginx 反向代理能正确识别和转发 WebSocket 请求
- 🔧 统一使用 publish_port 进行端口匹配，与 visitor 模式保持一致
  * 客户端使用 publish_port 查找代理配置
  * 连接池键改为 publish_port
  * 简化逻辑，避免端口匹配混乱

### 新增
- ✨ 添加 stats_addr 配置选项
  * 支持独立配置统计服务器绑定地址
  * 未配置时自动回退使用 bind_addr
  * 增强统计服务器的安全性和灵活性
- ✅ 增强配置验证机制
  * 验证 bind_addr 和 stats_addr 不为空
  * 在启动前检测配置错误，避免运行时问题

## [1.2.1] - 2025-12-25

### 修复
- 🐛 修复 systemd 服务注册中的错误变量名。
- 🔧 将 OpenSSL 依赖移到仅 Unix 平台，避免 Windows 编译问题。

## [1.2.0] - 2025-12-25

### 新增
- 📚 详细的架构文档 (`docs/ARCHITECTURE.md`)，包含系统拓扑、工作流程、模块依赖和设计特点说明。

### 改进
- ♻️ 将 `server.rs` 拆分为 7 个专用模块（registry, config, connection, yamux, visitor, stats, mod），显著提高代码可维护性和可读性。
- ♻️ 将 `client.rs` 拆分为 5 个专用模块（config, connection, visitor, stream, mod），增强模块化结构。
- 📖 使用 Mermaid 图表替代 ASCII 艺术，包括系统拓扑图、时序图、流程图、架构分层等（10+ 个可视化图表）。
- 📄 简化 README.md 架构部分，保持清晰简洁，详细说明链接至独立的架构文档。
- 🔧 修复 WSS 传输层的类型不匹配问题。

### 修复
- 🐛 修复 Visitor 模式时序图的 Mermaid 语法错误。

## [1.0.0] - 2025-12-24

### 新增
- ✨ 基于 Rust/Rustls/Tokio 的 TLS 1.3 隧道，单连接多路复用（Yamux）。
- 🔑 共享密钥认证，服务器拒绝未授权客户端。
- 📦 动态代理配置由客户端下发，服务器无需预配代理表。
- 🛠️ `generate` / `check` 命令，便捷生成与校验配置。
- 🌊 本地连接池（预热、清理、超时配置），池失败时自动回退直连。

### 改进
- 🏷️ 端口字段更直观：`publish_port`（服务器对外）/`local_port`（客户端本地）。
- 🔄 客户端/服务器重连与监听清理：断线后释放端口，重连可用。
- 🧰 配置与证书示例归档至 `examples/`（含 `examples/certs`）。
- 🧪 环境变量统一前缀 `TLS_TUNNEL_`，可调重连和连接池参数。

### 安全
- 🔒 TLS 1.3 默认启用；可选跳过验证仅限开发环境。
- 📛 自定义认证密钥；支持正式 CA 证书或自签证书。

1. **防止未授权访问**：
   - 只有拥有正确密钥的客户端才能连接
   - 认证失败会被记录和拒绝

2. **攻击防护**：
   - 限制密钥长度（最大 1024 字节）
   - 防止缓冲区溢出攻击
   - 记录所有可疑活动

3. **日志监控**：
   ```bash
   # 查看认证失败的尝试
   grep "Authentication failed" server.log
   ```

## 📚 相关文档

- [README.md](README.md) - 项目总览
- [PROTOCOL.md](docs/development/PROTOCOL.md) - 协议详细说明
- [QUICKSTART.md](docs/guides/QUICKSTART.md) - 快速开始
- [EXAMPLES.md](docs/guides/EXAMPLES.md) - 使用示例

## 🐛 已知问题

无

## 🚀 未来计划

1. 支持密钥轮换
2. 添加连接池管理
3. 实现心跳机制
4. 支持客户端证书认证（双向 TLS）
5. 添加流量统计
6. Web 管理界面

## 📞 反馈

如有问题或建议，欢迎提交 Issue！

---

**版本**: v0.1.0  
**更新日期**: 2025年12月24日  
**破坏性变更**: 是  
**建议操作**: 建议所有用户升级以获得更好的安全性
