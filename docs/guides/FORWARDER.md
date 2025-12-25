# Forward Proxy 指南

Forward Proxy 模式允许客户端通过 TLS Tunnel 服务器转发流量到外部目标，支持 HTTP CONNECT 和 SOCKS5 协议。

## 概述

### 工作原理

**转发路径**：本地应用 → 本地代理监听器 → TLS Tunnel 客户端 → TLS Tunnel 服务器 → 外部目标

**关键特点**：
- 客户端启动本地代理监听器（HTTP 或 SOCKS5）
- 支持任意外部目标（无需预先配置）
- 所有流量通过 TLS Tunnel 加密传输
- 服务器端需要开启 `allow_forward` 选项

### 与其他模式的区别

- **Proxy 模式**：外部 → 服务器 → 客户端 → 本地服务（内网穿透）
- **Visitor 模式**：客户端A → 服务器 → 客户端B → 本地服务（客户端互访）
- **Forwarder 模式**：本地应用 → 客户端 → 服务器 → 外部目标（正向代理）

## 配置说明

### 客户端配置

```toml
# 连接到服务器
server_addr = "example.com:3080"
auth_key = "your-secret-key"

# 配置 HTTP 代理
[[forwarders]]
name = "http-proxy"
proxy_type = "http"
bind_addr = "127.0.0.1"
bind_port = 8080

# 配置 SOCKS5 代理
[[forwarders]]
name = "socks5-proxy"
proxy_type = "socks5"
bind_addr = "127.0.0.1"
bind_port = 1080
```

**配置项说明**：
- `name`: 转发器名称（用于日志标识）
- `proxy_type`: 代理类型，可选值：
  - `"http"`: HTTP CONNECT 代理（适用于 HTTPS）
  - `"socks5"`: SOCKS5 代理（支持任意 TCP 连接）
- `bind_addr`: 本地监听地址（通常是 `127.0.0.1`）
- `bind_port`: 本地监听端口

### 服务器配置

```toml
# 服务器基本配置
bind_addr = "0.0.0.0:3080"
auth_key = "your-secret-key"

# 启用转发功能（必须）
allow_forward = true

# TLS 配置
tls_cert = "cert.pem"
tls_key = "key.pem"
```

**重要**：服务器端必须设置 `allow_forward = true` 才能处理转发请求，否则会拒绝所有转发连接。

## 使用示例

### 1. 使用 HTTP 代理

HTTP 代理适用于 HTTPS 流量，使用 CONNECT 方法建立隧道：

```bash
# 使用 curl
curl -x http://127.0.0.1:8080 https://www.google.com

# 使用 wget
wget -e use_proxy=yes -e http_proxy=127.0.0.1:8080 https://example.com

# 设置系统环境变量（Linux/macOS）
export http_proxy=http://127.0.0.1:8080
export https_proxy=http://127.0.0.1:8080

# 设置系统环境变量（Windows PowerShell）
$env:http_proxy="http://127.0.0.1:8080"
$env:https_proxy="http://127.0.0.1:8080"
```

**浏览器配置**：
- Firefox: 设置 → 网络设置 → 手动代理配置
  - HTTP 代理: `127.0.0.1` 端口 `8080`
  - 勾选"也将此代理用于 HTTPS"
- Chrome/Edge: 使用系统代理或安装代理扩展

### 2. 使用 SOCKS5 代理

SOCKS5 代理更加通用，支持任意 TCP 连接：

```bash
# 使用 curl
curl -x socks5://127.0.0.1:1080 https://www.google.com

# SSH 通过 SOCKS5 代理
ssh -o ProxyCommand="nc -X 5 -x 127.0.0.1:1080 %h %p" user@target.com

# Git 通过 SOCKS5 代理
git config --global http.proxy socks5://127.0.0.1:1080
git config --global https.proxy socks5://127.0.0.1:1080
```

**浏览器配置**：
- Firefox: 设置 → 网络设置 → 手动代理配置
  - SOCKS 主机: `127.0.0.1` 端口 `1080`
  - 选择 `SOCKS v5`

### 3. 应用程序配置

