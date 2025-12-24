# 反向代理支持

tls-tunnel 支持在 Nginx、Apache 等反向代理服务器后端运行。这允许您将 tls-tunnel 与 Web 服务器共存，并利用代理服务器的高级功能（如负载均衡、SSL 终止等）。

## 工作原理

当 `behind_proxy` 设置为 `true` 时：

1. **TLS 终止在前端代理**：前端代理（如 Nginx）处理 TLS 连接
2. **后端使用明文连接**：tls-tunnel 接收来自代理的明文 HTTP/2 或 WebSocket 连接
3. **无需 TLS 证书**：后端服务器不需要配置 TLS 证书和密钥

```
[客户端] --TLS--> [Nginx] --明文--> [tls-tunnel 服务器] <--明文--> [目标服务]
```

## 部署模式

tls-tunnel 支持两种反向代理部署模式：

### 1. 独立域名模式（推荐）

```
https://tunnel.example.com/ -> 后端 tls-tunnel
```

**客户端配置**：
```toml
[client]
server_addr = "tunnel.example.com"
server_port = 443
server_path = "/"  # 默认值
```

### 2. 子目录模式

```
https://example.com/tunnel/ -> 后端 tls-tunnel
https://example.com/         -> 其他服务（网站等）
```

**客户端配置**：
```toml
[client]
server_addr = "example.com"
server_port = 443
server_path = "/tunnel/"  # 必须匹配 Nginx location
```

**优势**：可以与其他服务共享同一域名和端口。

