# Testing Guide - TLS Tunnel

本指南说明如何测试 TLS Tunnel 程序。

## 准备工作

### 1. 编译程序

```bash
cargo build --release
```

编译后的二进制文件位于 `target/release/tls-tunnel` (Linux/macOS) 或 `target/release/tls-tunnel.exe` (Windows)。

### 2. 生成测试证书

```bash
# 生成私钥
openssl genrsa -out key.pem 2048

# 生成自签名证书（使用 localhost）
openssl req -new -x509 -key key.pem -out cert.pem -days 365 -subj "/CN=localhost"
```

### 3. 准备测试服务

我们将使用 Python 的简单 HTTP 服务器作为测试：

```bash
# 在客户端机器上运行（端口 3000）
python3 -m http.server 3000
```

或者使用 Node.js：

```bash
npx http-server -p 3000
```

## 测试场景

### 场景 1：本地测试（同一台机器）

#### 步骤 1：配置服务器

创建或使用 `server.toml`：

```toml

[server]
bind_addr = "127.0.0.1"
bind_port = 8443
cert_path = "cert.pem"
key_path = "key.pem"
auth_key = "test-secret-key-123"
```

#### 步骤 2：配置客户端

创建或使用 `client.toml`：

```toml

[client]
server_addr = "127.0.0.1"
server_port = 8443
skip_verify = true
auth_key = "test-secret-key-123"

[[proxies]]
name = "test-http"
publish_port = 8080
local_port = 3000
```

#### 步骤 3：启动本地 HTTP 服务

```bash
python3 -m http.server 3000
```

#### 步骤 4：启动服务器

在新终端中：

```bash
./target/release/tls-tunnel -c server.toml server
```

你应该看到类似的输出：

```
INFO Starting TLS tunnel server on 127.0.0.1:8443
INFO Server listening on 127.0.0.1:8443
INFO Waiting for client connections...
```

#### 步骤 5：启动客户端

在另一个新终端中：

```bash
./target/release/tls-tunnel -c client.toml client
```

你应该看到：

```
INFO Starting TLS tunnel client, connecting to 127.0.0.1:8443
INFO Connected to server: 127.0.0.1:8443
INFO TLS handshake completed
INFO Sent authentication key
INFO Authentication successful
INFO Sent proxy config 'test-http': local=8080, remote=3000
INFO Sent all proxy configurations
INFO Yamux connection established
```

在服务器终端，你应该看到：

```
INFO Accepted connection from 127.0.0.1:xxxxx
INFO TLS handshake completed with 127.0.0.1:xxxxx
INFO Client 127.0.0.1:xxxxx authenticated successfully
INFO Client has 1 proxy configurations
INFO Proxy 'test-http': 127.0.0.1:8080 -> client:3000
INFO Yamux connection established with 127.0.0.1:xxxxx
INFO Proxy 'test-http' listening on 127.0.0.1:8080 (forwarding to client 127.0.0.1:xxxxx:3000)
```

#### 步骤 6：测试连接

在新终端中测试：

```bash
curl http://127.0.0.1:8080
```

你应该看到 Python HTTP 服务器返回的目录列表。

### 场景 2：多代理测试

修改 `client.toml` 添加多个代理：

```toml
[[proxies]]
name = "service-1"
publish_port = 8080
local_port = 3000

[[proxies]]
name = "service-2"
publish_port = 8081
local_port = 3001
```

启动多个本地服务：

```bash
# 终端 1
python3 -m http.server 3000

# 终端 2
python3 -m http.server 3001
```

然后启动服务器和客户端，测试两个端口：

```bash
curl http://127.0.0.1:8080
curl http://127.0.0.1:8081
```

### 场景 3：跨机器测试

#### 在服务器机器上：

1. 修改 `server.toml`：

```toml
[server]
bind_addr = "0.0.0.0"  # 监听所有接口
```

2. 确保防火墙允许端口 8443 和 8080

3. 启动服务器：

```bash
./tls-tunnel -c server.toml server
```

#### 在客户端机器上：

1. 修改 `client.toml`：

```toml
[client]
server_addr = "your-server-ip"  # 替换为服务器的实际 IP
server_port = 8443
```

2. 启动本地服务（如 Python HTTP server）

3. 启动客户端：

```bash
./tls-tunnel -c client.toml client
```