许多应用程序支持代理设置：

```bash
# Node.js npm
npm config set proxy http://127.0.0.1:8080
npm config set https-proxy http://127.0.0.1:8080

# Python pip
pip install --proxy http://127.0.0.1:8080 package-name

# Docker
# 编辑 ~/.docker/config.json
{
  "proxies": {
    "default": {
      "httpProxy": "http://127.0.0.1:8080",
      "httpsProxy": "http://127.0.0.1:8080"
    }
  }
}
```

## 完整部署示例

### 场景：通过 VPS 访问被封锁的网站

#### 第一步：在 VPS 上部署服务器

```toml
# server.toml
bind_addr = "0.0.0.0:3080"
auth_key = "my-secret-key-12345"
allow_forward = true
tls_cert = "/etc/tls-tunnel/cert.pem"
tls_key = "/etc/tls-tunnel/key.pem"
```

```bash
# 启动服务器
./tls-tunnel server -c server.toml
```

#### 第二步：在本地部署客户端

```toml
# client.toml
server_addr = "your-vps-ip:3080"
auth_key = "my-secret-key-12345"

[[forwarders]]
name = "http-proxy"
proxy_type = "http"
bind_addr = "127.0.0.1"
bind_port = 8080

[[forwarders]]
name = "socks5-proxy"
proxy_type = "socks5"
bind_addr = "127.0.0.1"
bind_port = 1080
```

```bash
# 启动客户端
./tls-tunnel client -c client.toml
```

#### 第三步：配置应用程序

现在可以使用本地代理访问任何网站：

```bash
# 测试连接
curl -x http://127.0.0.1:8080 https://www.google.com -I

# 或使用 SOCKS5
curl -x socks5://127.0.0.1:1080 https://www.google.com -I
```

## 高级配置

### 同时配置多个代理

可以在不同端口上启动多个代理监听器：

```toml
# HTTP 代理用于一般浏览
[[forwarders]]
name = "http-general"
proxy_type = "http"
bind_addr = "127.0.0.1"
bind_port = 8080

# SOCKS5 代理用于特殊应用
[[forwarders]]
name = "socks5-special"
proxy_type = "socks5"
bind_addr = "127.0.0.1"
bind_port = 1080

# 另一个 SOCKS5 代理监听在不同接口
[[forwarders]]
name = "socks5-lan"
proxy_type = "socks5"
bind_addr = "192.168.1.100"
bind_port = 1081
```

### 仅使用 Forwarder（不配置 Proxy）

客户端可以只配置 `forwarders` 而不配置 `proxies`：

```toml
server_addr = "example.com:3080"
auth_key = "your-secret-key"

# 只有 forwarders，没有 proxies
[[forwarders]]
name = "my-proxy"
proxy_type = "socks5"
bind_addr = "127.0.0.1"
bind_port = 1080
```

这种配置适用于只需要正向代理功能的场景。

### 基于 GeoIP 的智能路由

使用 GeoIP 数据库和自定义规则实现智能路由，根据目标地址决定是直连还是走代理：

```toml
[[forwarders]]
name = "socks5-proxy-smart"
proxy_type = "socks5"
bind_addr = "127.0.0.1"
bind_port = 2080

# GeoIP 路由配置
[forwarders.routing]
# GeoIP 数据库路径（MaxMind GeoLite2 Country）
geoip_db = "GeoLite2-Country.mmdb"

# 直连国家列表（ISO 3166-1 alpha-2 代码）
direct_countries = ["CN", "HK", "TW", "MO"]

# 代理国家列表（为空表示其他所有国家）
proxy_countries = []

# 直连 IP/CIDR 列表（支持 CIDR 格式）
direct_ips = [
    "192.168.0.0/16",   # 内网
    "10.0.0.0/8",       # 内网
    "223.5.5.5",        # 阿里 DNS
]

# 代理 IP/CIDR 列表
proxy_ips = []

# 直连域名列表（支持通配符）
direct_domains = [
    "*.baidu.com",      # 百度所有子域名
    "*.qq.com",         # 腾讯所有子域名
    "example.com",      # 精确匹配
]

# 代理域名列表（支持通配符）
proxy_domains = []

# 默认策略："direct" 或 "proxy"
default_strategy = "proxy"
```

