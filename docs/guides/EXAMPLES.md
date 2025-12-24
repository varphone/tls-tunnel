# 使用示例

## 示例 1: 本地测试

### 场景
在同一台电脑上测试 TLS 隧道功能。

### 步骤

#### 1. 生成测试证书
```powershell
.\generate-cert.ps1
```

#### 2. 启动测试 HTTP 服务器
```powershell
# 终端 1: 在 3000 端口启动一个简单的 HTTP 服务器
python -m http.server 3000
```

#### 3. 修改客户端配置
编辑 `client.toml`：
```toml
[client]
server_addr = "localhost"  # 本地测试
server_port = 8443
skip_verify = true         # 自签名证书需要设为 true
auth_key = "your-secret-auth-key-change-me"

[[proxies]]
name = "web"
publish_port = 8080  # 服务器监听 8080
local_port = 3000  # 转发到客户端 3000
```

#### 4. 启动服务器
```powershell
# 终端 2
.\target\release\tls-tunnel.exe server -c server.toml
```

#### 5. 启动客户端
```powershell
# 终端 3
.\target\release\tls-tunnel.exe client -c client.toml
```

#### 6. 测试访问
```powershell
# 浏览器访问
http://localhost:8080

# 或使用 curl
curl http://localhost:8080
```

### 预期结果
访问 `http://localhost:8080` 应该显示 Python HTTP 服务器的目录列表。

---

## 示例 2: 远程服务器部署

### 场景
- 服务器 A (公网 IP: 203.0.113.1)
- 客户端 B (内网，运行 Web 服务在 3000 端口)
- 目标：外网通过服务器 A 的 8080 端口访问客户端 B 的 3000 端口

### 服务器 A 配置

#### 1. 获取正式证书
使用 Let's Encrypt 或其他 CA 获取证书：
```bash
# 使用 certbot (示例)
sudo certbot certonly --standalone -d yourdomain.com
```

#### 2. 配置 server.toml
```toml
[server]
bind_addr = "0.0.0.0"
bind_port = 8443
cert_path = "/etc/letsencrypt/live/yourdomain.com/fullchain.pem"
key_path = "/etc/letsencrypt/live/yourdomain.com/privkey.pem"
auth_key = "your-secret-auth-key-change-me"

# 注意：服务器不需要配置代理，由客户端动态提供
```

#### 3. 启动服务器
```bash
./tls-tunnel server -c server.toml
```

#### 4. 配置防火墙
```bash
# 允许 8080 和 8443 端口
sudo ufw allow 8080/tcp
sudo ufw allow 8443/tcp
```

### 客户端 B 配置

#### 1. 配置 client.toml
```toml
[client]
server_addr = "yourdomain.com"  # 或 203.0.113.1
server_port = 8443
skip_verify = false  # 使用正式证书，不跳过验证
auth_key = "your-secret-auth-key-change-me"

[[proxies]]
name = "web"
local_port = 8080  # 服务器监听 8080
remote_port = 3000  # 客户端本地 3000
```

#### 2. 启动客户端
```bash
./tls-tunnel client -c client.toml
```

### 测试
从任何地方访问：
```bash
curl http://203.0.113.1:8080
# 或
curl http://yourdomain.com:8080
```

---

## 示例 3: 多端口转发

### 场景
同时转发多个服务：
- Web 服务 (3000 -> 8080)
- SSH 服务 (22 -> 2222)
- 数据库 (5432 -> 5432)

### 服务器配置
```toml
[server]
bind_addr = "0.0.0.0"
bind_port = 8443
cert_path = "cert.pem"
key_path = "key.pem"
auth_key = "your-secret-auth-key-change-me"

# 注意：服务器不需要配置代理，由客户端动态提供
```

### 客户端配置
```toml
[client]
server_addr = "server.example.com"
server_port = 8443
skip_verify = false
auth_key = "your-secret-auth-key-change-me"

[[proxies]]
name = "web"
publish_port = 8080  # 服务器监听 8080
local_port = 3000  # 客户端本地 3000

[[proxies]]
name = "ssh"
publish_port = 2222  # 服务器监听 2222
local_port = 22    # 客户端本地 22

[[proxies]]
name = "database"
publish_port = 5432   # 服务器监听 5432
local_port = 5432  # 客户端本地 5432
```

### 使用
```bash
# 访问 Web 服务
curl http://server.example.com:8080

# SSH 连接
ssh user@server.example.com -p 2222

# 数据库连接
psql -h server.example.com -p 5432 -U username -d database
```

---

## 示例 4: 使用 systemd 自动启动 (Linux)

### 创建服务器 systemd 服务
```bash
sudo nano /etc/systemd/system/tls-tunnel-server.service
```

