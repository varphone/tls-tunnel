# TLS Tunnel 快速开始指南

## 1. 生成测试证书

```powershell
# 在项目根目录执行
.\generate-cert.ps1
```

或者如果你在 Linux/macOS 上：

```bash
chmod +x generate-cert.sh
./generate-cert.sh
```

## 2. 启动一个测试 Web 服务器（模拟客户端的服务）
cd examples\certs
./generate-cert.ps1
在一个新的终端窗口中：

```powershell
# 使用 Python 启动一个简单的 HTTP 服务器在 3000 端口
python -m http.server 3000
cd examples/certs
chmod +x generate-cert.sh
./generate-cert.sh
## 3. 启动服务器端

在另一个终端窗口中：

```powershell
# 修改 server.toml 中的配置（如果需要）
# 然后运行服务器
# 修改 examples/server.toml 中的配置（如果需要）
# 然后运行服务器
.\target\release\tls-tunnel.exe server -c examples/server.toml

或者使用 cargo run：

```powershell
cargo run --release -- server -c server.toml
cargo run --release -- server -c examples/server.toml

## 4. 启动客户端

在第三个终端窗口中：

```powershell
# 修改 client.toml：
# - 将 server_addr 改为 "localhost" 或实际服务器地址
# 修改 examples/client.toml：

.\target\release\tls-tunnel.exe client -c client.toml
```
.\target\release\tls-tunnel.exe client -c examples/client.toml
或者使用 cargo run：

```powershell
cargo run --release -- client -c client.toml
```
cargo run --release -- client -c examples/client.toml
## 5. 测试连接

现在你可以通过服务器的 8080 端口访问客户端机器上 3000 端口的服务：

```powershell
# 在浏览器中访问
http://localhost:8080

# 或使用 curl
curl http://localhost:8080

## 架构说明

```
[你的浏览器] ---> [服务器:8080] 
                      |
                      | (TLS 加密隧道 :8443)
                      |
                  [客户端:3000] ---> [本地 HTTP 服务器]
```

## 配置修改建议

### 测试环境配置

修改 `client.toml`：

```toml
[client]
server_addr = "localhost"  # 如果在本地测试
server_port = 8443
skip_verify = true          # 自签名证书需要设为 true
auth_key = "your-secret-auth-key-change-me"

[[proxies]]
name = "web"
publish_port = 8080  # 服务器上外部访问的端口
local_port = 3000  # 客户端本地服务端口
```

### 生产环境配置

1. 获取正式的 TLS 证书（如 Let's Encrypt）
2. 将 `skip_verify` 设为 `false`
3. 使用实际的服务器域名
4. 配置防火墙规则

## 常见问题

### Q: 连接失败怎么办？

A: 检查以下几点：
4. 客户端配置中的 `skip_verify` 是否设为 `true`（自签名证书）

### Q: 如何查看详细日志？

A: 使用 `--log-level debug` 参数：

```powershell
.\target\release\tls-tunnel.exe --log-level debug server -c examples/server.toml
```

### Q: 如何配置多个代理？

A: 在客户端配置文件中添加多个 `[[proxies]]` 部分。

## 下一步

- 阅读 [README.md](../../README.md) 了解更多详情
- 查看源代码了解实现细节
- 根据需求定制配置