**路由规则优先级**（从高到低）：
1. **域名匹配**：检查 `direct_domains` 和 `proxy_domains`
2. **IP/CIDR 匹配**：检查 `direct_ips` 和 `proxy_ips`
3. **GeoIP 国家**：查询 `direct_countries` 和 `proxy_countries`
4. **默认策略**：使用 `default_strategy`

**域名通配符规则**：
- `*.example.com` - 匹配 www.example.com、api.example.com 和 example.com
- `.example.com` - 匹配 www.example.com 但不匹配 example.com
- `example.com` - 仅精确匹配 example.com

**IP/CIDR 格式**：
- 单个 IP：`8.8.8.8`、`2001:4860:4860::8888`
- CIDR 网段：`192.168.0.0/16`、`10.0.0.0/8`
- IPv6 CIDR：`2001:db8::/32`

**获取 GeoIP 数据库**：
```bash
# 方式 1: MaxMind GeoLite2-Country（官方，需注册免费账号）
# 访问：https://dev.maxmind.com/geoip/geolite2-free-geolocation-data
# 下载 GeoLite2-Country.mmdb

# 方式 2: v2fly/geoip（社区版，需转换）
# 下载 geoip.dat：https://github.com/v2fly/geoip/releases
# 使用转换工具：https://github.com/varphone/geoip/tree/dev
# 转换命令：./geoip --input v2rayGeoIPDat --inputFile geoip.dat --output maxmindMMDB --outputFile geoip.mmdb
```

**使用场景**：
- ✅ 国内 IP 直连（速度快，延迟低）
- ✅ 国外 IP 走代理（绕过限制）
- ✅ 节省代理服务器带宽
- ✅ 提高访问速度

**注意事项**：
- 如果未配置 `geoip_db` 或数据库加载失败，将使用 `default_strategy`
- 如果目标是域名，会先解析为 IP 再查询地理位置
- 建议定期更新 GeoIP 数据库以保持准确性

## 安全注意事项

### 1. 绑定地址

**推荐**：绑定到 `127.0.0.1`（仅本地访问）
```toml
bind_addr = "127.0.0.1"
```

**警告**：绑定到 `0.0.0.0` 会允许网络中的其他设备访问你的代理
```toml
bind_addr = "0.0.0.0"  # 危险！任何人都可以使用你的代理
```

### 2. 服务器端访问控制

服务器端的 `allow_forward` 选项是重要的安全开关：

```toml
# 生产环境：谨慎启用
allow_forward = true

# 如果不需要 forward proxy 功能，应该禁用
allow_forward = false
```

**内置安全防护**：服务器端会自动阻止客户端访问以下地址：

- ✅ **Loopback 地址**：`127.0.0.1`, `::1`, `localhost` 等（防止访问服务器本机）
- ✅ **私有 IPv4 地址**：
  - `10.0.0.0/8` (10.x.x.x)
  - `172.16.0.0/12` (172.16.x.x - 172.31.x.x)
  - `192.168.0.0/16` (192.168.x.x)
  - `169.254.0.0/16` (169.254.x.x - Link-local)
- ✅ **私有 IPv6 地址**：
  - `fc00::/7` (Unique Local Address)
  - `fe80::/10` (Link-local)

这些防护措施确保客户端无法通过 forward proxy 访问服务器的内网资源，提高安全性。

### 3. 身份验证

确保使用强密钥：

```toml
# 不安全
auth_key = "123456"

# 安全
auth_key = "rAnd0m-5ecur3-k3y-w1th-m1n1mum-32-ch4rs"
```

可以使用随机生成器：
```bash
# Linux/macOS
openssl rand -base64 32

# PowerShell
-join ((48..57) + (65..90) + (97..122) | Get-Random -Count 32 | ForEach-Object {[char]$_})
```

### 4. TLS 配置

使用有效的 TLS 证书以防止中间人攻击：

