/// 客户端统计模块
///
/// 提供客户端代理的实时统计信息跟踪和 HTTP 服务器
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{error, info};

use crate::config::ProxyType;

/// 客户端代理统计信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientProxyStats {
    /// 代理名称
    pub name: String,
    /// 代理类型
    pub proxy_type: String,
    /// 本地监听地址
    pub bind_addr: String,
    /// 本地监听端口
    pub bind_port: u16,
    /// 目标地址（服务器端发布地址）
    pub target_addr: String,
    /// 目标端口（服务器端发布端口）
    pub target_port: u16,
    /// 当前活跃连接数
    pub active_connections: usize,
    /// 总连接数
    pub total_connections: u64,
    /// 发送字节数
    pub bytes_sent: u64,
    /// 接收字节数
    pub bytes_received: u64,
    /// 启动时间（Unix 时间戳）
    pub start_time: u64,
    /// 连接状态
    pub status: String,
}

/// 客户端统计跟踪器（线程安全）
#[derive(Clone)]
pub struct ClientStatsTracker {
    name: String,
    proxy_type: ProxyType,
    bind_addr: String,
    bind_port: u16,
    target_addr: String,
    target_port: u16,
    active_connections: Arc<AtomicUsize>,
    total_connections: Arc<AtomicU64>,
    bytes_sent: Arc<AtomicU64>,
    bytes_received: Arc<AtomicU64>,
    start_time: u64,
    status: Arc<parking_lot::RwLock<String>>,
}

impl ClientStatsTracker {
    /// 创建新的统计跟踪器
    pub fn new(
        name: String,
        proxy_type: ProxyType,
        bind_addr: String,
        bind_port: u16,
        target_addr: String,
        target_port: u16,
    ) -> Self {
        Self {
            name,
            proxy_type,
            bind_addr,
            bind_port,
            target_addr,
            target_port,
            active_connections: Arc::new(AtomicUsize::new(0)),
            total_connections: Arc::new(AtomicU64::new(0)),
            bytes_sent: Arc::new(AtomicU64::new(0)),
            bytes_received: Arc::new(AtomicU64::new(0)),
            start_time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            status: Arc::new(parking_lot::RwLock::new("Idle".to_string())),
        }
    }

    /// 连接开始
    pub fn connection_started(&self) {
        self.active_connections.fetch_add(1, Ordering::Relaxed);
        self.total_connections.fetch_add(1, Ordering::Relaxed);
        self.update_status("Connected");
    }

    /// 连接结束
    pub fn connection_ended(&self) {
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
        let active = self.active_connections.load(Ordering::Relaxed);
        if active == 0 {
            self.update_status("Idle");
        }
    }

    /// 记录发送字节数
    pub fn record_bytes_sent(&self, bytes: u64) {
        self.bytes_sent.fetch_add(bytes, Ordering::Relaxed);
    }

    /// 记录接收字节数
    pub fn record_bytes_received(&self, bytes: u64) {
        self.bytes_received.fetch_add(bytes, Ordering::Relaxed);
    }

    /// 更新状态
    pub fn update_status(&self, status: impl Into<String>) {
        let mut s = self.status.write();
        *s = status.into();
    }

    /// 获取统计快照
    pub fn snapshot(&self) -> ClientProxyStats {
        ClientProxyStats {
            name: self.name.clone(),
            proxy_type: format!("{:?}", self.proxy_type),
            bind_addr: self.bind_addr.clone(),
            bind_port: self.bind_port,
            target_addr: self.target_addr.clone(),
            target_port: self.target_port,
            active_connections: self.active_connections.load(Ordering::Relaxed),
            total_connections: self.total_connections.load(Ordering::Relaxed),
            bytes_sent: self.bytes_sent.load(Ordering::Relaxed),
            bytes_received: self.bytes_received.load(Ordering::Relaxed),
            start_time: self.start_time,
            status: self.status.read().clone(),
        }
    }

    /// 重置统计计数器（保留 start_time）
    pub fn reset(&self) {
        self.active_connections.store(0, Ordering::Relaxed);
        self.total_connections.store(0, Ordering::Relaxed);
        self.bytes_sent.store(0, Ordering::Relaxed);
        self.bytes_received.store(0, Ordering::Relaxed);
        self.update_status("Reset");
    }
}

