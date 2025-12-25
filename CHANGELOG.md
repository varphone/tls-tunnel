# 变更日志

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
