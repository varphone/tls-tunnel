# 统计功能

## 概述

TLS Tunnel 为服务端和客户端都提供了内置的 HTTP 统计服务器，通过统一的界面实时监控代理连接和流量。

## 功能特性

- **实时统计**：查看每个代理的活跃连接数、总连接数、发送/接收字节数
- **统一界面**：服务端和客户端使用相同的监控架构
- **HTML 仪表板**：美观的 Web 界面，每 5 秒自动刷新
- **JSON API**：RESTful API 端点，可编程访问
- **分代理指标**：每个配置的代理都有独立的统计数据
- **运行时长跟踪**：监控每个代理的运行时间
- **内置 Top 命令**：终端 UI 实时监控

## 配置

### 服务端配置

在服务端配置中添加 `stats_port` 选项：

```toml
[server]
bind_addr = "0.0.0.0"
bind_port = 3080
auth_key = "your-secret-key"
# ... other config ...

# 在 9090 端口启用统计服务器
stats_port = 9090
stats_addr = "127.0.0.1"  # 可选：绑定地址（默认：0.0.0.0）
```

如果 `stats_port` 未配置或被注释掉，统计服务器将不会启动。

### 客户端配置

在客户端配置中添加 `stats_port` 选项：

```toml
[client]
server_addr = "your-server.com"
server_port = 3080
auth_key = "your-secret-key"
# ... other config ...

# 在 9091 端口启用客户端统计服务器
stats_port = 9091
stats_addr = "127.0.0.1"  # 可选：绑定地址（默认：0.0.0.0）
```

**配置选项：**

- `stats_port`（可选）：统计 HTTP 服务器的端口号
  - 如果未配置，统计服务器将不会启动
  - 服务端和客户端使用不同的端口（例如：服务端 9090，客户端 9091）
  
- `stats_addr`（可选）：统计服务器的绑定地址
  - 默认值：`0.0.0.0`（监听所有网络接口）
  - 安全值：`127.0.0.1`（仅本地访问）
  - 生产环境建议使用 `127.0.0.1`，通过 SSH 隧道或反向代理访问

## 使用方法

### HTML 仪表板

**服务端统计：**
```
http://server-ip:9090/
```

**客户端统计：**
```
http://client-ip:9091/
```

仪表板显示：
- 代理总数
- 所有代理的活跃连接总数
- 总连接数
- 包含详细指标的分代理统计表格
  
页面每 5 秒自动刷新一次。

### JSON API

**服务端统计：**
```
http://server-ip:9090/stats
```

**客户端统计：**
```
http://client-ip:9091/stats
```

服务端响应示例：

```json
[
  {
    "name": "web",
    "publish_addr": "0.0.0.0",
    "publish_port": 8888,
    "local_port": 80,
    "total_connections": 42,
    "active_connections": 3,
    "bytes_sent": 1048576,
    "bytes_received": 524288,
    "start_time": 1700000000
  }
]
```

客户端响应示例：

```json
[
  {
    "name": "web",
    "bind_addr": "127.0.0.1",
    "bind_port": 8080,
    "target_addr": "example.com",
    "target_port": 80,
    "active_connections": 5,
    "total_connections": 123,
    "bytes_sent": 1048576,
    "bytes_received": 2097152,
    "start_time": 1704067200,
    "status": "Connected"
  }
]
```

### 命令行工具

可以使用 `curl` 获取统计信息：

```bash
# 获取服务端 JSON 统计
curl http://server-ip:9090/stats

# 获取客户端 JSON 统计
curl http://client-ip:9091/stats

# 使用 jq 美化输出
curl -s http://server-ip:9090/stats | jq .

# 获取 HTML 仪表板
curl http://server-ip:9090/
```

### 内置 Top 命令

`tls-tunnel` 可执行文件包含内置的 `top` 命令，适用于服务端和客户端：

```bash
# 查看服务端实时统计
tls-tunnel top --url http://localhost:9090

# 查看客户端实时统计
tls-tunnel top --url http://localhost:9091

# 自定义刷新间隔（默认：2 秒）
tls-tunnel top --url http://localhost:9090 --interval 5

# 简写形式
tls-tunnel top -u http://localhost:9090 -i 5
```

`top` 命令提供：
- **实时仪表板**：由 ratatui 驱动的美观终端 UI
- **自动刷新**：可配置的刷新间隔（默认 2 秒）
- **交互式控制**：
  - `q` 或 `Esc`：退出
  - `r`：手动刷新
- **格式化显示**：人类可读的字节数和时长
- **颜色编码**：活跃连接以绿色高亮显示
- **统一界面**：同一命令适用于服务端和客户端

详细的 `top` 命令使用说明，请参阅 [TOP_USAGE.md](TOP_USAGE.md)。