/// 全局客户端统计管理器
#[derive(Clone)]
pub struct ClientStatsManager {
    trackers: Arc<parking_lot::RwLock<Vec<ClientStatsTracker>>>,
}

impl ClientStatsManager {
    /// 创建新的统计管理器
    pub fn new() -> Self {
        Self {
            trackers: Arc::new(parking_lot::RwLock::new(Vec::new())),
        }
    }

    /// 添加统计跟踪器
    #[allow(dead_code)]
    pub fn add_tracker(&self, tracker: ClientStatsTracker) {
        let mut trackers = self.trackers.write();
        trackers.push(tracker);
    }

    /// 添加或更新统计跟踪器（如果已存在相同名称的跟踪器则替换）
    pub fn add_or_update_tracker(&self, tracker: ClientStatsTracker) {
        let mut trackers = self.trackers.write();
        // 查找是否已存在相同名称的跟踪器
        if let Some(pos) = trackers.iter().position(|t| t.name == tracker.name) {
            // 替换现有的跟踪器
            trackers[pos] = tracker;
        } else {
            // 添加新的跟踪器
            trackers.push(tracker);
        }
    }

    /// 获取所有统计信息
    pub fn get_all_stats(&self) -> Vec<ClientProxyStats> {
        let trackers = self.trackers.read();
        trackers.iter().map(|t| t.snapshot()).collect()
    }

    /// 根据名称获取统计跟踪器
    pub fn get_tracker(&self, name: &str) -> Option<ClientStatsTracker> {
        let trackers = self.trackers.read();
        trackers.iter().find(|t| t.name == name).cloned()
    }

    /// 重置所有统计信息
    #[allow(dead_code)]
    pub fn reset_all(&self) {
        let trackers = self.trackers.read();
        for tracker in trackers.iter() {
            tracker.reset();
        }
    }
}

impl Default for ClientStatsManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 启动客户端统计 HTTP 服务器
///
/// 提供 /stats 端点返回所有客户端代理的统计信息
pub async fn start_client_stats_server(
    bind_addr: String,
    port: u16,
    manager: ClientStatsManager,
) -> Result<()> {
    let listener = TcpListener::bind(format!("{}:{}", bind_addr, port))
        .await
        .context("Failed to bind client stats server port")?;

    info!(
        "Client stats server listening on http://{}:{}",
        bind_addr, port
    );

    loop {
        match listener.accept().await {
            Ok((mut stream, addr)) => {
                let manager = manager.clone();

                tokio::spawn(async move {
                    handle_client_stats_request(&mut stream, addr, &manager).await;
                });
            }
            Err(e) => {
                error!("Failed to accept client stats connection: {}", e);
            }
        }
    }
}

/// 处理单个统计请求
async fn handle_client_stats_request(
    stream: &mut tokio::net::TcpStream,
    _addr: SocketAddr,
    manager: &ClientStatsManager,
) {
    let mut buffer = vec![0u8; 4096];
    let n = match stream.read(&mut buffer).await {
        Ok(n) => n,
        Err(e) => {
            error!("Failed to read from client stats client {}: {}", _addr, e);
            return;
        }
    };

    // 解析HTTP请求
    let request = String::from_utf8_lossy(&buffer[..n]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");

    let response = if path == "/stats" || path == "/stats/" {
        // 返回JSON格式的统计信息
        let stats = manager.get_all_stats();
        let json = serde_json::to_string_pretty(&stats).unwrap_or_default();

        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            json.len(),
            json
        )
    } else if path == "/" || path.starts_with("/?") {
        // 返回HTML页面
        let html = generate_client_stats_html(manager);

        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\n\r\n{}",
            html.len(),
            html
        )
    } else {
        // 404
        let body = "404 Not Found";
        format!(
            "HTTP/1.1 404 Not Found\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{}",
            body.len(),
            body
        )
    };

    if let Err(e) = stream.write_all(response.as_bytes()).await {
        error!("Failed to write response to {}: {}", _addr, e);
    }
}

