# Top 命令使用指南

`top` 命令提供了一个实时终端界面来监控 TLS Tunnel 服务器的统计信息。

## 前提条件

服务器必须启用统计功能（在配置文件中设置 `stats_port`）。

## 基本用法

```bash
# 连接到本地服务器的统计端口
tls-tunnel top --url http://localhost:8080

# 使用自定义刷新间隔（默认 2 秒）
tls-tunnel top --url http://localhost:8080 --interval 5

# 简写形式
tls-tunnel top -u http://localhost:8080 -i 5
```

## 界面说明

### 顶部栏
显示统计信息的 URL 和最后更新时间。

### 主表格
显示所有代理的实时统计信息：

| 列名 | 说明 |
|------|------|
| Name | 代理名称 |
| Publish | 服务器发布地址和端口 |
| Local | 客户端本地服务端口 |
| Active | 当前活跃连接数（绿色表示有活跃连接） |
| Total | 总连接数 |
| Sent | 已发送数据量（自动格式化为 B/KB/MB/GB） |
| Received | 已接收数据量（自动格式化） |
| Uptime | 代理运行时长 |

### 底部控制栏
显示可用的快捷键：
- **q**: 退出程序
- **r**: 手动刷新数据
- **Esc**: 退出程序

## 示例场景

### 1. 监控本地测试服务器

```bash
# 启动带统计功能的服务器
tls-tunnel server --config test-server-with-stats.toml -v

# 在另一个终端查看统计
tls-tunnel top -u http://localhost:8080
```

### 2. 监控远程服务器

```bash
# 假设远程服务器统计端口为 8080
tls-tunnel top -u http://192.168.1.100:8080
```

### 3. 长时间监控（较长刷新间隔）

```bash
# 每 10 秒刷新一次
tls-tunnel top -u http://localhost:8080 -i 10
```

## 服务器配置示例

确保服务器配置文件包含 `stats_port`：

```toml
[server]
bind_addr = "0.0.0.0"
bind_port = 8443
transport = "tls"
auth_key = "your-secret-key"
stats_port = 8080  # 启用统计功能
cert_path = "cert.pem"
key_path = "key.pem"
```

## 故障排除

### 连接失败
- 确认服务器已启动且 `stats_port` 已配置
- 检查防火墙是否允许访问统计端口
- 确认 URL 格式正确（包含 http:// 前缀）

### 界面显示异常
- 确保终端窗口足够大（建议至少 100x30 字符）
- 检查终端是否支持 ANSI 颜色

### 数据不更新
- 按 'r' 键手动刷新
- 检查网络连接
- 确认服务器正在运行

## 性能说明

- `top` 命令对服务器影响极小，仅通过 HTTP 获取 JSON 数据
- 默认刷新间隔（2 秒）适合大多数场景
- 可根据需要调整刷新间隔，减少网络流量

## 快捷键参考

| 按键 | 功能 |
|------|------|
| q | 退出程序 |
| Esc | 退出程序 |
| r | 立即刷新数据 |

## 技术细节

- 使用 `ratatui` 渲染终端界面
- 使用 `reqwest` 异步获取统计数据
- 自动处理终端大小调整
- 优雅处理网络错误和连接中断
