use crate::stats::StatsManager;
use anyhow::{Context, Result};
use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{error, info};

/// 启动统计数据 HTTP 服务器
pub async fn start_stats_server(port: u16, stats_manager: StatsManager) -> Result<()> {
    let listener = TcpListener::bind(format!("0.0.0.0:{}", port))
        .await
        .context("Failed to bind stats server port")?;

    info!("Stats server listening on http://0.0.0.0:{}", port);

    loop {
        match listener.accept().await {
            Ok((mut stream, addr)) => {
                let stats_manager = stats_manager.clone();

                tokio::spawn(async move {
                    handle_stats_request(&mut stream, addr, &stats_manager).await;
                });
            }
            Err(e) => {
                error!("Failed to accept stats connection: {}", e);
            }
        }
    }
}

/// 处理单个统计请求
async fn handle_stats_request(
    stream: &mut tokio::net::TcpStream,
    _addr: SocketAddr,
    stats_manager: &StatsManager,
) {
    let mut buffer = vec![0u8; 4096];
    let n = match stream.read(&mut buffer).await {
        Ok(n) => n,
        Err(e) => {
            error!("Failed to read from stats client {}: {}", _addr, e);
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
        let stats = stats_manager.get_all_stats();
        let json = serde_json::to_string_pretty(&stats).unwrap_or_default();

        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            json.len(),
            json
        )
    } else if path == "/" || path.starts_with("/?") {
        // 返回HTML页面
        let html = generate_stats_html(stats_manager);

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

/// 生成统计信息HTML页面
fn generate_stats_html(stats_manager: &StatsManager) -> String {
    let stats = stats_manager.get_all_stats();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
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
            stat.publish_addr,
            stat.publish_port,
            stat.local_port,
            stat.active_connections,
            stat.total_connections,
            bytes_sent,
            bytes_received,
            uptime
        ));
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <meta http-equiv="refresh" content="5">
    <title>TLS Tunnel - Statistics</title>
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
            background: white;
            border-radius: 12px;
            box-shadow: 0 20px 60px rgba(0,0,0,0.3);
            overflow: hidden;
        }}
        header {{
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            padding: 30px;
            text-align: center;
        }}
        h1 {{
            font-size: 2.5em;
            font-weight: 600;
            margin-bottom: 10px;
        }}
        .subtitle {{
            font-size: 1.1em;
            opacity: 0.9;
        }}
        .info {{
            background: #f8f9fa;
            padding: 20px 30px;
            border-bottom: 2px solid #e9ecef;
            display: flex;
            justify-content: space-between;
            align-items: center;
            flex-wrap: wrap;
        }}
        .info-item {{
            display: flex;
            align-items: center;
            margin: 5px 15px;
        }}
        .info-label {{
            font-weight: 600;
            color: #495057;
            margin-right: 8px;
        }}
        .info-value {{
            color: #667eea;
            font-weight: 500;
        }}
        .content {{
            padding: 30px;
        }}
        table {{
            width: 100%;
            border-collapse: collapse;
            margin-top: 10px;
        }}
        th {{
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
            color: white;
            padding: 15px;
            text-align: left;
            font-weight: 600;
            font-size: 0.95em;
            text-transform: uppercase;
            letter-spacing: 0.5px;
        }}
        td {{
            padding: 15px;
            border-bottom: 1px solid #e9ecef;
        }}
        tr:hover {{
            background: #f8f9fa;
        }}
        .badge {{
            display: inline-block;
            padding: 4px 12px;
            border-radius: 20px;
            font-size: 0.85em;
            font-weight: 600;
        }}
        .badge-success {{
            background: #d4edda;
            color: #155724;
        }}
        .empty {{
            text-align: center;
            padding: 60px;
            color: #6c757d;
        }}
        .empty-icon {{
            font-size: 4em;
            margin-bottom: 20px;
            opacity: 0.3;
        }}
        footer {{
            text-align: center;
            padding: 20px;
            color: #6c757d;
            font-size: 0.9em;
            border-top: 1px solid #e9ecef;
        }}
        .refresh-note {{
            color: #6c757d;
            font-size: 0.85em;
            font-style: italic;
        }}
    </style>
</head>
<body>
    <div class="container">
        <header>
            <h1>🔐 TLS Tunnel Statistics</h1>
            <p class="subtitle">Real-time proxy monitoring dashboard</p>
        </header>
        
        <div class="info">
            <div class="info-item">
                <span class="info-label">Total Proxies:</span>
                <span class="info-value">{}</span>
            </div>
            <div class="info-item">
                <span class="info-label">Total Active Connections:</span>
                <span class="info-value">{}</span>
            </div>
            <div class="info-item">
                <span class="info-label">Total Connections:</span>
                <span class="info-value">{}</span>
            </div>
            <div class="info-item refresh-note">
                Auto-refresh: 5 seconds
            </div>
        </div>

        <div class="content">
            {}
        </div>

        <footer>
            <p>TLS Tunnel Server · Powered by Rust & Tokio</p>
            <p style="margin-top: 8px;"><a href="/stats" style="color: #667eea; text-decoration: none;">View JSON API</a></p>
        </footer>
    </div>
</body>
</html>"#,
        stats.len(),
        stats.iter().map(|s| s.active_connections).sum::<u64>(),
        stats.iter().map(|s| s.total_connections).sum::<u64>(),
        if stats.is_empty() {
            r#"<div class="empty">
                <div class="empty-icon">📊</div>
                <h2 style="color: #495057; margin-bottom: 10px;">No Proxies Connected</h2>
                <p>Waiting for clients to connect...</p>
            </div>"#
                .to_string()
        } else {
            format!(
                r#"<table>
                <thead>
                    <tr>
                        <th>Proxy Name</th>
                        <th>Published Address</th>
                        <th>Client Port</th>
                        <th>Active</th>
                        <th>Total</th>
                        <th>Sent</th>
                        <th>Received</th>
                        <th>Uptime</th>
                    </tr>
                </thead>
                <tbody>
                    {}
                </tbody>
            </table>"#,
                rows
            )
        }
    )
}

/// 格式化字节数为人类可读格式
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{} {}", bytes, UNITS[unit_idx])
    } else {
        format!("{:.2} {}", size, UNITS[unit_idx])
    }
}

/// 格式化持续时间为人类可读格式
pub fn format_duration(seconds: u64) -> String {
    let days = seconds / 86400;
    let hours = (seconds % 86400) / 3600;
    let minutes = (seconds % 3600) / 60;
    let secs = seconds % 60;

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
