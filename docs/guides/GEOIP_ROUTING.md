# GeoIP 路由功能使用说明

## 概述

GeoIP 路由功能允许 forwarder 根据目标 IP 的地理位置智能选择连接方式：
- **国内 IP**：直接连接（速度快，延迟低）
- **国外 IP**：通过代理服务器（绕过限制）

## 获取 GeoIP 数据库

### 💰 费用说明

**完全免费** ✅
- MaxMind GeoLite2 官方免费版
- 无需付费
- 满足路由分流需求

### 支持的格式

⚠️ **原生支持 MaxMind `.mmdb` 格式**
- ✅ 支持：`GeoLite2-Country.mmdb`（MaxMind 官方）
- ❌ 不支持：`geoip.dat`（v2ray 私有格式）
- ✅ 可转换：使用转换工具将 `.dat` 转为 `.mmdb`

### 方法一：使用 v2fly/geoip 数据（需转换）

如果你想使用 v2fly/geoip 的数据（包含更准确的中国 IP 分类），可以通过转换工具将其转为 `.mmdb` 格式：

**步骤 1：下载 v2fly/geoip 数据**
```bash
# 下载 geoip.dat
wget https://github.com/v2fly/geoip/releases/latest/download/geoip.dat
```

**步骤 2：转换为 mmdb 格式**
```bash
# 克隆转换工具
git clone -b dev https://github.com/varphone/geoip
cd geoip

# 编译转换工具（需要 Go 环境）
go build

# 转换 geoip.dat 到 geoip.mmdb
./geoip --input v2rayGeoIPDat --inputFile geoip.dat --output maxmindMMDB --outputFile geoip.mmdb

# 将转换后的文件移动到项目目录
mv geoip.mmdb /path/to/tls-tunnel/
```

**配置示例**：
```toml
[forwarders.routing]
geoip_db = "geoip.mmdb"  # 使用转换后的 v2fly 数据
direct_countries = ["CN", "HK", "TW", "MO"]
default_strategy = "proxy"
```

**优势**：
- ✅ 包含更准确的中国 IP 数据
- ✅ 包含国内 CDN IP 分类
- ✅ 社区维护更新及时

### 方法二：下载 MaxMind GeoLite2（官方）

**优势**：
- ✅ 官方维护，覆盖全球
- ✅ 每月定期更新
- ✅ 国家级别准确度 ~99.8%
- ✅ 标准 `.mmdb` 格式

**下载步骤**：
1. 访问 MaxMind 官网：https://www.maxmind.com/en/geolite2/signup
2. **注册免费账号**（无需信用卡）
3. 登录后进入：https://www.maxmind.com/en/accounts/current/geoip/downloads
4. 下载 **GeoLite2 Country** 数据库（mmdb 格式）
5. 解压后将 `GeoLite2-Country.mmdb` 文件放到项目目录

**配置示例**：
```toml
[forwarders.routing]
geoip_db = "GeoLite2-Country.mmdb"
direct_countries = ["CN", "HK", "TW", "MO"]
default_strategy = "proxy"
```

### 快速下载（需要账号）

```bash
# 下载并解压
wget "https://download.maxmind.com/app/geoip_download?edition_id=GeoLite2-Country&license_key=YOUR_LICENSE_KEY&suffix=tar.gz" -O GeoLite2-Country.tar.gz
tar -xzf GeoLite2-Country.tar.gz
mv GeoLite2-Country_*/GeoLite2-Country.mmdb ./
```

### 使用 geoipupdate 工具（自动更新）

```bash
# 安装 geoipupdate
# Ubuntu/Debian
sudo apt-get install geoipupdate

# macOS
brew install geoipupdate

# Windows
# 从 https://github.com/maxmind/geoipupdate/releases 下载

# 配置账号信息
# 编辑 /etc/GeoIP.conf 或 %PROGRAMDATA%\MaxMind\GeoIPUpdate\GeoIP.conf
AccountID YOUR_ACCOUNT_ID
LicenseKey YOUR_LICENSE_KEY
EditionIDs GeoLite2-Country

# 更新数据库
geoipupdate
```