## 指标说明

### 服务端指标

服务端跟踪每个客户端连接的代理统计信息：

- **代理名称**：代理的唯一标识符
- **发布地址**：代理监听的服务器地址
- **发布端口**：接受外部连接的服务器端口
- **客户端端口**：流量转发到客户端机器的端口
- **活跃连接数**：通过此代理当前打开的连接数
- **总连接数**：自代理启动以来的累计连接数
- **发送字节数**：发送到客户端的总数据量（格式：B、KB、MB、GB、TB）
- **接收字节数**：从客户端接收的总数据量（格式化显示）
- **运行时长**：自代理注册以来的时间（格式：天、小时、分钟、秒）

### 客户端指标

客户端跟踪每个本地代理配置的统计信息：

- **代理名称**：配置文件中的唯一标识符
- **绑定地址**：本地监听地址（通常是 127.0.0.1）
- **绑定端口**：代理监听的本地端口
- **目标地址**：TLS Tunnel 服务器地址
- **目标端口**：服务端发布的端口
- **活跃连接数**：当前活跃的本地连接数
- **总连接数**：自客户端启动以来的累计连接数
- **发送字节数**：通过隧道发送的总数据量
- **接收字节数**：通过隧道接收的总数据量
- **启动时间**：代理跟踪器创建的时间（Unix 时间戳）
- **状态**：连接状态（空闲、已连接、已断开）

### 全局指标

服务端和客户端都提供：

- **代理总数**：当前注册的活跃代理数量
- **活跃连接总数**：所有代理的活跃连接数之和
- **总连接数**：所有代理的连接数之和

## 安全注意事项

⚠️ **重要安全提示：**

1. **无身份验证**：统计端点不需要身份验证
2. **无加密**：HTTP 流量未加密
3. **防火墙保护**：配置防火墙规则，仅允许受信任的 IP 访问
4. **建议本地访问**：生产环境中，绑定到 `127.0.0.1` 并使用 SSH 隧道：

```bash
# 通过 SSH 隧道远程访问服务端统计
ssh -L 9090:localhost:9090 user@server-ip

# 通过 SSH 隧道远程访问客户端统计
ssh -L 9091:localhost:9091 user@client-machine

# 然后通过 http://localhost:9090 或 http://localhost:9091 访问
```

5. **替代方案**：使用反向代理（如 Nginx）添加身份验证和 HTTPS

**生产环境最佳实践：**

- 在服务端和客户端配置中都使用 `stats_addr = "127.0.0.1"`
- 仅通过安全通道访问统计信息（SSH 隧道、VPN 等）
- 实施防火墙规则，阻止外部访问统计端口
- 考虑使用 Nginx 配合 HTTP 基本身份验证和 SSL

## 测试

### 测试服务端统计

1. 启动启用了统计功能的服务端：
   ```bash
   ./tls-tunnel server --config test-server-with-stats.toml
   ```

2. 在另一个终端启动客户端：
   ```bash
   ./tls-tunnel client --config test-client.toml
   ```

3. 打开浏览器：
   ```
   http://localhost:9090
   ```

4. 对代理端口建立一些连接以查看统计更新：
   ```bash
   # 示例：连接到发布端口
   curl http://localhost:8888
   ```

5. 观察统计信息的实时更新（每 5 秒自动刷新）

### 测试客户端统计

1. 确保客户端配置已启用统计：
   ```toml
   [client]
   stats_port = 9091
   stats_addr = "127.0.0.1"
   ```

2. 启动客户端：
   ```bash
   ./tls-tunnel client --config client-with-stats.toml
   ```

3. 打开浏览器查看客户端统计：
   ```
   http://localhost:9091/
   ```

4. 使用 top 命令：
   ```bash
   tls-tunnel top --url http://localhost:9091
   ```

5. 通过隧道生成流量以查看统计更新

## 故障排查

### 统计服务器未启动

**服务端或客户端：**
- 检查是否配置了 `stats_port`
- 验证端口是否已被占用：
  ```bash
  # Linux/Mac
  lsof -i :9090
  
  # Windows
  netstat -ano | findstr :9090
  ```
- 检查日志中的错误信息
- 确保绑定地址有效

### 无法访问统计页面

- 验证服务端/客户端正在运行并监听统计端口
- 检查防火墙规则
- 确保使用了正确的 IP 地址
- 如果使用 `127.0.0.1`，必须从本地主机访问
- 先尝试从本地访问：`http://localhost:9090`

### 统计信息显示为零

**服务端：**
- 确保客户端已连接并通过身份验证
- 确保有实际流量通过代理
- 检查代理是否正确配置
- 验证客户端身份验证成功

**客户端：**
- 确认客户端正在运行并已连接到服务器
- 确保有流量流经本地代理
- 检查代理配置是否与服务器设置匹配
- 验证本地服务可访问

