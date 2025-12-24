auth_key = "your-secret-auth-key-change-me"
auth_key = "secret-key"
auth_key = "use-a-very-strong-secret-key-here-at-least-32-chars"
# 变更日志

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