/// 生成客户端统计信息HTML页面
fn generate_client_stats_html(manager: &ClientStatsManager) -> String {
    let stats = manager.get_all_stats();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();

    let mut rows = String::new();
    for stat in &stats {
        let uptime_seconds = now.saturating_sub(stat.start_time);
        let uptime = format_duration(uptime_seconds);
        let bytes_sent = format_bytes(stat.bytes_sent);
        let bytes_received = format_bytes(stat.bytes_received);

        rows.push_str(&format!(
            r#"
            <tr>
                <td>{}</td>
                <td>{}</td>
                <td>{}:{}</td>
                <td>{}:{}</td>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
                <td>{}</td>
            </tr>
            "#,
            stat.name,
            stat.proxy_type,
            stat.bind_addr,
            stat.bind_port,
            stat.target_addr,
            stat.target_port,
            stat.active_connections,
            stat.total_connections,
            bytes_sent,
            bytes_received,
            uptime,
            stat.status
        ));
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <meta http-equiv="refresh" content="5">
    <title>TLS Tunnel - 客户端统计</title>
    <style>
        * {{
            margin: 0;
            padding: 0;
            box-sizing: border-box;
        }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            min-height: 100vh;
            padding: 20px;
        }}
        .container {{
            max-width: 1400px;
            margin: 0 auto;
        }}
        .header {{
            background: white;
            border-radius: 10px;
            padding: 30px;
            margin-bottom: 20px;
            box-shadow: 0 2px 10px rgba(0,0,0,0.1);
        }}
        h1 {{
            color: #333;
            font-size: 28px;
            margin-bottom: 10px;
        }}
        .subtitle {{
            color: #666;
            font-size: 14px;
        }}
        .stats-table {{
            background: white;
            border-radius: 10px;
            padding: 20px;
            box-shadow: 0 2px 10px rgba(0,0,0,0.1);
            overflow-x: auto;
        }}
        table {{
            width: 100%;
            border-collapse: collapse;
        }}
        th {{
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            padding: 15px;
            text-align: left;
            font-weight: 600;
        }}
        td {{
            padding: 12px 15px;
            border-bottom: 1px solid #eee;
        }}
        tr:hover {{
            background-color: #f5f5f5;
        }}
        .badge {{
            display: inline-block;
            padding: 4px 8px;
            border-radius: 4px;
            font-size: 12px;
            font-weight: 600;
        }}
        .badge-success {{
            background-color: #d4edda;
            color: #155724;
        }}
        .badge-warning {{
            background-color: #fff3cd;
            color: #856404;
        }}
        .badge-info {{
            background-color: #d1ecf1;
            color: #0c5460;
        }}
        .summary {{
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
            gap: 15px;
            margin-bottom: 20px;
        }}
        .summary-card {{
            background: white;
            border-radius: 10px;
            padding: 20px;
            box-shadow: 0 2px 10px rgba(0,0,0,0.1);
        }}
        .summary-card h3 {{
            color: #666;
            font-size: 14px;
            margin-bottom: 10px;
        }}
        .summary-card p {{
            color: #333;
            font-size: 24px;
            font-weight: 600;
        }}
    </style>
</head>
<body>
    <div class="container">
        <div class="header">
            <h1>🚀 TLS Tunnel - 客户端统计面板</h1>
            <p class="subtitle">实时监控客户端代理连接状态和流量统计 (每 5 秒自动刷新)</p>
        </div>
        
        <div class="summary">
            <div class="summary-card">
                <h3>总代理数</h3>
                <p>{}</p>
            </div>
            <div class="summary-card">
                <h3>活跃连接数</h3>
                <p>{}</p>
            </div>
            <div class="summary-card">
                <h3>总连接数</h3>
                <p>{}</p>
            </div>
        </div>

        <div class="stats-table">
            <table>
                <thead>
                    <tr>
                        <th>代理名称</th>
                        <th>代理类型</th>
                        <th>本地地址</th>
                        <th>目标地址</th>
                        <th>活跃连接</th>
                        <th>总连接数</th>
                        <th>发送流量</th>
                        <th>接收流量</th>
                        <th>运行时长</th>
                        <th>状态</th>
                    </tr>
                </thead>
                <tbody>
                    {}
                </tbody>
            </table>
        </div>
    </div>
</body>
</html>"#,
        stats.len(),
        stats.iter().map(|s| s.active_connections).sum::<usize>(),
        stats.iter().map(|s| s.total_connections).sum::<u64>(),
        rows
    )
}