## 配置示例

### 基础配置（仅 GeoIP）

```toml
[[forwarders]]
name = "socks5-smart"
proxy_type = "socks5"
bind_addr = "127.0.0.1"
bind_port = 2080

[forwarders.routing]
geoip_db = "GeoLite2-Country.mmdb"
direct_countries = ["CN"]
default_strategy = "proxy"
```

### 组合配置（GeoIP + IP + 域名）

```toml
[[forwarders]]
name = "socks5-advanced"
proxy_type = "socks5"
bind_addr = "127.0.0.1"
bind_port = 2080

[forwarders.routing]
# GeoIP 数据库
geoip_db = "GeoLite2-Country.mmdb"

# 国家级路由
direct_countries = ["CN", "HK", "TW", "MO"]
proxy_countries = []

# IP/CIDR 级路由（优先级高于 GeoIP）
direct_ips = [
    "192.168.0.0/16",     # 内网
    "10.0.0.0/8",         # 内网
    "172.16.0.0/12",      # 内网
    "223.5.5.5",          # 阿里 DNS
    "119.29.29.29",       # 腾讯 DNS
]
proxy_ips = [
    "8.8.8.8",            # Google DNS 强制走代理
]

# 域名级路由（优先级最高）
direct_domains = [
    "*.baidu.com",        # 百度所有子域名
    "*.qq.com",           # 腾讯所有子域名
    "*.taobao.com",       # 淘宝所有子域名
    "*.alipay.com",       # 支付宝所有子域名
    "example.com",        # 精确匹配
]
proxy_domains = [
    "*.google.com",       # Google 所有子域名走代理
    "*.youtube.com",      # YouTube 所有子域名走代理
]

# 默认策略
default_strategy = "proxy"
```

### 纯域名/IP 配置（不使用 GeoIP）

```toml
[[forwarders]]
name = "socks5-rules-only"
proxy_type = "socks5"
bind_addr = "127.0.0.1"
bind_port = 2080

[forwarders.routing]
# 不配置 GeoIP 数据库
# geoip_db = "GeoLite2-Country.mmdb"

# 仅使用域名和 IP 规则
direct_domains = [
    "*.cn",               # 所有 .cn 域名
    "*.baidu.com",
    "*.qq.com",
]

direct_ips = [
    "192.168.0.0/16",
    "10.0.0.0/8",
]

# 其他所有流量走代理
default_strategy = "proxy"
```

### 多地区配置

```toml
[[forwarders]]
name = "socks5-asia-direct"
proxy_type = "socks5"
bind_addr = "127.0.0.1"
bind_port = 2080

[forwarders.routing]
geoip_db = "GeoLite2-Country.mmdb"
# 大中华区 + 亚洲部分国家直连
direct_countries = ["CN", "HK", "TW", "MO", "JP", "KR", "SG"]
default_strategy = "proxy"
```

### 反向配置（仅特定国家走代理）

```toml
[[forwarders]]
name = "socks5-us-only-proxy"
proxy_type = "socks5"
bind_addr = "127.0.0.1"
bind_port = 2080

[forwarders.routing]
geoip_db = "GeoLite2-Country.mmdb"
# 只有美国走代理
proxy_countries = ["US"]
# 其他所有国家直连
default_strategy = "direct"
```

## 路由规则详解

### 优先级顺序

路由规则按以下顺序匹配（从高到低）：

1. **域名匹配** - `direct_domains` 和 `proxy_domains`
2. **IP/CIDR 匹配** - `direct_ips` 和 `proxy_ips`
3. **GeoIP 国家匹配** - `direct_countries` 和 `proxy_countries`
4. **默认策略** - `default_strategy`

### 域名匹配规则