详细配置见 [Nginx 子目录配置](#nginx-子目录配置)。

## 配置要求

### 1. 传输类型限制

只有 HTTP/2 和 WebSocket 传输支持反向代理模式：

```toml
[server]
bind_addr = "127.0.0.1"
bind_port = 8080
transport = "http2"  # 或 "wss"
behind_proxy = true   # 启用反向代理模式
# cert_file 和 key_file 在 behind_proxy=true 时不需要
```

### 2. 配置验证

配置文件会进行以下验证：

- ✅ `http2` + `behind_proxy=true` - 允许
- ✅ `wss` + `behind_proxy=true` - 允许
- ❌ `tls` + `behind_proxy=true` - **不允许**（TLS 传输不支持反向代理）
- ✅ `behind_proxy=true` 时不需要 `cert_file` 和 `key_file`

## Nginx 配置示例

### HTTP/2 反向代理

```nginx
# 上游服务器配置
upstream tls_tunnel_http2 {
    server 127.0.0.1:8080;
}

server {
    listen 443 ssl http2;
    server_name tunnel.example.com;

    # SSL 证书配置
    ssl_certificate /path/to/cert.pem;
    ssl_certificate_key /path/to/key.pem;
    
    # SSL 优化
    ssl_protocols TLSv1.2 TLSv1.3;
    ssl_ciphers HIGH:!aNULL:!MD5;
    ssl_prefer_server_ciphers on;

    location / {
        # 反向代理到 tls-tunnel
        proxy_pass http://tls_tunnel_http2;
        
        # HTTP/2 CONNECT 支持
        proxy_http_version 1.1;
        proxy_set_header Connection "";
        
        # 保持客户端信息
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        
        # 超时设置（长连接）
        proxy_connect_timeout 3600s;
        proxy_send_timeout 3600s;
        proxy_read_timeout 3600s;
        send_timeout 3600s;
        
        # 缓冲设置
        proxy_buffering off;
        proxy_request_buffering off;
    }
}
```

### WebSocket 反向代理

```nginx
# 上游服务器配置
upstream tls_tunnel_wss {
    server 127.0.0.1:8080;
}

server {
    listen 443 ssl;
    server_name tunnel.example.com;

    # SSL 证书配置
    ssl_certificate /path/to/cert.pem;
    ssl_certificate_key /path/to/key.pem;
    
    # SSL 优化
    ssl_protocols TLSv1.2 TLSv1.3;
    ssl_ciphers HIGH:!aNULL:!MD5;
    ssl_prefer_server_ciphers on;

    location / {
        # 反向代理到 tls-tunnel
        proxy_pass http://tls_tunnel_wss;
        
        # WebSocket 支持
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        
        # 保持客户端信息
        proxy_set_header Host $host;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto $scheme;
        
        # 超时设置（长连接）
        proxy_connect_timeout 3600s;
        proxy_send_timeout 3600s;
        proxy_read_timeout 3600s;
        send_timeout 3600s;
        
        # 缓冲设置
        proxy_buffering off;
    }
}
```

### 负载均衡配置

如果运行多个 tls-tunnel 实例：

```nginx
upstream tls_tunnel_cluster {
    # 负载均衡算法
    least_conn;  # 最少连接数
    
    # 后端服务器
    server 127.0.0.1:8080 max_fails=3 fail_timeout=30s;
    server 127.0.0.1:8081 max_fails=3 fail_timeout=30s;
    server 127.0.0.1:8082 max_fails=3 fail_timeout=30s;
    
    # 保持连接
    keepalive 32;
}

server {
    listen 443 ssl http2;
    server_name tunnel.example.com;
    
    ssl_certificate /path/to/cert.pem;
    ssl_certificate_key /path/to/key.pem;

    location / {
        proxy_pass http://tls_tunnel_cluster;
        # ... 其他配置同上
    }
}
```

### Nginx 子目录配置

如果希望在同一域名下通过子路径访问 tls-tunnel：

```nginx
# 上游服务器配置
upstream tls_tunnel_http2 {
    server 127.0.0.1:8080;
    keepalive 32;
}

server {
    listen 443 ssl http2;
    server_name example.com;

    ssl_certificate /path/to/cert.pem;
    ssl_certificate_key /path/to/key.pem;

    # 主网站或其他服务
    location / {
        root /var/www/html;
        index index.html;
    }

    # tls-tunnel 反向代理（子目录）
    # 客户端需要配置：server_path = "/tunnel/"
    location /tunnel/ {
        proxy_pass http://tls_tunnel_http2/;
        
        # 重要：结尾的 / 会移除 /tunnel/ 前缀
        # 后端收到的路径为 /
        
        proxy_http_version 1.1;
        proxy_set_header Connection "";
        
        proxy_connect_timeout 3600s;
        proxy_send_timeout 3600s;
        proxy_read_timeout 3600s;
        
        proxy_buffering off;
        proxy_request_buffering off;
    }
}
```

**客户端配置**（`examples/client-subpath.toml`）：

```toml
[client]
server_addr = "example.com"
server_port = 443
server_path = "/tunnel/"  # 必须匹配 Nginx location

transport = "http2"
skip_verify = false
auth_key = "your-secret-auth-key"

[[proxies]]
name = "ssh"
publish_port = 2222
local_port = 22
```

**注意事项**：

- `server_path` 必须与 Nginx 的 `location` 配置匹配
- 路径建议以 `/` 开头和结尾
- `proxy_pass` 的结尾加 `/` 会移除前缀（推荐）
- 子目录模式允许多个服务共享同一域名

## Apache 配置示例

### HTTP/2 反向代理

```apache
<VirtualHost *:443>
    ServerName tunnel.example.com
    
    # SSL 配置
    SSLEngine on
    SSLCertificateFile /path/to/cert.pem
    SSLCertificateKeyFile /path/to/key.pem
    SSLProtocol all -SSLv3 -TLSv1 -TLSv1.1
    SSLCipherSuite HIGH:!aNULL:!MD5
    
    # 启用 HTTP/2
    Protocols h2 http/1.1
    
    # 反向代理
    ProxyPreserveHost On
    ProxyPass / http://127.0.0.1:8080/
    ProxyPassReverse / http://127.0.0.1:8080/
    
    # 超时设置
    ProxyTimeout 3600
    
    # 日志
    ErrorLog ${APACHE_LOG_DIR}/tunnel-error.log
    CustomLog ${APACHE_LOG_DIR}/tunnel-access.log combined
</VirtualHost>
```

### WebSocket 反向代理

```apache
<VirtualHost *:443>
    ServerName tunnel.example.com
    
    # SSL 配置
    SSLEngine on
    SSLCertificateFile /path/to/cert.pem
    SSLCertificateKeyFile /path/to/key.pem
    
    # WebSocket 支持
    RewriteEngine On
    RewriteCond %{HTTP:Upgrade} websocket [NC]
    RewriteCond %{HTTP:Connection} upgrade [NC]
    RewriteRule ^/?(.*) "ws://127.0.0.1:8080/$1" [P,L]
    
    # 反向代理
    ProxyPreserveHost On
    ProxyPass / http://127.0.0.1:8080/
    ProxyPassReverse / http://127.0.0.1:8080/
    
    # 超时设置
    ProxyTimeout 3600
    
    # 日志
    ErrorLog ${APACHE_LOG_DIR}/tunnel-error.log
    CustomLog ${APACHE_LOG_DIR}/tunnel-access.log combined
</VirtualHost>
```

## 配置文件完整示例

### 服务器配置（后端）

```toml
[server]
bind_addr = "127.0.0.1"  # 只监听本地，不暴露到公网
bind_port = 8080
transport = "http2"       # 或 "wss"
behind_proxy = true       # 启用反向代理模式

# 不需要 cert_file 和 key_file（TLS 由前端代理处理）

# 转发目标
[[server.forwards]]
local_port = 22
remote_addr = "localhost"
remote_port = 22
```

### 客户端配置（连接到 Nginx）

```toml
[client]
server_addr = "tunnel.example.com"  # Nginx 的域名
server_port = 443                   # Nginx 的 HTTPS 端口
transport = "http2"                 # 或 "wss"

# 需要信任 Nginx 的 SSL 证书
# 如果使用自签名证书，需要指定 CA
# ca_file = "/path/to/ca.pem"

# 本地监听
[[client.forwards]]
local_port = 2222
remote_addr = "localhost"
remote_port = 22
```

## 安全考虑

### 1. 本地绑定

后端 tls-tunnel 应该只监听本地地址：

```toml
bind_addr = "127.0.0.1"  # ✅ 只允许本地访问
# bind_addr = "0.0.0.0"  # ❌ 会暴露明文端口到公网
```

### 2. 防火墙规则

确保后端端口不被外部访问：

```bash
# 只允许本地访问 8080 端口
iptables -A INPUT -p tcp --dport 8080 -i lo -j ACCEPT
iptables -A INPUT -p tcp --dport 8080 -j DROP
```

### 3. Unix Socket（可选）

如果可能，使用 Unix socket 而不是 TCP 端口：

```nginx
upstream tls_tunnel {
    server unix:/var/run/tls-tunnel.sock;
}
```

注：当前版本暂不支持 Unix socket，这是未来的改进方向。

### 4. 访问控制

在 Nginx 中配置访问控制：

```nginx
# 限制客户端 IP
allow 192.168.1.0/24;
deny all;

# 或使用 HTTP 认证
auth_basic "Restricted";
auth_basic_user_file /etc/nginx/.htpasswd;
```

## 故障排查

### 1. 连接被重置

**症状**：客户端连接立即断开

**可能原因**：
- 前端代理未正确转发 HTTP/2 或 WebSocket
- 超时设置过短

**解决方法**：
- 检查 Nginx 配置中的 `proxy_http_version` 和 `Upgrade` 头
- 增加 `proxy_read_timeout` 等超时设置

### 2. 握手失败

**症状**：客户端报告握手错误

**可能原因**：
- 客户端仍使用 TLS 连接到后端端口
- Nginx 未正确配置 SSL

**解决方法**：
- 确认客户端连接到 Nginx 的 HTTPS 端口
- 检查 Nginx SSL 证书配置

### 3. 性能下降

**症状**：通过代理的连接比直连慢

**可能原因**：
- 代理缓冲导致延迟
- Nginx 工作进程数不足

**解决方法**：
- 禁用代理缓冲：`proxy_buffering off`
- 调整 Nginx 工作进程：`worker_processes auto`
- 启用 keepalive：`keepalive 32`

### 4. 日志调试

启用详细日志：

```nginx
# Nginx 错误日志
error_log /var/log/nginx/error.log debug;

# 访问日志
access_log /var/log/nginx/access.log combined;
```

```bash
# tls-tunnel 日志（使用 RUST_LOG）
RUST_LOG=debug tls-tunnel server
```

## 监控和指标

### Nginx 状态监控

```nginx
location /nginx_status {
    stub_status on;
    allow 127.0.0.1;
    deny all;
}
```

### 健康检查

```nginx
upstream tls_tunnel {
    server 127.0.0.1:8080 max_fails=3 fail_timeout=30s;
    
    # 健康检查（Nginx Plus）
    health_check interval=10s fails=3 passes=2;
}
```

## 性能优化

### 1. Nginx 调优

```nginx
# 增加工作进程
worker_processes auto;
worker_rlimit_nofile 65535;

events {
    worker_connections 4096;
    use epoll;
}

http {
    # 连接池
    keepalive_timeout 65;
    keepalive_requests 100;
    
    # 缓冲
    client_body_buffer_size 128k;
    client_max_body_size 10m;
    
    # 压缩（谨慎使用，可能影响加密流量）
    gzip off;
}
```

### 2. 系统调优

```bash
# 增加文件描述符限制
ulimit -n 65535

# 调整 TCP 参数
sysctl -w net.core.somaxconn=4096
sysctl -w net.ipv4.tcp_max_syn_backlog=8192
sysctl -w net.ipv4.tcp_tw_reuse=1
```

### 3. tls-tunnel 优化

```toml
[server]
# 增加连接池大小
max_connections = 1000

# 调整缓冲区大小
buffer_size = 8192
```

## 总结

反向代理模式的优势：

✅ **安全性**：TLS 终止在前端，统一管理证书  
✅ **灵活性**：可以与其他 Web 服务共存  
✅ **扩展性**：利用 Nginx 的负载均衡和缓存功能  
✅ **维护性**：集中管理 SSL 证书和配置  

适用场景：

- 需要与 Web 服务器共享同一端口
- 需要负载均衡多个隧道实例
- 需要统一的访问控制和日志
- 需要利用 CDN 或其他代理服务

不适用场景：

- 简单的点对点隧道（直连更高效）
- 对延迟极度敏感的应用
- 需要端到端加密验证的场景