内容：
```ini
[Unit]
Description=TLS Tunnel Server
After=network.target

[Service]
Type=simple
User=tls-tunnel
Group=tls-tunnel
WorkingDirectory=/opt/tls-tunnel
ExecStart=/opt/tls-tunnel/tls-tunnel server -c /opt/tls-tunnel/server.toml
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

### 创建客户端 systemd 服务
```bash
sudo nano /etc/systemd/system/tls-tunnel-client.service
```

内容：
```ini
[Unit]
Description=TLS Tunnel Client
After=network.target

[Service]
Type=simple
User=tls-tunnel
Group=tls-tunnel
WorkingDirectory=/opt/tls-tunnel
ExecStart=/opt/tls-tunnel/tls-tunnel client -c /opt/tls-tunnel/client.toml
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

### 启用和启动服务
```bash
# 重载 systemd
sudo systemctl daemon-reload

# 启用开机自启
sudo systemctl enable tls-tunnel-server  # 或 tls-tunnel-client

# 启动服务
sudo systemctl start tls-tunnel-server   # 或 tls-tunnel-client

# 查看状态
sudo systemctl status tls-tunnel-server  # 或 tls-tunnel-client

# 查看日志
sudo journalctl -u tls-tunnel-server -f  # 或 tls-tunnel-client
```

---

## 示例 5: Windows 服务 (使用 NSSM)

### 1. 下载 NSSM
从 https://nssm.cc/download 下载 NSSM

### 2. 安装服务器服务
```powershell
nssm install TLSTunnelServer "C:\tls-tunnel\tls-tunnel.exe" "server -c C:\tls-tunnel\server.toml"
nssm set TLSTunnelServer AppDirectory "C:\tls-tunnel"
nssm start TLSTunnelServer
```

### 3. 安装客户端服务
```powershell
nssm install TLSTunnelClient "C:\tls-tunnel\tls-tunnel.exe" "client -c C:\tls-tunnel\client.toml"
nssm set TLSTunnelClient AppDirectory "C:\tls-tunnel"
nssm start TLSTunnelClient
```

### 4. 管理服务
```powershell
# 查看状态
nssm status TLSTunnelServer

# 停止服务
nssm stop TLSTunnelServer

# 重启服务
nssm restart TLSTunnelServer

# 删除服务
nssm remove TLSTunnelServer confirm
```

---

## 故障排查

### 问题 1: 连接失败
```
Error: Failed to connect to server
```

**解决方案**:
1. 检查服务器是否运行：`netstat -an | grep 8443`
2. 检查防火墙规则
3. 验证服务器地址和端口配置
4. 检查 TLS 证书是否有效

### 问题 2: 证书验证失败
```
Error: TLS handshake failed
```

**解决方案**:
1. 如果使用自签名证书，设置 `skip_verify = true`
2. 检查证书是否过期
3. 确认服务器名称与证书 CN 匹配
4. 使用 `--log-level debug` 查看详细日志

### 问题 3: 端口占用
```
Error: Failed to bind port
```

**解决方案**:
1. 检查端口是否被占用：
   ```powershell
   # Windows
   netstat -ano | findstr :8080
   
   # Linux
   lsof -i :8080
   ```
2. 更换配置文件中的端口号
3. 关闭占用端口的程序

### 问题 4: 权限不足
```
Error: Permission denied
```

**解决方案**:
1. Linux/macOS: 使用 `sudo` 或绑定高于 1024 的端口
2. Windows: 以管理员身份运行
3. 确保证书文件有读取权限

---

## 性能调优

### 1. 系统限制
```bash
# Linux: 增加文件描述符限制
ulimit -n 65535

# 编辑 /etc/security/limits.conf
* soft nofile 65535
* hard nofile 65535
```

### 2. 日志级别
生产环境使用较低的日志级别：
```bash
tls-tunnel --log-level warn server -c server.toml
```

### 3. 监控
使用日志监控工具：
```bash
# 实时查看连接
journalctl -u tls-tunnel-server -f | grep "Accepted connection"

# 统计连接数
journalctl -u tls-tunnel-server | grep "Accepted connection" | wc -l
```

---

## 安全最佳实践

1. **使用正式 CA 证书**: 不要在生产环境使用自签名证书
2. **定期更新证书**: 设置自动更新机制
3. **限制访问**: 使用防火墙限制来源 IP
4. **最小权限**: 使用专门的用户运行服务
5. **监控日志**: 定期检查异常连接
6. **定期更新**: 保持依赖库和程序最新

---

## 更多示例

查看 [QUICKSTART.md](QUICKSTART.md) 获取更多快速开始指南。
查看 [README.md](../../README.md) 获取完整文档。
查看 [ARCHITECTURE.md](../development/ARCHITECTURE.md) 了解架构设计。