**通配符 `*`**：
```toml
direct_domains = ["*.example.com"]
```
- ✅ 匹配：`www.example.com`
- ✅ 匹配：`api.example.com`
- ✅ 匹配：`example.com`（也匹配根域名）
- ❌ 不匹配：`example.org`

**点前缀 `.`**：
```toml
direct_domains = [".example.com"]
```
- ✅ 匹配：`www.example.com`
- ✅ 匹配：`api.example.com`
- ❌ 不匹配：`example.com`（不匹配根域名）

**精确匹配**：
```toml
direct_domains = ["example.com"]
```
- ✅ 匹配：`example.com`
- ❌ 不匹配：`www.example.com`

### IP/CIDR 匹配规则

**单个 IP**：
```toml
direct_ips = ["8.8.8.8", "2001:4860:4860::8888"]
```

**CIDR 网段**：
```toml
direct_ips = [
    "192.168.0.0/16",      # 192.168.0.0 - 192.168.255.255
    "10.0.0.0/8",          # 10.0.0.0 - 10.255.255.255
    "172.16.0.0/12",       # 172.16.0.0 - 172.31.255.255
    "2001:db8::/32",       # IPv6 网段
]
```

### GeoIP 国家匹配

使用 ISO 3166-1 alpha-2 国家代码（两字母代码）：

```toml
direct_countries = ["CN", "HK", "TW", "MO"]  # 大中华区
proxy_countries = ["US", "GB"]                # 美国、英国
```

## 国家代码参考

常用的 ISO 3166-1 alpha-2 国家代码：

| 代码 | 国家/地区 |
|------|-----------|
| CN   | 中国大陆 |
| HK   | 香港 |
| TW   | 台湾 |
| MO   | 澳门 |
| JP   | 日本 |
| KR   | 韩国 |
| SG   | 新加坡 |
| US   | 美国 |
| GB   | 英国 |
| DE   | 德国 |
| FR   | 法国 |
| AU   | 澳大利亚 |
| CA   | 加拿大 |

完整列表：https://en.wikipedia.org/wiki/ISO_3166-1_alpha-2

## 测试路由策略

### 命令行测试

启动客户端后使用 curl 测试：

```bash
# Linux/macOS
curl -x socks5h://127.0.0.1:2080 https://www.baidu.com
curl -x socks5h://127.0.0.1:2080 https://www.google.com
curl -x socks5://127.0.0.1:2080 http://192.168.1.1

# Windows PowerShell
$proxy = [System.Net.WebProxy]::new('socks5://127.0.0.1:2080')
$wc = [System.Net.WebClient]::new()
$wc.Proxy = $proxy
$wc.DownloadString('https://www.baidu.com')
```

### 日志分析

启动客户端后，日志会显示路由决策：

```
INFO  Forwarder 'socks5-smart': GeoIP routing enabled
INFO  Forwarder 'socks5-smart': Forwarding to target: www.baidu.com:443
DEBUG Domain www.baidu.com matches direct_domains pattern *.baidu.com -> direct
INFO  Forwarder 'socks5-smart': Using direct connection

INFO  Forwarder 'socks5-smart': Forwarding to target: 8.8.8.8:53
DEBUG IP 8.8.8.8 matches proxy_ips -> proxy
INFO  Forwarder 'socks5-smart': Using proxy connection

INFO  Forwarder 'socks5-smart': Forwarding to target: unknown.cn:80
DEBUG IP 111.206.xxx.xxx is from country: CN
DEBUG Country CN is in direct_countries list -> direct
INFO  Forwarder 'socks5-smart': Using direct connection
```

### 调试模式

启用详细日志查看完整路由决策过程：

```bash
# Linux/macOS
export RUST_LOG=tls_tunnel=debug
./tls-tunnel client -c config.toml

# Windows
$env:RUST_LOG="tls_tunnel=debug"
.\tls-tunnel.exe client -c config.toml
```