/// 计算运行时长（格式化）
pub fn format_duration(seconds: u64) -> String {
    let duration = Duration::from_secs(seconds);
    let days = duration.as_secs() / 86400;
    let hours = (duration.as_secs() % 86400) / 3600;
    let minutes = (duration.as_secs() % 3600) / 60;
    let secs = duration.as_secs() % 60;

    if days > 0 {
        format!("{}d {}h", days, hours)
    } else if hours > 0 {
        format!("{}h {}m", hours, minutes)
    } else if minutes > 0 {
        format!("{}m {}s", minutes, secs)
    } else {
        format!("{}s", secs)
    }
}

/// 格式化字节数
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.2} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stats_tracker_creation() {
        let tracker = ClientStatsTracker::new(
            "test-proxy".to_string(),
            ProxyType::Tcp,
            "127.0.0.1".to_string(),
            8080,
            "server.example.com".to_string(),
            3080,
        );

        let stats = tracker.snapshot();
        assert_eq!(stats.name, "test-proxy");
        assert_eq!(stats.bind_port, 8080);
        assert_eq!(stats.target_port, 3080);
        assert_eq!(stats.active_connections, 0);
        assert_eq!(stats.total_connections, 0);
    }

    #[test]
    fn test_connection_tracking() {
        let tracker = ClientStatsTracker::new(
            "test".to_string(),
            ProxyType::Tcp,
            "127.0.0.1".to_string(),
            8080,
            "server".to_string(),
            3080,
        );

        tracker.connection_started();
        let stats = tracker.snapshot();
        assert_eq!(stats.active_connections, 1);
        assert_eq!(stats.total_connections, 1);

        tracker.connection_started();
        let stats = tracker.snapshot();
        assert_eq!(stats.active_connections, 2);
        assert_eq!(stats.total_connections, 2);

        tracker.connection_ended();
        let stats = tracker.snapshot();
        assert_eq!(stats.active_connections, 1);
        assert_eq!(stats.total_connections, 2);
    }

    #[test]
    fn test_bytes_tracking() {
        let tracker = ClientStatsTracker::new(
            "test".to_string(),
            ProxyType::Tcp,
            "127.0.0.1".to_string(),
            8080,
            "server".to_string(),
            3080,
        );

        tracker.record_bytes_sent(1024);
        tracker.record_bytes_received(2048);

        let stats = tracker.snapshot();
        assert_eq!(stats.bytes_sent, 1024);
        assert_eq!(stats.bytes_received, 2048);
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(30), "30s");
        assert_eq!(format_duration(90), "1m 30s");
        assert_eq!(format_duration(3661), "1h 1m");
        assert_eq!(format_duration(86400), "1d 0h");
        assert_eq!(format_duration(90000), "1d 1h");
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1048576), "1.00 MB");
        assert_eq!(format_bytes(1073741824), "1.00 GB");
        assert_eq!(format_bytes(1099511627776), "1.00 TB");
    }

    #[test]
    fn test_stats_manager() {
        let manager = ClientStatsManager::new();

        let tracker1 = ClientStatsTracker::new(
            "proxy1".to_string(),
            ProxyType::Tcp,
            "127.0.0.1".to_string(),
            8080,
            "server".to_string(),
            3080,
        );

        let tracker2 = ClientStatsTracker::new(
            "proxy2".to_string(),
            ProxyType::Tcp,
            "127.0.0.1".to_string(),
            8081,
            "server".to_string(),
            3081,
        );

        manager.add_tracker(tracker1);
        manager.add_tracker(tracker2);

        let stats = manager.get_all_stats();
        assert_eq!(stats.len(), 2);

        let found = manager.get_tracker("proxy1");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "proxy1");
    }
}
