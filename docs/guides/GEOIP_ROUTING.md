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

### 基础配置

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

启动客户端后，日志会显示路由决策：

```
INFO  Forwarder 'socks5-smart': GeoIP routing enabled (direct_countries: ["CN"], default: Proxy)
INFO  Forwarder 'socks5-smart': Forwarding to target: www.baidu.com:80
DEBUG IP 111.206.xxx.xxx is from country: CN
DEBUG Country CN is in direct_countries list, using direct connection
INFO  Forwarder 'socks5-smart': Using direct connection for www.baidu.com:80
```

## 故障排查

### 数据库加载失败

```
WARN  Failed to load GeoIP database from GeoLite2-Country.mmdb: ...
WARN  Routing will use default strategy for all addresses
```

**解决方法**：
1. 检查文件路径是否正确
2. 检查文件是否存在
3. 检查文件权限
4. 确认文件格式是 `.mmdb`

### 所有流量都走代理/直连

**可能原因**：
- 没有配置 `geoip_db` → 使用 `default_strategy`
- 数据库加载失败 → 使用 `default_strategy`
- 目标国家不在 `direct_countries` 或 `proxy_countries` 中 → 使用 `default_strategy`

**检查方法**：
设置日志级别为 `debug`：
```bash
# Linux/macOS
export RUST_LOG=debug
./tls-tunnel client -c config.toml

# Windows
$env:RUST_LOG="debug"
.\tls-tunnel.exe client -c config.toml
```

## 性能影响

- **GeoIP 查询**：内存查询，延迟 < 1ms
- **DNS 解析**：如果目标是域名，需要先解析 IP（系统 DNS 缓存有效）
- **建议**：定期更新 GeoIP 数据库（每月一次）

## 更新数据库

GeoIP 数据库应定期更新以保持准确性：

```bash
# 使用 geoipupdate 自动更新
geoipupdate

# 或手动从 MaxMind 网站下载最新版本
```

建议设置定时任务每月更新一次。