## 故障排查

### 问题：数据库加载失败

```
WARN  Failed to load GeoIP database from GeoLite2-Country.mmdb: ...
WARN  Routing will use default strategy for all addresses
```

**解决方法**：
1. 检查文件路径是否正确（相对于配置文件目录或工作目录）
2. 检查文件是否存在：`ls -l GeoLite2-Country.mmdb`
3. 检查文件权限：`chmod 644 GeoLite2-Country.mmdb`
4. 确认文件格式是 `.mmdb`（不是 v2fly .dat 格式）

### 问题：所有流量都走代理/直连

**可能原因**：
- 没有配置 `geoip_db` → 使用 `default_strategy`
- 数据库加载失败 → 使用 `default_strategy`
- 目标不匹配任何规则 → 使用 `default_strategy`

**检查方法**：
设置日志级别为 `debug` 查看匹配过程

### 问题：域名通配符不生效

**检查配置**：
```toml
direct_domains = ["*.baidu.com"]  # 正确：匹配所有子域名和根域名
direct_domains = ["*baidu.com"]   # 错误：缺少点号
direct_domains = [".baidu.com"]   # 正确：仅匹配子域名，不匹配根域名
```

### 问题：CIDR 格式错误

**正确示例**：
```toml
direct_ips = [
    "192.168.0.0/16",    # 正确：CIDR 格式
    "10.0.0.1",          # 正确：单个 IP
]
```

**错误示例**：
```toml
direct_ips = [
    "192.168.0.0-255",   # 错误：不是 CIDR 格式
    "192.168.0.*",       # 错误：通配符不支持
]
```

## 性能影响

- **GeoIP 查询**：内存查询，延迟 < 1ms
- **DNS 解析**：如果目标是域名，需要先解析 IP（系统 DNS 缓存有效）
- **建议**：定期更新 GeoIP 数据库（每月一次）

## 隐私和安全注意事项

### DNS 解析隐私

**重要**：在路由决策过程中，如果使用域名白名单（`direct_domains`/`proxy_domains`），系统会对未匹配的域名进行 DNS 解析以获取 IP 地址，然后再进行 GeoIP 查询。

**隐私影响**：
- DNS 查询会暴露你访问的域名给 DNS 服务器
- 在某些监控环境下，DNS 查询本身可能泄漏用户意图

**最佳实践**：
```toml
[forwarders.routing]
# ✅ 推荐：优先使用 IP/CIDR 规则（无需 DNS 解析）
direct_ips = [
    "192.168.0.0/16",
    "10.0.0.0/8",
]

# ⚠️  慎用：域名规则会触发 DNS 解析
direct_domains = ["*.example.com"]

# ✅ 最安全：使用 GeoIP 国家规则（仅在连接时解析）
direct_countries = ["CN"]
```

**建议**：
- 对于已知的固定 IP 服务，优先使用 `direct_ips`
- 如果必须使用域名规则，考虑使用加密 DNS（DoH/DoT）
- 敏感场景下，避免使用域名白名单

### 配置文件安全

配置文件包含认证密钥等敏感信息，应妥善保护：

**Linux/macOS**：
```bash
# 仅所有者可读写
chmod 600 config.toml

# 检查权限
ls -l config.toml
# 应显示：-rw------- (600)
```

**Windows**：
```powershell
# 移除其他用户的访问权限
icacls config.toml /inheritance:r
icacls config.toml /grant:r "$env:USERNAME:(R,W)"
```

**建议**：
- 不要将配置文件提交到 Git 仓库
- 使用环境变量或密钥管理工具存储 `auth_key`
- 定期轮换认证密钥

## 更新数据库

GeoIP 数据库应定期更新以保持准确性：

```bash
# 使用 geoipupdate 自动更新
geoipupdate

# 或手动从 MaxMind 网站下载最新版本
```

建议设置定时任务每月更新一次。