#### 在第三台机器上测试：

```bash
curl http://your-server-ip:8080
```

## 测试检查清单

- [ ] TLS 握手成功
- [ ] 认证通过
- [ ] Yamux 连接建立
- [ ] 代理监听器启动
- [ ] 数据转发正常
- [ ] 多个并发连接正常工作
- [ ] 多个代理同时工作
- [ ] 连接断开后清理资源

## 调试技巧

### 启用详细日志

```bash
./tls-tunnel -c config.toml --log-level debug server
./tls-tunnel -c config.toml --log-level debug client
```

### 使用 tcpdump 监控流量

```bash
# 监控 TLS 端口
sudo tcpdump -i any -n port 8443

# 监控代理端口
sudo tcpdump -i any -n port 8080
```

### 使用 netstat 检查连接

```bash
# Linux
netstat -tlnp | grep tls-tunnel

# macOS
netstat -an | grep LISTEN | grep 8443

# Windows PowerShell
netstat -ano | Select-String "8443"
```

## 常见问题

### 问题 1：连接被拒绝

**症状**：
```
Error: Failed to connect to server 127.0.0.1:8443
```

**解决方案**：
- 确保服务器正在运行
- 检查服务器地址和端口是否正确
- 检查防火墙设置

### 问题 2：认证失败

**症状**：
```
ERROR Authentication failed
```

**解决方案**：
- 确保服务器和客户端的 `auth_key` 完全一致
- 注意空格和大小写

### 问题 3：本地服务连接失败

**症状**：
```
ERROR Failed to connect to local service 127.0.0.1:3000
```

**解决方案**：
- 确保本地服务正在运行
- 确保端口号正确
- 使用 `netstat` 或 `lsof` 检查端口是否在监听

### 问题 4：端口已被占用

**症状**：
```
ERROR Failed to bind port 8080
```

**解决方案**：
- 使用不同的端口
- 或关闭占用该端口的程序

## 性能测试

### 使用 ab (Apache Bench)

```bash
# 测试基本性能
ab -n 1000 -c 10 http://127.0.0.1:8080/

# 测试并发性能
ab -n 10000 -c 100 http://127.0.0.1:8080/
```

### 使用 wrk

```bash
# 10 线程，100 连接，持续 30 秒
wrk -t10 -c100 -d30s http://127.0.0.1:8080/
```

### 预期性能

在本地测试中（loopback），你应该能够看到：
- 每秒处理数千个请求
- 低延迟（< 10ms）
- 高并发连接（受系统限制）

## 压力测试

### 测试多个并发流

启动多个客户端请求：

```bash
for i in {1..100}; do
  curl http://127.0.0.1:8080/ &
done
wait
```

### 测试长时间连接

```bash
# 保持连接 1 小时
curl -m 3600 http://127.0.0.1:8080/large-file
```

## 清理

测试完成后：

1. 按 Ctrl+C 停止服务器和客户端
2. 停止本地 HTTP 服务
3. 删除测试证书（如果需要）：

```bash
rm cert.pem key.pem
```

## 自动化测试脚本

可以创建一个测试脚本 `test.sh`：

```bash
#!/bin/bash

echo "Starting TLS Tunnel test..."

# 生成证书
openssl genrsa -out key.pem 2048 2>/dev/null
openssl req -new -x509 -key key.pem -out cert.pem -days 1 -subj "/CN=localhost" 2>/dev/null

# 启动本地服务
python3 -m http.server 3000 > /dev/null 2>&1 &
HTTP_PID=$!

# 启动服务器
./target/release/tls-tunnel -c server.toml server > server.log 2>&1 &
SERVER_PID=$!

sleep 2

# 启动客户端
./target/release/tls-tunnel -c client.toml client > client.log 2>&1 &
CLIENT_PID=$!

sleep 2

# 测试连接
echo "Testing connection..."
RESPONSE=$(curl -s http://127.0.0.1:8080/)

if [ -n "$RESPONSE" ]; then
    echo "✓ Test passed!"
else
    echo "✗ Test failed!"
fi

# 清理
kill $CLIENT_PID $SERVER_PID $HTTP_PID 2>/dev/null
rm key.pem cert.pem

echo "Test completed."
```

运行：

```bash
chmod +x test.sh
./test.sh
```