```toml
# 服务器端
tls_cert = "/path/to/cert.pem"
tls_key = "/path/to/key.pem"

# 客户端（如果需要验证服务器证书）
tls_ca = "/path/to/ca.pem"
```

## 故障排查

### 连接被拒绝

**症状**：代理无法连接到目标
**原因**：服务器端未启用 `allow_forward`
**解决**：
```toml
# 在服务器配置中添加
allow_forward = true
```

### 代理监听失败

**症状**：客户端启动失败，提示端口占用
**原因**：指定端口已被其他程序使用
**解决**：
```bash
# 检查端口占用（Windows）
netstat -ano | findstr :8080

# 检查端口占用（Linux/macOS）
lsof -i :8080

# 更改为其他端口
bind_port = 8081
```

### 性能问题

**症状**：代理速度慢
**排查**：
1. 检查客户端到服务器的网络延迟
2. 检查服务器的出口带宽
3. 查看日志中是否有错误或警告
4. 考虑使用 `http2` 传输模式以提升性能

```toml
# 客户端配置
transport = "http2"  # 或 "wss"
```

## 日志分析

启用详细日志以排查问题：

```bash
# 设置日志级别（Linux/macOS）
export RUST_LOG=debug
./tls-tunnel client -c client.toml

# 设置日志级别（Windows PowerShell）
$env:RUST_LOG="debug"
.\tls-tunnel.exe client -c client.toml
```

**关键日志信息**：
- `Forwarder 'xxx': Started listening` - 监听器启动成功
- `Forwarder 'xxx': Accepted connection` - 接受新连接
- `Forwarder 'xxx': Forwarding to target` - 开始转发
- `Server accepted connection` - 服务器接受转发请求
- `connection closed` - 连接正常关闭

## 性能优化

### 1. 使用 HTTP/2 传输

HTTP/2 多路复用可以提升并发性能：

```toml
# 客户端配置
transport = "http2"
server_addr = "https://example.com:3080"
```

### 2. 连接池配置

调整连接池大小以适应高并发场景：

```toml
# 客户端配置
max_idle_conns = 10  # 最大空闲连接数
```

### 3. 缓冲区大小

根据网络条件调整缓冲区（需修改代码中的 `BUFFER_SIZE` 常量）。

## 安全测试

验证服务器端的安全防护是否生效：

```bash
# 测试 1: 尝试访问 localhost（应该被拒绝）
curl -x socks5://127.0.0.1:1080 http://127.0.0.1:80 --connect-timeout 3
# 预期结果: Connection was reset 或 Empty reply from server

# 测试 2: 尝试访问内网地址（应该被拒绝）
curl -x http://127.0.0.1:8080 http://192.168.1.1 --connect-timeout 3
# 预期结果: Empty reply from server

# 测试 3: 尝试访问私有网络（应该被拒绝）
curl -x socks5://127.0.0.1:1080 http://10.0.0.1 --connect-timeout 3
# 预期结果: Connection was reset

# 测试 4: 访问外部网站（应该成功）
curl -x http://127.0.0.1:8080 https://www.example.com -I
# 预期结果: 正常返回 HTTP 响应头
```

**服务器日志输出示例**：
```
WARN  Blocked attempt to access loopback address: 127.0.0.1
WARN  Blocked attempt to access private IPv4 address: 192.168.1.1
ERROR Access denied: cannot forward to local or private address '10.0.0.1:80'
```

## 总结

Forward Proxy 模式提供了灵活的正向代理功能，主要用途包括：

- ✅ 通过远程服务器访问外部网站
- ✅ 绕过网络限制
- ✅ 加密本地应用的网络流量
- ✅ 统一管理出口流量

**关键配置**：
- 客户端：配置 `[[forwarders]]` 列表
- 服务器：启用 `allow_forward = true`
- 应用程序：配置代理地址（HTTP 或 SOCKS5）

**安全建议**：
- 绑定到 `127.0.0.1` 避免未授权访问
- 使用强身份验证密钥
- 仅在需要时启用 `allow_forward`
- 使用有效的 TLS 证书

如有问题，请查看项目文档或提交 Issue。