## 示例配置文件

启用了统计功能的服务端配置：

- `examples/standalone-server.toml` - 直连模式，带统计（取消注释 `stats_port`）
- `examples/proxied-server.toml` - 反向代理模式，带统计（取消注释 `stats_port`）

启用了统计功能的客户端配置：

- `examples/client-with-stats.toml` - 启用了统计的客户端
- `examples/standalone-client.toml` - 包含统计配置注释

## 服务端 vs 客户端统计对比

| 特性 | 服务端 | 客户端 |
|------|--------|--------|
| HTTP API | ✅ `/stats` 和 `/` | ✅ `/stats` 和 `/` |
| Top 命令 | ✅ `tls-tunnel top --url ...` | ✅ `tls-tunnel top --url ...` |
| 自动刷新 HTML | ✅ 5 秒 | ✅ 5 秒 |
| 实现方式 | 原生 TCP/HTTP | 原生 TCP/HTTP |
| 配置选项 | `stats_port`、`stats_addr` | `stats_port`、`stats_addr` |
| 监控对象 | 服务端发布的代理 | 客户端本地代理 |
| 统计跟踪 | 入站连接 | 出站隧道连接 |
| 默认端口 | 9090（建议） | 9091（建议） |

**关键区别：**
- **服务端**：监控从外部客户端到发布端口的连接
- **客户端**：监控从本地服务通过隧道的连接

## 与监控系统集成

JSON API 可以与服务端和客户端的监控系统集成：

### Prometheus

创建脚本导出指标：

```bash
#!/bin/bash
# prometheus_exporter.sh - 可用于服务端和客户端

STATS_URL="${1:-http://localhost:9090/stats}"  # 默认为服务端

curl -s "$STATS_URL" | jq -r '.[] | 
  "tls_tunnel_active_connections{proxy=\"\(.name)\"} \(.active_connections)\n" +
  "tls_tunnel_total_connections{proxy=\"\(.name)\"} \(.total_connections)\n" +
  "tls_tunnel_bytes_sent{proxy=\"\(.name)\"} \(.bytes_sent)\n" +
  "tls_tunnel_bytes_received{proxy=\"\(.name)\"} \(.bytes_received)"'

# 用法：
# ./prometheus_exporter.sh http://localhost:9090/stats  # 服务端统计
# ./prometheus_exporter.sh http://localhost:9091/stats  # 客户端统计
```

### 自定义监控

使用您喜欢的语言解析 JSON 端点：

```python
import requests
import json

# 同时监控服务端和客户端
endpoints = {
    'server': 'http://server-ip:9090/stats',
    'client': 'http://client-ip:9091/stats'
}

for name, url in endpoints.items():
    try:
        response = requests.get(url)
        stats = response.json()
        
        print(f"\n=== {name.upper()} Statistics ===")
        for proxy in stats:
            print(f"Proxy {proxy['name']}: {proxy['active_connections']} active connections")
    except Exception as e:
        print(f"Error fetching {name} stats: {e}")
```

### Grafana 仪表板

您可以创建一个统一的 Grafana 仪表板，同时显示服务端和客户端统计：

1. 使用上述导出脚本设置 Prometheus
2. 为服务端和客户端端点创建单独的任务
3. 构建可视化仪表板，显示：
   - 随时间变化的连接数
   - 带宽使用情况
   - 活跃连接 vs 总连接
   - 分代理明细

## 快速开始示例

### 同时监控服务端和客户端

1. **启动带统计的服务端：**
   ```bash
   # server-config.toml
   [server]
   stats_port = 9090
   stats_addr = "0.0.0.0"
   ```

2. **启动带统计的客户端：**
   ```bash
   # client-config.toml  
   [client]
   stats_port = 9091
   stats_addr = "127.0.0.1"
   ```

3. **在不同终端查看：**
   ```bash
   # 终端 1：服务端统计
   tls-tunnel top --url http://localhost:9090
   
   # 终端 2：客户端统计
   tls-tunnel top --url http://localhost:9091
   ```

4. **或使用 tmux/screen 分屏查看：**
   ```bash
   tmux new-session \; \
     send-keys 'tls-tunnel top --url http://localhost:9090' C-m \; \
     split-window -v \; \
     send-keys 'tls-tunnel top --url http://localhost:9091' C-m
   ```

## 未来增强

统计功能的潜在改进：

- [ ] 为统计端点添加身份验证
- [ ] 统计服务器支持 HTTPS
- [ ] 历史数据和图表
- [ ] Prometheus 指标端点
- [ ] WebSocket 实时更新
- [ ] 单连接详细信息
- [ ] 带宽速率限制可视化
- [ ] 告警阈值配置
