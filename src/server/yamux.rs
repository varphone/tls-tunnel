use super::registry::ProxyRegistry;
use super::visitor::handle_visitor_stream;
use crate::config::ServerConfig;
use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use yamux::Connection as YamuxConnection;

/// 运行yamux连接的poll循环
pub async fn run_yamux_connection<T>(
    mut yamux_conn: YamuxConnection<T>,
    mut stream_rx: mpsc::Receiver<(mpsc::Sender<yamux::Stream>, u16, String)>,
    proxy_registry: ProxyRegistry,
    _stream_tx_for_visitors: mpsc::Sender<(mpsc::Sender<yamux::Stream>, u16, String)>,
    server_config: &ServerConfig,
) -> Result<()>
where
    T: futures::io::AsyncRead + futures::io::AsyncWrite + Unpin,
{
    use futures::future::poll_fn;

    loop {
        // Poll yamux连接和stream请求
        tokio::select! {
            // 处理新的stream请求
            req = stream_rx.recv() => {
                if let Some((response_tx, _remote_port, proxy_name)) = req {
                    // 创建新的outbound stream
                    let stream = poll_fn(|cx| yamux_conn.poll_new_outbound(cx)).await
                        .context("Failed to create yamux stream")?;

                    info!("Created yamux stream for proxy '{}'", proxy_name);

                    if response_tx.send(stream).await.is_err() {
                        warn!("Failed to send stream back to handler");
                    }
                } else {
                    info!("Stream request channel closed");
                    break;
                }
            }
            // Poll yamux连接以处理incoming streams（来自其他客户端的visitor请求）
            stream_result = poll_fn(|cx| yamux_conn.poll_next_inbound(cx)) => {
                match stream_result {
                    Some(Ok(stream)) => {
                        info!("Received visitor stream from client");
                        let proxy_registry_clone = proxy_registry.clone();
                        let server_config_clone = server_config.clone();
                        tokio::spawn(async move {
                            if let Err(e) = handle_visitor_stream(stream, proxy_registry_clone, &server_config_clone).await {
                                error!("Failed to handle visitor stream: {}", e);
                            }
                        });
                    }
                    Some(Err(e)) => {
                        error!("Yamux poll error: {}", e);
                        break;
                    }
                    None => {
                        info!("Yamux connection closed by client");
                        break;
                    }
                }
            }
        }
    }

    info!("Yamux connection loop ended");
    Ok(())
}
